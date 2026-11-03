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
#[derive(Clone, Debug, Default, PartialEq)]
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

/// Everything a level is started from: enough to start the same level again
/// on another machine or another day. The engine fills it from its session and
/// starts every level through it, so a recording can't miss anything.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LevelStart {
    pub level: String,
    /// 0 Easy, 1 Medium, 2 Hard.
    pub difficulty: usize,
    pub panels: Vec<PanelInfo>,
    /// By slot: what each hero brings.
    pub carry: [Option<Carried>; 4],
    /// By slot: each hero's messages seen (the save's help_disp).
    pub help_disp: [Option<[u8; 256]>; 4],
    pub setup: LevelSetup,
    /// What the tower reads from the party's progress.
    pub progress: Progress,
    /// The experience in the heroes' other saved classes: Sumner welcomes only
    /// a party with none.
    pub saved_exp: Vec<u32>,
    /// A debug save state to start from.
    pub state: Option<savestate::SaveState>,
}

impl LevelStart {
    /// The game, and whether Sumner's welcome is due in it.
    pub fn build(&self, fs: &GameFs) -> Result<(Game, bool)> {
        let heroes: Vec<_> = self.panels.iter().map(PanelInfo::hero).collect();
        let classes: Vec<(&str, &str)> = heroes.iter().map(|h| (h.0, h.1)).collect();
        let models: Vec<Option<&str>> = heroes.iter().map(|h| h.2).collect();
        let mut g = Game::new_heroes(fs, &self.level, &classes, &models, self.difficulty)?;
        g.rules = self.setup.rules.clone();
        for (i, (info, p)) in self.panels.iter().zip(&mut g.players).enumerate() {
            let slot = info.slot % 4;
            if let Some(c) = &self.carry[slot] {
                p.restore(c);
            }
            if info.class == crate::stats::SUMNER {
                p.sumner = true;
                let c = self.carry[slot].clone();
                p.restore(&c.unwrap_or_else(Carried::sumner));
            } else if info.hidden.is_none() {
                crate::cheats::apply_cheats(&info.name, p);
            }
            if let Some(h) = &self.help_disp[slot] {
                g.msgs.set_help_disp(i, h);
            }
        }
        // The rules, the options that change play and the party's finds,
        // including the legend weapons a boss fight may use.
        g.apply_setup(&self.setup);
        let mut welcome = false;
        if g.wave == Some(gdl_formats::wdata::TOWER_WAVE) {
            let progress = &self.progress;
            g.close_portals(|w| progress.portal_open(w));
            g.enter_tower();
            g.set_start(progress.tower_start());
            // sumnerUpdatePresence: the welcome is for a new party's first
            // visit (no experience in any class of any player's save).
            let exps = g
                .players
                .iter()
                .map(|p| p.exp)
                .chain(self.saved_exp.iter().copied());
            welcome = crate::tower::welcome_due(progress, exps);
            let entry = crate::tower::Entry {
                welcome,
                banked_runes: progress.banked_runes,
                banked_shards: progress.banked_shards,
                last_beaten: progress.last_beaten,
            };
            let sumner = crate::tower::Sumner::new(fs, &mut g, entry);
            g.sumner = Some(Box::new(sumner));
        }
        if let Some(st) = &self.state {
            st.apply(&mut g);
        }
        Ok((g, welcome))
    }
}

/// What the party brings back to a level that waited for it: from the shop,
/// or from a secret level.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PartyBack {
    /// Per player, what they now carry.
    pub carried: Vec<Option<Carried>>,
    /// Back from a secret level: the party's finds (crystals, gargoyle items,
    /// shards) and each player's messages seen.
    pub finds: Option<([i32; 9], [i32; 3], u16)>,
    pub help_disp: Vec<Option<[u8; 256]>>,
}

/// A change to a running game from outside it, applied between ticks.
#[derive(Clone, Debug, PartialEq)]
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
    /// The party came back to this level.
    PartyBack(Box<PartyBack>),
    /// This player has left the game (their machine has gone): their hero
    /// leaves the level at once, and the party when the level ends.
    PlayerLeft(usize),
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

    pub fn command(&mut self, c: Command) {
        match c {
            Command::SetControls { player, prefs } => {
                if let Some(slot) = self.controls.get_mut(player) {
                    *slot = prefs;
                }
            }
            Command::PlayerLeft(player) => {
                if let Some(p) = self.players.get_mut(player) {
                    p.state = PlayerState::InTower;
                    p.tower_choice = false;
                    p.quit = true;
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
            Command::PartyBack(b) => {
                // The level ended to let the party go, and goes on.
                self.level_end = None;
                for (p, c) in self.players.iter_mut().zip(&b.carried) {
                    if let Some(c) = c {
                        p.restore(c);
                    }
                }
                if let Some((crystals, garg_items, shards)) = b.finds {
                    self.crystals = crystals;
                    self.garg_items = garg_items;
                    self.shards = shards;
                }
                for (i, h) in b.help_disp.iter().enumerate() {
                    if let Some(h) = h {
                        self.msgs.set_help_disp(i, h);
                    }
                }
            }
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
