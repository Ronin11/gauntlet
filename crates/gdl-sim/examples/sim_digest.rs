//! Plays levels headless on bot inputs and prints a digest per level, so two
//! builds (debug and release, x86 and ARM, two machines) can be compared:
//!
//!     cargo run -p gdl-sim --release --example sim_digest -- [--ticks N] [--data DIR] [LEVEL...]
//!
//! With no level named it plays every level on the disc. Identical output
//! means the builds simulate identically; `diff` finds the first level that
//! does not.

use gdl_sim::fs::GameFs;
use gdl_sim::game::Game;

const PARTY: [(&str, &str); 4] = [
    ("WAR", "RED"),
    ("VAL", "BLU"),
    ("WIZ", "YEL"),
    ("ARC", "GRE"),
];

fn main() -> anyhow::Result<()> {
    let mut ticks = 1800u32;
    let mut data = None;
    let mut levels = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--ticks" => ticks = args.next().unwrap_or_default().parse()?,
            "--data" => data = args.next(),
            _ => levels.push(a),
        }
    }
    let fs = match data {
        Some(d) => GameFs::open(std::path::Path::new(&d))?,
        None => GameFs::discover()?,
    };
    if levels.is_empty() {
        levels = fs.list_dirs("LEVELS");
    }
    for level in &levels {
        let mut g = match Game::new(&fs, level, &PARTY) {
            Ok(g) => g,
            Err(e) => {
                println!("{level} unloadable: {e}");
                continue;
            }
        };
        for _ in 0..ticks {
            let inputs: Vec<_> = (0..g.players.len()).map(|i| g.bot_input(i)).collect();
            g.update(1.0 / 60.0, &inputs);
        }
        println!("{level} {:016x}", g.fingerprint());
    }
    Ok(())
}
