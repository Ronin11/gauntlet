//! Windowed game: winit event loop, wgpu surface, the fixed-timestep loop, and the
//! screens. What happens in a game is the session's (gdl_sim::session); this draws it, sounds
//! it, and turns what players choose in the menus into its decisions.

use crate::attract::{self, Attract};
use crate::audio::Audio;
use crate::compat::Compat;
use crate::fs::GameFs;
use crate::game::setup::Command;
use crate::game::Game;
use crate::game::PlayerState;
use crate::gpu::Gpu;
use crate::hud::PanelInfo;
use crate::hud::{self, Hud};
use crate::input::{Input, PlayerInput, Source};
use crate::menu::{self, Title};
use crate::options::{Options, OptionsAction, Prefs};
use crate::progress::Progress;
use crate::replay::{Playback, Replay};
use crate::save::MemCard;
use crate::savestate::{self, SaveState};
use crate::scene::{Scene, CLEAR};
use crate::screen::Scr;
use crate::select::{Select, CLASSES, COLOURS};
use crate::session::{Carry, Chosen, Decision, Event, Heroes, Phase, Session};
use crate::ui::UiAssets;
use anyhow::{Context, Result};
use gdl_formats::wdata::TOWER_WAVE;
use gdl_net::lobby::Roster;
use std::sync::Arc;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Fullscreen, Window, WindowId};

const TICK: f32 = 1.0 / 60.0;

pub struct Config {
    pub fs: GameFs,
    pub level: String,
    /// Players chosen on the command line; empty shows the title and join screens.
    pub players: Vec<(String, String)>,
    /// A loyal follower's class and colour (`--companion`).
    pub companion: Option<(String, String)>,
    pub mute: bool,
    /// Where the memory card's save file is kept (None: saves last only
    /// for this run).
    pub save_dir: Option<std::path::PathBuf>,
    /// A debug save state to start in (`--load-state`: a slot or a file).
    pub load_state: Option<String>,
    /// The rules the games play by (classic unless asked otherwise).
    pub rules: crate::rules::Rules,
    /// A recording to watch instead of playing (`--replay`).
    pub replay: Option<Replay>,
    /// Keep each level's recording in this folder (`--record`).
    pub record: Option<std::path::PathBuf>,
    /// A network game, agreed in the lobby (`--host`, `--join`).
    pub net: Option<Roster>,
}

/// How to find a network game.
#[derive(Clone, Debug)]
pub enum NetSpec {
    Host { port: u16 },
    Join { addr: String },
}

pub struct NetLobby<'a> {
    pub spec: &'a NetSpec,
    pub players: usize,
    pub delay: Option<usize>,
}

/// Wait for the other players, before there is a window. They must have this
/// same copy of the game.
pub fn find_players(
    lobby: &NetLobby,
    fs: &GameFs,
    rules: &crate::rules::Rules,
    save_dir: Option<&std::path::Path>,
) -> Result<Roster> {
    let compat = Compat::local(fs, rules, crate::bugreport::BUILD)?;
    let mut prefs = Prefs::default();
    if let Some(o) = MemCard::open(save_dir).opts() {
        prefs.apply_game_opts(&o);
    }
    let say = |line: &str| println!("{line}");
    match lobby.spec {
        NetSpec::Host { port } => gdl_net::lobby::host(
            &gdl_net::lobby::Host {
                port: *port,
                players: lobby.players,
                delay: lobby.delay,
                difficulty: prefs.difficulty,
                friendly_fire: prefs.multiplayer,
                compat,
                controls: prefs.players[0],
            },
            say,
        ),
        NetSpec::Join { addr } => gdl_net::lobby::join(addr, &compat, &prefs.players[0], say),
    }
}

/// How long something said about the network stays on the screen, in seconds.
const NET_NOTE_FOR: f32 = 5.0;

/// A network game going on.
struct NetPlay {
    net: gdl_net::Net,
    /// This machine's own options and memory card, kept aside: the game goes by
    /// what everyone agreed, and writes nothing to the card.
    own_prefs: Prefs,
    own_card: MemCard,
    /// Esc, to hand to the network as this player's Start.
    escape: bool,
    /// Something to say on the screen, and until when.
    note: Option<(String, Instant)>,
    /// A fingerprint has differed: this game can't be trusted any more.
    desync: bool,
}

/// The agreed options, with this machine's own for what is only its own.
fn agreed_prefs(own: &Prefs, roster: &Roster) -> Prefs {
    let mut p = Prefs::default();
    p.music_vol = own.music_vol;
    p.sfx_vol = own.sfx_vol;
    p.stereo = own.stereo;
    p.compass = own.compass;
    p.difficulty = roster.difficulty;
    p.multiplayer = roster.friendly_fire;
    for (slot, c) in roster.controls.iter().enumerate().take(4) {
        p.players[slot] = *c;
    }
    p
}

/// Into player select with everyone, as every machine does at the same moment.
fn start_net(r: &mut Running, roster: &Roster) -> Result<Screen> {
    let net = gdl_net::Net::start(roster)?;
    let n = roster.addrs.len();
    r.input.start_net(roster.me, n);
    let own_prefs = r.prefs.clone();
    r.prefs = agreed_prefs(&own_prefs, roster);
    let own_card = std::mem::replace(&mut r.card, MemCard::in_memory());
    r.sources = (0..n).map(Source::Net).collect();
    r.session = Session::new(
        r.session.rules(),
        r.prefs.difficulty,
        r.prefs.multiplayer,
        Progress::default(),
    );
    r.session.phase = Phase::Select;
    let s = &r.session;
    let party = party_for(&r.card, &None, &s.heroes, &s.progress, &r.prefs);
    let mut sel = Select::with_first(Source::Net(0)).with_party(party);
    for i in 1..n {
        sel.join(Source::Net(i));
    }
    r.net = Some(NetPlay {
        net,
        own_prefs,
        own_card,
        escape: false,
        note: None,
        desync: false,
    });
    Ok(Screen::Select(sel))
}

/// Back to this machine alone, with its own options and card.
fn end_net(r: &mut Running) {
    if let Some(mut np) = r.net.take() {
        // What this player set for their own ears stays.
        np.own_prefs.music_vol = r.prefs.music_vol;
        np.own_prefs.sfx_vol = r.prefs.sfx_vol;
        np.own_prefs.stereo = r.prefs.stereo;
        np.own_prefs.compass = r.prefs.compass;
        r.prefs = np.own_prefs;
        r.card = np.own_card;
        r.input.end_net();
        r.sources.clear();
    }
}

/// The next tick of a network game, if everyone's input for it is here.
fn net_tick(r: &mut Running, fs: &GameFs) -> Result<bool> {
    let Some(np) = r.net.as_mut() else {
        return Ok(true);
    };
    if np.net.wants_input() {
        let local = r.input.take_local(std::mem::take(&mut np.escape));
        let session = &r.session;
        np.net.give(local, || session.fingerprint())?;
    }
    let Some(frame) = np.net.tick()? else {
        return Ok(false);
    };
    r.input.set_net_inputs(&frame.inputs);
    for player in frame.left {
        player_gone(r, fs, player);
    }
    Ok(true)
}

/// This player has gone, as of this tick on every machine.
fn player_gone(r: &mut Running, fs: &GameFs, player: usize) {
    let src = Source::Net(player);
    let index = r.sources.iter().position(|s| *s == src);
    if let Some(i) = index {
        if let Screen::Playing { pause, .. } = &mut r.screen {
            if pause.as_ref().is_some_and(|m| menu_owner(m) == i) {
                if pause.as_ref().and_then(|m| m.hints()).is_some() {
                    r.session.command(fs, Decision::Game(Command::HintsClosed));
                }
                *pause = None;
            }
        }
        if let Some(slot) = r.session.panels.get(i).map(|p| p.slot) {
            r.session.player_left(fs, slot);
        }
    }
    if let Screen::Select(sel) = &mut r.screen {
        sel.drop_source(src);
    }
    if let Some(np) = &mut r.net {
        np.note = Some((format!("Player {} has left", player + 1), Instant::now()));
    }
}

/// What the network has to say.
fn net_events(r: &mut Running, fs: &GameFs) {
    let Some(np) = r.net.as_mut() else {
        return;
    };
    np.net.poll();
    let mut lost = None;
    for e in np.net.events() {
        log::info!("network: {e:?}");
        let note = match e {
            gdl_net::NetEvent::Connected => None,
            gdl_net::NetEvent::Quiet { player } => {
                Some(format!("Nothing from player {}...", player + 1))
            }
            gdl_net::NetEvent::Back { player } => Some(format!("Player {} is back", player + 1)),
            gdl_net::NetEvent::Desync { player, frame } => {
                log::error!("network: out of sync with player {player} at tick {frame}");
                np.desync = true;
                None
            }
            gdl_net::NetEvent::Lost { player } => {
                lost = Some(player);
                None
            }
        };
        if let Some(n) = note {
            np.note = Some((n, Instant::now()));
        }
    }
    if let Some(player) = lost {
        // The game can't go on safely, here or on any machine still in it.
        log::error!("network: lost player {player}: the game is over");
        end_net(r);
        r.session.command(fs, Decision::QuitGame);
    }
}

/// A line about the network at the top of the screen.
fn draw_net_status(s: &mut Scr, np: &NetPlay) {
    let line = if np.desync {
        "OUT OF SYNC".to_string()
    } else if !np.net.connected() {
        "Connecting to the other players...".to_string()
    } else if np.net.waiting_for().as_secs_f32() > 0.5 {
        "Waiting for the other players...".to_string()
    } else {
        match &np.note {
            Some((n, at)) if at.elapsed().as_secs_f32() < NET_NOTE_FOR => n.clone(),
            _ => return,
        }
    };
    draw_net_line(s, &line);
}

/// A line of news about the network, centred at the top of the screen.
pub fn draw_net_line(s: &mut Scr, line: &str) {
    let w = s.text_width(crate::ui::FONT_8X8, 1.0, line);
    let x = 256.0 - (w * 0.5).floor();
    s.text(1.0, x, 12.0, crate::ui::FONT_8X8, 0xFFFFFF, 0, line);
}

/// What the engine keeps for the level the session is running.
struct Playing {
    scene: Scene,
    /// Seconds played, for fading out the loading art.
    intro: f32,
    /// The boss's health-meter textures (boss levels).
    boss_bank: Option<crate::hud::TexBank>,
}

/// An attract-mode flyby's level, which is no part of any session.
struct FlybyPlay {
    game: Game,
    scene: Scene,
}

/// A recording being watched (`--replay`).
struct Watch {
    replay: Replay,
    playback: Playback,
    /// This copy of the game, to say how it differs from the recorder's.
    ours: Option<Compat>,
    done: bool,
}

impl Watch {
    fn step(&mut self, g: &mut Game) {
        if self.done || self.playback.step(&self.replay, g) {
            return;
        }
        self.done = true;
        let verdict = self.replay.verdict(g, &self.playback, self.ours.as_ref());
        // The commands' last tick has run: say what was seen, once.
        println!("{verdict}");
        println!("the replay is over; Esc quits");
    }
}

