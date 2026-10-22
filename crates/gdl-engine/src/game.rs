//! Game state and simulation (independent of rendering).

use crate::character::{Animator, Model};
use crate::collision::{Collision, Tri};
use crate::fs::GameFs;
use crate::input::PlayerInput;
use crate::level::Level;
use anyhow::Result;
use gdl_formats::anim::Anim;
use gdl_formats::worlds::ItemType;
use gdl_formats::Objects;
use glam::{Vec2, Vec3};
use std::collections::HashMap;

pub const PLAYER_CLASSES: [&str; 4] = ["WAR", "VAL", "WIZ", "ARC"];
pub const PLAYER_COLORS: [&str; 4] = ["RED", "BLU", "YEL", "GRE"];

const GRAVITY: f32 = 60.0;
const RUN_SPEED: f32 = 13.0;
const TURBO_SPEED: f32 = 18.0;
const STEP_HEIGHT: f32 = 1.2;
const PLAYER_RADIUS: f32 = 1.0;
const PLAYER_HEIGHT: f32 = 4.5;
const PLAYER_MAX_HEALTH: f32 = 999.0;
const PLAYER_START_HEALTH: f32 = 600.0;
/// Health drains slowly over time, as in every Gauntlet.
const HEALTH_DRAIN_PER_SEC: f32 = 1.0;
const ATTACK_RANGE: f32 = 5.0;
const ATTACK_ARC_COS: f32 = 0.25;
const ENEMY_ACTIVATE_DIST: f32 = 60.0;
const GENERATOR_ACTIVATE_DIST: f32 = 40.0;
const MAX_ENEMIES: usize = 48;
/// Players are kept within this distance of the group's centre (stay on screen).
const PARTY_RADIUS: f32 = 13.0;

/// Small deterministic RNG (xorshift) so simulations are reproducible.
#[derive(Clone)]
pub struct Rng(u64);
impl Rng {
    pub fn next_u32(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 16) as u32
    }
    pub fn f32(&mut self) -> f32 {
        (self.next_u32() & 0xFFFFFF) as f32 / 16_777_216.0
    }
    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.f32()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PlayerState {
    Normal,
    Attacking { seq: usize, hit_done: bool },
    Dead { timer: f32 },
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
    pub keys: u32,
    pub potions: u32,
    pub score: u32,
    pub hurt_flash: f32,
    walk_phase: u8,
}

/// Animation sequence indices for a player model.
pub struct PlayerSeqs {
    ready: usize,
    walk: [usize; 2],
    run: [usize; 2],
    attack: Vec<usize>,
    falling: Option<usize>,
    death: Option<usize>,
}

impl PlayerSeqs {
    fn find(m: &Model) -> PlayerSeqs {
        let s = |n: &str| m.sequence(n);
        let ready = s("READY").unwrap_or(0);
        PlayerSeqs {
            ready,
            walk: [s("WALK1").unwrap_or(ready), s("WALK2").unwrap_or(ready)],
            run: [s("RUN1").unwrap_or(ready), s("RUN2").unwrap_or(ready)],
            attack: ["ATTQUICK1", "ATTQUICK2", "ATTQUICK3"]
                .iter()
                .filter_map(|n| s(n))
                .collect(),
            falling: s("FALLING"),
            death: s("DEATH"),
        }
    }
}

/// Animation sequences for a monster model.
#[derive(Clone)]
pub struct MonsterSeqs {
    ready: Option<usize>,
    walk: Option<usize>,
    attack: Vec<usize>,
    hit: Option<usize>,
    start: Option<usize>,
}

