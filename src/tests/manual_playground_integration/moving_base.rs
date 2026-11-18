use super::*;
use bevy_als::als::dynamics::{AlsComponentVelocity, AlsMovementBase, component_velocity};
use bevy_als::als::mantle::{
    AlsMantleTargetBase, AlsMantleTargetPermission, MantleDetectionInput, detect_mantle,
};
use bevy_als::als::motion::AlsActualVelocity;
use bevy_als::extensions::SceneQuery;
use crate::playground::{PlaygroundKinematic, PlaygroundKinematicKind};
use avian3d::prelude::{Collider, ColliderOf, SpatialQueryFilter};

const POSITION_TOLERANCE: f32 = 0.001;

fn object(scenario: IntegrationScenario) -> &'static str {
    if matches!(
        scenario,
        IntegrationScenario::RotatingBase | IntegrationScenario::Footprint
    ) {
        "Rotating:BP_SimpleRotatingObject_2"
    } else {
        "Moving:BP_SimpleMovingObject2"
    }
}

fn motion_sample(kind: PlaygroundKinematicKind, global: &GlobalTransform) -> (f32, f32, Vec3) {
    match kind {
        PlaygroundKinematicKind::MoveSpline {
            duration,
            start,
            end,
        } => (duration * 2.0, start.distance(end), global.translation()),
        PlaygroundKinematicKind::RotateWorldTime { rate_yzx_radians } => (
            (std::f64::consts::TAU / rate_yzx_radians.x.abs()) as f32,
            2.0,
            global.transform_point(Vec3::Z),
        ),
    }
}

fn mantle_approach(
    kind: PlaygroundKinematicKind,
    global: &GlobalTransform,
    extent: Vec3,
    radius: f32,
) -> Vec3 {
    let (center, reach) = match kind {
        PlaygroundKinematicKind::MoveSpline { start, end, .. } => (start.lerp(end, 0.5), extent.z),
        PlaygroundKinematicKind::RotateWorldTime { .. } => {
            (global.translation(), extent.x.hypot(extent.z))
        }
    };
    center + Vec3::Z * (reach + radius + 0.1)
}

fn moving_mantle_candidate(kind: PlaygroundKinematicKind, velocity: Vec3) -> bool {
    matches!(kind, PlaygroundKinematicKind::RotateWorldTime { .. }) || velocity.length() > 1.0
}

fn exit_direction_ready(
    kind: PlaygroundKinematicKind,
    position: Vec3,
    velocity: Vec3,
    half_extents: Vec3,
) -> bool {
    match kind {
        PlaygroundKinematicKind::MoveSpline { start, end, .. } => {
            let axis = (end - start).normalize_or_zero();
            (end - start).dot(velocity) >= 0.0
                && position.distance(end) <= half_extents.dot(axis.abs())
        }
        PlaygroundKinematicKind::RotateWorldTime { .. } => true,
    }
}

pub(super) fn install(app: &mut App) {
    app.add_systems(
        Update,
        drive
            .run_if(|state: Res<ManualIntegrationState>| {
                state.phase == IntegrationPhase::MovingBase
            })
            .after(crate::playground_collision::playground_kinematic_velocity_bridge)
            .before(bevy_als::als::view::AlsViewInputSet)
            .before(bevy_als::als::input_request::input_state_request_system),
    )
    .add_systems(
        PostUpdate,
        observe
            .run_if(|state: Res<ManualIntegrationState>| {
                state.phase == IntegrationPhase::MovingBase
            })
            .after(AlsCameraSet)
            .after(AlsRotationResponseSet)
            .after(FootIkApplySet::Telemetry)
            .after(observe_manual_integration)
            .in_set(ManualIntegrationObservationSet),
    );
}

const TAKEOFF_DISTANCE: f32 = 0.45;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub(super) enum Stage {
    #[default]
    Place,
    Settle,
    Ride,
    Exit,
    Flight,
    PrepareMantle,
    WaitMantle,
    Mantle,
    AfterMantle,
    Done,
}

#[derive(Default)]
pub(super) struct Probe {
    stage: Stage,
    since: Option<f64>,
    anchor: Option<Vec3>,
    ride_rotation: Option<Quat>,
    minimum: Vec3,
    maximum: Vec3,
    ride_since: Option<f64>,
    mantle_start: Option<Vec3>,
    mantle_base_before_physics: Option<AlsMantleTargetBase>,
    step_released: [bool; 2],
    step_unlocked: [bool; 2],
    pub(super) steps: [bool; 2],
    pub(super) mantle_requested: bool,
    pub(super) ride: bool,
    pub(super) air: bool,
    pub(super) landing: bool,
    pub(super) moving_mantle: bool,
    pub(super) after_mantle: bool,
    pub(super) failure: Option<String>,
}

impl Probe {
    fn observe_ride_rotation(&mut self, rotation: Quat) {
        let initial = *self.ride_rotation.get_or_insert(rotation);
        if rotation.angle_between(initial) > 0.001 {
            self.failure = Some(
                "Rotating base changed the source world-facing character orientation".to_owned(),
            );
        }
    }

    fn begin(&mut self, stage: Stage, now: f64) {
        self.stage = stage;
        self.since = Some(now);
        info!("[MANUAL-INTEGRATION][MOVING-BASE] stage={stage:?}");
    }

