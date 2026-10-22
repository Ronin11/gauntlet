//! "Press Start to join" screen: each controller (or the keyboard) joins, picks a
//! hero and readies up.

use crate::game::PLAYER_COLORS;
use crate::hud::{Canvas, Style, DIM, GOLD, PLAYER_RGB};
use crate::input::{Input, Source};

/// Selectable heroes: (directory, display name). The first four are the classic
/// classes; the rest are the Dark Legacy unlockables.
pub const HEROES: &[(&str, &str)] = &[
    ("WAR", "WARRIOR"),
    ("VAL", "VALKYRIE"),
    ("WIZ", "WIZARD"),
    ("ARC", "ARCHER"),
    ("DWF", "DWARF"),
    ("KNI", "KNIGHT"),
    ("SOR", "SORCERESS"),
    ("JES", "JESTER"),
    ("FAL", "FALCONESS"),
    ("JAC", "JACKAL"),
    ("TIG", "TIGRESS"),
    ("UNI", "UNICORN"),
    ("MED", "MEDUSA"),
    ("MIN", "MINOTAUR"),
    ("OGR", "OGRE"),
    ("HYE", "HYENA"),
];

struct Slot {
    source: Source,
    hero: usize,
    ready: bool,
    last_x: f32,
}

#[derive(Default)]
pub struct Select {
    slots: Vec<Slot>,
    countdown: Option<f32>,
}

impl Select {
    /// A sample screen for layout checks: three players joined, one ready.
    pub fn demo() -> Select {
        let slot = |hero, ready| Slot {
            source: Source::Keyboard,
            hero,
            ready,
            last_x: 0.0,
        };
        Select {
            slots: vec![slot(0, false), slot(5, true), slot(10, false)],
            countdown: None,
        }
    }

    /// A join screen with `first` already joined (the controller that left the title).
    pub fn with_first(first: Source) -> Select {
        Select {
            slots: vec![Slot {
                source: first,
                hero: 0,
                ready: false,
                last_x: 0.0,
            }],
            countdown: None,
        }
    }

    /// Process input. Returns the chosen (class, colour) list once everyone is ready.
    pub fn update(&mut self, input: &Input, dt: f32) -> Option<Vec<(String, String)>> {
        let sources = input.sources();
        for src in sources {
            let inp = input.read(src);
            let slot = self.slots.iter().position(|s| s.source == src);
            match slot {
                None => {
                    if (inp.start_pressed || inp.attack_pressed) && self.slots.len() < 4 {
                        // New players default to the next classic hero nobody has taken.
                        let hero = (0..4)
                            .find(|h| !self.slots.iter().any(|s| s.hero == *h))
                            .unwrap_or(0);
                        self.slots.push(Slot {
                            source: src,
                            hero,
                            ready: false,
                            last_x: 0.0,
                        });
                        self.countdown = None;
                    }
                }
                Some(i) => {
                    let s = &mut self.slots[i];
                    let x = inp.stick.x;
                    if !s.ready {
                        if x > 0.5 && s.last_x <= 0.5 {
                            s.hero = (s.hero + 1) % HEROES.len();
                        }
                        if x < -0.5 && s.last_x >= -0.5 {
                            s.hero = (s.hero + HEROES.len() - 1) % HEROES.len();
                        }
                        if inp.attack_pressed || inp.start_pressed {
                            s.ready = true;
                        }
                    } else if inp.magic_pressed {
                        s.ready = false;
                        self.countdown = None;
                    }
                    s.last_x = x;
                }
            }
        }
        if !self.slots.is_empty() && self.slots.iter().all(|s| s.ready) {
            let t = self.countdown.get_or_insert(1.5);
            *t -= dt;
            if *t <= 0.0 {
                return Some(
                    self.slots
                        .iter()
                        .enumerate()
                        .map(|(i, s)| (HEROES[s.hero].0.to_string(), PLAYER_COLORS[i].to_string()))
                        .collect(),
                );
            }
        }
        None
    }

    /// Input sources in player order (for the game that follows).
    pub fn sources(&self) -> Vec<Source> {
        self.slots.iter().map(|s| s.source).collect()
    }

