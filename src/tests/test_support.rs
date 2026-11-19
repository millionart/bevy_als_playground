use std::{sync::LazyLock, time::Duration};

use bevy::{ecs::system::RunSystemOnce, prelude::*, time::TimeUpdateStrategy};
use bevy_als::{
    AlsPlugin, Gait, LocomotionState, RotationMode, Stance,
    als_camera::{AlsCameraDefinitionHandle, AlsCharacterCamera},
    camera_component::AlsSourceCameraComponent,
    overlay_object::OverlayLibrary,
};
use bevy_animation_graph::core::{
    animated_scene::{AnimatedScene, AnimatedSceneHandle, AnimatedSceneInstance},
    animation_graph_player::AnimationGraphPlayer,
};

use crate::input::AlsInputPlugin;

pub(crate) static SOURCE_LIBRARY: LazyLock<OverlayLibrary> = LazyLock::new(|| {
    serde_json::from_slice(
        bevy_als::als_asset_bytes("ALS/Character/overlay_actor.runtime.json")
            .expect("ALS data must be packaged"),
    )
    .expect("ALS overlay definitions")
});

pub(crate) fn headless_app() -> App {
    let mut app = App::new();
    app.register_required_components::<bevy_als::character_assembly::AlsCharacterAvatar, bevy_als::animation_visibility::AlsAnimationTickOption>();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin::default(),
        bevy::gltf::GltfPlugin::default(),
        bevy::world_serialization::WorldSerializationPlugin,
        TransformPlugin,
        bevy::input::InputPlugin,
        bevy::gizmos::GizmoPlugin,
        bevy::animation::AnimationPlugin,
    ))
    .init_asset::<bevy::world_serialization::WorldAsset>()
    .init_asset::<Mesh>()
    .init_asset::<Image>()
    .init_asset::<StandardMaterial>()
    .init_asset::<bevy::mesh::skinning::SkinnedMeshInverseBindposes>()
    .add_plugins((AlsPlugin, AlsInputPlugin))
    .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(
        1.0 / 60.0,
    )));
    app
}

fn camera_scene() -> (App, Entity, Entity, Handle<AnimatedScene>) {
    let mut app = headless_app();
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
        100,
    )));
    let definition = app
        .world()
        .resource::<AssetServer>()
        .load(bevy_als::als_asset_path("ALSCamera/Data/als_camera.camera_component.ron"));
    let scene = app
        .world()
        .resource::<AssetServer>()
        .load(bevy_als::als_asset_path("ALSCamera/AB_Als_Camera.static.animscn.ron"));
    let character = app
        .world_mut()
        .spawn((
            Name::new("Owner"),
            Transform::IDENTITY,
            AlsCameraDefinitionHandle(definition),
            Gait::Walking,
            Stance::Standing,
            LocomotionState::Grounded,
            RotationMode::ViewDirection,
            bevy_als::als_camera::AlsCameraCustomTimeDilation(0.5),
        ))
        .id();
    let component = app
        .world_mut()
        .spawn((
            Name::new("Camera"),
            AlsCharacterCamera { character },
            AlsSourceCameraComponent::default(),
            AnimatedSceneHandle::new(scene.clone()),
            ChildOf(character),
        ))
        .id();
    app.world_mut().entity_mut(character).insert((
        bevy_als::als_camera::AlsCameraTarget,
        bevy_als::als::motion::AlsViewYaw(0.0),
        bevy_als::als::view::AlsViewPitch(0.0),
        bevy_als::als_camera::AlsCameraSocketLocations {
            root: Vec3::ZERO,
            head: Vec3::Y * 1.6,
            first_person: Vec3::Y * 1.6,
            trace_shoulder_right: Vec3::new(0.2, 1.4, 0.0),
            trace_shoulder_left: Vec3::new(-0.2, 1.4, 0.0),
            mesh_rotation: Quat::IDENTITY,
            mesh_scale: 1.0,
            valid: true,
        },
    ));
    app.finish();
    app.cleanup();
    (app, character, component, scene)
}

