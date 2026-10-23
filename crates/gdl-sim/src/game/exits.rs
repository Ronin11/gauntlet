//! Leaving a level through an exit (item type 9): a hero must stand still on
//! it for 6 ticks to start waiting (DoExit, pmotion.c:3900); the exit opens
//! as the party gathers on it (fn_800606FC case 9, items.c:4387); and nobody
//! sinks until every hero still playing waits for the same destination
//! (all_players_go_to_same_level, player.c:3107, and do_players' state 4,
//! player.c:2462).
//!
//! A child of `game`, so it reaches the game's private state.

use super::*;
use crate::paction::{AnimInfo, SlotSeq};

/// DoExit: ticks a hero stands still on an exit before waiting there.
pub(crate) const EXIT_STILL_TICKS: i32 = 6;
/// do_players' state 4: a waiting hero breathes once idle_timer reaches 600,
/// then every 540 ticks.
const BREATH_FIRST: i32 = 600;
const BREATH_EVERY: i32 = 540;

/// A hero's part in leaving: idle_timer, state 4 and the exit touched
/// (special_collision_item, read by fn_8005B8FC).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ExitWait {
    /// idle_timer: ticks stood still on an exit (also the breath timer).
    pub idle: i32,
    /// state 4: waiting on an exit for the others (still playing).
    pub waiting: bool,
    /// The exit touched this tick and where it leads (None = the tower).
    pub touch: Option<Option<u32>>,
    /// PlayerControl joymag this tick.
    pub joymag: f32,
}

/// The level's exit progress: lbl_803447D0 (the best exit's action, +10 once
/// everyone is on one, 0xE/0xF once it is fully open) and lbl_803447B4 (the
/// party has started sinking).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ExitState {
    pub progress: i32,
    pub started: bool,
}

/// An exit's animation: the generic item step for items without active & 1
/// (fn_800606FC: action follows daction once AnimateATree advances) and case
/// 9's state, with data.exit.onexit (the heroes on it, a bit each).
#[derive(Clone, Debug, PartialEq)]
pub struct ExitAnim {
    pub anim: AnimInfo,
    slots: Vec<SlotSeq>,
    /// Sequence shown (action), wanted (daction) and last tick's (paction):
    /// 0 IDLE, 1 READY, 2 ACTIVE1, 3 ACTIVE2, 4 ACTIVE3.
    pub action: i32,
    daction: i32,
    paction: i32,
    /// Ticks the exit holds before opening further.
    activetime: i32,
    pub onexit: u32,
}

/// What case 9 reads of the players: pmask (heroes in state 1, 4 or 5), any
/// in state 5 (gone), any in state 4 (waiting or sinking).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct PartyOnExits {
    pub pmask: u32,
    pub gone: bool,
    pub waiting: bool,
}

impl ExitAnim {
    pub(crate) fn new(model: Option<&Model>) -> ExitAnim {
        let slots = model
            .map(|m| {
                m.skeleton
                    .sequences
                    .iter()
                    .enumerate()
                    .map(|(i, s)| SlotSeq {
                        seq: i,
                        frames: s.frame_count as u16,
                        rate: s.frame_rate,
                    })
                    .collect()
            })
            .unwrap_or_default();
        ExitAnim {
            anim: AnimInfo::default(),
            slots,
            action: 0,
            daction: 0,
            paction: 0,
            activetime: 0,
            onexit: 0,
        }
    }

