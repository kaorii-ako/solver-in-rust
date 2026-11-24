use anyhow::{bail, Result};
use cfd_app::{compare, list_runs, load_geometry, plan_domain, run::run_blocking, Case, Event, RunResult};
use std::io::Write;
use std::path::{Path, PathBuf};

const HELP: &str = "cfd: external aerodynamics for small race cars

USAGE
  cfd run <case.toml>             run a case and save results under runs/
  cfd new <part.stl>... [-o case.toml]
                                  write a case file with sensible defaults
  cfd info <case.toml | part.stl> check the geometry and show the tunnel size
  cfd history [runs-dir]          list saved runs, newest first
  cfd show <run-dir>              print a saved run's statistics
  cfd compare <run-a> <run-b>     compare two saved runs side by side
  cfd demo [dir]                  write a demo F1-style car and its case file

The GUI is cfd-gui and the terminal dashboard is cfd-tui.";

fn main() {
    if let Err(e) = real_main() {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

fn case_from_arg(arg: &str) -> Result<Case> {
    let p = Path::new(arg);
    match p.extension().and_then(|e| e.to_str()) {
        Some("toml") => Case::load(p),
        _ => Ok(Case::for_geometry(&[p.to_path_buf()])),
    }
}

fn real_main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("help");
    match cmd {
        "run" => {
            let Some(path) = args.get(1) else { bail!("usage: cfd run <case.toml>") };
            run(case_from_arg(path)?)
        }
        "new" => {
            let mut parts = Vec::new();
            let mut out = PathBuf::from("case.toml");
            let mut it = args.iter().skip(1);
            while let Some(a) = it.next() {
                if a == "-o" {
                    out = it.next().map(PathBuf::from).unwrap_or(out);
                } else {
                    parts.push(PathBuf::from(a));
                }
            }
            if parts.is_empty() {
                bail!("usage: cfd new <part.stl>... [-o case.toml]");
            }
            let case = Case::for_geometry(&parts);
            let info = load_geometry(&case)?;
            println!("{}", info.summary());
            for w in &info.warnings {
                println!("warning: {w}");
            }
            case.save(&out)?;
            println!("wrote {}", out.display());
            Ok(())
        }
        "info" => {
            let Some(path) = args.get(1) else { bail!("usage: cfd info <case.toml | part.stl>") };
            info(case_from_arg(path)?)
        }
        "history" => {
            let dir = args.get(1).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("runs"));
            let runs = list_runs(&dir);
            if runs.is_empty() {
                println!("no saved runs in {}", dir.display());
            }
            for r in runs {
                println!("{}\n  {}\n  {}", r.title(), r.headline(), r.dir.as_ref().map(|d| d.display().to_string()).unwrap_or_default());
            }
            Ok(())
        }
        "show" => {
            let Some(path) = args.get(1) else { bail!("usage: cfd show <run-dir>") };
            print_result(&RunResult::load(Path::new(path))?);
            Ok(())
        }
        "compare" => {
            let (Some(a), Some(b)) = (args.get(1), args.get(2)) else { bail!("usage: cfd compare <run-a> <run-b>") };
            let (a, b) = (RunResult::load(Path::new(a))?, RunResult::load(Path::new(b))?);
            print!("{}", compare::text_table(&a, &b));
            Ok(())
        }
        "demo" => {
            let dir = args.get(1).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("cases/demo-car"));
            let case = write_demo(&dir)?;
            println!("wrote the demo car to {} and its case file to {}", dir.display(), dir.join("case.toml").display());
            println!("next: cfd run {}", dir.join("case.toml").display());
            drop(case);
            Ok(())
        }
        _ => {
            println!("{HELP}");
            Ok(())
        }
    }
}

