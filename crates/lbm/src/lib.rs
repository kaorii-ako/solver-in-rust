//! D3Q19 lattice Boltzmann solver: regularized BGK collision with a Smagorinsky LES
//! closure, half-way bounce-back walls, a moving-ground option, an
//! equilibrium far field and a constant-pressure outlet. Forces on each part
//! come from momentum exchange on every fluid-solid link.

use cfd_geom::VoxelGrid;
use rayon::prelude::*;

pub const Q: usize = 19;

pub const C: [[i32; 3]; Q] = [
    [0, 0, 0],
    [1, 0, 0],
    [-1, 0, 0],
    [0, 1, 0],
    [0, -1, 0],
    [0, 0, 1],
    [0, 0, -1],
    [1, 1, 0],
    [-1, -1, 0],
    [1, 0, 1],
    [-1, 0, -1],
    [0, 1, 1],
    [0, -1, -1],
    [1, -1, 0],
    [-1, 1, 0],
    [1, 0, -1],
    [-1, 0, 1],
    [0, 1, -1],
    [0, -1, 1],
];

pub const W: [f32; Q] = [
    1.0 / 3.0,
    1.0 / 18.0,
    1.0 / 18.0,
    1.0 / 18.0,
    1.0 / 18.0,
    1.0 / 18.0,
    1.0 / 18.0,
    1.0 / 36.0,
    1.0 / 36.0,
    1.0 / 36.0,
    1.0 / 36.0,
    1.0 / 36.0,
    1.0 / 36.0,
    1.0 / 36.0,
    1.0 / 36.0,
    1.0 / 36.0,
    1.0 / 36.0,
    1.0 / 36.0,
    1.0 / 36.0,
];

pub const fn opposite(q: usize) -> usize {
    if q == 0 {
        0
    } else if q % 2 == 1 {
        q + 1
    } else {
        q - 1
    }
}

pub const FLUID: u8 = 0;
pub const SOLID: u8 = 1;
pub const GROUND: u8 = 2;
pub const FARFIELD: u8 = 3;
pub const OUTLET: u8 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ground {
    None,
    Fixed,
    Moving,
    /// Frictionless floor: air slides along it with no boundary layer.
    Slip,
}

/// Direction with the z component flipped, used for specular reflection.
pub const fn mirror_z(q: usize) -> usize {
    let c = C[q];
    let mut m = 0;
    while m < Q {
        if C[m][0] == c[0] && C[m][1] == c[1] && C[m][2] == -c[2] {
            return m;
        }
        m += 1;
    }
    q
}

