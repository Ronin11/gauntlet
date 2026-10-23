//! The critter runtime (enemy/critter.c): bosses, golems, generals and
//! gargoyles, driven by the move, pattern, damage and colnode tables of their
//! `CRITTER/*.WAD` ([`gdl_formats::critter`]).
//!
//! A critter is a root part plus child parts (the Chimera's three heads) that
//! share one model: each child animates its own branch of the tree. The root
//! moves and turns; children follow it. Every frame [`Game::update_critters`]
//! runs, per root, the targeting, difficulty, move choice, animation, frame
//! events (damage), translation and rotation as `ProcessCritter`,
//! `CritterBossAI` and `CritterGolemAI` do.
//!
//! What critter.c hands to the effect system (sfx.c) is run as
//! ProcessEffects runs it: projectiles become enemy shots living their
//! effect's time, ground waves (expanding effects) grow and weaken over
//! theirs, and visual effects are the effect atrees of the monster directory
//! played once.

use crate::character::{Animator, Model};
use crate::game::{Game, Kind};
use gdl_formats::critter::{CritterWad, Damage, Move, Vis};
use glam::{Mat4, Vec2, Vec3};
use std::sync::Arc;

// MOVETYPE
pub const M_INIT: i32 = 0;
pub const M_SYNC: i32 = 1;
pub const M_START: i32 = 0x10;
pub const M_DEATH: i32 = 0x11;
pub const M_READY: i32 = 0x20;
pub const M_TAUNT: i32 = 0x21;
pub const M_ROAR: i32 = 0x22;
pub const M_BLOCK: i32 = 0x23;
pub const M_STEP_FIRST: i32 = 0x30;
pub const M_STEPL: i32 = 0x32;
pub const M_STEPR: i32 = 0x33;
pub const M_STEPB: i32 = 0x35;
pub const M_STEPFL: i32 = 0x36;
pub const M_STEPTOWARD: i32 = 0x38;
pub const M_STEP_LAST: i32 = 0x39;
pub const M_HITREACT: i32 = 0x40;
pub const M_KNOCKBACK: i32 = 0x41;
pub const M_KNOCKDOWN: i32 = 0x42;
pub const M_ATTACKS: i32 = 0x7F;
pub const M_CLAW: i32 = 0x80;
pub const M_GRAB: i32 = 0x81;
pub const M_BREATH: i32 = 0x83;
pub const M_SPRAY: i32 = 0x85;
pub const M_CHARGE: i32 = 0x86;
pub const M_TARGETED: i32 = 0x88;
pub const M_FINISH: i32 = 0xF0;

// CRIT_ETYPE
pub const ET_GOLEM: i16 = 3;
pub const ET_BOSS: i16 = 4;
pub const ET_GARGOYLE: i16 = 7;
pub const ET_GENERAL: i16 = 8;

// C_DMG bits used here.
pub const DMG_KNOCKBACK: u32 = 0x10;
pub const DMG_KNOCKDOWN: u32 = 0x20;
pub const DMG_BLOWNAWAY: u32 = 0x40;
pub const DMG_KNOCKOVER: u32 = 0x100;
pub const DMG_MAGIC: u32 = 0x200;
pub const DMG_POISONGAS: u32 = 0x800;
pub const DMG_THROWN: u32 = 0x8000;
pub const DMG_WHIRLWIND: u32 = 0x10000;
pub const DMG_SUPER: u32 = 0x100000;
pub const DMG_NOHITFX: u32 = 0x1000000;

/// lbl_8011AEAC: the ROAR burst (x50 damage within 3 s) by players in play.
pub const ROAR_SCALE: [f32; 5] = [1.0, 1.0, 1.5, 2.0, 2.0];
/// lbl_8011AEC0: what bosses take of a hit, by players in play.
pub const BOSS_PLAYER_SCALE: [f32; 5] = [1.0, 1.0, 0.5, 0.3, 0.2];
/// NODE/CONE hits skip a player hit in the last 0.25 s (player.fxhittime).
const PLAYER_REHIT: f32 = 0.25;
/// An enemy shot that brings Garm's escorts where it lands.
pub const SHOT_ADDS: u32 = 0x8000_0000;

/// One entry of a part's target list (CritterTargetInfo).
#[derive(Clone, Copy, Debug)]
pub struct Target {
    pub pidx: usize,
    pub dist: f32,
    pub dp: f32,
    pub testdist: f32,
    pub dpos: Vec3,
    pub invanger: f32,
}

/// Damage a player dealt a part and received from it (playerDamage).
#[derive(Clone, Copy, Debug, Default)]
pub struct PlayerDamage {
    pub received: f32,
    pub received_time: f32,
    pub dealt: f32,
    pub dealt_time: f32,
}

/// One critter instance: the root or a child part (`Critter`, 0xAE0).
#[derive(Clone, Debug)]
pub struct Part {
    /// TYPE index in the WAD.
    pub ty: usize,
    /// 0 INIT, 1 DYING, 2 IDLE, 3 ACTIVE.
    pub state: u8,
    pub health: f32,
    /// The spawn health (type health x ene_health).
    pub health0: f32,
    /// 0.5 at full health up to 5.0 at none (rateScale).
    pub difficulty: f32,
    /// Current, desired move (type-local, -1 none).
    pub cur: i32,
    pub next: i32,
    /// Running pattern, pattern chosen this frame (-2: a reaction), index in it.
    pub pattern: i32,
    pub dpattern: i32,
    pub patidx: i32,
    /// Current target player, the one chosen with the move, the one held.
    pub tgt: i32,
    pub dtgt: i32,
    pub grab: i32,
    pub targets: Vec<Target>,
    pub movetime: Vec<f32>,
    pub patterntime: Vec<f32>,
    /// Hold the current move until this time (rate).
    pub hold: f32,
    pub movedone: bool,
    /// Frame events already fired for this move (moveFlags) and its sounds.
    pub events: u8,
    pub sfx_events: u8,
    /// Damage taken in the current 3 s burst (counterValue), its flags and time.
    pub dmgtaken: f32,
    pub dmgtype: u32,
    pub dmgtime: f32,
    pub kb_in: Vec3,
    pub kb_vel: Vec3,
    pub pdamage: [PlayerDamage; 4],
    /// Per colnode (type-local): damage absorbed and the cap.
    pub node_damage: Vec<(f32, f32)>,
    pub anim: Animator,
    /// Child parts: the model node of their branch.
    pub root_node: Option<usize>,
    /// Model node per type-local colnode, and per move (its colnode).
    pub colnode_nodes: Vec<Option<usize>>,
    pub move_nodes: Vec<Option<usize>>,
    /// The move node's world position now and last frame (moveOrigin / prevpos).
    pub node_pos: Vec3,
    pub prev_pos: Vec3,
    pub node_mat: Mat4,
    /// Snapshot of the target's position for FOUNTAIN (TARGETED moves).
    pub tgtpos: Vec3,
    /// pausecnt (frozen), unkAC6 (blinded) in ticks, unkAC8 (attack cap, s).
    pub freeze: f32,
    pub blind: f32,
    pub fball_cap: f32,
    /// A hit flash (s).
    pub flash: f32,
    /// A child's branch is hidden once its death has played out.
    pub gone: bool,
}

/// A spawned critter (root + children).
#[derive(Clone, Debug)]
pub struct Critter {
    pub id: u64,
    pub wad: Arc<CritterWad>,
    /// CRIT_ETYPE of its DESC (3 golem, 4 boss, 7 gargoyle, 8 general).
    pub etype: i16,
    pub prefix: String,
    pub parts: Vec<Part>,
    pub init_yaw: f32,
    pub init_pos: Vec3,
    /// Leash centre and the root's current distance from it (unk4AC).
    pub movecenter: Vec3,
    pub leash: f32,
    /// World matrices of the model's nodes this frame (game space).
    pub nodes: Vec<Mat4>,
    /// Nodes whose object was swapped for its broken `<prefix>D<node>` one,
    /// and nodes removed with it (their moves are gone).
    pub swapped: Vec<bool>,
    pub removed: Vec<bool>,
    /// Model per SFXX record (its effect atree), when one loaded.
    pub fx_models: Vec<Option<usize>>,
    /// 1 opaque .. 0 invisible.
    pub alpha: f32,
    /// Sight radius for golems (placed `rad` x ene_visrad), 0 = none.
    pub visrad: f32,
    /// Gargoyles drop GARG<kind>.
    pub garg_kind: Option<String>,
    /// The hit node from the last collision query (hitnodeidx): part, colnode.
    pub hitnode: Option<(usize, usize)>,
    /// Set when the instance is deleted.
    pub deleted: bool,
    /// The death has started (boss: BossDying sent).
    pub dying_sent: bool,
    /// Its model (in Game::models).
    pub model: Option<usize>,
    /// typeFlags 0x800: the in-world GMETER model and its RED_FILLE node.
    pub gmeter: Option<(usize, usize)>,
    /// A tint and scale set on it (the Bellows' green, 0.8 size).
    pub tint: Option<[f32; 4]>,
    pub scale: f32,
}

impl PartialEq for Critter {
    fn eq(&self, o: &Critter) -> bool {
        self.id == o.id
    }
}

/// What the world is like for a critter this frame.
#[derive(Clone, Copy, Debug)]
pub struct PInfo {
    pub pos: Vec3,
    /// GetPlayerColPos / effectpos: the body centre.
    pub col: Vec3,
    pub active: bool,
    /// Invisible (player flags & 4).
    pub hidden: bool,
    pub attacking: bool,
}

/// ModifyDamage (combat.c:410) with a target's shield bits and armour.
pub fn modify_damage(
    mut dmg: f32,
    mut flags: u32,
    resist: u32,
    armor: f32,
    boss_level: bool,
) -> (f32, u32) {
    if resist & 0x100000 != 0 {
        dmg = if dmg > 1.0 { -0.1 * dmg } else { 0.0 };
    }
    if resist & 0x10000 != 0
        || (resist & 0x1000 != 0 && flags & DMG_MAGIC != 0)
        || (resist & 0x2000 != 0 && flags & DMG_POISONGAS != 0)
    {
        dmg = 0.0;
    }
    let (weak, res, strong) = if boss_level {
        (0.75, 1.25, 1.5)
    } else {
        (0.5, 1.5, 2.0)
    };
    if resist & 0x40000 != 0 {
        flags &= !0x10170;
    }
    if resist & 0x10 != 0 && flags & DMG_MAGIC != 0 {
        dmg *= weak;
    }
    if flags & (DMG_MAGIC | DMG_POISONGAS) == 0 {
        dmg = if dmg >= 0.0 {
            (dmg - armor).max(0.0)
        } else {
            -dmg
        };
    }
    let elem = flags & 0xF;
    if dmg > 0.0 && (1..=4).contains(&elem) {
        let e = elem - 1;
        let opposite = [1, 0, 3, 2][e as usize];
        if resist & (1 << e) != 0 {
            dmg *= weak;
        } else if resist & (0x100 << e) != 0 {
            dmg = 0.0;
        } else if resist & (1 << opposite) != 0 {
            dmg *= strong;
        } else {
            dmg *= res;
        }
    }
    (dmg, flags)
}

fn wrap(mut a: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    while a > PI {
        a -= TAU;
    }
    while a <= -PI {
        a += TAU;
    }
    a
}

/// YawVec3(v, -a) in the game's convention: turn a direction by +a.
fn turn(v: Vec3, a: f32) -> Vec3 {
    let (s, c) = a.sin_cos();
    Vec3::new(v.x * c + v.z * s, v.y, v.z * c - v.x * s)
}

/// PitchVec3: positive pitch tips the direction down.
fn pitch(v: Vec3, p: f32) -> Vec3 {
    let (s, c) = p.sin_cos();
    Vec3::new(
        s * v.x * v.y + v.x * c,
        v.y * c - s * (v.x * v.x + v.z * v.z),
        s * v.y * v.z + v.z * c,
    )
}

fn yaw_dir(yaw: f32) -> Vec3 {
    Vec3::new(yaw.sin(), 0.0, yaw.cos())
}

impl Part {
    fn new(wad: &CritterWad, ty: usize, ene_health: f32) -> Part {
        let t = &wad.types[ty];
        let health = t.health * ene_health;
        let nm = t.nmoves.max(0) as usize;
        Part {
            ty,
            state: 0,
            health,
            health0: health,
            difficulty: 0.5,
            cur: -1,
            next: -1,
            pattern: -1,
            dpattern: -1,
            patidx: -1,
            tgt: -1,
            dtgt: -1,
            grab: -1,
            targets: Vec::new(),
            movetime: vec![0.0; nm],
            patterntime: vec![0.0; t.npatterns.max(0) as usize],
            hold: 0.0,
            movedone: false,
            events: 0,
            sfx_events: 0,
            dmgtaken: 0.0,
            dmgtype: 0,
            dmgtime: 0.0,
            kb_in: Vec3::ZERO,
            kb_vel: Vec3::ZERO,
            pdamage: [PlayerDamage::default(); 4],
            node_damage: wad
                .type_colnodes(ty)
                .iter()
                .map(|n| (0.0, n.maxdamage * health))
                .collect(),
            anim: Animator::new(0),
            root_node: None,
            colnode_nodes: Vec::new(),
            move_nodes: Vec::new(),
            node_pos: Vec3::ZERO,
            prev_pos: Vec3::ZERO,
            node_mat: Mat4::IDENTITY,
            tgtpos: Vec3::ZERO,
            freeze: 0.0,
            blind: 0.0,
            fball_cap: 0.0,
            flash: 0.0,
            gone: false,
        }
    }

