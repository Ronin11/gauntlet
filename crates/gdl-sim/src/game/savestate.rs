//! Debug save states (not in the original): a quick save of a level in
//! progress that a *new build* can load, so a fix can be swapped in and play
//! picked up where it was.
//!
//! It is not a byte-exact snapshot. Loading starts the level fresh the normal
//! way and then applies what changed: the party (stats, inventory, power-ups,
//! where each hero stands), the progress, and the level's differences from a
//! fresh load, matched by each placed item's instance index in the level
//! file (stable across code changes, unlike entity ids):
//!
//! - placed items gone (picked up, broken, opened doors, killed placed
//!   monsters, destroyed generators): removed, and their collision with them;
//! - placed monsters' and generators' hit points, places and tiers; chests
//!   opened; triggers pressed; secret icons used;
//! - items lying loose (dropped from chests and monsters);
//! - trigger targets (elevators, bridges, gates): on/off and height;
//! - world animations' frames and the rotators' turns;
//! - the level clock, crystals, shards, runes and legend weapons.
//!
//! Not kept: monsters a generator made (it makes new ones), missiles and
//! effects, the camera (it is recomputed), monster AI state, exits half way
//! open, fallen rocks, critter (boss, golem) health, and the level a secret
//! level was entered from (a state saved in a secret level loads that secret
//! level on its own).
//!
//! The file is plain text (`key value` lines, `[section]` heads,
//! `key=value` fields), versioned; unknown keys are ignored and missing ones
//! take defaults, so older and newer builds read each other's states.

use super::PanelInfo;
use super::*;
use crate::powerups::{MiniInv, Slot};
use crate::progress::Progress;
use glam::Mat4;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// The format version written. Readers accept any version (tolerantly).
pub const VERSION: u32 = 1;

/// The slots the debug menu offers, and the one written on quitting a level.
pub const SLOTS: [&str; 3] = ["1", "2", "3"];
pub const AUTOSAVE: &str = "autosave";

#[derive(Clone, Debug, Default)]
pub struct SaveState {
    pub version: u32,
    /// The level's directory ("levelA1") and wave.
    pub level: String,
    pub wave: Option<u32>,
    /// Ticks and seconds played in the level.
    pub frame: u32,
    pub time: f32,
    /// The difficulty the level was played at (0 Easy, 1 Medium, 2 Hard).
    pub difficulty: usize,
    pub heroes: Vec<HeroState>,
    pub progress: Progress,
    pub world: WorldState,
}

/// One hero: the HUD panel that makes it, its controller, and its state.
#[derive(Clone, Debug, Default)]
pub struct HeroState {
    pub panel: PanelInfo,
    /// "keyboard", "pad <n>" (the n-th connected pad) or "device".
    pub source: String,
    pub pos: Vec3,
    pub yaw: f32,
    /// "normal", "intower" or "exited" (a dead hero comes back normal).
    pub state: String,
    pub health: f32,
    pub power: f32,
    pub level: u32,
    pub exp: u32,
    pub bonus: crate::stats::Atts,
    pub keys: u32,
    pub gold: u32,
    pub potions: Vec<u8>,
    pub powerups: Vec<Slot>,
    pub inv: MiniInv,
    pub stats: PlayStats,
    pub sumner_exp: u32,
    pub coins: u32,
    pub class_unlock: u16,
}

impl HeroState {
    /// What the hero carries from level to level, as saved.
    pub fn carried(&self) -> Carried {
        let mut slots = crate::powerups::Powerups::default();
        for (d, s) in slots.slots.iter_mut().zip(&self.powerups) {
            *d = *s;
        }
        Carried {
            health: self.health.max(1.0),
            gold: self.gold,
            keys: self.keys,
            potions: self.potions.clone(),
            level: self.level.max(1),
            exp: self.exp,
            bonus: self.bonus,
            powerups: slots,
            stats: self.stats,
            sumner_exp: self.sumner_exp,
        }
    }
}

/// What changed in the level.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WorldState {
    pub crystals: [i32; 9],
    pub garg_items: [i32; 3],
    pub shards: u16,
    pub runes: u16,
    pub boss_runes: u16,
    pub legend_held: u16,
    pub deaths: u32,
    pub secret_timer: Option<(f32, f32)>,
    pub hidden: Vec<usize>,
    /// Placed item instances no longer in the level.
    pub gone: Vec<u32>,
    pub placed: Vec<Placed>,
    pub loose: Vec<Loose>,
    pub movers: Vec<MoverState>,
    pub anims: Vec<AnimState>,
    pub rotators: Vec<RotState>,
}

/// A placed item still in the level (one record per entity it made).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Placed {
    pub inst: u32,
    /// What it is: enemy, generator, pickup, container, door, exit, hazard,
    /// secret, trigger, critter, faller, prop, pending (a placed monster not
    /// yet in view), other.
    pub tag: String,
    pub pos: Vec3,
    pub yaw: f32,
    pub hp: Option<f32>,
    pub strength: Option<usize>,
    /// recognized (enemy), pressed (trigger), used (secret), open (exit).
    pub flag: bool,
    pub idle: Option<f32>,
}

/// An item lying loose (not placed by the level file).
#[derive(Clone, Debug, PartialEq)]
pub struct Loose {
    pub kind: PickupKind,
    /// The item's model by item name, if it has one.
    pub model: Option<String>,
    pub pos: Vec3,
    pub yaw: f32,
    pub radius: f32,
}

/// A trigger target, by its root world object.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MoverState {
    pub root: usize,
    pub active: bool,
    pub offset: f32,
    pub shown: bool,
}

/// A world animation, by its object.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AnimState {
    pub obj: usize,
    /// The object's direction flags (FORWARD / REVERSE / AT_* / ACTING).
    pub flags: u32,
    pub state: u16,
    pub frame: f32,
}

/// A rotator, by its target object.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RotState {
    pub target: usize,
    pub curang: f32,
    pub on: bool,
    pub done: bool,
    pub node: Mat4,
}

/// World-object flag bits a save state keeps for animated objects.
const ANIM_FLAGS: u32 = crate::worldanim::wflags::FORWARD
    | crate::worldanim::wflags::REVERSE
    | crate::worldanim::wflags::AT_START
    | crate::worldanim::wflags::AT_END
    | crate::worldanim::wflags::ACTING
    | crate::worldanim::wflags::ANIMATING;

/// The tag a save state gives an entity's kind.
fn tag(k: &Kind) -> &'static str {
    match k {
        Kind::Enemy { .. } => "enemy",
        Kind::Generator { .. } => "generator",
        Kind::Pickup { .. } => "pickup",
        Kind::Container { .. } => "container",
        Kind::Door { .. } => "door",
        Kind::Exit { .. } => "exit",
        Kind::Hazard { .. } => "hazard",
        Kind::Secret { .. } => "secret",
        Kind::Trigger { .. } => "trigger",
        Kind::Critter(_) => "critter",
        Kind::Faller { .. } => "faller",
        Kind::Prop => "prop",
        _ => "other",
    }
}

