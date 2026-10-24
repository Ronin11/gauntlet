//! Windowed game: winit event loop, wgpu surface, fixed-timestep simulation, and the
//! screens (title -> hero select -> loading -> playing, with a pause menu).

use crate::attract::{self, Attract};
use crate::audio::Audio;
use crate::fs::GameFs;
use crate::game::Game;
use crate::game::{LevelEnd, PlayerState};
use crate::gpu::Gpu;
use crate::hud::PanelInfo;
use crate::hud::{self, Hud};
use crate::input::{Input, PlayerInput, Source};
use crate::menu::{self, Title};
use crate::options::{Options, OptionsAction, Prefs};
use crate::progress::Progress;
use crate::save::{HeroSave, MemCard};
use crate::savestate::{self, SaveState};
use crate::scene::{Scene, CLEAR};
use crate::screen::Scr;
use crate::select::{Select, CLASSES, COLOURS};
use crate::ui::UiAssets;
use anyhow::{Context, Result};
use gdl_formats::wdata::TOWER_WAVE;
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
    pub mute: bool,
    /// Where the memory card's save file is kept (None: saves last only
    /// for this run).
    pub save_dir: Option<std::path::PathBuf>,
    /// A debug save state to start in (`--load-state`: a slot or a file).
    pub load_state: Option<String>,
    /// The rules the games play by (classic unless asked otherwise).
    pub rules: crate::rules::Rules,
}

struct Playing {
    game: Game,
    scene: Scene,
    panels: Vec<PanelInfo>,
    /// Seconds played, for fading out the loading art.
    intro: f32,
    /// The boss's health-meter textures (boss levels).
    boss_bank: Option<crate::hud::TexBank>,
}

/// Health, score, keys and potions carried from one level to the next.
/// Indexed by slot (column), so players keep their stats whoever joins.
type Carry = [Option<crate::game::Carried>; 4];

