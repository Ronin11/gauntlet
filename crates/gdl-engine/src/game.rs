//! Game state and simulation (independent of rendering).

use crate::character::{Animator, Model};
use crate::collision::{Collision, Tri};
use crate::fs::GameFs;
use crate::input::PlayerInput;
use crate::level::Level;
use crate::triggers::{trigger_flags, Mover, TriggerMode};
use anyhow::Result;
use gdl_formats::anim::Anim;
use gdl_formats::worlds::ItemType;
use gdl_formats::Objects;
use glam::{Vec2, Vec3};
use std::collections::{HashMap, HashSet};

pub const PLAYER_CLASSES: [&str; 4] = ["WAR", "VAL", "WIZ", "ARC"];
pub const PLAYER_COLORS: [&str; 4] = ["RED", "BLU", "YEL", "GRE"];

const GRAVITY: f32 = 60.0;
const STEP_HEIGHT: f32 = 1.2;
/// PDATA width and height (col_radius 1.5, height 5).
const PLAYER_RADIUS: f32 = 1.5;
const PLAYER_HEIGHT: f32 = 5.0;
/// PlayerGetTarget: the farthest a target is looked for, and the melee reach
/// (2 plus the player's radius, from the target's surface).
const TARGET_RANGE: f32 = 30.0;
const MELEE_REACH: f32 = 2.0 + PLAYER_RADIUS;
/// The aim cone: dot >= 0.707 (0.5 on Easy).
const AIM_CONE: f32 = 0.707;
/// The potion ring's damage at full strength, and how long it spreads (s).
const MAGIC_DAMAGE: f32 = 40.0;
const MAGIC_TIME: f32 = 1.0;
/// Turbo meter: fill per second, and the costs of the PWRB / PWRC moves.
const TURBO_FILL: f32 = 2.0;
/// player_max_health: 100 per level above 1 plus 500, at most 9999. A new
/// character starts full at level 1.
pub fn max_health(level: u32) -> f32 {
    (100.0 * (level.max(1) - 1) as f32 + 500.0).min(9999.0)
}
/// How far in front of the camera focus (towards the camera) geometry is kept.
const CUTAWAY_DEPTH: f32 = 12.0;
/// Thrown weapons fly until they hit something; this only bounds strays.
const PROJECTILE_LIFE: f32 = 3.0;
/// Players are kept within this distance of the group's centre (stay on screen).
const PARTY_RADIUS: f32 = 13.0;

/// Small deterministic RNG (xorshift) so simulations are reproducible.
#[derive(Clone)]
pub struct Rng(u64);
impl Rng {
    pub fn next_u32(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 16) as u32
    }
    pub fn f32(&mut self) -> f32 {
        (self.next_u32() & 0xFFFFFF) as f32 / 16_777_216.0
    }
    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.f32()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PlayerState {
    /// In play; what the hero does is the action machine's (`Player::act`).
    Normal,
    Dead {
        timer: f32,
    },
    /// do_exit: sinking and spinning into an exit (50 ticks), then gone.
    Exiting {
        timer: f32,
    },
    /// Left the level through an exit.
    Exited,
    /// Dead and out of the level: the panel reads "IN TOWER" (state 0xB).
    InTower,
}

/// A character's running totals (PlayerShopStat).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlayStats {
    pub enemies_killed: u32,
    pub generators_destroyed: u32,
    pub gold_found: u32,
    /// Seconds played.
    pub playtime: f32,
}

/// What a player keeps from one level to the next (player_store_in_save).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Carried {
    pub health: f32,
    pub gold: u32,
    pub keys: u32,
    pub potions: Vec<u8>,
    pub level: u32,
    pub exp: u32,
    pub bonus: crate::stats::Atts,
    pub powerups: crate::powerups::Powerups,
    pub stats: PlayStats,
    /// The experience at Sumner's last level-up check.
    pub sumner_exp: u32,
}

impl Carried {
    /// A new character: level 1, full health, nothing else.
    pub fn new_character() -> Carried {
        Carried {
            health: max_health(1),
            level: 1,
            ..Default::default()
        }
    }
}

impl Player {
    pub fn carried(&self) -> Carried {
        Carried {
            health: self.health.max(1.0),
            gold: self.gold,
            keys: self.keys,
            potions: self.potion_colours.clone(),
            level: self.level,
            exp: self.exp,
            bonus: self.bonus,
            powerups: self.powerups.clone(),
            stats: self.stats,
            sumner_exp: self.sumner_exp,
        }
    }

    pub fn restore(&mut self, c: &Carried) {
        self.health = c.health;
        self.gold = c.gold;
        self.keys = c.keys;
        self.potions = c.potions.len() as u32;
        self.potion_colours = c.potions.clone();
        self.level = c.level.max(1);
        self.exp = c.exp;
        self.bonus = c.bonus;
        self.powerups = c.powerups.clone();
        self.stats = c.stats;
        self.sumner_exp = c.sumner_exp;
        self.refresh_stats();
    }

    /// PlayerUpdateAtts + player_scale_att for the current level.
    pub fn refresh_stats(&mut self) {
        let a = crate::stats::atts(self.class_id, self.level, self.bonus);
        self.derived = crate::stats::derived(self.class_id, a);
    }
}

impl PlayerState {
    /// In play: walking or attacking (state 1).
    pub fn active(&self) -> bool {
        matches!(self, PlayerState::Normal)
    }
}

/// A scroll's or a tower story message's text on screen (ControllerMessageBox):
/// one page per line, each closed with B.
#[derive(Clone, Debug)]
pub struct ScrollBox {
    /// The player who picked it up; None lets any player close it.
    pub player: Option<usize>,
    pub pages: Vec<String>,
    pub page: usize,
    /// The message's text scale (0.5 for scrolls, 0.6 for the story).
    pub scale: f32,
    /// Ticks the page has shown; input is ignored for the first 15.
    pub ticks: u32,
}

impl ScrollBox {
    pub fn new(player: Option<usize>, pages: Vec<String>, scale: f32) -> ScrollBox {
        ScrollBox {
            player,
            // The box draws "\r" page breaks as nothing.
            pages: pages.into_iter().map(|p| p.replace('\r', "")).collect(),
            page: 0,
            scale,
            ticks: 0,
        }
    }

    pub fn text(&self) -> &str {
        self.pages.get(self.page).map_or("", |s| s.as_str())
    }

    /// One tick; true once the last page is closed with B.
    pub fn update(&mut self, inputs: &[PlayerInput]) -> bool {
        self.ticks += 1;
        let pressed = match self.player {
            Some(p) => inputs.get(p).is_some_and(|i| i.b_pressed),
            None => inputs.iter().any(|i| i.b_pressed),
        };
        if self.ticks > 15 && pressed {
            self.page += 1;
            self.ticks = 0;
        }
        self.page >= self.pages.len()
    }
}

/// Damage-type flags (DMG_TYPE).
pub struct DmgType;
impl DmgType {
    pub const MAGIC: u32 = 0x200;
    pub const POISON_GAS: u32 = 0x800;
    /// Death's drain: armour doesn't stop it.
    pub const NO_ARMOR: u32 = 0x1000;
    pub const KNOCKBACK: u32 = 0x10;
    pub const KNOCKDOWN: u32 = 0x20;
    pub const BLOWNAWAY: u32 = 0x40;
    pub const STUN: u32 = 0x80;
    pub const KNOCKOVER: u32 = 0x100;
    pub const SPIKE: u32 = 0x2000;
    pub const WHIRLWIND: u32 = 0x10000;
    /// A low blow (short monsters): levitation dodges it.
    pub const LOW: u32 = 0x4000_0000;
}

/// ModifyDamage: an elemental hit on a target with no affinity does 1.5x. Every
/// potion, turbo move and amulet hit has an element; monsters here are neutral.
const ELEMENT_BONUS: f32 = 1.5;

/// A spreading potion blast.
#[derive(Clone, Debug)]
pub struct Ring {
    pub owner: usize,
    pub pos: Vec3,
    pub radius: f32,
    pub damage: f32,
    pub colour: u8,
    pub time: f32,
    /// Entities already hit.
    hit: Vec<u64>,
}

/// A monster's missile to launch.
#[derive(Clone, Copy, Debug)]
struct Shot {
    from: Vec3,
    to: Vec3,
    etype: usize,
    /// 0 arrow, 1 bomb, 2 fireball.
    slot: usize,
    class: usize,
}

/// EnemyMissileInfo: (damage, speed, burst radius, gravity) of a type's arrow,
/// bomb and fireball.
fn missile(etype: usize, slot: usize) -> (f32, f32, f32, f32) {
    match (etype, slot) {
        (17, 0) => (5.0, 15.0, 0.0, 0.0),
        (17, 1) => (10.0, 20.0, 0.0, 0.0),
        (17, 2) => (15.0, 25.0, 0.0, 0.0),
        (2 | 20 | 14, 2) => (15.0, 25.0, 0.0, 0.0),
        (7 | 24, 2) => (20.0, 20.0, 0.0, 0.0),
        (27, 2) => (25.0, 80.0, 0.0, 0.0),
        (_, 1) => (10.0, 20.0, 3.0, 20.0),
        (_, 0) => (10.0, 25.0, 0.0, 15.0),
        _ => (15.0, 25.0, 0.0, 0.0),
    }
}

/// Level-3 demons, sorcerers, plague, worms and warlocks cast fireballs.
fn caster(etype: usize, strength: usize) -> bool {
    strength == 3 && matches!(etype, 2 | 7 | 14 | 17 | 24)
}

/// A generator's request for a monster.
#[derive(Clone, Copy, Debug)]
struct Spawn {
    class: usize,
    strength: usize,
    pos: Vec3,
    yaw: f32,
    generator: u64,
    /// Every slot is taken: recycle the farthest monster off screen.
    recycle: bool,
}

thread_local! {
    /// Set while a potion's ring is dealing its damage (DMG_MAGIC).
    static MAGIC_HIT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// What one hit did.
#[derive(Clone, Copy, Debug, Default)]
struct HitResult {
    hit: bool,
    killed: bool,
    generator: bool,
    /// The monster's (or generator's) type, for experience.
    tier: usize,
    /// A breakable broke: where, and its content.
    drop: Option<(Vec3, Option<usize>)>,
}

/// How a level finished.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LevelEnd {
    /// Everyone left through exits: the wave they chose, or None for the tower.
    Exit(Option<u32>),
    /// Every player died.
    AllDead,
    /// A secret icon was touched: everyone goes to this secret level.
    Secret(u32),
    /// A secret level's time ran out: back to the level it was found in.
    SecretOver,
}

pub struct Player {
    pub model: usize,
    pub pos: Vec3,
    pub yaw: f32,
    pub vy: f32,
    pub on_ground: bool,
    pub animator: Animator,
    pub state: PlayerState,
    pub health: f32,
    /// Experience level (1..99) and the experience behind it.
    pub level: u32,
    pub exp: u32,
    /// Character class id (WAR 0 .. HYE 15).
    pub class_id: usize,
    /// Attribute bonuses bought in the shop.
    pub bonus: crate::stats::Atts,
    /// Combat values from the class, level and bonuses.
    pub derived: crate::stats::Derived,
    /// The turbo meter's target (0..100).
    pub power: f32,
    /// Each carried potion's colour, used last in first out.
    pub potion_colours: Vec<u8>,
    pub powerups: crate::powerups::Powerups,
    /// Lifetime totals for the shop's tally and Final Stats (PlayerShopStat).
    pub stats: PlayStats,
    /// What the active power-ups add up to this frame.
    pub fx: crate::powerups::Effects,
    pub keys: u32,
    pub potions: u32,
    /// Gold (GDL has no score), at most 99999.
    pub gold: u32,
    pub hurt_flash: f32,
    /// Where this player's exit leads (None = the tower).
    pub exit_dest: Option<u32>,
    /// Dead while others play on: "Wait In Tower" (A) / "Quit Game" (B) is up.
    pub tower_choice: bool,
    /// Chose "Quit Game": leaves the party when the level ends.
    pub quit: bool,
    /// Model thrown by this hero's ranged attack.
    pub proj_model: Option<usize>,
    /// Display name ("WARRIOR"), for announcer messages.
    pub name: String,
    /// Class and colour codes ("WAR", "RED") for sounds.
    pub class: String,
    pub color: String,
    /// The experience at Sumner's last level-up check (Player+0x1EDC).
    pub sumner_exp: u32,
    /// The power-up mini-inventory box.
    pub inv: crate::powerups::MiniInv,
    /// The action machine: what the hero is doing and animating.
    pub act: crate::paction::Action,
    /// This frame's hits for PlayerKnockback: damage, flags and the push
    /// direction; and the knock velocity they give.
    hit_damage: f32,
    hit_flags: u32,
    hit_force: Vec3,
    knock_vel: Vec3,
    /// Two-player combos: the linked partner, the one about to be linked, the
    /// combo timer (timer_1FA, ticks), the place before being picked up, and
    /// whose DUMMY node this hero rides while grabbed (hud 0x20).
    pub grab_partner: Option<usize>,
    grab_pending: Option<usize>,
    combo_timer: i32,
    saved_pos: Vec3,
    attached_to: Option<usize>,
}

/// Animation sequence indices for a player model.
pub struct PlayerSeqs {
    ready: usize,
    falling: Option<usize>,
    death: Option<usize>,
}

impl PlayerSeqs {
    fn find(m: &Model) -> PlayerSeqs {
        let s = |n: &str| m.sequence(n);
        PlayerSeqs {
            ready: s("READY").unwrap_or(0),
            falling: s("FALLING"),
            death: s("DEATH"),
        }
    }
}

/// Animation sequences for a monster model.
#[derive(Clone)]
pub struct MonsterSeqs {
    ready: Option<usize>,
    walk: Option<usize>,
    /// ATTACK1 / ATTACK1R (normal swing and recovery), ATTACK3 / ATTACK3R (power).
    attack: Option<(usize, Option<usize>)>,
    power: Option<(usize, Option<usize>)>,
    hit: Option<usize>,
    death: Option<usize>,
    start: Option<usize>,
    /// THROW1 (or THROW2) and THROWF: archers and bombers.
    throw: Option<(usize, Option<usize>)>,
    run: Option<usize>,
}

