//! The level runner's player: picks objectives the way a player would (the
//! exit; else keys, doors, levers and pads, breakables, then anything left
//! to pick up), walks there over the nav graph, fights what comes near, and
//! notices when it gets nowhere.

use super::nav::{xz, Goal, Nav};
use crate::game::{EnemyState, Game, Kind, LevelEnd, PickupKind, PlayerState, PLAYER_RADIUS};
use crate::input::PlayerInput;
use glam::{Vec2, Vec3};
use super::nav::{HashMap, HashSet};
use std::collections::VecDeque;

/// What a hero is after.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Obj {
    Exit(u64),
    Key(u64),
    Door(u64),
    Trigger(u64),
    Smash(u64),
    Pickup(u64),
    Food(u64),
    Boss,
    Lectern,
    Portal(u64),
    /// A rotator (items.c type 12, subtype 2) a touch sets turning: its
    /// target world object.
    Rotator(u64),
    /// Wait by a moving platform (world object) for it to come round.
    Wait(u64),
    /// Run for what a held pad (trigger id) opens before it shuts again.
    Run(u64),
    Follow,
    /// Nothing to do: stand (and fight).
    Idle,
}

impl Obj {
    pub fn name(&self) -> String {
        match self {
            Obj::Exit(i) => format!("exit#{i}"),
            Obj::Key(i) => format!("key#{i}"),
            Obj::Door(i) => format!("door#{i}"),
            Obj::Trigger(i) => format!("trigger#{i}"),
            Obj::Smash(i) => format!("smash#{i}"),
            Obj::Pickup(i) => format!("pickup#{i}"),
            Obj::Food(i) => format!("food#{i}"),
            Obj::Boss => "boss".into(),
            Obj::Lectern => "lectern".into(),
            Obj::Portal(i) => format!("portal#{i}"),
            Obj::Rotator(i) => format!("rotator#{i}"),
            Obj::Wait(i) => format!("wait-platform#{i}"),
            Obj::Run(i) => format!("run-from-trigger#{i}"),
            Obj::Follow => "follow".into(),
            Obj::Idle => "idle".into(),
        }
    }
    fn entity(&self) -> Option<u64> {
        match *self {
            Obj::Exit(i)
            | Obj::Key(i)
            | Obj::Door(i)
            | Obj::Trigger(i)
            | Obj::Smash(i)
            | Obj::Pickup(i)
            | Obj::Food(i)
            | Obj::Portal(i) => Some(i),
            _ => None,
        }
    }
}

/// How a level run ended.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Completed,
    Stuck(String),
    Unreachable(String),
    Timeout(String),
    Died(String),
    ExitNoEnd(String),
    Nan(String),
    /// Fell out of the world over and over.
    FellOut(String),
    /// Left through a secret icon (not a failure of the level).
    Secret(u32),
}

impl Outcome {
    pub fn tag(&self) -> &'static str {
        match self {
            Outcome::Completed => "completed",
            Outcome::Stuck(_) => "stuck",
            Outcome::Unreachable(_) => "unreachable",
            Outcome::Timeout(_) => "timeout",
            Outcome::Died(_) => "died",
            Outcome::ExitNoEnd(_) => "exit_no_end",
            Outcome::Nan(_) => "nan",
            Outcome::FellOut(_) => "fell_out",
            Outcome::Secret(_) => "secret",
        }
    }
    pub fn detail(&self) -> String {
        match self {
            Outcome::Completed => String::new(),
            Outcome::Secret(w) => format!("secret level {w:#x}"),
            Outcome::Stuck(s)
            | Outcome::Unreachable(s)
            | Outcome::Timeout(s)
            | Outcome::Died(s)
            | Outcome::ExitNoEnd(s)
            | Outcome::Nan(s)
            | Outcome::FellOut(s) => s.clone(),
        }
    }
}

/// One hero's plan.
#[derive(Default)]
pub struct Agent {
    pub obj: Option<Obj>,
    pub obj_t: f32,
    /// Where the objective is (followed directly once the path is done).
    pub target: Option<Vec3>,
    pub goals: Vec<Goal>,
    pub path: Vec<u32>,
    pub wp: usize,
    pub last_adv: f32,
    pub best_d: f32,
    pub stucks: u32,
    /// Arrived at a trigger / door / exit: since when.
    pub arrived: Option<f32>,
    pub fight_t: f32,
    pub calm_until: f32,
    pub potion_t: f32,
    pub replan: bool,
    pub on_pad_t: Option<f32>,
    pub hunt_t: f32,
    pub stuck_wp: usize,
    pub plan_t: f32,
}

/// Something worth a bug report found on the way (not necessarily the end).
pub struct Anomaly {
    pub what: String,
    pub tags: Vec<&'static str>,
    /// Capture now (the runner files the report with the game as it is).
    pub capture: bool,
}

pub struct Bot {
    pub agents: Vec<Agent>,
    pub nav: Nav,
    /// The graph with doors open and breakables gone.
    pub opt: Option<Nav>,
    pub tried: HashMap<Obj, u32>,
    pub failed: HashSet<Obj>,
    pub history: Vec<String>,
    pub trail: VecDeque<(f32, Vec3)>,
    pub god_at: Option<(f32, String)>,
    pub boss_killed_debug: bool,
    pub boss_fight_t: f32,
    pub anomalies: Vec<Anomaly>,
    pub reach_mark: usize,
    pub lectern_ok: Option<bool>,
    pub portals_reachable: Vec<String>,
    pub tower: bool,
    pub timeout: f32,
    pub exit_since: Option<f32>,
    pub used: Vec<String>,
    last_pos: Vec<Vec3>,
    falls: u32,
    fall_times: Vec<f32>,
    pub verbose: bool,
    giveup_t: Option<f32>,
    /// Seconds spent waiting by moving platforms, in all and per object.
    platform_wait: f32,
    mover_wait: f32,
    unblocks: u32,
    rounds: u32,
    anchor: (Vec3, f32),
    run_to: HashMap<u64, Vec3>,
    ran: HashSet<u64>,
    waited: HashMap<u64, f32>,
    boss_fail_t: f32,
    /// Wall seconds: inputs, planning, lookahead, nav refresh.
    pub timing: [f64; 4],
}

const MAX_EXPAND: usize = 300_000;
const STUCK_SECS: f32 = 3.0;
const STUCKS_PER_OBJ: u32 = 6;

impl Bot {
    pub fn new(g: &Game, timeout: f32, tower: bool, verbose: bool) -> Bot {
        let t0 = std::time::Instant::now();
        let nav = Nav::new(g);
        if verbose {
            println!(
                "nav: {} nodes ({}x{} cells) in {:.0} ms",
                nav.live_nodes(),
                nav.nx,
                nav.nz,
                t0.elapsed().as_secs_f64() * 1e3
            );
        }
        if verbose {
            for e in &g.entities {
                match &e.kind {
                    Kind::Exit { open, dest, radius, .. } => println!(
                        "exit #{} at {:?} r {radius:.1} open {open} dest {dest:?}",
                        e.id, e.pos
                    ),
                    Kind::Trigger { mover, mode, radius, shoot, flags, id, .. } => println!(
                        "trigger #{} id {id} at {:?} r {radius:.1} mover {mover:?} {mode:?} shoot {shoot} flags {flags:#x}",
                        e.id, e.pos
                    ),
                    Kind::Door { radius, .. } => println!("door #{} at {:?} r {radius:.1}", e.id, e.pos),
                    Kind::Secret { radius, wave, .. } => {
                        println!("secret #{} at {:?} r {radius:.1} -> {wave:?}", e.id, e.pos)
                    }
                    _ => {}
                }
            }
            for (i, m) in g.movers.iter().enumerate() {
                println!(
                    "mover {i} root {} at {:?} active {} offset {:.1} shown {}",
                    m.root, m.pos, m.active, m.offset, m.shown
                );
            }
            for (i, p) in g.transport.pads.iter().enumerate() {
                let to = p.dest.map(|d| g.transport.pads[d].pos);
                let ground = to.map(|to| g.collision.ground_height(to, to.y + 4.0));
                println!(
                    "pad {i} id {} at {:?} r {:.1} -> {:?} {:?} ground there {:?}",
                    p.id, p.pos, p.radius, p.dest, to, ground
                );
            }
        }
        Bot {
            agents: (0..g.players.len()).map(|_| Agent::default()).collect(),
            nav,
            opt: None,
            tried: HashMap::default(),
            failed: HashSet::default(),
            history: Vec::new(),
            trail: VecDeque::new(),
            god_at: None,
            boss_killed_debug: false,
            boss_fight_t: 0.0,
            anomalies: Vec::new(),
            reach_mark: 0,
            lectern_ok: None,
            portals_reachable: Vec::new(),
            tower,
            timeout,
            exit_since: None,
            used: Vec::new(),
            last_pos: g.players.iter().map(|p| p.pos).collect(),
            falls: 0,
            fall_times: Vec::new(),
            verbose,
            giveup_t: None,
            platform_wait: 0.0,
            mover_wait: 0.0,
            unblocks: 0,
            rounds: 0,
            anchor: (Vec3::splat(1e9), 0.0),
            run_to: HashMap::default(),
            ran: HashSet::default(),
            waited: HashMap::default(),
            boss_fail_t: -100.0,
            timing: [0.0; 4],
        }
    }

    fn log(&mut self, g: &Game, s: String) {
        log_to(&mut self.history, self.verbose, g, s);
    }

    // ---- the world, as the runner reads it ---------------------------------

