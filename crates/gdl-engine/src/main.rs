use gdl_engine::{app, audio, character, fs, game, gpu, hud, input, scene};

use anyhow::{Context, Result};
use glam::{Mat4, Vec2, Vec3};
use scene::mirror;
use std::path::PathBuf;

struct Args {
    data: Option<PathBuf>,
    level: String,
    players: Vec<(String, String)>,
    snapshot: Option<PathBuf>,
    eye: Option<Vec3>,
    at: Option<Vec3>,
    size: (u32, u32),
    character: Option<String>,
    seq: String,
    frame: f32,
    simulate: u32,
    hold: Vec2,
    attack_at: Option<u32>,
    bot: bool,
    mute: bool,
}

fn parse_vec3(s: &str) -> Result<Vec3> {
    let v: Vec<f32> = s
        .split(',')
        .map(|p| p.trim().parse())
        .collect::<Result<_, _>>()?;
    anyhow::ensure!(v.len() == 3, "expected x,y,z");
    Ok(Vec3::new(v[0], v[1], v[2]))
}

const HELP: &str = "\
gauntlet - Gauntlet Dark Legacy, reimplemented in Rust

USAGE: gauntlet [OPTIONS]

  --data <ISO|DIR>      Game disc image or extracted folder (default: auto-detect,
                        or $GDL_DATA)
  --level <name>        Level to start in (default levelA1)
  --mute                No music or sound (also: GDL_MUTE=1)
  --players <list>      Comma list of CLASS[/COLOR], e.g. WAR,VAL/BLU (default WAR)
                        Classes: WAR VAL WIZ ARC (and unlockables)

Controls: left stick / WASD move, A / J attack, X / Shift turbo,
          F11 fullscreen, Esc quit.

Testing:
  --snapshot <png>      Render one frame headless and exit
  --simulate <frames>   Simulate N 60 Hz frames before the snapshot
  --hold <x,y>          Stick input held during simulation
  --attack-at <frame>   Press attack on that frame
  --bot                 Let a simple bot play every player during simulation
  --char CLASS/COLOR    Snapshot a single character (or monster/NAME)
  --seq NAME --frame F  Animation sequence and frame for --char
  --eye x,y,z --at x,y,z --size WxH   Snapshot camera";

fn parse_args() -> Result<Args> {
    let mut a = Args {
        data: None,
        level: "levelA1".into(),
        players: Vec::new(),
        snapshot: None,
        eye: None,
        at: None,
        size: (1280, 720),
        character: None,
        seq: "READY".into(),
        frame: 0.0,
        simulate: 0,
        hold: Vec2::ZERO,
        attack_at: None,
        bot: false,
        mute: std::env::var_os("GDL_MUTE").is_some(),
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut val = || it.next().with_context(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "--data" => a.data = Some(val()?.into()),
            "--level" => a.level = val()?,
            "--players" => {
                a.players = val()?
                    .split(',')
                    .enumerate()
                    .map(|(i, p)| {
                        let (c, col) = p.split_once('/').unwrap_or((p, game::PLAYER_COLORS[i % 4]));
                        (c.to_ascii_uppercase(), col.to_ascii_uppercase())
                    })
                    .collect();
            }
            "--snapshot" => a.snapshot = Some(val()?.into()),
            "--char" => a.character = Some(val()?),
            "--seq" => a.seq = val()?,
            "--frame" => a.frame = val()?.parse()?,
            "--simulate" => a.simulate = val()?.parse()?,
            "--attack-at" => a.attack_at = Some(val()?.parse()?),
            "--bot" => a.bot = true,
            "--mute" => a.mute = true,
            "--hold" => {
                let v = val()?;
                let (x, y) = v.split_once(',').context("--hold x,y")?;
                a.hold = Vec2::new(x.parse()?, y.parse()?);
            }
            "--eye" => a.eye = Some(parse_vec3(&val()?)?),
            "--at" => a.at = Some(parse_vec3(&val()?)?),
            "--size" => {
                let v = val()?;
                let (w, h) = v.split_once('x').context("--size WxH")?;
                a.size = (w.parse()?, h.parse()?);
            }
            "-h" | "--help" => {
                println!("{HELP}");
                std::process::exit(0);
            }
            other => anyhow::bail!("unknown argument {other} (see --help)"),
        }
    }
    Ok(a)
}

fn open_fs(args: &Args) -> Result<fs::GameFs> {
    match &args.data {
        Some(p) => fs::GameFs::open(p),
        None => fs::GameFs::discover(),
    }
}

fn headless_gpu() -> Result<gpu::Gpu> {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))?;
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))?;
    Ok(gpu::Gpu::new(
        device,
        queue,
        wgpu::TextureFormat::Rgba8Unorm,
    ))
}

