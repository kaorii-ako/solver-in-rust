use crate::image::{base64, bmp, slice_rgb};
use crate::result::RunResult;
use crate::run::{Slice, Slices};
use std::fmt::Write;

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn img(slice: &Slice, alt: &str) -> String {
    if slice.width == 0 {
        return String::new();
    }
    let data = bmp(slice.width, slice.height, &slice_rgb(&slice.values, 1.4));
    format!(
        "<img alt=\"{alt}\" src=\"data:image/bmp;base64,{}\" style=\"width:100%;image-rendering:pixelated\">",
        base64(&data)
    )
}

fn history_svg(r: &RunResult) -> String {
    let h = &r.history;
    if h.len() < 2 {
        return String::new();
    }
    let (w, ht, pad) = (640.0, 220.0, 36.0);
    let t0 = h[0].time_s;
    let t1 = h[h.len() - 1].time_s.max(t0 + 1e-12);
    let vals = h.iter().flat_map(|p| [p.cd, p.cl]);
    let lo = vals.clone().fold(f64::INFINITY, f64::min).min(0.0);
    let hi = vals.fold(f64::NEG_INFINITY, f64::max).max(0.0);
    let span = (hi - lo).max(1e-9);
    let x = |t: f64| pad + (t - t0) / (t1 - t0) * (w - 2.0 * pad);
    let y = |v: f64| ht - pad - (v - lo) / span * (ht - 2.0 * pad);
    let line = |f: &dyn Fn(&crate::result::HistoryPoint) -> f64| {
        h.iter().map(|p| format!("{:.1},{:.1}", x(p.time_s), y(f(p)))).collect::<Vec<_>>().join(" ")
    };
    format!(
        r##"<svg viewBox="0 0 {w} {ht}" role="img" aria-label="Cd and Cl against time" style="width:100%;max-width:640px">
<line x1="{pad}" x2="{x2}" y1="{y0:.1}" y2="{y0:.1}" stroke="currentColor" stroke-opacity=".3"/>
<polyline fill="none" stroke="#2f6fd6" stroke-width="1.5" points="{cd}"/>
<polyline fill="none" stroke="#d65a2f" stroke-width="1.5" points="{cl}"/>
<text x="{pad}" y="20" font-size="12" fill="#2f6fd6">Cd</text>
<text x="{pad2}" y="20" font-size="12" fill="#d65a2f">Cl</text>
<text x="{x2}" y="{yb}" font-size="11" text-anchor="end" fill="currentColor">{t1:.3} s of flow</text>
<text x="{pad}" y="{yb}" font-size="11" fill="currentColor">0</text>
</svg>"##,
        x2 = w - pad,
        y0 = y(0.0),
        cd = line(&|p| p.cd),
        cl = line(&|p| p.cl),
        pad2 = pad + 30.0,
        yb = ht - 10.0,
    )
}

