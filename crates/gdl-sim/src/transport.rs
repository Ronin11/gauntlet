//! Transporter pads (items.c item type 11, `MatchTransporters`; pmotion.c
//! `DoTransporter`). Each pad names its own id and the id of the pad it sends
//! to; a hero touching a pad whose partner is on screen fades out over 30
//! ticks, appears on the partner's floor, and fades back in (message 9). The
//! hero can't go again until stepping off the pad.

use glam::{Vec2, Vec3};

/// One placed transporter (transdata).
#[derive(Clone)]
pub struct Pad {
    pub pos: Vec3,
    pub radius: f32,
    pub id: i32,
    pub dest_id: i32,
    /// MatchTransporters: the pad whose id is this one's dest id.
    pub dest: Option<usize>,
}

/// A hero's transport state (Player +0x93C counter, +0x940 timer, transport_pos).
#[derive(Clone, Copy, Debug, Default)]
pub struct HeroTransport {
    /// 60 Hz ticks of fade left: 60..30 fading out, 29..0 fading in.
    pub timer: f32,
    /// Set on arrival: no transport until off every pad.
    pub counter: i32,
    pub dest: Vec3,
}

/// What DoTransporter did this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Step {
    /// Not transporting.
    Idle,
    /// Fading: the hero doesn't move (DoTransporter returns 1).
    Hold,
    /// Moved to the destination (returns 2): the new position.
    Arrive(Vec3),
}

#[derive(Default, Clone)]
pub struct Transport {
    pub pads: Vec<Pad>,
    pub heroes: Vec<HeroTransport>,
}

impl Transport {
    /// MatchTransporters: pair each pad with the pad its dest id names.
    pub fn match_pads(&mut self) {
        for i in 0..self.pads.len() {
            let want = self.pads[i].dest_id;
            self.pads[i].dest = (0..self.pads.len()).find(|&j| j != i && self.pads[j].id == want);
            if self.pads[i].dest.is_none() {
                log::warn!(
                    "transporter {} has no destination {}",
                    self.pads[i].id,
                    want
                );
            }
        }
    }

    /// fn_8005B8B0: the pad a hero at `at` (radius `r`) touches, if its
    /// partner can take them (`visible`: the partner pad is on screen).
    pub fn touching(&self, at: Vec3, r: f32) -> Option<usize> {
        self.pads.iter().position(|p| {
            Vec2::new(at.x - p.pos.x, at.z - p.pos.z).length() < p.radius + r
                && (at.y - p.pos.y).abs() < 2.0
        })
    }

    /// The hero's alpha (MBTreeSetAlpha in DoTransporter): 1 opaque.
    pub fn alpha(&self, pi: usize) -> f32 {
        let Some(h) = self.heroes.get(pi) else {
            return 1.0;
        };
        if h.timer <= 0.0 {
            return 1.0;
        }
        let t = h.timer as i32;
        let clear = if t >= 30 {
            255 - (t - 30) * 255 / 30
        } else {
            t * 255 / 29
        };
        1.0 - (clear as f32 / 255.0).clamp(0.0, 1.0)
    }

    /// DoTransporter for hero `pi` at `at`. `on_pad` is the pad touched this
    /// frame (special_collision_item); `start` checks the destination
    /// (PointVisible and FloorCollide there) and gives its floor, which
    /// starts the fade; `floor` finds the floor under the destination on
    /// arrival.
    pub fn step(
        &mut self,
        pi: usize,
        on_pad: Option<usize>,
        dt: f32,
        start: impl FnOnce(Vec3) -> Option<bool>,
        floor: impl FnOnce(Vec3) -> Option<f32>,
    ) -> (Step, bool) {
        if self.heroes.len() <= pi {
            self.heroes.resize(pi + 1, HeroTransport::default());
        }
        let h = &mut self.heroes[pi];
        let ticks = dt * 60.0;
        if h.timer > 0.0 {
            let before = h.timer;
            h.timer = (h.timer - ticks * 2.0).max(0.0);
            if before >= 30.0 && h.timer < 30.0 {
                let mut to = h.dest;
                if let Some(y) = floor(to) {
                    to.y = y;
                }
                h.counter = 1;
                return (Step::Arrive(to), false);
            }
            return (Step::Hold, false);
        }
        let dest = on_pad
            .and_then(|p| self.pads.get(p))
            .and_then(|p| p.dest)
            .map(|d| self.pads[d].pos);
        match dest {
            Some(to) => {
                if h.counter <= 0 {
                    h.dest = to;
                    // PointVisible: only then does it hold the hero; a floor
                    // there starts the fade (and fn_8009C98C's sound).
                    if let Some(floor_ok) = start(to) {
                        let mut sound = false;
                        if floor_ok {
                            h.timer = 60.0;
                            sound = true;
                        }
                        return (Step::Hold, sound);
                    }
                }
            }
            None => {
                if h.counter > 0 {
                    h.counter -= 1;
                }
            }
        }
        (Step::Idle, false)
    }
}

