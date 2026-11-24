use anyhow::{Context, Result};
use std::io::Read;
use std::path::Path;

#[derive(Clone, Debug, Default)]
pub struct F3zArchive {
    pub designs: Vec<String>,
    pub json_files: Vec<(String, Vec<String>)>,
    pub other_files: Vec<String>,
}

pub fn read_f3z(path: &Path) -> Result<F3zArchive> {
    let file = std::fs::File::open(path).with_context(|| format!("could not open {}", path.display()))?;
    let mut zip = zip::ZipArchive::new(file).context("this .f3z is not a valid zip archive")?;
    let mut out = F3zArchive::default();
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        let name = entry.name().to_string();
        let lower = name.to_lowercase();
        if lower.ends_with(".f3d") {
            out.designs.push(name);
        } else if lower.ends_with(".json") {
            let mut s = String::new();
            entry.read_to_string(&mut s).ok();
            let keys = serde_json::from_str::<serde_json::Value>(&s)
                .ok()
                .and_then(|v| v.as_object().map(|o| o.keys().cloned().collect()))
                .unwrap_or_default();
            out.json_files.push((name, keys));
        } else {
            out.other_files.push(name);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn lists_designs_and_manifest() {
        let dir = std::env::temp_dir().join(format!("cfd-f3z-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("car.f3z");
        let mut zw = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        let opts = zip::write::SimpleFileOptions::default();
        zw.start_file("Body.f3d", opts).unwrap();
        zw.write_all(b"x").unwrap();
        zw.start_file("Wheel.f3d", opts).unwrap();
        zw.write_all(b"x").unwrap();
        zw.start_file("manifest.json", opts).unwrap();
        zw.write_all(br#"{"designs": [], "version": 1}"#).unwrap();
        zw.finish().unwrap();
        let a = read_f3z(&path).unwrap();
        assert_eq!(a.designs, vec!["Body.f3d", "Wheel.f3d"]);
        assert_eq!(a.json_files[0].1, vec!["designs", "version"]);
        std::fs::remove_dir_all(dir).ok();
    }
}
