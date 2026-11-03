//! A level's recording: how it started and what every player pressed. Played
//! back on the same build and files, it is the same game, down to the bit.
//!
//! The file is plain text, so a bug report's can be read and diffed:
//!
//! - the header says which game made it (see [`Compat`]); playing it on
//!   another build is allowed, since that is how a fix gets tested, but the
//!   check says so;
//! - `[start]` is the [`LevelStart`];
//! - `[commands]` are the changes from outside the game, by tick;
//! - `[checks]` holds the game's fingerprint every [`CHECK_EVERY`] ticks, so
//!   a game that drifts is caught within a second of where it happened;
//! - `[end]` holds the digest of each part of the state at the last tick, so
//!   the check can name what differs;
//! - `[inputs]` holds runs of ticks: how many, then each player's input.
//!
//! Ticks count `Game::update_packed` calls. A menu that stops the world makes
//! no ticks and needs no recording.
//!
//! Only the debug menu can still change a game behind the recording's back.
//! When it does, the recording stops there and says so (a "taint").

use crate::compat::{Compat, Mismatch};
use crate::fs::GameFs;
use crate::game::savestate::{esc, unesc, SaveState};
use crate::game::setup::{Command, LevelStart, PartyBack};
use crate::game::{Carried, Game, PanelInfo, PlayStats};
use crate::input::{PackedInput, PlayerPrefs};
use crate::powerups::Slot;
use crate::rules::Rules;
use crate::stats::Atts;
use anyhow::{bail, Context, Result};
use std::fmt::{self, Write as _};
use std::path::Path;
use std::str::FromStr;

pub const VERSION: u32 = 1;
/// Ticks between fingerprints.
pub const CHECK_EVERY: u32 = 60;

/// Ticks in a row with the same inputs.
#[derive(Clone, Debug, PartialEq)]
struct Run {
    ticks: u32,
    inputs: Vec<PackedInput>,
}

#[derive(Clone, Debug)]
pub struct Replay {
    pub start: LevelStart,
    /// Who made it; absent until the recording is written.
    pub compat: Option<Compat>,
    pub ticks: u32,
    runs: Vec<Run>,
    /// By tick: applied before that tick runs.
    commands: Vec<(u32, Command)>,
    checks: Vec<(u32, u64)>,
    end: Vec<(String, u64)>,
    /// Where the game stopped being reproducible, and why.
    pub taint: Option<(u32, String)>,
}

impl Replay {
    pub fn new(start: LevelStart) -> Replay {
        Replay {
            start,
            compat: None,
            ticks: 0,
            runs: Vec::new(),
            commands: Vec::new(),
            checks: Vec::new(),
            end: Vec::new(),
            taint: None,
        }
    }

    /// A command about to be applied to the game.
    pub fn command(&mut self, c: &Command) {
        if self.taint.is_none() {
            self.commands.push((self.ticks, c.clone()));
        }
    }

    /// A tick the game just ran on these inputs.
    pub fn tick(&mut self, inputs: &[PackedInput], g: &Game) {
        if self.taint.is_some() {
            return;
        }
        match self.runs.last_mut() {
            Some(r) if r.inputs == inputs => r.ticks += 1,
            _ => self.runs.push(Run {
                ticks: 1,
                inputs: inputs.to_vec(),
            }),
        }
        self.ticks += 1;
        if self.ticks % CHECK_EVERY == 0 {
            self.checks.push((self.ticks, g.fingerprint()));
        }
    }

    /// The game was changed some other way: the recording ends here.
    pub fn taint(&mut self, why: &str) {
        if self.taint.is_none() {
            self.taint = Some((self.ticks, why.to_string()));
        }
    }

    /// The recording as a file, with `g` as the state it ended in.
    pub fn to_text(&self, g: &Game, compat: &Compat) -> String {
        let mut done = self.clone();
        done.compat = Some(compat.clone());
        if self.taint.is_none() {
            done.end = g
                .state_sections()
                .into_iter()
                .map(|(name, h)| (name.to_string(), h))
                .collect();
        }
        done.text()
    }

    fn text(&self) -> String {
        let mut o = String::new();
        let _ = writeln!(o, "gauntlet-replay {VERSION}");
        if let Some(c) = &self.compat {
            let _ = writeln!(o, "build {}", c.build);
            let _ = writeln!(o, "content {:016x}", c.content);
            let _ = writeln!(o, "rules {:016x}", c.rules);
            let _ = writeln!(o, "env {:016x}", c.env);
        }
        let _ = writeln!(o, "ticks {}", self.ticks);
        if let Some((tick, why)) = &self.taint {
            let _ = writeln!(o, "taint {tick} {why}");
        }
        write_start(&mut o, &self.start);
        let _ = writeln!(o, "[commands]");
        for (tick, c) in &self.commands {
            write_command(&mut o, *tick, c);
        }
        let _ = writeln!(o, "[checks]");
        for (tick, h) in &self.checks {
            let _ = writeln!(o, "{tick} {h:016x}");
        }
        let _ = writeln!(o, "[end]");
        for (name, h) in &self.end {
            let _ = writeln!(o, "{name} {h:016x}");
        }
        let _ = writeln!(o, "[inputs]");
        for r in &self.runs {
            let _ = write!(o, "{}", r.ticks);
            for i in &r.inputs {
                let _ = write!(o, " {}", input_text(i));
            }
            let _ = writeln!(o);
        }
        o
    }