impl MonsterSeqs {
    fn find(m: &Model) -> MonsterSeqs {
        let s = |n: &str| m.sequence(n);
        let pair = |a: &str, r: &str| s(a).map(|a| (a, s(r)));
        MonsterSeqs {
            ready: s("READY"),
            walk: s("WALK").or(s("WALK1")).or(s("RUN")),
            attack: pair("ATTACK1", "ATTACK1R").or(pair("ATTACK2", "ATTACK2R")),
            power: pair("ATTACK3", "ATTACK3R"),
            hit: s("HIT1").or(s("HIT2")),
            death: s("DEATH").or(s("HIT2")),
            start: s("START"),
            throw: s("THROW1").or(s("THROW2")).map(|t| (t, s("THROWF"))),
            run: s("RUN").or(s("RUNATTACK1")),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EnemyState {
    Spawning,
    /// Not yet recognized a player: stands, or wanders on screen.
    Idle,
    Chase,
    /// Winding up a swing at a player (the hit lands as the wind-up ends).
    Attack {
        hit_done: bool,
    },
    /// The swing's recovery.
    Recover,
    Hurt {
        timer: f32,
    },
    Dying {
        timer: f32,
    },
    /// Placed with strength 0: drawn but inert until hurt.
    Asleep,
    /// Throwing an arrow or bomb (THROW1 then THROWF), or casting a fireball;
    /// the missile leaves when the first animation ends.
    Throw {
        fired: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PickupKind {
    /// Heals this much; negative is spoiled food that poisons.
    Food(f32),
    /// Keys in it (KEY 1, KEYRING 3 or a placed count).
    Key(u32),
    Gold(u32),
    /// A potion of this colour (0 random, 1 fire, 2 lightning, 3 light, 4 acid).
    Potion(u8),
    /// A GEM<colour> crystal: its colour index (crystal_order position).
    Crystal(usize),
    /// A scroll: the line of the level's SCROLLS message it shows.
    Scroll(usize),
    /// A runestone: the party's shard bit (0..12).
    Shard(u32),
    /// A legend weapon (1..11).
    Legend(u32),
    /// A gargoyle's golden item: 0 fangs, 1 feathers, 2 claws.
    Gargoyle(usize),
    /// A power-up: type, flags, amount (uses or bonus) and seconds (negative
    /// when counted in uses) before the class's powerup_time.
    Powerup {
        kind: u8,
        flags: u32,
        amount: f32,
        seconds: f32,
    },
    Other(u32),
}

/// What a power-up item gives (items.c fn_8005DE50 by subtype; values are the
/// item info's).
fn pickup_kind(info: &gdl_formats::worlds::ItemInfo, params: &[u8]) -> PickupKind {
    let s16 = |k: usize| i16::from_le_bytes([params[k], params[k + 1]]);
    let s32 = |k: usize| i32::from_le_bytes(params[k..k + 4].try_into().unwrap());
    match info.subtype_name() {
        "food" => PickupKind::Food(info.value as f32),
        "key" => PickupKind::Key(if info.name.eq_ignore_ascii_case("KEYRING") {
            3
        } else {
            (s16(0) as u32).max(1)
        }),
        "gold" => PickupKind::Gold(info.value.max(0) as u32),
        "potion" => PickupKind::Potion((info.properties & 0xF) as u8),
        "scroll" => PickupKind::Scroll((s32(4) - 1).max(0) as usize),
        "crystal" => match CRYSTAL_COLOURS
            .iter()
            .position(|c| !c.is_empty() && info.name.eq_ignore_ascii_case(&format!("GEM{c}")))
        {
            Some(c) => PickupKind::Crystal(c),
            None => PickupKind::Other(0),
        },
        "runestone" => PickupKind::Shard(info.value.clamp(0, 12) as u32),
        "legend_item" => PickupKind::Legend(info.value.max(0) as u32),
        "gargoyle_item" => PickupKind::Gargoyle(match info.name.to_ascii_uppercase().as_str() {
            n if n.starts_with("GARGSERP") => 0,
            n if n.starts_with("GARGEAGL") => 1,
            _ => 2,
        }),
        "weapon" | "armor" | "speed" | "magic" | "special" => PickupKind::Powerup {
            kind: info.subtype as u8,
            flags: info.properties,
            amount: info.value as f32,
            seconds: info.active_on as f32,
        },
        _ => PickupKind::Other(0),
    }
}

/// The help message for picking something up (items.c fn_8005DE50's msgPost).
fn pickup_message(kind: &PickupKind) -> Option<usize> {
    use crate::powerups as pu;
    Some(match *kind {
        PickupKind::Food(v) if v < 0.0 => 28,
        PickupKind::Food(v) if v >= 100.0 => 15,
        PickupKind::Food(v) if v >= 50.0 => 16,
        PickupKind::Gold(v) if v >= 25 => 17,
        PickupKind::Legend(v) => 113 + v as usize,
        PickupKind::Key(_) => 8,
        PickupKind::Potion(_) => 7,
        PickupKind::Powerup { kind, flags: f, .. } => match kind {
            pu::WEAPON if f & pu::W_3WAY != 0 => 37,
            pu::WEAPON if f & pu::W_5WAY != 0 => 47,
            pu::WEAPON if f & pu::W_REFLECT != 0 => 38,
            pu::WEAPON if f & pu::W_SUPER != 0 => 48,
            pu::WEAPON if f & pu::W_HAMMER != 0 => 86,
            pu::WEAPON if f & pu::W_RAPID != 0 => 87,
            pu::WEAPON => match f & 0xF {
                1 => 40,
                2 => 41,
                3 => 42,
                4 => 43,
                _ => return None,
            },
            pu::ARMOR if f & pu::A_GOLD != 0 => 54,
            pu::ARMOR if f & pu::A_INVULN != 0 => 35,
            pu::ARMOR if f & pu::A_ANTIDEATH != 0 => 49,
            pu::ARMOR if f & pu::A_REFLECT != 0 => 52,
            pu::ARMOR if f & pu::A_FIRE != 0 => 91,
            pu::ARMOR if f & pu::A_LIGHTNING != 0 => 92,
            pu::ARMOR if f & pu::A_GAS != 0 => 132,
            pu::SPEED => 32,
            pu::MAGIC => 33,
            pu::SPECIAL => {
                const MAP: [(u32, usize); 18] = [
                    (0x4, 36),
                    (0x2, 39),
                    (0x8, 51),
                    (0x1, 53),
                    (0x10, 81),
                    (0x20, 82),
                    (0x40, 83),
                    (0x80, 84),
                    (0x100, 88),
                    (0x200, 89),
                    (0x400, 93),
                    (0x2000, 98),
                    (0x1000, 99),
                    (0x8000, 100),
                    (0x4000, 100),
                    (0x80000, 113),
                    (0x100000, 148),
                    (0x200000, 149),
                ];
                if f & 0x400000 != 0 {
                    150
                } else {
                    MAP.iter().find(|(m, _)| f & m != 0)?.1
                }
            }
            _ => return None,
        },
        _ => return None,
    })
}

/// The message for a pickup a player can't take: full health, keys or potions.
fn refused_message(kind: &PickupKind) -> Option<usize> {
    match kind {
        PickupKind::Food(_) => Some(133),
        PickupKind::Key(_) => Some(4),
        PickupKind::Potion(_) => Some(3),
        _ => None,
    }
}

/// Whether a runestone is already the party's (msg 90).
fn shard_owned(shards: u16, kind: &PickupKind) -> bool {
    matches!(*kind, PickupKind::Shard(b) if shards & (1 << b) != 0)
}

/// Whether a player can take a pickup now: food isn't taken at full health,
/// and keys and potions stop at 9 (the item stays on the floor).
fn can_take(p: &Player, kind: &PickupKind) -> bool {
    match *kind {
        PickupKind::Food(hp) => hp < 0.0 || p.health < max_health(p.level),
        PickupKind::Key(_) => p.keys < MAX_KEYS,
        PickupKind::Potion(_) => p.potions < MAX_POTIONS,
        // Only one boss item (horns, mask, gauntlets) at a time.
        PickupKind::Powerup {
            kind: crate::powerups::SPECIAL,
            flags,
            ..
        } if flags & 0xF000 != 0 => p.fx.special & 0xF000 == 0,
        _ => true,
    }
}

/// Keys and potions a player can carry.
pub const MAX_KEYS: u32 = 9;
pub const MAX_POTIONS: u32 = 9;

/// Crystal colours by index (SM_CRYSTAL_<colour>, GEM<colour>), and how many of
/// each open the next realm's barrier (lbl_80124C70).
pub const CRYSTAL_COLOURS: [&str; 9] = [
    "", "ORANGE", "RED", "PURPLE", "BLUE", "GREEN", "YELLOW", "WHITE", "BLACK",
];
pub const CRYSTALS_NEEDED: [i32; 9] = [0, 15, 100, 125, 150, 175, 200, 225, 250];
/// Gargoyle items the tower's snake, eagle and lion gates need.
pub const GARG_NEEDED: [i32; 3] = [12, 20, 28];

/// An angle wrapped into (-pi, pi].
pub fn wrap_angle(a: f32) -> f32 {
    let mut a = a % std::f32::consts::TAU;
    if a > std::f32::consts::PI {
        a -= std::f32::consts::TAU;
    } else if a <= -std::f32::consts::PI {
        a += std::f32::consts::TAU;
    }
    a
}

/// towerAllPlayersMetBossReq: enough of a colour found, or that colour done (-1).
pub fn crystals_met(crystals: &[i32; 9], c: usize) -> bool {
    let n = crystals.get(c).copied().unwrap_or(0);
    n < 0 || n >= CRYSTALS_NEEDED.get(c).copied().unwrap_or(0)
}

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Enemy {
        class: usize,
        /// Tier 1-3 (placed 4-6: archer, bomber, suicide).
        strength: usize,
        hp: f32,
        state: EnemyState,
        /// The spawn freeze left, in seconds.
        cooldown: f32,
        /// Units per second.
        speed: f32,
        /// e_e_tpye.
        etype: usize,
        /// Has seen a player: chases from then on.
        recognized: bool,
        /// Swings so far (every 8th is the power attack).
        attacks: u32,
        /// The player a swing was aimed at, and whether it is the power attack.
        target: Option<(usize, bool)>,
        /// Seconds until the next throw or cast (idle_secs / counter1), or the
        /// suicide charge's clock.
        reload: f32,
        /// Death: health drained so far; IT and the rest: unused.
        drained: f32,
        /// The Warlock's blink: seconds left seen (> 0) or unseen (< 0).
        blink: f32,
    },
    Generator {
        class: usize,
        /// The tier it spawns (drops as it is damaged).
        strength: usize,
        hp: f32,
        /// Ticks until the next spawn.
        timer: f32,
        /// The spawn interval (tenths of a second).
        interval: f32,
        max: usize,
        spawned: Vec<u64>,
        etype: usize,
        /// Grows 1/(2 max) a spawn and wraps: the delay is interval x (1..2).
        genratio: f32,
        /// Instance flag 1: spawns even off screen.
        always: bool,
        /// Temple / Hell SPECIAL generators: which list they cycle through.
        special: Option<&'static [usize; 4]>,
    },
    Pickup {
        kind: PickupKind,
        radius: f32,
    },
    /// A chest (opened with a key on touch) or a breakable (barrels, walls).
    Container {
        hp: f32,
        drop: Option<usize>,
        chest: bool,
        armor: f32,
    },
    Door {
        radius: f32,
        coll: Option<(usize, usize)>,
    },
    Exit {
        radius: f32,
        /// The wave this exit leads to, or None for the tower.
        dest: Option<u32>,
        open: bool,
    },
    Hazard {
        radius: f32,
        damage: f32,
    },
    /// A SECRET_ICON: touching it takes the party to a secret level, once.
    Secret {
        radius: f32,
        wave: Option<u32>,
        used: bool,
    },
    Projectile {
        owner: usize,
        vel: Vec3,
        life: f32,
        damage: f32,
    },
    /// A monster's arrow, bomb or fireball.
    EnemyShot {
        vel: Vec3,
        damage: f32,
        life: f32,
        /// Bombs burst over this radius.
        splash: f32,
        gravity: f32,
        /// C_DMG flags handed to damage_player (critter shots).
        flags: u32,
        /// Effect model shown where it bursts.
        hit_model: Option<usize>,
    },
    /// A critter: a boss, golem, general or gargoyle (critter.rs).
    Critter(Box<crate::critter::Critter>),
    /// A one-shot effect model, gone after `life` seconds or its animation.
    Effect {
        life: f32,
    },
    /// Lever or pressure pad driving `Game::movers[mover]`.
    Trigger {
        mover: Option<usize>,
        mode: TriggerMode,
        radius: f32,
        /// Toggle cooldown.
        idle: f32,
        /// Only hits set it off (shoot_trigger).
        shoot: bool,
        /// Touched last frame (lever pulled).
        pressed: bool,
        /// Struck by an attack this frame.
        hit: bool,
        /// A tower barrier: the crystal colour (1-8) or gargoyle gate (101-103).
        gate: Option<usize>,
        /// The trigger's id (instance params byte 6).
        id: u8,
    },
    Prop,
}

/// Solid collision shape of an entity, from its item info.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    Cylinder {
        radius: f32,
    },
    /// Half extents along the entity's local x and z axes.
    Box {
        half: Vec2,
    },
}

pub struct Entity {
    pub id: u64,
    pub solid: Option<Shape>,
    pub kind: Kind,
    pub model: Option<usize>,
    pub pos: Vec3,
    pub yaw: f32,
    pub animator: Animator,
    pub spin: f32,
    pub alive: bool,
    /// 1 = fully shown, 0 = invisible (the tower's fade-ins).
    pub fade: f32,
}

/// A monster type (`GRU`, `RAT`, ...) with a model per strength level.
pub struct MonsterClass {
    pub name: String,
    /// Tier models 0-3, then the A (archer), B (bomber) and S (suicide) trees.
    models: [Option<usize>; 7],
    gen_models: [Option<usize>; 4],
    pub etype: usize,
    /// <PREFIX>_ARROW, _BOMB and _FBALL missile models.
    missiles: [Option<usize>; 3],
}

impl MonsterClass {
    pub(crate) fn model(&self, strength: usize) -> Option<usize> {
        if (4..=6).contains(&strength) {
            if let Some(m) = self.models[strength] {
                return Some(m);
            }
        }
        let s = strength.clamp(1, 3);
        self.models[s]
            .or(self.models[1])
            .or(self.models[2])
            .or(self.models[3])
            .or(self.models[0])
    }
    fn gen_model(&self, strength: usize) -> Option<usize> {
        let s = strength.min(3);
        self.gen_models[s]
            .or(self.gen_models[1])
            .or(self.gen_models[0])
            .or(self.gen_models[2])
    }
}

pub struct Game {
    pub level: Level,
    pub collision: Collision,
    pub models: Vec<Model>,
    pub player_seqs: Vec<PlayerSeqs>,
    monster_seqs: HashMap<usize, MonsterSeqs>,
    pub classes: Vec<MonsterClass>,
    pub players: Vec<Player>,
    pub entities: Vec<Entity>,
    /// World geometry moved by levers and pads.
    pub movers: Vec<Mover>,
    next_id: u64,
    pub cam_focus: Vec3,
    /// Camera heading (game yaw convention; pi looks along +z) and pitch, smoothed
    /// towards the nearest of the level's game-camera locators.
    pub cam_yaw: f32,
    pub cam_pitch: f32,
    pub time: f32,
    /// 60 Hz ticks since the level started.
    pub frame: u32,
    /// Ticks until the next low-health heartbeat.
    heartbeat: i32,
    pub rng: Rng,
    /// The level's wave, when a realm table lists it.
    pub wave: Option<u32>,
    pub worlds: gdl_formats::wdata::Worlds,
    /// This level's scroll texts (SCROLL_E.ROM "SCROLLS<level>"), by line.
    pub scrolls: Vec<String>,
    pub scroll_scale: f32,
    /// SCROLL_E.ROM, for the tower's story messages and Sumner's speeches.
    pub scroll_rom: Option<gdl_formats::text::TextRom>,
    /// A scroll being read: the game is paused until its player presses B.
    pub scroll_box: Option<ScrollBox>,
    /// The tower's wizard: speeches, story messages and trigger cameras.
    pub sumner: Option<Box<crate::tower::Sumner>>,
    /// Locked tower barriers touched this frame (crystal colour or 101-103).
    pub barrier_touches: Vec<usize>,
    /// World objects were hidden or shown: the scene's world mesh must be rebuilt.
    pub world_changed: bool,
    /// A camera set from outside (the attract flyby): eye and look-at.
    pub fixed_camera: Option<(Vec3, Vec3)>,
    /// Placed monsters not yet in view.
    pending_enemies: Vec<PendingEnemy>,
    /// Where the next damage_player hit comes from (for its push).
    pub damage_from: Option<Vec3>,
    /// The music area asked for (sMusicSubIndex) and how to change to it
    /// (sMusicSubState: 1 fade out first, 2 at once); and the sound items
    /// that select areas: (position, radius, area, fade).
    pub music_area: (usize, i16),
    music_items: Vec<(Vec3, f32, usize, i16)>,
    /// Ambient sound items: (position, radius, sound name).
    ambient_items: Vec<(Vec3, f32, String)>,
    /// The party's boss shards (bit = world order 1..11, as Progress::runes).
    pub runes: u16,
    /// Players who died in this level (inactivate_player outside the tower).
    pub deaths: u32,
    /// The level's monster types (realm ENMY + LEVL enemytype).
    pub roster: crate::enemies::Roster,
    /// The level's monster and generator tuning, normalized for the difficulty.
    pub tuning: gdl_formats::wdata::EnemyTuning,
    /// LEVL maxenemies: monsters alive at once.
    pub max_enemies: usize,
    /// The player who is IT: every monster goes after them.
    pub it_holder: Option<usize>,
    /// Round-robin counter for the Temple / Hell SPECIAL generators.
    special_counter: u32,
    /// Potion blasts spreading out.
    pub rings: Vec<Ring>,
    /// Lightning-shield strikes waiting out their second, by (player, monster).
    shield_cooldown: HashMap<(usize, u64), f32>,
    /// Damage-row re-hits waiting out their delay, by (player, target).
    damg_cooldown: HashMap<(usize, u64), f32>,
    /// 0 Easy, 1 Medium, 2 Hard.
    pub difficulty: usize,
    /// The Multiplayer option: 0 Normal (shots pass heroes), 1 Shots Stun,
    /// 2 Shots Hurt.
    pub friendly_fire: usize,
    /// Each player model's action slots (InitActions) and class PDATA.
    action_seqs: Vec<crate::paction::ActionSeqs>,
    pdata: Vec<gdl_formats::pdata::PlayerData>,
    /// Item models loaded for the level, by item name.
    item_models: HashMap<String, Option<usize>>,
    /// A secret level's countdown: (total, seconds left).
    pub secret_timer: Option<(f32, f32)>,
    /// The party's shards (runestones, 13 bits) and gargoyle items (fangs,
    /// feathers, claws; -1 once their gate has opened).
    pub shards: u16,
    pub garg_items: [i32; 3],
    /// Ticks the rune-stone icons stay up after one is found (welcome_timer).
    pub welcome: i32,
    /// Crystals found per colour; -1 once a colour has opened its barrier.
    pub crystals: [i32; 9],
    /// The crystal count to show on the HUD (colour, seconds left).
    pub crystal_shown: Option<(usize, f32)>,
    /// World objects hidden for good (closed tower portals).
    pub hidden: HashSet<usize>,
    /// Set once every player has left or died.
    pub level_end: Option<LevelEnd>,
    /// Help messages and announcements (msgPost).
    pub msgs: crate::messages::Messages,
    /// Sound effects requested this frame (names from AUDATPS2.ROM); drained by the app.
    pub sfx: Vec<String>,
    /// The level's boss, when it has one (boss.rs).
    pub boss: Option<crate::boss::Boss>,
    /// Boss runes (shards in the game's words) won here: bit = position in
    /// progress::CRYSTAL_ORDER. The app adds them to Progress.runes.
    pub boss_runes: u16,
    /// Legend weapons the party carries: bit = the item's value (the realm of
    /// the boss it is for). One is used up when its boss fight uses it.
    pub legend_held: u16,
    /// Items flying out of a dying boss.
    pub spews: Vec<crate::boss::Spew>,
    /// Critter ground waves (stomps, eruptions).
    pub crit_waves: Vec<crate::critter::Wave>,
    /// player.fxhittime: a critter's melee and cones skip a player until then.
    pub crit_hit_until: [f32; 4],
    /// Parsed CRITTER WADs by file name.
    pub critter_wads: HashMap<String, std::sync::Arc<gdl_formats::critter::CritterWad>>,
    /// The good wizard's model (atree WIZARD in the level's item file).
    pub wizard_model: Option<usize>,
    /// Where the hit being dealt landed (projectiles), for critter colnodes.
    pub(crate) hit_from: Option<Vec3>,
}

/// A placed monster waiting to come into view (an ENE item before
/// fn_80060114 turns it into an enemy).
struct PendingEnemy {
    kind: Kind,
    model: Option<usize>,
    pos: Vec3,
    yaw: f32,
    /// The item's visibility radius.
    visrad: f32,
}

/// Parsed objects/textures/anim files shared by several models.
pub(crate) struct AssetFile {
    objects: Objects,
    textures: Vec<u8>,
    anim: Option<Anim>,
}

fn load_asset(fs: &GameFs, dir: &str) -> Option<AssetFile> {
    let objects = Objects::parse(&fs.read(&format!("{dir}/objects.ngc")).ok()?).ok()?;
    let textures = fs.read(&format!("{dir}/textures.ngc")).ok()?;
    let anim = fs
        .read(&format!("{dir}/ANIM.PS2"))
        .ok()
        .and_then(|d| Anim::parse(&d).ok());
    Some(AssetFile {
        objects,
        textures,
        anim,
    })
}

/// World collision triangles. Triangles owned by items and by moving world objects
/// are stored relative to their owner, everything else is in world space.
fn build_collision(level: &Level) -> Vec<Tri> {
    let w = &level.worlds;
    let mut tris: Vec<Tri> = w
        .coll_tris
        .iter()
        .map(|t| Tri {
            v: t.verts.map(Vec3::from),
            n: Vec3::from(t.norm).normalize_or_zero(),
            owner: -1,
        })
        .collect();
    let mut place = |start: i32, count: i32, pos: Vec3, yaw: f32, radius: f32, owner: i32| {
        if start < 0 || count <= 0 {
            return;
        }
        let range = start as usize..((start + count) as usize).min(tris.len());
        // Local if the triangles sit around the origin rather than around the owner.
        let mut near_origin = 0;
        let mut near_owner = 0;
        for t in &tris[range.clone()] {
            let c = (t.v[0] + t.v[1] + t.v[2]) / 3.0;
            let r = radius.max(12.0);
            near_origin += (c.length() < r) as usize;
            near_owner += ((c - pos).length() < r) as usize;
        }
        let local = pos.length() > 1.0 && near_origin > near_owner;
        let rot = glam::Quat::from_rotation_y(yaw);
        for t in &mut tris[range] {
            if local {
                t.v = t.v.map(|v| pos + rot * v);
                t.n = rot * t.n;
            }
            t.owner = owner;
        }
    };
    let pos = &level.world_pos;
    for (i, o) in w.world_objects.iter().enumerate() {
        place(
            o.coll_tri_index,
            o.coll_tri_count as i32,
            Vec3::from(pos[i]),
            0.0,
            o.radius,
            i as i32,
        );
    }
    for (i, it) in w.item_instances.iter().enumerate() {
        place(
            it.coll_tri_index as i32,
            it.coll_tri_count as i32,
            Vec3::from(it.pos),
            it.rot[1],
            8.0,
            100_000 + i as i32,
        );
    }
    tris
}

fn item_shape(info: &gdl_formats::worlds::ItemInfo) -> Shape {
    match info.coll_type {
        3 => Shape::Box {
            half: Vec2::new(info.x_dim.max(0.5), info.z_dim.max(0.5)),
        },
        _ => Shape::Cylinder {
            radius: info.radius.clamp(0.4, 6.0),
        },
    }
}

impl Game {
    /// Where an EXIT item leads (items.c / gamemain.c): a nonzero first word
    /// returns to the tower; in the tower the 2-character code names a portal's
    /// level (FindWave). Elsewhere every exit leads back to the tower, except that
    /// the Temple and Hell realms chain E1 -> E2 and F1 -> F2.
    fn exit_dest(&self, params: &[u8]) -> Option<u32> {
        let back = params.get(0..4).is_some_and(|b| b != [0, 0, 0, 0]);
        if back {
            return None;
        }
        match self.wave {
            Some(gdl_formats::wdata::TOWER_WAVE) => {
                let code = String::from_utf8_lossy(params.get(4..8)?)
                    .trim_end_matches('\0')
                    .to_string();
                self.worlds.find_wave(&code)
            }
            Some(0x500) => Some(0x501),
            Some(0x600) => Some(0x601),
            _ => None,
        }
    }

    /// EnterTower: barriers whose crystals are done start open, and the tower's
    /// orange crystals are gone once orange is done.
    pub fn enter_tower(&mut self) {
        let crystals = self.crystals;
        let garg = self.garg_items;
        let mut open = Vec::new();
        for e in &mut self.entities {
            match &e.kind {
                Kind::Trigger {
                    gate: Some(c),
                    mover: Some(m),
                    ..
                } if (*c < 9 && crystals[*c] < 0)
                    || ((101..=103).contains(c) && garg[*c - 101] < 0) =>
                {
                    open.push(*m)
                }
                Kind::Pickup {
                    kind: PickupKind::Crystal(1),
                    ..
                } if crystals[1] < 0 => e.alive = false,
                _ => {}
            }
        }
        self.entities.retain(|e| e.alive);
        // The lion gate also brings the lift and elevator down to the Battle
        // realm's floor; the 13th shard opens the lift to the Citadel.
        let extra: Vec<usize> = self
            .entities
            .iter()
            .filter_map(|e| match e.kind {
                Kind::Trigger {
                    id: 104 | 199,
                    mover: Some(m),
                    ..
                } if garg[2] < 0 => Some(m),
                Kind::Trigger {
                    id: 255,
                    mover: Some(m),
                    ..
                } if self.shards & 0x1000 != 0 => Some(m),
                _ => None,
            })
            .collect();
        for m in open.into_iter().chain(extra) {
            if let Some(m) = self.movers.get_mut(m) {
                m.active = true;
            }
        }
    }

    /// msgPost about `player` (or the party).
    pub fn post(&mut self, index: usize, player: Option<usize>, args: &[String]) -> bool {
        // msgPost is refused while a trigger camera runs.
        if self.sumner.as_ref().is_some_and(|s| s.cam.is_some()) {
            return false;
        }
        let active: Vec<usize> = (0..self.players.len().min(4))
            .filter(|&i| self.players[i].state.active())
            .collect();
        // "%s %s HAS SHRUNK ENEMIES" and friends name the player's colour and class.
        let named;
        let args = if args.is_empty() && [0x32, 0x59, 0x5D].contains(&index) {
            let p = &self.players[player
                .unwrap_or(0)
                .min(self.players.len().saturating_sub(1))];
            let colour = match p.color.as_str() {
                "YEL" => "YELLOW",
                "BLU" => "BLUE",
                "RED" => "RED",
                _ => "GREEN",
            };
            named = [colour.to_string(), p.name.clone()];
            &named[..]
        } else {
            args
        };
        self.msgs.post(index, player, &active, args)
    }

    /// An item or effect model loaded for the level, by name.
    pub fn model_named(&self, name: &str) -> Option<usize> {
        self.item_models
            .get(&name.to_ascii_uppercase())
            .copied()
            .flatten()
    }

    pub fn entity_mut(&mut self, id: u64) -> Option<&mut Entity> {
        self.entities.iter_mut().find(|e| e.id == id)
    }

    /// A world object's position, by name.
    pub fn object_pos(&self, name: &str) -> Option<Vec3> {
        let objs = &self.level.worlds.world_objects;
        let i = objs
            .iter()
            .position(|o| o.name.eq_ignore_ascii_case(name))?;
        self.level.world_pos.get(i).map(|p| Vec3::from(*p))
    }

    /// Show a hidden world object again; false if it wasn't hidden.
    pub fn show_object(&mut self, name: &str) -> bool {
        let objs = &self.level.worlds.world_objects;
        let Some(i) = objs.iter().position(|o| o.name.eq_ignore_ascii_case(name)) else {
            return false;
        };
        let mut shown = false;
        for j in self.level.subtree(i) {
            shown |= self.hidden.remove(&j);
        }
        shown
    }

    /// ActivateSpecialTrigger(id, 0): set off the mover of trigger `id`.
    pub fn activate_trigger(&mut self, id: u8) {
        let movers: Vec<usize> = self
            .entities
            .iter()
            .filter_map(|e| match e.kind {
                Kind::Trigger {
                    id: i,
                    mover: Some(m),
                    ..
                } if i == id => Some(m),
                _ => None,
            })
            .collect();
        for m in movers {
            if let Some(m) = self.movers.get_mut(m) {
                m.active = true;
            }
        }
    }

    /// Hide a world object (and its children) by name; false if there is none.
    pub fn hide_object(&mut self, name: &str) -> bool {
        let objs = &self.level.worlds.world_objects;
        let Some(i) = objs.iter().position(|o| o.name.eq_ignore_ascii_case(name)) else {
            return false;
        };
        let sub = self.level.subtree(i);
        self.hidden.extend(sub);
        true
    }

    /// Close the tower portals whose level isn't open (fn_8005B5B8): the portal
    /// stops being an exit and its "_ACTIVE" glow object is hidden.
    pub fn close_portals(&mut self, open: impl Fn(u32) -> bool) {
        let mut glows = Vec::new();
        for e in &mut self.entities {
            if let Kind::Exit {
                dest: Some(w),
                open: o,
                ..
            } = &mut e.kind
            {
                if !open(*w) {
                    *o = false;
                    let letter = self.worlds.realm(*w >> 8).map_or('?', |r| r.letter);
                    glows.push(format!("L1NSNC{letter}{}_ACTIVE", (*w & 0xFF) + 1));
                }
            }
        }
        for g in glows {
            if !self.hide_object(&g) {
                log::warn!("No Exit Obj: {g}");
            }
        }
    }

    /// Put the party at start locator `index` (the locator's `next` field), if any.
    pub fn set_start(&mut self, index: u16) {
        let Some(l) = self
            .level
            .worlds
            .locators
            .iter()
            .find(|l| l.kind == gdl_formats::worlds::LocatorType::Start && l.next == index)
        else {
            return;
        };
        let base = Vec3::from(l.pos);
        for (i, p) in self.players.iter_mut().enumerate() {
            p.pos = base + Vec3::new((i as f32 - 1.5) * 2.5, 0.5, 0.0);
        }
        self.cam_focus = base;
    }

    pub fn new(fs: &GameFs, level_name: &str, classes: &[(&str, &str)]) -> Result<Game> {
        Self::new_with(fs, level_name, classes, 1)
    }

    /// A level at a difficulty (0 Easy, 1 Medium, 2 Hard).
    pub fn new_with(
        fs: &GameFs,
        level_name: &str,
        classes: &[(&str, &str)],
        difficulty: usize,
    ) -> Result<Game> {
        let level = Level::load(fs, level_name)?;
        let collision = Collision::new(build_collision(&level));

        let mut game = Game {
            level,
            collision,
            models: Vec::new(),
            player_seqs: Vec::new(),
            monster_seqs: HashMap::new(),
            classes: Vec::new(),
            players: Vec::new(),
            entities: Vec::new(),
            movers: Vec::new(),
            next_id: 1,
            cam_focus: Vec3::ZERO,
            cam_yaw: std::f32::consts::PI,
            cam_pitch: 60f32.to_radians(),
            time: 0.0,
            frame: 0,
            heartbeat: 0,
            rng: Rng(0x9E37_79B9_7F4A_7C15),
            wave: None,
            worlds: Default::default(),
            scrolls: Vec::new(),
            scroll_scale: 0.5,
            scroll_rom: None,
            scroll_box: None,
            sumner: None,
            barrier_touches: Vec::new(),
            world_changed: false,
            fixed_camera: None,
            pending_enemies: Vec::new(),
            damage_from: None,
            music_area: (0, 0),
            music_items: Vec::new(),
            ambient_items: Vec::new(),
            runes: 0,
            deaths: 0,
            roster: Default::default(),
            tuning: gdl_formats::wdata::EnemyTuning::default().normalized(1),
            max_enemies: 25,
            special_counter: 0,
            it_holder: None,
            rings: Vec::new(),
            shield_cooldown: HashMap::new(),
            damg_cooldown: HashMap::new(),
            difficulty,
            friendly_fire: 0,
            action_seqs: Vec::new(),
            pdata: Vec::new(),
            item_models: HashMap::new(),
            secret_timer: None,
            shards: 0,
            garg_items: [0; 3],
            welcome: 300,
            crystals: [0; 9],
            crystal_shown: None,
            hidden: HashSet::new(),
            level_end: None,
            msgs: Default::default(),
            sfx: vec!["S_ENTRANCE".into()],
            boss: None,
            boss_runes: 0,
            legend_held: 0,
            spews: Vec::new(),
            crit_waves: Vec::new(),
            crit_hit_until: [0.0; 4],
            critter_wads: HashMap::new(),
            wizard_model: None,
            hit_from: None,
        };
        if let Ok(rom) = fs.read("TEXT/ENGLISH.ROM") {
            if let Ok(rom) = gdl_formats::text::TextRom::parse(&rom) {
                game.msgs = crate::messages::Messages::new(
                    rom.messages.into_iter().map(|m| m.lines).collect(),
                );
            }
        }
        // SetScrollLevelMsgList: the scroll texts named after the level.
        if let Ok(rom) = fs.read("TEXT/SCROLL_E.ROM") {
            if let Ok(rom) = gdl_formats::text::TextRom::parse(&rom) {
                let code = level_name.trim_start_matches("level");
                if let Some(m) = rom.named(&format!("SCROLLS{code}")) {
                    game.scrolls = m.lines.clone();
                    game.scroll_scale = m.scale;
                }
                game.scroll_rom = Some(rom);
            }
        }
        match gdl_formats::wdata::Worlds::load(|p| fs.read(p)) {
            Ok(w) => {
                game.wave = w.wave_of_dir(level_name);
                // Secret levels last their wavetime (+0.99 s), then send the party back.
                if let Some(l) = game
                    .wave
                    .filter(|w| w >> 8 == 12)
                    .and_then(|wv| w.level(wv))
                {
                    let t = l.wavetime as f32 + 0.99;
                    game.secret_timer = Some((t, t));
                }
                // GetEnemyTypes: the level's monsters, plus Death in every
                // level without a boss except the tower.
                if let Some(wave) = game.wave {
                    if let (Some(r), Some(l)) = (w.realm(wave >> 8), w.level(wave)) {
                        let mut loaded: Vec<(i32, i32)> = l
                            .enemy_types
                            .iter()
                            .filter(|&&i| i >= 0)
                            .filter_map(|&i| r.enemies.get(i as usize).copied())
                            .collect();
                        if l.boss < 0 && wave != gdl_formats::wdata::TOWER_WAVE {
                            loaded.push((crate::enemies::DEATH as i32, 0));
                        }
                        game.roster = crate::enemies::Roster::new(&loaded);
                        game.tuning = l.enemy.normalized(difficulty);
                        game.max_enemies = (l.max_enemies.clamp(1, 25)) as usize;
                    }
                }
                game.worlds = w;
            }
            Err(e) => log::warn!("realm tables: {e:#}"),
        }

        for (class, color) in classes {
            let m = Model::load_player(fs, class, color)?;
            game.player_seqs.push(PlayerSeqs::find(&m));
            game.action_seqs
                .push(crate::paction::ActionSeqs::new(&m.skeleton));
            game.pdata.push(
                fs.read(&format!("PDATA/{class}.WAD"))
                    .ok()
                    .and_then(|d| gdl_formats::pdata::PlayerData::parse(&d).ok())
                    .unwrap_or_default(),
            );
            game.models.push(m);
        }
        let starts = game.level.start_positions();
        let _ = &starts;
        for i in 0..classes.len() {
            let base = starts[i % starts.len()];
            let pos = base + Vec3::new((i as f32 - 1.5) * 2.5, 0.5, 0.0);
            game.players.push(Player {
                model: i,
                pos,
                yaw: 0.0,
                vy: 0.0,
                on_ground: false,
                animator: Animator::new(game.player_seqs[i].ready),
                state: PlayerState::Normal,
                health: max_health(1),
                level: 1,
                exp: 0,
                class_id: crate::select::CLASSES
                    .iter()
                    .position(|c| *c == classes[i].0)
                    .unwrap_or(0)
                    .min(15),
                bonus: Default::default(),
                derived: Default::default(),
                power: 0.0,
                potion_colours: Vec::new(),
                powerups: Default::default(),
                stats: Default::default(),
                fx: Default::default(),
                keys: 0,
                potions: 0,
                gold: 0,
                hurt_flash: 0.0,
                exit_dest: None,
                tower_choice: false,
                quit: false,
                proj_model: None,
                name: crate::select::HEROES
                    .iter()
                    .find(|(k, _)| *k == classes[i].0)
                    .map(|(_, n)| n.to_string())
                    .unwrap_or_else(|| classes[i].0.to_string()),
                class: classes[i].0.to_string(),
                color: classes[i].1.to_string(),
                sumner_exp: 0,
                inv: Default::default(),
                act: crate::paction::Action::new(),
                hit_damage: 0.0,
                hit_flags: 0,
                hit_force: Vec3::ZERO,
                knock_vel: Vec3::ZERO,
                grab_partner: None,
                grab_pending: None,
                combo_timer: 0,
                saved_pos: Vec3::ZERO,
                attached_to: None,
            });
        }
        // Ranged-attack projectile: the class's throw model, else its weapon.
        for (i, (class, color)) in classes.iter().enumerate() {
            let dir = format!("PLAYERS/{class}/{color}");
            let Some(asset) = load_asset(fs, &dir) else {
                continue;
            };
            let throw = asset
                .objects
                .object_defs
                .iter()
                .find(|d| d.name.contains("_THROW0"))
                .or_else(|| {
                    asset
                        .objects
                        .object_defs
                        .iter()
                        .find(|d| d.name.starts_with("WEAP_"))
                })
                .map(|d| d.name.clone());
            if let Some(name) = throw {
                if let Some(m) = Model::from_objects_anim(
                    &asset.objects,
                    &asset.textures,
                    asset.anim.as_ref(),
                    &[&name],
                ) {
                    game.players[i].proj_model = Some(game.add_model(m));
                }
            }
        }
        game.cam_focus = game.players.first().map(|p| p.pos).unwrap_or(Vec3::ZERO);
        for p in &mut game.players {
            p.refresh_stats();
        }
        game.populate(fs, classes.len());
        if let Some((yaw, pitch)) = game.camera_locator(game.cam_focus) {
            game.cam_yaw = yaw;
            game.cam_pitch = pitch.clamp(0.7, 1.3);
        }
        Ok(game)
    }

    fn add_model(&mut self, m: Model) -> usize {
        self.models.push(m);
        self.models.len() - 1
    }

    /// Load (once) the models of monster type `etype` from its MONSTERS
    /// directory: tier trees `<DESC>0..3`, the A/B/S variants and the generators.
    pub(crate) fn monster_class(
        &mut self,
        fs: &GameFs,
        cache: &mut HashMap<String, Option<AssetFile>>,
        etype: usize,
    ) -> Option<usize> {
        let dir = self.roster.dir(etype);
        if let Some(i) = self.classes.iter().position(|c| c.name == dir) {
            return Some(i);
        }
        let desc = crate::enemies::TYPES.get(etype)?.desc.to_ascii_uppercase();
        let path = format!("MONSTERS/{dir}");
        let asset = cache
            .entry(path.clone())
            .or_insert_with(|| load_asset(fs, &path))
            .as_ref()?;
        let anim = asset.anim.as_ref()?;
        let mut pending = Vec::new();
        for (slot, suffix) in ["0", "1", "2", "3", "A", "B", "S"].iter().enumerate() {
            let tree = format!("{desc}{suffix}");
            if anim
                .atrees
                .iter()
                .any(|t| t.name.eq_ignore_ascii_case(&tree))
            {
                if let Ok(m) = Model::from_anim(&asset.objects, &asset.textures, anim, Some(&tree))
                {
                    pending.push((slot, m));
                }
            }
        }
        let mut gens = Vec::new();
        for s in 0..4 {
            let names = [format!("GEN_{desc}{s}L1"), format!("GEN_{desc}{s}")];
            if let Some(m) = Model::from_objects_anim(
                &asset.objects,
                &asset.textures,
                Some(anim),
                &[&names[0], &names[1]],
            ) {
                gens.push((s, m));
            }
        }
        if pending.is_empty() {
            log::warn!("monster {dir}: no animation trees found");
            return None;
        }
        let mut models = [None; 7];
        for (s, m) in pending {
            let ms = MonsterSeqs::find(&m);
            let idx = self.add_model(m);
            self.monster_seqs.insert(idx, ms);
            models[s] = Some(idx);
        }
        let mut gen_models = [None; 4];
        for (s, m) in gens {
            gen_models[s] = Some(self.add_model(m));
        }
        let mut missiles = [None; 3];
        for (i, kind) in ["ARROW", "BOMB", "FBALL"].iter().enumerate() {
            let tree = format!("{desc}_{kind}");
            let m = if anim
                .atrees
                .iter()
                .any(|t| t.name.eq_ignore_ascii_case(&tree))
            {
                Model::from_anim(&asset.objects, &asset.textures, anim, Some(&tree)).ok()
            } else {
                Model::from_objects_anim(&asset.objects, &asset.textures, Some(anim), &[&tree])
            };
            missiles[i] = m
                .filter(|m| !m.mesh.indices.is_empty())
                .map(|m| self.add_model(m));
        }
        self.classes.push(MonsterClass {
            name: dir,
            models,
            gen_models,
            etype,
            missiles,
        });
        Some(self.classes.len() - 1)
    }

    /// Whether a point is on the 4:3 screen, give or take `margin` units.
    fn on_screen(&self, pos: Vec3, margin: f32) -> bool {
        match crate::scene::project(self, pos) {
            Some((x, y)) => {
                let m = margin * 4.0;
                (-m..512.0 + m).contains(&x) && (-m..384.0 + m).contains(&y)
            }
            None => false,
        }
    }

    /// items.c case 0xD / AudioSecretProc: each ambient sound's volume (0..1),
    /// full within its radius and fading out by 1.5x the radius from the
    /// nearest hero; quiet (16/127) during Sumner's speeches and trigger cameras.
    pub fn ambient_levels(&self) -> Vec<(u64, &str, f32)> {
        let hushed = self
            .sumner
            .as_ref()
            .is_some_and(|s| s.cam.is_some() || s.controls_locked());
        self.ambient_items
            .iter()
            .enumerate()
            .map(|(i, (pos, range, name))| {
                let d = self
                    .players
                    .iter()
                    .filter(|p| p.state.active())
                    .map(|p| (p.pos - *pos).length())
                    .fold(f32::MAX, f32::min);
                let mut vol = if d == f32::MAX {
                    0.0
                } else if d < *range || *range <= 2.0 {
                    1.0
                } else {
                    (2.0 * (1.5 * range - d) / range).clamp(0.0, 1.0)
                };
                if hushed && vol > 0.0 {
                    vol = 16.0 / 127.0;
                }
                (i as u64, name.as_str(), vol)
            })
            .collect()
    }

    /// items.c case 0xD: the highest music area whose sound item has a hero
    /// within its radius is asked for.
    fn update_music_area(&mut self) {
        let mut best: Option<(usize, i16)> = None;
        for &(pos, radius, area, fade) in &self.music_items {
            let near = self
                .players
                .iter()
                .filter(|p| p.state.active())
                .map(|p| (p.pos - pos).length())
                .fold(f32::MAX, f32::min);
            if near < radius && best.is_none_or(|b| area > b.0) {
                best = Some((area, fade));
            }
        }
        if let Some((area, fade)) = best {
            if area != self.music_area.0 {
                self.music_area = (area, fade);
            }
        }
    }

    /// fn_80060114: a placed monster appears when a sphere four times its
    /// visibility radius is on screen and it is within 50 of the camera's
    /// look-at point (never in the attract flyby).
    fn spawn_placed_enemies(&mut self) {
        if self.fixed_camera.is_some() || self.pending_enemies.is_empty() {
            return;
        }
        let (_, at, _) = self.camera_view();
        let ready: Vec<usize> = (0..self.pending_enemies.len())
            .filter(|&i| {
                let p = &self.pending_enemies[i];
                (p.pos - at).length() <= 50.0 && self.on_screen(p.pos, 4.0 * p.visrad)
            })
            .collect();
        for i in ready.into_iter().rev() {
            let p = self.pending_enemies.remove(i);
            self.spawn(p.kind, p.model, p.pos, p.yaw);
        }
    }

    /// generate_enemy: place a generator's monster in front of it (or around it,
    /// for small monsters) where there's floor and no wall in the way.
    fn spawn_from_generator(&mut self, sp: Spawn) {
        use crate::enemies::TYPES;
        let etype = self.classes[sp.class].etype;
        let t = &TYPES[etype.min(33)];
        if sp.recycle {
            // find_enemy_slot: the farthest monster, off screen ones first.
            let focus = self.cam_focus;
            let victim = self
                .entities
                .iter()
                .enumerate()
                .filter(|(_, e)| {
                    e.alive && matches!(e.kind, Kind::Enemy { etype, .. } if etype != crate::enemies::IT)
                })
                .map(|(i, e)| {
                    let mut score = (e.pos - focus).length();
                    if !self.on_screen(e.pos, 15.0) {
                        score += 10_000.0;
                    }
                    (i, score)
                })
                .filter(|(_, s)| *s >= 10_000.0)
                .max_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(i, _)| i);
            match victim {
                Some(i) => self.entities[i].alive = false,
                None => return,
            }
        }
        // Medium and large monsters only use front and +-45 degrees.
        const OCTANTS: [f32; 8] = [0.0, 180.0, -90.0, 90.0, 45.0, -45.0, 135.0, -135.0];
        let dirs: &[usize] = if crate::enemies::is_small(etype) {
            &[0, 1, 2, 3, 4, 5, 6, 7]
        } else {
            &[0, 4, 5]
        };
        let dist = if etype == 17 { 0.0 } else { 5.0 + t.radius };
        let start = self.rng.next_u32() as usize % dirs.len();
        for k in 0..dirs.len() {
            let a = sp.yaw + OCTANTS[dirs[(start + k) % dirs.len()]].to_radians();
            let p = sp.pos + Vec3::new(a.sin(), 0.0, a.cos()) * dist;
            let from = sp.pos + Vec3::Y * 2.0;
            if dist > 0.0 && self.collision.raycast(from, p + Vec3::Y * 2.0).is_some() {
                continue;
            }
            let Some(g) = self.collision.ground_height(p, sp.pos.y + 4.0) else {
                continue;
            };
            if (g - sp.pos.y).abs() > 6.0 {
                continue;
            }
            let model = self.classes[sp.class].model(sp.strength);
            let kind = self.new_enemy(sp.class, sp.strength, EnemyState::Spawning);
            let id = self.spawn(kind, model, Vec3::new(p.x, g, p.z), a);
            if let Some(start) = model
                .and_then(|m| self.monster_seqs.get(&m))
                .and_then(|s| s.start)
            {
                if let Some(e) = self.entities.iter_mut().find(|e| e.id == id) {
                    e.animator.restart(start);
                    e.animator.force_loop = false;
                }
            }
            if let Some(Kind::Generator { spawned, .. }) = self
                .entities
                .iter_mut()
                .find(|e| e.id == sp.generator)
                .map(|e| &mut e.kind)
            {
                spawned.push(id);
            }
            return;
        }
    }

    /// A new monster of `class` at a tier, as init_enemy sets it up.
    pub(crate) fn new_enemy(&self, class: usize, strength: usize, state: EnemyState) -> Kind {
        let etype = self.classes[class].etype;
        let t = &crate::enemies::TYPES[etype.min(33)];
        Kind::Enemy {
            class,
            strength,
            hp: crate::enemies::health(etype, strength as i32, self.tuning.health),
            state,
            cooldown: 0.0,
            speed: t.speed * 60.0 * self.tuning.speed,
            etype,
            recognized: false,
            attacks: 0,
            target: None,
            reload: 0.0,
            drained: 0.0,
            blink: 0.0,
        }
    }

    pub(crate) fn spawn(&mut self, kind: Kind, model: Option<usize>, pos: Vec3, yaw: f32) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let seq = match (&kind, model) {
            (Kind::Enemy { .. }, Some(m)) => self
                .monster_seqs
                .get(&m)
                .and_then(|s| s.start.or(s.ready))
                .unwrap_or(0),
            _ => 0,
        };
        let mut animator = Animator::new(seq);
        animator.force_loop = !matches!(
            kind,
            Kind::Enemy { .. } | Kind::Critter(_) | Kind::Effect { .. }
        );
        self.entities.push(Entity {
            id,
            solid: None,
            kind,
            model,
            pos,
            yaw,
            animator,
            spin: 0.0,
            alive: true,
            fade: 1.0,
        });
        id
    }