/// lbl_8012406C (fn_8009C98C): the transporter sound by realm.
const TRANSPORT_SFX: [i32; 16] = [
    -1, 0x25000E, -1, 0x270026, -1, -1, -1, 0x290029, 0x2A002B, 0x2B0029, 0x2C002B, 0x2D0029,
    0x3B001A, 0, -1, -1,
];

impl crate::game::Game {
    /// DoTransporter for every hero in play, before they move. Returns the
    /// heroes held still this frame, with where they are to stay.
    pub(crate) fn update_transporters(&mut self, dt: f32) -> Vec<(usize, Vec3)> {
        use crate::game::PLAYER_RADIUS;
        let mut held = Vec::new();
        if self.transport.pads.is_empty() {
            return held;
        }
        for pi in 0..self.players.len() {
            if !self.players[pi].state.active() {
                continue;
            }
            let at = self.players[pi].pos;
            let pad = self.transport.touching(at, PLAYER_RADIUS);
            // fn_8005B8B0: the partner pad must be in view (item active &
            // 0x4000); PointVisible(-radius) the spot; FloorCollide 4 up
            // to 10 down from it.
            let pad_seen = pad
                .and_then(|p| self.transport.pads[p].dest)
                .is_some_and(|d| {
                    let p = &self.transport.pads[d];
                    self.on_screen(p.pos, p.radius)
                });
            let ground = |c: &crate::collision::Collision, to: Vec3| {
                c.ground_height(to, to.y + 4.0)
                    .filter(|g| *g >= to.y - 10.0)
            };
            let dest = pad
                .and_then(|p| self.transport.pads[p].dest)
                .map(|d| self.transport.pads[d].pos);
            let begin = dest.and_then(|to| {
                (pad_seen && self.on_screen(to, -PLAYER_RADIUS))
                    .then(|| ground(&self.collision, to).is_some())
            });
            let arrive = self
                .transport
                .heroes
                .get(pi)
                .and_then(|h| ground(&self.collision, h.dest));
            let start = |_| begin;
            let floor = |_| arrive;
            let pad = pad.filter(|_| pad_seen);
            let (step, sound) = self.transport.step(pi, pad, dt, start, floor);
            if sound {
                let id = self.realm_sound(&TRANSPORT_SFX);
                self.sfx_id(id);
            }
            match step {
                crate::transport::Step::Idle => {}
                crate::transport::Step::Hold => held.push((pi, at)),
                crate::transport::Step::Arrive(to) => {
                    let p = &mut self.players[pi];
                    p.pos = to;
                    p.vy = 0.0;
                    held.push((pi, p.pos));
                    self.post(9, Some(pi), &[]);
                }
            }
        }
        held
    }

