use super::*;
use bevy_als::als::aim::AlsLastFreeRotationMode;
use bevy_als::als::control_scheme::AlsControlSchemeActionDiscriminant;
use bevy_als::als::transition::{StateTransitionEvent, StateValue};
use bevy_als::extensions::SceneQuery;
use crate::playground_collision::PlaygroundMeshColliderBuilt;
use avian3d::prelude::{Collisions, SpatialQueryFilter};
use bevy_tnua::TnuaController;

const MESHES: [&str; 3] = ["SM_Als_Stairs_01", "SM_Als_Stairs_02", "SM_Als_Stairs_03"];
const WINDOWS: [&str; 18] = [
    "uphill_01_run_0",
    "uphill_01_run_1",
    "uphill_01_run_2",
    "uphill_01_sprint_0",
    "uphill_01_sprint_1",
    "uphill_01_sprint_2",
    "uphill_02_run_0",
    "uphill_02_run_1",
    "uphill_02_run_2",
    "uphill_02_sprint_0",
    "uphill_02_sprint_1",
    "uphill_02_sprint_2",
    "uphill_03_run_0",
    "uphill_03_run_1",
    "uphill_03_run_2",
    "uphill_03_sprint_0",
    "uphill_03_sprint_1",
    "uphill_03_sprint_2",
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
enum Stage {
    #[default]
    Ready,
    Approach,
    Flight,
    Landing,
}

#[derive(Default)]
pub(super) struct Probe {
    index: usize,
    start: Option<Vec3>,
    direction: Vec3,
    stage: Stage,
    stage_started: f64,
    launch_y: Option<f32>,
    ready: bool,
    air_seen: bool,
    landed_seen: bool,
    supported_air_frames: u32,
    done: bool,
    pub(super) completed: [bool; 18],
    pub(super) failure: Option<String>,
}

impl Probe {
    fn begin(&mut self, stage: Stage, now: f64) {
        if !matches!(
            (self.stage, stage),
            (Stage::Approach, Stage::Flight) | (Stage::Flight, Stage::Approach)
        ) {
            self.stage_started = now;
        }
        self.stage = stage;
        info!(
            "[MANUAL-INTEGRATION][UPHILL] case={} stage={stage:?}",
            self.index
        );
    }

    fn gait(&self) -> Gait {
        if self.index % 6 < 3 {
            Gait::Running
        } else {
            Gait::Sprinting
        }
    }
    fn launch_distance(&self) -> f32 {
        1.0 - [0.55, 0.15, -0.22][self.index % 3]
    }

    fn retry_landed_jump(
        &mut self,
        now: f64,
        pressed: bool,
        transitions: impl Iterator<Item = (StateValue, StateValue)>,
    ) -> bool {
        use StateValue::LocomotionState as Mode;
        let mut launched = false;
        let mut landed = false;
        for (from, to) in transitions {
            match (from, to) {
                (Mode(LocomotionState::Grounded), Mode(LocomotionState::Air)) => launched = true,
                (Mode(LocomotionState::Air), Mode(LocomotionState::Grounded)) if launched => {
                    landed = true
                }
                _ => {}
            }
        }
        if self.stage == Stage::Flight && !self.air_seen && pressed && landed {
            self.launch_y = None;
            self.begin(Stage::Approach, now);
            return true;
        }
        false
    }

    fn record(
        &mut self,
        now: f64,
        mode: LocomotionState,
        position: Vec3,
        velocity: Vec3,
        supported: bool,
        graph: &LocomotionGraphSample,
        action_free: bool,
    ) {
        let air_weight = graph.outer_weights[1] + graph.outer_weights[2];
        self.ready = graph.outer == Some(0) && graph.outer_transition == Some(false);
        self.air_seen |=
            self.launch_y.is_some() && mode == LocomotionState::Air && air_weight > 0.05;
        if self.stage == Stage::Flight
            && self.air_seen
            && supported
            && mode == LocomotionState::Grounded
        {
            if position.y <= self.launch_y.unwrap() + 0.05 {
                self.failure = Some(format!(
                    "case={} landing was not above the launch",
                    self.index
                ));
            }
            self.begin(Stage::Landing, now);
        }
        self.supported_air_frames = if self.air_seen
            && supported
            && mode == LocomotionState::Air
            && velocity.y.abs() < 0.001
        {
            self.supported_air_frames + 1
        } else {
            0
        };
        if self.supported_air_frames >= 2 {
            self.failure = Some(format!(
                "case={} supported capsule remained Air",
                self.index
            ));
        }
        if self.stage == Stage::Landing {
            self.landed_seen |= graph.outer_weights[3] + graph.outer_weights[4] > 0.01;
            if mode != LocomotionState::Grounded {
                self.failure = Some(format!(
                    "case={} landed capsule returned to Air",
                    self.index
                ));
            }
            self.done = self.landed_seen
                && now - self.stage_started >= 0.35
                && air_weight <= 0.01
                && self.ready
                && action_free;
        }
    }
}

pub(super) fn drive(
    time: Res<Time>,
    mut state: ResMut<ManualIntegrationState>,
    mut phase: ResMut<ManualRuntimePhaseEvidence>,
    stairs: Query<(&Collider, &GlobalTransform, &PlaygroundMeshColliderBuilt)>,
    mut characters: Query<
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
            &AlsMovementParamsHandle,
            &TnuaController<bevy_als::als::control_scheme::AlsControlScheme>,
        ),
        With<AlsCharacter>,
    >,
    mut rotation_state: Query<(
        Option<&mut AlsRotationResponseState>,
        &mut AlsAimingYawConstraint,
        Option<&mut AlsRotationAnimationState>,
    )>,
    params: Res<Assets<AlsMovementParams>>,
) {
    let Some(character) = state.character else {
        return;
    };
    let now = state.game_time_seconds;
    if state.uphill_probe.index == 18 || state.uphill_probe.failure.is_some() {
        state.timed_out = state.uphill_probe.failure.is_some();
        begin_phase(&mut state, IntegrationPhase::Finish);
        return;
    }
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
        desired_gait,
        desired_mode,
        handle,
        controller,
    )) = characters.get_mut(character)
    else {
        return;
    };
    if state.uphill_probe.start.is_none() {
        let name = MESHES[state.uphill_probe.index / 6];
        let Some((collider, global, _)) = stairs
            .iter()
            .find(|(_, _, source)| source.mesh_base == name)
        else {
            return;
        };
        let Some(params) = params.get(handle.id()) else {
            return;
        };
        let Some(route) = stairs::Route::from_collider(
            collider,
            global,
            params.crouch.standing_capsule_half_height,
            Some,
        ) else {
            return;
        };
        let direction = (route.top - route.bottom).with_y(0.0).normalize();
        let start = route.bottom - direction * 0.25;
        reset_character(
            &mut transform,
            &mut position,
            rotation,
            &mut velocity,
            &mut yaw,
            &mut pitch,
            start + Vec3::Y * 0.05,
            world_forward_to_yaw(direction),
        );
        if let Ok((response, mut constraint, animation)) = rotation_state.get_mut(character) {
            reinitialize_fixture_rotation_response(response, &mut constraint, animation);
        }
        rebase_temporal_fixture(&mut state, &mut phase, "uphill_jump");
        state.uphill_probe.start = Some(start);
        state.uphill_probe.direction = direction;
        state.uphill_probe.begin(Stage::Ready, now);
        return;
    }
    let probe = &mut state.uphill_probe;
    input.0.sprint_held = probe.gait() == Gait::Sprinting;
    input.0.walk_pressed = probe.gait() == Gait::Running && desired_gait.0 == Gait::Walking;
    match probe.stage {
        Stage::Ready => {
            input.0.rotation_mode_pressed = desired_mode.0 != RotationMode::ViewDirection;
            input.0.crouch_pressed = *stance != Stance::Standing;
            if *locomotion == LocomotionState::Grounded
                && *stance == Stance::Standing
                && velocity.0.with_y(0.0).length() < 0.05
                && controller.action_flow_status().ongoing().is_none()
                && now - probe.stage_started >= 0.3
            {
                probe.begin(Stage::Approach, now);
            }
        }
        Stage::Approach => {
            input.0.move_dir = Vec2::Y;
            let progress = (position.0 - probe.start.unwrap()).dot(probe.direction);
            let next_progress =
                progress + velocity.0.dot(probe.direction).max(0.0) * time.delta_secs();
            if next_progress >= probe.launch_distance()
                && *locomotion == LocomotionState::Grounded
                && controller.action_flow_status().ongoing().is_none()
            {
                input.0.jump_pressed = true;
                probe.launch_y = Some(position.y);
                probe.begin(Stage::Flight, now);
            }
        }
        Stage::Flight => {
            input.0.move_dir = Vec2::Y;
        }
        Stage::Landing => {
            input.0.move_dir = Vec2::Y;
            if probe.done {
                probe.completed[probe.index] = true;
                let completed = probe.completed;
                let index = probe.index + 1;
                *probe = Probe {
                    completed,
                    index,
                    ..default()
                };
            }
        }
    }
    if probe.start.is_some() && now - probe.stage_started > 5.0 {
        probe.failure = Some(format!(
            "case={} stage={:?} no_progress_seconds={:.3} position={:?} mode={locomotion:?}",
            probe.index,
            probe.stage,
            now - probe.stage_started,
            position.0
        ));
    }
}

