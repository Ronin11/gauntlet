//! The player select screen, as select.c draws and runs it: four 128-pixel
//! columns, one per player, each with its own New / name entry / class-and-colour
//! pick / character menu flow.
//!
//! There is no memory card, so (as the original does with no card inserted) Load
//! and Save are disabled.

use crate::input::{PlayerInput, Source};
use crate::optmenu::{Event, Item, OptMenu, Repeat};
use crate::screen::Scr;
use crate::ui::{FONT_32, FONT_8X8, FONT_INITIALS};

/// Class abbreviations by class id (0x801200B0). 16 is Sumner.
pub const CLASSES: [&str; 17] = [
    "WAR", "VAL", "WIZ", "ARC", "DWF", "KNI", "SOR", "JES", "MIN", "FAL", "JAC", "TIG", "OGR",
    "UNI", "MED", "HYE", "SUM",
];

/// Display names by class id, for the HUD.
pub const HEROES: &[(&str, &str)] = &[
    ("WAR", "WARRIOR"),
    ("VAL", "VALKYRIE"),
    ("WIZ", "WIZARD"),
    ("ARC", "ARCHER"),
    ("DWF", "DWARF"),
    ("KNI", "KNIGHT"),
    ("SOR", "SORCERESS"),
    ("JES", "JESTER"),
    ("MIN", "MINOTAUR"),
    ("FAL", "FALCONESS"),
    ("JAC", "JACKAL"),
    ("TIG", "TIGRESS"),
    ("OGR", "OGRE"),
    ("UNI", "UNICORN"),
    ("MED", "MEDUSA"),
    ("HYE", "HYENA"),
];

/// Costume colours by class_id (0x80120104).
pub const COLOURS: [&str; 4] = ["YEL", "BLU", "RED", "GRE"];

/// Level-1 Strength, Speed, Armor, Magic per class (PDATA PDAT fight/speed/armor/
/// magic_min).
const BASE_STATS: [[i32; 4]; 16] = [
    [600, 350, 300, 100],
    [300, 300, 450, 300],
    [250, 350, 150, 600],
    [250, 600, 200, 300],
    [600, 300, 350, 100],
    [350, 300, 450, 250],
    [200, 400, 150, 600],
    [200, 600, 200, 350],
    [650, 400, 350, 150],
    [350, 350, 500, 350],
    [300, 400, 200, 650],
    [300, 650, 250, 350],
    [650, 350, 400, 150],
    [400, 350, 500, 300],
    [250, 450, 200, 650],
    [250, 650, 250, 400],
];
const STAT_NAMES: [&str; 4] = ["Strength", "Speed", "Armor", "Magic"];

/// Name-entry colours per player (DoGetName).
const NAME_RGB: [u32; 4] = [0xFFFF00, 0x87CEEB, 0xFF0000, 0x00FF00];
/// Names given to players who enter none.
const RANDOM_NAMES: [&str; 20] = [
    "LARRY", "PELE", "CHUCK", "TRENT", "SPENCR", "JOFFRY", "PABLO", "JUSTIN", "MAT", "CHIP ",
    "FRED", "SHAWN", "JAKE", "CJ", "ALEX", "MARVIN", "WESLEY", "BRAND", "GORDON", "TRAVIS",
];
/// The default character clear_player leaves (Sorceress).
const DEFAULT_CLASS: usize = 6;

const NEW: i32 = 1000;
const LOAD: i32 = 1001;
const SAVE: i32 = 1002;
const CHANGE: i32 = 1003;
const QUIT: i32 = 1004;
const DONE: i32 = 1005;
const YES: i32 = 1006;
const NO: i32 = 1007;

/// The 11 blit slots of a column (placement table 0x801216A8), back to front.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SlotId {
    Panel = 0,
    PanelLo = 1,
    Weapon = 2,
    Portrait = 3,
    FlyOut = 4,
    Question = 7,
    Name = 8,
    Border = 9,
    BorderLo = 10,
}
const SLOT_POS: [(f32, f32); 11] = [
    (0.0, 0.0),
    (0.0, 256.0),
    (0.0, 0.0),
    (0.0, 28.0),
    (0.0, 28.0),
    (0.0, 160.0),
    (0.0, 160.0),
    (64.0, 160.0),
    (8.0, 272.0),
    (0.0, 0.0),
    (0.0, 256.0),
];
/// Draw order: deepest first (placement depths 9, 8, 7, 6, 5, 4, 3).
const DRAW_ORDER: [usize; 11] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum Anim {
    #[default]
    Idle,
    FlyOut,
    Pulse,
    FadeIn,
    DelayedFadeOut,
}