    fn command(
        &mut self,
        now: f64,
        exit_distance: f32,
        mantle_candidate: bool,
    ) -> crate::input::AlsInputCommand {
        let mut input = crate::input::AlsInputCommand::default();
        match self.stage {
            Stage::Exit => {
                input.move_dir = Vec2::NEG_Y;
                if exit_distance >= TAKEOFF_DISTANCE {
                    input.jump_pressed = true;
                    input.jump_held = true;
                    self.begin(Stage::Flight, now);
                }
            }
            Stage::Flight => {
                input.move_dir = Vec2::NEG_Y;
                input.jump_held = now - self.since.unwrap_or(now) < 0.4;
            }
            Stage::WaitMantle if mantle_candidate && now - self.since.unwrap_or(now) >= 0.35 => {
                input.move_dir = Vec2::Y;
                input.jump_pressed = true;
                self.mantle_requested = true;
                self.begin(Stage::Mantle, now);
            }
            _ => {}
        }
        input
    }

    fn observe_steps(
        &mut self,
        feet: Option<&AlsFootLockState>,
        playback: Option<&AlsTransitionPlayback>,
        slot: Option<&AlsRootTransitionSlotRuntime>,
        scale: f32,
    ) {
        for side in 0..2 {
            let foot = feet.map(|feet| if side == 0 { &feet.left } else { &feet.right });
            let current = playback.filter(|playback| {
                playback.kind == AlsTransitionKind::StandingDynamic && playback.index == side as f32
            });
            if let Some(playback) = current.filter(|playback| {
                playback.active
                    && slot.is_some_and(|slot| slot.active && slot.pose_ready && slot.weight > 0.05)
            }) {
                if playback.start_time != 0.0
                    || playback.play_rate != 1.5
                    || playback.blend_in != 0.2
                    || playback.blend_out != 0.2
                {
                    self.failure = Some(
                        "Rotating base dynamic transition playback differs from source".to_owned(),
                    );
                }
                self.step_unlocked[side] |= foot.is_some_and(|foot| foot.lock_amount < 0.1);
                self.step_released[side] |=
                    self.step_unlocked[side] && step_replacement_allowed(playback);
            }
            self.steps[side] |=
                standing_dynamic_foot_restored(self.step_released[side], foot, scale);
        }
    }

    fn riding(
        &mut self,
        local: Vec3,
        platform: Vec3,
        own_speed: f32,
        same_base: bool,
        now: f64,
        period: f32,
        span: f32,
    ) -> bool {
        let Some(anchor) = self.anchor else {
            self.anchor = Some(local);
            self.minimum = platform;
            self.maximum = platform;
            self.ride_since = Some(now);
            return false;
        };
        self.minimum = self.minimum.min(platform);
        self.maximum = self.maximum.max(platform);
        if !same_base
            || (local - anchor).with_y(0.0).length() > POSITION_TOLERANCE
            || own_speed > 0.01
        {
            self.failure = Some(format!(
                "stage={:?} base_matches={same_base} relative_error={} own_speed={own_speed}",
                self.stage,
                (local - anchor).with_y(0.0).length()
            ));
        }
        now - self.ride_since.unwrap_or(now) >= period as f64
            && (self.maximum - self.minimum).length() >= span * 0.9
    }

    fn watchdog(&mut self, now: f64, period: f32) {
        let limit = if matches!(self.stage, Stage::Ride | Stage::AfterMantle) {
            period as f64 + 4.0
        } else {
            8.0
        };
        let elapsed = now - *self.since.get_or_insert(now);
        if elapsed > limit && self.stage != Stage::Done {
            self.failure = Some(format!(
                "exit_reason=stage_progress_timeout stage={:?} no_progress_seconds={elapsed:.3} limit_seconds={limit:.3}",
                self.stage
            ));
        }
    }
}

fn step_replacement_allowed(playback: &AlsTransitionPlayback) -> bool {
    let name = bevy_als::stop_link::source_transition_sequence(playback);
    bevy_als::standing_source::MONTAGES[name]
        .curves
        .sample("AllowTransitions", playback.clip_time)
        .is_some_and(|value| value.abs() <= 1e-5)
}

