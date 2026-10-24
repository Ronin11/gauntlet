//! Tests of the player rules: damage_player's damagemul and block, AddExp's
//! level-up, chests and doors (fn_8005D730 cases 2 and 7), melee friendly
//! fire (PlayerMotion_FindClosestPlayer) and activate_player.

use super::*;

fn data() -> Option<GameFs> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game-data");
    GameFs::open(&root).ok()
}

/// levelA1 with nothing in it but the heroes.
fn bare(fs: &GameFs, heroes: &[(&str, &str)]) -> Game {
    let mut g = Game::new(fs, "levelA1", heroes).unwrap();
    g.entities.clear();
    g.pending_enemies.clear();
    g
}

fn level_mut(g: &mut Game) -> &mut gdl_formats::wdata::LevelDef {
    let w = g.wave.unwrap();
    let realm = g
        .worlds
        .realms
        .iter_mut()
        .find(|r| r.realm_type == w >> 8)
        .unwrap();
    &mut realm.levels[(w & 0xFF) as usize]
}

#[test]
fn the_levels_damagemul_scales_hits_over_one() {
    let Some(fs) = data() else { return };
    let mut g = bare(&fs, &[("WAR", "RED")]);
    level_mut(&mut g).damage = 2.0;
    let hp = g.players[0].health;
    g.damage_player(0, 20.0, DmgType::NO_ARMOR);
    assert_eq!(g.players[0].health, hp - 40.0);
    // A hit of 1 or less is left alone.
    g.damage_player(0, 1.0, DmgType::NO_ARMOR);
    assert_eq!(g.players[0].health, hp - 41.0);
}

#[test]
fn a_block_stops_only_undirected_hits() {
    let Some(fs) = data() else { return };
    let mut g = bare(&fs, &[("WAR", "RED")]);
    g.players[0].act.hud |= crate::paction::hud::BLOCK;
    let hp = g.players[0].health;
    g.damage_player(0, 20.0, DmgType::NO_ARMOR);
    assert_eq!(g.players[0].health, hp, "no direction: stopped");
    // The back-arc test never passes as shipped: a directed hit is halved.
    g.damage_from = Some(g.players[0].pos + Vec3::Z * 3.0);
    g.damage_player(0, 20.0, DmgType::NO_ARMOR);
    assert_eq!(g.players[0].health, hp - 10.0);
}

#[test]
fn a_level_up_adds_100_health_once_and_shows_its_effect() {
    let Some(fs) = data() else { return };
    let mut g = bare(&fs, &[("WAR", "RED"), ("VAL", "BLU")]);
    let hp = g.players[0].health;
    // Enough for several levels at once.
    g.add_exp(0, 3000.0, false);
    assert!(g.players[0].level > 2, "level {}", g.players[0].level);
    assert_eq!(g.players[0].health, hp + 100.0);
    let fx = g.fx_def(effect::FX_LEVELUP + 2).map(|d| d.model);
    assert!(fx.is_some(), "LEVELUP_RED");
    assert!(g
        .entities
        .iter()
        .any(|e| matches!(e.kind, Kind::Effect { .. }) && e.model == fx.unwrap()));
    // A combo partner shares the exp.
    g.players[0].grab_partner = Some(1);
    let before = g.players[1].exp;
    g.add_exp(0, 10.0, false);
    assert!(g.players[1].exp > before);
}

/// A chest of levelA1 with the hero beside it holding a key.
fn by_a_chest(fs: &GameFs, locked: bool) -> (Game, u64) {
    let mut g = Game::new(fs, "levelA1", &[("WAR", "RED")]).unwrap();
    g.pending_enemies.clear();
    g.entities
        .retain(|e| matches!(e.kind, Kind::Container { chest: true, .. }));
    let e = &g.entities[0];
    let (id, pos) = (e.id, e.pos);
    if !locked {
        let i = g.items.info[&id];
        std::sync::Arc::make_mut(&mut g.level).worlds.item_infos[i].active_type &= !0x10;
    }
    g.players[0].pos = pos + Vec3::X * 2.0;
    g.players[0].keys = 1;
    (g, id)
}