impl MonsterSeqs {
    fn find(m: &Model) -> MonsterSeqs {
        let s = |n: &str| m.sequence(n);
        MonsterSeqs {
            ready: s("READY"),
            walk: s("WALK").or(s("RUN")).or(s("WALK1")),
            attack: ["ATTACK1", "ATTACK2", "ATTACK3", "RUNATTACK1"]
                .iter()
                .filter_map(|n| s(n))
                .collect(),
            hit: s("HIT1").or(s("HIT2")),
            start: s("START"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EnemyState {
    Spawning,
    Idle,
    Chase,
    Attack { hit_done: bool },
    Hurt { timer: f32 },
    Dying { timer: f32 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PickupKind {
    Food(f32),
    Key,
    Gold(u32),
    Potion,
    Other(u32),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Enemy {
        class: usize,
        strength: usize,
        hp: f32,
        state: EnemyState,
        cooldown: f32,
        speed: f32,
    },
    Generator {
        class: usize,
        strength: usize,
        hp: f32,
        timer: f32,
        interval: f32,
        max: usize,
        spawned: Vec<u64>,
    },
    Pickup {
        kind: PickupKind,
        radius: f32,
    },
    Container {
        hp: f32,
        drop: Option<usize>,
    },
    Door {
        radius: f32,
        coll: Option<(usize, usize)>,
    },
    Exit {
        radius: f32,
        next: String,
    },
    Hazard {
        radius: f32,
        damage: f32,
    },
    Prop,
}

/// Solid collision shape of an entity, from its item info.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    Cylinder { radius: f32 },
    /// Half extents along the entity's local x and z axes.
    Box { half: Vec2 },
}

pub struct Entity {
    pub id: u64,
    pub solid: Option<Shape>,
    pub kind: Kind,
    pub model: Option<usize>,
    pub pos: Vec3,
    pub yaw: f32,
    pub animator: Animator,
    pub spin: f32,
    pub alive: bool,
}

/// A monster type (`GRU`, `RAT`, ...) with a model per strength level.
pub struct MonsterClass {
    pub name: String,
    models: [Option<usize>; 4],
    gen_models: [Option<usize>; 4],
}

impl MonsterClass {
    fn model(&self, strength: usize) -> Option<usize> {
        let s = strength.clamp(1, 3);
        self.models[s]
            .or(self.models[1])
            .or(self.models[2])
            .or(self.models[3])
            .or(self.models[0])
    }
    fn gen_model(&self, strength: usize) -> Option<usize> {
        let s = strength.min(3);
        self.gen_models[s]
            .or(self.gen_models[1])
            .or(self.gen_models[0])
            .or(self.gen_models[2])
    }
}

pub struct Game {
    pub level: Level,
    pub collision: Collision,
    pub models: Vec<Model>,
    pub player_seqs: Vec<PlayerSeqs>,
    monster_seqs: HashMap<usize, MonsterSeqs>,
    pub classes: Vec<MonsterClass>,
    pub players: Vec<Player>,
    pub entities: Vec<Entity>,
    next_id: u64,
    pub cam_focus: Vec3,
    pub time: f32,
    pub rng: Rng,
    /// Set when a player reaches the exit.
    pub exit_to: Option<String>,
    pub messages: Vec<(String, f32)>,
}

/// Parsed objects/textures/anim files shared by several models.
struct AssetFile {
    objects: Objects,
    textures: Vec<u8>,
    anim: Option<Anim>,
}

fn load_asset(fs: &GameFs, dir: &str) -> Option<AssetFile> {
    let objects = Objects::parse(&fs.read(&format!("{dir}/objects.ngc")).ok()?).ok()?;
    let textures = fs.read(&format!("{dir}/textures.ngc")).ok()?;
    let anim = fs
        .read(&format!("{dir}/ANIM.PS2"))
        .ok()
        .and_then(|d| Anim::parse(&d).ok());
    Some(AssetFile {
        objects,
        textures,
        anim,
    })
}

/// World collision triangles. Triangles owned by items and by moving world objects
/// are stored relative to their owner, everything else is in world space.
fn build_collision(level: &Level) -> Vec<Tri> {
    let w = &level.worlds;
    let mut tris: Vec<Tri> = w
        .coll_tris
        .iter()
        .map(|t| Tri {
            v: t.verts.map(Vec3::from),
            n: Vec3::from(t.norm).normalize_or_zero(),
            owner: -1,
        })
        .collect();
    let mut place = |start: i32, count: i32, pos: Vec3, yaw: f32, radius: f32, owner: i32| {
        if start < 0 || count <= 0 {
            return;
        }
        let range = start as usize..((start + count) as usize).min(tris.len());
        // Local if the triangles sit around the origin rather than around the owner.
        let mut near_origin = 0;
        let mut near_owner = 0;
        for t in &tris[range.clone()] {
            let c = (t.v[0] + t.v[1] + t.v[2]) / 3.0;
            let r = radius.max(12.0);
            near_origin += (c.length() < r) as usize;
            near_owner += ((c - pos).length() < r) as usize;
        }
        let local = pos.length() > 1.0 && near_origin > near_owner;
        let rot = glam::Quat::from_rotation_y(yaw);
        for t in &mut tris[range] {
            if local {
                t.v = t.v.map(|v| pos + rot * v);
                t.n = rot * t.n;
            }
            t.owner = owner;
        }
    };
    let pos = &level.world_pos;
    for (i, o) in w.world_objects.iter().enumerate() {
        place(
            o.coll_tri_index,
            o.coll_tri_count as i32,
            Vec3::from(pos[i]),
            0.0,
            o.radius,
            i as i32,
        );
    }
    for (i, it) in w.item_instances.iter().enumerate() {
        place(
            it.coll_tri_index as i32,
            it.coll_tri_count as i32,
            Vec3::from(it.pos),
            it.rot[1],
            8.0,
            100_000 + i as i32,
        );
    }
    tris
}

fn item_shape(info: &gdl_formats::worlds::ItemInfo) -> Shape {
    match info.coll_type {
        3 => Shape::Box { half: Vec2::new(info.x_dim.max(0.5), info.z_dim.max(0.5)) },
        _ => Shape::Cylinder { radius: info.radius.clamp(0.4, 6.0) },
    }
}

/// Next level name: `levelA1` -> `levelA2`.
fn next_level_name(cur: &str) -> String {
    let digits: String = cur
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let n: u32 = digits.parse().unwrap_or(0);
    format!("{}{}", &cur[..cur.len() - digits.len()], n + 1)
}

impl Game {
    pub fn new(fs: &GameFs, level_name: &str, classes: &[(&str, &str)]) -> Result<Game> {
        let level = Level::load(fs, level_name)?;
        let collision = Collision::new(build_collision(&level));

        let mut game = Game {
            level,
            collision,
            models: Vec::new(),
            player_seqs: Vec::new(),
            monster_seqs: HashMap::new(),
            classes: Vec::new(),
            players: Vec::new(),
            entities: Vec::new(),
            next_id: 1,
            cam_focus: Vec3::ZERO,
            time: 0.0,
            rng: Rng(0x9E37_79B9_7F4A_7C15),
            exit_to: None,
            messages: Vec::new(),
        };

        for (class, color) in classes {
            let m = Model::load_player(fs, class, color)?;
            game.player_seqs.push(PlayerSeqs::find(&m));
            game.models.push(m);
        }
        let starts = game.level.start_positions();
        for i in 0..classes.len() {
            let base = starts[i % starts.len()];
            let pos = base + Vec3::new((i as f32 - 1.5) * 2.5, 0.5, 0.0);
            game.players.push(Player {
                model: i,
                pos,
                yaw: 0.0,
                vy: 0.0,
                on_ground: false,
                animator: Animator::new(game.player_seqs[i].ready),
                state: PlayerState::Normal,
                health: PLAYER_START_HEALTH,
                keys: 0,
                potions: 0,
                score: 0,
                hurt_flash: 0.0,
                walk_phase: 0,
            });
        }
        game.cam_focus = game.players.first().map(|p| p.pos).unwrap_or(Vec3::ZERO);
        game.populate(fs, classes.len());
        Ok(game)
    }

    fn add_model(&mut self, m: Model) -> usize {
        self.models.push(m);
        self.models.len() - 1
    }

    fn monster_class(
        &mut self,
        fs: &GameFs,
        cache: &mut HashMap<String, Option<AssetFile>>,
        name: &str,
    ) -> Option<usize> {
        if let Some(i) = self.classes.iter().position(|c| c.name == name) {
            return Some(i);
        }
        let dir = format!("MONSTERS/{name}");
        let asset = cache
            .entry(dir.clone())
            .or_insert_with(|| load_asset(fs, &dir))
            .as_ref()?;
        let anim = asset.anim.as_ref()?;
        let mut pending = Vec::new();
        for s in 0..4 {
            let tree = format!("{name}{s}");
            if anim
                .atrees
                .iter()
                .any(|t| t.name.eq_ignore_ascii_case(&tree))
            {
                if let Ok(m) = Model::from_anim(&asset.objects, &asset.textures, anim, Some(&tree))
                {
                    pending.push((s, m));
                }
            }
        }
        let mut gens = Vec::new();
        for s in 0..4 {
            let names = [format!("GEN_{name}{s}L1"), format!("GEN_{name}{s}")];
            if let Some(m) =
                Model::from_objects(&asset.objects, &asset.textures, &[&names[0], &names[1]])
            {
                gens.push((s, m));
            }
        }
        if pending.is_empty() {
            log::warn!("monster {name}: no animation trees found");
            return None;
        }
        let mut models = [None; 4];
        for (s, m) in pending {
            let ms = MonsterSeqs::find(&m);
            let idx = self.add_model(m);
            self.monster_seqs.insert(idx, ms);
            models[s] = Some(idx);
        }
        let mut gen_models = [None; 4];
        for (s, m) in gens {
            gen_models[s] = Some(self.add_model(m));
        }
        self.classes.push(MonsterClass {
            name: name.to_string(),
            models,
            gen_models,
        });
        Some(self.classes.len() - 1)
    }

    fn spawn(&mut self, kind: Kind, model: Option<usize>, pos: Vec3, yaw: f32) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let seq = match (&kind, model) {
            (Kind::Enemy { .. }, Some(m)) => self
                .monster_seqs
                .get(&m)
                .and_then(|s| s.start.or(s.ready))
                .unwrap_or(0),
            _ => 0,
        };
        let mut animator = Animator::new(seq);
        animator.force_loop = !matches!(kind, Kind::Enemy { .. });
        self.entities.push(Entity {
            id,
            solid: None,
            kind,
            model,
            pos,
            yaw,
            animator,
            spin: 0.0,
            alive: true,
        });
        id
    }

    fn set_solid(&mut self, id: u64, shape: Shape) {
        if let Some(e) = self.entities.iter_mut().find(|e| e.id == id) {
            e.solid = Some(shape);
        }
    }

    /// Push a circle of `radius` at `pos` out of every solid entity.
    fn resolve_solids(entities: &[Entity], mut pos: Vec3, radius: f32, skip: u64) -> Vec3 {
        for e in entities {
            let Some(shape) = e.solid else { continue };
            if !e.alive || e.id == skip || (e.pos.y - pos.y).abs() > 4.0 {
                continue;
            }
            let d = Vec2::new(pos.x - e.pos.x, pos.z - e.pos.z);
            match shape {
                Shape::Cylinder { radius: r } => {
                    let len = d.length();
                    let min = r + radius;
                    if len < min {
                        let n = if len > 1e-4 { d / len } else { Vec2::X };
                        pos.x = e.pos.x + n.x * min;
                        pos.z = e.pos.z + n.y * min;
                    }
                }
                Shape::Box { half } => {
                    // Work in the box's local frame.
                    let (s, c) = e.yaw.sin_cos();
                    let local = Vec2::new(d.x * c - d.y * s, d.x * s + d.y * c);
                    let clamped = local.clamp(-half, half);
                    let diff = local - clamped;
                    let len = diff.length();
                    let push_local = if len > 1e-4 {
                        if len >= radius {
                            continue;
                        }
                        diff / len * (radius - len)
                    } else {
                        // Centre inside the box: push out along the shallowest axis.
                        let px = half.x + radius - local.x.abs();
                        let pz = half.y + radius - local.y.abs();
                        if px < pz {
                            Vec2::new(px * local.x.signum(), 0.0)
                        } else {
                            Vec2::new(0.0, pz * local.y.signum())
                        }
                    };
                    let push = Vec2::new(
                        push_local.x * c + push_local.y * s,
                        -push_local.x * s + push_local.y * c,
                    );
                    pos.x += push.x;
                    pos.z += push.y;
                }
            }
        }
        pos
    }

    fn item_model(
        &mut self,
        fs: &GameFs,
        cache: &mut HashMap<String, Option<AssetFile>>,
        known: &mut HashMap<String, Option<usize>>,
        dirs: &[String],
        name: &str,
    ) -> Option<usize> {
        if name.is_empty() {
            return None;
        }
        if let Some(m) = known.get(name) {
            return *m;
        }
        let mut found = None;
        for dir in dirs {
            let Some(asset) = cache
                .entry(dir.clone())
                .or_insert_with(|| load_asset(fs, dir))
                .as_ref()
            else {
                continue;
            };
            if let Some(anim) = &asset.anim {
                if anim
                    .atrees
                    .iter()
                    .any(|t| t.name.eq_ignore_ascii_case(name))
                {
                    if let Ok(m) =
                        Model::from_anim(&asset.objects, &asset.textures, anim, Some(name))
                    {
                        if !m.mesh.indices.is_empty() {
                            found = Some(m);
                            break;
                        }
                    }
                }
            }
            if let Some(m) = Model::from_objects(&asset.objects, &asset.textures, &[name]) {
                found = Some(m);
                break;
            }
        }
        let idx = found.map(|m| self.add_model(m));
        known.insert(name.to_string(), idx);
        idx
    }

    /// Create entities for the level's item placements.
    fn populate(&mut self, fs: &GameFs, num_players: usize) {
        let mut cache: HashMap<String, Option<AssetFile>> = HashMap::new();
        let mut known: HashMap<String, Option<usize>> = HashMap::new();
        let world_letter = self
            .level
            .name
            .trim_start_matches("level")
            .chars()
            .next()
            .unwrap_or('A');
        let dirs = [
            format!("ITEMS/level{world_letter}"),
            format!("ITEMS/{}", self.level.name),
            "POWERUPS".to_string(),
        ];

        let instances = self.level.worlds.item_instances.clone();
        let infos = self.level.worlds.item_infos.clone();
        for inst in &instances {
            if inst.min_players as usize > num_players.max(1) {
                continue;
            }
            let Some(mut info) = infos.get(inst.item_index as usize).cloned() else {
                continue;
            };
            // Random items pick one of their candidates.
            if info.item_type == ItemType::Random && !info.random_indices.is_empty() {
                let pick =
                    info.random_indices[self.rng.next_u32() as usize % info.random_indices.len()];
                match infos.get(pick as usize) {
                    Some(i) => info = i.clone(),
                    None => continue,
                }
            }
            let pos = Vec3::from(inst.pos);
            // Placement rotations use the game's pitch/yaw/roll convention; items stand
            // upright, so reduce to the yaw that the rotation turns +z towards.
            let fwd = crate::character::ryp_matrix(inst.rot).transform_vector3(Vec3::Z);
            let yaw = fwd.x.atan2(fwd.z);
            let p16 = |k: usize| i16::from_le_bytes([inst.params[k], inst.params[k + 1]]);

            match info.item_type {
                ItemType::Enemy => {
                    let strength = (p16(0).clamp(0, 3) as usize).max(1);
                    if let Some(c) = self.monster_class(fs, &mut cache, &info.name) {
                        let model = self.classes[c].model(strength);
                        let hp = 10.0 * strength as f32;
                        self.spawn(
                            Kind::Enemy {
                                class: c,
                                strength,
                                hp,
                                state: EnemyState::Idle,
                                cooldown: 0.0,
                                speed: 6.0 + strength as f32,
                            },
                            model,
                            pos,
                            yaw,
                        );
                    }
                }
                ItemType::Generator => {
                    let strength = p16(0).clamp(0, 3) as usize;
                    let max = p16(4).clamp(1, 12) as usize;
                    let interval = (p16(6).max(1) as f32) * 0.5;
                    if let Some(c) = self.monster_class(fs, &mut cache, &info.name) {
                        let model = self.classes[c].gen_model(strength);
                        let hp = 20.0 * strength.max(1) as f32;
                        let timer = self.rng.range(0.0, interval);
                        let id = self.spawn(
                            Kind::Generator {
                                class: c,
                                strength: strength.max(1),
                                hp,
                                timer,
                                interval,
                                max,
                                spawned: Vec::new(),
                            },
                            model,
                            pos,
                            yaw,
                        );
                        self.set_solid(id, Shape::Cylinder { radius: info.radius.max(1.0) });
                    }
                }
                ItemType::Powerup => {
                    let kind = match info.subtype_name() {
                        "food" => {
                            PickupKind::Food(if info.hit_points >= 2 { 200.0 } else { 100.0 })
                        }
                        "key" => PickupKind::Key,
                        "gold" => PickupKind::Gold(if info.value > 0 {
                            info.value as u32 / 10
                        } else {
                            100
                        }),
                        "potion" => PickupKind::Potion,
                        "crystal" => PickupKind::Gold(250),
                        _ => PickupKind::Other(50),
                    };
                    let model = self.item_model(fs, &mut cache, &mut known, &dirs, &info.name);
                    self.spawn(
                        Kind::Pickup {
                            kind,
                            radius: info.radius.max(1.0) + 1.0,
                        },
                        model,
                        pos,
                        yaw,
                    );
                }
                ItemType::Container | ItemType::Obstacle => {
                    let model = self.item_model(fs, &mut cache, &mut known, &dirs, &info.name);
                    if model.is_none() {
                        continue;
                    }
                    let drop = (info.item_type == ItemType::Container)
                        .then(|| i32::from_le_bytes(inst.params[0..4].try_into().unwrap()))
                        .filter(|&i| i >= 0)
                        .map(|i| i as usize);
                    let id = self.spawn(
                        Kind::Container {
                            hp: info.hit_points.max(1) as f32 * 2.0,
                            drop,
                        },
                        model,
                        pos,
                        yaw,
                    );
                    self.set_solid(id, item_shape(&info));
                }
                ItemType::Door => {
                    let model = self.item_model(fs, &mut cache, &mut known, &dirs, &info.name);
                    let coll = (inst.coll_tri_count > 0 && inst.coll_tri_index >= 0)
                        .then(|| (inst.coll_tri_index as usize, inst.coll_tri_count as usize));
                    let id = self.spawn(
                        Kind::Door {
                            radius: info.radius.max(2.0) + 1.5,
                            coll,
                        },
                        model,
                        pos,
                        yaw,
                    );
                    self.set_solid(id, item_shape(&info));
                }
                ItemType::Exit => {
                    let model = self.item_model(fs, &mut cache, &mut known, &dirs, &info.name);
                    let name = String::from_utf8_lossy(&inst.params[4..8])
                        .trim_end_matches('\0')
                        .to_string();
                    let next = if name.is_empty() {
                        next_level_name(&self.level.name)
                    } else {
                        format!("level{name}")
                    };
                    self.spawn(
                        Kind::Exit {
                            radius: info.radius.max(2.0) + 0.5,
                            next,
                        },
                        model,
                        pos,
                        yaw,
                    );
                }
                ItemType::DamageTile | ItemType::Trap => {
                    let model = self.item_model(fs, &mut cache, &mut known, &dirs, &info.name);
                    self.spawn(
                        Kind::Hazard {
                            radius: info.radius.max(1.0),
                            damage: 20.0,
                        },
                        model,
                        pos,
                        yaw,
                    );
                }
                ItemType::Trigger | ItemType::Transporter | ItemType::Rotator => {
                    let model = self.item_model(fs, &mut cache, &mut known, &dirs, &info.name);
                    if model.is_some() {
                        self.spawn(Kind::Prop, model, pos, yaw);
                    }
                }
                _ => {}
            }
        }
        let count = |f: fn(&Kind) -> bool| self.entities.iter().filter(|e| f(&e.kind)).count();
        log::info!(
            "{}: {} enemies, {} generators, {} pickups, {} containers, {} doors, {} models loaded",
            self.level.name,
            count(|k| matches!(k, Kind::Enemy { .. })),
            count(|k| matches!(k, Kind::Generator { .. })),
            count(|k| matches!(k, Kind::Pickup { .. })),
            count(|k| matches!(k, Kind::Container { .. })),
            count(|k| matches!(k, Kind::Door { .. })),
            self.models.len()
        );
    }

    /// Camera-relative movement basis on the ground plane (forward, right), in game coordinates.
    pub fn camera_basis(&self) -> (Vec2, Vec2) {
        // The camera looks along +z; with the X mirror applied at render time, screen-right is +x.
        (Vec2::new(0.0, 1.0), Vec2::new(1.0, 0.0))
    }

    fn snap_to_ground(collision: &Collision, pos: &mut Vec3, vy: &mut f32, dt: f32) -> bool {
        *vy -= GRAVITY * dt;
        pos.y += *vy * dt;
        match collision.ground_height(*pos, pos.y + STEP_HEIGHT) {
            Some(g) if pos.y <= g || (pos.y - g < 0.3 && *vy <= 0.0) => {
                pos.y = g;
                *vy = 0.0;
                true
            }
            _ => false,
        }
    }

    pub fn update(&mut self, dt: f32, inputs: &[PlayerInput]) {
        self.time += dt;
        self.messages.retain_mut(|(_, t)| {
            *t -= dt;
            *t > 0.0
        });
        self.update_players(dt, inputs);
        self.update_entities(dt);

        // Camera follows the centroid of living players.
        let alive: Vec<Vec3> = self
            .players
            .iter()
            .filter(|p| !matches!(p.state, PlayerState::Dead { .. }))
            .map(|p| p.pos)
            .collect();
        let pts = if alive.is_empty() {
            self.players.iter().map(|p| p.pos).collect()
        } else {
            alive
        };
        if !pts.is_empty() {
            let c = pts.iter().copied().sum::<Vec3>() / pts.len() as f32;
            let k = 1.0 - (-dt * 6.0).exp();
            self.cam_focus = self.cam_focus.lerp(c, k);
        }
    }

    fn update_players(&mut self, dt: f32, inputs: &[PlayerInput]) {
        let (fwd, right) = self.camera_basis();
        let mut attacks: Vec<(usize, Vec3, f32, f32)> = Vec::new();
        for (i, p) in self.players.iter_mut().enumerate() {
            let inp = inputs.get(i).copied().unwrap_or_default();
            let seqs = &self.player_seqs[p.model];
            let model = &self.models[p.model];
            p.hurt_flash = (p.hurt_flash - dt * 3.0).max(0.0);

            if let PlayerState::Dead { timer } = &mut p.state {
                *timer -= dt;
                if *timer <= 0.0 {
                    // Respawn next to the camera focus with fresh health (kid-friendly rules).
                    p.state = PlayerState::Normal;
                    p.health = PLAYER_START_HEALTH;
                    p.pos = self.cam_focus + Vec3::Y * 2.0;
                    p.vy = 0.0;
                    p.animator.restart(seqs.ready);
                }
                p.animator.update(&model.skeleton, dt);
                continue;
            }

            p.health -= HEALTH_DRAIN_PER_SEC * dt;
            if p.health <= 0.0 {
                p.health = 0.0;
                p.state = PlayerState::Dead { timer: 4.0 };
                if let Some(d) = seqs.death {
                    p.animator.restart(d);
                }
                continue;
            }

            // Attacks interrupt movement until the swing finishes.
            if let PlayerState::Attacking { seq, hit_done } = &mut p.state {
                let frames = model.skeleton.sequences[*seq].frame_count.max(1) as f32;
                if !*hit_done && p.animator.frame >= frames * 0.4 {
                    *hit_done = true;
                    attacks.push((i, p.pos, p.yaw, 10.0));
                }
                if p.animator.finished {
                    p.state = PlayerState::Normal;
                }
            }
            if (inp.attack_pressed || inp.attack)
                && p.state == PlayerState::Normal
                && !seqs.attack.is_empty()
            {
                let seq = seqs.attack[(self.time * 7.0) as usize % seqs.attack.len()];
                p.state = PlayerState::Attacking {
                    seq,
                    hit_done: false,
                };
                p.animator.restart(seq);
                p.animator.speed = 1.6;
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
                // Don't walk off into the void: require ground within reach.
                let probe = self.collision.ground_height(target, p.pos.y + STEP_HEIGHT);
                if probe.is_some_and(|g| g > p.pos.y - 20.0) || !p.on_ground {
                    p.pos = target;
                }
            }
            p.pos = self
                .collision
                .resolve_walls(p.pos, PLAYER_RADIUS, PLAYER_HEIGHT, STEP_HEIGHT);
            p.pos = Self::resolve_solids(&self.entities, p.pos, PLAYER_RADIUS, 0);
            p.on_ground = Self::snap_to_ground(&self.collision, &mut p.pos, &mut p.vy, dt);
            if p.pos.y < self.level.worlds.min_bounds[1] - 50.0 {
                p.pos = self.cam_focus + Vec3::Y * 2.0;
                p.vy = 0.0;
            }

            if p.state == PlayerState::Normal {
                if !p.on_ground && p.vy < -15.0 {
                    if let Some(f) = seqs.falling {
                        p.animator.play(f);
                    }
                } else if moving {
                    let set = if mag > 0.6 { seqs.run } else { seqs.walk };
                    if !set.contains(&p.animator.seq) {
                        p.walk_phase = 0;
                        p.animator.restart(set[0]);
                    } else if p.animator.finished {
                        p.walk_phase ^= 1;
                        p.animator.restart(set[p.walk_phase as usize]);
                    }
                    p.animator.speed = if inp.turbo { 1.3 } else { 1.0 };
                } else {
                    p.animator.play(seqs.ready);
                    p.animator.speed = 1.0;
                }
            }
            p.animator.update(&model.skeleton, dt);
        }
        // Players don't overlap each other.
        for a in 0..self.players.len() {
            for b in (a + 1)..self.players.len() {
                let d = Vec2::new(
                    self.players[b].pos.x - self.players[a].pos.x,
                    self.players[b].pos.z - self.players[a].pos.z,
                );
                let len = d.length();
                let min = PLAYER_RADIUS * 2.0;
                if len < min {
                    let n = if len > 1e-4 { d / len } else { Vec2::new(1.0, 0.0) };
                    let push = n * (min - len) * 0.5;
                    self.players[a].pos -= Vec3::new(push.x, 0.0, push.y);
                    self.players[b].pos += Vec3::new(push.x, 0.0, push.y);
                }
            }
        }
        // Keep the party together: nobody may wander too far from the group's centre.
        let alive: Vec<usize> =
            (0..self.players.len()).filter(|&i| !matches!(self.players[i].state, PlayerState::Dead { .. })).collect();
        if alive.len() > 1 {
            let c = alive.iter().map(|&i| self.players[i].pos).sum::<Vec3>() / alive.len() as f32;
            for &i in &alive {
                let p = &mut self.players[i];
                let d = Vec2::new(p.pos.x - c.x, p.pos.z - c.z);
                if d.length() > PARTY_RADIUS {
                    let lim = d.normalize() * PARTY_RADIUS;
                    p.pos.x = c.x + lim.x;
                    p.pos.z = c.z + lim.y;
                }
            }
        }
        for (pi, pos, yaw, dmg) in attacks {
            self.player_attack(pi, pos, yaw, dmg);
        }
    }

    fn player_attack(&mut self, pi: usize, pos: Vec3, yaw: f32, dmg: f32) {
        let dir = Vec2::new(yaw.sin(), yaw.cos());
        let mut drops = Vec::new();
        let mut score = 0;
        for e in self.entities.iter_mut().filter(|e| e.alive) {
            let d = Vec2::new(e.pos.x - pos.x, e.pos.z - pos.z);
            let dist = d.length();
            let reach = ATTACK_RANGE
                + match e.kind {
                    Kind::Generator { .. } => 1.5,
                    Kind::Container { .. } => 1.0,
                    _ => 0.5,
                };
            if dist > reach || (e.pos.y - pos.y).abs() > 5.0 {
                continue;
            }
            if dist > 1.0 && d.normalize().dot(dir) < ATTACK_ARC_COS {
                continue;
            }
            match &mut e.kind {
                Kind::Enemy { hp, state, .. } => {
                    if matches!(state, EnemyState::Dying { .. }) {
                        continue;
                    }
                    *hp -= dmg;
                    if *hp <= 0.0 {
                        *state = EnemyState::Dying { timer: 0.5 };
                        score += 10;
                    } else {
                        *state = EnemyState::Hurt { timer: 0.3 };
                    }
                    // Knock back.
                    let push = if dist > 0.01 { d / dist } else { dir };
                    e.pos.x += push.x * 1.2;
                    e.pos.z += push.y * 1.2;
                }
                Kind::Generator { hp, .. } => {
                    *hp -= dmg;
                    if *hp <= 0.0 {
                        e.alive = false;
                        score += 100;
                    }
                }
                Kind::Container { hp, drop } => {
                    *hp -= dmg;
                    if *hp <= 0.0 {
                        e.alive = false;
                        score += 5;
                        drops.push((e.pos, *drop));
                    }
                }
                _ => {}
            }
        }
        self.players[pi].score += score;
        for (pos, drop) in drops {
            // Containers drop their item, or some gold.
            let kind = match drop.and_then(|i| self.level.worlds.item_infos.get(i)) {
                Some(info) if info.subtype_name() == "food" => PickupKind::Food(100.0),
                Some(info) if info.subtype_name() == "key" => PickupKind::Key,
                Some(info) if info.subtype_name() == "potion" => PickupKind::Potion,
                _ => PickupKind::Gold(50),
            };
            let model = self
                .entities
                .iter()
                .find(|e| matches!(e.kind, Kind::Pickup { kind: k, .. } if std::mem::discriminant(&k) == std::mem::discriminant(&kind)))
                .and_then(|e| e.model);
            self.spawn(
                Kind::Pickup { kind, radius: 2.0 },
                model,
                pos + Vec3::Y * 0.5,
                0.0,
            );
        }
    }

    fn update_entities(&mut self, dt: f32) {
        let player_pos: Vec<(Vec3, bool)> = self
            .players
            .iter()
            .map(|p| (p.pos, !matches!(p.state, PlayerState::Dead { .. })))
            .collect();
        let nearest = |pos: Vec3| -> Option<(usize, f32)> {
            player_pos
                .iter()
                .enumerate()
                .filter(|(_, (_, alive))| *alive)
                .map(|(i, (pp, _))| (i, (*pp - pos).length()))
                .min_by(|a, b| a.1.total_cmp(&b.1))
        };
        let enemy_count = self
            .entities
            .iter()
            .filter(|e| e.alive && matches!(e.kind, Kind::Enemy { .. }))
            .count();
        let mut spawns: Vec<(usize, usize, Vec3, u64)> = Vec::new();
        let mut damage: Vec<(usize, f32)> = Vec::new();
        let mut pickups: Vec<(usize, PickupKind)> = Vec::new();
        let mut opened: Vec<(usize, usize)> = Vec::new();
        let mut key_used: Vec<usize> = Vec::new();
        let mut exit: Option<String> = None;

        for ei in 0..self.entities.len() {
            if !self.entities[ei].alive {
                continue;
            }
            let pos = self.entities[ei].pos;
            let near = nearest(pos);
            let e = &mut self.entities[ei];
            match &mut e.kind {
                Kind::Generator {
                    class,
                    strength,
                    timer,
                    interval,
                    max,
                    spawned,
                    ..
                } => {
                    if let Some((_, d)) = near {
                        if d <= GENERATOR_ACTIVATE_DIST {
                            *timer -= dt;
                            if *timer <= 0.0 {
                                *timer = *interval;
                                if spawned.len() < *max && enemy_count + spawns.len() < MAX_ENEMIES
                                {
                                    spawns.push((*class, *strength, pos, e.id));
                                }
                            }
                        }
                    }
                }
                Kind::Enemy {
                    strength,
                    state,
                    cooldown,
                    speed,
                    ..
                } => {
                    *cooldown = (*cooldown - dt).max(0.0);
                    let Some(model) = e.model else { continue };
                    let Some(seqs) = self.monster_seqs.get(&model).cloned() else {
                        continue;
                    };
                    let skel = &self.models[model].skeleton;
                    match state {
                        EnemyState::Dying { timer } => {
                            *timer -= dt;
                            e.pos.y -= dt * 3.0;
                            if *timer <= 0.0 {
                                e.alive = false;
                            }
                            continue;
                        }
                        EnemyState::Hurt { timer } => {
                            *timer -= dt;
                            if let Some(h) = seqs.hit {
                                e.animator.play(h);
                            }
                            if *timer <= 0.0 {
                                *state = EnemyState::Chase;
                            }
                        }
                        EnemyState::Spawning => {
                            if e.animator.finished || seqs.start.is_none() {
                                *state = EnemyState::Chase;
                            }
                        }
                        EnemyState::Idle => {
                            if near.is_some_and(|(_, d)| d < ENEMY_ACTIVATE_DIST) {
                                *state = EnemyState::Chase;
                            } else if let Some(r) = seqs.ready {
                                e.animator.play(r);
                                e.animator.force_loop = true;
                            }
                        }
                        EnemyState::Chase => match near {
                            Some((pi, d)) if d <= ENEMY_ACTIVATE_DIST * 1.5 => {
                                let to = player_pos[pi].0 - e.pos;
                                e.yaw = to.x.atan2(to.z);
                                if d < 2.6 && *cooldown <= 0.0 && !seqs.attack.is_empty() {
                                    *state = EnemyState::Attack { hit_done: false };
                                    let a = seqs.attack[(e.id as usize) % seqs.attack.len()];
                                    e.animator.restart(a);
                                    e.animator.force_loop = false;
                                } else if d > 2.0 {
                                    let dir = Vec3::new(e.yaw.sin(), 0.0, e.yaw.cos());
                                    e.pos += dir * *speed * dt;
                                    if let Some(w) = seqs.walk {
                                        e.animator.play(w);
                                        e.animator.force_loop = true;
                                    }
                                } else if let Some(r) = seqs.ready {
                                    e.animator.play(r);
                                    e.animator.force_loop = true;
                                }
                            }
                            _ => *state = EnemyState::Idle,
                        },
                        EnemyState::Attack { hit_done } => {
                            let frames = skel
                                .sequences
                                .get(e.animator.seq)
                                .map(|s| s.frame_count.max(1))
                                .unwrap_or(1) as f32;
                            if !*hit_done && e.animator.frame >= frames * 0.5 {
                                *hit_done = true;
                                if let Some((pi, d)) = near {
                                    if d < 3.5 {
                                        damage.push((pi, 6.0 * *strength as f32));
                                    }
                                }
                            }
                            if e.animator.finished {
                                *state = EnemyState::Chase;
                                *cooldown = 0.6;
                            }
                        }
                    }
                    // Monsters collide with the world like players do.
                    e.pos = self.collision.resolve_walls(e.pos, 0.9, 3.5, STEP_HEIGHT);
                    if let Some(g) = self.collision.ground_height(e.pos, e.pos.y + STEP_HEIGHT) {
                        if e.pos.y - g < 3.0 {
                            e.pos.y = g;
                        } else {
                            let mut vy = 0.0;
                            Self::snap_to_ground(&self.collision, &mut e.pos, &mut vy, dt);
                        }
                    }
                    e.animator.update(skel, dt);
                }
                Kind::Pickup { kind, radius } => {
                    e.spin += dt * 2.0;
                    for (pi, (pp, alive)) in player_pos.iter().enumerate() {
                        if *alive && (*pp - e.pos).length() < *radius + 0.5 {
                            pickups.push((pi, *kind));
                            e.alive = false;
                            break;
                        }
                    }
                }
                Kind::Door { radius, coll } => {
                    for (pi, (pp, alive)) in player_pos.iter().enumerate() {
                        if *alive
                            && (*pp - e.pos).length() < *radius
                            && self.players[pi].keys > 0
                            && !key_used.contains(&pi)
                        {
                            key_used.push(pi);
                            if let Some(c) = coll {
                                opened.push(*c);
                            }
                            e.alive = false;
                            break;
                        }
                    }
                }
                Kind::Exit { radius, next } => {
                    if player_pos
                        .iter()
                        .any(|(pp, alive)| *alive && (*pp - e.pos).length() < *radius)
                    {
                        exit = Some(next.clone());
                    }
                }
                Kind::Hazard {
                    radius,
                    damage: dmg,
                } => {
                    for (pi, (pp, alive)) in player_pos.iter().enumerate() {
                        if *alive
                            && (*pp - e.pos).length() < *radius
                            && (self.time * 2.0).fract() < dt * 2.0
                        {
                            damage.push((pi, *dmg));
                        }
                    }
                }
                _ => {}
            }
            if !matches!(e.kind, Kind::Enemy { .. }) {
                if let Some(m) = e.model {
                    e.animator.update(&self.models[m].skeleton, dt);
                }
            }
        }

        // Monsters can't walk through doors, barrels and generators either.
        let moved: Vec<(usize, Vec3)> = (0..self.entities.len())
            .filter(|&i| self.entities[i].alive && matches!(self.entities[i].kind, Kind::Enemy { .. }))
            .map(|i| (i, Self::resolve_solids(&self.entities, self.entities[i].pos, 0.9, self.entities[i].id)))
            .collect();
        for (i, p) in moved {
            self.entities[i].pos = p;
        }

        // Keep monsters from stacking on top of each other (and on players).
        let enemy_idx: Vec<usize> = (0..self.entities.len())
            .filter(|&i| self.entities[i].alive && matches!(self.entities[i].kind, Kind::Enemy { .. }))
            .collect();
        for a in 0..enemy_idx.len() {
            for b in (a + 1)..enemy_idx.len() {
                let (ia, ib) = (enemy_idx[a], enemy_idx[b]);
                let d = Vec2::new(self.entities[ib].pos.x - self.entities[ia].pos.x, self.entities[ib].pos.z - self.entities[ia].pos.z);
                let len = d.length();
                if len < 1.6 && len > 1e-4 {
                    let push = d / len * (1.6 - len) * 0.5;
                    self.entities[ia].pos -= Vec3::new(push.x, 0.0, push.y);
                    self.entities[ib].pos += Vec3::new(push.x, 0.0, push.y);
                }
            }
            for (pp, alive) in &player_pos {
                let e = &mut self.entities[enemy_idx[a]];
                let d = Vec2::new(e.pos.x - pp.x, e.pos.z - pp.z);
                let len = d.length();
                if *alive && len < 1.8 && len > 1e-4 {
                    let push = d / len * (1.8 - len);
                    e.pos += Vec3::new(push.x, 0.0, push.y);
                }
            }
        }

        for pi in key_used {
            self.players[pi].keys -= 1;
            self.messages.push(("Door unlocked".into(), 2.0));
        }
        for (start, count) in opened {
            for t in start..(start + count).min(self.collision.enabled.len()) {
                self.collision.enabled[t] = false;
            }
        }
        for (pi, dmg) in damage {
            let p = &mut self.players[pi];
            if matches!(p.state, PlayerState::Dead { .. }) {
                continue;
            }
            p.health -= dmg;
            p.hurt_flash = 1.0;
        }
        for (pi, kind) in pickups {
            let p = &mut self.players[pi];
            match kind {
                PickupKind::Food(hp) => p.health = (p.health + hp).min(PLAYER_MAX_HEALTH),
                PickupKind::Key => p.keys += 1,
                PickupKind::Gold(v) => p.score += v,
                PickupKind::Potion => p.potions += 1,
                PickupKind::Other(v) => p.score += v,
            }
        }
        for (class, strength, pos, gen_id) in spawns {
            let model = self.classes[class].model(strength);
            let offset = Vec3::new(self.rng.range(-2.5, 2.5), 0.0, self.rng.range(-2.5, 2.5));
            let id = self.spawn(
                Kind::Enemy {
                    class,
                    strength,
                    hp: 10.0 * strength as f32,
                    state: EnemyState::Spawning,
                    cooldown: 0.5,
                    speed: 6.0 + strength as f32,
                },
                model,
                pos + offset,
                0.0,
            );
            if let Some(Kind::Generator { spawned, .. }) = self
                .entities
                .iter_mut()
                .find(|e| e.id == gen_id)
                .map(|e| &mut e.kind)
            {
                spawned.push(id);
            }
        }
        // Forget dead spawns so generators can replace them.
        let alive_ids: std::collections::HashSet<u64> = self
            .entities
            .iter()
            .filter(|e| e.alive)
            .map(|e| e.id)
            .collect();
        for e in &mut self.entities {
            if let Kind::Generator { spawned, .. } = &mut e.kind {
                spawned.retain(|id| alive_ids.contains(id));
            }
        }
        self.entities.retain(|e| e.alive);
        if exit.is_some() {
            self.exit_to = exit;
        }
    }

    /// Camera eye and target in game coordinates.
    pub fn camera(&self) -> (Vec3, Vec3) {
        let spread = self.players.iter().map(|p| (p.pos - self.cam_focus).length()).fold(0.0f32, f32::max);
        let dist = 36.0 + spread * 0.8;
        let pitch = 60f32.to_radians();
        let at = self.cam_focus + Vec3::Y * 2.0;
        let mut eye = at + Vec3::new(0.0, pitch.sin() * dist, -pitch.cos() * dist);
        // Pull the camera in front of any wall that would hide the players.
        if let Some(f) = self.collision.raycast(at + Vec3::Y * 1.5, eye) {
            let f = (f - 0.04).max(0.25);
            eye = at + (eye - at) * f;
        }
        (eye, at)
    }
}

impl Game {
    /// Simple bot: walk towards the nearest enemy, generator or pickup and attack when close.
    /// Used for the attract/demo mode and for headless testing.
    pub fn bot_input(&self, pi: usize) -> PlayerInput {
        let Some(p) = self.players.get(pi) else { return PlayerInput::default() };
        let target = self
            .entities
            .iter()
            .filter(|e| match &e.kind {
                Kind::Enemy { state, .. } => !matches!(state, EnemyState::Dying { .. }),
                Kind::Generator { .. } | Kind::Pickup { .. } => true,
                _ => false,
            })
            .filter(|e| (e.pos.y - p.pos.y).abs() < 4.0)
            .min_by(|a, b| (a.pos - p.pos).length().total_cmp(&(b.pos - p.pos).length()));
        let Some(t) = target else { return PlayerInput::default() };
        let to = t.pos - p.pos;
        let d = Vec2::new(to.x, to.z);
        let fighting = !matches!(t.kind, Kind::Pickup { .. });
        let close = d.length() < if fighting { 3.5 } else { 0.5 };
        PlayerInput {
            stick: if close && fighting { d.normalize_or_zero() * 0.06 } else { d.normalize_or_zero() },
            attack: close && fighting,
            attack_pressed: close && fighting,
            ..Default::default()
        }
    }

    /// Totals for test output.
    pub fn stats(&self) -> String {
        let enemies = self.entities.iter().filter(|e| matches!(e.kind, Kind::Enemy { .. })).count();
        let gens = self.entities.iter().filter(|e| matches!(e.kind, Kind::Generator { .. })).count();
        let pick = self.entities.iter().filter(|e| matches!(e.kind, Kind::Pickup { .. })).count();
        format!("enemies {enemies} generators {gens} pickups {pick}")
    }
}
