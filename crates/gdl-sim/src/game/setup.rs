//! How the world outside the simulation is allowed to change a game.
//!
//! Peers of a networked game stay in step only if every one of them sees
//! the same changes on the same tick, so nothing local to a machine (its
//! options file, its menus) may write into `Game` directly. What a level
//! starts with travels as a [`LevelSetup`], agreed before the first tick.
//! What changes during play travels as a [`Command`], applied between ticks
//! at a tick all the peers agree on. Both are plain values, so they can be
//! sent, recorded and replayed.
//!
//! The debug menu and the headless tools still reach in directly: they are
//! local by nature, and a networked game turns them off.

use super::*;
use crate::input::PlayerPrefs;
use crate::progress::Progress;
use crate::rules::Rules;

/// What a level starts with, beyond its files and the party's heroes.
#[derive(Clone, Debug, PartialEq)]
pub struct LevelSetup {
    pub rules: Rules,
    /// Mixed into the game's random numbers. Zero keeps the fixed sequence
    /// the solo game has always used.
    pub seed: u64,
    /// The Multiplayer option (0 off, 1 and 2 as `Game::friendly_fire`).
    pub friendly_fire: usize,
    pub crystals: [i32; 9],
    pub garg_items: [i32; 3],
    pub shards: u16,
    pub runes: u16,
    pub legend_held: u16,
}

impl LevelSetup {
    pub fn new(rules: &Rules, friendly_fire: usize, progress: &Progress) -> LevelSetup {
        LevelSetup {
            rules: rules.clone(),
            seed: 0,
            friendly_fire,
            crystals: progress.crystals,
            garg_items: progress.garg_items,
            shards: progress.shards,
            runes: progress.runes,
            legend_held: progress.legend,
        }
    }
}

/// A change to a running game from outside it, applied between ticks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    /// A player changed their Controls options.
    SetControls { player: usize, prefs: PlayerPrefs },
    /// "Abort Level?" Yes: everyone is put out of the level and the party
    /// goes back to the tower.
    QuitLevel,
    /// The party left Sumner's hints menu: he waves them off.
    HintsClosed,
    /// The debug menu's no-damage switch.
    DebugGod(bool),
}

impl Game {
    /// Start the level from what was agreed; call it once, before the first
    /// tick and after the heroes are in place.
    pub fn apply_setup(&mut self, s: &LevelSetup) {
        self.rules = s.rules.clone();
        self.rng.mix(s.seed);
        self.friendly_fire = s.friendly_fire;
        self.crystals = s.crystals;
        self.garg_items = s.garg_items;
        self.shards = s.shards;
        self.runes = s.runes;
        self.set_legend_held(s.legend_held);
    }

    /// The party's finds, given back to a level resumed from a secret level.
    pub fn resume_with(&mut self, p: &Progress) {
        self.crystals = p.crystals;
        self.garg_items = p.garg_items;
        self.shards = p.shards;
    }

    pub fn command(&mut self, c: Command) {
        match c {
            Command::SetControls { player, prefs } => {
                if let Some(slot) = self.controls.get_mut(player) {
                    *slot = prefs;
                }
            }
            Command::QuitLevel => {
                for p in &mut self.players {
                    p.state = PlayerState::InTower;
                }
                self.level_end = Some(LevelEnd::AllDead);
            }
            Command::HintsClosed => {
                if let Some(mut sm) = self.sumner.take() {
                    sm.hints_closed(self);
                    self.sumner = Some(sm);
                }
            }
            Command::DebugGod(on) => self.debug_god = on,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::PlayerInput;

    fn game() -> Option<Game> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game-data");
        let fs = GameFs::open(&root).ok()?;
        Game::new(&fs, "levelA1", &[("WAR", "RED")]).ok()
    }

    #[test]
    fn everything_in_the_setup_reaches_the_fingerprint() {
        let Some(base) = game() else { return };
        let plain = LevelSetup::new(&Rules::classic(), 0, &Progress::default());
        let changed = |f: &dyn Fn(&mut LevelSetup)| {
            let mut s = plain.clone();
            f(&mut s);
            let mut g = base.clone();
            g.apply_setup(&s);
            g.fingerprint()
        };
        let same = changed(&|_| {});
        let edits: [(&str, &dyn Fn(&mut LevelSetup)); 8] = [
            ("rules", &|s| s.rules = Rules::remix()),
            ("seed", &|s| s.seed = 7),
            ("friendly fire", &|s| s.friendly_fire = 2),
            ("crystals", &|s| s.crystals[0] = 3),
            ("gargoyle items", &|s| s.garg_items[1] = 1),
            ("shards", &|s| s.shards = 5),
            ("runes", &|s| s.runes = 2),
            ("legend", &|s| s.legend_held = 4),
        ];
        for (name, edit) in edits {
            assert_ne!(changed(edit), same, "{name} is not in the fingerprint");
        }
    }

    #[test]
    fn a_zero_seed_keeps_the_solo_game() {
        let Some(base) = game() else { return };
        let mut g = base.clone();
        g.apply_setup(&LevelSetup::new(&Rules::classic(), 0, &Progress::default()));
        assert_eq!(g.rng.next_u32(), base.clone().rng.next_u32());
    }

    #[test]
    fn commands_reach_the_game_and_its_fingerprint() {
        let Some(mut g) = game() else { return };
        let before = g.fingerprint();
        let prefs = PlayerPrefs {
            control_style: 2,
            ..Default::default()
        };
        g.command(Command::SetControls { player: 0, prefs });
        assert_eq!(g.controls[0], prefs);
        let controls = g.fingerprint();
        assert_ne!(before, controls, "the controls are in the fingerprint");
        g.command(Command::DebugGod(true));
        assert!(g.debug_god);
        assert_ne!(controls, g.fingerprint(), "god mode is in the fingerprint");
        g.command(Command::QuitLevel);
        assert_eq!(g.players[0].state, PlayerState::InTower);
        assert!(g.level_end.is_some());
    }

    /// The box is state of the game, read by its tick on every peer alike.
    #[test]
    fn a_scroll_stops_the_tick_until_its_reader_closes_it() {
        let Some(mut g) = game() else { return };
        g.scroll_box = Some(ScrollBox::new(Some(0), vec!["page".into()], 0.5));
        let frame = g.frame;
        for _ in 0..20 {
            g.update(&[PlayerInput::default()]);
        }
        assert_eq!(g.frame, frame, "the world waits");
        assert!(g.scroll_box.is_some());
        g.update(&[PlayerInput {
            b_pressed: true,
            ..Default::default()
        }]);
        assert!(g.scroll_box.is_none(), "B closes the last page");
        g.update(&[PlayerInput::default()]);
        assert_eq!(g.frame, frame + 1, "and the world goes on");
    }
}
