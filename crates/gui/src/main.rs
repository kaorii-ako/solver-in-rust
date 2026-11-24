use cfd_app::{compare, image::slice_rgb, list_runs, run, Case, Event, GeometryInfo, GroundMode, Nose, Progress, Quality, RunResult, UpAxis};
use cfd_core::LengthUnit;
use cfd_geom::{silhouette, View};
use eframe::egui::{self, Color32, ColorImage, RichText, TextureHandle, TextureOptions};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

const PART_COLOURS: [Color32; 8] = [
    Color32::from_rgb(70, 130, 220),
    Color32::from_rgb(230, 120, 50),
    Color32::from_rgb(60, 170, 110),
    Color32::from_rgb(200, 70, 120),
    Color32::from_rgb(150, 110, 220),
    Color32::from_rgb(200, 170, 50),
    Color32::from_rgb(60, 170, 190),
    Color32::from_rgb(160, 160, 160),
];

#[derive(PartialEq, Eq, Clone, Copy)]
enum Tab {
    Car,
    Flow,
    Convergence,
    History,
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum SpeedUnit {
    Mps,
    Kmh,
}

type SilData = (String, [usize; 2], Vec<Color32>);

struct Loading {
    rx: std::sync::mpsc::Receiver<Result<(Arc<GeometryInfo>, Vec<SilData>), String>>,
    started: std::time::Instant,
    label: String,
}

struct Gui {
    gpu: Option<String>,
    gpu_rx: Option<std::sync::mpsc::Receiver<Option<String>>>,
    loading: Option<Loading>,
    autorun: bool,
    case: Case,
    case_path: Option<PathBuf>,
    geometry: Option<Arc<GeometryInfo>>,
    geometry_error: Option<String>,
    silhouettes: Vec<(String, TextureHandle, [usize; 2])>,
    handle: Option<run::RunHandle>,
    progress: Option<Progress>,
    result: Option<RunResult>,
    flow: Vec<(String, TextureHandle, [usize; 2])>,
    log: Vec<String>,
    error: Option<String>,
    tab: Tab,
    speed_unit: SpeedUnit,
    sweep_text: String,
    history: Vec<RunResult>,
    pick_a: Option<usize>,
    pick_b: Option<usize>,
}

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let autorun = args.iter().any(|a| a == "--run");
    let first = args.iter().find(|a| !a.starts_with("--")).map(PathBuf::from);
    eprintln!("cfd-gui: opening the window");
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1360.0, 860.0])
            .with_min_inner_size([960.0, 600.0])
            .with_drag_and_drop(true)
            .with_title("cfd: car aero"),
        ..Default::default()
    };
    let result = eframe::run_native(
        "cfd-gui",
        options,
        Box::new(move |cc| {
            let mut g = Gui::new();
            if let Some(p) = first {
                g.open_paths(&cc.egui_ctx, vec![p]);
                g.autorun = autorun;
            }
            Ok(Box::new(g))
        }),
    );
    if let Err(e) = &result {
        eprintln!("cfd-gui: could not open a window: {e}");
        eprintln!("If you are on Wayland, try: WAYLAND_DISPLAY= ./target/release/cfd-gui <file>");
    }
    result
}

