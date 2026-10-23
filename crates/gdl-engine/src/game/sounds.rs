//! Event sounds from sounds_evt.c / sounds.c (by the decomp's tables and ids),
//! the hero-side feedback that goes with them: the heartbeat (do_weakening),
//! damage_player's pain and rumble (do_vibe), the hit-streak cheers, the
//! got-it ticker (add_got_it / do_got_it), X-ray (do_see_thru) and the
//! compass (StartCompass / fn_80052134).

use super::*;

/// e_p_type order (char_type): the hero sound banks 4..11.
const CT: [&str; 8] = ["WAR", "VAL", "WIZ", "ARC", "DWF", "KNI", "SOR", "JES"];

/// Pojo (p->flags & 0x400) stands in for every hero sound: S_POJOTURBO,
/// S_POJOPAIN, S_POJOEATSFX, S_POJOPOISON.
const POJO: u32 = 0x400;
const S_POJOPAIN: i32 = 0x60;
const S_POJOEATSFX: i32 = 0x61;
const S_POJOPOISON: i32 = 0x62;

/// lbl_801237BC: AudioPlayerHit's rows (every column the same): S_PLYRDMG,
/// S_PLYRDMG2, S_PLYRDMG3, S_PLYRDMG4, S_PLYRDMG5.
const PLAYER_HIT: [i32; 5] = [0x2F, 0x31, 0x33, 0x35, 0x36];

/// lbl_8012382C: AudioDamageTile by realm and tile subtype.
const DAMAGE_TILE: [[&str; 7]; 13] = [
    ["", "", "", "", "", "", ""],
    [
        "S_SPIKEA",
        "",
        "S_FFIELDZAPA",
        "S_BUZZSAW",
        "S_TENTACLES",
        "S_TENTACLES",
        "",
    ],
    ["", "S_FIREHOLE", "", "", "", "", ""],
    [
        "S_SPIKEC",
        "S_FIREHOLEC",
        "S_FFIELDZAPC",
        "",
        "",
        "",
        "S_SPIKEGATE",
    ],
    [
        "S_LOGSPIKE",
        "S_FIREHOLED",
        "",
        "S_SWINGBLADE",
        "S_TENTACLESD",
        "S_TENTACLESD",
        "",
    ],
    ["", "", "", "", "", "", ""],
    ["", "S_FIREHOLEF", "", "", "", "", ""],
    ["", "S_FIREHOLEG", "", "", "", "", ""],
    ["", "S_FIREHOLEH", "", "", "", "", ""],
    ["", "S_FIREHOLEI", "", "", "", "", ""],
    ["", "S_FIREHOLEJ", "", "", "", "", ""],
    ["", "S_FIREHOLEK", "", "", "", "", ""],
    ["", "", "", "", "", "", ""],
];

/// lbl_80124330 / lbl_80124340: Skorne's taunts when the party leaves a
/// level's legend rune / runestone behind (fn_8009C3EC / fn_8009C378), each
/// in turn.
const LEFT_RUNE: [&str; 4] = [
    "S_SKORNTAUNT3",
    "S_QUIK2LEAVE",
    "S_SKORNTAUNT4",
    "S_FRGTLGND",
];
const LEFT_SHARD: [&str; 4] = ["S_SKORNTAUNT1", "S_TOOHASTY", "S_SKORNTAUNT2", "S_SRCHBTTR"];

/// lbl_8011AE8C: do_vibe's strengths.
pub const VIBE: [f32; 5] = [0.2, 0.4, 0.6, 0.8, 1.0];

/// The got-it ticker's slots (got_it[24]).
const GOT_IT_SLOTS: usize = 24;

/// One got-it ticker entry: a pickup's card sliding up over its player's panel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GotIt {
    /// 1 new, 2 sliding up, 3 holding, 4 sliding down.
    pub state: u8,
    pub player: usize,
    /// The item subtype (1 gold .. 16 gargoyle item) and its count.
    pub kind: u8,
    pub count: i32,
    /// The S3 strip's top (the icon is 16 below it).
    pub y: i32,
    timer: i32,
}

impl GotIt {
    /// do_got_it case 1: the icon blit drawn under the S3 strip.
    pub fn icon(&self, tower: bool) -> Option<&'static str> {
        Some(match self.kind {
            2 if self.count > 1 => "KEY_RING",
            2 => "KEY",
            3 if self.count >= 100 => "MEAT",
            3 if self.count <= -100 => "BADMEAT",
            3 if self.count >= 0 => "FRUIT",
            3 => "BADFRUIT",
            4 => "MAGIC",
            1 if tower => "COINHUD",
            1 if self.count > 10 => "GOLD",
            1 => "JUNK",
            5..=9 => "SPECIALS",
            10 => "RUNESTONE",
            15 => "CRYSTAL",
            16 => "GOLDNICON",
            13 => "LEGEND",
            _ => return None,
        })
    }
}