#[derive(Clone, Copy, Debug)]
pub struct Config {
    pub nu: f64,
    pub u_in: f64,
    pub smagorinsky: f64,
    pub ground: Ground,
    pub ramp_steps: usize,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PartForce {
    pub fx: f64,
    pub fy: f64,
    pub fz: f64,
    pub fz_x: f64,
}

#[derive(Clone, Debug, Default)]
pub struct StepStats {
    pub step: usize,
    pub parts: Vec<PartForce>,
    pub max_speed: f64,
    pub max_speed_cell: usize,
    pub finite: bool,
}

impl StepStats {
    pub fn total(&self) -> PartForce {
        let mut t = PartForce::default();
        for p in &self.parts {
            t.fx += p.fx;
            t.fy += p.fy;
            t.fz += p.fz;
            t.fz_x += p.fz_x;
        }
        t
    }
}

pub struct Lbm {
    pub nx: usize,
    pub ny: usize,
    pub nz: usize,
    pub flags: Vec<u8>,
    pub part: Vec<u16>,
    pub n_parts: usize,
    pub config: Config,
    pub step: usize,
    f: Vec<f32>,
    g: Vec<f32>,
    farfield: Vec<u32>,
    /// Surface velocity of moving solid cells as a fraction of the inflow speed
    /// (rolling wheels). Empty when every solid is at rest.
    wall_u: Vec<[f32; 3]>,
    /// For fluid cells next to a solid: index into `wall_delta`, else u32::MAX.
    wall_index: Vec<u32>,
    /// Fraction of each link from the fluid cell centre to the true surface
    /// (Bouzidi interpolated bounce-back). 0.5 is the plain half-way wall.
    wall_delta: Vec<[f32; Q]>,
    offsets: [isize; Q],
}

#[inline(always)]
fn feq(q: usize, rho: f32, ux: f32, uy: f32, uz: f32) -> f32 {
    let c = C[q];
    let cu = c[0] as f32 * ux + c[1] as f32 * uy + c[2] as f32 * uz;
    let uu = ux * ux + uy * uy + uz * uz;
    W[q] * rho * (1.0 + 3.0 * cu + 4.5 * cu * cu - 1.5 * uu)
}

fn moments(f: &[f32]) -> (f32, f32, f32, f32) {
    let mut rho = 0.0;
    let (mut mx, mut my, mut mz) = (0.0, 0.0, 0.0);
    for q in 0..Q {
        rho += f[q];
        mx += C[q][0] as f32 * f[q];
        my += C[q][1] as f32 * f[q];
        mz += C[q][2] as f32 * f[q];
    }
    (rho, mx / rho, my / rho, mz / rho)
}

impl Lbm {
    /// `domain` holds 0 for fluid and part+1 for solid cells. Its outer layer of
    /// cells is turned into boundaries: inlet and far field at x = 0, the sides
    /// and the top; outlet at x = nx - 1; ground at z = 0 unless `Ground::None`.
    pub fn new(domain: &VoxelGrid, n_parts: usize, config: Config) -> Self {
        let (nx, ny, nz) = (domain.nx, domain.ny, domain.nz);
        let n = nx * ny * nz;
        let mut flags = vec![FLUID; n];
        let mut part = vec![0u16; n];
        for k in 0..nz {
            for j in 0..ny {
                for i in 0..nx {
                    let c = i + nx * (j + ny * k);
                    let v = domain.cells[c];
                    if v != 0 {
                        flags[c] = SOLID;
                        part[c] = v - 1;
                    }
                    let edge_y = j == 0 || j == ny - 1;
                    if k == 0 && config.ground != Ground::None {
                        flags[c] = GROUND;
                        part[c] = u16::MAX;
                    } else if i == 0 || edge_y || k == 0 || k == nz - 1 {
                        flags[c] = FARFIELD;
                    } else if i == nx - 1 {
                        flags[c] = OUTLET;
                    }
                }
            }
        }
        let farfield: Vec<u32> = (0..n).filter(|&c| flags[c] == FARFIELD).map(|c| c as u32).collect();
        let mut offsets = [0isize; Q];
        for q in 0..Q {
            offsets[q] = C[q][0] as isize + nx as isize * (C[q][1] as isize + ny as isize * C[q][2] as isize);
        }
        let mut f = vec![0f32; n * Q];
        for c in 0..n {
            for q in 0..Q {
                f[c * Q + q] = W[q];
            }
        }
        let g = f.clone();
        let mut lbm = Self {
            nx,
            ny,
            nz,
            flags,
            part,
            n_parts,
            config,
            step: 0,
            f,
            g,
            farfield,
            wall_u: Vec::new(),
            wall_index: Vec::new(),
            wall_delta: Vec::new(),
            offsets,
        };
        lbm.set_farfield(lbm.inflow_now());
        lbm
    }

    /// Post-collision populations of the current step, cell-major (cell * 19 + q).
    pub fn populations(&self) -> &[f32] {
        &self.f
    }

    pub fn populations_mut(&mut self) -> &mut [f32] {
        &mut self.f
    }

    pub fn wall_velocities(&self) -> &[[f32; 3]] {
        &self.wall_u
    }

    /// Boundary-cell index per cell (u32::MAX when not next to a solid) and
    /// the wall fractions per boundary cell. Empty until `set_wall_distances`.
    pub fn wall_links(&self) -> (&[u32], &[[f32; Q]]) {
        (&self.wall_index, &self.wall_delta)
    }

    /// Part that each boundary cell's force is booked to: the part of its
    /// first solid neighbour.
    pub fn boundary_parts(&self) -> Vec<u16> {
        let mut out = vec![0u16; self.wall_delta.len()];
        for (c, &wi) in self.wall_index.iter().enumerate() {
            if wi == u32::MAX {
                continue;
            }
            for q in 1..Q {
                let src = (c as isize - self.offsets[q]) as usize;
                if self.flags[src] == SOLID {
                    out[wi as usize] = self.part[src];
                    break;
                }
            }
        }
        out
    }

