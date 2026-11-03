//! The loyal follower: a hero who plays beside you (`--companion VAL`). It sees
//! the game only as `&Game` and answers with a tick's input, so it has no
//! powers a player lacks, and a game with it records and replays like any other.
//!
//! It stays near the nearest other hero, fights what comes near them, leaves the
//! loot (food aside, when it is the one who is hurt), goes to an exit only when
//! its leader is waiting on one, and, if nobody else is left playing, makes for
//! the nearest exit so the party can go on.
//!
//! The game keeps every hero inside the camera's view (CameraLimitPlayerDpos), so
//! a walk that leads out of frame is refused, and a hero left behind holds the
//! others back. So it follows the leader's own footsteps while it can: that way
//! is known to walk, and it leads toward them. The nav graph is for the rest.

use super::bot::{boss_point, seg_dist};
use super::nav::{xz, Goal, Nav, WalkWorld, CELL};
use crate::game::{max_health, EnemyState, Game, Kind, PickupKind, Player, PLAYER_RADIUS};
use crate::input::PlayerInput;
use gdl_formats::wdata::TOWER_WAVE;
use glam::{Vec2, Vec3};
use std::collections::VecDeque;

/// Close enough to the leader, and how far they get before it sets off.
const NEAR: f32 = 4.0;
const FAR: f32 = 7.0;
/// More height between them than this is another floor.
const FLOOR: f32 = 3.0;
/// It swings at what is this close; hunts what is this close to the leader.
const REACH: f32 = 4.0;
const HUNT: f32 = 8.0;
/// Further than this from the leader, only what is on top of it gets fought.
const LEASH: f32 = 14.0;
/// Food it will go for: this far from it, and from the leader.
const FOOD_RANGE: f32 = 12.0;
const FOOD_LEASH: f32 = 16.0;
/// Hurt below this share of its health, it looks for food.
const HUNGRY: f32 = 0.5;
/// How long a path lasts, and how soon a moved target can have a new one.
const PLAN_EVERY: f32 = 1.0;
const PLAN_MOVED: f32 = 0.3;
/// A search gives up here, and at this much of a tick (it picks up the map as
/// it goes, so the next try gets further): a follower must never hitch the game.
const MAX_EXPAND: usize = 20_000;
const PLAN_BUDGET: std::time::Duration = std::time::Duration::from_micros(2500);
const STUCK_SECS: f32 = 2.5;
/// The leader's footsteps: one a unit apart, this many kept.
const TRAIL: usize = 240;
/// Footsteps looked at for one in a straight walk, and how often.
const TRAIL_LOOKS: usize = 16;
const TRAIL_EVERY: u32 = 10;

pub struct Companion {
    pi: usize,
    level: String,
    nav: Nav,
    path: Vec<u32>,
    wp: usize,
    target: Option<Vec3>,
    planned: f32,
    /// Where it last got somewhere, and when: replanning doesn't count.
    anchor: (Vec3, f32),
    on_pad: Option<f32>,
    potion_t: f32,
    walking: bool,
    last_pos: Vec3,
    /// Where the leader has been, oldest first, and whose steps they are.
    trail: VecDeque<Vec3>,
    trail_of: usize,
    /// The footstep being walked to, and no trail until this time if it failed.
    step: Option<Vec3>,
    no_trail_until: f32,
}

/// What it could fight from where it stands.
struct Foes {
    /// How many are close.
    near: usize,
    /// The closest in reach, and the closest worth walking to.
    swing: Option<Vec3>,
    hunt: Option<Vec3>,
}

impl Companion {
    pub fn new(g: &Game, pi: usize) -> Companion {
        Companion {
            pi,
            level: g.level.name.clone(),
            nav: Nav::new(g),
            path: Vec::new(),
            wp: 0,
            target: None,
            planned: f32::MIN,
            anchor: (g.players.get(pi).map_or(Vec3::ZERO, |p| p.pos), g.time),
            on_pad: None,
            potion_t: f32::MIN,
            walking: false,
            last_pos: g.players.get(pi).map_or(Vec3::ZERO, |p| p.pos),
            trail: VecDeque::new(),
            trail_of: usize::MAX,
            step: None,
            no_trail_until: f32::MIN,
        }
    }

