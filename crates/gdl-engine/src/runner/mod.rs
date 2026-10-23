//! The scripted level runner (a developer tool, not part of the original):
//! a bot that reads the live game state, plays a level headless from its
//! start to its exit at full simulation speed, and reports where and why it
//! failed (a bug report, a save state and a snapshot at the spot).
//!
//! `gauntlet --run-level <levelA1|A|all|levelA1,levelB2> [--runner-party WAR,VAL]
//! [--runner-timeout secs] [--runner-out dir] [--runner-verbose]`
//! `gauntlet --runner-merge <dir>` rebuilds `<dir>/summary.md` from the
//! per-level results (scripts/run-levels.sh runs levels in parallel).

pub mod bot;
pub mod nav;

use crate::fs::GameFs;
use crate::game::Game;
use crate::input::PlayerInput;
use anyhow::{Context, Result};
use bot::{Bot, Outcome};
use std::path::{Path, PathBuf};

pub struct Opts {
    pub levels: Vec<String>,
    pub party: Vec<(String, String)>,
    pub timeout: f32,
    pub out: PathBuf,
    pub verbose: bool,
    pub data: Option<PathBuf>,
    /// Snapshots and bug reports need a GPU; off for quick runs.
    pub capture: bool,
    /// Start from a debug save state instead of the level's start.
    pub state: Option<PathBuf>,
    /// Probe: put hero 0 at a spot and walk it to another, comparing the
    /// game with the nav's walking simulation.
    pub probe: Option<(glam::Vec3, Vec<glam::Vec3>)>,
}

/// One level's result.
pub struct RunResult {
    pub level: String,
    pub party: String,
    pub outcome: Outcome,
    pub game_time: f32,
    pub wall_secs: f64,
    pub objectives: Vec<String>,
    pub god: Option<String>,
    pub boss_debug_kill: bool,
    pub notes: Vec<String>,
    pub reports: Vec<PathBuf>,
}

