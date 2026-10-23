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
    /// The boss critter's floor-snapped spawn position (crit_inst+0x418,
    /// set from the matrix translation in CritterInitInst): the camera's
    /// attention source unless it tracks the boss.
    pub home: Vec3,
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
    /// The holder's forced throw fired its act bits (pmotion.c:2203).
    pub holder_fired: bool,
    pub meters: Vec<Meter>,
    pub meters_hidden: bool,
    /// The BOSSKEY effect: where it is, and when it was started.
    pub key: Option<(Vec3, f32)>,
    pub cam: Option<BossCam>,
    pub wizard: Option<Wizard>,
    /// Coin models (bronze, silver, gold) and Skorne's boss power-ups.
    pub coin_models: [Option<usize>; 3],
    pub key_model: Option<usize>,
    /// The Dragon's SEETHROUGH texture (ITEMS, sItemFile1), shown while the
    /// legend weapon holds him (pausecnt).
    pub seethrough: Option<gdl_formats::Image>,
    /// BOSSKEY2, and the BOSSKEY effect waiting to change into it.
    pub key2_model: Option<usize>,
    pub key_effect: Option<u64>,
    /// Safe rocks (CollectSafeRocks): None until collected on the first boss
    /// frame; empty when the level has none.
    pub safe_rocks: Option<Vec<SafeRock>>,
    /// SafeRockIdx: the last rock chosen.
    pub rock_idx: i32,
    /// sSafeRockCount: Skorne's stages (falling floor pieces).
    pub rock_stage: u32,
    /// Floor pieces flying off (subtype 0x33 obstacles).
    pub fallers: Vec<Faller>,
    /// Players' positions after last frame's camera limit (CameraLimitPlayerDpos).
    pub player_prev: Vec<Option<Vec3>>,
    /// The legend weapon's held model (FX 92, LEGENDHLD) and projectile
    /// (FX 93, LEGENDPRJ) from the level's item file, and their entities.
    pub legend_models: (Option<usize>, Option<usize>),
    /// LEGENDFX and LEGENDFX2 (FX_LEGEND1 / FX_LEGEND2).
    pub legend_fx: (Option<usize>, Option<usize>),
    /// Effects to change into another when they end (SfxSetMorph): the
    /// effect, where it is, the model it becomes and for how long (s).
    pub morphs: Vec<(u64, Vec3, Option<usize>, f32)>,
    /// The LEGENDFX riding on the Djinn (lbl_80344890), and until when.
    pub legend_aura: Option<(u64, f32)>,
    pub legend_hold: Option<u64>,
    pub legend_shot: Option<u64>,
    /// The safe rock's model (SAFEROCK0 in the realm's item file).
    pub safe_rock_model: Option<usize>,
}

/// A safe rock (obstacle subtype 0x29): an eruption spot, and cover once it
/// has risen.
#[derive(Clone, Debug)]
pub struct SafeRock {
    pub pos: Vec3,
    pub yaw: f32,
    pub model: Option<usize>,
    pub solid: Option<crate::game::Shape>,
    pub hit_points: f32,
    pub armor: f32,
    /// The entity while it is shown.
    pub entity: Option<u64>,
    /// data.obsticle.strength: -1 hidden, 0 risen; > 0 counts as active.
    pub strength: i32,
    /// SafeRockActiveCounter: seconds until it rises.
    pub counter: f32,
}

