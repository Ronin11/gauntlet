//! What the power-ups put on a hero (player.c PlayerProcessPowerups,
//! PlayerProcessSkinFX, PlayerProcessMikeyPUP) and the familiars' shots
//! (pmotion.c:2496-2617): the familiar tree (Pojo, the fire shield's flare,
//! the Phoenix, a breath, the wings), the level-30 / 80 familiar, an amulet's
//! glow in the weapon hand, the held objects (shields, boss items, halo, gas
//! mask, X-ray specs, super crossbow, hammer), the hero's see-through,
//! growth and levitation, the breath and hammer blasts, and the Mikey decoy
//! (with AppendBigapePowerupsToScene, which places the Mikey, Hand of Death
//! and Health Vampire pick-ups).

use crate::math::Det as _;
use super::*;
use crate::powerups as pu;
use glam::Mat4;

/// Where a worn model hangs: p->node (the hero's place, not animated), the
/// body (p->node->child->child, the skeleton's first bone), weapon_node
/// (<prefix>HEAD), hand_node (player_rhand) or mbnode2 (player_lhand).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Anchor {
    #[default]
    Root,
    Body,
    Head,
    RHand,
    LHand,
}

/// player_lhand / player_rhand (player.c:558) by class.
const LHAND: [&str; 16] = [
    "L_WRIST", "L_WRIST", "L_WRIST", "L_WRIST", "LEFTHAND", "LEFTHAND", "LEFTHAND", "LEFTHAND",
    "L_WRIST", "L_WRIST", "L_WRIST", "L_WRIST", "LEFTHAND", "LEFTHAND", "LEFTHAND", "LEFTHAND",
];
const RHAND: [&str; 16] = [
    "R_WRIST", "R_WRIST", "R_WRIST", "R_WRIST", "RIGHTHAN", "RIGHTHAN", "RIGHTHAN", "RHEND",
    "R_WRIST", "R_WRIST", "R_WRIST", "R_WRIST", "RIGHTHAN", "RIGHTHAN", "RIGHTHAN", "RHEND",
];

/// A hero's attachment nodes (load_player), as skeleton node indices.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Anchors {
    body: Option<usize>,
    head: Option<usize>,
    rhand: Option<usize>,
    lhand: Option<usize>,
}

impl Anchors {
    fn find(m: &Model, class_id: usize) -> Anchors {
        let nodes = &m.skeleton.nodes;
        Anchors {
            body: nodes.iter().position(|n| n.parent == 0),
            head: m.node_index("HEAD"),
            rhand: m.node_index(RHAND[class_id.min(15)]),
            lhand: m.node_index(LHAND[class_id.min(15)]),
        }
    }

    fn node(&self, a: Anchor) -> Option<usize> {
        match a {
            Anchor::Root => None,
            Anchor::Body => self.body,
            Anchor::Head => self.head,
            Anchor::RHand => self.rhand,
            Anchor::LHand => self.lhand,
        }
    }
}

/// The global effect rows these use (fx_type, sfx.c's name table).
const FX_EXPRING: usize = 0;
const FX_BREATHE_FIRE: usize = 1;
const FX_BREATHE_ACID: usize = 2;
const FX_BREATHE_ELEC: usize = 3;
const FX_BOSS_BREATHE: usize = 4;
const FX_NAMES: [&str; 5] = [
    "EXPRING",
    "FIREBREATHE",
    "ACIDBREATHE",
    "ELECBREATHE",
    "BOSS_BREATHE",
];

/// The objects PlayerProcessPowerups hangs with MBOX_FindObject (names cut
/// to the object table's 15 characters).
const OBJECTS: [&str; 15] = [
    "BOSSGAUNTL",
    "RF_SHLD",
    "FW_SHLD",
    "L_SHLD",
    "HEAD_HANDOFDEAT",
    "HEAD_HEALTHVAMP",
    "BOSSHORNS",
    "BOSSMASK",
    "HEAD_HALO",
    "HEAD_GAS",
    "HEAD_XRAY",
    "BOSSGAUNTR",
    "SUPERXBOW",
    "HAMMER_HD",
    "IT_SIGN",
];

/// The models, loaded once a level (InitPlayerMissiles' trees and the
/// objects above).
#[derive(Clone, Debug, Default)]
pub(crate) struct WornModels {
    /// PhoenixTree, WingsTree, PojoTree (POWERUPS) and FireShieldTree
    /// (WEAPONS FW_SHLD_ACTIVE).
    phoenix: Option<usize>,
    wings: Option<usize>,
    pojo: Option<usize>,
    fire_shield: Option<usize>,
    /// BreatheFireTree, BreatheAcidTree, BreatheElecTree (HEAD_BREATHEF/A/E).
    breath: [Option<usize>; 3],
    /// FamiliarSpit[4]: PHOENIX_FBALL, the Phoenix's and Pojo's shot.
    pub(crate) fball: Option<usize>,
    /// BallistaTree, BossElecTree, BossAcidTree: the super shot's arrow and the
    /// boss gauntlets' bolts.
    pub(crate) superarrow: Option<usize>,
    pub(crate) boss_elec: Option<usize>,
    pub(crate) boss_acid: Option<usize>,
    /// MIKEYPUP_ON: the Mikey let out.
    mikey: Option<usize>,
    /// The effect rows (FX_NAMES).
    fx: [Option<usize>; 5],
    objects: [Option<usize>; 15],
    /// Pojo's BODY1_HE#1 (lbl_80114274): where its fire comes from.
    pojo_head: Option<usize>,
    /// Per hero: its anchors, FamiliarTree[0..2] (FAMILIAR1/2), FamiliarSpit
    /// and WeapHoldFxTree (WEAP_HOLD_<colour> by element) from its SFX bank.
    anchors: Vec<Anchors>,
    pets: Vec<[Option<usize>; 2]>,
    spit: Vec<Option<usize>>,
    holds: Vec<[Option<usize>; 5]>,
}

/// DmgTypeDesc (0x801190E8): the element colours.
const DMG_TYPE_DESC: [&str; 5] = ["", "RED", "BLU", "YEL", "GRE"];

/// A worn model and its animation.
#[derive(Clone, Debug)]
pub struct WornModel {
    pub model: usize,
    pub anchor: Anchor,
    /// Its place and scale in the anchor's frame.
    pub local: Mat4,
    pub anim: Animator,
    /// MBTreeSetAlpha's (1 opaque).
    pub alpha: f32,
    /// MBTreeSetFlags 2: not drawn.
    pub hidden: bool,
}

impl WornModel {
    fn new(model: usize, anchor: Anchor, local: Mat4) -> WornModel {
        WornModel {
            model,
            anchor,
            local,
            anim: Animator::new(0),
            alpha: 1.0,
            hidden: false,
        }
    }
}

