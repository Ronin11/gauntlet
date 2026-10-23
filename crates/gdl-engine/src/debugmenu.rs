//! Developer debug menu. Not part of the original game: a testing tool for
//! warping to any level and setting progress, unlocks and the party.
//!
//! Opened with F1 on the keyboard, or on a gamepad by holding L + R + Z and
//! pressing Start, from the title screen, the tower or a level (the game is
//! frozen while it is open). Up/down choose, left/right change a value, A or
//! Enter applies, B goes back a page (closes on the main page), Esc or F1
//! closes.
//!
//! Progress edits go into the party's [`Progress`] (and the level's live
//! copies of it), the same state the game saves, so a debug session can be
//! saved like any other; every value is kept inside the range the game
//! itself produces, so a save made afterwards still loads normally.

use crate::cheats::{apply_cheats, CHEATS, HIDDEN};
use crate::game::unlock::SECRET_CLASS;
use crate::game::{max_health, Game, Kind, LevelEnd, CRYSTALS_NEEDED, GARG_NEEDED};
use crate::hud::PanelInfo;
use crate::input::{PlayerInput, Source};
use crate::powerups as pu;
use crate::progress::{Progress, CRYSTAL_ORDER};
use crate::save::HeroSave;
use crate::screen::Scr;
use crate::select::CLASSES;
use gdl_formats::wdata::{Worlds, REALMS, TOWER_WAVE};

/// Ticks a held direction waits before repeating, then between repeats.
const REPEAT_DELAY: u32 = 18;
const REPEAT_RATE: u32 = 4;
/// Rows shown at once.
const VISIBLE: usize = 17;
/// Ticks a status message stays up.
const STATUS_TICKS: u32 = 240;
/// Boss runes: bits 1..11 (positions in CRYSTAL_ORDER after the tower).
const ALL_BOSS_RUNES: u16 = 0xFFE;
/// The 13 runestones.
const ALL_SHARDS: u16 = 0x1FFF;
const MAX_GOLD: u32 = 999_999;

/// The gamepad combo: L, R and Z held as Start is pressed.
pub fn open_combo(i: &PlayerInput) -> bool {
    i.charge && i.strafe && i.combo && i.start_pressed
}

/// A level the warp page lists.
#[derive(Clone, Debug, PartialEq)]
pub struct LevelInfo {
    /// Directory code ("A1" -> levelA1).
    pub code: String,
    pub title: String,
    pub wave: u32,
    pub rune: i16,
    pub legend: i16,
    pub boss: bool,
}

/// A realm and its levels, in play order.
#[derive(Clone, Debug, PartialEq)]
pub struct RealmInfo {
    pub realm: u32,
    pub name: &'static str,
    pub levels: Vec<LevelInfo>,
}

/// A readable name per realm type.
pub fn realm_name(realm: u32) -> &'static str {
    match realm {
        1 => "Castle",
        2 => "Mountain",
        3 => "Desert",
        4 => "Forest",
        5 => "Temple",
        6 => "Hell",
        7 => "Town",
        8 => "Battle",
        9 => "Ice",
        10 => "Dream",
        11 => "Sky",
        12 => "Secret",
        13 => "Tower",
        _ => "?",
    }
}

/// The realm each crystal colour opens (progress.rs REALM_CRYSTAL, inverted).
const CRYSTAL_REALM: [u32; 9] = [0, 7, 2, 1, 11, 4, 3, 9, 10];

/// Every realm's levels (sWorldDataTypes order, the test realm left out).
pub fn realm_table(w: &Worlds) -> Vec<RealmInfo> {
    REALMS
        .iter()
        .filter(|(t, _, _)| *t != 0)
        .filter_map(|&(t, _, _)| {
            let r = w.realm(t)?;
            let levels = r
                .levels
                .iter()
                .enumerate()
                .map(|(i, l)| LevelInfo {
                    code: l.name.clone(),
                    title: format!("{} {}", l.prep, l.title).trim().to_string(),
                    wave: (t << 8) | i as u32,
                    rune: l.rune,
                    legend: l.legend,
                    // A realm's last level is its Guardian's.
                    boss: t <= 11 && i + 1 == r.levels.len(),
                })
                .collect();
            Some(RealmInfo {
                realm: t,
                name: realm_name(t),
                levels,
            })
        })
        .collect()
}

/// Power-ups the party page can give: the shop's and the pickups' values
/// (type, flags, amount, seconds; negative seconds are use-counted).
const POWERUPS: [(u8, u32, f32, f32); 30] = [
    (pu::WEAPON, pu::W_3WAY, 0.0, 45.0),
    (pu::WEAPON, pu::W_5WAY, 0.0, 45.0),
    (pu::WEAPON, pu::W_SUPER, 5.0, -1.0),
    (pu::WEAPON, pu::W_REFLECT, 0.0, 90.0),
    (pu::WEAPON, pu::W_RAPID, 0.0, 30.0),
    (pu::WEAPON, pu::W_HAMMER, 3.0, -1.0),
    (pu::WEAPON, 1, 0.0, 90.0),
    (pu::WEAPON, 2, 0.0, 90.0),
    (pu::WEAPON, 3, 0.0, 90.0),
    (pu::WEAPON, 4, 0.0, 90.0),
    (pu::ARMOR, pu::A_INVULN, 0.0, 30.0),
    (pu::ARMOR, pu::A_GOLD | pu::A_INVULN, 0.0, 20.0),
    (pu::ARMOR, pu::A_REFLECT, 0.0, 30.0),
    (pu::ARMOR, pu::A_ANTIDEATH, 0.0, 120.0),
    (pu::ARMOR, pu::A_FIRE, 0.0, 15.0),
    (pu::ARMOR, pu::A_LIGHTNING, 0.0, 15.0),
    (pu::ARMOR, 0x2008, 0.0, 15.0),
    (pu::SPECIAL, pu::S_LEVITATE, 0.0, 60.0),
    (pu::SPECIAL, pu::S_XRAY, 0.0, 120.0),
    (pu::SPECIAL, pu::S_INVISIBLE, 0.0, 15.0),
    (pu::SPECIAL, pu::S_STOPTIME, 0.0, 25.0),
    (pu::SPECIAL, 0x10, 5.0, -1.0),
    (pu::SPECIAL, 0x20, 5.0, -1.0),
    (pu::SPECIAL, 0x40, 5.0, -1.0),
    (pu::SPECIAL, pu::S_PHOENIX, 0.0, 45.0),
    (pu::SPECIAL, pu::S_GROWTH, 0.0, 30.0),
    (pu::SPECIAL, pu::S_SHRINK, 0.0, 15.0),
    (pu::SPECIAL, 0x10000, 4.0, 40.0),
    (pu::SPECIAL, pu::S_HANDOFDEATH, 0.0, 120.0),
    (pu::SPECIAL, pu::S_VAMPIRE, 0.0, 120.0),
];

/// Pojo, Mikey and the turbo special come from levels only; they are listed
/// after the shop's.
const POWERUPS_EXTRA: [(u8, u32, f32, f32); 3] = [
    (pu::SPECIAL, pu::S_POJO, 0.0, 30.0),
    (pu::SPECIAL, pu::S_MIKEY, 0.0, 120.0),
    (pu::SPECIAL, pu::S_TURBO, 0.0, 30.0),
];

fn powerup(i: usize) -> (u8, u32, f32, f32) {
    POWERUPS
        .get(i)
        .copied()
        .unwrap_or_else(|| POWERUPS_EXTRA[(i - POWERUPS.len()) % POWERUPS_EXTRA.len()])
}

const POWERUP_COUNT: usize = POWERUPS.len() + POWERUPS_EXTRA.len();

fn powerup_name(i: usize) -> &'static str {
    let (kind, flags, ..) = powerup(i);
    pu::label(&pu::Slot {
        kind,
        flags,
        ..Default::default()
    })
}

/// The distinct `Cheats[]` names, in table order.
fn cheat_names() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = Vec::new();
    for c in &CHEATS {
        if !v.contains(&c.name) {
            v.push(c.name);
        }
    }
    v
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Main,
    Warp,
    Progress,
    Unlocks,
    Party,
    Level,
    /// File a bug report (bugreport.rs): quick tags, then save.
    Bug,
    States,
}

impl Page {
    fn title(self) -> &'static str {
        match self {
            Page::Main => "DEBUG MENU",
            Page::Warp => "DEBUG: WARP TO LEVEL",
            Page::Progress => "DEBUG: PROGRESS",
            Page::Unlocks => "DEBUG: UNLOCKS",
            Page::Party => "DEBUG: PARTY",
            Page::Level => "DEBUG: LEVEL",
            Page::Bug => "DEBUG: FILE BUG REPORT",
            Page::States => "DEBUG: SAVE STATES",
        }
    }
}