    pub fn write(&self, path: &Path, g: &Game, compat: &Compat) -> Result<()> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, self.to_text(g, compat))
            .with_context(|| format!("writing {}", path.display()))
    }

    pub fn read(path: &Path) -> Result<Replay> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        Replay::from_text(&text).with_context(|| format!("reading {}", path.display()))
    }

    pub fn from_text(text: &str) -> Result<Replay> {
        let mut lines = text.lines().enumerate().peekable();
        let first = lines.next().map_or("", |(_, l)| l);
        let Some(version) = first.strip_prefix("gauntlet-replay ") else {
            bail!("not a replay file");
        };
        if version.trim().parse::<u32>().ok() != Some(VERSION) {
            bail!("replay version {version}; this game reads {VERSION}");
        }
        let mut r = Replay::new(LevelStart::default());
        let mut compat = Compat {
            build: String::new(),
            content: 0,
            rules: 0,
            env: 0,
        };
        let mut has_compat = false;
        let mut section = "";
        while let Some((n, line)) = lines.next() {
            let at = || format!("line {}: {line}", n + 1);
            if line.is_empty() {
                continue;
            }
            match line {
                "[start]" | "[commands]" | "[checks]" | "[end]" | "[inputs]" => {
                    section = line;
                    continue;
                }
                _ => {}
            }
            let (key, rest) = line.split_once(' ').unwrap_or((line, ""));
            match section {
                "" => match key {
                    "build" => {
                        compat.build = rest.to_string();
                        has_compat = true;
                    }
                    "content" => compat.content = hex64(rest).with_context(at)?,
                    "rules" => compat.rules = hex64(rest).with_context(at)?,
                    "env" => compat.env = hex64(rest).with_context(at)?,
                    "ticks" => r.ticks = rest.parse().with_context(at)?,
                    "taint" => {
                        let (t, why) = rest.split_once(' ').unwrap_or((rest, ""));
                        r.taint = Some((t.parse().with_context(at)?, why.to_string()));
                    }
                    _ => {}
                },
                "[start]" => {
                    if key == "state_lines" {
                        let count: usize = rest.parse().with_context(at)?;
                        let mut state = String::new();
                        for _ in 0..count {
                            let (_, l) = lines.next().context("the state is cut short")?;
                            state.push_str(l);
                            state.push('\n');
                        }
                        r.start.state = Some(SaveState::from_text(&state)?);
                    } else {
                        read_start_line(&mut r.start, key, rest).with_context(at)?;
                    }
                }
                "[commands]" => read_command_line(&mut r.commands, key, rest).with_context(at)?,
                "[checks]" => r.checks.push((key.parse().with_context(at)?, hex64(rest)?)),
                "[end]" => r.end.push((key.to_string(), hex64(rest).with_context(at)?)),
                "[inputs]" => {
                    let inputs = rest
                        .split_whitespace()
                        .map(parse_input)
                        .collect::<Result<Vec<_>>>()
                        .with_context(at)?;
                    r.runs.push(Run {
                        ticks: key.parse().with_context(at)?,
                        inputs,
                    });
                }
                _ => {}
            }
        }
        if has_compat {
            r.compat = Some(compat);
        }
        let recorded: u64 = r.runs.iter().map(|run| run.ticks as u64).sum();
        if recorded != r.ticks as u64 {
            bail!("{} ticks promised, {recorded} recorded", r.ticks);
        }
        Ok(r)
    }

    /// Start the recorded level. See [`LevelStart::build`].
    pub fn build(&self, fs: &GameFs) -> Result<(Game, bool)> {
        self.start.build(fs)
    }
}

/// Where a replayed game left the recorded one: the fingerprints agreed at
/// `good` and did not at `bad`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Diverged {
    pub good: u32,
    pub bad: u32,
}

/// Plays a recording on a game started from its [`LevelStart`].
#[derive(Clone, Debug, Default)]
pub struct Playback {
    tick: u32,
    run: usize,
    used: u32,
    command: usize,
    check: usize,
    last_good: u32,
    pub diverged: Option<Diverged>,
}

impl Playback {
    pub fn new() -> Playback {
        Playback::default()
    }

    pub fn tick(&self) -> u32 {
        self.tick
    }