/// A rumble for the input layer: do_vibe(player, strength, ticks).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rumble {
    pub player: usize,
    /// Index into [`VIBE`].
    pub strength: usize,
    pub ticks: u32,
}

/// do_see_thru: the chest a hero's X-ray looks into and what it holds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct XRay {
    /// The chest entity (drawn see-through: MBTreeSetAlpha 0xC0).
    pub chest: u64,
    /// The content's model, drawn at the chest at 0.65 scale.
    pub model: usize,
    pub pos: Vec3,
    pub yaw: f32,
}

/// Per-hero sound timers.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct HeroSound {
    /// heartbeat_timer (ticks).
    heartbeat: i32,
    /// pain_accum: damage since the last pain cry.
    pain: f32,
    /// timer_1F0: ticks until the next AudioPlayerHit thud.
    thud: i32,
    /// fall_frames / fall_time: the hit streak and when it last grew.
    streak: i32,
    streak_time: f32,
}

#[derive(Default, Clone)]
pub struct SoundState {
    pub(crate) heroes: [HeroSound; 4],
    /// damage_player's mode for the next call (2 tile / thrown, 3 poison
    /// tile, 0 no cries); None is 1.
    pub(crate) mode: Option<u8>,
    pub got_it: Vec<GotIt>,
    /// Rumbles started this frame, for the input layer.
    pub rumble: Vec<Rumble>,
    pub xray: [Option<XRay>; 4],
    /// The COMPASS model and whether to show it (the Compass option, off
    /// while the options are open).
    pub compass_model: Option<usize>,
    pub show_compass: bool,
    /// Ticks Death's drain keeps S_DEATHSUCK on (AudioPlayEvt102Follow; it
    /// drains every third tick); at 0 it stops (AudioPlayEvt102).
    pub(crate) death_suck: i32,
    /// lbl_803447B4: the first exit's sounds have played.
    exit_heard: bool,
    /// sSpecialItem13 / sSpecialItem10: the level's legend rune (its value)
    /// and runestone (its shard bit).
    pub(crate) special_rune: Option<u32>,
    pub(crate) special_shard: Option<u32>,
    /// sVoiceRotIdxB / sVoiceRotIdxA.
    taunt_rune: usize,
    taunt_shard: usize,
    /// Damage tiles' subtypes (rawSubtype, FLAMEH in realm 3 as 6), by entity.
    pub(crate) tile_sub: BTreeMap<u64, (i32, i32)>,
}

/// The id ambient loop for Death's drain (AudioPlayEvt102Follow).
pub const DEATH_SUCK_LOOP: u64 = 1 << 40;

/// char_type: WAR 0 .. JES 7 (secret heroes use their base's bank).
fn ct(p: &Player) -> usize {
    p.class_id % 8
}

/// AudioPlayerDies: S_PLAYERDIES and the hero's S_xxxDIE2 (lbl_8012380C).
pub(crate) fn player_dies(p: &Player) -> [String; 2] {
    let cry = if p.fx.special & POJO != 0 {
        format!("#{S_POJOPOISON:x}")
    } else {
        format!("S_{}DIE2", CT[ct(p)])
    };
    ["#1".into(), cry]
}

/// AudioPlayerEatSFX (pmotion.c's missile block): the throw's sound by the
/// weapon (field_11C): super / 3-way / 5-way S_SUPERSHOT, the amulets' own,
/// else the hero's S_xxxTHROW (lbl_801236A4). Breath power-ups (p->flags
/// 0x8000 / 0x4000) have theirs instead.
pub(crate) fn throw_sound(p: &Player) -> String {
    let id = if p.fx.special & 0x8000 != 0 {
        92
    } else if p.fx.special & 0x4000 != 0 {
        93
    } else if p.fx.weapon & 0x58_0000 != 0 {
        66
    } else {
        match p.fx.weapon & 0xF {
            1 => 68,
            2 => 70,
            3 => 69,
            4 => 67,
            _ => return format!("S_{}THROW", CT[ct(p)]),
        }
    };
    format!("#{id:x}")
}

/// AudioPlayerEatFood: one time in four the hero's eating line for the food
/// (snd_eat; 0 other, 1 apple, 2 banana, 3 pineapple), else the eating
/// sound (snd_eat_default; Pojo's own).
pub(crate) fn eat_food(rng: &mut Rng, p: &Player, food: usize) -> String {
    let c = ct(p);
    if rng.next_u32() % 4 == 0 {
        if p.fx.special & POJO != 0 {
            return String::new();
        }
        if c == 3 {
            return format!("S_ARCEAT{}", food.min(3) + 1);
        }
        return format!("S_{}EAT", CT[c]);
    }
    if p.fx.special & POJO != 0 {
        return format!("#{S_POJOEATSFX:x}");
    }
    format!("S_{}EATSFX", CT[c])
}

