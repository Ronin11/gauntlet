//! Bug reports (developer tool; not in the original game).
//!
//! The debug menu's "File bug report" writes a folder of plain text and PNG
//! files that describe where the player is and what just happened, for the
//! developer to read:
//!
//! - `report.md`: time, build, level, players, camera, progress, options, the
//!   tags picked in the menu, and a headless command line that reproduces
//!   the spot;
//! - `nearby.txt`: everything within [`NEAR`] units of each hero (entities,
//!   placed items, triggers and movers, transporters, world objects with
//!   their collision, locators, exits);
//! - `events.log`: the last [`KEEP_SECS`] seconds of events (log lines,
//!   messages, sounds, trigger and mover changes, pickups, damage, deaths,
//!   level changes, debug menu actions, and a per-second input summary);
//! - `screenshot.png`: the last frame (without the debug menu);
//! - `save.txt` and a copy of the memory card file.
//!
//! Reports go to `<save dir>/bugs/<YYYYMMDD-HHMMSS>-<level>/` (UTC time):
//! `$XDG_DATA_HOME/gauntlet-rs/bugs` (default `~/.local/share/gauntlet-rs/bugs`)
//! on desktop; the app's external files folder on Android.
//!
//! The event ring buffer runs all the time. Its feeds cost next to nothing:
//! a tee on the `log` logger, one call per sound, and [`Recorder::observe`]
//! once a tick, which compares a few small arrays with the previous tick's.

use crate::game::{Game, Kind, PlayerState};
use crate::hud::PanelInfo;
use crate::input::PlayerInput;
use crate::options::Prefs;
use crate::progress::Progress;
use crate::save::HeroSave;
use anyhow::{Context, Result};
use glam::Vec3;
use std::collections::{HashMap, VecDeque};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

/// Seconds of events kept.
pub const KEEP_SECS: f64 = 120.0;
/// At most this many events are kept, whatever their age.
pub const CAP: usize = 20_000;
/// nearby.txt's radius around each hero.
pub const NEAR: f32 = 40.0;
/// The quick tags the menu offers.
pub const TAGS: [&str; 6] = [
    "stuck",
    "can't reach",
    "wrong look",
    "wrong sound",
    "crash-ish",
    "other",
];
/// The build: `git describe` at compile time (build.rs), else the crate version.
pub const BUILD: &str = match option_env!("GDL_GIT_DESCRIBE") {
    Some(d) => d,
    None => env!("CARGO_PKG_VERSION"),
};

/// One recorded event.
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    /// Seconds since the program started.
    pub t: f64,
    /// The game's tick (Game::frame) when it happened.
    pub tick: u32,
    pub cat: &'static str,
    pub text: String,
}

/// The last `keep` seconds (and at most `cap`) of events.
#[derive(Debug)]
pub struct Ring {
    events: VecDeque<Event>,
    keep: f64,
    cap: usize,
}

impl Ring {
    pub const fn new(keep: f64, cap: usize) -> Ring {
        Ring {
            events: VecDeque::new(),
            keep,
            cap,
        }
    }

    pub fn push(&mut self, e: Event) {
        let oldest = e.t - self.keep;
        while self.events.front().is_some_and(|f| f.t < oldest) {
            self.events.pop_front();
        }
        while self.events.len() >= self.cap.max(1) {
            self.events.pop_front();
        }
        self.events.push_back(e);
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Event> {
        self.events.iter()
    }

    /// One line an event: `[  12.345s t=740 ] cat  text`.
    pub fn dump(&self) -> String {
        let mut s = String::new();
        for e in &self.events {
            let _ = writeln!(s, "{}", format_event(e));
        }
        s
    }
}

pub fn format_event(e: &Event) -> String {
    format!("[{:9.3}s t={:<6}] {:<6} {}", e.t, e.tick, e.cat, e.text)
}

static RING: Mutex<Ring> = Mutex::new(Ring::new(KEEP_SECS, CAP));
static TICK: AtomicU32 = AtomicU32::new(0);
static START: OnceLock<Instant> = OnceLock::new();
/// Ticks observed in all levels, for the headless clock.
static TICKS_SEEN: AtomicU32 = AtomicU32::new(0);
static TICK_CLOCK: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Time events by the ticks observed (1/60 s each) instead of the wall clock:
/// a headless simulation runs far faster than real time.
pub fn use_tick_clock() {
    TICK_CLOCK.store(true, Ordering::Relaxed);
}

/// Seconds since the first event (or the logger's installation), or the
/// ticks observed / 60 under [`use_tick_clock`].
pub fn now() -> f64 {
    if TICK_CLOCK.load(Ordering::Relaxed) {
        return TICKS_SEEN.load(Ordering::Relaxed) as f64 / 60.0;
    }
    START.get_or_init(Instant::now).elapsed().as_secs_f64()
}

/// Record an event in the global ring.
pub fn event(cat: &'static str, text: impl Into<String>) {
    let e = Event {
        t: now(),
        tick: TICK.load(Ordering::Relaxed),
        cat,
        text: text.into(),
    };
    RING.lock().unwrap_or_else(|p| p.into_inner()).push(e);
}

/// The global ring's events, one a line.
pub fn events_text() -> String {
    RING.lock().unwrap_or_else(|p| p.into_inner()).dump()
}

/// A logger that hands records to `inner` and keeps info and above in the ring.
struct Tee {
    inner: Box<dyn log::Log>,
}

/// Info lines from the graphics stack are many and not about the game.
fn noisy(target: &str) -> bool {
    ["wgpu", "naga", "winit", "calloop", "sctk"]
        .iter()
        .any(|t| target.starts_with(t))
}

impl log::Log for Tee {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.level() <= log::Level::Info || self.inner.enabled(m)
    }

    fn log(&self, r: &log::Record) {
        if self.inner.enabled(r.metadata()) {
            self.inner.log(r);
        }
        let l = r.level();
        if l <= log::Level::Warn || (l == log::Level::Info && !noisy(r.target())) {
            event("log", format!("{l} {}: {}", r.target(), r.args()));
        }
    }

    fn flush(&self) {
        self.inner.flush();
    }
}

/// Install `inner` (env_logger, android_logger) as the logger, with the ring
/// buffer beside it. `max` is the inner logger's own filter.
pub fn install_logger(inner: Box<dyn log::Log>, max: log::LevelFilter) {
    let _ = now();
    if log::set_boxed_logger(Box::new(Tee { inner })).is_ok() {
        log::set_max_level(max.max(log::LevelFilter::Info));
    }
}

/// A sound the game asked for.
pub fn sound(name: &str) {
    event("sound", name);
}

/// The desktop's report folder: `<save dir>/bugs`.
pub fn default_dir() -> Option<PathBuf> {
    crate::save::MemCard::default_dir().map(|d| d.join("bugs"))
}

static DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Send reports to `dir` instead (Android: the app's external files folder).
pub fn set_dir(dir: PathBuf) {
    *DIR.lock().unwrap_or_else(|p| p.into_inner()) = Some(dir);
}

/// Where reports go: what [`set_dir`] set, else [`default_dir`].
pub fn dir() -> Option<PathBuf> {
    DIR.lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone()
        .or_else(default_dir)
}

/// [`dir`] for the menu, with the home folder as `~`.
pub fn shown_dir() -> String {
    let Some(d) = dir() else {
        return "(nowhere: no home folder)".into();
    };
    let s = d.display().to_string();
    match std::env::var("HOME") {
        Ok(h) if !h.is_empty() && s.starts_with(&h) => format!("~{}", &s[h.len()..]),
        _ => s,
    }
}

// ---------------------------------------------------------------------------
// The per-tick observer.

#[derive(Clone, Debug, Default, PartialEq)]
struct PlayerSnap {
    health: f32,
    gold: u32,
    keys: u32,
    potions: u32,
    level: u32,
    state: &'static str,
    powerups: Vec<(u8, u32, u8)>,
    exit_waiting: bool,
    transporting: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct MoverSnap {
    active: bool,
    held: bool,
    shown: bool,
    moving: bool,
}

/// A tracked entity: its hit points (if it has any), and whether a trigger is pressed.
#[derive(Clone, Copy, Debug, PartialEq)]
struct EntSnap {
    hp: f32,
    pressed: bool,
    /// What it is (kind_tag, monster class, tier) and where, for its removal.
    tag: &'static str,
    class: usize,
    strength: usize,
    pos: Vec3,
}

/// One player's input over a second.
#[derive(Clone, Debug, Default)]
struct InputSum {
    ticks: u32,
    stick: glam::Vec2,
    stick_max: f32,
    moving: u32,
    attack: u32,
    magic: u32,
    turbo: u32,
    b: u32,
    start: u32,
    charge: u32,
    strafe: u32,
    slow: u32,
    fire: u32,
    dpad: u32,
    hurt: f32,
    healed: f32,
}

impl InputSum {
    fn add(&mut self, i: &PlayerInput) {
        self.ticks += 1;
        self.stick += i.stick;
        self.stick_max = self.stick_max.max(i.stick.length());
        self.moving += (i.stick.length() > 0.2) as u32;
        self.attack += i.attack_pressed as u32;
        self.magic += i.magic_pressed as u32;
        self.turbo += i.turbo as u32;
        self.b += i.b_pressed as u32;
        self.start += i.start_pressed as u32;
        self.charge += i.charge_pressed as u32;
        self.strafe += i.strafe_pressed as u32;
        self.slow += i.slow_pressed as u32;
        self.fire += (i.fire.length() > 0.2) as u32;
        self.dpad += (i.dpad.length() > 0.5) as u32;
    }

    fn idle(&self) -> bool {
        self.moving
            + self.attack
            + self.magic
            + self.turbo
            + self.b
            + self.start
            + self.charge
            + self.strafe
            + self.slow
            + self.fire
            + self.dpad
            == 0
            && self.hurt == 0.0
            && self.healed == 0.0
    }

    /// `stick avg (x,y) max m moving n/60 | attack 3 turbo 40/60 ...`
    pub fn summary(&self) -> String {
        let n = self.ticks.max(1) as f32;
        let avg = self.stick / n;
        let mut s = format!(
            "stick avg ({:+.2},{:+.2}) max {:.2} moving {}/{}",
            avg.x, avg.y, self.stick_max, self.moving, self.ticks
        );
        for (name, v, held) in [
            ("attack", self.attack, false),
            ("magic", self.magic, false),
            ("turbo", self.turbo, true),
            ("B", self.b, false),
            ("start", self.start, false),
            ("charge", self.charge, false),
            ("strafe", self.strafe, false),
            ("slow", self.slow, false),
            ("cstick", self.fire, true),
            ("dpad", self.dpad, true),
        ] {
            if v > 0 {
                if held {
                    let _ = write!(s, " {name} {v}/{}", self.ticks);
                } else {
                    let _ = write!(s, " {name} x{v}");
                }
            }
        }
        if self.hurt > 0.0 {
            let _ = write!(s, " | hurt {:.1}", self.hurt);
        }
        if self.healed > 0.0 {
            let _ = write!(s, " | healed {:.1}", self.healed);
        }
        s
    }
}

/// Watches the game once a tick and records what changed.
#[derive(Default)]
pub struct Recorder {
    level: String,
    last_frame: u32,
    players: Vec<PlayerSnap>,
    movers: Vec<MoverSnap>,
    ents: HashMap<u64, EntSnap>,
    scratch: HashMap<u64, EntSnap>,
    msg: Option<(usize, String)>,
    progress: (u16, u16, u16, [i32; 9], [i32; 3]),
    level_end: Option<String>,
    trigger_cam: bool,
    scroll: bool,
    inputs: Vec<InputSum>,
    dealt: f32,
    kills: u32,
    second_ticks: u32,
}

fn state_name(s: &PlayerState) -> &'static str {
    match s {
        PlayerState::Normal => "normal",
        PlayerState::Dead { .. } => "dead",
        PlayerState::Exiting { .. } => "exiting",
        PlayerState::Exited => "exited",
        PlayerState::InTower => "in tower",
    }
}

/// A short name for an entity, for events.
pub fn entity_name(g: &Game, k: &Kind) -> String {
    match k {
        Kind::Enemy {
            class, strength, ..
        } => format!(
            "{} (tier {strength})",
            g.classes.get(*class).map_or("?", |c| c.name.as_str())
        ),
        Kind::Generator {
            class, strength, ..
        } => format!(
            "generator {} (tier {strength})",
            g.classes.get(*class).map_or("?", |c| c.name.as_str())
        ),
        Kind::Container { chest, sub, .. } => {
            if *chest {
                format!("chest (sub {sub:#x})")
            } else {
                format!("breakable (sub {sub:#x})")
            }
        }
        Kind::Critter(c) => format!("critter {}", c.prefix),
        Kind::Trigger { id, .. } => format!("trigger {id}"),
        other => kind_tag(other).to_string(),
    }
}

/// The variant's name.
pub fn kind_tag(k: &Kind) -> &'static str {
    match k {
        Kind::Enemy { .. } => "Enemy",
        Kind::Generator { .. } => "Generator",
        Kind::Pickup { .. } => "Pickup",
        Kind::Container { .. } => "Container",
        Kind::Door { .. } => "Door",
        Kind::Exit { .. } => "Exit",
        Kind::Hazard { .. } => "Hazard",
        Kind::Secret { .. } => "Secret",
        Kind::Projectile { .. } => "Projectile",
        Kind::EnemyShot { .. } => "EnemyShot",
        Kind::Critter(_) => "Critter",
        Kind::Effect { .. } => "Effect",
        Kind::Trigger { .. } => "Trigger",
        Kind::Prop => "Prop",
        Kind::Faller { .. } => "Faller",
    }
}

fn v3(v: Vec3) -> String {
    format!("({:.2}, {:.2}, {:.2})", v.x, v.y, v.z)
}

fn v3c(v: Vec3) -> String {
    format!("{:.2},{:.2},{:.2}", v.x, v.y, v.z)
}

impl Recorder {
    pub fn new() -> Recorder {
        Recorder::default()
    }

