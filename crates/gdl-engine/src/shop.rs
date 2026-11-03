//! Drawing the between-level shop; its rules are `gdl_sim::shop`.

pub use gdl_sim::shop::*;

use crate::fs::GameFs;
use crate::game::max_health;
use crate::gpu::Gpu;
use crate::hud::{Hud, TexBank};
use crate::screen::Scr;
use crate::ui::{Tex, FONT_32};

/// Icons not in the front-end banks (INVENTORY: hand of death, health vampire,
/// Mikey), for a shop to draw with. Free them with `Hud::free_bank`.
pub fn load_icons(fs: &GameFs, gpu: &Gpu, hud: &mut Hud, shop: &Shop) -> TexBank {
    let missing = shop
        .items
        .iter()
        .any(|i| i.has_icon() && hud.ui.get(&i.blit).is_none());
    if !missing {
        return TexBank::default();
    }
    hud.load_bank(&gpu.device, &gpu.queue, fs, "INVENTORY")
        .unwrap_or_else(|e| {
            log::warn!("shop icons: {e:#}");
            TexBank::default()
        })
}

/// A shop and the textures it draws with.
pub struct ShopView<'a> {
    pub shop: &'a Shop,
    pub bank: &'a TexBank,
}

impl std::ops::Deref for ShopView<'_> {
    type Target = Shop;
    fn deref(&self) -> &Shop {
        self.shop
    }
}

