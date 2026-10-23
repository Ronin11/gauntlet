//! The player select screen, as select.c draws and runs it: four 128-pixel
//! columns, one per player, each with its own New / name entry / class-and-colour
//! pick / character menu flow.
//!
//! Heroes are saved to and loaded from a virtual memory card in Slot A
//! (`crate::save::MemCard`): always present and formatted, so the select
//! screen's format and no-card branches (motions 6 and 11) never come up, but
//! the first save offers to create the Gauntlet save file (motion 12).

use crate::game::Carried;
use crate::input::{PlayerInput, Source};
use crate::options::PlayerPrefs;
use crate::optmenu::{Event, Item, OptMenu, Repeat};
use crate::save::{HeroSave, MemCard};
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
    /// 0: New / Load.
    Top,
    /// 1: Save / Change / Load / Quit / Done.
    Character,
    /// 2: "Character Not Saved. Quit Anyway?"
    QuitConfirm,
    /// 3: name entry.
    Name,
    /// 4: class and colour pick.
    Class,
    /// 5: step 0 "Current Character Not Saved. Load new Character Anyway?",
    /// then the card list.
    CardLoad,
    /// 7: "No Gauntlet save data on Memory Card."
    NoData,
    /// 8: "Choose a File to Load".
    PickLoad,
    /// 9: loading, then "Load Complete" / "Load Failed!".
    Loading,
    /// 10: the card list for saving.
    CardSave,
    /// 12: "No Gauntlet save data on Memory Card. Create %d Blocks Save
    /// File?", then creating it.
    Create,
    /// 13: "Choose a Location to Write to".
    PickSave,
    /// 14: "Overwrite Existing File?", then writing.
    Writing,
}

/// Item codes of the card list and file lists (1000 + index).
const ENTRY: i32 = 1000;
/// The memory card list's one entry (setup_vmu_entries, no multitap).
const CARD_SLOT: &str = " SLOT  1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Selecting(Motion),
    Ready,
}

struct Player {
    /// The column (player index).
    col: usize,
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
    /// The save image (Player.save) and the current class's live state
    /// (None: a new character).
    hero: HeroSave,
    carried: Option<Carried>,
    /// save.saved: -1 for a new hero (nothing to lose), 0 when changed since
    /// the last save or load, 1 when saved or loaded.
    saved: i8,
    /// sel_step: a memory card sub-state's step and tick count.
    step: i32,
    /// sel_file_cursor: the file this player last loaded or picked to save to
    /// (-1 none); other players can't use it.
    file_cursor: i32,
    /// sel_save_file.
    save_file: usize,
    /// hidden_code: a hidden character's row in `cheats::HIDDEN`.
    hidden: Option<usize>,
}

