//! Replacement parts for the game's rigid-part models, read from the override
//! directory (`GDL_OVERRIDE`): `parts/<OBJECT NAME>.glb` stands in for the
//! object a skeleton node drives (`gdl parts` writes the originals as OBJ to
//! rebuild from). The glb is in the part's own frame, pivot at the origin, so
//! the original animation moves it exactly as it moved the original part.
//!
//! A mesh-swap run's frames (`BAROBJACTI16F01` .. `F24`, the barrel breaking)
//! can come from one animated glb named after the run, `parts/BAROBJACTI16.glb`:
//! its nodes are the pieces and its animation moves them, sampled at frame
//! `k - 1` for `F<k>` ([`AnimProp`]).
//!
//! A world object is replaced the same way, `parts/<WORLD OBJECT NAME>.glb`
//! placed where the object stands ([`world_props`]).

use gdl_formats::Image;
use gdl_modern::prop::{AnimProp, Prop};
use gdl_sim::level::Level;
use gdl_sim::mesh::{Batch, MeshData, Vertex};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Where a replacement part comes from.
enum Source {
    /// Its own glb.
    File(PathBuf),
    /// Frame `k` (from 0) of the run's animated glb.
    Frame(PathBuf, usize),
}

/// `BAROBJACTI16F07` -> (`BAROBJACTI16`, 6): a mesh-swap run's frame name.
fn run_frame(name: &str) -> Option<(&str, usize)> {
    let (run, f) = name.rsplit_once('F')?;
    if f.len() != 2 || !f.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if !run.bytes().last().is_some_and(|b| b.is_ascii_digit()) {
        return None;
    }
    let k: usize = f.parse().ok()?;
    Some((run, k.saturating_sub(1)))
}

fn source_for(dir: &Path, name: &str) -> Option<Source> {
    if name.is_empty() {
        return None;
    }
    let own = dir.join(format!("{name}.glb"));
    if own.is_file() {
        return Some(Source::File(own));
    }
    let (run, k) = run_frame(name)?;
    let path = dir.join(format!("{run}.glb"));
    path.is_file().then_some(Source::Frame(path, k))
}

/// Loaded glbs of one replacement pass, so a run's 24 frames read their glb
/// (and upload its texture) once.
#[derive(Default)]
struct Loaded {
    anims: HashMap<PathBuf, Option<(AnimProp, Option<usize>)>>,
}

impl Loaded {
    /// The prop for `src`, and its texture's slot in `out` (pushed on first use).
    fn prop(&mut self, src: &Source, out: &mut MeshData) -> Option<(Prop, Option<usize>)> {
        match src {
            Source::File(path) => {
                let mut prop = Prop::load(path)
                    .map_err(|e| log::warn!("part override {}: {e:#}", path.display()))
                    .ok()?;
                let tex = prop.texture.take().map(|t| push_texture(out, t));
                Some((prop, tex))
            }
            Source::Frame(path, k) => {
                let entry = self.anims.entry(path.clone()).or_insert_with(|| {
                    match AnimProp::load(path) {
                        Ok(mut a) => {
                            let tex = a.texture.take().map(|t| push_texture(out, t));
                            Some((a, tex))
                        }
                        Err(e) => {
                            log::warn!("part override {}: {e:#}", path.display());
                            None
                        }
                    }
                });
                let (anim, tex) = entry.as_ref()?;
                Some((anim.pose(*k), *tex))
            }
        }
    }
}

fn push_texture(out: &mut MeshData, t: gdl_modern::Texture) -> usize {
    out.textures.push(Image {
        width: t.width,
        height: t.height,
        rgba: t.rgba,
    });
    out.bumps.push(None);
    out.textures.len() - 1
}

/// How the part a replacement stands for was drawn: alpha blended,
/// additive, and when prelit (lit by its vertex colours) the mean of those
/// colours, so the replacement sits in the same light as its neighbours.
#[derive(Clone, Copy, Default)]
struct Style {
    blend: bool,
    additive: bool,
    prelit: Option<[f32; 4]>,
}

impl Style {
    /// The style of the vertices `batches` draw (a part's, or an object's).
    fn of<'a>(batches: impl Iterator<Item = &'a Batch>, vertices: &[Vertex], indices: &[u32]) -> Style {
        let mut style = Style::default();
        let (mut sum, mut n) = ([0.0f32; 4], 0usize);
        for b in batches {
            style.blend |= b.blend;
            style.additive |= b.additive;
            if b.prelit {
                for &i in &indices[b.start as usize..(b.start + b.count) as usize] {
                    for k in 0..4 {
                        sum[k] += vertices[i as usize].color[k];
                    }
                    n += 1;
                }
            }
        }
        if n > 0 {
            style.prelit = Some(sum.map(|c| c / n as f32));
        }
        style
    }
}

