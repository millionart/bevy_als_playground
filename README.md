# Bevy ALS Playground

A runnable example of a game using [bevy_als](https://github.com/millionart/bevy_als) with Bevy 0.20.1. The full scene includes the default keyboard, mouse, and gamepad adapter, example geometry, HUD, and scene regression tools.

## Add ALS to a game

1. Add `bevy = "0.20.1"` and copy the pinned `bevy_als` dependency from [Cargo.toml](Cargo.toml).
2. Register `.add_plugins((DefaultPlugins, bevy_als::AlsPlugin))` and call `bevy_als::spawn_als_character` in a startup system. The [character example](src/als_extras/levels/als_character.rs) shows the spawn call, camera, and an Avian floor.
3. Write `AlsInputCommand` on each controlled actor from your input system in `PreUpdate`. Refresh values every frame and pulse `*_pressed` fields for one update. The character example maps WASD, mouse look, and jump with Bevy input resources.

If you already have a camera, give it `bevy::core_pipeline::prepass::DepthPrepass` to render ALS footprints. If you already install Avian, use `AlsPlugin::physics_plugins()` to configure it.

For the full Playground bindings, see [the adapter](src/als_extras/core/input.rs) and [the app setup](src/als_extras/levels/manual_playground.rs). The app installs `AlsInputPlugin` and calls `spawn_als_actions` with `AlsInputMap::default()` for each local player. This adapter is where the app depends on Enhanced Input.

## Run the Playground

Cargo fetches the revision in [Cargo.toml](Cargo.toml). The plugin supplies the character, camera, and animation assets. From the Playground root, run:

```sh
cargo run --locked --jobs 2
```

Use WASD to move, the mouse to look, and Space to jump. Hold Left Shift to sprint or tap it to roll. Use Left Ctrl to crouch, right mouse to aim, X for ragdoll, and Q with the scroll wheel for the overlay menu.

Run the standalone input example with `cargo run --locked --jobs 2 --example als_character`. It uses the same ALS runtime and Bevy's built-in keyboard and mouse resources.

## Tests

```sh
cargo test --locked --jobs 2 -- --test-threads=1
```

A render integration scenario can be run on a desktop with a GPU. For example, `ALS_MANUAL_INTEGRATION_GATE=1 ALS_MANUAL_INTEGRATION_CASE=footprint cargo run --locked --jobs 2` checks the Playground's packed decal renderer as well as footprint attachment and lifetime.

`assets/ALSExtras` belongs to the Playground. `assets/Validation` contains alternative validation graphs and scenes. The code is MIT licensed. Bundled ALS-Refactored and Unreal Engine assets retain their original licenses. See [LICENSE](LICENSE).
