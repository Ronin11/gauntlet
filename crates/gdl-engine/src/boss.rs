//! Boss levels (boss/boss.c, boss/bosscam.c, ui/auxscreen.c): the boss's life
//! cycle, its health meters, the boss camera, the death spews and shard, the
//! legend-weapon sequence, and the good wizard who ends the level.

use crate::character::Model;
use crate::critter::Critter;
use crate::game::{Game, Kind, LevelEnd, PickupKind, PlayerState};
use gdl_formats::wdata::BossCamDef;
use glam::{Vec2, Vec3};
use std::f32::consts::{FRAC_PI_2, PI, TAU};

/// e_e_tpye of the first boss.
pub const BOSS_FIRST: i32 = 34;

/// The boss of type `t` (34 Dragon .. 44 Garm): its CRITTER WAD.
pub fn boss_wad(t: i32) -> Option<&'static str> {
    Some(match t {
        34 => "DRAGON.WAD",
        35 => "CHIMERA.WAD",
        36 => "DJINN.WAD",
        37 => "DRIDER.WAD",
        38 => "PBOSS.WAD",
        39 => "YETI.WAD",
        40 => "WRAITH.WAD",
        41 => "LICH.WAD",
        42 => "SKORNE1.WAD",
        43 => "SKORNE2.WAD",
        44 => "GARM.WAD",
        _ => return None,
    })
}

/// A boss health meter (HealthMeterStart).
#[derive(Clone, Debug)]
pub struct Meter {
    /// The critter part it shows.
    pub part: usize,
    /// Blit name prefix: `<name>_METER_FG<n>` (or `METER_FG<n>` unnamed).
    pub name: String,
    pub max: f32,
    /// The value shown, moving toward the health at 3 a tick.
    pub shown: f32,
    pub value: f32,
    pub pieces: i16,
    /// Insets (meterbordl / meterbordr).
    pub inset: (i16, i16),
    /// Draws the background (typeFlags & 8).
    pub bg: bool,
    pub x: f32,
}

/// The good wizard (good_wiz_state and friends).
#[derive(Clone, Debug)]
pub struct Wizard {
    pub state: u8,
    pub timer: f32,
    /// The wizard's model, placed as a prop entity.
    pub entity: Option<u64>,
    pub pos: Vec3,
    pub yaw: f32,
    /// 255 invisible .. 0 opaque.
    pub alpha: i32,
    /// The player it faces (good_wiz_plyr_attn).
    pub attn: usize,
    /// Caption: ENGLISH message, line, ticks typed, pause ticks.
    pub caption: Option<(usize, usize)>,
    pub speech_frame: u32,
    pub speech_pause: u32,
    /// quality: 0 none .. 3 both of this realm's runestones.
    pub quality: u8,
    pub all_rune_stones: bool,
    /// good_wiz_exit_timer (ticks), and the exit glow once it is low.
    pub exit_timer: i32,
    pub exit_glow: bool,
    pub idle: u32,
}

/// The boss camera's state (gGameCamera fields used by bosscam.c).
#[derive(Clone, Copy, Debug, Default)]
pub struct BossCam {
    pub def: BossCamDef,
    pub started: bool,
    pub attn: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub dist: f32,
    pub t: f32,
    pub vyaw: f32,
    pub vpitch: f32,
    pub vdist: f32,
    pub vattn: Vec3,
    pub zoom_in: bool,
    pub zoom_out: bool,
    pub settling: bool,
}

/// Everything about the level's boss (the boss.c globals).
#[derive(Clone, Debug, Default)]
pub struct Boss {
    /// gBossType (34..44).
    pub boss_type: i32,
    /// The boss critter's entity.
    pub entity: Option<u64>,
    /// gBossPos: where it was placed.
    pub pos: Vec3,
    pub active: bool,
    pub dying: bool,
    pub dead: bool,
    /// add_target done (the camera may aim at it).
    pub targeted: bool,
    /// lbl_8034464C: the wake timer.
    pub wake_time: f32,
    /// lbl_8034489C: 0 none, 1..6 the legend sequence, 99 boss dead.
    pub legend_state: u8,
    pub legend_timer: f32,
    /// The player using the legend weapon (quest_state > 0), and its step.
    pub legend_holder: Option<usize>,
    pub holder_step: u8,
    pub holder_timer: f32,
    pub meters: Vec<Meter>,
    pub meters_hidden: bool,
    /// The BOSSKEY effect: where it is, and when it was started.
    pub key: Option<(Vec3, f32)>,
    pub cam: Option<BossCam>,
    pub wizard: Option<Wizard>,
    /// Coin models (bronze, silver, gold) and Skorne's boss power-ups.
    pub coin_models: [Option<usize>; 3],
    pub key_model: Option<usize>,
}

/// An item flying out of the boss as it dies (gSpewItems).
#[derive(Clone, Copy, Debug)]
pub struct Spew {
    pub id: u64,
    pub vel: Vec3,
    pub landed: bool,
}

/// bossRuneReqTable (0x80121DD8) by realm id: this realm's runestone mask and
/// how many it has.
fn rune_req(realm: u32) -> (u16, u8) {
    match realm {
        1 => (0x1, 1),
        2 => (0x8, 1),
        3 => (0x40, 1),
        4 => (0x200, 1),
        7 => (0x180, 2),
        8 => (0x1000, 1),
        9 => (0x30, 2),
        10 => (0xC00, 2),
        11 => (0x6, 2),
        _ => (0, 0),
    }
}

/// lbl_801189E0: coins (bronze, silver, gold) spewed per realm type.
fn coin_counts(realm: u32) -> [u32; 3] {
    match realm {
        1 => [2, 1, 1],
        2 => [0, 5, 0],
        3 => [2, 1, 2],
        4 => [2, 0, 2],
        7 => [4, 4, 1],
        9 => [0, 3, 2],
        10 => [0, 0, 3],
        11 => [0, 4, 1],
        _ => [0, 0, 0],
    }
}

/// The good wizard's first caption (ENGLISH message) by boss type.
fn first_caption(t: i32) -> Option<usize> {
    Some(match t {
        34 => 148,
        35 => 147,
        36 => 149,
        37 => 150,
        38 => 152,
        39 => 151,
        40 => 154,
        41 => 153,
        42 => 159,
        43 => 162,
        44 => 163,
        _ => return None,
    })
}

/// lbl_8012417C: the wizard's voice clip by boss and column (0 defeat, 1..4
/// the runestone quality + 1).
fn wizard_voice(t: i32, col: usize) -> Option<String> {
    let letter = match t {
        34 => 'B',
        35 => 'A',
        36 => 'C',
        37 => 'D',
        38 => 'K',
        39 => 'I',
        40 => 'J',
        41 => 'G',
        42 => {
            return Some(if col == 0 { "S_E2VOXA" } else { "S_E2VOXB" }.into());
        }
        43 => return (col <= 1).then(|| "S_ENDVOX".into()),
        44 => return (col <= 1).then(|| "S_GRMDESTVOX".into()),
        _ => return None,
    };
    let two = matches!(t, 38..=41);
    Some(match col {
        0 => format!("S_DEFEATVOX{letter}"),
        1 => format!("S_RUNEVOX0{letter}"),
        2 => format!("S_RUNEVOX1{letter}"),
        3 | 4 if two => format!("S_RUNEVOX2{letter}"),
        3 | 4 => format!("S_RUNEVOX1{letter}"),
        _ => return None,
    })
}

