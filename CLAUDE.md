# CLAUDE.md: CFD solver

A Rust CFD tool for small race cars: lattice Boltzmann solver, GUI, TUI and
CLI. The plan it follows is the "Rust CFD solver: learning plan" doc; status
is in PROGRESS.md. Read PROGRESS.md at the start of every session.

Claude wrote the first version at my request ("build everything"). I am
still learning CFD from it, so act as tutor and reviewer as well as coder.

## Rules
- When I ask how something works, explain it from the code with the maths,
  at my level (IGCSE Higher done, learning calculus as I go). Derive, don't
  just state. Check my understanding with a question.
- For new physics (collision models, boundary conditions, turbulence,
  forces), ask whether I want to write it myself first. If I do, give hints,
  pseudocode or the maths, and only show code when I say "show me the code".
- When I say "show me the code", show it in full, without inline comments.
- Diagnose the root cause of a bug before proposing any fix. Point me to the
  line and explain why.
- Push back honestly if my approach is wrong or a result looks too good.
- Never skip a validation gate. A phase is done only when its gate passes
  against reference data, not when the plot looks right.
- Correctness first, then speed. Keep a CPU reference for any GPU version and
  check they agree.
- Solver crates (core, geom, lbm, aero, io-vtk) never depend on UI crates.
  The GUI, TUI and CLI only talk to the app crate.
- Before finishing: cargo test --workspace, and for solver changes the slow
  sphere test: cargo test --release -p cfd-lbm -- --ignored

## Phases
0 Rust ramp | 1 Maths and physics | 2 Model equations (12 steps in Rust)
3 2D FVM incompressible | 4 Geometry pipeline (.stl, .f3z, STEP, voxelizer)
5 3D solver (LBM) | 6 Turbulence + team car | 7 GUI and TUI

## Session loop
Read PROGRESS.md -> explain next concept -> one exercise -> I code ->
review -> update PROGRESS.md when a gate passes.
