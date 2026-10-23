//! Floor shadows: the flat SHADOW objects of each model bank (a 32x32 black
//! blob, alpha <= 146, on a square at y = 0) laid on the floor under heroes
//! (load_player_geo's SHADOWL1), monsters (SetEnemyObj's SHADOW1L1..3L1) and
//! critters whose type has flag 1 (CritterInitGeo). Items have none. Size
//! and opacity never depend on the height above the floor: a shadow node is
//! simply re-placed every frame. There is no option to turn them off.

use crate::character::Model;
use crate::fs::GameFs;
use crate::game::{EnemyState, Entity, Game, Kind, Player, PlayerState};
use gdl_formats::Objects;
use glam::{Mat3, Mat4, Vec3};

/// lbl_8011BFF8 (enemy.c) / lbl_8011AEA0 (critter.c): the shadow objects by
/// tier or critter subtype.
pub const ENEMY_SHADOWS: [&str; 3] = ["SHADOW1L1", "SHADOW2L1", "SHADOW3L1"];
/// load_player_geo's shadow object, in the hero's own bank.
pub const PLAYER_SHADOW: &str = "SHADOWL1";

/// lbl_80347BE0 (PlayerMotion) and CritterCollideWorld's 0.1: how far over
/// the floor a hero's or critter's shadow sits. A monster's sits at its feet.
pub const LIFT: f32 = 0.1;

/// The shadow nodes' zmod (mbnode 0x68, a z-buffer offset toward the eye):
/// -32 for monsters and critters (SetEnemyObj, CritterInitGeo), -0x24 for
/// heroes (load_player_geo), so a hero's shadow wins over a monster's.
pub const ENEMY_ZMOD: f32 = -32.0;
pub const PLAYER_ZMOD: f32 = -36.0;
/// [CHOICE] World units the shadow is pulled toward the eye per zmod step
/// (pb_objregs scales zmod by the light state's f78, not decoded): -32 is
/// 0.25 units, enough to clear the floor's depth at play distances.
const ZMOD_UNIT: f32 = 1.0 / 128.0;

/// CreateMat3Norm's snap: normals within 0.999 of straight up (or down) give
/// the identity (or its flip).
const SNAP: f32 = 0.999;

/// A shadow object as a static model, if the bank has it.
pub fn load(objects: &Objects, textures: &[u8], name: &str) -> Option<Model> {
    objects.object_by_name(name)?;
    Model::from_objects(objects, textures, &[name]).filter(|m| !m.mesh.indices.is_empty())
}

/// A hero's SHADOWL1 (PLAYERS/<class>/<color>).
pub fn load_player(fs: &GameFs, class: &str, color: &str) -> Option<Model> {
    let dir = format!("PLAYERS/{class}/{color}");
    let objects = Objects::parse(&fs.read(&format!("{dir}/objects.ngc")).ok()?).ok()?;
    let textures = fs.read(&format!("{dir}/textures.ngc")).ok()?;
    load(&objects, &textures, PLAYER_SHADOW)
}

/// SetEnemyObj: every monster type but 0, 21 (acid blob), DEATH and IT
/// gets a shadow.
pub fn enemy_has_shadow(etype: usize) -> bool {
    !matches!(etype, 0 | 21 | crate::enemies::DEATH | crate::enemies::IT)
}

/// SetEnemyObj: the level clamped to 1..=3 picks SHADOW<level>L1 (archers,
/// bombers and suicides, placed 4-6, take SHADOW3L1).
pub fn enemy_shadow_tier(level: usize) -> usize {
    level.clamp(1, 3) - 1
}

/// CritterInitGeo: SHADOW<subtype + 1>L1, SHADOW2L1 past subtype 2.
pub fn critter_shadow_name(subtype: i16) -> &'static str {
    ENEMY_SHADOWS[if subtype > 2 {
        1
    } else {
        subtype.max(0) as usize
    }]
}

/// CreateMat3Norm: an orthonormal basis whose Y axis is the floor normal `n`
/// (rows x = (-nz, 0, nx), y = n, z = (-nx ny, 1 - ny^2, -ny nz), the x and z
/// rows normalised); near-vertical normals snap to the identity or its flip.
pub fn floor_basis(n: Vec3) -> Mat3 {
    if n.y > SNAP {
        return Mat3::IDENTITY;
    }
    if n.y < -SNAP {
        // lbl_80127DA0: rows (1, 0, 0), (0, -1, 0), (0, 0, -1).
        return Mat3::from_diagonal(Vec3::new(1.0, -1.0, -1.0));
    }
    let x = Vec3::new(-n.z, 0.0, n.x).normalize();
    let z = Vec3::new(-n.x * n.y, 1.0 - n.y * n.y, -n.y * n.z).normalize();
    Mat3::from_cols(x, n, z)
}

