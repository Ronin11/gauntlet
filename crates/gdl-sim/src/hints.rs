//! Sumner's hint menu (ui/options.c HintMenu and the next_*_hint pickers): a
//! hint about the realms, their Guardians, the Items of Legend and the
//! Runestones, from HINTS_E.ROM. More lines of a hint unlock after the party has
//! failed at that Guardian (or beaten that level) once and twice.

use crate::progress::CRYSTAL_ORDER;
use gdl_formats::text::TextRom;

/// rune_idx_table (0x8011F9F8): the Runestones in realm order.
const RUNE_ORDER: [u16; 13] = [8, 9, 4, 1, 2, 3, 10, 7, 5, 6, 11, 12, 13];

/// Hint menu item codes (OPT_HINTS_*).
pub const GENERAL: i32 = 39;
pub const BOSS: i32 = 40;
pub const LEGEND: i32 = 41;
pub const RUNE: i32 = 42;

/// The pickers' positions; the general one lasts the session (hints_inited).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HintState {
    inited: bool,
    general_index: i32,
    general_num: i32,
    boss_index: i32,
    boss_num: i32,
    boss_pass: i32,
    legend_index: i32,
    legend_num: i32,
    legend_pass: i32,
    rune_index: i32,
    rune_num: i32,
    rune_pass: i32,
}

impl Default for HintState {
    fn default() -> Self {
        HintState {
            inited: false,
            general_index: -1,
            general_num: -1,
            boss_index: -1,
            boss_num: 0,
            boss_pass: 1,
            legend_index: -1,
            legend_num: 0,
            legend_pass: 1,
            rune_index: -1,
            rune_num: 0,
            rune_pass: 1,
        }
    }
}

/// What the pickers ask about the party.
#[derive(Clone, Copy, Debug, Default)]
pub struct HintFacts {
    /// WorldOpen per realm type.
    pub world_open: [bool; 16],
    /// Boss shards (bits in world order) and Runestones (13 bits).
    pub runes: u16,
    pub shards: u16,
    /// Realms (bit = realm type) whose Guardian beat a player once / twice.
    pub boss_attempts: [u16; 2],
    /// level_masks: Runestone levels beaten once / twice (bit = rune - 1),
    /// legend-item levels beaten once / twice (bit = realm type).
    pub level_masks: [u16; 4],
    /// Realms whose Item of Legend a player holds (towerGetRuneNearStat).
    pub legend_held: u16,
    /// The gargoyle sections unlocked (towerAllPlayersMetLevelReq).
    pub sections: [bool; 3],
}

pub struct Hints {
    pub state: HintState,
    facts: HintFacts,
    rom: Option<TextRom>,
    /// hint_submenu: the category on display.
    pub sub: i32,
}

impl Hints {
    /// HintMenu: the general picker starts once a session; the others restart.
    pub fn new(state: HintState, facts: HintFacts, rom: Option<TextRom>) -> Hints {
        let mut h = Hints {
            state,
            facts,
            rom,
            sub: 0,
        };
        if !h.state.inited {
            h.state.inited = true;
            h.state.general_index = -1;
            h.state.general_num = -1;
        }
        h.state.boss_index = 0;
        h.state.boss_num = 0;
        h.state.legend_index = 0;
        h.state.legend_num = 0;
        h.state.rune_index = -1;
        h.state.rune_num = 0;
        h
    }

    fn msg(&self, idx: usize) -> Option<&gdl_formats::text::Message> {
        self.rom.as_ref()?.messages.get(idx)
    }

    /// Message index of string list `list`, entry `i`.
    fn list_msg(&self, list: usize, i: usize) -> Option<&gdl_formats::text::Message> {
        let m = *self.rom.as_ref()?.lists.get(list)?.get(i)?;
        self.msg(m as usize)
    }

    fn line(&self, msg: usize, line: usize) -> String {
        self.msg(msg)
            .and_then(|m| m.lines.get(line).cloned())
            .unwrap_or_default()
    }

    /// Choose a category: advance its picker; the display's title.
    pub fn pick(&mut self, code: i32) -> String {
        self.sub = code;
        match code {
            GENERAL => {
                self.next_general();
                "A Hint for You".into()
            }
            BOSS => {
                self.next_boss();
                self.line(0xC, self.state.boss_index.max(0) as usize)
            }
            LEGEND => {
                self.next_legend();
                self.line(0x28, self.state.legend_index.max(0) as usize)
            }
            _ => {
                self.next_rune();
                let r = RUNE_ORDER[self.state.rune_index.clamp(0, 12) as usize] as usize;
                self.line(0x1B, r - 1)
            }
        }
    }

    /// The hint's text blocks, the gap after each (16, or 0 for runes), and
    /// the message's own text scale.
    pub fn body(&self) -> (Vec<String>, f32, f32) {
        let st = &self.state;
        let take = |m: Option<&gdl_formats::text::Message>, n: i32| -> (Vec<String>, f32) {
            m.map_or((Vec::new(), 1.0), |m| {
                (
                    m.lines.iter().take(n.max(0) as usize).cloned().collect(),
                    m.scale,
                )
            })
        };
        let ((blocks, scale), gap) = match self.sub {
            GENERAL => {
                let m = self.list_msg(3, st.general_index.max(0) as usize);
                let line = m.and_then(|m| m.lines.get(st.general_num.max(0) as usize).cloned());
                (
                    (line.into_iter().collect(), m.map_or(1.0, |m| m.scale)),
                    0.0,
                )
            }
            BOSS => (
                take(
                    self.list_msg(0, st.boss_index.max(0) as usize),
                    st.boss_pass,
                ),
                16.0,
            ),
            LEGEND => (
                take(
                    self.list_msg(2, st.legend_index.max(0) as usize),
                    st.legend_pass,
                ),
                16.0,
            ),
            _ => {
                let r = RUNE_ORDER[st.rune_index.clamp(0, 12) as usize] as usize;
                (take(self.list_msg(1, r - 1), st.rune_pass), 0.0)
            }
        };
        (blocks, gap, scale)
    }

