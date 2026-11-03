//! Playing together over the network, in lockstep: every machine runs the same
//! [`Session`](gdl_sim::session::Session), and a tick runs only once everyone's
//! input for it has arrived. Nothing but inputs travels, so the game can stay
//! the same everywhere only by being deterministic, which `gdl-sim` is; a
//! fingerprint of the game travels with the inputs now and then to prove it.
//!
//! [`lobby`] finds the players. [`Net`] carries the inputs, by GGRS.

pub mod lobby;

use anyhow::{bail, Result};
use gdl_sim::input::PackedInput;
use ggrs::{
    GgrsError, GgrsEvent, GgrsRequest, InputStatus, NonBlockingSocket, P2PSession, PlayerType,
    SessionBuilder, SessionState, UdpNonBlockingSocket,
};
use lobby::Roster;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

/// Ticks between fingerprints (two seconds).
pub const CHECK_EVERY: u64 = 120;

/// How long a silent player may hold everyone up. A level's load blocks a
/// machine's network, so this is longer than the slowest load.
const GIVE_UP_AFTER: Duration = Duration::from_secs(15);
/// When to start saying that someone has gone quiet.
const SAY_QUIET_AFTER: Duration = Duration::from_secs(3);

/// How long after saying they are leaving a player is cut off. Lockstep keeps
/// everyone within the input delay of each other, and word of a cut spreads
/// ahead of the ticks, so a cut made sooner could overtake a machine that has
/// not yet run the leaver's last ticks.
const CUT_AFTER: u64 = 120;

/// A player's input for a tick, as it travels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetInput {
    stick: [i8; 2],
    fire: [i8; 2],
    dpad: [i8; 2],
    buttons: u32,
    /// The sender's fingerprint of the game `delay` ticks before this input's
    /// (0: none this tick).
    check: u32,
}

impl NetInput {
    fn new(p: PackedInput, check: u32) -> NetInput {
        NetInput {
            stick: p.stick,
            fire: p.fire,
            dpad: p.dpad,
            buttons: p.buttons,
            check,
        }
    }

    fn packed(&self) -> PackedInput {
        PackedInput {
            stick: self.stick,
            fire: self.fire,
            dpad: self.dpad,
            buttons: self.buttons,
        }
    }
}

/// GGRS's types for this game.
#[derive(Debug)]
pub struct Cfg;

impl ggrs::Config for Cfg {
    type Input = NetInput;
    type InputPredictor = ggrs::PredictRepeatLast;
    /// Lockstep never rolls back, so GGRS never keeps a state.
    type State = ();
    type Address = SocketAddr;
}

/// What happened on the network, for the screen and the log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NetEvent {
    /// Everyone is connected: the game can start.
    Connected,
    /// Nothing from this player for a while.
    Quiet {
        player: usize,
    },
    Back {
        player: usize,
    },
    /// Someone's game is no longer ours: a bug in the determinism. The frame
    /// is the tick the fingerprints were taken before.
    Desync {
        player: usize,
        frame: u64,
    },
    /// This player's connection died with two or more others still playing.
    /// Lockstep can't say which of the last ticks each of them saw that
    /// player's input for (one may have run a tick the other never got the
    /// input for, and nothing rolls back), so the game can't safely go on.
    Lost {
        player: usize,
    },
}

/// A tick everyone's input has arrived for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    /// Each player's input, by player number. A player who has left has none.
    pub inputs: Vec<PackedInput>,
    /// A player whose connection died as of this tick, when one other machine
    /// is left to carry on. (With more, see [`NetEvent::Lost`].)
    pub left: Vec<usize>,
}

/// One machine's end of the game.
pub struct Net {
    session: P2PSession<Cfg>,
    me: usize,
    delay: u64,
    addrs: Vec<SocketAddr>,
    /// Ticks run.
    frame: u64,
    /// The local input for this tick has been handed over.
    given: bool,
    gone: Vec<bool>,
    /// Players who said they were leaving: not one input of theirs counts after
    /// the tick they said it on, and they are cut off at this tick. They stay
    /// on the line until then (see [`CUT_AFTER`]).
    dropped: Vec<Option<u64>>,
    /// This machine's player said they are leaving: the tick they may go at.
    leaving_at: Option<u64>,
    /// Our recent fingerprints, by the tick they were taken before.
    checks: VecDeque<(u64, u32)>,
    events: Vec<NetEvent>,
    connected: bool,
    /// Since when this machine has had to wait for a tick.
    waiting_since: Option<Instant>,
}

