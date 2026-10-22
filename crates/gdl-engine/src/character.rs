//! Animated rigid-part characters (players and monsters).

use crate::fs::GameFs;
use crate::mesh::{MeshBuilder, MeshData};
use anyhow::{Context, Result};
use gdl_formats::anim::{ATree, Anim, Pose};
use gdl_formats::Objects;
use glam::{Mat4, Vec3, Vec4};

/// The game's `CreateRYPMatrix` (ml_fmath.c): rotation from pitch/yaw/roll.
pub fn ryp_matrix(a: [f32; 3]) -> Mat4 {
    let (c0, s0) = (a[0].cos(), -a[0].sin());
    let (c1, s1) = (a[1].cos(), -a[1].sin());
    let (c2, s2) = (a[2].cos(), -a[2].sin());
    let aa = -c2 * s1;
    let b = -s2 * s1;
    Mat4::from_cols_array(&[
        c2 * c1, s2 * c1, s1, 0.0,
        -s2 * c0 + aa * s0, c2 * c0 + b * s0, c1 * s0, 0.0,
        s2 * s0 + aa * c0, -c2 * s0 + b * c0, c1 * c0, 0.0,
        0.0, 0.0, 0.0, 1.0,
    ])
}

/// The game's `CreatePYRMatrix`.
pub fn pyr_matrix(a: [f32; 3]) -> Mat4 {
    let (c0, s0) = (a[0].cos(), -a[0].sin());
    let (c1, s1) = (a[1].cos(), -a[1].sin());
    let (c2, s2) = (a[2].cos(), -a[2].sin());
    let aa = s0 * s1;
    let b = c0 * s1;
    Mat4::from_cols_array(&[
        c1 * c2, -aa * c2 + c0 * s2, b * c2 + s0 * s2, 0.0,
        -c1 * s2, aa * s2 + c0 * c2, b * -s2 + s0 * c2, 0.0,
        -s1, -s0 * c1, c0 * c1, 0.0,
        0.0, 0.0, 0.0, 1.0,
    ])
}

pub struct CharacterModel {
    pub mesh: MeshData,
    pub skeleton: ATree,
    /// Number of parts (one per skeleton node, plus one for the weapon).
    pub part_count: usize,
    weapon_node: Option<usize>,
}

impl CharacterModel {
    /// Load a player class (`WAR`, `VAL`, ...) in a colour variant (`RED`, `BLU`, ...).
    pub fn load_player(fs: &GameFs, class: &str, color: &str) -> Result<Self> {
        let anim = Anim::parse(&fs.read(&format!("PLAYERS/{class}/ANIM/ANIM.PS2"))?)?;
        let dir = format!("PLAYERS/{class}/{color}");
        let objects = Objects::parse(&fs.read(&format!("{dir}/objects.ngc"))?)?;
        let textures = fs.read(&format!("{dir}/textures.ngc"))?;
        let skeleton = anim.atrees.into_iter().next().context("player anim has no atree")?;
        Ok(Self::build(&objects, &textures, skeleton, &format!("{class}_{color}"), Some("WEAP_")))
    }

    /// Load a monster from `MONSTERS/<name>`.
    pub fn load_monster(fs: &GameFs, name: &str) -> Result<Self> {
        let dir = format!("MONSTERS/{name}");
        let anim = Anim::parse(&fs.read(&format!("{dir}/ANIM.PS2"))?)?;
        let objects = Objects::parse(&fs.read(&format!("{dir}/objects.ngc"))?)?;
        let textures = fs.read(&format!("{dir}/textures.ngc"))?;
        let skeleton = anim
            .atrees
            .into_iter()
            .max_by_key(|t| t.sequences.len())
            .context("monster anim has no atree")?;
        let prefix = skeleton.prefix.clone();
        Ok(Self::build(&objects, &textures, skeleton, &prefix, None))
    }

    fn build(objects: &Objects, textures: &[u8], skeleton: ATree, prefix: &str, weapon: Option<&str>) -> Self {
        let mut b = MeshBuilder::new(objects, textures);
        let weapon_node = skeleton.nodes.iter().position(|n| n.name == "R_WRIST");
        let weapon_part = skeleton.nodes.len() as u16;
        let mut weapon_added = false;
        for (def, obj) in objects.object_defs.iter().zip(&objects.objects) {
            let rest = def.name.strip_prefix(prefix).unwrap_or(&def.name);
            if let Some(ni) = skeleton.nodes.iter().position(|n| n.name.eq_ignore_ascii_case(rest)) {
                b.add_part(obj, Mat4::IDENTITY, ni as u16);
            } else if let Some(w) = weapon {
                // First weapon model (upgrades swap to HD2/HD3 later).
                if def.name.starts_with(w) && !weapon_added && weapon_node.is_some() {
                    b.add_part(obj, Mat4::IDENTITY, weapon_part);
                    weapon_added = true;
                }
            }
        }
        let part_count = skeleton.nodes.len() + 1;
        CharacterModel { mesh: b.finish(), skeleton, part_count, weapon_node }
    }

    /// Model-space transform of every part for the given node poses.
    pub fn part_transforms(&self, poses: &[Pose]) -> Vec<Mat4> {
        let nodes = &self.skeleton.nodes;
        let mut world: Vec<Mat4> = Vec::with_capacity(nodes.len() + 1);
        for (i, n) in nodes.iter().enumerate() {
            let p = poses.get(i).copied().unwrap_or_default();
            let rot = if p.pyr_order { pyr_matrix(p.pyr) } else { ryp_matrix(p.pyr) };
            let t = Vec3::from(n.init_pos) + Vec3::from(p.xyz);
            let local = Mat4::from_translation(t) * rot * Mat4::from_scale(Vec3::from(p.scale));
            let parent = usize::try_from(n.parent).ok().filter(|&pi| pi < i).map(|pi| world[pi]);
            world.push(parent.map_or(local, |pm| pm * local));
        }
        world.push(self.weapon_node.map_or(Mat4::IDENTITY, |w| world[w]));
        world
    }

    pub fn sequence(&self, name: &str) -> Option<usize> {
        self.skeleton.sequence_index(name)
    }
}

/// Playback state for one animated character.
#[derive(Clone, Debug)]
pub struct Animator {
    pub seq: usize,
    pub frame: f32,
    pub speed: f32,
    pub finished: bool,
}

impl Animator {
    pub fn new(seq: usize) -> Self {
        Animator { seq, frame: 0.0, speed: 1.0, finished: false }
    }

    pub fn play(&mut self, seq: usize) {
        if self.seq != seq {
            self.seq = seq;
            self.frame = 0.0;
            self.finished = false;
        }
    }

    pub fn update(&mut self, skel: &ATree, dt: f32) {
        let Some(s) = skel.sequences.get(self.seq) else { return };
        let frames = s.frame_count.max(1) as f32;
        self.frame += dt * s.frame_rate.max(1) as f32 * self.speed;
        if self.frame >= frames - 1.0 {
            if s.repeat {
                self.frame %= (frames - 1.0).max(1.0);
            } else {
                self.frame = frames - 1.0;
                self.finished = true;
            }
        }
    }

    pub fn poses(&self, skel: &ATree) -> Vec<Pose> {
        skel.sample(self.seq, self.frame)
    }
}

pub fn white() -> Vec4 {
    Vec4::ONE
}
