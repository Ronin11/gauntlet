//! Input: keyboard, gamepads (via gilrs on desktop), and key-event controllers
//! (Android), read into the simulation's [`PlayerInput`] per player.

pub use gdl_sim::input::*;

#[cfg(not(target_os = "android"))]
use gilrs::{Axis, Button, Gilrs};
use glam::Vec2;
use std::collections::{HashMap, HashSet};
use winit::keyboard::KeyCode;

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
    /// No device: the app gets this hero's input from the loyal follower.
    Companion,
    /// A player of a network game, by player number: what the network says
    /// they press this tick, this machine's own player included.
    Net(usize),
}

/// A network game: this tick's inputs for everyone, which is all the game sees.
struct NetInputs {
    me: usize,
    inputs: Vec<PlayerInput>,
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
    net: Option<NetInputs>,
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
            net: None,
        }
    }

    /// From now on the game sees only the network's inputs, for these players.
    pub fn start_net(&mut self, me: usize, players: usize) {
        self.net = Some(NetInputs {
            me,
            inputs: vec![PlayerInput::default(); players],
        });
    }

    pub fn end_net(&mut self) {
        self.net = None;
    }

    /// This tick's inputs, from the network.
    pub fn set_net_inputs(&mut self, inputs: &[PackedInput]) {
        if let Some(n) = &mut self.net {
            n.inputs = inputs.iter().map(PackedInput::unpack).collect();
        }
    }

    /// What this machine's player is pressing, on any of its controllers, for
    /// the network to carry. Hands over the presses, once.
    pub fn take_local(&mut self, escape: bool) -> PackedInput {
        let mut m = PackedInput::default();
        for src in self.device_sources() {
            let p = self.read(src).pack();
            m.buttons |= p.buttons;
            if m.stick == [0, 0] {
                m.stick = p.stick;
            }
            if m.fire == [0, 0] {
                m.fire = p.fire;
            }
            if m.dpad == [0, 0] {
                m.dpad = p.dpad;
            }
        }
        if escape {
            let start = PlayerInput {
                start_pressed: true,
                start: true,
                ..Default::default()
            };
            m.buttons |= start.pack().buttons;
        }
        self.clear_presses();
        m
    }

    /// do_vibe: rumble a pad at `strength` (0..1) for `ms`, replacing what it
    /// was doing. Pads without force feedback (and keyboards, and Android's
    /// key-event controllers) are left alone.
    #[allow(unused_variables)]
    pub fn rumble(&mut self, src: Source, strength: f32, ms: u32) {
        #[cfg(not(target_os = "android"))]
        {
            use gilrs::ff::{BaseEffect, BaseEffectType, EffectBuilder, Repeat, Replay, Ticks};
            // This machine's own player of a network game: their first pad.
            let src = match (src, &self.net) {
                (Source::Net(i), Some(n)) if i == n.me => match self.pads().first() {
                    Some(&id) => Source::Pad(id),
                    None => return,
                },
                _ => src,
            };
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
        if let Some(n) = &self.net {
            return (0..n.inputs.len()).map(Source::Net).collect();
        }
        self.device_sources()
    }

    fn device_sources(&self) -> Vec<Source> {
        let mut v = vec![Source::Keyboard];
        v.extend(self.pads().into_iter().map(Source::Pad));
        v.extend(self.devices().into_iter().map(Source::Device));
        v
    }

    pub fn read(&self, src: Source) -> PlayerInput {
        match src {
            Source::Companion => PlayerInput::default(),
            Source::Net(i) => self
                .net
                .as_ref()
                .and_then(|n| n.inputs.get(i).copied())
                .unwrap_or_default(),
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
                    charge: self.held(KeyCode::KeyH),
                    strafe: self.held(KeyCode::KeyU),
                    strafe_pressed: self.just_pressed(KeyCode::KeyU),
                    fire: Vec2::ZERO,
                    combo: self.held(KeyCode::KeyO),
                    start_pressed: self.just_pressed(KeyCode::Enter),
                    start: self.held(KeyCode::Enter),
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
                    charge: h(BUTTON_L1) || h(BUTTON_L2),
                    strafe: h(BUTTON_R2),
                    strafe_pressed: p(BUTTON_R2),
                    fire: Vec2::ZERO,
                    combo: h(BUTTON_R1),
                    start_pressed: p(BUTTON_START),
                    start: h(BUTTON_START),
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
                    charge: pad.is_pressed(Button::LeftTrigger2)
                        || pad.is_pressed(Button::LeftTrigger),
                    strafe: pad.is_pressed(Button::RightTrigger2),
                    strafe_pressed: pressed(Button::RightTrigger2),
                    fire: Vec2::new(pad.value(Axis::RightStickX), pad.value(Axis::RightStickY)),
                    combo: pad.is_pressed(Button::RightTrigger),
                    start_pressed: pressed(Button::Start),
                    start: pad.is_pressed(Button::Start),
                }
            }
            #[cfg(target_os = "android")]
            Source::Pad(_) => PlayerInput::default(),
        }
    }

    /// Clear per-frame edge state. In a network game presses last until the
    /// network has them ([`Input::take_local`]), however long a tick waits.
    pub fn end_frame(&mut self) {
        if self.net.is_none() {
            self.clear_presses();
        }
    }

    fn clear_presses(&mut self) {
        self.pressed.clear();
        #[cfg(not(target_os = "android"))]
        self.pad_pressed.clear();
        self.dev_pressed.clear();
    }
}