/// AudioPlayerSeverePain (lbl_80123624, the S_xxxPOISON cries): bad food and
/// gas.
pub(crate) fn severe_pain(p: &Player) -> String {
    if p.fx.special & POJO != 0 {
        return format!("#{S_POJOPOISON:x}");
    }
    format!("S_{}POISON", CT[ct(p)])
}

/// fn_8005DE50's pickup sounds: fn_8009CDF8 keys, fn_8009CE38 runestones,
/// fn_8009CFA8 gold (the treasure realm's coins by value and player),
/// fn_8009CEE0 power-ups, fn_8009D038 the rest. Food has only its eating,
/// a scroll (controller message) none.
pub(crate) fn pickup_sound(kind: &PickupKind, realm: Option<u32>, pi: usize) -> Option<String> {
    use crate::powerups as pu;
    let id = match *kind {
        PickupKind::Food(_) | PickupKind::Scroll(_) => return None,
        PickupKind::Key(_) => 0x25,
        PickupKind::Shard(_) => 0x28,
        PickupKind::Gold(v) if realm == Some(12) => {
            let metal = match v {
                50 => "BRONZE",
                100 => "SILVER",
                _ => "GOLD",
            };
            return Some(format!("S_PKUP{metal}{}", pi % 4 + 1));
        }
        PickupKind::Powerup { kind, flags, .. } => match kind {
            pu::ARMOR if flags & 0x20_0000 != 0 => 0x29,
            pu::SPECIAL if flags & pu::S_LEVITATE != 0 => 0x58,
            pu::SPECIAL if flags & pu::S_GROWTH != 0 => 0x56,
            pu::SPECIAL if flags & pu::S_SHRINK != 0 => 0x54,
            pu::SPECIAL if flags & POJO != 0 => 0x5E,
            _ => 0x27,
        },
        _ => 0x26,
    };
    Some(format!("#{id:x}"))
}

/// add_got_it's (subtype, count) for a pickup; None for the ones it ignores.
pub(crate) fn got_it_of(kind: &PickupKind) -> Option<(u8, i32)> {
    Some(match *kind {
        PickupKind::Gold(v) => (1, v as i32),
        PickupKind::Key(n) => (2, n as i32),
        PickupKind::Food(hp) => (3, hp as i32),
        PickupKind::Potion(_) => (4, 0),
        PickupKind::Powerup { kind, .. } => (kind.clamp(5, 9), 0),
        PickupKind::Shard(_) => (10, 0),
        PickupKind::Legend(v) => (13, v as i32),
        PickupKind::Crystal(_) => (15, 1),
        PickupKind::Gargoyle(i) => (16, i as i32),
        PickupKind::Scroll(_) | PickupKind::Other(_) => return None,
    })
}

/// The food's kind for snd_eat by its item name (fn_8005DE50: PINEAPPLE 3,
/// APPLE 1, BANANA 2).
pub(crate) fn food_kind(name: &str) -> usize {
    match name.to_ascii_uppercase().as_str() {
        "PINEAPPLE" => 3,
        "APPLE" => 1,
        "BANANA" => 2,
        _ => 0,
    }
}

impl Game {
    /// sMusicTrackHi: the level's realm.
    pub(crate) fn realm(&self) -> Option<u32> {
        self.wave.map(|w| w >> 8)
    }

    /// good_wiz_state <= 2: the announcer is quiet once the good wizard talks.
    fn announcer_free(&self) -> bool {
        !self
            .boss
            .as_ref()
            .and_then(|b| b.wizard.as_ref())
            .is_some_and(|w| w.state > 2)
    }

    fn voice(&mut self, name: &str) {
        if self.announcer_free() {
            self.sfx.push(format!("VOICE:{name}"));
        }
    }

    /// Per tick: the sound timers, the hit-streak cheers, the got-it ticker,
    /// X-ray and the heartbeat.
    pub(crate) fn update_sounds(&mut self) {
        for h in &mut self.snd.heroes {
            h.thud -= 1;
        }
        self.snd.death_suck = (self.snd.death_suck - 1).max(0);
        self.hit_streaks();
        self.do_got_it();
        for pi in 0..self.players.len().min(4) {
            self.do_see_thru(pi);
        }
        self.heartbeat();
    }

    /// do_weakening's heartbeat: only the playing hero with the least health
    /// counts down, while at 200 or less: S_WARN every 120 ticks from 100,
    /// 60 from 25, else 30, louder as health falls (AudioHeartBeat). Not in
    /// the tower, nor under invulnerability.
    fn heartbeat(&mut self) {
        let mut it = None;
        let mut best = 9999.0;
        for (i, p) in self.players.iter().enumerate() {
            if p.state.active() && p.health < best {
                it = Some(i);
                best = p.health;
            }
        }
        let Some(i) = it.filter(|&i| i < 4) else {
            return;
        };
        let p = &self.players[i];
        let h = p.health;
        if h < 1.0 || h > 200.0 {
            return;
        }
        let quiet = self.realm() == Some(13) || p.fx.armor & 0x11_0000 != 0;
        let t = &mut self.snd.heroes[i].heartbeat;
        *t -= 1;
        if *t > 0 {
            return;
        }
        *t = if h >= 100.0 {
            120
        } else if h >= 25.0 {
            60
        } else {
            30
        };
        if !quiet {
            let vol = if h <= 10.0 {
                202
            } else if h < 25.0 {
                177
            } else if h < 100.0 {
                152
            } else {
                127
            };
            self.sfx.push(format!("S_WARN@{vol}"));
        }
    }