/// The shadow node's matrix: the floor basis at `at`, pulled `zmod` toward
/// the eye along the line of sight (so it stays where it is on screen).
pub fn shadow_matrix(basis: Mat3, at: Vec3, eye: Vec3, zmod: f32) -> Mat4 {
    let pull = (eye - at).normalize_or_zero() * (-zmod * ZMOD_UNIT);
    Mat4::from_translation(at + pull) * Mat4::from_mat3(basis)
}

/// A shadow to draw: its model and game-space matrix.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    pub model: usize,
    pub transform: Mat4,
}

impl Game {
    /// The floor basis under `pos` (the triangle a probe from `top` lands
    /// on), or the identity where there is none.
    fn shadow_basis(&self, pos: Vec3, top: f32) -> Mat3 {
        self.collision
            .ground_tri(pos, top)
            .map_or(Mat3::IDENTITY, |(_, t)| {
                floor_basis(self.collision.tris[t].n)
            })
    }

    /// A hero's shadow (PlayerMotion): flat (the node keeps the identity),
    /// at its x/z and 0.1 over floor_base; hidden off the floor, out of
    /// play, and in wave 0x0C08 (sMusicTrackHi 0xC, Lo 8).
    pub fn player_shadow(&self, p: &Player) -> Option<Placed> {
        let model = *self.shadow_models.get(&p.model)?;
        if self.wave == Some(0x0C08)
            || matches!(p.state, PlayerState::Exited | PlayerState::InTower)
        {
            return None;
        }
        let floor = self.collision.ground_height(p.pos, p.pos.y + 0.5)?;
        let at = Vec3::new(p.pos.x, floor + LIFT, p.pos.z);
        let eye = self.camera_view().0;
        Some(Placed {
            model,
            transform: shadow_matrix(Mat3::IDENTITY, at, eye, PLAYER_ZMOD),
        })
    }