pub(crate) fn loaded_camera_scene() -> (App, Entity, Entity, Handle<AnimatedScene>) {
    let (mut app, character, component, scene) = camera_scene();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(40);
    loop {
        app.update();
        let server = app.world().resource::<AssetServer>();
        if let bevy::asset::LoadState::Failed(error) = server.load_state(scene.id()) {
            panic!("Camera asset failed: {error}");
        }
        if let Some(instance) = app.world().get::<AnimatedSceneInstance>(component) {
            let player = app
                .world()
                .get::<AnimationGraphPlayer>(instance.player_entity())
                .unwrap();
            assert!(player.get_error().is_none(), "{:?}", player.get_error());
            if player.get_default_output_pose().is_some() {
                break;
            }
        }
        assert!(
            std::time::Instant::now() < deadline,
            "Camera scene load made no progress: asset={:?}, world_root={}, instance={}",
            server.load_state(scene.id()),
            app.world()
                .get::<bevy::world_serialization::WorldAssetRoot>(component)
                .is_some(),
            app.world()
                .get::<AnimatedSceneInstance>(component)
                .is_some()
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    (app, character, component, scene)
}

fn ready_roll_player(app: &mut App, avatar: Entity) -> Entity {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(40);
    loop {
        app.update();
        if let Some(instance) = app.world().get::<AnimatedSceneInstance>(avatar)
            && let Some(player) = app
                .world()
                .get::<AnimationGraphPlayer>(instance.player_entity())
            && player.get_default_output_pose().is_some()
        {
            let target = instance.player_entity();
            if app
                .world_mut()
                .query::<(
                    &bevy_als::roll_drive::AlsRollSlotSampler,
                    &AnimationGraphPlayer,
                )>()
                .iter(app.world())
                .any(|(sampler, player)| {
                    sampler.target_player() == target && player.get_default_output_pose().is_some()
                })
            {
                return target;
            }
        }
        assert!(
            std::time::Instant::now() < deadline,
            "Roll player did not become ready"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

pub(crate) fn loaded_character() -> (App, Entity, Entity) {
    use bevy_als::character_assembly::{
        AlsCharacterAssembly, AlsCharacterBlueprint, spawn_als_character,
    };
    let (mut app, _, _, _) = loaded_camera_scene();
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        std::time::Duration::from_secs_f32(1.0 / 60.0),
    ));
    app.world_mut().spawn((
        avian3d::prelude::RigidBody::Static,
        avian3d::prelude::Collider::cuboid(100.0, 1.0, 100.0),
        Transform::from_xyz(0.0, -0.5, 0.0),
        avian3d::prelude::CollisionLayers::new(
            [
                bevy_als::als::character::AlsCollisionLayer::Default,
                bevy_als::als::character::AlsCollisionLayer::Visibility,
            ],
            avian3d::prelude::LayerMask::ALL,
        ),
    ));
    let character =
        app.world_mut()
            .run_system_once(
                |mut commands: Commands,
                 assets: Res<AssetServer>,
                 mut params: ResMut<Assets<bevy_als::als::params::AlsMovementParams>>,
                 mut configs: ResMut<
                    Assets<bevy_als::als::control_scheme::AlsControlSchemeConfig>,
                >| {
                    spawn_als_character(
                        &mut commands,
                        &assets,
                        &mut params,
                        &mut configs,
                        AlsCharacterBlueprint::new(Transform::from_xyz(0.0, 1.5, 0.0)),
                    )
                },
            )
            .unwrap();
    let avatar = app
        .world()
        .get::<AlsCharacterAssembly>(character)
        .unwrap()
        .avatar;
    ready_roll_player(&mut app, avatar);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while app
        .world()
        .get::<bevy_als::als::character::AlsBirthAnimationPending>(character)
        .is_some()
        || app
            .world()
            .get::<bevy_als::als::character::AlsInitialMovementModePending>(character)
            .is_some()
        || *app.world().get::<LocomotionState>(character).unwrap() != LocomotionState::Grounded
    {
        app.update();
        assert!(
            std::time::Instant::now() < deadline,
            "Roll fixture did not reach the ground"
        );
    }
    (app, character, avatar)
}

pub(crate) fn loaded_bow_scene() -> (App, Entity, Entity) {
    let (mut app, _, _, _) = loaded_camera_scene();
    let scene = app
        .world()
        .resource::<AssetServer>()
        .load(bevy_als::als_asset_path("ALS/OverlayObjects/Bow/B_Als_Bow.animscn.ron"));
    let object = app
        .world_mut()
        .spawn(AnimatedSceneHandle::new(scene.clone()))
        .id();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let player = loop {
        app.update();
        if let bevy::asset::LoadState::Failed(error) =
            app.world().resource::<AssetServer>().load_state(scene.id())
        {
            panic!("Bow scene failed: {error}");
        }
        if let Some(instance) = app.world().get::<AnimatedSceneInstance>(object) {
            break instance.player_entity();
        }
        assert!(
            std::time::Instant::now() < deadline,
            "Bow scene load timed out"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    };
    app.world_mut()
        .get_mut::<AnimationGraphPlayer>(player)
        .unwrap()
        .set_global_input_data("draw_amount", 0.0);
    app.update();
    (app, object, player)
}