const HELP: &str = "\
gauntlet --run-level <name|all|realm letter|a,b,c> [options]
  --runner-party WAR,VAL     heroes (default WAR); the 2nd and later follow the 1st
  --runner-timeout <secs>    game seconds per level (default 900)
  --runner-out <dir>         results folder (default runner-out)
  --runner-verbose           print the runner's decisions
  --runner-no-capture        no bug reports / snapshots on failure
  --runner-state <file>      start from a save state (e.g. a report's stuck.state)
  --runner-probe x,y,z:x,y,z walk hero 0 from one spot to another and compare
                             the game with the nav graph's walking simulation
  --data <ISO|DIR>
gauntlet --runner-merge <dir>  rebuild <dir>/summary.md from per-level results";

/// `gauntlet --run-level ...` / `--runner-merge`: returns Some when the
/// arguments were the runner's.
pub fn main_from_args(args: &[String]) -> Option<Result<()>> {
    if let Some(i) = args.iter().position(|a| a == "--runner-merge") {
        let dir = PathBuf::from(args.get(i + 1).map_or("runner-out", |s| s.as_str()));
        return Some(merge(&dir).map(|p| println!("wrote {}", p.display())));
    }
    if !args.iter().any(|a| a == "--run-level") {
        return None;
    }
    Some(parse(args).and_then(|o| run(&o)))
}

fn parse(args: &[String]) -> Result<Opts> {
    let mut o = Opts {
        levels: Vec::new(),
        party: vec![("WAR".into(), "RED".into())],
        timeout: 900.0,
        out: PathBuf::from("runner-out"),
        verbose: false,
        data: None,
        capture: true,
        state: None,
        probe: None,
    };
    let mut it = args.iter();
    let mut spec = String::new();
    while let Some(a) = it.next() {
        let mut val = || it.next().cloned().with_context(|| format!("{a} needs a value"));
        match a.as_str() {
            "--run-level" => spec = val()?,
            "--runner-party" => {
                o.party = val()?
                    .split(',')
                    .enumerate()
                    .map(|(i, p)| {
                        let (c, col) = p
                            .split_once('/')
                            .unwrap_or((p, crate::game::PLAYER_COLORS[i % 4]));
                        (c.to_ascii_uppercase(), col.to_ascii_uppercase())
                    })
                    .collect()
            }
            "--runner-timeout" => o.timeout = val()?.parse()?,
            "--runner-out" => o.out = val()?.into(),
            "--runner-verbose" => o.verbose = true,
            "--runner-no-capture" => o.capture = false,
            "--data" => o.data = Some(val()?.into()),
            "--runner-state" => o.state = Some(val()?.into()),
            "--runner-probe" => {
                let v = val()?;
                let p: Vec<f32> = v
                    .split([',', ':'])
                    .map(|x| x.trim().parse())
                    .collect::<Result<_, _>>()?;
                anyhow::ensure!(p.len() >= 6 && p.len() % 3 == 0, "--runner-probe x,y,z:x,y,z[:x,y,z...]");
                let v: Vec<glam::Vec3> = p.chunks(3).map(|c| glam::Vec3::new(c[0], c[1], c[2])).collect();
                o.probe = Some((v[0], v[1..].to_vec()));
            }
            "--mute" => {}
            "-h" | "--help" => {
                println!("{HELP}");
                std::process::exit(0);
            }
            other => anyhow::bail!("--run-level: unknown argument {other}\n{HELP}"),
        }
    }
    o.levels = spec.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
    anyhow::ensure!(!o.levels.is_empty(), "--run-level needs a level\n{HELP}");
    Ok(o)
}

/// Every real level folder (not ORIGlevelL1 or DEMO1), in realm order.
pub fn all_levels(fs: &GameFs) -> Vec<String> {
    let mut v: Vec<String> = fs
        .list_dirs("LEVELS")
        .into_iter()
        .filter(|n| n.starts_with("level"))
        .collect();
    v.sort();
    v.dedup();
    v
}

fn run(o: &Opts) -> Result<()> {
    let fs = match &o.data {
        Some(p) => GameFs::open(p)?,
        None => GameFs::discover()?,
    };
    let all = all_levels(&fs);
    let mut levels = Vec::new();
    for s in &o.levels {
        if s == "all" {
            levels.extend(all.iter().cloned());
        } else if s.len() == 1 {
            let pre = format!("level{}", s.to_ascii_uppercase());
            levels.extend(all.iter().filter(|l| l.starts_with(&pre)).cloned());
        } else if s.starts_with("level") {
            levels.push(s.clone());
        } else {
            levels.push(format!("level{}", s.to_ascii_uppercase()));
        }
    }
    std::fs::create_dir_all(&o.out)?;
    crate::bugreport::use_tick_clock();
    for level in &levels {
        let t0 = std::time::Instant::now();
        let fsr = &fs;
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_level(fsr, o, level)));
        let r = match res {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => RunResult::failed(level, o, "load_error", format!("{e:#}"), t0),
            Err(p) => {
                let msg = p
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_else(|| "panic".into());
                RunResult::failed(level, o, "crash", format!("panic: {msg}"), t0)
            }
        };
        println!(
            "{level}: {} {} ({:.0} s game, {:.1} s wall){}",
            r.outcome_tag(),
            r.outcome_detail(),
            r.game_time,
            r.wall_secs,
            r.god.as_ref().map_or(String::new(), |g| format!(" [god mode: {g}]"))
        );
        write_result(&o.out, &r)?;
    }
    let p = merge(&o.out)?;
    println!("summary {}", p.display());
    Ok(())
}

impl RunResult {
    fn failed(level: &str, o: &Opts, tag: &str, msg: String, t0: std::time::Instant) -> RunResult {
        RunResult {
            level: level.into(),
            party: party_name(&o.party),
            outcome: Outcome::Stuck(format!("{tag}: {msg}")),
            game_time: 0.0,
            wall_secs: t0.elapsed().as_secs_f64(),
            objectives: Vec::new(),
            god: None,
            boss_debug_kill: false,
            notes: vec![format!("{tag}")],
            reports: Vec::new(),
        }
    }
    fn outcome_tag(&self) -> String {
        if let Some(t) = self.notes.first().filter(|n| *n == "crash" || *n == "load_error") {
            return t.clone();
        }
        let mut t = self.outcome.tag().to_string();
        if matches!(self.outcome, Outcome::Completed) && self.god.is_some() {
            t = "completed_god".into();
        }
        if matches!(self.outcome, Outcome::Completed) && self.boss_debug_kill {
            t = "completed_debugkill".into();
        }
        t
    }
    fn outcome_detail(&self) -> String {
        self.outcome.detail()
    }
}

