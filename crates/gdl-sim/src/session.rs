//! The session: everything between the title screen and game over that every
//! peer of a networked game must agree on. It owns the running level, the
//! party, what they carry and have done, and which level or screen comes next.
//! It moves on only by [`Session::update`] (inputs, one 60 Hz tick) and
//! [`Session::command`] (decisions), so two sessions fed the same stay the same.
//!
//! Screens that read a player's own memory card or options (player select, the
//! pause menu) stay in the engine, and report what was decided as commands. The
//! engine also draws and sounds each phase, told what to do by [`Event`]s.

use crate::fs::GameFs;
use crate::game::savestate::SaveState;
use crate::game::setup::{Command, LevelSetup, LevelStart, PartyBack};
use crate::game::{Carried, Game, LevelEnd, PanelInfo, PlayerState};
use crate::input::{PackedInput, PlayerInput};
use crate::mapclock::MapClock;
use crate::progress::Progress;
use crate::replay::Replay;
use crate::rules::Rules;
use crate::save::HeroSave;
use crate::shop::{Shop, ShopMode};
use anyhow::Result;
use gdl_formats::movie::Avi;
use gdl_formats::wdata::{Worlds, TOWER_WAVE};

/// Indexed by slot (column), so players keep their things whoever joins.
pub type Carry = [Option<Carried>; 4];
pub type Heroes = [Option<HeroSave>; 4];

/// The tower's level directory.
pub const TOWER: &str = "levelL1";

/// Whether a level start refreshes the players' checkpoint (save_backup):
/// start_level skips PlayerSaveState in the secret levels (realm 12), and
/// Garm's and Skorne's levels.
fn checkpoint_at(wave: Option<u32>) -> bool {
    !matches!(wave, Some(0x501 | 0x601)) && wave.is_none_or(|w| w >> 8 != 12)
}

/// A movie's clock (PlayVQMovie): 30 frames a second, any newly pressed button
/// ending it a couple of frames later.
#[derive(Clone, Debug)]
pub struct MovieClock {
    frames: usize,
    rate: f32,
    /// Video frames due so far.
    pub shown: usize,
    time: f32,
    /// Frames left after a skip (PlayVQMovie stops 2-3 frames after the press).
    stopping: Option<u32>,
    /// A button was held last tick (skips need a fresh press).
    held: bool,
}

impl MovieClock {
    pub fn new(frames: usize, rate: f32) -> MovieClock {
        MovieClock {
            frames,
            rate,
            shown: 0,
            time: 0.0,
            stopping: None,
            held: true,
        }
    }

    /// `VQMOVIES/<name>.avi`. Movies that aren't 512 wide are refused, as the
    /// game does.
    pub fn open(fs: &GameFs, name: &str) -> Option<MovieClock> {
        let bytes = fs.read(&format!("VQMOVIES/{name}.avi")).ok()?;
        let avi = Avi::parse(&bytes).ok()?;
        (avi.width() == 512).then(|| MovieClock::new(avi.video_chunks.len(), avi.frame_rate()))
    }

    /// Advance by `dt` seconds. True once the movie has ended.
    pub fn update(&mut self, dt: f32, inputs: &[PlayerInput]) -> bool {
        let pressed = inputs.iter().any(|i| {
            i.attack
                || i.attack_pressed
                || i.magic_pressed
                || i.b_pressed
                || i.start_pressed
                || i.turbo
        });
        if pressed && !self.held && self.stopping.is_none() {
            self.stopping = Some(3);
        }
        self.held = pressed;
        self.time += dt;
        let want = ((self.time * self.rate) as usize + 1).min(self.frames);
        while self.shown < want {
            self.shown += 1;
            if let Some(n) = &mut self.stopping {
                *n -= 1;
                if *n == 0 {
                    return true;
                }
            }
        }
        self.shown >= self.frames && self.time * self.rate >= self.frames as f32
    }
}

/// What a movie leads to.
#[derive(Clone, Debug)]
pub enum AfterMovie {
    Level(String),
    Shop(Box<Shop>),
}

#[derive(Clone, Debug, Default)]
pub enum Phase {
    /// No session running: the engine's title and attract screens.
    #[default]
    Idle,
    /// Player select is up, somewhere; it reports with `Decision::PartyChosen`.
    Select,
    /// The level is about to load, by [`Session::load`].
    Loading {
        level: String,
    },
    /// In a level ([`Session::game`]).
    Playing,
    /// The map screen, then `level` after `movie`; or back to the level waiting
    /// for a party coming home from a secret level.
    Map {
        clock: MapClock,
        movie: String,
        level: String,
        resume: bool,
    },
    Movie {
        clock: MovieClock,
        then: AfterMovie,
    },
    /// The shop: after a level (then player select), or from the Tower Menu
    /// (then back to the tower as it was).
    Shop {
        shop: Box<Shop>,
        from_menu: bool,
    },
    /// Nobody left playing.
    Over,
}

/// A level waiting for the party to come back.
#[derive(Clone)]
struct Suspended {
    game: Box<Game>,
    recording: Option<Replay>,
}

/// What the engine should show or do next.
#[derive(Clone)]
pub enum Event {
    /// The level is gone, for good: the game as it ended, and its recording.
    LevelGone {
        game: Box<Game>,
        recording: Option<Box<Replay>>,
    },
    /// The level waits for the party to come back.
    LevelSuspended,
    /// The party is back in the level that waited.
    Resumed {
        from_shop: bool,
    },
    /// These players (by index in the party as it was) have left.
    PlayersLeft(Vec<usize>),
    /// A phase has begun; see [`Session::phase`].
    Select,
    Loading,
    Map {
        wave: u32,
    },
    Movie {
        name: String,
    },
    Shop,
    GameOver,
    /// Sumner's lectern: this player's hint menu. The session waits.
    HintMenu(usize),
}

/// Player select's result.
#[derive(Clone, Debug)]
pub struct Chosen {
    pub panels: Vec<PanelInfo>,
    pub carry: Carry,
    pub heroes: Heroes,
    /// The party's progress with what loaded heroes brought.
    pub progress: Progress,
}