    /// One tick: the generic animation step, then case 9 (which raises the
    /// level's `progress`).
    pub(crate) fn update(&mut self, dt: f32, party: PartyOnExits, progress: &mut i32) {
        if self.activetime > 0 {
            self.activetime -= 1;
        }
        if !self.slots.is_empty() {
            let mode = if self.action == self.daction { 0 } else { 2 };
            let s = self.slots[(self.daction.max(0) as usize).min(self.slots.len() - 1)];
            if self.anim.animate_slot(dt, s, mode) & 3 != 0 {
                self.action = self.daction;
            }
        }
        if party.gone {
            *progress = 0xF;
        } else if party.pmask != 0 && self.onexit == party.pmask {
            // Everyone is on it: open all the way, then let them go.
            if self.daction == 0 {
                self.activetime = 0;
            }
            if self.activetime <= 0 && self.daction < 4 {
                self.daction += 1;
            }
            if self.action < 4 {
                self.onexit = 0;
            }
            *progress = if self.action == 4 && self.activetime <= 0 {
                if party.waiting {
                    0xE
                } else {
                    0xF
                }
            } else {
                self.action + 10
            };
        } else if self.onexit != 0 {
            // Some are on it: open to ACTIVE2 and hold there.
            if self.activetime <= 0 {
                if self.daction < 3 {
                    self.daction += 1;
                } else if self.daction > 3 {
                    self.daction = 0;
                }
            }
            self.onexit = 0;
            *progress = (*progress).max(self.action);
            self.paction = 2;
        } else {
            // Nobody: close again.
            if self.action > 0 {
                if self.activetime <= 0 {
                    if self.daction < 4 {
                        self.daction += 1;
                    } else {
                        self.daction = 0;
                    }
                } else if self.action == 3 {
                    self.activetime = 0;
                    self.daction = 4;
                    self.action = 4;
                }
            }
            self.onexit = 0;
            *progress = (*progress).max(self.action);
            self.paction = 0;
        }
        if self.action == 3 && self.paction == 2 {
            self.activetime = 0x2D;
        }
        // atree +0x38: IDLE, READY and ACTIVE2 loop.
        self.anim.repeat = self.daction == 3 || self.daction <= 1;
        self.paction = self.action;
    }
}

impl Game {
    /// What every exit's case 9 sees of the party this tick.
    pub(crate) fn party_on_exits(&self) -> PartyOnExits {
        let mut party = PartyOnExits::default();
        for (i, p) in self.players.iter().enumerate().take(4) {
            match p.state {
                PlayerState::Normal | PlayerState::Exiting { .. } | PlayerState::Exited => {
                    party.pmask |= 1 << i;
                }
                _ => {}
            }
            party.gone |= p.state == PlayerState::Exited;
            party.waiting |= matches!(p.state, PlayerState::Exiting { .. })
                || (p.state.active() && p.exit.waiting);
        }
        party
    }

    /// fn_8005F0F4 case 9: an exit's reach grows by a unit per extra player.
    pub(crate) fn exit_reach(&self, radius: f32) -> f32 {
        radius + (self.players.len().clamp(1, 4) as f32 - 1.0)
    }

    /// all_players_go_to_same_level: once an exit is fully open with everyone
    /// on it, the destination every waiting hero shares; None while anyone
    /// still plays (not waiting) or the waiting heroes disagree.
    fn all_players_go_to_same_level(&self) -> Option<Option<u32>> {
        if self.exit_state.progress < 0xE {
            return None;
        }
        let mut dest = None;
        for p in self.players.iter().take(4) {
            if !p.state.active() {
                continue;
            }
            if !p.exit.waiting {
                return None;
            }
            let here = p.exit.touch?;
            match dest {
                None => dest = Some(here),
                Some(d) if d != here => return None,
                _ => {}
            }
        }
        dest
    }

    /// One tick of leaving, after the exits have seen who stands on them
    /// (`touched`: each player's exit this tick).
    pub(crate) fn update_exits(&mut self, touched: &[Option<Option<u32>>]) {
        // do_players: the destination, from last tick's waiting heroes.
        let exit_level = self.all_players_go_to_same_level();
        for (i, p) in self.players.iter_mut().enumerate() {
            p.exit.touch = touched.get(i).copied().flatten();
            if !p.state.active() {
                p.exit.waiting = false;
                p.exit.idle = 0;
            }
        }
        // State 4: sink once the party agrees (or has started) and nobody is
        // still walking or dying.
        let started = self.exit_state.started;
        if exit_level.is_some() || started {
            let walking = !started
                && self.players.iter().any(|p| {
                    (p.state.active() && !p.exit.waiting)
                        || matches!(p.state, PlayerState::Dead { .. })
                });
            if !walking {
                // do_exit: the agreed level; heroes who wait once the party
                // has gone follow it.
                let dest = exit_level.or_else(|| self.players_dest()).flatten();
                let mut sank = false;
                for p in &mut self.players {
                    if p.state.active() && p.exit.waiting {
                        p.state = PlayerState::Exiting { timer: 50.0 / 60.0 };
                        p.exit.waiting = false;
                        p.exit_dest = dest;
                        sank = true;
                    }
                }
                if sank {
                    self.exit_state.started = true;
                    self.exit_sounds();
                }
            }
        }
        // AudioPlayerBreath: a waiting hero's name and "waiting" now and then.
        let waiting = if self.realm() == Some(13) {
            "S_WAITINGL"
        } else {
            "S_WAITING"
        };
        for p in self.players.iter_mut().take(4) {
            if p.state.active() && p.exit.waiting && p.exit.idle >= BREATH_FIRST {
                p.exit.idle -= BREATH_EVERY;
                let name = format!("VOICE:S_{}{}1S+{waiting}", p.color, p.class);
                self.sfx.push(name);
            }
        }
        self.do_exit_rules();
    }

