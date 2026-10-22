//! Windowed game: winit event loop, wgpu surface, fixed-timestep simulation.

use crate::audio::Audio;
use crate::fs::GameFs;
use crate::game::Game;
use crate::gpu::Gpu;
use crate::hud::{self, Hud};
use crate::input::{Input, Source};
use crate::scene::{Scene, CLEAR};
use crate::select::Select;
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
    /// Players chosen on the command line; empty shows the join screen.
    pub players: Vec<(String, String)>,
    pub mute: bool,
}

struct Playing {
    game: Game,
    scene: Scene,
    players: Vec<(String, String)>,
}

struct Running {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    gpu: Gpu,
    depth: wgpu::TextureView,
    play: Option<Playing>,
    select: Option<Select>,
    hud: Hud,
    audio: Audio,
    input: Input,
    sources: Vec<Source>,
    last: Instant,
    acc: f32,
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
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
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

        let hud = Hud::new(&gpu.device, config.format);
        let mut audio = Audio::new(self.cfg.mute);
        audio.play_level_music(&self.cfg.fs, &self.cfg.level);
        let input = Input::new();
        let (play, select, sources) = if self.cfg.players.is_empty() {
            (None, Some(Select::default()), Vec::new())
        } else {
            let play = start_game(
                &self.cfg.fs,
                &gpu,
                &self.cfg.level,
                self.cfg.players.clone(),
            )?;
            let mut sources: Vec<Source> = input.pads().into_iter().map(Source::Pad).collect();
            sources.truncate(play.game.players.len());
            // The keyboard drives the first player without a gamepad.
            if sources.len() < play.game.players.len() {
                sources.push(Source::Keyboard);
            }
            (Some(play), None, sources)
        };
        Ok(Running {
            window,
            surface,
            config,
            gpu,
            depth,
            play,
            select,
            hud,
            audio,
            input,
            sources,
            last: Instant::now(),
            acc: 0.0,
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
                                KeyCode::Escape => el.exit(),
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
                let now = Instant::now();
                r.acc += (now - r.last).as_secs_f32().min(0.25);
                r.last = now;

                for id in r.input.poll() {
                    // Hot-plugged pad: give it to the keyboard's player if that is the
                    // only one, otherwise ignore (players are fixed for this session).
                    if !r.sources.contains(&Source::Pad(id)) {
                        if let Some(slot) = r.sources.iter().position(|s| *s == Source::Keyboard) {
                            r.sources[slot] = Source::Pad(id);
                        }
                    }
                }
                let frame = match r.surface.get_current_texture() {
                    Ok(f) => f,
                    Err(wgpu::SurfaceError::Outdated | wgpu::SurfaceError::Lost) => {
                        r.surface.configure(&r.gpu.device, &r.config);
                        return;
                    }
                    Err(e) => {
                        log::warn!("surface: {e}");
                        return;
                    }
                };
                let view = frame.texture.create_view(&Default::default());
                let mut canvas = hud::Canvas::new(r.config.width, r.config.height);

                if let Some(sel) = &mut r.select {
                    let mut chosen = None;
                    while r.acc >= TICK {
                        if chosen.is_none() {
                            chosen = sel.update(&r.input, TICK);
                        }
                        r.input.end_frame();
                        r.acc -= TICK;
                    }
                    r.gpu.render(
                        &view,
                        &r.depth,
                        &crate::gpu::globals(glam::Mat4::IDENTITY, glam::Vec3::ZERO, [0.0; 3], 0.0),
                        CLEAR,
                        &[],
                    );
                    sel.draw(
                        &mut canvas,
                        r.last.duration_since(self.started).as_secs_f32(),
                    );
                    r.hud.draw(&r.gpu.device, &r.gpu.queue, &view, &canvas);
                    frame.present();
                    if let Some(players) = chosen {
                        let sources = sel.sources();
                        match start_game(&self.cfg.fs, &r.gpu, &self.cfg.level, players) {
                            Ok(p) => {
                                r.sources = sources;
                                r.play = Some(p);
                                r.select = None;
                            }
                            Err(e) => {
                                self.error = Some(e);
                                el.exit();
                            }
                        }
                    }
                    return;
                }

                let Some(p) = &mut r.play else { return };
                while r.acc >= TICK {
                    let inputs: Vec<_> = r.sources.iter().map(|s| r.input.read(*s)).collect();
                    p.game.update(TICK, &inputs);
                    r.input.end_frame();
                    r.acc -= TICK;
                }

                p.scene.sync(&r.gpu, &p.game);
                let g = p
                    .scene
                    .globals(&p.game, r.config.width as f32 / r.config.height as f32);
                r.gpu
                    .render(&view, &r.depth, &g, CLEAR, &p.scene.draws(&p.game));
                let classes: Vec<String> = p.players.iter().map(|(c, _)| c.clone()).collect();
                hud::build(&mut canvas, &p.game, &classes);
                r.hud.draw(&r.gpu.device, &r.gpu.queue, &view, &canvas);
                frame.present();

                // Reaching an exit loads the next level, keeping each player's progress.
                if let Some(next) = p.game.exit_to.take() {
                    match start_game(&self.cfg.fs, &r.gpu, &next, p.players.clone()) {
                        Ok(mut np) => {
                            for (a, b) in np.game.players.iter_mut().zip(&p.game.players) {
                                a.health = b.health.max(300.0);
                                a.score = b.score;
                                a.keys = b.keys;
                                a.potions = b.potions;
                            }
                            log::info!("entering {next}");
                            r.audio.play_level_music(&self.cfg.fs, &next);
                            *p = np;
                        }
                        Err(e) => {
                            log::error!("loading {next}: {e:#}");
                            p.game.messages.push((format!("{next} not available"), 3.0));
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _el: &ActiveEventLoop) {
        if let Some(r) = &self.run {
            r.window.request_redraw();
        }
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
