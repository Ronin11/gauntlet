//! Potion magic: player.c start_magic and the effects it starts (sfx.c
//! StartMagicFX, StartThrowMagicFX, StartShieldFX, StartMagicHealFX,
//! StartMagicPlayerFX), run as ProcessEffects runs them.

use super::*;

/// lbl_80122088 + 3484 / 3504 / 3524 (magicid, throwid, shieldid) and FX 34,
/// 51, 50: the effect rows InitEffects resolves (lbl_80122118) by colour
/// (0 random, 1 fire, 2 lightning, 3 light, 4 acid).
const BLAST_FX: [&str; 5] = ["MP_FIRE", "MP_FIRE", "MP_ELEC", "MP_LIGHT", "MP_ACID"];
const THROW_FX: [&str; 5] = [
    "POT_RED_TW",
    "POT_RED_TW",
    "POT_BLU_TW",
    "POT_YEL_TW",
    "POT_GRE_TW",
];
const SHIELD_FX: [&str; 5] = ["MS_FIRE", "MS_FIRE", "MS_ELEC", "MS_LIGHT", "MS_ACID"];
/// FX_START_MAGIC, FX_MAGICHEAL and FX_HEALENEMY.
const START_FX: &str = "MAGICFX";
const HEAL_FX: &str = "MAGICHEALTH";
const HEAL_ENEMY_FX: &str = "LEVELUP_GRE";

/// AudioPotion's lbl_801239B4[shield][colour]: S_POTION2/1/3/4, and
/// S_SHIELD2/1/3/4 for the shield.
pub(crate) const POTION_SOUND: [[i32; 5]; 2] =
    [[0x7, 0x7, 0x6, 0x8, 0x9], [0x4D, 0x4D, 0x4C, 0x4E, 0x4F]];

/// StartThrowMagicFX: the bottle's lifetime (s), weight (fall rate) and
/// colrad; it bursts at the latest 0.0667 s before its end (ProcessEffects).
const THROW_TIME: f32 = 0.667;
const THROW_WEIGHT: f32 = 100.0;
const THROW_COLRAD: f32 = 0.5;
/// StartShieldFX's time.
const SHIELD_TIME: f32 = 3.0;

/// The effect models InitEffects finds for potion magic, by colour.
#[derive(Clone, Debug, Default)]
pub(crate) struct MagicModels {
    pub blast: [Option<usize>; 5],
    pub throw: [Option<usize>; 5],
    pub shield: [Option<usize>; 5],
    pub start: Option<usize>,
    pub heal: Option<usize>,
    pub heal_enemy: Option<usize>,
}

/// How a magic effect's damage spreads (ProcessEffects' mode).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum MagicStage {
    /// Mode 0: the thrown bottle, a point missile falling at THROW_WEIGHT.
    Thrown { vel: Vec3 },
    /// Mode 1 (flag 0x20): the ring grows from a third of its radius to the
    /// whole over the first two thirds of its life, weakening as it goes.
    Blast,
    /// Mode 2 (flags 0x30): the shield, its whole radius at full damage,
    /// riding its hero.
    Shield,
}

/// One live potion effect (an Effects[] slot with damage).
#[derive(Clone, Debug)]
pub(crate) struct MagicFx {
    /// owner - 1: the casting player, None for a generic blast.
    pub owner: Option<usize>,
    pub colour: u8,
    pub stage: MagicStage,
    /// damage, damageradius and the DMG_HEAL bit (0x800000) of damagetype:
    /// set on a level 25+ hero's magic, it has damage_enemy call
    /// do_heal_players.
    pub damage: f32,
    pub radius: f32,
    pub heal: bool,
    /// Seconds since the effect (or its morph) began, and maxtime.
    pub time: f32,
    pub maxtime: f32,
    pub pos: Vec3,
    /// hitscale: the morphed blast's node scale.
    hitscale: f32,
    /// The Kind::Effect entity showing it.
    pub entity: Option<u64>,
    /// fxhittime: each target's id and when it may be hit again.
    hit: Vec<(u64, f32)>,
}

/// pmotion.c:2800: the start_magic mode for a release: the throw release
/// (act 0x40000) throws (3 with the S_THROW_MAGIC level), a queued shield
/// (field_956 & 2) shields, anything else blasts the floor.
pub(crate) fn magic_mode(act_bits: u32, levels: u32, shield: bool) -> u8 {
    if act_bits & crate::paction::act::THROW_MAGIC_RELEASE != 0 {
        if levels & 0x10000 != 0 {
            3
        } else {
            2
        }
    } else if shield {
        1
    } else {
        0
    }
}