/// What a row does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    Open(Page),
    Close,
    WarpRealm,
    Warp(u32),
    OpenAll,
    ProgRealm,
    Beaten,
    BossRunes,
    Shards,
    CrystalColour,
    Crystals,
    Garg,
    Legend,
    ResetProgress,
    UnlockAll,
    LockAll,
    Class(usize),
    Cheat,
    Target,
    Level,
    Exp,
    Gold,
    Keys,
    Potions,
    FullHealth,
    Powerup,
    ClearPowerups,
    God,
    HeroClass,
    Skin,
    KillMonsters,
    DestroyGenerators,
    KillBoss,
    FinishLevel,
    XRay,
    Compass,
    /// A quick tag for the bug report (bugreport::TAGS index), on or off.
    BugTag(usize),
    SaveBug,
    StateSlot,
    SaveState,
    LoadState,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub item: Item,
    pub label: String,
    pub value: String,
    pub enabled: bool,
}

fn row(item: Item, label: impl Into<String>, value: impl Into<String>) -> Row {
    Row {
        item,
        label: label.into(),
        value: value.into(),
        enabled: true,
    }
}

/// What the app must do after a tick of the menu.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    None,
    /// The menu closed.
    Close,
    /// Load this level (directory name, "levelA1") with the party (the
    /// context's panels, which may have been changed).
    Warp(String),
    /// Write a bug report with these tags; the app answers with
    /// [`DebugMenu::bug_filed`].
    FileBug(Vec<&'static str>),
    /// Write a save state (savestate.rs) of the running level to this slot.
    SaveState(String),
    /// Load the save state in this slot (the menu closes if it loads).
    LoadState(String),
}

/// The game state the menu reads and changes.
pub struct Ctx<'a> {
    pub progress: &'a mut Progress,
    /// The running level or tower (None on the title screen).
    pub game: Option<&'a mut Game>,
    /// The party's HUD panels (class, colour and hidden skin per player).
    pub panels: Option<&'a mut Vec<PanelInfo>>,
    /// Each column's save image.
    pub heroes: &'a mut [Option<HeroSave>; 4],
    /// The Compass option.
    pub compass: &'a mut bool,
}

/// Menu-navigation edges for one tick.
#[derive(Clone, Copy, Debug, Default)]
struct Nav {
    up: bool,
    down: bool,
    left: bool,
    right: bool,
    a: bool,
    b: bool,
}

pub struct DebugMenu {
    pub open: bool,
    /// No damage for the heroes (Game::debug_god), kept across levels.
    pub god: bool,
    /// Secret classes unlocked from the menu this session (save.class_unlock
    /// bits), offered on player select too.
    pub class_unlock: u16,
    /// Who opened it (a warp from the title plays with that controller).
    pub owner: Option<Source>,
    /// Progress was changed while the tower was loaded: it is reloaded on
    /// closing so its portals and barriers follow.
    pub tower_dirty: bool,
    /// Progress set on the title screen, for the next new game.
    title_progress: Option<Progress>,
    page: Page,
    sel: usize,
    scroll: usize,
    /// The main page's row, for coming back to it.
    main_sel: usize,
    realms: Vec<RealmInfo>,
    warp_realm: usize,
    prog_realm: usize,
    crystal: usize,
    target: usize,
    pup: usize,
    cheat: usize,
    hero_class: usize,
    skin: usize,
    /// The save-state slot chosen (savestate::SLOTS, then the autosave).
    state_slot: usize,
    status: Option<(String, u32)>,
    rows: Vec<Row>,
    hold: [u32; 4],
    /// The bug page's tags picked (bugreport::TAGS), and the last report's folder.
    bug_tags: [bool; crate::bugreport::TAGS.len()],
    last_bug: Option<String>,
}

impl Default for DebugMenu {
    fn default() -> Self {
        DebugMenu::new(Vec::new())
    }
}

impl DebugMenu {
    pub fn new(realms: Vec<RealmInfo>) -> DebugMenu {
        DebugMenu {
            open: false,
            god: false,
            class_unlock: 0,
            owner: None,
            tower_dirty: false,
            title_progress: None,
            page: Page::Main,
            sel: 0,
            scroll: 0,
            main_sel: 0,
            realms,
            warp_realm: 0,
            prog_realm: 0,
            crystal: 1,
            target: 0,
            pup: 0,
            cheat: 0,
            hero_class: 0,
            skin: 0,
            state_slot: 0,
            status: None,
            rows: Vec::new(),
            hold: [0; 4],
            bug_tags: [false; crate::bugreport::TAGS.len()],
            last_bug: None,
        }
    }

    /// The level list is read from the disc the first time the menu opens.
    pub fn has_levels(&self) -> bool {
        !self.realms.is_empty()
    }

    pub fn set_levels(&mut self, realms: Vec<RealmInfo>) {
        self.realms = realms;
    }

    /// Open on the main page; the level's progress is brought into the
    /// party's first, so the menu shows (and edits) what is current.
    pub fn show(&mut self, owner: Option<Source>, ctx: &mut Ctx) {
        self.open = true;
        self.owner = owner;
        self.page = Page::Main;
        self.sel = self.main_sel;
        self.scroll = 0;
        self.hold = [u32::MAX; 4];
        if let Some(g) = ctx.game.as_deref() {
            pull(ctx.progress, g);
        }
        self.refresh(ctx);
    }

    pub fn close(&mut self) {
        self.open = false;
        self.status = None;
    }

    /// The progress a new game from the title starts with: what the menu set
    /// on the title screen, else nothing done.
    pub fn new_game_progress(&mut self) -> Progress {
        self.title_progress.take().unwrap_or_default()
    }

    /// Open a page at a row (headless snapshots).
    pub fn goto(&mut self, page: Page, sel: usize, ctx: &mut Ctx) {
        self.open = true;
        self.page = page;
        self.refresh(ctx);
        self.sel = sel.min(self.rows.len().saturating_sub(1));
        self.scroll = self.sel.saturating_sub(VISIBLE - 1);
    }

    pub fn page(&self) -> Page {
        self.page
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub fn status(&self) -> Option<&str> {
        self.status.as_ref().map(|s| s.0.as_str())
    }

    /// Show a message on the menu (the app's save-state results).
    pub fn report(&mut self, s: impl Into<String>) {
        self.say(s);
    }

    /// The save-state slot chosen: "1".."3" or "autosave".
    fn slot_name(&self) -> &'static str {
        use crate::savestate::{AUTOSAVE, SLOTS};
        SLOTS.get(self.state_slot).copied().unwrap_or(AUTOSAVE)
    }

    fn say(&mut self, s: impl Into<String>) {
        self.status = Some((s.into(), STATUS_TICKS));
    }

    /// The app wrote the bug report asked for (its folder) or failed.
    pub fn bug_filed(&mut self, result: Result<std::path::PathBuf, String>) {
        match result {
            Ok(dir) => {
                let name = dir.file_name().map_or(dir.display().to_string(), |n| {
                    n.to_string_lossy().into_owned()
                });
                self.say(format!("Bug report saved: {name}"));
                self.last_bug = Some(dir.display().to_string());
                self.bug_tags = [false; crate::bugreport::TAGS.len()];
            }
            Err(e) => self.say(format!("Bug report failed: {e}")),
        }
    }

    fn nav(&mut self, inputs: &[PlayerInput]) -> Nav {
        let any = |f: &dyn Fn(&PlayerInput) -> bool| inputs.iter().any(f);
        let held = [
            any(&|i| i.stick.y > 0.5 || i.dpad.y > 0.5),
            any(&|i| i.stick.y < -0.5 || i.dpad.y < -0.5),
            any(&|i| i.stick.x < -0.5 || i.dpad.x < -0.5),
            any(&|i| i.stick.x > 0.5 || i.dpad.x > 0.5),
        ];
        let mut fire = [false; 4];
        for k in 0..4 {
            if !held[k] {
                self.hold[k] = 0;
                continue;
            }
            // u32::MAX: held since the menu opened; wait for a release.
            if self.hold[k] == u32::MAX {
                continue;
            }
            self.hold[k] += 1;
            let h = self.hold[k];
            fire[k] =
                h == 1 || (h > REPEAT_DELAY && (h - REPEAT_DELAY).is_multiple_of(REPEAT_RATE));
        }
        Nav {
            up: fire[0],
            down: fire[1],
            left: fire[2],
            right: fire[3],
            a: any(&|i| i.attack_pressed || i.start_pressed),
            b: any(&|i| i.b_pressed),
        }
    }

