//! The player action machine (anim/action.c DoPlayerAction, game/pmotion.c
//! fn_80088938 and the motion-state switch): buttons become a motion state, the
//! state a requested animation, and DoPlayerAction decides from the current and
//! requested animations what plays next and when, firing the swing, hit and
//! throw bits the game resolves. Animations run on the original clock
//! (anim.c AnimateTree / CalcAnimInfo), not on the model's own loop flags.

use gdl_formats::anim::ATree;

/// player_action_type (include/game/player.h): the animation slots.
pub mod p {
    pub const READY: u16 = 0;
    pub const IDLE1: u16 = 1;
    pub const IDLE2: u16 = 2;
    pub const IDLE2_LOOP: u16 = 3;
    pub const DEFENDL: u16 = 4;
    pub const DEFENDR: u16 = 5;
    pub const DEFENDB: u16 = 6;
    pub const DEFENDF: u16 = 7;
    pub const SHOVE: u16 = 8;
    pub const STRAFE_WLKF: u16 = 9;
    pub const STRAFE_WLKF2: u16 = 10;
    pub const STRAFE_WLKB: u16 = 11;
    pub const STRAFE_WLKB2: u16 = 12;
    pub const STRAFE_WLKL: u16 = 13;
    pub const STRAFE_WLKL2: u16 = 14;
    pub const STRAFE_WLKR: u16 = 15;
    pub const STRAFE_WLKR2: u16 = 16;
    pub const WALK: u16 = 17;
    pub const WALK2: u16 = 18;
    pub const RUN: u16 = 19;
    pub const RUN2: u16 = 20;
    pub const SHIELD_READY: u16 = 21;
    pub const SHIELD_RUN: u16 = 22;
    pub const PIVOTL: u16 = 23;
    pub const PIVOTR: u16 = 24;
    pub const PUSH: u16 = 25;
    pub const PUSHED: u16 = 26;
    pub const HIT_REACT: u16 = 27;
    pub const PICKUP: u16 = 28;
    pub const DEATHGRAB: u16 = 29;
    pub const DEATHGRAB2: u16 = 30;
    pub const DEATHGRAB_R: u16 = 31;
    pub const ATTACK_SLOW: u16 = 32;
    pub const ATTACK_SLOW1: u16 = 33;
    pub const ATTACK_SLOW1_R: u16 = 34;
    pub const PWRA_CLOSE: u16 = 35;
    pub const PWRA_CLOSE_R: u16 = 36;
    pub const PWRA_MED: u16 = 37;
    pub const PWRA_MED_R: u16 = 38;
    pub const QUICK: u16 = 39;
    pub const QUICK2: u16 = 40;
    pub const QUICK3: u16 = 41;
    pub const QUICK2_R: u16 = 42;
    pub const QUICK3_R: u16 = 43;
    pub const RIGHT: u16 = 44;
    pub const RIGHT2: u16 = 45;
    pub const RIGHT_R: u16 = 46;
    pub const RIGHT2_R: u16 = 47;
    pub const LEFT: u16 = 48;
    pub const LEFT2: u16 = 49;
    pub const LEFT_R: u16 = 50;
    pub const LEFT2_R: u16 = 51;
    pub const A180: u16 = 52;
    pub const A1802: u16 = 53;
    pub const A180_R: u16 = 54;
    pub const A1802_R: u16 = 55;
    pub const A180L: u16 = 56;
    pub const A180L2: u16 = 57;
    pub const A180L_R: u16 = 58;
    pub const A180L2_R: u16 = 59;
    pub const A360: u16 = 60;
    pub const A360_R: u16 = 61;
    pub const STEP: u16 = 62;
    pub const STEP2: u16 = 63;
    pub const STEP3: u16 = 64;
    pub const STEP2_R: u16 = 65;
    pub const STEP3_R: u16 = 66;
    pub const Q3TOSTEP1: u16 = 67;
    pub const Q3TOSTEP1_R: u16 = 68;
    pub const WALK2_ATK: u16 = 69;
    pub const WALK2_ATK_R: u16 = 70;
    pub const STRAFE_ATKF: u16 = 71;
    pub const STRAFE_ATKF2: u16 = 72;
    pub const STRAFE_ATKB: u16 = 73;
    pub const STRAFE_ATKB2: u16 = 74;
    pub const STRAFE_ATKL: u16 = 75;
    pub const STRAFE_ATKL2: u16 = 76;
    pub const STRAFE_ATKR: u16 = 77;
    pub const STRAFE_ATKR2: u16 = 78;
    pub const LOW: u16 = 79;
    pub const LOW2: u16 = 80;
    pub const LOW_R: u16 = 81;
    pub const KICK: u16 = 82;
    pub const KICK_R: u16 = 83;
    pub const PWRA_LOW: u16 = 84;
    pub const PWRA_LOW_R: u16 = 85;
    pub const PWRB: u16 = 86;
    pub const PWRC: u16 = 87;
    pub const COMBO_ACTIVE1: u16 = 88;
    pub const COMBO_ACTIVE2: u16 = 89;
    pub const COMBO_ACTIVE3: u16 = 90;
    pub const THROW: u16 = 91;
    pub const THROWQ: u16 = 92;
    pub const THROW2: u16 = 93;
    pub const THROW2Q: u16 = 94;
    pub const THROW_RELEASE: u16 = 95;
    pub const THROW2_RELEASE: u16 = 96;
    pub const THROW_RECOVER: u16 = 97;
    pub const THROW2_RECOVER: u16 = 98;
    pub const PWRA_THROW: u16 = 99;
    pub const PWRA_THROW_R: u16 = 100;
    pub const THROW_STEP: u16 = 101;
    pub const THROW_STEP2: u16 = 102;
    pub const FIREL: u16 = 103;
    pub const FIRER: u16 = 104;
    pub const FIREL_R: u16 = 105;
    pub const FIRER_R: u16 = 106;
    pub const SSHOT: u16 = 107;
    pub const SSHOT2: u16 = 108;
    pub const SSHOT_R: u16 = 109;
    pub const BREATHE: u16 = 110;
    pub const BREATHE_R: u16 = 111;
    pub const HAMMER: u16 = 112;
    pub const HAMMER_R: u16 = 113;
    pub const LAST_ATTACK: u16 = 114;
    pub const USE_MAGIC: u16 = 115;
    pub const MAGIC_RELEASE: u16 = 116;
    pub const THROW_MAGIC: u16 = 117;
    pub const THROW_MAGIC_RELEASE: u16 = 118;
    pub const DEFEND1: u16 = 119;
    pub const DEFEND2: u16 = 120;
    pub const DEFEND3: u16 = 121;
    pub const DEATH_REACT: u16 = 122;
    pub const VICTORY: u16 = 123;
    pub const ACTION_START: u16 = 124;
    pub const ACTION_INIT: u16 = 125;
    pub const ACTION_DEATH: u16 = 126;
    pub const STUN_REACT: u16 = 127;
    pub const STUCK: u16 = 128;
    pub const SPIKE_HIT: u16 = 129;
    pub const KNOCKBACK: u16 = 130;
    pub const FALL_DOWN: u16 = 131;
    pub const GET_UP: u16 = 132;
    pub const FALL_DOWN_FWD: u16 = 133;
    pub const GET_UP_FWD: u16 = 134;
    pub const WHIRLWIND: u16 = 135;
    pub const COMBO_WAR1: u16 = 136;
    pub const COMBO_WAR2: u16 = 137;
    pub const COMBO_WAR3: u16 = 138;
    pub const COMBO_VAL: u16 = 139;
    pub const COMBO_WIZ: u16 = 140;
    pub const COMBO_ARC: u16 = 141;
    pub const COMBO_DWF1: u16 = 142;
    pub const COMBO_DWF2: u16 = 143;
    pub const COMBO_DWF3: u16 = 144;
    pub const COMBO_KNI: u16 = 145;
    pub const COMBO_SOR: u16 = 146;
    pub const COMBO_JES: u16 = 147;
    pub const GRABBED: u16 = 148;
}