    pub fn moves<'a>(&self, w: &'a CritterWad) -> &'a [Move] {
        w.type_moves(self.ty)
    }

    fn mv<'a>(&self, w: &'a CritterWad, i: i32) -> Option<&'a Move> {
        usize::try_from(i).ok().and_then(|i| self.moves(w).get(i))
    }

    /// CritterFindMoveType: the off-cooldown move of `kind` whose cooldown ran
    /// out first; mode != 0 takes any (READY as the fallback).
    pub fn find_move(&self, w: &CritterWad, kind: i32, mode: u8, now: f32) -> i32 {
        let mut result = -1;
        let mut best = 0.0f32;
        for (i, m) in self.moves(w).iter().enumerate() {
            if m.flags & 4 != 0 || m.kind != kind {
                continue;
            }
            let remaining = if m.interval > 0.0 {
                self.movetime[i] + m.interval - now
            } else {
                0.0
            };
            if (remaining <= 0.0 || mode != 0) && (result < 0 || remaining < best) {
                best = remaining;
                result = i as i32;
                if mode == 2 {
                    break;
                }
            }
        }
        if result < 0 && mode != 0 && kind != M_READY {
            return self.find_move(w, M_READY, 1, now);
        }
        result
    }

    /// CritterSetDifficulty.
    fn set_difficulty(&mut self, w: &CritterWad, ene_health: f32) {
        let ratio = self.health / (1.0 + w.types[self.ty].health * ene_health);
        self.difficulty = 0.5 + (1.0 - ratio) * 4.5;
    }

    /// CritterPlayerAnger: players who hurt it most (for what it did to them)
    /// come first.
    fn anger(&self, p: usize) -> f32 {
        let d = self.pdamage[p];
        if d.dealt < 1.0 {
            return 11.0;
        }
        (d.received / d.dealt).clamp(0.01, 10.0)
    }

    /// CritterReCalcTarget: score target `k` against a move's criteria.
    fn recalc(&self, vis: Option<&Vis>, k: usize, fwd: Vec3) -> f32 {
        let e = &self.targets[k];
        if let Some(v) = vis {
            if self.difficulty < v.dif_min {
                return 1.2e21;
            }
            if v.dif_max > v.dif_min && self.difficulty >= v.dif_max {
                return 1.2e21;
            }
            if e.dist < v.min {
                return 1.01e21;
            }
            if v.max > 0.0 && e.dist > v.max {
                return 1.02e21;
            }
            let f = turn(fwd, v.ang);
            let f = Vec3::new(f.x, 0.0, f.z).normalize_or_zero();
            let dot = e.dpos.x * f.x + e.dpos.z * f.z;
            if dot < v.arc {
                return 1.1e21;
            }
        }
        e.dist * e.invanger
    }
}

/// CritterCalcTarget: a score for aiming at `target` (lower is better;
/// >= 1e21 rejected), with the target record.
fn calc_target(
    difficulty: f32,
    vis: Option<&Vis>,
    pos: Vec3,
    fwd: Vec3,
    target: Vec3,
) -> (f32, f32, f32, Vec3) {
    if let Some(v) = vis {
        if difficulty < v.dif_min {
            return (1.2e21, 0.0, 0.0, Vec3::ZERO);
        }
        if v.dif_max > 0.0 && difficulty >= v.dif_max {
            return (1.2e21, 0.0, 0.0, Vec3::ZERO);
        }
    }
    let mut d = target - pos;
    let dy = d.y;
    d.y = 0.0;
    let dist = d.length();
    let dn = if dist > 0.0 { d / dist } else { Vec3::ZERO };
    let f = match vis {
        Some(v) => {
            if dist < v.min {
                return (1.01e21, 0.0, dist, dn);
            }
            if v.max > 0.0 && dist > v.max {
                return (1.02e21, 0.0, dist, dn);
            }
            if v.maxdy > 0.0 && dy.abs() > v.maxdy {
                return (1.03e21, 0.0, dist, dn);
            }
            turn(fwd, v.ang)
        }
        None => fwd,
    };
    let f = Vec3::new(f.x, 0.0, f.z).normalize_or_zero();
    let dot = dn.x * f.x + dn.z * f.z;
    if let Some(v) = vis {
        if dot < v.arc {
            return (1.1e21, dot, dist, dn);
        }
    }
    let score = if dot > 0.5 {
        dist / dot.abs()
    } else {
        2.0 * dist
    };
    (score, dot, dist, dn)
}

impl Critter {
    /// CritterNewInst: the root type and its child chain, at full health.
    pub fn new(
        id: u64,
        wad: Arc<CritterWad>,
        root: usize,
        model: &Model,
        pos: Vec3,
        yaw: f32,
        ene_health: f32,
    ) -> Critter {
        let w = &*wad;
        let desc = &w.descs[w.types[root].descidx.max(0) as usize];
        let mut parts = vec![Part::new(w, root, ene_health)];
        let mut t = w.types[root].childtype;
        while t >= 0 && (t as usize) < w.types.len() && parts.len() < 8 {
            let mut p = Part::new(w, t as usize, ene_health);
            p.root_node = model.node_index(&w.types[t as usize].rootnode);
            parts.push(p);
            t = w.types[t as usize].childtype;
        }
        for p in &mut parts {
            p.colnode_nodes = w
                .type_colnodes(p.ty)
                .iter()
                .map(|n| model.node_index(&n.node))
                .collect();
            p.move_nodes = w
                .type_moves(p.ty)
                .iter()
                .map(|m| {
                    if m.colnode.is_empty() {
                        None
                    } else {
                        model.node_index(&m.colnode)
                    }
                })
                .collect();
            p.anim = Animator::new(0);
        }
        let t0 = &w.types[root];
        let movecenter = if t0.movecenter[1] >= 999.0 {
            pos
        } else {
            Vec3::from(t0.movecenter)
        };
        let n = model.skeleton.nodes.len();
        Critter {
            id,
            etype: desc.etype,
            prefix: desc.prefix.clone(),
            parts,
            init_yaw: yaw,
            init_pos: pos,
            movecenter,
            leash: 0.0,
            nodes: vec![Mat4::IDENTITY; n],
            swapped: vec![false; n],
            removed: vec![false; n],
            fx_models: Vec::new(),
            alpha: 1.0,
            visrad: 0.0,
            garg_kind: None,
            hitnode: None,
            deleted: false,
            dying_sent: false,
            model: None,
            gmeter: None,
            tint: None,
            scale: 1.0,
            wad,
        }
    }

    pub fn is_boss(&self) -> bool {
        self.etype == ET_BOSS
    }

    /// The pose layers for drawing: the root's sequence, then each live
    /// child's own branch when it runs its own move.
    pub fn layers(&self) -> Vec<(Option<usize>, usize, f32)> {
        let mut v = vec![(None, self.parts[0].anim.seq, self.parts[0].anim.frame)];
        for p in &self.parts[1..] {
            if p.root_node.is_some() && p.cur >= 0 {
                v.push((p.root_node, p.anim.seq, p.anim.frame));
            }
        }
        v
    }

    /// Nodes to hide: the branches of children whose death has played out.
    pub fn hidden_nodes(&self, model: &Model) -> Vec<bool> {
        let mut h = self.removed.clone();
        for p in &self.parts[1..] {
            if let (true, Some(r)) = (p.gone, p.root_node) {
                for (i, x) in h.iter_mut().enumerate() {
                    if model.in_subtree(i, r) {
                        *x = true;
                    }
                }
            }
        }
        h
    }

    /// Refresh the node world matrices for the current pose.
    pub fn refresh_nodes(&mut self, model: &Model, pos: Vec3, yaw: f32) {
        let body = Mat4::from_translation(pos) * Mat4::from_rotation_y(yaw);
        self.nodes = model
            .pose_layers(&self.layers())
            .into_iter()
            .map(|m| body * m)
            .collect();
    }

    fn node_world(&self, node: Option<usize>, pos: Vec3, yaw: f32) -> Mat4 {
        match node.and_then(|n| self.nodes.get(n)) {
            Some(m) => *m,
            None => Mat4::from_translation(pos) * Mat4::from_rotation_y(yaw),
        }
    }

    /// The body centre (coll_pos) of part `pi`.
    pub fn coll_pos(&self, pi: usize, pos: Vec3, yaw: f32) -> Vec3 {
        let t = &self.wad.types[self.parts[pi].ty];
        pos + Mat4::from_rotation_y(yaw).transform_vector3(Vec3::from(t.coll))
    }

    /// Whether the critter can be hit (`CritterDamage` ignores state < 2).
    pub fn hittable(&self) -> bool {
        self.parts[0].state >= 2 && !self.deleted
    }

    /// The live colnodes: (part, colnode, world centre, radius).
    pub fn colliders(&self) -> Vec<(usize, usize, Vec3, f32)> {
        let mut v = Vec::new();
        for (pi, p) in self.parts.iter().enumerate() {
            if p.gone || p.state < 2 {
                continue;
            }
            for (ci, cn) in self.wad.type_colnodes(p.ty).iter().enumerate() {
                let Some(node) = p.colnode_nodes.get(ci).copied().flatten() else {
                    continue;
                };
                if self.removed.get(node).copied().unwrap_or(false) || cn.rad <= 0.2 {
                    continue;
                }
                // Exhausted nodes are skipped by the collision queries.
                let (d, max) = p.node_damage[ci];
                if max > 0.0 && d >= max {
                    continue;
                }
                let c = self.nodes[node].transform_point3(Vec3::from(cn.offset));
                v.push((pi, ci, c, cn.rad));
            }
        }
        v
    }

    /// The distance from `from` to the critter's surface, and the node it is
    /// nearest (CritterLineCollide / CritterExpCollide): colnode spheres when
    /// the type uses them (flags & 2, or child parts), else the body cylinder.
    pub fn surface(&self, from: Vec3, pos: Vec3, yaw: f32) -> (f32, Option<(usize, usize)>) {
        let t = &self.wad.types[self.parts[0].ty];
        let c = self.coll_pos(0, pos, yaw);
        let horiz = Vec2::new(from.x - c.x, from.z - c.z).length() - t.rad;
        let vert = (from.y - c.y).abs() - t.hht.max(1.0);
        let mut best = (horiz.max(vert), None);
        if t.flags & 2 != 0 || self.parts.len() > 1 {
            for (pi, ci, p, r) in self.colliders() {
                let d = (from - p).length() - r;
                if d < best.0 {
                    best = (d, Some((pi, ci)));
                }
            }
        }
        best
    }
}

impl Game {
    /// The players as critters see them.
    fn critter_players(&self) -> Vec<PInfo> {
        (0..4)
            .map(|i| match self.players.get(i) {
                Some(p) => PInfo {
                    pos: p.pos,
                    col: p.pos + Vec3::Y * 2.5,
                    active: p.state.active(),
                    hidden: p.fx.special & crate::powerups::S_INVISIBLE != 0,
                    attacking: (2..=12).contains(&p.act.action),
                },
                None => PInfo {
                    pos: Vec3::ZERO,
                    col: Vec3::ZERO,
                    active: false,
                    hidden: false,
                    attacking: false,
                },
            })
            .collect()
    }

    /// Players in play (NumPlayers, lbl_8034465C).
    pub(crate) fn num_active_players(&self) -> usize {
        self.players
            .iter()
            .filter(|p| p.state.active())
            .count()
            .min(4)
    }

    /// The level's boss-quest state (lbl_8034489C), 0 without a boss.
    pub(crate) fn legend_state(&self) -> u8 {
        self.boss.as_ref().map_or(0, |b| b.legend_state)
    }

    /// ProcessCritterList: every critter, once a frame.
    pub(crate) fn update_critters(&mut self, dt: f32) {
        let players = self.critter_players();
        for ei in 0..self.entities.len() {
            if !self.entities[ei].alive || !matches!(self.entities[ei].kind, Kind::Critter(_)) {
                continue;
            }
            let Kind::Critter(mut c) = std::mem::replace(&mut self.entities[ei].kind, Kind::Prop)
            else {
                unreachable!()
            };
            self.process_critter(ei, &mut c, &players, dt);
            let deleted = c.deleted;
            self.entities[ei].kind = Kind::Critter(c);
            if deleted {
                self.entities[ei].alive = false;
            }
        }
    }