    /// Is this the brain for that hero in this level?
    pub fn is_for(&self, g: &Game, pi: usize) -> bool {
        self.pi == pi && self.level == g.level.name
    }

    /// What it does this tick.
    pub fn input(&mut self, g: &Game) -> PlayerInput {
        let Some(me) = g.players.get(self.pi) else {
            return PlayerInput::default();
        };
        // A scroll stops the game: it reads its own and leaves yours to you.
        if let Some(b) = &g.scroll_box {
            return PlayerInput {
                b_pressed: b.player == Some(self.pi) && b.ticks >= 30,
                ..Default::default()
            };
        }
        if !me.state.active() || g.sumner.as_ref().is_some_and(|s| s.controls_locked()) {
            return PlayerInput::default();
        }
        if g.frame % 30 == 0 && self.nav.refresh(g) {
            self.planned = f32::MIN;
        }
        // Moved by the game (a transporter, put back on the floor).
        if (me.pos - self.last_pos).length() > 3.0 {
            self.planned = f32::MIN;
            self.on_pad = None;
        }
        self.last_pos = me.pos;
        let mut inp = self.decide(g, me);
        // Whatever it picks up, it uses.
        if g.frame % 30 == 0 {
            inp.inv = g.bot_input(self.pi).inv;
        }
        inp
    }

    fn decide(&mut self, g: &Game, me: &Player) -> PlayerInput {
        let leader = g
            .players
            .iter()
            .enumerate()
            .filter(|(i, p)| *i != self.pi && p.state.active())
            .min_by(|a, b| {
                (a.1.pos - me.pos)
                    .length()
                    .total_cmp(&(b.1.pos - me.pos).length())
            });
        if let Some((i, l)) = leader {
            self.leave_footprint(i, l.pos);
        }
        let leader = leader.map(|(_, l)| l);
        let foes = foes(g, me, leader.map(|l| l.pos));
        let Some(lead) = leader else {
            // Everyone else is out: the level is left to it to finish.
            if let Some(inp) = self.fight(g, me, &foes, false) {
                return inp;
            }
            return match nearest_exit(g, me).filter(|_| g.wave != Some(TOWER_WAVE)) {
                Some((goal, at)) if !goal.holds(me.pos) => self.go(g, vec![goal], at),
                _ => PlayerInput::default(),
            };
        };
        let lp = lead.pos;
        let apart = xz(lp - me.pos).length();
        let other_floor = (lp.y - me.pos.y).abs() > FLOOR;
        // The party is leaving: nobody goes until every hero waits on the same way out.
        if lead.exit.waiting {
            if let Some(dest) = lead.exit.touch {
                if me.exit.touch == Some(dest) {
                    return PlayerInput::default();
                }
                if let Some((goal, at)) = exit_to(g, dest, lp) {
                    return self.go(g, vec![goal], at);
                }
            }
        }
        if let Some(inp) = self.fight(g, me, &foes, apart > LEASH || other_floor) {
            return inp;
        }
        if let Some((goal, at)) = self.food(g, me, lead) {
            return self.go(g, vec![goal], at);
        }
        if self.walking {
            self.walking = apart >= NEAR || other_floor;
        } else {
            self.walking = apart > FAR || other_floor;
        }
        if self.walking {
            if let Some(at) = self.footstep(g, me.pos) {
                self.path.clear();
                self.target = Some(at);
                return self.walk(g);
            }
            return self.go(g, vec![Goal::new(lp, NEAR - 1.0, FLOOR)], lp);
        }
        self.step = None;
        // Not waiting on an exit that nobody else wants.
        if me.exit.touch.is_some() && me.exit.idle >= 3 {
            return PlayerInput {
                stick: g.stick_toward(xz(lp - me.pos).normalize_or_zero()),
                ..Default::default()
            };
        }
        // Up close there is nothing to walk for, so no stale path is kept.
        self.path.clear();
        self.target = None;
        PlayerInput::default()
    }