pub const NACTIONS: usize = 149;

/// lbl_80126C68: each slot's sequence name (as shipped, with its quirks:
/// 52/53 and 103-106 are named the other way round from the enum).
pub const NAMES: [&str; NACTIONS] = [
    "READY",
    "IDLE1",
    "IDLE2",
    "IDLE2_LOOP",
    "DEFENDLEFT",
    "DEFENDRIGHT",
    "DEFENDBACK",
    "DEFENDBACK",
    "SHOVE",
    "STRAFE_WLKF1",
    "STRAFE_WLKF2",
    "STRAFE_WLKB1",
    "STRAFE_WLKB2",
    "STRAFE_WLKL1",
    "STRAFE_WLKL2",
    "STRAFE_WLKR1",
    "STRAFE_WLKR2",
    "WALK1",
    "WALK2",
    "RUN1",
    "RUN2",
    "SHIELD_READY",
    "SHIELD_RUN",
    "PIVOTL",
    "PIVOTR",
    "PUSH",
    "PUSHED",
    "HITREACT",
    "PICK",
    "DEATHGRABS",
    "DEATHGRAB",
    "DEATHGRABR",
    "ATTSTART",
    "ATTSLOW1",
    "ATTSLOW1R",
    "ATTPWRACLOSE",
    "ATTPWRACLOSER",
    "ATTPWRAMED",
    "ATTPWRAMEDR",
    "ATTQUICK1",
    "ATTQUICK2",
    "ATTQUICK3",
    "ATTQUICK2R",
    "ATTQUICK3R",
    "ATTQ3RIGHT",
    "ATTQ2RIGHT",
    "ATTQ3RIGHTR",
    "ATTQ2RIGHTR",
    "ATTQ3LEFT",
    "ATTQ2LEFT",
    "ATTQ3LEFTR",
    "ATTQ2LEFTR",
    "ATTQ2180",
    "ATTQ3180",
    "ATTQ2180R",
    "ATTQ3180R",
    "ATTQ2180L",
    "ATTQ3180L",
    "ATTQ2180LR",
    "ATTQ3180LR",
    "ATT360",
    "ATT360R",
    "ATTSTEP1",
    "ATTSTEP2",
    "ATTSTEP3",
    "ATTSTEP2R",
    "ATTSTEP3R",
    "ATTQ3TOSTEP1",
    "ATTQ3TOSTEP1R",
    "ATTWALK2",
    "ATTWALK2R",
    "STRAFE_ATKF1",
    "STRAFE_ATKF2",
    "STRAFE_ATKB1",
    "STRAFE_ATKB2",
    "STRAFE_ATKL1",
    "STRAFE_ATKL2",
    "STRAFE_ATKR1",
    "STRAFE_ATKR2",
    "ATTLOW1",
    "ATTLOW2",
    "ATTLOWR",
    "ATTLOWK",
    "ATTLOWKR",
    "ATTPWRALOW",
    "ATTPWRALOWR",
    "ATTPWRB",
    "ATTPWRC",
    "COMBOACT1",
    "COMBOACT2",
    "COMBOACT3",
    "THROW1S",
    "THROW1S",
    "THROW2S",
    "THROW2S",
    "THROW1",
    "THROW2",
    "THROW1R",
    "THROW2R",
    "ATTPWRATHROW",
    "ATTPWRATHROWR",
    "STRAFE_ATKF1",
    "STRAFE_ATKF2",
    "ATTFIREL",
    "ATTFIRELR",
    "ATTFIRER",
    "ATTFIRERR",
    "SSHOT1",
    "SSHOT2",
    "SSHOTR",
    "ATTBREATHE",
    "ATTBREATHER",
    "ATTCHOP",
    "ATTCHOPR",
    "ATTACK1",
    "MAGICS",
    "MAGICR",
    "THROWPOTIONS",
    "THROWPOTIONR",
    "DEFEND1",
    "DEFEND2",
    "DEFENDR",
    "STUN2",
    "VICTORY",
    "START",
    "INIT",
    "DEATH",
    "STUN1",
    "WEBREACT",
    "HITREACT",
    "HITREACT",
    "FALLDOWN",
    "GETUP",
    "FALLFRNT",
    "GETUP2",
    "FLYUP",
    "COMBOWAR1",
    "COMBOWAR2",
    "COMBOWAR3",
    "COMBOVAL",
    "COMBOWIZ",
    "COMBOARC",
    "COMBODWF1",
    "COMBODWF2",
    "COMBODWF3",
    "COMBOKNI",
    "COMBOSOR",
    "COMBOJES",
    "GRABBED",
];

/// PlayerAttackType (action.c:1849).
pub fn attack_type(a: u16) -> u8 {
    match a {
        4..=8 | 119..=121 => 1,
        32..=38 => 2,
        39..=43 => 3,
        44..=59 => 4,
        60..=61 => 5,
        62..=70 => 6,
        71..=78 => 7,
        79..=85 => 8,
        91..=98 => 9,
        99..=106 => 10,
        86..=87 | 107..=113 => 11,
        88..=90 | 136..=147 => 12,
        _ => 0,
    }
}

/// Logical controller bits (controls.c SpecialData).
pub mod bits {
    pub const MAGIC: u32 = 0x100;
    pub const ATK_QUICK: u32 = 0x200;
    pub const ATK_SLOW: u32 = 0x400;
    pub const TURBO: u32 = 0x800;
    pub const DEFEND: u32 = 0x1000;
    pub const CHARGE: u32 = 0x2000;
    pub const STRAFE: u32 = 0x4000;
    pub const COMBO: u32 = 0x20000;
}

/// act_bits set by DoPlayerAction for the game to resolve this tick.
pub mod act {
    /// A swing or throw started.
    pub const SWING: u32 = 0x1;
    /// Plain hit.
    pub const HIT: u32 = 0x2;
    /// x2 with knockback (slow and step swings).
    pub const HIT2: u32 = 0x4;
    /// Kick: knockdown on a short target.
    pub const KICK: u32 = 0x8;
    /// x3 with knockdown (power-A swings).
    pub const HIT3: u32 = 0x10;
    /// Release the thrown weapon.
    pub const THROW: u32 = 0x100;
    pub const SUPER_SHOT: u32 = 0x800;
    /// The power-A throw: a double-size missile.
    pub const PWRA_THROW: u32 = 0x1000;
    pub const FIREL: u32 = 0x2000;
    pub const FIRER: u32 = 0x4000;
    pub const USE_MAGIC: u32 = 0x10000;
    pub const MAGIC_RELEASE: u32 = 0x20000;
    pub const THROW_MAGIC_RELEASE: u32 = 0x40000;
    pub const BREATHE: u32 = 0x100_0000;
    pub const HAMMER: u32 = 0x200_0000;
}

