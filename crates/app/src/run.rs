use crate::case::{Case, GroundMode};
use crate::prepare::{load_geometry, plan_domain, GeometryInfo};
use crate::result::{GridPoint, HistoryPoint, RunResult};
use anyhow::{bail, Context, Result};
use cfd_aero::{has_settled, per_part, summarise, Reference, Sample};
use cfd_geom::voxelize;
use cfd_io_vtk::{write_structured_points, Field};
use cfd_lbm::{Config, Ground, Lbm, FLUID};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::Instant;

/// Lattice speed of the incoming air. 0.08 keeps the lattice Mach number near 0.14.
pub const U_LATTICE: f64 = 0.08;
/// Smagorinsky constant for the LES closure.
pub const DEFAULT_SMAGORINSKY: f64 = 0.16;
const MAX_MEMORY: usize = 12 * 1024 * 1024 * 1024;

/// A 2D cut through the flow: |u| / U per pixel, -1 for solid. Row 0 is the top.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Slice {
    pub width: usize,
    pub height: usize,
    pub dx: f64,
    pub values: Vec<f32>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Slices {
    pub side: Slice,
    pub top: Slice,
}

pub fn slices_from_json(text: &str) -> Result<Slices> {
    Ok(serde_json::from_str(text)?)
}

#[derive(Clone, Debug)]
pub struct RunOutput {
    pub result: RunResult,
    pub slices: Slices,
}

#[derive(Clone, Debug, Default)]
pub struct Progress {
    pub run: usize,
    pub runs: usize,
    pub label: String,
    pub step: usize,
    pub max_steps: usize,
    pub car_lengths: f64,
    pub cd: f64,
    pub cl: f64,
    pub mlups: f64,
    pub elapsed_s: f64,
    pub history: Vec<HistoryPoint>,
}

pub enum Event {
    Log(String),
    Geometry(Arc<GeometryInfo>),
    Progress(Progress),
    RunDone(Box<RunOutput>),
    Finished,
    Failed(String),
}

pub struct RunHandle {
    pub events: Receiver<Event>,
    pub stop: Arc<AtomicBool>,
    pub thread: std::thread::JoinHandle<()>,
}

