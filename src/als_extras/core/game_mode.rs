use bevy_als::als::character::AlsInitialMovementModePending;
use bevy_als::als::control_scheme::AlsControlSchemeConfig;
use crate::input::AlsInputMap;
use bevy_als::als::params::AlsMovementParams;
use crate::player_start::{
    AlsPlayerStart, AlsPlayerStartCandidate, AlsPlayerStartOccupancy, AlsPlayerStartReady,
    AlsPlayerStartSelection, AlsPlayerStartSettings, PLAYER_START_TELEPORT_SKIN,
    choose_player_start, player_start_tag_matches, teleport_xy_adjustments,
};
use bevy_als::als::state::{AlsNotifyLocomotionAction, LocomotionState};
use bevy_als::als::transition::{Dimension, StateTransitionEvent, StateValue, locomotion_mode_locked};
use bevy_als::extensions::SceneQuery;
use avian3d::prelude::SpatialQueryFilter;
use bevy::prelude::*;

use bevy_als::character_assembly::{AlsCharacterBlueprint, spawn_als_character_with_materials};

#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlsScenePhysicsReady {
    pub ready: bool,
}

impl Default for AlsScenePhysicsReady {
    fn default() -> Self {
        Self { ready: false }
    }
}

#[derive(Component, Debug, Clone)]
pub struct AlsPlayerAdmission {
    pub input_map: AlsInputMap,
    pub spawn_input: bool,
    pub spawn_camera: bool,
    pub incoming_start_tag: Option<String>,
}

impl AlsPlayerAdmission {
    pub fn new() -> Self {
        Self {
            input_map: AlsInputMap::default(),
            spawn_input: true,
            spawn_camera: false,
            incoming_start_tag: None,
        }
    }
}

#[derive(Component, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AlsPlayerAdmissionPending;

#[derive(Component, Debug, Clone, PartialEq)]
pub struct AlsPlayerAdmissionComplete {
    pub character: Entity,
    pub selection: AlsPlayerStartSelection,
}

#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct AlsPlayerAdmissionFailed {
    pub reason: String,
}

#[derive(Resource, Debug, Clone, Copy)]
pub struct AlsGameModeRandom {
    state: u64,
}

impl Default for AlsGameModeRandom {
    fn default() -> Self {
        Self {
            state: 0xA1_5A_1C_0D_3E_4F_5A_6B,
        }
    }
}

impl AlsGameModeRandom {
    fn next_ticket(&mut self) -> u64 {
        self.state = self
            .state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.state
    }
}

pub fn request_als_player(commands: &mut Commands, admission: AlsPlayerAdmission) -> Entity {
    commands.spawn((admission, AlsPlayerAdmissionPending)).id()
}

pub struct AlsGameModePlugin;

impl Plugin for AlsGameModePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AlsGameModeRandom>()
            .init_resource::<AlsPlayerStartSettings>()
            .add_systems(
                Update,
                admit_pending_players
                    .after(crate::player_start::AlsPlayerStartPrepared)
                    .before(bevy_als::als::motion::AlsLocomotionUpdateSet),
            );
    }
}

fn spawn_capsules_overlap(a: Vec3, b: Vec3, half_height: f32, radius: f32) -> bool {
    let horizontal = Vec2::new(a.x - b.x, a.z - b.z).length_squared();
    let vertical = ((a.y - b.y).abs() - 2.0 * (half_height - radius).max(0.0)).max(0.0);
    horizontal + vertical * vertical < (2.0 * radius).powi(2)
}

fn fit_reserved_player_start(
    candidate: &AlsPlayerStartCandidate,
    reserved: &[Vec3],
    settings: &AlsPlayerStartSettings,
    spatial: &SceneQuery,
) -> Option<AlsPlayerStartCandidate> {
    let center = candidate.fit.transform.translation;
    if reserved.iter().all(|position| {
        !spawn_capsules_overlap(
            center,
            *position,
            settings.capsule_half_height,
            settings.capsule_radius,
        )
    }) {
        return Some(candidate.clone());
    }
    let shape = bevy_als::als::character::als_character_capsule(
        settings.capsule_half_height,
        settings.capsule_radius,
    );
    let filter =
        SpatialQueryFilter::default().with_mask(bevy_als::als::character::AlsCollisionLayer::Default);
    for blocker in reserved.iter().filter(|position| {
        spawn_capsules_overlap(
            center,
            **position,
            settings.capsule_half_height,
            settings.capsule_radius,
        )
    }) {
        let delta = Vec2::new(center.x - blocker.x, center.z - blocker.z);
        let shift = if delta.length_squared() > 1e-6 {
            delta.normalize()
                * (2.0 * settings.capsule_radius - delta.length() + PLAYER_START_TELEPORT_SKIN)
        } else {
            Vec2::X * (2.0 * settings.capsule_radius + PLAYER_START_TELEPORT_SKIN)
        };
        for xy in teleport_xy_adjustments(shift) {
            let mut adjusted = candidate.clone();
            let offset = Vec3::new(xy.x, 0.0, xy.y);
            adjusted.fit.transform.translation += offset;
            if reserved.iter().any(|position| {
                spawn_capsules_overlap(
                    adjusted.fit.transform.translation,
                    *position,
                    settings.capsule_half_height,
                    settings.capsule_radius,
                )
            }) || !spatial
                .shape_intersections(
                    &shape,
                    adjusted.fit.transform.translation,
                    adjusted.fit.transform.rotation,
                    &filter,
                )
                .is_empty()
            {
                continue;
            }
            adjusted.fit.teleport_adjustment += offset;
            adjusted.fit.occupancy = AlsPlayerStartOccupancy::OccupiedAdjusted;
            return Some(adjusted);
        }
    }
    None
}