/// hud_flags bits DoPlayerAction sets.
pub mod hud {
    pub const DEFEND1: u32 = 0x100;
    /// Blocking: hits taken are cancelled.
    pub const BLOCK: u32 = 0x200;
    /// Charging: hits taken are halved.
    pub const CHARGE: u32 = 0x400;
    pub const STRAFE_WALK: u32 = 0x4000;
    pub const STRAFE_ATTACK: u32 = 0x8000;
}

/// The strafe cycles alternate within the forward/back pair and within the
/// left/right pair: from a first cycle a request for F/B (L/R) plays its
/// second cycle, from a second cycle the first.
fn strafe_alternate(d: u16, next: u16, base: u16) -> u16 {
    let off = d - base;
    let pair = off / 4; // 0 forward/back, 1 left/right
    let firsts = [base + 4 * pair, base + 4 * pair + 2];
    if !firsts.contains(&next) {
        return next;
    }
    if off % 2 == 0 {
        next + 1
    } else {
        next
    }
}

/// One animation slot of a model: its sequence, frame count and rate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SlotSeq {
    pub seq: usize,
    pub frames: u16,
    pub rate: i16,
}

/// InitActions: the model's sequence for each slot (None plays READY, seq 0).
#[derive(Clone, Debug)]
pub struct ActionSeqs {
    pub slots: Vec<Option<SlotSeq>>,
}

impl ActionSeqs {
    pub fn new(tree: &ATree) -> ActionSeqs {
        let slots = NAMES
            .iter()
            .map(|n| {
                tree.sequences
                    .iter()
                    .position(|s| s.name.eq_ignore_ascii_case(n))
                    .map(|i| {
                        let s = &tree.sequences[i];
                        SlotSeq {
                            seq: i,
                            frames: s.frame_count as u16,
                            rate: s.frame_rate,
                        }
                    })
            })
            .collect();
        ActionSeqs { slots }
    }

    pub fn has(&self, a: u16) -> bool {
        self.slots.get(a as usize).is_some_and(|s| s.is_some())
    }

    fn get(&self, a: u16) -> SlotSeq {
        self.slots
            .get(a as usize)
            .copied()
            .flatten()
            .or(self.slots[0])
            .unwrap_or(SlotSeq {
                seq: 0,
                frames: 1,
                rate: 30,
            })
    }
}

/// An animinfo on the game clock (anim.c InitAnim / CalcAnimInfo).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnimInfo {
    pub seq: usize,
    /// The (fractional) frame shown.
    pub frame: f32,
    /// Seconds since frame 0.
    t: f32,
    /// Seconds a frame lasts (rate / 30 x animscale / 30).
    spf: f32,
    frames: u16,
    pub repeat: bool,
    pub animscale: f32,
    done: bool,
    looped: bool,
    /// A blend into READY holds the first frame this long.
    hold: f32,
}

impl Default for AnimInfo {
    fn default() -> Self {
        AnimInfo {
            seq: 0,
            frame: 0.0,
            t: 0.0,
            spf: 1.0 / 30.0,
            frames: 1,
            repeat: false,
            animscale: 1.0,
            done: false,
            looped: false,
            hold: 0.0,
        }
    }
}

impl AnimInfo {
    fn init(&mut self, s: SlotSeq, frame: i32, blend: f32) {
        let rate = if s.rate > 0 { s.rate as f32 } else { 30.0 };
        self.seq = s.seq;
        self.frames = s.frames;
        self.spf = rate / 30.0 * self.animscale / 30.0;
        let frame = if frame > s.frames as i32 {
            0
        } else {
            frame.max(0)
        };
        self.frame = frame as f32;
        self.t = self.frame * self.spf;
        self.hold = blend;
        self.done = false;
        self.looped = false;
    }

    /// CalcAnimInfo: true when a repeating sequence wrapped this call.
    fn calc(&mut self, dt: f32) -> bool {
        if self.done {
            if self.looped {
                self.done = false;
                self.looped = false;
            } else {
                return false;
            }
        }
        if self.frames == 0 {
            self.done = true;
            return false;
        }
        if self.hold > 0.0 {
            self.hold -= dt;
            return false;
        }
        self.t += dt;
        self.frame = self.t / self.spf;
        if self.frame >= self.frames as f32 - 0.5 {
            self.done = true;
            self.frame = (self.frames - 1) as f32;
            if self.repeat {
                self.t = 0.0;
                self.looped = true;
                return true;
            }
        }
        false
    }

    pub fn done(&self) -> bool {
        self.done
    }

    /// The sequence's length in 60 Hz ticks: (0.5 + frames x seqscale) x 2.
    pub fn length_ticks(&self) -> i32 {
        ((0.5 + self.frames as f32 * self.spf * 30.0) as i32) << 1
    }

    /// AnimateATree for things other than heroes (items): advance, and restart
    /// into `s` as `mode` says; nonzero when it restarted or finished.
    pub fn animate_slot(&mut self, dt: f32, s: SlotSeq, mode: u8) -> u32 {
        self.animate(dt, 0.0, s, 0, mode)
    }

    /// AnimateTree: advance, then restart into `s` if `mode` says so. Nonzero
    /// means the action advanced (1 restarted, 2 finished, 4 holding).
    fn animate(&mut self, dt: f32, blend: f32, s: SlotSeq, frame: i32, mode: u8) -> u32 {
        self.calc(dt);
        let cur = self.seq;
        let done = self.done;
        let restart = match mode {
            0 => done && s.seq != cur,
            1 => done,
            2 => done || s.seq != cur,
            _ => true,
        };
        let mut result = 0;
        if restart {
            self.init(s, frame, blend);
            result |= 1;
            // The restarted sequence is evaluated in the same call.
            self.calc(0.0);
        }
        if result != 0 && (done || self.done) {
            result |= 2;
        }
        if (self.repeat || s.seq == cur) && done {
            result |= 4;
        }
        result
    }
}

/// What the game knows about the player for this tick's request.
#[derive(Clone, Copy, Debug, Default)]
pub struct Ctx {
    /// Stick magnitude (0..1) and whether the player wants to move.
    pub joymag: f32,
    /// coll_flags: 1 hit range, 4 step range, 8 out of range, 2 short target.
    pub coll: u32,
    /// The target's angle from the body facing (positive to the right).
    pub melee_yaw: f32,
    /// The stick's angle from the body facing (for step swings).
    pub stick_yaw: f32,
    pub power: f32,
    pub potions: u32,
    /// Weapon power-up bits (field_11C) and special flags (p->flags).
    pub weapon: u32,
    pub special: u32,
    /// Armour shield flags (shield_flags).
    pub shield: u32,
    pub stat_armor: f32,
    /// char_type 0 WAR .. 7 JES (secret classes use their base class's).
    pub char_type: u8,
    /// A combo partner stands in reach (fn_80088EF4).
    pub partner: bool,
    /// PlayerKnockback's reaction this frame: 1 hit, 2 stun, 3 spike, 10/11
    /// knockback, 20/21 knocked down (from behind: forward), 30/31 whirlwind.
    pub reaction: u8,
    /// The linked combo partner's char_type, while linked.
    pub partner_ct: Option<u8>,
    /// The Robotron scheme's fire stick (firemag) and its angle from the
    /// move stick (fireang - joyang).
    pub firemag: f32,
    pub fire_yaw: f32,
    /// forceState: Auto Attack swings at what the hero walked into
    /// (pmotion.c:1707): the plain quick swing, no power-up's.
    pub forced: bool,
}

