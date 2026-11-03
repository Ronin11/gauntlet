//! Finding the other players, before there is a game: everyone connects to the
//! host, who checks that they can play together (the same build, game files and
//! rules), says how much input delay the connections need, and tells everyone
//! who is where. A plain text conversation over TCP; the game itself then runs
//! over UDP on the same port number.

use anyhow::{anyhow, bail, Context, Result};
use gdl_sim::compat::Compat;
use gdl_sim::input::PlayerPrefs;
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs, UdpSocket};
use std::time::{Duration, Instant};

/// The conversation's first line.
const HELLO: &str = "gdl-net 1";
/// The most input delay the host will choose for itself (a quarter second).
const MAX_DELAY: usize = 15;
/// How long the host waits for a joiner's next line.
const REPLY_WITHIN: Duration = Duration::from_secs(10);
/// How long a joiner keeps trying to reach the host.
const CONNECT_WITHIN: Duration = Duration::from_secs(30);
/// Sends that say which UDP address a joiner's packets come from (through any
/// router), and keep the way open until the game starts.
const PUNCH: &str = "gdl-punch";
const PUNCH_EVERY: Duration = Duration::from_millis(200);

/// What the host decides and every machine then shares.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Roster {
    /// This machine's player number.
    pub me: usize,
    /// The UDP port this machine plays on.
    pub port: u16,
    /// Where each player is (this machine's own entry is not used).
    pub addrs: Vec<SocketAddr>,
    /// Ticks between pressing a button and the game seeing it.
    pub delay: usize,
    pub difficulty: usize,
    pub friendly_fire: usize,
    /// Each player's Controls options.
    pub controls: Vec<PlayerPrefs>,
}

/// The host's side of the game.
#[derive(Clone, Debug)]
pub struct Host {
    pub port: u16,
    /// Players, with the host.
    pub players: usize,
    /// Input delay to use, instead of what the connections suggest.
    pub delay: Option<usize>,
    pub difficulty: usize,
    pub friendly_fire: usize,
    pub compat: Compat,
    pub controls: PlayerPrefs,
}

/// The input delay for a round trip: there and a little to spare.
pub fn delay_for(rtt: Duration) -> usize {
    let one_way = rtt.as_secs_f32() / 2.0;
    (((one_way * 60.0).ceil() as usize) + 2).min(MAX_DELAY)
}

fn compat_line(c: &Compat) -> String {
    format!(
        "compat {:016x} {:016x} {:016x} {}",
        c.content, c.rules, c.env, c.build
    )
}

fn parse_compat(rest: &str) -> Result<Compat> {
    let mut w = rest.splitn(4, ' ');
    let mut hex = || -> Result<u64> {
        let word = w.next().context("short compat line")?;
        Ok(u64::from_str_radix(word, 16)?)
    };
    let (content, rules, env) = (hex()?, hex()?, hex()?);
    Ok(Compat {
        content,
        rules,
        env,
        build: w.next().unwrap_or_default().to_string(),
    })
}

fn controls_text(p: &PlayerPrefs) -> String {
    format!(
        "{} {} {} {}",
        p.control_style, p.rumble as u8, p.auto_aim as u8, p.auto_attack as u8
    )
}

fn parse_controls(rest: &str) -> Result<PlayerPrefs> {
    let w: Vec<&str> = rest.split(' ').collect();
    let [style, rumble, aim, attack] = w[..] else {
        bail!("controls wants 4 values");
    };
    Ok(PlayerPrefs {
        control_style: style.parse()?,
        rumble: rumble == "1",
        auto_aim: aim == "1",
        auto_attack: attack == "1",
    })
}

/// A connection, a line at a time.
struct Wire {
    r: BufReader<TcpStream>,
    w: TcpStream,
}

impl Wire {
    fn new(s: TcpStream) -> Result<Wire> {
        s.set_nodelay(true)?;
        Ok(Wire {
            r: BufReader::new(s.try_clone()?),
            w: s,
        })
    }

    fn say(&mut self, line: &str) -> Result<()> {
        self.w.write_all(line.as_bytes())?;
        self.w.write_all(b"\n")?;
        Ok(())
    }

    fn timeout(&mut self, t: Option<Duration>) -> Result<()> {
        self.w.set_read_timeout(t)?;
        Ok(())
    }

