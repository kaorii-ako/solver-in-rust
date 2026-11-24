use cfd_geom::Mesh;
use std::path::Path;

fn fixture(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

#[test]
fn f3z_block_is_closed_with_exact_volume() {
    let r = cfd_fusion::read_fusion(&fixture("F30a.f3z"), 0.05).unwrap();
    let size = r.mesh.bounds().size();
    assert!((size.x - 40.0).abs() < 1e-6 && (size.y - 20.0).abs() < 1e-6 && (size.z - 10.0).abs() < 1e-6);
    assert!(r.mesh.watertight().is_watertight());
    assert!((r.mesh.signed_volume() - 5000.0).abs() < 1e-6);
}

#[test]
fn multi_body_design_gives_one_part_per_body() {
    let r = cfd_fusion::read_fusion(&fixture("copy_paste_two_bodies.f3d"), 0.05).unwrap();
    assert!(r.mesh.part_names.len() >= 2);
    for i in 0..r.mesh.part_names.len() {
        let tris = r.mesh.triangles.iter().zip(&r.mesh.part_of).filter(|(_, &p)| p as usize == i).map(|(t, _)| *t).collect();
        assert!(Mesh::single("p", tris).watertight().is_watertight());
    }
}