#[allow(clippy::type_complexity)]
fn admit_pending_players(
    mut commands: Commands,
    barrier: Option<Res<AlsScenePhysicsReady>>,
    settings: Res<AlsPlayerStartSettings>,
    spatial: SceneQuery,
    mut random: ResMut<AlsGameModeRandom>,
    all_starts: Query<
        (
            Entity,
            Option<&crate::player_start::AlsPlayerStartProbe>,
        ),
        With<AlsPlayerStart>,
    >,
    starts: Query<(Entity, &AlsPlayerStart, &AlsPlayerStartReady)>,
    pending: Query<
        (
            Entity,
            &AlsPlayerAdmission,
            Option<&AlsPlayerAdmissionComplete>,
        ),
        With<AlsPlayerAdmissionPending>,
    >,
    mut characters: Query<
        (
            Option<&mut crate::input::AlsInputCommand>,
            Option<&mut bevy_als::als::transition::AlsStateRequest>,
            &Transform,
            Option<&mut bevy_als::als::motion::AlsViewYaw>,
            Option<&mut bevy_als::als::view::AlsViewPitch>,
            Option<&mut LocomotionState>,
            Option<&bevy_als::als::mantle::AlsMantleState>,
            Option<&AlsNotifyLocomotionAction>,
        ),
        With<bevy_als::als::character::AlsCharacter>,
    >,
    mut transitions: Option<ResMut<Messages<StateTransitionEvent>>>,
    assets: Res<AssetServer>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut params_assets: ResMut<Assets<AlsMovementParams>>,
    mut config_assets: ResMut<Assets<AlsControlSchemeConfig>>,
) {
    if barrier.as_ref().is_some_and(|barrier| !barrier.ready) {
        return;
    }
    for (request, _, previous) in &pending {
        if let Some(previous) = previous
            && characters.contains(previous.character)
        {
            if let Ok((input, request, transform, yaw, pitch, locomotion, mantle, action)) =
                characters.get_mut(previous.character)
            {
                let forward = transform.forward().as_vec3();
                if let Some(mut yaw) = yaw {
                    yaw.0 = bevy_als::als::orientation::world_forward_to_yaw(forward);
                }
                if let Some(mut pitch) = pitch {
                    pitch.0 = forward.y.atan2(Vec2::new(forward.x, forward.z).length());
                }
                if let Some(mut input) = input {
                    input.jump_pressed = false;
                    input.jump_held = false;
                    input.move_dir = Vec2::ZERO;
                }
                if let Some(mut request) = request {
                    request.locomotion_state =
                        locomotion.is_none().then_some(LocomotionState::Grounded);
                }
                if let Some(mut locomotion) = locomotion
                    && *locomotion != LocomotionState::Grounded
                    && !locomotion_mode_locked(*locomotion, mantle, action)
                {
                    if let Some(transitions) = transitions.as_deref_mut() {
                        transitions.write(StateTransitionEvent {
                            entity: previous.character,
                            dimension: Dimension::LocomotionState,
                            from: StateValue::LocomotionState(*locomotion),
                            to: StateValue::LocomotionState(LocomotionState::Grounded),
                        });
                    }
                    *locomotion = LocomotionState::Grounded;
                    commands
                        .entity(previous.character)
                        .insert(AlsInitialMovementModePending);
                }
            }
            commands
                .entity(previous.character)
                .insert(avian3d::prelude::LinearVelocity::ZERO);
            commands
                .entity(request)
                .remove::<AlsPlayerAdmissionPending>();
        }
    }
    let candidates: Vec<AlsPlayerStartCandidate> = starts
        .iter()
        .map(|(entity, start, ready)| AlsPlayerStartCandidate {
            entity,
            tag: start.tag.clone(),
            fit: crate::player_start::AlsPlayerSpawnFit {
                transform: ready.transform,
                ground_y: ready.ground_y,
                adjustment: ready.adjustment,
                teleport_adjustment: ready.teleport_adjustment,
                occupancy: ready.occupancy,
            },
        })
        .collect();
    if candidates.is_empty() {
        let preparing = all_starts
            .iter()
            .any(|(_, probe)| probe.is_none_or(|probe| probe.occupancy.is_some()));
        if !all_starts.is_empty() && !preparing {
            for (request, _, previous) in &pending {
                if previous.is_some_and(|previous| characters.contains(previous.character)) {
                    continue;
                }
                commands
                    .entity(request)
                    .remove::<AlsPlayerAdmissionPending>();
                commands.entity(request).insert(AlsPlayerAdmissionFailed {
                    reason: "No PlayerStart can fit the default pawn".into(),
                });
                info!("[GAMEMODE][DENY] request={request:?} reason=no_fit -> PASS");
            }
        }
        return;
    }
    let mut reserved = Vec::new();
    for (request, admission, previous) in &pending {
        if previous.is_some_and(|previous| characters.contains(previous.character)) {
            continue;
        }
        let available: Vec<_> = candidates
            .iter()
            .filter_map(|candidate| {
                fit_reserved_player_start(candidate, &reserved, &settings, &spatial)
            })
            .collect();
        let tag = admission
            .incoming_start_tag
            .as_deref()
            .filter(|tag| !tag.is_empty());
        let tagged_reserved = tag.is_some_and(|tag| {
            candidates
                .iter()
                .any(|candidate| player_start_tag_matches(candidate.tag.as_deref(), tag))
                && !available
                    .iter()
                    .any(|candidate| player_start_tag_matches(candidate.tag.as_deref(), tag))
        });
        let selection = (!tagged_reserved)
            .then(|| choose_player_start(&available, tag, random.next_ticket()))
            .flatten();
        let Some(selection) = selection else {
            commands
                .entity(request)
                .remove::<AlsPlayerAdmissionPending>()
                .insert(AlsPlayerAdmissionFailed {
                    reason: "No PlayerStart can fit the default pawn".into(),
                });
            continue;
        };
        reserved.push(selection.fit.transform.translation);
        let character = spawn_als_character_with_materials(
            &mut commands,
            &assets,
            &mut materials,
            &mut params_assets,
            &mut config_assets,
            AlsCharacterBlueprint {
                transform: selection.fit.transform,
                spawn_camera: admission.spawn_camera,
            },
        );
        if admission.spawn_input {
            crate::input::spawn_als_actions(&mut commands, character, &admission.input_map);
        }
        info!(
            "[GAMEMODE][ADMIT] request={request:?} start={:?} occupancy={:?} fallback={} center=({:.4},{:.4},{:.4}) yaw={:.4} -> PASS",
            selection.entity,
            selection.fit.occupancy,
            selection.used_occupied_fallback,
            selection.fit.transform.translation.x,
            selection.fit.transform.translation.y,
            selection.fit.transform.translation.z,
            selection.fit.transform.rotation.to_euler(EulerRot::YXZ).0,
        );
        commands
            .entity(request)
            .remove::<AlsPlayerAdmissionPending>();
        commands.entity(request).insert(AlsPlayerAdmissionComplete {
            character,
            selection,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use avian3d::prelude::{Collider, PhysicsPlugins, RigidBody};
    use bevy::asset::AssetPlugin;
    use bevy_animation_graph::core::animated_scene::AnimatedScene;

    #[test]
    fn admission_uses_current_world_start_transform_and_unit_pawn_scale() {
        use bevy::ecs::system::RunSystemOnce;
        for case in [
            "root",
            "translation",
            "rotation",
            "uniform",
            "nonuniform",
            "nested",
            "changed_parent",
        ] {
            for blocked in [false, true] {
                let mut app = App::new();
                app.add_plugins((
                    MinimalPlugins,
                    AssetPlugin::default(),
                    bevy::mesh::MeshPlugin,
                    bevy::transform::TransformPlugin,
                    PhysicsPlugins::new(FixedPostUpdate),
                    crate::player_start::AlsPlayerStartPlugin,
                    AlsGameModePlugin,
                ))
                .init_asset::<StandardMaterial>()
                .init_asset::<AlsMovementParams>()
                .init_asset::<AlsControlSchemeConfig>()
                .init_asset::<AnimatedScene>()
                .init_asset::<bevy_als::als_camera::AlsCameraDefinition>();
                app.world_mut()
                    .resource_mut::<AlsPlayerStartSettings>()
                    .required_stable_frames = 1;
                app.finish();
                app.cleanup();
                app.update();
                let local = Transform::from_xyz(-3.0, 2.0, 1.0)
                    .with_rotation(Quat::from_euler(EulerRot::YXZ, 0.7, 0.4, -0.2))
                    .with_scale(Vec3::new(4.0, 2.0, 0.25));
                let start = app
                    .world_mut()
                    .spawn((AlsPlayerStart::default(), local))
                    .id();
                let mut world_location = local.translation;
                let mut world_rotation = local.rotation;
                if case != "root" {
                    let mut parent_transform = Transform::from_xyz(20.0, 1.0, -8.0);
                    if case != "translation" {
                        parent_transform.rotation = Quat::from_euler(EulerRot::YXZ, -1.1, 0.2, 0.1);
                    }
                    if case == "uniform" {
                        parent_transform.scale = Vec3::splat(2.0);
                    }
                    if matches!(case, "nonuniform" | "nested" | "changed_parent") {
                        parent_transform.scale = Vec3::new(2.0, 0.75, 3.0);
                    }
                    let parent = app.world_mut().spawn(parent_transform).id();
                    app.world_mut().entity_mut(start).insert(ChildOf(parent));
                    if case == "changed_parent" {
                        app.update();
                        parent_transform.translation += Vec3::new(7.0, 0.5, 4.0);
                        parent_transform.rotation = Quat::from_euler(EulerRot::YXZ, 0.4, -0.1, 0.3);
                        *app.world_mut().get_mut::<Transform>(parent).unwrap() = parent_transform;
                    }
                    world_location = parent_transform.transform_point(local.translation);
                    world_rotation = parent_transform.rotation * local.rotation;
                    if case == "nested" {
                        let ancestor = Transform::from_xyz(-7.0, 3.0, 11.0)
                            .with_rotation(Quat::from_euler(EulerRot::YXZ, 0.5, -0.1, 0.2))
                            .with_scale(Vec3::splat(1.5));
                        let ancestor_entity = app.world_mut().spawn(ancestor).id();
                        app.world_mut()
                            .entity_mut(parent)
                            .insert(ChildOf(ancestor_entity));
                        world_location = ancestor.transform_point(world_location);
                        world_rotation = ancestor.rotation * world_rotation;
                    }
                }
                if blocked {
                    for (axis, size) in [
                        (Vec3::X, Vec3::new(2.0, 20.0, 20.0)),
                        (Vec3::Z, Vec3::new(20.0, 20.0, 2.0)),
                    ] {
                        for sign in [-1.0, 1.0] {
                            app.world_mut().spawn((
                                RigidBody::Static,
                                Collider::cuboid(size.x, size.y, size.z),
                                Transform::from_translation(world_location + axis * sign * 1.2),
                            ));
                        }
                    }
                    app.world_mut().run_schedule(PostUpdate);
                    app.world_mut().run_schedule(FixedPostUpdate);
                }
                let request = app
                    .world_mut()
                    .spawn((
                        AlsPlayerAdmission {
                            spawn_input: false,
                            spawn_camera: false,
                            ..AlsPlayerAdmission::new()
                        },
                        AlsPlayerAdmissionPending,
                    ))
                    .id();
                app.update();
                if app
                    .world()
                    .get::<AlsPlayerAdmissionFailed>(request)
                    .is_some()
                {
                    assert!(
                        blocked,
                        "World PlayerStart occupancy: case={case} location={world_location:?}"
                    );
                    assert!(
                        app.world()
                            .get::<AlsPlayerAdmissionComplete>(request)
                            .is_none()
                    );
                    continue;
                }
                let admitted = app
                    .world()
                    .get::<AlsPlayerAdmissionComplete>(request)
                    .unwrap();
                let pawn = app.world().get::<Transform>(admitted.character).unwrap();
                assert_eq!(
                    admitted.selection.fit.occupancy,
                    if blocked {
                        AlsPlayerStartOccupancy::OccupiedAdjusted
                    } else {
                        AlsPlayerStartOccupancy::Unoccupied
                    },
                    "World PlayerStart occupancy: case={case} blocked={blocked}"
                );
                let adjustment = admitted.selection.fit.teleport_adjustment;
                if blocked {
                    assert!(adjustment.length_squared() > 0.0);
                }
                let expected_yaw = world_rotation.to_euler(EulerRot::YXZ).0;
                for (consumer, actual) in [
                    (
                        "ready",
                        app.world()
                            .get::<crate::player_start::AlsPlayerStartReady>(start)
                            .unwrap()
                            .transform,
                    ),
                    ("selection", admitted.selection.fit.transform),
                    ("pawn", *pawn),
                ] {
                    assert!(
                        actual
                            .translation
                            .abs_diff_eq(world_location + adjustment, 1e-4),
                        "World PlayerStart position: case={case} consumer={consumer} actual={:?} expected={world_location:?} adjustment={adjustment:?}",
                        actual.translation
                    );
                    assert!(
                        actual
                            .rotation
                            .abs_diff_eq(Quat::from_rotation_y(expected_yaw), 1e-5),
                        "World PlayerStart yaw: case={case} consumer={consumer} actual={:?} expected={expected_yaw}",
                        actual.rotation
                    );
                    assert_eq!(
                        actual.scale,
                        Vec3::ONE,
                        "World PlayerStart unit scale: case={case} consumer={consumer}"
                    );
                }
                if blocked {
                    let pose = *pawn;
                    let pawn_entity = admitted.character;
                    let settings = *app.world().resource::<AlsPlayerStartSettings>();
                    let hits = app
                        .world_mut()
                        .run_system_once(move |spatial: SceneQuery| {
                            let shape = bevy_als::als::character::als_character_capsule(
                                settings.capsule_half_height,
                                settings.capsule_radius,
                            );
                            let filter = SpatialQueryFilter::from_excluded_entities([pawn_entity])
                                .with_mask(bevy_als::als::character::AlsCollisionLayer::Default);
                            spatial.shape_intersections(
                                &shape,
                                pose.translation,
                                pose.rotation,
                                &filter,
                            )
                        })
                        .unwrap();
                    assert!(
                        hits.is_empty(),
                        "World PlayerStart adjusted pawn is clear: case={case} pawn={pawn_entity:?} pose={pose:?} hits={hits:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn mature_scene_admission_waits_for_player_start_stabilization() {
        for stable in [2, 1, 3] {
            for case in [
                "new",
                "changed_fit",
                "prewarmed",
                "barrier_later",
                "no_barrier",
                "unfit",
                "fit_becomes_unfit",
                "no_barrier_unfit",
                "barrier_later_unfit",
            ] {
                for request_count in [1, 2] {
                    let mut app = App::new();
                    app.add_plugins((
                        MinimalPlugins,
                        AssetPlugin::default(),
                        bevy::mesh::MeshPlugin,
                        bevy::transform::TransformPlugin,
                        PhysicsPlugins::new(FixedPostUpdate),
                        crate::player_start::AlsPlayerStartPlugin,
                        AlsGameModePlugin,
                    ))
                    .init_asset::<StandardMaterial>()
                    .init_asset::<AlsMovementParams>()
                    .init_asset::<AlsControlSchemeConfig>()
                    .init_asset::<AnimatedScene>()
                    .init_asset::<bevy_als::als_camera::AlsCameraDefinition>();
                    app.world_mut()
                        .resource_mut::<AlsPlayerStartSettings>()
                        .required_stable_frames = stable;
                    app.finish();
                    app.cleanup();
                    app.update();
                    if !case.starts_with("no_barrier") {
                        app.insert_resource(AlsScenePhysicsReady {
                            ready: !case.starts_with("barrier_later"),
                        });
                    }
                    let start = app
                        .world_mut()
                        .spawn((
                            AlsPlayerStart::default(),
                            Transform::from_xyz(0.0, 0.919, 0.0),
                        ))
                        .id();
                    let prewarmed =
                        matches!(case, "changed_fit" | "prewarmed" | "fit_becomes_unfit");
                    if prewarmed {
                        for _ in 0..stable {
                            app.update();
                        }
                        assert!(app.world().get::<AlsPlayerStartReady>(start).is_some());
                    }
                    if case == "changed_fit" {
                        app.world_mut()
                            .get_mut::<Transform>(start)
                            .unwrap()
                            .translation
                            .y += 0.1;
                    }
                    let unfit = matches!(
                        case,
                        "unfit" | "fit_becomes_unfit" | "no_barrier_unfit" | "barrier_later_unfit"
                    );
                    if unfit {
                        for (axis, dimensions) in [
                            (Vec3::X, Vec3::new(2.0, 20.0, 20.0)),
                            (Vec3::Z, Vec3::new(20.0, 20.0, 2.0)),
                        ] {
                            for sign in [-1.0, 1.0] {
                                app.world_mut().spawn((
                                    RigidBody::Static,
                                    Collider::cuboid(dimensions.x, dimensions.y, dimensions.z),
                                    Transform::from_translation(
                                        Vec3::Y * 0.919 + axis * sign * 1.2,
                                    ),
                                ));
                            }
                        }
                        app.world_mut().run_schedule(FixedPostUpdate);
                    }
                    let requests: Vec<_> = (0..request_count)
                        .map(|_| {
                            app.world_mut()
                                .spawn((
                                    AlsPlayerAdmission {
                                        spawn_input: false,
                                        spawn_camera: false,
                                        ..AlsPlayerAdmission::new()
                                    },
                                    AlsPlayerAdmissionPending,
                                ))
                                .id()
                        })
                        .collect();
                    let ready_frame = if case == "prewarmed" { 1 } else { stable };
                    for frame in 1..=stable + 2 {
                        if case.starts_with("barrier_later") && frame == stable + 2 {
                            app.world_mut().resource_mut::<AlsScenePhysicsReady>().ready = true;
                        }
                        app.update();
                        let barrier_ready =
                            !case.starts_with("barrier_later") || frame == stable + 2;
                        let complete = !unfit && frame >= ready_frame && barrier_ready;
                        let failed = unfit && barrier_ready;
                        for request in &requests {
                            assert_eq!(
                                app.world()
                                    .get::<AlsPlayerAdmissionComplete>(*request)
                                    .is_some(),
                                complete,
                                "case={case} stable={stable} requests={request_count} frame={frame}"
                            );
                            assert_eq!(
                                app.world()
                                    .get::<AlsPlayerAdmissionFailed>(*request)
                                    .is_some(),
                                failed,
                                "case={case} stable={stable} requests={request_count} frame={frame}"
                            );
                            assert_eq!(
                                app.world()
                                    .get::<AlsPlayerAdmissionPending>(*request)
                                    .is_some(),
                                !complete && !failed
                            );
                            if complete {
                                let admitted = app
                                    .world()
                                    .get::<AlsPlayerAdmissionComplete>(*request)
                                    .unwrap();
                                let height = app
                                    .world()
                                    .get::<Transform>(admitted.character)
                                    .unwrap()
                                    .translation
                                    .y;
                                assert!(
                                    (height - if case == "changed_fit" { 1.019 } else { 0.919 })
                                        .abs()
                                        < 0.0001
                                );
                            }
                        }
                        if complete || failed {
                            break;
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn source_player_start_preserves_legal_indoor_and_airborne_locations() {
        for (height, roofs, snap, blocked) in [
            (0.919, vec![3.2], None, false),
            (0.919, vec![3.2, 6.2], None, false),
            (0.919, vec![], None, false),
            (1.8, vec![], None, false),
            (1.8, vec![3.2], Some(false), false),
            (1.8, vec![], Some(true), false),
            (0.919, vec![3.2], Some(true), false),
            (0.919, vec![3.2], None, true),
        ] {
            let mut app = App::new();
            app.add_plugins((
                MinimalPlugins,
                AssetPlugin::default(),
                bevy::mesh::MeshPlugin,
                bevy::transform::TransformPlugin,
                PhysicsPlugins::new(FixedPostUpdate),
                crate::player_start::AlsPlayerStartPlugin,
                AlsGameModePlugin,
            ))
            .init_asset::<StandardMaterial>()
            .init_asset::<AlsMovementParams>()
            .init_asset::<AlsControlSchemeConfig>()
            .init_asset::<AnimatedScene>()
            .init_asset::<bevy_als::als_camera::AlsCameraDefinition>();
            app.world_mut()
                .resource_mut::<AlsPlayerStartSettings>()
                .required_stable_frames = 1;
            app.finish();
            app.cleanup();
            let mut policy = AlsPlayerStart::default();
            if let Some(snap) = snap {
                policy.snap_to_ground = snap;
            }
            let start = app
                .world_mut()
                .spawn((policy, Transform::from_xyz(0.0, height, 0.0)))
                .id();
            app.world_mut().spawn((
                RigidBody::Static,
                Collider::cuboid(20.0, 0.2, 20.0),
                Transform::from_xyz(0.0, -0.1, 0.0),
            ));
            for roof in &roofs {
                app.world_mut().spawn((
                    RigidBody::Static,
                    Collider::cuboid(20.0, 0.2, 20.0),
                    Transform::from_xyz(0.0, roof - 0.1, 0.0),
                ));
            }
            if blocked {
                app.world_mut().spawn((
                    RigidBody::Static,
                    Collider::cuboid(2.0, 20.0, 20.0),
                    Transform::from_xyz(-1.2, height, 0.0),
                ));
            }
            app.update();
            app.world_mut().run_schedule(FixedPostUpdate);
            app.update();
            let expected_y = if snap == Some(true) {
                roofs.first().copied().unwrap_or(0.0) + 0.919
            } else {
                height
            };
            let expected = Vec3::new(if blocked { 0.1 } else { 0.0 }, expected_y, 0.0);
            let ready = app.world().get::<AlsPlayerStartReady>(start).unwrap();
            assert!(
                ready.transform.translation.distance(expected) < 0.0001,
                "height={height} roofs={roofs:?} snap={snap:?}: {:?} != {expected:?}",
                ready.transform.translation
            );
            let request = app
                .world_mut()
                .spawn((
                    AlsPlayerAdmission {
                        spawn_input: false,
                        spawn_camera: false,
                        ..AlsPlayerAdmission::new()
                    },
                    AlsPlayerAdmissionPending,
                ))
                .id();
            app.update();
            let admitted = app
                .world()
                .get::<AlsPlayerAdmissionComplete>(request)
                .unwrap();
            assert!(
                app.world()
                    .get::<Transform>(admitted.character)
                    .unwrap()
                    .translation
                    .distance(expected)
                    < 0.0001
            );
            assert!(
                app.world()
                    .get::<AlsPlayerAdmissionFailed>(request)
                    .is_none()
            );
        }
    }

    #[test]
    fn orthogonal_overlap_admits_at_the_combined_native_adjustment() {
        let mut cases = Vec::new();
        for depths in [Vec2::new(0.1, 0.2), Vec2::splat(0.15)] {
            for signs in [
                Vec2::ONE,
                Vec2::new(-1.0, 1.0),
                Vec2::new(1.0, -1.0),
                Vec2::NEG_ONE,
            ] {
                cases.push((depths, signs, false));
            }
        }
        cases.extend([
            (Vec2::new(0.1, 0.0), Vec2::ONE, false),
            (Vec2::new(0.0, 0.2), Vec2::NEG_ONE, false),
            (Vec2::ZERO, Vec2::ONE, false),
            (Vec2::new(0.1, 0.2), Vec2::ONE, true),
        ]);
        for (depths, signs, enclosed) in cases {
            let mut app = App::new();
            app.add_plugins((
                MinimalPlugins,
                AssetPlugin::default(),
                bevy::mesh::MeshPlugin,
                bevy::transform::TransformPlugin,
                PhysicsPlugins::new(FixedPostUpdate),
                crate::player_start::AlsPlayerStartPlugin,
                AlsGameModePlugin,
            ))
            .init_asset::<StandardMaterial>()
            .init_asset::<AlsMovementParams>()
            .init_asset::<AlsControlSchemeConfig>()
            .init_asset::<AnimatedScene>()
            .init_asset::<bevy_als::als_camera::AlsCameraDefinition>();
            app.world_mut()
                .resource_mut::<AlsPlayerStartSettings>()
                .required_stable_frames = 1;
            app.finish();
            app.cleanup();
            let center = Vec3::Y * 0.919;
            let start = app
                .world_mut()
                .spawn((
                    AlsPlayerStart {
                        snap_to_ground: false,
                        ..default()
                    },
                    Transform::from_translation(center),
                ))
                .id();
            for (depth, sign, axis, dimensions) in [
                (depths.x, signs.x, Vec3::X, Vec3::new(2.0, 20.0, 20.0)),
                (depths.y, signs.y, Vec3::Z, Vec3::new(20.0, 20.0, 2.0)),
            ] {
                if depth > 0.0 {
                    app.world_mut().spawn((
                        RigidBody::Static,
                        Collider::cuboid(dimensions.x, dimensions.y, dimensions.z),
                        Transform::from_translation(center - axis * sign * (1.3 - depth)),
                    ));
                    if enclosed {
                        app.world_mut().spawn((
                            RigidBody::Static,
                            Collider::cuboid(dimensions.x, dimensions.y, dimensions.z),
                            Transform::from_translation(center + axis * sign * (1.3 - depth)),
                        ));
                    }
                }
            }
            app.update();
            app.world_mut().run_schedule(FixedPostUpdate);
            app.update();
            let expected = Vec3::new(depths.x * signs.x, 0.0, depths.y * signs.y);
            if enclosed {
                assert!(app.world().get::<AlsPlayerStartReady>(start).is_none());
            } else {
                let ready = app
                    .world()
                    .get::<AlsPlayerStartReady>(start)
                    .expect("Combined wall penetration must publish PlayerStart readiness");
                assert!(
                    ready.teleport_adjustment.distance(expected) < 0.0001,
                    "depths={depths:?} signs={signs:?}: {:?} != {expected:?}",
                    ready.teleport_adjustment
                );
            }
            app.insert_resource(AlsScenePhysicsReady { ready: true });
            let request = app
                .world_mut()
                .spawn((
                    AlsPlayerAdmission {
                        spawn_input: false,
                        spawn_camera: false,
                        ..AlsPlayerAdmission::new()
                    },
                    AlsPlayerAdmissionPending,
                ))
                .id();
            app.update();
            assert!(
                app.world()
                    .get::<AlsPlayerAdmissionPending>(request)
                    .is_none()
            );
            if enclosed {
                assert!(
                    app.world()
                        .get::<AlsPlayerAdmissionComplete>(request)
                        .is_none()
                );
                assert!(
                    app.world()
                        .get::<AlsPlayerAdmissionFailed>(request)
                        .is_some()
                );
            } else {
                let admitted = app
                    .world()
                    .get::<AlsPlayerAdmissionComplete>(request)
                    .expect("A recoverable orthogonal overlap must admit the pawn");
                assert!(
                    app.world()
                        .get::<Transform>(admitted.character)
                        .unwrap()
                        .translation
                        .distance(center + expected)
                        < 0.0001
                );
                assert!(
                    app.world()
                        .get::<AlsPlayerAdmissionFailed>(request)
                        .is_none()
                );
            }
        }
    }

    #[test]
    fn actual_existing_pawn_restart_player_restores_actor_control_rotation() {
        let (mut app, character, avatar) = crate::test_support::loaded_character();
        AlsGameModePlugin.build(&mut app);
        app.world_mut()
            .get_mut::<crate::input::AlsInputCommand>(character)
            .unwrap()
            .look_delta = Vec2::new(40.0, -25.0);
        app.update();
        app.world_mut()
            .get_mut::<crate::input::AlsInputCommand>(character)
            .unwrap()
            .look_delta = Vec2::ZERO;
        for _ in 0..5 {
            app.update();
        }
        let component = app
            .world()
            .get::<Children>(avatar)
            .unwrap()
            .iter()
            .find(|entity| {
                app.world()
                    .get::<bevy_als::camera_component::AlsSourceCameraComponent>(*entity)
                    .is_some()
            })
            .unwrap();
        let before = app
            .world()
            .get::<bevy_als::als_camera::AlsCameraViewInfo>(component)
            .unwrap()
            .rotation;
        let actor = *app.world().get::<Transform>(character).unwrap();
        let source_target = Quat::from_rotation_y(bevy_als::als::orientation::world_forward_to_yaw(
            *actor.forward(),
        ));
        assert!(before.angle_between(source_target) > 0.2);
        let selection = crate::player_start::AlsPlayerStartSelection {
            entity: Entity::PLACEHOLDER,
            tag: None,
            fit: crate::player_start::AlsPlayerSpawnFit {
                transform: actor,
                ground_y: 0.0,
                adjustment: 0.0,
                teleport_adjustment: Vec3::ZERO,
                occupancy: AlsPlayerStartOccupancy::Unoccupied,
            },
            used_occupied_fallback: false,
        };
        app.world_mut().spawn((
            AlsPlayerAdmission::new(),
            AlsPlayerAdmissionPending,
            AlsPlayerAdmissionComplete {
                character,
                selection,
            },
        ));
        app.update();
        let info = app
            .world()
            .get::<bevy_als::als_camera::AlsCameraViewInfo>(component)
            .unwrap();
        assert!(info.valid);
        let debug = app
            .world()
            .get::<bevy_als::als_camera::AlsCameraDebugInfo>(component)
            .unwrap();
        let lag = debug.curve_value("RotationLag");
        let alpha = bevy_als::extensions::control_rig_math::rig_damper_alpha(
            debug.unscaled_delta_seconds,
            lag,
        );
        let (yaw, pitch, roll) = before.to_euler(EulerRot::YXZ);
        let target_yaw = source_target.to_euler(EulerRot::YXZ).0;
        let expected = Quat::from_euler(
            EulerRot::YXZ,
            yaw + bevy_als::als::roll::wrap_angle(target_yaw - yaw) * alpha,
            pitch * (1.0 - alpha),
            roll * (1.0 - alpha),
        );
        assert!(
            info.rotation.angle_between(expected) < 1e-4,
            "Source RestartPlayer sets actor control rotation before actual camera lag: expected={expected:?}, actual={:?}",
            info.rotation
        );
    }

    #[test]
    fn actual_existing_pawn_restart_preserves_desired_crouch_and_allows_new_jump() {
        for crouched in [false, true] {
            let (mut app, character, _) = crate::test_support::loaded_character();
            AlsGameModePlugin.build(&mut app);
            let desired = if crouched {
                bevy_als::als::state::Stance::Crouching
            } else {
                bevy_als::als::state::Stance::Standing
            };
            app.world_mut()
                .get_mut::<bevy_als::als::transition::AlsStateRequest>(character)
                .unwrap()
                .stance = Some(desired);
            for _ in 0..3 {
                app.update();
            }
            assert_eq!(
                app.world().get::<bevy_als::als::state::Stance>(character),
                Some(&desired)
            );
            app.world_mut()
                .get_mut::<crate::input::AlsInputCommand>(character)
                .unwrap()
                .jump_pressed = true;
            let selection = crate::player_start::AlsPlayerStartSelection {
                entity: Entity::PLACEHOLDER,
                tag: None,
                fit: crate::player_start::AlsPlayerSpawnFit {
                    transform: *app.world().get::<Transform>(character).unwrap(),
                    ground_y: 0.0,
                    adjustment: 0.0,
                    teleport_adjustment: Vec3::ZERO,
                    occupancy: AlsPlayerStartOccupancy::Unoccupied,
                },
                used_occupied_fallback: false,
            };
            let request = app
                .world_mut()
                .spawn((
                    AlsPlayerAdmission::new(),
                    AlsPlayerAdmissionPending,
                    AlsPlayerAdmissionComplete {
                        character,
                        selection,
                    },
                ))
                .id();
            app.update();
            assert!(
                app.world()
                    .get::<AlsPlayerAdmissionPending>(request)
                    .is_none()
            );
            assert_eq!(
                app.world().get::<bevy_als::als::state::Stance>(character),
                Some(&desired)
            );
            assert_eq!(
                app.world()
                    .get::<bevy_als::als::transition::AlsDesiredStance>(character)
                    .unwrap()
                    .0,
                desired
            );
            assert_eq!(
                app.world()
                    .get::<bevy_als::als::state::LocomotionState>(character),
                Some(&bevy_als::als::state::LocomotionState::Grounded)
            );
            assert!(
                app.world()
                    .get::<avian3d::prelude::LinearVelocity>(character)
                    .unwrap()
                    .y
                    .abs()
                    < 0.02
            );
            app.update();
            app.world_mut()
                .get_mut::<crate::input::AlsInputCommand>(character)
                .unwrap()
                .jump_pressed = true;
            app.update();
            if crouched {
                assert_eq!(
                    app.world().get::<bevy_als::als::state::Stance>(character),
                    Some(&bevy_als::als::state::Stance::Standing)
                );
                assert_eq!(
                    app.world()
                        .get::<bevy_als::als::state::LocomotionState>(character),
                    Some(&bevy_als::als::state::LocomotionState::Grounded)
                );
                app.world_mut()
                    .get_mut::<crate::input::AlsInputCommand>(character)
                    .unwrap()
                    .jump_pressed = false;
                app.update();
                app.world_mut()
                    .get_mut::<crate::input::AlsInputCommand>(character)
                    .unwrap()
                    .jump_pressed = true;
                app.update();
            }
            assert_eq!(
                app.world()
                    .get::<bevy_als::als::state::LocomotionState>(character),
                Some(&bevy_als::als::state::LocomotionState::Air)
            );
            assert!(
                app.world()
                    .get::<avian3d::prelude::LinearVelocity>(character)
                    .unwrap()
                    .y
                    > 3.0,
                "Source fresh Jump after Restart must remain usable: crouched={crouched}"
            );
        }
    }

    #[test]
    fn actual_existing_pawn_restart_consumes_pending_move_before_new_device_input() {
        use bevy::input::gamepad::{RawGamepadAxisChangedEvent, RawGamepadEvent};
        for gamepad_input in [false, true] {
            let (mut app, character, _) = crate::test_support::loaded_character();
            AlsGameModePlugin.build(&mut app);
            let gamepad = app.world_mut().spawn(Gamepad::default()).id();
            crate::input::spawn_als_actions(
                &mut app.world_mut().commands(),
                character,
                &AlsInputMap::default(),
            );
            app.update();
            if gamepad_input {
                app.world_mut().write_message(RawGamepadEvent::Axis(
                    RawGamepadAxisChangedEvent::new(gamepad, GamepadAxis::LeftStickY, 1.0),
                ));
            } else {
                app.world_mut()
                    .resource_mut::<ButtonInput<KeyCode>>()
                    .press(KeyCode::KeyW);
            }
            app.update();
            assert!(
                app.world()
                    .get::<bevy_als::actual_motion::AlsActualMotion>(character)
                    .unwrap()
                    .has_input
            );
            assert!(
                app.world()
                    .get::<avian3d::prelude::LinearVelocity>(character)
                    .unwrap()
                    .0
                    .with_y(0.0)
                    .length()
                    > 0.01
            );
            let selection = crate::player_start::AlsPlayerStartSelection {
                entity: Entity::PLACEHOLDER,
                tag: None,
                fit: crate::player_start::AlsPlayerSpawnFit {
                    transform: *app.world().get::<Transform>(character).unwrap(),
                    ground_y: 0.0,
                    adjustment: 0.0,
                    teleport_adjustment: Vec3::ZERO,
                    occupancy: AlsPlayerStartOccupancy::Unoccupied,
                },
                used_occupied_fallback: false,
            };
            let action_count = app.world_mut().query_filtered::<Entity, With<bevy_enhanced_input::prelude::Action<crate::input::MoveAction>>>().iter(app.world()).count();
            let request = app
                .world_mut()
                .spawn((
                    AlsPlayerAdmission::new(),
                    AlsPlayerAdmissionPending,
                    AlsPlayerAdmissionComplete {
                        character,
                        selection,
                    },
                ))
                .id();
            app.update();
            assert!(
                app.world()
                    .get::<AlsPlayerAdmissionPending>(request)
                    .is_none()
            );
            assert_eq!(app.world_mut().query_filtered::<Entity, With<bevy_enhanced_input::prelude::Action<crate::input::MoveAction>>>().iter(app.world()).count(), action_count,
                "Source existing input component is reused without adding actions");
            let actual = app
                .world()
                .get::<avian3d::prelude::LinearVelocity>(character)
                .unwrap()
                .0
                .with_y(0.0);
            assert!(
                actual.length() < 1e-4,
                "Source Restart consumes pending movement before the current motor tick: gamepad={gamepad_input}, actual={actual:?}"
            );
            app.update();
            let next = app
                .world()
                .get::<avian3d::prelude::LinearVelocity>(character)
                .unwrap()
                .0
                .with_y(0.0);
            assert!(
                next.length() > 0.01,
                "Source held device must provide new movement after Restart consumed its pending input: gamepad={gamepad_input}, actual={next:?}"
            );
        }
    }

    #[test]
    fn actual_existing_pawn_restart_preserves_source_movement_owner_locks() {
        for ragdolling in [false, true] {
            let (mut app, character, _) = crate::test_support::loaded_character();
            AlsGameModePlugin.build(&mut app);
            if ragdolling {
                app.world_mut()
                    .get_mut::<bevy_als::als::transition::AlsStateRequest>(character)
                    .unwrap()
                    .ragdoll_start = true;
            } else {
                let start = app.world().get::<Transform>(character).unwrap().translation;
                app.world_mut()
                    .get_mut::<bevy_als::als::mantle::AlsMantleState>(character)
                    .unwrap()
                    .begin(
                        bevy_als::als::mantle::MantleKind::Low,
                        start,
                        start + Vec3::Y + Vec3::NEG_Z,
                        0.75,
                        0.0,
                        0.0,
                    );
            }
            for _ in 0..10 {
                app.update();
            }
            let expected_mode = if ragdolling {
                bevy_als::als::state::LocomotionState::None
            } else {
                bevy_als::als::state::LocomotionState::Custom
            };
            let expected_action = if ragdolling {
                bevy_als::als::state::AlsNotifyLocomotionAction::Ragdolling
            } else {
                bevy_als::als::state::AlsNotifyLocomotionAction::Mantling
            };
            assert_eq!(
                app.world()
                    .get::<bevy_als::als::state::LocomotionState>(character),
                Some(&expected_mode)
            );
            let selection = crate::player_start::AlsPlayerStartSelection {
                entity: Entity::PLACEHOLDER,
                tag: None,
                fit: crate::player_start::AlsPlayerSpawnFit {
                    transform: *app.world().get::<Transform>(character).unwrap(),
                    ground_y: 0.0,
                    adjustment: 0.0,
                    teleport_adjustment: Vec3::ZERO,
                    occupancy: AlsPlayerStartOccupancy::Unoccupied,
                },
                used_occupied_fallback: false,
            };
            let request = app
                .world_mut()
                .spawn((
                    AlsPlayerAdmission::new(),
                    AlsPlayerAdmissionPending,
                    AlsPlayerAdmissionComplete {
                        character,
                        selection,
                    },
                ))
                .id();
            app.update();
            assert!(
                app.world()
                    .get::<AlsPlayerAdmissionPending>(request)
                    .is_none()
            );
            assert_eq!(
                app.world()
                    .get::<bevy_als::als::state::LocomotionState>(character),
                Some(&expected_mode)
            );
            assert_eq!(
                app.world()
                    .get::<bevy_als::als::state::AlsNotifyLocomotionAction>(character),
                Some(&expected_action)
            );
            if !ragdolling {
                let mantle = app
                    .world()
                    .get::<bevy_als::als::mantle::AlsMantleState>(character)
                    .unwrap();
                assert!(mantle.active && mantle.root_motion_ready);
            }
        }
    }

    #[test]
    fn actual_existing_pawn_restart_restores_source_default_movement_mode() {
        for height in [0.0, 2.0] {
            let (mut app, character, _) = crate::test_support::loaded_character();
            AlsGameModePlugin.build(&mut app);
            app.world_mut()
                .get_mut::<bevy_als::als::transition::AlsStateRequest>(character)
                .unwrap()
                .locomotion_state = Some(bevy_als::als::state::LocomotionState::Custom);
            app.update();
            let mut transform = *app.world().get::<Transform>(character).unwrap();
            transform.translation.y += height;
            app.world_mut().entity_mut(character).insert((
                avian3d::prelude::Position(transform.translation),
                transform,
                GlobalTransform::from(transform),
            ));
            app.update();
            assert_eq!(
                app.world()
                    .get::<bevy_als::als::state::LocomotionState>(character),
                Some(&bevy_als::als::state::LocomotionState::Custom)
            );
            assert_eq!(
                app.world()
                    .get::<avian3d::prelude::LinearVelocity>(character)
                    .unwrap()
                    .0,
                Vec3::ZERO
            );
            let selection = crate::player_start::AlsPlayerStartSelection {
                entity: Entity::PLACEHOLDER,
                tag: None,
                fit: crate::player_start::AlsPlayerSpawnFit {
                    transform,
                    ground_y: 0.0,
                    adjustment: 0.0,
                    teleport_adjustment: Vec3::ZERO,
                    occupancy: AlsPlayerStartOccupancy::Unoccupied,
                },
                used_occupied_fallback: false,
            };
            app.world_mut().spawn((
                AlsPlayerAdmission::new(),
                AlsPlayerAdmissionPending,
                AlsPlayerAdmissionComplete {
                    character,
                    selection,
                },
            ));
            app.update();
            let actual = app
                .world()
                .get::<avian3d::prelude::LinearVelocity>(character)
                .unwrap()
                .y;
            let mode = if height == 0.0 {
                bevy_als::als::state::LocomotionState::Grounded
            } else {
                bevy_als::als::state::LocomotionState::Air
            };
            let expected = if height == 0.0 { 0.0 } else { -9.81 / 60.0 };
            assert!(
                (actual - expected).abs() < 0.02,
                "Source Restart restores Walking with a supporting floor or Falling without one: height={height}, actual={actual}, mode={:?}",
                app.world()
                    .get::<bevy_als::als::state::LocomotionState>(character)
            );
            assert_eq!(
                app.world()
                    .get::<bevy_als::als::state::LocomotionState>(character),
                Some(&mode)
            );
        }
    }

    #[test]
    fn actual_existing_pawn_restart_clears_source_jump_force() {
        let (mut app, character, _) = crate::test_support::loaded_character();
        AlsGameModePlugin.build(&mut app);
        let handle = app
            .world()
            .get::<bevy_als::als::character::AlsMovementParamsHandle>(character)
            .unwrap()
            .0
            .clone();
        app.world_mut()
            .resource_mut::<Assets<AlsMovementParams>>()
            .get_mut(&handle)
            .unwrap()
            .jump
            .max_hold_time = 0.2;
        app.world_mut()
            .get_mut::<crate::input::AlsInputCommand>(character)
            .unwrap()
            .jump_pressed = true;
        app.world_mut()
            .get_mut::<crate::input::AlsInputCommand>(character)
            .unwrap()
            .jump_held = true;
        app.update();
        app.world_mut()
            .get_mut::<crate::input::AlsInputCommand>(character)
            .unwrap()
            .jump_pressed = false;
        app.update();
        assert_eq!(
            app.world()
                .get::<bevy_als::als::state::LocomotionState>(character),
            Some(&bevy_als::als::state::LocomotionState::Air)
        );
        assert!(
            app.world()
                .get::<avian3d::prelude::LinearVelocity>(character)
                .unwrap()
                .y
                > 3.0
        );
        let mut unsupported = *app.world().get::<Transform>(character).unwrap();
        unsupported.translation.y += 2.0;
        app.world_mut().entity_mut(character).insert((
            avian3d::prelude::Position(unsupported.translation),
            unsupported,
            GlobalTransform::from(unsupported),
        ));
        let selection = crate::player_start::AlsPlayerStartSelection {
            entity: Entity::PLACEHOLDER,
            tag: None,
            fit: crate::player_start::AlsPlayerSpawnFit {
                transform: *app.world().get::<Transform>(character).unwrap(),
                ground_y: 0.0,
                adjustment: 0.0,
                teleport_adjustment: Vec3::ZERO,
                occupancy: AlsPlayerStartOccupancy::Unoccupied,
            },
            used_occupied_fallback: false,
        };
        let request = app
            .world_mut()
            .spawn((
                AlsPlayerAdmission::new(),
                AlsPlayerAdmissionPending,
                AlsPlayerAdmissionComplete {
                    character,
                    selection,
                },
            ))
            .id();
        app.update();
        assert!(
            app.world()
                .get::<AlsPlayerAdmissionPending>(request)
                .is_none()
        );
        let actual = app
            .world()
            .get::<avian3d::prelude::LinearVelocity>(character)
            .unwrap()
            .y;
        assert!(
            (actual + 9.81 / 60.0).abs() < 0.02,
            "Source Restart stops actual movement and clears active Jump force before the next physics tick: {actual}"
        );
        app.update();
        let after = app
            .world()
            .get::<avian3d::prelude::LinearVelocity>(character)
            .unwrap()
            .y;
        assert!(
            after < actual - 0.1,
            "Source reset Jump cannot resume its old held force without a new press: before={actual}, after={after}"
        );
    }

    #[test]
    fn repeated_player_admission_restarts_the_existing_source_pawn() {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            bevy::mesh::MeshPlugin,
            bevy::transform::TransformPlugin,
            PhysicsPlugins::new(FixedPostUpdate),
            AlsGameModePlugin,
        ))
        .init_asset::<StandardMaterial>()
        .init_asset::<AlsMovementParams>()
        .init_asset::<AlsControlSchemeConfig>()
        .init_asset::<AnimatedScene>()
        .init_asset::<bevy_als::als_camera::AlsCameraDefinition>();
        app.finish();
        app.cleanup();
        app.world_mut().spawn((
            AlsPlayerStart::default(),
            AlsPlayerStartReady {
                transform: Transform::from_xyz(0.0, 0.919, 0.0),
                ground_y: 0.0,
                adjustment: 0.0,
                teleport_adjustment: Vec3::ZERO,
                occupancy: AlsPlayerStartOccupancy::Unoccupied,
            },
        ));
        let request = app
            .world_mut()
            .spawn((
                AlsPlayerAdmission {
                    spawn_input: false,
                    spawn_camera: false,
                    ..AlsPlayerAdmission::new()
                },
                AlsPlayerAdmissionPending,
            ))
            .id();
        app.update();
        let character = app
            .world()
            .get::<AlsPlayerAdmissionComplete>(request)
            .unwrap()
            .character;
        let assembly = *app
            .world()
            .get::<bevy_als::character_assembly::AlsCharacterAssembly>(character)
            .unwrap();
        let retained =
            Transform::from_xyz(4.0, 0.919, 2.0).with_rotation(Quat::from_rotation_y(0.7));
        app.world_mut().entity_mut(character).insert((
            retained,
            avian3d::prelude::LinearVelocity(Vec3::X * 3.75),
            bevy_als::als::transition::AlsDesiredStance(bevy_als::als::state::Stance::Crouching),
            bevy_als::als::gait::AlsDesiredGait(bevy_als::als::state::Gait::Walking),
        ));
        app.world_mut()
            .entity_mut(request)
            .insert(AlsPlayerAdmissionPending);
        app.update();
        let restarted = app
            .world()
            .get::<AlsPlayerAdmissionComplete>(request)
            .unwrap()
            .character;
        assert_eq!(
            restarted, character,
            "Source RestartPlayer reuses its existing pawn"
        );
        assert_eq!(
            *app.world()
                .get::<bevy_als::character_assembly::AlsCharacterAssembly>(character)
                .unwrap(),
            assembly
        );
        assert_eq!(
            app.world_mut()
                .query_filtered::<Entity, With<bevy_als::als::character::AlsCharacter>>()
                .iter(app.world())
                .count(),
            1
        );
        assert_eq!(
            app.world()
                .get::<avian3d::prelude::LinearVelocity>(character)
                .unwrap()
                .0,
            Vec3::ZERO
        );
        assert_eq!(app.world().get::<Transform>(character), Some(&retained));
        assert_eq!(
            app.world()
                .get::<bevy_als::als::transition::AlsDesiredStance>(character)
                .unwrap()
                .0,
            bevy_als::als::state::Stance::Crouching
        );
        assert_eq!(
            app.world()
                .get::<bevy_als::als::gait::AlsDesiredGait>(character)
                .unwrap()
                .0,
            bevy_als::als::state::Gait::Walking
        );
        let starts: Vec<_> = app
            .world_mut()
            .query_filtered::<Entity, With<AlsPlayerStart>>()
            .iter(app.world())
            .collect();
        for start in starts {
            app.world_mut().despawn(start);
        }
        app.world_mut()
            .entity_mut(character)
            .insert(avian3d::prelude::LinearVelocity(Vec3::X));
        app.world_mut()
            .entity_mut(request)
            .insert(AlsPlayerAdmissionPending);
        app.update();
        assert!(
            app.world()
                .get::<AlsPlayerAdmissionPending>(request)
                .is_none()
        );
        assert_eq!(
            app.world()
                .get::<AlsPlayerAdmissionComplete>(request)
                .unwrap()
                .character,
            character
        );
        assert_eq!(
            app.world()
                .get::<avian3d::prelude::LinearVelocity>(character)
                .unwrap()
                .0,
            Vec3::ZERO
        );
        app.world_mut().despawn(character);
        app.world_mut().spawn((
            AlsPlayerStart::default(),
            AlsPlayerStartReady {
                transform: retained,
                ground_y: 0.0,
                adjustment: 0.0,
                teleport_adjustment: Vec3::ZERO,
                occupancy: AlsPlayerStartOccupancy::Unoccupied,
            },
        ));
        app.world_mut()
            .entity_mut(request)
            .insert(AlsPlayerAdmissionPending);
        app.update();
        let replacement = app
            .world()
            .get::<AlsPlayerAdmissionComplete>(request)
            .unwrap()
            .character;
        assert_ne!(
            replacement, character,
            "Source missing existing Pawn must use the original new Pawn branch"
        );
        assert!(
            app.world()
                .get::<bevy_als::als::character::AlsCharacter>(replacement)
                .is_some()
        );
        assert!(
            app.world()
                .get::<AlsPlayerAdmissionPending>(request)
                .is_none()
        );
    }

    #[test]
    fn reserved_named_player_start_uses_the_same_source_fname_identity() {
        for request_tag in [Some("Portal"), Some("pOrTaL"), None, Some("Missing")] {
            let mut app = App::new();
            app.add_plugins((
                MinimalPlugins,
                AssetPlugin::default(),
                bevy::mesh::MeshPlugin,
                bevy::transform::TransformPlugin,
                PhysicsPlugins::new(FixedPostUpdate),
                AlsGameModePlugin,
            ))
            .init_asset::<StandardMaterial>()
            .init_asset::<AlsMovementParams>()
            .init_asset::<AlsControlSchemeConfig>()
            .init_asset::<AnimatedScene>()
            .init_asset::<bevy_als::als_camera::AlsCameraDefinition>();
            app.finish();
            app.cleanup();
            let starts = [(Vec3::ZERO, "Portal"), (Vec3::X * 3.0, "Other")].map(|(offset, tag)| {
                app.world_mut()
                    .spawn((
                        AlsPlayerStart {
                            tag: Some(tag.into()),
                            ..default()
                        },
                        AlsPlayerStartReady {
                            transform: Transform::from_translation(offset + Vec3::Y * 0.919),
                            ground_y: 0.0,
                            adjustment: 0.0,
                            teleport_adjustment: Vec3::ZERO,
                            occupancy: AlsPlayerStartOccupancy::Unoccupied,
                        },
                    ))
                    .id()
            });
            for offset in teleport_xy_adjustments(Vec2::X * (0.6 + PLAYER_START_TELEPORT_SKIN)) {
                app.world_mut().spawn((
                    RigidBody::Static,
                    Collider::cuboid(0.1, 1.8, 0.1),
                    Transform::from_xyz(offset.x, 0.919, offset.y),
                ));
            }
            app.update();
            app.world_mut().run_schedule(FixedPostUpdate);
            app.update();
            let first = app
                .world_mut()
                .spawn((
                    AlsPlayerAdmission {
                        spawn_input: false,
                        spawn_camera: false,
                        incoming_start_tag: Some("Portal".into()),
                        ..AlsPlayerAdmission::new()
                    },
                    AlsPlayerAdmissionPending,
                ))
                .id();
            let second = app
                .world_mut()
                .spawn((
                    AlsPlayerAdmission {
                        spawn_input: false,
                        spawn_camera: false,
                        incoming_start_tag: request_tag.map(str::to_owned),
                        ..AlsPlayerAdmission::new()
                    },
                    AlsPlayerAdmissionPending,
                ))
                .id();
            app.update();
            assert_eq!(
                app.world()
                    .get::<AlsPlayerAdmissionComplete>(first)
                    .unwrap()
                    .selection
                    .entity,
                starts[0]
            );
            if matches!(request_tag, Some("Portal" | "pOrTaL")) {
                assert!(
                    app.world()
                        .get::<AlsPlayerAdmissionFailed>(second)
                        .is_some()
                        && app
                            .world()
                            .get::<AlsPlayerAdmissionComplete>(second)
                            .is_none(),
                    "Reserved PlayerStart FName identity: request={request_tag:?}"
                );
            } else {
                assert_eq!(
                    app.world()
                        .get::<AlsPlayerAdmissionComplete>(second)
                        .unwrap()
                        .selection
                        .entity,
                    starts[1]
                );
            }
        }
    }

    #[test]
    fn same_frame_admissions_do_not_share_one_capsule_space() {
        assert!(spawn_capsules_overlap(Vec3::ZERO, Vec3::ZERO, 0.9, 0.3));
        assert!(!spawn_capsules_overlap(
            Vec3::ZERO,
            Vec3::X * 0.61,
            0.9,
            0.3
        ));
        assert!(!spawn_capsules_overlap(
            Vec3::ZERO,
            Vec3::Y * 1.81,
            0.9,
            0.3
        ));
        for (starts, incoming_tag, blocked) in [
            (vec![(Vec3::ZERO, None)], None, false),
            (
                vec![(Vec3::ZERO, Some("Portal")), (Vec3::X * 3.0, Some("Other"))],
                Some("Portal"),
                false,
            ),
            (vec![(Vec3::ZERO, None), (Vec3::X * 3.0, None)], None, false),
            (vec![(Vec3::ZERO, None)], None, true),
        ] {
            let mut app = App::new();
            app.add_plugins((
                MinimalPlugins,
                AssetPlugin::default(),
                bevy::mesh::MeshPlugin,
                bevy::transform::TransformPlugin,
                PhysicsPlugins::new(FixedPostUpdate),
                AlsGameModePlugin,
            ))
            .init_asset::<StandardMaterial>()
            .init_asset::<AlsMovementParams>()
            .init_asset::<AlsControlSchemeConfig>()
            .init_asset::<AnimatedScene>()
            .init_asset::<bevy_als::als_camera::AlsCameraDefinition>();
            app.finish();
            app.cleanup();
            let mut starts_spawned = Vec::new();
            for (offset, tag) in starts {
                starts_spawned.push(
                    app.world_mut()
                        .spawn((
                            AlsPlayerStart {
                                tag: tag.map(str::to_owned),
                                ..default()
                            },
                            AlsPlayerStartReady {
                                transform: Transform::from_translation(offset + Vec3::Y * 0.919),
                                ground_y: 0.0,
                                adjustment: 0.0,
                                teleport_adjustment: Vec3::ZERO,
                                occupancy: AlsPlayerStartOccupancy::Unoccupied,
                            },
                        ))
                        .id(),
                );
            }
            if blocked {
                for offset in teleport_xy_adjustments(Vec2::X * (0.6 + PLAYER_START_TELEPORT_SKIN))
                {
                    app.world_mut().spawn((
                        RigidBody::Static,
                        Collider::cuboid(0.1, 1.8, 0.1),
                        Transform::from_xyz(offset.x, 0.919, offset.y),
                    ));
                }
                app.update();
                app.world_mut().run_schedule(FixedPostUpdate);
                app.update();
            }
            let admission = AlsPlayerAdmission {
                spawn_input: false,
                incoming_start_tag: incoming_tag.map(str::to_owned),
                ..AlsPlayerAdmission::new()
            };
            let first = app
                .world_mut()
                .spawn((admission.clone(), AlsPlayerAdmissionPending))
                .id();
            let second = app
                .world_mut()
                .spawn((admission, AlsPlayerAdmissionPending))
                .id();
            app.update();
            let first_spawn = app
                .world()
                .get::<AlsPlayerAdmissionComplete>(first)
                .unwrap();
            if blocked {
                assert!(
                    app.world()
                        .get::<AlsPlayerAdmissionComplete>(second)
                        .is_none()
                );
                assert!(
                    app.world()
                        .get::<AlsPlayerAdmissionFailed>(second)
                        .is_some()
                );
                continue;
            }
            let second_spawn = app
                .world()
                .get::<AlsPlayerAdmissionComplete>(second)
                .unwrap();
            let first_position = app
                .world()
                .get::<Transform>(first_spawn.character)
                .unwrap()
                .translation;
            let second_position = app
                .world()
                .get::<Transform>(second_spawn.character)
                .unwrap()
                .translation;
            assert!(first_position.distance(second_position) >= 0.6);
            if starts_spawned.len() == 1 || incoming_tag.is_some() {
                assert_eq!(first_spawn.selection.entity, starts_spawned[0]);
                assert_eq!(second_spawn.selection.entity, starts_spawned[0]);
                assert_eq!(
                    second_spawn.selection.fit.occupancy,
                    AlsPlayerStartOccupancy::OccupiedAdjusted
                );
            } else {
                assert_ne!(first_spawn.selection.entity, second_spawn.selection.entity);
            }
        }
    }
}
