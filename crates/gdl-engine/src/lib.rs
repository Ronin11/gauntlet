//! Gauntlet Dark Legacy engine: rendering, simulation, input and audio on top of the
//! original game data. The desktop binary (`gauntlet`) and the Android app both use it.

pub mod app;
pub mod attract;
pub mod audio;
pub mod character;
pub mod collision;
pub mod cursor;
pub mod fs;
pub mod game;
pub mod gpu;
pub mod hud;
pub mod input;
pub mod level;
pub mod mapscreen;
pub mod menu;
pub mod mesh;
pub mod movie;
pub mod options;
pub mod optmenu;
pub mod progress;
pub mod scene;
pub mod screen;
pub mod select;
pub mod stats;
pub mod triggers;
pub mod ui;