fn party_name(p: &[(String, String)]) -> String {
    p.iter().map(|(c, _)| c.as_str()).collect::<Vec<_>>().join("+")
}

/// Play one level to its end.
pub fn run_level(fs: &GameFs, o: &Opts, level: &str) -> Result<RunResult> {
    let t0 = std::time::Instant::now();
    let classes: Vec<(&str, &str)> = o.party.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let mut g = match &o.state {
        Some(p) => crate::savestate::SaveState::read(p)?.load_game(fs)?,
        None => Game::new(fs, level, &classes)?,
    };
    if let Some((from, tos)) = &o.probe {
        for to in tos {
            probe(&mut g, *from, *to);
        }
        anyhow::bail!("probe done");
    }
    let tower = g.wave == Some(gdl_formats::wdata::TOWER_WAVE);
    if tower {
        let progress = crate::progress::Progress::default();
        g.close_portals(|w| progress.portal_open(w));
        g.enter_tower();
        g.set_start(progress.tower_start());
        let s = crate::tower::Sumner::new(fs, &mut g, crate::tower::Entry::default());
        g.sumner = Some(Box::new(s));
    }
    let level_dir = o.out.join(level).join(party_name(&o.party));
    let _ = std::fs::remove_dir_all(&level_dir);
    std::fs::create_dir_all(&level_dir)?;
    let mut bot = Bot::new(&g, o.timeout, tower, o.verbose);
    let mut rec = crate::bugreport::Recorder::new();
    let mut reports = Vec::new();
    let mut notes = Vec::new();
    let mut captured = 0;
    let n = g.players.len();
    let dbg_from: Option<f32> = std::env::var("GDL_RUNNER_DEBUG").ok().and_then(|v| v.parse().ok());
    let save_at: Option<(f32, String)> = std::env::var("GDL_RUNNER_SAVE").ok().and_then(|v| {
        let (t, f) = v.split_once(':')?;
        Some((t.parse().ok()?, f.to_string()))
    });
    let outcome = loop {
        // A scroll's text box pauses the game until B.
        if let Some(b) = &mut g.scroll_box {
            let press = b.ticks >= 20;
            let inp = PlayerInput {
                b_pressed: press,
                ..Default::default()
            };
            if b.update(&vec![inp; n]) {
                g.scroll_box = None;
            }
            continue;
        }
        let inputs = match bot.inputs(&mut g) {
            Ok(i) => i,
            Err(out) => break out,
        };
        // Boss levels: past the time limit with god mode, the debug kill
        // (recorded), so the wizard and the level's end still get checked.
        if g.boss.as_ref().is_some_and(|b| !b.dead && !b.dying)
            && !bot.boss_killed_debug
            && (bot.boss_fight_t > 300.0 || g.time > o.timeout * 0.75)
        {
            bot.boss_killed_debug = true;
            let msg = format!(
                "boss not beaten after {:.0} s of fighting (t={:.0}s): debug kill",
                bot.boss_fight_t, g.time
            );
            notes.push(msg.clone());
            crate::bugreport::event("runner", msg);
            if o.capture && captured < 4 {
                captured += 1;
                if let Ok(p) = capture(fs, &g, &mut bot, o, &level_dir, "boss not beaten", &["other"]) {
                    reports.push(p);
                }
            }
            g.kill_boss();
        }
        if dbg_from.is_some_and(|t| g.time >= t) && g.frame % 30 == 0 {
            let p = &g.players[0];
            println!(
                "dbg t={:.1} pos {:?} stick {:?} act {} motion {} transport {:?} touch {:?} state {:?}",
                g.time, p.pos, inputs[0].stick, p.act.cur, p.act.motion,
                g.transport.heroes.first(), p.exit.touch, p.state
            );
        }
        // GDL_RUNNER_SAVE=<secs>:<file>: a save state at that game time.
        if let Some((t, f)) = save_at.as_ref() {
            if g.time >= *t && g.time - 1.0 / 60.0 < *t {
                let st = crate::savestate::SaveState::capture(&g, &[], &[], &[], &Default::default());
                st.write(Path::new(f))?;
                println!("saved state {f} at t={:.1}", g.time);
            }
        }
        g.update(1.0 / 60.0, &inputs);
        rec.observe(&g, &inputs);
        for s in g.sfx.drain(..) {
            crate::bugreport::sound(&s);
        }
        // Anomalies found on the way.
        let new: Vec<_> = bot.anomalies.drain(..).collect();
        for a in new {
            notes.push(a.what.clone());
            if a.capture && o.capture && captured < 4 {
                captured += 1;
                if let Ok(p) = capture(fs, &g, &mut bot, o, &level_dir, &a.what, &a.tags) {
                    reports.push(p);
                }
            }
        }
        if let Some(out) = bot.check_end(&g) {
            break out;
        }
    };
    let failed = !matches!(outcome, Outcome::Completed | Outcome::Secret(_));
    if failed && o.capture {
        let tags: &[&str] = match outcome {
            Outcome::Stuck(_) | Outcome::Timeout(_) => &["stuck"],
            Outcome::Unreachable(_) => &["can't reach"],
            Outcome::Nan(_) => &["crash-ish"],
            _ => &["other"],
        };
        let what = format!("{}: {}", outcome.tag(), outcome.detail());
        match capture(fs, &g, &mut bot, o, &level_dir, &what, tags) {
            Ok(p) => reports.push(p),
            Err(e) => notes.push(format!("capture failed: {e:#}")),
        }
    }
    if let Some(l) = bot.lectern_ok {
        notes.push(format!("lectern: {}", if l { "hint menu opened" } else { "FAILED" }));
    }
    if !bot.portals_reachable.is_empty() {
        notes.push(format!("portals: {}", bot.portals_reachable.join(" ")));
    }
    std::fs::write(level_dir.join("runner.log"), bot.note(&g))?;
    crate::bugreport::write_png(&level_dir.join("navmap.png"), &bot.nav_map(&g))?;
    Ok(RunResult {
        level: level.into(),
        party: party_name(&o.party),
        outcome,
        game_time: g.time,
        wall_secs: t0.elapsed().as_secs_f64(),
        objectives: bot.used.clone(),
        god: bot.god_at.as_ref().map(|(t, w)| format!("t={t:.0}s {w}")),
        boss_debug_kill: bot.boss_killed_debug,
        notes,
        reports,
    })
}

