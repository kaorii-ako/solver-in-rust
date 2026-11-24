use crate::mesh::Mesh;
use cfd_core::Vec3;
use rayon::prelude::*;

#[derive(Clone, Debug)]
pub struct VoxelGrid {
    pub nx: usize,
    pub ny: usize,
    pub nz: usize,
    pub origin: Vec3,
    pub dx: f64,
    pub cells: Vec<u16>,
}

impl VoxelGrid {
    pub fn idx(&self, i: usize, j: usize, k: usize) -> usize {
        i + self.nx * (j + self.ny * k)
    }

    pub fn centre(&self, i: usize, j: usize, k: usize) -> Vec3 {
        self.origin + Vec3::new(i as f64 + 0.5, j as f64 + 0.5, k as f64 + 0.5) * self.dx
    }

    pub fn solid_count(&self) -> usize {
        self.cells.iter().filter(|&&c| c != 0).count()
    }

    pub fn volume(&self) -> f64 {
        self.solid_count() as f64 * self.dx.powi(3)
    }

    pub fn frontal_cells(&self) -> usize {
        let mut n = 0;
        for k in 0..self.nz {
            for j in 0..self.ny {
                if (0..self.nx).any(|i| self.cells[self.idx(i, j, k)] != 0) {
                    n += 1;
                }
            }
        }
        n
    }

    pub fn frontal_area(&self) -> f64 {
        self.frontal_cells() as f64 * self.dx * self.dx
    }
}

pub fn voxelize(mesh: &Mesh, origin: Vec3, dx: f64, nx: usize, ny: usize, nz: usize) -> VoxelGrid {
    let yc = |j: usize| origin.y + (j as f64 + 0.5 + 1.2345e-4) * dx;
    let zc = |k: usize| origin.z + (k as f64 + 0.5 + 2.3456e-4) * dx;
    let mut columns: Vec<Vec<(f64, u16)>> = vec![Vec::new(); ny * nz];
    for (t, tri) in mesh.triangles.iter().enumerate() {
        let [a, b, c] = tri.v;
        let ymin = a.y.min(b.y).min(c.y);
        let ymax = a.y.max(b.y).max(c.y);
        let zmin = a.z.min(b.z).min(c.z);
        let zmax = a.z.max(b.z).max(c.z);
        let j0 = (((ymin - origin.y) / dx - 0.5).floor().max(0.0)) as usize;
        let j1 = (((ymax - origin.y) / dx - 0.5).ceil().max(0.0) as usize).min(ny.saturating_sub(1));
        let k0 = (((zmin - origin.z) / dx - 0.5).floor().max(0.0)) as usize;
        let k1 = (((zmax - origin.z) / dx - 0.5).ceil().max(0.0) as usize).min(nz.saturating_sub(1));
        if j0 > j1 || k0 > k1 {
            continue;
        }
        let det = (b.y - a.y) * (c.z - a.z) - (c.y - a.y) * (b.z - a.z);
        if det.abs() < 1e-300 {
            continue;
        }
        for k in k0..=k1 {
            for j in j0..=j1 {
                let (py, pz) = (yc(j), zc(k));
                let u = ((py - a.y) * (c.z - a.z) - (c.y - a.y) * (pz - a.z)) / det;
                let v = ((b.y - a.y) * (pz - a.z) - (py - a.y) * (b.z - a.z)) / det;
                if u >= 0.0 && v >= 0.0 && u + v <= 1.0 {
                    let x = a.x + u * (b.x - a.x) + v * (c.x - a.x);
                    columns[j + ny * k].push((x, mesh.part_of[t]));
                }
            }
        }
    }
    let mut cells = vec![0u16; nx * ny * nz];
    cells
        .par_chunks_mut(nx)
        .zip(columns.par_iter_mut())
        .for_each(|(row, hits)| {
            hits.sort_by(|p, q| p.1.cmp(&q.1).then(p.0.total_cmp(&q.0)));
            let mut start = 0;
            while start < hits.len() {
                let part = hits[start].1;
                let mut end = start;
                while end < hits.len() && hits[end].1 == part {
                    end += 1;
                }
                let mut m = start;
                while m + 1 < end {
                    let x_in = hits[m].0;
                    let x_out = hits[m + 1].0;
                    let i0 = ((x_in - origin.x) / dx - 0.5).ceil().max(0.0) as usize;
                    let i1 = ((x_out - origin.x) / dx - 0.5).floor();
                    if i1 >= 0.0 {
                        let i1 = (i1 as usize).min(nx - 1);
                        for cell in row.iter_mut().take(i1 + 1).skip(i0) {
                            if *cell == 0 {
                                *cell = part + 1;
                            }
                        }
                    }
                    m += 2;
                }
                start = end;
            }
        });
    VoxelGrid { nx, ny, nz, origin, dx, cells }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::{unit_cube, uv_sphere};

    #[test]
    fn cube_fills_exact_cells() {
        let m = Mesh::single("c", unit_cube(Vec3::new(2.0, 2.0, 2.0), Vec3::new(6.0, 5.0, 4.0)));
        let g = voxelize(&m, Vec3::ZERO, 1.0, 10, 10, 10);
        assert_eq!(g.solid_count(), 4 * 3 * 2);
        assert_eq!(g.frontal_cells(), 3 * 2);
    }

    #[test]
    fn overlapping_parts_still_fill() {
        let a = Mesh::single("a", unit_cube(Vec3::new(1.0, 1.0, 1.0), Vec3::new(5.0, 3.0, 3.0)));
        let b = Mesh::single("b", unit_cube(Vec3::new(3.0, 1.0, 1.0), Vec3::new(8.0, 3.0, 3.0)));
        let g = voxelize(&Mesh::merge(vec![a, b]), Vec3::ZERO, 1.0, 10, 4, 4);
        assert_eq!(g.solid_count(), 7 * 2 * 2);
    }

    #[test]
    fn sphere_volume_converges_with_resolution() {
        let m = Mesh::single("s", uv_sphere(Vec3::new(1.0, 1.0, 1.0), 0.8, 48, 96));
        let exact = m.signed_volume().abs();
        let mut errs = Vec::new();
        for n in [20usize, 40, 80] {
            let dx = 2.0 / n as f64;
            let g = voxelize(&m, Vec3::ZERO, dx, n, n, n);
            errs.push((g.volume() - exact).abs() / exact);
        }
        assert!(errs[2] < 0.01, "{errs:?}");
        assert!(errs[2] < errs[0], "{errs:?}");
    }
}