#[derive(Clone, Default)]
struct Slot {
    tex: Option<String>,
    hidden: bool,
    anim: Anim,
    timer: i32,
    fade: i32,
    /// Projected size (the weapon art is stretched to 128x320).
    size: Option<(f32, f32)>,
    shimmer: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Motion {
    Top,
    Character,
    QuitConfirm,
    Name,
    Class,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Selecting(Motion),
    Ready,
}

struct Player {
    source: Source,
    state: State,
    motion_save: Motion,
    menu: OptMenu,
    rep: Repeat,
    name: String,
    /// The character being typed (cycles A..Z _ 0..9 @).
    cursor: char,
    /// Ticks the finished name blinks for.
    name_timer: i32,
    /// Highlighted class (respawn_char) and the class locked in (character).
    class: usize,
    character: usize,
    colour: usize,
    /// Choosing a class for an existing character (from the character menu).
    in_tower: bool,
    slots: [Slot; 11],
    /// Previous frame's state, to notice entering the class pick.
    entered_class: bool,
}

/// Out-of-game player data the select screen reads (the save file).
#[derive(Clone, Copy, Default)]
pub struct Progress {
    /// Unlocked secret classes: bit (class - 8); 0x100 unlocks Sumner.
    pub class_unlock: u16,
}

pub struct Select {
    players: [Option<Player>; 4],
    progress: Progress,
    /// Consecutive frames with every joined player ready and idle.
    idle_frames: u32,
    ticks: u32,
    /// Sounds and announcer lines to play, drained by the caller.
    pub sfx: Vec<String>,
    /// Set when the last player backs out: return to the title.
    pub leave: bool,
}

impl Select {
    pub fn new(progress: Progress) -> Select {
        Select {
            players: Default::default(),
            progress,
            idle_frames: 0,
            ticks: 0,
            sfx: Vec::new(),
            leave: false,
        }
    }

    /// Entered from the title: the pad that chose Start has already joined.
    pub fn with_first(first: Source) -> Select {
        let mut s = Select::new(Progress::default());
        s.join(first);
        s
    }

    /// Back from a level (init_player_select(2)): each player who was in the game
    /// comes straight to their character menu, Done preselected.
    pub fn from_game(panels: &[crate::hud::PanelInfo], sources: &[Source]) -> Select {
        let mut s = Select::new(Progress::default());
        for (info, src) in panels.iter().zip(sources) {
            if s.players[info.slot].is_some() {
                continue;
            }
            // join() takes the first free column; fill the panel's own column.
            let taken: Vec<usize> = (0..info.slot).filter(|&c| s.players[c].is_none()).collect();
            for &c in &taken {
                s.players[c] = Some(placeholder(*src));
            }
            let col = s.join(*src).unwrap();
            for &c in &taken {
                s.players[c] = None;
            }
            let p = s.players[col].as_mut().unwrap();
            p.name = info.name.clone();
            p.colour = info.colour;
            p.character = info.class;
            p.class = info.class;
            p.in_tower = true;
            p.open_character_menu(DONE);
        }
        // Players joining here pass through the class pick as usual.
        s
    }

    /// A sample screen for snapshots: one player per state.
    pub fn demo() -> Select {
        let mut s = Select::new(Progress::default());
        for (i, src) in [Source::Device(0), Source::Device(1), Source::Device(2)]
            .into_iter()
            .enumerate()
        {
            let col = s.join(src).unwrap();
            let p = s.players[col].as_mut().unwrap();
            match i {
                0 => {}
                1 => {
                    p.state = State::Selecting(Motion::Name);
                    p.name = "BOB".into();
                }
                _ => {
                    p.name = "SAM".into();
                    p.class = 0;
                    p.colour = 2;
                    p.state = State::Selecting(Motion::Class);
                    p.entered_class = true;
                    p.update_class_spec(true);
                }
            }
        }
        for _ in 0..10 {
            s.serve();
        }
        s
    }

    /// assign_controller + new_player: the first free column.
    fn join(&mut self, src: Source) -> Option<usize> {
        let col = self.players.iter().position(|p| p.is_none())?;
        let mut menu = OptMenu::new(
            vec![Item::new("New", NEW), Item::new("Load", LOAD)],
            -(64.0 + 128.0 * col as f32),
            0.0,
            0.667,
        );
        // No memory card: Load is disabled.
        menu.items[1].enabled = false;
        let mut slots: [Slot; 11] = Default::default();
        slots[SlotId::Panel as usize].tex = Some(format!("S1_plyr{}", col + 1));
        slots[SlotId::PanelLo as usize].tex = Some(format!("S2_plyr{}", col + 1));
        slots[SlotId::Border as usize].tex = Some("S1_border".into());
        slots[SlotId::Border as usize].shimmer = true;
        slots[SlotId::BorderLo as usize].tex = Some("S2_border".into());
        slots[SlotId::BorderLo as usize].shimmer = true;
        for s in &mut slots[2..9] {
            s.hidden = true;
        }
        let mut p = Player {
            source: src,
            state: State::Selecting(Motion::Top),
            motion_save: Motion::Top,
            menu,
            rep: Repeat::default(),
            name: String::new(),
            cursor: '@',
            name_timer: 0,
            class: DEFAULT_CLASS,
            character: DEFAULT_CLASS,
            colour: col,
            in_tower: false,
            slots,
            entered_class: false,
        };
        p.center_menu();
        self.players[col] = Some(p);
        Some(col)
    }