    /// One tick with every controller's input.
    pub fn update(&mut self, inputs: &[PlayerInput], ctx: &mut Ctx) -> Action {
        if let Some((_, t)) = &mut self.status {
            *t = t.saturating_sub(1);
            if *t == 0 {
                self.status = None;
            }
        }
        let nav = self.nav(inputs);
        self.refresh(ctx);
        let n = self.rows.len();
        if n > 0 {
            if nav.up {
                self.sel = (self.sel + n - 1) % n;
            }
            if nav.down {
                self.sel = (self.sel + 1) % n;
            }
            self.sel = self.sel.min(n - 1);
        }
        let mut action = Action::None;
        if nav.b {
            if self.page == Page::Main {
                self.close();
                return Action::Close;
            }
            self.page = Page::Main;
            self.sel = self.main_sel;
        } else if let Some(r) = self.rows.get(self.sel).cloned() {
            let d = if nav.a {
                Some(0)
            } else if nav.left {
                Some(-1)
            } else if nav.right {
                Some(1)
            } else {
                None
            };
            if let Some(d) = d {
                if r.enabled {
                    crate::bugreport::event(
                        "debug",
                        format!(
                            "{} / {} {} ({})",
                            self.page.title(),
                            r.label.trim(),
                            r.value,
                            match d {
                                0 => "A",
                                -1 => "left",
                                _ => "right",
                            }
                        ),
                    );
                    action = self.apply(r.item, d, ctx);
                } else if d == 0 {
                    self.say("Not available here");
                }
            }
        }
        if action == Action::Close {
            self.close();
            return action;
        }
        self.refresh(ctx);
        if self.sel < self.scroll {
            self.scroll = self.sel;
        } else if self.sel >= self.scroll + VISIBLE {
            self.scroll = self.sel + 1 - VISIBLE;
        }
        action
    }

    /// Carry out a row: `d` is -1 / +1 for left / right, 0 for A.
    pub fn apply(&mut self, item: Item, d: i32, ctx: &mut Ctx) -> Action {
        let mut progress_changed = false;
        let action = match item {
            Item::Open(p) => {
                if d == 0 {
                    self.main_sel = self.sel;
                    self.page = p;
                    self.sel = 0;
                    self.scroll = 0;
                }
                Action::None
            }
            Item::Close => {
                if d == 0 {
                    Action::Close
                } else {
                    Action::None
                }
            }
            Item::WarpRealm => {
                self.warp_realm = step(self.warp_realm, d, self.realms.len());
                Action::None
            }
            Item::Warp(wave) => match self.level_dir(wave) {
                Some(dir) if d == 0 => {
                    if ctx.game.is_some() {
                        sync_party(ctx);
                    }
                    Action::Warp(dir)
                }
                _ => Action::None,
            },
            Item::OpenAll => {
                if d == 0 {
                    open_all(ctx.progress, &self.realms);
                    progress_changed = true;
                    self.say("Every portal, barrier and gate open");
                }
                Action::None
            }
            Item::ProgRealm => {
                self.prog_realm = step(self.prog_realm, d, self.prog_realms().len());
                Action::None
            }
            Item::Beaten => {
                if let Some(r) = self.prog_realms().get(self.prog_realm) {
                    let n = r.levels.len() as i32;
                    let realm = r.realm as usize & 15;
                    let now = ctx.progress.beaten[realm].count_ones() as i32;
                    let k = if d == 0 { n } else { (now + d).clamp(0, n) };
                    set_beaten(ctx.progress, realm, k as u32);
                    progress_changed = true;
                }
                Action::None
            }
            Item::BossRunes => {
                let now = (ctx.progress.runes & ALL_BOSS_RUNES).count_ones() as i32;
                let k = if d == 0 { 11 } else { (now + d).clamp(0, 11) };
                ctx.progress.runes = boss_runes(k as u32);
                progress_changed = true;
                Action::None
            }
            Item::Shards => {
                let now = (ctx.progress.shards & ALL_SHARDS).count_ones() as i32;
                let k = if d == 0 { 13 } else { (now + d).clamp(0, 13) };
                ctx.progress.shards = ((1u32 << k) - 1) as u16;
                progress_changed = true;
                Action::None
            }
            Item::CrystalColour => {
                self.crystal = 1 + step(self.crystal - 1, d, 8);
                Action::None
            }
            Item::Crystals => {
                let c = self.crystal;
                ctx.progress.crystals[c] = crystal_step(ctx.progress.crystals[c], d, c);
                progress_changed = true;
                Action::None
            }
            Item::Garg => {
                for (n, &needed) in ctx.progress.garg_items.iter_mut().zip(&GARG_NEEDED) {
                    *n = count_step(*n, d, needed, 1);
                }
                progress_changed = true;
                Action::None
            }
            Item::Legend => {
                let all = legend_bits(&self.realms);
                let order: Vec<u16> = (0..16).map(|b| 1 << b).filter(|b| all & b != 0).collect();
                let now = order
                    .iter()
                    .filter(|&&b| ctx.progress.legend & b != 0)
                    .count() as i32;
                let k = if d == 0 {
                    order.len() as i32
                } else {
                    (now + d).clamp(0, order.len() as i32)
                };
                ctx.progress.legend = order.iter().take(k as usize).fold(0, |m, b| m | b);
                progress_changed = true;
                Action::None
            }
            Item::ResetProgress => {
                if d == 0 {
                    reset_progress(ctx.progress);
                    if let Some(g) = ctx.game.as_deref_mut() {
                        g.boss_runes = 0;
                    }
                    progress_changed = true;
                    self.say("Progress reset (saves on the card are not touched)");
                }
                Action::None
            }
            Item::UnlockAll | Item::LockAll => {
                if d == 0 {
                    let on = item == Item::UnlockAll;
                    for &c in &SECRET_CLASS {
                        self.set_unlock(ctx, c, on);
                    }
                    self.say(if on {
                        "Every secret hero unlocked"
                    } else {
                        "Secret heroes locked"
                    });
                }
                Action::None
            }
            Item::Class(c) => {
                let on = !self.unlocked(ctx, c);
                self.set_unlock(ctx, c, on);
                Action::None
            }
            Item::Cheat => {
                let names = cheat_names();
                if d != 0 {
                    self.cheat = step(self.cheat, d, names.len());
                } else if let Some(g) = ctx.game.as_deref_mut() {
                    let name = names[self.cheat];
                    for pi in targets(self.target, g) {
                        apply_cheats(name, &mut g.players[pi]);
                    }
                    self.say(format!("Cheat {name} applied"));
                }
                Action::None
            }
            Item::Target => {
                let n = ctx.game.as_deref().map_or(0, |g| g.players.len());
                self.target = step(self.target, d, n + 1);
                Action::None
            }
            Item::Level | Item::Exp | Item::Gold | Item::Keys | Item::Potions => {
                if let Some(g) = ctx.game.as_deref_mut() {
                    for pi in targets(self.target, g) {
                        party_value(&mut g.players[pi], item, d);
                    }
                }
                Action::None
            }
            Item::FullHealth => {
                if d == 0 {
                    if let Some(g) = ctx.game.as_deref_mut() {
                        for pi in targets(self.target, g) {
                            let p = &mut g.players[pi];
                            p.health = max_health(p.level);
                            p.power = 100.0;
                        }
                        self.say("Health and turbo full");
                    }
                }
                Action::None
            }
            Item::Powerup => {
                if d != 0 {
                    self.pup = step(self.pup, d, POWERUP_COUNT);
                } else if let Some(g) = ctx.game.as_deref_mut() {
                    for pi in targets(self.target, g) {
                        give_powerup(&mut g.players[pi], self.pup);
                    }
                    self.say(format!(
                        "{} given (switch it on from the d-pad)",
                        powerup_name(self.pup)
                    ));
                }
                Action::None
            }
            Item::ClearPowerups => {
                if d == 0 {
                    if let Some(g) = ctx.game.as_deref_mut() {
                        for pi in targets(self.target, g) {
                            g.players[pi].powerups.clear();
                        }
                        self.say("Power-ups cleared");
                    }
                }
                Action::None
            }
            Item::God => {
                self.god = !self.god;
                if let Some(g) = ctx.game.as_deref_mut() {
                    g.debug_god = self.god;
                }
                Action::None
            }
            Item::HeroClass | Item::Skin => {
                if d != 0 {
                    if item == Item::HeroClass {
                        self.hero_class = step(self.hero_class, d, CLASSES.len());
                    } else {
                        self.skin = step(self.skin, d, HIDDEN.len() + 1);
                    }
                    Action::None
                } else {
                    self.change_heroes(item, ctx)
                }
            }
            Item::KillMonsters => {
                if d == 0 {
                    if let Some(g) = ctx.game.as_deref_mut() {
                        let n = kill_monsters(g);
                        self.say(format!("{n} monsters killed"));
                    }
                }
                Action::None
            }
            Item::DestroyGenerators => {
                if d == 0 {
                    if let Some(g) = ctx.game.as_deref_mut() {
                        let n = destroy_generators(g);
                        self.say(format!("{n} generators destroyed"));
                    }
                }
                Action::None
            }
            Item::KillBoss => {
                if d == 0 {
                    if let Some(g) = ctx.game.as_deref_mut() {
                        if g.boss.is_some() {
                            g.kill_boss();
                            self.say("The boss takes a fatal hit");
                        }
                    }
                }
                Action::None
            }
            Item::FinishLevel => {
                if d == 0 {
                    if let Some(g) = ctx.game.as_deref_mut() {
                        if finish_level(g) {
                            sync_party_progress(ctx.progress, g);
                            return Action::Close;
                        }
                    }
                }
                Action::None
            }
            Item::XRay => {
                if let Some(g) = ctx.game.as_deref_mut() {
                    let on = !xray_on(g);
                    set_xray(g, on);
                }
                Action::None
            }
            Item::Compass => {
                *ctx.compass = !*ctx.compass;
                Action::None
            }
            Item::BugTag(i) => {
                if let Some(t) = self.bug_tags.get_mut(i) {
                    *t = !*t;
                }
                Action::None
            }
            Item::SaveBug => {
                if d == 0 {
                    self.say("Saving bug report...");
                    Action::FileBug(self.picked_tags())
                } else {
                    Action::None
                }
            }
            Item::StateSlot => {
                self.state_slot = step(self.state_slot, d, crate::savestate::SLOTS.len() + 1);
                Action::None
            }
            Item::SaveState if d == 0 => Action::SaveState(self.slot_name().into()),
            Item::LoadState if d == 0 => Action::LoadState(self.slot_name().into()),
            Item::SaveState | Item::LoadState => Action::None,
        };
        if progress_changed {
            match ctx.game.as_deref_mut() {
                Some(g) => {
                    push(ctx.progress, g);
                    if g.wave == Some(TOWER_WAVE) {
                        self.tower_dirty = true;
                    }
                }
                None => self.title_progress = Some(ctx.progress.clone()),
            }
        }
        action
    }