/// fn_80088938's quarter of an angle `d` (from the facing): back beyond 135
/// degrees, right / left beyond 45, else forward; `(f, b, l, r)` states.
fn strafe_dir(d: f32, (f, b, l, r): (u8, u8, u8, u8)) -> u8 {
    if !(-2.356_194_5..=2.356_194_5).contains(&d) {
        b
    } else if d > 0.785_398_2 {
        r
    } else if d < -0.785_398_2 {
        l
    } else {
        f
    }
}

/// One player's action state.
#[derive(Clone, Debug, Default)]
pub struct Action {
    /// anim_208 (current) and anim_20C (requested).
    pub cur: u16,
    pub next: u16,
    pub anim: AnimInfo,
    /// field_8F8 (edges while an attack button is held), field_8F4 (held) and
    /// field_908 (chained swings).
    pub edge_hist: u32,
    pub held_hist: u32,
    pub chain: u32,
    pub act_bits: u32,
    pub hud: u32,
    /// The turbo cost of the move under way, paid when it lands.
    pub coll_score: f32,
    /// Move (A48), turn (A4C) and stick (A50) scales for the next motion.
    pub move_scale: f32,
    pub turn_scale: f32,
    pub stick_scale: f32,
    /// p->action: the attack type of the current animation.
    pub action: u8,
    /// vibe_timer / vibe_timer2: idle ticks before IDLE1 / IDLE2.
    idle1: u32,
    idle2: u32,
    /// vibe_on: the last motion state.
    pub motion: u8,
    /// The previous tick's frame, for the DAMG windows (combo_cd).
    pub prev_frame: f32,
}

impl Action {
    pub fn new() -> Action {
        Action {
            move_scale: 1.0,
            turn_scale: 1.0,
            stick_scale: 1.0,
            ..Default::default()
        }
    }

    /// fn_80088938: the motion state for this tick, and the edge history.
    pub fn motion_state(&mut self, levels: u32, edges: u32, ctx: &Ctx) -> u8 {
        let mut levels = levels;
        // Pojo turns the slow attack into the quick one.
        if ctx.special & 0x400 != 0 && levels & bits::ATK_SLOW != 0 {
            levels = (levels & !bits::ATK_SLOW) | bits::ATK_QUICK;
        }
        // Being a combo partner, or carrying one, comes first.
        let ms = if self.hud & 0x40 != 0 {
            0x27
        } else if self.hud & 0x10 != 0 {
            0x26
        } else if self.hud & 0x80 != 0 {
            0x17
        } else if levels & bits::MAGIC != 0 {
            0x18
        } else if levels & bits::COMBO != 0 && ctx.power >= 50.0 && ctx.partner {
            0x16
        } else if levels & bits::TURBO != 0 && edges & bits::ATK_QUICK != 0 && ctx.power >= 0.0 {
            0x15
        } else if edges & bits::DEFEND != 0 {
            2
        } else if ctx.firemag > 0.0 {
            // The fire stick attacks: a strafing swing by its angle from the
            // move stick, else a standing one.
            if ctx.joymag > 0.0 {
                strafe_dir(ctx.fire_yaw, (0x11, 0x12, 0x13, 0x14))
            } else if levels & bits::ATK_SLOW != 0 {
                0x10
            } else {
                0x0F
            }
        } else if levels & bits::STRAFE != 0 && ctx.joymag > 0.0 {
            let atk = levels & (bits::ATK_QUICK | bits::ATK_SLOW) != 0;
            let dirs = if atk {
                (0x11, 0x12, 0x13, 0x14)
            } else {
                (9, 0x0A, 0x0B, 0x0C)
            };
            strafe_dir(ctx.stick_yaw, dirs)
        } else if edges & bits::CHARGE != 0 && ctx.power >= 5.0 {
            7
        } else if levels & bits::ATK_QUICK != 0 {
            0x0F
        } else if levels & bits::ATK_SLOW != 0 {
            0x10
        } else if ctx.joymag > 0.75 {
            0x0D
        } else if ctx.joymag > 0.0 {
            8
        } else {
            1
        };
        if levels & (bits::ATK_QUICK | bits::ATK_SLOW) != 0 {
            self.edge_hist |= levels ^ self.held_hist;
            self.held_hist |= levels;
        } else {
            self.held_hist = 0;
        }
        self.motion = ms;
        ms
    }

    /// The motion state's requested animation (pmotion.c:1811-2089); returns
    /// a message to post when magic is asked for without potions.
    pub fn request(&mut self, ms: u8, ctx: &Ctx) -> Option<usize> {
        let moving = ctx.joymag > 0.0;
        let close = ctx.coll & 2 != 0;
        // Power-ups that replace the swing.
        // forcedAnim (none for an Auto Attack swing).
        let forced = if ctx.forced {
            None
        } else if ctx.special & 0x3000 != 0 {
            Some(p::BREATHE)
        } else if ctx.special & 0x8000 != 0 {
            Some(p::FIREL)
        } else if ctx.special & 0x4000 != 0 {
            Some(p::FIRER)
        } else if ctx.weapon & 0x10_0000 != 0 {
            Some(p::SSHOT)
        } else if ctx.weapon & 0x1000_0000 != 0 {
            Some(p::HAMMER)
        } else if ctx.special & 0x70 != 0 {
            Some(p::BREATHE)
        } else {
            None
        };
        let mut msg = None;
        self.next = match ms {
            2 => p::DEFEND1,
            7 => p::SHOVE,
            9 => p::STRAFE_WLKF,
            0x0A => p::STRAFE_WLKB,
            0x0B => p::STRAFE_WLKL,
            0x0C => p::STRAFE_WLKR,
            0x0D => p::RUN,
            8 => p::WALK,
            0x0F => match forced {
                Some(f) => f,
                None if ctx.coll & 4 != 0 && !close && moving => p::STEP,
                None if ctx.coll & 1 != 0 || ctx.forced => {
                    if close {
                        p::KICK
                    } else {
                        p::QUICK
                    }
                }
                None => p::THROWQ,
            },
            0x10 => match forced {
                Some(f) => f,
                None if ctx.weapon & 0x48_0000 != 0 => p::THROWQ,
                None if self.chain != 0 && moving && ctx.coll & 5 != 0 => p::ATTACK_SLOW,
                None if ctx.coll & 4 != 0 && !close && moving => p::STEP,
                None if ctx.coll & 1 != 0 => {
                    if close {
                        p::LOW
                    } else {
                        p::ATTACK_SLOW
                    }
                }
                None => p::PWRA_THROW,
            },
            0x11..=0x14 => forced.unwrap_or(
                [
                    p::STRAFE_ATKF,
                    p::STRAFE_ATKB,
                    p::STRAFE_ATKL,
                    p::STRAFE_ATKR,
                ][(ms - 0x11) as usize],
            ),
            0x15 => {
                if ctx.special & 0x400 != 0 && ctx.power >= 40.0 {
                    self.coll_score = 40.0;
                    p::BREATHE
                } else if ctx.power >= 100.0 {
                    self.coll_score = 100.0;
                    p::PWRC
                } else if ctx.power >= 40.0 {
                    self.coll_score = 40.0;
                    p::PWRB
                } else {
                    // Not enough meter: the request is left as it was.
                    self.next
                }
            }
            0x16 => {
                self.coll_score = 50.0;
                p::COMBO_ACTIVE1
            }
            0x17 => p::COMBO_ACTIVE2,
            // The partner plays the combo of the hero who started it.
            0x26 => match ctx.partner_ct {
                Some(0) => p::COMBO_WAR1,
                Some(1) => p::COMBO_VAL,
                Some(2) => p::COMBO_WIZ,
                Some(3) => p::COMBO_ARC,
                Some(4) => p::COMBO_DWF1,
                Some(5) => p::COMBO_KNI,
                Some(6) => p::COMBO_SOR,
                Some(7) => p::COMBO_JES,
                _ => p::READY,
            },
            // Thrown by the Warrior, or carrying the Dwarf.
            0x27 => match ctx.partner_ct {
                Some(0) => p::COMBO_WAR2,
                Some(4) => p::COMBO_DWF2,
                _ => p::READY,
            },
            0x18 => {
                if ctx.potions > 0 {
                    p::USE_MAGIC
                } else {
                    // COLLECT POTION BEFORE USING MAGIC; then as if idle.
                    msg = Some(6);
                    if ctx.joymag > 0.75 {
                        p::RUN
                    } else if moving {
                        p::WALK
                    } else {
                        p::READY
                    }
                }
            }
            _ => p::READY,
        };
        // A reaction takes over the motion (pmotion.c:1476-1563, 1838-2089); a
        // plain hit only staggers a hero who is standing, walking or running.
        self.next = match ctx.reaction {
            1 if matches!(ms, 1 | 8 | 0x0D) => p::HIT_REACT,
            2 => p::STUN_REACT,
            3 => p::SPIKE_HIT,
            10 | 11 => p::KNOCKBACK,
            20 => p::FALL_DOWN,
            21 => p::FALL_DOWN_FWD,
            30 | 31 => p::WHIRLWIND,
            _ => self.next,
        };
        msg
    }