/// This copy of the game, as a recording's header says it (slow: once).
fn local_compat(r: &mut Running, fs: &GameFs, rules: &crate::rules::Rules) -> Result<Compat> {
    if let Some(c) = &r.compat {
        return Ok(c.clone());
    }
    let c = Compat::local(fs, rules, crate::bugreport::BUILD)?;
    r.compat = Some(c.clone());
    Ok(c)
}

/// Keep a level's recording in the `--record` folder.
fn write_recording(r: &mut Running, fs: &GameFs, game: &Game, rec: &Replay) {
    let Some(dir) = r.record_dir.clone() else {
        return;
    };
    let rules = r.session.rules();
    let compat = match local_compat(r, fs, &rules) {
        Ok(c) => c,
        Err(e) => {
            log::warn!("recording: {e:#}");
            return;
        }
    };
    let name = crate::bugreport::folder_name(crate::bugreport::unix_now(), &game.level.name);
    let path = dir.join(format!("{name}.replay"));
    match rec.write(&path, game, &compat) {
        Ok(()) => log::info!("recording kept in {}", path.display()),
        Err(e) => log::warn!("recording: {e:#}"),
    }
}

/// What each hero presses this tick: their controller, or the follower's mind.
fn party_inputs(
    input: &Input,
    sources: &[Source],
    game: Option<&Game>,
    brain: &mut Option<crate::runner::companion::Companion>,
) -> Vec<PlayerInput> {
    use crate::runner::companion::Companion;
    let mut inputs: Vec<PlayerInput> = sources.iter().map(|s| input.read(*s)).collect();
    let follower = sources.iter().position(|s| *s == Source::Companion);
    if let (Some(pi), Some(g)) = (follower, game) {
        if !brain.as_ref().is_some_and(|c| c.is_for(g, pi)) {
            *brain = Some(Companion::new(g, pi));
        }
        if let Some(c) = brain {
            inputs[pi] = c.input(g);
        }
    }
    inputs
}

/// The screen being left takes its music and textures with it.
fn leave_screen(r: &mut Running) {
    match &mut r.screen {
        Screen::Map { map } => {
            r.audio.stop();
            map.release(&mut r.hud);
        }
        Screen::Shop { bank } => {
            r.audio.stop();
            r.hud.free_bank(std::mem::take(bank));
        }
        Screen::Intro { .. } => r.audio.stop(),
        _ => {}
    }
}

/// What the session has done since we last looked: the screens, music and
/// textures that follow from it. Returns the next screen, if it changes.
fn session_events(r: &mut Running, fs: &GameFs) -> Option<Screen> {
    let mut next = None;
    for ev in std::mem::take(&mut r.session.events) {
        match ev {
            Event::LevelGone { game, recording } => {
                if let Some(rec) = &recording {
                    write_recording(r, fs, &game, rec);
                }
                if let Screen::Playing { play, .. } = &mut r.screen {
                    if let Some(bank) = play.boss_bank.take() {
                        r.hud.free_bank(bank);
                    }
                }
                // Nothing waits any more: nor does what we kept to draw it.
                if !r.session.has_waiting() {
                    if let Some(bank) = r.suspended.take().and_then(|mut p| p.boss_bank.take()) {
                        r.hud.free_bank(bank);
                    }
                }
            }
            Event::LevelSuspended => {
                if matches!(r.screen, Screen::Playing { .. }) {
                    if let Screen::Playing { play, .. } =
                        std::mem::replace(&mut r.screen, Screen::Legal(0))
                    {
                        r.suspended = Some(play);
                    }
                }
            }
            Event::Resumed { from_shop } => {
                leave_screen(r);
                if let Some(play) = r.suspended.take() {
                    let streams = if from_shop {
                        vec!["tower".to_string()]
                    } else {
                        r.session
                            .game
                            .as_ref()
                            .and_then(|g| g.wave.map(|w| g.worlds.music(w)))
                            .unwrap_or_default()
                    };
                    r.audio.play_streams(fs, &streams);
                    r.last = Instant::now();
                    next = Some(Screen::Playing { play, pause: None });
                }
            }
            Event::PlayersLeft(left) => {
                let mut i = 0;
                r.sources.retain(|_| {
                    i += 1;
                    !left.contains(&(i - 1))
                });
                // The follower doesn't play on alone.
                if r.sources.iter().all(|s| *s == Source::Companion) {
                    r.session.command(fs, Decision::QuitGame);
                }
            }
            Event::Select => {
                leave_screen(r);
                let s = &r.session;
                let party = party_for(&r.card, &s.carry, &s.heroes, &s.progress, &r.prefs);
                let mut sel = Select::from_game(&s.panels, &r.sources)
                    .with_party(party)
                    .companion_ready();
                // Nobody waits for someone who has gone.
                if let Some(np) = &r.net {
                    for src in r.sources.iter().copied() {
                        if matches!(src, Source::Net(p) if np.net.is_gone(p)) {
                            sel.drop_source(src);
                        }
                    }
                }
                // check_active_players: a new pad joined in the tower.
                if let Some(src) = r.joining.take() {
                    sel.join(src);
                }
                next = Some(Screen::Select(sel));
            }
            Event::Loading => {
                leave_screen(r);
                next = Some(Screen::Loading { shown: false });
            }
            Event::Map { .. } => {
                leave_screen(r);
                if let Phase::Map { clock, .. } = &r.session.phase {
                    let map = crate::mapscreen::MapScreen::new(fs, &r.gpu, &mut r.hud, clock);
                    match &clock.music {
                        Some(m) => r.audio.play_bank_music(fs, m),
                        None => r.audio.stop(),
                    }
                    next = Some(Screen::Map { map: Box::new(map) });
                }
            }
            Event::Movie { name } => {
                leave_screen(r);
                r.audio.stop();
                let mut movie = crate::movie::Movie::open(fs, &name).ok();
                if let Some(a) = movie.as_mut().and_then(|m| m.audio.take()) {
                    r.audio.play_once(a.pcm, a.channels as u16, a.playback_rate);
                }
                next = Some(Screen::Intro {
                    movie: movie.map(Box::new),
                });
            }
            Event::Shop => {
                leave_screen(r);
                if let Phase::Shop { shop, .. } = &r.session.phase {
                    match shop.music() {
                        Some(m) => r.audio.play_bank_music(fs, &m),
                        None => r.audio.stop(),
                    }
                    next = Some(Screen::Shop {
                        bank: crate::shop::load_icons(fs, &r.gpu, &mut r.hud, shop),
                    });
                }
            }
            Event::GameOver => {
                leave_screen(r);
                r.audio.stop();
                next = Some(game_over_screen(fs));
            }
            // Sumner's lectern: HintMenu.
            Event::HintMenu(pi) => {
                if let (Screen::Playing { pause, .. }, Some(g)) =
                    (&mut r.screen, r.session.game.as_deref())
                {
                    let hints = crate::hints::Hints::new(
                        r.session.progress.hints,
                        hint_facts(&r.session.progress, g),
                        fs.read("TEXT/HINTS_E.ROM")
                            .ok()
                            .and_then(|d| gdl_formats::text::TextRom::parse(&d).ok()),
                    );
                    *pause = Some(Options::hint_menu(pi, hints));
                }
            }
        }
    }
    next
}

/// padUpdate's gPadResetHoldTimer: a pad holding exactly Start + X + B
/// (PAD_RESET_COMBO_B, 0x1600) counts up; past 500 ms sysPollResetButton
/// fires and the system resets (OSResetSystem).
const RESET_HOLD_MS: f32 = 500.0;

/// Start + X + B with no other button (stick moves don't count).
fn reset_combo(i: &PlayerInput) -> bool {
    i.start
        && i.magic
        && i.turbo
        && !i.attack
        && !i.slow
        && !i.strafe
        && !i.combo
        && i.dpad == glam::Vec2::ZERO
}

/// Advance each source's reset hold timer by one tick; true when one is past
/// [`RESET_HOLD_MS`].
fn reset_held(timers: &mut Vec<(Source, f32)>, all: &[(Source, PlayerInput)]) -> bool {
    timers.retain(|(s, _)| all.iter().any(|(a, _)| a == s));
    let mut fire = false;
    for (src, inp) in all {
        let t = match timers.iter_mut().find(|(s, _)| s == src) {
            Some((_, t)) => t,
            None => {
                timers.push((*src, 0.0));
                &mut timers.last_mut().unwrap().1
            }
        };
        *t = if reset_combo(inp) {
            *t + TICK * 1000.0
        } else {
            0.0
        };
        fire |= *t > RESET_HOLD_MS;
    }
    fire
}

/// PlayerControl[].levels: the buttons a source holds (and the stick and
/// d-pad directions), as ScreenSaver compares them from tick to tick.
fn held_levels(i: &PlayerInput) -> u32 {
    let bits = [
        i.attack,
        i.turbo,
        i.slow,
        i.magic,
        i.strafe,
        i.combo,
        i.start,
        i.stick.x > 0.5,
        i.stick.x < -0.5,
        i.stick.y > 0.5,
        i.stick.y < -0.5,
        i.dpad.x > 0.5,
        i.dpad.x < -0.5,
        i.dpad.y > 0.5,
        i.dpad.y < -0.5,
    ];
    bits.iter()
        .enumerate()
        .fold(0, |m, (k, &b)| m | (u32::from(b) << k))
}

/// ScreenSaver counts idle time except in the attract loop (title screen
/// included), the movies (MG_GAMEMOVIE) and the ending.
fn saver_counts(screen: &Screen) -> bool {
    matches!(
        screen,
        Screen::Select(_)
            | Screen::Loading { .. }
            | Screen::Map { .. }
            | Screen::Shop { .. }
            | Screen::Playing { .. }
            | Screen::GameOver { .. }
    )
}

/// OptionsStart: "Quit Level" is greyed out (value -1) while
/// sMusicTrackHi == 0xC.
fn quit_level_disabled(wave: Option<u32>) -> bool {
    wave.is_some_and(|w| w >> 8 == 0xC)
}

/// What player select starts with: the card, each column's carried state and
/// save image, and the party's progress.
fn party_for(
    card: &MemCard,
    carry: &Option<Carry>,
    heroes: &Heroes,
    progress: &Progress,
    prefs: &Prefs,
) -> crate::select::Party {
    let mut card = card.clone();
    // saveSave writes the current gameOpts with every hero.
    card.set_opts(prefs.game_opts());
    crate::select::Party {
        card,
        carry: carry.clone().unwrap_or_default(),
        heroes: heroes.clone(),
        progress: progress.clone(),
        controls: prefs.players,
    }
}

