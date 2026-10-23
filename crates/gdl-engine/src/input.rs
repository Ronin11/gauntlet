//! Input: keyboard, gamepads (via gilrs on desktop), and key-event controllers
//! (Bluetooth pads on Android report buttons as key events per input device).

#[cfg(not(target_os = "android"))]
use gilrs::{Axis, Button, Gilrs};
use glam::Vec2;
use std::collections::{HashMap, HashSet};
use winit::keyboard::KeyCode;

#[derive(Clone, Copy, Debug, Default)]
pub struct PlayerInput {
    /// Left stick / WASD, x = right, y = up (screen relative), magnitude 0..1.
    pub stick: Vec2,
    /// A: attack (held and newly pressed).
    pub attack: bool,
    pub attack_pressed: bool,
    /// X: magic (use a potion).
    pub magic_pressed: bool,
    /// B held: turbo / block (they share a button on GameCube).
    pub turbo: bool,
    /// B newly pressed: back in menus, "Quit Game" when dead.
    pub b_pressed: bool,
    /// Y: slow attack (newly pressed, and held).
    pub slow_pressed: bool,
    pub slow: bool,
    /// X held.
    pub magic: bool,
    /// GameCube L: charge; R held: strafe; Z held: two-player combo.
    pub charge_pressed: bool,
    pub strafe: bool,
    pub combo: bool,
    pub start_pressed: bool,
    /// The d-pad's share of `stick` (menus move with either; in play the d-pad
    /// runs the mini-inventory instead of walking).
    pub dpad: Vec2,
    /// D-pad presses: the mini-inventory (up opens / switches on, left and
    /// right choose, down closes).
    pub inv: crate::powerups::DPad,
}

#[cfg(not(target_os = "android"))]
pub type PadId = gilrs::GamepadId;
#[cfg(target_os = "android")]
pub type PadId = usize;

/// Where a player's input comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Keyboard,
    Pad(PadId),
    /// A controller that reports buttons as key events (Android), by device id.
    Device(u64),
}

/// Android key codes for gamepad buttons (android.view.KeyEvent).
pub mod android_keys {
    pub const DPAD_UP: u32 = 19;
    pub const DPAD_DOWN: u32 = 20;
    pub const DPAD_LEFT: u32 = 21;
    pub const DPAD_RIGHT: u32 = 22;
    pub const DPAD_CENTER: u32 = 23;
    pub const BUTTON_A: u32 = 96;
    pub const BUTTON_B: u32 = 97;
    pub const BUTTON_X: u32 = 99;
    pub const BUTTON_Y: u32 = 100;
    pub const BUTTON_L1: u32 = 102;
    pub const BUTTON_R1: u32 = 103;
    pub const BUTTON_L2: u32 = 104;
    pub const BUTTON_R2: u32 = 105;
    pub const BUTTON_START: u32 = 108;
}

pub struct Input {
    #[cfg(not(target_os = "android"))]
    gilrs: Option<Gilrs>,
    keys: HashSet<KeyCode>,
    pressed: HashSet<KeyCode>,
    #[cfg(not(target_os = "android"))]
    pad_pressed: Vec<(PadId, Button)>,
    dev_held: HashMap<u64, HashSet<u32>>,
    /// Left-stick axes of key-reporting controllers (Android joystick motion).
    dev_stick: HashMap<u64, Vec2>,
    dev_pressed: Vec<(u64, u32)>,
    /// The rumble playing on each pad (dropping an effect stops it).
    #[cfg(not(target_os = "android"))]
    rumbles: HashMap<PadId, gilrs::ff::Effect>,
}

impl Default for Input {
    fn default() -> Self {
        Self::new()
    }
}

