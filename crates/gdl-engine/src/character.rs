//! Animated models: rigid-part skeletons (players, many monsters, items) and
//! object animations that swap whole meshes per frame (grunts, barrels, ...).

use crate::fs::GameFs;
use crate::gpu::PartFx;
use crate::mesh::{MeshBuilder, MeshData, TexBank};
use crate::texanim::{self, SeqState};
use anyhow::{Context, Result};
use gdl_formats::anim::{ATree, Anim, Pose, TexMod, ANIM_OBJECT};
use gdl_formats::Objects;
use glam::{Mat4, Vec3};

/// The game's `CreateRYPMatrix` (ml_fmath.c): rotation from pitch/yaw/roll.
#[rustfmt::skip]
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
#[rustfmt::skip]
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

/// Mesh-swap frames of one object-animation node for one sequence.
#[derive(Clone, Debug)]
struct FrameRun {
    node: usize,
    frame_count: usize,
    /// Part index per frame.
    parts: Vec<u16>,
}

pub struct Model {
    pub mesh: MeshData,
    pub skeleton: ATree,
    /// Part index driven by each skeleton node (rigid parts).
    node_part: Vec<Option<u16>>,
    /// Extra parts attached to a node (weapon on the right wrist).
    attachments: Vec<(u16, usize)>,
    /// runs[seq] = mesh-swap runs active during that sequence.
    runs: Vec<Vec<FrameRun>>,
    /// Replacement parts shown instead of a node's own (a critter's broken
    /// armour plate `<prefix>D<node>`): (part, node).
    alt_parts: Vec<(u16, usize)>,
    pub part_count: usize,
    /// Sequence texmods and texmod nodes, linked to the mesh's flipbooks and
    /// parts (None when the model has none).
    texmods: Option<TexmodLinks>,
}

/// A texmod node: the parts of its subtree and its flipbook in `MeshData::seq_anims`.
struct TexmodNode {
    node: usize,
    anim: Option<usize>,
    parts: Vec<u16>,
}

struct TexmodLinks {
    /// seq[s][i]: the flipbook of sequence `s`'s texmod `i`.
    seq: Vec<Vec<Option<usize>>>,
    nodes: Vec<TexmodNode>,
    all_parts: Vec<u16>,
}

impl TexmodLinks {
    fn new(m: &Model) -> Option<Self> {
        let skel = &m.skeleton;
        let has_seq = skel.sequences.iter().any(|s| !s.texmods.is_empty());
        let has_nodes = skel.nodes.iter().any(|n| n.texmod.is_some());
        if !has_seq && !has_nodes {
            return None;
        }
        let anim_of = |tm: &TexMod| m.mesh.seq_anims.iter().position(|a| a.texmod == *tm);
        // The node each part hangs off.
        let mut part_node: Vec<Option<usize>> = vec![None; m.part_count];
        let mut set = |p: u16, n: usize| {
            if let Some(e) = part_node.get_mut(p as usize) {
                *e = Some(n);
            }
        };
        for (ni, p) in m.node_part.iter().enumerate() {
            if let Some(p) = p {
                set(*p, ni);
            }
        }
        for &(p, n) in m.attachments.iter().chain(&m.alt_parts) {
            set(p, n);
        }
        for r in m.runs.iter().flatten() {
            for &p in &r.parts {
                set(p, r.node);
            }
        }
        let nodes = skel
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(ni, n)| {
                let tm = n.texmod.as_ref()?;
                let parts = part_node
                    .iter()
                    .enumerate()
                    .filter(|(_, pn)| pn.is_some_and(|pn| m.in_subtree(pn, ni)))
                    .map(|(p, _)| p as u16)
                    .collect();
                Some(TexmodNode {
                    node: ni,
                    anim: anim_of(tm),
                    parts,
                })
            })
            .collect();
        Some(TexmodLinks {
            seq: skel
                .sequences
                .iter()
                .map(|s| s.texmods.iter().map(anim_of).collect())
                .collect(),
            nodes,
            all_parts: (0..m.part_count as u16).collect(),
        })
    }
}

