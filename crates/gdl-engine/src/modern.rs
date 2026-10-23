//! A modern skinned character standing in for a hero's rigid-part model.
//!
//! The glTF is posed from the hero's own animation (see `gdl_modern::retarget`)
//! and turned into an ordinary [`MeshData`], so it draws through the same
//! pipeline as every other model.

use crate::character::Model;
use crate::mesh::{Batch, MeshData, Vertex};
use anyhow::{Context, Result};
use gdl_formats::Image;
use gdl_modern::{mapping, retarget, ModernModel};
use glam::{Mat4, Vec3};
use std::path::Path;

pub struct ModernHero {
    model: ModernModel,
    retarget: retarget::Retarget,
    /// The game pose the modern bind pose stands for.
    game_ref: Vec<Mat4>,
    scale: f32,
}

impl ModernHero {
    /// `ref_seq`/`ref_frame`: the game pose that the glTF's bind pose matches.
    pub fn load(
        path: impl AsRef<Path>,
        hero: &Model,
        ref_seq: usize,
        ref_frame: f32,
    ) -> Result<Self> {
        let model = ModernModel::load(path)?;
        let game_ref = hero.node_world(ref_seq, ref_frame);
        let names: Vec<&str> = hero
            .skeleton
            .nodes
            .iter()
            .map(|n| n.name.as_str())
            .collect();
        let retarget = retarget::Retarget::new(
            &model,
            &names,
            &game_ref,
            mapping::BIPED,
            mapping::BIPED_AIM,
            mapping::BIPED_FORWARD,
        )?;
        let scale = retarget.scale(&model, &game_ref);
        Ok(ModernHero {
            model,
            retarget,
            game_ref,
            scale,
        })
    }

    /// The mesh posed as the game's `game_now` node matrices, in game model
    /// space.
    pub fn mesh(&self, game_now: &[Mat4]) -> Result<MeshData> {
        let mats = self
            .retarget
            .skin_matrices(&self.model, &self.game_ref, game_now);
        let posed = retarget::skin(&self.model, &mats);
        let tex = self
            .model
            .texture
            .as_ref()
            .context("glTF has no base-colour texture")?;
        let mut m = MeshData::default();
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for ((p, n), v) in posed.iter().zip(&self.model.vertices) {
            let p = self.retarget.to_game(*p) * self.scale;
            lo = lo.min(p);
            hi = hi.max(p);
            m.vertices.push(Vertex {
                pos: p.into(),
                normal: self.retarget.to_game(*n).into(),
                color: [1.0; 4],
                uv: v.uv.into(),
                lm_uv: [0.0; 2],
            });
        }
        // A mirroring conversion reverses winding.
        let mirrored = self.retarget.mirrors();
        for t in self.model.indices.chunks_exact(3) {
            m.indices.extend(if mirrored {
                [t[0], t[2], t[1]]
            } else {
                [t[0], t[1], t[2]]
            });
        }
        m.batches.push(Batch {
            start: 0,
            count: m.indices.len() as u32,
            texture: Some(0),
            lightmap: None,
            blend: false,
            prelit: false,
            additive: false,
            part: 0,
        });
        m.textures.push(Image {
            width: tex.width,
            height: tex.height,
            rgba: tex.rgba.clone(),
        });
        m.bounds_min = lo;
        m.bounds_max = hi;
        Ok(m)
    }
}
