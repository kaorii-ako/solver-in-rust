use crate::mesh::{Mesh, Triangle};
use anyhow::{bail, Context, Result};
use cfd_core::Vec3;
use std::path::Path;

pub fn read_stl(path: &Path) -> Result<Mesh> {
    let bytes = std::fs::read(path).with_context(|| format!("could not open {}", path.display()))?;
    let name = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "part".into());
    let tris = parse_stl(&bytes).with_context(|| format!("{} is not a readable STL file", path.display()))?;
    if tris.is_empty() {
        bail!("{} contains no triangles", path.display());
    }
    Ok(Mesh::single(&name, tris))
}

pub fn parse_stl(bytes: &[u8]) -> Result<Vec<Triangle>> {
    if bytes.len() >= 84 {
        let n = u32::from_le_bytes([bytes[80], bytes[81], bytes[82], bytes[83]]) as usize;
        if 84 + n * 50 == bytes.len() {
            return Ok(parse_binary(bytes, n));
        }
    }
    let text = std::str::from_utf8(bytes).context("file is neither binary STL nor ASCII text")?;
    if !text.trim_start().starts_with("solid") {
        bail!("file does not start with 'solid' and its size does not match a binary STL");
    }
    parse_ascii(text)
}

fn parse_binary(bytes: &[u8], n: usize) -> Vec<Triangle> {
    let f = |o: usize| f32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]) as f64;
    (0..n)
        .map(|t| {
            let base = 84 + t * 50 + 12;
            let v = |k: usize| {
                let o = base + k * 12;
                Vec3::new(f(o), f(o + 4), f(o + 8))
            };
            Triangle { v: [v(0), v(1), v(2)] }
        })
        .collect()
}

fn parse_ascii(text: &str) -> Result<Vec<Triangle>> {
    let mut tris = Vec::new();
    let mut verts = Vec::with_capacity(3);
    for (line_no, line) in text.lines().enumerate() {
        let mut it = line.split_whitespace();
        if it.next() == Some("vertex") {
            let mut c = [0.0; 3];
            for slot in &mut c {
                *slot = it
                    .next()
                    .with_context(|| format!("line {}: vertex needs 3 numbers", line_no + 1))?
                    .parse()
                    .with_context(|| format!("line {}: bad number", line_no + 1))?;
            }
            verts.push(Vec3::new(c[0], c[1], c[2]));
            if verts.len() == 3 {
                tris.push(Triangle { v: [verts[0], verts[1], verts[2]] });
                verts.clear();
            }
        }
    }
    Ok(tris)
}

pub fn write_binary_stl(path: &Path, tris: &[Triangle]) -> Result<()> {
    let mut out = vec![0u8; 80];
    out.extend((tris.len() as u32).to_le_bytes());
    for t in tris {
        let n = t.normal();
        let len = n.length().max(1e-30);
        for c in [n.x / len, n.y / len, n.z / len] {
            out.extend((c as f32).to_le_bytes());
        }
        for v in t.v {
            for c in [v.x, v.y, v.z] {
                out.extend((c as f32).to_le_bytes());
            }
        }
        out.extend([0u8, 0u8]);
    }
    std::fs::write(path, out)?;
    Ok(())
}

pub fn write_ascii_stl(path: &Path, name: &str, tris: &[Triangle]) -> Result<()> {
    use std::fmt::Write;
    let mut s = format!("solid {name}\n");
    for t in tris {
        let n = t.normal();
        writeln!(s, "  facet normal {} {} {}\n    outer loop", n.x, n.y, n.z)?;
        for v in t.v {
            writeln!(s, "      vertex {} {} {}", v.x, v.y, v.z)?;
        }
        writeln!(s, "    endloop\n  endfacet")?;
    }
    writeln!(s, "endsolid {name}")?;
    std::fs::write(path, s)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::unit_cube;

    #[test]
    fn binary_and_ascii_round_trip() {
        let dir = std::env::temp_dir().join(format!("cfd-stl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let tris = unit_cube(Vec3::ZERO, Vec3::new(1.0, 2.0, 3.0));
        let b = dir.join("cube_bin.stl");
        let a = dir.join("cube_ascii.stl");
        write_binary_stl(&b, &tris).unwrap();
        write_ascii_stl(&a, "cube", &tris).unwrap();
        for p in [&b, &a] {
            let m = read_stl(p).unwrap();
            assert_eq!(m.triangles.len(), 12);
            assert!((m.signed_volume().abs() - 6.0).abs() < 1e-5);
        }
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn garbage_is_rejected() {
        assert!(parse_stl(b"hello world").is_err());
    }
}