    /// ProcessCritter + the AI for one root critter.
    fn process_critter(&mut self, ei: usize, c: &mut Critter, players: &[PInfo], dt: f32) {
        let now = self.time;
        let Some(model) = self.entities[ei].model else {
            return;
        };
        let (pos, yaw) = (self.entities[ei].pos, self.entities[ei].yaw);
        c.refresh_nodes(&self.models[model], pos, yaw);
        let w = c.wad.clone();
        for p in &mut c.parts {
            p.flash = (p.flash - dt).max(0.0);
            if p.freeze > 0.0 {
                p.freeze = (p.freeze - dt * 60.0).max(0.0);
            }
            if p.blind > 0.0 {
                p.blind = (p.blind - dt * 60.0).max(0.0);
            }
            // CritterUpdateCounters.
            let kind = p.mv(&w, p.cur).map_or(0, |m| m.kind);
            if (p.dmgtime > 0.0 && now - p.dmgtime > 3.0)
                || kind == M_ROAR
                || (M_HITREACT..M_ATTACKS).contains(&kind)
            {
                p.dmgtaken = 0.0;
                p.dmgtype = 0;
                p.dmgtime = 0.0;
            }
            for d in &mut p.pdamage {
                if d.received_time > 0.0 && now - d.received_time > 15.0 {
                    d.received = 0.0;
                    d.received_time = 0.0;
                }
                if d.dealt_time > 0.0 && now - d.dealt_time > 15.0 {
                    d.dealt = 0.0;
                    d.dealt_time = 0.0;
                }
            }
        }
        // CritterDoKnockback (not bosses).
        if !c.is_boss() {
            let p = &mut c.parts[0];
            let mut scale = if p.health <= 0.0 {
                20.0
            } else if p.dmgtype & (DMG_WHIRLWIND | DMG_KNOCKOVER | DMG_BLOWNAWAY) != 0 {
                10.0
            } else if p.dmgtype & DMG_KNOCKDOWN != 0 {
                7.5
            } else if p.dmgtype & DMG_KNOCKBACK != 0 {
                5.0
            } else {
                0.0
            };
            if c.etype == ET_GOLEM {
                scale -= 5.0;
            }
            if scale > 0.0 {
                p.kb_vel += p.kb_in * scale;
                if p.kb_vel.length() > 40.0 {
                    p.kb_vel = p.kb_vel.normalize() * 40.0;
                }
                p.kb_in = Vec3::ZERO;
            }
        }
        // Every child dead: the root dies too.
        if c.parts.len() > 1
            && c.parts[0].state == 3
            && c.parts[1..].iter().all(|p| p.health <= 0.0)
        {
            c.parts[0].health = -1.0;
        }
        if c.parts[0].health <= 0.0 && c.parts[0].state != 1 && c.parts[0].state >= 2 {
            self.critter_kill(c, 0);
        }
        if c.is_boss() {
            self.boss_ai(ei, c, players, dt);
        } else {
            self.golem_ai(ei, c, players, dt);
        }
    }

    /// CritterGetTargetPlayers for part `pi`.
    fn get_targets(&self, c: &mut Critter, pi: usize, players: &[PInfo], pos: Vec3, yaw: f32) {
        let boss = c.is_boss();
        let cp = c.coll_pos(pi, pos, yaw);
        let fwd = yaw_dir(yaw);
        let w = c.wad.clone();
        let visrad = c.visrad;
        let p = &mut c.parts[pi];
        p.targets.clear();
        if p.health <= 0.0 {
            return;
        }
        let vis = w.types[p.ty].vis;
        for (i, pl) in players.iter().enumerate() {
            if !pl.active {
                continue;
            }
            if pl.hidden && p.state != 0 && !boss {
                continue;
            }
            let (score, dp, dist, dpos) = calc_target(p.difficulty, Some(&vis), cp, fwd, pl.col);
            if !boss && visrad > 0.0 && score > visrad {
                continue;
            }
            if score >= 1e21 {
                continue;
            }
            let mut testdist = score;
            if self.time < self.crit_hit_until.get(i).copied().unwrap_or(0.0) {
                testdist *= 1000.0;
            }
            let invanger = p.anger(i);
            let t = Target {
                pidx: i,
                dist,
                dp,
                testdist: testdist * invanger,
                dpos,
                invanger,
            };
            let at = p
                .targets
                .iter()
                .position(|x| t.testdist < x.testdist)
                .unwrap_or(p.targets.len());
            p.targets.insert(at, t);
            p.targets.truncate(4);
        }
    }

    /// CritterGetTargetSub: the best player for a move's criteria (mode 0
    /// rejects all-rejected lists; otherwise the parent is asked).
    fn target_sub(&self, c: &Critter, pi: usize, vis: Option<&Vis>, mode: u8, yaw: f32) -> i32 {
        if let Some(v) = vis {
            if v.maxrad > 0.0 && c.leash > v.maxrad {
                return -1;
            }
        }
        let p = &c.parts[pi];
        let fwd = yaw_dir(yaw);
        let mut best = -1i32;
        let mut score = 2e21f32;
        for k in 0..p.targets.len() {
            let s = p.recalc(vis, k, fwd);
            if best < 0 || s < score {
                best = k as i32;
                score = s;
            }
        }
        if mode == 0 && score >= 1e21 {
            best = -1;
        }
        if best >= 0 {
            p.targets[best as usize].pidx as i32
        } else if mode != 0 && pi != 0 {
            self.target_sub(c, 0, vis, mode, yaw)
        } else {
            -1
        }
    }

    /// CritterGetDoAction: lifecycle, links, the legend sequence and reactions.
    fn get_do_action(&self, c: &mut Critter, pi: usize) {
        let w = c.wad.clone();
        let now = self.time;
        let legend = self.legend_state();
        let boss_type = self.boss.as_ref().map_or(-1, |b| b.boss_type);
        let etype = c.etype;
        let p = &mut c.parts[pi];
        let cur = p.mv(&w, p.cur).cloned();
        let ck = cur.as_ref().map_or(-1, |m| m.kind);
        if p.cur < 0 || p.state == 0 || p.state == 2 {
            p.next = p.find_move(&w, M_INIT, 1, now);
        } else if ck == M_INIT {
            p.next = p.find_move(&w, M_START, 0, now);
        } else if p.state == 1 {
            p.next = p.find_move(&w, M_DEATH, 1, now);
        } else if cur.as_ref().is_some_and(|m| m.nextidx >= 0) {
            p.next = cur.as_ref().unwrap().nextidx as i32;
        } else if (1..=2).contains(&legend) {
            p.next = p.find_move(&w, M_READY, 1, now);
        } else if legend == 3 && ck != M_ROAR {
            p.next = p.find_move(&w, M_ROAR, 1, now);
        } else if (3..=5).contains(&legend) && boss_type == 35 {
            p.next = p.find_move(&w, M_READY, 0, now);
        } else if ck == M_START && etype == ET_BOSS {
            p.next = p.find_move(&w, M_READY, 0, now);
        }
        Self::reactions(p, &w, now, self.num_active_players());
        if p.next >= 0 && p.pattern >= 0 {
            for q in &mut c.parts {
                q.pattern = -1;
                q.patidx = -1;
            }
        }
        c.parts[pi].dmgtype &= !0x130;
    }

    /// The hit reactions shared by the root and child choosers.
    fn reactions(p: &mut Part, w: &CritterWad, now: f32, np: usize) {
        if p.next < 0 && p.dmgtype & 0x120 != 0 {
            if p.dmgtype & 0x100 != 0 {
                p.next = p.find_move(w, M_KNOCKDOWN, 0, now);
            }
            if p.next < 0 {
                p.next = p.find_move(w, M_KNOCKBACK, 0, now);
            }
        }
        if p.next < 0 && p.dmgtaken >= (50.0 * ROAR_SCALE[np.min(4)]) as i32 as f32 {
            p.next = p.find_move(w, M_ROAR, 0, now);
        }
        if p.next < 0 && p.dmgtype & 0x10 != 0 {
            p.next = p.find_move(w, M_HITREACT, 0, now);
        }
    }

    /// CritterChildGetPattern: a child's forced moves.
    fn child_get_pattern(&self, c: &mut Critter, pi: usize) {
        let w = c.wad.clone();
        let now = self.time;
        let legend = self.legend_state();
        let boss_type = self.boss.as_ref().map_or(-1, |b| b.boss_type);
        let np = self.num_active_players();
        let p = &mut c.parts[pi];
        if p.state == 1 {
            p.next = p.find_move(&w, M_DEATH, 1, now);
        } else if (3..=5).contains(&legend) && boss_type == 35 {
            p.next = p.find_move(&w, M_READY, 0, now);
        } else if let Some(m) = p.mv(&w, p.cur) {
            if m.nextidx >= 0 {
                p.next = m.nextidx as i32;
            }
        }
        Self::reactions(p, &w, now, np);
        if p.next >= 0 {
            p.dpattern = -2;
        }
        p.dmgtype &= !0x130;
    }

    fn move_usable(c: &Critter, pi: usize, m: &Move, idx: usize) -> bool {
        let p = &c.parts[pi];
        if m.flags & 4 != 0 {
            return false;
        }
        if m.flags & 2 != 0 && pi == 0 && c.parts[1..].iter().any(|q| q.health <= 0.0) {
            return false;
        }
        if m.flags & 0x10 != 0 {
            let alive = |i: usize| {
                p.move_nodes
                    .get(i)
                    .copied()
                    .flatten()
                    .is_some_and(|n| !c.removed[n])
            };
            if !alive(idx) {
                return false;
            }
            if m.nextidx >= 0 && !alive(m.nextidx as usize) {
                return false;
            }
        }
        true
    }

    /// CritterLookForCriticalMove: BLOCK against an attacking target.
    fn look_for_critical(&self, c: &mut Critter, pi: usize, players: &[PInfo], yaw: f32) {
        let w = c.wad.clone();
        let now = self.time;
        let moves = c.parts[pi].moves(&w);
        for (i, m) in moves.iter().enumerate() {
            if m.kind != M_BLOCK || !Self::move_usable(c, pi, m, i) {
                continue;
            }
            if m.interval > 0.0 && now < c.parts[pi].movetime[i] + m.interval {
                continue;
            }
            let t = self.target_sub(c, pi, Some(&m.vis), 0, yaw);
            if t >= 0 && players[t as usize].attacking {
                c.parts[pi].next = i as i32;
            }
        }
    }

    /// CritterChildCriticalMove: carry on a pattern, or pick the least recently
    /// used eligible pattern or attack.
    fn child_critical(&self, c: &mut Critter, pi: usize, yaw: f32) {
        let w = c.wad.clone();
        let now = self.time;
        {
            let p = &mut c.parts[pi];
            if p.pattern >= 0 && p.patidx + 1 < 8 {
                let m = w.type_patterns(p.ty)[p.pattern as usize].moves[(p.patidx + 1) as usize];
                if m >= 0 {
                    p.next = m as i32;
                    p.dpattern = p.pattern;
                    return;
                }
            }
        }
        let mut best = 999999.0f32;
        let (mut pat, mut mv, mut who) = (-1i32, -1i32, -1i32);
        let kids_dead = pi == 0 && c.parts[1..].iter().any(|q| q.health <= 0.0);
        for (i, pt) in w.type_patterns(c.parts[pi].ty).iter().enumerate() {
            if i as i32 == c.parts[pi].pattern
                || (pt.flags & 2 != 0 && kids_dead)
                || pt.flags & 0x1000 != 0
            {
                continue;
            }
            let t = c.parts[pi].patterntime[i];
            if now < t + pt.interval {
                continue;
            }
            let p = self.target_sub(c, pi, Some(&pt.vis), 0, yaw);
            if p >= 0 && t < best {
                pat = i as i32;
                who = p;
                best = t;
            }
        }
        let moves = c.parts[pi].moves(&w);
        for (i, m) in moves.iter().enumerate() {
            if i as i32 == c.parts[pi].cur || m.kind < M_ATTACKS || m.kind >= M_FINISH {
                continue;
            }
            if !Self::move_usable(c, pi, m, i) {
                continue;
            }
            let grab = c.parts[pi].grab;
            if grab >= 0 && m.kind == M_GRAB {
                pat = -1;
                mv = i as i32;
                who = grab;
                break;
            }
            let t = c.parts[pi].movetime[i];
            if m.interval > 0.0 && now < t + m.interval {
                continue;
            }
            let p = self.target_sub(c, pi, Some(&m.vis), 0, yaw);
            if p < 0 {
                continue;
            }
            if t < best {
                best = t;
                pat = -1;
                mv = i as i32;
                who = p;
            } else if pat < 0 && (mv < 0 || interrupt(&moves[mv as usize], m) > 1) {
                best = t;
                mv = i as i32;
                who = p;
            }
        }
        let p = &mut c.parts[pi];
        if pat >= 0 {
            p.dpattern = pat;
            p.dtgt = who;
            p.next = w.type_patterns(p.ty)[pat as usize].moves[0] as i32;
        } else if mv >= 0 {
            p.next = mv;
            p.dtgt = who;
        }
    }

    /// CritterLookForReady: the best step move toward the target.
    fn look_for_ready(&self, c: &mut Critter, pi: usize, players: &[PInfo], pos: Vec3, yaw: f32) {
        let w = c.wad.clone();
        let now = self.time;
        let p = &c.parts[pi];
        if w.types[p.ty].flags & 0x10000 == 0
            && !w
                .type_moves(p.ty)
                .iter()
                .any(|m| (M_STEP_FIRST..=M_STEP_LAST).contains(&m.kind))
        {
            return;
        }
        // CritterGetTarget: the best target's position.
        let Some(t) = p.targets.first() else { return };
        let tpos = players[t.pidx].col;
        let cp = c.coll_pos(pi, pos, yaw);
        let fwd = yaw_dir(yaw);
        let mut best = 1e21f32;
        let mut result = -1;
        for (i, m) in p.moves(&w).iter().enumerate() {
            if !(M_STEP_FIRST..=M_STEP_LAST).contains(&m.kind) || m.flags & 4 != 0 {
                continue;
            }
            if m.kind == M_STEPTOWARD && p.tgt < 0 {
                continue;
            }
            if m.interval > 0.0 && now < p.movetime[i] + m.interval {
                continue;
            }
            let (s, ..) = calc_target(p.difficulty, Some(&m.vis), cp, fwd, tpos);
            if s < best {
                best = s;
                result = i as i32;
            }
        }
        let p = &mut c.parts[pi];
        p.tgtpos = tpos;
        if result >= 0 {
            p.next = result;
        }
    }