    /// Input sources in column order, for the game that follows.
    pub fn sources(&self) -> Vec<Source> {
        self.players.iter().flatten().map(|p| p.source).collect()
    }

    /// Each joined player's HUD panel: column, name, colour and class.
    pub fn panels(&self) -> Vec<crate::hud::PanelInfo> {
        self.players
            .iter()
            .enumerate()
            .filter_map(|(slot, p)| {
                let p = p.as_ref()?;
                Some(crate::hud::PanelInfo {
                    slot,
                    name: p.name.clone(),
                    colour: p.colour,
                    class: p.character,
                })
            })
            .collect()
    }

    /// Player names in column order.
    pub fn names(&self) -> Vec<String> {
        self.players
            .iter()
            .flatten()
            .map(|p| p.name.clone())
            .collect()
    }

    fn known(&self, class: usize) -> bool {
        class < 8 || (class < 16 && self.progress.class_unlock & (1 << (class - 8)) != 0)
    }

    /// One 60 Hz tick. Returns the chosen (class, colour) list once everyone has
    /// locked in.
    pub fn update(&mut self, inputs: &[(Source, PlayerInput)]) -> Option<Vec<(String, String)>> {
        self.ticks += 1;
        // Join: START on a pad that isn't playing.
        for (src, inp) in inputs {
            if inp.start_pressed && !self.players.iter().flatten().any(|p| p.source == *src) {
                self.join(*src);
            }
        }
        for col in 0..4 {
            let Some(src) = self.players[col].as_ref().map(|p| p.source) else {
                continue;
            };
            let inp = inputs
                .iter()
                .find(|(s, _)| *s == src)
                .map(|(_, i)| *i)
                .unwrap_or_default();
            self.update_player(col, &inp);
        }
        let animating = self.serve();
        let joined = self.players.iter().flatten().count();
        if joined == 0 {
            self.leave = true;
            return None;
        }
        let all_ready = self
            .players
            .iter()
            .flatten()
            .all(|p| p.state == State::Ready);
        // "allIdle" must hold for more than 8 frames (16 ticks).
        if all_ready && !animating {
            self.idle_frames += 1;
            if self.idle_frames > 16 {
                return Some(
                    self.players
                        .iter()
                        .flatten()
                        .map(|p| {
                            (
                                CLASSES[p.character].to_string(),
                                COLOURS[p.colour].to_string(),
                            )
                        })
                        .collect(),
                );
            }
        } else {
            self.idle_frames = 0;
        }
        None
    }