/// The realm letter of a boss level, for the legend sounds.
fn boss_letter(t: i32) -> char {
    match t {
        34 => 'B',
        35 => 'A',
        36 => 'C',
        37 => 'D',
        38 => 'K',
        39 => 'I',
        40 => 'J',
        41 => 'G',
        42 => 'E',
        43 => 'F',
        _ => 'H',
    }
}

/// LimitCamVal2 (bosscam.c:1825): move `value` toward `target` with an
/// accelerating, braking velocity limited to [vmin, vmax].
#[allow(clippy::too_many_arguments)]
pub fn limit_cam_val(
    value: f32,
    target: f32,
    vmin: f32,
    vmax: f32,
    acc: f32,
    stop: f32,
    vel: &mut f32,
    wrap: bool,
    dt: f32,
) -> f32 {
    if dt <= 0.0 {
        return value;
    }
    let mut d = target - value;
    if wrap {
        d = wrap_pi(d);
    }
    let a = acc * dt;
    if stop > 0.0 && vel.abs() < dt * stop * vmax && d.abs() < dt * stop * vmax {
        *vel = 0.0;
    } else if d.abs() < dt * (vel.abs() + a) {
        *vel = d / dt;
    } else {
        if vel.abs() > 0.0 && vel.abs() / acc >= d.abs() / vel.abs() {
            // Brake, without passing through zero.
            if *vel > 0.0 {
                *vel = (*vel - a).max(0.0);
            } else {
                *vel = (*vel + a).min(0.0);
            }
        } else {
            *vel += d.signum() * a;
        }
        *vel = vel.clamp(vmin, vmax);
    }
    value + *vel * dt
}

fn wrap_pi(mut a: f32) -> f32 {
    while a > PI {
        a -= TAU;
    }
    while a <= -PI {
        a += TAU;
    }
    a
}

/// The boss camera's view direction for (yaw, pitch): `PitchVec3(YawVec3(z,
/// -yaw), -pitch)`; negative pitch looks down.
pub fn cam_forward(yaw: f32, pitch: f32) -> Vec3 {
    Vec3::new(yaw.sin() * pitch.cos(), pitch.sin(), yaw.cos() * pitch.cos())
}

impl Game {
    /// The boss critter, if it is still there.
    pub fn boss_critter(&self) -> Option<(&crate::game::Entity, &Critter)> {
        let id = self.boss.as_ref()?.entity?;
        self.entities.iter().find_map(|e| match &e.kind {
            Kind::Critter(c) if e.id == id && e.alive => Some((e, &**c)),
            _ => None,
        })
    }

    /// AddBoss: the level's boss at its locator, from CRITTER/<boss>.WAD and
    /// MONSTERS/<boss>.
    pub(crate) fn spawn_boss(&mut self, fs: &crate::fs::GameFs, num_players: usize) {
        let Some(level) = self.wave.and_then(|w| self.worlds.level(w)).cloned() else {
            return;
        };
        let Some(wad) = boss_wad(level.boss) else {
            return;
        };
        use gdl_formats::worlds::LocatorType;
        let Some(loc) = self
            .level
            .worlds
            .locators
            .iter()
            .find(|l| l.kind == LocatorType::Boss)
            .cloned()
        else {
            log::warn!("{}: boss level without a boss locator", self.level.name);
            return;
        };
        let pos = Vec3::from(loc.pos);
        // SetupLocations builds a YPR matrix from the locator: the boss faces
        // along its yaw.
        let yaw = loc.rot[1];
        let mut boss = Boss {
            boss_type: level.boss,
            pos,
            ..Default::default()
        };
        boss.cam = self.worlds.boss_cam(self.wave.unwrap_or(0)).map(|d| {
            let mut def = *d;
            def.cosmaxyaw = def.maxyaw.cos();
            if def.maxpdist == 0.0 {
                def.maxpdist = def.maxdist;
            }
            if def.maxattn[1] >= 999998.0 {
                def.maxattn = def.minattn;
            }
            BossCam {
                def,
                ..Default::default()
            }
        });
        self.boss = Some(boss);
        let id = self.spawn_critter(fs, wad, None, pos, yaw, num_players);
        if let Some(b) = self.boss.as_mut() {
            b.entity = id;
        }
        // The meters: one per part whose type has flags & 4.
        if let Some((_, c)) = self.boss_critter() {
            let mut meters = Vec::new();
            let mut next_x = 0.0;
            let mut x = 0.0;
            for (pi, p) in c.parts.iter().enumerate() {
                let t = &c.wad.types[p.ty];
                if t.flags & 4 == 0 || meters.len() >= 3 {
                    continue;
                }
                let bg = t.flags & 8 != 0;
                if bg {
                    x = next_x;
                    next_x += t.meter[1] as f32;
                }
                meters.push(Meter {
                    part: pi,
                    name: t.suffix.clone(),
                    max: p.health,
                    shown: p.health,
                    value: p.health,
                    pieces: t.meter[0],
                    inset: (t.meter[2], t.meter[3]),
                    bg,
                    x,
                });
            }
            if let Some(b) = self.boss.as_mut() {
                b.meters = meters;
            }
        }
        // The spewed coins, the boss key and the wizard's models.
        let letter = self.world_letter();
        let dirs = [
            format!("ITEMS/{}", self.level.name),
            format!("ITEMS/level{letter}"),
            "POWERUPS".to_string(),
        ];
        let mut cache = Default::default();
        let mut known = Default::default();
        let coins = ["COIN_BRONZE", "COIN_SILVER", "COIN_GOLD"]
            .map(|n| self.item_model_pub(fs, &mut cache, &mut known, &dirs, n));
        let key = self.item_model_pub(fs, &mut cache, &mut known, &dirs, "BOSSKEY");
        for n in ["BGNTR_IC", "BMASK_IC", "BHORN_IC", "BGNTL_IC"] {
            if level.boss == 42 {
                self.item_model_pub(fs, &mut cache, &mut known, &dirs, n);
            }
        }
        self.merge_item_models(known);
        if let Some(b) = self.boss.as_mut() {
            b.coin_models = coins;
            b.key_model = key;
        }
        // A boss level ends through the good wizard only.
        for e in &mut self.entities {
            if let Kind::Exit { open, .. } = &mut e.kind {
                *open = false;
            }
        }
    }

