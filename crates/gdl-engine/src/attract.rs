//! The attract sequence (attract.c init_attract_mode): power-on movies, the title,
//! and the loop of group shots and character movies shown when nobody plays.

use crate::fs::GameFs;
use crate::input::PlayerInput;
use crate::movie::Movie;
use crate::screen::Scr;

/// One entry of the attract screen list (lbl_80118188 + 0x10).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Entry {
    /// A movie row of the movie table (lbl_80118200).
    Movie(usize),
    /// A Screen2D slot: 0 is the one-frame prefs-load step, 1 and 2 the group shot.
    Screen2D(usize),
    Title,
    /// A level flyby (MA_FLYBY).
    Flyby,
    /// TODO: the credits roll (MA_CREDITS, every 16th loop) is not implemented.
    Credits,
}

const SCREENS: [Entry; 12] = [
    Entry::Movie(0),
    Entry::Movie(1),
    Entry::Movie(2),
    Entry::Screen2D(0),
    Entry::Title,
    Entry::Screen2D(1),
    Entry::Flyby,
    Entry::Movie(3),
    Entry::Screen2D(2),
    Entry::Flyby,
    Entry::Credits,
    Entry::Movie(4),
];

/// Movie rows by loop column (column 0 when a row has only one movie).
const MOVIES: [[&str; 4]; 5] = [
    ["midway"; 4],
    ["OPENING"; 4],
    ["TITLE2"; 4],
    ["WAR_DWA", "VAL_KNI", "WIZ_SOR", "JES_ARC"],
    ["FAIL"; 4],
];

/// What the attract loop is showing.
pub enum Show {
    Movie(Box<Movie>),
    /// A four-quadrant picture with "Press Start", for `ticks` more ticks.
    Screen2D {
        prefix: &'static str,
        ticks: i32,
    },
    /// The credits roll over CRED_ALL: three blocks of lines from the DOL.
    Credits {
        blocks: [Vec<String>; 3],
        scroll: i32,
        ticks: i32,
    },
}

pub enum Event {
    None,
    /// Go to the title screen.
    Title,
    /// A movie started: play its soundtrack.
    MovieStarted,
    /// A movie ended or was skipped: stop its soundtrack.
    MovieEnded,
    /// The credits started: play castle2b.
    Credits,
    /// A flyby of this level directory: load it and fly through (MA_FLYBY).
    Flyby(String),
}

pub struct Attract {
    /// Index of the current entry in [`SCREENS`].
    index: usize,
    /// Completed loops (lbl_803441F4) and the character-movie column.
    loops: u32,
    column: usize,
    /// Past the first Screen2D step: START now jumps to the title.
    started: bool,
    /// Set after a game over so the "fail" movie plays.
    pub game_over: bool,
    pub show: Option<Show>,
    /// NextAttractWave's cursor and the realm tables it walks.
    waves: crate::flyby::WaveCursor,
    worlds: Option<gdl_formats::wdata::Worlds>,
}

impl Attract {
    /// From power-on: the first movie is next.
    pub fn new() -> Attract {
        Attract {
            index: usize::MAX,
            loops: 0,
            column: 0,
            started: false,
            game_over: false,
            show: None,
            waves: Default::default(),
            worlds: None,
        }
    }

    /// The flyby has ended (or couldn't run): on to the next entry.
    pub fn flyby_over(&mut self, fs: &GameFs) -> Event {
        self.show = None;
        self.advance(fs)
    }

    /// The credits roll `scroll` pixels in, for snapshots.
    pub fn credits_demo(fs: &GameFs, scroll: i32) -> Attract {
        let mut a = Attract::new();
        a.show = credit_lines(fs).map(|blocks| Show::Credits {
            blocks,
            scroll,
            ticks: 0,
        });
        a
    }

    /// After "Quit Game" or a game over (MG_OVER): the "fail" movie plays next.
    pub fn game_over(mut self) -> Attract {
        self.game_over = true;
        self.started = true;
        self.index = 10;
        self.show = None;
        self
    }

    /// Back into the loop after the title timed out (the entry after the title).
    pub fn after_title(mut self) -> Attract {
        self.index = 4;
        self.show = None;
        self
    }

    /// Step to the next entry that can be shown, applying the skip rules.
    /// Returns Title when the title screen is next.
    fn advance(&mut self, fs: &GameFs) -> Event {
        loop {
            self.index = self.index.wrapping_add(1);
            if self.index >= SCREENS.len() {
                self.index = 0;
                self.loops += 1;
                self.column = (self.column + 1) % 4;
            }
            match SCREENS[self.index] {
                Entry::Movie(4) if !self.game_over => continue,
                Entry::Movie(row) => {
                    let name = MOVIES[row][self.column];
                    match Movie::open(fs, name) {
                        Ok(m) => {
                            self.show = Some(Show::Movie(Box::new(m)));
                            return Event::MovieStarted;
                        }
                        Err(e) => {
                            log::warn!("movie {name}: {e:#}");
                            continue;
                        }
                    }
                }
                Entry::Screen2D(0) => {
                    if !self.started {
                        self.started = true;
                    }
                    continue;
                }
                Entry::Screen2D(_) => {
                    self.show = Some(Show::Screen2D {
                        prefix: "GROUP_SHOT_MIDWAY",
                        ticks: 60,
                    });
                    return Event::None;
                }
                Entry::Title if self.loops == 0 => return Event::Title,
                // The credits roll every 16th loop.
                Entry::Credits if self.loops & 15 == 15 => match credit_lines(fs) {
                    Some(blocks) => {
                        self.show = Some(Show::Credits {
                            blocks,
                            scroll: 0,
                            ticks: 0,
                        });
                        return Event::Credits;
                    }
                    None => continue,
                },
                Entry::Flyby => {
                    if self.worlds.is_none() {
                        self.worlds = gdl_formats::wdata::Worlds::load(|p| fs.read(p)).ok();
                    }
                    let Some(w) = &self.worlds else { continue };
                    let wave = self.waves.next(w);
                    match w.dir(wave) {
                        Some(dir) => return Event::Flyby(dir),
                        None => continue,
                    }
                }
                Entry::Title | Entry::Credits => continue,
            }
        }
    }

