//! items.c's item damage and what comes of it: fn_8005C1DC (a hit on a world
//! item: potions set off magic, food spoils, explosions destroy power-ups,
//! barrels explode or gas), the blasts StartExplosion starts and how
//! ProcessEffects' expanding mode deals their damage, carried items (gotitem)
//! thrown out in a bag (StartBagFX), monsters on hurting floors
//! (EnemyWorldDamage), ItemVisible and the generator helpers
//! (fn_8004F87C, generate_enemy's directions, place_logic12).
//!
//! A child of `game`, so it reaches the game's private state.

use super::*;

/// Item-info subtypes (items.c).
pub(crate) const SUB_BARREL: i32 = 0x2B;
pub(crate) const SUB_EXPLODE: i32 = 0x2C;
pub(crate) const SUB_POISON: i32 = 0x2D;

/// sEnemyDefaultAlgorithm (enemy.c:255): the AI routine by monster type.
pub(crate) const DEFAULT_AI: [i32; 34] = [
    2, 7, 7, 2, 7, 7, 7, 7, 7, 7, 7, 7, 2, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 0, 19, 3,
    27, 7, 7,
];

/// DMG_TYPE bits used here.
const DMG_EXPLODE: u32 = 0x400;
const DMG_NOHITFX: u32 = 0x100_0000;

/// Effect types (fx_type) of the EffectInfo rows used here (effect.rs).
const FX_EXPLODE: usize = 22;
const FX_EXPBARREL: usize = 24;
const FX_EXPPOISON: usize = 25;
const FX_EXPPOISON2: usize = 26;
const FX_EXPPOISON3: usize = 27;
const FX_EXPCHEST: usize = 29;
const FX_CHESTDEST1: usize = 31;
const FX_CHESTDEST: usize = 32;
const FX_DESTSMOKE: usize = 33;
const FX_BAG_THROW: usize = 68;
const FX_BAG_HIT: usize = 69;

/// The debris objects left by a blast (MBOX_NewObject) and the models food
/// and gold turn into.
const MODEL_NAMES: [&str; 8] = [
    "BAREXP0",
    "BARPOI0",
    "ITEMEXP0",
    "CHESTGEXP0",
    "CHESTSEXP0",
    "TREAS_JUNK",
    "BADMEAT",
    "GAPPLE",
];

/// Realm sound tables (by sMusicTrackHi): lbl_80123A74 (fn_8009D9D8, a barrel
/// explodes), lbl_80123A3C (fn_8009DA28, one gasses), lbl_80123A04
/// (fn_8009DA78, one breaks).
const EXPLODE_SFX: [i32; 16] = [
    -1, 0x250009, 0x260026, 0x270022, 0x280005, -1, -1, 0x290027, 0x2A001B, 0x2B0027, 0x2C0029,
    0x2D0027, -1, 0, -1, 0x250041,
];
const POISON_SFX: [i32; 16] = [
    -1, 0x250008, 0x260025, 0x270021, 0x280004, -1, -1, 0x290026, 0x2A001A, 0x2B0026, 0x2C0028,
    0x2D0026, -1, 0, -1, 0x250009,
];
const BREAK_SFX: [i32; 16] = [
    -1, 0x250007, 0x260024, 0x270020, 0x280003, -1, -1, 0x290025, 0x2A0019, 0x2B0025, 0x2C0027,
    0x2D0025, -1, 0, -1, 0x250008,
];
/// fn_8009EF4C: a barrel hit but not broken.
const SFX_BARREL_HIT: i32 = 60;

/// A carried item (Enemy / Critter gotitem, or a bag's additem): enough to
/// place the pickup again.
#[derive(Clone, Debug, PartialEq)]
pub struct Carry {
    pub kind: PickupKind,
    pub radius: f32,
    pub model: Option<usize>,
    /// The item info it came from, if any.
    pub info: Option<usize>,
}

/// One StartExplosion / start_magic effect dealing damage the way
/// ProcessEffects' expanding mode (effect flags 0x20) does.
#[derive(Clone, Debug)]
pub struct Blast {
    pub pos: Vec3,
    /// Effect.damage, damagetype and damageradius.
    pub damage: f32,
    pub dtype: u32,
    pub radius: f32,
    /// Effect.flags: 1 heroes, 2 items, 8 monsters (and critters).
    pub flags: u32,
    /// The player who started it (Effect.owner - 1), if any.
    pub owner: Option<usize>,
    /// endtime - now and maxtime.
    pub remaining: f32,
    pub maxtime: f32,
    /// fxmorph / fxmorph2 still to come (0x4000): the type and its morphtime
    /// (0: its animation's length).
    pub morph: Vec<(usize, f32)>,
    /// The effect entity shown.
    pub fx: Option<u64>,
    /// Critters already hit (fn_80037ED0: once per effect).
    hit: Vec<u64>,
    /// damagedelay: the damage starts this long after the effect.
    pub delay: f32,
}

impl Blast {
    /// SfxSetDamage on an expanding effect (flags 0x20) lasting `maxtime`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        pos: Vec3,
        damage: f32,
        dtype: u32,
        radius: f32,
        flags: u32,
        owner: Option<usize>,
        maxtime: f32,
        delay: f32,
        fx: Option<u64>,
    ) -> Blast {
        Blast {
            pos,
            damage,
            dtype,
            radius,
            flags,
            owner,
            remaining: maxtime,
            maxtime,
            morph: Vec::new(),
            fx,
            hit: Vec::new(),
            delay,
        }
    }
}

/// A bag thrown out of a dying monster (StartBagFX): up at 20, pitching
/// over, falling at 50 for at most 3 s; whatever it hits it becomes BAG_HIT
/// (fxhit, placed on the floor), and its item is placed where that ends.
#[derive(Clone, Debug)]
pub struct Bag {
    pub pos: Vec3,
    pub vel: Vec3,
    pub item: Carry,
    /// Seconds of BAG_HIT left once it has landed.
    pub landed: Option<f32>,
    pub fx: Option<u64>,
    /// pyrvel[0]: pi/12 + Random(pi/3) rad/s.
    pub pitch_rate: f32,
    /// endtime - now while it flies.
    pub left: f32,
}

/// The item state that rides along with the game.
#[derive(Default)]
pub struct ItemsState {
    pub blasts: Vec<Blast>,
    pub bags: Vec<Bag>,
    /// The item info each placed item came from, by entity id.
    pub(crate) info: HashMap<u64, usize>,
    /// Item health (Item.health) where it isn't kept on the entity.
    health: HashMap<u64, i32>,
    /// gotitem: what a monster or critter carries, by its entity id, and
    /// where it was last (its attention point).
    pub(crate) carried: HashMap<u64, (Carry, Option<Vec3>)>,
    /// fxhittime of monsters and items against owner-less effects.
    fxhit: HashMap<u64, f32>,
    /// The debris models above.
    models: HashMap<&'static str, Option<usize>>,
}