    fn ent<'a>(g: &'a Game, id: u64) -> Option<&'a crate::game::Entity> {
        g.entities.iter().find(|e| e.id == id && e.alive)
    }

    fn boss_level(g: &Game) -> bool {
        g.boss.is_some()
    }

    fn leader(g: &Game) -> Option<usize> {
        g.players.iter().position(|p| p.state.active())
    }

    /// The goals of an objective, and the spot to steer to at the end.
    fn goals_of(&self, g: &Game, o: Obj) -> Option<(Vec<Goal>, Vec3)> {
        let e = o.entity().and_then(|id| Self::ent(g, id));
        Some(match (o, e) {
            (Obj::Exit(_) | Obj::Portal(_), Some(e)) => match e.kind {
                Kind::Exit { radius, height, .. } => (
                    vec![Goal::new(e.pos, (radius * 0.7).max(0.8), height.max(2.0))],
                    e.pos,
                ),
                _ => return None,
            },
            (Obj::Key(_) | Obj::Pickup(_) | Obj::Food(_), Some(e)) => match e.kind {
                Kind::Pickup { radius, .. } => {
                    (vec![Goal::new(e.pos, (radius + 0.3).max(1.0), 3.0)], e.pos)
                }
                _ => return None,
            },
            (Obj::Door(_), Some(e)) => match e.kind {
                Kind::Door { radius, .. } => {
                    (vec![Goal::new(e.pos, radius + 1.0, 2.5)], e.pos)
                }
                _ => return None,
            },
            (Obj::Trigger(_), Some(e)) => match e.kind {
                Kind::Trigger { radius, shoot, .. } => {
                    let r = if shoot {
                        7.0
                    } else {
                        (radius + PLAYER_RADIUS + 0.45).max(1.0)
                    };
                    (vec![Goal::new(e.pos, r, 3.5)], e.pos)
                }
                _ => return None,
            },
            (Obj::Smash(_), Some(e)) if wall_points(g, e).is_some() => {
                // A breakable wall: up against its collision, anywhere along it.
                let pts = wall_points(g, e)?;
                let me = g.players.first().map_or(e.pos, |p| p.pos);
                let near = pts
                    .iter()
                    .copied()
                    .min_by(|a, b| (*a - me).length().total_cmp(&(*b - me).length()))?;
                (pts.iter().map(|&p| Goal::new(p, PLAYER_RADIUS + 1.6, 4.0)).collect(), near)
            }
            (Obj::Smash(_), Some(e)) => {
                let r = solid_radius(e) + PLAYER_RADIUS + 1.2;
                (vec![Goal::new(e.pos, r, 3.0)], e.pos)
            }
            (Obj::Lectern, _) => {
                let e = g.entities.iter().find(|e| {
                    matches!(e.kind, Kind::Trigger { id, .. } if id == crate::tower::LECTERN)
                })?;
                let r = match e.kind {
                    Kind::Trigger { radius, .. } => radius + 1.0,
                    _ => 2.0,
                };
                (vec![Goal::new(e.pos, r, 3.5)], e.pos)
            }
            (Obj::Run(t), _) => {
                let at = *self.run_to.get(&t)?;
                (vec![Goal::new(at, 2.0, 2.5)], at)
            }
            (Obj::Rotator(t), _) => {
                let (at, r, _) = g
                    .world_anim
                    .untouched_rotators()
                    .into_iter()
                    .find(|x| x.2 as u64 == t)?;
                (vec![Goal::new(at, (r + PLAYER_RADIUS - 0.3).max(1.0), 2.5)], at)
            }
            (Obj::Boss, _) => {
                let (at, r) = boss_point(g, g.players[0].pos)?;
                (vec![Goal::new(at, r + 4.0, 12.0)], at)
            }
            _ => return None,
        })
    }

    // ---- choosing what to do ------------------------------------------------

    /// A path for hero `i` to the nearest of `cands` (objective, goals).
    fn plan_any(
        &mut self,
        g: &Game,
        i: usize,
        cands: &[(Obj, Vec<Goal>, Vec3)],
    ) -> Option<(Obj, Vec<u32>, f32)> {
        let t0 = std::time::Instant::now();
        let r = self.plan_any_inner(g, i, cands);
        self.timing[1] += t0.elapsed().as_secs_f64();
        r
    }

    fn plan_any_inner(
        &mut self,
        g: &Game,
        i: usize,
        cands: &[(Obj, Vec<Goal>, Vec3)],
    ) -> Option<(Obj, Vec<u32>, f32)> {
        if cands.is_empty() {
            return None;
        }
        let p = g.players[i].pos;
        let start = self.nav.start_node(g, p)?;
        let goals: Vec<Goal> = cands.iter().flat_map(|c| c.1.iter().copied()).collect();
        let (path, cost) = self.nav.path(g, start, &goals, g.time, MAX_EXPAND)?;
        let end = self.nav.nodes[*path.last()? as usize].pos;
        let (o, ..) = cands
            .iter()
            .filter(|c| c.1.iter().any(|gl| gl.holds(end)))
            .min_by(|a, b| xz(a.2 - end).length().total_cmp(&xz(b.2 - end).length()))?;
        Some((*o, path, cost))
    }

    fn cands(&self, g: &Game, f: impl Fn(&crate::game::Entity) -> Option<Obj>) -> Vec<(Obj, Vec<Goal>, Vec3)> {
        g.entities
            .iter()
            .filter(|e| e.alive)
            .filter_map(&f)
            .filter(|o| !self.failed.contains(o))
            .filter_map(|o| self.goals_of(g, o).map(|(gl, at)| (o, gl, at)))
            .collect()
    }

    fn set_obj(&mut self, g: &Game, i: usize, o: Obj, path: Vec<u32>, cost: f32) {
        let (goals, target) = self.goals_of(g, o).unwrap_or((Vec::new(), g.players[i].pos));
        let a = &mut self.agents[i];
        let same = a.obj == Some(o);
        a.obj = Some(o);
        a.goals = goals;
        a.target = Some(target);
        a.path = path;
        a.wp = 0;
        a.last_adv = g.time;
        a.best_d = f32::MAX;
        a.replan = false;
        a.plan_t = g.time;
        if !same {
            a.obj_t = g.time;
            a.stucks = 0;
            a.arrived = None;
            a.fight_t = 0.0;
            if i == 0 {
                let line = format!(
                    "hero {i} -> {} at ({:.1},{:.1},{:.1}) path {} nodes cost {:.0}",
                    o.name(),
                    target.x,
                    target.y,
                    target.z,
                    self.agents[i].path.len(),
                    cost
                );
                self.log(g, line);
            }
        }
    }

    /// The leader's next objective.
    fn decide_leader(&mut self, g: &mut Game, i: usize) -> Result<(), Outcome> {
        let health = g.players[i].health / crate::game::max_health(g.players[i].level);
        // Boss levels: the boss, then wait for the good wizard.
        if Self::boss_level(g) {
            let dead = g.boss.as_ref().is_some_and(|b| b.dead || b.dying);
            if !dead {
                let c = self.goals_of(g, Obj::Boss);
                if let Some((gl, at)) = c {
                    let cands = vec![(Obj::Boss, gl, at)];
                    // A boss out of walking reach (flying, in a pit) is steered
                    // at directly; the search is retried only now and then.
                    let retry = g.time - self.boss_fail_t > 15.0;
                    let r = if retry { self.plan_any(g, i, &cands) } else { None };
                    match r {
                        Some((o, path, cost)) => self.set_obj(g, i, o, path, cost),
                        None => {
                            if retry {
                                self.boss_fail_t = g.time;
                            }
                            self.set_obj(g, i, Obj::Boss, Vec::new(), 0.0)
                        }
                    }
                    return Ok(());
                }
            }
            // Coins and items the boss spewed, then stand for the wizard.
            let cands = self.cands(g, |e| match e.kind {
                Kind::Pickup { .. } if xz(e.pos - g.players[i].pos).length() < 30.0 => {
                    Some(Obj::Pickup(e.id))
                }
                _ => None,
            });
            if let Some((o, path, cost)) = self.plan_any(g, i, &cands) {
                if cost < 40.0 {
                    self.set_obj(g, i, o, path, cost);
                    return Ok(());
                }
            }
            self.set_obj(g, i, Obj::Idle, Vec::new(), 0.0);
            return Ok(());
        }
        // The tower: the lectern first, then the nearest open portal.
        if self.tower {
            if self.lectern_ok.is_none() && !self.failed.contains(&Obj::Lectern) {
                if let Some((gl, at)) = self.goals_of(g, Obj::Lectern) {
                    if let Some((o, path, cost)) = self.plan_any(g, i, &[(Obj::Lectern, gl, at)]) {
                        self.set_obj(g, i, o, path, cost);
                        return Ok(());
                    }
                    self.log(g, "lectern unreachable".into());
                    self.failed.insert(Obj::Lectern);
                    self.lectern_ok = Some(false);
                }
            }
            {
                // Which open portals can be walked to at all.
                self.portals_reachable.clear();
                let portals = self.cands(g, |e| match e.kind {
                    Kind::Exit { open: true, dest: Some(_), .. } => Some(Obj::Portal(e.id)),
                    _ => None,
                });
                for c in &portals {
                    let dest = match Self::ent(g, c.0.entity().unwrap()).map(|e| &e.kind) {
                        Some(Kind::Exit { dest: Some(w), .. }) => *w,
                        _ => 0,
                    };
                    let ok = self.plan_any(g, i, std::slice::from_ref(c)).is_some();
                    self.portals_reachable.push(format!(
                        "{}:{}",
                        g.worlds.level(dest).map_or(format!("{dest:#x}"), |l| l.name.clone()),
                        if ok { "reachable" } else { "UNREACHABLE" }
                    ));
                }
            }
            let portals = self.cands(g, |e| match e.kind {
                Kind::Exit { open: true, dest: Some(_), .. } => Some(Obj::Portal(e.id)),
                _ => None,
            });
            if let Some((o, path, cost)) = self.plan_any(g, i, &portals) {
                self.set_obj(g, i, o, path, cost);
                return Ok(());
            }
            // A new party's way out: the tower's crystals open its first
            // barrier (the barrier opens to a touch once enough are found).
            let crystals = self.cands(g, |e| match e.kind {
                Kind::Pickup { kind: PickupKind::Crystal(_), .. } => Some(Obj::Pickup(e.id)),
                _ => None,
            });
            if let Some((o, path, cost)) = self.plan_any(g, i, &crystals) {
                self.set_obj(g, i, o, path, cost);
                return Ok(());
            }
            let tried_map = self.tried.clone();
            let barriers: Vec<_> = self
                .cands(g, |e| match e.kind {
                    Kind::Trigger { gate: Some(c), .. }
                        if c < 100 && crate::game::crystals_met(&g.crystals, c) =>
                    {
                        Some(Obj::Trigger(e.id))
                    }
                    _ => None,
                })
                .into_iter()
                .filter(|c| tried_map.get(&c.0).copied().unwrap_or(0) < 2)
                .collect();
            if let Some((o, path, cost)) = self.plan_any(g, i, &barriers) {
                self.set_obj(g, i, o, path, cost);
                return Ok(());
            }
            return Err(Outcome::Unreachable(format!(
                "no open tower portal is reachable (crystals {:?})",
                g.crystals
            )));
        }
        // Low on health: food close by first.
        if health < 0.5 {
            let cands = self.cands(g, |e| match e.kind {
                Kind::Pickup { kind: PickupKind::Food(v), .. } if v > 0.0 => Some(Obj::Food(e.id)),
                _ => None,
            });
            if let Some((o, path, cost)) = self.plan_any(g, i, &cands) {
                if cost < 50.0 {
                    self.set_obj(g, i, o, path, cost);
                    return Ok(());
                }
            }
        }
        // The exit.
        let exits = self.cands(g, |e| match e.kind {
            Kind::Exit { open: true, .. } => Some(Obj::Exit(e.id)),
            _ => None,
        });
        // A timed (secret) level: gather what is there until the time runs out.
        if exits.is_empty() && g.secret_timer.is_some() {
            let picks = self.cands(g, |e| match e.kind {
                Kind::Pickup { .. } => Some(Obj::Pickup(e.id)),
                _ => None,
            });
            if let Some((o, path, cost)) = self.plan_any(g, i, &picks) {
                self.set_obj(g, i, o, path, cost);
            } else {
                self.set_obj(g, i, Obj::Idle, Vec::new(), 0.0);
            }
            return Ok(());
        }
        if exits.is_empty() && !self.failed.iter().any(|o| matches!(o, Obj::Exit(_))) {
            return Err(Outcome::Unreachable("the level has no open exit".into()));
        }
        if let Some((o, path, cost)) = self.plan_any(g, i, &exits) {
            self.set_obj(g, i, o, path, cost);
            return Ok(());
        }
        // What stands between the hero and the exit: plan as if doors were
        // open and breakables broken; the first one on that path is next.
        if let Some(b) = self.first_blocker(g, i, &exits) {
            let keys: u32 = g.players.iter().map(|p| p.keys).sum();
            let is_door = matches!(Self::ent(g, b).map(|e| &e.kind), Some(Kind::Door { .. }));
            let o = if is_door { Obj::Door(b) } else { Obj::Smash(b) };
            if !self.failed.contains(&o) {
                if is_door && keys == 0 {
                    let ks = self.cands(g, |e| match e.kind {
                        Kind::Pickup { kind: PickupKind::Key(_), .. } => Some(Obj::Key(e.id)),
                        _ => None,
                    });
                    if let Some((o, path, cost)) = self.plan_any(g, i, &ks) {
                        let line = format!("{} blocks the way to the exit: fetching a key", Obj::Door(b).name());
                        self.log(g, line);
                        self.set_obj(g, i, o, path, cost);
                        return Ok(());
                    }
                } else if let Some((gl, at)) = self.goals_of(g, o) {
                    if let Some((o, path, cost)) = self.plan_any(g, i, &[(o, gl, at)]) {
                        let line = format!("{} blocks the way to the exit", o.name());
                        self.log(g, line);
                        self.set_obj(g, i, o, path, cost);
                        return Ok(());
                    }
                }
            }
        }
        // Enablers. When the reachable area has grown since the last round,
        // everything may be worth another try.
        let reach = self
            .nav
            .start_node(g, g.players[i].pos)
            .map(|s| self.nav.reachable(g, s, 400_000))
            .unwrap_or_default();
        if self.verbose {
            // Where each lever and pad stands relative to what can be reached.
            for e in g.entities.iter().filter(|e| e.alive) {
                if let Kind::Trigger { gate: None, pressed, mode, .. } = e.kind {
                    let near = reach
                        .iter()
                        .map(|&n| self.nav.nodes[n as usize].pos)
                        .min_by(|a, b| (*a - e.pos).length().total_cmp(&(*b - e.pos).length()));
                    println!(
                        "  trigger#{} at {:?} {mode:?} pressed {pressed}: nearest reachable node {:?}",
                        e.id, e.pos, near
                    );
                }
            }
        }
        // What each reachable lever or pad would open up.
        let exit_goals: Vec<Goal> = exits.iter().flat_map(|c| c.1.iter().copied()).collect();
        let mut looks: Vec<(Obj, bool, i64)> = Vec::new();
        let trig_ids: Vec<u64> = g
            .entities
            .iter()
            .filter(|e| e.alive)
            .filter_map(|e| match e.kind {
                Kind::Trigger { gate: None, id, mover: Some(m), mode, pressed, .. }
                    if id != crate::tower::LECTERN
                        && !(mode == crate::triggers::TriggerMode::Latch
                            && (pressed || g.movers.get(m).is_some_and(|m| m.active))) =>
                {
                    Some(e.id)
                }
                _ => None,
            })
            .collect();
        for tid in trig_ids {
            let o = Obj::Trigger(tid);
            if self.failed.contains(&o) {
                continue;
            }
            let Some((gl, _)) = self.goals_of(g, o) else { continue };
            let at_hand = reach
                .iter()
                .any(|&n| gl.iter().any(|x| x.holds(self.nav.nodes[n as usize].pos)));
            if !at_hand {
                continue;
            }
            let la = self.lookahead(g, tid, &exit_goals, reach.len());
            if self.verbose {
                println!("  lookahead {}: {la:?}", o.name());
            }
            if let Some((exit_ok, gain)) = la {
                looks.push((o, exit_ok, gain));
            }
        }
        let reach = reach.len();
        // A lever that opens the way to the exit comes first.
        let best_exit: Vec<_> = looks
            .iter()
            .filter(|l| l.1)
            .filter_map(|l| self.goals_of(g, l.0).map(|(gl, at)| (l.0, gl, at)))
            .collect();
        if let Some((o, path, cost)) = self.plan_any(g, i, &best_exit) {
            let line = format!("{} should open the way to the exit", o.name());
            self.log(g, line);
            self.set_obj(g, i, o, path, cost);
            return Ok(());
        }
        if reach > self.reach_mark + self.reach_mark / 20 + 5 {
            if self.reach_mark > 0 {
                let line = format!("reachable area grew {} -> {reach} nodes", self.reach_mark);
                self.log(g, line);
            }
            self.reach_mark = reach;
            self.tried.retain(|o, _| !matches!(o, Obj::Trigger(_) | Obj::Smash(_)));
            self.failed
                .retain(|o| !matches!(o, Obj::Trigger(_) | Obj::Smash(_) | Obj::Door(_) | Obj::Exit(_)));
        }
        let keys: u32 = g.players.iter().map(|p| p.keys).sum();
        let tried_map = self.tried.clone();
        let tried = |o: &Obj| tried_map.get(o).copied().unwrap_or(0);
        // 1. A locked door, with a key.
        if keys > 0 {
            let doors: Vec<_> = self
                .cands(g, |e| matches!(e.kind, Kind::Door { .. }).then_some(Obj::Door(e.id)))
                .into_iter()
                .filter(|c| tried(&c.0) < 2)
                .collect();
            if let Some((o, path, cost)) = self.plan_any(g, i, &doors) {
                self.set_obj(g, i, o, path, cost);
                return Ok(());
            }
        }
        // 2. Keys.
        if keys < crate::game::MAX_KEYS {
            let ks = self.cands(g, |e| match e.kind {
                Kind::Pickup { kind: PickupKind::Key(_), .. } => Some(Obj::Key(e.id)),
                _ => None,
            });
            if let Some((o, path, cost)) = self.plan_any(g, i, &ks) {
                self.set_obj(g, i, o, path, cost);
                return Ok(());
            }
        }
        // 3. Levers and pads that open up more of the level (most first).
        let min_gain = (reach as i64 / 30).max(30);
        let mut gains: Vec<_> = looks.iter().filter(|l| l.2 >= min_gain && tried(&l.0) < 2).collect();
        gains.sort_by_key(|l| -l.2);
        if let Some(best) = gains.first() {
            let top = best.2;
            let cands: Vec<_> = gains
                .iter()
                .filter(|l| l.2 * 2 >= top)
                .filter_map(|l| self.goals_of(g, l.0).map(|(gl, at)| (l.0, gl, at)))
                .collect();
            if let Some((o, path, cost)) = self.plan_any(g, i, &cands) {
                let line = format!("{} should open up the level (+{} nodes)", o.name(), top);
                self.log(g, line);
                self.set_obj(g, i, o, path, cost);
                return Ok(());
            }
        }
        // Rotators a touch sets turning.
        let rots: Vec<_> = g
            .world_anim
            .untouched_rotators()
            .into_iter()
            .map(|r| Obj::Rotator(r.2 as u64))
            .filter(|o| !self.failed.contains(o))
            .filter_map(|o| self.goals_of(g, o).map(|(gl, at)| (o, gl, at)))
            .collect();
        if let Some((o, path, cost)) = self.plan_any(g, i, &rots) {
            self.set_obj(g, i, o, path, cost);
            return Ok(());
        }
        // Levers and pads not tried since the area last grew.
        let trigs: Vec<_> = self
            .cands(g, |e| match e.kind {
                Kind::Trigger { gate: None, id, mode, pressed, mover, .. }
                    if id != crate::tower::LECTERN
                        && !(mode == crate::triggers::TriggerMode::Latch && pressed)
                        // A release only switches its target off: not blindly.
                        && mode != crate::triggers::TriggerMode::Release
                        && !(mode == crate::triggers::TriggerMode::Latch
                            && mover.and_then(|m| g.movers.get(m)).is_some_and(|m| m.active)) =>
                {
                    Some(Obj::Trigger(e.id))
                }
                _ => None,
            })
            .into_iter()
            .filter(|c| tried(&c.0) == 0)
            // Not one the lookahead says shuts part of the level off.
            .filter(|c| !looks.iter().any(|l| l.0 == c.0 && l.2 < -20))
            .collect();
        if let Some((o, path, cost)) = self.plan_any(g, i, &trigs) {
            self.set_obj(g, i, o, path, cost);
            return Ok(());
        }
        // 4. Breakables: generators, barrels, secret walls.
        let smash: Vec<_> = self
            .cands(g, |e| match &e.kind {
                Kind::Generator { .. } => Some(Obj::Smash(e.id)),
                Kind::Wall(w) if w.health > 0 => Some(Obj::Smash(e.id)),
                _ => None,
            })
            .into_iter()
            .filter(|c| tried(&c.0) == 0)
            .collect();
        if let Some((o, path, cost)) = self.plan_any(g, i, &smash) {
            self.set_obj(g, i, o, path, cost);
            return Ok(());
        }
        // 5. Whatever can still be picked up (it may be what opens the way).
        let picks: Vec<_> = self
            .cands(g, |e| match e.kind {
                Kind::Pickup { kind, .. } => match kind {
                    PickupKind::Food(_) => None,
                    PickupKind::Potion(_) if g.players[i].potions >= crate::game::MAX_POTIONS => None,
                    PickupKind::Key(_) => None,
                    _ => Some(Obj::Pickup(e.id)),
                },
                _ => None,
            })
            .into_iter()
            .filter(|c| tried(&c.0) == 0)
            .collect();
        if let Some((o, path, cost)) = self.plan_any(g, i, &picks) {
            self.set_obj(g, i, o, path, cost);
            return Ok(());
        }
        // 6. Barrels and other breakables.
        let smash: Vec<_> = self
            .cands(g, |e| match &e.kind {
                Kind::Container { chest: false, armor, .. } if *armor >= 0.0 && e.solid.is_some() => {
                    Some(Obj::Smash(e.id))
                }
                _ => None,
            })
            .into_iter()
            .filter(|c| tried(&c.0) == 0)
            .collect();
        if let Some((o, path, cost)) = self.plan_any(g, i, &smash) {
            self.set_obj(g, i, o, path, cost);
            return Ok(());
        }
        // Moving platforms by the edge of the reachable area: wait by one
        // for it to come round (up to 20 s each, a minute in all).
        if self.platform_wait < 60.0 {
            let start = self.nav.start_node(g, g.players[i].pos);
            let reach_set = start.map(|s| self.nav.reachable(g, s, 400_000)).unwrap_or_default();
            let mut best: Option<(f32, u64, Vec3)> = None;
            for (o, c, ext) in self.nav.volatile(g) {
                let key = Obj::Wait(o.max(0) as u64);
                if o < 0 || self.waited.get(&(o as u64)).copied().unwrap_or(0.0) > 20.0 || self.failed.contains(&key) {
                    continue;
                }
                for &n in &reach_set {
                    let p = self.nav.nodes[n as usize].pos;
                    let d = (xz(p - c).length() - ext).max(0.0);
                    if d < 5.0 && (p.y - c.y).abs() < 12.0 && best.is_none_or(|b| d < b.0) {
                        best = Some((d, o as u64, p));
                    }
                }
            }
            if let Some((_, o, p)) = best {
                // Each look counts 2 s against the budgets.
                self.platform_wait += 2.0;
                *self.waited.entry(o).or_default() += 2.0;
                let cands = vec![(Obj::Wait(o), vec![Goal::new(p, 1.2, 2.0)], p)];
                if let Some((ob, path, cost)) = self.plan_any(g, i, &cands) {
                    self.set_obj(g, i, ob, path, cost);
                    self.agents[i].goals = cands[0].1.clone();
                    self.agents[i].target = Some(p);
                    return Ok(());
                }
            }
        }
        // Movers may still be on their way: wait for them before giving up.
        // Everything tried: like a player going round again, give every
        // enabler another chance (twice, while there is time).
        if self.rounds < 2 && g.time < self.timeout - 300.0 {
            self.rounds += 1;
            self.tried.clear();
            self.failed.retain(|o| matches!(o, Obj::Exit(_)));
            self.nav.blocked.clear();
            self.ran.clear();
            let line = format!("enablers exhausted: going round again (round {})", self.rounds + 1);
            self.log(g, line);
            self.set_obj(g, i, Obj::Idle, Vec::new(), 0.0);
            return Ok(());
        }
        // Edges blocked after getting stuck may be what hides the way:
        // forget them (a few times) and look again.
        if !self.nav.blocked.is_empty() && self.unblocks < 3 {
            self.unblocks += 1;
            self.nav.blocked.clear();
            self.log(g, "no way found: forgetting blocked edges and looking again".into());
            self.set_obj(g, i, Obj::Idle, Vec::new(), 0.0);
            return Ok(());
        }
        // (Not for ever: some targets never stop.)
        if g.movers.iter().any(|m| m.moving) && self.mover_wait < 20.0 {
            self.mover_wait += 2.0;
            self.set_obj(g, i, Obj::Idle, Vec::new(), 0.0);
            return Ok(());
        }
        let exits_failed: Vec<String> = self
            .failed
            .iter()
            .filter(|o| matches!(o, Obj::Exit(_)))
            .map(|o| o.name())
            .collect();
        let cracks = self.frontier_cracks(g);
        let crack_note = self.what_if(&cracks);
        Err(if exits_failed.is_empty() {
            Outcome::Unreachable(format!(
                "no path to any exit; enablers exhausted (reachable {reach} nodes, keys {keys}, tried {}){crack_note}",
                self.tried.len()
            ))
        } else {
            Outcome::Stuck(format!(
                "exit path found but kept getting stuck on the way ({})",
                exits_failed.join(", ")
            ))
        })
    }

    /// A follower: the leader's exit once the leader stands on it, the
    /// leader's trigger while the leader holds it, else the leader.
    fn decide_follower(&mut self, g: &Game, i: usize, lead: usize) {
        let lo = self.agents[lead].obj;
        let on = g.players[lead].exit.touch.is_some() || self.agents[lead].arrived.is_some();
        if let (Some(o @ (Obj::Exit(_) | Obj::Portal(_) | Obj::Trigger(_))), true) = (lo, on) {
            if let Some((gl, at)) = self.goals_of(g, o) {
                if let Some((o, path, cost)) = self.plan_any(g, i, &[(o, gl, at)]) {
                    self.set_obj(g, i, o, path, cost);
                    return;
                }
            }
        }
        let lp = g.players[lead].pos;
        let a = &mut self.agents[i];
        if xz(g.players[i].pos - lp).length() < 5.0 && (g.players[i].pos.y - lp.y).abs() < 3.0 {
            a.obj = Some(Obj::Follow);
            a.path.clear();
            a.target = None;
            a.goals.clear();
            a.last_adv = g.time;
            return;
        }
        let goals = vec![Goal::new(lp, 4.0, 3.0)];
        let start = self.nav.start_node(g, g.players[i].pos);
        let path = start
            .and_then(|s| self.nav.path(g, s, &goals, g.time, MAX_EXPAND / 4))
            .map(|p| p.0)
            .unwrap_or_default();
        let a = &mut self.agents[i];
        a.obj = Some(Obj::Follow);
        a.goals = goals;
        a.target = Some(lp);
        a.path = path;
        a.wp = 0;
        a.last_adv = g.time;
    }

    // ---- the optimistic graph ---------------------------------------------

    /// Doors, breakable walls, breakable containers and generators: what
    /// the optimistic graph walks through. With the collision triangles of
    /// doors and walls.
    fn removable(g: &Game) -> (HashSet<u64>, Vec<usize>) {
        let mut ids = super::nav::HashSet::default();
        let mut tris = Vec::new();
        for e in g.entities.iter().filter(|e| e.alive) {
            match &e.kind {
                Kind::Door { coll, .. } => {
                    ids.insert(e.id);
                    if let Some((a, n)) = coll {
                        tris.extend(*a..*a + *n);
                    }
                }
                Kind::Wall(w) if w.health > 0 => {
                    ids.insert(e.id);
                    if let Some((a, n)) = w.tris {
                        tris.extend(a..a + n);
                    }
                }
                Kind::Generator { .. } => {
                    ids.insert(e.id);
                }
                Kind::Container { chest: false, armor, .. } if *armor >= 0.0 => {
                    ids.insert(e.id);
                }
                _ => {}
            }
        }
        tris.retain(|&t| t < g.collision.tris.len());
        (ids, tris)
    }

    /// The first removable thing on an optimistic path from hero i to one
    /// of `cands`.
    fn first_blocker(
        &mut self,
        g: &mut Game,
        i: usize,
        cands: &[(Obj, Vec<Goal>, Vec3)],
    ) -> Option<u64> {
        if cands.is_empty() {
            return None;
        }
        let (ids, tris) = Self::removable(g);
        let saved: Vec<bool> = tris.iter().map(|&t| g.collision.enabled[t]).collect();
        for &t in &tris {
            g.collision.enabled[t] = false;
        }
        let gr: &Game = g;
        let opt = self.opt.get_or_insert_with(|| {
            let mut n = Nav::new(gr);
            n.ignore = ids.clone();
            n.reset_edges();
            n
        });
        if opt.ignore != ids {
            let grew = ids.iter().any(|id| !opt.ignore.contains(id));
            opt.ignore = ids.clone();
            if grew {
                opt.reset_edges();
            }
        }
        opt.refresh(gr);
        let goals: Vec<Goal> = cands.iter().flat_map(|c| c.1.iter().copied()).collect();
        let pos = gr.players[i].pos;
        let path = opt
            .start_node(gr, pos)
            .and_then(|s| opt.path(gr, s, &goals, gr.time, MAX_EXPAND));
        let pts: Vec<Vec3> = path
            .map(|(p, _)| {
                std::iter::once(pos)
                    .chain(p.iter().map(|&n| opt.nodes[n as usize].pos))
                    .collect()
            })
            .unwrap_or_default();
        for (k, &t) in tris.iter().enumerate() {
            g.collision.enabled[t] = saved[k];
        }
        if self.verbose {
            println!("optimistic path to the exit: {} points", pts.len());
        }
        if pts.is_empty() {
            return None;
        }
        let ents: Vec<(u64, Vec3, f32)> = g
            .entities
            .iter()
            .filter(|e| ids.contains(&e.id))
            .map(|e| {
                let at = match &e.kind {
                    Kind::Wall(w) => e.pos + w.coll,
                    _ => e.pos,
                };
                (e.id, at, solid_radius(e))
            })
            .collect();
        for w in pts.windows(2) {
            let (a, b) = (w[0], w[1]);
            let mut best: Option<(f32, u64)> = None;
            for &(id, at, r) in &ents {
                if (at.y - a.y).abs() > 4.0 {
                    continue;
                }
                let d = seg_dist(xz(a), xz(b), xz(at));
                if d < r + PLAYER_RADIUS + 0.3 {
                    let along = xz(at - a).length();
                    if best.is_none_or(|x| along < x.0) {
                        best = Some((along, id));
                    }
                }
            }
            if let Some((_, id)) = best {
                return Some(id);
            }
        }
        None
    }

    /// Pressing trigger `tid`: would the exit become reachable from where
    /// the hero then stands (riding the target if it stands on it), and how
    /// many more nodes could be reached than `base`.
    fn lookahead(&mut self, g: &mut Game, tid: u64, exits: &[Goal], base: usize) -> Option<(bool, i64)> {
        let t0 = std::time::Instant::now();
        let r = self.lookahead_inner(g, tid, exits, base);
        self.timing[2] += t0.elapsed().as_secs_f64();
        r
    }

    fn lookahead_inner(&mut self, g: &mut Game, tid: u64, exits: &[Goal], base: usize) -> Option<(bool, i64)> {
        use crate::worldanim::wflags;
        let e = Self::ent(g, tid)?;
        let (mi, tpos, radius) = match e.kind {
            Kind::Trigger { mover: Some(m), radius, .. } => (m, e.pos, radius),
            _ => return None,
        };
        let reach_r = radius + PLAYER_RADIUS + 0.45;
        let stand = self.nav.node_near(tpos, reach_r, 3.5)?;
        let stand_pos = self.nav.nodes[stand as usize].pos;
        let under = g.collision.ground_tri(stand_pos, stand_pos.y + 0.5).map(|(_, t)| t);
        // The whole collision as it is, to put back.
        let saved: Vec<([Vec3; 3], Vec3)> = g.collision.tris.iter().map(|t| (t.v, t.n)).collect();
        let saved_on = g.collision.enabled.clone();
        let m = g.movers.get(mi)?;
        // What a press does: latches and sets switch the target on, a
        // release switches it off, a toggle flips it.
        let mode = match Self::ent(g, tid).map(|e| &e.kind) {
            Some(Kind::Trigger { mode, .. }) => *mode,
            _ => return None,
        };
        use crate::triggers::TriggerMode as TM;
        let want = match mode {
            TM::Latch | TM::Set => true,
            TM::Release => false,
            TM::Toggle => !m.active,
        };
        if want == m.active && !m.moving {
            return Some((false, 0));
        }
        if m.animated && !m.fades() {
            // Its world animation at the other end.
            let mut wa = g.world_anim.clone();
            let to_end = want;
            let mut any = false;
            for &o in &m.objects {
                if wa.has_anim(o) {
                    any = true;
                    wa.restore_anim(o, 1, if to_end { 1e9 } else { 0.0 });
                    if let Some(f) = wa.flags.get_mut(o) {
                        *f = (*f & !(wflags::REVERSE | wflags::FORWARD))
                            | if to_end { wflags::FORWARD } else { wflags::REVERSE };
                    }
                }
            }
            if !any {
                return None;
            }
            wa.update(&mut g.collision, 0.0);
        } else {
            let (tris, _, solid) = m.end_state(want)?;
            for (i, v) in tris {
                match solid {
                    Some(on) => g.collision.enabled[i] = on,
                    None => g.collision.tris[i].v = v,
                }
            }
            g.collision.refit_dynamic();
        }
        // What changed, and how far the floor under the trigger moved.
        let mut lo = Vec2::splat(f32::MAX);
        let mut hi = Vec2::splat(f32::MIN);
        let mut changed = 0;
        for (i, t) in g.collision.tris.iter().enumerate() {
            if t.v != saved[i].0 || g.collision.enabled[i] != saved_on[i] {
                changed += 1;
                for p in t.v.iter().chain(saved[i].0.iter()) {
                    lo = lo.min(xz(*p));
                    hi = hi.max(xz(*p));
                }
            }
        }
        let dy = under
            .filter(|&t| g.collision.tris[t].v != saved[t].0)
            .map(|t| {
                let y = |v: &[Vec3; 3]| (v[0].y + v[1].y + v[2].y) / 3.0;
                y(&g.collision.tris[t].v) - y(&saved[t].0)
            });
        let result = if changed == 0 {
            None
        } else {
            let gr: &Game = g;
            self.nav.invalidate(gr, lo, hi);
            let seed_pos = stand_pos + Vec3::Y * dy.unwrap_or(0.0);
            let seed = self.nav.node_near(seed_pos, 2.0, 2.0);
            let r = seed.map(|s| self.nav.reachable(gr, s, 400_000)).unwrap_or_default();
            let exit_ok = r
                .iter()
                .any(|&n| exits.iter().any(|gl| gl.holds(self.nav.nodes[n as usize].pos)));
            // Where to run for if the target only stays switched while held:
            // the newly reachable spot nearest an exit, else farthest away.
            let pts: Vec<Vec3> = r.iter().map(|&n| self.nav.nodes[n as usize].pos).collect();
            let pick = if exits.is_empty() {
                pts.iter().copied().max_by(|a, b| xz(*a - stand_pos).length().total_cmp(&xz(*b - stand_pos).length()))
            } else {
                pts.iter().copied().min_by(|a, b| {
                    let d = |p: Vec3| exits.iter().map(|gl| xz(p - gl.pos).length()).fold(f32::MAX, f32::min);
                    d(*a).total_cmp(&d(*b))
                })
            };
            if let Some(p) = pick {
                self.run_to.insert(tid, p);
            }
            Some((exit_ok, r.len() as i64 - base as i64))
        };
        for (i, t) in g.collision.tris.iter_mut().enumerate() {
            t.v = saved[i].0;
            t.n = saved[i].1;
        }
        g.collision.enabled = saved_on;
        g.collision.refit_dynamic();
        if changed > 0 {
            let gr: &Game = g;
            self.nav.invalidate(gr, lo, hi);
        }
        result
    }

    // ---- objective state ----------------------------------------------------

    /// Whether hero i's objective is over: Some(true) done, Some(false) given up.
    fn obj_over(&mut self, g: &mut Game, i: usize) -> Option<bool> {
        let a = &self.agents[i];
        let o = a.obj?;
        let now = g.time;
        let gone = |id: u64| Self::ent(g, id).is_none();
        // Nothing takes forever.
        let cap = match o {
            Obj::Exit(_) | Obj::Portal(_) => 300.0,
            Obj::Boss | Obj::Idle | Obj::Follow => f32::MAX,
            Obj::Key(_) | Obj::Pickup(_) | Obj::Food(_) => 60.0,
            Obj::Wait(_) => 30.0,
            Obj::Run(_) => 20.0,
            _ => 120.0,
        };
        if now - a.obj_t > cap && !o.entity().is_some_and(gone) {
            return Some(false);
        }
        match o {
            Obj::Key(id) | Obj::Pickup(id) | Obj::Food(id) => {
                if gone(id) {
                    return Some(true);
                }
                (now - a.obj_t > 40.0).then_some(false)
            }
            Obj::Door(id) => {
                if gone(id) {
                    return Some(true);
                }
                let keys: u32 = g.players.iter().map(|p| p.keys).sum();
                let pushed = a.arrived.is_some_and(|t| now - t > 4.0);
                if pushed && keys > 0 {
                    // Pushed against it with a key and it stayed shut.
                    let (dpos, r) = Self::ent(g, id)
                        .map(|e| match e.kind {
                            Kind::Door { radius, .. } => (e.pos, radius),
                            _ => (e.pos, 0.0),
                        })
                        .unwrap_or_default();
                    let hp = g.players[i].pos;
                    let msg = format!(
                        "door#{id} at ({:.1},{:.1},{:.1}) did not open: hero {i} with {} keys pushed against it for 4 s from ({:.2},{:.2},{:.2}), {:.3} from its centre (opens within {:.3})",
                        dpos.x, dpos.y, dpos.z, keys, hp.x, hp.y, hp.z, (hp - dpos).length(), r
                    );
                    self.anomalies.push(Anomaly { what: msg, tags: vec!["can't reach"], capture: true });
                }
                (keys == 0 || pushed).then_some(false)
            }
            Obj::Trigger(_) => {
                let t = a.arrived?;
                let moving = g.movers.iter().any(|m| m.moving);
                ((now - t > 1.5 && !moving) || now - t > 15.0).then_some(true)
            }
            Obj::Smash(id) => {
                if gone(id) {
                    return Some(true);
                }
                (a.arrived.is_some_and(|t| now - t > 25.0)).then_some(false)
            }
            Obj::Boss => {
                let dead = g.boss.as_ref().is_some_and(|b| b.dead || b.dying);
                dead.then_some(true)
            }
            Obj::Lectern => {
                let got = g
                    .sumner
                    .as_mut()
                    .and_then(|s| s.hint_request.take())
                    .is_some();
                if got {
                    if let Some(mut s) = g.sumner.take() {
                        s.hints_closed(g);
                        g.sumner = Some(s);
                    }
                    self.lectern_ok = Some(true);
                    self.log(g, "lectern: Sumner's hint menu opened".into());
                    return Some(true);
                }
                if a.arrived.is_some_and(|t| now - t > 12.0) {
                    self.lectern_ok = Some(false);
                    self.log(g, "lectern: stood there 12 s, no hint menu".into());
                    return Some(false);
                }
                None
            }
            Obj::Follow => (now - a.obj_t > 1.0 || a.path.is_empty()).then_some(true),
            Obj::Idle => (now - a.obj_t > 2.0).then_some(true),
            Obj::Exit(id) | Obj::Portal(id) => gone(id).then_some(false),
            Obj::Rotator(t) => {
                let left = g.world_anim.untouched_rotators().iter().any(|x| x.2 as u64 == t);
                (!left).then_some(true)
            }
            // Standing by: look again every second.
            Obj::Wait(_) => a.arrived.is_some_and(|t| now - t > 1.0).then_some(true),
            Obj::Run(_) => {
                let inside = a.goals.iter().any(|gl| gl.holds(g.players[i].pos));
                if inside {
                    Some(true)
                } else {
                    (now - a.obj_t > 15.0).then_some(false)
                }
            }
        }
    }

    // ---- driving ------------------------------------------------------------

    /// One tick: decide, steer, fight. The inputs for every hero.
    pub fn inputs(&mut self, g: &mut Game) -> Result<Vec<PlayerInput>, Outcome> {
        let t0 = std::time::Instant::now();
        let r = self.inputs_inner(g);
        self.timing[0] += t0.elapsed().as_secs_f64();
        r
    }

    fn inputs_inner(&mut self, g: &mut Game) -> Result<Vec<PlayerInput>, Outcome> {
        let n = g.players.len();
        let mut out = vec![PlayerInput::default(); n];
        let tr = std::time::Instant::now();
        let changed = g.frame % 30 == 0 && self.nav.refresh(g);
        self.timing[3] += tr.elapsed().as_secs_f64();
        if changed {
            for a in &mut self.agents {
                a.replan = true;
            }
        }
        let Some(lead) = Self::leader(g) else {
            return Ok(out);
        };
        for i in 0..n {
            if !g.players[i].state.active() {
                continue;
            }
            if let Some(done) = self.obj_over(g, i) {
                let o = self.agents[i].obj.unwrap();
                if i == lead && !matches!(o, Obj::Follow | Obj::Idle) {
                    let line = format!(
                        "hero {i} {} {} after {:.1}s",
                        o.name(),
                        if done { "done" } else { "given up" },
                        g.time - self.agents[i].obj_t
                    );
                    self.log(g, line);
                    if done {
                        self.used.push(o.name());
                        // Getting things done is not being pinned.
                        self.anchor.1 = g.time;
                    }
                }
                *self.tried.entry(o).or_default() += 1;
                if !done {
                    self.failed.insert(o);
                }
                self.agents[i].obj = None;
            }
            let throttled = self.agents[i].obj.is_some() && g.time - self.agents[i].plan_t < 2.0;
            if self.agents[i].obj.is_none() || (self.agents[i].replan && !throttled) {
                let keep = self.agents[i].obj;
                if i == lead {
                    // A replan keeps the objective when it still has a path.
                    let mut kept = false;
                    if let Some(o) = keep.filter(|o| !matches!(o, Obj::Idle | Obj::Follow)) {
                        if let Some((gl, at)) = self.goals_of(g, o) {
                            if let Some((o2, path, cost)) = self.plan_any(g, i, &[(o, gl, at)]) {
                                self.set_obj(g, i, o2, path, cost);
                                kept = true;
                            }
                        }
                    }
                    if !kept {
                        self.agents[i].obj = None;
                        match self.decide_leader(g, i) {
                            Ok(()) => {
                                if self.agents[i].obj != Some(Obj::Idle) {
                                    self.giveup_t = None;
                                }
                            }
                            // Give the world a few seconds (a fall, a mover
                            // still on its way) before calling it.
                            Err(e) => {
                                let t0 = *self.giveup_t.get_or_insert(g.time);
                                if g.time - t0 > 8.0 {
                                    return Err(e);
                                }
                                self.set_obj(g, i, Obj::Idle, Vec::new(), 0.0);
                            }
                        }
                    }
                } else {
                    self.decide_follower(g, i, lead);
                }
            }
            out[i] = self.drive(g, i);
        }
        self.watch(g)?;
        Ok(out)
    }

    fn drive(&mut self, g: &mut Game, i: usize) -> PlayerInput {
        let now = g.time;
        let p = &g.players[i];
        let pos = p.pos;
        let maxh = crate::game::max_health(p.level);
        // Low health: god mode (recorded), so the level can still be checked.
        if p.health < maxh * 0.35 && !g.debug_god {
            let why = format!(
                "hero {i} health {:.0}/{maxh:.0} at ({:.1},{:.1},{:.1}) during {}",
                p.health,
                pos.x,
                pos.y,
                pos.z,
                self.agents[i].obj.map_or("-".into(), |o| o.name())
            );
            self.log(g, format!("GOD MODE switched on: {why}"));
            self.god_at = Some((now, why));
            g.debug_god = true;
        }
        let p = &g.players[i];
        let a = &self.agents[i];
        let obj = a.obj.unwrap_or(Obj::Idle);
        // Standing on the exit: stand still (DoExit counts still ticks).
        if matches!(obj, Obj::Exit(_) | Obj::Portal(_)) && p.exit.touch.is_some() {
            return PlayerInput::default();
        }
        let mut fight = self.combat(g, i);
        // A transporter in progress: hands off.
        if g.transport.heroes.get(i).is_some_and(|h| h.timer > 0.0) {
            self.agents[i].last_adv = now;
            return PlayerInput::default();
        }
        // Boss: attack it in reach.
        if obj == Obj::Boss {
            if let Some((at, r)) = boss_point(g, pos) {
                let d = xz(at - pos).length() - r;
                // In reach: swing. Out of walking reach (no path, or the walk
                // has stalled) but in sight: throw at it.
                let a = &self.agents[i];
                let stalled = a.path.is_empty() || now - a.last_adv > 1.5;
                let seen = g
                    .collision
                    .raycast(pos + Vec3::Y * 2.5, at)
                    .is_none_or(|t| t > 0.95);
                if d < 4.0 || (stalled && d < 30.0 && seen) {
                    self.boss_fight_t += 1.0 / 60.0;
                    self.agents[i].last_adv = now;
                    return self.attack_toward(g, i, at, true);
                }
            }
        }
        let a = &mut self.agents[i];
        // Arrival: where the objective acts.
        let target = a.target;
        let in_goal = a.goals.iter().any(|gl| gl.holds(pos));
        match obj {
            Obj::Trigger(id) if in_goal || a.arrived.is_some() => {
                let shoot = matches!(
                    Self::ent(g, id).map(|e| &e.kind),
                    Some(Kind::Trigger { shoot: true, .. })
                );
                if a.arrived.is_none() {
                    a.arrived = Some(now);
                }
                a.last_adv = now;
                if shoot {
                    if let Some(t) = target {
                        return self.attack_toward(g, i, t, false);
                    }
                }
                // A toggle flips again while stood on once its target stops
                // (and its 2 s cooldown runs out): step off it, staying on
                // the same floor (a lift's pad rides the lift).
                let toggle = matches!(
                    Self::ent(g, id).map(|e| &e.kind),
                    Some(Kind::Trigger { mode: crate::triggers::TriggerMode::Toggle, .. })
                );
                if let (true, Some(t)) = (toggle, target) {
                    let reach = match Self::ent(g, id).map(|e| &e.kind) {
                        Some(Kind::Trigger { radius, .. }) => radius + PLAYER_RADIUS + 0.6,
                        _ => 2.8,
                    };
                    if now - a.arrived.unwrap() > 0.8 && xz(pos - t).length() < reach + 0.5 {
                        let under = g.collision.ground_hit(pos, pos.y + 0.5).map(|h| h.1);
                        let away = (0..16)
                            .map(|k| {
                                let ang = k as f32 * std::f32::consts::TAU / 16.0;
                                Vec2::new(ang.sin(), ang.cos())
                            })
                            .filter(|d| {
                                let spot = t + Vec3::new(d.x, 0.0, d.y) * (reach + 1.0);
                                g.collision.ground_hit(spot, pos.y + 1.0).is_some_and(|h| {
                                    Some(h.1) == under && (h.0 - pos.y).abs() < 1.0
                                })
                            })
                            .min_by(|a, b| {
                                let pa = xz(t) + *a * (reach + 1.0);
                                let pb = xz(t) + *b * (reach + 1.0);
                                (pa - xz(pos)).length().total_cmp(&(pb - xz(pos)).length())
                            });
                        if let Some(d) = away {
                            let spot = xz(t) + d * (reach + 1.0);
                            return PlayerInput {
                                stick: g.stick_toward((spot - xz(pos)).normalize_or_zero()) * 0.7,
                                ..Default::default()
                            };
                        }
                    }
                }
                // A pad whose effect lasts only while held (a release, or a
                // set pad): once its target moves, run for the far side.
                let held_pad = matches!(
                    Self::ent(g, id).map(|e| &e.kind),
                    Some(Kind::Trigger { mode: crate::triggers::TriggerMode::Release | crate::triggers::TriggerMode::Set, .. })
                );
                if held_pad && now - a.arrived.unwrap() > 0.6 && self.run_to.contains_key(&id) && !self.ran.contains(&id) {
                    self.ran.insert(id);
                    let line = format!("hero {i}: pad trigger#{id} held, running for the far side");
                    self.log(g, line);
                    let o = Obj::Run(id);
                    if let Some((gl, at)) = self.goals_of(g, o) {
                        let path = self.plan_any(g, i, &[(o, gl, at)]).map(|p| p.1).unwrap_or_default();
                        self.set_obj(g, i, o, path, 0.0);
                    }
                    return PlayerInput::default();
                }
                let a = &mut self.agents[i];
                // Step onto it fully, then hold still (fights still happen).
                if let Some(t) = target {
                    let d = xz(t - pos);
                    let close = d.length() < 0.6 || !in_goal;
                    if !close && now - a.arrived.unwrap() < 0.6 {
                        return PlayerInput {
                            stick: g.stick_toward(d.normalize_or_zero()) * 0.5,
                            ..Default::default()
                        };
                    }
                }
                return fight.take().unwrap_or_default();
            }
            Obj::Door(_) if in_goal => {
                a.arrived.get_or_insert(now);
                a.last_adv = now;
                if let Some(t) = target {
                    let d = xz(t - pos).normalize_or_zero();
                    return PlayerInput {
                        stick: g.stick_toward(d),
                        ..Default::default()
                    };
                }
            }
            Obj::Smash(id) if in_goal => {
                a.arrived.get_or_insert(now);
                a.last_adv = now;
                if let Some(t) = target {
                    // A breakable wall takes the swings of a hero touching
                    // it: lean into it while swinging.
                    let wall = matches!(Self::ent(g, id).map(|e| &e.kind), Some(Kind::Wall(_)));
                    let mut inp = self.attack_toward(g, i, t, true);
                    if wall {
                        let d = xz(t - pos).normalize_or_zero();
                        inp.stick = g.stick_toward(d) * 0.5;
                    }
                    return inp;
                }
            }
            Obj::Lectern if in_goal => {
                a.arrived.get_or_insert(now);
                a.last_adv = now;
                if let Some(t) = target {
                    let d = xz(t - pos);
                    if d.length() > 0.8 {
                        return PlayerInput {
                            stick: g.stick_toward(d.normalize_or_zero()) * 0.5,
                            ..Default::default()
                        };
                    }
                }
                return PlayerInput::default();
            }
            Obj::Idle => {
                a.last_adv = now;
                return fight.take().unwrap_or_default();
            }
            Obj::Wait(_) if in_goal || a.arrived.is_some() => {
                a.arrived.get_or_insert(now);
                a.last_adv = now;
                return fight.take().unwrap_or_default();
            }
            Obj::Follow if a.path.is_empty() => {
                a.last_adv = now;
                return fight.take().unwrap_or_default();
            }
            _ => {}
        }
        // Fight what is in reach (within limits).
        if let Some(f) = fight {
            // Monsters keep coming: go for the generators making them.
            let a = &self.agents[i];
            if a.fight_t > 5.0
                && now - a.hunt_t > 10.0
                && !matches!(obj, Obj::Smash(_) | Obj::Boss)
                && (i == 0 || !matches!(obj, Obj::Follow))
            {
                self.agents[i].hunt_t = now;
                let gens = self.cands(g, |e| match e.kind {
                    Kind::Generator { .. }
                        if xz(e.pos - pos).length() < 18.0 && (e.pos.y - pos.y).abs() < 4.0 =>
                    {
                        Some(Obj::Smash(e.id))
                    }
                    _ => None,
                });
                if let Some((o, path, cost)) = self.plan_any(g, i, &gens) {
                    if cost < 40.0 {
                        let line = format!("hero {i}: monsters keep coming; detour to {}", o.name());
                        self.log(g, line);
                        self.set_obj(g, i, o, path, cost);
                        return f;
                    }
                }
            }
            let a = &mut self.agents[i];
            a.fight_t += 1.0 / 60.0;
            a.last_adv = now;
            if a.fight_t > 45.0 && now > a.calm_until {
                // Fought long enough here: push on for a while.
                a.calm_until = now + 6.0;
                a.fight_t = 0.0;
            }
            if now >= a.calm_until {
                return f;
            }
        }
        self.follow_path(g, i)
    }

    fn follow_path(&mut self, g: &Game, i: usize) -> PlayerInput {
        let now = g.time;
        let pos = g.players[i].pos;
        let a = &mut self.agents[i];
        // Waypoints reached.
        while a.wp < a.path.len() {
            let n = self.nav.nodes[a.path[a.wp] as usize];
            let last = a.wp + 1 == a.path.len();
            let d = xz(n.pos - pos).length();
            let reach = if last { 0.6 } else { 1.0 };
            // Passed it: nearer the next waypoint than it is.
            let passed = !last && d < 2.5 && {
                let nx = self.nav.nodes[a.path[a.wp + 1] as usize].pos;
                xz(nx - pos).length() + 0.3 < xz(nx - n.pos).length()
            };
            if (d < reach || passed) && (n.pos.y - pos.y).abs() < 2.0 {
                a.wp += 1;
                a.last_adv = now;
                a.best_d = f32::MAX;
                if a.wp >= a.stuck_wp + 4 {
                    a.stucks = 0;
                }
            } else {
                break;
            }
        }
        // Skip ahead when a later waypoint is straight ahead and walkable.
        if a.wp + 1 < a.path.len() && g.frame % 6 == 0 {
            let w = super::nav::WalkWorld {
                col: &g.collision,
                ents: &g.entities,
                ignore: &self.nav.ignore,
                min_y: g.level.worlds.min_bounds[1],
                cracks: Default::default(),
            };
            let far = (a.wp + 4).min(a.path.len() - 1);
            for k in (a.wp + 1..=far).rev() {
                let n = self.nav.nodes[a.path[k] as usize];
                // Not across a transporter trip.
                if (a.wp.max(1)..=k).any(|j| self.nav.pad_edge(a.path[j - 1], a.path[j]).is_some()) {
                    continue;
                }
                // Not a short cut past a secret icon.
                let near_secret = g.entities.iter().any(|e| matches!(e.kind, Kind::Secret { radius, used: false, .. } if seg_dist(xz(pos), xz(n.pos), xz(e.pos)) < radius + PLAYER_RADIUS + 0.5 && (e.pos.y - pos.y).abs() < 4.0));
                if !n.hurts && !near_secret && w.walks(pos, n.pos) {
                    a.wp = k;
                    a.best_d = f32::MAX;
                    a.last_adv = now;
                    break;
                }
            }
        }
        let goal = if a.wp < a.path.len() {
            // A transporter edge: touch the pad and stand there.
            let mut pad_goal = None;
            if a.wp > 0 {
                if let Some(pad) = self.nav.pad_edge(a.path[a.wp - 1], a.path[a.wp]) {
                    let dest = self.nav.nodes[a.path[a.wp] as usize].pos;
                    if xz(dest - pos).length() < super::nav::CELL * 1.5 + 0.5
                        && (dest.y - pos.y).abs() < 2.5
                    {
                        // Sent across.
                        a.wp += 1;
                        a.last_adv = now;
                        a.on_pad_t = None;
                        return self.follow_path(g, i);
                    }
                    let pp = g.transport.pads[pad].pos;
                    let reach = g.transport.pads[pad].radius + PLAYER_RADIUS - 0.4;
                    if xz(pp - pos).length() < reach {
                        let t0 = *a.on_pad_t.get_or_insert(now);
                        a.last_adv = now;
                        if now - t0 > 6.0 {
                            // Not taken: treat as blocked.
                            self.nav
                                .blocked
                                .insert((a.path[a.wp - 1], a.path[a.wp]), now + 120.0);
                            a.replan = true;
                            a.on_pad_t = None;
                        }
                        return PlayerInput::default();
                    }
                    pad_goal = Some(pp);
                }
            }
            if let Some(pp) = pad_goal {
                pp
            } else {
            a.on_pad_t = None;
            self.nav.nodes[a.path[a.wp] as usize].pos
            }
        } else {
            match a.target {
                Some(t) => t,
                None => return PlayerInput::default(),
            }
        };
        let d = xz(goal - pos);
        let dist = d.length();
        if dist + 0.3 < a.best_d {
            a.best_d = dist;
            a.last_adv = now;
        }
        // Stuck: no waypoint reached for a while.
        if now - a.last_adv > STUCK_SECS {
            a.stucks += 1;
            a.stuck_wp = a.wp;
            let (from, to) = if a.wp > 0 && a.wp < a.path.len() {
                (Some(a.path[a.wp - 1]), Some(a.path[a.wp]))
            } else {
                (None, None)
            };
            if let (Some(f), Some(t)) = (from, to) {
                self.nav.blocked.insert((f, t), now + 90.0);
            }
            if self.verbose {
                let fp = from.map(|f| self.nav.nodes[f as usize].pos);
                println!(
                    "  stuck #{} t={now:.1} at {pos:?} goal {goal:?} (from node {fp:?}) wp {}/{} act {}",
                    a.stucks,
                    a.wp,
                    a.path.len(),
                    g.players[i].act.cur
                );
                let pts: Vec<Vec3> = a.path.iter().map(|&n| self.nav.nodes[n as usize].pos).collect();
                for (k, w) in pts.windows(2).enumerate() {
                    if (w[1] - w[0]).length() > 4.0 {
                        println!("    path gap at {k}: {:?} -> {:?}", w[0], w[1]);
                    }
                }
            }
            a.last_adv = now;
            a.best_d = f32::MAX;
            a.replan = true;
            if a.stucks >= STUCKS_PER_OBJ {
                if let Some(o) = a.obj {
                    if !matches!(o, Obj::Follow | Obj::Idle | Obj::Boss) {
                        self.failed.insert(o);
                        let msg = format!(
                            "hero {i} stuck {} times going for {} at ({:.1},{:.1},{:.1}); given up",
                            a.stucks, o.name(), pos.x, pos.y, pos.z
                        );
                        a.obj = None;
                        log_to(&mut self.history, self.verbose, g, msg.clone());
                        {
                            self.anomalies.push(Anomaly {
                                what: msg,
                                tags: vec!["stuck"],
                                capture: true,
                            });
                        }
                    }
                }
            }
        }
        let mag = if a.wp >= a.path.len() && dist < 1.5 { 0.5 } else { 1.0 };
        // Keep to the planned line (the walk the nav checked): steer at a
        // point a little ahead on the segment from the last waypoint.
        let mut steer = d;
        if a.wp > 0 && a.wp < a.path.len() {
            let s0 = xz(self.nav.nodes[a.path[a.wp - 1] as usize].pos);
            let s1 = xz(goal);
            let seg = s1 - s0;
            let len = seg.length();
            if len > 0.5 {
                let t = ((xz(pos) - s0).dot(seg) / (len * len)).clamp(0.0, 1.0);
                let carrot = s0 + seg * (t + 1.2 / len).min(1.0);
                steer = carrot - xz(pos);
            }
        }
        PlayerInput {
            stick: {
                // Not getting anywhere for a second: wiggle the heading, as a
                // player would (a step that lands in a floor seam is refused;
                // another heading may clear it).
                let mut s = steer.normalize_or_zero();
                if now - a.last_adv > 1.0 {
                    let ang = (g.frame as f32 * 0.07).sin() * 0.7;
                    let (sn, cs) = ang.sin_cos();
                    s = Vec2::new(s.x * cs - s.y * sn, s.x * sn + s.y * cs);
                }
                g.stick_toward(s) * mag
            },
            ..Default::default()
        }
    }

    /// Face `at` and swing (or throw); turbo now and then.
    fn attack_toward(&mut self, g: &Game, i: usize, at: Vec3, turbo_ok: bool) -> PlayerInput {
        let p = &g.players[i];
        let d = xz(at - p.pos).normalize_or_zero();
        let f = g.frame;
        let turbo = turbo_ok && p.power >= 60.0 && f % 120 < 12;
        PlayerInput {
            stick: g.stick_toward(d) * 0.06,
            attack: true,
            attack_pressed: f % 4 == 0 || (turbo && f % 120 == 11),
            turbo,
            ..Default::default()
        }
    }

    /// Fight what is in reach: monsters, generators, critters (not a sleeping
    /// boss). A potion when hard pressed.
    fn combat(&mut self, g: &Game, i: usize) -> Option<PlayerInput> {
        let p = &g.players[i];
        let pos = p.pos;
        let mut near = 0;
        let mut best: Option<(f32, Vec3)> = None;
        for e in &g.entities {
            if !e.alive {
                continue;
            }
            let (at, r) = match &e.kind {
                Kind::Enemy { state, .. } if !matches!(state, EnemyState::Dying { .. }) => {
                    (e.pos, 1.0)
                }
                Kind::Generator { .. } => (e.pos, solid_radius(e)),
                Kind::Critter(c) if c.hittable() && !c.is_boss() => {
                    match crate::game::Game::aim_point(e, pos + Vec3::Y * 2.5) {
                        Some(x) => x,
                        None => continue,
                    }
                }
                _ => continue,
            };
            if (at.y - pos.y).abs() > 3.5 && !matches!(e.kind, Kind::Critter(_)) {
                continue;
            }
            let d = xz(at - pos).length() - r;
            if d < 6.0 {
                near += 1;
            }
            if d < 4.0 && best.is_none_or(|b| d < b.0) {
                best = Some((d, at));
            }
        }
        let (_, at) = best?;
        let maxh = crate::game::max_health(p.level);
        let mut inp = self.attack_toward(g, i, at, near >= 3);
        let a = &mut self.agents[i];
        if p.potions > 0 && p.health < maxh * 0.4 && near >= 3 && g.time - a.potion_t > 5.0 {
            a.potion_t = g.time;
            inp.magic = true;
            inp.magic_pressed = true;
        }
        Some(inp)
    }

    /// Per-tick sanity checks: NaN, falling out of the world, the trail.
    fn watch(&mut self, g: &Game) -> Result<(), Outcome> {
        for (i, p) in g.players.iter().enumerate() {
            if !p.pos.is_finite() {
                return Err(Outcome::Nan(format!("hero {i} position {:?}", p.pos)));
            }
            let last = self.last_pos.get(i).copied().unwrap_or(p.pos);
            let jump = (p.pos - last).length();
            let transporting = g.transport.heroes.get(i).is_some_and(|h| h.timer > 0.0 || h.counter > 0);
            // Moved by the game (a transporter, put back on the floor): replan.
            if jump > 3.0 {
                if let Some(a) = self.agents.get_mut(i) {
                    a.replan = true;
                    a.on_pad_t = None;
                }
            }
            if jump > 12.0 && !transporting && p.state.active() {
                self.falls += 1;
                let fell = last.y <= g.level.worlds.min_bounds[1] + 0.5;
                if self.falls <= 5 {
                    self.anomalies.push(Anomaly {
                        what: format!(
                            "hero {i} {} from ({:.1},{:.1},{:.1}) to ({:.1},{:.1},{:.1})",
                            if fell {
                                "fell out of the world (below the level's lowest bound) and was put back"
                            } else {
                                "was moved by the game"
                            },
                            last.x, last.y, last.z, p.pos.x, p.pos.y, p.pos.z
                        ),
                        tags: vec!["other"],
                        capture: self.falls == 1 && fell,
                    });
                }
                if fell {
                    self.fall_times.push(g.time);
                    self.fall_times.retain(|t| g.time - t < 60.0);
                    if self.fall_times.len() >= 5 {
                        return Err(Outcome::FellOut(format!(
                            "hero {i} fell out of the world 5 times in a minute; put back at ({:.1},{:.1},{:.1}) and fell again from there",
                            p.pos.x, p.pos.y, p.pos.z
                        )));
                    }
                }
            }
            if i < self.last_pos.len() {
                self.last_pos[i] = p.pos;
            }
        }
        if g.frame % 60 == 0 {
            if let Some(p) = g.players.first() {
                self.trail.push_back((g.time, p.pos));
                if self.trail.len() > 40 {
                    self.trail.pop_front();
                }
            }
        }
        Ok(())
    }

    /// The level ended (or the time ran out): the outcome, if over.
    pub fn check_end(&mut self, g: &Game) -> Option<Outcome> {
        if let Some(end) = g.level_end {
            return Some(match end {
                LevelEnd::Exit(_) => Outcome::Completed,
                LevelEnd::AllDead => Outcome::Died("every hero died".into()),
                LevelEnd::Secret(w) => Outcome::Secret(w),
                LevelEnd::SecretOver => Outcome::Completed,
            });
        }
        if g.players.iter().any(|p| matches!(p.state, PlayerState::Dead { .. })) && g.debug_god {
            return Some(Outcome::Died("a hero died (a blow bigger than the god-mode margin, or in god mode)".into()));
        }
        // Pinned: hero 0 hasn't got 3 units from one spot in 150 s (outside
        // boss fights, timed levels and the exit wait).
        if let Some(p) = g.players.first().filter(|p| p.state.active()) {
            if (p.pos - self.anchor.0).length() > 3.0 {
                self.anchor = (p.pos, g.time);
            } else if g.time - self.anchor.1 > 150.0
                && g.boss.is_none()
                && g.secret_timer.is_none()
                && p.exit.touch.is_none()
            {
                return Some(Outcome::Stuck(format!(
                    "hero 0 has not got 3 units from ({:.1},{:.1},{:.1}) in {:.0} s (last objective {})",
                    self.anchor.0.x,
                    self.anchor.0.y,
                    self.anchor.0.z,
                    g.time - self.anchor.1,
                    self.agents.first().and_then(|a| a.obj).map_or("-".into(), |o| o.name())
                )));
            }
        }
        // On the exit for long with nothing happening.
        let on_exit = g.players.iter().filter(|p| p.state.active()).all(|p| p.exit.touch.is_some());
        if on_exit && g.players.iter().any(|p| p.state.active()) {
            let t = *self.exit_since.get_or_insert(g.time);
            if g.time - t > 20.0 {
                return Some(Outcome::ExitNoEnd(format!(
                    "the whole party stood on an exit for {:.0} s; exit progress {:#x}, waiting {:?}",
                    g.time - t,
                    g.exit_state.progress,
                    g.players.iter().map(|p| p.exit.waiting).collect::<Vec<_>>()
                )));
            }
        } else {
            self.exit_since = None;
        }
        if g.time > self.timeout {
            let o = self.agents.first().and_then(|a| a.obj);
            return Some(Outcome::Timeout(format!(
                "{:.0} s of game time; last objective {}",
                g.time,
                o.map_or("-".into(), |o| o.name())
            )));
        }
        None
    }

    /// The runner's view, for reports: objective, path, recent places.
    pub fn note(&self, g: &Game) -> String {
        let mut s = String::new();
        for (i, a) in self.agents.iter().enumerate() {
            let p = &g.players[i];
            s += &format!(
                "- hero {i}: pos ({:.2}, {:.2}, {:.2}) health {:.0} keys {} state {:?}\n  objective {} (since t={:.1}s, stucks {}), target {:?}\n",
                p.pos.x, p.pos.y, p.pos.z, p.health, p.keys, p.state,
                a.obj.map_or("-".into(), |o| o.name()), a.obj_t, a.stucks,
                a.target.map(|t| (t.x, t.y, t.z))
            );
            s += &format!(
                "  level {} exp {}, killed {} monsters and {} generators, gold {}\n",
                p.level, p.exp, p.stats.enemies_killed, p.stats.generators_destroyed, p.gold
            );
            let path: Vec<String> = a
                .path
                .iter()
                .skip(a.wp.saturating_sub(1))
                .take(25)
                .map(|&n| {
                    let p = self.nav.nodes[n as usize].pos;
                    format!("({:.1},{:.1},{:.1})", p.x, p.y, p.z)
                })
                .collect();
            s += &format!("  path from waypoint {} of {}: {}\n", a.wp, a.path.len(), path.join(" "));
        }
        s += "\nLast positions of hero 0 (one a second):\n";
        for (t, p) in &self.trail {
            s += &format!("  t={t:7.1}s ({:.2}, {:.2}, {:.2})\n", p.x, p.y, p.z);
        }
        s += "\nObjectives so far:\n";
        for h in self.history.iter().rev().take(400).collect::<Vec<_>>().into_iter().rev() {
            s += &format!("  {h}\n");
        }
        s += &format!(
            "\nWall time: bot {:.1} s (planning {:.1}, lookahead {:.1}, nav refresh {:.1})\n",
            self.timing[0], self.timing[1], self.timing[2], self.timing[3]
        );
        s += &format!(
            "\nFailed objectives: {}\nBlocked edges: {}\nNav: {} live nodes, {} edge walks\n",
            self.failed.iter().map(|o| o.name()).collect::<Vec<_>>().join(", "),
            self.nav.blocked.len(),
            self.nav.live_nodes(),
            self.nav.walks
        );
        s
    }
}

