//! Game state and simulation (independent of rendering).

use crate::character::{Animator, CharacterModel};
use crate::collision::{Collision, Tri};
use crate::fs::GameFs;
use crate::input::PlayerInput;
use crate::level::Level;
use anyhow::Result;
use glam::{Vec2, Vec3};

pub const PLAYER_CLASSES: [&str; 4] = ["WAR", "VAL", "WIZ", "ARC"];
pub const PLAYER_COLORS: [&str; 4] = ["RED", "BLU", "YEL", "GRE"];

const GRAVITY: f32 = 60.0;
const RUN_SPEED: f32 = 13.0;
const TURBO_SPEED: f32 = 18.0;
const STEP_HEIGHT: f32 = 1.2;
const PLAYER_RADIUS: f32 = 1.0;
const PLAYER_HEIGHT: f32 = 4.5;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PlayerState {
    Normal,
    Attacking { seq: usize },
}

pub struct Player {
    pub model: usize,
    pub pos: Vec3,
    pub yaw: f32,
    pub vy: f32,
    pub on_ground: bool,
    pub animator: Animator,
    pub state: PlayerState,
    pub health: f32,
    walk_phase: u8,
}

pub struct Seqs {
    ready: usize,
    walk: [usize; 2],
    run: [usize; 2],
    attack: Vec<usize>,
    falling: Option<usize>,
}

impl Seqs {
    fn find(m: &CharacterModel) -> Seqs {
        let s = |n: &str| m.sequence(n);
        let ready = s("READY").unwrap_or(0);
        Seqs {
            ready,
            walk: [s("WALK1").unwrap_or(ready), s("WALK2").unwrap_or(ready)],
            run: [s("RUN1").unwrap_or(ready), s("RUN2").unwrap_or(ready)],
            attack: ["ATTQUICK1", "ATTQUICK2", "ATTQUICK3"]
                .iter()
                .filter_map(|n| s(n))
                .collect(),
            falling: s("FALLING"),
        }
    }
}

pub struct Game {
    pub level: Level,
    pub collision: Collision,
    pub models: Vec<CharacterModel>,
    pub seqs: Vec<Seqs>,
    pub players: Vec<Player>,
    /// Smoothed camera focus point.
    pub cam_focus: Vec3,
    pub time: f32,
}

impl Game {
    pub fn new(fs: &GameFs, level_name: &str, classes: &[(&str, &str)]) -> Result<Game> {
        let level = Level::load(fs, level_name)?;
        let tris = level
            .worlds
            .coll_tris
            .iter()
            .map(|t| Tri {
                v: t.verts.map(Vec3::from),
                n: Vec3::from(t.norm).normalize_or_zero(),
                owner: -1,
            })
            .collect();
        let collision = Collision::new(tris);
        let mut models = Vec::new();
        let mut seqs = Vec::new();
        for (class, color) in classes {
            let m = CharacterModel::load_player(fs, class, color)?;
            seqs.push(Seqs::find(&m));
            models.push(m);
        }
        let starts = level.start_positions();
        let players = (0..classes.len())
            .map(|i| {
                let base = starts[i % starts.len()];
                let pos = base + Vec3::new((i as f32 - 1.5) * 2.5, 0.5, 0.0);
                Player {
                    model: i,
                    pos,
                    yaw: 0.0,
                    vy: 0.0,
                    on_ground: false,
                    animator: Animator::new(seqs[i].ready),
                    state: PlayerState::Normal,
                    health: 1000.0,
                    walk_phase: 0,
                }
            })
            .collect::<Vec<_>>();
        let cam_focus = players.first().map(|p| p.pos).unwrap_or(Vec3::ZERO);
        Ok(Game {
            level,
            collision,
            models,
            seqs,
            players,
            cam_focus,
            time: 0.0,
        })
    }

    /// Camera-relative movement basis on the ground plane (forward, right), in game coordinates.
    pub fn camera_basis(&self) -> (Vec2, Vec2) {
        // The camera looks along +z; with the X mirror applied at render time, screen-right is +x.
        (Vec2::new(0.0, 1.0), Vec2::new(1.0, 0.0))
    }