    /// After a tick of `g` with these inputs (one per player).
    pub fn observe(&mut self, g: &Game, inputs: &[PlayerInput]) {
        TICK.store(g.frame, Ordering::Relaxed);
        TICKS_SEEN.fetch_add(1, Ordering::Relaxed);
        // A new level (or the same one loaded again).
        if g.level.name != self.level || g.frame < self.last_frame {
            self.level = g.level.name.clone();
            *self = Recorder {
                level: g.level.name.clone(),
                ..Recorder::default()
            };
            event(
                "level",
                format!(
                    "started {} wave {} with {} player(s)",
                    g.level.name,
                    g.wave.map_or("none".into(), |w| format!("{w:#x}")),
                    g.players.len()
                ),
            );
            self.seed(g);
        }
        self.last_frame = g.frame;

        // Players.
        self.inputs.resize_with(g.players.len(), Default::default);
        for (i, p) in g.players.iter().enumerate() {
            let now = player_snap(g, i);
            if let Some(i_) = inputs.get(i) {
                self.inputs[i].add(i_);
            }
            if let Some(old) = self.players.get(i) {
                let dh = now.health - old.health;
                if dh < 0.0 {
                    self.inputs[i].hurt -= dh;
                } else if dh > 0.0 {
                    self.inputs[i].healed += dh;
                }
                if dh.abs() >= 5.0 {
                    event(
                        "health",
                        format!("P{} {:+.1} -> {:.1}", i + 1, dh, now.health),
                    );
                }
                for (what, a, b) in [
                    ("gold", old.gold, now.gold),
                    ("keys", old.keys, now.keys),
                    ("potions", old.potions, now.potions),
                    ("level", old.level, now.level),
                ] {
                    if a != b {
                        event(
                            "pickup",
                            format!(
                                "P{} {what} {:+} -> {b} at {}",
                                i + 1,
                                b as i64 - a as i64,
                                v3(p.pos)
                            ),
                        );
                    }
                }
                if old.state != now.state {
                    event(
                        "player",
                        format!("P{} {} -> {} at {}", i + 1, old.state, now.state, v3(p.pos)),
                    );
                }
                if old.powerups != now.powerups {
                    event(
                        "pickup",
                        format!("P{} power-ups now {}", i + 1, powerups_text(p)),
                    );
                }
                if old.exit_waiting != now.exit_waiting {
                    event(
                        "exit",
                        format!(
                            "P{} {} on an exit at {}",
                            i + 1,
                            if now.exit_waiting {
                                "waiting"
                            } else {
                                "no longer waiting"
                            },
                            v3(p.pos)
                        ),
                    );
                }
                if old.transporting != now.transporting && now.transporting {
                    event(
                        "trans",
                        format!("P{} transporting from {}", i + 1, v3(p.pos)),
                    );
                }
            }
            if i < self.players.len() {
                self.players[i] = now;
            } else {
                self.players.push(now);
            }
        }

        // Movers.
        for (i, m) in g.movers.iter().enumerate() {
            let now = MoverSnap {
                active: m.active,
                held: m.held,
                shown: m.shown,
                moving: m.moving,
            };
            match self.movers.get_mut(i) {
                Some(old) if *old != now => {
                    let name = g
                        .level
                        .worlds
                        .world_objects
                        .get(m.root)
                        .map_or("?", |o| o.name.as_str());
                    event(
                        "mover",
                        format!(
                            "mover {i} {name}: active {} held {} shown {} moving {} offset {:.2}",
                            now.active, now.held, now.shown, now.moving, m.offset
                        ),
                    );
                    *old = now;
                }
                Some(_) => {}
                None => self.movers.push(now),
            }
        }

        // Entities with hit points, and triggers.
        let mut cur = std::mem::take(&mut self.scratch);
        cur.clear();
        for e in &g.entities {
            let (hp, pressed, class, strength) = match &e.kind {
                Kind::Enemy {
                    hp,
                    class,
                    strength,
                    ..
                }
                | Kind::Generator {
                    hp,
                    class,
                    strength,
                    ..
                } => (*hp, false, *class, *strength),
                Kind::Container { hp, sub, .. } => (*hp, false, *sub as usize, 0),
                Kind::Trigger { pressed, .. } => (0.0, *pressed, 0, 0),
                _ => continue,
            };
            let hp = if e.alive { hp } else { f32::NEG_INFINITY };
            let now = EntSnap {
                hp,
                pressed,
                tag: kind_tag(&e.kind),
                class,
                strength,
                pos: e.pos,
            };
            if let Some(old) = self.ents.get(&e.id) {
                if old.hp.is_finite() && hp.is_finite() && hp < old.hp {
                    self.dealt += old.hp - hp;
                }
                if old.hp.is_finite() && !hp.is_finite() {
                    self.kills += 1;
                    event(
                        "kill",
                        format!("{} id {} at {}", entity_name(g, &e.kind), e.id, v3(e.pos)),
                    );
                }
                if pressed && !old.pressed {
                    if let Kind::Trigger {
                        id, mover, mode, ..
                    } = &e.kind
                    {
                        event(
                            "trigger",
                            format!(
                                "trigger {id} ({mode:?}) set off at {} -> mover {}",
                                v3(e.pos),
                                mover.map_or("none".into(), |m| m.to_string())
                            ),
                        );
                    }
                }
            }
            cur.insert(e.id, now);
        }
        for (id, old) in &self.ents {
            if old.hp.is_finite() && old.hp > 0.0 && !cur.contains_key(id) {
                self.kills += 1;
                let what = match old.tag {
                    "Enemy" | "Generator" => format!(
                        "{}{} (tier {})",
                        if old.tag == "Generator" {
                            "generator "
                        } else {
                            ""
                        },
                        g.classes.get(old.class).map_or("?", |c| c.name.as_str()),
                        old.strength
                    ),
                    "Container" => format!("container (sub {:#x})", old.class),
                    t => t.to_string(),
                };
                event(
                    "kill",
                    format!(
                        "{what} id {id} gone at {} (hp was {:.1})",
                        v3(old.pos),
                        old.hp
                    ),
                );
            }
        }
        self.scratch = std::mem::replace(&mut self.ents, cur);

        // Messages, progress, cameras, the level's end.
        let msg = g
            .msgs
            .shown
            .as_ref()
            .map(|s| (s.index, s.lines.join(" / ")));
        if msg != self.msg {
            if let Some((i, text)) = &msg {
                let who = g
                    .msgs
                    .shown
                    .as_ref()
                    .and_then(|s| s.player)
                    .map_or(String::new(), |p| format!(" (P{})", p + 1));
                event("msg", format!("msgPost {i:#x}{who}: {text}"));
            }
            self.msg = msg;
        }
        let prog = (g.runes, g.shards, g.boss_runes, g.crystals, g.garg_items);
        if prog != self.progress {
            event(
                "prog",
                format!(
                    "runes {:#x} shards {:#x} boss runes {:#x} crystals {:?} garg {:?}",
                    prog.0, prog.1, prog.2, prog.3, prog.4
                ),
            );
            self.progress = prog;
        }
        let end = g.level_end.map(|e| format!("{e:?}"));
        if end != self.level_end {
            if let Some(e) = &end {
                event("level", format!("level end: {e}"));
            }
            self.level_end = end;
        }
        let tc = g.trigger_camera_on();
        if tc != self.trigger_cam {
            event(
                "camera",
                if tc {
                    "trigger camera on"
                } else {
                    "trigger camera off"
                },
            );
            self.trigger_cam = tc;
        }
        let sc = g.scroll_box.is_some();
        if sc != self.scroll {
            event(
                "msg",
                match &g.scroll_box {
                    Some(b) => format!("scroll box: {}", b.text().replace('\n', " / ")),
                    None => "scroll box closed".into(),
                },
            );
            self.scroll = sc;
        }

        // The per-second summary.
        self.second_ticks += 1;
        if self.second_ticks >= 60 {
            self.second_ticks = 0;
            for (i, s) in self.inputs.iter_mut().enumerate() {
                if let Some(p) = g.players.get(i) {
                    if !s.idle() {
                        event(
                            "input",
                            format!(
                                "P{} {} | at {} hp {:.0}",
                                i + 1,
                                s.summary(),
                                v3(p.pos),
                                p.health
                            ),
                        );
                    }
                }
                *s = InputSum::default();
            }
            if self.dealt > 0.0 || self.kills > 0 {
                event(
                    "damage",
                    format!("dealt {:.1} this second, {} killed", self.dealt, self.kills),
                );
            }
            self.dealt = 0.0;
            self.kills = 0;
        }
    }