#[test]
fn only_a_locked_chest_takes_a_key() {
    let Some(fs) = data() else { return };
    let opened = |g: &Game, id: u64| {
        g.entities
            .iter()
            .find(|e| e.id == id)
            .is_none_or(|e| matches!(e.kind, Kind::Container { hp, .. } if hp <= 0.0))
    };
    let (mut g, id) = by_a_chest(&fs, true);
    for _ in 0..5 {
        g.update(1.0 / 60.0, &[PlayerInput::default()]);
    }
    assert!(opened(&g, id));
    assert_eq!(g.players[0].keys, 0);
    let (mut g, id) = by_a_chest(&fs, false);
    for _ in 0..5 {
        g.update(1.0 / 60.0, &[PlayerInput::default()]);
    }
    assert!(!opened(&g, id));
    assert_eq!(g.players[0].keys, 1);
}

#[test]
fn a_door_opens_only_for_a_hero_walking_toward_it() {
    let Some(fs) = data() else { return };
    let mut g = Game::new(&fs, "levelA1", &[("WAR", "RED")]).unwrap();
    g.entities.retain(|e| matches!(e.kind, Kind::Door { .. }));
    g.pending_enemies.clear();
    let walk = |g: &Game, z: f32| PlayerInput {
        stick: g.stick_toward(Vec2::new(0.0, z)),
        ..Default::default()
    };
    // Up against the gate at z=27.9 (see locked_gate_blocks_until_key).
    for _ in 0..400 {
        g.update(1.0 / 60.0, &[walk(&g, 1.0)]);
    }
    let doors = g.entities.len();
    // Turned round (a hero walks along its facing) and stepping away.
    g.players[0].keys = 1;
    g.players[0].yaw = std::f32::consts::PI;
    for _ in 0..3 {
        g.update(1.0 / 60.0, &[walk(&g, -1.0)]);
    }
    assert_eq!(g.entities.len(), doors, "walking away leaves it shut");
    assert_eq!(g.players[0].keys, 1);
    for _ in 0..60 {
        g.update(1.0 / 60.0, &[walk(&g, 1.0)]);
    }
    assert_eq!(g.players[0].keys, 0, "walking back opens it");
}

#[test]
fn the_altar_door_opens_to_a_hero_held_at_its_edge() {
    // levelE1's door (bug report 20261023-233135): its cylinder keeps a hero's
    // centre exactly its open radius away, so a strict `<` never opened it.
    let Some(fs) = data() else { return };
    let mut g = Game::new(&fs, "levelE1", &[("JAC", "BLU")]).unwrap();
    g.entities.retain(|e| matches!(e.kind, Kind::Door { .. }));
    g.pending_enemies.clear();
    let door = g.entities[0].pos;
    let shut = |g: &Game| {
        g.entities
            .iter()
            .any(|e| matches!(e.kind, Kind::Door { .. }))
    };
    g.players[0].pos = Vec3::new(-0.37, 0.02, 86.79);
    g.players[0].keys = 1;
    for _ in 0..120 {
        let to = Vec2::new(door.x - g.players[0].pos.x, door.z - g.players[0].pos.z);
        let input = PlayerInput {
            stick: g.stick_toward(to.normalize()),
            ..Default::default()
        };
        g.update(1.0 / 60.0, &[input]);
        if !shut(&g) {
            break;
        }
    }
    let d = (g.players[0].pos - door).length();
    assert!(
        !shut(&g),
        "the door stayed shut with the hero {d:.3} from it"
    );
    assert_eq!(g.players[0].keys, 0, "the key was used");
}

#[test]
fn the_citadel_portal_rides_the_tower_lift_up() {
    // Bug report 20261023-234746: with the 13th runestone banked, the tower
    // raises the lift to the Citadel (ActivateSpecialTrigger(255, 1)) and the
    // portal placed on that lift must come up with it, as every item put on a
    // moving floor does (AddItemSub), or it stays buried under the hall floor.
    let Some(fs) = data() else { return };
    let mut g = Game::new(&fs, "levelL1", &[("JAC", "BLU")]).unwrap();
    g.pending_enemies.clear();
    let portal = |g: &Game| {
        g.entities
            .iter()
            .find(|e| {
                matches!(
                    e.kind,
                    Kind::Exit {
                        dest: Some(0x803),
                        ..
                    }
                )
            })
            .map(|e| e.pos)
            .unwrap()
    };
    let down = portal(&g);
    assert!(
        down.y < -37.0,
        "the portal starts on the lowered lift: {down:?}"
    );
    g.shards = 0x1FFF;
    g.enter_tower();
    for _ in 0..200 {
        g.update(1.0 / 60.0, &[PlayerInput::default()]);
    }
    let up = portal(&g);
    assert!(
        up.y > -29.5 && up.y < -28.0,
        "the portal rode the lift up: {up:?}"
    );
    // A hero on the raised lift beside it is within its reach.
    let hero = Vec3::new(2.19, -28.89, 67.69);
    let Kind::Exit { radius, .. } = g
        .entities
        .iter()
        .find(|e| {
            matches!(
                e.kind,
                Kind::Exit {
                    dest: Some(0x803),
                    ..
                }
            )
        })
        .map(|e| &e.kind)
        .unwrap()
    else {
        unreachable!()
    };
    let d = Vec2::new(hero.x - up.x, hero.z - up.z).length();
    assert!(
        d < *radius,
        "the hall-level hero reaches the portal: {d:.2} < {radius}"
    );
}

