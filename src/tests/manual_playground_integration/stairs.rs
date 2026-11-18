use super::*;
use bevy_als::als::aim::AlsLastFreeRotationMode;
use crate::input::AlsInputCommand;
use bevy_als::als::turn_in_place::wrap_angle_degrees;
use bevy_als::extensions::SceneQuery;
use crate::playground_collision::PlaygroundMeshColliderBuilt;
use avian3d::prelude::{ShapeCastConfig, SpatialQueryFilter};

const MESHES: [&str; 3] = ["SM_Als_Stairs_01", "SM_Als_Stairs_02", "SM_Als_Stairs_03"];
const NO_PROGRESS_SECONDS: f64 = 5.0;

#[derive(Clone, Copy, Debug)]
pub(super) struct Route {
    pub(super) bottom: Vec3,
    pub(super) top: Vec3,
}

impl Route {
    pub(super) fn from_collider(
        collider: &Collider,
        transform: &GlobalTransform,
        half_height: f32,
        mut supported_center: impl FnMut(Vec3) -> Option<Vec3>,
    ) -> Option<Self> {
        let mesh = collider.shape().as_trimesh()?;
        let vertices = mesh.vertices();
        let min = vertices
            .iter()
            .map(|point| Vec3::new(point.x, point.y, point.z))
            .reduce(Vec3::min)?;
        let max = vertices
            .iter()
            .map(|point| Vec3::new(point.x, point.y, point.z))
            .reduce(Vec3::max)?;
        let mut treads: Vec<_> = mesh
            .indices()
            .iter()
            .filter_map(|triangle| {
                let points = triangle.map(|index| {
                    let point = vertices[index as usize];
                    Vec3::new(point.x, point.y, point.z)
                });
                ((points[1] - points[0]).cross(points[2] - points[0]).y > 1e-4).then_some((
                    points[0].y,
                    points.iter().map(|point| point.z).reduce(f32::min).unwrap(),
                    points.iter().map(|point| point.z).reduce(f32::max).unwrap(),
                ))
            })
            .collect();
        treads.sort_by(|left, right| right.0.total_cmp(&left.0));
        treads.dedup_by(|next, previous| {
            if (next.0 - previous.0).abs() >= 1e-4 {
                return false;
            }
            previous.1 = previous.1.min(next.1);
            previous.2 = previous.2.max(next.2);
            true
        });
        let center_x = (min.x + max.x) * 0.5;
        let center_offset = Vec3::Y * half_height;
        Some(Self {
            bottom: transform.transform_point(Vec3::new(center_x, min.y, max.z + 0.75))
                + center_offset,
            top: treads.into_iter().find_map(|(height, start_z, end_z)| {
                supported_center(
                    transform.transform_point(Vec3::new(center_x, height, (start_z + end_z) * 0.5))
                        + center_offset,
                )
            })?,
        })
    }

    fn endpoints(self, index: usize) -> (Vec3, Vec3) {
        if index % 2 == 0 {
            (self.bottom, self.top)
        } else {
            (self.top, self.bottom)
        }
    }
}

fn supported_tread_center(
    spatial: &SceneQuery,
    capsule: &Collider,
    character: Entity,
    stairs: Entity,
    candidate: Vec3,
    half_height: f32,
) -> Option<Vec3> {
    let origin = candidate + Vec3::Y * half_height;
    let floor = spatial.cast_shape_predicate(
        capsule,
        origin,
        Quat::IDENTITY,
        Dir3::NEG_Y,
        &ShapeCastConfig::from_max_distance(half_height * 2.0),
        &SpatialQueryFilter::default(),
        &|entity| entity == stairs,
    )?;
    let center = origin
        + Vec3::Y
            * (bevy_als::als::character::ALS_CHARACTER_FLOOR_CLEARANCE
                + bevy_als::als::character::ALS_CHARACTER_TNUA_FLOAT_EPSILON
                - floor.distance);
    spatial
        .shape_intersections(
            capsule,
            center,
            Quat::IDENTITY,
            &SpatialQueryFilter::from_excluded_entities([character, stairs]),
        )
        .is_empty()
        .then_some(center)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
enum Stage {
    #[default]
    Ready,
    Traverse,
    Settle,
    VelocityMode,
    ViewMode,
}

#[derive(Default)]
pub(super) struct Probe {
    route: Option<Route>,
    index: usize,
    stage: Stage,
    stage_started: f64,
    last_progress_at: f64,
    furthest: f32,
    move_prefix_speed: Option<f32>,
    pub(super) completed: [bool; 6],
    pub(super) failure: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq)]
struct Frame {
    position: Vec3,
    velocity: Vec3,
    locomotion: LocomotionState,
    stance: Stance,
    desired_gait: Gait,
    desired_mode: RotationMode,
    current_mode: RotationMode,
    view_yaw: f32,
}

impl Probe {
    fn begin(&mut self, stage: Stage, now: f64) {
        self.stage = stage;
        self.stage_started = now;
        self.last_progress_at = now;
        info!(
            "[MANUAL-INTEGRATION][STAIRS] route={} stage={stage:?}",
            self.index
        );
    }