    fn taunt_or_ready(c: &mut Critter, pi: usize, now: f32) {
        let w = c.wad.clone();
        let p = &mut c.parts[pi];
        let mut s = -1;
        if p.difficulty < 0.8 {
            s = p.find_move(&w, M_TAUNT, 0, now);
        }
        if s < 0 {
            s = p.find_move(&w, M_READY, 1, now);
        }
        p.next = s;
    }

    /// CritterAnimate: switch to the desired move as its interrupt policy
    /// allows, advance the animation, and hand finished moves to MoveDone.
    fn critter_animate(&mut self, c: &mut Critter, pi: usize, model: usize, dt: f32) {
        let w = c.wad.clone();
        let now = self.time;
        let skel = &self.models[model].skeleton;
        let p = &mut c.parts[pi];
        // GetDmove: the pattern's next move, else the desired move.
        let mut cand = p.next;
        if p.dpattern >= 0 && p.patidx >= 0 && p.patidx < 7 {
            let m = w.type_patterns(p.ty)[p.dpattern as usize].moves[(p.patidx + 1) as usize];
            if m >= 0 {
                cand = m as i32;
            }
        }
        let cur = p.mv(&w, p.cur).cloned();
        let next = p.mv(&w, cand).cloned();
        let mut next_index = cand;
        let seq_of = |m: &Move| skel.sequence_index(&m.anim).unwrap_or(0);
        let (seq, mut transition) = match (&cur, &next) {
            (None, None) => (p.anim.seq, 0),
            (Some(c0), None) => (seq_of(c0), 0),
            (_, Some(n)) => {
                if Some(cand) != Some(p.cur)
                    && n.priority >= 0xF00
                    && cur.as_ref().is_none_or(|c0| c0.interrupt != 0)
                {
                    (seq_of(n), 3)
                } else if p.hold > now {
                    next_index = p.cur;
                    (cur.as_ref().map_or(p.anim.seq, seq_of), 0)
                } else {
                    match &cur {
                        Some(c0) => (seq_of(n), interrupt(c0, n)),
                        None => (seq_of(n), 3),
                    }
                }
            }
        };
        let same = next_index == p.cur;
        let mut done = false;
        if p.freeze > 0.0 {
            // pausecnt: the animation is frozen.
        } else if transition >= 2 && !same {
            p.anim.restart(seq);
            p.anim.force_loop = false;
            done = true;
        } else {
            p.anim.update(skel, dt);
            // AnimDone: a finished (or wrapping) animation lets a move whose
            // policy refused the switch hand over at its end.
            let ended = p.anim.finished;
            if !same && transition == 0 && now > p.hold && ended {
                transition = 1;
            }
            if ended && transition >= 1 {
                let looping = skel.sequences.get(seq).is_some_and(|s| s.repeat);
                if !same || looping || seq != p.anim.seq {
                    if seq != p.anim.seq || !same {
                        p.anim.restart(seq);
                    }
                    done = true;
                }
            }
        }
        if cur.as_ref().is_some_and(|c0| c0.kind == M_INIT) && same {
            done = false;
        }
        p.movedone = done;
        if done {
            let frames = skel
                .sequences
                .get(p.anim.seq)
                .map_or(2, |s| s.frame_count as i32);
            self.move_done(c, pi, next_index, frames);
        } else if next_index < 0 && c.parts[pi].anim.finished {
            c.parts[pi].cur = -1;
        }
    }

    /// CritterMoveDone: pattern progress, cooldowns and the legend sequence.
    fn move_done(&mut self, c: &mut Critter, pi: usize, new: i32, frames: i32) {
        let w = c.wad.clone();
        let now = self.time;
        if let Some(m) = c.parts[pi].mv(&w, c.parts[pi].cur) {
            if let Some(b) = self.boss.as_mut() {
                if m.kind == M_START && m.nextidx < 0 && b.legend_state == 1 {
                    b.legend_state = 2;
                    b.legend_timer = now;
                }
                if m.kind == M_ROAR && b.legend_state == 3 {
                    b.legend_state = 4;
                }
            }
        }
        let root_pattern = c.parts[0].pattern;
        let p = &mut c.parts[pi];
        if p.pattern >= 0 {
            if pi == 0 || root_pattern < 0 {
                p.patidx += 1;
                let next = if p.patidx < 8 {
                    w.type_patterns(p.ty)[p.pattern as usize].moves[p.patidx as usize]
                } else {
                    -1
                };
                if next < 0 {
                    if pi == 0 {
                        for q in &mut c.parts {
                            q.pattern = -1;
                            q.patidx = -1;
                        }
                    } else {
                        p.pattern = -1;
                        p.patidx = -1;
                    }
                }
            }
        } else if p.dpattern >= 0 {
            p.patterntime[p.dpattern as usize] = now;
            p.pattern = p.dpattern;
            p.patidx = 0;
        } else if new >= 0 && (new as usize) < p.movetime.len() {
            p.movetime[new as usize] = now + (frames - 2).max(0) as f32 / 30.0;
        }
        let p = &mut c.parts[pi];
        p.cur = new;
        p.hold = 0.0;
    }

    /// CritterBossAI.
    fn boss_ai(&mut self, ei: usize, c: &mut Critter, players: &[PInfo], dt: f32) {
        let now = self.time;
        let (pos, yaw) = (self.entities[ei].pos, self.entities[ei].yaw);
        let model = self.entities[ei].model.unwrap_or(0);
        let w = c.wad.clone();
        let eh = self.tuning.health;
        for pi in 0..c.parts.len() {
            self.get_targets(c, pi, players, pos, yaw);
            c.parts[pi].set_difficulty(&w, eh);
        }
        // CritterProcessSafeRocks (SafeRockHide: the WAD has a SPOUT2).
        let hide = w.damage.iter().any(|d| d.kind == 6);
        self.process_safe_rocks(hide, dt);
        // Wake-up.
        if c.parts[0].state == 0 {
            let t0 = &w.types[c.parts[0].ty];
            let far = c.parts[0]
                .targets
                .iter()
                .map(|t| t.dist)
                .fold(0.0f32, f32::max);
            let far = if far <= 0.0 { 1e21 } else { far };
            let b = self.boss.get_or_insert_with(Default::default);
            if b.wake_time == 0.0 {
                // typeFlags 0x80 (the Chimera) waits for its floor to move:
                // see the floor snap below.
                if t0.flags & 0x80 == 0 {
                    b.wake_time = now + 2.0;
                }
            } else if now >= b.wake_time && (t0.activedist <= 0.0 || far < t0.activedist) {
                // CritterActivateChain.
                for p in &mut c.parts {
                    p.state = 3;
                }
                self.boss_activate(true);
            }
        }
        // Move choice.
        for p in &mut c.parts {
            p.next = -1;
            p.dpattern = -1;
            p.dtgt = -1;
        }
        self.get_do_action(c, 0);
        if c.parts[0].next < 0 {
            self.look_for_critical(c, 0, players, yaw);
        }
        if c.parts[0].next < 0 {
            self.child_critical(c, 0, yaw);
            let mut linked = 0;
            for ci in 1..c.parts.len() {
                let rp = (c.parts[0].pattern, c.parts[0].patidx, c.parts[0].next);
                if rp.0 >= 0 {
                    let ch = &mut c.parts[ci];
                    if (rp.0 as usize) < w.type_patterns(ch.ty).len() {
                        ch.pattern = rp.0;
                        ch.patidx = rp.1;
                        ch.next = w.type_patterns(ch.ty)[rp.0 as usize].moves
                            [rp.1.clamp(0, 7) as usize] as i32;
                    }
                } else if rp.2 < 0 {
                    self.child_get_pattern(c, ci);
                    if c.parts[ci].next < 0 {
                        self.child_critical(c, ci, yaw);
                    }
                    let sel = c.parts[ci].next;
                    if sel < 0 {
                        Self::taunt_or_ready(c, ci, now);
                    } else if c.parts[ci].pattern >= 0 {
                        linked += 1;
                    } else if c.parts[ci].mv(&w, sel).is_some_and(|m| m.kind >= M_ATTACKS) {
                        linked += 1;
                    }
                }
            }
            if linked > 0 {
                c.parts[0].next = c.parts[0].find_move(&w, M_SYNC, 1, now);
            }
        }
        let cur_kind = c.parts[0].mv(&w, c.parts[0].cur).map_or(0, |m| m.kind);
        if c.parts[0].next < 0 && cur_kind < M_ATTACKS {
            self.look_for_ready(c, 0, players, pos, yaw);
        }
        if c.parts[0].next < 0 {
            Self::taunt_or_ready(c, 0, now);
        }
        self.critter_animate(c, 0, model, dt);
        if c.parts[0].cur < 0 {
            c.parts[0].cur = 0;
        }
        let root_move = c.parts[0]
            .mv(&w, c.parts[0].cur)
            .cloned()
            .unwrap_or_default();
        let legend = self.legend_state();
        for ci in 1..c.parts.len() {
            if c.parts[ci].state == 1 {
                c.parts[ci].next = c.parts[ci].find_move(&w, M_DEATH, 1, now);
                self.critter_animate(c, ci, model, dt);
                // The head's stump replaces it once its death has played.
                let p = &mut c.parts[ci];
                if p.mv(&w, p.cur).is_some_and(|m| m.kind == M_DEATH) && p.anim.finished {
                    p.gone = true;
                }
            } else if (root_move.kind == M_SYNC || c.parts[0].pattern >= 0)
                && !(2..=3).contains(&legend)
                && (c.parts[ci].cur >= 0 || c.parts[ci].next >= 0)
            {
                self.critter_animate(c, ci, model, dt);
            } else {
                // CritterCopyParentAnimation.
                let (s, f, d) = (
                    c.parts[0].anim.seq,
                    c.parts[0].anim.frame,
                    c.parts[0].movedone,
                );
                let ch = &mut c.parts[ci];
                ch.anim.seq = s;
                ch.anim.frame = f;
                ch.movedone = d;
                ch.cur = -1;
                ch.pattern = -1;
                ch.patidx = -1;
                if root_move.kind == M_SYNC {
                    ch.next = 0;
                }
            }
        }
        // Hold times and the death fade.
        let done = c.parts[0].anim.finished;
        if root_move.holdtime > 0.0 {
            if root_move.kind == M_DEATH {
                if !done {
                    c.parts[0].hold = now + root_move.holdtime;
                } else if now >= c.parts[0].hold {
                    self.critter_delete(ei, c);
                    return;
                } else {
                    let remaining = c.parts[0].hold - now;
                    if let Some(b) = self.boss.as_mut() {
                        b.dead = true;
                    }
                    // [CHOICE] The code's MBTreeSetAlpha(255 - 510 x remaining)
                    // with 255 = invisible would fade the boss in; we fade it out.
                    if remaining < 0.5 {
                        c.alpha = (remaining * 2.0).clamp(0.0, 1.0);
                    }
                }
            } else if !done || c.parts[0].hold == 0.0 {
                c.parts[0].hold = now + root_move.holdtime;
            }
        } else {
            c.parts[0].hold = 0.0;
        }
        if let Some(b) = self.boss.as_mut() {
            if root_move.kind == M_START && root_move.nextidx < 0 && done && b.legend_state == 1 {
                b.legend_state = 2;
                b.legend_timer = now;
            }
            if root_move.kind == M_ROAR && done && b.legend_state == 3 {
                b.legend_state = 4;
            }
        }
        // Per part: setup, frame events, translation, rotation.
        for pi in 0..c.parts.len() {
            let Some(m) = c.parts[pi].mv(&w, c.parts[pi].cur).cloned() else {
                continue;
            };
            let frame = c.parts[pi].anim.frame as i32;
            self.move_setup(c, pi, &m, ei);
            self.activate(c, pi, &m, frame, ei);
            if pi == 0 {
                self.translate(ei, c, &m, dt, players);
                self.rotate(ei, c, pi, &m, players, dt);
            }
        }
        // Floor snap (FloorCollide: a radius-5 sphere from 4 above, down).
        let t0 = &w.types[c.parts[0].ty];
        if let Some((g, floor)) = self.critter_floor_hit(self.entities[ei].pos) {
            self.entities[ei].pos.y = g + t0.y_offset;
            // A sleeping typeFlags-0x80 boss wakes when the floor it stands
            // on (or that floor's parent) moves: triggerstate 0x10.
            let asleep = self.boss.as_ref().is_some_and(|b| b.wake_time == 0.0);
            if c.parts[0].state == 0 && asleep && t0.flags & 0x80 != 0 && self.floor_moving(floor)
            {
                if let Some(b) = self.boss.as_mut() {
                    b.wake_time = now + 2.0;
                }
                self.boss_activate(false);
            }
        }
        // Held players ride on the move node.
        self.carry_grabbed(c, ei);
    }