pub fn write_demo(dir: &Path) -> Result<Case> {
    std::fs::create_dir_all(dir)?;
    let mut parts = Vec::new();
    for part in cfd_geom::demo::demo_car_parts() {
        let name = format!("{}.stl", part.part_names[0]);
        cfd_geom::stl::write_binary_stl(&dir.join(&name), &part.triangles)?;
        parts.push(PathBuf::from(name));
    }
    let mut case = Case::for_geometry(&parts);
    case.name = "demo-car".into();
    case.front_axle_m = Some(0.038);
    case.rear_axle_m = Some(0.160);
    case.output_dir = Some(PathBuf::from("../../runs"));
    case.save(&dir.join("case.toml"))?;
    Ok(case)
}

fn info(case: Case) -> Result<()> {
    let info = load_geometry(&case)?;
    println!("{}", info.summary());
    for (name, w) in &info.parts {
        println!("  part {name}: {}", if w.is_watertight() { "closed".to_string() } else { format!("{} open edges", w.open_edges) });
    }
    for w in &info.warnings {
        println!("warning: {w}");
    }
    println!("Reynolds number at {} m/s: {:.3e}", case.speed_mps, case.air.reynolds(case.speed_mps, info.length_m));
    for n in case.grids() {
        let p = plan_domain(&info, n, case.ground);
        println!(
            "  {n} cells across the car: {} x {} x {} = {:.2} M cells, {:.0} MB, {:.2} mm cells",
            p.nx,
            p.ny,
            p.nz,
            p.cells() as f64 / 1e6,
            p.memory_bytes() as f64 / 1e6,
            p.dx * 1e3
        );
    }
    Ok(())
}

fn run(case: Case) -> Result<()> {
    let mut last_line = 0usize;
    let outs = run_blocking(&case, |e| match e {
        Event::Log(m) => {
            if last_line > 0 {
                println!();
                last_line = 0;
            }
            println!("{m}");
        }
        Event::Progress(p) => {
            let line = format!(
                "\r  run {}/{}  step {:>6}/{}  {:>4.1} car lengths  Cd {:>7.4}  Cl {:>7.4}  {:>5.0} MLUPS",
                p.run, p.runs, p.step, p.max_steps, p.car_lengths, p.cd, p.cl, p.mlups
            );
            last_line = line.len();
            print!("{line}");
            std::io::stdout().flush().ok();
        }
        Event::RunDone(o) => {
            if last_line > 0 {
                println!();
                last_line = 0;
            }
            print_result(&o.result);
        }
        _ => {}
    })?;
    if outs.is_empty() {
        bail!("no runs finished");
    }
    Ok(())
}

fn print_result(r: &RunResult) {
    let s = &r.summary;
    println!("\n{}", r.title());
    println!("  Drag           {:>9.4} N    Cd  {:>7.4} ± {:.4}", s.drag_n.mean, s.cd.mean, s.cd.std);
    println!("  Downforce      {:>9.4} N    Cl  {:>7.4} ± {:.4}", s.downforce_n.mean, s.cl.mean, s.cl.std);
    println!("  CdA / ClA      {:>9.3e} / {:.3e} m²", s.cda_m2, s.cla_m2);
    println!("  Efficiency     {:>9.2}      (-Cl/Cd)", s.efficiency);
    println!(
        "  Aero balance   {:>9}      ({})",
        r.balance_text(),
        if r.axles_defaulted { "nose-to-tail" } else { "axles" }
    );
    println!("  Drag power     {:>9.3} W", s.drag_power_w);
    println!("  Parts:");
    for p in &r.parts {
        println!("    {:<14} drag {:>8.4} N ({:>3.0}%)  downforce {:>8.4} N", p.name, p.drag_n, p.drag_share_pct, p.downforce_n);
    }
    if let Some(g) = r.grid_change_cd_pct {
        println!("  Grid study: Cd moved {g:.1}% between the two finest grids");
    }
    if let Some(t) = RunResult::gci_text(&r.gci_cd) {
        println!("  Grid-independent Cd: {t}");
    }
    if let Some(t) = RunResult::gci_text(&r.gci_cl) {
        println!("  Grid-independent Cl: {t}");
    }
    for w in &r.warnings {
        println!("  note: {w}");
    }
    if let Some(d) = &r.dir {
        println!("  saved to {} (report.html)", d.display());
    }
}