    fn command(&mut self, now: f64, frame: Frame, delta_seconds: f32) -> AlsInputCommand {
        let mut command = AlsInputCommand::default();
        let Some(route) = self.route else {
            return command;
        };
        let (start, end) = route.endpoints(self.index);
        let direction = (end - start).with_y(0.0).normalize();
        let length = (end - start).with_y(0.0).length();
        let distance = (frame.position - start).dot(direction);
        let remaining = length - distance;
        let yaw_error =
            wrap_angle_degrees((world_forward_to_yaw(direction) - frame.view_yaw).to_degrees())
                .to_radians();
        let speed = frame.velocity.with_y(0.0).length();
        command.look_delta.x = -yaw_error / bevy_als::als::view::LOOK_MOUSE_RADIANS_PER_COUNT;
        command.crouch_pressed = frame.stance != Stance::Standing;
        let gait = if self.stage != Stage::Traverse || distance < 3.5 || remaining < 3.5 {
            Gait::Walking
        } else if distance < 8.0 {
            Gait::Running
        } else {
            Gait::Sprinting
        };
        command.sprint_held = gait == Gait::Sprinting;
        command.walk_pressed = matches!(
            (gait, frame.desired_gait),
            (Gait::Walking, Gait::Running | Gait::Sprinting) | (Gait::Running, Gait::Walking)
        );
        match self.stage {
            Stage::Ready => {
                command.rotation_mode_pressed = frame.desired_mode != RotationMode::ViewDirection;
                if frame.locomotion == LocomotionState::Grounded
                    && frame.stance == Stance::Standing
                    && speed < 0.05
                    && yaw_error.abs() < 1.0_f32.to_radians()
                    && frame.current_mode == RotationMode::ViewDirection
                    && frame.desired_mode == RotationMode::ViewDirection
                    && now - self.stage_started >= 0.3
                {
                    self.begin(Stage::Traverse, now);
                }
            }
            Stage::Traverse => {
                if frame.locomotion != LocomotionState::Grounded {
                    self.failure = Some(format!(
                        "route={} became airborne at distance={distance:.3}",
                        self.index
                    ));
                } else if remaining <= 0.03 {
                    self.begin(Stage::Settle, now);
                } else {
                    command.move_dir = Vec2::Y
                        * if gait == Gait::Walking && delta_seconds > 0.0 {
                            (remaining / (1.75 * delta_seconds)).clamp(0.0, 1.0)
                        } else {
                            1.0
                        };
                    if distance > self.furthest + 0.1 {
                        if distance.floor() > self.furthest.floor() {
                            info!(
                                "[MANUAL-INTEGRATION][STAIRS] route={} distance={distance:.3}",
                                self.index
                            );
                        }
                        self.furthest = distance;
                        self.last_progress_at = now;
                    }
                }
            }
            Stage::Settle => {
                if speed < 0.05
                    && frame.locomotion == LocomotionState::Grounded
                    && (frame.position - end).with_y(0.0).length() < 0.15
                    && (frame.position.y - end.y).abs() < 0.1
                {
                    self.begin(Stage::VelocityMode, now);
                }
            }
            Stage::VelocityMode | Stage::ViewMode => {
                let target = if self.stage == Stage::VelocityMode {
                    RotationMode::VelocityDirection
                } else {
                    RotationMode::ViewDirection
                };
                command.rotation_mode_pressed = frame.desired_mode != target;
                if frame.current_mode == target
                    && frame.desired_mode == target
                    && frame.locomotion == LocomotionState::Grounded
                    && now - self.stage_started >= 0.3
                {
                    if self.stage == Stage::VelocityMode {
                        self.begin(Stage::ViewMode, now);
                    } else {
                        self.completed[self.index] = true;
                        self.index += 1;
                        self.furthest = 0.0;
                        if self.index % 2 == 0 {
                            self.route = None;
                        }
                        self.begin(Stage::Ready, now);
                    }
                }
            }
        }
        if now - self.last_progress_at >= NO_PROGRESS_SECONDS {
            self.failure = Some(format!(
                "route={} stage={:?} no_progress_seconds={:.3} distance={distance:.3} frame={frame:?}",
                self.index,
                self.stage,
                now - self.last_progress_at
            ));
        }
        command
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(position: Vec3) -> Frame {
        Frame {
            position,
            velocity: Vec3::ZERO,
            locomotion: LocomotionState::Grounded,
            stance: Stance::Standing,
            desired_gait: Gait::Walking,
            desired_mode: RotationMode::ViewDirection,
            current_mode: RotationMode::ViewDirection,
            view_yaw: 0.0,
        }
    }

    #[test]
    fn stairs_route_uses_loaded_mesh_bounds_and_actor_transform() {
        let collider = Collider::trimesh(
            vec![
                Vec3::new(0.0, -0.18, 0.0),
                Vec3::new(6.67, -0.18, 0.0),
                Vec3::new(0.0, -0.18, -0.37),
                Vec3::new(0.0, 7.92, -16.65),
                Vec3::new(6.67, 7.92, -16.65),
                Vec3::new(0.0, 7.92, -16.28),
            ],
            vec![[0, 1, 2], [3, 5, 4]],
        );
        let transform = GlobalTransform::from(
            Transform::from_xyz(-3.333333, 0.18, -30.0)
                .with_rotation(Quat::from_rotation_y(0.7))
                .with_scale(Vec3::splat(1.5)),
        );
        let route = Route::from_collider(&collider, &transform, 0.9, Some).unwrap();
        assert!(route.bottom.abs_diff_eq(
            transform.transform_point(Vec3::new(3.335, -0.18, 0.75)) + Vec3::Y * 0.9,
            1e-5
        ));
        assert!(route.top.abs_diff_eq(
            transform.transform_point(Vec3::new(3.335, 7.92, -16.465)) + Vec3::Y * 0.9,
            1e-5
        ));
        assert_eq!(route.endpoints(1), (route.top, route.bottom));
        assert!(Route::from_collider(&Collider::sphere(1.0), &transform, 0.9, Some).is_none());
    }

    #[test]
    fn stairs_route_ends_on_a_supported_tread_before_the_original_terminal_wall() {
        use avian3d::prelude::{PhysicsPlugins, SpatialQueryFilter};
        use bevy::ecs::system::RunSystemOnce;
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            TransformPlugin,
            PhysicsPlugins::new(PostUpdate),
        ))
        .init_resource::<Assets<Mesh>>()
        .add_message::<bevy::asset::AssetEvent<Mesh>>();
        let mut vertices = Vec::new();
        let mut triangles = Vec::new();
        for step in 0..12 {
            let front = -(step as f32) * 0.37;
            let bottom = step as f32 * 0.18;
            let top = bottom + 0.18;
            let index = vertices.len() as u32;
            vertices.extend([
                Vec3::new(-3.335, bottom, front),
                Vec3::new(3.335, bottom, front),
                Vec3::new(-3.335, top, front),
                Vec3::new(3.335, top, front),
                Vec3::new(-3.335, top, front - 0.37),
                Vec3::new(3.335, top, front - 0.37),
            ]);
            triangles.extend([
                [index, index + 1, index + 2],
                [index + 1, index + 3, index + 2],
                [index + 2, index + 3, index + 4],
                [index + 3, index + 5, index + 4],
            ]);
        }
        let stairs = app
            .world_mut()
            .spawn((
                RigidBody::Static,
                Collider::trimesh(vertices, triangles),
                Transform::default(),
            ))
            .id();
        let wall = app
            .world_mut()
            .spawn((
                RigidBody::Static,
                Collider::cuboid(20.0, 2.0, 0.5),
                Transform::from_xyz(0.0, 2.5, -4.29),
            ))
            .id();
        app.finish();
        for _ in 0..3 {
            app.update();
        }
        app.init_resource::<ManualIntegrationState>()
            .init_resource::<ManualRuntimePhaseEvidence>()
            .init_resource::<Assets<AlsMovementParams>>();
        app.world_mut().run_system_once(drive).unwrap();
        let (route, intersections, floor) = app.world_mut().run_system_once(move |spatial: SceneQuery,
            colliders: Query<&Collider>, candidates: Query<bevy_als::als::motion::CharacterCollisionCandidates>| {
            let capsule = bevy_als::als::character::als_character_capsule(0.9, 0.3);
            let route = Route::from_collider(colliders.get(stairs).unwrap(), &GlobalTransform::IDENTITY, 0.9,
                |candidate| supported_tread_center(&spatial, &capsule, Entity::PLACEHOLDER, stairs, candidate, 0.9)).unwrap();
            let intersections = spatial.shape_intersections(&capsule, route.top, Quat::IDENTITY,
                &SpatialQueryFilter::default());
            let floor = bevy_als::als::motion::find_ground_floor(&spatial, route.top, 0.9, 0.3,
                &AlsMovementParams::default().cmc, LocomotionState::Grounded,
                &SpatialQueryFilter::default(), Entity::PLACEHOLDER, &candidates);
            (route, intersections, floor)
        }).unwrap();
        assert!(
            !intersections.contains(&wall),
            "route endpoint intersects terminal wall: {:?}",
            route.top
        );
        assert!(
            intersections.is_empty(),
            "route endpoint intersects the stair tread"
        );
        let clearance = bevy_als::als::character::ALS_CHARACTER_FLOOR_CLEARANCE
            + bevy_als::als::character::ALS_CHARACTER_TNUA_FLOAT_EPSILON;
        assert!(
            floor.is_some_and(|(center, _, entity)| entity == stairs
                && (route.top.y - center - clearance).abs() < 1e-4),
            "Route endpoint has no walkable stair support: route={route:?} floor={floor:?}"
        );
        assert!(route.top.z < -3.0 && route.top.z > -3.74);
    }

    #[test]
    fn stairs_driver_caps_final_walk_input_to_the_remaining_distance() {
        let route = Route {
            bottom: Vec3::ZERO,
            top: Vec3::new(0.0, 8.1, -16.475),
        };
        let mut probe = Probe {
            route: Some(route),
            stage: Stage::Traverse,
            ..default()
        };
        let mut sample = frame(route.top + Vec3::Z * 0.2);
        sample.velocity = Vec3::NEG_Z * 1.75;
        let command = probe.command(0.1879731, sample, 0.1879731);
        assert!(
            command.move_dir.length() * 1.75 * 0.1879731 <= 0.2 + 1e-6,
            "The source walking input must not request more than the remaining route distance during a long frame: command={command:?}"
        );
        assert!(command.move_dir.length() > 0.0);
    }

    #[test]
    fn stairs_sprint_segment_preserves_stable_visual_samples_at_source_speed() {
        for delta in [0.07051209_f64, 0.1] {
            let route = Route {
                bottom: Vec3::ZERO,
                top: Vec3::new(0.0, 8.1, -16.475),
            };
            let mut probe = Probe {
                route: Some(route),
                stage: Stage::Traverse,
                ..default()
            };
            let mut observations = Vec::new();
            for index in 0..32 {
                let elapsed = index as f64 * delta;
                let mut sample = frame(Vec3::NEG_Z * (8.0 + 6.5 * elapsed as f32));
                sample.velocity = Vec3::NEG_Z * 6.5;
                sample.desired_gait = Gait::Sprinting;
                let command = probe.command(elapsed, sample, delta as f32);
                if !command.sprint_held {
                    break;
                }
                if elapsed >= 0.5 {
                    observations.push(elapsed);
                }
            }
            assert!(
                observations.len() >= 3,
                "Source-speed sprint must hold long enough for three stable visual samples: delta={delta} observations={observations:?}"
            );
            assert!(
                observations.last().unwrap() - observations.first().unwrap() >= 0.12,
                "Source-speed sprint must preserve the original pose observation span: delta={delta} observations={observations:?}"
            );
        }
    }

    #[test]
    fn stairs_driver_requires_grounding_then_walk_run_sprint_and_hotkey_recovery() {
        let route = Route {
            bottom: Vec3::new(0.0, 0.9, 0.75),
            top: Vec3::new(0.0, 8.82, -16.465),
        };
        let mut probe = Probe {
            route: Some(route),
            ..default()
        };
        let mut sample = frame(route.bottom);
        sample.locomotion = LocomotionState::Air;
        assert_eq!(probe.command(0.4, sample, 1.0 / 60.0).move_dir, Vec2::ZERO);
        assert_eq!(probe.stage, Stage::Ready);
        sample.locomotion = LocomotionState::Grounded;
        probe.command(0.5, sample, 1.0 / 60.0);
        assert_eq!(probe.stage, Stage::Traverse);
        sample.position.z -= 1.0;
        sample.desired_gait = Gait::Running;
        assert!(probe.command(0.6, sample, 1.0 / 60.0).walk_pressed);
        sample.position.z = route.bottom.z - 5.0;
        sample.desired_gait = Gait::Walking;
        assert!(probe.command(0.7, sample, 1.0 / 60.0).walk_pressed);
        sample.position.z = route.bottom.z - 9.0;
        sample.desired_gait = Gait::Running;
        assert!(probe.command(0.8, sample, 1.0 / 60.0).sprint_held);
        sample.position = route.top;
        probe.command(0.9, sample, 1.0 / 60.0);
        assert_eq!(probe.stage, Stage::Settle);
        probe.command(1.0, sample, 1.0 / 60.0);
        assert_eq!(probe.stage, Stage::VelocityMode);
        assert!(probe.command(1.1, sample, 1.0 / 60.0).rotation_mode_pressed);
        sample.desired_mode = RotationMode::VelocityDirection;
        assert!(!probe.command(1.4, sample, 1.0 / 60.0).rotation_mode_pressed);
        assert_eq!(probe.stage, Stage::VelocityMode);
        assert!(!probe.completed[0]);
        sample.current_mode = RotationMode::VelocityDirection;
        probe.command(1.5, sample, 1.0 / 60.0);
        assert_eq!(probe.stage, Stage::ViewMode);
        assert!(probe.command(1.6, sample, 1.0 / 60.0).rotation_mode_pressed);
        sample.current_mode = RotationMode::ViewDirection;
        sample.desired_mode = RotationMode::ViewDirection;
        probe.command(1.9, sample, 1.0 / 60.0);
        assert!(probe.completed[0]);
        assert_eq!(probe.index, 1);
        assert!(probe.route.is_some());
        let command = probe.command(2.0, sample, 1.0 / 60.0);
        assert!(
            (command.look_delta.x.abs() * bevy_als::als::view::LOOK_MOUSE_RADIANS_PER_COUNT
                - std::f32::consts::PI)
                .abs()
                < 1e-5
        );
        assert_eq!(command.move_dir, Vec2::ZERO);
        probe.index = 0;
        probe.stage = Stage::Traverse;
        sample.position = route.top + Vec3::Z * 0.115;
        assert_eq!(probe.command(2.1, sample, 1.0 / 60.0).move_dir, Vec2::Y);
    }

    #[test]
    fn stairs_driver_watchdog_rejects_stalls_and_locked_rotation_mode() {
        let route = Route {
            bottom: Vec3::new(0.0, 0.9, 0.75),
            top: Vec3::new(0.0, 8.82, -16.465),
        };
        for stage in [Stage::Traverse, Stage::VelocityMode, Stage::ViewMode] {
            let mut probe = Probe {
                route: Some(route),
                stage,
                ..default()
            };
            let mut sample = frame(route.bottom);
            sample.desired_mode = RotationMode::VelocityDirection;
            sample.current_mode = RotationMode::Aiming;
            probe.command(NO_PROGRESS_SECONDS + 0.1, sample, 1.0 / 60.0);
            assert!(
                probe
                    .failure
                    .as_deref()
                    .unwrap()
                    .contains("no_progress_seconds=")
            );
            assert!(!probe.completed.into_iter().any(|value| value));
        }
    }

    #[test]
    fn stairs_observer_records_actual_frame_before_qualifying_animation_windows() {
        use bevy_als::als::character::AlsCmcAdapterRuntime;
        use bevy::ecs::system::RunSystemOnce;
        let mut world = World::new();
        let mut assets = Assets::<AlsMovementParams>::default();
        let handle = assets.add(AlsMovementParams::default());
        world.insert_resource(assets);
        let character = world
            .spawn((
                AlsCharacter,
                Position(Vec3::Y * 0.9),
                LinearVelocity(Vec3::NEG_Z * 1.75),
                LocomotionState::Grounded,
                Stance::Standing,
                AlsDesiredGait(Gait::Walking),
                Gait::Walking,
                AlsLastFreeRotationMode(RotationMode::ViewDirection),
                RotationMode::ViewDirection,
                AlsViewYaw(0.0),
                AlsInputOverride(AlsInputCommand {
                    move_dir: Vec2::Y,
                    ..default()
                }),
                AlsActualMotion {
                    horizontal_speed: 1.75,
                    ..default()
                },
                AlsCmcAdapterRuntime {
                    simulation_delta_seconds: 0.1,
                    ..default()
                },
                AlsMovementParamsHandle(handle),
                AlsStandingGraphTelemetry {
                    actual_movement_state: Some(0),
                    actual_movement_pose_ready: Some(true),
                    actual_standing_pose_ready: Some(true),
                    ..default()
                },
            ))
            .id();
        let mut controller =
            bevy_tnua::TnuaController::<bevy_als::als::control_scheme::AlsControlScheme>::default();
        controller.basis.desired_motion = Vec3::NEG_Z;
        world.entity_mut(character).insert((
            AlsMovementBaseDynamicsState {
                world_velocity: Vec3::NEG_Z * 1.75,
                relative_velocity: Vec3::NEG_Z * 1.75,
                ..default()
            },
            bevy_als::als::dynamics::AlsGroundedDynamics::default(),
            controller,
            Transform::from_xyz(0.0, 0.9, 0.0),
        ));
        world.insert_resource(ManualIntegrationState {
            character: Some(character),
            phase: IntegrationPhase::Stairs,
            stairs_probe: Probe {
                route: Some(Route {
                    bottom: Vec3::Y * 0.9,
                    top: Vec3::new(0.0, 9.0, -17.215),
                }),
                stage: Stage::Traverse,
                ..default()
            },
            ..default()
        });
        world.init_resource::<ManualIntegrationGateReport>();
        world.init_resource::<ManualAnimationWindowEvidence>();
        for index in 0..8 {
            world
                .resource_mut::<ManualIntegrationState>()
                .game_time_seconds = index as f64 * 0.1;
            world.get_mut::<Position>(character).unwrap().0.z = -index as f32 * 0.175;
            world.run_system_once(observe).unwrap();
            if index < 5 {
                assert!(
                    world
                        .resource::<ManualAnimationWindowEvidence>()
                        .label
                        .is_none()
                );
            }
        }
        let window = world.resource::<ManualAnimationWindowEvidence>();
        assert_eq!(window.label, Some("stairs_01_up_walk"));
        assert!(window.requires_source_pose_chain && !window.source_pose_chain.is_empty());
        let report = world.resource::<ManualIntegrationGateReport>();
        let sample = report.stairs_trace.last().unwrap();
        assert!((sample.displacement_speed.unwrap() - 1.75).abs() < 1e-5);
        let json = serde_json::to_value(sample).unwrap();
        assert_eq!(json["position"][2], sample.frame.position.z);
        assert_eq!(json["desired_mode"], "Als.RotationMode.ViewDirection");
        assert_eq!(json["movement_pose_ready"], true);
        assert!((sample.requested_displacement_speed - 1.75).abs() < 1e-5);
        assert!((sample.initial_speed - 1.75).abs() < 1e-5);
        world
            .get_mut::<AlsMovementBaseDynamicsState>(character)
            .unwrap()
            .relative_velocity = Vec3::NEG_Z * 3.0;
        world.run_system_once(observe).unwrap();
        let braking = world
            .resource::<ManualIntegrationGateReport>()
            .stairs_trace
            .last()
            .unwrap();
        assert!(
            (braking.requested_displacement_speed - 2.0).abs() < 1e-5,
            "Source two50ms braking steps have speeds2.25 and1.75, distinct from the final velocity: actual={}",
            braking.requested_displacement_speed
        );
        world
            .get_mut::<AlsMovementBaseDynamicsState>(character)
            .unwrap()
            .relative_velocity = Vec3::NEG_Z * 1.75;
        world.get_mut::<LinearVelocity>(character).unwrap().0.z = -2.0;
        world
            .resource_mut::<ManualIntegrationState>()
            .game_time_seconds = 0.8;
        world.run_system_once(observe).unwrap();
        let sample = world
            .resource::<ManualIntegrationGateReport>()
            .stairs_trace
            .last()
            .unwrap();
        assert_eq!(sample.frame.velocity.z, -2.0);
        assert_eq!(sample.animation_speed, 1.75);
        let start = world.get::<Position>(character).unwrap().0;
        world
            .get_mut::<bevy_als::als::character::AlsCmcAdapterRuntime>(character)
            .unwrap()
            .grounded_move_start = Some(start);
        world.get_mut::<Position>(character).unwrap().0.z -= 0.1;
        world.run_system_once(capture_move_prefix).unwrap();
        world.get_mut::<Position>(character).unwrap().0.z -= 0.075;
        world.get_mut::<LinearVelocity>(character).unwrap().0.z = -1.75;
        world
            .resource_mut::<ManualIntegrationState>()
            .game_time_seconds = 0.9;
        world.run_system_once(observe).unwrap();
        let sample = world
            .resource::<ManualIntegrationGateReport>()
            .stairs_trace
            .last()
            .unwrap();
        assert!((sample.safe_move_prefix_speed.unwrap() - 1.0).abs() < 1e-5);
        assert!((sample.displacement_speed.unwrap() - 1.75).abs() < 1e-5);
    }
}