    fn set_solid(&mut self, id: u64, shape: Shape) {
        if let Some(e) = self.entities.iter_mut().find(|e| e.id == id) {
            e.solid = Some(shape);
        }
    }

    /// Push a circle of `radius` at `pos` out of every solid entity.
    fn resolve_solids(entities: &[Entity], mut pos: Vec3, radius: f32, skip: u64) -> Vec3 {
        for e in entities {
            let Some(shape) = e.solid else { continue };
            if !e.alive || e.id == skip || (e.pos.y - pos.y).abs() > 4.0 {
                continue;
            }
            let d = Vec2::new(pos.x - e.pos.x, pos.z - e.pos.z);
            match shape {
                Shape::Cylinder { radius: r } => {
                    let len = d.length();
                    let min = r + radius;
                    if len < min {
                        let n = if len > 1e-4 { d / len } else { Vec2::X };
                        pos.x = e.pos.x + n.x * min;
                        pos.z = e.pos.z + n.y * min;
                    }
                }
                Shape::Box { half } => {
                    // Work in the box's local frame.
                    let (s, c) = e.yaw.sin_cos();
                    let local = Vec2::new(d.x * c - d.y * s, d.x * s + d.y * c);
                    let clamped = local.clamp(-half, half);
                    let diff = local - clamped;
                    let len = diff.length();
                    let push_local = if len > 1e-4 {
                        if len >= radius {
                            continue;
                        }
                        diff / len * (radius - len)
                    } else {
                        // Centre inside the box: push out along the shallowest axis.
                        let px = half.x + radius - local.x.abs();
                        let pz = half.y + radius - local.y.abs();
                        if px < pz {
                            Vec2::new(px * local.x.signum(), 0.0)
                        } else {
                            Vec2::new(0.0, pz * local.y.signum())
                        }
                    };
                    let push = Vec2::new(
                        push_local.x * c + push_local.y * s,
                        -push_local.x * s + push_local.y * c,
                    );
                    pos.x += push.x;
                    pos.z += push.y;
                }
            }
        }
        pos
    }

    fn item_model(
        &mut self,
        fs: &GameFs,
        cache: &mut HashMap<String, Option<AssetFile>>,
        known: &mut HashMap<String, Option<usize>>,
        dirs: &[String],
        name: &str,
    ) -> Option<usize> {
        if name.is_empty() {
            return None;
        }
        if let Some(m) = known.get(name) {
            return *m;
        }
        let mut found = None;
        for dir in dirs {
            let Some(asset) = cache
                .entry(dir.clone())
                .or_insert_with(|| load_asset(fs, dir))
                .as_ref()
            else {
                continue;
            };
            if let Some(anim) = &asset.anim {
                if anim
                    .atrees
                    .iter()
                    .any(|t| t.name.eq_ignore_ascii_case(name))
                {
                    if let Ok(m) =
                        Model::from_anim(&asset.objects, &asset.textures, anim, Some(name))
                    {
                        if !m.mesh.indices.is_empty() {
                            found = Some(m);
                            break;
                        }
                    }
                }
            }
            if let Some(m) = Model::from_objects_anim(
                &asset.objects,
                &asset.textures,
                asset.anim.as_ref(),
                &[name],
            ) {
                found = Some(m);
                break;
            }
        }
        let idx = found.map(|m| self.add_model(m));
        known.insert(name.to_string(), idx);
        idx
    }

    /// Create entities for the level's item placements.
    fn populate(&mut self, fs: &GameFs, num_players: usize) {
        let mut cache: HashMap<String, Option<AssetFile>> = HashMap::new();
        let mut known: HashMap<String, Option<usize>> = HashMap::new();
        let world_letter = self
            .level
            .name
            .trim_start_matches("level")
            .chars()
            .next()
            .unwrap_or('A');
        let dirs = [
            format!("ITEMS/level{world_letter}"),
            format!("ITEMS/{}", self.level.name),
            "POWERUPS".to_string(),
        ];

        let instances = self.level.worlds.item_instances.clone();
        let infos = self.level.worlds.item_infos.clone();
        for inst in &instances {
            if inst.min_players as usize > num_players.max(1) {
                continue;
            }
            let Some(mut info) = infos.get(inst.item_index as usize).cloned() else {
                continue;
            };
            // Random items pick one of their candidates.
            if info.item_type == ItemType::Random && !info.random_indices.is_empty() {
                let pick =
                    info.random_indices[self.rng.next_u32() as usize % info.random_indices.len()];
                match infos.get(pick as usize) {
                    Some(i) => info = i.clone(),
                    None => continue,
                }
            }
            let pos = Vec3::from(inst.pos);
            // Placement rotations use the game's pitch/yaw/roll convention; items stand
            // upright, so reduce to the yaw that the rotation turns +z towards.
            let fwd = crate::character::ryp_matrix(inst.rot).transform_vector3(Vec3::Z);
            let yaw = fwd.x.atan2(fwd.z);
            let p16 = |k: usize| i16::from_le_bytes([inst.params[k], inst.params[k + 1]]);

            match info.item_type {
                // Placed monsters: generic names (GRU, KNI, ...) become the realm's
                // monster; strength 0 sleeps, 1-3 is the tier, 4-6 the
                // archer / bomber / suicide variants.
                ItemType::Enemy => {
                    let strength = p16(0).clamp(0, 6) as usize;
                    let Some(ty) = crate::enemies::type_of(&info.name) else {
                        continue;
                    };
                    // IT only plays the tag game with company.
                    if ty == crate::enemies::IT && num_players < 2 {
                        continue;
                    }
                    // fn_80060114: golems, gargoyles and generals are critters.
                    if matches!(ty, 29 | 32 | 33) {
                        let variant = self.enemy_variant(ty);
                        let realm = self.wave.map_or(0, |w| w >> 8);
                        let wad = match ty {
                            29 if realm == 9 => "golemI.wad".to_string(),
                            29 if realm == 6 => "golemF.wad".to_string(),
                            29 => "GOLEM.WAD".to_string(),
                            33 => "GENERAL.WAD".to_string(),
                            _ => format!(
                                "GAR_{}.WAD",
                                variant
                                    .clone()
                                    .unwrap_or_else(|| "eagl".into())
                                    .to_ascii_uppercase()
                            ),
                        };
                        if ty == 32 {
                            let kind = variant.clone().unwrap_or_else(|| "eagl".into());
                            let name = format!("GARG{}", kind.to_ascii_uppercase());
                            self.item_model(fs, &mut cache, &mut known, &dirs, &name);
                        }
                        self.spawn_critter(fs, &wad, variant.as_deref(), pos, yaw, num_players);
                        continue;
                    }
                    let ty = self.roster.resolve(ty, strength as i32);
                    if let Some(c) = self.monster_class(fs, &mut cache, ty) {
                        let model = self.classes[c].model(strength.max(1));
                        let state = if strength == 0 && ty != crate::enemies::DEATH {
                            EnemyState::Asleep
                        } else {
                            EnemyState::Idle
                        };
                        let mut kind = self.new_enemy(c, strength.max(1), state);
                        if let Kind::Enemy { cooldown, .. } = &mut kind {
                            // A half-second freeze as it appears.
                            *cooldown = if strength < 4 { 0.5 } else { 0.0 };
                        }
                        // fn_80060114: it appears once it comes into view.
                        self.pending_enemies.push(PendingEnemy {
                            kind,
                            model,
                            pos,
                            yaw,
                            visrad: info.radius.max(1.0),
                        });
                    }
                }
                // SetItem case 3: tier, max alive and interval (with the level's
                // gen_max / gen_rate), 10 x tier x gen_health hit points.
                ItemType::Generator => {
                    let strength = (p16(0).clamp(1, 3)) as usize;
                    let special = if info.name.eq_ignore_ascii_case("SPECIAL") {
                        match self.wave.map(|w| w >> 8) {
                            Some(5) => Some(&crate::enemies::TEMPLE_SPECIAL),
                            Some(6) => Some(&crate::enemies::HELL_SPECIAL),
                            _ => None,
                        }
                    } else {
                        None
                    };
                    let ty = match special {
                        Some(list) => list[0],
                        None => match crate::enemies::type_of(&info.name) {
                            Some(t) => self.roster.resolve(t, 0),
                            None => continue,
                        },
                    };
                    let base_max = match p16(4) {
                        m if m > 0 => m as f32,
                        _ => [10.0, 5.0, 2.0][strength - 1],
                    };
                    let base_interval = match inst.params[6] {
                        0 => [5.0, 10.0, 15.0][strength - 1],
                        n => n as f32,
                    };
                    let max = ((base_max * self.tuning.gen_max) as i32).max(1) as usize;
                    let interval = (base_interval * self.tuning.gen_rate).floor().max(1.0);
                    if let Some(c) = self.monster_class(fs, &mut cache, ty) {
                        // Temple / Hell monsters all come along.
                        if let Some(list) = special {
                            for &t in list.iter() {
                                self.monster_class(fs, &mut cache, t);
                            }
                        }
                        let model = self.classes[c].gen_model(strength);
                        let hp = 10.0 * strength as f32 * self.tuning.gen_health;
                        let id = self.spawn(
                            Kind::Generator {
                                class: c,
                                strength,
                                hp,
                                timer: 0.0,
                                interval,
                                max,
                                spawned: Vec::new(),
                                etype: ty,
                                genratio: 0.0,
                                always: inst.flags & 1 != 0,
                                special,
                            },
                            model,
                            pos,
                            yaw,
                        );
                        self.set_solid(
                            id,
                            Shape::Cylinder {
                                radius: info.radius.max(1.0),
                            },
                        );
                    }
                }
                ItemType::Powerup => {
                    let kind = pickup_kind(&info, &inst.params);
                    let model = self.item_model(fs, &mut cache, &mut known, &dirs, &info.name);
                    self.spawn(
                        Kind::Pickup {
                            kind,
                            radius: info.radius.max(1.0) + 1.0,
                        },
                        model,
                        pos,
                        yaw,
                    );
                }
                ItemType::Container | ItemType::Obstacle => {
                    let model = self.item_model(fs, &mut cache, &mut known, &dirs, &info.name);
                    if model.is_none() {
                        continue;
                    }
                    let drop = (info.item_type == ItemType::Container)
                        .then(|| i32::from_le_bytes(inst.params[0..4].try_into().unwrap()))
                        .filter(|&i| i >= 0)
                        .map(|i| i as usize);
                    let id = self.spawn(
                        Kind::Container {
                            hp: info.hit_points.max(1) as f32,
                            drop,
                            chest: info.item_type == ItemType::Container,
                            armor: info.armor.max(0) as f32,
                        },
                        model,
                        pos,
                        yaw,
                    );
                    self.set_solid(id, item_shape(&info));
                }
                ItemType::Door => {
                    let model = self.item_model(fs, &mut cache, &mut known, &dirs, &info.name);
                    let coll = (inst.coll_tri_count > 0 && inst.coll_tri_index >= 0)
                        .then(|| (inst.coll_tri_index as usize, inst.coll_tri_count as usize));
                    let id = self.spawn(
                        Kind::Door {
                            radius: info.radius.max(2.0) + 1.5,
                            coll,
                        },
                        model,
                        pos,
                        yaw,
                    );
                    self.set_solid(id, item_shape(&info));
                }
                // A secret icon names its secret level ("S3").
                ItemType::Exit if info.subtype == 50 => {
                    let model = self.item_model(fs, &mut cache, &mut known, &dirs, &info.name);
                    let code = String::from_utf8_lossy(&inst.params[4..8])
                        .trim_end_matches('\0')
                        .to_string();
                    let wave = self.worlds.find_wave(&code);
                    self.spawn(
                        Kind::Secret {
                            radius: info.radius.max(2.0) + 0.5,
                            wave,
                            used: false,
                        },
                        model,
                        pos,
                        yaw,
                    );
                }
                ItemType::Exit => {
                    let model = self.item_model(fs, &mut cache, &mut known, &dirs, &info.name);
                    let dest = self.exit_dest(&inst.params);
                    self.spawn(
                        Kind::Exit {
                            radius: info.radius.max(2.0) + 0.5,
                            dest,
                            open: true,
                        },
                        model,
                        pos,
                        yaw,
                    );
                }
                ItemType::DamageTile | ItemType::Trap => {
                    let model = self.item_model(fs, &mut cache, &mut known, &dirs, &info.name);
                    self.spawn(
                        Kind::Hazard {
                            radius: info.radius.max(1.0),
                            damage: 20.0,
                        },
                        model,
                        pos,
                        yaw,
                    );
                }
                ItemType::Trigger => {
                    let model = self.item_model(fs, &mut cache, &mut known, &dirs, &info.name);
                    let (mode, flags8) = trigger_flags(info.subtype, p16(2) as u16);
                    let target = p16(0);
                    let mover = (target >= 0
                        && (target as usize) < self.level.worlds.world_objects.len())
                    .then(|| {
                        let t = target as usize;
                        // Several triggers can share a target (RegisterItemWobj).
                        self.movers
                            .iter()
                            .position(|m| m.root == t)
                            .unwrap_or_else(|| {
                                let m = Mover::new(
                                    &self.level,
                                    &self.collision,
                                    t,
                                    flags8,
                                    p16(8) as f32 * 0.1,
                                    p16(10) as f32 * 0.1,
                                );
                                m.place(&mut self.collision);
                                self.movers.push(m);
                                self.movers.len() - 1
                            })
                    });
                    self.spawn(
                        Kind::Trigger {
                            mover,
                            mode,
                            radius: info.radius.max(0.7),
                            idle: 0.0,
                            shoot: info.subtype_name() == "shoot_trigger",
                            pressed: false,
                            hit: false,
                            // Tower barriers (flag 0x40): ids 1-8 need that colour's
                            // crystals, 101-103 the gargoyle items.
                            gate: (p16(2) & 0x40 != 0
                                && matches!(inst.params[6], 1..=8 | 101..=103))
                            .then_some(inst.params[6] as usize),
                            id: inst.params[6],
                        },
                        model,
                        pos,
                        yaw,
                    );
                }
                // SetItem case 13: a sound item with a music area selects it
                // while a hero is within its radius.
                ItemType::Sound => {
                    let radius = f32::from_le_bytes(inst.params[0..4].try_into().unwrap());
                    let area = i32::from_le_bytes(inst.params[4..8].try_into().unwrap());
                    let fade = i16::from_le_bytes(inst.params[8..10].try_into().unwrap());
                    if area > 0 {
                        self.music_items
                            .push((pos, radius, area as usize - 1, fade));
                    } else if !inst.name.is_empty() {
                        // Otherwise it loops its sound (named by the instance).
                        self.ambient_items
                            .push((pos, radius, inst.name.to_ascii_uppercase()));
                    }
                }
                ItemType::Transporter | ItemType::Rotator => {
                    let model = self.item_model(fs, &mut cache, &mut known, &dirs, &info.name);
                    if model.is_some() {
                        self.spawn(Kind::Prop, model, pos, yaw);
                    }
                }
                _ => {}
            }
        }
        // The boss, its coins and key, and the good wizard (boss levels).
        self.merge_item_models(std::mem::take(&mut known));
        self.spawn_boss(fs, num_players);
        if self.boss.is_some() {
            let dir = format!("ITEMS/{}", self.level.name);
            if let Some(asset) = cache
                .entry(dir.clone())
                .or_insert_with(|| load_asset(fs, &dir))
                .as_ref()
            {
                if let Some(anim) = &asset.anim {
                    if let Ok(m) =
                        Model::from_anim(&asset.objects, &asset.textures, anim, Some("WIZARD"))
                    {
                        self.wizard_model = Some(self.add_model(m));
                    }
                }
            }
        }
        known = self.item_models.clone();
        // Load the models of chest and barrel contents too, for when they pop out.
        for inst in &instances {
            if let Some(c) = infos
                .get(i32::from_le_bytes(inst.params[0..4].try_into().unwrap()).max(0) as usize)
                .filter(|_| {
                    matches!(
                        infos.get(inst.item_index as usize).map(|i| i.item_type),
                        Some(ItemType::Container)
                    )
                })
            {
                if c.item_type == ItemType::Powerup {
                    self.item_model(fs, &mut cache, &mut known, &dirs, &c.name);
                }
            }
        }
        // Death comes out of chests in any level without a boss.
        if self
            .roster
            .subtype
            .iter()
            .any(|(t, _)| *t == crate::enemies::DEATH)
        {
            self.monster_class(fs, &mut cache, crate::enemies::DEATH);
        }
        // The tower's Sumners and the shard and runestone effects.
        if self.wave == Some(gdl_formats::wdata::TOWER_WAVE) {
            let mut names: Vec<String> = vec!["GWIZ".into(), "WIZARD".into()];
            names.extend((1..=8).map(|n| format!("SHARD{n}")));
            names.extend((1..=13).map(|n| format!("RUNE{n}")));
            for n in names {
                self.item_model(fs, &mut cache, &mut known, &dirs, &n);
            }
        }
        self.item_models = known
            .iter()
            .map(|(k, v)| (k.to_ascii_uppercase(), *v))
            .collect();
        let count = |f: fn(&Kind) -> bool| self.entities.iter().filter(|e| f(&e.kind)).count();
        log::info!(
            "{}: {} enemies, {} generators, {} pickups, {} containers, {} doors, {} models loaded",
            self.level.name,
            count(|k| matches!(k, Kind::Enemy { .. })),
            count(|k| matches!(k, Kind::Generator { .. })),
            count(|k| matches!(k, Kind::Pickup { .. })),
            count(|k| matches!(k, Kind::Container { .. })),
            count(|k| matches!(k, Kind::Door { .. })),
            self.models.len()
        );
    }

