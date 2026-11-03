//! The between-level shop (MG_SHOP, shop/shop.c do_shop and friends): per-player
//! columns with the end-of-level tally piles, the level-up panel, the scrolling
//! buy/sell list from `SHPDATA/SHOP.WAD`, the panel showing what was bought, the
//! inventory panel (MESSAGE.OBJ draw_inventory_panel) and the Final Stats.
//!
//! Each player runs the original's per-player state machine (`Player.field_A64`):
//!
//! | state | AfterLevel | FromMenu | Inventory |
//! |---|---|---|---|
//! | 0 | tally setup → 1 | → 6 | → 6 |
//! | 1-3 | piles rise; A → 4 | | |
//! | 4 | level-up panel (skipped if the level is unchanged) → 5, or 20 with Final Stats | | |
//! | 5 | re-seed the gold pile → 6 | | |
//! | 6/7 | shopping until EXIT → 8 | same | → 9 |
//! | 8 | "what you bought" panel; A → 9 | same | |
//! | 9 | → 10 (AudioTowerFX 2) | done | → 10 |
//! | 10/11 | inventory panel; A closes it | | same |
//! | 20 | Final Stats; A → done | | |
//!
//! The caller plays [`Shop::music`] with `Audio::play_bank_music` when the shop
//! starts, plays and drains [`Shop::sfx`] each tick, stops the music when
//! [`Shop::update`] returns true and takes the results with [`Shop::players`].
//! The original also draws the player info panels (WritePlayerInfo) under the
//! columns every frame; that is left to the caller.
//!
//! Choices where the original's behaviour is not pinned down:
//! * The three pile textures (SHP_GOLD/BONES/EXP) are all placed at the column's x
//!   with the full column width and the same depth (the DOL's blit table has
//!   x offset 0 for each). They are drawn tallest first, so the shorter ones sit
//!   in front of it, which is the only order that shows all three and is why
//!   fn_8009A0AC ranks them by height.
//! * Piles move `gFrameTicks + (gFrameTicks >> 1)` pixels a frame, which is 1 at
//!   the shop's 60 Hz (1 tick a frame).
//! * Power-up durations from the shop are scaled by the class's powerup_time like
//!   the pickups in game.rs (use-counted ones keep their -1).
//! * Shop item icons are drawn above the scroll and below the column borders.

use crate::fs::GameFs;
use crate::game::PanelInfo;
use crate::game::{max_health, Carried, MAX_KEYS, MAX_POTIONS};
use crate::input::{PlayerInput, Repeat};
use crate::stats::{self, Atts};
use anyhow::{bail, Context, Result};
use gdl_formats::wdata::LevelDef;

/// Column left x, centre and right (lbl_80122F30 / F40 / F50).
pub const COL_X: [i32; 4] = [0, 128, 256, 384];
pub const COL_CENTRE: [i32; 4] = [64, 192, 320, 448];
pub const COL_RIGHT: [i32; 4] = [122, 250, 378, 506];

/// Pile meter: top, bottom and the seed height (lbl_80343E0C / E10 / E14).
pub const PILE_TOP: i32 = 112;
pub const PILE_BOT: i32 = 320;
pub const PILE_BASE: i32 = 20;
/// The tally's minimum pile height.
pub const PILE_MIN: i32 = 64;
/// Pixels a pile moves per tick (gFrameTicks + (gFrameTicks >> 1) at 1 tick).
pub const PILE_STEP: i32 = 1;
/// Pile blits in table order (lbl_80122ED0), their labels and label y (0x80122F70,
/// 0x80122F7C).
pub const PILE_TEX: [&str; 3] = ["SHP_GOLD", "SHP_BONES", "SHP_EXP"];
pub const PILE_LABEL: [&str; 3] = ["Gold", "Kills", "Exp."];
pub const PILE_LABEL_Y: [i32; 3] = [32, 52, 72];
/// The tally's " Continue" row (0x80122F88).
pub const CONTINUE_Y: i32 = 92;

/// The buy list's visible band and its fade (lbl_80343E04 / E08 / E00).
pub const MENU_TOP: i32 = 72;
pub const MENU_BOT: i32 = 224;
pub const MENU_DIM: i32 = 64;
/// FONT32's cell height (FONTS/FONT32.FNT), for TextHeightMLines.
pub const FONT32_HEIGHT: f32 = 32.0;
/// Ticks a bought item's text stays red.
pub const RED_TICKS: i32 = 30;

/// Health per level (lbl_803483A8).
pub const HEALTH_PER_LEVEL: i32 = 100;

/// The continue prompt's icon (lbl_80344E48): GC A.
pub const BUTTON_A: &str = "BUTTON_X";

/// Sounds (COMMON bank ids): AudioCursorSelect 0x11, AudioBuzzer 0xA,
/// AudioClick 0x18 / 0x17 (lbl_80343E2C), fn_8009D038 0x26, AudioTowerFX(2) 0x16.
pub const SFX_SELECT: &str = "S_OPTMENUSEL";
pub const SFX_BUZZER: &str = "S_NO";
pub const SFX_CLICK: [&str; 2] = ["S_SECRETCLOCK2", "S_SECRETCLOCK1"];
pub const SFX_BUY: &str = "S_PICKUPMAGIC";
pub const SFX_INVENTORY: &str = "S_STNDGLASS";

/// lbl_80122F8C: the power-up slot (type, flags) each item type fills, which
/// decides whether it is owned (AudioFindPlayerSlot) and can be sold back.
pub const TYPE_SLOT: [(u8, u32); 40] = [
    (0, 0),
    (0, 0),
    (5, 0x200000),
    (0, 0),
    (0, 0),
    (0, 0),
    (0, 0),
    (0, 0),
    (0, 0),
    (6, 0x20000),
    (9, 0x80),
    (5, 0x2000_0000),
    (5, 0x80000),
    (5, 0x1000_0000),
    (6, 0x400000),
    (6, 0x200000),
    (0, 0),
    (0, 0),
    (9, 0x1),
    (0, 0),
    (5, 1),
    (5, 2),
    (5, 3),
    (5, 4),
    (5, 0x100000),
    (9, 0x10),
    (9, 0x40),
    (9, 0x20),
    (9, 0x10000),
    (0, 0),
    (9, 4),
    (6, 0x10000),
    (9, 2),
    (6, 0x2008),
    (5, 0x400000),
    (6, 0x80000),
    (9, 0x200000),
    (9, 0x400000),
    (9, 0x100000),
    (9, 8),
];