fn write_png(path: &PathBuf, img: &gdl_formats::Image) -> Result<()> {
    let f = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut enc = png::Encoder::new(f, img.width, img.height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(&img.rgba)?;
    println!("wrote {}", path.display());
    Ok(())
}

/// Render a single character in a given pose.
fn snapshot_character(args: &Args, spec: &str) -> Result<()> {
    let fs = open_fs(args)?;
    let (class, color) = spec.split_once('/').unwrap_or((spec, "RED"));
    let cm = if class.eq_ignore_ascii_case("monster") {
        let (dir, tree) = color
            .split_once(':')
            .map(|(d, t)| (d, Some(t)))
            .unwrap_or((color, None));
        character::Model::load_monster(&fs, dir, tree)?
    } else {
        character::Model::load_player(&fs, class, color)?
    };
    let names: Vec<_> = cm
        .skeleton
        .sequences
        .iter()
        .map(|s| s.name.as_str())
        .collect();
    log::info!("sequences: {}", names.join(" "));
    let gpu = headless_gpu()?;
    let gm = gpu.upload_mesh(&cm.mesh)?;
    let mut inst = gpu.new_instance(&gm);
    inst.transform = mirror();
    let seq = cm
        .sequence(&args.seq)
        .with_context(|| format!("no sequence {}", args.seq))?;
    (inst.parts, inst.hidden) = cm.evaluate(seq, args.frame);

    let at = args.at.unwrap_or(Vec3::new(0.0, 2.5, 0.0));
    let eye = args.eye.unwrap_or(Vec3::new(0.0, 3.0, 9.0));
    let (w, h) = args.size;
    let m = mirror();
    let view = Mat4::look_at_rh(m.transform_point3(eye), m.transform_point3(at), Vec3::Y);
    let proj = Mat4::perspective_rh(50f32.to_radians(), w as f32 / h as f32, 0.1, 500.0);
    let g = gpu::globals(proj * view, m.transform_point3(eye), [0.0; 3], 0.0);
    let img = gpu.render_to_image(w, h, &g, scene::CLEAR, &[(&gm, &inst)], None)?;
    write_png(args.snapshot.as_ref().unwrap(), &img)
}

/// Run the game headless for a number of frames, then render.
fn snapshot_game(args: &Args) -> Result<()> {
    let fs = open_fs(args)?;
    let players = if args.players.is_empty() {
        vec![("WAR".to_string(), "RED".to_string())]
    } else {
        args.players.clone()
    };
    let classes: Vec<(&str, &str)> = players
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    let mut g = game::Game::new(&fs, &args.level, &classes)?;
    if std::env::var("GDL_DEBUG_DOORS").is_ok() {
        for e in &g.entities {
            if let game::Kind::Door { .. } = e.kind {
                println!(
                    "door {} pos {:?} yaw {:.2} solid {:?} model {:?}",
                    e.id, e.pos, e.yaw, e.solid, e.model
                );
            }
        }
    }
    let mut sfx_seen: Vec<String> = Vec::new();
    for f in 0..args.simulate {
        let inp = input::PlayerInput {
            stick: args.hold,
            attack_pressed: args.attack_at == Some(f),
            ..Default::default()
        };
        let inputs: Vec<_> = (0..g.players.len())
            .map(|i| if args.bot { g.bot_input(i) } else { inp })
            .collect();
        g.update(1.0 / 60.0, &inputs);
        sfx_seen.extend(g.sfx.drain(..));
        if f % 600 == 0 {
            let p = &g.players[0];
            println!(
                "t={:4}s hp {:4.0} score {:5} keys {} | {}",
                f / 60,
                p.health,
                p.score,
                p.keys,
                g.stats()
            );
        }
    }
    for (i, p) in g.players.iter().enumerate() {
        println!(
            "player {i}: pos {:?} yaw {:.2} ground {} seq {} frame {:.1}",
            p.pos,
            p.yaw,
            p.on_ground,
            g.models[p.model].skeleton.sequences[p.animator.seq].name,
            p.animator.frame
        );
    }
    if std::env::var_os("GDL_CHECK_SFX").is_some() {
        sfx_seen.sort();
        sfx_seen.dedup();
        for (req, found) in audio::check_sounds(&fs, &sfx_seen) {
            match found {
                Some((alt, len)) => println!("sfx {req:60} -> {alt} ({len:.2}s)"),
                None => println!("sfx {req:60} -> MISSING"),
            }
        }
    }
    let gpu = headless_gpu()?;
    let mut sc = scene::Scene::new(&gpu, &g)?;
    sc.sync(&gpu, &g);
    let (w, h) = args.size;
    let mut globals = sc.globals(&g, w as f32 / h as f32);
    if let (Some(eye), Some(at)) = (args.eye, args.at) {
        let m = mirror();
        let view = Mat4::look_at_rh(m.transform_point3(eye), m.transform_point3(at), Vec3::Y);
        let proj = Mat4::perspective_rh(50f32.to_radians(), w as f32 / h as f32, 0.5, 2000.0);
        globals = gpu::globals(proj * view, m.transform_point3(eye), [0.0; 3], 0.0);
    }
    let hud = hud::Hud::new(&gpu.device, gpu.color_format);
    let classes: Vec<String> = players.iter().map(|(c, _)| c.clone()).collect();
    let draw_hud = |view: &wgpu::TextureView| {
        let mut canvas = hud::Canvas::new(w, h);
        hud::build(&mut canvas, &g, &classes);
        hud.draw(&gpu.device, &gpu.queue, view, &canvas);
    };
    let img = gpu.render_to_image(w, h, &globals, scene::CLEAR, &sc.draws(&g), Some(&draw_hud))?;
    write_png(args.snapshot.as_ref().unwrap(), &img)
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info,wgpu_core=warn,wgpu_hal=warn,naga=warn"),
    )
    .init();
    let args = parse_args()?;
    if args.snapshot.is_some() {
        return match &args.character {
            Some(c) => snapshot_character(&args, &c.clone()),
            None => snapshot_game(&args),
        };
    }
    let fs = open_fs(&args)?;
    app::run(app::Config {
        fs,
        level: args.level,
        players: args.players,
        mute: args.mute,
    })
}