    /// damage_player's sounds after the damage is taken: the rumble by the
    /// hit's size, the 150 / 50 health lines, the pain cries by mode (2
    /// every hurt, 3 the poison cry, 1 after 30 damage or a 60 drop) and the
    /// thud (AudioPlayerHit, 30 ticks apart) or severe pain for gas.
    pub(crate) fn damage_sounds(&mut self, pi: usize, mode: u8, dmg: f32, flags: u32, old: i32) {
        let Some(p) = self.players.get(pi) else {
            return;
        };
        let new = (p.health + 0.5) as i32;
        let dead = p.health < 1.0;
        if dmg > 0.0 {
            let f = if dmg <= 2.0 {
                flags & 0xFFFE_FE8F
            } else {
                flags
            };
            let (strength, ticks) = if f & 0x10040 != 0 {
                (3, 30)
            } else if f & 0x120 != 0 {
                (2, 20)
            } else if f & 0x90 != 0 {
                (1, 15)
            } else {
                (0, 10)
            };
            self.snd.rumble.push(Rumble {
                player: pi,
                strength,
                ticks,
            });
        }
        if dead || pi >= 4 {
            return;
        }
        let p = &self.players[pi];
        let state = p.state.active();
        let name = format!("VOICE:S_{}{}1", p.color, p.class);
        let c = CT[ct(p)];
        let pojo = p.fx.special & POJO != 0;
        let invuln = p.fx.armor & crate::powerups::A_INVULN != 0;
        let h = &mut self.snd.heroes[pi];
        if dmg > 0.0 {
            h.pain += dmg;
        }
        let mut mode = mode;
        let mut cry = None;
        if old > 150 && new <= 150 {
            self.sfx.push(format!("{name}+S_BADLY|VOICE:S_BADLY"));
        } else if old > 50 && new <= 50 {
            let line = if self.frame % 2 == 0 {
                "S_LIFEFORCE"
            } else {
                "S_ABOUT"
            };
            self.sfx.push(format!("{name}+{line}|VOICE:{line}"));
        } else if mode == 2 {
            if dmg > 0.0 {
                cry = Some(true);
            }
            h.pain = 0.0;
        } else if mode == 3 {
            // AudioPlayerPoison: lbl_80123564, the S_xxxDIE1 cries.
            if state && !invuln {
                cry = Some(false);
            }
            h.pain = 0.0;
        } else if mode != 0 {
            if old - new > 60 {
                if dmg > 0.0 {
                    cry = Some(true);
                }
                h.pain = 0.0;
                mode = 0;
            } else if h.pain >= 30.0 {
                h.pain -= 30.0;
                if dmg > 0.0 {
                    cry = Some(true);
                }
                mode = 0;
            }
        }
        let mut thud = None;
        if mode == 1 {
            if flags & 0x800 != 0 {
                if dmg > 0.0 {
                    let p = &self.players[pi];
                    self.sfx.push(severe_pain(p));
                }
            } else if h.thud <= 0 {
                let kind = if flags & 0x20000 != 0 {
                    1
                } else if flags & 0x40000 != 0 {
                    2
                } else {
                    0
                };
                if dmg > 0.0 && state {
                    thud = Some(PLAYER_HIT[kind]);
                }
                h.thud = 30;
            }
        }
        match cry {
            // AudioPlayerPain: one of lbl_801234E4's four at random.
            Some(true) if state => {
                let n = 1 + self.rng.next_u32() % 4;
                self.sfx.push(if pojo {
                    format!("#{S_POJOPAIN:x}")
                } else {
                    format!("S_{c}PAIN{n}")
                });
            }
            Some(false) => self.sfx.push(if pojo {
                format!("#{S_POJOPAIN:x}")
            } else {
                format!("S_{c}DIE1")
            }),
            _ => {}
        }
        if let Some(id) = thud {
            self.sfx_id(id);
        }
    }