pub(super) fn drive(
    time: Res<Time>,
    mut state: ResMut<ManualIntegrationState>,
    mut phase_evidence: ResMut<ManualRuntimePhaseEvidence>,
    stairs: Query<(
        Entity,
        &Collider,
        &GlobalTransform,
        &PlaygroundMeshColliderBuilt,
    )>,
    mut actors: ParamSet<(
        SceneQuery,
        Query<
            (
                &mut AlsInputOverride,
                &mut Transform,
                &mut Position,
                Option<&mut Rotation>,
                &mut LinearVelocity,
                &mut AlsViewYaw,
                &mut AlsViewPitch,
                &Stance,
                &LocomotionState,
                &AlsDesiredGait,
                &AlsLastFreeRotationMode,
                &RotationMode,
                &Collider,
            ),
            With<AlsCharacter>,
        >,
        bevy::transform::helper::TransformHelper,
    )>,
    mut rotation_state: Query<(
        &AlsMovementParamsHandle,
        Option<&mut AlsRotationResponseState>,
        &mut AlsAimingYawConstraint,
        Option<&mut AlsRotationAnimationState>,
    )>,
    params: Res<Assets<AlsMovementParams>>,
    parents: Query<&ChildOf>,
    mut commands: Commands,
) {
    let Some(character) = state.character else {
        return;
    };
    let now = state.game_time_seconds;
    if state.stairs_probe.index == 6 {
        begin_phase(&mut state, IntegrationPhase::Finish);
        return;
    }
    let pending_route = if state.stairs_probe.route.is_none() {
        let name = MESHES[state.stairs_probe.index / 2];
        let Some((stairs_entity, collider, global, _)) = stairs
            .iter()
            .find(|(_, _, _, source)| source.mesh_base == name)
        else {
            return;
        };
        let Ok((handle, _, _, _)) = rotation_state.get(character) else {
            return;
        };
        let Some(params) = params.get(handle.id()) else {
            return;
        };
        let half_height = params.crouch.standing_capsule_half_height;
        let Ok(capsule) = actors.p1().get(character).map(|parts| parts.12.clone()) else {
            return;
        };
        let spatial = actors.p0();
        let Some(route) = Route::from_collider(collider, global, half_height, |candidate| {
            supported_tread_center(
                &spatial,
                &capsule,
                character,
                stairs_entity,
                candidate,
                half_height,
            )
        }) else {
            return;
        };
        Some(route)
    } else {
        None
    };
    let reset_space = if pending_route.is_some() {
        let Ok(world) = actors.p2().compute_global_transform(character) else {
            return;
        };
        let parent_world = if let Ok(parent) = parents.get(character) {
            let Ok(parent_world) = actors.p2().compute_global_transform(parent.parent()) else {
                return;
            };
            parent_world
        } else {
            let parent_transform = Transform::from_scale(Vec3::splat(2.0));
            let parent = commands
                .spawn((
                    Name::new("manual_world_capsule_parent"),
                    parent_transform,
                    GlobalTransform::from(parent_transform),
                ))
                .id();
            commands.entity(character).insert(ChildOf(parent));
            GlobalTransform::from(parent_transform)
        };
        Some((world.to_scale_rotation_translation().0, parent_world))
    } else {
        None
    };
    let mut characters = actors.p1();
    let Ok((
        mut input,
        mut transform,
        mut position,
        rotation,
        mut velocity,
        mut yaw,
        mut pitch,
        stance,
        locomotion,
        gait,
        desired_mode,
        mode,
        _,
    )) = characters.get_mut(character)
    else {
        return;
    };
    if let Some(route) = pending_route {
        let Ok((_, response, mut constraint, animation)) = rotation_state.get_mut(character) else {
            return;
        };
        reset_character(
            &mut transform,
            &mut position,
            rotation,
            &mut velocity,
            &mut yaw,
            &mut pitch,
            route.bottom + Vec3::Y * 0.05,
            world_forward_to_yaw((route.top - route.bottom).with_y(0.0)),
        );
        if let Some((scale, parent_world)) = reset_space {
            *transform = GlobalTransform::from(Transform {
                translation: position.0,
                rotation: transform.rotation,
                scale,
            })
            .reparented_to(&parent_world);
        }
        reinitialize_fixture_rotation_response(response, &mut constraint, animation);
        rebase_temporal_fixture(&mut state, &mut phase_evidence, "stairs");
        state.stairs_probe.route = Some(route);
        state.stairs_probe.begin(Stage::Ready, now);
        return;
    }
    input.0 = state.stairs_probe.command(
        now,
        Frame {
            position: position.0,
            velocity: velocity.0,
            locomotion: *locomotion,
            stance: *stance,
            desired_gait: gait.0,
            desired_mode: desired_mode.0,
            current_mode: *mode,
            view_yaw: yaw.0,
        },
        time.delta_secs(),
    );
    if let Some(failure) = &state.stairs_probe.failure {
        error!("[MANUAL-INTEGRATION][STAIRS] {failure}");
        state.timed_out = true;
        begin_phase(&mut state, IntegrationPhase::Finish);
    }
}