pub(super) fn observe(
    mut state: ResMut<ManualIntegrationState>,
    mut report: ResMut<ManualIntegrationGateReport>,
    mut window: ResMut<ManualAnimationWindowEvidence>,
    motion: MotionProbe,
    spatial: SceneQuery,
    collisions: Collisions,
    candidates: Query<bevy_als::als::motion::CharacterCollisionCandidates>,
    mut transitions: MessageReader<StateTransitionEvent>,
    characters: Query<
        (
            &Position,
            &LinearVelocity,
            &LocomotionState,
            &Gait,
            &AlsInputOverride,
            &AlsMovementParamsHandle,
            &Transform,
            &Stance,
            &TnuaController<bevy_als::als::control_scheme::AlsControlScheme>,
        ),
        With<AlsCharacter>,
    >,
    avatars: Query<&AnimatedSceneInstance, With<AlsCharacterAvatar>>,
    players: Query<&AnimationGraphPlayer>,
    params: Res<Assets<AlsMovementParams>>,
) {
    let (Some(character), Some(avatar)) = (state.character, state.avatar) else {
        return;
    };
    let Ok((position, velocity, mode, gait, input, handle, transform, stance, controller)) =
        characters.get(character)
    else {
        return;
    };
    let Ok(scene) = avatars.get(avatar) else {
        return;
    };
    let Ok(player) = players.get(scene.player_entity()) else {
        return;
    };
    let Some(params) = params.get(handle.id()) else {
        return;
    };
    let now = state.game_time_seconds;
    let probe = &mut state.uphill_probe;
    if probe.index >= 18 || probe.start.is_none() {
        return;
    }
    let graph = locomotion_graph_sample(player, &motion);
    let half = if *stance == Stance::Crouching {
        params.crouch.crouched_capsule_half_height
    } else {
        params.crouch.standing_capsule_half_height
    } * transform.scale.y;
    let radius = params.crouch.standing_capsule_radius * transform.scale.x.min(transform.scale.z);
    let floor = bevy_als::als::motion::find_ground_floor(
        &spatial,
        position.0,
        half,
        radius,
        &params.cmc,
        *mode,
        &SpatialQueryFilter::from_excluded_entities([character]),
        character,
        &candidates,
    );
    let contact = collisions.collisions_with(character).any(|pair| {
        pair.manifolds.iter().any(|manifold| {
            let normal = if pair.collider1 == character {
                -manifold.normal
            } else {
                manifold.normal
            };
            normal.y > 0.0
                && manifold
                    .points
                    .iter()
                    .any(|point| point.normal_impulse > 0.0)
        })
    });
    let supported = floor.is_some_and(|(center, impact, _)| {
        (position.y - center).abs() <= 0.024 && impact < position.y - half + radius
    }) && (*mode == LocomotionState::Grounded || contact);
    let launch_y = probe.launch_y;
    let action = controller.action_flow_status().ongoing();
    let jump_landed = input.0.jump_pressed
        && *mode == LocomotionState::Grounded
        && action == Some(AlsControlSchemeActionDiscriminant::Jump);
    let character_transitions = transitions
        .read()
        .filter(|event| event.entity == character)
        .map(|event| (event.from, event.to));
    let same_frame_landing = probe.retry_landed_jump(now, jump_landed, character_transitions);
    probe.record(
        now,
        *mode,
        position.0,
        velocity.0,
        supported,
        &graph,
        action.is_none(),
    );
    report.uphill_trace.push(serde_json::json!({
        "case_index":probe.index,"mesh":MESHES[probe.index/6],"stage":probe.stage,"time":now,
        "requested_gait":probe.gait(),"actual_gait":gait,"launch_distance":probe.launch_distance(),
        "launch_y":launch_y,"position":position.0,"velocity":velocity.0,"mode":mode,"same_frame_landing":same_frame_landing,
        "input":input.0.move_dir,"jump_pressed":input.0.jump_pressed,"supported":supported,"floor":floor.map(|(center, impact, _)| [center, impact]),
        "contact":contact,"controller_action":format!("{action:?}"),
        "air_seen":probe.air_seen,"landed_seen":probe.landed_seen,"done":probe.done,"graph":graph,
    }));
    if matches!(probe.stage, Stage::Flight | Stage::Landing) {
        *window = ManualAnimationWindowEvidence {
            label: Some(WINDOWS[probe.index]),
            settled: true,
            capture_source_window: true,
            capture_every_frame: true,
            requires_pre_rig_motion: true,
            requires_final_pose_motion: true,
            requires_non_reference_root_pose: true,
            requires_source_pose_chain: true,
            source_node: Some("AB_Als_Locomotion/AnimGraph"),
            source_state: Some("Uphill jump and landing"),
            source_pose_chain: vec![ManualAnimationPoseBoundary {
                label: "locomotion_output",
                graph: LOCOMOTION_ROOT,
                node: "Output Pose\nAnimGraph",
                output: "Pose",
                minimum_translation_span: 0.0,
                minimum_rotation_span_radians: 0.05,
            }],
            minimum_pre_rig_rotation_span_radians: 0.05,
            minimum_final_rotation_span_radians: 0.05,
            minimum_observation_seconds: 0.12,
            ..default()
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_frame_jump_landing_retries_within_the_original_watchdog() {
        use StateValue::LocomotionState as Mode;
        let ground = Mode(LocomotionState::Grounded);
        let air = Mode(LocomotionState::Air);
        let mut probe = Probe {
            stage: Stage::Ready,
            ..default()
        };
        probe.begin(Stage::Approach, 1.0);
        probe.begin(Stage::Flight, 1.5);
        probe.launch_y = Some(1.1);
        assert!(!probe.retry_landed_jump(1.5, true, [(air, ground)].into_iter()));
        assert!(!probe.retry_landed_jump(1.5, false, [(ground, air), (air, ground)].into_iter()));
        assert!(probe.retry_landed_jump(1.5, true, [(ground, air), (air, ground)].into_iter()));
        assert_eq!(probe.stage, Stage::Approach);
        assert_eq!(probe.launch_y, None);
        assert!(!probe.air_seen);
        probe.begin(Stage::Flight, 1.6);
        assert_eq!(probe.stage_started, 1.0);
    }

    #[test]
    fn coarse_step_jump_input_does_not_wait_until_after_the_launch_window() {
        use bevy::ecs::system::RunSystemOnce;
        for (progress, speed, expected) in
            [(0.79, 3.75, true), (0.6, 3.75, false), (0.79, 0.0, false)]
        {
            let mut world = World::new();
            let mut clock = Time::<()>::default();
            clock.advance_by(std::time::Duration::from_secs_f32(0.046875));
            world.insert_resource(clock);
            let mut params = Assets::<AlsMovementParams>::default();
            let handle = params.add(AlsMovementParams::default());
            world.insert_resource(params);
            let start = Vec3::new(0.0, 0.9, 1.0);
            let position = start + Vec3::NEG_Z * progress;
            let actor = world
                .spawn((
                    AlsCharacter,
                    AlsInputOverride::default(),
                    Transform::from_translation(position),
                    Position(position),
                    LinearVelocity(Vec3::NEG_Z * speed),
                    AlsViewYaw(0.0),
                    AlsViewPitch(0.0),
                    Stance::Standing,
                    LocomotionState::Grounded,
                    AlsDesiredGait(Gait::Running),
                    AlsLastFreeRotationMode(RotationMode::ViewDirection),
                    AlsMovementParamsHandle(handle),
                    TnuaController::<bevy_als::als::control_scheme::AlsControlScheme>::default(),
                ))
                .id();
            world.insert_resource(ManualIntegrationState {
                character: Some(actor),
                phase: IntegrationPhase::UphillJump,
                game_time_seconds: 0.5,
                uphill_probe: Probe {
                    index: 13,
                    start: Some(start),
                    direction: Vec3::NEG_Z,
                    stage: Stage::Approach,
                    stage_started: 0.4,
                    ..default()
                },
                ..default()
            });
            world.init_resource::<ManualRuntimePhaseEvidence>();
            world.run_system_once(drive).unwrap();
            assert_eq!(
                world.get::<AlsInputOverride>(actor).unwrap().0.jump_pressed,
                expected,
                "Uphill input missed its declared launch window: progress={progress} speed={speed}"
            );
        }
    }

    #[test]
    fn uphill_probe_rejects_supported_air_and_waits_for_real_graph_recovery() {
        let air = LocomotionGraphSample {
            outer: Some(1),
            outer_transition: Some(false),
            outer_weights: [0.0, 1.0, 0.0, 0.0, 0.0],
            ..default()
        };
        let land = LocomotionGraphSample {
            outer: Some(4),
            outer_transition: Some(false),
            outer_weights: [0.0, 0.0, 0.0, 0.0, 1.0],
            ..default()
        };
        let grounded = LocomotionGraphSample {
            outer: Some(0),
            outer_transition: Some(false),
            outer_weights: [1.0, 0.0, 0.0, 0.0, 0.0],
            ..default()
        };
        let make = || Probe {
            stage: Stage::Flight,
            launch_y: Some(0.9),
            ..default()
        };
        let mut stuck = make();
        for time in [0.1, 0.2] {
            stuck.record(
                time,
                LocomotionState::Air,
                Vec3::Y * 1.5,
                Vec3::ZERO,
                true,
                &air,
                false,
            );
        }
        assert!(
            stuck
                .failure
                .as_deref()
                .unwrap()
                .contains("supported capsule remained Air")
        );
        let mut probe = make();
        probe.record(
            0.1,
            LocomotionState::Air,
            Vec3::Y * 1.5,
            Vec3::NEG_Y,
            false,
            &air,
            false,
        );
        probe.record(
            0.15,
            LocomotionState::Grounded,
            Vec3::Y * 1.5,
            Vec3::ZERO,
            false,
            &land,
            false,
        );
        assert_eq!(probe.stage, Stage::Flight);
        probe.record(
            0.2,
            LocomotionState::Grounded,
            Vec3::Y * 1.5,
            Vec3::ZERO,
            true,
            &land,
            false,
        );
        assert_eq!(probe.stage, Stage::Landing);
        probe.record(
            0.4,
            LocomotionState::Grounded,
            Vec3::Y * 1.5,
            Vec3::ZERO,
            true,
            &grounded,
            true,
        );
        assert!(!probe.done);
        let lingering = LocomotionGraphSample {
            outer: Some(0),
            outer_transition: Some(false),
            outer_weights: [0.5, 0.5, 0.0, 0.0, 0.0],
            ..default()
        };
        probe.record(
            0.6,
            LocomotionState::Grounded,
            Vec3::Y * 1.5,
            Vec3::ZERO,
            true,
            &lingering,
            true,
        );
        assert!(!probe.done);
        probe.record(
            0.7,
            LocomotionState::Grounded,
            Vec3::Y * 1.5,
            Vec3::ZERO,
            true,
            &grounded,
            false,
        );
        assert!(!probe.done);
        probe.record(
            0.8,
            LocomotionState::Grounded,
            Vec3::Y * 1.5,
            Vec3::ZERO,
            true,
            &grounded,
            true,
        );
        assert!(probe.done && probe.failure.is_none());
    }

    #[test]
    fn uphill_driver_uses_actual_jump_input_after_the_ready_and_distance_conditions() {
        use bevy::ecs::system::RunSystemOnce;
        let mut world = World::new();
        world.insert_resource(Time::<()>::default());
        let mut params = Assets::<AlsMovementParams>::default();
        let handle = params.add(AlsMovementParams::default());
        world.insert_resource(params);
        let start = Vec3::new(0.0, 0.9, 1.0);
        let character = world
            .spawn((
                AlsCharacter,
                AlsInputOverride::default(),
                Transform::from_translation(start),
                Position(start),
                LinearVelocity::ZERO,
                AlsViewYaw(0.0),
                AlsViewPitch(0.0),
                Stance::Standing,
                LocomotionState::Grounded,
                AlsDesiredGait(Gait::Running),
                AlsLastFreeRotationMode(RotationMode::ViewDirection),
                AlsMovementParamsHandle(handle),
                TnuaController::<bevy_als::als::control_scheme::AlsControlScheme>::default(),
            ))
            .id();
        world.insert_resource(ManualIntegrationState {
            character: Some(character),
            phase: IntegrationPhase::UphillJump,
            game_time_seconds: 0.1,
            uphill_probe: Probe {
                start: Some(start),
                direction: Vec3::NEG_Z,
                ..default()
            },
            ..default()
        });
        world.init_resource::<ManualRuntimePhaseEvidence>();
        world.run_system_once(drive).unwrap();
        assert_eq!(
            world.get::<AlsInputOverride>(character).unwrap().0.move_dir,
            Vec2::ZERO
        );
        assert_eq!(
            world
                .resource::<ManualIntegrationState>()
                .uphill_probe
                .stage,
            Stage::Ready
        );
        world
            .resource_mut::<ManualIntegrationState>()
            .game_time_seconds = 0.4;
        world.run_system_once(drive).unwrap();
        assert_eq!(
            world
                .resource::<ManualIntegrationState>()
                .uphill_probe
                .stage,
            Stage::Approach
        );
        world.get_mut::<Position>(character).unwrap().z = 0.4;
        world.run_system_once(drive).unwrap();
        assert!(
            world
                .get::<AlsInputOverride>(character)
                .unwrap()
                .0
                .jump_pressed
        );
        assert_eq!(
            world
                .resource::<ManualIntegrationState>()
                .uphill_probe
                .stage,
            Stage::Flight
        );
        world
            .resource_mut::<ManualIntegrationState>()
            .uphill_probe
            .stage = Stage::Landing;
        world
            .get_mut::<AlsInputOverride>(character)
            .unwrap()
            .0
            .reset();
        world.run_system_once(drive).unwrap();
        assert_eq!(
            world.get::<AlsInputOverride>(character).unwrap().0.move_dir,
            Vec2::Y
        );
        world
            .resource_mut::<ManualIntegrationState>()
            .uphill_probe
            .stage = Stage::Flight;
        world
            .resource_mut::<ManualIntegrationState>()
            .game_time_seconds = 6.0;
        world.run_system_once(drive).unwrap();
        assert!(
            world
                .resource::<ManualIntegrationState>()
                .uphill_probe
                .failure
                .as_deref()
                .unwrap()
                .contains("no_progress_seconds")
        );
        world.run_system_once(drive).unwrap();
        assert!(world.resource::<ManualIntegrationState>().timed_out);
        assert_eq!(
            world.resource::<ManualIntegrationState>().phase,
            IntegrationPhase::Finish
        );
        for index in 0..18 {
            let probe = Probe { index, ..default() };
            assert_eq!(
                probe.gait(),
                if index % 6 < 3 {
                    Gait::Running
                } else {
                    Gait::Sprinting
                }
            );
            assert!((probe.launch_distance() - [0.45, 0.85, 1.22][index % 3]).abs() < 1e-5);
        }
    }
}