/// combat.c DamageColor: the player slot whose colour an element is.
fn damage_color(colour: u8) -> Option<usize> {
    match colour {
        1 => Some(2),
        2 => Some(1),
        3 => Some(0),
        4 => Some(3),
        _ => None,
    }
}

impl Game {
    /// InitEffects: the potion rows from WEAPONS, else POWERUPS.
    pub(crate) fn load_magic_models(
        &mut self,
        fs: &GameFs,
        cache: &mut HashMap<String, Option<AssetFile>>,
    ) {
        let dirs = ["WEAPONS".to_string(), "POWERUPS".to_string()];
        let mut known = HashMap::new();
        let mut get = |g: &mut Game, n: &str| g.item_model(fs, cache, &mut known, &dirs, n);
        let mut m = MagicModels::default();
        for c in 0..5 {
            m.blast[c] = get(self, BLAST_FX[c]);
            m.throw[c] = get(self, THROW_FX[c]);
            m.shield[c] = get(self, SHIELD_FX[c]);
        }
        m.start = get(self, START_FX);
        m.heal = get(self, HEAL_FX);
        m.heal_enemy = get(self, HEAL_ENEMY_FX);
        self.magic_models = m;
    }

    /// StartMagicPlayerFX (act 0x10000): MAGICFX on the hero.
    pub(crate) fn start_magic_player_fx(&mut self, pi: usize) {
        if let Some(m) = self.magic_models.start {
            let pos = self.players[pi].pos;
            let life = self.fx_life(Some(m), 0.0);
            let id = self.spawn(Kind::Effect { life }, Some(m), pos, 0.0);
            self.fx_follow.push(FxFollow {
                id,
                player: pi,
                local: Vec3::ZERO,
                yaw: 0.0,
            });
        }
    }

    /// start_magic: a potion of colour `flags & 0xF` (0 takes the next of
    /// randpottype) used by player `pnum` (None: a generic blast at 0.8 size
    /// and 20 power) at `pos`, thrown (mode 2, 3), as a shield (1) or on the
    /// floor (0). The hero whose colour it is gets 1.1x; level 25 adds
    /// DMG_HEAL and level 75 the MAGICHEALTH effect.
    pub(crate) fn start_magic(
        &mut self,
        pnum: Option<usize>,
        pos: Vec3,
        flags: u32,
        mode: u8,
        power_scale: f32,
    ) {
        let mut scale = 1.0;
        let mut heal = false;
        let power = match pnum {
            Some(pi) => {
                let p = &self.players[pi];
                let mut power = p.derived.magic_power * power_scale;
                if damage_color((flags & 0xF) as u8) == Some(pi) {
                    power *= 1.1;
                    scale += 0.1;
                }
                heal = p.level >= 25;
                power
            }
            None => {
                scale = 0.8;
                20.0 * power_scale
            }
        };
        let mut colour = (flags & 0xF) as u8;
        if colour == 0 {
            colour = (self.randpottype % 4) as u8 + 1;
            self.randpottype = self.randpottype.wrapping_add(1);
        }
        let colour = colour.min(4);
        let mode = if pnum.is_none() { 0 } else { mode };
        let c = colour as usize;
        match (mode, pnum) {
            (2.., Some(pi)) => {
                // pw = 1.5 throw_str + 5 (throw_str is always 0 here); the
                // bottle starts 2 ahead and 4 up, at 45 degrees.
                let p = &self.players[pi];
                let fwd = Vec3::new(p.yaw.sin(), 0.0, p.yaw.cos());
                let pw = 1.5 * 0.0 + 5.0;
                let at = pos + 2.0 * fwd + Vec3::Y * 4.0;
                let vel = Vec3::new(fwd.x * 0.707, 0.707, fwd.z * 0.707) * pw;
                let radius = 0.75 * power;
                let model = self.magic_models.throw[c];
                let entity = self.magic_entity(model, at, vel.x.atan2(vel.z), THROW_TIME, 1.0);
                self.magic.push(MagicFx {
                    owner: pnum,
                    colour,
                    stage: MagicStage::Thrown { vel },
                    damage: 40.0 * scale,
                    radius,
                    heal,
                    time: 0.0,
                    maxtime: THROW_TIME,
                    pos: at,
                    hitscale: (radius / 32.0).min(1.0),
                    entity,
                    hit: Vec::new(),
                });
            }
            (1, Some(pi)) => {
                // Rides the hero: x and z scaled by size / 12 (0.33..1).
                let radius = 0.25 * power;
                let model = self.magic_models.shield[c];
                let at = self.players[pi].pos;
                let s = (radius / 12.0).clamp(0.33, 1.0);
                let entity = self.magic_entity(model, at, 0.0, SHIELD_TIME, s);
                self.magic.push(MagicFx {
                    owner: pnum,
                    colour,
                    stage: MagicStage::Shield,
                    damage: 25.0 * scale,
                    radius,
                    heal,
                    time: 0.0,
                    maxtime: SHIELD_TIME,
                    pos: at,
                    hitscale: 1.0,
                    entity,
                    hit: Vec::new(),
                });
                self.sfx_id(POTION_SOUND[1][c]);
            }
            _ => {
                // PlaceEffectOnFloor; the effect lasts its animation.
                let at = self.floor_under(pos, 1.0);
                let model = self.magic_models.blast[c];
                let time = self.fx_life(model, 0.0);
                let entity = self.magic_entity(model, at, 0.0, time, (power / 32.0).min(1.0));
                self.magic.push(MagicFx {
                    owner: pnum,
                    colour,
                    stage: MagicStage::Blast,
                    damage: 40.0 * scale,
                    radius: power,
                    heal,
                    time: 0.0,
                    maxtime: time,
                    pos: at,
                    hitscale: 1.0,
                    entity,
                    hit: Vec::new(),
                });
                if pnum.is_some() {
                    self.sfx_id(POTION_SOUND[0][c]);
                }
            }
        }
        if let Some(pi) = pnum {
            if self.players[pi].level >= 75 && mode != 1 {
                // StartMagicHealFX, parented to the hero, at power / 32.
                if let Some(m) = self.magic_models.heal {
                    let life = self.fx_life(Some(m), 0.0);
                    let at = self.players[pi].pos;
                    let id = self.spawn(Kind::Effect { life }, Some(m), at, 0.0);
                    if let Some(e) = self.entities.iter_mut().find(|e| e.id == id) {
                        e.scale = (power / 32.0).min(1.0);
                    }
                    self.fx_follow.push(FxFollow {
                        id,
                        player: pi,
                        local: Vec3::ZERO,
                        yaw: 0.0,
                    });
                }
            }
        }
    }