    /// Change the targeted heroes' class or hidden skin; the level reloads
    /// with them.
    fn change_heroes(&mut self, item: Item, ctx: &mut Ctx) -> Action {
        let (Some(g), Some(panels)) = (ctx.game.as_deref_mut(), ctx.panels.as_deref_mut()) else {
            return Action::None;
        };
        let tg = targets(self.target, g);
        for &pi in &tg {
            let Some(p) = panels.get_mut(pi) else {
                continue;
            };
            if item == Item::HeroClass {
                p.class = self.hero_class;
                p.hidden = None;
            } else if let Some(h) = self.skin.checked_sub(1).and_then(|h| HIDDEN.get(h)) {
                p.class = h.class;
                p.colour = h.colour;
                p.hidden = Some(self.skin - 1);
            } else {
                p.hidden = None;
            }
        }
        let dir = g
            .wave
            .and_then(|w| g.worlds.dir(w))
            .unwrap_or_else(|| g.level.name.clone());
        sync_party(ctx);
        Action::Warp(dir)
    }

    fn unlocked(&self, ctx: &Ctx, class: usize) -> bool {
        let bit = 1u16 << (class - 8);
        self.class_unlock & bit != 0
            || ctx
                .game
                .as_deref()
                .is_some_and(|g| g.players.iter().any(|p| p.class_unlock & bit != 0))
    }

    fn set_unlock(&mut self, ctx: &mut Ctx, class: usize, on: bool) {
        let bit = 1u16 << (class - 8);
        let set = |m: &mut u16| {
            if on {
                *m |= bit
            } else {
                *m &= !bit
            }
        };
        set(&mut self.class_unlock);
        if let Some(g) = ctx.game.as_deref_mut() {
            for p in &mut g.players {
                set(&mut p.class_unlock);
            }
        }
        for h in ctx.heroes.iter_mut().flatten() {
            set(&mut h.class_unlock);
        }
    }

