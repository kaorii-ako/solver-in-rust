//! Turns lattice forces into the numbers an aero engineer reads.
//!
//! Sign conventions: x points downstream (drag is +x), z points up, so
//! downforce is -Fz and Cl is negative when the car is pushed down.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Sample {
    pub fx: f64,
    pub fy: f64,
    pub fz: f64,
    pub fz_x: f64,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct Reference {
    pub density: f64,
    pub speed: f64,
    pub area: f64,
    pub front_axle_x: f64,
    pub rear_axle_x: f64,
}

impl Reference {
    pub fn q(&self) -> f64 {
        0.5 * self.density * self.speed * self.speed
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct MeanStd {
    pub mean: f64,
    pub std: f64,
}

impl MeanStd {
    pub fn of(values: &[f64]) -> Self {
        if values.is_empty() {
            return Self::default();
        }
        let n = values.len() as f64;
        let mean = values.iter().sum::<f64>() / n;
        let var = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n;
        Self { mean, std: var.sqrt() }
    }
}

mod nan_as_null {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &f64, s: S) -> Result<S::Ok, S::Error> {
        if v.is_finite() {
            s.serialize_f64(*v)
        } else {
            s.serialize_none()
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
        Ok(Option::<f64>::deserialize(d)?.unwrap_or(f64::NAN))
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct AeroSummary {
    // front_balance_pct is NaN when the car makes almost no lift or downforce,
    // because the centre of pressure is then undefined.
    pub drag_n: MeanStd,
    pub downforce_n: MeanStd,
    pub side_n: MeanStd,
    pub cd: MeanStd,
    pub cl: MeanStd,
    pub cda_m2: f64,
    pub cla_m2: f64,
    #[serde(with = "nan_as_null")]
    pub efficiency: f64,
    #[serde(with = "nan_as_null")]
    pub front_balance_pct: f64,
    #[serde(with = "nan_as_null")]
    pub centre_of_pressure_x: f64,
    pub drag_power_w: f64,
    pub reference_area_m2: f64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct PartSummary {
    pub name: String,
    pub drag_n: f64,
    pub downforce_n: f64,
    pub cd: f64,
    pub cl: f64,
    pub drag_share_pct: f64,
}

pub fn summarise(samples: &[Sample], r: &Reference) -> AeroSummary {
    let qa = r.q() * r.area;
    let drag: Vec<f64> = samples.iter().map(|s| s.fx).collect();
    let down: Vec<f64> = samples.iter().map(|s| -s.fz).collect();
    let side: Vec<f64> = samples.iter().map(|s| s.fy).collect();
    let cd: Vec<f64> = drag.iter().map(|d| d / qa).collect();
    let cl: Vec<f64> = samples.iter().map(|s| s.fz / qa).collect();
    let mean_fz = samples.iter().map(|s| s.fz).sum::<f64>() / samples.len().max(1) as f64;
    let mean_fz_x = samples.iter().map(|s| s.fz_x).sum::<f64>() / samples.len().max(1) as f64;
    let x_cp = if mean_fz.abs() > 1e-30 { mean_fz_x / mean_fz } else { f64::NAN };
    let wheelbase = r.rear_axle_x - r.front_axle_x;
    let cl_stats = MeanStd::of(&cl);
    let lift_is_clear = cl_stats.mean.abs() >= 0.05 && cl_stats.mean.abs() > cl_stats.std;
    let front = if wheelbase.abs() > 1e-12 && x_cp.is_finite() && lift_is_clear {
        100.0 * (r.rear_axle_x - x_cp) / wheelbase
    } else {
        f64::NAN
    };
    let cd = MeanStd::of(&cd);
    let cl = cl_stats;
    let drag = MeanStd::of(&drag);
    AeroSummary {
        drag_n: drag,
        downforce_n: MeanStd::of(&down),
        side_n: MeanStd::of(&side),
        cd,
        cl,
        cda_m2: cd.mean * r.area,
        cla_m2: cl.mean * r.area,
        efficiency: if cd.mean.abs() > 1e-12 { -cl.mean / cd.mean } else { f64::NAN },
        front_balance_pct: front,
        centre_of_pressure_x: x_cp,
        drag_power_w: drag.mean * r.speed,
        reference_area_m2: r.area,
    }
}

pub fn per_part(names: &[String], mean_forces: &[Sample], r: &Reference) -> Vec<PartSummary> {
    let qa = r.q() * r.area;
    let total_drag: f64 = mean_forces.iter().map(|s| s.fx).sum();
    names
        .iter()
        .zip(mean_forces)
        .map(|(name, s)| PartSummary {
            name: name.clone(),
            drag_n: s.fx,
            downforce_n: -s.fz,
            cd: s.fx / qa,
            cl: s.fz / qa,
            drag_share_pct: if total_drag.abs() > 1e-30 { 100.0 * s.fx / total_drag } else { 0.0 },
        })
        .collect()
}

/// Relative change of the last value against the one before, in percent.
/// With three grids this is the "how much does the answer still move" number.
pub fn grid_change_pct(values_coarse_to_fine: &[f64]) -> Option<f64> {
    let n = values_coarse_to_fine.len();
    if n < 2 {
        return None;
    }
    let (a, b) = (values_coarse_to_fine[n - 2], values_coarse_to_fine[n - 1]);
    Some(100.0 * (b - a).abs() / b.abs().max(1e-30))
}

/// True once the running mean of the last `window` values differs from the
/// mean of the window before it by less than `tol` (relative).
pub fn has_settled(series: &[f64], window: usize, tol: f64) -> bool {
    if window == 0 || series.len() < 2 * window {
        return false;
    }
    let n = series.len();
    let last = series[n - window..].iter().sum::<f64>() / window as f64;
    let prev = series[n - 2 * window..n - window].iter().sum::<f64>() / window as f64;
    (last - prev).abs() <= tol * last.abs().max(1e-12)
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    fn reference() -> Reference {
        Reference { density: 1.2, speed: 20.0, area: 0.01, front_axle_x: 0.0, rear_axle_x: 0.2 }
    }

    #[test]
    fn coefficients_from_forces() {
        let r = reference();
        let qa = 240.0 * 0.01;
        let s = Sample { fx: 1.2, fy: 0.0, fz: -2.4, fz_x: -2.4 * 0.05 };
        let a = summarise(&[s, s], &r);
        assert_relative_eq!(a.cd.mean, 1.2 / qa);
        assert_relative_eq!(a.cl.mean, -2.4 / qa);
        assert_relative_eq!(a.downforce_n.mean, 2.4);
        assert_relative_eq!(a.efficiency, 2.0);
        assert_relative_eq!(a.drag_power_w, 24.0);
        assert_relative_eq!(a.centre_of_pressure_x, 0.05);
        assert_relative_eq!(a.front_balance_pct, 75.0);
        assert_relative_eq!(a.cda_m2, a.cd.mean * 0.01);
    }

    #[test]
    fn balance_is_undefined_without_lift() {
        let r = reference();
        let s = Sample { fx: 1.2, fy: 0.0, fz: -0.01, fz_x: 0.003 };
        assert!(summarise(&[s], &r).front_balance_pct.is_nan());
    }

    #[test]
    fn nan_survives_json() {
        let a = AeroSummary { front_balance_pct: f64::NAN, ..Default::default() };
        let back: AeroSummary = serde_json::from_str(&serde_json::to_string(&a).unwrap()).unwrap();
        assert!(back.front_balance_pct.is_nan());
    }

    #[test]
    fn spread_of_unsteady_samples() {
        let m = MeanStd::of(&[1.0, 3.0]);
        assert_relative_eq!(m.mean, 2.0);
        assert_relative_eq!(m.std, 1.0);
    }

    #[test]
    fn parts_share_drag() {
        let r = reference();
        let names = vec!["body".to_string(), "wing".to_string()];
        let f = [Sample { fx: 3.0, ..Default::default() }, Sample { fx: 1.0, ..Default::default() }];
        let p = per_part(&names, &f, &r);
        assert_relative_eq!(p[0].drag_share_pct, 75.0);
    }

    #[test]
    fn settling_detects_flat_series() {
        let flat: Vec<f64> = (0..100).map(|i| 1.0 + 0.5 * (-(i as f64) / 5.0).exp()).collect();
        assert!(has_settled(&flat, 20, 0.005));
        let ramp: Vec<f64> = (0..100).map(|i| i as f64).collect();
        assert!(!has_settled(&ramp, 20, 0.005));
    }
}