    /// Take the level's state as it is, without events.
    fn seed(&mut self, g: &Game) {
        self.players = (0..g.players.len()).map(|i| player_snap(g, i)).collect();
        self.movers = g
            .movers
            .iter()
            .map(|m| MoverSnap {
                active: m.active,
                held: m.held,
                shown: m.shown,
                moving: m.moving,
            })
            .collect();
        self.progress = (g.runes, g.shards, g.boss_runes, g.crystals, g.garg_items);
    }
}

fn player_snap(g: &Game, i: usize) -> PlayerSnap {
    let p = &g.players[i];
    PlayerSnap {
        health: p.health,
        gold: p.gold,
        keys: p.keys,
        potions: p.potions,
        level: p.level,
        state: state_name(&p.state),
        powerups: p
            .powerups
            .slots
            .iter()
            .filter(|s| s.time != 0.0)
            .map(|s| (s.kind, s.flags, s.state))
            .collect(),
        exit_waiting: p.exit.waiting,
        transporting: g.transport.heroes.get(i).is_some_and(|h| h.timer > 0.0),
    }
}

fn powerups_text(p: &crate::game::Player) -> String {
    let v: Vec<String> = p
        .powerups
        .slots
        .iter()
        .filter(|s| s.time != 0.0)
        .map(|s| {
            let state = match s.state {
                crate::powerups::FRESH => "fresh",
                crate::powerups::ON => "on",
                crate::powerups::OFF => "off",
                _ => "?",
            };
            let left = if s.time < 0.0 {
                format!("{} uses", s.amount)
            } else {
                format!("{:.0}s", s.time)
            };
            format!("{} [{state}, {left}]", crate::powerups::label(s))
        })
        .collect();
    if v.is_empty() {
        "none".into()
    } else {
        v.join(", ")
    }
}

// ---------------------------------------------------------------------------
// Writing a report.

/// What a report is made from.
pub struct Report<'a> {
    pub game: Option<&'a Game>,
    /// The party's HUD panels (class, colour, slot per player).
    pub panels: &'a [PanelInfo],
    pub progress: Option<&'a Progress>,
    pub heroes: &'a [Option<HeroSave>],
    pub prefs: Option<&'a Prefs>,
    /// The memory card file to copy.
    pub card_file: Option<&'a Path>,
    /// The quick tags picked in the menu.
    pub tags: &'a [&'a str],
    /// The screen the game was on ("playing", "title", ...).
    pub screen: &'a str,
    /// The frames the app has drawn, or the headless simulation's frame.
    pub frames: u64,
    /// Where it was filed from (the menu, GDL_BUGREPORT).
    pub via: &'a str,
}

/// (year, month, day, hour, minute, second) of a Unix time (UTC).
pub fn civil(secs: u64) -> (i64, u32, u32, u32, u32, u32) {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + (m <= 2) as i64;
    (
        y,
        m,
        d,
        (rem / 3600) as u32,
        (rem / 60 % 60) as u32,
        (rem % 60) as u32,
    )
}