    /// The bug page's tags picked.
    pub fn picked_tags(&self) -> Vec<&'static str> {
        crate::bugreport::TAGS
            .iter()
            .zip(&self.bug_tags)
            .filter(|(_, &on)| on)
            .map(|(t, _)| *t)
            .collect()
    }

    fn level_dir(&self, wave: u32) -> Option<String> {
        self.realms
            .iter()
            .flat_map(|r| &r.levels)
            .find(|l| l.wave == wave)
            .map(|l| format!("level{}", l.code))
    }

    /// The realms with levels to beat (not the secret levels or the tower).
    fn prog_realms(&self) -> Vec<&RealmInfo> {
        self.realms
            .iter()
            .filter(|r| r.realm != 12 && r.realm != 13)
            .collect()
    }

    /// Rebuild the rows for the current page.
    pub fn refresh(&mut self, ctx: &Ctx) {
        let game = ctx.game.as_deref();
        let in_game = game.is_some();
        let p = &*ctx.progress;
        let mut rows = Vec::new();
        match self.page {
            Page::Main => {
                rows.push(row(Item::Open(Page::Warp), "Warp to level", ">"));
                rows.push(row(Item::Open(Page::Progress), "Progress", ">"));
                rows.push(row(Item::Open(Page::Unlocks), "Unlocks and cheats", ">"));
                let mut r = row(Item::Open(Page::Party), "Party", ">");
                r.enabled = in_game;
                rows.push(r);
                let mut r = row(Item::Open(Page::Level), "Level", ">");
                r.enabled = in_game;
                rows.push(r);
                rows.push(row(Item::Open(Page::Bug), "File bug report", ">"));
                rows.push(row(Item::Open(Page::States), "Save / load state", ">"));
                rows.push(row(Item::Close, "Close", ""));
            }
            Page::Bug => {
                rows.push(row(Item::SaveBug, "Save bug report now (A)", ""));
                for (i, t) in crate::bugreport::TAGS.iter().enumerate() {
                    rows.push(row(
                        Item::BugTag(i),
                        format!("  Tag: {t}"),
                        if self.bug_tags[i] { "[x]" } else { "[ ]" },
                    ));
                }
                let info = |label: String| Row {
                    enabled: false,
                    ..row(Item::SaveBug, label, "")
                };
                rows.push(info("Pick tags with A or left/right, then save.".into()));
                rows.push(info("Saves the screen, nearby things, the last".into()));
                rows.push(info("2 minutes of events and the save, to:".into()));
                let dir = crate::bugreport::shown_dir();
                for chunk in dir.as_bytes().chunks(50) {
                    rows.push(info(format!("  {}", String::from_utf8_lossy(chunk))));
                }
                if let Some(last) = &self.last_bug {
                    let name = last.rsplit(['/', '\\']).next().unwrap_or(last);
                    rows.push(info(format!("Last: {name}")));
                }
            }
            Page::Warp => {
                let Some(realm) = self.realms.get(self.warp_realm) else {
                    rows.push(Row {
                        enabled: false,
                        ..row(Item::Close, "No level data", "")
                    });
                    self.rows = rows;
                    return;
                };
                rows.push(row(Item::WarpRealm, "Realm", format!("< {} >", realm.name)));
                for l in &realm.levels {
                    let tag = if l.boss { "  (boss)" } else { "" };
                    rows.push(row(
                        Item::Warp(l.wave),
                        format!("{}  {}{tag}", l.code, l.title),
                        "",
                    ));
                }
            }
            Page::Progress => {
                rows.push(row(Item::OpenAll, "Open everything", ""));
                let realms = self.prog_realms();
                if let Some(r) = realms.get(self.prog_realm) {
                    rows.push(row(Item::ProgRealm, "Realm", format!("< {} >", r.name)));
                    let n = r.levels.len();
                    let k = p.beaten[r.realm as usize & 15].count_ones();
                    rows.push(row(Item::Beaten, "  Levels beaten", format!("< {k}/{n} >")));
                }
                rows.push(row(
                    Item::BossRunes,
                    "Boss runes",
                    format!("< {}/11 >", (p.runes & ALL_BOSS_RUNES).count_ones()),
                ));
                rows.push(row(
                    Item::Shards,
                    "Runestones (shards)",
                    format!("< {}/13 >", (p.shards & ALL_SHARDS).count_ones()),
                ));
                let c = self.crystal;
                rows.push(row(
                    Item::CrystalColour,
                    "Crystal colour",
                    format!("< {c}: {} >", realm_name(CRYSTAL_REALM[c])),
                ));
                rows.push(row(
                    Item::Crystals,
                    "  Crystals",
                    format!("< {} >", count_text(p.crystals[c], CRYSTALS_NEEDED[c])),
                ));
                rows.push(row(
                    Item::Garg,
                    "Gargoyle fangs/feathers/claws",
                    format!(
                        "< {} >",
                        (0..3)
                            .map(|j| count_text(p.garg_items[j], GARG_NEEDED[j]))
                            .collect::<Vec<_>>()
                            .join(" ")
                    ),
                ));
                let all = legend_bits(&self.realms);
                rows.push(row(
                    Item::Legend,
                    "Legend weapons",
                    format!("< {}/{} >", (p.legend & all).count_ones(), all.count_ones()),
                ));
                rows.push(row(Item::ResetProgress, "Reset progress", ""));
            }
            Page::Unlocks => {
                rows.push(row(Item::UnlockAll, "Unlock every secret hero", ""));
                rows.push(row(Item::LockAll, "Lock every secret hero", ""));
                for &c in &SECRET_CLASS {
                    let on = self.unlocked(ctx, c);
                    rows.push(row(
                        Item::Class(c),
                        format!("  {}", class_name(c)),
                        if on { "unlocked" } else { "locked" },
                    ));
                }
                let names = cheat_names();
                let mut r = row(
                    Item::Cheat,
                    "Name cheat (A: give to party)",
                    format!("< {} >", names[self.cheat.min(names.len() - 1)]),
                );
                r.enabled = in_game;
                rows.push(r);
            }
            Page::Party => {
                let Some(g) = game else {
                    self.page = Page::Main;
                    return self.refresh(ctx);
                };
                let t = if self.target == 0 || self.target > g.players.len() {
                    "All".to_string()
                } else {
                    let pi = self.target - 1;
                    format!("P{} {}", pi + 1, g.players[pi].class)
                };
                rows.push(row(Item::Target, "Player", format!("< {t} >")));
                let first = targets(self.target, g).first().map(|&i| &g.players[i]);
                let show = |f: &dyn Fn(&crate::game::Player) -> String| {
                    first.map_or(String::new(), |p| format!("< {} >", f(p)))
                };
                rows.push(row(
                    Item::Level,
                    "Hero level",
                    show(&|p| p.level.to_string()),
                ));
                rows.push(row(
                    Item::Exp,
                    "Experience (+-1000)",
                    show(&|p| p.exp.to_string()),
                ));
                rows.push(row(
                    Item::Gold,
                    "Gold (+-1000)",
                    show(&|p| p.gold.to_string()),
                ));
                rows.push(row(Item::Keys, "Keys", show(&|p| p.keys.to_string())));
                rows.push(row(
                    Item::Potions,
                    "Potions",
                    show(&|p| p.potions.to_string()),
                ));
                rows.push(row(Item::FullHealth, "Full health and turbo", ""));
                rows.push(row(
                    Item::Powerup,
                    "Give power-up",
                    format!("< {} >", powerup_name(self.pup)),
                ));
                rows.push(row(Item::ClearPowerups, "Clear power-ups", ""));
                rows.push(row(Item::God, "God mode (no damage)", on_off(self.god)));
                rows.push(row(
                    Item::HeroClass,
                    "Class (A: reload level)",
                    format!("< {} >", class_name(self.hero_class)),
                ));
                let skin = match self.skin.checked_sub(1).and_then(|h| HIDDEN.get(h)) {
                    Some(h) => format!("< {} {} >", h.name, h.dir),
                    None => "< none >".into(),
                };
                rows.push(row(Item::Skin, "Hidden skin (A: reload)", skin));
            }
            Page::Level => {
                let Some(g) = game else {
                    self.page = Page::Main;
                    return self.refresh(ctx);
                };
                let tower = g.wave == Some(TOWER_WAVE);
                rows.push(row(Item::KillMonsters, "Kill every monster", ""));
                rows.push(row(Item::DestroyGenerators, "Destroy every generator", ""));
                let mut r = row(Item::KillBoss, "Kill the boss", "");
                r.enabled = g.boss.is_some();
                rows.push(r);
                let mut r = row(Item::FinishLevel, "Finish level (exit)", "");
                r.enabled = !tower;
                rows.push(r);
                rows.push(row(Item::XRay, "X-ray", on_off(xray_on(g))));
                rows.push(row(Item::Compass, "Compass", on_off(*ctx.compass)));
            }
            Page::States => {
                rows.push(row(
                    Item::StateSlot,
                    "Slot",
                    format!("< {} >", self.slot_name()),
                ));
                let mut r = row(Item::SaveState, "Save state", "");
                r.enabled = in_game;
                rows.push(r);
                rows.push(row(Item::LoadState, "Load state", ""));
            }
        }
        self.rows = rows;
    }

    /// Draw on the 512x384 screen.
    pub fn draw(&self, s: &mut Scr) {
        use crate::ui::FONT_8X8;
        if !self.open {
            return;
        }
        let (x0, y0, w, h) = (36.0, 16.0, 440.0, 272.0);
        s.rect(x0, y0, w, h, [0.02, 0.02, 0.06, 0.88]);
        s.rect(x0, y0, w, 1.0, [1.0, 0.3, 0.3, 1.0]);
        s.rect(x0, y0 + h - 1.0, w, 1.0, [1.0, 0.3, 0.3, 1.0]);
        s.text(
            1.0,
            x0 + 8.0,
            y0 + 8.0,
            FONT_8X8,
            0xFF6060,
            0,
            self.page.title(),
        );
        let tag = "not in the original";
        let tw = s.text_width(FONT_8X8, 1.0, tag);
        s.text(1.0, x0 + w - 8.0 - tw, y0 + 8.0, FONT_8X8, 0x808080, 0, tag);
        let line = 12.0;
        let top = y0 + 26.0;
        for (i, r) in self.rows.iter().enumerate().skip(self.scroll).take(VISIBLE) {
            let y = top + (i - self.scroll) as f32 * line;
            let selected = i == self.sel;
            if selected {
                s.rect(x0 + 4.0, y - 2.0, w - 8.0, line, [0.35, 0.25, 0.05, 0.9]);
            }
            let colour = match (r.enabled, selected) {
                (false, _) => 0x707070,
                (true, true) => 0xFFFF60,
                (true, false) => 0xE0E0E0,
            };
            s.text(1.0, x0 + 10.0, y, FONT_8X8, colour, 0, &r.label);
            if !r.value.is_empty() {
                let vw = s.text_width(FONT_8X8, 1.0, &r.value);
                s.text(1.0, x0 + w - 10.0 - vw, y, FONT_8X8, colour, 0, &r.value);
            }
        }
        if self.rows.len() > VISIBLE {
            let more = format!(
                "{}-{} of {}",
                self.scroll + 1,
                (self.scroll + VISIBLE).min(self.rows.len()),
                self.rows.len()
            );
            let mw = s.text_width(FONT_8X8, 1.0, &more);
            s.text(
                1.0,
                x0 + w - 10.0 - mw,
                y0 + h - 38.0,
                FONT_8X8,
                0x808080,
                0,
                &more,
            );
        }
        if let Some((m, _)) = &self.status {
            s.text(1.0, x0 + 10.0, y0 + h - 36.0, FONT_8X8, 0x80FF80, 0, m);
        }
        let keys = "Keys: Arrows  Enter apply  Backspace back  Esc close";
        let pad = "Pad:  D-pad   A apply      B back";
        s.text(0.9, x0 + 10.0, y0 + h - 22.0, FONT_8X8, 0xA0A0A0, 0, keys);
        s.text(0.9, x0 + 10.0, y0 + h - 11.0, FONT_8X8, 0xA0A0A0, 0, pad);
    }

    /// A reminder while god mode is on and the menu is shut.
    pub fn draw_overlay(&self, s: &mut Scr) {
        if self.god && !self.open {
            s.text(1.0, 4.0, 4.0, crate::ui::FONT_8X8, 0xFF6060, 0, "GOD");
        }
    }
}

fn class_name(c: usize) -> &'static str {
    match c {
        crate::stats::SUMNER => "SUMNER",
        _ => crate::select::HEROES.get(c).map_or("?", |h| h.1),
    }
}

fn on_off(b: bool) -> &'static str {
    if b {
        "on"
    } else {
        "off"
    }
}

fn count_text(n: i32, needed: i32) -> String {
    if n < 0 {
        "done".into()
    } else {
        format!("{n}/{needed}")
    }
}

/// Move `i` by `d` through `n` choices, wrapping.
fn step(i: usize, d: i32, n: usize) -> usize {
    if n == 0 {
        return 0;
    }
    ((i as i32 + d).rem_euclid(n as i32)) as usize
}

/// A count that ends at -1 once done: left/right move through 0..needed by
/// `unit`, then "done"; A jumps to done.
fn count_step(n: i32, d: i32, needed: i32, unit: i32) -> i32 {
    match d {
        0 => -1,
        _ if n < 0 => {
            if d < 0 {
                needed
            } else {
                -1
            }
        }
        _ => {
            let v = n + d * unit;
            if v > needed {
                -1
            } else {
                v.clamp(0, needed)
            }
        }
    }
}

