//! Monster AI: do_ai (enemy.c) and the move_logic routines it dispatches each
//! monster to, with their wall probes (fn_8004C8CC), corner hugging (the
//! blocked00/07/10/20 responses of do_enemy_move) and milestone pathing
//! (update_player_milestone, update_enemy_milestone, find_neighbor_milestone).
//!
//! A routine works in 60 Hz ticks on a [`Brain`] (the Enemy record's AI
//! fields) and yields the frame's translation, facing and requested action
//! ([`Out`]); the game commits the move and reports what stopped it back
//! through [`Brain::after_move`], as do_enemy_move does.

use crate::math::Det as _;
use glam::Vec3;
use std::f32::consts::{PI, TAU};

/// enemy_action_type (enemy.h): the actions a routine requests.
pub mod act {
    pub const READY: u8 = 0;
    pub const START: u8 = 1;
    pub const WALK: u8 = 3;
    pub const RUN: u8 = 4;
    pub const READYTORUN: u8 = 9;
    pub const ATTACK: u8 = 12;
    pub const ATTACK2: u8 = 14;
    pub const ATTACK_PWR: u8 = 16;
    pub const RUNATTACK: u8 = 22;
    pub const RUNATTACK2: u8 = 23;
    pub const THROW: u8 = 24;
    pub const THROW_FINISH: u8 = 26;
    pub const HIT_REACT1: u8 = 28;
}

/// e_actpri (action.c): a request replaces the frame's earlier one only when
/// it ranks higher.
const ACTPRI: [u16; 33] = [
    100, 900, 900, 200, 200, 200, 200, 200, 200, 200, 200, 200, 300, 300, 300, 300, 300, 300, 300,
    300, 300, 300, 300, 300, 300, 300, 300, 150, 400, 450, 460, 900, 999,
];

/// sEnemyDefaultAlgorithm (enemy.c): each type's routine when none is given.
pub const DEFAULT_ALG: [i32; 34] = [
    2, 7, 7, 2, 7, 7, 7, 7, 7, 7, 7, 7, 2, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 0, 19, 3,
    27, 7, 7,
];

/// lbl_8011BED8: the turn rate a tick (every type but the unused 28).
const TURN_RATE: f32 = 0.049_087_387;
/// lbl_8011C0C4: move_logic00's heading sweep, added once per try.
const SWEEP00: [f32; 10] = [
    0.0,
    0.392_699_1,
    0.392_699_1,
    0.392_699_1,
    0.392_699_1,
    0.392_699_1,
    0.392_699_1,
    0.392_699_1,
    0.392_699_1,
    0.392_699_1,
];
/// lbl_8011C044..lbl_8011C0A4: the corner-hug offsets by collision count
/// (0 to 157.5 degrees in eighths of pi).
const HUG: [f32; 8] = [
    0.0,
    0.392_699_1,
    0.785_398_2,
    1.178_097_2,
    1.570_796_4,
    1.963_495_4,
    2.356_194_5,
    2.748_893_5,
];
/// Two degrees: the heading-flip guard of the sweeps.
const FLIP: f32 = 0.034_906_585;

/// lbl_8011BF60: the leap / retreat deflections, +-5 degrees growing by 5.
fn leap(i: i32) -> f32 {
    let a = (i / 2 + 1) as f32 * 5.0_f32.to_radians();
    if i & 1 == 0 {
        a
    } else {
        -a
    }
}

/// The game's one-step heading wrap into (-pi, pi].
pub fn wrap(a: f32) -> f32 {
    if a > PI {
        a - TAU
    } else if a <= -PI {
        a + TAU
    } else {
        a
    }
}

/// get_yaw: the heading from `from` to `to`.
pub fn yaw_to(to: Vec3, from: Vec3) -> f32 {
    (to.x - from.x).det_atan2(to.z - from.z)
}

fn flat(a: Vec3, b: Vec3) -> f32 {
    ((a.x - b.x).det_powi(2) + (a.z - b.z).det_powi(2)).sqrt()
}

/// RandInt.
pub trait Dice {
    /// 0..n (0 when n is 0).
    fn int(&mut self, n: u32) -> i32;
}

impl Dice for crate::game::Rng {
    fn int(&mut self, n: u32) -> i32 {
        if n == 0 {
            0
        } else {
            (self.next_u32() % n) as i32
        }
    }
}

/// The world queries the routines make.
pub trait Probe {
    /// FastWallCollide: a wall between the two points.
    fn wall(&self, from: Vec3, to: Vec3) -> bool;
    /// fn_8004C8CC's other half: another monster (fn_8004646C) or a solid item
    /// (fn_8005EFAC) in the way of a cylinder swept from `from` to `to`.
    fn occupied(&self, slot: usize, from: Vec3, to: Vec3, rad: f32, hht: f32) -> bool;
    /// Where a monster (by entity id) stands, if it still lives.
    fn enemy_pos(&self, id: u64) -> Option<Vec3>;
}

/// The level as the routines probe it: its walls, the live monsters and the
/// solid items (barrels, doors, generators).
pub struct WorldProbe<'a> {
    pub coll: &'a crate::collision::Collision,
    /// (slot, id, feet, radius, half-height) of each live monster.
    pub enemies: &'a [(usize, u64, Vec3, f32, f32)],
    /// (position, radius) of each solid item.
    pub solids: &'a [(Vec3, f32)],
}

/// The horizontal distance from `p` to the segment `a`-`b`.
fn seg_dist(p: Vec3, a: Vec3, b: Vec3) -> f32 {
    let ab = glam::Vec2::new(b.x - a.x, b.z - a.z);
    let ap = glam::Vec2::new(p.x - a.x, p.z - a.z);
    let t = if ab.length_squared() > 1e-8 {
        (ap.dot(ab) / ab.length_squared()).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (ap - ab * t).length()
}

impl Probe for WorldProbe<'_> {
    fn wall(&self, from: Vec3, to: Vec3) -> bool {
        self.coll
            .raycast_hit(from, to)
            .is_some_and(|(_, n)| n.y < 0.5)
    }

    fn occupied(&self, slot: usize, from: Vec3, to: Vec3, rad: f32, hht: f32) -> bool {
        // LineCylinderCollide against each other monster, as the step goes
        // in (one already touching may be stepped away from); the probe
        // runs at rad above the feet.
        let feet = from.y - rad;
        let into = |p: Vec3, reach: f32| {
            let d = seg_dist(p, from, to);
            d < reach && flat(p, to) < flat(p, from)
        };
        self.enemies
            .iter()
            .any(|&(s, _, p, r, h)| s != slot && (p.y - feet).abs() < hht + h && into(p, rad + r))
            || self
                .solids
                .iter()
                .any(|&(p, r)| (p.y - feet).abs() < 4.0 && into(p, rad + r))
    }

    fn enemy_pos(&self, id: u64) -> Option<Vec3> {
        self.enemies.iter().find(|e| e.1 == id).map(|e| e.2)
    }
}

/// A player as the routines see it.
#[derive(Clone, Copy, Debug, Default)]
pub struct Seen {
    pub pos: Vec3,
    /// state == ACTIVE.
    pub active: bool,
    /// Invisible (flags & 4): never chosen.
    pub hidden: bool,
    /// The anti-death halo (shield_flags 0x80000): Death keeps away.
    pub halo: bool,
    /// The hero's Mikey while it is out (field_A1C > 2): monsters measure to
    /// and steer at it instead (calc_enemy_to_player_distance, mikey_worldmat).
    pub decoy: Option<Vec3>,
}

impl Seen {
    /// Where monsters go after this hero: its Mikey while one is out.
    pub fn target(&self) -> Vec3 {
        self.decoy.unwrap_or(self.pos)
    }
}

/// The suicide grunt running at the party (lbl_80344748).
#[derive(Clone, Copy, Debug)]
pub struct Bomber {
    pub slot: usize,
    pub pos: Vec3,
    pub actual_dist: f32,
}

/// What a frame's routines share.
pub struct Frame<'a> {
    pub players: &'a [Seen],
    /// The player who is IT (lbl_80344B24): everyone goes after them.
    pub it: Option<usize>,
    pub bomber: Option<Bomber>,
    pub paths: &'a Paths,
    /// lbl_80344800: the frame counter (closest players are re-chosen every
    /// eighth frame, by slot).
    pub frame: u32,
    /// gFrameTicks.
    pub ticks: i32,
    /// ene_mrate.
    pub mrate: f32,
}

/// The monster itself this frame.
#[derive(Clone, Copy, Debug)]
pub struct Me {
    /// Its slot (the parity picks the wander direction; the low three bits the
    /// frame its closest player is re-chosen on).
    pub slot: usize,
    pub etype: usize,
    pub pos: Vec3,
    /// pyr[1], its facing.
    pub yaw: f32,
    pub rad: f32,
    pub hht: f32,
    /// lbl_80250E40: its move a tick (type speed x ene_speed).
    pub spd: f32,
    /// visactive: on screen, with a margin.
    pub visactive: bool,
    /// The current action when it is not free to walk (attacking, throwing,
    /// hurt, appearing); None when it walks or stands.
    pub busy: Option<u8>,
    /// Throws refused (idle_secs > 0).
    pub reloading: bool,
    /// Spawned by a generator that still stands.
    pub has_generator: bool,
    /// That generator's gen.flags is 7: its logic-12 eggs hatch.
    pub hatch: bool,
}

/// A routine's result.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Out {
    /// The move this frame (trans), before collisions.
    pub trans: Vec3,
    /// The new facing (pyr[1]).
    pub yaw: f32,
    /// The action requested (daction).
    pub daction: u8,
    /// Frozen this frame (out of sight and off screen: fn_8004D958).
    pub frozen: bool,
    /// The suicide grunt blows up (damage_enemy 999).
    pub explode: bool,
    /// move_logic13 gave up its chain: these monsters go back to chasing.
    pub release: Option<u64>,
}

/// What stopped a committed move (do_enemy_move / do_enemy_collide).
#[derive(Clone, Copy, Debug, Default)]
pub struct Report {
    /// It tried to move (moveDistance > 0.001).
    pub tried: bool,
    /// Walls or a ledge left it (nearly) where it was.
    pub wall: bool,
    /// Touching this player (coll_pnum): where they stand.
    pub player: Option<Vec3>,
    /// Ran into another monster: where it stands.
    pub enemy: Option<Vec3>,
    /// Ran into a solid item (coll_ip): where it stands.
    pub item: Option<Vec3>,
}

/// The routine-specific checks after the move (in the routines after their
/// do_enemy_move call).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Post {
    #[default]
    None,
    /// 03/16/18/21/24/29: a move clears dead_end; a fresh block remembers the
    /// heading and restarts the deflections.
    Corner { stuck: bool },
    /// 18: the run lasts 240 ticks or until it touches someone.
    Suicide { stuck: bool },
}

