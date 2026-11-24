use crate::case::Case;
use anyhow::{Context, Result};
use cfd_aero::{AeroSummary, PartSummary, Reference};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct HistoryPoint {
    pub step: usize,
    pub time_s: f64,
    pub cd: f64,
    pub cl: f64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct GridPoint {
    pub cells_across_length: usize,
    pub cells: usize,
    pub cd: f64,
    pub cl: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RunResult {
    pub name: String,
    pub created_unix: u64,
    pub case: Case,
    pub speed_mps: f64,
    pub cells_across_length: usize,
    pub dims: [usize; 3],
    pub cells: usize,
    pub dx_m: f64,
    pub reynolds: f64,
    pub steps: usize,
    pub physical_time_s: f64,
    pub wall_time_s: f64,
    pub mlups: f64,
    pub settled: bool,
    pub stopped_early: bool,
    pub length_m: f64,
    pub width_m: f64,
    pub height_m: f64,
    pub part_names: Vec<String>,
    pub reference: Reference,
    pub axles_defaulted: bool,
    pub area_measured: bool,
    pub summary: AeroSummary,
    pub parts: Vec<PartSummary>,
    pub history: Vec<HistoryPoint>,
    #[serde(default)]
    pub grid_study: Vec<GridPoint>,
    #[serde(default)]
    pub grid_change_cd_pct: Option<f64>,
    /// Richardson extrapolation and uncertainty from the three Final grids.
    #[serde(default)]
    pub gci_cd: Option<cfd_aero::GridConvergence>,
    #[serde(default)]
    pub gci_cl: Option<cfd_aero::GridConvergence>,
    #[serde(default)]
    pub interpolated_walls: bool,
    #[serde(default)]
    pub device: String,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub dir: Option<PathBuf>,
}

impl RunResult {
    pub fn title(&self) -> String {
        format!(
            "{} · {:.1} m/s · {} · {}",
            self.name,
            self.speed_mps,
            self.case.quality.label(),
            crate::time::format_utc(self.created_unix)
        )
    }

    pub fn save(&self, dir: &Path) -> Result<()> {
        std::fs::create_dir_all(dir)?;
        std::fs::write(dir.join("result.json"), serde_json::to_string_pretty(self)?)?;
        self.case.save(&dir.join("case.toml"))?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<RunResult> {
        let file = if path.is_dir() { path.join("result.json") } else { path.to_path_buf() };
        let text = std::fs::read_to_string(&file).with_context(|| format!("could not open {}", file.display()))?;
        let mut r: RunResult = serde_json::from_str(&text).with_context(|| format!("{} is not a run result", file.display()))?;
        r.dir = file.parent().map(Path::to_path_buf);
        Ok(r)
    }

    /// "0.312 ± 1.8%" style grid-independent estimate, when a 3-grid study ran.
    pub fn gci_text(g: &Option<cfd_aero::GridConvergence>) -> Option<String> {
        let g = g.as_ref()?;
        Some(if g.monotonic {
            format!("{:.4} ± {:.1}%", g.extrapolated, g.gci_fine_pct)
        } else {
            "not converging smoothly".into()
        })
    }

    pub fn balance_text(&self) -> String {
        let b = self.summary.front_balance_pct;
        if b.is_finite() {
            format!("{b:.0}% front")
        } else {
            "n/a (almost no lift)".into()
        }
    }

    /// One-line summary for logs and the TUI.
    pub fn headline(&self) -> String {
        let s = &self.summary;
        format!(
            "Cd {:.3}  Cl {:.3}  drag {:.3} N  downforce {:.3} N  -Cl/Cd {:.2}  balance {}",
            s.cd.mean,
            s.cl.mean,
            s.drag_n.mean,
            s.downforce_n.mean,
            s.efficiency,
            self.balance_text()
        )
    }
}

/// Every saved run under `runs_dir`, newest first.
pub fn list_runs(runs_dir: &Path) -> Vec<RunResult> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(runs_dir) {
        for e in entries.flatten() {
            if e.path().join("result.json").exists() {
                if let Ok(r) = RunResult::load(&e.path()) {
                    out.push(r);
                }
            }
        }
    }
    out.sort_by(|a, b| b.created_unix.cmp(&a.created_unix).then(b.name.cmp(&a.name)));
    out
}
