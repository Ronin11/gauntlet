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
    /// TODO: level flybys (MA_FLYBY) are not implemented; the entry is skipped.
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
}

pub enum Event {
    None,
    /// Go to the title screen.
    Title,
    /// A movie started: play its soundtrack.
    MovieStarted,
    /// A movie ended or was skipped: stop its soundtrack.
    MovieEnded,
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
        }
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
                Entry::Title | Entry::Flyby | Entry::Credits => continue,
            }
        }
    }

    /// One 60 Hz tick.
    pub fn update(&mut self, fs: &GameFs, inputs: &[PlayerInput]) -> Event {
        let ended = match &mut self.show {
            None => true,
            Some(Show::Movie(m)) => m.update(1.0 / 60.0, inputs),
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
        let was_movie = matches!(self.show, Some(Show::Movie(_)));
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
