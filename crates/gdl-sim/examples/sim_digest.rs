//! Plays levels headless on bot inputs and prints a digest per level, so two
//! builds (debug and release, x86 and ARM, two machines) can be compared:
//!
//!     cargo run -p gdl-sim --release --example sim_digest -- \\
//!         [--ticks N] [--data DIR] [--sections] [LEVEL...]
//!
//! With no level named it plays every level on the disc. Identical output
//! means the builds simulate identically; `diff` finds the first level that
//! does not, and `--sections` then names what differs in it.
//!
//! `--record DIR` also writes each level's recording there, and `--check
//! FILE...` plays recordings instead, so a recording made on one machine can
//! be tried on another without a window or a GPU.

use gdl_sim::compat::Compat;
use gdl_sim::fs::GameFs;
use gdl_sim::game::setup::{LevelSetup, LevelStart};
use gdl_sim::game::{Game, PanelInfo};
use gdl_sim::progress::Progress;
use gdl_sim::replay::Replay;
use gdl_sim::rules::Rules;

const PARTY: [(&str, &str); 4] = [
    ("WAR", "RED"),
    ("VAL", "BLU"),
    ("WIZ", "YEL"),
    ("ARC", "GRE"),
];

/// Four fresh heroes on Medium.
fn start_of(level: &str) -> LevelStart {
    let progress = Progress::default();
    LevelStart {
        level: level.to_string(),
        difficulty: 1,
        panels: PARTY
            .iter()
            .enumerate()
            .map(|(slot, (class, colour))| PanelInfo {
                slot,
                name: String::new(),
                colour: gdl_sim::heroes::COLOURS
                    .iter()
                    .position(|c| c == colour)
                    .unwrap_or(0),
                class: gdl_sim::heroes::CLASSES
                    .iter()
                    .position(|c| c == class)
                    .unwrap_or(0),
                hidden: None,
            })
            .collect(),
        setup: LevelSetup::new(&Rules::classic(), 0, &progress),
        progress,
        ..Default::default()
    }
}

fn main() -> anyhow::Result<()> {
    let mut ticks = 1800u32;
    let mut data = None;
    let mut sections = false;
    let mut levels = Vec::new();
    let mut record = None;
    let mut check = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--ticks" => ticks = args.next().unwrap_or_default().parse()?,
            "--data" => data = args.next(),
            "--sections" => sections = true,
            "--record" => record = args.next(),
            "--check" => check = true,
            _ => levels.push(a),
        }
    }
    let fs = match data {
        Some(d) => GameFs::open(std::path::Path::new(&d))?,
        None => GameFs::discover()?,
    };
    if check {
        let mut differ = 0;
        for path in &levels {
            let verdict = Replay::read(std::path::Path::new(path))?.check(&fs, None)?;
            println!("{verdict}");
            differ += !verdict.matches() as u32;
        }
        println!("{differ} of {} differ", levels.len());
        std::process::exit((differ > 0) as i32);
    }
    if levels.is_empty() {
        levels = fs.list_dirs("LEVELS");
    }
    let compat = Compat {
        build: "sim_digest".into(),
        content: 0,
        rules: Rules::classic().digest(),
        env: 0,
    };
    for level in &levels {
        let start = start_of(level);
        let mut g = match start.build(&fs) {
            Ok((g, _)) => g,
            Err(e) => {
                println!("{level} unloadable: {e}");
                continue;
            }
        };
        let mut replay = Replay::new(start);
        for _ in 0..ticks {
            let inputs: Vec<_> = (0..g.players.len())
                .map(|i| g.bot_input(i).pack())
                .collect();
            g.update_packed(&inputs);
            replay.tick(&inputs, &g);
        }
        if let Some(dir) = &record {
            let path = std::path::Path::new(dir).join(format!("{level}.replay"));
            replay.write(&path, &g, &compat)?;
        }
        let t0 = std::time::Instant::now();
        let digest = g.state_digest();
        if sections {
            eprintln!("{level}: digest took {:?}, a clone {:?}", t0.elapsed(), {
                let t = std::time::Instant::now();
                let _ = g.clone();
                t.elapsed()
            });
        }
        println!("{level} {digest:016x}");
        if sections {
            for (name, h) in g.state_sections() {
                println!("  {name} {h:016x}");
            }
        }
    }
    Ok(())
}
