//! Loads a real Tripo rig when `GDL_MODERN_TEST_GLB` points at one.

use gdl_modern::{mapping, ModernModel};

#[test]
fn biped_mapping_covers_the_rig() {
    let Ok(path) = std::env::var("GDL_MODERN_TEST_GLB") else {
        eprintln!("GDL_MODERN_TEST_GLB not set; skipping");
        return;
    };
    let m = ModernModel::load(path).unwrap();
    eprintln!("{} verts, {} tris, {} joints", m.vertices.len(), m.indices.len() / 3, m.joints.len());
    for (node, joint) in mapping::BIPED {
        assert!(m.joint(joint).is_some(), "{node} -> {joint}: no such joint");
    }
    assert!(m.texture.is_some());
    assert!(m.vertices.iter().all(|v| (v.weights.iter().sum::<f32>() - 1.0).abs() < 0.05));
}