/// Object names are limited to 15 characters, so compare against a truncated name.
fn name_matches(object: &str, full: &str) -> bool {
    let full = if full.len() > 15 { &full[..15] } else { full };
    object.eq_ignore_ascii_case(full)
}

impl Model {
    /// Load a player class (`WAR`, `VAL`, ...) in a colour variant (`RED`, `BLU`, ...).
    pub fn load_player(fs: &GameFs, class: &str, color: &str) -> Result<Self> {
        let dir = format!("PLAYERS/{class}/{color}");
        let objects = Objects::parse(&fs.read(&format!("{dir}/objects.ngc"))?)?;
        let textures = fs.read(&format!("{dir}/textures.ngc"))?;
        let prefix = format!("{class}_{color}");
        let anim = match fs.read(&format!("PLAYERS/{class}/ANIM/ANIM.PS2")) {
            Ok(d) => Anim::parse(&d)?,
            Err(_) => {
                // Secret heroes reuse a base class's animations: pick the skeleton
                // whose node names match the most of this model's parts.
                let mut best: Option<(usize, Anim)> = None;
                for base in ["WAR", "VAL", "WIZ", "ARC", "DWF", "KNI", "SOR", "JES"] {
                    let Ok(d) = fs.read(&format!("PLAYERS/{base}/ANIM/ANIM.PS2")) else {
                        continue;
                    };
                    let Ok(a) = Anim::parse(&d) else { continue };
                    let Some(t) = a.atrees.first() else { continue };
                    let score = t
                        .nodes
                        .iter()
                        .filter(|n| {
                            let full = format!("{prefix}{}", n.name);
                            objects
                                .object_defs
                                .iter()
                                .any(|d| name_matches(&d.name, &full))
                        })
                        .count();
                    if best.as_ref().is_none_or(|(s, _)| score > *s) {
                        best = Some((score, a));
                    }
                }
                best.context("no base animation set for hero")?.1
            }
        };
        let skeleton = anim
            .atrees
            .into_iter()
            .next()
            .context("player anim has no atree")?;
        // The model directory's own ANIM.PS2 carries its texmods (InitTexMods on
        // the model buffer, player.c), e.g. the wizard's RED_HANDGLOW, whose
        // frames come from WEAPONS.
        let texmods = fs
            .read(&format!("{dir}/ANIM.PS2"))
            .ok()
            .and_then(|d| Anim::parse(&d).ok())
            .map(|a| a.texmods)
            .unwrap_or_default();
        let weapons = if texmods.iter().any(|t| t.source == TexMod::EXTERNAL) {
            fs.read("WEAPONS/objects.ngc")
                .and_then(|d| Objects::parse(&d))
                .and_then(|o| Ok((o, fs.read("WEAPONS/textures.ngc")?)))
                .ok()
        } else {
            None
        };
        let banks: Vec<TexBank> = weapons
            .iter()
            .map(|(objects, textures)| TexBank { objects, textures })
            .collect();
        Ok(Self::build(
            &objects,
            &textures,
            skeleton,
            &format!("{class}_{color}"),
            Some("WEAP_"),
            &[],
            (&texmods, &banks),
        ))
    }

    /// Load the main animation tree of a monster directory (`MONSTERS/GRU`), picking
    /// the tree named `tree` if given, otherwise the one with the most sequences.
    pub fn load_monster(fs: &GameFs, dir: &str, tree: Option<&str>) -> Result<Self> {
        let path = format!("MONSTERS/{dir}");
        let anim = Anim::parse(&fs.read(&format!("{path}/ANIM.PS2"))?)?;
        let objects = Objects::parse(&fs.read(&format!("{path}/objects.ngc"))?)?;
        let textures = fs.read(&format!("{path}/textures.ngc"))?;
        Self::from_anim(&objects, &textures, &anim, tree)
    }

