//! The options / pause menu stack (options.c: DoOptions, do_optmenu, show_optmenu,
//! start_optmenu, end_optmenu): the title screen's "Options" tree, the in-game
//! "Game Menu" and the tower's "Tower Menu", drawn and run as the original does.
//!
//! The menu definitions are the OPTMENU structs in main.dol (0x8011DD20 + offset);
//! the addresses are noted on each entry of [`MENUS`].

use crate::input::PlayerInput;
use crate::optmenu::Repeat;
use crate::screen::{glow_alpha, Scr};
use crate::ui::{Tex, FONT_32};

/// OPTMENU_FINISH_FRAMES: the fade-in and the fade-out on close, in ticks.
const FINISH_FRAMES: i32 = 30;
/// OPTMENU_MARGIN_TITLE / OPTMENU_MARGIN_PLAYER: title and "Player N" below the backdrop top.
const MARGIN_TITLE: f32 = 58.0;
const MARGIN_PLAYER: f32 = 34.0;
/// msg_scale and optplyr_scale.
const MSG_SCALE: f32 = 0.667;
const PLAYER_SCALE: f32 = 0.667;
/// The colour trios at 0x8011DDD4 (off, on, hi) and 0x8011DDF8 (title text).
const RGB_OFF: u32 = 0x5C1A03;
const RGB_ON: u32 = 0xFFFFFF;
const RGB_HI: u32 = 0x8200EA;
const RGB_TEXT: u32 = 0xFFFFFF;
/// The audio sliders: slider_dxl, slider_dxr, OPTAUDIO_VOL_WIDTH.
const SLIDER_DXL: f32 = -52.0;
const SLIDER_DXR: f32 = -24.0;
const VOL_WIDTH: i32 = 264;
/// The burn overlay rect shared by every menu that has one.
const BURN_RECT: (f32, f32, f32, f32) = (290.0, 142.0, 224.0, 172.0);
/// Control-style names at 0x8011F048. `% 3` in DoOptions keeps "One Handed" out of reach.
pub const CONTROL_STYLES: [&str; 4] = ["Default", "Arcade", "Robotron", "One Handed"];
/// ctl_blits at 0x8011FAF8: the controller diagram, each at its natural size.
const CTL_BLITS: [(&str, f32, f32); 3] = [
    ("CONTROLER_1", 96.0, 104.0),
    ("CONTROLER_2", 352.0, 104.0),
    ("CONTROLER_3", 128.0, 232.0),
];

// OPTMENU flag bits.
const F_BACK: u32 = 0x1;
const F_SELECT: u32 = 0x2;
const F_CHANGE: u32 = 0x4;
const F_CENTER: u32 = 0x8;
const F_PLAYER: u32 = 0x10;
const F_FADE: u32 = 0x20;
const F_PARCH: u32 = 0x40;
const F_GARAMOND: u32 = 0x80;
const F_ACCEPT: u32 = 0x100;
const F_SELECT_SOUND: u32 = 0x400;

// do_optmenu / item codes (options.c OPT_*).
const OPT_ABORTALL: i32 = -2;
const OPT_ABORT: i32 = -1;
const OPT_NONE: i32 = 0;
const OPT_LEFT_EDGE: i32 = 3;
const OPT_RIGHT_EDGE: i32 = 4;
const OPT_LEFT_LEVEL: i32 = 5;
const OPT_RIGHT_LEVEL: i32 = 6;
const OPT_OPTIONS: i32 = 12;
const OPT_CHARSEL: i32 = 13;
const OPT_SHOP: i32 = 14;
const OPT_INVENTORY: i32 = 15;
const OPT_GAMEOPTIONS: i32 = 16;
const OPT_AUDIO: i32 = 17;
const OPT_DIFFICULTY: i32 = 18;
const OPT_MULTIPLAYER: i32 = 19;
const OPT_COMPASS: i32 = 20;
const OPT_CONTROLS: i32 = 21;
const OPT_MUSICVOL: i32 = 23;
const OPT_SFXVOL: i32 = 24;
const OPT_STEREO: i32 = 25;
const OPT_OFF: i32 = 26;
const OPT_ON: i32 = 27;
const OPT_EASY: i32 = 28;
const OPT_MEDIUM: i32 = 29;
const OPT_HARD: i32 = 30;
const OPT_SHOW: i32 = 31;
const OPT_HIDE: i32 = 32;
const OPT_CTLSCHEME: i32 = 33;
const OPT_RUMBLE: i32 = 34;
const OPT_AUTOAIM: i32 = 35;
const OPT_AUTOATTACK: i32 = 36;
const OPT_QUITGAME: i32 = 37;
const OPT_QUITLEVEL: i32 = 38;

/// The menus this module runs, named by their titles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuId {
    Options,
    Tower,
    TowerSettings,
    Game,
    GameSettings,
    GameOptions,
    AbortLevel,
    Audio,
    QuitGame,
    Difficulty,
    Multiplayer,
    Compass,
    Controls,
    ControlStyle,
    Rumble,
    AutoAim,
    AutoAttack,
}

const MENU_COUNT: usize = 17;

/// One OPTITEM: text, code, dy, text2.
type ItemDef = (&'static str, i32, i32, Option<&'static str>);

/// The fixed part of an OPTMENU.
#[derive(Debug)]
struct Def {
    id: MenuId,
    title: &'static str,
    x: i32,
    y: i32,
    flags: u32,
    scale: f32,
    /// Backdrop rect (bx, by, bw, bh); -1 = centred.
    b: (i32, i32, i32, i32),
    burn: bool,
    items: &'static [ItemDef],
}

const FULL: (i32, i32, i32, i32) = (-1, 8, 480, 360);
const BOX: (i32, i32, i32, i32) = (-1, 64, 320, 220);

#[allow(clippy::too_many_arguments)]
const fn def(
    id: MenuId,
    title: &'static str,
    x: i32,
    y: i32,
    flags: u32,
    scale: f32,
    b: (i32, i32, i32, i32),
    burn: bool,
    items: &'static [ItemDef],
) -> Def {
    Def {
        id,
        title,
        x,
        y,
        flags,
        scale,
        b,
        burn,
        items,
    }
}

