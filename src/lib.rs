//! The Playground scene, input bindings, and integration probes for bevy_als.

#[path = "als_extras/ui/als_hud.rs"]
pub mod als_hud;
#[path = "tests/anim_probe.rs"]
pub mod anim_probe;
#[path = "tests/animation_visual_validation.rs"]
pub mod animation_visual_validation;
#[path = "tests/decal_diagnostics.rs"]
pub mod decal_diagnostics;
#[path = "als_extras/core/game_mode.rs"]
pub mod game_mode;
#[path = "als_extras/core/input.rs"]
pub mod input;
#[path = "tests/manual_playground_integration/mod.rs"]
pub mod manual_playground_integration;
#[path = "als_extras/ui/overlay_mode_menu.rs"]
pub mod overlay_mode_menu;
#[path = "als_extras/core/player_start.rs"]
pub mod player_start;
#[path = "als_extras/levels/playground.rs"]
pub mod playground;
#[path = "als_extras/levels/playground_collision.rs"]
pub mod playground_collision;
#[path = "als_extras/environment/playground_materials.rs"]
mod playground_materials;
#[path = "als_extras/environment/playground_sky.rs"]
mod playground_sky;
#[path = "bevy_ext/source_gaussian_bloom.rs"]
mod source_gaussian_bloom;
#[path = "bevy_ext/source_world_time.rs"]
pub mod source_world_time;
#[path = "bevy_ext/text_overflow.rs"]
mod text_overflow;

#[cfg(test)]
#[path = "tests/input_integration.rs"]
mod input_integration;
#[cfg(test)]
#[path = "tests/scene_integration.rs"]
mod scene_integration;
#[cfg(test)]
#[path = "tests/test_support.rs"]
mod test_support;

#[path = "bevy_ext/packed_footprints.rs"]
mod packed_footprints;
#[path = "bevy_ext/projected_decal.rs"]
mod projected_decal;
pub use packed_footprints::PackedFootprintsPlugin;
