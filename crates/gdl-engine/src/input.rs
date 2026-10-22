//! Input: keyboard (one player) and up to four gamepads, mapped to per-player commands.

use gilrs::{Axis, Button, Gilrs};
use glam::Vec2;
use std::collections::HashSet;
use winit::keyboard::KeyCode;

#[derive(Clone, Copy, Debug, Default)]
pub struct PlayerInput {
    /// Left stick / WASD, x = right, y = up (screen relative), magnitude 0..1.
    pub stick: Vec2,
    pub attack: bool,
    pub attack_pressed: bool,
    pub magic_pressed: bool,
    pub turbo: bool,
    pub start_pressed: bool,
}

/// Where a player's input comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Keyboard,
    Pad(gilrs::GamepadId),
}

pub struct Input {
    gilrs: Option<Gilrs>,
    keys: HashSet<KeyCode>,
    pressed: HashSet<KeyCode>,
    pad_pressed: Vec<(gilrs::GamepadId, Button)>,
}

impl Input {
    pub fn new() -> Self {
        let gilrs = match Gilrs::new() {
            Ok(g) => {
                for (_, pad) in g.gamepads() {
                    log::info!("gamepad: {} ({:?})", pad.name(), pad.power_info());
                }
                Some(g)
            }
            Err(e) => {
                log::warn!("gamepads unavailable: {e}");
                None
            }
        };
        Input {
            gilrs,
            keys: HashSet::new(),
            pressed: HashSet::new(),
            pad_pressed: Vec::new(),
        }
    }

    pub fn key(&mut self, code: KeyCode, down: bool) {
        if down {
            if self.keys.insert(code) {
                self.pressed.insert(code);
            }
        } else {
            self.keys.remove(&code);
        }
    }

    pub fn held(&self, code: KeyCode) -> bool {
        self.keys.contains(&code)
    }

    pub fn just_pressed(&self, code: KeyCode) -> bool {
        self.pressed.contains(&code)
    }

    /// Drain gamepad events. Call once per frame before reading.
    pub fn poll(&mut self) -> Vec<gilrs::GamepadId> {
        let mut connected = Vec::new();
        if let Some(g) = &mut self.gilrs {
            while let Some(ev) = g.next_event() {
                match ev.event {
                    gilrs::EventType::ButtonPressed(b, _) => self.pad_pressed.push((ev.id, b)),
                    gilrs::EventType::Connected => {
                        log::info!("gamepad connected: {}", g.gamepad(ev.id).name());
                        connected.push(ev.id);
                    }
                    _ => {}
                }
            }
        }
        connected
    }

    /// All gamepads currently connected.
    pub fn pads(&self) -> Vec<gilrs::GamepadId> {
        self.gilrs
            .as_ref()
            .map(|g| g.gamepads().map(|(id, _)| id).collect())
            .unwrap_or_default()
    }

    pub fn pad_pressed_any(&self, id: gilrs::GamepadId, b: Button) -> bool {
        self.pad_pressed.iter().any(|&(i, bb)| i == id && bb == b)
    }

    pub fn read(&self, src: Source) -> PlayerInput {
        match src {
            Source::Keyboard => {
                let mut v = Vec2::ZERO;
                if self.held(KeyCode::KeyW) || self.held(KeyCode::ArrowUp) {
                    v.y += 1.0;
                }
                if self.held(KeyCode::KeyS) || self.held(KeyCode::ArrowDown) {
                    v.y -= 1.0;
                }
                if self.held(KeyCode::KeyD) || self.held(KeyCode::ArrowRight) {
                    v.x += 1.0;
                }
                if self.held(KeyCode::KeyA) || self.held(KeyCode::ArrowLeft) {
                    v.x -= 1.0;
                }
                PlayerInput {
                    stick: v.normalize_or_zero(),
                    attack: self.held(KeyCode::KeyJ) || self.held(KeyCode::Space),
                    attack_pressed: self.just_pressed(KeyCode::KeyJ)
                        || self.just_pressed(KeyCode::Space),
                    magic_pressed: self.just_pressed(KeyCode::KeyK),
                    turbo: self.held(KeyCode::KeyL) || self.held(KeyCode::ShiftLeft),
                    start_pressed: self.just_pressed(KeyCode::Enter),
                }
            }
            Source::Pad(id) => {
                let Some(g) = &self.gilrs else {
                    return PlayerInput::default();
                };
                let pad = g.gamepad(id);
                let mut v = Vec2::new(pad.value(Axis::LeftStickX), pad.value(Axis::LeftStickY));
                if pad.is_pressed(Button::DPadUp) {
                    v.y = 1.0;
                }
                if pad.is_pressed(Button::DPadDown) {
                    v.y = -1.0;
                }
                if pad.is_pressed(Button::DPadLeft) {
                    v.x = -1.0;
                }
                if pad.is_pressed(Button::DPadRight) {
                    v.x = 1.0;
                }
                // Radial dead zone.
                let mag = v.length();
                let v = if mag < 0.2 {
                    Vec2::ZERO
                } else {
                    v / mag * ((mag - 0.2) / 0.8).min(1.0)
                };
                PlayerInput {
                    stick: v,
                    attack: pad.is_pressed(Button::South),
                    attack_pressed: self.pad_pressed_any(id, Button::South),
                    magic_pressed: self.pad_pressed_any(id, Button::East),
                    turbo: pad.is_pressed(Button::West) || pad.is_pressed(Button::RightTrigger2),
                    start_pressed: self.pad_pressed_any(id, Button::Start),
                }
            }
        }
    }

    /// Clear per-frame edge state.
    pub fn end_frame(&mut self) {
        self.pressed.clear();
        self.pad_pressed.clear();
    }
}