/// do_shopping's PlayerAddPowerup per item type: (type, flags, amount, seconds),
/// the floats from sdata2 0x803483CC.. (a negative time counts uses).
pub fn powerup_for(kind: u32) -> Option<(u8, u32, f32, f32)> {
    Some(match kind {
        2 => (5, 0x200000, 0.0, 90.0),
        4 => (9, 0x100, 0.0, 30.0),
        9 => (6, 0x20000, 0.0, 30.0),
        10 => (9, 0x80, 0.0, 45.0),
        11 => (5, 0x2000_0000, 0.0, 30.0),
        12 => (5, 0x80000, 0.0, 45.0),
        13 => (5, 0x1000_0000, 3.0, -1.0),
        14 => (6, 0x400000, 0.0, 15.0),
        15 => (6, 0x200000, 0.0, 15.0),
        16 => (6, 0x110000, 0.0, 20.0),
        18 => (9, 1, 0.0, 60.0),
        19 => (9, 0x100, 0.0, 30.0),
        20..=23 => (5, kind - 19, 0.0, 90.0),
        24 => (5, 0x100000, 5.0, -1.0),
        25 => (9, 0x10, 5.0, -1.0),
        26 => (9, 0x40, 5.0, -1.0),
        27 => (9, 0x20, 5.0, -1.0),
        28 => (9, 0x10000, 4.0, 40.0),
        29 => (9, 0x200, 0.0, 15.0),
        30 => (9, 4, 0.0, 15.0),
        31 => (6, 0x10000, 0.0, 30.0),
        32 => (9, 2, 0.0, 120.0),
        33 => (6, 0x2008, 0.0, 15.0),
        34 => (5, 0x400000, 0.0, 45.0),
        35 => (6, 0x80000, 0.0, 120.0),
        36 => (9, 0x200000, 0.0, 120.0),
        37 => (9, 0x400000, 0.0, 120.0),
        38 => (9, 0x100000, 0.0, 120.0),
        39 => (9, 8, 0.0, 25.0),
        _ => return None,
    })
}

/// Which screen the shop was opened for (init_shop's argument).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShopMode {
    /// init_shop(0): after a level, starting with the tally.
    AfterLevel,
    /// init_shop(1): the Tower Menu's "Shop", straight to the buy list.
    FromMenu,
    /// init_shop(2): the Tower Menu's "Inventory", only the inventory panel.
    Inventory,
}

/// One SHOP.WAD "ITEM" record (0x50 bytes).
#[derive(Clone, Debug, PartialEq)]
pub struct ShopItem {
    /// Icon texture name; empty for EXIT.
    pub blit: String,
    /// Description, lines separated by `\n`.
    pub desc: String,
    /// Text scale factor (times 0.5).
    pub scale: f32,
    /// do_shopping's effect selector.
    pub kind: u32,
    pub price: i32,
    /// Health healed by food.
    pub amount: u32,
}

impl ShopItem {
    /// The icon's blit exists (calc_shop_ypos's MBBlitGetTex > 0).
    pub fn has_icon(&self) -> bool {
        !self.blit.is_empty()
    }
}

/// Parse SHOP.WAD: the "ITEM" chunk from the directory at the end of the file.
pub fn parse_items(d: &[u8]) -> Result<Vec<ShopItem>> {
    let u32_at = |o: usize| -> Result<u32> {
        Ok(u32::from_le_bytes(
            d.get(o..o + 4).context("SHOP.WAD truncated")?.try_into()?,
        ))
    };
    let cstr = |b: &[u8]| {
        let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
        String::from_utf8_lossy(&b[..end]).into_owned()
    };
    let mut i = d.len().checked_sub(16).context("SHOP.WAD too short")?;
    let (off, count) = loop {
        if &d[i..i + 4] == b"METI" {
            break (u32_at(i + 4)? as usize, u32_at(i + 8)? as usize);
        }
        if i < 16 || !d[i..i + 4].iter().all(|c| c.is_ascii_alphanumeric()) {
            bail!("SHOP.WAD has no ITEM chunk");
        }
        i -= 16;
    };
    (0..count)
        .map(|k| {
            let o = off + k * 0x50;
            let r = d.get(o..o + 0x50).context("ITEM record past end")?;
            Ok(ShopItem {
                blit: cstr(&r[..32]),
                desc: cstr(&r[32..64]),
                scale: f32::from_le_bytes(r[64..68].try_into()?),
                kind: u32_at(o + 68)?,
                price: u32_at(o + 72)? as i32,
                amount: u32_at(o + 76)?,
            })
        })
        .collect()
}

/// ShopLoadData: the 34 shop items.
pub fn load_items(fs: &GameFs) -> Result<Vec<ShopItem>> {
    parse_items(&fs.read("SHPDATA/SHOP.WAD")?)
}

/// The TEXT/ENGLISH.ROM strings the shop panels use.
#[derive(Clone, Debug, Default)]
pub struct ShopText {
    /// String 21, the level-99 title.
    pub legend: String,
    /// String 23: class names, the title below level 10.
    pub classes: Vec<String>,
    /// List 0 by character: the class's rank titles.
    pub ranks: Vec<Vec<String>>,
    /// Strings 184 / 185 by char_type: the level 25 / 50 potion messages.
    pub magic: [Vec<String>; 2],
}

impl ShopText {
    pub fn load(fs: &GameFs) -> ShopText {
        fs.read("TEXT/ENGLISH.ROM")
            .ok()
            .and_then(|d| gdl_formats::text::TextRom::parse(&d).ok())
            .map(|r| ShopText::from_rom(&r))
            .unwrap_or_default()
    }

    pub fn from_rom(r: &gdl_formats::text::TextRom) -> ShopText {
        let lines = |i: usize| {
            r.messages
                .get(i)
                .map(|m| m.lines.clone())
                .unwrap_or_default()
        };
        ShopText {
            legend: lines(21).first().cloned().unwrap_or_default(),
            classes: lines(23),
            ranks: r
                .lists
                .first()
                .map(|l| l.iter().map(|&m| lines(m.max(0) as usize)).collect())
                .unwrap_or_default(),
            magic: [lines(184), lines(185)],
        }
    }

    /// shop_show_lv's title: Legend at 99, the class name below 10, else the
    /// class's rank `(level / 10) >> 1`.
    pub fn rank(&self, class: usize, level: u32) -> String {
        let s = if level == 99 {
            Some(&self.legend)
        } else if level < 10 {
            self.classes.get(class)
        } else {
            self.ranks
                .get(class)
                .and_then(|r| r.get(((level / 10) >> 1) as usize))
        };
        s.cloned().unwrap_or_default()
    }
}

/// What the inventory panel shows for a player (the tower progress the original
/// reads from the save: completion counts, crystals, legend items, rune pieces).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Collection {
    pub fangs: i32,
    pub feathers: i32,
    pub claws: i32,
    /// Crystals per colour (orange .. black); -1 once complete.
    pub crystals: [i32; 8],
    /// Legend items by towerGetRuneNearStat index (scimitar 1 .. javelin 11).
    pub legend: [bool; 12],
    /// Boss rune pieces by PlayerHasRune index (lich 1 .. wraith 8).
    pub runes: [bool; 9],
}

/// A panel piece (PanelPieceRow): texture, x, y, and for the counters the text
/// scale and offsets.
pub struct Piece {
    pub name: Option<&'static str>,
    pub x: i32,
    pub y: i32,
    pub scale: f32,
    pub dx: i32,
    pub dy: i32,
}

pub const fn piece(
    name: Option<&'static str>,
    x: i32,
    y: i32,
    scale: f32,
    dx: i32,
    dy: i32,
) -> Piece {
    Piece {
        name,
        x,
        y,
        scale,
        dx,
        dy,
    }
}

