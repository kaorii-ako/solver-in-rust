use cfd_core::{Aabb, Vec3};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Triangle {
    pub v: [Vec3; 3],
}

impl Triangle {
    pub fn normal(&self) -> Vec3 {
        (self.v[1] - self.v[0]).cross(self.v[2] - self.v[0])
    }
}

#[derive(Clone, Debug, Default)]
pub struct Mesh {
    pub triangles: Vec<Triangle>,
    pub part_of: Vec<u16>,
    pub part_names: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct WatertightReport {
    pub edges: usize,
    pub open_edges: usize,
    pub overshared_edges: usize,
    pub open_edge_points: Vec<Vec3>,
}

impl WatertightReport {
    pub fn is_watertight(&self) -> bool {
        self.open_edges == 0 && self.overshared_edges == 0
    }
}

impl Mesh {
    pub fn single(name: &str, triangles: Vec<Triangle>) -> Self {
        let n = triangles.len();
        Self {
            triangles,
            part_of: vec![0; n],
            part_names: vec![name.to_string()],
        }
    }

    pub fn merge(parts: Vec<Mesh>) -> Mesh {
        let mut out = Mesh::default();
        for part in parts {
            let offset = out.part_names.len() as u16;
            out.part_names.extend(part.part_names);
            out.part_of.extend(part.part_of.iter().map(|p| p + offset));
            out.triangles.extend(part.triangles);
        }
        out
    }

    pub fn bounds(&self) -> Aabb {
        let mut b = Aabb::empty();
        for t in &self.triangles {
            for v in t.v {
                b.grow(v);
            }
        }
        b
    }

    pub fn map_vertices(&mut self, f: impl Fn(Vec3) -> Vec3) {
        for t in &mut self.triangles {
            for v in &mut t.v {
                *v = f(*v);
            }
        }
    }

    pub fn scale(&mut self, s: f64) {
        self.map_vertices(|v| v * s);
    }

    pub fn translate(&mut self, d: Vec3) {
        self.map_vertices(|v| v + d);
    }

    pub fn y_up_to_z_up(&mut self) {
        self.map_vertices(|v| Vec3::new(v.x, -v.z, v.y));
    }

    pub fn rotate_z_180(&mut self) {
        self.map_vertices(|v| Vec3::new(-v.x, -v.y, v.z));
    }

    pub fn swap_x_y(&mut self) {
        self.map_vertices(|v| Vec3::new(v.y, -v.x, v.z));
    }

    pub fn signed_volume(&self) -> f64 {
        self.triangles
            .iter()
            .map(|t| t.v[0].dot(t.v[1].cross(t.v[2])) / 6.0)
            .sum()
    }

