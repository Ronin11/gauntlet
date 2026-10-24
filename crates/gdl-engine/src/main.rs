use gdl_engine::{
    app, audio, character, fs, game, gpu, hud, input, mapscreen, menu, options, progress,
    savestate, scene, screen, select, shop, stats, ui,
};

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
    spawn: Option<Vec3>,
    screen: Option<String>,
    size: (u32, u32),
    character: Option<String>,
    /// --modern: a rigged .glb drawn beside --char, in the same pose.
    modern: Option<PathBuf>,
    /// --modern-ref SEQ[:FRAME]: the game pose the glTF's bind pose matches.
    modern_ref: String,
    /// --modern-clip: pose the glTF from a keyframe clip instead of --seq.
    modern_clip: Option<PathBuf>,
    modern_time: f32,
    seq: String,
    frame: f32,
    simulate: u32,
    hold: Vec2,
    attack_at: Vec<u32>,
    block_at: Option<u32>,
    turbo_at: Option<u32>,
    power: Option<f32>,
    yaw: Option<f32>,
    magic_at: Option<u32>,
    kill_boss_at: Option<u32>,
    legend: bool,
    bot: bool,
    mute: bool,
    /// --load-state: a debug save state (slot name or file) to start from.
    load_state: Option<String>,
    /// --save-state / GDL_SAVE_STATE: [frame:]slot-or-file to save during
    /// --simulate (at the end without a frame).
    save_state: Option<String>,
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
  --screen <name>       Snapshot a screen: title, titlemenu, select, loading, pause,
                        select:load|save|result|char (memory card screens),
                        debug[:main|warp|progress|unlocks|party|level|bug|states[:row]]
                        (the developer debug menu over --level),
                        map:<level>[:ticks] (e.g. map:A6:300),
                        shop[:after|menu|inv|final[:ticks[:events]]] with a fake
                        2-player party after A1; events are <tick><key>[<player>]
                        joined by commas, keys a b x u d (e.g. shop:after:900:600a)
  --simulate <frames>   Simulate N 60 Hz frames before the snapshot
                        (GDL_BUGREPORT=<frame>: stop there and file a bug report,
                        GDL_BUGTAGS=stuck,other its tags)
  --hold <x,y>          Stick input held during simulation
  --attack-at <f,...>   Press attack on those frames (a comma list)
  --block-at <frame>    Press B (block) on that frame and hold it after
  --magic-at <frame>    Use a potion on that frame (one is given if none)
  --turbo-at <frame>    Hold turbo (B) around that frame and press attack on it:
                        the turbo move (PWRB at 40+ meter, PWRC at 100)
  --power <n>           Player 0's turbo meter at the start (e.g. 50 or 100)
  --yaw <radians>       Player 0's facing at the start (0 faces +z)
                        (GDL_GENS=1 prints the generators near player 0 as
                        their health changes)
  --bot                 Let a simple bot play every player during simulation
  --kill-boss-at <f>    Boss levels: the boss dies on that frame (its death, the
                        coins and the good wizard follow)
  --legend              Boss levels: the party carries this boss's legend weapon
  --load-state <s>      Start from a debug save state: a slot (1, 2, 3, autosave)
                        or a .state file; works with and without --snapshot
  --save-state [N:]<s>  With --snapshot: save a state on simulated frame N (default:
                        after the simulation) to slot or file <s> (GDL_SAVE_STATE)
  --char CLASS/COLOR    Snapshot a single character (or monster/NAME)
  --modern <glb>        With --char: also draw a rigged glTF in the same pose, beside it
  --modern-ref SEQ[:F]  The game pose the glTF's bind pose matches (default READY:0)
  --modern-clip <glb[:name]>  Pose the glTF from this clip's keyframes at --modern-time seconds
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
        spawn: None,
        screen: None,
        size: (1280, 720),
        character: None,
        modern: None,
        modern_ref: "READY".into(),
        modern_clip: None,
        modern_time: 0.0,
        seq: "READY".into(),
        frame: 0.0,
        simulate: 0,
        hold: Vec2::ZERO,
        attack_at: Vec::new(),
        block_at: None,
        turbo_at: None,
        power: None,
        yaw: None,
        magic_at: None,
        kill_boss_at: None,
        legend: false,
        bot: false,
        mute: std::env::var_os("GDL_MUTE").is_some(),
        load_state: None,
        save_state: std::env::var("GDL_SAVE_STATE")
            .ok()
            .filter(|v| !v.is_empty()),
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
            "--modern" => a.modern = Some(val()?.into()),
            "--modern-ref" => a.modern_ref = val()?,
            "--modern-clip" => a.modern_clip = Some(val()?.into()),
            "--modern-time" => a.modern_time = val()?.parse()?,
            "--seq" => a.seq = val()?,
            "--frame" => a.frame = val()?.parse()?,
            "--simulate" => a.simulate = val()?.parse()?,
            "--attack-at" => {
                a.attack_at = val()?
                    .split(',')
                    .map(|f| f.trim().parse())
                    .collect::<Result<_, _>>()?
            }
            "--block-at" => a.block_at = Some(val()?.parse()?),
            "--magic-at" => a.magic_at = Some(val()?.parse()?),
            "--turbo-at" => a.turbo_at = Some(val()?.parse()?),
            "--power" => a.power = Some(val()?.parse()?),
            "--yaw" => a.yaw = Some(val()?.parse()?),
            "--kill-boss-at" => a.kill_boss_at = Some(val()?.parse()?),
            "--legend" => a.legend = true,
            "--bot" => a.bot = true,
            "--load-state" => a.load_state = Some(val()?),
            "--save-state" => a.save_state = Some(val()?),
            "--mute" => a.mute = true,
            "--hold" => {
                let v = val()?;
                let (x, y) = v.split_once(',').context("--hold x,y")?;
                a.hold = Vec2::new(x.parse()?, y.parse()?);
            }
            "--eye" => a.eye = Some(parse_vec3(&val()?)?),
            "--at" => a.at = Some(parse_vec3(&val()?)?),
            "--spawn" => a.spawn = Some(parse_vec3(&val()?)?),
            "--screen" => a.screen = Some(val()?),
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
    } else if class.eq_ignore_ascii_case("bank") {
        // bank/<DIR>:<TREE>, e.g. bank/ITEMS/levelC5:COIN_GOLD (the rest of
        // the spec after the first '/').
        let rest = &spec[class.len() + 1..];
        let (dir, tree) = rest.split_once(':').context("bank/<dir>:<tree>")?;
        let objects = gdl_formats::Objects::parse(&fs.read(&format!("{dir}/objects.ngc"))?)?;
        let textures = fs.read(&format!("{dir}/textures.ngc"))?;
        let anim = gdl_formats::anim::Anim::parse(&fs.read(&format!("{dir}/ANIM.PS2"))?)?;
        character::Model::from_anim(&objects, &textures, &anim, Some(tree))?
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
    let mut gm = gpu.upload_mesh(&cm.mesh)?;
    // --simulate N: the global texture animations after N game frames.
    gm.animate_textures(args.simulate);
    let mut inst = gpu.new_instance(&gm);
    inst.transform = mirror();
    let seq = cm
        .sequence(&args.seq)
        .with_context(|| format!("no sequence {}", args.seq))?;
    (inst.parts, inst.hidden) = cm.evaluate(seq, args.frame);
    inst.part_fx = cm.texmod_fx(seq, args.frame);

    // --modern: the glTF in the same pose, to the right of the original.
    let modern = match &args.modern {
        Some(path) => {
            let (rs, rf) = args
                .modern_ref
                .split_once(':')
                .unwrap_or((&args.modern_ref, "0"));
            let ref_seq = cm
                .sequence(rs)
                .with_context(|| format!("no sequence {rs} for --modern-ref"))?;
            let hero = gdl_engine::modern::ModernHero::load(path, &cm, ref_seq, rf.parse()?)?;
            let posed = match &args.modern_clip {
                Some(c) => {
                    let spec = c.to_string_lossy();
                    let (file, name) = match spec.rsplit_once(':') {
                        Some((f, n)) => (f, Some(n)),
                        None => (&*spec, None),
                    };
                    let clip = gdl_modern::clip::Clip::load_named(file, hero.model(), name)?;
                    hero.mesh_clip(&clip, args.modern_time)?
                }
                None => hero.mesh(&cm.node_world(seq, args.frame))?,
            };
            let mesh = gpu.upload_mesh(&posed)?;
            let mut inst = gpu.new_instance(&mesh);
            inst.transform = mirror() * Mat4::from_translation(Vec3::new(4.0, 0.0, 0.0));
            Some((mesh, inst))
        }
        None => None,
    };
    let at = args.at.unwrap_or(if modern.is_some() {
        Vec3::new(-2.0, 2.5, 0.0)
    } else {
        Vec3::new(0.0, 2.5, 0.0)
    });
    let eye = args.eye.unwrap_or(Vec3::new(at.x, 3.0, 9.0));
    let (w, h) = args.size;
    let m = mirror();
    let view = Mat4::look_at_rh(m.transform_point3(eye), m.transform_point3(at), Vec3::Y);
    let proj = Mat4::perspective_rh(50f32.to_radians(), w as f32 / h as f32, 0.1, 500.0);
    let g = gpu::globals(proj * view, m.transform_point3(eye), [0.0; 3], 0.0);
    let mut draws = vec![(&gm, &inst)];
    if let Some((m, i)) = &modern {
        draws.push((m, i));
    }
    let img = gpu.render_to_image(w, h, &g, scene::CLEAR, &draws, None, None)?;
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
    let save_dir = gdl_engine::save::MemCard::default_dir();
    let mut g = match &args.load_state {
        Some(spec) => {
            let path = savestate::resolve(spec, save_dir.as_deref());
            let st = savestate::SaveState::read(&path)?;
            println!("loaded state {}: {}", path.display(), st.summary());
            st.load_game(&fs)?
        }
        None => game::Game::new(&fs, &args.level, &classes)?,
    };
    // --save-state [frame:]spec: when, and where to.
    let save_at: Option<(Option<u32>, PathBuf)> = args.save_state.as_ref().map(|v| {
        let (f, spec) = match v.split_once(':') {
            Some((f, s)) if f.parse::<u32>().is_ok() => (f.parse().ok(), s),
            _ => (None, v.as_str()),
        };
        (f, savestate::resolve(spec, save_dir.as_deref()))
    });
    let save_state = |g: &game::Game, path: &PathBuf| -> Result<()> {
        let st = savestate::SaveState::capture(g, &[], &[], &progress::Progress::default());
        st.write(path)?;
        println!("saved state {}: {}", path.display(), st.summary());
        Ok(())
    };
    if std::env::var("GDL_DEBUG_DOORS").is_ok() {
        for e in &g.entities {
            if let game::Kind::Door { .. } | game::Kind::Trigger { .. } = e.kind {
                println!(
                    "{:?} {} pos {:?} yaw {:.2} solid {:?} model {:?}",
                    e.kind, e.id, e.pos, e.yaw, e.solid, e.model
                );
            }
        }
    }
    // GDL_SUMNER=welcome | runes=..,shards=..,banked_runes=..,banked_shards=..,
    // beaten=..,exp=..: the tower with Sumner, as the app would enter it (masks
    // in hex, exp in decimal).
    // GDL_BOX_CLOSE=n presses B n ticks into each message box page.
    if let Ok(spec) = std::env::var("GDL_SUMNER") {
        let mut entry = gdl_engine::tower::Entry::default();
        for kv in spec.split(',') {
            let (k, v) = kv.split_once('=').unwrap_or((kv, "1"));
            let n = u32::from_str_radix(v.trim_start_matches("0x"), 16).unwrap_or(0);
            match k {
                "welcome" => entry.welcome = true,
                "runes" => g.runes = n as u16,
                "shards" => g.shards = n as u16,
                "banked_runes" => entry.banked_runes = n as u16,
                "banked_shards" => entry.banked_shards = n as u16,
                "beaten" => entry.last_beaten = Some(n),
                "exp" => {
                    let n: u32 = v.parse()?;
                    g.players.iter_mut().for_each(|p| {
                        p.exp = n;
                        p.level = stats::exp_to_level(n);
                    })
                }
                "crystals" => g.crystals[1] = n as i32,
                _ => anyhow::bail!("GDL_SUMNER: unknown key {k}"),
            }
        }
        let progress = progress::Progress::default();
        g.close_portals(|w| progress.portal_open(w));
        g.enter_tower();
        g.set_start(progress.tower_start());
        let s = gdl_engine::tower::Sumner::new(&fs, &mut g, entry);
        g.sumner = Some(Box::new(s));
    }
    // After GDL_SUMNER, whose tower start would move the party.
    if let Some(p) = args.spawn {
        g.players[0].pos = p;
        g.cam_focus = p;
    }
    if let Some(y) = args.yaw {
        g.players[0].yaw = y;
    }
    if let Some(n) = args.power {
        g.players[0].power = n;
    }
    let gen_log = std::env::var_os("GDL_GENS").is_some();
    let gens_near = |g: &game::Game| -> Vec<(u64, f32, usize, bool)> {
        let p = g.players[0].pos;
        g.entities
            .iter()
            .filter_map(|e| match &e.kind {
                game::Kind::Generator { hp, strength, .. } if (e.pos - p).length() < 20.0 => {
                    Some((e.id, *hp, *strength, e.alive))
                }
                _ => None,
            })
            .collect()
    };
    let mut gens_seen = gens_near(&g);
    if gen_log {
        for (id, hp, tier, alive) in &gens_seen {
            println!("t=0 generator {id}: hp {hp:.1} tier {tier} alive {alive}");
        }
    }
    // GDL_LEGEND=1: the party holds this realm's Item of Legend.
    if std::env::var_os("GDL_LEGEND").is_some() {
        let realm = g.wave.map_or(0, |w| w >> 8);
        g.set_legend_held(1 << realm);
    }
    // GDL_KEYS=n: player 0 holds n keys (for opening chests and doors).
    if let Some(n) = std::env::var("GDL_KEYS").ok().and_then(|v| v.parse().ok()) {
        g.players[0].keys = n;
    }
    // GDL_INV=1: player 0 holds a 3-way shot and levitation, the inventory
    // open on levitation and switched on.
    if std::env::var_os("GDL_INV").is_some() {
        use gdl_engine::powerups as pu;
        let p = &mut g.players[0];
        p.powerups.add(pu::WEAPON, pu::W_3WAY, 0.0, 45.0);
        p.powerups.add(pu::SPECIAL, pu::S_LEVITATE, 0.0, 30.0);
        p.powerups.slots[1].state = pu::ON;
        p.inv = pu::MiniInv {
            state: 1,
            sel: 1,
            slide: 128,
        };
    }
    // GDL_PUP=<type>:<hex flags>[,...]: player 0 holds these power-ups
    // switched on (e.g. 9:80 the Phoenix, 9:400 Pojo, 6:200000 fire armour);
    // `lvl:<n>` sets every hero's level (the familiars come at 30 and 80).
    if let Ok(v) = std::env::var("GDL_PUP") {
        use gdl_engine::powerups as pu;
        for item in v.split(',').filter(|s| !s.is_empty()) {
            let (k, f) = item.split_once(':').unwrap_or((item, "0"));
            if k == "lvl" {
                let n: u32 = f.parse()?;
                g.players.iter_mut().for_each(|p| {
                    p.level = n.clamp(1, 99);
                    p.refresh_stats();
                });
                continue;
            }
            let kind: u8 = k.parse()?;
            let flags = u32::from_str_radix(f, 16)?;
            let p = &mut g.players[0];
            p.powerups.add(kind, flags, 3.0, 60.0);
            if let Some(sl) = p
                .powerups
                .slots
                .iter_mut()
                .find(|s| s.kind == kind && s.flags == flags)
            {
                sl.state = pu::ON;
            }
        }
    }
    // GDL_CONTROLS=style[,aim,attack]: every hero's Controls options (style
    // 0 Default, 1 Arcade, 2 Robotron; aim / attack 0 off, 1 on).
    if let Ok(v) = std::env::var("GDL_CONTROLS") {
        let f: Vec<usize> = v.split(',').filter_map(|s| s.trim().parse().ok()).collect();
        for c in &mut g.controls {
            c.control_style = f.first().copied().unwrap_or(0).min(2);
            c.auto_aim = f.get(1).is_none_or(|&a| a != 0);
            c.auto_attack = f.get(2).is_none_or(|&a| a != 0);
        }
    }
    // GDL_TRIGGER=id: player 0 starts on the trigger with that id (its
    // camera, shake or ambush follow).
    if let Some(id) = std::env::var("GDL_TRIGGER")
        .ok()
        .and_then(|v| v.parse::<u8>().ok())
    {
        let at = g.entities.iter().find_map(|e| match e.kind {
            gdl_engine::game::Kind::Trigger { id: i, .. } if i == id => Some(e.pos),
            _ => None,
        });
        match (at, g.players.first_mut()) {
            (Some(at), Some(p)) => p.pos = at + Vec3::new(0.0, 0.5, 0.0),
            _ => println!("GDL_TRIGGER: no trigger {id}"),
        }
    }
    // GDL_COMPASS=1: the Compass option on. GDL_XRAY=1: every hero's X-ray on.
    if std::env::var_os("GDL_COMPASS").is_some() {
        g.snd.show_compass = true;
    }
    if std::env::var_os("GDL_XRAY").is_some() {
        use gdl_engine::powerups as pu;
        for p in &mut g.players {
            p.powerups.add(pu::SPECIAL, pu::S_XRAY, 0.0, 600.0);
            if let Some(sl) = p
                .powerups
                .slots
                .iter_mut()
                .find(|s| s.flags & pu::S_XRAY != 0)
            {
                sl.state = pu::ON;
            }
        }
    }
    let box_close: Option<u32> = std::env::var("GDL_BOX_CLOSE")
        .ok()
        .and_then(|v| v.parse().ok());
    if args.legend {
        let realm = g.wave.map_or(0, |w| w >> 8);
        g.set_legend_held(1 << realm);
    }
    // GDL_FLYBY=1: the attract flyby of --level (no heroes, flyby camera).
    let mut flyby = std::env::var_os("GDL_FLYBY").and_then(|_| {
        for p in &mut g.players {
            p.state = game::PlayerState::Exited;
        }
        gdl_engine::flyby::FlybyCam::new(&g.level.worlds)
    });
    let mut sfx_seen: Vec<String> = Vec::new();
    // GDL_BUGREPORT=<frame>: file a bug report as the debug menu would, after
    // that frame (the simulation stops there; the snapshot is its screenshot).
    let bug_at: Option<u32> = std::env::var("GDL_BUGREPORT")
        .ok()
        .and_then(|v| v.parse().ok());
    let simulate = bug_at.map_or(args.simulate, |b| b.min(args.simulate));
    let mut bugs = gdl_engine::bugreport::Recorder::new();
    gdl_engine::bugreport::use_tick_clock();
    for f in 0..simulate {
        if let Some((Some(at), path)) = &save_at {
            if *at == f {
                save_state(&g, path)?;
            }
        }
        if let Some(cam) = &mut flyby {
            if f % 2 == 1 {
                cam.update(2);
            }
            let (eye, at) = cam.view();
            g.fixed_camera = Some((eye, at));
            g.update(1.0 / 60.0, &[input::PlayerInput::default()]);
            g.cam_focus = at;
            g.sfx.clear();
            if f % 300 == 0 {
                println!("flyby t={f} eye {eye:?} done {}", cam.done);
            }
            continue;
        }
        if args.kill_boss_at == Some(f) {
            g.kill_boss();
        }
        if let Some(b) = &mut g.scroll_box {
            let press = box_close.is_some_and(|n| b.ticks + 1 >= n);
            let inp = input::PlayerInput {
                b_pressed: press,
                ..Default::default()
            };
            if b.update(&vec![inp; g.players.len()]) {
                g.scroll_box = None;
            }
            continue;
        }
        if args.magic_at == Some(f) {
            for p in &mut g.players {
                if p.potions == 0 {
                    p.potions = 1;
                    p.potion_colours.push(0);
                }
            }
        }
        let turbo = args.turbo_at.is_some_and(|t| f + 5 >= t && f <= t + 5);
        let turbo_press = args.turbo_at == Some(f);
        let inp = input::PlayerInput {
            stick: args.hold,
            attack: turbo_press,
            attack_pressed: args.attack_at.contains(&f) || turbo_press,
            // --block-at F: press B (block) on frame F and hold it after.
            turbo: turbo || args.block_at.is_some_and(|b| f >= b),
            b_pressed: args.block_at == Some(f),
            magic: args.magic_at == Some(f),
            magic_pressed: args.magic_at == Some(f),
            ..Default::default()
        };
        // GDL_FUZZ=seed: every hero mashes random buttons and sticks.
        let fuzz = std::env::var("GDL_FUZZ")
            .ok()
            .and_then(|v| v.parse::<u64>().ok());
        let inputs: Vec<_> = (0..g.players.len())
            .map(|i| match fuzz {
                Some(seed) => fuzz_input(seed, f, i),
                None if args.bot => g.bot_input(i),
                None => inp,
            })
            .collect();
        // GDL_FX=<row|GENHIT|GENDIE|EXP<type>|BLOCK>[:frame]: start an effect (or
        // a StartExplosion) in front of
        // player 0 (default on the first frame).
        if let Ok(v) = std::env::var("GDL_FX") {
            let (what, at) = v.split_once(':').unwrap_or((&v, "1"));
            if at.parse::<u32>().ok() == Some(f) && !g.debug_fx(what) {
                println!("GDL_FX: no effect {what}");
            }
        }
        let ended = g.level_end.is_some();
        g.update(1.0 / 60.0, &inputs);
        bugs.observe(&g, &inputs);
        for s in &g.sfx {
            gdl_engine::bugreport::sound(s);
        }
        if !ended {
            if let Some(end) = &g.level_end {
                println!("t={f} level end {end:?}");
            }
        }
        sfx_seen.extend(g.sfx.drain(..));
        if gen_log {
            let now = gens_near(&g);
            for (id, hp, tier, alive) in &now {
                if !gens_seen.iter().any(|s| s == &(*id, *hp, *tier, *alive)) {
                    println!(
                        "t={f} generator {id}: hp {hp:.1} tier {tier} alive {alive} (hero act {}, turbo {:.0})",
                        g.players[0].act.cur, g.players[0].power
                    );
                }
            }
            for (id, ..) in &gens_seen {
                if !g.entities.iter().any(|e| e.id == *id) {
                    println!("t={f} generator {id}: destroyed");
                }
            }
            gens_seen = now;
        }
        if f % 600 == 0 {
            let p = &g.players[0];
            println!(
                "t={:4}s hp {:4.0} gold {:5} keys {} lv {} exp {} turbo {:.0} | {}",
                f / 60,
                p.health,
                p.gold,
                p.keys,
                p.level,
                p.exp,
                p.power,
                g.stats()
            );
        }
    }
    if let Some((None, path)) = &save_at {
        save_state(&g, path)?;
    }
    // Debugging aid: GDL_HIDE=OBJ1,OBJ2 leaves world objects out of the render.
    if let Ok(list) = std::env::var("GDL_HIDE") {
        for name in list.split(',') {
            g.hide_object(name);
        }
    }
    if std::env::var_os("GDL_PSYS").is_some() {
        let p = &g.particles;
        let live: usize = p.systems.iter().map(|(_, s)| s.len()).sum();
        println!(
            "psys {} systems, {} particles, {} textures, {} streaks",
            p.systems.len(),
            live,
            p.textures.len(),
            p.streaks.len()
        );
        for (o, s) in p
            .systems
            .iter()
            .filter(|(o, s)| !s.is_empty() || !matches!(o, gdl_engine::psys::Owner::World(_)))
        {
            println!("  {o:?}: {} at {:?}", s.len(), s.node.w_axis.truncate());
        }
    }
    if std::env::var_os("GDL_XRAY").is_some() {
        println!("x-ray {:?}", g.snd.xray);
    }
    if std::env::var_os("GDL_DEBUG_CAM").is_some() {
        let (eye, at, near) = g.camera_view();
        println!("camera eye {eye:?} at {at:?} near {near:.1}");
        if let Some(c) = g.boss.as_ref().and_then(|b| b.cam) {
            println!("boss cam {c:?}");
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
    // GDL_BLEND=alpha: one more tick, drawn `alpha` of the way to it as a
    // faster display would (interp.rs); without it the tick is drawn exactly.
    if let Some(alpha) = std::env::var("GDL_BLEND")
        .ok()
        .and_then(|v| v.parse::<f32>().ok())
    {
        g.update(1.0 / 60.0, &[input::PlayerInput::default()]);
        let t0 = std::time::Instant::now();
        sc.sync(&gpu, &g);
        let t1 = std::time::Instant::now();
        let n = 100;
        for _ in 0..n {
            sc.blend(&g, alpha);
        }
        println!(
            "blend {alpha}: sync {:.3} ms, blend {:.3} ms",
            (t1 - t0).as_secs_f64() * 1e3,
            t1.elapsed().as_secs_f64() * 1e3 / n as f64
        );
    }
    let (w, h) = args.size;
    let mut globals = sc.globals(&g, w as f32 / h as f32);
    if let (Some(eye), Some(at)) = (args.eye, args.at) {
        let m = mirror();
        let view = Mat4::look_at_rh(m.transform_point3(eye), m.transform_point3(at), Vec3::Y);
        let proj = Mat4::perspective_rh(50f32.to_radians(), w as f32 / h as f32, 0.5, 2000.0);
        globals = gpu::globals(proj * view, m.transform_point3(eye), [0.0; 3], 0.0);
    }
    let mut hud = hud::Hud::new(
        &gpu.device,
        &gpu.queue,
        gpu.color_format,
        ui::UiAssets::load(&fs),
    );
    // --screen map:<level code>[:ticks]: the map screen after that many ticks.
    let map_screen = match args.screen.as_deref().and_then(|s| s.strip_prefix("map:")) {
        Some(spec) => {
            let (code, ticks) = spec.split_once(':').unwrap_or((spec, "300"));
            let ticks: u32 = ticks.parse().context("--screen map:<code>:<ticks>")?;
            let wave = g
                .worlds
                .wave_of_dir(code)
                .with_context(|| format!("no level {code} in the realm tables"))?;
            let mut m = mapscreen::MapScreen::new(&fs, &gpu, &mut hud, &g.worlds, wave);
            println!("map screen {code} (wave {wave:#x}), music {:?}", m.music);
            for t in 1..=ticks {
                let done = m.update();
                for s in m.sfx.drain(..) {
                    println!("tick {t}: sfx {s}");
                }
                if done {
                    println!("tick {t}: finished");
                    break;
                }
            }
            Some((m, ticks))
        }
        None => None,
    };
    // --screen shop[:mode[:ticks[:events]]]: the shop after that many ticks.
    let shop_screen = match args.screen.as_deref() {
        Some(spec) if spec == "shop" || spec.starts_with("shop:") => {
            let mut parts = spec.split(':').skip(1);
            let mode_name = parts.next().unwrap_or("after");
            let (mode, final_stats) = match mode_name {
                "after" => (shop::ShopMode::AfterLevel, false),
                "final" => (shop::ShopMode::AfterLevel, true),
                "menu" => (shop::ShopMode::FromMenu, false),
                "inv" => (shop::ShopMode::Inventory, false),
                m => anyhow::bail!("--screen shop: unknown mode {m}"),
            };
            let ticks: u32 = match parts.next() {
                Some(t) => t.parse().context("--screen shop:<mode>:<ticks>")?,
                None => 600,
            };
            // <tick><key>[<player>]
            let mut events: Vec<(u32, char, Option<usize>)> = Vec::new();
            for e in parts
                .next()
                .unwrap_or("")
                .split(',')
                .filter(|e| !e.is_empty())
            {
                let k = e
                    .find(|c: char| c.is_ascii_alphabetic())
                    .context("shop event key")?;
                let t: u32 = e[..k].parse().context("shop event tick")?;
                let key = e[k..].chars().next().unwrap();
                let who = e[k + 1..].parse().ok();
                events.push((t, key, who));
            }
            let level = g
                .worlds
                .wave_of_dir("A1")
                .and_then(|w| g.worlds.level(w))
                .cloned();
            let party = shop::demo_party();
            let mut s = shop::Shop::new(&fs, &gpu, &mut hud, party, level, mode, final_stats);
            if let Some(c) = s.collections.first_mut() {
                c.fangs = 3;
                c.crystals = [15, 40, 0, 0, 0, 0, 0, 0];
                c.legend[1] = true;
                c.legend[9] = true;
                c.runes[2] = true;
            }
            println!("shop {mode_name}, music {:?}", s.music());
            let fuzz = std::env::var("GDL_FUZZ")
                .ok()
                .and_then(|v| v.parse::<u64>().ok());
            for t in 1..=ticks {
                let mut inputs = vec![input::PlayerInput::default(); 2];
                if let Some(seed) = fuzz {
                    inputs = (0..2).map(|p| fuzz_input(seed, t, p)).collect();
                }
                for &(et, key, who) in &events {
                    if et != t {
                        continue;
                    }
                    for (p, i) in inputs.iter_mut().enumerate() {
                        if who.is_some_and(|w| w != p) {
                            continue;
                        }
                        match key {
                            'a' => i.attack_pressed = true,
                            'b' => i.b_pressed = true,
                            'x' => i.magic_pressed = true,
                            'u' => i.stick.y = 1.0,
                            'd' => i.stick.y = -1.0,
                            _ => {}
                        }
                    }
                }
                let done = s.update(&inputs);
                for x in s.sfx.drain(..) {
                    println!("tick {t}: sfx {x}");
                }
                if done {
                    println!("tick {t}: finished");
                    break;
                }
            }
            for (i, c) in s.players().iter().enumerate() {
                println!(
                    "player {i}: gold {} keys {} potions {:?} health {} bonus {:?}",
                    c.gold, c.keys, c.potions, c.health, c.bonus
                );
            }
            Some((s, ticks))
        }
        _ => None,
    };
    let panels: Vec<hud::PanelInfo> = players
        .iter()
        .enumerate()
        .map(|(slot, (c, col))| hud::PanelInfo {
            slot,
            name: ["SAM", "BOB", "ANN", "JO"][slot % 4].into(),
            colour: select::COLOURS
                .iter()
                .position(|k| k == col)
                .unwrap_or(slot % 4),
            class: select::CLASSES.iter().position(|k| k == c).unwrap_or(0),
            hidden: None,
        })
        .collect();
    // --screen debug[:page[:row]]: the debug menu over --level (pages main,
    // warp, progress, unlocks, party, level).
    let debug_menu = match args.screen.as_deref() {
        Some(s) if s == "debug" || s.starts_with("debug:") => {
            use gdl_engine::debugmenu::{realm_table, Ctx, DebugMenu, Page};
            let mut parts = s.split(':').skip(1);
            let page = match parts.next().unwrap_or("main") {
                "main" => Page::Main,
                "warp" => Page::Warp,
                "progress" => Page::Progress,
                "unlocks" => Page::Unlocks,
                "party" => Page::Party,
                "level" => Page::Level,
                "bug" => Page::Bug,
                "states" => Page::States,
                p => anyhow::bail!("--screen debug: unknown page {p}"),
            };
            let row: usize = parts.next().and_then(|n| n.parse().ok()).unwrap_or(0);
            let mut m = DebugMenu::new(realm_table(&g.worlds));
            let mut progress = progress::Progress::default();
            let mut heroes = Default::default();
            let mut compass = false;
            let mut dpanels = panels.clone();
            let mut ctx = Ctx {
                progress: &mut progress,
                game: Some(&mut g),
                panels: Some(&mut dpanels),
                heroes: &mut heroes,
                compass: &mut compass,
            };
            m.show(None, &mut ctx);
            m.goto(page, row, &mut ctx);
            Some(m)
        }
        _ => None,
    };
    let boss_bank = match g.boss_model_dir() {
        Some(d) => Some(hud.load_images(&gpu.device, &gpu.queue, &hud::boss_meter_images(&fs, &d))),
        None if g.wave.is_some_and(|w| w >> 8 == 12) => {
            Some(hud.load_images(&gpu.device, &gpu.queue, &hud::coin_images(&fs)))
        }
        None => None,
    };
    let draw_hud = |view: &wgpu::TextureView| {
        let mut canvas = hud::Canvas::new(w, h);
        hud::build(
            &mut screen::Scr::new(&mut canvas, &hud.ui, 100),
            &g,
            &panels,
        );
        hud::build_coins(
            &mut screen::Scr::new(&mut canvas, &hud.ui, 100),
            &g,
            &panels,
            boss_bank.as_ref(),
        );
        hud::build_boss(
            &mut screen::Scr::new(&mut canvas, &hud.ui, 100),
            &g,
            boss_bank.as_ref(),
        );
        if let Some(b) = &g.scroll_box {
            menu::draw_scroll(
                &mut screen::Scr::new(&mut canvas, &hud.ui, 100),
                b.text(),
                b.scale,
            );
        }
        hud.draw(&gpu.device, &gpu.queue, view, &canvas);
    };
    // Menu screens for checking their layout headlessly.
    // Ticks the options screens have run (GDL_OPTIONS_TICKS overrides).
    let demo_ticks: u32 = std::env::var("GDL_OPTIONS_TICKS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(100);
    let cursor = std::cell::RefCell::new(gdl_engine::cursor::Cursor::load(&fs, &gpu).ok());
    let draw_screen = |view: &wgpu::TextureView| {
        let mut canvas = hud::Canvas::new(w, h);
        let mut arrow = None;
        match args.screen.as_deref() {
            Some("title") => {
                menu::Title::demo(false).draw(&mut screen::Scr::new(&mut canvas, &hud.ui, 100))
            }
            Some("titlemenu") => {
                let t = menu::Title::demo(true);
                let mut s = screen::Scr::new(&mut canvas, &hud.ui, 100);
                t.draw(&mut s);
                arrow = t.cursor(&s);
            }
            Some("select") => {
                select::Select::demo().draw(&mut screen::Scr::new(&mut canvas, &hud.ui, 100))
            }
            // select:load|save|result|char: the memory card screens.
            Some(name) if name.starts_with("select:") => {
                if let Some(sel) = select::Select::demo_card(&name[7..]) {
                    sel.draw(&mut screen::Scr::new(&mut canvas, &hud.ui, 100))
                }
            }
            Some("loading") => menu::draw_loading(&mut screen::Scr::new(&mut canvas, &hud.ui, 40)),
            Some(name) if name.starts_with("gamemenu") || name.starts_with("towermenu") => {
                let mut s = screen::Scr::new(&mut canvas, &hud.ui, 100);
                hud::build(&mut s, &g, &panels);
                if let Some(o) = options::Options::demo(name, demo_ticks) {
                    o.draw(&mut s);
                    arrow = o.cursor(&s);
                }
            }
            Some(name) if name.starts_with("debug") => {
                let mut s = screen::Scr::new(&mut canvas, &hud.ui, 100);
                hud::build(&mut s, &g, &panels);
                if let Some(m) = &debug_menu {
                    m.draw(&mut s);
                }
            }
            // hintmenu[:path]: Sumner's menu with the town open (GDL_HINT_FACTS
            // = boss attempts, e.g. 0x80,0x80 for two deaths at the Lich).
            Some(name) if name.starts_with("hintmenu") => {
                let mut s = screen::Scr::new(&mut canvas, &hud.ui, 100);
                hud::build(&mut s, &g, &panels);
                let mut facts = gdl_engine::hints::HintFacts::default();
                facts.world_open[13] = true;
                facts.world_open[7] = true;
                if let Ok(v) = std::env::var("GDL_HINT_FACTS") {
                    for (i, b) in v.split(',').take(2).enumerate() {
                        facts.boss_attempts[i] =
                            u16::from_str_radix(b.trim_start_matches("0x"), 16).unwrap_or(0);
                    }
                }
                let rom = fs
                    .read("TEXT/HINTS_E.ROM")
                    .ok()
                    .and_then(|d| gdl_formats::text::TextRom::parse(&d).ok());
                let h = gdl_engine::hints::Hints::new(Default::default(), facts, rom);
                let path = name.split_once(':').map_or("", |p| p.1);
                options::Options::hint_menu(0, h)
                    .drive(path, demo_ticks)
                    .draw(&mut s);
            }
            Some(name) if name.starts_with("options") => {
                let mut s = screen::Scr::new(&mut canvas, &hud.ui, 100);
                s.bars(1.0);
                s.quad_screen("TITLE", 0);
                if let Some(o) = options::Options::demo(name, demo_ticks) {
                    o.draw(&mut s);
                    arrow = o.cursor(&s);
                }
            }
            // credits[:scroll[:rz]]: rz as if R and Z were held (the version).
            Some(name) if name.starts_with("credits") => {
                let mut parts = name.split(':').skip(1);
                let scroll = parts.next().and_then(|n| n.parse().ok()).unwrap_or(300);
                let mut a = gdl_engine::attract::Attract::credits_demo(&fs, scroll);
                a.triggers = parts.next() == Some("rz");
                a.draw(&mut screen::Scr::new(&mut canvas, &hud.ui, 100), None);
            }
            // saver[:ticks]: the screen saver's weapons after that many ticks.
            Some(name) if name == "saver" || name.starts_with("saver:") => {
                let ticks = name
                    .strip_prefix("saver:")
                    .and_then(|n| n.parse().ok())
                    .unwrap_or(300);
                let mut s = gdl_engine::screensaver::Saver::new(1);
                for _ in 0..ticks {
                    s.tick(gdl_engine::camera::half_tans());
                }
                if let Ok(mut m) = gdl_engine::screensaver::SaverModels::load(&fs, &gpu) {
                    let k = (w as f32 / 512.0).min(h as f32 / 384.0);
                    let vp = [
                        (w as f32 - 512.0 * k) / 2.0,
                        (h as f32 - 384.0 * k) / 2.0,
                        512.0 * k,
                        384.0 * k,
                    ];
                    m.draw(&s, &gpu, view, &gpu.depth_view(w, h), vp);
                }
                return;
            }
            Some(name) if name == "shop" || name.starts_with("shop:") => {
                if let Some((sh, ticks)) = &shop_screen {
                    sh.draw(&mut screen::Scr::new(&mut canvas, &hud.ui, *ticks));
                }
            }
            Some(s) if s.starts_with("map:") => {
                if let Some((m, ticks)) = &map_screen {
                    m.draw(&mut screen::Scr::new(&mut canvas, &hud.ui, *ticks));
                }
            }
            _ => {}
        }
        hud.draw(&gpu.device, &gpu.queue, view, &canvas);
        if let (Some(at), Some(c)) = (arrow, cursor.borrow_mut().as_mut()) {
            let k = (w as f32 / 512.0).min(h as f32 / 384.0);
            let vp = [
                (w as f32 - 512.0 * k) / 2.0,
                (h as f32 - 384.0 * k) / 2.0,
                512.0 * k,
                384.0 * k,
            ];
            c.draw(&gpu, view, &gpu.depth_view(w, h), vp, at);
        }
    };
    let img = match args.screen.as_deref() {
        Some(name)
            if name.starts_with("gamemenu")
                || name.starts_with("towermenu")
                || name.starts_with("hintmenu")
                || name.starts_with("debug") =>
        {
            gpu.render_to_image(
                w,
                h,
                &globals,
                scene::CLEAR,
                &sc.draws(&g),
                Some(sc.sprites()),
                Some(&draw_screen),
            )?
        }
        Some(_) => {
            gpu.render_to_image(w, h, &globals, scene::CLEAR, &[], None, Some(&draw_screen))?
        }
        None => gpu.render_to_image(
            w,
            h,
            &globals,
            scene::CLEAR,
            &sc.draws(&g),
            Some(sc.sprites()),
            Some(&draw_hud),
        )?,
    };
    if bug_at.is_some() {
        let dir = gdl_engine::bugreport::dir().context("no folder for bug reports")?;
        let card =
            gdl_engine::save::MemCard::default_dir().map(|d| d.join(gdl_engine::save::CARD_FILE));
        let progress = progress::Progress::default();
        let prefs = options::Prefs::default();
        let tags: Vec<&str> = std::env::var("GDL_BUGTAGS")
            .ok()
            .map(|v| {
                gdl_engine::bugreport::TAGS
                    .iter()
                    .copied()
                    .filter(|t| v.split(',').any(|w| w.trim() == *t))
                    .collect()
            })
            .unwrap_or_default();
        let report = gdl_engine::bugreport::Report {
            game: Some(&g),
            panels: &panels,
            progress: Some(&progress),
            heroes: &[None, None, None, None],
            prefs: Some(&prefs),
            card_file: card.as_deref(),
            tags: &tags,
            screen: "playing (headless)",
            frames: simulate as u64,
            via: "GDL_BUGREPORT",
        };
        let out = gdl_engine::bugreport::file(&dir, &report, Some(&img))?;
        println!("bug report {}", out.display());
    }
    write_png(args.snapshot.as_ref().unwrap(), &img)
}

fn main() -> Result<()> {
    // env_logger, with the bug reports' event ring beside it (bugreport.rs).
    let logger = env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info,wgpu_core=warn,wgpu_hal=warn,naga=warn"),
    )
    .build();
    let max = logger.filter();
    gdl_engine::bugreport::install_logger(Box::new(logger), max);
    // The level runner (runner/mod.rs) takes its own arguments.
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if let Some(r) = gdl_engine::runner::main_from_args(&argv) {
        return r;
    }
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
        save_dir: gdl_engine::save::MemCard::default_dir(),
        load_state: args.load_state,
    })
}