/// arrH (0x8011D778): the window, the fang / feather / claw counters and the
/// eight crystal counters.
pub const PANEL_H: [Piece; 12] = [
    piece(Some("WINDOW_EMPTY"), 0, 0, 0.0, 0, 0),
    piece(Some("FANGS"), 56, 116, 0.5, 30, 2),
    piece(Some("FEATHER"), 56, 140, 0.5, 30, 2),
    piece(Some("CLAW"), 56, 164, 0.5, 30, 2),
    piece(Some("ORANGE_CRYSTLE"), 6, 200, 0.35, 28, 2),
    piece(Some("RED_CRYSTLE"), 6, 216, 0.35, 28, 2),
    piece(Some("PURPLE_CRYSTLE"), 6, 232, 0.35, 28, 2),
    piece(Some("CYAN_CRYSTLE"), 6, 248, 0.35, 28, 2),
    piece(Some("GREEN_CRYSTLE"), 64, 200, 0.35, 28, 2),
    piece(Some("YELLOW_CRYSTLE"), 64, 216, 0.35, 28, 2),
    piece(Some("WHITE_CRYSTLE"), 64, 232, 0.35, 28, 2),
    piece(Some("BLACK_CRYSTLE"), 64, 248, 0.35, 28, 2),
];
/// arrF (0x8011DAFC): the legend items; `_EMPTY` when not found.
pub const PANEL_F: [Piece; 12] = [
    piece(None, 0, 0, 0.0, 0, 0),
    piece(Some("SCIMITAR"), 96, 32, 0.0, 0, 0),
    piece(Some("ICE_AX"), 76, 32, 0.0, 0, 0),
    piece(Some("LAMP"), 94, 58, 0.0, 0, 0),
    piece(Some("BILLOWS"), 77, 56, 0.0, 0, 0),
    piece(Some("SOUL_SAVIOR"), 98, 82, 0.0, 0, 0),
    piece(None, 0, 0, 0.0, 0, 0),
    piece(Some("BOOK"), 54, 32, 0.0, 0, 0),
    piece(None, 0, 0, 0.0, 0, 0),
    piece(Some("FIRE_SCROLL"), 56, 80, 0.0, 0, 0),
    piece(Some("LANTERN"), 77, 81, 0.0, 0, 0),
    piece(Some("JAVILIN"), 54, 56, 0.0, 0, 0),
];
/// arrE (0x8011D928): the boss rune pieces, drawn only when owned.
pub const PANEL_E: [Piece; 9] = [
    piece(None, 0, 0, 0.0, 0, 0),
    piece(Some("LITCH_PIECE"), 0, 86, 0.0, 0, 0),
    piece(Some("DRAGON_PIECE"), 0, 124, 0.0, 0, 0),
    piece(Some("CHIMERA_PIECE"), 0, 167, 0.0, 0, 0),
    piece(Some("PLAGUE_PIECE"), 22, 131, 0.0, 0, 0),
    piece(Some("DRYDER_PIECE"), 0, 106, 0.0, 0, 0),
    piece(Some("GENIE_PIECE"), 25, 72, 0.0, 0, 0),
    piece(Some("YETTI_PIECE"), 0, 148, 0.0, 0, 0),
    piece(Some("WRAITH_PIECE"), 0, 63, 0.0, 0, 0),
];
/// Crystals needed per colour (lbl_80124C70[1..8]).
pub const CRYSTALS_NEEDED: [i32; 8] = [15, 100, 125, 150, 175, 200, 225, 250];
/// Inventory panel fly-in / fly-out lengths (lbl_803473E4 / E8).
pub const INV_IN: i32 = 120;
pub const INV_OUT: i32 = 15;

/// What a column showed this tick (the original draws while it updates).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum View {
    None,
    /// show_gold.
    Tally,
    /// shop_show_lv: `t` is the panel tick read this frame.
    Level {
        final_: bool,
        t: u32,
        old_level: u32,
        prompt: bool,
    },
    /// do_shopping / write_shop_menu.
    Shopping,
    /// draw_inventory_panel: mode 0 in, 1 shown, 2 out; progress 0..1.
    Inventory {
        mode: u32,
        prog: f32,
        prompt: bool,
    },
    /// shop_show_final_stats.
    Final {
        t: u32,
        prompt: bool,
    },
}

/// One player's column.
#[derive(Clone, Debug)]
pub struct Col {
    pub info: PanelInfo,
    pub now: Carried,
    pub start: Carried,
    /// field_A64.
    pub state: u32,
    /// field_A6C: the panels' tick counter.
    pub timer: u32,
    /// field_A68: the cursor in the buy list.
    pub cursor: i32,
    /// field_A70..A7C: stats bought in this visit.
    pub bought: Atts,
    // The tally piles by rank (tallest first): target, shown height, value and
    // which pile texture (0 gold, 1 bones, 2 exp).
    pub pile_targ: [i32; 3],
    pub pile_shown: [i32; 3],
    pub pile_val: [i32; 3],
    pub pile_count: [usize; 3],
    pub pile_vis: [bool; 3],
    pub pile_grew: [bool; 3],
    pub tally_done: bool,
    /// The gold pile's height while shopping (show_piles), if shown.
    pub gold_pile: Option<i32>,
    /// SHOP_SCROLL_1/2 and the item blits are up.
    pub scroll: bool,
    // do_shopping's per-player words: top_y, cursor_y (dim), curfreeze,
    // last_available.
    pub top: i32,
    pub dim: i32,
    pub freeze: i32,
    pub last_available: i32,
    /// Item blits' current y; the first menu frame snaps them.
    pub item_y: Vec<i32>,
    pub first_menu: bool,
    /// Per item: red flash ticks, drawn red this frame, owned (flag 4),
    /// buyable (flag 2), skipped by the cursor.
    pub timers: Vec<i32>,
    pub red: Vec<bool>,
    pub owned: Vec<bool>,
    pub buyable: Vec<bool>,
    pub skip: Vec<bool>,
    pub repeat: Repeat,
    /// draw_inventory_panel's state / state2.
    pub inv_mode: u32,
    pub inv_t: i32,
    pub view: View,
    pub rng: u32,
}

#[derive(Clone, Debug)]
pub struct Shop {
    /// Sound names to play this tick, drained by the caller.
    pub sfx: Vec<String>,
    /// What each player's inventory panel shows, in player order.
    pub collections: Vec<Collection>,
    pub items: Vec<ShopItem>,
    pub text: ShopText,
    pub cols: Vec<Col>,
    pub level: Option<LevelDef>,
    pub mode: ShopMode,
    pub final_stats: bool,
}

impl Shop {
    /// init_shop / shop_setup. `players` are (panel, Carried now, Carried at the
    /// level's start) in slot order; `level` is the level just played (its
    /// shop_max scales the tally, its letter picks the music).
    pub fn new(
        fs: &GameFs,
        players: Vec<(PanelInfo, Carried, Carried)>,
        level: Option<LevelDef>,
        mode: ShopMode,
        final_stats: bool,
    ) -> Shop {
        let items = load_items(fs).unwrap_or_else(|e| {
            log::warn!("shop items: {e:#}");
            Vec::new()
        });
        Shop::with_data(items, ShopText::load(fs), players, level, mode, final_stats)
    }