impl ShopView<'_> {
    /// Draw every column on the 512x384 screen.
    pub fn draw(&self, s: &mut Scr) {
        for slot in 0..4 {
            let col = self.cols.iter().find(|c| c.info.slot == slot);
            self.draw_column(s, slot, col);
        }
    }

    fn tex(&self, s: &Scr, name: &str) -> Option<Tex> {
        s.ui.get(name).or_else(|| self.bank.get(name))
    }

    fn draw_column(&self, s: &mut Scr, slot: usize, col: Option<&Col>) {
        let x = COL_X[slot] as f32;
        if let Some(t) = s.ui.get(&format!("S1_PLYR{}", slot + 1)) {
            s.blit(t, x, 0.0, 128.0, t.h as f32, 0);
        }
        if let Some(t) = s.ui.get(&format!("S2_PLYR{}", slot + 1)) {
            s.blit(t, x, 256.0, 128.0, t.h as f32, 0);
        }
        if let Some(c) = col {
            self.draw_blits(s, slot, c);
        }
        for (name, y) in [("S1_BORDER", 0.0), ("S2_BORDER", 256.0)] {
            if let Some(t) = s.ui.get(name) {
                s.blit_shimmer(t, x, y, 128.0, t.h as f32, 0);
            }
        }
        let Some(c) = col else { return };
        let top = format!("SHOP_TOP_{}", crate::select::COLOURS[c.info.colour.min(3)]);
        s.blit_named(&top, x + 32.0, 0.0, 0);
        match c.view {
            View::None => {}
            View::Tally => self.draw_tally(s, slot, c),
            View::Level {
                final_,
                t,
                old_level,
                prompt,
            } => self.draw_level(s, slot, c, final_, t, old_level, prompt),
            View::Shopping => self.draw_menu(s, slot, c),
            View::Inventory { mode, prog, prompt } => {
                self.draw_inventory(s, slot, c, mode, prog, prompt)
            }
            View::Final { t, prompt } => draw_final(s, slot, c, t, prompt),
        }
    }

    /// The blits behind the column border: piles, scroll and item icons.
    fn draw_blits(&self, s: &mut Scr, slot: usize, c: &Col) {
        let x = COL_X[slot] as f32;
        let pile = |s: &mut Scr, kind: usize, h: i32| {
            if let Some(t) = s.ui.get(PILE_TEX[kind]) {
                let h = h as f32;
                s.blit_uv(
                    t,
                    x,
                    (PILE_BOT as f32) - h,
                    128.0,
                    h,
                    [0.0, 0.0, 1.0, h / 256.0],
                    0,
                );
            }
        };
        if c.view == View::Tally {
            for k in 0..3 {
                if c.pile_vis[k] {
                    pile(s, c.pile_count[k], c.pile_shown[k]);
                }
            }
        }
        if let Some(h) = c.gold_pile {
            pile(s, 0, h);
        }
        if c.scroll {
            s.blit_named("SHOP_SCROLL_1", x, 0.0, 0);
            s.blit_named("SHOP_SCROLL_2", x, 256.0, 0);
            for (j, it) in self.items.iter().enumerate() {
                let a = item_alpha(c, j).0;
                if a >= 256 || !it.has_icon() {
                    continue;
                }
                if let Some(t) = self.tex(s, &it.blit) {
                    s.blit(t, x + 20.0, c.item_y[j] as f32, t.w as f32, t.h as f32, a);
                }
            }
        }
    }

    fn draw_tally(&self, s: &mut Scr, slot: usize, c: &Col) {
        let x = COL_X[slot] as f32;
        for k in 0..3 {
            let kind = c.pile_count[k];
            let label = format!("{}: {}", PILE_LABEL[kind], c.pile_val[k]);
            let y = PILE_LABEL_Y[kind] as f32;
            if c.pile_grew[k] {
                s.glow_text(0.5, x + 16.0, y, &label);
            } else {
                s.text(0.5, x + 16.0, y, FONT_32, 0xFFFFFF, 0, &label);
            }
        }
        if c.tally_done {
            let y = CONTINUE_Y as f32;
            s.glow_text(0.5, x + 32.0, y, " Continue");
            if let Some(t) = s.ui.get(BUTTON_A) {
                s.blit(t, x + 16.0, y - 3.0, 20.0, 20.0, 0);
            }
        }
        header(s, slot, "Stats");
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_level(
        &self,
        s: &mut Scr,
        slot: usize,
        c: &Col,
        final_: bool,
        t: u32,
        old_level: u32,
        prompt: bool,
    ) {
        let (x1, x2) = ((COL_X[slot] + 8) as f32, (COL_X[slot] + 88) as f32);
        let xc = -(COL_CENTRE[slot] as f32);
        let k = 0.48;
        let level = c.now.level;
        let s_level = format!("Level {level}");
        if !final_ {
            s.glow_text(0.75, xc, 32.0, &s_level);
        } else {
            s.text(0.75, xc, 32.0, FONT_32, 0xFFFFFF, 0, &s_level);
        }
        let rank = self.text.rank(c.info.class, level);
        s.text(0.6, xc, 64.0, FONT_32, 0xFFFFFF, 0, &rank);
        let (old, cur) = level_atts(c, old_level);
        let mut gate = if final_ { 30 } else { 90 };
        for (row, label) in ["Strength", "Armor", "Magic", "Speed"].iter().enumerate() {
            let y = 96.0 + 20.0 * row as f32;
            let chg = old[row] != cur[row];
            if chg && t > gate && t < gate + 60 {
                s.glow_text(k, x1, y, label);
            } else {
                s.text(k, x1, y, FONT_32, 0xFFFFFF, 0, label);
            }
            if chg && t > gate {
                s.glow_text(k, x2, y, &(cur[row] as i32).to_string());
            } else {
                s.text(
                    k,
                    x2,
                    y,
                    FONT_32,
                    0xFFFFFF,
                    0,
                    &(old[row] as i32).to_string(),
                );
            }
            if chg {
                gate += 60;
            }
        }
        let health = !final_;
        if health && t > gate && t < gate + 60 {
            s.glow_text(k, x1, 188.0, "Max");
            s.glow_text(k, x1, 204.0, "Health");
        } else {
            s.text(k, x1, 188.0, FONT_32, 0xFFFFFF, 0, "Max");
            s.text(k, x1, 204.0, FONT_32, 0xFFFFFF, 0, "Health");
        }
        let max = max_health(level) as i32;
        if health && t > gate {
            s.glow_text(k, x2, 196.0, &max.to_string());
        } else {
            let v = if final_ {
                max
            } else {
                max - HEALTH_PER_LEVEL * (level as i32 - old_level as i32)
            };
            s.text(k, x2, 196.0, FONT_32, 0xFFFFFF, 0, &v.to_string());
        }
        if !final_ && (level == 25 || level == 50) {
            // DrawStringText(184 / 185, char_type): the message's own scale.
            let msg = &self.text.magic[(level == 50) as usize];
            if let Some(m) = msg.get(c.info.class & 3) {
                text_lines(s, 0.45, xc, 224.0, 0xFF80C0, 0, m);
            }
        }
        if prompt {
            continue_prompt(s, x1 + 8.0, x1 + 32.0);
        }
        header(s, slot, "Stats");
    }

    fn draw_menu(&self, s: &mut Scr, slot: usize, c: &Col) {
        let xr = (COL_RIGHT[slot] - 64) as f32;
        let xc = COL_CENTRE[slot] as f32;
        let (mut up, mut dn) = (false, false);
        for (j, it) in self.items.iter().enumerate() {
            let (a, u, d) = item_alpha(c, j);
            up |= u;
            dn |= d;
            if a >= 256 {
                continue;
            }
            let sel = j as i32 == c.cursor;
            let hl = if c.red[j] { 0xFF0000 } else { 0 };
            let y = c.item_y[j] as f32;
            if it.price > 0 {
                let owned = c.owned[j];
                let line = |s: &mut Scr, y: f32, txt: String| {
                    if sel {
                        s.glow_text(0.5, xr, y, &txt);
                    } else {
                        s.text(0.5, xr, y, FONT_32, hl, a, &txt);
                    }
                };
                line(
                    s,
                    if owned { y - 6.0 } else { y + 12.0 },
                    format!("B:{}", it.price),
                );
                if owned {
                    line(s, y + 12.0, format!("S:{}", it.price * 3 / 4));
                }
            }
            let y = y + if it.has_icon() { 32.0 } else { 12.0 };
            if !it.desc.is_empty() {
                let scale = 0.5 * it.scale;
                if sel {
                    s.glow_text_lines(scale, -xc, y, &it.desc);
                } else {
                    text_lines(s, scale, -xc, y, hl, a, &it.desc);
                }
            }
        }
        if up {
            s.blit_named("MORE_UP", xc - 32.0, (MENU_TOP - 40) as f32, 0);
        }
        if dn {
            s.blit_named("MORE_DOWN", xc - 32.0, (MENU_BOT + 56) as f32, 0);
        }
        header(s, slot, "Shop");
    }

    fn draw_inventory(
        &self,
        s: &mut Scr,
        slot: usize,
        c: &Col,
        mode: u32,
        prog: f32,
        prompt: bool,
    ) {
        let xoff = (slot as i32) << 7;
        let coll = self
            .cols
            .iter()
            .position(|k| std::ptr::eq(k, c))
            .and_then(|i| self.collections.get(i))
            .cloned()
            .unwrap_or_default();
        let mut pieces: Vec<(&Piece, String)> = Vec::new();
        for p in &PANEL_H {
            if let Some(n) = p.name {
                pieces.push((p, n.to_string()));
            }
        }
        for (j, p) in PANEL_F.iter().enumerate() {
            if let Some(n) = p.name {
                let suffix = if coll.legend[j] { "" } else { "_EMPTY" };
                pieces.push((p, format!("{n}{suffix}")));
            }
        }
        for (j, p) in PANEL_E.iter().enumerate() {
            if let (Some(n), true) = (p.name, coll.runes[j]) {
                pieces.push((p, n.to_string()));
            }
        }
        for (p, name) in pieces {
            if let Some(t) = self.tex(s, &name) {
                panel_piece(s, t, p, xoff, mode, prog);
            }
        }
        // print_n_of_m's font alpha.
        let alpha = match mode {
            0 => (255.0 * (1.0 - prog)) as i32,
            2 => (255.0 * prog) as i32,
            _ => 0,
        };
        let counts = [(coll.fangs, 12), (coll.feathers, 20), (coll.claws, 28)]
            .into_iter()
            .chain(
                coll.crystals
                    .iter()
                    .zip(CRYSTALS_NEEDED)
                    .map(|(&n, m)| (n, m)),
            );
        for (style, (n, m)) in counts.enumerate() {
            let p = &PANEL_H[style + 1];
            if p.scale <= 0.0 {
                continue;
            }
            let n = if n < 0 || n > m { m } else { n };
            let xr = (xoff + p.x + p.dx) as f32;
            let y = (p.y + p.dy) as f32;
            let ns = n.to_string();
            let w = s.text_width(FONT_32, p.scale, &ns);
            s.text(p.scale, xr - w, y, FONT_32, 0xFFFFFF, alpha, &ns);
            s.text(p.scale, xr, y, FONT_32, 0xFFFFFF, alpha, &format!("/{m}"));
        }
        if prompt {
            continue_prompt(s, (xoff + 16) as f32, (xoff + 40) as f32);
        }
        header(s, slot, "Inv");
    }
}