pub fn html(r: &RunResult, slices: &Slices) -> String {
    let s = &r.summary;
    let mut o = String::new();
    let _ = write!(
        o,
        r#"<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>{name} aero report</title>
<style>
:root{{--bg:#fff;--fg:#1b1c1f;--muted:#5d6068;--line:#d9dade;--accent:#2f6fd6}}
@media (prefers-color-scheme:dark){{:root{{--bg:#16171a;--fg:#ececef;--muted:#a3a6ad;--line:#33353b}}}}
body{{background:var(--bg);color:var(--fg);font:15px/1.5 system-ui,sans-serif;max-width:860px;margin:32px auto;padding:0 16px}}
h1{{font-size:24px;margin:0 0 4px}} h2{{font-size:17px;margin:28px 0 8px}}
.muted{{color:var(--muted)}} table{{border-collapse:collapse;width:100%}}
td,th{{border-bottom:1px solid var(--line);padding:6px 8px;text-align:right}} td:first-child,th:first-child{{text-align:left}}
.big{{display:grid;grid-template-columns:repeat(auto-fit,minmax(150px,1fr));gap:12px}}
.card{{border:1px solid var(--line);border-radius:8px;padding:12px}} .card b{{font-size:22px;display:block}}
.warn{{border-left:3px solid #d69a2f;padding:4px 10px;margin:6px 0}}
</style></head><body>
<h1>{name}</h1><div class="muted">{speed:.1} m/s · {quality} · {date} · Re {re:.2e}</div>
<h2>Headline numbers</h2><div class="big">
<div class="card">Drag<b>{drag:.3} N</b><span class="muted">Cd {cd:.3} ± {cds:.3}</span></div>
<div class="card">Downforce<b>{down:.3} N</b><span class="muted">Cl {cl:.3} ± {cls:.3}</span></div>
<div class="card">Efficiency −Cl/Cd<b>{eff:.2}</b></div>
<div class="card">Aero balance<b>{bal}</b><span class="muted">{baln}</span></div>
<div class="card">Drag power<b>{pow:.2} W</b></div>
</div>
<h2>All statistics</h2><table>
<tr><th>Statistic</th><th>Value</th></tr>
<tr><td>Cd (mean ± std)</td><td>{cd:.4} ± {cds:.4}</td></tr>
<tr><td>Cl (mean ± std, negative = downforce)</td><td>{cl:.4} ± {cls:.4}</td></tr>
<tr><td>CdA</td><td>{cda:.3e} m²</td></tr>
<tr><td>ClA</td><td>{cla:.3e} m²</td></tr>
<tr><td>Side force</td><td>{side:.4} N</td></tr>
<tr><td>Centre of pressure from nose</td><td>{xcp}</td></tr>
<tr><td>Reference area ({areasrc})</td><td>{area:.0} mm²</td></tr>
</table>
"#,
        name = esc(&r.name),
        speed = r.speed_mps,
        quality = r.case.quality.label(),
        date = crate::time::format_utc(r.created_unix),
        re = r.reynolds,
        drag = s.drag_n.mean,
        cd = s.cd.mean,
        cds = s.cd.std,
        down = s.downforce_n.mean,
        cl = s.cl.mean,
        cls = s.cl.std,
        eff = s.efficiency,
        bal = r.balance_text(),
        baln = if r.axles_defaulted { "nose-to-tail split" } else { "axle split" },
        pow = s.drag_power_w,
        cda = s.cda_m2,
        cla = s.cla_m2,
        side = s.side_n.mean,
        xcp = if s.front_balance_pct.is_finite() { format!("{:.1} mm", s.centre_of_pressure_x * 1e3) } else { "n/a".into() },
        areasrc = if r.area_measured { "measured from the geometry" } else { "set by the user" },
        area = s.reference_area_m2 * 1e6,
    );
    o += "<h2>By part</h2><table><tr><th>Part</th><th>Drag N</th><th>Downforce N</th><th>Cd</th><th>Cl</th><th>Share of drag</th></tr>";
    for p in &r.parts {
        let _ = write!(
            o,
            "<tr><td>{}</td><td>{:.4}</td><td>{:.4}</td><td>{:.4}</td><td>{:.4}</td><td>{:.0}%</td></tr>",
            esc(&p.name),
            p.drag_n,
            p.downforce_n,
            p.cd,
            p.cl,
            p.drag_share_pct
        );
    }
    o += "</table>";
    if !r.grid_study.is_empty() {
        o += "<h2>Grid convergence</h2><table><tr><th>Cells across car</th><th>Total cells</th><th>Cd</th><th>Cl</th></tr>";
        for g in &r.grid_study {
            let _ = write!(o, "<tr><td>{}</td><td>{}</td><td>{:.4}</td><td>{:.4}</td></tr>", g.cells_across_length, g.cells, g.cd, g.cl);
        }
        o += "</table>";
        if let Some(p) = r.grid_change_cd_pct {
            let _ = write!(o, "<p>Cd changed by {p:.1}% between the two finest grids.</p>");
        }
    }
    let _ = write!(o, "<h2>Convergence</h2>{}", history_svg(r));
    let _ = write!(
        o,
        "<h2>Flow speed, side view through the centreline</h2>{}<h2>Flow speed, top view at half height</h2>{}<p class=\"muted\">Blue is slow air, red is faster than the free stream (scale 0 to 1.4 × U). Grey is the car and the floor.</p>",
        img(&slices.side, "side view of flow speed"),
        img(&slices.top, "top view of flow speed")
    );
    if !r.warnings.is_empty() {
        o += "<h2>Notes</h2>";
        for w in &r.warnings {
            let _ = write!(o, "<div class=\"warn\">{}</div>", esc(w));
        }
    }
    let _ = write!(
        o,
        "<h2>Settings</h2><table>\
<tr><td>Car size L × W × H</td><td>{:.1} × {:.1} × {:.1} mm</td></tr>\
<tr><td>Ground</td><td>{:?}</td></tr>\
<tr><td>Grid</td><td>{} × {} × {} = {:.2} M cells, {:.2} mm per cell</td></tr>\
<tr><td>Steps / flow time</td><td>{} / {:.4} s</td></tr>\
<tr><td>Settled</td><td>{}</td></tr>\
<tr><td>Wall time</td><td>{:.0} s at {:.0} M cell updates/s</td></tr>\
<tr><td>Air</td><td>{} kg/m³, ν = {:.3e} m²/s</td></tr>\
</table><p class=\"muted\">Lattice Boltzmann D3Q19, regularized BGK collision with Smagorinsky LES, bounce-back walls, momentum-exchange forces.</p></body></html>",
        r.length_m * 1e3,
        r.width_m * 1e3,
        r.height_m * 1e3,
        r.case.ground,
        r.dims[0],
        r.dims[1],
        r.dims[2],
        r.cells as f64 / 1e6,
        r.dx_m * 1e3,
        r.steps,
        r.physical_time_s,
        if r.settled { "yes" } else { "no" },
        r.wall_time_s,
        r.mlups,
        r.case.air.density,
        r.case.air.kinematic_viscosity
    );
    o
}