    /// DoPlayerAction: from the current and requested animations, what plays
    /// now, and the bits and scales that follow.
    pub fn do_action(&mut self, seqs: &ActionSeqs, ctx: &Ctx, dt: f32) {
        use p::*;
        let next = self.next;
        let cur = self.cur;
        let mut act = next;
        let atk_next = attack_type(next);
        let atk_cur = attack_type(cur);
        let mut d = cur;
        let mut mode: u8 = 0;
        let mut didt = false;
        let mut frame: i32 = 0;
        let mut blend = 0.0f32;
        let anim_frame = self.anim.frame;
        if act >= USE_MAGIC && atk_cur != 0 && atk_cur < 11 {
            d = READY;
        }
        if (DEATHGRAB..ATTACK_SLOW).contains(&act) {
            mode = 2;
        }
        let rpt = if (FALL_DOWN..=GRABBED).contains(&act) {
            mode = 2;
            if act == GRABBED {
                2
            } else {
                1
            }
        } else {
            0
        };
        let atk_d = attack_type(d);
        if atk_d != 0 && atk_d < 11 && atk_next >= 11 {
            d = READY;
            mode = 2;
        }
        if !(2..=6).contains(&atk_d) && atk_d != 8 {
            self.chain = 0;
        }
        let pwra = |s: &Action| -> Option<u16> {
            (s.edge_hist & bits::ATK_SLOW != 0 && s.chain != 0).then_some(match s.chain {
                1 => PWRA_CLOSE,
                2 => PWRA_MED,
                _ => A360,
            })
        };
        let round = |f: f32| (0.5 + f) as i32;
        match d {
            ACTION_INIT => mode = 2,
            READY => {
                didt = true;
                mode = 2;
                if next == READY {
                    if self.idle1 > 1800 {
                        mode = 1;
                        act = IDLE1;
                    } else if self.idle2 > 600 {
                        act = IDLE2;
                        mode = 1;
                    }
                }
            }
            IDLE1 => {
                mode = 2;
                if next == READY {
                    act = READY;
                    mode = 1;
                }
            }
            IDLE2 => {
                mode = 2;
                if next == READY {
                    act = IDLE2_LOOP;
                    mode = 1;
                }
            }
            IDLE2_LOOP => {
                didt = true;
                mode = 2;
                if next == READY {
                    act = IDLE2_LOOP;
                    mode = 0;
                }
            }
            SHIELD_READY => {
                didt = true;
                mode = 2;
            }
            DEFENDL | DEFENDR | DEFENDB | DEFENDF | DEFEND1 | DEFEND2 => {
                mode = 1;
                act = if d == DEFEND1 { DEFEND2 } else { DEFEND3 };
            }
            STRAFE_WLKF..=STRAFE_WLKR2 => {
                if next >= ATTACK_SLOW || atk_next != 0 {
                    mode = 2;
                }
                act = strafe_alternate(d, next, STRAFE_WLKF);
            }
            STRAFE_ATKF..=STRAFE_ATKR2 => act = strafe_alternate(d, next, STRAFE_ATKF),
            PIVOTL | PIVOTR => mode = 2,
            WALK | WALK2 => {
                if next >= HIT_REACT || atk_next != 0 {
                    mode = 2;
                }
                if next == WALK {
                    act = if d == WALK { WALK2 } else { WALK };
                }
            }
            RUN => {
                if next >= HIT_REACT || atk_next != 0 {
                    mode = 2;
                }
                if next == RUN {
                    act = RUN2;
                }
            }
            RUN2 => {
                if next == HIT_REACT || atk_next != 0 {
                    mode = 2;
                }
                if next == RUN {
                    act = RUN;
                }
            }
            SHIELD_RUN => {
                if next >= ATTACK_SLOW || atk_next != 0 {
                    mode = 2;
                }
            }
            SHOVE => {
                didt = true;
                if next >= ATTACK_SLOW {
                    mode = 2;
                }
            }
            ATTACK_SLOW => {
                if atk_next >= 11 || matches!(atk_next, 9 | 10) {
                    mode = 2;
                    frame = round(anim_frame);
                } else {
                    act = ATTACK_SLOW1;
                }
            }
            ATTACK_SLOW1 => act = ATTACK_SLOW1_R,
            ATTACK_SLOW1_R => {
                if atk_next == 1 {
                    mode = 2;
                }
            }
            QUICK | QUICK2 | QUICK3 | STEP | STEP2 | STEP3 => {
                let quick = matches!(d, QUICK | QUICK2 | QUICK3);
                // cur == QUICK2 / STEP2 continue to 3, the others to 2.
                let two = matches!(cur, QUICK2 | STEP2);
                if let Some(a) = pwra(self) {
                    act = a;
                } else if self.edge_hist != 0 || self.held_hist != 0 {
                    act = if ctx.coll & 8 != 0 {
                        match (quick, two) {
                            (true, true) => QUICK2_R,
                            (true, false) => QUICK3_R,
                            (false, true) => STEP2_R,
                            (false, false) => STEP3_R,
                        }
                    } else if ctx.coll & 4 != 0 {
                        if two {
                            STEP3
                        } else {
                            STEP2
                        }
                    } else if two {
                        QUICK3
                    } else {
                        QUICK2
                    };
                } else {
                    act = match (quick, two) {
                        (true, true) => QUICK2_R,
                        (true, false) => QUICK3_R,
                        (false, true) => STEP2_R,
                        (false, false) => STEP3_R,
                    };
                }
            }
            QUICK2_R | QUICK3_R => {
                if let Some(a) = pwra(self) {
                    act = a;
                } else if self.edge_hist != 0 && anim_frame <= 2.0 && ctx.coll & 1 != 0 {
                    act = if cur == QUICK2_R { QUICK3 } else { QUICK2 };
                    mode = 2;
                }
                if atk_next == 1 {
                    mode = 2;
                }
            }
            STEP2_R | STEP3_R => {
                if let Some(a) = pwra(self) {
                    act = a;
                } else if atk_next == 1 {
                    mode = 2;
                }
            }
            Q3TOSTEP1 | Q3TOSTEP1_R | WALK2_ATK | WALK2_ATK_R => {
                if atk_next == 1 {
                    mode = 2;
                }
            }
            RIGHT | RIGHT2 | LEFT | LEFT2 | A180 | A1802 | A180L | A180L2 => {
                act = match d {
                    RIGHT => RIGHT_R,
                    RIGHT2 => RIGHT2_R,
                    LEFT => LEFT_R,
                    LEFT2 => LEFT2_R,
                    A180 => A180_R,
                    A1802 => A1802_R,
                    A180L => A180L_R,
                    _ => A180L2_R,
                };
            }
            RIGHT_R | RIGHT2_R | LEFT_R | LEFT2_R | A180_R | A1802_R | A180L_R | A180L2_R
            | A360_R => {
                if atk_next == 1 {
                    mode = 2;
                }
            }
            A360 => {
                act = if self.edge_hist & bits::ATK_SLOW != 0 && self.chain != 0 {
                    PWRA_MED
                } else {
                    A360_R
                };
            }
            PWRA_LOW => act = PWRA_LOW_R,
            PWRA_CLOSE => act = PWRA_CLOSE_R,
            PWRA_MED => act = PWRA_MED_R,
            PWRA_THROW => act = PWRA_THROW_R,
            PWRB | PWRC => {}
            COMBO_ACTIVE1 => {
                mode = 1;
                if seqs.has(COMBO_ACTIVE2) {
                    act = COMBO_ACTIVE2;
                } else if seqs.has(COMBO_ACTIVE3) {
                    act = COMBO_ACTIVE3;
                } else if rpt < 2 {
                    mode = 0;
                }
            }
            COMBO_ACTIVE3 | COMBO_WAR1 | COMBO_WAR3 | COMBO_VAL | COMBO_WIZ | COMBO_ARC
            | COMBO_DWF1 | COMBO_DWF3 | COMBO_KNI | COMBO_SOR | COMBO_JES => {}
            COMBO_ACTIVE2 | COMBO_WAR2 | COMBO_DWF2 => {
                didt = true;
                if next != d {
                    if seqs.has(d + 1) {
                        act = d + 1;
                    }
                    mode = 2;
                }
            }
            PWRA_CLOSE_R | PWRA_MED_R | PWRA_LOW_R | PWRA_THROW_R => {}
            LOW | LOW2 => {
                act = if next == LOW {
                    if cur == LOW {
                        LOW2
                    } else {
                        LOW
                    }
                } else {
                    LOW_R
                };
            }
            LOW_R => {
                if atk_next == 1 {
                    mode = 2;
                } else if next == LOW {
                    act = if seqs.has(LOW2) { LOW2 } else { LOW };
                } else {
                    mode = 1;
                }
            }
            KICK => act = pwra(self).map_or(KICK_R, |_| PWRA_LOW),
            KICK_R => {
                if pwra(self).is_some() {
                    act = PWRA_LOW;
                } else if atk_next == 1 {
                    mode = 2;
                } else {
                    mode = 1;
                }
            }
            THROW | THROWQ | THROW2 | THROW2Q => {
                let dance = matches!(d, THROW2 | THROW2Q);
                if (atk_next > 1 && atk_next != 9 && atk_next != 10) || atk_next == 7 {
                    mode = 2;
                    frame = round(anim_frame);
                } else if next == USE_MAGIC || next == THROW_MAGIC {
                    mode = 2;
                } else if next == THROW_STEP {
                    mode = 2;
                    frame = round(anim_frame);
                } else {
                    act = if dance { THROW2_RELEASE } else { THROW_RELEASE };
                    if next != THROW && anim_frame >= 2.0 {
                        mode = 2;
                    }
                }
            }
            THROW_RELEASE | THROW2_RELEASE => {
                act = if cur == THROW_RELEASE {
                    THROW_RECOVER
                } else {
                    THROW2_RECOVER
                };
            }
            THROW_RECOVER | THROW2_RECOVER => {
                if atk_next == 1 {
                    mode = 2;
                }
            }
            SSHOT | SSHOT2 => {
                if next == SSHOT {
                    act = SSHOT2;
                    didt = true;
                } else if atk_next != 0 {
                    mode = 1;
                } else {
                    act = SSHOT_R;
                }
            }
            BREATHE => act = BREATHE_R,
            HAMMER => act = HAMMER_R,
            FIREL => act = FIREL_R,
            FIRER => act = FIRER_R,
            USE_MAGIC => {
                // action.c:1168: on to the throw unless field_956 & 4 holds
                // the magic in place, which nothing sets: every potion is
                // thrown (0x800ACDC0: lha 0x956; rlwinm. 0,29,29).
                if next == THROW_MAGIC {
                    act = THROW_MAGIC;
                    mode = 2;
                } else {
                    act = THROW_MAGIC;
                    mode = 0;
                }
            }
            THROW_MAGIC => act = THROW_MAGIC_RELEASE,
            VICTORY => didt = true,
            PUSH | PUSHED => {
                if next != READY {
                    mode = 2;
                    didt = true;
                }
            }
            ACTION_DEATH | GET_UP | GET_UP_FWD | PICKUP => {
                if next == cur {
                    act = READY;
                }
            }
            HIT_REACT => {
                mode = if act >= KNOCKBACK { 3 } else { 2 };
                if matches!(next, READY | WALK | RUN) {
                    mode = 0;
                }
            }
            STUN_REACT | SPIKE_HIT | KNOCKBACK => {
                if next >= FALL_DOWN {
                    mode = 3;
                }
                if next == cur {
                    act = READY;
                }
            }
            FALL_DOWN | FALL_DOWN_FWD | WHIRLWIND => {
                if next != d {
                    act = if d == FALL_DOWN_FWD {
                        GET_UP_FWD
                    } else {
                        GET_UP
                    };
                    mode = 1;
                }
            }
            GRABBED => {
                mode = 2;
                didt = true;
            }
            _ => {}
        }

        // Direction and follow-up refinement of the chosen action.
        let ang = ctx.melee_yaw;
        let dir3 = |a: u16| -> u16 {
            if ang > 2.356_194_5 {
                A180
            } else if ang < -2.356_194_5 {
                A180L
            } else if ang > std::f32::consts::FRAC_PI_3 {
                RIGHT
            } else if ang < -std::f32::consts::FRAC_PI_3 {
                LEFT
            } else {
                a
            }
        };
        match act {
            READY => {
                if ctx.shield & 0x62_0000 != 0 {
                    act = SHIELD_READY;
                }
                if cur != READY
                    && !(PWRB..=COMBO_JES).contains(&cur)
                    && cur != HIT_REACT
                    && !(SPIKE_HIT..=KNOCKBACK).contains(&cur)
                    && !(ctx.char_type == 3 && cur == QUICK2_R)
                {
                    blend = 0.066_667;
                }
            }
            WALK | WALK2 | RUN | RUN2 => {
                if ctx.shield & 0x62_0000 != 0 {
                    act = SHIELD_RUN;
                    if cur == SHIELD_RUN {
                        mode = 0;
                    }
                    didt = true;
                }
            }
            QUICK | QUICK3 => act = dir3(act),
            QUICK2 | STEP2 => {
                act = if ang > 2.356_194_5 {
                    A1802
                } else if ang < -2.356_194_5 {
                    A180L2
                } else if ang > std::f32::consts::FRAC_PI_3 {
                    RIGHT2
                } else if ang < -std::f32::consts::FRAC_PI_3 {
                    LEFT2
                } else {
                    act
                };
            }
            THROW | THROWQ | THROW_STEP => {
                if cur == WALK || cur == RUN {
                    act = match act {
                        THROW => THROW2,
                        THROWQ => THROW2Q,
                        _ => THROW_STEP2,
                    };
                }
            }
            STEP => {
                if cur == WALK2 || cur == RUN2 {
                    act = WALK2_ATK;
                } else if cur == QUICK || cur == QUICK3 {
                    act = Q3TOSTEP1;
                }
                act = dir3(act);
            }
            STEP3 => act = dir3(act),
            PWRA_CLOSE => {
                if ctx.coll & 2 != 0 {
                    act = PWRA_LOW;
                }
            }
            _ => {}
        }

        // The sequence, falling back on PWRA_CLOSE for a missing PWRA_LOW.
        let mut d = act;
        if act == PWRA_LOW && !seqs.has(act) {
            d = PWRA_CLOSE;
        }
        if act == PWRA_LOW_R && !seqs.has(act) {
            d = PWRA_CLOSE_R;
        }
        self.anim.repeat = didt;
        let seq = seqs.get(d);

        // Per-action animation speed (the next InitAnim uses it).
        self.anim.animscale = if self.hud & 0xD0 != 0 || atk_next >= 11 || atk_next == 1 {
            1.0
        } else if (COMBO_ACTIVE1..=COMBO_ACTIVE3).contains(&act)
            || (COMBO_WAR1..=COMBO_JES).contains(&act)
        {
            1.0
        } else if ctx.weapon & 0x2000_0000 != 0 && matches!(atk_next, 9 | 10) {
            0.75
        } else if act == DEFEND2 {
            (0.2 * ctx.stat_armor).max(0.25)
        } else if ctx.special & 0x10000 != 0 {
            0.75
        } else {
            1.0
        };

        let adv = self.anim.animate(dt, blend, seq, frame, mode) != 0;
        self.action = attack_type(if adv { act } else { cur });

        if adv {
            self.prev_frame = -1.0;
            match cur {
                IDLE1 => {
                    self.idle1 = 0;
                    self.idle2 = 1;
                }
                IDLE2_LOOP if act != IDLE2_LOOP => {
                    self.idle1 = 0;
                    self.idle2 = 0;
                }
                QUICK | QUICK2 | QUICK3 | RIGHT | RIGHT2 | LEFT | LEFT2 | A180 | A1802 | A180L
                | A180L2 | A360 | LOW | LOW2 => self.act_bits |= act::HIT,
                ATTACK_SLOW1 | STEP | STEP2 | STEP3 | Q3TOSTEP1 | WALK2_ATK => {
                    self.act_bits |= act::HIT2
                }
                KICK => self.act_bits |= act::KICK,
                PWRA_LOW => self.act_bits |= act::HIT3,
                PWRA_CLOSE | PWRA_MED if ctx.char_type != 6 => self.act_bits |= act::HIT3,
                PWRA_THROW if ctx.char_type != 6 => self.act_bits |= act::PWRA_THROW,
                STRAFE_ATKF..=STRAFE_ATKR2
                | THROW_RELEASE
                | THROW2_RELEASE
                | THROW_STEP
                | THROW_STEP2 => self.act_bits |= act::THROW,
                SSHOT | SSHOT2 if matches!(act, SSHOT2 | SSHOT_R) => {
                    self.act_bits |= act::SUPER_SHOT
                }
                FIREL if act == FIREL_R => self.act_bits |= act::FIREL,
                FIRER if act == FIRER_R => self.act_bits |= act::FIRER,
                _ => {}
            }
            self.hud &= !0xC702;
            match act {
                ATTACK_SLOW | ATTACK_SLOW1 | QUICK | QUICK2 | QUICK3 | RIGHT | RIGHT2 | LEFT
                | LEFT2 | A180 | A1802 | A180L | A180L2 | A360 | STEP | STEP2 | STEP3
                | Q3TOSTEP1 | WALK2_ATK | LOW | LOW2 | KICK => {
                    self.act_bits |= act::SWING;
                    self.chain = if self.edge_hist != 0 {
                        self.chain + 1
                    } else {
                        0
                    };
                    self.edge_hist = 0;
                }
                THROW | THROWQ | THROW2 | THROW2Q | THROW_STEP | THROW_STEP2 | SSHOT | SSHOT2 => {
                    self.act_bits |= act::SWING;
                    self.edge_hist = 0;
                }
                USE_MAGIC | THROW_MAGIC => {
                    self.act_bits |= act::USE_MAGIC;
                    self.edge_hist = 0;
                }
                MAGIC_RELEASE => {
                    self.act_bits |= act::MAGIC_RELEASE;
                    self.edge_hist = 0;
                }
                THROW_MAGIC_RELEASE => {
                    self.act_bits |= act::THROW_MAGIC_RELEASE;
                    self.edge_hist = 0;
                }
                BREATHE => {
                    self.act_bits |= act::BREATHE;
                    self.edge_hist = 0;
                }
                HAMMER_R => {
                    self.act_bits |= act::HAMMER;
                    self.edge_hist = 0;
                }
                VICTORY => self.hud |= 0x802,
                ACTION_DEATH | FALL_DOWN | GET_UP | FALL_DOWN_FWD | GET_UP_FWD | WHIRLWIND => {
                    self.hud |= 2
                }
                DEFEND1 => self.hud |= hud::DEFEND1,
                DEFENDL | DEFENDR | DEFENDB | DEFENDF | DEFEND2 => self.hud |= hud::BLOCK,
                STRAFE_WLKF..=STRAFE_WLKR2 => self.hud |= hud::STRAFE_WALK,
                STRAFE_ATKF..=STRAFE_ATKR2 => {
                    self.hud |= hud::STRAFE_ATTACK;
                    self.act_bits |= act::SWING;
                    self.edge_hist = 0;
                }
                SHOVE => {
                    self.hud |= hud::CHARGE;
                    self.edge_hist = 0;
                }
                ATTACK_SLOW1_R | QUICK2_R | QUICK3_R | RIGHT_R | RIGHT2_R | LEFT_R | LEFT2_R
                | A180_R | A1802_R | A180L_R | A180L2_R | A360_R | STEP2_R | STEP3_R
                | Q3TOSTEP1_R | WALK2_ATK_R | LOW_R | KICK_R => {}
                // THROW_RECOVER(2) and PWRA_THROW_R only hide the held weapon.
                THROW_RECOVER | THROW2_RECOVER | PWRA_THROW_R => {}
                _ => self.edge_hist = 0,
            }
        }

        self.scales(cur, ctx.char_type, anim_frame);
        self.cur = if adv { act } else { cur };
    }