    /// Build a model from an already-parsed objects/anim pair.
    pub fn from_anim(
        objects: &Objects,
        textures: &[u8],
        anim: &Anim,
        tree: Option<&str>,
    ) -> Result<Self> {
        let skeleton = match tree {
            Some(t) => anim.atrees.iter().find(|a| a.name.eq_ignore_ascii_case(t)),
            None => anim.atrees.iter().max_by_key(|t| t.sequences.len()),
        }
        .with_context(|| format!("animation tree {tree:?} not found"))?
        .clone();
        let prefix = skeleton.prefix.clone();
        Ok(Self::build(
            objects,
            textures,
            skeleton,
            &prefix,
            None,
            &[],
            (&anim.texmods, &[]),
        ))
    }

    /// [`Model::from_anim`] plus replacement objects for some nodes, by name
    /// (shown with [`Model::evaluate_nodes`]'s `swapped`).
    pub fn from_anim_alts(
        objects: &Objects,
        textures: &[u8],
        anim: &Anim,
        tree: &str,
        alts: &[(String, String)],
    ) -> Result<Self> {
        let skeleton = anim
            .atrees
            .iter()
            .find(|a| a.name.eq_ignore_ascii_case(tree))
            .with_context(|| format!("animation tree {tree} not found"))?
            .clone();
        let prefix = skeleton.prefix.clone();
        let alts: Vec<(usize, &str)> = alts
            .iter()
            .filter_map(|(node, obj)| {
                let ni = skeleton
                    .nodes
                    .iter()
                    .position(|n| n.name.eq_ignore_ascii_case(node))?;
                Some((ni, obj.as_str()))
            })
            .collect();
        Ok(Self::build(
            objects,
            textures,
            skeleton,
            &prefix,
            None,
            &alts,
            (&anim.texmods, &[]),
        ))
    }