pub(super) fn drive(
    mut commands: Commands,
    mut state: ResMut<ManualIntegrationState>,
    mut phase: ResMut<ManualRuntimePhaseEvidence>,
    time: Res<Time>,
    platforms: Query<
        (Entity, &Name, &PlaygroundKinematic, &GlobalTransform),
        Without<AlsCharacter>,
    >,
    colliders: Query<(Entity, &Collider, &ColliderOf, &GlobalTransform), Without<AlsCharacter>>,
    characters: Query<
        (
            &Transform,
            &Position,
            &LinearVelocity,
            &AlsViewYaw,
            &AlsViewPitch,
            &Stance,
            &LocomotionState,
            &AlsMovementParamsHandle,
        ),
        With<AlsCharacter>,
    >,
    params: Res<Assets<AlsMovementParams>>,
    spatial: SceneQuery,
    candidates: Query<bevy_als::als::motion::CharacterCollisionCandidates>,
    velocities: Query<
        (
            Option<&RigidBody>,
            Option<&LinearVelocity>,
            Option<&AlsComponentVelocity>,
        ),
        Without<AlsCharacter>,
    >,
    permissions: Query<Option<&AlsMantleTargetPermission>>,
    owners: Query<&ColliderOf>,
) {
    let Some(character) = state.character else {
        return;
    };
    let now = time.elapsed_secs_f64();
    state.game_time_seconds = now;
    let object = object(state.scenario);
    let platform = platforms
        .iter()
        .find(|(_, name, _, _)| name.as_str() == object);
    let period = platform.map_or(3.5, |(_, _, path, global)| {
        motion_sample(path.kind, global).0
    });
    state.moving_base_probe.watchdog(now, period);
    if state.moving_base_probe.failure.is_some() || state.moving_base_probe.stage == Stage::Done {
        state.timed_out = state.moving_base_probe.failure.is_some();
        begin_phase(&mut state, IntegrationPhase::Finish);
        return;
    }
    state.moving_base_probe.mantle_base_before_physics = None;
    let Some((body, _, path, global)) = platform else {
        return;
    };
    let Some((collider, shape, _, collider_world)) =
        colliders.iter().find(|(_, _, owner, _)| owner.body == body)
    else {
        return;
    };
    state.moving_base_probe.mantle_base_before_physics = Some(AlsMantleTargetBase {
        entity: collider,
        translation: collider_world.translation(),
        rotation: collider_world.rotation(),
        scale: collider_world.scale(),
        relative_location: true,
    });
    let Ok((transform, position, velocity, yaw, pitch, stance, mode, handle)) =
        characters.get(character)
    else {
        return;
    };
    let Some(params) = params.get(handle.id()) else {
        return;
    };
    let half = params.crouch.standing_capsule_half_height * transform.scale.y;
    let radius = params.crouch.standing_capsule_radius * transform.scale.x.min(transform.scale.z);
    let bounds = shape.shape_scaled().compute_local_aabb();
    let spawn = match state.moving_base_probe.stage {
        Stage::Place => Some(
            global.translation()
                + Vec3::Y * (bounds.maxs.y + half + 0.024)
                + if matches!(
                    state.scenario,
                    IntegrationScenario::RotatingBase | IntegrationScenario::Footprint
                ) {
                    global.rotation() * Vec3::X * 0.65
                } else {
                    Vec3::ZERO
                },
        ),
        Stage::PrepareMantle => {
            let horizontal = mantle_approach(path.kind, global, bounds.half_extents(), radius);
            bevy_als::als::motion::find_ground_floor(
                &spatial,
                horizontal + Vec3::Y * (half + 0.024),
                half,
                radius,
                &params.cmc,
                *mode,
                &SpatialQueryFilter::from_excluded_entities([character, body, collider]),
                character,
                &candidates,
            )
            .map(|(floor, _, _)| horizontal.with_y(floor + 0.024))
        }
        _ => None,
    };
    if let Some(spawn) = spawn {
        let (mut transform, mut position, mut velocity, mut yaw, mut pitch) =
            (*transform, *position, *velocity, *yaw, *pitch);
        let mut rotation = Rotation(transform.rotation);
        reset_character_with_rotation(
            &mut transform,
            &mut position,
            Some(&mut rotation),
            &mut velocity,
            &mut yaw,
            &mut pitch,
            spawn,
            0.0,
        );
        commands.entity(character).insert((
            transform,
            position,
            rotation,
            velocity,
            yaw,
            pitch,
            AlsMovementBase::default(),
            AlsMovementBaseDynamicsState::default(),
            AlsRotationResponseState::default(),
            AlsAimingYawConstraint::default(),
            AlsRotationAnimationState::default(),
            AlsInputOverride::default(),
        ));
        rebase_temporal_fixture(&mut state, &mut phase, "moving_base_fixture");
        let next = if state.moving_base_probe.stage == Stage::Place {
            Stage::Settle
        } else {
            Stage::WaitMantle
        };
        state.moving_base_probe.begin(next, now);
        return;
    }
    let candidate = state.moving_base_probe.stage == Stage::WaitMantle
        && *mode == LocomotionState::Grounded
        && velocities.get(body).is_ok_and(|(_, velocity, _)| {
            velocity.is_some_and(|velocity| moving_mantle_candidate(path.kind, velocity.0))
        })
        && detect_mantle(
            &spatial,
            character,
            MantleDetectionInput {
                origin: position.0,
                actor_yaw: world_forward_to_yaw(*transform.forward()),
                velocity_yaw: bevy_als::als::motion::locomotion_movement_flags(
                    false,
                    velocity.0.with_y(0.0).length(),
                )
                .0
                .then_some(world_forward_to_yaw(velocity.0)),
                input_yaw: Some(0.0),
                locomotion: *mode,
                stance: *stance,
                overlay: OverlayState::Default,
                capsule_half_height: half,
                capsule_radius: radius,
                capsule_scale: transform.scale.y,
            },
            &velocities,
            &permissions,
            &owners,
        )
        .intent
        .is_some_and(|intent| intent.target_entity == collider);
    let local = global.affine().inverse().transform_point3(position.0);
    let exit_distance = state
        .moving_base_probe
        .anchor
        .map_or(0.0, |anchor| (local - anchor).with_y(0.0).length());
    let input = state
        .moving_base_probe
        .command(now, exit_distance, candidate);
    commands.entity(character).insert(AlsInputOverride(input));
    phase.phase = "MovingBase".to_owned();
    phase.elapsed_seconds = (now - state.moving_base_probe.since.unwrap_or(now)) as f32;
}