    /// A monster's or critter's shadow. Monsters (enemy_update, the
    /// collision's CopyMat3): the floor basis under it at its own position,
    /// hidden while it emerges (E_START). Critters (CritterCollideWorld): the
    /// floor basis at its x/z, 0.1 over the floor.
    pub fn entity_shadow(&self, e: &Entity) -> Option<Placed> {
        let eye = self.camera_view().0;
        match &e.kind {
            Kind::Enemy {
                class,
                strength,
                state,
                etype,
                ..
            } => {
                if !enemy_has_shadow(*etype) || matches!(state, EnemyState::Spawning) {
                    return None;
                }
                let model = self.classes.get(*class)?.shadows[enemy_shadow_tier(*strength)]?;
                let hht = crate::enemies::TYPES
                    .get(*etype)
                    .map_or(3.0, |t| t.height * 0.5);
                let basis = self.shadow_basis(e.pos, e.pos.y + hht);
                Some(Placed {
                    model,
                    transform: shadow_matrix(basis, e.pos, eye, ENEMY_ZMOD),
                })
            }
            Kind::Critter(_) => {
                let model = *self.shadow_models.get(&e.model?)?;
                let y = self.critter_floor(e.pos)?;
                let basis = self.shadow_basis(e.pos, e.pos.y + 4.0);
                let at = Vec3::new(e.pos.x, y + LIFT, e.pos.z);
                Some(Placed {
                    model,
                    transform: shadow_matrix(basis, at, eye, ENEMY_ZMOD),
                })
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> Option<GameFs> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game-data");
        GameFs::open(&root).ok()
    }

    #[test]
    fn flat_floor_is_identity() {
        assert_eq!(floor_basis(Vec3::Y), Mat3::IDENTITY);
        assert_eq!(
            floor_basis(Vec3::new(0.0, -1.0, 0.0)),
            Mat3::from_diagonal(Vec3::new(1.0, -1.0, -1.0))
        );
    }

    /// On a slope the basis is orthonormal, right-handed, and its Y axis is
    /// the normal: the shadow's square lies in the floor plane.
    #[test]
    fn slope_basis_lies_in_the_floor() {
        let n = Vec3::new(0.3, 0.9, -0.2).normalize();
        let b = floor_basis(n);
        assert!((b.y_axis - n).length() < 1e-6);
        assert!(b.x_axis.dot(n).abs() < 1e-6 && b.z_axis.dot(n).abs() < 1e-6);
        assert!(b.x_axis.dot(b.z_axis).abs() < 1e-6);
        assert!((b.determinant() - 1.0).abs() < 1e-5);
        // A corner of the square lands on the plane through the origin.
        let c = b * Vec3::new(1.59375, 0.0, -1.59375);
        assert!(c.dot(n).abs() < 1e-5);
        // CreateMat3Norm's x row has no y.
        assert_eq!(b.x_axis.y, 0.0);
    }

    /// The zmod pull moves the shadow along the line of sight toward the eye.
    #[test]
    fn zmod_pulls_toward_the_eye() {
        let at = Vec3::new(10.0, 0.0, 5.0);
        let eye = Vec3::new(10.0, 40.0, -25.0);
        let m = shadow_matrix(Mat3::IDENTITY, at, eye, ENEMY_ZMOD);
        let p = m.transform_point3(Vec3::ZERO);
        assert!(((p - at).length() - 0.25).abs() < 1e-5);
        assert!((p - at).normalize().dot((eye - at).normalize()) > 0.9999);
        let hero = shadow_matrix(Mat3::IDENTITY, at, eye, PLAYER_ZMOD);
        assert!(hero.transform_point3(Vec3::ZERO).distance(eye) < p.distance(eye));
    }

    #[test]
    fn tiers_and_subtypes() {
        assert_eq!(enemy_shadow_tier(0), 0);
        assert_eq!(enemy_shadow_tier(2), 1);
        assert_eq!(enemy_shadow_tier(6), 2);
        assert_eq!(critter_shadow_name(0), "SHADOW1L1");
        assert_eq!(critter_shadow_name(2), "SHADOW3L1");
        assert_eq!(critter_shadow_name(5), "SHADOW2L1");
        assert!(!enemy_has_shadow(0) && !enemy_has_shadow(21) && !enemy_has_shadow(30));
        assert!(enemy_has_shadow(crate::enemies::GRUNT));
    }

    /// The bank's shadows are flat blended squares: SHADOW1L1 1.59 across in
    /// GRU, the Yeti's 10.
    #[test]
    fn shadow_objects_load() {
        let Some(fs) = data() else { return };
        let o = Objects::parse(&fs.read("MONSTERS/GRU/objects.ngc").unwrap()).unwrap();
        let t = fs.read("MONSTERS/GRU/textures.ngc").unwrap();
        let m = load(&o, &t, "SHADOW1L1").unwrap();
        assert_eq!(m.mesh.bounds_min.y, 0.0);
        assert_eq!(m.mesh.bounds_max.y, 0.0);
        assert!((m.mesh.bounds_max.x - 1.59375).abs() < 1e-4);
        assert!(m.mesh.batches.iter().all(|b| b.blend));
        assert!(load_player(&fs, "WAR", "RED").is_some());
    }

    /// In a level every hero and the monsters get a shadow on the floor
    /// under them.
    #[test]
    fn level_shadows_sit_on_the_floor() {
        let Some(fs) = data() else { return };
        let g = Game::new(&fs, "levelA1", &[("WAR", "RED")]).unwrap();
        let s = g.player_shadow(&g.players[0]).expect("hero shadow");
        let at = s.transform.transform_point3(Vec3::ZERO);
        let floor = g
            .collision
            .ground_height(g.players[0].pos, g.players[0].pos.y + 0.5)
            .unwrap();
        assert!(
            (at.y - (floor + LIFT)).abs() < 0.3,
            "{at:?} vs floor {floor}"
        );
        let mut n = 0;
        for e in &g.entities {
            if let Some(s) = g.entity_shadow(e) {
                assert!(s.model >= g.players.len());
                n += 1;
            }
        }
        assert!(n > 0, "no monster shadows");
    }

    /// The Yeti's type has flag 1: its 10-unit SHADOW1L1 lies 0.1 over the
    /// floor under it; the Dragon (no flag) has none.
    #[test]
    fn critter_shadows_by_type_flag() {
        let Some(fs) = data() else { return };
        let g = Game::new(&fs, "levelI5", &[("WAR", "RED")]).unwrap();
        let yeti = g
            .entities
            .iter()
            .find(|e| matches!(&e.kind, Kind::Critter(c) if c.wad.types[0].flags & 1 != 0))
            .expect("the Yeti");
        let s = g.entity_shadow(yeti).expect("its shadow");
        assert!((g.models[s.model].mesh.bounds_max.x - 10.0).abs() < 1e-3);
        let at = s.transform.transform_point3(Vec3::ZERO);
        let floor = g.critter_floor(yeti.pos).unwrap();
        assert!((at.y - (floor + LIFT)).abs() < 0.3, "{at:?} vs {floor}");
        let g = Game::new(&fs, "levelB6", &[("WAR", "RED")]).unwrap();
        assert!(g
            .entities
            .iter()
            .filter(|e| matches!(e.kind, Kind::Critter(_)))
            .all(|e| g.entity_shadow(e).is_none()));
    }
}