    /// CritterGolemAI: golems, generals and gargoyles.
    fn golem_ai(&mut self, ei: usize, c: &mut Critter, players: &[PInfo], dt: f32) {
        let now = self.time;
        let (pos, yaw) = (self.entities[ei].pos, self.entities[ei].yaw);
        let model = self.entities[ei].model.unwrap_or(0);
        let w = c.wad.clone();
        // CritterGetSingleTargetPlayer: the best target only.
        self.get_targets(c, 0, players, pos, yaw);
        c.parts[0].targets.truncate(1);
        c.parts[0].set_difficulty(&w, self.tuning.health);
        if c.parts[0].state == 0 {
            c.parts[0].state = 3;
        }
        let p = &mut c.parts[0];
        p.next = -1;
        p.dpattern = -1;
        p.dtgt = -1;
        self.get_do_action(c, 0);
        let frozen = self
            .players
            .iter()
            .any(|p| p.fx.special & crate::powerups::S_STOPTIME != 0);
        if !frozen {
            if c.parts[0].next < 0 {
                self.look_for_critical(c, 0, players, yaw);
            }
            if c.parts[0].next < 0 {
                self.child_critical(c, 0, yaw);
            }
            if c.parts[0].next < 0 {
                self.look_for_ready(c, 0, players, pos, yaw);
            }
            if c.parts[0].next < 0 {
                Self::taunt_or_ready(c, 0, now);
            }
        }
        if c.parts[0].next < 0 {
            c.parts[0].next = c.parts[0].cur;
        }
        let cur_kind = c.parts[0].mv(&w, c.parts[0].cur).map_or(-1, |m| m.kind);
        let next_kind = c.parts[0].mv(&w, c.parts[0].next).map_or(-1, |m| m.kind);
        if !frozen
            || c.parts[0].cur < 0
            || [cur_kind, next_kind]
                .iter()
                .any(|&k| k == M_DEATH || k == M_START)
        {
            self.critter_animate(c, 0, model, dt);
        }
        if c.parts[0].cur < 0 {
            c.parts[0].cur = 0;
        }
        let m = c.parts[0]
            .mv(&w, c.parts[0].cur)
            .cloned()
            .unwrap_or_default();
        let frame = c.parts[0].anim.frame as i32;
        if m.kind == M_DEATH {
            if c.parts[0].anim.finished {
                self.critter_drop_item(c, ei);
                self.critter_delete(ei, c);
                return;
            }
            let frames = self.models[model]
                .skeleton
                .sequences
                .get(c.parts[0].anim.seq)
                .map_or(1, |s| s.frame_count as i32);
            if m.actionframe > 0 {
                let el = frame - m.actionframe;
                let total = frames - m.actionframe;
                if el > 0 && total > 0 {
                    c.alpha = 1.0 - el as f32 / total as f32;
                }
            }
        }
        if m.kind == M_DEATH || !frozen {
            self.move_setup(c, 0, &m, ei);
            self.activate(c, 0, &m, frame, ei);
            if !self.translate(ei, c, &m, dt, players) {
                self.rotate(ei, c, 0, &m, players, dt);
            }
        }
        self.carry_grabbed(c, ei);
    }

    /// CritterMoveSetup: the target and the move node.
    fn move_setup(&mut self, c: &mut Critter, pi: usize, m: &Move, ei: usize) {
        let (pos, yaw) = (self.entities[ei].pos, self.entities[ei].yaw);
        let movedone = c.parts[pi].movedone;
        if c.parts[pi].tgt < 0 || movedone {
            let t = if c.parts[pi].blind > 0.0 {
                -1
            } else if c.parts[pi].dtgt >= 0 {
                c.parts[pi].dtgt
            } else {
                self.target_sub(c, pi, Some(&m.vis), 1, yaw)
            };
            c.parts[pi].tgt = t;
        }
        let cur = c.parts[pi].cur.max(0) as usize;
        let node = c.parts[pi].move_nodes.get(cur).copied().flatten();
        let node = node.filter(|&n| !c.removed[n]);
        let mat = c.node_world(node, pos, yaw);
        let p = &mut c.parts[pi];
        if movedone {
            p.events = 0;
            p.sfx_events = 0;
        }
        p.prev_pos = p.node_pos;
        p.node_mat = mat;
        p.node_pos = mat.w_axis.truncate();
        if movedone {
            p.prev_pos = p.node_pos;
        }
    }

    /// CritterActivate: frame-window events fire their damage, and the move's
    /// sounds play.
    fn activate(&mut self, c: &mut Critter, pi: usize, m: &Move, frame: i32, ei: usize) {
        let w = c.wad.clone();
        if m.actionframe >= 0 {
            let ev = Self::frame_events(&c.parts[pi], m, frame);
            let old = c.parts[pi].events;
            if ev & 1 != 0 {
                if m.kind != M_SPRAY {
                    c.parts[pi].events |= 1;
                }
                if m.kind == M_TARGETED {
                    if let Some(t) = usize::try_from(c.parts[pi].tgt)
                        .ok()
                        .and_then(|t| self.players.get(t))
                    {
                        c.parts[pi].tgtpos = t.pos;
                    }
                }
                if m.damageidx >= 0 {
                    self.do_damage(c, pi, m.damageidx as usize, 1, old & 1 == 0, ei);
                }
            }
            if ev & 2 != 0 {
                if m.kind != M_SPRAY {
                    c.parts[pi].events |= 2;
                }
                if m.damageidx2 >= 0 {
                    self.do_damage(c, pi, m.damageidx2 as usize, 2, old & 2 == 0, ei);
                }
            }
        }
        if m.damageidx >= 0 {
            let d = &w.damage[m.damageidx as usize];
            if d.flags & 0x4000 != 0 && c.parts[pi].fball_cap > 0.0 && d.kind != 1 {
                return;
            }
        }
        for (bit, fx, at) in [(1u8, m.movefx, m.fxframe), (2, m.movefx2, m.fxframe2)] {
            if c.parts[pi].sfx_events & bit == 0 && fx >= 0 && frame >= at as i32 && at < 1000 {
                c.parts[pi].sfx_events |= bit;
                let p = c.parts[pi].node_pos;
                self.critter_sfx(c, fx as usize, p);
            }
        }
    }

    /// The CritterCopyAnim classifier: which events fire on this frame.
    fn frame_events(p: &Part, m: &Move, frame: i32) -> u8 {
        let mut r = 0;
        let (af, af2) = (m.actionframe, m.actionframe2);
        let (ef, ef2) = (m.endframe as i32, m.endframe2 as i32);
        match m.kind {
            M_CLAW | M_BREATH | M_CHARGE => {
                if frame >= af && frame <= ef {
                    r |= 1;
                }
                if af2 >= 0 && frame >= af2 && frame <= ef2 {
                    r |= 2;
                }
            }
            M_GRAB => {
                if frame >= af && frame <= ef {
                    r |= 1;
                }
                if af2 >= 0 && p.events & 2 == 0 && frame >= af2 {
                    r |= 2;
                }
            }
            M_SPRAY => {
                let period = m.delay;
                let tick = |d: i32| period <= 0.0 || (d as f32).rem_euclid(period).abs() < 1e-3;
                if frame >= af && frame <= ef && tick(frame - af) {
                    r |= 1;
                }
                if af2 >= 0 && frame >= af2 && frame <= ef2 && tick(frame - af2) {
                    r |= 2;
                }
            }
            M_TARGETED => {
                if p.events & 1 == 0 && frame >= af && p.tgt >= 0 {
                    r |= 1;
                }
                if af2 >= 0 && p.events & 1 != 0 && p.events & 2 == 0 && frame >= af2 {
                    r |= 2;
                }
            }
            _ => {
                if p.events & 1 == 0 && frame >= af {
                    r |= 1;
                }
                if af2 >= 0 && p.events & 2 == 0 && frame >= af2 {
                    r |= 2;
                }
            }
        }
        r
    }

    /// Play SFXX `i` (its sound, and its effect model once) at `at`.
    fn critter_sfx(&mut self, c: &Critter, i: usize, at: Vec3) {
        let Some(s) = c.wad.sfx.get(i) else { return };
        // CritterDoSfx: flag 2 shakes the camera, 0x20 runs SafeRockSetup.
        if s.flags & 2 != 0 {
            self.shake.start(0, 0, 90, 0.1, 100);
        }
        if s.flags & 0x20 != 0 {
            self.safe_rock_setup();
        }
        if !s.snddesc.is_empty() {
            let letter = self.world_letter().to_string();
            let name = s.snddesc.replace("%c", &letter).replace("%s", &letter);
            self.sfx.push(name);
        }
        if let Some(Some(m)) = c.fx_models.get(i) {
            let off = Vec3::from(s.offset);
            self.spawn_effect(*m, at + off, 0.0);
        }
    }

    /// CritterDoDamage: a move event's damage by its kind.
    fn do_damage(
        &mut self,
        c: &mut Critter,
        pi: usize,
        idx: usize,
        phase: u8,
        active: bool,
        ei: usize,
    ) {
        let w = c.wad.clone();
        let Some(d) = w.damage.get(idx) else { return };
        // Legend weakening: flagged attacks are off (the rune quest stage).
        if d.flags & 0x4000 != 0 && c.parts[pi].fball_cap > 0.0 && d.kind != 1 {
            return;
        }
        let (pos, yaw) = (self.entities[ei].pos, self.entities[ei].yaw);
        let body = Mat4::from_translation(pos) * Mat4::from_rotation_y(yaw);
        let amount = d.amount * self.tuning.damage;
        // Effect position: kinds 2/3 in body space, the rest at the move node.
        let at = match d.kind {
            2 | 3 => body.transform_point3(Vec3::from(d.offset)),
            8 => {
                c.parts[pi].tgtpos
                    + Mat4::from_rotation_y(yaw).transform_vector3(Vec3::from(d.offset))
            }
            _ => {
                c.parts[pi].node_pos
                    + Mat4::from_rotation_y(yaw).transform_vector3(Vec3::from(d.offset))
            }
        };
        // (SPOUT effects go to the safe rocks instead of the move node.)
        if active && d.fx >= 0 && !matches!(d.kind, 1 | 5 | 6) {
            self.critter_sfx(c, d.fx as usize, at);
        }
        match d.kind {
            0 => self.node_collide(c, pi, d, amount),
            4 => self.cone_collide(c, pi, d, amount, yaw),
            1 if active => self.critter_fireball(c, pi, d, amount, yaw),
            7 if phase == 1 => self.critter_grab(c, pi, d),
            7 if phase == 2 => self.critter_release(c, pi, d, amount, yaw),
            9 if active => {
                let v = pitch(turn(yaw_dir(c.init_yaw), d.angle), d.pitch) * d.speed_min;
                let spread = d.arc.clamp(-1.0, 1.0).acos();
                let at = c.parts[pi].node_pos;
                self.boss_spew_coins(at, v, spread);
            }
            // SPOUT: an eruption at every safe rock. SPOUT2: at the hidden rock
            // nearest the target, which rises when the eruption is over.
            5 if active => {
                for at in self.safe_rock_positions() {
                    if d.fx >= 0 {
                        self.critter_sfx(c, d.fx as usize, at);
                    }
                    let w = self.critter_wave(c, d, at, amount, yaw);
                    self.crit_waves.push(w);
                }
            }
            6 if active => {
                let fx_secs = usize::try_from(d.fx)
                    .ok()
                    .and_then(|i| c.fx_models.get(i).copied().flatten())
                    .and_then(|m| self.models[m].skeleton.sequences.first())
                    .map_or(1.0, |s| (s.frame_count as f32 - 1.0).max(0.0) / 30.0);
                if let Some(at) = self.safe_rock_spout2(c.parts[pi].tgt, fx_secs) {
                    if d.fx >= 0 {
                        self.critter_sfx(c, d.fx as usize, at);
                    }
                    let w = self.critter_wave(c, d, at, amount, yaw);
                    self.crit_waves.push(w);
                }
            }
            2 | 3 | 8 if active => {
                let w = self.critter_wave(c, d, at, amount, yaw);
                self.crit_waves.push(w);
            }
            _ => {}
        }
    }

    /// CritterNodePlayerCollide: a melee node hits players it touches.
    fn node_collide(&mut self, c: &mut Critter, pi: usize, d: &Damage, amount: f32) {
        let p = &c.parts[pi];
        let point = p.node_mat.transform_point3(Vec3::from(d.offset));
        let prev = p.prev_pos;
        let r = d.radius;
        let shrunk = !c.is_boss()
            && self
                .players
                .iter()
                .any(|q| q.fx.special & crate::powerups::S_SHRINK != 0);
        let mut hits = Vec::new();
        for (i, pl) in self.players.iter().enumerate().take(4) {
            if !pl.state.active() || self.time < self.crit_hit_until[i] {
                continue;
            }
            let dxz = Vec2::new(pl.pos.x - point.x, pl.pos.z - point.z).length();
            if dxz < 1.5 + r && point.y > pl.pos.y - r && point.y < pl.pos.y + 5.0 + r {
                let swing = (point - prev).normalize_or_zero();
                let away =
                    Vec3::new(pl.pos.x - point.x, 1.0, pl.pos.z - point.z).normalize_or_zero();
                hits.push((i, 2.0 * (swing + away)));
            }
        }
        for (i, _dir) in hits {
            self.crit_hit_until[i] = self.time + PLAYER_REHIT;
            let dmg = if shrunk { amount * 0.5 } else { amount };
            self.critter_hits_player(c, pi, i, dmg, d.dmgtype);
            if d.hitfx >= 0 {
                let at = self.players[i].pos + Vec3::Y * 2.5;
                self.critter_sfx(c, d.hitfx as usize, at);
            }
        }
    }