    /// The ENMY variant string of the level's record for monster type `ty`
    /// (gargoyle kind, golem model directory), if any.
    fn enemy_variant(&self, ty: usize) -> Option<String> {
        let wave = self.wave?;
        let r = self.worlds.realm(wave >> 8)?;
        let l = self.worlds.level(wave)?;
        let pick = l
            .enemy_types
            .iter()
            .filter(|&&i| i >= 0)
            .map(|&i| i as usize)
            .find(|&i| r.enemies.get(i).is_some_and(|e| e.0 == ty as i32))
            .or_else(|| r.enemies.iter().position(|e| e.0 == ty as i32))?;
        r.enemy_variants
            .get(pick)
            .filter(|v| !v.is_empty())
            .cloned()
    }

    /// Camera-relative movement basis on the ground plane (forward, right), in game coordinates.
    pub fn camera_basis(&self) -> (Vec2, Vec2) {
        // Forward on the ground plane, and screen-right (accounting for the render-time X mirror).
        let f = Vec2::new(-self.cam_yaw.sin(), -self.cam_yaw.cos());
        (f, Vec2::new(f.y, -f.x))
    }

    /// The level's game-camera locator nearest to `p`: (yaw, pitch).
    fn camera_locator(&self, p: Vec3) -> Option<(f32, f32)> {
        use gdl_formats::worlds::LocatorType;
        self.level
            .worlds
            .locators
            .iter()
            .filter(|l| l.kind == LocatorType::CameraGame)
            .min_by(|a, b| {
                let da = (Vec3::from(a.pos) - p) * Vec3::new(1.0, 0.5, 1.0);
                let db = (Vec3::from(b.pos) - p) * Vec3::new(1.0, 0.5, 1.0);
                da.length_squared().total_cmp(&db.length_squared())
            })
            .map(|l| (l.rot[1], l.rot[0]))
    }

    fn snap_to_ground(collision: &Collision, pos: &mut Vec3, vy: &mut f32, dt: f32) -> bool {
        *vy -= GRAVITY * dt;
        pos.y += *vy * dt;
        match collision.ground_height(*pos, pos.y + STEP_HEIGHT) {
            Some(g) if pos.y <= g || (pos.y - g < 0.3 && *vy <= 0.0) => {
                pos.y = g;
                *vy = 0.0;
                true
            }
            _ => false,
        }
    }

    /// do_weakening's heartbeat (S_WARN): for the living player with the least
    /// health, while it is 200 or less, every 120 ticks above 100, 60 above 25 and
    /// 30 below.
    /// do_players: the level is over once nobody is still playing. If anyone left
    /// through an exit the party follows them; otherwise everyone died.
    fn check_level_end(&mut self) {
        if self.level_end.is_some()
            || self.players.is_empty()
            || self.players.iter().any(|p| {
                p.state.active()
                    || matches!(
                        p.state,
                        PlayerState::Exiting { .. } | PlayerState::Dead { .. }
                    )
            })
        {
            return;
        }
        self.level_end = Some(
            match self.players.iter().find(|p| p.state == PlayerState::Exited) {
                Some(p) => LevelEnd::Exit(p.exit_dest),
                None => LevelEnd::AllDead,
            },
        );
    }

    fn heartbeat(&mut self) {
        let low = self
            .players
            .iter()
            .filter(|p| p.state.active() && p.health > 1.0)
            .map(|p| p.health)
            .min_by(|a, b| a.total_cmp(b));
        match low {
            Some(h) if h <= 200.0 => {
                self.heartbeat -= 1;
                if self.heartbeat <= 0 {
                    self.sfx.push("S_WARN".into());
                    self.heartbeat = if h >= 100.0 {
                        120
                    } else if h >= 25.0 {
                        60
                    } else {
                        30
                    };
                }
            }
            _ => self.heartbeat = 0,
        }
    }

    pub fn update(&mut self, dt: f32, inputs: &[PlayerInput]) {
        self.time += dt;
        if let Some((_, t)) = &mut self.crystal_shown {
            *t -= dt;
            if *t <= 0.0 {
                self.crystal_shown = None;
            }
        }
        if let Some((_, left)) = &mut self.secret_timer {
            let before = left.ceil() as i32;
            *left -= dt;
            let now = left.ceil() as i32;
            if before != now && now == 8 {
                self.sfx.push("S_TIMEISRUNNING".into());
            }
            if *left <= 0.0 && self.level_end.is_none() {
                self.level_end = Some(LevelEnd::SecretOver);
            }
        }
        self.msgs.tick();
        self.welcome = (self.welcome - 1).max(0);
        let voices: Vec<String> = self.msgs.voice.drain(..).collect();
        self.sfx.extend(voices);
        self.frame += 1;
        // Sumner's speeches and trigger cameras take the controls away.
        let blank;
        let mut inputs = inputs;
        if let Some(mut s) = self.sumner.take() {
            s.update(self);
            if s.controls_locked() {
                blank = vec![PlayerInput::default(); inputs.len()];
                inputs = &blank;
            }
            self.sumner = Some(s);
        }
        let inputs = self.legend_freeze_inputs(inputs);
        self.update_players(dt, &inputs);
        self.update_music_area();
        self.legend_face_boss();
        self.update_rings(dt);
        self.spawn_placed_enemies();
        self.update_entities(dt);
        self.update_critters(dt);
        self.update_waves(dt);
        self.update_spews(dt);
        self.update_boss(dt);
        self.heartbeat();
        self.check_level_end();
        if self
            .boss
            .as_ref()
            .and_then(|b| b.cam.as_ref())
            .is_some_and(|c| c.started)
        {
            return;
        }

        // Camera follows the centroid of living players.
        let alive: Vec<Vec3> = self
            .players
            .iter()
            .filter(|p| p.state.active())
            .map(|p| p.pos)
            .collect();
        let pts = if alive.is_empty() {
            self.players.iter().map(|p| p.pos).collect()
        } else {
            alive
        };
        if !pts.is_empty() {
            let c = pts.iter().copied().sum::<Vec3>() / pts.len() as f32;
            let k = 1.0 - (-dt * 6.0).exp();
            self.cam_focus = self.cam_focus.lerp(c, k);
        }
        // Turn the camera gently towards the local game camera's heading.
        if let Some((yaw, pitch)) = self.camera_locator(self.cam_focus) {
            let mut d = yaw - self.cam_yaw;
            while d > std::f32::consts::PI {
                d -= std::f32::consts::TAU;
            }
            while d < -std::f32::consts::PI {
                d += std::f32::consts::TAU;
            }
            let k = 1.0 - (-dt * 1.5).exp();
            self.cam_yaw += d * k;
            self.cam_pitch += (pitch.clamp(0.7, 1.3) - self.cam_pitch) * k;
        }
    }

    fn update_players(&mut self, dt: f32, inputs: &[PlayerInput]) {
        let (fwd, right) = self.camera_basis();
        // Swings landing (player, damage multiplier, knockdown), throws (x size),
        // potion blasts, breath and hammer power-ups, damage rows firing.
        let mut swings: Vec<(usize, f32, bool)> = Vec::new();
        let mut throws: Vec<(usize, Vec3, f32, f32)> = Vec::new();
        let mut rings: Vec<(usize, Vec3, u8)> = Vec::new();
        let mut breaths: Vec<usize> = Vec::new();
        let mut hammers: Vec<usize> = Vec::new();
        let mut damg: Vec<(usize, gdl_formats::pdata::DamageRow)> = Vec::new();
        let mut posts: Vec<(usize, usize)> = Vec::new();
        let mut key_drops: Vec<(Vec3, u32)> = Vec::new();
        let in_tower = self.wave == Some(gdl_formats::wdata::TOWER_WAVE);
        let pu_rate = if self.boss.as_ref().is_some_and(|b| b.active && !b.dead) {
            3.0
        } else {
            1.0
        };
        let mut it_cleared = false;
        // Things an attack can be aimed at, with their radius.
        // "Short" targets (small monsters, breakables) get kicks and low swings.
        let mut targets: Vec<(Vec3, f32, bool)> = self
            .entities
            .iter()
            .filter(|e| e.alive && !matches!(e.kind, Kind::Critter(_)))
            .filter_map(|e| {
                let short = match &e.kind {
                    Kind::Enemy { etype, .. } => {
                        crate::enemies::TYPES[(*etype).min(33)].height * 0.5 <= 2.0
                    }
                    Kind::Container { .. } => true,
                    _ => false,
                };
                Some((e.pos, Self::target_radius(e)?, short))
            })
            .collect();
        // Critters are aimed at by their colnodes (at the players' height).
        for e in self.entities.iter().filter(|e| e.alive) {
            if let Kind::Critter(c) = &e.kind {
                if !c.hittable() {
                    continue;
                }
                let feet = self.players.first().map_or(e.pos.y, |p| p.pos.y);
                for (_, _, p, r) in c.colliders() {
                    targets.push((Vec3::new(p.x, feet, p.z), r, false));
                }
                let cp = c.coll_pos(0, e.pos, e.yaw);
                targets.push((
                    Vec3::new(cp.x, feet, cp.z),
                    c.wad.types[c.parts[0].ty].rad,
                    false,
                ));
            }
        }
        // fn_80088EF4's candidates: who could be grabbed for a combo.
        let combo_info: Vec<(Vec3, bool, u8)> = self
            .players
            .iter()
            .map(|p| {
                let free = p.state.active()
                    && p.grab_partner.is_none()
                    && p.grab_pending.is_none()
                    && p.fx.special & 0x400 == 0;
                (p.pos, free, (p.class_id % 8) as u8)
            })
            .collect();
        for (i, p) in self.players.iter_mut().enumerate() {
            let mut inp = inputs.get(i).copied().unwrap_or_default();
            // The d-pad runs the mini-inventory; only the stick walks.
            inp.stick -= inp.dpad;
            let seqs = &self.player_seqs[p.model];
            let model = &self.models[p.model];
            p.hurt_flash = (p.hurt_flash - dt * 3.0).max(0.0);
            for snd in p.inv.update(&mut p.powerups, inp.inv) {
                self.sfx.push(snd.into());
            }
            // A turbo boost switched on fills the meter and is used up.
            for sl in &mut p.powerups.slots {
                if sl.state == crate::powerups::ON
                    && sl.kind == crate::powerups::SPECIAL
                    && sl.flags & crate::powerups::S_TURBO != 0
                {
                    p.power = 100.0;
                    if sl.time >= 0.0 {
                        *sl = Default::default();
                    }
                }
            }
            // PlayerProcessPowerups: timed power-ups run down outside the tower,
            // three times as fast during a boss fight.
            if !in_tower {
                p.powerups.tick(dt * pu_rate);
            }
            p.fx = p.powerups.effects();
            if !in_tower {
                p.stats.playtime += dt;
            }
            p.refresh_stats();
            p.derived.speed = (p.derived.speed + p.fx.speed).min(12.5);
            p.derived.magic_power = (p.derived.magic_power + p.fx.magic).min(32.0);

            match &mut p.state {
                PlayerState::Dead { timer } => {
                    // The death animation plays out, then inactivate_player: the
                    // player waits "in tower" until the level ends.
                    *timer -= dt;
                    if *timer <= 0.0 {
                        p.state = PlayerState::InTower;
                        p.tower_choice = true;
                    }
                    p.animator.update(&model.skeleton, dt);
                    continue;
                }
                PlayerState::Exiting { timer } => {
                    // do_exit: sink and spin into the exit.
                    *timer -= dt;
                    p.pos.y -= dt * 2.4;
                    p.yaw += dt * 12.0;
                    if *timer <= 0.0 {
                        p.state = PlayerState::Exited;
                    }
                    p.animator.update(&model.skeleton, dt);
                    continue;
                }
                PlayerState::InTower => {
                    // A waits in the tower; B quits the game (abort_player).
                    if p.tower_choice {
                        if inp.attack_pressed {
                            p.tower_choice = false;
                        } else if inp.b_pressed {
                            p.tower_choice = false;
                            p.quit = true;
                        }
                    }
                    continue;
                }
                PlayerState::Exited => continue,
                _ => {}
            }

            if p.health <= 0.0 {
                // damage_player: dying empties the turbo meter; the keys drop.
                p.health = 0.0;
                p.power = 0.0;
                p.powerups.clear();
                if self.it_holder == Some(i) {
                    it_cleared = true;
                }
                p.state = PlayerState::Dead { timer: 4.0 };
                self.deaths += 1;
                self.sfx.push(format!("S_{}DIE1", p.class));
                if let Some(d) = seqs.death {
                    p.animator.restart(d);
                }
                if p.keys > 0 {
                    key_drops.push((p.pos, p.keys));
                    p.keys = 0;
                }
                continue;
            }
            use crate::paction::{act, bits, p as pa};
            let ct = (p.class_id % 8) as u8;
            // The stick as a world direction (camera relative), and the
            // logical buttons (controls.c): held levels and new edges.
            let wish = fwd * inp.stick.y + right * inp.stick.x;
            let joymag = wish.length().min(1.0);
            let joyang = if joymag > 0.0 {
                wish.x.atan2(wish.y)
            } else {
                p.yaw
            };
            let mut levels = 0;
            let mut edges = 0;
            for (held, press, b) in [
                (inp.attack, inp.attack_pressed, bits::ATK_QUICK),
                (inp.slow, inp.slow_pressed, bits::ATK_SLOW),
                (inp.turbo, inp.b_pressed, bits::TURBO | bits::DEFEND),
                (inp.magic, inp.magic_pressed, bits::MAGIC),
                (false, inp.charge_pressed, bits::CHARGE),
                (inp.strafe, false, bits::STRAFE),
                (inp.combo, false, bits::COMBO),
            ] {
                if held || press {
                    levels |= b;
                }
                if press {
                    edges |= b;
                }
            }
            let mut ctx = crate::paction::Ctx {
                joymag,
                stick_yaw: wrap_angle(joyang - p.yaw),
                power: p.power,
                potions: p.potions,
                weapon: p.fx.weapon,
                special: p.fx.special,
                shield: p.fx.armor,
                stat_armor: p.derived.armor,
                char_type: ct,
                partner_ct: p.grab_partner.and_then(|g| combo_info.get(g)).map(|c| c.2),
                ..Default::default()
            };
            // A combo partner: the nearest free hero within 5 in a 45 degree
            // cone ahead, at about the same height.
            let facing = Vec2::new(p.yaw.sin(), p.yaw.cos());
            let candidate = if p.grab_partner.is_none()
                && p.grab_pending.is_none()
                && p.fx.special & 0x400 == 0
            {
                combo_info
                    .iter()
                    .enumerate()
                    .filter(|&(j, (pos, free, _))| {
                        let d = Vec2::new(pos.x - p.pos.x, pos.z - p.pos.z);
                        j != i
                            && *free
                            && d.length() <= 5.0
                            && (pos.y - p.pos.y).abs() <= 3.0
                            && d.normalize_or_zero().dot(facing) >= 0.707
                    })
                    .min_by(|a, b| {
                        let da = (a.1 .0 - p.pos).length();
                        let db = (b.1 .0 - p.pos).length();
                        da.total_cmp(&db)
                    })
                    .map(|(j, _)| j)
            } else {
                None
            };
            ctx.partner = candidate.is_some();
            let ms = p.act.motion_state(levels, edges, &ctx);
            p.act.tick_idle(1);
            if ms == 0x16 {
                p.grab_pending = candidate;
            }
            // PlayerKnockback: this frame's hits push and pick a reaction, unless
            // the hero is in an untouchable move.
            let mut reaction = 0u8;
            if p.hit_damage > 1.0 && p.act.action < 11 {
                let f = p.hit_flags;
                let pojo = p.fx.special & 0x400 != 0;
                let (k, r) = if f & DmgType::WHIRLWIND != 0 {
                    (100.0, 30)
                } else if f & DmgType::BLOWNAWAY != 0 {
                    (100.0, 20)
                } else if f & (DmgType::KNOCKDOWN | DmgType::KNOCKOVER) != 0 {
                    (if pojo { 80.0 } else { 32.0 }, 20)
                } else if f & DmgType::KNOCKBACK != 0 {
                    (16.0, 10)
                } else if f & DmgType::SPIKE != 0 {
                    (0.0, 3)
                } else if f & DmgType::STUN != 0 {
                    (0.0, 2)
                } else {
                    (0.0, 1)
                };
                p.knock_vel += p.hit_force * k;
                reaction = r;
                // Hit from behind: the forward fall.
                if r >= 10 && p.hit_force.length() > 0.0 {
                    let from = (-p.hit_force.x).atan2(-p.hit_force.z);
                    if wrap_angle(from - p.yaw).abs() > std::f32::consts::FRAC_PI_2 {
                        reaction += 1;
                    }
                }
            }
            p.hit_damage = 0.0;
            p.hit_flags = 0;
            p.hit_force = Vec3::ZERO;
            ctx.reaction = reaction;

            // PlayerGetTarget: the nearest thing in the cone around the heading
            // (the stick, else the facing), measured to its surface.
            let heading = if joymag > 0.0 { joyang } else { p.yaw };
            let hdir = Vec2::new(heading.sin(), heading.cos());
            let cone = if self.difficulty == 0 { 0.5 } else { 0.707 };
            let found = targets
                .iter()
                .filter(|(t, _, _)| (t.y - p.pos.y).abs() < 5.0)
                .filter_map(|(t, r, short)| {
                    let d = Vec2::new(t.x - p.pos.x, t.z - p.pos.z);
                    let surf = d.length() - r;
                    (surf < TARGET_RANGE && (d.length() < 0.5 || d.normalize().dot(hdir) >= cone))
                        .then_some((surf, d, *short))
                })
                .min_by(|a, b| a.0.total_cmp(&b.0));
            let (tdist, attack_yaw, short) = match found {
                Some((dist, d, short)) => (dist, d.x.atan2(d.y), short),
                None => (TARGET_RANGE, p.yaw, false),
            };
            // Strafing and blocking keep the stick's direction.
            let attack_yaw =
                if p.act.hud & 0xC000 != 0 || levels & (bits::STRAFE | bits::DEFEND) != 0 {
                    heading
                } else {
                    attack_yaw
                };
            ctx.melee_yaw = wrap_angle(attack_yaw - p.yaw);
            let r = PLAYER_RADIUS;
            let bias = if matches!(ms, 0x0F | 0x10) { 1.0 } else { 0.0 };
            let mut coll = if short && tdist < 2.0 + r { 2 } else { 0 };
            coll |= if tdist < 1.0 + r + bias {
                1
            } else if tdist < 2.0 + r + bias {
                4
            } else {
                8
            };
            ctx.coll = coll;
            if let Some(m) = p.act.request(ms, &ctx) {
                posts.push((m, i));
            }
            // A step swing turns by the stick, not the target.
            if p.act.next == pa::STEP {
                ctx.melee_yaw = ctx.stick_yaw;
            }
            p.act.do_action(&self.action_seqs[p.model], &ctx, dt);

            // The bits the action fired.
            let bits_now = std::mem::take(&mut p.act.act_bits);
            if bits_now & 0xFE != 0 {
                let (mult, knock) = if bits_now & 0xF0 != 0 {
                    // Power swings: x3 and knockdown; the move's turbo is paid.
                    p.power = (p.power - p.act.coll_score).max(0.0);
                    p.act.coll_score = 0.0;
                    (3.0, true)
                } else if bits_now & act::HIT2 != 0 {
                    (2.0, true)
                } else {
                    (1.0, bits_now & act::KICK != 0)
                };
                swings.push((i, mult, knock));
            }
            if bits_now & act::THROW != 0 {
                throws.push((i, p.pos, attack_yaw, 1.0));
            }
            if bits_now & act::PWRA_THROW != 0 {
                throws.push((i, p.pos, attack_yaw, 2.0));
            }
            if bits_now & (act::SUPER_SHOT | act::FIREL | act::FIRER) != 0 {
                throws.push((i, p.pos, attack_yaw, 1.0));
            }
            if bits_now & act::USE_MAGIC != 0 && p.potions > 0 {
                p.potions -= 1;
                self.sfx.push("S_POTION1".into());
            }
            if bits_now & (act::MAGIC_RELEASE | act::THROW_MAGIC_RELEASE) != 0 {
                let colour = p.potion_colours.pop().unwrap_or(0);
                rings.push((i, p.pos, colour));
            }
            if bits_now & act::BREATHE != 0 {
                breaths.push(i);
            }
            if bits_now & act::HAMMER != 0 {
                hammers.push(i);
            }
            // PWRA / 360 / PWRB / PWRC damage rows fire on their frames.
            let frame = p.act.anim.frame;
            let rows = &self.pdata[p.model].rows;
            let firing: Vec<i16> = match p.act.cur {
                pa::PWRA_CLOSE => vec![rows.turbo_a_close],
                pa::PWRA_LOW => vec![rows.turbo_a_low],
                pa::PWRA_MED => vec![rows.turbo_a_step],
                pa::A360 => vec![rows.turbo_a_360],
                pa::PWRA_THROW => vec![rows.turbo_a_throw],
                pa::PWRB => vec![rows.turbo_b],
                pa::PWRC => vec![rows.turbo_c1, rows.turbo_c2],
                pa::COMBO_ACTIVE1 if p.grab_partner.is_some() => vec![rows.combo1],
                pa::COMBO_ACTIVE3 if p.grab_partner.is_some() => vec![rows.combo2],
                _ => vec![],
            };
            let (t0, t1) = (p.act.prev_frame, frame);
            for first in firing {
                for row in self.pdata[p.model].chain(first) {
                    let sf = row.startframe as f32;
                    let ef = if row.endframe < 0 || row.flags & 0x80 != 0 {
                        sf
                    } else {
                        row.endframe as f32 + 1.0
                    };
                    if t1 >= sf && t0 < ef {
                        if row.amount != 0.0 {
                            p.power = (p.power - p.act.coll_score).max(0.0);
                            p.act.coll_score = 0.0;
                        }
                        damg.push((i, row.clone()));
                    }
                }
            }
            p.act.prev_frame = frame;
            // The meter fills while not in a turbo move; a charge drains it.
            if p.act.action < 11 {
                p.power = (p.power + TURBO_FILL * dt).min(100.0);
            }
            if p.act.cur == pa::SHOVE {
                p.power = (p.power - 20.0 * dt).max(0.0);
            }

            // PlayerMotion: move and turn with the scales the action set.
            let a = &p.act;
            let mut movement = joymag * a.stick_scale;
            let mut mdir = joyang;
            // The hero carrying the Dwarf runs where the Dwarf steers.
            if a.cur == pa::COMBO_DWF2 {
                if let Some(g) = p.grab_partner {
                    let gi = inputs.get(g).copied().unwrap_or_default();
                    let gw = fwd * gi.stick.y + right * gi.stick.x;
                    if gw.length() > 0.0 {
                        movement = gw.length().min(1.0);
                        mdir = gw.x.atan2(gw.y);
                    }
                }
            }
            if a.cur == pa::SHOVE {
                movement = 1.0;
                if joymag < 0.25 {
                    mdir = p.yaw;
                }
            }
            let lunge = joymag == 0.0
                && ((ct == 0 && matches!(a.cur, pa::PWRA_CLOSE | pa::PWRA_MED))
                    || (pa::STEP..=pa::STRAFE_ATKR2).contains(&a.cur));
            if lunge {
                movement = 0.5;
                mdir = p.yaw;
            }
            // Blocking and strafing don't turn; attacks face their target.
            let mut head = if movement > 0.0 { mdir } else { p.yaw };
            if matches!(ms, 2..=6 | 9..=12 | 0x11..=0x14) {
                head = p.yaw;
            }
            if (1..=10).contains(&a.action) && a.action != 7 && movement == 0.0 {
                head = attack_yaw;
            }
            let turn = 15.707_963 * dt * a.turn_scale;
            p.yaw += wrap_angle(head - p.yaw).clamp(-turn, turn);
            let speed = a.move_scale * p.derived.speed * movement;
            let grabbed = a.hud & 0x20 != 0;
            let moving = speed > 0.0 && !grabbed;
            if moving {
                let dir = Vec3::new(p.yaw.sin(), 0.0, p.yaw.cos());
                let target = p.pos + dir * speed * dt;
                // Don't walk off into the void: require ground within reach.
                let probe = self.collision.ground_height(target, p.pos.y + STEP_HEIGHT);
                if probe.is_some_and(|g| g > p.pos.y - 20.0) || !p.on_ground {
                    p.pos = target;
                }
            }
            // The knock velocity carries the hero (none in untouchable moves),
            // decaying x0.667 a 30 Hz frame.
            if p.act.action >= 11 {
                p.knock_vel = Vec3::ZERO;
            }
            if !grabbed {
                p.pos += p.knock_vel * dt;
                p.knock_vel *= 0.667f32.powf(dt * 30.0);
                p.pos =
                    self.collision
                        .resolve_walls(p.pos, PLAYER_RADIUS, PLAYER_HEIGHT, STEP_HEIGHT);
                p.pos = Self::resolve_solids(&self.entities, p.pos, PLAYER_RADIUS, 0);
                p.on_ground = Self::snap_to_ground(&self.collision, &mut p.pos, &mut p.vy, dt);
            }
            if p.pos.y < self.level.worlds.min_bounds[1] - 50.0 {
                p.pos = self.cam_focus + Vec3::Y * 2.0;
                p.vy = 0.0;
            }
            // The model shows the action's animation (falling while airborne).
            if !p.on_ground && p.vy < -15.0 && seqs.falling.is_some() {
                p.animator.play(seqs.falling.unwrap_or(0));
                p.animator.update(&model.skeleton, dt);
            } else {
                p.animator.seq = p.act.anim.seq;
                p.animator.frame = p.act.anim.frame;
            }
        }
        self.update_combos();
        // Players don't overlap each other.
        for a in 0..self.players.len() {
            for b in (a + 1)..self.players.len() {
                let d = Vec2::new(
                    self.players[b].pos.x - self.players[a].pos.x,
                    self.players[b].pos.z - self.players[a].pos.z,
                );
                let len = d.length();
                let min = PLAYER_RADIUS * 2.0;
                if len < min {
                    let n = if len > 1e-4 {
                        d / len
                    } else {
                        Vec2::new(1.0, 0.0)
                    };
                    let push = n * (min - len) * 0.5;
                    self.players[a].pos -= Vec3::new(push.x, 0.0, push.y);
                    self.players[b].pos += Vec3::new(push.x, 0.0, push.y);
                }
            }
        }
        // Keep the party together: nobody may wander too far from the group's centre.
        let alive: Vec<usize> = (0..self.players.len())
            .filter(|&i| self.players[i].state.active())
            .collect();
        if alive.len() > 1 {
            let c = alive.iter().map(|&i| self.players[i].pos).sum::<Vec3>() / alive.len() as f32;
            for &i in &alive {
                let p = &mut self.players[i];
                let d = Vec2::new(p.pos.x - c.x, p.pos.z - c.z);
                if d.length() > PARTY_RADIUS {
                    let lim = d.normalize() * PARTY_RADIUS;
                    p.pos.x = c.x + lim.x;
                    p.pos.z = c.z + lim.y;
                }
            }
        }
        if it_cleared {
            self.it_holder = None;
        }
        for (m, pi) in posts {
            self.post(m, Some(pi), &[]);
        }
        for (pi, mult, knock) in swings {
            let (pos, yaw) = (self.players[pi].pos, self.players[pi].yaw);
            self.player_attack(pi, pos, yaw, mult, knock);
        }
        for pi in breaths {
            use crate::powerups as pu;
            if self.players[pi].powerups.use_one(pu::SPECIAL, pu::S_BREATH)
                || self.players[pi].fx.special & 0x400 != 0
            {
                let (pos, yaw) = (self.players[pi].pos, self.players[pi].yaw);
                let front = pos + Vec3::new(yaw.sin(), 0.0, yaw.cos()) * 10.0;
                self.area_hit(pi, front, 20.0, 40.0 * ELEMENT_BONUS);
            }
        }
        for pi in hammers {
            if self.players[pi]
                .powerups
                .use_one(crate::powerups::WEAPON, crate::powerups::W_HAMMER)
            {
                self.sfx.push("S_HAMMER|S_EXPLODE1".into());
                let pos = self.players[pi].pos;
                self.area_hit(pi, pos, 35.0, 100.0);
            }
        }
        for (pi, row) in damg {
            self.fire_damage_row(pi, &row);
        }
        self.shield_contact(dt);
        // start_magic: the potion's ring spreads out from where it was used.
        for (pi, pos, colour) in rings {
            let power = self.players[pi].derived.magic_power;
            self.rings.push(Ring {
                owner: pi,
                pos,
                radius: power,
                damage: MAGIC_DAMAGE,
                colour,
                time: 0.0,
                hit: Vec::new(),
            });
        }
        for (pi, pos, yaw, size) in throws {
            use crate::powerups as pu;
            self.sfx.push(format!("S_{}THROW", self.players[pi].class));
            let p = &self.players[pi];
            let (speed, mut damage, model, w) = (
                p.derived.missile_speed,
                p.derived.missile_damage,
                p.proj_model,
                p.fx.weapon,
            );
            if w & pu::W_ELEMENT != 0 {
                damage *= ELEMENT_BONUS;
            }
            // A super shot doubles the damage, one use a throw; the power-A
            // throw is a double-size missile.
            if self.players[pi].powerups.use_one(pu::WEAPON, pu::W_SUPER) {
                damage *= 2.0;
            }
            damage *= size;
            // 3-way and 5-way shots fan out 15 degrees apart.
            let spread: &[f32] = if w & pu::W_5WAY != 0 {
                &[0.0, 15.0, -15.0, 30.0, -30.0]
            } else if w & pu::W_3WAY != 0 {
                &[0.0, 15.0, -15.0]
            } else {
                &[0.0]
            };
            for a in spread {
                let y = yaw + a.to_radians();
                let dir = Vec3::new(y.sin(), 0.0, y.cos());
                self.spawn(
                    Kind::Projectile {
                        owner: pi,
                        vel: dir * speed,
                        life: PROJECTILE_LIFE,
                        damage,
                    },
                    model,
                    pos + Vec3::Y * 2.5 + dir * 1.2,
                    y,
                );
            }
        }
        for (pos, n) in key_drops {
            let model = self.pickup_model(&PickupKind::Key(1));
            self.spawn(
                Kind::Pickup {
                    kind: PickupKind::Key(n),
                    radius: 2.0,
                },
                model,
                pos + Vec3::Y * 0.5,
                0.0,
            );
        }
    }

