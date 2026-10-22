//! Title screen, loading screen and pause menu, drawn in the game's own style
//! (see [`crate::hud::Style`]).

use crate::input::{PlayerInput, Source};
use crate::optmenu::{Event, Item, OptMenu};
use crate::screen::Scr;

/// The title screen (attract.c do_titlescreen): TITLE00-03 with the animated
/// GLOWCROP glow, "Press Start", then the Start / Options title menu for the pad
/// that pressed it.
pub struct Title {
    /// Ticks since the screen opened (lbl_80344270).
    ticks: u32,
    /// The pad that pressed Start and owns the menu.
    owner: Option<Source>,
    menu: OptMenu,
    open: bool,
    /// Ticks left of the glow's fade after choosing Start (lbl_80344274).
    starting: Option<i32>,
    /// Sounds to play, drained by the caller.
    pub sfx: Vec<&'static str>,
    /// The Options screens, opened from the title menu.
    pub options: Option<crate::options::Options>,
    /// Their sounds, and whether a volume changed, for the caller.
    pub option_sfx: Vec<String>,
    pub volume_changed: bool,
    /// Idle ticks left before the attract loop resumes (lbl_80344260).
    idle: i32,
    /// The idle timer ran out: back to the attract loop.
    pub timed_out: bool,
}

impl Default for Title {
    fn default() -> Self {
        let mut menu = OptMenu::new(
            vec![Item::new("Start", 11), Item::new("Options", 12)],
            -256.0,
            304.0,
            1.0,
        );
        menu.title = true;
        Title {
            ticks: 0,
            owner: None,
            menu,
            open: false,
            starting: None,
            sfx: Vec::new(),
            options: None,
            option_sfx: Vec::new(),
            volume_changed: false,
            idle: 1800,
            timed_out: false,
        }
    }
}

impl Title {
    /// Where the menus' 3D arrow goes, if a menu with one is showing.
    pub fn cursor(&self, s: &Scr) -> Option<(f32, f32, f32)> {
        match &self.options {
            Some(o) => o.cursor(s),
            None if self.open && self.starting.is_none() => Some(self.menu.cursor(s)),
            None => None,
        }
    }

    /// The screen after its fade-in, for snapshots; `menu` opens the title menu.
    pub fn demo(menu: bool) -> Title {
        Title {
            ticks: 100,
            owner: menu.then_some(Source::Keyboard),
            open: menu,
            ..Title::default()
        }
    }

    /// One 60 Hz tick. Returns the pad that chose Start once the glow has faded.
    pub fn update(
        &mut self,
        inputs: &[(Source, PlayerInput)],
        prefs: &mut crate::options::Prefs,
    ) -> Option<Source> {
        self.ticks += 1;
        // The Options screens take over the owner's input while open; the idle
        // timer doesn't run under them.
        if let Some(o) = &mut self.options {
            let input = inputs
                .iter()
                .find(|(s, _)| Some(*s) == self.owner)
                .map(|(_, i)| *i)
                .unwrap_or_default();
            let action = o.update(&input, prefs);
            self.option_sfx.extend(o.sfx.drain(..));
            if action == crate::options::OptionsAction::VolumeChanged {
                self.volume_changed = true;
            }
            if !o.is_drawing() {
                self.options = None;
            }
            return None;
        }
        if let Some(t) = &mut self.starting {
            *t -= 1;
            return (*t <= 0).then_some(self.owner).flatten();
        }
        self.idle -= 1;
        if self.idle <= 0 {
            self.timed_out = true;
            return None;
        }
        let Some(owner) = self.owner else {
            // new_start: a Start press assigns that pad, and the menu opens.
            if self.ticks > 1 {
                if let Some((src, _)) = inputs.iter().find(|(_, i)| i.start_pressed) {
                    self.owner = Some(*src);
                    self.open = true;
                    self.sfx.push("S_OPTMENUSEL");
                    // The press that opened the menu doesn't also choose.
                    let _ = self.menu.update(&PlayerInput::default(), 0, true);
                }
            }
            return None;
        };
        let input = inputs
            .iter()
            .find(|(s, _)| *s == owner)
            .map(|(_, i)| *i)
            .unwrap_or_default();
        match self.menu.update(&input, 1, true) {
            Event::Chose(11) => {
                self.open = false;
                self.starting = Some(30);
                self.sfx.push("S_CHOOSESELSFX");
            }
            Event::Chose(12) => {
                self.sfx.push("S_OPTMENUSEL");
                self.options = Some(crate::options::Options::title(0));
            }
            Event::Chose(_) | Event::Refused => self.sfx.push("S_NO"),
            Event::Moved => {
                self.sfx.push("S_OPTMENUMOVVRT");
                self.idle = 1800;
            }
            Event::Back => {
                // B closes the menu and gives up the pad.
                self.open = false;
                self.owner = None;
            }
            _ => {}
        }
        None
    }