    /// CritterFirePlayerCollide: a breath or beam cone along the move node.
    fn cone_collide(&mut self, c: &mut Critter, pi: usize, d: &Damage, amount: f32, yaw: f32) {
        let p = &c.parts[pi];
        let fwd = p.node_mat.z_axis.truncate().normalize_or_zero();
        // A node whose forward points nowhere useful: use the body's.
        let fwd = if fwd.length() < 0.5 {
            yaw_dir(yaw)
        } else {
            fwd
        };
        let dir = pitch(turn(fwd, d.angle), d.pitch).normalize_or_zero();
        let start = p.node_mat.transform_point3(Vec3::from(d.offset));
        let end = start + dir * d.radius;
        let mut hits = Vec::new();
        for (i, pl) in self.players.iter().enumerate().take(4) {
            if !pl.state.active() || self.time < self.crit_hit_until[i] {
                continue;
            }
            let h = Vec2::new(pl.pos.x - start.x, pl.pos.z - start.z).length();
            if h < d.minrad || h > d.radius + 1.5 {
                continue;
            }
            // Capsule test against the player's body segment.
            let a = pl.pos + Vec3::Y * 0.5;
            let b = pl.pos + Vec3::Y * 4.5;
            if seg_seg_dist(start, end, a, b) < d.hitrad + 1.5 {
                if self
                    .collision
                    .raycast(start, pl.pos + Vec3::Y * 2.5)
                    .is_some()
                {
                    continue;
                }
                hits.push(i);
            }
        }
        for i in hits {
            self.crit_hit_until[i] = self.time + PLAYER_REHIT;
            self.critter_hits_player(c, pi, i, amount, d.dmgtype);
        }
    }

    /// A FIREBALL: an aimed (or ballistic) projectile carrying the damage.
    fn critter_fireball(&mut self, c: &mut Critter, pi: usize, d: &Damage, amount: f32, yaw: f32) {
        let at = c.parts[pi].node_pos;
        self.critter_fireball_at(c, pi, d, amount, yaw, at);
    }

    /// A FIREBALL from `from` (the move node, or a breaking colnode).
    fn critter_fireball_at(
        &mut self,
        c: &mut Critter,
        pi: usize,
        d: &Damage,
        amount: f32,
        yaw: f32,
        from: Vec3,
    ) {
        let p = &c.parts[pi];
        let rot = Mat4::from_rotation_y(yaw);
        let origin = from + rot.transform_vector3(Vec3::from(d.offset));
        let speed = if d.speed_min > 0.0 {
            let k = p.difficulty.clamp(0.5, 1.5);
            0.75 * (k - 0.5) * (d.speed_max - d.speed_min) + d.speed_min
        } else {
            30.0
        };
        let fwd = rot.transform_vector3(Vec3::Z);
        let target = usize::try_from(p.tgt)
            .ok()
            .and_then(|t| self.players.get(t))
            .filter(|t| t.state.active())
            .map(|t| t.pos + Vec3::Y * 2.5);
        let mut v = if d.flags & 4 != 0 {
            fwd
        } else if d.flags & 1 != 0 && target.is_some() {
            target.unwrap() - origin
        } else {
            Vec3::new(fwd.x, -0.5, fwd.z)
        };
        let gravity = d.weight.max(0.0);
        let mut vel = if d.flags & 8 == 0 {
            // CalcTargetDir: the launch that reaches the point at this speed.
            let flat = Vec3::new(v.x, 0.0, v.z);
            let t = (flat.length() / speed).max(0.05);
            flat.normalize_or_zero() * speed + Vec3::Y * (v.y / t + 0.5 * gravity * t)
        } else {
            v = v.normalize_or_zero();
            v * speed
        };
        let mut a = d.angle;
        if d.dyaw > 0.0 {
            a += -0.5 * d.dyaw + self.rng.f32() * d.dyaw;
        }
        if a != 0.0 {
            vel = turn(vel, a);
        }
        if d.flags & 8 != 0 && d.pitch != 0.0 {
            vel = pitch(vel.normalize_or_zero(), d.pitch) * speed;
        }
        let life = match c.parts[pi].fball_cap {
            cap if cap > 0.0 && d.flags & 0x4000 != 0 => cap,
            _ => self.critter_fx_time(c, d.fx),
        };
        let model = usize::try_from(d.fx)
            .ok()
            .and_then(|i| c.fx_models.get(i).copied().flatten());
        let hit_model = usize::try_from(d.hitfx)
            .ok()
            .and_then(|i| c.fx_models.get(i).copied().flatten());
        if let Some(s) = usize::try_from(d.fx).ok().and_then(|i| c.wad.sfx.get(i)) {
            if !s.snddesc.is_empty() {
                self.sfx
                    .push(s.snddesc.replace("%c", &self.world_letter().to_string()));
            }
        }
        // Garm's GAR2GEN shots bring two GARM2 where they land
        // (BossGenerateEnemy; marked with a bit C_DMG doesn't use).
        let adds = usize::try_from(d.fx)
            .ok()
            .and_then(|i| c.wad.sfx.get(i))
            .is_some_and(|s| s.fxdesc.to_ascii_uppercase().starts_with("GAR2GEN"));
        self.spawn(
            Kind::EnemyShot {
                vel,
                damage: amount,
                life,
                splash: d.radius.max(0.0),
                gravity,
                flags: d.dmgtype | if adds { SHOT_ADDS } else { 0 },
                hit_model,
            },
            model,
            origin,
            vel.x.atan2(vel.z),
        );
    }

    /// CritterGrabPlayer: the nearest player touching the node is held.
    fn critter_grab(&mut self, c: &mut Critter, pi: usize, d: &Damage) {
        if c.parts[pi].grab >= 0 {
            return;
        }
        let point = c.parts[pi].node_mat.transform_point3(Vec3::from(d.offset));
        let who = self
            .players
            .iter()
            .enumerate()
            .take(4)
            .filter(|(_, p)| p.state.active())
            .map(|(i, p)| (i, Vec2::new(p.pos.x - point.x, p.pos.z - point.z).length()))
            .filter(|(_, dist)| *dist < 1.5 + d.radius)
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((i, _)) = who {
            c.parts[pi].grab = i as i32;
            self.sfx.push("S_GRAB".into());
        }
    }

    /// CritterReleasePlayer: the held player is thrown along the critter's
    /// facing for the damage.
    fn critter_release(&mut self, c: &mut Critter, pi: usize, d: &Damage, amount: f32, yaw: f32) {
        let g = c.parts[pi].grab;
        c.parts[pi].grab = -1;
        if g >= 0 {
            let fwd = Vec3::new(yaw.sin(), 0.0, yaw.cos());
            self.damage_from = self.players.get(g as usize).map(|p| p.pos - fwd);
            self.critter_hits_player(
                c,
                pi,
                g as usize,
                amount,
                d.dmgtype | DMG_THROWN | DMG_BLOWNAWAY | DMG_KNOCKBACK,
            );
            self.damage_from = None;
        }
    }

    fn carry_grabbed(&mut self, c: &Critter, _ei: usize) {
        for p in &c.parts {
            if let Some(pl) = usize::try_from(p.grab)
                .ok()
                .and_then(|g| self.players.get_mut(g))
            {
                if pl.state.active() {
                    let at = p.node_pos;
                    pl.pos = Vec3::new(at.x, pl.pos.y.max(at.y - 4.0), at.z);
                    // Held: no falling meanwhile.
                    pl.vy = 0.0;
                }
            }
        }
    }

    /// The damage a critter attack does to a player (damage_player), and what
    /// the critter remembers of it (playerDamage.received).
    fn critter_hits_player(
        &mut self,
        c: &mut Critter,
        pi: usize,
        player: usize,
        dmg: f32,
        flags: u32,
    ) {
        let before = self.players[player].health;
        // Our damage_player reads 0x1000 as "no armour" (Death's drain); the
        // critter bit there is DEATHSTUN, so keep it out.
        self.damage_player(player, dmg, flags & !0x1000);
        let taken = (before - self.players[player].health).max(0.0);
        let p = &mut c.parts[pi];
        p.pdamage[player].received += taken;
        p.pdamage[player].received_time = self.time;
    }

    /// CritterTranslate: step moves, knockback, and the leash (bosses) or the
    /// walls (everything else). True when a player was pushed.
    fn translate(
        &mut self,
        ei: usize,
        c: &mut Critter,
        m: &Move,
        dt: f32,
        players: &[PInfo],
    ) -> bool {
        let w = c.wad.clone();
        let t0 = &w.types[c.parts[0].ty];
        let leash = t0.moverad;
        if leash <= 0.0 {
            return false;
        }
        let e = &self.entities[ei];
        let (pos, yaw) = (e.pos, e.yaw);
        let spd = self.tuning.speed * m.speed * dt;
        let is_boss = c.is_boss();
        let init_yaw = c.init_yaw;
        let movecenter = c.movecenter;
        let p = &mut c.parts[0];
        let mut delta = if m.kind == M_STEPTOWARD {
            let d = p.tgtpos - pos;
            Vec3::new(d.x, 0.0, d.z).normalize_or_zero() * spd
        } else {
            let f = if t0.flags & 0x40 != 0 {
                yaw_dir(init_yaw)
            } else {
                yaw_dir(yaw)
            };
            let (fx, fz) = (f.x, f.z);
            let (dx, dz) = match m.kind {
                M_STEPB => (-spd * fx, -spd * fz),
                M_STEPL => (-spd * fz, spd * fx),
                M_STEPR => (spd * fz, -spd * fx),
                M_STEPFL => (spd * fx - spd * fz, spd * fz + spd * fx),
                _ => (spd * fx, spd * fz),
            };
            Vec3::new(dx, 0.0, dz)
        };
        delta += p.kb_vel * dt;
        p.kb_vel *= 0.8;
        for k in 0..3 {
            if p.kb_vel[k].abs() < 0.01 {
                p.kb_vel[k] = 0.0;
            }
        }
        if p.kb_vel.y > 0.0 {
            p.kb_vel.y = (p.kb_vel.y - 100.0 * dt).max(0.0);
        }
        let state = p.state;
        if is_boss {
            let mut d = pos + delta - movecenter;
            if state != 1 {
                let dist = Vec2::new(d.x, d.z).length();
                c.leash = dist;
                if t0.flags & 0x20 != 0 {
                    d.x = d.x.clamp(-leash, leash);
                    d.z = d.z.clamp(-leash, leash);
                } else if dist > leash {
                    d.x *= leash / dist;
                    d.z *= leash / dist;
                }
            }
            let e = &mut self.entities[ei];
            e.pos = Vec3::new(movecenter.x + d.x, e.pos.y, movecenter.z + d.z);
            false
        } else {
            let mut np = pos + Vec3::new(delta.x, 0.0, delta.z);
            let (moved, wall) =
                self.collision
                    .resolve_walls_hit(np, t0.rad.min(3.0), t0.hht * 1.5, 1.2);
            np = moved;
            // CritterWorldDamage: hurting walls and floors, every frame.
            let floor = self.collision.ground_hit(np, np.y + 2.0);
            for (owner, at) in wall
                .into_iter()
                .chain(floor.map(|(y, o)| (o, Vec3::new(np.x, y, np.z))))
            {
                let Some(flags) = self.hurting_surface(owner) else {
                    continue;
                };
                let (dmg, knock) = match (flags >> 16) & 0xF {
                    1 => (5.0, 0),
                    2 => (5.0, DMG_KNOCKBACK),
                    3..=5 => (15.0, DMG_KNOCKDOWN),
                    _ => (0.0, 0),
                };
                if dmg > 0.0 {
                    let d = pos - at;
                    let dir = Vec3::new(d.x, 0.0, d.z).normalize_or_zero();
                    self.critter_damage(c, 0, dmg, None, knock, dir, None);
                }
            }
            // Floor follow, at most 16 units/s down.
            if let Some((g, _)) = floor {
                np.y = if g < np.y {
                    (np.y - 16.0 * dt).max(g)
                } else {
                    g
                };
            }
            // Players block the critter and are pushed away.
            let mut pushed = false;
            for (i, pl) in players.iter().enumerate() {
                if !pl.active {
                    continue;
                }
                let d = Vec2::new(pl.pos.x - np.x, pl.pos.z - np.z);
                let min = t0.rad + 1.5;
                if d.length() < min && (pl.pos.y - np.y).abs() < t0.hht * 2.0 {
                    let n = d.normalize_or_zero();
                    if let Some(q) = self.players.get_mut(i) {
                        let push = n * (min - d.length()).clamp(0.0, 3.0);
                        q.pos += Vec3::new(push.x, 0.0, push.y);
                    }
                    pushed = true;
                    np.x = pos.x;
                    np.z = pos.z;
                }
            }
            self.entities[ei].pos = np;
            pushed
        }
    }