/// The radius an entity takes up (its solid shape).
pub fn solid_radius(e: &crate::game::Entity) -> f32 {
    match e.solid {
        Some(crate::game::Shape::Cylinder { radius }) => radius,
        Some(crate::game::Shape::Box { half }) => half.max_element(),
        None => match &e.kind {
            Kind::Wall(w) => w.radius,
            _ => 1.0,
        },
    }
}

/// The boss's aim point and radius.
pub fn boss_point(g: &Game, from: Vec3) -> Option<(Vec3, f32)> {
    let e = g.entities.iter().find(|e| match &e.kind {
        Kind::Critter(c) => c.is_boss() && !c.deleted,
        _ => false,
    })?;
    let _ = Vec2::ZERO;
    crate::game::Game::aim_point(e, from + Vec3::Y * 2.5)
}

fn log_to(history: &mut Vec<String>, verbose: bool, g: &Game, s: String) {
    let line = format!("t={:7.2}s {s}", g.time);
    if verbose {
        println!("{line}");
    }
    crate::bugreport::event("runner", s);
    history.push(line);
}

/// Distance from p to the segment a-b.
fn seg_dist(a: Vec2, b: Vec2, p: Vec2) -> f32 {
    let ab = b - a;
    let t = if ab.length_squared() > 1e-6 {
        ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (a + ab * t - p).length()
}

impl Bot {
    /// A top-down map of the nav graph: floors reachable from hero 0 in
    /// green (brighter = higher), the rest grey, hero 0 red, its path white,
    /// exits yellow, levers and pads blue (cyan once pressed), doors brown,
    /// keys magenta, generators orange, transporter pads purple.
    pub fn nav_map(&mut self, g: &Game) -> gdl_formats::Image {
        const PX: i32 = 6;
        let (nx, nz) = (self.nav.nx, self.nav.nz);
        let (w, h) = ((nx * PX) as u32, (nz * PX) as u32);
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        let reach = self
            .nav
            .start_node(g, g.players[0].pos)
            .map(|s| self.nav.reachable(g, s, 400_000))
            .unwrap_or_default();
        let (lo, hi) = (g.level.worlds.min_bounds[1], g.level.worlds.max_bounds[1]);
        let span = (hi - lo).max(1.0);
        let put = |rgba: &mut Vec<u8>, c: (i32, i32), col: [u8; 3], size: i32| {
            for dz in 0..size {
                for dx in 0..size {
                    let x = c.0 * PX + dx - (size - PX) / 2;
                    // North (+z) up.
                    let z = (nz - 1 - c.1) * PX + dz - (size - PX) / 2;
                    if x < 0 || z < 0 || x >= w as i32 || z >= h as i32 {
                        continue;
                    }
                    let k = ((z as u32 * w + x as u32) * 4) as usize;
                    rgba[k..k + 4].copy_from_slice(&[col[0], col[1], col[2], 255]);
                }
            }
        };
        // Highest node per cell drawn last.
        let mut order: Vec<u32> = (0..self.nav.nodes.len() as u32)
            .filter(|&n| self.nav.nodes[n as usize].alive)
            .collect();
        order.sort_by(|a, b| {
            self.nav.nodes[*a as usize].pos.y.total_cmp(&self.nav.nodes[*b as usize].pos.y)
        });
        for n in order {
            let node = self.nav.nodes[n as usize];
            let t = ((node.pos.y - lo) / span).clamp(0.0, 1.0);
            let v = (60.0 + 160.0 * t) as u8;
            let col = if reach.contains(&n) {
                [0, v, 0]
            } else if node.hurts {
                [v, 0, 0]
            } else {
                [v / 2, v / 2, v / 2]
            };
            put(&mut rgba, node.cell, col, PX);
        }
        if let Some(a) = self.agents.first() {
            for &n in &a.path {
                let c = self.nav.nodes[n as usize].cell;
                put(&mut rgba, c, [255, 255, 255], 3);
            }
        }
        for e in g.entities.iter().filter(|e| e.alive) {
            let col = match &e.kind {
                Kind::Exit { open: true, .. } => [255, 255, 0],
                Kind::Trigger { pressed: false, .. } => [40, 80, 255],
                Kind::Trigger { pressed: true, .. } => [0, 255, 255],
                Kind::Door { .. } => [160, 90, 30],
                Kind::Pickup { kind: PickupKind::Key(_), .. } => [255, 0, 255],
                Kind::Generator { .. } => [255, 140, 0],
                _ => continue,
            };
            put(&mut rgba, self.nav.cell_of(e.pos), col, 10);
        }
        for c in self.frontier_cracks(g) {
            put(&mut rgba, self.nav.cell_of(c), [255, 120, 200], 8);
        }
        for p in &g.transport.pads {
            put(&mut rgba, self.nav.cell_of(p.pos), [150, 0, 200], 10);
        }
        for p in &g.players {
            put(&mut rgba, self.nav.cell_of(p.pos), [255, 0, 0], 14);
        }
        gdl_formats::Image { width: w, height: h, rgba }
    }
}

/// Points along a breakable wall's collision (its triangles' corners, edge
/// middles and centres, at their lowest height), for standing against it.
pub fn wall_points(g: &Game, e: &crate::game::Entity) -> Option<Vec<Vec3>> {
    let Kind::Wall(w) = &e.kind else { return None };
    let (a, n) = w.tris?;
    let tris = g.collision.tris.get(a..a + n)?;
    let floor = tris
        .iter()
        .flat_map(|t| t.v.iter().map(|v| v.y))
        .fold(f32::MAX, f32::min);
    let mut pts = Vec::new();
    for t in tris {
        let c = (t.v[0] + t.v[1] + t.v[2]) / 3.0;
        for p in [t.v[0], t.v[1], t.v[2], (t.v[0] + t.v[1]) * 0.5, (t.v[1] + t.v[2]) * 0.5, (t.v[2] + t.v[0]) * 0.5, c] {
            pts.push(Vec3::new(p.x, floor, p.z));
        }
    }
    (!pts.is_empty()).then_some(pts)
}

impl Bot {
    /// Floor seams (see nav::WalkWorld::cracks) next to the area hero 0 can
    /// reach: places where a step found no floor though floor goes on.
    pub fn frontier_cracks(&mut self, g: &Game) -> Vec<Vec3> {
        let Some(s) = self.nav.start_node(g, g.players[0].pos) else {
            return Vec::new();
        };
        let reach = self.nav.reachable(g, s, 400_000);
        let mut out: Vec<Vec3> = Vec::new();
        for &c in &self.nav.cracks {
            let near = reach.iter().any(|&n| {
                let p = self.nav.nodes[n as usize].pos;
                xz(p - c).length() < 2.5 && (p.y - c.y).abs() < 2.0
            });
            if near && !out.iter().any(|o| (*o - c).length() < 2.0) {
                out.push(c);
            }
        }
        out
    }
}

impl Bot {
    /// Notes on an unreachable exit: floor seams that stopped the nav's
    /// walks at the edge of the reachable area.
    fn what_if(&mut self, cracks: &[Vec3]) -> String {
        let mut out = Vec::new();
        let eg = cracks
            .iter()
            .take(3)
            .map(|c| format!("({:.1},{:.1},{:.1})", c.x, c.y, c.z))
            .collect::<Vec<_>>()
            .join(" ");
        if !cracks.is_empty() {
            out.push(format!("{} possible floor seams at the edge of the reachable area, e.g. {eg}", cracks.len()));
        }
        if out.is_empty() {
            String::new()
        } else {
            format!("; {}", out.join("; "))
        }
    }
}