/// Every menu, in [`MenuId`] order (dumped from main.dol).
static MENUS: [Def; MENU_COUNT] = [
    // 0x8011E058
    def(
        MenuId::Options,
        "Options",
        128,
        -1,
        0x4F3,
        1.0,
        FULL,
        true,
        &[
            ("Audio", OPT_AUDIO, 0, None),
            ("Game Options", OPT_GAMEOPTIONS, 0, None),
            ("Compass", OPT_COMPASS, 0, None),
            ("Controls", OPT_CONTROLS, 0, None),
        ],
    ),
    // 0x8011E218
    def(
        MenuId::Tower,
        "Tower Menu",
        128,
        -1,
        0x4F3,
        1.0,
        FULL,
        true,
        &[
            ("Settings", OPT_OPTIONS, 0, None),
            ("Manage Character", OPT_CHARSEL, 0, None),
            ("Shop", OPT_SHOP, 0, None),
            ("Inventory", OPT_INVENTORY, 0, None),
            ("Quit Game", OPT_QUITGAME, 0, None),
        ],
    ),
    // 0x8011E390
    def(
        MenuId::TowerSettings,
        "Settings",
        128,
        -1,
        0x4F3,
        1.0,
        FULL,
        true,
        &[
            ("Audio", OPT_AUDIO, 0, None),
            ("Compass", OPT_COMPASS, 0, None),
            ("Controls", OPT_CONTROLS, 0, None),
        ],
    ),
    // 0x8011E4E4
    def(
        MenuId::Game,
        "Game Menu",
        128,
        -1,
        0x4F3,
        1.0,
        FULL,
        true,
        &[
            ("Settings", OPT_OPTIONS, 0, None),
            ("Quit Level", OPT_QUITLEVEL, 0, None),
        ],
    ),
    // 0x8011E638
    def(
        MenuId::GameSettings,
        "Settings",
        128,
        -1,
        0x4F3,
        1.0,
        FULL,
        true,
        &[
            ("Audio", OPT_AUDIO, 0, None),
            ("Controls", OPT_CONTROLS, 0, None),
        ],
    ),
    // 0x8011E78C
    def(
        MenuId::GameOptions,
        "Game Options",
        128,
        -1,
        0x4F3,
        1.0,
        FULL,
        false,
        &[
            ("Difficulty", OPT_DIFFICULTY, 0, None),
            ("Multiplayer Mode", OPT_MULTIPLAYER, 0, None),
        ],
    ),
    // 0x8011E8E0
    def(
        MenuId::AbortLevel,
        "Abort Level?",
        -256,
        -1,
        0x60,
        1.0,
        BOX,
        false,
        &[("No", OPT_ABORT, 0, None), ("Yes", OPT_QUITLEVEL, 0, None)],
    ),
    // 0x8011E9C8
    def(
        MenuId::Audio,
        "Audio",
        128,
        108,
        0xF5,
        0.8,
        FULL,
        false,
        &[
            ("Music Volume", OPT_MUSICVOL, 52, None),
            ("Sfx Volume", OPT_SFXVOL, 52, None),
            ("Mono", OPT_STEREO, 0, Some("Stereo")),
        ],
    ),
    // 0x8011EB1C
    def(
        MenuId::QuitGame,
        "Quit Game?",
        -256,
        -1,
        0x60,
        1.0,
        BOX,
        false,
        &[("No", OPT_ABORT, 0, None), ("Yes", OPT_QUITGAME, 0, None)],
    ),
    // 0x8011EC94
    def(
        MenuId::Difficulty,
        "Difficulty",
        128,
        -1,
        0x4F3,
        1.0,
        FULL,
        false,
        &[
            ("Easy", OPT_EASY, 0, None),
            ("Medium", OPT_MEDIUM, 0, None),
            ("Hard", OPT_HARD, 0, None),
        ],
    ),
    // 0x8011EE0C
    def(
        MenuId::Multiplayer,
        "Multiplayer Mode",
        128,
        -1,
        0x4F3,
        0.667,
        FULL,
        false,
        &[
            ("Normal", OPT_EASY, 0, None),
            ("Shots Stun Other Players", OPT_MEDIUM, 0, None),
            ("Shots Hurt Other Players", OPT_HARD, 0, None),
        ],
    ),
    // 0x8011EF60
    def(
        MenuId::Compass,
        "Compass",
        128,
        -1,
        0x4F3,
        1.0,
        FULL,
        true,
        &[("Hide", OPT_HIDE, 0, None), ("Show", OPT_SHOW, 0, None)],
    ),
    // 0x8011F13C
    def(
        MenuId::Controls,
        "Controls",
        128,
        -1,
        0x4F3,
        1.0,
        FULL,
        false,
        &[
            ("Style ", OPT_CTLSCHEME, 0, None),
            ("Rumble Feature ", OPT_RUMBLE, 0, None),
            ("Auto Aim ", OPT_AUTOAIM, 0, None),
            ("Auto Attack ", OPT_AUTOATTACK, 0, None),
        ],
    ),
    // 0x8011F26C (icon_dx -48); the item's text is replaced by the style name.
    def(
        MenuId::ControlStyle,
        "Control Style",
        -256,
        265,
        0x1F6,
        1.0,
        FULL,
        false,
        &[("Default", OPT_CTLSCHEME, 0, None)],
    ),
    // 0x8011F3C0: "On" really has code 28 (OPT_EASY).
    def(
        MenuId::Rumble,
        "Rumble Feature",
        128,
        -1,
        0x4F3,
        1.0,
        FULL,
        true,
        &[("Off", OPT_OFF, 0, None), ("On", OPT_EASY, 0, None)],
    ),
    // 0x8011F514
    def(
        MenuId::AutoAim,
        "Auto Aim",
        128,
        -1,
        0x4F3,
        1.0,
        FULL,
        true,
        &[("On", OPT_ON, 0, None), ("Off", OPT_OFF, 0, None)],
    ),
    // 0x8011F668
    def(
        MenuId::AutoAttack,
        "Auto Attack",
        128,
        -1,
        0x4F3,
        1.0,
        FULL,
        true,
        &[("On", OPT_ON, 0, None), ("Off", OPT_OFF, 0, None)],
    ),
];

fn menu_def(id: MenuId) -> &'static Def {
    &MENUS[id as usize]
}

/// One player's control settings (PlayerControl[p]; controls.c defaults).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlayerPrefs {
    /// 0 Default, 1 Arcade, 2 Robotron (see [`CONTROL_STYLES`]).
    pub control_style: usize,
    pub rumble: bool,
    pub auto_aim: bool,
    pub auto_attack: bool,
}

impl Default for PlayerPrefs {
    fn default() -> Self {
        PlayerPrefs {
            control_style: 0,
            rumble: true,
            auto_aim: true,
            auto_attack: true,
        }
    }
}

/// The settings the menus edit (optglobals, init_prefs defaults) plus each
/// player's controls.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prefs {
    /// 0..255.
    pub music_vol: i32,
    /// 0..255.
    pub sfx_vol: i32,
    pub stereo: bool,
    /// optglobals.style: 0 Easy, 1 Medium, 2 Hard.
    pub difficulty: usize,
    /// optglobals.subtitles: 0 Normal, 1 Shots Stun, 2 Shots Hurt Other Players.
    pub multiplayer: usize,
    /// optglobals.vibration: show the compass.
    pub compass: bool,
    pub players: [PlayerPrefs; 4],
    /// Each OPTMENU's `sel`, which the original keeps in its static struct
    /// between openings.
    menu_sel: [usize; MENU_COUNT],
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            music_vol: 128,
            sfx_vol: 128,
            stereo: true,
            difficulty: 1,
            multiplayer: 0,
            compass: false,
            players: [PlayerPrefs::default(); 4],
            menu_sel: [0; MENU_COUNT],
        }
    }
}

/// What the menus want the caller to do after a tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OptionsAction {
    None,
    /// The menus have closed: the game may run again (in-game), or the title
    /// menu takes over (title). Keep drawing while [`Options::is_drawing`].
    Closed,
    /// "Abort Level?" Yes (opt_restart_request).
    QuitLevel,
    /// "Quit Game?" Yes (opt_quit_request).
    QuitGame,
    /// Tower "Manage Character": the menus are gone; open player select.
    ManageCharacter,
    /// Tower "Shop": the menus are gone; init_shop(1).
    Shop,
    /// Tower "Inventory": the menus are gone; init_shop(2).
    Inventory,
    /// Music / sfx volume or mono/stereo changed: apply [`Prefs`] to the audio.
    VolumeChanged,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Title,
    Game,
    Tower,
}

#[derive(Clone, Debug)]
struct ItemState {
    text: String,
    /// Radio check: `" ~"` follows part `on - 1`.
    on: i32,
    /// < 0 disabled; for Mono/Stereo, which part is highlighted.
    value: i32,
}