    /// Gives a solid cell a surface velocity, as a fraction of the inflow speed.
    pub fn set_wall_velocity(&mut self, cell: usize, u: [f32; 3]) {
        if self.wall_u.is_empty() {
            self.wall_u = vec![[0.0; 3]; self.cells()];
        }
        self.wall_u[cell] = u;
    }

    pub fn cells(&self) -> usize {
        self.nx * self.ny * self.nz
    }

    /// Turns on interpolated bounce-back. `delta(cell, q)` returns where the
    /// surface lies along the link from fluid `cell` to its solid neighbour
    /// at `cell - c_q`, as a fraction of the link (None = unknown, use 0.5).
    pub fn set_wall_distances(&mut self, delta: impl Fn(usize, usize) -> Option<f64> + Sync) -> usize {
        let n = self.cells();
        let flags = &self.flags;
        let offsets = self.offsets;
        let rows: Vec<(usize, [f32; Q])> = (0..n)
            .into_par_iter()
            .filter(|&c| flags[c] == FLUID)
            .filter_map(|c| {
                let mut d = [0.5f32; Q];
                let mut any = false;
                for q in 1..Q {
                    let src = (c as isize - offsets[q]) as usize;
                    if flags[src] == SOLID {
                        any = true;
                        d[q] = delta(c, q).map(|v| v.clamp(0.0, 1.0) as f32).unwrap_or(0.5);
                    }
                }
                any.then_some((c, d))
            })
            .collect();
        self.wall_index = vec![u32::MAX; n];
        self.wall_delta = Vec::with_capacity(rows.len());
        for (k, (c, d)) in rows.into_iter().enumerate() {
            self.wall_index[c] = k as u32;
            self.wall_delta.push(d);
        }
        self.wall_delta.len()
    }

    pub fn tau0(&self) -> f64 {
        3.0 * self.config.nu + 0.5
    }

    pub fn memory_bytes(&self) -> usize {
        self.cells() * (2 * Q * 4 + 3)
    }

    pub fn inflow_now(&self) -> f32 {
        let ramp = if self.config.ramp_steps == 0 {
            1.0
        } else {
            (self.step as f64 / self.config.ramp_steps as f64).min(1.0)
        };
        let s = (ramp * std::f64::consts::PI * 0.5).sin();
        (self.config.u_in * s) as f32
    }

    fn set_farfield(&mut self, u: f32) {
        let mut eq = [0f32; Q];
        for q in 0..Q {
            eq[q] = feq(q, 1.0, u, 0.0, 0.0);
        }
        for &c in &self.farfield {
            let c = c as usize;
            self.f[c * Q..c * Q + Q].copy_from_slice(&eq);
            self.g[c * Q..c * Q + Q].copy_from_slice(&eq);
        }
    }