    /// AudioPlayerHit for a monster's blow (enemy.c:5760): grunts, knights and
    /// lizardmen thud by tier (S_PLYRDMG4 from tier 2, else S_PLYRDMG5);
    /// these, small monsters, demons, mummies and treefolk then hurt without
    /// cries (damage mode 0).
    pub(crate) fn monster_blow(&mut self, pi: usize, etype: usize, strength: usize) {
        use crate::enemies::{IT, TYPES};
        let small = TYPES[etype.min(33)].height * 0.5 <= 2.0 && etype != IT;
        match etype {
            4 | 5 | 10 if !small => {
                if self.players.get(pi).is_some_and(|p| p.state.active()) {
                    self.sfx_id(PLAYER_HIT[if strength >= 2 { 3 } else { 4 }]);
                }
                self.snd.mode = Some(0);
            }
            _ if small || matches!(etype, 2 | 8 | 11) => self.snd.mode = Some(0),
            _ => {}
        }
    }

    /// AudioDamageTile: the realm's sound for the tile's subtype (the
    /// tentacles louder; Dragon's flame holes S_FIREHOLE2).
    pub(crate) fn damage_tile_sound(&mut self, sub: i32) {
        let Some(row) = self.realm().and_then(|r| DAMAGE_TILE.get(r as usize)) else {
            return;
        };
        let Some(name) = row.get(sub.max(0) as usize).filter(|n| !n.is_empty()) else {
            return;
        };
        if *name == "S_FIREHOLE" && self.boss_type() == Some(34) {
            self.sfx.push("S_FIREHOLE2".into());
        } else {
            self.sfx.push(name.to_string());
        }
    }

    fn boss_type(&self) -> Option<i32> {
        self.boss.as_ref().map(|b| b.boss_type)
    }

    /// AudioGeneratorDamaged / AudioGeneratorDies: the realm's S_GENDAMx /
    /// S_GENKILLx (lbl_80124148 / lbl_80124114; realm 10's warlock
    /// generators their own); a generator dies quietly in a boss level.
    pub(crate) fn generator_sound(&self, etype: usize, destroyed: bool) -> Option<String> {
        let r = self.realm()?;
        if !(1..=11).contains(&r) {
            return None;
        }
        if destroyed && self.boss.is_some() {
            return None;
        }
        let what = if destroyed { "KILL" } else { "DAM" };
        if r == 10 && etype == 24 {
            return Some(format!("S_GEN{what}WAR"));
        }
        Some(format!("S_GEN{what}{}", (b'A' + r as u8 - 1) as char))
    }

    /// PlayerMotion's melee hit streak (pmotion.c:2701): each swing that
    /// lands on a critter or a tall monster adds 1 (3 if it kills); two
    /// seconds without one, 45 or more is "Bravery" and 30 "Heroic"
    /// (player.c:2553).
    pub(crate) fn streak_hit(&mut self, pi: usize, killed: bool) {
        if let Some(h) = self.snd.heroes.get_mut(pi) {
            h.streak += if killed { 3 } else { 1 };
            h.streak_time = self.time;
        }
    }

    fn hit_streaks(&mut self) {
        for pi in 0..self.players.len().min(4) {
            let h = self.snd.heroes[pi];
            if !self.players[pi].state.active() || h.streak_time <= 0.0 {
                continue;
            }
            if self.time > h.streak_time + 2.0 {
                if h.streak >= 45 {
                    self.voice("S_BRAVERY");
                } else if h.streak >= 30 {
                    self.voice("S_HEROIC");
                }
                self.snd.heroes[pi].streak = 0;
                self.snd.heroes[pi].streak_time = 0.0;
            }
        }
    }

    /// AudioNumRunesFound: "rune stone found", or "N rune stones found",
    /// after a runestone is picked up (the party's count).
    pub(crate) fn runes_found(&mut self) {
        let n = (self.shards & 0x1FFF).count_ones();
        match n {
            0 => {}
            1 => self.voice("S_RUNEFOUND1"),
            2..=12 => self.voice(&format!("S_RUNE{n}+S_RUNEFOUND2")),
            _ => {}
        }
    }

    /// do_exit's first exit of the level: S_TUNNEL, then towerRuneNearAudio
    /// (tower.c:339): Skorne taunts a party that leaves the level's legend
    /// rune, or failing that its runestone, behind (not in realms 5 and 8).
    pub(crate) fn exit_sounds(&mut self) {
        if self.snd.exit_heard {
            return;
        }
        self.snd.exit_heard = true;
        self.sfx_id(4);
        if matches!(self.realm(), Some(5 | 8)) {
            return;
        }
        if let Some(v) = self.snd.special_rune {
            if self.legend_held & (1u16 << v.min(15)) == 0 {
                let line = LEFT_RUNE[self.snd.taunt_rune % 4];
                self.snd.taunt_rune = (self.snd.taunt_rune + 1) % 4;
                self.voice(line);
            }
            return;
        }
        if let Some(bit) = self.snd.special_shard {
            if self.shards & (1u16 << bit.min(15)) == 0 {
                let line = LEFT_SHARD[self.snd.taunt_shard % 4];
                self.snd.taunt_shard = (self.snd.taunt_shard + 1) % 4;
                self.voice(line);
            }
        }
    }