enum Screen {
    /// The licence screen shown at power-on, for this many more ticks.
    Legal(u32),
    /// The attract loop (movies, group shots); its state is `Running::attract`.
    Attract,
    /// An attract-mode level flyby (MA_FLYBY): the level with no heroes, seen
    /// from the flyby camera, for this many ticks so far.
    Flyby {
        play: Box<FlybyPlay>,
        cam: Box<crate::flyby::FlybyCam>,
        ticks: u32,
    },
    /// MG_OVER: "GAME OVER" typed out for 240 ticks, then the attract loop.
    GameOver {
        ticks: i32,
        text: String,
    },
    Title(Title),
    Select(Select),
    /// The loading art is shown for one frame, then the level the session
    /// names loads.
    Loading {
        shown: bool,
    },
    /// The map screen (init_mapscreen) while the next level "loads", with the
    /// session's clock.
    Map {
        map: Box<crate::mapscreen::MapScreen>,
    },
    /// The session's shop, and its icons not in the front-end banks.
    Shop {
        bank: crate::hud::TexBank,
    },
    /// A movie the session is running (init_gamemovie, or "garm", "victory").
    Intro {
        movie: Option<Box<crate::movie::Movie>>,
    },
    Playing {
        play: Box<Playing>,
        /// The Game Menu / Tower Menu, while open (the game is frozen).
        pause: Option<Options>,
    },
}

// Field order is drop order: the surface must go before the window it draws to.
struct Running {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    gpu: Gpu,
    depth: wgpu::TextureView,
    screen: Screen,
    /// The attract sequence, kept across visits to the title screen.
    attract: Attract,
    /// The level music's area playing, the one being faded to, and the fade.
    music_now: usize,
    music_switch: Option<usize>,
    music_fade: f32,
    /// The party, the level and what comes next.
    session: Session,
    /// A level waiting in the session for the party to come back: what we keep
    /// to draw it.
    suspended: Option<Box<Playing>>,
    /// A new pad that has asked to join in the tower.
    joining: Option<Source>,
    /// The loyal follower's brain for this level, and its ticks in a shop.
    companion: Option<crate::runner::companion::Companion>,
    companion_ticks: u32,
    /// A recording being watched instead of played.
    watch: Option<Watch>,
    /// Where `--record` keeps recordings.
    record_dir: Option<std::path::PathBuf>,
    /// This copy of the game, once something has asked (it reads every file).
    compat: Option<Compat>,
    /// Options settings (volumes, difficulty, controls).
    prefs: Prefs,
    /// The memory card in Slot A.
    card: MemCard,
    /// The current movie frame's texture.
    movie_tex: Option<crate::ui::Tex>,
    /// The menus' 3D arrow.
    cursor: Option<crate::cursor::Cursor>,
    hud: Hud,
    audio: Audio,
    input: Input,
    /// Each player's input source, in player order.
    sources: Vec<Source>,
    /// Escape was pressed this frame.
    escape: bool,
    /// Each input source's reset-combo hold (ms).
    reset_hold: Vec<(Source, f32)>,
    /// ScreenSaver: ticks with no change in the held buttons (lbl_80344A48),
    /// each source's held buttons last tick, the running saver and its models.
    idle: u32,
    idle_levels: Vec<(Source, u32)>,
    saver: Option<crate::screensaver::Saver>,
    saver_models: Option<crate::screensaver::SaverModels>,
    /// The developer debug menu (F1 / L+R+Z+Start; not in the original).
    debug: crate::debugmenu::DebugMenu,
    /// Watches the running level for the bug reports' event log.
    bugs: crate::bugreport::Recorder,
    /// Frames drawn.
    frames: u64,
    /// Debug save states (savestate.rs): the folder they are kept under
    /// (states/ in it), and the level time of the last autosave.
    save_dir: Option<std::path::PathBuf>,
    autosaved: f32,
    last: Instant,
    acc: f32,
    window: Arc<Window>,
    net: Option<NetPlay>,
}

struct App {
    cfg: Config,
    started: Instant,
    run: Option<Running>,
    error: Option<anyhow::Error>,
}

impl App {
    fn init(&mut self, el: &ActiveEventLoop) -> Result<Running> {
        let window = Arc::new(
            el.create_window(
                Window::default_attributes()
                    .with_title("Gauntlet Dark Legacy (gauntlet-rs)")
                    .with_inner_size(winit::dpi::LogicalSize::new(1280, 720)),
            )?,
        );
        // Vulkan/Metal/DX12 on desktop; GL only where nothing else exists (Android emulators).
        let backends = if cfg!(target_os = "android") {
            wgpu::Backends::all()
        } else {
            wgpu::Backends::PRIMARY
        };
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::from_env().unwrap_or(backends),
            ..Default::default()
        });
        let surface = instance.create_surface(window.clone())?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))?;
        log::info!("adapter: {:?}", adapter.get_info());
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))?;
        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .context("surface not supported by adapter")?;
        // Lighting is done in gamma space like the original hardware, so prefer a
        // non-sRGB swapchain format.
        let caps = surface.get_capabilities(&adapter);
        if let Some(f) = caps.formats.iter().find(|f| !f.is_srgb()) {
            config.format = *f;
        }
        config.present_mode = wgpu::PresentMode::AutoVsync;
        surface.configure(&device, &config);
        let gpu = Gpu::new(device, queue, config.format);
        let cursor = crate::cursor::Cursor::load(&self.cfg.fs, &gpu)
            .map_err(|e| log::warn!("menu arrow: {e:#}"))
            .ok();
        let depth = gpu.depth_view(config.width, config.height);

        let hud = Hud::new(
            &gpu.device,
            &gpu.queue,
            config.format,
            UiAssets::load(&self.cfg.fs),
        );
        let mut audio = Audio::new(self.cfg.mute);
        audio.load_sounds(&self.cfg.fs);
        let input = Input::new();
        let (screen, sources) = if self.cfg.players.is_empty() {
            // TODO: the original shows the licence screen for as long as its
            // start-up loading takes; two seconds stands in for that.
            (Screen::Legal(120), Vec::new())
        } else {
            let n = self.cfg.players.len();
            let mut sources: Vec<Source> = input.pads().into_iter().map(Source::Pad).collect();
            sources.truncate(n);
            // The keyboard drives the first player without a gamepad.
            if sources.len() < n {
                sources.push(Source::Keyboard);
            }
            if self.cfg.companion.is_some() {
                sources.push(Source::Companion);
            }
            (Screen::Loading { shown: false }, sources)
        };
        // init_prefs, then check_prefs_loaded: the options saved on the card.
        let card = MemCard::open(self.cfg.save_dir.as_deref());
        let mut prefs = Prefs::default();
        if let Some(o) = card.opts() {
            prefs.apply_game_opts(&o);
        }
        audio.set_volumes(&prefs);
        let mut session = Session::new(
            self.cfg.rules.clone(),
            prefs.difficulty,
            prefs.multiplayer,
            Progress::default(),
        );
        if !self.cfg.players.is_empty() {
            let mut players = self.cfg.players.clone();
            players.extend(self.cfg.companion.clone());
            session.panels = panels_for(&players);
            session.phase = Phase::Loading {
                level: self.cfg.level.clone(),
            };
        }
        let mut r = Running {
            surface,
            config,
            gpu,
            depth,
            screen,
            attract: Attract::new(),
            music_now: 0,
            music_switch: None,
            music_fade: 1.0,
            session,
            prefs,
            suspended: None,
            joining: None,
            companion: None,
            companion_ticks: 0,
            watch: None,
            record_dir: self.cfg.record.clone(),
            compat: None,
            card,
            movie_tex: None,
            cursor,
            hud,
            audio,
            input,
            sources,
            escape: false,
            reset_hold: Vec::new(),
            idle: 0,
            idle_levels: Vec::new(),
            saver: None,
            saver_models: None,
            debug: Default::default(),
            bugs: Default::default(),
            frames: 0,
            save_dir: self.cfg.save_dir.clone(),
            autosaved: 0.0,
            last: Instant::now(),
            acc: 0.0,
            window,
            net: None,
        };
        if let Some(roster) = self.cfg.net.take() {
            r.screen = start_net(&mut r, &roster)?;
            r.audio.play_select_music(&self.cfg.fs);
        }
        // --replay: straight into the recorded level.
        if let Some(replay) = self.cfg.replay.take() {
            let ours = Compat::local(
                &self.cfg.fs,
                &replay.start.setup.rules,
                crate::bugreport::BUILD,
            )
            .map_err(|e| log::warn!("--replay: {e:#}"))
            .ok();
            r.sources = vec![Source::Keyboard; replay.start.panels.len()];
            r.session.progress = replay.start.progress.clone();
            r.session.panels = replay.start.panels.clone();
            r.session.phase = Phase::Loading {
                level: replay.start.level.clone(),
            };
            r.session.start_override = Some(replay.start.clone());
            r.screen = Screen::Loading { shown: false };
            r.watch = Some(Watch {
                replay,
                playback: Playback::new(),
                ours,
                done: false,
            });
        }
        // --load-state: straight into the saved level.
        if let Some(spec) = &self.cfg.load_state {
            let path = savestate::resolve(spec, r.save_dir.as_deref());
            match SaveState::read(&path) {
                Ok(st) => {
                    log::info!("loading state {}: {}", path.display(), st.summary());
                    r.screen = begin_state(&mut r, &self.cfg.fs, st);
                }
                Err(e) => log::error!("--load-state: {e:#}"),
            }
        }
        Ok(r)
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.run.is_none() {
            match self.init(el) {
                Ok(r) => self.run = Some(r),
                Err(e) => {
                    self.error = Some(e);
                    el.exit();
                }
            }
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(r) = &mut self.run else { return };
        match event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Resized(size) => {
                r.config.width = size.width.max(1);
                r.config.height = size.height.max(1);
                r.surface.configure(&r.gpu.device, &r.config);
                r.depth = r.gpu.depth_view(r.config.width, r.config.height);
            }
            WindowEvent::KeyboardInput {
                device_id, event, ..
            } => {
                let down = event.state == ElementState::Pressed;
                match event.physical_key {
                    PhysicalKey::Code(code) => {
                        r.input.key(code, down);
                        if down && !event.repeat {
                            match code {
                                KeyCode::Escape => r.escape = true,
                                KeyCode::F11 => {
                                    let fs = r
                                        .window
                                        .fullscreen()
                                        .is_none()
                                        .then_some(Fullscreen::Borderless(None));
                                    r.window.set_fullscreen(fs);
                                }
                                _ => {}
                            }
                        }
                    }
                    // Gamepad buttons on Android arrive as unmapped key codes, per device.
                    PhysicalKey::Unidentified(winit::keyboard::NativeKeyCode::Android(code)) => {
                        use std::hash::{Hash, Hasher};
                        let mut h = std::collections::hash_map::DefaultHasher::new();
                        device_id.hash(&mut h);
                        r.input.device_key(h.finish(), code, down);
                    }
                    _ => {}
                }
            }
            // Android delivers a gamepad's stick as joystick motion, which winit
            // passes on as a touch whose position is the raw AXIS_X / AXIS_Y.
            WindowEvent::Touch(t)
                if t.phase == winit::event::TouchPhase::Moved
                    && t.location.x.abs() <= 1.0
                    && t.location.y.abs() <= 1.0 =>
            {
                use std::hash::{Hash, Hasher};
                let mut h = std::collections::hash_map::DefaultHasher::new();
                t.device_id.hash(&mut h);
                r.input
                    .device_axes(h.finish(), t.location.x as f32, -t.location.y as f32);
            }
            WindowEvent::RedrawRequested => {
                if let Err(e) = self.frame(el) {
                    self.error = Some(e);
                    el.exit();
                }
            }
            _ => {}
        }
    }

    /// Tear down GPU and audio while the window system connection is still open;
    /// dropping them after the event loop returns crashes inside the EGL/Wayland driver.
    fn exiting(&mut self, _el: &ActiveEventLoop) {
        // A recording being watched leaves nothing behind.
        if let Some(r) = self.run.as_mut().filter(|r| r.watch.is_none()) {
            autosave(r);
            if let (Some(game), Some(rec)) = (r.session.game.take(), r.session.recording.take()) {
                write_recording(r, &self.cfg.fs, &game, &rec);
            }
        }
        if let Some(r) = self.run.take() {
            let _ = r.gpu.device.poll(wgpu::PollType::Wait);
            drop(r);
        }
    }

    fn about_to_wait(&mut self, _el: &ActiveEventLoop) {
        if let Some(r) = &self.run {
            r.window.request_redraw();
        }
    }
}

