//! Windowed game: winit event loop, wgpu surface, fixed-timestep simulation, and the
//! screens (title -> hero select -> loading -> playing, with a pause menu).

use crate::audio::Audio;
use crate::fs::GameFs;
use crate::game::Game;
use crate::gpu::Gpu;
use crate::hud::{self, Hud, Style};
use crate::input::{Input, PlayerInput, Source};
use crate::menu::{self, Pause, PauseAction, Title};
use crate::scene::{Scene, CLEAR};
use crate::select::{Select, HEROES};
use crate::ui::UiAssets;
use anyhow::{Context, Result};
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
    players: Vec<(String, String)>,
}

/// Health, score, keys and potions carried from one level to the next.
type Carry = Vec<(f32, u32, u32, u32)>;

enum Screen {
    Title(Title),
    Select(Select),
    /// The loading art is shown for one frame, then the level loads.
    Loading {
        level: String,
        players: Vec<(String, String)>,
        carry: Option<Carry>,
        shown: bool,
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
            audio.play_music(&self.cfg.fs, &["tower".into()]);
            (Screen::Title(Title::default()), Vec::new())
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
                players: self.cfg.players.clone(),
                carry: None,
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

/// Display names of the chosen heroes, for the HUD.
fn hero_names(players: &[(String, String)]) -> Vec<String> {
    players
        .iter()
        .map(|(c, _)| {
            HEROES
                .iter()
                .find(|(k, _)| k == c)
                .map(|(_, n)| n.to_string())
                .unwrap_or_else(|| c.clone())
        })
        .collect()
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
            players,
            carry,
            shown: true,
        } = &mut r.screen
        {
            let (level, players, carry) = (level.clone(), players.clone(), carry.take());
            match start_game(fs, &r.gpu, &level, players) {
                Ok(mut p) => {
                    if let Some(carry) = carry {
                        for (a, b) in p.game.players.iter_mut().zip(carry) {
                            a.health = b.0.max(300.0);
                            (a.score, a.keys, a.potions) = (b.1, b.2, b.3);
                        }
                    }
                    log::info!("entering {level}");
                    r.audio.play_level_music(fs, &level);
                    r.screen = Screen::Playing {
                        play: Box::new(p),
                        pause: None,
                    };
                }
                Err(e) => {
                    log::error!("loading {level}: {e:#}");
                    r.screen = Screen::Title(Title::default());
                    r.audio.play_music(fs, &["tower".into()]);
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
                Screen::Title(t) => {
                    if escape {
                        el.exit();
                    }
                    if let Some(src) = t.update(&all, TICK) {
                        next = Some(Screen::Select(Select::with_first(src)));
                    }
                }
                Screen::Select(sel) => {
                    if escape {
                        next = Some(Screen::Title(Title::default()));
                    } else if let Some(players) = sel.update(&r.input, TICK) {
                        r.sources = sel.sources();
                        next = Some(Screen::Loading {
                            level: self.cfg.level.clone(),
                            players,
                            carry: None,
                            shown: false,
                        });
                    }
                }
                Screen::Loading { .. } => {}
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
                                next = Some(Screen::Select(Select::default()));
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
                        // Start (or Esc) on any player's controller pauses.
                        if escape || inputs.iter().any(|i| i.start_pressed) {
                            *pause = Some(Pause::default());
                        } else {
                            play.game.update(TICK, &inputs);
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
                r.audio.play_music(fs, &["tower".into()]);
            }
            r.screen = s;
        }

        // Draw.
        let style = Style { ui: &r.hud.ui };
        let mut in_game = false;
        match &mut r.screen {
            Screen::Title(t) => t.draw(&mut canvas, &style, time),
            Screen::Select(sel) => sel.draw(&mut canvas, &style, time),
            Screen::Loading { level, shown, .. } => {
                menu::draw_loading(&mut canvas, &style, level);
                *shown = true;
            }
            Screen::Playing { play, pause } => {
                in_game = true;
                let p = &mut **play;
                for snd in p.game.sfx.drain(..) {
                    r.audio.play(fs, &snd);
                }
                p.scene.sync(&r.gpu, &p.game);
                let g = p
                    .scene
                    .globals(&p.game, r.config.width as f32 / r.config.height as f32);
                r.gpu
                    .render(&view, &r.depth, &g, CLEAR, &p.scene.draws(&p.game));
                hud::build(&mut canvas, &r.hud.ui, &p.game, &hero_names(&p.players));
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

        // Reaching an exit loads the next level, keeping each player's progress.
        if let Screen::Playing { play, .. } = &mut r.screen {
            if let Some(next) = play.game.exit_to.take() {
                let carry = play
                    .game
                    .players
                    .iter()
                    .map(|p| (p.health, p.score, p.keys, p.potions))
                    .collect();
                if fs.exists(&format!("LEVELS/{next}/WORLDS.PS2")) {
                    r.screen = Screen::Loading {
                        level: next,
                        players: play.players.clone(),
                        carry: Some(carry),
                        shown: false,
                    };
                } else {
                    play.game
                        .messages
                        .push((format!("{next} not available"), 3.0));
                }
            }
        }
        Ok(())
    }
}

fn start_game(
    fs: &GameFs,
    gpu: &Gpu,
    level: &str,
    players: Vec<(String, String)>,
) -> Result<Playing> {
    let classes: Vec<(&str, &str)> = players
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    let game = Game::new(fs, level, &classes)?;
    let scene = Scene::new(gpu, &game)?;
    Ok(Playing {
        game,
        scene,
        players,
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