    /// `texmods`: the bank's texmod table and the other banks external
    /// texmods may draw from (see [`MeshBuilder::with_texmods`]).
    fn build(
        objects: &Objects,
        textures: &[u8],
        skeleton: ATree,
        prefix: &str,
        weapon: Option<&str>,
        alts: &[(usize, &str)],
        texmods: (&[TexMod], &[TexBank]),
    ) -> Self {
        let mut b = MeshBuilder::new(objects, textures).with_texmods(texmods.0, texmods.1);
        let mut next_part: u16 = 0;
        let mut node_part = vec![None; skeleton.nodes.len()];
        let defs = &objects.object_defs;

        // Rigid parts: object named <prefix><node>.
        for (ni, node) in skeleton.nodes.iter().enumerate() {
            if node.anim_type == ANIM_OBJECT {
                continue;
            }
            let full = format!("{prefix}{}", node.name);
            if let Some((oi, _)) = defs
                .iter()
                .enumerate()
                .find(|(_, d)| name_matches(&d.name, &full))
            {
                if let Some(obj) = objects.objects.get(defs[oi].obj_index.max(0) as usize) {
                    // Glow/fire nodes use additive "fb_add" blending.
                    b.additive = node.mb_flags & (1 << 23) != 0;
                    b.add_part(obj, Mat4::IDENTITY, next_part);
                    b.additive = false;
                    node_part[ni] = Some(next_part);
                    next_part += 1;
                }
            }
        }

        // Weapon attached to the right wrist (first of the upgrade models).
        let mut attachments = Vec::new();
        if let (Some(w), Some(wrist)) = (
            weapon,
            skeleton.nodes.iter().position(|n| n.name == "R_WRIST"),
        ) {
            if let Some(d) = defs.iter().find(|d| d.name.starts_with(w)) {
                if let Some(obj) = objects.objects.get(d.obj_index.max(0) as usize) {
                    b.add_part(obj, Mat4::IDENTITY, next_part);
                    attachments.push((next_part, wrist));
                    next_part += 1;
                }
            }
        }

        let mut alt_parts = Vec::new();
        for &(ni, name) in alts {
            if let Some(d) = defs.iter().find(|d| name_matches(&d.name, name)) {
                if let Some(obj) = objects.objects.get(d.obj_index.max(0) as usize) {
                    b.add_part(obj, Mat4::IDENTITY, next_part);
                    alt_parts.push((next_part, ni));
                    next_part += 1;
                }
            }
        }

        // Object animations: every frame mesh becomes its own part.
        let mut runs = vec![Vec::new(); skeleton.sequences.len()];
        let mut by_prefix: std::collections::HashMap<String, Vec<u16>> = Default::default();
        for (ni, node) in skeleton.nodes.iter().enumerate() {
            let Some(first) = node.obj_anim_first else {
                continue;
            };
            for (si, run) in runs.iter_mut().enumerate() {
                let Some(oa) = skeleton.obj_anims.get(first + si) else {
                    continue;
                };
                let p = oa.prefix().to_string();
                let parts = by_prefix
                    .entry(p.clone())
                    .or_insert_with(|| {
                        let mut frames: Vec<(&str, usize)> = defs
                            .iter()
                            .filter(|d| d.name.len() > p.len() && d.name.starts_with(&p))
                            .filter(|d| d.name[p.len()..].starts_with('F'))
                            .map(|d| (d.name.as_str(), d.obj_index.max(0) as usize))
                            .collect();
                        frames.sort();
                        let mut parts = Vec::new();
                        for (_, oi) in frames {
                            if let Some(obj) = objects.objects.get(oi) {
                                b.add_part(obj, Mat4::IDENTITY, next_part);
                                parts.push(next_part);
                                next_part += 1;
                            }
                        }
                        parts
                    })
                    .clone();
                if !parts.is_empty() {
                    run.push(FrameRun {
                        node: ni,
                        frame_count: oa.frame_count.max(1) as usize,
                        parts,
                    });
                }
            }
        }

        let mut model = Model {
            mesh: b.finish(),
            skeleton,
            node_part,
            attachments,
            runs,
            alt_parts,
            part_count: next_part as usize,
            texmods: None,
        };
        model.texmods = TexmodLinks::new(&model);
        model
    }

    /// Effects of the sequence texmods and texmod nodes on each part at frame
    /// `frame` of sequence `seq` (the texmod half of `DoAnimateTree` /
    /// `AnimateNode`); empty when the model has none.
    pub fn texmod_fx(&self, seq: usize, frame: f32) -> Vec<PartFx> {
        let (Some(links), Some(s)) = (&self.texmods, self.skeleton.sequences.get(seq)) else {
            return Vec::new();
        };
        let mut fx = vec![PartFx::default(); self.part_count];
        // (s32)(0.5 + frame), counted from the end for reversed sequences.
        let mut f = (0.5 + frame as f64) as i32;
        if s.flags & 1 != 0 {
            f = s.frame_count as i32 - f - 1;
        }
        let apply = |fx: &mut [PartFx], state: SeqState, anim: Option<usize>, parts: &[u16]| {
            for &p in parts {
                let Some(e) = fx.get_mut(p as usize) else {
                    continue;
                };
                match state {
                    SeqState::Frame(n) => e.alt = anim.map(|a| (a, n)),
                    SeqState::Uv(uv) => e.uv = Some(uv),
                    SeqState::Alpha(a) => e.alpha = Some(a),
                    SeqState::None => {}
                }
            }
        };
        // DoSeqTexMods on the root: the whole tree.
        let ids = links.seq.get(seq).map_or(&[][..], |v| v);
        for (state, &anim) in texanim::seq_states(&s.texmods, s.repeat, f)
            .into_iter()
            .zip(ids)
        {
            apply(&mut fx, state, anim, &links.all_parts);
        }
        // AnimateNode: each texmod node on its own subtree.
        for n in &links.nodes {
            if let Some(tm) = &self.skeleton.nodes[n.node].texmod {
                apply(&mut fx, texanim::seq_state(tm, f), n.anim, &n.parts);
            }
        }
        if fx.iter().all(|e| *e == PartFx::default()) {
            return Vec::new();
        }
        fx
    }