impl Gui {
    fn new() -> Self {
        let case = Case::for_geometry(&[]);
        let history = list_runs(&case.runs_dir());
        let (gtx, grx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let name = cfd_app::gpu_name();
            match &name {
                Some(n) => eprintln!("cfd-gui: GPU found: {n}"),
                None => eprintln!("cfd-gui: no GPU found, runs use the CPU"),
            }
            gtx.send(name).ok();
        });
        Self {
            gpu: None,
            gpu_rx: Some(grx),
            loading: None,
            autorun: false,
            case,
            case_path: None,
            geometry: None,
            geometry_error: None,
            silhouettes: Vec::new(),
            handle: None,
            progress: None,
            result: None,
            flow: Vec::new(),
            log: Vec::new(),
            error: None,
            tab: Tab::Car,
            speed_unit: SpeedUnit::Mps,
            sweep_text: String::new(),
            history,
            pick_a: None,
            pick_b: None,
        }
    }

    fn running(&self) -> bool {
        self.handle.is_some()
    }

    fn open_paths(&mut self, ctx: &egui::Context, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        if let Some(toml) = paths.iter().find(|p| p.extension().and_then(|e| e.to_str()) == Some("toml")) {
            match Case::load(toml) {
                Ok(c) => {
                    self.case = c;
                    self.case_path = Some(toml.clone());
                    self.sweep_text = self
                        .case
                        .sweep
                        .as_ref()
                        .map(|s| s.speeds_mps.iter().map(|v| format!("{v}")).collect::<Vec<_>>().join(", "))
                        .unwrap_or_default();
                }
                Err(e) => {
                    self.error = Some(format!("{e:#}"));
                    return;
                }
            }
        } else {
            let keep = self.case.clone();
            self.case = Case::for_geometry(&paths);
            self.case.speed_mps = keep.speed_mps;
            self.case.quality = keep.quality;
            self.case.ground = keep.ground;
            self.case.output_dir = keep.output_dir;
            self.case_path = None;
        }
        self.reload_geometry(ctx);
        self.history = list_runs(&self.case.runs_dir());
        self.tab = Tab::Car;
    }

    fn reload_geometry(&mut self, _ctx: &egui::Context) {
        self.silhouettes.clear();
        self.geometry = None;
        self.geometry_error = None;
        let case = self.case.clone();
        let label = case
            .geometry
            .iter()
            .filter_map(|p| p.path.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        eprintln!("cfd-gui: reading {label}");
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let t = std::time::Instant::now();
            let out = cfd_app::load_geometry(&case).map_err(|e| format!("{e:#}")).map(|info| {
                let sils = [("Side", View::Side), ("Top", View::Top), ("Front", View::Front)]
                    .into_iter()
                    .map(|(label, view)| {
                        let s = silhouette(&info.mesh, view, 360);
                        let px = s
                            .filled
                            .iter()
                            .zip(&s.part)
                            .map(|(&f, &p)| if f { PART_COLOURS[p as usize % PART_COLOURS.len()] } else { Color32::TRANSPARENT })
                            .collect();
                        (label.to_string(), [s.width, s.height], px)
                    })
                    .collect();
                (Arc::new(info), sils)
            });
            match &out {
                Ok((info, _)) => eprintln!("cfd-gui: read in {:.1} s: {}", t.elapsed().as_secs_f64(), info.summary()),
                Err(e) => eprintln!("cfd-gui: could not read the geometry: {e}"),
            }
            tx.send(out).ok();
        });
        self.loading = Some(Loading { rx, started: std::time::Instant::now(), label });
    }

    fn poll_loading(&mut self, ctx: &egui::Context) {
        let Some(l) = &self.loading else { return };
        match l.rx.try_recv() {
            Ok(Ok((info, sils))) => {
                for (label, size, px) in sils {
                    let img = ColorImage { size, pixels: px };
                    let tex = ctx.load_texture(format!("sil-{label}"), img, TextureOptions::LINEAR);
                    self.silhouettes.push((label, tex, size));
                }
                self.geometry = Some(info);
                self.loading = None;
                if self.autorun {
                    self.autorun = false;
                    self.start_run();
                }
            }
            Ok(Err(e)) => {
                self.geometry_error = Some(e);
                self.loading = None;
                self.autorun = false;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(200)),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.geometry_error = Some("Reading the geometry crashed; run `cfd info <file>` in a terminal to see why.".into());
                self.loading = None;
            }
        }
    }

    fn start_run(&mut self) {
        let speeds: Vec<f64> = self
            .sweep_text
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter_map(|t| t.trim().parse::<f64>().ok())
            .filter(|v| *v > 0.0)
            .collect();
        self.case.sweep = if speeds.len() > 1 { Some(cfd_app::case::Sweep { speeds_mps: speeds }) } else { None };
        self.error = None;
        self.progress = None;
        self.log.clear();
        self.tab = Tab::Convergence;
        self.handle = Some(run::start_with(self.case.clone(), self.geometry.clone()));
    }

    fn poll(&mut self, ctx: &egui::Context) {
        let Some(h) = &self.handle else { return };
        let mut done = false;
        while let Ok(e) = h.events.try_recv() {
            match e {
                Event::Log(m) => self.log.push(m),
                Event::Geometry(_) => {}
                Event::Progress(p) => self.progress = Some(p),
                Event::RunDone(o) => {
                    self.flow.clear();
                    for (label, s) in [("Side view through the centreline", &o.slices.side), ("Top view at half height", &o.slices.top)] {
                        let rgb = slice_rgb(&s.values, 1.4);
                        let img = ColorImage {
                            size: [s.width, s.height],
                            pixels: rgb.iter().map(|c| Color32::from_rgb(c[0], c[1], c[2])).collect(),
                        };
                        let tex = ctx.load_texture(label, img, TextureOptions::NEAREST);
                        self.flow.push((label.to_string(), tex, [s.width, s.height]));
                    }
                    self.result = Some(o.result);
                }
                Event::Finished => done = true,
                Event::Failed(m) => {
                    self.error = Some(m);
                    done = true;
                }
            }
        }
        if done {
            if let Some(h) = self.handle.take() {
                h.thread.join().ok();
            }
            self.history = list_runs(&self.case.runs_dir());
            if self.result.is_some() && self.error.is_none() {
                self.tab = Tab::Flow;
            }
        }
        ctx.request_repaint_after(Duration::from_millis(100));
    }

    fn steps_bar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let ready = self.geometry.is_some();
            step_card(ui, "1  Car file", ready, |ui| {
                let label = if self.case.geometry.is_empty() {
                    "Open or drop a car file…".to_string()
                } else {
                    self.case.geometry.iter().filter_map(|p| p.path.file_name()).map(|n| n.to_string_lossy().to_string()).collect::<Vec<_>>().join(", ")
                };
                if ui.add_enabled(!self.running(), egui::Button::new(truncate(&label, 34))).clicked() {
                    if let Some(files) = rfd::FileDialog::new()
                        .add_filter("Car geometry or case", &["stl", "toml", "f3z", "f3d", "step", "stp"])
                        .pick_files()
                    {
                        self.open_paths(ui.ctx(), files);
                    }
                }
            });
            step_card(ui, "2  Speed", self.case.speed_mps > 0.0, |ui| {
                ui.add_enabled_ui(!self.running(), |ui| {
                    let (factor, suffix) = match self.speed_unit {
                        SpeedUnit::Mps => (1.0, " m/s"),
                        SpeedUnit::Kmh => (3.6, " km/h"),
                    };
                    let mut shown = self.case.speed_mps * factor;
                    if ui.add(egui::DragValue::new(&mut shown).range(1.0..=150.0 * factor).speed(0.1).suffix(suffix)).changed() {
                        self.case.speed_mps = shown / factor;
                    }
                    ui.selectable_value(&mut self.speed_unit, SpeedUnit::Mps, "m/s");
                    ui.selectable_value(&mut self.speed_unit, SpeedUnit::Kmh, "km/h");
                });
            });
            step_card(ui, "3  Quality", true, |ui| {
                ui.add_enabled_ui(!self.running(), |ui| {
                    egui::ComboBox::from_id_salt("quality")
                        .selected_text(self.case.quality.label())
                        .show_ui(ui, |ui| {
                            for q in Quality::ALL {
                                ui.selectable_value(&mut self.case.quality, q, q.label());
                            }
                        });
                });
                if let Some(info) = &self.geometry {
                    let n = *self.case.grids().last().unwrap_or(&48);
                    let p = cfd_app::plan_domain(info, n, self.case.ground);
                    let on = match (self.case.device, &self.gpu) {
                        (cfd_app::Device::Cpu, _) | (_, None) => "CPU".to_string(),
                        (cfd_app::Device::Auto, Some(g)) if g.contains("software") => "CPU".to_string(),
                        (_, Some(g)) => truncate(g.split(" (").next().unwrap_or(g), 18),
                    };
                    ui.label(RichText::new(format!("{:.1} M cells · {on}", p.cells() as f64 / 1e6)).weak());
                }
            });
            step_card(ui, "4  Run", false, |ui| {
                if self.running() {
                    if ui.add(egui::Button::new(RichText::new("■ Stop").size(16.0)).min_size([110.0, 30.0].into())).clicked() {
                        if let Some(h) = &self.handle {
                            h.request_stop();
                        }
                    }
                } else {
                    let can = self.geometry.is_some() && self.case.check().is_empty();
                    let btn = egui::Button::new(RichText::new("▶ Run").size(16.0).strong().color(Color32::WHITE))
                        .fill(Color32::from_rgb(47, 111, 214))
                        .min_size([110.0, 30.0].into());
                    if ui.add_enabled(can, btn).clicked() {
                        self.start_run();
                    }
                }
            });
        });
        ui.add_space(6.0);
    }

    fn setup_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Car");
        if let Some(e) = &self.geometry_error {
            ui.colored_label(Color32::from_rgb(220, 90, 70), e);
        }
        let mut reload = false;
        if let Some(info) = self.geometry.clone() {
            egui::Grid::new("car").num_columns(2).show(ui, |ui| {
                ui.label("Size");
                ui.label(format!("{:.1} × {:.1} × {:.1} mm", info.length_m * 1e3, info.width_m * 1e3, info.height_m * 1e3));
                ui.end_row();
                ui.label("Frontal area");
                ui.label(format!("{:.0} mm²", info.frontal_area_m2 * 1e6));
                ui.end_row();
                ui.label("Units");
                ui.label(format!("{}{}", info.unit_used.label(), if info.unit_was_guessed { " (guessed)" } else { "" }));
                ui.end_row();
                ui.label("Reynolds");
                ui.label(format!("{:.2e}", self.case.air.reynolds(self.case.speed_mps, info.length_m)));
                ui.end_row();
            });
            ui.add_space(4.0);
            for (i, (name, w)) in info.parts.iter().enumerate() {
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size([10.0, 10.0].into(), egui::Sense::hover());
                    ui.painter().rect_filled(r, 2.0, PART_COLOURS[i % PART_COLOURS.len()]);
                    ui.label(name);
                    if w.is_watertight() {
                        ui.label(RichText::new("closed").weak());
                    } else {
                        ui.colored_label(Color32::from_rgb(214, 154, 47), format!("{} open edges", w.open_edges));
                    }
                    if self.case.ground == GroundMode::Moving && self.case.is_wheel(name) {
                        ui.label(RichText::new("rolls").weak());
                    }
                });
            }
            for w in &info.warnings {
                ui.colored_label(Color32::from_rgb(214, 154, 47), format!("⚠ {w}"));
            }
        } else if self.loading.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Reading the car file…");
            });
        } else if self.geometry_error.is_none() {
            ui.label("Open your car (Fusion .f3z or .f3d, or STL), or drop it on this window. Each body or STL file becomes a part in the per-part breakdown.");
            if ui.button("Make a demo car to try").clicked() {
                let dir = std::env::temp_dir().join("cfd-demo-car");
                std::fs::create_dir_all(&dir).ok();
                let mut paths = Vec::new();
                for part in cfd_geom::demo::demo_car_parts() {
                    let p = dir.join(format!("{}.stl", part.part_names[0]));
                    if cfd_geom::stl::write_binary_stl(&p, &part.triangles).is_ok() {
                        paths.push(p);
                    }
                }
                self.open_paths(ui.ctx(), paths);
                self.case.front_axle_m = Some(0.038);
                self.case.rear_axle_m = Some(0.160);
            }
        }

        ui.separator();
        ui.heading("Floor");
        ui.add_enabled_ui(!self.running(), |ui| {
            for g in GroundMode::ALL {
                if ui.radio_value(&mut self.case.ground, g, g.label()).changed() {
                    reload = false;
                }
            }
        });
        if self.case.ground == GroundMode::Moving {
            ui.colored_label(
                Color32::from_rgb(214, 154, 47),
                "Gaps under the car must be several cells high for a moving belt to give right numbers. Use Fine quality or the Road floor.",
            );
        }

        ui.separator();
        ui.add_enabled_ui(!self.running(), |ui| {
            egui::CollapsingHeader::new("Advanced").default_open(false).show(ui, |ui| {
                egui::Grid::new("adv").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                    ui.label("Units");
                    egui::ComboBox::from_id_salt("units").selected_text(self.case.units.label()).show_ui(ui, |ui| {
                        for u in [LengthUnit::Auto, LengthUnit::Mm, LengthUnit::Cm, LengthUnit::M, LengthUnit::In] {
                            reload |= ui.selectable_value(&mut self.case.units, u, u.label()).changed();
                        }
                    });
                    ui.end_row();
                    ui.label("Up axis");
                    ui.horizontal(|ui| {
                        reload |= ui.selectable_value(&mut self.case.up_axis, UpAxis::Z, "Z").changed();
                        reload |= ui.selectable_value(&mut self.case.up_axis, UpAxis::Y, "Y").changed();
                    });
                    ui.end_row();
                    ui.label("Nose points");
                    egui::ComboBox::from_id_salt("nose").selected_text(format!("{:?}", self.case.nose)).show_ui(ui, |ui| {
                        for n in [Nose::MinusX, Nose::PlusX, Nose::MinusY, Nose::PlusY] {
                            reload |= ui.selectable_value(&mut self.case.nose, n, format!("{n:?}")).changed();
                        }
                    });
                    ui.end_row();
                    ui.label("Front axle (mm from nose)");
                    opt_mm(ui, &mut self.case.front_axle_m);
                    ui.end_row();
                    ui.label("Rear axle (mm from nose)");
                    opt_mm(ui, &mut self.case.rear_axle_m);
                    ui.end_row();
                    ui.label("Reference area (mm²)");
                    let mut on = self.case.reference_area_m2.is_some();
                    ui.horizontal(|ui| {
                        if ui.checkbox(&mut on, "").changed() {
                            self.case.reference_area_m2 = if on { self.geometry.as_ref().map(|g| g.frontal_area_m2) } else { None };
                        }
                        if let Some(a) = &mut self.case.reference_area_m2 {
                            let mut mm2 = *a * 1e6;
                            if ui.add(egui::DragValue::new(&mut mm2).range(1.0..=1e7)).changed() {
                                *a = mm2 / 1e6;
                            }
                        } else {
                            ui.label(RichText::new("measured").weak());
                        }
                    });
                    ui.end_row();
                    ui.label("Run on");
                    egui::ComboBox::from_id_salt("device").selected_text(self.case.device.label()).show_ui(ui, |ui| {
                        for d in cfd_app::Device::ALL {
                            ui.selectable_value(&mut self.case.device, d, d.label());
                        }
                    });
                    ui.end_row();
                    ui.label("Speed sweep (m/s)");
                    ui.add(egui::TextEdit::singleline(&mut self.sweep_text).hint_text("e.g. 15, 20, 25").desired_width(130.0));
                    ui.end_row();
                    ui.label("Cells across car");
                    let mut custom = self.case.cells_across_length.is_some();
                    ui.horizontal(|ui| {
                        if ui.checkbox(&mut custom, "").changed() {
                            self.case.cells_across_length = if custom { Some(64) } else { None };
                        }
                        if let Some(n) = &mut self.case.cells_across_length {
                            ui.add(egui::DragValue::new(n).range(16..=400));
                        } else {
                            ui.label(RichText::new("from quality").weak());
                        }
                    });
                    ui.end_row();
                    ui.label("Max run length");
                    let mut l = self.case.max_car_lengths.unwrap_or(10.0);
                    if ui.add(egui::DragValue::new(&mut l).range(2.0..=60.0).suffix(" car lengths")).changed() {
                        self.case.max_car_lengths = Some(l);
                    }
                    ui.end_row();
                    ui.label("Smagorinsky Cs");
                    let mut cs = self.case.smagorinsky.unwrap_or(run::DEFAULT_SMAGORINSKY);
                    if ui.add(egui::DragValue::new(&mut cs).range(0.0..=0.4).speed(0.005)).changed() {
                        self.case.smagorinsky = Some(cs);
                    }
                    ui.end_row();
                    ui.label("Air density");
                    ui.add(egui::DragValue::new(&mut self.case.air.density).range(0.5..=1.5).speed(0.001).suffix(" kg/m³"));
                    ui.end_row();
                    ui.label("Write ParaView file");
                    ui.checkbox(&mut self.case.write_vtk, "");
                    ui.end_row();
                });
            });
        });
        if reload {
            let ctx = ui.ctx().clone();
            self.reload_geometry(&ctx);
        }
        ui.separator();
        if ui.button("Save case file…").clicked() {
            if let Some(p) = rfd::FileDialog::new().set_file_name("case.toml").save_file() {
                if let Err(e) = self.case.save(&p) {
                    self.error = Some(format!("{e:#}"));
                } else {
                    self.case_path = Some(p);
                }
            }
        }
        if let Some(p) = &self.case_path {
            ui.label(RichText::new(p.display().to_string()).weak().small());
        }
    }

    fn results_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Results");
        if let Some(e) = &self.error {
            ui.colored_label(Color32::from_rgb(220, 90, 70), e);
            ui.separator();
        }
        if let Some(p) = &self.progress {
            if self.running() {
                ui.label(format!("Live  Cd {:.3}   Cl {:.3}", p.cd, p.cl));
                ui.label(RichText::new("Final numbers are averaged once the run settles.").weak());
                ui.separator();
            }
        }
        let Some(r) = self.result.clone() else {
            if !self.running() {
                ui.label("Press Run. Drag, downforce, balance and the per-part split appear here.");
            }
            return;
        };
        let s = &r.summary;
        ui.label(RichText::new(r.title()).weak().small());
        ui.add_space(4.0);
        big_number(ui, "Drag", &format!("{:.3} N", s.drag_n.mean), &format!("Cd {:.3} ± {:.3}", s.cd.mean, s.cd.std));
        big_number(ui, "Downforce", &format!("{:.3} N", s.downforce_n.mean), &format!("Cl {:.3} ± {:.3}", s.cl.mean, s.cl.std));
        egui::Grid::new("stats").num_columns(2).striped(true).show(ui, |ui| {
            row(ui, "Efficiency −Cl/Cd", format!("{:.2}", s.efficiency));
            row(ui, "Aero balance", r.balance_text());
            row(ui, "CdA", format!("{:.3e} m²", s.cda_m2));
            row(ui, "ClA", format!("{:.3e} m²", s.cla_m2));
            row(ui, "Drag power", format!("{:.2} W", s.drag_power_w));
            row(ui, "Side force", format!("{:.3} N", s.side_n.mean));
            row(ui, "Reference area", format!("{:.0} mm²{}", s.reference_area_m2 * 1e6, if r.area_measured { " (measured)" } else { "" }));
            if let Some(g) = r.grid_change_cd_pct {
                row(ui, "Grid change in Cd", format!("{g:.1}%"));
            }
            if let Some(t) = RunResult::gci_text(&r.gci_cd) {
                row(ui, "Cd, grid-independent", t);
            }
            if let Some(t) = RunResult::gci_text(&r.gci_cl) {
                row(ui, "Cl, grid-independent", t);
            }
            row(ui, "Settled", if r.settled { "yes".into() } else { "no".into() });
            if !r.device.is_empty() {
                row(ui, "Ran on", truncate(&r.device, 30));
            }
        });
        ui.add_space(6.0);
        ui.label(RichText::new("By part").strong());
        egui::Grid::new("parts").num_columns(4).striped(true).show(ui, |ui| {
            ui.label(RichText::new("Part").weak());
            ui.label(RichText::new("Drag N").weak());
            ui.label(RichText::new("Share").weak());
            ui.label(RichText::new("Down N").weak());
            ui.end_row();
            for (i, p) in r.parts.iter().enumerate() {
                ui.colored_label(PART_COLOURS[i % PART_COLOURS.len()], &p.name);
                ui.label(format!("{:.3}", p.drag_n));
                share_bar(ui, p.drag_share_pct);
                ui.label(format!("{:.3}", p.downforce_n));
                ui.end_row();
            }
        });
        if !r.warnings.is_empty() {
            ui.add_space(6.0);
            for w in &r.warnings {
                ui.label(RichText::new(format!("• {w}")).small().color(Color32::from_rgb(190, 140, 50)));
            }
        }
        ui.add_space(6.0);
        if let Some(dir) = &r.dir {
            ui.horizontal(|ui| {
                if ui.button("Open report").clicked() {
                    open_path(&dir.join("report.html"));
                }
                if ui.button("Open folder").clicked() {
                    open_path(dir);
                }
            });
        }
    }

    fn central(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.tab, Tab::Car, "Car");
            ui.selectable_value(&mut self.tab, Tab::Flow, "Flow");
            ui.selectable_value(&mut self.tab, Tab::Convergence, "Convergence");
            ui.selectable_value(&mut self.tab, Tab::History, "History & compare");
        });
        ui.separator();
        match self.tab {
            Tab::Car => self.car_tab(ui),
            Tab::Flow => self.flow_tab(ui),
            Tab::Convergence => self.convergence_tab(ui),
            Tab::History => self.history_tab(ui),
        }
    }

    fn car_tab(&mut self, ui: &mut egui::Ui) {
        if let Some(l) = &self.loading {
            ui.centered_and_justified(|ui| {
                ui.horizontal_centered(|ui| {
                    ui.spinner();
                    ui.label(
                        RichText::new(format!(
                            "Reading {} … {:.0} s\nFusion files are decoded and turned into triangles first; a whole car can take a minute or two.",
                            l.label,
                            l.started.elapsed().as_secs_f64()
                        ))
                        .size(16.0),
                    );
                });
            });
            return;
        }
        if self.silhouettes.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new("Drop a car file here\n(.f3z, .f3d or STL)").size(18.0).weak());
            });
            return;
        }
        ui.label(RichText::new("Air flows left to right in the side and top views. If the nose is on the right, change Advanced › Nose points.").weak());
        egui::ScrollArea::vertical().show(ui, |ui| {
            let w = ui.available_width();
            for (label, tex, size) in &self.silhouettes {
                ui.label(RichText::new(label.as_str()).strong());
                let scale = (w / size[0] as f32).min(if label == "Front" { 1.0 } else { 3.0 });
                ui.add(egui::Image::new((tex.id(), egui::vec2(size[0] as f32 * scale, size[1] as f32 * scale))));
                ui.add_space(8.0);
            }
        });
    }

    fn flow_tab(&mut self, ui: &mut egui::Ui) {
        if self.flow.is_empty() {
            ui.label("Flow pictures appear after a run.");
            return;
        }
        ui.horizontal(|ui| {
            ui.label("Speed relative to the free stream:");
            let (r, _) = ui.allocate_exact_size([220.0, 14.0].into(), egui::Sense::hover());
            for i in 0..110 {
                let t = i as f32 / 110.0;
                let c = cfd_app::image::colormap(t);
                let x0 = r.left() + r.width() * t;
                ui.painter().rect_filled(
                    egui::Rect::from_min_max([x0, r.top()].into(), [x0 + r.width() / 110.0 + 0.5, r.bottom()].into()),
                    0.0,
                    Color32::from_rgb(c[0], c[1], c[2]),
                );
            }
            ui.label("0 to 1.4 × U   (grey: car and floor)");
        });
        egui::ScrollArea::vertical().show(ui, |ui| {
            let w = ui.available_width();
            for (label, tex, size) in &self.flow {
                ui.label(RichText::new(label.as_str()).strong());
                let scale = w / size[0] as f32;
                ui.add(egui::Image::new((tex.id(), egui::vec2(w, size[1] as f32 * scale))));
                ui.add_space(8.0);
            }
            ui.label(RichText::new("For full 3D views open the .vtk file from runs/latest-vtk in ParaView.").weak());
        });
    }

    fn convergence_tab(&mut self, ui: &mut egui::Ui) {
        let hist = match (&self.progress, &self.result) {
            (Some(p), _) if self.running() => p.history.clone(),
            (_, Some(r)) => r.history.clone(),
            _ => Vec::new(),
        };
        if hist.is_empty() {
            ui.label(if self.running() { "Starting…" } else { "Cd and Cl are plotted here while a run goes." });
        }
        let cd: Vec<[f64; 2]> = hist.iter().map(|h| [h.time_s * 1e3, h.cd]).collect();
        let cl: Vec<[f64; 2]> = hist.iter().map(|h| [h.time_s * 1e3, h.cl]).collect();
        let log_h = 140.0;
        egui_plot::Plot::new("conv")
            .legend(egui_plot::Legend::default())
            .x_axis_label("flow time (ms)")
            .height((ui.available_height() - log_h).max(200.0))
            .show(ui, |pu| {
                pu.line(egui_plot::Line::new(egui_plot::PlotPoints::from(cd)).name("Cd").width(2.0_f32));
                pu.line(egui_plot::Line::new(egui_plot::PlotPoints::from(cl)).name("Cl").width(2.0_f32));
            });
        ui.label(RichText::new("Log").strong());
        egui::ScrollArea::vertical().stick_to_bottom(true).max_height(log_h - 24.0).show(ui, |ui| {
            for l in &self.log {
                ui.label(RichText::new(l).monospace().small());
            }
        });
    }

    fn history_tab(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button("Refresh").clicked() {
                self.history = list_runs(&self.case.runs_dir());
            }
            ui.label(RichText::new(format!("Saved runs in {}", self.case.runs_dir().display())).weak());
        });
        if self.history.is_empty() {
            ui.label("No saved runs yet.");
            return;
        }
        let mut load = None;
        egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
            egui::Grid::new("hist").num_columns(6).striped(true).show(ui, |ui| {
                ui.label(RichText::new("A").weak());
                ui.label(RichText::new("B").weak());
                ui.label(RichText::new("Run").weak());
                ui.label(RichText::new("Cd").weak());
                ui.label(RichText::new("Cl").weak());
                ui.label("");
                ui.end_row();
                for (i, r) in self.history.iter().enumerate() {
                    ui.radio_value(&mut self.pick_a, Some(i), "");
                    ui.radio_value(&mut self.pick_b, Some(i), "");
                    ui.label(r.title());
                    ui.label(format!("{:.3}", r.summary.cd.mean));
                    ui.label(format!("{:.3}", r.summary.cl.mean));
                    if ui.small_button("Show").clicked() {
                        load = Some(i);
                    }
                    ui.end_row();
                }
            });
        });
        if let Some(i) = load {
            let r = self.history[i].clone();
            if let Some(dir) = &r.dir {
                if let Ok(text) = std::fs::read_to_string(dir.join("slices.json")) {
                    if let Ok(sl) = serde_json_from_str(&text) {
                        self.show_slices(ui.ctx(), &sl);
                    }
                }
            }
            self.result = Some(r);
        }
        ui.separator();
        let (Some(a), Some(b)) = (self.pick_a, self.pick_b) else {
            ui.label("Pick an A and a B run to compare them.");
            return;
        };
        let (ra, rb) = (&self.history[a], &self.history[b]);
        ui.label(RichText::new(format!("A: {}", ra.title())).small());
        ui.label(RichText::new(format!("B: {}", rb.title())).small());
        egui::Grid::new("cmp").num_columns(5).striped(true).show(ui, |ui| {
            for h in ["", "A", "B", "B − A", ""] {
                ui.label(RichText::new(h).weak());
            }
            ui.end_row();
            for row in compare::rows(ra, rb) {
                let label = if row.unit.is_empty() { row.label.to_string() } else { format!("{} ({})", row.label, row.unit) };
                ui.label(label);
                ui.label(format!("{:.4}", row.a));
                ui.label(format!("{:.4}", row.b));
                ui.label(format!("{:+.1}%", row.pct()));
                match row.b_is_better() {
                    Some(true) => ui.colored_label(Color32::from_rgb(60, 170, 90), "better"),
                    Some(false) => ui.colored_label(Color32::from_rgb(210, 80, 70), "worse"),
                    None => ui.label(""),
                };
                ui.end_row();
            }
        });
        if ra.speed_mps != rb.speed_mps {
            ui.label(RichText::new("The runs used different speeds, so forces in N are not like for like; compare Cd and Cl.").weak());
        }
    }

    fn show_slices(&mut self, ctx: &egui::Context, sl: &cfd_app::run::Slices) {
        self.flow.clear();
        for (label, s) in [("Side view through the centreline", &sl.side), ("Top view at half height", &sl.top)] {
            if s.width == 0 {
                continue;
            }
            let rgb = slice_rgb(&s.values, 1.4);
            let img = ColorImage { size: [s.width, s.height], pixels: rgb.iter().map(|c| Color32::from_rgb(c[0], c[1], c[2])).collect() };
            let tex = ctx.load_texture(label, img, TextureOptions::NEAREST);
            self.flow.push((label.to_string(), tex, [s.width, s.height]));
        }
    }
}

