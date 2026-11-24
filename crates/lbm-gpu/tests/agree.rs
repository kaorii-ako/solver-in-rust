//! The GPU must reproduce the CPU solver. Skips when no GPU adapter exists.

use cfd_core::Vec3;
use cfd_geom::{mesh::unit_cube, mesh::uv_sphere, raycast::TriangleBins, voxelize, Mesh};
use cfd_lbm::{Config, Ground, Lbm, C};
use cfd_lbm_gpu::GpuLbm;

fn setup(mesh: &Mesh, dims: (usize, usize, usize), ground: Ground, spin: bool) -> Lbm {
    let g = voxelize(mesh, Vec3::ZERO, 1.0, dims.0, dims.1, dims.2);
    let mut lbm = Lbm::new(&g, 1, Config { nu: 0.01, u_in: 0.06, smagorinsky: 0.12, ground, ramp_steps: 40 });
    let bins = TriangleBins::new(mesh, Vec3::ZERO, 1.0);
    let (nx, ny) = (lbm.nx, lbm.ny);
    let centre = |c: usize| Vec3::new((c % nx) as f64 + 0.5, ((c / nx) % ny) as f64 + 0.5, (c / (nx * ny)) as f64 + 0.5);
    lbm.set_wall_distances(|c, q| {
        let a = centre(c);
        bins.first_hit(a, a - Vec3::new(C[q][0] as f64, C[q][1] as f64, C[q][2] as f64))
    });
    if spin {
        for c in 0..lbm.cells() {
            if lbm.flags[c] == cfd_lbm::SOLID {
                lbm.set_wall_velocity(c, [0.3, 0.0, -0.2]);
            }
        }
    }
    lbm
}

fn compare(mut cpu: Lbm, steps: usize) {
    let mut gpu = match GpuLbm::new(&cpu) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("skipping: {e:#}");
            return;
        }
    };
    eprintln!("GPU: {}", gpu.adapter);
    let mut last = None;
    for _ in 0..steps {
        last = Some(cpu.advance());
    }
    let g = gpu.advance(steps);
    let c = last.unwrap();
    let (ct, gt) = (c.total(), g.total());
    eprintln!("CPU F = ({:.6e}, {:.6e}, {:.6e})  GPU F = ({:.6e}, {:.6e}, {:.6e})", ct.fx, ct.fy, ct.fz, gt.fx, gt.fy, gt.fz);
    assert!(g.finite);
    let scale = ct.fx.abs().max(ct.fz.abs()).max(1e-9);
    for (a, b) in [(ct.fx, gt.fx), (ct.fy, gt.fy), (ct.fz, gt.fz)] {
        assert!((a - b).abs() / scale < 2e-3, "force mismatch: cpu {a} gpu {b}");
    }
    let mut back = Lbm::new(
        &cfd_geom::VoxelGrid { nx: cpu.nx, ny: cpu.ny, nz: cpu.nz, origin: Vec3::ZERO, dx: 1.0, cells: vec![0; cpu.cells()] },
        1,
        cpu.config,
    );
    gpu.download_into(&mut back);
    let worst = cpu
        .populations()
        .iter()
        .zip(back.populations())
        .map(|(a, b)| (a - b).abs())
        .fold(0f32, f32::max);
    eprintln!("largest population difference after {steps} steps: {worst:e}");
    assert!(worst < 1e-4, "{worst}");
}

#[test]
fn sphere_in_free_air_matches_cpu() {
    let m = Mesh::single("s", uv_sphere(Vec3::new(14.0, 12.0, 12.0), 5.0, 32, 64));
    compare(setup(&m, (48, 24, 24), Ground::None, false), 300);
}

#[test]
fn block_on_moving_floor_with_spinning_surface_matches_cpu() {
    let m = Mesh::single("b", unit_cube(Vec3::new(10.0, 8.0, 1.0), Vec3::new(20.0, 16.0, 7.0)));
    compare(setup(&m, (48, 24, 16), Ground::Moving, true), 300);
}

#[test]
fn block_on_slip_floor_matches_cpu() {
    let m = Mesh::single("b", unit_cube(Vec3::new(10.0, 8.0, 2.3), Vec3::new(20.0, 16.0, 7.0)));
    compare(setup(&m, (48, 24, 16), Ground::Slip, false), 300);
}