impl App {
    /// Advance and draw one frame.
    fn frame(&mut self, el: &ActiveEventLoop) -> Result<()> {
        let fs = &self.cfg.fs;
        let Some(r) = &mut self.run else {
            return Ok(());
        };
        let now = Instant::now();
        r.acc += (now - r.last).as_secs_f32().min(0.25);
        r.last = now;
        let time = now.duration_since(self.started).as_secs_f32();

        // A network game ends with its game: on the title or attract screens
        // this machine is on its own; and one who is leaving goes when they may.
        let net_over = matches!(
            r.screen,
            Screen::Title(_) | Screen::Attract | Screen::Legal(_)
        ) || r.net.as_ref().is_some_and(|np| np.net.may_go());
        if net_over {
            end_net(r);
        }
        net_events(r, fs);

        for id in r.input.poll() {
            // Hot-plugged pad: give it to the keyboard's player if that is the
            // only one, otherwise ignore (players are fixed for this session).
            if matches!(r.screen, Screen::Playing { .. }) && !r.sources.contains(&Source::Pad(id)) {
                if let Some(slot) = r.sources.iter().position(|s| *s == Source::Keyboard) {
                    r.sources[slot] = Source::Pad(id);
                }
            }
        }

        // A level load blocks, so it happens on the frame after the loading art showed.
        if matches!(r.screen, Screen::Loading { shown: true }) {
            let level = match &r.session.phase {
                Phase::Loading { level } => level.clone(),
                _ => String::new(),
            };
            let state = r.session.pending_state.as_ref().map(SaveState::summary);
            if r.watch.is_none() {
                r.session.command(
                    fs,
                    Decision::Settings {
                        difficulty: r.prefs.difficulty,
                        friendly_fire: r.prefs.multiplayer,
                    },
                );
            }
            let loaded = r.session.load(fs).and_then(|()| {
                let g = r.session.game.as_deref().context("no level")?;
                Ok(Playing {
                    scene: Scene::new(&r.gpu, g)?,
                    intro: 0.0,
                    boss_bank: None,
                })
            });
            match loaded {
                Ok(mut p) => {
                    let Some(game) = r.session.game.as_deref() else {
                        anyhow::bail!("the level has gone");
                    };
                    if let Some(dir) = game.boss_model_dir() {
                        let imgs = hud::boss_meter_images(fs, &dir);
                        p.boss_bank = Some(r.hud.load_images(&r.gpu.device, &r.gpu.queue, &imgs));
                    } else if game.realm() == Some(12) {
                        // A secret level has no boss: its overlay bank holds the
                        // coin icons.
                        let imgs = hud::coin_images(fs);
                        p.boss_bank = Some(r.hud.load_images(&r.gpu.device, &r.gpu.queue, &imgs));
                    }
                    // A debug save state: the level's changes and the party.
                    if let Some(st) = state {
                        r.autosaved = game.time;
                        log::info!("state applied: {st}");
                    }
                    log::info!("entering {level}");
                    let streams = game.wave.map(|w| game.worlds.music(w)).unwrap_or_default();
                    r.music_now = 0;
                    r.music_switch = None;
                    r.music_fade = 1.0;
                    r.audio.set_music_fade(1.0);
                    if !r.audio.play_streams(fs, &streams) {
                        r.audio.play_level_music(fs, &level);
                    }
                    r.screen = Screen::Playing {
                        play: Box::new(p),
                        pause: None,
                    };
                }
                Err(e) => {
                    log::error!("loading {level}: {e:#}");
                    r.session.game = None;
                    r.session.phase = Phase::Idle;
                    r.screen = Screen::Title(Title::default());
                    r.audio.play_select_music(fs);
                }
            }
            // Don't fast-forward the game through the time spent loading.
            r.last = Instant::now();
            r.acc = 0.0;
        }

        // Ticks owed to a game that had to wait aren't all run at once.
        if r.net.is_some() {
            r.acc = r.acc.min(6.0 * TICK);
        }
        let frame = match r.surface.get_current_texture() {
            Ok(f) => f,
            Err(wgpu::SurfaceError::Outdated | wgpu::SurfaceError::Lost) => {
                r.surface.configure(&r.gpu.device, &r.config);
                return Ok(());
            }
            Err(e) => {
                log::warn!("surface: {e}");
                return Ok(());
            }
        };
        let view = frame.texture.create_view(&Default::default());
        let mut canvas = hud::Canvas::new(r.config.width, r.config.height);

        // Fixed-rate simulation and menu input.
        let mut escape = std::mem::take(&mut r.escape);
        let mut next: Option<Screen> = None;
        // In a network game Esc is this player's Start, like any other button.
        if let Some(np) = &mut r.net {
            np.escape |= std::mem::take(&mut escape);
        }
        while r.acc >= TICK && next.is_none() {
            // Everyone's inputs for the tick, or wait for them.
            if !net_tick(r, fs)? {
                break;
            }
            let all: Vec<(Source, PlayerInput)> = r
                .input
                .sources()
                .into_iter()
                .map(|s| (s, r.input.read(s)))
                .collect();
            let in_net = r.net.is_some();
            if !in_net && reset_held(&mut r.reset_hold, &all) {
                // OSResetSystem: the game boots again; only the memory card
                // (and the options) remain.
                r.reset_hold.clear();
                r.audio.stop();
                r.session = Session::new(
                    self.cfg.rules.clone(),
                    r.prefs.difficulty,
                    r.prefs.multiplayer,
                    Progress::default(),
                );
                r.suspended = None;
                r.attract = Attract::new();
                next = Some(Screen::Legal(120));
                escape = false;
                r.input.end_frame();
                r.acc -= TICK;
                continue;
            }
            // The debug menu, while open, has the tick (the game is frozen).
            let debug = if r.watch.is_none() && !in_net {
                debug_tick(r, fs, &all, escape)
            } else {
                None
            };
            if let Some(s) = debug {
                next = s;
                escape = false;
                r.input.end_frame();
                r.acc -= TICK;
                continue;
            }
            // ScreenSaver: any change in what a controller holds resets the
            // idle count, or ends a running saver (ScreenSaverEnd).
            let levels: Vec<(Source, u32)> =
                all.iter().map(|(s, i)| (*s, held_levels(i))).collect();
            let changed = levels != r.idle_levels;
            r.idle_levels = levels;
            if r.saver.is_some() {
                if changed {
                    r.saver = None;
                    r.idle = 0;
                    match &r.screen {
                        Screen::Shop { .. } => {
                            if let Phase::Shop { shop, .. } = &r.session.phase {
                                if let Some(m) = shop.music() {
                                    r.audio.play_bank_music(fs, &m);
                                }
                            }
                        }
                        Screen::Select(_) => r.audio.play_select_music(fs),
                        Screen::Playing { .. } => {
                            if let Some(g) = &r.session.game {
                                let streams = g.wave.map(|w| g.worlds.music(w)).unwrap_or_default();
                                if !r.audio.play_streams(fs, &streams) {
                                    r.audio.play_level_music(fs, &g.level.name);
                                }
                            }
                        }
                        _ => {}
                    }
                } else if let Some(s) = r.saver.as_mut() {
                    s.tick(crate::camera::half_tans());
                }
                escape = false;
                r.input.end_frame();
                r.acc -= TICK;
                continue;
            }
            r.idle = if !saver_counts(&r.screen) || changed || r.watch.is_some() || in_net {
                0
            } else {
                r.idle + 1
            };
            if r.idle >= crate::screensaver::IDLE_TICKS {
                // ScreenSaverStart: the music stops and the screen blanks.
                r.idle = 0;
                if r.saver_models.is_none() {
                    match crate::screensaver::SaverModels::load(fs, &r.gpu) {
                        Ok(m) => r.saver_models = Some(m),
                        Err(e) => log::warn!("screen saver: {e:#}"),
                    }
                }
                if r.saver_models.is_some() {
                    r.audio.stop();
                    r.saver = Some(crate::screensaver::Saver::new(time.to_bits()));
                }
            }
            match &mut r.screen {
                Screen::Legal(t) => {
                    *t = t.saturating_sub(1);
                    if *t == 0 {
                        next = Some(Screen::Attract);
                    }
                }
                Screen::GameOver { ticks, .. } => {
                    *ticks += 1;
                    if *ticks == 60 {
                        r.audio.play(fs, "S_GAMEOVERVOX");
                    }
                    if *ticks >= 240 {
                        r.attract = std::mem::take(&mut r.attract).game_over();
                        next = Some(Screen::Attract);
                    }
                }
                Screen::Attract => {
                    let inputs: Vec<PlayerInput> = all.iter().map(|(_, i)| *i).collect();
                    let ev = if escape {
                        attract::Event::Title
                    } else {
                        r.attract.update(fs, &inputs)
                    };
                    next = attract_event(fs, &r.gpu, &mut r.audio, &mut r.attract, ev);
                }
                Screen::Flyby { play, cam, ticks } => {
                    // do_flyby: the camera at the retail 30 frames a second (2
                    // ticks a frame), the world and monsters running, no heroes,
                    // no sound; done 30 ticks after the path ends, or at 60 s.
                    *ticks += 1;
                    if *ticks % 2 == 0 {
                        cam.update(2);
                    }
                    let (eye, at) = cam.view();
                    play.game.fixed_camera = Some((eye, at));
                    play.game.update(&[PlayerInput::default()]);
                    play.game.cam_focus = at;
                    play.game.sfx.clear();
                    // attract_check_input: Start goes to the title; after a
                    // second, the face buttons skip to the next screen.
                    let start = escape || all.iter().any(|(_, i)| i.start_pressed);
                    let skip = *ticks > 60
                        && all
                            .iter()
                            .any(|(_, i)| i.attack_pressed || i.b_pressed || i.magic_pressed);
                    if start {
                        next = attract_event(
                            fs,
                            &r.gpu,
                            &mut r.audio,
                            &mut r.attract,
                            attract::Event::Title,
                        );
                    } else if cam.done > 30 || *ticks > 3600 || skip {
                        let ev = r.attract.flyby_over(fs);
                        next = attract_event(fs, &r.gpu, &mut r.audio, &mut r.attract, ev)
                            .or(Some(Screen::Attract));
                    }
                }
                Screen::Title(t) => {
                    if escape {
                        el.exit();
                    }
                    let picked = t.update(&all, &mut r.prefs);
                    for snd in t.option_sfx.drain(..) {
                        r.audio.play(fs, &snd);
                    }
                    if std::mem::take(&mut t.volume_changed) {
                        r.audio.set_volumes(&r.prefs);
                    }
                    if let Some(src) = picked {
                        // A new game from the title: a new party with nothing done
                        // (loaded heroes bring theirs in player select), or
                        // what the debug menu set on the title.
                        r.session = Session::new(
                            self.cfg.rules.clone(),
                            r.prefs.difficulty,
                            r.prefs.multiplayer,
                            r.debug.new_game_progress(),
                        );
                        r.session.phase = Phase::Select;
                        r.suspended = None;
                        let s = &r.session;
                        let party = party_for(&r.card, &None, &s.heroes, &s.progress, &r.prefs);
                        let mut sel = Select::with_first(src).with_party(party);
                        if let Some((class, colour)) = &self.cfg.companion {
                            let info =
                                panels_for(&[Default::default(), (class.clone(), colour.clone())]);
                            sel.add_companion(info[1].class, info[1].colour);
                        }
                        next = Some(Screen::Select(sel));
                    } else if t.timed_out {
                        r.audio.stop();
                        r.attract = std::mem::take(&mut r.attract).after_title();
                        next = Some(Screen::Attract);
                    }
                    for snd in t.sfx.drain(..) {
                        r.audio.play(fs, snd);
                    }
                }
                Screen::Select(sel) => {
                    for snd in sel.sfx.drain(..) {
                        r.audio.play(fs, &snd);
                    }
                    if escape || sel.leave {
                        r.card = sel.card().clone();
                        r.prefs.players = sel.controls();
                        r.session.phase = Phase::Idle;
                        next = Some(Screen::Title(Title::default()));
                    } else if sel.update(&all).is_some() {
                        r.sources = sel.sources();
                        let panels = sel.panels();
                        // New players start fresh; returning ones keep their stats,
                        // and loaded heroes take their class's saved state. Each
                        // comes into play invisible for a while (activate_player).
                        let mut carry: Carry = Default::default();
                        for info in &panels {
                            let mut c = sel
                                .carried(info.slot % 4)
                                .unwrap_or_else(crate::game::Carried::new_character);
                            c.activate(info.class);
                            carry[info.slot % 4] = Some(c);
                        }
                        // A loaded hero's achievements join the party's.
                        let mut progress = r.session.progress.clone();
                        sel.merge_progress(&mut progress);
                        r.card = sel.card().clone();
                        r.prefs.players = sel.controls();
                        let chosen = Chosen {
                            panels,
                            carry,
                            heroes: sel.heroes(),
                            progress,
                        };
                        r.session
                            .command(fs, Decision::PartyChosen(Box::new(chosen)));
                    }
                }
                Screen::Loading { .. } => {}
                Screen::Map { .. } => r.session.update(fs, &[]),
                Screen::Intro { movie } => {
                    if escape {
                        r.session.command(fs, Decision::SkipMovie);
                    } else {
                        let packed: Vec<_> = all.iter().map(|(_, i)| i.pack()).collect();
                        r.session.update(fs, &packed);
                    }
                    if let (Some(m), Phase::Movie { clock, .. }) = (movie, &r.session.phase) {
                        m.show(clock.shown);
                    }
                }
                Screen::Shop { .. } => {
                    r.companion_ticks = r.companion_ticks.wrapping_add(1);
                    let t = r.companion_ticks;
                    let packed: Vec<_> = r
                        .sources
                        .iter()
                        .map(|s| match s {
                            Source::Companion => crate::runner::companion::shop_input(t).pack(),
                            // Someone gone shops the same way: they leave it.
                            Source::Net(p)
                                if r.net.as_ref().is_some_and(|np| np.net.is_gone(*p)) =>
                            {
                                crate::runner::companion::shop_input(t).pack()
                            }
                            _ => r.input.read(*s).pack(),
                        })
                        .collect();
                    r.session.update(fs, &packed);
                }
                Screen::Playing { play, pause } => {
                    let watching = r.watch.is_some();
                    if watching && escape {
                        el.exit();
                    }
                    let inputs = party_inputs(
                        &r.input,
                        &r.sources,
                        r.session.game.as_deref(),
                        &mut r.companion,
                    );
                    let game = r.session.game.as_deref();
                    let sumner_busy = game
                        .and_then(|g| g.sumner.as_ref())
                        .is_some_and(|s| s.controls_locked());
                    let wave = game.and_then(|g| g.wave);
                    let scrolling = game.is_some_and(|g| g.scroll_box.is_some());
                    let active = |i: usize| {
                        game.and_then(|g| g.players.get(i))
                            .is_some_and(|p| p.state.active())
                    };
                    let joiner = (!watching
                        && wave == Some(TOWER_WAVE)
                        && !sumner_busy
                        && r.sources.len() < 4)
                        .then(|| {
                            all.iter()
                                .find(|(s, i)| {
                                    !r.sources.contains(s) && (i.start_pressed || i.attack_pressed)
                                })
                                .map(|(s, _)| *s)
                        })
                        .flatten();
                    // OptionsStart: a playing player's Start opens the Game Menu,
                    // or the Tower Menu in the tower.
                    let opener = (0..inputs.len())
                        .find(|&i| inputs[i].start_pressed && !sumner_busy && active(i))
                        .or(escape.then_some(0))
                        .filter(|_| !watching);
                    if let Some(menu) = pause {
                        // The menu's owner drives it; Esc acts as their Start.
                        let owner = menu_owner(menu);
                        let mut inp = inputs.get(owner).copied().unwrap_or_default();
                        inp.start_pressed |= escape;
                        let action = menu.update(&inp, &mut r.prefs);
                        for snd in menu.sfx.drain(..) {
                            r.audio.play(fs, &snd);
                        }
                        match action {
                            OptionsAction::Closed => {
                                // Leaving Sumner's menu: he waves the party off.
                                if let Some(h) = pause.as_ref().and_then(|m| m.hints()) {
                                    r.session.progress.hints = h.state;
                                    r.session.command(fs, Decision::Game(Command::HintsClosed));
                                }
                                *pause = None
                            }
                            OptionsAction::VolumeChanged => r.audio.set_volumes(&r.prefs),
                            // "Abort Level?" Yes: everyone is put out of the level
                            // and the party goes back to the tower.
                            OptionsAction::QuitLevel => {
                                autosave_session(&r.session, &r.sources, &r.input, &r.save_dir);
                                *pause = None;
                                r.session.command(fs, Decision::Game(Command::QuitLevel));
                            }
                            OptionsAction::ManageCharacter => {
                                r.session.command(fs, Decision::ManageParty);
                            }
                            // TODO: the shop and the inventory screen (shop.c).
                            OptionsAction::Shop | OptionsAction::Inventory => {
                                let mode = if action == OptionsAction::Shop {
                                    crate::shop::ShopMode::FromMenu
                                } else {
                                    crate::shop::ShopMode::Inventory
                                };
                                *pause = None;
                                r.session.command(fs, Decision::OpenShop(mode));
                            }
                            OptionsAction::QuitGame => {
                                // In a network game only the one who chose it
                                // leaves; for the others the game goes on.
                                let quitter = match (&r.net, r.sources.get(owner)) {
                                    (Some(np), Some(Source::Net(p))) if *p != np.net.me() => {
                                        Some(*p)
                                    }
                                    _ => None,
                                };
                                if let Some(p) = quitter {
                                    if let Some(np) = &mut r.net {
                                        np.net.drop_player(p);
                                        np.note = Some((
                                            format!("Player {} has left", p + 1),
                                            Instant::now(),
                                        ));
                                    }
                                    *pause = None;
                                    if let Some(slot) = r.session.panels.get(owner).map(|i| i.slot)
                                    {
                                        r.session.player_left(fs, slot);
                                    }
                                } else {
                                    if let Some(np) = &mut r.net {
                                        np.net.leave();
                                    }
                                    autosave_session(&r.session, &r.sources, &r.input, &r.save_dir);
                                    r.session.command(fs, Decision::QuitGame);
                                }
                            }
                            OptionsAction::None => {}
                        }
                        if !menu_drawing(pause) {
                            *pause = None;
                        }
                        if pause.is_none() {
                            r.session.command(fs, Decision::Unpause);
                        }
                    } else if scrolling {
                        // A scroll being read: only its box runs.
                        match (&mut r.watch, r.session.game.as_deref_mut()) {
                            (Some(w), Some(g)) => w.step(g),
                            _ => {
                                let packed: Vec<_> = inputs.iter().map(|i| i.pack()).collect();
                                r.session.update(fs, &packed);
                            }
                        }
                    } else if let Some(src) = joiner {
                        // check_active_players: a new pad joins in the tower, which
                        // opens player select for it (init_player_select(1)).
                        r.joining = Some(src);
                        r.session.command(fs, Decision::ManageParty);
                    } else if let Some(owner) = opener {
                        r.audio.play(fs, "S_OPTMENUSEL");
                        let slot = r.session.panels.get(owner).map_or(owner, |p| p.slot);
                        r.session.command(fs, Decision::Pause(owner));
                        *pause = Some(if wave == Some(TOWER_WAVE) {
                            Options::tower(owner).with_slot(slot)
                        } else {
                            let mut o = Options::game(owner).with_slot(slot);
                            // No "Quit Level" in the secret levels (realm 0xC).
                            o.set_quit_level_enabled(!quit_level_disabled(wave));
                            o
                        });
                    } else {
                        // PlayerControl[i]: each player's Controls options,
                        // kept by panel slot (as select and do_vibe do).
                        for i in 0..4 {
                            let slot = r.session.panels.get(i).map_or(i, |p| p.slot);
                            let prefs = r.prefs.players[slot % 4];
                            let differs = r
                                .session
                                .game
                                .as_ref()
                                .is_some_and(|g| g.controls[i] != prefs);
                            if !watching && differs {
                                r.session.command(
                                    fs,
                                    Decision::Game(Command::SetControls { player: i, prefs }),
                                );
                            }
                        }
                        if r.acc < 2.0 * TICK {
                            // interp.rs: the tick being left, if not yet drawn.
                            if let Some(g) = r.session.game.as_deref() {
                                play.scene.pre_tick(&r.gpu, g);
                            }
                        }
                        let god = r.session.game.as_ref().is_some_and(|g| g.debug_god);
                        if !watching && r.net.is_none() && god != r.debug.god {
                            r.session
                                .command(fs, Decision::Game(Command::DebugGod(r.debug.god)));
                        }
                        match (&mut r.watch, r.session.game.as_deref_mut()) {
                            (Some(w), Some(g)) => w.step(g),
                            _ => {
                                let packed: Vec<_> = inputs.iter().map(|i| i.pack()).collect();
                                r.session.update(fs, &packed);
                            }
                        }
                        // The level may have just ended.
                        let mut autosave_due = false;
                        if let Some(game) = r.session.game.as_deref_mut() {
                            r.bugs.observe(game, &inputs);
                            play.intro += TICK;
                            // Debug save states: an autosave every minute of play.
                            if game.time < r.autosaved {
                                r.autosaved = game.time;
                            } else if !watching
                                && r.net.is_none()
                                && game.time - r.autosaved >= AUTOSAVE_EVERY
                            {
                                r.autosaved = game.time;
                                autosave_due = true;
                            }
                            // do_vibe: only for players with the Vibration option on.
                            for v in game.snd.rumble.drain(..) {
                                let slot =
                                    r.session.panels.get(v.player).map_or(v.player, |p| p.slot);
                                if r.prefs.players[slot % 4].rumble {
                                    if let Some(&src) = r.sources.get(v.player) {
                                        let k = crate::game::sounds::VIBE[v.strength.min(4)];
                                        r.input.rumble(src, k, v.ticks * 1000 / 60);
                                    }
                                }
                            }
                            // AudioMusicVolUpdate / AudioSetupLevelStreams: a new music
                            // area fades the stream out (3/127 a tick; or switches at
                            // once), plays the area's stream, and fades back in.
                            let (want, how) = game.music_area;
                            if want != r.music_now && r.music_switch != Some(want) {
                                r.music_switch = Some(want);
                            }
                            if let Some(area) = r.music_switch {
                                if how >= 2 || r.music_fade <= 3.0 / 127.0 {
                                    if let Some(w) = game.wave {
                                        let (streams, n) = game.worlds.music_area(w, area);
                                        if n > 1 {
                                            r.audio.play_streams(fs, &streams);
                                        }
                                    }
                                    r.music_now = area;
                                    r.music_switch = None;
                                } else {
                                    r.music_fade = (r.music_fade - 3.0 / 127.0).max(0.0);
                                }
                            } else if r.music_fade < 1.0 {
                                r.music_fade = (r.music_fade + 8.0 / 127.0).min(1.0);
                            }
                            r.audio.set_music_fade(r.music_fade);
                            for (id, name, vol) in game.ambient_levels() {
                                r.audio.ambient(fs, id, name, vol);
                            }
                            if std::mem::take(&mut game.world_changed) {
                                if let Ok(sc) = Scene::new(&r.gpu, game) {
                                    play.scene = sc;
                                }
                            }
                        }
                        if autosave_due {
                            autosave_session(&r.session, &r.sources, &r.input, &r.save_dir);
                        }
                    }
                }
            }
            for snd in r.session.sfx.drain(..) {
                crate::bugreport::sound(&snd);
                r.audio.play(fs, &snd);
            }
            if let Some(s) = session_events(r, fs) {
                next = Some(s);
            }
            escape = false;
            r.input.end_frame();
            r.acc -= TICK;
        }
        if let Some(s) = next {
            // Out of a level, the music plays at full volume again.
            if !matches!(s, Screen::Playing { .. }) {
                r.music_fade = 1.0;
                r.audio.set_music_fade(1.0);
            }
            // A new screen starts with no ambient sounds.
            r.audio.ambient_clear();
            if matches!(s, Screen::Title(_) | Screen::Select(_)) {
                r.audio.play_select_music(fs);
            }
            r.screen = s;
        }

        // Draw.
        let ticks = (time * 60.0) as u32;
        let mut in_game = false;
        match &mut r.screen {
            Screen::GameOver { ticks, text } => {
                // DrawStringText centres the whole string, then it is cut to
                // (t - 60) / 8 letters, so they appear in place one by one.
                let mut s = Scr::new(&mut canvas, &r.hud.ui, *ticks as u32);
                s.bars(1.0);
                s.rect(0.0, 0.0, 512.0, 384.0, [0.0, 0.0, 0.0, 1.0]);
                let n = ((*ticks - 60) >> 3).clamp(0, 9) as usize;
                let w = s.text_width(crate::ui::FONT_32, 2.0, text);
                let x = 256.0 - (w * 0.5).floor();
                let shown: String = text.chars().take(n).collect();
                s.text(2.0, x, 120.0, crate::ui::FONT_32, 0xFFFFFF, 0, &shown);
            }
            Screen::Legal(_) => attract::draw_legal(&mut Scr::new(&mut canvas, &r.hud.ui, ticks)),
            Screen::Attract => {
                if let Some(attract::Show::Movie(m)) = &mut r.attract.show {
                    if m.dirty {
                        m.dirty = false;
                        let (w, h) = m.size();
                        let rgba = m.rgba();
                        r.movie_tex =
                            Some(r.hud.stream_frame(&r.gpu.device, &r.gpu.queue, w, h, &rgba));
                    }
                }
                r.attract
                    .draw(&mut Scr::new(&mut canvas, &r.hud.ui, ticks), r.movie_tex);
            }
            Screen::Flyby { play, .. } => {
                in_game = true;
                play.scene.sync(&r.gpu, &play.game);
                let (w, h) = (r.config.width as f32, r.config.height as f32);
                let k = (w / 512.0).min(h / 384.0);
                let vp = [
                    (w - 512.0 * k) / 2.0,
                    (h - 384.0 * k) / 2.0,
                    512.0 * k,
                    384.0 * k,
                ];
                let g = play.scene.globals(&play.game, 4.0 / 3.0);
                r.gpu.render_viewport(
                    &view,
                    &r.depth,
                    &g,
                    CLEAR,
                    &play.scene.draws(&play.game),
                    Some(vp),
                    Some(play.scene.sprites()),
                );
                let mut s = Scr::new(&mut canvas, &r.hud.ui, ticks);
                s.bars(1.0);
                // (LOGO_B is placed at (368, -304), off the top of the screen.)
                s.glow_text(1.0, -256.0, 320.0, "Press Start");
            }
            Screen::Map { map } => {
                if let Phase::Map { clock, .. } = &r.session.phase {
                    map.draw(clock, &mut Scr::new(&mut canvas, &r.hud.ui, ticks));
                }
            }
            Screen::Shop { bank } => {
                if let Phase::Shop { shop, .. } = &r.session.phase {
                    crate::shop::ShopView { shop, bank }.draw(&mut Scr::new(
                        &mut canvas,
                        &r.hud.ui,
                        ticks,
                    ));
                }
            }
            Screen::Intro { movie: None } => {}
            Screen::Intro { movie: Some(movie) } => {
                if movie.dirty {
                    movie.dirty = false;
                    let (w, h) = movie.size();
                    let rgba = movie.rgba();
                    r.movie_tex =
                        Some(r.hud.stream_frame(&r.gpu.device, &r.gpu.queue, w, h, &rgba));
                }
                let mut s = Scr::new(&mut canvas, &r.hud.ui, ticks);
                s.bars(1.0);
                if let Some(t) = r.movie_tex {
                    s.blit(t, 0.0, 0.0, 512.0, 384.0, 0);
                }
            }
            Screen::Title(t) => t.draw(&mut Scr::new(&mut canvas, &r.hud.ui, ticks)),
            Screen::Select(sel) => sel.draw(&mut Scr::new(&mut canvas, &r.hud.ui, ticks)),
            Screen::Loading { shown, .. } => {
                menu::draw_loading(&mut Scr::new(&mut canvas, &r.hud.ui, ticks));
                *shown = true;
            }
            Screen::Playing { play, pause } => {
                let Some(game) = r.session.game.as_deref_mut() else {
                    frame.present();
                    return Ok(());
                };
                in_game = true;
                let p = &mut **play;
                for snd in game.sfx.drain(..) {
                    crate::bugreport::sound(&snd);
                    r.audio.play(fs, &snd);
                }
                // fn_80052134: the compass hides while the options are open.
                game.snd.show_compass = r.prefs.compass && pause.is_none();
                p.scene.sync(&r.gpu, &*game);
                // Smooth at any refresh: between the last two ticks (interp.rs).
                p.scene.blend(&*game, r.acc / TICK);
                // The game draws a 4:3 picture; keep it, centred, in any window.
                let (w, h) = (r.config.width as f32, r.config.height as f32);
                let k = (w / 512.0).min(h / 384.0);
                let vp = [
                    (w - 512.0 * k) / 2.0,
                    (h - 384.0 * k) / 2.0,
                    512.0 * k,
                    384.0 * k,
                ];
                let g = p.scene.globals(&*game, 4.0 / 3.0);
                r.gpu.render_viewport(
                    &view,
                    &r.depth,
                    &g,
                    CLEAR,
                    &p.scene.draws(&*game),
                    Some(vp),
                    Some(p.scene.sprites()),
                );
                hud::build(
                    &mut Scr::new(&mut canvas, &r.hud.ui, ticks),
                    &*game,
                    &r.session.panels,
                );
                hud::build_coins(
                    &mut Scr::new(&mut canvas, &r.hud.ui, ticks),
                    &*game,
                    &r.session.panels,
                    p.boss_bank.as_ref(),
                );
                hud::build_boss(
                    &mut Scr::new(&mut canvas, &r.hud.ui, ticks),
                    &*game,
                    p.boss_bank.as_ref(),
                );
                let fade = menu::loading_fade(p.intro);
                if fade < 255 {
                    menu::transition(&mut Scr::new(&mut canvas, &r.hud.ui, ticks), fade);
                }
                if let Some(b) = &game.scroll_box {
                    menu::draw_scroll(
                        &mut Scr::new(&mut canvas, &r.hud.ui, ticks),
                        b.text(),
                        b.scale,
                    );
                }
                if let Some(menu) = pause {
                    menu.draw(&mut Scr::new(&mut canvas, &r.hud.ui, ticks));
                }
            }
        }
        {
            let mut s = Scr::new(&mut canvas, &r.hud.ui, ticks);
            r.debug.draw_overlay(&mut s);
            r.debug.draw(&mut s);
            if let Some(np) = &r.net {
                draw_net_status(&mut s, np);
            }
        }
        if !in_game {
            r.gpu.render(
                &view,
                &r.depth,
                &crate::gpu::globals(glam::Mat4::IDENTITY, glam::Vec3::ZERO, [0.0; 3], 0.0),
                CLEAR,
                &[],
            );
        }
        r.hud.draw(&r.gpu.device, &r.gpu.queue, &view, &canvas);
        // The menus' 3D arrow goes over the 2D overlay.
        let arrow = {
            let mut scratch = hud::Canvas::new(r.config.width, r.config.height);
            let s = Scr::new(&mut scratch, &r.hud.ui, ticks);
            match &r.screen {
                _ if r.debug.open => None,
                Screen::Title(t) => t.cursor(&s),
                Screen::Playing { pause: Some(m), .. } => m.cursor(&s),
                _ => None,
            }
        };
        if let (Some(at), Some(c)) = (arrow, r.cursor.as_mut()) {
            let (w, h) = (r.config.width as f32, r.config.height as f32);
            let k = (w / 512.0).min(h / 384.0);
            let vp = [
                (w - 512.0 * k) / 2.0,
                (h - 384.0 * k) / 2.0,
                512.0 * k,
                384.0 * k,
            ];
            c.draw(&r.gpu, &view, &r.depth, vp, at);
        }
        if let (Some(s), Some(m)) = (&r.saver, r.saver_models.as_mut()) {
            let (w, h) = (r.config.width as f32, r.config.height as f32);
            let k = (w / 512.0).min(h / 384.0);
            let vp = [
                (w - 512.0 * k) / 2.0,
                (h - 384.0 * k) / 2.0,
                512.0 * k,
                384.0 * k,
            ];
            m.draw(s, &r.gpu, &view, &r.depth, vp);
        }
        frame.present();
        r.frames += 1;

        Ok(())
    }
}

