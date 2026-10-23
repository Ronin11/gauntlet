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