    /// The next line; None if the timeout passed first.
    fn try_hear(&mut self) -> Result<Option<String>> {
        let mut line = String::new();
        match self.r.read_line(&mut line) {
            Ok(0) => bail!("the connection closed"),
            Ok(_) => Ok(Some(line.trim_end().to_string())),
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn hear(&mut self) -> Result<String> {
        self.try_hear()?.context("no reply in time")
    }

    /// The next line, which must be `key ...`: the rest of it.
    fn expect(&mut self, key: &str) -> Result<String> {
        let line = self.hear()?;
        match line.split_once(' ') {
            Some((k, rest)) if k == key => Ok(rest.to_string()),
            _ if line == key => Ok(String::new()),
            _ => bail!("expected {key}, heard {line:?}"),
        }
    }
}

/// A joiner the host has let in.
struct Joined {
    wire: Wire,
    addr: SocketAddr,
    rtt: Duration,
    controls: PlayerPrefs,
}

/// Bring one joiner in: their copy of the game, the round trip, and where their
/// UDP packets come from.
fn admit(stream: TcpStream, h: &Host, udp: &UdpSocket, player: usize) -> Result<Joined> {
    stream.set_nonblocking(false)?;
    let mut wire = Wire::new(stream)?;
    wire.timeout(Some(REPLY_WITHIN))?;
    let hello = wire.hear()?;
    if hello != HELLO {
        let _ = wire.say(&format!("refuse the host speaks {HELLO}, not {hello}"));
        bail!("wrong version: {hello}");
    }
    let theirs = parse_compat(&wire.expect("compat")?)?;
    if let Err(why) = h.compat.check(&theirs) {
        let _ = wire.say(&format!("refuse {why}"));
        bail!("{why}");
    }
    let controls = parse_controls(&wire.expect("controls")?)?;
    wire.say("ok")?;
    // The round trip, the best of a few.
    let mut rtt = Duration::MAX;
    for _ in 0..4 {
        let sent = Instant::now();
        wire.say("ping")?;
        wire.expect("pong")?;
        rtt = rtt.min(sent.elapsed());
    }
    // Their UDP address as this side sees it.
    let token = format!("{PUNCH} {player}");
    wire.say(&format!("punch {token}"))?;
    let until = Instant::now() + REPLY_WITHIN;
    let mut buf = [0u8; 64];
    let addr = loop {
        match udp.recv_from(&mut buf) {
            Ok((n, from)) if buf[..n] == *token.as_bytes() => break from,
            Ok(_) => {}
            Err(e) if e.kind() == ErrorKind::WouldBlock => {
                if Instant::now() > until {
                    let _ = wire.say("refuse no UDP from you reached the host");
                    bail!("no UDP arrived from player {player}");
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(e) if e.kind() == ErrorKind::ConnectionReset => {}
            Err(e) => return Err(e.into()),
        }
    };
    wire.say("wait")?;
    wire.timeout(None)?;
    Ok(Joined {
        wire,
        addr,
        rtt,
        controls,
    })
}

/// Wait for the players, as the host. `say` hears how it is going.
pub fn host(h: &Host, mut say: impl FnMut(&str)) -> Result<Roster> {
    anyhow::ensure!((2..=4).contains(&h.players), "2 to 4 players can play");
    let tcp = TcpListener::bind(("0.0.0.0", h.port))
        .with_context(|| format!("port {} is taken", h.port))?;
    tcp.set_nonblocking(true)?;
    let udp = UdpSocket::bind(("0.0.0.0", h.port))
        .with_context(|| format!("UDP port {} is taken", h.port))?;
    udp.set_nonblocking(true)?;
    let mut joined: Vec<Joined> = Vec::new();
    say(&format!(
        "Waiting for {} more on port {} (TCP and UDP)",
        h.players - 1,
        h.port
    ));
    while joined.len() + 1 < h.players {
        match tcp.accept() {
            Ok((stream, from)) => match admit(stream, h, &udp, joined.len() + 1) {
                Ok(j) => {
                    say(&format!(
                        "Player {} joined from {} ({} ms)",
                        joined.len() + 2,
                        j.addr,
                        j.rtt.as_millis()
                    ));
                    joined.push(j);
                }
                Err(e) => say(&format!("Turned away {from}: {e:#}")),
            },
            Err(e) if e.kind() == ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(20));
                // Whoever gave up while waiting doesn't hold the game up.
                let before = joined.len();
                joined.retain_mut(still_there);
                if joined.len() != before {
                    say("A player left the lobby");
                }
            }
            Err(e) => return Err(e.into()),
        }
    }
    let worst = joined.iter().map(|j| j.rtt).max().unwrap_or(Duration::ZERO);
    let delay = h.delay.unwrap_or_else(|| delay_for(worst)).min(60);
    say(&format!("Everyone's here: {delay} ticks of input delay"));
    let host_addr: SocketAddr = ([127, 0, 0, 1], h.port).into();
    let mut addrs = vec![host_addr];
    addrs.extend(joined.iter().map(|j| j.addr));
    let mut controls = vec![h.controls];
    controls.extend(joined.iter().map(|j| j.controls));
    for (i, j) in joined.iter_mut().enumerate() {
        let w = &mut j.wire;
        w.say(&format!(
            "start {} {} {delay} {} {}",
            i + 1,
            addrs.len(),
            h.difficulty,
            h.friendly_fire
        ))?;
        for (p, addr) in addrs.iter().enumerate().skip(1) {
            w.say(&format!("peer {p} {addr}"))?;
        }
        for (p, c) in controls.iter().enumerate() {
            w.say(&format!("player {p} {}", controls_text(c)))?;
        }
        w.say("go")?;
    }
    Ok(Roster {
        me: 0,
        port: h.port,
        addrs,
        delay,
        difficulty: h.difficulty,
        friendly_fire: h.friendly_fire,
        controls,
    })
}

/// Is this joiner still connected? They say nothing while they wait.
fn still_there(j: &mut Joined) -> bool {
    let s = &j.wire.w;
    if s.set_nonblocking(true).is_err() {
        return false;
    }
    let mut buf = [0u8; 1];
    let there = match s.peek(&mut buf) {
        Ok(0) => false,
        Ok(_) => true,
        Err(e) => e.kind() == ErrorKind::WouldBlock,
    };
    let _ = s.set_nonblocking(false);
    there
}

/// Connect to a host at `addr` (`host:port`). Blocks until the game starts.
pub fn join(
    addr: &str,
    compat: &Compat,
    controls: &PlayerPrefs,
    mut say: impl FnMut(&str),
) -> Result<Roster> {
    let host_addr = addr
        .to_socket_addrs()
        .with_context(|| format!("no such host: {addr}"))?
        .next()
        .ok_or_else(|| anyhow!("no such host: {addr}"))?;
    say(&format!("Connecting to {host_addr}"));
    let until = Instant::now() + CONNECT_WITHIN;
    let stream = loop {
        match TcpStream::connect_timeout(&host_addr, Duration::from_secs(3)) {
            Ok(s) => break s,
            Err(e) if Instant::now() > until => {
                return Err(e).with_context(|| format!("nobody answered at {host_addr}"));
            }
            Err(_) => std::thread::sleep(Duration::from_millis(250)),
        }
    };
    let udp = UdpSocket::bind(("0.0.0.0", 0))?;
    let port = udp.local_addr()?.port();
    let mut wire = Wire::new(stream)?;
    wire.timeout(Some(REPLY_WITHIN))?;
    wire.say(HELLO)?;
    wire.say(&compat_line(compat))?;
    wire.say(&format!("controls {}", controls_text(controls)))?;
    let reply = |line: &str| -> Result<()> {
        match line.strip_prefix("refuse ") {
            Some(why) => bail!("the host refused: {why}"),
            None => Ok(()),
        }
    };
    let first = wire.hear()?;
    reply(&first)?;
    anyhow::ensure!(first == "ok", "unexpected reply: {first}");
    // The round trips, then proof of where our UDP comes from.
    let token = loop {
        let line = wire.hear()?;
        reply(&line)?;
        match line.split_once(' ') {
            Some(("punch", token)) => break token.to_string(),
            _ if line == "ping" => wire.say("pong")?,
            _ => bail!("unexpected reply: {line}"),
        }
    };
    // Keep sending until the host has it, then until the game starts.
    wire.timeout(Some(PUNCH_EVERY))?;
    let start = loop {
        let _ = udp.send_to(token.as_bytes(), host_addr);
        match wire.try_hear()? {
            None => continue,
            Some(line) if line == "wait" => say("Waiting for the other players"),
            Some(line) => {
                reply(&line)?;
                break line;
            }
        }
    };
    wire.timeout(Some(REPLY_WITHIN))?;
    let words: Vec<&str> = start
        .strip_prefix("start ")
        .with_context(|| format!("unexpected reply: {start}"))?
        .split(' ')
        .collect();
    let [me, players, delay, difficulty, friendly_fire] = words[..] else {
        bail!("short start line: {start}");
    };
    let players: usize = players.parse()?;
    let mut addrs = vec![host_addr; players];
    let mut controls = vec![PlayerPrefs::default(); players];
    loop {
        let line = wire.hear()?;
        if line == "go" {
            break;
        }
        let mut w = line.splitn(3, ' ');
        let key = w.next().unwrap_or_default();
        let p: usize = w.next().context("short line")?.parse()?;
        let rest = w.next().context("short line")?;
        anyhow::ensure!(p < players, "no player {p}");
        match key {
            "peer" => addrs[p] = rest.parse()?,
            "player" => controls[p] = parse_controls(rest)?,
            _ => bail!("unexpected line: {line}"),
        }
    }
    Ok(Roster {
        me: me.parse()?,
        port,
        addrs,
        delay: delay.parse()?,
        difficulty: difficulty.parse()?,
        friendly_fire: friendly_fire.parse()?,
        controls,
    })
}
