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
        g.level.worlds.item_infos[i].active_type &= !0x10;
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
    g.player_attack(0, pos, yaw, 1.0, false);
    assert_eq!(g.players[1].health, hp, "Normal: melee passes heroes");
    g.friendly_fire = 2;
    g.player_attack(0, pos, yaw, 1.0, false);
    assert!(g.players[1].health < hp, "Shots Hurt: the swing lands");
    // Behind the swing: missed.
    let hp = g.players[1].health;
    g.players[1].pos = pos - Vec3::new(0.0, 0.0, 3.0);
    g.player_attack(0, pos, yaw, 1.0, false);
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
