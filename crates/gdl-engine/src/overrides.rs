//! Replacement parts for the game's rigid-part models, read from the override
//! directory (`GDL_OVERRIDE`): `parts/<OBJECT NAME>.glb` stands in for the
//! object a skeleton node drives (`gdl parts` writes the originals as OBJ to
//! rebuild from). The glb is in the part's own frame, pivot at the origin, so
//! the original animation moves it exactly as it moved the original part.

use gdl_formats::Image;
use gdl_modern::prop::Prop;
use gdl_sim::mesh::{Batch, MeshData, Vertex};
use std::path::PathBuf;

/// `mesh` with every part that has a replacement swapped for it; `None` when
/// nothing is replaced (keep the original).
pub fn with_parts(mesh: &MeshData, part_names: &[String]) -> Option<MeshData> {
    let dir = gdl_formats::texture::override_dir()?.join("parts");
    let found: Vec<(u16, PathBuf)> = part_names
        .iter()
        .enumerate()
        .filter(|(_, n)| !n.is_empty())
        .filter_map(|(p, n)| {
            let path = dir.join(format!("{n}.glb"));
            path.is_file().then_some((p as u16, path))
        })
        .collect();
    if found.is_empty() {
        return None;
    }
    let mut out = MeshData {
        vertices: mesh.vertices.clone(),
        indices: mesh.indices.clone(),
        batches: Vec::new(),
        textures: mesh.textures.clone(),
        tex_anims: mesh.tex_anims.clone(),
        seq_anims: mesh.seq_anims.clone(),
        bounds_min: mesh.bounds_min,
        bounds_max: mesh.bounds_max,
    };
    let replaced: Vec<u16> = found.iter().map(|(p, _)| *p).collect();
    out.batches.extend(
        mesh.batches
            .iter()
            .filter(|b| !replaced.contains(&b.part))
            .cloned(),
    );
    for (part, path) in found {
        let prop = match Prop::load(&path) {
            Ok(p) => p,
            Err(e) => {
                log::warn!("part override {}: {e:#}", path.display());
                out.batches
                    .extend(mesh.batches.iter().filter(|b| b.part == part).cloned());
                continue;
            }
        };
        let texture = prop.texture.map(|t| {
            out.textures.push(Image {
                width: t.width,
                height: t.height,
                rgba: t.rgba,
            });
            out.textures.len() - 1
        });
        let base = out.vertices.len() as u32;
        for i in 0..prop.positions.len() {
            let p = prop.positions[i];
            out.bounds_min = out.bounds_min.min(p);
            out.bounds_max = out.bounds_max.max(p);
            out.vertices.push(Vertex {
                pos: p.to_array(),
                normal: prop.normals[i].to_array(),
                color: [1.0; 4],
                uv: prop.uvs[i].to_array(),
                lm_uv: [0.0; 2],
            });
        }
        let start = out.indices.len() as u32;
        out.indices.extend(prop.indices.iter().map(|i| i + base));
        out.batches.push(Batch {
            start,
            count: prop.indices.len() as u32,
            texture,
            lightmap: None,
            blend: false,
            prelit: false,
            additive: false,
            part,
        });
        log::info!("part {} <- {}", part_names[part as usize], path.display());
    }
    Some(out)
}