/// Whether a level start refreshes the players' checkpoint (save_backup):
/// start_level skips PlayerSaveState in the secret levels (realm 12), and
/// PlayerSaveState(i, 1) keeps the old backup in waves 5/1 and 6/1.
fn checkpoint_at(wave: Option<u32>) -> bool {
    !matches!(wave, Some(0x501 | 0x601)) && wave.is_none_or(|w| w >> 8 != 12)
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

/// Each column's save image (Player.save), kept between visits to player
/// select.
type Heroes = [Option<HeroSave>; 4];

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

/// player select opened from the tower: the players as they are now.
fn party_in_game(
    play: &Playing,
    card: &MemCard,
    heroes: &Heroes,
    progress: &Progress,
    prefs: &Prefs,
) -> crate::select::Party {
    let mut carry: Carry = Default::default();
    for (info, p) in play.panels.iter().zip(&play.game.players) {
        carry[info.slot % 4] = Some(p.carried());
    }
    let progress = live_progress(play, progress);
    let mut heroes = heroes.clone();
    bank_heroes(&play.game, &play.panels, &mut heroes);
    party_for(card, &Some(carry), &heroes, &progress, prefs)
}

/// The party's progress as it stands in the level being played, with what
/// Sumner has banked in the tower so far: a hero saved from the tower (or the
/// tower re-entered after player select) keeps the speeches already given.
fn live_progress(play: &Playing, progress: &Progress) -> Progress {
    let mut progress = progress.clone();
    progress.crystals = play.game.crystals;
    progress.garg_items = play.game.garg_items;
    progress.shards = play.game.shards;
    progress.runes |= play.game.runes | play.game.boss_runes;
    progress.legend = play.game.legend_held;
    if let Some(sm) = &play.game.sumner {
        (progress.banked_runes, progress.banked_shards) = sm.banked();
        progress.last_beaten = sm.last_beaten();
    }
    progress
}

/// What lives in each player's save image during play goes back to the
/// column's hero: the secret classes unlocked (save.class_unlock) and the
/// messages seen (save.help_disp).
fn bank_heroes(game: &Game, panels: &[PanelInfo], heroes: &mut Heroes) {
    for (i, (info, p)) in panels.iter().zip(&game.players).enumerate() {
        let h = heroes[info.slot % 4]
            .get_or_insert_with(|| HeroSave::new(&info.name, info.class.min(15), info.colour));
        h.class_unlock |= p.class_unlock;
        h.help_disp = game.msgs.help_disp(i);
    }
}

/// A level's heroes as their saves and names make them: playable Sumner's
/// loadout, the messages each has seen, and the name cheats
/// (set_hidden_player from load_player_geo).
fn setup_heroes(game: &mut Game, panels: &[PanelInfo], carry: &Option<Carry>, heroes: &Heroes) {
    for (i, (info, p)) in panels.iter().zip(&mut game.players).enumerate() {
        let slot = info.slot % 4;
        if info.class == crate::stats::SUMNER {
            p.sumner = true;
            let c = carry.as_ref().and_then(|c| c[slot].clone());
            p.restore(&c.unwrap_or_else(crate::game::Carried::sumner));
        } else if info.hidden.is_none() {
            crate::cheats::apply_cheats(&info.name, p);
        }
        if let Some(h) = &heroes[slot] {
            game.msgs.set_help_disp(i, &h.help_disp);
        }
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
        play: Box<Playing>,
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
    /// The loading art is shown for one frame, then the level loads.
    Loading {
        level: String,
        panels: Vec<PanelInfo>,
        shown: bool,
    },
    /// The map screen (init_mapscreen) while the next level "loads", then its
    /// intro movie.
    Map {
        map: Box<crate::mapscreen::MapScreen>,
        movie: String,
        level: String,
        panels: Vec<PanelInfo>,
        /// Back from a secret level: resume the suspended level afterwards.
        resume: bool,
    },
    /// The shop: after a level (then player select and the tower), or from
    /// the Tower Menu (then back to the tower as it was).
    Shop {
        shop: Box<crate::shop::Shop>,
        panels: Vec<PanelInfo>,
        from_menu: bool,
    },
    /// A level's intro movie (init_gamemovie), then the level; or an ending
    /// movie ("garm", "victory"), then `after` (the shop).
    Intro {
        movie: Box<crate::movie::Movie>,
        level: String,
        panels: Vec<PanelInfo>,
        after: Option<Box<Screen>>,
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
    /// The party's realm progress.
    progress: Progress,
    /// The level a secret icon was found in, kept while the secret level runs.
    suspended: Option<Box<Playing>>,
    /// Options settings (volumes, difficulty, controls).
    prefs: Prefs,
    /// Each player's health, gold, keys and potions now, and as they were when
    /// the current level started (restored for players who die in it).
    carry: Option<Carry>,
    checkpoint: Option<Carry>,
    /// The memory card in Slot A, and each column's save image.
    card: MemCard,
    heroes: Heroes,
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
    /// (states/ in it), the state to apply once its level has loaded, and
    /// the level time of the last autosave.
    save_dir: Option<std::path::PathBuf>,
    pending_state: Option<SaveState>,
    autosaved: f32,
    last: Instant,
    acc: f32,
    window: Arc<Window>,
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
            let screen = Screen::Loading {
                level: self.cfg.level.clone(),
                panels: panels_for(&self.cfg.players),
                shown: false,
            };
            (screen, sources)
        };
        // init_prefs, then check_prefs_loaded: the options saved on the card.
        let card = MemCard::open(self.cfg.save_dir.as_deref());
        let mut prefs = Prefs::default();
        if let Some(o) = card.opts() {
            prefs.apply_game_opts(&o);
        }
        audio.set_volumes(&prefs);
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
            progress: Progress::default(),
            prefs,
            suspended: None,
            carry: None,
            checkpoint: None,
            card,
            heroes: Default::default(),
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
            pending_state: None,
            autosaved: 0.0,
            last: Instant::now(),
            acc: 0.0,
            window,
        };
        // --load-state: straight into the saved level.
        if let Some(spec) = &self.cfg.load_state {
            let path = savestate::resolve(spec, r.save_dir.as_deref());
            match SaveState::read(&path) {
                Ok(st) => {
                    log::info!("loading state {}: {}", path.display(), st.summary());
                    r.screen = begin_state(&mut r, st);
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
        if let Some(r) = &self.run {
            autosave(r);
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
        if let Screen::Loading {
            level,
            panels,
            shown: true,
        } = &mut r.screen
        {
            let (level, panels) = (level.clone(), panels.clone());
            match start_game(
                fs,
                &r.gpu,
                &level,
                panels,
                r.prefs.difficulty,
                &self.cfg.rules,
            ) {
                Ok(mut p) => {
                    if let Some(carry) = &r.carry {
                        for (a, info) in p.game.players.iter_mut().zip(&p.panels) {
                            if let Some(b) = &carry[info.slot % 4] {
                                a.restore(b);
                            }
                        }
                    }
                    if checkpoint_at(p.game.wave) || r.checkpoint.is_none() {
                        r.checkpoint = r.carry.clone();
                    }
                    setup_heroes(&mut p.game, &p.panels, &r.carry, &r.heroes);
                    p.game.friendly_fire = r.prefs.multiplayer;
                    p.game.crystals = r.progress.crystals;
                    p.game.garg_items = r.progress.garg_items;
                    p.game.shards = r.progress.shards;
                    p.game.runes = r.progress.runes;
                    // The party's legend weapons; a boss fight may use one.
                    p.game.set_legend_held(r.progress.legend);
                    if let Some(dir) = p.game.boss_model_dir() {
                        let imgs = hud::boss_meter_images(fs, &dir);
                        p.boss_bank = Some(r.hud.load_images(&r.gpu.device, &r.gpu.queue, &imgs));
                    } else if p.game.realm() == Some(12) {
                        // A secret level has no boss: its overlay bank holds the
                        // coin icons.
                        let imgs = hud::coin_images(fs);
                        p.boss_bank = Some(r.hud.load_images(&r.gpu.device, &r.gpu.queue, &imgs));
                    }
                    match p.game.wave {
                        Some(TOWER_WAVE) => {
                            let progress = r.progress.clone();
                            p.game.close_portals(|w| progress.portal_open(w));
                            p.game.enter_tower();
                            p.game.set_start(progress.tower_start());
                            // sumnerUpdatePresence: the welcome is for a new party's
                            // first visit (no experience in any class of any
                            // player's save).
                            let exps =
                                p.game.players.iter().zip(&p.panels).flat_map(|(pl, info)| {
                                    let saved = r.heroes[info.slot % 4]
                                        .iter()
                                        .flat_map(|h| h.classes.iter())
                                        .filter_map(|c| c.carried.as_ref().map(|k| k.exp));
                                    std::iter::once(pl.exp).chain(saved)
                                });
                            let welcome = crate::tower::welcome_due(&progress, exps);
                            r.progress.entered_tower = true;
                            r.progress.welcomed |= welcome;
                            let entry = crate::tower::Entry {
                                welcome,
                                banked_runes: progress.banked_runes,
                                banked_shards: progress.banked_shards,
                                last_beaten: progress.last_beaten,
                            };
                            let sumner = crate::tower::Sumner::new(fs, &mut p.game, entry);
                            p.game.sumner = Some(Box::new(sumner));
                            // Closed portals' glows are hidden from the world mesh.
                            p.scene = Scene::new(&r.gpu, &p.game).unwrap_or(p.scene);
                        }
                        Some(w) => r.progress.last_realm = Some(w >> 8),
                        None => {}
                    }
                    // A debug save state: the level's changes and the party.
                    if let Some(st) = r.pending_state.take() {
                        st.apply(&mut p.game);
                        r.autosaved = p.game.time;
                        log::info!("state applied: {}", st.summary());
                    }
                    log::info!("entering {level}");
                    let streams = p
                        .game
                        .wave
                        .map(|w| p.game.worlds.music(w))
                        .unwrap_or_default();
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
                    r.screen = Screen::Title(Title::default());
                    r.audio.play_select_music(fs);
                }
            }
            // Don't fast-forward the game through the time spent loading.
            r.last = Instant::now();
            r.acc = 0.0;
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
        let mut menu_shop: Option<(crate::shop::Shop, Vec<PanelInfo>)> = None;
        while r.acc >= TICK && next.is_none() {
            let all: Vec<(Source, PlayerInput)> = r
                .input
                .sources()
                .into_iter()
                .map(|s| (s, r.input.read(s)))
                .collect();
            if reset_held(&mut r.reset_hold, &all) {
                // OSResetSystem: the game boots again; only the memory card
                // (and the options) remain.
                r.reset_hold.clear();
                r.audio.stop();
                r.progress = Progress::default();
                r.carry = None;
                r.checkpoint = None;
                r.suspended = None;
                r.heroes = Default::default();
                r.attract = Attract::new();
                menu_shop = None;
                next = Some(Screen::Legal(120));
                escape = false;
                r.input.end_frame();
                r.acc -= TICK;
                continue;
            }
            // The debug menu, while open, has the tick (the game is frozen).
            if let Some(s) = debug_tick(r, fs, &all, escape) {
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
                        Screen::Shop { shop, .. } => {
                            if let Some(m) = shop.music() {
                                r.audio.play_bank_music(fs, &m);
                            }
                        }
                        Screen::Select(_) => r.audio.play_select_music(fs),
                        Screen::Playing { play, .. } => {
                            let streams = play
                                .game
                                .wave
                                .map(|w| play.game.worlds.music(w))
                                .unwrap_or_default();
                            if !r.audio.play_streams(fs, &streams) {
                                r.audio.play_level_music(fs, &play.game.level.name);
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
            r.idle = if !saver_counts(&r.screen) || changed {
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
                    play.game.update(TICK, &[PlayerInput::default()]);
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
                        r.progress = r.debug.new_game_progress();
                        r.carry = None;
                        r.checkpoint = None;
                        r.suspended = None;
                        r.heroes = Default::default();
                        let party = party_for(&r.card, &None, &r.heroes, &r.progress, &r.prefs);
                        next = Some(Screen::Select(Select::with_first(src).with_party(party)));
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
                        r.carry = Some(carry);
                        // A loaded hero's achievements join the party's.
                        sel.merge_progress(&mut r.progress);
                        r.heroes = sel.heroes();
                        r.card = sel.card().clone();
                        r.prefs.players = sel.controls();
                        // Player select always leads into the tower.
                        next = Some(Screen::Loading {
                            level: "levelL1".into(),
                            panels,
                            shown: false,
                        });
                    }
                }
                Screen::Loading { .. } => {}
                Screen::Map {
                    map,
                    movie,
                    level,
                    panels,
                    resume,
                } => {
                    let done = map.update();
                    for snd in map.sfx.drain(..) {
                        r.audio.play(fs, &snd);
                    }
                    if done {
                        r.audio.stop();
                        map.release(&mut r.hud);
                        next = Some(match r.suspended.take().filter(|_| *resume) {
                            // Back where the secret was found, with what was
                            // gained there.
                            Some(mut play) => {
                                if let Some(carry) = &r.carry {
                                    for (p, info) in play.game.players.iter_mut().zip(&play.panels)
                                    {
                                        if let Some(c) = &carry[info.slot % 4] {
                                            p.restore(c);
                                        }
                                    }
                                }
                                play.game.crystals = r.progress.crystals;
                                play.game.garg_items = r.progress.garg_items;
                                play.game.shards = r.progress.shards;
                                for (i, info) in play.panels.iter().enumerate() {
                                    if let Some(h) = &r.heroes[info.slot % 4] {
                                        play.game.msgs.set_help_disp(i, &h.help_disp);
                                    }
                                }
                                let streams = play
                                    .game
                                    .wave
                                    .map(|w| play.game.worlds.music(w))
                                    .unwrap_or_default();
                                r.audio.play_streams(fs, &streams);
                                r.last = Instant::now();
                                Screen::Playing { play, pause: None }
                            }
                            None => intro_or_level(fs, &mut r.audio, movie, level, panels),
                        });
                    }
                }
                Screen::Intro {
                    movie,
                    level,
                    panels,
                    after,
                } => {
                    let inputs: Vec<PlayerInput> = all.iter().map(|(_, i)| *i).collect();
                    if escape || movie.update(TICK, &inputs) {
                        r.audio.stop();
                        next = Some(match after.take() {
                            Some(s) => {
                                if let Screen::Shop { shop, .. } = &*s {
                                    if let Some(m) = shop.music() {
                                        r.audio.play_bank_music(fs, &m);
                                    }
                                }
                                *s
                            }
                            None => Screen::Loading {
                                level: level.clone(),
                                panels: panels.clone(),
                                shown: false,
                            },
                        });
                    }
                }
                Screen::Shop {
                    shop,
                    panels,
                    from_menu,
                } => {
                    let inputs: Vec<PlayerInput> =
                        r.sources.iter().map(|s| r.input.read(*s)).collect();
                    let done = shop.update(&inputs);
                    for snd in shop.sfx.drain(..) {
                        r.audio.play(fs, &snd);
                    }
                    if done {
                        // What was bought goes with each player.
                        let mut carry = r.carry.clone().unwrap_or_default();
                        for (info, c) in panels.iter().zip(shop.players()) {
                            carry[info.slot % 4] = Some(c);
                        }
                        r.carry = Some(carry);
                        shop.release(&mut r.hud);
                        r.audio.stop();
                        next = Some(match r.suspended.take().filter(|_| *from_menu) {
                            Some(mut play) => {
                                if let Some(carry) = &r.carry {
                                    for (p, info) in play.game.players.iter_mut().zip(&play.panels)
                                    {
                                        if let Some(c) = &carry[info.slot % 4] {
                                            p.restore(c);
                                        }
                                    }
                                }
                                r.audio.play_streams(fs, &["tower".into()]);
                                Screen::Playing { play, pause: None }
                            }
                            None => {
                                Screen::Select(Select::from_game(panels, &r.sources).with_party(
                                    party_for(&r.card, &r.carry, &r.heroes, &r.progress, &r.prefs),
                                ))
                            }
                        });
                    }
                }
                Screen::Playing { play, pause } => {
                    let inputs: Vec<PlayerInput> =
                        r.sources.iter().map(|s| r.input.read(*s)).collect();
                    let sumner_busy = play
                        .game
                        .sumner
                        .as_ref()
                        .is_some_and(|s| s.controls_locked());
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
                                    r.progress.hints = h.state;
                                    if let Some(mut sm) = play.game.sumner.take() {
                                        sm.hints_closed(&mut play.game);
                                        play.game.sumner = Some(sm);
                                    }
                                }
                                *pause = None
                            }
                            OptionsAction::VolumeChanged => r.audio.set_volumes(&r.prefs),
                            // "Abort Level?" Yes: everyone is put out of the level
                            // and the party goes back to the tower.
                            OptionsAction::QuitLevel => {
                                autosave_play(play, &r.sources, &r.input, &r.progress, &r.save_dir);
                                *pause = None;
                                for p in &mut play.game.players {
                                    p.state = PlayerState::InTower;
                                }
                                play.game.level_end = Some(LevelEnd::AllDead);
                            }
                            OptionsAction::ManageCharacter => {
                                r.progress = live_progress(play, &r.progress);
                                let party =
                                    party_in_game(play, &r.card, &r.heroes, &r.progress, &r.prefs);
                                next = Some(Screen::Select(
                                    Select::from_game(&play.panels, &r.sources).with_party(party),
                                ));
                            }
                            // TODO: the shop and the inventory screen (shop.c).
                            OptionsAction::Shop | OptionsAction::Inventory => {
                                // init_shop(1 / 2) from the tower: the tower waits.
                                let mode = if action == OptionsAction::Shop {
                                    crate::shop::ShopMode::FromMenu
                                } else {
                                    crate::shop::ShopMode::Inventory
                                };
                                let party: Vec<_> = play
                                    .panels
                                    .iter()
                                    .zip(&play.game.players)
                                    .map(|(info, p)| (info.clone(), p.carried(), p.carried()))
                                    .collect();
                                let shop = crate::shop::Shop::new(
                                    fs, &r.gpu, &mut r.hud, party, None, mode, false,
                                );
                                if let Some(m) = shop.music() {
                                    r.audio.play_bank_music(fs, &m);
                                }
                                *pause = None;
                                menu_shop = Some((shop, play.panels.clone()));
                            }
                            OptionsAction::QuitGame => {
                                autosave_play(play, &r.sources, &r.input, &r.progress, &r.save_dir);
                                r.audio.stop();
                                next = Some(game_over_screen(fs));
                            }
                            OptionsAction::None => {}
                        }
                        if !menu_drawing(pause) {
                            *pause = None;
                        }
                    } else if let Some(b) = &mut play.game.scroll_box {
                        // A scroll being read stops the game until its reader presses B.
                        if b.update(&inputs) {
                            play.game.scroll_box = None;
                        }
                    } else if let Some(src) = (play.game.wave == Some(TOWER_WAVE)
                        && !sumner_busy
                        && r.sources.len() < 4)
                        .then(|| {
                            all.iter()
                                .find(|(s, i)| {
                                    !r.sources.contains(s) && (i.start_pressed || i.attack_pressed)
                                })
                                .map(|(s, _)| *s)
                        })
                        .flatten()
                    {
                        // check_active_players: a new pad joins in the tower, which
                        // opens player select for it (init_player_select(1)).
                        r.progress = live_progress(play, &r.progress);
                        let party = party_in_game(play, &r.card, &r.heroes, &r.progress, &r.prefs);
                        let mut sel = Select::from_game(&play.panels, &r.sources).with_party(party);
                        sel.join(src);
                        next = Some(Screen::Select(sel));
                    } else if let Some(owner) = (0..inputs.len())
                        .find(|&i| {
                            inputs[i].start_pressed
                                && !sumner_busy
                                && play.game.players[i].state.active()
                        })
                        .or(escape.then_some(0))
                    {
                        // OptionsStart: a playing player's Start opens the Game
                        // Menu, or the Tower Menu in the tower.
                        r.audio.play(fs, "S_OPTMENUSEL");
                        let slot = play.panels.get(owner).map_or(owner, |p| p.slot);
                        *pause = Some(if play.game.wave == Some(TOWER_WAVE) {
                            Options::tower(owner).with_slot(slot)
                        } else {
                            let mut o = Options::game(owner).with_slot(slot);
                            // No "Quit Level" in the secret levels (realm 0xC).
                            o.set_quit_level_enabled(!quit_level_disabled(play.game.wave));
                            o
                        });
                    } else {
                        // PlayerControl[i]: each player's Controls options,
                        // kept by panel slot (as select and do_vibe do).
                        for i in 0..4 {
                            let slot = play.panels.get(i).map_or(i, |p| p.slot);
                            play.game.controls[i] = r.prefs.players[slot % 4];
                        }
                        if r.acc < 2.0 * TICK {
                            // interp.rs: the tick being left, if not yet drawn.
                            play.scene.pre_tick(&r.gpu, &play.game);
                        }
                        play.game.debug_god = r.debug.god;
                        play.game.update(TICK, &inputs);
                        r.bugs.observe(&play.game, &inputs);
                        play.intro += TICK;
                        // Debug save states: an autosave every minute of play.
                        if play.game.time < r.autosaved {
                            r.autosaved = play.game.time;
                        } else if play.game.time - r.autosaved >= AUTOSAVE_EVERY {
                            r.autosaved = play.game.time;
                            autosave_play(play, &r.sources, &r.input, &r.progress, &r.save_dir);
                        }
                        // do_vibe: only for players with the Vibration option on.
                        for v in play.game.snd.rumble.drain(..) {
                            let slot = play.panels.get(v.player).map_or(v.player, |p| p.slot);
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
                        let (want, how) = play.game.music_area;
                        if want != r.music_now && r.music_switch != Some(want) {
                            r.music_switch = Some(want);
                        }
                        if let Some(area) = r.music_switch {
                            if how >= 2 || r.music_fade <= 3.0 / 127.0 {
                                if let Some(w) = play.game.wave {
                                    let (streams, n) = play.game.worlds.music_area(w, area);
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
                        for (id, name, vol) in play.game.ambient_levels() {
                            r.audio.ambient(fs, id, name, vol);
                        }
                        // Sumner's lectern: HintMenu.
                        if let Some(pi) = play
                            .game
                            .sumner
                            .as_mut()
                            .and_then(|s| s.hint_request.take())
                        {
                            let hints = crate::hints::Hints::new(
                                r.progress.hints,
                                hint_facts(&r.progress, &play.game),
                                fs.read("TEXT/HINTS_E.ROM")
                                    .ok()
                                    .and_then(|d| gdl_formats::text::TextRom::parse(&d).ok()),
                            );
                            *pause = Some(Options::hint_menu(pi, hints));
                        }
                        if std::mem::take(&mut play.game.world_changed) {
                            if let Ok(sc) = Scene::new(&r.gpu, &play.game) {
                                play.scene = sc;
                            }
                        }
                    }
                }
            }
            escape = false;
            r.input.end_frame();
            r.acc -= TICK;
        }
        if let Some((shop, panels)) = menu_shop.take() {
            let old = std::mem::replace(
                &mut r.screen,
                Screen::Shop {
                    shop: Box::new(shop),
                    panels,
                    from_menu: true,
                },
            );
            if let Screen::Playing { play, .. } = old {
                r.suspended = Some(play);
            }
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
            Screen::Map { map, .. } => map.draw(&mut Scr::new(&mut canvas, &r.hud.ui, ticks)),
            Screen::Shop { shop, .. } => shop.draw(&mut Scr::new(&mut canvas, &r.hud.ui, ticks)),
            Screen::Intro { movie, .. } => {
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
                in_game = true;
                let p = &mut **play;
                for snd in p.game.sfx.drain(..) {
                    crate::bugreport::sound(&snd);
                    r.audio.play(fs, &snd);
                }
                // fn_80052134: the compass hides while the options are open.
                p.game.snd.show_compass = r.prefs.compass && pause.is_none();
                p.scene.sync(&r.gpu, &p.game);
                // Smooth at any refresh: between the last two ticks (interp.rs).
                p.scene.blend(&p.game, r.acc / TICK);
                // The game draws a 4:3 picture; keep it, centred, in any window.
                let (w, h) = (r.config.width as f32, r.config.height as f32);
                let k = (w / 512.0).min(h / 384.0);
                let vp = [
                    (w - 512.0 * k) / 2.0,
                    (h - 384.0 * k) / 2.0,
                    512.0 * k,
                    384.0 * k,
                ];
                let g = p.scene.globals(&p.game, 4.0 / 3.0);
                r.gpu.render_viewport(
                    &view,
                    &r.depth,
                    &g,
                    CLEAR,
                    &p.scene.draws(&p.game),
                    Some(vp),
                    Some(p.scene.sprites()),
                );
                hud::build(
                    &mut Scr::new(&mut canvas, &r.hud.ui, ticks),
                    &p.game,
                    &p.panels,
                );
                hud::build_coins(
                    &mut Scr::new(&mut canvas, &r.hud.ui, ticks),
                    &p.game,
                    &p.panels,
                    p.boss_bank.as_ref(),
                );
                hud::build_boss(
                    &mut Scr::new(&mut canvas, &r.hud.ui, ticks),
                    &p.game,
                    p.boss_bank.as_ref(),
                );
                let fade = menu::loading_fade(p.intro);
                if fade < 255 {
                    menu::transition(&mut Scr::new(&mut canvas, &r.hud.ui, ticks), fade);
                }
                if let Some(b) = &p.game.scroll_box {
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

        // A finished level: tower portals lead to their level (after its intro
        // movie); a level leads back to the tower through player select.
        // A secret icon: the level is kept as it is, to come back to when the
        // secret level's time runs out (SaveAllRecords / LoadAllRecords).
        let secret = match &mut r.screen {
            Screen::Playing { play, .. } => match play.game.level_end {
                Some(LevelEnd::Secret(w)) => Some(Some(w)),
                Some(LevelEnd::SecretOver) => Some(None),
                _ => None,
            },
            _ => None,
        };
        if let Some(to) = secret {
            let Screen::Playing { mut play, .. } =
                std::mem::replace(&mut r.screen, Screen::Legal(0))
            else {
                unreachable!()
            };
            play.game.level_end = None;
            bank_heroes(&play.game, &play.panels, &mut r.heroes);
            if to.is_none() {
                if let Some(bank) = play.boss_bank.take() {
                    r.hud.free_bank(bank);
                }
            }
            r.progress.crystals = play.game.crystals;
            r.progress.garg_items = play.game.garg_items;
            r.progress.shards = play.game.shards;
            r.progress.legend = play.game.legend_held;
            let mut carry: Carry = Default::default();
            for (p, info) in play.game.players.iter().zip(&play.panels) {
                carry[info.slot % 4] = Some(p.carried());
            }
            r.carry = Some(carry);
            let panels = play.panels.clone();
            let map_wave = to.or(play.game.wave).unwrap_or(0xC00);
            let map = crate::mapscreen::MapScreen::new(
                fs,
                &r.gpu,
                &mut r.hud,
                &play.game.worlds,
                map_wave,
            );
            r.audio.stop();
            let level = to.and_then(|w| play.game.worlds.dir(w)).unwrap_or_default();
            if to.is_some() {
                r.suspended = Some(play);
            }
            r.screen = Screen::Map {
                map: Box::new(map),
                movie: String::new(),
                level,
                panels,
                resume: to.is_none(),
            };
            return Ok(());
        }
        if let Screen::Playing { play, .. } = &mut r.screen {
            if let Some(end) = play.game.level_end.take() {
                bank_heroes(&play.game, &play.panels, &mut r.heroes);
                // Crystals are kept whether or not the level was finished.
                r.progress.crystals = play.game.crystals;
                r.progress.garg_items = play.game.garg_items;
                r.progress.shards = play.game.shards;
                r.progress.legend = play.game.legend_held;
                // BossDeath: PlayerGiveRune, the boss's shard (a "rune" in the code).
                r.progress.runes |= play.game.boss_runes | play.game.runes;
                if let Some(bank) = play.boss_bank.take() {
                    r.hud.free_bank(bank);
                }
                // A death on a Guardian's level counts as an attempt at it (more
                // of its hint unlocks).
                if let Some(w) = play.game.wave.filter(|&w| w != TOWER_WAVE) {
                    let last = play
                        .game
                        .worlds
                        .realm(w >> 8)
                        .map_or(0, |r| r.levels.len().saturating_sub(1));
                    if (w & 0xFF) as usize == last {
                        for _ in 0..play.game.deaths {
                            r.progress.record_boss_death(w >> 8);
                        }
                    }
                }
                if let Some(sm) = &play.game.sumner {
                    (r.progress.banked_runes, r.progress.banked_shards) = sm.banked();
                    r.progress.last_beaten = sm.last_beaten();
                }
                let checkpoint = r.checkpoint.clone().unwrap_or_default();
                // Players who died in the level get back what they had at its start.
                let mut carry: Carry = Default::default();
                for (p, info) in play.game.players.iter().zip(&play.panels) {
                    let slot = info.slot % 4;
                    carry[slot] = match (p.state, &checkpoint[slot]) {
                        (PlayerState::InTower, Some(c)) => Some(c.clone()),
                        _ => Some(p.carried()),
                    };
                }
                r.carry = Some(carry);
                // Players who chose "Quit Game" while dead leave the party.
                let mut panels = Vec::new();
                let mut sources = Vec::new();
                for (i, info) in play.panels.iter().enumerate() {
                    if play.game.players.get(i).is_some_and(|p| p.quit) {
                        if let Some(c) = r.carry.as_mut() {
                            c[info.slot % 4] = None;
                        }
                        continue;
                    }
                    panels.push(info.clone());
                    if let Some(s) = r.sources.get(i) {
                        sources.push(*s);
                    }
                }
                r.sources = sources;
                if panels.is_empty() {
                    // Nobody left playing: game over (MG_OVER), then the attract
                    // loop's fail movie.
                    r.audio.stop();
                    r.screen = game_over_screen(fs);
                    return Ok(());
                }
                let wave = play.game.wave;
                let worlds = std::mem::take(&mut play.game.worlds);
                let tower = "levelL1".to_string();
                r.screen = match (wave, end) {
                    (Some(TOWER_WAVE), LevelEnd::Exit(Some(w))) => {
                        let level = worlds.dir(w).unwrap_or(tower);
                        let movie = worlds.level(w).map(|l| l.movie.clone()).unwrap_or_default();
                        let map =
                            crate::mapscreen::MapScreen::new(fs, &r.gpu, &mut r.hud, &worlds, w);
                        match &map.music {
                            Some(m) => r.audio.play_bank_music(fs, m),
                            None => r.audio.stop(),
                        }
                        Screen::Map {
                            map: Box::new(map),
                            movie,
                            level,
                            panels,
                            resume: false,
                        }
                    }
                    (Some(TOWER_WAVE), _) => Screen::Loading {
                        level: tower,
                        panels,
                        shown: false,
                    },
                    // Handled above, before the level is torn down.
                    (_, LevelEnd::Secret(_) | LevelEnd::SecretOver) => unreachable!(),
                    (_, LevelEnd::AllDead) => Screen::Loading {
                        level: tower,
                        panels,
                        shown: false,
                    },
                    (w, LevelEnd::Exit(dest)) => {
                        if let Some(w) = w {
                            let (rune, legend) =
                                worlds.level(w).map_or((0, 0), |l| (l.rune, l.legend));
                            r.progress.record_beaten(w, rune, legend);
                        }
                        match dest.and_then(|d| Some((d, worlds.dir(d)?))) {
                            // E1 -> E2 and F1 -> F2 chain straight on, through the
                            // map screen.
                            Some((d, level)) => {
                                let movie =
                                    worlds.level(d).map(|l| l.movie.clone()).unwrap_or_default();
                                let map = crate::mapscreen::MapScreen::new(
                                    fs, &r.gpu, &mut r.hud, &worlds, d,
                                );
                                match &map.music {
                                    Some(m) => r.audio.play_bank_music(fs, m),
                                    None => r.audio.stop(),
                                }
                                Screen::Map {
                                    map: Box::new(map),
                                    movie,
                                    level,
                                    panels,
                                    resume: false,
                                }
                            }
                            // init_shop(0): the tally and the shop, then player
                            // select and the tower. After Garm, Final Stats.
                            None => {
                                let carry = r.carry.clone().unwrap_or_default();
                                let start = r.checkpoint.clone().unwrap_or_default();
                                let party: Vec<_> = panels
                                    .iter()
                                    .map(|info| {
                                        let now = carry[info.slot % 4]
                                            .clone()
                                            .unwrap_or_else(crate::game::Carried::new_character);
                                        let was =
                                            start[info.slot % 4].clone().unwrap_or(now.clone());
                                        (info.clone(), now, was)
                                    })
                                    .collect();
                                let shop = crate::shop::Shop::new(
                                    fs,
                                    &r.gpu,
                                    &mut r.hud,
                                    party,
                                    w.and_then(|w| worlds.level(w)).cloned(),
                                    crate::shop::ShopMode::AfterLevel,
                                    w == Some(0x803),
                                );
                                let ending = match w {
                                    Some(0x601) => "garm",
                                    Some(0x803) => "victory",
                                    _ => "",
                                };
                                let movie = (!ending.is_empty())
                                    .then(|| crate::movie::Movie::open(fs, ending).ok())
                                    .flatten();
                                match movie {
                                    // After Skorne in the Underworld, "garm"; after
                                    // Garm, "victory"; then the shop (gamemain.c).
                                    Some(mut m) => {
                                        r.audio.stop();
                                        if let Some(a) = m.audio.take() {
                                            r.audio.play_once(
                                                a.pcm,
                                                a.channels as u16,
                                                a.playback_rate,
                                            );
                                        }
                                        Screen::Intro {
                                            movie: Box::new(m),
                                            level: String::new(),
                                            panels: panels.clone(),
                                            after: Some(Box::new(Screen::Shop {
                                                shop: Box::new(shop),
                                                panels,
                                                from_menu: false,
                                            })),
                                        }
                                    }
                                    None => {
                                        match shop.music() {
                                            Some(m) => r.audio.play_bank_music(fs, &m),
                                            None => r.audio.stop(),
                                        }
                                        Screen::Shop {
                                            shop: Box::new(shop),
                                            panels,
                                            from_menu: false,
                                        }
                                    }
                                }
                            }
                        }
                    }
                };
            }
        }
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
    let (game, panels) = match &mut r.screen {
        Screen::Playing { play, .. } => (Some(&mut play.game), Some(&mut play.panels)),
        _ => (None, None),
    };
    let mut ctx = Ctx {
        progress: &mut r.progress,
        game,
        panels,
        heroes: &mut r.heroes,
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
    let dirty = std::mem::take(&mut r.debug.tower_dirty);
    match action {
        Action::None => {
            r.debug.tower_dirty = dirty;
            Some(None)
        }
        Action::Close => {
            let tower = matches!(&r.screen,
                Screen::Playing { play, .. } if play.game.wave == Some(TOWER_WAVE));
            // The tower is reloaded so its portals and barriers follow the
            // progress set.
            Some((dirty && tower).then(|| debug_warp(r, "levelL1".into())))
        }
        Action::Warp(level) => {
            r.debug.close();
            Some(Some(debug_warp(r, level)))
        }
        Action::FileBug(tags) => {
            r.debug.tower_dirty = dirty;
            let res = file_bug(r, &tags);
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
                    Some(Some(begin_state(r, st)))
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
    let Screen::Playing { play, .. } = &r.screen else {
        anyhow::bail!("not in a level");
    };
    write_state(play, &r.sources, &r.input, &r.progress, &r.save_dir, slot)
}

fn write_state(
    play: &Playing,
    sources: &[Source],
    input: &Input,
    progress: &Progress,
    save_dir: &Option<std::path::PathBuf>,
    slot: &str,
) -> Result<std::path::PathBuf> {
    let pads = input.pads();
    let names: Vec<String> = sources
        .iter()
        .map(|s| crate::input::source_name(s, &pads))
        .collect();
    let st = SaveState::capture(&play.game, &play.panels, &names, progress);
    let path = savestate::resolve(slot, save_dir.as_deref());
    st.write(&path)?;
    Ok(path)
}

/// The autosave slot, written when a level is quit (and every minute of
/// play), so `--load-state autosave` picks up after a crash or a rebuild.
fn autosave(r: &Running) {
    if let Screen::Playing { play, .. } = &r.screen {
        autosave_play(play, &r.sources, &r.input, &r.progress, &r.save_dir);
    }
}

fn autosave_play(
    play: &Playing,
    sources: &[Source],
    input: &Input,
    progress: &Progress,
    save_dir: &Option<std::path::PathBuf>,
) {
    // Not a finished level or a party that is all out of it.
    if play.game.level_end.is_some() || play.game.players.iter().all(|p| !p.state.active()) {
        return;
    }
    match write_state(
        play,
        sources,
        input,
        progress,
        save_dir,
        savestate::AUTOSAVE,
    ) {
        Ok(p) => log::info!("autosaved {}", p.display()),
        Err(e) => log::warn!("autosave: {e:#}"),
    }
}

/// Start a debug save state: its party, controllers and progress, then its
/// level (the changes are applied once it has loaded).
fn begin_state(r: &mut Running, st: SaveState) -> Screen {
    r.audio.stop();
    r.suspended = None;
    if let Screen::Playing { mut play, .. } = std::mem::replace(&mut r.screen, Screen::Legal(0)) {
        if let Some(bank) = play.boss_bank.take() {
            r.hud.free_bank(bank);
        }
    }
    r.sources = crate::input::bind_sources(&st.heroes, &r.input.pads(), &r.sources);
    r.progress = st.progress.clone();
    let mut carry: Carry = Default::default();
    for h in &st.heroes {
        carry[h.panel.slot % 4] = Some(h.carried());
    }
    r.carry = Some(carry);
    r.checkpoint = r.carry.clone();
    r.prefs.difficulty = st.difficulty.min(2);
    let panels: Vec<PanelInfo> = st.heroes.iter().map(|h| h.panel.clone()).collect();
    let level = st.level.clone();
    r.pending_state = Some(st);
    Screen::Loading {
        level,
        panels,
        shown: false,
    }
}

/// The debug menu's "File bug report" (bugreport.rs; not in the original).
fn file_bug(r: &Running, tags: &[&'static str]) -> std::result::Result<std::path::PathBuf, String> {
    let root = crate::bugreport::dir().ok_or("no folder for reports")?;
    let shot = match bug_screenshot(r) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("bug report screenshot: {e:#}");
            None
        }
    };
    let (game, panels): (Option<&Game>, &[PanelInfo]) = match &r.screen {
        Screen::Playing { play, .. } => (Some(&play.game), &play.panels),
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
        progress: Some(&r.progress),
        heroes: &r.heroes,
        prefs: Some(&r.prefs),
        card_file: r.card.path(),
        tags,
        screen,
        frames: r.frames,
        via: "debug menu",
    };
    crate::bugreport::file(&root, &report, shot.as_ref()).map_err(|e| format!("{e:#}"))
}

/// The level as last drawn, with its HUD but not the debug menu (1024x768).
fn bug_screenshot(r: &Running) -> Result<Option<gdl_formats::Image>> {
    let Screen::Playing { play, .. } = &r.screen else {
        return Ok(None);
    };
    let p = &**play;
    let (w, h) = (1024, 768);
    let ticks = 100;
    let globals = p.scene.globals(&p.game, 4.0 / 3.0);
    let draw = |view: &wgpu::TextureView| {
        let mut canvas = hud::Canvas::new(w, h);
        hud::build(
            &mut Scr::new(&mut canvas, &r.hud.ui, ticks),
            &p.game,
            &p.panels,
        );
        hud::build_coins(
            &mut Scr::new(&mut canvas, &r.hud.ui, ticks),
            &p.game,
            &p.panels,
            p.boss_bank.as_ref(),
        );
        hud::build_boss(
            &mut Scr::new(&mut canvas, &r.hud.ui, ticks),
            &p.game,
            p.boss_bank.as_ref(),
        );
        if let Some(b) = &p.game.scroll_box {
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
        &p.scene.draws(&p.game),
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
fn debug_warp(r: &mut Running, level: String) -> Screen {
    r.audio.stop();
    r.suspended = None;
    let panels = match std::mem::replace(&mut r.screen, Screen::Legal(0)) {
        Screen::Playing { mut play, .. } => {
            bank_heroes(&play.game, &play.panels, &mut r.heroes);
            let mut carry: Carry = Default::default();
            for (p, info) in play.game.players.iter().zip(&play.panels) {
                carry[info.slot % 4] = Some(p.carried());
            }
            r.carry = Some(carry);
            if let Some(bank) = play.boss_bank.take() {
                r.hud.free_bank(bank);
            }
            play.panels.clone()
        }
        _ => {
            r.sources = vec![r.debug.owner.unwrap_or(Source::Keyboard)];
            r.carry = None;
            r.checkpoint = None;
            panels_for(&[("WAR".into(), "RED".into())])
        }
    };
    Screen::Loading {
        level,
        panels,
        shown: false,
    }
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

fn start_game(
    fs: &GameFs,
    gpu: &Gpu,
    level: &str,
    panels: Vec<PanelInfo>,
    difficulty: usize,
    rules: &crate::rules::Rules,
) -> Result<Playing> {
    // Sumner is the Wizard (character 2) with hidden_code "sum"; a hidden
    // character is its class with its own model.
    let classes: Vec<(&str, &str)> = panels
        .iter()
        .map(|p| {
            (
                if p.class >= 16 {
                    "WIZ"
                } else {
                    CLASSES[p.class]
                },
                COLOURS[p.colour % 4],
            )
        })
        .collect();
    let models: Vec<Option<&str>> = panels
        .iter()
        .map(|p| match p.hidden {
            _ if p.class >= 16 => Some("SUM"),
            Some(h) => crate::cheats::HIDDEN.get(h).map(|h| h.dir),
            None => None,
        })
        .collect();
    let mut game = Game::new_heroes(fs, level, &classes, &models, difficulty)?;
    game.rules = rules.clone();
    let scene = Scene::new(gpu, &game)?;
    Ok(Playing {
        game,
        scene,
        panels,
        intro: 0.0,
        boss_bank: None,
    })
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

/// init_gamemovie: the level's intro movie if it has one, else straight to it.
fn intro_or_level(
    fs: &GameFs,
    audio: &mut Audio,
    movie: &str,
    level: &str,
    panels: &[PanelInfo],
) -> Screen {
    let intro = (!movie.is_empty())
        .then(|| crate::movie::Movie::open(fs, movie).ok())
        .flatten();
    match intro {
        Some(mut m) => {
            if let Some(a) = m.audio.take() {
                audio.play_once(a.pcm, a.channels as u16, a.playback_rate);
            }
            Screen::Intro {
                movie: Box::new(m),
                level: level.to_string(),
                panels: panels.to_vec(),
                after: None,
            }
        }
        None => Screen::Loading {
            level: level.to_string(),
            panels: panels.to_vec(),
            shown: false,
        },
    }
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
                    play: Box::new(Playing {
                        game,
                        scene,
                        panels: Vec::new(),
                        intro: 0.0,
                        boss_bank: None,
                    }),
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
        let Some(fs) = data() else { return };
        let panels = [
            panel(0, "BOB", 16, None),
            panel(1, "10000K", 0, None),
            panel(2, "ICE600", 4, Some(0)),
        ];
        let classes = [("WIZ", "YEL"), ("WAR", "BLU"), ("DWF", "RED")];
        let models = [Some("SUM"), None, Some("GEI")];
        let mut g = Game::new_heroes(&fs, "levelA1", &classes, &models, 1).unwrap();
        let mut heroes: Heroes = Default::default();
        let mut seen = HeroSave::new("10000K", 0, 1);
        seen.help_disp[15] = crate::save::SEEN_EVER;
        heroes[1] = Some(seen);
        setup_heroes(&mut g, &panels, &None, &heroes);
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