/// Walk hero 0 from `from` to `to` in the game and in the nav simulation.
fn probe(g: &mut Game, from: glam::Vec3, to: glam::Vec3) {
    let ignore = std::collections::HashSet::new();
    g.players[0].pos = from;
    g.update(1.0 / 60.0, &vec![PlayerInput::default(); g.players.len()]);
    let start = g.players[0].pos;
    let w = nav::WalkWorld {
        col: &g.collision,
        ents: &g.entities,
        ignore: &ignore,
        min_y: g.level.worlds.min_bounds[1],
    };
    // GDL_NAV_NODES=r: the nav nodes within r of the target and their edges.
    if let Some(r) = std::env::var("GDL_NAV_NODES").ok().and_then(|v| v.parse::<f32>().ok()) {
        let mut nv = nav::Nav::new(g);
        let ids: Vec<u32> = (0..nv.nodes.len() as u32)
            .filter(|&n| {
                let p = nv.nodes[n as usize].pos;
                nv.nodes[n as usize].alive && nav::xz(p - to).length() < r && (p.y - to.y).abs() < 4.0
            })
            .collect();
        for n in ids {
            let p = nv.nodes[n as usize].pos;
            let e: Vec<String> = nv
                .edges(g, n)
                .iter()
                .map(|e| {
                    let q = nv.nodes[e.to as usize].pos;
                    format!("({:.1},{:.1},{:.1})", q.x, q.y, q.z)
                })
                .collect();
            println!("  node {n} ({:.2},{:.2},{:.2}) -> {}", p.x, p.y, p.z, e.join(" "));
        }
    }
    // GDL_NAV_HMAP=x0,z0,x1,z1: an ASCII map of the floor heights (the
    // highest floor at most 2 above `from`, in units, '.' none), north up.
    if let Ok(v) = std::env::var("GDL_NAV_HMAP") {
        let r: Vec<f32> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
        if r.len() == 4 {
            let mut z = r[3];
            while z >= r[1] {
                let mut line = format!("{z:7.1} ");
                let mut x = r[0];
                while x <= r[2] {
                    let p = glam::Vec3::new(x, from.y + 2.0, z);
                    line.push(match g.collision.ground_height(p, from.y + 2.0) {
                        Some(y) if (y - from.y).abs() < 0.6 => '=',
                        Some(y) if y > from.y => '+',
                        Some(y) if y > from.y - 3.0 => '-',
                        Some(_) => '_',
                        None => '.',
                    });
                    x += 1.0;
                }
                println!("{line}");
                z -= 1.0;
            }
            println!("        x from {} to {} ('=' level with the start, '+' up to 2 higher, '-' up to 3 lower, '_' lower)", r[0], r[2]);
        }
    }
    if std::env::var_os("GDL_NAV_DEBUG").is_some() {
        // The collision triangles under the target.
        for (k, t) in g.collision.tris.iter().enumerate() {
            let p = glam::Vec2::new(to.x, to.z);
            let (a, b, c) = (nav::xz(t.v[0]), nav::xz(t.v[1]), nav::xz(t.v[2]));
            let s = |p0: glam::Vec2, p1: glam::Vec2| (p1 - p0).perp_dot(p - p0);
            let (d1, d2, d3) = (s(a, b), s(b, c), s(c, a));
            let inside = !((d1 < 0.0 || d2 < 0.0 || d3 < 0.0) && (d1 > 0.0 || d2 > 0.0 || d3 > 0.0));
            if inside {
                println!(
                    "  tri {k} owner {} ({}) n {:?} enabled {} stairs {} v {:?}",
                    t.owner,
                    g.level.worlds.world_objects.get(t.owner.max(0) as usize).map_or("?", |o| o.name.as_str()),
                    t.n,
                    g.collision.enabled[k],
                    g.collision.stairs[k],
                    t.v
                );
            }
        }
    }
    let sim = w.walk(start, to);
    println!("probe: start {start:?} -> {to:?}; nav simulation ends at {sim:?} (walks: {})", w.walks(start, to));
    for t in 0..240 {
        let p = g.players[0].pos;
        let d = nav::xz(to - p);
        if d.length() < 0.3 {
            println!("probe: game reached the target at tick {t}");
            break;
        }
        let stick = g.stick_toward(d.normalize_or_zero());
        let mut inp = vec![PlayerInput::default(); g.players.len()];
        inp[0].stick = stick;
        g.update(1.0 / 60.0, &inp);
        if t % 10 == 0 {
            println!("  tick {t}: {:?} act {}", g.players[0].pos, g.players[0].act.cur);
        }
    }
    println!("probe: game ends at {:?}", g.players[0].pos);
}

