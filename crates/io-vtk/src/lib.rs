//! Legacy VTK writer (binary STRUCTURED_POINTS) that ParaView opens directly.

use anyhow::Result;
use std::io::Write;
use std::path::Path;

pub enum Field<'a> {
    Scalar(&'a str, &'a [f32]),
    Vector(&'a str, &'a [[f32; 3]]),
}

pub fn write_structured_points(
    path: &Path,
    dims: [usize; 3],
    origin: [f64; 3],
    spacing: f64,
    fields: &[Field],
) -> Result<()> {
    let n = dims[0] * dims[1] * dims[2];
    let mut w = std::io::BufWriter::new(std::fs::File::create(path)?);
    writeln!(w, "# vtk DataFile Version 3.0")?;
    writeln!(w, "rust cfd solver output")?;
    writeln!(w, "BINARY")?;
    writeln!(w, "DATASET STRUCTURED_POINTS")?;
    writeln!(w, "DIMENSIONS {} {} {}", dims[0], dims[1], dims[2])?;
    writeln!(w, "ORIGIN {} {} {}", origin[0], origin[1], origin[2])?;
    writeln!(w, "SPACING {spacing} {spacing} {spacing}")?;
    writeln!(w, "POINT_DATA {n}")?;
    for field in fields {
        match field {
            Field::Scalar(name, data) => {
                anyhow::ensure!(data.len() == n, "field {name} has {} values, grid has {n}", data.len());
                writeln!(w, "SCALARS {name} float 1")?;
                writeln!(w, "LOOKUP_TABLE default")?;
                for v in data.iter() {
                    w.write_all(&v.to_be_bytes())?;
                }
                writeln!(w)?;
            }
            Field::Vector(name, data) => {
                anyhow::ensure!(data.len() == n, "field {name} has {} values, grid has {n}", data.len());
                writeln!(w, "VECTORS {name} float")?;
                for v in data.iter() {
                    for c in v {
                        w.write_all(&c.to_be_bytes())?;
                    }
                }
                writeln!(w)?;
            }
        }
    }
    w.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_and_size_are_right() {
        let p = std::env::temp_dir().join(format!("cfd-vtk-{}.vtk", std::process::id()));
        let s = vec![1.0f32; 8];
        let v = vec![[1.0f32, 0.0, 0.0]; 8];
        write_structured_points(&p, [2, 2, 2], [0.0; 3], 0.5, &[Field::Scalar("p", &s), Field::Vector("u", &v)]).unwrap();
        let bytes = std::fs::read(&p).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("DIMENSIONS 2 2 2"));
        assert!(text.contains("SCALARS p float 1"));
        assert!(text.contains("VECTORS u float"));
        assert!(bytes.len() > 8 * 4 + 8 * 12);
        assert!(write_structured_points(&p, [3, 2, 2], [0.0; 3], 0.5, &[Field::Scalar("p", &s)]).is_err());
        std::fs::remove_file(p).ok();
    }
}
