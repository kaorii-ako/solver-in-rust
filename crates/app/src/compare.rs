use crate::result::RunResult;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Better {
    Lower,
    Higher,
    Neither,
}

#[derive(Clone, Debug)]
pub struct Row {
    pub label: &'static str,
    pub unit: &'static str,
    pub a: f64,
    pub b: f64,
    pub better: Better,
}

impl Row {
    pub fn delta(&self) -> f64 {
        self.b - self.a
    }

    pub fn pct(&self) -> f64 {
        100.0 * self.delta() / self.a.abs().max(1e-30)
    }

    /// Some(true) when B improves on A for this number.
    pub fn b_is_better(&self) -> Option<bool> {
        let d = self.delta();
        if d.abs() < 1e-12 || !d.is_finite() {
            return None;
        }
        match self.better {
            Better::Lower => Some(d < 0.0),
            Better::Higher => Some(d > 0.0),
            Better::Neither => None,
        }
    }
}

pub fn rows(a: &RunResult, b: &RunResult) -> Vec<Row> {
    let (x, y) = (&a.summary, &b.summary);
    vec![
        Row { label: "Cd", unit: "", a: x.cd.mean, b: y.cd.mean, better: Better::Lower },
        Row { label: "Cl", unit: "", a: x.cl.mean, b: y.cl.mean, better: Better::Lower },
        Row { label: "Drag", unit: "N", a: x.drag_n.mean, b: y.drag_n.mean, better: Better::Lower },
        Row { label: "Downforce", unit: "N", a: x.downforce_n.mean, b: y.downforce_n.mean, better: Better::Higher },
        Row { label: "CdA", unit: "m²", a: x.cda_m2, b: y.cda_m2, better: Better::Lower },
        Row { label: "ClA", unit: "m²", a: x.cla_m2, b: y.cla_m2, better: Better::Lower },
        Row { label: "Efficiency -Cl/Cd", unit: "", a: x.efficiency, b: y.efficiency, better: Better::Higher },
        Row { label: "Front balance", unit: "%", a: x.front_balance_pct, b: y.front_balance_pct, better: Better::Neither },
        Row { label: "Drag power", unit: "W", a: x.drag_power_w, b: y.drag_power_w, better: Better::Lower },
    ]
}

pub fn text_table(a: &RunResult, b: &RunResult) -> String {
    let mut s = format!("A: {}\nB: {}\n\n", a.title(), b.title());
    s += &format!("{:<20} {:>12} {:>12} {:>10} {:>8}\n", "", "A", "B", "B - A", "");
    for r in rows(a, b) {
        let verdict = match r.b_is_better() {
            Some(true) => "better",
            Some(false) => "worse",
            None => "",
        };
        let label = if r.unit.is_empty() { r.label.to_string() } else { format!("{} ({})", r.label, r.unit) };
        s += &format!("{:<20} {:>12.4} {:>12.4} {:>+9.1}% {:>8}\n", label, r.a, r.b, r.pct(), verdict);
    }
    if a.speed_mps != b.speed_mps {
        s += "\nNote: the two runs used different speeds, so forces in N are not like for like.\n";
    }
    s
}