    pub fn watertight(&self) -> WatertightReport {
        let scale = self.bounds().size().max_component().max(1e-12);
        let q = |v: Vec3| {
            let k = 1e7 / scale;
            (
                (v.x * k).round() as i64,
                (v.y * k).round() as i64,
                (v.z * k).round() as i64,
            )
        };
        let mut edges: HashMap<((i64, i64, i64), (i64, i64, i64)), (usize, Vec3)> = HashMap::new();
        for t in &self.triangles {
            for e in 0..3 {
                let a = t.v[e];
                let b = t.v[(e + 1) % 3];
                let (qa, qb) = (q(a), q(b));
                if qa == qb {
                    continue;
                }
                let key = if qa < qb { (qa, qb) } else { (qb, qa) };
                let entry = edges.entry(key).or_insert((0, (a + b) * 0.5));
                entry.0 += 1;
            }
        }
        let mut report = WatertightReport {
            edges: edges.len(),
            ..Default::default()
        };
        for (count, mid) in edges.values() {
            if *count == 1 {
                report.open_edges += 1;
                if report.open_edge_points.len() < 200 {
                    report.open_edge_points.push(*mid);
                }
            } else if *count > 2 {
                report.overshared_edges += 1;
            }
        }
        report
    }
}

pub fn unit_cube(min: Vec3, max: Vec3) -> Vec<Triangle> {
    let p = |i: usize| {
        Vec3::new(
            if i & 1 == 0 { min.x } else { max.x },
            if i & 2 == 0 { min.y } else { max.y },
            if i & 4 == 0 { min.z } else { max.z },
        )
    };
    hexahedron([p(0), p(1), p(2), p(3), p(4), p(5), p(6), p(7)])
}

/// Corners in cube order: bit 0 = +x, bit 1 = +y, bit 2 = +z.
pub fn hexahedron(c: [Vec3; 8]) -> Vec<Triangle> {
    let quads = [
        [0, 2, 3, 1],
        [4, 5, 7, 6],
        [0, 1, 5, 4],
        [2, 6, 7, 3],
        [0, 4, 6, 2],
        [1, 3, 7, 5],
    ];
    let mut tris = Vec::new();
    for q in quads {
        tris.push(Triangle { v: [c[q[0]], c[q[1]], c[q[2]]] });
        tris.push(Triangle { v: [c[q[0]], c[q[2]], c[q[3]]] });
    }
    tris
}

pub fn cylinder_y(centre: Vec3, radius: f64, width: f64, segments: usize) -> Vec<Triangle> {
    use std::f64::consts::PI;
    let (y0, y1) = (centre.y - width / 2.0, centre.y + width / 2.0);
    let rim = |i: usize, y: f64| {
        let a = 2.0 * PI * (i % segments) as f64 / segments as f64;
        Vec3::new(centre.x + radius * a.cos(), y, centre.z + radius * a.sin())
    };
    let c0 = Vec3::new(centre.x, y0, centre.z);
    let c1 = Vec3::new(centre.x, y1, centre.z);
    let mut tris = Vec::new();
    for i in 0..segments {
        let (a0, b0, a1, b1) = (rim(i, y0), rim(i + 1, y0), rim(i, y1), rim(i + 1, y1));
        tris.push(Triangle { v: [a0, b0, b1] });
        tris.push(Triangle { v: [a0, b1, a1] });
        tris.push(Triangle { v: [c0, b0, a0] });
        tris.push(Triangle { v: [c1, a1, b1] });
    }
    tris
}

/// Which axis the silhouette looks along.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    /// Looking along x (front view): image axes are y (right) and z (up).
    Front,
    /// Looking along y (side view): image axes are x (right) and z (up).
    Side,
    /// Looking along z (top view): image axes are x (right) and y (up).
    Top,
}

#[derive(Clone, Debug)]
pub struct Silhouette {
    pub width: usize,
    pub height: usize,
    pub pixel: f64,
    pub origin: (f64, f64),
    pub filled: Vec<bool>,
    pub part: Vec<u16>,
}

impl Silhouette {
    pub fn area(&self) -> f64 {
        self.filled.iter().filter(|&&f| f).count() as f64 * self.pixel * self.pixel
    }
}

