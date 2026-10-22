//! Title screen, loading screen and pause menu, drawn in the game's own style
//! (see [`crate::hud::Style`]).

use crate::hud::{Canvas, Style, DIM, GOLD};
use crate::input::{PlayerInput, Source};

/// Up/down navigation shared by the menus: one step per stick push.
#[derive(Default)]
struct Nav {
    last_y: f32,
}

impl Nav {
    /// -1 (up), +1 (down) or 0 for this tick.
    fn step(&mut self, inputs: &[(Source, PlayerInput)]) -> i32 {
        let y = inputs
            .iter()
            .map(|(_, i)| i.stick.y)
            .max_by(|a, b| a.abs().total_cmp(&b.abs()))
            .unwrap_or(0.0);
        let s = if y > 0.5 && self.last_y <= 0.5 {
            -1
        } else if y < -0.5 && self.last_y >= -0.5 {
            1
        } else {
            0
        };
        self.last_y = y;
        s
    }
}

/// "Press Start" title screen. Returns the source that pressed start.
#[derive(Default)]
pub struct Title {
    time: f32,
}

impl Title {
    pub fn update(&mut self, inputs: &[(Source, PlayerInput)], dt: f32) -> Option<Source> {
        self.time += dt;
        // Ignore presses carried over from the previous screen.
        if self.time < 0.4 {
            return None;
        }
        inputs
            .iter()
            .find(|(_, i)| i.start_pressed || i.attack_pressed)
            .map(|(s, _)| *s)
    }

    pub fn draw(&self, c: &mut Canvas, st: &Style, time: f32) {
        let (w, h) = (c.width(), c.height());
        st.backdrop(c, 0.6);
        let u = (h / 720.0).min(w / 1120.0);
        let lw = (w * 0.62).min(h * 1.3);
        let lh = st.logo(c, w * 0.5, h * 0.12, lw, time);
        if (time * 1.6).fract() < 0.7 {
            st.centered(
                c,
                w * 0.5,
                h * 0.12 + lh + 70.0 * u,
                54.0 * u,
                GOLD,
                "Press Start",
            );
        }
        let hint = "A / ENTER";
        let hw = st.small_width(hint, 18.0 * u);
        st.small(
            c,
            (w - hw) * 0.5,
            h * 0.12 + lh + 140.0 * u,
            18.0 * u,
            DIM,
            hint,
        );
    }
}

/// Full-screen art shown while a level loads.
pub fn draw_loading(c: &mut Canvas, st: &Style, level: &str) {
    let (w, h) = (c.width(), c.height());
    st.backdrop(c, 0.0);
    let u = (h / 720.0).min(w / 1120.0);
    let label = "Loading";
    let lw = st.width(label, 44.0 * u);
    st.text(c, w - lw - 40.0 * u, h - 80.0 * u, 44.0 * u, GOLD, label);
    let code = level.trim_start_matches("level");
    let cw = st.small_width(code, 18.0 * u);
    st.small(c, w - cw - 40.0 * u, h - 30.0 * u, 18.0 * u, DIM, code);
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PauseAction {
    Resume,
    ChangeHeroes,
    QuitToTitle,
    Quit,
}

const PAUSE_ITEMS: [(&str, PauseAction); 4] = [
    ("Continue", PauseAction::Resume),
    ("Change Heroes", PauseAction::ChangeHeroes),
    ("Quit to Title", PauseAction::QuitToTitle),
    ("Quit Game", PauseAction::Quit),
];

/// In-game pause menu.
#[derive(Default)]
pub struct Pause {
    sel: usize,
    nav: Nav,
    time: f32,
}

impl Pause {
    pub fn update(&mut self, inputs: &[(Source, PlayerInput)], dt: f32) -> Option<PauseAction> {
        self.time += dt;
        let n = PAUSE_ITEMS.len();
        match self.nav.step(inputs) {
            -1 => self.sel = (self.sel + n - 1) % n,
            1 => self.sel = (self.sel + 1) % n,
            _ => {}
        }
        if self.time < 0.2 {
            return None;
        }
        if inputs
            .iter()
            .any(|(_, i)| i.magic_pressed || i.start_pressed)
        {
            return Some(PauseAction::Resume);
        }
        if inputs.iter().any(|(_, i)| i.attack_pressed) {
            return Some(PAUSE_ITEMS[self.sel].1);
        }
        None
    }

    pub fn draw(&self, c: &mut Canvas, st: &Style, time: f32) {
        let (w, h) = (c.width(), c.height());
        let u = (h / 720.0).min(w / 1120.0);
        c.rect(0.0, 0.0, w, h, [0.0, 0.0, 0.0, 0.55]);
        let pw = 520.0 * u;
        let ph = 400.0 * u;
        let (px, py) = ((w - pw) * 0.5, (h - ph) * 0.5);
        st.panel(c, px, py, pw, ph, [1.0; 4]);
        st.centered(c, w * 0.5, py + 34.0 * u, 56.0 * u, GOLD, "Paused");
        for (k, (label, _)) in PAUSE_ITEMS.iter().enumerate() {
            let y = py + 128.0 * u + k as f32 * 58.0 * u;
            let on = k == self.sel;
            let tw = st.width(label, 40.0 * u);
            let x = (w - tw) * 0.5;
            st.text(c, x, y, 40.0 * u, if on { GOLD } else { DIM }, label);
            if on {
                st.marker(c, x - 52.0 * u, y + 2.0 * u, 36.0 * u, time);
            }
        }
    }
}