    /// A static (non-animated) model made of the named objects.
    pub fn from_objects(objects: &Objects, textures: &[u8], names: &[&str]) -> Option<Self> {
        Self::from_objects_anim(objects, textures, None, names)
    }

    /// [`Model::from_objects`], with the bank's texture animations (the global
    /// texmods of its ANIM.PS2) when given.
    pub fn from_objects_anim(
        objects: &Objects,
        textures: &[u8],
        anim: Option<&Anim>,
        names: &[&str],
    ) -> Option<Self> {
        let texmods = anim.map_or(&[][..], |a| &a.texmods);
        let mut b = MeshBuilder::new(objects, textures).with_texmods(texmods, &[]);
        let mut any = false;
        for n in names {
            if let Some(obj) = objects.object_by_name(n) {
                b.add_part(obj, Mat4::IDENTITY, 0);
                any = true;
            }
        }
        any.then(|| Model {
            mesh: b.finish(),
            skeleton: ATree {
                name: names.first().map(|s| s.to_string()).unwrap_or_default(),
                prefix: String::new(),
                nodes: Vec::new(),
                sequences: Vec::new(),
                tracks: Vec::new(),
                obj_anims: Vec::new(),
            },
            node_part: Vec::new(),
            attachments: Vec::new(),
            runs: Vec::new(),
            alt_parts: Vec::new(),
            part_count: 1,
            texmods: None,
        })
    }

    pub fn sequence(&self, name: &str) -> Option<usize> {
        self.skeleton.sequence_index(name)
    }

    /// Part transforms and visibility for a pose of sequence `seq` at `frame`.
    pub fn evaluate(&self, seq: usize, frame: f32) -> (Vec<Mat4>, Vec<bool>) {
        let poses = self.skeleton.sample(seq, frame);
        let world = self.node_transforms(&poses);
        self.evaluate_nodes(seq, frame, &world, &[], &[])
    }

    /// Index of the node called `name`.
    pub fn node_index(&self, name: &str) -> Option<usize> {
        self.skeleton
            .nodes
            .iter()
            .position(|n| n.name.eq_ignore_ascii_case(name))
    }

    /// The part a node draws itself (MBTreeSetFlags 1 hides just it).
    pub fn node_part(&self, node: usize) -> Option<u16> {
        self.node_part.get(node).copied().flatten()
    }

