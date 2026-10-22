//! The game's list-menu engine (OPTMENU in options.c): the title menu, the
//! options screens and the per-player menus on the select screen.

use crate::input::PlayerInput;
use crate::screen::{glow_alpha, Scr};
use crate::ui::FONT_32;

#[derive(Clone, Debug)]
pub struct Item {
    pub text: String,
    pub code: i32,
    /// Disabled items are drawn half transparent and can't be chosen.
    pub enabled: bool,
}

impl Item {
    pub fn new(text: &str, code: i32) -> Item {
        Item {
            text: text.into(),
            code,
            enabled: true,
        }
    }
}

/// What a menu did this tick (do_optmenu's return codes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    None,
    /// The cursor moved (S_OPTMENUMOVVRT).
    Moved,
    /// A on an item: its code.
    Chose(i32),
    /// A on a disabled item (S_NO).
    Refused,
    Back,
    Left,
    Right,
    /// Up / down when the menu doesn't navigate itself.
    Up,
    Down,
}

/// Edge-triggered directions from the stick / d-pad.
#[derive(Clone, Copy, Debug, Default)]
pub struct Dirs {
    last: (i32, i32),
}

impl Dirs {
    /// New presses this tick: (x, y) each -1, 0 or 1, y = 1 meaning up.
    pub fn edges(&mut self, input: &PlayerInput) -> (i32, i32) {
        let q = |v: f32| (v > 0.5) as i32 - (v < -0.5) as i32;
        let now = (q(input.stick.x), q(input.stick.y));
        let e = (
            if now.0 != self.last.0 { now.0 } else { 0 },
            if now.1 != self.last.1 { now.1 } else { 0 },
        );
        self.last = now;
        e
    }
}

/// controls.c rep_speed: ticks between auto-repeats while a direction is held.
const REP_SPEED: [u32; 13] = [30, 20, 10, 6, 3, 3, 3, 3, 2, 2, 2, 2, 1];

/// Directions with the game's auto-repeat edges (PlayerControls).
#[derive(Clone, Copy, Debug, Default)]
pub struct Repeat {
    held: (i32, i32),
    count: u32,
    index: usize,
}

impl Repeat {
    /// This tick's presses and repeats: (x, y) each -1, 0 or 1, y = 1 meaning up.
    pub fn edges(&mut self, input: &PlayerInput) -> (i32, i32) {
        let q = |v: f32| (v > 0.5) as i32 - (v < -0.5) as i32;
        let now = (q(input.stick.x), q(input.stick.y));
        if now == (0, 0) || now != self.held {
            let e = (
                if now.0 != self.held.0 { now.0 } else { 0 },
                if now.1 != self.held.1 { now.1 } else { 0 },
            );
            self.held = now;
            self.count = 0;
            self.index = 0;
            return e;
        }
        self.count += 1;
        if self.count >= REP_SPEED[self.index] {
            self.count -= REP_SPEED[self.index];
            self.index = (self.index + 1).min(REP_SPEED.len() - 1);
            return now;
        }
        (0, 0)
    }
}

#[derive(Clone, Debug)]
pub struct OptMenu {
    pub items: Vec<Item>,
    /// Left edge of the items, or -centre when negative.
    pub x: f32,
    /// Top of the first item.
    pub y: f32,
    pub scale: f32,
    pub rgb_off: u32,
    pub rgb_on: u32,
    pub rgb_hi: u32,
    pub sel: usize,
    /// Menu clock in ticks; drives the selected item's glow.
    pub time: u32,
    /// START counts as A (OPTMENU_TITLE).
    pub title: bool,
    /// Extra space under every item (the menu entries' dy; 4 in the memory
    /// card lists).
    pub dy: f32,
    dirs: Dirs,
}

impl OptMenu {
    pub fn new(items: Vec<Item>, x: f32, y: f32, scale: f32) -> OptMenu {
        OptMenu {
            items,
            x,
            y,
            scale,
            rgb_off: 0xB4320A,
            rgb_on: 0xFFFFFF,
            rgb_hi: 0x8200EA,
            sel: 0,
            time: 0,
            title: false,
            dy: 0.0,
            dirs: Dirs::default(),
        }
    }

    /// Line advance: FontHeight(6, scale) + the items' dy.
    pub fn line_height(&self) -> f32 {
        (32.0 * self.scale).floor() + self.dy
    }

    /// do_optmenu: advance the clock by `ticks` and handle one player's input.
    pub fn update(&mut self, input: &PlayerInput, ticks: u32, nav: bool) -> Event {
        self.time += ticks;
        let n = self.items.len();
        if n == 0 {
            return Event::None;
        }
        if !self.items.iter().any(|i| i.enabled) {
            return Event::None;
        }
        while !self.items[self.sel % n].enabled {
            self.sel = (self.sel + 1) % n;
        }
        self.sel %= n;
        let (dx, dy) = self.dirs.edges(input);
        if input.attack_pressed || (self.title && input.start_pressed) {
            let it = &self.items[self.sel];
            return if it.enabled {
                Event::Chose(it.code)
            } else {
                Event::Refused
            };
        }
        if input.b_pressed {
            return Event::Back;
        }
        if dx < 0 {
            return Event::Left;
        }
        if dx > 0 {
            return Event::Right;
        }
        if dy != 0 {
            if !nav {
                return if dy > 0 { Event::Up } else { Event::Down };
            }
            let step = if dy > 0 { n - 1 } else { 1 };
            let mut s = self.sel;
            loop {
                s = (s + step) % n;
                if self.items[s].enabled || s == self.sel {
                    break;
                }
            }
            if s != self.sel {
                self.sel = s;
                return Event::Moved;
            }
        }
        Event::None
    }

    /// The 3D arrow's place for menus with an icon (the title menu): left of the
    /// item block, on the selected line, pitched pi on odd items.
    pub fn cursor(&self, s: &Scr) -> (f32, f32, f32) {
        let mut x = self.x;
        if x < 0.0 {
            let w = self
                .items
                .iter()
                .map(|it| s.text_width(FONT_32, self.scale, &it.text))
                .fold(0.0, f32::max);
            x = -(x + w / 2.0);
        }
        let lh = self.line_height();
        let angle = if self.sel & 1 != 0 {
            std::f32::consts::PI
        } else {
            0.0
        };
        (
            x - 16.0,
            self.y + self.sel as f32 * lh + (lh / 2.0).floor(),
            angle,
        )
    }

    /// show_optmenu: unselected items in rgb_off; the selected one twice, a pulsing
    /// glow pass in rgb_hi then solid rgb_on. `fade` (0..255) fades everything.
    pub fn draw(&self, s: &mut Scr, fade: i32) {
        let lh = self.line_height();
        for (i, it) in self.items.iter().enumerate() {
            let y = self.y + i as f32 * lh;
            let text_fade = if it.enabled { fade } else { fade.max(0x80) };
            if i == self.sel {
                let a = (fade + glow_alpha(self.time)).min(255);
                s.glow_pass(self.scale, self.x, y, self.rgb_hi, a, &it.text);
                s.text(
                    self.scale,
                    self.x,
                    y,
                    FONT_32,
                    self.rgb_on,
                    text_fade,
                    &it.text,
                );
            } else {
                s.text(
                    self.scale,
                    self.x,
                    y,
                    FONT_32,
                    self.rgb_off,
                    text_fade,
                    &it.text,
                );
            }
        }
    }
}
