//! Level loading: world geometry, collision and item placements.

use crate::fs::GameFs;
use crate::mesh::{MeshBuilder, MeshData};
use anyhow::{Context, Result};
use gdl_formats::{Objects, Worlds};
use glam::{Mat4, Vec3};

pub struct Level {
    pub name: String,
    pub objects: Objects,
    pub textures_file: Vec<u8>,
    pub worlds: Worlds,
    /// Absolute position of each world object.
    pub world_pos: Vec<[f32; 3]>,
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
        Ok(Level {
            name: name.to_string(),
            objects,
            textures_file,
            worlds,
            world_pos,
        })
    }

    /// Static world geometry: every world object that names a model in the level's objects file.
    pub fn world_mesh(&self) -> MeshData {
        let mut b = MeshBuilder::new(&self.objects, &self.textures_file);
        let mut placed = 0;
        for (wo, pos) in self.worlds.world_objects.iter().zip(&self.world_pos) {
            if let Some(obj) = self.objects.object_by_name(&wo.name) {
                b.additive = wo.mb_flags & (1 << 23) != 0;
                b.add_object(obj, Mat4::from_translation(Vec3::from(*pos)));
                placed += 1;
            }
        }
        log::info!(
            "{}: placed {placed} of {} world objects ({} models in file)",
            self.name,
            self.worlds.world_objects.len(),
            self.objects.objects.len()
        );
        b.finish()
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