/// What the party brings to player select: the memory card, each column's
/// state carried from the last level and save image, and the party's
/// progress (copied into saves).
#[derive(Clone, Default)]
pub struct Party {
    pub card: MemCard,
    pub carry: [Option<Carried>; 4],
    pub heroes: [Option<HeroSave>; 4],
    pub progress: crate::progress::Progress,
    /// Each pad's settings (PlayerControl), saved with a hero and set from a
    /// loaded one.
    pub controls: [PlayerPrefs; 4],
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
    card: MemCard,
    /// The party's realm progress, for saves.
    party: crate::progress::Progress,
    /// PlayerControl per column.
    controls: [PlayerPrefs; 4],
    /// lbl_80344BB4: ticks before another save may start.
    debounce: i32,
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
            card: MemCard::in_memory(),
            party: Default::default(),
            controls: Default::default(),
            debounce: 0,
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
            p.hidden = info.hidden;
            // PlayerSaveState cleared save.saved during play.
            p.saved = 0;
            p.hero = HeroSave::new(&p.name, p.character, p.colour);
            p.open_character_menu(DONE);
        }
        // Players joining here pass through the class pick as usual.
        s
    }

    /// The memory card, and each column's carried state, save image and the
    /// party's progress.
    pub fn with_party(mut self, party: Party) -> Select {
        self.card = party.card;
        self.party = party.progress;
        self.controls = party.controls;
        for (col, p) in self.players.iter_mut().enumerate() {
            let Some(p) = p else {
                continue;
            };
            p.carried = party.carry[col].clone();
            if let Some(h) = &party.heroes[col] {
                p.hero = h.clone();
                p.hero.name = p.name.clone();
            }
        }
        self
    }

    /// The debug menu's session unlocks (debugmenu.rs, not in the original):
    /// secret classes offered as if a save had them.
    pub fn set_debug_unlock(&mut self, class_unlock: u16) {
        self.progress.class_unlock = class_unlock;
    }

    /// The state each column's player takes into the game (their class's
    /// slot of their save, or what they carried in); None for a new
    /// character.
    pub fn carried(&self, col: usize) -> Option<Carried> {
        self.players.get(col)?.as_ref()?.carried.clone()
    }

    /// Each column's pad settings, as a loaded hero may have changed them.
    pub fn controls(&self) -> [PlayerPrefs; 4] {
        self.controls
    }

    /// Each column's save image, kept for the next visit.
    pub fn heroes(&self) -> [Option<HeroSave>; 4] {
        std::array::from_fn(|c| self.players[c].as_ref().map(|p| p.hero.clone()))
    }

    /// Every player's save accumulates the party's achievements, so the
    /// party gets the union of what each player's class record holds (a
    /// loaded hero brings its own).
    pub fn merge_progress(&self, party: &mut crate::progress::Progress) {
        for p in self.players.iter().flatten() {
            if let Some(cs) = p.hero.classes.get(p.character) {
                cs.progress.merge_into(party);
            }
        }
    }

    /// The memory card, handed back for the next visit.
    pub fn card(&self) -> &MemCard {
        &self.card
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

    /// Sample memory card screens for snapshots (`--screen select:<name>`):
    /// `load` (overwrite prompt, no data, file list, loading), `save` (card
    /// list, create prompt, file list, overwrite prompt), `result` (load and
    /// save complete, file created, a ready player) and `char` (character
    /// menu, quit confirm, a loaded hero's class pick).
    pub fn demo_card(name: &str) -> Option<Select> {
        let mut s = Select::new(Progress::default());
        let mut vet = Carried::new_character();
        vet.exp = 20000;
        vet.bonus.armor = 40.0;
        let mut hero = HeroSave::new("BOB", 3, 2);
        hero.store(3, 2, &vet, &Default::default());
        let mut card = MemCard::in_memory();
        if name != "load" {
            card.create();
            card.write(0, &hero);
            card.write(3, &HeroSave::new("ANN", 1, 0));
        }
        s.card = card.clone();
        let names = card.names();
        for col in 0..4 {
            s.join(Source::Device(col as u64))?;
            let p = s.players[col].as_mut()?;
            p.name = ["BOB", "ANN", "SAM", "JO"][col].into();
            p.character = [3, 1, 4, 5][col];
            p.colour = col;
            p.motion_save = Motion::Character;
            match (name, col) {
                ("load", 0) => {
                    p.state = State::Selecting(Motion::CardLoad);
                    p.step = 0;
                    p.yes_no();
                }
                ("load", 1) => {
                    p.state = State::Selecting(Motion::NoData);
                    p.step = 30;
                }
                ("load", 2) => {
                    let mut c = card.clone();
                    c.create();
                    c.write(1, &hero);
                    p.goto_files(Motion::PickLoad, &c.names(), &[], true);
                }
                ("load", _) => {
                    p.state = State::Selecting(Motion::Loading);
                    p.step = 2;
                }
                ("save", 0) => p.goto_card(Motion::CardSave),
                ("save", 1) => {
                    p.state = State::Selecting(Motion::Create);
                    p.step = 0;
                    p.yes_no();
                }
                ("save", 2) => p.goto_files(Motion::PickSave, &names, &[3], false),
                ("save", _) => {
                    p.state = State::Selecting(Motion::Writing);
                    p.save_file = 0;
                    p.step = 0;
                    p.yes_no();
                }
                ("result", 0) => {
                    p.state = State::Selecting(Motion::Loading);
                    p.step = 50;
                }
                ("result", 1) => {
                    p.state = State::Selecting(Motion::Writing);
                    p.step = 50;
                }
                ("result", 2) => {
                    p.state = State::Selecting(Motion::Create);
                    p.step = 50;
                }
                ("result", _) => {
                    p.state = State::Ready;
                    p.set_name_banner();
                }
                ("char", 0) => {
                    p.carried = Some(vet.clone());
                    p.open_character_menu(DONE);
                }
                ("char", 1) => {
                    p.state = State::Selecting(Motion::QuitConfirm);
                    p.yes_no();
                }
                ("char", 2) => {
                    p.apply_hero(hero.clone());
                    p.state = State::Selecting(Motion::Class);
                    p.entered_class = true;
                    p.update_class_spec(true);
                }
                ("char", _) => {
                    p.state = State::Selecting(Motion::Writing);
                    p.save_file = 3;
                    p.step = 2;
                }
                _ => return None,
            }
            p.update_panel();
            if p.state == State::Ready {
                let weapon = format!("S12_WEAP_{}", CLASSES[p.character & 7]);
                p.set(SlotId::Weapon, weapon);
                p.slot(SlotId::Weapon).size = Some((128.0, 320.0));
            }
        }
        for _ in 0..10 {
            s.serve();
        }
        Some(s)
    }

    /// assign_controller + new_player: the first free column.
    pub fn join(&mut self, src: Source) -> Option<usize> {
        let col = self.players.iter().position(|p| p.is_none())?;
        let menu = OptMenu::new(
            vec![Item::new("New", NEW), Item::new("Load", LOAD)],
            -(64.0 + 128.0 * col as f32),
            0.0,
            0.667,
        );
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
            col,
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
            hero: HeroSave::new("", DEFAULT_CLASS, col),
            carried: None,
            saved: -1,
            step: 0,
            file_cursor: -1,
            save_file: 0,
            hidden: None,
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
                    hidden: p.hidden,
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

    #[cfg(test)]
    fn known(&self, class: usize) -> bool {
        known_of(class, self.progress.class_unlock)
    }

    /// One 60 Hz tick. Returns the chosen (class, colour) list once everyone has
    /// locked in.
    pub fn update(&mut self, inputs: &[(Source, PlayerInput)]) -> Option<Vec<(String, String)>> {
        self.ticks += 1;
        if self.debounce > 0 {
            self.debounce -= 1;
        }
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
        // verify_vmu_file_ok: files other joined players have claimed.
        let claimed: Vec<usize> = self
            .players
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != col)
            .filter_map(|(_, p)| usize::try_from(p.as_ref()?.file_cursor).ok())
            .collect();
        // Waiting on the background loader takes a game frame per step.
        let frame = self.ticks.is_multiple_of(2);
        let progress = self.progress;
        let ticks = self.ticks;
        let mut remove = false;
        let sfx = &mut self.sfx;
        let card = &mut self.card;
        let party = &self.party;
        let debounce = &mut self.debounce;
        let controls = &mut self.controls[col];
        let p = self.players[col].as_mut().unwrap();
        let unlock = progress.class_unlock | p.hero.class_unlock;
        // do_optmenu's sounds: A plays the select sound, B the exit sound.
        let ev = |sfx: &mut Vec<String>, e: Event| {
            match e {
                Event::Chose(_) => sfx.push("S_OPTMENUSEL".into()),
                Event::Back => sfx.push("S_OPTMENUEXIT".into()),
                Event::Moved => sfx.push("S_OPTMENUMOVVRT".into()),
                Event::Refused => sfx.push("S_NO".into()),
                _ => {}
            }
            e
        };
        p.update_panel();
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
                match ev(sfx, p.menu.update(inp, 1, true)) {
                    Event::Chose(NEW) => {
                        p.new_player();
                        p.state = State::Selecting(Motion::Name);
                        p.cursor = '@';
                        p.name_timer = 0;
                    }
                    Event::Chose(LOAD) => {
                        p.step = 1;
                        p.goto_card(Motion::CardLoad);
                    }
                    Event::Back => remove = true,
                    _ => {}
                }
            }
            State::Selecting(Motion::Name) => {
                p.hide_select_blits();
                if p.update_name(inp, sfx, ticks) {
                    p.hero.name = p.name.clone();
                    // set_hidden_player: a hidden character's name skips the
                    // class pick.
                    if let Some(h) = crate::cheats::hidden_for(&p.name) {
                        p.init_hidden(h, party, sfx);
                    } else {
                        p.class = DEFAULT_CLASS;
                        p.state = State::Selecting(Motion::Class);
                        p.entered_class = false;
                    }
                } else if p.name_timer == 0 && inp.b_pressed && p.name.is_empty() {
                    // B with nothing typed goes back to New / Load.
                    p.back_to_save();
                }
            }
            State::Selecting(Motion::Class) => {
                let (dx, dy) = p.menu_dirs(inp);
                let mut changed = !p.entered_class;
                p.entered_class = true;
                let old = (p.class, p.colour);
                if dx != 0 {
                    p.class = step(p.class, dx, unlock);
                    changed = true;
                }
                if dy != 0 {
                    p.colour = (p.colour as i32 + dy).rem_euclid(4) as usize;
                    changed = true;
                }
                if changed {
                    sfx.push("S_OPTMENUCHAR".into());
                    if old != (p.class, p.colour) {
                        p.fly_out(old.0, known_of(old.0, unlock));
                    }
                }
                let known_class = known_of(p.class, unlock);
                p.update_class_spec(known_class);
                if inp.b_pressed {
                    sfx.push("S_OPTMENUEXIT".into());
                    p.slots[SlotId::Portrait as usize].hidden = true;
                    if p.motion_save == Motion::Top {
                        // new_player: back to a blank hero.
                        p.new_player();
                    }
                    p.back_to_save();
                } else if inp.attack_pressed {
                    if !known_class {
                        sfx.push("S_NO".into());
                    } else {
                        sfx.push("S_OPTMENUSEL".into());
                        p.change_player(p.class, party);
                        // AudioWelcomeBack (S_WELCOME) for a new character,
                        // AudioWelcome (S_WELCOMEBACK) for one with experience.
                        // AudioWithName: the greeting, then the select bank's
                        // colour + class line (S_YELDWF1S = "Yellow" + "Dwarf").
                        // Sumner has no name line (wflag 0): the greeting alone.
                        let exp = p.carried.as_ref().map_or(0, |c| c.exp);
                        let greet = if exp == 0 {
                            "S_WELCOME"
                        } else {
                            "S_WELCOMEBACK"
                        };
                        sfx.push(if p.character == 16 {
                            format!("VOICE:{greet}")
                        } else {
                            format!(
                                "VOICE:{greet}+S_{}{}1S|VOICE:{greet}",
                                COLOURS[p.colour], CLASSES[p.character]
                            )
                        });
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
                match ev(sfx, p.menu.update(inp, 1, true)) {
                    Event::Chose(DONE) => p.state = State::Ready,
                    Event::Chose(SAVE) => p.goto_card(Motion::CardSave),
                    Event::Chose(LOAD) => {
                        // An unsaved hero is asked first.
                        p.step = if p.saved != 0 { 1 } else { 0 };
                        if p.step == 0 {
                            p.state = State::Selecting(Motion::CardLoad);
                            p.yes_no();
                        } else {
                            p.goto_card(Motion::CardLoad);
                        }
                    }
                    // set_hidden_player: a hidden character stays one.
                    Event::Chose(CHANGE) => {
                        match p.hidden.or_else(|| crate::cheats::hidden_for(&p.name)) {
                            Some(h) => p.init_hidden(h, party, sfx),
                            None => {
                                p.class = p.character;
                                p.entered_class = false;
                                p.state = State::Selecting(Motion::Class);
                            }
                        }
                    }
                    Event::Chose(QUIT) => {
                        if p.saved != 0 {
                            remove = true;
                        } else {
                            p.state = State::Selecting(Motion::QuitConfirm);
                            p.yes_no();
                        }
                    }
                    _ => {}
                }
            }
            State::Selecting(Motion::QuitConfirm) => match ev(sfx, p.menu.update(inp, 1, true)) {
                Event::Chose(YES) => remove = true,
                Event::Chose(NO) | Event::Back => p.open_character_menu(DONE),
                _ => {}
            },
            State::Selecting(Motion::CardLoad) if p.step == 0 => {
                match ev(sfx, p.menu.update(inp, 1, true)) {
                    Event::Chose(YES) => {
                        p.step = 1;
                        p.goto_card(Motion::CardLoad);
                    }
                    Event::Chose(NO) | Event::Back => p.back_to_save(),
                    _ => {}
                }
            }
            State::Selecting(Motion::CardLoad) => match ev(sfx, p.menu.update(inp, 1, true)) {
                Event::Back => p.back_to_save(),
                // setup_file_entries(pl, 1): no Gauntlet file is "no data".
                Event::Chose(ENTRY) if card.has_save_file() => {
                    p.goto_files(Motion::PickLoad, &card.names(), &claimed, true)
                }
                Event::Chose(ENTRY) => {
                    p.state = State::Selecting(Motion::NoData);
                    p.step = 1;
                }
                _ => {}
            },
            State::Selecting(Motion::NoData) => {
                p.step += 1;
                if p.step >= 120 {
                    p.step = 0;
                    p.back_to_save();
                }
            }
            State::Selecting(Motion::PickLoad) => {
                p.file_enables(&card.names(), &claimed, true);
                match ev(sfx, p.menu.update(inp, 1, true)) {
                    Event::Back => p.goto_card(Motion::CardLoad),
                    Event::Chose(c) if c >= ENTRY && !claimed.contains(&((c - ENTRY) as usize)) => {
                        p.state = State::Selecting(Motion::Loading);
                        p.save_file = (c - ENTRY) as usize;
                        p.file_cursor = c - ENTRY;
                        p.step = 0;
                    }
                    _ => {}
                }
            }
            State::Selecting(Motion::Loading) => match p.step {
                0..=2 => p.step += frame as i32,
                3 => match card.load(p.save_file) {
                    Some(h) => {
                        *controls = p.apply_hero(h);
                        p.step = 4;
                    }
                    None => p.step = -1,
                },
                t if t >= 0 => {
                    p.step += 1;
                    if p.step >= 120 {
                        p.step = -1;
                        if let Some(h) = crate::cheats::hidden_for(&p.name) {
                            // set_hidden_player: a hidden character's save.
                            p.init_hidden(h, party, sfx);
                        } else {
                            // The loaded hero picks a class (its own
                            // highlighted).
                            p.class = p.character;
                            p.entered_class = false;
                            p.state = State::Selecting(Motion::Class);
                            p.in_tower = true;
                        }
                    }
                }
                _ => {
                    p.step -= 1;
                    if p.step <= -120 {
                        p.step = -1;
                        p.back_to_save();
                    }
                }
            },
            State::Selecting(Motion::CardSave) => match ev(sfx, p.menu.update(inp, 1, true)) {
                Event::Back => p.back_to_save(),
                Event::Chose(ENTRY) if card.has_save_file() => {
                    p.goto_files(Motion::PickSave, &card.names(), &claimed, false)
                }
                Event::Chose(ENTRY) => {
                    p.state = State::Selecting(Motion::Create);
                    p.step = 0;
                    p.yes_no();
                }
                _ => {}
            },
            State::Selecting(Motion::Create) => match p.step {
                0 => {
                    let mut e = p.menu.update(inp, 1, true);
                    if card.has_save_file() {
                        e = Event::Back;
                    }
                    match ev(sfx, e) {
                        Event::Chose(YES) => p.step = 1,
                        Event::Chose(NO) | Event::Back => p.goto_card(Motion::CardSave),
                        _ => {}
                    }
                }
                1 | 2 => p.step += frame as i32,
                3 => {
                    p.step = if card.free_bytes() < crate::save::SAVE_FILE_SIZE {
                        1000
                    } else if card.create() {
                        4
                    } else {
                        -1
                    }
                }
                t if t >= 1000 => {
                    p.step += 1;
                    if p.step >= 0x460 {
                        p.step = 0;
                        p.back_to_save();
                    }
                }
                t if t >= 0 => {
                    p.step += 1;
                    if p.step >= 120 {
                        p.step = 0;
                        p.goto_files(Motion::PickSave, &card.names(), &claimed, false);
                    }
                }
                _ => {
                    p.step -= 1;
                    if p.step <= -120 {
                        p.step = 0;
                        p.back_to_save();
                    }
                }
            },
            State::Selecting(Motion::PickSave) => {
                p.file_enables(&card.names(), &claimed, false);
                match ev(sfx, p.menu.update(inp, 1, true)) {
                    Event::Back => p.goto_card(Motion::CardSave),
                    Event::Chose(c) if c >= ENTRY && *debounce == 0 => {
                        let k = (c - ENTRY) as usize;
                        p.state = State::Selecting(Motion::Writing);
                        p.save_file = k;
                        p.file_cursor = c - ENTRY;
                        p.step = 0;
                        *debounce = 30;
                        if !card.is_empty(k) {
                            p.yes_no();
                        }
                    }
                    _ => {}
                }
            }
            State::Selecting(Motion::Writing) => match p.step {
                0 if card.is_empty(p.save_file) => p.step = 1,
                0 => match ev(sfx, p.menu.update(inp, 1, true)) {
                    Event::Chose(YES) => p.step = 1,
                    Event::Chose(NO) | Event::Back => {
                        p.file_cursor = -1;
                        p.goto_files(Motion::PickSave, &card.names(), &claimed, false);
                    }
                    _ => {}
                },
                1 | 2 => p.step += frame as i32,
                3 => {
                    let ok = p.write(card, party, *controls);
                    // PlayerWriteSaveFile marks the hero saved either way.
                    p.saved = 1;
                    p.step = if ok { 4 } else { -1 };
                }
                t if t >= 0 => {
                    p.step += 1;
                    if p.step >= 120 {
                        p.step = 0;
                        p.open_character_menu(DONE);
                    }
                }
                _ => {
                    p.step -= 1;
                    if p.step <= -120 {
                        p.step = 0;
                        p.back_to_save();
                    }
                }
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
        col: 0,
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
        hero: HeroSave::new("", 0, 0),
        carried: None,
        saved: -1,
        step: 0,
        file_cursor: -1,
        save_file: 0,
        hidden: None,
    }
}

/// The class pick's `class < 8 || class_unlock & (1 << (class - 8))`; for
/// Sumner (16) that is bit 0x100.
fn known_of(class: usize, unlock: u16) -> bool {
    class < 8 || (class <= 16 && unlock & (1 << (class - 8)) != 0)
}

fn step(class: usize, dir: i32, unlock: u16) -> usize {
    let mut c = class as i32;
    loop {
        c = (c + dir).rem_euclid(17);
        if c != 16 || unlock & 0x100 != 0 {
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

    /// The menu template: items centred on the column at scale 0.667.
    fn column_menu(&mut self, items: Vec<Item>, sel: usize) {
        self.menu.items = items;
        self.menu.x = -(64.0 + 128.0 * self.col as f32);
        self.menu.scale = 0.667;
        self.menu.dy = 0.0;
        self.menu.sel = sel;
        self.center_menu();
    }

    fn reset_top(&mut self) {
        self.column_menu(vec![Item::new("New", NEW), Item::new("Load", LOAD)], 0);
        self.hide_select_blits();
    }

    /// motion 1: Save / Change / Load / Quit / Done, with `sel` preselected.
    fn open_character_menu(&mut self, sel: i32) {
        let items = vec![
            Item::new("Save", SAVE),
            Item::new("Change", CHANGE),
            Item::new("Load", LOAD),
            Item::new("Quit", QUIT),
            Item::new("Done", DONE),
        ];
        let sel = items.iter().position(|i| i.code == sel).unwrap_or(4);
        self.column_menu(items, sel);
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

    /// setup_sel_menu mode 15: Yes / No with No preselected, top at y = 164.
    fn yes_no(&mut self) {
        self.column_menu(vec![Item::new("Yes", YES), Item::new("No", NO)], 1);
        self.menu.y = 164.0;
    }

    /// Back to motion_state_save's menu.
    fn back_to_save(&mut self) {
        match self.motion_save {
            Motion::Character => self.open_character_menu(DONE),
            _ => {
                self.state = State::Selecting(Motion::Top);
                self.reset_top();
            }
        }
    }

    /// new_player: a blank hero in this column.
    fn new_player(&mut self) {
        self.name.clear();
        self.class = DEFAULT_CLASS;
        self.character = DEFAULT_CLASS;
        self.in_tower = false;
        self.hero = HeroSave::new("", DEFAULT_CLASS, self.colour);
        self.carried = None;
        self.saved = -1;
        self.file_cursor = -1;
        self.hidden = None;
    }

    /// Motions 5 and 10: the memory card list, left-aligned at x + 4, y 70,
    /// scale 0.6.
    fn goto_card(&mut self, m: Motion) {
        self.state = State::Selecting(m);
        self.menu.items = vec![Item::new(CARD_SLOT, ENTRY)];
        self.menu.x = 128.0 * self.col as f32 + 4.0;
        self.menu.y = 70.0;
        self.menu.scale = 0.6;
        self.menu.dy = 4.0;
        self.menu.sel = 0;
    }

    /// Motions 8 and 13: the save file's entries at x + 8, y 70, scale 0.6,
    /// this player's last file selected.
    fn goto_files(&mut self, m: Motion, names: &[String], claimed: &[usize], load: bool) {
        self.state = State::Selecting(m);
        self.menu.items = names
            .iter()
            .enumerate()
            .map(|(k, n)| Item::new(n, ENTRY + k as i32))
            .collect();
        self.menu.x = 128.0 * self.col as f32 + 8.0;
        self.menu.y = 70.0;
        self.menu.scale = 0.6;
        self.menu.dy = 4.0;
        self.menu.sel = self.file_cursor.max(0) as usize % names.len().max(1);
        self.file_enables(names, claimed, load);
    }

    /// setup_file_entries: entries another player has claimed are disabled,
    /// and so are empty ones when loading.
    fn file_enables(&mut self, names: &[String], claimed: &[usize], load: bool) {
        for (k, it) in self.menu.items.iter_mut().enumerate() {
            let empty = names
                .get(k)
                .is_some_and(|n| n.starts_with(crate::save::EMPTY));
            it.enabled = !claimed.contains(&k) && (!load || !empty);
        }
    }

    /// PlayerLoadSaveFile: the hero's name, colour and last class, with that
    /// class's state.
    /// Also player_get_from_save's PlayerControl from the save, and help_disp
    /// keeps only what was ever seen. Returns the hero's pad settings.
    fn apply_hero(&mut self, h: HeroSave) -> PlayerPrefs {
        let h = h.loaded();
        self.name = h.name.clone();
        self.colour = h.colour & 3;
        self.character = h.character.min(15);
        self.class = self.character;
        self.carried = h.carried(self.character);
        self.hidden = None;
        let controls = h.controls;
        self.hero = h;
        self.saved = 1;
        controls
    }

    /// change_player: the class being left keeps its state in the save image
    /// and the new class takes its own slot (a class never played starts
    /// over at level 1). Class 16 is Sumner, with his fixed loadout
    /// (player_get_from_save), never saved; any other class drops a hidden
    /// character's model.
    fn change_player(&mut self, class: usize, party: &crate::progress::Progress) {
        if let Some(c) = self.carried.take() {
            self.hero.store(self.character, self.colour, &c, party);
        }
        self.character = class;
        self.hidden = None;
        self.carried = if class == crate::stats::SUMNER {
            Some(Carried::sumner())
        } else {
            self.hero.carried(class)
        };
    }

    /// set_hidden_player found a hidden character (by name, or the one
    /// already chosen): init_player_change takes the player straight to
    /// ready as its class and colour, welcomed without a name.
    fn init_hidden(&mut self, h: usize, party: &crate::progress::Progress, sfx: &mut Vec<String>) {
        let hid = crate::cheats::HIDDEN[h];
        self.colour = hid.colour;
        self.change_player(hid.class, party);
        self.hidden = Some(h);
        self.class = self.character;
        // AudioWelcomeBack(i, 0) plays S_WELCOME for a new character,
        // AudioWelcome(i, 0) S_WELCOMEBACK.
        let exp = self.carried.as_ref().map_or(0, |c| c.exp);
        sfx.push(
            if exp == 0 {
                "VOICE:S_WELCOME"
            } else {
                "VOICE:S_WELCOMEBACK"
            }
            .into(),
        );
        self.lock_in();
        self.in_tower = true;
        self.state = State::Ready;
    }

    /// PlayerWriteSaveFile: store the live state (and PlayerControl) and
    /// write the image.
    fn write(
        &mut self,
        card: &mut MemCard,
        party: &crate::progress::Progress,
        controls: PlayerPrefs,
    ) -> bool {
        let c = self.carried.clone().unwrap_or_else(Carried::new_character);
        self.hero.name = self.name.clone();
        self.hero.store(self.character, self.colour, &c, party);
        self.hero.controls = controls;
        card.write(self.save_file, &self.hero)
    }

    /// The state a class would play with: the live one for the current class.
    fn class_carried(&self, class: usize) -> Option<Carried> {
        if class == self.character && self.carried.is_some() {
            self.carried.clone()
        } else {
            self.hero.carried(class)
        }
    }

    /// update_class_spec outside the class pick: the weapon art shows behind
    /// the character menu, quit confirm and memory card screens; the name
    /// banner only on the character menu.
    fn update_panel(&mut self) {
        let State::Selecting(m) = self.state else {
            return;
        };
        if matches!(m, Motion::Top | Motion::Name | Motion::Class) {
            return;
        }
        let weapon = format!("S12_WEAP_{}", CLASSES[self.character & 7]);
        let w = self.slot(SlotId::Weapon);
        w.tex = Some(weapon);
        w.hidden = false;
        w.size = Some((128.0, 320.0));
        self.slot(SlotId::Question).hidden = true;
        if m != Motion::Character {
            self.slot(SlotId::Name).hidden = true;
        }
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
        let delete = dx < 0 || (inp.b_pressed && !self.name.is_empty());
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
        let unlock = sel.progress.class_unlock | self.hero.class_unlock;
        // do_sel_menu's message block: font 0, scale 1.2, white, centred on
        // the column, lines 10 apart (None skips a line).
        let lines = |s: &mut Scr, y: f32, ls: &[Option<&str>]| {
            for (k, l) in ls.iter().enumerate() {
                if let Some(l) = l {
                    s.text(1.2, -cx, y + 10.0 * k as f32, FONT_8X8, 0xFFFFFF, 0, l);
                }
            }
        };
        const DO_NOT_TOUCH: [Option<&str>; 5] = [
            Some("Do not touch"),
            Some("the POWER"),
            Some("button"),
            Some("or the"),
            Some("Memory Card!"),
        ];
        let card_text = |s: &mut Scr| {
            // Mode 10's text, 3 lines above the card list's y 70 (less 8).
            let ls = [
                "The",
                "Memory",
                "Card",
                "in Slot A",
                "will now be",
                "accessed.",
                "Do not touch",
                "the",
                "POWER Button",
                "or the",
                "Memory Card",
            ];
            lines(s, 70.0 - 8.0 - 30.0, &ls.map(Some));
        };
        // Progress / result screens (modes 9, 12 and 14 past their prompts).
        let working = |s: &mut Scr, head: [&str; 3]| {
            lines(s, 100.0, &head.map(Some));
            lines(s, 140.0, &DO_NOT_TOUCH);
        };
        match self.state {
            State::Selecting(Motion::Top) | State::Selecting(Motion::Character) => {
                self.menu.draw(s, 0);
                if self.state == State::Selecting(Motion::Character) {
                    let exp = self.carried.as_ref().map_or(0, |c| c.exp);
                    let level = format!("Level {}", crate::stats::exp_to_level(exp));
                    s.text(1.2, -cx, 292.0, FONT_8X8, 0xFFFFFF, 0, &level);
                }
                footer(s, x, self.state == State::Selecting(Motion::Top));
            }
            State::Selecting(Motion::QuitConfirm) => {
                lines(
                    s,
                    100.0,
                    &[Some("Character"), Some("Not Saved."), Some("Quit Anyway?")],
                );
                self.menu.draw(s, 0);
                footer(s, x, false);
            }
            State::Selecting(Motion::CardLoad) if self.step == 0 => {
                let ls = [
                    "Current",
                    "Character",
                    "Not Saved.",
                    "Load new",
                    "Character",
                    "Anyway?",
                ];
                lines(s, 164.0 - 8.0 - 60.0, &ls.map(Some));
                self.menu.draw(s, 0);
                footer(s, x, false);
            }
            State::Selecting(Motion::CardLoad) | State::Selecting(Motion::CardSave) => {
                self.menu.draw(s, 0);
                card_text(s);
                footer(s, x, true);
            }
            State::Selecting(Motion::NoData) => {
                let ls = ["No Gauntlet", "save data on", "Memory", "Card."];
                lines(s, 100.0, &ls.map(Some));
                footer(s, x, false);
            }
            State::Selecting(Motion::PickLoad) => {
                self.menu.draw(s, 0);
                lines(
                    s,
                    70.0 - 8.0 - 20.0,
                    &[Some("Choose a"), Some("File to Load")],
                );
                footer(s, x, true);
            }
            State::Selecting(Motion::PickSave) => {
                self.menu.draw(s, 0);
                let ls = ["Choose a", "Location to", "Write to"];
                lines(s, 70.0 - 8.0 - 30.0, &ls.map(Some));
                footer(s, x, true);
            }
            State::Selecting(Motion::Loading) => match self.step {
                0..=3 => working(s, ["Loading from", "Memory", "Card..."]),
                t if t >= 4 => lines(s, 100.0, &[Some("Load"), Some("Complete")]),
                -1 => {}
                _ => lines(s, 100.0, &[Some("Load"), Some("Failed!")]),
            },
            State::Selecting(Motion::Writing) => match self.step {
                0 => {
                    lines(
                        s,
                        100.0,
                        &[Some("Overwrite"), Some("Existing"), Some("File?")],
                    );
                    if !sel.card.is_empty(self.save_file) {
                        self.menu.draw(s, 0);
                    }
                    footer(s, x, false);
                }
                1..=3 => working(s, ["Writing to", "Memory", "Card..."]),
                t if t >= 4 => lines(s, 100.0, &[Some("Save"), Some("Complete")]),
                -1 => {}
                _ => lines(s, 100.0, &[Some("Save"), Some("Failed!")]),
            },
            State::Selecting(Motion::Create) => match self.step {
                0 => {
                    let need = format!(
                        "{} Blocks",
                        crate::save::SAVE_FILE_SIZE / crate::save::BLOCK
                    );
                    let free =
                        format!("{} Blocks Free", sel.card.free_bytes() / crate::save::BLOCK);
                    let ls = [
                        Some("No Gauntlet"),
                        Some("save data on"),
                        Some("Memory"),
                        Some("Card."),
                        None,
                        Some("Create"),
                        Some(need.as_str()),
                        Some("Save File?"),
                        None,
                        Some(free.as_str()),
                    ];
                    lines(s, 100.0 - 50.0, &ls);
                    self.menu.draw(s, 0);
                    footer(s, x, false);
                }
                1..=3 => working(s, ["Creating", "Gauntlet", "Save File..."]),
                t if t >= 1000 => lines(s, 100.0, &[Some("Insufficient"), Some("Free Space.")]),
                t if t >= 4 => lines(s, 100.0, &[Some("Save File"), Some("Created.")]),
                _ => lines(
                    s,
                    100.0,
                    &[Some("Create"), Some("Save File"), Some("Failed!")],
                ),
            },
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
                let known = known_of(self.class, unlock);
                if known {
                    self.draw_stats(s, x);
                }
                if let (Some(l), Some(r)) = (s.ui.get("BUTTON_L"), s.ui.get("BUTTON_R")) {
                    s.blit(l, x + 10.0, 232.0, 19.0, 19.0, 0);
                    s.blit(r, x + 29.0, 232.0, 19.0, 19.0, 0);
                }
                s.text(1.2, x + 56.0, 236.0, FONT_8X8, 0xFFFFFF, 0, "Change");
                if known {
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
        // The class's level and attribute bonuses from the save image.
        let c = self.class_carried(self.class);
        let exp = c.as_ref().map_or(0, |c| c.exp);
        let lvl = crate::stats::exp_to_level(exp) as f32;
        let b = c.map(|c| c.bonus).unwrap_or_default();
        let add = [b.fight, b.speed, b.armor, b.magic];
        let stats: [i32; 4] = std::array::from_fn(|r| {
            ((add[r] + (BASE_STATS[self.class][r] as f32 + (lvl - 1.0) * 5.0)) as i32).min(999)
        });
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
            let v = format!("{:03}", stats[r]);
            s.text(0.5, x + 84.0, y, FONT_INITIALS, 0xFFFFFF, 0, &v);
        }
        let level = if exp > 0 {
            format!("Level {}", lvl as u32)
        } else {
            "NEW".to_string()
        };
        s.text(1.2, -(64.0 + x), 292.0, FONT_8X8, 0xFFFFFF, 0, &level);
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
    /// Random presses on four pads for a long while: player select (with its
    /// memory-card screens) must never panic.
    #[test]
    fn fuzzed_players_never_crash_select() {
        for seed in 1..=6u64 {
            let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
            let mut next = move || {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                x
            };
            let srcs = [
                Source::Keyboard,
                Source::Device(1),
                Source::Device(2),
                Source::Device(3),
            ];
            let mut sel = Select::with_first(Source::Keyboard);
            for _ in 0..20_000 {
                let inputs: Vec<(Source, PlayerInput)> = srcs
                    .iter()
                    .map(|&src| {
                        let r = next();
                        let b = |k: u32| r >> k & 7 == 0;
                        let dir = (r >> 40) % 5;
                        let stick = match dir {
                            1 => glam::Vec2::new(0.0, 1.0),
                            2 => glam::Vec2::new(0.0, -1.0),
                            3 => glam::Vec2::new(1.0, 0.0),
                            4 => glam::Vec2::new(-1.0, 0.0),
                            _ => glam::Vec2::ZERO,
                        };
                        (
                            src,
                            PlayerInput {
                                stick,
                                attack: b(0),
                                attack_pressed: b(0),
                                b_pressed: b(4),
                                turbo: b(4),
                                start_pressed: b(8) && b(12),
                                magic_pressed: b(16),
                                ..Default::default()
                            },
                        )
                    })
                    .collect();
                if sel.update(&inputs).is_some() || sel.leave {
                    sel = Select::with_first(Source::Keyboard);
                }
            }
        }
    }

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
        // One announcement, naming the class picked.
        let voices: Vec<_> = sel.sfx.iter().filter(|s| s.starts_with("VOICE:")).collect();
        assert_eq!(voices, ["VOICE:S_WELCOME+S_YELJES1S|VOICE:S_WELCOME"]);
    }

    #[test]
    fn secret_classes_are_locked() {
        let sel = Select::new(Progress::default());
        assert!(sel.known(7));
        assert!(!sel.known(8));
        assert_eq!(step(15, 1, 0), 0);
    }

    fn a() -> PlayerInput {
        PlayerInput {
            attack_pressed: true,
            ..Default::default()
        }
    }
    fn b() -> PlayerInput {
        PlayerInput {
            b_pressed: true,
            ..Default::default()
        }
    }
    fn stick(x: f32, y: f32) -> PlayerInput {
        PlayerInput {
            stick: Vec2::new(x, y),
            ..Default::default()
        }
    }
    fn idle(sel: &mut Select, n: usize) -> Option<Vec<(String, String)>> {
        let mut out = None;
        for _ in 0..n {
            out = out.or(tick(sel, PlayerInput::default()));
        }
        out
    }
    fn motion(sel: &Select, col: usize) -> State {
        sel.players[col].as_ref().unwrap().state
    }
    fn pl(sel: &Select, col: usize) -> &Player {
        sel.players[col].as_ref().unwrap()
    }

    /// A hero back from a level: a level-5-ish Warrior with gold, and a party
    /// that has done a few things.
    fn veteran() -> (Select, Carried, crate::progress::Progress) {
        let mut c = Carried::new_character();
        c.exp = 5000;
        c.level = crate::stats::exp_to_level(5000);
        c.gold = 999;
        c.keys = 2;
        c.potions = vec![1, 1];
        let mut progress = crate::progress::Progress::default();
        progress.runes = 0x6;
        progress.crystals[3] = 2;
        progress.beaten[1] = 0b11;
        let panels = [crate::hud::PanelInfo {
            slot: 0,
            name: "BOB".into(),
            colour: 2,
            class: 0,
            hidden: None,
        }];
        let sel = Select::from_game(&panels, &[Source::Keyboard]).with_party(Party {
            card: MemCard::in_memory(),
            carry: [Some(c.clone()), None, None, None],
            heroes: Default::default(),
            progress: progress.clone(),
            controls: Default::default(),
        });
        (sel, c, progress)
    }

    /// From the character menu (Done selected) up to Save, then through the
    /// card list, the create prompt and the file list to a written file.
    fn save_to(sel: &mut Select, file: usize) {
        for _ in 0..4 {
            tick(sel, stick(0.0, 1.0));
        }
        assert_eq!(pl(sel, 0).menu.items[pl(sel, 0).menu.sel].code, SAVE);
        tick(sel, a());
        assert_eq!(motion(sel, 0), State::Selecting(Motion::CardSave));
        assert_eq!(pl(sel, 0).menu.items[0].text, " SLOT  1");
        tick(sel, a());
        if !sel.card.has_save_file() {
            // "No Gauntlet save data ... Create 15 Blocks Save File?" No is
            // preselected.
            assert_eq!(motion(sel, 0), State::Selecting(Motion::Create));
            assert_eq!(pl(sel, 0).menu.sel, 1);
            tick(sel, stick(0.0, 1.0));
            tick(sel, a());
            assert!((1..4).contains(&pl(sel, 0).step), "Creating...");
            idle(sel, 30);
            assert!(sel.card.has_save_file());
            assert!(pl(sel, 0).step >= 4, "Save File Created.");
        }
        idle(sel, 35);
        assert_eq!(motion(sel, 0), State::Selecting(Motion::PickSave));
        for _ in 0..file {
            tick(sel, stick(0.0, -1.0));
        }
        // The 30-tick debounce from an earlier save may still be running.
        idle(sel, 16);
        tick(sel, a());
        assert_eq!(motion(sel, 0), State::Selecting(Motion::Writing));
    }

    #[test]
    fn save_then_load_restores_the_hero() {
        let (mut sel, carried, progress) = veteran();
        assert_eq!(pl(&sel, 0).saved, 0);
        save_to(&mut sel, 2);
        idle(&mut sel, 30);
        assert!(pl(&sel, 0).step >= 4, "Save Complete");
        idle(&mut sel, 35);
        // Back to the character menu with Done selected.
        assert_eq!(motion(&sel, 0), State::Selecting(Motion::Character));
        assert_eq!(pl(&sel, 0).menu.items[pl(&sel, 0).menu.sel].code, DONE);
        assert_eq!(pl(&sel, 0).saved, 1);
        assert_eq!(sel.card.names()[2], "BOB");
        assert!(sel.card.is_empty(0));
        let card = sel.card().clone();

        // A new game: New / Load, down to Load.
        let mut sel = Select::with_first(Source::Keyboard).with_party(Party {
            card,
            ..Default::default()
        });
        tick(&mut sel, stick(0.0, -1.0));
        tick(&mut sel, a());
        assert_eq!(motion(&sel, 0), State::Selecting(Motion::CardLoad));
        tick(&mut sel, a());
        assert_eq!(motion(&sel, 0), State::Selecting(Motion::PickLoad));
        let menu = &pl(&sel, 0).menu;
        assert_eq!(menu.items.len(), crate::save::FILES);
        assert!(!menu.items[0].enabled, "Empty entries can't be loaded");
        assert!(menu.items[2].enabled);
        // The cursor skips the empty entries.
        tick(&mut sel, a());
        assert_eq!(motion(&sel, 0), State::Selecting(Motion::Loading));
        assert_eq!(pl(&sel, 0).save_file, 2);
        idle(&mut sel, 30);
        assert!(pl(&sel, 0).step >= 4, "Load Complete");
        assert_eq!(pl(&sel, 0).name, "BOB");
        assert_eq!(pl(&sel, 0).colour, 2);
        assert_eq!(pl(&sel, 0).saved, 1);
        idle(&mut sel, 35);
        // The loaded hero picks a class, their own highlighted.
        assert_eq!(motion(&sel, 0), State::Selecting(Motion::Class));
        assert_eq!(pl(&sel, 0).class, 0);
        tick(&mut sel, a());
        assert_eq!(motion(&sel, 0), State::Selecting(Motion::Character));
        assert!(sel.sfx.iter().any(|s| s.starts_with("VOICE:S_WELCOMEBACK")));
        tick(&mut sel, a());
        assert_eq!(motion(&sel, 0), State::Ready);
        let out = idle(&mut sel, 60);
        assert_eq!(out, Some(vec![("WAR".to_string(), "RED".to_string())]));
        assert_eq!(sel.carried(0), Some(carried));
        let mut party = crate::progress::Progress::default();
        party.crystals[3] = 3;
        sel.merge_progress(&mut party);
        assert_eq!(party.runes, progress.runes);
        assert_eq!(party.beaten[1], progress.beaten[1]);
        assert_eq!(party.crystals[3], 3, "the party's larger count stays");
    }

    #[test]
    fn load_with_no_save_file() {
        let mut sel = Select::with_first(Source::Keyboard);
        tick(&mut sel, stick(0.0, -1.0));
        tick(&mut sel, a());
        tick(&mut sel, a());
        assert_eq!(motion(&sel, 0), State::Selecting(Motion::NoData));
        idle(&mut sel, 50);
        assert_eq!(motion(&sel, 0), State::Selecting(Motion::NoData));
        idle(&mut sel, 15);
        assert_eq!(motion(&sel, 0), State::Selecting(Motion::Top));
        // B from the card list goes back too.
        tick(&mut sel, stick(0.0, -1.0));
        tick(&mut sel, a());
        tick(&mut sel, b());
        assert_eq!(motion(&sel, 0), State::Selecting(Motion::Top));
    }

    #[test]
    fn quit_asks_only_when_not_saved() {
        let (mut sel, _, _) = veteran();
        tick(&mut sel, stick(0.0, 1.0));
        tick(&mut sel, a());
        assert_eq!(motion(&sel, 0), State::Selecting(Motion::QuitConfirm));
        assert_eq!(pl(&sel, 0).menu.sel, 1, "No preselected");
        tick(&mut sel, a());
        assert_eq!(motion(&sel, 0), State::Selecting(Motion::Character));

        save_to(&mut sel, 0);
        idle(&mut sel, 130);
        tick(&mut sel, stick(0.0, 1.0));
        tick(&mut sel, a());
        assert!(sel.players[0].is_none(), "a saved hero quits at once");
    }

    #[test]
    fn load_over_an_unsaved_hero_asks_first() {
        let (mut sel, _, _) = veteran();
        tick(&mut sel, stick(0.0, 1.0));
        tick(&mut sel, stick(0.0, 1.0));
        assert_eq!(pl(&sel, 0).menu.items[pl(&sel, 0).menu.sel].code, LOAD);
        tick(&mut sel, a());
        assert_eq!(motion(&sel, 0), State::Selecting(Motion::CardLoad));
        assert_eq!(pl(&sel, 0).step, 0, "Current Character Not Saved.");
        tick(&mut sel, b());
        assert_eq!(motion(&sel, 0), State::Selecting(Motion::Character));
        tick(&mut sel, stick(0.0, 1.0));
        tick(&mut sel, stick(0.0, 1.0));
        tick(&mut sel, a());
        tick(&mut sel, stick(0.0, 1.0));
        tick(&mut sel, a());
        assert_eq!(pl(&sel, 0).step, 1);
        assert_eq!(pl(&sel, 0).menu.items[0].text, " SLOT  1");
    }

    #[test]
    fn overwrite_prompt_and_claimed_files() {
        let (mut sel, _, _) = veteran();
        save_to(&mut sel, 0);
        idle(&mut sel, 130);
        // Saving to the same file again asks first; No goes back to the list.
        save_to(&mut sel, 0);
        assert_eq!(pl(&sel, 0).step, 0);
        assert_eq!(pl(&sel, 0).menu.items[0].text, "Yes");
        assert_eq!(pl(&sel, 0).menu.sel, 1);
        tick(&mut sel, a());
        assert_eq!(motion(&sel, 0), State::Selecting(Motion::PickSave));
        assert_eq!(pl(&sel, 0).file_cursor, -1);
        // Yes overwrites.
        idle(&mut sel, 16);
        tick(&mut sel, a());
        tick(&mut sel, stick(0.0, 1.0));
        tick(&mut sel, a());
        assert!(
            (1..4).contains(&pl(&sel, 0).step),
            "Writing to Memory Card..."
        );
        idle(&mut sel, 130);
        assert_eq!(motion(&sel, 0), State::Selecting(Motion::Character));

        // A second player can't pick the file the first one saved to.
        sel.join(Source::Device(1));
        let names = sel.card.names();
        let p1 = sel.players[1].as_mut().unwrap();
        p1.goto_files(Motion::PickSave, &names, &[], false);
        sel.update(&[]);
        let items = &pl(&sel, 1).menu.items;
        assert!(!items[0].enabled);
        assert!(items[1].enabled, "empty entries can be saved to");
    }

    #[test]
    fn changing_class_uses_that_class_slot() {
        let (mut sel, carried, _) = veteran();
        // Change: the Warrior's state goes into the save image and the
        // Archer starts over.
        tick(&mut sel, stick(0.0, 1.0));
        tick(&mut sel, stick(0.0, 1.0));
        tick(&mut sel, stick(0.0, 1.0));
        assert_eq!(pl(&sel, 0).menu.items[pl(&sel, 0).menu.sel].code, CHANGE);
        tick(&mut sel, a());
        assert_eq!(motion(&sel, 0), State::Selecting(Motion::Class));
        for _ in 0..3 {
            tick(&mut sel, stick(1.0, 0.0));
        }
        assert_eq!(pl(&sel, 0).class, 3);
        tick(&mut sel, a());
        assert_eq!(pl(&sel, 0).character, 3);
        assert!(pl(&sel, 0).carried.is_none(), "a new character");
        assert_eq!(pl(&sel, 0).hero.carried(0), Some(carried.clone()));
        // And back: the Warrior is as he was.
        tick(&mut sel, stick(0.0, 1.0));
        tick(&mut sel, stick(0.0, 1.0));
        tick(&mut sel, stick(0.0, 1.0));
        tick(&mut sel, a());
        for _ in 0..3 {
            tick(&mut sel, stick(-1.0, 0.0));
        }
        tick(&mut sel, a());
        assert_eq!(pl(&sel, 0).carried, Some(carried));
        // The Archer was stored as a level 1 character.
        assert_eq!(pl(&sel, 0).hero.carried(3).map(|c| c.exp), None);
    }

    #[test]
    fn ready_start_returns_to_the_character_menu() {
        let (mut sel, _, _) = veteran();
        sel.join(Source::Device(1));
        let both = |sel: &mut Select, inp: PlayerInput| {
            sel.update(&[(Source::Keyboard, inp)]);
            sel.update(&[]);
        };
        both(&mut sel, a());
        assert_eq!(motion(&sel, 0), State::Ready);
        both(
            &mut sel,
            PlayerInput {
                start_pressed: true,
                ..Default::default()
            },
        );
        assert_eq!(motion(&sel, 0), State::Selecting(Motion::Character));
    }

    /// A new hero named `name`: New, the name, the end mark and its blink.
    fn named(name: &str) -> Select {
        let mut sel = Select::with_first(Source::Keyboard);
        tick(&mut sel, a());
        sel.players[0].as_mut().unwrap().name = name.into();
        tick(&mut sel, a());
        idle(&mut sel, 40);
        sel
    }

    /// Load card file `file` into a new game's first column (cursor on it).
    fn load_from(card: MemCard) -> Select {
        let mut sel = Select::with_first(Source::Keyboard).with_party(Party {
            card,
            ..Default::default()
        });
        tick(&mut sel, stick(0.0, -1.0));
        tick(&mut sel, a());
        tick(&mut sel, a());
        tick(&mut sel, a());
        assert_eq!(motion(&sel, 0), State::Selecting(Motion::Loading));
        idle(&mut sel, 30);
        sel
    }

    #[test]
    fn hidden_names_skip_the_class_pick() {
        let sel = named("ICE600");
        let p = pl(&sel, 0);
        assert_eq!(p.state, State::Ready);
        assert_eq!((p.character, p.colour, p.hidden), (4, 2, Some(0)));
        let panel = &sel.panels()[0];
        assert_eq!((panel.class, panel.colour, panel.hidden), (4, 2, Some(0)));
        assert!(sel.sfx.iter().any(|s| s == "VOICE:S_WELCOME"));
        // DBRNKR is row 16, AYA555's Valkyrie.
        assert_eq!(pl(&named("DBRNKR"), 0).hidden, Some(16));
        assert_eq!(pl(&named("DBRNKR"), 0).character, 1);
        // Other names pick a class.
        assert_eq!(motion(&named("BOB"), 0), State::Selecting(Motion::Class));
        assert_eq!(motion(&named("10000K"), 0), State::Selecting(Motion::Class));
        // Change keeps a hidden character one.
        let mut sel = named("PNK666");
        sel.players[0].as_mut().unwrap().open_character_menu(CHANGE);
        tick(&mut sel, a());
        assert_eq!(motion(&sel, 0), State::Ready);
        assert_eq!(pl(&sel, 0).hidden, Some(4));
        // A saved hidden character loads straight back as one.
        sel.players[0].as_mut().unwrap().open_character_menu(DONE);
        save_to(&mut sel, 0);
        idle(&mut sel, 70);
        let mut sel = load_from(sel.card().clone());
        idle(&mut sel, 35);
        assert_eq!(motion(&sel, 0), State::Ready);
        assert_eq!((pl(&sel, 0).character, pl(&sel, 0).hidden), (7, Some(4)));
    }

    #[test]
    fn unlocked_sumner_plays_his_fixed_loadout() {
        assert!(!known_of(16, 0xFF));
        assert!(known_of(16, 0x100));
        assert_eq!(step(15, 1, 0x100), 16);
        assert_eq!(step(15, 1, 0xFF), 0);
        let mut sel = named("BOB");
        let p = sel.players[0].as_mut().unwrap();
        p.hero.class_unlock = 0x100;
        p.class = 16;
        tick(&mut sel, a());
        assert_eq!(motion(&sel, 0), State::Ready);
        let c = sel.carried(0).unwrap();
        assert_eq!((c.level, c.exp, c.gold, c.keys), (99, 0x54218, 5000, 9));
        assert_eq!((c.potions.len(), c.health), (9, 9999.0));
        assert_eq!(sel.panels()[0].class, 16);
        // Leaving him saves nothing over a class.
        let p = sel.players[0].as_mut().unwrap();
        p.change_player(2, &Default::default());
        assert_eq!(p.carried, None);
        assert!(p.hero.classes.iter().all(|c| c.carried.is_none()));
        // A secret class shows once its bit is set.
        let mut sel = named("ANN");
        sel.players[0].as_mut().unwrap().hero.class_unlock = 1 << 2;
        sel.players[0].as_mut().unwrap().class = 10;
        tick(&mut sel, a());
        assert_eq!(motion(&sel, 0), State::Ready);
        assert_eq!(sel.panels()[0].class, 10);
    }

    #[test]
    fn a_hero_keeps_its_pad_settings_and_seen_messages() {
        let (mut sel, _, _) = veteran();
        let mine = PlayerPrefs {
            control_style: 2,
            rumble: false,
            auto_attack: false,
            auto_aim: true,
        };
        sel.controls[0] = mine;
        sel.players[0].as_mut().unwrap().hero.help_disp[5] =
            crate::save::SEEN_NOW | crate::save::SEEN_EVER;
        save_to(&mut sel, 1);
        idle(&mut sel, 70);
        let card = sel.card().clone();
        assert_eq!(card.load(1).unwrap().controls, mine);
        let sel = load_from(card);
        assert_eq!(sel.controls()[0], mine, "player_get_from_save");
        assert_eq!(pl(&sel, 0).hero.help_disp[5], crate::save::SEEN_EVER);
    }
}