    /// SetItem: the level's legend rune and runestone (sSpecialItem13 /
    /// sSpecialItem10), for the exit taunts.
    pub(crate) fn note_special(&mut self, kind: &PickupKind) {
        match *kind {
            PickupKind::Legend(v) => self.snd.special_rune = Some(v),
            PickupKind::Shard(b) => self.snd.special_shard = Some(b),
            _ => {}
        }
    }

    /// add_got_it: queue a card in a free slot (of 24).
    pub(crate) fn add_got_it(&mut self, player: usize, kind: u8, count: i32) {
        if !matches!(kind, 1..=10 | 13 | 15 | 16) {
            return;
        }
        if self.snd.got_it.len() >= GOT_IT_SLOTS {
            return;
        }
        self.snd.got_it.push(GotIt {
            state: 1,
            player,
            kind,
            count,
            y: 0,
            timer: 0,
        });
    }

    /// do_got_it: a card appears at y 384, slides up a pixel a tick to 304,
    /// holds 90 ticks, then slides down and is gone at 400.
    fn do_got_it(&mut self) {
        for g in &mut self.snd.got_it {
            match g.state {
                1 => {
                    g.y = 0x180;
                    g.state = 2;
                }
                2 => {
                    g.y -= 1;
                    if g.y <= 0x130 {
                        g.y = 0x130;
                        g.state = 3;
                        g.timer = 0x5A;
                    }
                }
                3 => {
                    g.timer -= 1;
                    if g.timer <= 0 {
                        g.state = 4;
                    }
                }
                _ => {
                    g.y += 1;
                    if g.y >= 400 {
                        g.state = 0;
                    }
                }
            }
        }
        self.snd.got_it.retain(|g| g.state != 0);
    }

    /// ClosestChest: the nearest unopened chest within 10.
    fn closest_chest(&self, pos: Vec3) -> Option<usize> {
        self.entities
            .iter()
            .enumerate()
            .filter(|(_, e)| e.alive && matches!(e.kind, Kind::Container { chest: true, .. }))
            .map(|(i, e)| (i, (e.pos - pos).length()))
            .filter(|&(_, d)| d < 10.0)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }

    /// The model the X-ray shows for a chest: Death's DEATH_ICON, else the
    /// item's own (a random item's pick is by the chest, as RandItemIdx).
    fn chest_content_model(&self, ei: usize) -> Option<usize> {
        let e = &self.entities[ei];
        let Kind::Container { drop: Some(d), .. } = e.kind else {
            return None;
        };
        let infos = &self.level.worlds.item_infos;
        let mut info = infos.get(d)?;
        if info.item_type == ItemType::Random && !info.random_indices.is_empty() {
            let pick = info.random_indices[e.id as usize % info.random_indices.len()];
            info = infos.get(pick as usize)?;
        }
        let name = if info.item_type == ItemType::Enemy {
            "DEATH_ICON".to_string()
        } else if info.item_type == ItemType::Powerup {
            info.name.to_ascii_uppercase()
        } else {
            return None;
        };
        self.item_models.get(&name).copied().flatten()
    }

    /// do_see_thru: while a hero's X-ray is on, the closest chest on screen
    /// turns see-through with its content inside (S_XRAY when it changes);
    /// a chest another hero already looks into shows nothing.
    fn do_see_thru(&mut self, pi: usize) {
        let p = &self.players[pi];
        if !p.state.active() || p.fx.special & crate::powerups::S_XRAY == 0 {
            self.snd.xray[pi] = None;
            return;
        }
        let found = self.closest_chest(p.pos).filter(|&i| {
            let e = &self.entities[i];
            let r = self.item_radius(e).unwrap_or(1.0);
            self.on_screen(e.pos, 2.0 * r)
        });
        let view = found.and_then(|i| {
            let e = &self.entities[i];
            let taken =
                (0..4).any(|j| j != pi && self.snd.xray[j].is_some_and(|x| x.chest == e.id));
            if taken {
                return None;
            }
            Some(XRay {
                chest: e.id,
                model: self.chest_content_model(i)?,
                pos: e.pos,
                yaw: e.yaw,
            })
        });
        let fresh = view.is_some_and(|v| self.snd.xray[pi].is_none_or(|o| o.chest != v.chest));
        self.snd.xray[pi] = view;
        if fresh {
            self.sfx_id(0x51);
        }
    }

    fn item_radius(&self, e: &Entity) -> Option<f32> {
        match &e.kind {
            Kind::Container { .. } => self.solid_radius(e),
            _ => None,
        }
    }