    /// next_general_hint(1): one line at a time from the highest unlocked tier
    /// down, then round again.
    fn next_general(&mut self) {
        let lines = |h: &Hints, t: i32| {
            h.list_msg(3, t as usize)
                .map_or(0, |m| m.lines.len() as i32)
        };
        loop {
            let st = &mut self.state;
            if st.general_index < 0 {
                st.general_index = 3;
            }
            while st.general_index > 0 && !self.facts.sections[st.general_index as usize - 1] {
                st.general_num = -1;
                st.general_index -= 1;
            }
            st.general_num += 1;
            let t = st.general_index;
            if self.state.general_num < lines(self, t) {
                break;
            }
            let st = &mut self.state;
            st.general_num = -1;
            st.general_index -= 1;
        }
    }

    /// next_boss_hint(1). PlayerHasRune(-1, realm) tests bit `1 << realm`, while
    /// the shards are kept by world order: the "already beaten" filter looks at
    /// the wrong bit, as in the game.
    fn next_boss(&mut self) {
        let f = self.facts;
        let has_rune = |realm: u32| f.runes & (1 << realm) != 0;
        let st = &mut self.state;
        let mut n = st.boss_index + 1;
        let i = loop {
            if n >= 11 {
                n %= 11;
                st.boss_num = 1;
            }
            if n == 0 {
                n = 1;
            }
            let found = (n..11).find(|&i| {
                let id = CRYSTAL_ORDER[i as usize];
                f.world_open[id as usize] && (st.boss_num != 0 || !has_rune(id))
            });
            if let Some(i) = found {
                break i;
            }
            if st.boss_num > 1 {
                st.boss_num = 1;
                break 1;
            }
            st.boss_num += 1;
            n = 1;
        };
        st.boss_index = i;
        let bit = 1 << CRYSTAL_ORDER[i as usize];
        st.boss_pass = pass(f.boss_attempts, bit) + 1;
    }

    /// next_legend_hint(1): the same over the eight crystal realms and the Temple.
    fn next_legend(&mut self) {
        let f = self.facts;
        let st = &mut self.state;
        let mut n = st.legend_index + 1;
        let i = loop {
            if n >= 10 {
                n %= 10;
                st.legend_num = 1;
            }
            if n == 0 {
                n = 1;
            }
            let found = (n..10).find(|&i| {
                let id = CRYSTAL_ORDER[i as usize];
                f.world_open[id as usize] && (st.legend_num != 0 || f.legend_held & (1 << id) == 0)
            });
            if let Some(i) = found {
                break i;
            }
            if st.legend_num > 1 {
                st.legend_num = 1;
                break 1;
            }
            st.legend_num += 1;
            n = 1;
        };
        st.legend_index = i;
        let bit = 1 << CRYSTAL_ORDER[i as usize];
        st.legend_pass = pass([f.level_masks[2], f.level_masks[3]], bit) + 1;
    }

    /// next_rune_hint(1): the Runestones not yet held, in realm order.
    fn next_rune(&mut self) {
        let f = self.facts;
        let st = &mut self.state;
        let mut n = st.rune_index + 1;
        let i = loop {
            if n >= 13 {
                n %= 13;
                st.rune_num = 1;
            }
            let found = (n..13)
                .find(|&i| st.rune_num != 0 || f.shards & (1 << (RUNE_ORDER[i as usize] - 1)) == 0);
            if let Some(i) = found {
                break i;
            }
            if st.rune_num > 1 {
                st.rune_num = 1;
                break 0;
            }
            st.rune_num += 1;
            n = 1;
        };
        st.rune_index = i;
        let bit = 1 << (RUNE_ORDER[i as usize] - 1);
        st.rune_pass = pass([f.level_masks[0], f.level_masks[1]], bit) + 1;
    }
}

/// 1 if `bit` is in the first mask, 2 if in the second.
fn pass(masks: [u16; 2], bit: u16) -> i32 {
    if masks[1] & bit != 0 {
        2
    } else if masks[0] & bit != 0 {
        1
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> HintFacts {
        let mut f = HintFacts::default();
        // Only the Forsaken Province (town, realm 7) is open.
        f.world_open[13] = true;
        f.world_open[7] = true;
        f
    }

    #[test]
    fn guardians_cycle_over_open_realms() {
        let mut h = Hints::new(HintState::default(), facts(), None);
        h.pick(BOSS);
        assert_eq!((h.state.boss_index, h.state.boss_pass), (1, 1));
        // The Lich beat a player twice: all three lines.
        let mut f = facts();
        f.boss_attempts = [1 << 7, 1 << 7];
        let mut h = Hints::new(HintState::default(), f, None);
        h.pick(BOSS);
        assert_eq!(h.state.boss_pass, 3);
        // The town's shard is world-order bit 1, but bit 7 is tested.
        let mut f = facts();
        f.runes = 1 << 1;
        let mut h = Hints::new(HintState::default(), f, None);
        h.pick(BOSS);
        assert_eq!(h.state.boss_index, 1);
    }

    #[test]
    fn runestones_skip_the_ones_held() {
        let mut f = facts();
        f.shards = 1 << 7; // Runestone 8 (the first in realm order)
        let mut h = Hints::new(HintState::default(), f, None);
        h.pick(RUNE);
        assert_eq!(h.state.rune_index, 1);
        h.pick(RUNE);
        assert_eq!(h.state.rune_index, 2);
    }
}
