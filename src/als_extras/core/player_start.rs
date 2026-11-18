use bevy_als::extensions::SceneQuery;
use avian3d::prelude::{Collider, SpatialQueryFilter};
use bevy::prelude::*;

const SPAWN_ENCROACH_EPSILON: f32 = 0.0015;
pub const ALS_PLAYER_START_SOURCE_CLEARANCE: f32 = 0.019;
pub const PLAYER_START_VERTICAL_ADJUSTMENTS: [f32; 8] =
    [0.0, 0.005, 0.01, 0.02, 0.04, 0.08, 0.16, 0.32];
pub const PLAYER_START_TELEPORT_SKIN: f32 = 0.001;

#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct AlsPlayerStart {
    pub tag: Option<String>,
    pub snap_to_ground: bool,
}

impl Default for AlsPlayerStart {
    fn default() -> Self {
        Self {
            tag: None,
            snap_to_ground: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlsPlayerStartOccupancy {
    Unoccupied,
    OccupiedAdjusted,
}

#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct AlsPlayerStartReady {
    pub transform: Transform,
    pub ground_y: f32,
    pub adjustment: f32,
    pub teleport_adjustment: Vec3,
    pub occupancy: AlsPlayerStartOccupancy,
}

#[derive(Component, Debug, Default, Clone, Copy)]
pub(crate) struct AlsPlayerStartProbe {
    ground_y: f32,
    adjustment: f32,
    teleport_adjustment: Vec3,
    pub(crate) occupancy: Option<AlsPlayerStartOccupancy>,
    stable_frames: u32,
}

#[derive(Resource, Debug, Clone, Copy)]
pub struct AlsPlayerStartSettings {
    pub capsule_half_height: f32,
    pub capsule_radius: f32,
    pub probe_height: f32,
    pub probe_distance: f32,
    pub required_stable_frames: u32,
}

impl Default for AlsPlayerStartSettings {
    fn default() -> Self {
        Self {
            capsule_half_height: bevy_als::als::character::ALS_CHARACTER_CAPSULE_HALF_HEIGHT,
            capsule_radius: bevy_als::als::character::ALS_CHARACTER_CAPSULE_RADIUS,
            probe_height: 5.0,
            probe_distance: 10.0,
            required_stable_frames: 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AlsPlayerSpawnFit {
    pub transform: Transform,
    pub ground_y: f32,
    pub adjustment: f32,
    pub teleport_adjustment: Vec3,
    pub occupancy: AlsPlayerStartOccupancy,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AlsPlayerStartCandidate {
    pub entity: Entity,
    pub tag: Option<String>,
    pub fit: AlsPlayerSpawnFit,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AlsPlayerStartSelection {
    pub entity: Entity,
    pub tag: Option<String>,
    pub fit: AlsPlayerSpawnFit,
    pub used_occupied_fallback: bool,
}

pub fn player_start_grounded_height(ground_y: f32, capsule_half_height: f32) -> f32 {
    ground_y + capsule_half_height + ALS_PLAYER_START_SOURCE_CLEARANCE
}

pub fn yaw_only_transform(mut transform: Transform) -> Transform {
    let (yaw, _, _) = transform.rotation.to_euler(EulerRot::YXZ);
    transform.rotation = Quat::from_rotation_y(yaw);
    transform.scale = Vec3::ONE;
    transform
}

pub(crate) fn player_start_tag_matches(tag: Option<&str>, incoming: &str) -> bool {
    let is_none = |value: &str| {
        value.is_empty()
            || value.eq_ignore_ascii_case("None")
            || value.strip_prefix('_').is_some_and(|digits| {
                digits
                    .parse::<u32>()
                    .ok()
                    .filter(|number| *number < i32::MAX as u32)
                    .is_some_and(|number| number.to_string() == digits)
            })
    };
    let tag = tag.unwrap_or_default();
    tag.eq_ignore_ascii_case(incoming) || (is_none(tag) && is_none(incoming))
}

pub fn choose_player_start(
    candidates: &[AlsPlayerStartCandidate],
    incoming_tag: Option<&str>,
    random_ticket: u64,
) -> Option<AlsPlayerStartSelection> {
    if let Some(tag) = incoming_tag.filter(|tag| !tag.is_empty()) {
        if let Some(candidate) = candidates
            .iter()
            .find(|candidate| player_start_tag_matches(candidate.tag.as_deref(), tag))
        {
            return Some(AlsPlayerStartSelection {
                entity: candidate.entity,
                tag: candidate.tag.clone(),
                fit: candidate.fit,
                used_occupied_fallback: candidate.fit.occupancy
                    == AlsPlayerStartOccupancy::OccupiedAdjusted,
            });
        }
    }
    let unoccupied: Vec<&AlsPlayerStartCandidate> = candidates
        .iter()
        .filter(|candidate| candidate.fit.occupancy == AlsPlayerStartOccupancy::Unoccupied)
        .collect();
    let occupied: Vec<&AlsPlayerStartCandidate> = candidates
        .iter()
        .filter(|candidate| candidate.fit.occupancy == AlsPlayerStartOccupancy::OccupiedAdjusted)
        .collect();
    let (group, used_occupied_fallback) = if !unoccupied.is_empty() {
        (unoccupied, false)
    } else if !occupied.is_empty() {
        (occupied, true)
    } else {
        return None;
    };
    let candidate = group[(random_ticket as usize) % group.len()];
    Some(AlsPlayerStartSelection {
        entity: candidate.entity,
        tag: candidate.tag.clone(),
        fit: candidate.fit,
        used_occupied_fallback,
    })
}

pub fn teleport_xy_adjustments(adjustment: Vec2) -> [Vec2; 8] {
    let x = adjustment.x;
    let z = adjustment.y;
    if x.abs() > f32::EPSILON && z.abs() > f32::EPSILON {
        [
            Vec2::new(x, z),
            Vec2::new(-x, z),
            Vec2::new(x, -z),
            Vec2::new(-x, -z),
            Vec2::new(z, x),
            Vec2::new(-z, x),
            Vec2::new(z, -x),
            Vec2::new(-z, -x),
        ]
    } else {
        let dominant = if z.abs() <= f32::EPSILON { x } else { z };
        [
            Vec2::new(x, z),
            Vec2::new(-x, -z),
            Vec2::new(z, x),
            Vec2::new(-z, -x),
            Vec2::new(dominant, dominant),
            Vec2::new(dominant, -dominant),
            Vec2::new(-dominant, dominant),
            Vec2::new(-dominant, -dominant),
        ]
    }
}

fn spawn_shape(half_height: f32, radius: f32) -> avian3d::prelude::Collider {
    bevy_als::als::character::als_character_capsule(half_height, radius)
}

fn shape_intersections(
    spatial: &SceneQuery,
    shape: &avian3d::prelude::Collider,
    transform: &Transform,
    colliders: &Query<(&Collider, &GlobalTransform)>,
    filter: &SpatialQueryFilter,
) -> Vec<Entity> {
    spatial
        .shape_intersections(shape, transform.translation, transform.rotation, filter)
        .into_iter()
        .filter(|entity| {
            let Ok((collider, obstacle)) = colliders.get(*entity) else {
                return true;
            };
            if collider.shape_scaled().as_cuboid().is_none() {
                return true;
            }
            let obstacle = obstacle.compute_transform();
            avian3d::collision::collider::contact_query::contact(
                shape,
                transform.translation,
                transform.rotation,
                collider,
                obstacle.translation,
                obstacle.rotation,
                0.0,
            )
            .map_or(true, |contact| {
                contact.is_some_and(|contact| contact.penetration > 0.0)
            })
        })
        .collect()
}

fn proposed_overlap_adjustment(
    transform: &Transform,
    shape: &Collider,
    intersections: &[Entity],
    colliders: &Query<(&Collider, &GlobalTransform)>,
) -> Option<Vec3> {
    let mut adjustment = Vec3::ZERO;
    for entity in intersections {
        let (collider, obstacle) = colliders.get(*entity).ok()?;
        let obstacle = obstacle.compute_transform();
        let contact = avian3d::collision::collider::contact_query::contact(
            shape,
            transform.translation,
            transform.rotation,
            collider,
            obstacle.translation,
            obstacle.rotation,
            0.0,
        )
        .ok()??;
        let normal = transform.rotation * contact.local_normal1;
        let depth = if let (Some(first), Some(second)) = (
            shape.shape_scaled().as_support_map(),
            collider.shape_scaled().as_support_map(),
        ) {
            let first = transform.translation
                + transform.rotation * first.local_support_point(contact.local_normal1);
            let second = obstacle.translation
                + obstacle.rotation * second.local_support_point(contact.local_normal2);
            (first - second).dot(normal)
        } else {
            contact.penetration
        };
        adjustment -= normal * depth;
    }
    (adjustment.length_squared() > f32::EPSILON).then_some(adjustment)
}

pub fn find_teleport_spot(
    spatial: &SceneQuery,
    transform: Transform,
    capsule_half_height: f32,
    capsule_radius: f32,
    colliders: &Query<(&Collider, &GlobalTransform)>,
    filter: &SpatialQueryFilter,
) -> Option<(Transform, Vec3)> {
    let full_shape = spawn_shape(capsule_half_height, capsule_radius);
    let shrink = SPAWN_ENCROACH_EPSILON;
    let shape = spawn_shape(capsule_half_height - shrink, capsule_radius - shrink);
    let intersections = shape_intersections(spatial, &shape, &transform, colliders, filter);
    if intersections.is_empty() {
        return Some((transform, Vec3::ZERO));
    }
    let adjustment =
        proposed_overlap_adjustment(&transform, &full_shape, &intersections, colliders)?;
    if adjustment.y.abs() > f32::EPSILON {
        let mut candidate = transform;
        candidate.translation.y += adjustment.y;
        if shape_intersections(spatial, &shape, &candidate, colliders, filter).is_empty() {
            return Some((candidate, Vec3::Y * adjustment.y));
        }
    }
    let xy_adjustments = teleport_xy_adjustments(Vec2::new(adjustment.x, adjustment.z));
    for xy in xy_adjustments {
        let mut candidate = transform;
        candidate.translation.x += xy.x;
        candidate.translation.z += xy.y;
        if shape_intersections(spatial, &shape, &candidate, colliders, filter).is_empty() {
            return Some((candidate, Vec3::new(xy.x, 0.0, xy.y)));
        }
    }
    if adjustment.y.abs() > f32::EPSILON {
        for xy in xy_adjustments {
            let mut candidate = transform;
            candidate.translation.x += xy.x;
            candidate.translation.y += adjustment.y;
            candidate.translation.z += xy.y;
            if shape_intersections(spatial, &shape, &candidate, colliders, filter).is_empty() {
                return Some((candidate, Vec3::new(xy.x, adjustment.y, xy.y)));
            }
        }
    }
    None
}

pub fn fit_player_start(
    spatial: &SceneQuery,
    start: Transform,
    ground_y: f32,
    capsule_half_height: f32,
    capsule_radius: f32,
    colliders: &Query<(&Collider, &GlobalTransform)>,
    filter: &SpatialQueryFilter,
) -> Option<AlsPlayerSpawnFit> {
    let mut base = yaw_only_transform(start);
    base.translation.y = player_start_grounded_height(ground_y, capsule_half_height);
    let shrink = SPAWN_ENCROACH_EPSILON;
    let shape = spawn_shape(capsule_half_height - shrink, capsule_radius - shrink);
    if shape_intersections(spatial, &shape, &base, colliders, filter).is_empty() {
        return Some(AlsPlayerSpawnFit {
            transform: base,
            ground_y,
            adjustment: 0.0,
            teleport_adjustment: Vec3::ZERO,
            occupancy: AlsPlayerStartOccupancy::Unoccupied,
        });
    }
    if let Some((transform, teleport_adjustment)) = find_teleport_spot(
        spatial,
        base,
        capsule_half_height,
        capsule_radius,
        colliders,
        filter,
    ) {
        return Some(AlsPlayerSpawnFit {
            transform,
            ground_y,
            adjustment: teleport_adjustment.y.max(0.0),
            teleport_adjustment,
            occupancy: AlsPlayerStartOccupancy::OccupiedAdjusted,
        });
    }
    for vertical_adjustment in PLAYER_START_VERTICAL_ADJUSTMENTS.into_iter().skip(1) {
        let mut candidate = base;
        candidate.translation.y += vertical_adjustment;
        if shape_intersections(spatial, &shape, &candidate, colliders, filter).is_empty() {
            return Some(AlsPlayerSpawnFit {
                transform: candidate,
                ground_y,
                adjustment: vertical_adjustment,
                teleport_adjustment: Vec3::Y * vertical_adjustment,
                occupancy: AlsPlayerStartOccupancy::OccupiedAdjusted,
            });
        }
    }
    None
}

fn player_start_ready_matches(current: &AlsPlayerStartReady, next: &AlsPlayerStartReady) -> bool {
    current
        .transform
        .translation
        .distance_squared(next.transform.translation)
        <= 1e-6
        && (1.0
            - current
                .transform
                .rotation
                .dot(next.transform.rotation)
                .abs())
            <= 1e-6
        && (current.ground_y - next.ground_y).abs() <= 1e-3
        && (current.adjustment - next.adjustment).abs() <= 1e-3
        && current
            .teleport_adjustment
            .distance_squared(next.teleport_adjustment)
            <= 1e-6
        && current.occupancy == next.occupancy
}

pub struct AlsPlayerStartPlugin;

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AlsPlayerStartPrepared;

impl Plugin for AlsPlayerStartPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AlsPlayerStartSettings>()
            .add_systems(Update, prepare_player_starts.in_set(AlsPlayerStartPrepared));
    }
}

fn prepare_player_starts(
    mut commands: Commands,
    settings: Res<AlsPlayerStartSettings>,
    spatial: SceneQuery,
    colliders: Query<(&Collider, &GlobalTransform)>,
    characters: Query<Entity, With<bevy_als::als::character::AlsCharacter>>,
    transforms: Query<&Transform>,
    parents: Query<&ChildOf>,
    mut starts: Query<
        (
            Entity,
            &AlsPlayerStart,
            &Transform,
            Option<&mut AlsPlayerStartProbe>,
            Option<&mut AlsPlayerStartReady>,
        ),
        With<AlsPlayerStart>,
    >,
) {
    let ground_filter = SpatialQueryFilter::from_excluded_entities(characters.iter());
    let occupancy_filter =
        SpatialQueryFilter::default().with_mask(bevy_als::als::character::AlsCollisionLayer::Default);
    for (entity, player_start, start, probe, ready) in &mut starts {
        let mut start = *start;
        for parent in transforms
            .iter_many(parents.iter_ancestors(entity))
            .filter_map(Result::ok)
        {
            start.translation = parent.transform_point(start.translation);
            start.rotation = parent.rotation * start.rotation;
        }
        let ground_y = if player_start.snap_to_ground {
            let probe_origin = start.translation + Vec3::Y * settings.probe_height;
            let Some(hit) = spatial.cast_ray(
                probe_origin,
                Dir3::NEG_Y,
                settings.probe_distance,
                true,
                &ground_filter,
            ) else {
                commands
                    .entity(entity)
                    .remove::<AlsPlayerStartReady>()
                    .insert(AlsPlayerStartProbe::default());
                continue;
            };
            probe_origin.y - hit.distance
        } else {
            start.translation.y - settings.capsule_half_height - ALS_PLAYER_START_SOURCE_CLEARANCE
        };
        let Some(fit) = fit_player_start(
            &spatial,
            start,
            ground_y,
            settings.capsule_half_height,
            settings.capsule_radius,
            &colliders,
            &occupancy_filter,
        ) else {
            commands
                .entity(entity)
                .remove::<AlsPlayerStartReady>()
                .insert(AlsPlayerStartProbe::default());
            continue;
        };
        let stable_frames = match probe {
            Some(mut probe)
                if (probe.ground_y - fit.ground_y).abs() <= 1e-3
                    && (probe.adjustment - fit.adjustment).abs() <= 1e-3
                    && probe
                        .teleport_adjustment
                        .distance_squared(fit.teleport_adjustment)
                        <= 1e-6
                    && probe.occupancy == Some(fit.occupancy) =>
            {
                probe.stable_frames += 1;
                probe.stable_frames
            }
            Some(mut probe) => {
                probe.ground_y = fit.ground_y;
                probe.adjustment = fit.adjustment;
                probe.teleport_adjustment = fit.teleport_adjustment;
                probe.occupancy = Some(fit.occupancy);
                probe.stable_frames = 1;
                probe.stable_frames
            }
            None => {
                commands.entity(entity).insert(AlsPlayerStartProbe {
                    ground_y: fit.ground_y,
                    adjustment: fit.adjustment,
                    teleport_adjustment: fit.teleport_adjustment,
                    occupancy: Some(fit.occupancy),
                    stable_frames: 1,
                });
                1
            }
        };
        if stable_frames < settings.required_stable_frames.max(1) {
            if ready.is_some() {
                commands.entity(entity).remove::<AlsPlayerStartReady>();
            }
            continue;
        }
        let next_ready = AlsPlayerStartReady {
            transform: fit.transform,
            ground_y: fit.ground_y,
            adjustment: fit.adjustment,
            teleport_adjustment: fit.teleport_adjustment,
            occupancy: fit.occupancy,
        };
        let ready_changed = ready
            .as_ref()
            .is_none_or(|current| !player_start_ready_matches(current, &next_ready));
        if let Some(mut ready) = ready {
            *ready = next_ready;
        } else {
            commands.entity(entity).insert(next_ready);
        }
        if ready_changed {
            info!(
                "[PLAYERSTART] entity={entity:?} occupancy={:?} ground_y={:.4} center_y={:.4} adjustment=({:.4},{:.4},{:.4}) stable_frames={} -> PASS",
                fit.occupancy,
                fit.ground_y,
                fit.transform.translation.y,
                fit.teleport_adjustment.x,
                fit.teleport_adjustment.y,
                fit.teleport_adjustment.z,
                stable_frames,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn existing_character_blocks_player_start_occupancy() {
        use avian3d::prelude::{PhysicsPlugins, RigidBody};
        use bevy::{asset::AssetPlugin, mesh::MeshPlugin, transform::TransformPlugin};
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            MeshPlugin,
            TransformPlugin,
            PhysicsPlugins::new(FixedPostUpdate),
            AlsPlayerStartPlugin,
        ));
        app.world_mut()
            .resource_mut::<AlsPlayerStartSettings>()
            .required_stable_frames = 1;
        app.finish();
        app.cleanup();
        let center = Vec3::Y * player_start_grounded_height(0.0, 0.9);
        let start = app
            .world_mut()
            .spawn((
                AlsPlayerStart::default(),
                Transform::from_translation(center),
            ))
            .id();
        app.world_mut().spawn((
            RigidBody::Static,
            avian3d::prelude::Collider::cuboid(20.0, 0.2, 20.0),
            Transform::from_xyz(0.0, -0.1, 0.0),
        ));
        let occupant = app
            .world_mut()
            .spawn((
                bevy_als::als::character::AlsCharacter,
                RigidBody::Static,
                bevy_als::als::character::als_character_capsule(0.9, 0.3),
                Transform::from_translation(center),
            ))
            .id();
        app.update();
        app.world_mut().run_schedule(FixedPostUpdate);
        app.update();
        assert!(app.world().entities().contains(occupant));
        let ready = app.world().get::<AlsPlayerStartReady>(start).unwrap();
        assert!(ready.ground_y.abs() < 1e-3);
        assert_eq!(ready.occupancy, AlsPlayerStartOccupancy::OccupiedAdjusted);

        app.world_mut().entity_mut(occupant).despawn();
        app.world_mut()
            .resource_mut::<AlsPlayerStartSettings>()
            .required_stable_frames = 2;
        app.update();
        app.world_mut().run_schedule(FixedPostUpdate);
        app.update();
        app.update();
        assert_eq!(
            app.world()
                .get::<AlsPlayerStartReady>(start)
                .unwrap()
                .occupancy,
            AlsPlayerStartOccupancy::Unoccupied
        );
        app.world_mut().spawn((
            bevy_als::als::character::AlsCharacter,
            RigidBody::Static,
            bevy_als::als::character::als_character_capsule(0.9, 0.3),
            Transform::from_translation(center),
        ));
        app.world_mut().run_schedule(FixedPostUpdate);
        app.update();
        assert!(
            app.world().get::<AlsPlayerStartReady>(start).is_none(),
            "Stale unoccupied readiness must not admit a pawn during occupancy stabilization: {:?}",
            app.world().get::<AlsPlayerStartReady>(start)
        );
        app.update();
        assert_eq!(
            app.world()
                .get::<AlsPlayerStartReady>(start)
                .unwrap()
                .occupancy,
            AlsPlayerStartOccupancy::OccupiedAdjusted
        );
    }

    fn fit(occupancy: AlsPlayerStartOccupancy) -> AlsPlayerSpawnFit {
        AlsPlayerSpawnFit {
            transform: Transform::IDENTITY,
            ground_y: 0.0,
            adjustment: 0.0,
            teleport_adjustment: Vec3::ZERO,
            occupancy,
        }
    }

    fn candidate(
        entity: u32,
        tag: Option<&str>,
        occupancy: AlsPlayerStartOccupancy,
    ) -> AlsPlayerStartCandidate {
        AlsPlayerStartCandidate {
            entity: Entity::from_raw_u32(entity).unwrap(),
            tag: tag.map(str::to_owned),
            fit: fit(occupancy),
        }
    }

    #[test]
    fn player_start_height_preserves_the_source_clearance() {
        assert!((player_start_grounded_height(0.0, 0.90) - 0.919).abs() < 1e-6);
        assert!((player_start_grounded_height(1.25, 0.90) - 2.169).abs() < 1e-6);
    }

    #[test]
    fn yaw_only_spawn_transform_discards_pitch_and_roll() {
        let input = Transform::from_rotation(Quat::from_euler(EulerRot::YXZ, 0.7, 0.4, -0.2))
            .with_scale(Vec3::new(4.0, 2.0, 0.25));
        let output = yaw_only_transform(input);
        let (yaw, pitch, roll) = output.rotation.to_euler(EulerRot::YXZ);
        assert!((yaw - 0.7).abs() < 1e-6);
        assert!(pitch.abs() < 1e-6 && roll.abs() < 1e-6);
        assert_eq!(output.scale, Vec3::ONE);
    }

    #[test]
    fn named_player_start_preserves_source_fname_identity() {
        for (tag, request, named) in [
            (Some("DoorA"), Some("doora"), true),
            (Some("Door_0"), Some("door_0"), true),
            (Some("Door_0"), Some("door"), false),
            (Some("Door_01"), Some("door_1"), false),
            (Some("Door_2147483647"), Some("door_2147483647"), true),
            (Some("None_0"), Some("none_0"), true),
            (Some("None_0"), Some("None"), false),
            (None, Some("None"), true),
            (None, Some("nOnE"), true),
            (Some(""), Some("None"), true),
            (Some("_17"), Some("none"), true),
            (None, Some("_0"), true),
            (None, Some("_17"), true),
            (None, Some("_2147483646"), true),
            (None, Some("_00"), false),
            (None, Some("_01"), false),
            (None, Some("_2147483647"), false),
            (None, Some("_4294967296"), false),
            (None, Some("_-1"), false),
            (None, Some("_+1"), false),
            (None, Some("_"), false),
            (None, Some("None_0"), false),
            (None, Some(" None"), false),
            (None, Some("Missing"), false),
            (None, Some(""), false),
            (None, None, false),
        ] {
            let candidates = [
                candidate(1, tag, AlsPlayerStartOccupancy::Unoccupied),
                candidate(2, Some("Other"), AlsPlayerStartOccupancy::Unoccupied),
            ];
            let selection = choose_player_start(&candidates, request, 1).unwrap();
            let expected = &candidates[usize::from(!named)];
            assert_eq!(
                selection.entity, expected.entity,
                "Named PlayerStart FName identity: tag={tag:?} request={request:?}"
            );
            assert_eq!(selection.tag, expected.tag);
        }
    }

    #[test]
    fn choose_player_start_prioritizes_unoccupied_then_tag_then_adjusted() {
        let candidates = [
            candidate(1, Some("Portal"), AlsPlayerStartOccupancy::OccupiedAdjusted),
            candidate(2, None, AlsPlayerStartOccupancy::Unoccupied),
            candidate(3, None, AlsPlayerStartOccupancy::Unoccupied),
        ];
        let unoccupied = choose_player_start(&candidates, None, 1).unwrap();
        assert!(unoccupied.fit.occupancy == AlsPlayerStartOccupancy::Unoccupied);
        let tagged = choose_player_start(&candidates, Some("Portal"), 0).unwrap();
        assert_eq!(tagged.entity, Entity::from_raw_u32(1).unwrap());
        assert!(tagged.used_occupied_fallback);
        let occupied_only = choose_player_start(&candidates[..1], None, 0).unwrap();
        assert!(occupied_only.used_occupied_fallback);
    }

    #[test]
    fn teleport_xy_permutations_match_engine_order() {
        assert_eq!(
            teleport_xy_adjustments(Vec2::new(2.0, 3.0)),
            [
                Vec2::new(2.0, 3.0),
                Vec2::new(-2.0, 3.0),
                Vec2::new(2.0, -3.0),
                Vec2::new(-2.0, -3.0),
                Vec2::new(3.0, 2.0),
                Vec2::new(-3.0, 2.0),
                Vec2::new(3.0, -2.0),
                Vec2::new(-3.0, -2.0),
            ]
        );
        let one_axis = teleport_xy_adjustments(Vec2::new(2.0, 0.0));
        assert_eq!(one_axis[0], Vec2::new(2.0, 0.0));
        assert_eq!(one_axis[4], Vec2::splat(2.0));
    }

    #[test]
    fn default_settings_match_the_als_capsule() {
        let settings = AlsPlayerStartSettings::default();
        assert_eq!(
            settings.capsule_half_height,
            bevy_als::als::character::ALS_CHARACTER_CAPSULE_HALF_HEIGHT
        );
        assert_eq!(
            settings.capsule_radius,
            bevy_als::als::character::ALS_CHARACTER_CAPSULE_RADIUS
        );
        assert_eq!(settings.required_stable_frames, 2);
        assert!(!AlsPlayerStart::default().snap_to_ground);
    }
}