    fn update_player(&mut self, col: usize, inp: &PlayerInput) {
        let others_choosing = self
            .players
            .iter()
            .enumerate()
            .any(|(i, p)| i != col && matches!(p, Some(p) if p.state != State::Ready));
        let progress = self.progress;
        let mut remove = false;
        let p = self.players[col].as_mut().unwrap();
        match p.state {
            State::Ready => {
                p.set_name_banner();
                let idle = p.slots.iter().all(|s| s.anim == Anim::Idle);
                if others_choosing && idle && inp.start_pressed {
                    p.open_character_menu(DONE);
                }
            }
            State::Selecting(Motion::Top) => {
                p.motion_save = Motion::Top;
                match p.menu.update(inp, 1, true) {
                    Event::Chose(NEW) => {
                        self.sfx.push("S_OPTMENUSEL".into());
                        p.state = State::Selecting(Motion::Name);
                        p.name.clear();
                        p.cursor = '@';
                        p.name_timer = 0;
                    }
                    Event::Moved => self.sfx.push("S_OPTMENUMOVVRT".into()),
                    Event::Refused => self.sfx.push("S_NO".into()),
                    Event::Back => {
                        self.sfx.push("S_OPTMENUEXIT".into());
                        remove = true;
                    }
                    _ => {}
                }
            }
            State::Selecting(Motion::Name) => {
                p.hide_select_blits();
                if p.update_name(inp, &mut self.sfx, self.ticks) {
                    p.class = DEFAULT_CLASS;
                    p.state = State::Selecting(Motion::Class);
                    p.entered_class = false;
                } else if p.name_timer == 0 && inp.magic_pressed && p.name.is_empty() {
                    // B with nothing typed goes back to New / Load.
                    p.state = State::Selecting(p.motion_save);
                    p.center_menu();
                }
            }
            State::Selecting(Motion::Class) => {
                let (dx, dy) = p.menu_dirs(inp);
                let mut changed = !p.entered_class;
                p.entered_class = true;
                let old = (p.class, p.colour);
                if dx != 0 {
                    p.class = step(p.class, dx, progress);
                    changed = true;
                }
                if dy != 0 {
                    p.colour = (p.colour as i32 + dy).rem_euclid(4) as usize;
                    changed = true;
                }
                if changed {
                    self.sfx.push("S_OPTMENUCHAR".into());
                    if old != (p.class, p.colour) {
                        p.fly_out(old.0, known_of(old.0, progress));
                    }
                }
                let known_class = known_of(p.class, progress);
                p.update_class_spec(known_class);
                if inp.magic_pressed {
                    self.sfx.push("S_OPTMENUEXIT".into());
                    p.slots[SlotId::Portrait as usize].hidden = true;
                    p.state = State::Selecting(p.motion_save);
                    if p.motion_save == Motion::Top {
                        p.reset_top();
                    }
                } else if inp.attack_pressed {
                    if !known_class {
                        self.sfx.push("S_NO".into());
                    } else {
                        self.sfx.push("S_OPTMENUSEL".into());
                        p.character = p.class;
                        let welcome = format!(
                            "VOICE:S_WELCOME+S_{}{}1S",
                            COLOURS[p.colour], CLASSES[p.character]
                        );
                        self.sfx.push(welcome);
                        p.lock_in();
                        if p.in_tower {
                            p.open_character_menu(DONE);
                        } else {
                            p.state = State::Ready;
                        }
                        p.in_tower = true;
                    }
                }
            }
            State::Selecting(Motion::Character) => {
                p.motion_save = Motion::Character;
                p.set_name_banner();
                match p.menu.update(inp, 1, true) {
                    Event::Chose(DONE) => {
                        self.sfx.push("S_OPTMENUSEL".into());
                        p.state = State::Ready;
                    }
                    Event::Chose(CHANGE) => {
                        self.sfx.push("S_OPTMENUSEL".into());
                        p.class = p.character;
                        p.entered_class = false;
                        p.state = State::Selecting(Motion::Class);
                    }
                    Event::Chose(QUIT) => {
                        self.sfx.push("S_OPTMENUSEL".into());
                        p.state = State::Selecting(Motion::QuitConfirm);
                        p.yes_no();
                    }
                    Event::Moved => self.sfx.push("S_OPTMENUMOVVRT".into()),
                    Event::Refused => self.sfx.push("S_NO".into()),
                    _ => {}
                }
            }
            State::Selecting(Motion::QuitConfirm) => match p.menu.update(inp, 1, true) {
                Event::Chose(YES) => {
                    self.sfx.push("S_OPTMENUSEL".into());
                    remove = true;
                }
                Event::Chose(NO) | Event::Back => {
                    self.sfx.push("S_OPTMENUEXIT".into());
                    p.open_character_menu(QUIT);
                }
                Event::Moved => self.sfx.push("S_OPTMENUMOVVRT".into()),
                _ => {}
            },
        }
        if remove {
            self.players[col] = None;
        }
    }

    /// serve_blits for every column; true while any slot is animating (the
    /// questmark pulse doesn't count).
    fn serve(&mut self) -> bool {
        let frame = self.ticks.is_multiple_of(2);
        let mut animating = false;
        for p in self.players.iter_mut().flatten() {
            for s in p.slots.iter_mut() {
                animating |= s.serve(frame);
            }
        }
        animating
    }