/// The column header: DrawTextKeepScale(0.45, -centre, 8, font 6, black).
fn header(s: &mut Scr, slot: usize, txt: &str) {
    s.text(0.45, -(COL_CENTRE[slot] as f32), 8.0, FONT_32, 0, 0, txt);
}

/// The A icon (16x16) and a glowing "Continue" at y 280.
fn continue_prompt(s: &mut Scr, icon_x: f32, text_x: f32) {
    if let Some(t) = s.ui.get(BUTTON_A) {
        s.blit(t, icon_x, 280.0, 16.0, 16.0, 0);
    }
    s.glow_text(0.5, text_x, 280.0, "Continue");
}

/// DrawTextMLines: one line per `\n`, FontHeight apart.
fn text_lines(s: &mut Scr, scale: f32, x: f32, y: f32, color: u32, alpha: i32, txt: &str) {
    let lh = s.font_height(FONT_32, scale);
    for (i, l) in txt.split('\n').enumerate() {
        s.text(scale, x, y + lh * i as f32, FONT_32, color, alpha, l);
    }
}

/// animate_panel_piece: fly in from a spiral (mode 0), sit (1), fly out (2).
fn panel_piece(s: &mut Scr, t: Tex, p: &Piece, xoff: i32, mode: u32, prog: f32) {
    let (w, h) = (t.w as f32, t.h as f32);
    let (x, y, w, h, fade) = match mode {
        0 => {
            let p2 = prog * prog;
            let idx = (p.y + p.x + (60.0 * (3.0 * (p2 * prog))) as i32).rem_euclid(60);
            let ang = (idx as f32 * 6.0).to_radians();
            let f = 1.0 - prog;
            let x = ((p.x + xoff) as f32 + f * (60.0 * ang.cos())) as i32;
            let y = (p.y as f32 + f * (60.0 * ang.sin())) as i32;
            let k = 1.0 - p2;
            let (w, h) = ((w * k + w) as i32, (h * k + h) as i32);
            (x, y, w, h, (255.0 * k) as i32)
        }
        2 => {
            let dx = (p.x - 64) as f32 * prog;
            let dy = (p.y - 180) as f32 * prog;
            let x = ((p.x + xoff) as f32 + 10.0 * dx) as i32;
            let y = (p.y as f32 + 10.0 * dy) as i32;
            let (w, h) = ((w + 5.0 * (w * prog)) as i32, (h + 5.0 * (h * prog)) as i32);
            (x, y, w, h, (255.0 * prog) as i32)
        }
        _ => (p.x + xoff, p.y, w as i32, h as i32, 0),
    };
    s.blit(t, x as f32, y as f32, w as f32, h as f32, fade);
}