    /// Per-action move / turn / stick scales, keyed by the animation that was
    /// current (action.c:1707-1827).
    fn scales(&mut self, cur: u16, ct: u8, frame: f32) {
        use p::*;
        self.stick_scale = 1.0;
        let (m, t) = if (ATTACK_SLOW..LAST_ATTACK).contains(&cur) {
            if cur >= SSHOT {
                (0.0, 0.5)
            } else if cur >= FIREL {
                (0.25, 1.0)
            } else if cur >= THROW_STEP {
                (1.0, 1.0)
            } else if cur >= PWRA_THROW {
                (0.25, 1.0)
            } else if cur >= THROW {
                (0.0, 0.5)
            } else if cur >= COMBO_ACTIVE1 {
                (0.0, 0.0)
            } else if cur >= PWRC {
                if ct == 6 && frame > 11.0 {
                    (0.0, 0.0)
                } else {
                    (0.0, 0.25)
                }
            } else if cur >= PWRB {
                (0.0, 1.0)
            } else if cur >= PWRA_LOW {
                (0.25, 1.0)
            } else if cur >= LOW {
                (1.0, 1.0)
            } else if cur >= STRAFE_ATKF {
                (0.667, 1.0)
            } else if cur >= STEP {
                (1.0, 0.25)
            } else if cur >= A360 {
                (0.5, 1.0)
            } else if cur >= RIGHT {
                (1.0, 1.0)
            } else if cur >= QUICK {
                (0.25, 0.0)
            } else if cur == PWRA_MED {
                match ct {
                    6 | 7 => (0.0, 0.0),
                    2 | 3 => (0.25, 1.0),
                    _ => (0.5, 1.0),
                }
            } else if cur >= PWRA_CLOSE {
                match ct {
                    5 | 6 => (0.0, 0.0),
                    2 => (0.25, 1.0),
                    _ => (0.5, 1.0),
                }
            } else {
                (0.0, 1.0)
            }
        } else if cur == STUCK {
            (0.4, 0.5)
        } else if matches!(cur, RUN | RUN2 | SHIELD_RUN) {
            (1.3, 1.0)
        } else if cur == COMBO_DWF2 {
            (1.5, 0.5)
        } else if cur == SHOVE {
            (1.5, 1.0)
        } else if (STRAFE_WLKF..=STRAFE_WLKR2).contains(&cur) {
            (0.667, 1.0)
        } else if cur > LAST_ATTACK && cur != VICTORY {
            // (The shipped DEFEND test, 119..7, is an empty range: blocking
            // lands here too.)
            self.stick_scale = 0.0;
            (1.0, 1.0)
        } else {
            (1.0, 1.0)
        };
        self.move_scale = m;
        self.turn_scale = t;
    }

