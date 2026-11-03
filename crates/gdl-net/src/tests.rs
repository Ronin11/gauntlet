use super::*;
use gdl_sim::compat::Compat;
use gdl_sim::fs::GameFs;
use gdl_sim::game::PanelInfo;
use gdl_sim::input::PlayerPrefs;
use gdl_sim::progress::Progress;
use gdl_sim::rules::Rules;
use gdl_sim::session::{Phase, Session};
use ggrs::Message;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// A network in memory that loses, delays and reorders packets, from a seed.
#[derive(Default)]
struct Wire {
    now: u64,
    queued: Vec<(u64, SocketAddr, SocketAddr, Message)>,
    /// Out of 100.
    loss: u64,
    /// Steps a packet can take, at most.
    jitter: u64,
    rng: u64,
    /// Whose packets no longer go anywhere.
    cut: Vec<SocketAddr>,
}

impl Wire {
    fn roll(&mut self) -> u64 {
        // xorshift: the test is the same every run.
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        self.rng
    }
}

struct Port {
    wire: Rc<RefCell<Wire>>,
    addr: SocketAddr,
}

impl NonBlockingSocket<SocketAddr> for Port {
    fn send_to(&mut self, msg: &Message, to: &SocketAddr) {
        let mut w = self.wire.borrow_mut();
        if w.cut.contains(&self.addr) || w.cut.contains(to) {
            return;
        }
        if w.loss > 0 && w.roll() % 100 < w.loss {
            return;
        }
        let at = w.now + 1 + if w.jitter > 0 { w.roll() % w.jitter } else { 0 };
        w.queued.push((at, self.addr, *to, msg.clone()));
    }

    fn receive_all_messages(&mut self) -> Vec<(SocketAddr, Message)> {
        let mut w = self.wire.borrow_mut();
        let now = w.now;
        let mut out = Vec::new();
        let addr = self.addr;
        w.queued.retain(|(at, from, to, m)| {
            let due = *at <= now && *to == addr;
            if due {
                out.push((*from, m.clone()));
            }
            !due
        });
        out
    }
}

fn addr(i: usize) -> SocketAddr {
    ([10, 0, 0, i as u8 + 1], 7000).into()
}

fn peers(n: usize, delay: usize, wire: &Rc<RefCell<Wire>>, give_up: Duration) -> Vec<Net> {
    (0..n)
        .map(|me| {
            let roster = Roster {
                me,
                port: 0,
                addrs: (0..n).map(addr).collect(),
                delay,
                difficulty: 1,
                friendly_fire: 0,
                controls: vec![PlayerPrefs::default(); n],
            };
            let port = Port {
                wire: wire.clone(),
                addr: addr(me),
            };
            Net::build(&roster, port, give_up).unwrap()
        })
        .collect()
}

/// A player's buttons on a tick, as if they were pressing things.
fn pressed(player: usize, tick: u64) -> PackedInput {
    PackedInput {
        stick: [((tick * 7 + player as u64) % 200) as i8, player as i8],
        buttons: ((tick / 3) as u32) ^ (player as u32) << 8,
        ..Default::default()
    }
}

/// Steps the machines; each hands over input as it is wanted and keeps what
/// comes back. Returns each machine's frames.
fn run(nets: &mut [Net], wire: &Rc<RefCell<Wire>>, frames: u64, skip: &[usize]) -> Vec<Vec<Frame>> {
    let mut out = vec![Vec::new(); nets.len()];
    let until = Instant::now() + LONG_ENOUGH;
    while Instant::now() < until {
        step(wire);
        for (i, net) in nets.iter_mut().enumerate() {
            if skip.contains(&i) {
                continue;
            }
            net.poll();
            if net.wants_input() {
                let tick = net.frame();
                net.give(pressed(i, tick), || 0x1234).unwrap();
            }
            if let Some(f) = net.tick().unwrap() {
                out[i].push(f);
            }
        }
        if nets
            .iter()
            .enumerate()
            .all(|(i, n)| skip.contains(&i) || n.frame() >= frames)
        {
            break;
        }
    }
    out
}

