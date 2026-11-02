//! Gauntlet Dark Legacy, the simulation: the game rules rebuilt from the
//! GameCube original, running on its data. Nothing here opens a window,
//! talks to a GPU or plays a sound: `Game::update` takes each player's input
//! for one 60 Hz tick and the engine draws and sounds what it reads back.
//! The same code runs on every peer of a networked game, so everything in
//! it is deterministic: fixed-step, seeded, and ordered.

pub mod ai;
pub mod boss;
pub mod camera;
pub mod character;
pub mod cheats;
pub mod collision;
pub mod critter;
pub mod enemies;
pub mod fs;
pub mod game;
pub(crate) use game::items;
pub use game::savestate;
pub mod heroes;
pub mod hints;
pub mod input;
pub mod interp;
pub mod level;
pub mod math;
pub mod mesh;
pub mod messages;
pub mod missile;
pub mod paction;
pub mod powerups;
pub mod progress;
pub mod psys;
pub mod rules;
pub mod save;
pub mod shadow;
pub mod soundlib;
pub mod stats;
pub mod texanim;
pub mod tower;
pub mod transport;
pub mod triggers;
pub mod worldanim;