    /// One 60 Hz tick.
    pub fn update(&mut self, fs: &GameFs, inputs: &[PlayerInput]) -> Event {
        let ended = match &mut self.show {
            None => true,
            Some(Show::Movie(m)) => m.update(1.0 / 60.0, inputs),
            Some(Show::Credits {
                blocks,
                scroll,
                ticks,
            }) => {
                if self.started
                    && inputs
                        .iter()
                        .any(|i| i.start_pressed || i.attack_pressed || i.b_pressed)
                {
                    self.show = None;
                    return Event::Title;
                }
                *ticks += 1;
                // scroll_credits: 1 a frame, 2 once the first block has passed.
                let (alive, past) = credits_state(blocks, *scroll);
                if *ticks % 2 == 0 {
                    *scroll += if past { 2 } else { 1 };
                }
                !alive || *ticks >= 5400
            }
            Some(Show::Screen2D { ticks, .. }) => {
                // attract_check_input: Start or a face button goes to the title.
                if self.started
                    && inputs.iter().any(|i| {
                        i.start_pressed || i.attack_pressed || i.b_pressed || i.magic_pressed
                    })
                {
                    self.show = None;
                    return Event::Title;
                }
                *ticks -= 1;
                *ticks <= 0
            }
        };
        if !ended {
            return Event::None;
        }
        let was_movie = matches!(self.show, Some(Show::Movie(_) | Show::Credits { .. }));
        self.show = None;
        match self.advance(fs) {
            Event::None if was_movie => Event::MovieEnded,
            e => e,
        }
    }

    pub fn draw(&self, s: &mut Scr, movie_tex: Option<crate::ui::Tex>) {
        s.bars(1.0);
        match &self.show {
            Some(Show::Movie(_)) => {
                if let Some(t) = movie_tex {
                    s.blit(t, 0.0, 0.0, 512.0, 384.0, 0);
                }
            }
            Some(Show::Screen2D { prefix, .. }) => {
                s.quad_screen(prefix, 0);
                s.glow_text_lines(1.0, -432.0, -320.0, "Press\nStart");
            }
            Some(Show::Credits { blocks, scroll, .. }) => {
                s.quad_screen("CRED_ALL", 0);
                for (x, y, scale, line) in credit_layout(blocks, *scroll) {
                    // 16-pixel fade bands at the top and bottom of the screen.
                    let a = if 384.0 - y < 16.0 {
                        255 - (384 - y as i32) * 16
                    } else if y < 16.0 {
                        (16 - y as i32) * 16
                    } else {
                        0
                    };
                    let font = crate::ui::FONT_CREDITS;
                    s.text(scale, x, 383.0 - y, font, 0xFFFFFF, a.clamp(0, 255), line);
                }
            }
            None => s.rect(0.0, 0.0, 512.0, 384.0, [0.0, 0.0, 0.0, 1.0]),
        }
    }
}

impl Default for Attract {
    fn default() -> Self {
        Attract::new()
    }
}

/// The licence screen (main.c): SONY_SCREEN00-03, shown while the game loads.
pub fn draw_legal(s: &mut Scr) {
    s.bars(1.0);
    s.quad_screen("SONY_SCREEN", 0);
}

/// The credits text: entries 337.., 429.. and 469.. of the string table at
/// 0x80118188 in main.dol (92, 40 and 10 lines).
fn credit_lines(fs: &GameFs) -> Option<[Vec<String>; 3]> {
    let dol = gdl_formats::dol::Dol::new(fs.read_dol().ok()?);
    let block = |first: u32, n: u32| -> Vec<String> {
        (first..first + n)
            .map(|i| dol.str_ptr(0x8011_8188, i).unwrap_or_default())
            .collect()
    };
    Some([block(337, 92), block(429, 40), block(469, 10)])
}

/// Each visible credit line: x, its scroll height (0 at the bottom), scale and
/// text. Block 1 lines are 21 apart at scale 1; the others 15 apart at 0.833.
fn credit_layout(blocks: &[Vec<String>; 3], scroll: i32) -> Vec<(f32, f32, f32, &str)> {
    let mut out = Vec::new();
    let mut y = scroll;
    for (b, lines) in blocks.iter().enumerate() {
        let (step, scale) = if b == 0 { (21, 1.0) } else { (15, 0.833) };
        for l in lines {
            if y < 0 {
                return out;
            }
            if y < 384 {
                out.push((32.0, y as f32, scale, l.as_str()));
            }
            y -= step;
        }
    }
    out
}

/// Whether the roll is still running (a line is on screen, or the first has
/// yet to rise past the top), and whether the first block's last line is past
/// the middle (the roll then doubles its speed).
fn credits_state(blocks: &[Vec<String>; 3], scroll: i32) -> (bool, bool) {
    let layout = credit_layout(blocks, scroll);
    let last_first = scroll - 21 * (blocks[0].len() as i32 - 1);
    (
        !layout.is_empty() || scroll < 384,
        last_first > 192 || layout.is_empty(),
    )
}