    /// player.c: idle ticks count while the motion state is idle.
    pub fn tick_idle(&mut self, ticks: u32) {
        if self.motion == 1 {
            if self.idle2 == 0 {
                self.idle1 += ticks;
            } else {
                self.idle2 += ticks;
            }
        } else {
            self.idle1 = 0;
            self.idle2 = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn war() -> Option<ActionSeqs> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game-data");
        let fs = crate::fs::GameFs::open(&root).ok()?;
        let d = fs.read("PLAYERS/WAR/ANIM/ANIM.PS2").ok()?;
        let anim = gdl_formats::anim::Anim::parse(&d).ok()?;
        Some(ActionSeqs::new(anim.atrees.first()?))
    }

    /// Run `ticks` with the A / Y buttons as given; the animations that played.
    fn run(a: &mut Action, s: &ActionSeqs, ctx: &Ctx, input: &[(u32, u32)]) -> Vec<u16> {
        let mut played = vec![];
        for &(levels, edges) in input {
            let ms = a.motion_state(levels, edges, ctx);
            a.request(ms, ctx);
            a.do_action(s, ctx, 1.0 / 60.0);
            a.act_bits = 0;
            if played.last() != Some(&a.cur) {
                played.push(a.cur);
            }
        }
        played
    }

    #[test]
    fn names_resolve_for_the_warrior() {
        let Some(s) = war() else { return };
        for a in [
            p::QUICK,
            p::QUICK2,
            p::A360,
            p::PWRA_CLOSE,
            p::THROWQ,
            p::DEFEND2,
        ] {
            assert!(s.has(a), "{}", NAMES[a as usize]);
        }
    }

    #[test]
    fn held_attack_chains_quick_swings() {
        let Some(s) = war() else { return };
        let ctx = Ctx {
            coll: 1,
            ..Default::default()
        };
        let mut a = Action::new();
        let mut input = vec![(bits::ATK_QUICK, bits::ATK_QUICK)];
        input.extend(std::iter::repeat_n((bits::ATK_QUICK, 0), 80));
        let played = run(&mut a, &s, &ctx, &input);
        assert_eq!(
            &played[..4],
            &[p::QUICK, p::QUICK2, p::QUICK3, p::QUICK2],
            "{played:?}"
        );
    }

    #[test]
    fn a_then_y_is_a_power_swing() {
        let Some(s) = war() else { return };
        let ctx = Ctx {
            coll: 1,
            ..Default::default()
        };
        let mut a = Action::new();
        // A, then Y pressed during the first swing.
        let mut input = vec![(bits::ATK_QUICK, bits::ATK_QUICK)];
        input.extend(std::iter::repeat_n((bits::ATK_QUICK, 0), 3));
        input.push((bits::ATK_SLOW, bits::ATK_SLOW));
        input.extend(std::iter::repeat_n((bits::ATK_SLOW, 0), 60));
        let played = run(&mut a, &s, &ctx, &input);
        assert!(played.contains(&p::PWRA_CLOSE), "{played:?}");
    }

    #[test]
    fn nothing_in_reach_throws() {
        let Some(s) = war() else { return };
        let ctx = Ctx {
            coll: 8,
            ..Default::default()
        };
        let mut a = Action::new();
        let mut bitsseen = 0;
        let mut input = vec![(bits::ATK_QUICK, bits::ATK_QUICK)];
        input.extend(std::iter::repeat_n((0, 0), 40));
        for &(l, e) in &input {
            let ms = a.motion_state(l, e, &ctx);
            a.request(ms, &ctx);
            a.do_action(&s, &ctx, 1.0 / 60.0);
            bitsseen |= a.act_bits;
            a.act_bits = 0;
        }
        assert!(bitsseen & act::THROW != 0);
        assert_eq!(a.cur, p::READY);
    }

    #[test]
    fn block_lasts_by_armour() {
        let Some(s) = war() else { return };
        let ctx = Ctx {
            stat_armor: 2.0,
            ..Default::default()
        };
        let mut a = Action::new();
        let mut input = vec![(bits::DEFEND | bits::TURBO, bits::DEFEND | bits::TURBO)];
        input.extend(std::iter::repeat_n((0, 0), 120));
        let played = run(&mut a, &s, &ctx, &input);
        assert_eq!(
            &played[..3],
            &[p::DEFEND1, p::DEFEND2, p::DEFEND3],
            "{played:?}"
        );
    }
}