    /// Swing at what is in reach, or walk to what is near the party.
    fn fight(&mut self, g: &Game, me: &Player, foes: &Foes, leashed: bool) -> Option<PlayerInput> {
        // Left behind, it stops for nothing.
        if leashed {
            return None;
        }
        if let Some(at) = foes.swing {
            if !clear_of_friends(g, self.pi, me, at) {
                return None;
            }
            let d = xz(at - me.pos).normalize_or_zero();
            let turbo = foes.near >= 3 && me.power >= 60.0 && g.frame % 120 < 12;
            let mut inp = PlayerInput {
                stick: g.stick_toward(d) * 0.06,
                attack: true,
                attack_pressed: g.frame % 4 == 0 || (turbo && g.frame % 120 == 11),
                turbo,
                ..Default::default()
            };
            let hard_pressed = me.health < max_health(me.level) * 0.4 && foes.near >= 3;
            if me.potions > 0 && hard_pressed && g.time - self.potion_t > 5.0 {
                self.potion_t = g.time;
                inp.magic = true;
                inp.magic_pressed = true;
            }
            return Some(inp);
        }
        let at = foes.hunt?;
        Some(self.go(g, vec![Goal::new(at, REACH - 1.0, 3.5)], at))
    }

    /// Food within reach, if it is hurt and hurt more than the leader.
    fn food(&self, g: &Game, me: &Player, lead: &Player) -> Option<(Goal, Vec3)> {
        let mine = me.health / max_health(me.level);
        let theirs = lead.health / max_health(lead.level);
        if mine >= HUNGRY || mine > theirs {
            return None;
        }
        g.entities
            .iter()
            .filter(|e| e.alive)
            .filter_map(|e| match e.kind {
                Kind::Pickup {
                    kind: PickupKind::Food(heals),
                    radius,
                    ..
                } if heals > 0.0 => Some((e.pos, radius)),
                _ => None,
            })
            .filter(|(at, _)| {
                xz(*at - me.pos).length() < FOOD_RANGE
                    && xz(*at - lead.pos).length() < FOOD_LEASH
                    && (at.y - me.pos.y).abs() < FLOOR
            })
            .min_by(|a, b| (a.0 - me.pos).length().total_cmp(&(b.0 - me.pos).length()))
            .map(|(at, r)| (Goal::new(at, (r + 0.3).max(1.0), 3.0), at))
    }

    fn leave_footprint(&mut self, who: usize, at: Vec3) {
        // Another leader, or sent somewhere by the game: the old steps don't join up.
        let jumped = self.trail.back().is_some_and(|l| (*l - at).length() > 5.0);
        if who != self.trail_of || jumped {
            self.trail.clear();
            self.step = None;
            self.trail_of = who;
        }
        if self.trail.back().is_none_or(|l| (*l - at).length() >= 1.0) {
            self.trail.push_back(at);
            if self.trail.len() > TRAIL {
                self.trail.pop_front();
            }
        }
    }

    /// The newest of the leader's footsteps it can walk straight to.
    fn footstep(&mut self, g: &Game, pos: Vec3) -> Option<Vec3> {
        if g.time < self.no_trail_until {
            self.step = None;
            return None;
        }
        if g.frame % TRAIL_EVERY != 0 {
            if let Some(s) = self.step.filter(|s| xz(*s - pos).length() > 1.0) {
                return Some(s);
            }
        }
        let w = WalkWorld {
            col: &g.collision,
            ents: &g.entities,
            ignore: &self.nav.ignore,
            min_y: g.level.worlds.min_bounds[1],
            cracks: Default::default(),
        };
        self.step = self
            .trail
            .iter()
            .rev()
            .step_by((self.trail.len() / TRAIL_LOOKS).max(1))
            .take(TRAIL_LOOKS)
            .find(|s| (s.y - pos.y).abs() < 2.5 && w.walks(pos, **s))
            .copied();
        self.step
    }