    /// The shop with its data already loaded.
    pub fn with_data(
        items: Vec<ShopItem>,
        text: ShopText,
        players: Vec<(PanelInfo, Carried, Carried)>,
        level: Option<LevelDef>,
        mode: ShopMode,
        final_stats: bool,
    ) -> Shop {
        let n = items.len();
        let cols: Vec<Col> = players
            .into_iter()
            .map(|(info, now, start)| {
                let mut c = Col {
                    rng: 0x2545_F491 ^ (info.slot as u32 + 1).wrapping_mul(0x9E37_79B9),
                    info,
                    now,
                    start,
                    state: 0,
                    timer: 0,
                    cursor: 0,
                    bought: Atts::default(),
                    pile_targ: [0; 3],
                    pile_shown: [0; 3],
                    pile_val: [0; 3],
                    pile_count: [0, 1, 2],
                    pile_vis: [false; 3],
                    pile_grew: [false; 3],
                    tally_done: false,
                    gold_pile: None,
                    scroll: false,
                    top: 0,
                    dim: 0,
                    freeze: 0,
                    last_available: 0,
                    item_y: vec![0; n],
                    first_menu: true,
                    timers: vec![0; n],
                    red: vec![false; n],
                    owned: vec![false; n],
                    buyable: vec![false; n],
                    skip: vec![false; n],
                    repeat: Repeat::default(),
                    inv_mode: 2,
                    inv_t: 0,
                    view: View::None,
                };
                // shop_setup: the cursor bound counts the affordable items, then
                // the owned / buyable / skip flags.
                c.last_available = items
                    .iter()
                    .filter(|i| c.now.gold as i64 >= i.price as i64)
                    .count() as i32;
                rebuild_flags(&mut c, &items);
                c
            })
            .collect();
        let collections = vec![Collection::default(); cols.len()];
        Shop {
            sfx: Vec::new(),
            collections,
            items,
            text,
            cols,
            level,
            mode,
            final_stats,
        }
    }

    /// ShopMusicStart: `S_SHOP_<letter>` for the realm of the level just played
    /// ('T' plays as 'G'; the tower, secret levels and anything from 'L' on use 'A').
    pub fn music(&self) -> Option<String> {
        let l = self
            .level
            .as_ref()
            .and_then(|l| l.name.chars().next())
            .map(|c| c.to_ascii_uppercase())
            .unwrap_or('A');
        let l = if l == 'T' { 'G' } else { l };
        let l = if l >= 'L' { 'A' } else { l };
        Some(format!("S_SHOP_{l}"))
    }

    /// The players' Carried after buying and selling, in the order given.
    pub fn players(&self) -> Vec<Carried> {
        self.cols.iter().map(|c| c.now.clone()).collect()
    }

    /// do_shop for one 60 Hz tick; `inputs` per player in the order given.
    /// True once every player is done.
    pub fn update(&mut self, inputs: &[PlayerInput]) -> bool {
        let mut all_done = true;
        for i in 0..self.cols.len() {
            let input = inputs.get(i).copied().unwrap_or_default();
            if !self.update_col(i, &input) {
                all_done = false;
            }
        }
        all_done
    }

    /// One column's state machine; true when it has left (state 100).
    fn update_col(&mut self, i: usize, input: &PlayerInput) -> bool {
        let (_, dy) = self.cols[i].repeat.edges(input);
        let (down, up) = (dy < 0, dy > 0);
        let a = input.attack_pressed;
        let mode = self.mode;
        let max = self.level.as_ref().map_or([0; 3], |l| l.shop_max);
        let col = &mut self.cols[i];
        let sfx = &mut self.sfx;
        col.view = View::None;
        match col.state {
            0 => {
                col.top = 0;
                col.dim = 0;
                col.freeze = 0;
                init_piles(col, max);
                col.timer = 0;
                col.bought = Atts::default();
                if mode != ShopMode::AfterLevel {
                    reseed_gold(col);
                    col.state = 6;
                } else {
                    col.state = 1;
                }
            }
            1..=3 => {
                col.view = View::Tally;
                show_gold(col);
                if col.tally_done && a {
                    sfx.push(SFX_SELECT.into());
                    col.pile_vis = [false; 3];
                    col.state = 4;
                    col.timer = 0;
                }
            }
            4 => {
                if show_lv(col, false, a, sfx) {
                    col.state = if self.final_stats { 20 } else { 5 };
                }
            }
            5 => {
                reseed_gold(col);
                col.state = 6;
            }
            6 | 7 => {
                if col.state == 6 {
                    if mode == ShopMode::Inventory {
                        col.scroll = false;
                        col.state = 9;
                        return false;
                    }
                    col.scroll = true;
                    col.state = 7;
                }
                col.view = View::Shopping;
                show_piles(col);
                let b = input.b_pressed;
                let x = input.magic_pressed;
                if do_shopping(col, &self.items, (down, up, a, x, b), sfx) {
                    col.scroll = false;
                    col.gold_pile = None;
                    col.state = 8;
                }
            }
            8 => {
                if show_lv(col, true, a, sfx) {
                    col.state = 9;
                }
            }
            9 => {
                if mode == ShopMode::FromMenu {
                    col.state = 100;
                } else {
                    // init_panel_blits, AudioTowerFX(2).
                    col.inv_mode = 0;
                    col.inv_t = 0;
                    sfx.push(SFX_INVENTORY.into());
                    col.state = 10;
                }
            }
            10 => {
                inventory_panel(col);
                if a {
                    sfx.push(SFX_SELECT.into());
                    // init_inventory_panel: fly out.
                    col.inv_mode = 2;
                    col.inv_t = 0;
                    col.state = 11;
                }
            }
            11 => {
                if inventory_panel(col) {
                    col.state = 12;
                }
            }
            12 => col.state = 100,
            20 => {
                if final_stats(col, a, sfx) {
                    col.state = 21;
                }
            }
            _ => {
                col.state = 100;
                return true;
            }
        }
        false
    }
}

/// TextHeightMLines for FONT32.
pub fn text_height(scale: f32, txt: &str) -> i32 {
    (FONT32_HEIGHT * scale).floor() as i32 * txt.split('\n').count() as i32
}

/// The level panel's attributes: (before, now) as fight, armor, magic, speed.
/// "Before" is the class at `old_level` less what was bought here.
pub fn level_atts(c: &Col, old_level: u32) -> ([f32; 4], [f32; 4]) {
    let class = c.info.class;
    let o = stats::atts(class, old_level, c.now.bonus);
    let n = stats::atts(class, c.now.level, c.now.bonus);
    let b = c.bought;
    (
        [
            o.fight - b.fight,
            o.armor - b.armor,
            o.magic - b.magic,
            o.speed - b.speed,
        ],
        [n.fight, n.armor, n.magic, n.speed],
    )
}