/// Append `prop`'s geometry to `out` as one batch of `part`, placed by `xf`.
fn append(out: &mut MeshData, prop: &Prop, texture: Option<usize>, part: u16, xf: glam::Mat4, style: Style) {
    let base = out.vertices.len() as u32;
    let nm = xf.inverse().transpose();
    for i in 0..prop.positions.len() {
        let p = xf.transform_point3(prop.positions[i]);
        out.bounds_min = out.bounds_min.min(p);
        out.bounds_max = out.bounds_max.max(p);
        out.vertices.push(Vertex {
            pos: p.to_array(),
            normal: nm.transform_vector3(prop.normals[i]).normalize_or_zero().to_array(),
            color: style.prelit.unwrap_or([1.0; 4]),
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
        blend: style.blend,
        prelit: style.prelit.is_some(),
        additive: style.additive,
        part,
    });
}

fn clone_without_batches(mesh: &MeshData) -> MeshData {
    MeshData {
        vertices: mesh.vertices.clone(),
        indices: mesh.indices.clone(),
        batches: Vec::new(),
        textures: mesh.textures.clone(),
        bumps: mesh.bumps.clone(),
        tex_anims: mesh.tex_anims.clone(),
        seq_anims: mesh.seq_anims.clone(),
        bounds_min: mesh.bounds_min,
        bounds_max: mesh.bounds_max,
    }
}

/// `mesh` with every part that has a replacement swapped for it; `None` when
/// nothing is replaced (keep the original). A replacement draws the way the
/// part it stands for drew (alpha blended, additive).
pub fn with_parts(mesh: &MeshData, part_names: &[String]) -> Option<MeshData> {
    let dir = gdl_formats::texture::override_dir()?.join("parts");
    let found: Vec<(u16, Source)> = part_names
        .iter()
        .enumerate()
        .filter_map(|(p, n)| Some((p as u16, source_for(&dir, n)?)))
        .collect();
    if found.is_empty() {
        return None;
    }
    let mut out = clone_without_batches(mesh);
    let replaced: Vec<u16> = found.iter().map(|(p, _)| *p).collect();
    out.batches.extend(
        mesh.batches
            .iter()
            .filter(|b| !replaced.contains(&b.part))
            .cloned(),
    );
    let mut loaded = Loaded::default();
    for (part, src) in found {
        let Some((prop, texture)) = loaded.prop(&src, &mut out) else {
            out.batches
                .extend(mesh.batches.iter().filter(|b| b.part == part).cloned());
            continue;
        };
        // The model's baked light stays with the model: a replacement part
        // is lit by the engine instead (its texture is already its look).
        let style = Style {
            prelit: None,
            ..Style::of(
                mesh.batches.iter().filter(|b| b.part == part),
                &mesh.vertices,
                &mesh.indices,
            )
        };
        append(&mut out, &prop, texture, part, glam::Mat4::IDENTITY, style);
        log::info!("part {} <- {}", part_names[part as usize], match &src {
            Source::File(p) | Source::Frame(p, _) => p.display(),
        });
    }
    Some(out)
}

/// World objects with a replacement model: their indices (to leave out of
/// the world mesh) and the glb each one takes.
pub fn world_prop_sources(level: &Level) -> Vec<(usize, PathBuf)> {
    let Some(dir) = gdl_formats::texture::override_dir().map(|d| d.join("parts")) else {
        return Vec::new();
    };
    let mut seen: HashMap<String, Option<PathBuf>> = HashMap::new();
    level
        .worlds
        .world_objects
        .iter()
        .enumerate()
        .filter_map(|(i, o)| {
            let path = seen
                .entry(o.name.clone())
                .or_insert_with(|| {
                    let p = dir.join(format!("{}.glb", o.name));
                    p.is_file().then_some(p)
                })
                .clone()?;
            Some((i, path))
        })
        .collect()
}

/// Put the replacement models of `sources` (from [`world_prop_sources`], less
/// the objects in `skip`) into the world mesh, each where its object stands.
pub fn append_world_props(
    mesh: &mut MeshData,
    level: &Level,
    sources: &[(usize, PathBuf)],
    skip: &HashSet<usize>,
) {
    let mut props: HashMap<&Path, Option<(Prop, Option<usize>)>> = HashMap::new();
    let mut placed = 0;
    for (i, path) in sources {
        if skip.contains(i) {
            continue;
        }
        let entry = props.entry(path.as_path()).or_insert_with(|| {
            match Prop::load(path) {
                Ok(mut p) => {
                    let tex = p.texture.take().map(|t| push_texture(mesh, t));
                    Some((p, tex))
                }
                Err(e) => {
                    log::warn!("world override {}: {e:#}", path.display());
                    None
                }
            }
        });
        let Some((prop, tex)) = entry.as_ref() else {
            continue;
        };
        let pos = level.world_pos.get(*i).copied().unwrap_or_default();
        // The level's objects carry their light in vertex colours (x2 in
        // the shader); the replacement takes the object's mean colour so it
        // sits in the same light as its neighbours. An object drawn without
        // them (a flat card facing the camera, lit in full) is replaced by a
        // model drawn flat too, at its texture's colour, not shaded by the
        // engine's light, which would dim every side turned from it.
        let (orig, _) = level.objects_mesh(|j| j == *i);
        let mut style = Style::of(orig.batches.iter(), &orig.vertices, &orig.indices);
        style.prelit.get_or_insert([0.5, 0.5, 0.5, 1.0]);
        append(mesh, prop, *tex, 0, glam::Mat4::from_translation(pos.into()), style);
        placed += 1;
    }
    if placed > 0 {
        log::info!("world: {placed} objects replaced from {} models", props.len());
    }
}