fn serde_json_from_str(text: &str) -> anyhow::Result<cfd_app::run::Slices> {
    Ok(cfd_app::run::slices_from_json(text)?)
}

impl eframe::App for Gui {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
        if !dropped.is_empty() && !self.running() {
            self.open_paths(ctx, dropped);
        }
        if let Some(rx) = &self.gpu_rx {
            if let Ok(name) = rx.try_recv() {
                self.gpu = name;
                self.gpu_rx = None;
            }
        }
        self.poll_loading(ctx);
        self.poll(ctx);
        egui::TopBottomPanel::top("steps").show(ctx, |ui| self.steps_bar(ui));
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if let Some(p) = &self.progress {
                    let frac = (p.step as f32 / p.max_steps.max(1) as f32).clamp(0.0, 1.0);
                    let text = if self.running() {
                        format!("run {}/{} · {} · {:.1} car lengths · {:.0} M cells/s · {:.0} s", p.run, p.runs, p.label, p.car_lengths, p.mlups, p.elapsed_s)
                    } else {
                        "finished".into()
                    };
                    ui.add(egui::ProgressBar::new(if self.running() { frac } else { 1.0 }).text(text).desired_width(ui.available_width() * 0.7));
                } else {
                    ui.label(RichText::new("Ready").weak());
                }
                if let Some(l) = self.log.last() {
                    ui.label(RichText::new(truncate(l, 90)).weak().small());
                }
            });
        });
        egui::SidePanel::left("setup").resizable(true).default_width(310.0).min_width(260.0).show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| self.setup_panel(ui));
        });
        egui::SidePanel::right("results").resizable(true).default_width(330.0).min_width(300.0).show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| self.results_panel(ui));
        });
        egui::CentralPanel::default().show(ctx, |ui| self.central(ui));
    }
}