/// fn_8009A0AC: this level's gold, kills and experience as pile heights, ranked
/// tallest first.
pub fn init_piles(c: &mut Col, max: [i32; 3]) {
    let range = PILE_BOT - PILE_TOP;
    let kills = |k: &Carried| (k.stats.enemies_killed + k.stats.generators_destroyed) as i64;
    let gold = c.now.gold as i64 - c.start.gold as i64;
    let kill = kills(&c.now) - kills(&c.start);
    let exp = c.now.exp as i64 - c.start.exp as i64;
    let h = |v: i64, m: i32| -> i32 {
        let t = v * range as i64 / (m as i64 + 1);
        if t < PILE_MIN as i64 {
            PILE_MIN
        } else if t > range as i64 {
            range
        } else {
            t as i32
        }
    };
    let (g, k, e) = (h(gold, max[0]), h(kill, max[1]), h(exp, max[2]));
    // ra / rb / rc: the ranks of gold, exp and kills.
    let (ra, rb, rc) = if g >= e && g >= k {
        if e >= k {
            (0, 1, 2)
        } else {
            (0, 2, 1)
        }
    } else if e >= g && e >= k {
        if g >= k {
            (1, 0, 2)
        } else {
            (2, 0, 1)
        }
    } else if g >= e {
        (1, 2, 0)
    } else {
        (2, 1, 0)
    };
    c.pile_count[ra] = 0;
    c.pile_count[rb] = 2;
    c.pile_count[rc] = 1;
    c.pile_shown = [PILE_BASE; 3];
    c.pile_targ[ra] = g;
    c.pile_targ[rb] = e;
    c.pile_targ[rc] = k;
    c.pile_val[ra] = gold as i32;
    c.pile_val[rb] = exp as i32;
    c.pile_val[rc] = kill as i32;
}

/// The rank holding the gold pile (the first pile_count entry that is 0).
pub fn gold_rank(c: &Col) -> usize {
    c.pile_count.iter().position(|&k| k == 0).unwrap_or(0)
}

/// do_shop states 0 (from the menu) and 5: the gold pile starts at its tally
/// height and scales against the gold held now.
pub fn reseed_gold(c: &mut Col) {
    let r = gold_rank(c);
    c.pile_shown[r] = c.pile_targ[r];
    c.pile_targ[r] = c.now.gold as i32;
}

/// show_gold: the piles rise one after another.
pub fn show_gold(c: &mut Col) {
    let mut done = 0;
    for k in 0..3 {
        c.pile_grew[k] = false;
        c.pile_vis[k] = done == 0;
        if done != 0 {
            continue;
        }
        if c.pile_shown[k] < c.pile_targ[k] {
            c.pile_shown[k] = (c.pile_shown[k] + PILE_STEP).min(c.pile_targ[k]);
            done = k + 1;
            c.pile_grew[k] = true;
        } else {
            c.pile_shown[k] = c.pile_targ[k];
        }
    }
    c.tally_done = done == 0;
}

/// show_piles: while shopping the gold pile sinks as gold is spent.
pub fn show_piles(c: &mut Col) {
    let r = gold_rank(c);
    let range = PILE_BOT - PILE_TOP - PILE_BASE;
    let gold = c.now.gold as i64;
    if gold == 0 {
        c.gold_pile = None;
        return;
    }
    let tgt = PILE_BASE + (gold * range as i64 / (c.pile_targ[r] as i64 + 1)) as i32;
    let mut cur = c.pile_shown[r];
    if tgt < cur {
        cur = (cur - PILE_STEP).max(tgt);
    } else {
        cur = tgt;
    }
    c.pile_shown[r] = cur;
    c.gold_pile = Some(cur);
}

/// shop_show_lv: the level-up panel (final = false) or what was bought here
/// (final = true). True when the player continues.
pub fn show_lv(c: &mut Col, final_: bool, a: bool, sfx: &mut Vec<String>) -> bool {
    let t = c.timer & 0xFFFF;
    let old_level = if final_ {
        c.now.level
    } else {
        stats::exp_to_level(c.start.exp)
    };
    if !final_ && old_level == c.now.level {
        return true;
    }
    if c.timer == 0 {
        if !final_ {
            // AudioExp(pad, 1): "<name> has gained a level".
            let col = crate::heroes::COLOURS[c.info.colour.min(3)];
            let class = crate::heroes::CLASSES[c.info.class.min(16)];
            sfx.push(format!(
                "VOICE:S_{col}{class}1+S_HAS+S_GAINEDLEVEL|VOICE:S_HAS+S_GAINEDLEVEL"
            ));
        }
        c.timer += 1;
    }
    if t < 0xF000 {
        c.timer += 1;
    }
    let (old, cur) = level_atts(c, old_level);
    let mut gate = if final_ { 30 } else { 90 };
    gate += 60 * (0..4).filter(|&i| old[i] != cur[i]).count() as u32;
    if !final_ {
        gate += 60;
    }
    let done = t >= gate;
    c.view = View::Level {
        final_,
        t,
        old_level,
        prompt: done && !a,
    };
    if done && a {
        sfx.push(SFX_SELECT.into());
        c.timer = 0;
        return true;
    }
    false
}

/// shop_show_final_stats's timing; true when the player continues.
pub fn final_stats(c: &mut Col, a: bool, sfx: &mut Vec<String>) -> bool {
    let t = c.timer & 0xFFFF;
    if t < 0xF000 {
        c.timer += 1;
    }
    let done = t >= 330;
    c.view = View::Final {
        t,
        prompt: done && !a,
    };
    if done && a {
        sfx.push(SFX_SELECT.into());
        c.timer = 0;
        return true;
    }
    false
}

/// draw_inventory_panel's timing; true once the fly-out has finished.
pub fn inventory_panel(c: &mut Col) -> bool {
    let prog = c.inv_t as f32 / if c.inv_mode == 0 { INV_IN } else { INV_OUT } as f32;
    c.view = View::Inventory {
        mode: c.inv_mode,
        prog,
        prompt: c.inv_mode != 2,
    };
    match c.inv_mode {
        0 => {
            c.inv_t += 1;
            if c.inv_t >= INV_IN {
                c.inv_mode = 1;
            }
            false
        }
        2 => {
            c.inv_t += 1;
            c.inv_t >= INV_OUT
        }
        _ => false,
    }
}

/// AudioFindPlayerSlot: the active power-up slot holding exactly (type, flags).
/// As shipped, the search stops at the first slot that isn't counting down
/// (timeleft <= 0), so slots after an empty or use-counted one are not found.
pub fn find_slot(c: &Carried, kind: u8, flags: u32) -> Option<usize> {
    for (i, s) in c.powerups.slots.iter().enumerate() {
        if s.time <= 0.0 {
            return None;
        }
        if s.flags == flags && s.kind == kind {
            return Some(i);
        }
    }
    None
}

/// Flag 4: the player has the item (a power-up slot, keys or potions).
pub fn owned(c: &Carried, it: &ShopItem) -> bool {
    let (kind, flags) = TYPE_SLOT.get(it.kind as usize).copied().unwrap_or((0, 0));
    if kind != 0 && flags != 0 && find_slot(c, kind, flags).is_some() {
        return true;
    }
    // PlayerItemState.
    match it.kind {
        1 => c.keys != 0,
        3 => !c.potions.is_empty(),
        _ => false,
    }
}

/// calculate_player_shopping_parameters: affordable and not at a limit.
pub fn buyable(col: &Col, it: &ShopItem) -> bool {
    let c = &col.now;
    if (c.gold as i64) < it.price as i64 {
        return false;
    }
    let a = stats::atts(col.info.class, c.level, c.bonus);
    match it.kind {
        1 => c.keys < MAX_KEYS,
        3 => (c.potions.len() as u32) < MAX_POTIONS,
        5 => a.fight < 999.0,
        6 => a.speed < 999.0,
        7 => a.armor < 999.0,
        8 => a.magic < 999.0,
        17 => c.health < max_health(c.level),
        _ => true,
    }
}