    /// One recorded tick, and the commands before it. False when there are
    /// no more: the game is then in the state the recording ended in.
    pub fn step(&mut self, r: &Replay, g: &mut Game) -> bool {
        while let Some((_, c)) = r
            .commands
            .get(self.command)
            .filter(|(t, _)| *t <= self.tick)
        {
            g.command(c.clone());
            self.command += 1;
        }
        let Some(run) = r.runs.get(self.run) else {
            return false;
        };
        g.update_packed(&run.inputs);
        // The live game turns this into a menu; the menu's end is a command.
        if let Some(s) = &mut g.sumner {
            s.hint_request = None;
        }
        self.tick += 1;
        self.used += 1;
        if self.used >= run.ticks {
            self.run += 1;
            self.used = 0;
        }
        if let Some((t, h)) = r.checks.get(self.check).filter(|(t, _)| *t == self.tick) {
            self.check += 1;
            if g.fingerprint() == *h {
                self.last_good = *t;
            } else if self.diverged.is_none() {
                self.diverged = Some(Diverged {
                    good: self.last_good,
                    bad: *t,
                });
            }
        }
        if self.tick >= r.ticks {
            // The commands after the last tick (a level left from a menu).
            while let Some((_, c)) = r.commands.get(self.command) {
                g.command(c.clone());
                self.command += 1;
            }
            return false;
        }
        true
    }
}

/// What playing a recording showed.
#[derive(Clone, Debug)]
pub struct Verdict {
    pub level: String,
    pub ticks: u32,
    /// Ways this copy of the game differs from the one that recorded.
    pub differences: Vec<Mismatch>,
    pub taint: Option<(u32, String)>,
    pub diverged: Option<Diverged>,
    /// The parts of the final state that differ.
    pub end_differs: Vec<String>,
}

impl Verdict {
    pub fn matches(&self) -> bool {
        self.diverged.is_none() && self.end_differs.is_empty()
    }
}

impl fmt::Display for Verdict {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(f, "{}: {} ticks", self.level, self.ticks)?;
        for d in &self.differences {
            writeln!(f, "recorded on another copy: {d}")?;
        }
        if let Some((tick, why)) = &self.taint {
            writeln!(f, "the recording stops at tick {tick}: {why}")?;
        }
        if let Some(d) = &self.diverged {
            writeln!(
                f,
                "DIVERGED between ticks {} and {} ({:.1} s in)",
                d.good,
                d.bad,
                d.good as f32 / 60.0
            )?;
        }
        if !self.end_differs.is_empty() {
            writeln!(
                f,
                "the end state differs in: {}",
                self.end_differs.join(", ")
            )?;
        }
        if self.matches() {
            write!(f, "the replay matches the recording")?;
        } else {
            write!(f, "the replay does NOT match the recording")?;
        }
        Ok(())
    }
}

impl Replay {
    /// Compare the end of a played recording with what was recorded.
    pub fn verdict(&self, g: &Game, playback: &Playback, ours: Option<&Compat>) -> Verdict {
        let sections = g.state_sections();
        let end_differs = self
            .end
            .iter()
            .filter(|(name, h)| {
                sections
                    .iter()
                    .find(|(n, _)| n == name)
                    .is_none_or(|(_, ours)| ours != h)
            })
            .map(|(name, _)| name.clone())
            .collect();
        let differences = match (ours, &self.compat) {
            (Some(a), Some(b)) => a.differences(b),
            _ => Vec::new(),
        };
        Verdict {
            level: self.start.level.clone(),
            ticks: playback.tick(),
            differences,
            taint: self.taint.clone(),
            diverged: playback.diverged,
            end_differs,
        }
    }

    /// Play the whole recording headless.
    pub fn check(&self, fs: &GameFs, ours: Option<&Compat>) -> Result<Verdict> {
        let (mut g, _) = self.build(fs)?;
        let mut playback = Playback::new();
        while playback.step(self, &mut g) {}
        Ok(self.verdict(&g, &playback, ours))
    }
}

// ---- The text of the parts ------------------------------------------------

fn hex64(s: &str) -> Result<u64> {
    u64::from_str_radix(s.trim(), 16).with_context(|| format!("bad hex {s:?}"))
}

fn input_text(i: &PackedInput) -> String {
    if *i == PackedInput::default() {
        return "-".into();
    }
    let mut s = String::with_capacity(20);
    for v in i.stick.iter().chain(&i.fire).chain(&i.dpad) {
        let _ = write!(s, "{:02x}", *v as u8);
    }
    let _ = write!(s, "{:x}", i.buttons);
    s
}

fn parse_input(s: &str) -> Result<PackedInput> {
    if s == "-" {
        return Ok(PackedInput::default());
    }
    if s.len() < 13 || !s.is_ascii() {
        bail!("bad input {s:?}");
    }
    let axis = |i: usize| -> Result<i8> { Ok(u8::from_str_radix(&s[i * 2..i * 2 + 2], 16)? as i8) };
    Ok(PackedInput {
        stick: [axis(0)?, axis(1)?],
        fire: [axis(2)?, axis(3)?],
        dpad: [axis(4)?, axis(5)?],
        buttons: u32::from_str_radix(&s[12..], 16)?,
    })
}

fn csv<T: fmt::Display>(v: &[T]) -> String {
    let mut s = String::new();
    for (i, x) in v.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        let _ = write!(s, "{x}");
    }
    if s.is_empty() {
        s.push('-');
    }
    s
}