    /// Toward `at`, over the nav graph.
    fn go(&mut self, g: &Game, goals: Vec<Goal>, at: Vec3) -> PlayerInput {
        let moved = self.target.is_none_or(|t| (t - at).length() > 2.0);
        let age = g.time - self.planned;
        if age > PLAN_EVERY || (moved && age > PLAN_MOVED) {
            self.plan(g, &goals);
        }
        self.target = Some(at);
        self.walk(g)
    }

    fn plan(&mut self, g: &Game, goals: &[Goal]) {
        self.planned = g.time;
        let pos = g.players[self.pi].pos;
        let deadline = std::time::Instant::now() + PLAN_BUDGET;
        let path = self.nav.start_node(g, pos).and_then(|s| {
            self.nav
                .path_until(g, s, goals, g.time, MAX_EXPAND, Some(deadline))
        });
        // Out of time, or no way there: the old path may still be some use.
        let Some((path, _)) = path else {
            // It learned more of the map: straight on with the search next tick.
            if std::time::Instant::now() >= deadline {
                self.planned = f32::MIN;
            }
            return;
        };
        self.path = path;
        self.wp = 0;
        self.on_pad = None;
    }

    /// Along the path, then straight at the target.
    fn walk(&mut self, g: &Game) -> PlayerInput {
        let now = g.time;
        let pos = g.players[self.pi].pos;
        // Waypoints reached.
        while self.wp < self.path.len() {
            let n = self.nav.nodes[self.path[self.wp] as usize];
            let last = self.wp + 1 == self.path.len();
            let d = xz(n.pos - pos).length();
            let passed = !last && d < 2.5 && {
                let next = self.nav.nodes[self.path[self.wp + 1] as usize].pos;
                xz(next - pos).length() + 0.3 < xz(next - n.pos).length()
            };
            if (d < if last { 0.6 } else { 1.0 } || passed) && (n.pos.y - pos.y).abs() < 2.0 {
                self.wp += 1;
            } else {
                break;
            }
        }
        self.short_cut(g, pos);
        let goal = match self.pad_goal(g, pos) {
            PadStep::Wait => return PlayerInput::default(),
            PadStep::Go(p) => p,
            PadStep::None if self.wp < self.path.len() => {
                self.nav.nodes[self.path[self.wp] as usize].pos
            }
            PadStep::None => match self.target {
                Some(t) => t,
                None => return PlayerInput::default(),
            },
        };
        let d = xz(goal - pos);
        if (pos - self.anchor.0).length() > 1.0 {
            self.anchor = (pos, now);
        }
        // Stuck: that step is no good for a while.
        if now - self.anchor.1 > STUCK_SECS {
            if self.wp > 0 && self.wp < self.path.len() {
                let step = (self.path[self.wp - 1], self.path[self.wp]);
                self.nav.blocked.insert(step, now + 60.0);
            }
            if self.step.is_some() {
                self.no_trail_until = now + 3.0;
                self.step = None;
            }
            self.planned = f32::MIN;
            self.anchor = (pos, now);
        }
        let mut s = d.normalize_or_zero();
        // Not getting anywhere: try another heading, as a player would.
        if now - self.anchor.1 > 1.0 {
            let (sn, cs) = ((g.frame as f32 * 0.07).sin() * 0.7).sin_cos();
            s = Vec2::new(s.x * cs - s.y * sn, s.x * sn + s.y * cs);
        }
        let slow = self.wp >= self.path.len() && d.length() < 1.5;
        PlayerInput {
            stick: g.stick_toward(s) * if slow { 0.5 } else { 1.0 },
            ..Default::default()
        }
    }