/// A decision from outside the session, applied between ticks.
#[derive(Clone, Debug)]
pub enum Decision {
    /// A change to the running level.
    Game(Command),
    /// A player opened a menu: the level waits.
    Pause(usize),
    Unpause,
    /// The options that change a level, from the next one on.
    Settings {
        difficulty: usize,
        friendly_fire: usize,
    },
    /// The Tower Menu's Shop or Inventory: the tower waits.
    OpenShop(ShopMode),
    /// Back to player select from the tower (Manage Character, or a new player
    /// joining).
    ManageParty,
    /// Player select is done: into the tower.
    PartyChosen(Box<Chosen>),
    /// "Quit Game" from a menu.
    QuitGame,
    /// Leave a movie now.
    SkipMovie,
}

#[derive(Clone, Default)]
pub struct Session {
    pub phase: Phase,
    /// The running level.
    pub game: Option<Box<Game>>,
    /// Its recording, from its start.
    pub recording: Option<Replay>,
    /// The player whose menu has the level waiting.
    pub paused: Option<usize>,
    pub panels: Vec<PanelInfo>,
    pub progress: Progress,
    /// What each slot brings to the next level.
    pub carry: Option<Carry>,
    /// What each slot had at the last checkpoint: heroes who die go back to it.
    pub checkpoint: Option<Carry>,
    pub heroes: Heroes,
    /// A debug save state to apply to the next level.
    pub pending_state: Option<SaveState>,
    /// Start the next level from exactly this, unrecorded (a watched replay).
    pub start_override: Option<LevelStart>,
    pub rules: Option<Rules>,
    pub difficulty: usize,
    pub friendly_fire: usize,
    /// Slots whose players have left: their heroes play no more levels.
    leavers: Vec<usize>,
    /// Sounds to play, drained by the engine.
    pub sfx: Vec<String>,
    /// What happened since the engine last looked, drained by it.
    pub events: Vec<Event>,
    suspended: Option<Suspended>,
}

impl Session {
    pub fn new(rules: Rules, difficulty: usize, friendly_fire: usize, progress: Progress) -> Self {
        Session {
            rules: Some(rules),
            difficulty,
            friendly_fire,
            progress,
            ..Default::default()
        }
    }

    pub fn rules(&self) -> Rules {
        self.rules.clone().unwrap_or_else(Rules::classic)
    }

    /// Player select is up, for a new party.
    pub fn enter_select(&mut self) {
        self.phase = Phase::Select;
        self.events.push(Event::Select);
    }

    /// Go to a level directly, with this party.
    pub fn enter_level(&mut self, level: String, panels: Vec<PanelInfo>) {
        self.panels = panels;
        self.phase = Phase::Loading { level };
        self.events.push(Event::Loading);
    }

    /// What this session starts the level from.
    pub fn level_start(&self, level: &str) -> LevelStart {
        let hero = |info: &PanelInfo| self.heroes[info.slot % 4].as_ref();
        let saved_exp = self
            .panels
            .iter()
            .filter_map(hero)
            .flat_map(|h| h.classes.iter())
            .filter_map(|c| c.carried.as_ref().map(|k| k.exp))
            .collect();
        let mut help_disp = [None; 4];
        for info in &self.panels {
            help_disp[info.slot % 4] = hero(info).map(|h| h.help_disp);
        }
        LevelStart {
            level: level.to_string(),
            difficulty: self.difficulty,
            panels: self.panels.clone(),
            carry: self.carry.clone().unwrap_or_default(),
            help_disp,
            setup: LevelSetup::new(&self.rules(), self.friendly_fire, &self.progress),
            progress: self.progress.clone(),
            saved_exp,
            state: self.pending_state.clone(),
        }
    }

    /// Load the level [`Phase::Loading`] names. On failure the session is over
    /// and the engine goes back to its title.
    pub fn load(&mut self, fs: &GameFs) -> Result<()> {
        let Phase::Loading { level } = &self.phase else {
            anyhow::bail!("no level to load");
        };
        let level = level.clone();
        let watched = self.start_override.take();
        let start = match &watched {
            Some(s) => s.clone(),
            None => self.level_start(&level),
        };
        let (game, welcome) = match start.build(fs) {
            Ok(built) => built,
            Err(e) => {
                self.phase = Phase::Idle;
                return Err(e);
            }
        };
        self.recording = watched.is_none().then(|| Replay::new(start));
        if checkpoint_at(game.wave) || self.checkpoint.is_none() {
            self.checkpoint = self.carry.clone();
        }
        match game.wave {
            Some(TOWER_WAVE) => {
                self.progress.entered_tower = true;
                self.progress.welcomed |= welcome;
            }
            Some(w) => self.progress.last_realm = Some(w >> 8),
            None => {}
        }
        self.pending_state = None;
        self.game = Some(Box::new(game));
        self.paused = None;
        self.phase = Phase::Playing;
        for slot in self.leavers.clone() {
            if let Some(i) = self.panels.iter().position(|p| p.slot == slot) {
                self.command(fs, Decision::Game(Command::PlayerLeft(i)));
            }
        }
        Ok(())
    }

    /// The party's progress as it stands in the level being played, with what
    /// Sumner has banked in the tower so far: a hero saved from the tower (or
    /// the tower re-entered after player select) keeps the speeches given.
    pub fn live_progress(&self) -> Progress {
        let mut progress = self.progress.clone();
        if let Some(g) = &self.game {
            progress.crystals = g.crystals;
            progress.garg_items = g.garg_items;
            progress.shards = g.shards;
            progress.runes |= g.runes | g.boss_runes;
            progress.legend = g.legend_held;
            if let Some(sm) = &g.sumner {
                (progress.banked_runes, progress.banked_shards) = sm.banked();
                progress.last_beaten = sm.last_beaten();
            }
        }
        progress
    }

    /// What lives in each player's save image during play goes back to the
    /// column's hero: the secret classes unlocked (save.class_unlock) and the
    /// messages seen (save.help_disp).
    pub fn bank_heroes(&mut self) {
        if let Some(g) = &self.game {
            bank_heroes(g, &self.panels, &mut self.heroes);
        }
    }

    /// What each slot carries right now.
    pub fn live_carry(&self) -> Carry {
        let mut carry: Carry = Default::default();
        if let Some(g) = &self.game {
            for (p, info) in g.players.iter().zip(&self.panels) {
                carry[info.slot % 4] = Some(p.carried());
            }
        }
        carry
    }