    pub fn draw(&self, s: &mut Scr) {
        s.bars(1.0);
        for col in 0..4 {
            let x = 128.0 * col as f32;
            match &self.players[col] {
                Some(p) => p.draw(s, col, x, self),
                None => {
                    // An empty column: nebula panel and border only.
                    for (name, y, shimmer) in [
                        (format!("S1_plyr{}", col + 1), 0.0, false),
                        (format!("S2_plyr{}", col + 1), 256.0, false),
                        ("S1_border".to_string(), 0.0, true),
                        ("S2_border".to_string(), 256.0, true),
                    ] {
                        if let Some(t) = s.ui.get(&name) {
                            if shimmer {
                                s.blit_shimmer(t, x, y, t.w as f32, t.h as f32, 0);
                            } else {
                                s.blit(t, x, y, t.w as f32, t.h as f32, 0);
                            }
                        }
                    }
                }
            }
        }
    }
}

/// A column-holder used while placing returning players in their own columns.
fn placeholder(src: Source) -> Player {
    Player {
        source: src,
        state: State::Ready,
        motion_save: Motion::Top,
        menu: OptMenu::new(Vec::new(), 0.0, 0.0, 1.0),
        rep: Repeat::default(),
        name: String::new(),
        cursor: '@',
        name_timer: 0,
        class: 0,
        character: 0,
        colour: 0,
        in_tower: false,
        slots: Default::default(),
        entered_class: false,
    }
}

fn known_of(class: usize, progress: Progress) -> bool {
    class < 8 || (class < 16 && progress.class_unlock & (1 << (class - 8)) != 0)
}

fn step(class: usize, dir: i32, progress: Progress) -> usize {
    let mut c = class as i32;
    loop {
        c = (c + dir).rem_euclid(17);
        if c != 16 || progress.class_unlock & 0x100 != 0 {
            return c as usize;
        }
    }
}

/// The portrait texture for a class and colour: S12_<CLS>_<COL>, the shadow for
/// a locked class, or S12_SUM.
fn portrait(class: usize, colour: usize, known: bool) -> String {
    if class == 16 {
        "S12_SUM".into()
    } else if known {
        format!("S12_{}_{}", CLASSES[class], COLOURS[colour])
    } else {
        format!("S12_{}_shadw", CLASSES[class])
    }
}

impl Slot {
    /// One tick of serve_blits. `frame` marks the ticks that start a 30 Hz game
    /// frame (the pulse advances once per frame).
    fn serve(&mut self, frame: bool) -> bool {
        match self.anim {
            Anim::Idle => false,
            Anim::FlyOut => {
                self.timer += 1;
                let u = self.timer * self.timer;
                self.fade = u;
                if u >= 256 {
                    self.hidden = true;
                    self.anim = Anim::Idle;
                }
                true
            }
            Anim::Pulse => {
                if frame {
                    self.timer = (self.timer + 1) & 63;
                }
                false
            }
            Anim::FadeIn => {
                self.timer += 1;
                let a = ((self.timer >> 1) * (self.timer >> 1)).min(256);
                self.fade = 256 - a;
                if self.timer >= 32 {
                    self.fade = 0;
                    self.anim = Anim::Idle;
                }
                true
            }
            Anim::DelayedFadeOut => {
                self.timer += 1;
                let half = ((self.timer >> 1) - 16).max(0);
                self.fade = half * half;
                if self.fade >= 256 {
                    self.hidden = true;
                    self.anim = Anim::Idle;
                }
                true
            }
        }
    }
}

impl Player {
    fn slot(&mut self, id: SlotId) -> &mut Slot {
        &mut self.slots[id as usize]
    }

    /// setup_tex: show `name` in a slot at its own size, opaque.
    fn set(&mut self, id: SlotId, name: String) {
        let s = self.slot(id);
        s.tex = Some(name);
        s.hidden = false;
        s.fade = 0;
        s.size = None;
    }

    /// hide_select_blits(p, 0): panels and borders only.
    fn hide_select_blits(&mut self) {
        for (i, s) in self.slots.iter_mut().enumerate() {
            s.hidden = (2..9).contains(&i);
            s.anim = Anim::Idle;
        }
    }

    /// The OPTMENU's items centred on y = 128.
    fn center_menu(&mut self) {
        let h = self.menu.line_height() * self.menu.items.len() as f32;
        self.menu.y = 128.0 - (h / 2.0).floor();
    }

    fn reset_top(&mut self) {
        self.menu.items = vec![Item::new("New", NEW), Item::new("Load", LOAD)];
        self.menu.items[1].enabled = false;
        self.menu.sel = 0;
        self.center_menu();
        self.hide_select_blits();
    }

    /// motion 1: Save / Change / Load / Quit / Done, with `sel` preselected.
    fn open_character_menu(&mut self, sel: i32) {
        self.menu.items = vec![
            Item::new("Save", SAVE),
            Item::new("Change", CHANGE),
            Item::new("Load", LOAD),
            Item::new("Quit", QUIT),
            Item::new("Done", DONE),
        ];
        // No memory card: Save and Load are disabled.
        self.menu.items[0].enabled = false;
        self.menu.items[2].enabled = false;
        self.menu.sel = self
            .menu
            .items
            .iter()
            .position(|i| i.code == sel)
            .unwrap_or(4);
        self.center_menu();
        self.state = State::Selecting(Motion::Character);
        self.motion_save = Motion::Character;
        let weapon = format!("S12_WEAP_{}", CLASSES[self.character & 7]);
        self.set(SlotId::Weapon, weapon);
        self.slot(SlotId::Weapon).size = Some((128.0, 320.0));
        for id in [SlotId::Portrait, SlotId::FlyOut, SlotId::Question] {
            self.slot(id).hidden = true;
        }
        self.set_name_banner();
    }