fn parse_csv<T: FromStr>(s: &str) -> Result<Vec<T>> {
    if s == "-" {
        return Ok(Vec::new());
    }
    s.split(',')
        .map(|v| v.parse().map_err(|_| anyhow::anyhow!("bad value {v:?}")))
        .collect()
}

fn parse_array<T: FromStr + Copy, const N: usize>(s: &str) -> Result<[T; N]> {
    let v: Vec<T> = parse_csv(s)?;
    <[T; N]>::try_from(v).map_err(|v| anyhow::anyhow!("{} values, wanted {N}", v.len()))
}

/// `key=value` fields.
struct Fields<'a>(Vec<(&'a str, &'a str)>);

impl<'a> Fields<'a> {
    fn new(s: &'a str) -> Fields<'a> {
        Fields(
            s.split_whitespace()
                .filter_map(|t| t.split_once('='))
                .collect(),
        )
    }

    fn str(&self, key: &str) -> Result<&'a str> {
        self.0
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| *v)
            .with_context(|| format!("no {key}"))
    }

    fn get<T: FromStr>(&self, key: &str) -> Result<T> {
        let v = self.str(key)?;
        v.parse().map_err(|_| anyhow::anyhow!("bad {key}: {v:?}"))
    }
}

fn carried_text(c: &Carried) -> String {
    let b = &c.bonus;
    let st = &c.stats;
    let mut slots = Vec::new();
    for (i, s) in c.powerups.slots.iter().enumerate() {
        if *s != Slot::default() {
            slots.push(format!(
                "{i}:{}:{}:{}:{}:{}",
                s.time, s.kind, s.amount, s.flags, s.state
            ));
        }
    }
    format!(
        "health={} gold={} keys={} potions={} level={} exp={} bonus={},{},{},{} powerups={} \
         stats={},{},{},{} sumner_exp={}",
        c.health,
        c.gold,
        c.keys,
        csv(&c.potions),
        c.level,
        c.exp,
        b.fight,
        b.speed,
        b.armor,
        b.magic,
        csv(&slots),
        st.enemies_killed,
        st.generators_destroyed,
        st.gold_found,
        st.playtime,
        c.sumner_exp
    )
}

fn parse_carried(s: &str) -> Result<Carried> {
    let f = Fields::new(s);
    let [fight, speed, armor, magic] = parse_array::<f32, 4>(f.str("bonus")?)?;
    let stats: Vec<&str> = f.str("stats")?.split(',').collect();
    let [killed, gens, gold, time] =
        <[&str; 4]>::try_from(stats).map_err(|_| anyhow::anyhow!("stats wants 4 values"))?;
    let mut c = Carried {
        health: f.get("health")?,
        gold: f.get("gold")?,
        keys: f.get("keys")?,
        potions: parse_csv(f.str("potions")?)?,
        level: f.get("level")?,
        exp: f.get("exp")?,
        bonus: Atts {
            fight,
            speed,
            armor,
            magic,
        },
        powerups: Default::default(),
        stats: PlayStats {
            enemies_killed: killed.parse()?,
            generators_destroyed: gens.parse()?,
            gold_found: gold.parse()?,
            playtime: time.parse()?,
        },
        sumner_exp: f.get("sumner_exp")?,
    };
    for slot in parse_csv::<String>(f.str("powerups")?)? {
        let v: Vec<&str> = slot.split(':').collect();
        let [i, time, kind, amount, flags, state] =
            <[&str; 6]>::try_from(v).map_err(|_| anyhow::anyhow!("bad powerup {slot:?}"))?;
        let s = c
            .powerups
            .slots
            .get_mut(i.parse::<usize>()?)
            .context("no such powerup slot")?;
        *s = Slot {
            time: time.parse()?,
            kind: kind.parse()?,
            amount: amount.parse()?,
            flags: flags.parse()?,
            state: state.parse()?,
        };
    }
    Ok(c)
}

fn help_text(h: &[u8; 256]) -> String {
    let mut s = String::with_capacity(512);
    for b in h {
        let _ = write!(s, "{b:02x}");
    }
    s
}

fn parse_help(s: &str) -> Result<[u8; 256]> {
    if s.len() != 512 || !s.is_ascii() {
        bail!("bad help_disp");
    }
    let mut h = [0; 256];
    for (i, b) in h.iter_mut().enumerate() {
        *b = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16)?;
    }
    Ok(h)
}