    /// CritterRotate: turn toward the target within the move's turn rate.
    fn rotate(
        &mut self,
        ei: usize,
        c: &mut Critter,
        pi: usize,
        m: &Move,
        players: &[PInfo],
        dt: f32,
    ) {
        let w = c.wad.clone();
        let t0 = &w.types[c.parts[0].ty];
        let e = &self.entities[ei];
        let (pos, yaw) = (e.pos, e.yaw);
        let p = &c.parts[pi];
        let delta = if m.flags & 0x20 != 0 {
            c.init_yaw - yaw
        } else if p.grab >= 0 {
            0.0
        } else if p.tgt >= 0 && m.turn > 0.0 {
            let tp = players[p.tgt as usize].col - pos;
            if t0.flags & 0x400 != 0 {
                tp.x.atan2(tp.z) - yaw
            } else {
                let a = wrap(tp.x.atan2(tp.z) - c.init_yaw).clamp(-t0.maxyaw, t0.maxyaw);
                c.init_yaw + a - yaw
            }
        } else {
            0.0
        };
        if delta != 0.0 {
            let mut step = m.turn * dt;
            if p.blind > 0.0 {
                step *= 0.1;
            }
            let d = wrap(delta).clamp(-step, step);
            self.entities[ei].yaw = wrap(yaw + d);
        }
    }

    /// CritterKill: the part starts dying.
    fn critter_kill(&mut self, c: &mut Critter, pi: usize) {
        if c.parts[pi].state == 1 {
            return;
        }
        c.parts[pi].state = 1;
        let exp = 0.2 * c.wad.types[c.parts[pi].ty].base_exp;
        for i in 0..self.players.len().min(4) {
            if self.players[i].state.active() {
                self.add_exp_pub(i, exp);
            }
        }
        if pi == 0 {
            for p in &mut c.parts[1..] {
                p.health = 1.0;
            }
            if c.is_boss() && !c.dying_sent {
                c.dying_sent = true;
                self.boss_dying();
            }
        }
    }

    /// CritterDelInst: the critter is gone; a boss's death ends the fight.
    fn critter_delete(&mut self, _ei: usize, c: &mut Critter) {
        c.deleted = true;
        if c.is_boss() {
            let key = c.init_pos + Vec3::from(c.wad.types[c.parts[0].ty].keydest);
            self.boss_death(key);
        }
    }

    /// CritterDropItem: what the critter carries (gotitem), else a
    /// gargoyle's gold GARG item, comes out in a bag (StartBagFX); generals
    /// and gargoyles announce it when there is one.
    fn critter_drop_item(&mut self, c: &Critter, ei: usize) {
        let pos = self.entities[ei].pos;
        let item = match self.take_carried(self.entities[ei].id) {
            Some(item) => item,
            None if c.etype == ET_GARGOYLE => {
                let kind = c.garg_kind.clone().unwrap_or_else(|| "eagl".into());
                self.garg_carry(&kind)
            }
            None => return,
        };
        match c.etype {
            ET_GARGOYLE => {
                self.post(0x8A, None, &[]);
            }
            ET_GENERAL => {
                self.post(0x86, None, &[]);
            }
            _ => {}
        }
        self.start_bag(pos, item);
    }

    /// CritterDamage: a player's hit on part `pi` (source 1 melee, 2 missile).
    /// Returns (damage done, killed).
    pub(crate) fn critter_damage(
        &mut self,
        c: &mut Critter,
        pi: usize,
        damage: f32,
        player: Option<usize>,
        flags: u32,
        dir: Vec3,
        node: Option<usize>,
    ) -> (f32, bool) {
        let w = c.wad.clone();
        let now = self.time;
        if c.parts[pi].state < 2 {
            return (0.0, false);
        }
        let t = &w.types[c.parts[pi].ty];
        let mut damage = damage;
        let mut flags = flags;
        if c.parts[pi]
            .mv(&w, c.parts[pi].cur)
            .is_some_and(|m| m.kind == M_BLOCK)
        {
            damage *= 0.25;
            flags &= !(DMG_KNOCKBACK | DMG_KNOCKDOWN | DMG_KNOCKOVER);
            self.sfx.push("S_WEAPONHITMETA".into());
        }
        let boss_level = self.boss.is_some();
        (damage, flags) = modify_damage(damage, flags, t.resist, t.armor, boss_level);
        let shrunk = self
            .players
            .iter()
            .any(|p| p.fx.special & crate::powerups::S_SHRINK != 0);
        if !c.is_boss() && shrunk {
            damage *= 2.0;
        }
        c.parts[pi].dmgtaken += damage;
        let legend = self.legend_state();
        let np = self.num_active_players();
        if c.is_boss() && !(1..=4).contains(&legend) {
            damage *= BOSS_PLAYER_SCALE[np];
        }
        if let Some(pl) = player.filter(|&p| p < 4) {
            let max = t.health * self.tuning.health;
            let credited = damage.clamp(0.0, c.parts[pi].health.max(0.0));
            let ratio = (credited / (1.0 + max)).min(1.0);
            let mut exp = (ratio * t.base_exp) as i32;
            if c.is_boss() {
                exp *= np as i32;
            }
            if exp > 0 {
                self.add_exp_pub(pl, exp as f32);
            }
            let p = &mut c.parts[pi];
            p.pdamage[pl].dealt += credited;
            p.pdamage[pl].dealt_time = now;
            let plevel = self
                .wave
                .and_then(|wv| self.worlds.level(wv))
                .map_or(0.0, |l| l.player_level);
            if !c.is_boss() && plevel > 0.0 {
                let lv = self.players[pl].level as f32;
                let k = if lv < plevel {
                    1.0 - 0.02 * (plevel - lv)
                } else {
                    1.0
                };
                damage *= k.max(0.1);
            }
        }
        // A colnode takes the hit up to what it can absorb; a DESTROY node
        // breaks at its cap (Garm's armour plates).
        if flags & (DMG_SUPER | DMG_MAGIC | DMG_KNOCKOVER | DMG_KNOCKDOWN) == 0 {
            if let Some(ci) = node.filter(|&ci| ci < c.parts[pi].node_damage.len()) {
                let cn = &w.type_colnodes(c.parts[pi].ty)[ci];
                let (got, max) = c.parts[pi].node_damage[ci];
                if got >= max {
                    damage = 0.0;
                } else {
                    damage *= cn.damagemul;
                    if got + damage > max {
                        damage = max - got;
                        if cn.flags & 2 != 0 {
                            self.break_colnode(c, pi, ci);
                        }
                    }
                    c.parts[pi].node_damage[ci].0 += damage;
                }
            }
        }
        if damage <= 0.0 {
            return (0.0, false);
        }
        {
            let p = &mut c.parts[pi];
            p.dmgtype |= flags;
            p.kb_in += dir;
            p.dmgtime = now;
            p.health -= damage;
            p.flash = 0.15;
        }
        if c.parts[pi].health <= 0.0 {
            self.critter_kill(c, pi);
            return (damage, true);
        }
        if pi > 0 {
            c.parts[0].health -= damage;
            if c.parts[0].health <= 0.0 {
                self.critter_kill(c, 0);
                return (damage, true);
            }
        } else if c.parts.len() > 1 {
            let live: Vec<usize> = (1..c.parts.len())
                .filter(|&k| c.parts[k].state >= 2)
                .collect();
            if !live.is_empty() {
                let share = 0.5 * damage / live.len() as f32;
                for k in live {
                    c.parts[k].health -= share;
                    if c.parts[k].health <= 0.0 {
                        self.critter_kill(c, k);
                    }
                }
            }
        }
        if flags & DMG_NOHITFX == 0 {
            let fx = t.hit1fx;
            if fx >= 0 {
                let at = c.parts[pi].node_pos;
                self.critter_sfx(c, fx as usize, at);
            }
        }
        (damage, false)
    }

    /// A DESTROY colnode reached its cap: its object becomes `<prefix>D<node>`
    /// and the moves that need it are gone (with its children for flag 4).
    fn break_colnode(&mut self, c: &mut Critter, pi: usize, ci: usize) {
        let w = c.wad.clone();
        let cn = &w.type_colnodes(c.parts[pi].ty)[ci];
        // The node's destroy DAMG runs from where the node is (Garm's plates
        // fire a GAR2GEN shot that brings his escorts).
        if cn.destroyidx >= 0 {
            if let Some(d) = w.damage.get(cn.destroyidx as usize) {
                let at = c.parts[pi]
                    .colnode_nodes
                    .get(ci)
                    .copied()
                    .flatten()
                    .map_or(c.parts[pi].node_pos, |n| c.nodes[n].w_axis.truncate());
                if d.kind == 1 {
                    // (The body yaw isn't at hand here; the spawn yaw is
                    // within maxyaw of it.)
                    let yaw = c.init_yaw;
                    let amount = d.amount * self.tuning.damage;
                    self.critter_fireball_at(c, pi, d, amount, yaw, at);
                } else if d.fx >= 0 {
                    self.critter_sfx(c, d.fx as usize, at);
                }
            }
        }
        let Some(node) = c.parts[pi].colnode_nodes.get(ci).copied().flatten() else {
            return;
        };
        c.swapped[node] = true;
        if cn.flags & 4 != 0 {
            // DESTROY_CHILDREN: the nodes below go, and moves on them.
            let Some(model) = c.model else {
                c.removed[node] = true;
                return;
            };
            let m = &self.models[model];
            for i in 0..c.removed.len() {
                if i != node && m.in_subtree(i, node) {
                    c.removed[i] = true;
                }
            }
        }
        self.sfx.push("S_WEAPONHITMETA".into());
    }
}

/// CritterAnimInterrupt: 0 can't, 1 at the end, 2 now.
pub fn interrupt(cur: &Move, next: &Move) -> u8 {
    let (a, b) = (cur.priority, next.priority);
    match cur.interrupt {
        0 => 0,
        20 if (b & !0xFF) > (a & !0xFF) => 2,
        60 if b >= a => 2,
        80 if b > 0 => 2,
        90 => 2,
        20 | 60 | 80 => 1,
        _ if b > a => 2,
        _ => 1,
    }
}