    /// Skip ahead when a later waypoint is straight ahead and walkable.
    fn short_cut(&mut self, g: &Game, pos: Vec3) {
        if self.wp + 1 >= self.path.len() || g.frame % 6 != 0 {
            return;
        }
        let w = WalkWorld {
            col: &g.collision,
            ents: &g.entities,
            ignore: &self.nav.ignore,
            min_y: g.level.worlds.min_bounds[1],
            cracks: Default::default(),
        };
        let far = (self.wp + 4).min(self.path.len() - 1);
        for k in (self.wp + 1..=far).rev() {
            let n = self.nav.nodes[self.path[k] as usize];
            let pad = (self.wp.max(1)..=k)
                .any(|j| self.nav.pad_edge(self.path[j - 1], self.path[j]).is_some());
            // It never wanders into a secret level on its own.
            let secret = g.entities.iter().any(|e| {
                matches!(e.kind, Kind::Secret { radius, used: false, .. }
                    if seg_dist(xz(pos), xz(n.pos), xz(e.pos)) < radius + PLAYER_RADIUS + 0.5
                        && (e.pos.y - pos.y).abs() < 4.0)
            });
            if !pad && !n.hurts && !secret && w.walks(pos, n.pos) {
                self.wp = k;
                return;
            }
        }
    }

    /// A transporter on the path: touch the pad and stand there to be sent.
    fn pad_goal(&mut self, g: &Game, pos: Vec3) -> PadStep {
        if self.wp == 0 || self.wp >= self.path.len() {
            self.on_pad = None;
            return PadStep::None;
        }
        let (from, to) = (self.path[self.wp - 1], self.path[self.wp]);
        let Some(pad) = self.nav.pad_edge(from, to) else {
            self.on_pad = None;
            return PadStep::None;
        };
        let dest = self.nav.nodes[to as usize].pos;
        if xz(dest - pos).length() < CELL * 1.5 + 0.5 && (dest.y - pos.y).abs() < 2.5 {
            // Sent across.
            self.wp += 1;
            self.on_pad = None;
            self.anchor = (pos, g.time);
            return PadStep::None;
        }
        let p = &g.transport.pads[pad];
        if xz(p.pos - pos).length() < p.radius + PLAYER_RADIUS - 0.4 {
            let since = *self.on_pad.get_or_insert(g.time);
            self.anchor = (pos, g.time);
            if g.time - since > 6.0 {
                self.nav.blocked.insert((from, to), g.time + 60.0);
                self.planned = f32::MIN;
                self.on_pad = None;
            }
            return PadStep::Wait;
        }
        PadStep::Go(p.pos)
    }
}

enum PadStep {
    None,
    Wait,
    Go(Vec3),
}

/// What is around the hero and their leader.
fn foes(g: &Game, me: &Player, leader: Option<Vec3>) -> Foes {
    let mut out = Foes {
        near: 0,
        swing: None,
        hunt: None,
    };
    let (mut swing_d, mut hunt_d) = (f32::MAX, f32::MAX);
    let mut see = |at: Vec3, r: f32, tall: bool| {
        if !tall && (at.y - me.pos.y).abs() > 3.5 {
            return;
        }
        let d = xz(at - me.pos).length() - r;
        if d < 6.0 {
            out.near += 1;
        }
        if d < REACH && d < swing_d {
            swing_d = d;
            out.swing = Some(at);
        }
        // It walks to what threatens the leader, or to anything when alone.
        let by_leader = leader.is_none_or(|l| xz(at - l).length() - r < HUNT);
        if by_leader && d < hunt_d {
            hunt_d = d;
            out.hunt = Some(at);
        }
    };
    for e in g.entities.iter().filter(|e| e.alive) {
        match &e.kind {
            Kind::Enemy { state, .. } if !matches!(state, EnemyState::Dying { .. }) => {
                see(e.pos, 1.0, false)
            }
            Kind::Generator { .. } => see(e.pos, super::bot::solid_radius(e), false),
            Kind::Critter(c) if c.hittable() => {
                if let Some((at, r)) = Game::aim_point(e, me.pos + Vec3::Y * 2.5) {
                    see(at, r, true);
                }
            }
            _ => {}
        }
    }
    if let Some((at, r)) = boss_point(g, me.pos) {
        see(at, r, true);
    }
    out
}