/// PlayerMikeyState (Player+0x96C..0xA1E): the decoy.
#[derive(Clone, Debug, Default)]
pub struct Mikey {
    /// field_A1C: 0 none, 1 waiting to be let out, 2.. out (counting frames),
    /// 300 going back.
    pub state: i16,
    /// Where it was let out (the hero's feet then; drawn at its effectpos).
    pub pos: Vec3,
    pub entity: Option<u64>,
}

impl Mikey {
    /// field_A1C > 2: monsters go after it.
    pub fn out(&self) -> bool {
        self.state > 2
    }
}

/// Everything PlayerProcessPowerups keeps on a hero.
#[derive(Clone, Debug)]
pub struct Worn {
    /// familiar_tree.
    pub familiar: Option<WornModel>,
    /// Player+0x748: the level-30 / 80 familiar.
    pub pet: Option<WornModel>,
    /// weaphold_tree: an amulet's glow.
    pub weaphold: Option<WornModel>,
    /// pup_object, gem_object, wand_object and shield_object.
    pub objects: Vec<WornModel>,
    /// Pojo hides the hero (MBTreeSetFlags(platform, 2)).
    pub hide_body: bool,
    /// shield_object hides the held weapon (weaphold_node).
    pub hide_weapon: bool,
    /// A held shield hides the hero's own (flag 1 on one node).
    pub hide_node: Option<usize>,
    /// MBTreeSetAlpha(p->node): 1 opaque.
    pub alpha: f32,
    /// PlayerApplyPowerupScale.
    pub scale: f32,
    /// Levitation lifts the skeleton 1.5 (pmotion.c:2908).
    pub lift: f32,
    pub mikey: Mikey,
    /// old_flags: last frame's p->flags.
    old_special: u32,
    /// act_bits 0x10000000: a missile left this frame (its aim and whether it
    /// had a target); the familiars answer it the frame after.
    pub(crate) shot: Option<(Vec3, bool)>,
    fire: Option<(Vec3, bool)>,
    /// act_bits 0x20000000: Pojo's swing.
    pub(crate) pojo_swing: bool,
}

impl Default for Worn {
    fn default() -> Self {
        Worn {
            familiar: None,
            pet: None,
            weaphold: None,
            objects: Vec::new(),
            hide_body: false,
            hide_weapon: false,
            hide_node: None,
            alpha: 1.0,
            scale: 1.0,
            lift: 0.0,
            mikey: Mikey::default(),
            old_special: 0,
            shot: None,
            fire: None,
            pojo_swing: false,
        }
    }
}

/// An effect riding a hero's node (SfxSetParent): the head, Pojo's head, or
/// the hero's place; the blast it carries moves with it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct WornFx {
    id: u64,
    player: usize,
    on: FxOn,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum FxOn {
    Root,
    Head,
    PojoHead,
}

/// fNewPUPDescs (player.c:671): the GameCube's extra power-ups by level:
/// 0 Hand of Death, 1 Health Vampire, 2 Mikey (gOverlayPUPParams).
const NEW_PUPS: [(&str, usize, [f32; 3]); 39] = [
    ("G3", 0, [99.9, -20.0, -293.6]),
    ("B3", 2, [72.3, 35.3, -13.5]),
    ("B4", 1, [1.2, 1.0, 25.3]),
    ("B5", 2, [67.3, 5.8, -186.5]),
    ("A1", 0, [50.9, 0.0, 20.6]),
    ("A3", 1, [79.6, 43.6, -111.3]),
    ("A4", 2, [0.6, -16.1, -34.2]),
    ("K1", 2, [57.8, 0.0, 21.8]),
    ("K2", 0, [-21.8, 49.6, 71.7]),
    ("K3", 1, [-116.0, 35.3, -378.6]),
    ("K4", 2, [-77.2, 3.0, 46.0]),
    ("K4", 1, [-201.4, -16.0, 101.7]),
    ("D1", 1, [2.7, 14.6, 95.2]),
    ("D1", 2, [111.8, 12.2, 8.5]),
    ("D2", 1, [32.0, 18.0, -87.3]),
    ("D2", 0, [86.8, 6.4, -47.5]),
    ("D3", 2, [-126.9, 69.3, -17.9]),
    ("D3", 2, [138.7, 65.5, 20.0]),
    ("D4", 1, [-151.2, 8.4, -75.1]),
    ("D4", 2, [-53.5, 5.0, -147.1]),
    ("D4", 0, [97.4, -18.8, -178.0]),
    ("C1", 1, [6.5, -12.1, 12.7]),
    ("C3", 2, [-64.7, 30.0, 58.5]),
    ("C4", 0, [-112.3, -10.0, -142.9]),
    ("C4", 2, [-135.3, 0.0, -26.7]),
    ("C4", 0, [-30.5, 0.0, -13.4]),
    ("C4", 1, [-42.5, 0.0, -27.2]),
    ("I1", 2, [167.4, 4.2, 71.9]),
    ("I1", 0, [295.5, 10.0, -188.8]),
    ("I2", 1, [-46.2, 35.0, 83.0]),
    ("I4", 1, [-17.9, -56.3, 15.1]),
    ("J1", 2, [-5.8, 0.0, -19.2]),
    ("J1", 0, [-89.8, 0.0, -10.6]),
    ("J2", 0, [80.9, 1.5, 48.0]),
    ("J2", 2, [-69.8, 0.1, 144.8]),
    ("J2", 1, [-158.8, 28.0, 111.4]),
    ("J3", 0, [80.9, 55.5, -68.6]),
    ("J4", 2, [133.5, 49.4, 92.1]),
    ("J4", 1, [190.0, 38.6, 174.9]),
];

/// gOverlayPUPParams: the pick-up's tree and its special flags.
const NEW_PUP_PARAMS: [(&str, u32); 3] = [
    ("HANDOFDEATHPUP", pu::S_HANDOFDEATH),
    ("HEALTHVAMPPUP", pu::S_VAMPIRE),
    ("MIKEYPUP", pu::S_MIKEY),
];

/// InLevel: the tag names the level's code (up to its length).
fn in_level(code: &str, tag: &str) -> bool {
    let (c, t) = (code.as_bytes(), tag.as_bytes());
    let at = |s: &[u8], i: usize| s.get(i).copied().unwrap_or(0);
    at(c, 0) == at(t, 0) && (1..4).all(|i| at(c, i) == 0 || at(c, i) == at(t, i))
}