/// GGRS's retries run on the clock, so a step of the wire takes real time.
fn step(wire: &Rc<RefCell<Wire>>) {
    wire.borrow_mut().now += 1;
    std::thread::sleep(Duration::from_micros(500));
}

const LONG_ENOUGH: Duration = Duration::from_secs(40);

fn wire(loss: u64, jitter: u64) -> Rc<RefCell<Wire>> {
    Rc::new(RefCell::new(Wire {
        loss,
        jitter,
        rng: 0x9E3779B97F4A7C15,
        ..Default::default()
    }))
}

#[test]
fn every_machine_sees_the_same_inputs_on_a_good_line() {
    let w = wire(0, 0);
    let mut nets = peers(2, 2, &w, GIVE_UP_AFTER);
    let got = run(&mut nets, &w, 600, &[]);
    assert!(got[0].len() >= 600, "only {} ticks", got[0].len());
    let n = got[0].len().min(got[1].len());
    assert_eq!(got[0][..n], got[1][..n]);
    // Inputs arrive `delay` ticks after they were pressed.
    assert_eq!(got[0][10].inputs[1], pressed(1, 8));
    assert_eq!(got[0][10].inputs[0], pressed(0, 8));
}

#[test]
fn every_machine_sees_the_same_inputs_on_a_bad_line() {
    let w = wire(15, 12);
    let mut nets = peers(4, 3, &w, GIVE_UP_AFTER);
    let got = run(&mut nets, &w, 600, &[]);
    let n = got.iter().map(Vec::len).min().unwrap();
    assert!(n >= 600, "only {n} ticks");
    for other in &got[1..] {
        assert_eq!(got[0][..n], other[..n]);
    }
    // A press isn't lost or doubled, however long its tick waited.
    for (t, f) in got[0].iter().enumerate().skip(3).take(n - 3) {
        for (p, input) in f.inputs.iter().enumerate() {
            assert_eq!(*input, pressed(p, t as u64 - 3), "player {p} tick {t}");
        }
    }
}

#[test]
fn nothing_runs_ahead_of_the_slowest_machine() {
    let w = wire(0, 0);
    let mut nets = peers(2, 2, &w, GIVE_UP_AFTER);
    run(&mut nets, &w, 60, &[]);
    // Machine 1 stops, but is not yet given up on.
    let until = Instant::now() + Duration::from_millis(400);
    let mut alone = 0;
    while Instant::now() < until {
        step(&w);
        nets[0].poll();
        if nets[0].wants_input() {
            let t = nets[0].frame();
            nets[0].give(pressed(0, t), || 1).unwrap();
        }
        alone += nets[0].tick().unwrap().is_some() as u32;
    }
    assert!(alone <= 4, "ran {alone} ticks alone");
    assert!(nets[0].waiting_for() > Duration::from_millis(300));
}

#[test]
fn a_game_that_differs_is_noticed_and_one_that_does_not_is_not() {
    let w = wire(5, 4);
    let mut nets = peers(2, 2, &w, GIVE_UP_AFTER);
    let mut desyncs = vec![Vec::new(), Vec::new()];
    let digest = |i: usize, tick: u64| -> u32 {
        // Machine 1's game goes wrong at tick 500.
        if i == 1 && tick >= 500 {
            0xDEAD0
        } else {
            0xBEEF0
        }
    };
    let until = Instant::now() + LONG_ENOUGH;
    while Instant::now() < until {
        step(&w);
        for (i, net) in nets.iter_mut().enumerate() {
            net.poll();
            if net.wants_input() {
                let tick = net.frame();
                let fp = digest(i, tick);
                net.give(pressed(i, tick), || fp).unwrap();
            }
            net.tick().unwrap();
            desyncs[i].extend(net.events().into_iter().filter_map(|e| match e {
                NetEvent::Desync { frame, .. } => Some(frame),
                _ => None,
            }));
        }
        if nets.iter().all(|n| n.frame() >= 900) {
            break;
        }
    }
    for found in &desyncs {
        assert_eq!(found.first(), Some(&600), "{found:?}");
    }
}

