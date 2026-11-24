use crate::case::{Case, GroundMode, Nose, UpAxis};
use anyhow::{bail, Result};
use cfd_core::{LengthUnit, Vec3};
use cfd_geom::{f3z::read_f3z, silhouette, stl::read_stl, Mesh, View, WatertightReport};

#[derive(Clone, Debug)]
pub struct GeometryInfo {
    /// Normalised mesh in metres: nose at x = 0, centred on y = 0, lowest point at z = 0.
    pub mesh: Mesh,
    pub unit_used: LengthUnit,
    pub unit_was_guessed: bool,
    pub length_m: f64,
    pub width_m: f64,
    pub height_m: f64,
    pub frontal_area_m2: f64,
    pub volume_m3: f64,
    pub parts: Vec<(String, WatertightReport)>,
    pub warnings: Vec<String>,
}

impl GeometryInfo {
    pub fn summary(&self) -> String {
        format!(
            "{} part(s), {} triangles, {:.1} x {:.1} x {:.1} mm (L x W x H), frontal area {:.0} mm², units {}{}",
            self.parts.len(),
            self.mesh.triangles.len(),
            self.length_m * 1e3,
            self.width_m * 1e3,
            self.height_m * 1e3,
            self.frontal_area_m2 * 1e6,
            self.unit_used.label(),
            if self.unit_was_guessed { " (guessed)" } else { "" }
        )
    }
}

/// Reads one geometry file. Fusion files come back in millimetres with one
/// part per body; `notes` collects what the user should know about them.
fn load_part(spec: &crate::case::PartSpec, notes: &mut Vec<String>, fusion_mm: &mut bool) -> Result<Mesh> {
    let p = &spec.path;
    let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let mut mesh = match ext.as_str() {
        "stl" => read_stl(p)?,
        "f3z" | "f3d" => {
            let r = cfd_fusion::read_fusion(p, 0.05).map_err(|e| {
                let designs = if ext == "f3z" {
                    read_f3z(p).map(|a| format!(" It holds: {}.", a.designs.join(", "))).unwrap_or_default()
                } else {
                    String::new()
                };
                anyhow::anyhow!(
                    "{e:#}.{designs} If this keeps failing, export the car as STL from Fusion \
                     (File > Export > STL, or tools/fusion_export.py) and load that."
                )
            })?;
            *fusion_mm = true;
            notes.push(format!(
                "Read {} directly: {} bodies, {} faces, {} triangles. This uses an experimental, reverse-engineered \
                 Fusion decoder; check the Car views look right, and compare once against an STL export.",
                p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
                r.mesh.part_names.len(),
                r.faces,
                r.mesh.triangles.len()
            ));
            if r.failed_faces > 0 {
                notes.push(format!(
                    "{} face(s) of the Fusion model could not be turned into triangles and are missing.",
                    r.failed_faces
                ));
            }
            r.mesh
        }
        "step" | "stp" => bail!(
            "STEP import is not built in yet. Export {} as STL from Fusion and load that instead.",
            p.display()
        ),
        _ => bail!("{} is not a file type this tool reads. Use an STL export of the car.", p.display()),
    };
    if let Some(name) = &spec.name {
        mesh.part_names = vec![name.clone()];
    }
    Ok(mesh)
}

pub fn load_geometry(case: &Case) -> Result<GeometryInfo> {
    if case.geometry.is_empty() {
        bail!("Add a car file first (STL).");
    }
    let mut notes = Vec::new();
    let mut fusion_mm = false;
    let parts = case
        .geometry
        .iter()
        .map(|g| load_part(g, &mut notes, &mut fusion_mm))
        .collect::<Result<Vec<_>>>()?;
    let mut mesh = Mesh::merge(parts);
    if case.up_axis == UpAxis::Y {
        mesh.y_up_to_z_up();
    }
    let (unit_used, guessed) = match case.units {
        LengthUnit::Auto if fusion_mm => (LengthUnit::Mm, false),
        LengthUnit::Auto => (LengthUnit::guess_from_extent(mesh.bounds().size().max_component()), true),
        u => (u, false),
    };
    mesh.scale(unit_used.to_metres());
    match case.nose {
        Nose::MinusX => {}
        Nose::PlusX => mesh.rotate_z_180(),
        Nose::MinusY => mesh.swap_x_y(),
        Nose::PlusY => {
            mesh.swap_x_y();
            mesh.rotate_z_180();
        }
    }
    let b = mesh.bounds();
    mesh.translate(Vec3::new(-b.min.x, -(b.min.y + b.max.y) / 2.0, -b.min.z));
    let size = mesh.bounds().size();

    let mut warnings = notes;
    let mut parts = Vec::new();
    for (i, name) in mesh.part_names.iter().enumerate() {
        let tris = mesh
            .triangles
            .iter()
            .zip(&mesh.part_of)
            .filter(|(_, &p)| p as usize == i)
            .map(|(t, _)| *t)
            .collect();
        let report = Mesh::single(name, tris).watertight();
        if !report.is_watertight() {
            let at = report
                .open_edge_points
                .first()
                .map(|p| format!(" (first one near x {:.0} mm, z {:.0} mm)", p.x * 1e3, p.z * 1e3))
                .unwrap_or_default();
            warnings.push(format!(
                "'{name}' is not a closed shape: {} open edge(s){at}. The solver needs closed shapes; small gaps \
                 usually voxelize fine, large ones can fill in the wrong place. Re-export from Fusion with a finer \
                 mesh or close the gap.",
                report.open_edges + report.overshared_edges
            ));
        }
        parts.push((name.clone(), report));
    }
    if size.y > size.x * 1.2 {
        warnings.push(
            "The car looks wider than it is long. If the flow should hit the nose, change 'Nose direction'.".into(),
        );
    }
    if size.max_component() < 0.01 || size.max_component() > 10.0 {
        warnings.push(format!(
            "The car is {:.3} m long, which looks wrong. Check the units setting.",
            size.max_component()
        ));
    }
    let frontal = silhouette(&mesh, View::Front, 400).area();
    let volume = mesh.signed_volume().abs();
    Ok(GeometryInfo {
        mesh,
        unit_used,
        unit_was_guessed: guessed,
        length_m: size.x,
        width_m: size.y,
        height_m: size.z,
        frontal_area_m2: frontal,
        volume_m3: volume,
        parts,
        warnings,
    })
}

