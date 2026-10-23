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

pub(crate) mod effect;
mod exits;
pub(crate) mod items;
mod magic;
mod particles;
#[cfg(test)]
mod rule_tests;
pub mod sounds;
pub(crate) mod unlock;
pub mod worn;

pub const PLAYER_CLASSES: [&str; 4] = ["WAR", "VAL", "WIZ", "ARC"];
pub const PLAYER_COLORS: [&str; 4] = ["RED", "BLU", "YEL", "GRE"];

const GRAVITY: f32 = 60.0;
pub(crate) const STEP_HEIGHT: f32 = 1.2;
/// PlayerCollideFloor's limit on a floor change in one move (lbl_80347BF8):
/// how high a stairs (or ramp) object's wall a player walks up.
pub(crate) const STAIR_RISE: f32 = 3.0;
/// PDATA width and height (col_radius 1.5, height 5).
pub(crate) const PLAYER_RADIUS: f32 = 1.5;
pub(crate) const PLAYER_HEIGHT: f32 = 5.0;
/// PlayerGetTarget: the farthest a target is looked for, and the melee reach
/// (2 plus the player's radius, from the target's surface).
const TARGET_RANGE: f32 = 30.0;
const MELEE_REACH: f32 = 2.0 + PLAYER_RADIUS;
/// The aim cone: dot >= 0.707 (0.5 on Easy).
const AIM_CONE: f32 = 0.707;
/// Turbo meter: fill per second, and the costs of the PWRB / PWRC moves.
const TURBO_FILL: f32 = 2.0;
/// player_max_health: 100 per level above 1 plus 500, at most 9999. A new
/// character starts full at level 1.
pub fn max_health(level: u32) -> f32 {
    (100.0 * (level.max(1) - 1) as f32 + 500.0).min(9999.0)
}
/// Thrown weapons fly until they hit something; this only bounds strays.
const PROJECTILE_LIFE: f32 = 3.0;

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

    /// player_get_from_save's fixed loadout for playable Sumner: level 99,
    /// full health, 5000 gold, 9 keys, 9 potions and no power-ups (his 999
    /// attributes come with `Player::sumner`).
    pub fn sumner() -> Carried {
        Carried {
            health: max_health(99),
            gold: 5000,
            keys: 9,
            potions: vec![0; 9],
            level: 99,
            exp: 0x54218,
            ..Default::default()
        }
    }

    /// activate_player: a hero coming out of player select into play (the
    /// tower, where no level-start camera runs) gets 5 x its class's
    /// power-up time of invisibility (PlayerAddPowerup(0, p, 9, 4, 5)).
    pub fn activate(&mut self, class: usize) {
        use crate::powerups as pu;
        let t = 5.0 * crate::stats::powerup_time(class);
        self.powerups.add(pu::SPECIAL, pu::S_INVISIBLE, 0.0, t);
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
        let class = if self.sumner {
            crate::stats::SUMNER
        } else {
            self.class_id
        };
        let a = crate::stats::atts(class, self.level, self.bonus);
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

/// DoAudioTallySFX's sounds by count (lbl_801234B8).
const TALLY_SFX: [i32; 11] = [
    0x2000b, 0x2000a, 0x20009, 0x20008, 0x20007, 0x20006, 0x20005, 0x20004, 0x20003, 0x20002,
    0x20001,
];

/// fn_80055AFC's state: ticks in the level (lbl_803447CC), the period clock
/// (F4), nags so far (E8), whether this period and the last were quiet (EC,
/// 80344780), and fighting this frame (lbl_803447E4).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct FollowNag {
    pub level_ticks: i32,
    pub timer: i32,
    pub count: i32,
    pub quiet: bool,
    pub was_quiet: bool,
    pub fighting: bool,
}

/// Death's exit (enemy.c:5313): alpha +4 a tick to 255, 64 ticks.
const DEATH_EXIT: f32 = 64.0 / 60.0;

/// ModifyDamage: an elemental hit on a target with no affinity does 1.5x. Every
/// potion, turbo move and amulet hit has an element; monsters here are neutral.
const ELEMENT_BONUS: f32 = 1.5;

/// A monster's missile to launch.
#[derive(Clone, Copy, Debug)]
/// A missile launch this tick (pmotion.c's missile block).
struct Throw {
    pi: usize,
    pos: Vec3,
    /// The player's facing, and the unit direction to its target (else along
    /// the attack yaw).
    yaw: f32,
    aim: Vec3,
    target: bool,
    bits: u32,
    /// Seconds since the throw began (sMusicFadeBase - combo_fade).
    since_swing: f32,
}

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
    /// The generator's ai and height (generate_enemy's spew and ang).
    ai: i32,
    height: f32,
}

thread_local! {
    /// Set while a potion's effect is dealing its damage (DMG_MAGIC): its
    /// element (1-4), else 0.
    static MAGIC_HIT: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
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
    /// Standing and waiting on an exit (DoExit).
    pub exit: exits::ExitWait,
    /// This frame's attempted move before collision (dpos), for doors.
    trial_move: Vec3,
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
    /// When a damage tile may hurt this hero again (fxhittime).
    fxhittime: f32,
    /// combo_fade: when the last swing or throw began.
    combo_fade: f32,
    /// floor_fx_time: when a hurting floor or wall may hurt again.
    floor_fx_time: f32,
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
    /// What the power-ups put on the hero (PlayerProcessPowerups).
    pub worn: worn::Worn,
    /// Playable Sumner (character 2 with hidden_code player_sumner_desc):
    /// the fixed 999 attributes.
    pub sumner: bool,
    /// got_count / got_timer for a secret level's coin counter
    /// (towerAwardWorldRunes), and the secret classes this hero has unlocked
    /// in the level (save.class_unlock bits).
    pub coins: u32,
    pub coin_timer: f32,
    pub class_unlock: u16,
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
    /// HIT2 and GETUP: E_HIT_REACT2 and what follows it.
    hit2: Option<usize>,
    getup: Option<usize>,
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
            hit2: s("HIT2"),
            getup: s("GETUP"),
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
        /// E_HIT_REACT2 (then GETUP): a heavy hit's knockdown, which a
        /// lighter hit doesn't interrupt (action.c:202).
        heavy: bool,
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

/// fn_8009DB24's argument: a selector (5 the level's hit sound, 6/7 the
/// 3/5-way burst) or, from a plyr_sfx hit record (SfxSetHit), its sound.
#[derive(Clone, Debug, PartialEq)]
pub enum WallSound {
    None,
    Sel(u8),
    Sound(String),
}

/// A plyr_sfx record resolved for one hero (fn_8008A678): whether its effect
/// was found (InitCustomEffect), the effect's model if it has geometry, and
/// its sound (AudioFindSound).
#[derive(Clone, Debug, Default)]
pub(crate) struct PlyrSfx {
    effect: bool,
    model: Option<usize>,
    sound: Option<String>,
    /// A particle record's texture (its fxdesc, MBOX_FindTexture_Sub).
    psys_tex: Option<usize>,
}

/// An effect parented to a hero's model (DoPlyrSfx's SfxSetParent): it keeps
/// its place in the hero's frame.
struct FxFollow {
    id: u64,
    player: usize,
    local: Vec3,
    yaw: f32,
}

/// One effect DoPlyrSfx started.
#[derive(Clone, Copy)]
struct PlyrFx {
    id: u64,
    pos: Vec3,
}

/// A damage row due this frame: (player, row, frame window, grab partner).
type DamgFire = (
    usize,
    gdl_formats::pdata::DamageRow,
    f32,
    f32,
    Option<usize>,
);

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
        /// Death's flag2: it left having drained its fill (or its victim),
        /// and says so as it goes (0x81 / 0x83); unused for the rest.
        drained: bool,
        /// The Warlock's blink: seconds left seen (> 0) or unseen (< 0).
        blink: f32,
        /// do_ai's state: its move_logic routine and what that keeps.
        brain: Box<crate::ai::Brain>,
        /// Enemy.algorithm: the AI routine (init_enemy, after fn_8004F87C):
        /// a placed monster's or generator's params[2] override, else
        /// sEnemyDefaultAlgorithm by type. Set by the item code; the movement
        /// AI reads it (items::algorithm).
        ai: i32,
        /// Enemy.dest: where a logic-12 generator sent it (place_logic12),
        /// for the AI to walk to; None otherwise.
        dest: Option<Vec3>,
        /// Enemy.pushed: the knockback velocity (units a second) a hit gave
        /// it (fn_8004DC2C), decaying x0.8 a frame (enemy.c:5391), and
        /// Enemy.pushang, the way it faces while pushed (back at the hit).
        pushed: Vec3,
        pushang: f32,
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
        /// gendata ai (params[2], else sEnemyDefaultAlgorithm): handed to
        /// generate_enemy; 12 sends spawns to either side (place_logic12).
        ai: i32,
        /// The item's height: how far in front monsters appear (+ their radius).
        height: f32,
        /// place_logic12's side (data[4]): 0 left, 1 right, then in place.
        side: u8,
        /// gendata's spawn bookkeeping for ai 13 / 14 (the zombie line's tail).
        chain: crate::ai::GenAi,
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
        /// The item's armour; -1 (walls, signs, chests) turns hits away.
        armor: f32,
        /// The item-info subtype (0x2B barrel, 0x2C explosive barrel or trap
        /// chest, 0x2D poison barrel; 0 for the boss's spouts).
        sub: i32,
    },
    Door {
        radius: f32,
        coll: Option<(usize, usize)>,
    },
    Exit {
        /// fn_8005F0F4 case 9: the hero's radius plus the item's (xz), and
        /// the item's height (plus the hero's) either way.
        radius: f32,
        height: f32,
        /// The wave this exit leads to, or None for the tower.
        dest: Option<u32>,
        open: bool,
        anim: Box<exits::ExitAnim>,
    },
    /// A damage tile (spikes, flames, force fields, saw blades, tentacles):
    /// it hurts only while its cycle is in sequence 2 or 4.
    Hazard {
        radius: f32,
        damage: f32,
        /// The tile's properties (element / knock flags), with STUN.
        flags: u32,
        cycle: Box<ItemCycle>,
    },
    /// A SECRET_ICON: touching it takes the party to a secret level, once.
    Secret {
        radius: f32,
        wave: Option<u32>,
        used: bool,
    },
    /// A hero's missile (StartMissile): it falls at `weight` and tumbles at
    /// `tumble` (rad/s; 0 points it along its flight), bursts on walls and
    /// floors, or rebounds off them with DMG_REFLECT.
    Projectile {
        owner: usize,
        vel: Vec3,
        life: f32,
        damage: f32,
        weight: f32,
        /// colrad: hits within it, walls within half of it.
        radius: f32,
        tumble: f32,
        reflect: bool,
        /// A super shot passes through what it hits, each once.
        pierced: Option<Vec<u64>>,
        /// Played when it bursts on a wall (fn_8009DB24).
        wall_sound: WallSound,
        /// minendtime: seconds left in which a wall stops it without
        /// bursting it.
        hold: f32,
        /// SfxSetMorph: at this much life left it changes to this model.
        morph: Option<(f32, Option<usize>)>,
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
        /// Set off last frame (daction 2: the lever shows pulled).
        pressed: bool,
        /// Struck by an attack this frame.
        hit: bool,
        /// A tower barrier: the crystal colour (1-8) or gargoyle gate (101-103).
        gate: Option<usize>,
        /// The trigger's id (instance params byte 6).
        id: u8,
        /// triggerdata.flags (triggers::tflags).
        flags: u16,
        /// The trigger camera it starts (camid; LinkTriggerToCam).
        cam: Option<(Vec3, Vec3, u8)>,
        /// A door_switch (subtype 0x17): it posts the traps hint (message 5).
        door_switch: bool,
    },
    Prop,
    /// items.c obstacles 0x28/0x31/0x34/0x35: a rock that falls (or sinks)
    /// once a hero comes within its radius (fn_8005D730), or once shot (0x34).
    Faller {
        sub: i32,
        active: bool,
        vel: Vec3,
        /// Tumble rate (rad/s): lbl_8011C904[index & 7] x the subtype's rate.
        tumble: f32,
        radius: f32,
        height: f32,
        tris: Option<(usize, usize)>,
    },
}

/// Sound ids by realm (sMusicTrackHi): lbl_80123AAC (fn_8009D91C, a rock
/// falls), lbl_80123AE4 (fn_8009D8CC, one sinks), lbl_801240DC
/// (AudioWorldHitPlyr).
const ROCKFALL_SFX: [i32; 16] = [
    -1, 0x250041, 0x260001, 0x270027, 0x280001, 0x370000, 0x38001E, 0x290034, 0x2A002C, 0x2B0049,
    -1, -1, -1, 0, -1, -1,
];
const ROCKSINK_SFX: [i32; 16] = [
    -1, -1, -1, -1, 0x280002, -1, -1, -1, -1, 0x2B004B, -1, -1, -1, 0, -1, 0x260002,
];
const WORLD_HIT_SFX: [i32; 16] = [
    -1, -1, -1, -1, -1, -1, -1, -1, -1, 0x2B0048, -1, -1, -1, 0, -1, 0x250034,
];

/// lbl_8011C904: the fallers' tumble multipliers.
const FALLER_TUMBLE: [f32; 8] = [-4.0, -3.0, -2.0, -1.0, 1.0, 2.0, 3.0, 4.0];

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
    /// Tumble about the model's x axis, and its size.
    pub pitch: f32,
    pub scale: f32,
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
    /// SetEnemyObj's SHADOW1L1..SHADOW3L1 (lbl_8011BFF8), by tier.
    pub(crate) shadows: [Option<usize>; 3],
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
    /// WorldObjGetAllFlags per world object (PlayerMotion_FloorFX).
    obj_flags: Vec<u32>,
    pub models: Vec<Model>,
    pub player_seqs: Vec<PlayerSeqs>,
    monster_seqs: HashMap<usize, MonsterSeqs>,
    /// A hero's or critter's shadow object (load_player_geo's SHADOWL1,
    /// CritterInitGeo's SHADOW<n>L1), by its body model.
    pub(crate) shadow_models: HashMap<usize, usize>,
    pub classes: Vec<MonsterClass>,
    pub players: Vec<Player>,
    pub entities: Vec<Entity>,
    /// World geometry moved by levers and pads.
    pub movers: Vec<Mover>,
    next_id: u64,
    /// The camera's attention point.
    pub cam_focus: Vec3,
    /// The yaw from the camera's attention point to its eye (pi: the eye is
    /// toward -z) and its downward pitch; the controls are relative to them.
    pub cam_yaw: f32,
    pub cam_pitch: f32,
    /// The gameplay camera (NEWCAM's standard camera, camera.rs).
    pub camera: crate::camera::GameCamera,
    /// ShakeCamera's state, applied by DoShake to the game and boss cameras.
    pub shake: crate::camera::Shake,
    /// The player start the party was put at (SetPlayerStart).
    pub start_index: u16,
    pub time: f32,
    /// 60 Hz ticks since the level started.
    pub frame: u32,
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
    /// A trigger's camera (TriggerCameraActivate from item update case 5).
    pub trigger_cam: Option<crate::triggers::TriggerCamera>,
    /// Each player's control settings (PlayerControl scheme / autoaim /
    /// autoattack), by player index.
    pub controls: [crate::options::PlayerPrefs; 4],
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
    /// The milestone network and the players' trails through it (do_ai).
    pub ai_paths: crate::ai::Paths,
    /// Round-robin counter for the Temple / Hell SPECIAL generators.
    special_counter: u32,
    /// Potion effects in flight or spreading (start_magic).
    pub(crate) magic: Vec<magic::MagicFx>,
    pub(crate) magic_models: magic::MagicModels,
    /// The power-up trees and objects heroes wear (worn.rs).
    pub(crate) worn_models: worn::WornModels,
    /// Effects riding a hero's node (the breath, the hammer's ring).
    worn_fx: Vec<worn::WornFx>,
    /// Item damage state: blasts, bags, carried items (items.rs).
    pub(crate) items: items::ItemsState,
    /// randpottype: the colour a colourless potion takes next.
    randpottype: u32,
    /// Lightning-shield strikes waiting out their second, by (player, monster).
    shield_cooldown: HashMap<(usize, u64), f32>,
    /// Damage-row re-hits waiting out their delay, by (player, target).
    damg_cooldown: HashMap<(usize, u64), f32>,
    /// 0 Easy, 1 Medium, 2 Hard.
    pub difficulty: usize,
    /// The Multiplayer option: 0 Normal (shots pass heroes), 1 Shots Stun,
    /// 2 Shots Hurt.
    pub friendly_fire: usize,
    /// The party's progress out of the level (lbl_803447D0 / lbl_803447B4).
    pub(crate) exit_state: exits::ExitState,
    /// Each player model's action slots (InitActions) and class PDATA.
    action_seqs: Vec<crate::paction::ActionSeqs>,
    pdata: Vec<gdl_formats::pdata::PlayerData>,
    /// Each player model's plyr_sfx records, resolved.
    psfx: Vec<Vec<PlyrSfx>>,
    fx_follow: Vec<FxFollow>,
    /// Item models loaded for the level, by item name.
    item_models: HashMap<String, Option<usize>>,
    /// A secret level's countdown: (total, seconds left).
    pub secret_timer: Option<(f32, f32)>,
    /// sVisibleSumCoinCount: the gold pieces placed in the level.
    pub visible_coins: u32,
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
    /// World-object animations, rotators and the floors they carry (worldanim.rs).
    pub world_anim: crate::worldanim::WorldAnims,
    /// Transporter pads and each hero's trip (transport.rs).
    pub transport: crate::transport::Transport,
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
    /// The debug menu's god mode (not in the original): heroes take no damage.
    pub debug_god: bool,
    /// Event-sound timers, the got-it ticker, rumble, X-ray and compass.
    pub snd: sounds::SoundState,
    /// Items flying out of a dying boss.
    pub spews: Vec<crate::boss::Spew>,
    /// Critter ground waves (stomps, eruptions).
    pub crit_waves: Vec<crate::critter::Wave>,
    /// player.fxhittime: a critter's melee and cones skip a player until then.
    pub crit_hit_until: [f32; 4],
    /// CritterPlayerNTargets: critter parts aiming at each player this frame.
    pub crit_ntargets: [f32; 4],
    /// Parsed CRITTER WADs by file name.
    pub critter_wads: HashMap<String, std::sync::Arc<gdl_formats::critter::CritterWad>>,
    /// The good wizard's model (atree WIZARD in the level's item file).
    pub wizard_model: Option<usize>,
    /// Where the hit being dealt landed (projectiles), for critter colnodes.
    pub(crate) hit_from: Option<Vec3>,
    /// Player.hit_streak: generator monsters each hero has killed since
    /// last destroying a generator (PlayerDamagedEnemy / PlayerDamagedItem).
    pub(crate) gen_streak: [u32; 4],
    /// fn_80055AFC's "Follow" nag.
    pub(crate) nag: FollowNag,
    /// The effect table and the effects it runs (effect.rs).
    pub(crate) effects: effect::Effects,
    /// The particle systems and missile streaks (psys.rs, particles.rs).
    pub particles: crate::psys::Particles,
    /// WEAPONS', POWERUPS' and STATIC's textures, for MBOX_FindTexture.
    psys_banks: Vec<(Objects, Vec<u8>)>,
}

/// items.c's animated-item cycle (an item with an atree and active & 1): it
/// steps through its sequences, holding each for active_on (sequences after
/// the first) or active_off (the first; x trap_rate for tiles) x 2 ticks —
/// random in [n/2, 1.5n) when negative — or the sequence's own length.
#[derive(Clone, Debug, PartialEq)]
pub struct ItemCycle {
    pub anim: crate::paction::AnimInfo,
    slots: Vec<crate::paction::SlotSeq>,
    /// Sequence shown (action) and wanted (daction).
    pub action: usize,
    daction: usize,
    /// Ticks left in this part of the cycle.
    activetime: i32,
    /// iteminfo activetype (bit 1 cycling, 2 stop at the end, 4 advance, 0x40
    /// update off screen, 0x400 no hold times).
    active: i16,
    on: i16,
    off: i16,
    /// trap_rate applies to the off time (damage tiles).
    rate: f32,
}

impl ItemCycle {
    fn new(model: &Model, active: i16, on: i16, off: i16, rate: f32, rng: &mut Rng) -> ItemCycle {
        let slots = model
            .skeleton
            .sequences
            .iter()
            .enumerate()
            .map(|(i, s)| crate::paction::SlotSeq {
                seq: i,
                frames: s.frame_count as u16,
                rate: s.frame_rate,
            })
            .collect();
        let mut c = ItemCycle {
            anim: Default::default(),
            slots,
            action: 0,
            daction: 0,
            activetime: 0,
            active,
            on,
            off,
            rate,
        };
        // SetItem case 8: the first off time, random when negative.
        let mut t = off as i32 * 2;
        if t < 0 {
            t = (-t >> 1) + (rng.next_u32() % (-t) as u32) as i32;
        }
        c.activetime = (t as f32 * rate) as i32;
        c
    }

    /// One tick; `visible` is whether it is on screen.
    fn update(&mut self, dt: f32, visible: bool, rng: &mut Rng) {
        if self.slots.is_empty() {
            return;
        }
        let a = self.active;
        if a & 0x40 == 0 && !visible && !(a & 4 != 0 && self.action > 0) {
            return;
        }
        if self.activetime > 0 {
            self.activetime -= 1;
        }
        if a & 1 == 0 {
            return;
        }
        if self.activetime <= 0 && a & 4 != 0 {
            self.daction += 1;
            if self.daction >= self.slots.len() {
                if a & 2 != 0 {
                    self.daction -= 1;
                } else {
                    self.daction = 0;
                }
            }
        }
        let mode = if self.action == self.daction { 0 } else { 2 };
        let s = self.slots[self.daction];
        if self.anim.animate_slot(dt, s, mode) & 3 == 0 {
            return;
        }
        if a & 0x400 == 0 {
            let t = if self.daction != 0 {
                self.on as i32 * 2
            } else {
                (self.off as f32 * 2.0 * self.rate) as i32
            };
            let mut w = self.anim.length_ticks();
            let mut t = t;
            if self.slots.len() > 4 && t < 0 {
                t = 0;
                w = 0;
            }
            self.activetime = if t > 0 {
                t
            } else if t < 0 {
                (-t >> 1) + (rng.next_u32() % (-t) as u32) as i32
            } else {
                w
            };
        }
        self.action = self.daction;
    }
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
    /// Death waiting as its statue (enemydata.flags bit 1 clear).
    statue: Option<crate::triggers::Statue>,
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

    /// gTriggerCameraState != 0: a trigger camera (Sumner's or a trigger
    /// item's) runs.
    pub fn trigger_camera_on(&self) -> bool {
        self.trigger_cam.is_some() || self.sumner.as_ref().is_some_and(|s| s.cam.is_some())
    }

    /// The trigger camera's view while it shows (eye, direction).
    pub fn trigger_camera_view(&self) -> Option<(Vec3, Vec3)> {
        if let Some(c) = self.sumner.as_ref().and_then(|s| s.cam) {
            return Some((c.eye, c.dir));
        }
        self.trigger_cam
            .filter(|c| c.showing())
            .map(|c| (c.eye, c.dir))
    }

    /// TriggerCamUpdate for a trigger item's camera: when it ends the
    /// controls come back (EnablePlayerControls) and the game camera goes on.
    fn update_trigger_camera(&mut self, dt: f32) {
        let Some(mut c) = self.trigger_cam else {
            return;
        };
        let ticks = (dt * 60.0).round().max(1.0) as i32;
        let moving = c
            .target
            .and_then(|m| self.movers.get(m))
            .is_some_and(|m| m.moving);
        self.trigger_cam = c.tick(ticks, moving).then_some(c);
    }

