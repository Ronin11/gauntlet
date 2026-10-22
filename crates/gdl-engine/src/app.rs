//! Windowed game: winit event loop, wgpu surface, fixed-timestep simulation.

use crate::audio::Audio;
use crate::fs::GameFs;
use crate::game::Game;
use crate::gpu::Gpu;
use crate::hud::{self, Hud};
use crate::input::{Input, Source};
use crate::scene::{Scene, CLEAR};
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
    pub players: Vec<(String, String)>,
    pub mute: bool,
}

struct Running {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    gpu: Gpu,
    depth: wgpu::TextureView,
    game: Game,
    scene: Scene,
    hud: Hud,
    audio: Audio,
    input: Input,
    sources: Vec<Source>,
    last: Instant,
    acc: f32,
}

struct App {
    cfg: Config,
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

        let classes: Vec<(&str, &str)> = self
            .cfg
            .players
            .iter()
            .map(|(a, b)| (a.as_str(), b.as_str()))
            .collect();
        let game = Game::new(&self.cfg.fs, &self.cfg.level, &classes)?;
        let scene = Scene::new(&gpu, &game)?;
        let hud = Hud::new(&gpu.device, config.format);
        let mut audio = Audio::new(self.cfg.mute);
        audio.play_level_music(&self.cfg.fs, &self.cfg.level);
        let input = Input::new();
        let mut sources: Vec<Source> = input.pads().into_iter().map(Source::Pad).collect();
        sources.truncate(game.players.len());
        // The keyboard drives the first player without a gamepad.
        if sources.len() < game.players.len() {
            sources.push(Source::Keyboard);
        }
        Ok(Running {
            window,
            surface,
            config,
            gpu,
            depth,
            game,
            scene,
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
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    let down = event.state == ElementState::Pressed;
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
                while r.acc >= TICK {
                    let inputs: Vec<_> = r.sources.iter().map(|s| r.input.read(*s)).collect();
                    r.game.update(TICK, &inputs);
                    r.input.end_frame();
                    r.acc -= TICK;
                }

                r.scene.sync(&r.gpu, &r.game);
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
                let g = r
                    .scene
                    .globals(&r.game, r.config.width as f32 / r.config.height as f32);
                r.gpu
                    .render(&view, &r.depth, &g, CLEAR, &r.scene.draws(&r.game));
                let mut canvas = hud::Canvas::new(r.config.width, r.config.height);
                let classes: Vec<String> =
                    self.cfg.players.iter().map(|(c, _)| c.clone()).collect();
                hud::build(&mut canvas, &r.game, &classes);
                r.hud.draw(&r.gpu.device, &r.gpu.queue, &view, &canvas);
                frame.present();

                // Reaching an exit loads the next level, keeping each player's progress.
                if let Some(next) = r.game.exit_to.take() {
                    let classes: Vec<(&str, &str)> = self
                        .cfg
                        .players
                        .iter()
                        .map(|(a, b)| (a.as_str(), b.as_str()))
                        .collect();
                    match Game::new(&self.cfg.fs, &next, &classes) {
                        Ok(mut g) => {
                            for (np, op) in g.players.iter_mut().zip(&r.game.players) {
                                np.health = op.health.max(300.0);
                                np.score = op.score;
                                np.keys = op.keys;
                                np.potions = op.potions;
                            }
                            match Scene::new(&r.gpu, &g) {
                                Ok(sc) => {
                                    log::info!("entering {next}");
                                    r.audio.play_level_music(&self.cfg.fs, &next);
                                    r.game = g;
                                    r.scene = sc;
                                }
                                Err(e) => log::error!("loading {next}: {e:#}"),
                            }
                        }
                        Err(e) => {
                            log::error!("loading {next}: {e:#}");
                            r.game.messages.push((format!("{next} not available"), 3.0));
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

pub fn run(cfg: Config) -> Result<()> {
    let el = EventLoop::new()?;
    el.set_control_flow(ControlFlow::Poll);
    let mut app = App {
        cfg,
        run: None,
        error: None,
    };
    el.run_app(&mut app)?;
    match app.error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}