/// The familiar_tree choice (player.c:5791): the model, its anchor.
fn familiar_choice(
    wm: &WornModels,
    special: u32,
    armor: u32,
    action: u16,
) -> Option<(usize, Anchor)> {
    let pick = if special & pu::S_POJO != 0 {
        (wm.pojo, Anchor::Root)
    } else if armor & pu::A_FIRE != 0 && action == 22 {
        (wm.fire_shield, Anchor::Root)
    } else if special & pu::S_PHOENIX != 0 {
        (wm.phoenix, Anchor::Root)
    } else if special & 0x10 != 0 {
        (wm.breath[0], Anchor::Head)
    } else if special & 0x20 != 0 {
        (wm.breath[1], Anchor::Head)
    } else if special & 0x40 != 0 {
        (wm.breath[2], Anchor::Head)
    } else if special & pu::S_LEVITATE != 0 {
        (wm.wings, Anchor::Body)
    } else {
        return None;
    };
    pick.0.map(|m| (m, pick.1))
}

/// Pojo's sequence for the hero's action (player.c:5813): READY 0, RUN 1,
/// ATTACK 2, ATTPWR 3, HIT 4, DEATH 5, and the AnimateATree mode.
fn pojo_anim(action: u16) -> (usize, u8) {
    match action {
        8 | 17..=20 | 22 | 25 | 26 => (1, 0),
        110 => (3, 2),
        27 | 127..=131 | 133 | 135 | 148 => (4, 2),
        126 => (5, 2),
        _ => (0, 0),
    }
}

/// AnimateATree: advance, then change to `seq` (a missing one is 0) in
/// `mode` 0 once the current one is done, in 2 at once (replaying a
/// finished one).
fn animate(anim: &mut Animator, model: &Model, seq: usize, mode: u8, dt: f32) {
    anim.update(&model.skeleton, dt);
    let seq = if seq < model.skeleton.sequences.len() {
        seq
    } else {
        0
    };
    let restart = match mode {
        0 => anim.finished && seq != anim.seq,
        2 => anim.finished || seq != anim.seq,
        _ => true,
    };
    if restart {
        anim.restart(seq);
    }
}

/// MBTreeSetAlpha's value (255 clear) as an opacity.
fn opacity(v: i32) -> f32 {
    (255 - v.clamp(0, 255)) as f32 / 255.0
}

/// CalcTargetDir (combat.c:653): the flat part made unit, the rise lobbed
/// to cover the flat distance at `speed` under `gravity`, `lift` added.
pub(crate) fn calc_target_dir(
    v: &mut Vec3,
    target_scale: f32,
    speed: f32,
    gravity: f32,
    lift: f32,
) {
    let horizontal = v.x.det_hypot(v.z);
    let norm = if horizontal > 0.001 {
        1.0 / horizontal
    } else {
        1.0
    };
    v.x *= norm;
    v.z *= norm;
    v.y = speed * (0.5 * gravity * (horizontal * speed) + (v.y + lift) * (target_scale * norm));
}

impl Game {
    /// InitPlayerMissiles' power-up trees and the objects PlayerProcessPowerups
    /// hangs, from POWERUPS and WEAPONS; each hero's familiars, spit and
    /// amulet glows from its SFX bank.
    pub(super) fn load_worn_models(
        &mut self,
        fs: &GameFs,
        cache: &mut BTreeMap<String, Option<AssetFile>>,
        classes: &[(&str, &str)],
    ) {
        let pups = ["POWERUPS".to_string()];
        let weapons = ["WEAPONS".to_string()];
        let both = ["POWERUPS".to_string(), "WEAPONS".to_string()];
        let mut known = BTreeMap::new();
        let mut get = |g: &mut Game, dirs: &[String], n: &str| {
            let m = g.item_model(fs, cache, &mut known, dirs, n);
            known.remove(n);
            m
        };
        let mut wm = WornModels {
            phoenix: get(self, &pups, "PHOENIX"),
            wings: get(self, &pups, "WINGS"),
            pojo: get(self, &pups, "POJO"),
            fire_shield: get(self, &weapons, "FW_SHLD_ACTIVE"),
            breath: [
                get(self, &pups, "HEAD_BREATHEF"),
                get(self, &pups, "HEAD_BREATHEA"),
                get(self, &pups, "HEAD_BREATHEE"),
            ],
            fball: get(self, &weapons, "PHOENIX_FBALL"),
            superarrow: get(self, &weapons, "SUPERARROW"),
            boss_elec: get(self, &weapons, "BOSSG_ELEC"),
            boss_acid: get(self, &weapons, "BOSSG_ACID"),
            mikey: get(self, &pups, "MIKEYPUP_ON"),
            ..Default::default()
        };
        for (i, n) in FX_NAMES.iter().enumerate() {
            wm.fx[i] = get(self, &weapons, n);
        }
        for (i, n) in OBJECTS.iter().enumerate() {
            wm.objects[i] = get(self, &both, n);
        }
        wm.pojo_head = wm
            .pojo
            .and_then(|m| self.models[m].node_index("BODY1_HE#1"));
        for (i, (_, color)) in classes.iter().enumerate() {
            let ct = self.players[i].class_id;
            let base = crate::heroes::CLASSES[if (8..16).contains(&ct) { ct - 8 } else { ct }];
            let dir = [format!(
                "PLAYERS/{base}/SFX{}",
                color.get(..3).unwrap_or(color)
            )];
            wm.anchors
                .push(Anchors::find(&self.models[self.players[i].model], ct));
            wm.pets
                .push([get(self, &dir, "FAMILIAR1"), get(self, &dir, "FAMILIAR2")]);
            wm.spit.push(get(self, &dir, "FAMILIAR_SPIT"));
            let mut holds = [None; 5];
            for (k, c) in DMG_TYPE_DESC.iter().enumerate().skip(1) {
                holds[k] = get(self, &dir, &format!("WEAP_HOLD_{c}"));
            }
            wm.holds.push(holds);
        }
        self.worn_models = wm;
    }

    /// AppendBigapePowerupsToScene / AppendItemToLevel: this level's Hand of
    /// Death, Health Vampire and Mikey pick-ups (special, 30 seconds).
    pub(super) fn append_bigape_powerups(
        &mut self,
        fs: &GameFs,
        cache: &mut BTreeMap<String, Option<AssetFile>>,
        known: &mut BTreeMap<String, Option<usize>>,
    ) {
        let code = self.level.name.trim_start_matches("level").to_string();
        let dirs = ["POWERUPS".to_string()];
        for (tag, ty, pos) in NEW_PUPS {
            if !in_level(&code, tag) {
                continue;
            }
            let (name, flags) = NEW_PUP_PARAMS[ty];
            let kind = PickupKind::Powerup {
                kind: pu::SPECIAL,
                flags,
                amount: 0.0,
                seconds: 30.0,
            };
            self.note_special(&kind);
            let model = self.item_model(fs, cache, known, &dirs, name);
            // The template's 0.5 radius, as SetItem widens it.
            self.spawn(
                Kind::Pickup { kind, radius: 2.0 },
                model,
                Vec3::from(pos),
                0.0,
            );
        }
    }