fn write_start(o: &mut String, s: &LevelStart) {
    let _ = writeln!(o, "[start]");
    let _ = writeln!(o, "level {}", s.level);
    let _ = writeln!(o, "difficulty {}", s.difficulty);
    for p in &s.panels {
        let hidden = p.hidden.map_or("-".to_string(), |h| h.to_string());
        let _ = writeln!(
            o,
            "panel slot={} class={} colour={} hidden={hidden} name={}",
            p.slot,
            p.class,
            p.colour,
            esc(&p.name)
        );
    }
    for (slot, c) in s.carry.iter().enumerate() {
        if let Some(c) = c {
            let _ = writeln!(o, "carried {slot} {}", carried_text(c));
        }
    }
    for (slot, h) in s.help_disp.iter().enumerate() {
        if let Some(h) = h {
            let _ = writeln!(o, "help {slot} {}", help_text(h));
        }
    }
    let u = &s.setup;
    let _ = writeln!(o, "rules_name {}", u.rules.name());
    let _ = writeln!(o, "seed {}", u.seed);
    let _ = writeln!(o, "friendly_fire {}", u.friendly_fire);
    let _ = writeln!(o, "setup_crystals {}", csv(&u.crystals));
    let _ = writeln!(o, "setup_garg_items {}", csv(&u.garg_items));
    let _ = writeln!(o, "setup_finds {} {} {}", u.shards, u.runes, u.legend_held);
    let p = &s.progress;
    let _ = writeln!(o, "beaten {}", csv(&p.beaten));
    let _ = writeln!(o, "progress_finds {} {} {}", p.runes, p.shards, p.legend);
    let _ = writeln!(o, "crystals {}", csv(&p.crystals));
    let _ = writeln!(o, "garg_items {}", csv(&p.garg_items));
    if let Some(r) = p.last_realm {
        let _ = writeln!(o, "last_realm {r}");
    }
    if let Some(b) = p.last_beaten {
        let _ = writeln!(o, "last_beaten {b}");
    }
    let _ = writeln!(o, "banked {} {}", p.banked_runes, p.banked_shards);
    let _ = writeln!(
        o,
        "tower_seen {} {}",
        p.entered_tower as u8, p.welcomed as u8
    );
    let _ = writeln!(o, "boss_attempts {}", csv(&p.boss_attempts));
    let _ = writeln!(o, "level_masks {}", csv(&p.level_masks));
    let _ = writeln!(o, "saved_exp {}", csv(&s.saved_exp));
    if let Some(st) = &s.state {
        let text = st.to_text();
        let _ = writeln!(o, "state_lines {}", text.lines().count());
        o.push_str(&text);
        if !text.ends_with('\n') {
            o.push('\n');
        }
    }
}

fn read_start_line(s: &mut LevelStart, key: &str, rest: &str) -> Result<()> {
    let words: Vec<&str> = rest.split_whitespace().collect();
    let three = |what: &str| -> Result<[u16; 3]> {
        <[&str; 3]>::try_from(words.clone())
            .map_err(|_| anyhow::anyhow!("{what} wants 3 values"))
            .and_then(|w| Ok([w[0].parse()?, w[1].parse()?, w[2].parse()?]))
    };
    let two = || -> Result<[u16; 2]> {
        <[&str; 2]>::try_from(words.clone())
            .map_err(|_| anyhow::anyhow!("wanted 2 values"))
            .and_then(|w| Ok([w[0].parse()?, w[1].parse()?]))
    };
    let slot = |text: &str| -> Result<usize> {
        let n: usize = text.parse()?;
        if n >= 4 {
            bail!("no slot {n}");
        }
        Ok(n)
    };
    let p = &mut s.progress;
    match key {
        "level" => s.level = rest.to_string(),
        "difficulty" => s.difficulty = rest.parse()?,
        "panel" => {
            let f = Fields::new(rest);
            let hidden = f.str("hidden")?;
            s.panels.push(PanelInfo {
                slot: f.get("slot")?,
                name: unesc(f.str("name").unwrap_or("")),
                colour: f.get("colour")?,
                class: f.get("class")?,
                hidden: if hidden == "-" {
                    None
                } else {
                    Some(hidden.parse()?)
                },
            });
        }
        "carried" => {
            let (n, c) = rest.split_once(' ').context("carried wants a slot")?;
            s.carry[slot(n)?] = Some(parse_carried(c)?);
        }
        "help" => {
            let (n, h) = rest.split_once(' ').context("help wants a slot")?;
            s.help_disp[slot(n)?] = Some(parse_help(h)?);
        }
        "rules_name" => {
            s.setup.rules =
                Rules::named(rest).with_context(|| format!("unknown rules {rest:?}"))?;
        }
        "seed" => s.setup.seed = rest.parse()?,
        "friendly_fire" => s.setup.friendly_fire = rest.parse()?,
        "setup_crystals" => s.setup.crystals = parse_array(rest)?,
        "setup_garg_items" => s.setup.garg_items = parse_array(rest)?,
        "setup_finds" => {
            [s.setup.shards, s.setup.runes, s.setup.legend_held] = three("setup_finds")?;
        }
        "beaten" => p.beaten = parse_array(rest)?,
        "progress_finds" => [p.runes, p.shards, p.legend] = three("progress_finds")?,
        "crystals" => p.crystals = parse_array(rest)?,
        "garg_items" => p.garg_items = parse_array(rest)?,
        "last_realm" => p.last_realm = Some(rest.parse()?),
        "last_beaten" => p.last_beaten = Some(rest.parse()?),
        "banked" => [p.banked_runes, p.banked_shards] = two()?,
        "tower_seen" => {
            let [a, b] = two()?;
            p.entered_tower = a != 0;
            p.welcomed = b != 0;
        }
        "boss_attempts" => p.boss_attempts = parse_array(rest)?,
        "level_masks" => p.level_masks = parse_array(rest)?,
        "saved_exp" => s.saved_exp = parse_csv(rest)?,
        _ => {}
    }
    Ok(())
}