fn step_card(ui: &mut egui::Ui, title: &str, done: bool, body: impl FnOnce(&mut egui::Ui)) {
    let stroke = if done { Color32::from_rgb(60, 170, 90) } else { ui.visuals().widgets.noninteractive.bg_stroke.color };
    egui::Frame::group(ui.style()).stroke(egui::Stroke::new(1.5_f32, stroke)).rounding(8.0).inner_margin(8.0).show(ui, |ui| {
        ui.set_min_width(220.0);
        ui.vertical(|ui| {
            ui.label(RichText::new(title).strong());
            ui.horizontal(body);
        });
    });
}

fn big_number(ui: &mut egui::Ui, label: &str, value: &str, sub: &str) {
    egui::Frame::group(ui.style()).rounding(8.0).show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        ui.label(RichText::new(label).weak());
        ui.label(RichText::new(value).size(24.0).strong());
        ui.label(RichText::new(sub).weak());
    });
}

fn row(ui: &mut egui::Ui, a: &str, b: String) {
    ui.label(a);
    ui.label(b);
    ui.end_row();
}

fn share_bar(ui: &mut egui::Ui, pct: f64) {
    let (r, _) = ui.allocate_exact_size([70.0, 12.0].into(), egui::Sense::hover());
    ui.painter().rect_filled(r, 2.0, ui.visuals().faint_bg_color);
    let w = r.width() * (pct.clamp(0.0, 100.0) as f32 / 100.0);
    ui.painter().rect_filled(egui::Rect::from_min_size(r.min, [w, r.height()].into()), 2.0, Color32::from_rgb(47, 111, 214));
    ui.painter().text(r.right_center() + egui::vec2(4.0, 0.0), egui::Align2::LEFT_CENTER, format!("{pct:.0}%"), egui::FontId::proportional(11.0), ui.visuals().text_color());
    ui.add_space(26.0);
}

fn opt_mm(ui: &mut egui::Ui, v: &mut Option<f64>) {
    ui.horizontal(|ui| {
        let mut on = v.is_some();
        if ui.checkbox(&mut on, "").changed() {
            *v = if on { Some(0.0) } else { None };
        }
        if let Some(m) = v {
            let mut mm = *m * 1e3;
            if ui.add(egui::DragValue::new(&mut mm).range(0.0..=10000.0).speed(0.5)).changed() {
                *m = mm / 1e3;
            }
        } else {
            ui.label(RichText::new("nose / tail").weak());
        }
    });
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(n - 1).collect::<String>())
    }
}

fn open_path(p: &std::path::Path) {
    #[cfg(target_os = "windows")]
    let r = std::process::Command::new("cmd").args(["/C", "start", ""]).arg(p).spawn();
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg(p).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let r = std::process::Command::new("xdg-open").arg(p).spawn();
    let _ = r;
}