    /// The Kind::Effect showing a magic effect; looping when it has a set
    /// time (StartFXTree's oneshot).
    fn magic_entity(
        &mut self,
        model: Option<usize>,
        at: Vec3,
        yaw: f32,
        life: f32,
        scale: f32,
    ) -> Option<u64> {
        let model = model?;
        let id = self.spawn(Kind::Effect { life }, Some(model), at, yaw);
        let e = self.entities.iter_mut().find(|e| e.id == id)?;
        e.scale = scale;
        Some(id)
    }

    /// FloorCollide(1 + colrad, 5 + colrad, -10): the floor under `at`,
    /// 0.1 above it, else `at`.
    fn floor_under(&self, at: Vec3, colrad: f32) -> Vec3 {
        match self.collision.ground_height(at, at.y + 5.0 + colrad) {
            Some(y) if y >= at.y - 10.0 => Vec3::new(at.x, y + 0.1, at.z),
            _ => at,
        }
    }

    /// ProcessEffects for the potion effects: the bottle flies until it hits
    /// a monster, item or wall (or its time is nearly up) and becomes the
    /// MP blast; blasts and shields damage what their radius reaches, each
    /// target again only after its fxhittime.
    pub(crate) fn update_magic(&mut self, dt: f32) {
        let mut list = std::mem::take(&mut self.magic);
        for fx in &mut list {
            fx.time += dt;
            match fx.stage {
                MagicStage::Thrown { vel } => self.magic_thrown(fx, vel, dt),
                MagicStage::Blast | MagicStage::Shield => {
                    if let (MagicStage::Shield, Some(pi)) = (fx.stage, fx.owner) {
                        if let Some(p) = self.players.get(pi) {
                            fx.pos = p.pos;
                        }
                    }
                    self.magic_spread(fx);
                }
            }
            // The entity follows its effect.
            let remaining = (fx.maxtime - fx.time).max(0.0);
            if let Some(e) = fx
                .entity
                .and_then(|id| self.entities.iter_mut().find(|e| e.id == id && e.alive))
            {
                e.pos = fx.pos;
                if let Kind::Effect { life } = &mut e.kind {
                    *life = remaining;
                }
            }
        }
        list.retain(|fx| fx.time < fx.maxtime);
        list.append(&mut self.magic);
        self.magic = list;
    }