impl Game {
    /// populate's bookkeeping: the entities made since `first` came from
    /// item instance `inst`.
    pub(crate) fn note_placed(&mut self, (inst, first): (u32, u64)) {
        for id in first..self.next_id {
            self.placed.insert(id, inst);
        }
    }

    /// The level's state, as a save state keeps it (the party and progress
    /// are filled in by [`SaveState::capture`]).
    pub fn capture_world(&self) -> WorldState {
        let mut w = WorldState {
            crystals: self.crystals,
            garg_items: self.garg_items,
            shards: self.shards,
            runes: self.runes,
            boss_runes: self.boss_runes,
            legend_held: self.legend_held,
            deaths: self.deaths,
            secret_timer: self.secret_timer,
            ..Default::default()
        };
        w.hidden = self.hidden.iter().copied().collect();
        w.hidden.sort_unstable();
        let mut present: BTreeSet<u32> = BTreeSet::new();
        let model_names: BTreeMap<usize, &String> = self
            .item_models
            .iter()
            .filter_map(|(n, m)| m.map(|m| (m, n)))
            .collect();
        for e in self.entities.iter().filter(|e| e.alive) {
            let Some(&inst) = self.placed.get(&e.id) else {
                if let Kind::Pickup { kind, radius } = &e.kind {
                    w.loose.push(Loose {
                        kind: *kind,
                        model: e
                            .model
                            .and_then(|m| model_names.get(&m))
                            .map(|s| s.to_string()),
                        pos: e.pos,
                        yaw: e.yaw,
                        radius: *radius,
                    });
                }
                continue;
            };
            present.insert(inst);
            let mut p = Placed {
                inst,
                tag: tag(&e.kind).into(),
                pos: e.pos,
                yaw: e.yaw,
                ..Default::default()
            };
            match &e.kind {
                Kind::Enemy {
                    hp,
                    recognized,
                    strength,
                    ..
                } => {
                    p.hp = Some(*hp);
                    p.flag = *recognized;
                    p.strength = Some(*strength);
                }
                Kind::Generator { hp, strength, .. } => {
                    p.hp = Some(*hp);
                    p.strength = Some(*strength);
                }
                Kind::Container { hp, .. } => p.hp = Some(*hp),
                Kind::Trigger { pressed, idle, .. } => {
                    p.flag = *pressed;
                    p.idle = Some(*idle);
                }
                Kind::Secret { used, .. } => p.flag = *used,
                Kind::Exit { open, .. } => p.flag = *open,
                _ => {}
            }
            w.placed.push(p);
        }
        for pe in &self.pending_enemies {
            present.insert(pe.inst);
            w.placed.push(Placed {
                inst: pe.inst,
                tag: "pending".into(),
                pos: pe.pos,
                yaw: pe.yaw,
                ..Default::default()
            });
        }
        let mut gone: Vec<u32> = self
            .placed
            .values()
            .copied()
            .filter(|i| !present.contains(i))
            .collect();
        gone.sort_unstable();
        gone.dedup();
        w.gone = gone;
        w.movers = self
            .movers
            .iter()
            .map(|m| MoverState {
                root: m.root,
                active: m.active,
                offset: m.offset,
                shown: m.shown,
            })
            .collect();
        let (anims, rots) = self.world_anim.save_state();
        w.anims = anims
            .into_iter()
            .map(|(obj, state, frame)| AnimState {
                obj,
                flags: self.world_anim.flags.get(obj).copied().unwrap_or(0) & ANIM_FLAGS,
                state,
                frame,
            })
            .collect();
        w.rotators = rots
            .into_iter()
            .map(|(target, curang, on, done, node)| RotState {
                target,
                curang,
                on,
                done,
                node,
            })
            .collect();
        w
    }

    /// Apply a saved level state to this freshly loaded level.
    pub fn apply_world(&mut self, w: &WorldState) {
        self.crystals = w.crystals;
        self.garg_items = w.garg_items;
        self.shards = w.shards;
        self.runes = w.runes;
        self.boss_runes = w.boss_runes;
        self.set_legend_held(w.legend_held);
        self.deaths = w.deaths;
        if self.secret_timer.is_some() {
            self.secret_timer = w.secret_timer.or(self.secret_timer);
        }
        let hidden: BTreeSet<usize> = w.hidden.iter().copied().collect();
        if hidden != self.hidden {
            self.hidden = hidden;
            self.world_changed = true;
        }

        let gone: BTreeSet<u32> = w.gone.iter().copied().collect();
        let saved_insts: BTreeSet<u32> = w.placed.iter().map(|p| p.inst).collect();
        // Gone: the item and its collision.
        for e in &mut self.entities {
            let Some(&inst) = self.placed.get(&e.id) else {
                continue;
            };
            if gone.contains(&inst) {
                e.alive = false;
            }
        }
        for inst in &gone {
            let owner = 100_000 + *inst as i32;
            for (t, en) in self
                .collision
                .tris
                .iter()
                .zip(self.collision.enabled.iter_mut())
            {
                if t.owner == owner {
                    *en = false;
                }
            }
        }
        // Placed monsters killed before coming into view, or in view now.
        let pending_saved: BTreeSet<u32> = w
            .placed
            .iter()
            .filter(|p| p.tag == "pending")
            .map(|p| p.inst)
            .collect();
        // A placed monster that was in view (saved as an entity) but waits
        // here comes into the level now, to be matched below.
        let mut i = 0;
        while i < self.pending_enemies.len() {
            let inst = self.pending_enemies[i].inst;
            if gone.contains(&inst) {
                self.pending_enemies.remove(i);
                // Remembered as placed, so a state saved from here keeps it gone.
                self.placed.insert(u64::MAX - inst as u64, inst);
            } else if saved_insts.contains(&inst) && !pending_saved.contains(&inst) {
                let pe = self.pending_enemies.remove(i);
                if pe.statue.is_none() {
                    let id = self.spawn(pe.kind, pe.model, pe.pos, pe.yaw);
                    self.placed.insert(id, inst);
                }
            } else {
                i += 1;
            }
        }

        // Match each saved record to the fresh entity of the same instance and kind.
        let mut by_inst: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
        for (i, e) in self.entities.iter().enumerate() {
            if !e.alive {
                continue;
            }
            if let Some(&inst) = self.placed.get(&e.id) {
                by_inst.entry(inst).or_default().push(i);
            }
        }
        let mut used: BTreeSet<usize> = BTreeSet::new();
        for p in &w.placed {
            if p.tag == "pending" {
                continue;
            }
            let found = by_inst.get(&p.inst).and_then(|v| {
                v.iter()
                    .copied()
                    .find(|&i| !used.contains(&i) && tag(&self.entities[i].kind) == p.tag)
            });
            let Some(i) = found else {
                continue;
            };
            used.insert(i);
            self.apply_placed(i, p);
        }
        // Entities of a saved instance that the save doesn't list (a risen
        // statue's prop) are gone too.
        for (inst, v) in &by_inst {
            if !saved_insts.contains(inst) {
                continue;
            }
            for &i in v {
                if !used.contains(&i) {
                    self.entities[i].alive = false;
                }
            }
        }
        // Loose items made by the load itself are in the saved list already.
        for e in &mut self.entities {
            if matches!(e.kind, Kind::Pickup { .. }) && !self.placed.contains_key(&e.id) {
                e.alive = false;
            }
        }
        self.entities.retain(|e| e.alive);

        for l in &w.loose {
            let model = l
                .model
                .as_ref()
                .and_then(|n| {
                    self.item_models
                        .get(&n.to_ascii_uppercase())
                        .copied()
                        .flatten()
                })
                .or_else(|| self.pickup_model(&l.kind));
            self.spawn(
                Kind::Pickup {
                    kind: l.kind,
                    radius: l.radius,
                },
                model,
                l.pos,
                l.yaw,
            );
        }

        for m in &w.movers {
            if let Some(mv) = self.movers.iter_mut().find(|mv| mv.root == m.root) {
                mv.restore_state(
                    m.active,
                    m.offset,
                    m.shown,
                    &mut self.collision,
                    &mut self.world_anim,
                );
            }
        }
        for a in &w.anims {
            if let Some(f) = self.world_anim.flags.get_mut(a.obj) {
                *f = (*f & !ANIM_FLAGS) | (a.flags & ANIM_FLAGS);
            }
            self.world_anim.restore_anim(a.obj, a.state, a.frame);
        }
        for r in &w.rotators {
            self.world_anim
                .restore_rotator(r.target, r.curang, r.on, r.done, r.node);
        }
        // Pose the world at the restored frames, and its collision with it,
        // and the items parented to its moving floors where it puts them.
        self.world_anim.update(&mut self.collision, 0.0);
        self.collision.refit_dynamic();
        self.pose_riding_items();
    }