/// A live OPTMENU.
#[derive(Clone, Debug)]
struct Menu {
    def: &'static Def,
    items: Vec<ItemState>,
    sel: usize,
    /// Menu clock (m->time).
    time: i32,
    /// Fade-out counter; 0 when not finishing.
    finish: i32,
    /// Finished and removed (active = 0): no longer drawn.
    removed: bool,
    /// The backdrop blit (title_blit) exists: the backdrop and title are drawn.
    backdrop: bool,
    burn: bool,
    y: f32,
    b: (f32, f32, f32, f32),
    /// The arrow's glide (icon_y, icon_t): the line it is leaving, and ticks into
    /// the 15-tick move (0 when resting).
    icon_y: f32,
    icon_t: i32,
}

/// OPTMENU_ICON_TIME: ticks for the arrow to glide to a new item.
const ICON_TIME: i32 = 15;

impl Menu {
    /// Where the arrow's line is: half a line below the selected item's top.
    fn icon_target(&self) -> f32 {
        (line_height(self.def.scale) / 2) as f32
            + self.draw_y(self.sel.min(self.items.len().saturating_sub(1)))
    }

    /// One tick of the glide.
    fn glide(&mut self) {
        let ty = self.icon_target();
        if self.icon_t == 0 {
            if ty != self.icon_y {
                self.icon_t = 1;
            }
        } else if self.icon_t < ICON_TIME {
            self.icon_t += 1;
        } else {
            self.icon_y = ty;
            self.icon_t = 0;
        }
    }

    fn new(id: MenuId) -> Menu {
        let def = menu_def(id);
        let items = def
            .items
            .iter()
            .map(|&(t, ..)| ItemState {
                text: t.into(),
                on: 0,
                value: 0,
            })
            .collect();
        // start_optmenu_nostack's layout.
        let lh = line_height(def.scale);
        let h: i32 = def.items.iter().map(|i| lh + i.2).sum();
        let y = match def.y {
            -1 => 192 - h / 2,
            y if y < 0 => -(y + h / 2),
            y => y,
        };
        let (bx, by, bw, bh) = def.b;
        let bx = if bx < 0 { 256 - bw / 2 } else { bx };
        let by = if by < 0 { 192 - bh / 2 } else { by };
        let mut m = Menu {
            def,
            items,
            sel: 0,
            time: 0,
            finish: 0,
            removed: false,
            backdrop: true,
            burn: def.burn,
            y: y as f32,
            b: (bx as f32, by as f32, bw as f32, bh as f32),
            icon_y: 0.0,
            icon_t: 0,
        };
        m.restart();
        m
    }

    /// start_optmenu_nostack on a menu struct that already exists.
    fn restart(&mut self) {
        self.time = 0;
        self.finish = 0;
        self.removed = false;
        self.backdrop = true;
        self.burn = self.def.burn;
    }

    fn id(&self) -> MenuId {
        self.def.id
    }

    fn flags(&self) -> u32 {
        self.def.flags
    }

    /// The y each item was drawn at (it->draw_y).
    fn draw_y(&self, i: usize) -> f32 {
        let lh = line_height(self.def.scale);
        self.y + self.def.items[..i].iter().map(|it| lh + it.2).sum::<i32>() as f32
    }

    /// end_optmenu on this menu with mode <= 0: fade out if it fades, else go.
    fn end(&mut self) {
        self.backdrop = false;
        self.burn = false;
        if self.flags() & F_FADE != 0 {
            self.finish = 1;
        } else {
            self.removed = true;
        }
    }

    /// finish_optmenu's tick: true while still fading.
    fn tick_finish(&mut self) -> bool {
        if self.removed || self.finish == 0 {
            return false;
        }
        self.finish += 1;
        if self.finish > FINISH_FRAMES {
            self.finish = 0;
            self.removed = true;
            return false;
        }
        true
    }

    fn fading(&self) -> bool {
        !self.removed && self.finish != 0
    }
}

/// FontHeight(OPTMENU_FONT, scale) for FONT32.
fn line_height(scale: f32) -> i32 {
    (32.0 * scale) as i32
}

/// The backdrop burning away when the last menu closes (StartFireScroll /
/// ServeFireScroll): 21 frames of greenCircTrans, one per 2 ticks.
#[derive(Clone, Copy, Debug)]
struct FireScroll {
    rect: (f32, f32, f32, f32),
    ticks: i32,
}

const FIRE_FRAMES: i32 = 21;

/// The options menu stack for one player.
pub struct Options {
    kind: Kind,
    player: usize,
    stack: Vec<Menu>,
    /// The menu just popped off the top, still fading (stack[level + 1]).
    popped: Option<Menu>,
    scroll: Option<FireScroll>,
    /// optmenu_nochoice: START was pressed in a titled menu; every tick then
    /// returns ABORTALL until everything has closed.
    nochoice: bool,
    /// options_state != OPTMENU_INACTIVE.
    open: bool,
    /// The stack has emptied (OPTMENU_FINISH) and the root is fading out.
    finishing: bool,
    /// The root's remembered sel still has to be loaded from [`Prefs`].
    fresh: bool,
    quit_level_enabled: bool,
    control_style: usize,
    sfx_count: i32,
    sfx_alt: usize,
    music_val: i32,
    sfx_val: i32,
    rep: Repeat,
    /// Sound names to play, drained by the caller.
    pub sfx: Vec<String>,
}

impl Options {
    fn new(kind: Kind, player: usize, root: MenuId) -> Options {
        Options {
            kind,
            player: player.min(3),
            stack: vec![Menu::new(root)],
            popped: None,
            scroll: None,
            nochoice: false,
            open: true,
            finishing: false,
            fresh: true,
            quit_level_enabled: true,
            control_style: 0,
            sfx_count: 0,
            sfx_alt: 0,
            music_val: 0,
            sfx_val: 0,
            rep: Repeat::default(),
            // OptionsStart / the title menu's "Options": AudioCursorSelect.
            sfx: vec!["S_OPTMENUSEL".into()],
        }
    }

    /// The title screen's "Options" menu, owned by `player` (0-based).
    pub fn title(player: usize) -> Options {
        Options::new(Kind::Title, player, MenuId::Options)
    }

    /// The in-game "Game Menu" opened by `player`'s START.
    pub fn game(player: usize) -> Options {
        Options::new(Kind::Game, player, MenuId::Game)
    }

    /// The tower's "Tower Menu" opened by `player`'s START.
    pub fn tower(player: usize) -> Options {
        Options::new(Kind::Tower, player, MenuId::Tower)
    }

    /// OptionsStart greys out "Quit Level" (value -1) when sMusicTrackHi == 0xC.
    pub fn set_quit_level_enabled(&mut self, enabled: bool) {
        self.quit_level_enabled = enabled;
        for m in self.stack.iter_mut().filter(|m| m.id() == MenuId::Game) {
            for (it, d) in m.items.iter_mut().zip(m.def.items) {
                if d.1 == OPT_QUITLEVEL {
                    it.value = if enabled { 0 } else { -1 };
                }
            }
        }
    }