    /// Debug warp: to another level with the party as it stands.
    pub fn warp(&mut self, level: String, default_panels: impl FnOnce() -> Vec<PanelInfo>) {
        self.discard_waiting();
        if self.game.is_some() {
            self.bank_heroes();
            self.carry = Some(self.live_carry());
            self.drop_level();
        } else {
            self.carry = None;
            self.checkpoint = None;
            self.panels = default_panels();
        }
        self.phase = Phase::Loading { level };
        self.events.push(Event::Loading);
    }

    /// Start a debug save state: its party and progress, then its level (the
    /// changes are applied once it has loaded).
    pub fn begin_state(&mut self, st: SaveState) {
        self.discard_waiting();
        self.drop_level();
        self.progress = st.progress.clone();
        let mut carry: Carry = Default::default();
        for h in &st.heroes {
            carry[h.panel.slot % 4] = Some(h.carried());
        }
        self.carry = Some(carry);
        self.checkpoint = self.carry.clone();
        self.difficulty = st.difficulty.min(2);
        self.panels = st.heroes.iter().map(|h| h.panel.clone()).collect();
        let level = st.level.clone();
        self.pending_state = Some(st);
        self.phase = Phase::Loading { level };
        self.events.push(Event::Loading);
    }

    /// This slot's player has left the game for good: their hero leaves the
    /// level, or any level to come, and the party when that level ends.
    pub fn player_left(&mut self, fs: &GameFs, slot: usize) {
        if !self.leavers.contains(&slot) {
            self.leavers.push(slot);
        }
        let Some(i) = self.panels.iter().position(|p| p.slot == slot) else {
            return;
        };
        if self.paused == Some(i) {
            self.paused = None;
        }
        if self.game.is_some() {
            self.command(fs, Decision::Game(Command::PlayerLeft(i)));
        }
        if let Some(s) = &mut self.suspended {
            let c = Command::PlayerLeft(i);
            if let Some(rec) = &mut s.recording {
                rec.command(&c);
            }
            s.game.command(c);
        }
    }

    /// Is a level waiting for the party to come back?
    pub fn has_waiting(&self) -> bool {
        self.suspended.is_some()
    }

    /// The waiting level won't be wanted; its recording is still kept.
    fn discard_waiting(&mut self) {
        if let Some(s) = self.suspended.take() {
            self.events.push(Event::LevelGone {
                game: s.game,
                recording: s.recording.map(Box::new),
            });
        }
    }

    /// The level goes, and the engine hears of it.
    fn drop_level(&mut self) {
        if let Some(game) = self.game.take() {
            let recording = self.recording.take().map(Box::new);
            self.events.push(Event::LevelGone { game, recording });
        }
        self.paused = None;
    }

    pub fn command(&mut self, fs: &GameFs, d: Decision) {
        match d {
            Decision::Game(c) => {
                if let Some(rec) = &mut self.recording {
                    rec.command(&c);
                }
                if let Some(g) = &mut self.game {
                    g.command(c);
                }
            }
            Decision::Pause(player) => self.paused = Some(player),
            Decision::Unpause => self.paused = None,
            Decision::Settings {
                difficulty,
                friendly_fire,
            } => {
                self.difficulty = difficulty;
                self.friendly_fire = friendly_fire;
            }
            Decision::OpenShop(mode) => self.open_shop(fs, mode),
            Decision::ManageParty => {
                self.progress = self.live_progress();
                self.bank_heroes();
                self.carry = Some(self.live_carry());
                self.drop_level();
                self.enter_select();
            }
            Decision::PartyChosen(c) => {
                self.carry = Some(c.carry);
                self.heroes = c.heroes;
                self.progress = c.progress;
                self.leavers
                    .retain(|slot| c.panels.iter().any(|p| p.slot == *slot));
                // Player select always leads into the tower.
                self.enter_level(TOWER.to_string(), c.panels);
            }
            Decision::QuitGame => {
                self.discard_waiting();
                self.drop_level();
                self.phase = Phase::Over;
                self.events.push(Event::GameOver);
            }
            Decision::SkipMovie => {
                if matches!(self.phase, Phase::Movie { .. }) {
                    self.movie_over();
                }
            }
        }
    }

    /// One 60 Hz tick.
    pub fn update(&mut self, fs: &GameFs, inputs: &[PackedInput]) {
        let unpacked = || -> Vec<PlayerInput> { inputs.iter().map(PackedInput::unpack).collect() };
        match &mut self.phase {
            Phase::Playing => {
                if self.paused.is_some() {
                    return;
                }
                let ended = self.game.as_ref().is_some_and(|g| g.level_end.is_some());
                if !ended {
                    let Some(g) = &mut self.game else { return };
                    g.update_packed(inputs);
                    if let Some(rec) = &mut self.recording {
                        rec.tick(inputs, g);
                    }
                    if let Some(pi) = g.sumner.as_mut().and_then(|s| s.hint_request.take()) {
                        self.paused = Some(pi);
                        self.events.push(Event::HintMenu(pi));
                    }
                }
                self.check_level_end(fs);
            }
            Phase::Map { clock, .. } => {
                let done = clock.update();
                self.sfx.append(&mut clock.sfx);
                if done {
                    self.map_over(fs);
                }
            }
            Phase::Movie { clock, .. } => {
                if clock.update(crate::game::TICK, &unpacked()) {
                    self.movie_over();
                }
            }
            Phase::Shop { shop, .. } => {
                let done = shop.update(&unpacked());
                self.sfx.append(&mut shop.sfx);
                if done {
                    self.shop_over(fs);
                }
            }
            Phase::Idle | Phase::Select | Phase::Loading { .. } | Phase::Over => {}
        }
    }

    fn check_level_end(&mut self, fs: &GameFs) {
        let Some(end) = self.game.as_ref().and_then(|g| g.level_end) else {
            return;
        };
        // The level's last sounds still play.
        if let Some(g) = &mut self.game {
            self.sfx.append(&mut g.sfx);
        }
        match end {
            LevelEnd::Secret(w) => self.leave_for_secret(fs, Some(w)),
            LevelEnd::SecretOver => self.leave_for_secret(fs, None),
            end => self.level_finished(fs, end),
        }
    }