    fn apply_placed(&mut self, i: usize, p: &Placed) {
        let mut gen_model = None;
        let e = &mut self.entities[i];
        match &mut e.kind {
            Kind::Enemy {
                hp,
                recognized,
                strength,
                ..
            } => {
                if let Some(h) = p.hp {
                    *hp = h;
                }
                *recognized = p.flag;
                if let Some(s) = p.strength {
                    *strength = s;
                }
                e.pos = p.pos;
                e.yaw = p.yaw;
            }
            Kind::Generator {
                hp,
                strength,
                class,
                ..
            } => {
                if let Some(h) = p.hp {
                    *hp = h;
                }
                if let Some(s) = p.strength.filter(|s| s != strength) {
                    *strength = s;
                    gen_model = Some((*class, s));
                }
            }
            Kind::Container { hp, chest, .. } => {
                if let Some(h) = p.hp {
                    *hp = h;
                    // An opened chest shows open.
                    if h <= 0.0 && *chest {
                        if let Some(s) = e.model.and_then(|m| self.models[m].sequence("OPEN")) {
                            e.animator.restart(s);
                        }
                    }
                }
            }
            Kind::Trigger { pressed, idle, .. } => {
                *pressed = p.flag;
                if let Some(t) = p.idle {
                    *idle = t;
                }
            }
            Kind::Secret { used, .. } => *used = p.flag,
            Kind::Pickup { .. } => e.pos = p.pos,
            // Critters walk about (their health and state start over).
            Kind::Critter(_) => {
                e.pos = p.pos;
                e.yaw = p.yaw;
            }
            _ => {}
        }
        if let Some((class, s)) = gen_model {
            if let Some(m) = self.classes.get(class).and_then(|c| c.gen_model(s)) {
                self.entities[i].model = Some(m);
            }
        }
    }

    /// Where a hero can stand near `pos`: `pos` itself when there is floor
    /// under it and no wall around it, else the nearest such place within 24
    /// units, else `fallback`.
    pub fn safe_spot(&self, pos: Vec3, fallback: Vec3) -> Vec3 {
        let ok = |p: Vec3| -> Option<Vec3> {
            let y = self.collision.ground_height(p, p.y + STEP_HEIGHT)?;
            if p.y - y > 6.0 {
                return None;
            }
            let at = Vec3::new(p.x, y.max(p.y.min(y + 0.5)), p.z);
            let pushed =
                self.collision
                    .resolve_walls(at, PLAYER_RADIUS, PLAYER_HEIGHT, STEP_HEIGHT);
            (Vec2::new(pushed.x - at.x, pushed.z - at.z).length() < 0.25).then_some(at)
        };
        if let Some(p) = ok(pos) {
            return p;
        }
        for r in 1..=24 {
            for k in 0..16 {
                let a = k as f32 * std::f32::consts::TAU / 16.0;
                let q = pos + Vec3::new(a.cos() * r as f32, 0.0, a.sin() * r as f32);
                for dy in [0.0, 3.0, -3.0, 6.0] {
                    if let Some(p) = ok(q + Vec3::Y * dy) {
                        return p;
                    }
                }
            }
        }
        fallback
    }

    /// Put a hero back as saved.
    pub fn apply_hero(&mut self, pi: usize, h: &HeroState) {
        let Some(fallback) = self.players.get(pi).map(|p| p.pos) else {
            return;
        };
        let c = h.carried();
        let pos = self.safe_spot(h.pos, fallback);
        let p = &mut self.players[pi];
        p.restore(&c);
        p.health = p.health.max(1.0);
        p.power = h.power;
        p.inv = h.inv;
        p.coins = h.coins;
        p.class_unlock |= h.class_unlock;
        p.pos = pos;
        p.yaw = h.yaw;
        p.vy = 0.0;
        p.state = match h.state.as_str() {
            "intower" => PlayerState::InTower,
            "exited" => PlayerState::Exited,
            _ => PlayerState::Normal,
        };
        if pi == 0 {
            self.cam_focus = pos;
        }
    }
}