    pub fn draw(&self, c: &mut Canvas, st: &Style, time: f32) {
        let ui = st.ui;
        let (w, h) = (c.width(), c.height());
        let u = (h / 720.0).min(w / 1120.0);
        c.rect(0.0, 0.0, w, h, [0.0, 0.0, 0.0, 1.0]);
        st.backdrop(c, 0.82);
        st.centered(c, w * 0.5, 22.0 * u, 50.0 * u, GOLD, "Choose Your Hero");

        // Four tall frames (S1_BORDER over the S1_PLYRn nebula), 1:2 like the art.
        let gap = 18.0 * u;
        let fw = ((w - gap * 5.0) / 4.0).min((h - 190.0 * u) * 0.5);
        let fh = fw * 2.0;
        let x0 = (w - (fw * 4.0 + gap * 3.0)) * 0.5;
        let y = 92.0 * u;
        for i in 0..4 {
            let x = x0 + i as f32 * (fw + gap);
            let rgb = PLAYER_RGB[i];
            let col = [rgb[0], rgb[1], rgb[2], 1.0];
            let slot = self.slots.get(i);
            let lit = if slot.is_some() { 1.0 } else { 0.45 };
            match ui.get(&format!("S1_PLYR{}", i + 1)) {
                Some(t) => c.sprite(t, x, y, fw, fh, [lit, lit, lit, 1.0]),
                None => c.rect(
                    x,
                    y,
                    fw,
                    fh,
                    [rgb[0] * 0.2, rgb[1] * 0.2, rgb[2] * 0.2, 1.0],
                ),
            }
            match slot {
                None => {
                    if (time * 1.6).fract() < 0.7 {
                        st.centered(c, x + fw * 0.5, y + fh * 0.4, 34.0 * u, col, "Press");
                        st.centered(
                            c,
                            x + fw * 0.5,
                            y + fh * 0.4 + 40.0 * u,
                            34.0 * u,
                            col,
                            "Start",
                        );
                    }
                }
                Some(slot) => {
                    let (code, name) = HEROES[slot.hero];
                    let color = PLAYER_COLORS[i];
                    let portrait = ui.find(&[
                        &format!("S12_{code}_{color}"),
                        &format!("S12_{code}_"),
                        &format!("S12_{code}"),
                    ]);
                    if let Some(t) = portrait {
                        c.sprite(t, x, y, fw, fh, [1.0; 4]);
                    }
                    // Name banner along the bottom of the frame.
                    let by = y + fh - fh * 0.16;
                    c.rect(x, by - 6.0 * u, fw, fh * 0.13, [0.0, 0.0, 0.0, 0.55]);
                    match ui.get(&format!("{code}_NAME")) {
                        Some(t) => {
                            let bw = fw * 0.86;
                            let bh = bw * t.h as f32 / t.w as f32;
                            c.sprite(t, x + (fw - bw) * 0.5, by, bw, bh, GOLD);
                        }
                        None => st.centered(c, x + fw * 0.5, by, 30.0 * u, GOLD, name),
                    }
                    if slot.ready {
                        st.centered(c, x + fw * 0.5, y + fh * 0.06, 40.0 * u, col, "Ready!");
                    } else {
                        // Arrows either side while choosing.
                        let a = 64.0 * u;
                        let ay = y + fh * 0.45;
                        let pulse = 0.7 + 0.3 * (time * 5.0).sin();
                        if let Some(t) = ui.get("MARKER_LEFT") {
                            let aw = a * 2.0;
                            c.sprite(t, x - aw * 0.25, ay, aw, a, [pulse, pulse, pulse, 1.0]);
                        }
                        if let Some(t) = ui.get("MARKER_RIGHT") {
                            let aw = a * 2.0;
                            c.sprite(t, x + fw - aw * 0.75, ay, aw, a, [pulse, pulse, pulse, 1.0]);
                        }
                    }
                }
            }
            match ui.get("S1_BORDER") {
                Some(t) => c.sprite(
                    t,
                    x,
                    y,
                    fw,
                    fh,
                    [
                        0.85 + rgb[0] * 0.15,
                        0.85 + rgb[1] * 0.15,
                        0.85 + rgb[2] * 0.15,
                        1.0,
                    ],
                ),
                None => c.rect(x, y, fw, 3.0 * u, col),
            }
            // Which device the player is using, under the frame.
            if let Some(slot) = slot {
                let dev = match slot.source {
                    Source::Keyboard => "KEYBOARD",
                    Source::Pad(_) | Source::Device(_) => "CONTROLLER",
                };
                let dw = st.small_width(dev, 16.0 * u);
                st.small(
                    c,
                    x + (fw - dw) * 0.5,
                    y + fh + 10.0 * u,
                    16.0 * u,
                    DIM,
                    dev,
                );
            }
        }
        let help = "LEFT/RIGHT  CHOOSE      A  READY      B  BACK";
        let hw = st.small_width(help, 18.0 * u);
        st.small(c, (w - hw) * 0.5, h - 36.0 * u, 18.0 * u, DIM, help);
    }
}