#[test]
fn melee_hurts_heroes_only_when_the_option_says() {
    let Some(fs) = data() else { return };
    let mut g = bare(&fs, &[("WAR", "RED"), ("VAL", "BLU")]);
    let (pos, yaw) = (g.players[0].pos, 0.0);
    g.players[0].yaw = yaw;
    g.players[1].pos = pos + Vec3::new(0.0, 0.0, 3.0);
    let hp = g.players[1].health;
    g.player_attack(0, pos, yaw, 1.0, 0);
    assert_eq!(g.players[1].health, hp, "Normal: melee passes heroes");
    g.friendly_fire = 2;
    g.player_attack(0, pos, yaw, 1.0, 0);
    assert!(g.players[1].health < hp, "Shots Hurt: the swing lands");
    // Behind the swing: missed.
    let hp = g.players[1].health;
    g.players[1].pos = pos - Vec3::new(0.0, 0.0, 3.0);
    g.player_attack(0, pos, yaw, 1.0, 0);
    assert_eq!(g.players[1].health, hp);
}

#[test]
fn heroes_from_player_select_come_in_invisible() {
    use crate::powerups as pu;
    let mut c = Carried::new_character();
    c.activate(0);
    let s = c
        .powerups
        .slots
        .iter()
        .find(|s| s.time != 0.0)
        .expect("a power-up");
    assert_eq!((s.kind, s.flags), (pu::SPECIAL, pu::S_INVISIBLE));
    assert_eq!(s.time, 5.0 * crate::stats::powerup_time(0));
}

#[test]
fn placements_with_flag_2_get_no_geometry() {
    // SetItem skips SetItemGeo for an instance with flag 2: the tower's chained
    // activator pads and levelA1's two hidden switches work but aren't drawn.
    let Some(fs) = data() else { return };
    for level in ["levelL1", "levelA1"] {
        let g = Game::new(&fs, level, &[("WAR", "RED")]).unwrap();
        let infos = &g.level.worlds.item_infos;
        let (mut hidden, mut shown) = (0, 0);
        for inst in &g.level.worlds.item_instances {
            let Some(info) = infos.get(inst.item_index as usize) else {
                continue;
            };
            if info.item_type != ItemType::Trigger || inst.min_players > 1 {
                continue;
            }
            let pos = Vec3::from(inst.pos);
            let geo = items::attach_geometry(inst.flags);
            // Hidden and shown pads can share a spot: look for one that fits
            // (AddItemSub has put them on the floor).
            let fits = g.entities.iter().any(|e| {
                matches!(e.kind, Kind::Trigger { .. })
                    && (e.pos - pos).with_y(0.0).length() < 1e-3
                    && e.model.is_some() == geo
            });
            assert!(fits, "{level}: trigger at {pos} flags {}", inst.flags);
            if geo {
                shown += 1;
            } else {
                hidden += 1;
            }
        }
        assert!(
            hidden > 0 && shown > 0,
            "{level}: {hidden} hidden, {shown} shown"
        );
    }
}

/// levelA1 with nothing but chests, and the hero, settled on a side of the
/// chest nearest `at` that he can stand at, holding a key.
fn at_chest(fs: &GameFs, at: Vec3) -> (Game, u64, Vec3) {
    for side in 0..8 {
        let mut g = Game::new(fs, "levelA1", &[("WAR", "RED")]).unwrap();
        g.pending_enemies.clear();
        g.entities
            .retain(|e| matches!(e.kind, Kind::Container { chest: true, .. }));
        let e = g
            .entities
            .iter()
            .min_by(|a, b| a.pos.distance(at).total_cmp(&b.pos.distance(at)))
            .unwrap();
        let (id, pos) = (e.id, e.pos);
        let a = side as f32 * std::f32::consts::FRAC_PI_4;
        let stand = pos + Vec3::new(a.cos(), 0.0, a.sin()) * 2.8;
        for _ in 0..30 {
            g.players[0].pos = stand;
            g.update(1.0 / 60.0, &[PlayerInput::default()]);
        }
        let p = g.players[0].pos;
        if Vec2::new(p.x - pos.x, p.z - pos.z).length() < 2.9 && (p.y - pos.y).abs() < 1.0 {
            g.players[0].keys = 1;
            return (g, id, pos);
        }
    }
    panic!("no side of the chest at {at} to stand at");
}