/// A pseudo-random input for hero `i` on frame `f` (held for 8 frames).
fn fuzz_input(seed: u64, f: u32, i: usize) -> input::PlayerInput {
    let mut x = seed ^ ((f / 8) as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (i as u64) << 32;
    let mut next = || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    let r = next();
    let edge = f % 8 == 0;
    let bit = |k: u32| r >> k & 1 == 1;
    let stick = Vec2::new(
        ((next() % 200) as f32 / 100.0) - 1.0,
        ((next() % 200) as f32 / 100.0) - 1.0,
    )
    .clamp_length_max(1.0);
    input::PlayerInput {
        stick,
        attack: bit(0),
        attack_pressed: edge && bit(0),
        slow: bit(1),
        slow_pressed: edge && bit(1),
        turbo: bit(2),
        b_pressed: edge && bit(2),
        magic: bit(3) && bit(9),
        magic_pressed: edge && bit(3) && bit(9),
        charge_pressed: edge && bit(4) && bit(10),
        charge: bit(4) && bit(10),
        strafe: bit(5) && bit(11),
        strafe_pressed: edge && bit(5) && bit(11),
        combo: bit(6),
        // The C-stick (it aims under the Robotron style).
        fire: if bit(18) {
            Vec2::new(
                ((next() % 200) as f32 / 100.0) - 1.0,
                ((next() % 200) as f32 / 100.0) - 1.0,
            )
            .clamp_length_max(1.0)
        } else {
            Vec2::ZERO
        },
        inv: gdl_engine::powerups::DPad {
            up: edge && bit(7) && bit(12),
            down: edge && bit(8) && bit(13),
            left: edge && bit(14) && bit(15),
            right: edge && bit(16) && bit(17),
        },
        ..Default::default()
    }
}