const WINDOWS: [[&str; 3]; 6] = [
    [
        "stairs_01_up_walk",
        "stairs_01_up_run",
        "stairs_01_up_sprint",
    ],
    [
        "stairs_01_down_walk",
        "stairs_01_down_run",
        "stairs_01_down_sprint",
    ],
    [
        "stairs_02_up_walk",
        "stairs_02_up_run",
        "stairs_02_up_sprint",
    ],
    [
        "stairs_02_down_walk",
        "stairs_02_down_run",
        "stairs_02_down_sprint",
    ],
    [
        "stairs_03_up_walk",
        "stairs_03_up_run",
        "stairs_03_up_sprint",
    ],
    [
        "stairs_03_down_walk",
        "stairs_03_down_run",
        "stairs_03_down_sprint",
    ],
];

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Sample {
    route_index: usize,
    mesh: &'static str,
    stage: Stage,
    elapsed_seconds: f64,
    simulation_delta_seconds: f32,
    distance_m: f32,
    route_length_m: f32,
    input: Vec2,
    rotation_hotkey: bool,
    actual_gait: Gait,
    capsule_world_scale: f32,
    capsule_local_scale: f32,
    #[serde(flatten)]
    frame: Frame,
    expected_speed: f32,
    requested_speed: f32,
    requested_displacement_speed: f32,
    initial_speed: f32,
    animation_speed: f32,
    safe_move: bool,
    safe_move_prefix_speed: Option<f32>,
    displacement_speed: Option<f32>,
    movement_state: Option<u8>,
    movement_pose_ready: Option<bool>,
    standing_pose_ready: Option<bool>,
    gait_started_seconds: f64,
}