/// File a bug report (with the runner's note), a save state and a snapshot.
fn capture(
    fs: &GameFs,
    g: &Game,
    bot: &mut Bot,
    o: &Opts,
    level_dir: &Path,
    what: &str,
    tags: &[&str],
) -> Result<PathBuf> {
    let img = match render(fs, g, &o.party) {
        Ok(i) => Some(i),
        Err(e) => {
            log::warn!("runner snapshot: {e:#}");
            None
        }
    };
    let panels = panels(&o.party);
    let progress = crate::progress::Progress::default();
    let prefs = crate::options::Prefs::default();
    let report = crate::bugreport::Report {
        game: Some(g),
        panels: &panels,
        progress: Some(&progress),
        heroes: &[None, None, None, None],
        prefs: Some(&prefs),
        card_file: None,
        tags,
        screen: "playing (level runner)",
        frames: g.frame as u64,
        via: "level runner",
    };
    let bugs = level_dir.join("bugs");
    std::fs::create_dir_all(&bugs)?;
    let dir = crate::bugreport::file(&bugs, &report, img.as_ref())?;
    std::fs::write(
        dir.join("runner.md"),
        format!("# Level runner\n\n**{what}**\n\nParty {}\n\n{}", party_name(&o.party), bot.note(g)),
    )?;
    let st = crate::savestate::SaveState::capture(g, &[], &[], &[], &progress);
    st.write(&dir.join("stuck.state"))?;
    crate::bugreport::write_png(&dir.join("navmap.png"), &bot.nav_map(g))?;
    if let Some(img) = &img {
        let n = std::fs::read_dir(level_dir).map_or(0, |d| d.count());
        crate::bugreport::write_png(&level_dir.join(format!("snap-{n}.png")), img)?;
    }
    println!("  report {}", dir.display());
    Ok(dir)
}