/// `YYYYMMDD-HHMMSS` (UTC).
pub fn stamp(secs: u64) -> String {
    let (y, mo, d, h, mi, s) = civil(secs);
    format!("{y:04}{mo:02}{d:02}-{h:02}{mi:02}{s:02}")
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// A folder name: `<stamp>-<level>`, the level kept to file-name characters.
pub fn folder_name(secs: u64, level: &str) -> String {
    let level: String = level
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let level = if level.is_empty() {
        "nolevel".into()
    } else {
        level
    };
    format!("{}-{level}", stamp(secs))
}

/// The headless command line that puts the party at this spot.
pub fn repro_command(
    level: &str,
    players: &[(String, String)],
    spawn: Vec3,
    eye: Vec3,
    at: Vec3,
) -> String {
    let list: Vec<String> = players
        .iter()
        .map(|(c, col)| format!("{c}/{col}"))
        .collect();
    format!(
        "GDL_MUTE=1 gauntlet --level {level} --players {} --spawn {} --eye {} --at {} --snapshot out.png",
        if list.is_empty() { "WAR/RED".to_string() } else { list.join(",") },
        v3c(spawn),
        v3c(eye),
        v3c(at)
    )
}

fn level_title(g: &Game) -> String {
    g.wave
        .and_then(|w| g.worlds.level(w))
        .map(|l| format!("{} {}", l.prep, l.title).trim().to_string())
        .unwrap_or_default()
}

/// report.md.
pub fn report_md(r: &Report, secs: u64) -> String {
    let mut s = String::new();
    let (y, mo, d, h, mi, se) = civil(secs);
    let level = r
        .game
        .map_or("(none)".to_string(), |g| g.level.name.clone());
    let _ = writeln!(s, "# Bug report: {level}\n");
    let _ = writeln!(
        s,
        "- Filed: {y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{se:02} UTC (via {})",
        r.via
    );
    let _ = writeln!(s, "- Build: gauntlet-rs {BUILD}");
    let _ = writeln!(s, "- Screen: {}", r.screen);
    let _ = writeln!(s, "- Frames drawn: {}", r.frames);
    let tags = if r.tags.is_empty() {
        "(none)".to_string()
    } else {
        r.tags.join(", ")
    };
    let _ = writeln!(s, "- Tags: **{tags}**");
    let _ = writeln!(
        s,
        "- Files: report.md, nearby.txt, events.log (last {KEEP_SECS:.0} s), screenshot.png, save.txt, memcard copy"
    );
    if let Some(g) = r.game {
        game_section(&mut s, g, r);
    } else {
        let _ = writeln!(s, "\nNo level was running.");
    }
    if let Some(p) = r.prefs {
        let _ = writeln!(s, "\n## Options\n");
        let _ = writeln!(
            s,
            "- music {} sfx {} stereo {} difficulty {} multiplayer {} compass {}",
            p.music_vol, p.sfx_vol, p.stereo, p.difficulty, p.multiplayer, p.compass
        );
        for (i, c) in p.players.iter().enumerate() {
            let style = crate::options::CONTROL_STYLES
                .get(c.control_style)
                .copied()
                .unwrap_or("?");
            let _ = writeln!(
                s,
                "- slot {}: control style {} ({style}), rumble {}, auto aim {}, auto attack {}",
                i + 1,
                c.control_style,
                c.rumble,
                c.auto_aim,
                c.auto_attack
            );
        }
    }
    if let Some(p) = r.progress {
        let _ = writeln!(s, "\n## Party progress (app)\n");
        let _ = writeln!(
            s,
            "- beaten per realm {:?}\n- runes {:#x} shards {:#x} legend {:#x}\n- crystals {:?}\n- gargoyle items {:?}\n- last realm {:?} last beaten {:?}",
            p.beaten, p.runes, p.shards, p.legend, p.crystals, p.garg_items, p.last_realm,
            p.last_beaten.map(|w| format!("{w:#x}"))
        );
    }
    s
}

fn game_section(s: &mut String, g: &Game, r: &Report) {
    let _ = writeln!(s, "\n## Level\n");
    let realm = g.wave.map_or(0, |w| w >> 8);
    let _ = writeln!(
        s,
        "- {} \"{}\", wave {}, realm {} ({})",
        g.level.name,
        level_title(g),
        g.wave.map_or("none".into(), |w| format!("{w:#x}")),
        realm,
        crate::debugmenu::realm_name(realm)
    );
    let _ = writeln!(
        s,
        "- tick {} ({:.1} s in the level), difficulty {}, friendly fire {}, god mode {}",
        g.frame, g.time, g.difficulty, g.friendly_fire, g.debug_god
    );
    let _ = writeln!(
        s,
        "- start index {}, deaths {}, level end {:?}, boss {}, secret timer {:?}",
        g.start_index,
        g.deaths,
        g.level_end,
        if g.boss.is_some() { "yes" } else { "no" },
        g.secret_timer
    );
    let alive =
        |f: &dyn Fn(&Kind) -> bool| g.entities.iter().filter(|e| e.alive && f(&e.kind)).count();
    let _ = writeln!(
        s,
        "- {} entities: {} monsters, {} generators alive",
        g.entities.len(),
        alive(&|k| matches!(k, Kind::Enemy { .. })),
        alive(&|k| matches!(k, Kind::Generator { .. }))
    );
    if let Some(m) = &g.msgs.shown {
        let _ = writeln!(
            s,
            "- message on screen: {:#x} \"{}\" ({} ticks left)",
            m.index,
            m.lines.join(" / "),
            m.ticks
        );
    }
    if let Some(b) = &g.scroll_box {
        let _ = writeln!(
            s,
            "- scroll box open: \"{}\"",
            b.text().replace('\n', " / ")
        );
    }

    let (eye, at, near) = g.camera_view();
    let _ = writeln!(s, "\n## Camera\n");
    let _ = writeln!(
        s,
        "- eye {} at {} near {near:.2}; yaw {:.3} pitch {:.3}; focus {}",
        v3(eye),
        v3(at),
        g.cam_yaw,
        g.cam_pitch,
        v3(g.cam_focus)
    );
    let _ = writeln!(
        s,
        "- trigger camera {}, fixed camera {:?}",
        if g.trigger_camera_on() { "on" } else { "off" },
        g.fixed_camera
    );

    let _ = writeln!(s, "\n## Players\n");
    for (i, p) in g.players.iter().enumerate() {
        let panel = r.panels.get(i);
        let _ = writeln!(
            s,
            "### P{} {} {} ({}){}\n",
            i + 1,
            p.color,
            p.name,
            p.class,
            panel.map_or(String::new(), |pn| format!(
                ", slot {}{}{}",
                pn.slot + 1,
                if pn.name.is_empty() {
                    String::new()
                } else {
                    format!(", name \"{}\"", pn.name)
                },
                pn.hidden
                    .map_or(String::new(), |h| format!(", hidden skin {h}"))
            ))
        );
        let _ = writeln!(
            s,
            "- state {:?}; level {} exp {}; health {:.1}/{:.0}; turbo {:.0}",
            p.state,
            p.level,
            p.exp,
            p.health,
            crate::game::max_health(p.level),
            p.power
        );
        let _ = writeln!(
            s,
            "- gold {} keys {} potions {} (colours {:?})",
            p.gold, p.keys, p.potions, p.potion_colours
        );
        let _ = writeln!(s, "- power-ups: {}", powerups_text(p));
        let _ = writeln!(
            s,
            "- pos {} yaw {:.3} vy {:.2} on ground {}",
            v3(p.pos),
            p.yaw,
            p.vy,
            p.on_ground
        );
        let _ = writeln!(s, "- ground: {}", ground_text(g, i));
        let seq = g
            .models
            .get(p.model)
            .and_then(|m| m.skeleton.sequences.get(p.animator.seq))
            .map_or("?", |q| q.name.as_str());
        let _ = writeln!(
            s,
            "- animation {} frame {:.1}; action cur {} next {}",
            seq, p.animator.frame, p.act.cur, p.act.next
        );
        let _ = writeln!(
            s,
            "- exit: waiting {} idle {} dest {:?}; transport {:?}",
            p.exit.waiting,
            p.exit.idle,
            p.exit_dest,
            g.transport.heroes.get(i)
        );
        let c = &g.controls[i.min(3)];
        let _ = writeln!(
            s,
            "- controls: style {} auto aim {} auto attack {}\n",
            c.control_style, c.auto_aim, c.auto_attack
        );
    }

    let _ = writeln!(s, "## Progress (level)\n");
    let _ = writeln!(
        s,
        "- crystals {:?}\n- shards {:#06x} ({}/13), runes {:#06x}, boss runes won {:#x}, legend held {:#x}\n- gargoyle items {:?}",
        g.crystals,
        g.shards,
        g.shards.count_ones(),
        g.runes,
        g.boss_runes,
        g.legend_held,
        g.garg_items
    );
    let portals: Vec<String> = g
        .entities
        .iter()
        .filter_map(|e| match &e.kind {
            Kind::Exit { dest, open, .. } => Some(format!(
                "{}{} at {}",
                dest.map_or("tower".into(), |w| format!("{w:#x}")),
                if *open { "" } else { " (closed)" },
                v3(e.pos)
            )),
            _ => None,
        })
        .collect();
    let _ = writeln!(s, "- exits / portals: {}", portals.join("; "));

    let _ = writeln!(s, "\n## Reproduce\n");
    let players: Vec<(String, String)> = g
        .players
        .iter()
        .map(|p| (p.class.clone(), p.color.clone()))
        .collect();
    let p0 = g.players.first().map_or(Vec3::ZERO, |p| p.pos);
    let _ = writeln!(
        s,
        "```sh\n{}\n```",
        repro_command(&g.level.name, &players, p0, eye, at)
    );
    let _ = writeln!(
        s,
        "`--spawn` puts P1 there; the others start at the level's start. Add `--simulate 1` to let the floor settle."
    );
}

fn ground_text(g: &Game, pi: usize) -> String {
    let p = &g.players[pi];
    let mut s = match g.collision.ground_tri(p.pos, p.pos.y + 1.0) {
        Some((y, t)) => {
            let tri = &g.collision.tris[t];
            let owner = g
                .level
                .worlds
                .world_objects
                .get(tri.owner.max(0) as usize)
                .filter(|_| tri.owner >= 0)
                .map_or("?", |o| o.name.as_str());
            format!(
                "floor y {y:.2} ({:+.2} below feet), tri {t} owner {} {owner}, normal {}, enabled {}",
                p.pos.y - y,
                tri.owner,
                v3(tri.n),
                g.collision.enabled.get(t).copied().unwrap_or(true)
            )
        }
        None => "no floor under the hero".into(),
    };
    if let Some(Some(f)) = g.world_anim.floors.get(pi) {
        let name = g
            .level
            .worlds
            .world_objects
            .get(*f as usize)
            .map_or("?", |o| o.name.as_str());
        let _ = write!(s, "; riding object {f} {name}");
    }
    s
}

/// nearby.txt.
pub fn nearby_txt(g: &Game) -> String {
    let mut s = String::new();
    let heroes: Vec<Vec3> = g.players.iter().map(|p| p.pos).collect();
    let near = |at: Vec3, extra: f32| -> Option<(usize, f32)> {
        heroes
            .iter()
            .enumerate()
            .map(|(i, h)| (i, h.distance(at)))
            .filter(|&(_, d)| d <= NEAR + extra)
            .min_by(|a, b| a.1.total_cmp(&b.1))
    };
    let w = &g.level.worlds;
    let _ = writeln!(
        s,
        "Everything within {NEAR} units of a hero in {} (frame {}). Distances are to the nearest hero.",
        g.level.name, g.frame
    );
    for (i, h) in heroes.iter().enumerate() {
        let _ = writeln!(s, "P{} at {}", i + 1, v3(*h));
    }

    // Entities.
    let mut ents: Vec<(f32, usize, &crate::game::Entity)> = g
        .entities
        .iter()
        .filter_map(|e| near(e.pos, 0.0).map(|(pi, d)| (d, pi, e)))
        .collect();
    ents.sort_by(|a, b| a.0.total_cmp(&b.0));
    let _ = writeln!(s, "\n== Entities ({}) ==", ents.len());
    for (d, pi, e) in ents {
        let model = e
            .model
            .and_then(|m| g.models.get(m))
            .map(|m| m.skeleton.name.clone());
        let seq = e
            .model
            .and_then(|m| g.models.get(m))
            .and_then(|m| m.skeleton.sequences.get(e.animator.seq))
            .map_or(e.animator.seq.to_string(), |q| q.name.clone());
        let placed = placed_as(g, e.pos);
        let _ = writeln!(
            s,
            "- id {} {} d {d:.1} (P{}) pos {} yaw {:.2} alive {} fade {:.2} scale {:.2}\n    model {} anim {} frame {:.1}{} solid {:?}{}\n    {}",
            e.id,
            kind_tag(&e.kind),
            pi + 1,
            v3(e.pos),
            e.yaw,
            e.alive,
            e.fade,
            e.scale,
            model.as_deref().unwrap_or("none"),
            seq,
            e.animator.frame,
            if e.animator.finished { " (finished)" } else { "" },
            e.solid,
            placed.map_or(String::new(), |p| format!("\n    placed as {p}")),
            kind_detail(g, &e.kind)
        );
    }

    // Placed items.
    let _ = writeln!(s, "\n== Placed item instances ==");
    for (i, inst) in w.item_instances.iter().enumerate() {
        let pos = Vec3::from(inst.pos);
        let Some((_, d)) = near(pos, 0.0) else {
            continue;
        };
        let info = w.item_infos.get(inst.item_index as usize);
        let _ = writeln!(
            s,
            "- inst {i} \"{}\" info {} {} type {:?} sub {:#x} pos {} d {d:.1} min players {} flags {:#x} params {:02x?}",
            inst.name,
            inst.item_index,
            info.map_or("?", |f| f.name.as_str()),
            info.map(|f| f.item_type),
            info.map_or(0, |f| f.subtype),
            v3(pos),
            inst.min_players,
            inst.flags,
            inst.params
        );
    }

    // Movers (with the triggers that drive them).
    let _ = writeln!(
        s,
        "\n== Movers near (trigger targets: elevators, bridges, barricades; {} in the level) ==",
        g.movers.len()
    );
    for (i, m) in g.movers.iter().enumerate() {
        let drivers: Vec<String> = g
            .entities
            .iter()
            .filter_map(|e| match &e.kind {
                Kind::Trigger {
                    mover: Some(mi),
                    id,
                    ..
                } if *mi == i => Some(format!("trigger {id} (entity {}) at {}", e.id, v3(e.pos))),
                _ => None,
            })
            .collect();
        let driver_near = g.entities.iter().any(|e| {
            matches!(&e.kind, Kind::Trigger { mover: Some(mi), .. } if *mi == i)
                && near(e.pos, 0.0).is_some()
        });
        let Some((_, d)) = near(m.pos, 20.0).or(driver_near.then_some((0, f32::NAN))) else {
            continue;
        };
        let root = w.world_objects.get(m.root).map_or("?", |o| o.name.as_str());
        let _ = writeln!(
            s,
            "- mover {i} root {} {root} pos {} d {d:.1}: active {} held {} shown {} moving {} animated {} offset {:.2}, {} objects\n    driven by: {}",
            m.root,
            v3(m.pos),
            m.active,
            m.held,
            m.shown,
            m.moving,
            m.animated,
            m.offset,
            m.objects.len(),
            if drivers.is_empty() { "nothing".into() } else { drivers.join("; ") }
        );
    }
    if let Some(c) = &g.trigger_cam {
        let _ = writeln!(s, "trigger camera running: {c:?}");
    }

    // Transporters.
    let _ = writeln!(s, "\n== Transporter pads ==");
    for (i, p) in g.transport.pads.iter().enumerate() {
        let Some((_, d)) = near(p.pos, 0.0) else {
            continue;
        };
        let dest = p
            .dest
            .and_then(|k| g.transport.pads.get(k))
            .map_or("unmatched".into(), |q| format!("pad at {}", v3(q.pos)));
        let _ = writeln!(
            s,
            "- pad {i} id {} -> dest id {} ({dest}) pos {} radius {:.2} d {d:.1}",
            p.id,
            p.dest_id,
            v3(p.pos),
            p.radius
        );
    }
    for (i, h) in g.transport.heroes.iter().enumerate() {
        let _ = writeln!(s, "P{} transport {h:?}", i + 1);
    }

    // World objects.
    let mut tris = vec![(0u32, 0u32); w.world_objects.len()];
    for (t, tri) in g.collision.tris.iter().enumerate() {
        if let Some(c) = usize::try_from(tri.owner)
            .ok()
            .and_then(|o| tris.get_mut(o))
        {
            if g.collision.enabled.get(t).copied().unwrap_or(true) {
                c.0 += 1;
            } else {
                c.1 += 1;
            }
        }
    }
    let _ = writeln!(
        s,
        "\n== World objects with collision, motion or changed flags (flags: file / live; tris enabled / disabled) =="
    );
    let mut skipped = 0;
    for (i, o) in w.world_objects.iter().enumerate() {
        let pos = g
            .world_anim
            .world
            .get(i)
            .map(|m| m.w_axis.truncate())
            .filter(|p| *p != Vec3::ZERO)
            .unwrap_or_else(|| Vec3::from(g.level.world_pos.get(i).copied().unwrap_or(o.pos)));
        let Some((_, d)) = near(pos, o.radius) else {
            continue;
        };
        let live = g.world_anim.flags.get(i).copied().unwrap_or(o.flags);
        let hidden = g.hidden.contains(&i) || g.world_anim.hidden.get(i).copied().unwrap_or(false);
        let (on, off) = tris[i];
        // Only what has collision, moves, hides or is driven: the rest is scenery.
        let moved = g.movers.iter().any(|m| m.objects.contains(&i));
        if on + off == 0
            && !hidden
            && !moved
            && !g.world_anim.dynamic.contains(&i)
            && live == o.flags
        {
            skipped += 1;
            continue;
        }
        let _ = writeln!(
            s,
            "- obj {i} {} pos {} r {:.1} d {d:.1} flags {:#x} / {live:#x} mb {:#x} nocol {} tris {on}/{off}{}{}{}",
            o.name,
            v3(pos),
            o.radius,
            o.flags,
            o.mb_flags,
            o.no_collision,
            if hidden { " HIDDEN" } else { "" },
            if g.world_anim.animating(i) { " animating" } else { "" },
            if g.world_anim.dynamic.contains(&i) { " dynamic" } else { "" }
        );
    }

    let _ = writeln!(
        s,
        "({skipped} more nearby objects: scenery without collision)"
    );

    // Locators.
    let p0 = heroes.first().copied().unwrap_or(Vec3::ZERO);
    let mut locs: Vec<(f32, usize)> = w
        .locators
        .iter()
        .enumerate()
        .map(|(i, l)| (Vec3::from(l.pos).distance(p0), i))
        .collect();
    locs.sort_by(|a, b| a.0.total_cmp(&b.0));
    use gdl_formats::worlds::LocatorType as L;
    let is_cam = |k: &L| {
        matches!(
            k,
            L::CameraStart
                | L::CameraGame
                | L::TriggerCamera
                | L::CameraAttract
                | L::CameraAttractStart
        )
    };
    let loc_line = |s: &mut String, d: f32, i: usize| {
        let l = &w.locators[i];
        let _ = writeln!(
            s,
            "- loc {i} {:?} pos {} rot ({:.2}, {:.2}, {:.2}) delay {} next {} d {d:.1}",
            l.kind,
            v3(Vec3::from(l.pos)),
            l.rot[0],
            l.rot[1],
            l.rot[2],
            l.delay,
            l.next
        );
    };
    let _ = writeln!(s, "\n== Nearest locators to P1 ==");
    for &(d, i) in locs
        .iter()
        .filter(|(_, i)| !is_cam(&w.locators[*i].kind))
        .take(10)
    {
        loc_line(&mut s, d, i);
    }
    let _ = writeln!(s, "\n== Nearest camera locators to P1 ==");
    for &(d, i) in locs
        .iter()
        .filter(|(_, i)| is_cam(&w.locators[*i].kind))
        .take(6)
    {
        loc_line(&mut s, d, i);
    }

    // Landmarks anywhere in the level.
    let _ = writeln!(s, "\n== Exits and generators in the whole level ==");
    for e in &g.entities {
        if matches!(e.kind, Kind::Exit { .. } | Kind::Generator { .. }) {
            let d = p0.distance(e.pos);
            let _ = writeln!(
                s,
                "- id {} {} at {} d {d:.1} alive {}: {}",
                e.id,
                kind_tag(&e.kind),
                v3(e.pos),
                e.alive,
                kind_detail(g, &e.kind)
            );
        }
    }
    s
}

/// The placed item an entity stands on the spot of (it was made from it).
fn placed_as(g: &Game, pos: Vec3) -> Option<String> {
    let w = &g.level.worlds;
    w.item_instances
        .iter()
        .enumerate()
        .find(|(_, i)| Vec3::from(i.pos).distance_squared(pos) < 0.01)
        .map(|(k, i)| {
            let info = w.item_infos.get(i.item_index as usize);
            format!(
                "inst {k} \"{}\" info {} type {:?} sub {:#x}",
                info.map_or("?", |f| f.name.as_str()),
                i.item_index,
                info.map(|f| f.item_type),
                info.map_or(0, |f| f.subtype)
            )
        })
}

/// The interesting fields of an entity's kind.
pub fn kind_detail(g: &Game, k: &Kind) -> String {
    match k {
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
            ai,
            dest,
            ..
        } => format!(
            "{} hp {hp:.1} state {state:?} recognized {recognized} target {target:?} attacks {attacks} cooldown {cooldown:.2} speed {speed:.1} etype {etype} ai {ai} dest {dest:?} strength {strength}",
            entity_name(g, k)
        ),
        Kind::Generator {
            hp,
            timer,
            interval,
            max,
            spawned,
            always,
            ai,
            ..
        } => format!(
            "{} hp {hp:.1} timer {timer:.1} interval {interval:.1} spawned {}/{max} always {always} ai {ai}",
            entity_name(g, k),
            spawned.len()
        ),
        Kind::Critter(c) => format!(
            "critter {} etype {} alpha {:.2} deleted {} statue {} garg {:?} parts {}",
            c.prefix,
            c.etype,
            c.alpha,
            c.deleted,
            c.statue.is_some(),
            c.garg_kind,
            c.parts.len()
        ),
        Kind::Exit {
            radius,
            height,
            dest,
            open,
            ..
        } => format!(
            "exit to {} open {open} radius {radius:.2} height {height:.2}",
            dest.map_or("tower".into(), |w| format!("{w:#x}"))
        ),
        Kind::Hazard {
            radius,
            damage,
            flags,
            cycle,
        } => format!(
            "hazard radius {radius:.2} damage {damage:.1} flags {flags:#x} cycle {}",
            truncate(&format!("{cycle:?}"), 200)
        ),
        other => truncate(&format!("{other:?}"), 400),
    }
}