impl Input {
    pub fn new() -> Self {
        #[cfg(not(target_os = "android"))]
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
            #[cfg(not(target_os = "android"))]
            gilrs,
            keys: HashSet::new(),
            pressed: HashSet::new(),
            #[cfg(not(target_os = "android"))]
            pad_pressed: Vec::new(),
            dev_held: HashMap::new(),
            dev_stick: HashMap::new(),
            dev_pressed: Vec::new(),
            #[cfg(not(target_os = "android"))]
            rumbles: HashMap::new(),
        }
    }

    /// do_vibe: rumble a pad at `strength` (0..1) for `ms`, replacing what it
    /// was doing. Pads without force feedback (and keyboards, and Android's
    /// key-event controllers) are left alone.
    #[allow(unused_variables)]
    pub fn rumble(&mut self, src: Source, strength: f32, ms: u32) {
        #[cfg(not(target_os = "android"))]
        {
            use gilrs::ff::{BaseEffect, BaseEffectType, EffectBuilder, Repeat, Replay, Ticks};
            let (Source::Pad(id), Some(g)) = (src, &mut self.gilrs) else {
                return;
            };
            if !g.connected_gamepad(id).is_some_and(|p| p.is_ff_supported()) {
                return;
            }
            let magnitude = (strength.clamp(0.0, 1.0) * u16::MAX as f32) as u16;
            let len = Ticks::from_ms(ms.max(1));
            let fx = EffectBuilder::new()
                .add_effect(BaseEffect {
                    kind: BaseEffectType::Strong { magnitude },
                    scheduling: Replay {
                        play_for: len,
                        ..Default::default()
                    },
                    ..Default::default()
                })
                .add_effect(BaseEffect {
                    kind: BaseEffectType::Weak { magnitude },
                    scheduling: Replay {
                        play_for: len,
                        ..Default::default()
                    },
                    ..Default::default()
                })
                .gamepads(&[id])
                .repeat(Repeat::For(len))
                .finish(g);
            match fx {
                Ok(fx) => {
                    if fx.play().is_ok() {
                        self.rumbles.insert(id, fx);
                    }
                }
                Err(e) => log::debug!("rumble: {e}"),
            }
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

    /// A key-reporting controller's left stick (x right, y up, -1..1).
    pub fn device_axes(&mut self, dev: u64, x: f32, y: f32) {
        self.dev_held.entry(dev).or_default();
        self.dev_stick.insert(dev, Vec2::new(x, y));
    }

    /// A raw button event from a key-reporting controller.
    pub fn device_key(&mut self, dev: u64, code: u32, down: bool) {
        let held = self.dev_held.entry(dev).or_default();
        if down {
            if held.insert(code) {
                self.dev_pressed.push((dev, code));
            }
        } else {
            held.remove(&code);
        }
    }

    pub fn held(&self, code: KeyCode) -> bool {
        self.keys.contains(&code)
    }

    pub fn just_pressed(&self, code: KeyCode) -> bool {
        self.pressed.contains(&code)
    }

    /// Drain gamepad events. Call once per frame before reading.
    pub fn poll(&mut self) -> Vec<PadId> {
        #[allow(unused_mut)]
        let mut connected = Vec::new();
        #[cfg(not(target_os = "android"))]
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
    pub fn pads(&self) -> Vec<PadId> {
        #[cfg(not(target_os = "android"))]
        {
            self.gilrs
                .as_ref()
                .map(|g| g.gamepads().map(|(id, _)| id).collect())
                .unwrap_or_default()
        }
        #[cfg(target_os = "android")]
        Vec::new()
    }

    /// Key-reporting controllers that have sent any button.
    pub fn devices(&self) -> Vec<u64> {
        let mut v: Vec<u64> = self.dev_held.keys().copied().collect();
        v.sort();
        v
    }

    /// Every input source that could join a game.
    pub fn sources(&self) -> Vec<Source> {
        let mut v = vec![Source::Keyboard];
        v.extend(self.pads().into_iter().map(Source::Pad));
        v.extend(self.devices().into_iter().map(Source::Device));
        v
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
                    magic_pressed: self.just_pressed(KeyCode::KeyL),
                    turbo: self.held(KeyCode::KeyK) || self.held(KeyCode::ShiftLeft),
                    b_pressed: self.just_pressed(KeyCode::KeyK)
                        || self.just_pressed(KeyCode::Backspace),
                    slow_pressed: self.just_pressed(KeyCode::KeyI),
                    slow: self.held(KeyCode::KeyI),
                    magic: self.held(KeyCode::KeyL),
                    charge_pressed: self.just_pressed(KeyCode::KeyH),
                    strafe: self.held(KeyCode::KeyU),
                    combo: self.held(KeyCode::KeyO),
                    start_pressed: self.just_pressed(KeyCode::Enter),
                    dpad: Vec2::ZERO,
                    // Tab / Q / E / R stand in for the d-pad.
                    inv: crate::powerups::DPad {
                        up: self.just_pressed(KeyCode::Tab),
                        left: self.just_pressed(KeyCode::KeyQ),
                        right: self.just_pressed(KeyCode::KeyE),
                        down: self.just_pressed(KeyCode::KeyR),
                    },
                }
            }
            Source::Device(dev) => {
                use android_keys::*;
                let empty = HashSet::new();
                let held = self.dev_held.get(&dev).unwrap_or(&empty);
                let h = |c: u32| held.contains(&c);
                let p = |c: u32| self.dev_pressed.contains(&(dev, c));
                let mut v = Vec2::ZERO;
                if h(DPAD_UP) {
                    v.y += 1.0;
                }
                if h(DPAD_DOWN) {
                    v.y -= 1.0;
                }
                if h(DPAD_RIGHT) {
                    v.x += 1.0;
                }
                if h(DPAD_LEFT) {
                    v.x -= 1.0;
                }
                let dpad = v.normalize_or_zero();
                // A pad whose stick reports walks with it, and its d-pad runs the
                // mini-inventory; one without walks with the d-pad.
                let (stick, dpad, inv) = match self.dev_stick.get(&dev) {
                    Some(&a) => {
                        let mag = a.length();
                        let a = if mag < 0.2 {
                            Vec2::ZERO
                        } else {
                            a / mag * ((mag - 0.2) / 0.8).min(1.0)
                        };
                        let inv = crate::powerups::DPad {
                            up: p(DPAD_UP),
                            down: p(DPAD_DOWN),
                            left: p(DPAD_LEFT),
                            right: p(DPAD_RIGHT),
                        };
                        ((a + dpad).clamp_length_max(1.0), dpad, inv)
                    }
                    None => (dpad, Vec2::ZERO, Default::default()),
                };
                PlayerInput {
                    stick,
                    dpad,
                    inv,
                    attack: h(BUTTON_A) || h(DPAD_CENTER),
                    attack_pressed: p(BUTTON_A) || p(DPAD_CENTER),
                    magic_pressed: p(BUTTON_X),
                    turbo: h(BUTTON_B),
                    b_pressed: p(BUTTON_B),
                    slow_pressed: p(BUTTON_Y),
                    slow: h(BUTTON_Y),
                    magic: h(BUTTON_X),
                    charge_pressed: p(BUTTON_L1) || p(BUTTON_L2),
                    strafe: h(BUTTON_R2),
                    combo: h(BUTTON_R1),
                    start_pressed: p(BUTTON_START),
                }
            }
            #[cfg(not(target_os = "android"))]
            Source::Pad(id) => {
                let Some(g) = &self.gilrs else {
                    return PlayerInput::default();
                };
                let pad = g.gamepad(id);
                let v = Vec2::new(pad.value(Axis::LeftStickX), pad.value(Axis::LeftStickY));
                // Radial dead zone.
                let mag = v.length();
                let v = if mag < 0.2 {
                    Vec2::ZERO
                } else {
                    v / mag * ((mag - 0.2) / 0.8).min(1.0)
                };
                let mut dpad = Vec2::ZERO;
                if pad.is_pressed(Button::DPadUp) {
                    dpad.y = 1.0;
                }
                if pad.is_pressed(Button::DPadDown) {
                    dpad.y = -1.0;
                }
                if pad.is_pressed(Button::DPadLeft) {
                    dpad.x = -1.0;
                }
                if pad.is_pressed(Button::DPadRight) {
                    dpad.x = 1.0;
                }
                let pressed =
                    |b: Button| self.pad_pressed.iter().any(|&(i, bb)| i == id && bb == b);
                PlayerInput {
                    stick: (v + dpad).clamp_length_max(1.0),
                    dpad,
                    inv: crate::powerups::DPad {
                        up: pressed(Button::DPadUp),
                        down: pressed(Button::DPadDown),
                        left: pressed(Button::DPadLeft),
                        right: pressed(Button::DPadRight),
                    },
                    attack: pad.is_pressed(Button::South),
                    attack_pressed: pressed(Button::South),
                    magic_pressed: pressed(Button::West),
                    turbo: pad.is_pressed(Button::East),
                    b_pressed: pressed(Button::East),
                    slow_pressed: pressed(Button::North),
                    slow: pad.is_pressed(Button::North),
                    magic: pad.is_pressed(Button::West),
                    charge_pressed: pressed(Button::LeftTrigger2) || pressed(Button::LeftTrigger),
                    strafe: pad.is_pressed(Button::RightTrigger2),
                    combo: pad.is_pressed(Button::RightTrigger),
                    start_pressed: pressed(Button::Start),
                }
            }
            #[cfg(target_os = "android")]
            Source::Pad(_) => PlayerInput::default(),
        }
    }

    /// Clear per-frame edge state.
    pub fn end_frame(&mut self) {
        self.pressed.clear();
        #[cfg(not(target_os = "android"))]
        self.pad_pressed.clear();
        self.dev_pressed.clear();
    }
}