fn panels(party: &[(String, String)]) -> Vec<crate::hud::PanelInfo> {
    party
        .iter()
        .enumerate()
        .map(|(slot, (c, col))| crate::hud::PanelInfo {
            slot,
            name: ["SAM", "BOB", "ANN", "JO"][slot % 4].into(),
            colour: crate::select::COLOURS.iter().position(|k| k == col).unwrap_or(slot % 4),
            class: crate::select::CLASSES.iter().position(|k| k == c).unwrap_or(0),
            hidden: None,
        })
        .collect()
}

thread_local! {
    static GPU: std::cell::RefCell<Option<std::rc::Rc<crate::gpu::Gpu>>> = const { std::cell::RefCell::new(None) };
}

fn headless_gpu() -> Result<std::rc::Rc<crate::gpu::Gpu>> {
    if let Some(g) = GPU.with(|c| c.borrow().clone()) {
        return Ok(g);
    }
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))?;
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))?;
    let gpu = std::rc::Rc::new(crate::gpu::Gpu::new(device, queue, wgpu::TextureFormat::Rgba8Unorm));
    GPU.with(|c| *c.borrow_mut() = Some(gpu.clone()));
    Ok(gpu)
}

/// The level as the player sees it, with the HUD (headless).
fn render(fs: &GameFs, g: &Game, party: &[(String, String)]) -> Result<gdl_formats::Image> {
    let gpu = headless_gpu()?;
    let mut sc = crate::scene::Scene::new(&gpu, g)?;
    sc.sync(&gpu, g);
    let (w, h) = (960u32, 540u32);
    let globals = sc.globals(g, w as f32 / h as f32);
    let hud = crate::hud::Hud::new(&gpu.device, &gpu.queue, gpu.color_format, crate::ui::UiAssets::load(fs));
    let panels = panels(party);
    let draw_hud = |view: &wgpu::TextureView| {
        let mut canvas = crate::hud::Canvas::new(w, h);
        crate::hud::build(&mut crate::screen::Scr::new(&mut canvas, &hud.ui, 100), g, &panels);
        hud.draw(&gpu.device, &gpu.queue, view, &canvas);
    };
    gpu.render_to_image(
        w,
        h,
        &globals,
        crate::scene::CLEAR,
        &sc.draws(g),
        Some(sc.sprites()),
        Some(&draw_hud),
    )
}

// ---- results ----------------------------------------------------------------

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n").replace('\t', " ")
}