/// Distance between segments ab and cd.
fn seg_seg_dist(a: Vec3, b: Vec3, c: Vec3, d: Vec3) -> f32 {
    let (u, v, w) = (b - a, d - c, a - c);
    let (aa, bb, cc, dd, ee) = (u.dot(u), u.dot(v), v.dot(v), u.dot(w), v.dot(w));
    let den = aa * cc - bb * bb;
    let mut s = if den > 1e-6 {
        ((bb * ee - cc * dd) / den).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let mut t = if cc > 1e-6 {
        ((bb * s + ee) / cc).clamp(0.0, 1.0)
    } else {
        0.0
    };
    if aa > 1e-6 {
        s = ((bb * t - dd) / aa).clamp(0.0, 1.0);
        t = if cc > 1e-6 {
            ((bb * s + ee) / cc).clamp(0.0, 1.0)
        } else {
            0.0
        };
    }
    ((a + u * s) - (c + v * t)).length()
}

/// A critter's damaging effect on the ground (CritterDoTexmodNode with
/// flags 0x20): STOMP, SPOUT and FOUNTAIN expand (mode 1), ARC holds its
/// whole radius (flags 0x30, mode 2), each over its effect's time.
#[derive(Clone, Debug)]
pub struct Wave {
    pub pos: Vec3,
    /// damageradius, damage and damagetype.
    pub max_radius: f32,
    pub damage: f32,
    pub flags: u32,
    /// Mode 2: the whole radius at full damage, again each second.
    pub hold: bool,
    /// endtime - now, and maxtime.
    pub left: f32,
    pub maxtime: f32,
    /// mindp (-1: all round) and the effect's facing.
    pub mindp: f32,
    pub dir: Vec2,
    /// The radius this frame (for tests and debugging).
    pub radius: f32,
}

impl Game {
    /// The critter floor probe (`FloorCollide(5.0, 4.0, -1000.0, pos)`, a
    /// sphere of radius 5 swept down from 4 above `pos`): the highest floor
    /// it touches. A floor edge within 5 units counts, as the sphere rests on
    /// it (the Dragon stands at the lip of its ledge this way).
    pub(crate) fn critter_floor(&self, pos: Vec3) -> Option<f32> {
        self.critter_floor_hit(pos).map(|(g, _)| g)
    }

    /// The floor object `obj`'s triggerstate, or its parent's, has bit 0x10:
    /// a trigger's target that is moving (the item world-object update).
    pub(crate) fn floor_moving(&self, obj: i32) -> bool {
        let objs = &self.level.worlds.world_objects;
        if obj < 0 || obj as usize >= objs.len() {
            return false;
        }
        let obj = obj as usize;
        let is_child = |root: usize| {
            let mut ch = objs[root].child_index;
            while ch >= 0 && (ch as usize) < objs.len() {
                if ch as usize == obj {
                    return true;
                }
                ch = objs[ch as usize].next_index;
            }
            false
        };
        self.movers
            .iter()
            .any(|m| m.moving && (m.root == obj || is_child(m.root)))
    }

    /// [`Game::critter_floor`] with the world object of the floor it rests
    /// on (gFloorCollisionResult.obj, -1 for none).
    pub(crate) fn critter_floor_hit(&self, pos: Vec3) -> Option<(f32, i32)> {
        const R: f32 = 5.0;
        let top = pos.y + 4.0;
        let mut best: Option<(f32, i32)> = None;
        let mut probe = |p: Vec3| {
            if let Some((g, o)) = self.collision.ground_hit(p, top) {
                if best.is_none_or(|b| g > b.0) {
                    best = Some((g, o));
                }
            }
        };
        probe(pos);
        for ring in [0.5 * R, R] {
            for k in 0..8 {
                let a = k as f32 * std::f32::consts::FRAC_PI_4;
                probe(pos + Vec3::new(a.sin(), 0.0, a.cos()) * ring);
            }
        }
        best
    }

    /// The wave of damage `d` at `at`: its time is the effect's.
    fn critter_wave(&self, c: &Critter, d: &Damage, at: Vec3, amount: f32, yaw: f32) -> Wave {
        let maxtime = self.critter_fx_time(c, d.fx);
        let dir = turn(yaw_dir(yaw), d.angle);
        Wave {
            pos: at,
            max_radius: d.radius,
            damage: amount,
            flags: d.dmgtype,
            hold: d.kind == 2,
            left: maxtime,
            maxtime,
            mindp: d.arc,
            dir: Vec2::new(dir.x, dir.z).normalize_or_zero(),
            radius: 0.0,
        }
    }

    /// An SFXX effect's time (StartFXSub's time: its maxlen, else its
    /// tree's animation, nframes x rate / 900).
    pub(crate) fn critter_fx_time(&self, c: &Critter, fx: i16) -> f32 {
        let Some(i) = usize::try_from(fx).ok() else {
            return 1.0;
        };
        if let Some(l) = c.wad.sfx.get(i).map(|s| s.maxlen).filter(|&l| l > 0.0) {
            return l;
        }
        c.fx_models
            .get(i)
            .copied()
            .flatten()
            .and_then(|m| self.models[m].skeleton.sequences.first())
            .map_or(1.0, |s| {
                let n = if s.frame_count == 0 {
                    30.0
                } else {
                    s.frame_count as f32
                };
                let rate = if s.frame_rate > 0 {
                    s.frame_rate as f32
                } else {
                    30.0
                };
                n * rate * 0.001_111_11
            })
    }

    /// ProcessEffects' players part for the ground waves. Mode 1: over the
    /// first two thirds of its time the radius grows from a third of
    /// damageradius to all of it as the damage falls from 1.5x to nothing;
    /// mode 2: all of it at full damage. A hero within it (height within
    /// half theirs plus it, a wall in the way past 10, inside its mindp
    /// arc) is hit, then not until their fxhittime (the rest of the effect
    /// in mode 1, a second at most in mode 2; half a second for gas).
    pub(crate) fn update_waves(&mut self, dt: f32) {
        let mut hits = Vec::new();
        for w in &mut self.crit_waves {
            w.left -= dt;
            let remaining = w.left;
            let (radius, scale, age) = if w.hold {
                (w.max_radius, 1.0, remaining.min(1.0))
            } else {
                let phase = if remaining > w.maxtime {
                    0.0
                } else if w.maxtime <= 0.033_333_3 {
                    1.0
                } else {
                    remaining / w.maxtime
                };
                let (r, k) = if phase > 0.33 {
                    (w.max_radius * (0.33 + (1.0 - phase)), 1.5 * (phase - 0.33))
                } else {
                    (0.0, 0.0)
                };
                (r, k, remaining + 0.066_667)
            };
            w.radius = radius;
            if radius <= 0.0 {
                continue;
            }
            let dmg = w.damage * scale;
            let flags = if dmg < 5.0 {
                (w.flags & !0x170) | DMG_NOHITFX
            } else {
                w.flags
            };
            for (i, p) in self.players.iter().enumerate().take(4) {
                if !p.state.active() || self.time < self.crit_hit_until[i] {
                    continue;
                }
                let at = p.pos + Vec3::Y * (crate::game::PLAYER_HEIGHT * 0.5);
                let d = at - w.pos;
                if d.y.abs() > crate::game::PLAYER_HEIGHT * 0.5 + radius {
                    continue;
                }
                let flat = Vec2::new(d.x, d.z);
                let dist = flat.length();
                let sumr = radius + crate::game::PLAYER_RADIUS;
                if dist > sumr {
                    continue;
                }
                if dist > 10.0 && self.collision.raycast(w.pos + Vec3::Y * 0.1, at).is_some() {
                    continue;
                }
                if w.mindp > -1.0 {
                    let mut mindp = w.mindp;
                    if dist < 0.3 * sumr {
                        mindp *= 0.85;
                    }
                    if flat.normalize_or_zero().dot(w.dir) < mindp {
                        continue;
                    }
                }
                let until = if w.flags & DMG_POISONGAS != 0 {
                    Some(0.5)
                } else {
                    (dmg > 2.0).then_some(age)
                };
                hits.push((i, dmg, flags, until));
            }
        }
        self.crit_waves.retain(|w| w.left > 0.033_32);
        for (i, d, f, until) in hits {
            if let Some(t) = until {
                self.crit_hit_until[i] = self.time + t;
            }
            self.damage_player(i, d, f & !0x1000);
        }
    }

    /// Load the effect models of a critter's SFXX records from its monster
    /// directory (effect atrees such as FBALL_LOOP, STOMPFX, CLAWSWIPEL).
    pub(crate) fn critter_fx_models(
        &mut self,
        wad: &CritterWad,
        objects: &gdl_formats::Objects,
        textures: &[u8],
        anim: Option<&gdl_formats::anim::Anim>,
    ) -> Vec<Option<usize>> {
        let mut known: std::collections::HashMap<String, Option<usize>> = Default::default();
        wad.sfx
            .iter()
            .map(|s| {
                let name = s.fxdesc.to_ascii_uppercase();
                if name.is_empty() || name == "NULLFX" || s.flags & 0x0F00_0100 != 0 {
                    return None;
                }
                if let Some(m) = known.get(&name) {
                    return *m;
                }
                let m = anim
                    .filter(|a| a.atrees.iter().any(|t| t.name.eq_ignore_ascii_case(&name)))
                    .and_then(|a| Model::from_anim(objects, textures, a, Some(&name)).ok())
                    .filter(|m| !m.mesh.indices.is_empty() || !m.psys.is_empty())
                    .or_else(|| Model::from_objects_anim(objects, textures, anim, &[&name]));
                let idx = m.map(|m| self.add_model_pub(m));
                known.insert(name, idx);
                idx
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modify_damage_rules() {
        // Armour comes off; an unresisted element is x1.25 on boss levels.
        assert_eq!(modify_damage(10.0, 0, 0, 3.0, true).0, 7.0);
        assert_eq!(modify_damage(10.0, 1, 0, 2.0, true).0, 10.0);
        // Fire resistant: x0.75; acid immune: 0; magic immune.
        assert_eq!(modify_damage(10.0, 1, 0x1, 2.0, true).0, 6.0);
        assert_eq!(modify_damage(10.0, 4, 0x800, 0.0, true).0, 0.0);
        assert_eq!(modify_damage(10.0, DMG_MAGIC, 0x1000, 0.0, true).0, 0.0);
        // Magic skips armour; resist-magic halves it off boss levels.
        assert_eq!(modify_damage(10.0, DMG_MAGIC, 0x10, 3.0, false).0, 5.0);
    }

    #[test]
    fn interrupt_policies() {
        let m = |interrupt, priority| Move {
            interrupt,
            priority,
            ..Default::default()
        };
        assert_eq!(interrupt(&m(0, 1), &m(0, 4095)), 0);
        assert_eq!(interrupt(&m(40, 512), &m(0, 544)), 2);
        assert_eq!(interrupt(&m(40, 544), &m(0, 512)), 1);
        assert_eq!(interrupt(&m(20, 544), &m(0, 576)), 1);
        assert_eq!(interrupt(&m(20, 544), &m(0, 3328)), 2);
        assert_eq!(interrupt(&m(90, 4095), &m(0, 1)), 2);
    }

    #[test]
    fn cone_directions_turn_right_for_positive_angles() {
        // Facing +z, a positive yaw offset turns towards +x.
        let v = turn(Vec3::Z, 0.5);
        assert!(v.x > 0.0 && v.z > 0.0);
        let d = pitch(Vec3::Z, 0.3);
        assert!(d.y < 0.0);
        let (score, ..) = calc_target(
            1.0,
            Some(&Vis {
                min: 0.0,
                max: 50.0,
                ang: 0.0,
                arc: 0.5,
                ..Default::default()
            }),
            Vec3::ZERO,
            Vec3::Z,
            Vec3::new(0.0, 0.0, 10.0),
        );
        assert!((score - 10.0).abs() < 1e-4);
    }

    fn data() -> Option<crate::fs::GameFs> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game-data");
        crate::fs::GameFs::open(&root).ok()
    }

    /// A placed golem, general or gargoyle: it attacks a player in front of it
    /// for its DAMG damage, takes hits through its armour, and dies; the
    /// gargoyle leaves its GARG item.
    #[test]
    fn placed_critters_fight_and_die() {
        let Some(fs) = data() else { return };
        let mut seen = std::collections::BTreeSet::new();
        for lvl in [
            "levelA1", "levelB2", "levelC1", "levelD1", "levelF1", "levelI1",
        ] {
            let mut g = Game::new(&fs, lvl, &[("WAR", "RED")]).unwrap();
            let crits: Vec<(u64, i16)> = g
                .entities
                .iter()
                .filter_map(|e| match &e.kind {
                    Kind::Critter(c) => Some((e.id, c.etype)),
                    _ => None,
                })
                .collect();
            for (id, etype) in crits {
                if seen.contains(&(etype, lvl.as_bytes()[5])) {
                    continue;
                }
                let i = g.entities.iter().position(|e| e.id == id).unwrap();
                let (pos, yaw) = (g.entities[i].pos, g.entities[i].yaw);
                // Stand in front of it.
                g.players[0].pos = pos + yaw_dir(yaw) * 5.0;
                g.cam_focus = g.players[0].pos;
                let mut hurt = 0.0;
                let mut moves = std::collections::BTreeSet::new();
                g.players[0].state = crate::game::PlayerState::Normal;
                for _ in 0..600 {
                    g.players[0].health = 500.0;
                    if let Some(e) = g.entities.iter().find(|e| e.id == id) {
                        g.players[0].pos = e.pos + yaw_dir(e.yaw) * 5.0;
                    }
                    g.update(1.0 / 60.0, &[crate::input::PlayerInput::default()]);
                    hurt += 500.0 - g.players[0].health;
                    if let Some(Kind::Critter(c)) =
                        g.entities.iter().find(|e| e.id == id).map(|e| &e.kind)
                    {
                        let p = &c.parts[0];
                        if let Some(m) = p.moves(&c.wad).get(p.cur.max(0) as usize) {
                            moves.insert(m.name.clone());
                        }
                    }
                }
                eprintln!("{lvl} etype {etype}: took {hurt:.0}, moves {moves:?}");
                assert!(hurt > 0.0, "{lvl} etype {etype} never hurt the player");
                // Beat it to death.
                let hp = |g: &Game| match g.entities.iter().find(|e| e.id == id).map(|e| &e.kind) {
                    Some(Kind::Critter(c)) => c.parts[0].health,
                    _ => -1.0,
                };
                let before = hp(&g);
                // At or above the level's plevel a player deals full damage.
                g.players[0].level = 99;
                let i = g.entities.iter().position(|e| e.id == id).unwrap();
                g.hit_entity(0, i, 50.0, Vec2::ZERO, true);
                assert!(hp(&g) < before, "{lvl} etype {etype}: no damage");
                for _ in 0..40 {
                    if let Some(i) = g.entities.iter().position(|e| e.id == id) {
                        g.hit_entity(0, i, 100.0, Vec2::ZERO, true);
                    }
                }
                let garg_before = g
                    .entities
                    .iter()
                    .filter(|e| {
                        matches!(
                            e.kind,
                            Kind::Pickup {
                                kind: crate::game::PickupKind::Gargoyle(_),
                                ..
                            }
                        )
                    })
                    .count();
                for _ in 0..600 {
                    g.players[0].health = 500.0;
                    g.update(1.0 / 60.0, &[crate::input::PlayerInput::default()]);
                }
                assert!(
                    g.entities.iter().all(|e| e.id != id),
                    "{lvl} etype {etype} didn't go"
                );
                if etype == ET_GARGOYLE {
                    let after = g
                        .entities
                        .iter()
                        .filter(|e| {
                            matches!(
                                e.kind,
                                Kind::Pickup {
                                    kind: crate::game::PickupKind::Gargoyle(_),
                                    ..
                                }
                            )
                        })
                        .count();
                    assert_eq!(after, garg_before + 1, "{lvl}: no GARG item");
                }
                seen.insert((etype, lvl.as_bytes()[5]));
            }
        }
        let kinds: std::collections::BTreeSet<i16> = seen.iter().map(|s| s.0).collect();
        assert!(
            kinds.contains(&ET_GOLEM)
                && kinds.contains(&ET_GENERAL)
                && kinds.contains(&ET_GARGOYLE),
            "{seen:?}"
        );
    }
}