    /// The fire shield burns any monster touching the player (3 fire damage a
    /// game frame); the lightning shield strikes each for 20, once a second.
    fn shield_contact(&mut self, dt: f32) {
        use crate::powerups as pu;
        self.shield_cooldown.retain(|_, t| {
            *t -= dt;
            *t > 0.0
        });
        self.damg_cooldown.retain(|_, t| {
            *t -= dt;
            *t > 0.0
        });
        let mut hits = Vec::new();
        for (pi, p) in self.players.iter().enumerate() {
            let armor = p.fx.armor;
            if !p.state.active() || armor & (pu::A_FIRE | pu::A_LIGHTNING) == 0 {
                continue;
            }
            for (i, e) in self.entities.iter().enumerate() {
                if !e.alive || !matches!(e.kind, Kind::Enemy { .. }) {
                    continue;
                }
                let d = Vec2::new(e.pos.x - p.pos.x, e.pos.z - p.pos.z);
                if d.length() > PLAYER_RADIUS + 1.5 {
                    continue;
                }
                if armor & pu::A_FIRE != 0 {
                    // 3 a 30 Hz frame, in the fire element.
                    hits.push((
                        pi,
                        i,
                        3.0 * ELEMENT_BONUS * dt * 30.0,
                        d.normalize_or_zero(),
                    ));
                }
                if armor & pu::A_LIGHTNING != 0 && !self.shield_cooldown.contains_key(&(pi, e.id)) {
                    hits.push((pi, i, 20.0 * ELEMENT_BONUS, d.normalize_or_zero()));
                    self.shield_cooldown.insert((pi, e.id), 1.0);
                }
            }
        }
        for (pi, i, dmg, push) in hits {
            self.hit_entity(pi, i, dmg, push, false);
        }
    }

    /// How far an attack's reach is measured from: the target's radius, or None
    /// for things attacks don't aim at.
    fn target_radius(e: &Entity) -> Option<f32> {
        match &e.kind {
            Kind::Enemy { state, .. } if !matches!(state, EnemyState::Dying { .. }) => Some(1.0),
            Kind::Generator { .. } => Some(2.0),
            Kind::Container { chest: false, .. } => Some(1.0),
            Kind::Trigger { .. } => Some(0.5),
            Kind::Critter(c) if c.hittable() => Some(c.wad.types[c.parts[0].ty].rad),
            _ => None,
        }
    }

    /// Where an attack from `from` aims at entity `e`: the point and its
    /// radius (a critter's nearest colnode, else the entity itself).
    fn aim_point(e: &Entity, from: Vec3) -> Option<(Vec3, f32)> {
        let r = match &e.kind {
            Kind::Critter(c) => c.wad.types[c.parts[0].ty].rad,
            _ => Self::target_radius(e)?,
        };
        if let Kind::Critter(c) = &e.kind {
            let (_, node) = c.surface(from, e.pos, e.yaw);
            if let Some((pi, ci)) = node {
                if let Some((_, _, p, r)) = c
                    .colliders()
                    .into_iter()
                    .find(|(a, b, _, _)| *a == pi && *b == ci)
                {
                    return Some((Vec3::new(p.x, from.y, p.z), r));
                }
            }
            return Some((c.coll_pos(0, e.pos, e.yaw), r));
        }
        Some((e.pos, r))
    }

    /// A melee swing (PlayerMotion_DamageTarget): one target, the nearest in the
    /// forward cone within reach of its surface, takes the player's damage.
    /// PlayerMotion's combo block (pmotion.c:2938-3120): link the partner as
    /// COMBOACT1 starts (paying the turbo), hold or ride them by class, the
    /// Warrior's throw and the Dwarf's ride, then let go; heroes held follow
    /// the other's DUMMY node.
    fn update_combos(&mut self) {
        use crate::paction::p as pa;
        let n = self.players.len();
        let grab = |g: &mut Game, x: usize, onto: usize| {
            let p = &mut g.players[x];
            p.saved_pos = p.pos;
            p.act.hud |= 0x20;
            p.attached_to = Some(onto);
        };
        let release = |g: &mut Game, x: usize, restore: bool| {
            let p = &mut g.players[x];
            if restore {
                p.pos = p.saved_pos;
            }
            p.act.hud &= !0x20;
            p.attached_to = None;
        };
        let unlink = |g: &mut Game, i: usize, partner: usize| {
            g.players[partner].grab_partner = None;
            g.players[i].grab_partner = None;
        };
        for i in 0..n {
            let cur = self.players[i].act.cur;
            let combo_anim = matches!(cur, pa::COMBO_ACTIVE1 | pa::COMBO_ACTIVE2);
            self.players[i].combo_timer -= 1;
            if combo_anim {
                if let Some(t) = self.players[i].grab_pending.take() {
                    if t < n {
                        let p = &mut self.players[i];
                        p.grab_partner = Some(t);
                        p.power = (p.power - p.act.coll_score).max(0.0);
                        p.act.coll_score = 0.0;
                        self.players[t].act.hud |= 0x10;
                        self.players[t].grab_partner = Some(i);
                    }
                }
            } else {
                self.players[i].grab_pending = None;
            }
            let pojo = self.players[i].fx.special & 0x400 != 0;
            let Some(partner) = self.players[i].grab_partner else {
                continue;
            };
            // Only the hero who started the combo runs it.
            if self.players[partner].act.hud & 0x50 == 0 && self.players[i].act.hud & 0x50 != 0 {
                continue;
            }
            let kind = if pojo {
                99
            } else {
                (self.players[i].class_id % 8) as u8
            };
            let ph = self.players[partner].act.hud;
            match kind {
                // Wizard, Knight, Sorceress, Jester: hold the partner on the
                // DUMMY node (the Jester lets go again at once).
                2 | 5 | 6 | 7 => {
                    if combo_anim {
                        if ph & 0x20 == 0 {
                            grab(self, partner, i);
                            continue;
                        }
                        if kind != 7 {
                            continue;
                        }
                    }
                    if ph & 0x10 != 0 {
                        if ph & 0x20 != 0 {
                            release(self, partner, true);
                        }
                        self.players[partner].act.hud &= !0x10;
                        unlink(self, i, partner);
                    }
                }
                // Valkyrie, Archer: ride the partner, who turns to face them.
                1 | 3 => {
                    if combo_anim {
                        if self.players[i].act.hud & 0x20 == 0 {
                            let d = self.players[i].pos - self.players[partner].pos;
                            self.players[partner].yaw = d.x.atan2(d.z);
                            grab(self, i, partner);
                        }
                    } else if ph & 0x10 != 0 {
                        if self.players[i].act.hud & 0x20 != 0 {
                            release(self, i, true);
                        }
                        self.players[partner].act.hud &= !0x10;
                        unlink(self, i, partner);
                    }
                }
                // Warrior: lift the partner for 30 frames, then throw them; they
                // fly as COMBOWAR2 for 240 ticks.
                0 => {
                    let lifting = cur == pa::COMBO_ACTIVE1 && self.players[i].act.anim.frame < 30.0;
                    if lifting {
                        if ph & 0x20 == 0 {
                            grab(self, partner, i);
                        }
                    } else {
                        if ph & 0x10 != 0 {
                            if ph & 0x20 != 0 {
                                release(self, partner, false);
                            }
                            let h = &mut self.players[partner].act.hud;
                            *h = (*h & !0x10) | 0x40;
                            self.players[i].combo_timer = 240;
                        }
                        if self.players[partner].act.hud & 0x40 != 0
                            && self.players[i].combo_timer <= 0
                        {
                            self.players[partner].act.hud &= !0x40;
                            self.players[i].act.hud &= !0x80;
                            unlink(self, i, partner);
                        }
                    }
                }
                // Dwarf: climb on the partner, who then runs with him (steered
                // by the Dwarf) for 240 ticks.
                4 => {
                    if cur == pa::COMBO_ACTIVE1 {
                        if self.players[i].act.hud & 0x20 == 0 {
                            let d = self.players[i].pos - self.players[partner].pos;
                            self.players[partner].yaw = d.x.atan2(d.z);
                            grab(self, i, partner);
                            self.players[i].act.hud |= 0x80;
                        }
                        self.players[i].combo_timer = 240;
                    } else if ph & 0x10 != 0 {
                        let h = &mut self.players[partner].act.hud;
                        *h = (*h & !0x10) | 0x40;
                    } else if ph & 0x40 != 0 && self.players[i].combo_timer <= 0 {
                        if self.players[i].act.hud & 0x20 != 0 {
                            release(self, i, false);
                        }
                        self.players[partner].act.hud &= !0x40;
                        self.players[i].act.hud &= !0x80;
                        unlink(self, i, partner);
                    }
                }
                _ => {}
            }
        }
        // Held heroes ride the other's DUMMY node.
        for x in 0..n {
            let Some(y) = self.players[x].attached_to else {
                continue;
            };
            let (pos, yaw, model, seq, frame) = {
                let h = &self.players[y];
                (h.pos, h.yaw, h.model, h.animator.seq, h.animator.frame)
            };
            let node = self.models[model]
                .node_matrix(seq, frame, "DUMMY")
                .map_or(Vec3::Y * 3.0, |m| m.w_axis.truncate());
            let p = &mut self.players[x];
            p.pos = pos + glam::Quat::from_rotation_y(yaw) * node;
            p.yaw = yaw;
        }
        // The thrown Warrior's partner and the Dwarf's carrier knock down what
        // they run into (50 and 10).
        for pi in 0..n {
            let cur = self.players[pi].act.cur;
            let dmg = match cur {
                pa::COMBO_WAR2 => 50.0,
                pa::COMBO_DWF2 => 10.0,
                _ => continue,
            };
            let pos = self.players[pi].pos;
            let hits: Vec<(usize, Vec2)> = self
                .entities
                .iter()
                .enumerate()
                .filter(|(_, e)| {
                    e.alive
                        && matches!(e.kind, Kind::Enemy { .. } | Kind::Critter(_))
                        && !self.damg_cooldown.contains_key(&(pi, e.id))
                })
                .filter_map(|(i, e)| {
                    let d = Vec2::new(e.pos.x - pos.x, e.pos.z - pos.z);
                    (d.length() < PLAYER_RADIUS + 2.0 && (e.pos.y - pos.y).abs() < 4.0)
                        .then_some((i, d.normalize_or_zero()))
                })
                .collect();
            for (i, push) in hits {
                let id = self.entities[i].id;
                self.damg_cooldown.insert((pi, id), 0.5);
                self.hit_entity(pi, i, dmg, push, true);
            }
        }
    }