fn truncate(s: &str, n: usize) -> String {
    match s.char_indices().nth(n) {
        Some((i, _)) => format!("{}...", &s[..i]),
        None => s.to_string(),
    }
}

/// save.txt: the heroes' save images and the party's progress, as text.
pub fn save_txt(r: &Report) -> String {
    let mut s = String::new();
    let _ = writeln!(
        s,
        "Memory card file: {}",
        r.card_file
            .map_or("(none)".into(), |p| p.display().to_string())
    );
    for (col, h) in r.heroes.iter().enumerate() {
        let Some(h) = h else {
            let _ = writeln!(s, "\n== Column {}: no hero ==", col + 1);
            continue;
        };
        let _ = writeln!(
            s,
            "\n== Column {}: \"{}\" character {} colour {} class unlock {:#x} ==",
            col + 1,
            h.name,
            h.character,
            h.colour,
            h.class_unlock
        );
        let _ = writeln!(s, "controls {:?}", h.controls);
        let seen = h.help_disp.iter().filter(|&&b| b != 0).count();
        let _ = writeln!(s, "help messages seen: {seen}");
        for (c, cs) in h.classes.iter().enumerate() {
            if let Some(car) = &cs.carried {
                let _ = writeln!(s, "class {c}: {car:?}\n  progress {:?}", cs.progress);
            }
        }
    }
    if let Some(p) = r.progress {
        let _ = writeln!(s, "\n== Party progress ==\n{p:#?}");
    }
    if let Some(g) = r.game {
        let _ = writeln!(s, "\n== Heroes in the level ==");
        for (i, p) in g.players.iter().enumerate() {
            let c = p.carried();
            let _ = writeln!(
                s,
                "P{} health {:.1} gold {} keys {} potions {:?} level {} exp {} bonus {:?} stats {:?}\n   power-ups: {}",
                i + 1,
                c.health,
                c.gold,
                c.keys,
                c.potions,
                c.level,
                c.exp,
                c.bonus,
                c.stats,
                powerups_text(p)
            );
        }
    }
    s
}