/// The per-item flags, rebuilt after every buy and sell (EXIT keeps none).
pub fn rebuild_flags(c: &mut Col, items: &[ShopItem]) {
    for (j, it) in items.iter().enumerate().skip(1) {
        c.owned[j] = owned(&c.now, it);
        c.buyable[j] = buyable(c, it);
        c.skip[j] = !c.buyable[j] && !c.owned[j];
    }
}

/// The cursor's wrap bound: one past the last item that is affordable or owned.
pub fn last_available(c: &Col, items: &[ShopItem]) -> i32 {
    let mut cnt = 0;
    for (j, it) in items.iter().enumerate() {
        if c.now.gold as i64 >= it.price as i64 || owned(&c.now, it) {
            cnt = j as i32;
        }
    }
    cnt + 1
}

/// The next random potion colour (RandInt(4) + 1).
pub fn rand4(c: &mut Col) -> u8 {
    c.rng ^= c.rng << 13;
    c.rng ^= c.rng >> 17;
    c.rng ^= c.rng << 5;
    (c.rng % 4) as u8 + 1
}

/// do_shopping: cursor, buying (A), selling back (X) and B to EXIT. True on EXIT.
pub fn do_shopping(
    c: &mut Col,
    items: &[ShopItem],
    (down, up, a, x, b): (bool, bool, bool, bool, bool),
    sfx: &mut Vec<String>,
) -> bool {
    let n = items.len() as i32;
    if n == 0 {
        return true;
    }
    let mut exit = false;
    let mut speed = if c.freeze != 0 { c.freeze } else { 1 };
    let mut moved = false;
    let mut click = false;
    let mut bought = false;
    let mut paid = false;
    c.dim = c.cursor - c.top;
    // The direction edges stay set for the whole frame, so the cursor keeps
    // moving until it reaches an item that isn't skipped.
    let mut guard = 0;
    loop {
        if down {
            if !click {
                sfx.push(SFX_CLICK[0].into());
                click = true;
            }
            if c.freeze != 0 {
                speed = c.freeze + 1;
            }
            c.cursor += 1;
            if c.cursor >= c.last_available || c.cursor >= n {
                c.cursor = 0;
                moved = true;
                c.dim = 0;
                c.top = 0;
            }
        }
        if up {
            if !click {
                sfx.push(SFX_CLICK[1].into());
                click = true;
            }
            if c.freeze != 0 {
                speed = c.freeze + 1;
            }
            c.cursor -= 1;
            if c.cursor < 0 {
                c.last_available = last_available(c, items);
                c.cursor = c.last_available - 1;
                c.dim = c.cursor;
                c.top = c.dim - 6;
                if c.top < 0 {
                    c.dim -= c.top;
                    c.top = 0;
                }
                moved = true;
            }
        }
        guard += 1;
        if !c.skip[c.cursor as usize] || !(down || up) || guard > 2 * n {
            break;
        }
    }
    let cur = c.cursor as usize;
    let price = items[cur].price;
    let (buy, sell) = if c.freeze == 0 {
        (a, x)
    } else {
        (false, false)
    };
    // X: sell an owned item back for 3/4 of its price.
    if sell && c.owned[cur] {
        let it = &items[cur];
        match it.kind {
            1 => {
                if c.now.keys > 0 {
                    c.now.keys -= 1;
                    bought = true;
                }
            }
            3 => {
                if c.now.potions.pop().is_some() {
                    bought = true;
                }
            }
            k => {
                let (kind, flags) = TYPE_SLOT.get(k as usize).copied().unwrap_or((0, 0));
                if kind != 0 && flags != 0 {
                    if let Some(r) = find_slot(&c.now, kind, flags) {
                        c.now.powerups.slots[r] = Default::default();
                    }
                    bought = true;
                }
            }
        }
        if bought {
            c.now.gold = (c.now.gold as i64 + (price as i64 * 3 / 4)) as u32;
            sfx.push(SFX_BUY.into());
        } else {
            sfx.push(SFX_BUZZER.into());
        }
    }
    if buy {
        if c.now.gold as i64 >= price as i64 {
            paid = true;
            buy_effect(c, &items[cur], &mut paid, &mut exit);
            if paid {
                c.now.gold = (c.now.gold as i64 - price as i64) as u32;
                if c.cursor != 0 {
                    sfx.push(SFX_BUY.into());
                }
            } else {
                sfx.push(SFX_BUZZER.into());
            }
        } else {
            sfx.push(SFX_BUZZER.into());
        }
    } else if b && c.cursor != 0 {
        c.cursor = 0;
        moved = true;
    }
    if paid || bought {
        rebuild_flags(c, items);
        c.timers[cur] = RED_TICKS;
        // Walk back to an item that is affordable and not skipped (EXIT is both).
        while c.cursor > 0 && {
            let k = c.cursor as usize;
            (c.now.gold as i64) < items[k].price as i64 || c.skip[k]
        } {
            c.cursor -= 1;
        }
        if c.cursor < c.top {
            c.top = c.cursor;
            c.last_available = last_available(c, items);
            moved = true;
        }
    }
    if !moved {
        let top = c.top;
        let d = c.cursor - top;
        if d > 6 {
            c.top = d + top - 6;
        } else if d < 0 {
            c.top = top + d;
        } else {
            let v0 = c.dim;
            if v0 < d {
                if v0 >= 3 && (c.last_available - 1) - (top + 6) > 0 {
                    c.top = top + 1;
                }
            } else if v0 > d && v0 <= 3 && top > 0 {
                c.top = top - 1;
            }
        }
    }
    if moved {
        speed = -1;
    }
    c.freeze = write_shop_menu(c, items, speed);
    exit
}

/// do_shopping's buy switch: apply the item. Clears `paid` when the item can't
/// be taken (keys / potions at 9) and sets `exit` for EXIT.
pub fn buy_effect(c: &mut Col, it: &ShopItem, paid: &mut bool, exit: &mut bool) {
    let class = c.info.class;
    match it.kind {
        1 => {
            if c.now.keys < MAX_KEYS {
                c.now.keys += 1;
            } else {
                *paid = false;
            }
        }
        3 => {
            if (c.now.potions.len() as u32) < MAX_POTIONS {
                let kind = rand4(c);
                c.now.potions.push(kind);
            } else {
                *paid = false;
            }
        }
        5 => {
            c.now.bonus.fight += 10.0;
            c.bought.fight += 10.0;
        }
        6 => {
            c.now.bonus.speed += 10.0;
            c.bought.speed += 10.0;
        }
        7 => {
            c.now.bonus.armor += 10.0;
            c.bought.armor += 10.0;
        }
        8 => {
            c.now.bonus.magic += 10.0;
            c.bought.magic += 10.0;
        }
        17 => {
            // heal_player.
            c.now.health = (c.now.health + it.amount as f32).min(max_health(c.now.level));
        }
        k => match powerup_for(k) {
            Some((kind, flags, amount, secs)) => {
                let t = if secs > 0.0 {
                    secs * stats::powerup_time(class)
                } else {
                    secs
                };
                c.now.powerups.add(kind, flags, amount, t);
            }
            None => *exit = true,
        },
    }
}