    fn solid_radius(&self, e: &Entity) -> Option<f32> {
        match e.solid? {
            Shape::Cylinder { radius } => Some(radius),
            Shape::Box { half } => Some(half.length()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn got_it_slides_up_holds_and_leaves() {
        let Some(fs) = data() else { return };
        let mut g = Game::new(&fs, "levelA1", &[("WAR", "RED")]).unwrap();
        g.add_got_it(0, 2, 3);
        g.add_got_it(0, 12, 1); // not a ticker subtype
        assert_eq!(g.snd.got_it.len(), 1);
        assert_eq!(g.snd.got_it[0].icon(false), Some("KEY_RING"));
        let (mut ticks, mut top) = (0, i32::MAX);
        while !g.snd.got_it.is_empty() && ticks < 1000 {
            g.do_got_it();
            if let Some(c) = g.snd.got_it.first() {
                top = top.min(c.y);
            }
            ticks += 1;
        }
        assert_eq!(top, 304);
        // Shown at 384, 80 ticks up, 90 held, 96 down to 400.
        assert_eq!(ticks, 1 + 80 + 90 + 96);
    }

    #[test]
    fn damage_modes_cry_thud_and_rumble() {
        let Some(fs) = data() else { return };
        let mut g = Game::new(&fs, "levelA1", &[("WAR", "RED")]).unwrap();
        g.sfx.clear();
        // Mode 2 (a damage tile): a pain cry every hurt.
        g.snd.mode = Some(2);
        g.damage_player(0, 10.0, 0);
        assert!(
            g.sfx.iter().any(|s| s.starts_with("S_WARPAIN")),
            "{:?}",
            g.sfx
        );
        assert_eq!(
            g.snd.rumble.last().map(|r| (r.strength, r.ticks)),
            Some((0, 10))
        );
        g.sfx.clear();
        // Mode 3 (spikes): AudioPlayerPoison's S_WARDIE1.
        g.snd.mode = Some(3);
        g.damage_player(0, 10.0, 0);
        assert_eq!(g.sfx, vec!["S_WARDIE1".to_string()]);
        g.sfx.clear();
        // Mode 1: the thud, then nothing for 30 ticks; a knockdown rumbles harder.
        g.damage_player(0, 5.0, DmgType::KNOCKDOWN);
        assert_eq!(g.sfx, vec!["#2f".to_string()]);
        assert_eq!(
            g.snd.rumble.last().map(|r| (r.strength, r.ticks)),
            Some((2, 20))
        );
        g.sfx.clear();
        g.damage_player(0, 5.0, 0);
        assert!(g.sfx.is_empty(), "{:?}", g.sfx);
    }

    #[test]
    fn generator_runes_streak_and_exit_sounds() {
        let Some(fs) = data() else { return };
        let mut g = Game::new(&fs, "levelA1", &[("WAR", "RED")]).unwrap();
        assert_eq!(g.realm(), Some(1));
        assert_eq!(g.generator_sound(4, false).as_deref(), Some("S_GENDAMA"));
        assert_eq!(g.generator_sound(4, true).as_deref(), Some("S_GENKILLA"));
        g.sfx.clear();
        g.shards = 0b101;
        g.runes_found();
        assert_eq!(g.sfx, vec!["VOICE:S_RUNE2+S_RUNEFOUND2".to_string()]);
        g.sfx.clear();
        g.time = 1.0;
        for _ in 0..10 {
            g.streak_hit(0, true);
        }
        g.time += 2.5;
        g.hit_streaks();
        assert_eq!(g.sfx, vec!["VOICE:S_HEROIC".to_string()]);
        g.sfx.clear();
        g.snd.special_rune = None;
        g.snd.special_shard = Some(7);
        g.exit_sounds();
        g.exit_sounds();
        assert_eq!(
            g.sfx,
            vec!["#4".to_string(), "VOICE:S_SKORNTAUNT1".to_string()]
        );
    }

    #[test]
    fn food_icons_by_amount() {
        let g = |count| GotIt {
            state: 2,
            player: 0,
            kind: 3,
            count,
            y: 0,
            timer: 0,
        };
        assert_eq!(g(100).icon(false), Some("MEAT"));
        assert_eq!(g(-100).icon(false), Some("BADMEAT"));
        assert_eq!(g(20).icon(false), Some("FRUIT"));
        assert_eq!(g(-20).icon(false), Some("BADFRUIT"));
    }

    fn data() -> Option<GameFs> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game-data");
        GameFs::open(&root).ok()
    }

    #[test]
    fn compass_and_xray_models_load() {
        let Some(fs) = data() else { return };
        let g = Game::new(&fs, "levelA1", &[("WAR", "RED")]).unwrap();
        assert!(g.snd.compass_model.is_some(), "no COMPASS");
        assert!(g.item_models.get("DEATH_ICON").copied().flatten().is_some());
        // MBWindowTo3D's point projects back to (64, 128).
        let at = crate::scene::unproject(&g, 64.0, 128.0, 10.0);
        let (x, y) = crate::scene::project(&g, at).unwrap();
        assert!((x - 64.0).abs() < 0.5 && (y - 128.0).abs() < 0.5, "{x} {y}");
    }

    #[test]
    fn xray_sees_into_the_nearest_chest() {
        let Some(fs) = data() else { return };
        let mut g = Game::new(&fs, "levelA1", &[("WAR", "RED")]).unwrap();
        let start = g.players[0].pos;
        let chest = (0..g.entities.len())
            .filter(|&i| {
                matches!(g.entities[i].kind, Kind::Container { chest: true, .. })
                    && g.chest_content_model(i).is_some()
            })
            .min_by(|&a, &b| {
                let d = |i: usize| (g.entities[i].pos - start).length();
                d(a).total_cmp(&d(b))
            })
            .expect("a chest with a power-up");
        let (id, pos) = (g.entities[chest].id, g.entities[chest].pos);
        g.players[0].pos = pos + Vec3::new(4.0, 0.0, 0.0);
        g.players[0]
            .powerups
            .add(crate::powerups::SPECIAL, crate::powerups::S_XRAY, 0.0, 60.0);
        g.players[0].powerups.slots[0].state = crate::powerups::ON;
        let mut heard = false;
        for _ in 0..300 {
            g.players[0].pos = pos + Vec3::new(4.0, 0.0, 0.0);
            g.update(1.0 / 60.0, &[PlayerInput::default()]);
            heard |= g.sfx.drain(..).any(|s| s == "#51");
        }
        let x = g.snd.xray[0].expect("no x-ray view");
        assert_eq!(x.chest, id);
        assert!(heard, "no S_XRAY");
    }

    #[test]
    fn heartbeat_beats_faster_and_louder_as_health_falls() {
        let Some(fs) = data() else { return };
        let mut g = Game::new(&fs, "levelA1", &[("WAR", "RED")]).unwrap();
        let beats = |g: &mut Game, hp: f32| {
            g.players[0].health = hp;
            let mut v = Vec::new();
            for t in 0..240 {
                g.players[0].health = hp;
                g.update(1.0 / 60.0, &[PlayerInput::default()]);
                v.extend(
                    g.sfx
                        .drain(..)
                        .filter(|s| s.starts_with("S_WARN"))
                        .map(|s| (t, s)),
                );
            }
            v
        };
        assert!(beats(&mut g, 400.0).is_empty());
        let b = beats(&mut g, 150.0);
        assert!(
            b.len() == 2 && b.iter().all(|(_, s)| s == "S_WARN@127"),
            "{b:?}"
        );
        let b = beats(&mut g, 8.0);
        assert!(
            b.len() >= 7 && b.iter().all(|(_, s)| s == "S_WARN@202"),
            "{b:?}"
        );
    }

    #[test]
    fn event_sound_names_resolve() {
        let Some(fs) = data() else { return };
        let mut reqs: Vec<String> = Vec::new();
        for c in CT {
            for n in 1..=4 {
                reqs.push(format!("S_{c}PAIN{n}"));
            }
            for s in ["DIE1", "DIE2", "POISON", "EATSFX", "THROW"] {
                reqs.push(format!("S_{c}{s}"));
            }
            if c != "ARC" {
                reqs.push(format!("S_{c}EAT"));
            }
        }
        reqs.extend((1..=4).map(|n| format!("S_ARCEAT{n}")));
        for row in DAMAGE_TILE {
            reqs.extend(row.iter().filter(|n| !n.is_empty()).map(|n| n.to_string()));
        }
        for l in "ABCDEFGHIJK".chars() {
            reqs.push(format!("S_GENDAM{l}"));
            reqs.push(format!("S_GENKILL{l}"));
        }
        reqs.extend((2..=12).map(|n| format!("S_RUNE{n}")));
        reqs.extend(LEFT_RUNE.iter().chain(&LEFT_SHARD).map(|s| s.to_string()));
        for s in [
            "S_GENDAMWAR",
            "S_GENKILLWAR",
            "S_FIREHOLE2",
            "S_HEROIC",
            "S_BRAVERY",
            "S_RUNEFOUND1",
            "S_RUNEFOUND2",
            "S_DEATHSUCK",
            "S_PKUPBRONZE1",
            "S_PKUPSILVER4",
            "S_PKUPGOLD2",
            "#1",
            "#4",
            "#51",
            "#60",
            "#65",
            "#68",
            "#2f",
            "#36",
        ] {
            reqs.push(s.into());
        }
        let missing: Vec<String> = crate::audio::check_sounds(&fs, &reqs)
            .into_iter()
            .filter(|(_, f)| f.as_ref().is_none_or(|f| f.1 <= 0.0))
            .map(|(r, _)| r)
            .collect();
        assert!(missing.is_empty(), "{missing:?}");
    }

    #[test]
    fn food_kinds_by_name() {
        assert_eq!(food_kind("PINEAPPLE"), 3);
        assert_eq!(food_kind("apple"), 1);
        assert_eq!(food_kind("BANANA"), 2);
        assert_eq!(food_kind("MEAT"), 0);
    }
}