    pub fn advance(&mut self) -> StepStats {
        let u_ramp = self.inflow_now();
        let ground_u = match self.config.ground {
            Ground::Moving => u_ramp,
            _ => 0.0,
        };
        let tau0 = self.tau0() as f32;
        let cs2 = (self.config.smagorinsky * self.config.smagorinsky) as f32;
        let (nx, ny) = (self.nx, self.ny);
        let n_parts = self.n_parts;
        let flags = &self.flags;
        let part = &self.part;
        let slip = self.config.ground == Ground::Slip;
        let f = &self.f;
        let wall_u = &self.wall_u;
        let moving_solids = !wall_u.is_empty();
        let wall_index = &self.wall_index;
        let wall_delta = &self.wall_delta;
        let interpolated = !wall_index.is_empty();
        let offsets = self.offsets;

        let (parts, max_speed, finite) = self
            .g
            .par_chunks_mut(nx * Q)
            .enumerate()
            .fold(
                || (vec![PartForce::default(); n_parts], (0f32, 0usize), true),
                |(mut acc, mut vmax, mut ok), (row, out)| {
                    let base = row * nx;
                    let _ = ny;
                    for i in 0..nx {
                        let cell = base + i;
                        let o = &mut out[i * Q..i * Q + Q];
                        match flags[cell] {
                            FLUID => {}
                            FARFIELD => {
                                for q in 0..Q {
                                    o[q] = feq(q, 1.0, u_ramp, 0.0, 0.0);
                                }
                                continue;
                            }
                            OUTLET => {
                                let nb = cell - 1;
                                let (_, ux, uy, uz) = moments(&f[nb * Q..nb * Q + Q]);
                                for q in 0..Q {
                                    o[q] = feq(q, 1.0, ux, uy, uz);
                                }
                                continue;
                            }
                            _ => {
                                o.copy_from_slice(&f[cell * Q..cell * Q + Q]);
                                continue;
                            }
                        }
                        let mut fl = [0f32; Q];
                        for q in 0..Q {
                            let src = (cell as isize - offsets[q]) as usize;
                            let sf = flags[src];
                            if slip && sf == GROUND && C[q][2] == 1 {
                                let along = (cell as isize - C[q][0] as isize - nx as isize * C[q][1] as isize) as usize;
                                let fa = flags[along];
                                fl[q] = if fa == SOLID || fa == GROUND {
                                    f[cell * Q + opposite(q)]
                                } else {
                                    f[along * Q + mirror_z(q)]
                                };
                                continue;
                            }
                            if sf == SOLID || sf == GROUND {
                                let qb = opposite(q);
                                let post = f[cell * Q + qb];
                                let cq = C[q];
                                let uw: [f32; 3] = if sf == GROUND {
                                    [ground_u, 0.0, 0.0]
                                } else if moving_solids {
                                    let w = wall_u[src];
                                    [u_ramp * w[0], u_ramp * w[1], u_ramp * w[2]]
                                } else {
                                    [0.0; 3]
                                };
                                let cu = cq[0] as f32 * uw[0] + cq[1] as f32 * uw[1] + cq[2] as f32 * uw[2];
                                let wall_term = 6.0 * W[q] * cu;
                                let wi = if interpolated && sf == SOLID { wall_index[cell] } else { u32::MAX };
                                let val = if wi == u32::MAX {
                                    post + wall_term
                                } else {
                                    let delta = wall_delta[wi as usize][q];
                                    if delta < 0.5 {
                                        let next = (cell as isize + offsets[q]) as usize;
                                        if flags[next] == FLUID {
                                            2.0 * delta * post + (1.0 - 2.0 * delta) * f[next * Q + qb] + wall_term
                                        } else {
                                            post + wall_term
                                        }
                                    } else {
                                        let k = 0.5 / delta;
                                        k * post + (1.0 - k) * f[cell * Q + q] + k * wall_term
                                    }
                                };
                                fl[q] = val;
                                if sf == SOLID {
                                    let p = part[src] as usize;
                                    let s = (post + val - 2.0 * W[q]) as f64;
                                    let d = (post - val) as f64;
                                    let fx = -(cq[0] as f64) * s - uw[0] as f64 * d;
                                    let fy = -(cq[1] as f64) * s - uw[1] as f64 * d;
                                    let fz = -(cq[2] as f64) * s - uw[2] as f64 * d;
                                    let a = &mut acc[p];
                                    a.fx += fx;
                                    a.fy += fy;
                                    a.fz += fz;
                                    a.fz_x += fz * (i as f64 - 0.5 * cq[0] as f64);
                                }
                            } else {
                                fl[q] = f[src * Q + q];
                            }
                        }
                        let (rho, ux, uy, uz) = moments(&fl);
                        if !(rho.is_finite() && rho > 0.0) {
                            ok = false;
                            o.copy_from_slice(&fl);
                            continue;
                        }
                        let sp = ux * ux + uy * uy + uz * uz;
                        if sp > vmax.0 {
                            vmax = (sp, cell);
                        }
                        let mut eq = [0f32; Q];
                        for q in 0..Q {
                            eq[q] = feq(q, rho, ux, uy, uz);
                        }
                        let mut p = [0f32; 6];
                        for q in 0..Q {
                            let d = fl[q] - eq[q];
                            let c = C[q];
                            let (cx, cy, cz) = (c[0] as f32, c[1] as f32, c[2] as f32);
                            p[0] += cx * cx * d;
                            p[1] += cy * cy * d;
                            p[2] += cz * cz * d;
                            p[3] += cx * cy * d;
                            p[4] += cx * cz * d;
                            p[5] += cy * cz * d;
                        }
                        let tau = if cs2 > 0.0 {
                            let qn = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]
                                + 2.0 * (p[3] * p[3] + p[4] * p[4] + p[5] * p[5]))
                                .sqrt();
                            0.5 * (tau0 + (tau0 * tau0 + 18.0 * std::f32::consts::SQRT_2 * cs2 * qn / rho).sqrt())
                        } else {
                            tau0
                        };
                        let keep = 1.0 - 1.0 / tau;
                        let trace = (p[0] + p[1] + p[2]) / 3.0;
                        for q in 0..Q {
                            let c = C[q];
                            let (cx, cy, cz) = (c[0] as f32, c[1] as f32, c[2] as f32);
                            let neq = 4.5
                                * W[q]
                                * (cx * cx * p[0] + cy * cy * p[1] + cz * cz * p[2]
                                    + 2.0 * (cx * cy * p[3] + cx * cz * p[4] + cy * cz * p[5])
                                    - trace);
                            o[q] = eq[q] + keep * neq;
                        }
                    }
                    (acc, vmax, ok)
                },
            )
            .reduce(
                || (vec![PartForce::default(); n_parts], (0f32, 0usize), true),
                |(mut a, va, oa), (b, vb, ob)| {
                    for (x, y) in a.iter_mut().zip(b) {
                        x.fx += y.fx;
                        x.fy += y.fy;
                        x.fz += y.fz;
                        x.fz_x += y.fz_x;
                    }
                    (a, if vb.0 > va.0 { vb } else { va }, oa && ob)
                },
            );