/// HUD panels for players given on the command line: slots in order, no names.
/// The developer debug menu's tick (debugmenu.rs; not in the original).
/// None: the menu is shut and stays shut, the screen runs as usual.
/// Some(next): the menu had this tick (and may change the screen).
fn debug_tick(
    r: &mut Running,
    fs: &GameFs,
    all: &[(Source, PlayerInput)],
    escape: bool,
) -> Option<Option<Screen>> {
    use crate::debugmenu::{self, Action, Ctx};
    let f1 = r.input.just_pressed(KeyCode::F1);
    let mut opening = None;
    if !r.debug.open {
        if let Screen::Select(sel) = &mut r.screen {
            sel.set_debug_unlock(r.debug.class_unlock);
        }
        if !matches!(r.screen, Screen::Title(_) | Screen::Playing { .. }) {
            return None;
        }
        opening = Some(if f1 {
            Source::Keyboard
        } else {
            all.iter().find(|(_, i)| debugmenu::open_combo(i))?.0
        });
        if !r.debug.has_levels() {
            match gdl_formats::wdata::Worlds::load(|p| fs.read(p)) {
                Ok(w) => r.debug.set_levels(debugmenu::realm_table(&w)),
                Err(e) => log::warn!("debug menu levels: {e:#}"),
            }
        }
        r.audio.play(fs, "S_OPTMENUSEL");
    }
    r.idle = 0;
    // The menu can change the game: a recording can't follow that.
    let in_level = matches!(r.screen, Screen::Playing { .. });
    let before = r
        .session
        .game
        .as_ref()
        .filter(|_| in_level)
        .map(|g| g.fingerprint());
    let (game, panels) = if in_level {
        (r.session.game.as_deref_mut(), Some(&mut r.session.panels))
    } else {
        (None, None)
    };
    let mut ctx = Ctx {
        progress: &mut r.session.progress,
        game,
        panels,
        heroes: &mut r.session.heroes,
        compass: &mut r.prefs.compass,
    };
    if opening.is_some() {
        r.debug.show(opening, &mut ctx);
        return Some(None);
    }
    let action = if f1 || escape {
        r.debug.close();
        Action::Close
    } else {
        let inputs: Vec<PlayerInput> = all.iter().map(|(_, i)| *i).collect();
        r.debug.update(&inputs, &mut ctx)
    };
    if let (Some(before), Some(g)) = (before, r.session.game.as_deref()) {
        if g.fingerprint() != before {
            if let Some(rec) = &mut r.session.recording {
                rec.taint("the debug menu changed the game");
            }
        }
    }
    let dirty = std::mem::take(&mut r.debug.tower_dirty);
    match action {
        Action::None => {
            r.debug.tower_dirty = dirty;
            Some(None)
        }
        Action::Close => {
            let tower = matches!(r.screen, Screen::Playing { .. })
                && r.session
                    .game
                    .as_ref()
                    .is_some_and(|g| g.wave == Some(TOWER_WAVE));
            // The tower is reloaded so its portals and barriers follow the
            // progress set.
            Some((dirty && tower).then(|| debug_warp(r, fs, "levelL1".into())))
        }
        Action::Warp(level) => {
            r.debug.close();
            Some(Some(debug_warp(r, fs, level)))
        }
        Action::FileBug(tags) => {
            r.debug.tower_dirty = dirty;
            let res = file_bug(r, fs, &tags);
            if let Err(e) = &res {
                log::warn!("bug report: {e}");
            }
            r.debug.bug_filed(res);
            Some(None)
        }
        Action::SaveState(slot) => {
            let msg = match save_state(r, &slot) {
                Ok(p) => format!("Saved state {slot} ({})", p.display()),
                Err(e) => format!("Save failed: {e:#}"),
            };
            log::info!("{msg}");
            r.debug.report(msg);
            Some(None)
        }
        Action::LoadState(slot) => {
            let path = savestate::resolve(&slot, r.save_dir.as_deref());
            match SaveState::read(&path) {
                Ok(st) => {
                    log::info!("loading state {}: {}", path.display(), st.summary());
                    r.debug.close();
                    Some(Some(begin_state(r, fs, st)))
                }
                Err(e) => {
                    r.debug.report(format!("No state {slot}: {e:#}"));
                    Some(None)
                }
            }
        }
    }
}

