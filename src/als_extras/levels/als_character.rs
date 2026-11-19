mod logging;

use avian3d::prelude::{Collider, CollisionLayers, LayerMask, RigidBody};
use bevy::{
    input::{InputSystems, mouse::AccumulatedMouseMotion},
    prelude::*,
};
use bevy_als::{
    AlsCharacterBlueprint, AlsCollisionLayer, AlsControlSchemeConfig, AlsInputCommand,
    AlsMovementParams, AlsPlugin, spawn_als_character,
};

use bevy_als::character_assembly::AlsPlayerCharacter;

fn main() {
    App::new()
        .add_plugins((DefaultPlugins.set(logging::log_plugin()), AlsPlugin))
        .add_systems(Startup, setup)
        .add_systems(PreUpdate, drive_character.after(InputSystems))
        .run();
}

fn setup(
    mut commands: Commands,
    assets: Res<AssetServer>,
    mut params: ResMut<Assets<AlsMovementParams>>,
    mut configs: ResMut<Assets<AlsControlSchemeConfig>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        RigidBody::Static,
        Collider::cuboid(24.0, 0.2, 24.0),
        CollisionLayers::new(
            [AlsCollisionLayer::Default, AlsCollisionLayer::Visibility],
            LayerMask::ALL,
        ),
        Mesh3d(meshes.add(Cuboid::new(24.0, 0.2, 24.0))),
        MeshMaterial3d(materials.add(Color::srgb(0.24, 0.32, 0.39))),
        Transform::from_xyz(0.0, -0.1, 0.0),
    ));
    commands.spawn((
        DirectionalLight::default(),
        Transform::from_xyz(3.0, 8.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    let character = spawn_als_character(
        &mut commands,
        &assets,
        &mut params,
        &mut configs,
        AlsCharacterBlueprint {
            spawn_camera: true,
            ..AlsCharacterBlueprint::new(Transform::from_xyz(0.0, 1.5, 0.0))
        },
    );
    commands.entity(character).insert(AlsPlayerCharacter);
}

/// A host can translate any input source into this command each frame.
fn drive_character(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<AccumulatedMouseMotion>,
    mut players: Query<&mut AlsInputCommand, With<AlsPlayerCharacter>>,
) {
    let axis =
        |positive, negative| f32::from(keys.pressed(positive)) - f32::from(keys.pressed(negative));
    for mut input in &mut players {
        *input = AlsInputCommand {
            move_dir: Vec2::new(
                axis(KeyCode::KeyD, KeyCode::KeyA),
                axis(KeyCode::KeyW, KeyCode::KeyS),
            )
            .clamp_length_max(1.0),
            look_delta: mouse.delta,
            jump_pressed: keys.just_pressed(KeyCode::Space),
            jump_held: keys.pressed(KeyCode::Space),
            ..default()
        };
    }
}
