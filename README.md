# cfd: car aero for small race cars

A Rust wind tunnel for small race cars. Drop in an STL of your car, set the
speed and press Run: you get drag, downforce, efficiency, aero balance, drag
power and a per-part breakdown, with flow pictures and a one-page report.
The same engine runs behind a GUI, a terminal dashboard and a CLI, so all
three give identical numbers for the same case.

The solver is a 3D lattice Boltzmann method (D3Q19, regularized BGK
collision, Smagorinsky LES, interpolated walls) on a voxel grid. It runs on
the GPU through wgpu (Vulkan, so AMD cards such as the RX 6800 XT work) and
falls back to the CPU with rayon. Both give the same numbers.

## Build

You need a Rust toolchain (rustup.rs). On Bazzite, install rustup in your home
folder or use a distrobox; the GUI needs only the desktop libraries Bazzite
already has.

```bash
cargo build --release
```

This builds three programs in `target/release/`: `cfd-gui`, `cfd-tui` and `cfd`.

## Quick start

```bash
./target/release/cfd demo
```

```bash
./target/release/cfd-gui cases/demo-car/case.toml
```

Press Run. A Quick run of the demo car takes about two minutes on a 4-core
machine.

For your own car, open the Fusion file directly:

```bash
./target/release/cfd-gui ~/Downloads/peak_car.f3z
```

`.f3z` and `.f3d` files are read with cadmpeg, an experimental, reverse-engineered
Fusion decoder. Each Fusion body becomes its own part, so name bodies like
`front_wing` or `rear_wheel` to get a readable per-part breakdown. Check the
Car views look right. If a file will not load, export STL from Fusion (one file
per part); `tools/fusion_export.py` does that for every body at once and writes
a `case.toml`.

## The four steps

1. **Car file.** Fusion .f3z or .f3d, or STL (binary or ASCII). Units are guessed (anything over 20 long
   is taken as mm) and shown so you can correct them. Parts that are not
   closed shapes get a warning with a location.
2. **Speed.** m/s or km/h.
3. **Quality.** Quick (48 cells along the car), Standard (72), Fine (96),
   Ultra (144, about 1.4 mm cells on a 200 mm car; GPU only in practice), or
   Final, which runs all three and reports a grid-independent estimate of
   Cd and Cl with an uncertainty band (Richardson extrapolation and the Grid
   Convergence Index of Celik et al. 2008).
4. **Run.** The tunnel is sized from the car, the run stops by itself once Cd
   has settled to 1%, and results are averaged over the last two car lengths
   of flow.

Everything else sits under Advanced: CPU or GPU, units, up axis, nose direction, axle
positions, reference area, a speed sweep, grid size, run length, the
Smagorinsky constant and air density.

## What it reports

| Number | Meaning |
| --- | --- |
| Drag, Cd | Force along the flow; Cd = D / (½ρU²A) |
| Downforce, Cl | Force pushing the car down; Cl is negative when the car makes downforce |
| CdA, ClA | Coefficient × area, so no argument about which area to use |
| Efficiency | −Cl/Cd |
| Aero balance | Share of the vertical load on the front axle, from the centre of pressure. Set the axle positions in Advanced; otherwise nose and tail are used. Shown as n/a when the car makes almost no lift either way. |
| Drag power | D × U |
| ± values | Standard deviation over the averaging window (the wake is unsteady) |
| By part | Drag and downforce of each STL part |

A is the frontal area measured from the geometry unless you set it.

## Floor options

- **Road** (default): a frictionless floor. Like a moving road it has no
  boundary layer, and it stays stable at every quality.
- **Moving belt + rolling wheels**: the floor moves at the car's speed, and
  parts named wheel, tyre, tire or rim spin. Closest to reality, but every gap
  under the car must be several cells high. At Quick quality a 3 mm gap is
  under one cell and the numbers come out wrong, so use Fine or bigger, or the
  Road floor.
- **Fixed floor**: like a wind tunnel without a belt.
- **No floor**: the car in open air.

## Terminal dashboard

```bash
./target/release/cfd-tui cases/demo-car/case.toml
```

Keys: `r` run, `s` stop, `+`/`-` speed, `g` quality, `f` floor, `q` quit.
It works over SSH, so you can run on the desktop and watch from the laptop.

## CLI

```bash
cfd run case.toml
cfd new body.stl front_wing.stl rear_wing.stl wheels.stl -o case.toml
cfd info case.toml
cfd history
cfd compare runs/<run-a> runs/<run-b>
cfd bench
```

`cfd bench` runs the demo car at three grid sizes on the GPU and the CPU and
prints the speed of each. Run it once on a new machine.

Every run is saved in `runs/<time>-<name>-<speed>/` with `result.json`,
`case.toml`, `report.html` and the flow slices. The 3D flow field goes to
`runs/latest-vtk/` for ParaView.

## Validation

| Check | Result |
| --- | --- |
| Voxel volume of a sphere at 80 cells across | within 1% |
| Uniform flow through an empty tunnel | holds to 0.1% |
| Slip floor | no boundary layer, mass conserved to 0.1% |
| Sphere at Re 100, 16 cells across (`cargo test --release -p cfd-lbm -- --ignored`) | Cd 1.152 vs 1.09 (6% high); half-way walls gave 1.217 |
| Fusion .f3z and .f3d test files | bodies come out closed with exact CAD volume |
| Same case from CLI and GUI | identical Cd and Cl |
| GPU against CPU, 300 steps (sphere; block on moving floor with spinning walls; slip floor) | populations agree to 3e-7, forces to 5 figures |
| Demo car through the whole app, GPU and CPU | identical Cd 0.8477 and Cl -0.4278 |

Not yet checked against your Fluent results or the Ahmed body. Treat absolute
numbers as approximate and comparisons between designs (A/B) as the main use
until that is done.

## Limits

- The GPU code has only been run on a software Vulkan driver here, never on
  a real card, so its speed on your GPU is unknown until you run `cfd bench`.
- Quick quality uses 4 mm cells on a 200 mm car: thin wings are one or two
  cells thick. Use Fine or Final for numbers you will quote.
- .f3z and .f3d are read with an experimental decoder that has been checked
  on the cadmpeg test corpus, not on a full car. STEP is not read yet.
- No RANS model (k-ω SST) yet, so this is not the same model as Fluent.

## Layout

```text
crates/
  core/     vectors, air properties, units
  fusion/   .f3z and .f3d reader (cadmpeg decoder and tessellator)
  geom/     STL reader and writer, .f3z reader, watertight check, voxelizer, silhouettes, demo car
  lbm/      D3Q19 lattice Boltzmann solver (CPU reference)
  lbm-gpu/  the same solver as a WGSL compute shader, through wgpu
  aero/     coefficients, balance, per-part split, averaging
  io-vtk/   ParaView output
  app/      case files, tunnel sizing, run thread, results, reports, comparison
  cli/      cfd
  tui/      cfd-tui
  gui/      cfd-gui
tools/fusion_export.py   Fusion script: every body to STL + STEP + case.toml
```