    /// The hero's place, turn and PlayerApplyPowerupScale size (p->node).
    pub fn player_xform(&self, pi: usize) -> Mat4 {
        let p = &self.players[pi];
        Mat4::from_translation(p.pos)
            * Mat4::from_rotation_y(p.yaw)
            * Mat4::from_scale(Vec3::splat(p.worn.scale))
    }

    /// An anchor's world matrix this frame.
    fn anchor_world(&self, pi: usize, a: Anchor) -> Mat4 {
        let p = &self.players[pi];
        let base = self.player_xform(pi);
        match self.worn_models.anchors.get(pi).and_then(|n| n.node(a)) {
            Some(node) => {
                let world =
                    self.models[p.model].pose_layers(&[(None, p.animator.seq, p.animator.frame)]);
                base * Mat4::from_translation(Vec3::Y * p.worn.lift)
                    * world.get(node).copied().unwrap_or(Mat4::IDENTITY)
            }
            None => base,
        }
    }

    /// The hero's worn models in its model space (after the player
    /// transform): model, matrix, sequence, frame and opacity.
    pub fn worn_draws(&self, pi: usize) -> Vec<(usize, Mat4, usize, f32, f32)> {
        let p = &self.players[pi];
        let w = &p.worn;
        let anchors = self
            .worn_models
            .anchors
            .get(pi)
            .copied()
            .unwrap_or_default();
        let model = &self.models[p.model];
        let mut world = None;
        let mut out = Vec::new();
        let skeleton_hidden = w.hide_body;
        let all = w
            .familiar
            .iter()
            .chain(w.pet.iter())
            .chain(w.weaphold.iter())
            .chain(w.objects.iter());
        for m in all {
            if m.hidden {
                continue;
            }
            let at = match anchors.node(m.anchor) {
                Some(node) => {
                    // Things on the skeleton go with it (Pojo hides them).
                    if skeleton_hidden {
                        continue;
                    }
                    let world = world.get_or_insert_with(|| {
                        model.pose_layers(&[(None, p.animator.seq, p.animator.frame)])
                    });
                    Mat4::from_translation(Vec3::Y * w.lift)
                        * world.get(node).copied().unwrap_or(Mat4::IDENTITY)
                }
                None if m.anchor == Anchor::Root => Mat4::IDENTITY,
                None => continue,
            };
            out.push((m.model, at * m.local, m.anim.seq, m.anim.frame, m.alpha));
        }
        out
    }

    /// Each frame after the heroes move: the familiars' shots, the Mikey, and
    /// what the power-ups put on each hero; the riding effects follow.
    pub(super) fn update_worn(&mut self, dt: f32) {
        let ticks = (dt * 60.0).round().max(1.0) as i32;
        for pi in 0..self.players.len() {
            if let Some(shot) = self.players[pi].worn.fire.take() {
                self.familiar_shot(pi, shot);
            }
            for _ in 0..ticks {
                self.update_mikey(pi);
            }
            self.dress(pi, dt);
            let w = &mut self.players[pi].worn;
            w.fire = w.shot.take();
        }
        self.follow_worn_fx();
    }

