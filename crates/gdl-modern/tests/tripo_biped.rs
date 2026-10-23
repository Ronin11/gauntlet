//! Loads a real Tripo rig when `GDL_MODERN_TEST_GLB` points at one.

use gdl_modern::{mapping, ModernModel};

#[test]
fn biped_mapping_covers_the_rig() {
    let Ok(path) = std::env::var("GDL_MODERN_TEST_GLB") else {
        eprintln!("GDL_MODERN_TEST_GLB not set; skipping");
        return;
    };
    let m = ModernModel::load(path).unwrap();
    eprintln!(
        "{} verts, {} tris, {} joints",
        m.vertices.len(),
        m.indices.len() / 3,
        m.joints.len()
    );
    for (node, joint) in mapping::BIPED {
        assert!(m.joint(joint).is_some(), "{node} -> {joint}: no such joint");
    }
    assert!(m.texture.is_some());
    assert!(m
        .vertices
        .iter()
        .all(|v| (v.weights.iter().sum::<f32>() - 1.0).abs() < 0.05));
}

/// With the game pose equal to its reference, the skin matrices are identity.
#[test]
fn rest_pose_skins_to_itself() {
    use gdl_modern::retarget::{skin, Retarget};
    let Ok(path) = std::env::var("GDL_MODERN_TEST_GLB") else {
        return;
    };
    let m = ModernModel::load(path).unwrap();
    let names = ["L1ROOT", "ROOT_PELVIS", "L_HIP", "R_HIP"];
    let mut game = vec![glam::Mat4::IDENTITY; 4];
    game[1] = glam::Mat4::from_translation(glam::Vec3::new(0.0, 2.6, 0.0));
    game[2] = glam::Mat4::from_translation(glam::Vec3::new(-0.47, 2.7, 0.0));
    game[3] = glam::Mat4::from_translation(glam::Vec3::new(0.5, 2.7, 0.0));
    let map = [
        ("ROOT_PELVIS", "Hip"),
        ("L_HIP", "L_Thigh"),
        ("R_HIP", "R_Thigh"),
    ];
    let r = Retarget::new(&m, &names, &game, &map, &[], mapping::BIPED_FORWARD).unwrap();
    let mats = r.skin_matrices(&m, &game, &game);
    let posed = skin(&m, &mats);
    let worst = posed
        .iter()
        .zip(&m.vertices)
        .map(|((p, _), v)| p.distance(v.pos))
        .fold(0.0, f32::max);
    assert!(worst < 1e-3, "rest pose moved a vertex by {worst}");
}