/// The spawn bookkeeping gendata.ai drives (items.c fn_800606FC): ai 13
/// chains each spawn to the one before (tail), ai 14 alternates them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GenAi {
    pub tail: Option<u64>,
    /// num_generated (ai 12 counts only what a hit egg sets: 3).
    pub generated: u32,
    /// gen.flags: logic 12's eggs laid (1 left, 2 right, 4 in place); 7
    /// hatches them.
    pub flags: u8,
}

/// The Enemy record's AI fields (init_enemy_vars / format_brain).
#[derive(Clone, Debug, PartialEq)]
pub struct Brain {
    pub algorithm: i32,
    pub old_ai: i32,
    pub prev_ai: i32,
    /// The generate_enemy spew still to resolve on the first tick.
    spew: Option<i32>,
    /// The algorithm the item code already settled (Kind::Enemy.ai).
    preset: Option<i32>,
    /// org_lvl: the tier it was made at (placed archers 4..6).
    pub level: i32,
    pub ang: f32,
    angbak: f32,
    anghit: f32,
    /// lbl_80344720: the bearing a routine worked out this frame.
    face: f32,
    pub route: i32,
    pub collided: i32,
    pub stuck_count: i32,
    pub dead_end: i32,
    /// What stopped the last move: 0 nothing, 1 a wall, 2 a monster, 3 an item.
    pub area: u8,
    count: i32,
    play: i32,
    counter1: i32,
    counter2: i32,
    flag1: i32,
    flag2: i32,
    mode1: i32,
    mode2: i32,
    lost: i32,
    pub closest: Option<usize>,
    prev_closest: Option<usize>,
    pub close_dist: f32,
    pub actual_dist: f32,
    pub sight: f32,
    pub recognized: bool,
    view: f32,
    ai_flags: u8,
    /// 1 for placed monsters (they never flee the suicide grunt).
    pub birth_style: u8,
    pub plr_ms: i32,
    ms_idx: i32,
    max_msidx: i32,
    operation_count: i32,
    operation_speed: i32,
    /// The zombie chain (prev_enemy / next_enemy), by entity id.
    pub prev: Option<u64>,
    pub next: Option<u64>,
    /// idle_time: scales the throw cooldown (idle_time x ene_mrate).
    pub idle_time: f32,
    /// The generator that made it (entity id).
    pub generator: Option<u64>,
    daction: u8,
    /// The last free action (walk, run, ready ...) it was granted.
    free_act: u8,
    coll_pnum: Option<usize>,
    coll_enemy: bool,
    coll_item: bool,
    moved: bool,
    /// The algorithm that made this frame's move (its blocked responses).
    move_alg: i32,
    post: Post,
    explode: bool,
    /// The frame's translation and facing.
    trans: Vec3,
    yaw: f32,
}

impl Brain {
    /// init_enemy_vars for a monster of `etype` made at `level` with
    /// generate_enemy's spew (the item's ai; -1 for its type's default),
    /// facing `yaw`. The algorithm is settled on the first tick.
    pub fn new(level: i32, spew: i32, yaw: f32, sight: f32) -> Brain {
        Brain {
            algorithm: -1,
            old_ai: -1,
            prev_ai: -1,
            spew: Some(spew),
            preset: None,
            level,
            ang: yaw,
            angbak: yaw,
            anghit: 0.0,
            face: yaw,
            route: 0,
            collided: 0,
            stuck_count: 0,
            dead_end: 0,
            area: 0,
            count: 0,
            play: 0,
            counter1: 0,
            counter2: 0,
            flag1: 0,
            flag2: 0,
            mode1: 0,
            mode2: 0,
            lost: 0,
            closest: None,
            prev_closest: None,
            close_dist: 1.0,
            actual_dist: 1.0,
            sight,
            recognized: false,
            view: 3.159_046,
            ai_flags: 0,
            birth_style: 0,
            plr_ms: -1,
            ms_idx: 0,
            max_msidx: 4,
            operation_count: 0,
            operation_speed: 8,
            prev: None,
            next: None,
            idle_time: 1.0,
            generator: None,
            daction: act::READY,
            free_act: act::READY,
            coll_pnum: None,
            coll_enemy: false,
            coll_item: false,
            moved: false,
            move_alg: -1,
            post: Post::None,
            explode: false,
            trans: Vec3::ZERO,
            yaw,
        }
    }

    /// With the algorithm given outright (tests, scripted monsters).
    pub fn with_algorithm(mut self, alg: i32) -> Brain {
        self.spew = None;
        self.algorithm = alg;
        self.old_ai = alg;
        self.prev_ai = alg;
        self
    }

    /// The algorithm it runs (once settled).
    pub fn alg(&self) -> i32 {
        self.algorithm
    }

    /// mode1: logic 12's egg is laid (1) or hatched (2).
    pub fn mode1(&self) -> i32 {
        self.mode1
    }

    /// Take the algorithm init_enemy settled (until the first tick).
    pub fn adopt(&mut self, alg: i32) {
        if self.spew.is_some() {
            self.preset = Some(alg);
        }
    }

    /// Its heading as it appears (e->ang = angbak = pyr[1]).
    pub fn set_heading(&mut self, yaw: f32) {
        self.ang = yaw;
        self.angbak = yaw;
        self.face = yaw;
        self.yaw = yaw;
    }

    /// Generator ai 14: alternate spawns veer 45 degrees either way and
    /// zig-zag starting that way.
    pub fn zigzag(&mut self, odd: bool) {
        let (d, f) = if odd {
            (-0.785_398_2, -1)
        } else {
            (0.785_398_2, 1)
        };
        self.ang = wrap(self.ang + d);
        self.angbak = self.ang;
        self.flag1 = f;
    }

    /// move_logic13 cut it loose: it chases from now on.
    pub fn release(&mut self) {
        self.algorithm = 7;
        self.old_ai = 7;
        self.spew = None;
        self.next = None;
        self.prev = None;
    }

    /// Walls slide its moves (SlideAlongWall unless ai_flags & 1: the
    /// wanderers stop dead instead).
    pub fn slides(&self) -> bool {
        self.ai_flags & 1 == 0
    }

    /// The suicide grunt running at the party (lbl_80344748's test).
    pub fn bombing(&self) -> bool {
        self.algorithm == 18 && self.free_act == act::RUN
    }

    /// fn_8004E448: the missile its throws and casts fire (0 arrow, 1 bomb,
    /// 2 fireball).
    pub fn missile_kind(&self) -> usize {
        match self.algorithm {
            16 | 23 => 0,
            17 | 26 => 1,
            _ => 2,
        }
    }

    /// fn_8004DF58: attacks that launch a fireball when not touching anyone.
    pub fn casts(&self) -> bool {
        matches!(self.algorithm, 28..=31)
    }

    /// One frame of do_enemies for this monster: the closest player
    /// (fn_800516F8), the freeze test (fn_8004D958), then do_ai.
    pub fn think(
        &mut self,
        me: &Me,
        f: &Frame,
        offsets: &mut [f32],
        probe: &dyn Probe,
        dice: &mut dyn Dice,
        contact: Option<usize>,
    ) -> Out {
        if let Some(spew) = self.spew.take() {
            let alg = match self.preset.take() {
                Some(a) => a,
                None => spawn_algorithm(me.etype, self.level, spew, dice),
            };
            self.algorithm = alg;
            self.old_ai = alg;
            self.prev_ai = alg;
            self.format_brain(dice);
        }
        self.old_ai = self.algorithm;
        self.operation_count += f.ticks;
        self.daction = if me.etype == 0 { act::WALK } else { act::READY };
        self.coll_pnum = contact;
        self.yaw = me.yaw;
        self.trans = Vec3::ZERO;
        self.closest_player(me, f, offsets);
        let mut out = Out {
            yaw: me.yaw,
            ..Default::default()
        };
        if self.actual_dist > self.sight && !me.visactive {
            out.frozen = true;
        } else {
            if self.prev_closest.is_some() && self.closest != self.prev_closest {
                self.ms_idx = 0;
                self.max_msidx = 4;
            }
            let mut c = Ctx {
                b: self,
                me,
                f,
                probe,
                dice,
                out: &mut out,
            };
            c.do_ai();
            if self.explode {
                out.explode = true;
            }
            out.trans = self.trans;
            out.yaw = self.yaw;
            out.daction = self.daction;
            if me.busy.is_none() {
                self.free_act = match self.daction {
                    act::READYTORUN => act::RUN,
                    a @ (act::READY | act::WALK | act::RUN | act::RUNATTACK | act::RUNATTACK2) => a,
                    _ => self.free_act,
                };
            }
        }
        self.prev_ai = self.algorithm;
        self.algorithm = self.old_ai;
        if self.operation_count >= self.operation_speed {
            self.operation_count -= self.operation_speed;
        }
        out
    }

    /// do_enemy_move / do_enemy_collide after the game has committed this
    /// frame's move: the blocked responses that steer the corner hugging, and
    /// the routine's own after-move checks. Returns a new facing when a
    /// response snaps it.
    pub fn after_move(&mut self, etype: usize, pos: Vec3, r: Report) -> Option<f32> {
        let alg = self.move_alg;
        let mut snap = None;
        self.moved = r.tried;
        if etype == crate::enemies::IT || self.dead_end <= 0 {
            self.area = 0;
        }
        if r.wall && r.tried {
            match alg {
                0 => self.blocked00(&mut snap),
                7 | 8 => self.blocked(10, 60, &mut snap, false),
                10 => self.blocked(10, 60, &mut snap, false),
                20 => self.blocked(3, 30, &mut snap, true),
                _ => {
                    if self.dead_end <= 0 {
                        self.dead_end = 20;
                    }
                }
            }
            self.area = 1;
        }
        let mut blocked = false;
        if let Some(pp) = r.player {
            // Touching a player: the move is undone (fn_80046140 swings).
            self.moved = false;
            self.route = turn_dir(pos, pp);
        } else {
            for (hit, area, short, long) in [(r.enemy, 2u8, 15, 50), (r.item, 3, 15, 15)] {
                let Some(other) = hit else { continue };
                if blocked {
                    break;
                }
                self.moved = false;
                blocked = true;
                if alg == 0 {
                    self.route = turn_dir(pos, other);
                    if self.dead_end <= 0 {
                        self.dead_end = 60;
                    }
                } else if matches!(alg, 7 | 8 | 10 | 20) {
                    if self.route == 0 || self.route.abs() > 2 {
                        self.route = turn_dir(pos, other);
                        self.collided = 0;
                    }
                    let (s, l) = match (alg, area) {
                        (8, 2) => (10, 60),
                        (20, 2) => (10, 30),
                        _ => (short, long),
                    };
                    self.blocked(s, l, &mut snap, alg == 20);
                } else if self.dead_end <= 0 {
                    self.dead_end = 20;
                }
                self.area = area;
            }
            if !blocked {
                if alg == 0 && self.dead_end <= 0 {
                    self.route = 1;
                    self.collided = 0;
                } else if matches!(alg, 2 | 4) {
                    self.play -= 1;
                    if self.play <= 0 {
                        self.count = 0;
                    }
                }
            }
        }
        self.coll_enemy = r.enemy.is_some();
        self.coll_item = r.item.is_some();
        match self.post {
            Post::None => {}
            Post::Corner { stuck } => {
                if self.moved {
                    self.dead_end = 0;
                }
                if !stuck && self.dead_end > 0 {
                    self.anghit = self.ang;
                    self.counter1 = 0;
                }
            }
            Post::Suicide { stuck } => {
                if self.moved {
                    self.dead_end = 0;
                }
                if self.flag2 >= 240 || r.player.is_some() {
                    self.explode = true;
                } else if !stuck && self.dead_end > 0 {
                    self.anghit = self.ang;
                    self.counter1 = 0;
                }
            }
        }
        self.post = Post::None;
        snap
    }