/// Seconds of level play between autosaves.
const AUTOSAVE_EVERY: f32 = 60.0;

/// Write a debug save state of the running level to `slot` (or a file).
fn save_state(r: &Running, slot: &str) -> Result<std::path::PathBuf> {
    if !matches!(r.screen, Screen::Playing { .. }) {
        anyhow::bail!("not in a level");
    }
    write_state(&r.session, &r.sources, &r.input, &r.save_dir, slot)
}

fn write_state(
    session: &Session,
    sources: &[Source],
    input: &Input,
    save_dir: &Option<std::path::PathBuf>,
    slot: &str,
) -> Result<std::path::PathBuf> {
    let game = session.game.as_deref().context("not in a level")?;
    let pads = input.pads();
    let names: Vec<String> = sources
        .iter()
        .map(|s| crate::input::source_name(s, &pads))
        .collect();
    let st = SaveState::capture(game, &session.panels, &names, &session.progress);
    let path = savestate::resolve(slot, save_dir.as_deref());
    st.write(&path)?;
    Ok(path)
}

/// The autosave slot, written when a level is quit (and every minute of
/// play), so `--load-state autosave` picks up after a crash or a rebuild.
fn autosave(r: &Running) {
    if matches!(r.screen, Screen::Playing { .. }) {
        autosave_session(&r.session, &r.sources, &r.input, &r.save_dir);
    }
}