    /// The heroes DoTransporter holds (it returns 1) don't move this frame.
    pub(crate) fn hold_transporting(&mut self, held: Vec<(usize, Vec3)>) {
        for (pi, at) in held {
            if let Some(p) = self.players.get_mut(pi) {
                p.pos = at;
                p.vy = 0.0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair() -> Transport {
        let mut t = Transport::default();
        t.pads.push(Pad {
            pos: Vec3::ZERO,
            radius: 1.6,
            id: 1,
            dest_id: 2,
            dest: None,
        });
        t.pads.push(Pad {
            pos: Vec3::new(20.0, 3.0, 0.0),
            radius: 1.6,
            id: 2,
            dest_id: 1,
            dest: None,
        });
        t.match_pads();
        t
    }

    #[test]
    fn pads_pair_up_and_send_a_hero_across() {
        let mut t = pair();
        assert_eq!(t.pads[0].dest, Some(1));
        assert_eq!(t.pads[1].dest, Some(0));
        let mut at = Vec3::new(0.5, 0.0, 0.0);
        let dt = 1.0 / 60.0;
        let (s, sound) = t.step(0, t.touching(at, 1.5), dt, |_| Some(true), |_| None);
        assert_eq!(s, Step::Hold);
        assert!(sound);
        assert!(t.alpha(0) > 0.9, "starts opaque");
        let mut frames = 0;
        let mut faded = false;
        loop {
            frames += 1;
            let (s, _) = t.step(0, t.touching(at, 1.5), dt, |_| Some(true), |p| Some(p.y));
            if t.alpha(0) < 0.1 {
                faded = true;
            }
            if let Step::Arrive(to) = s {
                at = to;
                break;
            }
            assert_eq!(s, Step::Hold);
            assert!(frames < 60);
        }
        assert!(faded, "faded out before moving");
        assert_eq!(frames, 16, "past half the 60-tick timer at 2 a tick");
        assert_eq!(at, Vec3::new(20.0, 3.0, 0.0));
        // Fading in on the far pad; it doesn't send the hero straight back.
        for _ in 0..40 {
            let (s, _) = t.step(0, t.touching(at, 1.5), dt, |_| Some(true), |_| None);
            assert_ne!(s, Step::Arrive(Vec3::ZERO));
        }
        assert_eq!(t.alpha(0), 1.0);
        assert_eq!(t.heroes[0].counter, 1);
        // Off the pad and back on: it goes again.
        let off = Vec3::new(30.0, 3.0, 0.0);
        assert_eq!(
            t.step(0, t.touching(off, 1.5), dt, |_| Some(true), |_| None)
                .0,
            Step::Idle
        );
        assert_eq!(t.heroes[0].counter, 0);
        assert_eq!(
            t.step(0, t.touching(at, 1.5), dt, |_| Some(true), |_| None)
                .0,
            Step::Hold
        );
    }

    /// levelA1's first pair of pads: standing on one sends the hero to the other.
    #[test]
    fn a_transporter_pair_moves_a_hero() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game-data");
        let Ok(fs) = crate::fs::GameFs::open(&root) else {
            return;
        };
        let mut g = crate::game::Game::new(&fs, "levelA1", &[("WAR", "RED")]).unwrap();
        assert!(g.transport.pads.len() >= 4);
        assert!(
            g.transport.pads.iter().all(|p| p.dest.is_some()),
            "all paired"
        );
        let (from, to) = (g.transport.pads[0].pos, g.transport.pads[1].pos);
        assert_eq!(g.transport.pads[0].dest, Some(1));
        let input = [crate::input::PlayerInput::default()];
        g.players[0].pos = from + Vec3::Y * 0.2;
        // A view of both pads.
        let mid = (from + to) * 0.5;
        g.sumner = None;
        g.fixed_camera = Some((mid + Vec3::new(0.0, 40.0, -30.0), mid));
        let mut faded = false;
        let mut left = false;
        for _ in 0..120 {
            g.update(&input);
            faded |= g.transport.alpha(0) < 0.2;
            let p = g.players[0].pos;
            if Vec2::new(p.x - to.x, p.z - to.z).length() < 0.5 {
                left = true;
                break;
            }
        }
        g.update(&input);
        assert!(faded, "faded out");
        assert!(left, "hero at {} not on the far pad {to}", g.players[0].pos);
        assert!(
            g.sfx.iter().any(|s| s.contains("S_TRANSPORTER")),
            "message 9 speaks: {:?}",
            g.sfx
        );
    }

    #[test]
    fn an_unseen_destination_does_nothing() {
        let mut t = pair();
        let (s, _) = t.step(0, Some(0), 1.0 / 60.0, |_| None, |_| None);
        assert_eq!(s, Step::Idle);
        assert_eq!(t.heroes[0].timer, 0.0);
    }
}