    /// Where the heroes already gone are headed (later waiters follow them).
    fn players_dest(&self) -> Option<Option<u32>> {
        self.players
            .iter()
            .find(|p| matches!(p.state, PlayerState::Exiting { .. } | PlayerState::Exited))
            .map(|p| p.exit_dest)
    }

    /// DoExit for every hero in play: standing still on an exit for 6 ticks
    /// starts waiting (msg 11, "same exit", in a multiplayer game outside a
    /// boss level); stepping off stops it. Nothing changes while a trigger
    /// camera or Sumner holds the controls.
    fn do_exit_rules(&mut self) {
        if self.sumner.as_ref().is_some_and(|s| s.controls_locked()) {
            return;
        }
        let multi = self.players.len() > 1;
        let mut posts = Vec::new();
        for (i, p) in self.players.iter_mut().enumerate() {
            if !p.state.active() {
                continue;
            }
            if p.exit.touch.is_some() {
                if p.exit.joymag == 0.0 {
                    p.exit.idle += 1;
                }
            } else {
                p.exit.idle = 0;
            }
            if p.exit.idle >= EXIT_STILL_TICKS {
                p.exit.waiting = true;
                if self.boss.is_none() && multi && !self.exit_state.started {
                    posts.push(i);
                }
            } else {
                p.exit.waiting = false;
            }
        }
        for i in posts {
            self.post(11, Some(i), &[]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> Option<GameFs> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game-data");
        GameFs::open(&root).ok()
    }

    /// `level` with only its exits left, nothing to fight.
    fn exits_only(fs: &GameFs, level: &str, heroes: &[(&str, &str)]) -> Game {
        let mut g = Game::new(fs, level, heroes).unwrap();
        g.entities.retain(|e| matches!(e.kind, Kind::Exit { .. }));
        g.pending_enemies.clear();
        g
    }

    fn exit_pos(g: &Game, n: usize) -> Vec3 {
        g.entities
            .iter()
            .filter(|e| matches!(e.kind, Kind::Exit { open: true, .. }))
            .nth(n)
            .expect("an exit")
            .pos
    }

    fn idle(n: usize) -> Vec<PlayerInput> {
        vec![PlayerInput::default(); n]
    }

    #[test]
    fn standing_still_on_the_exit_waits_then_sinks() {
        let Some(fs) = data() else { return };
        let mut g = exits_only(&fs, "levelA1", &[("WAR", "RED")]);
        g.players[0].pos = exit_pos(&g, 0);
        // DoExit: 6 ticks stood still before waiting.
        for _ in 0..EXIT_STILL_TICKS - 1 {
            g.update(1.0 / 60.0, &idle(1));
        }
        assert!(!g.players[0].exit.waiting);
        g.update(1.0 / 60.0, &idle(1));
        assert!(g.players[0].exit.waiting);
        assert!(g.players[0].state.active());
        // Alone on it, the exit is already open (ACTIVE3 counts as open
        // while it plays): the hero sinks within a tick or two.
        let mut sank = false;
        for _ in 0..3 {
            g.update(1.0 / 60.0, &idle(1));
            if matches!(g.players[0].state, PlayerState::Exiting { .. }) {
                sank = true;
                break;
            }
        }
        assert!(sank, "the hero sinks once the exit is open");
        for _ in 0..80 {
            g.update(1.0 / 60.0, &idle(1));
        }
        assert_eq!(g.level_end, Some(LevelEnd::Exit(None)));
    }

    #[test]
    fn walking_over_the_exit_does_not_take_it() {
        let Some(fs) = data() else { return };
        let mut g = exits_only(&fs, "levelA1", &[("WAR", "RED")]);
        let at = exit_pos(&g, 0);
        let walk = PlayerInput {
            stick: Vec2::new(0.0, 1.0),
            ..Default::default()
        };
        for _ in 0..200 {
            g.players[0].pos = at;
            g.update(1.0 / 60.0, &[walk]);
        }
        assert!(g.players[0].exit.touch.is_some());
        assert!(!g.players[0].exit.waiting);
        assert!(g.players[0].state.active());
        assert_eq!(g.level_end, None);
    }

    #[test]
    fn nobody_sinks_until_the_whole_party_waits() {
        let Some(fs) = data() else { return };
        let mut g = exits_only(&fs, "levelA1", &[("WAR", "RED"), ("VAL", "BLU")]);
        let at = exit_pos(&g, 0);
        g.players[0].pos = at;
        let away = at + Vec3::new(30.0, 0.0, 0.0);
        let mut same_exit = false;
        for _ in 0..300 {
            g.players[1].pos = away;
            g.update(1.0 / 60.0, &idle(2));
            same_exit |= g.msgs.shown.as_ref().is_some_and(|s| s.index == 11);
        }
        assert!(g.players[0].exit.waiting);
        assert!(g.players.iter().all(|p| p.state.active()), "nobody sinks");
        assert!(same_exit, "msg 11: everyone must use the same exit");
        // A waiting hero can't be hurt.
        let hp = g.players[0].health;
        g.damage_player(0, 50.0, DmgType::NO_ARMOR);
        assert_eq!(g.players[0].health, hp);
        // The other hero joins them: both go.
        g.players[1].pos = at + Vec3::new(0.5, 0.0, 0.0);
        // The exit, held at ACTIVE2 while only some were on it, opens 45
        // ticks after the last arrives.
        let mut ticks = 0;
        while g.players.iter().all(|p| p.state.active()) && ticks < 200 {
            g.update(1.0 / 60.0, &idle(2));
            ticks += 1;
        }
        assert!(ticks > 45, "sank after {ticks}");
        for _ in 0..80 {
            g.update(1.0 / 60.0, &idle(2));
        }
        assert_eq!(g.level_end, Some(LevelEnd::Exit(None)));
    }

    #[test]
    fn heroes_on_different_portals_stay() {
        let Some(fs) = data() else { return };
        let mut g = exits_only(&fs, "levelL1", &[("WAR", "RED"), ("VAL", "BLU")]);
        let (a, b) = (exit_pos(&g, 0), exit_pos(&g, 1));
        for _ in 0..300 {
            g.players[0].pos = a;
            g.players[1].pos = b;
            g.update(1.0 / 60.0, &idle(2));
        }
        assert!(g.players.iter().all(|p| p.exit.waiting));
        assert!(g.players.iter().all(|p| p.state.active()));
        assert_eq!(g.level_end, None);
    }

    #[test]
    fn the_bot_stops_on_an_exit() {
        let Some(fs) = data() else { return };
        let mut g = exits_only(&fs, "levelA1", &[("WAR", "RED")]);
        g.players[0].pos = exit_pos(&g, 0);
        for _ in 0..300 {
            let inp = vec![g.bot_input(0)];
            g.update(1.0 / 60.0, &inp);
        }
        assert_eq!(g.level_end, Some(LevelEnd::Exit(None)));
    }

    #[test]
    fn the_exit_opens_then_closes() {
        let Some(fs) = data() else { return };
        let mut g = exits_only(&fs, "levelA1", &[("WAR", "RED"), ("VAL", "BLU")]);
        let at = exit_pos(&g, 0);
        let action = |g: &Game| {
            g.entities
                .iter()
                .find_map(|e| match &e.kind {
                    Kind::Exit { anim, .. } => Some(anim.action),
                    _ => None,
                })
                .unwrap()
        };
        for _ in 0..60 {
            g.players[0].pos = at;
            g.players[1].pos = at + Vec3::new(30.0, 0.0, 0.0);
            g.update(1.0 / 60.0, &idle(2));
        }
        // One of two on it: open to ACTIVE2 and hold.
        assert_eq!(action(&g), 3);
        for _ in 0..120 {
            g.players[0].pos = at + Vec3::new(-30.0, 0.0, 0.0);
            g.update(1.0 / 60.0, &idle(2));
        }
        assert_eq!(action(&g), 0, "closed again");
    }
}