    /// A swing lands (pmotion.c:2621): the one target in the facing cone within
    /// 3.5, for the melee damage times `mult` (x2 slow / step, x3 power).
    fn player_attack(&mut self, pi: usize, pos: Vec3, yaw: f32, mult: f32, _knock: bool) {
        let dir = Vec2::new(yaw.sin(), yaw.cos());
        let target = self
            .entities
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                e.alive && (matches!(e.kind, Kind::Critter(_)) || (e.pos.y - pos.y).abs() < 5.0)
            })
            .filter_map(|(i, e)| {
                let (at, r) = Self::aim_point(e, pos + Vec3::Y * 2.5)?;
                let d = Vec2::new(at.x - pos.x, at.z - pos.z);
                let surf = d.length() - r;
                (surf < MELEE_REACH && (d.length() < 0.5 || d.normalize().dot(dir) >= AIM_CONE))
                    .then_some((i, surf))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i);
        let Some(t) = target else { return };
        if let Kind::Trigger { hit, .. } = &mut self.entities[t].kind {
            *hit = true;
            return;
        }
        let p = &self.players[pi];
        let mut dmg = p.derived.damage * mult;
        // Growth doubles melee; an amulet gives every hit its element.
        if p.fx.special & crate::powerups::S_GROWTH != 0 {
            dmg *= 2.0;
        }
        if p.fx.weapon & crate::powerups::W_ELEMENT != 0 {
            dmg *= ELEMENT_BONUS;
        }
        self.hit_entity(pi, t, dmg, dir, true);
    }

    /// Damage every enemy, generator and breakable within `radius`.
    fn area_hit(&mut self, pi: usize, pos: Vec3, radius: f32, dmg: f32) {
        let hits: Vec<(usize, Vec2)> = self
            .entities
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                e.alive
                    && matches!(
                        e.kind,
                        Kind::Enemy { .. }
                            | Kind::Generator { .. }
                            | Kind::Container { chest: false, .. }
                            | Kind::Critter(_)
                    )
            })
            .filter_map(|(i, e)| {
                let (at, r) = match &e.kind {
                    Kind::Critter(_) => Self::aim_point(e, pos)?,
                    _ => (e.pos, 0.0),
                };
                let d = Vec2::new(at.x - pos.x, at.z - pos.z);
                (d.length() - r < radius).then(|| (i, d.normalize_or_zero()))
            })
            .collect();
        for (i, push) in hits {
            self.hit_entity(pi, i, dmg, push, false);
        }
    }

    /// PlyrSfxDoDamage for one PDATA damage row of a power or turbo move:
    /// projectiles fly from the player, effect rows hurt what is within their
    /// radius (and arc, when given) around the player, each target at most
    /// once per the row's re-hit delay.
    fn fire_damage_row(&mut self, pi: usize, row: &gdl_formats::pdata::DamageRow) {
        let p = &self.players[pi];
        let (pos, yaw) = (p.pos, p.yaw);
        let mut dmg = if row.amount < 0.0 {
            -row.amount * p.derived.damage
        } else {
            row.amount
        };
        // The class colour (dmgtype's element) is worth x1.5 on neutral monsters.
        if row.dmgtype & 0xF != 0 {
            dmg *= ELEMENT_BONUS;
        }
        if dmg <= 0.0 {
            return;
        }
        let fwd = Vec3::new(yaw.sin(), 0.0, yaw.cos());
        let side = Vec3::new(fwd.z, 0.0, -fwd.x);
        match row.kind {
            0 | 5 => {}
            2 | 10 => {
                // A projectile (type 10: a burst either side, at +-angle).
                let angles: &[f32] = if row.kind == 10 && row.angle != 0.0 {
                    &[1.0, -1.0]
                } else {
                    &[0.0]
                };
                let speed = row.speed_max.max(row.speed_min).max(10.0);
                for k in angles {
                    let y = yaw + k * row.angle.to_radians();
                    let dir = Vec3::new(y.sin(), 0.0, y.cos());
                    let model = self.players[pi].proj_model;
                    self.spawn(
                        Kind::Projectile {
                            owner: pi,
                            vel: dir * speed,
                            life: PROJECTILE_LIFE,
                            damage: dmg,
                        },
                        model,
                        pos + Vec3::Y * 2.5 + dir * 1.2,
                        y,
                    );
                }
            }
            _ => {
                let o = row.offset;
                let at = pos + side * o[0] + Vec3::Y * o[1] + fwd * o[2];
                let delay = row.delay.max(1.0 / 60.0);
                let hits: Vec<(usize, Vec2)> = self
                    .entities
                    .iter()
                    .enumerate()
                    .filter(|(_, e)| {
                        e.alive
                            && matches!(
                                e.kind,
                                Kind::Enemy { .. }
                                    | Kind::Generator { .. }
                                    | Kind::Container { chest: false, .. }
                                    | Kind::Critter(_)
                            )
                            && !self.damg_cooldown.contains_key(&(pi, e.id))
                    })
                    .filter_map(|(i, e)| {
                        let (pt, r) = match &e.kind {
                            Kind::Critter(_) => Self::aim_point(e, at)?,
                            _ => (e.pos, 0.0),
                        };
                        let d = Vec2::new(pt.x - at.x, pt.z - at.z);
                        let dist = d.length() - r;
                        let n = d.normalize_or_zero();
                        let in_arc = row.arc == 0.0 || n.dot(Vec2::new(fwd.x, fwd.z)) >= row.arc;
                        (dist < row.radius && dist >= row.minrad && in_arc).then_some((i, n))
                    })
                    .collect();
                for (i, push) in hits {
                    let id = self.entities[i].id;
                    self.damg_cooldown.insert((pi, id), delay);
                    self.hit_entity(pi, i, dmg, push, false);
                }
            }
        }
    }

    /// Damage entity `t` on player `pi`'s behalf: sounds, experience (more for
    /// melee, which also charges the turbo meter) and anything it drops.
    pub(crate) fn hit_entity(&mut self, pi: usize, t: usize, dmg: f32, push: Vec2, melee: bool) {
        if matches!(self.entities[t].kind, Kind::Critter(_)) {
            self.hit_critter(pi, t, dmg, push, melee);
            return;
        }
        let shrunk = self
            .players
            .iter()
            .any(|p| p.fx.special & crate::powerups::S_SHRINK != 0);
        let is_enemy = matches!(self.entities[t].kind, Kind::Enemy { .. });
        let is_gen = matches!(self.entities[t].kind, Kind::Generator { .. });
        let mut dmg = dmg;
        // damage_enemy / generators: the player's level against the level's
        // plevel scales the hit (at least 1).
        if is_enemy || is_gen {
            let plevel = self
                .wave
                .and_then(|w| self.worlds.level(w))
                .map_or(0.0, |l| l.player_level);
            let lv = self.players[pi].level as f32;
            if plevel > 0.0 {
                if lv < plevel {
                    dmg *= 1.0 - 0.01 * (plevel - lv);
                } else if lv > plevel {
                    dmg *= 1.0 + 0.1 * (lv - plevel);
                }
            }
            dmg = dmg.max(1.0);
        }
        if shrunk && is_enemy {
            dmg *= 2.0;
        }
        let r = Self::apply_hit(&mut self.entities[t], dmg, push);
        // The monster staggers (HIT1) or dies (DEATH / HIT2).
        if r.hit
            && !MAGIC_HIT.with(|m| m.get())
            && matches!(
                self.entities[t].kind,
                Kind::Enemy {
                    etype: crate::enemies::DEATH,
                    ..
                }
            )
        {
            self.post(0, Some(pi), &[]);
        }
        if is_enemy && r.hit {
            let e = &mut self.entities[t];
            let seqs = e.model.and_then(|m| self.monster_seqs.get(&m)).cloned();
            if let (Some(seqs), Kind::Enemy { state, etype, .. }) = (seqs, &mut e.kind) {
                if r.killed {
                    let small = crate::enemies::is_small(*etype);
                    *state = EnemyState::Dying {
                        timer: if small { 0.25 } else { 0.7 },
                    };
                    if let Some(d) = seqs.death {
                        e.animator.restart(d);
                        e.animator.force_loop = false;
                    }
                } else if let Some(h) = seqs.hit {
                    e.animator.restart(h);
                    e.animator.force_loop = false;
                }
            }
        }
        // Generators drop a tier at 2/3 and 1/3 of their health: they then spawn
        // the lower tier, allow twice as many alive, and change model.
        if is_gen && !r.killed {
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
                }
            }
        }
        let e = &self.entities[t];
        if let Some(snd) = self.hit_sound(e, Self::is_destroyed(e)) {
            self.sfx.push(snd);
        }
        if r.killed {
            let st = &mut self.players[pi].stats;
            if r.generator {
                st.generators_destroyed += 1;
            } else {
                st.enemies_killed += 1;
            }
        }
        if r.hit {
            // PlayerDamagedEnemy / Item: exp by monster type, five times as much
            // for generators; melee has its own (larger) values.
            use crate::enemies::{exp_hit, exp_kill};
            let amount = match (r.generator, r.killed) {
                (true, false) => 5.0 * exp_hit(r.tier, false),
                (true, true) => 5.0 * exp_kill(r.tier, false),
                (false, false) => exp_hit(r.tier, melee),
                (false, true) => exp_kill(r.tier, melee),
            };
            self.add_exp(pi, amount, melee);
        }
        if let Some((pos, item)) = r.drop {
            self.spawn_content(pos, item);
        }
    }

    /// A player's hit on a critter (pmotion.c:3633 / sfx.c:3168 into
    /// CritterDamage): the part and colnode nearest where it landed take it.
    fn hit_critter(&mut self, pi: usize, t: usize, dmg: f32, push: Vec2, melee: bool) {
        let from = self.hit_from.take().unwrap_or_else(|| {
            self.players
                .get(pi)
                .map_or(Vec3::ZERO, |p| p.pos + Vec3::Y * 2.5)
        });
        let (pos, yaw) = (self.entities[t].pos, self.entities[t].yaw);
        let Kind::Critter(mut c) = std::mem::replace(&mut self.entities[t].kind, Kind::Prop) else {
            return;
        };
        // PlayerDamagedEnemy ignores the boss until it wakes.
        if !c.hittable() || (c.is_boss() && !self.boss.as_ref().is_some_and(|b| b.active)) {
            self.entities[t].kind = Kind::Critter(c);
            return;
        }
        let (_, node) = c.surface(from, pos, yaw);
        let (part, colnode) = node.unwrap_or((0, usize::MAX));
        let magic = MAGIC_HIT.with(|m| m.get());
        let mut flags = if magic { crate::critter::DMG_MAGIC } else { 0 };
        let mut dmg = dmg;
        let elem = self
            .players
            .get(pi)
            .map_or(0, |p| p.fx.weapon & crate::powerups::W_ELEMENT);
        if !magic && elem != 0 {
            // The hit already carries the neutral-target element bonus; let
            // ModifyDamage apply this critter's own affinity instead.
            flags |= elem;
            dmg /= ELEMENT_BONUS;
        }
        let dir = Vec3::new(push.x, 0.0, push.y);
        let node = (colnode != usize::MAX).then_some(colnode);
        let (done, killed) = self.critter_damage(&mut c, part, dmg, Some(pi), flags, dir, node);
        self.entities[t].kind = Kind::Critter(c);
        if done > 0.0 {
            self.sfx.push("S_WEAPONHITMETA".into());
            if melee {
                // Melee charges the turbo meter as it does on monsters.
                let p = &mut self.players[pi];
                p.power = (p.power + 0.025 * done).min(100.0);
            }
        }
        if killed {
            self.players[pi].stats.enemies_killed += 1;
        }
    }

    /// AddExp: scaled by the level's xpmul (less for players above its plevel);
    /// melee exp also charges the turbo meter. Each level gained adds 100 health.
    fn add_exp(&mut self, pi: usize, amount: f32, melee: bool) {
        let (xp_mul, plevel) = self
            .wave
            .and_then(|w| self.worlds.level(w))
            .map_or((1.0, 0.0), |l| (l.xp_mul.max(0.0), l.player_level));
        let p = &mut self.players[pi];
        let gained = crate::stats::scale_exp(amount, p.level, xp_mul, plevel);
        if melee {
            p.power = (p.power + 0.025 * gained as f32).min(100.0);
        }
        p.exp = p.exp.saturating_add(gained);
        let level = crate::stats::exp_to_level(p.exp);
        if level > p.level {
            p.health += 100.0 * (level - p.level) as f32;
            p.level = level;
            p.refresh_stats();
            self.post(0x22, Some(pi), &[level.to_string()]);
        }
    }

    /// damage_player: armour takes its share off each hit (not poison gas or
    /// magic); blocking halves it. The announcer speaks as health crosses 150 and
    /// 50, and the hero grunts in pain.
    pub fn damage_player(&mut self, pi: usize, dmg: f32, flags: u32) {
        let Some(p) = self.players.get_mut(pi) else {
            return;
        };
        if !p.state.active() || flags & DmgType::MAGIC != 0 {
            return;
        }
        // Nothing hurts before a boss wakes, or while the good wizard talks
        // (except being thrown).
        if let Some(b) = &self.boss {
            if flags & 0x8000 == 0 && (!b.active || b.wizard.as_ref().is_some_and(|w| w.state >= 4))
            {
                return;
            }
        }
        // Turbo moves, combos and the like (action >= 11), and both heroes of a
        // two-player combo, can't be hurt.
        if p.act.action >= 11 || p.act.hud & 0x10 != 0 || p.grab_partner.is_some() {
            return;
        }
        let mut dmg = dmg;
        let armor = p.fx.armor;
        use crate::powerups as pu;
        // ModifyDamage: invulnerable takes nothing, gold invulnerability turns a
        // hit into a 10% heal, a gas mask stops poison.
        if armor & pu::A_INVULN != 0 {
            if armor & pu::A_GOLD != 0 && dmg > 1.0 {
                p.health = (p.health + 0.1 * dmg).min(max_health(p.level));
            }
            return;
        }
        if armor & pu::A_GAS != 0 && flags & DmgType::POISON_GAS != 0 {
            return;
        }
        if flags & (DmgType::POISON_GAS | DmgType::NO_ARMOR) == 0 {
            dmg = (dmg - p.derived.armor).max(0.0);
        }
        // Levitation lifts the hero over low blows.
        let mut flags = flags;
        if flags & DmgType::LOW != 0 && p.fx.special & crate::powerups::S_LEVITATE != 0 {
            dmg = 0.0;
        }
        // damage_player: a block (DEFEND2) stops the hit; a charge halves it;
        // either way a heavy knock becomes a plain knockback.
        if dmg > 1.0 && p.act.hud & (crate::paction::hud::BLOCK | crate::paction::hud::CHARGE) != 0
        {
            if p.act.hud & crate::paction::hud::BLOCK != 0 {
                dmg = 0.0;
            } else {
                dmg *= 0.5;
            }
            if flags & 0x10160 != 0 {
                flags = (flags & !0x10160) | DmgType::KNOCKBACK;
            }
        }
        // For PlayerKnockback: the damage, its flags and the push away from
        // where it came from.
        p.hit_damage += dmg;
        p.hit_flags |= flags;
        if let Some(src) = self.damage_from.take() {
            let d = p.pos - src;
            p.hit_force += Vec3::new(d.x, 0.0, d.z).normalize_or_zero();
        }
        let old = (p.health + 0.5) as i32;
        p.health -= dmg;
        let new = (p.health + 0.5) as i32;
        let name = format!("VOICE:S_{}{}1", p.color, p.class);
        if old > 150 && new <= 150 {
            self.sfx.push(format!("{name}+S_BADLY|VOICE:S_BADLY"));
        } else if old > 50 && new <= 50 {
            let line = if self.frame % 2 == 0 {
                "S_LIFEFORCE"
            } else {
                "S_ABOUT"
            };
            self.sfx.push(format!("{name}+{line}|VOICE:{line}"));
        } else if p.hurt_flash < 0.3 && dmg > 0.0 {
            let n = 1 + (self.time * 13.0) as u32 % 4;
            self.sfx.push(format!("S_{}PAIN{n}|S_PLYRDMG", p.class));
        }
        if dmg > 0.0 {
            p.hurt_flash = 1.0;
        }
    }

    /// Advance the potion rings: each grows from a third of its radius to the
    /// whole of it, hitting each monster once as it passes, harder near the start.
    fn update_rings(&mut self, dt: f32) {
        let mut hits = Vec::new();
        for ring in &mut self.rings {
            ring.time += dt;
            let phase = 1.0 - ring.time / MAGIC_TIME;
            if phase <= 0.33 {
                continue;
            }
            let radius = ring.radius * (0.33 + (1.0 - phase));
            let dmg = ring.damage * 1.5 * (phase - 0.33) * ELEMENT_BONUS;
            for (i, e) in self.entities.iter().enumerate() {
                if !e.alive || ring.hit.contains(&e.id) {
                    continue;
                }
                if !matches!(
                    e.kind,
                    Kind::Enemy { .. } | Kind::Generator { .. } | Kind::Critter(_)
                ) {
                    continue;
                }
                let (at, r) = match &e.kind {
                    Kind::Critter(_) => match Self::aim_point(e, ring.pos) {
                        Some(a) => a,
                        None => continue,
                    },
                    _ => (e.pos, 0.0),
                };
                let d = Vec2::new(at.x - ring.pos.x, at.z - ring.pos.z);
                if d.length() - r < radius {
                    ring.hit.push(e.id);
                    hits.push((ring.owner, i, dmg, d.normalize_or_zero()));
                }
            }
        }
        self.rings.retain(|r| r.time < MAGIC_TIME);
        MAGIC_HIT.with(|m| m.set(true));
        for (pi, i, dmg, push) in hits {
            if self.entities.get(i).is_some_and(|e| e.alive) {
                self.hit_entity(pi, i, dmg, push, false);
            }
        }
        MAGIC_HIT.with(|m| m.set(false));
    }

    pub(crate) fn world_letter(&self) -> char {
        self.level
            .name
            .trim_start_matches("level")
            .chars()
            .next()
            .unwrap_or('A')
    }

    /// Sound for hitting (or destroying) entity `e`.
    fn hit_sound(&self, e: &Entity, destroyed: bool) -> Option<String> {
        let w = self.world_letter();
        Some(match &e.kind {
            Kind::Enemy { class, .. } if destroyed => {
                let n = self
                    .classes
                    .get(*class)
                    .map(|c| c.name.as_str())
                    .unwrap_or("");
                let p = match n {
                    "GRU" => "GRUNT",
                    "GHO" => "GHOST",
                    "SKE" => "SKEL",
                    "DEM" => "DEMON",
                    "KNI" => "KNIGHT",
                    "TRO" => "TROLL",
                    "SCO" => "SCORP",
                    "SNA" => "SNAK",
                    "SOR" => "SORC",
                    "MUM" => "MUMMY",
                    "WOL" => "WOLF",
                    "SPI" => "SPID",
                    "GAR" => "GRG",
                    other => other,
                };
                format!("S_{p}1DIECLOS|S_{p}DIECLOSE|S_{p}DIECLOS|S_{p}2DIECLOS|S_WEAPONHITMETA")
            }
            Kind::Enemy { .. } => "S_WEAPONHITMETA".into(),
            Kind::Generator { .. } if destroyed => format!("S_GEN{w}CHIT1|S_WEAPONHITSTON"),
            Kind::Generator { .. } => "S_WEAPONHITSTON".into(),
            Kind::Container { .. } if destroyed => "S_CHEST".into(),
            Kind::Container { .. } => "S_WEAPONHITWOOD".into(),
            _ => return None,
        })
    }

    fn is_destroyed(e: &Entity) -> bool {
        !e.alive
            || matches!(
                e.kind,
                Kind::Enemy {
                    state: EnemyState::Dying { .. },
                    ..
                }
            )
    }

    /// Damage one entity.
    fn apply_hit(e: &mut Entity, dmg: f32, push: Vec2) -> HitResult {
        let mut r = HitResult::default();
        match &mut e.kind {
            Kind::Enemy {
                hp, state, etype, ..
            } => {
                if matches!(state, EnemyState::Dying { .. }) {
                    return r;
                }
                // IT can't be hurt; Death only loses 1 a hit to anything but magic.
                if *etype == crate::enemies::IT {
                    return r;
                }
                let dmg = if *etype == crate::enemies::DEATH && !MAGIC_HIT.with(|m| m.get()) {
                    1.0
                } else if *etype == crate::enemies::DEATH {
                    999.0
                } else {
                    dmg
                };
                r.hit = true;
                r.tier = *etype;
                *hp -= dmg.max(1.0);
                if *hp <= 0.0 {
                    *state = EnemyState::Dying { timer: 0.5 };
                    r.killed = true;
                } else {
                    *state = EnemyState::Hurt { timer: 0.3 };
                }
                let _ = push;
            }
            Kind::Generator { hp, etype, .. } => {
                r.hit = true;
                r.generator = true;
                r.tier = *etype;
                *hp -= dmg;
                if *hp <= 0.0 {
                    e.alive = false;
                    r.killed = true;
                }
            }
            // Breakables lose the damage less their armour, at least 1 a hit.
            Kind::Container {
                hp,
                drop,
                chest: false,
                armor,
            } => {
                *hp -= (dmg - *armor).round().max(1.0);
                if *hp <= 0.0 {
                    e.alive = false;
                    r.drop = Some((e.pos, *drop));
                }
            }
            _ => {}
        }
        r
    }

    /// A model already loaded for this kind of pickup, if any.
    fn pickup_model(&self, kind: &PickupKind) -> Option<usize> {
        self.entities
            .iter()
            .find(|e| matches!(e.kind, Kind::Pickup { kind: k, .. } if std::mem::discriminant(&k) == std::mem::discriminant(kind)))
            .and_then(|e| e.model)
    }

    /// A chest or breakable's content: the placed item, or nothing.
    fn spawn_content(&mut self, pos: Vec3, item: Option<usize>) {
        let Some(info) = item
            .and_then(|i| self.level.worlds.item_infos.get(i))
            .cloned()
        else {
            return;
        };
        // Death waits in some chests.
        if info.item_type == ItemType::Enemy {
            if let Some(c) = self
                .classes
                .iter()
                .position(|c| c.etype == crate::enemies::DEATH)
            {
                let model = self.classes[c].model(1);
                let mut kind = self.new_enemy(c, 1, EnemyState::Chase);
                if let Kind::Enemy { recognized, .. } = &mut kind {
                    *recognized = true;
                }
                self.spawn(kind, model, pos, 0.0);
            }
            return;
        }
        if info.item_type != ItemType::Powerup {
            return;
        }
        let kind = pickup_kind(&info, &[0u8; 16]);
        let model = self
            .item_models
            .get(&info.name.to_ascii_uppercase())
            .copied()
            .flatten()
            .or_else(|| self.pickup_model(&kind));
        self.spawn(
            Kind::Pickup {
                kind,
                radius: info.radius.max(1.0) + 1.0,
            },
            model,
            pos + Vec3::Y * 0.5,
            0.0,
        );
    }

    fn update_entities(&mut self, dt: f32) {
        // Where each monster stood before this frame's move (do_enemy_move
        // reverts a move that runs into another monster or a hero).
        let before: HashMap<u64, Vec3> = self
            .entities
            .iter()
            .filter(|e| matches!(e.kind, Kind::Enemy { .. }))
            .map(|e| (e.id, e.pos))
            .collect();
        let player_pos: Vec<(Vec3, bool)> = self
            .players
            .iter()
            .map(|p| (p.pos, p.state.active()))
            .collect();
        use crate::powerups as pu;
        // Invisible players are never chased; stop time freezes every monster;
        // shrink halves what monsters deal (and doubles what they take).
        let hidden: Vec<bool> = self
            .players
            .iter()
            .map(|p| p.fx.special & pu::S_INVISIBLE != 0)
            .collect();
        let frozen = self
            .players
            .iter()
            .any(|p| p.fx.special & pu::S_STOPTIME != 0);
        let shrunk = self
            .players
            .iter()
            .any(|p| p.fx.special & pu::S_SHRINK != 0);
        let levitating: Vec<bool> = self
            .players
            .iter()
            .map(|p| p.fx.special & pu::S_LEVITATE != 0)
            .collect();
        let it = self
            .it_holder
            .filter(|&pi| player_pos.get(pi).is_some_and(|p| p.1));
        let mut barrier_touches = Vec::new();
        let nearest = |pos: Vec3| -> Option<(usize, f32)> {
            // Every monster goes after the player who is IT, at any range.
            if let Some(pi) = it {
                return Some((pi, (player_pos[pi].0 - pos).length().min(1.0)));
            }
            player_pos
                .iter()
                .enumerate()
                .filter(|(i, (_, alive))| *alive && !hidden[*i])
                .map(|(i, (pp, _))| (i, (*pp - pos).length()))
                .min_by(|a, b| a.1.total_cmp(&b.1))
        };
        let enemy_count = self
            .entities
            .iter()
            .filter(|e| e.alive && matches!(e.kind, Kind::Enemy { .. }))
            .count();
        // Generators spawn while on screen (or flagged always active); monsters
        // freeze when off screen and out of sight.
        let visible: Vec<bool> = self
            .entities
            .iter()
            .map(|e| self.on_screen(e.pos, 40.0))
            .collect();
        let class_of: Vec<(usize, usize)> = self
            .classes
            .iter()
            .enumerate()
            .map(|(i, c)| (c.etype, i))
            .collect();
        let sight = 30.0 * self.tuning.visrad;
        let boss_dying = self.boss.as_ref().is_some_and(|b| b.dying);
        let mut spawns: Vec<Spawn> = Vec::new();
        let mut shots: Vec<Shot> = Vec::new();
        let mut blasts: Vec<(Vec3, f32)> = Vec::new();
        let mut drains: Vec<(usize, f32)> = Vec::new();
        let mut death_done: Vec<usize> = Vec::new();
        let mut tagged: Option<usize> = None;
        let halo: Vec<bool> = self
            .players
            .iter()
            .map(|p| p.fx.armor & crate::powerups::A_ANTIDEATH != 0)
            .collect();
        let mut damage: Vec<(usize, f32, u32)> = Vec::new();
        let mut bursts: Vec<(usize, Vec3)> = Vec::new();
        let mut adds: Vec<(Vec3, Vec3)> = Vec::new();
        // Monster melee hits: (monster, player, damage).
        let mut melee: Vec<(usize, usize, f32)> = Vec::new();
        let mut pickups: Vec<(usize, PickupKind)> = Vec::new();
        let mut opened: Vec<(usize, usize)> = Vec::new();
        let mut key_used: Vec<usize> = Vec::new();
        let mut exits: Vec<(usize, Option<u32>)> = Vec::new();
        let mut secret: Option<u32> = None;
        let mut chest_used: Vec<usize> = Vec::new();
        let mut posts: Vec<(usize, usize)> = Vec::new();
        let mut chests_opened: Vec<(Vec3, Option<usize>)> = Vec::new();
        let mut proj_hits: Vec<(usize, usize, f32, Vec2)> = Vec::new();

        for ei in 0..self.entities.len() {
            if !self.entities[ei].alive {
                continue;
            }
            let pos = self.entities[ei].pos;
            let near = nearest(pos);
            let e = &mut self.entities[ei];
            match &mut e.kind {
                Kind::Generator {
                    class,
                    strength,
                    timer,
                    interval,
                    max,
                    spawned,
                    genratio,
                    always,
                    special,
                    ..
                } => {
                    let on = visible[ei];
                    // generate_enemy refuses everything once the boss is dying.
                    if (*always || on) && spawned.len() < *max && !boss_dying {
                        if *timer > 0.0 {
                            *timer -= dt * 60.0;
                        } else {
                            // With every monster slot taken only a generator on
                            // screen may spawn, by recycling one off screen.
                            let full = enemy_count + spawns.len() >= self.max_enemies;
                            if !full || on {
                                let class = match special {
                                    Some(list) => {
                                        let t = list[(self.special_counter as usize) & 3];
                                        self.special_counter += 1;
                                        class_of
                                            .iter()
                                            .find(|(ty, _)| *ty == t)
                                            .map_or(*class, |c| c.1)
                                    }
                                    None => *class,
                                };
                                spawns.push(Spawn {
                                    class,
                                    strength: *strength,
                                    pos,
                                    yaw: e.yaw,
                                    generator: e.id,
                                    recycle: full,
                                });
                                // 0.1 s x interval x (1..2), growing over 2 x max
                                // spawns and starting again.
                                *timer = 6.0 * *interval * (1.0 + *genratio);
                                *genratio += 1.0 / (2.0 * *max as f32);
                                if *genratio > 1.0 {
                                    *genratio = 0.0;
                                }
                            }
                        }
                    }
                }
                Kind::Enemy { .. } if frozen => {}
                Kind::Enemy {
                    strength,
                    hp,
                    state,
                    cooldown,
                    speed,
                    etype,
                    recognized,
                    attacks,
                    target,
                    reload,
                    drained,
                    blink,
                    ..
                } => {
                    let Some(model) = e.model else { continue };
                    let Some(seqs) = self.monster_seqs.get(&model).cloned() else {
                        continue;
                    };
                    let skel = &self.models[model].skeleton;
                    let ty = &crate::enemies::TYPES[(*etype).min(33)];
                    // fn_8004D958: the Warlock blinks while standing or walking,
                    // 1-2 s seen then 1-2 s unseen, fading 16/255 a tick; any
                    // other action fades it back in.
                    if *etype == 24 {
                        let calm = matches!(state, EnemyState::Idle | EnemyState::Chase);
                        let mut hide = false;
                        if calm {
                            if *blink == 0.0 {
                                *blink = self.rng.range(1.0, 2.0);
                            } else if *blink > 0.0 {
                                *blink -= dt;
                                if *blink <= 0.0 {
                                    *blink = -self.rng.range(1.0, 2.0);
                                }
                            } else {
                                *blink += dt;
                                if *blink >= 0.0 {
                                    *blink = self.rng.range(1.0, 2.0);
                                }
                            }
                            hide = *blink < 0.0;
                        }
                        let step = 16.0 / 255.0 * dt * 60.0;
                        e.fade = if hide {
                            (e.fade - step).max(0.0)
                        } else {
                            (e.fade + step).min(1.0)
                        };
                    }
                    // Recognize a player within sight; chase from then on.
                    if !*recognized && near.is_some_and(|(_, d)| d <= sight) {
                        *recognized = true;
                    }
                    // fn_8004D958: out of sight and off screen, nothing moves.
                    let in_sight = near.is_some_and(|(_, d)| d <= sight);
                    if !in_sight && !visible[ei] && !matches!(state, EnemyState::Dying { .. }) {
                        continue;
                    }
                    if *cooldown > 0.0 && !matches!(state, EnemyState::Dying { .. }) {
                        *cooldown -= dt;
                        e.animator.update(skel, dt);
                        continue;
                    }
                    // do_enemy_move: touching the player (radius + 0.5 + the
                    // player's 1.5) requests a swing.
                    let contact = near.and_then(|(pi, _)| {
                        let pp = player_pos[pi].0;
                        let d = Vec2::new(pp.x - e.pos.x, pp.z - e.pos.z).length();
                        (d < ty.radius + 0.5 + PLAYER_RADIUS && (pp.y - e.pos.y).abs() < ty.height)
                            .then_some(pi)
                    });
                    let play_loop = |a: &mut Animator, s: Option<usize>| {
                        if let Some(s) = s {
                            a.play(s);
                            a.force_loop = true;
                        }
                    };
                    match state {
                        EnemyState::Dying { timer } => {
                            // The death (or HIT2) animation, then gone: at once for
                            // small monsters, after the 0.7 s burn for the rest.
                            *timer -= dt;
                            if *timer <= 0.0 {
                                e.alive = false;
                            }
                            e.animator.update(skel, dt);
                            continue;
                        }
                        EnemyState::Asleep => {
                            e.animator.update(skel, dt);
                            continue;
                        }
                        EnemyState::Hurt { .. } => {
                            if e.animator.finished {
                                *state = EnemyState::Chase;
                            }
                        }
                        EnemyState::Spawning => {
                            if e.animator.finished || seqs.start.is_none() {
                                *state = EnemyState::Chase;
                            }
                        }
                        EnemyState::Idle => {
                            if *recognized {
                                *state = EnemyState::Chase;
                            } else {
                                play_loop(&mut e.animator, seqs.ready);
                            }
                        }
                        EnemyState::Chase => match near {
                            Some((pi, _)) => {
                                // turn_enemy_ang: 0.049 rad a tick toward the target.
                                let to = player_pos[pi].0 - e.pos;
                                let want = to.x.atan2(to.z);
                                let mut d = want - e.yaw;
                                while d > std::f32::consts::PI {
                                    d -= std::f32::consts::TAU;
                                }
                                while d < -std::f32::consts::PI {
                                    d += std::f32::consts::TAU;
                                }
                                let turn = 0.0491 * 60.0 * dt;
                                e.yaw += d.clamp(-turn, turn);
                                let dist = (player_pos[pi].0 - e.pos).length();
                                let on = visible[ei] && (player_pos[pi].0.y - e.pos.y).abs() < 10.0;
                                *reload -= dt;
                                use crate::enemies::{DEATH, IT};
                                if *etype == IT {
                                    // Tag: the touched player becomes IT.
                                    if let Some(pi) = contact {
                                        tagged = Some(pi);
                                        e.alive = false;
                                        continue;
                                    }
                                    let dir = Vec3::new(e.yaw.sin(), 0.0, e.yaw.cos());
                                    e.pos += dir * *speed * dt;
                                    play_loop(&mut e.animator, seqs.walk);
                                } else if *etype == DEATH {
                                    // Death drains whoever it touches, 3 ticks at a
                                    // time, until it has taken 100, then leaves.
                                    if let Some(pi) = contact.filter(|&pi| !halo[pi]) {
                                        *reload -= 0.0;
                                        if *reload <= 0.0 {
                                            *reload = 3.0 / 60.0;
                                            let f = self.tuning.damage;
                                            drains.push((pi, f));
                                            *drained += f;
                                            if *drained >= 100.0 {
                                                *state = EnemyState::Dying { timer: 1.0 };
                                                death_done.push(pi);
                                            }
                                        }
                                        if let Some((a, _)) = seqs.attack {
                                            if e.animator.seq != a {
                                                e.animator.restart(a);
                                            }
                                        }
                                    } else {
                                        let dir = Vec3::new(e.yaw.sin(), 0.0, e.yaw.cos());
                                        e.pos += dir * *speed * dt;
                                        play_loop(&mut e.animator, seqs.walk);
                                    }
                                } else if *strength == 6 {
                                    // The suicide grunt: waits a second, then charges
                                    // at 1.5x; on contact or after 4 s it explodes.
                                    if *reload > -1.0 {
                                        play_loop(&mut e.animator, seqs.ready);
                                    } else {
                                        let dir = Vec3::new(e.yaw.sin(), 0.0, e.yaw.cos());
                                        e.pos += dir * *speed * 1.5 * dt;
                                        play_loop(&mut e.animator, seqs.run.or(seqs.walk));
                                        if contact.is_some() || *reload < -5.0 {
                                            blasts.push((e.pos, 50.0 * self.tuning.damage));
                                            *state = EnemyState::Dying { timer: 0.1 };
                                        }
                                    }
                                } else if (*strength == 4 || *strength == 5) && seqs.throw.is_some()
                                {
                                    // Archers and bombers stand and shoot while the
                                    // player is in sight and on screen.
                                    play_loop(&mut e.animator, seqs.ready);
                                    if on && dist <= sight && *reload <= 0.0 && d.abs() < 0.785 {
                                        *state = EnemyState::Throw { fired: false };
                                        *target = Some((pi, false));
                                        e.animator.restart(seqs.throw.unwrap().0);
                                        e.animator.force_loop = false;
                                    }
                                } else if caster(*etype, *strength)
                                    && contact.is_none()
                                    && dist > 6.0
                                    && on
                                    && *reload <= 0.0
                                    && seqs.power.or(seqs.attack).is_some()
                                {
                                    // Level-3 casters roam and throw fireballs every
                                    // so often (90 x mrate ticks, randomised).
                                    let n = 90.0 * self.tuning.mrate;
                                    *reload =
                                        (20.0 + n + (ei as f32 * 7.3) % (n * 0.5 + 10.0)) / 60.0;
                                    *state = EnemyState::Throw { fired: false };
                                    *target = Some((pi, true));
                                    let a = seqs.power.or(seqs.attack).unwrap().0;
                                    e.animator.restart(a);
                                    e.animator.force_loop = false;
                                } else if let (Some(pi), Some((swing, _))) = (contact, seqs.attack)
                                {
                                    // Every 8th swing is the power attack.
                                    let power = *attacks & 7 == 7 && seqs.power.is_some();
                                    let a = if power { seqs.power.unwrap().0 } else { swing };
                                    *state = EnemyState::Attack { hit_done: false };
                                    *target = Some((pi, power));
                                    e.animator.restart(a);
                                    e.animator.force_loop = false;
                                } else {
                                    let dir = Vec3::new(e.yaw.sin(), 0.0, e.yaw.cos());
                                    e.pos += dir * *speed * dt;
                                    play_loop(&mut e.animator, seqs.walk);
                                }
                            }
                            None => play_loop(&mut e.animator, seqs.ready),
                        },
                        EnemyState::Attack { hit_done } => {
                            // The hit lands as the wind-up ends, wherever the
                            // player is now.
                            if !*hit_done && e.animator.finished {
                                *hit_done = true;
                                if let Some((pi, power)) = target.take() {
                                    let mut f = crate::enemies::fight(
                                        *etype,
                                        *hp,
                                        self.tuning.health,
                                        self.tuning.damage,
                                    );
                                    if power {
                                        f *= 1.5;
                                    }
                                    melee.push((ei, pi, f));
                                }
                                *attacks += 1;
                                let recover = if *attacks & 7 == 0 {
                                    seqs.power.and_then(|p| p.1)
                                } else {
                                    seqs.attack.and_then(|a| a.1)
                                };
                                match recover {
                                    Some(r) => {
                                        e.animator.restart(r);
                                        *state = EnemyState::Recover;
                                    }
                                    None => *state = EnemyState::Chase,
                                }
                            }
                        }
                        EnemyState::Recover => {
                            if e.animator.finished {
                                *state = EnemyState::Chase;
                            }
                        }
                        EnemyState::Throw { fired } => {
                            if !*fired && e.animator.finished {
                                *fired = true;
                                if let Some((pi, cast)) = target.take() {
                                    let slot = if cast {
                                        2
                                    } else if *strength == 5 {
                                        1
                                    } else {
                                        0
                                    };
                                    shots.push(Shot {
                                        from: e.pos,
                                        to: player_pos[pi].0 + Vec3::Y * 2.5,
                                        etype: *etype,
                                        slot,
                                        class: self
                                            .classes
                                            .iter()
                                            .position(|c| Some(model) == c.model(*strength))
                                            .unwrap_or(0),
                                    });
                                }
                                // The throw cooldown: 1 s x the missile rate, plus
                                // the finish.
                                if !matches!(target, Some((_, true))) && *strength != 6 {
                                    *reload = self.tuning.mrate;
                                }
                                match seqs.throw.and_then(|t| t.1) {
                                    Some(f) if *strength == 4 || *strength == 5 => {
                                        e.animator.restart(f)
                                    }
                                    _ => *state = EnemyState::Chase,
                                }
                            } else if *fired && e.animator.finished {
                                *state = EnemyState::Chase;
                            }
                        }
                    }
                    let _ = strength;
                    // Monsters collide with the world like players do.
                    e.pos = self.collision.resolve_walls(
                        e.pos,
                        ty.radius.min(1.5),
                        ty.height * 0.75,
                        STEP_HEIGHT,
                    );
                    if let Some(g) = self.collision.ground_height(e.pos, e.pos.y + STEP_HEIGHT) {
                        if e.pos.y - g < 3.0 {
                            e.pos.y = g;
                        } else {
                            let mut vy = 0.0;
                            Self::snap_to_ground(&self.collision, &mut e.pos, &mut vy, dt);
                        }
                    }
                    e.animator.update(skel, dt);
                }
                Kind::Pickup { kind, radius } => {
                    e.spin += dt * 2.0;
                    for (pi, (pp, alive)) in player_pos.iter().enumerate() {
                        if *alive && (*pp - e.pos).length() < *radius + 0.5 {
                            if shard_owned(self.shards, kind) {
                                posts.push((90, pi));
                                continue;
                            }
                            if can_take(&self.players[pi], kind) {
                                pickups.push((pi, *kind));
                                e.alive = false;
                                break;
                            }
                            if let Some(m) = refused_message(kind) {
                                posts.push((m, pi));
                            }
                        }
                    }
                }
                // Chests open with a key when touched (activetype 0x16 "locked").
                Kind::Container {
                    hp,
                    drop,
                    chest: true,
                    ..
                } if *hp > 0.0 => {
                    for (pi, (pp, alive)) in player_pos.iter().enumerate() {
                        let close =
                            *alive && Vec2::new(pp.x - e.pos.x, pp.z - e.pos.z).length() < 3.0;
                        if close && self.players[pi].keys == 0 {
                            posts.push((2, pi));
                        }
                        if close
                            && self.players[pi].keys > 0
                            && !key_used.contains(&pi)
                            && !chest_used.contains(&pi)
                        {
                            chest_used.push(pi);
                            *hp = 0.0;
                            chests_opened.push((e.pos, *drop));
                            if let Some(s) = e.model.and_then(|m| self.models[m].sequence("OPEN")) {
                                e.animator.restart(s);
                            }
                            break;
                        }
                    }
                }
                Kind::Door { radius, coll } => {
                    for (pi, (pp, alive)) in player_pos.iter().enumerate() {
                        if *alive && (*pp - e.pos).length() < *radius && self.players[pi].keys == 0
                        {
                            posts.push((1, pi));
                        }
                        if *alive
                            && (*pp - e.pos).length() < *radius
                            && self.players[pi].keys > 0
                            && !key_used.contains(&pi)
                        {
                            key_used.push(pi);
                            if let Some(c) = coll {
                                opened.push(*c);
                            }
                            e.alive = false;
                            break;
                        }
                    }
                }
                Kind::Trigger {
                    mover,
                    mode,
                    radius,
                    idle,
                    shoot,
                    pressed,
                    hit,
                    gate,
                    ..
                } => {
                    *idle -= dt;
                    // A crystal barrier only works once enough crystals are found.
                    let locked = gate.is_some_and(|c| match c {
                        101..=103 => self.garg_items[c - 101] >= 0,
                        c => !crystals_met(&self.crystals, c),
                    });
                    // Walking into a lever or onto a pad sets it off; wall levers sit in
                    // the wall, so allow for the player's own radius.
                    let contact = *hit
                        || (!*shoot
                            && player_pos.iter().any(|(pp, alive)| {
                                *alive
                                    && Vec2::new(pp.x - pos.x, pp.z - pos.z).length()
                                        < *radius + PLAYER_RADIUS + 0.6
                                    && (pp.y - pos.y).abs() < 4.0
                            }));
                    // A locked barrier says what it needs (TowerNeedCrystalsMsg).
                    if locked && contact {
                        barrier_touches.extend(*gate);
                    }
                    let touched = !locked && contact;
                    *hit = false;
                    let mut on = touched;
                    if let Some(m) = mover.and_then(|m| self.movers.get_mut(m)) {
                        m.trigger(*mode, touched, idle);
                        on |= m.active;
                    }
                    // Levers swing over (ONA) and back (OFFA), then rest in ON/OFF.
                    if let Some(model) = e.model.map(|m| &self.models[m]) {
                        let a = &mut e.animator;
                        a.force_loop = false;
                        let (go, rest) = if on { ("ONA", "ON") } else { ("OFFA", "OFF") };
                        if on != *pressed {
                            if let Some(s) = model.sequence(go) {
                                a.restart(s);
                            }
                        } else if a.finished || a.seq == 0 && on {
                            if let Some(s) = model.sequence(rest) {
                                a.play(s);
                            }
                        }
                    }
                    *pressed = on;
                }
                Kind::Secret { radius, wave, used } => {
                    if !*used && self.secret_timer.is_none() {
                        if let Some(w) = *wave {
                            if player_pos
                                .iter()
                                .any(|(pp, alive)| *alive && (*pp - e.pos).length() < *radius)
                            {
                                *used = true;
                                secret = Some(w);
                            }
                        }
                    }
                }
                Kind::Exit { radius, dest, open } => {
                    for (pi, (pp, alive)) in player_pos.iter().enumerate() {
                        if *open && *alive && (*pp - e.pos).length() < *radius {
                            exits.push((pi, *dest));
                        }
                    }
                }
                Kind::EnemyShot {
                    vel,
                    damage: dmg,
                    life,
                    splash,
                    gravity,
                    flags,
                    hit_model,
                } => {
                    *life -= dt;
                    vel.y -= *gravity * dt;
                    let from = e.pos;
                    let to = from + *vel * dt;
                    let hit = player_pos.iter().position(|(pp, alive)| {
                        *alive
                            && Vec2::new(pp.x - to.x, pp.z - to.z).length() < 1.5
                            && (to.y - pp.y) > -0.5
                            && (to.y - pp.y) < 5.0
                    });
                    let wall = self.collision.raycast(from, to).is_some();
                    let floor = self
                        .collision
                        .ground_height(to, to.y + 0.5)
                        .is_some_and(|g| to.y <= g);
                    if hit.is_some() || wall || floor || *life <= 0.0 {
                        e.alive = false;
                        let k = if shrunk { 0.5 } else { 1.0 };
                        if let Some(m) = hit_model {
                            bursts.push((*m, to));
                        }
                        if *flags & crate::critter::SHOT_ADDS != 0 {
                            adds.push((to, *vel));
                        }
                        if *splash > 0.0 {
                            for (pi, (pp, alive)) in player_pos.iter().enumerate() {
                                if *alive && (*pp - to).length() < *splash + PLAYER_RADIUS {
                                    damage.push((pi, *dmg * k, *flags));
                                }
                            }
                        } else if let Some(pi) = hit {
                            damage.push((pi, *dmg * k, *flags));
                        }
                    } else {
                        e.pos = to;
                    }
                }
                Kind::Projectile {
                    owner,
                    vel,
                    life,
                    damage: dmg,
                } => {
                    *life -= dt;
                    let from = e.pos;
                    let to = from + *vel * dt;
                    if *life <= 0.0 || self.collision.raycast(from, to).is_some() {
                        e.alive = false;
                    } else {
                        e.pos = to;
                        e.spin += dt * 20.0;
                        proj_hits.push((
                            ei,
                            *owner,
                            *dmg,
                            Vec2::new(vel.x, vel.z).normalize_or_zero(),
                        ));
                    }
                }
                Kind::Effect { life } => {
                    *life -= dt;
                    if *life <= 0.0 {
                        e.alive = false;
                    }
                }
                Kind::Hazard {
                    radius,
                    damage: dmg,
                } => {
                    for (pi, (pp, alive)) in player_pos.iter().enumerate() {
                        if *alive
                            && (*pp - e.pos).length() < *radius
                            && (self.time * 2.0).fract() < dt * 2.0
                        {
                            if !levitating[pi] {
                                damage.push((pi, *dmg, 0));
                            }
                        }
                    }
                }
                _ => {}
            }
            if !matches!(e.kind, Kind::Enemy { .. } | Kind::Critter(_)) {
                if let Some(m) = e.model {
                    e.animator.update(&self.models[m].skeleton, dt);
                }
            }
        }

        self.barrier_touches = barrier_touches;

        // Projectiles hit the first damageable thing they touch.
        for (pidx, owner, dmg, dir) in proj_hits {
            let ppos = self.entities[pidx].pos;
            let target = (0..self.entities.len()).find(|&t| {
                let e = &self.entities[t];
                if let Kind::Critter(c) = &e.kind {
                    if !e.alive || !c.hittable() {
                        return false;
                    }
                    let (surf, node) = c.surface(ppos, e.pos, e.yaw);
                    let t0 = &c.wad.types[c.parts[0].ty];
                    let cp = c.coll_pos(0, e.pos, e.yaw);
                    return surf < 0.4 && (node.is_some() || (ppos.y - cp.y).abs() < t0.hht + 2.0);
                }
                let r = match &e.kind {
                    Kind::Enemy { state, .. } if !matches!(state, EnemyState::Dying { .. }) => 1.4,
                    Kind::Generator { .. } => 2.2,
                    Kind::Container { chest: false, .. } => 1.6,
                    _ => return false,
                };
                e.alive
                    && Vec2::new(e.pos.x - ppos.x, e.pos.z - ppos.z).length() < r
                    && (e.pos.y + 2.0 - ppos.y).abs() < 3.5
            });
            if let Some(t) = target {
                if owner < self.players.len() {
                    self.hit_from = Some(ppos);
                    self.hit_entity(owner, t, dmg, dir, false);
                    self.hit_from = None;
                }
                self.entities[pidx].alive = false;
            } else if self.friendly_fire > 0 {
                // The Multiplayer option: a hero's shot stuns (1) or hurts (2)
                // another hero it hits (combat.c:1129).
                let hit = (0..self.players.len()).find(|&pi| {
                    let p = &self.players[pi];
                    pi != owner
                        && p.state.active()
                        && Vec2::new(p.pos.x - ppos.x, p.pos.z - ppos.z).length() < PLAYER_RADIUS
                        && (p.pos.y + 2.5 - ppos.y).abs() < 3.0
                });
                if let Some(pi) = hit {
                    self.entities[pidx].alive = false;
                    if self.friendly_fire == 1 {
                        let p = &mut self.players[pi];
                        p.hit_damage = p.hit_damage.max(2.0);
                        p.hit_flags |= DmgType::STUN;
                    } else {
                        self.damage_from = Some(ppos);
                        self.damage_player(pi, dmg, 0);
                        self.damage_from = None;
                    }
                }
            }
        }

        // Monsters can't walk through doors, barrels and generators either.
        let moved: Vec<(usize, Vec3)> = (0..self.entities.len())
            .filter(|&i| {
                self.entities[i].alive && matches!(self.entities[i].kind, Kind::Enemy { .. })
            })
            .map(|i| {
                (
                    i,
                    Self::resolve_solids(
                        &self.entities,
                        self.entities[i].pos,
                        0.9,
                        self.entities[i].id,
                    ),
                )
            })
            .collect();
        for (i, p) in moved {
            self.entities[i].pos = p;
        }

        // do_enemy_move: a monster's move is undone if it brings it into
        // another monster (their radii touching) or into a hero (radius + 0.5 +
        // the hero's 1.5); moving apart is always allowed.
        let enemy_idx: Vec<usize> = (0..self.entities.len())
            .filter(|&i| {
                self.entities[i].alive && matches!(self.entities[i].kind, Kind::Enemy { .. })
            })
            .collect();
        let size = |e: &Entity| match &e.kind {
            Kind::Enemy { etype, .. } => {
                let t = &crate::enemies::TYPES[(*etype).min(33)];
                (t.radius, t.height * 0.5)
            }
            _ => (1.5, 3.0),
        };
        for &i in &enemy_idx {
            let Some(&old) = before.get(&self.entities[i].id) else {
                continue;
            };
            let new = self.entities[i].pos;
            if new == old {
                continue;
            }
            let (ri, hi) = size(&self.entities[i]);
            let flat = |a: Vec3, b: Vec3| Vec2::new(a.x - b.x, a.z - b.z).length();
            let into = |other: Vec3, reach: f32, height: f32| {
                (new.y - other.y).abs() < height
                    && flat(new, other) < reach
                    && flat(new, other) < flat(old, other) - 1e-4
            };
            let mut blocked = enemy_idx.iter().any(|&j| {
                if j == i {
                    return false;
                }
                let (rj, hj) = size(&self.entities[j]);
                into(self.entities[j].pos, ri + rj, hi + hj)
            });
            blocked |= player_pos
                .iter()
                .any(|(pp, alive)| *alive && into(*pp, ri + 0.5 + PLAYER_RADIUS, hi + 2.5));
            if blocked {
                self.entities[i].pos = old;
            }
        }

        let w = self.world_letter();
        for m in &mut self.movers {
            let (started, stopped) = m.update(&mut self.collision, dt);
            if started {
                self.sfx.push(format!("S_ELVSTONE{w}|S_STONEGATE"));
            }
            if stopped {
                self.sfx.push(format!("S_ELVSTONESTP{w}|S_STONEGATESTOP"));
            }
        }

        for pi in chest_used {
            self.players[pi].keys -= 1;
        }
        for (pos, item) in chests_opened {
            self.sfx.push("S_CHEST".into());
            self.spawn_content(pos + Vec3::Y, item);
        }
        for pi in key_used {
            self.players[pi].keys -= 1;

            let w = self.world_letter();
            self.sfx.push(format!("S_GATE{w}1|S_SECRETWALL"));
        }
        for (start, count) in opened {
            for t in start..(start + count).min(self.collision.enabled.len()) {
                self.collision.enabled[t] = false;
            }
        }
        for (pi, dmg, flags) in damage {
            self.damage_player(pi, dmg, flags & !(0x1000 | crate::critter::SHOT_ADDS));
        }
        for (m, pos) in bursts {
            self.spawn_effect(m, pos, 0.0);
        }
        for (pos, vel) in adds {
            self.boss_generate_enemy(pos, vel);
        }
        for (ei, pi, dmg) in melee {
            let dmg = if shrunk { dmg * 0.5 } else { dmg };
            let special = self.players[pi].fx.special;
            if special & (pu::S_HANDOFDEATH | pu::S_VAMPIRE) != 0 {
                // Hand of Death turns the blow back on the monster; Health Vampire
                // does that too and heals the player by it.
                if let Some(e) = self.entities.get_mut(ei) {
                    Self::apply_hit(e, dmg, Vec2::ZERO);
                }
                if special & pu::S_VAMPIRE != 0 {
                    let p = &mut self.players[pi];
                    p.health = (p.health + dmg).min(max_health(p.level));
                }
            } else {
                // Short monsters' blows are low.
                let (from, low) = self.entities.get(ei).map_or((None, false), |e| {
                    let low = matches!(e.kind, Kind::Enemy { etype, .. }
                        if crate::enemies::TYPES[etype.min(33)].height * 0.5 <= 2.0);
                    (Some(e.pos), low)
                });
                self.damage_from = from;
                self.damage_player(pi, dmg, if low { DmgType::LOW } else { 0 });
                self.damage_from = None;
            }
        }
        let mut poisoned: Vec<(usize, f32)> = Vec::new();
        for (pi, kind) in &pickups {
            if let Some(m) = pickup_message(kind) {
                posts.push((m, *pi));
            }
        }
        for (pi, kind) in pickups {
            let p = &mut self.players[pi];
            self.sfx.push(match kind {
                PickupKind::Food(_) => format!("S_{}EAT|S_{}EATSFX", p.class, p.class),
                PickupKind::Key(_) => "S_PICKUPKEY".into(),
                PickupKind::Gold(_) => "S_PICKUPCRYST".into(),
                PickupKind::Potion(_) => "S_PICKUPMAGIC".into(),
                PickupKind::Crystal(_) => "S_PICKUPCRYST".into(),
                PickupKind::Scroll(_)
                | PickupKind::Powerup { .. }
                | PickupKind::Legend(_)
                | PickupKind::Gargoyle(_) => "S_PICKUPSPECIAL".into(),
                PickupKind::Shard(_) => "S_PICKUPRUNE|S_PICKUPSPECIAL".into(),
                PickupKind::Other(_) => "S_PICKUPSPECIAL".into(),
            });
            match kind {
                // heal_player: food never takes health past the maximum; spoiled
                // food is poison gas, which armour doesn't stop.
                PickupKind::Food(hp) if hp >= 0.0 => {
                    p.health = (p.health + hp).min(max_health(p.level));
                }
                PickupKind::Food(hp) => poisoned.push((pi, -hp)),
                PickupKind::Key(n) => p.keys = (p.keys + n).min(MAX_KEYS),
                PickupKind::Gold(v) => {
                    p.gold = (p.gold + v).min(99_999);
                    p.stats.gold_found += v;
                }
                // Potions are used last in, first out.
                PickupKind::Potion(c) => {
                    p.potions += 1;
                    p.potion_colours.push(c);
                }
                // towerAdvanceBossRecord: +1 for the party, up to the requirement;
                // reaching it opens that colour's barrier for good.
                PickupKind::Crystal(c) => {
                    let n = &mut self.crystals[c];
                    if *n >= 0 {
                        // TowerCheckMessages turns a met count into -1 with
                        // its "UnlockLevel" message.
                        *n = (*n + 1).min(CRYSTALS_NEEDED[c]);
                        self.crystal_shown = Some((c, 3.0));
                    }
                }
                // ControllerMessageBox: the game stops until the box is closed.
                PickupKind::Scroll(line) => {
                    if let Some(text) = self.scrolls.get(line) {
                        self.scroll_box = Some(ScrollBox::new(
                            Some(pi),
                            vec![text.clone()],
                            self.scroll_scale,
                        ));
                    }
                }
                // PlayerAddPowerup, for the class's powerup_time; it waits in
                // the mini-inventory until switched on.
                PickupKind::Powerup {
                    kind,
                    flags,
                    amount,
                    seconds,
                } => {
                    let t = seconds * crate::stats::powerup_time(p.class_id);
                    p.powerups.add(kind, flags, amount, t);
                }
                // PlayerGiveShard: every player gets the runestone.
                PickupKind::Shard(bit) => {
                    self.shards |= 1 << bit;
                    self.welcome = 300;
                }
                PickupKind::Legend(v) => {
                    self.legend_held |= 1 << v.min(15);
                }
                // towerAdvanceLevelRecord.
                PickupKind::Gargoyle(i) => {
                    if self.garg_items[i] >= 0 {
                        // -1 (gate open) comes with the tower's "UnlockSection".
                        self.garg_items[i] = (self.garg_items[i] + 1).min(GARG_NEEDED[i]);
                    }
                }
                PickupKind::Other(_) => {}
            }
        }
        for (m, pi) in posts {
            self.post(m, Some(pi), &[]);
        }
        for (pi, dmg) in poisoned {
            self.damage_player(pi, dmg, DmgType::POISON_GAS);
        }
        for sp in spawns {
            self.spawn_from_generator(sp);
        }
        // Monster missiles: fireballs fly straight at the player; arrows and
        // bombs are lobbed.
        for sh in shots {
            let (dmg, spd, splash, gravity) = missile(sh.etype, sh.slot);
            let speed = spd * self.tuning.mspeed;
            let up = if sh.slot == 2 { 1.5 } else { 2.5 };
            let from = sh.from + Vec3::Y * up;
            let to = sh.to - from;
            let flat = Vec3::new(to.x, 0.0, to.z);
            let dir = flat.normalize_or_zero();
            let vel = if gravity == 0.0 {
                to.normalize_or_zero() * speed
            } else {
                let t = (flat.length() / speed).max(0.1);
                dir * speed + Vec3::Y * (to.y / t + 0.5 * gravity * t)
            };
            let model = self.classes.get(sh.class).and_then(|c| c.missiles[sh.slot]);
            self.spawn(
                Kind::EnemyShot {
                    vel,
                    damage: dmg,
                    life: 4.0,
                    splash,
                    gravity,
                    flags: 0,
                    hit_model: None,
                },
                model,
                from + dir * 3.0,
                dir.x.atan2(dir.z),
            );
        }
        // The suicide grunt's blast hurts everyone near it.
        for (pos, dmg) in blasts {
            self.sfx.push("S_EXPLODE1|S_SUICIDEEXP".into());
            for pi in 0..self.players.len() {
                if (self.players[pi].pos - pos).length() < 8.0 {
                    self.damage_from = Some(pos);
                    self.damage_player(pi, dmg, 0);
                    self.damage_from = None;
                }
            }
        }
        // Death's drain goes round armour; once it has had 100 it leaves.
        for (pi, f) in drains {
            self.damage_player(pi, f, DmgType::NO_ARMOR);
        }
        for pi in death_done {
            self.post(0x83, Some(pi), &[]);
        }
        if let Some(pi) = tagged {
            self.it_holder = Some(pi);
            self.post(0x32, Some(pi), &[]);
        }
        // Forget dead spawns so generators can replace them.
        let alive_ids: std::collections::HashSet<u64> = self
            .entities
            .iter()
            .filter(|e| e.alive)
            .map(|e| e.id)
            .collect();
        for e in &mut self.entities {
            if let Kind::Generator { spawned, .. } = &mut e.kind {
                spawned.retain(|id| alive_ids.contains(id));
            }
        }
        self.entities.retain(|e| e.alive);
        if let Some(w) = secret {
            self.level_end = Some(LevelEnd::Secret(w));
            self.sfx.push("S_SECRETCHAR|S_PICKUPSPECIAL".into());
        }
        for (pi, dest) in exits {
            let p = &mut self.players[pi];
            if p.state.active() {
                p.state = PlayerState::Exiting { timer: 50.0 / 60.0 };
                p.exit_dest = dest;
                self.sfx.push("S_EXIT|S_PICKUPSPECIAL".into());
            }
        }
    }

    /// Camera eye and target in game coordinates.
    pub fn camera(&self) -> (Vec3, Vec3) {
        let (eye, at, _) = self.camera_view();
        (eye, at)
    }

    /// Camera eye, target and near-plane distance. The near plane is pushed out to just
    /// in front of the players so roofs and walls between them and the camera are cut away.
    pub fn camera_view(&self) -> (Vec3, Vec3, f32) {
        if let Some((eye, at)) = self.fixed_camera {
            return (eye, at, 0.5);
        }
        if let Some(c) = self.sumner.as_ref().and_then(|s| s.cam) {
            return (c.eye, c.eye + c.dir * 30.0, 0.5);
        }
        if let Some(v) = self.boss_camera_view() {
            return v;
        }
        self.camera_view_normal()
    }

    /// The level camera (do_camera): behind the party's centre.
    pub(crate) fn camera_view_normal(&self) -> (Vec3, Vec3, f32) {
        let spread = self
            .players
            .iter()
            .map(|p| (p.pos - self.cam_focus).length())
            .fold(0.0f32, f32::max);
        let dist = 36.0 + spread * 0.8;
        let pitch = self.cam_pitch;
        let at = self.cam_focus + Vec3::Y * 2.0;
        let (f, _) = self.camera_basis();
        let eye = at
            + Vec3::new(
                -f.x * pitch.cos() * dist,
                pitch.sin() * dist,
                -f.y * pitch.cos() * dist,
            );
        let near = (dist - CUTAWAY_DEPTH).max(1.0);
        (eye, at, near)
    }
}