    pub fn draw(&self, s: &mut Scr) {
        s.bars(1.0);
        s.quad_screen("TITLE", 0);
        let mut fade = if self.ticks < 60 {
            255 - (self.ticks as i32 * 255) / 60
        } else {
            0
        };
        if let Some(t) = self.starting {
            fade = 255 - (t * 60) / 30;
        } else if self.idle < 30 {
            // The glow fades out over the last half second before the attract loop.
            fade = 255 - (self.idle.max(0) * 255) / 30;
        }
        if let Some(o) = &self.options {
            o.draw(s);
            return;
        }
        if let Some(t) = s.ui.frame("GLOWCROP_00", (self.ticks as usize >> 2) % 10) {
            s.blit(t, 192.0, 0.0, 128.0, 128.0, fade);
        }
        if self.starting.is_some() {
            s.glow_text(1.0, -256.0, 320.0, "Loading...");
        } else if self.open {
            self.menu.draw(s, 0);
        } else {
            s.glow_text(1.0, -256.0, 320.0, "Press Start");
        }
    }
}

/// The loading screen, as shop.c draws it while the next level loads:
/// TransitionBlitShow(0) then DrawGlowText(1.0, 340, 260, "Loading...").
pub fn draw_loading(s: &mut Scr) {
    transition(s, 0);
    s.glow_text(1.0, 340.0, 260.0, "Loading...");
}

/// TransitionBlitShow: TRANSITION_SCREEN stretched over the 512x384 screen, at
/// MBBlitSetAlpha `fade`.
pub fn transition(s: &mut Scr, fade: i32) {
    s.bars(crate::screen::fade_opacity(fade));
    if let Some(t) = s.ui.get("TRANSITION_SCREEN") {
        s.blit(t, 0.0, 0.0, 512.0, 384.0, fade);
    }
}

/// The loading art's MBBlitSetAlpha fade `t` seconds into a level: gauntworld.c
/// fades it out as 255 * (1 - t / 2) over the first two seconds.
pub fn loading_fade(t: f32) -> i32 {
    255 - (255.0 * (1.0 - 0.5 * t)).clamp(0.0, 255.0) as i32
}

/// ControllerMessageBox: a scroll's text on parchment, centred on (256, 160),
/// with "Press [B] Button when done." under it.
pub fn draw_scroll(s: &mut Scr, text: &str) {
    use crate::ui::FONT_32;
    const PROMPT: &str = "Press     Button when done.";
    let scale = 0.5;
    let lines: Vec<&str> = text.split('\n').collect();
    let lh = s.font_height(FONT_32, scale) + 4.0;
    let tw = lines
        .iter()
        .map(|l| s.text_width(FONT_32, scale, l))
        .fold(0.0, f32::max);
    let min_w = s.text_width(FONT_32, scale, PROMPT) + 32.0;
    let w = (tw + 96.0).clamp(min_w, 512.0);
    let h = lines.len() as f32 * lh + 96.0;
    let (left, top) = (256.0 - w / 2.0, 160.0 - h / 2.0);
    if let Some(t) = s.ui.get("SCROLL_A") {
        s.blit(t, left, top, w, h, 0);
    }
    let mut y = top + 32.0;
    for l in &lines {
        s.text(scale, -256.0, y, FONT_32, 0x160C03, 0, l);
        y += lh;
    }
    let py = y + 8.0;
    s.glow_text(scale, -256.0, py, PROMPT);
    if let Some(t) = s.ui.get("BUTTON_TRI") {
        s.blit(t, 190.0, py - 2.0, 20.0, 20.0, 0);
    }
}