/// Write a report folder under `root`; returns its path.
pub fn file(root: &Path, r: &Report, shot: Option<&gdl_formats::Image>) -> Result<PathBuf> {
    let secs = unix_now();
    let level = r.game.map_or(r.screen, |g| g.level.name.as_str());
    let base = folder_name(secs, level);
    let mut dir = root.join(&base);
    let mut n = 2;
    while dir.exists() {
        dir = root.join(format!("{base}-{n}"));
        n += 1;
    }
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    event(
        "bug",
        format!(
            "bug report filed: {} tags [{}]",
            dir.display(),
            r.tags.join(", ")
        ),
    );
    std::fs::write(dir.join("report.md"), report_md(r, secs))?;
    if let Some(g) = r.game {
        std::fs::write(dir.join("nearby.txt"), nearby_txt(g))?;
    }
    std::fs::write(dir.join("events.log"), events_text())?;
    std::fs::write(dir.join("save.txt"), save_txt(r))?;
    if let Some(card) = r.card_file.filter(|p| p.exists()) {
        let name = card
            .file_name()
            .map_or("memcard".into(), |n| n.to_string_lossy().into_owned());
        if let Err(e) = std::fs::copy(card, dir.join(format!("memcard-{name}"))) {
            log::warn!("bug report: copying {}: {e}", card.display());
        }
    }
    if let Some(img) = shot {
        write_png(&dir.join("screenshot.png"), img)?;
    }
    log::info!("bug report saved to {}", dir.display());
    Ok(dir)
}