/// Steps those who stay until each has run `frames` ticks or `lost` is heard.
fn stay(
    nets: &mut [Net],
    staying: &[usize],
    w: &Rc<RefCell<Wire>>,
    frames: u64,
) -> (Vec<Vec<Frame>>, Vec<usize>) {
    let mut got = vec![Vec::new(); nets.len()];
    let mut lost = Vec::new();
    let until = Instant::now() + Duration::from_secs(10);
    while Instant::now() < until {
        step(w);
        for &i in staying {
            nets[i].poll();
            if nets[i].wants_input() {
                let tick = nets[i].frame();
                nets[i].give(pressed(i, tick), || 0x1234).unwrap();
            }
            if let Some(f) = nets[i].tick().unwrap() {
                got[i].push(f);
            }
            for e in nets[i].events() {
                if let NetEvent::Lost { player } = e {
                    assert_eq!(player, nets.len() - 1);
                    lost.push(i);
                }
            }
        }
        if lost.len() == staying.len() || staying.iter().all(|&i| nets[i].frame() >= frames) {
            break;
        }
    }
    (got, lost)
}

#[test]
fn when_the_other_machine_dies_the_survivor_plays_on() {
    let w = wire(0, 3);
    let mut nets = peers(2, 2, &w, Duration::from_millis(200));
    run(&mut nets, &w, 120, &[]);
    w.borrow_mut().cut.push(addr(1));
    let (got, lost) = stay(&mut nets, &[0], &w, 400);
    assert!(nets[0].frame() >= 400, "stuck at {}", nets[0].frame());
    assert!(lost.is_empty());
    assert!(nets[0].is_gone(1));
    assert_eq!(
        got[0].iter().filter(|f| f.left == [1]).count(),
        1,
        "told once"
    );
    // Their hero presses nothing any more.
    assert_eq!(got[0].last().unwrap().inputs[1], PackedInput::default());
}

/// With two or more left, nobody can say which ticks the dead line's last
/// inputs reached, so the game must stop rather than quietly come apart.
#[test]
fn when_a_machine_dies_among_several_the_game_stops() {
    for n in [3usize, 4] {
        let w = wire(0, 3);
        let mut nets = peers(n, 2, &w, Duration::from_millis(200));
        run(&mut nets, &w, 120, &[]);
        w.borrow_mut().cut.push(addr(n - 1));
        let staying: Vec<usize> = (0..n - 1).collect();
        let (_, mut lost) = stay(&mut nets, &staying, &w, u64::MAX);
        lost.sort();
        assert_eq!(lost, staying, "{n} players");
    }
}

/// Someone who says they are leaving goes at an exact tick, even if the last
/// inputs they sent reach some and not others.
#[test]
fn someone_who_says_they_are_leaving_leaves_at_the_same_tick_everywhere() {
    const LEAVES_AT: u64 = 200;
    for n in [2usize, 3, 4] {
        let w = wire(5, 6);
        let mut nets = peers(n, 3, &w, Duration::from_secs(10));
        let leaver = n - 1;
        let mut got = vec![Vec::new(); n];
        let until = Instant::now() + LONG_ENOUGH;
        while Instant::now() < until {
            step(&w);
            for i in 0..n {
                // They go as soon as they may, not a tick later.
                if i == leaver && nets[i].may_go() {
                    continue;
                }
                nets[i].poll();
                if nets[i].wants_input() {
                    let tick = nets[i].frame();
                    nets[i].give(pressed(i, tick), || 0x1234).unwrap();
                }
                if let Some(f) = nets[i].tick().unwrap() {
                    got[i].push(f);
                    // The tick in which everyone saw them choose to go.
                    if nets[i].frame() == LEAVES_AT {
                        if i == leaver {
                            nets[i].leave();
                        } else {
                            nets[i].drop_player(leaver);
                        }
                    }
                }
                for e in nets[i].events() {
                    assert!(!matches!(e, NetEvent::Lost { .. }), "{n}: {e:?}");
                }
            }
            if (0..leaver).all(|i| nets[i].frame() >= 600) {
                break;
            }
        }
        for i in 0..leaver {
            assert!(nets[i].frame() >= 600, "{n} players: {i} stuck");
            assert_eq!(got[i][..600], got[0][..600], "{n} players: machine {i}");
            assert_eq!(
                got[i][LEAVES_AT as usize - 1].inputs[leaver],
                pressed(leaver, LEAVES_AT - 4),
                "their last tick"
            );
            assert_eq!(
                got[i][LEAVES_AT as usize].inputs[leaver],
                PackedInput::default(),
                "the tick after"
            );
        }
    }
}