/// Crystals step by 5 (the colours need 15 to 250).
fn crystal_step(n: i32, d: i32, c: usize) -> i32 {
    let needed = CRYSTALS_NEEDED[c];
    if n >= 0 && d != 0 {
        // Land on the exact count, whatever the steps.
        let v = n + d * 5;
        if d > 0 && n < needed && v > needed {
            return needed;
        }
    }
    count_step(n, d, needed, 5)
}

/// The players a party row acts on: everyone (0) or one (1-based).
fn targets(target: usize, g: &Game) -> Vec<usize> {
    match target {
        0 => (0..g.players.len()).collect(),
        t if t <= g.players.len() => vec![t - 1],
        _ => (0..g.players.len()).collect(),
    }
}

fn set_level(p: &mut crate::game::Player, level: u32) {
    p.level = level.clamp(1, 99);
    p.exp = crate::stats::level_to_exp(p.level);
    p.refresh_stats();
    p.health = max_health(p.level);
}

/// Change a numeric party value by one step.
fn party_value(p: &mut crate::game::Player, item: Item, d: i32) {
    match item {
        Item::Level => {
            let lv = if d == 0 {
                99
            } else {
                (p.level as i32 + d).clamp(1, 99)
            };
            set_level(p, lv as u32);
        }
        Item::Exp => {
            let exp =
                (p.exp as i64 + 1000 * d as i64).clamp(0, crate::stats::level_to_exp(99) as i64);
            p.exp = exp as u32;
            let lv = crate::stats::exp_to_level(p.exp);
            if lv != p.level {
                p.level = lv;
                p.refresh_stats();
                p.health = p.health.min(max_health(lv));
            }
        }
        Item::Gold => {
            p.gold = if d == 0 {
                MAX_GOLD.min(p.gold + 10_000)
            } else {
                (p.gold as i64 + 1000 * d as i64).clamp(0, MAX_GOLD as i64) as u32
            };
        }
        Item::Keys => {
            p.keys = if d == 0 {
                9
            } else {
                (p.keys as i32 + d).clamp(0, 9) as u32
            };
        }
        Item::Potions => {
            let n = if d == 0 {
                9
            } else {
                (p.potions as i32 + d).clamp(0, 9) as usize
            };
            p.potion_colours.resize(n, 0);
            p.potions = n as u32;
        }
        _ => {}
    }
}

/// PlayerAddPowerup with the class's power-up time, as a pickup does.
fn give_powerup(p: &mut crate::game::Player, i: usize) {
    let (kind, flags, amount, secs) = powerup(i);
    let t = if secs > 0.0 {
        secs * crate::stats::powerup_time(p.class_id)
    } else {
        secs
    };
    p.powerups.add(kind, flags, amount, t);
}

fn xray_on(g: &Game) -> bool {
    g.players.iter().any(|p| {
        p.powerups
            .slots
            .iter()
            .any(|s| s.time != 0.0 && s.kind == pu::SPECIAL && s.flags == pu::S_XRAY)
    })
}

/// Every hero holds a switched-on X-ray (or none does).
fn set_xray(g: &mut Game, on: bool) {
    for p in &mut g.players {
        if on {
            p.powerups.add(pu::SPECIAL, pu::S_XRAY, 0.0, 600.0);
            for s in p.powerups.slots.iter_mut() {
                if s.kind == pu::SPECIAL && s.flags == pu::S_XRAY {
                    s.state = pu::ON;
                }
            }
        } else {
            p.powerups.remove(pu::SPECIAL, pu::S_XRAY);
        }
    }
}

/// The first hero in play, for the kills' experience.
fn killer(g: &Game) -> usize {
    (0..g.players.len())
        .find(|&i| g.players[i].state.active())
        .unwrap_or(0)
}

/// A fatal hit on every live monster (not IT, which can't be hurt; Death
/// leaves after enough hits).
pub fn kill_monsters(g: &mut Game) -> usize {
    let pi = killer(g);
    let mut n = 0;
    for t in 0..g.entities.len() {
        let live = g.entities[t].alive
            && matches!(
                g.entities[t].kind,
                Kind::Enemy { state, .. } if !matches!(state, crate::game::EnemyState::Dying { .. })
            );
        if live {
            for _ in 0..32 {
                g.hit_entity(pi, t, 1.0e6, glam::Vec2::ZERO, false);
                let gone = !g.entities[t].alive
                    || matches!(
                        g.entities[t].kind,
                        Kind::Enemy {
                            state: crate::game::EnemyState::Dying { .. },
                            ..
                        }
                    );
                if gone {
                    n += 1;
                    break;
                }
                if !matches!(g.entities[t].kind, Kind::Enemy { etype, .. } if etype == crate::enemies::DEATH)
                {
                    break;
                }
            }
        }
    }
    n
}

/// A fatal hit on every generator.
pub fn destroy_generators(g: &mut Game) -> usize {
    let pi = killer(g);
    let mut n = 0;
    for t in 0..g.entities.len() {
        if g.entities[t].alive && matches!(g.entities[t].kind, Kind::Generator { .. }) {
            g.hit_entity(pi, t, 1.0e6, glam::Vec2::ZERO, false);
            if !g.entities[t].alive {
                n += 1;
            }
        }
    }
    n
}

/// End the level as if everyone left through its exit (a secret level: as
/// if its time ran out). False in the tower.
pub fn finish_level(g: &mut Game) -> bool {
    match g.wave {
        Some(TOWER_WAVE) => false,
        Some(w) if w >> 8 == 12 => {
            g.level_end = Some(LevelEnd::SecretOver);
            true
        }
        _ => {
            g.level_end = Some(LevelEnd::Exit(None));
            true
        }
    }
}

/// The level's copies of the party's progress, brought into it (what the
/// app does at a level's end).
pub fn pull(p: &mut Progress, g: &Game) {
    p.crystals = g.crystals;
    p.garg_items = g.garg_items;
    p.shards = g.shards;
    p.legend = g.legend_held;
    p.runes |= g.runes | g.boss_runes;
}

/// The party's progress, given back to the running level.
pub fn push(p: &Progress, g: &mut Game) {
    g.crystals = p.crystals;
    g.garg_items = p.garg_items;
    g.shards = p.shards;
    g.runes = p.runes;
    g.boss_runes &= p.runes;
    g.set_legend_held(p.legend);
}

fn sync_party_progress(p: &mut Progress, g: &Game) {
    pull(p, g);
}

/// Before a warp: the level's progress into the party's.
fn sync_party(ctx: &mut Ctx) {
    if let Some(g) = ctx.game.as_deref() {
        sync_party_progress(ctx.progress, g);
    }
}

fn set_beaten(p: &mut Progress, realm: usize, k: u32) {
    p.beaten[realm] = ((1u32 << k.min(16)) - 1) as u16;
}

/// Boss runes for the first `k` realms in unlock order.
fn boss_runes(k: u32) -> u16 {
    (1..=k.min(11)).fold(0, |m, b| m | (1 << b))
}

/// Every legend weapon the levels hand out (bit = the item's value).
fn legend_bits(realms: &[RealmInfo]) -> u16 {
    realms
        .iter()
        .flat_map(|r| &r.levels)
        .filter(|l| (1..=15).contains(&l.legend))
        .fold(0, |m, l| m | (1 << l.legend))
}

/// Everything beaten and found: every level, boss rune, runestone, crystal
/// colour, gargoyle gate and legend weapon. Sumner is told about the runes
/// and shards already, so the tower does not play every speech at once.
pub fn open_all(p: &mut Progress, realms: &[RealmInfo]) {
    for r in realms.iter().filter(|r| r.realm != 12 && r.realm != 13) {
        set_beaten(p, r.realm as usize & 15, r.levels.len() as u32);
        for l in &r.levels {
            if (1..=13).contains(&l.rune) {
                p.level_masks[0] |= 1 << (l.rune - 1);
            }
            if (1..=15).contains(&l.legend) {
                p.level_masks[2] |= 1 << l.legend;
            }
        }
    }
    p.runes = ALL_BOSS_RUNES;
    p.shards = ALL_SHARDS;
    for c in 1..9 {
        p.crystals[c] = -1;
    }
    p.garg_items = [-1; 3];
    p.legend = legend_bits(realms);
    p.banked_runes = p.runes;
    p.banked_shards = p.shards;
    p.entered_tower = true;
    p.last_realm = Some(CRYSTAL_ORDER[1]);
}