/// gGameOptions.items (a debug option; GDL_ITEMS): a player count every item
/// is placed for exactly, 0 for the real one.
pub(super) fn items_option() -> usize {
    std::env::var("GDL_ITEMS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

/// ItemVisible: an item's minplayers above 10 is "exactly minplayers - 10
/// players", otherwise "at least"; the Items option asks for exactly its count.
pub(crate) fn item_visible(min_players: i8, num_players: usize, items: usize) -> bool {
    let mut val = num_players as i32;
    let mut minp = min_players as i32;
    let mut eq = false;
    if minp > 10 {
        eq = true;
        minp -= 10;
    }
    if items > 0 {
        val = items as i32;
        eq = true;
    }
    if eq {
        val == minp
    } else {
        val >= minp
    }
}

/// fn_8004F87C: the spew (AI) generate_enemy hands init_enemy for a type and
/// level. RandInt(1) is always 0, so the small types' 2-or-4 is 2.
pub(crate) fn resolve_spew(etype: usize, level: i32, spew: i32) -> i32 {
    match etype {
        0 | 3 | 6 | 9 | 12 | 15 | 18 | 21 | 22 => {
            if spew != 2 && spew != 4 {
                2
            } else {
                spew
            }
        }
        2 | 7 | 14 | 17 | 24 if spew == 0 => match level {
            3 => 30,
            4 => 23,
            5 => 17,
            6 => 18,
            _ => 7,
        },
        1 | 4 | 5 | 8 | 10 | 11 | 13 | 16 | 19 | 20 | 25 | 32 | 33 if spew == 0 => match level {
            4 => 23,
            5 => 17,
            6 => 18,
            _ => 7,
        },
        27 => 31,
        30 => 3,
        31 => 27,
        29 => 19,
        _ => spew,
    }
}

/// init_enemy: the algorithm from a spew (1 is 0, 10 is 7; outside 0..31
/// the type's default).
pub(crate) fn algorithm(etype: usize, spew: i32) -> i32 {
    match spew {
        1 => 0,
        10 => 7,
        s if !(0..=31).contains(&s) => DEFAULT_AI[etype.min(33)],
        s => s,
    }
}

/// generate_enemy: how many of gendir's directions a generator's monster
/// may try, and the mask of those it can't (medium and large ones only the
/// front and +-45; 18 front and back; spew 12 the front only).
pub(crate) fn gen_dirs(etype: usize, spew: i32) -> (usize, u32) {
    let (n, mask) = match etype {
        1 | 4 | 5 | 7 | 8 | 10 | 11 | 14 | 15 | 19 | 24 | 25 => (8, 0xFFCE),
        18 => (2, 0),
        _ => (8, 0),
    };
    if spew == 12 {
        (1, mask)
    } else {
        (n, mask)
    }
}

impl Game {
    /// Load the debris objects the level's items may need (the effect rows
    /// are InitEffects', effect.rs): only levels with blasting barrels, trap
    /// chests or carried items.
    pub(super) fn load_item_fx(
        &mut self,
        fs: &GameFs,
        cache: &mut HashMap<String, Option<AssetFile>>,
        known: &mut HashMap<String, Option<usize>>,
        dirs: &[String],
    ) {
        let needed = !self.items.carried.is_empty()
            || self
                .entities
                .iter()
                .any(|e| matches!(e.kind, Kind::Critter(_)))
            || self.entities.iter().any(|e| {
                matches!(
                    e.kind,
                    Kind::Container {
                        sub: SUB_EXPLODE | SUB_POISON,
                        ..
                    }
                )
            });
        if !needed {
            return;
        }
        let mut all = dirs.to_vec();
        all.push("WEAPONS".into());
        for name in MODEL_NAMES {
            let m = self.item_model(fs, cache, known, &all, name);
            self.items.models.insert(name, m);
        }
    }

    /// fn_8005D04C / fn_8005D0C4: a golem, gargoyle or general placed over a
    /// power-up (within 2, 3 up or down) carries it; it is hidden till then.
    pub(super) fn link_carried(&mut self, carriers: &[(u64, Vec3)]) {
        for &(id, at) in carriers {
            let best = self
                .entities
                .iter()
                .enumerate()
                .filter(|(_, e)| e.alive && matches!(e.kind, Kind::Pickup { .. }))
                .filter(|(_, e)| (e.pos.y - at.y).abs() < 3.0)
                .map(|(i, e)| (i, Vec2::new(e.pos.x - at.x, e.pos.z - at.z).length()))
                .filter(|&(_, d)| d < 2.0)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(i, _)| i);
            let Some(i) = best else { continue };
            let e = self.entities.remove(i);
            if let Kind::Pickup { kind, radius } = e.kind {
                let info = self.items.info.remove(&e.id);
                let carry = Carry {
                    kind,
                    radius,
                    model: e.model,
                    info,
                };
                self.items.carried.insert(id, (carry, None));
            }
        }
    }

    /// Take what a critter or monster carries (CritterDropItem / kill_enemy).
    pub(crate) fn take_carried(&mut self, id: u64) -> Option<Carry> {
        self.items.carried.remove(&id).map(|(c, _)| c)
    }

    /// StartBagFX: throw `item` out of a bag from `pos` (scale 0: 20).
    pub(crate) fn start_bag(&mut self, pos: Vec3, item: Carry) {
        use super::effect::{BAG_SPEED, BAG_TIME};
        let fx = self.start_fx(FX_BAG_THROW, pos, 0.0, BAG_TIME);
        let pitch_rate = 0.261_799_4 + self.rng.f32() * std::f32::consts::FRAC_PI_3;
        self.items.bags.push(Bag {
            pos,
            vel: Vec3::Y * BAG_SPEED,
            item,
            landed: None,
            fx,
            pitch_rate,
            left: BAG_TIME,
        });
    }

    /// A gargoyle's GARG<kind> item, to drop in a bag (CritterDropItem).
    pub(crate) fn garg_carry(&self, kind: &str) -> Carry {
        let name = format!("GARG{}", kind.to_ascii_uppercase());
        let info = self
            .level
            .worlds
            .item_infos
            .iter()
            .position(|i| i.name.eq_ignore_ascii_case(&name));
        let pk = match info.and_then(|i| self.level.worlds.item_infos.get(i)) {
            Some(i) => pickup_kind(i, &[0u8; 16]),
            None => PickupKind::Gargoyle(match kind {
                "serp" => 0,
                "eagl" => 1,
                _ => 2,
            }),
        };
        Carry {
            kind: pk,
            radius: 2.0,
            model: self.item_model_named(&name),
            info,
        }
    }

    /// Place a carried item as a pickup on the floor below `pos` (AddItemSub).
    fn place_carry(&mut self, pos: Vec3, c: Carry) {
        let y = self
            .collision
            .ground_height(pos, pos.y + 2.0)
            .unwrap_or(pos.y);
        let id = self.spawn(
            Kind::Pickup {
                kind: c.kind,
                radius: c.radius,
            },
            c.model,
            Vec3::new(pos.x, y + 0.5, pos.z),
            0.0,
        );
        if let Some(i) = c.info {
            self.items.info.insert(id, i);
        }
    }

    /// MBOX_NewObject: leave a debris object where an item was.
    fn leave_debris(&mut self, name: &str, pos: Vec3, yaw: f32) {
        if let Some(m) = self.items.models.get(name).copied().flatten() {
            self.spawn(Kind::Prop, Some(m), pos, yaw);
        }
    }

    /// StartExplosion: a typed blast at `pos` doing `damage` — 24 the
    /// explosive barrel (radius 12, fire + knockdown + explode), 25 the poison
    /// barrel (6.5, poison gas, then POISONEXP2 for 4 s and POISONEXP3), 29 the
    /// trap chest (EXPLOSION with EXPCHEST over it), the rest radius 6. Its
    /// effect hits heroes, items and monsters (flags 0x2B).
    pub(crate) fn start_explosion(&mut self, pos: Vec3, ty: usize, damage: f32) {
        let shown = if ty == FX_EXPCHEST { FX_EXPLODE } else { ty };
        let (radius, dtype, morph) = match ty {
            FX_EXPPOISON => (
                6.5,
                DmgType::POISON_GAS,
                vec![(FX_EXPPOISON2, 4.0), (FX_EXPPOISON3, 0.0)],
            ),
            FX_EXPBARREL | FX_EXPCHEST => (12.0, 0x421, Vec::new()),
            _ => (6.0, 0x421, Vec::new()),
        };
        if ty == FX_EXPCHEST {
            self.start_fx(FX_EXPCHEST, pos, 0.0, 0.0);
        }
        let maxtime = self.fx_time(shown);
        let fx = self.start_fx(shown, pos, 0.0, maxtime + 1.0);
        self.items.blasts.push(Blast {
            pos,
            damage,
            dtype,
            radius,
            flags: 0x2B,
            owner: None,
            remaining: maxtime,
            maxtime,
            morph,
            fx,
            hit: Vec::new(),
            delay: 0.0,
        });
    }

    /// Item health, from the item info the first time.
    fn item_health(&mut self, id: u64, info: &gdl_formats::worlds::ItemInfo) -> i32 {
        *self
            .items
            .health
            .entry(id)
            .or_insert(info.hit_points as i32)
    }

    /// fn_8005C1DC for power-ups and containers: a hit of `power` with damage
    /// type `flags`. Returns the remaining health, -1 when the item takes no
    /// harm, -2 for a heavy gas hit.
    pub(crate) fn damage_item(
        &mut self,
        t: usize,
        power: f32,
        flags: u32,
        owner: Option<usize>,
    ) -> f32 {
        let e = &self.entities[t];
        if !e.alive {
            return -1.0;
        }
        let (id, pos, yaw) = (e.id, e.pos, e.yaw);
        let info = self
            .items
            .info
            .get(&id)
            .and_then(|&i| self.level.worlds.item_infos.get(i))
            .cloned();
        match self.entities[t].kind.clone() {
            Kind::Pickup { kind, .. } => {
                let Some(info) = info else {
                    // Dropped things without an info: potions still go off.
                    if let PickupKind::Potion(c) = kind {
                        self.entities[t].alive = false;
                        self.start_magic(None, pos, c as u32, 0, 0.8);
                        return 0.0;
                    }
                    return -1.0;
                };
                let mut power = power;
                let mut destroyed = false;
                let ret = if flags & DmgType::POISON_GAS != 0 {
                    let r = if power <= 2.0 { -1 } else { -2 };
                    power = 0.0;
                    r
                } else {
                    let mut health = self.item_health(id, &info);
                    if info.armor >= 0 {
                        power -= info.armor as f32;
                        if power <= 0.0 {
                            power = 1.0;
                        }
                        health = (health - power.round() as i32).max(0);
                        self.items.health.insert(id, health);
                        destroyed = health == 0;
                    }
                    if info.armor == -1 {
                        -1
                    } else {
                        health
                    }
                };
                let _ = destroyed;
                let explode = flags & DMG_EXPLODE != 0 && power >= 5.0;
                match info.subtype {
                    // A potion breaks into its magic.
                    4 => {
                        if ret == 0 {
                            self.entities[t].alive = false;
                            self.start_magic(None, pos, info.properties, 0, 0.8);
                        }
                    }
                    2 | 10..=16 => {}
                    // Gold blown up is junk worth 10.
                    1 => {
                        if explode {
                            self.start_fx(FX_CHESTDEST, pos, yaw, 0.0);
                            self.start_fx(FX_DESTSMOKE, pos, yaw, 0.0);
                            if let Some(m) = self.items.models.get("TREAS_JUNK").copied().flatten()
                            {
                                self.entities[t].model = Some(m);
                            }
                            if let Kind::Pickup { kind, .. } = &mut self.entities[t].kind {
                                *kind = PickupKind::Gold(10);
                            }
                            self.items.info.remove(&id);
                        }
                    }
                    sub => {
                        // Heavy gas spoils food: the big ones to BADMEAT (-100),
                        // the rest to GAPPLE (-50).
                        if sub == 3 && ret == -2 {
                            let health = self.item_health(id, &info);
                            let (name, v) = if health == 2 {
                                ("BADMEAT", -100.0)
                            } else {
                                ("GAPPLE", -50.0)
                            };
                            if let Some(m) = self.items.models.get(name).copied().flatten() {
                                self.entities[t].model = Some(m);
                            }
                            if let Kind::Pickup { kind, .. } = &mut self.entities[t].kind {
                                *kind = PickupKind::Food(v);
                            }
                            self.post(0x88, None, &[]);
                        }
                        // An explosion destroys the rest.
                        if explode {
                            self.start_fx(FX_CHESTDEST, pos, yaw, 0.0);
                            self.start_fx(FX_DESTSMOKE, pos, yaw, 0.0);
                            self.leave_debris("ITEMEXP0", pos, yaw);
                            self.entities[t].alive = false;
                            self.post(0x87, None, &[]);
                        }
                    }
                }
                ret as f32
            }
            Kind::Container {
                chest: true,
                hp,
                drop,
                sub,
                ..
            } => {
                // Chests have armour -1: only an explosion (power 5 and up)
                // gets in. A trap chest opens (and goes off once open); any
                // other is destroyed with what is in it (Death gets out).
                if flags & DMG_EXPLODE == 0 || power < 5.0 {
                    return -1.0;
                }
                if sub == SUB_EXPLODE {
                    if hp > 0.0 {
                        self.open_chest(t);
                        self.spawn_content(pos + Vec3::Y, drop);
                    }
                    return -1.0;
                }
                let death = drop
                    .and_then(|i| self.level.worlds.item_infos.get(i))
                    .is_some_and(|i| i.item_type == ItemType::Enemy);
                if death && hp > 0.0 {
                    self.spawn_content(pos + Vec3::Y, drop);
                }
                self.start_fx(FX_CHESTDEST1, pos, yaw, 0.0);
                self.start_fx(FX_DESTSMOKE, pos, yaw, 0.0);
                self.leave_debris(
                    if sub == 0x30 {
                        "CHESTSEXP0"
                    } else {
                        "CHESTGEXP0"
                    },
                    pos,
                    yaw,
                );
                self.entities[t].alive = false;
                -1.0
            }
            Kind::Container { armor, .. } => {
                // fn_80096FF4: magic passes barrels; armour -1 only explosions.
                if flags & DmgType::MAGIC != 0 || (armor < 0.0 && flags & DMG_EXPLODE == 0) {
                    return -1.0;
                }
                let push = Vec2::ZERO;
                match owner {
                    Some(pi) => self.hit_entity(pi, t, power, push, false),
                    None => {
                        let r = Self::apply_hit(&mut self.entities[t], power, push);
                        self.barrel_hit(t);
                        if let Some((pos, item)) = r.drop {
                            self.spawn_content(pos, item);
                        }
                    }
                }
                match self.entities[t].kind {
                    Kind::Container { hp, .. } => hp,
                    _ => -1.0,
                }
            }
            _ => -1.0,
        }
    }

    /// Open a chest (as a key does): its OPEN animation from the start.
    fn open_chest(&mut self, t: usize) {
        let e = &mut self.entities[t];
        if let Kind::Container { hp, .. } = &mut e.kind {
            *hp = 0.0;
        }
        if let Some(s) = e.model.and_then(|m| self.models[m].sequence("OPEN")) {
            e.animator.restart(s);
            e.animator.force_loop = false;
        }
    }

    /// fn_8005C1DC's type-10 / type-2 barrel branches after a hit on entity
    /// `t`: a broken explosive barrel goes off (StartExplosion 24, 30 x
    /// trap_damage) and a poison one gasses (25, 10 x trap_damage), each
    /// leaving its debris; a plain one breaks (msg 0x1B when it held
    /// something); one hit but whole knocks (fn_8009EF4C).
    pub(super) fn barrel_hit(&mut self, t: usize) {
        let e = &self.entities[t];
        let Kind::Container {
            chest: false,
            sub,
            drop,
            armor,
            ..
        } = e.kind
        else {
            return;
        };
        if sub == 0 || (armor < 0.0 && e.alive) {
            return;
        }
        let (pos, yaw) = (e.pos, e.yaw);
        if e.alive {
            self.sfx_id(SFX_BARREL_HIT);
            return;
        }
        match sub {
            SUB_EXPLODE => {
                self.start_explosion(pos, FX_EXPBARREL, 30.0 * self.tuning.trap_damage);
                let id = self.realm_sound(&EXPLODE_SFX);
                self.sfx_id(id);
                self.leave_debris("BAREXP0", pos, yaw);
            }
            SUB_POISON => {
                self.start_explosion(pos, FX_EXPPOISON, 10.0 * self.tuning.trap_damage);
                let id = self.realm_sound(&POISON_SFX);
                self.sfx_id(id);
                self.leave_debris("BARPOI0", pos, yaw);
            }
            _ => {
                let id = self.realm_sound(&BREAK_SFX);
                self.sfx_id(id);
                if sub == SUB_BARREL && drop.is_some() {
                    self.post(0x1B, None, &[]);
                }
            }
        }
    }

    /// Whether a hero's missile at `p` (radius `mr`) touches a potion lying at
    /// entity `e` (fn_8005EE18 lets missiles hit only potions among power-ups;
    /// fn_8005F0F4's cylinder of its radius and height).
    pub(super) fn missile_hits_potion(&self, e: &Entity, p: Vec3, mr: f32) -> bool {
        if !e.alive
            || !matches!(
                e.kind,
                Kind::Pickup {
                    kind: PickupKind::Potion(_),
                    ..
                }
            )
        {
            return false;
        }
        let (r, h) = self
            .items
            .info
            .get(&e.id)
            .and_then(|&i| self.level.worlds.item_infos.get(i))
            .map_or((0.5, 2.0), |i| (i.radius, i.height));
        Vec2::new(e.pos.x - p.x, e.pos.z - p.z).length() <= mr + r
            && (p.y - e.pos.y).abs() <= h + mr
    }

    /// A hit on entity `t` by something that may have no player behind it
    /// (a blast or a no-owner ring): power-ups and containers through
    /// fn_8005C1DC, everything else as damage_enemy / CritterDamage with
    /// owner -1 (no experience, no credit).
    pub(crate) fn world_hit(
        &mut self,
        owner: Option<usize>,
        t: usize,
        dmg: f32,
        push: Vec2,
        dtype: u32,
    ) {
        match (&self.entities[t].kind, owner) {
            (Kind::Pickup { .. } | Kind::Container { .. }, _) => {
                self.damage_item(t, dmg, dtype, owner);
            }
            (_, Some(pi)) if pi < self.players.len() => {
                self.effects.hit_dtype = Some(dtype);
                self.hit_entity(pi, t, dmg, push, false);
                self.effects.hit_dtype = None;
            }
            (Kind::Critter(_), _) => {
                let (pos, yaw) = (self.entities[t].pos, self.entities[t].yaw);
                let Kind::Critter(mut c) =
                    std::mem::replace(&mut self.entities[t].kind, Kind::Prop)
                else {
                    return;
                };
                if c.hittable() && !(c.is_boss() && !self.boss.as_ref().is_some_and(|b| b.active)) {
                    let from = self.hit_from.unwrap_or(pos);
                    let part = c.surface(from, pos, yaw).1.map_or(0, |n| n.0);
                    let dir = Vec3::new(push.x, 0.0, push.y);
                    self.critter_damage(&mut c, part, dmg, None, dtype, dir, None);
                }
                self.entities[t].kind = Kind::Critter(c);
            }
            (Kind::Enemy { .. }, _) => {
                let Kind::Enemy { state: was, .. } = self.entities[t].kind else {
                    return;
                };
                let r = Self::apply_hit(&mut self.entities[t], dmg, push);
                if !r.hit {
                    return;
                }
                self.enemy_hit_fx(t, dtype, r.killed);
                let dir = Vec3::new(push.x, 0.0, push.y);
                self.enemy_react(t, was, r.killed, dmg, dtype, dir);
                let e = &self.entities[t];
                if let Some(snd) = self.hit_sound(e, Self::is_destroyed(e)) {
                    self.sfx.push(snd);
                }
            }
            (Kind::Generator { .. }, _) => {
                let r = Self::apply_hit(&mut self.entities[t], dmg, push);
                let mut dropped = r.killed;
                if !r.killed {
                    let thr = 10.0 * self.tuning.gen_health;
                    let e = &mut self.entities[t];
                    if let Kind::Generator {
                        hp,
                        strength,
                        max,
                        class,
                        ..
                    } = &mut e.kind
                    {
                        let tier = if *hp <= thr {
                            1
                        } else if *hp <= 2.0 * thr {
                            2
                        } else {
                            3
                        };
                        if tier < *strength {
                            *strength = tier;
                            *max <<= 1;
                            if let Some(m) = self.classes[*class].gen_model(tier) {
                                e.model = Some(m);
                            }
                            dropped = true;
                        }
                    }
                }
                if dropped {
                    self.generator_fx(t, r.killed);
                }
                let e = &self.entities[t];
                if let Some(snd) = self.hit_sound(e, Self::is_destroyed(e)) {
                    self.sfx.push(snd);
                }
            }
            _ => {}
        }
    }

    /// place_logic12: a logic-12 generator sends its first monster 3.5 to
    /// its left, the next 3.5 to its right (each once the spot is free), and
    /// any after that stay where they appear. Each egg laid sets its bit of
    /// the generator's flags (move_logic12 case 0); a full generator sets 7.
    pub(super) fn place_logic12(&mut self, gen: u64, enemy: u64) {
        let Some(g) = self.entities.iter().position(|e| e.id == gen) else {
            return;
        };
        let (gyaw, side, max) = match &self.entities[g].kind {
            Kind::Generator { side, max, .. } => (self.entities[g].yaw, *side, *max),
            _ => return,
        };
        let Some(i) = self.entities.iter().position(|e| e.id == enemy) else {
            return;
        };
        let at = self.entities[i].pos;
        let dest = match side {
            0 | 1 => {
                let a = wrap_angle(if side == 0 {
                    gyaw + std::f32::consts::FRAC_PI_2
                } else {
                    gyaw - std::f32::consts::FRAC_PI_2
                });
                at + Vec3::new(a.sin(), 0.0, a.cos()) * 3.5
            }
            _ => at,
        };
        // check_vacancy: no other monster standing there.
        let r = match &self.entities[i].kind {
            Kind::Enemy { etype, .. } => crate::enemies::TYPES[(*etype).min(33)].radius,
            _ => 1.0,
        };
        let free = side > 1
            || !self.entities.iter().any(|e| {
                e.alive
                    && e.id != enemy
                    && matches!(&e.kind, Kind::Enemy { etype, .. }
                        if Vec2::new(e.pos.x - dest.x, e.pos.z - dest.z).length()
                            < r + crate::enemies::TYPES[(*etype).min(33)].radius)
            });
        if let Kind::Enemy { dest: d, .. } = &mut self.entities[i].kind {
            *d = Some(dest);
        }
        if free {
            if let Kind::Generator { side, chain, .. } = &mut self.entities[g].kind {
                chain.flags |= match *side {
                    0 => 1,
                    1 => 2,
                    _ => 4,
                };
                *side = side.saturating_add(1);
                if *side as usize >= max {
                    *side = 3;
                    chain.flags = 7;
                }
            }
        }
    }

    /// The item work done each frame after the entities move: blasts deal
    /// their damage, bags fly, carried items follow their carriers (and drop
    /// when they are gone), opened trap chests go off, and monsters on
    /// hurting floors and walls take EnemyWorldDamage.
    pub(super) fn update_items(&mut self, dt: f32) {
        self.update_blasts(dt);
        self.update_bags(dt);
        self.update_carried();
        self.update_trap_chests();
        self.enemy_world_damage();
    }

    /// ProcessEffects, expanding mode: over the first two thirds of its time
    /// a blast's radius grows from a third of damageradius to all of it and
    /// its damage falls from 1.5x to nothing. Heroes are hit again once
    /// their fxhittime passes (half a second for gas), monsters and items
    /// once theirs (about the rest of the effect), critters once.
    fn update_blasts(&mut self, dt: f32) {
        let mut blasts = std::mem::take(&mut self.items.blasts);
        for b in &mut blasts {
            b.remaining -= dt;
            let age = b.remaining + 0.066_667;
            let fade = if age < 1.0 { 1.0 } else { age };
            let dtime = b.maxtime - b.delay;
            let phase = if b.remaining > dtime {
                0.0
            } else if dtime <= 0.033_333_3 {
                1.0
            } else {
                b.remaining / dtime
            };
            let (radius, scale) = if phase > 0.33 && b.radius > 0.0 {
                (b.radius * (0.33 + (1.0 - phase)), 1.5 * (phase - 0.33))
            } else {
                (0.0, 0.0)
            };
            let dmg = b.damage * scale;
            if radius > 0.0 {
                self.blast_players(b, radius, dmg, age);
                self.blast_monsters(b, radius, dmg, fade);
                self.blast_items(b, radius, dmg, age);
            }
            if let Some(fx) = b.fx {
                if let Some(e) = self.entities.iter_mut().find(|e| e.id == fx) {
                    if let Kind::Effect { life } = &mut e.kind {
                        *life = b.remaining.max(0.0) + 1.0;
                    }
                }
            }
            // The effect ends: change to the next morph (0x4000), the last
            // one without damage, or go.
            if b.remaining <= 0.033_32 {
                if b.morph.is_empty() {
                    b.remaining = -1.0;
                } else {
                    let (ty, t) = b.morph.remove(0);
                    let t = if t > 0.0 { t } else { self.fx_time(ty) };
                    b.remaining = t;
                    b.maxtime = t;
                    if b.morph.is_empty() {
                        b.radius = 0.0;
                    }
                    let m = self.fx_def(ty).and_then(|d| d.model);
                    if let Some(e) =
                        b.fx.and_then(|fx| self.entities.iter_mut().find(|e| e.id == fx))
                    {
                        e.model = m;
                        e.animator = crate::character::Animator::new(0);
                    }
                }
            }
            if b.remaining < 0.0 {
                if let Some(e) =
                    b.fx.and_then(|fx| self.entities.iter_mut().find(|e| e.id == fx))
                {
                    e.alive = false;
                }
            }
        }
        blasts.retain(|b| b.remaining >= 0.0);
        blasts.append(&mut self.items.blasts);
        self.items.blasts = blasts;
    }

    /// The hero part (flags 1): within the radius plus theirs, height within
    /// the radius plus half theirs, and past 10 away not behind a wall.
    fn blast_players(&mut self, b: &Blast, radius: f32, dmg: f32, age: f32) {
        if b.flags & 1 == 0 || b.flags & 0x200 != 0 {
            return;
        }
        let dtype = if dmg < 5.0 {
            (b.dtype & !0x170) | DMG_NOHITFX
        } else {
            b.dtype
        };
        for pi in 0..self.players.len() {
            let p = &self.players[pi];
            if !p.state.active() || self.time < p.fxhittime || Some(pi) == b.owner {
                continue;
            }
            let at = p.pos + Vec3::Y * (PLAYER_HEIGHT * 0.5);
            let d = at - b.pos;
            if d.y.abs() > PLAYER_HEIGHT * 0.5 + radius {
                continue;
            }
            let dist = Vec2::new(d.x, d.z).length();
            if dist > radius + PLAYER_RADIUS {
                continue;
            }
            if dist > 10.0 && self.collision.raycast(b.pos + Vec3::Y * 0.1, at).is_some() {
                continue;
            }
            self.damage_from = Some(b.pos);
            self.damage_player(pi, dmg, dtype);
            self.damage_from = None;
            let p = &mut self.players[pi];
            if b.dtype & DmgType::POISON_GAS != 0 {
                p.fxhittime = self.time + 0.5;
            } else if dmg > 2.0 {
                p.fxhittime = self.time + age;
            }
        }
    }

    /// The monster part (flags 8): awake or asleep monsters within the
    /// radius plus theirs, then critters.
    fn blast_monsters(&mut self, b: &mut Blast, radius: f32, dmg: f32, fade: f32) {
        if b.flags & 8 == 0 {
            return;
        }
        let mut hits = Vec::new();
        for (i, e) in self.entities.iter().enumerate() {
            if !e.alive {
                continue;
            }
            match &e.kind {
                Kind::Enemy { state, etype, .. } => {
                    if matches!(state, EnemyState::Dying { .. })
                        || (matches!(state, EnemyState::Asleep) && b.flags & 0x100 != 0)
                        || self.items.fxhit.get(&e.id).is_some_and(|&t| self.time < t)
                    {
                        continue;
                    }
                    let d = Vec2::new(e.pos.x - b.pos.x, e.pos.z - b.pos.z);
                    if d.length() <= radius + crate::enemies::TYPES[(*etype).min(33)].radius {
                        hits.push((i, e.id, d.normalize_or_zero(), false));
                    }
                }
                Kind::Critter(c) if c.hittable() && !b.hit.contains(&e.id) => {
                    let Some((at, r)) = Self::aim_point(e, b.pos) else {
                        continue;
                    };
                    let d = Vec2::new(at.x - b.pos.x, at.z - b.pos.z);
                    if d.length() - r < radius {
                        hits.push((i, e.id, d.normalize_or_zero(), true));
                    }
                }
                _ => {}
            }
        }
        for (i, id, push, critter) in hits {
            if critter {
                b.hit.push(id);
            } else if dmg > 2.0 {
                self.items.fxhit.insert(id, self.time + fade);
            }
            self.hit_from = Some(b.pos);
            self.world_hit(b.owner, i, dmg, push, b.dtype);
            self.hit_from = None;
        }
    }

    /// The item part (flags 2): an explosion reaches 1.5 less far; what
    /// fn_80096FF4 skips is left alone.
    fn blast_items(&mut self, b: &Blast, radius: f32, dmg: f32, age: f32) {
        if b.flags & 2 == 0 {
            return;
        }
        let r = if b.dtype & DMG_EXPLODE != 0 {
            (radius - 1.5).max(0.0)
        } else {
            radius
        };
        if r <= 0.0 {
            return;
        }
        let mut hits = Vec::new();
        for (i, e) in self.entities.iter().enumerate() {
            if !e.alive
                || !matches!(
                    e.kind,
                    Kind::Pickup { .. } | Kind::Container { .. } | Kind::Generator { .. }
                )
            {
                continue;
            }
            if b.flags & DMG_NOHITFX == 0
                && self.items.fxhit.get(&e.id).is_some_and(|&t| self.time < t)
            {
                continue;
            }
            let info = self
                .items
                .info
                .get(&e.id)
                .and_then(|&i| self.level.worlds.item_infos.get(i));
            let (ir, ih) = info.map_or((1.0, 2.0), |i| (i.radius, i.height));
            // SfxSkipItem: 0x100 effects (magic) pass power-ups, armour -1
            // power-ups need magic or an explosion, 0x1000 ones generators.
            let skip = match &e.kind {
                Kind::Pickup { .. } => {
                    b.flags & 0x100 != 0
                        || (info.is_some_and(|i| i.armor == -1) && b.dtype & 0x600 == 0)
                }
                Kind::Generator { .. } => b.flags & 0x1000 != 0,
                _ => false,
            };
            if skip {
                continue;
            }
            let d = Vec2::new(e.pos.x - b.pos.x, e.pos.z - b.pos.z);
            if d.length() > r + ir || (e.pos.y - b.pos.y).abs() > ih + r {
                continue;
            }
            hits.push((i, e.id, d.normalize_or_zero()));
        }
        for (i, id, push) in hits {
            if !self.entities.get(i).is_some_and(|e| e.id == id && e.alive) {
                continue;
            }
            self.world_hit(b.owner, i, dmg, push, b.dtype);
            if dmg > 2.0 {
                self.items.fxhit.insert(id, self.time + age);
            }
        }
    }

    /// Bags rise, pitch and fall (ProcessEffects with flags 4): hitting the
    /// world (WeaponWallCollide, half the colrad of 1) they become BAG_HIT
    /// on the floor below (0x200000); their item is placed where that ends,
    /// or where the bag is after its 3 s.
    fn update_bags(&mut self, dt: f32) {
        use super::effect::BAG_WEIGHT;
        let mut bags = std::mem::take(&mut self.items.bags);
        let mut place = Vec::new();
        for b in &mut bags {
            match &mut b.landed {
                Some(t) => {
                    *t -= dt;
                    if *t <= 0.0 {
                        place.push((b.pos, b.item.clone()));
                    }
                }
                None => {
                    let from = b.pos;
                    let to = from + b.vel * dt;
                    b.vel.y -= BAG_WEIGHT * dt;
                    b.left -= dt;
                    let fx =
                        b.fx.and_then(|fx| self.entities.iter().position(|e| e.id == fx));
                    if let Some((at, _)) = self.collision.sweep(from, to, 0.5) {
                        // fxhit: BAG_HIT, PlaceEffectOnFloor.
                        let y = self
                            .collision
                            .ground_height(at, at.y + 1.0)
                            .map_or(at.y, |g| g + 0.1);
                        b.pos = Vec3::new(at.x, y, at.z);
                        if let Some(i) = fx {
                            self.entities[i].alive = false;
                        }
                        let t = self.fx_time(FX_BAG_HIT);
                        self.start_fx(FX_BAG_HIT, b.pos, 0.0, t);
                        b.landed = Some(t);
                    } else if b.left <= 0.033_32 || to.y < self.level.worlds.min_bounds[1] - 25.0 {
                        // Its time is up (or it fell out of the world): the
                        // item comes out where it is.
                        if let Some(i) = fx {
                            self.entities[i].alive = false;
                        }
                        b.pos = to;
                        place.push((to, b.item.clone()));
                        b.landed = Some(-1.0);
                    } else {
                        b.pos = to;
                        if let Some(i) = fx {
                            let e = &mut self.entities[i];
                            e.pos = to;
                            e.pitch += b.pitch_rate * dt;
                            if let Kind::Effect { life } = &mut e.kind {
                                *life = b.left + 1.0;
                            }
                        }
                    }
                }
            }
        }
        bags.retain(|b| b.landed.is_none_or(|t| t > 0.0));
        bags.append(&mut self.items.bags);
        self.items.bags = bags;
        for (pos, item) in place {
            self.place_carry(pos, item);
        }
    }

    /// kill_enemy: a monster carrier gone (died, or its slot taken) throws
    /// out what it carried from its attention point. (Critters drop theirs in
    /// CritterDropItem; one deleted without dying loses it.)
    fn update_carried(&mut self) {
        if self.items.carried.is_empty() {
            return;
        }
        let mut gone = Vec::new();
        for (id, (_, at)) in self.items.carried.iter_mut() {
            match self.entities.iter().find(|e| e.id == *id) {
                Some(e) if e.alive => {
                    if let Kind::Enemy { etype, .. } = &e.kind {
                        let h = crate::enemies::TYPES[(*etype).min(33)].height;
                        *at = Some(e.pos + Vec3::Y * (h * 0.5));
                    }
                }
                _ => gone.push(*id),
            }
        }
        for id in gone {
            if let Some((c, Some(at))) = self.items.carried.remove(&id) {
                self.start_bag(at, c);
            }
        }
    }

    /// items.c:4104: a trap chest that has opened (its OPEN animation done)
    /// goes off — StartExplosion 29 at 50 x trap_damage — and is gone.
    fn update_trap_chests(&mut self) {
        let done: Vec<usize> = (0..self.entities.len())
            .filter(|&i| {
                let e = &self.entities[i];
                e.alive
                    && matches!(e.kind, Kind::Container { chest: true, sub: SUB_EXPLODE, hp, .. } if hp <= 0.0)
                    && (e.animator.finished
                        || e.model.and_then(|m| self.models[m].sequence("OPEN")).is_none())
            })
            .collect();
        for i in done {
            let pos = self.entities[i].pos;
            self.entities[i].alive = false;
            self.start_explosion(pos, FX_EXPCHEST, 50.0 * self.tuning.trap_damage);
            let id = self.realm_sound(&EXPLODE_SFX);
            self.sfx_id(id);
            self.post(0x89, None, &[]);
        }
    }

    /// EnemyWorldDamage: a monster moving into a hurting wall, or over a
    /// hurting floor, takes 5 (0x10000), 5 with knockback (0x20000) or 15
    /// with knockdown (0x30000-0x50000) every move, pushed away from it;
    /// not Garm's second form.
    fn enemy_world_damage(&mut self) {
        let mut hits = Vec::new();
        for (i, e) in self.entities.iter().enumerate() {
            let Kind::Enemy {
                state,
                etype,
                recognized,
                ..
            } = &e.kind
            else {
                continue;
            };
            if !e.alive
                || *etype == 27
                || matches!(
                    state,
                    EnemyState::Dying { .. } | EnemyState::Asleep | EnemyState::Spawning
                )
                || (!*recognized && !self.on_screen(e.pos, 4.0))
            {
                continue;
            }
            let t = &crate::enemies::TYPES[(*etype).min(33)];
            let (_, wall) =
                self.collision
                    .resolve_walls_hit(e.pos, t.radius * 1.5, t.height, STEP_HEIGHT);
            let floor = self
                .collision
                .ground_hit(e.pos, e.pos.y + 2.0)
                .map(|(y, o)| (o, Vec3::new(e.pos.x, y, e.pos.z)));
            for (owner, at) in wall.into_iter().chain(floor) {
                let Some(flags) = self.hurting_surface(owner) else {
                    continue;
                };
                let (dmg, knock) = match (flags >> 16) & 0xF {
                    1 => (5.0, 0),
                    2 => (5.0, DmgType::KNOCKBACK),
                    3..=5 => (15.0, DmgType::KNOCKDOWN),
                    _ => continue,
                };
                let d = e.pos - at;
                hits.push((i, dmg, knock, Vec2::new(d.x, d.z).normalize_or_zero()));
            }
        }
        for (i, dmg, knock, push) in hits {
            if self.entities[i].alive {
                self.world_hit(None, i, dmg, push, knock);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_visible_counts() {
        // At least N.
        assert!(item_visible(2, 2, 0));
        assert!(item_visible(2, 3, 0));
        assert!(!item_visible(3, 2, 0));
        // Above 10: exactly N - 10.
        assert!(item_visible(12, 2, 0));
        assert!(!item_visible(12, 3, 0));
        assert!(!item_visible(12, 1, 0));
        // The Items option asks for exactly its count.
        assert!(item_visible(3, 1, 3));
        assert!(!item_visible(2, 4, 3));
        assert!(item_visible(13, 1, 3));
        assert!(item_visible(0, 1, 0));
    }

    #[test]
    fn spawn_ai_and_directions() {
        // Small types always come out as 2 (or 4), Death 3, IT 27.
        assert_eq!(resolve_spew(0, 1, 7), 2);
        assert_eq!(resolve_spew(3, 1, 4), 4);
        assert_eq!(resolve_spew(30, 1, 0), 3);
        assert_eq!(resolve_spew(31, 1, 0), 27);
        // Casters at level 3 get 30, archers 23, the rest keep theirs.
        assert_eq!(resolve_spew(7, 3, 0), 30);
        assert_eq!(resolve_spew(4, 4, 0), 23);
        assert_eq!(resolve_spew(4, 1, 12), 12);
        assert_eq!(algorithm(4, 1), 0);
        assert_eq!(algorithm(4, 10), 7);
        assert_eq!(algorithm(4, -1), DEFAULT_AI[4]);
        assert_eq!(algorithm(4, 99), 7);
        // Grunts: front and +-45 only; logic 12 straight ahead; 18 two ways.
        assert_eq!(gen_dirs(4, 7), (8, 0xFFCE));
        assert_eq!(gen_dirs(4, 12).0, 1);
        assert_eq!(gen_dirs(18, 2), (2, 0));
        assert_eq!(gen_dirs(3, 2), (8, 0));
    }

    fn data() -> Option<GameFs> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game-data");
        GameFs::open(&root).ok()
    }

    /// A level with no monsters about, the hero at its start.
    fn quiet(fs: &GameFs, want: &[fn(&Kind) -> bool]) -> Game {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../game-data/files/Gauntlet/LEVELS");
        let mut levels: Vec<String> = std::fs::read_dir(root)
            .unwrap()
            .filter_map(|d| d.ok()?.file_name().into_string().ok())
            .filter(|n| n.starts_with("level"))
            .collect();
        levels.sort();
        for l in levels {
            let Ok(mut g) = Game::new(fs, &l, &[("WAR", "RED")]) else {
                continue;
            };
            if !want
                .iter()
                .all(|f| g.entities.iter().any(|e| e.alive && f(&e.kind)))
            {
                continue;
            }
            g.pending_enemies.clear();
            g.items.carried.clear();
            g.entities.retain(|e| {
                !matches!(
                    e.kind,
                    Kind::Enemy { .. } | Kind::Generator { .. } | Kind::Critter(_)
                )
            });
            g.players[0].health = 10_000.0;
            return g;
        }
        panic!("no level has them all");
    }

    fn is_exp(k: &Kind) -> bool {
        matches!(
            k,
            Kind::Container {
                sub: SUB_EXPLODE,
                chest: false,
                ..
            }
        )
    }
    fn is_poi(k: &Kind) -> bool {
        matches!(
            k,
            Kind::Container {
                sub: SUB_POISON,
                chest: false,
                ..
            }
        )
    }
    fn is_pot(k: &Kind) -> bool {
        matches!(
            k,
            Kind::Pickup {
                kind: PickupKind::Potion(_),
                ..
            }
        )
    }
    fn is_food(k: &Kind) -> bool {
        matches!(k, Kind::Pickup { kind: PickupKind::Food(v), .. } if *v > 0.0)
    }
    fn is_big_food(k: &Kind) -> bool {
        matches!(k, Kind::Pickup { kind: PickupKind::Food(v), .. } if *v >= 100.0)
    }

    fn step(g: &mut Game, secs: f32) {
        for _ in 0..(secs * 60.0) as usize {
            g.update(1.0 / 60.0, &[PlayerInput::default()]);
        }
    }

    /// A copy of the level's first item matching `pred`, moved to `at`.
    fn copy_item(g: &mut Game, pred: impl Fn(&Kind) -> bool, at: Vec3) -> Option<u64> {
        let i = g.entities.iter().position(|e| e.alive && pred(&e.kind))?;
        let (kind, model, solid) = (
            g.entities[i].kind.clone(),
            g.entities[i].model,
            g.entities[i].solid,
        );
        let info = g.items.info.get(&g.entities[i].id).copied();
        let id = g.spawn(kind, model, at, 0.0);
        if let Some(s) = solid {
            g.set_solid(id, s);
        }
        if let Some(n) = info {
            g.items.info.insert(id, n);
        }
        Some(id)
    }

    fn grunt(g: &mut Game, fs: &GameFs, at: Vec3) -> u64 {
        let mut cache = HashMap::new();
        let c = g
            .monster_class(fs, &mut cache, crate::enemies::GRUNT)
            .unwrap();
        let mut kind = g.new_enemy(c, 1, EnemyState::Idle);
        if let Kind::Enemy { recognized, .. } = &mut kind {
            *recognized = true;
        }
        let m = g.classes[c].model(1);
        g.spawn(kind, m, at, 0.0)
    }

    fn alive(g: &Game, id: u64) -> bool {
        g.entities.iter().any(|e| e.id == id && e.alive)
    }

    #[test]
    fn explosive_barrels_blow_up_and_set_each_other_off() {
        let Some(fs) = data() else { return };
        let mut g = quiet(&fs, &[is_exp]);
        let at = g.players[0].pos;
        let a =
            copy_item(&mut g, is_exp, at + Vec3::new(8.0, 0.0, 0.0)).expect("an explosive barrel");
        let b = copy_item(&mut g, is_exp, at + Vec3::new(14.0, 0.0, 0.0)).unwrap();
        let mon = grunt(&mut g, &fs, at + Vec3::new(8.0, 0.0, 4.0));
        g.players[0].pos = at + Vec3::new(2.0, 0.0, 0.0);
        let before = g.players[0].health;
        let t = g.entities.iter().position(|e| e.id == a).unwrap();
        g.hit_entity(0, t, 50.0, Vec2::X, false);
        assert!(!alive(&g, a), "the barrel breaks");
        assert_eq!(g.items.blasts.len(), 1);
        assert!((g.items.blasts[0].damage - 30.0 * g.tuning.trap_damage).abs() < 1e-3);
        assert_eq!(g.items.blasts[0].radius, 12.0);
        // Its debris stays.
        let debris = g.items.models.get("BAREXP0").copied().flatten();
        assert!(debris.is_some() && g.entities.iter().any(|e| e.model == debris));
        step(&mut g, 2.0);
        assert!(!alive(&g, b), "the blast sets the next barrel off");
        assert!(!alive(&g, mon), "the grunt beside it dies");
        assert!(g.players[0].health < before, "the hero 6 away is hurt");
    }

    #[test]
    fn poison_gas_hurts_over_time_and_spoils_food() {
        let Some(fs) = data() else { return };
        let mut g = quiet(&fs, &[is_poi, is_big_food]);
        let at = g.players[0].pos;
        let a = copy_item(&mut g, is_poi, at + Vec3::new(3.0, 0.0, 0.0)).expect("a poison barrel");
        // A big food (2 hit points) by it.
        let food = copy_item(&mut g, is_big_food, at + Vec3::new(4.0, 0.0, 1.0));
        let t = g.entities.iter().position(|e| e.id == a).unwrap();
        g.world_hit(None, t, 50.0, Vec2::X, 0);
        assert_eq!(g.items.blasts.len(), 1);
        assert_eq!(g.items.blasts[0].dtype, DmgType::POISON_GAS);
        let mut hurts = 0;
        let mut last = g.players[0].health;
        for _ in 0..(6 * 60) {
            g.players[0].pos = at;
            g.update(1.0 / 60.0, &[PlayerInput::default()]);
            if g.players[0].health < last {
                hurts += 1;
            }
            last = g.players[0].health;
        }
        assert!(hurts >= 3, "gas hurts again every half second: {hurts}");
        assert!(g.items.blasts.is_empty(), "the cloud is gone");
        if let Some(f) = food {
            let e = g
                .entities
                .iter()
                .find(|e| e.id == f)
                .expect("the food stays");
            assert!(
                matches!(e.kind, Kind::Pickup { kind: PickupKind::Food(v), .. } if v == -100.0)
            );
        }
    }

    #[test]
    fn explosions_destroy_power_ups_and_shots_set_potions_off() {
        let Some(fs) = data() else { return };
        let mut g = quiet(&fs, &[is_food, is_pot]);
        let at = g.players[0].pos;
        let food = copy_item(&mut g, is_food, at + Vec3::new(20.0, 0.0, 0.0)).expect("food");
        let t = g.entities.iter().position(|e| e.id == food).unwrap();
        g.damage_item(t, 10.0, DMG_EXPLODE, None);
        assert!(!alive(&g, food), "an explosion destroys food");
        // A shot potion goes off as generic magic.
        let pot = copy_item(&mut g, is_pot, at + Vec3::new(0.0, 0.0, 6.0)).expect("a potion");
        let magic = g.magic.len();
        g.spawn(
            Kind::Projectile {
                owner: 0,
                vel: Vec3::Z * 30.0,
                life: 2.0,
                damage: 10.0,
                weight: 0.0,
                radius: 0.6,
                tumble: 0.0,
                reflect: false,
                pierced: None,
                wall_sound: WallSound::None,
                hold: 0.0,
                morph: None,
            },
            None,
            at + Vec3::new(0.0, 1.0, 3.0),
            0.0,
        );
        step(&mut g, 0.3);
        assert!(!alive(&g, pot), "the shot breaks the potion");
        assert!(g.magic.len() > magic, "and sets off its magic");
    }

    #[test]
    fn carried_items_come_out_in_a_bag() {
        let Some(fs) = data() else { return };
        let mut g = quiet(&fs, &[is_food]);
        let at = g.players[0].pos + Vec3::new(5.0, 0.0, 0.0);
        let food = copy_item(&mut g, is_food, at + Vec3::new(0.5, 0.0, 0.5)).expect("food");
        let carrier = grunt(&mut g, &fs, at);
        g.link_carried(&[(carrier, at)]);
        // Seen alive once (its attention point), then it dies.
        g.update_carried();
        assert!(
            !g.entities.iter().any(|e| e.id == food),
            "the carried item is hidden"
        );
        assert!(g.items.carried.contains_key(&carrier));
        let pickups = |g: &Game| {
            g.entities
                .iter()
                .filter(|e| matches!(e.kind, Kind::Pickup { .. }))
                .count()
        };
        let n = pickups(&g);
        // kill_enemy: the carrier gone, a bag flies and leaves the item.
        g.entities.retain(|e| e.id != carrier);
        step(&mut g, 0.1);
        assert_eq!(g.items.bags.len(), 1);
        assert!(g.items.bags[0].pos.y > at.y, "the bag rises");
        step(&mut g, 3.0);
        assert!(g.items.bags.is_empty());
        assert_eq!(pickups(&g), n + 1, "the item is back on the floor");
    }

    fn is_trap_chest(k: &Kind) -> bool {
        matches!(
            k,
            Kind::Container {
                sub: SUB_EXPLODE,
                chest: true,
                ..
            }
        )
    }

    #[test]
    fn trap_chests_go_off_once_open() {
        let Some(fs) = data() else { return };
        let mut g = quiet(&fs, &[is_trap_chest]);
        let at = g.players[0].pos + Vec3::new(30.0, 0.0, 0.0);
        let c = copy_item(&mut g, is_trap_chest, at).unwrap();
        let t = g.entities.iter().position(|e| e.id == c).unwrap();
        // A plain hit does nothing to a chest; an explosion opens it.
        g.damage_item(t, 50.0, 0, None);
        assert!(matches!(g.entities[t].kind, Kind::Container { hp, .. } if hp > 0.0));
        g.damage_item(t, 10.0, DMG_EXPLODE, None);
        let mut blast = None;
        for _ in 0..180 {
            g.update(1.0 / 60.0, &[PlayerInput::default()]);
            if let Some(b) = g.items.blasts.first() {
                blast.get_or_insert((b.damage, b.radius));
            }
        }
        assert!(!alive(&g, c), "the open trap chest is gone");
        let (dmg, r) = blast.expect("it went off");
        assert!((dmg - 50.0 * g.tuning.trap_damage).abs() < 1e-3);
        assert_eq!(r, 12.0);
    }

    #[test]
    fn placed_monsters_take_their_ai_and_sleep_at_strength_0() {
        let Some(fs) = data() else { return };
        let mut seen = 0;
        for l in ["levelA1", "levelB1", "levelC1", "levelD1"] {
            let g = Game::new(&fs, l, &[("WAR", "RED")]).unwrap();
            for p in &g.pending_enemies {
                let Kind::Enemy {
                    ai,
                    state,
                    cooldown,
                    etype,
                    ..
                } = &p.kind
                else {
                    continue;
                };
                assert!((0..=31).contains(ai), "{l}: ai {ai}");
                if *state == EnemyState::Asleep {
                    assert_eq!(*cooldown, 0.0, "dormant ones aren't stunned");
                    assert!(*etype != crate::enemies::DEATH && *etype != crate::enemies::IT);
                } else if *etype != crate::enemies::DEATH {
                    assert!(*cooldown == 0.5 || *cooldown == 0.0);
                }
                seen += 1;
            }
        }
        assert!(seen > 0);
    }

    #[test]
    fn monsters_burn_on_hurting_floors() {
        let Some(fs) = data() else { return };
        let mut g = Game::new(&fs, "levelH1", &[("WAR", "RED")]).unwrap();
        g.pending_enemies.clear();
        let tri = g
            .collision
            .tris
            .iter()
            .find(|t| {
                t.n.y > 0.9
                    && t.owner >= 0
                    && g.obj_flags
                        .get(t.owner as usize)
                        .is_some_and(|f| f & 0xF0000 == 0x10000 && f & 0x2000000 == 0)
            })
            .copied()
            .expect("a burning floor");
        let pos = (tri.v[0] + tri.v[1] + tri.v[2]) / 3.0;
        let mon = grunt(&mut g, &fs, pos + Vec3::Y * 0.05);
        let hp = |g: &Game| match g.entities.iter().find(|e| e.id == mon).map(|e| &e.kind) {
            Some(Kind::Enemy { hp, .. }) => *hp,
            _ => 0.0,
        };
        let before = hp(&g);
        g.enemy_world_damage();
        assert_eq!(hp(&g), before - 5.0, "5 a move on a 0x10000 floor");
    }

    #[test]
    fn logic_12_generators_send_monsters_left_then_right() {
        let Some(fs) = data() else { return };
        let mut g = quiet(&fs, &[]);
        let at = g.players[0].pos;
        let mut cache = HashMap::new();
        let c = g
            .monster_class(&fs, &mut cache, crate::enemies::GRUNT)
            .unwrap();
        let gen = g.spawn(
            Kind::Generator {
                class: c,
                strength: 1,
                hp: 10.0,
                timer: 0.0,
                interval: 10.0,
                max: 5,
                spawned: Vec::new(),
                etype: crate::enemies::GRUNT,
                genratio: 0.0,
                always: true,
                special: None,
                ai: 12,
                height: 5.0,
                side: 0,
                chain: Default::default(),
            },
            None,
            at + Vec3::new(0.0, 0.0, 10.0),
            0.0,
        );
        let mut dests = Vec::new();
        for k in 0..3 {
            let kind = g.new_enemy(c, 1, EnemyState::Idle);
            let a = g.spawn(kind, None, at + Vec3::new(30.0 * k as f32, 0.0, 15.0), 0.0);
            g.place_logic12(gen, a);
            let e = g.entities.iter().find(|e| e.id == a).unwrap();
            match &e.kind {
                Kind::Enemy { dest: Some(d), .. } => dests.push(*d - e.pos),
                _ => panic!("no dest"),
            }
        }
        // Facing +z: left (+90) is +x, right -x, then where it stands.
        assert!(
            (dests[0] - Vec3::new(3.5, 0.0, 0.0)).length() < 1e-3,
            "{:?}",
            dests[0]
        );
        assert!(
            (dests[1] - Vec3::new(-3.5, 0.0, 0.0)).length() < 1e-3,
            "{:?}",
            dests[1]
        );
        assert!(dests[2].length() < 1e-3);
    }

    /// A grunt generator with gendata ai `ai` 8 in front of the hero.
    fn generator(g: &mut Game, fs: &GameFs, ai: i32) -> (u64, usize) {
        let at = g.players[0].pos;
        let mut cache = HashMap::new();
        let c = g
            .monster_class(fs, &mut cache, crate::enemies::GRUNT)
            .unwrap();
        let gen = g.spawn(
            Kind::Generator {
                class: c,
                strength: 1,
                hp: 1000.0,
                timer: 1e9,
                interval: 10.0,
                max: 5,
                spawned: Vec::new(),
                etype: crate::enemies::GRUNT,
                genratio: 0.0,
                always: true,
                special: None,
                ai,
                height: 2.0,
                side: 0,
                chain: Default::default(),
            },
            None,
            at + Vec3::new(0.0, 0.0, 8.0),
            0.0,
        );
        (gen, c)
    }

    fn gen_of(g: &Game, gen: u64) -> &Kind {
        &g.entities.iter().find(|e| e.id == gen).unwrap().kind
    }

    #[test]
    fn hitting_an_egg_hatches_the_clutch() {
        let Some(fs) = data() else { return };
        let mut g = quiet(&fs, &[]);
        let (gen, c) = generator(&mut g, &fs, 12);
        let at = g.players[0].pos + Vec3::new(0.0, 0.0, 6.0);
        let mut kind = g.new_enemy(c, 1, EnemyState::Idle);
        if let Kind::Enemy {
            ai, brain, speed, ..
        } = &mut kind
        {
            *ai = 12;
            *speed = 0.0;
            brain.generator = Some(gen);
        }
        let model = g.classes[c].model(1);
        let egg = g.spawn(kind, model, at, 0.0);
        g.place_logic12(gen, egg);
        let mode1 = |g: &Game| match &g.entities.iter().find(|e| e.id == egg).unwrap().kind {
            Kind::Enemy { brain, .. } => brain.mode1(),
            _ => -1,
        };
        for _ in 0..10 {
            g.update(1.0 / 60.0, &[PlayerInput::default()]);
        }
        // One egg laid (the left one): it waits.
        assert!(matches!(gen_of(&g, gen), Kind::Generator { chain, .. } if chain.flags == 1));
        assert_eq!(mode1(&g), 1);
        let t = g.entities.iter().position(|e| e.id == egg).unwrap();
        if let Kind::Enemy { hp, .. } = &mut g.entities[t].kind {
            *hp = 100.0;
        }
        g.hit_entity(0, t, 1.0, Vec2::Y, false);
        assert!(matches!(gen_of(&g, gen), Kind::Generator { chain, .. }
            if chain.flags == 7 && chain.generated == 3));
        for _ in 0..30 {
            g.update(1.0 / 60.0, &[PlayerInput::default()]);
        }
        assert_eq!(mode1(&g), 2, "hatched");
    }

    #[test]
    fn a_hit_lookout_turns_its_generator_ordinary() {
        let Some(fs) = data() else { return };
        let mut g = quiet(&fs, &[]);
        let (gen, _) = generator(&mut g, &fs, 15);
        g.update(1.0 / 60.0, &[PlayerInput::default()]);
        g.update(1.0 / 60.0, &[PlayerInput::default()]);
        // generate_single: one lookout, at once and only one.
        let Kind::Generator { spawned, .. } = gen_of(&g, gen) else {
            panic!()
        };
        assert_eq!(spawned.len(), 1);
        let id = spawned[0];
        let t = g.entities.iter().position(|e| e.id == id).unwrap();
        assert!(matches!(&g.entities[t].kind, Kind::Enemy { ai: 15, .. }));
        for _ in 0..30 {
            g.update(1.0 / 60.0, &[PlayerInput::default()]);
        }
        assert!(matches!(gen_of(&g, gen), Kind::Generator { spawned, .. } if spawned.len() == 1));
        let t = g.entities.iter().position(|e| e.id == id).unwrap();
        g.hit_entity(0, t, 1000.0, Vec2::Y, false);
        assert!(
            matches!(gen_of(&g, gen), Kind::Generator { ai: 0, spawned, .. }
            if spawned.is_empty())
        );
    }
}