impl Net {
    /// Over UDP, on the port the lobby used.
    pub fn start(roster: &Roster) -> Result<Net> {
        let socket = UdpNonBlockingSocket::bind_to_port(roster.port)?;
        Net::with_socket(roster, socket)
    }

    /// Over anything that carries packets.
    pub fn with_socket<S>(roster: &Roster, socket: S) -> Result<Net>
    where
        S: NonBlockingSocket<SocketAddr> + 'static,
    {
        Net::build(roster, socket, GIVE_UP_AFTER)
    }

    fn build<S>(roster: &Roster, socket: S, give_up: Duration) -> Result<Net>
    where
        S: NonBlockingSocket<SocketAddr> + 'static,
    {
        let n = roster.addrs.len();
        let mut b = SessionBuilder::<Cfg>::new()
            .with_num_players(n)?
            .with_max_prediction_window(0)
            .with_input_delay(roster.delay)
            .with_disconnect_timeout(give_up)
            .with_disconnect_notify_delay(SAY_QUIET_AFTER.min(give_up / 2))
            .with_fps(60)?;
        for (i, addr) in roster.addrs.iter().enumerate() {
            let t = if i == roster.me {
                PlayerType::Local
            } else {
                PlayerType::Remote(*addr)
            };
            b = b.add_player(t, i)?;
        }
        Ok(Net {
            session: b.start_p2p_session(socket)?,
            me: roster.me,
            delay: roster.delay as u64,
            addrs: roster.addrs.clone(),
            frame: 0,
            given: false,
            gone: vec![false; n],
            dropped: vec![None; n],
            leaving_at: None,
            checks: VecDeque::new(),
            events: Vec::new(),
            connected: false,
            waiting_since: None,
        })
    }

    /// This machine's player number.
    pub fn me(&self) -> usize {
        self.me
    }

    pub fn players(&self) -> usize {
        self.addrs.len()
    }

    /// Ticks run so far.
    pub fn frame(&self) -> u64 {
        self.frame
    }

    pub fn is_gone(&self, player: usize) -> bool {
        self.gone.get(player).copied().unwrap_or(true)
            || self.dropped.get(player).is_some_and(Option::is_some)
    }

    /// Everyone has connected.
    pub fn connected(&self) -> bool {
        self.connected
    }

    /// How long this machine has been waiting for someone's input.
    pub fn waiting_for(&self) -> Duration {
        self.waiting_since.map_or(Duration::ZERO, |t| t.elapsed())
    }

    /// The round trip to a player, as GGRS last measured it.
    pub fn ping_ms(&self, player: usize) -> Option<u128> {
        if player == self.me || self.is_gone(player) {
            return None;
        }
        self.session
            .network_stats(player)
            .ok()
            .map(|s| s.ping as u128)
    }

    /// Keep the network moving; call at least once a frame, ticks or none.
    pub fn poll(&mut self) {
        self.session.poll_remote_clients();
        self.collect_events();
    }

    /// Is this machine's input for the next tick wanted? It is wanted once per
    /// tick, however long the tick then has to wait for everyone else's.
    pub fn wants_input(&self) -> bool {
        !self.given && self.session.current_state() == SessionState::Running
    }

    /// This machine's input for the next tick. `fingerprint` is asked for the
    /// state of the game every [`CHECK_EVERY`] ticks.
    pub fn give(&mut self, input: PackedInput, fingerprint: impl FnOnce() -> u32) -> Result<()> {
        let mut check = 0;
        if self.frame % CHECK_EVERY == 0 {
            // Never 0, which means no fingerprint.
            check = fingerprint() | 1;
            self.checks.push_back((self.frame, check));
            while self.checks.len() > 64 {
                self.checks.pop_front();
            }
        }
        self.session
            .add_local_input(self.me, NetInput::new(input, check))?;
        self.given = true;
        Ok(())
    }

    /// The next tick, if everyone's input for it is here.
    pub fn tick(&mut self) -> Result<Option<Frame>> {
        if !self.given {
            self.poll();
            self.wait();
            return Ok(None);
        }
        let requests = match self.session.advance_frame() {
            Ok(r) => r,
            Err(GgrsError::NotSynchronized | GgrsError::PredictionThreshold) => {
                self.collect_events();
                self.wait();
                return Ok(None);
            }
            Err(e) => bail!("network: {e}"),
        };
        self.collect_events();
        for request in requests {
            if let GgrsRequest::AdvanceFrame { inputs } = request {
                self.waiting_since = None;
                self.given = false;
                return Ok(Some(self.ran(&inputs)));
            }
        }
        self.wait();
        Ok(None)
    }