    /// Quit confirm: Yes / No with No preselected, top at y = 164.
    fn yes_no(&mut self) {
        self.menu.items = vec![Item::new("Yes", YES), Item::new("No", NO)];
        self.menu.sel = 1;
        self.menu.y = 164.0;
    }

    fn set_name_banner(&mut self) {
        let name = if self.character == 16 {
            "SUM_NAME".to_string()
        } else {
            format!("{}_NAME", CLASSES[self.character])
        };
        self.set(SlotId::Name, name);
    }

    fn menu_dirs(&mut self, inp: &PlayerInput) -> (i32, i32) {
        match self.menu.update(inp, 1, false) {
            Event::Left => (-1, 0),
            Event::Right => (1, 0),
            Event::Up => (0, 1),
            Event::Down => (0, -1),
            _ => (0, 0),
        }
    }

    /// DoGetName. Returns true once the name is finished and has blinked.
    fn update_name(&mut self, inp: &PlayerInput, sfx: &mut Vec<String>, ticks: u32) -> bool {
        if self.name_timer > 0 {
            self.name_timer -= 1;
            return self.name_timer == 0;
        }
        const CYCLE: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ_0123456789@";
        let (dx, dy) = self.rep.edges(inp);
        let pos = CYCLE.find(self.cursor).unwrap_or(0) as i32;
        let n = CYCLE.len() as i32;
        if dy != 0 {
            let next = (pos + dy).rem_euclid(n) as usize;
            self.cursor = CYCLE.as_bytes()[next] as char;
            sfx.push("S_OPTMENUMOVVRT".into());
        }
        let delete = dx < 0 || (inp.magic_pressed && !self.name.is_empty());
        let accept = inp.attack_pressed || dx > 0;
        let mut done = false;
        if delete && !self.name.is_empty() {
            self.name.pop();
            sfx.push("S_OPTMENUMOVHRZ".into());
        } else if accept {
            if self.cursor == '@' {
                done = inp.attack_pressed;
            } else {
                self.name.push(self.cursor);
                self.cursor = '@';
                sfx.push("S_CHOOSESELSFX".into());
                done = self.name.len() >= 6;
            }
        }
        if done {
            if self.name.trim().is_empty() {
                self.name = RANDOM_NAMES[ticks as usize % RANDOM_NAMES.len()].into();
            }
            self.name_timer = 60;
        }
        false
    }

    /// The old portrait flies off when the class or colour changes.
    fn fly_out(&mut self, old_class: usize, old_known: bool) {
        let tex = portrait(old_class, self.colour, old_known);
        let s = self.slot(SlotId::FlyOut);
        s.tex = Some(tex);
        s.hidden = false;
        s.anim = Anim::FlyOut;
        s.timer = 0;
        s.fade = 0;
    }

    /// update_class_spec for the class pick.
    fn update_class_spec(&mut self, known: bool) {
        self.slot(SlotId::Weapon).hidden = true;
        let tex = portrait(self.class, self.colour, known);
        self.set(SlotId::Portrait, tex);
        if known {
            let name = if self.class == 16 {
                "SUM_NAME".to_string()
            } else {
                format!("{}_NAME", CLASSES[self.class])
            };
            self.set(SlotId::Name, name);
            let q = self.slot(SlotId::Question);
            q.hidden = true;
            q.anim = Anim::Idle;
        } else {
            self.slot(SlotId::Name).hidden = true;
            let q = self.slot(SlotId::Question);
            q.tex = Some("selscrn_questmark".into());
            q.hidden = false;
            if q.anim != Anim::Pulse {
                q.anim = Anim::Pulse;
                q.timer = 0;
            }
        }
    }

    /// The lock-in animation: the weapon art fades in while the portrait holds,
    /// then fades away.
    fn lock_in(&mut self) {
        let weapon = format!("S12_WEAP_{}", CLASSES[self.character & 7]);
        self.set(SlotId::Weapon, weapon);
        let w = self.slot(SlotId::Weapon);
        w.size = Some((128.0, 320.0));
        w.fade = 255;
        w.anim = Anim::FadeIn;
        w.timer = 0;
        let p = self.slot(SlotId::Portrait);
        p.anim = Anim::DelayedFadeOut;
        p.timer = 0;
        let q = self.slot(SlotId::Question);
        q.hidden = true;
        q.anim = Anim::Idle;
    }

