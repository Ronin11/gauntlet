//! "Press Start to join" screen: each controller (or the keyboard) joins, picks a
//! hero and readies up.

use crate::game::PLAYER_COLORS;
use crate::hud::{Canvas, PLAYER_RGB};
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
    ("FAL", "FALCONER"),
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
    /// Process input. Returns the chosen (class, colour) list once everyone is ready.
    pub fn update(&mut self, input: &Input, dt: f32) -> Option<Vec<(String, String)>> {
        let mut sources = vec![Source::Keyboard];
        sources.extend(input.pads().into_iter().map(Source::Pad));
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

    pub fn draw(&self, c: &mut Canvas, time: f32) {
        let scale = (c.height() / 360.0)
            .min(c.width() / 560.0)
            .clamp(1.0, 4.0)
            .floor();
        let title = "GAUNTLET  DARK LEGACY";
        let s = (scale * 3.0)
            .min((c.width() * 0.95 / (title.len() as f32 * 8.0)).floor())
            .max(1.0);
        c.text_shadow(
            (c.width() - title.len() as f32 * 8.0 * s) * 0.5,
            c.height() * 0.1,
            s,
            [1.0, 0.8, 0.2, 1.0],
            title,
        );
        let w = (c.width() - 10.0 * scale) / 4.0;
        for i in 0..4 {
            let x = 5.0 * scale + i as f32 * w;
            let y = c.height() * 0.35;
            let rgb = PLAYER_RGB[i];
            c.rect(
                x + 4.0 * scale,
                y,
                w - 8.0 * scale,
                c.height() * 0.4,
                [rgb[0] * 0.2, rgb[1] * 0.2, rgb[2] * 0.2, 0.85],
            );
            c.rect(
                x + 4.0 * scale,
                y,
                w - 8.0 * scale,
                3.0 * scale,
                [rgb[0], rgb[1], rgb[2], 1.0],
            );
            let cx = |len: usize, sc: f32| x + (w - len as f32 * 8.0 * sc) * 0.5;
            match self.slots.get(i) {
                None => {
                    if (time * 2.0).fract() < 0.6 {
                        let t = "PRESS START";
                        c.text_shadow(
                            cx(t.len(), scale),
                            y + 40.0 * scale,
                            scale,
                            [1.0, 1.0, 1.0, 1.0],
                            t,
                        );
                    }
                }
                Some(slot) => {
                    let name = HEROES[slot.hero].1;
                    let ns = scale * 2.0;
                    let arrows = if slot.ready {
                        name.to_string()
                    } else {
                        format!("< {name} >")
                    };
                    c.text_shadow(
                        cx(arrows.len(), ns),
                        y + 30.0 * scale,
                        ns,
                        [rgb[0], rgb[1], rgb[2], 1.0],
                        &arrows,
                    );
                    let dev = match slot.source {
                        Source::Keyboard => "KEYBOARD",
                        Source::Pad(_) => "CONTROLLER",
                    };
                    c.text_shadow(
                        cx(dev.len(), scale),
                        y + 60.0 * scale,
                        scale,
                        [0.7, 0.7, 0.7, 1.0],
                        dev,
                    );
                    let st = if slot.ready { "READY!" } else { "A TO CONFIRM" };
                    c.text_shadow(
                        cx(st.len(), scale),
                        y + 80.0 * scale,
                        scale,
                        [1.0, 1.0, 0.6, 1.0],
                        st,
                    );
                }
            }
        }
        let help = "LEFT/RIGHT CHOOSE HERO   A / ENTER READY   B BACK";
        c.text_shadow(
            (c.width() - help.len() as f32 * 8.0 * scale) * 0.5,
            c.height() * 0.85,
            scale,
            [0.8, 0.8, 0.8, 1.0],
            help,
        );
    }
}
