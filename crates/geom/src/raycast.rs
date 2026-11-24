//! Short-segment versus mesh intersection, used to find where the real
//! surface lies between a fluid cell and its solid neighbour.

use crate::mesh::Mesh;
use cfd_core::Vec3;
use std::collections::HashMap;

pub struct TriangleBins<'a> {
    mesh: &'a Mesh,
    origin: Vec3,
    dx: f64,
    bins: HashMap<(i32, i32, i32), Vec<u32>>,
}

impl<'a> TriangleBins<'a> {
    /// Bins triangles into cubes of size `dx` starting at `origin`.
    pub fn new(mesh: &'a Mesh, origin: Vec3, dx: f64) -> Self {
        let mut bins: HashMap<(i32, i32, i32), Vec<u32>> = HashMap::new();
        let cell = |v: f64, o: f64| ((v - o) / dx).floor() as i32;
        for (t, tri) in mesh.triangles.iter().enumerate() {
            let lo = tri.v[0].min(tri.v[1]).min(tri.v[2]);
            let hi = tri.v[0].max(tri.v[1]).max(tri.v[2]);
            for k in cell(lo.z, origin.z)..=cell(hi.z, origin.z) {
                for j in cell(lo.y, origin.y)..=cell(hi.y, origin.y) {
                    for i in cell(lo.x, origin.x)..=cell(hi.x, origin.x) {
                        bins.entry((i, j, k)).or_default().push(t as u32);
                    }
                }
            }
        }
        Self { mesh, origin, dx, bins }
    }

    /// Fraction t in [0, 1] along a -> b of the first surface crossing.
    pub fn first_hit(&self, a: Vec3, b: Vec3) -> Option<f64> {
        let cell = |v: f64, o: f64| ((v - o) / self.dx).floor() as i32;
        let lo = a.min(b);
        let hi = a.max(b);
        let d = b - a;
        let mut best: Option<f64> = None;
        for k in cell(lo.z, self.origin.z)..=cell(hi.z, self.origin.z) {
            for j in cell(lo.y, self.origin.y)..=cell(hi.y, self.origin.y) {
                for i in cell(lo.x, self.origin.x)..=cell(hi.x, self.origin.x) {
                    let Some(list) = self.bins.get(&(i, j, k)) else { continue };
                    for &t in list {
                        let tri = &self.mesh.triangles[t as usize];
                        if let Some(s) = segment_triangle(a, d, tri.v) {
                            if best.map_or(true, |b| s < b) {
                                best = Some(s);
                            }
                        }
                    }
                }
            }
        }
        best
    }
}

/// Möller–Trumbore, restricted to the segment a + s d, s in [0, 1].
fn segment_triangle(a: Vec3, d: Vec3, v: [Vec3; 3]) -> Option<f64> {
    let e1 = v[1] - v[0];
    let e2 = v[2] - v[0];
    let p = d.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-30 {
        return None;
    }
    let inv = 1.0 / det;
    let tv = a - v[0];
    let u = tv.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = tv.cross(e1);
    let w = d.dot(q) * inv;
    if w < 0.0 || u + w > 1.0 {
        return None;
    }
    let s = e2.dot(q) * inv;
    (0.0..=1.0).contains(&s).then_some(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::uv_sphere;

    #[test]
    fn finds_sphere_surface() {
        let m = Mesh::single("s", uv_sphere(Vec3::ZERO, 1.0, 64, 128));
        let bins = TriangleBins::new(&m, Vec3::new(-2.0, -2.0, -2.0), 0.25);
        let t = bins.first_hit(Vec3::new(1.5, 0.0, 0.0), Vec3::new(0.5, 0.0, 0.0)).unwrap();
        assert!((t - 0.5).abs() < 0.01, "{t}");
        assert!(bins.first_hit(Vec3::new(1.5, 0.0, 0.0), Vec3::new(1.2, 0.0, 0.0)).is_none());
    }
}