/// calc_shop_ypos: each item's y relative to the cursor's item.
pub fn calc_ypos(cursor: usize, items: &[ShopItem]) -> Vec<i32> {
    let mut y = 0;
    let mut v = Vec::with_capacity(items.len());
    for it in items {
        v.push(y);
        if it.has_icon() {
            y += 24;
        }
        if !it.desc.is_empty() {
            y += text_height(0.5 * it.scale, &it.desc) + 16;
        }
    }
    let base = v.get(cursor).copied().unwrap_or(0);
    v.iter().map(|y| (y - base).clamp(-1024, 1024)).collect()
}

/// write_shop_menu's movement: centre the cursor in the band (clamped to the
/// list's ends) and scroll each item toward its place at `scroll` px a tick
/// (at least 2; -1 snaps). Returns the speed while still scrolling, else 0.
pub fn write_shop_menu(c: &mut Col, items: &[ShopItem], scroll: i32) -> i32 {
    let mut mv = scroll;
    if mv > 0 && mv < 2 {
        mv = 2;
    }
    if c.first_menu {
        mv = -1;
        c.first_menu = false;
    }
    let rel = calc_ypos(c.cursor as usize, items);
    let mut ybase = MENU_TOP + (MENU_BOT - MENU_TOP) / 2;
    if ybase + rel[0] > MENU_TOP {
        ybase = MENU_TOP - rel[0];
    }
    let last = rel[rel.len() - 1];
    if ybase + last < MENU_BOT {
        ybase = MENU_BOT - last;
    }
    let mut did = false;
    for (j, &r) in rel.iter().enumerate() {
        c.red[j] = c.timers[j] > 0;
        if c.timers[j] > 0 {
            c.timers[j] = (c.timers[j] - 1).max(0);
        }
        let yt = ybase + r;
        let y = c.item_y[j];
        if (y - yt).abs() > 1 {
            c.item_y[j] = if mv < 0 {
                yt
            } else if y > yt {
                did = true;
                (y - mv).max(yt)
            } else {
                did = true;
                (y + mv).min(yt)
            };
        }
    }
    if mv > 0 && did {
        mv
    } else {
        0
    }
}

/// An item's MB alpha (0 opaque .. 255, 256 = hidden) in the band, and whether
/// it asks for the up / down arrows (a non-skipped item outside the band).
pub fn item_alpha(c: &Col, j: usize) -> (i32, bool, bool) {
    let y = c.item_y[j];
    let skip = c.skip[j];
    let (mut a, up, dn) = if y < MENU_TOP - MENU_DIM {
        (256, !skip, false)
    } else if y < MENU_TOP {
        (((MENU_TOP - y) * 510 / MENU_DIM).min(255), !skip, false)
    } else if y > MENU_BOT + MENU_DIM {
        (256, false, !skip)
    } else if y > MENU_BOT {
        (((y - MENU_BOT) * 510 / MENU_DIM).min(255), false, !skip)
    } else {
        (0, false, false)
    };
    if a == 0 && skip {
        a = 160;
    }
    (a, up, dn)
}