/// shop_show_final_stats's drawing.
fn draw_final(s: &mut Scr, slot: usize, c: &Col, t: u32, prompt: bool) {
    let xc = -(COL_CENTRE[slot] as f32);
    let k = 0.48;
    let st = &c.now.stats;
    let label = |s: &mut Scr, glow: bool, y: f32, txt: &str| {
        if glow {
            s.glow_text(k, xc, y, txt);
        } else {
            s.text(k, xc, y, FONT_32, 0xFFFFFF, 0, txt);
        }
    };
    s.glow_text(0.56, xc, 32.0, "Final Stats");
    label(s, t > 90 && t < 150, 60.0, "Enemies Killed");
    if t > 90 {
        s.glow_text(k, xc, 78.0, &st.enemies_killed.to_string());
    }
    let g = t > 150 && t < 210;
    label(s, g, 98.0, "Generators");
    label(s, g, 116.0, "Destroyed");
    if t > 150 {
        s.glow_text(k, xc, 134.0, &st.generators_destroyed.to_string());
    }
    label(s, t > 210 && t < 270, 154.0, "Gold Found");
    if t > 210 {
        s.glow_text(k, xc, 172.0, &st.gold_found.to_string());
    }
    label(s, t > 270 && t < 330, 192.0, "Total Playtime");
    if t > 270 {
        // Playtime in ticks: 5,184,000 a day, 216,000 an hour, 3,600 a minute.
        let mut ticks = st.playtime as f64 * 60.0;
        let days = (ticks / 5_184_000.0) as i32;
        ticks -= 5_184_000.0 * days as f64;
        let hours = (ticks / 216_000.0) as i32;
        ticks -= 216_000.0 * hours as f64;
        let mins = (ticks / 3600.0) as i32;
        s.glow_text(k, xc, 210.0, &format!("{days} Days"));
        s.glow_text(k, xc, 228.0, &format!("{hours} Hours"));
        s.glow_text(k, xc, 246.0, &format!("{mins} Minutes"));
    }
    if prompt {
        let x8 = (COL_X[slot] + 8) as f32;
        continue_prompt(s, x8 + 8.0, x8 + 32.0);
    }
    header(s, slot, "Stats");
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

    #[test]
    fn rank_titles_and_sounds_exist() {
        let Some((fs, _)) = data() else { return };
        let t = ShopText::load(&fs);
        assert_eq!(t.rank(0, 5), "Warrior");
        assert_eq!(t.rank(0, 15), "Fighter");
        assert_eq!(t.rank(0, 99), "Legend");
        assert!(t.magic[0][0].starts_with("Magic Potions"));
        let names: Vec<String> = [
            SFX_SELECT,
            SFX_BUZZER,
            SFX_CLICK[0],
            SFX_CLICK[1],
            SFX_BUY,
            SFX_INVENTORY,
            "S_HAS",
            "S_GAINEDLEVEL",
            "S_REDWAR1",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        for (n, found) in crate::audio::check_sounds(&fs, &names) {
            assert!(found.is_some(), "missing sound {n}");
        }
        // The music is a sequence call in SHOP_<letter>.VBK (play_bank_music).
        let rom = fs.read("AUDIO/AUDATPS2.ROM").unwrap();
        let table = gdl_formats::audio::parse_sound_names(&rom);
        for l in 'A'..='K' {
            let (bank, _, _) = table[&format!("S_SHOP_{l}")];
            assert_eq!(gdl_formats::audio::BANKS[bank], format!("SHOP_{l}"));
            assert!(fs.exists(&format!("AUDIO/SHOP_{l}.VBK")));
        }
    }
}