    pub fn update(&mut self, dt: f32, inputs: &[PlayerInput]) {
        self.time += dt;
        let (fwd, right) = self.camera_basis();
        for (i, p) in self.players.iter_mut().enumerate() {
            let inp = inputs.get(i).copied().unwrap_or_default();
            let seqs = &self.seqs[p.model];
            let model = &self.models[p.model];

            // Attacks interrupt movement until the swing finishes.
            if let PlayerState::Attacking { .. } = p.state {
                if p.animator.finished {
                    p.state = PlayerState::Normal;
                }
            }
            if inp.attack_pressed && p.state == PlayerState::Normal && !seqs.attack.is_empty() {
                let seq = seqs.attack[(self.time * 10.0) as usize % seqs.attack.len()];
                p.state = PlayerState::Attacking { seq };
                p.animator.play(seq);
                p.animator.frame = 0.0;
                p.animator.finished = false;
            }

            let wish = fwd * inp.stick.y + right * inp.stick.x;
            let mag = wish.length().min(1.0);
            let moving = mag > 0.05 && p.state == PlayerState::Normal;
            if mag > 0.05 {
                p.yaw = wish.x.atan2(wish.y);
            }
            if moving {
                let speed = if inp.turbo { TURBO_SPEED } else { RUN_SPEED } * mag;
                let dir = Vec3::new(p.yaw.sin(), 0.0, p.yaw.cos());
                let target = p.pos + dir * speed * dt;
                // Only accept the move if there is ground within a step of our feet.
                let probe = self.collision.ground_height(target, p.pos.y + STEP_HEIGHT);
                if probe.is_some_and(|g| g > p.pos.y - 20.0) || !p.on_ground {
                    p.pos = target;
                }
            }
            p.pos = self
                .collision
                .resolve_walls(p.pos, PLAYER_RADIUS, PLAYER_HEIGHT, STEP_HEIGHT);

            // Gravity and ground snapping.
            p.vy -= GRAVITY * dt;
            p.pos.y += p.vy * dt;
            match self.collision.ground_height(p.pos, p.pos.y + STEP_HEIGHT) {
                Some(g) if p.pos.y <= g => {
                    p.pos.y = g;
                    p.vy = 0.0;
                    p.on_ground = true;
                }
                Some(g) if p.pos.y - g < 0.3 && p.vy <= 0.0 => {
                    p.pos.y = g;
                    p.vy = 0.0;
                    p.on_ground = true;
                }
                _ => p.on_ground = false,
            }
            if p.pos.y < self.level.worlds.min_bounds[1] - 50.0 {
                // Fell out of the world: respawn at the start.
                p.pos = self.level.start_positions()[0] + Vec3::Y;
                p.vy = 0.0;
            }

            // Animation selection.
            if p.state == PlayerState::Normal {
                if !p.on_ground && p.vy < -15.0 {
                    if let Some(f) = seqs.falling {
                        p.animator.play(f);
                    }
                } else if moving {
                    let set = if mag > 0.6 { seqs.run } else { seqs.walk };
                    if !set.contains(&p.animator.seq) {
                        p.walk_phase = 0;
                        p.animator.play(set[0]);
                    } else if p.animator.finished {
                        p.walk_phase ^= 1;
                        p.animator.play(set[p.walk_phase as usize]);
                    }
                    p.animator.speed = if inp.turbo { 1.3 } else { 1.0 };
                } else {
                    p.animator.play(seqs.ready);
                    p.animator.speed = 1.0;
                }
            }
            p.animator.update(&model.skeleton, dt);
        }

        // Camera follows the centroid of all players.
        if !self.players.is_empty() {
            let c = self.players.iter().map(|p| p.pos).sum::<Vec3>() / self.players.len() as f32;
            let k = 1.0 - (-dt * 6.0).exp();
            self.cam_focus = self.cam_focus.lerp(c, k);
        }
    }

    /// Camera eye and target in game coordinates.
    pub fn camera(&self) -> (Vec3, Vec3) {
        let spread = self
            .players
            .iter()
            .map(|p| (p.pos - self.cam_focus).length())
            .fold(0.0f32, f32::max);
        let dist = 34.0 + spread * 0.8;
        let pitch = 55f32.to_radians();
        let at = self.cam_focus + Vec3::Y * 2.0;
        let eye = at + Vec3::new(0.0, pitch.sin() * dist, -pitch.cos() * dist);
        (eye, at)
    }
}