/// With friendly fire on, it holds its swing while a friend is in the way.
fn clear_of_friends(g: &Game, pi: usize, me: &Player, at: Vec3) -> bool {
    if g.friendly_fire == 0 {
        return true;
    }
    let aim = xz(at - me.pos).normalize_or_zero();
    !g.players.iter().enumerate().any(|(i, p)| {
        let to = xz(p.pos - me.pos);
        i != pi
            && p.state.active()
            && to.length() < REACH + 1.0
            && to.normalize_or_zero().dot(aim) > 0.6
    })
}

/// Standing on any exit will do when nobody else is left.
fn nearest_exit(g: &Game, me: &Player) -> Option<(Goal, Vec3)> {
    open_exits(g)
        .min_by(|a, b| (a.1 - me.pos).length().total_cmp(&(b.1 - me.pos).length()))
        .map(|(_, at, goal)| (goal, at))
}

/// The exit the leader is on: the nearest to them with that destination.
fn exit_to(g: &Game, dest: Option<u32>, near: Vec3) -> Option<(Goal, Vec3)> {
    open_exits(g)
        .filter(|(d, _, _)| *d == dest)
        .min_by(|a, b| (a.1 - near).length().total_cmp(&(b.1 - near).length()))
        .map(|(_, at, goal)| (goal, at))
}

fn open_exits(g: &Game) -> impl Iterator<Item = (Option<u32>, Vec3, Goal)> + '_ {
    g.entities
        .iter()
        .filter(|e| e.alive)
        .filter_map(|e| match e.kind {
            Kind::Exit {
                radius,
                height,
                dest,
                open: true,
                ..
            } => Some((
                dest,
                e.pos,
                Goal::new(e.pos, (radius * 0.7).max(0.8), height.max(2.0)),
            )),
            _ => None,
        })
}