    fn draw(&self, s: &mut Scr, col: usize, x: f32, sel: &Select) {
        for &i in &DRAW_ORDER {
            let slot = &self.slots[i];
            if slot.hidden {
                continue;
            }
            let Some(t) = slot.tex.as_deref().and_then(|n| s.ui.get(n)) else {
                continue;
            };
            let (px, py) = SLOT_POS[i];
            let (w, h) = slot.size.unwrap_or((t.w as f32, t.h as f32));
            match slot.anim {
                Anim::FlyOut => {
                    let tt = slot.timer as f32;
                    let u = tt * tt;
                    s.blit(
                        t,
                        px + x + u / 8.0,
                        u + 3.0 * tt,
                        128.0,
                        256.0 - u,
                        slot.fade,
                    );
                }
                Anim::Pulse => {
                    let amp = if slot.timer < 32 {
                        slot.timer
                    } else {
                        32 - (slot.timer & 31)
                    };
                    let f = 1.0 + 0.025 * amp as f32;
                    let (w, h) = (w * f, h * f);
                    s.blit(t, px + x - w / 2.0, py - h / 2.0, w, h, slot.fade);
                }
                _ if slot.shimmer => s.blit_shimmer(t, px + x, py, w, h, slot.fade),
                _ => s.blit(t, px + x, py, w, h, slot.fade),
            }
        }
        let cx = 64.0 + x;
        let animating = self
            .slots
            .iter()
            .any(|s| matches!(s.anim, Anim::FlyOut | Anim::FadeIn | Anim::DelayedFadeOut));
        match self.state {
            State::Selecting(Motion::Top) | State::Selecting(Motion::Character) => {
                self.menu.draw(s, 0);
                if self.state == State::Selecting(Motion::Character) {
                    s.text(1.2, -cx, 292.0, FONT_8X8, 0xFFFFFF, 0, "Level 1");
                }
                footer(s, x, self.state == State::Selecting(Motion::Top));
            }
            State::Selecting(Motion::QuitConfirm) => {
                for (k, l) in ["Character", "Not Saved.", "Quit Anyway?"]
                    .iter()
                    .enumerate()
                {
                    s.text(1.2, -cx, 100.0 + 10.0 * k as f32, FONT_8X8, 0xFFFFFF, 0, l);
                }
                self.menu.draw(s, 0);
                footer(s, x, false);
            }
            State::Selecting(Motion::Name) => {
                for (k, l) in ["Enter", "Your", "Name"].iter().enumerate() {
                    s.text(0.8, -cx, 64.0 + 26.0 * k as f32, FONT_32, 0xFFFFFF, 0, l);
                }
                let hints = [
                    (180.0, &["BUTTON_U", "BUTTON_D"][..], "Change"),
                    (200.0, &["BUTTON_L", "BUTTON_R"][..], "Edit"),
                    (220.0, &["", "BUTTON_X"][..], "Accept"),
                    (240.0, &["", "BUTTON_TRI"][..], "Cancel"),
                ];
                for (y, icons, label) in hints {
                    for (k, icon) in icons.iter().enumerate() {
                        if let Some(t) = s.ui.get(icon) {
                            s.blit(t, x + 10.0 + 19.0 * k as f32, y, 19.0, 19.0, 0);
                        }
                    }
                    s.text(1.2, x + 56.0, y + 4.0, FONT_8X8, 0xFFFFFF, 0, label);
                }
                self.draw_name(s, col, sel.ticks);
            }
            State::Selecting(Motion::Class) => {
                if sel.known(self.class) {
                    self.draw_stats(s, x);
                }
                if let (Some(l), Some(r)) = (s.ui.get("BUTTON_L"), s.ui.get("BUTTON_R")) {
                    s.blit(l, x + 10.0, 232.0, 19.0, 19.0, 0);
                    s.blit(r, x + 29.0, 232.0, 19.0, 19.0, 0);
                }
                s.text(1.2, x + 56.0, 236.0, FONT_8X8, 0xFFFFFF, 0, "Change");
                if sel.known(self.class) {
                    footer(s, x, false);
                }
            }
            State::Ready => {
                let others = sel
                    .players
                    .iter()
                    .flatten()
                    .any(|p| p.state != State::Ready);
                if others && !animating {
                    for (k, l) in ["Press START", "to Save/Load", "or Change", "Character"]
                        .iter()
                        .enumerate()
                    {
                        s.text(1.2, -cx, 142.0 + 12.0 * k as f32, FONT_8X8, 0xFFFFFF, 0, l);
                    }
                }
            }
        }
    }