    /// The extra parts hung on nodes (the held weapon, weaphold_node).
    pub fn attachment_parts(&self) -> impl Iterator<Item = u16> + '_ {
        self.attachments.iter().map(|&(p, _)| p)
    }

    /// Whether node `n` is `root` or below it.
    pub fn in_subtree(&self, n: usize, root: usize) -> bool {
        let mut i = n as i32;
        while i >= 0 {
            if i as usize == root {
                return true;
            }
            let p = self.skeleton.nodes[i as usize].parent;
            if p >= i {
                return false;
            }
            i = p;
        }
        false
    }

    /// Model-space node matrices for a layered pose: the first layer's
    /// sequence drives the whole tree, and each later layer `(Some(root), seq,
    /// frame)` replaces the poses of `root`'s subtree (a critter's child parts,
    /// such as the Chimera's heads, animate their own branch of the shared tree).
    pub fn pose_layers(&self, layers: &[(Option<usize>, usize, f32)]) -> Vec<Mat4> {
        let Some(&(_, seq, frame)) = layers.first() else {
            return self.node_transforms(&[]);
        };
        let mut poses = self.skeleton.sample(seq, frame);
        for &(root, seq, frame) in &layers[1..] {
            let Some(root) = root else { continue };
            let sub = self.skeleton.sample(seq, frame);
            for (i, p) in poses.iter_mut().enumerate() {
                if self.in_subtree(i, root) {
                    if let Some(s) = sub.get(i) {
                        *p = *s;
                    }
                }
            }
        }
        self.node_transforms(&poses)
    }

    /// Part transforms and visibility from node matrices ([`Model::pose_layers`]):
    /// `hidden_nodes` hides a node's parts, `swapped` shows its replacement part.
    pub fn evaluate_nodes(
        &self,
        seq: usize,
        frame: f32,
        world: &[Mat4],
        hidden_nodes: &[bool],
        swapped: &[bool],
    ) -> (Vec<Mat4>, Vec<bool>) {
        let mut parts = vec![Mat4::IDENTITY; self.part_count];
        let mut hidden = vec![false; self.part_count];
        for (ni, p) in self.node_part.iter().enumerate() {
            if let Some(p) = p {
                parts[*p as usize] = world[ni];
            }
        }
        for &(p, node) in &self.attachments {
            parts[p as usize] = world[node];
        }
        // Hide every mesh-swap frame, then show the current one of each active run.
        for runs in &self.runs {
            for r in runs {
                for &p in &r.parts {
                    hidden[p as usize] = true;
                }
            }
        }
        if let Some(runs) = self.runs.get(seq) {
            for r in runs {
                let f = (frame.max(0.0) as usize).min(r.frame_count - 1);
                let idx = (f * r.parts.len() / r.frame_count).min(r.parts.len() - 1);
                let p = r.parts[idx] as usize;
                hidden[p] = false;
                parts[p] = world[r.node];
            }
        }
        for &(p, node) in &self.alt_parts {
            parts[p as usize] = world[node];
            hidden[p as usize] = !swapped.get(node).copied().unwrap_or(false);
        }
        for (ni, p) in self.node_part.iter().enumerate() {
            if let Some(p) = p {
                if hidden_nodes.get(ni).copied().unwrap_or(false)
                    || (swapped.get(ni).copied().unwrap_or(false)
                        && self.alt_parts.iter().any(|&(_, n)| n == ni))
                {
                    hidden[*p as usize] = true;
                }
            }
        }
        (parts, hidden)
    }

    /// A skeleton node's model-space matrix at a frame of a sequence.
    pub fn node_matrix(&self, seq: usize, frame: f32, name: &str) -> Option<Mat4> {
        let i = self
            .skeleton
            .nodes
            .iter()
            .position(|n| n.name.eq_ignore_ascii_case(name))?;
        let poses = self.skeleton.sample(seq, frame);
        self.node_transforms(&poses).get(i).copied()
    }

    fn node_transforms(&self, poses: &[Pose]) -> Vec<Mat4> {
        let nodes = &self.skeleton.nodes;
        let mut world: Vec<Mat4> = Vec::with_capacity(nodes.len());
        for (i, n) in nodes.iter().enumerate() {
            let p = poses.get(i).copied().unwrap_or_default();
            let rot = if p.pyr_order {
                pyr_matrix(p.pyr)
            } else {
                ryp_matrix(p.pyr)
            };
            let t = Vec3::from(n.init_pos) + Vec3::from(p.xyz);
            let local = Mat4::from_translation(t) * rot * Mat4::from_scale(Vec3::from(p.scale));
            let parent = usize::try_from(n.parent)
                .ok()
                .filter(|&pi| pi < i)
                .map(|pi| world[pi]);
            world.push(parent.map_or(local, |pm| pm * local));
        }
        world
    }
}

/// Playback state for one animated model.
#[derive(Clone, Debug)]
pub struct Animator {
    pub seq: usize,
    pub frame: f32,
    pub speed: f32,
    pub finished: bool,
    /// Loop even when the sequence itself isn't flagged as repeating.
    pub force_loop: bool,
}