impl SaveState {
    /// The running level and party. `panels`, `sources` and `pads` give each
    /// hero's panel and controller name (empty: from the heroes themselves).
    pub fn capture(
        g: &Game,
        panels: &[PanelInfo],
        source_names: &[String],
        progress: &Progress,
    ) -> SaveState {
        let level = g
            .wave
            .and_then(|w| g.worlds.dir(w))
            .unwrap_or_else(|| g.level.name.clone());
        let heroes = g
            .players
            .iter()
            .enumerate()
            .map(|(i, p)| HeroState {
                panel: panels.get(i).cloned().unwrap_or_else(|| PanelInfo {
                    slot: i,
                    name: String::new(),
                    colour: crate::heroes::COLOURS
                        .iter()
                        .position(|c| *c == p.color)
                        .unwrap_or(i % 4),
                    class: p.class_id,
                    hidden: None,
                }),
                source: source_names.get(i).cloned().unwrap_or_default(),
                pos: p.pos,
                yaw: p.yaw,
                state: match p.state {
                    PlayerState::InTower => "intower",
                    PlayerState::Exited => "exited",
                    _ => "normal",
                }
                .into(),
                health: p.health,
                power: p.power,
                level: p.level,
                exp: p.exp,
                bonus: p.bonus,
                keys: p.keys,
                gold: p.gold,
                potions: p.potion_colours.clone(),
                powerups: p.powerups.slots.to_vec(),
                inv: p.inv,
                stats: p.stats,
                sumner_exp: p.sumner_exp,
                coins: p.coins,
                class_unlock: p.class_unlock,
            })
            .collect();
        let mut progress = progress.clone();
        progress.crystals = g.crystals;
        progress.garg_items = g.garg_items;
        progress.shards = g.shards;
        progress.runes |= g.runes | g.boss_runes;
        progress.legend = g.legend_held;
        SaveState {
            version: VERSION,
            level,
            wave: g.wave,
            frame: g.frame,
            time: g.time,
            difficulty: g.difficulty,
            heroes,
            progress,
            world: g.capture_world(),
        }
    }

    /// The classes, colours and hidden models to start the level with
    /// (Game::new_heroes' arguments).
    pub fn classes(&self) -> Vec<(String, String, Option<String>)> {
        self.heroes
            .iter()
            .map(|h| {
                let p = &h.panel;
                let class = if p.class >= 16 {
                    "WIZ"
                } else {
                    crate::heroes::CLASSES
                        .get(p.class)
                        .copied()
                        .unwrap_or("WAR")
                };
                let model = match p.hidden {
                    _ if p.class >= 16 => Some("SUM".to_string()),
                    Some(k) => crate::cheats::HIDDEN.get(k).map(|h| h.dir.to_string()),
                    None => None,
                };
                (
                    class.to_string(),
                    crate::heroes::COLOURS[p.colour % 4].to_string(),
                    model,
                )
            })
            .collect()
    }

    /// Start the saved level the way the headless runner does and apply the
    /// state to it.
    pub fn load_game(&self, fs: &crate::fs::GameFs) -> anyhow::Result<Game> {
        let cl = self.classes();
        let classes: Vec<(&str, &str)> = cl
            .iter()
            .map(|(c, k, _)| (c.as_str(), k.as_str()))
            .collect();
        let models: Vec<Option<&str>> = cl.iter().map(|(_, _, m)| m.as_deref()).collect();
        let mut g = Game::new_heroes(fs, &self.level, &classes, &models, self.difficulty)?;
        self.apply(&mut g);
        Ok(g)
    }

    /// Apply to a freshly started level: its changes, then the heroes.
    pub fn apply(&self, g: &mut Game) {
        g.apply_world(&self.world);
        g.time = self.time;
        g.frame = self.frame;
        for (i, h) in self.heroes.iter().enumerate() {
            g.apply_hero(i, h);
        }
        // The camera starts on the heroes (as in the tower), not with the
        // level-start view.
        g.camera = crate::camera::GameCamera::start(
            &g.level.worlds,
            g.camera_params(),
            true,
            g.start_index,
            &g.camera_party(),
        );
    }

    /// A short description for logs and tests.
    pub fn summary(&self) -> String {
        let mut s = format!(
            "{} t={:.1}s gone={} placed={} loose={} movers_on={}",
            self.level,
            self.time,
            self.world.gone.len(),
            self.world.placed.len(),
            self.world.loose.len(),
            self.world.movers.iter().filter(|m| m.active).count()
        );
        for (i, h) in self.heroes.iter().enumerate() {
            let _ = write!(
                s,
                " | P{i} pos ({:.1},{:.1},{:.1}) hp {:.0} gold {} keys {} potions {} lv {} exp {}",
                h.pos.x,
                h.pos.y,
                h.pos.z,
                h.health,
                h.gold,
                h.keys,
                h.potions.len(),
                h.level,
                h.exp
            );
        }
        s
    }

    // ---- text format ----