/// The name a save state records for a hero's controller.
pub fn source_name(s: &Source, pads: &[PadId]) -> String {
    use Source;
    match s {
        Source::Keyboard => "keyboard".into(),
        Source::Pad(id) => match pads.iter().position(|p| p == id) {
            Some(n) => format!("pad {n}"),
            None => "pad".into(),
        },
        Source::Device(_) => "device".into(),
        Source::Companion => "companion".into(),
        Source::Net(i) => format!("net {i}"),
    }
}

/// Controllers for a loaded state's heroes: the saved pad where it is still
/// connected, else a free pad, else the keyboard.
pub fn bind_sources(
    heroes: &[gdl_sim::savestate::HeroState],
    pads: &[PadId],
    current: &[Source],
) -> Vec<Source> {
    use Source;
    let mut out: Vec<Option<Source>> = vec![None; heroes.len()];
    let mut taken: Vec<Source> = Vec::new();
    // Saved bindings first.
    for (i, h) in heroes.iter().enumerate() {
        let mut t = h.source.split_whitespace();
        let want = match (t.next(), t.next().and_then(|n| n.parse::<usize>().ok())) {
            (Some("keyboard"), _) => Some(Source::Keyboard),
            (Some("companion"), _) => Some(Source::Companion),
            (Some("pad"), Some(n)) => pads.get(n).map(|&p| Source::Pad(p)),
            _ => None,
        };
        if let Some(s) = want.filter(|s| !taken.contains(s)) {
            taken.push(s);
            out[i] = Some(s);
        }
    }
    // Then the controllers playing now, then any pad, then the keyboard.
    let mut spare: Vec<Source> = current
        .iter()
        .copied()
        .filter(|s| *s != Source::Companion)
        .collect();
    spare.extend(pads.iter().map(|&p| Source::Pad(p)));
    spare.push(Source::Keyboard);
    for o in out.iter_mut().filter(|o| o.is_none()) {
        if let Some(s) = spare.iter().copied().find(|s| !taken.contains(s)) {
            taken.push(s);
            *o = Some(s);
        }
    }
    out.into_iter()
        .map(|o| o.unwrap_or(Source::Keyboard))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tick of a network game can wait a long time for the others. A press
    /// must reach the network, once, however many frames go by first.
    #[test]
    fn in_a_network_game_a_press_lasts_until_the_network_has_it() {
        let mut i = Input::new();
        i.start_net(0, 2);
        i.key(KeyCode::Space, true);
        i.key(KeyCode::Space, false);
        // Frames go by, waiting.
        i.end_frame();
        i.end_frame();
        let first = i.take_local(false).unpack();
        assert!(first.attack_pressed, "the press was lost");
        assert!(!first.attack, "it was let go of");
        assert!(!i.take_local(false).unpack().attack_pressed, "only once");
    }

    #[test]
    fn the_game_sees_only_what_the_network_says() {
        let mut i = Input::new();
        i.start_net(1, 2);
        assert_eq!(i.sources(), [Source::Net(0), Source::Net(1)]);
        // This machine's own keyboard reaches the game only by the network.
        i.key(KeyCode::KeyW, true);
        assert_eq!(i.read(Source::Net(1)), PlayerInput::default());
        let mine = i.take_local(false);
        assert!(mine.unpack().stick.y > 0.9);
        i.set_net_inputs(&[PackedInput::default(), mine]);
        assert_eq!(i.read(Source::Net(1)).stick, mine.unpack().stick);
        assert_eq!(i.read(Source::Net(0)), PlayerInput::default());
        // Out of the network game, the keyboard is back.
        i.end_net();
        assert!(i.sources().contains(&Source::Keyboard));
        assert!(i.read(Source::Keyboard).stick.y > 0.9);
    }

    #[test]
    fn esc_is_start_to_the_network() {
        let mut i = Input::new();
        i.start_net(0, 2);
        assert!(i.take_local(true).unpack().start_pressed);
        assert!(!i.take_local(false).unpack().start_pressed);
    }
}