        std::mem::swap(&mut self.f, &mut self.g);
        self.step += 1;
        StepStats {
            step: self.step,
            parts,
            max_speed: (max_speed.0 as f64).sqrt(),
            max_speed_cell: max_speed.1,
            finite,
        }
    }

    pub fn idx(&self, i: usize, j: usize, k: usize) -> usize {
        i + self.nx * (j + self.ny * k)
    }

    /// Density and velocity of one cell, in lattice units.
    pub fn macroscopic(&self, cell: usize) -> (f32, [f32; 3]) {
        let (rho, ux, uy, uz) = moments(&self.f[cell * Q..cell * Q + Q]);
        (rho, [ux, uy, uz])
    }

    pub fn total_mass(&self) -> f64 {
        (0..self.cells())
            .filter(|&c| self.flags[c] == FLUID)
            .map(|c| self.f[c * Q..c * Q + Q].iter().map(|&v| v as f64).sum::<f64>())
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cfd_core::Vec3;
    use cfd_geom::{mesh::uv_sphere, voxelize, Mesh};

    #[test]
    fn opposites_are_consistent() {
        for q in 0..Q {
            let o = opposite(q);
            assert_eq!(opposite(o), q);
            for a in 0..3 {
                assert_eq!(C[q][a], -C[o][a]);
            }
        }
        let wsum: f32 = W.iter().sum();
        assert!((wsum - 1.0).abs() < 1e-6);
    }

    #[test]
    fn empty_tunnel_keeps_uniform_flow() {
        let g = VoxelGrid {
            nx: 24,
            ny: 12,
            nz: 12,
            origin: Vec3::ZERO,
            dx: 1.0,
            cells: vec![0; 24 * 12 * 12],
        };
        let mut lbm = Lbm::new(
            &g,
            1,
            Config { nu: 0.02, u_in: 0.05, smagorinsky: 0.0, ground: Ground::Moving, ramp_steps: 0 },
        );
        for _ in 0..300 {
            assert!(lbm.advance().finite);
        }
        let (rho, u) = lbm.macroscopic(lbm.idx(12, 6, 6));
        assert!((u[0] - 0.05).abs() < 1e-3, "{u:?}");
        assert!(u[1].abs() < 1e-4 && u[2].abs() < 1e-4);
        assert!((rho - 1.0).abs() < 1e-3);
        let (_, u_low) = lbm.macroscopic(lbm.idx(12, 6, 1));
        assert!((u_low[0] - 0.05).abs() < 2e-3, "moving ground should leave no boundary layer: {u_low:?}");
    }

    #[test]
    fn slip_floor_has_no_boundary_layer() {
        let g = VoxelGrid { nx: 40, ny: 8, nz: 16, origin: Vec3::ZERO, dx: 1.0, cells: vec![0; 40 * 8 * 16] };
        let mut lbm = Lbm::new(
            &g,
            1,
            Config { nu: 0.05, u_in: 0.05, smagorinsky: 0.0, ground: Ground::Slip, ramp_steps: 0 },
        );
        let m0 = lbm.total_mass();
        for _ in 0..800 {
            assert!(lbm.advance().finite);
        }
        let (_, near) = lbm.macroscopic(lbm.idx(30, 4, 1));
        assert!((near[0] - 0.05).abs() < 1e-3, "{near:?}");
        assert!((lbm.total_mass() - m0).abs() / m0 < 1e-3);
    }

    #[test]
    fn fixed_ground_slows_the_flow_next_to_it() {
        let g = VoxelGrid {
            nx: 40,
            ny: 8,
            nz: 16,
            origin: Vec3::ZERO,
            dx: 1.0,
            cells: vec![0; 40 * 8 * 16],
        };
        let mut lbm = Lbm::new(
            &g,
            1,
            Config { nu: 0.05, u_in: 0.05, smagorinsky: 0.0, ground: Ground::Fixed, ramp_steps: 0 },
        );
        for _ in 0..800 {
            lbm.advance();
        }
        let (_, near) = lbm.macroscopic(lbm.idx(30, 4, 1));
        let (_, far) = lbm.macroscopic(lbm.idx(30, 4, 12));
        assert!(near[0] < 0.6 * far[0], "near {near:?} far {far:?}");
    }

    fn sphere_cd(d_cells: f64, re: f64, steps: usize, width: f64) -> f64 {
        let up: f64 = std::env::var("SPHERE_UP").ok().and_then(|v| v.parse().ok()).unwrap_or(5.0);
        let (nx, ny, nz) = ((d_cells * (up + 8.0)) as usize, (d_cells * width) as usize, (d_cells * width) as usize);
        let centre = Vec3::new(d_cells * up, ny as f64 / 2.0, nz as f64 / 2.0);
        let m = Mesh::single("sphere", uv_sphere(centre, d_cells / 2.0, 48, 96));
        let g = voxelize(&m, Vec3::ZERO, 1.0, nx, ny, nz);
        let u = 0.05;
        let mut lbm = Lbm::new(
            &g,
            1,
            Config { nu: u * d_cells / re, u_in: u, smagorinsky: 0.0, ground: Ground::None, ramp_steps: 200 },
        );
        if std::env::var("SPHERE_HALFWAY").is_err() {
            let bins = cfd_geom::raycast::TriangleBins::new(&m, Vec3::ZERO, 1.0);
            let (nxx, nyy) = (lbm.nx, lbm.ny);
            let centre_of = |c: usize| Vec3::new((c % nxx) as f64 + 0.5, ((c / nxx) % nyy) as f64 + 0.5, (c / (nxx * nyy)) as f64 + 0.5);
            lbm.set_wall_distances(|c, q| {
                let a = centre_of(c);
                let b = a - Vec3::new(C[q][0] as f64, C[q][1] as f64, C[q][2] as f64);
                bins.first_hit(a, b)
            });
        }
        let area = std::f64::consts::PI * d_cells * d_cells / 4.0;
        let mut sum = 0.0;
        let mut count = 0;
        for s in 0..steps {
            let st = lbm.advance();
            assert!(st.finite);
            if s > steps * 3 / 4 {
                sum += st.total().fx;
                count += 1;
            }
        }
        2.0 * (sum / count as f64) / (u * u * area)
    }

    #[test]
    #[ignore = "slow: run with cargo test --release -- --ignored"]
    fn sphere_drag_at_re_100() {
        let w: f64 = std::env::var("SPHERE_W").ok().and_then(|v| v.parse().ok()).unwrap_or(7.0);
        let d: f64 = std::env::var("SPHERE_D").ok().and_then(|v| v.parse().ok()).unwrap_or(16.0);
        let cd = sphere_cd(d, 100.0, (d * 300.0) as usize, w);
        eprintln!("sphere Cd at Re 100 = {cd:.3} (reference 1.09)");
        assert!((cd - 1.09).abs() / 1.09 < 0.10, "Cd = {cd}");
    }
}