#[derive(Clone, Copy, Debug)]
pub struct DomainPlan {
    pub dx: f64,
    pub nx: usize,
    pub ny: usize,
    pub nz: usize,
    pub origin: Vec3,
}

impl DomainPlan {
    pub fn cells(&self) -> usize {
        self.nx * self.ny * self.nz
    }

    pub fn memory_bytes(&self) -> usize {
        self.cells() * (2 * 19 * 4 + 3)
    }
}

/// Wind tunnel sized from the car: inlet well upstream, a long wake region,
/// and side and top clearance scaled by the larger of width and height.
pub fn plan_domain(info: &GeometryInfo, cells_across_length: usize, ground: GroundMode) -> DomainPlan {
    let (l, w, h) = (info.length_m, info.width_m, info.height_m);
    let big = w.max(h);
    let upstream = (0.8 * l).max(4.0 * big);
    let downstream = 2.5 * l;
    let side = 2.5 * big;
    let top = 3.0 * big;
    let dx = l / cells_across_length as f64;
    let nx = ((upstream + l + downstream) / dx).ceil() as usize + 2;
    let ny = ((w + 2.0 * side) / dx).ceil() as usize + 2;
    let (nz, z0) = match ground {
        GroundMode::None => (((h + 2.0 * top) / dx).ceil() as usize + 2, -top - dx),
        _ => (((h + top) / dx).ceil() as usize + 2, -dx),
    };
    DomainPlan {
        dx,
        nx,
        ny,
        nz,
        origin: Vec3::new(-upstream - dx, -(ny as f64) * dx / 2.0, z0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cfd_geom::stl::write_binary_stl;

    fn demo_case(dir: &std::path::Path) -> Case {
        std::fs::create_dir_all(dir).unwrap();
        let mut paths = Vec::new();
        for part in cfd_geom::demo::demo_car_parts() {
            let p = dir.join(format!("{}.stl", part.part_names[0]));
            write_binary_stl(&p, &part.triangles).unwrap();
            paths.push(p);
        }
        Case::for_geometry(&paths)
    }

    #[test]
    fn demo_car_loads_in_millimetres() {
        let dir = std::env::temp_dir().join(format!("cfd-prep-{}", std::process::id()));
        let case = demo_case(&dir);
        let info = load_geometry(&case).unwrap();
        assert_eq!(info.unit_used, LengthUnit::Mm);
        assert!((info.length_m - 0.206).abs() < 1e-6, "{}", info.length_m);
        assert_eq!(info.parts.len(), 4);
        assert!(info.frontal_area_m2 > 0.001 && info.frontal_area_m2 < 0.01);
        let plan = plan_domain(&info, 48, GroundMode::Moving);
        assert!(plan.cells() < 3_000_000, "{}", plan.cells());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn nose_direction_flips_the_car() {
        let dir = std::env::temp_dir().join(format!("cfd-nose-{}", std::process::id()));
        let mut case = demo_case(&dir);
        let a = load_geometry(&case).unwrap();
        case.nose = Nose::PlusX;
        let b = load_geometry(&case).unwrap();
        let wing_x = |i: &GeometryInfo| {
            let tris: Vec<_> = i.mesh.triangles.iter().zip(&i.mesh.part_of).filter(|(_, &p)| p == 1).collect();
            tris.iter().map(|(t, _)| t.v[0].x).sum::<f64>() / tris.len() as f64
        };
        assert!(wing_x(&a) < 0.05 && wing_x(&b) > 0.15);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn fusion_archive_loads_directly() {
        let f = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../fusion/tests/fixtures/F30a.f3z");
        let info = load_geometry(&Case::for_geometry(&[f])).unwrap();
        assert_eq!(info.unit_used, LengthUnit::Mm);
        assert!((info.length_m - 0.040).abs() < 1e-9, "{}", info.length_m);
        assert!((info.volume_m3 - 5.0e-6).abs() < 1e-9, "{}", info.volume_m3);
        assert!(info.parts.iter().all(|(_, w)| w.is_watertight()));
    }

    #[test]
    fn f3z_gets_a_helpful_message() {
        let mut case = Case::for_geometry(&[std::path::PathBuf::from("/nope/car.step")]);
        let e = load_geometry(&case).unwrap_err().to_string();
        assert!(e.contains("Export"), "{e}");
        case.geometry[0].path = "/nope/car.obj".into();
        assert!(load_geometry(&case).is_err());
    }
}