fn autosave_session(
    session: &Session,
    sources: &[Source],
    input: &Input,
    save_dir: &Option<std::path::PathBuf>,
) {
    let Some(game) = session.game.as_deref() else {
        return;
    };
    // Not a finished level or a party that is all out of it.
    if game.level_end.is_some() || game.players.iter().all(|p| !p.state.active()) {
        return;
    }
    match write_state(session, sources, input, save_dir, savestate::AUTOSAVE) {
        Ok(p) => log::info!("autosaved {}", p.display()),
        Err(e) => log::warn!("autosave: {e:#}"),
    }
}

/// Start a debug save state: its party, controllers and progress, then its
/// level (the changes are applied once it has loaded).
fn begin_state(r: &mut Running, fs: &GameFs, st: SaveState) -> Screen {
    r.audio.stop();
    r.sources = crate::input::bind_sources(&st.heroes, &r.input.pads(), &r.sources);
    r.prefs.difficulty = st.difficulty.min(2);
    r.session.begin_state(st);
    session_events(r, fs).unwrap_or(Screen::Loading { shown: false })
}

/// The debug menu's "File bug report" (bugreport.rs; not in the original).
fn file_bug(
    r: &mut Running,
    fs: &GameFs,
    tags: &[&'static str],
) -> std::result::Result<std::path::PathBuf, String> {
    let rules = r.session.rules();
    let compat = local_compat(r, fs, &rules)
        .map_err(|e| log::warn!("bug report replay: {e:#}"))
        .ok();
    let level = r
        .session
        .game
        .as_deref()
        .filter(|_| matches!(r.screen, Screen::Playing { .. }));
    let replay = match (level, &r.session.recording, &compat) {
        (Some(g), Some(rec), Some(c)) => Some(rec.to_text(g, c)),
        _ => None,
    };
    let root = crate::bugreport::dir().ok_or("no folder for reports")?;
    let shot = match bug_screenshot(r) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("bug report screenshot: {e:#}");
            None
        }
    };
    let (game, panels): (Option<&Game>, &[PanelInfo]) = match (&r.screen, r.session.game.as_deref())
    {
        (Screen::Playing { .. }, Some(g)) => (Some(g), &r.session.panels),
        _ => (None, &[]),
    };
    let screen = match &r.screen {
        Screen::Legal(_) => "legal",
        Screen::Attract => "attract",
        Screen::Flyby { .. } => "flyby",
        Screen::GameOver { .. } => "game over",
        Screen::Title(_) => "title",
        Screen::Select(_) => "select",
        Screen::Loading { .. } => "loading",
        Screen::Map { .. } => "map",
        Screen::Shop { .. } => "shop",
        Screen::Intro { .. } => "intro movie",
        Screen::Playing { pause: Some(_), .. } => "playing (game menu open)",
        Screen::Playing { .. } => "playing",
    };
    let report = crate::bugreport::Report {
        game,
        panels,
        progress: Some(&r.session.progress),
        heroes: &r.session.heroes,
        prefs: Some(&r.prefs),
        card_file: r.card.path(),
        tags,
        screen,
        frames: r.frames,
        via: "debug menu",
        replay: replay.as_deref(),
    };
    crate::bugreport::file(&root, &report, shot.as_ref()).map_err(|e| format!("{e:#}"))
}