    /// The menus are up (options_state != OPTMENU_INACTIVE): the game is frozen.
    /// The player the menus belong to.
    pub fn player(&self) -> usize {
        self.player
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Something is still on screen: an open menu, a fade-out or the fire scroll.
    pub fn is_drawing(&self) -> bool {
        self.open || self.scroll.is_some() || self.popped.as_ref().is_some_and(Menu::fading)
    }

    /// The 3D arrow's place (show_optmenu's icon glide): virtual screen x and y,
    /// and its pitch, which turns half a circle per move and rests at pi on odd
    /// items. None while no menu is shown.
    pub fn cursor(&self, s: &Scr) -> Option<(f32, f32, f32)> {
        if !self.open || self.finishing {
            return None;
        }
        let m = self.stack.last()?;
        if m.finish != 0 || m.items.is_empty() {
            return None;
        }
        let base = if m.sel & 1 != 0 {
            std::f32::consts::PI
        } else {
            0.0
        };
        let target = m.icon_target();
        let (y, angle) = if m.icon_t == 0 {
            (m.icon_y, base)
        } else {
            let k = m.icon_t as f32 / ICON_TIME as f32;
            (
                m.icon_y + (target - m.icon_y) * k,
                base + std::f32::consts::PI * k,
            )
        };
        let mut x = m.def.x as f32;
        if x < 0.0 {
            let w = m
                .items
                .iter()
                .map(|it| s.text_width(crate::ui::FONT_32, m.def.scale, &it.text))
                .fold(0.0, f32::max);
            x = -(x + w / 2.0);
        }
        let dx = if m.id() == MenuId::ControlStyle {
            -48.0
        } else {
            -16.0
        };
        Some((x + dx, y, angle))
    }

    /// The top menu, if any.
    pub fn current(&self) -> Option<MenuId> {
        self.stack.last().map(Menu::id)
    }

    /// The top menu's selected item index.
    pub fn selected(&self) -> Option<usize> {
        self.stack.last().map(|m| m.sel)
    }

    /// A menu screen for snapshots, `ticks` after it opened: `options`,
    /// `gamemenu`, `towermenu`, or `options:<path>` / `gamemenu:<path>` where
    /// each path letter is a tick of input: `d`/`u` down/up, `l`/`r` left/right,
    /// `a`/`b`/`s` A/B/START, `.` idle.
    pub fn demo(screen: &str, ticks: u32) -> Option<Options> {
        let (root, path) = screen.split_once(':').unwrap_or((screen, ""));
        let mut o = match root {
            "options" => Options::title(0),
            "gamemenu" => Options::game(0),
            "towermenu" => Options::tower(0),
            _ => return None,
        };
        let mut prefs = Prefs::default();
        for c in path.chars() {
            let mut i = PlayerInput::default();
            match c {
                'd' => i.stick.y = -1.0,
                'u' => i.stick.y = 1.0,
                'l' => i.stick.x = -1.0,
                'r' => i.stick.x = 1.0,
                'a' => i.attack_pressed = true,
                'b' => i.b_pressed = true,
                's' => i.start_pressed = true,
                _ => {}
            }
            o.update(&i, &mut prefs);
            o.update(&PlayerInput::default(), &mut prefs);
        }
        for _ in 0..ticks {
            o.update(&PlayerInput::default(), &mut prefs);
        }
        o.sfx.clear();
        Some(o)
    }

    fn play(&mut self, name: &str) {
        self.sfx.push(name.into());
    }

    /// One 60 Hz tick of DoOptions for the owning player's input.
    pub fn update(&mut self, input: &PlayerInput, prefs: &mut Prefs) -> OptionsAction {
        if self.fresh {
            self.fresh = false;
            let root = &mut self.stack[0];
            root.sel = prefs.menu_sel[root.id() as usize];
            root.icon_y = root.icon_target();
            self.set_quit_level_enabled(self.quit_level_enabled);
        }
        if let Some(m) = self.stack.last_mut() {
            m.glide();
        }
        // ServeFireScroll: while it runs, menu input is cleared.
        let mut input = *input;
        if let Some(f) = &mut self.scroll {
            f.ticks += 1;
            if f.ticks >> 1 >= FIRE_FRAMES {
                self.scroll = None;
            } else {
                input = PlayerInput::default();
            }
        }
        if let Some(p) = &mut self.popped {
            if !p.tick_finish() {
                self.popped = None;
            }
        }
        if self.finishing {
            if self.popped.is_none() {
                self.finishing = false;
                if self.open {
                    self.open = false;
                    return OptionsAction::Closed;
                }
            }
            return OptionsAction::None;
        }
        if !self.open || self.stack.is_empty() {
            return OptionsAction::None;
        }
        // Covered levels finish fading.
        let n = self.stack.len();
        if n > 1 {
            self.stack[n - 2].tick_finish();
        }
        self.refresh(prefs);
        let mut choice = self.do_optmenu(&input);
        let mut action = OptionsAction::None;
        let mut skip_back_sound = false;
        let id = self.stack.last().unwrap().id();
        let sel = self.stack.last().unwrap().sel;
        let item_code = self.stack.last().unwrap().def.items[sel].1;
        let p = self.player;
        match id {
            MenuId::Tower => match choice {
                OPT_OPTIONS => self.push(MenuId::TowerSettings, prefs),
                OPT_CHARSEL => action = self.close_now(OptionsAction::ManageCharacter),
                OPT_SHOP => {
                    action = self.close_now(OptionsAction::Shop);
                    self.play("S_OPTMENUSEL");
                }
                OPT_INVENTORY => {
                    self.play("S_OPTMENUSEL");
                    action = self.close_now(OptionsAction::Inventory);
                }
                OPT_QUITGAME => {
                    self.push(MenuId::QuitGame, prefs);
                    self.top().sel = 0;
                }
                _ => {}
            },
            MenuId::Game => match choice {
                OPT_OPTIONS => self.push(MenuId::GameSettings, prefs),
                OPT_QUITLEVEL => {
                    self.push(MenuId::AbortLevel, prefs);
                    self.top().sel = 0;
                }
                _ => {}
            },
            MenuId::Options | MenuId::TowerSettings | MenuId::GameSettings => match choice {
                OPT_AUDIO => {
                    self.push(MenuId::Audio, prefs);
                    self.music_val = prefs.music_vol;
                    self.sfx_val = prefs.sfx_vol;
                    self.sfx_count = 0;
                }
                OPT_GAMEOPTIONS => self.push(MenuId::GameOptions, prefs),
                OPT_COMPASS => {
                    self.push(MenuId::Compass, prefs);
                    self.top().sel = prefs.compass as usize;
                }
                OPT_CONTROLS => self.push(MenuId::Controls, prefs),
                _ => {}
            },
            MenuId::GameOptions => match choice {
                OPT_DIFFICULTY => {
                    self.push(MenuId::Difficulty, prefs);
                    self.top().sel = prefs.difficulty;
                }
                OPT_MULTIPLAYER => {
                    self.push(MenuId::Multiplayer, prefs);
                    self.top().sel = prefs.multiplayer;
                }
                _ => {}
            },
            MenuId::QuitGame => {
                if choice == OPT_QUITGAME {
                    self.collapse();
                    action = OptionsAction::QuitGame;
                }
            }
            MenuId::AbortLevel => {
                if choice == OPT_QUITLEVEL {
                    self.collapse();
                    action = OptionsAction::QuitLevel;
                }
            }
            MenuId::Audio => {
                // do_audiomenu's test-sound timer.
                self.sfx_count += 1;
                match choice {
                    OPT_LEFT_EDGE | OPT_LEFT_LEVEL | OPT_RIGHT_EDGE | OPT_RIGHT_LEVEL => {
                        let step = if matches!(choice, OPT_LEFT_EDGE | OPT_LEFT_LEVEL) {
                            -1
                        } else {
                            1
                        };
                        match item_code {
                            OPT_SFXVOL => {
                                self.sfx_val = (self.sfx_val + step).clamp(0, 255);
                                if prefs.sfx_vol != self.sfx_val {
                                    prefs.sfx_vol = self.sfx_val;
                                    action = OptionsAction::VolumeChanged;
                                } else if self.sfx_count > 15 {
                                    self.play("S_VOLMOVE");
                                    self.sfx_count = 0;
                                }
                            }
                            OPT_MUSICVOL => {
                                self.music_val = (self.music_val + step).clamp(0, 255);
                                if prefs.music_vol != self.music_val {
                                    action = OptionsAction::VolumeChanged;
                                }
                                prefs.music_vol = self.music_val;
                            }
                            OPT_STEREO if matches!(choice, OPT_LEFT_EDGE | OPT_RIGHT_EDGE) => {
                                self.play("S_OPTMENUMOVHRZ");
                                prefs.stereo = !prefs.stereo;
                                action = OptionsAction::VolumeChanged;
                            }
                            _ => {}
                        }
                    }
                    OPT_MUSICVOL | OPT_SFXVOL | OPT_STEREO => {}
                    OPT_ABORT | OPT_ABORTALL => {
                        self.play("S_CHOOSESELSFX");
                        skip_back_sound = true;
                    }
                    _ => {
                        if item_code == OPT_SFXVOL && self.sfx_count > 60 {
                            // fn_8009EE2C(0): alternate test sounds.
                            let name = ["S_WARN", "S_PICKUPMAGIC"][self.sfx_alt];
                            self.play(name);
                            self.sfx_alt ^= 1;
                            self.sfx_count = 0;
                        }
                    }
                }
            }
            MenuId::Difficulty | MenuId::Multiplayer => {
                if matches!(choice, OPT_EASY | OPT_MEDIUM | OPT_HARD) {
                    if id == MenuId::Difficulty {
                        prefs.difficulty = sel;
                    } else {
                        prefs.multiplayer = sel;
                    }
                    self.play("S_CHOOSESELSFX");
                }
            }
            MenuId::Compass => match choice {
                OPT_SHOW | OPT_HIDE => {
                    prefs.compass = choice == OPT_SHOW;
                    self.play("S_CHOOSESELSFX");
                }
                _ => {}
            },
            MenuId::Controls => match choice {
                OPT_CTLSCHEME => {
                    self.control_style = prefs.players[p].control_style;
                    self.push(MenuId::ControlStyle, prefs);
                    self.play("S_CHOOSESELSFX");
                }
                OPT_RUMBLE => self.push(MenuId::Rumble, prefs),
                OPT_AUTOAIM => self.push(MenuId::AutoAim, prefs),
                OPT_AUTOATTACK => self.push(MenuId::AutoAttack, prefs),
                // OPT_ABORT: player_save_controls; the prefs are already live.
                _ => {}
            },
            MenuId::ControlStyle => match choice {
                OPT_LEFT_EDGE | OPT_RIGHT_EDGE => {
                    if item_code == OPT_CTLSCHEME {
                        let add = if choice == OPT_LEFT_EDGE { -2 } else { 0 }
                            + self.control_style as i32;
                        let mut style = add + 1;
                        if style < 0 {
                            style = add + 4;
                        }
                        self.control_style = (style % 3) as usize;
                        self.play("S_OPTMENUMOVHRZ");
                    }
                }
                // B does nothing here: only A (or START) leaves.
                OPT_ABORT => choice = OPT_NONE,
                OPT_ABORTALL | OPT_CTLSCHEME => {
                    prefs.players[p].control_style = self.control_style;
                    self.play("S_CHOOSESELSFX");
                    self.pop();
                }
                _ => {}
            },
            MenuId::Rumble => {
                if matches!(choice, OPT_OFF | OPT_EASY | OPT_MEDIUM | OPT_HARD) {
                    prefs.players[p].rumble = sel != 0;
                    self.play("S_CHOOSESELSFX");
                }
            }
            MenuId::AutoAim | MenuId::AutoAttack => {
                if matches!(choice, OPT_OFF | OPT_ON) {
                    if id == MenuId::AutoAim {
                        prefs.players[p].auto_aim = sel == 0;
                    } else {
                        prefs.players[p].auto_attack = sel == 0;
                    }
                    self.play("S_CHOOSESELSFX");
                }
            }
        }
        // Back out one level.
        if matches!(choice, OPT_ABORT | OPT_ABORTALL) && !self.stack.is_empty() {
            self.pop();
            if !skip_back_sound {
                self.play("S_OPTMENUEXIT");
            }
        }
        self.refresh(prefs);
        for m in &self.stack {
            prefs.menu_sel[m.id() as usize] = m.sel;
        }
        if action == OptionsAction::None && self.stack.is_empty() && self.kind == Kind::Title {
            // Back on the title menu (OPTMENU_TITLE) straight away.
            if self.open {
                self.open = false;
                self.finishing = false;
                return OptionsAction::Closed;
            }
        }
        action
    }

    fn top(&mut self) -> &mut Menu {
        self.stack.last_mut().unwrap()
    }

    /// The per-frame item state DoOptions writes: radio checks, Mono/Stereo,
    /// the control-style name.
    fn refresh(&mut self, prefs: &Prefs) {
        let p = prefs.players[self.player];
        let style = self.control_style;
        let Some(m) = self.stack.last_mut() else {
            return;
        };
        let radio = match m.id() {
            MenuId::Difficulty => Some(prefs.difficulty),
            MenuId::Multiplayer => Some(prefs.multiplayer),
            MenuId::Compass => Some(prefs.compass as usize),
            MenuId::Rumble => Some(p.rumble as usize),
            MenuId::AutoAim => Some(!p.auto_aim as usize),
            MenuId::AutoAttack => Some(!p.auto_attack as usize),
            _ => None,
        };
        if let Some(r) = radio {
            for (i, it) in m.items.iter_mut().enumerate() {
                it.on = (i == r) as i32;
            }
        }
        match m.id() {
            MenuId::Audio => {
                let mode = prefs.stereo as i32;
                for (it, d) in m.items.iter_mut().zip(m.def.items) {
                    if d.1 == OPT_STEREO {
                        it.on = mode + 1;
                        it.value = mode;
                    }
                }
            }
            MenuId::ControlStyle => m.items[0].text = CONTROL_STYLES[style].into(),
            _ => {}
        }
    }

    /// do_optmenu with navigation on: the menu's return code.
    fn do_optmenu(&mut self, input: &PlayerInput) -> i32 {
        let (code, sound) = self.do_optmenu_inner(input);
        if let Some(name) = sound {
            self.play(name);
        }
        code
    }

    fn do_optmenu_inner(&mut self, input: &PlayerInput) -> (i32, Option<&'static str>) {
        let (dx, dy) = self.rep.edges(input);
        let q = |v: f32| (v > 0.5) as i32 - (v < -0.5) as i32;
        let held = (q(input.stick.x), q(input.stick.y));
        // A titled menu: START aborts everything, and keeps doing so.
        if input.start_pressed {
            self.nochoice = true;
        }
        let nochoice = self.nochoice;
        let m = self.stack.last_mut().unwrap();
        m.time += 1;
        let n = m.items.len();
        while m.sel < n && m.items[m.sel].value < 0 {
            m.sel += 1;
        }
        if m.sel >= n {
            m.sel = 0;
        }
        if nochoice {
            return (OPT_ABORTALL, None);
        }
        if input.attack_pressed {
            let (value, code) = (m.items[m.sel].value, m.def.items[m.sel].1);
            if value >= 0 {
                let sound = (m.flags() & F_SELECT_SOUND != 0).then_some("S_OPTMENUSEL");
                return (code, sound);
            }
            return (OPT_NONE, Some("S_NO"));
        }
        if input.b_pressed {
            return (OPT_ABORT, None);
        }
        if dx < 0 {
            return (OPT_LEFT_EDGE, None);
        }
        if dx > 0 {
            return (OPT_RIGHT_EDGE, None);
        }
        if dy != 0 {
            let start = m.sel;
            loop {
                m.sel = if dy < 0 {
                    (m.sel + 1) % n
                } else {
                    (m.sel + n - 1) % n
                };
                if m.sel == start || m.items[m.sel].value >= 0 {
                    break;
                }
            }
            return (1, Some("S_OPTMENUMOVVRT"));
        }
        let code = match held {
            (x, _) if x < 0 => OPT_LEFT_LEVEL,
            (x, _) if x > 0 => OPT_RIGHT_LEVEL,
            (_, y) if y > 0 => 9,
            (_, y) if y < 0 => 10,
            _ => OPT_NONE,
        };
        (code, None)
    }

    /// start_optmenu(m): the current top fades out under the new menu.
    fn push(&mut self, id: MenuId, prefs: &Prefs) {
        if let Some(t) = self.stack.last_mut() {
            t.end();
        }
        // stack[level + 1] is overwritten: a popped menu stops being served.
        self.popped = None;
        let mut m = Menu::new(id);
        m.sel = prefs.menu_sel[id as usize].min(m.items.len() - 1);
        m.icon_y = m.icon_target();
        self.stack.push(m);
        self.refresh(prefs);
    }

    /// start_optmenu(NULL): pop one level and restart the one below; the last
    /// one burns its backdrop away.
    fn pop(&mut self) {
        let Some(mut m) = self.stack.pop() else {
            return;
        };
        let had_backdrop = m.backdrop;
        let rect = m.b;
        m.end();
        self.popped = Some(m);
        match self.stack.last_mut() {
            Some(t) => t.restart(),
            None => {
                if had_backdrop {
                    self.start_scroll(rect);
                }
                self.finishing = true;
            }
        }
    }

    /// end_optmenu(-1, -1): drop every level, fade the top out, burn the backdrop.
    fn collapse(&mut self) {
        let Some(mut m) = self.stack.pop() else {
            return;
        };
        self.stack.clear();
        let had_backdrop = m.backdrop;
        let rect = m.b;
        m.end();
        self.popped = Some(m);
        if had_backdrop {
            self.start_scroll(rect);
        }
        self.finishing = true;
    }

    /// end_optmenu(-1, 1) on every level: gone at once, no fade or scroll.
    fn close_now(&mut self, action: OptionsAction) -> OptionsAction {
        self.stack.clear();
        self.popped = None;
        self.open = false;
        self.finishing = false;
        action
    }

    fn start_scroll(&mut self, rect: (f32, f32, f32, f32)) {
        if self.scroll.is_none() {
            self.play("S_OPTMENUSCROLL");
            self.scroll = Some(FireScroll { rect, ticks: 0 });
        }
    }

    /// Draw everything on screen: the fire scroll, fading menus, then the top menu.
    pub fn draw(&self, s: &mut Scr) {
        if let Some(f) = &self.scroll {
            draw_fire_scroll(s, f);
        }
        let n = self.stack.len();
        if n > 1 && self.stack[n - 2].fading() {
            self.show(s, &self.stack[n - 2]);
        }
        if let Some(p) = &self.popped {
            if p.fading() {
                self.show(s, p);
            }
        }
        if !self.finishing {
            if let Some(m) = self.stack.last() {
                self.show(s, m);
            }
        }
    }

    /// show_optmenu, plus the blits do_audiomenu / do_controlsmenu place.
    fn show(&self, s: &mut Scr, m: &Menu) {
        let flags = m.flags();
        let parch = if flags & F_PARCH != 0 {
            s.ui.get("FONT32_PARCH")
        } else {
            None
        };
        let normal = s.ui.font(FONT_32).map(|f| f.tex);
        let (bx, by, bw, bh) = m.b;

        // Fade in / out (font alpha: 0 opaque .. 255 invisible).
        let mut fade = 0;
        if flags & F_FADE != 0 {
            if m.finish != 0 {
                fade = m.finish * 255 / FINISH_FRAMES;
            } else if m.time < FINISH_FRAMES {
                fade = (FINISH_FRAMES - m.time) * 255 / FINISH_FRAMES;
            }
        } else if m.finish != 0 {
            return;
        }
        // Garamond burn-in: FONT32GAR0..5 over the first frames (reversed on close).
        let mut hifont: Option<Tex> = None;
        if flags & F_GARAMOND != 0 {
            let fr = if m.finish != 0 {
                6 - m.finish * 6 / FINISH_FRAMES
            } else {
                (m.time - 10) / 2
            };
            if fr < 0 {
                hifont = s.ui.get("FONT32GAR0");
            } else if fr < 6 {
                hifont = s.ui.frame("FONT32GAR0", fr as usize);
            }
        }

        // Backdrop (OPTMENU_SCROLLZ, behind everything) and the burn overlay.
        if m.backdrop {
            if let Some(t) = s.ui.get("SCROLL_A") {
                s.blit(t, bx, by, bw, bh, 0);
            }
        }
        if m.burn {
            if let Some(t) = s.ui.frame("LOGO_BURN1", ((m.time >> 3) % 5) as usize) {
                let (x, y, w, h) = BURN_RECT;
                s.blit(t, x, y, w, h, 0);
            }
        }
        if m.finish == 0 && self.stack.last().is_some_and(|t| std::ptr::eq(t, m)) {
            self.show_extras(s, m);
        }

        // Title, centred on the backdrop.
        if m.backdrop {
            let tex = parch.or(normal);
            if let Some(t) = tex {
                s.text_tex(
                    t,
                    1.2,
                    -(bx + (bw as i32 / 2) as f32),
                    by + MARGIN_TITLE,
                    RGB_TEXT,
                    0,
                    m.def.title,
                );
            }
        }

        // Items. The selected one is drawn twice: a pulsing glow, then solid.
        let scale = m.def.scale;
        let lh = line_height(scale) as f32;
        let width = |s: &Scr, t: &str| s.text_width(FONT_32, scale, t).floor();
        let mut y = m.y;
        let mut idx = 0usize;
        let mut part = 0;
        let mut sel = m.sel as i32;
        while idx < m.items.len() {
            let it = &m.items[idx];
            let d = &m.def.items[idx];
            let mut alpha = fade;
            if it.value < 0 && fade < 0x80 {
                alpha = 0x80;
            }
            let mut glow = false;
            let mut tex = hifont.or(parch);
            let color;
            let hi = idx as i32 == sel && it.value >= 0;
            if hi {
                alpha = (alpha + glow_alpha(m.time.max(0) as u32)).min(255);
                glow = true;
                color = RGB_HI;
            } else if -1 - idx as i32 == sel && part == it.value {
                // The binary loads rgb_on here (0x80072EFC); options.c says rgb_hi.
                color = RGB_ON;
                tex = hifont;
            } else if parch.is_some() && hifont.is_none() {
                color = 0xFFFFFF;
            } else {
                color = RGB_OFF;
            }
            let tex = tex.or(normal);
            let (mut x, text) = if part != 0 || (hi && it.value > 0) {
                let x = m.def.x as f32 + width(s, &it.text) + width(s, " ~ ");
                (x, d.3.unwrap_or(""))
            } else {
                (m.def.x as f32, it.text.as_str())
            };
            let put = |s: &mut Scr, x: f32, t: &str| {
                if glow {
                    s.glow_pass(scale, x, y, color, alpha, t);
                } else if let Some(tex) = tex {
                    s.text_tex(tex, scale, x, y, color, alpha, t);
                }
            };
            put(s, x, text);
            x += width(s, text);
            if it.on > 0 && it.on == part + 1 {
                put(s, x, " ~");
            }
            if idx as i32 == sel {
                sel = -1 - sel;
            } else if d.3.is_some() && part == 0 {
                part = 1;
            } else {
                y += d.2 as f32 + lh;
                part = 0;
                idx += 1;
            }
        }

        // "Player N".
        if flags & F_PLAYER != 0 {
            if let Some(t) = parch.or(normal) {
                let px = (self.player * 100 + 106) as f32;
                let tag = format!("Player {}", self.player + 1);
                s.text_tex(t, PLAYER_SCALE, -px, by + MARGIN_PLAYER, RGB_TEXT, 0, &tag);
            }
        }

        // TODO: the 3D ICON_ARROW cursor (POWERUPS atree, scale 0.05, z 1.1) at
        // x = item x + icon_dx (-16; -48 on Control Style), centred on the
        // selected line, gliding over 15 ticks with a half turn.

        // Prompt row.
        if flags & 0xF != 0 && m.finish == 0 {
            self.prompts(s, m, parch.or(normal));
        }
    }

    /// The audio sliders and the controller diagram (top menu only).
    fn show_extras(&self, s: &mut Scr, m: &Menu) {
        match m.id() {
            MenuId::Audio => {
                let fh = line_height(m.def.scale) as f32;
                for (i, d) in m.def.items.iter().enumerate() {
                    let val = match d.1 {
                        OPT_MUSICVOL => self.music_val,
                        OPT_SFXVOL => self.sfx_val,
                        _ => continue,
                    };
                    let y = m.draw_y(i) + fh;
                    slider(s, val, m.def.x as f32, y, i != m.sel);
                }
            }
            MenuId::ControlStyle => {
                for (name, x, y) in CTL_BLITS {
                    s.blit_named(name, x, y, 0);
                }
                // TODO: the 16 per-style labels (string list 3 of ENGLISH.ROM,
                // black, at ctl_label_pos 0x8011FA38) are not decoded yet.
            }
            _ => {}
        }
    }

    /// The prompt row: Back, Change, Select / Accept, Center, evenly spaced.
    fn prompts(&self, s: &mut Scr, m: &Menu, tex: Option<Tex>) {
        let flags = m.flags();
        let n = [F_BACK, F_SELECT, F_CHANGE, F_CENTER]
            .iter()
            .filter(|&&f| flags & f != 0)
            .count() as i32;
        let px = 512 / (n + 1);
        let sz = (32.0 * MSG_SCALE) as i32 as f32;
        let py = 304.0;
        let mut idx = 0;
        let prompt = |s: &mut Scr, idx: i32, label: &str, icons: &[&str]| {
            if let Some(t) = tex {
                s.text_tex(t, MSG_SCALE, -(idx as f32), py, RGB_TEXT, 0, label);
            }
            let pw = s.text_width(FONT_32, MSG_SCALE, label) as i32;
            let mut x = (idx - pw / 2) as f32 - sz - 4.0;
            for icon in icons.iter().rev() {
                if let Some(t) = s.ui.get(icon) {
                    s.blit(t, x, py, sz, sz, 0);
                }
                x -= sz;
            }
        };
        if flags & F_BACK != 0 {
            idx = px;
            prompt(s, idx, "Back", &["BUTTON_TRI"]);
        }
        if flags & F_CHANGE != 0 {
            idx += px;
            prompt(s, idx, "Change", &["BUTTON_L", "BUTTON_R"]);
        }
        if flags & F_SELECT != 0 {
            idx += px;
            let label = if flags & (F_ACCEPT | F_SELECT) == F_ACCEPT | F_SELECT {
                "Accept"
            } else {
                "Select"
            };
            prompt(s, idx, label, &["BUTTON_X"]);
        }
        if flags & F_CENTER != 0 {
            idx += px;
            prompt(s, idx, "Center", &["BUTTON_O"]);
        }
    }
}

/// position_audioslider: markers, empty bar, pink fill and knob; inactive rows faded.
fn slider(s: &mut Scr, val: i32, x: f32, y: f32, inactive: bool) {
    let fill = (VOL_WIDTH * val / 255).max(1) as f32;
    let a = inactive as i32;
    let w = VOL_WIDTH as f32;
    if let Some(t) = s.ui.get("EMPTY_BAR") {
        s.blit(t, x, y + 11.0, w, 32.0, a * 150);
    }
    if let Some(t) = s.ui.get("PINK_BAR") {
        s.blit(t, x, y + 15.0, fill, 32.0, a * 160);
    }
    s.blit_named("MARKER_LEFT", x + SLIDER_DXL, y, a * 100);
    s.blit_named("MARKER_RIGHT", x + w + SLIDER_DXR, y, a * 100);
    s.blit_named("SLIDER", x + fill - 20.0, y + 2.0, a * 100);
}

/// The fire scroll: the backdrop with the greenCircTrans ring burning outwards.
fn draw_fire_scroll(s: &mut Scr, f: &FireScroll) {
    let t = (f.ticks >> 1).clamp(0, FIRE_FRAMES - 1);
    let (x, y, w, h) = f.rect;
    // TODO: the original erases the backdrop through the greenCircTransM mask
    // frames (blit entry flag 0x10000); without a mask blend the backdrop fades.
    if let Some(img) = s.ui.get("SCROLL_A") {
        let fade = (t * 255 / FIRE_FRAMES).min(255);
        s.blit(img, x, y, w, h, fade);
    }
    if let Some(c) = s.ui.frame("greenCircTrans", 1 + t as usize) {
        s.blit(c, x, y, w, h, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec2;

    fn idle() -> PlayerInput {
        PlayerInput::default()
    }
    fn a() -> PlayerInput {
        PlayerInput {
            attack_pressed: true,
            ..idle()
        }
    }
    fn b() -> PlayerInput {
        PlayerInput {
            b_pressed: true,
            ..idle()
        }
    }
    fn start() -> PlayerInput {
        PlayerInput {
            start_pressed: true,
            ..idle()
        }
    }
    fn stick(x: f32, y: f32) -> PlayerInput {
        PlayerInput {
            stick: Vec2::new(x, y),
            ..idle()
        }
    }

    fn run(o: &mut Options, p: &mut Prefs, i: PlayerInput) -> OptionsAction {
        o.update(&i, p)
    }

    #[test]
    fn audio_volume_back_and_close() {
        let mut p = Prefs::default();
        let mut o = Options::title(0);
        assert_eq!(o.current(), Some(MenuId::Options));
        assert_eq!(run(&mut o, &mut p, idle()), OptionsAction::None);
        o.sfx.clear();
        // A on "Audio".
        run(&mut o, &mut p, a());
        assert_eq!(o.current(), Some(MenuId::Audio));
        assert_eq!(o.sfx, ["S_OPTMENUSEL"]);
        // Hold right on Music Volume: +1 a tick.
        let mut changed = 0;
        for _ in 0..10 {
            if run(&mut o, &mut p, stick(1.0, 0.0)) == OptionsAction::VolumeChanged {
                changed += 1;
            }
        }
        assert_eq!(p.music_vol, 138);
        assert_eq!(changed, 10);
        run(&mut o, &mut p, idle());
        // Down to Sfx Volume, left.
        o.sfx.clear();
        run(&mut o, &mut p, stick(0.0, -1.0));
        assert_eq!(o.selected(), Some(1));
        assert_eq!(o.sfx, ["S_OPTMENUMOVVRT"]);
        run(&mut o, &mut p, idle());
        for _ in 0..5 {
            run(&mut o, &mut p, stick(-1.0, 0.0));
        }
        assert_eq!(p.sfx_vol, 123);
        run(&mut o, &mut p, idle());
        // Down to Mono/Stereo and toggle.
        run(&mut o, &mut p, stick(0.0, -1.0));
        run(&mut o, &mut p, idle());
        assert!(p.stereo);
        assert_eq!(
            run(&mut o, &mut p, stick(-1.0, 0.0)),
            OptionsAction::VolumeChanged
        );
        assert!(!p.stereo);
        run(&mut o, &mut p, idle());
        // Down wraps to Music Volume.
        run(&mut o, &mut p, stick(0.0, -1.0));
        assert_eq!(o.selected(), Some(0));
        run(&mut o, &mut p, idle());
        // B leaves Audio with S_CHOOSESELSFX instead of the exit sound.
        o.sfx.clear();
        run(&mut o, &mut p, b());
        assert_eq!(o.current(), Some(MenuId::Options));
        assert_eq!(o.sfx, ["S_CHOOSESELSFX"]);
        assert_eq!(o.selected(), Some(0));
        // B on the root closes back to the title menu, burning the scroll away.
        o.sfx.clear();
        assert_eq!(run(&mut o, &mut p, b()), OptionsAction::Closed);
        assert!(!o.is_open());
        assert!(o.sfx.contains(&"S_OPTMENUEXIT".to_string()));
        assert!(o.sfx.contains(&"S_OPTMENUSCROLL".to_string()));
        assert!(o.is_drawing());
        for _ in 0..60 {
            assert_eq!(run(&mut o, &mut p, a()), OptionsAction::None);
        }
        assert!(!o.is_drawing());
    }

    #[test]
    fn radio_menus_store_prefs() {
        let mut p = Prefs::default();
        let mut o = Options::title(1);
        run(&mut o, &mut p, stick(0.0, -1.0));
        run(&mut o, &mut p, idle());
        run(&mut o, &mut p, a()); // Game Options
        assert_eq!(o.current(), Some(MenuId::GameOptions));
        run(&mut o, &mut p, a()); // Difficulty, opens on Medium
        assert_eq!(o.current(), Some(MenuId::Difficulty));
        assert_eq!(o.selected(), Some(1));
        run(&mut o, &mut p, stick(0.0, -1.0));
        run(&mut o, &mut p, idle());
        o.sfx.clear();
        run(&mut o, &mut p, a());
        assert_eq!(p.difficulty, 2);
        assert_eq!(o.sfx, ["S_OPTMENUSEL", "S_CHOOSESELSFX"]);
        assert_eq!(o.current(), Some(MenuId::Difficulty));
        run(&mut o, &mut p, b());
        run(&mut o, &mut p, b());
        assert_eq!(o.current(), Some(MenuId::Options));
        // Controls > Auto Aim > Off for player 2.
        for _ in 0..2 {
            run(&mut o, &mut p, stick(0.0, -1.0));
            run(&mut o, &mut p, idle());
        }
        run(&mut o, &mut p, a());
        assert_eq!(o.current(), Some(MenuId::Controls));
        for _ in 0..2 {
            run(&mut o, &mut p, stick(0.0, -1.0));
            run(&mut o, &mut p, idle());
        }
        run(&mut o, &mut p, a());
        assert_eq!(o.current(), Some(MenuId::AutoAim));
        run(&mut o, &mut p, stick(0.0, -1.0));
        run(&mut o, &mut p, idle());
        run(&mut o, &mut p, a());
        assert!(!p.players[1].auto_aim);
        assert!(p.players[0].auto_aim);
    }

    #[test]
    fn game_menu_abort_defaults_to_no_and_start_closes() {
        let mut p = Prefs::default();
        let mut o = Options::game(0);
        run(&mut o, &mut p, stick(0.0, -1.0));
        run(&mut o, &mut p, idle());
        run(&mut o, &mut p, a()); // Quit Level
        assert_eq!(o.current(), Some(MenuId::AbortLevel));
        assert_eq!(o.selected(), Some(0));
        run(&mut o, &mut p, a()); // No: back
        assert_eq!(o.current(), Some(MenuId::Game));
        run(&mut o, &mut p, a());
        run(&mut o, &mut p, stick(0.0, -1.0));
        run(&mut o, &mut p, idle());
        assert_eq!(run(&mut o, &mut p, a()), OptionsAction::QuitLevel);
        assert_eq!(o.current(), None);

        // START from a submenu closes every level, one a tick, then fades out.
        // The menu reopens on the item it was left on.
        let mut o = Options::game(0);
        run(&mut o, &mut p, idle());
        assert_eq!(o.selected(), Some(1));
        run(&mut o, &mut p, stick(0.0, 1.0));
        run(&mut o, &mut p, idle());
        run(&mut o, &mut p, a()); // Settings
        assert_eq!(o.current(), Some(MenuId::GameSettings));
        run(&mut o, &mut p, start());
        assert_eq!(o.current(), Some(MenuId::Game));
        run(&mut o, &mut p, idle());
        assert_eq!(o.current(), None);
        assert!(o.is_open());
        let mut ticks = 0;
        while run(&mut o, &mut p, idle()) != OptionsAction::Closed {
            ticks += 1;
            assert!(ticks < 40);
        }
        assert!(!o.is_open());
    }

    #[test]
    fn disabled_quit_level_is_skipped() {
        let mut p = Prefs::default();
        let mut o = Options::game(0);
        o.set_quit_level_enabled(false);
        run(&mut o, &mut p, stick(0.0, -1.0));
        assert_eq!(o.selected(), Some(0));
    }

    #[test]
    fn tower_actions() {
        let mut p = Prefs::default();
        let mut o = Options::tower(0);
        for _ in 0..2 {
            run(&mut o, &mut p, stick(0.0, -1.0));
            run(&mut o, &mut p, idle());
        }
        assert_eq!(run(&mut o, &mut p, a()), OptionsAction::Shop);
        assert!(!o.is_open() && !o.is_drawing());
        // The menu remembers its selection.
        let mut o = Options::tower(0);
        run(&mut o, &mut p, idle());
        assert_eq!(o.selected(), Some(2));
    }

    #[test]
    fn control_style_cycles_three() {
        let mut p = Prefs::default();
        let mut o = Options::title(0);
        for _ in 0..3 {
            run(&mut o, &mut p, stick(0.0, -1.0));
            run(&mut o, &mut p, idle());
        }
        run(&mut o, &mut p, a()); // Controls
        run(&mut o, &mut p, a()); // Style
        assert_eq!(o.current(), Some(MenuId::ControlStyle));
        run(&mut o, &mut p, stick(-1.0, 0.0));
        run(&mut o, &mut p, idle());
        run(&mut o, &mut p, b()); // ignored
        assert_eq!(o.current(), Some(MenuId::ControlStyle));
        run(&mut o, &mut p, a());
        assert_eq!(p.players[0].control_style, 2);
        assert_eq!(o.current(), Some(MenuId::Controls));
    }
}
