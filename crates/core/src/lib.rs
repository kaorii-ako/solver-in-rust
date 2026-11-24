use serde::{Deserialize, Serialize};
use std::ops::{Add, AddAssign, Div, Mul, Neg, Sub};

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Vec3 {
    pub const ZERO: Vec3 = Vec3 { x: 0.0, y: 0.0, z: 0.0 };

    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    pub fn dot(self, o: Vec3) -> f64 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }

    pub fn cross(self, o: Vec3) -> Vec3 {
        Vec3::new(
            self.y * o.z - self.z * o.y,
            self.z * o.x - self.x * o.z,
            self.x * o.y - self.y * o.x,
        )
    }

    pub fn length(self) -> f64 {
        self.dot(self).sqrt()
    }

    pub fn min(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x.min(o.x), self.y.min(o.y), self.z.min(o.z))
    }

    pub fn max(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x.max(o.x), self.y.max(o.y), self.z.max(o.z))
    }

    pub fn max_component(self) -> f64 {
        self.x.max(self.y).max(self.z)
    }
}

impl Add for Vec3 {
    type Output = Vec3;
    fn add(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}

impl AddAssign for Vec3 {
    fn add_assign(&mut self, o: Vec3) {
        *self = *self + o;
    }
}

impl Sub for Vec3 {
    type Output = Vec3;
    fn sub(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}

impl Mul<f64> for Vec3 {
    type Output = Vec3;
    fn mul(self, s: f64) -> Vec3 {
        Vec3::new(self.x * s, self.y * s, self.z * s)
    }
}

impl Div<f64> for Vec3 {
    type Output = Vec3;
    fn div(self, s: f64) -> Vec3 {
        Vec3::new(self.x / s, self.y / s, self.z / s)
    }
}

impl Neg for Vec3 {
    type Output = Vec3;
    fn neg(self) -> Vec3 {
        Vec3::new(-self.x, -self.y, -self.z)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Aabb {
    pub fn empty() -> Self {
        Self {
            min: Vec3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY),
            max: Vec3::new(f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY),
        }
    }

    pub fn grow(&mut self, p: Vec3) {
        self.min = self.min.min(p);
        self.max = self.max.max(p);
    }

    pub fn size(&self) -> Vec3 {
        self.max - self.min
    }

    pub fn is_valid(&self) -> bool {
        self.min.x <= self.max.x && self.min.y <= self.max.y && self.min.z <= self.max.z
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Air {
    pub density: f64,
    pub kinematic_viscosity: f64,
}

impl Default for Air {
    fn default() -> Self {
        Self {
            density: 1.204,
            kinematic_viscosity: 1.516e-5,
        }
    }
}

impl Air {
    pub fn dynamic_pressure(&self, speed: f64) -> f64 {
        0.5 * self.density * speed * speed
    }

    pub fn reynolds(&self, speed: f64, length: f64) -> f64 {
        speed * length / self.kinematic_viscosity
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum LengthUnit {
    #[default]
    Auto,
    Mm,
    Cm,
    M,
    In,
}

impl LengthUnit {
    pub fn to_metres(self) -> f64 {
        match self {
            LengthUnit::Auto | LengthUnit::M => 1.0,
            LengthUnit::Mm => 0.001,
            LengthUnit::Cm => 0.01,
            LengthUnit::In => 0.0254,
        }
    }

    pub fn guess_from_extent(max_extent: f64) -> LengthUnit {
        if max_extent > 20.0 {
            LengthUnit::Mm
        } else {
            LengthUnit::M
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            LengthUnit::Auto => "auto",
            LengthUnit::Mm => "mm",
            LengthUnit::Cm => "cm",
            LengthUnit::M => "m",
            LengthUnit::In => "in",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cross_product_is_right_handed() {
        let z = Vec3::new(1.0, 0.0, 0.0).cross(Vec3::new(0.0, 1.0, 0.0));
        assert_eq!(z, Vec3::new(0.0, 0.0, 1.0));
    }

    #[test]
    fn reynolds_of_small_car() {
        let re = Air::default().reynolds(20.0, 0.2);
        assert!((re - 2.64e5).abs() < 0.01e5);
    }

    #[test]
    fn unit_guess() {
        assert_eq!(LengthUnit::guess_from_extent(210.0), LengthUnit::Mm);
        assert_eq!(LengthUnit::guess_from_extent(0.21), LengthUnit::M);
    }
}