/// Nothing done, as a new party (the hint pickers and the tower welcome are
/// kept, so the session goes on as it was).
pub fn reset_progress(p: &mut Progress) {
    *p = Progress {
        entered_tower: p.entered_tower,
        hints: p.hints,
        last_realm: p.last_realm,
        ..Default::default()
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn realms() -> Vec<RealmInfo> {
        let lv = |realm: u32, i: u32, rune: i16, legend: i16| LevelInfo {
            code: format!("{}{}", (b'A' + realm as u8 - 1) as char, i + 1),
            title: String::new(),
            wave: (realm << 8) | i,
            rune,
            legend,
            boss: false,
        };
        vec![
            RealmInfo {
                realm: 1,
                name: "Castle",
                levels: vec![lv(1, 0, 1, 0), lv(1, 1, 0, 0), lv(1, 2, 0, 1)],
            },
            RealmInfo {
                realm: 2,
                name: "Mountain",
                levels: vec![lv(2, 0, 0, 0), lv(2, 1, 2, 2)],
            },
            RealmInfo {
                realm: 13,
                name: "Tower",
                levels: vec![LevelInfo {
                    code: "L1".into(),
                    ..lv(13, 0, 0, 0)
                }],
            },
        ]
    }

    struct World {
        progress: Progress,
        heroes: [Option<HeroSave>; 4],
        compass: bool,
    }

    impl World {
        fn new() -> World {
            World {
                progress: Progress::default(),
                heroes: Default::default(),
                compass: false,
            }
        }
        fn ctx(&mut self) -> Ctx<'_> {
            Ctx {
                progress: &mut self.progress,
                game: None,
                panels: None,
                heroes: &mut self.heroes,
                compass: &mut self.compass,
            }
        }
    }

    fn press(dir: &str) -> PlayerInput {
        let mut i = PlayerInput::default();
        match dir {
            "up" => i.dpad.y = 1.0,
            "down" => i.dpad.y = -1.0,
            "left" => i.dpad.x = -1.0,
            "right" => i.dpad.x = 1.0,
            "a" => i.attack_pressed = true,
            "b" => i.b_pressed = true,
            _ => {}
        }
        i
    }

    /// Feed presses, each followed by a released tick.
    fn drive(m: &mut DebugMenu, w: &mut World, keys: &[&str]) -> Vec<Action> {
        let mut out = Vec::new();
        for k in keys {
            out.push(m.update(&[press(k)], &mut w.ctx()));
            out.push(m.update(&[PlayerInput::default()], &mut w.ctx()));
        }
        out
    }

    #[test]
    fn filing_a_bug_report_with_tags() {
        let mut m = DebugMenu::new(realms());
        let mut w = World::new();
        m.show(None, &mut w.ctx());
        m.update(&[PlayerInput::default()], &mut w.ctx());
        // Main: Warp, Progress, Unlocks, Party, Level, File bug report,
        // Save / load state, Close.
        assert_eq!(m.rows()[5].item, Item::Open(Page::Bug));
        drive(&mut m, &mut w, &["up", "up", "up", "a"]);
        assert_eq!(m.page(), Page::Bug);
        // Tag "stuck" with A, "wrong look" with right; save from the top row.
        drive(&mut m, &mut w, &["down", "a", "down", "down", "right"]);
        assert_eq!(m.picked_tags(), vec!["stuck", "wrong look"]);
        let acts = drive(&mut m, &mut w, &["up", "up", "up", "a"]);
        assert!(
            acts.contains(&Action::FileBug(vec!["stuck", "wrong look"])),
            "{acts:?}"
        );
        assert!(m.open, "the menu stays open");
        m.bug_filed(Ok("/x/bugs/20261023-120000-levelA1".into()));
        assert_eq!(
            m.status(),
            Some("Bug report saved: 20261023-120000-levelA1")
        );
        assert!(m.picked_tags().is_empty(), "tags cleared for the next one");
        m.bug_filed(Err("disk full".into()));
        assert_eq!(m.status(), Some("Bug report failed: disk full"));
    }

    #[test]
    fn save_state_page_picks_a_slot() {
        let mut m = DebugMenu::new(realms());
        let mut w = World::new();
        m.show(Some(Source::Keyboard), &mut w.ctx());
        let i = m
            .rows()
            .iter()
            .position(|r| r.item == Item::Open(Page::States))
            .unwrap();
        m.goto(Page::Main, i, &mut w.ctx());
        drive(&mut m, &mut w, &["a"]);
        assert_eq!(m.page(), Page::States);
        // From the title only loading is offered.
        assert!(!m.rows()[1].enabled && m.rows()[2].enabled);
        // Slot 1 -> 2 -> 3 -> autosave.
        let acts = drive(
            &mut m,
            &mut w,
            &["right", "right", "right", "down", "down", "a"],
        );
        assert!(
            acts.contains(&Action::LoadState("autosave".into())),
            "{acts:?}"
        );
    }

    #[test]
    fn the_combo_needs_l_r_and_z_held() {
        let mut i = PlayerInput {
            start_pressed: true,
            ..Default::default()
        };
        assert!(!open_combo(&i), "Start alone is the pause menu");
        i.charge = true;
        i.strafe = true;
        assert!(!open_combo(&i));
        i.combo = true;
        assert!(open_combo(&i));
        i.start_pressed = false;
        assert!(!open_combo(&i));
    }

    #[test]
    fn open_all_opens_every_portal() {
        let mut p = Progress::default();
        open_all(&mut p, &realms());
        assert_eq!(p.beaten[1], 0b111);
        assert_eq!(p.beaten[2], 0b11);
        assert_eq!(p.beaten[13], 0, "the tower is not a level to beat");
        for w in [0x100, 0x102, 0x101, 0x500, 0x600, 0x800, 0x803] {
            assert!(p.portal_open(w), "{w:#x}");
        }
        assert_eq!(p.legend, (1 << 1) | (1 << 2));
        assert_eq!(p.level_masks[0], 0b11);
        assert!(p.crystals[1..].iter().all(|&c| c == -1));
        assert_eq!(p.garg_items, [-1; 3]);
        assert_eq!(p.banked_runes, p.runes, "Sumner already told");
        reset_progress(&mut p);
        assert_eq!(p.beaten, [0; 16]);
        assert_eq!((p.runes, p.shards, p.legend), (0, 0, 0));
        assert!(p.entered_tower, "the session's welcome is kept");
    }

    #[test]
    fn counts_step_and_end_done() {
        assert_eq!(count_step(0, 1, 12, 1), 1);
        assert_eq!(count_step(0, -1, 12, 1), 0);
        assert_eq!(count_step(12, 1, 12, 1), -1, "past the end: done");
        assert_eq!(count_step(-1, -1, 12, 1), 12);
        assert_eq!(count_step(-1, 1, 12, 1), -1);
        assert_eq!(count_step(3, 0, 12, 1), -1, "A: done");
        assert_eq!(crystal_step(10, 1, 1), 15, "lands on the 15 needed");
        assert_eq!(crystal_step(15, 1, 1), -1);
        assert_eq!(crystal_step(0, 1, 2), 5);
        assert_eq!(boss_runes(0), 0);
        assert_eq!(boss_runes(8), 0x1FE, "the temple's eight");
        assert_eq!(boss_runes(11), ALL_BOSS_RUNES);
        assert_eq!(step(0, -1, 3), 2);
        assert_eq!(step(2, 1, 3), 0);
    }

    #[test]
    fn navigating_the_progress_page() {
        let mut m = DebugMenu::new(realms());
        let mut w = World::new();
        m.show(None, &mut w.ctx());
        // A tick with nothing held: directions count from then on.
        m.update(&[PlayerInput::default()], &mut w.ctx());
        assert_eq!(m.page(), Page::Main);
        // Main: Warp, Progress, ... ; Party and Level greyed out on the title.
        assert!(!m.rows()[3].enabled);
        drive(&mut m, &mut w, &["down", "a"]);
        assert_eq!(m.page(), Page::Progress);
        // Realm (row 1) -> Mountain, then Levels beaten +1, +1, +1 (capped).
        drive(
            &mut m,
            &mut w,
            &["down", "right", "down", "right", "right", "right"],
        );
        assert_eq!(w.progress.beaten[2], 0b11);
        assert_eq!(w.progress.beaten[1], 0);
        drive(&mut m, &mut w, &["left"]);
        assert_eq!(w.progress.beaten[2], 0b1);
        // Boss runes: A gives all 11; left takes one away.
        drive(&mut m, &mut w, &["down", "a", "left"]);
        assert_eq!(w.progress.runes, boss_runes(10));
        // Runestones +1.
        drive(&mut m, &mut w, &["down", "right"]);
        assert_eq!(w.progress.shards, 1);
        // Progress set on the title is what the next new game starts with.
        assert_eq!(m.new_game_progress().shards, 1);
        assert_eq!(m.new_game_progress().shards, 0, "only once");
        // B: back to the main page; B again closes.
        let acts = drive(&mut m, &mut w, &["b", "b"]);
        assert!(acts.contains(&Action::Close));
        assert!(!m.open);
    }

    #[test]
    fn held_directions_repeat_and_the_opening_press_is_ignored() {
        let mut m = DebugMenu::new(realms());
        let mut w = World::new();
        m.show(None, &mut w.ctx());
        // Held from before the menu opened: nothing until it is let go.
        for _ in 0..30 {
            m.update(&[press("down")], &mut w.ctx());
        }
        assert_eq!(m.sel, 0);
        m.update(&[PlayerInput::default()], &mut w.ctx());
        // Down held: one step, then repeats after the delay.
        for _ in 0..(REPEAT_DELAY + REPEAT_RATE) {
            m.update(&[press("down")], &mut w.ctx());
        }
        assert_eq!(m.sel, 2);
    }

    #[test]
    fn unlocks_reach_the_heroes_and_the_session() {
        let mut m = DebugMenu::new(realms());
        let mut w = World::new();
        w.heroes[1] = Some(HeroSave::new("BOB", 0, 0));
        m.show(None, &mut w.ctx());
        m.page = Page::Unlocks;
        m.apply(Item::UnlockAll, 0, &mut w.ctx());
        assert_eq!(m.class_unlock, 0x1FF, "the eight classes and Sumner");
        assert_eq!(w.heroes[1].as_ref().unwrap().class_unlock, 0x1FF);
        // Toggle the Jackal (class 10) off.
        m.apply(Item::Class(10), 0, &mut w.ctx());
        assert_eq!(m.class_unlock, 0x1FF & !(1 << 2));
        assert_eq!(
            w.heroes[1].as_ref().unwrap().class_unlock,
            0x1FF & !(1 << 2)
        );
        m.apply(Item::LockAll, 0, &mut w.ctx());
        assert_eq!(m.class_unlock, 0);
    }

    #[test]
    fn warp_lists_levels_by_realm() {
        let mut m = DebugMenu::new(realms());
        let mut w = World::new();
        m.show(None, &mut w.ctx());
        m.apply(Item::Open(Page::Warp), 0, &mut w.ctx());
        m.refresh(&w.ctx());
        assert_eq!(m.rows().len(), 4, "realm row and Castle's three");
        m.apply(Item::WarpRealm, -1, &mut w.ctx());
        m.refresh(&w.ctx());
        assert_eq!(m.rows()[1].item, Item::Warp(0xD00));
        assert_eq!(
            m.apply(Item::Warp(0xD00), 0, &mut w.ctx()),
            Action::Warp("levelL1".into())
        );
        assert_eq!(m.apply(Item::Warp(0x101), 1, &mut w.ctx()), Action::None);
        assert_eq!(
            m.apply(Item::Warp(0x101), 0, &mut w.ctx()),
            Action::Warp("levelA2".into())
        );
    }

    #[test]
    fn crystals_and_gargoyle_items_stay_in_range() {
        let mut m = DebugMenu::new(realms());
        let mut w = World::new();
        m.show(None, &mut w.ctx());
        for _ in 0..10 {
            m.apply(Item::Crystals, 1, &mut w.ctx());
        }
        assert_eq!(w.progress.crystals[1], -1, "colour 1: 15 needed, then done");
        m.apply(Item::CrystalColour, 1, &mut w.ctx());
        m.apply(Item::Crystals, 1, &mut w.ctx());
        assert_eq!(w.progress.crystals[2], 5);
        m.apply(Item::Garg, 0, &mut w.ctx());
        assert_eq!(w.progress.garg_items, [-1; 3]);
        m.apply(Item::Garg, -1, &mut w.ctx());
        assert_eq!(w.progress.garg_items, GARG_NEEDED);
        // What a save keeps of it round-trips.
        let cp = crate::save::ClassProgress::from_progress(&w.progress);
        let mut back = Progress::default();
        cp.merge_into(&mut back);
        assert_eq!(back.crystals, w.progress.crystals);
    }

    fn data() -> Option<crate::fs::GameFs> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game-data");
        crate::fs::GameFs::open(&root).ok()
    }

    /// The party page and the level page on a real level.
    #[test]
    fn party_and_level_actions() {
        let Some(fs) = data() else { return };
        let mut g = Game::new(&fs, "levelA1", &[("WAR", "RED"), ("VAL", "BLU")]).unwrap();
        let realms = realm_table(&g.worlds);
        assert!(realms.iter().any(|r| r.realm == 12 && r.levels.len() == 9));
        assert!(realms.iter().any(|r| r.realm == 13));
        let mut m = DebugMenu::new(realms);
        let mut w = World::new();
        let mut panels = vec![
            PanelInfo {
                slot: 0,
                name: "A".into(),
                colour: 2,
                class: 0,
                hidden: None,
            },
            PanelInfo {
                slot: 1,
                name: "B".into(),
                colour: 1,
                class: 1,
                hidden: None,
            },
        ];
        let mut ctx = Ctx {
            progress: &mut w.progress,
            game: Some(&mut g),
            panels: Some(&mut panels),
            heroes: &mut w.heroes,
            compass: &mut w.compass,
        };
        m.show(None, &mut ctx);
        // Target player 2 only.
        m.apply(Item::Target, 1, &mut ctx);
        m.apply(Item::Target, 1, &mut ctx);
        m.apply(Item::Level, 1, &mut ctx);
        m.apply(Item::Keys, 0, &mut ctx);
        m.apply(Item::Potions, 1, &mut ctx);
        m.apply(Item::Gold, -1, &mut ctx);
        {
            let g = ctx.game.as_deref().unwrap();
            assert_eq!(g.players[0].level, 1, "player 1 untouched");
            assert_eq!(g.players[1].level, 2);
            assert_eq!(g.players[1].exp, crate::stats::level_to_exp(2));
            assert_eq!(g.players[1].health, max_health(2));
            assert_eq!(g.players[1].keys, 9);
            assert_eq!(
                g.players[1].potion_colours.len(),
                g.players[1].potions as usize
            );
            assert_eq!(g.players[1].gold, 0, "never below zero");
        }
        // Everyone gets a 3-way shot.
        m.apply(Item::Target, -1, &mut ctx);
        m.apply(Item::Target, -1, &mut ctx);
        m.apply(Item::Powerup, 0, &mut ctx);
        for p in &ctx.game.as_deref().unwrap().players {
            assert!(p
                .powerups
                .slots
                .iter()
                .any(|s| s.flags == pu::W_3WAY && s.time > 0.0));
        }
        // God mode reaches the level.
        m.apply(Item::God, 0, &mut ctx);
        assert!(ctx.game.as_deref().unwrap().debug_god);
        {
            let g = ctx.game.as_deref_mut().unwrap();
            let hp = g.players[0].health;
            g.damage_player(0, 50.0, 0);
            assert_eq!(g.players[0].health, hp, "no damage in god mode");
        }
        // X-ray on and off.
        m.apply(Item::XRay, 0, &mut ctx);
        assert!(xray_on(ctx.game.as_deref().unwrap()));
        m.apply(Item::XRay, 0, &mut ctx);
        assert!(!xray_on(ctx.game.as_deref().unwrap()));
        // Monsters and generators.
        let gens = |g: &Game| {
            g.entities
                .iter()
                .filter(|e| e.alive && matches!(e.kind, Kind::Generator { .. }))
                .count()
        };
        let before = gens(ctx.game.as_deref().unwrap());
        assert!(before > 0, "A1 has generators");
        m.apply(Item::DestroyGenerators, 0, &mut ctx);
        assert_eq!(gens(ctx.game.as_deref().unwrap()), 0);
        m.apply(Item::KillMonsters, 0, &mut ctx);
        assert!(ctx
            .game
            .as_deref()
            .unwrap()
            .entities
            .iter()
            .all(|e| !e.alive
                || !matches!(e.kind, Kind::Enemy { state, etype, .. }
                if !matches!(state, crate::game::EnemyState::Dying { .. })
                    && etype != crate::enemies::IT
                    && etype != crate::enemies::DEATH)));
        // Progress edits reach the level's copies.
        m.apply(Item::Shards, 0, &mut ctx);
        assert_eq!(ctx.game.as_deref().unwrap().shards, ALL_SHARDS);
        // A skin change reloads the level with the new panels.
        m.apply(Item::Skin, 1, &mut ctx);
        let a = m.apply(Item::Skin, 0, &mut ctx);
        assert_eq!(a, Action::Warp("levelA1".into()));
        assert_eq!(panels[0].hidden, Some(0));
        assert_eq!(panels[0].class, HIDDEN[0].class);
    }

    #[test]
    fn finishing_a_level() {
        let Some(fs) = data() else { return };
        let mut g = Game::new(&fs, "levelA1", &[("WAR", "RED")]).unwrap();
        assert!(finish_level(&mut g));
        assert_eq!(g.level_end, Some(LevelEnd::Exit(None)));
        let mut g = Game::new(&fs, "levelS1", &[("WAR", "RED")]).unwrap();
        assert!(finish_level(&mut g));
        assert_eq!(g.level_end, Some(LevelEnd::SecretOver));
    }
}