pub fn write_png(path: &Path, img: &gdl_formats::Image) -> Result<()> {
    let f = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut enc = png::Encoder::new(f, img.width, img.height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(&img.rgba)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(t: f64, text: &str) -> Event {
        Event {
            t,
            tick: (t * 60.0) as u32,
            cat: "test",
            text: text.into(),
        }
    }

    #[test]
    fn ring_drops_old_events() {
        let mut r = Ring::new(120.0, 1000);
        for i in 0..300 {
            r.push(ev(i as f64, &i.to_string()));
        }
        // 179..=299 are within 120 s of the newest.
        assert_eq!(r.len(), 121);
        assert_eq!(r.iter().next().unwrap().text, "179");
        assert_eq!(r.iter().last().unwrap().text, "299");
    }

    #[test]
    fn ring_caps_its_size() {
        let mut r = Ring::new(120.0, 50);
        for i in 0..200 {
            r.push(ev(i as f64 * 0.001, &i.to_string()));
        }
        assert_eq!(r.len(), 50);
        assert_eq!(r.iter().next().unwrap().text, "150");
        assert!(Ring::new(1.0, 0).is_empty());
    }

    #[test]
    fn ring_dump_has_a_line_an_event() {
        let mut r = Ring::new(120.0, 10);
        r.push(ev(1.5, "hello"));
        r.push(ev(2.0, "world"));
        let d = r.dump();
        assert_eq!(d.lines().count(), 2);
        assert!(d.lines().next().unwrap().contains("test   hello"), "{d}");
        assert!(d.contains("t=90"), "{d}");
    }

    #[test]
    fn global_events_and_logger() {
        event("test", "global ring works");
        assert!(events_text().contains("global ring works"));
    }

    #[test]
    fn dates() {
        assert_eq!(civil(0), (1970, 1, 1, 0, 0, 0));
        // 2026-10-23 14:05:09 UTC.
        assert_eq!(stamp(1_792_764_309), "20261023-140509");
        // A leap day.
        assert_eq!(stamp(951_782_400), "20000229-000000");
        assert_eq!(folder_name(0, "levelA1"), "19700101-000000-levelA1");
        assert_eq!(folder_name(0, "a/b c"), "19700101-000000-a_b_c");
        assert_eq!(folder_name(0, ""), "19700101-000000-nolevel");
    }

    #[test]
    fn repro_line() {
        let c = repro_command(
            "levelA1",
            &[("WAR".into(), "RED".into()), ("VAL".into(), "BLU".into())],
            Vec3::new(1.0, 2.5, -3.25),
            Vec3::new(0.0, 10.0, 0.0),
            Vec3::new(1.0, 2.0, 3.0),
        );
        assert_eq!(
            c,
            "GDL_MUTE=1 gauntlet --level levelA1 --players WAR/RED,VAL/BLU --spawn 1.00,2.50,-3.25 \
             --eye 0.00,10.00,0.00 --at 1.00,2.00,3.00 --snapshot out.png"
        );
        assert!(
            repro_command("levelB2", &[], Vec3::ZERO, Vec3::ZERO, Vec3::ZERO)
                .contains("--players WAR/RED")
        );
    }

    #[test]
    fn report_without_a_level() {
        let progress = Progress::default();
        let prefs = Prefs::default();
        let r = Report {
            game: None,
            panels: &[],
            progress: Some(&progress),
            heroes: &[None, None, None, None],
            prefs: Some(&prefs),
            card_file: None,
            tags: &["stuck", "wrong look"],
            screen: "title",
            frames: 42,
            via: "test",
        };
        let md = report_md(&r, 0);
        assert!(md.starts_with("# Bug report: (none)"), "{md}");
        assert!(md.contains("- Tags: **stuck, wrong look**"), "{md}");
        assert!(md.contains("1970-01-01 00:00:00 UTC (via test)"), "{md}");
        assert!(md.contains("No level was running."), "{md}");
        assert!(md.contains("## Options"), "{md}");
        assert!(md.contains("slot 4: control style"), "{md}");
        assert!(md.contains(BUILD), "{md}");
        let save = save_txt(&r);
        assert!(save.contains("Column 1: no hero"), "{save}");
        // Writing it.
        let root = std::env::temp_dir().join(format!("gdl-bugtest-{}", std::process::id()));
        let img = gdl_formats::Image {
            width: 2,
            height: 1,
            rgba: vec![255; 8],
        };
        let dir = file(&root, &r, Some(&img)).unwrap();
        for f in ["report.md", "events.log", "save.txt", "screenshot.png"] {
            assert!(dir.join(f).exists(), "{f}");
        }
        assert!(dir
            .file_name()
            .unwrap()
            .to_string_lossy()
            .ends_with("-title"));
        // A second report the same second gets its own folder.
        let dir2 = file(&root, &r, None).unwrap();
        assert_ne!(dir, dir2);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn input_summary() {
        let mut s = InputSum::default();
        assert!(s.idle());
        for k in 0..60 {
            s.add(&PlayerInput {
                stick: glam::Vec2::new(1.0, 0.0),
                attack_pressed: k % 20 == 0,
                turbo: k < 30,
                ..Default::default()
            });
        }
        assert!(!s.idle());
        let t = s.summary();
        assert!(
            t.contains("stick avg (+1.00,+0.00) max 1.00 moving 60/60"),
            "{t}"
        );
        assert!(t.contains("attack x3"), "{t}");
        assert!(t.contains("turbo 30/60"), "{t}");
    }
}