    /// A secret icon: the level is kept as it is, to come back to when the
    /// secret level's time runs out (SaveAllRecords / LoadAllRecords).
    fn leave_for_secret(&mut self, fs: &GameFs, to: Option<u32>) {
        // level_end stays set: a recording's last fingerprint has it, as its
        // replay will. A level that waits has it cleared when it comes back.
        let Some(game) = self.game.take() else {
            return;
        };
        bank_heroes(&game, &self.panels, &mut self.heroes);
        self.progress.crystals = game.crystals;
        self.progress.garg_items = game.garg_items;
        self.progress.shards = game.shards;
        self.progress.legend = game.legend_held;
        let mut carry: Carry = Default::default();
        for (p, info) in game.players.iter().zip(&self.panels) {
            carry[info.slot % 4] = Some(p.carried());
        }
        self.carry = Some(carry);
        let wave = to.or(game.wave).unwrap_or(0xC00);
        let mut clock = MapClock::new(fs, &game.worlds, wave);
        // The way to and from a secret level is silent.
        clock.music = None;
        let level = to.and_then(|w| game.worlds.dir(w)).unwrap_or_default();
        let recording = self.recording.take();
        if to.is_some() {
            self.suspended = Some(Suspended { game, recording });
            self.events.push(Event::LevelSuspended);
        } else {
            self.events.push(Event::LevelGone {
                game,
                recording: recording.map(Box::new),
            });
        }
        self.phase = Phase::Map {
            clock,
            movie: String::new(),
            level,
            resume: to.is_none(),
        };
        self.events.push(Event::Map { wave });
    }

    /// A finished level: tower portals lead to their level (after its intro
    /// movie); a level leads back to the tower through player select.
    fn level_finished(&mut self, fs: &GameFs, end: LevelEnd) {
        // The party fell in a secret level: the level it left won't come back.
        self.discard_waiting();
        // level_end stays set: a recording's last fingerprint has it, as its
        // replay will.
        let Some(mut game) = self.game.take() else {
            return;
        };
        let recording = self.recording.take().map(Box::new);
        bank_heroes(&game, &self.panels, &mut self.heroes);
        // Crystals are kept whether or not the level was finished.
        let p = &mut self.progress;
        p.crystals = game.crystals;
        p.garg_items = game.garg_items;
        p.shards = game.shards;
        p.legend = game.legend_held;
        // BossDeath: PlayerGiveRune, the boss's shard (a "rune" in the code).
        p.runes |= game.boss_runes | game.runes;
        // A death on a Guardian's level counts as an attempt at it (more of its
        // hint unlocks).
        if let Some(w) = game.wave.filter(|&w| w != TOWER_WAVE) {
            let last = game
                .worlds
                .realm(w >> 8)
                .map_or(0, |r| r.levels.len().saturating_sub(1));
            if (w & 0xFF) as usize == last {
                for _ in 0..game.deaths {
                    p.record_boss_death(w >> 8);
                }
            }
        }
        if let Some(sm) = &game.sumner {
            (p.banked_runes, p.banked_shards) = sm.banked();
            p.last_beaten = sm.last_beaten();
        }
        let checkpoint = self.checkpoint.clone().unwrap_or_default();
        // Players who died in the level get back what they had at its start.
        let mut carry: Carry = Default::default();
        for (p, info) in game.players.iter().zip(&self.panels) {
            let slot = info.slot % 4;
            carry[slot] = match (p.state, &checkpoint[slot]) {
                (PlayerState::InTower, Some(c)) => Some(c.clone()),
                _ => Some(p.carried()),
            };
        }
        // Players who chose "Quit Game" while dead leave the party.
        let mut panels = Vec::new();
        let mut left = Vec::new();
        for (i, info) in self.panels.iter().enumerate() {
            if game.players.get(i).is_some_and(|p| p.quit) {
                carry[info.slot % 4] = None;
                left.push(i);
            } else {
                panels.push(info.clone());
            }
        }
        self.carry = Some(carry);
        self.panels = panels;
        let wave = game.wave;
        let worlds = std::mem::take(&mut game.worlds);
        self.events.push(Event::LevelGone { game, recording });
        if !left.is_empty() {
            self.events.push(Event::PlayersLeft(left));
        }
        if self.panels.is_empty() {
            // Nobody left playing: game over (MG_OVER), then the attract loop's
            // fail movie.
            self.phase = Phase::Over;
            self.events.push(Event::GameOver);
            return;
        }
        match (wave, end) {
            (Some(TOWER_WAVE), LevelEnd::Exit(Some(w))) => {
                let level = worlds.dir(w).unwrap_or_else(|| TOWER.to_string());
                let movie = worlds.level(w).map(|l| l.movie.clone()).unwrap_or_default();
                self.enter_map(fs, &worlds, w, movie, level);
            }
            (Some(TOWER_WAVE), _) | (_, LevelEnd::AllDead) => {
                self.phase = Phase::Loading {
                    level: TOWER.to_string(),
                };
                self.events.push(Event::Loading);
            }
            // Handled before this, with the level kept.
            (_, LevelEnd::Secret(_) | LevelEnd::SecretOver) => {}
            (w, LevelEnd::Exit(dest)) => {
                if let Some(w) = w {
                    let (rune, legend) = worlds.level(w).map_or((0, 0), |l| (l.rune, l.legend));
                    self.progress.record_beaten(w, rune, legend);
                }
                match dest.and_then(|d| Some((d, worlds.dir(d)?))) {
                    // E1 -> E2 and F1 -> F2 chain straight on, through the map.
                    Some((d, level)) => {
                        let movie = worlds.level(d).map(|l| l.movie.clone()).unwrap_or_default();
                        self.enter_map(fs, &worlds, d, movie, level);
                    }
                    None => self.after_level_shop(fs, &worlds, w),
                }
            }
        }
    }

    fn enter_map(&mut self, fs: &GameFs, worlds: &Worlds, wave: u32, movie: String, level: String) {
        self.phase = Phase::Map {
            clock: MapClock::new(fs, worlds, wave),
            movie,
            level,
            resume: false,
        };
        self.events.push(Event::Map { wave });
    }