fn write_result(out: &Path, r: &RunResult) -> Result<()> {
    let dir = out.join(&r.level);
    std::fs::create_dir_all(&dir)?;
    let reports: Vec<String> = r.reports.iter().map(|p| p.display().to_string()).collect();
    let tsv = [
        r.level.clone(),
        r.party.clone(),
        r.outcome_tag(),
        format!("{:.0}", r.game_time),
        format!("{:.1}", r.wall_secs),
        r.objectives.join(" "),
        r.god.clone().unwrap_or_default(),
        r.outcome_detail(),
        r.notes.join(" | "),
        reports.join(" "),
    ]
    .iter()
    .map(|s| s.replace(['\t', '\n'], " "))
    .collect::<Vec<_>>()
    .join("\t");
    std::fs::write(dir.join(format!("result-{}.tsv", r.party)), tsv + "\n")?;
    let json = format!(
        "{{\"level\":\"{}\",\"party\":\"{}\",\"result\":\"{}\",\"game_time\":{:.1},\"wall_secs\":{:.1},\"objectives\":[{}],\"god\":{},\"detail\":\"{}\",\"notes\":[{}],\"reports\":[{}]}}\n",
        esc(&r.level),
        esc(&r.party),
        r.outcome_tag(),
        r.game_time,
        r.wall_secs,
        r.objectives.iter().map(|s| format!("\"{}\"", esc(s))).collect::<Vec<_>>().join(","),
        r.god.as_ref().map_or("null".into(), |g| format!("\"{}\"", esc(g))),
        esc(&r.outcome_detail()),
        r.notes.iter().map(|s| format!("\"{}\"", esc(s))).collect::<Vec<_>>().join(","),
        reports.iter().map(|s| format!("\"{}\"", esc(s))).collect::<Vec<_>>().join(","),
    );
    std::fs::write(dir.join(format!("result-{}.json", r.party)), json)?;
    Ok(())
}

/// Gather `<out>/*/result-*.tsv` into summary.md, summary.tsv, summary.json.
pub fn merge(out: &Path) -> Result<PathBuf> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut jsons = Vec::new();
    let mut dirs: Vec<_> = std::fs::read_dir(out)?.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    dirs.sort();
    for d in dirs {
        let mut files: Vec<_> = std::fs::read_dir(&d)?.flatten().map(|e| e.path()).collect();
        files.sort();
        for f in files {
            let name = f.file_name().unwrap_or_default().to_string_lossy().to_string();
            if name.starts_with("result-") && name.ends_with(".tsv") {
                let s = std::fs::read_to_string(&f)?;
                rows.push(s.trim_end_matches('\n').split('\t').map(String::from).collect());
            } else if name.starts_with("result-") && name.ends_with(".json") {
                jsons.push(std::fs::read_to_string(&f)?.trim().to_string());
            }
        }
    }
    let ok = |r: &Vec<String>| r.get(2).is_some_and(|t| t.starts_with("completed"));
    let clean = rows.iter().filter(|r| r.get(2).is_some_and(|t| t == "completed")).count();
    let mut md = format!(
        "# Level runner summary\n\n{} runs: {} completed ({} without god mode or debug kills), {} failed.\n\n| level | party | result | game s | objectives used | god mode | detail / notes | reports |\n|---|---|---|---|---|---|---|---|\n",
        rows.len(),
        rows.iter().filter(|r| ok(r)).count(),
        clean,
        rows.iter().filter(|r| !ok(r)).count()
    );
    let mut tsv = String::from("level\tparty\tresult\tgame_s\twall_s\tobjectives\tgod\tdetail\tnotes\treports\n");
    for r in &rows {
        let f = |i: usize| r.get(i).cloned().unwrap_or_default().replace('|', "/");
        let objs = f(5);
        let objs = if objs.len() > 120 { format!("{}...", &objs[..objs.floor_char_boundary(120)]) } else { objs };
        let detail = [f(7), f(8)].iter().filter(|s| !s.is_empty()).cloned().collect::<Vec<_>>().join("; ");
        md += &format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} |\n",
            f(0), f(1), f(2), f(3), objs, f(6), detail, f(9)
        );
        tsv += &(r.join("\t") + "\n");
    }
    std::fs::write(out.join("summary.tsv"), tsv)?;
    std::fs::write(out.join("summary.json"), format!("[\n{}\n]\n", jsons.join(",\n")))?;
    let p = out.join("summary.md");
    std::fs::write(&p, md)?;
    Ok(p)
}