fn data() -> Option<GameFs> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game-data");
    GameFs::open(&root).ok()
}

fn party() -> Vec<PanelInfo> {
    (0..2)
        .map(|slot| PanelInfo {
            slot,
            name: String::new(),
            colour: slot,
            class: slot,
            hidden: None,
        })
        .collect()
}

/// The real game, two copies on a bad line: each hero played by the bot from
/// that machine's own copy of the game, as a person would, seeing only their
/// own screen. Their games must not differ at any point.
#[test]
fn two_games_on_a_bad_line_stay_the_same_game() {
    let Some(fs) = data() else { return };
    let w = wire(10, 8);
    let mut nets = peers(2, 3, &w, GIVE_UP_AFTER);
    let start = |fs: &GameFs| {
        let mut s = Session::new(Rules::classic(), 1, 0, Progress::default());
        s.enter_level("levelA1".to_string(), party());
        s.load(fs).unwrap();
        s
    };
    let mut games = [start(&fs), start(&fs)];
    let frames = 1500;
    let mut prints: [HashMap<u64, u32>; 2] = Default::default();
    let until = Instant::now() + LONG_ENOUGH;
    while Instant::now() < until {
        step(&w);
        for i in 0..2 {
            nets[i].poll();
            if nets[i].wants_input() {
                let input = match &games[i].game {
                    Some(g) => g.bot_input(i).pack(),
                    None => PackedInput::default(),
                };
                let game = &games[i];
                nets[i].give(input, || game.fingerprint()).unwrap();
            }
            if let Some(f) = nets[i].tick().unwrap() {
                games[i].update(&fs, &f.inputs);
                prints[i].insert(nets[i].frame(), games[i].fingerprint());
            }
            for e in nets[i].events() {
                assert!(!matches!(e, NetEvent::Desync { .. }), "{e:?}");
            }
        }
        if nets.iter().all(|n| n.frame() >= frames) {
            break;
        }
    }
    assert!(nets[0].frame() >= frames, "only {} ticks", nets[0].frame());
    for t in 1..=frames {
        assert_eq!(prints[0][&t], prints[1][&t], "tick {t}");
    }
    assert!(matches!(games[0].phase, Phase::Playing));
    // The deep digest as well, once both have stopped at the same tick.
    let t = nets[0].frame().min(nets[1].frame());
    if nets[0].frame() == t && nets[1].frame() == t {
        assert_eq!(games[0].digest(), games[1].digest());
    }
}

fn compat(build: &str) -> Compat {
    Compat {
        build: build.to_string(),
        content: 1,
        rules: 2,
        env: 3,
    }
}

fn free_port() -> u16 {
    // Free on both protocols, as the host wants it.
    loop {
        let t = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = t.local_addr().unwrap().port();
        if std::net::UdpSocket::bind(("0.0.0.0", port)).is_ok() {
            return port;
        }
    }
}