/// pickup_kind for an item with default instance parameters.
pub(crate) fn pickup_kind_pub(info: &gdl_formats::worlds::ItemInfo) -> PickupKind {
    pickup_kind(info, &[0u8; 16])
}

/// Helpers the critter and boss code use.
impl Game {
    pub(crate) fn add_exp_pub(&mut self, pi: usize, amount: f32) {
        self.add_exp(pi, amount, false);
    }

    pub(crate) fn add_model_pub(&mut self, m: Model) -> usize {
        self.add_model(m)
    }

    /// The id the next spawned entity gets.
    pub(crate) fn peek_next_id(&self) -> u64 {
        self.next_id
    }

    pub(crate) fn item_model_pub(
        &mut self,
        fs: &GameFs,
        cache: &mut HashMap<String, Option<AssetFile>>,
        known: &mut HashMap<String, Option<usize>>,
        dirs: &[String],
        name: &str,
    ) -> Option<usize> {
        self.item_model(fs, cache, known, dirs, name)
    }

    pub(crate) fn merge_item_models(&mut self, known: HashMap<String, Option<usize>>) {
        for (k, v) in known {
            self.item_models.entry(k.to_ascii_uppercase()).or_insert(v);
        }
    }

    pub(crate) fn item_model_named(&self, name: &str) -> Option<usize> {
        self.item_models
            .get(&name.to_ascii_uppercase())
            .copied()
            .flatten()
    }

    pub(crate) fn camera_locator_pub(&self, p: Vec3) -> Option<(f32, f32)> {
        self.camera_locator(p)
    }

    /// A one-shot effect model at `pos`.
    pub(crate) fn spawn_effect(&mut self, model: usize, pos: Vec3, yaw: f32) {
        let life = self.models[model]
            .skeleton
            .sequences
            .first()
            .map_or(1.0, |s| {
                // A frame lasts rate/900 s (Animator::update).
                let rate = if s.frame_rate > 0 {
                    s.frame_rate as f32
                } else {
                    30.0
                };
                (s.frame_count.max(1) as f32 * rate / 900.0).clamp(0.2, 10.0)
            });
        if self
            .entities
            .iter()
            .filter(|e| matches!(e.kind, Kind::Effect { .. }))
            .count()
            < 48
        {
            self.spawn(Kind::Effect { life }, Some(model), pos, yaw);
        }
    }

