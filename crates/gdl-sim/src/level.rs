//! Level loading: world geometry, collision and item placements.

use crate::fs::GameFs;
use crate::mesh::{MeshBuilder, MeshData, TexBank};
use anyhow::{Context, Result};
use gdl_formats::anim::{Anim, TexMod};
use gdl_formats::{Objects, Worlds};
use glam::{Mat4, Vec3};
use std::collections::HashSet;

#[derive(Clone)]
pub struct Level {
    pub name: String,
    pub objects: Objects,
    pub textures_file: Vec<u8>,
    pub worlds: Worlds,
    /// Absolute position of each world object.
    pub world_pos: Vec<[f32; 3]>,
    /// The level's texture animations: the texmod table of its ANIM.PS2
    /// (`gWorldInfo.atreelist`, run by `DoWorldAnimation` every frame).
    pub texmods: Vec<TexMod>,
    /// WEAPONS' objects and textures, where external texmods (the torches'
    /// `TORCH00`...) find their frames; loaded only when needed.
    pub weapons: Option<(Objects, Vec<u8>)>,
}

impl Level {
    pub fn load(fs: &GameFs, name: &str) -> Result<Level> {
        let dir = format!("LEVELS/{name}");
        let objects =
            Objects::parse(&fs.read(&format!("{dir}/objects.ngc"))?).context("level objects")?;
        let textures_file = fs.read(&format!("{dir}/textures.ngc"))?;
        let worlds =
            Worlds::parse(&fs.read(&format!("{dir}/WORLDS.PS2"))?).context("level worlds")?;
        let world_pos = worlds.world_positions();
        let texmods = match fs.read(&format!("{dir}/ANIM.PS2")).map(|d| Anim::parse(&d)) {
            Ok(Ok(a)) => a.texmods,
            Ok(Err(e)) => {
                log::warn!("{name}: ANIM.PS2: {e:#}");
                Vec::new()
            }
            Err(_) => Vec::new(),
        };
        let weapons = if texmods.iter().any(|t| t.source == TexMod::EXTERNAL) {
            fs.read("WEAPONS/objects.ngc")
                .and_then(|d| Objects::parse(&d))
                .and_then(|o| Ok((o, fs.read("WEAPONS/textures.ngc")?)))
                .map_err(|e| log::warn!("WEAPONS textures: {e:#}"))
                .ok()
        } else {
            None
        };
        Ok(Level {
            name: name.to_string(),
            objects,
            textures_file,
            worlds,
            world_pos,
            texmods,
            weapons,
        })
    }

    /// Static world geometry: every world object that names a model in the level's
    /// objects file, except the moving ones in `skip`.
    pub fn world_mesh(&self, skip: &HashSet<usize>) -> MeshData {
        let (mesh, placed) = self.objects_mesh(|i| !skip.contains(&i));
        log::info!(
            "{}: placed {placed} of {} world objects ({} models in file)",
            self.name,
            self.worlds.world_objects.len(),
            self.objects.objects.len()
        );
        mesh
    }

    /// Geometry of the world objects selected by `want`, and how many were placed.
    pub fn objects_mesh(&self, want: impl Fn(usize) -> bool) -> (MeshData, usize) {
        let banks: Vec<TexBank> = self
            .weapons
            .iter()
            .map(|(objects, textures)| TexBank { objects, textures })
            .collect();
        let mut b = MeshBuilder::new(&self.objects, &self.textures_file)
            .with_texmods(&self.texmods, &banks);
        let mut placed = 0;
        for (i, (wo, pos)) in self
            .worlds
            .world_objects
            .iter()
            .zip(&self.world_pos)
            .enumerate()
        {
            if !want(i) {
                continue;
            }
            if let Some(obj) = self.objects.object_by_name(&wo.name) {
                b.additive = wo.mb_flags & (1 << 23) != 0;
                b.add_object(obj, Mat4::from_translation(Vec3::from(*pos)));
                placed += 1;
            }
        }
        (b.finish(), placed)
    }

    /// The world objects in `objs`, each its own rigid part (its index in
    /// `objs`), so moving objects can each be placed every frame.
    pub fn parts_mesh(&self, objs: &[usize]) -> MeshData {
        let banks: Vec<TexBank> = self
            .weapons
            .iter()
            .map(|(objects, textures)| TexBank { objects, textures })
            .collect();
        let mut b = MeshBuilder::new(&self.objects, &self.textures_file)
            .with_texmods(&self.texmods, &banks);
        for (k, &i) in objs.iter().enumerate() {
            let (Some(wo), Some(pos)) = (self.worlds.world_objects.get(i), self.world_pos.get(i))
            else {
                continue;
            };
            if let Some(obj) = self.objects.object_by_name(&wo.name) {
                b.additive = wo.mb_flags & (1 << 23) != 0;
                b.add_part(obj, Mat4::from_translation(Vec3::from(*pos)), k as u16);
            }
        }
        b.finish()
    }

