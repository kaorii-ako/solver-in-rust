use anyhow::{bail, Result};
use cfd_app::{run, Case, Event, GroundMode, Progress, Quality, RunResult};
use crossterm::event::{self, Event as TermEvent, KeyCode, KeyEventKind};
use ratatui::prelude::*;
use ratatui::symbols::Marker;
use ratatui::widgets::{Axis, Block, Borders, Cell, Chart, Dataset, Gauge, GraphType, Paragraph, Row, Table, Wrap};
use std::path::Path;
use std::time::Duration;

struct App {
    case: Case,
    handle: Option<cfd_app::RunHandle>,
    progress: Option<Progress>,
    result: Option<RunResult>,
    log: Vec<String>,
    status: String,
    quit: bool,
}

fn load_case(arg: Option<&String>) -> Result<Case> {
    let path = match arg {
        Some(a) => a.clone(),
        None if Path::new("case.toml").exists() => "case.toml".into(),
        None => bail!("usage: cfd-tui <case.toml | car.stl>\n\nKeys: r run, s stop, +/- speed, g quality, f floor, q quit"),
    };
    let p = Path::new(&path);
    if p.extension().and_then(|e| e.to_str()) == Some("toml") {
        Case::load(p)
    } else {
        Ok(Case::for_geometry(&[p.to_path_buf()]))
    }
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let case = match load_case(args.first()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e:#}");
            std::process::exit(1);
        }
    };
    let mut terminal = ratatui::init();
    let mut app = App {
        case,
        handle: None,
        progress: None,
        result: None,
        log: vec!["Press r to run.".into()],
        status: "ready".into(),
        quit: false,
    };
    let res = run_loop(&mut terminal, &mut app);
    if let Some(h) = &app.handle {
        h.request_stop();
    }
    ratatui::restore();
    res
}

fn run_loop(terminal: &mut ratatui::DefaultTerminal, app: &mut App) -> Result<()> {
    while !app.quit {
        drain(app);
        terminal.draw(|f| draw(f, app))?;
        if event::poll(Duration::from_millis(150))? {
            if let TermEvent::Key(k) = event::read()? {
                if k.kind == KeyEventKind::Press {
                    key(app, k.code);
                }
            }
        }
    }
    Ok(())
}

fn running(app: &App) -> bool {
    app.handle.is_some()
}

fn key(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Char('q') | KeyCode::Esc => app.quit = true,
        KeyCode::Char('r') if !running(app) => {
            app.progress = None;
            app.status = "running".into();
            app.log.push(format!("Starting {} at {:.1} m/s, {}", app.case.name, app.case.speed_mps, app.case.quality.label()));
            app.handle = Some(run::start(app.case.clone()));
        }
        KeyCode::Char('s') => {
            if let Some(h) = &app.handle {
                h.request_stop();
                app.status = "stopping".into();
            }
        }
        KeyCode::Char('+') | KeyCode::Char('=') if !running(app) => app.case.speed_mps += 1.0,
        KeyCode::Char('-') if !running(app) => app.case.speed_mps = (app.case.speed_mps - 1.0).max(1.0),
        KeyCode::Char('g') if !running(app) => {
            let i = Quality::ALL.iter().position(|q| *q == app.case.quality).unwrap_or(0);
            app.case.quality = Quality::ALL[(i + 1) % Quality::ALL.len()];
        }
        KeyCode::Char('f') if !running(app) => {
            let i = GroundMode::ALL.iter().position(|g| *g == app.case.ground).unwrap_or(0);
            app.case.ground = GroundMode::ALL[(i + 1) % GroundMode::ALL.len()];
        }
        _ => {}
    }
}

fn drain(app: &mut App) {
    let Some(h) = &app.handle else { return };
    let mut done = false;
    while let Ok(e) = h.events.try_recv() {
        match e {
            Event::Log(m) => app.log.push(m),
            Event::Geometry(_) => {}
            Event::Progress(p) => app.progress = Some(p),
            Event::RunDone(o) => {
                if let Some(d) = &o.result.dir {
                    app.log.push(format!("Saved {}", d.join("report.html").display()));
                }
                app.result = Some(o.result);
            }
            Event::Finished => {
                app.status = "done".into();
                done = true;
            }
            Event::Failed(m) => {
                app.status = "failed".into();
                for line in m.lines() {
                    app.log.push(format!("Error: {line}"));
                }
                done = true;
            }
        }
    }
    if done {
        if let Some(h) = app.handle.take() {
            h.thread.join().ok();
        }
    }
    let keep = app.log.len().saturating_sub(200);
    app.log.drain(..keep);
}

