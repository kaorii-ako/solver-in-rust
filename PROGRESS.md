# Progress

Claude built the first working version on 2026-11-24 at Tawin's request.
Phases 0 to 2 are learning phases; they stay open until Tawin does them.

| Phase | Status | Evidence |
| --- | --- | --- |
| 0 Rust ramp | open | |
| 1 Maths and physics | open | |
| 2 Model equations | open | |
| 3 2D FVM solver | skipped for now | went straight to 3D LBM; cavity and cylinder gates not run |
| 4 Geometry pipeline | mostly done | STL in/out, .f3z listing, watertight check, per-part voxelizer; sphere voxel volume within 1% at 80 cells. STEP and .f3d geometry not read. |
| 5 3D solver (LBM, CPU) | Gate B passed, A and C open | sphere Re 100, 16 cells across, 7 D wide tunnel, inlet 5 D upstream: Cd 1.177 vs 1.09 reference (8%). No duct-flow or Ahmed-body check yet. No GPU backend yet. |
| 6 Turbulence + team car | started | Smagorinsky LES (Cs 0.16) on regularized BGK. Not yet compared with Fluent. |
| 7 GUI and TUI | Gate A passed | demo car: CLI and GUI give the same Cd 0.866 and Cl -0.064. Gate B (a teammate gets a report in 5 minutes unaided) not tried. |

## Demo car, Quick quality, Road floor, 20 m/s

Cd 0.88 ± 0.11, drag 0.59 N, almost no lift. About 2.5 minutes on 4 CPU cores
(31 M cell updates/s, 1.5 M cells).

## What went wrong on the way, and the fixes

1. Plain BGK blew up at Re 2.7e5: the relaxation time was 0.50004. Fixed with
   regularized collision plus Smagorinsky LES.
2. Lift was off by about -28 Cl: wheels touching the floor had no air
   underneath, so absolute pressure pushed them down. Fixed by computing
   forces from gauge pressure (subtracting the rest state).
3. Moving belt gave nonsense (Cd 4.8, a wing with thrust): 3 to 6 mm gaps
   under the car were about one cell high at Quick quality, and the moving
   wall pumped air into them. Added a frictionless Road floor as the default;
   the moving belt stays as an option with a warning.
4. Moving surfaces need a correction term in momentum exchange
   (Galilean-invariant form); added.
5. Overlapping parts broke inside/outside parity in the voxelizer; each part
   is now voxelized on its own.

## Open questions

- Checkerboard ripples upstream of the car in the flow pictures: probably
  pressure waves from start-up that the regularized model damps slowly.
  Check whether they change Cd.
- Compare one team car against Fluent at the same speed and area.
- wgpu backend for the RX 6800 XT, checked against the CPU version.
- Ahmed body (Phase 5 Gate C) and duct flow (Gate A).