    /// msgPost about `player` (or the party).
    pub fn post(&mut self, index: usize, player: Option<usize>, args: &[String]) -> bool {
        // msgPost is refused while a trigger camera runs.
        if self.trigger_camera_on() {
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
    /// stops being an exit, its model becomes EXIT_OFF (the dark centre) and
    /// its "_ACTIVE" glow object is hidden.
    pub fn close_portals(&mut self, open: impl Fn(u32) -> bool) {
        let mut glows = Vec::new();
        let off = self.model_named("EXIT_OFF");
        let collision = &self.collision;
        let floor = |p: Vec3| collision.ground_height(p + Vec3::Y * 2.0, p.y + 2.0);
        for e in &mut self.entities {
            if let Kind::Exit {
                dest: Some(w),
                open: o,
                ..
            } = &mut e.kind
            {
                if !open(*w) {
                    *o = false;
                    if off.is_some() {
                        e.model = off;
                        e.animator = Animator::new(0);
                        // Not in the original: the pads sit up to 0.25 under
                        // the ring's floor, which hid the flat disc here.
                        if let Some(y) = floor(e.pos) {
                            e.pos.y = e.pos.y.max(y + 0.02);
                        }
                    }
                    // "L1NSNC%c%d_ACTIVE" with world + 0x40 and gate + 1.
                    let letter = char::from(0x40 + (*w >> 8) as u8);
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
        // The camera starts over at the next update, from this start.
        self.start_index = index;
        self.camera = Default::default();
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
        Self::new_heroes(fs, level_name, classes, &[], difficulty)
    }

    /// A level whose heroes may wear a hidden character's model
    /// (load_player_geo's hidden_code: PLAYERS/<class>/<dir> in place of the
    /// colour's model; the class's sounds and colour stay).
    pub fn new_heroes(
        fs: &GameFs,
        level_name: &str,
        classes: &[(&str, &str)],
        models: &[Option<&str>],
        difficulty: usize,
    ) -> Result<Game> {
        let model_dir = |i: usize| models.get(i).copied().flatten();
        let level = Level::load(fs, level_name)?;
        let mut collision = Collision::new(build_collision(&level));
        // fn_80088714: a wall of a world object flagged 0x38 (the STAIRS and
        // RAMP objects) doesn't slide or stop the player.
        for (i, t) in collision.tris.iter().enumerate() {
            collision.stairs[i] = usize::try_from(t.owner)
                .ok()
                .and_then(|o| level.worlds.world_objects.get(o))
                .is_some_and(|o| o.flags & 0x38 != 0);
        }
        let world_anim = crate::worldanim::WorldAnims::new(&level, &mut collision);

        let mut game = Game {
            obj_flags: level.worlds.all_flags(),
            level,
            collision,
            models: Vec::new(),
            player_seqs: Vec::new(),
            monster_seqs: HashMap::new(),
            shadow_models: HashMap::new(),
            classes: Vec::new(),
            players: Vec::new(),
            entities: Vec::new(),
            movers: Vec::new(),
            next_id: 1,
            cam_focus: Vec3::ZERO,
            cam_yaw: std::f32::consts::PI,
            cam_pitch: 60f32.to_radians(),
            camera: Default::default(),
            shake: Default::default(),
            start_index: 0,
            time: 0.0,
            frame: 0,
            rng: Rng(0x9E37_79B9_7F4A_7C15),
            wave: None,
            worlds: Default::default(),
            scrolls: Vec::new(),
            scroll_scale: 0.5,
            scroll_rom: None,
            scroll_box: None,
            sumner: None,
            trigger_cam: None,
            controls: Default::default(),
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
            ai_paths: Default::default(),
            magic: Vec::new(),
            items: Default::default(),
            magic_models: Default::default(),
            worn_models: Default::default(),
            worn_fx: Vec::new(),
            randpottype: 0,
            shield_cooldown: HashMap::new(),
            damg_cooldown: HashMap::new(),
            difficulty,
            friendly_fire: 0,
            exit_state: Default::default(),
            action_seqs: Vec::new(),
            pdata: Vec::new(),
            psfx: Vec::new(),
            fx_follow: Vec::new(),
            item_models: HashMap::new(),
            secret_timer: None,
            visible_coins: 0,
            shards: 0,
            garg_items: [0; 3],
            welcome: 300,
            crystals: [0; 9],
            crystal_shown: None,
            hidden: HashSet::new(),
            world_anim,
            transport: Default::default(),
            level_end: None,
            msgs: Default::default(),
            sfx: vec!["S_ENTRANCE".into()],
            boss: None,
            boss_runes: 0,
            legend_held: 0,
            debug_god: false,
            snd: Default::default(),
            spews: Vec::new(),
            crit_waves: Vec::new(),
            crit_hit_until: [0.0; 4],
            crit_ntargets: [0.0; 4],
            critter_wads: HashMap::new(),
            wizard_model: None,
            hit_from: None,
            gen_streak: [0; 4],
            nag: FollowNag::default(),
            effects: Default::default(),
            particles: Default::default(),
            psys_banks: Vec::new(),
        };
        game.ai_paths =
            crate::ai::Paths::new(&game.level.worlds.locators, game.level.start_positions()[0]);
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
                // fn_800553B4: a timed level (flags & 4: the secret ones)
                // lasts its wavetime (+0.99 s), then sends the party back.
                if let Some(l) = game
                    .wave
                    .and_then(|wv| w.level(wv))
                    .filter(|l| l.flags & 4 != 0)
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

        for (i, (class, color)) in classes.iter().enumerate() {
            let m = Model::load_player(fs, class, model_dir(i).unwrap_or(color))?;
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
        // load_player_geo: each hero's SHADOWL1, from its own model bank.
        for (i, (class, color)) in classes.iter().enumerate() {
            if let Some(m) = crate::shadow::load_player(fs, class, model_dir(i).unwrap_or(color)) {
                let idx = game.add_model(m);
                game.shadow_models.insert(i, idx);
            }
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
                exit: Default::default(),
                trial_move: Vec3::ZERO,
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
                fxhittime: 0.0,
                combo_fade: 0.0,
                floor_fx_time: 0.0,
                hit_force: Vec3::ZERO,
                knock_vel: Vec3::ZERO,
                grab_partner: None,
                grab_pending: None,
                combo_timer: 0,
                saved_pos: Vec3::ZERO,
                attached_to: None,
                worn: Default::default(),
                sumner: false,
                coins: 0,
                coin_timer: -1.0,
                class_unlock: 0,
            });
        }
        // Ranged-attack projectile: the class's throw model, else its weapon.
        for (i, (class, color)) in classes.iter().enumerate() {
            let dir = format!("PLAYERS/{class}/{}", model_dir(i).unwrap_or(color));
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
        // PlayerSfxInitData: each record's effect from the hero's SFX tree
        // (players/<class>/sfx<colour>; a secret hero's base class), else
        // WEAPONS or POWERUPS (InitCustomEffectSub), and its sound.
        let mut cache: HashMap<String, Option<AssetFile>> = HashMap::new();
        for (i, (_, color)) in classes.iter().enumerate() {
            let ct = game.players[i].class_id;
            let base = crate::select::CLASSES[if (8..16).contains(&ct) { ct - 8 } else { ct }];
            let dirs = [
                format!("PLAYERS/{base}/SFX{}", color.get(..3).unwrap_or(color)),
                "WEAPONS".to_string(),
                "POWERUPS".to_string(),
            ];
            let mut known = HashMap::new();
            let recs = game.pdata[i].sfx.clone();
            let res = recs
                .iter()
                .map(|r| game.resolve_plyr_sfx(fs, &mut cache, &mut known, &dirs, r))
                .collect();
            game.psfx.push(res);
        }
        game.load_magic_models(fs, &mut cache);
        game.load_worn_models(fs, &mut cache, classes);
        game.cam_focus = game.players.first().map(|p| p.pos).unwrap_or(Vec3::ZERO);
        for p in &mut game.players {
            p.refresh_stats();
        }
        game.populate(fs, classes.len());
        game.init_particles(fs);
        Ok(game)
    }

    /// fn_8008A678: a particle or skin record (flags & 0x0F000100) names a
    /// texture, anything else an effect tree; a particle's snddesc is the
    /// node it hangs from, not a sound.
    fn resolve_plyr_sfx(
        &mut self,
        fs: &GameFs,
        cache: &mut HashMap<String, Option<AssetFile>>,
        known: &mut HashMap<String, Option<usize>>,
        dirs: &[String],
        r: &gdl_formats::pdata::SfxRecord,
    ) -> PlyrSfx {
        let name = &r.fxdesc;
        let effect = r.flags & 0x0F00_0100 == 0
            && !name.is_empty()
            && dirs.iter().any(|d| {
                cache
                    .entry(d.clone())
                    .or_insert_with(|| load_asset(fs, d))
                    .as_ref()
                    .and_then(|a| a.anim.as_ref())
                    .is_some_and(|a| a.atrees.iter().any(|t| t.name.eq_ignore_ascii_case(name)))
            });
        let model = if effect {
            self.item_model(fs, cache, known, dirs, name)
        } else {
            None
        };
        let sound =
            (r.flags & 0x0F00_0000 == 0 && !r.snddesc.is_empty()).then(|| r.snddesc.clone());
        // A particle's texture: the hero's SFX bank, else the global ones.
        let psys_tex = if r.flags & 0x0F00_0000 != 0 {
            let own = dirs.iter().find_map(|d| {
                let a = cache
                    .entry(d.clone())
                    .or_insert_with(|| load_asset(fs, d))
                    .as_ref()?;
                crate::mesh::decode_bitmap(&a.objects, &a.textures, name)
            });
            self.psys_texture(name, own.as_ref())
        } else {
            None
        };
        PlyrSfx {
            effect,
            model,
            sound,
            psys_tex,
        }
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
        let mut shadows = [None; 3];
        for (slot, name) in crate::shadow::ENEMY_SHADOWS.iter().enumerate() {
            if let Some(m) = crate::shadow::load(&asset.objects, &asset.textures, name) {
                shadows[slot] = Some(self.add_model(m));
            }
        }
        self.classes.push(MonsterClass {
            name: dir,
            models,
            gen_models,
            etype,
            missiles,
            shadows,
        });
        Some(self.classes.len() - 1)
    }

    /// Whether a point is on the 4:3 screen, give or take `margin` units.
    pub(crate) fn on_screen(&self, pos: Vec3, margin: f32) -> bool {
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
        let hushed =
            self.trigger_camera_on() || self.sumner.as_ref().is_some_and(|s| s.controls_locked());
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
            .chain(std::iter::once((
                sounds::DEATH_SUCK_LOOP,
                "S_DEATHSUCK",
                if self.snd.death_suck > 0 { 1.0 } else { 0.0 },
            )))
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
                p.statue.is_none() && self.placement_in_view(at, p.pos, p.visrad)
            })
            .collect();
        for i in ready.into_iter().rev() {
            let p = self.pending_enemies.remove(i);
            self.spawn(p.kind, p.model, p.pos, p.yaw);
        }
    }

    /// fn_80060114's test: a placement within 50 of the camera's attention
    /// point `at` and on screen (4 x its visibility radius).
    fn placement_in_view(&self, at: Vec3, pos: Vec3, visrad: f32) -> bool {
        (pos - at).length() <= 50.0 && self.on_screen(pos, 4.0 * visrad)
    }

    /// The statues (triggers::Statue): a hero bumping one wakes it
    /// (fn_8005D730 case 4, when its placement's rad is not negative); a
    /// woken one in view plays its ACTIVE sequence (daction 1) and then the
    /// monster takes its place (fn_80060114).
    fn update_statues(&mut self) {
        use crate::triggers::Statue;
        let heroes: Vec<Vec3> = self
            .players
            .iter()
            .filter(|p| p.state.active())
            .map(|p| p.pos)
            .collect();
        // PlayerCollideEnemies' cylinder test against the item.
        let touched = |s: &Statue| {
            s.touch
                && heroes.iter().any(|h| {
                    Vec2::new(h.x - s.pos.x, h.z - s.pos.z).length() < s.radius + PLAYER_RADIUS
                        && (h.y - s.pos.y).abs() < s.height.max(PLAYER_HEIGHT)
                })
        };
        let at = if self.fixed_camera.is_some() {
            None
        } else {
            Some(self.camera_view().1)
        };
        // Each statue's next state: still waiting, or done (the monster's turn).
        let step = |g: &Game, s: &mut Statue| -> bool {
            if !s.woken && touched(s) {
                s.woken = true;
            }
            if s.woken
                && !s.rising
                && at.is_some_and(|at| g.placement_in_view(at, s.pos, s.radius.max(1.0)))
            {
                s.rising = true;
            }
            if !s.rising {
                return false;
            }
            // Done once its ACTIVE sequence has played (or it has none).
            match s.prop.and_then(|id| g.entities.iter().find(|e| e.id == id)) {
                Some(e) => {
                    let active = e.model.and_then(|m| g.models[m].sequence("ACTIVE"));
                    active.is_none_or(|a| e.animator.seq == a && e.animator.finished)
                }
                None => true,
            }
        };
        let mut starts = Vec::new();
        let mut done = Vec::new();
        for i in 0..self.entities.len() {
            let Kind::Critter(c) = &self.entities[i].kind else {
                continue;
            };
            let Some(mut s) = c.statue.filter(|_| self.entities[i].alive) else {
                continue;
            };
            let was = s.rising;
            let finished = step(self, &mut s);
            if s.rising && !was {
                starts.extend(s.prop);
            }
            if let Kind::Critter(c) = &mut self.entities[i].kind {
                if finished {
                    c.statue = None;
                    c.alpha = 1.0;
                    done.extend(s.prop);
                } else {
                    c.statue = Some(s);
                }
            }
        }
        for i in 0..self.pending_enemies.len() {
            let Some(mut s) = self.pending_enemies[i].statue else {
                continue;
            };
            let was = s.rising;
            let finished = step(self, &mut s);
            if s.rising && !was {
                starts.extend(s.prop);
            }
            if finished {
                done.extend(s.prop);
            }
            self.pending_enemies[i].statue = (!finished).then_some(s);
        }
        for e in self.entities.iter_mut() {
            if starts.contains(&e.id) {
                if let Some(a) = e.model.and_then(|m| self.models[m].sequence("ACTIVE")) {
                    e.animator.force_loop = false;
                    e.animator.restart(a);
                }
            }
            if done.contains(&e.id) {
                e.alive = false;
            }
        }
    }

    /// Trigger flag 0x2000 (item update case 5, fn_80062FF0(10, pos, 4)):
    /// wake the nearest statue that is on screen (item active 0x4000) or
    /// always active (0x40), if its edge is within 10 of the trigger.
    pub(crate) fn ambush(&mut self, at: Vec3) {
        // (fn_80062FF0 passes over statues already woken.)
        let found =
            |s: &crate::triggers::Statue| !s.woken && (s.always || self.on_screen(s.pos, s.radius));
        // (critter entity or pending Death, distance to its edge)
        let mut best: Option<(bool, usize, f32)> = None;
        let mut consider = |critter: bool, i: usize, s: &crate::triggers::Statue| {
            let d = Vec2::new(s.pos.x - at.x, s.pos.z - at.z).length() - s.radius;
            if best.is_none_or(|b| d < b.2) {
                best = Some((critter, i, d));
            }
        };
        for (i, e) in self.entities.iter().enumerate() {
            if let Kind::Critter(c) = &e.kind {
                if let Some(s) = c.statue.filter(|s| e.alive && found(s)) {
                    consider(true, i, &s);
                }
            }
        }
        for (i, p) in self.pending_enemies.iter().enumerate() {
            if let Some(s) = p.statue.filter(|s| found(s)) {
                consider(false, i, &s);
            }
        }
        match best {
            Some((true, i, d)) if d < crate::triggers::AMBUSH_RANGE => {
                if let Kind::Critter(c) = &mut self.entities[i].kind {
                    if let Some(s) = &mut c.statue {
                        s.woken = true;
                    }
                }
            }
            Some((false, i, d)) if d < crate::triggers::AMBUSH_RANGE => {
                if let Some(s) = &mut self.pending_enemies[i].statue {
                    s.woken = true;
                }
            }
            _ => {}
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
        // fn_8004F87C: the spew the monster's algorithm comes from; logic 12
        // spawns straight ahead only.
        let spew = crate::items::resolve_spew(etype, sp.strength as i32, sp.ai);
        let (ndirs, mask) = crate::items::gen_dirs(etype, spew);
        // gendir_8004FBC8's eight directions, from a random one round, the
        // masked ones skipped; the item's height + the monster's radius out.
        const OCTANTS: [f32; 8] = [0.0, 180.0, -90.0, 90.0, 45.0, -45.0, 135.0, -135.0];
        let dist = if etype == 17 {
            0.0
        } else {
            sp.height + t.radius
        };
        let start = self.rng.next_u32() as usize % ndirs;
        for k in 0..ndirs {
            let d = (start + k) % ndirs;
            if mask & (1 << d) != 0 {
                continue;
            }
            let a = sp.yaw + OCTANTS[d].to_radians();
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
            let mut kind = self.new_enemy(sp.class, sp.strength, EnemyState::Spawning);
            // fn_800606FC: ai 13 chains each spawn behind the last (zombie
            // lines); ai 14 sends them off 45 degrees either way in turn.
            let chain = self
                .entities
                .iter()
                .find(|e| e.id == sp.generator)
                .and_then(|e| match &e.kind {
                    Kind::Generator { chain, .. } => Some(*chain),
                    _ => None,
                })
                .unwrap_or_default();
            if let Kind::Enemy { ai, brain, .. } = &mut kind {
                // generate_single sets a lookout's algorithm outright.
                *ai = if sp.ai == 15 {
                    15
                } else {
                    crate::items::algorithm(etype, spew)
                };
                brain.generator = Some(sp.generator);
                if spew == 13 {
                    brain.prev = chain.tail;
                }
            }
            // items.c:3935: once a hit egg has set num_generated to 3, the
            // next logic-12 spawn starts a new clutch.
            if spew == 12 {
                if let Some(Kind::Generator { chain, .. }) = self
                    .entities
                    .iter_mut()
                    .find(|e| e.id == sp.generator)
                    .map(|e| &mut e.kind)
                {
                    if chain.generated >= 3 {
                        chain.generated -= 3;
                        chain.flags = 0;
                    }
                }
            }
            let id = self.spawn(kind, model, Vec3::new(p.x, g, p.z), a);
            if spew == 12 {
                self.place_logic12(sp.generator, id);
            }
            if spew == 14 {
                if let Some(Kind::Enemy { brain, .. }) =
                    self.entities.last_mut().map(|e| &mut e.kind)
                {
                    brain.zigzag(chain.generated & 1 != 0);
                }
            }
            // generate_enemy: a tall tier 1-3 monster comes out of GENFX.
            if 0.5 * t.height > 2.0
                && sp.strength <= 3
                && self.effects.genfx.get(etype).copied().unwrap_or(false)
            {
                self.start_gen_fx(Vec3::new(p.x, g, p.z), sp.strength);
            }
            if let Some(Kind::Generator { chain, .. }) = self
                .entities
                .iter_mut()
                .find(|e| e.id == sp.generator)
                .map(|e| &mut e.kind)
            {
                // place_logic12's spawns aren't counted (items.c:3973).
                if spew != 12 {
                    chain.generated += 1;
                }
                if spew == 13 {
                    chain.tail = Some(id);
                }
            }
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
        self.new_enemy_ai(class, strength, state, -1)
    }

    /// new_enemy with generate_enemy's spew: the generator's or placement's
    /// ai (-1 for the type's default routine).
    pub(crate) fn new_enemy_ai(
        &self,
        class: usize,
        strength: usize,
        state: EnemyState,
        ai: i32,
    ) -> Kind {
        let etype = self.classes[class].etype;
        let t = &crate::enemies::TYPES[etype.min(33)];
        let sight = 30.0 * self.tuning.visrad;
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
            drained: false,
            blink: 0.0,
            brain: Box::new(crate::ai::Brain::new(strength as i32, ai, 0.0, sight)),
            ai: crate::items::algorithm(
                etype,
                crate::items::resolve_spew(etype, strength as i32, ai),
            ),
            dest: None,
            pushed: Vec3::ZERO,
            pushang: 0.0,
        }
    }

    pub(crate) fn spawn(
        &mut self,
        mut kind: Kind,
        model: Option<usize>,
        pos: Vec3,
        yaw: f32,
    ) -> u64 {
        if let Kind::Enemy { brain, .. } = &mut kind {
            brain.set_heading(yaw);
        }
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
            pitch: 0.0,
            scale: 1.0,
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
                        // A tree of only particle nodes still is one.
                        if !m.mesh.indices.is_empty() || !m.psys.is_empty() {
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
        let items_option = items::items_option();
        // Golems, gargoyles and generals placed, for what they carry.
        let mut carriers: Vec<(u64, Vec3)> = Vec::new();
        for inst in &instances {
            // ItemVisible: placed for this many players.
            if !items::item_visible(inst.min_players, num_players.max(1), items_option) {
                continue;
            }
            let mut info_idx = inst.item_index as usize;
            let Some(mut info) = infos.get(info_idx).cloned() else {
                continue;
            };
            // Random items pick one of their candidates.
            if info.item_type == ItemType::Random && !info.random_indices.is_empty() {
                let pick =
                    info.random_indices[self.rng.next_u32() as usize % info.random_indices.len()];
                match infos.get(pick as usize) {
                    Some(i) => {
                        info = i.clone();
                        info_idx = pick as usize;
                    }
                    None => continue,
                }
            }
            let first_id = self.next_id;
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
                    // SetItem case 4: golems, gargoyles and Death wait as
                    // statues (the model from the monster's own files).
                    let statue = match crate::triggers::statue_model(ty, p16(0)) {
                        Some((name, woken)) => {
                            let variant = self.enemy_variant(ty);
                            let letter = self.world_letter();
                            let sdirs: Vec<String> = match ty {
                                29 => vec![
                                    format!(
                                        "MONSTERS/GOLEM/{}",
                                        variant
                                            .filter(|v| v.starts_with("level"))
                                            .unwrap_or_else(|| format!("level{letter}"))
                                    ),
                                    "MONSTERS/GOLEM/levelA".into(),
                                ],
                                32 => vec![format!(
                                    "MONSTERS/GAR_{}",
                                    variant
                                        .unwrap_or_else(|| "eagl".into())
                                        .to_ascii_uppercase()
                                )],
                                _ => vec!["MONSTERS/DEATH".into()],
                            };
                            let model = self.item_model(fs, &mut cache, &mut known, &sdirs, name);
                            let rad = f32::from_le_bytes(inst.params[4..8].try_into().unwrap());
                            Some(crate::triggers::Statue {
                                pos,
                                radius: info.radius,
                                height: info.height,
                                touch: rad >= 0.0,
                                always: inst.flags & 1 != 0,
                                woken,
                                rising: false,
                                prop: model.map(|m| self.spawn(Kind::Prop, Some(m), pos, yaw)),
                            })
                        }
                        None => None,
                    };
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
                        if let Some(id) =
                            self.spawn_critter(fs, &wad, variant.as_deref(), pos, yaw, num_players)
                        {
                            carriers.push((id, pos));
                            if let Some(Kind::Critter(c)) = self.entity_mut(id).map(|e| &mut e.kind)
                            {
                                // Only its statue shows until it wakes.
                                c.statue = statue;
                                if statue.is_some_and(|s| s.prop.is_some()) {
                                    c.alpha = 0.0;
                                }
                            }
                        }
                        continue;
                    }
                    // SetItem case 4: the placement's ai (its name's default
                    // routine when negative), sight radius and throw interval.
                    let ai = match p16(2) {
                        a if a < 0 => crate::ai::DEFAULT_ALG[ty.min(33)],
                        a => a as i32,
                    };
                    let rad = f32::from_le_bytes(inst.params[4..8].try_into().unwrap());
                    let interval = p16(8);
                    let ty = self.roster.resolve(ty, strength as i32);
                    if let Some(c) = self.monster_class(fs, &mut cache, ty) {
                        let model = self.classes[c].model(strength.max(1));
                        // fn_80060114: strength 0 lies dormant (not Death or IT);
                        // otherwise below 4 it starts stunned for 30 ticks.
                        let asleep = strength == 0
                            && ty != crate::enemies::DEATH
                            && ty != crate::enemies::IT;
                        let state = if asleep {
                            EnemyState::Asleep
                        } else {
                            EnemyState::Idle
                        };
                        let mut kind = self.new_enemy(c, strength.max(1), state);
                        if let Kind::Enemy {
                            cooldown,
                            ai: alg,
                            brain,
                            ..
                        } = &mut kind
                        {
                            *cooldown = if !asleep && strength < 4 { 0.5 } else { 0.0 };
                            // SetItem case 4: params[2] picks the AI (-1: the
                            // name's own), through generate_enemy's fn_8004F87C.
                            *alg =
                                items::algorithm(ty, items::resolve_spew(ty, strength as i32, ai));
                            // fn_80060114: placed monsters never flee the suicide
                            // grunt; Death sees 1000 off.
                            brain.birth_style = 1;
                            if ty == crate::enemies::DEATH {
                                brain.sight = 1000.0;
                            } else if rad > 0.0 {
                                brain.sight = rad * self.tuning.visrad;
                            }
                            if interval > 0 {
                                brain.idle_time = if interval == 1 {
                                    0.0
                                } else {
                                    0.1 * interval as f32
                                };
                            }
                        }
                        // fn_80060114: it appears once it comes into view.
                        self.pending_enemies.push(PendingEnemy {
                            kind,
                            model,
                            pos,
                            yaw,
                            visrad: info.radius.max(1.0),
                            statue,
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
                                ai: match p16(2) {
                                    a if a < 0 => crate::items::DEFAULT_AI[ty.min(33)],
                                    a => a as i32,
                                },
                                height: info.height,
                                side: 0,
                                chain: Default::default(),
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
                    self.note_special(&kind);
                    if matches!(kind, PickupKind::Gold(_)) {
                        self.visible_coins += 1;
                    }
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
                // SetItem case 10: params[0] overrides the subtype.
                ItemType::Obstacle
                    if matches!(
                        if p16(0) > 0 {
                            p16(0) as i32
                        } else {
                            info.subtype
                        },
                        0x28 | 0x31 | 0x34 | 0x35
                    ) && info.name.is_empty() =>
                {
                    let sub = if p16(0) > 0 {
                        p16(0) as i32
                    } else {
                        info.subtype
                    };
                    // The model is the level object the instance names.
                    let model = Model::from_objects(
                        &self.level.objects,
                        &self.level.textures_file,
                        &[&inst.name],
                    )
                    .map(|m| self.add_model(m));
                    let rate = match sub {
                        0x31 => 0.174_532_93,
                        0x35 => 0.017_453_292,
                        _ => 0.349_065_85,
                    };
                    let i = self.entities.len();
                    let tris = (inst.coll_tri_count > 0 && inst.coll_tri_index >= 0)
                        .then(|| (inst.coll_tri_index as usize, inst.coll_tri_count as usize));
                    self.spawn(
                        Kind::Faller {
                            sub,
                            active: false,
                            vel: Vec3::ZERO,
                            tumble: rate * FALLER_TUMBLE[i & 7],
                            radius: info.radius,
                            height: info.height,
                            tris,
                        },
                        model,
                        pos,
                        0.0,
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
                    // fn_8005C1DC goes by the info's subtype (not SetItem case
                    // 10's params[0], which barrels set to 0x2B). A type-2
                    // barrel breaks rather than opening.
                    let sub = info.subtype;
                    let id = self.spawn(
                        Kind::Container {
                            hp: info.hit_points.max(1) as f32,
                            drop,
                            chest: info.item_type == ItemType::Container
                                && sub != crate::items::SUB_BARREL,
                            armor: info.armor as f32,
                            sub,
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
                    let anim = Box::new(exits::ExitAnim::new(model.map(|m| &self.models[m])));
                    self.spawn(
                        Kind::Exit {
                            radius: PLAYER_RADIUS + info.radius,
                            height: info.height + PLAYER_HEIGHT,
                            dest,
                            open: true,
                            anim,
                        },
                        model,
                        pos,
                        yaw,
                    );
                }
                // SetItem case 8: damage = the placed value (or the item's) x
                // trap_damage; a placed delay sets the off time to -3 x it.
                ItemType::DamageTile | ItemType::Trap => {
                    let model = self.item_model(fs, &mut cache, &mut known, &dirs, &info.name);
                    let placed = p16(0);
                    let damage = if placed != 0 {
                        placed as f32
                    } else {
                        info.value as f32
                    } * self.tuning.trap_damage;
                    let off = match p16(2) {
                        0 => info.active_off,
                        d => -d * 3,
                    };
                    let mut active = info.active_type;
                    if inst.flags & 1 != 0 {
                        active |= 0x40;
                    }
                    let cycle = match model {
                        Some(m) => ItemCycle::new(
                            &self.models[m],
                            active,
                            info.active_on,
                            off,
                            self.tuning.trap_rate,
                            &mut self.rng,
                        ),
                        None => ItemCycle {
                            anim: Default::default(),
                            slots: Vec::new(),
                            action: 2,
                            daction: 2,
                            activetime: 0,
                            active: 1,
                            on: 0,
                            off: 0,
                            rate: 1.0,
                        },
                    };
                    let id = self.spawn(
                        Kind::Hazard {
                            radius: info.radius.max(1.0),
                            damage,
                            flags: info.properties | DmgType::STUN,
                            cycle: Box::new(cycle),
                        },
                        model,
                        pos,
                        yaw,
                    );
                    // AudioDamageTile's subtype: realm 3's FLAMEH plays as 6.
                    let raw = info.subtype;
                    let sub = if self.wave.map(|w| w >> 8) == Some(3)
                        && raw == 1
                        && info.name.eq_ignore_ascii_case("FLAMEH")
                    {
                        6
                    } else {
                        raw
                    };
                    self.snd.tile_sub.insert(id, (raw, sub));
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
                                self.add_mover(m)
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
                            flags: crate::triggers::trigger_bits(info.subtype, p16(2) as u16),
                            // LinkTriggerToCam: the tower keeps some indices
                            // for its own cameras.
                            cam: (self.wave != Some(gdl_formats::wdata::TOWER_WAVE)
                                || !crate::triggers::tower_camera_index(inst.params[6]))
                            .then(|| {
                                crate::triggers::trigger_camera(&self.level.worlds, inst.params[6])
                            })
                            .flatten(),
                            door_switch: info.subtype == 0x17,
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
                    // SetItem case 11: the pad's id and its destination's.
                    if info.item_type == ItemType::Transporter {
                        let w = |k: usize| {
                            i32::from_le_bytes(inst.params[k..k + 4].try_into().unwrap())
                        };
                        self.transport.pads.push(crate::transport::Pad {
                            pos,
                            radius: info.radius.max(0.5),
                            id: w(0),
                            dest_id: w(4),
                            dest: None,
                        });
                    }
                    let model = self.item_model(fs, &mut cache, &mut known, &dirs, &info.name);
                    if model.is_some() {
                        self.spawn(Kind::Prop, model, pos, yaw);
                    }
                }
                _ => {}
            }
            // SetItem: an instance with flag 2 gets no geometry (SetItemGeo is
            // skipped) - an invisible trigger pad, chain link or hidden item that
            // still works. Placed monsters get theirs from the enemy code.
            if !items::attach_geometry(inst.flags) {
                for e in self.entities.iter_mut().filter(|e| e.id >= first_id) {
                    if !matches!(e.kind, Kind::Enemy { .. }) {
                        e.model = None;
                    }
                }
            }
            // Remember which info each power-up, container and generator is.
            for e in self.entities.iter().filter(|e| e.id >= first_id) {
                if matches!(
                    e.kind,
                    Kind::Pickup { .. } | Kind::Container { .. } | Kind::Generator { .. }
                ) {
                    self.items.info.insert(e.id, info_idx);
                }
            }
        }
        self.link_carried(&carriers);
        self.load_item_fx(fs, &mut cache, &mut known, &dirs);
        self.append_bigape_powerups(fs, &mut cache, &mut known);
        self.transport.match_pads();
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
            // EXIT_OFF: the closed portals' model (fn_8005B5B8).
            let mut names: Vec<String> = vec!["GWIZ".into(), "WIZARD".into(), "EXIT_OFF".into()];
            names.extend((1..=8).map(|n| format!("SHARD{n}")));
            names.extend((1..=13).map(|n| format!("RUNE{n}")));
            for n in names {
                self.item_model(fs, &mut cache, &mut known, &dirs, &n);
            }
        }
        // do_see_thru's DEATH_ICON and KEYRING, StartCompass's COMPASS.
        for n in ["DEATH_ICON", "KEYRING", "COMPASS"] {
            self.item_model(fs, &mut cache, &mut known, &dirs, n);
        }
        self.item_models = known
            .iter()
            .map(|(k, v)| (k.to_ascii_uppercase(), *v))
            .collect();
        self.snd.compass_model = self.item_models.get("COMPASS").copied().flatten();
        self.init_effects(fs, &mut cache);
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

    /// The level's game-camera locator nearest to `p`, as its sTriggerCameras
    /// record holds it: (locator yaw + pi, locator pitch).
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
            .map(|l| {
                (
                    crate::camera::wrap(l.rot[1] + std::f32::consts::PI),
                    l.rot[0],
                )
            })
    }

    /// A realm sound table's entry for this level (sMusicTrackHi = realm).
    pub(crate) fn realm_sound(&self, table: &[i32; 16]) -> i32 {
        self.wave
            .and_then(|w| table.get((w >> 8) as usize))
            .copied()
            .unwrap_or(-1)
    }

    /// Play a sound by its id (sndid.h; bank << 16 | index), if any.
    pub(crate) fn sfx_id(&mut self, id: i32) {
        if id >= 0 {
            self.sfx.push(format!("#{id:x}"));
        }
    }

    /// get_player_pos (in play): a spot on the floor around another hero in
    /// play (0.5 + radius out, sixteen tries each, from a random one), else
    /// around the start position.
    fn get_player_pos(&mut self, pi: usize) {
        // lbl_80120BF0 + 16: the tries' (x, z) directions.
        const SPREAD: [(f32, f32); 16] = [
            (-2.0, -2.0),
            (-2.0, 0.0),
            (-2.0, 2.0),
            (0.0, -2.0),
            (0.0, 2.0),
            (2.0, -2.0),
            (2.0, 0.0),
            (2.0, 2.0),
            (-2.0, -1.0),
            (-2.0, 1.0),
            (-1.0, -2.0),
            (-1.0, 2.0),
            (1.0, -2.0),
            (1.0, 2.0),
            (2.0, -1.0),
            (2.0, 1.0),
        ];
        let r = 0.5 + PLAYER_RADIUS;
        let n = self.players.len();
        let first = self.rng.next_u32() as usize % 4;
        let mut bases: Vec<Vec3> = (0..4)
            .map(|j| (first + j) % 4)
            .filter(|&i| i != pi && i < n)
            .filter(|&i| self.players[i].state.active() && self.players[i].on_ground)
            .map(|i| self.players[i].pos)
            .collect();
        let start = self
            .level
            .worlds
            .locators
            .iter()
            .find(|l| {
                l.kind == gdl_formats::worlds::LocatorType::Start && l.next == self.start_index
            })
            .map(|l| Vec3::from(l.pos));
        bases.extend(start);
        let free = |c: &Collision, at: Vec3| {
            c.resolve_walls(at, PLAYER_RADIUS, PLAYER_HEIGHT, STEP_HEIGHT)
                .distance(at)
                < 0.01
        };
        for base in bases {
            for (dx, dz) in SPREAD {
                let at = base + Vec3::new(dx * r, 0.0, dz * r);
                if let Some(g) = self.collision.ground_height(at, base.y + 3.0) {
                    let at = Vec3::new(at.x, g, at.z);
                    if (g - base.y).abs() <= 3.0 && free(&self.collision, at) {
                        let p = &mut self.players[pi];
                        p.pos = at;
                        p.vy = 0.0;
                        p.knock_vel = Vec3::ZERO;
                        return;
                    }
                }
            }
        }
        // Nowhere free: the start itself.
        if let Some(b) = start {
            self.players[pi].pos = b;
            self.players[pi].vy = 0.0;
        }
    }

    /// WorldObjGetAllFlags of a collision triangle's owner when it hurts: a
    /// 0xF0000 material, and for 0x2000000 objects only while they move.
    pub(crate) fn hurting_surface(&self, owner: i32) -> Option<u32> {
        let o = usize::try_from(owner).ok()?;
        let live = self
            .world_anim
            .flags
            .get(o)
            .map_or(0, |f| f & crate::worldanim::wflags::HAS_ANIM);
        let flags = *self.obj_flags.get(o)? | live;
        if flags & 0xF0000 == 0 {
            return None;
        }
        if flags & 0x2000000 != 0 && !self.world_anim.animating(o) {
            return None;
        }
        Some(flags)
    }

    /// PlayerMotion_FloorFX: a world object flagged 0xF0000 (lava, fire, spikes)
    /// hurts the hero standing on or touching it, at most once a second. Objects
    /// flagged 0x2000000 only while they move; nothing in a reaction state.
    fn floor_fx(&mut self, pi: usize, owner: i32, dv: Vec3) {
        let Some(flags) = self.hurting_surface(owner) else {
            return;
        };
        let p = &self.players[pi];
        if p.act.motion >= 31 || self.time < p.floor_fx_time {
            return;
        }
        let dv = Vec3::new(dv.x, 0.0, dv.z).normalize_or_zero();
        let (dmg, knock) = match (flags >> 16) & 0xF {
            2 => (10.0, DmgType::KNOCKBACK),
            3..=5 => (15.0, DmgType::KNOCKDOWN),
            _ => (5.0, 0),
        };
        self.players[pi].floor_fx_time = self.time + 1.0;
        self.damage_from = (knock != 0).then_some(self.players[pi].pos - dv);
        self.damage_player(pi, dmg, knock);
        self.damage_from = None;
        if knock == DmgType::KNOCKDOWN {
            let id = self.realm_sound(&WORLD_HIT_SFX);
            self.sfx_id(id);
        }
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

    pub fn update(&mut self, dt: f32, inputs: &[PlayerInput]) {
        // Rumbles are the caller's to take after each update.
        self.snd.rumble.clear();
        self.time += dt;
        self.shake.age(1);
        if let Some((_, t)) = &mut self.crystal_shown {
            *t -= dt;
            if *t <= 0.0 {
                self.crystal_shown = None;
            }
        }
        for p in &mut self.players {
            if p.coin_timer > 0.0 {
                p.coin_timer -= dt;
            }
        }
        if let Some((_, left)) = &mut self.secret_timer {
            // fn_80054E78: each whole second a tick or tock (AudioFootstep:
            // 24 odd, 23 even, 25 the last), "time is running out" at 8 and
            // the 5-4-3-2-1 tally (DoAudioTallySFX) unless the level is
            // already ending.
            let before = *left as i32;
            *left -= dt;
            let now = *left;
            let n = now as i32;
            if now <= 0.0 {
                if self.level_end.is_none() {
                    self.level_end = Some(LevelEnd::SecretOver);
                }
            } else if before != n {
                self.sfx_id(match n {
                    0 => 25,
                    n if n & 1 != 0 => 24,
                    _ => 23,
                });
                if n == 8 {
                    self.sfx.push("S_TIMEISRUNNING".into());
                } else if n <= 5 && self.level_end.is_none() {
                    self.sfx_id(TALLY_SFX[n as usize]);
                }
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
        let mut locked = false;
        if let Some(mut s) = self.sumner.take() {
            s.update(self);
            locked = s.controls_locked();
            self.sumner = Some(s);
        }
        self.update_trigger_camera(dt);
        if locked || self.trigger_cam.is_some() {
            blank = vec![PlayerInput::default(); inputs.len()];
            inputs = &blank;
        }
        let inputs = self.legend_freeze_inputs(inputs);
        self.update_world_motion(dt);
        let held = self.update_transporters(dt);
        self.update_players(dt, &inputs);
        self.hold_transporting(held);
        self.update_music_area();
        self.legend_face_boss();
        self.update_magic(dt);
        self.update_statues();
        self.spawn_placed_enemies();
        self.update_entities(dt);
        self.update_items(dt);
        self.update_effects(dt);
        self.update_critters(dt);
        self.update_waves(dt);
        self.update_spews(dt);
        self.update_boss(dt);
        self.update_sounds();
        self.follow_nag(dt);
        self.check_level_end();
        self.update_camera(dt);
        self.update_particles();
    }

    /// fn_80055AFC: 20 s into a level (not a secret one) with no boss, a
    /// stretch of 10 s (7 s once it has nagged) without fighting brings out
    /// the milestone arrows and the "Follow" voice (msg 29), once per quiet
    /// stretch. Retail creates no milestone arrow nodes, so only the voice
    /// comes, and only the test level T2 has milestones at all.
    fn follow_nag(&mut self, dt: f32) {
        let ticks = (dt * 60.0).round() as i32;
        let n = &mut self.nag;
        let fighting = std::mem::take(&mut n.fighting);
        if self.wave.is_some_and(|w| w >> 8 == 12) {
            return;
        }
        n.level_ticks += ticks;
        if n.level_ticks < 1200 {
            return;
        }
        let limit = if n.count == 0 { 600 } else { 420 };
        let mut fighting = fighting;
        if !self.players.iter().any(|p| p.state.active()) {
            n.timer = limit;
            fighting = true;
        }
        if self.boss.is_some() {
            return;
        }
        if fighting {
            n.quiet = false;
        }
        n.timer += ticks;
        if n.timer < limit {
            return;
        }
        let post = n.quiet && !n.was_quiet && !self.ai_paths.milestones.is_empty();
        if n.quiet {
            n.count += 1;
        }
        n.was_quiet = n.quiet;
        n.quiet = true;
        n.timer -= limit;
        if post {
            self.post(29, None, &[]);
        }
    }

    fn update_players(&mut self, dt: f32, inputs: &[PlayerInput]) {
        let (fwd, right) = self.camera_basis();
        // Swings landing (player, damage multiplier, knockdown), throws (x size),
        // potion blasts, breath and hammer power-ups, damage rows firing.
        let mut swings: Vec<(usize, f32, u32)> = Vec::new();
        let mut throws: Vec<Throw> = Vec::new();
        // Potions released: (player, where, colour, start_magic mode), and
        // the heroes starting the MAGICFX.
        let mut rings: Vec<(usize, Vec3, u8, u8)> = Vec::new();
        let mut magic_fx: Vec<usize> = Vec::new();
        // pmotion.c:2800: no magic while the legend sequence (lbl_8034489C
        // 1..4) runs; the holder's own bits go to the legend weapon.
        let magic_ok = self
            .boss
            .as_ref()
            .is_none_or(|b| b.legend_state == 0 || b.legend_state >= 5);
        let mut breaths: Vec<usize> = Vec::new();
        let mut hammers: Vec<usize> = Vec::new();
        let mut damg: Vec<DamgFire> = Vec::new();
        let mut posts: Vec<(usize, usize)> = Vec::new();
        // lbl_80344740: the monsters on screen (do_enemies' visible count),
        // for the turbo and combo hints.
        let crowd = self
            .entities
            .iter()
            .filter(|e| match &e.kind {
                Kind::Enemy { etype, .. } if e.alive => {
                    let r = crate::enemies::TYPES[(*etype).min(33)].radius;
                    self.on_screen(e.pos, 2.0 * r)
                }
                _ => false,
            })
            .count();
        let party = self.players.len();
        let mut key_drops: Vec<(Vec3, u32)> = Vec::new();
        let mut floor_fx: Vec<(usize, i32, Vec3)> = Vec::new();
        let mut fallen: Vec<usize> = Vec::new();
        let legend_forced: Vec<_> = (0..self.players.len())
            .map(|i| self.legend_forced_action(i))
            .collect();
        let mut legend_fired = false;
        let in_tower = self.wave == Some(gdl_formats::wdata::TOWER_WAVE);
        let pu_rate = if self.boss.as_ref().is_some_and(|b| b.active && !b.dead) {
            3.0
        } else {
            1.0
        };
        let mut it_cleared = false;
        // Things an attack can be aimed at, with their radius.
        // "Short" targets (small monsters, breakables) get kicks and low swings.
        // The last flag: Auto Attack swings at it when walked into (a monster,
        // a non-boss critter or a generator; pmotion.c:1707).
        let mut targets: Vec<(Vec3, f32, bool, bool)> = self
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
                let bump = matches!(e.kind, Kind::Enemy { .. } | Kind::Generator { .. });
                Some((e.pos, Self::target_radius(e)?, short, bump))
            })
            .collect();
        // PlayerCollideItems: the monsters and critters a hero walks into
        // (position, radius, half height).
        let mut blockers: Vec<(Vec3, f32, f32)> = self
            .entities
            .iter()
            .filter_map(|e| match &e.kind {
                Kind::Enemy { state, etype, .. }
                    if e.alive
                        && *etype != crate::enemies::IT
                        && !matches!(state, EnemyState::Dying { .. }) =>
                {
                    let t = &crate::enemies::TYPES[(*etype).min(33)];
                    Some((e.pos, t.radius, t.height * 0.5))
                }
                _ => None,
            })
            .collect();
        // Critters are aimed at by their colnodes (at the players' height).
        for e in self.entities.iter().filter(|e| e.alive) {
            if let Kind::Critter(c) = &e.kind {
                if !c.hittable() {
                    continue;
                }
                let feet = self.players.first().map_or(e.pos.y, |p| p.pos.y);
                let bump = !c.is_boss();
                for (_, _, p, r) in c.colliders() {
                    targets.push((Vec3::new(p.x, feet, p.z), r, false, bump));
                }
                let cp = c.coll_pos(0, e.pos, e.yaw);
                let t0 = &c.wad.types[c.parts[0].ty];
                targets.push((Vec3::new(cp.x, feet, cp.z), t0.rad, false, bump));
                blockers.push((Vec3::new(cp.x, feet, cp.z), t0.rad, t0.hht.max(2.0)));
            }
        }
        // fn_80088EF4's candidates: who could be grabbed for a combo.
        let controls = self.controls;
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
                // AudioPlayerDies; kill_got_it.
                self.sfx.extend(sounds::player_dies(p));
                self.snd.got_it.retain(|g| g.player != i);
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
            p.exit.joymag = joymag;
            let joyang = if joymag > 0.0 {
                wish.x.atan2(wish.y)
            } else {
                p.yaw
            };
            // The player's Control Style picks the buttons (SpecialData);
            // Robotron's C-stick aims (fireang / firemag).
            let prefs = controls[i.min(3)];
            let (levels, edges) = inp.logical(prefs.control_style);
            let fire = inp.fire_stick(prefs.control_style);
            let fire_w = fwd * fire.y + right * fire.x;
            let firemag = fire_w.length().min(1.0);
            let fireang = fire_w.x.atan2(fire_w.y);
            let mut ctx = crate::paction::Ctx {
                joymag,
                stick_yaw: wrap_angle(joyang - p.yaw),
                firemag,
                fire_yaw: wrap_angle(fireang - joyang),
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
            // (the fire stick, the move stick, else the facing), measured to
            // its surface.
            let heading = if firemag > 0.0 {
                fireang
            } else if joymag > 0.0 {
                joyang
            } else {
                p.yaw
            };
            let hdir = Vec2::new(heading.sin(), heading.cos());
            let cone = if self.difficulty == 0 { 0.5 } else { 0.707 };
            let found = targets
                .iter()
                .filter(|(t, _, _, _)| (t.y - p.pos.y).abs() < 5.0)
                .filter_map(|(t, r, short, bump)| {
                    let d = Vec2::new(t.x - p.pos.x, t.z - p.pos.z);
                    let surf = d.length() - r;
                    (surf < TARGET_RANGE && (d.length() < 0.5 || d.normalize().dot(hdir) >= cone))
                        .then_some((surf, d, *short, t.y - p.pos.y, *bump))
                })
                .min_by(|a, b| a.0.total_cmp(&b.0));
            let (tdist, attack_yaw, short) = match found {
                Some((dist, d, short, _, _)) => (dist, d.x.atan2(d.y), short),
                None => (TARGET_RANGE, p.yaw, false),
            };
            // PlayerGetTarget's attackDir climbs to the target.
            let aim_rise = found.map(|(_, d, _, dy, _)| dy.atan2(d.length()));
            // Strafing and blocking keep the stick's direction, and with Auto
            // Aim on so does the fire stick (pmotion.c:1584).
            let attack_yaw = if p.act.hud & 0xC000 != 0
                || levels & (bits::STRAFE | bits::DEFEND) != 0
                || (prefs.auto_aim && firemag > 0.0)
            {
                heading
            } else {
                attack_yaw
            };
            ctx.melee_yaw = wrap_angle(attack_yaw - p.yaw);
            // Auto Attack (pmotion.c:1707): walking or running into a monster
            // or generator, with no attack button down, swings at it.
            let mut ms = ms;
            if prefs.auto_attack
                && matches!(ms, 8 | 0x0D)
                && p.act.edge_hist == 0
                && !(pa::QUICK..=114).contains(&p.act.cur)
                && found.is_some_and(|f| f.4)
                && tdist < 1.0 + PLAYER_RADIUS
            {
                ms = 0x0F;
                ctx.forced = true;
            }
            // contactRadius and movingBias are 0 for that swing.
            let r = if ctx.forced { 0.0 } else { PLAYER_RADIUS };
            let bias = if matches!(ms, 0x0F | 0x10) && !ctx.forced {
                1.0
            } else {
                0.0
            };
            let mut coll = if short && tdist < 2.0 + PLAYER_RADIUS {
                2
            } else {
                0
            };
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
            // The legend-weapon holder plays the throw (restarted unless it
            // already is).
            let forced = legend_forced[i];
            if let Some((a, b)) = forced {
                // (A class without the animation throws at once.)
                if self.action_seqs[p.model]
                    .slots
                    .get(a as usize)
                    .is_none_or(|s| s.is_none())
                {
                    legend_fired = true;
                } else if p.act.cur != a && p.act.cur != b {
                    p.act.cur = 0;
                    p.act.next = a;
                }
            }
            // A step swing turns by the stick, not the target.
            if p.act.next == pa::STEP {
                ctx.melee_yaw = ctx.stick_yaw;
            }
            p.act.do_action(&self.action_seqs[p.model], &ctx, dt);

            // The bits the action fired; the holder's go to the legend weapon.
            let mut bits_now = std::mem::take(&mut p.act.act_bits);
            if forced.is_some() && bits_now & !1 != 0 {
                bits_now = 0;
                legend_fired = true;
            }
            if bits_now & 0xFE != 0 {
                let mult = if bits_now & 0xF0 != 0 {
                    // Power swings: x3 and knockdown; the move's turbo is paid.
                    p.power = (p.power - p.act.coll_score).max(0.0);
                    p.act.coll_score = 0.0;
                    3.0
                } else if bits_now & act::HIT2 != 0 {
                    2.0
                } else {
                    1.0
                };
                swings.push((i, mult, bits_now));
            }
            if bits_now & act::SWING != 0 {
                p.combo_fade = self.time;
            }
            if bits_now & 0xFF00 != 0 {
                let rise = aim_rise.unwrap_or(0.0);
                throws.push(Throw {
                    pi: i,
                    pos: p.pos,
                    yaw: p.yaw,
                    aim: Vec3::new(
                        attack_yaw.sin() * rise.cos(),
                        rise.sin(),
                        attack_yaw.cos() * rise.cos(),
                    ),
                    target: aim_rise.is_some(),
                    bits: bits_now,
                    since_swing: self.time - p.combo_fade,
                });
            }
            // USE_MAGIC and THROW_MAGIC each start a MAGICFX on the hero.
            if bits_now & act::USE_MAGIC != 0 {
                magic_fx.push(i);
            }
            // The release uses the last potion picked up (none: a random
            // colour) in the mode the release chose.
            if bits_now & (act::MAGIC_RELEASE | act::THROW_MAGIC_RELEASE) != 0 && magic_ok {
                let mode = magic::magic_mode(bits_now, levels, false);
                let colour = if p.potions > 0 {
                    p.potions -= 1;
                    p.potion_colours.pop().unwrap_or(0)
                } else {
                    0
                };
                rings.push((i, p.pos, colour, mode));
            }
            if bits_now & act::BREATHE != 0 {
                breaths.push(i);
            }
            if bits_now & act::HAMMER != 0 {
                hammers.push(i);
            }
            // PWRA / 360 / PWRB / PWRC / combo / victory rows fire on their frames.
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
                pa::VICTORY => vec![rows.victory],
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
                        damg.push((i, row.clone(), t0, t1, p.grab_partner));
                    }
                }
            }
            p.act.prev_frame = frame;
            // The meter fills while not in a turbo move; a charge drains it.
            if p.act.action < 11 {
                p.power = (p.power + TURBO_FILL * dt).min(100.0);
                // pmotion.c:1755: with 15 or more monsters on screen, a full
                // meter says to use turbo (msg 110); at 40 with partners, to
                // try a combo (111).
                if crowd >= 15 {
                    if p.power >= 100.0 {
                        posts.push((110, i));
                    } else if p.power >= 40.0 && party > 1 {
                        posts.push((111, i));
                    }
                }
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
            // Blocking and strafing don't turn and walk with the stick; the
            // fire stick turns the body while the move stick walks
            // (pmotion.c:995); standing attacks face their target with Auto
            // Aim on (pmotion.c:2483).
            let mut head = if movement > 0.0 { mdir } else { p.yaw };
            let mut walk = None;
            if firemag > 0.0 {
                head = fireang;
                walk = Some(mdir);
            } else if matches!(ms, 2..=6 | 9..=12 | 0x11..=0x14) {
                head = p.yaw;
                walk = Some(mdir);
            }
            if (1..=10).contains(&a.action)
                && a.action != 7
                && movement == 0.0
                && levels & (bits::STRAFE | bits::DEFEND) == 0
                && prefs.auto_aim
                && firemag == 0.0
            {
                head = attack_yaw;
            }
            let turn = 15.707_963 * dt * a.turn_scale;
            p.yaw += wrap_angle(head - p.yaw).clamp(-turn, turn);
            let speed = a.move_scale * p.derived.speed * movement;
            let grabbed = a.hud & 0x20 != 0;
            let moving = speed > 0.0 && !grabbed;
            let start = p.pos;
            p.trial_move = Vec3::ZERO;
            if moving {
                let yaw = walk.unwrap_or(p.yaw);
                let dir = Vec3::new(yaw.sin(), 0.0, yaw.cos());
                let mut target = p.pos + dir * speed * dt;
                // PlayerCollideItems: walking into a monster stops the hero
                // with Auto Attack on (to swing at it), else slides round
                // it; a charge pushes on at 0.75.
                if !matches!(a.cur, pa::COMBO_WAR2 | pa::COMBO_DWF2) {
                    let flat = |a: Vec3, b: Vec3| Vec2::new(a.x - b.x, a.z - b.z).length();
                    let hits: Vec<&(Vec3, f32, f32)> = blockers
                        .iter()
                        .filter(|(b, r, h)| {
                            flat(target, *b) < r + PLAYER_RADIUS
                                && flat(target, *b) < flat(p.pos, *b)
                                && target.y > b.y - 1.0
                                && target.y < b.y + 2.0 * h
                        })
                        .collect();
                    if let Some(&&(b, r, _)) = hits.first() {
                        if a.cur == pa::SHOVE {
                            target = p.pos + (target - p.pos) * 0.75;
                        } else if prefs.auto_attack || hits.len() > 1 {
                            target = p.pos;
                        } else {
                            let out = Vec2::new(target.x - b.x, target.z - b.z).normalize_or_zero()
                                * (r + PLAYER_RADIUS);
                            target = Vec3::new(b.x + out.x, target.y, b.z + out.y);
                        }
                    }
                }
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
                p.trial_move = p.pos - start;
                let (mut pos, wall) = self.collision.resolve_walls_stairs(
                    p.pos,
                    PLAYER_RADIUS,
                    PLAYER_HEIGHT,
                    STEP_HEIGHT,
                    STAIR_RISE,
                );
                // PlayerCollideFloor: a stairs floor up to 3 above the feet is
                // stepped straight onto (floor_base follows it).
                if p.on_ground {
                    if let Some((y, t)) = self.collision.ground_tri(pos, pos.y + STAIR_RISE) {
                        if self.collision.stairs[t] && y > pos.y {
                            pos.y = y;
                        }
                    }
                }
                p.pos = Self::resolve_solids(&self.entities, pos, PLAYER_RADIUS, 0);
                p.on_ground = Self::snap_to_ground(&self.collision, &mut p.pos, &mut p.vy, dt);
                // PlayerMotion_FloorFX: the wall touched, then the floor stood on.
                if let Some((owner, at)) = wall {
                    floor_fx.push((i, owner, p.pos - at));
                }
                if p.on_ground {
                    if let Some((_, owner)) = self.collision.ground_hit(p.pos, p.pos.y + 0.1) {
                        floor_fx.push((i, owner, Vec3::ZERO));
                    }
                }
            }
            // Below the world: get_player_pos puts the hero back.
            if p.pos.y <= self.level.worlds.min_bounds[1] {
                fallen.push(i);
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
        for (pi, owner, dv) in floor_fx {
            self.floor_fx(pi, owner, dv);
        }
        for pi in fallen {
            self.get_player_pos(pi);
        }
        if legend_fired {
            if let Some(b) = self.boss.as_mut() {
                b.holder_fired = true;
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
        if it_cleared {
            self.it_holder = None;
        }
        for (m, pi) in posts {
            self.post(m, Some(pi), &[]);
        }
        for (pi, mult, bits) in swings {
            let (pos, yaw) = (self.players[pi].pos, self.players[pi].yaw);
            // pmotion.c:2716: Pojo's swing plays its ATTACK.
            if self.players[pi].fx.special & crate::powerups::S_POJO != 0 {
                self.players[pi].worn.pojo_swing = true;
            }
            self.player_attack(pi, pos, yaw, mult, bits);
        }
        for pi in hammers {
            self.start_hammer(pi);
        }
        for pi in breaths {
            self.start_breath(pi);
        }
        for (pi, row, t0, t1, partner) in damg {
            self.fire_damage_row(pi, &row, t0, t1, partner);
        }
        self.update_fx_follow();
        self.update_worn(dt);
        self.shield_contact(dt);
        for pi in magic_fx {
            self.start_magic_player_fx(pi);
        }
        for (pi, pos, colour, mode) in rings {
            self.start_magic(Some(pi), pos, colour as u32, mode, 1.0);
        }
        for t in throws {
            self.player_start_missile(t);
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

    /// pmotion.c's missile block and PlayerStartMissile: the throw's charge,
    /// scale and launch offset, the aimed arc, the launch sweep (a wall in the
    /// way bursts it, a barrel or generator there takes the hit), then a
    /// missile per shot of the fan (StartMissile).
    fn player_start_missile(&mut self, t: Throw) {
        use crate::missile as m;
        use crate::paction::act;
        use crate::powerups as pu;
        let pi = t.pi;
        self.sfx.push(sounds::throw_sound(&self.players[pi]));
        // act_bits 0x10000000: the familiars answer the throw.
        self.players[pi].worn.shot = Some((t.aim, t.target));
        let p = &self.players[pi];
        let ct = p.class_id % 8;
        let (mut damage, w) = (p.derived.missile_damage, p.fx.weapon);
        let mut flags = w & (pu::W_ELEMENT | pu::W_3WAY | pu::W_5WAY | pu::W_REFLECT);
        let pdata = self.pdata.get(p.model);
        // FIREL/R and the power-A throw have a fixed charge; the power-A
        // missile is big, double size and from the turbo-A hand.
        let (charge, scale, offset) = if t.bits & (act::FIREL | act::FIRER) != 0 {
            (0.06, 1.0, None)
        } else if t.bits & act::PWRA_THROW != 0 {
            flags |= m::DMG_TURBO | DmgType::KNOCKBACK;
            (0.06, 2.0, pdata.map(|d| d.turboa_offset))
        } else if t.bits & act::SUPER_SHOT != 0 {
            (0.0, 1.0, None)
        } else {
            let offset = pdata.map(|d| d.weapon_offset);
            (m::throw_charge(t.since_swing), 1.0, offset)
        };
        // PlayerStartMissile: Pojo spits from lbl_8011A1A8 whatever the mode.
        let offset = if p.fx.special & pu::S_POJO != 0 {
            Some(m::POJO_AIM)
        } else {
            offset
        };
        if w & pu::W_ELEMENT != 0 {
            damage *= ELEMENT_BONUS;
        }
        // A super shot doubles the damage, one use a throw.
        if self.players[pi].powerups.use_one(pu::WEAPON, pu::W_SUPER) {
            damage *= 2.0;
            flags |= m::DMG_SUPER;
        }
        let from = Vec3::new(t.yaw.sin(), 0.0, t.yaw.cos());
        let mut dir = t.aim;
        let reach = m::modify_dpos(from, &mut dir, 0.0, ct, flags, t.target, charge);
        let hand = t.pos
            + Vec3::Y * (PLAYER_HEIGHT * 0.5)
            + offset.map_or(Vec3::ZERO, |o| m::rotate_offset(o, t.yaw));
        self.start_missile_fan(pi, hand, dir, reach, damage, flags, scale, 1.0);
    }

    /// PlayerStartMissile from the launch point on: the sweep (a wall in the
    /// way bursts it, a barrel or generator there takes the hit), then a
    /// missile per shot of the fan (StartMissile), each falling at the
    /// class's weight x `weight_mul`. Returns the shots started.
    #[allow(clippy::too_many_arguments)]
    fn start_missile_fan(
        &mut self,
        pi: usize,
        hand: Vec3,
        dir: Vec3,
        speed_arg: f32,
        mut damage: f32,
        flags: u32,
        scale: f32,
        weight_mul: f32,
    ) -> usize {
        use crate::missile as m;
        let p = &self.players[pi];
        let (ct, speed, level) = (p.class_id % 8, p.derived.missile_speed, p.level);
        // PlayerStartMissile's tree: the boss gauntlets' bolts, Pojo's
        // PHOENIX_FBALL, the super shot's SUPERARROW, else the class's throw.
        let special = p.fx.special;
        let wm = &self.worn_models;
        let model = if special & 0x8000 != 0 {
            wm.boss_elec
        } else if special & 0x4000 != 0 {
            wm.boss_acid
        } else if special & crate::powerups::S_POJO != 0 {
            wm.fball
        } else if flags & m::DMG_SUPER != 0 && flags & m::DMG_TURBO == 0 {
            wm.superarrow
        } else {
            None
        }
        .or(p.proj_model);
        damage *= scale;
        let desc = m::MissileInfo::for_player(ct, special, flags);
        // A bolt's own element counts as an amulet's would.
        if desc.damage_type & 0xF != 0 && flags & 0xF == 0 {
            damage *= ELEMENT_BONUS;
        }
        // PlyrSfxDoDamage scales the started missiles' weight.
        let weight = if weight_mul > 0.0 {
            desc.weight * weight_mul
        } else {
            desc.weight
        };
        let pt0 = hand + dir * m::SWEEP_BACK;
        let mut pt1 = hand + dir * m::SWEEP_AHEAD;
        let r = desc.collision_radius;
        let push = Vec2::new(dir.x, dir.z).normalize_or_zero();
        // Only a wall faced from the sweep's start counts (not one at the back).
        let wall = self
            .collision
            .sweep(pt0, pt1, r)
            .is_some_and(|(_, n)| n.dot(pt1 - pt0) < 0.0);
        if flags & m::DMG_SUPER == 0 && wall {
            if flags & m::DMG_REFLECT == 0 {
                return 0;
            }
            pt1 = pt0;
        }
        // fn_8005ED44: a barrel, generator or potion in the sweep is hit at once.
        let item = (0..self.entities.len()).find(|&i| {
            let e = &self.entities[i];
            if matches!(e.kind, Kind::Pickup { .. }) {
                let (a, b) = (Vec2::new(pt0.x, pt0.z), Vec2::new(pt1.x, pt1.z));
                let q = Vec2::new(e.pos.x, e.pos.z);
                let ab = b - a;
                let k = ((q - a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
                let at = pt0 + (pt1 - pt0) * k;
                return self.missile_hits_potion(e, at, r);
            }
            if !e.alive
                || !matches!(
                    e.kind,
                    Kind::Generator { .. } | Kind::Container { chest: false, .. }
                )
            {
                return false;
            }
            let Some(reach) = Self::target_radius(e) else {
                return false;
            };
            let (a, b) = (Vec2::new(pt0.x, pt0.z), Vec2::new(pt1.x, pt1.z));
            let q = Vec2::new(e.pos.x, e.pos.z);
            let ab = b - a;
            let k = ((q - a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
            (a + ab * k).distance(q) < reach + r && (e.pos.y + 2.0 - hand.y).abs() < 3.5
        });
        if let Some(i) = item {
            if matches!(self.entities[i].kind, Kind::Pickup { .. }) {
                self.damage_item(i, damage, flags, Some(pi));
            } else {
                self.hit_from = Some(pt1);
                self.hit_entity(pi, i, damage, push, false);
                self.hit_from = None;
            }
            if flags & m::DMG_SUPER == 0 {
                return 0;
            }
        }
        let vel = m::launch_velocity(dir, speed, speed_arg, desc.weight, flags);
        let size = m::model_scale(scale, level, flags);
        let shots = m::shot_count(flags);
        for i in 0..shots {
            let v = m::spread(vel, i);
            let id = self.spawn(
                Kind::Projectile {
                    owner: pi,
                    vel: v,
                    life: m::lifetime(flags),
                    damage,
                    weight,
                    radius: desc.radius(flags),
                    tumble: desc.angular_velocity[0],
                    reflect: flags & m::DMG_REFLECT != 0,
                    pierced: (flags & m::DMG_SUPER != 0).then(Vec::new),
                    // StartMissile: the fan's first shot has its own sound.
                    wall_sound: match (flags & 0x480000 != 0, i) {
                        (false, _) => WallSound::Sel(5),
                        (true, 0) if flags & 0x400000 != 0 => WallSound::Sel(7),
                        (true, 0) => WallSound::Sel(6),
                        _ => WallSound::None,
                    },
                    hold: 0.0,
                    morph: None,
                },
                model,
                pt1,
                v.x.atan2(v.z),
            );
            if let Some(e) = self.entities.last_mut().filter(|e| e.id == id) {
                e.scale = size;
                e.pitch = Self::missile_pitch(v, desc.angular_velocity[0], 0.0);
            }
            // SfxSetHit / SfxSetDamage: its hit row and damage type.
            self.effects
                .missiles
                .insert(id, (desc.hit_effect, flags | desc.damage_type));
            self.start_streak(id, pi, flags, desc.radius(flags), m::lifetime(flags));
        }
        shots
    }

    /// A missile's pitch: a tumbler turns by its pyrvel, anything else points
    /// along its flight (CreateDirMatrix).
    fn missile_pitch(vel: Vec3, tumble: f32, pitch: f32) -> f32 {
        if tumble != 0.0 {
            pitch
        } else {
            -vel.y.atan2(vel.x.hypot(vel.z))
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
    fn player_attack(&mut self, pi: usize, pos: Vec3, yaw: f32, mult: f32, bits: u32) {
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
        let Some(t) = target else {
            if self.friendly_fire == 2 {
                self.find_closest_player(pi, dir, mult);
            }
            return;
        };
        if let Kind::Trigger { hit, .. } = &mut self.entities[t].kind {
            *hit = true;
            return;
        }
        // PlayerMotion_HitTarget: a swing that lands on a nameless obstacle
        // (the breakable walls) says it takes multiple hits (msg 20).
        if let Kind::Container {
            chest: false,
            armor,
            sub,
            ..
        } = self.entities[t].kind
        {
            let nameless = self.level.worlds.item_infos.iter().any(|i| {
                i.item_type == ItemType::Obstacle && i.subtype == sub && i.name.is_empty()
            });
            if nameless && armor >= 0.0 && self.entities[t].alive {
                self.post(20, Some(pi), &[]);
            }
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
        // The hit streak counts critters and tall monsters: 3 for a kill (or
        // a blow that does nothing: damage_enemy's -1), else 1.
        let tall = match &self.entities[t].kind {
            Kind::Critter(_) => true,
            Kind::Enemy { etype, .. } => crate::enemies::TYPES[(*etype).min(33)].height * 0.5 > 2.0,
            _ => false,
        };
        // pmotion.c:2654: the swing's damage type is the weapon's, a power move
        // knocking down, a HIT2 swing knocking back, and a kick knocking down
        // a monster at 2 health or less; its direction rises 0.05 x the
        // damage (at most 2) (PlayerMotion_DamageTarget).
        use crate::paction::act;
        let mut dtype = p.fx.weapon;
        if bits & 0xF0 != 0 {
            dtype |= DmgType::KNOCKDOWN;
        } else if bits & act::HIT2 != 0 {
            dtype |= DmgType::KNOCKBACK;
        } else if bits & act::KICK != 0
            && matches!(self.entities[t].kind, Kind::Enemy { hp, .. } if hp <= 2.0)
        {
            dtype |= DmgType::KNOCKDOWN;
        }
        let was_down = Self::is_destroyed(&self.entities[t]);
        self.effects.hit_dtype = Some(dtype);
        self.effects.hit_lift = Some((0.05 * dmg).min(2.0));
        self.hit_entity(pi, t, dmg, dir, true);
        self.effects.hit_dtype = None;
        self.effects.hit_lift = None;
        if tall {
            let down = self.entities.get(t).is_none_or(Self::is_destroyed);
            self.streak_hit(pi, was_down || down);
        }
    }

    /// PlayerMotion_FindClosestPlayer (Multiplayer option 2, Shots Hurt): a
    /// swing that found nothing else hits the nearest other hero in play
    /// within a 45 degree cone ahead and 2 of the swinger's radius from their
    /// surface, with the swing's damage and its direction.
    fn find_closest_player(&mut self, pi: usize, dir: Vec2, mult: f32) {
        let Some(p) = self.players.get(pi) else {
            return;
        };
        let fwd = Vec3::new(dir.x, 0.0, dir.y);
        let mut best = 2.0 + PLAYER_RADIUS;
        let mut closest = None;
        for (i, op) in self.players.iter().enumerate() {
            if i == pi || !op.state.active() || op.exit.waiting {
                continue;
            }
            let d = op.pos - p.pos;
            let len = d.length();
            if d.normalize_or_zero().dot(fwd) < 0.707 {
                continue;
            }
            let adj = len - PLAYER_RADIUS;
            if adj < best {
                best = adj;
                closest = Some(i);
            }
        }
        let Some(c) = closest else { return };
        let (dmg, from) = (p.derived.damage * mult, p.pos);
        // The swing's knock: a power swing knocks down (0x20), HIT2 back.
        let flags = if mult >= 3.0 {
            DmgType::KNOCKDOWN
        } else if mult >= 2.0 {
            DmgType::KNOCKBACK
        } else {
            0
        };
        self.snd.mode = Some(2);
        self.damage_from = Some(from);
        self.damage_player(c, dmg, flags);
        self.damage_from = None;
    }

    /// PlyrSfxDoDamage for one PDATA damage row due in the frame window
    /// [t0, t1): type 10 fires the class's missiles, 0 and 5 nothing, the
    /// rest start the row's effect (PlyrSfxDoDamageSub; type 2 in the world,
    /// the others riding the hero).
    fn fire_damage_row(
        &mut self,
        pi: usize,
        row: &gdl_formats::pdata::DamageRow,
        t0: f32,
        t1: f32,
        partner: Option<usize>,
    ) {
        match row.kind {
            0 | 5 => {}
            10 => self.damage_row_missiles(pi, row, t0, t1),
            k => self.plyr_sfx_do_damage_sub(pi, row, k != 2, partner),
        }
    }

    /// PlyrSfxDoDamage's type 10, PlayerStartMissile unaimed at x2 damage
    /// without the 3/5-way spread: on a single frame, a missile along the
    /// facing turned by the row's angle; over a window, one every `delay`
    /// frames, the angle swept in (0x100) or out (0x200) across it and the
    /// direction scaled by the row's offset, falling at x`weight`.
    fn damage_row_missiles(
        &mut self,
        pi: usize,
        row: &gdl_formats::pdata::DamageRow,
        t0: f32,
        t1: f32,
    ) {
        use crate::powerups as pu;
        if t1 == t0 {
            return;
        }
        let p = &self.players[pi];
        let flags = p.fx.weapon & (pu::W_ELEMENT | pu::W_REFLECT);
        let mut damage = p.derived.missile_damage;
        if flags & pu::W_ELEMENT != 0 {
            damage *= ELEMENT_BONUS;
        }
        let hand = p.pos + Vec3::Y * (PLAYER_HEIGHT * 0.5);
        let yaw = p.yaw;
        let dir = |a: f32| Vec3::new((yaw + a).sin(), 0.0, (yaw + a).cos());
        let sf = row.startframe as f32;
        let ef = if row.endframe < 0 || row.flags & 0x80 != 0 {
            sf
        } else {
            row.endframe as f32 + 1.0
        };
        if sf == ef {
            if row.delay == 0.0 {
                self.start_missile_fan(pi, hand, dir(row.angle), -1.0, damage, flags, 2.0, 1.0);
            } else if row.delay > 0.0 {
                // A fan of 1/delay steps, lobbed 5 ahead.
                let mut acc = 0.0;
                while acc < 1.0 {
                    let d = dir(row.angle * acc) * Vec3::new(0.3, 1.5, 0.3);
                    self.start_missile_fan(pi, hand, d, 5.0, damage, flags, 2.0, 1.0);
                    acc += 1.0 / row.delay;
                }
            }
            return;
        }
        // PlyrSfxWrapTime: only on the frame each `delay` begins.
        let rate = row.delay;
        let frac = t1 - sf;
        if rate > 0.0 {
            let w = if rate.abs() > frac.abs() {
                frac
            } else {
                frac - rate * (frac / rate).trunc()
            };
            if w as i32 != 0 {
                return;
            }
        }
        let mut angle = row.angle;
        if row.flags & 0x300 != 0 {
            let mut k = if ef > sf { (t1 - sf) / (ef - sf) } else { 1.0 };
            if row.flags & 0x200 != 0 {
                k = 1.0 - k;
            }
            angle *= k;
        }
        let (d, o) = (dir(angle), row.offset);
        let d = Vec3::new(d.x * o[0], o[1], d.z * o[2]);
        self.start_missile_fan(pi, hand, d, -1.0, damage, flags, 2.0, row.weight);
    }

    /// PlyrSfxDoDamageSub: start the row's effect at its offset (in the
    /// hero's frame when `riding`, else placed in the world), then arm it. A
    /// type-2 row flies off at the mean of its speeds along the facing turned
    /// by its angle (SfxSetPhysics), falling at its weight with its hitrad;
    /// the others hurt what is within their radius (and arc) of it, each
    /// target once per the row's delay. No effect started, no damage.
    fn plyr_sfx_do_damage_sub(
        &mut self,
        pi: usize,
        row: &gdl_formats::pdata::DamageRow,
        riding: bool,
        partner: Option<usize>,
    ) {
        use crate::missile as m;
        let p = &self.players[pi];
        let (pos, yaw, model) = (p.pos, p.yaw, p.model);
        let at = if riding {
            Vec3::from(row.offset)
        } else {
            pos + m::rotate_offset(row.offset, yaw)
        };
        let Some(fx) = self.do_plyr_sfx_sub(pi, row.fxidx as i32, Some(at), riding, partner, 0)
        else {
            return;
        };
        let fx_flags = self.pdata[model]
            .sfx
            .get(row.fxidx as usize)
            .map_or(0, |r| r.flags);
        let ei = self.entities.iter().position(|e| e.id == fx.id);
        if let Some(e) = ei.map(|i| &mut self.entities[i]) {
            // PlaceEffectOnFloor, then YawMat3 by the row's angle.
            if fx_flags & 0x10 != 0 && row.hitfxidx < 0 {
                if let Some(g) = self.collision.ground_height(e.pos, e.pos.y + 1.0) {
                    e.pos.y = g;
                }
            }
            e.yaw += row.angle;
        }
        if let Some(f) = self.fx_follow.iter_mut().find(|f| f.id == fx.id) {
            f.yaw += row.angle;
        }
        let mut dmg = if row.amount < 0.0 {
            -row.amount * self.players[pi].derived.damage
        } else {
            row.amount
        };
        if dmg <= 0.0 {
            return;
        }
        // The class colour (dmgtype's element) is worth x1.5 on neutral monsters.
        if row.dmgtype & 0xF != 0 {
            dmg *= ELEMENT_BONUS;
        }
        if row.kind == 2 && row.speed_min > 0.0 {
            let spd = 0.5 * (row.speed_max - row.speed_min) + row.speed_min;
            let fwd = Vec3::new(yaw.sin(), 0.0, yaw.cos());
            // Level: the floor's slope (field_8BC) is not tracked. Flag 8's
            // PitchVec3 is set by no shipped row.
            let vel = if row.flags & 4 != 0 { fwd } else { fwd * spd };
            let vel = glam::Quat::from_rotation_y(row.angle) * vel;
            // SfxSetHit: the hit record's sound plays on walls.
            let wall_sound = (row.hitfxidx >= 0)
                .then(|| self.psfx[model].get(row.hitfxidx as usize))
                .flatten()
                .and_then(|h| h.sound.clone())
                .map_or(WallSound::None, WallSound::Sound);
            // It lasts its animation, then (SfxSetMorph) becomes the loop
            // record's effect for maxtime; walls only stop it for mintime.
            let morph = (row.loopfxidx >= 0)
                .then(|| self.psfx[model].get(row.loopfxidx as usize).cloned())
                .flatten()
                .filter(|l| l.effect)
                .map(|l| {
                    let t = if row.maxtime > 0.0 {
                        row.maxtime
                    } else {
                        self.fx_life(l.model, 0.0)
                    };
                    (t, l.model)
                });
            if let Some(e) = ei.map(|i| &mut self.entities[i]) {
                let first = match e.kind {
                    Kind::Effect { life } => life,
                    _ => PROJECTILE_LIFE,
                };
                e.kind = Kind::Projectile {
                    owner: pi,
                    vel,
                    life: first + morph.map_or(0.0, |m| m.0),
                    damage: dmg,
                    weight: row.weight.max(0.0),
                    radius: if row.hitrad >= 0.0 { row.hitrad } else { 1.0 },
                    tumble: 0.0,
                    reflect: row.dmgtype & m::DMG_REFLECT != 0,
                    pierced: (row.dmgtype & m::DMG_SUPER != 0).then(Vec::new),
                    wall_sound,
                    hold: row.mintime,
                    morph,
                };
                e.yaw = vel.x.atan2(vel.z);
                e.pitch = Self::missile_pitch(vel, 0.0, 0.0);
            }
            return;
        }
        let at = fx.pos;
        let fwd = Vec3::new(yaw.sin(), 0.0, yaw.cos());
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

    /// DoPlyrSfxSub: run plyr_sfx record `idx` and those chained after it:
    /// start its effect at its offset (turned into the hero's frame unless
    /// `riding`, which flag 1 sets) plus `offset`, play its sound, shake the
    /// camera (flag 2). Returns the first record's effect.
    fn do_plyr_sfx_sub(
        &mut self,
        pi: usize,
        idx: i32,
        offset: Option<Vec3>,
        mut riding: bool,
        partner: Option<usize>,
        depth: usize,
    ) -> Option<PlyrFx> {
        let m = self.players[pi].model;
        if idx < 0 || depth > 16 {
            return None;
        }
        let rec = self.pdata[m].sfx.get(idx as usize)?.clone();
        let res = self.psfx[m].get(idx as usize).cloned().unwrap_or_default();
        let f = rec.flags;
        let mut result = None;
        // Hiding the hero (0x400) and SetSkinFX (0x100) are not drawn;
        // particle records (0x0F000000) are PsfxDoParticle's.
        if f & 0x0F00_0300 == 0 {
            let l = if riding {
                Vec3::from(rec.offset)
            } else {
                crate::missile::rotate_offset(rec.offset, self.players[pi].yaw)
            };
            let at = if f & 1 != 0 {
                riding = true;
                l
            } else {
                offset.map_or(l, |o| o + l)
            };
            result = self.do_plyr_sfx(pi, &rec, &res, at, riding, partner);
        } else if f & 0x0F00_0000 != 0 {
            self.psfx_particle(pi, &rec, res.psys_tex);
        }
        if f & 0x0F00_0000 == 0 {
            if let Some(s) = res.sound {
                self.sfx.push(s);
            }
        }
        if f & 2 != 0 {
            self.shake.start(0, 0, 90, 0.1, 100);
        }
        // Flag 0x20 (SafeRockSetup) is set by no shipped record.
        self.do_plyr_sfx_sub(pi, rec.nextfxidx, offset, riding, partner, depth + 1);
        result
    }

    /// DoPlyrSfx: the record's effect at `at`, lasting its maxlen (else its
    /// animation): at the hero with its facing (flag 0x80), with the facing
    /// (0x40), else riding the hero's model when `riding` (the grab
    /// partner's with 0x2000), else in the world unturned.
    fn do_plyr_sfx(
        &mut self,
        pi: usize,
        rec: &gdl_formats::pdata::SfxRecord,
        res: &PlyrSfx,
        at: Vec3,
        riding: bool,
        partner: Option<usize>,
    ) -> Option<PlyrFx> {
        if !res.effect {
            return None;
        }
        let f = rec.flags;
        let owner = match partner {
            Some(o) if f & 0x2000 != 0 && o < self.players.len() => o,
            _ => pi,
        };
        let (p, o) = (&self.players[pi], &self.players[owner]);
        let in_frame = |q: &Player| q.pos + crate::missile::rotate_offset(at.into(), q.yaw);
        let (pos, yaw, follow) = if f & 0x80 != 0 {
            (p.pos, p.yaw, None)
        } else if f & 0x40 != 0 {
            (if riding { in_frame(o) } else { at }, o.yaw, None)
        } else if riding && f & 0x800 == 0 {
            (in_frame(o), o.yaw, Some(owner))
        } else {
            (at, 0.0, None)
        };
        // StartFXSub fails with every effect slot in use.
        let live = self
            .entities
            .iter()
            .filter(|e| e.alive && matches!(e.kind, Kind::Effect { .. }))
            .count();
        if live >= 48 {
            return None;
        }
        let life = self.fx_life(res.model, rec.maxlen);
        let id = self.spawn(Kind::Effect { life }, res.model, pos, yaw);
        if let Some(player) = follow {
            self.fx_follow.push(FxFollow {
                id,
                player,
                local: at,
                yaw: 0.0,
            });
        }
        Some(PlyrFx { id, pos })
    }

    /// StartFXTree's lifetime: `time`, else one play of the effect's
    /// animation (30 frames when it has none).
    fn fx_life(&self, model: Option<usize>, time: f32) -> f32 {
        if time > 0.0 {
            return time;
        }
        let seq = model.and_then(|m| self.models[m].skeleton.sequences.first());
        let n = seq.map_or(0, |s| s.frame_count);
        let rate = seq.map_or(0, |s| s.frame_rate);
        let n = if n == 0 { 30 } else { n };
        let rate = if rate > 0 { rate as f32 } else { 30.0 };
        n as f32 * rate / 900.0
    }

    /// Effects riding a hero follow it.
    fn update_fx_follow(&mut self) {
        let (players, entities) = (&self.players, &mut self.entities);
        self.fx_follow.retain(|f| {
            let Some(p) = players.get(f.player) else {
                return false;
            };
            let Some(e) = entities
                .iter_mut()
                .find(|e| e.id == f.id && e.alive && matches!(e.kind, Kind::Effect { .. }))
            else {
                return false;
            };
            e.pos = p.pos + crate::missile::rotate_offset(f.local.into(), p.yaw);
            e.yaw = p.yaw + f.yaw;
            true
        });
    }

    /// Damage entity `t` on player `pi`'s behalf: sounds, experience (more for
    /// melee, which also charges the turbo meter) and anything it drops.
    pub(crate) fn hit_entity(&mut self, pi: usize, t: usize, dmg: f32, push: Vec2, melee: bool) {
        // lbl_803447E4: a hero fighting (damage_enemy, or a swing at a
        // generator or obstacle) holds off the Follow nag.
        if matches!(
            self.entities[t].kind,
            Kind::Enemy { .. } | Kind::Critter(_) | Kind::Generator { .. }
        ) || melee && matches!(self.entities[t].kind, Kind::Container { .. })
        {
            self.nag.fighting = true;
        }
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
        let was = match &self.entities[t].kind {
            Kind::Enemy { state, .. } => Some(*state),
            _ => None,
        };
        // birth_style 0: made by a generator (not placed, nor a lookout).
        let spawned_by_gen = matches!(&self.entities[t].kind,
            Kind::Enemy { brain, ai, .. } if brain.generator.is_some() && *ai != 15);
        let death = match &self.entities[t].kind {
            Kind::Enemy {
                etype: crate::enemies::DEATH,
                hp,
                strength,
                ..
            } => Some((*hp, *strength == 2)),
            _ => None,
        };
        let r = Self::apply_hit(&mut self.entities[t], dmg, push);
        // AudioPlayEvt101: Death dies.
        if r.killed && r.tier == crate::enemies::DEATH && !r.generator {
            self.sfx_id(0x65);
        }
        // damage_enemy (enemy.c:5976): magic kills Death and heals a hero
        // above level 75 by part of what it had left; with the anti-Death
        // shield each hit gives +1 health (a tier-2 Death's: experience),
        // without it the hero hears it can't be hurt (msg 0).
        if let (Some((hp, tier2)), true) = (death, r.hit) {
            let p = &mut self.players[pi];
            if MAGIC_HIT.with(|m| m.get()) != 0 {
                if p.level > 75 {
                    let k = 0.032 * (p.level - 75) as f32 + 0.2;
                    let cap = max_health(p.level);
                    if p.health < cap {
                        p.health = (p.health + hp * k).min(cap);
                    }
                }
            } else if p.fx.armor & crate::powerups::A_ANTIDEATH != 0 {
                if tier2 {
                    self.death_exp(pi, 1);
                } else {
                    p.health += 1.0;
                }
            } else {
                self.post(0, Some(pi), &[]);
            }
        }
        // damage_enemy's burst by the hit's element (the potion's, the
        // weapon's, or what the caller dealt).
        let dtype = self.effects.hit_dtype.take().unwrap_or_else(|| {
            let magic = MAGIC_HIT.with(|m| m.get());
            if magic != 0 {
                magic | DmgType::MAGIC
            } else {
                self.players
                    .get(pi)
                    .map_or(0, |p| p.fx.weapon & crate::powerups::W_ELEMENT)
            }
        });
        if is_enemy && r.hit {
            self.enemy_hit_fx(t, dtype, r.killed);
        }
        if is_gen && r.killed {
            self.generator_fx(t, true);
        }
        if let (Some(was), true) = (was, r.hit) {
            // PlayerMotion_DamageTarget's hit direction rises with the damage.
            let lift = self.effects.hit_lift.take().unwrap_or(0.0);
            let dir = Vec3::new(push.x, lift, push.y);
            self.enemy_react(t, was, r.killed, dmg, dtype, dir);
        }
        // Generators drop a tier at 2/3 and 1/3 of their health: they then spawn
        // the lower tier, allow twice as many alive, and change model.
        let mut dropped = false;
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
                    dropped = true;
                }
            }
        }
        if dropped {
            self.generator_fx(t, false);
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
            // PlayerDamagedEnemy: ten generator monsters killed without
            // destroying a generator says to destroy them (msg 22), outside
            // boss levels; destroying one starts the count again.
            if let Some(n) = self.gen_streak.get_mut(pi) {
                if r.generator {
                    *n = 0;
                } else if spawned_by_gen {
                    *n += 1;
                    if *n >= 10 && self.boss.is_none() {
                        self.post(22, Some(pi), &[]);
                    }
                }
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
        // A barrel knocks, breaks, explodes or gasses.
        self.barrel_hit(t);
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
        // Item damage case 4: striking a statue wakes it.
        if let Some(s) = &mut c.statue {
            s.woken = true;
            self.entities[t].kind = Kind::Critter(c);
            return;
        }
        // PlayerDamagedEnemy ignores the boss until it wakes.
        if !c.hittable() || (c.is_boss() && !self.boss.as_ref().is_some_and(|b| b.active)) {
            self.entities[t].kind = Kind::Critter(c);
            return;
        }
        let (_, node) = c.surface(from, pos, yaw);
        let (part, colnode) = node.unwrap_or((0, usize::MAX));
        let element = MAGIC_HIT.with(|m| m.get());
        let magic = element != 0;
        let mut flags = if magic {
            // The potion's element, for this critter's own affinity.
            crate::critter::DMG_MAGIC | element
        } else {
            0
        };
        let mut dmg = if magic { dmg / ELEMENT_BONUS } else { dmg };
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
    /// melee exp also charges the turbo meter (outside the untouchable
    /// moves). A level-up, however many levels it is, posts msg 0x22, starts
    /// StartLevelUpFX on the hero and adds 100 health once. A combo partner
    /// gets the same exp (scaled again for them, mode -1).
    fn add_exp(&mut self, pi: usize, amount: f32, melee: bool) {
        let gained = self.add_exp_sub(pi, amount, melee);
        if let Some(g) = self.players.get(pi).and_then(|p| p.grab_partner) {
            if g != pi && g < self.players.len() {
                self.add_exp_sub(g, gained as f32, false);
            }
        }
    }

    /// AddExp for one hero (no forwarding); the exp it scaled to.
    fn add_exp_sub(&mut self, pi: usize, amount: f32, melee: bool) -> u32 {
        let (xp_mul, plevel) = self
            .wave
            .and_then(|w| self.worlds.level(w))
            .map_or((1.0, 0.0), |l| (l.xp_mul.max(0.0), l.player_level));
        let p = &mut self.players[pi];
        let gained = crate::stats::scale_exp(amount, p.level, xp_mul, plevel);
        if melee && p.act.action < 11 {
            p.power = (p.power + 0.025 * gained as f32).min(100.0);
        }
        p.exp = p.exp.saturating_add(gained);
        let level = crate::stats::exp_to_level(p.exp);
        if level > p.level && gained > 0 {
            p.health += 100.0;
            p.level = level;
            p.refresh_stats();
            self.post(0x22, Some(pi), &[level.to_string()]);
            self.start_level_up_fx(pi);
        }
        gained
    }

    /// damage_player: the level's damagemul scales each hit over 1, armour
    /// takes its share off (not poison gas or magic), and a block or charge
    /// halves it. The announcer speaks as health crosses 150 and 50, and the
    /// hero grunts in pain.
    pub fn damage_player(&mut self, pi: usize, dmg: f32, flags: u32) {
        let mode = self.snd.mode.take().unwrap_or(1);
        if self.debug_god {
            return;
        }
        let damagemul = self.damage_mul();
        // A hit with a direction (dir != NULL): where it came from.
        let directed = self.damage_from.is_some();
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
        // Unless it is a throw (0x8000): a hero waiting on an exit (state 4)
        // can't be hurt, and the level's damagemul scales any hit over 1.
        if flags & 0x8000 == 0 {
            if p.exit.waiting {
                return;
            }
            if dmg > 1.0 {
                dmg *= damagemul;
            }
        }
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
        // damage_player: a block (DEFEND2) stops only a hit with no
        // direction; the back-arc test that should stop the rest can never
        // pass as shipped (player.c:3368), so those are halved, as a charge
        // halves everything. Either way a heavy knock becomes a knockback.
        let mut clank = None;
        let mut learn_block = false;
        if dmg > 1.0 && p.act.hud & (crate::paction::hud::BLOCK | crate::paction::hud::CHARGE) != 0
        {
            let dam = dmg;
            if p.act.hud & crate::paction::hud::BLOCK != 0 && !directed {
                dmg = 0.0;
            } else {
                dmg *= 0.5;
            }
            // StartBlockFX when the block took more than 2 off (hud 0x2000:
            // this hero has blocked).
            if dam - dmg > 2.0 {
                clank = Some(dmg);
                p.act.hud |= 0x2000;
            }
            if flags & 0x10160 != 0 {
                flags = (flags & !0x10160) | DmgType::KNOCKBACK;
            }
        } else if dmg > 15.0 && flags & 0x10160 != 0 && p.act.hud & 0x2000 == 0 && self.time > 60.0
        {
            // player.c:3403: a heavy hit taken unblocked, by a hero who has
            // never blocked, a minute in: learn to block (msg 0x7D).
            learn_block = true;
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
        if dmg > 0.0 {
            p.hurt_flash = 1.0;
        }
        if let Some(reduced) = clank {
            self.start_block_fx(pi, reduced);
        }
        if learn_block {
            self.post(0x7D, Some(pi), &[]);
        }
        self.damage_sounds(pi, mode, dmg, flags, old);
    }

    /// gCurLevel->damagemul (1 without a level).
    fn damage_mul(&self) -> f32 {
        self.wave
            .and_then(|w| self.worlds.level(w))
            .map_or(1.0, |l| l.damage)
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
            // AudioGeneratorDamaged / AudioGeneratorDies (items.c:2648).
            Kind::Generator { class, .. } => {
                let etype = self.classes.get(*class).map_or(0, |c| c.etype);
                return self.generator_sound(etype, destroyed);
            }
            // Barrels and walls sound in barrel_hit.
            Kind::Container { sub, .. } if *sub != 0 => return None,
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

    /// fn_80046140's drain by Death `ei` of hero `pi`: a tier-2 Death takes
    /// experience (AddExp mode -2; msg 0x80), the rest health round armour
    /// (msg 0x82). Death pays `fight` from its own health each time, and
    /// leaves when that runs out or its victim dies (AudioPlayEvt104);
    /// otherwise S_DEATHSUCK goes on (AudioPlayEvt102Follow).
    fn death_drain(&mut self, ei: usize, pi: usize, fight: f32, tier2: bool) {
        self.nag.fighting = true;
        let mut died = false;
        if tier2 {
            self.death_exp(pi, -1);
            self.post(0x80, Some(pi), &[]);
        } else {
            let was = self.players[pi].health > 0.0;
            self.damage_player(pi, fight, DmgType::NO_ARMOR);
            died = was && self.players[pi].health <= 0.0;
            self.post(0x82, Some(pi), &[]);
        }
        let Kind::Enemy {
            hp, state, drained, ..
        } = &mut self.entities[ei].kind
        else {
            return;
        };
        if died {
            *hp = 0.0;
        }
        *hp -= fight;
        if *hp >= 0.0 {
            self.snd.death_suck = 4;
        } else {
            *drained = true;
            *state = EnemyState::Dying { timer: DEATH_EXIT };
            self.sfx_id(0x68);
        }
    }

    /// AddExp mode -2 (Death's): `sign` x 1% of the level's step, (lv - 1) x
    /// 60 + 1000 (4600 past 60); none taken at 99. Lost experience can take
    /// levels away (ModifyExp; AudioExp "lost a level"), gained can add them.
    fn death_exp(&mut self, pi: usize, sign: i32) {
        let p = &mut self.players[pi];
        if sign < 0 && p.level == 99 {
            return;
        }
        let step = if p.level <= 60 {
            (p.level as i32 - 1) * 60 + 1000
        } else {
            4600
        };
        let amount = sign * (0.01 * step as f32) as i32;
        p.exp = (p.exp as i64 + amount as i64).max(0) as u32;
        let level = crate::stats::exp_to_level(p.exp);
        if amount < 0 && level < p.level {
            p.level = level.max(1);
            p.refresh_stats();
            let name = format!("VOICE:S_{}{}1", p.color, p.class);
            self.sfx
                .push(format!("{name}+S_LOSTLEVEL|VOICE:S_LOSTLEVEL"));
        } else if amount > 0 && level > p.level {
            p.level = level.min(99);
            p.refresh_stats();
            p.health += 100.0;
            self.post(0x22, Some(pi), &[level.to_string()]);
            self.start_level_up_fx(pi);
        }
    }

    /// damage_enemy (enemy.c:6116) / uncouple_enemy: hitting an egg not yet
    /// hatched sets its generator's flags to 7 (the clutch hatches) and
    /// num_generated to 3; hitting or killing a lookout turns its generator
    /// into an ordinary one (ai 0), and a dead lookout's no longer counts.
    fn alert_generator(&mut self, t: usize, killed: bool) {
        let Kind::Enemy { brain, etype, .. } = &self.entities[t].kind else {
            return;
        };
        if *etype == crate::enemies::DEATH {
            return;
        }
        let (alg, mode1) = (brain.alg(), brain.mode1());
        let Some(gid) = brain.generator else { return };
        let Some(Kind::Generator {
            ai, chain, spawned, ..
        }) = self
            .entities
            .iter_mut()
            .find(|e| e.id == gid && e.alive)
            .map(|e| &mut e.kind)
        else {
            return;
        };
        if alg == 12 && mode1 < 2 {
            chain.flags = 7;
            chain.generated = 3;
        } else if alg == 15 {
            *ai = 0;
            if killed {
                spawned.clear();
            }
        }
    }

    /// fn_8004DC2C: monster `t`'s reaction to the hit it just took (it was in
    /// state `was`). Heavy hits (knockdown, blown away, knockover, whirlwind,
    /// or magic over 10) knock it down (E_HIT_REACT2) and push it 40 (small
    /// monsters) or 20 along the hit; a knockback hit staggers it (HIT1) and
    /// pushes it 8; the push is capped at 40. A reaction already playing isn't
    /// restarted (action.c's HIT_REACT cases wait for the end).
    pub(crate) fn enemy_react(
        &mut self,
        t: usize,
        was: EnemyState,
        killed: bool,
        dmg: f32,
        dtype: u32,
        dir: Vec3,
    ) {
        use crate::enemies::{ACID, DEATH, GOLEM, IT};
        self.alert_generator(t, killed);
        let e = &mut self.entities[t];
        let seqs = e.model.and_then(|m| self.monster_seqs.get(&m)).cloned();
        let Kind::Enemy {
            state,
            etype,
            pushed,
            pushang,
            ..
        } = &mut e.kind
        else {
            return;
        };
        let et = *etype;
        let hht = crate::enemies::TYPES[et.min(33)].height * 0.5;
        // damage_enemy returns before the reaction for Death.
        let react = if et == DEATH {
            None
        } else if dtype & 0x10160 != 0 || (dmg > 10.0 && dtype & DmgType::MAGIC != 0) {
            let scale = if et == ACID {
                0.0
            } else if hht <= 2.0 && et != IT {
                40.0
            } else if et == GOLEM {
                2.0
            } else {
                20.0
            };
            Some((true, scale))
        } else if dtype & DmgType::KNOCKBACK != 0 {
            (et != GOLEM && et != ACID).then_some((false, 8.0))
        } else {
            (et != GOLEM).then_some((false, 0.0))
        };
        if let Some((_, scale)) = react.filter(|r| r.1 > 0.0) {
            *pushed += dir * scale;
            *pushang = dir.x.atan2(dir.z) + std::f32::consts::PI;
            if *pushang > std::f32::consts::PI {
                *pushang -= std::f32::consts::TAU;
            }
            if pushed.length_squared() > 1600.0 {
                *pushed = pushed.normalize() * 40.0;
            }
        }
        if killed && et == DEATH {
            *state = EnemyState::Dying { timer: DEATH_EXIT };
            return;
        }
        let Some(seqs) = seqs else { return };
        if killed {
            let small = crate::enemies::is_small(et);
            *state = EnemyState::Dying {
                timer: if small { 0.25 } else { 0.7 },
            };
            if let Some(d) = seqs.death {
                e.animator.restart(d);
                e.animator.force_loop = false;
            }
            return;
        }
        let Some((heavy, _)) = react else {
            // No reaction: whatever it was doing goes on.
            *state = was;
            return;
        };
        let busy = match was {
            EnemyState::Hurt { heavy: h, .. } => h || !heavy,
            _ => false,
        };
        *state = EnemyState::Hurt {
            timer: 0.3,
            heavy: heavy || matches!(was, EnemyState::Hurt { heavy: true, .. }),
        };
        if busy {
            return;
        }
        // DoEnemyAction: a model without HIT2 shows READY.
        let anim = if heavy {
            seqs.hit2.or(seqs.ready)
        } else {
            seqs.hit
        };
        if let Some(h) = anim {
            e.animator.restart(h);
            e.animator.force_loop = false;
        }
    }

    /// Damage one entity (fn_8004DC2C's reaction and push: `enemy_react`).
    fn apply_hit(e: &mut Entity, dmg: f32, _push: Vec2) -> HitResult {
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
                let dmg = if *etype == crate::enemies::DEATH && MAGIC_HIT.with(|m| m.get()) == 0 {
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
                    let heavy = matches!(state, EnemyState::Hurt { heavy: true, .. });
                    *state = EnemyState::Hurt { timer: 0.3, heavy };
                }
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
            // Breakables lose the damage less their armour, at least 1 a hit
            // (fn_8005C1DC); armour -1 turns it away.
            Kind::Container {
                hp,
                drop,
                chest: false,
                armor,
                ..
            } => {
                if *armor < 0.0 || !e.alive {
                    return r;
                }
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
        self.note_special(&kind);
        let model = self
            .item_models
            .get(&info.name.to_ascii_uppercase())
            .copied()
            .flatten()
            .or_else(|| self.pickup_model(&kind));
        let id = self.spawn(
            Kind::Pickup {
                kind,
                radius: info.radius.max(1.0) + 1.0,
            },
            model,
            pos + Vec3::Y * 0.5,
            0.0,
        );
        if let Some(i) = item {
            self.items.info.insert(id, i);
        }
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
        let tcam = self.trigger_camera_on();
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
        // Item update case 5: what each hero stands on, who is in play
        // (lbl_803447E0, lbl_8034476C, lbl_80344768), and the triggers set
        // off this frame (position, flags, mover, camera, held by a hero).
        let trigger_floors = self.world_anim.floors.clone();
        let all_heroes: u32 = (0..self.players.len().min(4))
            .filter(|&i| self.players[i].state.active())
            .fold(0, |m, i| m | 1 << i);
        let joined = self.players.len();
        let in_play = self
            .players
            .iter()
            .filter(|p| !matches!(p.state, PlayerState::Dead { .. } | PlayerState::InTower))
            .count();
        type Fired = (Vec3, u16, Option<usize>, Option<(Vec3, Vec3, u8)>, bool);
        let mut trigger_fired: Vec<Fired> = Vec::new();
        let mut door_hint = false;
        let mut trap_hits: Vec<(usize, f32, u32, Vec3, i32, u64)> = Vec::new();
        let mut fallers_started: Vec<(i32, Option<(usize, usize)>)> = Vec::new();
        let mut wall_sounds: Vec<WallSound> = Vec::new();
        let mut fallers_moved: Vec<((usize, usize), Vec3)> = Vec::new();
        // Which damage tiles are on screen (off-screen ones pause their cycle).
        let hazard_vis: HashSet<u64> = self
            .entities
            .iter()
            .filter(|e| matches!(e.kind, Kind::Hazard { .. }) && self.on_screen(e.pos, 8.0))
            .map(|e| e.id)
            .collect();
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
        let boss_dying = self.boss.as_ref().is_some_and(|b| b.dying);
        let mut spawns: Vec<Spawn> = Vec::new();
        let mut shots: Vec<Shot> = Vec::new();
        let mut blasts: Vec<(Vec3, f32)> = Vec::new();
        // Death's drains: (Death, victim, its fight, tier 2).
        let mut drains: Vec<(usize, usize, f32, bool)> = Vec::new();
        // Deaths gone after draining: their message, and to whom.
        let mut death_left: Vec<(usize, Option<usize>)> = Vec::new();
        // Garm's swings (StartEnemyAtkFX) and deaths (fn_8004F1DC).
        let mut garm_atk: Vec<(u64, usize)> = Vec::new();
        let mut garm_dead: Vec<Vec3> = Vec::new();
        // Death and whom it drains (StartDeathFX on both).
        let mut drain_fx: Vec<(u64, usize)> = Vec::new();
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
        // Each pickup's food kind for snd_eat (by its model's item name).
        let mut pickup_food: Vec<usize> = Vec::new();
        let mut opened: Vec<(usize, usize)> = Vec::new();
        let mut key_used: Vec<usize> = Vec::new();
        // Each player's exit this tick (fn_8005D730 case 9), what the exits'
        // case 9 sees of the party, and the level's exit progress (reset each
        // tick until the party starts sinking).
        let mut touched: Vec<Option<Option<u32>>> = vec![None; self.players.len()];
        let party = self.party_on_exits();
        let mut exit_progress = if self.exit_state.started {
            self.exit_state.progress
        } else {
            0
        };
        let mut secret: Option<u32> = None;
        let mut chest_used: Vec<usize> = Vec::new();
        let mut chest_refused: Vec<(usize, i32)> = Vec::new();
        let mut posts: Vec<(usize, usize)> = Vec::new();
        let mut chests_opened: Vec<usize> = Vec::new();
        let mut proj_hits: Vec<(usize, usize, f32, Vec2, f32)> = Vec::new();
        // Hero missiles that burst on the world: (id, where, yaw, scale).
        let mut missile_walls: Vec<(u64, Vec3, f32, f32)> = Vec::new();

        // do_enemies: what the monsters' routines share this frame.
        self.ai_paths.update_players(&player_pos);
        let seen: Vec<crate::ai::Seen> = self
            .players
            .iter()
            .enumerate()
            .map(|(i, p)| crate::ai::Seen {
                pos: p.pos,
                active: p.state.active(),
                hidden: hidden[i],
                halo: halo[i],
                decoy: p.worn.mikey.out().then_some(p.worn.mikey.pos),
            })
            .collect();
        let ticks = (dt * 60.0).round().max(1.0) as i32;
        let ai_enemies: Vec<(usize, u64, Vec3, f32, f32)> = self
            .entities
            .iter()
            .enumerate()
            .filter_map(|(i, e)| match &e.kind {
                Kind::Enemy { etype, state, .. }
                    if e.alive && !matches!(state, EnemyState::Dying { .. }) =>
                {
                    let t = &crate::enemies::TYPES[(*etype).min(33)];
                    Some((i, e.id, e.pos, t.radius, t.height * 0.5))
                }
                _ => None,
            })
            .collect();
        let ai_solids: Vec<(Vec3, f32)> = self
            .entities
            .iter()
            .filter(|e| e.alive)
            .filter_map(|e| match e.solid? {
                Shape::Cylinder { radius } => Some((e.pos, radius)),
                Shape::Box { half } => Some((e.pos, half.x.max(half.y))),
            })
            .collect();
        // lbl_80344748: the suicide grunt running at the party.
        let bomber = self
            .entities
            .iter()
            .enumerate()
            .find_map(|(i, e)| match &e.kind {
                Kind::Enemy { brain, .. } if e.alive && visible[i] && brain.bombing() => {
                    Some(crate::ai::Bomber {
                        slot: i,
                        pos: e.pos,
                        actual_dist: brain.actual_dist,
                    })
                }
                _ => None,
            });
        let generators: HashMap<u64, bool> = self
            .entities
            .iter()
            .filter(|e| e.alive)
            .filter_map(|e| match &e.kind {
                Kind::Generator { chain, .. } => Some((e.id, chain.flags == 7)),
                _ => None,
            })
            .collect();
        let mut offsets = vec![0.0f32; self.players.len()];
        let mut releases: Vec<u64> = Vec::new();
        // Each monster's move for do_enemy_move's report: (from, tried, walls
        // stopped it, touching this player).
        let mut ai_moves: HashMap<u64, (Vec3, bool, bool, Option<Vec3>)> = HashMap::new();
        let exit_extra = self.exit_reach(0.0);

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
                    ai,
                    height,
                    ..
                } => {
                    let (ai, height) = (*ai, *height);
                    let on = visible[ei];
                    let full = enemy_count + spawns.len() >= self.max_enemies;
                    if ai == 15 {
                        // generate_single: a lookout generator keeps one
                        // lookout out, on screen or not (items.c:3917).
                        if spawned.is_empty() && !boss_dying && !full {
                            spawns.push(Spawn {
                                class: *class,
                                ai: 15,
                                strength: *strength,
                                pos,
                                yaw: e.yaw,
                                generator: e.id,
                                recycle: false,
                                height,
                            });
                        }
                    // generate_enemy refuses everything once the boss is dying.
                    } else if (*always || on) && spawned.len() < *max && !boss_dying {
                        if *timer > 0.0 {
                            *timer -= dt * 60.0;
                        } else {
                            // With every monster slot taken only a generator on
                            // screen may spawn, by recycling one off screen.
                            let full = enemy_count + spawns.len() >= self.max_enemies;
                            if !full || on {
                                // generate_enemy -2 / -3: the Temple's and Hell's
                                // lists bring their own routines.
                                let (class, spew) = match special {
                                    Some(list) => {
                                        let k = (self.special_counter as usize) & 3;
                                        let t = list[k];
                                        self.special_counter += 1;
                                        let spew = if list[0] == crate::enemies::HELL_SPECIAL[0] {
                                            [30, 30, 7, 7][k]
                                        } else {
                                            7
                                        };
                                        let c = class_of
                                            .iter()
                                            .find(|(ty, _)| *ty == t)
                                            .map_or(*class, |c| c.1);
                                        (c, spew)
                                    }
                                    None => (*class, ai),
                                };
                                spawns.push(Spawn {
                                    class,
                                    ai: spew,
                                    strength: *strength,
                                    pos,
                                    yaw: e.yaw,
                                    generator: e.id,
                                    recycle: full,
                                    height,
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
                // gTriggerCameraState: monsters stand (daction 0) while a
                // trigger camera runs.
                Kind::Enemy { state, .. } if tcam && !matches!(state, EnemyState::Dying { .. }) => {
                    if let Some(model) = e.model {
                        if let Some(s) = self.monster_seqs.get(&model).and_then(|s| s.ready) {
                            e.animator.play(s);
                            e.animator.force_loop = true;
                        }
                        e.animator.update(&self.models[model].skeleton, dt);
                    }
                }
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
                    brain,
                    ai,
                    dest,
                    pushed,
                    pushang,
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
                    if *cooldown > 0.0 && !matches!(state, EnemyState::Dying { .. }) {
                        *cooldown -= dt;
                        e.animator.update(skel, dt);
                        continue;
                    }
                    // do_enemy_move adds the knockback (enemy.c:1153); while it
                    // is pushed the monster faces back at the hit (enemy.c:1380).
                    // The push then decays x0.8 a 30 Hz frame, an upward one
                    // also by 100 a second (enemy.c:5391).
                    let lifted = *pushed != Vec3::ZERO;
                    if lifted {
                        e.pos += *pushed * dt;
                        if pushed.x * pushed.x + pushed.z * pushed.z > 0.1 {
                            e.yaw = *pushang;
                        }
                        *pushed *= 0.8f32.powf(dt * 30.0);
                        for c in [&mut pushed.x, &mut pushed.y, &mut pushed.z] {
                            if c.abs() < 0.01 {
                                *c = 0.0;
                            }
                        }
                        if pushed.y > 0.0 {
                            pushed.y = (pushed.y - 100.0 * dt).max(0.0);
                        }
                    }
                    // do_enemy_move: a step that would touch the player (radius
                    // + 0.5 + the player's 1.5; fn_80046680's sweep) requests a
                    // swing.
                    let step = *speed * dt * 2.0 + 0.01;
                    let contact = near.and_then(|(pi, _)| {
                        let pp = player_pos[pi].0;
                        let d = Vec2::new(pp.x - e.pos.x, pp.z - e.pos.z).length();
                        (d < ty.radius + 0.5 + PLAYER_RADIUS + step
                            && (pp.y - e.pos.y).abs() < ty.height)
                            .then_some(pi)
                    });
                    let play_loop = |a: &mut Animator, s: Option<usize>| {
                        if let Some(s) = s {
                            a.play(s);
                            a.force_loop = true;
                        }
                    };
                    // do_ai: the monster's routine for this frame; busy with
                    // a swing, throw, hit or its appearance it only turns.
                    let start = e.pos;
                    let mut ai_moved = false;
                    let busy = match state {
                        EnemyState::Attack { .. } | EnemyState::Recover => {
                            Some(crate::ai::act::ATTACK)
                        }
                        EnemyState::Throw { .. } => Some(crate::ai::act::THROW),
                        EnemyState::Hurt { .. } => Some(crate::ai::act::HIT_REACT1),
                        EnemyState::Spawning => Some(crate::ai::act::START),
                        _ => None,
                    };
                    let out = if matches!(state, EnemyState::Dying { .. } | EnemyState::Asleep) {
                        None
                    } else {
                        brain.recognized |= *recognized;
                        brain.adopt(*ai);
                        // place_logic12's spot: the egg is put there.
                        if let Some(d) = dest.take() {
                            e.pos = d;
                        }
                        let me = crate::ai::Me {
                            slot: ei,
                            etype: *etype,
                            pos: e.pos,
                            yaw: e.yaw,
                            rad: ty.radius,
                            hht: ty.height * 0.5,
                            spd: *speed * dt,
                            visactive: visible[ei],
                            busy,
                            reloading: *reload > 0.0 && seqs.throw.is_some(),
                            has_generator: brain
                                .generator
                                .is_some_and(|g| generators.contains_key(&g)),
                            hatch: brain
                                .generator
                                .and_then(|g| generators.get(&g).copied())
                                .unwrap_or(false),
                        };
                        let probe = crate::ai::WorldProbe {
                            coll: &self.collision,
                            enemies: &ai_enemies,
                            solids: &ai_solids,
                        };
                        let frame = crate::ai::Frame {
                            players: &seen,
                            it,
                            bomber,
                            paths: &self.ai_paths,
                            frame: self.frame,
                            ticks,
                            mrate: self.tuning.mrate,
                        };
                        let o =
                            brain.think(&me, &frame, &mut offsets, &probe, &mut self.rng, contact);
                        *recognized = brain.recognized;
                        // fn_8004D958: out of sight and off screen, nothing moves.
                        if o.frozen {
                            continue;
                        }
                        if pushed.x * pushed.x + pushed.z * pushed.z <= 0.1 {
                            e.yaw = o.yaw;
                        }
                        releases.extend(o.release);
                        Some(o)
                    };
                    match state {
                        EnemyState::Dying { timer } => {
                            // Death rises 10 a second as it fades out (alpha +4
                            // a tick), then is gone; having drained its fill
                            // it says so (enemy.c:5313).
                            if *etype == crate::enemies::DEATH {
                                *timer -= dt;
                                e.fade = (*timer / DEATH_EXIT).clamp(0.0, 1.0);
                                e.pos.y += 10.0 * dt;
                                if *timer <= 0.0 {
                                    e.alive = false;
                                    if *drained {
                                        let m = if *strength == 2 { 0x81 } else { 0x83 };
                                        death_left.push((m, target.map(|t| t.0)));
                                    }
                                }
                                continue;
                            }
                            // The death (or HIT2) animation, then gone: at once for
                            // small monsters, after the 0.7 s burn for the rest.
                            *timer -= dt;
                            if lifted {
                                e.pos = self.collision.resolve_walls(
                                    e.pos,
                                    ty.radius.min(1.5),
                                    ty.height * 0.75,
                                    STEP_HEIGHT,
                                );
                            }
                            if *timer <= 0.0 {
                                e.alive = false;
                                // kill_enemy: Garm's pup leaves its death shot.
                                if *etype == 27 {
                                    garm_dead.push(e.pos);
                                }
                            }
                            e.animator.update(skel, dt);
                            continue;
                        }
                        EnemyState::Asleep => {
                            e.animator.update(skel, dt);
                            continue;
                        }
                        EnemyState::Hurt { heavy, .. } => {
                            // E_HIT_REACT2 goes on to GETUP where the model has it.
                            if e.animator.finished {
                                match seqs.getup.filter(|&g| *heavy && e.animator.seq != g) {
                                    Some(g) => {
                                        e.animator.restart(g);
                                        e.animator.force_loop = false;
                                    }
                                    None => *state = EnemyState::Chase,
                                }
                            }
                        }
                        EnemyState::Spawning => {
                            if e.animator.finished || seqs.start.is_none() {
                                *state = EnemyState::Chase;
                            }
                        }
                        EnemyState::Idle | EnemyState::Chase => {
                            // do_ai's routine moves it and asks for its throws,
                            // casts and runs; touching a player swings at them
                            // (fn_80046140).
                            *state = EnemyState::Chase;
                            let o = out.unwrap_or_default();
                            *reload -= dt;
                            let aim = brain.closest.or(near.map(|n| n.0));
                            use crate::ai::act;
                            use crate::enemies::{DEATH, IT};
                            let alg = brain.alg();
                            let mut walk = true;
                            if *etype == IT {
                                // Tag: the touched player becomes IT.
                                if let Some(pi) = contact {
                                    tagged = Some(pi);
                                    e.alive = false;
                                    continue;
                                }
                            } else if *etype == DEATH {
                                // fn_80046140: Death drains whoever it touches
                                // every 3 ticks (health, or a tier-2 Death's
                                // experience), paying its fight from its own
                                // health each time.
                                if let Some(pi) = contact.filter(|&pi| !halo[pi]) {
                                    walk = false;
                                    // coll_pnum: whom it says it leaves.
                                    *target = Some((pi, false));
                                    drain_fx.push((e.id, pi));
                                    if *reload <= 0.0 {
                                        *reload = 3.0 / 60.0;
                                        drains.push((ei, pi, self.tuning.damage, *strength == 2));
                                    }
                                    if let Some((a, _)) = seqs.attack {
                                        if e.animator.seq != a {
                                            e.animator.restart(a);
                                        }
                                    }
                                }
                            } else if o.explode {
                                // move_logic18: the suicide grunt's run ends in a
                                // blast.
                                blasts.push((e.pos + Vec3::Y * 3.0, 50.0 * self.tuning.damage));
                                *state = EnemyState::Dying { timer: 0.1 };
                                walk = false;
                            } else if let (Some(pi), Some((swing, _)), false) =
                                (contact, seqs.attack, matches!(alg, 18 | 31))
                            {
                                // Every 8th swing is the power attack.
                                let power = *attacks & 7 == 7 && seqs.power.is_some();
                                let a = if power { seqs.power.unwrap().0 } else { swing };
                                *state = EnemyState::Attack { hit_done: false };
                                // action.c: Garm's swings carry ATK1FX / ATK2FX.
                                if *etype == 27 {
                                    garm_atk.push((e.id, power as usize));
                                }
                                *target = Some((pi, power));
                                e.animator.restart(a);
                                e.animator.force_loop = false;
                                walk = false;
                            } else {
                                match o.daction {
                                    act::THROW => {
                                        if let (Some(pi), Some((t, _))) = (aim, seqs.throw) {
                                            *state = EnemyState::Throw { fired: false };
                                            *target = Some((pi, false));
                                            e.animator.restart(t);
                                            e.animator.force_loop = false;
                                            walk = false;
                                        }
                                    }
                                    act::ATTACK | act::ATTACK2 | act::ATTACK_PWR => {
                                        // fn_8004DF58: 28-31 cast at whoever they
                                        // face when touching no one (31 swipes when
                                        // near).
                                        let cast = brain.casts()
                                            && !(alg == 31 && o.daction != act::ATTACK_PWR);
                                        let anim = if o.daction == act::ATTACK_PWR {
                                            seqs.power.or(seqs.attack)
                                        } else {
                                            seqs.attack.or(seqs.power)
                                        };
                                        if let (Some(pi), Some((a, _))) = (aim, anim) {
                                            if cast {
                                                *state = EnemyState::Throw { fired: false };
                                                *target = Some((pi, true));
                                            } else {
                                                *state = EnemyState::Attack { hit_done: false };
                                                *target = (brain.actual_dist <= 7.5)
                                                    .then_some((pi, false));
                                                if *etype == 27 {
                                                    let n = (o.daction == act::ATTACK_PWR) as usize;
                                                    garm_atk.push((e.id, n));
                                                }
                                            }
                                            e.animator.restart(a);
                                            e.animator.force_loop = false;
                                            walk = false;
                                        }
                                    }
                                    act::RUNATTACK if *reload <= 0.0 && seqs.throw.is_some() => {
                                        // Backing off it shoots as it goes.
                                        if let Some(pi) = aim {
                                            shots.push(Shot {
                                                from: e.pos,
                                                to: player_pos[pi].0 + Vec3::Y * 2.5,
                                                etype: *etype,
                                                slot: brain.missile_kind(),
                                                class: self
                                                    .classes
                                                    .iter()
                                                    .position(|c| Some(model) == c.model(*strength))
                                                    .unwrap_or(0),
                                            });
                                            *reload = brain.idle_time * self.tuning.mrate;
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            if walk {
                                let mut tr = o.trans;
                                // fn_80045C30: no stepping off a ledge (or into a
                                // pit) higher than the step allows.
                                let len = Vec2::new(tr.x, tr.z).length();
                                if len > 0.0 {
                                    let dir = Vec3::new(tr.x, 0.0, tr.z) / len;
                                    let tol = 2.0 * (0.1 + len + ty.radius);
                                    let here = self
                                        .collision
                                        .ground_height(e.pos, e.pos.y + STEP_HEIGHT)
                                        .is_some();
                                    let ahead = e.pos + dir * (ty.radius + len);
                                    let ok = self
                                        .collision
                                        .ground_height(ahead, e.pos.y + tol)
                                        .is_some_and(|g| (g - e.pos.y).abs() <= tol);
                                    if here && !ok {
                                        tr = Vec3::ZERO;
                                    }
                                }
                                e.pos += tr;
                                let moving = o.trans.length_squared() > 1e-8;
                                let running = matches!(
                                    o.daction,
                                    act::RUN | act::READYTORUN | act::RUNATTACK
                                );
                                let anim = match (moving || running, running) {
                                    (true, true) => seqs.run.or(seqs.walk),
                                    (true, false) => seqs.walk,
                                    _ => seqs.ready,
                                };
                                play_loop(&mut e.animator, anim);
                                ai_moved = o.trans.length() > 0.001;
                            }
                        }
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
                                    let slot = if cast { 2 } else { brain.missile_kind() };
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
                                // The throw cooldown (idle_secs): idle_time x the
                                // missile rate, plus the finish.
                                if !brain.casts() {
                                    *reload = brain.idle_time * self.tuning.mrate;
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
                    // Monsters collide with the world like players do; walls
                    // slide them (SlideAlongWall) unless their routine stops
                    // dead at walls instead.
                    let want = e.pos;
                    e.pos = self.collision.resolve_walls(
                        e.pos,
                        ty.radius.min(1.5),
                        ty.height * 0.75,
                        STEP_HEIGHT,
                    );
                    if ai_moved && !brain.slides() && (e.pos - want).length() > 1e-3 {
                        e.pos = self.collision.resolve_walls(
                            start,
                            ty.radius.min(1.5),
                            ty.height * 0.75,
                            STEP_HEIGHT,
                        );
                    }
                    if out.is_some() {
                        let walled = ai_moved
                            && Vec2::new(e.pos.x - start.x, e.pos.z - start.z).length() < 0.01;
                        ai_moves.insert(
                            e.id,
                            (start, ai_moved, walled, contact.map(|pi| player_pos[pi].0)),
                        );
                    }
                    if let Some(g) = self.collision.ground_height(e.pos, e.pos.y + STEP_HEIGHT) {
                        if e.pos.y - g < 3.0 {
                            // A knocked-up monster falls back at most 16 a
                            // second (EnemyFloorCollide's gravity).
                            e.pos.y = if lifted && e.pos.y > g {
                                (e.pos.y - 16.0 * dt).max(g)
                            } else {
                                g
                            };
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
                                pickup_food.push(
                                    e.model
                                        .and_then(|m| {
                                            self.item_models.iter().find(|(_, v)| **v == Some(m))
                                        })
                                        .map_or(0, |(n, _)| sounds::food_kind(n)),
                                );
                                e.alive = false;
                                break;
                            }
                            if let Some(m) = refused_message(kind) {
                                posts.push((m, pi));
                            }
                        }
                    }
                }
                // fn_8005D730 case 2: only a locked chest (active & 0x10) takes
                // a key when touched; without one it says so (msg 2, or msg
                // 23 for a silver chest when 2 is refused).
                Kind::Container {
                    hp,
                    chest: true,
                    sub,
                    ..
                } if *hp > 0.0
                    && self
                        .items
                        .info
                        .get(&e.id)
                        .and_then(|&i| self.level.worlds.item_infos.get(i))
                        .is_some_and(|i| i.active_type & 0x10 != 0) =>
                {
                    for (pi, (pp, alive)) in player_pos.iter().enumerate() {
                        let close =
                            *alive && Vec2::new(pp.x - e.pos.x, pp.z - e.pos.z).length() < 3.0;
                        if close && self.players[pi].keys == 0 {
                            chest_refused.push((pi, *sub));
                        }
                        if close
                            && self.players[pi].keys > 0
                            && !key_used.contains(&pi)
                            && !chest_used.contains(&pi)
                        {
                            chest_used.push(pi);
                            *hp = 0.0;
                            chests_opened.push(ei);
                            break;
                        }
                    }
                }
                // fn_800606FC: an opened chest runs its cycle (the lid opens
                // in ACTIVE, OPEN is action 2). At action 2 a chest with no
                // contents attached (all of them) is gone, its item left
                // standing free; a trap goes off (update_trap_chests); a gold
                // chest stays, and touching it (fn_8005D730 case 2, action 2)
                // gives its gold, gone 8 ticks later.
                Kind::Container {
                    chest: true,
                    drop,
                    sub,
                    ..
                } if self.items.chests.contains_key(&e.id) => {
                    let open = self.items.chests.get_mut(&e.id).unwrap();
                    open.cycle.update(dt, true, &mut self.rng);
                    if !open.cycle.slots.is_empty() {
                        e.animator.seq = open.cycle.anim.seq;
                        e.animator.frame = open.cycle.anim.frame;
                    }
                    if let Some(t) = &mut open.gone_in {
                        *t -= 1;
                        if *t <= 0 {
                            e.alive = false;
                        }
                    } else if open.cycle.action >= 2 {
                        match *sub {
                            items::SUB_EXPLODE => {}
                            items::SUB_GOLDCHEST => {
                                let gold = drop
                                    .and_then(|i| self.level.worlds.item_infos.get(i))
                                    .map_or(0, |i| i.value.max(0) as u32);
                                let toucher = player_pos.iter().position(|(pp, alive)| {
                                    *alive
                                        && Vec2::new(pp.x - e.pos.x, pp.z - e.pos.z).length() < 3.0
                                });
                                if let Some(pi) = toucher {
                                    pickups.push((pi, PickupKind::Gold(gold)));
                                    pickup_food.push(0);
                                    open.gone_in = Some(8);
                                    *drop = None;
                                }
                            }
                            _ => e.alive = false,
                        }
                    }
                }
                // fn_8005D730 case 7: a door opens (or says it needs a key,
                // msg 1) only for a hero not moving away from it.
                Kind::Door { radius, coll } => {
                    for (pi, (pp, alive)) in player_pos.iter().enumerate() {
                        let mv = self.players[pi].trial_move;
                        let toward = Vec2::new(mv.x, mv.z)
                            .dot(Vec2::new(e.pos.x - pp.x, e.pos.z - pp.z))
                            >= 0.0;
                        let touching = *alive && toward && (*pp - e.pos).length() < *radius;
                        if touching && self.players[pi].keys == 0 {
                            posts.push((1, pi));
                        }
                        if touching && self.players[pi].keys > 0 && !key_used.contains(&pi) {
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
                    flags,
                    cam,
                    door_switch,
                    ..
                } => {
                    use crate::triggers::tflags;
                    // A latching door switch posts the traps hint every frame.
                    door_hint |= *door_switch && *mode == TriggerMode::Latch && mover.is_some();
                    *idle -= dt;
                    // A crystal barrier only works once enough crystals are found.
                    let locked = gate.is_some_and(|c| match c {
                        101..=103 => self.garg_items[c - 101] >= 0,
                        c => !crystals_met(&self.crystals, c),
                    });
                    // Item collision case 5: a met crystal barrier reaches twice as far.
                    let reach = if gate.is_some_and(|c| c < 100) && !locked {
                        *radius * 2.0
                    } else {
                        *radius
                    };
                    // Walking into a lever or onto a pad sets it off; wall levers sit in
                    // the wall, so allow for the player's own radius.
                    let root = mover.and_then(|m| self.movers.get(m)).map(|m| m.root);
                    let objs = &self.level.worlds.world_objects;
                    let mut mask = 0u32;
                    for (pi, (pp, alive)) in player_pos.iter().enumerate() {
                        if *shoot
                            || !*alive
                            || Vec2::new(pp.x - pos.x, pp.z - pos.z).length()
                                >= reach + PLAYER_RADIUS + 0.6
                            || (pp.y - pos.y).abs() >= 4.0
                        {
                            continue;
                        }
                        // fn_8005D730 case 5: flag 0x100 counts only a hero on
                        // the target; flag 0x400's first hero hears it needs
                        // everyone (126 on a platform, else 127).
                        let floor = trigger_floors.get(pi).copied().flatten();
                        let on = root.is_none_or(|r| crate::triggers::on_target(objs, r, floor));
                        if *flags & tflags::ON_TARGET != 0 && !on {
                            continue;
                        }
                        if mask == 0 && joined > 1 && *flags & tflags::ALL_HEROES != 0 {
                            let m = if *flags & tflags::ON_TARGET != 0 {
                                126
                            } else {
                                127
                            };
                            posts.push((m, pi));
                        }
                        mask |= 1 << pi;
                    }
                    // Flag 0x400: nothing until every hero in play is on it.
                    if *flags & tflags::ALL_HEROES != 0 && mask != all_heroes {
                        mask = 0;
                    }
                    let contact = *hit || mask != 0;
                    // A locked barrier says what it needs (TowerNeedCrystalsMsg).
                    if locked && contact {
                        barrier_touches.extend(*gate);
                    }
                    let touched = !locked && contact;
                    *hit = false;
                    let mut active = None;
                    if let Some(m) = mover.and_then(|m| self.movers.get_mut(m)) {
                        m.trigger(*mode, touched, idle);
                        active = Some(m.active);
                    }
                    // Flag 0x800: an empty toggle may flip again after a
                    // second per other hero in play.
                    if !touched && *mode == TriggerMode::Toggle && *flags & tflags::SHARE_IDLE != 0
                    {
                        *idle = in_play.saturating_sub(1) as f32;
                    }
                    let on = crate::triggers::daction(*mode, touched, active, *pressed);
                    // Set off this frame: shake, ambush and the trigger camera.
                    if on && !*pressed {
                        trigger_fired.push((pos, *flags, *mover, *cam, touched));
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
                Kind::Exit {
                    radius,
                    height,
                    dest,
                    open,
                    anim,
                } => {
                    // A closed tower portal (active = -0x8000) is no longer
                    // updated: EXIT_OFF stands still.
                    if !*open {
                        continue;
                    }
                    for (pi, (pp, alive)) in player_pos.iter().enumerate() {
                        let xz = Vec2::new(pp.x - e.pos.x, pp.z - e.pos.z).length();
                        if *open
                            && *alive
                            && xz <= *radius + exit_extra
                            && (pp.y - e.pos.y).abs() <= *height
                        {
                            touched[pi] = Some(*dest);
                            if pi < 4 {
                                anim.onexit |= 1 << pi;
                            }
                        }
                    }
                    anim.update(dt, party, &mut exit_progress);
                    if e.model.is_some() {
                        e.animator.seq = anim.anim.seq;
                        e.animator.frame = anim.anim.frame;
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
                    weight,
                    radius,
                    tumble,
                    reflect,
                    wall_sound,
                    hold,
                    morph,
                    ..
                } => {
                    // ProcessEffects: move by the velocity, then fall.
                    *life -= dt;
                    *hold -= dt;
                    if let Some((t, m)) = *morph {
                        if *life <= t {
                            e.model = m;
                            *morph = None;
                        }
                    }
                    let from = e.pos;
                    let mut to = from + *vel * dt;
                    vel.y -= *weight * dt;
                    let mut gone = *life <= 0.0;
                    // A wall or floor (within half the radius) bursts it, or
                    // turns it back with DMG_REFLECT (rising at 0.4), its time
                    // cut to 10 s or by 1 s.
                    if let Some((at, n)) = self.collision.sweep(from, to, 0.5 * *radius) {
                        if *reflect {
                            to = at;
                            *vel -= 2.0 * vel.dot(n) * n;
                            if vel.y > 0.0 {
                                vel.y *= 0.4;
                            }
                            *life = if *life > 10.0 { 10.0 } else { *life - 1.0 };
                        } else if *hold > 0.0 {
                            to = at;
                        } else {
                            gone = true;
                            wall_sounds.push(wall_sound.clone());
                            missile_walls.push((e.id, at, e.yaw, e.scale));
                        }
                    }
                    if gone {
                        e.alive = false;
                    } else {
                        e.pos = to;
                        e.yaw = vel.x.atan2(vel.z);
                        e.pitch = Self::missile_pitch(*vel, *tumble, e.pitch + *tumble * dt);
                        proj_hits.push((
                            ei,
                            *owner,
                            *dmg,
                            Vec2::new(vel.x, vel.z).normalize_or_zero(),
                            *radius,
                        ));
                    }
                }
                Kind::Faller {
                    sub,
                    active,
                    vel,
                    tumble,
                    radius,
                    height,
                    tris,
                } => {
                    // fn_8005D730 via fn_8005F0F4: a hero within its cylinder
                    // sets it off (not the shot-down kind).
                    if !*active
                        && *sub != 0x34
                        && player_pos.iter().any(|(pp, alive)| {
                            *alive
                                && Vec2::new(pp.x - e.pos.x, pp.z - e.pos.z).length()
                                    <= *radius + PLAYER_RADIUS
                                && (pp.y - e.pos.y).abs() <= *height + PLAYER_HEIGHT
                        })
                    {
                        *active = true;
                        fallers_started.push((*sub, *tris));
                    }
                    if *active {
                        // 1 (sinking) or 2 units/s off the fall a 30 Hz frame;
                        // gone 200 below the level.
                        let g = if *sub == 0x31 { 1.0 } else { 2.0 };
                        vel.y -= g * dt * 30.0;
                        e.pos += *vel * dt;
                        e.spin += *tumble * dt;
                        if let Some(t) = *tris {
                            fallers_moved.push((t, *vel * dt));
                        }
                        if e.pos.y < self.level.worlds.min_bounds[1] - 200.0 {
                            e.alive = false;
                        }
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
                    flags,
                    cycle,
                } => {
                    cycle.update(dt, hazard_vis.contains(&e.id), &mut self.rng);
                    if !cycle.slots.is_empty() {
                        e.animator.seq = cycle.anim.seq;
                        e.animator.frame = cycle.anim.frame;
                    }
                    // fn_8005D730 case 8: only while the cycle is in sequence 2
                    // or 4; levitation floats over it.
                    if cycle.active & 1 != 0 && matches!(cycle.action, 2 | 4) {
                        for (pi, (pp, alive)) in player_pos.iter().enumerate() {
                            if *alive
                                && !levitating[pi]
                                && Vec2::new(pp.x - e.pos.x, pp.z - e.pos.z).length()
                                    < *radius + PLAYER_RADIUS
                                && (pp.y - e.pos.y).abs() < 4.0
                            {
                                let fwd = Vec3::new(e.yaw.sin(), 0.0, e.yaw.cos());
                                trap_hits.push((
                                    pi,
                                    *dmg,
                                    *flags,
                                    *pp + fwd,
                                    cycle.activetime,
                                    e.id,
                                ));
                            }
                        }
                    }
                }
                _ => {}
            }
            if !matches!(
                e.kind,
                Kind::Enemy { .. } | Kind::Critter(_) | Kind::Hazard { .. }
            ) && !self.items.chests.contains_key(&e.id)
                && !self.items.broken.contains_key(&e.id)
            {
                if let Some(m) = e.model {
                    e.animator.update(&self.models[m].skeleton, dt);
                }
            }
        }

        self.barrier_touches = barrier_touches;
        // (Not in the attract flyby: MODE_GROUP_ATTRACT.)
        if door_hint && self.fixed_camera.is_none() {
            self.post(5, None, &[]);
        }
        for (pos, flags, mover, cam, held) in trigger_fired {
            use crate::triggers::tflags;
            if flags & tflags::SHAKE != 0 {
                self.shake.start(0, 0, 0xB4, 0.1, 0x64);
            }
            if flags & tflags::AMBUSH != 0 {
                self.ambush(pos);
            }
            // TriggerCameraActivate(target, eye, pyr, subtype, 0x1E, 0).
            if let (Some((eye, dir, duration)), true) = (cam, held) {
                let mut c = crate::triggers::TriggerCamera::new(eye, dir, duration as i32, 0x1E, 0);
                c.target = mover;
                self.trigger_cam = Some(c);
            }
        }
        // fn_8005EE18 case 0x34: a hero's missile sets a shoot-fall rock off.
        for &(pidx, ..) in &proj_hits {
            let ppos = self.entities[pidx].pos;
            for e in self.entities.iter_mut().filter(|e| e.alive) {
                if let Kind::Faller {
                    sub: 0x34,
                    active,
                    radius,
                    height,
                    tris,
                    ..
                } = &mut e.kind
                {
                    if !*active
                        && Vec2::new(ppos.x - e.pos.x, ppos.z - e.pos.z).length() <= *radius
                        && (ppos.y - e.pos.y).abs() <= *height
                    {
                        *active = true;
                        fallers_started.push((0x34, *tris));
                    }
                }
            }
        }
        // The rocks' collision goes with them (UpdateObjWorldMat).
        for ((start, count), d) in fallers_moved {
            for t in start..(start + count).min(self.collision.tris.len()) {
                for v in &mut self.collision.tris[t].v {
                    *v += d;
                }
            }
        }
        // ProcessEffects: a missile that hit the world becomes its hit row.
        for (id, at, yaw, scale) in missile_walls {
            self.missile_burst(id, at, yaw, scale, true);
        }
        // fn_8009DB24: missiles bursting on walls.
        for snd in wall_sounds {
            match snd {
                WallSound::Sel(5) => {
                    let name = self
                        .wave
                        .and_then(|w| self.worlds.level_sound(w, true))
                        .map(str::to_string);
                    if let Some(n) = name {
                        // The sound table's names are 15 characters at most.
                        self.sfx.push(n.chars().take(15).collect());
                    }
                }
                WallSound::Sel(6) => self.sfx_id(63),
                WallSound::Sel(7) => self.sfx_id(64),
                WallSound::Sound(n) => self.sfx.push(n),
                _ => {}
            }
        }
        // fn_8009D8CC (sinking) / fn_8009D91C (falling): the realm's sound.
        for (sub, _) in fallers_started {
            let id = match self.wave {
                Some(0x601) if sub != 0x31 => 0x390000, // S_ROCKBREAKF2
                Some(0x904) if sub != 0x31 => 0x32000D, // S_ICEBREAKY
                _ if sub == 0x31 => self.realm_sound(&ROCKSINK_SFX),
                _ => self.realm_sound(&ROCKFALL_SFX),
            };
            self.sfx_id(id);
        }
        // Each hero is hurt by a tile at most once per what is left of its
        // cycle (fxhittime); a knocking tile pushes back along its facing.
        for (pi, dmg, flags, from, activetime, id) in trap_hits {
            if self.time < self.players[pi].fxhittime {
                continue;
            }
            self.players[pi].fxhittime = self.time + (activetime + 1) as f32 / 30.0;
            self.damage_from = (flags & 0x30 != 0).then_some(from);
            // Spikes (subtypes 0, 3, 4) hurt as mode 3, the rest as 2; then
            // AudioDamageTile.
            let (raw, sub) = self.snd.tile_sub.get(&id).copied().unwrap_or((-1, -1));
            self.snd.mode = Some(if matches!(raw, 0 | 3 | 4) { 3 } else { 2 });
            self.damage_player(pi, dmg, flags);
            self.damage_from = None;
            self.damage_tile_sound(sub);
            self.post(21, Some(pi), &[]);
        }

        // Projectiles hit the first damageable thing they touch; a monster
        // within the missile's radius of its cylinder (MissileCollideEnemy). A
        // super shot goes on through, hitting each thing once.
        for (pidx, owner, dmg, dir, mr) in proj_hits {
            let ppos = self.entities[pidx].pos;
            let pierced = match &self.entities[pidx].kind {
                Kind::Projectile { pierced, .. } => pierced.clone(),
                _ => None,
            };
            let target = (0..self.entities.len()).find(|&t| {
                let e = &self.entities[t];
                if pierced.as_ref().is_some_and(|p| p.contains(&e.id)) {
                    return false;
                }
                if let Kind::Critter(c) = &e.kind {
                    // A statue is struck through its item cylinder (and wakes).
                    if let Some(s) = c.statue.filter(|_| e.alive) {
                        let d = Vec2::new(s.pos.x - ppos.x, s.pos.z - ppos.z).length();
                        return d < s.radius + 0.4 && (ppos.y - s.pos.y).abs() < s.height.max(2.0);
                    }
                    if !e.alive || !c.hittable() {
                        return false;
                    }
                    let (surf, node) = c.surface(ppos, e.pos, e.yaw);
                    let t0 = &c.wad.types[c.parts[0].ty];
                    let cp = c.coll_pos(0, e.pos, e.yaw);
                    return surf < 0.4 && (node.is_some() || (ppos.y - cp.y).abs() < t0.hht + 2.0);
                }
                // fn_8005EE18: of the power-ups only potions stop missiles.
                if matches!(e.kind, Kind::Pickup { .. }) {
                    return self.missile_hits_potion(e, ppos, mr);
                }
                let flat = Vec2::new(e.pos.x - ppos.x, e.pos.z - ppos.z).length();
                if let Kind::Enemy { state, etype, .. } = &e.kind {
                    let ty = &crate::enemies::TYPES[(*etype).min(33)];
                    let hht = ty.height * 0.5;
                    return e.alive
                        && !matches!(state, EnemyState::Dying { .. })
                        && flat < mr + ty.radius
                        && (e.pos.y + hht - ppos.y).abs() < mr + hht;
                }
                let r = match &e.kind {
                    Kind::Generator { .. } => 2.2,
                    Kind::Container { chest: false, .. } => 1.6,
                    _ => return false,
                };
                e.alive && flat < r && (e.pos.y + 2.0 - ppos.y).abs() < 3.5
            });
            if let Some(t) = target {
                if matches!(self.entities[t].kind, Kind::Pickup { .. }) {
                    // fn_8005C1DC: a shot potion sets off its magic.
                    self.damage_item(t, dmg, 0, Some(owner));
                } else if owner < self.players.len() {
                    // The missile's own damage type (SfxSetDamage) goes with it.
                    let pid = self.entities[pidx].id;
                    self.effects.hit_dtype = self.effects.missiles.get(&pid).map(|m| m.1);
                    self.hit_from = Some(ppos);
                    self.hit_entity(owner, t, dmg, dir, false);
                    self.hit_from = None;
                    self.effects.hit_dtype = None;
                }
                let id = self.entities[t].id;
                match &mut self.entities[pidx].kind {
                    Kind::Projectile {
                        pierced: Some(p), ..
                    } => p.push(id),
                    _ => {
                        let e = &mut self.entities[pidx];
                        e.alive = false;
                        let (mid, yaw, scale) = (e.id, e.yaw, e.scale);
                        self.missile_burst(mid, ppos, yaw, scale, false);
                    }
                }
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
        // fn_8005D20C: what pushed it off a solid item stood there (coll_ip).
        let mut item_hit: HashMap<u64, Vec3> = HashMap::new();
        for (i, p) in moved {
            let e = &mut self.entities[i];
            if (p - e.pos).length_squared() > 1e-8 {
                let push = Vec3::new(e.pos.x - p.x, 0.0, e.pos.z - p.z).normalize_or_zero();
                item_hit.insert(e.id, p + push * 2.0);
            }
            e.pos = p;
        }

        // do_enemy_move: a monster's move is undone if it brings it into
        // another monster (their radii touching) or into a hero (radius + 0.5 +
        // the hero's 1.5); moving apart is always allowed.
        let enemy_idx: Vec<usize> = (0..self.entities.len())
            .filter(|&i| {
                self.entities[i].alive && matches!(self.entities[i].kind, Kind::Enemy { .. })
            })
            .collect();
        let mut enemy_hit: HashMap<u64, Vec3> = HashMap::new();
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
            let other = enemy_idx.iter().copied().find(|&j| {
                let (rj, hj) = size(&self.entities[j]);
                j != i && into(self.entities[j].pos, ri + rj, hi + hj)
            });
            let by_enemy = other.map(|j| self.entities[j].pos);
            let by_player = player_pos
                .iter()
                .find(|(pp, alive)| *alive && into(*pp, ri + 0.5 + PLAYER_RADIUS, hi + 2.5))
                .map(|p| p.0);
            // enemy.c:1260: a monster knocked back into another goes half the
            // way and hands it half its push.
            let knocked = match &self.entities[i].kind {
                Kind::Enemy {
                    pushed,
                    state: EnemyState::Hurt { .. } | EnemyState::Dying { .. },
                    ..
                } if pushed.x * pushed.x + pushed.z * pushed.z > 1.0 => Some(*pushed),
                _ => None,
            };
            if let (Some(j), Some(push), None) = (other, knocked, by_player) {
                self.entities[i].pos = old + (new - old) * 0.5;
                if let Kind::Enemy { pushed, .. } = &mut self.entities[j].kind {
                    *pushed += push * 0.5;
                }
            } else if by_enemy.is_some() || by_player.is_some() {
                self.entities[i].pos = old;
            }
            if let Some(b) = by_enemy {
                enemy_hit.insert(self.entities[i].id, b);
            }
        }
        // do_enemy_move's blocked responses, which steer the routines' corner
        // hugging next frame.
        for e in self.entities.iter_mut() {
            let Kind::Enemy { brain, etype, .. } = &mut e.kind else {
                continue;
            };
            let Some(&(_, tried, wall, player)) = ai_moves.get(&e.id) else {
                continue;
            };
            let report = crate::ai::Report {
                tried,
                wall,
                player,
                enemy: enemy_hit.get(&e.id).copied(),
                item: item_hit.get(&e.id).copied(),
            };
            if let Some(yaw) = brain.after_move(*etype, e.pos, report) {
                e.yaw = yaw;
            }
        }
        // move_logic13: a zombie left behind lets the rest of its line go.
        for mut id in releases {
            for _ in 0..64 {
                let Some(Kind::Enemy { brain, .. }) = self
                    .entities
                    .iter_mut()
                    .find(|e| e.id == id)
                    .map(|e| &mut e.kind)
                else {
                    break;
                };
                let prev = brain.prev;
                brain.release();
                match prev {
                    Some(p) => id = p,
                    None => break,
                }
            }
        }

        let w = self.world_letter();
        let floors = self.world_anim.floors.clone();
        for m in &mut self.movers {
            let ridden = floors.iter().any(|f| *f == Some(m.root as i32));
            let (started, stopped) =
                m.update(&mut self.collision, &mut self.world_anim, ridden, dt);
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
        // fn_8005E90C: what is in it comes out where the chest stands (SetItem
        // at its matrix; no attach node is ever set, so it is free) — except
        // a gold chest's gold, given on touch once open, and a trap's nothing.
        for t in chests_opened {
            self.sfx.push("S_CHEST".into());
            self.open_chest(t, false);
            if let Kind::Container { drop, sub, .. } = self.entities[t].kind {
                if sub != items::SUB_GOLDCHEST && sub != items::SUB_EXPLODE {
                    self.spawn_content(self.entities[t].pos, drop);
                }
            }
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
            self.nag.fighting = true;
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
                if let Some(&Kind::Enemy {
                    etype, strength, ..
                }) = self.entities.get(ei).map(|e| &e.kind)
                {
                    self.monster_blow(pi, etype, strength);
                }
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
        let realm = self.realm();
        let mut runes_found = false;
        let mut coins_got = 0;
        for ((pi, kind), food) in pickups.into_iter().zip(pickup_food) {
            if let Some((sub, count)) = sounds::got_it_of(&kind) {
                self.add_got_it(pi, sub, count);
            }
            let p = &mut self.players[pi];
            self.sfx.extend(sounds::pickup_sound(&kind, realm, pi));
            // Food: bad food's severe pain, else the eating sound.
            match kind {
                PickupKind::Food(hp) if hp < 0.0 => self.sfx.push(sounds::severe_pain(p)),
                PickupKind::Food(_) => {
                    let s = sounds::eat_food(&mut self.rng, p, food);
                    if !s.is_empty() {
                        self.sfx.push(s);
                    }
                }
                PickupKind::Shard(_) => runes_found = true,
                _ => {}
            }
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
                    coins_got += 1;
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
        // The secret realm's gold counts towards its secret character.
        if realm == Some(12) {
            for _ in 0..coins_got {
                self.secret_coin();
            }
        }
        for (m, pi) in posts {
            self.post(m, Some(pi), &[]);
        }
        for (pi, sub) in chest_refused {
            if !self.post(2, Some(pi), &[]) && sub == 48 {
                self.post(23, Some(pi), &[]);
            }
        }
        for (pi, dmg) in poisoned {
            // damage_player(-amt, 0, 2048): no cries of its own.
            self.snd.mode = Some(0);
            self.damage_player(pi, dmg, DmgType::POISON_GAS);
        }
        if runes_found {
            self.runes_found();
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
        // The suicide grunt's blast (enemy_dies: SuicideExplosion at its
        // collision point).
        for (pos, dmg) in blasts {
            self.suicide_explosion(pos, dmg);
        }
        for (ei, pi, f, tier2) in drains {
            self.death_drain(ei, pi, f, tier2);
        }
        for (m, pi) in death_left {
            self.post(m, pi, &[]);
        }
        for (id, pi) in drain_fx {
            self.note_drain(id, pi);
        }
        for (id, n) in garm_atk {
            self.start_enemy_atk_fx(n, id);
        }
        for at in garm_dead {
            self.garm_death_fx(at);
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
        self.exit_state.progress = exit_progress;
        self.update_exits(&touched);
    }

    /// Camera eye and target in game coordinates.
    pub fn camera(&self) -> (Vec3, Vec3) {
        let (eye, at, _) = self.camera_view();
        (eye, at)
    }

    /// Camera eye, target and near-plane distance.
    pub fn camera_view(&self) -> (Vec3, Vec3, f32) {
        if let Some((eye, at)) = self.fixed_camera {
            return (eye, at, 0.5);
        }
        if let Some((eye, dir)) = self.trigger_camera_view() {
            return (eye, eye + dir * 30.0, 0.5);
        }
        let (mut eye, mut at, f) = self
            .boss_camera_view()
            .unwrap_or_else(|| self.camera_view_normal());
        self.shake.apply(&mut eye, &mut at);
        (eye, at, f)
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
        // On an exit: stand still so DoExit lets the bot wait there.
        if p.exit.touch.is_some() {
            return PlayerInput::default();
        }
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
        for _ in 0..120 {
            // Toward +z (the stick is camera relative).
            let fire = PlayerInput {
                stick: g.stick_toward(Vec2::new(0.0, 0.4)),
                attack: true,
                attack_pressed: true,
                ..Default::default()
            };
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

    /// A grunt standing still 2.5 in front of a lone Warrior, and its index.
    fn grunt_in_front(g: &mut Game) -> usize {
        g.entities.clear();
        g.pending_enemies.clear();
        for _ in 0..30 {
            g.update(1.0 / 60.0, &[PlayerInput::default()]);
        }
        let p = g.players[0].pos;
        let class = g.classes.iter().position(|c| c.name == "GRU").unwrap();
        let model = g.classes[class].model(1);
        let mut kind = g.new_enemy(class, 3, EnemyState::Idle);
        if let Kind::Enemy { speed, hp, .. } = &mut kind {
            *speed = 0.0;
            *hp = 1000.0;
        }
        g.spawn(kind, model, p + Vec3::new(0.0, 0.0, 2.5), 0.0);
        g.entities.len() - 1
    }

    #[test]
    fn heavy_hits_knock_monsters_back() {
        let Some(fs) = data() else { return };
        // fn_8004DC2C: a knockdown pushes a big monster 20 along the hit and
        // knocks it down (HIT2); a knockback pushes 8 and staggers (HIT1); a
        // plain hit only staggers.
        for (dtype, heavy, push) in [
            (DmgType::KNOCKDOWN, true, 20.0),
            (DmgType::KNOCKBACK, false, 8.0),
            (0, false, 0.0),
        ] {
            let least = push / 30.0;
            let mut g = Game::new(&fs, "levelA1", &[("WAR", "RED")]).unwrap();
            let t = grunt_in_front(&mut g);
            let start = g.entities[t].pos;
            g.effects.hit_dtype = Some(dtype);
            g.hit_entity(0, t, 5.0, Vec2::Y, true);
            g.effects.hit_dtype = None;
            let Kind::Enemy { state, pushed, .. } = g.entities[t].kind else {
                panic!()
            };
            assert!(
                matches!(state, EnemyState::Hurt { heavy: h, .. } if h == heavy),
                "{dtype:x}: {state:?}"
            );
            if heavy {
                let m = g.entities[t].model.unwrap();
                let hit2 = g.monster_seqs[&m].hit2;
                assert!(hit2.is_some() && g.entities[t].animator.seq == hit2.unwrap());
            }
            assert!((pushed.z - push).abs() < 1e-3, "{dtype:x}: {pushed}");
            for _ in 0..30 {
                g.update(1.0 / 60.0, &[PlayerInput::default()]);
            }
            let moved = g.entities[t].pos.z - start.z;
            assert!(moved >= least, "{dtype:x}: moved {moved}");
            if least == 0.0 {
                assert!(moved.abs() < 0.2, "{dtype:x}: moved {moved}");
            }
            let Kind::Enemy { pushed, .. } = g.entities[t].kind else {
                panic!()
            };
            assert!(pushed.length() < 1.0, "the push decays: {pushed}");
        }
    }

    #[test]
    fn power_swing_knocks_a_monster_down() {
        let Some(fs) = data() else { return };
        let mut g = Game::new(&fs, "levelA1", &[("WAR", "RED")]).unwrap();
        let t = grunt_in_front(&mut g);
        let (pos, yaw) = (g.players[0].pos, g.players[0].yaw);
        g.player_attack(0, pos, yaw, 3.0, 0x10);
        assert!(matches!(
            g.entities[t].kind,
            Kind::Enemy {
                state: EnemyState::Hurt { heavy: true, .. },
                ..
            }
        ));
        let Kind::Enemy { pushed, .. } = g.entities[t].kind else {
            panic!()
        };
        // Along the hero's facing, rising with the damage; at most 40.
        assert!(pushed.z > 10.0 && pushed.y > 0.0 && pushed.length() <= 40.0 + 1e-3);
    }

    /// A lone Warrior on the first level with a Death class, and Death (tier
    /// `tier`) touching him, still; returns its index.
    fn death_touching(fs: &GameFs, tier: usize) -> Option<(Game, usize)> {
        for lv in [
            "levelA1", "levelA2", "levelA3", "levelB1", "levelB2", "levelC1",
        ] {
            let mut g = Game::new(fs, lv, &[("WAR", "RED")]).unwrap();
            let Some(class) = g
                .classes
                .iter()
                .position(|c| c.etype == crate::enemies::DEATH)
            else {
                continue;
            };
            g.entities.clear();
            g.pending_enemies.clear();
            for _ in 0..30 {
                g.update(1.0 / 60.0, &[PlayerInput::default()]);
            }
            let model = g.classes[class].model(1);
            let mut kind = g.new_enemy(class, tier, EnemyState::Chase);
            if let Kind::Enemy {
                speed, recognized, ..
            } = &mut kind
            {
                *speed = 0.0;
                *recognized = true;
            }
            let at = g.players[0].pos + Vec3::new(0.0, 0.0, 2.6);
            g.spawn(kind, model, at, std::f32::consts::PI);
            let t = g.entities.len() - 1;
            return Some((g, t));
        }
        None
    }

    #[test]
    fn tier_two_death_drains_experience() {
        let Some(fs) = data() else { return };
        let Some((mut g, t)) = death_touching(&fs, 2) else {
            return;
        };
        // Level 10: 1% of 9 x 60 + 1000 = 15 a drain.
        g.players[0].exp = crate::stats::level_to_exp(10) + 20;
        g.players[0].level = 10;
        let hp = g.players[0].health;
        for _ in 0..30 {
            g.update(1.0 / 60.0, &[PlayerInput::default()]);
        }
        let p = &g.players[0];
        assert!(p.level < 10, "drained a level away: {} {}", p.level, p.exp);
        assert!(p.health >= hp, "health untouched");
        assert!(g.sfx.iter().any(|s| s.contains("S_LOSTLEVEL")));
        assert!(g.sfx.iter().any(|v| v.contains("S_DEATHDRAINXP")));
        let Kind::Enemy { hp, .. } = g.entities[t].kind else {
            panic!()
        };
        assert!(hp < 100.0, "each drain costs Death its fight");
    }

    #[test]
    fn death_leaves_when_its_victim_dies_and_fades_out() {
        let Some(fs) = data() else { return };
        let Some((mut g, t)) = death_touching(&fs, 1) else {
            return;
        };
        let id = g.entities[t].id;
        g.players[0].health = 1.0;
        let mut rose = false;
        let mut faded = false;
        let y0 = g.entities[t].pos.y;
        for _ in 0..120 {
            g.update(1.0 / 60.0, &[PlayerInput::default()]);
            if let Some(e) = g.entities.iter().find(|e| e.id == id) {
                rose |= e.pos.y > y0 + 3.0;
                faded |= e.fade < 0.5 && e.fade > 0.0;
            }
        }
        assert!(rose && faded, "rises as it fades");
        assert!(
            !g.entities.iter().any(|e| e.id == id),
            "gone after its exit"
        );
        assert!(
            g.sfx.iter().any(|v| v.contains("S_DEATHDRAINS")),
            "{:?}",
            g.sfx
        );
    }

    #[test]
    fn hitting_death_with_the_shield_heals() {
        let Some(fs) = data() else { return };
        let Some((mut g, t)) = death_touching(&fs, 1) else {
            return;
        };
        g.players[0].fx.armor |= crate::powerups::A_ANTIDEATH;
        let hp = g.players[0].health;
        g.hit_entity(0, t, 20.0, Vec2::Y, true);
        assert_eq!(g.players[0].health, hp + 1.0);
        let Kind::Enemy { hp: dhp, .. } = g.entities[t].kind else {
            panic!()
        };
        assert_eq!(dhp, 99.0);
    }

    #[test]
    fn follow_nag_after_a_quiet_stretch() {
        let Some(fs) = data() else { return };
        // Only the test level T2 has milestones; without them nothing is said.
        let mut g = Game::new(&fs, "levelT2", &[("WAR", "RED")]).unwrap();
        assert!(!g.ai_paths.milestones.is_empty());
        g.entities.clear();
        g.pending_enemies.clear();
        let mut at = None;
        for f in 0..(48 * 60) {
            g.msgs.shown = None;
            g.update(1.0 / 60.0, &[PlayerInput::default()]);
            if g.sfx.iter().any(|s| s.contains("S_FOLLOW")) {
                assert!(at.is_none(), "once per quiet stretch");
                at = Some(f);
            }
            g.sfx.clear();
        }
        // 20 s in, a first (hidden) period of 10 s, then the first quiet one.
        let f = at.expect("the Follow voice");
        assert!((40 * 60 - 2..=40 * 60 + 2).contains(&f), "{f}");
    }

    #[test]
    fn hints_for_blocking_and_generators() {
        let Some(fs) = data() else { return };
        let mut g = Game::new(&fs, "levelA1", &[("WAR", "RED")]).unwrap();
        let shown = |g: &Game| g.msgs.shown.as_ref().map(|s| s.index);
        // A heavy hit taken unblocked a minute in: learn to block.
        g.time = 61.0;
        g.msgs.shown = None;
        g.damage_player(0, 30.0, DmgType::KNOCKDOWN);
        assert_eq!(shown(&g), Some(0x7D));
        // Ten generator monsters killed: destroy the generators.
        let t = grunt_in_front(&mut g);
        let gen = g.spawn(Kind::Prop, None, Vec3::ZERO, 0.0);
        g.msgs = crate::messages::Messages::new(g.msgs.text.clone());
        for k in 0..10 {
            let t = if k == 0 {
                t
            } else {
                let class = g.classes.iter().position(|c| c.name == "GRU").unwrap();
                let p = g.players[0].pos + Vec3::new(0.0, 0.0, 2.5);
                g.spawn(g.new_enemy(class, 1, EnemyState::Idle), None, p, 0.0);
                g.entities.len() - 1
            };
            if let Kind::Enemy { brain, hp, .. } = &mut g.entities[t].kind {
                brain.generator = Some(gen);
                *hp = 1.0;
            }
            assert_ne!(shown(&g), Some(22), "{k}");
            g.hit_entity(0, t, 100.0, Vec2::Y, true);
        }
        assert_eq!(g.gen_streak[0], 10);
        assert_eq!(shown(&g), Some(22));
    }

    #[test]
    fn turbo_and_wall_hints() {
        let Some(fs) = data() else { return };
        let mut g = Game::new(&fs, "levelA1", &[("WAR", "RED")]).unwrap();
        let t = grunt_in_front(&mut g);
        // Fifteen monsters on screen and a full meter: use turbo (110).
        let (class, model) = match g.entities[t].kind {
            Kind::Enemy { class, .. } => (class, g.entities[t].model),
            _ => unreachable!(),
        };
        let p = g.players[0].pos;
        for k in 0..15 {
            let a = k as f32 * 0.4;
            let mut kind = g.new_enemy(class, 1, EnemyState::Idle);
            if let Kind::Enemy { cooldown, .. } = &mut kind {
                *cooldown = 99.0;
            }
            g.spawn(kind, model, p + Vec3::new(a.sin(), 0.0, a.cos()) * 7.0, 0.0);
        }
        g.msgs = crate::messages::Messages::new(g.msgs.text.clone());
        g.players[0].power = 100.0;
        g.update(1.0 / 60.0, &[PlayerInput::default()]);
        assert_eq!(g.msgs.shown.as_ref().map(|s| s.index), Some(110));
        // A swing at a breakable wall (the nameless obstacle 0x2A, which the
        // port doesn't place yet): it takes multiple hits (20).
        let mut g = Game::new(&fs, "levelA1", &[("WAR", "RED")]).unwrap();
        g.entities.clear();
        let at = g.players[0].pos;
        g.spawn(
            Kind::Container {
                hp: 25.0,
                drop: None,
                chest: false,
                armor: 1.0,
                sub: 0x2A,
            },
            None,
            at + Vec3::Z * 2.0,
            0.0,
        );
        g.msgs = crate::messages::Messages::new(g.msgs.text.clone());
        g.player_attack(0, at, 0.0, 1.0, 0);
        assert_eq!(g.msgs.shown.as_ref().map(|s| s.index), Some(20));
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
    fn rocks_fall_when_a_hero_comes_near() {
        let Some(fs) = data() else { return };
        for (l, sub) in [("levelE2", 0x28), ("levelI5", 0x35)] {
            let mut g = Game::new(&fs, l, &[("WAR", "RED")]).unwrap();
            let rocks: Vec<(u64, Vec3, bool)> = g
                .entities
                .iter()
                .filter_map(|e| match e.kind {
                    Kind::Faller { sub: s, .. } if s == sub => {
                        Some((e.id, e.pos, e.model.is_some()))
                    }
                    _ => None,
                })
                .collect();
            assert!(
                !rocks.is_empty() && rocks.iter().all(|r| r.2),
                "{l}: rocks with models"
            );
            g.pending_enemies.clear();
            g.entities
                .retain(|e| !matches!(e.kind, Kind::Enemy { .. } | Kind::Generator { .. }));
            let (id, at, _) = rocks[0];
            g.players[0].pos = at;
            g.players[0].health = 10_000.0;
            for _ in 0..30 {
                g.players[0].pos.x = at.x;
                g.players[0].pos.z = at.z;
                g.update(1.0 / 60.0, &[PlayerInput::default()]);
            }
            let e = g.entities.iter().find(|e| e.id == id).unwrap();
            assert!(
                matches!(e.kind, Kind::Faller { active: true, .. }),
                "{l}: set off"
            );
            assert!(e.pos.y < at.y, "{l}: falling");
            // The others stay put.
            let other = g
                .entities
                .iter()
                .find(|e| e.id == rocks[rocks.len() - 1].0)
                .unwrap();
            assert!(rocks.len() == 1 || matches!(other.kind, Kind::Faller { active: false, .. }));
        }
    }

    #[test]
    fn hurting_floors_burn_once_a_second() {
        let Some(fs) = data() else { return };
        let mut g = Game::new(&fs, "levelH1", &[("WAR", "RED")]).unwrap();
        let tri = g
            .collision
            .tris
            .iter()
            .find(|t| {
                t.n.y > 0.9
                    && t.owner >= 0
                    && g.obj_flags
                        .get(t.owner as usize)
                        .is_some_and(|f| f & 0xF0000 == 0x10000)
            })
            .copied()
            .expect("a burning floor");
        let pos = (tri.v[0] + tri.v[1] + tri.v[2]) / 3.0;
        g.pending_enemies.clear();
        g.entities
            .retain(|e| !matches!(e.kind, Kind::Enemy { .. } | Kind::Generator { .. }));
        g.players[0].pos = pos + Vec3::Y * 0.05;
        g.players[0].health = 10_000.0;
        g.cam_focus = pos;
        let mut hits = 0;
        let mut last = g.players[0].health;
        for _ in 0..600 {
            g.players[0].pos.x = pos.x;
            g.players[0].pos.z = pos.z;
            g.update(1.0 / 60.0, &[PlayerInput::default()]);
            if g.players[0].health < last {
                hits += 1;
            }
            last = g.players[0].health;
        }
        println!("{hits} burns in 10 s");
        assert!((9..=11).contains(&hits), "{hits} burns in 10 s");
    }

    #[test]
    fn damage_tiles_cycle_and_hurt_only_when_up() {
        let Some(fs) = data() else { return };
        let mut checked = 0;
        for l in [
            "levelA1", "levelA3", "levelB2", "levelC2", "levelG2", "levelI3",
        ] {
            let mut g = Game::new(&fs, l, &[("WAR", "RED")]).unwrap();
            let Some((id, pos, n)) = g.entities.iter().find_map(|e| match &e.kind {
                Kind::Hazard { cycle, .. } if cycle.slots.len() >= 3 && cycle.active & 1 != 0 => {
                    Some((e.id, e.pos, cycle.slots.len()))
                }
                _ => None,
            }) else {
                continue;
            };
            g.pending_enemies.clear();
            g.entities
                .retain(|e| !matches!(e.kind, Kind::Enemy { .. } | Kind::Generator { .. }));
            // Stand on it (with the camera looking at it) for 20 s.
            g.players[0].pos = pos + Vec3::Y;
            g.players[0].health = 100_000.0;
            g.cam_focus = pos;
            let mut actions = std::collections::BTreeSet::new();
            let mut hits = 0;
            let mut last = g.players[0].health;
            for _ in 0..1200 {
                g.players[0].pos.x = pos.x;
                g.players[0].pos.z = pos.z;
                g.update(1.0 / 60.0, &[PlayerInput::default()]);
                if let Some(Kind::Hazard { cycle, .. }) =
                    g.entities.iter().find(|e| e.id == id).map(|e| &e.kind)
                {
                    actions.insert(cycle.action);
                }
                if g.players[0].health < last {
                    hits += 1;
                }
                last = g.players[0].health;
            }
            println!("{l}: {n} seqs, actions {actions:?}, {hits} hits in 20 s");
            // (A tile without the advance bit waits to be switched on.)
            if actions.len() == 1 {
                assert_eq!(hits, 0, "{l}: a resting tile doesn't hurt");
                continue;
            }
            assert!(hits < 300, "{l}: not every tick");
            checked += 1;
        }
        assert!(checked > 0);
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
        // Walk toward +z (the stick is camera relative).
        let walk = |g: &Game| PlayerInput {
            stick: g.stick_toward(Vec2::new(0.0, 1.0)),
            ..Default::default()
        };
        for _ in 0..400 {
            g.update(1.0 / 60.0, &[walk(&g)]);
        }
        assert!(
            g.players[0].pos.z < 27.0,
            "gate at z=27.9 should block, got {}",
            g.players[0].pos.z
        );
        g.players[0].keys = 1;
        for _ in 0..200 {
            g.update(1.0 / 60.0, &[walk(&g)]);
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

    /// The tower's crystal barriers (target flags 0x10|0x20) stand, solid,
    /// until their colour's crystals are found and a hero walks up; then they
    /// fade out and lose their collision (fn_80062A00: `on` = st & 0x2F).
    #[test]
    fn crystal_barriers_open_when_met() {
        let Some(fs) = data() else { return };
        let barrier = |g: &Game, c: usize| {
            let (pos, m) = g
                .entities
                .iter()
                .find_map(|e| match e.kind {
                    Kind::Trigger {
                        gate: Some(gc),
                        mover: Some(m),
                        ..
                    } if gc == c => Some((e.pos, m)),
                    _ => None,
                })
                .unwrap();
            let mv = &g.movers[m];
            let solid = g.collision.tris.iter().enumerate().any(|(i, t)| {
                t.owner >= 0 && mv.objects.contains(&(t.owner as usize)) && g.collision.enabled[i]
            });
            (pos, mv.shown, solid)
        };
        let step = |g: &mut Game, n: usize| {
            for _ in 0..n {
                g.update(1.0 / 60.0, &[PlayerInput::default()]);
            }
        };
        let mut g = Game::new(&fs, "levelL1", &[("WAR", "RED")]).unwrap();
        g.crystals[1] = CRYSTALS_NEEDED[1] - 1;
        step(&mut g, 5);
        for c in 1..=8 {
            let (_, shown, solid) = barrier(&g, c);
            assert!(shown && solid, "barrier {c} stands while locked");
        }
        // One orange short: walking up to the orange barrier leaves it up.
        let (orange, ..) = barrier(&g, 1);
        g.players[0].pos = orange + Vec3::Y * 0.5;
        step(&mut g, 2);
        assert!(g.barrier_touches.contains(&1), "it says what it needs");
        step(&mut g, 60);
        let (_, shown, solid) = barrier(&g, 1);
        assert!(shown && solid, "locked barrier stays");
        // All 15: it opens (fades out, no collision); red stays.
        g.crystals[1] = CRYSTALS_NEEDED[1];
        g.players[0].pos = orange + Vec3::Y * 0.5;
        step(&mut g, 60);
        let (_, shown, solid) = barrier(&g, 1);
        assert!(!shown && !solid, "orange barrier opens");
        let (red, shown, solid) = barrier(&g, 2);
        assert!(shown && solid, "red barrier stays");
        g.players[0].pos = red + Vec3::Y * 0.5;
        step(&mut g, 60);
        let (_, shown, solid) = barrier(&g, 2);
        assert!(shown && solid, "red barrier stays when walked into");
        assert!(!barrier(&g, 1).1, "orange stays open");
        // Entering with orange done (-1) starts it open.
        let mut g = Game::new(&fs, "levelL1", &[("WAR", "RED")]).unwrap();
        g.crystals[1] = -1;
        g.enter_tower();
        step(&mut g, 5);
        let (_, shown, solid) = barrier(&g, 1);
        assert!(!shown && !solid, "done barrier starts open");
        for c in 2..=8 {
            let (_, shown, solid) = barrier(&g, c);
            assert!(shown && solid, "barrier {c} still up");
        }
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

    /// A settled hero on levelA1 with nothing else about, and a throw of it.
    fn missile_game(fs: &GameFs, class: &str) -> (Game, Throw) {
        let mut g = Game::new(fs, "levelA1", &[(class, "RED")]).unwrap();
        g.entities.clear();
        for _ in 0..30 {
            g.update(1.0 / 60.0, &[PlayerInput::default()]);
        }
        let p = &g.players[0];
        let t = Throw {
            pi: 0,
            pos: p.pos,
            yaw: p.yaw,
            aim: Vec3::new(p.yaw.sin(), 0.0, p.yaw.cos()),
            target: false,
            bits: crate::paction::act::THROW,
            since_swing: 1.0,
        };
        (g, t)
    }

    fn missiles(g: &Game) -> Vec<(Vec3, Vec3, f32)> {
        g.entities
            .iter()
            .filter_map(|e| match &e.kind {
                Kind::Projectile { vel, .. } if e.alive => Some((e.pos, *vel, e.scale)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn missiles_bursting_on_the_world_play_the_level_hit_sound() {
        let Some(fs) = data() else { return };
        let (mut g, t) = missile_game(&fs, "WAR");
        g.player_start_missile(t);
        g.sfx.clear();
        let mut heard = false;
        for _ in 0..240 {
            g.update(1.0 / 60.0, &[PlayerInput::default()]);
            heard |= g.sfx.drain(..).any(|s| s == "S_WEAPONHITSTON");
        }
        assert!(missiles(&g).is_empty() && heard);
    }

    #[test]
    fn thrown_axe_leaves_the_hand_and_arcs_to_the_floor() {
        let Some(fs) = data() else { return };
        let (mut g, t) = missile_game(&fs, "WAR");
        let p = g.players[0].pos;
        // On an open, flat floor.
        let (lo, hi) = (p - Vec3::splat(500.0), p + Vec3::splat(500.0));
        let tri = |v: [Vec3; 3]| crate::collision::Tri {
            v,
            n: Vec3::Y,
            owner: -1,
        };
        g.collision = crate::collision::Collision::new(vec![
            tri([
                Vec3::new(lo.x, p.y, lo.z),
                Vec3::new(hi.x, p.y, lo.z),
                Vec3::new(hi.x, p.y, hi.z),
            ]),
            tri([
                Vec3::new(lo.x, p.y, lo.z),
                Vec3::new(hi.x, p.y, hi.z),
                Vec3::new(lo.x, p.y, hi.z),
            ]),
        ]);
        let yaw = t.yaw;
        g.player_start_missile(t);
        let m = missiles(&g);
        assert_eq!(m.len(), 1);
        // effectpos + the PDATA weapon_offset (-0.5, 0.5, 1.5), then 2 along
        // the aim.
        let want = p
            + Vec3::Y * 2.5
            + crate::missile::rotate_offset([-0.5, 0.5, 1.5], yaw)
            + Vec3::new(yaw.sin(), 0.0, yaw.cos()) * 2.0;
        assert!((m[0].0 - want).length() < 1e-3, "{:?} vs {want:?}", m[0].0);
        // Lobbed: rising, at the clamped missile speed across.
        let speed = g.players[0].derived.missile_speed.clamp(1.0, 100.0);
        assert!(m[0].1.y > 0.0);
        assert!((Vec2::new(m[0].1.x, m[0].1.z).length() - speed).abs() < 1e-3);
        let start = m[0].0;
        let (mut top, mut last) = (start.y, start);
        for _ in 0..180 {
            g.update(1.0 / 60.0, &[PlayerInput::default()]);
            match missiles(&g).first() {
                Some(&(pos, _v, _)) => {
                    top = top.max(pos.y);
                    last = pos;
                }
                None => break,
            }
        }
        assert!(missiles(&g).is_empty(), "bursts before its 3 s are up");
        assert!(top > start.y + 0.5, "peaks at {top}");
        // Comes down 0.5 under the hand 35 out (speedArg), then on down to the
        // floor 3 below it: 4.14 t - 6 t^2 = -2.9 at t = 1.12 s, 49 out.
        let out = Vec2::new(last.x - start.x, last.z - start.z).length();
        assert!(
            (45.0..55.0).contains(&out) && last.y < p.y + 0.5,
            "{out} out at {last:?} from {start:?}"
        );
    }

    #[test]
    fn five_way_fans_by_the_spread_table() {
        let Some(fs) = data() else { return };
        let (mut g, t) = missile_game(&fs, "ARC");
        g.players[0]
            .powerups
            .add(crate::powerups::WEAPON, crate::powerups::W_5WAY, 0.0, 45.0);
        g.players[0].powerups.slots[0].state = crate::powerups::ON;
        g.update(1.0 / 60.0, &[PlayerInput::default()]);
        let yaw = g.players[0].yaw;
        g.player_start_missile(Throw { yaw, ..t });
        let mut turns: Vec<i32> = missiles(&g)
            .iter()
            .map(|(_, v, _)| wrap_angle(v.x.atan2(v.z) - yaw).to_degrees().round() as i32)
            .collect();
        turns.sort();
        assert_eq!(turns, [-30, -15, 0, 15, 30]);
        // A fanned throw lasts 2 s, not 3.
        assert!(g.entities.iter().all(|e| match e.kind {
            Kind::Projectile { life, .. } => life == 2.0,
            _ => true,
        }));
    }

    #[test]
    fn power_a_throw_is_big_and_rebound_shots_bounce() {
        let Some(fs) = data() else { return };
        let (mut g, t) = missile_game(&fs, "WAR");
        g.player_start_missile(Throw {
            bits: crate::paction::act::PWRA_THROW,
            ..t
        });
        let e = g
            .entities
            .iter()
            .find(|e| matches!(e.kind, Kind::Projectile { .. }));
        let e = e.unwrap();
        assert_eq!(e.scale, 2.0);
        assert!(matches!(e.kind, Kind::Projectile { radius, .. } if (radius - 1.8).abs() < 1e-5));
        g.entities.clear();
        g.players[0].powerups.add(
            crate::powerups::WEAPON,
            crate::powerups::W_REFLECT,
            0.0,
            45.0,
        );
        g.players[0].powerups.slots[0].state = crate::powerups::ON;
        g.update(1.0 / 60.0, &[PlayerInput::default()]);
        let yaw = g.players[0].yaw;
        g.player_start_missile(Throw { yaw, ..t });
        let mut bounced = false;
        for _ in 0..120 {
            let before = missiles(&g);
            g.update(1.0 / 60.0, &[PlayerInput::default()]);
            let after = missiles(&g);
            if let (Some(b), Some(a)) = (before.first(), after.first()) {
                bounced |= b.1.y < 0.0 && a.1.y > 0.0;
            }
        }
        assert!(bounced, "a rebound shot comes back up off the floor");
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
    fn timed_levels_count_down_aloud() {
        let Some(fs) = data() else { return };
        let mut g = Game::new(&fs, "levelS3", &[("WAR", "RED")]).unwrap();
        // A normal level isn't timed (flags & 4).
        assert!(Game::new(&fs, "levelA1", &[("WAR", "RED")])
            .unwrap()
            .secret_timer
            .is_none());
        g.secret_timer = Some((45.99, 9.5));
        g.sfx.clear();
        let mut heard = Vec::new();
        for _ in 0..(10 * 60) {
            g.update(1.0 / 60.0, &[PlayerInput::default()]);
            heard.extend(
                g.sfx
                    .drain(..)
                    .filter(|s| s.starts_with("#") || s.contains("TIMEIS")),
            );
            if g.level_end.is_some() {
                break;
            }
        }
        // 23 (0x17) on even seconds, 24 on odd, 25 at the last; the warning
        // at 8; the tally's 5..0 (lbl_801234B8[n]).
        let want = [
            "#17",
            "S_TIMEISRUNNING",
            "#18",
            "#17",
            "#18",
            "#20006",
            "#17",
            "#20007",
            "#18",
            "#20008",
            "#17",
            "#20009",
            "#18",
            "#2000a",
            "#19",
            "#2000b",
        ];
        assert_eq!(heard, want, "{heard:?}");
        assert_eq!(g.level_end, Some(LevelEnd::SecretOver));
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
    fn monsters_run_their_routines() {
        let Some(fs) = data() else { return };
        let mut g = Game::new(&fs, "levelA1", &[("WAR", "RED")]).unwrap();
        let hp = g.players[0].health;
        let mut algs: std::collections::BTreeMap<i32, usize> = Default::default();
        let mut walked = HashSet::new();
        let mut last: HashMap<u64, Vec3> = HashMap::new();
        for _ in 0..900 {
            g.update(1.0 / 60.0, &[PlayerInput::default()]);
            for e in &g.entities {
                if let Kind::Enemy { brain, .. } = &e.kind {
                    *algs.entry(brain.alg()).or_default() += 1;
                    if last
                        .get(&e.id)
                        .is_some_and(|p| (*p - e.pos).length() > 0.05)
                    {
                        walked.insert(e.id);
                    }
                    last.insert(e.id, e.pos);
                }
            }
        }
        eprintln!(
            "algorithms {algs:?}, {} walked, hp {hp} -> {}",
            walked.len(),
            g.players[0].health
        );
        // Grunt generators chase (7); the rats drift (2 / 4).
        assert!(algs.contains_key(&7), "{algs:?}");
        assert!(algs.keys().any(|a| matches!(a, 2 | 4)), "{algs:?}");
        assert!(walked.len() >= 3);
        assert!(
            g.players[0].health < hp,
            "the monsters reached the idle hero"
        );
    }

    #[test]
    fn suicide_grunts_charge_and_blow_up() {
        let Some(fs) = data() else { return };
        for level in ["levelA3", "levelB4", "levelC4", "levelJ3", "levelT3"] {
            let mut g = Game::new(&fs, level, &[("WAR", "RED")]).unwrap();
            let Some((pos, yaw)) = g
                .pending_enemies
                .iter()
                .find(|e| matches!(e.kind, Kind::Enemy { strength: 6, .. }))
                .map(|e| (e.pos, e.yaw))
            else {
                continue;
            };
            g.players[0].pos = pos + Vec3::new(yaw.sin(), 0.0, yaw.cos()) * 15.0 + Vec3::Y;
            let hp = g.players[0].health;
            let mut ran = false;
            let mut gone = false;
            let mut seen = None;
            for _ in 0..900 {
                g.update(1.0 / 60.0, &[PlayerInput::default()]);
                let grunt = g.entities.iter().find(|e| {
                    e.alive && matches!(&e.kind, Kind::Enemy { strength: 6, brain, .. } if brain.alg() == 18)
                });
                match (grunt, seen) {
                    (Some(e), _) => {
                        seen = Some(e.id);
                        let Kind::Enemy { brain, .. } = &e.kind else {
                            unreachable!()
                        };
                        ran |= brain.bombing();
                    }
                    (None, Some(_)) => {
                        gone = true;
                        break;
                    }
                    _ => {}
                }
            }
            assert!(seen.is_some(), "{level}: no suicide grunt came into view");
            assert!(ran, "{level}: it never ran");
            assert!(gone, "{level}: it never blew up");
            assert!(g.players[0].health < hp, "{level}: the blast missed");
            eprintln!("{level}: hp {hp} -> {}", g.players[0].health);
            return;
        }
        panic!("no suicide grunt found");
    }

    #[test]
    fn tier_three_demons_cast_and_come_on() {
        let Some(fs) = data() else { return };
        let mut g = Game::new(&fs, "levelB4", &[("WAR", "RED")]).unwrap();
        g.entities
            .retain(|e| !matches!(e.kind, Kind::Enemy { .. } | Kind::Generator { .. }));
        g.pending_enemies.clear();
        for _ in 0..30 {
            g.update(1.0 / 60.0, &[PlayerInput::default()]);
        }
        let class = g.classes.iter().position(|c| c.name == "DEM").unwrap();
        // A level-3 generator's demon (gen ai 0): move_logic30.
        let kind = g.new_enemy_ai(class, 3, EnemyState::Chase, 0);
        let model = g.classes[class].model(3);
        // Somewhere 16 off in the open.
        let p = g.players[0].pos;
        let (at, yaw) = (0..16)
            .map(|k| k as f32 * std::f32::consts::TAU / 16.0)
            .map(|a| (p + Vec3::new(a.sin(), 0.0, a.cos()) * 16.0, a))
            .find(|(q, _)| {
                (1..=16).all(|s| {
                    let r = p + (*q - p) * (s as f32 / 16.0);
                    g.collision
                        .raycast(p + Vec3::Y * 2.0, r + Vec3::Y * 2.0)
                        .is_none()
                        && g.collision
                            .ground_height(r, p.y + 2.0)
                            .is_some_and(|y| (y - p.y).abs() < 1.0)
                })
            })
            .expect("open ground near the start");
        let id = g.spawn(kind, model, at, yaw + std::f32::consts::PI);
        let mut shots = 0;
        let mut closest = f32::MAX;
        for _ in 0..600 {
            g.update(1.0 / 60.0, &[PlayerInput::default()]);
            shots += g
                .entities
                .iter()
                .filter(|e| matches!(e.kind, Kind::EnemyShot { .. }))
                .count();
            if let Some(e) = g.entities.iter().find(|e| e.id == id) {
                let Kind::Enemy { brain, .. } = &e.kind else {
                    unreachable!()
                };
                assert_eq!(brain.alg(), 30);
                closest = closest.min((e.pos - g.players[0].pos).length());
            }
        }
        assert!(shots > 0, "the demon never cast");
        // Between casts it comes on (move_logic00).
        assert!(closest < 14.0, "the demon kept away ({closest})");
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

    #[test]
    fn every_hero_effect_record_resolves() {
        let Some(fs) = data() else { return };
        for class in [
            "WAR", "VAL", "WIZ", "ARC", "DWF", "KNI", "SOR", "JES", "MIN", "JAC",
        ] {
            let g = Game::new(&fs, "levelA1", &[(class, "BLU")]).unwrap();
            let missing: Vec<&str> = g.pdata[0]
                .sfx
                .iter()
                .zip(&g.psfx[0])
                .filter(|(r, s)| r.flags & 0x0F00_0100 == 0 && !r.fxdesc.is_empty() && !s.effect)
                .map(|(r, _)| r.fxdesc.as_str())
                .collect();
            assert!(missing.is_empty(), "{class}: {missing:?}");
            // NULLFX (WEAPONS) is found but has nothing to draw; the power
            // B ring has a model.
            let named = |n: &str| {
                let i = g.pdata[0].sfx.iter().position(|r| r.fxdesc == n);
                i.map(|i| &g.psfx[0][i])
            };
            assert!(named("NULLFX").is_some_and(|s| s.effect));
            let b = g.pdata[0].rows.turbo_b as usize;
            let fx = g.pdata[0].damage[b].fxidx as usize;
            assert!(g.psfx[0][fx].model.is_some(), "{class} power B model");
            let sounds: Vec<String> = g.psfx[0].iter().filter_map(|s| s.sound.clone()).collect();
            for (req, found) in crate::audio::check_sounds(&fs, &sounds) {
                assert!(found.is_some(), "{class}: {req} did not resolve");
            }
        }
    }

    #[test]
    fn power_b_starts_its_effect_sound_and_shake() {
        let Some(fs) = data() else { return };
        let (mut g, _) = missile_game(&fs, "WIZ");
        let row = g.pdata[0].damage[g.pdata[0].rows.turbo_b as usize].clone();
        g.sfx.clear();
        // (Not the level's STARTFX.)
        g.entities
            .retain(|e| !matches!(e.kind, Kind::Effect { .. }));
        g.fire_damage_row(0, &row, -1.0, 0.0, None);
        // WIZ_POWERB (flags 6: shake) chains to the wrist particles (sound
        // field = node, not played).
        assert_eq!(g.sfx, vec!["S_WIZTURBOB".to_string()]);
        assert!(g.shake.shaking);
        let fx: Vec<&Entity> = g
            .entities
            .iter()
            .filter(|e| matches!(e.kind, Kind::Effect { .. }))
            .collect();
        assert_eq!(fx.len(), 1);
        assert!(fx[0].model.is_some());
        // It rides the wizard.
        g.players[0].pos += Vec3::X * 3.0;
        g.update_fx_follow();
        let id = fx_id(&g);
        let e = g.entities.iter().find(|e| e.id == id).unwrap();
        assert!((e.pos - g.players[0].pos).length() < 1e-4);
    }

    fn fx_id(g: &Game) -> u64 {
        g.entities
            .iter()
            .find(|e| matches!(e.kind, Kind::Effect { .. }))
            .map(|e| e.id)
            .unwrap()
    }

    #[test]
    fn rows_without_an_effect_do_no_damage() {
        let Some(fs) = data() else { return };
        let (mut g, _) = missile_game(&fs, "WAR");
        // WAR dmg10 (combo): fx -1, 10 damage; a grunt in reach is untouched.
        let mut row = g.pdata[0].damage[10].clone();
        assert_eq!(row.fxidx, -1);
        row.radius = 50.0;
        let class = g.classes.iter().position(|c| c.name == "GRU").unwrap();
        let kind = g.new_enemy(class, 1, EnemyState::Idle);
        let model = g.classes[class].model(1);
        let p = g.players[0].pos;
        g.spawn(kind, model, p + Vec3::X * 2.0, 0.0);
        g.fire_damage_row(0, &row, -1.0, 0.0, None);
        assert!(g.damg_cooldown.is_empty());
        // The 360 (NULLFX, radius 10) does hit it.
        let row = g.pdata[0].damage[g.pdata[0].rows.turbo_a_360 as usize].clone();
        g.fire_damage_row(0, &row, 0.0, 1.0, None);
        assert_eq!(g.damg_cooldown.len(), 1);
    }

    #[test]
    fn turbo_c_wave_flies_at_the_row_speed() {
        let Some(fs) = data() else { return };
        let (mut g, _) = missile_game(&fs, "WAR");
        let row = g.pdata[0].damage[g.pdata[0].rows.turbo_c2 as usize].clone();
        assert_eq!(row.kind, 2);
        let (p, yaw) = (g.players[0].pos, g.players[0].yaw);
        g.fire_damage_row(0, &row, 8.0, 9.0, None);
        let e = g
            .entities
            .iter()
            .find(|e| matches!(e.kind, Kind::Projectile { .. }))
            .unwrap();
        let Kind::Projectile {
            vel,
            radius,
            pierced,
            wall_sound,
            ..
        } = &e.kind
        else {
            unreachable!()
        };
        // 30 along the facing, from (0, 1, 5) in the hero's frame; a super
        // (pass-through) shot of radius hitrad 10 with no hit record.
        let fwd = Vec3::new(yaw.sin(), 0.0, yaw.cos());
        assert!((*vel - fwd * 30.0).length() < 1e-3);
        assert!((e.pos - (p + Vec3::Y + fwd * 5.0)).length() < 1e-3);
        assert_eq!((*radius, pierced.is_some()), (10.0, true));
        assert_eq!(*wall_sound, WallSound::None);
        assert!(e.model.is_some(), "WAR_PWRC3A");
        // Walls only stop it for mintime 6; after its animation it becomes
        // WAR_PWRC3B for maxtime 6.
        let Kind::Projectile { hold, morph, .. } = &e.kind else {
            unreachable!()
        };
        assert_eq!(*hold, 6.0);
        assert!(matches!(morph, Some((t, Some(_))) if *t == 6.0));
    }

    #[test]
    fn victory_row_plays_the_victory_sound() {
        let Some(fs) = data() else { return };
        let (mut g, _) = missile_game(&fs, "WAR");
        let row = g.pdata[0].damage[g.pdata[0].rows.victory as usize].clone();
        g.sfx.clear();
        g.fire_damage_row(0, &row, 9.0, 10.0, None);
        assert_eq!(g.sfx, vec!["S_WARVICTORY".to_string()]);
    }

    #[test]
    fn archer_turbo_fan_fires_every_delay_frames() {
        let Some(fs) = data() else { return };
        let (mut g, _) = missile_game(&fs, "ARC");
        // dmg7: type 10, frames 16..26, every 2.25 frames, swept in from 90
        // degrees left (0x200).
        let row = g.pdata[0].damage[7].clone();
        assert_eq!((row.kind, row.startframe, row.endframe), (10, 16, 26));
        let yaw = g.players[0].yaw;
        let mut dirs = Vec::new();
        for f in 16..27 {
            let before = missiles(&g).len();
            g.fire_damage_row(0, &row, f as f32 - 1.0, f as f32, None);
            for m in &missiles(&g)[before..] {
                dirs.push(m.1.normalize());
            }
        }
        // PlyrSfxWrapTime: frames 16, 19, 21, 23 and 25; the angle shrinks
        // to the facing.
        assert_eq!(dirs.len(), 5);
        let fwd = Vec3::new(yaw.sin(), 0.0, yaw.cos());
        assert!(dirs[0].dot(fwd).abs() < 0.05, "first goes out sideways");
        assert!(dirs.last().unwrap().dot(fwd) > 0.9, "last nearly ahead");
    }

    #[test]
    fn turbo_moves_play_their_records() {
        let Some(fs) = data() else { return };
        for class in ["WAR", "VAL", "WIZ", "ARC", "DWF", "KNI", "SOR", "JES"] {
            let mut g = Game::new(&fs, "levelA1", &[(class, "RED")]).unwrap();
            let mut heard = Vec::new();
            for f in 0..900 {
                g.players[0].power = 100.0;
                let inp = PlayerInput {
                    turbo: true,
                    attack: f % 60 < 5,
                    attack_pressed: f % 60 == 0,
                    slow: (f + 30) % 60 < 5,
                    slow_pressed: f % 60 == 30,
                    ..Default::default()
                };
                g.update(1.0 / 60.0, &[inp]);
                heard.extend(g.sfx.drain(..).filter(|s| s.contains("TURBO")));
            }
            assert!(!heard.is_empty(), "{class}: no turbo sounds");
        }
    }

    /// SetItem case 4: Death of strength 0 or 2 waits as DEATHSTATUE1/2 (1
    /// and 3 wake at once) until a hero bumps it.
    #[test]
    fn death_waits_as_a_statue_until_bumped() {
        let Some(fs) = data() else { return };
        let mut g = Game::new(&fs, "levelA1", &[("WAR", "RED")]).unwrap();
        let i = g
            .pending_enemies
            .iter()
            .position(|p| p.statue.is_some_and(|s| !s.woken))
            .expect("a Death statue");
        let s = g.pending_enemies[i].statue.unwrap();
        assert!(s.prop.is_some(), "DEATHSTATUE stands there");
        let waiting = |g: &Game| {
            g.pending_enemies
                .iter()
                .any(|p| p.statue.is_some_and(|t| t.pos == s.pos && !t.woken))
        };
        g.players[0].pos = s.pos + Vec3::new(s.radius + 6.0, 0.0, 0.0);
        g.players[0].health = 10_000.0;
        for _ in 0..300 {
            g.update(1.0 / 60.0, &[PlayerInput::default()]);
        }
        assert!(waiting(&g), "still a statue beside a hero");
        g.players[0].pos = s.pos;
        g.update(1.0 / 60.0, &[PlayerInput::default()]);
        assert!(!waiting(&g), "bumped awake");
        // In view, Death takes the statue's place.
        let death = |e: &Entity| matches!(e.kind, Kind::Enemy { etype, .. } if etype == crate::enemies::DEATH);
        let mut made = false;
        for _ in 0..240 {
            g.players[0].pos = s.pos;
            g.update(1.0 / 60.0, &[PlayerInput::default()]);
            made |= g.entities.iter().any(death);
        }
        assert!(made, "Death made");
        assert!(
            g.entities.iter().all(|e| Some(e.id) != s.prop),
            "statue gone"
        );
    }

    /// levelA1 with only a still grunt `ahead` of the hero (who faces +z).
    fn hero_and_grunt(fs: &GameFs, ahead: Vec3) -> Game {
        let mut g = Game::new(fs, "levelA1", &[("WAR", "RED")]).unwrap();
        g.entities.clear();
        g.pending_enemies.clear();
        for _ in 0..30 {
            g.update(1.0 / 60.0, &[PlayerInput::default()]);
        }
        let p = g.players[0].pos;
        let class = g.classes.iter().position(|c| c.name == "GRU").unwrap();
        let model = g.classes[class].model(1);
        let mut kind = g.new_enemy(class, 3, EnemyState::Idle);
        if let Kind::Enemy {
            cooldown,
            speed,
            hp,
            ..
        } = &mut kind
        {
            *cooldown = 99.0;
            *speed = 0.0;
            *hp = 1e6;
        }
        g.spawn(kind, model, p + ahead, 0.0);
        g
    }

    /// Auto Attack (pmotion.c:1412, :1707): walking into a monster stops the
    /// hero and swings at it; with it off the hero slides by, no swing.
    #[test]
    fn auto_attack_swings_at_what_the_hero_walks_into() {
        let Some(fs) = data() else { return };
        use crate::paction::p as pa;
        for auto in [true, false] {
            let mut g = hero_and_grunt(&fs, Vec3::new(0.3, 0.0, 4.0));
            g.controls[0].auto_attack = auto;
            let walk = PlayerInput {
                stick: g.stick_toward(Vec2::new(0.0, 1.0)),
                ..Default::default()
            };
            let mut swung = false;
            let mut passed = false;
            let grunt = g
                .entities
                .iter()
                .find(|e| matches!(e.kind, Kind::Enemy { .. }))
                .unwrap()
                .pos;
            for _ in 0..90 {
                g.update(1.0 / 60.0, &[walk]);
                let cur = g.players[0].act.cur;
                swung |= matches!(cur, pa::QUICK | pa::KICK | pa::STEP);
                passed |= g.players[0].pos.z > grunt.z + 0.5;
            }
            assert_eq!(swung, auto, "auto attack {auto}: swung");
            assert_eq!(passed, !auto, "auto attack {auto}: walked past");
        }
    }

    /// Robotron (control style 2): the C-stick turns the hero and attacks
    /// that way while the move stick walks.
    #[test]
    fn robotron_style_aims_with_the_c_stick() {
        let Some(fs) = data() else { return };
        let mut g = hero_and_grunt(&fs, Vec3::new(0.0, 0.0, 30.0));
        let fire = PlayerInput {
            fire: g.stick_toward(Vec2::new(1.0, 0.0)),
            ..Default::default()
        };
        // The Default style ignores the C-stick.
        for _ in 0..20 {
            g.update(1.0 / 60.0, &[fire]);
        }
        assert!(g.players[0].yaw.abs() < 0.1, "default: still facing +z");
        g.controls[0].control_style = crate::input::TWIN_STICK;
        let mut swung = false;
        for _ in 0..30 {
            g.update(1.0 / 60.0, &[fire]);
            swung |= g.players[0].act.motion == 0x0F;
        }
        assert!(
            (wrap_angle(g.players[0].yaw - std::f32::consts::FRAC_PI_2)).abs() < 0.1,
            "faces +x"
        );
        assert!(swung, "fires");
        // Walking -z while firing +x: a strafing attack, moving backwards.
        let z0 = g.players[0].pos.z;
        let both = PlayerInput {
            stick: g.stick_toward(Vec2::new(0.0, -1.0)),
            ..fire
        };
        let mut strafed = false;
        for _ in 0..30 {
            g.update(1.0 / 60.0, &[both]);
            strafed |= (0x11..=0x14).contains(&g.players[0].act.motion);
        }
        assert!(strafed, "strafing attack");
        assert!(g.players[0].pos.z < z0 - 1.0, "walked with the move stick");
        assert!(
            (wrap_angle(g.players[0].yaw - std::f32::consts::FRAC_PI_2)).abs() < 0.2,
            "kept aiming +x"
        );
    }

    /// Auto Aim (pmotion.c:2483): a standing swing turns to its target; off,
    /// the hero swings where it faces.
    #[test]
    fn auto_aim_turns_a_standing_swing_to_its_target() {
        let Some(fs) = data() else { return };
        for aim in [true, false] {
            // A grunt ahead and a little to the right.
            let mut g = hero_and_grunt(&fs, Vec3::new(1.2, 0.0, 2.0));
            g.controls[0].auto_aim = aim;
            let atk = PlayerInput {
                attack: true,
                attack_pressed: true,
                ..Default::default()
            };
            for _ in 0..10 {
                g.update(1.0 / 60.0, &[atk]);
            }
            let turned = g.players[0].yaw > 0.2;
            assert_eq!(turned, aim, "auto aim {aim}");
        }
    }
}
