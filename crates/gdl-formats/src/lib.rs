//! File formats used by Gauntlet Dark Legacy (GameCube release, GUNE5D).
//!
//! Format knowledge comes from the community tools (MosesofEgypt/gdl_tools,
//! haekb/io_scene_gdl, MeltyTool) and was verified against the retail disc.

pub mod anim;
pub mod audio;
pub mod bin;
pub mod mesh;
pub mod movie;
pub mod objects;
pub mod sequence;
pub mod texture;
pub mod wdata;
pub mod worlds;

pub use mesh::Mesh;
pub use objects::Objects;
pub use texture::Image;
pub use worlds::Worlds;