impl RunHandle {
    pub fn request_stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

pub fn start(case: Case) -> RunHandle {
    let (tx, rx) = channel();
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = stop.clone();
    let thread = std::thread::Builder::new()
        .name("cfd-run".into())
        .spawn(move || {
            match run_all(&case, &tx, &stop2) {
                Ok(()) => tx.send(Event::Finished).ok(),
                Err(e) => tx.send(Event::Failed(format!("{e:#}"))).ok(),
            };
        })
        .expect("could not start the run thread");
    RunHandle { events: rx, stop, thread }
}

/// Runs a case to completion on the current thread, calling `on_event` for each event.
pub fn run_blocking(case: &Case, mut on_event: impl FnMut(Event)) -> Result<Vec<RunOutput>> {
    let h = start(case.clone());
    let mut outs = Vec::new();
    let mut err = None;
    for e in h.events.iter() {
        match e {
            Event::RunDone(o) => {
                outs.push((*o).clone());
                on_event(Event::RunDone(o));
            }
            Event::Failed(m) => err = Some(m),
            other => on_event(other),
        }
    }
    h.thread.join().ok();
    match err {
        Some(m) => bail!(m),
        None => Ok(outs),
    }
}

fn run_all(case: &Case, tx: &Sender<Event>, stop: &AtomicBool) -> Result<()> {
    let problems = case.check();
    if !problems.is_empty() {
        bail!(problems.join("\n"));
    }
    tx.send(Event::Log("Reading geometry".into())).ok();
    let info = Arc::new(load_geometry(case)?);
    tx.send(Event::Log(info.summary())).ok();
    for w in &info.warnings {
        tx.send(Event::Log(format!("Warning: {w}"))).ok();
    }
    tx.send(Event::Geometry(info.clone())).ok();
    let speeds = case.speeds();
    let grids = case.grids();
    let runs = speeds.len() * grids.len();
    let mut run = 0;
    for &speed in &speeds {
        let mut study = Vec::new();
        let mut last = None;
        for &n in &grids {
            run += 1;
            if stop.load(Ordering::Relaxed) {
                break;
            }
            let out = run_one(case, &info, speed, n, run, runs, tx, stop)?;
            study.push(GridPoint {
                cells_across_length: n,
                cells: out.result.cells,
                cd: out.result.summary.cd.mean,
                cl: out.result.summary.cl.mean,
            });
            tx.send(Event::Log(format!("{} cells across: {}", n, out.result.headline()))).ok();
            last = Some(out);
        }
        if let Some(mut out) = last {
            if study.len() > 1 {
                out.result.grid_change_cd_pct =
                    cfd_aero::grid_change_pct(&study.iter().map(|g| g.cd).collect::<Vec<_>>());
                out.result.grid_study = study;
            }
            save(case, &mut out)?;
            tx.send(Event::RunDone(Box::new(out))).ok();
        }
    }
    Ok(())
}

fn save(case: &Case, out: &mut RunOutput) -> Result<()> {
    let r = &mut out.result;
    let dir = case.runs_dir().join(format!(
        "{}-{}-{:.0}mps",
        crate::time::stamp(r.created_unix),
        sanitize(&r.name),
        r.speed_mps
    ));
    std::fs::create_dir_all(&dir).with_context(|| format!("could not create {}", dir.display()))?;
    r.dir = Some(dir.clone());
    r.save(&dir)?;
    std::fs::write(dir.join("slices.json"), serde_json::to_string(&out.slices)?)?;
    std::fs::write(dir.join("report.html"), crate::report::html(r, &out.slices))?;
    Ok(())
}

fn sanitize(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect()
}

#[allow(clippy::too_many_arguments)]
fn run_one(
    case: &Case,
    info: &GeometryInfo,
    speed: f64,
    n_across: usize,
    run: usize,
    runs: usize,
    tx: &Sender<Event>,
    stop: &AtomicBool,
) -> Result<RunOutput> {
    let started = Instant::now();
    let plan = plan_domain(info, n_across, case.ground);
    if plan.memory_bytes() > MAX_MEMORY {
        bail!(
            "This quality needs {:.1} GB of memory ({} cells). Pick a lower quality.",
            plan.memory_bytes() as f64 / 1e9,
            plan.cells()
        );
    }
    let label = format!("{:.1} m/s, {} cells across the car", speed, n_across);
    tx.send(Event::Log(format!(
        "Run {run}/{runs}: {label}; tunnel {} x {} x {} = {:.2} M cells, {:.0} MB",
        plan.nx,
        plan.ny,
        plan.nz,
        plan.cells() as f64 / 1e6,
        plan.memory_bytes() as f64 / 1e6
    )))
    .ok();
    let vox = voxelize(&info.mesh, plan.origin, plan.dx, plan.nx, plan.ny, plan.nz);
    let dx = plan.dx;
    let dt = U_LATTICE * dx / speed;
    let nu_lat = case.air.kinematic_viscosity * dt / (dx * dx);
    let reynolds = case.air.reynolds(speed, info.length_m);
    let spl = (n_across as f64 / U_LATTICE).ceil() as usize;
    let ramp = spl / 2;
    let ground = match case.ground {
        GroundMode::Road => Ground::Slip,
        GroundMode::Moving => Ground::Moving,
        GroundMode::Fixed => Ground::Fixed,
        GroundMode::None => Ground::None,
    };
    let mut lbm = Lbm::new(
        &vox,
        info.mesh.part_names.len(),
        Config {
            nu: nu_lat,
            u_in: U_LATTICE,
            smagorinsky: case.smagorinsky.unwrap_or(DEFAULT_SMAGORINSKY),
            ground,
            ramp_steps: ramp,
        },
    );

    let wheels = if case.ground == GroundMode::Moving {
        spin_wheels(&mut lbm, &vox, case, &info.mesh.part_names)
    } else {
        0
    };
    if wheels > 0 {
        tx.send(Event::Log(format!("{wheels} rolling wheel(s) found"))).ok();
    }

    let conv = case.air.density * dx * dx * (speed / U_LATTICE).powi(2);
    let x_shift = plan.origin.x + 0.5 * dx;
    let to_sample = |f: &cfd_lbm::PartForce| Sample {
        fx: conv * f.fx,
        fy: conv * f.fy,
        fz: conv * f.fz,
        fz_x: conv * (dx * f.fz_x + x_shift * f.fz),
    };
    let area = case.reference_area_m2.unwrap_or(info.frontal_area_m2);
    let reference = Reference {
        density: case.air.density,
        speed,
        area,
        front_axle_x: case.front_axle_m.unwrap_or(0.0),
        rear_axle_x: case.rear_axle_m.unwrap_or(info.length_m),
    };
    let qa = reference.q() * area;

    let max_lengths = case.max_car_lengths.unwrap_or(10.0);
    let min_steps = ramp + 4 * spl;
    let max_steps = ramp + (max_lengths * spl as f64) as usize;
    let sample_every = (spl / 100).max(1);
    let samples_per_length = spl / sample_every;
    let mut totals: Vec<Sample> = Vec::new();
    let mut part_samples: Vec<Vec<Sample>> = Vec::new();
    let mut history: Vec<HistoryPoint> = Vec::new();
    let mut cd_series = Vec::new();
    let mut settled = false;
    let mut stopped = false;
    let mut last_send = Instant::now();
    let loop_start = Instant::now();

    while lbm.step < max_steps {
        if stop.load(Ordering::Relaxed) {
            stopped = true;
            break;
        }
        let st = lbm.advance();
        if !st.finite || st.max_speed > 0.4 {
            let c = st.max_speed_cell;
            let (i, j, k) = (c % lbm.nx, (c / lbm.nx) % lbm.ny, c / (lbm.nx * lbm.ny));
            let at = plan.origin + cfd_core::Vec3::new(i as f64 + 0.5, j as f64 + 0.5, k as f64 + 0.5) * dx;
            tx.send(Event::Log(format!(
                "Diagnostics: fastest cell at x {:.0} mm, y {:.0} mm, z {:.0} mm from the nose ({:.2} x lattice limit)",
                at.x * 1e3, at.y * 1e3, at.z * 1e3, st.max_speed / 0.4
            )))
            .ok();
            bail!(
                "The flow became unstable after {} steps. Try a higher quality setting, check for very thin \
                 parts or gaps in the geometry, or lower the speed.",
                st.step
            );
        }
        if st.step % sample_every == 0 && st.step >= ramp {
            let total = to_sample(&st.total());
            totals.push(total);
            part_samples.push(st.parts.iter().map(&to_sample).collect());
            let cd = total.fx / qa;
            let cl = total.fz / qa;
            cd_series.push(cd);
            history.push(HistoryPoint { step: st.step, time_s: st.step as f64 * dt, cd, cl });
            if st.step >= min_steps && has_settled(&cd_series, samples_per_length, 0.01) {
                settled = true;
                break;
            }
        }
        if last_send.elapsed().as_millis() > 200 {
            last_send = Instant::now();
            let secs = loop_start.elapsed().as_secs_f64();
            let last = history.last().cloned().unwrap_or_default();
            tx.send(Event::Progress(Progress {
                run,
                runs,
                label: label.clone(),
                step: st.step,
                max_steps,
                car_lengths: (st.step.saturating_sub(ramp)) as f64 / spl as f64,
                cd: last.cd,
                cl: last.cl,
                mlups: lbm.cells() as f64 * st.step as f64 / secs / 1e6,
                elapsed_s: started.elapsed().as_secs_f64(),
                history: history.clone(),
            }))
            .ok();
        }
    }
    if totals.is_empty() {
        bail!("Stopped before any forces were recorded.");
    }
    let window = (2 * samples_per_length).min(totals.len());
    let start = totals.len() - window;
    let summary = summarise(&totals[start..], &reference);
    let mut means = vec![Sample::default(); lbm.n_parts];
    for s in &part_samples[start..] {
        for (m, p) in means.iter_mut().zip(s) {
            m.fx += p.fx / window as f64;
            m.fy += p.fy / window as f64;
            m.fz += p.fz / window as f64;
            m.fz_x += p.fz_x / window as f64;
        }
    }
    let parts = per_part(&info.mesh.part_names, &means, &reference);
    let wall = started.elapsed().as_secs_f64();

    let mut warnings = info.warnings.clone();
    if !settled && !stopped {
        warnings.push(format!(
            "Cd had not settled to within 1% after {max_lengths} car lengths of flow; treat the numbers as rough."
        ));
    }
    if stopped {
        warnings.push("Stopped early by the user; numbers are from a partial run.".into());
    }
    if case.front_axle_m.is_none() || case.rear_axle_m.is_none() {
        warnings.push(
            "Aero balance uses the nose and tail as the axle positions. Set the axle positions for a true axle split."
                .into(),
        );
    }

    let slices = take_slices(&lbm, &vox, info);
    let result = RunResult {
        name: case.name.clone(),
        created_unix: crate::time::now_unix(),
        case: case.clone(),
        speed_mps: speed,
        cells_across_length: n_across,
        dims: [plan.nx, plan.ny, plan.nz],
        cells: plan.cells(),
        dx_m: dx,
        reynolds,
        steps: lbm.step,
        physical_time_s: lbm.step as f64 * dt,
        wall_time_s: wall,
        mlups: lbm.cells() as f64 * lbm.step as f64 / loop_start.elapsed().as_secs_f64() / 1e6,
        settled,
        stopped_early: stopped,
        length_m: info.length_m,
        width_m: info.width_m,
        height_m: info.height_m,
        part_names: info.mesh.part_names.clone(),
        reference,
        axles_defaulted: case.front_axle_m.is_none() || case.rear_axle_m.is_none(),
        area_measured: case.reference_area_m2.is_none(),
        summary,
        parts,
        history,
        grid_study: Vec::new(),
        grid_change_cd_pct: None,
        warnings,
        dir: None,
    };

    if case.write_vtk && lbm.cells() < 40_000_000 {
        let dir = case.runs_dir().join("latest-vtk");
        std::fs::create_dir_all(&dir).ok();
        let path: PathBuf = dir.join(format!("{}-{n_across}.vtk", sanitize(&case.name)));
        write_vtk(&lbm, &plan, speed, &path)?;
        tx.send(Event::Log(format!("Flow field written to {} (open in ParaView)", path.display()))).ok();
    }
    Ok(RunOutput { result, slices })
}

fn take_slices(lbm: &Lbm, vox: &cfd_geom::VoxelGrid, info: &GeometryInfo) -> Slices {
    let j_mid = ((0.0 - vox.origin.y) / vox.dx - 0.5).round().clamp(0.0, (lbm.ny - 1) as f64) as usize;
    let k_mid = ((info.height_m * 0.5 - vox.origin.z) / vox.dx - 0.5).round().clamp(0.0, (lbm.nz - 1) as f64) as usize;
    let speed_at = |c: usize| -> f32 {
        if lbm.flags[c] != FLUID {
            return -1.0;
        }
        let (_, u) = lbm.macroscopic(c);
        ((u[0] * u[0] + u[1] * u[1] + u[2] * u[2]).sqrt() / U_LATTICE as f32).max(0.0)
    };
    let mut side = Vec::with_capacity(lbm.nx * lbm.nz);
    for k in (0..lbm.nz).rev() {
        for i in 0..lbm.nx {
            side.push(speed_at(lbm.idx(i, j_mid, k)));
        }
    }
    let mut top = Vec::with_capacity(lbm.nx * lbm.ny);
    for j in (0..lbm.ny).rev() {
        for i in 0..lbm.nx {
            top.push(speed_at(lbm.idx(i, j, k_mid)));
        }
    }
    Slices {
        side: Slice { width: lbm.nx, height: lbm.nz, dx: vox.dx, values: side },
        top: Slice { width: lbm.nx, height: lbm.ny, dx: vox.dx, values: top },
    }
}

fn write_vtk(lbm: &Lbm, plan: &crate::prepare::DomainPlan, speed: f64, path: &std::path::Path) -> Result<()> {
    let n = lbm.cells();
    let scale = (speed / U_LATTICE) as f32;
    let q_lat = 0.5 * (U_LATTICE * U_LATTICE) as f32;
    let mut vel = Vec::with_capacity(n);
    let mut cp = Vec::with_capacity(n);
    let mut solid = Vec::with_capacity(n);
    for c in 0..n {
        let (rho, u) = lbm.macroscopic(c);
        let fluid = lbm.flags[c] == FLUID;
        vel.push(if fluid { u.map(|v| v * scale) } else { [0.0; 3] });
        cp.push(if fluid { (rho - 1.0) / 3.0 / q_lat } else { 0.0 });
        solid.push(if fluid { 0.0 } else { 1.0 });
    }
    let o = plan.origin;
    let h = 0.5 * plan.dx;
    write_structured_points(
        path,
        [lbm.nx, lbm.ny, lbm.nz],
        [o.x + h, o.y + h, o.z + h],
        plan.dx,
        &[Field::Vector("velocity", &vel), Field::Scalar("Cp", &cp), Field::Scalar("solid", &solid)],
    )
}

/// Finds each wheel (a connected blob of a wheel part's cells) and gives its
/// surface the velocity of a wheel rolling on the moving ground.
fn spin_wheels(lbm: &mut Lbm, vox: &cfd_geom::VoxelGrid, case: &Case, names: &[String]) -> usize {
    let wheel_part: Vec<bool> = names.iter().map(|n| case.is_wheel(n)).collect();
    if !wheel_part.iter().any(|&w| w) {
        return 0;
    }
    let (nx, ny, nz) = (vox.nx, vox.ny, vox.nz);
    let mut seen = vec![false; vox.cells.len()];
    let mut count = 0;
    for start in 0..vox.cells.len() {
        let v = vox.cells[start];
        if v == 0 || seen[start] || !wheel_part[(v - 1) as usize] {
            continue;
        }
        let mut stack = vec![start];
        let mut blob = Vec::new();
        seen[start] = true;
        while let Some(c) = stack.pop() {
            blob.push(c);
            let (i, j, k) = (c % nx, (c / nx) % ny, c / (nx * ny));
            let mut push = |n: usize| {
                if !seen[n] && vox.cells[n] == v {
                    seen[n] = true;
                    stack.push(n);
                }
            };
            if i > 0 { push(c - 1); }
            if i + 1 < nx { push(c + 1); }
            if j > 0 { push(c - nx); }
            if j + 1 < ny { push(c + nx); }
            if k > 0 { push(c - nx * ny); }
            if k + 1 < nz { push(c + nx * ny); }
        }
        let (mut i0, mut i1, mut k0, mut k1) = (usize::MAX, 0, usize::MAX, 0);
        for &c in &blob {
            let (i, k) = (c % nx, c / (nx * ny));
            i0 = i0.min(i);
            i1 = i1.max(i);
            k0 = k0.min(k);
            k1 = k1.max(k);
        }
        let cx = (i0 + i1 + 1) as f32 / 2.0;
        let cz = (k0 + k1 + 1) as f32 / 2.0;
        let r = ((k1 + 1 - k0) as f32 / 2.0).max(1.0);
        for &c in &blob {
            let (i, k) = (c % nx, c / (nx * ny));
            let (px, pz) = (i as f32 + 0.5, k as f32 + 0.5);
            lbm.set_wall_velocity(c, [(cz - pz) / r, 0.0, (px - cx) / r]);
        }
        count += 1;
    }
    count
}
