//! Reads Fusion .f3d and .f3z files directly, through the reverse-engineered
//! cadmpeg decoder, and tessellates every body into triangles: one part per
//! body, in millimetres.

use anyhow::{bail, Context, Result};
use cadmpeg_codec_f3d::F3dCodec;
use cadmpeg_ir::{Codec, DecodeOptions};
use cadmpeg_tessellate::{Options, Tessellator};
use cfd_core::Vec3;
use cfd_geom::{Mesh, Triangle};
use std::collections::HashMap;
use std::path::Path;

pub struct FusionImport {
    pub mesh: Mesh,
    pub faces: usize,
    pub failed_faces: usize,
    pub notes: Vec<String>,
}

pub fn read_fusion(path: &Path, chord_mm: f64) -> Result<FusionImport> {
    let mut file = std::fs::File::open(path).with_context(|| format!("could not open {}", path.display()))?;
    let decoded = F3dCodec
        .decode(&mut file, &DecodeOptions::default())
        .map_err(|e| anyhow::anyhow!("{e}"))
        .with_context(|| format!("the Fusion decoder could not read {}", path.display()))?;
    let mut notes: Vec<String> = decoded
        .report()
        .losses
        .iter()
        .take(20)
        .map(|l| format!("{:?}: {}", l.severity, l.message))
        .collect();
    let ir = decoded.ir();
    let model = &ir.model;
    let shell_region: HashMap<_, _> = model.shells.iter().map(|s| (s.id.clone(), s.region.clone())).collect();
    let region_body: HashMap<_, _> = model.regions.iter().map(|r| (r.id.clone(), r.body.clone())).collect();
    let mut body_part = HashMap::new();
    let mut names: Vec<String> = Vec::new();
    for (i, b) in model.bodies.iter().enumerate() {
        let name = b.name.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| format!("body_{}", i + 1));
        let mut unique = name.clone();
        let mut k = 2;
        while names.contains(&unique) {
            unique = format!("{name}_{k}");
            k += 1;
        }
        names.push(unique);
        body_part.insert(b.id.clone(), (i as u16, b.transform));
    }
    let mut tess = Tessellator::new(ir, Options { chord: chord_mm, angle: 0.2, ..Options::default() });
    let mut triangles = Vec::new();
    let mut part_of = Vec::new();
    let mut failed = 0usize;
    for face in &model.faces {
        let Some(region) = shell_region.get(&face.shell) else { continue };
        let Some(body) = region_body.get(region) else { continue };
        let Some(&(part, transform)) = body_part.get(body) else { continue };
        let mesh = match tess.face(&face.id) {
            Ok(m) => m,
            Err(e) => {
                failed += 1;
                if failed <= 5 {
                    notes.push(format!("face could not be tessellated: {e:?}"));
                }
                continue;
            }
        };
        let pts: Vec<Vec3> = mesh
            .positions
            .iter()
            .map(|p| {
                let q = match transform {
                    Some(t) => t.apply_point(*p).map(|f| f.into()).unwrap_or(*p),
                    None => *p,
                };
                Vec3::new(q.x, q.y, q.z)
            })
            .collect();
        for t in &mesh.triangles {
            triangles.push(Triangle { v: [pts[t[0]], pts[t[1]], pts[t[2]]] });
            part_of.push(part);
        }
    }
    if triangles.is_empty() {
        bail!(
            "no solid geometry could be read from {} ({} bodies, {} faces). Export STL from Fusion instead.",
            path.display(),
            model.bodies.len(),
            model.faces.len()
        );
    }
    let used: std::collections::BTreeSet<u16> = part_of.iter().copied().collect();
    let remap: HashMap<u16, u16> = used.iter().enumerate().map(|(n, &p)| (p, n as u16)).collect();
    let part_names = used.iter().map(|&p| names[p as usize].clone()).collect();
    let part_of = part_of.iter().map(|p| remap[p]).collect();
    Ok(FusionImport {
        mesh: Mesh { triangles, part_of, part_names },
        faces: model.faces.len(),
        failed_faces: failed,
        notes,
    })
}
