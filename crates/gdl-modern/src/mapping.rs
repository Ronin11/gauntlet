//! Which glTF joint stands in for each game skeleton node.
//!
//! The game's heroes are 26-node rigid-part skeletons (`L1ROOT`, `ROOT_PELVIS`,
//! `L_HIP`, ...). A modern rig has many more joints; each game node drives one
//! joint (the rest follow their parents), so a mapping is a short list of
//! `game node -> joint` pairs.

/// `(game node, joint)` pairs for a Tripo/Mixamo-style biped.
pub const BIPED: &[(&str, &str)] = &[
    ("ROOT_PELVIS", "Hip"),
    ("L_HIP", "L_Thigh"),
    ("L_KNEE", "L_Calf"),
    ("L_ANKLE", "L_Foot"),
    ("R_HIP", "R_Thigh"),
    ("R_KNEE", "R_Calf"),
    ("R_ANKLE", "R_Foot"),
    ("TORSO_SEC", "Waist"),
    ("CHEST_SE", "Spine02"),
    ("CHEST_L_ARM_2", "L_Clavicle"),
    ("L_SHOULD", "L_Upperarm"),
    ("L_ELBO", "L_Forearm"),
    ("L_WRIST", "L_Hand"),
    ("CHEST_R_ARM_2", "R_Clavicle"),
    ("R_SHOULD", "R_Upperarm"),
    ("R_ELBO", "R_Forearm"),
    ("R_WRIST", "R_Hand"),
    ("NECK", "NeckTwist01"),
    ("HEAD", "Head"),
];

/// `(game node, game node it points at)`: bones retargeted by direction.
/// Pelvis, hands, feet and head have no aim and follow the game's rotation.
pub const BIPED_AIM: &[(&str, &str)] = &[
    ("L_HIP", "L_KNEE"),
    ("L_KNEE", "L_ANKLE"),
    ("R_HIP", "R_KNEE"),
    ("R_KNEE", "R_ANKLE"),
    ("TORSO_SEC", "CHEST_SE"),
    ("CHEST_SE", "NECK"),
    ("CHEST_L_ARM_2", "L_SHOULD"),
    ("L_SHOULD", "L_ELBO"),
    ("L_ELBO", "L_WRIST"),
    ("CHEST_R_ARM_2", "R_SHOULD"),
    ("R_SHOULD", "R_ELBO"),
    ("R_ELBO", "R_WRIST"),
    ("NECK", "HEAD"),
];

/// Two joints (heel, toe) that point the way a biped rig faces.
pub const BIPED_FORWARD: (&str, &str) = ("L_Foot", "L_ToeBase");

/// The root joint of each limb of a biped; see [`crate::ModernModel::separate_limbs`].
pub const BIPED_LIMBS: &[&str] = &["L_Thigh", "R_Thigh", "L_Clavicle", "R_Clavicle"];
