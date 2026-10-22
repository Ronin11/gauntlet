//! The party's progress through the realms (tower.c): levels beaten, boss runes,
//! shards and crystals, which decide which tower portals are open.

use gdl_formats::wdata::TOWER_WAVE;

/// crystal_order (0x80124D14): realm types in unlock order. The index is also the
/// tower start locator used when coming back from that realm.
pub const CRYSTAL_ORDER: [u32; 12] = [13, 7, 2, 1, 11, 4, 3, 9, 10, 5, 6, 8];

#[derive(Clone, Debug, Default)]
pub struct Progress {
    /// Per realm type, a bit per level index beaten (towerRecordLevelBeaten).
    pub beaten: [u16; 16],
    /// Boss runes (bit = position in CRYSTAL_ORDER) and the 13 shards.
    pub runes: u16,
    pub shards: u16,
    /// Crystals found per colour, -1 once a colour is done.
    pub crystals: [i32; 9],
    /// Gargoyle items (fangs, feathers, claws), -1 once their gate is open.
    pub garg_items: [i32; 3],
    /// The last realm other than the tower that was played (lbl_803448C8).
    pub last_realm: Option<u32>,
    /// The last level beaten (lbl_803448A8/AC); Sumner clears it once he has
    /// spoken about the 13th runestone.
    pub last_beaten: Option<u32>,
    /// What Sumner has banked (save.rune_stones2 / rune_stones): new bits in
    /// `runes` / `shards` start his speeches.
    pub banked_runes: u16,
    pub banked_shards: u16,
    /// EnterTower has run this session (lbl_80344C4C).
    pub entered_tower: bool,
    /// boss_attempt1/2: realms (bit = realm type) whose Guardian has killed a
    /// player once / twice.
    pub boss_attempts: [u16; 2],
    /// level_masks: Runestone levels beaten once / twice (bit = rune - 1) and
    /// legend-item levels beaten once / twice (bit = the item's realm).
    pub level_masks: [u16; 4],
    /// Items of Legend held (bit = realm).
    pub legend_items: u16,
    /// Sumner's hint pickers.
    pub hints: crate::hints::HintState,
}

/// lbl_80124D4C: the crystal colour that opens each realm type (0 = none).
const REALM_CRYSTAL: [usize; 16] = [0, 3, 2, 6, 5, 0, 0, 1, 0, 7, 8, 4, 0, 0, 0, 0];

impl Progress {
    /// WorldOpen for the realms gated by runes and shards.
    pub fn world_open(&self, realm: u32) -> bool {
        match realm {
            5 => self.runes & 0x1FE == 0x1FE,
            6 => self.runes & 0x3FE == 0x3FE && self.shards & 0xFFF == 0xFFF,
            8 => self.runes & 0x7FE == 0x7FE,
            _ => true,
        }
    }

    /// Is the tower portal to `wave` open? A realm's first level always is; each
    /// later one opens when the one before it is beaten. Temple, Hell and Battle
    /// also need their runes, and Battle's Citadel all 13 shards.
    pub fn portal_open(&self, wave: u32) -> bool {
        let (realm, gate) = (wave >> 8, wave & 0xFF);
        let prev_beaten = gate == 0 || self.beaten[realm as usize & 15] & (1 << (gate - 1)) != 0;
        match realm {
            5 | 6 => self.world_open(realm),
            8 if gate == 3 => self.world_open(8) && self.shards & 0x1FFF == 0x1FFF,
            8 => self.world_open(8) && prev_beaten,
            _ => prev_beaten,
        }
    }

    /// WorldOpen for any realm: the rune realms by runes and shards, the rest
    /// by their crystal colour (used by the hints).
    pub fn world_open_any(&self, realm: u32, crystals: &[i32; 9]) -> bool {
        match realm {
            5 | 6 | 8 => self.world_open(realm),
            13 => true,
            r => crate::game::crystals_met(crystals, REALM_CRYSTAL[r as usize & 15]),
        }
    }

    /// towerRecordLevelBeaten: the level's bit, and its Runestone / legend-item
    /// masks (a second win sets the "twice" mask).
    pub fn record_beaten(&mut self, wave: u32, rune: i16, legend: i16) {
        if wave == TOWER_WAVE {
            return;
        }
        self.beaten[(wave >> 8) as usize & 15] |= 1 << (wave & 0xFF);
        self.last_beaten = Some(wave);
        let twice = |masks: &mut [u16], bit: u16| {
            if masks[0] & bit != 0 {
                masks[1] |= bit;
            } else {
                masks[0] |= bit;
            }
        };
        if (1..=13).contains(&rune) {
            twice(&mut self.level_masks[0..2], 1 << (rune - 1));
        }
        if (1..=15).contains(&legend) {
            twice(&mut self.level_masks[2..4], 1 << legend);
        }
    }

    /// playerGiveGargItem on a boss level: a death there is an attempt.
    pub fn record_boss_death(&mut self, realm: u32) {
        let bit = 1 << (realm & 15);
        if self.boss_attempts[0] & bit != 0 {
            self.boss_attempts[1] |= bit;
        } else {
            self.boss_attempts[0] |= bit;
        }
    }

    /// The tower start locator: the portal ring of the last realm played.
    pub fn tower_start(&self) -> u16 {
        self.last_realm
            .and_then(|r| CRYSTAL_ORDER.iter().position(|&c| c == r))
            .unwrap_or(0) as u16
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portals_open_one_level_at_a_time() {
        let mut p = Progress::default();
        assert!(p.portal_open(0x100));
        assert!(!p.portal_open(0x101));
        p.record_beaten(0x100, 0, 0);
        assert!(p.portal_open(0x101));
        assert!(!p.portal_open(0x500), "the temple needs the 8 boss runes");
        p.runes = 0x1FE;
        assert!(p.portal_open(0x500));
        p.last_realm = Some(1);
        assert_eq!(p.tower_start(), 3);
    }
}