pub(super) fn observe(
    mut state: ResMut<ManualIntegrationState>,
    mut report: ResMut<ManualIntegrationGateReport>,
    mut window: ResMut<ManualAnimationWindowEvidence>,
    motion: MotionProbe,
    assets: Res<bevy_als::mantle_drive::MantleAssets>,
    platforms: Query<
        (
            Entity,
            &Name,
            &PlaygroundKinematic,
            &GlobalTransform,
            &LinearVelocity,
            &RigidBody,
            Option<&AlsComponentVelocity>,
        ),
        Without<AlsCharacter>,
    >,
    colliders: Query<(Entity, &Collider, &ColliderOf), Without<AlsCharacter>>,
    characters: Query<
        (
            &Position,
            &Transform,
            &LocomotionState,
            &AlsMovementBase,
            &AlsActualVelocity,
            &LinearVelocity,
            &AlsMantleState,
            Option<&AlsNotifyLocomotionAction>,
            &AlsInputOverride,
            Option<&AlsFootLockState>,
            Option<&AlsTransitionMontageStack>,
            Option<&AlsRootTransitionSlotRuntime>,
        ),
        With<AlsCharacter>,
    >,
    avatars: Query<&AnimatedSceneInstance, With<AlsCharacterAvatar>>,
    players: Query<&AnimationGraphPlayer>,
) {
    let (Some(character), Some(avatar)) = (state.character, state.avatar) else {
        return;
    };
    let object = object(state.scenario);
    let rotating = matches!(
        state.scenario,
        IntegrationScenario::RotatingBase | IntegrationScenario::Footprint
    );
    let footprints = state.scenario == IntegrationScenario::Footprint;
    let Some((body, _, path, global, platform_velocity, kind, reported_velocity)) = platforms
        .iter()
        .find(|(_, name, _, _, _, _, _)| name.as_str() == object)
    else {
        return;
    };
    let Some((collider, shape, _)) = colliders.iter().find(|(_, _, owner)| owner.body == body)
    else {
        return;
    };
    let Ok((
        position,
        transform,
        mode,
        base,
        own,
        velocity,
        mantle,
        action,
        input,
        feet,
        playback,
        slot,
    )) = characters.get(character)
    else {
        return;
    };
    let transition_stack = playback;
    let playback = transition_stack.and_then(AlsTransitionMontageStack::latest);
    let Ok(scene) = avatars.get(avatar) else {
        return;
    };
    let Ok(player) = players.get(scene.player_entity()) else {
        return;
    };
    let graph = locomotion_graph_sample(player, &motion);
    let now = state.game_time_seconds;
    let local = global.affine().inverse().transform_point3(position.0);
    let same_base = base.entity == Some(collider);
    let (period, span, motion_point) = motion_sample(path.kind, global);
    let stage = state.moving_base_probe.stage;
    let mantle_base_before_physics = state.moving_base_probe.mantle_base_before_physics;
    let mut expected = None;
    if mantle.root_motion_ready
        && *mode == LocomotionState::Custom
        && let Some(base) = mantle_base_before_physics
        && let Some(clip) = motion.clips.get(assets.clip(mantle.kind))
        && let Some(skeleton) = motion.skeletons.get(&clip.skeleton)
        && let Some(root) =
            bevy_als::mantle_drive::sample_mantle_root(clip, skeleton, mantle.clip_time())
    {
        let mut reference = mantle.clone();
        reference.refresh_target_base(Some(base));
        expected = Some(reference.target_actor_transform_at(mantle.time, root));
    }
    let probe = &mut state.moving_base_probe;
    match stage {
        Stage::Settle
            if *mode == LocomotionState::Grounded
                && same_base
                && own.horizontal_speed < 0.01
                && now - probe.since.unwrap_or(now) >= 0.35 =>
        {
            probe.begin(Stage::Ride, now)
        }
        Stage::Ride => {
            if rotating {
                probe.observe_ride_rotation(transform.rotation);
                probe.observe_steps(feet, playback, slot, transform.scale.y);
            }
            if probe.riding(
                local,
                motion_point,
                own.horizontal_speed,
                same_base,
                now,
                period,
                span,
            ) && exit_direction_ready(
                path.kind,
                global.translation(),
                platform_velocity.0,
                shape.shape_scaled().compute_local_aabb().half_extents(),
            ) && (!rotating || probe.steps.iter().all(|value| *value))
            {
                probe.ride = true;
                probe.begin(if footprints { Stage::Done } else { Stage::Exit }, now);
            }
        }
        Stage::Flight => {
            probe.air |= *mode == LocomotionState::Air
                && base.entity.is_none()
                && graph.outer_weights[1] + graph.outer_weights[2] > 0.05;
            if probe.air
                && *mode == LocomotionState::Grounded
                && base.entity.is_some()
                && !same_base
                && graph.outer == Some(0)
                && graph.outer_transition == Some(false)
            {
                probe.landing = true;
                probe.begin(Stage::PrepareMantle, now);
            }
        }
        Stage::Mantle => {
            if let Some(expected) = expected {
                let initial = *probe.mantle_start.get_or_insert(motion_point);
                let error = position.0.distance(expected.translation);
                if error > POSITION_TOLERANCE
                    || transform.rotation.angle_between(expected.rotation) > 0.001
                {
                    probe.failure = Some(format!(
                        "moving_mantle_target_error={error} position={:?} expected={:?}",
                        position.0, expected.translation
                    ));
                }
                probe.moving_mantle |= motion_point.distance(initial) > 0.25 && mantle.time > 0.2;
            }
            if probe.moving_mantle
                && !mantle.active
                && !mantle.root_motion_ready
                && *mode == LocomotionState::Grounded
                && action == Some(&AlsNotifyLocomotionAction::None)
            {
                if !same_base {
                    probe.failure =
                        Some("Mantle did not return to the current source object".to_owned());
                }
                probe.anchor = None;
                probe.ride_since = None;
                probe.begin(Stage::AfterMantle, now);
            }
        }
        Stage::AfterMantle => {
            if now - probe.since.unwrap_or(now) >= 0.35
                && own.horizontal_speed < 0.01
                && probe.riding(
                    local,
                    motion_point,
                    own.horizontal_speed,
                    same_base,
                    now,
                    period,
                    span,
                )
            {
                probe.after_mantle = true;
                probe.begin(Stage::Done, now);
            }
        }
        _ => {}
    }
    let mut snapshot = serde_json::json!({
        "stage":stage,"time":now,"body":format!("{body:?}"),"collider":format!("{collider:?}"),"object":object,
        "period":period,"span":span,"platform":global.translation(),"platform_rotation":global.rotation(),
        "platform_velocity":platform_velocity.0,"position":position.0,"rotation":transform.rotation,"local":local,
        "target_velocity":component_velocity(kind, platform_velocity, reported_velocity),
        "velocity":velocity.0,"own_velocity":own.world_velocity,"mode":mode,"base_matches":same_base,"has_base":base.entity.is_some(),
        "jump_pressed":input.0.jump_pressed,"input":input.0.move_dir,"mantle_active":mantle.active,"root_motion_ready":mantle.root_motion_ready,
        "mantle_time":mantle.time,"expected_position":expected.map(|value| value.translation),
        "expected_rotation":expected.map(|value| value.rotation),"graph":graph,
        "motion_point":motion_point,
        "feet":feet.map(|feet| [&feet.left, &feet.right].map(|foot| serde_json::json!({
            "lock_amount":foot.lock_amount,"target":foot.target_location,"lock":foot.lock_location
        }))),
        "step":playback.map(|playback| serde_json::json!({"kind":format!("{:?}", playback.kind),"side":playback.index,
            "active":playback.active,"clip_time":playback.clip_time,"duration":playback.duration,"weight":playback.weight,
            "play_rate":playback.play_rate,"start_time":playback.start_time,"blend_in":playback.blend_in,"blend_out":playback.blend_out})),
        "transitions":transition_stack.map(|stack| stack.entries.iter().filter(|entry| entry.active).map(|entry| serde_json::json!({
            "side":entry.index,"elapsed":entry.elapsed,"clip_time":entry.clip_time,"weight":entry.weight,
            "blend_in":entry.blend_in,"stop_elapsed":entry.stop_elapsed,"stop_start_weight":entry.stop_start_weight,"stop_duration":entry.stop_duration
        })).collect::<Vec<_>>()).unwrap_or_default(),
        "slot":slot.map(|slot| serde_json::json!({"active":slot.active,"pose_ready":slot.pose_ready,"weight":slot.weight})),
    });
    if let Some(base) = mantle_base_before_physics.filter(|_| expected.is_some()) {
        snapshot["mantle_base_before_physics"] = serde_json::json!({
            "translation": base.translation,
            "rotation": base.rotation,
            "scale": base.scale,
        });
    }
    snapshot["view"] = motion.rendered_views.iter().find(|(camera, _, owner)| camera.is_active && owner.character == character)
        .map(|(camera, transform, _)| serde_json::json!({
            "position":transform.translation,"rotation":transform.rotation,"viewport":camera.logical_viewport_size(),
            "foot_pixels":feet.map(|feet| [&feet.left, &feet.right].map(|foot|
                camera.world_to_viewport(&GlobalTransform::from(*transform), foot.target_location.as_vec3()).ok())),
        })).unwrap_or(serde_json::Value::Null);
    snapshot["takeoff_ready"] = serde_json::Value::Bool(
        probe.stage == Stage::Exit
            && probe
                .anchor
                .is_some_and(|anchor| (local - anchor).with_y(0.0).length() >= TAKEOFF_DISTANCE),
    );
    report.moving_base_trace.push(snapshot);
    if rotating
        && stage == Stage::Ride
        && let Some(playback) =
            playback.filter(|playback| playback.kind == AlsTransitionKind::StandingDynamic)
    {
        let side = usize::from(playback.index > 0.5);
        let ready = slot.is_some_and(|slot| slot.active && slot.pose_ready && slot.weight > 0.05);
        let completed = probe.steps[side] && step_replacement_allowed(playback);
        let chain = vec![ManualAnimationPoseBoundary {
            label: "root_transition_slot",
            graph: "ALS/Character/AB_Als/AnimGraph.animgraph.ron",
            node: "Slot 'Transition'\nGroup 'Grounded'",
            output: "Pose",
            minimum_translation_span: 0.0,
            minimum_rotation_span_radians: 0.05,
        }];
        *window = ManualAnimationWindowEvidence {
            label: Some(if side == 0 {
                "rotating_base_step_left"
            } else {
                "rotating_base_step_right"
            }),
            settled: ready || completed,
            capture_source_window: true,
            capture_every_frame: true,
            requires_pre_rig_motion: true,
            requires_final_pose_motion: true,
            requires_non_reference_root_pose: true,
            requires_source_pose_chain: true,
            source_node: Some("AB_Als_Standing/Idle/RefreshDynamicTransitions"),
            source_state: Some("Idle"),
            source_pose_chain: chain,
            expected_animation_variant: Some(if side == 0 {
                "A_Als_Stand_DynamicTransition_Left"
            } else {
                "A_Als_Stand_DynamicTransition_Right"
            }),
            observed_animation_variant: Some(bevy_als::stop_link::source_transition_sequence(
                playback,
            )),
            observed_animation_clip_time: Some(playback.clip_time),
            observed_animation_weight: Some(playback.weight),
            observed_animation_play_rate: Some(playback.play_rate),
            expected_lifecycle_stages: &[
                "playing",
                "foot_unlocked",
                "foot_relocked",
                "replacement_allowed",
            ],
            observed_lifecycle_stages: if completed {
                &[
                    "playing",
                    "foot_unlocked",
                    "foot_relocked",
                    "replacement_allowed",
                ]
            } else if probe.step_unlocked[side] {
                &["playing", "foot_unlocked"]
            } else if ready {
                &["playing"]
            } else {
                &[]
            },
            minimum_pre_rig_rotation_span_radians: 0.05,
            minimum_final_rotation_span_radians: 0.05,
            minimum_observation_seconds: 0.12,
            canonical_clip_time: Some(0.4),
            maximum_clip_time_distance: 0.06,
            expected_direction: Some(if side == 0 { "Left" } else { "Right" }),
            observed_direction: Some(if side == 0 { "Left" } else { "Right" }),
            direction_signature_group: Some("rotating_base_step"),
            required_direction_signature_members: 2,
            minimum_direction_signature_rotation_radians: 1e-4,
            ..default()
        };
    }
    if matches!(stage, Stage::Exit | Stage::Flight) {
        *window = ManualAnimationWindowEvidence {
            label: Some("moving_base_jump"),
            settled: true,
            capture_source_window: true,
            capture_every_frame: true,
            requires_pre_rig_motion: true,
            requires_final_pose_motion: true,
            requires_non_reference_root_pose: true,
            requires_source_pose_chain: true,
            source_node: Some("AB_Als_Locomotion/AnimGraph"),
            source_state: Some("Moving base jump and landing"),
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
    fn translating_mantle_requires_normal_motion_and_a_midpath_approach() {
        let kind = PlaygroundKinematicKind::MoveSpline {
            start: Vec3::new(19.0, 0.0, -28.75),
            end: Vec3::new(12.5, 0.0, -28.75),
            duration: 1.75,
        };
        let approach = mantle_approach(kind, &GlobalTransform::IDENTITY, Vec3::ONE, 0.3);
        assert!(approach.distance(Vec3::new(15.75, 0.0, -27.35)) < 1e-5);
        assert!(!moving_mantle_candidate(kind, Vec3::X * 0.09));
        assert!(!moving_mantle_candidate(kind, Vec3::X));
        assert!(moving_mantle_candidate(kind, Vec3::X * 4.0));
        assert!(moving_mantle_candidate(
            PlaygroundKinematicKind::RotateWorldTime {
                rate_yzx_radians: bevy::math::DVec3::new(-100.0_f64.to_radians(), 0.0, 0.0)
            },
            Vec3::ZERO
        ));
    }

    #[test]
    fn riding_preserves_the_settled_world_orientation_instead_of_forcing_zero() {
        let mut probe = Probe::default();
        let initial = Quat::from_rotation_y(-4.72_f32.to_radians());
        probe.observe_ride_rotation(initial);
        probe.observe_ride_rotation(initial);
        assert!(probe.failure.is_none());
        probe.observe_ride_rotation(Quat::from_rotation_y(0.1) * initial);
        assert!(probe.failure.is_some());
    }

    #[test]
    fn rotating_mantle_approach_stays_clear_and_reaches_the_actual_component() {
        use bevy::ecs::system::RunSystemOnce;
        fn candidate(
            In((character, origin, target, half, radius)): In<(Entity, Vec3, Entity, f32, f32)>,
            spatial: SceneQuery,
            velocities: Query<
                (
                    Option<&RigidBody>,
                    Option<&LinearVelocity>,
                    Option<&AlsComponentVelocity>,
                ),
                Without<AlsCharacter>,
            >,
            permissions: Query<Option<&AlsMantleTargetPermission>>,
            owners: Query<&ColliderOf>,
        ) -> bool {
            detect_mantle(
                &spatial,
                character,
                MantleDetectionInput {
                    origin,
                    actor_yaw: 0.0,
                    velocity_yaw: None,
                    input_yaw: Some(0.0),
                    locomotion: LocomotionState::Grounded,
                    stance: Stance::Standing,
                    overlay: OverlayState::Default,
                    capsule_half_height: half,
                    capsule_radius: radius,
                    capsule_scale: 1.0,
                },
                &velocities,
                &permissions,
                &owners,
            )
            .intent
            .is_some_and(|intent| intent.target_entity == target)
        }
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            bevy::mesh::MeshPlugin,
            TransformPlugin,
            avian3d::prelude::PhysicsPlugins::new(PostUpdate),
        ));
        app.finish();
        app.cleanup();
        let body = app
            .world_mut()
            .spawn((RigidBody::Kinematic, Transform::from_xyz(0.0, -0.5, 0.0)))
            .id();
        let collider = app
            .world_mut()
            .spawn((
                ChildOf(body),
                Collider::cuboid(3.0, 3.0, 3.0),
                Transform::IDENTITY,
                avian3d::prelude::CollisionLayers::new(
                    [
                        bevy_als::als::character::AlsCollisionLayer::Default,
                        bevy_als::als::character::AlsCollisionLayer::Visibility,
                    ],
                    avian3d::prelude::LayerMask::ALL,
                ),
            ))
            .id();
        app.world_mut().spawn((
            RigidBody::Static,
            Collider::cuboid(20.0, 0.2, 20.0),
            Transform::from_xyz(0.0, -0.1, 0.0),
        ));
        let character = app.world_mut().spawn_empty().id();
        let params = AlsMovementParams::default();
        let (half, radius) = (
            params.crouch.standing_capsule_half_height,
            params.crouch.standing_capsule_radius,
        );
        let kind = PlaygroundKinematicKind::RotateWorldTime {
            rate_yzx_radians: bevy::math::DVec3::new(-100.0_f64.to_radians(), 0.0, 0.0),
        };
        let mut candidates = 0;
        for sample in 0..72 {
            let transform = Transform::from_xyz(0.0, -0.5, 0.0)
                .with_rotation(Quat::from_rotation_y(sample as f32 * 5.0_f32.to_radians()));
            app.world_mut()
                .entity_mut(body)
                .insert((transform, Rotation(transform.rotation)));
            app.update();
            let global = app.world().get::<GlobalTransform>(body).unwrap();
            let origin =
                mantle_approach(kind, global, Vec3::splat(1.5), radius).with_y(half + 0.024);
            let local = global.affine().inverse().transform_point3(origin);
            assert!(
                local.distance(local.clamp(Vec3::splat(-1.5), Vec3::splat(1.5))) >= radius + 0.099
            );
            candidates += usize::from(
                app.world_mut()
                    .run_system_once_with(candidate, (character, origin, collider, half, radius))
                    .unwrap(),
            );
        }
        assert!(
            candidates > 1,
            "The original rotating mesh never becomes a valid mantle target"
        );
    }

    #[test]
    fn rotating_source_cycle_uses_a_moving_point_and_rejects_a_frozen_mesh() {
        let kind = PlaygroundKinematicKind::RotateWorldTime {
            rate_yzx_radians: bevy::math::DVec3::new(-100.0_f64.to_radians(), 0.0, 0.0),
        };
        for frozen in [false, true] {
            let mut probe = Probe::default();
            probe.begin(Stage::Ride, 0.0);
            let mut complete = false;
            for frame in 0..=370 {
                let time = frame as f32 / 100.0;
                let global =
                    GlobalTransform::from(Transform::from_xyz(13.5, -0.5, -24.0).with_rotation(
                        Quat::from_rotation_y(if frozen {
                            0.0
                        } else {
                            -100.0_f32.to_radians() * time
                        }),
                    ));
                let (period, span, point) = motion_sample(kind, &global);
                assert!((period - 3.6).abs() < 1e-5);
                complete |= probe.riding(
                    Vec3::X * 0.65,
                    point,
                    0.0,
                    true,
                    f64::from(time),
                    period,
                    span,
                );
            }
            assert_eq!(complete, !frozen);
            assert!(probe.failure.is_none());
        }
        assert_eq!(
            object(IntegrationScenario::RotatingBase),
            "Rotating:BP_SimpleRotatingObject_2"
        );
        assert_eq!(
            initial_phase(IntegrationScenario::RotatingBase),
            IntegrationPhase::MovingBase
        );
    }

    #[test]
    fn translating_exit_waits_for_the_source_return_direction() {
        let path = PlaygroundKinematicKind::MoveSpline {
            start: Vec3::new(19.0, 0.0, -28.75),
            end: Vec3::new(12.5, 0.0, -28.75),
            duration: 1.75,
        };
        let extent = Vec3::ONE;
        assert!(!exit_direction_ready(
            path,
            Vec3::new(17.05, 0.0, -28.75),
            Vec3::X * 14.8,
            extent
        ));
        assert!(!exit_direction_ready(
            path,
            Vec3::new(18.99, 0.0, -28.75),
            Vec3::NEG_X * 0.17,
            extent
        ));
        assert!(exit_direction_ready(
            path,
            Vec3::new(13.02, 0.0, -28.75),
            Vec3::NEG_X * 5.86,
            extent
        ));
        assert!(exit_direction_ready(
            path,
            Vec3::new(12.5, 0.0, -28.75),
            Vec3::ZERO,
            extent
        ));
    }

    #[test]
    fn rotating_steps_require_real_playback_unlock_source_release_and_relock() {
        for missing in ["none", "pose", "unlock", "release", "relock", "source_rate"] {
            let mut probe = Probe::default();
            for side in 0..2 {
                let mut feet = AlsFootLockState::default();
                let foot = if side == 0 {
                    &mut feet.left
                } else {
                    &mut feet.right
                };
                foot.lock_amount = if missing == "unlock" { 1.0 } else { 0.0 };
                let mut playback = AlsTransitionPlayback::standing_dynamic(side as f32);
                playback.clip_time = 0.4;
                playback.weight = 1.0;
                if missing == "source_rate" {
                    playback.play_rate = 1.0;
                }
                let slot = AlsRootTransitionSlotRuntime {
                    active: true,
                    pose_ready: missing != "pose",
                    weight: 1.0,
                };
                probe.observe_steps(Some(&feet), Some(&playback), Some(&slot), 1.0);
                let foot = if side == 0 {
                    &mut feet.left
                } else {
                    &mut feet.right
                };
                foot.lock_amount = 1.0;
                if missing == "relock" {
                    foot.lock_location = bevy::math::DVec3::X;
                }
                playback.clip_time = if missing == "release" { 0.5 } else { 0.55 };
                probe.observe_steps(Some(&feet), Some(&playback), Some(&slot), 1.0);
            }
            assert_eq!(
                probe.steps.iter().all(|value| *value) && probe.failure.is_none(),
                missing == "none",
                "Missing step evidence: {missing}"
            );
        }
    }

    #[test]
    fn driver_and_observer_initialize_with_the_public_character_resources() {
        use bevy::ecs::system::RunSystemOnce;
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            TransformPlugin,
            bevy::input::InputPlugin,
            bevy::gizmos::GizmoPlugin,
        ))
        .init_asset::<bevy::world_serialization::WorldAsset>()
        .init_asset::<Mesh>()
        .init_asset::<Image>()
        .init_asset::<StandardMaterial>()
        .add_plugins(bevy_als::playable_character::AlsPlayableCharacterPlugin)
        .add_plugins(crate::playground_collision::PlaygroundCollisionPlugin)
        .init_resource::<ManualIntegrationState>()
        .init_resource::<ManualRuntimePhaseEvidence>()
        .init_resource::<ManualIntegrationGateReport>()
        .init_resource::<ManualAnimationWindowEvidence>();
        install(&mut app);
        app.add_systems(
            PostUpdate,
            observe_manual_integration.in_set(ManualIntegrationObservationSet),
        );
        app.finish();
        app.cleanup();
        app.world_mut().run_schedule(Startup);
        app.world_mut().run_system_once(drive).unwrap();
        app.world_mut().run_system_once(observe).unwrap();
        {
            let mut state = app.world_mut().resource_mut::<ManualIntegrationState>();
            state.character = Some(Entity::PLACEHOLDER);
            state.phase = IntegrationPhase::MovingBase;
            state.moving_base_probe.failure = Some("fixture failure".to_owned());
        }
        app.world_mut().run_schedule(Update);
        assert_eq!(
            app.world().resource::<ManualIntegrationState>().phase,
            IntegrationPhase::Finish
        );
        app.world_mut()
            .schedule_scope(PostUpdate, |world, schedule| {
                schedule.initialize(world).unwrap();
                let types = schedule
                    .systems()
                    .unwrap()
                    .map(|(_, system)| system.system_type())
                    .collect::<Vec<_>>();
                let generic_type = IntoSystem::into_system(observe_manual_integration).system_type();
                let specific_type = IntoSystem::into_system(observe).system_type();
                let generic = types
                    .iter()
                    .position(|kind| *kind == generic_type)
                    .unwrap();
                let specific = types
                    .iter()
                    .position(|kind| *kind == specific_type)
                    .unwrap();
                assert!(
                    specific > generic,
                    "The generic turn fallback must not overwrite the source step window"
                );
            });
    }

    #[test]
    fn action_inputs_are_single_pulses_and_wait_for_a_valid_mantle_target() {
        let mut probe = Probe::default();
        probe.begin(Stage::Exit, 0.0);
        assert!(!probe.command(0.1, 0.4, false).jump_pressed);
        assert!(probe.command(0.2, 0.5, false).jump_pressed);
        assert!(!probe.command(0.3, 0.6, false).jump_pressed);
        probe.begin(Stage::WaitMantle, 1.0);
        assert!(!probe.command(1.5, 0.0, false).jump_pressed);
        assert!(probe.command(1.6, 0.0, true).jump_pressed);
        assert!(!probe.command(1.7, 0.0, true).jump_pressed);
    }

    #[test]
    fn ride_gate_rejects_frozen_targets_drift_and_lost_support() {
        for (position, local, base, succeeds) in [
            (Vec3::ZERO, Vec3::ZERO, true, false),
            (Vec3::X * 6.5, Vec3::ZERO, true, true),
            (Vec3::X * 6.5, Vec3::X, true, false),
            (Vec3::X * 6.5, Vec3::ZERO, false, false),
        ] {
            let mut probe = Probe::default();
            probe.begin(Stage::Ride, 0.0);
            assert!(!probe.riding(Vec3::ZERO, Vec3::ZERO, 0.0, true, 0.0, 3.5, 6.5));
            let complete = probe.riding(local, position, 0.0, base, 3.6, 3.5, 6.5);
            assert_eq!(complete && probe.failure.is_none(), succeeds);
        }
    }

    #[test]
    fn stage_watchdog_reports_a_fixed_progress_limit() {
        let mut probe = Probe::default();
        probe.begin(Stage::WaitMantle, 1.0);
        probe.watchdog(9.1, 3.5);
        assert!(probe.failure.unwrap().contains("stage_progress_timeout"));
    }
}