    /// PlayerProcessPowerups from the flag loop on, for one hero.
    fn dress(&mut self, pi: usize, dt: f32) {
        let wm = &self.worn_models;
        let p = &self.players[pi];
        let (special, armor, weapon) = (p.fx.special, p.fx.armor, p.fx.weapon);
        let action = p.act.cur;
        let ct = p.class_id % 8;
        // The timers the flag loop keeps: alpha_time (invisibility) and
        // familiar_time (the last second fades the familiar out).
        let mut alpha_time = 0.0f32;
        let mut familiar_time = -1.0f32;
        for s in p
            .powerups
            .slots
            .iter()
            .filter(|s| s.time != 0.0 && s.state == pu::ON)
        {
            match s.kind {
                pu::ARMOR if s.flags & pu::A_FIRE != 0 && s.time > familiar_time => {
                    familiar_time = s.time;
                }
                pu::SPECIAL => {
                    if s.flags & pu::S_INVISIBLE != 0 && (s.time < 0.0 || s.time > alpha_time) {
                        alpha_time = s.time;
                    }
                    if s.flags & 0x70_04F1 != 0 {
                        if s.time < 0.0 {
                            familiar_time = 99999.0;
                        } else if s.time > familiar_time {
                            familiar_time = s.time;
                        }
                    }
                }
                _ => {}
            }
        }
        // MBTreeSetAlpha(p->node): see-through for the Sorceress's combo
        // (action 0x92) and invisibility, which blinks in its last 3 seconds.
        let alpha = if action == 0x92 {
            opacity(95)
        } else if special & pu::S_INVISIBLE != 0 {
            let v = if !(0.0..=3.0).contains(&alpha_time) || (8.0 * alpha_time) as i32 & 1 != 0 {
                160 + (16.0 * (std::f32::consts::TAU * alpha_time).det_sin()) as i32
            } else {
                0
            };
            opacity(v)
        } else {
            1.0
        };
        // pup_object (left hand), gem_object and wand_object (head),
        // shield_object (right hand).
        let obj = |i: usize, a: Anchor| wm.objects[i].map(|m| WornModel::new(m, a, Mat4::IDENTITY));
        let pup = if special & 0x8000 != 0 {
            obj(0, Anchor::LHand)
        } else if armor & pu::A_REFLECT != 0 {
            obj(1, Anchor::LHand)
        } else if armor & pu::A_FIRE != 0 {
            obj(2, Anchor::LHand)
        } else if armor & pu::A_LIGHTNING != 0 {
            obj(3, Anchor::LHand)
        } else {
            None
        };
        let gem = if special & pu::S_HANDOFDEATH != 0 {
            obj(4, Anchor::Head)
        } else if special & pu::S_VAMPIRE != 0 {
            obj(5, Anchor::Head)
        } else {
            None
        };
        let wand = if special & 0x1000 != 0 {
            obj(6, Anchor::Head)
        } else if special & 0x2000 != 0 {
            obj(7, Anchor::Head)
        } else if armor & pu::A_ANTIDEATH != 0 {
            obj(8, Anchor::Head)
        } else if armor & pu::A_GAS != 0 {
            obj(9, Anchor::Head)
        } else if special & pu::S_XRAY != 0 {
            obj(10, Anchor::Head)
        } else {
            None
        };
        // [CHOICE] lbl_8034489C (the legend sequence) never holds the
        // shield_object back here.
        let shield = if special & 0x4000 != 0 {
            obj(11, Anchor::RHand)
        } else if weapon & pu::W_SUPER != 0 {
            obj(12, Anchor::RHand)
        } else if weapon & pu::W_HAMMER != 0 {
            obj(13, Anchor::RHand)
        } else {
            None
        };
        let anchors = wm.anchors.get(pi).copied().unwrap_or_default();
        // A shield in the left hand hides the Valkyrie's and Knight's own
        // (the hand's parent) and the Jester's hand.
        let hide_node = pup.as_ref().and_then(|_| match ct {
            1 | 5 => anchors
                .lhand
                .and_then(|n| usize::try_from(self.models[p.model].skeleton.nodes[n].parent).ok()),
            7 => anchors.lhand,
            _ => None,
        });
        // weaphold_tree: an amulet's WEAP_HOLD_<colour> in the weapon hand at
        // the tier's offset and scale, unless shield_object took the hand.
        let tier = (p.level / 10).min(9) as usize;
        let kind = (weapon & 0xF) as usize;
        let hold = match (
            &shield,
            wm.holds
                .get(pi)
                .and_then(|h| h.get(kind).copied().flatten()),
        ) {
            (None, Some(m)) if kind != 0 => {
                let pd = self.pdata.get(p.model);
                let off = pd.map_or([0.0; 3], |d| d.weapon_fx_offset[tier]);
                let sc = pd.map_or([0.0; 3], |d| d.weapon_fx_scale[tier]);
                let mut local = Mat4::from_translation(Vec3::from(off));
                if sc[0] != 0.0 {
                    local *= Mat4::from_scale(Vec3::from(sc));
                }
                Some((m, local))
            }
            _ => None,
        };
        let fam = familiar_choice(wm, special, armor, action);
        // PlayerProcessSkinFX: FAMILIAR2 from level 80, FAMILIAR1 from 30, at
        // familiar_offset (x1.2 at 99); hidden while the Phoenix flies.
        let pet_model = wm.pets.get(pi).and_then(|t| {
            if p.level >= 80 {
                t[1]
            } else if p.level >= 30 {
                t[0]
            } else {
                None
            }
        });
        let pet_local = {
            let o = self
                .pdata
                .get(p.model)
                .map_or([0.0; 3], |d| d.familiar_offset);
            let k = if p.level >= 99 { 1.2 } else { 1.0 };
            Mat4::from_translation(Vec3::from(o) * k)
        };
        let scale = if p.class_id == 12 {
            1.6
        } else if special & pu::S_GROWTH != 0 {
            1.3
        } else if p.level >= 99 {
            1.2
        } else {
            1.0
        };
        let growth_ended = p.class_id != 12
            && special & pu::S_GROWTH == 0
            && p.level < 99
            && p.worn.old_special & pu::S_GROWTH != 0;
        let old = p.worn.old_special;
        let shot = p.worn.shot.is_some();

        let models = &self.models;
        let w = &mut self.players[pi].worn;
        w.alpha = alpha;
        w.scale = scale;
        w.lift = if special & pu::S_LEVITATE != 0 {
            1.5
        } else {
            0.0
        };
        w.hide_body = special & pu::S_POJO != 0;
        w.hide_weapon = shield.is_some();
        w.hide_node = hide_node;
        // (The Hand of Death's and Health Vampire's StartGemFX sparkle has no
        // effect here yet.)
        w.objects = [pup, gem, wand, shield].into_iter().flatten().collect();
        for o in &mut w.objects {
            o.alpha = alpha;
        }
        // ReplaceTree keeps a tree that is already there.
        match hold {
            Some((m, local)) => {
                if w.weaphold.as_ref().is_none_or(|h| h.model != m) {
                    w.weaphold = Some(WornModel::new(m, Anchor::RHand, local));
                }
                let h = w.weaphold.as_mut().unwrap();
                h.local = local;
                h.alpha = alpha;
                animate(&mut h.anim, &models[m], 0, 0, dt);
            }
            None => w.weaphold = None,
        }
        match fam {
            Some((m, anchor)) => {
                if w.familiar.as_ref().is_none_or(|f| f.model != m) {
                    w.familiar = Some(WornModel::new(m, anchor, Mat4::IDENTITY));
                }
                let f = w.familiar.as_mut().unwrap();
                let model = &models[m];
                let (mut seq, mut mode) = (0, 0);
                if special & pu::S_POJO != 0 {
                    (seq, mode) = pojo_anim(action);
                    if w.pojo_swing {
                        (seq, mode) = (2, 2);
                    }
                    w.pojo_swing = false;
                } else if shot && model.skeleton.sequences.len() > 1 {
                    (seq, mode) = (1, 2);
                }
                if f.anim.seq == 0 {
                    mode = 2;
                }
                animate(&mut f.anim, model, seq, mode, dt);
                f.alpha = if (0.0..1.0).contains(&familiar_time) {
                    opacity((255.0 * (1.0 - familiar_time)) as i32)
                } else {
                    alpha
                };
            }
            None => w.familiar = None,
        }
        match pet_model {
            Some(m) => {
                if w.pet.as_ref().is_none_or(|f| f.model != m) {
                    w.pet = Some(WornModel::new(m, Anchor::Root, pet_local));
                }
                let f = w.pet.as_mut().unwrap();
                f.local = pet_local;
                f.alpha = alpha;
                f.hidden = special & pu::S_PHOENIX != 0;
                if !f.hidden {
                    let (seq, mode) = if shot { (1, 2) } else { (0, 0) };
                    animate(&mut f.anim, &models[m], seq, mode, dt);
                }
            }
            None => w.pet = None,
        }
        w.old_special = special;
        // The ends: fn_8009D5A0 (Pojo), fn_8009D560 (growth), fn_8009D4F0
        // (levitation).
        if old & pu::S_POJO != 0 && special & pu::S_POJO == 0 {
            self.sfx_id(99);
        }
        if growth_ended {
            self.sfx_id(87);
        }
        if old & pu::S_LEVITATE != 0 && special & pu::S_LEVITATE == 0 {
            self.sfx_id(89);
        }
    }