fn write_command(o: &mut String, tick: u32, c: &Command) {
    match c {
        Command::SetControls { player, prefs } => {
            let _ = writeln!(
                o,
                "{tick} controls {player} {} {} {} {}",
                prefs.control_style,
                prefs.rumble as u8,
                prefs.auto_aim as u8,
                prefs.auto_attack as u8
            );
        }
        Command::QuitLevel => {
            let _ = writeln!(o, "{tick} quit");
        }
        Command::HintsClosed => {
            let _ = writeln!(o, "{tick} hints_closed");
        }
        Command::PlayerLeft(player) => {
            let _ = writeln!(o, "{tick} left {player}");
        }
        Command::DebugGod(on) => {
            let _ = writeln!(o, "{tick} god {}", *on as u8);
        }
        Command::PartyBack(b) => {
            match &b.finds {
                Some((crystals, garg, shards)) => {
                    let _ = writeln!(o, "{tick} party {} {} {shards}", csv(crystals), csv(garg));
                }
                None => {
                    let _ = writeln!(o, "{tick} party");
                }
            }
            for (i, c) in b.carried.iter().enumerate() {
                if let Some(c) = c {
                    let _ = writeln!(o, "{tick} party_carried {i} {}", carried_text(c));
                }
            }
            for (i, h) in b.help_disp.iter().enumerate() {
                if let Some(h) = h {
                    let _ = writeln!(o, "{tick} party_help {i} {}", help_text(h));
                }
            }
        }
    }
}