    /// CritterDropItem for gargoyles: the level's GARG<kind> item.
    pub(crate) fn spawn_garg_item(&mut self, kind: &str, pos: Vec3) {
        let name = format!("GARG{}", kind.to_ascii_uppercase());
        let info = self
            .level
            .worlds
            .item_infos
            .iter()
            .find(|i| i.name.eq_ignore_ascii_case(&name))
            .cloned();
        let kind = match &info {
            Some(i) => pickup_kind(i, &[0u8; 16]),
            None => PickupKind::Gargoyle(match kind {
                "serp" => 0,
                "eagl" => 1,
                _ => 2,
            }),
        };
        let model = self.item_model_named(&name);
        self.spawn(
            Kind::Pickup { kind, radius: 2.0 },
            model,
            pos + Vec3::Y * 0.5,
            0.0,
        );
    }
}

impl Game {
    /// Simple bot: walk towards the nearest enemy, generator or pickup and attack when close.
    /// Used for the attract/demo mode and for headless testing.
    pub fn bot_input(&self, pi: usize) -> PlayerInput {
        let mut inp = self.bot_moves(pi);
        inp.inv = self.bot_inventory(pi);
        inp
    }

    /// The bot switches on whatever it picks up, a press every half second.
    fn bot_inventory(&self, pi: usize) -> crate::powerups::DPad {
        use crate::powerups::{DPad, FRESH};
        let Some(p) = self.players.get(pi) else {
            return DPad::default();
        };
        if self.frame % 30 != 0 {
            return DPad::default();
        }
        let fresh = p.powerups.slots[..10]
            .iter()
            .any(|s| s.time != 0.0 && s.state == FRESH);
        let sel_fresh = p.inv.sel >= 0 && p.powerups.slots[p.inv.sel as usize].state == FRESH;
        match p.inv.state {
            0 if fresh => DPad {
                up: true,
                ..Default::default()
            },
            1 if sel_fresh => DPad {
                up: true,
                ..Default::default()
            },
            1 if fresh => DPad {
                left: true,
                ..Default::default()
            },
            1 => DPad {
                down: true,
                ..Default::default()
            },
            _ => DPad::default(),
        }
    }

    fn bot_moves(&self, pi: usize) -> PlayerInput {
        let Some(p) = self.players.get(pi) else {
            return PlayerInput::default();
        };
        let target =
            self.entities
                .iter()
                .filter(|e| match &e.kind {
                    Kind::Enemy { state, .. } => !matches!(state, EnemyState::Dying { .. }),
                    // A sleeping boss is walked up to (and swung at, harmlessly).
                    Kind::Critter(c) => c.hittable() || (c.is_boss() && !c.deleted),
                    Kind::Generator { .. } | Kind::Pickup { .. } => true,
                    _ => false,
                })
                .filter_map(|e| {
                    let (at, r) = match &e.kind {
                        Kind::Critter(_) => Self::aim_point(e, p.pos + Vec3::Y * 2.5)?,
                        _ => (e.pos, 0.0),
                    };
                    ((at.y - p.pos.y).abs() < 4.0 || matches!(e.kind, Kind::Critter(_)))
                        .then_some((e, Vec3::new(at.x, p.pos.y, at.z), r))
                })
                .min_by(|a, b| {
                    ((a.1 - p.pos).length() - a.2).total_cmp(&((b.1 - p.pos).length() - b.2))
                });
        let Some((t, at, r)) = target else {
            return PlayerInput::default();
        };
        let to = at - p.pos;
        let world = Vec2::new(to.x, to.z);
        // The stick is camera-relative: turn the world direction into it.
        let (cf, cr) = self.camera_basis();
        let d = Vec2::new(world.dot(cr), world.dot(cf));
        let fighting = !matches!(t.kind, Kind::Pickup { .. });
        let close = d.length() - r < if fighting { 3.5 } else { 0.5 };
        if fighting && !close && d.length() < 22.0 && (self.time * 3.0).fract() < 0.5 {
            // In throwing range: face the target and fire.
            return PlayerInput {
                stick: d.normalize_or_zero() * 0.35,
                attack: true,
                attack_pressed: true,
                ..Default::default()
            };
        }
        PlayerInput {
            stick: if close && fighting {
                d.normalize_or_zero() * 0.06
            } else {
                d.normalize_or_zero()
            },
            attack: close && fighting,
            attack_pressed: close && fighting,
            ..Default::default()
        }
    }

    /// Totals for test output.
    pub fn stats(&self) -> String {
        let enemies = self
            .entities
            .iter()
            .filter(|e| matches!(e.kind, Kind::Enemy { .. }))
            .count();
        let gens = self
            .entities
            .iter()
            .filter(|e| matches!(e.kind, Kind::Generator { .. }))
            .count();
        let pick = self
            .entities
            .iter()
            .filter(|e| matches!(e.kind, Kind::Pickup { .. }))
            .count();
        let critters = self
            .entities
            .iter()
            .filter(|e| matches!(e.kind, Kind::Critter(_)))
            .count();
        format!("enemies {enemies} generators {gens} pickups {pick} critters {critters}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> Option<GameFs> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game-data");
        GameFs::open(&root).ok()
    }

    #[test]
    fn thrown_weapon_kills_grunt() {
        let Some(fs) = data() else {
            eprintln!("game data not found; skipping");
            return;
        };
        let mut g = Game::new(&fs, "levelA1", &[("WAR", "RED")]).unwrap();
        assert!(
            g.players[0].proj_model.is_some(),
            "warrior has a throw model"
        );
        g.entities.clear();
        // Let the player settle on the ground, facing +z.
        for _ in 0..30 {
            g.update(1.0 / 60.0, &[PlayerInput::default()]);
        }
        let p = g.players[0].pos;
        let class = g
            .classes
            .iter()
            .position(|c| c.name == "GRU")
            .unwrap_or_else(|| {
                let names: Vec<&str> = g.classes.iter().map(|c| c.name.as_str()).collect();
                panic!("grunts not loaded: {names:?}")
            });
        let model = g.classes[class].model(1);
        let mut kind = g.new_enemy(class, 1, EnemyState::Idle);
        if let Kind::Enemy {
            cooldown, speed, ..
        } = &mut kind
        {
            // Hold still for the test.
            *cooldown = 99.0;
            *speed = 0.0;
        }
        let id = g.spawn(kind, model, p + Vec3::new(0.0, 0.0, 12.0), 0.0);
        let fire = PlayerInput {
            stick: Vec2::new(0.0, 0.4),
            attack: true,
            attack_pressed: true,
            ..Default::default()
        };
        for _ in 0..120 {
            g.update(1.0 / 60.0, &[fire]);
            if !g.entities.iter().any(|e| e.id == id) {
                break;
            }
        }
        assert!(
            !g.entities.iter().any(|e| e.id == id),
            "grunt should be dead"
        );
        assert!(g.players[0].exp > 0, "a kill gives experience");
    }

    #[test]
    fn held_attack_chains_swings_on_a_grunt_in_reach() {
        let Some(fs) = data() else { return };
        let mut g = Game::new(&fs, "levelA1", &[("WAR", "RED")]).unwrap();
        g.entities.clear();
        for _ in 0..30 {
            g.update(1.0 / 60.0, &[PlayerInput::default()]);
        }
        let p = g.players[0].pos;
        let class = g.classes.iter().position(|c| c.name == "GRU").unwrap();
        let model = g.classes[class].model(1);
        let mut kind = g.new_enemy(class, 3, EnemyState::Idle);
        if let Kind::Enemy {
            cooldown, speed, ..
        } = &mut kind
        {
            *cooldown = 99.0;
            *speed = 0.0;
        }
        // In hit range in front (the hero faces +z).
        g.spawn(kind, model, p + Vec3::new(0.0, 0.0, 2.5), 0.0);
        let hold = PlayerInput {
            attack: true,
            attack_pressed: true,
            ..Default::default()
        };
        use crate::paction::p as pa;
        let mut seen = Vec::new();
        for _ in 0..90 {
            g.update(1.0 / 60.0, &[hold]);
            let cur = g.players[0].act.cur;
            if seen.last() != Some(&cur) {
                seen.push(cur);
            }
        }
        assert!(
            seen.contains(&pa::QUICK) && seen.contains(&pa::QUICK2) && seen.contains(&pa::QUICK3),
            "{seen:?}"
        );
        assert!(g.players[0].exp > 0, "the swings landed");
    }

    #[test]
    fn hits_stagger_and_knock_down() {
        let Some(fs) = data() else { return };
        let mut g = Game::new(&fs, "levelA1", &[("WAR", "RED")]).unwrap();
        g.entities.clear();
        g.pending_enemies.clear();
        let idle = [PlayerInput::default()];
        for _ in 0..30 {
            g.update(1.0 / 60.0, &idle);
        }
        use crate::paction::p as pa;
        let p0 = g.players[0].pos;
        g.damage_player(0, 10.0, 0);
        g.update(1.0 / 60.0, &idle);
        assert_eq!(g.players[0].act.cur, pa::HIT_REACT);
        for _ in 0..60 {
            g.update(1.0 / 60.0, &idle);
        }
        // Knocked down from behind (the hero faces +z): the forward fall, a push
        // forward, then up again.
        g.damage_from = Some(p0 - Vec3::Z * 3.0);
        g.damage_player(0, 10.0, DmgType::KNOCKDOWN);
        let mut seen = vec![];
        for _ in 0..180 {
            g.update(1.0 / 60.0, &idle);
            let c = g.players[0].act.cur;
            if seen.last() != Some(&c) {
                seen.push(c);
            }
        }
        assert!(
            seen.starts_with(&[pa::FALL_DOWN_FWD, pa::GET_UP_FWD]),
            "{seen:?}"
        );
        assert!(g.players[0].pos.z > p0.z + 0.5, "pushed forward");
    }

    #[test]
    fn wizard_combo_holds_the_partner() {
        let Some(fs) = data() else { return };
        let mut g = Game::new(&fs, "levelA1", &[("WIZ", "BLU"), ("WAR", "RED")]).unwrap();
        g.entities.clear();
        g.pending_enemies.clear();
        let idle = [PlayerInput::default(), PlayerInput::default()];
        for _ in 0..30 {
            g.update(1.0 / 60.0, &idle);
        }
        // The Wizard faces +z with the Warrior 3 ahead.
        let base = g.players[0].pos;
        g.players[0].yaw = 0.0;
        g.players[1].pos = base + Vec3::Z * 3.0;
        g.players[0].power = 60.0;
        let z = [
            PlayerInput {
                combo: true,
                ..Default::default()
            },
            PlayerInput::default(),
        ];
        use crate::paction::p as pa;
        let mut partner_anims = vec![];
        let mut linked = false;
        for _ in 0..240 {
            g.update(1.0 / 60.0, &z);
            linked |= g.players[1].grab_partner == Some(0);
            let c = g.players[1].act.cur;
            if partner_anims.last() != Some(&c) {
                partner_anims.push(c);
            }
        }
        assert!(linked, "the combo linked");
        assert!(partner_anims.contains(&pa::COMBO_WIZ), "{partner_anims:?}");
        assert!(g.players[0].power < 20.0, "turbo paid");
        assert!(
            g.players[1].grab_partner.is_none() && g.players[1].act.hud & 0x30 == 0,
            "let go"
        );
    }

    #[test]
    fn warrior_combo_throws_the_partner() {
        let Some(fs) = data() else { return };
        let mut g = Game::new(&fs, "levelA1", &[("WAR", "RED"), ("VAL", "BLU")]).unwrap();
        g.entities.clear();
        g.pending_enemies.clear();
        let idle = [PlayerInput::default(), PlayerInput::default()];
        for _ in 0..30 {
            g.update(1.0 / 60.0, &idle);
        }
        let base = g.players[0].pos;
        g.players[0].yaw = 0.0;
        g.players[1].pos = base + Vec3::Z * 3.0;
        g.players[0].power = 60.0;
        let z = [
            PlayerInput {
                combo: true,
                ..Default::default()
            },
            PlayerInput::default(),
        ];
        use crate::paction::p as pa;
        let mut seen = vec![];
        for _ in 0..600 {
            g.update(1.0 / 60.0, &z);
            let c = g.players[1].act.cur;
            if seen.last() != Some(&c) {
                seen.push(c);
            }
        }
        assert!(
            seen.contains(&pa::COMBO_WAR1) && seen.contains(&pa::COMBO_WAR2),
            "{seen:?}"
        );
        assert!(
            g.players[1].grab_partner.is_none(),
            "free again after the throw"
        );
    }

    #[test]
    fn music_areas_exist_and_have_streams() {
        let Some(fs) = data() else { return };
        let wd = gdl_formats::wdata::Worlds::load(|p| fs.read(p)).unwrap();
        let mut with_items = 0;
        for d in std::fs::read_dir(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../game-data/files/Gauntlet/LEVELS"),
        )
        .unwrap()
        {
            let name = d.unwrap().file_name().into_string().unwrap();
            // (The unused test levels name streams that aren't on the disc.)
            if name.starts_with("levelT") {
                continue;
            }
            let Some(w) = wd.wave_of_dir(&name) else {
                continue;
            };
            let (_, n) = wd.music_area(w, 0);
            for a in 0..n {
                for s in wd.music_area(w, a).0 {
                    assert!(
                        fs.read(&format!("STREAMS/{s}.ads")).is_ok(),
                        "{name} area {a}: {s}"
                    );
                }
            }
            let g = Game::new(&fs, &name, &[("WAR", "RED")]).unwrap();
            if !g.music_items.is_empty() {
                // (An area past the music's last is ignored, as in the game.)
                with_items += 1;
            }
        }
        assert!(with_items > 0, "some levels switch music by area");
    }

    #[test]
    fn ambient_sounds_resolve() {
        let Some(fs) = data() else { return };
        let mut names = Vec::new();
        for l in ["levelA1", "levelG1", "levelI1", "levelD1", "levelL1"] {
            let g = Game::new(&fs, l, &[("WAR", "RED")]).unwrap();
            names.extend(g.ambient_items.iter().map(|a| a.2.clone()));
        }
        names.sort();
        names.dedup();
        let found = crate::audio::check_sounds(&fs, &names);
        let missing: Vec<_> = found
            .iter()
            .filter(|(_, f)| f.is_none())
            .map(|(n, _)| n)
            .collect();
        println!("{} ambient sounds, missing {missing:?}", names.len());
        assert!(missing.len() * 4 < names.len(), "{missing:?}");
    }

    #[test]
    fn holding_attack_throws_at_original_rate() {
        let Some(fs) = data() else { return };
        let mut g = Game::new(&fs, "levelA1", &[("WAR", "RED")]).unwrap();
        g.entities.clear();
        let hold = PlayerInput {
            attack: true,
            ..Default::default()
        };
        let mut throws = 0;
        for _ in 0..300 {
            g.update(1.0 / 60.0, &[hold]);
            throws += g.sfx.drain(..).filter(|s| s.ends_with("THROW")).count();
        }
        // THROW1S cut at frame 2, THROW1, THROW1R, all rate 24 (37.5 frames a
        // second): ~0.37 s a throw, about 13-14 in 5 s.
        assert!((12..=16).contains(&throws), "{throws} throws in 5 s");
    }

    #[test]
    fn lever_lowers_trap_wall() {
        let Some(fs) = data() else { return };
        let mut g = Game::new(&fs, "levelA1", &[("WAR", "RED")]).unwrap();
        let wall = g
            .movers
            .iter()
            .position(|m| g.level.worlds.world_objects[m.root].name == "A1TRAPW2")
            .expect("door switch target registered");
        let lever = g
            .entities
            .iter()
            .find(|e| matches!(e.kind, Kind::Trigger { mover: Some(m), .. } if m == wall))
            .unwrap()
            .pos;
        let tri = g
            .collision
            .tris
            .iter()
            .position(|t| t.owner == g.movers[wall].root as i32)
            .expect("wall has collision");
        let y0 = g.collision.tris[tri].v[0].y;
        assert_eq!(g.movers[wall].offset, 0.0);
        // Run into the lever.
        g.players[0].pos = lever + Vec3::new(0.0, 0.5, 1.2);
        for _ in 0..10 {
            g.update(1.0 / 60.0, &[PlayerInput::default()]);
        }
        assert!(g.movers[wall].active, "lever pulled");
        g.players[0].pos = lever + Vec3::new(0.0, 0.5, 30.0);
        for _ in 0..120 {
            g.update(1.0 / 60.0, &[PlayerInput::default()]);
        }
        // door_switch latches: the wall sinks 4 units and stays down.
        assert!(
            (g.movers[wall].offset + 4.0).abs() < 0.01,
            "{}",
            g.movers[wall].offset
        );
        assert!((g.collision.tris[tri].v[0].y - (y0 - 4.0)).abs() < 0.01);
    }

    #[test]
    fn wall_lever_flips_when_pulled() {
        let Some(fs) = data() else { return };
        let g = Game::new(&fs, "levelA1", &[("WAR", "RED")]).unwrap();
        let m = g
            .entities
            .iter()
            .find_map(|e| match e.kind {
                Kind::Trigger { .. } if g.models[e.model?].sequence("ONA").is_some() => e.model,
                _ => None,
            })
            .expect("an animated lever");
        let model = &g.models[m];
        let shown = |seq: &str| {
            let (_, hidden) = model.evaluate(model.sequence(seq).unwrap(), 0.0);
            hidden
                .iter()
                .enumerate()
                .filter(|(_, h)| !**h)
                .map(|(i, _)| i)
                .collect::<Vec<_>>()
        };
        assert_ne!(shown("OFF"), shown("ON"), "lever looks different when on");
    }

    #[test]
    fn locked_gate_blocks_until_key() {
        let Some(fs) = data() else { return };
        let mut g = Game::new(&fs, "levelA1", &[("WAR", "RED")]).unwrap();
        g.entities.retain(|e| matches!(e.kind, Kind::Door { .. }));
        let walk = PlayerInput {
            stick: Vec2::new(0.0, 1.0),
            ..Default::default()
        };
        for _ in 0..400 {
            g.update(1.0 / 60.0, &[walk]);
        }
        assert!(
            g.players[0].pos.z < 27.0,
            "gate at z=27.9 should block, got {}",
            g.players[0].pos.z
        );
        g.players[0].keys = 1;
        for _ in 0..200 {
            g.update(1.0 / 60.0, &[walk]);
        }
        assert!(
            g.players[0].pos.z > 29.0,
            "gate should open with a key, got {}",
            g.players[0].pos.z
        );
        assert_eq!(g.players[0].keys, 0);
    }

    #[test]
    fn potion_clears_nearby_monsters() {
        let Some(fs) = data() else { return };
        let mut g = Game::new(&fs, "levelA1", &[("WAR", "RED")]).unwrap();
        let p = g.players[0].pos;
        let near = |e: &Entity| matches!(e.kind, Kind::Enemy { .. }) && (e.pos - p).length() < 25.0;
        let before = g.entities.iter().filter(|e| near(e)).count();
        g.players[0].potions = 1;
        let magic = PlayerInput {
            magic_pressed: true,
            ..Default::default()
        };
        g.update(1.0 / 60.0, &[magic]);
        for _ in 0..60 {
            g.update(1.0 / 60.0, &[PlayerInput::default()]);
        }
        let after = g.entities.iter().filter(|e| near(e)).count();
        assert_eq!(g.players[0].potions, 0);
        assert!(
            before == 0 || after < before,
            "before {before} after {after}"
        );
    }

    #[test]
    fn tower_portals_follow_progress() {
        let Some(fs) = data() else { return };
        let mut g = Game::new(&fs, "levelL1", &[("WAR", "RED")]).unwrap();
        assert_eq!(g.wave, Some(0xD00));
        let progress = crate::progress::Progress::default();
        g.close_portals(|w| progress.portal_open(w));
        let open: Vec<u32> = g
            .entities
            .iter()
            .filter_map(|e| match e.kind {
                Kind::Exit {
                    dest: Some(w),
                    open: true,
                    ..
                } => Some(w),
                _ => None,
            })
            .collect();
        // Every realm's first level is open; nothing else.
        assert!(!open.is_empty());
        assert!(
            open.iter()
                .all(|w| w & 0xFF == 0 && ![5, 6, 8].contains(&(w >> 8))),
            "{open:x?}"
        );
        assert!(!g.hidden.is_empty(), "closed portals hide their glow");
        let orange = g
            .entities
            .iter()
            .filter(|e| {
                matches!(
                    e.kind,
                    Kind::Pickup {
                        kind: PickupKind::Crystal(1),
                        ..
                    }
                )
            })
            .count();
        assert_eq!(orange, 15, "the tower holds 15 orange crystals");
        let barriers = g
            .entities
            .iter()
            .filter(|e| {
                matches!(
                    e.kind,
                    Kind::Trigger {
                        gate: Some(1..=8),
                        ..
                    }
                )
            })
            .count();
        assert_eq!(barriers, 8, "one crystal barrier per crystal realm");
    }

    #[test]
    fn powerup_items_match_the_spec() {
        let Some(fs) = data() else { return };
        // Find the power-up items across the levels and check their data.
        let mut seen = std::collections::HashMap::new();
        for dir in fs.list_dirs("LEVELS") {
            let Ok(level) = crate::level::Level::load(&fs, &dir) else {
                continue;
            };
            for info in &level.worlds.item_infos {
                if let PickupKind::Powerup {
                    kind,
                    flags,
                    amount,
                    seconds,
                } = pickup_kind(info, &[0; 16])
                {
                    seen.insert(
                        info.name.to_ascii_uppercase(),
                        (kind, flags, amount, seconds),
                    );
                }
            }
        }
        assert_eq!(seen.get("XBOWICON"), Some(&(5, 0x100000, 5.0, -1.0)));
        assert_eq!(seen.get("MULTIICON"), Some(&(5, 0x80000, 0.0, 45.0)));
        assert_eq!(seen.get("INVULICON"), Some(&(6, 0x10000, 0.0, 30.0)));
    }

    #[test]
    fn three_way_throws_three() {
        let Some(fs) = data() else { return };
        let mut g = Game::new(&fs, "levelA1", &[("ARC", "GRE")]).unwrap();
        // Nothing to aim at (throws turn toward the nearest target).
        g.entities.retain(|e| {
            !matches!(
                e.kind,
                Kind::Enemy { .. } | Kind::Generator { .. } | Kind::Container { .. }
            )
        });
        g.players[0]
            .powerups
            .add(crate::powerups::WEAPON, crate::powerups::W_3WAY, 0.0, 45.0);
        // Switched on from the mini-inventory.
        g.players[0].powerups.slots[0].state = crate::powerups::ON;
        let attack = PlayerInput {
            attack_pressed: true,
            attack: true,
            ..Default::default()
        };
        let mut most = 0;
        for _ in 0..40 {
            g.update(1.0 / 60.0, &[attack]);
            let n = g
                .entities
                .iter()
                .filter(|e| matches!(e.kind, Kind::Projectile { .. }))
                .count();
            most = most.max(n);
        }
        assert_eq!(most % 3, 0);
        assert!(most >= 3);
    }

    #[test]
    fn secret_icon_leads_to_its_secret_level() {
        let Some(fs) = data() else { return };
        let mut g = Game::new(&fs, "levelA6", &[("WAR", "RED")]).unwrap();
        let icon = g
            .entities
            .iter()
            .find(|e| matches!(e.kind, Kind::Secret { .. }))
            .map(|e| e.pos)
            .expect("A6 has a secret icon");
        g.players[0].pos = icon;
        g.update(1.0 / 60.0, &[PlayerInput::default()]);
        // S3 (Space) is the secret realm's third level.
        assert_eq!(g.level_end, Some(LevelEnd::Secret(0xC02)));
        let s = Game::new(&fs, "levelS3", &[("WAR", "RED")]).unwrap();
        assert_eq!(s.secret_timer.map(|t| t.0), Some(45.99));
    }

    #[test]
    fn realms_have_their_own_monsters() {
        let Some(fs) = data() else { return };
        let names = |level: &str| -> Vec<String> {
            let g = Game::new(&fs, level, &[("WAR", "RED")]).unwrap();
            g.classes.iter().map(|c| c.name.clone()).collect()
        };
        let b1 = names("levelB1");
        assert!(b1.contains(&"TRO".to_string()), "{b1:?}");
        assert!(!b1.contains(&"GRU".to_string()), "{b1:?}");
        // Big levels: the generators make the large monster.
        let b4 = names("levelB4");
        assert!(b4.contains(&"DEM".to_string()), "{b4:?}");
    }

    #[test]
    fn archers_shoot() {
        let Some(fs) = data() else { return };
        for level in ["levelA3", "levelB4", "levelC4", "levelJ3"] {
            let mut g = Game::new(&fs, level, &[("WAR", "RED")]).unwrap();
            // Placed monsters wait to come into view.
            let Some(archer) = g
                .pending_enemies
                .iter()
                .find(|e| {
                    matches!(
                        e.kind,
                        Kind::Enemy {
                            strength: 4 | 5,
                            ..
                        }
                    )
                })
                .map(|e| (e.pos, e.yaw))
            else {
                continue;
            };
            // Stand 12 units in front of it.
            let (pos, yaw) = archer;
            g.players[0].pos = pos + Vec3::new(yaw.sin(), 0.0, yaw.cos()) * 12.0 + Vec3::Y;
            let mut shots = 0;
            for _ in 0..600 {
                g.update(1.0 / 60.0, &[PlayerInput::default()]);
                shots += g
                    .entities
                    .iter()
                    .filter(|e| matches!(e.kind, Kind::EnemyShot { .. }))
                    .count();
            }
            assert!(shots > 0, "{level}: the archer never shot");
            return;
        }
        panic!("no archer found");
    }

    #[test]
    fn event_sounds_resolve() {
        let Some(fs) = data() else { return };
        let reqs: Vec<String> = [
            "VOICE:S_REDWAR1+S_NEEDSFOOD+S_BADLY",
            "VOICE:S_BLUVAL1+S_NEEDSFOOD+S_BADLY",
            "S_WARPAIN2|S_PLYRDMG",
            "S_WAREAT|S_WAREATSFX",
            "S_WARDIE1",
            "S_POTION1",
            "S_GATEA1|S_SECRETWALL",
            "S_CHEST",
            "S_PICKUPMAGIC",
            "S_OPTMENUSEL",
            "S_OPTMENUMOVVRT",
            "S_CHOOSESELSFX",
            "S_NO",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        for (req, found) in crate::audio::check_sounds(&fs, &reqs) {
            let (alt, len) = found.unwrap_or_else(|| panic!("{req} did not resolve"));
            assert!(len > 0.1, "{alt} is empty");
            eprintln!("{req} -> {alt} {len:.2}s");
        }
    }
}