    /// The initials being typed, or the finished name blinking.
    fn draw_name(&self, s: &mut Scr, col: usize, ticks: u32) {
        const LEFT: [f32; 4] = [42.0, 172.0, 300.0, 425.0];
        if self.name_timer > 0 {
            if self.name_timer & 16 != 0 {
                let cx = 64.0 + 128.0 * col as f32;
                s.text(
                    0.75,
                    -cx,
                    340.0,
                    FONT_INITIALS,
                    NAME_RGB[col],
                    0,
                    &self.name,
                );
            }
            return;
        }
        let mut x = LEFT[col] - 30.0;
        for k in 0..6 {
            let (ch, rgb) = match self.name.chars().nth(k) {
                Some(c) => (c, NAME_RGB[col]),
                None if k == self.name.len() => (
                    self.cursor,
                    if ticks & 0x10 != 0 {
                        0xFFFFFF
                    } else {
                        0x404040
                    },
                ),
                None => ('_', 0xFFFFFF),
            };
            s.text(0.9, x - 4.0, 340.0, FONT_INITIALS, rgb, 0, &ch.to_string());
            x += 18.0;
        }
    }

    /// update_class_attr: the four stats as 3-digit numbers, the best one glowing.
    fn draw_stats(&self, s: &mut Scr, x: f32) {
        if self.class == 16 {
            return;
        }
        let stats = BASE_STATS[self.class];
        let mut best = 0;
        for r in 1..4 {
            if stats[r] > stats[best] {
                best = r;
            }
        }
        for r in 0..4 {
            let y = 162.0 + 16.0 * r as f32;
            let label = STAT_NAMES[r];
            let lw = s.text_width(FONT_32, 0.5, label);
            if r == best {
                if let Some(t) = s.ui.get("ATT_GLOW") {
                    s.blit(t, x + 78.0, y - 6.0, 68.0, t.h as f32, 0);
                }
                s.glow_text(0.5, x + 81.0 - lw, y - 2.0, label);
            } else {
                s.text(0.5, x + 81.0 - lw, y - 2.0, FONT_32, 0xFFFFFF, 0, label);
            }
            let v = format!("{:03}", stats[r].min(999));
            s.text(0.5, x + 84.0, y, FONT_INITIALS, 0xFFFFFF, 0, &v);
        }
        s.text(1.2, -(64.0 + x), 292.0, FONT_8X8, 0xFFFFFF, 0, "NEW");
    }
}

/// The Select (A) footer, and Back (B) under it.
fn footer(s: &mut Scr, x: f32, back: bool) {
    if let Some(t) = s.ui.get("BUTTON_X") {
        s.blit(t, x + 20.0, 252.0, 19.0, 19.0, 0);
    }
    s.text(1.2, x + 47.0, 256.0, FONT_8X8, 0xFFFFFF, 0, "Select");
    if back {
        if let Some(t) = s.ui.get("BUTTON_TRI") {
            s.blit(t, x + 20.0, 272.0, 19.0, 19.0, 0);
        }
        s.text(1.2, x + 47.0, 276.0, FONT_8X8, 0xFFFFFF, 0, "Back");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec2;

    fn tick(sel: &mut Select, inp: PlayerInput) -> Option<Vec<(String, String)>> {
        let r = sel.update(&[(Source::Keyboard, inp)]);
        // Release everything so the next press is a new edge.
        sel.update(&[(Source::Keyboard, PlayerInput::default())])
            .or(r)
    }

    #[test]
    fn new_name_class_and_ready() {
        let a = PlayerInput {
            attack_pressed: true,
            ..Default::default()
        };
        let right = PlayerInput {
            stick: Vec2::new(1.0, 0.0),
            ..Default::default()
        };
        let mut sel = Select::with_first(Source::Keyboard);
        // New
        assert!(tick(&mut sel, a).is_none());
        // Accept the end mark at once: a random name, which blinks for 60 ticks.
        tick(&mut sel, a);
        for _ in 0..40 {
            tick(&mut sel, PlayerInput::default());
        }
        let p = sel.players[0].as_ref().unwrap();
        assert_eq!(p.state, State::Selecting(Motion::Class));
        assert!(!p.name.is_empty());
        // Sorceress -> Jester, then lock in.
        tick(&mut sel, right);
        assert_eq!(sel.players[0].as_ref().unwrap().class, 7);
        tick(&mut sel, a);
        let mut out = None;
        for _ in 0..60 {
            out = out.or(tick(&mut sel, PlayerInput::default()));
        }
        assert_eq!(out, Some(vec![("JES".to_string(), "YEL".to_string())]));
        assert!(sel.sfx.iter().any(|s| s.contains("S_WELCOME")));
    }

    #[test]
    fn secret_classes_are_locked() {
        let sel = Select::new(Progress::default());
        assert!(sel.known(7));
        assert!(!sel.known(8));
        assert_eq!(step(15, 1, Progress::default()), 0);
    }
}