impl Sample {
    fn stable(&self) -> bool {
        self.stage == Stage::Traverse
            && self.input.length() >= 0.999
            && self.elapsed_seconds - self.gait_started_seconds >= 0.5
    }

    fn gait_index(&self) -> usize {
        match self.frame.desired_gait {
            Gait::Walking => 0,
            Gait::Sprinting => 2,
            _ => 1,
        }
    }
}

pub(super) fn observe(
    state: Res<ManualIntegrationState>,
    mut report: ResMut<ManualIntegrationGateReport>,
    mut window: ResMut<ManualAnimationWindowEvidence>,
    characters: Query<
        (
            &Position,
            &LinearVelocity,
            &LocomotionState,
            &Stance,
            &AlsDesiredGait,
            &Gait,
            &AlsLastFreeRotationMode,
            &RotationMode,
            &AlsViewYaw,
            &AlsInputOverride,
            &AlsActualMotion,
            &bevy_als::als::character::AlsCmcAdapterRuntime,
            &AlsMovementParamsHandle,
            &AlsStandingGraphTelemetry,
            (
                &AlsMovementBaseDynamicsState,
                &bevy_als::als::dynamics::AlsGroundedDynamics,
                &bevy_tnua::TnuaController<bevy_als::als::control_scheme::AlsControlScheme>,
                Option<&bevy_als::als::dynamics::AlsLandingBrakingFriction>,
                &Transform,
            ),
        ),
        With<AlsCharacter>,
    >,
    params: Res<Assets<AlsMovementParams>>,
    transforms: bevy::transform::helper::TransformHelper,
) {
    if state.stairs_probe.index >= 6 {
        return;
    }
    let Some(character) = state.character else {
        return;
    };
    let Ok((
        position,
        velocity,
        locomotion,
        stance,
        desired_gait,
        gait,
        desired_mode,
        mode,
        yaw,
        input,
        motion,
        cmc,
        handle,
        telemetry,
        (dynamics, grounded, controller, braking, local),
    )) = characters.get(character)
    else {
        return;
    };
    let Ok(world) = transforms.compute_global_transform(character) else {
        return;
    };
    let Some(route) = state.stairs_probe.route else {
        return;
    };
    let Some(params) = params.get(handle.id()) else {
        return;
    };
    let index = state.stairs_probe.index;
    let stage = state.stairs_probe.stage;
    let now = state.game_time_seconds;
    let (start, end) = route.endpoints(index);
    let direction = (end - start).with_y(0.0).normalize();
    let previous = report
        .stairs_trace
        .last()
        .filter(|sample| sample.route_index == index);
    let max_speed = bevy_als::als::dynamics::max_allowed_speed(
        params,
        *stance,
        *mode,
        desired_gait.0,
        dynamics.relative_velocity,
        yaw.0,
    );
    let requested_displacement =
        bevy_als::als::dynamics::simulate_grounded_motion_with_braking_friction(
            dynamics.relative_velocity,
            controller.basis.desired_motion,
            max_speed,
            grounded.0,
            &params.cmc,
            braking.map_or(bevy_als::als::dynamics::BRAKING_FRICTION_FACTOR, |state| {
                state.factor
            }),
            cmc.simulation_delta_seconds,
        )
        .2;
    let sample = Sample {
        route_index: index,
        mesh: MESHES[index / 2],
        stage,
        elapsed_seconds: now,
        simulation_delta_seconds: cmc.simulation_delta_seconds,
        distance_m: (position.0 - start).dot(direction),
        route_length_m: (end - start).with_y(0.0).length(),
        input: input.0.move_dir,
        rotation_hotkey: input.0.rotation_mode_pressed,
        actual_gait: *gait,
        capsule_world_scale: world.to_scale_rotation_translation().0.y,
        capsule_local_scale: local.scale.y,
        frame: Frame {
            position: position.0,
            velocity: velocity.0,
            locomotion: *locomotion,
            stance: *stance,
            desired_gait: desired_gait.0,
            desired_mode: desired_mode.0,
            current_mode: *mode,
            view_yaw: yaw.0,
        },
        expected_speed: bevy_als::als::motion::gait_max_speed(params, stance, &desired_gait.0),
        requested_speed: (dynamics.world_velocity + dynamics.motor_boost)
            .with_y(0.0)
            .length(),
        requested_displacement_speed: requested_displacement.with_y(0.0).length()
            / cmc.simulation_delta_seconds,
        initial_speed: dynamics.relative_velocity.with_y(0.0).length(),
        animation_speed: motion.horizontal_speed,
        safe_move: cmc.grounded_move_start.is_some(),
        safe_move_prefix_speed: state.stairs_probe.move_prefix_speed,
        displacement_speed: previous.filter(|_| cmc.simulation_delta_seconds > 0.0).map(
            |previous| {
                (position.0 - previous.frame.position).with_y(0.0).length()
                    / cmc.simulation_delta_seconds
            },
        ),
        movement_state: telemetry.actual_movement_state,
        movement_pose_ready: telemetry.actual_movement_pose_ready,
        standing_pose_ready: telemetry.actual_standing_pose_ready,
        gait_started_seconds: previous
            .filter(|sample| {
                sample.frame.desired_gait == desired_gait.0
                    && sample.stage == Stage::Traverse
                    && stage == Stage::Traverse
            })
            .map_or(now, |sample| sample.gait_started_seconds),
    };
    if sample.stable() {
        *window = ManualAnimationWindowEvidence {
            label: Some(WINDOWS[index][sample.gait_index()]),
            settled: true,
            capture_source_window: true,
            capture_every_frame: true,
            requires_pre_rig_motion: true,
            requires_final_pose_motion: true,
            requires_non_reference_root_pose: true,
            requires_source_pose_chain: true,
            source_node: Some("AB_Als_Standing/AnimGraph"),
            source_state: Some("Grounded stair traversal"),
            source_pose_chain: generated_standing_source_pose_chain(),
            minimum_pre_rig_rotation_span_radians: 0.05,
            minimum_final_rotation_span_radians: 0.05,
            minimum_observation_seconds: 0.12,
            expected_runtime_state: Some(0),
            observed_runtime_state: telemetry.actual_movement_state,
            expected_direction: Some("Forward"),
            observed_direction: telemetry
                .actual_movement_state
                .and_then(movement_direction_from_fsm_index)
                .map(movement_direction_name),
            observed_parent_runtime_state: telemetry.actual_standing_state,
            observed_parent_transition_active: telemetry.actual_standing_transition_active,
            ..default()
        };
    }
    report.stairs_trace.push(sample);
}

pub(super) fn capture_move_prefix(
    mut state: ResMut<ManualIntegrationState>,
    characters: Query<
        (&Position, &bevy_als::als::character::AlsCmcAdapterRuntime),
        With<AlsCharacter>,
    >,
) {
    state.stairs_probe.move_prefix_speed = state
        .character
        .and_then(|character| characters.get(character).ok())
        .and_then(|(position, cmc)| {
            cmc.grounded_move_start
                .filter(|_| cmc.simulation_delta_seconds > 0.0)
                .map(|start| {
                    (position.0 - start).with_y(0.0).length() / cmc.simulation_delta_seconds
                })
        });
}