    /// The thrown bottle (mode 0): it moves, then falls; a monster, item or
    /// critter within its colrad takes the whole damage and the bottle
    /// bursts there; so does a wall or floor within half of it, or the end
    /// of its time.
    fn magic_thrown(&mut self, fx: &mut MagicFx, mut vel: Vec3, dt: f32) {
        let from = fx.pos;
        let mut to = from + vel * dt;
        vel.y -= THROW_WEIGHT * dt;
        fx.stage = MagicStage::Thrown { vel };
        if to.y + 2.0 * THROW_COLRAD < self.level.worlds.min_bounds[1] - 25.0 {
            fx.time = fx.maxtime;
            self.end_magic_entity(fx);
            return;
        }
        // The burst's fxhittime: the MP animation's length (frames / 30).
        let blast = self.magic_models.blast[fx.colour as usize];
        let frames = blast
            .and_then(|m| self.models[m].skeleton.sequences.first())
            .map_or(0, |s| s.frame_count);
        let fade = if frames > 0 {
            (frames as f32 / 30.0).max(0.2)
        } else {
            0.25
        };
        let mut burst = false;
        if let Some(t) = self.magic_point_target(from, to, THROW_COLRAD, fx) {
            self.magic_damage(fx, t, fx.damage, fade, Vec2::new(vel.x, vel.z));
            burst = true;
        } else if let Some((at, _)) = self.collision.sweep(from, to, 0.5 * THROW_COLRAD) {
            to = at;
            burst = true;
        } else if fx.time >= fx.maxtime - 0.0667 {
            burst = true;
        }
        fx.pos = to;
        if let Some(e) = fx
            .entity
            .and_then(|id| self.entities.iter_mut().find(|e| e.id == id))
        {
            e.yaw = vel.x.atan2(vel.z);
        }
        if burst {
            // The hit morph (fxhit = magicid): the MP blast, on the floor,
            // at hitscale, for its animation, now hitting in a ring.
            self.sfx_id(POTION_SOUND[0][fx.colour as usize]);
            fx.pos = self.floor_under(fx.pos, 1.0);
            fx.stage = MagicStage::Blast;
            fx.time = 0.0;
            fx.maxtime = self.fx_life(blast, 0.0);
            // A new instance: the scene keeps an entity's first mesh.
            self.end_magic_entity(fx);
            fx.entity = self.magic_entity(blast, fx.pos, 0.0, fx.maxtime, fx.hitscale);
        }
    }

    fn end_magic_entity(&mut self, fx: &MagicFx) {
        if let Some(e) = fx
            .entity
            .and_then(|id| self.entities.iter_mut().find(|e| e.id == id))
        {
            e.alive = false;
        }
    }