    /// blocked00: alternate sides on each block, then turn back on the bearing.
    fn blocked00(&mut self, snap: &mut Option<f32>) {
        self.collided += 1;
        self.arm(if self.route.abs() <= 2 { 5 } else { 60 });
        if self.collided >= 9 {
            self.route = -self.route * 2;
            self.collided = 0;
            if self.route.abs() > 2 {
                self.ang = self.face;
                *snap = Some(self.face);
            }
        }
    }

    /// blocked07/08/10/20: count blocks on this side, flip to the other side
    /// (doubling the route) after seven, and after both sides give up head
    /// straight back on the bearing (20: away from it).
    fn blocked(&mut self, short: i32, long: i32, snap: &mut Option<f32>, away: bool) {
        if self.route.abs() <= 2 {
            self.collided += 1;
            self.arm(short);
        } else {
            self.arm(long);
            let a = if away {
                wrap(PI + self.face)
            } else {
                self.face
            };
            self.ang = a;
            *snap = Some(a);
            self.collided = 0;
            self.route = 0;
        }
        if self.collided >= 7 {
            self.route = -self.route * 2;
            self.collided = 0;
        }
    }

    /// fn_8004D030: arm dead_end unless it already runs.
    fn arm(&mut self, ticks: i32) {
        if self.dead_end <= 0 {
            self.dead_end = ticks;
        }
    }

    /// format_brain: the routine's starting state.
    fn format_brain(&mut self, dice: &mut dyn Dice) {
        self.view = 3.159_046;
        self.ai_flags = 0;
        match self.algorithm {
            0 | 27 => {
                self.route = 1;
                self.collided = 0;
            }
            3 => {
                self.route = 1;
                self.collided = 0;
                self.counter1 = 0;
                self.counter2 = -1;
                self.flag2 = 0;
            }
            2 | 4 => {
                self.play = 0;
                self.ai_flags |= 1;
            }
            5 | 6 => self.ai_flags |= 1,
            7 | 10 | 13 | 20 => {
                self.route = 0;
                self.collided = 0;
                self.stuck_count = 0;
            }
            14 => {
                self.route = 0;
                self.collided = 0;
                self.stuck_count = 0;
                self.counter1 = 0;
            }
            16 | 23 | 17 | 26 => {
                self.flag2 = dice.int(10);
                self.counter1 = 0;
            }
            21 | 24 => self.counter1 = 0,
            28 | 29 | 31 => {
                self.flag2 = dice.int(30);
                self.counter1 = 0;
                self.counter2 = 0;
            }
            30 => {
                self.route = 1;
                self.collided = 0;
                self.counter1 = dice.int(60) + 60;
            }
            _ => {}
        }
        self.count = 0;
        self.dead_end = 0;
        self.plr_ms = -1;
        self.ms_idx = 0;
        self.max_msidx = 4;
        self.operation_speed = 8;
        self.operation_count = dice.int(self.operation_speed as u32);
    }

    /// fn_800516F8: the player it goes after. IT draws everyone; otherwise the
    /// nearest visible player within sight, every eighth frame, each monster
    /// already on someone adding 2 to their distance so monsters spread out.
    fn closest_player(&mut self, me: &Me, f: &Frame, offsets: &mut [f32]) {
        let dist = |p: &Seen| (me.pos - p.target()).length();
        if !f.players.iter().any(|p| p.active) {
            self.recognized = false;
        }
        let death = me.etype == crate::enemies::DEATH;
        let it = f.it.filter(|&i| {
            f.players
                .get(i)
                .is_some_and(|p| p.active && !p.hidden && !(death && p.halo))
        });
        if let Some(i) = it {
            self.prev_closest = self.closest;
            self.closest = Some(i);
            self.actual_dist = dist(&f.players[i]);
            self.close_dist = self.actual_dist + offsets.get(i).copied().unwrap_or(0.0);
        } else {
            let mut go = (f.frame & 7) as usize == (me.slot & 7) || self.closest.is_none();
            if self
                .closest
                .is_some_and(|c| !f.players.get(c).is_some_and(|p| p.active))
            {
                go = true;
            }
            if go {
                self.prev_closest = self.closest;
                self.closest = None;
                self.close_dist = 100_000.0;
                self.actual_dist = 100_000.0;
                if death {
                    self.counter2 = -1;
                }
                let mut special = 100_000.0;
                for (i, p) in f.players.iter().enumerate() {
                    if !p.active || p.hidden {
                        continue;
                    }
                    let d = dist(p);
                    let mut range = d;
                    if range > self.sight {
                        continue;
                    }
                    if death && p.halo {
                        if range < special {
                            special = range;
                            self.counter2 = i as i32;
                        }
                        continue;
                    }
                    if range > 5.0 * me.rad {
                        range += offsets.get(i).copied().unwrap_or(0.0);
                    }
                    if range >= self.close_dist {
                        continue;
                    }
                    if self.view < PI {
                        let ad = (yaw_to(p.target(), me.pos) - me.yaw).abs();
                        if ad > self.view {
                            continue;
                        }
                    }
                    self.close_dist = range;
                    self.actual_dist = d;
                    self.closest = Some(i);
                }
            }
        }
        match self.closest {
            Some(c) => {
                if self.actual_dist <= self.sight {
                    self.recognized = true;
                    if let Some(o) = offsets.get_mut(c) {
                        *o += 2.0;
                    }
                }
            }
            None => {
                self.actual_dist = 100_000.0;
                self.close_dist = 100_000.0;
            }
        }
    }
}

/// fn_8004F87C + init_enemy_vars: the routine a monster gets from its type,
/// tier and the generator's or placement's ai.
pub fn spawn_algorithm(etype: usize, level: i32, spew: i32, dice: &mut dyn Dice) -> i32 {
    let by_level = |caster: bool| match level {
        3 if caster => 30,
        4 => 23,
        5 => 17,
        6 => 18,
        _ => 7,
    };
    let mut spew = match etype {
        0 | 3 | 6 | 9 | 12 | 15 | 18 | 21 | 22 => {
            if spew != 2 && spew != 4 {
                if dice.int(1) != 0 {
                    4
                } else {
                    2
                }
            } else {
                spew
            }
        }
        2 | 7 | 14 | 17 | 24 if spew == 0 => by_level(true),
        1 | 4 | 5 | 8 | 10 | 11 | 13 | 16 | 19 | 20 | 25 | 32 | 33 if spew == 0 => by_level(false),
        27 => 31,
        30 => 3,
        31 => 27,
        29 => 19,
        _ => spew,
    };
    if spew == 1 {
        spew = 0;
    }
    if spew == 10 {
        spew = 7;
    }
    if !(0..=31).contains(&spew) {
        DEFAULT_ALG.get(etype).copied().unwrap_or(7)
    } else {
        spew
    }
}

/// fn_8004CFAC: which way to turn around something, on the axis with the
/// larger separation.
fn turn_dir(pos: Vec3, target: Vec3) -> i32 {
    let dx = (pos.x - target.x).abs();
    let dz = (pos.z - target.z).abs();
    if dx >= dz {
        if pos.z < target.z {
            1
        } else {
            -1
        }
    } else if pos.x < target.x {
        -1
    } else {
        1
    }
}

/// The level's milestone network (locators of type 5) and the players'
/// milestone history.
#[derive(Clone, Debug, Default)]
pub struct Paths {
    /// Position and the heading back along its arrow (pi + its yaw).
    pub milestones: Vec<(Vec3, f32)>,
    /// sEnemyMilestoneRoute (fn_80051C78): the chain from the player start.
    pub route: Vec<usize>,
    /// LookoutParams (types 8 and 10): position and the next one's index.
    pub lookouts: Vec<(Vec3, i32)>,
    /// Player.milestone: the last five milestones each player passed, newest
    /// first.
    pub player_ms: [[i32; 5]; 4],
}

impl Paths {
    /// From the level's locators; `start` is the default player position.
    pub fn new(locators: &[gdl_formats::worlds::Locator], start: Vec3) -> Paths {
        use gdl_formats::worlds::LocatorType;
        let milestones = locators
            .iter()
            .filter(|l| l.kind == LocatorType::Milestone)
            .map(|l| {
                let fwd = crate::character::ryp_matrix(l.rot).transform_vector3(Vec3::Z);
                (Vec3::from(l.pos), wrap(PI + fwd.x.det_atan2(fwd.z)))
            })
            .collect();
        let lookouts = locators
            .iter()
            .filter(|l| matches!(l.kind, LocatorType::Sentry | LocatorType::Event))
            .map(|l| (Vec3::from(l.pos), l.next as i16 as i32))
            .collect();
        Paths::from_parts(milestones, lookouts, start)
    }

    pub fn from_parts(
        milestones: Vec<(Vec3, f32)>,
        lookouts: Vec<(Vec3, i32)>,
        start: Vec3,
    ) -> Paths {
        let mut p = Paths {
            milestones,
            route: Vec::new(),
            lookouts,
            player_ms: [[-1; 5]; 4],
        };
        p.build_route(start);
        p
    }

    /// fn_80051C78: follow each milestone's arrow from the one nearest the
    /// start until the chain loops or ends.
    fn build_route(&mut self, start: Vec3) {
        self.route.clear();
        let Some(mut cur) = self.nearest(start) else {
            return;
        };
        loop {
            let prev = cur;
            self.route.push(cur);
            cur = self.next_along(prev, 0.174_532_92);
            if self.route.contains(&cur) || prev == cur || self.route.len() >= 128 {
                break;
            }
        }
    }

    /// fn_80051480: the milestone nearest a point.
    fn nearest(&self, pos: Vec3) -> Option<usize> {
        self.milestones
            .iter()
            .enumerate()
            .map(|(i, m)| (i, (pos - m.0).length()))
            .filter(|(_, d)| *d < 100_000.0)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }

    /// fn_800511D0: the nearest milestone within `tol` of this one's arrow
    /// (the lower of the two nearest when they differ in height).
    fn next_along(&self, ms: usize, tol: f32) -> usize {
        let (pos, base) = self.milestones[ms];
        let mut best = (None, 100_000.0f32, 100_000.0f32);
        let mut second = (None, 100_000.0f32, 100_000.0f32);
        for (i, m) in self.milestones.iter().enumerate() {
            if i == ms {
                continue;
            }
            if wrap(yaw_to(m.0, pos) - base).abs() > tol {
                continue;
            }
            let d = (m.0 - pos).length();
            let dy = (m.0.y - pos.y).abs();
            if d < best.1 {
                second = best;
                best = (Some(i), d, dy);
            } else if d < second.1 {
                second = (Some(i), d, dy);
            }
        }
        let pick = if best.2 > second.2 { second.0 } else { best.0 };
        pick.unwrap_or(ms)
    }

    /// find_neighbor_milestone: the route entry `nth` steps from `ms`
    /// (reflecting at the ends; the one nearer the origin when both exist).
    fn neighbor(&self, ms: i32, nth: i32) -> i32 {
        let Some(idx) = self.route.iter().position(|&r| r as i32 == ms) else {
            return -1;
        };
        let idx = idx as i32;
        let count = self.route.len() as i32;
        let at = |i: i32| self.route.get(i as usize).map_or(-1, |&r| r as i32);
        let lo = idx - nth;
        if lo < 0 {
            return at(idx + nth);
        }
        let hi = idx + nth;
        if hi > count - 1 {
            return at(lo);
        }
        let (m_lo, m_hi) = (at(lo), at(hi));
        let len = |m: i32| self.milestones[m as usize].0.length();
        if len(m_lo) < len(m_hi) {
            m_lo
        } else {
            m_hi
        }
    }

    fn pos(&self, ms: i32) -> Vec3 {
        self.milestones
            .get(ms.max(0) as usize)
            .map_or(Vec3::ZERO, |m| m.0)
    }

    /// update_player_milestone: a player within 4.5 (and 3.5 in height) of a
    /// milestone other than their latest pushes it onto their history.
    pub fn update_players(&mut self, players: &[(Vec3, bool)]) {
        for (pi, (pos, active)) in players.iter().enumerate().take(4) {
            if !active {
                continue;
            }
            for (i, m) in self.milestones.iter().enumerate() {
                let h = &mut self.player_ms[pi];
                if (pos.y - m.0.y).abs() < 3.5 && flat(*pos, m.0) < 4.5 && h[0] != i as i32 {
                    h.copy_within(0..4, 1);
                    h[0] = i as i32;
                }
            }
        }
    }
}

/// One monster's do_ai, with what it reads.
struct Ctx<'a> {
    b: &'a mut Brain,
    me: &'a Me,
    f: &'a Frame<'a>,
    probe: &'a dyn Probe,
    dice: &'a mut dyn Dice,
    out: &'a mut Out,
}