/// The level as last drawn, with its HUD but not the debug menu (1024x768).
fn bug_screenshot(r: &Running) -> Result<Option<gdl_formats::Image>> {
    let Screen::Playing { play, .. } = &r.screen else {
        return Ok(None);
    };
    let Some(game) = r.session.game.as_deref() else {
        return Ok(None);
    };
    let p = &**play;
    let (w, h) = (1024, 768);
    let ticks = 100;
    let globals = p.scene.globals(game, 4.0 / 3.0);
    let draw = |view: &wgpu::TextureView| {
        let mut canvas = hud::Canvas::new(w, h);
        hud::build(
            &mut Scr::new(&mut canvas, &r.hud.ui, ticks),
            game,
            &r.session.panels,
        );
        hud::build_coins(
            &mut Scr::new(&mut canvas, &r.hud.ui, ticks),
            game,
            &r.session.panels,
            p.boss_bank.as_ref(),
        );
        hud::build_boss(
            &mut Scr::new(&mut canvas, &r.hud.ui, ticks),
            game,
            p.boss_bank.as_ref(),
        );
        if let Some(b) = &game.scroll_box {
            menu::draw_scroll(
                &mut Scr::new(&mut canvas, &r.hud.ui, ticks),
                b.text(),
                b.scale,
            );
        }
        r.hud.draw(&r.gpu.device, &r.gpu.queue, view, &canvas);
    };
    let mut img = r.gpu.render_to_image(
        w,
        h,
        &globals,
        CLEAR,
        &p.scene.draws(game),
        Some(p.scene.sprites()),
        Some(&draw),
    )?;
    if matches!(
        r.gpu.color_format,
        wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
    ) {
        for px in img.rgba.chunks_exact_mut(4) {
            px.swap(0, 2);
        }
    }
    for px in img.rgba.chunks_exact_mut(4) {
        px[3] = 255;
    }
    Ok(Some(img))
}

/// The debug menu's warp: the party (as it is now) into `level`; from the
/// title, a Warrior on the controller that opened the menu.
fn debug_warp(r: &mut Running, fs: &GameFs, level: String) -> Screen {
    r.audio.stop();
    if r.session.game.is_none() {
        r.sources = vec![r.debug.owner.unwrap_or(Source::Keyboard)];
    }
    r.session
        .warp(level, || panels_for(&[("WAR".into(), "RED".into())]));
    session_events(r, fs).unwrap_or(Screen::Loading { shown: false })
}

fn panels_for(players: &[(String, String)]) -> Vec<PanelInfo> {
    players
        .iter()
        .enumerate()
        .map(|(slot, (class, colour))| PanelInfo {
            slot,
            name: String::new(),
            colour: COLOURS.iter().position(|c| c == colour).unwrap_or(slot % 4),
            class: CLASSES.iter().position(|c| c == class).unwrap_or(0),
            hidden: None,
        })
        .collect()
}

pub fn run(cfg: Config) -> Result<()> {
    run_with(cfg, EventLoop::new()?)
}

/// Android entry: the event loop must be built around the activity.
#[cfg(target_os = "android")]
pub fn run_android(cfg: Config, app: winit::platform::android::activity::AndroidApp) -> Result<()> {
    use winit::platform::android::EventLoopBuilderExtAndroid;
    let el = EventLoop::builder().with_android_app(app).build()?;
    run_with(cfg, el)
}

fn run_with(cfg: Config, el: EventLoop<()>) -> Result<()> {
    el.set_control_flow(ControlFlow::Poll);
    let mut app = App {
        cfg,
        started: Instant::now(),
        run: None,
        error: None,
    };
    el.run_app(&mut app)?;
    match app.error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// The player a menu stack belongs to.
fn menu_owner(menu: &Options) -> usize {
    menu.player()
}

/// Is a closed menu still finishing its fade?
fn menu_drawing(menu: &Option<Options>) -> bool {
    menu.as_ref().is_some_and(|m| m.is_drawing())
}

/// MG_OVER with ENGLISH.ROM message 169 ("GAME OVER").
fn game_over_screen(fs: &GameFs) -> Screen {
    let text = fs
        .read("TEXT/ENGLISH.ROM")
        .ok()
        .and_then(|d| gdl_formats::text::TextRom::parse(&d).ok())
        .and_then(|r| r.messages.get(169).and_then(|m| m.lines.first().cloned()))
        .unwrap_or_else(|| "GAME OVER".into());
    Screen::GameOver { ticks: 0, text }
}

/// What Sumner's hint pickers ask about the party.
fn hint_facts(
    progress: &crate::progress::Progress,
    game: &crate::game::Game,
) -> crate::hints::HintFacts {
    let mut world_open = [false; 16];
    for (r, o) in world_open.iter_mut().enumerate() {
        *o = progress.world_open_any(r as u32, &game.crystals);
    }
    let banked = game.sumner.as_ref().map_or((0, 0), |s| s.banked());
    crate::hints::HintFacts {
        world_open,
        runes: progress.runes | game.runes | banked.0,
        shards: progress.shards | game.shards | banked.1,
        boss_attempts: progress.boss_attempts,
        level_masks: progress.level_masks,
        legend_held: progress.legend | game.legend_held,
        sections: std::array::from_fn(|j| {
            game.garg_items[j] < 0 || game.garg_items[j] >= crate::game::GARG_NEEDED[j]
        }),
    }
}

/// What the attract loop's event means for the app: sound, and the next screen
/// (the title, a flyby), if it changes.
fn attract_event(
    fs: &GameFs,
    gpu: &Gpu,
    audio: &mut Audio,
    attract: &mut Attract,
    ev: attract::Event,
) -> Option<Screen> {
    match ev {
        attract::Event::MovieStarted => {
            if let Some(attract::Show::Movie(m)) = &mut attract.show {
                if let Some(a) = m.audio.take() {
                    audio.play_once(a.pcm, a.channels as u16, a.playback_rate);
                }
            }
            None
        }
        attract::Event::MovieEnded => {
            audio.stop();
            None
        }
        attract::Event::Credits => {
            audio.play_streams(fs, &["castle2b".into()]);
            None
        }
        attract::Event::Title => {
            audio.stop();
            attract.show = None;
            Some(Screen::Title(Title::default()))
        }
        attract::Event::Flyby(level) => {
            // The flyby is silent (AudioReset, no music); its level loads with
            // one hero who stays out of play.
            audio.stop();
            let flyby = (|| -> Result<Screen> {
                let mut game = Game::new_with(fs, &level, &[("WAR", "RED")], 1)?;
                for p in &mut game.players {
                    p.state = PlayerState::Exited;
                }
                let cam =
                    crate::flyby::FlybyCam::new(&game.level.worlds).context("no attract camera")?;
                game.fixed_camera = Some(cam.view());
                game.cam_focus = cam.view().1;
                let scene = Scene::new(gpu, &game)?;
                Ok(Screen::Flyby {
                    play: Box::new(FlybyPlay { game, scene }),
                    cam: Box::new(cam),
                    ticks: 0,
                })
            })();
            match flyby {
                Ok(s) => Some(s),
                Err(e) => {
                    log::warn!("flyby {level}: {e:#}");
                    let ev = attract.flyby_over(fs);
                    attract_event(fs, gpu, audio, attract, ev).or(Some(Screen::Attract))
                }
            }
        }
        attract::Event::None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::setup::LevelStart;
    use crate::save::HeroSave;

    fn data() -> Option<GameFs> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game-data");
        GameFs::open(&root).ok()
    }

    fn panel(slot: usize, name: &str, class: usize, hidden: Option<usize>) -> PanelInfo {
        PanelInfo {
            slot,
            name: name.into(),
            colour: slot,
            class,
            hidden,
        }
    }

    /// The level's heroes: Sumner's loadout, a name cheat, a hidden
    /// character without its cheats; and what they bring back.
    #[test]
    fn heroes_go_into_a_level_and_back() {
        use crate::session::bank_heroes;
        let Some(fs) = data() else { return };
        let panels = [
            panel(0, "BOB", 16, None),
            panel(1, "10000K", 0, None),
            panel(2, "ICE600", 4, Some(0)),
        ];
        let mut seen = HeroSave::new("10000K", 0, 1);
        seen.help_disp[15] = crate::save::SEEN_EVER;
        let mut start = LevelStart {
            level: "levelA1".into(),
            difficulty: 1,
            panels: panels.to_vec(),
            ..Default::default()
        };
        start.help_disp[1] = Some(seen.help_disp);
        let (mut g, _) = start.build(&fs).unwrap();
        let mut heroes: Heroes = Default::default();
        heroes[1] = Some(seen);
        let sumner = &g.players[0];
        assert!(sumner.sumner);
        assert_eq!((sumner.level, sumner.gold, sumner.keys), (99, 5000, 9));
        let full = crate::stats::derived(2, crate::stats::atts(16, 99, Default::default()));
        assert_eq!(sumner.derived, full, "999 in every attribute");
        assert_eq!(g.players[1].gold, 10000, "10000K");
        assert_eq!(g.players[2].gold, 0, "a hidden character gets no cheats");
        assert_eq!(g.msgs.help_disp(1)[15], crate::save::SEEN_EVER);
        // Back from the level: unlocks and seen messages go to the heroes.
        g.players[2].class_unlock = 0x100;
        bank_heroes(&g, &panels, &mut heroes);
        assert_eq!(heroes[2].as_ref().unwrap().class_unlock, 0x100);
        assert_eq!(
            heroes[1].as_ref().unwrap().help_disp[15],
            crate::save::SEEN_EVER
        );
        assert_eq!(heroes[0].as_ref().unwrap().class_unlock, 0);
    }

    /// The options block round-trips through the card's GameOpts.
    #[test]
    fn options_round_trip_through_the_card() {
        let mut p = Prefs::default();
        p.sfx_vol = 40;
        p.music_vol = 200;
        p.stereo = false;
        p.difficulty = 2;
        p.compass = true;
        p.multiplayer = 1;
        p.players[0].rumble = false;
        let mut q = Prefs::default();
        q.apply_game_opts(&p.game_opts());
        assert_eq!((q.sfx_vol, q.music_vol, q.stereo), (40, 200, false));
        assert_eq!((q.difficulty, q.compass, q.multiplayer), (2, true, 1));
        // The pads' settings go with the heroes, not the options block.
        assert!(q.players[0].rumble);
        assert_eq!(
            Prefs::default().game_opts().data,
            [128, 128, 1, 0, 0, 1, 0, 0]
        );
    }

    /// Start + X + B alone, held past half a second, resets; anything else
    /// held with it, or letting go, starts the count again.
    #[test]
    fn start_x_b_held_resets() {
        let combo = PlayerInput {
            start: true,
            magic: true,
            turbo: true,
            ..Default::default()
        };
        let mut t = Vec::new();
        let all = [(Source::Keyboard, combo)];
        let ticks = (0..60).take_while(|_| !reset_held(&mut t, &all)).count();
        assert_eq!(ticks, 30, "fires on the tick past 500 ms");
        let mut t = Vec::new();
        let extra = PlayerInput {
            attack: true,
            ..combo
        };
        for _ in 0..120 {
            assert!(!reset_held(&mut t, &[(Source::Keyboard, extra)]));
        }
        for _ in 0..20 {
            reset_held(&mut t, &all);
        }
        reset_held(&mut t, &[(Source::Keyboard, PlayerInput::default())]);
        for _ in 0..25 {
            assert!(!reset_held(&mut t, &all));
        }
    }

    #[test]
    fn no_quit_level_in_the_secret_levels() {
        assert!(quit_level_disabled(Some(0xC02)));
        assert!(!quit_level_disabled(Some(0x102)));
        assert!(!quit_level_disabled(None));
    }
}