fn read_command_line(out: &mut Vec<(u32, Command)>, tick: &str, rest: &str) -> Result<()> {
    let tick: u32 = tick.parse()?;
    let (key, rest) = rest.split_once(' ').unwrap_or((rest, ""));
    let w: Vec<&str> = rest.split_whitespace().collect();
    let c = match key {
        "controls" => {
            let [player, style, rumble, aim, attack] =
                <[&str; 5]>::try_from(w).map_err(|_| anyhow::anyhow!("controls wants 5 values"))?;
            Command::SetControls {
                player: player.parse()?,
                prefs: PlayerPrefs {
                    control_style: style.parse()?,
                    rumble: rumble == "1",
                    auto_aim: aim == "1",
                    auto_attack: attack == "1",
                },
            }
        }
        "quit" => Command::QuitLevel,
        "hints_closed" => Command::HintsClosed,
        "left" => Command::PlayerLeft(rest.parse()?),
        "god" => Command::DebugGod(rest == "1"),
        "party" => {
            let finds = match w.as_slice() {
                [] => None,
                [crystals, garg, shards] => {
                    Some((parse_array(crystals)?, parse_array(garg)?, shards.parse()?))
                }
                _ => bail!("party wants the finds or nothing"),
            };
            Command::PartyBack(Box::new(PartyBack {
                finds,
                ..Default::default()
            }))
        }
        "party_carried" | "party_help" => {
            let Some((_, Command::PartyBack(b))) = out.last_mut() else {
                bail!("{key} with no party before it");
            };
            let (i, text) = rest.split_once(' ').context("wants a player")?;
            let i: usize = i.parse()?;
            if i >= 4 {
                bail!("no player {i}");
            }
            if key == "party_carried" {
                b.carried.resize(b.carried.len().max(i + 1), None);
                b.carried[i] = Some(parse_carried(text)?);
            } else {
                b.help_disp.resize(b.help_disp.len().max(i + 1), None);
                b.help_disp[i] = Some(parse_help(text)?);
            }
            return Ok(());
        }
        _ => bail!("unknown command {key:?}"),
    };
    out.push((tick, c));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::setup::LevelSetup;
    use crate::game::PlayerState;
    use crate::input::PlayerInput;
    use crate::progress::Progress;
    use glam::Vec2;

    fn data() -> Option<GameFs> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game-data");
        GameFs::open(&root).ok()
    }

    fn compat() -> Compat {
        Compat {
            build: "test".into(),
            content: 1,
            rules: 2,
            env: 3,
        }
    }

    fn panel(slot: usize, class: usize) -> PanelInfo {
        PanelInfo {
            slot,
            name: format!("P {slot}"),
            colour: slot,
            class,
            hidden: None,
        }
    }

    fn start(level: &str) -> LevelStart {
        let progress = Progress::default();
        LevelStart {
            level: level.into(),
            difficulty: 1,
            panels: vec![panel(0, 0), panel(1, 1)],
            setup: LevelSetup::new(&Rules::classic(), 0, &progress),
            progress,
            ..Default::default()
        }
    }

    /// Bots play, as they do in the engine: packed, recorded, then ticked.
    fn record(g: &mut Game, r: &mut Replay, ticks: u32) {
        for _ in 0..ticks {
            let inputs: Vec<PackedInput> = (0..g.players.len())
                .map(|i| g.bot_input(i).pack())
                .collect();
            g.update_packed(&inputs);
            r.tick(&inputs, g);
        }
    }

    fn sample_carried() -> Carried {
        let mut c = Carried::new_character();
        c.gold = 123;
        c.potions = vec![0, 2, 1];
        c.bonus.fight = 3.5;
        c.health = 0.1 + 0.2;
        c.stats.playtime = 1.0 / 3.0;
        c.powerups.slots[3] = Slot {
            time: -1.0,
            kind: 7,
            amount: 0.3,
            flags: 0x8000_0001,
            state: 2,
        };
        c
    }

    #[test]
    fn a_stick_survives_packing_at_its_extremes() {
        let i = PlayerInput {
            stick: Vec2::new(1.0, -1.0),
            fire: Vec2::new(2.0, 0.0),
            ..Default::default()
        };
        let u = i.pack().unpack();
        assert_eq!(u.stick, Vec2::new(1.0, -1.0));
        assert_eq!(u.fire.x, 1.0, "past the edge is the edge");
        assert_eq!(
            PlayerInput::default().pack().unpack(),
            PlayerInput::default()
        );
    }

    #[test]
    fn packing_is_stable() {
        for raw in -127..=127i8 {
            let p = PackedInput {
                stick: [raw, raw.wrapping_neg()],
                ..Default::default()
            };
            assert_eq!(p.unpack().pack(), p);
        }
    }

    #[test]
    fn every_button_has_its_own_bit() {
        for bit in 0..19 {
            let p = PackedInput {
                buttons: 1 << bit,
                ..Default::default()
            };
            assert_eq!(p.unpack().pack(), p, "bit {bit}");
            assert_ne!(p.unpack(), PlayerInput::default(), "bit {bit}");
        }
    }

    #[test]
    fn inputs_read_back() {
        for i in [
            PackedInput::default(),
            PackedInput {
                stick: [-127, 127],
                fire: [1, -1],
                dpad: [0, -127],
                buttons: 0x7FFFF,
            },
        ] {
            assert_eq!(parse_input(&input_text(&i)).unwrap(), i);
        }
    }

    #[test]
    fn the_start_and_commands_read_back() {
        let mut s = start("levelA1");
        s.carry[1] = Some(sample_carried());
        s.help_disp[0] = Some([9; 256]);
        s.panels[1].hidden = Some(2);
        s.setup.rules = Rules::remix();
        s.setup.seed = u64::MAX;
        s.setup.crystals[2] = -1;
        s.progress.beaten[3] = 5;
        s.progress.last_realm = Some(4);
        s.progress.welcomed = true;
        s.saved_exp = vec![1, 2];
        let mut r = Replay::new(s.clone());
        let party = Command::PartyBack(Box::new(PartyBack {
            carried: vec![None, Some(sample_carried())],
            finds: Some(([1; 9], [-1, 0, 2], 7)),
            help_disp: vec![Some([3; 256])],
        }));
        let commands = [
            Command::SetControls {
                player: 1,
                prefs: PlayerPrefs {
                    control_style: 2,
                    rumble: false,
                    auto_aim: true,
                    auto_attack: false,
                },
            },
            Command::QuitLevel,
            Command::HintsClosed,
            Command::PlayerLeft(1),
            Command::DebugGod(true),
            party,
        ];
        for c in &commands {
            r.command(c);
        }
        r.compat = Some(compat());
        let back = Replay::from_text(&r.text()).unwrap();
        assert_eq!(back.start, s);
        assert_eq!(back.compat, Some(compat()));
        let got: Vec<&Command> = back.commands.iter().map(|(_, c)| c).collect();
        assert_eq!(got, commands.iter().collect::<Vec<_>>());
    }

    #[test]
    fn a_stranger_file_is_refused() {
        assert!(Replay::from_text("hello").is_err());
        assert!(Replay::from_text("gauntlet-replay 999\n").is_err());
        assert!(Replay::from_text("gauntlet-replay 1\nticks 5\n[inputs]\n2 -\n").is_err());
    }

    /// The whole loop: record, write, read, play, compare every section.
    fn round_trip(s: LevelStart, ticks: u32, during: &dyn Fn(&mut Game, &mut Replay)) -> Verdict {
        let fs = data().unwrap();
        let (mut g, _) = s.build(&fs).unwrap();
        let mut r = Replay::new(s);
        record(&mut g, &mut r, ticks / 2);
        during(&mut g, &mut r);
        record(&mut g, &mut r, ticks / 2);
        let text = r.to_text(&g, &compat());
        let back = Replay::from_text(&text).unwrap();
        back.check(&fs, None).unwrap()
    }

    #[test]
    fn a_level_plays_back_the_same() {
        if data().is_none() {
            return;
        }
        let v = round_trip(start("levelA1"), 1200, &|_, _| {});
        assert!(v.matches(), "{v}");
        assert_eq!(v.ticks, 1200);
    }

    #[test]
    fn heroes_with_things_play_back_the_same() {
        if data().is_none() {
            return;
        }
        let mut s = start("levelA1");
        s.carry[0] = Some(sample_carried());
        s.carry[1] = Some(sample_carried());
        s.help_disp[1] = Some([1; 256]);
        s.setup.seed = 99;
        s.setup.rules = Rules::remix();
        let v = round_trip(s, 600, &|_, _| {});
        assert!(v.matches(), "{v}");
    }

    #[test]
    fn the_tower_plays_back_the_same() {
        if data().is_none() {
            return;
        }
        let v = round_trip(start("levelL1"), 600, &|_, _| {});
        assert!(v.matches(), "{v}");
    }

    #[test]
    fn commands_play_back_at_their_tick() {
        if data().is_none() {
            return;
        }
        let v = round_trip(start("levelA1"), 600, &|g, r| {
            for c in [
                Command::DebugGod(true),
                Command::SetControls {
                    player: 0,
                    prefs: PlayerPrefs {
                        control_style: 1,
                        ..Default::default()
                    },
                },
                Command::PartyBack(Box::new(PartyBack {
                    carried: vec![Some(sample_carried())],
                    finds: Some(([1; 9], [2; 3], 3)),
                    help_disp: vec![None, Some([1; 256])],
                })),
            ] {
                r.command(&c);
                g.command(c);
            }
        });
        assert!(v.matches(), "{v}");
    }

    #[test]
    fn a_quit_after_the_last_tick_is_played() {
        let Some(fs) = data() else { return };
        let s = start("levelA1");
        let (mut g, _) = s.build(&fs).unwrap();
        let mut r = Replay::new(s);
        record(&mut g, &mut r, 30);
        r.command(&Command::QuitLevel);
        g.command(Command::QuitLevel);
        let back = Replay::from_text(&r.to_text(&g, &compat())).unwrap();
        let v = back.check(&fs, None).unwrap();
        assert!(v.matches(), "{v}");
        let (mut p, _) = back.build(&fs).unwrap();
        let mut pb = Playback::new();
        while pb.step(&back, &mut p) {}
        assert_eq!(p.players[0].state, PlayerState::InTower);
    }

    #[test]
    fn a_save_state_start_plays_back_the_same() {
        let Some(fs) = data() else { return };
        let (mut g, _) = start("levelA1").build(&fs).unwrap();
        for _ in 0..300 {
            let inputs: Vec<_> = (0..g.players.len()).map(|i| g.bot_input(i)).collect();
            g.update(&inputs);
        }
        let mut s = start("levelA1");
        s.state = Some(SaveState::capture(&g, &s.panels, &[], &s.progress));
        let v = round_trip(s, 600, &|_, _| {});
        assert!(v.matches(), "{v}");
    }

    #[test]
    fn a_changed_input_is_caught_near_where_it_happened() {
        let Some(fs) = data() else { return };
        let s = start("levelA1");
        let (mut g, _) = s.build(&fs).unwrap();
        let mut r = Replay::new(s);
        record(&mut g, &mut r, 600);
        let mut back = Replay::from_text(&r.to_text(&g, &compat())).unwrap();
        // One walk, cut short from tick 300 on.
        let (mut t, mut changed) = (0, false);
        for run in &mut back.runs {
            if t >= 300 && !changed && run.inputs.iter().any(|i| i.stick != [0, 0]) {
                for i in &mut run.inputs {
                    i.stick = [-i.stick[0], -i.stick[1]];
                }
                changed = true;
            }
            t += run.ticks;
        }
        assert!(changed, "the bots never walked");
        let v = back.check(&fs, None).unwrap();
        assert!(!v.matches());
        let d = v.diverged.expect("a check catches it");
        assert!(
            d.good >= 300 - CHECK_EVERY && d.bad == d.good + CHECK_EVERY,
            "{d:?}"
        );
        assert!(!v.end_differs.is_empty());
    }

    #[test]
    fn a_taint_ends_the_recording_and_is_told() {
        let Some(fs) = data() else { return };
        let s = start("levelA1");
        let (mut g, _) = s.build(&fs).unwrap();
        let mut r = Replay::new(s);
        record(&mut g, &mut r, 120);
        r.taint("the debug menu changed the game");
        g.players[0].gold = 999;
        record(&mut g, &mut r, 120);
        assert_eq!(r.ticks, 120);
        let back = Replay::from_text(&r.to_text(&g, &compat())).unwrap();
        let v = back.check(&fs, None).unwrap();
        assert!(v.matches(), "{v}");
        assert_eq!(v.ticks, 120);
        assert!(v.to_string().contains("debug menu"));
    }

    #[test]
    fn another_copy_is_mentioned_but_still_plays() {
        let Some(fs) = data() else { return };
        let s = start("levelA1");
        let (mut g, _) = s.build(&fs).unwrap();
        let mut r = Replay::new(s);
        record(&mut g, &mut r, 60);
        let back = Replay::from_text(&r.to_text(&g, &compat())).unwrap();
        let mut ours = compat();
        ours.build = "newer".into();
        ours.content = 9;
        let v = back.check(&fs, Some(&ours)).unwrap();
        assert!(v.matches());
        assert_eq!(v.differences.len(), 2);
        assert!(v.to_string().contains("newer"));
    }
}