fn draw(f: &mut Frame, app: &App) {
    let rows = Layout::vertical([Constraint::Length(3), Constraint::Min(10), Constraint::Length(9)]).split(f.area());
    let header = format!(
        " {}  ·  {:.1} m/s  ·  {}  ·  {}  ·  {}",
        app.case.name,
        app.case.speed_mps,
        app.case.quality.label(),
        app.case.ground.label(),
        app.status
    );
    f.render_widget(
        Paragraph::new(header).block(Block::default().borders(Borders::ALL).title(" cfd ")),
        rows[0],
    );
    let mid = Layout::horizontal([Constraint::Percentage(58), Constraint::Percentage(42)]).split(rows[1]);
    let left = Layout::vertical([Constraint::Min(6), Constraint::Length(3)]).split(mid[0]);
    draw_chart(f, app, left[0]);
    let (ratio, label) = match &app.progress {
        Some(p) => (
            (p.step as f64 / p.max_steps.max(1) as f64).clamp(0.0, 1.0),
            format!(
                "run {}/{} · {:.1} car lengths · {:.0} M cells/s · {:.0} s",
                p.run, p.runs, p.car_lengths, p.mlups, p.elapsed_s
            ),
        ),
        None => (0.0, "not started".into()),
    };
    f.render_widget(
        Gauge::default()
            .block(Block::default().borders(Borders::ALL).title(" progress (stops early once Cd settles) "))
            .gauge_style(Style::default().fg(Color::Cyan))
            .ratio(ratio)
            .label(label),
        left[1],
    );
    draw_stats(f, app, mid[1]);
    let n = rows[2].height.saturating_sub(2) as usize;
    let lines: Vec<Line> = app.log.iter().rev().take(n).rev().map(|l| Line::raw(l.clone())).collect();
    f.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" log ")
                .title_bottom(" r run · s stop · +/- speed · g quality · f floor · q quit "),
        ),
        rows[2],
    );
}

fn draw_chart(f: &mut Frame, app: &App, area: Rect) {
    let hist = match (&app.progress, &app.result) {
        (Some(p), _) if running(app) => p.history.clone(),
        (_, Some(r)) => r.history.clone(),
        (Some(p), None) => p.history.clone(),
        _ => Vec::new(),
    };
    let cd: Vec<(f64, f64)> = hist.iter().map(|h| (h.time_s * 1e3, h.cd)).collect();
    let cl: Vec<(f64, f64)> = hist.iter().map(|h| (h.time_s * 1e3, h.cl)).collect();
    let t1 = cd.last().map(|p| p.0).unwrap_or(1.0).max(1e-6);
    let all = cd.iter().chain(&cl).map(|p| p.1);
    let lo = all.clone().fold(0.0f64, f64::min);
    let hi = all.fold(0.0f64, f64::max);
    let pad = ((hi - lo) * 0.1).max(0.05);
    let ds = vec![
        Dataset::default().name("Cd").marker(Marker::Braille).graph_type(GraphType::Line).style(Style::default().fg(Color::Cyan)).data(&cd),
        Dataset::default().name("Cl").marker(Marker::Braille).graph_type(GraphType::Line).style(Style::default().fg(Color::LightRed)).data(&cl),
    ];
    let chart = Chart::new(ds)
        .block(Block::default().borders(Borders::ALL).title(" Cd and Cl against flow time "))
        .x_axis(Axis::default().title("ms").bounds([0.0, t1]).labels(vec!["0".to_string(), format!("{t1:.1}")]))
        .y_axis(
            Axis::default()
                .bounds([lo - pad, hi + pad])
                .labels(vec![format!("{:.2}", lo - pad), format!("{:.2}", hi + pad)]),
        );
    f.render_widget(chart, area);
}

fn draw_stats(f: &mut Frame, app: &App, area: Rect) {
    let mut rows: Vec<Row> = Vec::new();
    let head = Style::default().add_modifier(Modifier::BOLD);
    if let Some(r) = &app.result {
        let s = &r.summary;
        let stat = |a: &str, b: String| Row::new(vec![Cell::from(a.to_string()), Cell::from(b)]);
        rows.push(stat("Drag", format!("{:.3} N", s.drag_n.mean)));
        rows.push(stat("Downforce", format!("{:.3} N", s.downforce_n.mean)));
        rows.push(stat("Cd", format!("{:.3} ± {:.3}", s.cd.mean, s.cd.std)));
        rows.push(stat("Cl", format!("{:.3} ± {:.3}", s.cl.mean, s.cl.std)));
        rows.push(stat("CdA", format!("{:.2e} m²", s.cda_m2)));
        rows.push(stat("ClA", format!("{:.2e} m²", s.cla_m2)));
        rows.push(stat("-Cl/Cd", format!("{:.2}", s.efficiency)));
        rows.push(stat("Balance", r.balance_text()));
        rows.push(stat("Drag power", format!("{:.2} W", s.drag_power_w)));
        if let Some(g) = r.grid_change_cd_pct {
            rows.push(stat("Grid change", format!("{g:.1}% in Cd")));
        }
        rows.push(Row::new(vec![Cell::from(""), Cell::from("")]));
        rows.push(Row::new(vec![Cell::from("Part"), Cell::from("drag N · share · down N")]).style(head));
        for p in &r.parts {
            rows.push(Row::new(vec![
                Cell::from(p.name.clone()),
                Cell::from(format!("{:.3} · {:>3.0}% · {:.3}", p.drag_n, p.drag_share_pct, p.downforce_n)),
            ]));
        }
    } else if let Some(p) = &app.progress {
        rows.push(Row::new(vec![Cell::from("Cd (live)"), Cell::from(format!("{:.3}", p.cd))]));
        rows.push(Row::new(vec![Cell::from("Cl (live)"), Cell::from(format!("{:.3}", p.cl))]));
        rows.push(Row::new(vec![Cell::from(""), Cell::from("final numbers appear when the run ends")]));
    } else {
        rows.push(Row::new(vec![Cell::from("Status"), Cell::from("no results yet: press r to run")]));
    }
    f.render_widget(
        Table::new(rows, [Constraint::Length(12), Constraint::Min(10)])
            .block(Block::default().borders(Borders::ALL).title(" results ")),
        area,
    );
}