    /// A player has told everyone they are leaving, in a tick all have run:
    /// call it after that tick, on every machine that stays. The cut is exact,
    /// so this works however many are playing, unlike a line going dead. The
    /// one leaving calls [`Net::leave`], and stays on the line until
    /// [`Net::may_go`].
    pub fn drop_player(&mut self, player: usize) {
        if player != self.me && self.dropped[player].is_none() {
            self.dropped[player] = Some(self.frame + CUT_AFTER);
        }
    }

    /// This machine's player has told everyone they are leaving, in a tick all
    /// have run. Those who stay need their input for a while yet.
    pub fn leave(&mut self) {
        self.leaving_at.get_or_insert(self.frame + CUT_AFTER + 1);
    }

    /// May this machine now go without holding anyone up?
    pub fn may_go(&self) -> bool {
        self.leaving_at.is_some_and(|at| self.frame >= at)
    }

    fn wait(&mut self) {
        if self.waiting_since.is_none() {
            self.waiting_since = Some(Instant::now());
        }
    }

    fn ran(&mut self, inputs: &[(NetInput, InputStatus)]) -> Frame {
        let frame = self.frame;
        self.frame += 1;
        let mut left = Vec::new();
        for player in 0..self.dropped.len() {
            if self.dropped[player] == Some(self.frame) && !self.gone[player] {
                self.gone[player] = true;
                if let Err(e) = self.session.disconnect_player(player) {
                    log::warn!("network: cutting off player {player}: {e}");
                }
            }
        }
        for (player, (input, status)) in inputs.iter().enumerate() {
            if self.dropped[player].is_some() {
                continue;
            }
            match status {
                InputStatus::Disconnected => {
                    if !self.gone[player] {
                        self.gone[player] = true;
                        // Those still here, with this machine.
                        let here = (0..self.gone.len()).filter(|&p| !self.is_gone(p)).count();
                        if here > 1 {
                            self.events.push(NetEvent::Lost { player });
                        } else {
                            left.push(player);
                        }
                    }
                }
                // Having left, this machine's game is its own affair.
                _ if input.check != 0 && player != self.me && self.leaving_at.is_none() => {
                    self.compare(player, frame, input.check);
                }
                _ => {}
            }
        }
        Frame {
            inputs: inputs
                .iter()
                .enumerate()
                .map(|(p, (i, s))| match s {
                    InputStatus::Disconnected => PackedInput::default(),
                    _ if self.dropped[p].is_some() => PackedInput::default(),
                    _ => i.packed(),
                })
                .collect(),
            left,
        }
    }

    /// A fingerprint that came with the input for tick `frame`.
    fn compare(&mut self, player: usize, frame: u64, theirs: u32) {
        let Some(taken) = frame.checked_sub(self.delay) else {
            return;
        };
        let ours = self
            .checks
            .iter()
            .find(|(f, _)| *f == taken)
            .map(|(_, check)| *check);
        if ours.is_some_and(|ours| ours != theirs) {
            self.events.push(NetEvent::Desync {
                player,
                frame: taken,
            });
        }
    }

    fn player_at(&self, addr: &SocketAddr) -> Option<usize> {
        (0..self.addrs.len()).find(|&i| i != self.me && self.addrs[i] == *addr)
    }

    fn collect_events(&mut self) {
        if !self.connected && self.session.current_state() == SessionState::Running {
            self.connected = true;
            self.events.push(NetEvent::Connected);
        }
        let events: Vec<_> = self.session.events().collect();
        for e in events {
            match e {
                GgrsEvent::NetworkInterrupted { addr, .. } => {
                    if let Some(player) = self.player_at(&addr) {
                        self.events.push(NetEvent::Quiet { player });
                    }
                }
                GgrsEvent::NetworkResumed { addr } => {
                    if let Some(player) = self.player_at(&addr) {
                        self.events.push(NetEvent::Back { player });
                    }
                }
                // Lockstep has no use for the rest: a late machine just holds
                // everyone up, and who has left is told tick by tick.
                other => log::debug!("network: {other:?}"),
            }
        }
    }

    /// What has happened since we last looked.
    pub fn events(&mut self) -> Vec<NetEvent> {
        std::mem::take(&mut self.events)
    }
}

#[cfg(test)]
mod tests;