/// A fake two-player party for snapshots (`--screen shop`): a warrior who levelled
/// up and a wizard who didn't.
pub fn demo_party() -> Vec<(PanelInfo, Carried, Carried)> {
    let warrior_start = Carried {
        level: 4,
        exp: stats::level_to_exp(4) + 200,
        gold: 300,
        keys: 2,
        potions: vec![1],
        health: 600.0,
        ..Carried::new_character()
    };
    let mut warrior = warrior_start.clone();
    warrior.gold = 1900;
    warrior.exp = stats::level_to_exp(5) + 40;
    warrior.level = 5;
    warrior.health = 420.0;
    warrior.stats.enemies_killed = 140;
    warrior.stats.generators_destroyed = 9;
    warrior.stats.gold_found = 1600;
    warrior.stats.playtime = 2.0 * 3600.0 + 17.0 * 60.0;
    let wizard_start = Carried {
        level: 9,
        exp: stats::level_to_exp(9) + 100,
        gold: 50,
        ..Carried::new_character()
    };
    let mut wizard = wizard_start.clone();
    wizard.gold = 700;
    wizard.exp += 1000;
    wizard.health = 1300.0;
    wizard.stats.enemies_killed = 60;
    wizard.stats.generators_destroyed = 3;
    wizard.powerups.add(5, 0x80000, 0.0, 45.0);
    vec![
        (
            PanelInfo {
                slot: 0,
                name: "SAM".into(),
                colour: 2,
                class: 0,
                hidden: None,
            },
            warrior,
            warrior_start,
        ),
        (
            PanelInfo {
                slot: 1,
                name: "BOB".into(),
                colour: 1,
                class: 2,
                hidden: None,
            },
            wizard,
            wizard_start,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> Option<(GameFs, Vec<ShopItem>)> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game-data");
        let fs = GameFs::open(&root).ok()?;
        let items = load_items(&fs).ok()?;
        Some((fs, items))
    }

    fn shop(items: Vec<ShopItem>, mode: ShopMode) -> Shop {
        Shop::with_data(items, ShopText::default(), demo_party(), None, mode, false)
    }

    fn press(f: impl Fn(&mut PlayerInput)) -> Vec<PlayerInput> {
        let mut i = PlayerInput::default();
        f(&mut i);
        vec![i, PlayerInput::default()]
    }

    fn idle() -> Vec<PlayerInput> {
        vec![PlayerInput::default(); 2]
    }

    /// Run idle ticks until player 0's list has stopped scrolling.
    fn settle(s: &mut Shop) {
        for _ in 0..200 {
            s.update(&idle());
            if s.cols[0].freeze == 0 {
                break;
            }
        }
    }

    /// Down presses (each released again) until the cursor is on `item`.
    fn cursor_to(s: &mut Shop, item: i32) {
        for _ in 0..80 {
            if s.cols[0].cursor == item {
                return;
            }
            s.update(&press(|i| i.stick.y = -1.0));
            s.update(&idle());
            settle(s);
        }
        panic!("cursor never reached item {item}");
    }

    #[test]
    fn items_parse() {
        let Some((_, items)) = data() else {
            eprintln!("game data not found; skipping");
            return;
        };
        assert_eq!(items.len(), 34);
        assert_eq!(items[0].desc, "EXIT");
        assert_eq!(
            (items[2].blit.as_str(), items[2].kind, items[2].price),
            ("SHP_KEY", 1, 100)
        );
        assert_eq!(items[4].amount, 100);
        assert_eq!(items[33].desc, "Add 10 to\nMagic");
        assert_eq!(items.iter().map(|i| i.price).max(), Some(1000));
    }

    #[test]
    fn tally_values() {
        let Some((_, items)) = data() else { return };
        let mut s = shop(items, ShopMode::AfterLevel);
        s.update(&idle());
        let c = &s.cols[0];
        assert_eq!(c.state, 1);
        // Gold 1900 - 300, kills 149, experience 1240 + 40 - 200.
        let mut vals: Vec<(usize, i32)> =
            (0..3).map(|k| (c.pile_count[k], c.pile_val[k])).collect();
        vals.sort();
        let exp = stats::level_to_exp(5) as i32 + 40 - (stats::level_to_exp(4) as i32 + 200);
        assert_eq!(vals, vec![(0, 1600), (1, 149), (2, exp)]);
        // With no level, every scale is 1: all piles at the 208 maximum.
        assert_eq!(c.pile_targ, [208; 3]);
        assert_eq!(c.pile_shown, [PILE_BASE; 3]);
        // A level's maxima scale them, clamped to at least 64; tallest first.
        let lv = LevelDef {
            shop_max: [3200, 10_000, 1000],
            ..Default::default()
        };
        let mut s = Shop::with_data(
            s.items.clone(),
            ShopText::default(),
            demo_party(),
            Some(lv),
            ShopMode::AfterLevel,
            false,
        );
        s.update(&idle());
        let c = &s.cols[0];
        // gold 1600*208/3201 = 103, kills 149*208/10001 = 3 -> 64, exp 1080 -> 208.
        assert_eq!(c.pile_targ, [208, 103, 64]);
        assert_eq!(c.pile_count, [2, 0, 1]);
        // The piles rise one at a time; A only works once all are up.
        s.update(&press(|i| i.attack_pressed = true));
        assert_eq!(s.cols[0].state, 1);
        for _ in 0..400 {
            s.update(&idle());
        }
        assert!(s.cols[0].tally_done);
        assert_eq!(s.cols[0].pile_shown, [208, 103, 64]);
        s.update(&press(|i| i.attack_pressed = true));
        assert_eq!(s.cols[0].state, 4);
        assert_eq!(s.music().as_deref(), Some("S_SHOP_A"));
    }

    #[test]
    fn buy_key_and_cap() {
        let Some((_, items)) = data() else { return };
        let mut s = shop(items, ShopMode::FromMenu);
        s.update(&idle());
        assert_eq!(s.cols[0].state, 6);
        settle(&mut s);
        assert_eq!(s.cols[0].state, 7);
        cursor_to(&mut s, 2);
        let gold = s.cols[0].now.gold;
        s.update(&press(|i| i.attack_pressed = true));
        assert_eq!(s.cols[0].now.keys, 3);
        assert_eq!(s.cols[0].now.gold, gold - 100);
        assert!(s.sfx.iter().any(|x| x == SFX_BUY));
        s.cols[0].now.keys = 8;
        s.cols[0].now.gold = 5000;
        rebuild_flags(&mut s.cols[0], &s.items.clone());
        settle(&mut s);
        s.update(&press(|i| i.attack_pressed = true));
        assert_eq!(s.cols[0].now.keys, 9);
        assert_eq!(s.cols[0].now.gold, 4900);
        // At 9 keys the purchase is refused with the buzzer and costs nothing.
        settle(&mut s);
        assert_eq!(s.cols[0].cursor, 2, "keys stay selectable while owned");
        s.sfx.clear();
        s.update(&press(|i| i.attack_pressed = true));
        assert_eq!(s.cols[0].now.keys, 9);
        assert_eq!(s.cols[0].now.gold, 4900);
        assert!(s.sfx.iter().any(|x| x == SFX_BUZZER));
    }

    #[test]
    fn sell_back_refunds_three_quarters() {
        let Some((_, items)) = data() else { return };
        let mut s = shop(items.clone(), ShopMode::FromMenu);
        s.update(&idle());
        settle(&mut s);
        cursor_to(&mut s, 2);
        let gold = s.cols[0].now.gold;
        s.update(&press(|i| i.magic_pressed = true));
        assert_eq!(s.cols[0].now.keys, 1);
        assert_eq!(s.cols[0].now.gold, gold + 75);
        // The wizard's 3 Way Shot (item 21, 550) sells back for 412.
        settle(&mut s);
        let w = &mut s.cols[1];
        assert!(w.owned[21]);
        w.cursor = 21;
        w.freeze = 0;
        let gold = w.now.gold;
        let mut x = idle();
        x[1].magic_pressed = true;
        s.update(&x);
        let w = &s.cols[1];
        assert_eq!(w.now.gold, gold + 412);
        assert!(w.now.powerups.slots.iter().all(|p| p.time == 0.0));
        assert!(!w.owned[21]);
    }

    #[test]
    fn b_jumps_to_exit_and_exit_finishes() {
        let Some((_, items)) = data() else { return };
        let mut s = shop(items, ShopMode::FromMenu);
        s.update(&idle());
        settle(&mut s);
        cursor_to(&mut s, 4);
        s.update(&press(|i| i.b_pressed = true));
        assert_eq!(s.cols[0].cursor, 0);
        // A on EXIT: the bought-stats panel, then A after 30 ticks: done.
        settle(&mut s);
        s.update(&press(|i| i.attack_pressed = true));
        assert_eq!(s.cols[0].state, 8);
        for _ in 0..40 {
            s.update(&idle());
        }
        s.update(&press(|i| i.attack_pressed = true));
        assert_eq!(s.cols[0].state, 9);
        s.update(&idle());
        assert_eq!(s.cols[0].state, 100);
        // Player 1 is still shopping, on EXIT.
        assert!(!s.update(&idle()));
        let mut a = idle();
        a[1].attack_pressed = true;
        s.update(&a);
        assert_eq!(s.cols[1].state, 8);
        for _ in 0..40 {
            s.update(&idle());
        }
        s.update(&a);
        assert!(!s.update(&idle()));
        assert!(s.update(&idle()));
    }

    #[test]
    fn after_level_runs_to_the_end() {
        let Some((_, items)) = data() else { return };
        let mut s = shop(items, ShopMode::AfterLevel);
        let a = vec![
            PlayerInput {
                attack_pressed: true,
                ..Default::default()
            };
            2
        ];
        let mut done_at = None;
        // Hold nothing for a while, then press A every 30 ticks: tally, level
        // panel, EXIT, bought panel, inventory.
        for t in 0..6000 {
            let inp = if t % 30 == 29 { a.clone() } else { idle() };
            if s.update(&inp) {
                done_at = Some(t);
                break;
            }
        }
        assert!(
            done_at.is_some(),
            "states {:?}",
            s.cols.iter().map(|c| c.state).collect::<Vec<_>>()
        );
        assert!(s.sfx.iter().any(|x| x == SFX_INVENTORY));
        assert!(s.sfx.iter().any(|x| x.contains("S_GAINEDLEVEL")));
        // The levelled-up warrior's voice line, with a fallback.
        assert!(s.sfx.iter().any(|x| x.starts_with("VOICE:S_REDWAR1+S_HAS")));
    }

    #[test]
    fn potion_colours_and_food() {
        let Some((_, items)) = data() else { return };
        let mut s = shop(items, ShopMode::FromMenu);
        s.update(&idle());
        settle(&mut s);
        cursor_to(&mut s, 5);
        s.update(&press(|i| i.attack_pressed = true));
        let p = &s.cols[0].now.potions;
        assert_eq!(p.len(), 2);
        assert!((1..=4).contains(&p[1]));
        // Cherry heals 20.
        settle(&mut s);
        s.cols[0].cursor = 1;
        s.cols[0].freeze = 0;
        let h = s.cols[0].now.health;
        s.update(&press(|i| i.attack_pressed = true));
        assert_eq!(s.cols[0].now.health, h + 20.0);
    }
}