    /// PlayerProcessMikeyPUP, one frame: switched on it is let out where the
    /// hero stands (the slot goes back to fresh); it stays out 298 frames or
    /// until switched on again, then comes back.
    fn update_mikey(&mut self, pi: usize) {
        let slot = self.players[pi]
            .powerups
            .slots
            .iter()
            .position(|s| s.kind == pu::SPECIAL && s.flags == pu::S_MIKEY);
        let on = slot.is_some_and(|i| self.players[pi].powerups.slots[i].state == pu::ON);
        let p = &mut self.players[pi];
        if p.fx.special & pu::S_MIKEY != 0 && p.worn.mikey.state == 0 {
            p.worn.mikey.state = 1;
        }
        match p.worn.mikey.state {
            0 => {}
            1 => {
                let Some(i) = slot.filter(|_| on) else { return };
                let pos = p.pos;
                p.powerups.slots[i].state = pu::FRESH;
                p.worn.mikey.state = 2;
                p.worn.mikey.pos = pos;
                let model = self.worn_models.mikey;
                let at = pos + Vec3::Y * (PLAYER_HEIGHT * 0.5);
                let id = self.spawn(Kind::Prop, model, at, 0.0);
                self.players[pi].worn.mikey.entity = Some(id);
            }
            300 => {
                if let Some(id) = p.worn.mikey.entity.take() {
                    if let Some(e) = self.entities.iter_mut().find(|e| e.id == id) {
                        e.alive = false;
                    }
                }
                p.worn.mikey.state = 0;
                if let Some(i) = slot {
                    p.powerups.slots[i].state = pu::FRESH;
                }
            }
            _ => {
                // StartGemFX's sparkles every 10 frames of the first 60.
                let t = p.worn.mikey.state;
                if t < 0x3C && t % 10 == 0 {
                    let at = p.worn.mikey.pos + Vec3::Y * (PLAYER_HEIGHT * 0.5);
                    let n = (self.rng.next_u32() % 4 + 1) as i32;
                    self.start_gem_fx(at, n);
                }
                let p = &mut self.players[pi];
                p.worn.mikey.state += 1;
                if on {
                    p.worn.mikey.state = 300;
                }
            }
        }
    }

    /// pmotion.c:2561: the frame after a throw the Phoenix (else the level-30
    /// familiar) shoots too: from fam_proj_offset in the hero's frame at the
    /// throw's aim (ModifyPlayerDpos with 0.03, CalcTargetDir to 50 at 0.02,
    /// lobbed under 10 off boss levels), fn_80093918 at 35: PHOENIX_FBALL for
    /// 10 (fire, knockback), or the hero's FAMILIAR_SPIT for 0.1 x (level -
    /// 25) + 2.5; 3 seconds, colrad 1.
    fn familiar_shot(&mut self, pi: usize, (aim, target): (Vec3, bool)) {
        use crate::missile as m;
        let p = &self.players[pi];
        let phoenix = p.fx.special & pu::S_PHOENIX != 0;
        if !phoenix && p.worn.pet.is_none() {
            return;
        }
        let offset = self
            .pdata
            .get(p.model)
            .map_or([0.0; 3], |d| d.fam_proj_offset);
        let hit = p.pos + m::rotate_offset((Vec3::from(offset) * p.worn.scale).into(), p.yaw);
        let from = Vec3::new(p.yaw.det_sin(), 0.0, p.yaw.det_cos());
        let mut dir = aim;
        let reach = m::modify_dpos(
            from,
            &mut dir,
            0.0,
            p.class_id % 8,
            p.fx.weapon,
            target,
            0.03,
        );
        let mut vel = dir * reach;
        let boss = self.boss.is_some();
        let (weight, lift) = if boss { (0.0, 0.0) } else { (10.0, -0.5) };
        calc_target_dir(&mut vel, 50.0, 0.02, weight, lift);
        let (model, damage, dtype) = if phoenix {
            (self.worn_models.fball, 10.0, 0x11)
        } else {
            let d = (0.1 * (p.level as f64 - 25.0) + 2.5) as f32;
            (self.worn_models.spit.get(pi).copied().flatten(), d, 0)
        };
        // The element counts as the class colour does on neutral monsters.
        let damage = if dtype & 0xF != 0 {
            damage * ELEMENT_BONUS
        } else {
            damage
        };
        let v = vel * 35.0;
        self.spawn(
            Kind::Projectile {
                owner: pi,
                vel: v,
                life: 3.0,
                damage,
                weight,
                radius: 1.0,
                tumble: 0.0,
                reflect: false,
                pierced: None,
                wall_sound: WallSound::None,
                hold: 0.0,
                morph: None,
            },
            model,
            hit,
            v.x.det_atan2(v.z),
        );
    }

    /// pmotion.c:2504, act_bits 0x01000000: the breath. The boss horns or
    /// mask breathe BOSS_BREATHE (50; S 90 / 91), fire breath and Pojo
    /// FIREBREATHE, acid ACIDBREATHE, lightning ELECBREATHE (40; turbo
    /// defence sounds 71 / 72 / 73), radius 20, knocking down, from the head
    /// (Pojo's: its turbo meter pays, S_POJOTURBO); a breath use is spent.
    pub(super) fn start_breath(&mut self, pi: usize) {
        let special = self.players[pi].fx.special;
        let (fx, damage, dtype, sound) = if special & 0x3000 != 0 {
            let s = if special & 0x1000 != 0 { 90 } else { 91 };
            (FX_BOSS_BREATHE, 50.0, 33, Some(s))
        } else if special & 0x410 != 0 {
            (FX_BREATHE_FIRE, 40.0, 33, None)
        } else if special & 0x20 != 0 {
            (FX_BREATHE_ACID, 40.0, 36, None)
        } else if special & 0x40 != 0 {
            (FX_BREATHE_ELEC, 40.0, 34, None)
        } else {
            return;
        };
        match sound {
            Some(s) => self.sfx_id(s),
            // AudioTurboDefense.
            None => {
                if special & 0x10 != 0 {
                    self.sfx_id(71);
                } else if special & 0x20 != 0 {
                    self.sfx_id(72);
                } else if special & 0x40 != 0 {
                    self.sfx_id(73);
                }
            }
        }
        let pojo = special & pu::S_POJO != 0 && self.players[pi].worn.familiar.is_some();
        let on = if pojo { FxOn::PojoHead } else { FxOn::Head };
        if pojo {
            let p = &mut self.players[pi];
            p.power = (p.power - p.act.coll_score).max(0.0);
            p.act.coll_score = 0.0;
            // AudioPlayerTurbo with Pojo: S_POJOTURBO.
            self.sfx_id(95);
        }
        self.start_worn_blast(pi, fx, on, damage, dtype, 20.0, 0.0);
        self.players[pi].powerups.use_one(pu::SPECIAL, pu::S_BREATH);
    }