impl Animator {
    pub fn new(seq: usize) -> Self {
        Animator {
            seq,
            frame: 0.0,
            speed: 1.0,
            finished: false,
            force_loop: false,
        }
    }

    pub fn play(&mut self, seq: usize) {
        if self.seq != seq {
            self.restart(seq);
        }
    }

    pub fn restart(&mut self, seq: usize) {
        self.seq = seq;
        self.frame = 0.0;
        self.finished = false;
    }

    pub fn update(&mut self, skel: &ATree, dt: f32) {
        let Some(s) = skel.sequences.get(self.seq) else {
            return;
        };
        let frames = s.frame_count.max(1) as f32;
        if s.repeat || self.force_loop {
            // For looping sequences `finished` pulses for one update per loop.
            self.finished = false;
        }
        // InitAnim / CalcAnimInfo: a frame lasts rate/30 x animscale x 1/30 s, so
        // `rate` is an inverse speed (30 plays 30 frames a second, 60 plays 15);
        // `speed` is 1/animscale. A sequence is done half a frame past its last.
        let rate = if s.frame_rate > 0 {
            s.frame_rate as f32
        } else {
            30.0
        };
        self.frame += dt * 900.0 / rate * self.speed;
        let end = (frames - 0.5).max(0.5);
        if self.frame >= end {
            if s.repeat || self.force_loop {
                // A loop restarts from frame 0.
                self.frame = 0.0;
            } else {
                self.frame = (frames - 1.0).max(0.0);
            }
            self.finished = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::TexAnimKind;

    fn bank(dir: &str) -> Option<(Objects, Vec<u8>, Anim)> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game-data");
        let fs = GameFs::open(&root).ok()?;
        Some((
            Objects::parse(&fs.read(&format!("{dir}/objects.ngc")).ok()?).ok()?,
            fs.read(&format!("{dir}/textures.ngc")).ok()?,
            Anim::parse(&fs.read(&format!("{dir}/ANIM.PS2")).ok()?).ok()?,
        ))
    }

    #[test]
    fn sequence_texmods_flip_the_coin() {
        let Some((objects, textures, anim)) = bank("ITEMS/levelC5") else {
            eprintln!("game data not found; skipping");
            return;
        };
        let m = Model::from_anim(&objects, &textures, &anim, Some("COIN_GOLD")).unwrap();
        // ACTIVE's texmod flips NEL_COIN through NEL_COIN00..28.
        let id = m
            .mesh
            .seq_anims
            .iter()
            .position(|a| a.texmod.name == "NEL_COIN")
            .expect("NEL_COIN flipbook");
        let TexAnimKind::Flip(frames) = &m.mesh.seq_anims[id].kind else {
            panic!()
        };
        assert_eq!(frames.len(), 29);
        let seq = m.sequence("ACTIVE").unwrap();
        // Frame 6.6 rounds to 7 ((s32)(0.5 + frame)); rate 1.
        let fx = m.texmod_fx(seq, 6.6);
        assert_eq!(fx.len(), m.part_count);
        assert!(fx.iter().all(|e| e.alt == Some((id, 7))));
        // A model without texmods costs nothing.
        let Some((objects, textures, anim)) = bank("MONSTERS/GRU") else {
            return;
        };
        let m = Model::from_anim(&objects, &textures, &anim, Some("GRU1")).unwrap();
        assert!(m.texmod_fx(0, 3.0).is_empty());
        // The grunt's thrown bomb flips BOMB00..04 with its flight sequence.
        let m = Model::from_anim(&objects, &textures, &anim, Some("GRU_BOMB")).unwrap();
        let fx = m.texmod_fx(0, 3.0);
        assert!(!fx.is_empty() && fx.iter().all(|e| e.alt.map(|a| a.1) == Some(3)));
    }
}
