//! Secret characters (world/tower.c towerAwardWorldRunes, items.c fn_8005DE50
//! case 1): every gold piece picked up in a secret level counts for each hero
//! in play; once a hero's count reaches the level's gold pieces, every hero in
//! play unlocks that level's secret character.

use super::{Game, ScrollBox};

/// lbl_80124DA0: the class each secret level (by level index) unlocks; 16 is
/// Sumner.
pub const SECRET_CLASS: [usize; 9] = [10, 11, 9, 8, 16, 12, 15, 14, 13];

/// lbl_8034858C: seconds the coin counter stays up after a pickup.
const COIN_SHOW: f32 = 60.0;

impl Game {
    /// towerAwardWorldRunes: true when the coins are all in and the class has
    /// been unlocked.
    pub(crate) fn tower_award_world_runes(&mut self) -> bool {
        let Some(&class) = self
            .wave
            .and_then(|w| SECRET_CLASS.get((w & 0xFF) as usize))
        else {
            return false;
        };
        let mut awarded = false;
        for p in self.players.iter_mut().filter(|p| p.state.active()) {
            p.coins += 1;
            p.coin_timer = COIN_SHOW;
            if p.coins >= self.visible_coins {
                awarded = true;
            }
        }
        if awarded {
            for p in self.players.iter_mut().filter(|p| p.state.active()) {
                p.class_unlock |= 1 << (class - 8);
            }
        }
        awarded
    }

    /// The gold pickup's secret-realm branch: on the award, the "AllCoins"
    /// message box with S_SECRETCHAR (fn_8009FB30), and the hourglass jumps to
    /// its last second (lbl_80344818 = 1.0) without its tally sounds.
    pub(super) fn secret_coin(&mut self) {
        if !self.tower_award_world_runes() {
            return;
        }
        if let Some(m) = self.scroll_rom.as_ref().and_then(|r| r.named("AllCoins")) {
            self.scroll_box = Some(ScrollBox::new(None, m.lines.clone(), m.scale));
        }
        // sndFxQueAddEx: queued like an announcement (no good wizard here).
        self.sfx.push("VOICE:S_SECRETCHAR".into());
        if let Some((_, left)) = &mut self.secret_timer {
            *left = 1.0;
        }
    }

    /// The coin counter a hero's panel shows (do_got_it, got_type >= 0x200):
    /// the 16_<CLS>COIN (16_SUM) icon and found / placed.
    pub fn coin_counter(&self, pi: usize) -> Option<(String, u32, u32)> {
        let p = self.players.get(pi)?;
        if p.coin_timer <= 0.0 {
            return None;
        }
        let class = *SECRET_CLASS.get((self.wave? & 0xFF) as usize)?;
        let icon = if class < 16 {
            format!("16_{}COIN", crate::select::CLASSES[class])
        } else {
            "16_SUM".to_string()
        };
        Some((icon, p.coins, self.visible_coins))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::GameFs;

    fn data() -> Option<GameFs> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game-data");
        GameFs::open(&root).ok()
    }

    #[test]
    fn secret_classes_come_from_the_dol() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game-data");
        let Ok(dol) = std::fs::read(root.join("main.dol")) else {
            return;
        };
        let dol = gdl_formats::dol::Dol::new(dol);
        for (i, &c) in SECRET_CLASS.iter().enumerate() {
            assert_eq!(dol.u32_at(0x8012_4DA0 + 4 * i as u32), Some(c as u32));
        }
        assert_eq!(dol.f32_at(0x8034_858C), Some(COIN_SHOW));
    }

    /// Every gold piece counts for every hero in play; the last one unlocks
    /// the level's class for all of them.
    #[test]
    fn all_coins_unlock_the_secret_character() {
        let Some(fs) = data() else { return };
        let mut g = Game::new(&fs, "levelS1", &[("WAR", "RED"), ("VAL", "BLU")]).unwrap();
        let total = g.visible_coins;
        assert!(total > 0, "S1 has gold");
        for n in 1..total {
            g.secret_coin();
            assert_eq!(g.players[0].coins, n);
            assert_eq!(g.players[1].coins, n);
            assert_eq!(g.players[0].class_unlock, 0);
        }
        assert_eq!(
            g.coin_counter(1),
            Some(("16_JACCOIN".to_string(), total - 1, total))
        );
        g.secret_coin();
        // S1 unlocks the Jackal (class 10).
        assert_eq!(g.players[0].class_unlock, 1 << 2);
        assert_eq!(g.players[1].class_unlock, 1 << 2);
        assert!(g.scroll_box.is_some(), "the AllCoins message");
        assert!(g.sfx.iter().any(|s| s == "VOICE:S_SECRETCHAR"));
        assert_eq!(g.secret_timer.map(|t| t.1), Some(1.0));
        // S5 is Sumner's.
        let mut g = Game::new(&fs, "levelS5", &[("WAR", "RED")]).unwrap();
        for _ in 0..g.visible_coins {
            g.secret_coin();
        }
        assert_eq!(g.players[0].class_unlock, 0x100);
        assert_eq!(g.coin_counter(0).unwrap().0, "16_SUM");
    }

    /// do_got_it's icons come from the secret levels' item bank.
    #[test]
    fn coin_icons_exist() {
        let Some(fs) = data() else { return };
        let imgs = crate::hud::coin_images(&fs);
        for c in SECRET_CLASS {
            let name = if c < 16 {
                format!("16_{}COIN", crate::select::CLASSES[c])
            } else {
                "16_SUM".into()
            };
            assert!(imgs.iter().any(|(n, _)| *n == name), "{name}");
        }
    }
}