/// A floor piece thrown up by Skorne's stages (items.c case 0x33).
#[derive(Clone, Copy, Debug)]
pub struct Faller {
    pub id: u64,
    pub vel: Vec3,
    /// Tumble rates (rad a 30 Hz frame) from lbl_8011C904.
    pub spin: (f32, f32),
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
    Vec3::new(
        yaw.sin() * pitch.cos(),
        pitch.sin(),
        yaw.cos() * pitch.cos(),
    )
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
        // Debugging aid: GDL_NO_BOSS_CAMERA keeps the level camera.
        if std::env::var_os("GDL_NO_BOSS_CAMERA").is_some() {
            boss.cam = None;
        }
        self.boss = Some(boss);
        let id = self.spawn_critter(fs, wad, None, pos, yaw, num_players);
        let home = self.boss_critter().map_or(pos, |(e, _)| e.pos);
        if let Some(b) = self.boss.as_mut() {
            b.entity = id;
            b.home = home;
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
        if level.boss == 34 {
            let dir = format!("ITEMS/{}", self.level.name);
            let img = (|| {
                let objs = gdl_formats::objects::Objects::parse(
                    &fs.read(&format!("{dir}/objects.ngc")).ok()?,
                )
                .ok()?;
                let tex = fs.read(&format!("{dir}/textures.ngc")).ok()?;
                let def = objs.bitmap_defs.iter().find(|d| d.name == "SEETHROUGH")?;
                let bm = objs.bitmaps.get(def.tex_index as usize)?;
                gdl_formats::texture::decode_ngc(bm, &tex).ok()
            })();
            if let Some(b) = self.boss.as_mut() {
                b.seethrough = img;
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
        let key2 = self.item_model_pub(fs, &mut cache, &mut known, &dirs, "BOSSKEY2");
        // [CHOICE] The SAFEROCK item has models SAFEROCK0..3 (by damage, we
        // take it); the undamaged one is used.
        let safe_rock_model = self
            .item_model_pub(fs, &mut cache, &mut known, &dirs, "SAFEROCK0")
            .or_else(|| self.item_model_pub(fs, &mut cache, &mut known, &dirs, "SAFEROCK"));
        let legend_models = (
            self.item_model_pub(fs, &mut cache, &mut known, &dirs, "LEGENDHLD"),
            self.item_model_pub(fs, &mut cache, &mut known, &dirs, "LEGENDPRJ"),
        );
        let legend_fx = (
            self.item_model_pub(fs, &mut cache, &mut known, &dirs, "LEGENDFX"),
            self.item_model_pub(fs, &mut cache, &mut known, &dirs, "LEGENDFX2"),
        );
        for n in ["BGNTR_IC", "BMASK_IC", "BHORN_IC", "BGNTL_IC"] {
            if level.boss == 42 {
                self.item_model_pub(fs, &mut cache, &mut known, &dirs, n);
            }
        }
        // Garm's escorts.
        if level.boss == 44 {
            let mut mcache = Default::default();
            self.monster_class(fs, &mut mcache, 27);
        }
        self.merge_item_models(known);
        if let Some(b) = self.boss.as_mut() {
            b.coin_models = coins;
            b.key_model = key;
            b.key2_model = key2;
            b.legend_models = legend_models;
            b.legend_fx = legend_fx;
            b.safe_rock_model = safe_rock_model;
        }
        // Skorne's altar: the floor pieces his stages throw up (obstacles of
        // subtype 0x33 whose model is the level object named by the instance).
        if level.boss == 42 {
            self.place_altar_pieces();
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
            // Gargoyles: monsters/<name>_<kind> (GAR_EAGL, GAR_LION, GAR_SERP).
            7 => format!(
                "MONSTERS/{}_{}",
                desc.name.to_ascii_uppercase(),
                variant.unwrap_or("eagl").to_ascii_uppercase()
            ),
            _ => format!("MONSTERS/{}", desc.name.to_ascii_uppercase()),
        };
        let dirs = [
            dir.clone(),
            format!("MONSTERS/{}/levelA", desc.name.to_ascii_uppercase()),
        ];
        let (objects, textures, anim) = dirs.iter().find_map(|d| {
            let o =
                gdl_formats::Objects::parse(&fs.read(&format!("{d}/objects.ngc")).ok()?).ok()?;
            let t = fs.read(&format!("{d}/textures.ngc")).ok()?;
            let a =
                gdl_formats::anim::Anim::parse(&fs.read(&format!("{d}/ANIM.PS2")).ok()?).ok()?;
            Some((o, t, a))
        })?;
        let tree = format!("{}{}", desc.prefix, wad.types[root].suffix);
        let mut alts: Vec<(String, String)> = wad
            .type_colnodes(root)
            .iter()
            .filter(|n| n.flags & 2 != 0)
            .map(|n| (n.node.clone(), format!("{}D{}", desc.prefix, n.node)))
            .collect();
        if desc.etype == 4 && desc.prefix.eq_ignore_ascii_case("PBOSS") {
            alts.push(("EYEBALL".into(), "PBOSSQEYEBALL".into()));
        }
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
        c.model = Some(midx);
        // typeFlags 1: a SHADOW<n>L1 on the floor under it (CritterInitGeo).
        if wad.types[root].flags & 1 != 0 {
            let name = crate::shadow::critter_shadow_name(wad.types[root].subtype);
            if let Some(m) = crate::shadow::load(&objects, &textures, name) {
                let s = self.add_model_pub(m);
                self.shadow_models.insert(midx, s);
            }
        }
        // typeFlags 0x800: the GMETER bar over its head (CritterAddHealthMeter).
        if wad.types[root].flags & 0x800 != 0 {
            if let Ok(m) = Model::from_anim(&objects, &textures, &anim, Some("GMETER")) {
                if let Some(node) = m.node_index("RED_FILLE") {
                    c.gmeter = Some((self.add_model_pub(m), node));
                }
            }
        }
        // Placed on the floor (CritterInitGeo, the same probe as every frame).
        let mut p = pos;
        if let Some(g) = self.critter_floor(pos) {
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
            if b.boss_type < 43
                && held & (1 << realm) != 0
                && holder.is_some()
                && b.legend_state == 0
            {
                b.legend_holder = holder;
                b.legend_state = 1;
            }
        }
    }

    /// Testing aid: the boss wakes (if asleep) and takes a fatal hit.
    pub fn kill_boss(&mut self) {
        if !self.boss.as_ref().is_some_and(|b| b.active) {
            self.boss_activate(true);
        }
        let Some((ei, _)) = self.boss_critter_mut() else {
            return;
        };
        let Kind::Critter(mut c) = std::mem::replace(&mut self.entities[ei].kind, Kind::Prop)
        else {
            return;
        };
        for p in &mut c.parts {
            if p.state == 0 {
                p.state = 3;
            }
        }
        let hp = c.parts[0].health.max(1.0);
        self.critter_damage(
            &mut c,
            0,
            hp * 100.0 + 1000.0,
            None,
            crate::critter::DMG_SUPER,
            Vec3::ZERO,
            None,
        );
        self.entities[ei].kind = Kind::Critter(c);
    }

    /// The boss's model directory (for its health-meter textures).
    pub fn boss_model_dir(&self) -> Option<String> {
        let (_, c) = self.boss_critter()?;
        let d = &c.wad.descs[c.wad.types[c.parts[0].ty].descidx.max(0) as usize];
        Some(format!("MONSTERS/{}", d.name.to_ascii_uppercase()))
    }

    /// BossGenerateEnemy: Garm's shot landed; two GARM2 (strength 3) come
    /// out of it, 3 units out at +-45 degrees, facing away.
    pub(crate) fn boss_generate_enemy(&mut self, pos: Vec3, dir: Vec3) {
        if self
            .boss
            .as_ref()
            .is_none_or(|b| b.boss_type != 44 || b.dying)
        {
            return;
        }
        let Some(class) = self.classes.iter().position(|c| c.etype == 27) else {
            return;
        };
        let yaw = if dir.x.abs() + dir.z.abs() > 1e-3 {
            dir.x.atan2(dir.z)
        } else {
            0.0
        };
        for a in [-FRAC_PI_2 * 0.5, FRAC_PI_2 * 0.5] {
            let y = yaw + a;
            let mut p = pos + Vec3::new(y.sin(), 0.0, y.cos()) * 3.0;
            if let Some(g) = self.collision.ground_height(p, p.y + 4.0) {
                p.y = g;
            }
            let model = self.classes[class].model(3);
            let mut kind = self.new_enemy(class, 3, crate::game::EnemyState::Chase);
            if let Kind::Enemy { recognized, .. } = &mut kind {
                *recognized = true;
            }
            self.spawn(kind, model, p, y);
        }
    }

    /// CritterProcessSafeRocks: collect the rocks once (hiding them when the
    /// boss has a SPOUT2), then count down the risen-rock timers.
    pub(crate) fn process_safe_rocks(&mut self, hide: bool, dt: f32) {
        if self.boss.as_ref().is_some_and(|b| b.safe_rocks.is_none()) {
            let rocks = self.collect_safe_rocks(hide);
            let n = rocks.len();
            let pick = if n > 0 {
                (self.rng.next_u32() as usize % n) as i32
            } else {
                -1
            };
            if let Some(b) = self.boss.as_mut() {
                b.safe_rocks = Some(rocks);
                b.rock_idx = pick;
            }
            return;
        }
        let mut rise = Vec::new();
        if let Some(rocks) = self.boss.as_mut().and_then(|b| b.safe_rocks.as_mut()) {
            for (i, r) in rocks.iter_mut().enumerate() {
                if r.counter > 0.0 {
                    r.counter -= dt;
                    if r.counter <= 0.0 {
                        rise.push(i);
                    }
                }
            }
        }
        for i in rise {
            self.safe_rock_activate(i);
        }
    }

    /// CollectSafeRocks: the level's obstacle items of subtype 0x29 (at most
    /// 16), hidden (strength -1) when `hide`.
    fn collect_safe_rocks(&mut self, hide: bool) -> Vec<SafeRock> {
        use gdl_formats::worlds::ItemType;
        let np = self.players.len().max(1);
        let mut out = Vec::new();
        let insts = self.level.worlds.item_instances.clone();
        for inst in &insts {
            let Some(info) = self.level.worlds.item_infos.get(inst.item_index as usize) else {
                continue;
            };
            if info.item_type != ItemType::Obstacle || info.subtype != 0x29 {
                continue;
            }
            if inst.min_players as usize > np {
                continue;
            }
            let pos = Vec3::from(inst.pos);
            let fwd = crate::character::ryp_matrix(inst.rot).transform_vector3(Vec3::Z);
            let yaw = fwd.x.atan2(fwd.z);
            let found = self
                .entities
                .iter_mut()
                .find(|e| {
                    e.alive
                        && matches!(e.kind, Kind::Container { chest: false, .. })
                        && (e.pos - pos).length() < 0.05
                })
                .map(|e| {
                    if hide {
                        e.alive = false;
                    }
                    (e.id, e.model, e.solid)
                });
            let model = found
                .and_then(|f| f.1)
                .or(self.boss.as_ref().and_then(|b| b.safe_rock_model));
            let solid = found
                .and_then(|f| f.2)
                .or(Some(crate::game::Shape::Cylinder {
                    radius: info.radius.clamp(0.4, 6.0),
                }));
            let mut r = SafeRock {
                pos,
                yaw,
                model,
                solid,
                hit_points: info.hit_points.max(1) as f32,
                armor: info.armor.max(0) as f32,
                entity: found.map(|f| f.0),
                strength: 1,
                counter: 0.0,
            };
            if hide {
                r.entity = None;
                r.strength = -1;
            } else if r.entity.is_none() {
                // Shown from the start (no SPOUT2): place it.
                let id = self.spawn(
                    Kind::Container {
                        hp: r.hit_points,
                        drop: None,
                        chest: false,
                        armor: r.armor,
                        sub: 0,
                    },
                    model,
                    pos,
                    yaw,
                );
                if let Some(e) = self.entities.iter_mut().find(|e| e.id == id) {
                    e.solid = solid;
                }
                r.entity = Some(id);
            }
            out.push(r);
            if out.len() >= 16 {
                break;
            }
        }
        out
    }

    /// Skorne's floor pieces as props, from the level's own objects.
    fn place_altar_pieces(&mut self) {
        use gdl_formats::worlds::ItemType;
        let insts = self.level.worlds.item_instances.clone();
        let mut made: std::collections::HashMap<String, Option<usize>> = Default::default();
        for inst in &insts {
            let Some(info) = self.level.worlds.item_infos.get(inst.item_index as usize) else {
                continue;
            };
            if info.item_type != ItemType::Obstacle || info.subtype != 0x33 || inst.name.is_empty()
            {
                continue;
            }
            let model = match made.get(&inst.name) {
                Some(m) => *m,
                None => {
                    let m = Model::from_objects(
                        &self.level.objects,
                        &self.level.textures_file,
                        &[&inst.name],
                    )
                    .map(|m| self.add_model_pub(m));
                    made.insert(inst.name.clone(), m);
                    m
                }
            };
            if model.is_some() {
                self.spawn(Kind::Prop, model, Vec3::from(inst.pos), 0.0);
            }
        }
    }

    /// SafeRockActive: shown, unbroken and armed.
    fn safe_rock_active(&self, i: usize) -> bool {
        let Some(r) = self
            .boss
            .as_ref()
            .and_then(|b| b.safe_rocks.as_ref())
            .and_then(|v| v.get(i))
        else {
            return false;
        };
        let hp = r.entity.and_then(|id| {
            self.entities
                .iter()
                .find(|e| e.id == id && e.alive)
                .and_then(|e| match e.kind {
                    Kind::Container { hp, .. } => Some(hp),
                    _ => None,
                })
        });
        hp.is_some_and(|h| h > 0.0) && r.strength > 0
    }

    /// SafeRockActivate: the rock is back, with 3x its hit points.
    fn safe_rock_activate(&mut self, i: usize) {
        let Some(r) = self
            .boss
            .as_ref()
            .and_then(|b| b.safe_rocks.as_ref())
            .and_then(|v| v.get(i))
            .cloned()
        else {
            return;
        };
        if let Some(id) = r.entity {
            if let Some(e) = self.entities.iter_mut().find(|e| e.id == id) {
                e.alive = false;
            }
        }
        let id = self.spawn(
            Kind::Container {
                hp: r.hit_points * 3.0,
                drop: None,
                chest: false,
                armor: r.armor,
                sub: 0,
            },
            r.model,
            r.pos,
            r.yaw,
        );
        if let Some(e) = self.entities.iter_mut().find(|e| e.id == id) {
            e.solid = r.solid;
        }
        if let Some(rr) = self
            .boss
            .as_mut()
            .and_then(|b| b.safe_rocks.as_mut())
            .and_then(|v| v.get_mut(i))
        {
            rr.entity = Some(id);
            rr.strength = 0;
        }
    }

    /// Where every collected safe rock is (SPOUT).
    pub(crate) fn safe_rock_positions(&self) -> Vec<Vec3> {
        self.boss
            .as_ref()
            .and_then(|b| b.safe_rocks.as_ref())
            .map(|v| v.iter().map(|r| r.pos).collect())
            .unwrap_or_default()
    }

    /// SPOUT2: SafeRockNearestTarget picks an inactive rock (nearest the
    /// target player, or the next one round from SafeRockIdx without one);
    /// it is busy for the effect's length, then rises. Its position, if any.
    pub(crate) fn safe_rock_spout2(&mut self, target: i32, fx_secs: f32) -> Option<Vec3> {
        let n = self.boss.as_ref()?.safe_rocks.as_ref()?.len();
        if n == 0 {
            return None;
        }
        let idx = self.boss.as_ref()?.rock_idx;
        let mut pick = -1i32;
        if target < 0 {
            for i in 0..n {
                let k = ((idx + i as i32 + 1).rem_euclid(n as i32)) as usize;
                // As shipped: the test is on rock i, the answer rock k.
                if !self.safe_rock_active(i) {
                    pick = k as i32;
                    break;
                }
            }
        } else {
            let p = self.players.get(target as usize)?.pos;
            let mut best = 1e21f32;
            let rocks = self.boss.as_ref()?.safe_rocks.clone()?;
            for (i, r) in rocks.iter().enumerate() {
                if self.safe_rock_active(i) {
                    continue;
                }
                let d = Vec2::new(r.pos.x - p.x, r.pos.z - p.z).length();
                if d < best {
                    best = d;
                    pick = i as i32;
                }
            }
        }
        let b = self.boss.as_mut()?;
        b.rock_idx = pick;
        let r = b
            .safe_rocks
            .as_mut()?
            .get_mut(usize::try_from(pick).ok()?)?;
        r.counter = fx_secs;
        Some(r.pos)
    }

    /// SafeRockSetup (an SFXX with flag 0x20): the Lich's dirt mound and
    /// Garm's purple glow go; Skorne's stage advances, throwing up the next
    /// floor pieces.
    pub(crate) fn safe_rock_setup(&mut self) {
        let Some(t) = self.boss.as_ref().map(|b| b.boss_type) else {
            return;
        };
        match t {
            41 => {
                self.hide_object("G5BIGDIRT");
            }
            44 => {
                self.hide_object("H4NSFFXL_PURPLE");
            }
            42 => {
                let stage = {
                    let b = self.boss.as_mut().unwrap();
                    b.rock_stage += 1;
                    b.rock_stage
                };
                self.launch_fallers(stage);
            }
            _ => {}
        }
    }

    /// items.c case 0x33: on each stage the floor pieces get their launch.
    fn launch_fallers(&mut self, stage: u32) {
        use gdl_formats::worlds::ItemType;
        let centre = self.boss.as_ref().map_or(Vec3::ZERO, |b| b.pos);
        let insts = self.level.worlds.item_instances.clone();
        for (k, inst) in insts.iter().enumerate() {
            let Some(info) = self.level.worlds.item_infos.get(inst.item_index as usize) else {
                continue;
            };
            if info.item_type != ItemType::Obstacle || info.subtype != 0x33 {
                continue;
            }
            let pos = Vec3::from(inst.pos);
            let Some(e) = self
                .entities
                .iter()
                .find(|e| e.alive && (e.pos - pos).length() < 0.05)
            else {
                continue;
            };
            let id = e.id;
            let mut r = |lo: f32, span: f32| lo + self.rng.f32() * span;
            let vel = match stage {
                1 => Vec3::new(0.0, r(20.0, 10.0), 0.0),
                2 => Vec3::new(0.0, r(30.0, 15.0), 0.0),
                // [CHOICE] Stage 3 throws them outward from a point the code
                // reads from a table we haven't identified; the boss's place.
                _ => {
                    let d =
                        Vec2::new(pos.x - centre.x, pos.z - centre.z).normalize_or_zero() * 10.0;
                    Vec3::new(d.x, r(50.0, 100.0), d.y)
                }
            };
            const SPIN: [f32; 8] = [-4.0, -3.0, -2.0, -1.0, 1.0, 2.0, 3.0, 4.0];
            let spin = (SPIN[k & 7] * 0.349_065_85, SPIN[!k & 7] * 0.349_065_85);
            // The piece's collision goes with it.
            let owner = 100_000 + k as i32;
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
            if let Some(b) = self.boss.as_mut() {
                b.fallers.retain(|f| f.id != id);
                b.fallers.push(Faller { id, vel, spin });
            }
        }
    }

    /// The thrown floor pieces fly and tumble; 2 units/s of speed come off a
    /// (30 Hz) frame, and they go 200 below the level.
    fn update_fallers(&mut self, dt: f32) {
        let Some(b) = self.boss.as_mut() else { return };
        if b.fallers.is_empty() {
            return;
        }
        let floor = self.level.worlds.min_bounds[1] - 200.0;
        let frames = dt * 30.0;
        let mut keep = Vec::new();
        for mut f in std::mem::take(&mut b.fallers) {
            let Some(e) = self.entities.iter_mut().find(|e| e.id == f.id && e.alive) else {
                continue;
            };
            f.vel.y -= 2.0 * frames;
            e.pos += f.vel * dt;
            e.spin += f.spin.0 * frames;
            e.solid = None;
            if e.pos.y < floor {
                e.alive = false;
                continue;
            }
            keep.push(f);
        }
        if let Some(b) = self.boss.as_mut() {
            b.fallers = keep;
        }
    }

    /// CameraLimitPlayerDpos / CamLimitPlayerDpos: while the boss camera
    /// runs, a player's move is clipped by the view frustum of a camera
    /// `view_scale` back from the boss camera's attention point (lbl_80343C5C
    /// is set on the GameCube, so this is NEWCAM's CalcFrustrumNormals +
    /// fn_8006DC64; see camera.rs). view_scale is 0.85 of maxdist in the
    /// fight, of 1.9375 maxpdist before it. The move is the change since last
    /// frame (players move before the boss is updated).
    fn limit_players_to_camera(&mut self) {
        let Some(b) = self.boss.as_ref() else { return };
        let Some(cam) = b.cam.filter(|c| c.started) else {
            return;
        };
        // After the Lich, Spider Queen or Genie die, players may roam for the
        // gold (fn_800629B0: gold left in the level).
        let gold_left = || {
            self.entities.iter().any(|e| {
                e.alive
                    && matches!(
                        e.kind,
                        Kind::Pickup {
                            kind: PickupKind::Gold(_),
                            ..
                        }
                    )
            })
        };
        let free = (b.dead || b.dying) && matches!(b.boss_type, 36 | 37 | 41) && gold_left();
        let def = cam.def;
        let view_scale = 0.85
            * if b.active {
                def.maxdist
            } else if def.maxpdist > 0.0 {
                1.9375 * def.maxpdist
            } else {
                1.9375 * def.maxdist
            };
        let f = Game::boss_frustum(cam.attn, cam_forward(cam.yaw, cam.pitch), view_scale);
        let prev = b.player_prev.clone();
        let mut now = Vec::new();
        for i in 0..self.players.len() {
            let cur = self.players[i].pos;
            let old = prev.get(i).copied().flatten();
            if let (Some(old), true, false) = (old, self.players[i].state.active(), free) {
                if let Some(new) = self.clip_player(&f, old, cur) {
                    self.players[i].pos = new;
                }
            }
            now.push(Some(self.players[i].pos));
        }
        if let Some(b) = self.boss.as_mut() {
            b.player_prev = now;
        }
    }

    /// BossActivate(obj, flag): the fight starts (gBossActive; the boss
    /// becomes a camera target once) and, with `music`, the level music
    /// switches to its boss part.
    pub(crate) fn boss_activate(&mut self, music: bool) {
        if let Some(b) = self.boss.as_mut() {
            b.active = true;
            b.targeted = true;
            if music {
                // sMusicSubIndex = sMusicSubState = 1: the boss part, faded in.
                self.music_area = (1, 1);
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
        if let Some(bit) = crate::progress::CRYSTAL_ORDER
            .iter()
            .position(|&r| r == realm)
        {
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
            // StartFXSub plays BOSSKEY once, then SfxSetMorph(30) changes it
            // into BOSSKEY2 for 30 s.
            if let Some(m) = key_model {
                let life = self.models[m].skeleton.sequences.first().map_or(1.0, |s| {
                    s.frame_count.max(1) as f32 * s.frame_rate.max(1) as f32 / 900.0
                });
                let id = self.spawn(Kind::Effect { life }, Some(m), key_pos, 0.0);
                if let Some(b) = self.boss.as_mut() {
                    b.key_effect = Some(id);
                }
            }
        }
    }

    /// An effect playing its model's first sequence once.
    fn spawn_effect_once(&mut self, m: usize, at: Vec3) -> u64 {
        let life = self.models[m].skeleton.sequences.first().map_or(1.0, |s| {
            s.frame_count.max(1) as f32 * s.frame_rate.max(1) as f32 / 900.0
        });
        self.spawn(Kind::Effect { life }, Some(m), at, 0.0)
    }

    /// SfxSetMorph: an ended effect becomes its next one; the legend effects
    /// fade over their last 3 s (gauntworld.c:1333); the Djinn's LEGENDFX
    /// rides on him.
    fn update_morphs(&mut self) {
        let Some(b) = self.boss.as_mut() else { return };
        let morphs = std::mem::take(&mut b.morphs);
        let aura = b.legend_aura;
        let mut keep = Vec::new();
        for (id, pos, next, time) in morphs {
            if self.entities.iter().any(|e| e.id == id && e.alive) {
                keep.push((id, pos, next, time));
            } else if let Some(m) = next {
                self.spawn(Kind::Effect { life: time }, Some(m), pos, 0.0);
            }
        }
        if let Some(b) = self.boss.as_mut() {
            b.morphs = keep;
        }
        for e in self.entities.iter_mut() {
            if let Kind::Effect { life } = e.kind {
                if life < 3.0
                    && self.boss.as_ref().is_some_and(|b| {
                        e.model.is_some() && (e.model == b.legend_fx.0 || e.model == b.legend_fx.1)
                    })
                {
                    e.fade = (life / 3.0).clamp(0.0, 1.0);
                }
            }
        }
        if let Some((id, until)) = aura {
            let at = self.boss_critter().map(|(e, _)| e.pos + Vec3::Y * 6.0);
            let now = self.time;
            if let Some(e) = self.entities.iter_mut().find(|e| e.id == id) {
                if let Some(at) = at {
                    e.pos = at;
                }
                if let Kind::Effect { life } = &mut e.kind {
                    *life = (until - now).max(0.0);
                }
            }
            if now >= until {
                if let Some(b) = self.boss.as_mut() {
                    b.legend_aura = None;
                }
            }
        }
    }

    /// The BOSSKEY effect's morph into BOSSKEY2 once it has played.
    fn update_boss_key(&mut self) {
        let Some(b) = self.boss.as_ref() else { return };
        let (Some(id), Some(pos)) = (b.key_effect, b.key.map(|k| k.0)) else {
            return;
        };
        if self.entities.iter().any(|e| e.id == id && e.alive) {
            return;
        }
        let key2 = b.key2_model;
        if let Some(b) = self.boss.as_mut() {
            b.key_effect = None;
        }
        if let Some(m) = key2 {
            self.spawn(Kind::Effect { life: 30.0 }, Some(m), pos, 0.0);
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
        for (k, (count, value)) in coin_counts(realm)
            .iter()
            .zip([500u32, 1000, 5000])
            .enumerate()
        {
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
                    s.vel.y = if s.vel.y.abs() < 0.1 {
                        0.0
                    } else {
                        -0.4 * s.vel.y
                    };
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
        self.update_fallers(dt);
        self.update_boss_key();
        self.update_morphs();
        self.limit_players_to_camera();
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
        let mut spawn_hold = None;
        match b.legend_state {
            2 => {
                // The cutscene: everyone faces the boss and waits.
                let wait = if matches!(t, 35 | 41 | 42) { 1.0 } else { 3.0 };
                if b.holder_step == 0 {
                    b.holder_step = 1;
                    // player.c:2508: the weapon is used up.
                    let realm = self.wave.map_or(0, |w| w >> 8);
                    self.legend_held &= !(1 << realm);
                    // FX 92: the weapon appears in the holder's hand.
                    spawn_hold = b.legend_models.0.filter(|_| b.legend_hold.is_none());
                }
                if now - b.legend_timer >= wait {
                    b.legend_state = 3;
                    b.legend_timer = now;
                    b.holder_step = 2;
                    b.holder_timer = now + 1.0;
                    self.sfx.push("S_PICKUPMAGIC".into());
                }
            }
            // After its 60-tick wind-up (quest_state 3) the holder is made to
            // play the throw (quest_state 4, see Game::legend_forced_action);
            // the weapon flies when that animation fires (the throw runs on
            // its own clock, even if the ROAR ends first).
            3..=5 if (2..=3).contains(&b.holder_step) => {
                if b.holder_step == 2 && now >= b.holder_timer {
                    b.holder_step = 3;
                    b.holder_fired = false;
                    self.sfx.push(format!("S_{letter}LEGWTHROW"));
                } else if b.holder_step == 3 && b.holder_fired {
                    b.holder_step = 4;
                    self.sfx.push(format!("S_{letter}LEGWFLY"));
                    self.legend_throw();
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
                        // PBOSSEYEBALL comes back.
                        for s in c.swapped.iter_mut() {
                            if t == 38 {
                                *s = false;
                            }
                        }
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
        if let Some(m) = spawn_hold {
            let at = self.legend_hand();
            let id = self.spawn(Kind::Effect { life: 1e9 }, Some(m), at, 0.0);
            if let Some(b) = self.boss.as_mut() {
                b.legend_hold = Some(id);
            }
        }
        self.update_legend_fx(dt);
    }

    /// Where the legend weapon is held: the right hand for bosses 34-39,
    /// (0, 8, 0) over the holder for the rest.
    fn legend_hand(&self) -> Vec3 {
        let Some(b) = self.boss.as_ref() else {
            return Vec3::ZERO;
        };
        let Some(p) = b.legend_holder.and_then(|i| self.players.get(i)) else {
            return Vec3::ZERO;
        };
        if (34..=39).contains(&b.boss_type) {
            let m = &self.models[p.model];
            if let Some(n) = m.node_index("R_WRIST") {
                let w = m.pose_layers(&[(None, p.animator.seq, p.animator.frame)]);
                let body = glam::Mat4::from_translation(p.pos) * glam::Mat4::from_rotation_y(p.yaw);
                return body.transform_point3(w[n].w_axis.truncate());
            }
        }
        p.pos + Vec3::new(0.0, 8.0, 0.0)
    }

    /// Where the legend projectile goes: the Chimera's second head, else
    /// the boss's body centre.
    fn legend_target(&self) -> Option<Vec3> {
        let (e, c) = self.boss_critter()?;
        let t = self.boss.as_ref()?.boss_type;
        if t == 35 && c.parts.len() > 2 {
            if let Some(p) = c.colliders().into_iter().find(|x| x.0 == 2) {
                return Some(p.2);
            }
        }
        Some(c.coll_pos(0, e.pos, e.yaw))
    }

    /// The throw (pmotion.c:2203-2390): the held weapon goes; the Dragon,
    /// Chimera, Genie and Plague Fiend get a projectile (FX 93) flying at
    /// 20 units/s that hits on arrival; the rest take the hit at once, with
    /// FX 93 shown at the boss.
    /// pmotion.c:2150: the action the legend-weapon holder is made to play at
    /// quest_state 4, and its follow-on, by boss.
    pub(crate) fn legend_forced_action(&self, pi: usize) -> Option<(u16, u16)> {
        use crate::paction::p as s;
        let b = self.boss.as_ref()?;
        if b.legend_holder != Some(pi) || b.holder_step != 3 {
            return None;
        }
        Some(match b.boss_type {
            36 | 37 => (s::SSHOT, s::SSHOT),
            34 | 35 | 38 | 39 => (s::PWRA_THROW, s::PWRA_THROW_R),
            _ => (s::USE_MAGIC, s::MAGIC_RELEASE),
        })
    }

    fn legend_throw(&mut self) {
        let Some(b) = self.boss.as_ref() else { return };
        let t = b.boss_type;
        let (hold, prj) = (b.legend_hold, b.legend_models.1);
        let ((fx1, fx2), home) = (b.legend_fx, b.home);
        let from = self.legend_hand();
        if (34..=39).contains(&t) || t == 41 {
            if let Some(id) = hold {
                if let Some(e) = self.entities.iter_mut().find(|e| e.id == id) {
                    e.alive = false;
                }
            }
            if let Some(b) = self.boss.as_mut() {
                b.legend_hold = None;
            }
        }
        if matches!(t, 34 | 35 | 36 | 38) {
            if let Some(m) = prj {
                let id = self.spawn(Kind::Effect { life: 10.0 }, Some(m), from, 0.0);
                if let Some(b) = self.boss.as_mut() {
                    b.legend_shot = Some(id);
                }
                return;
            }
            self.legend_hit();
            return;
        }
        // pmotion.c:2283-2360: an effect at the boss's home spot that changes
        // into LEGENDFX (or LEGENDFX2) when it ends: 41 FX 93 -> 90 for 5 s;
        // 39 FX 90 -> 91, 40 and 42 FX 93 -> 90, for 30 s.
        let plan = match t {
            41 => Some((prj, Vec3::ZERO, fx1, 5.0)),
            39 => Some((fx1, Vec3::new(-7.0, -5.0, 85.0), fx2, 30.0)),
            40 => Some((prj, Vec3::new(0.0, -5.0, 10.0), fx1, 30.0)),
            42 => Some((prj, Vec3::new(0.0, 17.0, 3.0), fx1, 30.0)),
            _ => None,
        };
        if let Some((Some(m), off, next, time)) = plan {
            let id = self.spawn_effect_once(m, home + off);
            if let Some(b) = self.boss.as_mut() {
                b.morphs.push((id, home + off, next, time));
            }
        } else if let (37, Some(m)) = (t, prj) {
            self.spawn(
                Kind::Effect { life: 3.0 },
                Some(m),
                from + Vec3::Z * 5.0,
                0.0,
            );
        }
        self.legend_hit();
    }

    /// The held weapon follows the hand; the projectile homes on its target.
    fn update_legend_fx(&mut self, dt: f32) {
        let Some(b) = self.boss.as_ref() else { return };
        let (hold, shot) = (b.legend_hold, b.legend_shot);
        if let Some(id) = hold {
            let at = self.legend_hand();
            match self.entities.iter_mut().find(|e| e.id == id) {
                Some(e) => {
                    e.pos = at;
                    e.spin += dt * 3.0;
                }
                None => {
                    if let Some(b) = self.boss.as_mut() {
                        b.legend_hold = None;
                    }
                }
            }
        }
        if let Some(id) = shot {
            let target = self.legend_target();
            let arrived = match (
                self.entities.iter_mut().find(|e| e.id == id && e.alive),
                target,
            ) {
                (Some(e), Some(t)) => {
                    let d = t - e.pos;
                    let step = 20.0 * dt;
                    e.yaw = d.x.atan2(d.z);
                    if d.length() <= step.max(2.0) {
                        e.alive = false;
                        true
                    } else {
                        e.pos += d.normalize() * step;
                        false
                    }
                }
                _ => true,
            };
            if arrived {
                if let Some(b) = self.boss.as_mut() {
                    b.legend_shot = None;
                }
                self.legend_hit();
            }
        }
    }

    /// The legend projectile hits the boss: its damage and weakening (spec
    /// table §5.2; sfx.c's delivery is replaced by an instant hit).
    fn legend_hit(&mut self) {
        let Some(b) = self.boss.as_ref() else { return };
        let t = b.boss_type;
        let holder = b.legend_holder;
        let letter = boss_letter(t);
        let Some(ei) = self
            .boss
            .as_ref()
            .and_then(|b| b.entity)
            .and_then(|id| self.entities.iter().position(|e| e.id == id))
        else {
            return;
        };
        let Kind::Critter(mut c) = std::mem::replace(&mut self.entities[ei].kind, Kind::Prop)
        else {
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
            36 => {
                c.parts[0].blind = 1800.0;
                // sfx.c:3236: FX_LEGEND1 rides on him, 6 up, for 1800 ticks.
                if let Some(m) = self.boss.as_ref().and_then(|b| b.legend_fx.0) {
                    let at = self.entities[ei].pos + Vec3::Y * 6.0;
                    let until = self.time + 30.0;
                    let id = self.spawn(Kind::Effect { life: 30.0 }, Some(m), at, 0.0);
                    if let Some(b) = self.boss.as_mut() {
                        b.legend_aura = Some((id, until));
                    }
                }
            }
            38 => {
                c.parts[0].blind = 18000.0;
                // The eye becomes PBOSSQEYEBALL until the weakness ends.
                if let Some(n) = c.model.and_then(|m| self.models[m].node_index("EYEBALL")) {
                    if let Some(s) = c.swapped.get_mut(n) {
                        *s = true;
                    }
                }
            }
            37 => {
                c.parts[0].fball_cap = 0.5;
                // Tinted 0xFF40FF40 and shrunk to 0.8, for good.
                c.tint = Some([0.25, 1.0, 0.25, 1.0]);
                c.scale = 0.8;
            }
            39 => c.parts[0].fball_cap = 0.5,
            40 => c.parts[0].fball_cap = 0.25,
            42 => c.parts[0].fball_cap = 0.1,
            _ => {}
        }
        if dmg > 0.0 {
            self.critter_damage(
                &mut c,
                part,
                dmg,
                holder,
                0x200000 | 0x100000,
                Vec3::ZERO,
                None,
            );
        }
        self.entities[ei].kind = Kind::Critter(c);
        self.sfx
            .push(format!("S_{letter}LEGWHIT|S_{letter}LEGWALSTP"));
        if let Some(b) = self.boss.as_mut() {
            if b.legend_state == 3 {
                b.legend_state = 4;
            }
        }
    }

    pub(crate) fn boss_critter_mut(&mut self) -> Option<(usize, &mut Critter)> {
        let id = self.boss.as_ref()?.entity?;
        self.entities
            .iter_mut()
            .enumerate()
            .find_map(|(i, e)| match &mut e.kind {
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
        let gold_left = self.entities.iter().any(|e| {
            e.alive
                && matches!(
                    e.kind,
                    Kind::Pickup {
                        kind: PickupKind::Gold(_),
                        ..
                    }
                )
        });
        let active: Vec<usize> = (0..self.players.len())
            .filter(|&i| self.players[i].state.active())
            .collect();
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
                    voice = wizard_voice(
                        t,
                        if t >= 42 {
                            w.all_rune_stones as usize
                        } else {
                            0
                        },
                    );
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
                                        w.attn = active[(active
                                            .iter()
                                            .position(|&i| i == w.attn)
                                            .map_or(0, |k| k + 1))
                                            % active.len()];
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
                w.exit_timer = if gold_left || w.all_rune_stones {
                    600
                } else {
                    120
                };
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
        // The boss's movevec (crit_inst+0x4C): its position raised by its
        // type's y_attn (hdr+0xB4, ProcessCritter), and its radius (hdr+0x78).
        let boss = self.boss_critter().map(|(e, c)| {
            let t = &c.wad.types[c.parts[0].ty];
            (e.pos + Vec3::Y * t.y_attn, t.hht, c.init_yaw)
        });
        let wiz = b
            .wizard
            .as_ref()
            .filter(|w| w.state >= 4)
            .map(|w| w.pos + Vec3::Y * 10.0);
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
            td = self.zoom_dist(
                &cam,
                def.minpdist,
                1.9375 * def.maxpdist,
                false,
                focus,
                focus_rad,
            );
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
            } else if let Some((bp, brad, _)) = boss {
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
                    b.home
                };
                (src, bp, brad)
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
            let maxd = if wiz.is_some() {
                2.0 * def.maxdist
            } else {
                def.maxdist
            };
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
            cam.attn[k] = limit_cam_val(
                cam.attn[k],
                attn_target[k],
                -200.0,
                200.0,
                75.0,
                0.0,
                &mut v,
                false,
                dt,
            );
            cam.vattn[k] = v;
        }
        let (mut vy, mut vp, mut vd) = (cam.vyaw, cam.vpitch, cam.vdist);
        if jump {
            let d = cam.attn - eye;
            let len = d.length().max(1.0);
            cam.yaw = d.x.atan2(d.z);
            cam.pitch = (d.y / len).clamp(-1.0, 1.0).asin();
            cam.dist = if wiz.is_some() || key.is_some() {
                len
            } else {
                cam.dist.max(len)
            };
            (vy, vp, vd) = (0.0, 0.0, 0.0);
        } else {
            cam.yaw = wrap_pi(limit_cam_val(
                cam.yaw,
                ty,
                -FRAC_PI_2,
                FRAC_PI_2,
                PI / 6.0,
                0.1,
                &mut vy,
                true,
                dt,
            ));
            cam.pitch = limit_cam_val(
                cam.pitch,
                tp,
                -FRAC_PI_2,
                FRAC_PI_2,
                PI / 12.0,
                0.1,
                &mut vp,
                false,
                dt,
            );
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
    fn zoom_dist(
        &self,
        cam: &BossCam,
        min: f32,
        max: f32,
        boss: bool,
        focus: Vec3,
        focus_rad: f32,
    ) -> f32 {
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
    point_view_clip(eye, fwd, p, r).0
}

/// The camera's view basis (right, up) for a forward direction.
fn view_basis(fwd: Vec3) -> (Vec3, Vec3) {
    let right = fwd.cross(Vec3::Y).normalize_or_zero();
    let up = right.cross(fwd).normalize_or_zero();
    (right, up)
}

/// PointViewDist with gPointViewClipFlags: 1 near, 0x10 / 0x20 the left /
/// right planes, 0x200 / 0x100 the bottom / top planes (bit set when the
/// sphere pokes out through it).
fn point_view_clip(eye: Vec3, fwd: Vec3, p: Vec3, r: f32) -> (f32, u32) {
    let (right, up) = view_basis(fwd);
    let d = p - eye;
    let (x, y, z) = (d.dot(right), d.dot(up), d.dot(fwd));
    let hv = 22.5f32.to_radians();
    let hh = (hv.tan() * 4.0 / 3.0).atan();
    // [CHOICE] The status panels cover the bottom 80 of the 384 lines, so the
    // frame's bottom edge is taken at line 304. (The game's window is 60
    // degrees wide (newcam zoom 1.0472), ours 45 degrees high at 4:3.)
    let hb = (hv.tan() * (2.0f32 * 304.0 / 384.0 - 1.0)).atan();
    let side = |a: f32, h: f32| z * h.sin() - a * h.cos();
    let planes = [
        (z - 1.0 - r, 1u32),
        (side(-x, hh) - r, 0x10),
        (side(x, hh) - r, 0x20),
        (side(-y, hb) - r, 0x200),
        (side(y, hv) - r, 0x100),
    ];
    let mut best = 1e20f32;
    let mut flags = 0;
    for (d, bit) in planes {
        best = best.min(d);
        if d < 0.0 {
            flags |= bit;
        }
    }
    (best, flags)
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
    pub(crate) fn legend_freeze_inputs(
        &self,
        inputs: &[crate::input::PlayerInput],
    ) -> Vec<crate::input::PlayerInput> {
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
        let Some(bp) = self.boss_critter().map(|(e, _)| e.pos) else {
            return;
        };
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
            assert!(
                !g.boss.as_ref().unwrap().meters.is_empty(),
                "{lvl}: no meters"
            );
            let act = g.boss_critter().unwrap().1.wad.types[0].activedist;
            face_boss(&mut g, if act > 0.0 { act * 0.5 } else { 25.0 });
            move_boss_floor(&mut g);
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

    /// Skorne's camera aims at his movevec (y_attn 24 above his feet in the
    /// lava pit) plus BCAM's attn offset: the eye stays above the arena floor
    /// with the hero on the carpet in view.
    #[test]
    fn skorne_camera_frames_the_arena() {
        let Some(fs) = data() else { return };
        let mut g = Game::new(&fs, "levelE2", &[("WAR", "RED")]).unwrap();
        let hp0 = g.players[0].health;
        for _ in 0..450 {
            g.players[0].health = hp0;
            let input = g.bot_input(0);
            g.update(1.0 / 60.0, &[input]);
        }
        assert!(g.boss.as_ref().unwrap().active, "Skorne never woke");
        let (eye, at, _) = g.boss_camera_view().expect("boss camera");
        let hero = g.players[0].pos;
        let fwd = (at - eye).normalize();
        eprintln!("E2: eye {eye:?} at {at:?} hero {hero:?}");
        assert!(
            eye.y > hero.y + 5.0,
            "eye {eye:?} below the floor ({hero:?})"
        );
        assert!(at.y > hero.y - 5.0, "aimed into the pit: {at:?}");
        assert!(
            point_view_dist(eye, fwd, hero + Vec3::Y * 2.5, 0.0) > 0.0,
            "hero {hero:?} out of view"
        );
    }

    #[test]
    fn skorne_throws_the_held_hero_back_onto_the_arena() {
        let Some(fs) = data() else { return };
        let mut g = Game::new(&fs, "levelF2", &[("WAR", "RED")]).unwrap();
        let (mut held, mut thrown) = (false, false);
        for _ in 0..900 {
            let input = g.bot_input(0);
            g.update(1.0 / 60.0, &[input]);
            let now = g.entities.iter().any(
                |e| matches!(&e.kind, Kind::Critter(c) if c.parts.iter().any(|q| q.grab == 0)),
            );
            if held {
                // Held heroes don't build up a fall.
                assert!(g.players[0].vy.abs() < 1e-3 || !now);
            }
            thrown |= held && !now;
            held = now;
        }
        assert!(thrown, "never grabbed and released");
        let p = &g.players[0];
        assert!(p.on_ground && (p.pos.y + 10.0).abs() < 1.0, "{:?}", p.pos);
    }

    #[test]
    fn heroes_below_the_world_come_back_beside_a_partner() {
        let Some(fs) = data() else { return };
        let mut g = Game::new(&fs, "levelA1", &[("WAR", "RED"), ("VAL", "BLU")]).unwrap();
        for _ in 0..30 {
            g.update(
                1.0 / 60.0,
                &[
                    crate::input::PlayerInput::default(),
                    crate::input::PlayerInput::default(),
                ],
            );
        }
        let partner = g.players[0].pos;
        g.players[1].pos = Vec3::new(partner.x, g.level.worlds.min_bounds[1] - 1.0, partner.z);
        g.update(
            1.0 / 60.0,
            &[
                crate::input::PlayerInput::default(),
                crate::input::PlayerInput::default(),
            ],
        );
        let p = g.players[1].pos;
        assert!(
            p.distance(partner) < 6.0 && (p.y - partner.y).abs() <= 3.0,
            "{p:?} vs {partner:?}"
        );
    }

    #[test]
    fn the_boss_key_changes_into_bosskey2() {
        let Some(fs) = data() else { return };
        let mut g = Game::new(&fs, "levelA5", &[("WAR", "RED")]).unwrap();
        let (k1, k2) = {
            let b = g.boss.as_ref().unwrap();
            (b.key_model, b.key2_model)
        };
        assert!(k1.is_some() && k2.is_some(), "{k1:?} {k2:?}");
        let at = g.players[0].pos;
        g.boss_death(at);
        let mut seen2 = false;
        for _ in 0..1200 {
            g.update(1.0 / 60.0, &[crate::input::PlayerInput::default()]);
            seen2 |= g.entities.iter().any(|e| e.alive && e.model == k2);
        }
        assert!(seen2);
    }

    #[test]
    fn legend_hits_leave_their_effects() {
        let Some(fs) = data() else { return };
        let mut checked = 0;
        for l in [
            "levelA5", "levelB6", "levelC5", "levelD5", "levelG5", "levelH4", "levelI5", "levelJ5",
            "levelK5",
        ] {
            let mut g = Game::new(&fs, l, &[("WAR", "RED")]).unwrap();
            let realm = g.wave.map_or(0, |w| w >> 8);
            g.set_legend_held(1 << realm);
            face_boss(&mut g, 25.0);
            let (t, fx) = {
                let b = g.boss.as_ref().unwrap();
                (b.boss_type, b.legend_fx)
            };
            let mut seen = false;
            for _ in 0..2400 {
                g.players[0].health = 500.0;
                g.update(1.0 / 60.0, &[crate::input::PlayerInput::default()]);
                seen |= g
                    .entities
                    .iter()
                    .any(|e| e.alive && e.model.is_some() && (e.model == fx.0 || e.model == fx.1));
            }
            if matches!(t, 36 | 39 | 40 | 41 | 42) && fx.0.is_some() {
                assert!(seen, "{l}: boss {t}");
                checked += 1;
            }
        }
        assert!(checked > 0);
    }

    /// Set off the mover the boss stands on (the Chimera's elevator), as
    /// its pressure pad would.
    fn move_boss_floor(g: &mut Game) {
        let Some(pos) = g.boss_critter().map(|(e, _)| e.pos) else {
            return;
        };
        let Some((_, floor)) = g.critter_floor_hit(pos) else {
            return;
        };
        for m in &mut g.movers {
            if floor >= 0 && m.objects.contains(&(floor as usize)) {
                m.active = true;
            }
        }
    }

    fn wake(g: &mut Game) {
        move_boss_floor(g);
        for _ in 0..1200 {
            if g.boss.as_ref().is_some_and(|b| b.active)
                && g.boss_critter().is_some_and(|(_, c)| c.parts[0].state == 3)
            {
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
        assert!(
            (hp0 - boss_health(&g) - 100.0).abs() < 1e-3,
            "{}",
            boss_health(&g)
        );
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
                    .filter(|e| {
                        matches!(
                            e.kind,
                            Kind::Pickup {
                                kind: PickupKind::Gold(_),
                                ..
                            }
                        )
                    })
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
            eprintln!(
                "at {at:?} surface {:?} colliders {:?}",
                c.surface(at, e.pos, e.yaw),
                c.colliders()
            );
        }
        g.hit_entity(0, t, 102.0, Vec2::ZERO, false);
        let (_, c) = g.boss_critter().unwrap();
        // During the legend sequence (states 1-4) there is no player scaling;
        // armour 2.
        assert!(
            (c.parts[1].health - (900.0 - 100.0)).abs() < 1e-2,
            "{}",
            c.parts[1].health
        );
        assert!((c.parts[0].health - (root - 100.0)).abs() < 1e-2);
        // The legend sequence: the START ends the intro, the cutscene, the
        // throw, and the hit kills the second head.
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..1800 {
            g.update(1.0 / 60.0, &[crate::input::PlayerInput::default()]);
            seen.insert(g.legend_state());
        }
        assert!(
            seen.contains(&2) && seen.contains(&3) && seen.contains(&5),
            "{seen:?}"
        );
        let (_, c) = g.boss_critter().unwrap();
        assert!(
            c.parts[2].health <= 0.0,
            "the lion's head: {}",
            c.parts[2].health
        );
        assert_eq!(g.legend_held & 2, 0, "the scimitar is used up");
    }

    fn rocks(g: &Game) -> Vec<SafeRock> {
        g.boss
            .as_ref()
            .and_then(|b| b.safe_rocks.clone())
            .unwrap_or_default()
    }

    /// The Yeti has a SPOUT2 (POUND), so I5's safe rocks are collected hidden;
    /// a POUND erupts at the hidden rock nearest its target, which rises with
    /// 3x its hit points once the eruption is over.
    #[test]
    fn yeti_pound_raises_the_safe_rock_nearest_the_target() {
        let Some(fs) = data() else { return };
        let mut g = Game::new(&fs, "levelI5", &[("WAR", "RED")]).unwrap();
        g.update(1.0 / 60.0, &[crate::input::PlayerInput::default()]);
        let r = rocks(&g);
        assert!(r.len() >= 5, "{} rocks", r.len());
        assert!(r.iter().all(|r| r.strength == -1 && r.entity.is_none()));
        // A POUND aimed at the hero standing by rock 3.
        g.players[0].pos = r[3].pos + Vec3::new(2.0, 0.0, 0.0);
        let at = g.safe_rock_spout2(0, 0.5).expect("a rock erupts");
        assert!((at - r[3].pos).length() < 1e-3);
        for _ in 0..60 {
            g.update(1.0 / 60.0, &[crate::input::PlayerInput::default()]);
        }
        let r = rocks(&g);
        let id = r[3].entity.expect("the rock rose");
        assert_eq!(r[3].strength, 0);
        let e = g.entities.iter().find(|e| e.id == id).unwrap();
        assert!(
            matches!(e.kind, Kind::Container { hp, .. } if (hp - 3.0 * r[3].hit_points).abs() < 1e-3)
        );
        // In the fight the Yeti pounds on its own.
        let mut g = Game::new(&fs, "levelI5", &[("WAR", "RED")]).unwrap();
        face_boss(&mut g, 45.0);
        let mut risen = 0;
        for _ in 0..3600 {
            g.players[0].health = 500.0;
            g.update(1.0 / 60.0, &[crate::input::PlayerInput::default()]);
            risen = risen.max(rocks(&g).iter().filter(|r| r.entity.is_some()).count());
        }
        assert!(risen >= 1, "no rock rose in a minute of fighting");
    }

    /// The Plague Fiend's SPOUT erupts at every safe rock; with no SPOUT2 its
    /// rocks stay where they are.
    #[test]
    fn plague_fiend_spouts_at_every_safe_rock() {
        let Some(fs) = data() else { return };
        let mut g = Game::new(&fs, "levelK5", &[("WAR", "RED")]).unwrap();
        g.update(1.0 / 60.0, &[crate::input::PlayerInput::default()]);
        let r = rocks(&g);
        assert!(!r.is_empty());
        assert!(r.iter().all(|r| r.strength == 1 && r.entity.is_some()));
        assert_eq!(g.safe_rock_positions().len(), r.len());
    }

    /// SafeRockSetup: the Lich's dirt mound is hidden; Skorne's stages throw
    /// up the floor pieces (subtype 0x33).
    #[test]
    fn safe_rock_setup_per_boss() {
        let Some(fs) = data() else { return };
        let mut g = Game::new(&fs, "levelG5", &[("WAR", "RED")]).unwrap();
        let n = g.hidden.len();
        g.safe_rock_setup();
        assert!(g.hidden.len() > n, "G5BIGDIRT hidden");
        let mut g = Game::new(&fs, "levelE2", &[("WAR", "RED")]).unwrap();
        g.safe_rock_setup();
        let flying = g.boss.as_ref().unwrap().fallers.len();
        eprintln!("E2 stage 1: {flying} pieces");
        assert_eq!(g.boss.as_ref().unwrap().rock_stage, 1);
    }

    /// The Bellows tint the Spider Queen green and shrink her; the Javelin
    /// swaps the Plague Fiend's eye; the weapon is held, then thrown.
    #[test]
    fn legend_weapon_visuals() {
        let Some(fs) = data() else { return };
        let mut g = Game::new(&fs, "levelD5", &[("WAR", "RED")]).unwrap();
        g.set_legend_held(1 << 4);
        face_boss(&mut g, 25.0);
        let mut held = false;
        for _ in 0..1800 {
            g.players[0].health = 500.0;
            g.update(1.0 / 60.0, &[crate::input::PlayerInput::default()]);
            held |= g.boss.as_ref().unwrap().legend_hold.is_some();
        }
        assert!(held, "the bellows were held");
        let (_, c) = g.boss_critter().unwrap();
        assert_eq!(c.scale, 0.8);
        assert!(c.tint.is_some());
        let mut g = Game::new(&fs, "levelK5", &[("WAR", "RED")]).unwrap();
        g.set_legend_held(1 << 11);
        face_boss(&mut g, 25.0);
        let mut swapped = false;
        let mut shot = false;
        let mut threw = false;
        for _ in 0..1800 {
            g.players[0].health = 500.0;
            g.update(1.0 / 60.0, &[crate::input::PlayerInput::default()]);
            // The javelin goes with the holder's forced PWRA_THROW (boss 38).
            threw |= g.players[0].act.cur == crate::paction::p::PWRA_THROW;
            assert!(!shot || threw);
            shot |= g.boss.as_ref().unwrap().legend_shot.is_some();
            swapped |= g
                .boss_critter()
                .is_some_and(|(_, c)| c.swapped.iter().any(|&s| s));
        }
        assert!(shot, "the javelin flew");
        assert!(swapped, "PBOSSQEYEBALL");
    }

    /// CameraLimitPlayerDpos: a step out of the boss camera's frame is
    /// clipped; a step back in is not.
    #[test]
    fn players_stay_in_the_boss_cameras_frame() {
        let Some(fs) = data() else { return };
        let mut g = Game::new(&fs, "levelB6", &[("WAR", "RED")]).unwrap();
        for _ in 0..600 {
            g.update(1.0 / 60.0, &[crate::input::PlayerInput::default()]);
        }
        let c = g.boss.as_ref().unwrap().cam.unwrap();
        let fwd = cam_forward(c.yaw, c.pitch);
        let eye = c.attn - fwd * c.dist;
        let (right, _) = view_basis(fwd);
        // Walk the hero to the edge of the frame, sideways, a unit at a time.
        let start = g.players[0].pos;
        let mut p = start;
        for _ in 0..200 {
            g.boss.as_mut().unwrap().player_prev = vec![Some(p)];
            g.players[0].pos = p + right * 1.0;
            g.limit_players_to_camera();
            p = g.players[0].pos;
        }
        let vd = point_view_dist(eye, fwd, p + Vec3::Y * 2.5, 1.9375 * 2.5);
        assert!(vd > -1.5, "walked off screen: {vd}");
        assert!((p - start).length() > 2.0, "never moved");
        // Back toward the middle is free.
        g.boss.as_mut().unwrap().player_prev = vec![Some(p)];
        g.players[0].pos = p - right;
        g.limit_players_to_camera();
        assert!((g.players[0].pos - (p - right)).length() < 1e-4);
    }

    /// Golems and gargoyles carry the GMETER bar; generals don't.
    #[test]
    fn gmeter_on_golems_and_gargoyles() {
        let Some(fs) = data() else { return };
        let g = Game::new(&fs, "levelA1", &[("WAR", "RED")]).unwrap();
        let mut seen = std::collections::BTreeMap::new();
        for e in &g.entities {
            if let Kind::Critter(c) = &e.kind {
                seen.insert(c.etype, c.gmeter.is_some());
            }
        }
        assert_eq!(seen.get(&crate::critter::ET_GOLEM), Some(&true));
        assert_eq!(seen.get(&crate::critter::ET_GARGOYLE), Some(&true));
        assert_eq!(seen.get(&crate::critter::ET_GENERAL), Some(&false));
    }

    /// The Dragon's floor probe (a radius-5 sphere) finds the lip of the
    /// ledge, so it stands at ledge height + 18.5 rather than hanging over
    /// the lava with no floor; the boss camera then settles above the
    /// players' ledge, framing the hero and the Dragon.
    #[test]
    fn dragon_stands_at_the_ledge_and_the_camera_frames_it() {
        let Some(fs) = data() else { return };
        let mut g = Game::new(&fs, "levelB6", &[("WAR", "RED")]).unwrap();
        let bp = g.boss_critter().unwrap().0.pos;
        assert!((bp.y - (29.65 + 18.5)).abs() < 0.5, "dragon at {bp:?}");
        for _ in 0..600 {
            g.update(1.0 / 60.0, &[crate::input::PlayerInput::default()]);
        }
        let (eye, at, _) = g.camera_view();
        let p = g.players[0].pos;
        assert!(
            eye.y > p.y + 5.0,
            "eye {eye:?} below the ledge (player {p:?})"
        );
        let c = g.boss.as_ref().unwrap().cam.unwrap();
        let fwd = cam_forward(c.yaw, c.pitch);
        assert!(
            point_view_dist(eye, fwd, p + Vec3::Y * 2.5, 2.5) >= 0.0,
            "hero off screen"
        );
        assert!((at - bp).length() < 30.0);
    }

    /// Garm's armour plates take at most their share of its health, then
    /// break (GARMD<node>); the left arm's hand attacks go with it. His
    /// GAR2GEN shots bring GARM2 escorts.
    #[test]
    fn garm_plates_break_and_escorts_come() {
        let Some(fs) = data() else { return };
        let mut g = Game::new(&fs, "levelH4", &[("WAR", "RED")]).unwrap();
        face_boss(&mut g, 25.0);
        wake(&mut g);
        let plate = |g: &Game| {
            let (_, c) = g.boss_critter().unwrap();
            let ci = c
                .wad
                .type_colnodes(0)
                .iter()
                .position(|n| n.node == "L_ARM")
                .unwrap();
            (
                ci,
                c.colliders().into_iter().find(|x| x.1 == ci).map(|x| x.2),
                c.parts[0].node_damage[ci],
            )
        };
        let (ci, at, (_, cap)) = plate(&g);
        let at = at.expect("L_ARM is hittable");
        let hp0 = boss_health(&g);
        for _ in 0..30 {
            if plate(&g).1.is_none() {
                break;
            }
            g.hit_from = plate(&g).1;
            let t = boss_index(&g);
            g.hit_entity(0, t, 60.0, Vec2::ZERO, false);
        }
        let (_, c) = g.boss_critter().unwrap();
        // The plate absorbed at most its cap, then broke and was swapped.
        assert!((c.parts[0].node_damage[ci].0 - cap).abs() < 1e-3);
        assert!(hp0 - boss_health(&g) <= cap + 1e-3);
        let node = c.parts[0].colnode_nodes[ci].unwrap();
        assert!(c.swapped[node], "the L_ARM plate is broken");
        let _ = at;
        // Moves on the left hand (flag 0x10) are gone.
        let moves = c.parts[0].moves(&c.wad);
        let lost: Vec<String> = moves
            .iter()
            .enumerate()
            .filter(|(i, m)| {
                m.flags & 0x10 != 0 && c.parts[0].move_nodes[*i].is_some_and(|n| c.removed[n])
            })
            .map(|(_, m)| m.name.clone())
            .collect();
        assert!(!lost.is_empty(), "no hand attack lost");
        // Escorts.
        let mut escorts = 0;
        for _ in 0..3600 {
            g.players[0].health = 500.0;
            g.update(1.0 / 60.0, &[crate::input::PlayerInput::default()]);
            escorts = escorts.max(
                g.entities
                    .iter()
                    .filter(|e| matches!(e.kind, Kind::Enemy { etype: 27, .. }))
                    .count(),
            );
        }
        eprintln!("H4: lost {lost:?}, escorts {escorts}");
        assert!(escorts >= 2, "no GARM2 escorts");
    }

    /// CritterBossAI: the Chimera (typeFlags 0x80) sleeps however close the
    /// players are, until the floor it stands on (A5FLOOR#00, child of the
    /// A5ELEVATOR mover) moves; it wakes 2 s later.
    #[test]
    fn chimera_wakes_by_its_elevator() {
        let Some(fs) = data() else { return };
        let mut g = Game::new(&fs, "levelA5", &[("WAR", "RED")]).unwrap();
        let elevator = g
            .movers
            .iter()
            .position(|m| g.level.worlds.world_objects[m.root].name == "A5ELEVATOR")
            .unwrap();
        let pad = g
            .entities
            .iter()
            .find(|e| matches!(e.kind, Kind::Trigger { mover: Some(m), .. } if m == elevator))
            .map(|e| e.pos)
            .unwrap();
        // Next to the boss but off the pad: it stays asleep.
        face_boss(&mut g, 10.0);
        for _ in 0..300 {
            g.players[0].pos.z = g.players[0].pos.z.min(pad.z - 6.0);
            g.update(1.0 / 60.0, &[crate::input::PlayerInput::default()]);
        }
        assert!(!g.boss.as_ref().unwrap().active, "{}", boss_state(&g));
        assert_eq!(g.boss_critter().unwrap().1.parts[0].state, 0);
        // On the pad the elevator moves: BossActivate(c, 0) now, the
        // critters wake 2 s on (CritterActivateChain).
        g.players[0].pos = pad;
        let mut woke = None;
        for f in 0..600 {
            g.update(1.0 / 60.0, &[crate::input::PlayerInput::default()]);
            if g.boss_critter().unwrap().1.parts[0].state == 3 {
                woke = Some(f);
                break;
            }
        }
        assert!(g.boss.as_ref().unwrap().active);
        let f = woke.expect("the Chimera never woke");
        assert!((115..=125).contains(&f), "woke on frame {f}");
    }

    /// CritterLookAtPlayer: a boss's head node turns toward its target
    /// beyond what the animation does (the Dragon's neck, the Lich's head).
    #[test]
    fn boss_heads_turn_toward_their_target() {
        let Some(fs) = data() else { return };
        for lvl in ["B6", "G5", "A5"] {
            let mut g = Game::new(&fs, &format!("level{lvl}"), &[("WAR", "RED")]).unwrap();
            face_boss(&mut g, 25.0);
            wake(&mut g);
            let mut best = f32::MAX;
            let mut turned = 0;
            for f in 0..900 {
                // Keep the hero off to one side.
                if f == 0 {
                    let (bp, yaw) = g.boss_critter().map(|(e, _)| (e.pos, e.yaw)).unwrap();
                    let side = Vec3::new(yaw.cos(), 0.0, -yaw.sin());
                    let p = g.players[0].pos + side * 12.0;
                    if let Some(gy) = g.collision.ground_height(p, bp.y + 30.0) {
                        g.players[0].pos = Vec3::new(p.x, gy, p.z);
                    }
                }
                g.players[0].health = 999.0;
                g.update(1.0 / 60.0, &[crate::input::PlayerInput::default()]);
                let (e, c) = g.boss_critter().unwrap();
                let model = &g.models[e.model.unwrap()];
                let body = glam::Mat4::from_translation(e.pos) * glam::Mat4::from_rotation_y(e.yaw);
                for p in &c.parts {
                    let (Some(n), true) = (p.head_node, p.tgt >= 0) else {
                        continue;
                    };
                    if !c.look.iter().any(|l| l.0 == n) {
                        continue;
                    }
                    let to = g.players[p.tgt as usize].pos + Vec3::Y * 2.5;
                    let err = |m: glam::Mat4| {
                        let w = body * m;
                        let f = w.z_axis.truncate();
                        let d = to - w.w_axis.truncate();
                        let a = f.x.atan2(f.z) - d.x.atan2(d.z);
                        a.sin().atan2(a.cos()).abs()
                    };
                    let looked = err(c.pose(model)[n]);
                    let plain = err(model.pose_layers(&c.layers())[n]);
                    best = best.min(looked - plain);
                    turned += 1;
                }
            }
            assert!(turned > 0, "{lvl}: no head looked");
            assert!(
                best < -0.05,
                "{lvl}: the head never turned toward the hero ({best})"
            );
        }
    }

    /// CritterResolveMultipleTargets: a lone hero who hasn't hurt the
    /// Chimera (invanger 11) is aimed at by two of its four parts; the
    /// other heads, farthest first, let it go.
    #[test]
    fn chimera_heads_spread_their_targets() {
        let Some(fs) = data() else { return };
        let mut g = Game::new(&fs, "levelA5", &[("WAR", "RED")]).unwrap();
        face_boss(&mut g, 20.0);
        wake(&mut g);
        let mut checked = 0;
        for _ in 0..10 {
            g.update(1.0 / 60.0, &[crate::input::PlayerInput::default()]);
            let (_, c) = g.boss_critter().unwrap();
            let aiming = c.parts.iter().filter(|p| !p.targets.is_empty()).count();
            if c.parts[0].pattern < 0 && !c.parts[0].targets.is_empty() {
                assert_eq!(
                    aiming,
                    2,
                    "{:?}",
                    c.parts.iter().map(|p| p.targets.len()).collect::<Vec<_>>()
                );
                assert_eq!(g.crit_ntargets[0], 2.0);
                checked += 1;
            }
        }
        assert!(checked > 0);
    }
}