    /// Load and place a critter from `CRITTER/<wad>` (bosses, golems,
    /// generals, gargoyles). `variant` is the ENMY string: a golem's model
    /// directory or a gargoyle's kind.
    pub(crate) fn spawn_critter(
        &mut self,
        fs: &crate::fs::GameFs,
        wad_name: &str,
        variant: Option<&str>,
        pos: Vec3,
        yaw: f32,
        _num_players: usize,
    ) -> Option<u64> {
        let wad = match self.critter_wads.get(wad_name) {
            Some(w) => w.clone(),
            None => {
                let data = fs
                    .read(&format!("CRITTER/{wad_name}"))
                    .or_else(|_| fs.read(&format!("CRITTER/{}", wad_name.to_ascii_uppercase())))
                    .ok()?;
                let w = match gdl_formats::critter::CritterWad::parse(&data) {
                    Ok(w) => std::sync::Arc::new(w),
                    Err(e) => {
                        log::warn!("{wad_name}: {e:#}");
                        return None;
                    }
                };
                self.critter_wads.insert(wad_name.to_string(), w.clone());
                w
            }
        };
        let root = (0..wad.types.len())
            .find(|&t| wad.types[t].parenttype < 0 && wad.types[t].subtype >= 0)
            .unwrap_or(0);
        let desc = &wad.descs[wad.types[root].descidx.max(0) as usize];
        let letter = self.world_letter();
        // GetCritterDesc: golems and generals by world, gargoyles by kind.
        let dir = match desc.etype {
            3 | 8 => {
                let sub = variant
                    .filter(|v| v.starts_with("level"))
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("level{letter}"));
                format!("MONSTERS/{}/{sub}", desc.name.to_ascii_uppercase())
            }
            _ => format!("MONSTERS/{}", desc.name.to_ascii_uppercase()),
        };
        let dirs = [dir.clone(), format!("MONSTERS/{}/levelA", desc.name.to_ascii_uppercase())];
        let (objects, textures, anim) = dirs.iter().find_map(|d| {
            let o = gdl_formats::Objects::parse(&fs.read(&format!("{d}/objects.ngc")).ok()?).ok()?;
            let t = fs.read(&format!("{d}/textures.ngc")).ok()?;
            let a = gdl_formats::anim::Anim::parse(&fs.read(&format!("{d}/ANIM.PS2")).ok()?).ok()?;
            Some((o, t, a))
        })?;
        let tree = format!("{}{}", desc.prefix, wad.types[root].suffix);
        let alts: Vec<(String, String)> = wad
            .type_colnodes(root)
            .iter()
            .filter(|n| n.flags & 2 != 0)
            .map(|n| (n.node.clone(), format!("{}D{}", desc.prefix, n.node)))
            .collect();
        let model = match Model::from_anim_alts(&objects, &textures, &anim, &tree, &alts) {
            Ok(m) => m,
            Err(e) => {
                log::warn!("{wad_name}: {e:#}");
                return None;
            }
        };
        let id = self.peek_next_id();
        let mut c = Critter::new(id, wad.clone(), root, &model, pos, yaw, self.tuning.health);
        c.fx_models = self.critter_fx_models(&wad, &objects, &textures, Some(&anim));
        if desc.etype == 7 {
            c.garg_kind = Some(variant.unwrap_or("eagl").to_string());
        }
        c.visrad = 0.0;
        let midx = self.add_model_pub(model);
        // Placed on the floor (CritterInitGeo).
        let mut p = pos;
        if let Some(g) = self.collision.ground_height(pos, pos.y + 4.0) {
            p.y = g + wad.types[root].y_offset;
        }
        let etype = c.etype;
        let id2 = self.spawn(Kind::Critter(Box::new(c)), Some(midx), p, yaw);
        debug_assert_eq!(id, id2);
        log::info!("critter {wad_name} (etype {etype}) at {p:?}");
        Some(id2)
    }

    /// The party's legend weapons, as the level starts (fn_80057024): on a boss
    /// level (not Skorne 2 or Garm) the first player in play uses this boss's
    /// weapon, if the party carries it.
    pub fn set_legend_held(&mut self, held: u16) {
        self.legend_held = held;
        let realm = self.wave.map_or(0, |w| w >> 8);
        let holder = (0..self.players.len()).find(|&i| self.players[i].state.active());
        if let Some(b) = self.boss.as_mut() {
            if b.boss_type < 43 && held & (1 << realm) != 0 && holder.is_some() && b.legend_state == 0 {
                b.legend_holder = holder;
                b.legend_state = 1;
            }
        }
    }

    /// The boss's model directory (for its health-meter textures).
    pub fn boss_model_dir(&self) -> Option<String> {
        let (_, c) = self.boss_critter()?;
        let d = &c.wad.descs[c.wad.types[c.parts[0].ty].descidx.max(0) as usize];
        Some(format!("MONSTERS/{}", d.name.to_ascii_uppercase()))
    }

    /// BossActivate: the fight starts (the boss becomes a target and the
    /// level music switches to its boss part).
    pub(crate) fn boss_activate(&mut self) {
        if let Some(b) = self.boss.as_mut() {
            if !b.active {
                b.active = true;
                b.targeted = true;
            }
        }
    }

    /// BossDying: every monster dies and generators stop.
    pub(crate) fn boss_dying(&mut self) {
        if let Some(b) = self.boss.as_mut() {
            b.dying = true;
        }
        for e in &mut self.entities {
            if let Kind::Enemy { state, .. } = &mut e.kind {
                if !matches!(state, crate::game::EnemyState::Dying { .. }) {
                    *state = crate::game::EnemyState::Dying { timer: 0.5 };
                }
            }
        }
    }

    /// BossDeath: the good wizard comes, the players get the boss's shard
    /// (rune), and bosses 34-41 leave their key.
    pub(crate) fn boss_death(&mut self, key_pos: Vec3) {
        let realm = self.wave.map_or(0, |w| w >> 8);
        if let Some(bit) = crate::progress::CRYSTAL_ORDER.iter().position(|&r| r == realm) {
            self.boss_runes |= 1 << bit;
        }
        let Some(b) = self.boss.as_mut() else { return };
        b.dead = true;
        b.meters_hidden = true;
        let t = b.boss_type;
        let key_model = b.key_model;
        b.wizard = Some(Wizard {
            state: 1,
            timer: 0.0,
            entity: None,
            pos: Vec3::ZERO,
            yaw: 0.0,
            alpha: 255,
            attn: 0,
            caption: None,
            speech_frame: 0,
            speech_pause: 0,
            quality: 0,
            all_rune_stones: false,
            exit_timer: -1,
            exit_glow: false,
            idle: 0,
        });
        if (34..=41).contains(&t) {
            b.key = Some((key_pos, self.time));
            self.sfx.push(format!("S_BOSSKEY{}", boss_letter(t)));
            if let Some(m) = key_model {
                self.spawn(Kind::Effect { life: 1e9 }, Some(m), key_pos, 0.0);
            }
        }
    }

    /// BossSpewCoins: the death throws out coins (Skorne 1: the four boss
    /// power-ups) over `spread` radians either side of `dir`.
    pub(crate) fn boss_spew_coins(&mut self, pos: Vec3, dir: Vec3, spread: f32) {
        let Some(b) = self.boss.as_ref() else { return };
        let t = b.boss_type;
        let coins = b.coin_models;
        let turn = |v: Vec3, a: f32| {
            let (s, c) = a.sin_cos();
            Vec3::new(v.x * c + v.z * s, v.y, v.z * c - v.x * s)
        };
        self.sfx.push("S_EXPLODE1".into());
        if t == 42 {
            let step = 2.0 * spread * 0.25;
            let mut a = -spread + 0.5 * step;
            for name in ["BGNTR_IC", "BMASK_IC", "BHORN_IC", "BGNTL_IC"] {
                if let Some(info) = self
                    .level
                    .worlds
                    .item_infos
                    .iter()
                    .find(|i| i.name.eq_ignore_ascii_case(name))
                    .cloned()
                {
                    let kind = crate::game::pickup_kind_pub(&info);
                    let model = self.item_model_named(name);
                    let id = self.spawn(Kind::Pickup { kind, radius: 2.0 }, model, pos, 0.0);
                    self.spews.push(Spew {
                        id,
                        vel: turn(dir, a),
                        landed: false,
                    });
                }
                a += step;
            }
            return;
        }
        let realm = self.wave.map_or(0, |w| w >> 8);
        // lbl_8034476C, the coin multiplier, is [INF] the number of players.
        let mul = self.players.len().max(1) as u32;
        for (k, (count, value)) in coin_counts(realm).iter().zip([500u32, 1000, 5000]).enumerate() {
            let n = count * mul;
            if n == 0 {
                continue;
            }
            let step = 2.0 * spread / n as f32;
            let mut a = -spread + 0.5 * step;
            for _ in 0..n {
                if self.spews.len() >= 32 {
                    break;
                }
                // As shipped, the random speed factors are overwritten by the
                // yawed direction.
                let v = turn(dir, a);
                let id = self.spawn(
                    Kind::Pickup {
                        kind: PickupKind::Gold(value),
                        radius: 2.0,
                    },
                    coins[k],
                    pos,
                    0.0,
                );
                self.spews.push(Spew {
                    id,
                    vel: v,
                    landed: false,
                });
                a += step;
            }
        }
    }

    /// ProcessSpewItems: spewed items fly, fall and bounce until picked up.
    pub(crate) fn update_spews(&mut self, dt: f32) {
        let mut keep = Vec::new();
        for mut s in std::mem::take(&mut self.spews) {
            let Some(e) = self.entities.iter_mut().find(|e| e.id == s.id && e.alive) else {
                continue;
            };
            let mut p = e.pos + s.vel * dt;
            let lim;
            match self.collision.ground_height(p, p.y + 2.0) {
                Some(g) if p.y - g < 0.1 => {
                    s.vel.y = if s.vel.y.abs() < 0.1 { 0.0 } else { -0.4 * s.vel.y };
                    p.y = g + 1.0;
                    s.landed = true;
                    lim = 4.0 * dt;
                }
                _ => {
                    s.vel.y -= 32.0 * dt;
                    lim = 0.5 * dt;
                }
            }
            for v in [&mut s.vel.x, &mut s.vel.z] {
                if v.abs() > lim {
                    *v -= lim * *v;
                } else {
                    *v = 0.0;
                }
            }
            if self.collision.raycast(e.pos, p).is_some() {
                s.vel.x = -0.5 * s.vel.x;
                s.vel.z = -0.5 * s.vel.z;
                p = Vec3::new(e.pos.x, p.y, e.pos.z);
            }
            e.pos = p;
            keep.push(s);
        }
        self.spews = keep;
    }

    /// The boss's per-frame bookkeeping: meters, the legend sequence, the
    /// good wizard and the level end.
    pub(crate) fn update_boss(&mut self, dt: f32) {
        if self.boss.is_none() {
            return;
        }
        self.update_meters(dt);
        self.update_legend(dt);
        self.update_wizard(dt);
        self.update_boss_cam(dt);
    }

    fn update_meters(&mut self, dt: f32) {
        let healths: Vec<f32> = match self.boss_critter() {
            Some((_, c)) => c.parts.iter().map(|p| p.health.max(0.0)).collect(),
            None => Vec::new(),
        };
        let Some(b) = self.boss.as_mut() else { return };
        let step = 3.0 * 60.0 * dt;
        for m in &mut b.meters {
            let v = healths.get(m.part).copied().unwrap_or(0.0);
            m.value = v;
            if m.shown > v {
                m.shown = (m.shown - step).max(v);
            } else if m.shown < v {
                m.shown = (m.shown + step).min(v);
            }
        }
    }

    /// The legend-weapon sequence (lbl_8034489C, gauntworld.c:1258-1345 and
    /// pmotion.c:2092-2440), shortened to its effect on the fight.
    fn update_legend(&mut self, dt: f32) {
        let now = self.time;
        let Some(b) = self.boss.as_mut() else { return };
        if b.dead {
            b.legend_state = 99;
            return;
        }
        let t = b.boss_type;
        let letter = boss_letter(t);
        match b.legend_state {
            2 => {
                // The cutscene: everyone faces the boss and waits.
                let wait = if matches!(t, 35 | 41 | 42) { 1.0 } else { 3.0 };
                if b.holder_step == 0 {
                    b.holder_step = 1;
                    // player.c:2508: the weapon is used up.
                    let realm = self.wave.map_or(0, |w| w >> 8);
                    self.legend_held &= !(1 << realm);
                }
                if now - b.legend_timer >= wait {
                    b.legend_state = 3;
                    b.legend_timer = now;
                    b.holder_step = 2;
                    b.holder_timer = now + 1.0;
                    self.sfx.push("S_PICKUPMAGIC".into());
                }
            }
            3 => {
                // The holder throws after its 60-tick wind-up; the hit ends it.
                if b.holder_step == 2 && now >= b.holder_timer {
                    b.holder_step = 3;
                    b.holder_timer = now + 0.5;
                    self.sfx.push(format!("S_{letter}LEGWTHROW"));
                } else if b.holder_step == 3 && now >= b.holder_timer {
                    b.holder_step = 4;
                    self.sfx.push(format!("S_{letter}LEGWFLY"));
                    self.legend_hit();
                }
            }
            4 => {
                b.legend_state = 5;
                b.legend_timer = now;
            }
            5 => {
                // The weakened phase ends after 29 s for these bosses.
                if matches!(t, 36 | 38 | 39 | 40 | 42) && now - b.legend_timer >= 29.0 {
                    b.legend_state = 6;
                    self.sfx.push(format!("S_{letter}LEGWPDN"));
                    if let Some((_, c)) = self.boss_critter_mut() {
                        for p in &mut c.parts {
                            if t != 36 {
                                p.fball_cap = 0.0;
                            }
                            if t == 38 {
                                p.blind = 0.0;
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        let _ = dt;
    }

    /// The legend projectile hits the boss: its damage and weakening (spec
    /// table §5.2; sfx.c's delivery is replaced by an instant hit).
    fn legend_hit(&mut self) {
        let Some(b) = self.boss.as_ref() else { return };
        let t = b.boss_type;
        let holder = b.legend_holder;
        let letter = boss_letter(t);
        let Some(ei) = self.boss.as_ref().and_then(|b| b.entity).and_then(|id| self.entities.iter().position(|e| e.id == id)) else {
            return;
        };
        let Kind::Critter(mut c) = std::mem::replace(&mut self.entities[ei].kind, Kind::Prop) else {
            return;
        };
        let (part, dmg) = match t {
            // gBossObj->next->next: the second head in the chain (LION; the
            // spec's summary says SNAKE, the code says next->next).
            35 if c.parts.len() > 2 => (2, 1.5 * c.parts[2].health),
            40 => (0, 500.0),
            41 => (0, 0.25 * c.parts[0].health),
            _ => (0, 0.1 * c.parts[0].health),
        };
        match t {
            34 => c.parts[0].freeze = 1200.0,
            36 => c.parts[0].blind = 1800.0,
            38 => c.parts[0].blind = 18000.0,
            37 | 39 => c.parts[0].fball_cap = 0.5,
            40 => c.parts[0].fball_cap = 0.25,
            42 => c.parts[0].fball_cap = 0.1,
            _ => {}
        }
        if dmg > 0.0 {
            self.critter_damage(&mut c, part, dmg, holder, 0x200000 | 0x100000, Vec3::ZERO, None);
        }
        self.entities[ei].kind = Kind::Critter(c);
        self.sfx.push(format!("S_{letter}LEGWHIT|S_{letter}LEGWALSTP"));
        if let Some(b) = self.boss.as_mut() {
            if b.legend_state == 3 {
                b.legend_state = 4;
            }
        }
    }

    pub(crate) fn boss_critter_mut(&mut self) -> Option<(usize, &mut Critter)> {
        let id = self.boss.as_ref()?.entity?;
        self.entities.iter_mut().enumerate().find_map(|(i, e)| match &mut e.kind {
            Kind::Critter(c) if e.id == id => Some((i, &mut **c)),
            _ => None,
        })
    }

    /// DoGoodWizard: the wizard appears, speaks, and sends everyone home.
    fn update_wizard(&mut self, dt: f32) {
        let now = self.time;
        let boss_pos = self.boss.as_ref().map_or(Vec3::ZERO, |b| b.pos);
        let realm = self.wave.map_or(0, |w| w >> 8);
        let (mask, n) = rune_req(realm);
        let held = self.shards;
        let quality = if mask & held == mask {
            if n > 1 {
                3
            } else {
                2
            }
        } else if held & mask != 0 {
            1
        } else {
            0
        };
        let all12 = self.shards & 0xFFF == 0xFFF;
        let gold_left = self
            .entities
            .iter()
            .any(|e| e.alive && matches!(e.kind, Kind::Pickup { kind: PickupKind::Gold(_), .. }));
        let active: Vec<usize> = (0..self.players.len()).filter(|&i| self.players[i].state.active()).collect();
        let ppos: Vec<Vec3> = self.players.iter().map(|p| p.pos).collect();
        let Some(b) = self.boss.as_mut() else { return };
        let t = b.boss_type;
        let Some(w) = b.wizard.as_mut() else { return };
        w.quality = quality;
        let ticks = (dt * 60.0).round().max(1.0) as u32;
        let mut voice: Option<String> = None;
        let mut spawn_model = false;
        match w.state {
            1 => {
                w.timer = now + if t == 42 || t == 44 { 10.0 } else { 5.0 };
                w.state = 2;
            }
            2 => {
                if now >= w.timer {
                    w.state = 3;
                }
            }
            3 => {
                // calc_wizard_pos: the mean of the boss and the players, at
                // the players' height.
                w.pos = if t == 42 || t == 43 {
                    Vec3::new(0.0, -12.0, 6.0)
                } else {
                    let mut sum = boss_pos;
                    let mut count = 1.0;
                    for (k, &i) in active.iter().enumerate() {
                        let mut p = ppos[i];
                        if k == 0 {
                            p.y *= 2.0;
                            sum.y = 0.0;
                        }
                        sum += p;
                        count += 1.0;
                    }
                    let mut p = sum / count;
                    p.y += 3.0;
                    p
                };
                w.attn = (realm % 4) as usize;
                w.all_rune_stones = t == 42 && all12;
                w.alpha = 255;
                w.state = 4;
                spawn_model = true;
            }
            4 => {
                // Fade in at 4 a (30 Hz) frame.
                w.alpha = (w.alpha - 2 * ticks as i32).max(0);
                if w.alpha == 0 {
                    w.state = 5;
                    w.caption = first_caption(t).map(|m| (m, 0));
                    w.speech_frame = 0;
                    w.speech_pause = 0;
                    voice = wizard_voice(t, if t >= 42 { w.all_rune_stones as usize } else { 0 });
                }
            }
            5 | 6 => {
                w.speech_frame += ticks;
                if let Some((msg, line)) = w.caption {
                    let lines = self.msgs.text.get(msg).cloned().unwrap_or_default();
                    match lines.get(line) {
                        Some(text) => {
                            if caption_done(text, w.speech_frame >> 1) {
                                w.speech_pause += ticks;
                                if w.speech_pause >= 60 {
                                    w.caption = Some((msg, line + 1));
                                    w.speech_frame = 0;
                                    w.speech_pause = 0;
                                    // calc_good_wiz_attn: face the next player.
                                    if !active.is_empty() {
                                        w.attn = active[(active.iter().position(|&i| i == w.attn).map_or(0, |k| k + 1)) % active.len()];
                                    }
                                }
                            }
                        }
                        None => {
                            // The caption is over: on to the runestones (bosses
                            // before Skorne), Skorne's verdict, or the exit.
                            w.caption = None;
                            w.timer = now + if w.state == 5 { 0.5 } else { 1.0 };
                        }
                    }
                } else if now >= w.timer {
                    if w.state == 5 {
                        w.state = 6;
                        let second = match t {
                            42 => Some(if w.all_rune_stones { 160 } else { 161 }),
                            43 | 44 => None,
                            _ => Some(155 + quality as usize),
                        };
                        if t < 42 {
                            voice = wizard_voice(t, quality as usize + 1);
                        }
                        w.caption = second.map(|m| (m, 0));
                        w.speech_frame = 0;
                        w.speech_pause = 0;
                        if second.is_none() {
                            w.timer = now + 1.0;
                        }
                    } else {
                        w.state = 7;
                    }
                }
            }
            7 => w.state = 8,
            8 => {
                w.exit_timer = if gold_left || w.all_rune_stones { 600 } else { 120 };
                w.state = 9;
            }
            9 => w.state = 10,
            10 => {
                if w.exit_timer <= 35 {
                    w.exit_glow = true;
                    w.state = 11;
                }
            }
            _ => {}
        }
        if w.state >= 9 && !gold_left {
            w.exit_timer = w.exit_timer.min(120);
        }
        if w.state >= 9 {
            if w.exit_timer > 0 {
                w.exit_timer -= ticks as i32;
            } else {
                // lbl_80344808: once everyone has stood idle 6 ticks the level
                // is done; it goes back to the tower through the shop.
                w.idle += ticks;
                if w.idle >= 6 && self.level_end.is_none() {
                    self.level_end = Some(LevelEnd::Exit(None));
                }
            }
        }
        // Turn toward the player it is talking to, at up to pi/2 rad/s.
        if let Some(&p) = ppos.get(w.attn) {
            let d = p - w.pos;
            let want = d.x.atan2(d.z);
            let step = FRAC_PI_2 * dt;
            w.yaw += wrap_pi(want - w.yaw).clamp(-step, step);
        }
        let (wpos, wyaw, went) = (w.pos, w.yaw, w.entity);
        if let Some(v) = voice {
            self.sfx.push(format!("VOICE:{v}"));
        }
        if spawn_model {
            if let Some(m) = self.wizard_model {
                let id = self.spawn(Kind::Prop, Some(m), wpos, wyaw);
                if let Some(w) = self.boss.as_mut().and_then(|b| b.wizard.as_mut()) {
                    w.entity = Some(id);
                }
            }
        } else if let Some(id) = went {
            if let Some(e) = self.entities.iter_mut().find(|e| e.id == id) {
                e.pos = wpos;
                e.yaw = wyaw;
            }
        }
    }

    /// The text of the wizard's caption as far as it has been typed, if any.
    pub fn wizard_caption(&self) -> Option<String> {
        let w = self.boss.as_ref()?.wizard.as_ref()?;
        let (msg, line) = w.caption?;
        let text = self.msgs.text.get(msg)?.get(line)?;
        Some(caption_typed(text, w.speech_frame >> 1))
    }

    /// The camera the boss camera wants this frame: (eye, target, near).
    pub fn boss_camera_view(&self) -> Option<(Vec3, Vec3, f32)> {
        let cam = self.boss.as_ref()?.cam.as_ref()?;
        if !cam.started {
            return None;
        }
        let fwd = cam_forward(cam.yaw, cam.pitch);
        let eye = cam.attn - fwd * cam.dist;
        // Keep the near plane short of the nearest player so nobody is cut away.
        let near = self
            .players
            .iter()
            .filter(|p| p.state.active())
            .map(|p| (p.pos - eye).dot(fwd))
            .fold(cam.dist, f32::min);
        Some((eye, cam.attn, (near - 8.0).clamp(1.0, cam.dist)))
    }

    /// BossCameraUpdate: the player calc until the fight starts, then the
    /// boss calc.
    fn update_boss_cam(&mut self, dt: f32) {
        let Some(b) = self.boss.as_ref() else { return };
        let Some(mut cam) = b.cam else { return };
        let active: Vec<Vec3> = self
            .players
            .iter()
            .filter(|p| p.state.active())
            .map(|p| p.pos)
            .collect();
        let avg = if active.is_empty() {
            self.cam_focus
        } else {
            active.iter().copied().sum::<Vec3>() / active.len() as f32
        };
        let def = cam.def;
        if !cam.started {
            // The first frame: from the normal camera's position.
            let (eye, _, _) = self.camera_view_normal();
            cam.attn = avg;
            let d = cam.attn - eye;
            cam.dist = d.length().max(1.0);
            cam.yaw = d.x.atan2(d.z);
            cam.pitch = (d.y / cam.dist).clamp(-1.0, 1.0).asin();
            cam.started = true;
            cam.settling = true;
        }
        let boss = self.boss_critter().map(|(e, c)| (e.pos, e.yaw, c.init_yaw));
        let wiz = b.wizard.as_ref().filter(|w| w.state >= 4).map(|w| w.pos + Vec3::Y * 10.0);
        let key = b.key.map(|k| k.0);
        let dying = b.dying || b.dead;
        let fight = b.active;
        let (ty, tp, td, focus, focus_rad, attn_target);
        if !fight {
            // BossCamPlayerCalc: the players' average, the level camera's angle.
            attn_target = avg;
            focus = avg;
            focus_rad = 0.0;
            let (yaw, pitch) = match self.camera_locator_pub(avg) {
                Some((y, p)) => (y - PI, -p.clamp(0.7, 1.3)),
                None => (cam.yaw, cam.pitch),
            };
            ty = yaw;
            tp = pitch;
            td = self.zoom_dist(&cam, def.minpdist, 1.9375 * def.maxpdist, false, focus, focus_rad);
            if (td - cam.dist).abs() < 5.0 {
                cam.settling = false;
            }
        } else {
            // BossCamBossCalc.
            let offs = Vec3::from(def.minattn).lerp(Vec3::from(def.maxattn), cam.t);
            let (src, f, r) = if let Some(w) = wiz {
                (w + Vec3::from(def.wizattn) - offs, w, 4.0)
            } else if let Some(k) = key {
                (k + Vec3::from(def.keyattn) - offs, k, 5.0)
            } else if dying {
                (cam.attn - offs, cam.attn, 5.0)
            } else if let Some((bp, _, _)) = boss {
                let src = if def.flags & 0x10 != 0 {
                    // TRACKALL: the centre of the players and the boss.
                    let mut lo = bp;
                    let mut hi = bp;
                    for p in &active {
                        lo = lo.min(*p);
                        hi = hi.max(*p);
                    }
                    (lo + hi) * 0.5
                } else if def.flags & 1 != 0 {
                    bp
                } else {
                    b.pos
                };
                (src, bp, 7.5)
            } else {
                (cam.attn - offs, cam.attn, 5.0)
            };
            attn_target = src + offs;
            focus = f;
            focus_rad = r;
            // The view runs from the players toward the attention point, kept
            // within maxyaw of the boss's front (or back) axis.
            let mut v = Vec2::new(attn_target.x - avg.x, attn_target.z - avg.z);
            if v.length() < 1.0 {
                v = Vec2::new(cam.yaw.sin(), cam.yaw.cos());
            }
            let v = v.normalize();
            let mut yaw = v.x.atan2(v.y);
            if let Some((_, _, iy)) = boss.filter(|_| wiz.is_none() && key.is_none()) {
                let front = wrap_pi(iy + PI);
                let back = iy;
                let axis = if def.flags & 4 != 0 {
                    back
                } else if def.flags & 2 != 0 {
                    front
                } else if wrap_pi(yaw - front).abs() <= wrap_pi(yaw - back).abs() {
                    front
                } else {
                    back
                };
                let d = wrap_pi(yaw - axis).clamp(-def.maxyaw, def.maxyaw);
                yaw = axis + d;
            }
            ty = yaw;
            let maxd = if wiz.is_some() { 2.0 * def.maxdist } else { def.maxdist };
            td = self.zoom_dist(&cam, def.mindist, maxd, true, focus, focus_rad);
            let range = def.maxdist - def.mindist;
            cam.t = if range <= 0.01 {
                1.0
            } else {
                ((cam.dist - def.mindist) / range).clamp(0.0, 1.0)
            };
            // [CHOICE] bosscam.c only retargets the pitch once the distance has
            // settled (|dist - target| < 10); after a jump that can lock the
            // camera steeply above everyone while it zooms, so the pitch
            // always eases toward its band here.
            tp = -(def.minpitch + cam.t * (def.maxpitch - def.minpitch));
        }
        // Attention point: rate-limited per axis (BossCamLimitAttn's speed).
        // A jump (the fight starts, the key or the wizard appears): the
        // attention point travels, and the camera turns in place to follow it.
        let jump = fight && (cam.attn - attn_target).length() > 200.0 * dt;
        let eye = cam.attn - cam_forward(cam.yaw, cam.pitch) * cam.dist;
        for k in 0..3 {
            let mut v = cam.vattn[k];
            cam.attn[k] = limit_cam_val(cam.attn[k], attn_target[k], -200.0, 200.0, 75.0, 0.0, &mut v, false, dt);
            cam.vattn[k] = v;
        }
        let (mut vy, mut vp, mut vd) = (cam.vyaw, cam.vpitch, cam.vdist);
        if jump {
            let d = cam.attn - eye;
            let len = d.length().max(1.0);
            cam.yaw = d.x.atan2(d.z);
            cam.pitch = (d.y / len).clamp(-1.0, 1.0).asin();
            cam.dist = if wiz.is_some() || key.is_some() { len } else { cam.dist.max(len) };
            (vy, vp, vd) = (0.0, 0.0, 0.0);
        } else {
            cam.yaw = wrap_pi(limit_cam_val(cam.yaw, ty, -FRAC_PI_2, FRAC_PI_2, PI / 6.0, 0.1, &mut vy, true, dt));
            cam.pitch = limit_cam_val(cam.pitch, tp, -FRAC_PI_2, FRAC_PI_2, PI / 12.0, 0.1, &mut vp, false, dt);
            cam.dist = limit_cam_val(cam.dist, td, -50.0, 200.0, 75.0, 0.1, &mut vd, false, dt);
        }
        cam.vyaw = vy;
        cam.vpitch = vp;
        cam.vdist = vd;
        cam.pitch = cam.pitch.clamp(-1.4, 0.3);
        if let Some(b) = self.boss.as_mut() {
            b.cam = Some(cam);
        }
        // Controls stay camera-relative: the normal camera follows.
        self.cam_yaw = cam.yaw + PI;
        self.cam_focus = cam.attn;
    }

    /// The zoom rule: back off while someone is near the frame's edge, close
    /// in while everyone is well inside.
    fn zoom_dist(&self, cam: &BossCam, min: f32, max: f32, boss: bool, focus: Vec3, focus_rad: f32) -> f32 {
        let fwd = cam_forward(cam.yaw, cam.pitch);
        let eye = cam.attn - fwd * cam.dist;
        let mut vd = f32::MAX;
        for p in self.players.iter().filter(|p| p.state.active()) {
            vd = vd.min(point_view_dist(eye, fwd, p.pos + Vec3::Y * 2.5, 2.5));
        }
        if boss && focus_rad > 0.0 {
            vd = vd.min(point_view_dist(eye, fwd, focus, focus_rad));
        }
        let mut d = cam.dist;
        let (zin, zout) = if boss { (4.0, 2.5) } else { (2.5, 2.25) };
        if vd < 0.0 {
            d += 10.0;
        } else if vd < 2.0 && d < max {
            d += 2.0 * (2.5 - vd);
        } else if vd < 2.25 && d < max && cam.zoom_out {
            d += 2.0 * (2.5 - vd);
        } else if vd > zin {
            d -= vd - zout;
        } else if vd > zout && cam.zoom_in {
            d -= vd - zout;
        }
        d.clamp(min, 2.0 * max)
    }
}

/// PointViewDist: how far a sphere is inside the view frustum (negative when
/// it pokes out), for the game's 45-degree 4:3 view.
fn point_view_dist(eye: Vec3, fwd: Vec3, p: Vec3, r: f32) -> f32 {
    let right = fwd.cross(Vec3::Y).normalize_or_zero();
    let up = right.cross(fwd).normalize_or_zero();
    let d = p - eye;
    let (x, y, z) = (d.dot(right), d.dot(up), d.dot(fwd));
    let hv = 22.5f32.to_radians();
    let hh = (hv.tan() * 4.0 / 3.0).atan();
    let side = |a: f32, h: f32| z * h.sin() - a.abs() * h.cos();
    let near = z - 1.0;
    near.min(side(x, hh)).min(side(y, hv)) - r
}

/// CaptionTextSub's typewriter: the text revealed after `budget` (ticks / 2).
pub fn caption_typed(text: &str, budget: u32) -> String {
    let mut spent = 0.0f32;
    let mut out = String::new();
    for ch in text.chars() {
        let cost = match ch {
            ',' | '.' => 3.75,
            '\t' => 5.0,
            '\r' => 30.0,
            _ => 1.75,
        };
        spent += cost;
        if spent > budget as f32 {
            break;
        }
        match ch {
            '\t' => {}
            '\r' => out.clear(),
            c => out.push(c),
        }
    }
    out
}

/// Whether the whole line has been typed.
pub fn caption_done(text: &str, budget: u32) -> bool {
    let total: f32 = text
        .chars()
        .map(|ch| match ch {
            ',' | '.' => 3.75,
            '\t' => 5.0,
            '\r' => 30.0,
            _ => 1.75,
        })
        .sum();
    budget as f32 >= total
}

impl Game {
    /// Players stand still facing the boss during the legend cutscene
    /// (pmotion.c:988).
    pub(crate) fn legend_freeze_inputs(&self, inputs: &[crate::input::PlayerInput]) -> Vec<crate::input::PlayerInput> {
        if (2..=3).contains(&self.legend_state()) {
            vec![crate::input::PlayerInput::default(); inputs.len()]
        } else {
            inputs.to_vec()
        }
    }

    pub(crate) fn legend_face_boss(&mut self) {
        if !(2..=3).contains(&self.legend_state()) {
            return;
        }
        let Some(bp) = self.boss_critter().map(|(e, _)| e.pos) else { return };
        for p in &mut self.players {
            if matches!(p.state, PlayerState::Normal) {
                let d = bp - p.pos;
                p.yaw = d.x.atan2(d.z);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::GameFs;

    fn data() -> Option<GameFs> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game-data");
        GameFs::open(&root).ok()
    }

    pub(crate) fn boss_state(g: &Game) -> String {
        let b = g.boss.as_ref().unwrap();
        let c = g.boss_critter().map(|(e, c)| {
            let p = &c.parts[0];
            let m = p
                .moves(&c.wad)
                .get(p.cur.max(0) as usize)
                .map_or("-", |m| m.name.as_str())
                .to_string();
            format!(
                "pos {:?} yaw {:.2} hp {:.0} state {} move {m} tgts {}",
                e.pos,
                e.yaw,
                p.health,
                p.state,
                p.targets.len()
            )
        });
        format!(
            "active {} dying {} dead {} cam {:?} | {c:?}",
            b.active,
            b.dying,
            b.dead,
            b.cam.map(|c| (c.started, c.dist))
        )
    }

    /// Put the party in front of the boss, on the floor.
    fn face_boss(g: &mut Game, dist: f32) {
        let Some((bp, yaw)) = g.boss_critter().map(|(e, c)| (e.pos, c.init_yaw)) else {
            return;
        };
        for d in [dist, dist * 0.6, dist * 1.5, dist * 0.3] {
            let p = bp + Vec3::new(yaw.sin(), 0.0, yaw.cos()) * d;
            if let Some(gy) = g.collision.ground_height(p, bp.y + 30.0) {
                for (i, pl) in g.players.iter_mut().enumerate() {
                    pl.pos = Vec3::new(p.x + i as f32 * 2.5, gy, p.z);
                }
                g.cam_focus = p;
                return;
            }
        }
    }

    /// Every boss level spawns its boss, which wakes up, fights, and moves
    /// through its moves; the run is printed with --nocapture.
    #[test]
    fn every_boss_wakes_and_fights() {
        let Some(fs) = data() else { return };
        for (lvl, ty) in [
            ("A5", 35),
            ("B6", 34),
            ("C5", 36),
            ("D5", 37),
            ("E2", 42),
            ("F2", 43),
            ("G5", 41),
            ("H4", 44),
            ("I5", 39),
            ("J5", 40),
            ("K5", 38),
        ] {
            let mut g = Game::new(&fs, &format!("level{lvl}"), &[("WAR", "RED")]).unwrap();
            assert_eq!(g.boss.as_ref().map(|b| b.boss_type), Some(ty), "{lvl}");
            assert!(g.boss_critter().is_some(), "{lvl}: no boss critter");
            assert!(!g.boss.as_ref().unwrap().meters.is_empty(), "{lvl}: no meters");
            let act = g.boss_critter().unwrap().1.wad.types[0].activedist;
            face_boss(&mut g, if act > 0.0 { act * 0.5 } else { 25.0 });
            let mut moves = std::collections::BTreeSet::new();
            let hp0 = g.players[0].health;
            let mut hurt = 0.0f32;
            for _ in 0..1800 {
                // Keep the hero alive and in place: only the boss acts.
                g.players[0].health = hp0;
                let before = g.players[0].health;
                g.update(1.0 / 60.0, &[crate::input::PlayerInput::default()]);
                hurt += (before - g.players[0].health).max(0.0);
                if let Some((_, c)) = g.boss_critter() {
                    let p = &c.parts[0];
                    if let Some(m) = p.moves(&c.wad).get(p.cur.max(0) as usize) {
                        moves.insert(m.name.clone());
                    }
                }
            }
            eprintln!(
                "{lvl}: {} | player {:?} took {hurt:.0} | moves {moves:?}",
                boss_state(&g),
                g.players[0].pos
            );
            assert!(g.boss.as_ref().unwrap().active, "{lvl}: never woke");
            assert!(moves.len() >= 3, "{lvl}: only {moves:?}");
        }
    }

    fn wake(g: &mut Game) {
        for _ in 0..1200 {
            if g.boss.as_ref().is_some_and(|b| b.active) {
                return;
            }
            g.update(1.0 / 60.0, &[crate::input::PlayerInput::default()]);
        }
        panic!("the boss never woke");
    }

    fn boss_index(g: &Game) -> usize {
        let id = g.boss.as_ref().unwrap().entity.unwrap();
        g.entities.iter().position(|e| e.id == id).unwrap()
    }

    fn boss_health(g: &Game) -> f32 {
        g.boss_critter().map_or(0.0, |(_, c)| c.parts[0].health)
    }

    /// Hits take health off (armour first, x1 for one player), a dead boss
    /// spews its coins, gives the rune, and the good wizard ends the level.
    #[test]
    fn dragon_dies_and_the_wizard_ends_the_level() {
        let Some(fs) = data() else { return };
        let mut g = Game::new(&fs, "levelB6", &[("WAR", "RED")]).unwrap();
        face_boss(&mut g, 25.0);
        let t = boss_index(&g);
        // Asleep, it can't be hurt.
        let hp0 = boss_health(&g);
        g.hit_entity(0, t, 100.0, Vec2::ZERO, true);
        assert_eq!(boss_health(&g), hp0);
        wake(&mut g);
        let exp0 = g.players[0].exp;
        let t = boss_index(&g);
        g.hit_entity(0, t, 101.0, Vec2::ZERO, true);
        // Dragon armour 1: 100 off its 2000.
        assert!((hp0 - boss_health(&g) - 100.0).abs() < 1e-3, "{}", boss_health(&g));
        assert!(g.players[0].exp > exp0, "exp for the hit");
        // Two players: a boss takes half of every hit.
        while boss_health(&g) > 0.0 {
            let t = boss_index(&g);
            g.hit_entity(0, t, 500.0, Vec2::ZERO, true);
        }
        assert!(g.boss.as_ref().unwrap().dying);
        let mut coins = 0;
        let mut caption = false;
        for _ in 0..6000 {
            g.players[0].health = 500.0;
            g.update(1.0 / 60.0, &[crate::input::PlayerInput::default()]);
            coins = coins.max(
                g.entities
                    .iter()
                    .filter(|e| matches!(e.kind, Kind::Pickup { kind: PickupKind::Gold(_), .. }))
                    .count(),
            );
            if let Some(c) = g.wizard_caption() {
                caption |= c.contains("Dragon");
            }
            if g.level_end.is_some() {
                break;
            }
        }
        assert!(g.boss_critter().is_none(), "the boss is gone");
        assert_eq!(coins, 5, "the mountain's five silver coins");
        assert!(caption, "the wizard's DRAGON_SPEECH");
        assert_eq!(g.level_end, Some(LevelEnd::Exit(None)));
        // The mountain is crystal_order position 2 (world order: Mountain Kingdom).
        assert_eq!(g.boss_runes, 1 << 2);
    }

    /// The Chimera's heads have their own health: a head hit also hurts the
    /// body, a body hit is shared among the heads, and the legend weapon
    /// (the Scimitar) kills a head outright.
    #[test]
    fn chimera_heads_and_the_scimitar() {
        let Some(fs) = data() else { return };
        let mut g = Game::new(&fs, "levelA5", &[("WAR", "RED")]).unwrap();
        g.set_legend_held(1 << 1);
        assert_eq!(g.boss.as_ref().unwrap().legend_state, 1);
        face_boss(&mut g, 25.0);
        wake(&mut g);
        let (heads, root) = {
            let (_, c) = g.boss_critter().unwrap();
            (c.parts.len() - 1, c.parts[0].health)
        };
        assert_eq!(heads, 3);
        // A hit landing on the eagle's head.
        let at = {
            let (_, c) = g.boss_critter().unwrap();
            c.colliders().into_iter().find(|x| x.0 == 1).unwrap().2
        };
        g.hit_from = Some(at);
        let t = boss_index(&g);
        {
            let (e, c) = g.boss_critter().unwrap();
            eprintln!("at {at:?} surface {:?} colliders {:?}", c.surface(at, e.pos, e.yaw), c.colliders());
        }
        g.hit_entity(0, t, 102.0, Vec2::ZERO, false);
        let (_, c) = g.boss_critter().unwrap();
        // During the legend sequence (states 1-4) there is no player scaling;
        // armour 2.
        assert!((c.parts[1].health - (900.0 - 100.0)).abs() < 1e-2, "{}", c.parts[1].health);
        assert!((c.parts[0].health - (root - 100.0)).abs() < 1e-2);
        // The legend sequence: the START ends the intro, the cutscene, the
        // throw, and the hit kills the second head.
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..1800 {
            g.update(1.0 / 60.0, &[crate::input::PlayerInput::default()]);
            seen.insert(g.legend_state());
        }
        assert!(seen.contains(&2) && seen.contains(&3) && seen.contains(&5), "{seen:?}");
        let (_, c) = g.boss_critter().unwrap();
        assert!(c.parts[2].health <= 0.0, "the lion's head: {}", c.parts[2].health);
        assert_eq!(g.legend_held & 2, 0, "the scimitar is used up");
    }
}