/// Rasterises the mesh projected along one axis. Row 0 is the top of the image.
pub fn silhouette(mesh: &Mesh, view: View, max_pixels: usize) -> Silhouette {
    let b = mesh.bounds();
    let pick = |v: Vec3| match view {
        View::Front => (v.y, v.z),
        View::Side => (v.x, v.z),
        View::Top => (v.x, v.y),
    };
    let (u0, v0) = pick(b.min);
    let (u1, v1) = pick(b.max);
    let span = (u1 - u0).max(v1 - v0).max(1e-12);
    let pixel = span / max_pixels as f64;
    let width = (((u1 - u0) / pixel).ceil() as usize).max(1);
    let height = (((v1 - v0) / pixel).ceil() as usize).max(1);
    let mut filled = vec![false; width * height];
    let mut part = vec![u16::MAX; width * height];
    for (t, tri) in mesh.triangles.iter().enumerate() {
        let p: Vec<(f64, f64)> = tri.v.iter().map(|&v| pick(v)).collect();
        let det = (p[1].0 - p[0].0) * (p[2].1 - p[0].1) - (p[2].0 - p[0].0) * (p[1].1 - p[0].1);
        if det.abs() < 1e-300 {
            continue;
        }
        let umin = p.iter().map(|q| q.0).fold(f64::INFINITY, f64::min);
        let umax = p.iter().map(|q| q.0).fold(f64::NEG_INFINITY, f64::max);
        let vmin = p.iter().map(|q| q.1).fold(f64::INFINITY, f64::min);
        let vmax = p.iter().map(|q| q.1).fold(f64::NEG_INFINITY, f64::max);
        let i0 = (((umin - u0) / pixel) - 0.5).floor().max(0.0) as usize;
        let i1 = ((((umax - u0) / pixel) - 0.5).ceil().max(0.0) as usize).min(width - 1);
        let j0 = (((vmin - v0) / pixel) - 0.5).floor().max(0.0) as usize;
        let j1 = ((((vmax - v0) / pixel) - 0.5).ceil().max(0.0) as usize).min(height - 1);
        for j in j0..=j1 {
            for i in i0..=i1 {
                let (pu, pv) = (u0 + (i as f64 + 0.5) * pixel, v0 + (j as f64 + 0.5) * pixel);
                let a = ((pu - p[0].0) * (p[2].1 - p[0].1) - (p[2].0 - p[0].0) * (pv - p[0].1)) / det;
                let c = ((p[1].0 - p[0].0) * (pv - p[0].1) - (pu - p[0].0) * (p[1].1 - p[0].1)) / det;
                if a >= 0.0 && c >= 0.0 && a + c <= 1.0 {
                    let idx = i + width * (height - 1 - j);
                    filled[idx] = true;
                    part[idx] = mesh.part_of[t];
                }
            }
        }
    }
    Silhouette { width, height, pixel, origin: (u0, v0), filled, part }
}

pub fn uv_sphere(centre: Vec3, radius: f64, stacks: usize, slices: usize) -> Vec<Triangle> {
    use std::f64::consts::PI;
    let point = |i: usize, j: usize| {
        let theta = PI * i as f64 / stacks as f64;
        let phi = 2.0 * PI * (j % slices) as f64 / slices as f64;
        centre
            + Vec3::new(
                theta.sin() * phi.cos(),
                theta.sin() * phi.sin(),
                theta.cos(),
            ) * radius
    };
    let mut tris = Vec::new();
    for i in 0..stacks {
        for j in 0..slices {
            let a = point(i, j);
            let b = point(i + 1, j);
            let c = point(i + 1, j + 1);
            let d = point(i, j + 1);
            if i != 0 {
                tris.push(Triangle { v: [a, b, d] });
            }
            if i != stacks - 1 {
                tris.push(Triangle { v: [b, c, d] });
            }
        }
    }
    tris
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cube_is_watertight_with_correct_volume() {
        let m = Mesh::single("cube", unit_cube(Vec3::ZERO, Vec3::new(2.0, 3.0, 4.0)));
        assert!(m.watertight().is_watertight());
        assert!((m.signed_volume().abs() - 24.0).abs() < 1e-9);
    }

    #[test]
    fn missing_triangle_is_reported() {
        let mut tris = unit_cube(Vec3::ZERO, Vec3::new(1.0, 1.0, 1.0));
        tris.pop();
        let r = Mesh::single("cube", tris).watertight();
        assert_eq!(r.open_edges, 3);
    }

    #[test]
    fn frontal_area_of_sphere() {
        let m = Mesh::single("s", uv_sphere(Vec3::ZERO, 1.0, 64, 128));
        let a = silhouette(&m, View::Front, 400).area();
        assert!((a - std::f64::consts::PI).abs() / std::f64::consts::PI < 0.01, "{a}");
    }

    #[test]
    fn cylinder_is_closed() {
        let m = Mesh::single("w", cylinder_y(Vec3::ZERO, 1.0, 0.5, 48));
        assert!(m.watertight().is_watertight());
    }

    #[test]
    fn sphere_volume_converges() {
        let m = Mesh::single("s", uv_sphere(Vec3::ZERO, 1.0, 64, 128));
        assert!(m.watertight().is_watertight());
        let exact = 4.0 / 3.0 * std::f64::consts::PI;
        assert!((m.signed_volume().abs() - exact).abs() / exact < 0.01);
    }
}
