use anyhow::{Context, Result};
use cfd_core::{Air, LengthUnit};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Quality {
    #[default]
    Quick,
    Standard,
    Fine,
    /// 144 cells along the car: about 1.4 mm on a 200 mm car. Needs a GPU.
    Ultra,
    /// Three grids, for a grid-convergence check.
    Final,
}

impl Quality {
    pub fn cells_across_length(self) -> Vec<usize> {
        match self {
            Quality::Quick => vec![48],
            Quality::Standard => vec![72],
            Quality::Fine => vec![96],
            Quality::Ultra => vec![144],
            Quality::Final => vec![48, 72, 96],
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Quality::Quick => "Quick",
            Quality::Standard => "Standard",
            Quality::Fine => "Fine",
            Quality::Ultra => "Ultra (GPU)",
            Quality::Final => "Final (3 grids)",
        }
    }

    pub const ALL: [Quality; 5] = [Quality::Quick, Quality::Standard, Quality::Fine, Quality::Ultra, Quality::Final];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Device {
    /// The GPU when one is found, otherwise the CPU.
    #[default]
    Auto,
    Cpu,
    Gpu,
}

impl Device {
    pub fn label(self) -> &'static str {
        match self {
            Device::Auto => "Auto (GPU if found)",
            Device::Cpu => "CPU",
            Device::Gpu => "GPU",
        }
    }

    pub const ALL: [Device; 3] = [Device::Auto, Device::Cpu, Device::Gpu];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum GroundMode {
    /// Frictionless road: no boundary layer, like a moving road, and stable
    /// at every quality.
    #[default]
    Road,
    /// Moving belt with rolling wheels. Closest to reality, but gaps under
    /// the car must be several cells high or the numbers are wrong.
    Moving,
    /// Floor stands still, like a wind tunnel without a belt.
    Fixed,
    /// No floor: the body floats in open air.
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum UpAxis {
    #[default]
    Z,
    Y,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Nose {
    /// Car length along x, nose at the low-x end.
    #[default]
    MinusX,
    PlusX,
    MinusY,
    PlusY,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PartSpec {
    pub path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sweep {
    pub speeds_mps: Vec<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Case {
    pub name: String,
    pub geometry: Vec<PartSpec>,
    pub speed_mps: f64,
    #[serde(default)]
    pub quality: Quality,
    #[serde(default)]
    pub ground: GroundMode,
    #[serde(default)]
    pub device: Device,
    #[serde(default)]
    pub units: LengthUnit,
    #[serde(default)]
    pub up_axis: UpAxis,
    #[serde(default)]
    pub nose: Nose,
    #[serde(default)]
    pub air: Air,
    /// Frontal area for Cd and Cl. Measured from the geometry when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference_area_m2: Option<f64>,
    /// Axle positions measured from the nose, for aero balance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub front_axle_m: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rear_axle_m: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cells_across_length: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub smagorinsky: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_car_lengths: Option<f64>,
    /// Parts that roll on the ground. When empty, parts whose name contains
    /// wheel, tyre, tire or rim are used.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub wheel_parts: Vec<String>,
    #[serde(default = "yes")]
    pub write_vtk: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sweep: Option<Sweep>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_dir: Option<PathBuf>,
}

fn yes() -> bool {
    true
}

impl GroundMode {
    pub fn label(self) -> &'static str {
        match self {
            GroundMode::Road => "Road (frictionless floor)",
            GroundMode::Moving => "Moving belt + rolling wheels",
            GroundMode::Fixed => "Fixed floor (wind tunnel)",
            GroundMode::None => "No floor (free air)",
        }
    }

    pub const ALL: [GroundMode; 4] = [GroundMode::Road, GroundMode::Moving, GroundMode::Fixed, GroundMode::None];
}

impl Case {
    pub fn for_geometry(paths: &[PathBuf]) -> Case {
        let name = paths
            .first()
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "car".into());
        Case {
            name,
            geometry: paths.iter().map(|p| PartSpec { path: p.clone(), name: None }).collect(),
            speed_mps: 20.0,
            quality: Quality::Quick,
            ground: GroundMode::Road,
            device: Device::Auto,
            units: LengthUnit::Auto,
            up_axis: UpAxis::Z,
            nose: Nose::MinusX,
            air: Air::default(),
            reference_area_m2: None,
            front_axle_m: None,
            rear_axle_m: None,
            cells_across_length: None,
            smagorinsky: None,
            max_car_lengths: None,
            wheel_parts: Vec::new(),
            write_vtk: true,
            sweep: None,
            output_dir: None,
        }
    }

    pub fn load(path: &Path) -> Result<Case> {
        let text = std::fs::read_to_string(path).with_context(|| format!("could not open case file {}", path.display()))?;
        let mut case: Case = toml::from_str(&text).with_context(|| format!("{} has a mistake", path.display()))?;
        let base = path.parent().unwrap_or(Path::new("."));
        for part in &mut case.geometry {
            if part.path.is_relative() {
                part.path = base.join(&part.path);
            }
        }
        if let Some(out) = &case.output_dir {
            if out.is_relative() {
                case.output_dir = Some(base.join(out));
            }
        }
        Ok(case)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        std::fs::write(path, self.to_toml()?)?;
        Ok(())
    }

    pub fn to_toml(&self) -> Result<String> {
        Ok(toml::to_string_pretty(self)?)
    }

    pub fn grids(&self) -> Vec<usize> {
        match self.cells_across_length {
            Some(n) => vec![n],
            None => self.quality.cells_across_length(),
        }
    }

    pub fn speeds(&self) -> Vec<f64> {
        match &self.sweep {
            Some(s) if !s.speeds_mps.is_empty() => s.speeds_mps.clone(),
            _ => vec![self.speed_mps],
        }
    }

    pub fn is_wheel(&self, part_name: &str) -> bool {
        if !self.wheel_parts.is_empty() {
            return self.wheel_parts.iter().any(|w| w == part_name);
        }
        let n = part_name.to_lowercase();
        ["wheel", "tyre", "tire", "rim"].iter().any(|k| n.contains(k))
    }

    pub fn runs_dir(&self) -> PathBuf {
        self.output_dir.clone().unwrap_or_else(|| PathBuf::from("runs"))
    }

    /// Problems a newcomer could not guess from a crash, in plain words.
    pub fn check(&self) -> Vec<String> {
        let mut p = Vec::new();
        if self.geometry.is_empty() {
            p.push("Add a car file first (STL).".into());
        }
        if !(self.speed_mps > 0.0 && self.speed_mps < 150.0) {
            p.push(format!("Speed {} m/s is outside 0 to 150 m/s; this solver assumes low-speed air.", self.speed_mps));
        }
        if let Some(a) = self.reference_area_m2 {
            if a <= 0.0 {
                p.push("Reference area must be above zero.".into());
            }
        }
        if let (Some(f), Some(r)) = (self.front_axle_m, self.rear_axle_m) {
            if r <= f {
                p.push("Rear axle must be further back than the front axle.".into());
            }
        }
        p
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toml_round_trip_and_defaults() {
        let text = r#"
name = "car"
speed_mps = 20.0
[[geometry]]
path = "car.stl"
"#;
        let c: Case = toml::from_str(text).unwrap();
        assert_eq!(c.quality, Quality::Quick);
        assert_eq!(c.ground, GroundMode::Road);
        assert!(c.write_vtk);
        let back: Case = toml::from_str(&c.to_toml().unwrap()).unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn check_catches_bad_input() {
        let mut c = Case::for_geometry(&[]);
        c.speed_mps = -1.0;
        assert_eq!(c.check().len(), 2);
    }
}