    /// init_shop(0): the tally and the shop, then player select and the tower.
    /// After Garm, Final Stats.
    fn after_level_shop(&mut self, fs: &GameFs, worlds: &Worlds, wave: Option<u32>) {
        let carry = self.carry.clone().unwrap_or_default();
        let start = self.checkpoint.clone().unwrap_or_default();
        let party: Vec<_> = self
            .panels
            .iter()
            .map(|info| {
                let now = carry[info.slot % 4]
                    .clone()
                    .unwrap_or_else(Carried::new_character);
                let was = start[info.slot % 4].clone().unwrap_or(now.clone());
                (info.clone(), now, was)
            })
            .collect();
        let shop = Box::new(Shop::new(
            fs,
            party,
            wave.and_then(|w| worlds.level(w)).cloned(),
            ShopMode::AfterLevel,
            wave == Some(0x803),
        ));
        // After Skorne in the Underworld, "garm"; after Garm, "victory"; then
        // the shop (gamemain.c).
        let ending = match wave {
            Some(0x601) => "garm",
            Some(0x803) => "victory",
            _ => "",
        };
        match (!ending.is_empty())
            .then(|| MovieClock::open(fs, ending))
            .flatten()
        {
            Some(clock) => {
                self.phase = Phase::Movie {
                    clock,
                    then: AfterMovie::Shop(shop),
                };
                self.events.push(Event::Movie {
                    name: ending.to_string(),
                });
            }
            None => {
                self.phase = Phase::Shop {
                    shop,
                    from_menu: false,
                };
                self.events.push(Event::Shop);
            }
        }
    }

    /// init_shop(1 / 2) from the tower: the tower waits.
    fn open_shop(&mut self, fs: &GameFs, mode: ShopMode) {
        let Some(game) = self.game.take() else {
            return;
        };
        let party: Vec<_> = self
            .panels
            .iter()
            .zip(&game.players)
            .map(|(info, p)| (info.clone(), p.carried(), p.carried()))
            .collect();
        let recording = self.recording.take();
        self.suspended = Some(Suspended { game, recording });
        self.paused = None;
        self.events.push(Event::LevelSuspended);
        self.phase = Phase::Shop {
            shop: Box::new(Shop::new(fs, party, None, mode, false)),
            from_menu: true,
        };
        self.events.push(Event::Shop);
    }

    fn resume_level(&mut self, s: Suspended, back: PartyBack, from_shop: bool) {
        self.game = Some(s.game);
        self.recording = s.recording;
        self.paused = None;
        self.phase = Phase::Playing;
        let c = Command::PartyBack(Box::new(back));
        if let Some(rec) = &mut self.recording {
            rec.command(&c);
        }
        if let Some(g) = &mut self.game {
            g.command(c);
        }
        self.events.push(Event::Resumed { from_shop });
    }

    fn carried_now(&self) -> Vec<Option<Carried>> {
        self.panels
            .iter()
            .map(|info| self.carry.as_ref().and_then(|c| c[info.slot % 4].clone()))
            .collect()
    }

    fn map_over(&mut self, fs: &GameFs) {
        let Phase::Map {
            movie,
            level,
            resume,
            ..
        } = std::mem::take(&mut self.phase)
        else {
            return;
        };
        // Going in, the level stays where it is, waiting.
        let waiting = if resume { self.suspended.take() } else { None };
        match waiting {
            // Back where the secret was found, with what was gained there.
            Some(s) => {
                let p = &self.progress;
                let back = PartyBack {
                    carried: self.carried_now(),
                    finds: Some((p.crystals, p.garg_items, p.shards)),
                    help_disp: self
                        .panels
                        .iter()
                        .map(|info| self.heroes[info.slot % 4].as_ref().map(|h| h.help_disp))
                        .collect(),
                };
                self.resume_level(s, back, false);
            }
            // init_gamemovie: the level's intro movie if it has one, else straight to it.
            None => match (!movie.is_empty())
                .then(|| MovieClock::open(fs, &movie))
                .flatten()
            {
                Some(clock) => {
                    self.phase = Phase::Movie {
                        clock,
                        then: AfterMovie::Level(level),
                    };
                    self.events.push(Event::Movie { name: movie });
                }
                None => {
                    self.phase = Phase::Loading { level };
                    self.events.push(Event::Loading);
                }
            },
        }
    }

    fn movie_over(&mut self) {
        let Phase::Movie { then, .. } = std::mem::take(&mut self.phase) else {
            return;
        };
        match then {
            AfterMovie::Level(level) => {
                self.phase = Phase::Loading { level };
                self.events.push(Event::Loading);
            }
            AfterMovie::Shop(shop) => {
                self.phase = Phase::Shop {
                    shop,
                    from_menu: false,
                };
                self.events.push(Event::Shop);
            }
        }
    }

    fn shop_over(&mut self, _fs: &GameFs) {
        let Phase::Shop { shop, from_menu } = std::mem::take(&mut self.phase) else {
            return;
        };
        // What was bought goes with each player.
        let mut carry = self.carry.clone().unwrap_or_default();
        for (info, c) in self.panels.iter().zip(shop.players()) {
            carry[info.slot % 4] = Some(c);
        }
        self.carry = Some(carry);
        match self.suspended.take().filter(|_| from_menu) {
            Some(s) => {
                let back = PartyBack {
                    carried: self.carried_now(),
                    ..Default::default()
                };
                self.resume_level(s, back, true);
            }
            None => self.enter_select(),
        }
    }

    /// A cheap check that two sessions are the same, cheap enough to send every
    /// few seconds of play; [`Session::digest`] looks deeper.
    pub fn fingerprint(&self) -> u32 {
        let text = format!(
            "{:?} {:?} {:?} {:?} {:?} {:?} {}",
            self.phase,
            self.paused,
            self.panels,
            self.progress,
            self.carry,
            self.leavers,
            self.game.as_ref().map_or(0, |g| g.fingerprint()),
        );
        let mut h: u32 = 0x811c9dc5;
        for b in text.bytes() {
            h ^= b as u32;
            h = h.wrapping_mul(0x01000193);
        }
        h
    }

    /// The state peers must agree on, as one number.
    pub fn digest(&self) -> u64 {
        let text = format!(
            "{:?} {:?} {:?} {:?} {:?} {:?} {:?} {}",
            self.phase,
            self.paused,
            self.panels,
            self.progress,
            self.carry,
            self.checkpoint,
            self.heroes
                .iter()
                .map(|h| h.as_ref().map(|h| h.help_disp))
                .collect::<Vec<_>>(),
            self.game.as_ref().map_or(0, |g| g.state_digest()),
        );
        let mut h: u64 = 0xcbf29ce484222325;
        for b in text.bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        h
    }
}

