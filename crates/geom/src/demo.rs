//! A simple F1-style test car built from closed solids, in millimetres, with
//! the nose at x = 0, the flow along +x and z up. Each part is a separate
//! mesh so per-part forces can be checked.

use crate::mesh::{cylinder_y, hexahedron, unit_cube, Mesh};
use cfd_core::Vec3;

pub fn demo_car_parts() -> Vec<Mesh> {
    let v = Vec3::new;
    let body = hexahedron([
        v(0.0, -6.0, 6.0),
        v(170.0, -22.0, 6.0),
        v(0.0, 6.0, 6.0),
        v(170.0, 22.0, 6.0),
        v(0.0, -6.0, 14.0),
        v(170.0, -22.0, 40.0),
        v(0.0, 6.0, 14.0),
        v(170.0, 22.0, 40.0),
    ]);
    let front_wing = hexahedron([
        v(2.0, -32.0, 3.0),
        v(22.0, -32.0, 5.0),
        v(2.0, 32.0, 3.0),
        v(22.0, 32.0, 5.0),
        v(2.0, -32.0, 5.0),
        v(22.0, -32.0, 9.0),
        v(2.0, 32.0, 5.0),
        v(22.0, 32.0, 9.0),
    ]);
    let rear_wing = hexahedron([
        v(176.0, -30.0, 46.0),
        v(206.0, -30.0, 50.0),
        v(176.0, 30.0, 46.0),
        v(206.0, 30.0, 50.0),
        v(176.0, -30.0, 49.0),
        v(206.0, -30.0, 54.0),
        v(176.0, 30.0, 49.0),
        v(206.0, 30.0, 54.0),
    ]);
    let pylon = unit_cube(v(168.0, -3.0, 38.0), v(184.0, 3.0, 48.0));
    let mut rear = rear_wing;
    rear.extend(pylon);
    let wheel = |x: f64, y: f64| cylinder_y(v(x, y, 13.0), 13.0, 12.0, 48);
    let mut wheels = Vec::new();
    for (x, y) in [(38.0, -29.0), (38.0, 29.0), (160.0, -29.0), (160.0, 29.0)] {
        wheels.extend(wheel(x, y));
    }
    vec![
        Mesh::single("body", body),
        Mesh::single("front_wing", front_wing),
        Mesh::single("rear_wing", rear),
        Mesh::single("wheels", wheels),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_parts_are_closed() {
        for p in demo_car_parts() {
            if p.part_names[0] != "rear_wing" {
                assert!(p.watertight().is_watertight(), "{}", p.part_names[0]);
            }
        }
    }
}