#[test]
fn a_lobby_agrees_on_who_is_where() {
    let port = free_port();
    let h = lobby::Host {
        port,
        players: 3,
        delay: None,
        difficulty: 2,
        friendly_fire: 1,
        compat: compat("v1"),
        controls: PlayerPrefs {
            control_style: 2,
            ..Default::default()
        },
    };
    let host = std::thread::spawn(move || lobby::host(&h, |_| {}).unwrap());
    let there = format!("127.0.0.1:{port}");
    let joiner = |style: usize| {
        let there = there.clone();
        std::thread::spawn(move || {
            let c = PlayerPrefs {
                control_style: style,
                ..Default::default()
            };
            lobby::join(&there, &compat("v1"), &c, |_| {}).unwrap()
        })
    };
    let (a, b) = (joiner(1), joiner(0));
    let host = host.join().unwrap();
    let (a, b) = (a.join().unwrap(), b.join().unwrap());
    let mut mine = [host.me, a.me, b.me];
    mine.sort();
    assert_eq!(mine, [0, 1, 2]);
    assert_eq!(host.addrs.len(), 3);
    for r in [&a, &b] {
        assert_eq!(r.delay, host.delay);
        assert_eq!(r.difficulty, 2);
        assert_eq!(r.friendly_fire, 1);
        assert_eq!(r.controls, host.controls);
        // Each is told the others' true addresses, and its own port is its own.
        assert_eq!(r.addrs[0].port(), port);
        for p in 1..3 {
            assert_eq!(r.addrs[p], host.addrs[p]);
        }
        assert_eq!(host.addrs[r.me].port(), r.port);
    }
    assert_eq!(host.controls[0].control_style, 2);
    assert!(host.delay >= 2);
}

#[test]
fn a_different_copy_of_the_game_is_turned_away_and_the_host_waits_on() {
    let port = free_port();
    let h = lobby::Host {
        port,
        players: 2,
        delay: Some(4),
        difficulty: 1,
        friendly_fire: 0,
        compat: compat("v1"),
        controls: PlayerPrefs::default(),
    };
    let host = std::thread::spawn(move || lobby::host(&h, |_| {}).unwrap());
    let there = format!("127.0.0.1:{port}");
    let refused = lobby::join(&there, &compat("v2"), &PlayerPrefs::default(), |_| {});
    let why = format!("{:#}", refused.unwrap_err());
    assert!(why.contains("different builds"), "{why}");
    let ok = lobby::join(&there, &compat("v1"), &PlayerPrefs::default(), |_| {}).unwrap();
    assert_eq!(ok.delay, 4);
    assert_eq!(host.join().unwrap().delay, 4);
}

#[test]
fn over_real_sockets_two_machines_run_together() {
    let port = free_port();
    let h = lobby::Host {
        port,
        players: 2,
        delay: None,
        difficulty: 1,
        friendly_fire: 0,
        compat: compat("v1"),
        controls: PlayerPrefs::default(),
    };
    let host = std::thread::spawn(move || lobby::host(&h, |_| {}).unwrap());
    let there = format!("127.0.0.1:{port}");
    let them = lobby::join(&there, &compat("v1"), &PlayerPrefs::default(), |_| {}).unwrap();
    let us = host.join().unwrap();
    let play = |roster: Roster| {
        std::thread::spawn(move || {
            let mut net = Net::start(&roster).unwrap();
            let mut got = Vec::new();
            let until = Instant::now() + Duration::from_secs(20);
            while net.frame() < 300 && Instant::now() < until {
                net.poll();
                if net.wants_input() {
                    let t = net.frame();
                    net.give(pressed(roster.me, t), || 7).unwrap();
                }
                if let Some(f) = net.tick().unwrap() {
                    got.push(f);
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            // Stay on the line until the other has what it needs.
            let linger = Instant::now() + Duration::from_millis(500);
            while Instant::now() < linger {
                net.poll();
                std::thread::sleep(Duration::from_millis(1));
            }
            got
        })
    };
    let (a, b) = (play(us), play(them));
    let (a, b) = (a.join().unwrap(), b.join().unwrap());
    assert!(a.len() >= 300 && b.len() >= 300, "{} {}", a.len(), b.len());
    assert_eq!(a[..300], b[..300]);
}