/// See [`Session::bank_heroes`].
pub fn bank_heroes(game: &Game, panels: &[PanelInfo], heroes: &mut Heroes) {
    for (i, (info, p)) in panels.iter().zip(&game.players).enumerate() {
        let h = heroes[info.slot % 4]
            .get_or_insert_with(|| HeroSave::new(&info.name, info.class.min(15), info.colour));
        h.class_unlock |= p.class_unlock;
        h.help_disp = game.msgs.help_disp(i);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> Option<GameFs> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game-data");
        GameFs::open(&root).ok()
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

    /// Two heroes, in the tower.
    fn in_tower(fs: &GameFs) -> Session {
        let mut s = Session::new(Rules::classic(), 1, 0, Progress::default());
        s.enter_select();
        let panels = vec![panel(0, 0), panel(1, 1)];
        let mut carry: Carry = Default::default();
        for p in &panels {
            carry[p.slot] = Some(Carried::new_character());
        }
        s.command(
            fs,
            Decision::PartyChosen(Box::new(Chosen {
                panels,
                carry,
                heroes: Default::default(),
                progress: Progress::default(),
            })),
        );
        s.load(fs).unwrap();
        s
    }

    fn end_level(s: &mut Session, end: LevelEnd) {
        s.game.as_mut().unwrap().level_end = Some(end);
    }

    fn a_button() -> PackedInput {
        PlayerInput {
            attack: true,
            attack_pressed: true,
            ..Default::default()
        }
        .pack()
    }

    /// Mash A on alternate ticks through whatever is showing.
    fn mash(s: &mut Session, fs: &GameFs, until: impl Fn(&Session) -> bool) {
        for t in 0..60_000 {
            if until(s) {
                return;
            }
            let i = if t % 8 == 0 {
                a_button()
            } else {
                PackedInput::default()
            };
            s.update(fs, &[i, i]);
        }
        panic!("never got there: {:?}", s.phase);
    }

    fn loading(s: &Session) -> bool {
        matches!(s.phase, Phase::Loading { .. })
    }

    #[test]
    fn select_leads_into_the_tower() {
        let Some(fs) = data() else { return };
        let s = in_tower(&fs);
        assert!(matches!(s.phase, Phase::Playing));
        assert_eq!(s.game.as_ref().unwrap().wave, Some(TOWER_WAVE));
        assert!(s.progress.entered_tower);
        assert!(s.recording.is_some());
    }

    #[test]
    fn a_portal_leads_through_the_map_to_its_level() {
        let Some(fs) = data() else { return };
        let mut s = in_tower(&fs);
        end_level(&mut s, LevelEnd::Exit(Some(0x100)));
        s.update(&fs, &[]);
        assert!(matches!(s.phase, Phase::Map { .. }));
        assert!(s.game.is_none());
        mash(&mut s, &fs, loading);
        s.load(&fs).unwrap();
        assert_eq!(s.game.as_ref().unwrap().wave, Some(0x100));
        assert_eq!(s.progress.last_realm, Some(1));
    }

    #[test]
    fn a_finished_level_goes_through_the_shop_to_select() {
        let Some(fs) = data() else { return };
        let mut s = in_tower(&fs);
        s.enter_level("levelA1".into(), s.panels.clone());
        s.load(&fs).unwrap();
        let w = s.game.as_ref().unwrap().wave.unwrap();
        s.game.as_mut().unwrap().players[0].gold = 1234;
        end_level(&mut s, LevelEnd::Exit(None));
        s.update(&fs, &[]);
        assert!(matches!(
            s.phase,
            Phase::Shop {
                from_menu: false,
                ..
            }
        ));
        assert_ne!(
            s.progress.beaten[(w >> 8) as usize & 15],
            0,
            "recorded as beaten"
        );
        assert_eq!(s.carry.as_ref().unwrap()[0].as_ref().unwrap().gold, 1234);
        mash(&mut s, &fs, |s| matches!(s.phase, Phase::Select));
    }

    #[test]
    fn the_dead_go_back_to_the_tower_with_their_checkpoint() {
        let Some(fs) = data() else { return };
        let mut s = in_tower(&fs);
        s.enter_level("levelA1".into(), s.panels.clone());
        s.load(&fs).unwrap();
        let g = s.game.as_mut().unwrap();
        g.players[0].gold = 777;
        g.players[1].gold = 777;
        g.players[0].state = PlayerState::InTower;
        end_level(&mut s, LevelEnd::AllDead);
        s.update(&fs, &[]);
        assert!(loading(&s));
        let carry = s.carry.as_ref().unwrap();
        assert_eq!(
            carry[0].as_ref().unwrap().gold,
            0,
            "died: back to the start"
        );
        assert_eq!(carry[1].as_ref().unwrap().gold, 777, "lived: keeps it");
    }

    #[test]
    fn a_secret_level_comes_back_to_the_level_that_waited() {
        let Some(fs) = data() else { return };
        let mut s = in_tower(&fs);
        s.enter_level("levelA1".into(), s.panels.clone());
        s.load(&fs).unwrap();
        for _ in 0..30 {
            s.update(&fs, &[PackedInput::default(); 2]);
        }
        let frame = s.game.as_ref().unwrap().frame;
        let ticks = s.recording.as_ref().unwrap().ticks;
        end_level(&mut s, LevelEnd::Secret(0xC00));
        s.update(&fs, &[]);
        assert!(matches!(s.phase, Phase::Map { resume: false, .. }));
        mash(&mut s, &fs, loading);
        s.load(&fs).unwrap();
        s.game.as_mut().unwrap().players[0].gold = 4321;
        s.game.as_mut().unwrap().crystals[1] = 3;
        end_level(&mut s, LevelEnd::SecretOver);
        s.update(&fs, &[]);
        assert!(matches!(s.phase, Phase::Map { resume: true, .. }));
        mash(&mut s, &fs, |s| matches!(s.phase, Phase::Playing));
        let g = s.game.as_ref().unwrap();
        assert_eq!(g.frame, frame, "the very level, where it was");
        assert_eq!(g.players[0].gold, 4321, "with what was gained there");
        assert_eq!(g.crystals[1], 3);
        assert_eq!(g.level_end, None);
        assert_eq!(
            s.recording.as_ref().unwrap().ticks,
            ticks,
            "its recording too"
        );
        s.update(&fs, &[PackedInput::default(); 2]);
        assert_eq!(s.game.as_ref().unwrap().frame, frame + 1, "and it goes on");
    }

    #[test]
    fn the_tower_shop_comes_back_to_the_tower() {
        let Some(fs) = data() else { return };
        let mut s = in_tower(&fs);
        s.command(&fs, Decision::Pause(0));
        s.command(&fs, Decision::OpenShop(ShopMode::FromMenu));
        assert!(matches!(
            s.phase,
            Phase::Shop {
                from_menu: true,
                ..
            }
        ));
        // The exit is the last row: B jumps there, A takes it.
        let b = PlayerInput {
            b_pressed: true,
            ..Default::default()
        }
        .pack();
        for t in 0..20_000 {
            if matches!(s.phase, Phase::Playing) {
                break;
            }
            let i = match t % 16 {
                0 => a_button(),
                8 => b,
                _ => PackedInput::default(),
            };
            s.update(&fs, &[i, i]);
        }
        assert!(matches!(s.phase, Phase::Playing), "{:?}", s.phase);
        assert_eq!(s.paused, None);
        assert_eq!(s.game.as_ref().unwrap().wave, Some(TOWER_WAVE));
    }

    #[test]
    fn a_pause_stops_the_level_and_its_recording() {
        let Some(fs) = data() else { return };
        let mut s = in_tower(&fs);
        let frame = s.game.as_ref().unwrap().frame;
        s.command(&fs, Decision::Pause(1));
        s.update(&fs, &[PackedInput::default(); 2]);
        assert_eq!(s.game.as_ref().unwrap().frame, frame);
        assert_eq!(s.recording.as_ref().unwrap().ticks, 0);
        s.command(&fs, Decision::Unpause);
        s.update(&fs, &[PackedInput::default(); 2]);
        assert_eq!(s.recording.as_ref().unwrap().ticks, 1);
    }

    #[test]
    fn a_quitter_leaves_and_when_all_have_it_is_over() {
        let Some(fs) = data() else { return };
        let mut s = in_tower(&fs);
        s.game.as_mut().unwrap().players[0].quit = true;
        end_level(&mut s, LevelEnd::AllDead);
        s.update(&fs, &[]);
        assert_eq!(s.panels.len(), 1);
        assert_eq!(s.panels[0].slot, 1);
        assert!(s.carry.as_ref().unwrap()[0].is_none());
        assert!(s
            .events
            .iter()
            .any(|e| matches!(e, Event::PlayersLeft(v) if v == &[0])));
        s.load(&fs).unwrap();
        s.game.as_mut().unwrap().players[0].quit = true;
        end_level(&mut s, LevelEnd::AllDead);
        s.update(&fs, &[]);
        assert!(matches!(s.phase, Phase::Over));
    }

    #[test]
    fn manage_party_keeps_what_the_heroes_have() {
        let Some(fs) = data() else { return };
        let mut s = in_tower(&fs);
        s.game.as_mut().unwrap().players[1].gold = 55;
        s.command(&fs, Decision::ManageParty);
        assert!(matches!(s.phase, Phase::Select));
        assert!(s.game.is_none());
        assert_eq!(s.carry.as_ref().unwrap()[1].as_ref().unwrap().gold, 55);
        assert!(s.heroes[0].is_some(), "banked");
    }

    #[test]
    fn a_movie_ends_on_a_new_press_or_when_it_runs_out() {
        let idle = [PlayerInput::default()];
        let press = [PlayerInput {
            attack_pressed: true,
            ..Default::default()
        }];
        let mut m = MovieClock::new(30, 30.0);
        let t = (1..500)
            .find(|_| m.update(crate::game::TICK, &idle))
            .unwrap();
        assert!((59..=61).contains(&t), "{t}");
        let mut m = MovieClock::new(3000, 30.0);
        m.update(crate::game::TICK, &idle);
        let t = (0..20)
            .find(|_| m.update(crate::game::TICK, &press))
            .unwrap();
        assert!(t <= 8, "{t}");
    }

    /// The whole road, twice: the same inputs and decisions make the same
    /// session at every tick, through every phase.
    fn the_road(fs: &GameFs) -> Vec<u64> {
        let mut digests = Vec::new();
        let mut s = in_tower(fs);
        let step = |s: &mut Session, digests: &mut Vec<u64>, n: u32, a: bool| {
            for t in 0..n {
                let inputs: Vec<PackedInput> = match &s.game {
                    Some(g) => (0..g.players.len())
                        .map(|i| g.bot_input(i).pack())
                        .collect(),
                    None if a && t % 8 == 0 => vec![a_button(); 2],
                    None => vec![PackedInput::default(); 2],
                };
                s.update(fs, &inputs);
                if t % 20 == 0 {
                    digests.push(s.digest());
                }
            }
        };
        step(&mut s, &mut digests, 100, false);
        s.command(fs, Decision::Game(Command::DebugGod(true)));
        end_level(&mut s, LevelEnd::Exit(Some(0x100)));
        while !loading(&s) {
            step(&mut s, &mut digests, 20, true);
        }
        s.load(fs).unwrap();
        step(&mut s, &mut digests, 300, false);
        end_level(&mut s, LevelEnd::Exit(None));
        while !matches!(s.phase, Phase::Select) {
            step(&mut s, &mut digests, 20, true);
        }
        digests.push(s.digest());
        digests
    }

    #[test]
    fn two_sessions_walk_the_same_road() {
        let Some(fs) = data() else { return };
        let (a, b) = (the_road(&fs), the_road(&fs));
        assert!(a.len() > 30, "{}", a.len());
        assert_eq!(a, b);
        assert!(a.windows(2).any(|w| w[0] != w[1]), "the digest moves");
    }

    #[test]
    fn a_session_can_be_cloned_mid_shop() {
        let Some(fs) = data() else { return };
        let mut s = in_tower(&fs);
        s.command(&fs, Decision::OpenShop(ShopMode::FromMenu));
        for _ in 0..30 {
            s.update(&fs, &[PackedInput::default(); 2]);
        }
        let mut c = s.clone();
        for _ in 0..30 {
            s.update(&fs, &[a_button(); 2]);
            c.update(&fs, &[a_button(); 2]);
        }
        assert_eq!(s.digest(), c.digest());
    }

    /// The recordings of the levels that have gone.
    fn gone(s: &mut Session) -> Vec<(Box<Game>, Box<Replay>)> {
        std::mem::take(&mut s.events)
            .into_iter()
            .filter_map(|e| match e {
                Event::LevelGone {
                    game,
                    recording: Some(r),
                } => Some((game, r)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_level_that_ended_replays_to_the_bit() {
        let Some(fs) = data() else { return };
        let mut s = in_tower(&fs);
        s.enter_level("levelA1".into(), s.panels.clone());
        s.load(&fs).unwrap();
        s.events.clear();
        for _ in 0..90 {
            s.update(&fs, &[a_button(); 2]);
        }
        s.command(&fs, Decision::Game(Command::QuitLevel));
        s.update(&fs, &[]);
        let gone = gone(&mut s);
        assert_eq!(gone.len(), 1);
        let (game, rec) = &gone[0];
        let compat = crate::compat::Compat::local(&fs, &Rules::classic(), "test").unwrap();
        let back = Replay::from_text(&rec.to_text(game, &compat)).unwrap();
        let v = back.check(&fs, None).unwrap();
        assert!(v.end_differs.is_empty(), "{:?}", v.end_differs);
        assert!(v.diverged.is_none());
    }

    #[test]
    fn a_level_left_waiting_still_keeps_its_recording() {
        let Some(fs) = data() else { return };
        let mut s = in_tower(&fs);
        s.enter_level("levelA1".into(), s.panels.clone());
        s.load(&fs).unwrap();
        end_level(&mut s, LevelEnd::Secret(0xC00));
        s.update(&fs, &[]);
        mash(&mut s, &fs, loading);
        s.load(&fs).unwrap();
        assert!(s.has_waiting());
        s.events.clear();
        s.command(&fs, Decision::QuitGame);
        assert!(!s.has_waiting());
        let names: Vec<_> = gone(&mut s)
            .iter()
            .map(|(g, _)| g.level.name.clone())
            .collect();
        assert_eq!(names, ["levelA1", "levelS1"]);
    }

    /// Someone gone from a network game: their hero leaves the level at once,
    /// and the party when the level ends, and the game goes on without them.
    #[test]
    fn a_player_who_has_left_is_out_of_the_level_and_then_the_party() {
        let Some(fs) = data() else { return };
        let mut s = in_tower(&fs);
        s.events.clear();
        let idle = [PackedInput::default(); 2];
        s.player_left(&fs, 1);
        let g = s.game.as_deref().unwrap();
        assert_eq!(g.players[1].state, PlayerState::InTower);
        assert!(g.players[1].quit);
        assert!(g.players[0].state.active());
        for _ in 0..30 {
            s.update(&fs, &idle);
        }
        // The other hero keeps the level going.
        assert!(s.game.as_deref().unwrap().level_end.is_none());
        s.command(&fs, Decision::Game(Command::QuitLevel));
        s.update(&fs, &idle);
        assert_eq!(s.panels.len(), 1);
        assert_eq!(s.panels[0].slot, 0);
        assert!(s.carry.as_ref().unwrap()[1].is_none());
        assert!(s
            .events
            .iter()
            .any(|e| matches!(e, Event::PlayersLeft(l) if l == &[1])));
    }

    /// Someone who goes between levels is out of the next one from its start.
    #[test]
    fn a_player_who_left_between_levels_is_not_in_the_next_one() {
        let Some(fs) = data() else { return };
        let mut s = in_tower(&fs);
        s.warp(TOWER.to_string(), Vec::new);
        assert!(matches!(s.phase, Phase::Loading { .. }));
        s.player_left(&fs, 0);
        s.load(&fs).unwrap();
        let g = s.game.as_deref().unwrap();
        assert_eq!(g.players[0].state, PlayerState::InTower);
        assert!(g.players[1].state.active());
    }

    /// Left in a level, it is in that level's recording, which still replays.
    #[test]
    fn a_game_someone_left_replays_to_the_bit() {
        let Some(fs) = data() else { return };
        let mut s = in_tower(&fs);
        let idle = [PackedInput::default(); 2];
        for _ in 0..30 {
            s.update(&fs, &idle);
        }
        s.player_left(&fs, 1);
        for _ in 0..30 {
            s.update(&fs, &idle);
        }
        let rec = s.recording.clone().unwrap();
        let g = s.game.as_deref().unwrap();
        let compat = crate::compat::Compat::local(&fs, &s.rules(), "test").unwrap();
        let back = Replay::from_text(&rec.to_text(g, &compat)).unwrap();
        assert!(back.to_text(g, &compat).contains(" left 1"));
        let verdict = back.check(&fs, Some(&compat)).unwrap();
        assert!(verdict.matches(), "{verdict:?}");
    }

    /// Two sessions fed the same have the same fingerprint, and it sees more
    /// than the level.
    #[test]
    fn the_fingerprint_sees_the_session_and_not_only_the_level() {
        let Some(fs) = data() else { return };
        let mut a = in_tower(&fs);
        let mut b = in_tower(&fs);
        assert_eq!(a.fingerprint(), b.fingerprint());
        let idle = [PackedInput::default(); 2];
        a.update(&fs, &idle);
        assert_ne!(a.fingerprint(), b.fingerprint());
        b.update(&fs, &idle);
        assert_eq!(a.fingerprint(), b.fingerprint());
        b.paused = Some(1);
        assert_ne!(a.fingerprint(), b.fingerprint());
    }

    #[test]
    fn checkpoints_skip_two_waves_and_the_secret_levels() {
        assert!(checkpoint_at(Some(0x500)));
        assert!(checkpoint_at(Some(0x502)));
        assert!(!checkpoint_at(Some(0x501)));
        assert!(!checkpoint_at(Some(0x601)));
        assert!(!checkpoint_at(Some(0xC03)));
        assert!(checkpoint_at(Some(0xD00)));
        assert!(checkpoint_at(None));
    }
}