#[test]
fn an_opened_chest_lifts_its_lid_then_leaves_its_item_to_take() {
    // fn_8005D730 case 2 sets active |= 1; fn_800606FC steps the chest's
    // cycle (activetype 0x16) through ACTIVE, the lid opening, to OPEN; at
    // action 2 the chest (no contents attached) is gone and fn_8005E90C's
    // item stands where it was.
    let Some(fs) = data() else { return };
    let (mut g, id, pos) = at_chest(&fs, Vec3::new(22.625, -2.40625, -3.75));
    let chest = |g: &Game| {
        g.entities
            .iter()
            .find(|e| e.id == id && e.alive)
            .map(|e| (e.model, e.animator.seq, e.animator.frame))
    };
    let model = chest(&g).unwrap().0.unwrap();
    let active = g.models[model].sequence("ACTIVE").unwrap();
    let step = |g: &mut Game| g.update(1.0 / 60.0, &[PlayerInput::default()]);
    step(&mut g);
    assert_eq!(g.players[0].keys, 0);
    let item = g
        .entities
        .iter()
        .find(|e| matches!(e.kind, Kind::Pickup { .. }))
        .map(|e| (e.id, e.pos))
        .expect("the chest's item");
    assert!(Vec2::new(item.1.x - pos.x, item.1.z - pos.z).length() < 1e-3);
    // The lid opens: ACTIVE plays through.
    let mut last = -1.0;
    for _ in 0..30 {
        step(&mut g);
        let (_, seq, frame) = chest(&g).expect("still there while opening");
        assert_eq!(seq, active);
        assert!(frame > last, "the opening plays");
        last = frame;
    }
    // Then OPEN, and the chest is gone.
    let mut ticks = 0;
    while chest(&g).is_some() {
        step(&mut g);
        ticks += 1;
        assert!(ticks < 300, "the chest never finished opening");
    }
    // Its item is there for the taking.
    assert!(g.entities.iter().any(|e| e.id == item.0 && e.alive));
    g.players[0].health = 100.0;
    g.players[0].pos = item.1;
    for _ in 0..3 {
        step(&mut g);
    }
    assert!(
        !g.entities.iter().any(|e| e.id == item.0 && e.alive),
        "taken"
    );
    assert!(g.players[0].health > 100.0, "BANANNA eaten");
}

#[test]
fn an_opened_gold_chest_gives_its_gold_on_touch() {
    // fn_8005E90C keeps a gold chest's treasure value; touching the open
    // chest gives it (fn_8005D730 case 2, action 2) and it goes 8 ticks on.
    let Some(fs) = data() else { return };
    let (mut g, id, _) = at_chest(&fs, Vec3::new(6.46875, -2.34375, -20.4375));
    assert!(matches!(
        g.entities.iter().find(|e| e.id == id).unwrap().kind,
        Kind::Container {
            sub: items::SUB_GOLDCHEST,
            ..
        }
    ));
    for _ in 0..300 {
        g.update(1.0 / 60.0, &[PlayerInput::default()]);
    }
    assert_eq!(g.players[0].gold, 100, "TREAS_SILVER");
    assert!(!g.entities.iter().any(|e| e.id == id && e.alive));
    assert!(!g
        .entities
        .iter()
        .any(|e| matches!(e.kind, Kind::Pickup { .. })));
}

/// The tower's world object called `name`.
fn object(g: &Game, name: &str) -> usize {
    g.level
        .worlds
        .world_objects
        .iter()
        .position(|o| o.name == name)
        .unwrap_or_else(|| panic!("no world object {name}"))
}

/// The mover of the trigger target `name`.
fn mover<'a>(g: &'a Game, name: &str) -> &'a crate::triggers::Mover {
    let o = object(g, name);
    g.movers
        .iter()
        .find(|m| m.root == o)
        .unwrap_or_else(|| panic!("no mover for {name}"))
}

/// The trigger entity with `id`.
fn trigger(g: &Game, id: u8) -> &Entity {
    g.entities
        .iter()
        .find(|e| matches!(e.kind, Kind::Trigger { id: t, .. } if t == id))
        .unwrap_or_else(|| panic!("no trigger {id}"))
}