    pub fn to_text(&self) -> String {
        let mut o = String::new();
        let _ = writeln!(
            o,
            "# gauntlet-rs debug save state (not in the original game)"
        );
        let _ = writeln!(
            o,
            "# Plain text: unknown keys are ignored, missing ones default."
        );
        let _ = writeln!(o, "version {}", self.version.max(1));
        let _ = writeln!(o, "build {}", env!("CARGO_PKG_VERSION"));
        let _ = writeln!(o, "level {}", self.level);
        if let Some(w) = self.wave {
            let _ = writeln!(o, "wave 0x{w:X}");
        }
        let _ = writeln!(o, "frame {}", self.frame);
        let _ = writeln!(o, "time {}", self.time);
        let _ = writeln!(o, "difficulty {}", self.difficulty);
        for h in &self.heroes {
            let p = &h.panel;
            let _ = writeln!(o, "\n[hero]");
            let _ = writeln!(o, "slot {}", p.slot);
            let _ = writeln!(o, "name {}", esc(&p.name));
            let _ = writeln!(o, "class {}", p.class);
            let _ = writeln!(o, "colour {}", p.colour);
            if let Some(k) = p.hidden {
                let _ = writeln!(o, "hidden {k}");
            }
            let _ = writeln!(o, "source {}", h.source);
            let _ = writeln!(o, "state {}", h.state);
            let _ = writeln!(o, "pos {}", v3(h.pos));
            let _ = writeln!(o, "yaw {}", h.yaw);
            let _ = writeln!(o, "health {}", h.health);
            let _ = writeln!(o, "turbo {}", h.power);
            let _ = writeln!(o, "level {}", h.level);
            let _ = writeln!(o, "exp {}", h.exp);
            let b = &h.bonus;
            let _ = writeln!(o, "bonus {} {} {} {}", b.fight, b.speed, b.armor, b.magic);
            let _ = writeln!(o, "gold {}", h.gold);
            let _ = writeln!(o, "keys {}", h.keys);
            let _ = writeln!(o, "potions {}", join(&h.potions));
            for s in h.powerups.iter().filter(|s| s.kind != 0 || s.time != 0.0) {
                let _ = writeln!(
                    o,
                    "powerup kind={} flags=0x{:X} time={} amount={} state={}",
                    s.kind, s.flags, s.time, s.amount, s.state
                );
            }
            let _ = writeln!(o, "inv {} {} {}", h.inv.state, h.inv.sel, h.inv.slide);
            let st = &h.stats;
            let _ = writeln!(
                o,
                "stats {} {} {} {}",
                st.enemies_killed, st.generators_destroyed, st.gold_found, st.playtime
            );
            let _ = writeln!(o, "sumner_exp {}", h.sumner_exp);
            let _ = writeln!(o, "coins {}", h.coins);
            let _ = writeln!(o, "class_unlock 0x{:X}", h.class_unlock);
        }
        let p = &self.progress;
        let _ = writeln!(o, "\n[progress]");
        let _ = writeln!(o, "beaten {}", hex(&p.beaten));
        let _ = writeln!(o, "runes 0x{:X}", p.runes);
        let _ = writeln!(o, "shards 0x{:X}", p.shards);
        let _ = writeln!(o, "crystals {}", join(&p.crystals));
        let _ = writeln!(o, "garg_items {}", join(&p.garg_items));
        if let Some(r) = p.last_realm {
            let _ = writeln!(o, "last_realm {r}");
        }
        if let Some(b) = p.last_beaten {
            let _ = writeln!(o, "last_beaten 0x{b:X}");
        }
        let _ = writeln!(o, "banked_runes 0x{:X}", p.banked_runes);
        let _ = writeln!(o, "banked_shards 0x{:X}", p.banked_shards);
        let _ = writeln!(o, "entered_tower {}", p.entered_tower as u8);
        let _ = writeln!(o, "boss_attempts {}", hex(&p.boss_attempts));
        let _ = writeln!(o, "level_masks {}", hex(&p.level_masks));
        let _ = writeln!(o, "legend 0x{:X}", p.legend);
        let w = &self.world;
        let _ = writeln!(o, "\n[world]");
        let _ = writeln!(o, "crystals {}", join(&w.crystals));
        let _ = writeln!(o, "garg_items {}", join(&w.garg_items));
        let _ = writeln!(o, "shards 0x{:X}", w.shards);
        let _ = writeln!(o, "runes 0x{:X}", w.runes);
        let _ = writeln!(o, "boss_runes 0x{:X}", w.boss_runes);
        let _ = writeln!(o, "legend_held 0x{:X}", w.legend_held);
        let _ = writeln!(o, "deaths {}", w.deaths);
        if let Some((a, b)) = w.secret_timer {
            let _ = writeln!(o, "secret_timer {a} {b}");
        }
        let _ = writeln!(o, "hidden {}", join(&w.hidden));
        let _ = writeln!(o, "# placed item instances no longer in the level");
        let _ = writeln!(o, "gone {}", join(&w.gone));
        for m in &w.movers {
            let _ = writeln!(
                o,
                "mover root={} active={} offset={} shown={}",
                m.root, m.active as u8, m.offset, m.shown as u8
            );
        }
        for a in &w.anims {
            let _ = writeln!(
                o,
                "anim obj={} flags=0x{:X} state=0x{:X} frame={}",
                a.obj, a.flags, a.state, a.frame
            );
        }
        for r in &w.rotators {
            let n = r.node.to_cols_array();
            let _ = writeln!(
                o,
                "rotator target={} ang={} on={} done={} node={}",
                r.target,
                r.curang,
                r.on as u8,
                r.done as u8,
                n.iter()
                    .map(|f: &f32| f.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            );
        }
        for p in &w.placed {
            let mut l = format!(
                "placed {} {} pos={} yaw={}",
                p.inst,
                p.tag,
                v3(p.pos),
                p.yaw
            );
            if let Some(h) = p.hp {
                let _ = write!(l, " hp={h}");
            }
            if let Some(s) = p.strength {
                let _ = write!(l, " strength={s}");
            }
            if p.flag {
                l.push_str(" flag=1");
            }
            if let Some(t) = p.idle {
                let _ = write!(l, " idle={t}");
            }
            let _ = writeln!(o, "{l}");
        }
        for l in &w.loose {
            let _ = writeln!(
                o,
                "loose kind={} model={} pos={} yaw={} radius={}",
                kind_text(&l.kind),
                l.model.as_deref().unwrap_or("-"),
                v3(l.pos),
                l.yaw,
                l.radius
            );
        }
        o
    }

    /// Read a state, tolerantly: unknown keys and sections are skipped,
    /// missing values default, bad numbers default.
    pub fn from_text(text: &str) -> anyhow::Result<SaveState> {
        let mut st = SaveState::default();
        let mut section = String::new();
        let mut saw_level = false;
        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(h) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                section = h.trim().to_ascii_lowercase();
                if section == "hero" {
                    st.heroes.push(HeroState {
                        level: 1,
                        state: "normal".into(),
                        ..Default::default()
                    });
                    let n = st.heroes.len() - 1;
                    st.heroes[n].panel.slot = n;
                }
                continue;
            }
            let l = Line::parse(line);
            match section.as_str() {
                "" => match l.key {
                    "version" => st.version = l.num(0),
                    "level" => {
                        st.level = l.word(0).to_string();
                        saw_level = !st.level.is_empty();
                    }
                    "wave" => st.wave = l.opt(0),
                    "frame" => st.frame = l.num(0),
                    "time" => st.time = l.num(0),
                    "difficulty" => st.difficulty = l.num::<usize>(0).min(2),
                    _ => {}
                },
                "hero" => {
                    let h = st.heroes.last_mut().unwrap();
                    match l.key {
                        "slot" => h.panel.slot = l.num::<usize>(0) % 4,
                        "name" => h.panel.name = unesc(l.rest),
                        "class" => h.panel.class = l.num::<usize>(0).min(16),
                        "colour" | "color" => h.panel.colour = l.num::<usize>(0) % 4,
                        "hidden" => h.panel.hidden = l.opt(0),
                        "source" => h.source = l.rest.to_string(),
                        "state" => h.state = l.word(0).to_string(),
                        "pos" => h.pos = l.vec3(0).unwrap_or(h.pos),
                        "yaw" => h.yaw = l.num(0),
                        "health" => h.health = l.num(0),
                        "turbo" => h.power = l.num(0),
                        "level" => h.level = l.num::<u32>(0).clamp(1, 99),
                        "exp" => h.exp = l.num(0),
                        "bonus" => {
                            h.bonus = crate::stats::Atts {
                                fight: l.num(0),
                                speed: l.num(1),
                                armor: l.num(2),
                                magic: l.num(3),
                            }
                        }
                        "gold" => h.gold = l.num(0),
                        "keys" => h.keys = l.num::<u32>(0).min(MAX_KEYS),
                        "potions" => {
                            h.potions = l.all();
                            h.potions.truncate(MAX_POTIONS as usize);
                        }
                        "powerup" if h.powerups.len() < 11 => h.powerups.push(Slot {
                            kind: l.field("kind"),
                            flags: l.field("flags"),
                            time: l.field("time"),
                            amount: l.field("amount"),
                            state: l.field("state"),
                        }),
                        "inv" => {
                            h.inv = MiniInv {
                                state: l.num(0),
                                sel: l.opt(1).unwrap_or(-1),
                                slide: l.num(2),
                            }
                        }
                        "stats" => {
                            h.stats = PlayStats {
                                enemies_killed: l.num(0),
                                generators_destroyed: l.num(1),
                                gold_found: l.num(2),
                                playtime: l.num(3),
                            }
                        }
                        "sumner_exp" => h.sumner_exp = l.num(0),
                        "coins" => h.coins = l.num(0),
                        "class_unlock" => h.class_unlock = l.num(0),
                        _ => {}
                    }
                }
                "progress" => {
                    let p = &mut st.progress;
                    match l.key {
                        "beaten" => fill(&mut p.beaten, &l.all::<u16>()),
                        "runes" => p.runes = l.num(0),
                        "shards" => p.shards = l.num(0),
                        "crystals" => fill(&mut p.crystals, &l.all::<i32>()),
                        "garg_items" => fill(&mut p.garg_items, &l.all::<i32>()),
                        "last_realm" => p.last_realm = l.opt(0),
                        "last_beaten" => p.last_beaten = l.opt(0),
                        "banked_runes" => p.banked_runes = l.num(0),
                        "banked_shards" => p.banked_shards = l.num(0),
                        "entered_tower" => p.entered_tower = l.num::<u32>(0) != 0,
                        "boss_attempts" => fill(&mut p.boss_attempts, &l.all::<u16>()),
                        "level_masks" => fill(&mut p.level_masks, &l.all::<u16>()),
                        "legend" => p.legend = l.num(0),
                        _ => {}
                    }
                }
                "world" => {
                    let w = &mut st.world;
                    match l.key {
                        "crystals" => fill(&mut w.crystals, &l.all::<i32>()),
                        "garg_items" => fill(&mut w.garg_items, &l.all::<i32>()),
                        "shards" => w.shards = l.num(0),
                        "runes" => w.runes = l.num(0),
                        "boss_runes" => w.boss_runes = l.num(0),
                        "legend_held" => w.legend_held = l.num(0),
                        "deaths" => w.deaths = l.num(0),
                        "secret_timer" => w.secret_timer = Some((l.num(0), l.num(1))),
                        "hidden" => w.hidden = l.all(),
                        "gone" => w.gone.extend(l.all::<u32>()),
                        "mover" => w.movers.push(MoverState {
                            root: l.field("root"),
                            active: l.field::<u32>("active") != 0,
                            offset: l.field("offset"),
                            shown: l.opt_field::<u32>("shown").is_none_or(|v| v != 0),
                        }),
                        "anim" => w.anims.push(AnimState {
                            obj: l.field("obj"),
                            flags: l.field("flags"),
                            state: l.field("state"),
                            frame: l.field("frame"),
                        }),
                        "rotator" => {
                            let n: Vec<f32> = l
                                .get("node")
                                .unwrap_or("")
                                .split(',')
                                .filter_map(|v| v.trim().parse().ok())
                                .collect();
                            if n.len() == 16 {
                                w.rotators.push(RotState {
                                    target: l.field("target"),
                                    curang: l.field("ang"),
                                    on: l.field::<u32>("on") != 0,
                                    done: l.field::<u32>("done") != 0,
                                    node: Mat4::from_cols_slice(&n),
                                });
                            }
                        }
                        "placed" => {
                            if let Some(inst) = l.opt::<u32>(0) {
                                w.placed.push(Placed {
                                    inst,
                                    tag: l.word(1).to_string(),
                                    pos: l.get("pos").and_then(parse_v3).unwrap_or_default(),
                                    yaw: l.field("yaw"),
                                    hp: l.opt_field("hp"),
                                    strength: l.opt_field("strength"),
                                    flag: l.field::<u32>("flag") != 0,
                                    idle: l.opt_field("idle"),
                                });
                            }
                        }
                        "loose" => {
                            if let (Some(kind), Some(pos)) = (
                                l.get("kind").and_then(parse_kind),
                                l.get("pos").and_then(parse_v3),
                            ) {
                                w.loose.push(Loose {
                                    kind,
                                    model: l.get("model").filter(|m| *m != "-").map(String::from),
                                    pos,
                                    yaw: l.field("yaw"),
                                    radius: l.opt_field("radius").unwrap_or(2.0),
                                });
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        anyhow::ensure!(saw_level, "not a save state (no level line)");
        Ok(st)
    }

    pub fn write(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        // Written beside and renamed, so a crash mid-write keeps the old one.
        let tmp = path.with_extension("state.tmp");
        std::fs::write(&tmp, self.to_text())?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    pub fn read(path: &Path) -> anyhow::Result<SaveState> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
        SaveState::from_text(&text)
    }
}

/// Where states are kept: `<data dir>/states` (the memory card's folder:
/// $XDG_DATA_HOME/gauntlet-rs, or the Android app's files).
pub fn states_dir(save_dir: Option<&Path>) -> Option<PathBuf> {
    save_dir
        .map(Path::to_path_buf)
        .or_else(crate::save::MemCard::default_dir)
        .map(|d| d.join("states"))
}

/// A `--load-state` argument: a slot name ("1", "autosave") in the states
/// folder, or a file path.
pub fn resolve(spec: &str, save_dir: Option<&Path>) -> PathBuf {
    let p = Path::new(spec);
    let is_path = spec.contains('/') || spec.contains('\\') || spec.ends_with(".state");
    if is_path || (p.is_file() && !SLOTS.contains(&spec) && spec != AUTOSAVE) {
        return p.to_path_buf();
    }
    states_dir(save_dir)
        .unwrap_or_else(|| PathBuf::from("states"))
        .join(format!("{spec}.state"))
}

// ---- text helpers ----

/// One `key positional... name=value...` line.
struct Line<'a> {
    key: &'a str,
    /// Everything after the key.
    rest: &'a str,
    words: Vec<&'a str>,
    fields: Vec<(&'a str, &'a str)>,
}

trait Num: Sized + Default {
    fn parse_tok(s: &str) -> Option<Self>;
}
macro_rules! int_num {
    ($($t:ty),*) => {$(
        impl Num for $t {
            fn parse_tok(s: &str) -> Option<Self> {
                let (neg, s) = match s.strip_prefix('-') { Some(r) => (true, r), None => (false, s) };
                let v = match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
                    Some(h) => i128::from_str_radix(h, 16).ok()?,
                    None => s.parse::<i128>().ok().or_else(|| s.parse::<f64>().ok().map(|f| f as i128))?,
                };
                <$t>::try_from(if neg { -v } else { v }).ok()
            }
        }
    )*};
}
int_num!(u8, u16, u32, u64, usize, i32);
impl Num for f32 {
    fn parse_tok(s: &str) -> Option<Self> {
        s.parse::<f32>().ok().filter(|f| f.is_finite())
    }
}

impl<'a> Line<'a> {
    fn parse(line: &'a str) -> Line<'a> {
        let (key, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        let rest = rest.trim();
        let mut words = Vec::new();
        let mut fields = Vec::new();
        for t in rest.split_whitespace() {
            match t.split_once('=') {
                Some((k, v)) => fields.push((k, v)),
                None => words.push(t),
            }
        }
        Line {
            key,
            rest,
            words,
            fields,
        }
    }
    fn word(&self, i: usize) -> &'a str {
        self.words.get(i).copied().unwrap_or("")
    }
    fn opt<T: Num>(&self, i: usize) -> Option<T> {
        self.words.get(i).and_then(|w| T::parse_tok(w))
    }
    fn num<T: Num>(&self, i: usize) -> T {
        self.opt(i).unwrap_or_default()
    }
    fn all<T: Num>(&self) -> Vec<T> {
        self.words.iter().filter_map(|w| T::parse_tok(w)).collect()
    }
    fn get(&self, k: &str) -> Option<&'a str> {
        self.fields.iter().find(|(f, _)| *f == k).map(|(_, v)| *v)
    }
    fn opt_field<T: Num>(&self, k: &str) -> Option<T> {
        self.get(k).and_then(T::parse_tok)
    }
    fn field<T: Num>(&self, k: &str) -> T {
        self.opt_field(k).unwrap_or_default()
    }
    fn vec3(&self, i: usize) -> Option<Vec3> {
        self.words.get(i).and_then(|w| parse_v3(w))
    }
}

fn fill<T: Copy>(dst: &mut [T], src: &[T]) {
    for (d, s) in dst.iter_mut().zip(src) {
        *d = *s;
    }
}

fn join<T: std::fmt::Display>(v: &[T]) -> String {
    v.iter()
        .map(|x| x.to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

fn hex(v: &[u16]) -> String {
    v.iter()
        .map(|x| format!("0x{x:X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn v3(v: Vec3) -> String {
    format!("{},{},{}", v.x, v.y, v.z)
}

fn parse_v3(s: &str) -> Option<Vec3> {
    let v: Vec<f32> = s.split(',').filter_map(f32::parse_tok).collect();
    (v.len() == 3).then(|| Vec3::new(v[0], v[1], v[2]))
}

/// Names may hold spaces: written with %20 and friends.
fn esc(s: &str) -> String {
    let mut o = String::new();
    for c in s.chars() {
        match c {
            '%' | '=' | '#' | ' ' | '\t' | '\n' | '\r' => {
                let _ = write!(o, "%{:02X}", c as u32);
            }
            c => o.push(c),
        }
    }
    o
}

fn unesc(s: &str) -> String {
    let b = s.as_bytes();
    let mut o = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                o.push(v);
                i += 3;
                continue;
            }
        }
        o.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&o).into_owned()
}

fn kind_text(k: &PickupKind) -> String {
    match *k {
        PickupKind::Food(v) => format!("food:{v}"),
        PickupKind::Key(n) => format!("key:{n}"),
        PickupKind::Gold(n) => format!("gold:{n}"),
        PickupKind::Potion(c) => format!("potion:{c}"),
        PickupKind::Crystal(c) => format!("crystal:{c}"),
        PickupKind::Scroll(n) => format!("scroll:{n}"),
        PickupKind::Shard(b) => format!("shard:{b}"),
        PickupKind::Legend(v) => format!("legend:{v}"),
        PickupKind::Gargoyle(g) => format!("garg:{g}"),
        PickupKind::Powerup {
            kind,
            flags,
            amount,
            seconds,
        } => format!("powerup:{kind}:0x{flags:X}:{amount}:{seconds}"),
        PickupKind::Other(n) => format!("other:{n}"),
    }
}

fn parse_kind(s: &str) -> Option<PickupKind> {
    let mut t = s.split(':');
    let name = t.next()?;
    let a: Vec<&str> = t.collect();
    let n = |i: usize| a.get(i).copied().unwrap_or("0");
    Some(match name {
        "food" => PickupKind::Food(f32::parse_tok(n(0))?),
        "key" => PickupKind::Key(u32::parse_tok(n(0))?),
        "gold" => PickupKind::Gold(u32::parse_tok(n(0))?),
        "potion" => PickupKind::Potion(u8::parse_tok(n(0))?),
        "crystal" => PickupKind::Crystal(usize::parse_tok(n(0))?.min(8)),
        "scroll" => PickupKind::Scroll(usize::parse_tok(n(0))?),
        "shard" => PickupKind::Shard(u32::parse_tok(n(0))?.min(12)),
        "legend" => PickupKind::Legend(u32::parse_tok(n(0))?),
        "garg" => PickupKind::Gargoyle(usize::parse_tok(n(0))?.min(2)),
        "powerup" => PickupKind::Powerup {
            kind: u8::parse_tok(n(0))?,
            flags: u32::parse_tok(n(1))?,
            amount: f32::parse_tok(n(2))?,
            seconds: f32::parse_tok(n(3))?,
        },
        "other" => PickupKind::Other(u32::parse_tok(n(0)).unwrap_or(0)),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> SaveState {
        let mut s = SaveState {
            version: VERSION,
            level: "levelA1".into(),
            wave: Some(0x101),
            frame: 1234,
            time: 20.5667,
            difficulty: 1,
            ..Default::default()
        };
        s.heroes.push(HeroState {
            panel: PanelInfo {
                slot: 1,
                name: "BOB THE 50%".into(),
                colour: 2,
                class: 3,
                hidden: Some(4),
            },
            source: "pad 0".into(),
            pos: Vec3::new(1.25, -3.5, 1e-3),
            yaw: -2.75,
            state: "normal".into(),
            health: 612.5,
            power: 77.0,
            level: 12,
            exp: 34567,
            bonus: crate::stats::Atts {
                fight: 5.0,
                speed: 10.0,
                armor: 0.0,
                magic: 15.0,
            },
            keys: 3,
            gold: 4321,
            potions: vec![0, 2, 4],
            powerups: vec![Slot {
                time: 41.5,
                kind: crate::powerups::WEAPON,
                amount: 0.0,
                flags: crate::powerups::W_3WAY,
                state: crate::powerups::ON,
            }],
            inv: MiniInv {
                state: 1,
                sel: -1,
                slide: 128,
            },
            stats: PlayStats {
                enemies_killed: 40,
                generators_destroyed: 2,
                gold_found: 300,
                playtime: 99.5,
            },
            sumner_exp: 7,
            coins: 0,
            class_unlock: 0x10,
        });
        s.progress.beaten[1] = 0x3;
        s.progress.crystals[2] = 5;
        s.progress.last_realm = Some(1);
        s.progress.legend = 0x2;
        s.world.crystals[2] = 5;
        s.world.gone = vec![3, 7, 120];
        s.world.secret_timer = Some((60.0, 12.5));
        s.world.placed.push(Placed {
            inst: 9,
            tag: "enemy".into(),
            pos: Vec3::new(10.0, 0.5, -4.0),
            yaw: 1.0,
            hp: Some(12.0),
            strength: Some(2),
            flag: true,
            idle: None,
        });
        s.world.placed.push(Placed {
            inst: 11,
            tag: "trigger".into(),
            idle: Some(1.5),
            flag: true,
            ..Default::default()
        });
        s.world.loose.push(Loose {
            kind: PickupKind::Powerup {
                kind: 9,
                flags: 0x80,
                amount: 0.0,
                seconds: 45.0,
            },
            model: Some("PHOENIX".into()),
            pos: Vec3::new(1.0, 2.0, 3.0),
            yaw: 0.0,
            radius: 2.5,
        });
        s.world.movers.push(MoverState {
            root: 44,
            active: true,
            offset: 7.25,
            shown: true,
        });
        s.world.anims.push(AnimState {
            obj: 5,
            flags: crate::worldanim::wflags::FORWARD,
            state: 0x101,
            frame: 12.5,
        });
        s.world.rotators.push(RotState {
            target: 6,
            curang: 0.5,
            on: true,
            done: false,
            node: Mat4::from_rotation_y(0.5) * Mat4::from_translation(Vec3::new(1.0, 2.0, 3.0)),
        });
        s
    }

    #[test]
    fn round_trips_through_text() {
        let s = sample();
        let text = s.to_text();
        let back = SaveState::from_text(&text).unwrap();
        // The text is stable, so every field came back.
        assert_eq!(back.to_text(), text);
        assert_eq!(back.world, s.world);
        let (a, b) = (&back.heroes[0], &s.heroes[0]);
        assert_eq!(a.panel.name, b.panel.name);
        assert_eq!(a.panel.hidden, Some(4));
        assert_eq!(
            (a.pos, a.yaw, a.health, a.power),
            (b.pos, b.yaw, b.health, b.power)
        );
        assert_eq!(a.powerups, b.powerups);
        assert_eq!(a.potions, b.potions);
        assert_eq!(a.inv, b.inv);
        assert_eq!(a.stats, b.stats);
        assert_eq!(back.time, s.time);
        assert_eq!(back.progress.beaten, s.progress.beaten);
        assert_eq!(back.progress.last_realm, Some(1));
    }

    #[test]
    fn loads_tolerantly() {
        // Unknown keys and sections, missing fields, bad numbers, a future
        // version: all read, with defaults.
        let text = "\
version 99
level levelB2
shiny_new_thing 1 2 3
[hero]
class 1
health lots
mystery x=1
powerup kind=6 flags=0x200000 newfield=3
[sidequests]
dragon slain
[world]
gone 4 five 6
placed 12 enemy hp=3 future=yes
placed oops enemy
loose kind=weird:1 pos=1,2,3
loose kind=gold:25 pos=1,2,3
mover root=2 active=1
";
        let s = SaveState::from_text(text).unwrap();
        assert_eq!(s.version, 99);
        assert_eq!(s.level, "levelB2");
        assert_eq!(s.heroes.len(), 1);
        let h = &s.heroes[0];
        assert_eq!(h.panel.class, 1);
        assert_eq!(h.health, 0.0);
        assert_eq!(h.level, 1);
        assert_eq!(h.state, "normal");
        assert_eq!(h.powerups.len(), 1);
        assert_eq!(h.powerups[0].flags, 0x200000);
        assert_eq!(s.world.gone, vec![4, 6]);
        assert_eq!(s.world.placed.len(), 1);
        assert_eq!(s.world.placed[0].hp, Some(3.0));
        assert_eq!(s.world.loose.len(), 1);
        assert_eq!(s.world.loose[0].kind, PickupKind::Gold(25));
        assert!(s.world.movers[0].active && s.world.movers[0].shown);
        assert!(SaveState::from_text("# nothing\n").is_err());
    }

    #[test]
    fn slots_and_paths_resolve() {
        let d = Path::new("/data/gauntlet-rs");
        assert_eq!(
            resolve("2", Some(d)),
            PathBuf::from("/data/gauntlet-rs/states/2.state")
        );
        assert_eq!(
            resolve(AUTOSAVE, Some(d)),
            PathBuf::from("/data/gauntlet-rs/states/autosave.state")
        );
        assert_eq!(
            resolve("/tmp/x.state", Some(d)),
            PathBuf::from("/tmp/x.state")
        );
        assert_eq!(resolve("dir/x", Some(d)), PathBuf::from("dir/x"));
    }

    #[test]
    fn names_escape() {
        for n in ["", "A B", "100%", "x=y#z", "plain"] {
            assert_eq!(unesc(&esc(n)), n);
        }
    }

    fn data() -> Option<crate::fs::GameFs> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game-data");
        crate::fs::GameFs::open(&root).ok()
    }

    /// Play a while, save, start the level again in a new game and load:
    /// the heroes, their inventory and the level's changes come back.
    #[test]
    fn a_level_state_comes_back_in_a_fresh_load() {
        let Some(fs) = data() else {
            return;
        };
        let mut g = Game::new(&fs, "levelA1", &[("WAR", "RED")]).unwrap();
        for _ in 0..2400 {
            let inp = g.bot_input(0);
            g.update(1.0 / 60.0, &[inp]);
            g.scroll_box = None;
        }
        // Make sure something changed: take a placed pickup and kill a
        // placed thing, and set off a trigger target.
        let pick = g
            .entities
            .iter()
            .position(|e| matches!(e.kind, Kind::Pickup { .. }) && g.placed.contains_key(&e.id))
            .unwrap();
        g.entities[pick].alive = false;
        g.entities.retain(|e| e.alive);
        if let Some(m) = g.movers.first_mut() {
            m.active = true;
        }
        for _ in 0..120 {
            g.update(1.0 / 60.0, &[PlayerInput::default()]);
        }
        g.players[0].gold = 777;
        g.players[0].keys = 4;
        let s = SaveState::capture(&g, &[], &[], &Progress::default());
        assert!(!s.world.gone.is_empty());
        let text = s.to_text();
        let back = SaveState::from_text(&text).unwrap();
        let g2 = back.load_game(&fs).unwrap();
        let s2 = SaveState::capture(&g2, &[], &[], &Progress::default());
        println!("saved  {}\nloaded {}", s.summary(), s2.summary());
        let (a, b) = (&s.heroes[0], &s2.heroes[0]);
        assert_eq!((b.gold, b.keys, b.level, b.exp), (777, 4, a.level, a.exp));
        assert_eq!(b.potions, a.potions);
        assert!((b.pos - a.pos).length() < 1.0, "{:?} vs {:?}", a.pos, b.pos);
        assert_eq!(s2.world.gone, s.world.gone);
        for (m, n) in s.world.movers.iter().zip(&s2.world.movers) {
            assert_eq!(m.root, n.root);
            assert!((m.offset - n.offset).abs() < 1e-3, "mover {}", m.root);
        }
        assert_eq!(s2.world.loose.len(), s.world.loose.len());
    }
}
