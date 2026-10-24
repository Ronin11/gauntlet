//! A modern skinned character standing in for a hero's rigid-part model.
//!
//! The glTF is posed from the hero's own animation (see `gdl_modern::retarget`)
//! and turned into an ordinary [`MeshData`], so it draws through the same
//! pipeline as every other model.

use crate::character::Model;
use crate::mesh::{Batch, MeshData, Vertex};
use anyhow::{Context, Result};
use gdl_formats::Image;
use gdl_modern::prop::{Mount, Prop};
use gdl_modern::{mapping, retarget, ModernModel};
use glam::{Mat4, Vec3};
use std::path::Path;

pub struct ModernHero {
    model: ModernModel,
    retarget: retarget::Retarget,
    /// The game pose the modern bind pose stands for.
    game_ref: Vec<Mat4>,
    scale: f32,
    /// Weapons riding on joints: the prop, the joint index and its placement.
    props: Vec<(Prop, usize, Mat4)>,
}

impl ModernHero {
    /// `ref_seq`/`ref_frame`: the game pose that the glTF's bind pose matches.
    pub fn load(
        path: impl AsRef<Path>,
        hero: &Model,
        ref_seq: usize,
        ref_frame: f32,
    ) -> Result<Self> {
        let mut model = ModernModel::load(path)?;
        model.separate_limbs(mapping::BIPED_LIMBS);
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
            props: Vec::new(),
        })
    }

    /// The mesh posed as the game's `game_now` node matrices, in game model
    /// space.
    pub fn mesh(&self, game_now: &[Mat4]) -> Result<MeshData> {
        let mats = self
            .retarget
            .skin_matrices(&self.model, &self.game_ref, game_now);
        self.mesh_from(&mats)
    }

    /// The mesh posed by a keyframe clip of the glTF at `time` seconds.
    pub fn mesh_clip(&self, clip: &gdl_modern::clip::Clip, time: f32) -> Result<MeshData> {
        self.mesh_from(&clip.skin_matrices(&self.model, time))
    }

    /// Carry the glTF at `path` on `joint`, placed by `mount`. `pose`: skin
    /// matrices of the pose an `aim` is given in.
    pub fn attach(
        &mut self,
        path: impl AsRef<Path>,
        joint: &str,
        mount: &Mount,
        pose: Option<&[Mat4]>,
    ) -> Result<()> {
        let prop = Prop::load(path)?;
        let j = self
            .model
            .joint(joint)
            .with_context(|| format!("no joint {joint}"))?;
        let mut mount = *mount;
        // An aim is given in game space; the joint wants it in model space.
        mount.aim = mount.aim.map(|a| self.retarget.from_game(a));
        // The aim is meant for the pose given (an arm hanging), else the bind pose.
        let rest = {
            let rest_world = self.model.joints[j].rest_world;
            let m = pose.map_or(rest_world, |p| p[j] * rest_world);
            glam::Quat::from_mat3(&glam::Mat3::from_cols(
                m.x_axis.truncate().normalize_or_zero(),
                m.y_axis.truncate().normalize_or_zero(),
                m.z_axis.truncate().normalize_or_zero(),
            ))
            .normalize()
        };
        let place = prop.placement(&mount, rest);
        self.props.push((prop, j, place));
        Ok(())
    }

    pub fn model(&self) -> &ModernModel {
        &self.model
    }

    fn mesh_from(&self, mats: &[Mat4]) -> Result<MeshData> {
        let posed = retarget::skin(&self.model, mats);
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
        for (prop, j, place) in &self.props {
            let pose = mats[*j] * self.model.joints[*j].rest_world * *place;
            let base = m.vertices.len() as u32;
            let start = m.indices.len() as u32;
            for ((p, n), uv) in prop.positions.iter().zip(&prop.normals).zip(&prop.uvs) {
                let q = self.retarget.to_game(pose.transform_point3(*p)) * self.scale;
                lo = lo.min(q);
                hi = hi.max(q);
                m.vertices.push(Vertex {
                    pos: q.into(),
                    normal: self
                        .retarget
                        .to_game(pose.transform_vector3(*n))
                        .normalize_or_zero()
                        .into(),
                    color: [1.0; 4],
                    uv: (*uv).into(),
                    lm_uv: [0.0; 2],
                });
            }
            for t in prop.indices.chunks_exact(3) {
                m.indices.extend(if mirrored {
                    [base + t[0], base + t[2], base + t[1]]
                } else {
                    [base + t[0], base + t[1], base + t[2]]
                });
            }
            let tex = prop
                .texture
                .as_ref()
                .context("prop has no base-colour texture")?;
            m.batches.push(Batch {
                start,
                count: m.indices.len() as u32 - start,
                texture: Some(m.textures.len()),
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
        }
        m.bounds_min = lo;
        m.bounds_max = hi;
        Ok(m)
    }
}