impl Ctx<'_> {
    fn t(&self) -> i32 {
        self.f.ticks
    }

    /// The current action: the busy one, else the last free one granted.
    fn action(&self) -> u8 {
        self.me.busy.unwrap_or(self.b.free_act)
    }

    /// RequestEnemyAction.
    fn request(&mut self, a: u8) {
        if self.me.reloading
            && ((act::ATTACK..=20).contains(&a) || (act::THROW..=act::THROW_FINISH).contains(&a))
        {
            return;
        }
        if ACTPRI[self.b.daction as usize] >= ACTPRI[a as usize] {
            return;
        }
        self.b.daction = a;
    }

    fn player(&self, i: usize) -> Vec3 {
        self.f.players.get(i).map_or(self.me.pos, |p| p.target())
    }

    /// get_face_ang: the bearing to the closest player (else the heading).
    fn face_ang(&self) -> f32 {
        match self.b.closest {
            Some(c) => yaw_to(self.player(c), self.me.pos),
            None => self.b.ang,
        }
    }

    /// turn_enemy_ang: turn the facing toward `want` at the type's rate (three
    /// times while running or hurt); none while appearing.
    fn turn(&self, want: f32) -> f32 {
        let a = self.action();
        let cur = self.b.yaw;
        if a >= act::HIT_REACT1 || a == act::START {
            return cur;
        }
        let mut rate = if self.me.etype == 28 { 0.0 } else { TURN_RATE };
        if a == act::RUN {
            rate *= 3.0;
        }
        let d = wrap(want - cur);
        let step = rate * self.t() as f32;
        let to = if d.abs() <= step {
            want
        } else if d >= 0.0 {
            cur + step
        } else {
            cur - step
        };
        wrap(to)
    }

    fn face(&mut self, want: f32) {
        self.b.yaw = self.turn(want);
    }

    /// set_enemy_trans: ask to walk (run at 1.25 and up) and, if walking,
    /// running or touching a player, move `k` x the type's speed along `ang`.
    fn set_trans(&mut self, k: f32, ang: f32) {
        let a = self.action();
        if a == act::START {
            return;
        }
        self.request(if k >= 1.25 { act::RUN } else { act::WALK });
        if matches!(a, act::WALK | act::RUN | act::RUNATTACK | act::RUNATTACK2)
            || self.b.coll_pnum.is_some()
        {
            let s = k * self.me.spd;
            self.b.trans.x += s * ang.det_sin();
            self.b.trans.z += s * ang.det_cos();
        }
    }

    /// fn_8004C8CC: is `dest` (a probe point one step ahead) clear of other
    /// monsters, solid items and walls?
    fn clear(&self, dest: Vec3) -> bool {
        let me = self.me;
        let from = Vec3::new(me.pos.x, dest.y, me.pos.z);
        if self
            .probe
            .occupied(me.slot, from, dest, me.rad + 0.1, me.hht + 0.1)
        {
            return false;
        }
        !self.probe.wall(from, dest)
    }

    /// One step ahead along `ang`, at the probe height.
    fn ahead(&self, ang: f32, dist: f32) -> Vec3 {
        let p = self.me.pos + Vec3::Y * (0.1 + self.me.rad);
        p + Vec3::new(ang.det_sin(), 0.0, ang.det_cos()) * dist
    }

    /// FoundSuicideBomber: the running suicide grunt is within 10.
    fn bomber_near(&self) -> bool {
        let Some(bm) = self.f.bomber else {
            return false;
        };
        if bm.actual_dist > self.b.sight {
            return false;
        }
        bm.slot != self.me.slot
            && self.b.birth_style == 0
            && self.b.dead_end <= 0
            && (bm.pos - self.me.pos).length_squared() < 100.0
    }

    /// The routine changed since last frame: format_brain.
    fn fresh(&mut self) {
        if self.b.algorithm != self.b.prev_ai {
            self.b.format_brain(self.dice);
        }
    }

    /// Hand over to another routine for this frame.
    fn delegate(&mut self, alg: i32) {
        self.b.algorithm = alg;
        self.do_ai();
    }

    fn wander_or(&mut self) -> bool {
        if !self.b.recognized || self.b.closest.is_none() {
            let w = (self.me.slot & 1) as i32 + 5;
            self.delegate(w);
            return true;
        }
        false
    }

    /// do_enemy_move: the move is committed by the game.
    fn commit(&mut self, post: Post) {
        self.b.move_alg = self.b.algorithm;
        self.b.post = post;
    }

    /// do_ai: dispatch on the algorithm.
    fn do_ai(&mut self) {
        self.b.trans = Vec3::ZERO;
        match self.b.algorithm {
            0 | 9 => self.logic00(),
            2 => self.logic02(1.0),
            3 => self.logic03(),
            4 => self.logic02(-1.0),
            5 => self.logic05(-1.0),
            6 => self.logic05(1.0),
            7 => self.logic07(),
            10 => self.logic10(),
            12 => self.logic12(),
            13 => self.logic13(),
            14 => self.logic14(),
            15 => self.logic15(),
            16 | 26 => self.logic16(),
            17 | 23 => self.logic23(),
            18 => self.logic18(),
            20 => self.logic20(),
            21 => self.logic21(),
            24 => self.logic24(),
            27 => {
                if self.b.closest.is_none() {
                    self.commit(Post::None);
                } else {
                    self.b.sight = 999_999.0;
                    self.b.algorithm = 0;
                    self.logic00();
                }
            }
            28 => self.logic28(),
            29 => self.logic29(),
            30 => self.logic30(),
            31 => self.logic31(),
            // Not reached by any shipped monster in this port: 1 (milestone
            // patrol) and 8 (chest guard) chase; 11/19/22 stand in for the
            // spinning decoys, golems (critters here) and hands.
            1 | 8 => self.logic07(),
            _ => self.logic00(),
        }
    }

    /// move_logic00 (and 09): face the closest player and sweep the heading
    /// up to 8 x 22.5 degrees to one side until a step is clear.
    fn logic00(&mut self) {
        if self.bomber_near() {
            return self.delegate(24);
        }
        if self.wander_or() {
            return;
        }
        self.fresh();
        if self.b.dead_end > 0 {
            self.b.dead_end -= self.t();
        }
        if self.b.dead_end <= 0 {
            let mut ang = self.face_ang();
            self.b.face = ang;
            let mut i = 0;
            while i < 9 {
                ang = if self.b.route > 0 {
                    ang + SWEEP00[i]
                } else {
                    ang - SWEEP00[i]
                };
                ang = wrap(ang);
                let dest = self.ahead(ang, self.me.spd);
                let d = wrap(ang - self.b.ang);
                let flip = (self.b.ang - self.b.angbak).abs() > FLIP
                    && (ang - self.b.angbak).abs() <= FLIP;
                if !flip && d.abs() < 3.106_686 && self.clear(dest) {
                    break;
                }
                i += 1;
            }
            if i >= 9 {
                ang = self.b.face;
            } else {
                self.b.collided = i as i32;
            }
            self.b.angbak = self.b.ang;
            self.b.ang = ang;
        }
        let a = self.b.ang;
        self.set_trans(1.0, a);
        self.face(a);
        self.commit(Post::None);
    }

    /// move_logic02 / 04: small monsters drift, turning 45 degrees (02 left,
    /// 04 right) whenever blocked and swapping routines after four turns;
    /// within 8 of a player they chase.
    fn logic02(&mut self, side: f32) {
        if self.bomber_near() {
            return self.delegate(24);
        }
        if self.b.closest.is_some() && self.b.close_dist <= 8.0 {
            return self.delegate(0);
        }
        self.fresh();
        if self.b.dead_end > 0 {
            self.b.dead_end -= self.t();
            if self.b.dead_end <= 0 {
                self.b.ang = wrap(self.b.ang + side * 0.785_398_2);
                self.b.count += 1;
                if self.b.count >= 4 {
                    self.b.count = 0;
                    let other = if side > 0.0 { 4 } else { 2 };
                    self.b.algorithm = other;
                    self.b.old_ai = other;
                    self.b.play = 4;
                }
            }
        }
        if let Some(pi) = self.b.coll_pnum {
            self.b.ang = yaw_to(self.player(pi), self.me.pos);
        }
        let a = self.b.ang;
        self.set_trans(1.0, a);
        self.face(a);
        self.commit(Post::None);
    }

    /// move_logic03 (Death): chase once it has someone; else back away from a
    /// player wearing the anti-death halo, deflecting a little more each
    /// time it is stopped; else wander.
    fn logic03(&mut self) {
        if self.b.recognized && self.b.closest.is_some() {
            return self.delegate(0);
        }
        if self.b.counter2 < 0 {
            let w = (self.me.slot & 1) as i32 + 5;
            return self.delegate(w);
        }
        let stuck = self.b.dead_end > 0;
        let mut spd = 0.0;
        if self.b.dead_end > 0 && self.b.counter1 < 8 {
            spd = leap(self.b.counter1);
            self.b.counter1 += 1;
            self.b.dead_end = 0;
        }
        let face = yaw_to(self.player(self.b.counter2 as usize), self.me.pos);
        self.b.ang = wrap(spd + PI + face);
        let a = self.b.ang;
        self.set_trans(1.0, a);
        self.face(a);
        self.b.trans *= 0.9;
        self.commit(Post::Corner { stuck });
    }

    /// move_logic05 / 06: wander, turning 90 degrees (05 left, 06 right) at
    /// walls, monsters and items ahead.
    fn logic05(&mut self, side: f32) {
        if self.bomber_near() {
            return self.delegate(24);
        }
        self.fresh();
        let quarter = side * 1.570_796_4;
        if self.b.dead_end > 0 {
            self.b.dead_end -= self.t();
            if self.b.dead_end <= 0 {
                self.b.ang = wrap(self.b.ang + quarter);
                self.b.count += 1;
                if self.b.count >= 4 {
                    self.b.count = 0;
                }
            }
        }
        let probe = self.ahead(0.0, 0.0);
        let end = self.ahead(self.b.ang, self.me.rad + 0.5);
        let blocked = if self.probe.wall(probe, end) {
            true
        } else {
            !self.clear(self.ahead(self.b.ang, self.me.spd))
        };
        if blocked {
            self.b.ang = wrap(self.b.ang + quarter);
            self.b.arm(20);
        }
        let a = self.b.ang;
        self.set_trans(1.0, a);
        self.face(a);
        self.commit(Post::None);
    }

    /// The corner-hug heading of move_logic07/10/20: straight at the bearing
    /// (`toward`); after a wall the bearing turned by the hug offset for the
    /// blocks so far, to the side `route` picked; after a monster or item the
    /// heading turned the same way. With a milestone (move_logic10) it always
    /// turns off the milestone's bearing; `pnum` keeps the bearing while
    /// touching a player. Kept if a step that way is clear; returns it and
    /// whether the step was blocked.
    fn hug(&mut self, toward: f32, milestone: bool, pnum: bool) -> (f32, bool) {
        let col = (self.b.collided.max(0) as usize).min(HUG.len() - 1);
        let off = |route: i32, base: f32| {
            if route > 0 {
                base + HUG[col]
            } else {
                base - HUG[col]
            }
        };
        let mut cand = if milestone {
            if self.b.route == 0 {
                self.b.route = self.route_to_milestone();
            }
            off(self.b.route, toward)
        } else if pnum && self.b.coll_pnum.is_some() {
            toward
        } else if self.b.area == 1 {
            if self.b.route == 0 {
                self.b.route = self.route_to_player();
            }
            off(self.b.route, toward)
        } else if self.b.coll_item || self.b.coll_enemy {
            off(self.b.route, self.b.ang)
        } else {
            toward
        };
        cand = wrap(cand);
        let dest = self.ahead(cand, self.me.spd);
        let flip =
            (self.b.ang - self.b.angbak).abs() > FLIP && (cand - self.b.angbak).abs() <= FLIP;
        let mut found = false;
        if flip || !self.clear(dest) {
            found = true;
            self.b.stuck_count += 1;
        } else {
            self.b.stuck_count = 0;
        }
        if self.b.stuck_count > 10 {
            cand = toward;
            self.b.ang = cand;
        }
        if !found {
            self.b.angbak = self.b.ang;
            self.b.ang = cand;
        }
        (cand, found)
    }

    /// fn_8004CE38: turn whichever way brings it nearer the player.
    fn route_to_player(&self) -> i32 {
        let Some(c) = self.b.closest else { return 1 };
        side_toward(self.b.yaw, self.me.pos - self.player(c))
    }

    /// set_turn_to_ms: turn whichever way brings it nearer its milestone.
    fn route_to_milestone(&self) -> i32 {
        side_toward(self.b.yaw, self.me.pos - self.f.paths.pos(self.b.plr_ms))
    }

    /// move_logic07: the default chase, hugging corners on the way.
    fn logic07(&mut self) {
        if self.bomber_near() {
            return self.delegate(24);
        }
        if self.wander_or() {
            return;
        }
        self.fresh();
        self.b.face = self.face_ang();
        self.chase_step(false, true);
        if self.me.etype == crate::enemies::RAT && self.b.daction == act::READY {
            self.request(act::WALK);
        }
    }

    /// The shared tail of move_logic07/10: count down dead_end, pick the hug
    /// heading toward the frame's bearing, walk it.
    fn chase_step(&mut self, milestone: bool, pnum: bool) {
        if self.b.dead_end > 0 {
            self.b.dead_end -= self.t();
        }
        let face = self.b.face;
        let (cand, found) = if self.b.dead_end <= 0 {
            self.hug(face, milestone, pnum)
        } else {
            (self.b.ang, false)
        };
        self.set_trans(1.0, cand);
        if !found || cand == face {
            self.face(cand);
        }
        self.commit(Post::None);
    }

    fn ms_bearing(&self, ms: i32) -> f32 {
        yaw_to(self.f.paths.pos(ms), self.me.pos)
    }

    /// move_logic10: chase like 07; after five blocks follow the closest
    /// player's trail of milestones instead (mode 1), stepping along the
    /// level's route when one stays out of reach, back to the chase when the
    /// trail runs out.
    fn logic10(&mut self) {
        if self.bomber_near() {
            return self.delegate(24);
        }
        if self.wander_or() {
            return;
        }
        self.update_milestone();
        let Some(c) = self.b.closest else { return };
        let hist = self.f.paths.player_ms.get(c).copied().unwrap_or([-1; 5]);
        if self.b.mode1 <= 1 && self.b.coll_pnum.is_some() {
            // logic10_attacking: face and push into the player.
            self.fresh();
            self.b.ang = self.face_ang();
            self.b.dead_end = 0;
            let a = self.b.ang;
            self.set_trans(1.0, a);
            self.face(a);
            return self.commit(Post::None);
        }
        self.fresh();
        match self.b.mode1 {
            0 => {
                if self.b.collided >= 5 {
                    self.b.stuck_count = 0;
                    self.b.plr_ms = hist[0];
                    if self.b.plr_ms >= 0 {
                        self.b.mode1 += 1;
                        self.b.mode2 = 0;
                        self.b.collided = 0;
                    }
                } else {
                    self.b.face = self.face_ang();
                }
                self.chase_step(false, false);
            }
            1 => {
                let ms = self.b.plr_ms;
                if ms >= 0 {
                    if self.b.stuck_count >= 5 {
                        self.b.mode2 += 1;
                        self.b.plr_ms = self.f.paths.neighbor(ms, self.b.mode2);
                        if self.b.plr_ms < 0 {
                            self.b.plr_ms = ms;
                        }
                        self.b.stuck_count = 0;
                        self.b.collided = 0;
                    }
                    self.b.face = self.ms_bearing(self.b.plr_ms);
                } else {
                    let mut got = false;
                    self.b.mode2 -= 1;
                    if self.b.mode2 > 0 {
                        self.b.plr_ms = self.f.paths.neighbor(self.b.plr_ms, self.b.mode2);
                        if self.b.plr_ms >= 0 {
                            got = true;
                            self.b.stuck_count = 0;
                            self.b.collided = 0;
                            self.b.face = self.ms_bearing(self.b.plr_ms);
                        }
                    }
                    if !got {
                        self.b.face = self.face_ang();
                        self.b.mode1 = 0;
                        self.b.mode2 = 0;
                        self.b.stuck_count = 0;
                        self.b.collided = 0;
                    }
                }
                self.chase_step(self.b.plr_ms >= 0, false);
            }
            _ => {
                if self.b.plr_ms >= 0 {
                    if self.b.stuck_count < 5 {
                        self.b.face = self.ms_bearing(self.b.plr_ms);
                    } else {
                        self.b.ms_idx += 1;
                        let v = hist.get(self.b.ms_idx as usize).copied().unwrap_or(-1);
                        if self.b.ms_idx > self.b.max_msidx || v < 0 {
                            self.b.ms_idx = 0;
                            self.b.max_msidx = 4;
                            self.b.plr_ms = -1;
                            self.b.face = self.face_ang();
                        } else {
                            self.b.plr_ms = v;
                            self.b.stuck_count = 0;
                        }
                    }
                } else if self.b.stuck_count >= 5 {
                    self.b.plr_ms = hist.get(self.b.ms_idx as usize).copied().unwrap_or(-1);
                    self.b.face = if self.b.plr_ms >= 0 {
                        self.ms_bearing(self.b.plr_ms)
                    } else {
                        self.face_ang()
                    };
                } else {
                    self.b.face = self.face_ang();
                }
                self.chase_step(self.b.plr_ms >= 0, false);
            }
        }
    }

    /// update_enemy_milestone: reaching its milestone (1.4 across, 2.5 in
    /// height) moves it one step newer along the player's trail.
    fn update_milestone(&mut self) {
        if self.b.plr_ms < 0 {
            return;
        }
        let m = self.f.paths.pos(self.b.plr_ms);
        let d = self.me.pos - m;
        if d.y.abs() < 2.5 && flat(self.me.pos, m) < 1.4 {
            // adjust_msidx
            let hist = self
                .b
                .closest
                .and_then(|c| self.f.paths.player_ms.get(c).copied())
                .unwrap_or([-1; 5]);
            match hist.iter().position(|&h| h == self.b.plr_ms) {
                Some(i) => {
                    self.b.ms_idx = i as i32;
                    self.b.max_msidx = self.b.max_msidx.max(self.b.ms_idx);
                }
                None => {
                    self.b.ms_idx = 0;
                    self.b.max_msidx = 4;
                }
            }
            self.b.plr_ms = -1;
            self.b.operation_count = self.b.operation_speed;
            if self.b.ms_idx > 0 {
                self.b.ms_idx -= 1;
                self.b.max_msidx = self.b.ms_idx;
            }
            if self.b.algorithm == 10 {
                self.b.collided = 0;
                self.b.route = 0;
                self.b.stuck_count = 0;
            }
        }
    }

    /// move_logic12: a logic-12 generator's egg stands where place_logic12
    /// put it (the game snaps it to Enemy.dest) facing the player until its
    /// generator's flags reach 7 (all three laid, the generator full, or an
    /// egg hit), then chases.
    fn logic12(&mut self) {
        if self.bomber_near() {
            return self.delegate(24);
        }
        if self.b.mode1 >= 2 || !self.me.has_generator {
            return self.delegate(0);
        }
        self.fresh();
        self.b.ang = self.face_ang();
        let a = self.b.ang;
        self.face(a);
        self.commit(Post::None);
        if self.b.mode1 == 0 {
            self.b.mode1 = 1;
        } else if self.me.hatch {
            self.b.mode1 = 2;
        }
    }

    /// move_logic13: zombies follow the one made before them; one left more
    /// than 15 behind for three seconds lets the chain go.
    fn logic13(&mut self) {
        if self.bomber_near() {
            return self.delegate(24);
        }
        if self.wander_or() {
            return;
        }
        let prev = self
            .b
            .prev
            .and_then(|id| self.probe.enemy_pos(id).map(|p| (id, p)));
        let Some((id, pp)) = prev.filter(|_| self.me.has_generator) else {
            return self.delegate(7);
        };
        self.fresh();
        self.b.ang = yaw_to(pp, self.me.pos);
        let a = self.b.ang;
        self.set_trans(1.0, a);
        self.face(a);
        self.commit(Post::None);
        if (pp - (self.me.pos + self.b.trans)).length() >= 15.0 {
            self.b.lost += self.t();
            if self.b.lost >= 180 {
                self.out.release = Some(id);
                self.b.prev = None;
            }
        }
    }

    /// move_logic14: zig-zag at the player, swinging 90 degrees every 45
    /// ticks; chase within 8.
    fn logic14(&mut self) {
        if self.bomber_near() {
            return self.delegate(24);
        }
        if self.wander_or() {
            return;
        }
        if self.b.close_dist <= 8.0 {
            return self.delegate(0);
        }
        self.fresh();
        self.b.face = self.face_ang();
        self.b.count -= self.t();
        if self.b.count <= 0 {
            self.b.flag1 = -self.b.flag1;
            let q = if self.b.flag1 > 0 {
                1.570_796_4
            } else {
                -1.570_796_4
            };
            self.b.ang = wrap(self.b.ang + q);
            self.b.count += 45;
            self.b.mode1 += 1;
        }
        let a = self.b.ang;
        self.face(a);
        self.set_trans(1.0, a);
        self.commit(Post::None);
        let drift = wrap(self.b.face - self.b.ang);
        if self.b.counter1 <= 0
            && (self.b.dead_end > 0 || (self.b.mode1 >= 4 && drift.abs() > 1.570_796_4))
        {
            let scale = 0.261_799_4 * self.b.flag2 as f32 + 0.785_398_2;
            if self.b.mode1 >= 4 {
                self.b.flag1 = -self.b.flag1;
            }
            self.b.dead_end = 0;
            let c = self.b.closest.unwrap_or(0);
            let base = yaw_to(self.player(c), self.me.pos);
            self.b.ang = wrap(if self.b.flag1 > 0 {
                base + scale
            } else {
                base - scale
            });
            self.b.count = 0;
            self.b.mode1 = 0;
            self.b.flag2 += 1;
            self.b.counter1 = 30;
        } else {
            if self.b.flag2 > 0 {
                self.b.flag2 -= 1;
            }
            self.b.counter1 -= self.t();
        }
    }

    /// move_logic15: patrol the lookout path from the nearest lookout until a
    /// player comes within 0.8 x sight, then chase.
    fn logic15(&mut self) {
        if self.bomber_near() {
            return self.delegate(24);
        }
        if let Some(c) = self.b.closest {
            if self.b.close_dist <= 0.8 * self.b.sight
                && flat(self.player(c), self.me.pos) <= 0.8 * self.b.sight
            {
                self.delegate(0);
                self.b.mode1 = 0;
                return;
            }
        }
        self.fresh();
        let looks = &self.f.paths.lookouts;
        if self.b.mode1 == 0 {
            self.b.flag1 = looks
                .iter()
                .enumerate()
                .filter(|(_, l)| l.1 >= 0)
                .map(|(i, l)| (i, (l.0 - self.me.pos).length()))
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map_or(-1, |(i, _)| i as i32);
            self.b.mode1 = 1;
        }
        if self.b.mode1 == 1 {
            if let Some(&(lp, next)) = looks
                .get(self.b.flag1.max(0) as usize)
                .filter(|_| self.b.flag1 >= 0)
            {
                self.b.ang = yaw_to(lp, self.me.pos);
                if (lp.y - self.me.pos.y).abs() < 4.0 && flat(lp, self.me.pos) < 1.0 {
                    self.b.flag1 = next;
                }
            }
        }
        let a = self.b.ang;
        self.set_trans(1.0, a);
        self.face(a);
        self.commit(Post::None);
    }

    /// move_logic16 (archers 16, bombers 26): throw from afar; inside 0.6 x
    /// sight back away while shooting (RUNATTACK), deflecting when stopped,
    /// until past 0.8 x sight.
    fn logic16(&mut self) {
        let stuck = self.b.dead_end > 0;
        if self.bomber_near() {
            return self.delegate(24);
        }
        self.fresh();
        self.b.ang = self.face_ang();
        let mut leapspeed = 0.0;
        if let Some(c) = self.b.closest {
            let dv = self.me.pos.y - self.player(c).y;
            if self.me.visactive && (-10.0..=10.0).contains(&dv) {
                if self.b.flag1 == 0 {
                    if self.b.actual_dist <= 0.6 * self.b.sight {
                        self.b.flag1 = 1;
                    }
                } else if self.b.actual_dist > 0.8 * self.b.sight {
                    self.b.flag1 = 0;
                } else if self.b.dead_end > 0 {
                    if self.b.counter1 < 8 {
                        leapspeed = leap(self.b.counter1);
                        self.b.counter1 += 1;
                        self.b.dead_end = 0;
                    } else {
                        self.b.flag1 = 0;
                    }
                }
                if self.b.flag2 > 0 {
                    self.b.flag2 -= self.t();
                } else if self.b.flag1 == 0 {
                    self.request(act::THROW);
                } else {
                    let la = wrap(PI + self.b.ang + leapspeed);
                    self.request(act::RUNATTACK);
                    if matches!(self.action(), act::RUNATTACK | act::RUNATTACK2) {
                        self.set_trans(0.8, la);
                    }
                    self.b.dead_end = 0;
                }
            }
        }
        let a = self.b.ang;
        self.face(a);
        self.commit(Post::Corner { stuck });
    }

    /// move_logic18 (the suicide grunt): wait for a player in sight, a second
    /// of crouch, then run at them at 1.5 x speed, deflecting off what stops
    /// it; after 240 ticks of running or on touching someone it blows up.
    fn logic18(&mut self) {
        let stuck = self.b.dead_end > 0;
        self.fresh();
        self.b.ang = self.face_ang();
        if self.b.mode1 <= 0 {
            if self.b.closest.is_some() && self.b.actual_dist <= self.b.sight {
                self.b.mode1 = 1;
                self.b.flag1 = 60;
                self.b.flag2 = 0;
            }
        } else if self.b.mode1 == 1 {
            if self.b.closest.is_some() && self.action() != act::RUN {
                self.b.flag1 -= self.t();
                if self.b.flag1 <= 0 {
                    self.request(act::READYTORUN);
                }
            }
            if self.action() == act::RUN {
                self.b.mode1 = 2;
                if self.run18() {
                    return;
                }
            }
        } else if self.run18() {
            return;
        }
        let a = self.b.ang;
        self.face(a);
        self.commit(Post::Suicide { stuck });
    }

    /// move_logic18's run; true when it handed over to the wander.
    fn run18(&mut self) -> bool {
        self.b.flag2 += self.t();
        if !self.b.recognized || self.b.closest.is_none() {
            let w = (self.me.slot & 1) as i32 + 5;
            self.delegate(w);
            return true;
        }
        if self.b.area == 2 && self.b.dead_end > 0 {
            self.b.dead_end -= self.t();
        }
        if self.b.dead_end > 0 {
            let mut ls = 0.0;
            if self.b.counter1 < 16 {
                ls = leap(self.b.counter1);
                self.b.counter1 += 1;
                self.b.dead_end = 0;
            }
            self.b.ang = wrap(self.b.ang + ls);
        }
        if self.b.dead_end <= 0 || (self.b.ang - self.b.anghit).abs() >= 0.104_719_76 {
            self.b.dead_end = 0;
            let a = self.b.ang;
            self.set_trans(1.5, a);
        }
        false
    }

    /// move_logic20: corner-hug away from the player at double speed.
    fn logic20(&mut self) {
        if self.wander_or() {
            return;
        }
        self.fresh();
        self.b.face = self.face_ang();
        let away = wrap(PI + self.b.face);
        if self.b.dead_end > 0 {
            self.b.dead_end -= self.t();
        }
        let (cand, found) = if self.b.dead_end <= 0 {
            self.hug(away, false, true)
        } else {
            (self.b.ang, false)
        };
        self.set_trans(2.0, cand);
        if !found || cand == away {
            self.face(cand);
        }
        self.commit(Post::None);
    }

    /// move_logic21: back straight away from the player at double speed.
    fn logic21(&mut self) {
        let stuck = self.b.dead_end > 0;
        if self.wander_or() {
            return;
        }
        self.fresh();
        let mut spd = 0.0;
        if self.b.dead_end > 0 && self.b.counter1 < 8 {
            spd = leap(self.b.counter1);
            self.b.counter1 += 1;
            self.b.dead_end = 0;
        }
        self.b.ang = wrap(spd + PI + self.face_ang());
        let a = self.b.ang;
        self.set_trans(2.0, a);
        self.face(a);
        self.commit(Post::Corner { stuck });
    }

    /// move_logic23 (archers 23, bombers 17): stand facing the player and
    /// throw while they are in sight and near its height.
    fn logic23(&mut self) {
        self.fresh();
        self.b.ang = self.face_ang();
        if let Some(c) = self.b.closest {
            let dy = self.me.pos.y - self.player(c).y;
            if self.me.visactive
                && self.b.actual_dist <= self.b.sight
                && (-10.0..=10.0).contains(&dy)
            {
                if self.b.flag2 <= 0 {
                    self.request(act::THROW);
                } else {
                    self.b.flag2 -= self.t();
                }
            }
        }
        let a = self.b.ang;
        self.face(a);
        self.commit(Post::None);
    }

    /// move_logic24: run from the suicide grunt.
    fn logic24(&mut self) {
        let stuck = self.b.dead_end > 0;
        self.fresh();
        let mut spd = 0.0;
        if self.b.dead_end > 0 && self.b.counter1 < 8 {
            spd = leap(self.b.counter1);
            self.b.counter1 += 1;
            self.b.dead_end = 0;
        }
        let from = self.f.bomber.map_or(self.me.pos, |b| b.pos);
        self.b.ang = wrap(spd + PI + yaw_to(from, self.me.pos));
        let a = self.b.ang;
        self.set_trans(2.0, a);
        self.face(a);
        self.commit(Post::Corner { stuck });
    }

    /// The ranged swing request of 28/29: the power attack at tier 3, else
    /// alternating attacks.
    fn swing(&mut self) {
        if self.b.level >= 3 {
            self.request(act::ATTACK_PWR);
        } else {
            self.b.counter2 += 1;
            self.request(if self.b.counter2 & 1 != 0 {
                act::ATTACK
            } else {
                act::ATTACK2
            });
        }
    }

    /// move_logic28: stand and cast at a player in sight; chase within 6.
    fn logic28(&mut self) {
        if self.b.closest.is_some() && self.b.actual_dist <= 6.0 {
            return self.delegate(7);
        }
        self.fresh();
        self.b.ang = self.face_ang();
        if let Some(c) = self.b.closest {
            let dy = self.me.pos.y - self.player(c).y;
            if self.me.visactive
                && self.b.actual_dist <= self.b.sight
                && (-10.0..=10.0).contains(&dy)
            {
                if self.b.flag2 <= 0 {
                    self.swing();
                } else {
                    self.b.flag2 -= self.t();
                }
            }
        }
        let a = self.b.ang;
        self.face(a);
        self.commit(Post::None);
    }

    /// move_logic29: keep between 8 and 18 of the player (backing off when
    /// nearer than 8, closing in from beyond 18) and cast in between; chase
    /// within 6.
    fn logic29(&mut self) {
        let stuck = self.b.dead_end > 0;
        if self.bomber_near() {
            return self.delegate(24);
        }
        if self.b.closest.is_some() && self.b.actual_dist <= 6.0 {
            return self.delegate(7);
        }
        self.fresh();
        self.b.ang = self.face_ang();
        let mut leapspeed = 0.0;
        if let Some(c) = self.b.closest {
            let dv = self.me.pos.y - self.player(c).y;
            if self.me.visactive && (-10.0..=10.0).contains(&dv) {
                let d = self.b.actual_dist;
                if self.b.flag1 == 0 {
                    if d <= 8.0 {
                        self.b.flag1 = 1;
                    }
                    if d > 18.0 {
                        self.b.flag1 = 2;
                    }
                } else {
                    if self.b.flag1 == 1 {
                        if d > 10.0 {
                            self.b.flag1 = 0;
                        }
                    } else if d <= 16.0 {
                        self.b.flag1 = 0;
                    }
                    if self.b.flag1 != 0 && self.b.dead_end > 0 {
                        if self.b.counter1 < 8 {
                            leapspeed = leap(self.b.counter1);
                            self.b.counter1 += 1;
                            self.b.dead_end = 0;
                        } else {
                            self.b.flag1 = 0;
                        }
                    }
                }
                if self.b.flag2 > 0 {
                    self.b.flag2 -= self.t();
                } else if self.b.flag1 == 0 || self.b.area == 1 {
                    self.swing();
                } else {
                    let a = if self.b.flag1 == 1 {
                        PI + self.b.ang + leapspeed
                    } else {
                        self.b.ang + leapspeed
                    };
                    self.set_trans(0.8, wrap(a));
                    self.request(act::WALK);
                    self.b.dead_end = 0;
                }
            }
        }
        let a = self.b.ang;
        self.face(a);
        self.commit(Post::Corner { stuck });
    }

    /// move_logic30 (tier-3 casters): chase as move_logic00, every 90 x
    /// mrate (+ up to half again) ticks stopping 20-30 ticks to cast; melee
    /// within 6.
    fn logic30(&mut self) {
        if self.bomber_near() {
            return self.delegate(24);
        }
        if self.wander_or() {
            return;
        }
        if self.b.actual_dist <= 6.0 {
            return self.delegate(7);
        }
        self.fresh();
        if self.b.dead_end <= 0 {
            self.b.counter1 -= self.t();
            if self.b.counter1 <= 0 {
                let n = (90.0 * self.f.mrate) as i32;
                let r = self.dice.int(10) + 20;
                self.b.arm(r);
                self.b.counter1 = self.b.dead_end + self.dice.int((n >> 1).max(0) as u32) + n;
            }
        }
        if self.b.dead_end > 0 && self.me.visactive {
            self.request(if self.b.level >= 2 {
                act::ATTACK_PWR
            } else {
                act::ATTACK
            });
        }
        self.b.algorithm = 0;
        self.logic00();
        self.b.algorithm = 30;
    }

    /// move_logic31 (the Garm's pups): close in, swiping within 10 and
    /// casting beyond, with a 0.5-1 s pause between.
    fn logic31(&mut self) {
        self.fresh();
        self.b.ang = self.face_ang();
        if self.b.closest.is_some() && self.me.visactive {
            self.b.daction = act::READY;
            let a = self.action();
            let ang = self.b.ang;
            let s = self.me.spd;
            if a == act::ATTACK || a == act::ATTACK + 1 {
                self.b.trans += Vec3::new(ang.det_sin(), 0.0, ang.det_cos()) * s;
                self.b.flag2 = self.dice.int(30) + 30;
            } else if a == act::ATTACK_PWR || a == act::ATTACK_PWR + 1 {
                self.b.flag2 = self.dice.int(30) + 30;
            } else if self.b.flag2 <= 0 {
                self.request(if self.b.actual_dist <= 10.0 {
                    act::ATTACK
                } else {
                    act::ATTACK_PWR
                });
            } else if a != act::START {
                self.b.flag2 -= self.t();
                self.b.trans += Vec3::new(ang.det_sin(), 0.0, ang.det_cos()) * s * 0.5;
            }
        }
        let a = self.b.ang;
        self.face(a);
        self.commit(Post::None);
    }
}