    /// pmotion.c:2493, act_bits 0x02000000: the hammer's EXPRING on the hero
    /// (100, radius 35, after 0.1, knocking down); a use is spent, the camera
    /// shakes, fn_8009D4B0's sound.
    pub(super) fn start_hammer(&mut self, pi: usize) {
        self.start_worn_blast(pi, FX_EXPRING, FxOn::Root, 100.0, 32, 35.0, 0.1);
        self.players[pi].powerups.use_one(pu::WEAPON, pu::W_HAMMER);
        self.shake.start(0, 0, 30, 0.3, 200);
        self.sfx_id(80);
    }

    /// StartFXSub(type, NULL, 42, 0x800) parented to the hero, with
    /// SfxSetDamage: an expanding blast on monsters and items (flags 0x2A).
    #[allow(clippy::too_many_arguments)]
    fn start_worn_blast(
        &mut self,
        pi: usize,
        fx: usize,
        on: FxOn,
        damage: f32,
        dtype: u32,
        radius: f32,
        delay: f32,
    ) {
        let model = self.worn_models.fx[fx];
        let pos = self.worn_fx_pos(pi, on);
        let maxtime = self.fx_life(model, 0.0);
        let id = self.spawn(
            Kind::Effect {
                life: maxtime + 1.0,
            },
            model,
            pos,
            self.players[pi].yaw,
        );
        self.worn_fx.push(WornFx { id, player: pi, on });
        self.items.blasts.push(items::Blast::new(
            pos,
            damage,
            dtype,
            radius,
            0x2A,
            Some(pi),
            maxtime,
            delay,
            Some(id),
        ));
    }

    fn worn_fx_pos(&self, pi: usize, on: FxOn) -> Vec3 {
        match on {
            FxOn::Root => self.players[pi].pos,
            FxOn::Head => self.anchor_world(pi, Anchor::Head).w_axis.truncate(),
            FxOn::PojoHead => {
                let p = &self.players[pi];
                let base = self.player_xform(pi);
                let node = self.worn_models.pojo_head;
                match (&p.worn.familiar, node) {
                    (Some(f), Some(n)) => {
                        let world =
                            self.models[f.model].pose_layers(&[(None, f.anim.seq, f.anim.frame)]);
                        (base * world.get(n).copied().unwrap_or(Mat4::IDENTITY))
                            .w_axis
                            .truncate()
                    }
                    _ => base.w_axis.truncate(),
                }
            }
        }
    }

    /// The riding effects (and their blasts) keep to their nodes.
    fn follow_worn_fx(&mut self) {
        let fx = std::mem::take(&mut self.worn_fx);
        let mut keep = Vec::new();
        for f in fx {
            if f.player >= self.players.len() {
                continue;
            }
            let pos = self.worn_fx_pos(f.player, f.on);
            let yaw = self.players[f.player].yaw;
            let Some(e) = self
                .entities
                .iter_mut()
                .find(|e| e.id == f.id && e.alive && matches!(e.kind, Kind::Effect { .. }))
            else {
                continue;
            };
            e.pos = pos;
            e.yaw = yaw;
            for b in self.items.blasts.iter_mut().filter(|b| b.fx == Some(f.id)) {
                b.pos = pos;
            }
            keep.push(f);
        }
        self.worn_fx = keep;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> Option<GameFs> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game-data");
        GameFs::open(&root).ok()
    }

    /// A settled level A1 hero with the level emptied.
    fn game(class: &str) -> Option<Game> {
        let fs = data()?;
        let mut g = Game::new(&fs, "levelA1", &[(class, "RED")]).ok()?;
        g.entities.clear();
        for _ in 0..30 {
            g.update(&[PlayerInput::default()]);
        }
        Some(g)
    }

    fn switch_on(g: &mut Game, kind: u8, flags: u32, amount: f32, seconds: f32) {
        let p = &mut g.players[0];
        p.powerups.add(kind, flags, amount, seconds);
        let s = p
            .powerups
            .slots
            .iter_mut()
            .find(|s| s.flags == flags)
            .unwrap();
        s.state = pu::ON;
    }

    fn throw(g: &Game) -> Throw {
        let p = &g.players[0];
        Throw {
            pi: 0,
            pos: p.pos,
            yaw: p.yaw,
            aim: Vec3::new(p.yaw.det_sin(), 0.0, p.yaw.det_cos()),
            target: false,
            bits: crate::paction::act::THROW,
            since_swing: 1.0,
        }
    }