    /// A world object and all of its descendants.
    pub fn subtree(&self, root: usize) -> Vec<usize> {
        let objs = &self.worlds.world_objects;
        let mut out = vec![root];
        let mut i = 0;
        while i < out.len() {
            let mut c = objs[out[i]].child_index;
            while c >= 0 && (c as usize) < objs.len() && !out.contains(&(c as usize)) {
                out.push(c as usize);
                c = objs[c as usize].next_index;
            }
            i += 1;
        }
        out
    }

    /// Player start positions (locators of type Start), falling back to the world centre.
    pub fn start_positions(&self) -> Vec<Vec3> {
        let v: Vec<Vec3> = self
            .worlds
            .locators
            .iter()
            .filter(|l| l.kind == gdl_formats::worlds::LocatorType::Start)
            .map(|l| Vec3::from(l.pos))
            .collect();
        if v.is_empty() {
            let w = &self.worlds;
            vec![(Vec3::from(w.min_bounds) + Vec3::from(w.max_bounds)) * 0.5]
        } else {
            v
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::TexAnimKind;

    fn data() -> Option<GameFs> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game-data");
        GameFs::open(&root).ok()
    }

    /// Every level's texture animations resolve: each flipbook frame decodes.
    /// `--nocapture` lists them with the extent of the geometry they cover.
    #[test]
    fn every_level_texmod_resolves() {
        let Some(fs) = data() else {
            eprintln!("game data not found; skipping");
            return;
        };
        let mut bad = Vec::new();
        for dir in fs.list_dirs("LEVELS") {
            let Ok(level) = Level::load(&fs, &dir) else {
                continue;
            };
            let mesh = level.world_mesh(&HashSet::new());
            let mut used = Vec::new();
            for a in &mesh.tex_anims {
                if let TexAnimKind::Flip(frames) = &a.kind {
                    if frames.iter().any(|f| f.is_none()) {
                        bad.push(format!("{dir} {}", a.texmod.name));
                    }
                }
                let mut lo = Vec3::splat(f32::MAX);
                let mut hi = Vec3::splat(f32::MIN);
                for b in mesh.batches.iter().filter(|b| b.texture == Some(a.slot)) {
                    for &i in &mesh.indices[b.start as usize..(b.start + b.count) as usize] {
                        let p = Vec3::from(mesh.vertices[i as usize].pos);
                        lo = lo.min(p);
                        hi = hi.max(p);
                    }
                }
                used.push(format!("{} {:.0?}..{:.0?}", a.texmod.name, lo, hi));
            }
            println!("{dir}: {}", used.join(" "));
        }
        assert!(bad.is_empty(), "undecodable frames: {bad:?}");
    }

    #[test]
    fn level_texture_animations() {
        let Some(fs) = data() else {
            eprintln!("game data not found; skipping");
            return;
        };
        let level = Level::load(&fs, "levelB6").unwrap();
        assert!(level.weapons.is_some(), "torches need WEAPONS' TORCH00");
        let mesh = level.world_mesh(&HashSet::new());
        let find = |name: &str| mesh.tex_anims.iter().find(|a| a.texmod.name == name);
        // The lava sea flips through LAVA00..LAVA29.
        let lava = find("LAVA").expect("lava texmod");
        let TexAnimKind::Flip(frames) = &lava.kind else {
            panic!("lava is a flipbook: {:?}", lava.kind);
        };
        assert_eq!(frames.len(), 30);
        assert!(frames.iter().all(|f| f.is_some()));
        assert!(mesh.batches.iter().any(|b| b.texture == Some(lava.slot)));
        // Its first frame is the LAVA00 bitmap, not LAVA's own glow stand-in.
        let img = &mesh.textures[frames[0].unwrap()];
        assert_eq!((img.width, img.height), (64, 64));
        assert_ne!(img.rgba, mesh.textures[lava.slot].rgba);
        // The sky scrolls in U.
        assert_eq!(find("SKY").map(|a| &a.kind), Some(&TexAnimKind::ScrollU));

        // levelC4's torches take 15 frames from WEAPONS (16x32 each).
        let level = Level::load(&fs, "levelC4").unwrap();
        let mesh = level.world_mesh(&HashSet::new());
        let torches: Vec<_> = mesh
            .tex_anims
            .iter()
            .filter(|a| a.texmod.name.starts_with("TORCH"))
            .collect();
        assert!(!torches.is_empty(), "levelC4 has torches");
        for t in torches {
            let TexAnimKind::Flip(frames) = &t.kind else {
                panic!("{:?}", t.kind)
            };
            assert_eq!(frames.len(), 15);
            let img = &mesh.textures[frames[0].unwrap()];
            assert_eq!((img.width, img.height), (16, 32));
        }
        // levelA1's moat water is a 30-frame flipbook.
        let level = Level::load(&fs, "levelA1").unwrap();
        let mesh = level.world_mesh(&HashSet::new());
        assert!(mesh.tex_anims.iter().any(|a| a.texmod.name == "MOAT_WATER"
            && matches!(&a.kind, TexAnimKind::Flip(f) if f.len() == 30)));
    }
}