    /// A blast or shield this frame: its radius and damage (mode 1 grows
    /// from 0.33 of the radius as its strength falls from 1.5 x 0.67; mode 2
    /// is constant), every monster, item and critter in it hit, each until
    /// its fxhittime.
    fn magic_spread(&mut self, fx: &mut MagicFx) {
        let remaining = fx.maxtime - fx.time;
        let (radius, scale, fade) = match fx.stage {
            MagicStage::Shield => (fx.radius, 1.0, 1.0),
            _ => {
                let phase = if fx.maxtime <= 1.0 / 30.0 {
                    1.0
                } else {
                    (remaining / fx.maxtime).max(0.0)
                };
                let age = remaining + 0.066_667;
                let fade = if age < 1.0 { 1.0 } else { age };
                if phase > 0.33 {
                    (
                        fx.radius * (0.33 + (1.0 - phase)),
                        1.5 * (phase - 0.33),
                        fade,
                    )
                } else {
                    (0.0, 0.0, fade)
                }
            }
        };
        if radius <= 0.0 {
            return;
        }
        let now = self.time;
        let pos = fx.pos;
        let targets: Vec<(usize, Vec2)> = self
            .entities
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                e.alive && !fx.hit.iter().any(|&(id, until)| id == e.id && now < until)
            })
            .filter_map(|(i, e)| {
                let (at, r) = match &e.kind {
                    Kind::Enemy { state, etype, .. } => {
                        if matches!(state, EnemyState::Dying { .. }) {
                            return None;
                        }
                        (e.pos, crate::enemies::TYPES[(*etype).min(33)].radius)
                    }
                    Kind::Generator { .. } => (e.pos, 2.0),
                    Kind::Container { chest: false, .. } => (e.pos, 1.0),
                    Kind::Critter(c) if c.hittable() => Self::aim_point(e, pos)?,
                    _ => return None,
                };
                let d = Vec2::new(at.x - pos.x, at.z - pos.z);
                (d.length() <= radius + r).then(|| (i, d.normalize_or_zero()))
            })
            .collect();
        for (i, push) in targets {
            self.magic_damage(fx, i, fx.damage * scale, fade, push);
        }
        // fn_8005BA1C on the breakable walls the ring reaches (DMG_HEAL
        // magic of a hero); SfxSkipItem then keeps the magic off them.
        if let Some(pi) = fx.owner.filter(|&pi| fx.heal && pi < self.players.len()) {
            let walls: Vec<usize> = (0..self.entities.len())
                .filter(|&i| self.effect_reaches_wall(&self.entities[i], pos, radius))
                .collect();
            for t in walls {
                self.wall_magic(pi, t);
            }
        }
    }

    /// The point missile's target: a live monster within `r` of its
    /// cylinder (MissileCollideEnemy), a generator or breakable
    /// (fn_8005ED44), or a critter's surface.
    fn magic_point_target(&self, from: Vec3, to: Vec3, r: f32, fx: &MagicFx) -> Option<usize> {
        let now = self.time;
        (0..self.entities.len()).find(|&t| {
            let e = &self.entities[t];
            if !e.alive || fx.hit.iter().any(|&(id, until)| id == e.id && now < until) {
                return false;
            }
            let seg = |p: Vec3| {
                let a = Vec2::new(from.x, from.z);
                let b = Vec2::new(to.x, to.z);
                let q = Vec2::new(p.x, p.z);
                let ab = b - a;
                let k = if ab.length_squared() > 1e-8 {
                    ((q - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                ((a + ab * k - q).length(), from.y + (to.y - from.y) * k)
            };
            match &e.kind {
                Kind::Critter(c) => {
                    if !c.hittable() {
                        return false;
                    }
                    let (surf, node) = c.surface(to, e.pos, e.yaw);
                    let t0 = &c.wad.types[c.parts[0].ty];
                    let cp = c.coll_pos(0, e.pos, e.yaw);
                    surf < r && (node.is_some() || (to.y - cp.y).abs() < t0.hht + 2.0)
                }
                Kind::Enemy { state, etype, .. } => {
                    let ty = &crate::enemies::TYPES[(*etype).min(33)];
                    let hht = ty.height * 0.5;
                    let (d, y) = seg(e.pos);
                    !matches!(state, EnemyState::Dying { .. })
                        && d < r + ty.radius
                        && (e.pos.y + hht - y).abs() < r + hht
                }
                Kind::Generator { .. } | Kind::Container { chest: false, .. } => {
                    let rad = if matches!(e.kind, Kind::Generator { .. }) {
                        2.2
                    } else {
                        1.6
                    };
                    let (d, y) = seg(e.pos);
                    d < rad && (e.pos.y + 2.0 - y).abs() < 3.5
                }
                _ => false,
            }
        })
    }

    /// damage_enemy / CritterDamage / the item hit with DMG_MAGIC and the
    /// potion's element: the target's fxhittime is set when it took more
    /// than 2, and a DMG_HEAL hit on a monster heals (do_heal_players).
    fn magic_damage(&mut self, fx: &mut MagicFx, t: usize, dmg: f32, fade: f32, push: Vec2) {
        let id = self.entities[t].id;
        if dmg > 2.0 {
            fx.hit.retain(|&(h, _)| h != id);
            fx.hit.push((id, self.time + fade));
        }
        let hp = |e: &Entity| match &e.kind {
            Kind::Enemy { hp, .. } => Some(*hp),
            _ => None,
        };
        let before = hp(&self.entities[t]);
        let at = self.entities[t].pos;
        match fx.owner.filter(|&pi| pi < self.players.len()) {
            Some(pi) => {
                MAGIC_HIT.with(|m| m.set(fx.colour as u32));
                self.hit_from = Some(fx.pos);
                self.hit_entity(pi, t, dmg * ELEMENT_BONUS, push.normalize_or_zero(), false);
                self.hit_from = None;
                MAGIC_HIT.with(|m| m.set(0));
                if fx.heal {
                    if let Some(b) = before {
                        let after = hp(&self.entities[t]).unwrap_or(0.0).max(0.0);
                        let healed = (b - after).clamp(0.0, b.max(0.0));
                        self.do_heal_players(healed, pi, at);
                    }
                }
            }
            None => {
                // A generic blast (owner 0) still hurts, with the reactions
                // and item rules of any owner-less hit.
                self.hit_from = Some(fx.pos);
                let dtype = DmgType::MAGIC | (fx.colour as u32 & 0xF);
                self.world_hit(None, t, dmg * ELEMENT_BONUS, push, dtype);
                self.hit_from = None;
            }
        }
    }

    /// do_heal_players: a level 75+ caster gets 0.1 + 0.016 per level over
    /// 75 of `amount`, and every other hero within its magic power half of
    /// that; FX_HEALENEMY plays where the monster was.
    pub(crate) fn do_heal_players(&mut self, amount: f32, pi: usize, at: Vec3) {
        let Some(p) = self.players.get(pi) else {
            return;
        };
        if p.level < 75 {
            return;
        }
        let give = amount * (0.016 * (p.level - 75) as f32 + 0.1);
        let (ppos, range) = (p.pos, p.derived.magic_power);
        for (j, q) in self.players.iter_mut().enumerate() {
            let alive = matches!(q.state, PlayerState::Normal);
            let gift = if j == pi {
                give
            } else if alive && Vec2::new(ppos.x - q.pos.x, ppos.z - q.pos.z).length() < range {
                0.5 * give
            } else {
                continue;
            };
            let cap = max_health(q.level);
            if !(gift > 0.0 && q.health >= cap) {
                q.health = (q.health + gift).min(cap);
            }
        }
        self.post(0x93, Some(pi), &[]);
        if let Some(m) = self.magic_models.heal_enemy {
            self.spawn_effect(m, at, 0.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_follow_the_release() {
        use crate::paction::act;
        assert_eq!(magic_mode(act::THROW_MAGIC_RELEASE, 0, false), 2);
        assert_eq!(magic_mode(act::THROW_MAGIC_RELEASE, 0x10000, true), 3);
        assert_eq!(magic_mode(act::MAGIC_RELEASE, 0, true), 1);
        assert_eq!(magic_mode(act::MAGIC_RELEASE, 0, false), 0);
    }

    #[test]
    fn colour_matches_its_player_slot() {
        // Player 1 is yellow (light), 2 blue (lightning), 3 red, 4 green.
        assert_eq!(damage_color(3), Some(0));
        assert_eq!(damage_color(2), Some(1));
        assert_eq!(damage_color(1), Some(2));
        assert_eq!(damage_color(4), Some(3));
        assert_eq!(damage_color(0), None);
    }

    fn dol() -> Option<gdl_formats::dol::Dol> {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game-data/main.dol");
        Some(gdl_formats::dol::Dol::new(std::fs::read(p).ok()?))
    }

    #[test]
    fn potion_sounds_match_the_dol() {
        let Some(d) = dol() else { return };
        for (row, sounds) in POTION_SOUND.iter().enumerate() {
            for (c, &id) in sounds.iter().enumerate() {
                let v = d.u32_at(0x8012_39B4 + (row * 20 + c * 4) as u32);
                assert_eq!(v, Some(id as u32), "row {row} colour {c}");
            }
        }
    }

    #[test]
    fn effect_rows_match_the_dol() {
        let Some(d) = dol() else { return };
        // magicid / throwid / shieldid by colour, then each row's name in
        // InitEffects' lbl_80122118 (40 bytes a row).
        let name = |fx: u32| d.str_at(0x8012_2118 + 40 * fx).unwrap_or_default();
        for c in 0..5u32 {
            let id = |base: u32| d.u32_at(0x8012_2088 + base + 4 * c).unwrap();
            assert_eq!(name(id(3484)), BLAST_FX[c as usize]);
            assert_eq!(name(id(3504)), THROW_FX[c as usize]);
            assert_eq!(name(id(3524)), SHIELD_FX[c as usize]);
        }
        assert_eq!(name(34), START_FX);
        assert_eq!(name(50), HEAL_ENEMY_FX);
        assert_eq!(name(51), HEAL_FX);
    }

    fn data() -> Option<GameFs> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game-data");
        GameFs::open(&root).ok()
    }

    /// A settled level A1 warrior, nothing else about, and a held grunt
    /// `at` from him: (game, grunt id).
    fn arena(at: Vec3) -> Option<(Game, u64)> {
        let fs = data()?;
        let mut g = Game::new(&fs, "levelA1", &[("WAR", "RED")]).unwrap();
        g.entities.clear();
        for _ in 0..30 {
            g.update(1.0 / 60.0, &[PlayerInput::default()]);
        }
        let class = g.classes.iter().position(|c| c.name == "GRU")?;
        let model = g.classes[class].model(1);
        let mut kind = g.new_enemy(class, 1, EnemyState::Idle);
        if let Kind::Enemy {
            cooldown,
            speed,
            hp,
            ..
        } = &mut kind
        {
            *cooldown = 99.0;
            *speed = 0.0;
            *hp = 1.0e6;
        }
        let p = g.players[0].pos;
        let id = g.spawn(kind, model, p + at, 0.0);
        Some((g, id))
    }

    fn hp(g: &Game, id: u64) -> f32 {
        g.entities
            .iter()
            .find(|e| e.id == id)
            .map_or(0.0, |e| match e.kind {
                Kind::Enemy { hp, .. } => hp,
                _ => 0.0,
            })
    }

    fn effect_model(g: &Game, fx: &MagicFx) -> Option<usize> {
        let id = fx.entity?;
        g.entities.iter().find(|e| e.id == id && e.alive)?.model
    }

    #[test]
    fn effect_models_load() {
        let Some((g, _)) = arena(Vec3::new(0.0, 0.0, 50.0)) else {
            return;
        };
        let m = &g.magic_models;
        for c in 1..5 {
            assert!(m.blast[c].is_some(), "{}", BLAST_FX[c]);
            assert!(m.throw[c].is_some(), "{}", THROW_FX[c]);
            assert!(m.shield[c].is_some(), "{}", SHIELD_FX[c]);
        }
        assert!(m.start.is_some() && m.heal.is_some() && m.heal_enemy.is_some());
    }

    #[test]
    fn a_potion_is_thrown_and_bursts_ahead() {
        // The bottle starts 2 ahead and 4 up at 3.5 a second forward and
        // up; it comes down about 3 ahead. The grunt stands where the blast
        // (radius 0.75 x magic power) reaches.
        let Some((mut g, id)) = arena(Vec3::new(0.0, 0.0, 7.0)) else {
            return;
        };
        g.players[0].yaw = 0.0;
        g.players[0].derived.magic_power = 12.0;
        g.players[0].potions = 1;
        g.players[0].potion_colours = vec![2];
        let press = PlayerInput {
            magic: true,
            magic_pressed: true,
            ..Default::default()
        };
        g.update(1.0 / 60.0, &[press]);
        let mut thrown = None;
        let mut burst = None;
        let mut started = false;
        for f in 0..240 {
            g.update(1.0 / 60.0, &[PlayerInput::default()]);
            started |= g
                .entities
                .iter()
                .any(|e| e.alive && e.model.is_some() && e.model == g.magic_models.start);
            if let Some(fx) = g.magic.first() {
                match fx.stage {
                    MagicStage::Thrown { .. } if thrown.is_none() => {
                        assert_eq!(effect_model(&g, fx), g.magic_models.throw[2]);
                        thrown = Some(f);
                    }
                    MagicStage::Blast if burst.is_none() => {
                        assert_eq!(effect_model(&g, fx), g.magic_models.blast[2]);
                        burst = Some((f, fx.pos));
                    }
                    _ => {}
                }
            }
        }
        assert!(started, "MAGICFX on the hero");
        let t = thrown.expect("the potion is thrown");
        let (b, at) = burst.expect("the bottle bursts");
        // It flies at most 0.6 s.
        assert!(b > t && b - t <= 37, "flew {} frames", b - t);
        let p = g.players[0].pos;
        assert!((at.z - p.z) > 1.5 && (at.z - p.z) < 6.0, "burst at {at:?}");
        assert_eq!(g.players[0].potions, 0);
        assert!(hp(&g, id) < 1.0e6, "the grunt is hurt");
        assert!(g.magic.is_empty(), "the blast has played out");
        // The burst's sound (hit_audio = colour + 7: S_POTION1 for
        // lightning); none on the press.
        assert!(g.sfx.iter().any(|s| s == "#6"), "{:?}", g.sfx);
        assert!(!g.sfx.iter().any(|s| s == "S_POTION1"));
    }

    #[test]
    fn the_floor_blast_spreads_and_weakens() {
        // A grunt 6 out: the ring (radius = magic power) reaches it part way.
        let Some((mut g, id)) = arena(Vec3::new(6.0, 0.0, 0.0)) else {
            return;
        };
        g.players[0].derived.magic_power = 12.0;
        let pos = g.players[0].pos;
        g.sfx.clear();
        g.start_magic(Some(0), pos, 1, 0, 1.0);
        assert!(
            g.sfx.iter().any(|s| s == "#7"),
            "S_POTION2 for fire: {:?}",
            g.sfx
        );
        let fx = g.magic[0].clone();
        assert_eq!(fx.stage, MagicStage::Blast);
        assert_eq!(effect_model(&g, &fx), g.magic_models.blast[1]);
        assert!((fx.radius - 12.0).abs() < 1e-4);
        // Red (fire) is player 3's colour, not player 1's: 40, not 44.
        assert!((fx.damage - 40.0).abs() < 1e-4);
        let total = fx.maxtime;
        let mut hits = Vec::new();
        let mut last = hp(&g, id);
        let mut t = 0.0;
        while !g.magic.is_empty() {
            g.update(1.0 / 60.0, &[PlayerInput::default()]);
            t += 1.0 / 60.0;
            let now = hp(&g, id);
            if now < last {
                hits.push((t, last - now));
            }
            last = now;
        }
        assert_eq!(hits.len(), 1, "hit once: {hits:?}");
        let (when, dmg) = hits[0];
        // Radius 12 (0.33 + 1 - phase) reaches 6 - 1.5 (the grunt's radius)
        // at phase 0.955.
        let phase = 1.0 - when / total;
        assert!((phase - 0.955).abs() < 0.03, "phase {phase}");
        // 40 x 1.5 (phase - 0.33) x 1.5 (element), level-scaled at least 1.
        let want = 40.0 * 1.5 * (phase - 0.33) * ELEMENT_BONUS;
        assert!(dmg > 0.5 * want && dmg < 2.0 * want, "{dmg} vs {want}");
    }

    #[test]
    fn the_shield_rides_and_hits_once_a_second() {
        let Some((mut g, id)) = arena(Vec3::new(2.0, 0.0, 0.0)) else {
            return;
        };
        g.players[0].derived.magic_power = 16.0;
        let pos = g.players[0].pos;
        g.sfx.clear();
        g.start_magic(Some(0), pos, 3, 1, 1.0);
        // S_SHIELD3 for light; player 1 is yellow: 1.1x power, 27.5 damage.
        assert!(g.sfx.iter().any(|s| s == "#4e"), "{:?}", g.sfx);
        let fx = g.magic[0].clone();
        assert_eq!(fx.stage, MagicStage::Shield);
        assert_eq!(effect_model(&g, &fx), g.magic_models.shield[3]);
        assert!((fx.damage - 27.5).abs() < 1e-4);
        assert!((fx.radius - 0.25 * 16.0 * 1.1).abs() < 1e-3);
        let mut hits = 0;
        let mut last = hp(&g, id);
        for _ in 0..200 {
            g.update(1.0 / 60.0, &[PlayerInput::default()]);
            let now = hp(&g, id);
            if now < last {
                hits += 1;
            }
            last = now;
        }
        assert!(g.magic.is_empty(), "the shield lasts 3 s");
        assert_eq!(hits, 3, "once a second for 3 s");
    }

    #[test]
    fn level_75_magic_heals() {
        let Some((mut g, _)) = arena(Vec3::new(3.0, 0.0, 0.0)) else {
            return;
        };
        g.players[0].level = 80;
        g.players[0].health = 100.0;
        g.players[0].derived.magic_power = 12.0;
        let pos = g.players[0].pos;
        g.start_magic(Some(0), pos, 4, 0, 1.0);
        assert!(g.magic[0].heal);
        assert!(
            g.entities
                .iter()
                .any(|e| e.alive && e.model.is_some() && e.model == g.magic_models.heal),
            "MAGICHEALTH on the hero"
        );
        while !g.magic.is_empty() {
            g.update(1.0 / 60.0, &[PlayerInput::default()]);
        }
        assert!(g.players[0].health > 100.0, "{}", g.players[0].health);
    }

    #[test]
    fn heal_shares_with_heroes_in_range() {
        let Some(fs) = data() else { return };
        let mut g = Game::new(&fs, "levelA1", &[("WAR", "RED"), ("VAL", "BLU")]).unwrap();
        g.players[0].level = 75;
        g.players[0].derived.magic_power = 10.0;
        for p in &mut g.players {
            p.health = 100.0;
        }
        let p0 = g.players[0].pos;
        g.players[1].pos = p0 + Vec3::new(3.0, 0.0, 0.0);
        g.do_heal_players(100.0, 0, p0);
        assert!((g.players[0].health - 110.0).abs() < 1e-3);
        assert!((g.players[1].health - 105.0).abs() < 1e-3);
        // Below level 75 nothing happens.
        g.players[0].level = 74;
        g.do_heal_players(100.0, 0, p0);
        assert!((g.players[0].health - 110.0).abs() < 1e-3);
    }

    #[test]
    fn colourless_potions_cycle() {
        let Some((mut g, _)) = arena(Vec3::new(50.0, 0.0, 0.0)) else {
            return;
        };
        let pos = g.players[0].pos;
        for want in [1, 2, 3, 4, 1] {
            g.start_magic(None, pos, 0, 2, 1.0);
            let fx = g.magic.last().unwrap();
            assert_eq!(fx.colour, want);
            // A generic blast is always on the floor, 0.8 size, 20 power.
            assert_eq!(fx.stage, MagicStage::Blast);
            assert!((fx.damage - 32.0).abs() < 1e-4 && (fx.radius - 20.0).abs() < 1e-4);
        }
    }
}