    fn shots_of(g: &Game, model: Option<usize>) -> Vec<(Vec3, f32, f32)> {
        g.entities
            .iter()
            .filter(|e| e.alive && e.model == model)
            .filter_map(|e| match e.kind {
                Kind::Projectile {
                    vel, damage, life, ..
                } => Some((vel, damage, life)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_phoenix_rides_along_and_answers_a_throw() {
        let Some(mut g) = game("WAR") else { return };
        switch_on(&mut g, pu::SPECIAL, pu::S_PHOENIX, 0.0, 60.0);
        g.update(&[PlayerInput::default()]);
        let phoenix = g.worn_models.phoenix;
        assert!(phoenix.is_some());
        let f = g.players[0].worn.familiar.as_ref().unwrap();
        assert_eq!((Some(f.model), f.anchor), (phoenix, Anchor::Root));
        assert!(g.worn_draws(0).iter().any(|d| Some(d.0) == phoenix));
        let fball = g.worn_models.fball;
        g.player_start_missile(throw(&g));
        g.update(&[PlayerInput::default()]);
        // The throw's frame: the Phoenix plays ATTACK; it shoots the next.
        assert_eq!(g.players[0].worn.familiar.as_ref().unwrap().anim.seq, 1);
        assert!(shots_of(&g, fball).is_empty());
        g.update(&[PlayerInput::default()]);
        let shots = shots_of(&g, fball);
        assert_eq!(shots.len(), 1, "one PHOENIX_FBALL");
        let (vel, damage, life) = shots[0];
        assert!((vel.x.det_hypot(vel.z) - 35.0).abs() < 0.5, "{vel}");
        assert!((damage - 10.0 * ELEMENT_BONUS).abs() < 1e-4);
        assert!(life <= 3.0);
    }

    #[test]
    fn the_level_30_familiar_spits_too() {
        let Some(mut g) = game("WAR") else { return };
        g.players[0].level = 50;
        g.update(&[PlayerInput::default()]);
        assert!(g.players[0].worn.pet.is_some(), "FAMILIAR1 at 30");
        let spit = g.worn_models.spit[0];
        g.player_start_missile(throw(&g));
        g.update(&[PlayerInput::default()]);
        g.update(&[PlayerInput::default()]);
        let shots = shots_of(&g, spit);
        assert_eq!(shots.len(), 1);
        assert!((shots[0].1 - 5.0).abs() < 1e-4, "0.1 x 25 + 2.5");
    }

    #[test]
    fn pojo_hides_the_hero_spits_fireballs_and_breathes_on_its_meter() {
        let Some(mut g) = game("WAR") else { return };
        switch_on(&mut g, pu::SPECIAL, pu::S_POJO, 0.0, 60.0);
        g.update(&[PlayerInput::default()]);
        let w = &g.players[0].worn;
        assert!(w.hide_body);
        assert_eq!(w.familiar.as_ref().map(|f| f.model), g.worn_models.pojo);
        let fball = g.worn_models.fball;
        g.player_start_missile(throw(&g));
        assert_eq!(shots_of(&g, fball).len(), 1, "Pojo spits PHOENIX_FBALL");
        g.players[0].power = 100.0;
        g.players[0].act.coll_score = 40.0;
        g.sfx.clear();
        g.start_breath(0);
        assert_eq!(g.players[0].power, 60.0);
        assert!(g.sfx.iter().any(|s| s == "#5f"), "S_POJOTURBO");
        assert!(g
            .items
            .blasts
            .iter()
            .any(|b| b.owner == Some(0) && b.radius == 20.0 && b.damage == 40.0));
        // Switched off: the hero is back, with fn_8009D5A0's sound.
        g.players[0].powerups.clear();
        g.sfx.clear();
        g.update(&[PlayerInput::default()]);
        assert!(!g.players[0].worn.hide_body);
        assert!(g.sfx.iter().any(|s| s == "#63"));
    }

    #[test]
    fn the_hammer_rings_once_a_use() {
        let Some(mut g) = game("WAR") else { return };
        switch_on(&mut g, pu::WEAPON, pu::W_HAMMER, 3.0, -1.0);
        g.update(&[PlayerInput::default()]);
        // HAMMER_HD in the hand puts the weapon away.
        assert!(g.players[0].worn.hide_weapon);
        g.sfx.clear();
        g.start_hammer(0);
        assert!(g.sfx.iter().any(|s| s == "#50"));
        let b = g.items.blasts.last().unwrap();
        assert_eq!((b.radius, b.damage, b.delay), (35.0, 100.0, 0.1));
        assert_eq!(g.players[0].powerups.slots[0].amount, 2.0);
    }

    #[test]
    fn a_mikey_goes_out_lures_and_comes_back() {
        let Some(mut g) = game("WAR") else { return };
        switch_on(&mut g, pu::SPECIAL, pu::S_MIKEY, 0.0, 30.0);
        let at = g.players[0].pos;
        g.update(&[PlayerInput::default()]);
        let m = &g.players[0].worn.mikey;
        assert_eq!(m.state, 2);
        assert_eq!(m.pos, at);
        let id = m.entity.unwrap();
        assert!(g.entities.iter().any(|e| e.id == id && e.alive));
        assert_eq!(g.players[0].powerups.slots[0].state, pu::FRESH);
        for _ in 0..297 {
            g.update(&[PlayerInput::default()]);
        }
        assert!(g.players[0].worn.mikey.out());
        for _ in 0..3 {
            g.update(&[PlayerInput::default()]);
        }
        assert_eq!(g.players[0].worn.mikey.state, 0);
        assert!(!g.entities.iter().any(|e| e.id == id && e.alive));
        // Again, and switched on while out it comes straight back.
        g.players[0].powerups.slots[0].state = pu::ON;
        g.update(&[PlayerInput::default()]);
        assert_eq!(g.players[0].worn.mikey.state, 2);
        g.players[0].powerups.slots[0].state = pu::ON;
        g.update(&[PlayerInput::default()]);
        g.update(&[PlayerInput::default()]);
        assert_eq!(g.players[0].worn.mikey.state, 0);
    }

    #[test]
    fn level_a1_has_its_hand_of_death() {
        let Some(fs) = data() else { return };
        let g = Game::new(&fs, "levelA1", &[("WAR", "RED")]).unwrap();
        assert!(g.entities.iter().any(|e| matches!(
            e.kind,
            Kind::Pickup {
                kind: PickupKind::Powerup {
                    kind: pu::SPECIAL,
                    flags: pu::S_HANDOFDEATH,
                    ..
                },
                ..
            }
        ) && e.model.is_some()
            && (e.pos - Vec3::new(50.9, 0.0, 20.6)).length() < 1e-3));
    }

    #[test]
    fn in_level_matches_the_code() {
        assert!(in_level("A1", "A1"));
        assert!(!in_level("A2", "A1"));
        assert!(!in_level("A1S", "A1"));
        assert_eq!(
            NEW_PUPS
                .iter()
                .filter(|(t, _, _)| in_level("K4", t))
                .count(),
            2
        );
    }

    #[test]
    fn familiar_order_follows_player_c() {
        let wm = WornModels {
            phoenix: Some(1),
            wings: Some(2),
            pojo: Some(3),
            fire_shield: Some(4),
            breath: [Some(5), Some(6), Some(7)],
            ..Default::default()
        };
        let c = |s, a, act| familiar_choice(&wm, s, a, act);
        assert_eq!(c(pu::S_POJO | pu::S_PHOENIX, 0, 0), Some((3, Anchor::Root)));
        assert_eq!(c(pu::S_PHOENIX, pu::A_FIRE, 22), Some((4, Anchor::Root)));
        assert_eq!(c(pu::S_PHOENIX, pu::A_FIRE, 0), Some((1, Anchor::Root)));
        assert_eq!(c(0x20 | pu::S_LEVITATE, 0, 0), Some((6, Anchor::Head)));
        assert_eq!(c(pu::S_LEVITATE, 0, 0), Some((2, Anchor::Body)));
        assert_eq!(c(0, 0, 0), None);
    }

    #[test]
    fn calc_target_dir_lobs_the_familiar_shot() {
        // Straight ahead at reach 21: unit flat, rising to fall back.
        let mut v = Vec3::new(0.0, 0.0, 21.0);
        calc_target_dir(&mut v, 50.0, 0.02, 10.0, -0.5);
        assert!((v.z - 1.0).abs() < 1e-6);
        let want = 0.02 * (0.5 * 10.0 * (21.0 * 0.02) + -0.5 * (50.0 / 21.0));
        assert!((v.y - want).abs() < 1e-6);
    }

    #[test]
    fn pojo_runs_and_gets_hit() {
        assert_eq!(pojo_anim(8), (1, 0));
        assert_eq!(pojo_anim(110), (3, 2));
        assert_eq!(pojo_anim(129), (4, 2));
        assert_eq!(pojo_anim(126), (5, 2));
        assert_eq!(pojo_anim(0), (0, 0));
    }
}
