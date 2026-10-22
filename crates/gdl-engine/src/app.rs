//! Windowed game: winit event loop, wgpu surface, fixed-timestep simulation, and the
//! screens (title -> hero select -> loading -> playing, with a pause menu).

use crate::attract::{self, Attract};
use crate::audio::Audio;
use crate::fs::GameFs;
use crate::game::Game;
use crate::game::{LevelEnd, PlayerState};
use crate::gpu::Gpu;
use crate::hud::PanelInfo;
use crate::hud::{self, Hud, Style};
use crate::input::{Input, PlayerInput, Source};
use crate::menu::{self, Pause, PauseAction, Title};
use crate::progress::Progress;
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
}

struct Playing {
    game: Game,
    scene: Scene,
    panels: Vec<PanelInfo>,
    /// Seconds played, for fading out the loading art.
    intro: f32,
}

/// Health, score, keys and potions carried from one level to the next.
type Carry = Vec<(f32, u32, u32, u32)>;

enum Screen {
    /// The licence screen shown at power-on, for this many more ticks.
    Legal(u32),
    /// The attract loop (movies, group shots); its state is `Running::attract`.
    Attract,
    Title(Title),
    Select(Select),
    /// The loading art is shown for one frame, then the level loads.
    Loading {
        level: String,
        panels: Vec<PanelInfo>,
        shown: bool,
    },
    /// A level's intro movie (init_gamemovie), then the level.
    Intro {
        movie: Box<crate::movie::Movie>,
        level: String,
        panels: Vec<PanelInfo>,
    },
    Playing {
        play: Box<Playing>,
        pause: Option<Pause>,
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
    /// The party's realm progress.
    progress: Progress,
    /// Each player's health, gold, keys and potions now, and as they were when
    /// the current level started (restored for players who die in it).
    carry: Option<Carry>,
    checkpoint: Option<Carry>,
    /// The current movie frame's texture.
    movie_tex: Option<crate::ui::Tex>,
    hud: Hud,
    audio: Audio,
    input: Input,
    /// Each player's input source, in player order.
    sources: Vec<Source>,
    /// Escape was pressed this frame.
    escape: bool,
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
        Ok(Running {
            surface,
            config,
            gpu,
            depth,
            screen,
            attract: Attract::new(),
            progress: Progress::default(),
            carry: None,
            checkpoint: None,
            movie_tex: None,
            hud,
            audio,
            input,
            sources,
            escape: false,
            last: Instant::now(),
            acc: 0.0,
            window,
        })
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
            match start_game(fs, &r.gpu, &level, panels) {
                Ok(mut p) => {
                    if let Some(carry) = &r.carry {
                        for (a, b) in p.game.players.iter_mut().zip(carry) {
                            a.health = b.0;
                            (a.score, a.keys, a.potions) = (b.1, b.2, b.3);
                        }
                    }
                    r.checkpoint = r.carry.clone();
                    p.game.crystals = r.progress.crystals;
                    match p.game.wave {
                        Some(TOWER_WAVE) => {
                            let progress = r.progress.clone();
                            p.game.close_portals(|w| progress.portal_open(w));
                            p.game.enter_tower();
                            p.game.set_start(progress.tower_start());
                            // Closed portals' glows are hidden from the world mesh.
                            p.scene = Scene::new(&r.gpu, &p.game).unwrap_or(p.scene);
                        }
                        Some(w) => r.progress.last_realm = Some(w >> 8),
                        None => {}
                    }
                    log::info!("entering {level}");
                    let streams = p
                        .game
                        .wave
                        .map(|w| p.game.worlds.music(w))
                        .unwrap_or_default();
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
        while r.acc >= TICK && next.is_none() {
            let all: Vec<(Source, PlayerInput)> = r
                .input
                .sources()
                .into_iter()
                .map(|s| (s, r.input.read(s)))
                .collect();
            match &mut r.screen {
                Screen::Legal(t) => {
                    *t = t.saturating_sub(1);
                    if *t == 0 {
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
                    match ev {
                        attract::Event::MovieStarted => {
                            if let Some(attract::Show::Movie(m)) = &mut r.attract.show {
                                if let Some(a) = m.audio.take() {
                                    r.audio.play_once(a.pcm, a.channels as u16, a.playback_rate);
                                }
                            }
                        }
                        attract::Event::MovieEnded => r.audio.stop(),
                        attract::Event::Title => {
                            r.audio.stop();
                            r.attract.show = None;
                            next = Some(Screen::Title(Title::default()));
                        }
                        attract::Event::None => {}
                    }
                }
                Screen::Title(t) => {
                    if escape {
                        el.exit();
                    }
                    if let Some(src) = t.update(&all) {
                        next = Some(Screen::Select(Select::with_first(src)));
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
                        next = Some(Screen::Title(Title::default()));
                    } else if sel.update(&all).is_some() {
                        r.sources = sel.sources();
                        let panels = sel.panels();
                        // New players start fresh; returning ones keep their stats.
                        let old = r.carry.take().unwrap_or_default();
                        r.carry = Some(
                            panels
                                .iter()
                                .enumerate()
                                .map(|(i, _)| {
                                    old.get(i).copied().unwrap_or((
                                        crate::game::max_health(1),
                                        0,
                                        0,
                                        0,
                                    ))
                                })
                                .collect(),
                        );
                        // Player select always leads into the tower.
                        next = Some(Screen::Loading {
                            level: "levelL1".into(),
                            panels,
                            shown: false,
                        });
                    }
                }
                Screen::Loading { .. } => {}
                Screen::Intro {
                    movie,
                    level,
                    panels,
                } => {
                    let inputs: Vec<PlayerInput> = all.iter().map(|(_, i)| *i).collect();
                    if escape || movie.update(TICK, &inputs) {
                        r.audio.stop();
                        next = Some(Screen::Loading {
                            level: level.clone(),
                            panels: panels.clone(),
                            shown: false,
                        });
                    }
                }
                Screen::Playing { play, pause } => {
                    if let Some(menu) = pause {
                        let action = if escape {
                            Some(PauseAction::Resume)
                        } else {
                            menu.update(&all, TICK)
                        };
                        match action {
                            Some(PauseAction::Resume) => *pause = None,
                            Some(PauseAction::ChangeHeroes) => {
                                next = Some(Screen::Select(Select::from_game(
                                    &play.panels,
                                    &r.sources,
                                )));
                            }
                            Some(PauseAction::QuitToTitle) => {
                                next = Some(Screen::Title(Title::default()));
                            }
                            Some(PauseAction::Quit) => el.exit(),
                            None => {}
                        }
                    } else {
                        let inputs: Vec<PlayerInput> =
                            r.sources.iter().map(|s| r.input.read(*s)).collect();
                        // A scroll being read stops the game until its reader presses B.
                        if let Some(b) = &mut play.game.scroll_box {
                            let inp = inputs.get(b.player).copied().unwrap_or_default();
                            if b.update(&inp) {
                                play.game.scroll_box = None;
                            }
                        } else if escape || inputs.iter().any(|i| i.start_pressed) {
                            // Start (or Esc) on any player's controller pauses.
                            *pause = Some(Pause::default());
                        } else {
                            play.game.update(TICK, &inputs);
                            play.intro += TICK;
                        }
                    }
                }
            }
            escape = false;
            r.input.end_frame();
            r.acc -= TICK;
        }
        if let Some(s) = next {
            if matches!(s, Screen::Title(_) | Screen::Select(_)) {
                r.audio.play_select_music(fs);
            }
            r.screen = s;
        }

        // Draw.
        let style = Style { ui: &r.hud.ui };
        let ticks = (time * 60.0) as u32;
        let mut in_game = false;
        match &mut r.screen {
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
                    r.audio.play(fs, &snd);
                }
                p.scene.sync(&r.gpu, &p.game);
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
                );
                hud::build(
                    &mut Scr::new(&mut canvas, &r.hud.ui, ticks),
                    &p.game,
                    &p.panels,
                );
                let fade = menu::loading_fade(p.intro);
                if fade < 255 {
                    menu::transition(&mut Scr::new(&mut canvas, &r.hud.ui, ticks), fade);
                }
                if let Some(b) = &p.game.scroll_box {
                    menu::draw_scroll(&mut Scr::new(&mut canvas, &r.hud.ui, ticks), &b.text);
                }
                if let Some(menu) = pause {
                    menu.draw(&mut canvas, &style, time);
                }
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
        frame.present();

        // A finished level: tower portals lead to their level (after its intro
        // movie); a level leads back to the tower through player select.
        if let Screen::Playing { play, .. } = &mut r.screen {
            if let Some(end) = play.game.level_end.take() {
                // Crystals are kept whether or not the level was finished.
                r.progress.crystals = play.game.crystals;
                let checkpoint = r.checkpoint.clone().unwrap_or_default();
                // Players who died in the level get back what they had at its start.
                let carry: Carry = play
                    .game
                    .players
                    .iter()
                    .enumerate()
                    .map(|(i, p)| match (p.state, checkpoint.get(i)) {
                        (PlayerState::InTower, Some(c)) => *c,
                        _ => (p.health.max(1.0), p.score, p.keys, p.potions),
                    })
                    .collect();
                r.carry = Some(carry);
                let panels = play.panels.clone();
                let wave = play.game.wave;
                let worlds = std::mem::take(&mut play.game.worlds);
                let tower = "levelL1".to_string();
                r.screen = match (wave, end) {
                    (Some(TOWER_WAVE), LevelEnd::Exit(Some(w))) => {
                        let level = worlds.dir(w).unwrap_or(tower);
                        let intro = worlds
                            .level(w)
                            .filter(|l| !l.movie.is_empty())
                            .and_then(|l| crate::movie::Movie::open(fs, &l.movie).ok());
                        match intro {
                            Some(mut movie) => {
                                if let Some(a) = movie.audio.take() {
                                    r.audio.play_once(a.pcm, a.channels as u16, a.playback_rate);
                                }
                                Screen::Intro {
                                    movie: Box::new(movie),
                                    level,
                                    panels,
                                }
                            }
                            None => Screen::Loading {
                                level,
                                panels,
                                shown: false,
                            },
                        }
                    }
                    (Some(TOWER_WAVE), _) => Screen::Loading {
                        level: tower,
                        panels,
                        shown: false,
                    },
                    (_, LevelEnd::AllDead) => Screen::Loading {
                        level: tower,
                        panels,
                        shown: false,
                    },
                    (w, LevelEnd::Exit(dest)) => {
                        if let Some(w) = w {
                            r.progress.record_beaten(w);
                        }
                        match dest.and_then(|d| worlds.dir(d)) {
                            // E1 -> E2 and F1 -> F2 chain straight on.
                            Some(level) => Screen::Loading {
                                level,
                                panels,
                                shown: false,
                            },
                            // TODO: the shop comes first (init_shop(0)).
                            None => {
                                r.audio.play_select_music(fs);
                                Screen::Select(Select::from_game(&panels, &r.sources))
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
fn panels_for(players: &[(String, String)]) -> Vec<PanelInfo> {
    players
        .iter()
        .enumerate()
        .map(|(slot, (class, colour))| PanelInfo {
            slot,
            name: String::new(),
            colour: COLOURS.iter().position(|c| c == colour).unwrap_or(slot % 4),
            class: CLASSES.iter().position(|c| c == class).unwrap_or(0),
        })
        .collect()
}

fn start_game(fs: &GameFs, gpu: &Gpu, level: &str, panels: Vec<PanelInfo>) -> Result<Playing> {
    // Sumner plays with the Wizard's model.
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
    let game = Game::new(fs, level, &classes)?;
    let scene = Scene::new(gpu, &game)?;
    Ok(Playing {
        game,
        scene,
        panels,
        intro: 0.0,
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