fn step(g: &mut Game, n: usize) {
    for _ in 0..n {
        g.update(1.0 / 60.0, &[PlayerInput::default()]);
    }
}

#[test]
fn the_dragon_gate_opens_through_its_trigger_chain() {
    // "The dragon doors didn't open": the pad of the serpent gate (trigger
    // 101) drives only an invisible wall; the statue's heads, wings and jaws
    // hang off a chain of triggers named by nextid (LinkItemTriggers:
    // 101 -> 71 -> ... -> 78 -> 20 -> ... -> 27), each with its own target.
    // Setting off the head sets off the chain; a link target (flags 0x200)
    // never fires for a hero walking into it on its own.
    let Some(fs) = data() else { return };
    let mut g = Game::new(&fs, "levelL1", &[("WAR", "RED")]).unwrap();
    g.pending_enemies.clear();
    g.garg_items[0] = -1;
    use crate::worldanim::wflags::{AT_END, FORWARD};
    let parts = ["L1RSNAKEWA", "L1LSNAKEWD", "L1RSNAKERE", "L1LLSNAKET"];
    step(&mut g, 5);
    for p in parts {
        assert!(!mover(&g, p).active, "{p} waits for the gate");
    }
    // A hero on a chained pad (trigger 71's) on its own: nothing.
    g.players[0].pos = trigger(&g, 71).pos + Vec3::Y * 0.5;
    step(&mut g, 10);
    assert!(
        !mover(&g, "L1RSNAKEWA").active,
        "a link target is not set off by a hero on it"
    );
    // On the gate's own pad: the whole chain goes.
    g.players[0].pos = trigger(&g, 101).pos + Vec3::Y * 0.5;
    step(&mut g, 10);
    assert!(
        mover(&g, "L1TRAPWEASYA").active,
        "the gate's own wall opens"
    );
    for p in parts {
        let m = mover(&g, p);
        assert!(m.active, "{p} is set off down the chain");
        let f = g.world_anim.flags[m.root];
        assert!(f & FORWARD != 0, "{p} plays forward: {f:#x}");
    }
    step(&mut g, 600);
    for p in parts {
        let f = g.world_anim.flags[mover(&g, p).root];
        assert!(f & AT_END != 0, "{p} reaches its open pose: {f:#x}");
    }
}

#[test]
fn the_tower_starts_with_what_is_earned_already_in_place() {
    // EnterTower uses ActivateSpecialTrigger(id, 1): a target already earned
    // jumps to its finished state. The lion gate's lift and elevator must
    // not ride down again every time the tower loads, and the Citadel lift
    // and the statues' parts start at their ends.
    let Some(fs) = data() else { return };
    let mut g = Game::new(&fs, "levelL1", &[("WAR", "RED")]).unwrap();
    g.pending_enemies.clear();
    let lift = object(&g, "L1LIFT01");
    g.garg_items = [-1, -1, -1];
    g.shards = 0x1FFF;
    g.enter_tower();
    step(&mut g, 2);
    let dy = g.world_anim.delta(lift).w_axis.y;
    assert!(
        (-17.0..-16.0).contains(&dy),
        "the lift is already down at the Battle floor: {dy}"
    );
    assert!(!mover(&g, "L1LIFT01").moving, "and not riding there");
    let elev = mover(&g, "L1ELEV02");
    assert!(
        !elev.moving && (elev.offset - 1.0).abs() < 1e-3,
        "the elevator sits at its raised height: {} moving {}",
        elev.offset,
        elev.moving
    );
    use crate::worldanim::wflags::AT_END;
    for p in [
        "L1ELEV669",
        "L1RSNAKEWA",
        "L1LEAGLEWA",
        "L1LIONUARM",
        "L1BLK669",
    ] {
        let m = mover(&g, p);
        let f = g.world_anim.flags[m.root];
        assert!(
            m.active && !m.moving && f & AT_END != 0,
            "{p} starts at its end: {f:#x} moving {}",
            m.moving
        );
    }
    // Nothing in the tower is still on its way anywhere.
    step(&mut g, 60);
    assert!(
        g.movers.iter().all(|m| !m.moving),
        "no target animates on entry"
    );
    let again = g.world_anim.delta(lift).w_axis.y;
    assert!(
        (again - dy).abs() < 1e-3,
        "the lift stays put: {again} vs {dy}"
    );
}