/// fn_8004CE38 / set_turn_to_ms: +1 to turn right, -1 left, whichever of
/// the facing +-30 degrees ends nearer the target (`d` = me - target).
fn side_toward(yaw: f32, d: Vec3) -> i32 {
    let r = wrap(0.523_598_8 + yaw);
    let l = wrap(yaw - 0.523_598_8);
    let (x1, z1) = (d.x + r.det_sin(), d.z + r.det_cos());
    let (x2, z2) = (d.x + l.det_sin(), d.z + l.det_cos());
    if x2 * x2 + z2 * z2 <= x1 * x1 + z1 * z1 {
        -1
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Axis-aligned wall segments on the xz plane.
    struct Walls(Vec<(f32, f32, f32, f32)>);

    fn seg_hit(a: (f32, f32), b: (f32, f32), c: (f32, f32), d: (f32, f32)) -> bool {
        let cross = |o: (f32, f32), p: (f32, f32), q: (f32, f32)| {
            (p.0 - o.0) * (q.1 - o.1) - (p.1 - o.1) * (q.0 - o.0)
        };
        let d1 = cross(c, d, a);
        let d2 = cross(c, d, b);
        let d3 = cross(a, b, c);
        let d4 = cross(a, b, d);
        (d1 > 0.0) != (d2 > 0.0) && (d3 > 0.0) != (d4 > 0.0)
    }

    impl Walls {
        fn blocks(&self, from: Vec3, to: Vec3) -> bool {
            self.0
                .iter()
                .any(|w| seg_hit((from.x, from.z), (to.x, to.z), (w.0, w.1), (w.2, w.3)))
        }
        /// Distance from a point to the nearest wall.
        fn gap(&self, p: Vec3) -> f32 {
            self.0
                .iter()
                .map(|w| {
                    let a = glam::Vec2::new(w.0, w.1);
                    let b = glam::Vec2::new(w.2, w.3);
                    let q = glam::Vec2::new(p.x, p.z);
                    let t = ((q - a).dot(b - a) / (b - a).length_squared()).clamp(0.0, 1.0);
                    (a + (b - a) * t - q).length()
                })
                .fold(f32::MAX, f32::min)
        }
    }

    impl Probe for Walls {
        fn wall(&self, from: Vec3, to: Vec3) -> bool {
            self.blocks(from, to)
        }
        /// No other monsters or items here.
        fn occupied(&self, _: usize, _: Vec3, _: Vec3, _: f32, _: f32) -> bool {
            false
        }
        fn enemy_pos(&self, _: u64) -> Option<Vec3> {
            None
        }
    }

    struct Seq(u32);
    impl Dice for Seq {
        fn int(&mut self, n: u32) -> i32 {
            self.0 = self.0.wrapping_mul(1_103_515_245).wrapping_add(12345);
            if n == 0 {
                0
            } else {
                ((self.0 >> 16) % n) as i32
            }
        }
    }

    /// Run one monster for `ticks` against `walls`, committing moves the way
    /// do_enemy_move does (a move that would bring it within its radius of a
    /// wall is refused and reported as blocked).
    fn run(
        brain: &mut Brain,
        mut pos: Vec3,
        player: Vec3,
        walls: &Walls,
        paths: &Paths,
        ticks: usize,
    ) -> (Vec3, usize, Vec<(Vec3, i32, i32)>) {
        let mut trace = Vec::new();
        let mut dice = Seq(7);
        let mut yaw = brain.ang;
        let players = [Seen {
            pos: player,
            active: true,
            ..Default::default()
        }];
        let mut paths = paths.clone();
        for t in 0..ticks {
            paths.update_players(&[(player, true)]);
            let f = Frame {
                players: &players,
                it: None,
                bomber: None,
                paths: &paths,
                frame: t as u32,
                ticks: 1,
                mrate: 1.0,
            };
            let me = Me {
                slot: 0,
                etype: 4,
                pos,
                yaw,
                rad: 1.5,
                hht: 3.0,
                spd: 0.1,
                visactive: true,
                busy: None,
                reloading: false,
                has_generator: false,
                hatch: false,
            };
            let touching = flat(pos, player) < 1.5 + 0.5 + 1.5;
            let out = brain.think(&me, &f, &mut [0.0], walls, &mut dice, touching.then_some(0));
            yaw = out.yaw;
            // Walls slide the move (SlideAlongWall): keep whichever axis of it
            // stays clear; a move left under 0.01 counts as blocked.
            let tried = out.trans.length() > 0.001;
            let ok = |p: Vec3| walls.gap(p) >= 1.5 && !walls.blocks(pos, p);
            let old = pos;
            for want in [
                pos + out.trans,
                pos + Vec3::new(out.trans.x, 0.0, 0.0),
                pos + Vec3::new(0.0, 0.0, out.trans.z),
            ] {
                if ok(want) {
                    pos = want;
                    break;
                }
            }
            let refused = tried && flat(pos, old) < 0.01;
            let rep = Report {
                tried,
                wall: refused,
                player: touching.then_some(player),
                ..Default::default()
            };
            if let Some(s) = brain.after_move(4, pos, rep) {
                yaw = s;
            }
            trace.push((pos, brain.route, brain.dead_end));
            if touching {
                return (pos, t, trace);
            }
        }
        (pos, ticks, trace)
    }

    #[test]
    fn spawn_algorithms_follow_type_and_tier() {
        let mut d = Seq(1);
        // Grunts chase (7) unless given one; level-3 sorcerers cast (30).
        assert_eq!(spawn_algorithm(4, 1, 0, &mut d), 7);
        assert_eq!(spawn_algorithm(4, 1, -1, &mut d), 7);
        assert_eq!(spawn_algorithm(7, 3, 0, &mut d), 30);
        assert_eq!(spawn_algorithm(4, 3, 0, &mut d), 7);
        // Placed archers, bombers and suicide grunts.
        assert_eq!(spawn_algorithm(4, 4, 0, &mut d), 23);
        assert_eq!(spawn_algorithm(4, 5, 0, &mut d), 17);
        assert_eq!(spawn_algorithm(4, 6, 18, &mut d), 18);
        // Rats drift (2 or 4), Death 3, IT 27, 10 becomes 7.
        assert!(matches!(spawn_algorithm(3, 1, 7, &mut d), 2 | 4));
        assert_eq!(spawn_algorithm(30, 1, 0, &mut d), 3);
        assert_eq!(spawn_algorithm(31, 1, 0, &mut d), 27);
        assert_eq!(spawn_algorithm(4, 1, 10, &mut d), 7);
    }

    #[test]
    fn chaser_hugs_off_a_wall_it_meets_head_on() {
        // Straight at a wall: blocked07 counts the blocks and move_logic07
        // turns the heading off the bearing by growing hug offsets, first to
        // one side, then (route doubled) the other, then gives up for a
        // second (dead_end 60) facing the bearing again.
        let walls = Walls(vec![(-30.0, 0.0, 30.0, 0.0)]);
        let mut b = Brain::new(1, 7, 0.0, 60.0).with_algorithm(7);
        let start = Vec3::new(0.0, 0.0, -8.0);
        let (_, _, trace) = run(
            &mut b,
            start,
            Vec3::new(0.0, 0.0, 10.0),
            &walls,
            &Paths::default(),
            400,
        );
        let routes: Vec<i32> = trace.iter().map(|t| t.1).collect();
        assert!(routes.contains(&-1) || routes.contains(&1), "{routes:?}");
        assert!(routes.iter().any(|r| r.abs() == 2), "the route flips sides");
        assert!(trace.iter().any(|t| t.2 >= 50), "then waits on the bearing");
        assert!(
            trace.iter().any(|t| (t.0.x - start.x).abs() > 0.5),
            "it moves off the line"
        );
    }

    #[test]
    fn chaser_slides_round_a_wall_end() {
        // Coming at a wall at an angle the slide carries it along to the end
        // and round.
        let walls = Walls(vec![(-30.0, 0.0, 3.0, 0.0)]);
        let mut b = Brain::new(1, 7, 0.0, 60.0).with_algorithm(7);
        let (pos, t, _) = run(
            &mut b,
            Vec3::new(0.0, 0.0, -8.0),
            Vec3::new(3.0, 0.0, 6.0),
            &walls,
            &Paths::default(),
            3000,
        );
        assert!(t < 3000, "never reached the player (ended at {pos})");
    }

    #[test]
    fn straight_chaser_stalls_without_hugging() {
        // Against a long wall the plain bearing never gets through: what the
        // corner logic is for.
        let walls = Walls(vec![(-300.0, 0.0, 300.0, 0.0)]);
        let mut b = Brain::new(1, 7, 0.0, 60.0).with_algorithm(7);
        let (pos, t, _) = run(
            &mut b,
            Vec3::new(0.0, 0.0, -8.0),
            Vec3::new(0.0, 0.0, 10.0),
            &walls,
            &Paths::default(),
            600,
        );
        assert_eq!(t, 600);
        assert!(pos.z < 0.0);
    }

    #[test]
    fn milestones_lead_round_a_corner() {
        // Two corridors side by side, joined at their north ends:
        //
        //   +-----------------+   z = 20
        //   |  ms0      ms1   |
        //   |    +            |   the dividing wall x = 5, z = -10..10
        //   |    |            |
        //   | M  |      P     |   z = -8
        //   +----+------------+   z = -10
        //  x=-5  5           25
        //
        // The player went up the west corridor, along the top and down the
        // east one, past the two milestones. Straight at the player is
        // straight into the dividing wall; after five blocks move_logic10
        // follows the player's milestones round the top instead.
        let walls = Walls(vec![
            (-5.0, -10.0, -5.0, 20.0),
            (-5.0, 20.0, 25.0, 20.0),
            (25.0, 20.0, 25.0, -10.0),
            (25.0, -10.0, -5.0, -10.0),
            (5.0, -10.0, 5.0, 10.0),
        ]);
        let start = Vec3::new(0.0, 0.0, -8.0);
        let ms = vec![
            (Vec3::new(0.0, 0.0, 15.0), 0.0),
            (Vec3::new(15.0, 0.0, 15.0), 0.0),
        ];
        let mut paths = Paths::from_parts(ms, Vec::new(), start);
        paths.player_ms[0] = [1, 0, -1, -1, -1];
        let player = Vec3::new(15.0, 0.0, -8.0);
        let mut b = Brain::new(1, 10, 0.0, 100.0).with_algorithm(10);
        let (pos, t, _) = run(&mut b, start, player, &walls, &paths, 4000);
        assert!(t < 4000, "never reached the player (ended at {pos})");
        // The plain chase stays stuck on the dividing wall.
        let mut b = Brain::new(1, 7, 0.0, 100.0).with_algorithm(7);
        let (pos7, t7, _) = run(&mut b, start, player, &walls, &paths, 4000);
        assert_eq!(t7, 4000, "the chase got round too ({pos7})");
        assert!(pos7.x < 5.0);
    }

    #[test]
    fn a_mikey_draws_the_monster_to_it() {
        let paths = Paths::default();
        let mut b = Brain::new(6, 18, 0.0, 60.0);
        // Hero 0 stands 40 off with its Mikey out 5 away; hero 1 is at 20.
        let players = [
            Seen {
                pos: Vec3::new(0.0, 0.0, 40.0),
                active: true,
                decoy: Some(Vec3::new(5.0, 0.0, 0.0)),
                ..Default::default()
            },
            Seen {
                pos: Vec3::new(0.0, 0.0, 20.0),
                active: true,
                ..Default::default()
            },
        ];
        let f = Frame {
            players: &players,
            it: None,
            bomber: None,
            paths: &paths,
            frame: 0,
            ticks: 1,
            mrate: 1.0,
        };
        let me = Me {
            slot: 0,
            etype: 4,
            pos: Vec3::ZERO,
            yaw: 0.0,
            rad: 1.5,
            hht: 3.0,
            spd: 0.1,
            visactive: true,
            busy: None,
            reloading: false,
            has_generator: false,
            hatch: false,
        };
        b.closest_player(&me, &f, &mut [0.0, 0.0]);
        assert_eq!(b.closest, Some(0));
        assert!((b.actual_dist - 5.0).abs() < 1e-4);
        assert_eq!(players[0].target(), Vec3::new(5.0, 0.0, 0.0));
    }

    #[test]
    fn suicide_grunt_waits_crouches_then_runs() {
        let walls = Walls(Vec::new());
        let paths = Paths::default();
        let mut b = Brain::new(6, 18, 0.0, 60.0).with_algorithm(18);
        let mut dice = Seq(3);
        let players = [Seen {
            pos: Vec3::new(0.0, 0.0, 40.0),
            active: true,
            ..Default::default()
        }];
        let mut first_move = None;
        let mut pos = Vec3::ZERO;
        for t in 0..400 {
            let f = Frame {
                players: &players,
                it: None,
                bomber: None,
                paths: &paths,
                frame: t,
                ticks: 1,
                mrate: 1.0,
            };
            let me = Me {
                slot: 1,
                etype: 4,
                pos,
                yaw: 0.0,
                rad: 1.5,
                hht: 3.0,
                spd: 0.1,
                visactive: true,
                busy: None,
                reloading: false,
                has_generator: false,
                hatch: false,
            };
            let out = b.think(&me, &f, &mut [0.0], &walls, &mut dice, None);
            if out.explode {
                // 240 ticks of running.
                let m = first_move.expect("ran");
                assert!((t - m) as i32 >= 239, "blew up after {} ticks", t - m);
                return;
            }
            if out.trans.length() > 0.0 && first_move.is_none() {
                first_move = Some(t);
                // A second's crouch first; then 1.5 x speed.
                assert!(t >= 60, "ran at {t}");
                assert!((out.trans.length() - 0.15).abs() < 1e-4);
            }
            pos += out.trans;
            b.after_move(
                4,
                pos,
                Report {
                    tried: out.trans.length() > 0.001,
                    ..Default::default()
                },
            );
        }
        panic!("never blew up");
    }

    #[test]
    fn archer_throws_from_afar_and_backs_off_close() {
        let walls = Walls(Vec::new());
        let paths = Paths::default();
        let mut dice = Seq(5);
        let step = |b: &mut Brain, d: f32, dice: &mut Seq, t: u32| {
            let players = [Seen {
                pos: Vec3::new(0.0, 0.0, d),
                active: true,
                ..Default::default()
            }];
            let f = Frame {
                players: &players,
                it: None,
                bomber: None,
                paths: &paths,
                frame: t,
                ticks: 1,
                mrate: 1.0,
            };
            let me = Me {
                slot: 0,
                etype: 4,
                pos: Vec3::ZERO,
                yaw: 0.0,
                rad: 1.5,
                hht: 3.0,
                spd: 0.1,
                visactive: true,
                busy: None,
                reloading: false,
                has_generator: false,
                hatch: false,
            };
            b.think(&me, &f, &mut [0.0], &walls, dice, None)
        };
        let mut b = Brain::new(4, 16, 0.0, 30.0).with_algorithm(16);
        let mut acts = Vec::new();
        for t in 0..20 {
            acts.push(step(&mut b, 25.0, &mut dice, t).daction);
        }
        assert!(acts.contains(&act::THROW), "{acts:?}");
        // Inside 0.6 x sight it backs away (once granted the run-attack).
        let mut back = false;
        for t in 20..60 {
            let o = step(&mut b, 10.0, &mut dice, t);
            back |= o.trans.z < 0.0;
        }
        assert!(back);
    }
}
