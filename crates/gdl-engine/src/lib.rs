//! Gauntlet Dark Legacy engine: rendering, simulation, input and audio on top of the
//! original game data. The desktop binary (`gauntlet`) and the Android app both use it.

pub use gdl_sim::{
    ai, boss, camera, character, cheats, collision, compat, critter, enemies, fs, game, heroes,
    hints, interp, level, mesh, messages, missile, paction, powerups, progress, psys, replay,
    rules, save, savestate, session, shadow, soundlib, stats, texanim, tower, transport, triggers,
    worldanim,
};

pub mod app;
pub mod attract;
pub mod audio;
pub mod bugreport;
pub mod cursor;
pub mod debugmenu;
pub mod flyby;
pub mod gpu;
pub mod hud;
pub mod input;
pub mod mapscreen;
pub mod menu;
pub mod modern;
pub mod movie;
pub mod options;
pub mod optmenu;
pub mod overrides;
pub mod runner;
pub mod scene;
pub mod screen;
pub mod screensaver;
pub mod select;
pub mod shop;
pub mod ui;
