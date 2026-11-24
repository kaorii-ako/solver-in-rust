use cfd_geom::Mesh;
fn main() {
    for arg in std::env::args().skip(1) {
        let p = std::path::Path::new(&arg);
        let t = std::time::Instant::now();
        match cfd_fusion::read_fusion(p, 0.05) {
            Ok(r) => {
                let b = r.mesh.bounds().size();
                let mut closed = 0;
                let mut vol = 0.0;
                for (i, name) in r.mesh.part_names.iter().enumerate() {
                    let tris = r.mesh.triangles.iter().zip(&r.mesh.part_of).filter(|(_, &p)| p as usize == i).map(|(t, _)| *t).collect();
                    let m = Mesh::single(name, tris);
                    if m.watertight().is_watertight() { closed += 1; }
                    vol += m.signed_volume();
                }
                println!("{:<45} parts {:>2} closed {:>2} tris {:>6} faces {:>4} failed {:>3} size {:.1}x{:.1}x{:.1} mm vol {:.1} mm3 {:.2}s",
                    p.file_name().unwrap().to_string_lossy(), r.mesh.part_names.len(), closed, r.mesh.triangles.len(), r.faces, r.failed_faces, b.x, b.y, b.z, vol, t.elapsed().as_secs_f64());
            }
            Err(e) => println!("{:<45} ERROR {}", p.file_name().unwrap().to_string_lossy(), format!("{e:#}").chars().take(120).collect::<String>()),
        }
    }
}