/// What it presses in the shop: on through the tally and out, without waiting
/// for the others. Attack moves the screens on; B leaves the shelves.
pub fn shop_input(tick: u32) -> PlayerInput {
    PlayerInput {
        attack_pressed: tick % 16 == 0,
        b_pressed: tick % 16 == 8,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::GameFs;
    use crate::game::setup::{Command, LevelStart};
    use crate::game::LevelEnd;
    use crate::hud::PanelInfo;
    use crate::progress::Progress;
    use crate::replay::Replay;
    use crate::runner::bot::Bot;

    fn data() -> Option<GameFs> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game-data");
        GameFs::open(&root).ok()
    }

    fn pair_of(fs: &GameFs, level: &str) -> anyhow::Result<(Game, bool)> {
        LevelStart {
            level: level.into(),
            difficulty: 1,
            panels: vec![panel(0, 0), panel(1, 1)],
            ..Default::default()
        }
        .build(fs)
    }

    fn pair(fs: &GameFs, level: &str) -> Game {
        pair_of(fs, level).unwrap().0
    }

    fn panel(slot: usize, class: usize) -> PanelInfo {
        PanelInfo {
            slot,
            name: String::new(),
            colour: slot,
            class,
            hidden: None,
        }
    }

    /// Ticks with the leader doing nothing and the companion on its own.
    fn run(g: &mut Game, c: &mut Companion, ticks: u32) {
        for _ in 0..ticks {
            let inputs = [PlayerInput::default(), c.input(g)];
            g.update(&inputs);
        }
    }

    #[test]
    fn it_only_looks() {
        let Some(fs) = data() else { return };
        let mut g = pair(&fs, "levelA1");
        let mut c = Companion::new(&g, 1);
        for _ in 0..240 {
            let before = g.state_sections();
            let inp = c.input(&g);
            assert_eq!(before, g.state_sections());
            g.update(&[PlayerInput::default(), inp]);
        }
    }

    #[test]
    fn it_stays_with_a_leader_who_walks() {
        keeps_up(&["levelA1", "levelB1", "levelG1"], 0.8);
    }

    /// One level from most realms; slow, so asked for by name.
    #[test]
    #[ignore]
    fn it_keeps_up_across_the_realms() {
        keeps_up(
            &[
                "levelA2", "levelA5", "levelB3", "levelC1", "levelC4", "levelD1", "levelD3",
                "levelE1", "levelF2", "levelG3", "levelH1", "levelI1", "levelJ1", "levelK1",
            ],
            0.5,
        );
    }

    fn keeps_up(levels: &[&str], needs: f32) {
        let Some(fs) = data() else { return };
        for level in levels {
            let Ok((mut g, _)) = pair_of(&fs, level) else {
                println!("{level}: no such level");
                continue;
            };
            g.debug_god = true;
            let mut bot = Bot::new(&g, 600.0, false, false);
            let mut c = Companion::new(&g, 1);
            let (mut near, mut total, mut worst) = (0, 0, 0.0f32);
            let (mut slowest, mut start) = (0.0f64, g.players[0].pos);
            let mut walked = 0.0;
            for _ in 0..60 * 60 {
                let Ok(mut inputs) = bot.inputs(&mut g) else {
                    break;
                };
                let t = std::time::Instant::now();
                inputs[1] = c.input(&g);
                slowest = slowest.max(t.elapsed().as_secs_f64());
                g.update(&inputs);
                if g.level_end.is_some() {
                    break;
                }
                walked += (g.players[0].pos - start).length();
                start = g.players[0].pos;
                let d = xz(g.players[0].pos - g.players[1].pos).length();
                worst = worst.max(d);
                total += 1;
                near += (d < 14.0) as u32;
            }
            let share = near as f32 / total as f32;
            println!(
                "{level}: near {:.0}% of {total} ticks, worst {worst:.0}, leader walked {walked:.0}, slowest tick {:.1} ms",
                share * 100.0,
                slowest * 1000.0
            );
            // Alone, how far does that leader get? A follower must not hold them back.
            let mut solo = LevelStart {
                level: level.to_string(),
                difficulty: 1,
                panels: vec![panel(0, 0)],
                ..Default::default()
            }
            .build(&fs)
            .unwrap()
            .0;
            solo.debug_god = true;
            let mut bot = Bot::new(&solo, 600.0, false, false);
            let (mut alone, mut was) = (0.0, solo.players[0].pos);
            for _ in 0..60 * 60 {
                let Ok(inputs) = bot.inputs(&mut solo) else {
                    break;
                };
                solo.update(&inputs);
                if solo.level_end.is_some() {
                    break;
                }
                alone += (solo.players[0].pos - was).length();
                was = solo.players[0].pos;
            }
            println!("  alone the leader walked {alone:.0}");
            assert!(share > needs, "{level}: near only {share}");
            assert!(
                walked > alone * 0.5 || alone < 30.0,
                "{level}: it held the leader back: {walked:.0} against {alone:.0} alone"
            );
        }
    }

    #[test]
    fn it_fights_what_comes_near() {
        let Some(fs) = data() else { return };
        let mut g = pair(&fs, "levelA1");
        g.debug_god = true;
        let start = g.players[1].stats.enemies_killed + g.players[1].stats.generators_destroyed;
        let mut c = Companion::new(&g, 1);
        // The leader stands by the nearest generator, or monster.
        let at = g
            .entities
            .iter()
            .filter(|e| e.alive && matches!(e.kind, Kind::Generator { .. } | Kind::Enemy { .. }))
            .map(|e| e.pos)
            .min_by(|a, b| {
                (*a - g.players[0].pos)
                    .length()
                    .total_cmp(&(*b - g.players[0].pos).length())
            })
            .unwrap();
        g.players[0].pos = at + Vec3::new(3.0, 0.5, 0.0);
        run(&mut g, &mut c, 60 * 60);
        let done = g.players[1].stats.enemies_killed + g.players[1].stats.generators_destroyed;
        assert!(done > start, "it should have killed something");
    }

    #[test]
    fn it_leaves_with_the_party_and_not_before() {
        let Some(fs) = data() else { return };
        let mut g = pair(&fs, "levelL1");
        let mut c = Companion::new(&g, 1);
        run(&mut g, &mut c, 120);
        // Sumner's welcome, read and gone.
        for _ in 0..60 * 60 {
            let busy = g.sumner.as_ref().is_some_and(|s| s.controls_locked());
            if !busy && g.scroll_box.is_none() {
                break;
            }
            let b = PlayerInput {
                b_pressed: g.scroll_box.as_ref().is_some_and(|b| b.ticks >= 20),
                ..Default::default()
            };
            g.update(&[b, c.input(&g)]);
        }
        // Alone by a portal, it doesn't take it; with the leader on it, it does.
        let (dest, at) = g
            .entities
            .iter()
            .filter_map(|e| match e.kind {
                Kind::Exit {
                    dest: Some(d),
                    open: true,
                    ..
                } => Some((d, e.pos)),
                _ => None,
            })
            .last()
            .expect("an open portal");
        g.players[1].pos = at;
        g.players[0].pos = at + Vec3::new(4.0, 0.0, 0.0);
        run(&mut g, &mut c, 300);
        assert_eq!(g.level_end, None, "it doesn't take a portal on its own");
        assert!(!g.players[1].exit.waiting);
        g.players[1].pos = at + Vec3::new(4.0, 0.0, 0.0);
        g.players[0].pos = at;
        for _ in 0..60 * 60 {
            run(&mut g, &mut c, 1);
            if g.level_end.is_some() {
                break;
            }
        }
        assert_eq!(g.level_end, Some(LevelEnd::Exit(Some(dest))));
    }

    #[test]
    fn it_eats_only_when_it_needs_to() {
        let Some(fs) = data() else { return };
        let food_near = |g: &Game| {
            g.entities
                .iter()
                .filter(|e| e.alive)
                .filter(
                    |e| matches!(e.kind, Kind::Pickup { kind: PickupKind::Food(h), .. } if h > 0.0),
                )
                .map(|e| e.id)
                .collect::<Vec<_>>()
        };
        for hurt in [false, true] {
            let mut g = pair(&fs, "levelA1");
            g.debug_god = true;
            let mut c = Companion::new(&g, 1);
            let id = food_near(&g)[0];
            let at = g.entities.iter().find(|e| e.id == id).unwrap().pos;
            for p in &mut g.players {
                p.pos = at + Vec3::new(5.0, 0.5, 0.0);
            }
            if hurt {
                g.players[1].health = max_health(g.players[1].level) * 0.2;
            }
            run(&mut g, &mut c, 60 * 8);
            let eaten = !g.entities.iter().any(|e| e.id == id && e.alive);
            assert_eq!(eaten, hurt, "hurt {hurt}");
        }
    }

    #[test]
    fn a_game_with_it_replays_to_the_bit() {
        let Some(fs) = data() else { return };
        use crate::rules::Rules;
        use crate::session::{Decision, Event, Session};
        let mut s = Session::new(Rules::classic(), 1, 0, Progress::default());
        s.enter_level("levelA1".into(), vec![panel(0, 0), panel(1, 1)]);
        s.load(&fs).unwrap();
        let mut c = Companion::new(s.game.as_ref().unwrap(), 1);
        for _ in 0..600 {
            let inp = c.input(s.game.as_ref().unwrap()).pack();
            s.update(&fs, &[PlayerInput::default().pack(), inp]);
        }
        s.command(&fs, Decision::Game(Command::QuitLevel));
        s.update(&fs, &[]);
        let compat = crate::compat::Compat::local(&fs, &Rules::classic(), "test").unwrap();
        let text = s
            .events
            .iter()
            .find_map(|e| match e {
                Event::LevelGone {
                    game,
                    recording: Some(r),
                } => Some(r.to_text(game, &compat)),
                _ => None,
            })
            .unwrap();
        let v = Replay::from_text(&text).unwrap().check(&fs, None).unwrap();
        assert!(v.end_differs.is_empty() && v.diverged.is_none(), "{v}");
    }
}
