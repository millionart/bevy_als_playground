use bevy_als::als::character::AlsCollisionLayer;
use std::num::NonZeroU8;
use std::time::{Duration, Instant};

mod bow;
mod footprints;
mod hud_layout;
mod lighting;
mod machine_weight;
mod moving_base;
mod overlays;
mod ragdoll_debug;
mod reroute;
mod stairs;
mod uphill;
pub use lighting::LightingProbePlugin;

use bevy_als::als::character::{
    ALS_CHARACTER_CAPSULE_HALF_HEIGHT, ALS_CHARACTER_FLOOR_CLEARANCE, AlsCharacter,
    AlsMovementParamsHandle,
};
use bevy_als::als::dynamics::AlsMovementBaseDynamicsState;
use bevy_als::als::foot_ik::{AlsFootBones, AlsFootLockState, build_foot_bone_map};
use bevy_als::als::gait::{AlsDesiredGait, AlsMaxAllowedGait};
use bevy_als::als::input_request::{AlsCameraLeftShoulder, AlsFirstPersonView};
use bevy_als::als::mantle::{AlsMantleControlSettings, AlsMantleState};
use bevy_als::als::motion::{
    AlsAimingYawConstraint, AlsCrouchTransaction, AlsInputOverride, AlsRotationAnimationState,
    AlsRotationResponseSet, AlsRotationResponseState, AlsViewYaw,
};
use bevy_als::als::movement_direction::{AlsMovementDirection, AlsMovementDirectionState};
use bevy_als::als::orientation::world_forward_to_yaw;
use bevy_als::als::params::AlsMovementParams;
use bevy_als::als::roll::AlsRollState;
use bevy_als::als::start_stop::{StartStopPhase, StartStopState};
use bevy_als::als::state::{Gait, LocomotionState, OverlayState, RotationMode, Stance};
use bevy_als::als::transition::{AlsDesiredStance, AlsStateRequest};
use bevy_als::als::view::AlsViewPitch;
use bevy_als::als_camera::{
    AlsCameraDebugInfo, AlsCameraRig, AlsCameraSet, AlsCameraShakeRequest, AlsCameraSocketBinding,
    AlsCameraSocketLocations,
};
use bevy_als::extensions::source_cached_pose::CACHED_POSE_VALUE;
use bevy_als::extensions::source_instance::SourceInstanceProperties;
use bevy_als::extensions::{
    ALS_LAYER_CURVE_NAMES, RagdollCollisionMember, RagdollPhysicsProfile,
    RagdollPhysicsProfileApplied, SourcePhysicsJoint,
};
use avian3d::prelude::{Collider, CollisionLayers, LinearVelocity, Position, RigidBody, Rotation};
use bevy::animation::AnimationTargetId;
use bevy::ecs::message::{MessageReader, MessageWriter};
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::reflect::PartialReflect;
use bevy::time::{Real, Virtual};
use bevy_animation_graph::core::animated_scene::{
    AnimatedScene, AnimatedSceneHandle, AnimatedSceneInstance,
};
use bevy_animation_graph::core::animation_graph::{AnimationGraph, NodeId, SourcePin, TargetPin};
use bevy_animation_graph::core::animation_graph_player::{AnimationGraphPlayer, AnimationSource};
use bevy_animation_graph::core::context::node_states::StateKey;
use bevy_animation_graph::core::edge_data::DataValue;
use bevy_animation_graph::core::ragdoll::definition::{BodyLabel, ColliderLabel, JointLabel};
use bevy_animation_graph::core::{
    pose::Pose, skeleton::Skeleton, state_machine::high_level::StateMachine,
};
use bevy_tnua::TnuaToggle;
use serde::Serialize;

use bevy_als::actual_motion::AlsActualMotion;
use bevy_als::animation_context_projection::AlsRootAnimationContext;
use bevy_als::animation_events::{
    AlsAnimationEvent, AlsNotifyEarlyBlendOut, AlsNotifyLocomotionAction, AlsNotifyRootMotionScale,
};
use crate::animation_visual_validation::AnimationVisualValidationProgress;
use bevy_als::character_assembly::{
    ALS_LINKED_ANIMATED_SCENE, AlsCharacterAssembly, AlsCharacterAvatar, avatar_transform,
};
use bevy_als::control_rig_apply::{AlsControlRigExecution, AlsControlRigInput, AlsControlRigState};
use bevy_als::crouching_layer::{AlsCrouchingGraphTelemetry, AlsCrouchingTelemetrySet};
use bevy_als::debug_display::AlsDebugDisplayStats;
use bevy_als::foot_ik_apply::{
    AlsFootIkApplyTelemetry, AlsFootIkSpaceState, AlsFootIkTelemetry, AlsFootIkTelemetryEnabled,
    AlsFootIkWeightOverride, AlsFootLockTeleportTelemetry, FootIkApplySet,
};
use crate::game_mode::AlsPlayerAdmissionComplete;
use bevy_als::grounded_layer::AlsGroundedEntryMode;
use bevy_als::grounded_layer::AlsGroundedGraphTelemetry;
use bevy_als::head_layer::AlsHeadState;
use bevy_als::layering_layer::AlsLayeringState;
use bevy_als::mantle_drive::AlsMantleMontages;
use crate::overlay_mode_menu::OverlayMenuState;
use bevy_als::overlay_object::{AlsOverlayObject, AlsOverlayObjectState};
use bevy_als::ragdoll_link::{
    AlsRagdollGroundFollow, AlsRagdollJointDrive, RAGDOLL_ANGULAR_STIFFNESS,
};
use bevy_als::roll_drive::AlsRollSlotRuntime;
use bevy_als::show_debug_hud::{
    AlsShowDebugHudRoot, AlsShowDebugHudState, DebugCategory, apply_show_debug_key,
};
use bevy_als::stance_layer::AlsStanceBlend;
use bevy_als::standing_layer::{AlsPivotNotify, AlsStandingGraphTelemetry};
use bevy_als::stop_link::{
    AlsRootTransitionSlotRuntime, AlsTransitionKind, AlsTransitionMontageStack,
    AlsTransitionPlayback, RIGHT_TRANSITION_START_TIME, STOP_TRANSITION_BLEND_IN,
    STOP_TRANSITION_BLEND_OUT, STOP_TRANSITION_PLAY_RATE,
};
use bevy_als::transition_notify::TransitionNotifyTelemetry;
use bevy_als::turn_layer::{
    AlsStandingTurnSlotRuntime, AlsTurnDrive, AlsTurnMontageStack, AlsTurnSlotBridgeSet,
    TURN_DUR_90,
};
use bevy_als::{air_jump_layer::AlsJumpVariantState, gait_layer::AlsGaitBlend};
use crate::als_hud::{AlsHudDebugSection, AlsHudInputSection, AlsHudRoot, AlsHudState};

const GATE_TIMEOUT: Duration = Duration::from_secs(180);
const FOOT_AVATAR_REPLACEMENT_START_SECONDS: f32 = 5.5;
const FOOT_AVATAR_REPLACEMENT_NO_PROGRESS_LIMIT_SECONDS: f32 = 10.0;
const CROUCH_CLEARANCE_TIMEOUT: f32 = 90.0;
const SPRINT_START: Vec3 = Vec3::new(-45.0, 0.0, -26.0);
const SPRINT_YAW: f32 = -std::f32::consts::FRAC_PI_2;
const GROUNDED_START: Vec3 = Vec3::new(-45.0, 0.0, -28.0);
const GROUNDED_ROLL_YAW: f32 = 0.0;
const LOCOMOTION_DROP_HEIGHT: f32 = 30.0;
const DIRECTION_CASE_SECONDS: f32 = 3.0;
const DIRECTION_CASE_SETTLE_SECONDS: f32 = 1.6;
const STATE_MACHINE_TURN_FIRST_START: f32 = 0.5;
const STATE_MACHINE_TURN_FIRST_END: f32 = 1.75;
const STATE_MACHINE_TURN_SECOND_START: f32 = 4.0;
const STATE_MACHINE_TURN_SECOND_END: f32 = 5.25;
const STATE_MACHINE_TURN_END: f32 = STATE_MACHINE_TURN_SECOND_END + TURN_DUR_90 + 1.0;
const DIRECTION_CASE_COUNT: usize = 11;
const CROUCH_DIRECTION_CASE_SECONDS: f32 = 3.0;
const CROUCH_DIRECTION_CASE_SETTLE_SECONDS: f32 = 1.6;
const CROUCH_DIRECTION_CASE_COUNT: usize = 8;
const CROUCH_STATIONARY_TIMEOUT_SECONDS: f32 = 40.0;
const CROUCH_STATIONARY_STABLE_FRAMES: u8 = 3;
const CROUCH_ROTATE_SOURCE_MIN_YAW_CONVERGENCE_RADIANS: f32 = 1.4;
const CROUCH_TURN_SOURCE_MIN_YAW_CONVERGENCE_RADIANS: f32 = 1.4;
const DIRECTION_BRANCH_FORWARD: u8 = 1 << 0;
const DIRECTION_BRANCH_BACKWARD: u8 = 1 << 1;
const DIRECTION_BRANCH_LEFT: u8 = 1 << 2;
const DIRECTION_BRANCH_RIGHT: u8 = 1 << 3;
const FOOT_IK_BASELINE_MASK: u8 = 0b1_1111;
const FOOT_IK_WEIGHT_MASK: u8 = 0b11_1111;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CrouchStationaryValidation {
    RotateRight,
    RotateLeft,
    TurnRight,
    TurnLeft,
}

impl CrouchStationaryValidation {
    fn is_left(self) -> bool {
        matches!(self, Self::RotateLeft | Self::TurnLeft)
    }

    fn is_turn(self) -> bool {
        matches!(self, Self::TurnRight | Self::TurnLeft)
    }

    fn side_index(self) -> usize {
        self.is_left() as usize
    }
}

fn next_crouch_stationary_validation(
    coverage: &IntegrationCoverage,
) -> Option<CrouchStationaryValidation> {
    if coverage.crouch_turn_completion_mask & (1 << 0) == 0 {
        Some(CrouchStationaryValidation::TurnRight)
    } else if coverage.crouch_turn_completion_mask & (1 << 1) == 0 {
        Some(CrouchStationaryValidation::TurnLeft)
    } else if coverage.crouch_rotate_completion_mask & (1 << 0) == 0 {
        Some(CrouchStationaryValidation::RotateRight)
    } else if coverage.crouch_rotate_completion_mask & (1 << 1) == 0 {
        Some(CrouchStationaryValidation::RotateLeft)
    } else {
        None
    }
}

fn generated_stop_quick_input_step(
    input_started: bool,
    input_released: bool,
    telemetry: Option<&AlsStandingGraphTelemetry>,
) -> (bool, bool, bool) {
    let input_started = input_started
        || telemetry.is_some_and(|telemetry| {
            telemetry.actual_standing_state == Some(0)
                && telemetry.actual_standing_pose_ready == Some(true)
        });
    let input_released = input_released
        || (input_started
            && telemetry.is_some_and(|telemetry| {
                telemetry.actual_standing_transition_active == Some(true)
            }));
    (
        input_started,
        input_released,
        input_started && !input_released,
    )
}

fn generated_stop_quick_observed(baseline: &mut Option<u32>, released: bool, count: u32) -> bool {
    if !released {
        *baseline = Some(count);
    }
    released && baseline.is_some_and(|previous| count > previous)
}

const STANDING_STOP_CASES: [(&str, &str, Vec2, RotationMode, f32); 7] = [
    (
        "stop_lock_left",
        "Lock Left Foot",
        Vec2::Y,
        RotationMode::Aiming,
        0.0,
    ),
    (
        "stop_lock_right",
        "Lock Right Foot",
        Vec2::Y,
        RotationMode::Aiming,
        1.0,
    ),
    (
        "stop_plant_left",
        "Plant Left Foot",
        Vec2::Y,
        RotationMode::Aiming,
        0.0,
    ),
    (
        "stop_plant_right",
        "Plant Right Foot",
        Vec2::Y,
        RotationMode::Aiming,
        1.0,
    ),
    (
        "quick_stop_default",
        "Move to Idle",
        Vec2::Y,
        RotationMode::Aiming,
        0.0,
    ),
    (
        "quick_stop_left",
        "Move to Idle",
        Vec2::NEG_X,
        RotationMode::VelocityDirection,
        0.0,
    ),
    (
        "quick_stop_right",
        "Move to Idle",
        Vec2::X,
        RotationMode::VelocityDirection,
        1.0,
    ),
];

const CROUCHING_STOP_CASES: [(&str, &str, Vec2, RotationMode, f32); 4] = [
    (
        "crouch_stop_right",
        "Stop",
        Vec2::Y,
        RotationMode::Aiming,
        1.0,
    ),
    (
        "crouch_quick_stop_default",
        "Move to Idle",
        Vec2::Y,
        RotationMode::Aiming,
        0.0,
    ),
    (
        "crouch_quick_stop_left",
        "Move to Idle",
        Vec2::new(-0.1, 1.0),
        RotationMode::VelocityDirection,
        0.0,
    ),
    (
        "crouch_quick_stop_right",
        "Move to Idle",
        Vec2::new(0.1, 1.0),
        RotationMode::VelocityDirection,
        1.0,
    ),
];

fn stop_cases(crouching: bool) -> &'static [(&'static str, &'static str, Vec2, RotationMode, f32)] {
    if crouching {
        &CROUCHING_STOP_CASES
    } else {
        &STANDING_STOP_CASES
    }
}

fn action_graph_state(
    crouching: bool,
    standing: Option<&AlsStandingGraphTelemetry>,
    crouch: Option<&AlsCrouchingGraphTelemetry>,
) -> (Option<u8>, Option<bool>, Option<bool>) {
    if crouching {
        crouch.map(|graph| {
            (
                graph.actual_outer_state,
                graph.actual_outer_transition_active,
                graph.actual_outer_pose_ready,
            )
        })
    } else {
        standing.map(|graph| {
            (
                graph.actual_standing_state,
                graph.actual_standing_transition_active,
                graph.actual_standing_pose_ready,
            )
        })
    }
    .unwrap_or_default()
}

const STANDING_TURN_CASES: [(&str, &str, f32); 8] = [
    ("turn_90_left", "A_Als_Turn_90_Left", 90.0),
    ("turn_90_right", "A_Als_Turn_90_Right", -90.0),
    ("turn_180_left", "A_Als_Turn_180_Left", 160.0),
    ("turn_180_right", "A_Als_Turn_180_Right", -160.0),
    ("rotate_left", "A_Als_Rotate_90_Left", 160.0),
    ("rotate_right", "A_Als_Rotate_90_Right", -160.0),
    ("turn_continuous", "", 0.0),
    ("aim_continuous", "", 0.0),
];

const STANDING_DYNAMIC_CASES: [(&str, f32); 2] = [("dynamic_right", 1.0), ("dynamic_left", 0.0)];

fn dynamic_idle_command(
    body_yaw: f32,
    view_yaw: f32,
    current: RotationMode,
) -> crate::input::AlsInputCommand {
    crate::input::AlsInputCommand {
        rotation_mode_pressed: current != RotationMode::VelocityDirection,
        look_delta: Vec2::new(
            -bevy_als::als::motion::wrap_yaw_radians(body_yaw - view_yaw)
                / bevy_als::als::view::LOOK_MOUSE_RADIANS_PER_COUNT,
            0.0,
        ),
        ..default()
    }
}

fn crouching_dynamic_fixture_pose(
    mut pose: Transform,
    feet: &AlsFootLockState,
    side: f32,
) -> Option<Transform> {
    if !feet.valid
        || !bevy_als::als::foot_ik::is_full_animation_weight(feet.left.lock_amount)
        || !bevy_als::als::foot_ik::is_full_animation_weight(feet.right.lock_amount)
    {
        return None;
    }
    let pivot = if side > 0.5 { &feet.left } else { &feet.right };
    pose.rotate_around(
        pivot.target_location.as_vec3(),
        Quat::from_rotation_y(std::f32::consts::FRAC_PI_6),
    );
    Some(pose)
}

fn standing_dynamic_foot_restored(
    unlocked: bool,
    foot: Option<&bevy_als::als::foot_ik::AlsFootLockFootState>,
    scale: f32,
) -> bool {
    unlocked
        && foot.is_some_and(|foot| {
            foot.lock_amount >= 1.0 - 1e-5
                && dynamic_distance_squared_cm(foot.target_location, foot.lock_location)
                    <= dynamic_threshold_squared_cm(scale)
        })
}

fn dynamic_distance_squared_cm(
    target: impl Into<bevy::math::DVec3>,
    locked: impl Into<bevy::math::DVec3>,
) -> f64 {
    let delta = target.into() * 100.0 - locked.into() * 100.0;
    delta.x.powi(2) + delta.z.powi(2) + delta.y.powi(2)
}

fn dynamic_threshold_squared_cm(scale: f32) -> f64 {
    f64::from((8.0 * scale).powi(2))
}

fn standing_dynamic_observed<'a>(
    stack: Option<&'a AlsTransitionMontageStack>,
    previous: Option<&AlsTransitionPlayback>,
    kind: AlsTransitionKind,
    index: f32,
) -> Option<&'a AlsTransitionPlayback> {
    stack.and_then(|stack| {
        stack.entries.iter().rev().find(|playback| {
            playback.kind == kind
                && playback.index == index
                && previous.is_none_or(|previous| previous.instance_id == playback.instance_id)
        })
    })
}

fn standing_dynamic_stopped(
    branch_seen: bool,
    previous: Option<&AlsTransitionPlayback>,
    current: Option<&AlsTransitionPlayback>,
    idle: bool,
) -> bool {
    branch_seen
        && previous.is_some_and(|playback| playback.clip_time > 0.8)
        && current.is_none_or(|playback| !playback.active)
        && idle
}

fn dynamic_transition_side(crouching: bool, sequence: f32) -> f32 {
    sequence - if crouching { 2.0 } else { 0.0 }
}

#[derive(Default)]
struct StandingTurnProbe {
    idle_ready: bool,
    started_at: Option<f32>,
    base_yaw: f32,
    active_seen: bool,
    clip_peak: f32,
    epoch_baseline: f32,
    sides_mask: u8,
    max_play_rate: f32,
    invalid_rate: bool,
    previous_yaw: Option<f32>,
    yaw_travel: [f32; 2],
}

fn standing_turn_mouse_command(
    index: usize,
    elapsed: Option<f32>,
    base_yaw: f32,
    view_yaw: f32,
    current: RotationMode,
) -> crate::input::AlsInputCommand {
    let aiming = matches!(index, 4 | 5 | 7);
    let offset = elapsed.map(|elapsed| {
        if index == 7 {
            if elapsed < 4.0 {
                720.0 * elapsed
            } else if elapsed < 8.0 {
                720.0 * (8.0 - elapsed)
            } else if elapsed < 12.0 {
                let phase = (elapsed - 8.0) % 0.5;
                720.0 * phase.min(0.5 - phase)
            } else {
                0.0
            }
        } else if index == 6 {
            [75.0_f32, -75.0, 75.0, -75.0, 75.0, -75.0, 0.0][((elapsed / 1.2) as usize).min(6)]
        } else if aiming {
            STANDING_TURN_CASES[index].2.signum() * (elapsed * 360.0).min(160.0)
        } else {
            STANDING_TURN_CASES[index].2
        }
    });
    crate::input::AlsInputCommand {
        aim_held: aiming,
        rotation_mode_pressed: !aiming && current == RotationMode::VelocityDirection,
        look_delta: offset.map_or(Vec2::ZERO, |offset| {
            Vec2::new(
                -bevy_als::als::turn_in_place::wrap_angle_degrees(
                    (base_yaw - view_yaw).to_degrees() + offset,
                )
                .to_radians()
                    / bevy_als::als::view::LOOK_MOUSE_RADIANS_PER_COUNT,
                0.0,
            )
        }),
        ..default()
    }
}

fn standing_turn_completed(
    index: usize,
    probe: &StandingTurnProbe,
    idle: bool,
    slot_idle: bool,
    epoch: f32,
    elapsed: f32,
) -> bool {
    probe.started_at.is_some()
        && probe.active_seen
        && !probe.invalid_rate
        && idle
        && slot_idle
        && if index == 7 {
            elapsed > 12.0
                && probe.clip_peak > 0.9
                && probe.max_play_rate > 2.9
                && probe.sides_mask == 3
                && probe
                    .yaw_travel
                    .iter()
                    .all(|angle| *angle > 6.0 * std::f32::consts::TAU)
        } else if matches!(index, 4 | 5) {
            probe.clip_peak > 0.9 && probe.max_play_rate > 1.3 && elapsed > 0.5
        } else {
            probe.clip_peak > 1.9
                && epoch > probe.epoch_baseline
                && (index != 6
                    || (elapsed > 7.2
                        && epoch - probe.epoch_baseline >= 3.0
                        && probe.sides_mask == 3))
        }
}

fn standing_turn_rotation_settled(
    index: usize,
    yaw_error_degrees: f32,
    curve_speed: f32,
    rotate_active: bool,
) -> bool {
    let threshold = if matches!(index, 4 | 5 | 7) {
        bevy_als::als::rotate_in_place::RotateInPlaceConfig::default().view_yaw_angle_threshold_degrees
    } else {
        bevy_als::als::turn_in_place::TurnInPlaceConfig::default().view_yaw_angle_threshold_degrees
    };
    !rotate_active && curve_speed.abs() <= f32::EPSILON && yaw_error_degrees.abs() <= threshold
}

fn standing_stop_release_phase(index: usize, foot_planted: Option<f32>) -> bool {
    foot_planted.is_some_and(|value| match index {
        0 => value <= -0.75,
        1 => value >= 0.75,
        2 => (-0.4..=-0.1).contains(&value),
        3 => (0.1..=0.4).contains(&value),
        _ => false,
    })
}

fn standing_stop_command(
    index: usize,
    current: RotationMode,
    moving: bool,
    crouching: bool,
) -> crate::input::AlsInputCommand {
    let (_, _, direction, mode, _) = stop_cases(crouching)[index];
    crate::input::AlsInputCommand {
        move_dir: if moving { direction } else { Vec2::ZERO },
        aim_held: mode == RotationMode::Aiming,
        rotation_mode_pressed: mode == RotationMode::VelocityDirection && current != mode,
        ..default()
    }
}

fn standing_stop_observation_active(state: &ManualIntegrationState) -> bool {
    state.phase == IntegrationPhase::StandingStop && !state.entered
}

fn standing_stop_completed(
    branch_seen: bool,
    previous: Option<&AlsTransitionPlayback>,
    current: Option<&AlsTransitionPlayback>,
    idle: bool,
) -> bool {
    branch_seen
        && previous.is_some_and(|playback| playback.clip_time > 0.8)
        && current.is_some_and(|playback| {
            !playback.active
                && playback.clip_time
                    == playback.duration
                        - bevy_als::extensions::profile_blend::MONTAGE_SECTION_END_OFFSET
        })
        && idle
}

#[derive(Resource, Default)]
pub struct ManualPlaygroundStartupReady {
    pub passed: bool,
}

#[derive(Resource, Clone, Debug, Default)]
pub struct ManualAnimationWindowEvidence {
    pub label: Option<&'static str>,
    pub index: Option<usize>,
    pub settled: bool,
    pub capture_source_window: bool,
    pub capture_every_frame: bool,
    pub requires_pre_rig_motion: bool,
    pub requires_final_pose_motion: bool,
    pub requires_non_reference_root_pose: bool,
    pub requires_source_pose_chain: bool,
    pub requires_actor_yaw_motion: bool,
    pub requires_actor_yaw_convergence: bool,
    pub actor_yaw_driver: Option<&'static str>,
    pub expected_actor_yaw_delta_sign: Option<i8>,
    pub expected_actor_yaw_baseline: Option<f32>,
    pub expected_animation_variant: Option<&'static str>,
    pub source_node: Option<&'static str>,
    pub source_state: Option<&'static str>,
    pub source_pose_chain: Vec<ManualAnimationPoseBoundary>,
    pub minimum_pre_rig_translation_span: f32,
    pub minimum_pre_rig_rotation_span_radians: f32,
    pub minimum_final_translation_span: f32,
    pub minimum_final_rotation_span_radians: f32,
    pub minimum_actor_yaw_delta_radians: f32,
    pub minimum_actor_yaw_convergence_radians: f32,
    pub minimum_observation_seconds: f32,
    pub source_sync_group: Option<&'static str>,
    pub sync_phase_global: Option<&'static str>,
    pub canonical_sync_phase: Option<f32>,
    pub maximum_sync_phase_distance: f32,
    pub canonical_clip_time: Option<f32>,
    pub maximum_clip_time_distance: f32,
    pub direction_signature_group: Option<&'static str>,
    pub required_direction_signature_members: usize,
    pub minimum_direction_signature_rotation_radians: f32,
    pub expected_direction: Option<&'static str>,
    pub observed_direction: Option<&'static str>,
    pub expected_runtime_state: Option<u8>,
    pub observed_runtime_state: Option<u8>,
    pub expected_parent_runtime_state: Option<u8>,
    pub expected_terminal_parent_runtime_state: Option<u8>,
    pub expected_terminal_parent_transition_active: Option<bool>,
    pub expected_terminal_requested_parent_transition_active: Option<bool>,
    pub observed_parent_runtime_state: Option<u8>,
    pub observed_parent_transition_active: Option<bool>,
    pub observed_requested_parent_runtime_state: Option<u8>,
    pub observed_requested_parent_transition_active: Option<bool>,
    pub observed_actor_yaw: Option<f32>,
    pub observed_view_yaw: Option<f32>,
    pub observed_animation_yaw_speed_degrees: Option<f32>,
    pub observed_animation_curve_direct_yaw_speed_degrees: Option<f32>,
    pub observed_animation_clip_time: Option<f32>,
    pub observed_animation_weight: Option<f32>,
    pub observed_animation_play_rate: Option<f32>,
    pub observed_animation_curve_stack_weight: Option<f32>,
    pub observed_animation_curve_stack_entries: Vec<ManualTurnStackEntryEvidence>,
    pub observed_animation_variant: Option<&'static str>,
    pub expected_lifecycle_stages: &'static [&'static str],
    pub observed_lifecycle_stages: &'static [&'static str],
}

#[derive(Clone, Debug, Serialize)]
pub struct ManualTurnStackEntryEvidence {
    pub animation_variant: &'static str,
    pub elapsed_seconds: f32,
    pub weight: f32,
    pub play_rate: f32,
    pub sampling_ready: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct ManualAnimationPoseBoundary {
    pub label: &'static str,
    pub graph: &'static str,
    pub node: &'static str,
    pub output: &'static str,
    pub minimum_translation_span: f32,
    pub minimum_rotation_span_radians: f32,
}

#[derive(Resource, Clone, Debug, Default)]
pub struct ManualRuntimePhaseEvidence {
    pub scenario: String,
    pub phase: String,
    pub elapsed_seconds: f32,
    pub temporal_rebase_epoch: u64,
    pub locomotion: String,
    pub gait: String,
    pub stance: String,
    pub rotation_mode: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ManualIntegrationGateResult {
    pub name: String,
    pub covered: bool,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct ManualCameraTraceSample {
    pub phase_elapsed_seconds: f64,
    pub obstacles_present: bool,
    pub obstacles_ignored: bool,
    pub blocked_ratio: Option<f32>,
    pub first_person_requested: bool,
    pub actual_first_person: bool,
    pub ragdoll_active: bool,
    pub camera: AlsCameraDebugInfo,
}

#[derive(Default)]
struct CameraMovementProbe {
    previous_view: Option<f32>,
    samples: usize,
    lagged: bool,
    invalid: bool,
}

impl CameraMovementProbe {
    fn observe(
        &mut self,
        view: Option<f32>,
        control: f32,
        input: Vec2,
        snapshot: Option<f32>,
        desired: Option<Vec3>,
    ) {
        if input.length_squared() > 1e-6
            && let Some(previous) = self.previous_view
        {
            self.samples += 1;
            let expected = bevy_als::als::motion::view_relative_move_input(input, previous);
            self.invalid |= snapshot.is_none_or(|value| {
                bevy_als::als::motion::wrap_yaw_radians(value - previous).abs() > 1e-5
            }) || desired.is_none_or(|value| !value.abs_diff_eq(expected, 1e-5));
            self.lagged |= view.is_some_and(|value| {
                bevy_als::als::motion::wrap_yaw_radians(value - control).abs() > 0.01
            });
        }
        self.previous_view = view;
    }

    fn passed(&self) -> bool {
        self.samples >= 8 && self.lagged && !self.invalid
    }
}

#[derive(Resource, Clone, Debug, Default, Serialize, PartialEq)]
pub struct ManualIntegrationGateReport {
    pub scenario: String,
    pub timed_out: bool,
    pub completed: bool,
    pub passed: Option<bool>,
    pub results: Vec<ManualIntegrationGateResult>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sprint_trace: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub direction_trace: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub camera_trace: Vec<ManualCameraTraceSample>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub stairs_trace: Vec<stairs::Sample>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub uphill_trace: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub moving_base_trace: Vec<serde_json::Value>,
}

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ManualIntegrationObservationSet;

#[derive(SystemParam)]
struct ManualObservationResources<'w> {
    state: ResMut<'w, ManualIntegrationState>,
    animation_window: ResMut<'w, ManualAnimationWindowEvidence>,
    gate_report: ResMut<'w, ManualIntegrationGateReport>,
}

pub struct ManualPlaygroundIntegrationPlugin;

impl Plugin for ManualPlaygroundIntegrationPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ManualPlaygroundStartupReady>()
            .init_resource::<ManualIntegrationState>()
            .init_resource::<ManualAnimationWindowEvidence>()
            .init_resource::<ManualRuntimePhaseEvidence>()
            .init_resource::<ManualIntegrationGateReport>()
            .add_systems(
                Update,
                initialize_manual_integration.before(bevy_als::als::view::AlsViewInputSet),
            )
            .add_systems(
                PreUpdate,
                drive_manual_integration
                    .after(bevy_enhanced_input::prelude::EnhancedInputSystems::Apply),
            )
            .add_systems(
                PreUpdate,
                drive_foot_ik_avatar_replacement.after(drive_manual_integration),
            )
            .add_systems(
                PreUpdate,
                drive_crouch_direction_reference_frame
                    .after(drive_manual_integration)
                    .after(bevy_enhanced_input::prelude::EnhancedInputSystems::Apply),
            )
            .add_systems(
                PreUpdate,
                stairs::drive.after(drive_manual_integration).run_if(
                    |state: Res<ManualIntegrationState>| state.phase == IntegrationPhase::Stairs,
                ),
            )
            .add_systems(
                PostUpdate,
                stairs::observe
                    .run_if(|state: Res<ManualIntegrationState>| {
                        state.phase == IntegrationPhase::Stairs
                    })
                    .after(AlsCameraSet)
                    .after(AlsRotationResponseSet)
                    .after(FootIkApplySet::Telemetry)
                    .before(observe_manual_integration)
                    .in_set(ManualIntegrationObservationSet),
            )
            .add_systems(
                PostUpdate,
                stairs::capture_move_prefix
                    .run_if(|state: Res<ManualIntegrationState>| {
                        state.phase == IntegrationPhase::Stairs
                    })
                    .before(avian3d::prelude::PhysicsSystems::First),
            )
            .add_systems(
                PreUpdate,
                uphill::drive.after(drive_manual_integration).run_if(
                    |state: Res<ManualIntegrationState>| {
                        state.phase == IntegrationPhase::UphillJump
                    },
                ),
            )
            .add_systems(
                PostUpdate,
                uphill::observe
                    .run_if(|state: Res<ManualIntegrationState>| {
                        state.phase == IntegrationPhase::UphillJump
                    })
                    .after(AlsCameraSet)
                    .after(AlsRotationResponseSet)
                    .after(FootIkApplySet::Telemetry)
                    .before(observe_manual_integration)
                    .in_set(ManualIntegrationObservationSet),
            )
            .add_systems(
                PostUpdate,
                (drive_control_rig_view_offset, drive_manual_rig_curves)
                    .after(avian3d::prelude::PhysicsSystems::Last)
                    .before(
                        bevy_als::extensions::post_physics_animation::PostPhysicsAnimationSet::Inputs,
                    ),
            )
            .add_systems(
                PostUpdate,
                prepare_get_up_physics_fixture
                    .after(bevy_als::extensions::RagdollAngularDriveSet)
                    .before(avian3d::prelude::PhysicsSystems::First),
            )
            .add_systems(
                PostUpdate,
                observe_manual_integration
                    .after(AlsCameraSet)
                    .after(bevy_als::debug_display::AlsDebugDisplaySet)
                    .after(bevy_als::show_debug_hud::AlsShowDebugHudSet::Sync)
                    .after(bevy_als::animation_context_projection::AlsRootAnimationContextSet)
                    .after(AlsCrouchingTelemetrySet)
                    .after(AlsTurnSlotBridgeSet)
                    .after(FootIkApplySet::Telemetry)
                    .after(AlsRotationResponseSet)
                    .in_set(ManualIntegrationObservationSet),
            );
        app.add_systems(
            PostUpdate,
            observe_animation_clock
                .after(bevy_animation_graph::core::plugin::AnimationGraphSet::Final)
                .before(observe_manual_integration)
                .in_set(ManualIntegrationObservationSet),
        );
        app.add_systems(
            PostUpdate,
            observe_visibility_case
                .after(bevy_animation_graph::core::plugin::AnimationGraphSet::Final)
                .before(observe_manual_integration)
                .in_set(ManualIntegrationObservationSet),
        );
        moving_base::install(app);
        footprints::install(app);
        hud_layout::install(app);
        reroute::install(app);
        ragdoll_debug::install(app);
        machine_weight::install(app);
        if std::env::var("ALS_MANUAL_RAGDOLL_PLAYER_REPLACEMENT").as_deref() == Ok("1") {
            app.add_systems(
                PreUpdate,
                replace_ragdoll_player_once.after(drive_manual_integration),
            );
        }
        if std::env::var("ALS_MANUAL_RIG_DEFINITION_HOT_SWAP").as_deref() == Ok("1") {
            app.add_systems(
                PostUpdate,
                drive_control_rig_definition_hot_swap
                    .after(AlsCameraSet)
                    .after(FootIkApplySet::Telemetry)
                    .before(observe_manual_integration),
            );
        }
    }
}

#[derive(Component)]
struct ManualIntegrationPlatform;

#[derive(Component)]
struct ManualRagdollPlayerReplacement {
    old_root: Entity,
}

#[derive(Default)]
struct ManualRigDefinitionHotSwap {
    stage: u8,
    graph: Option<bevy::asset::AssetId<AnimationGraph>>,
    node: Option<NodeId>,
    initialization_count: u64,
    register: usize,
}

#[derive(Component)]
struct ManualFootIkTerrain;

fn drive_control_rig_definition_hot_swap(
    mut state: ResMut<ManualIntegrationState>,
    mut probe: Local<ManualRigDefinitionHotSwap>,
    avatars: Query<&AnimatedSceneInstance>,
    players: Query<&AnimationGraphPlayer>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
) {
    use bevy_als::extensions::{
        control_rig_math::RigValue,
        control_rig_program::RigValueRef,
        root_graph_boundaries::{ControlRigBoundaryNode, current_control_rig},
    };
    use bevy_animation_graph::core::animation_node::dyn_node_like::DynNodeLike;
    if !state.initialized
        || state.scenario != IntegrationScenario::ControlRig
        || state.phase != IntegrationPhase::ControlRig
        || state.phase_elapsed() < 0.5
        || state.coverage.control_rig_definition_hot_swap
    {
        return;
    }
    let Some(avatar) = state.avatar else {
        return;
    };
    let Ok(instance) = avatars.get(avatar) else {
        return;
    };
    let Ok(player) = players.get(instance.player_entity()) else {
        return;
    };
    let Some((context, node, rig)) = current_control_rig(player, &graphs) else {
        return;
    };
    let graph_id = context.get_graph_id();
    let node_id = node.id;
    let count = rig.initialization_count;
    let value = (probe.stage > 0)
        .then(|| {
            rig.value(&RigValueRef {
                register: probe.register,
                path: Vec::new(),
            })
            .ok()
        })
        .flatten();
    if probe.stage > 0 && (probe.graph != Some(graph_id) || probe.node != Some(node_id)) {
        return;
    }
    match probe.stage {
        0 if count > 0 => {
            let Some(mut graph) = graphs.get_mut(graph_id) else {
                return;
            };
            let Some(node) = graph.nodes.get_mut(&node_id) else {
                return;
            };
            let Some(mut rig) = node
                .try_inner_downcast_ref::<ControlRigBoundaryNode>()
                .cloned()
            else {
                return;
            };
            let Some(definition) = rig.definition.as_mut() else {
                return;
            };
            probe.register = definition.program.initial_values.len();
            definition.program.initial_values.push(RigValue::F32(123.0));
            node.inner = DynNodeLike::new(rig);
            probe.graph = Some(graph_id);
            probe.node = Some(node_id);
            probe.initialization_count = count;
            probe.stage = 1;
            info!(
                "[MANUAL-INTEGRATION][RIG-DEFINITION-HOT-SWAP] stage=layout node={node_id:?} register={} initializations={count}",
                probe.register
            );
        }
        1 if count == probe.initialization_count + 1 && value == Some(RigValue::F32(123.0)) => {
            let Some(mut graph) = graphs.get_mut(graph_id) else {
                return;
            };
            let Some(node) = graph.nodes.get_mut(&node_id) else {
                return;
            };
            let Some(mut rig) = node
                .try_inner_downcast_ref::<ControlRigBoundaryNode>()
                .cloned()
            else {
                return;
            };
            let Some(definition) = rig.definition.as_mut() else {
                return;
            };
            let Some(register) = definition.program.initial_values.get_mut(probe.register) else {
                return;
            };
            *register = RigValue::F32(124.0);
            node.inner = DynNodeLike::new(rig);
            probe.stage = 2;
            info!(
                "[MANUAL-INTEGRATION][RIG-DEFINITION-HOT-SWAP] stage=constant node={node_id:?} register={} initializations={count}",
                probe.register
            );
        }
        2 if count == probe.initialization_count + 2 && value == Some(RigValue::F32(124.0)) => {
            state.coverage.control_rig_definition_hot_swap = true;
            info!(
                "[MANUAL-INTEGRATION][RIG-DEFINITION-HOT-SWAP] stage=complete node={node_id:?} register={} initializations={count} -> PASS",
                probe.register
            );
        }
        _ => {}
    }
}

fn drive_control_rig_view_offset(
    state: Res<ManualIntegrationState>,
    mut characters: Query<(&Transform, &mut AlsViewYaw), With<AlsCharacter>>,
) {
    if state.scenario != IntegrationScenario::ControlRig
        || !matches!(
            state.phase,
            IntegrationPhase::ControlRig | IntegrationPhase::Finish
        )
    {
        return;
    }
    let Some(character) = state.character else {
        return;
    };
    let Ok((transform, mut view_yaw)) = characters.get_mut(character) else {
        return;
    };
    let forward = transform.rotation * Vec3::NEG_Z;
    view_yaw.0 = bevy_als::als::orientation::world_forward_to_yaw(forward) + 60.0_f32.to_radians();
}

fn control_rig_action_evidence(
    player: &AnimationGraphPlayer,
    graphs: &Assets<AnimationGraph>,
    skeletons: &Assets<Skeleton>,
) -> Option<[f32; 3]> {
    use bevy_als::extensions::{
        control_rig_hierarchy::RigSpace,
        control_rig_math::RigValue,
        control_rig_program::RigInstruction,
        root_graph_boundaries::{ControlRigBoundaryNode, current_control_rig},
    };
    let (context, node, state) = current_control_rig(player, graphs)?;
    let rig = node.try_inner_downcast_ref::<ControlRigBoundaryNode>()?;
    let definition = rig.definition.as_ref()?;
    let mut weights = [0.0; 2];
    for (index, name) in ["hand_l", "hand_r"].into_iter().enumerate() {
        let call = state.calls.iter().rev().find(|call| {
            matches!(definition.program.instructions.get(call.instruction), Some(RigInstruction::Call { function, .. })
                if function == "FRigUnit_TwoBoneIKSimplePerItem::Execute")
                && matches!(call.arguments.get(2), Some(RigValue::List(item))
                    if matches!(item.as_slice(), [RigValue::Name(kind), RigValue::Name(bone)] if kind == "Bone" && bone == name))
        })?;
        let RigValue::F32(weight) = call.arguments.get(13)? else {
            return None;
        };
        weights[index] = *weight;
    }
    let graph = graphs.get(context.get_graph_id())?;
    let SourcePin::NodeData(source, pin) = graph
        .edges_inverted
        .get(&TargetPin::NodeData(node.id, rig.pose_input.clone()))?
    else {
        return None;
    };
    let DataValue::Pose(pose) =
        context
            .node_caches
            .get_output_data(*source, StateKey::Default, pin.clone())?
    else {
        return None;
    };
    let skeleton = skeletons.get(&pose.skeleton)?;
    let spine = skeleton.iter_paths().find(|path| {
        path.parts
            .last()
            .is_some_and(|name| name.as_str() == "spine_01")
    })?;
    let before = pose
        .get_bone(spine.id())
        .and_then(|bone| bone.rotation)
        .unwrap_or(skeleton.default_transforms(spine.id())?.local.rotation);
    let index = definition
        .bones
        .iter()
        .position(|bone| bone.name == "spine_01")?;
    let after = state
        .snapshot_hierarchy()?
        .transform(index, RigSpace::Local, false)
        .ok()?
        .rotation;
    let angle = before
        .as_dquat()
        .normalize()
        .angle_between(after.normalize()) as f32;
    (weights
        .iter()
        .all(|weight| weight.is_finite() && (*weight - 1.0).abs() <= 1e-5)
        && angle.is_finite()
        && angle > 1e-4)
        .then_some([weights[0], weights[1], angle])
}

fn set_manual_rig_curve_input(
    graph: &mut AnimationGraph,
    driver: &mut Option<NodeId>,
    values: &[(&str, f32)],
) {
    use bevy_als::extensions::{
        root_graph_boundaries::ControlRigBoundaryNode, source_pose_nodes::SourceCurveModifyNode,
    };
    use bevy_animation_graph::core::animation_node::{AnimationNode, dyn_node_like::DynNodeLike};
    let (curve_names, pin_defaults): (Vec<_>, Vec<_>) = values
        .iter()
        .map(|(name, value)| ((*name).to_owned(), *value))
        .unzip();
    if curve_names.is_empty() {
        if let Some(node) = driver.take() {
            let target = graph
                .edges
                .get(&SourcePin::NodeData(node, "Pose".into()))
                .cloned();
            let source = graph.remove_edge_by_target(&TargetPin::NodeData(node, "Source".into()));
            if let (Some(target), Some(source)) = (target, source) {
                graph.remove_edge_by_target(&target);
                graph.add_edge(source, target);
            }
            graph.remove_node(node);
        }
        return;
    }
    let modifier = SourceCurveModifyNode {
        source_pose_pin: "Source".into(),
        curve_names,
        pin_defaults,
        alpha: 1.0,
        ..default()
    };
    if let Some(node) = driver.and_then(|node| graph.nodes.get_mut(&node)) {
        node.inner = DynNodeLike::new(modifier);
        return;
    }
    let Some(target) = graph.nodes.values().find_map(|node| {
        node.try_inner_downcast_ref::<ControlRigBoundaryNode>()
            .map(|rig| TargetPin::NodeData(node.id, rig.pose_input.clone()))
    }) else {
        return;
    };
    let Some(source) = graph.remove_edge_by_target(&target) else {
        return;
    };
    let node = AnimationNode::new("Manual Rig curve input", modifier);
    graph.add_edge(source, TargetPin::NodeData(node.id, "Source".into()));
    graph.add_edge(SourcePin::NodeData(node.id, "Pose".into()), target);
    *driver = Some(node.id);
    graph.add_node(node);
}

fn drive_manual_rig_curves(
    state: Res<ManualIntegrationState>,
    weights: Res<AlsFootIkWeightOverride>,
    avatars: Query<(&ChildOf, &AnimatedSceneInstance), With<AlsCharacterAvatar>>,
    players: Query<&AnimationGraphPlayer>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    mut driver: Local<Option<(bevy::asset::AssetId<AnimationGraph>, NodeId)>>,
) {
    let values: Vec<_> = if state.scenario == IntegrationScenario::FootIk
        && state.phase == IntegrationPhase::FootIkWeightRamp
    {
        [("FootLeftIk", weights.left), ("FootRightIk", weights.right)]
            .into_iter()
            .filter_map(|(name, value)| value.map(|value| (name, value)))
            .collect()
    } else if state.scenario == IntegrationScenario::ControlRig
        && matches!(
            state.phase,
            IntegrationPhase::ControlRig | IntegrationPhase::Finish
        )
    {
        vec![
            ("PoseAiming", 1.0),
            ("HandLeftIk", 1.0),
            ("HandRightIk", 1.0),
        ]
    } else {
        Vec::new()
    };
    if let Some((graph_id, node)) = driver.take() {
        if let Some(mut graph) = graphs.get_mut(graph_id) {
            let mut node = Some(node);
            set_manual_rig_curve_input(&mut graph, &mut node, &values);
            *driver = node.map(|node| (graph_id, node));
        }
        return;
    }
    if values.is_empty() {
        return;
    }
    let Some((_, instance)) = avatars
        .iter()
        .find(|(parent, _)| Some(parent.parent()) == state.character)
    else {
        return;
    };
    let Ok(player) = players.get(instance.player_entity()) else {
        return;
    };
    let Some((context, _, _)) =
        bevy_als::extensions::root_graph_boundaries::current_control_rig(player, &graphs)
    else {
        return;
    };
    let graph_id = context.get_graph_id();
    let Some(mut graph) = graphs.get_mut(graph_id) else {
        return;
    };
    let mut node = None;
    set_manual_rig_curve_input(&mut graph, &mut node, &values);
    if node.is_some() {
        info!("[MANUAL-INTEGRATION][RIG-CURVE-INPUT] values={values:?}");
    }
    *driver = node.map(|node| (graph_id, node));
}

#[cfg(test)]
pub(crate) fn run_manual_rig_curve_fixture(
    world: &mut World,
    actor: Entity,
    weights: AlsFootIkWeightOverride,
    control_rig: bool,
) {
    world.insert_resource(ManualIntegrationState {
        character: Some(actor),
        scenario: if control_rig {
            IntegrationScenario::ControlRig
        } else {
            IntegrationScenario::FootIk
        },
        phase: if control_rig {
            IntegrationPhase::ControlRig
        } else if weights.left.is_some() || weights.right.is_some() {
            IntegrationPhase::FootIkWeightRamp
        } else {
            IntegrationPhase::FootIkTeleport
        },
        ..default()
    });
    world.insert_resource(weights);
    world.run_system_cached(drive_manual_rig_curves).unwrap();
}

fn drive_crouch_direction_reference_frame(
    fixture: Option<Res<ManualIntegrationFixture>>,
    state: Res<ManualIntegrationState>,
    mut characters: Query<
        (&mut Transform, Option<&mut Rotation>, &mut AlsViewYaw),
        With<AlsCharacter>,
    >,
) {
    if state.phase != IntegrationPhase::CrouchDirectionRegression {
        return;
    }
    if !crouch_direction_reference_frame_active(&state.coverage.crouch_direction_cases) {
        return;
    }
    let Some(fixture) = fixture else {
        return;
    };
    let Some(character) = state.character else {
        return;
    };
    let Ok((mut transform, rotation, mut view_yaw)) = characters.get_mut(character) else {
        return;
    };
    let rotation_value = Quat::from_rotation_y(fixture.facing_yaw);
    transform.rotation = rotation_value;
    if let Some(mut rotation) = rotation {
        rotation.0 = rotation_value;
    }
    view_yaw.0 = fixture.facing_yaw;
}

fn crouch_direction_reference_frame_active(cases: &[bool; CROUCH_DIRECTION_CASE_COUNT]) -> bool {
    cases.iter().any(|covered| !covered)
}

#[derive(Resource)]
struct ManualIntegrationFixture {
    platform: Entity,
    platform_home: Vec3,
    locomotion_start: Vec3,
    mantle_start: Vec3,
    roll_start: Vec3,
    facing_yaw: f32,
    obstacle_mesh: Handle<Mesh>,
    obstacle_material: Handle<StandardMaterial>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum IntegrationPhase {
    #[default]
    Await,
    DirectionRegression,
    StandingResponse,
    StandingStop,
    StandingTurn,
    StandingDynamic,
    Sprint,
    FootIkNoHit,
    FootIkWeightRamp,
    FootIkTeleport,
    FootIkTerrain,
    ControlRig,
    DefaultOverlay,
    BowOverlay,
    Overlays,
    Footprints,
    Head,
    StateMachineGrounded,
    StateMachineAir,
    StateMachineRotate,
    TransitionNotify,
    Locomotion,
    CrouchRecovery,
    CrouchClearance,
    CrouchDirectionRegression,
    Turn,
    Mantle,
    Platform,
    Roll,
    GroundedJoint,
    LocomotionActions,
    Ragdoll,
    Camera,
    Stairs,
    UphillJump,
    MovingBase,
    Interface,
    Notify,
    Finish,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum CrouchClearanceStage {
    #[default]
    AwaitStanding,
    Cycling,
    PrepareCanopyCrouch,
    StabilizeCanopyCrouch,
    ArmCanopy,
    AwaitCanopyReady,
    RequestStanding,
    AwaitBlocked,
    MoveAway,
    AwaitRecovery,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IntegrationScenario {
    Base,
    Grounded,
    StandingDirection,
    StandingResponse,
    StandingStop,
    StandingTurn,
    StandingDynamic,
    Crouching,
    CrouchingActions,
    LocomotionActions,
    ControlRig,
    DefaultOverlay,
    BowOverlay,
    Overlays,
    Footprint,
    Head,
    StateMachine,
    TransitionNotify,
    FootIk,
    Sprint,
    Ragdoll,
    GetUp,
    Camera,
    Stairs,
    UphillJump,
    MovingBase,
    RotatingBase,
    Interface,
    Notify,
    Visibility,
    AnimationClock,
}

impl IntegrationScenario {
    fn from_environment() -> Self {
        match std::env::var("ALS_MANUAL_INTEGRATION_CASE").ok().as_deref() {
            Some("grounded") => Self::Grounded,
            Some("standing_direction") => Self::StandingDirection,
            Some("standing_response") => Self::StandingResponse,
            Some("standing_stop") => Self::StandingStop,
            Some("standing_turn") => Self::StandingTurn,
            Some("standing_dynamic") => Self::StandingDynamic,
            Some("crouching") => Self::Crouching,
            Some("crouching_actions") => Self::CrouchingActions,
            Some("locomotion") => Self::LocomotionActions,
            Some("control_rig") => Self::ControlRig,
            Some("default_overlay") => Self::DefaultOverlay,
            Some("bow") => Self::BowOverlay,
            Some("overlays") => Self::Overlays,
            Some("footprint") => Self::Footprint,
            Some("head") => Self::Head,
            Some("state_machine") => Self::StateMachine,
            Some("transition_notify") => Self::TransitionNotify,
            Some("foot_ik") => Self::FootIk,
            Some("sprint") => Self::Sprint,
            Some("ragdoll") => Self::Ragdoll,
            Some("get_up") => Self::GetUp,
            Some("camera") => Self::Camera,
            Some("stairs") => Self::Stairs,
            Some("uphill_jump") => Self::UphillJump,
            Some("moving_base") => Self::MovingBase,
            Some("rotating_base") => Self::RotatingBase,
            Some("interface") => Self::Interface,
            Some("notify") => Self::Notify,
            Some("visibility") => Self::Visibility,
            Some("animation_clock") => Self::AnimationClock,
            _ => Self::Base,
        }
    }
}

fn initial_phase(scenario: IntegrationScenario) -> IntegrationPhase {
    match scenario {
        IntegrationScenario::Base
        | IntegrationScenario::StandingDirection
        | IntegrationScenario::StateMachine
        | IntegrationScenario::FootIk => IntegrationPhase::DirectionRegression,
        IntegrationScenario::StandingResponse => IntegrationPhase::StandingResponse,
        IntegrationScenario::StandingStop | IntegrationScenario::CrouchingActions => {
            IntegrationPhase::StandingStop
        }
        IntegrationScenario::StandingTurn => IntegrationPhase::StandingTurn,
        IntegrationScenario::StandingDynamic => IntegrationPhase::StandingDynamic,
        IntegrationScenario::TransitionNotify => IntegrationPhase::TransitionNotify,
        IntegrationScenario::Grounded => IntegrationPhase::Roll,
        IntegrationScenario::LocomotionActions => IntegrationPhase::LocomotionActions,
        IntegrationScenario::Sprint => IntegrationPhase::Sprint,
        IntegrationScenario::ControlRig => IntegrationPhase::ControlRig,
        IntegrationScenario::DefaultOverlay => IntegrationPhase::DefaultOverlay,
        IntegrationScenario::BowOverlay => IntegrationPhase::BowOverlay,
        IntegrationScenario::Overlays => IntegrationPhase::Overlays,
        IntegrationScenario::Footprint => IntegrationPhase::Footprints,
        IntegrationScenario::Head => IntegrationPhase::Head,
        IntegrationScenario::Crouching => IntegrationPhase::CrouchDirectionRegression,
        IntegrationScenario::Ragdoll | IntegrationScenario::GetUp => IntegrationPhase::Ragdoll,
        IntegrationScenario::Camera => IntegrationPhase::Camera,
        IntegrationScenario::Stairs => IntegrationPhase::Stairs,
        IntegrationScenario::UphillJump => IntegrationPhase::UphillJump,
        IntegrationScenario::MovingBase | IntegrationScenario::RotatingBase => {
            IntegrationPhase::MovingBase
        }
        IntegrationScenario::Interface => IntegrationPhase::Interface,
        IntegrationScenario::Notify
        | IntegrationScenario::Visibility
        | IntegrationScenario::AnimationClock => IntegrationPhase::Notify,
    }
}

const LOCOMOTION_ROOT: &str =
    "ALS/Character/AnimationInstances/AB_Als_Locomotion/AnimGraph.animgraph.ron";
const LOCOMOTION_JUMP: &str = "ALS/Character/AnimationInstances/AB_Als_Locomotion/AnimGraph/SM9_Locomotion_States/S2_Jump.animgraph.ron";
const LOCOMOTION_FALL: &str = "ALS/Character/AnimationInstances/AB_Als_Locomotion/AnimGraph/SM9_Locomotion_States/S1_Fall.animgraph.ron";
const DEFAULT_OVERLAY_GRAPH: &str =
    "ALS/Character/AnimationInstances/Overlays/AB_Als_Default/Overlay.animgraph.ron";
const LAYERING_GRAPH: &str =
    "ALS/Character/AnimationInstances/AB_Als_Layering/AnimGraph.animgraph.ron";
const LAYERING_BOUNDARIES: [(&str, &str, &str); 11] = [
    (
        "layering_locomotion",
        "Input Pose\nLocomotion Input",
        "Pose",
    ),
    ("layering_overlay", "Input Pose\nOverlay Input", "Pose"),
    (
        "layering_base_poses",
        "Save cached pose 'Base Poses'",
        CACHED_POSE_VALUE,
    ),
    (
        "layering_legs",
        "Save cached pose 'Legs'",
        CACHED_POSE_VALUE,
    ),
    (
        "layering_pelvis",
        "Save cached pose 'Pelvis'",
        CACHED_POSE_VALUE,
    ),
    (
        "layering_spine",
        "Save cached pose 'Spine'",
        CACHED_POSE_VALUE,
    ),
    (
        "layering_head",
        "Save cached pose 'Head'",
        CACHED_POSE_VALUE,
    ),
    (
        "layering_arm_left",
        "Save cached pose 'Arm Left'",
        CACHED_POSE_VALUE,
    ),
    (
        "layering_arm_right",
        "Save cached pose 'Arm Right'",
        CACHED_POSE_VALUE,
    ),
    (
        "layering_fingers",
        "d650094f-41a3-2706-67fb-b5a5994b2139",
        "Pose",
    ),
    ("layering_output", "Output Pose\nAnimGraph", "Pose"),
];
const DEFAULT_OVERLAY_SYNC: &str = "sync_groups.secondary_motion.phase";
const DEFAULT_OVERLAY_EVALUATORS: [&str; 5] = [
    "b8f30b3f-4d30-eab1-ce6b-6bb9ec868234",
    "3890697f-4347-5dd1-f323-f28f64443083",
    "560a6265-48e4-0f6a-f34a-dbb080a5c73a",
    "8a5fdbcc-4f32-99cb-55d9-0d8111d1c8b0",
    "75971f25-4595-b9cd-095b-82b4a661595d",
];
const DEFAULT_OVERLAY_WINDOWS: [&str; 6] = [
    "default_idle",
    "default_walk",
    "default_crouch",
    "default_air",
    "default_prediction",
    "default_return",
];

const HEAD_GRAPH: &str = "ALS/Character/AnimationInstances/AB_Als_Head/AnimGraph.animgraph.ron";
const HEAD_WINDOWS: [&str; 7] = [
    "head_down",
    "head_forward",
    "head_up_desired_aiming",
    "head_velocity_input",
    "head_velocity_release",
    "head_first_person",
    "head_return",
];
const HEAD_BOUNDARIES: [(&str, &str, &str); 5] = [
    ("head_look", "BS_Als_Look\nBlendspace Evaluator", "pose"),
    ("head_refresh", "RefreshHead\r\nOn Update", "Pose"),
    (
        "head_initialize",
        "InitializeHead\r\nOn Become Relevant",
        "Pose",
    ),
    ("head_additive", "Apply Mesh Space Additive", "Pose"),
    ("head_output", "Output Pose\nAnimGraph", "Pose"),
];

#[derive(Default, Debug, Serialize)]
struct HeadGraphSample {
    ready: bool,
    elapsed_seconds: f32,
    frame: u64,
    refreshed: bool,
    alpha: f32,
    state: [f32; 4],
    inputs: [f32; 8],
    modes: [bool; 3],
    weights: [f32; 3],
    times: [Option<f32>; 3],
    sample_time: Option<f32>,
    additive_effect: Option<f32>,
    errors: Vec<String>,
}

#[derive(Default)]
struct HeadProbe {
    case_index: usize,
    sample: HeadGraphSample,
    previous: Option<(u64, AlsHeadState, bool, f32)>,
    covered: [bool; 7],
    seconds: [f32; 7],
    samples_seen: [bool; 3],
    valid: bool,
    unscaled_pitch_seen: bool,
    additive_seen: bool,
}

fn head_look_weights(pitch: f32) -> [f32; 3] {
    let amount = (pitch / 90.0).clamp(-1.0, 1.0);
    [1.0 - amount.abs(), (-amount).max(0.0), amount.max(0.0)]
}

fn head_weighted_pose(poses: [Option<Pose>; 3], weights: [f32; 3]) -> Option<Pose> {
    poses
        .into_iter()
        .zip(weights)
        .filter(|(_, weight)| *weight > 1e-5)
        .map(|(pose, weight)| {
            pose.filter(|pose| !pose.bones.is_empty())
                .map(|pose| pose.scalar_mult(weight))
        })
        .collect::<Option<Vec<_>>>()?
        .into_iter()
        .reduce(|a, b| a.linear_add(&b))
        .map(|pose| pose.normalize_quat())
}

fn head_state_values(state: AlsHeadState) -> [f32; 4] {
    [
        state.pitch_angle,
        state.yaw_angle,
        state.yaw_velocity,
        state.yaw_amount,
    ]
}

fn head_actor_world_inputs_match(
    player: &AnimationGraphPlayer,
    local: &Transform,
    world: &GlobalTransform,
) -> bool {
    let expected_yaw =
        -bevy_als::als::orientation::world_forward_to_yaw(*world.forward()).to_degrees();
    world.rotation().angle_between(local.rotation).abs() > 0.5
        && graph_global_f32(player, "locomotion_state.rotation_yaw")
            .is_some_and(|yaw| bevy_als::als::head::unwind_degrees(yaw - expected_yaw).abs() < 1e-3)
        && player
            .get_global_input_data("locomotion_state.rotation_quaternion_world_space")
            .and_then(|value| value.as_quat().ok())
            .is_some_and(|rotation| {
                rotation.abs_diff_eq(world.rotation(), 1e-6)
                    || rotation.abs_diff_eq(-world.rotation(), 1e-6)
            })
}

fn head_graph_sample(
    player: &AnimationGraphPlayer,
    probe: &MotionProbe,
    previous: Option<&(u64, AlsHeadState, bool, f32)>,
) -> HeadGraphSample {
    use bevy_als::als::head::{AlsHeadSettings, AlsHeadUpdateInput, update_head_state};
    let scalar = |key| graph_global_f32(player, key).unwrap_or(0.0);
    let boolean = |key| {
        player
            .get_global_input_data(key)
            .and_then(|value| value.as_bool().ok())
            .unwrap_or(false)
    };
    let state = bevy_als::head_layer::head_state_from_player(player);
    let mut sample = HeadGraphSample {
        alpha: scalar("view_state.head_blend_amount"),
        state: head_state_values(state),
        weights: head_look_weights(state.pitch_angle),
        inputs: [
            "animation.delta_seconds",
            "animation.delta_seconds_without_time_dilation",
            "locomotion_state.input_yaw_angle_world_space",
            "locomotion_state.target_yaw_angle_world_space",
            "locomotion_state.rotation_yaw",
            "locomotion_state.yaw_velocity_world_space",
            "view_state.yaw_angle",
            "view_state.pitch_angle",
        ]
        .map(scalar),
        modes: [
            "view_mode.first_person",
            "rotation_mode.velocity_direction",
            "locomotion_state.b_has_input",
        ]
        .map(boolean),
        ..default()
    };
    let Some(arena) = player.get_context_arena() else {
        return sample;
    };
    sample.frame = arena.frame_index();
    sample.refreshed = arena
        .get_toplevel()
        .node_states
        .get::<SourceInstanceProperties>(NodeId::default(), StateKey::Default)
        .is_ok_and(|properties| {
            properties.write_frames.get("head_state.yaw_amount") == Some(&sample.frame)
        });
    if let Some((_, previous_state, refreshed, previous_alpha)) =
        previous.filter(|(frame, _, _, _)| frame + 1 == sample.frame)
    {
        if (sample.alpha - previous_alpha).abs() > 1e-5 {
            sample
                .errors
                .push("head alpha did not consume previous root curves".into());
        }
        let mut expected = *previous_state;
        if sample.refreshed {
            if !refreshed {
                expected.initialization_required = true;
            }
            update_head_state(
                &mut expected,
                AlsHeadUpdateInput {
                    delta_seconds: sample.inputs[0],
                    delta_seconds_without_time_dilation: sample.inputs[1],
                    first_person: boolean("view_mode.first_person"),
                    rotation_mode: if boolean("rotation_mode.velocity_direction") {
                        RotationMode::VelocityDirection
                    } else {
                        RotationMode::ViewDirection
                    },
                    has_input: boolean("locomotion_state.b_has_input"),
                    input_yaw_world: sample.inputs[2],
                    target_yaw_world: sample.inputs[3],
                    body_yaw_world: sample.inputs[4],
                    body_yaw_velocity_world: sample.inputs[5],
                    view_yaw_angle: sample.inputs[6],
                    view_pitch_angle: sample.inputs[7],
                    movement_base_relative_yaw: if boolean("movement_base.b_has_relative_rotation")
                    {
                        scalar("movement_base.delta_yaw")
                    } else {
                        0.0
                    },
                },
                AlsHeadSettings::default(),
            );
        }
        if head_state_values(expected)
            .into_iter()
            .zip(sample.state)
            .any(|(a, b)| !b.is_finite() || (a - b).abs() > 1e-4)
            || expected.initialization_required != state.initialization_required
            || expected.switching_look_sides != state.switching_look_sides
        {
            sample
                .errors
                .push("Head callback output differs from source update".into());
        }
    }
    let check =
        |errors: &mut Vec<String>, label: &str, actual: Option<Pose>, expected: Option<Pose>| {
            if actual
                .zip(expected)
                .and_then(|(actual, expected)| default_overlay_pose_error(&actual, &expected))
                .is_none_or(|error| error > 2e-5)
            {
                errors.push(format!("{label}: missing or incorrect Head pose"));
            }
        };
    for id in arena.iter_context_ids() {
        let Some(context) = arena.get_context(id) else {
            continue;
        };
        if probe
            .asset_server
            .get_path(context.get_graph_id())
            .is_none_or(|path| path.path() != std::path::Path::new(HEAD_GRAPH))
        {
            continue;
        }
        let Some(graph) = probe.graphs.get(context.get_graph_id()) else {
            continue;
        };
        let data = |name: &str, output: &str| {
            graph
                .nodes
                .values()
                .find(|node| node.name == name)
                .filter(|node| context.node_caches.is_updated(node.id, StateKey::Default))
                .and_then(|node| {
                    context
                        .node_caches
                        .get_output_data(node.id, StateKey::Default, output.into())
                })
        };
        let pose =
            |name: &str, output: &str| data(name, output).and_then(|value| value.into_pose().ok());
        let base = pose("Input Pose\nView Input", "Pose");
        let actual = pose("Apply Mesh Space Additive", "Pose");
        if actual.is_none() {
            continue;
        }
        if sample.refreshed != (sample.alpha > 1e-5) {
            sample
                .errors
                .push("Head callback relevance differs from additive alpha".into());
        }
        let mut expected_samples: [Option<Pose>; 3] = default();
        let mut source_samples = Vec::new();
        for index in 0..3 {
            if sample.weights[index] <= 1e-5 || sample.alpha <= 1e-5 {
                continue;
            }
            let name = format!("Blendspace sample {index}");
            let Some(node) = graph.nodes.values().find(|node| node.name == name) else {
                continue;
            };
            let Some(clip_node) = node
                .try_inner_downcast_ref::<bevy_animation_graph::builtin_nodes::clip_node::ClipNode>(
                )
            else {
                continue;
            };
            let Some(clip) = clip_node.reflect_ref().as_struct().ok().and_then(|node| node.field("clip"))
                .and_then(|field| field.try_downcast_ref::<Handle<bevy_animation_graph::core::animation_clip::GraphClip>>())
                .and_then(|handle| probe.clips.get(handle)) else { continue; };
            let time = state.yaw_amount.clamp(0.0, 1.0) * clip.duration;
            let clip_pose = pose(&name, "pose");
            sample.times[index] = clip_pose.as_ref().map(|pose| pose.timestamp);
            if sample.times[index].is_none_or(|actual| (actual - time).abs() > 1e-5) {
                sample
                    .errors
                    .push(format!("Look sample {index}: incorrect normalized time"));
            }
            let expected = bevy_als::extensions::source_sequence_player::sample_pose(clip, time);
            check(&mut sample.errors, &name, clip_pose, Some(expected.clone()));
            let difference_name = format!("additive difference {name}");
            let reference = graph
                .nodes
                .values()
                .find(|node| node.name == difference_name)
                .and_then(|node| {
                    graph
                        .edges_inverted
                        .get(&TargetPin::NodeData(node.id, "reference_pose".into()))
                })
                .and_then(|pin| match pin {
                    SourcePin::NodeData(node, pin) => {
                        context
                            .node_caches
                            .get_output_data(*node, StateKey::Default, pin.clone())
                    }
                    _ => None,
                })
                .and_then(|value| value.into_pose().ok());
            if let Some(mut reference) = reference
                && let Some(skeleton) = probe.skeletons.get(&clip.skeleton)
            {
                let mut expected = expected;
                bevy_als::extensions::source_sequence_player::rebuild_virtual_bones(
                    &mut reference,
                    skeleton,
                );
                bevy_als::extensions::source_sequence_player::rebuild_virtual_bones(
                    &mut expected,
                    skeleton,
                );
                if reference.timestamp.abs() > 1e-5 {
                    sample
                        .errors
                        .push("Look reference frame is not zero".into());
                }
                expected_samples[index] = Some(
                    bevy_als::extensions::mesh_space_difference::make_mesh_space_difference(
                        &reference, &expected, skeleton,
                    ),
                );
                if let Some(source) =
                    bevy_als::extensions::source_pose::SourcePose::from_complete_pose(
                        &reference, skeleton,
                    )
                    .zip(
                        bevy_als::extensions::source_pose::SourcePose::from_complete_pose(
                            &expected, skeleton,
                        ),
                    )
                    .and_then(|(base, target)| {
                        bevy_als::extensions::mesh_space_difference::source_pose_difference(
                            &base, &target, skeleton, true,
                        )
                    })
                {
                    source.project_into(expected_samples[index].as_mut().unwrap());
                    source_samples.push((source, sample.weights[index]));
                }
                check(
                    &mut sample.errors,
                    &difference_name,
                    pose(&difference_name, "pose"),
                    expected_samples[index].clone(),
                );
            }
        }
        let source_additive = (source_samples.len()
            == sample
                .weights
                .iter()
                .filter(|weight| **weight > 1e-5)
                .count())
        .then(|| {
            bevy_als::extensions::source_pose::blend_source_poses(
                source_samples.iter().map(|(pose, weight)| (pose, *weight)),
                1 + usize::from(source_samples.len() > 1),
            )
        })
        .flatten();
        let additive = if let Some(source) = &source_additive {
            expected_samples
                .into_iter()
                .flatten()
                .next()
                .map(|mut pose| {
                    source.project_into(&mut pose);
                    pose
                })
        } else {
            head_weighted_pose(expected_samples, sample.weights)
        };
        sample.sample_time =
            data(HEAD_BOUNDARIES[0].1, "sample_time").and_then(|value| value.as_f32().ok());
        if sample.alpha > 1e-5 {
            for (_, name, output) in &HEAD_BOUNDARIES[..3] {
                check(
                    &mut sample.errors,
                    name,
                    pose(name, output),
                    additive.clone(),
                );
            }
        }
        if let Some(mut expected) = base
            && let Some(skeleton) = probe.skeletons.get(&expected.skeleton)
        {
            let source_base = graph
                .nodes
                .values()
                .find(|node| node.name == "Input Pose\nView Input")
                .and_then(|node| {
                    bevy_als::extensions::animation_context::node_context_state(
                        &context.node_states,
                        StateKey::Default,
                        node.id,
                    )
                })
                .and_then(|state| state.source_pose_for(&expected).cloned());
            if source_base.is_none() {
                bevy_als::extensions::mesh_space_additive::materialize_missing_reference_bones(
                    &mut expected,
                    skeleton,
                );
            }
            sample.additive_effect = actual
                .as_ref()
                .and_then(|actual| default_overlay_pose_error(actual, &expected));
            if let Some(additive) = additive {
                bevy_als::extensions::mesh_space_additive::apply_source_additive_pose(
                    &mut expected,
                    &additive,
                    source_base.as_deref().zip(source_additive.as_ref()),
                    sample.alpha,
                    skeleton,
                    true,
                    true,
                );
            }
            check(
                &mut sample.errors,
                "mesh additive",
                actual.clone(),
                Some(expected),
            );
            check(
                &mut sample.errors,
                "Head output",
                pose("Output Pose\nAnimGraph", "Pose"),
                actual,
            );
            sample.ready = sample.alpha > 1e-5
                && sample.refreshed
                && sample.sample_time.is_some()
                && sample.errors.is_empty();
        }
    }
    sample
}

fn head_window(index: usize, sample: &HeadGraphSample) -> ManualAnimationWindowEvidence {
    ManualAnimationWindowEvidence {
        label: Some(HEAD_WINDOWS[index]),
        index: Some(index),
        settled: true,
        capture_source_window: true,
        capture_every_frame: true,
        requires_pre_rig_motion: true,
        requires_final_pose_motion: true,
        requires_non_reference_root_pose: true,
        requires_source_pose_chain: true,
        source_node: Some("AB_Als_Head/AnimGraph"),
        source_state: Some(HEAD_WINDOWS[index]),
        source_pose_chain: HEAD_BOUNDARIES
            .into_iter()
            .enumerate()
            .filter(|(boundary, _)| index != 4 || *boundary >= 3)
            .map(|(_, (label, node, output))| ManualAnimationPoseBoundary {
                label,
                graph: HEAD_GRAPH,
                node,
                output,
                minimum_translation_span: 0.0,
                minimum_rotation_span_radians: if index < 3 || index >= 5 { 0.001 } else { 0.0 },
            })
            .collect(),
        minimum_pre_rig_rotation_span_radians: 0.001,
        minimum_final_rotation_span_radians: 0.001,
        minimum_observation_seconds: 0.3,
        observed_animation_clip_time: sample.sample_time,
        observed_animation_weight: Some(sample.alpha),
        observed_animation_play_rate: Some(0.0),
        ..default()
    }
}

#[derive(Default, Debug, Serialize)]
struct DefaultOverlayGraphSample {
    ready: bool,
    inputs: [Option<f32>; 4],
    evaluator_times: [Option<f32>; 5],
    expected_times: [Option<f32>; 5],
    prediction_input: Option<f32>,
    prediction_alpha: Option<f32>,
    prediction_expected: Option<f32>,
    idle_time: Option<f32>,
    idle_duration: Option<f32>,
    idle_phase: Option<f32>,
    idle_weight: Option<f32>,
    idle_rate: Option<f32>,
    additive_alpha: Option<f32>,
    additive_effect: Option<f32>,
    layering: LayeringGraphSample,
    errors: Vec<String>,
}

#[derive(Default)]
struct DefaultOverlayProbe {
    case_index: usize,
    sample: DefaultOverlayGraphSample,
    covered: [bool; 6],
    seconds: [f32; 6],
    canonical_seen: [bool; 3],
    evaluators_seen: [bool; 5],
    valid: bool,
    curves_seen: bool,
    additive_seen: bool,
    prediction_previous: Option<f32>,
    previous_layering: Option<(u64, AlsLayeringState)>,
    prediction_increased: bool,
    prediction_decreased: bool,
    jump_started_at: Option<f64>,
    airborne_seen: bool,
    jump_completed: bool,
}

impl DefaultOverlayProbe {
    fn air_window(&self, prediction: Option<f32>) -> Option<usize> {
        if !self.airborne_seen || !(self.case_index == 3 || self.case_index == 4) {
            return None;
        }
        if prediction.is_some_and(|value| value < 0.1) {
            Some(3)
        } else if prediction.is_some_and(|value| value > 0.1 && value < 0.99) {
            Some(4)
        } else {
            None
        }
    }
}

fn default_overlay_pose_error(actual: &Pose, expected: &Pose) -> Option<f32> {
    if actual.paths.len() != expected.paths.len() {
        return None;
    }
    let mut error = 0.0_f32;
    for (id, index) in &expected.paths {
        let actual = actual.get_bone(*id)?;
        let expected = expected.bones.get(*index)?;
        for (a, b) in [
            (actual.translation, expected.translation),
            (actual.scale, expected.scale),
        ] {
            match (a, b) {
                (Some(a), Some(b)) if a.is_finite() && b.is_finite() => {
                    error = error.max(a.distance(b))
                }
                (None, None) => {}
                _ => return None,
            }
        }
        match (actual.rotation, expected.rotation) {
            (Some(a), Some(b)) if a.is_finite() && b.is_finite() => {
                error = error.max((a - b).length().min((a + b).length()))
            }
            (None, None) => {}
            _ => return None,
        }
    }
    Some(error)
}

fn active_layer_pose(player: &AnimationGraphPlayer, probe: &MotionProbe, path: &str) -> bool {
    let Some(arena) = player.get_context_arena() else {
        return false;
    };
    arena
        .iter_context_ids()
        .filter_map(|id| arena.get_context(id))
        .any(|context| {
            if probe
                .asset_server
                .get_path(context.get_graph_id())
                .is_none_or(|asset| asset.path() != std::path::Path::new(path))
            {
                return false;
            }
            let Some(graph) = probe.graphs.get(context.get_graph_id()) else {
                return false;
            };
            let Some(SourcePin::NodeData(node, pin)) = graph
                .edges_inverted
                .get(&TargetPin::OutputData("Result".to_owned()))
            else {
                return false;
            };
            context.node_caches.is_updated(*node, StateKey::Default)
                && context
                    .node_caches
                    .get_output_data(*node, StateKey::Default, pin.clone())
                    .and_then(|value| value.into_pose().ok())
                    .is_some_and(|pose| !pose.paths.is_empty())
        })
}

#[derive(Default, Debug, Serialize)]
struct LayeringGraphSample {
    ready: bool,
    input_curve_frame: Option<u64>,
    evaluation_frame: Option<u64>,
    inputs: std::collections::BTreeMap<String, (Option<f32>, f32)>,
    boundaries: Vec<&'static str>,
    base_evaluators: usize,
    curve_operators: usize,
    errors: Vec<String>,
}

fn layering_input_sample(
    player: &AnimationGraphPlayer,
    previous: AlsLayeringState,
) -> std::collections::BTreeMap<String, (Option<f32>, f32)> {
    bevy_als::layering_layer::layering_state_inputs(previous)
        .into_iter()
        .map(|(key, expected)| {
            (
                key.into(),
                (locomotion_source_scalar(player, key), expected),
            )
        })
        .collect()
}

fn layering_curve_error(
    actual: &bevy_als::extensions::SourceCurveValues,
    expected: &bevy_als::extensions::SourceCurveValues,
) -> Option<f32> {
    if actual.len() != expected.len() {
        return None;
    }
    expected
        .iter()
        .try_fold(0.0_f32, |error, (name, expected)| {
            let actual = actual.get(name)?;
            (actual.is_finite() && expected.is_finite())
                .then_some(error.max((actual - expected).abs()))
        })
}

fn layering_graph_sample(
    player: &AnimationGraphPlayer,
    probe: &MotionProbe,
    previous: Option<&(u64, AlsLayeringState)>,
) -> LayeringGraphSample {
    let mut sample = LayeringGraphSample::default();
    let Some(arena) = player.get_context_arena() else {
        return sample;
    };
    sample.evaluation_frame = Some(arena.frame_index());
    if let Some((frame, previous)) = previous.filter(|(frame, _)| *frame + 1 == arena.frame_index())
    {
        sample.input_curve_frame = Some(*frame);
        sample.inputs = layering_input_sample(player, *previous);
        for (key, (actual, expected)) in &sample.inputs {
            if actual.is_none_or(|actual| (actual - expected).abs() > 1e-5) {
                sample
                    .errors
                    .push(format!("{key}: previous curve input mismatch"));
            }
        }
    }
    let mut contexts = 0;
    for context_id in arena.iter_context_ids() {
        let Some(context) = arena.get_context(context_id) else {
            continue;
        };
        if probe
            .asset_server
            .get_path(context.get_graph_id())
            .is_none_or(|path| path.path() != std::path::Path::new(LAYERING_GRAPH))
        {
            continue;
        }
        let Some(graph) = probe.graphs.get(context.get_graph_id()) else {
            continue;
        };
        let pose = |id| {
            context
                .node_caches
                .is_updated(id, StateKey::Default)
                .then(|| {
                    context
                        .node_caches
                        .get_output_data(id, StateKey::Default, "Pose".into())
                })
                .flatten()
                .and_then(|value| value.into_pose().ok())
        };
        let input_id = |id, pin: &str| match graph
            .edges_inverted
            .get(&TargetPin::NodeData(id, pin.into()))
        {
            Some(SourcePin::NodeData(source, _)) => Some(*source),
            _ => None,
        };
        if graph
            .nodes
            .values()
            .find(|node| node.name == "Output Pose\nAnimGraph")
            .and_then(|node| pose(node.id))
            .is_none()
        {
            continue;
        }
        contexts += 1;
        for (label, name, output) in LAYERING_BOUNDARIES {
            if graph
                .nodes
                .values()
                .find(|node| node.name == name || node.id.uuid().to_string() == name)
                .filter(|node| context.node_caches.is_updated(node.id, StateKey::Default))
                .and_then(|node| {
                    context
                        .node_caches
                        .get_output_data(node.id, StateKey::Default, output.into())
                })
                .and_then(|value| value.into_pose().ok())
                .is_some_and(|pose| !pose.paths.is_empty())
            {
                sample.boundaries.push(label);
            }
        }
        for node in graph.nodes.values() {
            let Some(actual) = pose(node.id) else {
                continue;
            };
            if let Some(sequence) =
                node.try_inner_downcast_ref::<bevy_als::extensions::SourceSequenceEvaluatorNode>()
            {
                if let Some(clip) = probe.clips.get(&sequence.clip) {
                    let time = ((sequence.explicit_value as f64 / sequence.frames_per_second)
                        as f32)
                        .clamp(0.0, clip.duration);
                    let expected =
                        bevy_als::extensions::source_sequence_player::sample_source_clip_pose(
                            clip,
                            time,
                            &probe.skeletons,
                        );
                    let error = default_overlay_pose_error(&actual, &expected);
                    if (actual.timestamp - time).abs() > 1e-5
                        || error.is_none_or(|error| error > 2e-5)
                    {
                        sample.errors.push(format!(
                            "{}: source base pose frame mismatch",
                            node.id.uuid()
                        ));
                    }
                    sample.base_evaluators += 1;
                }
            }
            if let Some(blend) =
                node.try_inner_downcast_ref::<bevy_als::extensions::SourceTwoWayBlendNode>()
                && !blend.alpha_key.is_empty()
            {
                if let Ok(state) = context
                    .node_states
                    .get::<bevy_als::extensions::source_pose_nodes::SourceTwoWayBlendState>(
                    node.id,
                    StateKey::Default,
                ) {
                    if locomotion_source_scalar(player, &blend.alpha_key)
                        .is_none_or(|value| (value - state.input_alpha).abs() > 1e-5)
                    {
                        sample
                            .errors
                            .push(format!("{}: consumed alpha mismatch", blend.alpha_key));
                    }
                } else {
                    sample
                        .errors
                        .push(format!("{}: missing consumed alpha", blend.alpha_key));
                }
            }
            let modify = node.try_inner_downcast_ref::<bevy_als::extensions::SourceCurveModifyNode>();
            let blend = node.try_inner_downcast_ref::<bevy_als::extensions::SourceCurvesBlendNode>();
            if modify.is_none() && blend.is_none() {
                continue;
            }
            let base_pin = modify.map_or("BasePose", |node| node.source_pose_pin.as_str());
            let Some(base_id) = input_id(node.id, base_pin) else {
                sample.errors.push("missing curve base edge".into());
                continue;
            };
            let Some(base) = pose(base_id) else {
                sample.errors.push("missing curve base pose".into());
                continue;
            };
            if default_overlay_pose_error(&actual, &base).is_none_or(|error| error > 2e-5) {
                sample
                    .errors
                    .push("curve operator changed its source pose".into());
            }
            let Some(base_context) = bevy_als::extensions::animation_context::node_context_state(
                &context.node_states,
                StateKey::Default,
                base_id,
            ) else {
                sample.errors.push("missing base curve context".into());
                continue;
            };
            let Some(output_context) = bevy_als::extensions::animation_context::node_context_state(
                &context.node_states,
                StateKey::Default,
                node.id,
            ) else {
                sample.errors.push("missing output curve context".into());
                continue;
            };
            let mut expected = base_context.curves.clone();
            if let Some(modify) = modify {
                for (name, value) in modify.curve_names.iter().zip(&modify.pin_defaults) {
                    let name = bevy_als::extensions::SourceCurveName::from(name);
                    let base = expected.get(&name).copied().unwrap_or(0.0);
                    expected.insert(name, base + (*value - base) * modify.alpha);
                }
            }
            if let Some(blend) = blend {
                let Some(curve_id) = input_id(node.id, "CurvePose") else {
                    sample.errors.push("missing curve input edge".into());
                    continue;
                };
                let Some(curve_context) = bevy_als::extensions::animation_context::node_context_state(
                    &context.node_states,
                    StateKey::Default,
                    curve_id,
                ) else {
                    sample.errors.push("missing source curve context".into());
                    continue;
                };
                if blend.blend_mode
                    == bevy_als::extensions::source_pose_nodes::SourceCurvesBlendMode::Override
                {
                    expected.clear();
                }
                for (name, value) in &curve_context.curves {
                    *expected.entry(name.clone()).or_default() += value * blend.blend_amount;
                }
            }
            if layering_curve_error(&output_context.curves, &expected)
                .is_none_or(|error| error > 1e-5)
            {
                sample.errors.push(format!(
                    "{}: source curve arithmetic mismatch",
                    node.id.uuid()
                ));
            }
            sample.curve_operators += 1;
        }
    }
    sample.ready = contexts == 1
        && sample.input_curve_frame.is_some()
        && sample.boundaries.len() == LAYERING_BOUNDARIES.len()
        && sample.base_evaluators > 0
        && sample.curve_operators == 3;
    sample
}

fn default_overlay_prediction_alpha(
    previous: Option<f32>,
    raw: f32,
    delta: f32,
    mapping: &bevy_als::extensions::source_pose_nodes::SourceAlphaMapping,
) -> f32 {
    previous.map_or(raw, |previous| {
        bevy_als::extensions::f_interp_to(
            previous,
            raw,
            delta,
            if raw >= previous {
                mapping.interpolate_speed_increasing
            } else {
                mapping.interpolate_speed_decreasing
            },
        )
    })
}

fn default_overlay_foot_targets_ready(spaces: &AlsFootIkSpaceState) -> bool {
    [spaces.left, spaces.right].iter().all(|side| {
        side.solve_input.is_none()
            || side
                .trace_target_world
                .distance(side.solver_reference_world)
                < 1e-5
    })
}

fn default_overlay_additive_pose(base: &Pose, delta: &Pose, alpha: f32) -> Pose {
    use bevy_animation_graph::core::{
        edge_data::bone_mask::BoneMask, interpolation::additive::AdditiveInterpolator,
    };
    let mut expected = base.clone();
    AdditiveInterpolator {
        bone_mask: BoneMask::all(),
    }
    .interpolate_pose(&mut expected, &delta, alpha);
    for (bone, delta_index) in &delta.paths {
        let Some(source) = base.get_bone(*bone) else {
            continue;
        };
        let Some(index) = expected.paths.get(bone).copied() else {
            continue;
        };
        let delta_bone = &delta.bones[*delta_index];
        if let (Some(rotation), Some(delta_rotation)) = (source.rotation, delta_bone.rotation) {
            let sign = if delta_rotation.w < 0.0 { -1.0 } else { 1.0 };
            expected.bones[index].rotation = Some(
                ((Quat::IDENTITY * (sign * (1.0 - alpha)) + delta_rotation * alpha).normalize()
                    * rotation)
                    .normalize(),
            );
        }
        if let (Some(scale), Some(delta_scale)) = (source.scale, delta_bone.scale) {
            expected.bones[index].scale = Some(scale * (Vec3::ONE + delta_scale * alpha));
        }
    }
    expected
}

fn default_overlay_stance_pose(poses: [Option<Pose>; 2], weights: [f32; 2]) -> Option<Pose> {
    let poses = poses
        .into_iter()
        .zip(weights)
        .filter(|(_, weight)| *weight > 1e-5)
        .map(|(pose, weight)| pose.map(|pose| pose.scalar_mult(weight)))
        .collect::<Option<Vec<_>>>()?;
    poses
        .into_iter()
        .reduce(|a, b| a.linear_add(&b))
        .map(|pose| pose.normalize_quat())
}

fn default_overlay_graph_sample(
    player: &AnimationGraphPlayer,
    probe: &MotionProbe,
    previous_prediction: Option<f32>,
) -> DefaultOverlayGraphSample {
    let scalar = |key| locomotion_source_scalar(player, key);
    let mut sample = DefaultOverlayGraphSample {
        inputs: [
            "pose_state.standing_amount",
            "pose_state.crouching_amount",
            "pose_state.gait_walking_amount",
            "pose_state.in_air_amount",
        ]
        .map(scalar),
        ..default()
    };
    let Some(arena) = player.get_context_arena() else {
        return sample;
    };
    let ticks = bevy_als::extensions::sync_group_phase::source_sync_players(
        arena,
        &probe.graphs,
        DEFAULT_OVERLAY_SYNC,
    );
    for context_id in arena.iter_context_ids() {
        let Some(context) = arena.get_context(context_id) else {
            continue;
        };
        if probe
            .asset_server
            .get_path(context.get_graph_id())
            .is_none_or(|path| path.path() != std::path::Path::new(DEFAULT_OVERLAY_GRAPH))
        {
            continue;
        }
        let Some(graph) = probe.graphs.get(context.get_graph_id()) else {
            continue;
        };
        let pose = |id| {
            context
                .node_caches
                .is_updated(id, StateKey::Default)
                .then(|| {
                    context
                        .node_caches
                        .get_output_data(id, StateKey::Default, "Pose".into())
                })
                .flatten()
                .and_then(|value| value.into_pose().ok())
        };
        let input = |id, pin: &str| match graph
            .edges_inverted
            .get(&TargetPin::NodeData(id, pin.into()))
        {
            Some(SourcePin::NodeData(source, output)) if output == "Pose" => pose(*source),
            _ => None,
        };
        let check = |errors: &mut Vec<String>, label: &str, actual: &Pose, expected: &Pose| {
            if default_overlay_pose_error(actual, expected).is_none_or(|error| error > 2e-5) {
                errors.push(format!("{label}: pose mismatch"));
            }
        };
        for node in graph.nodes.values() {
            let Some(actual) = pose(node.id) else {
                continue;
            };
            if let Some(sequence) =
                node.try_inner_downcast_ref::<bevy_als::extensions::SourceSequenceEvaluatorNode>()
            {
                let Some(index) = DEFAULT_OVERLAY_EVALUATORS
                    .iter()
                    .position(|id| *id == node.id.uuid().to_string())
                else {
                    continue;
                };
                let Some(clip) = probe.clips.get(&sequence.clip) else {
                    continue;
                };
                let expected_time = ((sequence.explicit_value as f64 / sequence.frames_per_second)
                    as f32)
                    .clamp(0.0, clip.duration);
                sample.evaluator_times[index] = Some(actual.timestamp);
                sample.expected_times[index] = Some(expected_time);
                if (actual.timestamp - expected_time).abs() > 1e-5 {
                    sample
                        .errors
                        .push(format!("evaluator_{index}: sample time mismatch"));
                }
                let expected = bevy_als::extensions::source_sequence_player::sample_source_clip_pose(
                    clip,
                    expected_time,
                    &probe.skeletons,
                );
                let error = default_overlay_pose_error(&actual, &expected);
                if error.is_none_or(|error| error > 2e-5) {
                    sample.errors.push(format!(
                        "{}: pose mismatch",
                        DEFAULT_OVERLAY_EVALUATORS[index]
                    ));
                }
            }
            if let Some(blend) =
                node.try_inner_downcast_ref::<bevy_als::extensions::SourceTwoWayBlendNode>()
            {
                let Ok(state) = context
                    .node_states
                    .get::<bevy_als::extensions::source_pose_nodes::SourceTwoWayBlendState>(
                    node.id,
                    StateKey::Default,
                ) else {
                    continue;
                };
                let alpha = state.blend_alpha;
                let raw = state.input_alpha;
                let expected_alpha = if blend.mapping.interpolate_result {
                    let expected = previous_prediction.map(|previous| {
                        default_overlay_prediction_alpha(
                            Some(previous),
                            raw,
                            scalar("animation.delta_seconds").unwrap_or(0.0),
                            &blend.mapping,
                        )
                    });
                    sample.prediction_input = Some(raw);
                    sample.prediction_alpha = Some(alpha);
                    sample.prediction_expected = expected;
                    expected
                } else {
                    Some(raw.clamp(0.0, 1.0))
                };
                if expected_alpha.is_some_and(|expected| (alpha - expected).abs() > 1e-4) {
                    sample
                        .errors
                        .push(format!("{}: source alpha mismatch", blend.alpha_key));
                }
                let expected = if alpha <= 1e-5 {
                    input(node.id, "A")
                } else if alpha >= 1.0 - 1e-5 {
                    input(node.id, "B")
                } else {
                    input(node.id, "A").zip(input(node.id, "B")).map(|(a, b)| {
                        a.scalar_mult(1.0 - alpha)
                            .linear_add(&b.scalar_mult(alpha))
                            .normalize_quat()
                    })
                };
                if let Some(expected) = expected {
                    check(
                        &mut sample.errors,
                        &node.id.uuid().to_string(),
                        &actual,
                        &expected,
                    );
                } else {
                    sample
                        .errors
                        .push(format!("{}: missing blend input pose", node.id.uuid()));
                }
            }
            if let Some(blend) =
                node.try_inner_downcast_ref::<bevy_als::extensions::SourceMultiWayBlendNode>()
            {
                let weights: Vec<f32> = blend
                    .desired_alpha_keys
                    .iter()
                    .filter_map(|key| scalar(key))
                    .collect();
                let total = weights.iter().sum::<f32>();
                if weights.len() != 2 || total <= 0.0 {
                    continue;
                }
                let expected = default_overlay_stance_pose(
                    [input(node.id, "Poses_0"), input(node.id, "Poses_1")],
                    [weights[0] / total, weights[1] / total],
                );
                if let Some(expected) = expected {
                    check(&mut sample.errors, "stance blend", &actual, &expected);
                } else {
                    sample
                        .errors
                        .push("stance blend: missing input pose".into());
                }
            }
            if let Some(sequence) =
                node.try_inner_downcast_ref::<bevy_als::extensions::SourceSequencePlayerNode>()
            {
                let tick = ticks.and_then(|ticks| {
                    ticks
                        .iter()
                        .find(|tick| tick.identity == (context_id, node.id))
                });
                sample.idle_time = Some(actual.timestamp);
                sample.idle_duration = tick.map(|tick| tick.duration);
                sample.idle_phase = tick.map(|tick| tick.phase);
                sample.idle_weight = tick.map(|tick| tick.weight);
                sample.idle_rate = tick.map(|tick| tick.play_rate);
                if tick
                    .is_none_or(|tick| (actual.timestamp - tick.phase * tick.duration).abs() > 1e-4)
                {
                    sample.errors.push("idle: sync time mismatch".into());
                }
                let expected = probe.clips.get(&sequence.clip).and_then(|target| {
                    let base = probe.clips.get(sequence.additive_reference.as_ref()?)?;
                    let skeleton = probe.skeletons.get(&target.skeleton)?;
                    let sample = bevy_als::extensions::source_sequence_player::sample_source_clip_pose;
                    let target = sample(target, actual.timestamp, &probe.skeletons);
                    let base = sample(base, sequence.additive_reference_position, &probe.skeletons);
                    let mut expected =
                        bevy_als::extensions::make_local_difference(&base, &target, skeleton);
                    for (id, &index) in &expected.paths {
                        let bone = &mut expected.bones[index];
                        if bone.scale.is_none()
                            && actual.get_bone(*id).and_then(|bone| bone.scale) == Some(Vec3::ZERO)
                        {
                            bone.scale = Some(Vec3::ZERO);
                        }
                    }
                    Some(expected)
                });
                if let Some(expected) = expected {
                    check(
                        &mut sample.errors,
                        "idle additive sequence",
                        &actual,
                        &expected,
                    );
                } else {
                    sample
                        .errors
                        .push("idle additive sequence: reference asset missing".into());
                }
            }
            if let Some(additive) =
                node.try_inner_downcast_ref::<bevy_als::extensions::SourceApplyAdditiveNode>()
            {
                sample.additive_alpha = Some(additive.constant_alpha);
                if let Some((base, delta)) = input(node.id, "Base").zip(input(node.id, "Additive"))
                {
                    let expected =
                        default_overlay_additive_pose(&base, &delta, additive.constant_alpha);
                    check(&mut sample.errors, "apply additive", &actual, &expected);
                    sample.additive_effect = default_overlay_pose_error(&actual, &base);
                    sample.ready = !actual.bones.is_empty();
                }
            }
        }
    }
    sample.ready &= sample.inputs.iter().all(Option::is_some) && sample.idle_phase.is_some();
    sample
}

fn default_overlay_window(
    index: usize,
    sample: &DefaultOverlayGraphSample,
) -> ManualAnimationWindowEvidence {
    ManualAnimationWindowEvidence {
        label: Some(DEFAULT_OVERLAY_WINDOWS[index]),
        index: Some(index),
        settled: true,
        capture_source_window: true,
        capture_every_frame: true,
        requires_pre_rig_motion: true,
        requires_final_pose_motion: true,
        requires_non_reference_root_pose: true,
        requires_source_pose_chain: true,
        source_node: Some("AB_Als_Default/Overlay"),
        source_state: Some(DEFAULT_OVERLAY_WINDOWS[index]),
        source_pose_chain: vec![
            ManualAnimationPoseBoundary {
                label: "secondary_motion",
                graph: DEFAULT_OVERLAY_GRAPH,
                node: "A_Als_Idle\nSequence Player\nSync group Secondary Motion",
                output: "Pose",
                minimum_translation_span: 0.0,
                minimum_rotation_span_radians: 0.001,
            },
            ManualAnimationPoseBoundary {
                label: "apply_additive",
                graph: DEFAULT_OVERLAY_GRAPH,
                node: "Apply Additive",
                output: "Pose",
                minimum_translation_span: 0.0,
                minimum_rotation_span_radians: 0.001,
            },
            ManualAnimationPoseBoundary {
                label: "overlay_output",
                graph: DEFAULT_OVERLAY_GRAPH,
                node: "Output Pose\nOverlay",
                output: "Pose",
                minimum_translation_span: 0.0,
                minimum_rotation_span_radians: 0.001,
            },
        ]
        .into_iter()
        .chain(
            LAYERING_BOUNDARIES
                .iter()
                .filter(|(label, _, _)| match *label {
                    "layering_base_poses" => false,
                    "layering_locomotion" => index != 0,
                    _ => true,
                })
                .map(|(label, node, output)| ManualAnimationPoseBoundary {
                    label,
                    graph: LAYERING_GRAPH,
                    node,
                    output,
                    minimum_translation_span: 0.0,
                    minimum_rotation_span_radians: if *label == "layering_output" {
                        0.001
                    } else {
                        0.0
                    },
                }),
        )
        .collect(),
        minimum_pre_rig_rotation_span_radians: 0.001,
        minimum_final_rotation_span_radians: 0.001,
        minimum_observation_seconds: if index < 3 || index == 5 { 0.3 } else { 0.06 },
        source_sync_group: Some("Secondary Motion"),
        sync_phase_global: Some(DEFAULT_OVERLAY_SYNC),
        canonical_clip_time: sample
            .idle_duration
            .filter(|_| index < 3)
            .map(|duration| duration * 0.35),
        maximum_clip_time_distance: sample.idle_duration.unwrap_or(0.0) * 0.04,
        direction_signature_group: (index < 3).then_some("default_ground_poses"),
        required_direction_signature_members: if index < 3 { 3 } else { 0 },
        minimum_direction_signature_rotation_radians: if index < 3 { 0.01 } else { 0.0 },
        expected_direction: (index < 3).then_some(DEFAULT_OVERLAY_WINDOWS[index]),
        observed_direction: (index < 3).then_some(DEFAULT_OVERLAY_WINDOWS[index]),
        expected_animation_variant: Some("A_Als_Idle"),
        observed_animation_variant: Some("A_Als_Idle"),
        observed_animation_clip_time: sample.idle_time,
        observed_animation_weight: sample.additive_alpha,
        observed_animation_play_rate: sample.idle_rate,
        ..default()
    }
}

fn default_overlay_should_timeout(covered: bool, elapsed: f32) -> bool {
    !covered && elapsed >= 12.0
}

const LOCOMOTION_LAND: &str = "ALS/Character/AnimationInstances/AB_Als_Locomotion/AnimGraph/SM9_Locomotion_States/S5_Land.animgraph.ron";
const LOCOMOTION_LAND_MOVEMENT: &str = "ALS/Character/AnimationInstances/AB_Als_Locomotion/AnimGraph/SM9_Locomotion_States/S4_Land_Movement.animgraph.ron";
const LOCOMOTION_WINDOW_LABELS: [&str; 14] = [
    "jump_walk_left",
    "jump_walk_right",
    "jump_run_left",
    "jump_run_right",
    "jump_loop",
    "jump_flail",
    "fall_fast",
    "fall",
    "fall_flail",
    "ground_prediction",
    "land_light",
    "land_mixed",
    "land_movement",
    "land_complete",
];
const LOCOMOTION_CLIPS: [(&str, &str); 13] = [
    (
        "A_Als_Jump_Walk_Left",
        "A_Als_Jump_Walk_Left\nSequence Player\nSync group Jump",
    ),
    (
        "A_Als_Jump_Walk_Right",
        "A_Als_Jump_Walk_Right\nSequence Player\nSync group Jump",
    ),
    (
        "A_Als_Jump_Run_Left",
        "A_Als_Jump_Run_Left\nSequence Player\nSync group Jump",
    ),
    (
        "A_Als_Jump_Run_Right",
        "A_Als_Jump_Run_Right\nSequence Player\nSync group Jump",
    ),
    (
        "A_Als_Jump_Loop",
        "A_Als_Jump_Loop\nSequence Player\nSync group Flail",
    ),
    (
        "A_Als_Flail",
        "A_Als_Flail\nSequence Player\nSync group Flail",
    ),
    (
        "A_Als_Fall_Fast",
        "A_Als_Fall_Fast\nSequence Player\nSync group Fall",
    ),
    ("A_Als_Fall", "A_Als_Fall\nSequence Player\nSync group Fall"),
    ("A_Als_Flail", "A_Als_Flail\nSequence Player"),
    (
        "A_Als_Land_Light",
        "A_Als_Land_Light\nSequence Player\nSync group Land",
    ),
    (
        "A_Als_Land_Heavy",
        "A_Als_Land_Heavy\nSequence Player\nSync group Land",
    ),
    (
        "A_Als_Land_Light_Additive",
        "A_Als_Land_Light_Additive\nSequence Player\nSync group Land",
    ),
    (
        "A_Als_Land_Heavy_Additive",
        "A_Als_Land_Heavy_Additive\nSequence Player\nSync group Land",
    ),
];

#[derive(Clone, Copy, Debug, Serialize)]
struct LocomotionClipSample {
    index: usize,
    time: f32,
    duration: f32,
    weight: f32,
    phase: Option<f32>,
    play_rate: f32,
    looping: bool,
}

#[derive(Default, Debug, Serialize)]
struct LocomotionGraphSample {
    outer: Option<u8>,
    jump: Option<u8>,
    outer_transition: Option<bool>,
    outer_weights: [f32; 5],
    jump_weights: [f32; 4],
    jump_run_alpha: [Option<f32>; 2],
    clips: Vec<LocomotionClipSample>,
    blends: Vec<(String, f32)>,
    vertical: Option<f32>,
    foot_planted: Option<f32>,
    prediction: Option<f32>,
    prediction_pose_ready: bool,
}

#[derive(Default)]
struct LocomotionActionProbe {
    case_index: usize,
    sample: LocomotionGraphSample,
    triggered: bool,
    completed: bool,
    valid: bool,
    airborne_seen: bool,
    ready_since: Option<f64>,
    landed_since: Option<(f64, u8)>,
    land_notify_start: u32,
    covered: [bool; 14],
    canonical_seen: [bool; 14],
    window_seconds: [f32; 14],
}

impl LocomotionActionProbe {
    fn record_airborne(&mut self, actual: &LocomotionGraphSample, locomotion: LocomotionState) {
        self.airborne_seen |= self.triggered
            && locomotion == LocomotionState::Air
            && (actual.outer_weights[1] > 0.05 || actual.outer_weights[2] > 0.05);
    }

    fn landing_completed(&self, actual: &LocomotionGraphSample, time: f64, notify: u32) -> bool {
        actual.outer == Some(0)
            && actual.outer_transition == Some(false)
            && self.landed_since.is_some_and(|(since, state)| {
                time - since >= 0.35 && (state == 4 || notify > self.land_notify_start)
            })
    }

    fn record_window(
        &mut self,
        index: usize,
        window: &ManualAnimationWindowEvidence,
        sample: LocomotionClipSample,
        delta: f32,
    ) {
        self.window_seconds[index] += delta;
        self.canonical_seen[index] |= locomotion_window_canonical(window, sample);
        self.covered[index] |= self.window_seconds[index] >= 0.18 && self.canonical_seen[index];
    }
}

fn locomotion_state_index(title: &str, label: &str) -> Option<u8> {
    let labels: &[&str] = match title {
        "Locomotion States\nState Machine" => {
            &["Grounded", "Fall", "Jump", "Land", "Land Movement"]
        }
        "Jump States\nState Machine" => &["Left Foot", "Right Foot", "Loop", "Flail"],
        _ => return None,
    };
    labels
        .iter()
        .position(|candidate| *candidate == label)
        .map(|index| index as u8)
}

fn locomotion_source_scalar(player: &AnimationGraphPlayer, key: &str) -> Option<f32> {
    bevy_als::extensions::source_instance::source_player_property(player, key)
        .and_then(|value| value.as_f32().ok())
        .or_else(|| graph_global_f32(player, key))
}

fn layering_crouching_input_ready(player: &AnimationGraphPlayer) -> bool {
    locomotion_source_scalar(player, "pose_state.crouching_amount")
        .is_some_and(|value| value > 1e-4)
}

fn locomotion_actions_should_timeout(probe: &LocomotionActionProbe, elapsed: f32) -> bool {
    !probe.completed && elapsed >= 12.0
}

fn locomotion_graph_sample(
    player: &AnimationGraphPlayer,
    probe: &MotionProbe,
) -> LocomotionGraphSample {
    let scalar = |key| locomotion_source_scalar(player, key);
    let mut output = LocomotionGraphSample {
        vertical: scalar("in_air_state.vertical_velocity_world_space")
            .map(|value| (f64::from(value) * 0.01) as f32),
        foot_planted: scalar("feet_state.foot_planted_amount"),
        prediction: scalar("in_air_state.ground_prediction_amount"),
        ..default()
    };
    let Some(arena) = player.get_context_arena() else {
        return output;
    };
    let ticks = ["jump", "fall", "flail", "land"].iter().find_map(|group| {
        bevy_als::extensions::sync_group_phase::source_sync_players(
            arena,
            &probe.graphs,
            &format!("sync_groups.{group}.phase"),
        )
    });
    let mut flail_alpha = None;
    let mut prediction_alpha = None;
    for id in arena.iter_context_ids() {
        let Some(child) = arena.get_context(id) else {
            continue;
        };
        let Some(path) = probe.asset_server.get_path(child.get_graph_id()) else {
            continue;
        };
        if !path
            .path()
            .starts_with("ALS/Character/AnimationInstances/AB_Als_Locomotion")
        {
            continue;
        }
        let Some(graph) = probe.graphs.get(child.get_graph_id()) else {
            continue;
        };
        for node in graph.nodes.values() {
            if !child.node_caches.is_updated(node.id, StateKey::Default) {
                continue;
            }
            if let Some(fsm) =
                node.try_inner_downcast_ref::<bevy_als::extensions::InterruptibleFsmNode>()
            {
                let Some(machine) = probe.machines.get(&fsm.fsm) else {
                    continue;
                };
                let Ok(mut states) = child
                    .node_states
                    .get_all_upcoming_states::<bevy_als::extensions::InterruptibleFsmState>(node.id)
                else {
                    continue;
                };
                let Some(state) = states.find(|state| state.last_output_pose().is_some()) else {
                    continue;
                };
                let actual = match &state.state {
                    bevy_animation_graph::core::state_machine::low_level::LowLevelStateId::HlState(id) => machine.states.get(id).and_then(|value| locomotion_state_index(&node.name, &value.label)),
                    _ => None,
                };
                let weights = if node.name == "Locomotion States\nState Machine" {
                    output.outer = actual;
                    output.outer_transition = Some(matches!(state.state, bevy_animation_graph::core::state_machine::low_level::LowLevelStateId::HlTransition(_)));
                    &mut output.outer_weights[..]
                } else if node.name == "Jump States\nState Machine" {
                    output.jump = actual;
                    &mut output.jump_weights[..]
                } else {
                    continue;
                };
                for entry in machine.states.values() {
                    if let Some(index) = locomotion_state_index(&node.name, &entry.label) {
                        weights[index as usize] =
                            state.recorded_state_weight(&entry.id.uuid().to_string());
                    }
                }
            }
            if let Some(blend) =
                node.try_inner_downcast_ref::<bevy_als::extensions::SourceTwoWayBlendNode>()
                && let Ok(state) = child
                    .node_states
                    .get::<bevy_als::extensions::source_pose_nodes::SourceTwoWayBlendState>(
                    node.id,
                    StateKey::Default,
                )
            {
                output
                    .blends
                    .push((node.id.uuid().to_string(), state.blend_alpha));
                if blend.alpha_key == "locomotion_state.speed"
                    && let Some(side) = (0..2).find(|side| {
                        path.path() == std::path::Path::new(locomotion_clip_graph(*side))
                    })
                {
                    output.jump_run_alpha[side] = Some(state.blend_alpha);
                }
                if path.path() == std::path::Path::new(LOCOMOTION_FALL) {
                    if blend.alpha_key == "in_air_state.ground_prediction_amount" {
                        prediction_alpha = Some(state.blend_alpha);
                    }
                    if blend.alpha_key == "in_air_state.vertical_velocity_world_space"
                        && blend.mapping.input_min == -500.0
                        && blend.mapping.input_max == -3000.0
                    {
                        flail_alpha = Some(state.blend_alpha);
                    }
                }
            }
            if node.name.ends_with("Sequence Evaluator (by frame)")
                && node.name.starts_with("A_Als_Land_")
            {
                output.prediction_pose_ready |= child
                    .node_caches
                    .get_output_data(node.id, StateKey::Default, "Pose".into())
                    .is_some_and(|value| value.as_pose().is_ok_and(|pose| !pose.bones.is_empty()));
            }
            let Some(sequence) =
                node.try_inner_downcast_ref::<bevy_als::extensions::SourceSequencePlayerNode>()
            else {
                continue;
            };
            let Some(index) = LOCOMOTION_CLIPS
                .iter()
                .position(|(_, title)| *title == node.name)
            else {
                continue;
            };
            let Some(clip) = probe.clips.get(&sequence.clip) else {
                continue;
            };
            let tick =
                ticks.and_then(|ticks| ticks.iter().find(|tick| tick.identity == (id, node.id)));
            output.clips.push(LocomotionClipSample {
                index,
                time: child.node_states.get_time(node.id, StateKey::Default),
                duration: clip.duration,
                weight: tick.map_or(0.0, |tick| tick.weight),
                phase: tick.map(|tick| tick.phase),
                play_rate: tick.map_or(sequence.play_rate, |tick| tick.play_rate),
                looping: sequence.loop_animation,
            });
        }
    }
    for sample in &mut output.clips {
        if sample.index == 8 {
            sample.weight = flail_alpha.unwrap_or(0.0)
                * (1.0 - prediction_alpha.unwrap_or(0.0))
                * output.outer_weights[1];
        }
    }
    output
}

fn locomotion_jump_ready(index: usize, outer: Option<u8>, speed: f32, foot: Option<f32>) -> bool {
    outer == Some(0)
        && if index < 2 {
            (1.0..=2.0).contains(&speed)
        } else {
            speed >= 3.6
        }
        && foot.is_some_and(|value| {
            if index % 2 == 0 {
                value < -0.5
            } else {
                value > 0.5
            }
        })
}

fn locomotion_jump_weights_match(
    clips: &[LocomotionClipSample],
    run_alpha: [Option<f32>; 2],
) -> bool {
    for (walk, run) in [(0, 2), (1, 3)] {
        let weight = |index| {
            clips
                .iter()
                .filter(|sample| sample.index == index)
                .map(|sample| sample.weight)
                .sum::<f32>()
        };
        let total = weight(walk) + weight(run);
        if total > 0.05
            && run_alpha[walk].is_none_or(|alpha| {
                !alpha.is_finite() || (weight(run) / total - alpha).abs() > 0.015
            })
        {
            return false;
        }
    }
    true
}

fn locomotion_clip_graph(index: usize) -> &'static str {
    match index {
        0 | 2 => {
            "ALS/Character/AnimationInstances/AB_Als_Locomotion/AnimGraph/SM9_Locomotion_States/S2_Jump/SM0_Jump_States/S1_Left_Foot.animgraph.ron"
        }
        1 | 3 => {
            "ALS/Character/AnimationInstances/AB_Als_Locomotion/AnimGraph/SM9_Locomotion_States/S2_Jump/SM0_Jump_States/S2_Right_Foot.animgraph.ron"
        }
        4 => {
            "ALS/Character/AnimationInstances/AB_Als_Locomotion/AnimGraph/SM9_Locomotion_States/S2_Jump/SM0_Jump_States/S3_Loop.animgraph.ron"
        }
        5 => {
            "ALS/Character/AnimationInstances/AB_Als_Locomotion/AnimGraph/SM9_Locomotion_States/S2_Jump/SM0_Jump_States/S4_Flail.animgraph.ron"
        }
        6..=8 => LOCOMOTION_FALL,
        9 | 10 => LOCOMOTION_LAND,
        _ => LOCOMOTION_LAND_MOVEMENT,
    }
}

fn locomotion_window_canonical(
    window: &ManualAnimationWindowEvidence,
    sample: LocomotionClipSample,
) -> bool {
    if let Some(target) = window.canonical_sync_phase {
        sample.phase.is_some_and(|phase| {
            let distance = (phase - target).abs();
            distance.min(1.0 - distance) <= window.maximum_sync_phase_distance
        })
    } else {
        window
            .canonical_clip_time
            .is_none_or(|target| (sample.time - target).abs() <= window.maximum_clip_time_distance)
    }
}

fn locomotion_window(
    index: usize,
    sample: LocomotionClipSample,
    actual: &LocomotionGraphSample,
) -> ManualAnimationWindowEvidence {
    let graph = locomotion_clip_graph(sample.index);
    let group = match sample.index {
        0..=3 => Some(("Jump", "sync_groups.jump.phase")),
        4..=5 => Some(("Flail", "sync_groups.flail.phase")),
        6..=7 => Some(("Fall", "sync_groups.fall.phase")),
        9..=12 => Some(("Land", "sync_groups.land.phase")),
        _ => None,
    };
    let mut chain = vec![ManualAnimationPoseBoundary {
        label: "sequence_player",
        graph,
        node: LOCOMOTION_CLIPS[sample.index].1,
        output: "Pose",
        minimum_translation_span: 0.0,
        minimum_rotation_span_radians: 0.05,
    }];
    if sample.index <= 5 {
        chain.push(ManualAnimationPoseBoundary {
            label: "jump_fsm",
            graph: LOCOMOTION_JUMP,
            node: "Jump States\nState Machine",
            output: "Pose",
            minimum_translation_span: 0.0,
            minimum_rotation_span_radians: 0.05,
        });
    }
    chain.push(ManualAnimationPoseBoundary {
        label: "locomotion_fsm",
        graph: LOCOMOTION_ROOT,
        node: "Locomotion States\nState Machine",
        output: "Pose",
        minimum_translation_span: 0.0,
        minimum_rotation_span_radians: 0.05,
    });
    ManualAnimationWindowEvidence {
        label: Some(LOCOMOTION_WINDOW_LABELS[index]),
        index: Some(index),
        settled: true,
        capture_source_window: true,
        capture_every_frame: true,
        requires_pre_rig_motion: true,
        requires_final_pose_motion: true,
        requires_non_reference_root_pose: true,
        requires_source_pose_chain: true,
        source_node: Some("AB_Als_Locomotion/Locomotion States"),
        source_state: Some(if sample.index <= 5 {
            "Jump"
        } else if sample.index <= 8 {
            "Fall"
        } else if sample.index <= 10 {
            "Land"
        } else {
            "Land Movement"
        }),
        source_pose_chain: chain,
        minimum_pre_rig_rotation_span_radians: 0.05,
        minimum_final_rotation_span_radians: 0.05,
        minimum_observation_seconds: 0.12,
        source_sync_group: group.map(|value| value.0),
        sync_phase_global: group.map(|value| value.1),
        canonical_sync_phase: (sample.index <= 7).then_some(if sample.index <= 3 {
            0.35
        } else {
            0.65
        }),
        maximum_sync_phase_distance: 0.04,
        canonical_clip_time: (sample.index >= 8).then_some(0.2),
        maximum_clip_time_distance: 0.04,
        direction_signature_group: if index <= 3 {
            Some("jump_starts")
        } else if matches!(index, 6 | 7) {
            Some("fall_sources")
        } else {
            None
        },
        required_direction_signature_members: if index <= 3 {
            4
        } else if matches!(index, 6 | 7) {
            2
        } else {
            0
        },
        minimum_direction_signature_rotation_radians: if index <= 3 || matches!(index, 6 | 7) {
            0.05
        } else {
            0.0
        },
        expected_direction: (index <= 3 || matches!(index, 6 | 7))
            .then_some(LOCOMOTION_WINDOW_LABELS[index]),
        observed_direction: (index <= 3 || matches!(index, 6 | 7))
            .then_some(LOCOMOTION_WINDOW_LABELS[sample.index]),
        expected_runtime_state: if index <= 3 {
            Some((index % 2) as u8)
        } else if matches!(index, 6 | 7) {
            Some(1)
        } else {
            None
        },
        observed_runtime_state: if index <= 3 {
            actual.jump
        } else if matches!(index, 6 | 7) {
            actual.outer
        } else {
            None
        },
        expected_parent_runtime_state: (!(6..=8).contains(&sample.index)).then_some(
            if sample.index <= 5 {
                2
            } else if sample.index <= 10 {
                3
            } else {
                4
            },
        ),
        observed_parent_runtime_state: actual.outer,
        observed_parent_transition_active: actual.outer_transition,
        expected_animation_variant: Some(LOCOMOTION_CLIPS[sample.index].0),
        observed_animation_variant: Some(LOCOMOTION_CLIPS[sample.index].0),
        observed_animation_clip_time: Some(sample.time),
        observed_animation_weight: Some(sample.weight),
        observed_animation_play_rate: Some(sample.play_rate),
        ..default()
    }
}

fn observe_locomotion_actions(
    state: &mut ManualIntegrationState,
    window: &mut ManualAnimationWindowEvidence,
    report: &mut ManualIntegrationGateReport,
    player: &AnimationGraphPlayer,
    context: Option<&AlsRootAnimationContext>,
    motion: &MotionProbe,
    character: Entity,
    transform: &Transform,
    locomotion: LocomotionState,
    stance: Stance,
) {
    let actual = locomotion_graph_sample(player, motion);
    let speed = graph_global_f32(player, "locomotion_state.speed").unwrap_or(0.0);
    let delta = graph_global_f32(player, "animation.delta_seconds").unwrap_or(0.0);
    let notify = motion
        .transition_notify
        .get(character)
        .ok()
        .flatten()
        .map_or(0, |value| value.land_to_grounded_count);
    let probe = &mut state.locomotion_probe;
    let index = probe.case_index;
    if actual.outer == Some(0)
        && actual.outer_transition == Some(false)
        && locomotion == LocomotionState::Grounded
        && stance == Stance::Standing
    {
        probe.ready_since.get_or_insert(state.game_time_seconds);
    } else {
        probe.ready_since = None;
    }
    if !probe.airborne_seen {
        probe.land_notify_start = notify;
    }
    probe.record_airborne(&actual, locomotion);
    probe.valid &= locomotion_jump_weights_match(&actual.clips, actual.jump_run_alpha);
    let clip = |index| {
        actual
            .clips
            .iter()
            .copied()
            .filter(|sample| sample.index == index && sample.weight > 0.05)
            .max_by(|a, b| a.weight.total_cmp(&b.weight))
    };
    let mut selection = None;
    if index < 4 && actual.outer == Some(2) && actual.jump == Some((index % 2) as u8) {
        selection = clip(index).map(|sample| (index, sample));
    } else if index == 4 && actual.outer == Some(2) {
        let target = if !probe.covered[4] { 4 } else { 5 };
        selection = clip(target).map(|sample| (target, sample));
        probe.completed = probe.covered[4] && probe.covered[5] && actual.jump == Some(3);
    } else if index == 5 {
        let target = (6..=8).find(|target| !probe.covered[*target]);
        if let Some(target) = target {
            selection = clip(target)
                .filter(|_| target != 7 || actual.vertical.is_some_and(|value| value <= -10.0))
                .map(|sample| (target, sample));
        }
        if target.is_none()
            && actual.prediction.is_some_and(|amount| amount > 0.05)
            && actual.prediction_pose_ready
            && actual.outer == Some(1)
        {
            *window = ManualAnimationWindowEvidence {
                label: Some(LOCOMOTION_WINDOW_LABELS[9]),
                settled: true,
                capture_source_window: true,
                capture_every_frame: true,
                requires_non_reference_root_pose: true,
                requires_source_pose_chain: true,
                source_node: Some("AB_Als_Locomotion/Fall/GroundPrediction"),
                source_state: Some("Fall"),
                source_pose_chain: vec![ManualAnimationPoseBoundary {
                    label: "predicted_land_pose",
                    graph: LOCOMOTION_FALL,
                    node: "A_Als_Land_Heavy\nSequence Evaluator (by frame)",
                    output: "Pose",
                    minimum_translation_span: 0.0,
                    minimum_rotation_span_radians: 0.0,
                }],
                expected_runtime_state: Some(1),
                observed_runtime_state: actual.outer,
                minimum_observation_seconds: 0.06,
                ..default()
            };
            probe.window_seconds[9] += delta;
            probe.covered[9] |= probe.window_seconds[9] >= 0.08;
            probe.completed = probe.covered[9];
        }
    }
    if locomotion == LocomotionState::Grounded && probe.airborne_seen {
        if let Some(landed_state @ (3 | 4)) = actual.outer {
            probe
                .landed_since
                .get_or_insert((state.game_time_seconds, landed_state))
                .1 = landed_state;
            if index == 0 && actual.outer == Some(3) {
                selection = clip(9).map(|sample| (10, sample));
            }
            if index == 6 && actual.outer == Some(3) {
                selection = clip(10).map(|sample| (11, sample));
            }
            if index == 2 && actual.outer == Some(4) {
                selection = clip(11).map(|sample| (12, sample));
            }
        }
        if probe.landing_completed(&actual, state.game_time_seconds, notify) {
            probe.completed = probe
                .ready_since
                .is_some_and(|since| state.game_time_seconds - since >= 0.12);
            if index == 0 {
                probe.covered[13] |= probe.completed;
                *window = ManualAnimationWindowEvidence {
                    label: Some(LOCOMOTION_WINDOW_LABELS[13]),
                    settled: true,
                    capture_source_window: true,
                    capture_every_frame: true,
                    requires_non_reference_root_pose: true,
                    source_node: Some("AB_Als_Locomotion/AnimNotify_LandToGrounded"),
                    source_state: Some("Grounded"),
                    expected_runtime_state: Some(0),
                    observed_runtime_state: actual.outer,
                    minimum_observation_seconds: 0.06,
                    expected_lifecycle_stages: &["complete"],
                    observed_lifecycle_stages: &["complete"],
                    ..default()
                };
            }
        }
    }
    if let Some((target, selected)) = selection {
        *window = locomotion_window(target, selected, &actual);
        probe.record_window(target, window, selected, delta);
    }
    let curves = [
        "PoseStanding",
        "PoseGrounded",
        "PoseInAir",
        "GroundPredictionBlock",
        "FootLeftIk",
        "FootRightIk",
        "FootLeftLock",
        "FootRightLock",
    ]
    .map(|key| (key, context.and_then(|value| value.f32(key))));
    report.direction_trace.push(serde_json::json!({
        "case": index, "elapsed": state.game_time_seconds - state.phase_started_seconds,
        "position": transform.translation.to_array(), "speed": speed, "locomotion": format!("{locomotion:?}"),
        "triggered": probe.triggered, "completed": probe.completed, "valid": probe.valid, "covered": probe.covered,
        "land_to_grounded_count": notify, "source": actual,
        "curves": curves,
    }));
    probe.sample = actual;
}

const GROUNDED_JOINT_CASES: [(&str, Stance, bool, bool); 8] = [
    ("grounded_stand_to_crouch", Stance::Crouching, false, false),
    ("grounded_crouch_to_stand", Stance::Standing, false, false),
    ("grounded_moving_to_crouch", Stance::Crouching, true, false),
    ("grounded_moving_to_stand", Stance::Standing, true, false),
    (
        "grounded_standing_moving_aim",
        Stance::Standing,
        false,
        true,
    ),
    (
        "grounded_crouching_moving_aim",
        Stance::Crouching,
        false,
        true,
    ),
    (
        "grounded_standing_rapid_aim_stop",
        Stance::Standing,
        false,
        true,
    ),
    (
        "grounded_crouching_rapid_aim_stop",
        Stance::Crouching,
        false,
        true,
    ),
];

#[derive(Default)]
struct GroundedJointProbe {
    case_index: usize,
    ready_since: Option<f64>,
    triggered_at: Option<f64>,
    base_yaw: f32,
    last_sequence_time: Option<f32>,
    sequence_completed: bool,
    valid: bool,
    state_mask: u8,
    reset_start: u32,
}

impl GroundedJointProbe {
    fn record(
        &mut self,
        mesh_base_offset: Option<f32>,
        sequence: Option<(f32, f32, bool, f32)>,
        delta: f32,
    ) {
        self.valid &= mesh_base_offset.is_some_and(|value| {
            (value - bevy_als::character_assembly::AVATAR_MESH_ROOT_WORLD_Y).abs() < 1e-5
        });
        if let Some((position, duration, looping, rate)) = sequence {
            self.valid &= !looping
                && position.is_finite()
                && self.last_sequence_time.is_none_or(|last| position >= last);
            self.last_sequence_time = Some(position);
            self.sequence_completed |= position + delta * rate + 1e-5 >= duration;
        }
    }
}

fn grounded_joint_command(
    index: usize,
    elapsed: Option<f32>,
    base_yaw: f32,
    view_yaw: f32,
) -> crate::input::AlsInputCommand {
    if index >= 6 {
        return standing_turn_mouse_command(
            7,
            elapsed.map(|time| time + 8.0),
            base_yaw,
            view_yaw,
            RotationMode::Aiming,
        );
    }
    let (_, _, moving, aiming) = GROUNDED_JOINT_CASES[index];
    let elapsed = elapsed.unwrap_or(-1.0);
    let target = base_yaw
        + if aiming {
            ((elapsed - 1.2).clamp(0.0, 1.0) * if index == 4 { -45.0_f32 } else { 45.0 })
                .to_radians()
        } else {
            0.0
        };
    crate::input::AlsInputCommand {
        aim_held: aiming,
        jump_held: index == 1 && (0.0..0.5).contains(&elapsed),
        move_dir: if moving || (aiming && elapsed >= 0.5) {
            Vec2::Y
        } else {
            Vec2::ZERO
        },
        look_delta: if aiming {
            Vec2::new(
                -bevy_als::als::motion::wrap_yaw_radians(target - view_yaw)
                    / bevy_als::als::view::LOOK_MOUSE_RADIANS_PER_COUNT,
                0.0,
            )
        } else {
            Vec2::ZERO
        },
        ..default()
    }
}

fn grounded_joint_source_chain() -> Vec<ManualAnimationPoseBoundary> {
    generated_standing_source_pose_chain()
        .into_iter()
        .filter(|boundary| boundary.label == "grounded_fsm")
        .collect()
}

#[derive(Clone, Copy)]
struct DirectionRegressionCase {
    name: &'static str,
    move_dir: Vec2,
    desired_gait: Gait,
    expected_allowed_gait: Gait,
    expected_actual_gait: Gait,
    sprint_held: bool,
    first_person: bool,
    rotation_mode: RotationMode,
    yaw_offset: f32,
    branch_mask: u8,
    branch_from_actual_local: bool,
    expected_movement_state: Option<AlsMovementDirection>,
    source_state: Option<&'static str>,
    direction_signature_group: Option<&'static str>,
}

impl DirectionRegressionCase {
    fn command(&self) -> crate::input::AlsInputCommand {
        crate::input::AlsInputCommand {
            move_dir: self.move_dir,
            sprint_held: self.sprint_held,
            aim_held: self.rotation_mode == RotationMode::Aiming,
            ..default()
        }
    }
}

#[derive(Clone, Copy)]
struct CrouchDirectionRegressionCase {
    name: &'static str,
    move_dir: Vec2,
    expected_direction: AlsMovementDirection,
}

fn direction_regression_case(index: usize) -> Option<DirectionRegressionCase> {
    let running = |name, move_dir, branch_mask| DirectionRegressionCase {
        name,
        move_dir,
        desired_gait: Gait::Running,
        expected_allowed_gait: Gait::Running,
        expected_actual_gait: Gait::Running,
        sprint_held: false,
        first_person: false,
        rotation_mode: RotationMode::Aiming,
        yaw_offset: 0.0,
        branch_mask,
        branch_from_actual_local: false,
        expected_movement_state: None,
        source_state: None,
        direction_signature_group: None,
    };
    match index {
        0 => Some(DirectionRegressionCase {
            name: "walk_forward",
            move_dir: Vec2::Y,
            desired_gait: Gait::Walking,
            expected_allowed_gait: Gait::Walking,
            expected_actual_gait: Gait::Walking,
            sprint_held: false,
            first_person: false,
            rotation_mode: RotationMode::Aiming,
            yaw_offset: 0.0,
            branch_mask: DIRECTION_BRANCH_FORWARD,
            branch_from_actual_local: false,
            expected_movement_state: Some(AlsMovementDirection::Forward),
            source_state: Some("Move Forward"),
            direction_signature_group: Some("AB_Als_Standing/Movement States"),
        }),
        1 => Some(DirectionRegressionCase {
            expected_movement_state: Some(AlsMovementDirection::Backward),
            source_state: Some("Move Backward"),
            direction_signature_group: Some("AB_Als_Standing/Movement States"),
            ..running("run_backward", Vec2::NEG_Y, DIRECTION_BRANCH_BACKWARD)
        }),
        2 => Some(DirectionRegressionCase {
            expected_movement_state: Some(AlsMovementDirection::RightForward),
            source_state: Some("Move Right Backward to Move Right Forward"),
            direction_signature_group: None,
            ..running("run_right_from_backward", Vec2::X, DIRECTION_BRANCH_RIGHT)
        }),
        3 => Some(DirectionRegressionCase {
            expected_movement_state: Some(AlsMovementDirection::LeftForward),
            source_state: Some("Move Left Forward"),
            direction_signature_group: Some("AB_Als_Standing/Movement States"),
            ..running(
                "run_left_from_right_backward",
                Vec2::NEG_X,
                DIRECTION_BRANCH_LEFT,
            )
        }),
        4 => Some(DirectionRegressionCase {
            expected_movement_state: Some(AlsMovementDirection::Forward),
            source_state: Some("Move Forward"),
            direction_signature_group: None,
            ..running(
                "run_forward_from_left_forward",
                Vec2::Y,
                DIRECTION_BRANCH_FORWARD,
            )
        }),
        5 => Some(DirectionRegressionCase {
            expected_movement_state: Some(AlsMovementDirection::Backward),
            source_state: Some("Move Backward"),
            direction_signature_group: None,
            ..running(
                "run_backward_from_forward",
                Vec2::NEG_Y,
                DIRECTION_BRANCH_BACKWARD,
            )
        }),
        6 => Some(DirectionRegressionCase {
            expected_movement_state: Some(AlsMovementDirection::LeftForward),
            source_state: Some("Move Left Backward to Move Left Forward"),
            direction_signature_group: None,
            ..running(
                "run_left_forward_from_backward",
                Vec2::NEG_X,
                DIRECTION_BRANCH_LEFT,
            )
        }),
        7 => Some(DirectionRegressionCase {
            expected_movement_state: Some(AlsMovementDirection::RightForward),
            source_state: Some("Move Right Forward"),
            direction_signature_group: Some("AB_Als_Standing/Movement States"),
            ..running(
                "run_right_from_left_backward",
                Vec2::X,
                DIRECTION_BRANCH_RIGHT,
            )
        }),
        8 => Some(DirectionRegressionCase {
            name: "sprint_allowed_forward",
            move_dir: Vec2::Y,
            desired_gait: Gait::Sprinting,
            expected_allowed_gait: Gait::Sprinting,
            expected_actual_gait: Gait::Sprinting,
            sprint_held: true,
            first_person: false,
            rotation_mode: RotationMode::VelocityDirection,
            yaw_offset: 0.0,
            branch_mask: DIRECTION_BRANCH_FORWARD,
            branch_from_actual_local: false,
            expected_movement_state: Some(AlsMovementDirection::Forward),
            source_state: Some("Move Forward"),
            direction_signature_group: None,
        }),
        9 => Some(DirectionRegressionCase {
            name: "sprint_rejected_first_person_lateral",
            move_dir: Vec2::X,
            desired_gait: Gait::Sprinting,
            expected_allowed_gait: Gait::Running,
            expected_actual_gait: Gait::Running,
            sprint_held: true,
            first_person: true,
            rotation_mode: RotationMode::ViewDirection,
            yaw_offset: 0.0,
            branch_mask: DIRECTION_BRANCH_RIGHT,
            branch_from_actual_local: false,
            expected_movement_state: None,
            source_state: None,
            direction_signature_group: None,
        }),
        10 => Some(DirectionRegressionCase {
            name: "run_forward_rotated_90",
            move_dir: Vec2::Y,
            desired_gait: Gait::Running,
            expected_allowed_gait: Gait::Running,
            expected_actual_gait: Gait::Running,
            sprint_held: false,
            first_person: false,
            rotation_mode: RotationMode::Aiming,
            yaw_offset: std::f32::consts::FRAC_PI_2,
            branch_mask: 0,
            branch_from_actual_local: true,
            expected_movement_state: None,
            source_state: None,
            direction_signature_group: None,
        }),
        _ => None,
    }
}

fn standing_response_step(elapsed: f32) -> (&'static str, Vec2) {
    if elapsed < 1.5 {
        ("response_idle_before", Vec2::ZERO)
    } else if elapsed < 3.5 {
        ("response_left", Vec2::NEG_X)
    } else if elapsed < 5.5 {
        ("response_reverse_right", Vec2::X)
    } else if elapsed < 8.5 {
        [
            ("response_cycle_w", Vec2::Y),
            ("response_cycle_a", Vec2::NEG_X),
            ("response_cycle_s", Vec2::NEG_Y),
            ("response_cycle_d", Vec2::X),
        ][((elapsed - 5.5) / 0.25) as usize % 4]
    } else if elapsed < 11.5 {
        ("response_stop", Vec2::ZERO)
    } else {
        ("response_idle_after", Vec2::ZERO)
    }
}

fn crouch_direction_regression_case(index: usize) -> Option<CrouchDirectionRegressionCase> {
    let names = [
        "crouch_forward",
        "crouch_backward",
        "crouch_right_from_backward",
        "crouch_left_from_right_backward",
        "crouch_forward_repeat",
        "crouch_backward_repeat",
        "crouch_left_from_backward",
        "crouch_right_from_left_backward",
    ];
    let case = direction_regression_case(index).filter(|_| index < names.len())?;
    Some(CrouchDirectionRegressionCase {
        name: names[index],
        move_dir: case.move_dir,
        expected_direction: case.expected_movement_state?,
    })
}

fn crouch_direction_source_node(direction: AlsMovementDirection) -> &'static str {
    match direction {
        AlsMovementDirection::Forward => "AB_Als_Crouching/A_Als_Crouch_Walk_Forward",
        AlsMovementDirection::Backward => "AB_Als_Crouching/A_Als_Crouch_Walk_Backward",
        AlsMovementDirection::RightForward => "AB_Als_Crouching/A_Als_Crouch_Walk_Right_Forward",
        AlsMovementDirection::RightBackward => "AB_Als_Crouching/A_Als_Crouch_Walk_Right_Backward",
        AlsMovementDirection::LeftForward => "AB_Als_Crouching/A_Als_Crouch_Walk_Left_Forward",
        AlsMovementDirection::LeftBackward => "AB_Als_Crouching/A_Als_Crouch_Walk_Left_Backward",
    }
}

fn crouch_direction_graph_node(direction: AlsMovementDirection) -> &'static str {
    match direction {
        AlsMovementDirection::Forward => {
            "A_Als_Crouch_Walk_Forward\nSequence Player\nSync group Movement"
        }
        AlsMovementDirection::Backward => {
            "A_Als_Crouch_Walk_Backward\nSequence Player\nSync group Movement"
        }
        AlsMovementDirection::RightForward => {
            "A_Als_Crouch_Walk_Right_Forward\nSequence Player\nSync group Movement"
        }
        AlsMovementDirection::RightBackward => {
            "A_Als_Crouch_Walk_Right_Backward\nSequence Player\nSync group Movement"
        }
        AlsMovementDirection::LeftForward => {
            "A_Als_Crouch_Walk_Left_Forward\nSequence Player\nSync group Movement"
        }
        AlsMovementDirection::LeftBackward => {
            "A_Als_Crouch_Walk_Left_Backward\nSequence Player\nSync group Movement"
        }
    }
}

fn crouch_direction_cache_node(direction: AlsMovementDirection) -> &'static str {
    match direction {
        AlsMovementDirection::Forward => "Save cached pose 'Move Forward'",
        AlsMovementDirection::Backward => "Save cached pose 'Move Backward'",
        AlsMovementDirection::RightForward => "Save cached pose 'Move Right Forward'",
        AlsMovementDirection::RightBackward => "Save cached pose 'Move Right Backward'",
        AlsMovementDirection::LeftForward => "Save cached pose 'Move Left Forward'",
        AlsMovementDirection::LeftBackward => "Save cached pose 'Move Left Backward'",
    }
}

fn generated_standing_source_pose_chain() -> Vec<ManualAnimationPoseBoundary> {
    vec![
        ManualAnimationPoseBoundary {
            label: "standing_movement_cache",
            graph: "ALS/Character/AnimationInstances/Stances/AB_Als_Standing/AnimGraph.animgraph.ron",
            node: "Save cached pose 'Movement'",
            output: "__als_source_cached_pose",
            minimum_translation_span: 0.0,
            minimum_rotation_span_radians: 0.05,
        },
        ManualAnimationPoseBoundary {
            label: "standing_movement_details_fsm",
            graph: "ALS/Character/AnimationInstances/Stances/AB_Als_Standing/AnimGraph.animgraph.ron",
            node: "Movement Details States\nState Machine",
            output: "Pose",
            minimum_translation_span: 0.0,
            minimum_rotation_span_radians: 0.05,
        },
        ManualAnimationPoseBoundary {
            label: "standing_movement_states_fsm",
            graph: "ALS/Character/AnimationInstances/Stances/AB_Als_Standing/AnimGraph.animgraph.ron",
            node: "Movement States\nState Machine",
            output: "Pose",
            minimum_translation_span: 0.0,
            minimum_rotation_span_radians: 0.05,
        },
        ManualAnimationPoseBoundary {
            label: "standing_states_fsm",
            graph: "ALS/Character/AnimationInstances/Stances/AB_Als_Standing/AnimGraph.animgraph.ron",
            node: "Standing States\nState Machine",
            output: "Pose",
            minimum_translation_span: 0.0,
            minimum_rotation_span_radians: 0.05,
        },
        ManualAnimationPoseBoundary {
            label: "standing_apply_additive",
            graph: "ALS/Character/AnimationInstances/Stances/AB_Als_Standing/AnimGraph.animgraph.ron",
            node: "Apply Additive",
            output: "Pose",
            minimum_translation_span: 0.0,
            minimum_rotation_span_radians: 0.05,
        },
        ManualAnimationPoseBoundary {
            label: "standing_refresh_rotate_in_place",
            graph: "ALS/Character/AnimationInstances/Stances/AB_Als_Standing/AnimGraph.animgraph.ron",
            node: "RefreshRotateInPlace\r\nOn Update",
            output: "Pose",
            minimum_translation_span: 0.0,
            minimum_rotation_span_radians: 0.05,
        },
        ManualAnimationPoseBoundary {
            label: "standing_graph_output",
            graph: "ALS/Character/AnimationInstances/Stances/AB_Als_Standing/AnimGraph.animgraph.ron",
            node: "Output Pose\nAnimGraph",
            output: "Pose",
            minimum_translation_span: 0.0,
            minimum_rotation_span_radians: 0.05,
        },
        ManualAnimationPoseBoundary {
            label: "grounded_standing_cache",
            graph: "ALS/Character/AnimationInstances/AB_Als_Grounded/AnimGraph.animgraph.ron",
            node: "Save cached pose 'Standing'",
            output: "__als_source_cached_pose",
            minimum_translation_span: 0.0,
            minimum_rotation_span_radians: 0.05,
        },
        ManualAnimationPoseBoundary {
            label: "grounded_fsm",
            graph: "ALS/Character/AnimationInstances/AB_Als_Grounded/AnimGraph.animgraph.ron",
            node: "Grounded States\nState Machine",
            output: "Pose",
            minimum_translation_span: 0.0,
            minimum_rotation_span_radians: 0.05,
        },
    ]
}

fn generated_stop_source_pose_chain(crouching: bool) -> Vec<ManualAnimationPoseBoundary> {
    let mut chain = if crouching {
        crouch_turn_source_pose_chain()
    } else {
        generated_standing_source_pose_chain()
    }
    .into_iter()
    .filter(|boundary| {
        matches!(
            boundary.label,
            "outer_fsm"
                | "refresh_rotate"
                | "root_entry"
                | "standing_states_fsm"
                | "standing_graph_output"
                | "grounded_standing_cache"
                | "grounded_fsm"
        )
    })
    .collect::<Vec<_>>();
    for boundary in &mut chain {
        boundary.minimum_translation_span = 0.0;
        boundary.minimum_rotation_span_radians = 0.0;
    }
    chain
}

fn quick_stop_source_pose_chain() -> Vec<ManualAnimationPoseBoundary> {
    vec![ManualAnimationPoseBoundary {
        label: "transition_slot",
        graph: "ALS/Character/AB_Als/AnimGraph.animgraph.ron",
        node: "Slot 'Transition'\nGroup 'Grounded'",
        output: "Pose",
        minimum_translation_span: 0.0,
        minimum_rotation_span_radians: 0.0,
    }]
}

fn state_machine_grounded_window(elapsed: f32) -> ManualAnimationWindowEvidence {
    let moving = (1.0..2.4).contains(&elapsed) || elapsed >= 3.5;
    ManualAnimationWindowEvidence {
        label: Some("state_machine_grounded"),
        settled: moving,
        capture_source_window: moving,
        requires_pre_rig_motion: true,
        requires_final_pose_motion: true,
        requires_non_reference_root_pose: true,
        requires_source_pose_chain: true,
        source_node: Some("AB_Als_Standing/Standing States"),
        source_pose_chain: generated_standing_source_pose_chain(),
        source_sync_group: Some("Movement"),
        sync_phase_global: Some("sync_groups.movement.phase"),
        ..default()
    }
}

fn sprint_window(
    accelerating: bool,
    phase: f32,
    telemetry: &AlsStandingGraphTelemetry,
) -> ManualAnimationWindowEvidence {
    let (label, node) = if accelerating {
        (
            "sprint_acceleration",
            "A_Als_Sprint_Acceleration\nSequence Player\nSync group Movement",
        )
    } else {
        (
            "sprint_steady",
            "A_Als_Sprint\nSequence Player\nSync group Movement",
        )
    };
    let mut source_pose_chain = generated_standing_source_pose_chain();
    source_pose_chain.insert(0, ManualAnimationPoseBoundary {
        label: "sprint_sequence",
        graph: "ALS/Character/AnimationInstances/Stances/AB_Als_Standing/AnimGraph.animgraph.ron",
        node,
        output: "Pose",
        minimum_translation_span: 0.0,
        minimum_rotation_span_radians: 0.05,
    });
    ManualAnimationWindowEvidence {
        label: Some(label),
        settled: true,
        capture_source_window: true,
        capture_every_frame: accelerating,
        requires_pre_rig_motion: true,
        requires_final_pose_motion: true,
        requires_non_reference_root_pose: true,
        requires_source_pose_chain: true,
        source_node: Some("AB_Als_Standing/AnimGraph"),
        source_state: Some(node),
        source_pose_chain,
        source_sync_group: Some("Movement"),
        sync_phase_global: Some("sync_groups.movement.phase"),
        canonical_sync_phase: Some(phase),
        maximum_sync_phase_distance: 0.12,
        direction_signature_group: Some("AB_Als_Standing/Sprint"),
        required_direction_signature_members: 2,
        minimum_direction_signature_rotation_radians: 0.01,
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
    }
}

fn source_sprint_values(player: &AnimationGraphPlayer) -> Option<(f32, f32, bool)> {
    let arena = player.get_context_arena()?;
    let source = arena
        .get_toplevel()
        .node_states
        .get::<SourceInstanceProperties>(NodeId::default(), StateKey::Default)
        .ok()?;
    source_sprint_values_at_frame(source, arena.frame_index())
}

fn source_sprint_values_at_frame(
    source: &SourceInstanceProperties,
    frame: u64,
) -> Option<(f32, f32, bool)> {
    let updated = source.write_frames.get("standing_state.sprint_time") == Some(&frame);
    Some((
        source
            .values
            .get("standing_state.sprint_time")?
            .as_f32()
            .ok()?,
        source
            .values
            .get("standing_state.sprint_acceleration_amount")?
            .as_f32()
            .ok()?,
        updated,
    ))
}

fn crouch_direction_source_pose_chain(
    direction: AlsMovementDirection,
) -> Vec<ManualAnimationPoseBoundary> {
    vec![
        ManualAnimationPoseBoundary {
            label: "sequence_player",
            graph: "ALS/Character/AnimationInstances/Stances/AB_Als_Crouching/AnimGraph.animgraph.ron",
            node: crouch_direction_graph_node(direction),
            output: "Pose",
            minimum_translation_span: 0.0,
            minimum_rotation_span_radians: 0.05,
        },
        ManualAnimationPoseBoundary {
            label: "cache_producer",
            graph: "ALS/Character/AnimationInstances/Stances/AB_Als_Crouching/AnimGraph.animgraph.ron",
            node: crouch_direction_cache_node(direction),
            output: "__als_source_cached_pose",
            minimum_translation_span: 0.0,
            minimum_rotation_span_radians: 0.05,
        },
        ManualAnimationPoseBoundary {
            label: "cache_consumer",
            graph: "ALS/Character/AnimationInstances/Stances/AB_Als_Crouching/AnimGraph.animgraph.ron",
            node: "Movement States\nState Machine",
            output: "Pose",
            minimum_translation_span: 0.0,
            minimum_rotation_span_radians: 0.05,
        },
        ManualAnimationPoseBoundary {
            label: "outer_fsm",
            graph: "ALS/Character/AnimationInstances/Stances/AB_Als_Crouching/AnimGraph.animgraph.ron",
            node: "Crouching States\nState Machine",
            output: "Pose",
            minimum_translation_span: 0.0,
            minimum_rotation_span_radians: 0.05,
        },
        ManualAnimationPoseBoundary {
            label: "additive",
            graph: "ALS/Character/AnimationInstances/Stances/AB_Als_Crouching/AnimGraph.animgraph.ron",
            node: "Apply Additive",
            output: "Pose",
            minimum_translation_span: 0.0,
            minimum_rotation_span_radians: 0.05,
        },
        ManualAnimationPoseBoundary {
            label: "root_entry",
            graph: "ALS/Character/AnimationInstances/Stances/AB_Als_Crouching/AnimGraph.animgraph.ron",
            node: "Output Pose\nAnimGraph",
            output: "Pose",
            minimum_translation_span: 0.0,
            minimum_rotation_span_radians: 0.05,
        },
    ]
}

fn source_sequence_sample(
    player: &AnimationGraphPlayer,
    graphs: &Assets<AnimationGraph>,
    name: &str,
) -> Option<(f32, f32, bool)> {
    let arena = player.get_context_arena()?;
    arena
        .iter_context_ids()
        .filter_map(|id| {
            let context = arena.get_context(id)?;
            let graph = graphs.get(context.get_graph_id())?;
            let node = graph.nodes.values().find(|node| node.name == name)?;
            let sequence =
                node.try_inner_downcast_ref::<bevy_als::extensions::SourceSequencePlayerNode>()?;
            let position = context.node_states.get_time(node.id, StateKey::Default);
            Some((
                position,
                sequence
                    .curves
                    .sample_all(position)
                    .get(&bevy_als::extensions::SourceCurveName::from(
                        "RotationYawSpeed",
                    ))
                    .copied()
                    .unwrap_or(0.0),
                context.node_caches.is_updated(node.id, StateKey::Default),
            ))
        })
        .max_by_key(|sample| sample.2)
}

fn crouch_rotate_source_pose_chain(left: bool) -> Vec<ManualAnimationPoseBoundary> {
    let graph = if left {
        "ALS/Character/AnimationInstances/Stances/AB_Als_Crouching/AnimGraph/SM2_Crouching_States/S3_Rotate_Left.animgraph.ron"
    } else {
        "ALS/Character/AnimationInstances/Stances/AB_Als_Crouching/AnimGraph/SM2_Crouching_States/S7_Rotate_Right.animgraph.ron"
    };
    let sequence = if left {
        "A_Als_Crouch_Rotate_90_Left\nSequence Player"
    } else {
        "A_Als_Crouch_Rotate_90_Right\nSequence Player"
    };
    vec![
        ManualAnimationPoseBoundary {
            label: "sequence_player",
            graph,
            node: sequence,
            output: "Pose",
            minimum_translation_span: 0.0,
            minimum_rotation_span_radians: 0.05,
        },
        ManualAnimationPoseBoundary {
            label: "curve_output",
            graph,
            node: "Modify Curve",
            output: "Pose",
            minimum_translation_span: 0.0,
            minimum_rotation_span_radians: 0.05,
        },
        ManualAnimationPoseBoundary {
            label: "outer_fsm",
            graph: "ALS/Character/AnimationInstances/Stances/AB_Als_Crouching/AnimGraph.animgraph.ron",
            node: "Crouching States\nState Machine",
            output: "Pose",
            minimum_translation_span: 0.0,
            minimum_rotation_span_radians: 0.05,
        },
        ManualAnimationPoseBoundary {
            label: "refresh_rotate",
            graph: "ALS/Character/AnimationInstances/Stances/AB_Als_Crouching/AnimGraph.animgraph.ron",
            node: "RefreshRotateInPlace\r\nOn Update",
            output: "Pose",
            minimum_translation_span: 0.0,
            minimum_rotation_span_radians: 0.05,
        },
        ManualAnimationPoseBoundary {
            label: "root_entry",
            graph: "ALS/Character/AnimationInstances/Stances/AB_Als_Crouching/AnimGraph.animgraph.ron",
            node: "Output Pose\nAnimGraph",
            output: "Pose",
            minimum_translation_span: 0.0,
            minimum_rotation_span_radians: 0.05,
        },
    ]
}

fn crouch_turn_source_pose_chain() -> Vec<ManualAnimationPoseBoundary> {
    vec![
        ManualAnimationPoseBoundary {
            label: "slot",
            graph: "ALS/Character/AnimationInstances/Stances/AB_Als_Crouching/AnimGraph/SM2_Crouching_States/S8_Idle.animgraph.ron",
            node: "Slot 'TurnInPlaceCrouching'\nGroup 'Grounded'",
            output: "Pose",
            minimum_translation_span: 0.0,
            minimum_rotation_span_radians: 0.05,
        },
        ManualAnimationPoseBoundary {
            label: "refresh_turn",
            graph: "ALS/Character/AnimationInstances/Stances/AB_Als_Crouching/AnimGraph/SM2_Crouching_States/S8_Idle.animgraph.ron",
            node: "RefreshTurnInPlace\r\nOn Update",
            output: "Pose",
            minimum_translation_span: 0.0,
            minimum_rotation_span_radians: 0.05,
        },
        ManualAnimationPoseBoundary {
            label: "outer_fsm",
            graph: "ALS/Character/AnimationInstances/Stances/AB_Als_Crouching/AnimGraph.animgraph.ron",
            node: "Crouching States\nState Machine",
            output: "Pose",
            minimum_translation_span: 0.0,
            minimum_rotation_span_radians: 0.05,
        },
        ManualAnimationPoseBoundary {
            label: "refresh_rotate",
            graph: "ALS/Character/AnimationInstances/Stances/AB_Als_Crouching/AnimGraph.animgraph.ron",
            node: "RefreshRotateInPlace\r\nOn Update",
            output: "Pose",
            minimum_translation_span: 0.0,
            minimum_rotation_span_radians: 0.05,
        },
        ManualAnimationPoseBoundary {
            label: "root_entry",
            graph: "ALS/Character/AnimationInstances/Stances/AB_Als_Crouching/AnimGraph.animgraph.ron",
            node: "Output Pose\nAnimGraph",
            output: "Pose",
            minimum_translation_span: 0.0,
            minimum_rotation_span_radians: 0.05,
        },
    ]
}

fn crouch_rotate_window(
    left: bool,
    active: bool,
    observed_lifecycle_stages: &'static [&'static str],
) -> ManualAnimationWindowEvidence {
    ManualAnimationWindowEvidence {
        actor_yaw_driver: None,
        label: Some(if left {
            "crouch_rotate_left"
        } else {
            "crouch_rotate_right"
        }),
        index: None,
        settled: active || observed_lifecycle_stages.contains(&"settled"),
        capture_source_window: active || observed_lifecycle_stages.contains(&"settled"),
        capture_every_frame: true,
        requires_pre_rig_motion: active,
        requires_final_pose_motion: active,
        requires_non_reference_root_pose: active,
        requires_source_pose_chain: active,
        requires_actor_yaw_motion: active,
        requires_actor_yaw_convergence: false,
        expected_actor_yaw_delta_sign: Some(if left { 1 } else { -1 }),
        expected_actor_yaw_baseline: None,
        expected_animation_variant: Some(crouch_stationary_animation_variant(left, false)),
        source_node: Some(if left {
            "AB_Als_Crouching/A_Als_Crouch_Rotate_90_Left"
        } else {
            "AB_Als_Crouching/A_Als_Crouch_Rotate_90_Right"
        }),
        source_state: Some(if left { "Rotate Left" } else { "Rotate Right" }),
        source_pose_chain: active
            .then(|| crouch_rotate_source_pose_chain(left))
            .unwrap_or_default(),
        minimum_pre_rig_translation_span: 0.0,
        minimum_pre_rig_rotation_span_radians: if active { 0.05 } else { 0.0 },
        minimum_final_translation_span: 0.0,
        minimum_final_rotation_span_radians: if active { 0.05 } else { 0.0 },
        minimum_actor_yaw_delta_radians: if active {
            CROUCH_ROTATE_SOURCE_MIN_YAW_CONVERGENCE_RADIANS
        } else {
            0.0
        },
        minimum_actor_yaw_convergence_radians: 0.0,
        minimum_observation_seconds: 0.0,
        source_sync_group: None,
        sync_phase_global: None,
        canonical_sync_phase: None,
        maximum_sync_phase_distance: 0.0,
        canonical_clip_time: Some(0.5),
        maximum_clip_time_distance: 0.06,
        direction_signature_group: Some("crouch_rotate"),
        required_direction_signature_members: 2,
        minimum_direction_signature_rotation_radians: 1e-4,
        expected_direction: Some(if left { "Left" } else { "Right" }),
        observed_direction: None,
        expected_runtime_state: None,
        observed_runtime_state: None,
        expected_parent_runtime_state: Some(if left { 2 } else { 3 }),
        expected_terminal_parent_runtime_state: Some(0),
        expected_terminal_parent_transition_active: Some(false),
        expected_terminal_requested_parent_transition_active: None,
        observed_parent_runtime_state: None,
        observed_parent_transition_active: None,
        observed_requested_parent_runtime_state: None,
        observed_requested_parent_transition_active: None,
        observed_actor_yaw: None,
        observed_view_yaw: None,
        observed_animation_yaw_speed_degrees: None,
        observed_animation_curve_direct_yaw_speed_degrees: None,
        observed_animation_clip_time: None,
        observed_animation_weight: None,
        observed_animation_play_rate: None,
        observed_animation_curve_stack_weight: None,
        observed_animation_curve_stack_entries: Vec::new(),
        observed_animation_variant: None,
        expected_lifecycle_stages: &["sequence_completed", "settled"],
        observed_lifecycle_stages,
    }
}

fn crouch_turn_window(
    left: bool,
    active: bool,
    observed_lifecycle_stages: &'static [&'static str],
) -> ManualAnimationWindowEvidence {
    ManualAnimationWindowEvidence {
        actor_yaw_driver: None,
        label: Some(if left {
            "crouch_turn_left"
        } else {
            "crouch_turn_right"
        }),
        index: None,
        settled: active || observed_lifecycle_stages.contains(&"settled"),
        capture_source_window: active || !observed_lifecycle_stages.is_empty(),
        capture_every_frame: true,
        requires_pre_rig_motion: active,
        requires_final_pose_motion: active,
        requires_non_reference_root_pose: active,
        requires_source_pose_chain: active,
        requires_actor_yaw_motion: active,
        requires_actor_yaw_convergence: active,
        expected_actor_yaw_delta_sign: Some(if left { 1 } else { -1 }),
        expected_actor_yaw_baseline: None,
        expected_animation_variant: Some(crouch_stationary_animation_variant(left, true)),
        source_node: Some("AB_Als_Crouching/Slot 'TurnInPlaceCrouching'"),
        source_state: Some("Idle"),
        source_pose_chain: active
            .then(crouch_turn_source_pose_chain)
            .unwrap_or_default(),
        minimum_pre_rig_translation_span: 0.0,
        minimum_pre_rig_rotation_span_radians: if active { 0.05 } else { 0.0 },
        minimum_final_translation_span: 0.0,
        minimum_final_rotation_span_radians: if active { 0.05 } else { 0.0 },
        minimum_actor_yaw_delta_radians: if active {
            CROUCH_TURN_SOURCE_MIN_YAW_CONVERGENCE_RADIANS
        } else {
            0.0
        },
        minimum_actor_yaw_convergence_radians: if active {
            CROUCH_TURN_SOURCE_MIN_YAW_CONVERGENCE_RADIANS
        } else {
            0.0
        },
        minimum_observation_seconds: 0.0,
        source_sync_group: None,
        sync_phase_global: None,
        canonical_sync_phase: None,
        maximum_sync_phase_distance: 0.0,
        canonical_clip_time: Some(0.5),
        maximum_clip_time_distance: 0.06,
        direction_signature_group: Some("crouch_turn"),
        required_direction_signature_members: 2,
        minimum_direction_signature_rotation_radians: 1e-4,
        expected_direction: Some(if left { "Left" } else { "Right" }),
        observed_direction: None,
        expected_runtime_state: None,
        observed_runtime_state: None,
        expected_parent_runtime_state: Some(0),
        expected_terminal_parent_runtime_state: Some(0),
        expected_terminal_parent_transition_active: Some(false),
        expected_terminal_requested_parent_transition_active: None,
        observed_parent_runtime_state: None,
        observed_parent_transition_active: None,
        observed_requested_parent_runtime_state: None,
        observed_requested_parent_transition_active: None,
        observed_actor_yaw: None,
        observed_view_yaw: None,
        observed_animation_yaw_speed_degrees: None,
        observed_animation_curve_direct_yaw_speed_degrees: None,
        observed_animation_clip_time: None,
        observed_animation_weight: None,
        observed_animation_play_rate: None,
        observed_animation_curve_stack_weight: None,
        observed_animation_curve_stack_entries: Vec::new(),
        observed_animation_variant: None,
        expected_lifecycle_stages: &[
            "slot_active",
            "natural_completion",
            "inertialization",
            "settled",
        ],
        observed_lifecycle_stages,
    }
}

fn crouch_stationary_animation_variant(left: bool, turn: bool) -> &'static str {
    match (turn, left) {
        (false, true) => "A_Als_Crouch_Rotate_90_Left",
        (false, false) => "A_Als_Crouch_Rotate_90_Right",
        (true, true) => "A_Als_Crouch_Turn_90_Left",
        (true, false) => "A_Als_Crouch_Turn_90_Right",
    }
}

fn record_animation_window_trace(
    window: &mut ManualAnimationWindowEvidence,
    transform: &Transform,
    view_yaw: f32,
    rotation_animation: Option<&AlsRotationAnimationState>,
    crouching_graph: Option<&AlsCrouchingGraphTelemetry>,
    turn_drive: Option<&AlsTurnDrive>,
    turn_stack: Option<&AlsTurnMontageStack>,
    crouching: bool,
    animation_variant: Option<&'static str>,
) {
    window.observed_actor_yaw = Some(world_forward_to_yaw(transform.rotation * Vec3::NEG_Z));
    window.observed_view_yaw = Some(view_yaw);
    window.observed_animation_yaw_speed_degrees =
        rotation_animation.map(|value| value.yaw_speed_degrees);
    window.observed_animation_curve_direct_yaw_speed_degrees =
        crouching_graph.and_then(|value| value.graph_turn_curve_direct_yaw_speed_target);
    window.observed_animation_clip_time = turn_drive.map(|drive| {
        if drive.rotate_active || drive.rotate_finishing {
            drive.rotate_clip_time
        } else {
            drive.clip_time
        }
    });
    window.observed_animation_weight = turn_drive.map(|drive| {
        if drive.rotate_active || drive.rotate_finishing {
            1.0
        } else {
            drive.weight
        }
    });
    window.observed_animation_play_rate = turn_drive.map(|drive| {
        if drive.rotate_active || drive.rotate_finishing {
            drive.rotate_rate
        } else {
            drive.play_rate
        }
    });
    window.observed_animation_curve_stack_weight = turn_stack.map(|stack| {
        stack
            .entries
            .iter()
            .filter(|entry| entry.crouching == crouching)
            .map(|entry| entry.weight())
            .sum()
    });
    window.observed_animation_curve_stack_entries = turn_stack
        .map(|stack| {
            stack
                .entries
                .iter()
                .filter(|entry| entry.crouching == crouching)
                .map(|entry| ManualTurnStackEntryEvidence {
                    animation_variant: bevy_als::turn_layer::source_turn_sequence(
                        crouching,
                        entry.is_180,
                        entry.lr,
                    ),
                    elapsed_seconds: entry.elapsed,
                    weight: entry.weight(),
                    play_rate: entry.play_rate,
                    sampling_ready: entry.sampling_ready,
                })
                .collect()
        })
        .unwrap_or_default();
    window.observed_animation_variant = animation_variant.or_else(|| {
        turn_drive
            .map(|drive| bevy_als::turn_layer::source_turn_sequence(crouching, drive.is_180, drive.lr))
    });
}

fn crouch_stationary_source_precondition(
    motion: Option<&AlsActualMotion>,
    telemetry: Option<&AlsCrouchingGraphTelemetry>,
    drive: Option<&AlsTurnDrive>,
    slot: Option<&AlsStandingTurnSlotRuntime>,
) -> bool {
    motion.is_some_and(|motion| motion.horizontal_speed <= bevy_als::turn_layer::TURN_GROUND_SPEED_EPS)
        && telemetry.is_some_and(|telemetry| {
            telemetry.actual_outer_state == Some(0)
                && telemetry.actual_outer_transition_active == Some(false)
                && telemetry.actual_outer_pose_ready == Some(true)
        })
        && drive.is_none_or(|drive| {
            !drive.rotate_active
                && !drive.rotate_finishing
                && !drive.playing
                && drive.weight <= 0.05
        })
        && slot.is_none_or(|slot| !slot.active && slot.weight <= 0.05)
}

fn source_rotate_window_completed(active: bool, cycle_completed: bool, stationary: bool) -> bool {
    active && cycle_completed && stationary
}

fn advance_crouch_stationary_stability(frames: u8, stable: bool) -> u8 {
    if stable {
        frames
            .saturating_add(1)
            .min(CROUCH_STATIONARY_STABLE_FRAMES)
    } else {
        0
    }
}

fn crouch_direction_case_complete(ready_at: Option<f32>, elapsed: f32) -> bool {
    ready_at.is_some_and(|ready| {
        elapsed - ready >= CROUCH_DIRECTION_CASE_SECONDS - CROUCH_DIRECTION_CASE_SETTLE_SECONDS
    })
}

fn crouch_stationary_validation_holds_aim(validation: Option<CrouchStationaryValidation>) -> bool {
    validation.is_some_and(|validation| !validation.is_turn())
}

fn movement_direction_fsm_index(direction: AlsMovementDirection) -> u8 {
    match direction {
        AlsMovementDirection::Forward => 0,
        AlsMovementDirection::Backward => 1,
        AlsMovementDirection::RightForward => 2,
        AlsMovementDirection::RightBackward => 3,
        AlsMovementDirection::LeftForward => 4,
        AlsMovementDirection::LeftBackward => 5,
    }
}

fn movement_direction_from_fsm_index(index: u8) -> Option<AlsMovementDirection> {
    match index {
        0 => Some(AlsMovementDirection::Forward),
        1 => Some(AlsMovementDirection::Backward),
        2 => Some(AlsMovementDirection::RightForward),
        3 => Some(AlsMovementDirection::RightBackward),
        4 => Some(AlsMovementDirection::LeftForward),
        5 => Some(AlsMovementDirection::LeftBackward),
        _ => None,
    }
}

fn movement_direction_name(direction: AlsMovementDirection) -> &'static str {
    match direction {
        AlsMovementDirection::Forward => "Forward",
        AlsMovementDirection::Backward => "Backward",
        AlsMovementDirection::RightForward => "RightForward",
        AlsMovementDirection::RightBackward => "RightBackward",
        AlsMovementDirection::LeftForward => "LeftForward",
        AlsMovementDirection::LeftBackward => "LeftBackward",
    }
}

fn direction_branch_matches(index: Option<u8>, mask: u8) -> bool {
    index.is_some_and(|index| index < 8 && mask & (1 << index) != 0)
}

fn actual_velocity_branch_mask(motion: &AlsActualMotion) -> u8 {
    let weights = [
        motion.velocity_blend.forward,
        motion.velocity_blend.backward,
        motion.velocity_blend.left,
        motion.velocity_blend.right,
    ];
    let maximum = weights.into_iter().fold(0.0_f32, f32::max);
    if maximum <= f32::EPSILON {
        return 0;
    }
    weights
        .into_iter()
        .enumerate()
        .filter(|(_, weight)| *weight >= maximum - 1e-4)
        .fold(0, |mask, (index, _)| mask | (1 << index))
}

fn foot_ik_terrain_command(elapsed: f32, delta: f32) -> crate::input::AlsInputCommand {
    let pressed = |at| elapsed >= at && elapsed - delta < at;
    crate::input::AlsInputCommand {
        move_dir: if (1.0..3.0).contains(&elapsed) || (3.2..4.2).contains(&elapsed) {
            Vec2::Y
        } else {
            Vec2::ZERO
        },
        crouch_pressed: pressed(3.0) || pressed(4.2),
        jump_pressed: pressed(5.0),
        jump_held: (5.0..5.6).contains(&elapsed),
        roll_pressed: pressed(8.0),
        ..default()
    }
}

fn foot_ik_baseline_bit(index: usize) -> Option<u8> {
    match index {
        0 => Some(1 << 0),
        1 => Some(1 << 1),
        2 => Some(1 << 2),
        3 => Some(1 << 3),
        10 => Some(1 << 4),
        _ => None,
    }
}

#[derive(Default)]
struct IntegrationCoverage {
    response_reverse: bool,
    response_cycle: bool,
    response_idle: bool,
    assembly: bool,
    graph: bool,
    locomotion: bool,
    backward: bool,
    sideways: bool,
    standing_graph: bool,
    direction_cases: [bool; DIRECTION_CASE_COUNT],
    direction_source_states: u8,
    crouch_transaction: bool,
    crouch_graph: bool,
    crouch_air_forced_standing: bool,
    crouch_air_landed: bool,
    crouch_graph_recovered: bool,
    crouch_cycles: bool,
    crouch_scaled: bool,
    crouch_canopy_visible: bool,
    crouch_canopy_blocked: bool,
    crouch_canopy_recovered: bool,
    crouch_canopy_cleaned: bool,
    crouch_moving: bool,
    crouching: bool,
    crouch_direction_cases: [bool; CROUCH_DIRECTION_CASE_COUNT],
    crouch_direction_diagnostics: [bool; CROUCH_DIRECTION_CASE_COUNT],
    crouch_direction_state_mask: u8,
    crouch_direction_lean: bool,
    crouch_rotate_mask: u8,
    crouch_rotate_source_completion_mask: u8,
    crouch_rotate_completion_mask: u8,
    crouch_turn_active_mask: u8,
    crouch_turn_completion_mask: u8,
    sprinting: bool,
    airborne: bool,
    landed: bool,
    overlay: bool,
    overlay_interface_binding: bool,
    overlay_interface_fallback: bool,
    overlay_interface_reentry: bool,
    gait_layer: bool,
    stance_layer: bool,
    jump_variant: bool,
    turn_in_place: bool,
    layering: bool,
    overlay_object: bool,
    start_stop: bool,
    view_head: bool,
    control_rig: bool,
    control_rig_actions: bool,
    control_rig_stages: bool,
    control_rig_definition_hot_swap: bool,
    control_rig_input: bool,
    control_rig_foot_targets: bool,
    mantle_started: bool,
    mantle_completed: bool,
    mantle_debug: bool,
    ground_prediction_debug: bool,
    camera_debug_geometry: bool,
    movement_base: bool,
    foot_probe: bool,
    decal_receiver_swap: bool,
    reroute_passthrough: bool,
    machine_weight_partial: bool,
    machine_weight_full: bool,
    machine_weight_low_start: bool,
    machine_weight_full_start: bool,
    foot_lock: bool,
    foot_ik_space_baseline: bool,
    foot_ik_no_hit_recovered: bool,
    foot_ik_weight_ramp: bool,
    foot_ik_teleport: bool,
    foot_ik_slope: bool,
    foot_ik_step: bool,
    foot_ik_crouching: bool,
    foot_ik_jump_air: bool,
    foot_ik_landed: bool,
    foot_ik_roll: bool,
    foot_ik_mantle: bool,
    foot_ik_terrain_cleaned: bool,
    foot_bones_replaced: bool,
    animation_phase_rebound: bool,
    camera_root_rebound: bool,
    sprint_curve_relay: bool,
    roll_started: bool,
    roll_slot: bool,
    grounded_roll_exit_transition: bool,
    grounded_roll_completed: bool,
    grounded_stop_callback: bool,
    grounded_joint: [bool; 8],
    ragdoll_entered: bool,
    ragdoll_pa_profile: bool,
    ragdoll_player_replaced: bool,
    ragdoll_debug_render: bool,
    ragdoll_follow: bool,
    ragdoll_scaled: bool,
    ragdoll_joint_drive: bool,
    ragdoll_recovered: bool,
    ragdoll_camera_pivot: bool,
    get_up_front: bool,
    get_up_back: bool,
    get_up_input_locked: [bool; 2],
    get_up_input_released: [bool; 2],
    get_up_airborne: bool,
    get_up_rig_space_mismatch: bool,
    get_up_native_jump_rejected_during_action: bool,
    camera_rig: bool,
    camera_collision: bool,
    camera_recovery: bool,
    camera_first_person: bool,
    camera_shoulder: bool,
    camera_shake: bool,
    camera_unscaled_time: bool,
    hud_layout: bool,
    hud_toggle: bool,
    hud_slomo: bool,
    overlay_menu: bool,
    show_debug: bool,
    debug_draw: bool,
    notify_sprint: bool,
    notify_sprint_acceleration: bool,
    notify_roll: bool,
    notify_roll_reentry: bool,
    notify_marker: bool,
    notify_grounded_entry: bool,
    notify_root_scale: bool,
    notify_rolling_action: bool,
    notify_mantling_action: bool,
    notify_early_blend_out: bool,
    p1_ground: bool,
    p1_stance: bool,
    p1_gait: bool,
    p1_air: bool,
    p1_turn: bool,
    generated_transition_stop_quick: bool,
    standing_stop: [bool; 7],
    standing_turn: [bool; 8],
    standing_dynamic: [bool; 2],
    turn_slot: bool,
    turn_inertialization: bool,
}

#[derive(Resource)]
struct ManualIntegrationState {
    character: Option<Entity>,
    avatar: Option<Entity>,
    camera: Option<Entity>,
    spawn_translation: Option<Vec3>,
    spawn_yaw: Option<f32>,
    scenario: IntegrationScenario,
    phase: IntegrationPhase,
    phase_started_seconds: f64,
    game_time_seconds: f64,
    deadline: Instant,
    entered: bool,
    initialized: bool,
    camera_obstacles: [Option<Entity>; 3],
    camera_obstacle_ignored: bool,
    camera_blocked_ratio: Option<f32>,
    shake_requested: bool,
    dilation_active: bool,
    initial_view_yaw: Option<f32>,
    initial_view_pitch: Option<f32>,
    mantle_root_released: bool,
    ragdoll_seen: bool,
    expect_ragdoll_player_replacement: bool,
    expect_ragdoll_debug_render: bool,
    expect_control_rig_definition_hot_swap: bool,
    expect_decal_receiver_swap: bool,
    expect_reroute_passthrough: bool,
    expect_machine_weight: bool,
    machine_weight_partial_phase: Option<IntegrationPhase>,
    ragdoll_exit_requested: bool,
    ragdoll_frames: u32,
    get_up_case: usize,
    get_up_jump_requested_at: Option<f64>,
    get_up_fixture_seeded: bool,
    ragdoll_grounded_at: Option<f64>,
    ragdoll_recovered_at: Option<f64>,
    foot_replacement_avatar: Option<Entity>,
    foot_replacement_old_left: Option<Entity>,
    foot_replacement_old_root: Option<Entity>,
    foot_replacement_waited: f32,
    foot_replacement_progress: &'static str,
    ragdoll_current_joint_anchor_error_m: f32,
    ragdoll_current_joint_anchor_label: Option<String>,
    ragdoll_max_joint_anchor_error_m: f32,
    ragdoll_max_joint_anchor_label: Option<String>,
    ragdoll_handoff_locked_joint_anchor_error_m: f32,
    ragdoll_handoff_locked_joint_anchor_label: Option<String>,
    ragdoll_handoff_pose_error_m: f32,
    ragdoll_handoff_rotation_error_radians: f32,
    camera_scenario_armed: bool,
    camera_last_probe_secs: f32,
    camera_view_mode_requested: bool,
    camera_shoulder_requested: bool,
    camera_movement: CameraMovementProbe,
    interface_debug_armed: bool,
    interface_hud_hidden_seen: bool,
    interface_hud_restored_seen: bool,
    interface_slomo_seen: bool,
    interface_slomo_restored_seen: bool,
    interface_menu_open_seen: bool,
    interface_menu_closed_seen: bool,
    interface_overlay_bow_seen: bool,
    interface_overlay_default_after_bow_seen: bool,
    interface_overlay_reentry_seen: bool,
    interface_parent_instance: Option<u64>,
    interface_parent_preserved: bool,
    notify_roll_armed: bool,
    notify_roll_restarted: bool,
    notify_mantle_armed: bool,
    direction_case_index: usize,
    sprint_signature_phase: Option<f32>,
    sprint_samples: [Option<(f32, f32)>; 2],
    crouch_direction_case_index: usize,
    crouch_direction_case_started_at: f32,
    crouch_direction_ready_at: Option<f32>,
    crouch_stationary_validation: Option<CrouchStationaryValidation>,
    temporal_rebase_epoch: u64,
    crouch_stationary_precondition_ready: [bool; 2],
    crouch_stationary_precondition_frames: [u8; 2],
    crouch_stationary_base_yaw: [Option<f32>; 2],
    crouch_stationary_view_yaw: f32,
    crouch_stationary_started_at: [Option<f32>; 2],
    crouch_turn_epoch_baseline: [Option<f32>; 2],
    foot_ik_baseline_mask: u8,
    foot_ik_no_hit_logged: bool,
    foot_ik_weight_mask: u8,
    foot_ik_teleport_requested: bool,
    foot_ik_teleport_baseline: Option<(u32, u32, u32)>,
    foot_ik_terrain_air_seen: bool,
    crouch_recovery_jump_requested: bool,
    crouch_recovery_air_crouch_requested: bool,
    crouch_recovery_standing_requested: bool,
    crouch_clearance_stage: CrouchClearanceStage,
    crouch_clearance_cycles: u16,
    crouch_clearance_last_actual: Stance,
    crouch_canopy: Option<Entity>,
    crouch_canopy_center: Option<Vec3>,
    crouch_canopy_prepare_at: Option<f64>,
    crouch_canopy_armed_at: Option<f64>,
    foot_ik_slope: Option<Entity>,
    foot_ik_step: Option<Entity>,
    p1_pivot_sent: bool,
    p1_ground_mask: u8,
    p1_gait_mask: u8,
    p1_air_mask: u8,
    p1_turn_mask: u8,
    p1_stance_mask: u8,
    generated_stop_quick_input_started: bool,
    generated_stop_quick_input_released: bool,
    generated_stop_quick_count_before: Option<u32>,
    stop_case_index: usize,
    stop_foot_planted: Option<f32>,
    stop_branch_seen: bool,
    stop_playback: Option<AlsTransitionPlayback>,
    turn_case_index: usize,
    turn_probe: StandingTurnProbe,
    dynamic_unlocked: bool,
    dynamic_selection_valid: bool,
    grounded_roll_exit_diagnostic: String,
    grounded_roll_playback: Option<AlsTransitionPlayback>,
    grounded_joint_probe: GroundedJointProbe,
    locomotion_probe: LocomotionActionProbe,
    default_overlay_probe: DefaultOverlayProbe,
    bow_probe: bow::Probe,
    overlays_probe: overlays::Probe,
    footprint_probe: footprints::Probe,
    head_probe: HeadProbe,
    stairs_probe: stairs::Probe,
    uphill_probe: uphill::Probe,
    moving_base_probe: moving_base::Probe,
    timed_out: bool,
    final_logged: bool,
    exit_requested: bool,
    coverage: IntegrationCoverage,
}

impl Default for ManualIntegrationState {
    fn default() -> Self {
        let now = Instant::now();
        Self {
            character: None,
            avatar: None,
            camera: None,
            spawn_translation: None,
            spawn_yaw: None,
            scenario: IntegrationScenario::from_environment(),
            phase: IntegrationPhase::Await,
            phase_started_seconds: 0.0,
            game_time_seconds: 0.0,
            deadline: now + GATE_TIMEOUT,
            entered: false,
            initialized: false,
            camera_obstacles: [None; 3],
            camera_obstacle_ignored: false,
            camera_blocked_ratio: None,
            shake_requested: false,
            dilation_active: false,
            initial_view_yaw: None,
            initial_view_pitch: None,
            mantle_root_released: false,
            ragdoll_seen: false,
            expect_ragdoll_player_replacement: std::env::var(
                "ALS_MANUAL_RAGDOLL_PLAYER_REPLACEMENT",
            )
            .as_deref()
                == Ok("1"),
            expect_ragdoll_debug_render: std::env::var("ALS_MANUAL_RAGDOLL_DEBUG_RENDER")
                .as_deref()
                == Ok("1"),
            expect_control_rig_definition_hot_swap: std::env::var(
                "ALS_MANUAL_RIG_DEFINITION_HOT_SWAP",
            )
            .as_deref()
                == Ok("1"),
            expect_decal_receiver_swap: std::env::var("ALS_MANUAL_DECAL_RECEIVER_SWAP").as_deref()
                == Ok("1"),
            expect_reroute_passthrough: std::env::var("ALS_MANUAL_REROUTE_PASSTHROUGH").as_deref()
                == Ok("1"),
            expect_machine_weight: std::env::var("ALS_MANUAL_MACHINE_WEIGHT").as_deref() == Ok("1"),
            machine_weight_partial_phase: None,
            ragdoll_exit_requested: false,
            ragdoll_frames: 0,
            get_up_case: 0,
            get_up_jump_requested_at: None,
            get_up_fixture_seeded: false,
            ragdoll_grounded_at: None,
            ragdoll_recovered_at: None,
            foot_replacement_avatar: None,
            foot_replacement_old_left: None,
            foot_replacement_old_root: None,
            foot_replacement_waited: 0.0,
            foot_replacement_progress: "not_started",
            ragdoll_current_joint_anchor_error_m: 0.0,
            ragdoll_current_joint_anchor_label: None,
            ragdoll_max_joint_anchor_error_m: 0.0,
            ragdoll_max_joint_anchor_label: None,
            ragdoll_handoff_locked_joint_anchor_error_m: 0.0,
            ragdoll_handoff_locked_joint_anchor_label: None,
            ragdoll_handoff_pose_error_m: 0.0,
            ragdoll_handoff_rotation_error_radians: 0.0,
            camera_scenario_armed: false,
            camera_last_probe_secs: -1.0,
            camera_view_mode_requested: false,
            camera_shoulder_requested: false,
            camera_movement: default(),
            interface_debug_armed: false,
            interface_hud_hidden_seen: false,
            interface_hud_restored_seen: false,
            interface_slomo_seen: false,
            interface_slomo_restored_seen: false,
            interface_menu_open_seen: false,
            interface_menu_closed_seen: false,
            interface_overlay_bow_seen: false,
            interface_overlay_default_after_bow_seen: false,
            interface_overlay_reentry_seen: false,
            interface_parent_instance: None,
            interface_parent_preserved: true,
            notify_roll_armed: false,
            notify_roll_restarted: false,
            notify_mantle_armed: false,
            direction_case_index: usize::MAX,
            sprint_signature_phase: None,
            sprint_samples: [None; 2],
            crouch_direction_case_index: usize::MAX,
            crouch_direction_case_started_at: 0.0,
            crouch_direction_ready_at: None,
            crouch_stationary_validation: None,
            temporal_rebase_epoch: 0,
            crouch_stationary_precondition_ready: [false; 2],
            crouch_stationary_precondition_frames: [0; 2],
            crouch_stationary_base_yaw: [None; 2],
            crouch_stationary_view_yaw: 0.0,
            crouch_stationary_started_at: [None; 2],
            crouch_turn_epoch_baseline: [None; 2],
            foot_ik_baseline_mask: 0,
            foot_ik_no_hit_logged: false,
            foot_ik_weight_mask: 0,
            foot_ik_teleport_requested: false,
            foot_ik_teleport_baseline: None,
            foot_ik_terrain_air_seen: false,
            crouch_recovery_jump_requested: false,
            crouch_recovery_air_crouch_requested: false,
            crouch_recovery_standing_requested: false,
            crouch_clearance_stage: CrouchClearanceStage::default(),
            crouch_clearance_cycles: 0,
            crouch_clearance_last_actual: Stance::Standing,
            crouch_canopy: None,
            crouch_canopy_center: None,
            crouch_canopy_prepare_at: None,
            crouch_canopy_armed_at: None,
            foot_ik_slope: None,
            foot_ik_step: None,
            p1_pivot_sent: false,
            p1_ground_mask: 0,
            p1_gait_mask: 0,
            p1_air_mask: 0,
            p1_turn_mask: 0,
            p1_stance_mask: 0,
            generated_stop_quick_input_started: false,
            generated_stop_quick_input_released: false,
            generated_stop_quick_count_before: None,
            stop_case_index: 0,
            stop_foot_planted: None,
            stop_branch_seen: false,
            stop_playback: None,
            turn_case_index: 0,
            turn_probe: StandingTurnProbe::default(),
            dynamic_unlocked: false,
            dynamic_selection_valid: true,
            grounded_roll_exit_diagnostic: "not observed".to_owned(),
            grounded_roll_playback: None,
            grounded_joint_probe: default(),
            locomotion_probe: default(),
            default_overlay_probe: default(),
            bow_probe: default(),
            overlays_probe: default(),
            footprint_probe: default(),
            head_probe: default(),
            stairs_probe: default(),
            uphill_probe: default(),
            moving_base_probe: default(),
            timed_out: false,
            final_logged: false,
            exit_requested: false,
            coverage: IntegrationCoverage::default(),
        }
    }
}

impl ManualIntegrationState {
    fn phase_elapsed(&self) -> f32 {
        (self.game_time_seconds - self.phase_started_seconds).max(0.0) as f32
    }
}

#[derive(SystemParam)]
struct InterfaceProbe<'w, 's> {
    virtual_time: Res<'w, Time<Virtual>>,
    visual_progress: Option<Res<'w, AnimationVisualValidationProgress>>,
    hud_state: Res<'w, AlsHudState>,
    menu_state: Res<'w, OverlayMenuState>,
    menu_nodes: Query<'w, 's, &'static Node>,
    show_debug: Res<'w, AlsShowDebugHudState>,
    debug_stats: Res<'w, AlsDebugDisplayStats>,
    overlay_props: Query<'w, 's, (), With<AlsOverlayObject>>,
    hud_roots: Query<'w, 's, (), With<AlsHudRoot>>,
    hud_debug_nodes: Query<'w, 's, &'static Node, With<AlsHudDebugSection>>,
    hud_input_nodes: Query<'w, 's, &'static Node, With<AlsHudInputSection>>,
    show_debug_roots:
        Query<'w, 's, (&'static Node, &'static Visibility), With<AlsShowDebugHudRoot>>,
    debug_values: Query<'w, 's, (&'static Name, &'static Text)>,
    rotation_responses: Query<'w, 's, &'static AlsRotationResponseState>,
    desired_rotation_modes: Query<'w, 's, &'static bevy_als::als::aim::AlsLastFreeRotationMode>,
    camera_assemblies: Query<'w, 's, &'static AlsCharacterAssembly>,
    camera_sources: Query<'w, 's, &'static bevy_als::als_camera::AlsCameraViewSource>,
    camera_debug: Query<'w, 's, &'static AlsCameraDebugInfo>,
    motion_debug: Query<'w, 's, &'static bevy_als::als::motion::AlsCharacterDebug>,
    ground_debug: Query<'w, 's, &'static bevy_als::locomotion_layer::AlsGroundPredictionTrace>,
}

#[derive(SystemParam)]
struct MotionProbe<'w, 's> {
    transforms: bevy::transform::helper::TransformHelper<'w, 's>,
    physics: Query<'w, 's, (&'static Position, &'static Rotation), With<AlsCharacter>>,
    player_views: Query<'w, 's, &'static bevy_als::als::view::AlsPlayerViewYaw>,
    controllers: Query<
        'w,
        's,
        &'static bevy_tnua::TnuaController<bevy_als::als::control_scheme::AlsControlScheme>,
    >,
    rendered_views: Query<
        'w,
        's,
        (
            &'static Camera,
            &'static Transform,
            &'static bevy_als::als_camera::AlsCharacterCamera,
        ),
        (With<Camera3d>, With<bevy_als::als_camera::AlsCameraViewSource>),
    >,
    source_inputs: Query<'w, 's, &'static bevy_als::extensions::SourceAnimationInputs>,
    asset_server: Res<'w, AssetServer>,
    graphs: Res<'w, Assets<AnimationGraph>>,
    clips: Res<'w, Assets<bevy_animation_graph::core::animation_clip::GraphClip>>,
    avatar_space:
        Query<'w, 's, (&'static Transform, &'static GlobalTransform), With<AlsCharacterAvatar>>,
    skeletons: Res<'w, Assets<Skeleton>>,
    machines: Res<'w, Assets<StateMachine>>,
    keys: Res<'w, ButtonInput<KeyCode>>,
    inputs: Query<
        'w,
        's,
        (
            Option<&'static crate::input::AlsInputCommand>,
            Option<&'static AlsInputOverride>,
        ),
        With<AlsCharacter>,
    >,
    rotation_mode: Query<'w, 's, &'static RotationMode, With<AlsCharacter>>,
    standing: Query<'w, 's, &'static AlsStandingGraphTelemetry, With<AlsCharacter>>,
    crouching_graph: Query<'w, 's, &'static AlsCrouchingGraphTelemetry, With<AlsCharacter>>,
    grounded: Query<'w, 's, &'static AlsGroundedGraphTelemetry, With<AlsCharacter>>,
    crouch: Query<'w, 's, &'static AlsCrouchTransaction, With<AlsCharacter>>,
    actual: Query<'w, 's, &'static AlsActualMotion, With<AlsCharacter>>,
    gait: Query<'w, 's, (&'static AlsDesiredGait, &'static AlsMaxAllowedGait), With<AlsCharacter>>,
    rotation_animation:
        Query<'w, 's, Option<&'static AlsRotationAnimationState>, With<AlsCharacter>>,
    transition_notify:
        Query<'w, 's, Option<&'static TransitionNotifyTelemetry>, With<AlsCharacter>>,
    overlay_objects: Query<
        'w,
        's,
        (
            &'static AlsOverlayObject,
            Option<&'static bevy_als::extensions::overlay_materials::AvatarMaterialApplied>,
        ),
    >,
    object_scenes: Query<'w, 's, &'static AnimatedSceneInstance>,
    object_players: Query<'w, 's, &'static AnimationGraphPlayer>,
    object_transforms: Query<'w, 's, &'static Transform>,
    object_children: Query<'w, 's, &'static Children>,
    object_mesh_materials: Query<
        'w,
        's,
        (
            Option<&'static MeshMaterial3d<StandardMaterial>>,
            Option<&'static crate::projected_decal::StandardMaterialSource>,
        ),
        With<Mesh3d>,
    >,
    object_material_assets: Res<'w, Assets<StandardMaterial>>,
}

#[derive(SystemParam)]
struct RagdollPaProbe<'w, 's> {
    replacements: Query<'w, 's, &'static ManualRagdollPlayerReplacement>,
    alive: Query<'w, 's, Entity>,
    labels: Query<'w, 's, &'static BodyLabel>,
    colliders: Query<'w, 's, (&'static Collider, &'static ColliderLabel)>,
    joints: Query<'w, 's, (&'static SourcePhysicsJoint, &'static JointLabel)>,
    profiles: Query<
        'w,
        's,
        (
            &'static RagdollPhysicsProfileApplied,
            &'static RagdollPhysicsProfile,
        ),
    >,
    collision_members: Query<'w, 's, &'static RagdollCollisionMember>,
    collision_layers: Query<'w, 's, &'static CollisionLayers>,
    body_modes: Query<'w, 's, &'static RigidBody>,
    get_ups: Query<
        'w,
        's,
        (
            Option<&'static bevy_als::get_up::AlsGetUpState>,
            Has<bevy_als::als::motion::AlsMovementInputBlocked>,
            Option<&'static bevy_als::als_camera::AlsCameraCustomTimeDilation>,
        ),
        With<AlsCharacter>,
    >,
    camera_sockets: Query<
        'w,
        's,
        (
            &'static bevy_als::als_camera::AlsCameraSocketLocations,
            &'static GlobalTransform,
            &'static Collider,
        ),
        With<AlsCharacter>,
    >,
}

fn get_up_fixture_rotation(facing_up: bool, yaw: f32) -> Quat {
    Quat::from_rotation_y(yaw)
        * Quat::from_rotation_x(if facing_up {
            -std::f32::consts::FRAC_PI_2
        } else {
            std::f32::consts::FRAC_PI_2
        })
}

fn prepare_get_up_physics_fixture(
    mut state: ResMut<ManualIntegrationState>,
    mut phase: ResMut<ManualRuntimePhaseEvidence>,
    characters: Query<&AlsNotifyLocomotionAction, With<AlsCharacter>>,
    avatars: Query<(&AlsCharacterAvatar, &AnimatedSceneInstance)>,
    players: Query<&AnimationGraphPlayer>,
    labels: Query<&BodyLabel>,
    mut bodies: Query<
        (
            &mut Position,
            &mut Rotation,
            &mut LinearVelocity,
            &mut avian3d::prelude::AngularVelocity,
            &RigidBody,
        ),
        Without<AlsCharacter>,
    >,
) {
    if state.scenario != IntegrationScenario::GetUp
        || state.get_up_fixture_seeded
        || state.phase != IntegrationPhase::Ragdoll
    {
        return;
    }
    let Some(character) = state.character else {
        return;
    };
    if characters.get(character) != Ok(&AlsNotifyLocomotionAction::Ragdolling) {
        return;
    }
    let Some(spawned) = avatars
        .iter()
        .find(|(avatar, _)| avatar.character == character)
        .and_then(|(_, instance)| players.get(instance.player_entity()).ok())
        .and_then(AnimationGraphPlayer::get_spawned_ragdoll)
    else {
        return;
    };
    let Some(pelvis) = spawned
        .bodies
        .values()
        .find(|entity| labels.get(**entity).is_ok_and(|label| label.0 == "pelvis"))
    else {
        return;
    };
    let Ok((position, rotation, _, _, _)) = bodies.get(*pelvis) else {
        return;
    };
    let pivot = position.0;
    let facing_up = state.get_up_case == 1;
    let delta = get_up_fixture_rotation(facing_up, state.spawn_yaw.unwrap_or_default())
        * rotation.0.inverse();
    for entity in spawned.bodies.values() {
        let Ok((mut position, mut rotation, mut velocity, mut angular_velocity, body)) =
            bodies.get_mut(*entity)
        else {
            continue;
        };
        if *body != RigidBody::Dynamic {
            continue;
        }
        position.0 = pivot + delta * (position.0 - pivot);
        rotation.0 = (delta * rotation.0).normalize();
        velocity.0 = Vec3::ZERO;
        angular_velocity.0 = Vec3::ZERO;
    }
    state.get_up_fixture_seeded = true;
    rebase_temporal_fixture(
        &mut state,
        &mut phase,
        if facing_up {
            "get_up_back_initial_orientation"
        } else {
            "get_up_front_initial_orientation"
        },
    );
}

fn spawn_manual_integration_fixture(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    origin: Vec3,
    facing_yaw: f32,
    scenario: IntegrationScenario,
) {
    let facing = Quat::from_rotation_y(facing_yaw);
    let forward = facing * Vec3::NEG_Z;
    let right = facing * Vec3::X;
    let ground_y = origin.y - 0.92;
    let mantle_height = if scenario == IntegrationScenario::BowOverlay {
        1.6
    } else {
        0.8
    };
    let ledge_center =
        origin + forward * 6.0 + Vec3::Y * (ground_y + mantle_height * 0.5 - origin.y);
    let mantle_start = origin - forward * 1.5;
    let platform_home =
        origin + right * 9.0 + forward * 2.0 + Vec3::Y * (ground_y + 0.25 - origin.y);
    let locomotion_start = origin - right * 8.0 - forward * 2.0;
    let roll_start = origin - right * 8.0 + forward * 7.0;
    let ledge_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.12, 0.48, 0.76),
        perceptual_roughness: 0.7,
        ..default()
    });
    let platform_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.95, 0.48, 0.08),
        perceptual_roughness: 0.6,
        ..default()
    });
    let obstacle_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.75, 0.08, 0.18),
        perceptual_roughness: 0.5,
        ..default()
    });
    commands.spawn((
        Name::new("manual_integration_mantle_ledge"),
        RigidBody::Static,
        CollisionLayers::new(
            [AlsCollisionLayer::Default, AlsCollisionLayer::Visibility],
            avian3d::prelude::LayerMask::ALL,
        ),
        Collider::cuboid(2.4, mantle_height, 1.2),
        Transform::from_translation(ledge_center),
        Mesh3d(meshes.add(Mesh::from(Cuboid::new(4.8, 1.6, 2.4)))),
        MeshMaterial3d(ledge_material),
    ));
    let platform = commands
        .spawn((
            Name::new("manual_integration_platform"),
            ManualIntegrationPlatform,
            RigidBody::Kinematic,
            CollisionLayers::new(
                [AlsCollisionLayer::Default, AlsCollisionLayer::Visibility],
                avian3d::prelude::LayerMask::ALL,
            ),
            Collider::cuboid(6.0, 0.5, 6.0),
            LinearVelocity(Vec3::X * 1.5),
            Transform::from_translation(platform_home),
            Mesh3d(meshes.add(Mesh::from(Cuboid::new(6.0, 0.5, 6.0)))),
            MeshMaterial3d(platform_material),
        ))
        .id();
    commands.insert_resource(ManualIntegrationFixture {
        platform,
        platform_home,
        locomotion_start,
        mantle_start,
        roll_start,
        facing_yaw,
        obstacle_mesh: meshes.add(Mesh::from(Cuboid::new(1.6, 1.6, 1.6))),
        obstacle_material,
    });
    info!(
        "[MANUAL-INTEGRATION] visible fixture attached to playground origin=({:.2},{:.2},{:.2})",
        origin.x, origin.y, origin.z
    );
}

fn initialize_manual_integration(
    mut commands: Commands,
    mut state: ResMut<ManualIntegrationState>,
    virtual_time: Res<Time<Virtual>>,
    mut foot_ik_telemetry_enabled: ResMut<AlsFootIkTelemetryEnabled>,
    mut foot_ik_weight_override: ResMut<AlsFootIkWeightOverride>,
    startup: Res<ManualPlaygroundStartupReady>,
    admissions: Query<&AlsPlayerAdmissionComplete>,
    assemblies: Query<&AlsCharacterAssembly>,
    avatars: Query<&AnimatedSceneInstance, With<AlsCharacterAvatar>>,
    players: Query<&AnimationGraphPlayer>,
    cameras: (
        Query<(&AlsCameraRig, &AlsCameraDebugInfo)>,
        Query<&bevy_als::als_camera::AlsCameraViewSource>,
    ),
    toggles: Query<&TnuaToggle, With<AlsCharacter>>,
    view: Query<(&AlsViewYaw, &AlsViewPitch), With<AlsCharacter>>,
    (transforms, unowned): (
        Query<&Transform, With<AlsCharacter>>,
        Query<Entity, (With<AlsCharacter>, Without<AlsInputOverride>)>,
    ),
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for character in &unowned {
        commands
            .entity(character)
            .insert(AlsInputOverride::default());
    }
    if state.initialized || !startup.passed {
        return;
    }
    state.game_time_seconds = virtual_time.elapsed_secs_f64();
    let Ok(admission) = admissions.single() else {
        return;
    };
    let character = admission.character;
    let Ok(assembly) = assemblies.get(character) else {
        return;
    };
    let Ok(instance) = avatars.get(assembly.avatar) else {
        return;
    };
    let Ok(player) = players.get(instance.player_entity()) else {
        return;
    };
    let Some(camera) = assembly.camera else {
        return;
    };
    let camera = cameras.1.get(camera).map_or(camera, |source| source.0);
    let Ok((_rig, debug)) = cameras.0.get(camera) else {
        return;
    };
    let Ok(toggle) = toggles.get(character) else {
        return;
    };
    let Ok((view_yaw, view_pitch)) = view.get(character) else {
        return;
    };
    let Ok(transform) = transforms.get(character) else {
        return;
    };
    let graph_ready = matches!(player.get_animation_source(), AnimationSource::Graph(_))
        && player.get_error().is_none();
    if !graph_ready || !debug.valid || *toggle != TnuaToggle::Enabled {
        return;
    }
    let spawn_yaw = transform.rotation.to_euler(EulerRot::YXZ).0;
    spawn_manual_integration_fixture(
        &mut commands,
        &mut meshes,
        &mut materials,
        transform.translation,
        spawn_yaw,
        state.scenario,
    );
    state.character = Some(character);
    state.avatar = Some(assembly.avatar);
    state.camera = Some(camera);
    state.spawn_translation = Some(transform.translation);
    state.spawn_yaw = Some(spawn_yaw);
    state.initial_view_yaw = Some(view_yaw.0);
    state.initial_view_pitch = Some(view_pitch.0);
    state.initialized = true;
    foot_ik_telemetry_enabled.0 = true;
    foot_ik_weight_override.left = None;
    foot_ik_weight_override.right = None;
    state.coverage.assembly = true;
    state.coverage.graph = true;
    let first_phase = initial_phase(state.scenario);
    begin_phase(&mut state, first_phase);
    info!(
        "[MANUAL-INTEGRATION][EXECUTED] runtime_assembly scenario={:?} character={character:?} avatar={:?} camera={camera:?} playable_plugin=AlsPlayableCharacterPlugin runtime_plugin=AlsCharacterRuntimePlugin graph=Graph tnua=Enabled camera_valid=true",
        state.scenario, assembly.avatar,
    );
}

fn drive_foot_ik_avatar_replacement(
    mut commands: Commands,
    mut state: ResMut<ManualIntegrationState>,
    mut phase_evidence: ResMut<ManualRuntimePhaseEvidence>,
    assets: Res<AssetServer>,
    scenes: Res<Assets<AnimatedScene>>,
    mut assemblies: Query<&mut AlsCharacterAssembly>,
    instances: Query<&AnimatedSceneInstance>,
    scene_handles: Query<&AnimatedSceneHandle>,
    mut players: Query<&mut AnimationGraphPlayer>,
    bones: Query<&AlsFootBones>,
    locks: Query<&AlsFootLockState>,
    children: Query<&Children>,
    names: Query<&Name>,
    targets: Query<&AnimationTargetId>,
    mut camera_bindings: Query<(&mut AlsCameraSocketBinding, &AlsCameraSocketLocations)>,
    rebuild_queries: (
        ResMut<bevy::world_serialization::WorldInstanceSpawner>,
        Query<&GlobalTransform>,
        Query<&bevy::animation::AnimatedBy>,
        Query<Entity, With<bevy_als::extensions::post_physics_animation::PostPhysicsAnimation>>,
        Query<Entity>,
    ),
) {
    let (mut world_spawner, globals, bone_players, late_players, alive) = rebuild_queries;
    if state.scenario != IntegrationScenario::FootIk
        || state.phase != IntegrationPhase::Roll
        || state.phase_elapsed() < FOOT_AVATAR_REPLACEMENT_START_SECONDS
        || (state.coverage.foot_bones_replaced && state.coverage.camera_root_rebound)
    {
        return;
    }
    let Some(character) = state.character else {
        return;
    };
    let Some(old_avatar) = state.avatar else {
        return;
    };
    let Some(new_avatar) = state.foot_replacement_avatar else {
        let Ok(old_bones) = bones.get(character) else {
            return;
        };
        let Ok((camera_binding, _)) = camera_bindings.get_mut(character) else {
            return;
        };
        let Some(old_root) = camera_binding.root_bone else {
            return;
        };
        commands.entity(character).insert(Visibility::Inherited);
        let rebuild = std::env::var_os("ALS_MANUAL_ANIMATION_PHASE_REBUILD").is_some();
        let new_avatar = if rebuild {
            let Ok(instance) = instances.get(old_avatar) else {
                return;
            };
            let Ok(handle) = scene_handles.get(old_avatar) else {
                return;
            };
            let Some(scene) = scenes.get(&handle.handle) else {
                return;
            };
            let Some(processed) = scene.processed_scene.as_ref() else {
                return;
            };
            let Ok(world) = globals.get(instance.player_entity()) else {
                return;
            };
            commands
                .entity(instance.player_entity())
                .remove::<ChildOf>()
                .insert((world.compute_transform(), Visibility::Hidden));
            world_spawner.spawn_as_child(processed.clone(), old_avatar);
            old_avatar
        } else {
            commands
                .spawn((
                    AnimatedSceneHandle {
                        handle: assets.load(ALS_LINKED_ANIMATED_SCENE),
                        override_source: Some(AnimationSource::None),
                    },
                    AlsCharacterAvatar { character },
                    avatar_transform(),
                    Visibility::Hidden,
                    ChildOf(character),
                ))
                .id()
        };
        state.foot_replacement_avatar = Some(new_avatar);
        state.foot_replacement_old_left = Some(old_bones.left);
        state.foot_replacement_old_root = Some(old_root);
        state.foot_replacement_progress = "await_instance";
        info!("[MANUAL-INTEGRATION][FOOT-BONES-REPLACE] spawned avatar={new_avatar:?}");
        return;
    };
    let Ok(instance) = instances.get(new_avatar) else {
        return;
    };
    state.foot_replacement_progress = "await_player";
    if !players.contains(instance.player_entity()) {
        return;
    }
    state.foot_replacement_progress = "await_bones";
    let Some(new_bones) =
        build_foot_bone_map(instance.player_entity(), &children, &names, &targets)
    else {
        return;
    };
    if std::env::var_os("ALS_MANUAL_ANIMATION_PHASE_REBUILD").is_some() {
        let Some(old_left) = state.foot_replacement_old_left else {
            return;
        };
        if new_bones.left == old_left {
            return;
        }
        let Ok(old_player) = bone_players.get(old_left) else {
            return;
        };
        if !late_players.contains(instance.player_entity()) || late_players.contains(old_player.0) {
            state.foot_replacement_progress = "await_player_phase";
            return;
        }
        if !state.coverage.animation_phase_rebound {
            if let Ok(mut previous) = players.get_mut(old_player.0) {
                previous.set_animation(AnimationSource::None);
            }
            state.coverage.animation_phase_rebound = true;
            rebase_temporal_fixture(
                &mut state,
                &mut phase_evidence,
                "retained_avatar_player_rebuild",
            );
            info!(
                "[MANUAL-INTEGRATION][ANIMATION-PHASE-REBUILD] avatar={old_avatar:?} old_player={:?} current_player={:?} -> PASS",
                old_player.0,
                instance.player_entity()
            );
        }
    }
    if old_avatar != new_avatar {
        let Ok(mut player) = players.get_mut(instance.player_entity()) else {
            return;
        };
        let Ok(mut assembly) = assemblies.get_mut(character) else {
            return;
        };
        let Ok(scene_handle) = scene_handles.get(new_avatar) else {
            return;
        };
        let Some(scene) = scenes.get(&scene_handle.handle) else {
            return;
        };
        player.set_animation(AnimationSource::Graph(scene.animation_graph.clone()));
        drop(player);
        if let Ok(old_instance) = instances.get(old_avatar)
            && let Ok(mut old_player) = players.get_mut(old_instance.player_entity())
        {
            old_player.set_animation(AnimationSource::None);
        }
        assembly.avatar = new_avatar;
        commands.entity(old_avatar).insert((
            Visibility::Hidden,
            Transform::from_xyz(4.0, 1.0, -2.0)
                .with_rotation(Quat::from_rotation_y(0.7))
                .with_scale(Vec3::splat(0.8)),
        ));
        commands.entity(new_avatar).insert(Visibility::Visible);
        let Ok((mut camera_binding, _)) = camera_bindings.get_mut(character) else {
            return;
        };
        camera_binding.avatar = new_avatar;
        state.avatar = Some(new_avatar);
        state.foot_replacement_progress = "await_current_foot_map";
        rebase_temporal_fixture(&mut state, &mut phase_evidence, "foot_ik_avatar_replace");
        info!("[MANUAL-INTEGRATION][FOOT-BONES-REPLACE] selected avatar={new_avatar:?}");
        return;
    }
    let Some(old_left) = state.foot_replacement_old_left else {
        return;
    };
    let Some(old_root) = state.foot_replacement_old_root else {
        return;
    };
    let old_alive = alive.get(old_left).is_ok();
    let map_matches = bones
        .get(character)
        .is_ok_and(|current| *current == new_bones);
    let camera_ready = camera_bindings
        .get_mut(character)
        .is_ok_and(|(binding, sockets)| {
            alive.get(old_root).is_ok()
                && binding.root_bone.is_some_and(|root| {
                    root != old_root
                        && children
                            .iter_descendants(new_avatar)
                            .any(|entity| entity == root)
                })
                && sockets.valid
        });
    state.coverage.camera_root_rebound |= camera_ready;
    state.foot_replacement_progress = if !old_alive {
        "old_foot_missing"
    } else if !map_matches {
        "await_current_foot_map"
    } else {
        "await_foot_lock"
    };
    if !state.coverage.foot_bones_replaced
        && new_bones.left != old_left
        && old_alive
        && map_matches
        && locks.get(character).is_ok_and(|lock| lock.valid)
    {
        state.coverage.foot_bones_replaced = true;
        info!(
            "[MANUAL-INTEGRATION][FOOT-BONES-REPLACE] old={old_left:?} new={:?} -> PASS",
            new_bones.left
        );
    }
    if state.coverage.foot_bones_replaced && state.coverage.camera_root_rebound {
        state.foot_replacement_progress = "complete";
        info!(
            "[MANUAL-INTEGRATION][CAMERA-ROOT-REPLACE] old={old_root:?} current_avatar={new_avatar:?} -> PASS"
        );
    } else if state.coverage.foot_bones_replaced {
        state.foot_replacement_progress = "await_camera_root";
    }
}

fn foot_avatar_replacement_timed_out(elapsed: f32, covered: bool) -> bool {
    !covered
        && elapsed
            >= FOOT_AVATAR_REPLACEMENT_START_SECONDS
                + FOOT_AVATAR_REPLACEMENT_NO_PROGRESS_LIMIT_SECONDS
}

#[allow(clippy::type_complexity)]
fn replace_ragdoll_player_once(world: &mut World) {
    let state = world.resource::<ManualIntegrationState>();
    if !state.initialized
        || state.scenario != IntegrationScenario::Ragdoll
        || state.phase != IntegrationPhase::Ragdoll
        || state.phase_elapsed() > 0.15
    {
        return;
    }
    let Some(avatar) = state.avatar else {
        return;
    };
    let Some(instance) = world.get::<AnimatedSceneInstance>(avatar) else {
        return;
    };
    let player_entity = instance.player_entity();
    if world
        .get::<ManualRagdollPlayerReplacement>(player_entity)
        .is_some()
    {
        return;
    }
    let Some(old_root) = world
        .get::<AnimationGraphPlayer>(player_entity)
        .and_then(AnimationGraphPlayer::get_spawned_ragdoll)
        .map(|spawned| spawned.root)
    else {
        return;
    };
    if world
        .get::<RagdollPhysicsProfileApplied>(player_entity)
        .is_none_or(|applied| applied.spawned_root != Some(old_root))
    {
        return;
    }
    let Some(mut player) = world
        .entity_mut(player_entity)
        .take::<AnimationGraphPlayer>()
    else {
        return;
    };
    bevy_als::ragdoll_link::clear_spawned_ragdoll(&mut player);
    world
        .entity_mut(player_entity)
        .insert((player, ManualRagdollPlayerReplacement { old_root }));
    info!(
        "[MANUAL-INTEGRATION][RAGDOLL-PLAYER-REPLACEMENT] player={player_entity:?} old_root={old_root:?} -> PASS"
    );
}

fn actor_montage_clock_passed(rows: &[serde_json::Value], label: &str) -> bool {
    let rows: Vec<_> = rows.iter().filter(|row| row[label] == true).collect();
    rows.len() >= 8
        && rows.iter().all(|row| row["dilation"].as_f64() == Some(0.5))
        && rows.windows(2).all(|pair| {
            let values = [
                pair[0]["game_time"].as_f64(),
                pair[1]["game_time"].as_f64(),
                pair[0]["elapsed"].as_f64(),
                pair[1]["elapsed"].as_f64(),
            ];
            matches!(values, [Some(a), Some(b), Some(c), Some(d)]
                if b > a && ((d - c) - (b - a) * 0.5).abs() < 1e-5)
        })
}

fn grounded_roll_exit_timed_out(elapsed: f32, completed: bool) -> bool {
    !completed && elapsed >= 20.0
}

fn animation_clock_results(rows: &[serde_json::Value]) -> Vec<(&'static str, bool)> {
    let frozen = |start: f64, end: f64| {
        let rows = rows
            .iter()
            .filter(|row| {
                row["elapsed"]
                    .as_f64()
                    .is_some_and(|time| time >= start && time < end)
            })
            .collect::<Vec<_>>();
        rows.len() >= 2
            && rows.iter().all(|row| {
                row["state"] == rows[0]["state"] && row["state_elapsed"] == rows[0]["state_elapsed"]
            })
    };
    let advancing = |start: f64, end: f64| {
        let rows = rows
            .iter()
            .filter(|row| {
                row["elapsed"]
                    .as_f64()
                    .is_some_and(|time| time >= start && time < end)
            })
            .collect::<Vec<_>>();
        rows.len() >= 2
            && rows.last().unwrap()["state_elapsed"].as_f64()
                > rows.first().unwrap()["state_elapsed"].as_f64()
    };
    let one = rows
        .iter()
        .filter(|row| {
            row["elapsed"]
                .as_f64()
                .is_some_and(|time| time >= 2.4 && time < 3.6)
        })
        .map(|row| &row["state_elapsed"])
        .collect::<std::collections::HashSet<_>>();
    let movement = [0.0, 0.5, 1.0, 2.0].into_iter().all(|dilation| {
        let samples = rows
            .iter()
            .filter(|row| {
                row["dilation"].as_f64() == Some(dilation)
                    && row["movement_clock_error"].as_f64().is_some()
            })
            .collect::<Vec<_>>();
        samples.len() >= 3
            && samples.iter().all(|row| {
                row["movement_clock_error"]
                    .as_f64()
                    .is_some_and(|error| error < 0.002)
            })
    });
    vec![
        ("clock_paused_transition", frozen(2.4, 2.9)),
        ("clock_one_frame", one.len() == 2),
        ("clock_repaused", frozen(3.3, 3.7)),
        ("clock_custom_dilation", advancing(3.85, 4.25)),
        ("clock_resumed", advancing(4.6, 5.8)),
        ("clock_actor_movement", movement),
    ]
}

fn observe_animation_clock(
    time: Res<Time>,
    state: Res<ManualIntegrationState>,
    mut report: ResMut<ManualIntegrationGateReport>,
    instances: Query<&AnimatedSceneInstance>,
    players: Query<&AnimationGraphPlayer>,
    assets: Res<AssetServer>,
    graphs: Res<Assets<AnimationGraph>>,
    characters: Query<(
        &Position,
        &bevy_als::als::motion::AlsActualVelocity,
        Option<&bevy_als::als_camera::AlsCameraCustomTimeDilation>,
    )>,
) {
    if state.scenario != IntegrationScenario::AnimationClock
        || state.phase != IntegrationPhase::Notify
    {
        return;
    }
    let Some(avatar) = state.avatar else {
        return;
    };
    let Ok(instance) = instances.get(avatar) else {
        return;
    };
    let Ok(player) = players.get(instance.player_entity()) else {
        return;
    };
    let Some(arena) = player.get_context_arena() else {
        return;
    };
    for id in arena.iter_context_ids() {
        let Some(context) = arena.get_context(id) else {
            continue;
        };
        if assets.get_path(context.get_graph_id()).is_none_or(|path| {
            path.path()
                != std::path::Path::new(
                    "ALS/Character/AnimationInstances/AB_Als_Grounded/AnimGraph.animgraph.ron",
                )
        }) {
            continue;
        }
        let Some(graph) = graphs.get(context.get_graph_id()) else {
            continue;
        };
        let Some(node) = graph
            .nodes
            .values()
            .find(|node| node.name == "Grounded States\nState Machine")
        else {
            continue;
        };
        let Some(fsm) = context
            .node_states
            .get_all_upcoming_states::<bevy_als::extensions::InterruptibleFsmState>(node.id)
            .ok()
            .and_then(|mut states| states.find(|state| state.last_output_pose().is_some()))
        else {
            continue;
        };
        let elapsed = state.phase_elapsed();
        let bucket = (elapsed * 10.0) as u32;
        let current = fsm.current_state_elapsed_time();
        let Some(character) = state.character else {
            return;
        };
        let Ok((position, velocity, dilation)) = characters.get(character) else {
            return;
        };
        let dilation = dilation.map_or(1.0, |value| value.0);
        let movement_error = report
            .direction_trace
            .last()
            .and_then(|row| row["position"].as_array())
            .map(|previous| {
                let previous = Vec2::new(
                    previous[0].as_f64().unwrap() as f32,
                    previous[1].as_f64().unwrap() as f32,
                );
                (position.0.xz()
                    - previous
                    - velocity.world_velocity.xz() * time.delta_secs() * dilation)
                    .length()
            });
        report.direction_trace.push(serde_json::json!({"bucket":bucket,"elapsed":elapsed,"state":format!("{:?}",fsm.state),"state_elapsed":current,"paused":player.is_paused(),
            "position":[position.x,position.z],"dilation":dilation,"movement_clock_error":movement_error}));
        info!(
            "[MANUAL-INTEGRATION][ANIMATION-CLOCK] elapsed={elapsed:.3} state_elapsed={current:.6} paused={}",
            player.is_paused()
        );
    }
}

fn locomotion_crouch_pressed(elapsed: f64, delta: f64) -> bool {
    [7.0, 8.5]
        .into_iter()
        .any(|boundary| elapsed >= boundary && elapsed - delta < boundary)
}

fn locomotion_mouse_delta(elapsed: f32, delta: f32) -> Vec2 {
    let overlap = (elapsed - 0.5).clamp(0.0, 1.0) - (elapsed - delta - 0.5).clamp(0.0, 1.0);
    Vec2::new(360.0, -180.0) * overlap
}

fn visibility_case_phase(elapsed: f32) -> &'static str {
    if elapsed < 2.0 {
        "visible"
    } else if elapsed < 4.5 {
        "hidden"
    } else if elapsed < 6.5 {
        "resume"
    } else if elapsed < 9.5 {
        "montage"
    } else if elapsed < 11.5 {
        "always_pose"
    } else if elapsed < 13.5 {
        "visible_final"
    } else {
        "finish"
    }
}

fn visibility_case_results(rows: &[serde_json::Value]) -> Vec<(&'static str, bool)> {
    let stage = |name: &'static str, minimum: f64| {
        rows.iter().filter(move |row| {
            row["visibility_phase"] == name
                && row["elapsed"].as_f64().is_some_and(|time| time >= minimum)
        })
    };
    let frozen = |name: &'static str, minimum: f64| {
        let values = stage(name, minimum).collect::<Vec<_>>();
        values.len() >= 2
            && values.iter().all(|row| {
                row["update_graph"] == false
                    && row["refresh_bones"] == false
                    && row["frame"] == values[0]["frame"]
            })
    };
    let advancing = |name: &'static str, minimum: f64, bones: bool| {
        let values = stage(name, minimum).collect::<Vec<_>>();
        values.len() >= 2
            && values
                .iter()
                .all(|row| row["update_graph"] == true && row["refresh_bones"] == bones)
            && values.last().unwrap()["frame"].as_u64() > values.first().unwrap()["frame"].as_u64()
    };
    vec![
        ("visibility_initial", advancing("visible", 0.5, true)),
        ("visibility_graph_stopped", frozen("hidden", 3.5)),
        (
            "visibility_pending_resume",
            stage("resume", 4.5).any(|row| row["pending"] == true && row["update_graph"] == true),
        ),
        ("visibility_montage_graph_stopped", frozen("montage", 8.0)),
        (
            "visibility_montage_clock",
            stage("montage", 8.0)
                .any(|row| row["roll_time"].as_f64().is_some_and(|time| time > 0.9)),
        ),
        (
            "visibility_montage_notifications",
            stage("montage", 0.0).all(|row| row["shakes"] == 0 && row["footsteps"] == 0)
                && stage("montage", 8.0).any(|row| {
                    row["update_graph"] == false
                        && row["roll_action"] == true
                        && row["roll_slot_relevant"] == false
                })
                && stage("montage", 8.0).any(|row| {
                    row["update_graph"] == false
                        && row["roll_action"] == false
                        && row["roll_time"].as_f64().is_some_and(|time| time > 0.9)
                }),
        ),
        (
            "visibility_always_pose",
            advancing("always_pose", 9.6, false),
        ),
        (
            "visibility_final_resume",
            advancing("visible_final", 12.0, true),
        ),
    ]
}

fn observe_visibility_case(
    state: Res<ManualIntegrationState>,
    mut report: ResMut<ManualIntegrationGateReport>,
    instances: Query<&AnimatedSceneInstance>,
    players: Query<(
        &AnimationGraphPlayer,
        &bevy_als::extensions::post_physics_animation::AnimationPoseUpdateGate,
    )>,
    rolls: Query<&AlsRollState>,
    actions: Query<&AlsNotifyLocomotionAction>,
    mut shakes: MessageReader<AlsCameraShakeRequest>,
    mut events: MessageReader<AlsAnimationEvent>,
) {
    if state.scenario != IntegrationScenario::Visibility || state.phase != IntegrationPhase::Notify
    {
        return;
    }
    let Some((character, avatar)) = state.character.zip(state.avatar) else {
        return;
    };
    let Ok(instance) = instances.get(avatar) else {
        return;
    };
    let Ok((player, gate)) = players.get(instance.player_entity()) else {
        return;
    };
    let Some(arena) = player.get_context_arena() else {
        return;
    };
    let elapsed = state.phase_elapsed();
    let phase = visibility_case_phase(elapsed);
    let pending = player
        .get_global_input_data("animation.pending_update")
        .and_then(|value| value.as_bool().ok())
        .unwrap_or(false);
    let shakes = shakes
        .read()
        .filter(|event| event.target == character)
        .count();
    let footsteps = events
        .read()
        .filter(|event| {
            event.character == character && event.class_name == "AlsAnimNotify_FootstepEffects"
        })
        .count();
    let roll_action = actions
        .get(character)
        .is_ok_and(|action| *action == AlsNotifyLocomotionAction::Rolling);
    let roll_slot_relevant = bevy_als::extensions::source_slot::relevant_notify_slots(arena)
        .any(|(slot, _)| slot == "PostLocomotion");
    let bucket = (elapsed * 4.0) as u32;
    if shakes == 0
        && footsteps == 0
        && report.direction_trace.last().is_some_and(|row| {
            row["bucket"] == bucket
                && row["pending"] == pending
                && row["update_graph"] == gate.update_graph
                && row["roll_action"] == roll_action
                && row["roll_slot_relevant"] == roll_slot_relevant
        })
    {
        return;
    }
    report.direction_trace.push(serde_json::json!({"visibility_phase": phase, "bucket": bucket, "elapsed": elapsed, "frame": arena.frame_index(),
        "pending": pending, "update_graph": gate.update_graph, "refresh_bones": gate.refresh_bones,
        "roll_time": rolls.get(character).map_or(0.0, |roll| roll.time), "roll_action":roll_action,
        "roll_slot_relevant":roll_slot_relevant, "shakes": shakes, "footsteps": footsteps}));
    info!(
        "[MANUAL-INTEGRATION][VISIBILITY] phase={phase} elapsed={elapsed:.3} frame={} graph={} bones={} pending={pending}",
        arena.frame_index(),
        gate.update_graph,
        gate.refresh_bones
    );
}

fn roll_tail_reentry_ready(
    roll: Option<&AlsRollState>,
    locomotion: LocomotionState,
    requested: bool,
) -> bool {
    !requested
        && locomotion == LocomotionState::Grounded
        && roll.is_some_and(|roll| {
            roll.active
                && roll.time
                    >= bevy_als::als::roll::ROLL_DURATION_SECS - bevy_als::als::roll::ROLL_WEIGHT_OUT_SECS
                        + 0.04
                && roll.weight() > 0.0
        })
}

fn drive_manual_integration(
    mut commands: Commands,
    fixture: Option<Res<ManualIntegrationFixture>>,
    mut state: ResMut<ManualIntegrationState>,
    mut animation_window: ResMut<ManualAnimationWindowEvidence>,
    mut phase_evidence: ResMut<ManualRuntimePhaseEvidence>,
    mut virtual_time: ResMut<Time<Virtual>>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut show_debug: ResMut<AlsShowDebugHudState>,
    mut shake_requests: MessageWriter<AlsCameraShakeRequest>,
    mut pivot_notices: MessageWriter<AlsPivotNotify>,
    mut foot_ik_weight_override: ResMut<AlsFootIkWeightOverride>,
    (params_assets, get_ups, visual_progress): (
        Res<Assets<AlsMovementParams>>,
        Query<(
            &bevy_als::get_up::AlsGetUpState,
            Has<bevy_als::als::motion::AlsMovementInputBlocked>,
        )>,
        Option<Res<AnimationVisualValidationProgress>>,
    ),
    mut characters: Query<
        (
            &mut AlsInputOverride,
            &mut Transform,
            &mut Position,
            Option<&mut Rotation>,
            &mut LinearVelocity,
            &mut AlsStateRequest,
            &Stance,
            &LocomotionState,
            &mut Gait,
            &mut OverlayState,
            &mut RotationMode,
            (&mut AlsViewYaw, Option<&bevy_als::als::view::AlsPlayerViewYaw>),
            &mut AlsViewPitch,
            Option<&AlsRagdollGroundFollow>,
            Option<&AlsStandingGraphTelemetry>,
        ),
        (With<AlsCharacter>, Without<ManualIntegrationPlatform>),
    >,
    mut crouch_state: Query<
        (
            &AlsMovementParamsHandle,
            &AlsCrouchTransaction,
            Option<&mut AlsRotationResponseState>,
            &mut AlsAimingYawConstraint,
            Option<&mut AlsRotationAnimationState>,
            Option<&AlsCrouchingGraphTelemetry>,
            Option<&AlsRollState>,
            Option<&AlsMantleState>,
            Option<&AlsFootLockState>,
        ),
        With<AlsCharacter>,
    >,
    mut platforms: Query<
        (&mut Transform, Option<&mut Position>, &mut LinearVelocity),
        (With<ManualIntegrationPlatform>, Without<AlsCharacter>),
    >,
    (cameras, parents): (
        Query<&AlsCameraDebugInfo, With<AlsCameraRig>>,
        Query<&ChildOf>,
    ),
) {
    if !state.initialized || state.final_logged {
        return;
    }
    state.game_time_seconds = virtual_time.elapsed_secs_f64();
    phase_evidence.scenario = format!("{:?}", state.scenario);
    phase_evidence.phase = format!("{:?}", state.phase);
    phase_evidence.elapsed_seconds = state.phase_elapsed();
    phase_evidence.temporal_rebase_epoch = state.temporal_rebase_epoch;
    let Some(fixture) = fixture else {
        return;
    };
    if Instant::now() >= state.deadline {
        cleanup_crouch_canopy(&mut commands, &mut state);
        cleanup_foot_ik_terrain(&mut commands, &mut state);
        state.timed_out = true;
        virtual_time.set_relative_speed(1.0);
        begin_phase(&mut state, IntegrationPhase::Finish);
        return;
    }
    let Some(character) = state.character else {
        return;
    };
    let Ok((
        mut input,
        mut transform,
        mut position,
        mut rotation,
        mut velocity,
        mut request,
        stance,
        locomotion,
        mut gait,
        mut overlay,
        mut rotation_mode,
        (mut view_yaw, player_view),
        mut view_pitch,
        ground_follow,
        standing_graph,
    )) = characters.get_mut(character)
    else {
        return;
    };
    if state.scenario == IntegrationScenario::Base {
        show_debug.visible = true;
        show_debug.traces = true;
    }
    let Ok((
        params_handle,
        transaction,
        rotation_response,
        mut aiming_yaw_constraint,
        rotation_animation,
        crouching_graph,
        roll_for_input,
        mantle_for_input,
        foot_lock,
    )) = crouch_state.get_mut(character)
    else {
        return;
    };
    let elapsed = state.phase_elapsed();
    let crouching_actions = state.scenario == IntegrationScenario::CrouchingActions;
    let (action_state, action_transition, action_pose_ready) =
        action_graph_state(crouching_actions, standing_graph, crouching_graph);
    let action_stance = if crouching_actions {
        Stance::Crouching
    } else {
        Stance::Standing
    };
    let entering = std::mem::take(&mut state.entered);
    phase_evidence.locomotion = format!("{:?}", locomotion);
    phase_evidence.gait = format!("{:?}", gait);
    phase_evidence.stance = format!("{:?}", stance);
    phase_evidence.rotation_mode = format!("{:?}", rotation_mode);
    input.0.reset();
    if !matches!(
        state.phase,
        IntegrationPhase::DirectionRegression | IntegrationPhase::CrouchDirectionRegression
    ) {
        *animation_window = ManualAnimationWindowEvidence::default();
    }

    match state.phase {
        IntegrationPhase::Await
        | IntegrationPhase::Stairs
        | IntegrationPhase::UphillJump
        | IntegrationPhase::MovingBase => {}
        IntegrationPhase::LocomotionActions => {
            let index = state.locomotion_probe.case_index;
            if entering {
                let covered = state.locomotion_probe.covered;
                let canonical_seen = state.locomotion_probe.canonical_seen;
                let window_seconds = state.locomotion_probe.window_seconds;
                let valid = index == 0 || state.locomotion_probe.valid;
                state.locomotion_probe = LocomotionActionProbe {
                    case_index: index,
                    covered,
                    canonical_seen,
                    window_seconds,
                    triggered: index == 6,
                    valid,
                    ..default()
                };
                if let Ok((mut platform, platform_position, mut platform_velocity)) =
                    platforms.get_mut(fixture.platform)
                {
                    platform.translation = if matches!(index, 4 | 5) {
                        GROUNDED_START.with_y(LOCOMOTION_DROP_HEIGHT)
                    } else {
                        fixture.platform_home
                    };
                    if let Some(mut position) = platform_position {
                        position.0 = platform.translation;
                    }
                    platform_velocity.0 = Vec3::ZERO;
                }
                let start = if matches!(index, 4 | 5) {
                    GROUNDED_START
                        .with_y(LOCOMOTION_DROP_HEIGHT + 0.25 + fixture.locomotion_start.y)
                } else {
                    GROUNDED_START.with_y(fixture.locomotion_start.y)
                        + if index == 6 {
                            Vec3::Y
                                * (6.2 * 6.2
                                    / (2.0
                                        * 9.81
                                        * params_assets
                                            .get(params_handle.id())
                                            .unwrap()
                                            .jump
                                            .gravity_scale))
                        } else {
                            Vec3::ZERO
                        }
                };
                reset_character(
                    &mut transform,
                    &mut position,
                    rotation,
                    &mut velocity,
                    &mut view_yaw,
                    &mut view_pitch,
                    start,
                    SPRINT_YAW,
                );
                reinitialize_fixture_rotation_response(
                    rotation_response,
                    &mut aiming_yaw_constraint,
                    rotation_animation,
                );
                rebase_temporal_fixture(&mut state, &mut phase_evidence, "locomotion_actions");
                request.stance = Some(Stance::Standing);
                *overlay = OverlayState::Default;
                *rotation_mode = RotationMode::ViewDirection;
            }
            request.gait = Some(if matches!(index, 2..=4) {
                Gait::Running
            } else {
                Gait::Walking
            });
            if index < 4 {
                let jumped = state.locomotion_probe.triggered;
                input.0.move_dir =
                    if !jumped || index >= 2 || !state.locomotion_probe.covered[index] {
                        Vec2::Y
                    } else {
                        Vec2::ZERO
                    };
                if !jumped
                    && locomotion_jump_ready(
                        index,
                        state.locomotion_probe.sample.outer,
                        Vec2::new(velocity.x, velocity.z).length(),
                        state.locomotion_probe.sample.foot_planted,
                    )
                {
                    input.0.jump_pressed = true;
                    state.locomotion_probe.triggered = true;
                }
            } else if index <= 5 {
                let ready = state
                    .locomotion_probe
                    .ready_since
                    .is_some_and(|since| state.game_time_seconds - since >= 0.3);
                if !state.locomotion_probe.triggered
                    && ready
                    && (index == 5 || Vec2::new(velocity.x, velocity.z).length() >= 3.6)
                {
                    state.locomotion_probe.triggered = true;
                    input.0.jump_pressed = index == 4;
                }
                if (ready || state.locomotion_probe.triggered)
                    && transform.translation.x < GROUNDED_START.x + 4.0
                {
                    input.0.move_dir = Vec2::Y;
                }
            }
            if state.locomotion_probe.completed {
                if index < 6 {
                    state.locomotion_probe.case_index += 1;
                    begin_phase(&mut state, IntegrationPhase::LocomotionActions);
                } else {
                    begin_phase(&mut state, IntegrationPhase::Finish);
                }
            } else if locomotion_actions_should_timeout(&state.locomotion_probe, elapsed) {
                state.timed_out = true;
                begin_phase(&mut state, IntegrationPhase::Finish);
            }
        }
        IntegrationPhase::StandingResponse => {
            if entering {
                state.coverage.response_idle = true;
                reinitialize_fixture_rotation_response(
                    rotation_response,
                    &mut aiming_yaw_constraint,
                    rotation_animation,
                );
                reset_character(
                    &mut transform,
                    &mut position,
                    rotation,
                    &mut velocity,
                    &mut view_yaw,
                    &mut view_pitch,
                    fixture.locomotion_start,
                    fixture.facing_yaw,
                );
                rebase_temporal_fixture(&mut state, &mut phase_evidence, "standing_response");
                *overlay = OverlayState::Default;
            }
            let (label, direction) = standing_response_step(elapsed);
            *rotation_mode = RotationMode::ViewDirection;
            request.locomotion_state = Some(LocomotionState::Grounded);
            request.stance = Some(Stance::Standing);
            request.gait = Some(Gait::Running);
            request.rotation_mode = Some(RotationMode::ViewDirection);
            input.0.move_dir = direction;
            *animation_window = ManualAnimationWindowEvidence {
                label: Some(label),
                settled: elapsed > 0.75,
                capture_source_window: true,
                requires_pre_rig_motion: direction != Vec2::ZERO,
                requires_final_pose_motion: direction != Vec2::ZERO,
                requires_non_reference_root_pose: true,
                source_node: Some("AB_Als_Standing/Standing States"),
                source_state: Some("Standing input response"),
                ..default()
            };
            if elapsed >= 12.5 {
                begin_phase(&mut state, IntegrationPhase::Finish);
            }
        }
        IntegrationPhase::StandingTurn => {
            let index = state.turn_case_index;
            if entering {
                state.turn_probe = StandingTurnProbe::default();
                if index == 0 && !crouching_actions {
                    reinitialize_fixture_rotation_response(
                        rotation_response,
                        &mut aiming_yaw_constraint,
                        rotation_animation,
                    );
                    reset_character(
                        &mut transform,
                        &mut position,
                        rotation,
                        &mut velocity,
                        &mut view_yaw,
                        &mut view_pitch,
                        fixture.locomotion_start,
                        fixture.facing_yaw,
                    );
                }
                rebase_temporal_fixture(
                    &mut state,
                    &mut phase_evidence,
                    STANDING_TURN_CASES[index].0,
                );
                *overlay = OverlayState::Default;
            }
            request.locomotion_state = Some(LocomotionState::Grounded);
            request.stance = Some(action_stance);
            request.gait = Some(Gait::Walking);
            if elapsed > 0.5 && state.turn_probe.idle_ready && state.turn_probe.started_at.is_none()
            {
                state.turn_probe.started_at = Some(elapsed);
                state.turn_probe.base_yaw = world_forward_to_yaw(*transform.forward());
            }
            let driven_elapsed = state.turn_probe.started_at.map(|started| elapsed - started);
            input.0 = standing_turn_mouse_command(
                index,
                driven_elapsed,
                state.turn_probe.base_yaw,
                view_yaw.0,
                *rotation_mode,
            );
            if state.coverage.standing_turn[index] {
                state.turn_case_index += 1;
                if crouching_actions && index == 3 {
                    state.stop_case_index = 0;
                    begin_phase(&mut state, IntegrationPhase::StandingDynamic);
                } else {
                    begin_phase(
                        &mut state,
                        if index + 1 == STANDING_TURN_CASES.len() {
                            IntegrationPhase::Finish
                        } else {
                            IntegrationPhase::StandingTurn
                        },
                    );
                }
            } else if elapsed >= if index == 7 { 20.0 } else { 12.0 } {
                state.timed_out = true;
                begin_phase(&mut state, IntegrationPhase::Finish);
            }
        }
        IntegrationPhase::StandingDynamic => {
            let index = state.stop_case_index;
            let (label, side) = STANDING_DYNAMIC_CASES[index];
            if entering {
                state.turn_probe = StandingTurnProbe::default();
                state.stop_playback = None;
                state.stop_branch_seen = false;
                state.dynamic_unlocked = false;
                state.dynamic_selection_valid = true;
                if index == 0 && !crouching_actions {
                    reinitialize_fixture_rotation_response(
                        rotation_response,
                        &mut aiming_yaw_constraint,
                        rotation_animation,
                    );
                    reset_character_with_rotation(
                        &mut transform,
                        &mut position,
                        rotation.as_deref_mut(),
                        &mut velocity,
                        &mut view_yaw,
                        &mut view_pitch,
                        fixture.locomotion_start,
                        fixture.facing_yaw,
                    );
                }
                rebase_temporal_fixture(&mut state, &mut phase_evidence, label);
                *overlay = OverlayState::Default;
            }
            request.locomotion_state = Some(LocomotionState::Grounded);
            request.stance = Some(action_stance);
            request.gait = Some(Gait::Walking);
            if elapsed > 0.5 && state.turn_probe.idle_ready && state.turn_probe.started_at.is_none()
            {
                if crouching_actions {
                    if let Some(pose) = foot_lock
                        .and_then(|feet| crouching_dynamic_fixture_pose(*transform, feet, side))
                    {
                        reset_character(
                            &mut transform,
                            &mut position,
                            rotation,
                            &mut velocity,
                            &mut view_yaw,
                            &mut view_pitch,
                            pose.translation,
                            world_forward_to_yaw(*pose.forward()),
                        );
                        state.turn_probe.started_at = Some(elapsed);
                    }
                } else {
                    let forward = *transform.forward();
                    transform.translation -= forward * 0.1;
                    position.0 = transform.translation;
                    state.turn_probe.started_at = Some(elapsed);
                }
            }
            input.0 = dynamic_idle_command(
                world_forward_to_yaw(*transform.forward()),
                view_yaw.0,
                *rotation_mode,
            );
            if state.coverage.standing_dynamic[index] {
                state.stop_case_index += 1;
                begin_phase(
                    &mut state,
                    if index + 1 == STANDING_DYNAMIC_CASES.len() {
                        IntegrationPhase::Finish
                    } else {
                        IntegrationPhase::StandingDynamic
                    },
                );
            } else if elapsed >= 12.0 {
                state.timed_out = true;
                begin_phase(&mut state, IntegrationPhase::Finish);
            }
        }
        IntegrationPhase::StandingStop => {
            let index = state.stop_case_index;
            let cases = stop_cases(crouching_actions);
            let (label, _, _, mode, _) = cases[index];
            let quick = index >= if crouching_actions { 1 } else { 4 };
            if entering {
                state.generated_stop_quick_input_started = false;
                state.generated_stop_quick_input_released = false;
                state.stop_playback = None;
                state.stop_branch_seen = false;
                if index == 0 {
                    reinitialize_fixture_rotation_response(
                        rotation_response,
                        &mut aiming_yaw_constraint,
                        rotation_animation,
                    );
                    reset_character(
                        &mut transform,
                        &mut position,
                        rotation,
                        &mut velocity,
                        &mut view_yaw,
                        &mut view_pitch,
                        fixture.locomotion_start,
                        fixture.facing_yaw,
                    );
                } else {
                    view_yaw.0 = world_forward_to_yaw(*transform.forward());
                }
                rebase_temporal_fixture(&mut state, &mut phase_evidence, label);
                *overlay = OverlayState::Default;
            }
            request.locomotion_state = Some(LocomotionState::Grounded);
            request.stance = Some(action_stance);
            request.gait = Some(if quick { Gait::Running } else { Gait::Walking });
            if elapsed > 0.5
                && *stance == action_stance
                && *rotation_mode == mode
                && (action_state, action_transition, action_pose_ready)
                    == (Some(0), Some(false), Some(true))
            {
                state.generated_stop_quick_input_started = true;
            }
            if state.generated_stop_quick_input_started && {
                if quick {
                    action_transition == Some(true)
                } else {
                    elapsed > 1.5
                        && action_state == Some(1)
                        && action_transition == Some(false)
                        && (crouching_actions
                            || standing_stop_release_phase(index, state.stop_foot_planted))
                }
            } {
                state.generated_stop_quick_input_released = true;
            }
            input.0 = standing_stop_command(
                index,
                *rotation_mode,
                state.generated_stop_quick_input_started
                    && !state.generated_stop_quick_input_released,
                crouching_actions,
            );
            *animation_window = ManualAnimationWindowEvidence::default();
            if state.coverage.standing_stop[index] {
                state.stop_case_index += 1;
                let next_phase = if index + 1 == cases.len() {
                    if crouching_actions {
                        state.turn_case_index = 2;
                        IntegrationPhase::StandingTurn
                    } else {
                        IntegrationPhase::Finish
                    }
                } else {
                    IntegrationPhase::StandingStop
                };
                begin_phase(&mut state, next_phase);
            } else if elapsed >= 7.0 {
                state.timed_out = true;
                begin_phase(&mut state, IntegrationPhase::Finish);
            }
        }
        IntegrationPhase::DirectionRegression => {
            if entering {
                state.direction_case_index = usize::MAX;
                state.coverage.direction_cases = [false; DIRECTION_CASE_COUNT];
                state.coverage.direction_source_states = 0;
                state.foot_ik_baseline_mask = 0;
                state.coverage.foot_ik_space_baseline = false;
                state.coverage.foot_ik_no_hit_recovered = false;
                state.coverage.foot_ik_weight_ramp = false;
                state.foot_ik_weight_mask = 0;
                if state.scenario == IntegrationScenario::StateMachine {
                    state.coverage.p1_ground = false;
                    state.coverage.p1_stance = false;
                    state.coverage.p1_gait = false;
                    state.coverage.p1_air = false;
                    state.coverage.p1_turn = false;
                    state.coverage.generated_transition_stop_quick = false;
                    state.coverage.turn_slot = false;
                    state.coverage.turn_inertialization = false;
                    state.p1_pivot_sent = false;
                    state.p1_ground_mask = 0;
                    state.p1_gait_mask = 0;
                    state.p1_air_mask = 0;
                    state.p1_turn_mask = 0;
                    state.p1_stance_mask = 0;
                }
            }
            let case_index = (elapsed / DIRECTION_CASE_SECONDS) as usize;
            let Some(case) = direction_regression_case(case_index) else {
                *animation_window = ManualAnimationWindowEvidence::default();
                commands.entity(character).remove::<AlsFirstPersonView>();
                if state.scenario == IntegrationScenario::FootIk {
                    begin_phase(&mut state, IntegrationPhase::FootIkNoHit);
                } else if state.scenario == IntegrationScenario::StateMachine {
                    begin_phase(&mut state, IntegrationPhase::StateMachineGrounded);
                } else if state.scenario == IntegrationScenario::StandingDirection {
                    begin_phase(&mut state, IntegrationPhase::Finish);
                } else {
                    begin_phase(&mut state, IntegrationPhase::Locomotion);
                }
                return;
            };
            *animation_window = ManualAnimationWindowEvidence {
                actor_yaw_driver: None,
                label: Some(case.name),
                index: Some(case_index),
                settled: elapsed
                    >= case_index as f32 * DIRECTION_CASE_SECONDS + DIRECTION_CASE_SETTLE_SECONDS,
                capture_source_window: true,
                capture_every_frame: false,
                requires_pre_rig_motion: true,
                requires_final_pose_motion: true,
                requires_non_reference_root_pose: true,
                requires_source_pose_chain: true,
                requires_actor_yaw_motion: false,
                requires_actor_yaw_convergence: false,
                expected_actor_yaw_delta_sign: None,
                expected_actor_yaw_baseline: None,
                expected_animation_variant: None,
                source_node: Some("AB_Als_Standing/Movement States"),
                source_state: case.source_state,
                source_pose_chain: generated_standing_source_pose_chain(),
                minimum_pre_rig_translation_span: 0.0,
                minimum_pre_rig_rotation_span_radians: 0.0,
                minimum_final_translation_span: 0.0,
                minimum_final_rotation_span_radians: 0.0,
                minimum_actor_yaw_delta_radians: 0.0,
                minimum_actor_yaw_convergence_radians: 0.0,
                minimum_observation_seconds: 0.0,
                source_sync_group: case.direction_signature_group.map(|_| "Movement"),
                sync_phase_global: case
                    .direction_signature_group
                    .map(|_| "sync_groups.movement.phase"),
                canonical_sync_phase: case.direction_signature_group.map(|_| 0.5),
                canonical_clip_time: None,
                maximum_clip_time_distance: 0.0,
                maximum_sync_phase_distance: if case.direction_signature_group.is_some() {
                    0.12
                } else {
                    0.0
                },
                direction_signature_group: case.direction_signature_group,
                required_direction_signature_members: if case.direction_signature_group.is_some() {
                    4
                } else {
                    0
                },
                minimum_direction_signature_rotation_radians: if case
                    .direction_signature_group
                    .is_some()
                {
                    0.01
                } else {
                    0.0
                },
                expected_direction: case.expected_movement_state.map(movement_direction_name),
                observed_direction: None,
                expected_runtime_state: case
                    .expected_movement_state
                    .map(movement_direction_fsm_index),
                observed_runtime_state: None,
                expected_parent_runtime_state: None,
                expected_terminal_parent_runtime_state: None,
                expected_terminal_parent_transition_active: None,
                expected_terminal_requested_parent_transition_active: None,
                observed_parent_runtime_state: None,
                observed_parent_transition_active: None,
                observed_requested_parent_runtime_state: None,
                observed_requested_parent_transition_active: None,
                observed_actor_yaw: None,
                observed_view_yaw: None,
                observed_animation_yaw_speed_degrees: None,
                observed_animation_curve_direct_yaw_speed_degrees: None,
                observed_animation_clip_time: None,
                observed_animation_weight: None,
                observed_animation_play_rate: None,
                observed_animation_curve_stack_weight: None,
                observed_animation_curve_stack_entries: Vec::new(),
                observed_animation_variant: None,
                expected_lifecycle_stages: &[],
                observed_lifecycle_stages: &[],
            };
            if state.direction_case_index != case_index {
                let yaw = fixture.facing_yaw + case.yaw_offset;
                let start = if case.yaw_offset.abs() > f32::EPSILON {
                    Vec3::new(-19.0, fixture.locomotion_start.y, -20.0)
                } else {
                    fixture.locomotion_start
                };
                if entering || case.yaw_offset.abs() > f32::EPSILON {
                    reset_character_without_rotation(
                        &mut transform,
                        &mut position,
                        &mut velocity,
                        &mut view_yaw,
                        &mut view_pitch,
                        start,
                        yaw,
                    );
                    rebase_temporal_fixture(&mut state, &mut phase_evidence, "direction_fixture");
                }
                *overlay = OverlayState::Default;
                request.locomotion_state = Some(LocomotionState::Grounded);
                request.stance = Some(Stance::Standing);
                request.gait = Some(case.desired_gait);
                request.rotation_mode = Some(case.rotation_mode);
                if case.first_person {
                    commands.entity(character).insert(AlsFirstPersonView);
                } else {
                    commands.entity(character).remove::<AlsFirstPersonView>();
                }
                state.direction_case_index = case_index;
                info!(
                    "[MANUAL-INTEGRATION][DIRECTION][START] case={} input=({:.3},{:.3}) desired={:?} first_person={} yaw_offset={:.1}",
                    case.name,
                    case.move_dir.x,
                    case.move_dir.y,
                    case.desired_gait,
                    case.first_person,
                    case.yaw_offset.to_degrees(),
                );
            }
            input.0 = case.command();
            *rotation_mode = case.rotation_mode;
            request.locomotion_state = Some(LocomotionState::Grounded);
            request.stance = Some(Stance::Standing);
            request.gait = Some(case.desired_gait);
            request.rotation_mode = Some(case.rotation_mode);
        }
        IntegrationPhase::Sprint => {
            if entering {
                reinitialize_fixture_rotation_response(
                    rotation_response,
                    &mut aiming_yaw_constraint,
                    rotation_animation,
                );
                reset_character(
                    &mut transform,
                    &mut position,
                    rotation,
                    &mut velocity,
                    &mut view_yaw,
                    &mut view_pitch,
                    SPRINT_START.with_y(fixture.locomotion_start.y),
                    SPRINT_YAW,
                );
                rebase_temporal_fixture(&mut state, &mut phase_evidence, "sprint_fixture");
                *gait = Gait::Walking;
                *overlay = OverlayState::Default;
                *rotation_mode = RotationMode::VelocityDirection;
                request.locomotion_state = Some(LocomotionState::Grounded);
                request.stance = Some(Stance::Standing);
                request.gait = Some(Gait::Walking);
                request.rotation_mode = Some(RotationMode::VelocityDirection);
                state.coverage.sprint_curve_relay = false;
                state.sprint_signature_phase = None;
                state.sprint_samples = [None; 2];
                info!("[MANUAL-INTEGRATION][SPRINT] source-curve relay probe started");
            }
            let active = elapsed >= 1.5;
            input.0.move_dir = if active { Vec2::Y } else { Vec2::ZERO };
            input.0.sprint_held = active;
            request.locomotion_state = Some(LocomotionState::Grounded);
            request.stance = Some(Stance::Standing);
            request.gait = Some(if active {
                Gait::Sprinting
            } else {
                Gait::Walking
            });
            request.rotation_mode = Some(RotationMode::VelocityDirection);
            if elapsed >= 10.0 {
                begin_phase(&mut state, IntegrationPhase::Finish);
            }
        }
        IntegrationPhase::FootIkNoHit => {
            if entering {
                state.foot_ik_no_hit_logged = false;
                reset_character_without_rotation(
                    &mut transform,
                    &mut position,
                    &mut velocity,
                    &mut view_yaw,
                    &mut view_pitch,
                    fixture.locomotion_start,
                    fixture.facing_yaw,
                );
                *gait = Gait::Running;
                *overlay = OverlayState::Default;
                *rotation_mode = RotationMode::ViewDirection;
                request.locomotion_state = Some(LocomotionState::Grounded);
                request.stance = Some(Stance::Standing);
                request.gait = Some(Gait::Running);
                request.rotation_mode = Some(RotationMode::ViewDirection);
                commands.entity(character).insert(AlsMantleControlSettings {
                    allow_mantling: false,
                    auto_start_in_air: false,
                    ..default()
                });
            }
            input.0.jump_pressed = (0.5..0.7).contains(&elapsed);
            input.0.jump_held = (0.5..1.0).contains(&elapsed);
            if elapsed >= 6.0 {
                commands
                    .entity(character)
                    .remove::<AlsMantleControlSettings>();
                begin_phase(&mut state, IntegrationPhase::FootIkWeightRamp);
            }
        }
        IntegrationPhase::FootIkWeightRamp => {
            if entering {
                reset_character(
                    &mut transform,
                    &mut position,
                    rotation,
                    &mut velocity,
                    &mut view_yaw,
                    &mut view_pitch,
                    fixture.locomotion_start,
                    fixture.facing_yaw,
                );
                *gait = Gait::Running;
                *overlay = OverlayState::Default;
                *rotation_mode = RotationMode::ViewDirection;
                request.locomotion_state = Some(LocomotionState::Grounded);
                request.stance = Some(Stance::Standing);
                request.gait = Some(Gait::Running);
                request.rotation_mode = Some(RotationMode::ViewDirection);
            }
            let target_weight = if elapsed < 0.75 {
                0.0
            } else if elapsed < 1.5 {
                0.25
            } else if elapsed < 2.25 {
                0.5
            } else if elapsed < 3.0 {
                0.75
            } else if elapsed < 3.75 {
                1.0
            } else {
                0.0
            };
            foot_ik_weight_override.left = Some(target_weight);
            foot_ik_weight_override.right = Some(target_weight);
            if elapsed >= 4.75 {
                foot_ik_weight_override.left = None;
                foot_ik_weight_override.right = None;
                begin_phase(&mut state, IntegrationPhase::FootIkTeleport);
            }
        }
        IntegrationPhase::FootIkTeleport => {
            if entering {
                reset_character(
                    &mut transform,
                    &mut position,
                    rotation,
                    &mut velocity,
                    &mut view_yaw,
                    &mut view_pitch,
                    fixture.locomotion_start,
                    fixture.facing_yaw,
                );
                *gait = Gait::Running;
                *overlay = OverlayState::Default;
                *rotation_mode = RotationMode::ViewDirection;
                request.locomotion_state = Some(LocomotionState::Grounded);
                request.stance = Some(Stance::Standing);
                request.gait = Some(Gait::Running);
                request.rotation_mode = Some(RotationMode::ViewDirection);
                state.foot_ik_teleport_requested = false;
                state.foot_ik_teleport_baseline = None;
                state.coverage.foot_ik_teleport = false;
            }
            if elapsed >= 1.0 && !state.foot_ik_teleport_requested {
                let right = Quat::from_rotation_y(fixture.facing_yaw) * Vec3::X;
                let destination = transform.translation + right * 0.6;
                reset_character_without_rotation(
                    &mut transform,
                    &mut position,
                    &mut velocity,
                    &mut view_yaw,
                    &mut view_pitch,
                    destination,
                    fixture.facing_yaw,
                );
                state.foot_ik_teleport_requested = true;
                info!(
                    "[MANUAL-INTEGRATION][FOOT-IK-TELEPORT][REQUEST] distance_m=0.600 destination=({:.3},{:.3},{:.3})",
                    destination.x, destination.y, destination.z,
                );
            }
            if elapsed >= 3.0 {
                begin_phase(&mut state, IntegrationPhase::FootIkTerrain);
            }
        }
        IntegrationPhase::FootIkTerrain => {
            if entering {
                cleanup_foot_ik_terrain(&mut commands, &mut state);
                reset_character(
                    &mut transform,
                    &mut position,
                    rotation,
                    &mut velocity,
                    &mut view_yaw,
                    &mut view_pitch,
                    fixture.locomotion_start,
                    fixture.facing_yaw,
                );
                *gait = Gait::Walking;
                *overlay = OverlayState::Default;
                *rotation_mode = RotationMode::ViewDirection;
                request.locomotion_state = Some(LocomotionState::Grounded);
                request.stance = Some(Stance::Standing);
                request.gait = Some(Gait::Walking);
                request.rotation_mode = Some(RotationMode::ViewDirection);
                state.foot_ik_terrain_air_seen = false;
                state.coverage.foot_ik_slope = false;
                state.coverage.foot_ik_step = false;
                state.coverage.foot_ik_crouching = false;
                state.coverage.foot_ik_jump_air = false;
                state.coverage.foot_ik_landed = false;
                state.coverage.foot_ik_roll = false;
                state.coverage.foot_ik_terrain_cleaned = false;
                let forward = Quat::from_rotation_y(fixture.facing_yaw) * Vec3::NEG_Z;
                let ground_y = fixture.locomotion_start.y - 0.92;
                let slope = commands
                    .spawn((
                        Name::new("manual_foot_ik_slope"),
                        ManualFootIkTerrain,
                        RigidBody::Static,
                        CollisionLayers::new(
                            [AlsCollisionLayer::Default, AlsCollisionLayer::Visibility],
                            avian3d::prelude::LayerMask::ALL,
                        ),
                        Collider::cuboid(3.0, 0.12, 2.5),
                        Transform::from_translation(
                            fixture.locomotion_start
                                + forward * 0.8
                                + Vec3::Y * (ground_y + 0.20 - fixture.locomotion_start.y),
                        )
                        .with_rotation(Quat::from_rotation_x(-18.0_f32.to_radians()))
                        .with_scale(Vec3::new(3.75, 0.18, 3.125)),
                        Mesh3d(fixture.obstacle_mesh.clone()),
                        MeshMaterial3d(fixture.obstacle_material.clone()),
                    ))
                    .id();
                let step = commands
                    .spawn((
                        Name::new("manual_foot_ik_step"),
                        ManualFootIkTerrain,
                        RigidBody::Static,
                        CollisionLayers::new(
                            [AlsCollisionLayer::Default, AlsCollisionLayer::Visibility],
                            avian3d::prelude::LayerMask::ALL,
                        ),
                        Collider::cuboid(1.6, 0.20, 1.5),
                        Transform::from_translation(
                            fixture.locomotion_start
                                + forward * 3.5
                                + Vec3::Y * (ground_y + 0.20 - fixture.locomotion_start.y),
                        )
                        .with_scale(Vec3::new(2.0, 0.25, 1.875)),
                        Mesh3d(fixture.obstacle_mesh.clone()),
                        MeshMaterial3d(fixture.obstacle_material.clone()),
                    ))
                    .id();
                state.foot_ik_slope = Some(slope);
                state.foot_ik_step = Some(step);
                commands.entity(character).insert(AlsMantleControlSettings {
                    allow_mantling: false,
                    auto_start_in_air: false,
                    ..default()
                });
                info!(
                    "[MANUAL-INTEGRATION][FOOT-IK-TERRAIN] slope={slope:?} step={step:?} slope_center=({:.3},{:.3},{:.3}) step_center=({:.3},{:.3},{:.3})",
                    fixture.locomotion_start.x + forward.x * 0.8,
                    ground_y + 0.20,
                    fixture.locomotion_start.z + forward.z * 0.8,
                    fixture.locomotion_start.x + forward.x * 3.5,
                    ground_y + 0.20,
                    fixture.locomotion_start.z + forward.z * 3.5,
                );
            }
            input.0 = foot_ik_terrain_command(elapsed, virtual_time.delta_secs());
            if elapsed >= 10.0 {
                commands
                    .entity(character)
                    .remove::<AlsMantleControlSettings>();
                cleanup_foot_ik_terrain(&mut commands, &mut state);
                begin_phase(&mut state, IntegrationPhase::Mantle);
            }
        }
        IntegrationPhase::ControlRig => {
            if entering {
                reset_character(
                    &mut transform,
                    &mut position,
                    rotation,
                    &mut velocity,
                    &mut view_yaw,
                    &mut view_pitch,
                    fixture.locomotion_start,
                    fixture.facing_yaw,
                );
                *gait = Gait::Running;
                *overlay = OverlayState::Default;
                *rotation_mode = RotationMode::Aiming;
                request.locomotion_state = Some(LocomotionState::Grounded);
                request.stance = Some(Stance::Standing);
                request.rotation_mode = Some(RotationMode::Aiming);
                request.overlay_state = Some(OverlayState::Default);
            }
            input.0.move_dir = Vec2::new(0.65, 1.0).normalize();
            input.0.aim_held = true;
            view_yaw.0 = fixture.facing_yaw + 35.0_f32.to_radians();
            request.rotation_mode = Some(RotationMode::Aiming);
            request.overlay_state = Some(OverlayState::Default);
            if elapsed >= 6.0 {
                begin_phase(&mut state, IntegrationPhase::Finish);
            }
        }
        IntegrationPhase::Head => {
            let index = state.head_probe.case_index;
            if entering && index == 0 {
                state.head_probe = HeadProbe {
                    valid: true,
                    ..default()
                };
                reset_character(
                    &mut transform,
                    &mut position,
                    rotation,
                    &mut velocity,
                    &mut view_yaw,
                    &mut view_pitch,
                    GROUNDED_START.with_y(fixture.locomotion_start.y),
                    SPRINT_YAW,
                );
                let parent_transform = Transform::from_xyz(5.0, 2.0, -3.0)
                    .with_rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2))
                    .with_scale(Vec3::splat(1.5));
                let parent = commands
                    .spawn((Name::new("Head actor parent fixture"), parent_transform))
                    .id();
                *transform = GlobalTransform::from(*transform)
                    .reparented_to(&GlobalTransform::from(parent_transform));
                commands.entity(character).insert(ChildOf(parent));
                reinitialize_fixture_rotation_response(
                    rotation_response,
                    &mut aiming_yaw_constraint,
                    rotation_animation,
                );
                rebase_temporal_fixture(&mut state, &mut phase_evidence, "head_start");
                *overlay = OverlayState::Default;
                *rotation_mode = RotationMode::ViewDirection;
                commands.entity(character).remove::<AlsFirstPersonView>();
                commands
                    .entity(character)
                    .remove::<bevy_als::als_camera::AlsCameraCustomTimeDilation>();
            }
            request.overlay_state = Some(OverlayState::Default);
            let desired_mode = if matches!(index, 3 | 4) {
                RotationMode::VelocityDirection
            } else {
                RotationMode::ViewDirection
            };
            input.0.rotation_mode_pressed = entering && *rotation_mode != desired_mode;
            if entering && index == 2 {
                commands
                    .entity(character)
                    .insert(bevy_als::als::aim::AlsLastFreeRotationMode(
                        RotationMode::Aiming,
                    ));
                input.0.rotation_mode_pressed = false;
            }
            request.stance = Some(Stance::Standing);
            request.gait = Some(Gait::Walking);
            view_yaw.0 = SPRINT_YAW + (elapsed * 2.0).sin() * 55.0_f32.to_radians();
            view_pitch.0 = match index {
                0 => -60.0,
                1 => 0.0,
                2 => 60.0,
                5 | 6 => (elapsed * 3.0).sin() * 60.0,
                _ => 0.0_f32,
            }
            .to_radians();
            if index == 3 {
                input.0.move_dir = Vec2::new((elapsed * 3.0).sin() * 0.8, 1.0).normalize();
            }
            if entering && matches!(index, 5 | 6) {
                input.0.view_mode_pressed = true;
            }
            if entering && index == 5 {
                commands
                    .entity(character)
                    .insert(bevy_als::als_camera::AlsCameraCustomTimeDilation(0.5));
            }
            if entering && index == 6 {
                commands
                    .entity(character)
                    .remove::<bevy_als::als_camera::AlsCameraCustomTimeDilation>();
            }
            virtual_time.set_relative_speed(if index == 5 { 0.25 } else { 1.0 });
            if state.head_probe.covered[index] {
                state.head_probe.case_index += 1;
                if index == 6 {
                    virtual_time.set_relative_speed(1.0);
                }
                begin_phase(
                    &mut state,
                    if index == 6 {
                        IntegrationPhase::Finish
                    } else {
                        IntegrationPhase::Head
                    },
                );
            } else if default_overlay_should_timeout(false, elapsed) {
                state.head_probe.sample.elapsed_seconds = elapsed;
                state.timed_out = true;
                virtual_time.set_relative_speed(1.0);
                begin_phase(&mut state, IntegrationPhase::Finish);
            }
        }
        IntegrationPhase::Footprints => {
            let stage = state.footprint_probe.stage;
            show_debug.visible = true;
            show_debug.traces = true;
            state.footprint_probe.waited = elapsed;
            if entering && stage == 0 {
                reset_character(
                    &mut transform,
                    &mut position,
                    rotation,
                    &mut velocity,
                    &mut view_yaw,
                    &mut view_pitch,
                    GROUNDED_START.with_y(fixture.locomotion_start.y),
                    SPRINT_YAW,
                );
                rebase_temporal_fixture(&mut state, &mut phase_evidence, "footprint_start");
                *gait = Gait::Walking;
                *rotation_mode = RotationMode::ViewDirection;
            }
            request.rotation_mode = Some(RotationMode::ViewDirection);
            request.overlay_state = Some(OverlayState::Default);
            request.gait = Some(if stage == 3 {
                Gait::Running
            } else {
                Gait::Walking
            });
            request.stance = Some(if stage == 5 {
                Stance::Crouching
            } else {
                Stance::Standing
            });
            if matches!(stage, 1 | 3 | 5) {
                input.0.move_dir = Vec2::Y;
            }
            if state.footprint_probe.covered[stage]
                && (stage != 0 || state.footprint_probe.gpu.complete)
            {
                state.footprint_probe.stage += 1;
                let next = if state.footprint_probe.stage == footprints::LABELS.len() {
                    IntegrationPhase::MovingBase
                } else {
                    IntegrationPhase::Footprints
                };
                begin_phase(&mut state, next);
            } else if (stage != 0 || state.footprint_probe.gpu.complete)
                && default_overlay_should_timeout(false, elapsed)
            {
                state.timed_out = true;
                state.footprint_probe.errors.push(format!(
                    "Footprint stage {stage} made no progress for 12 seconds"
                ));
                begin_phase(&mut state, IntegrationPhase::Finish);
            }
        }
        IntegrationPhase::Overlays => {
            let mode = state.overlays_probe.mode;
            let stage = state.overlays_probe.stage;
            state.overlays_probe.waited = elapsed;
            if entering {
                commands
                    .entity(character)
                    .insert(bevy_als::overlay_object::AlsOverlayObjectConfig {
                        use_gun_bones: stage != 9,
                    });
                if stage == 0 {
                    reset_character(
                        &mut transform,
                        &mut position,
                        rotation,
                        &mut velocity,
                        &mut view_yaw,
                        &mut view_pitch,
                        GROUNDED_START.with_y(fixture.locomotion_start.y),
                        SPRINT_YAW,
                    );
                    rebase_temporal_fixture(
                        &mut state,
                        &mut phase_evidence,
                        "overlay_source_class_boundary",
                    );
                    *gait = Gait::Walking;
                    *rotation_mode = RotationMode::ViewDirection;
                    request.rotation_mode = Some(RotationMode::ViewDirection);
                }
            }
            request.overlay_state = Some(overlays::MODES[mode]);
            request.gait = Some(if stage == 6 {
                Gait::Sprinting
            } else {
                Gait::Walking
            });
            request.stance = Some(if matches!(stage, 2 | 4) {
                Stance::Crouching
            } else {
                Stance::Standing
            });
            input.0 = overlays::command(stage);
            input.0.jump_pressed = stage == 8 && entering;
            view_pitch.0 = overlays::pitch(stage, elapsed);
            if state.overlays_probe.covered[mode][stage] {
                let complete = state.overlays_probe.advance();
                begin_phase(
                    &mut state,
                    if complete {
                        IntegrationPhase::Finish
                    } else {
                        IntegrationPhase::Overlays
                    },
                );
            } else if default_overlay_should_timeout(false, elapsed) {
                state.timed_out = true;
                begin_phase(&mut state, IntegrationPhase::Finish);
            }
        }
        IntegrationPhase::BowOverlay => {
            let index = state.bow_probe.index;
            state.bow_probe.waited_seconds = elapsed;
            if entering && index == 0 {
                reset_character(
                    &mut transform,
                    &mut position,
                    rotation,
                    &mut velocity,
                    &mut view_yaw,
                    &mut view_pitch,
                    GROUNDED_START.with_y(fixture.locomotion_start.y),
                    SPRINT_YAW,
                );
                rebase_temporal_fixture(&mut state, &mut phase_evidence, "bow_start");
                *gait = Gait::Walking;
                *rotation_mode = RotationMode::ViewDirection;
                request.rotation_mode = Some(RotationMode::ViewDirection);
            }
            request.overlay_state = Some(OverlayState::Bow);
            request.gait = Some(if index == 8 {
                Gait::Sprinting
            } else {
                Gait::Walking
            });
            request.stance = Some(if index == 4 {
                Stance::Crouching
            } else {
                Stance::Standing
            });
            input.0 = bow::command(index);
            input.0.jump_pressed = index == 9 && entering;
            view_pitch.0 = bow::pitch(index, elapsed);
            if state.bow_probe.covered[index] {
                state.bow_probe.index += 1;
                begin_phase(
                    &mut state,
                    if index == bow::LABELS.len() - 1 {
                        IntegrationPhase::Mantle
                    } else {
                        IntegrationPhase::BowOverlay
                    },
                );
            } else if default_overlay_should_timeout(false, elapsed) {
                state.timed_out = true;
                begin_phase(&mut state, IntegrationPhase::Finish);
            }
        }
        IntegrationPhase::DefaultOverlay => {
            let index = state.default_overlay_probe.case_index;
            if entering && index == 0 {
                state.default_overlay_probe = DefaultOverlayProbe {
                    valid: true,
                    ..default()
                };
                reset_character(
                    &mut transform,
                    &mut position,
                    rotation,
                    &mut velocity,
                    &mut view_yaw,
                    &mut view_pitch,
                    GROUNDED_START.with_y(fixture.locomotion_start.y),
                    SPRINT_YAW,
                );
                rebase_temporal_fixture(&mut state, &mut phase_evidence, "default_overlay_start");
                *gait = Gait::Walking;
                *overlay = OverlayState::Default;
                *rotation_mode = RotationMode::Aiming;
            }
            request.overlay_state = Some(OverlayState::Default);
            request.rotation_mode = Some(RotationMode::Aiming);
            request.gait = Some(Gait::Walking);
            request.stance = Some(if index == 2 {
                Stance::Crouching
            } else {
                Stance::Standing
            });
            if index == 1 || index == 2 {
                input.0.move_dir = Vec2::Y;
            }
            if index == 3 || index == 4 {
                let probe = &state.default_overlay_probe;
                let ready = *stance == Stance::Standing
                    && *locomotion == LocomotionState::Grounded
                    && velocity.0.xz().length() < 0.1
                    && probe.sample.ready
                    && probe.sample.inputs[0].is_some_and(|value| value > 0.99)
                    && probe.sample.inputs[3].is_some_and(|value| value < 0.01);
                if probe.jump_started_at.is_none() && ready {
                    state.default_overlay_probe.jump_started_at = Some(state.game_time_seconds);
                    input.0.jump_pressed = true;
                }
                input.0.jump_held = state
                    .default_overlay_probe
                    .jump_started_at
                    .is_some_and(|start| state.game_time_seconds - start < 0.2);
            }
            let completed = if index == 3 {
                state.default_overlay_probe.jump_completed
            } else if index == 4 {
                state.default_overlay_probe.jump_completed
                    && state.default_overlay_probe.covered[3..5]
                        .iter()
                        .all(|value| *value)
                    && state.default_overlay_probe.prediction_increased
                    && state.default_overlay_probe.prediction_decreased
            } else {
                state.default_overlay_probe.covered[index]
            };
            if completed {
                state.default_overlay_probe.case_index += 1;
                state.default_overlay_probe.jump_started_at = None;
                state.default_overlay_probe.airborne_seen = false;
                state.default_overlay_probe.jump_completed = false;
                begin_phase(
                    &mut state,
                    if index == 5 {
                        IntegrationPhase::Finish
                    } else {
                        IntegrationPhase::DefaultOverlay
                    },
                );
            } else if default_overlay_should_timeout(completed, elapsed) {
                state.timed_out = true;
                begin_phase(&mut state, IntegrationPhase::Finish);
            }
        }
        IntegrationPhase::StateMachineGrounded => {
            *animation_window = state_machine_grounded_window(elapsed);
            if entering {
                reset_character(
                    &mut transform,
                    &mut position,
                    rotation,
                    &mut velocity,
                    &mut view_yaw,
                    &mut view_pitch,
                    fixture.locomotion_start,
                    fixture.facing_yaw,
                );
                *gait = Gait::Running;
                *overlay = OverlayState::Default;
                *rotation_mode = RotationMode::Aiming;
                request.locomotion_state = Some(LocomotionState::Grounded);
                request.stance = Some(Stance::Standing);
                request.gait = Some(Gait::Running);
                request.rotation_mode = Some(RotationMode::Aiming);
                rebase_temporal_fixture(&mut state, &mut phase_evidence, "state_machine_grounded");
                info!("[MANUAL-INTEGRATION][P1][GROUND] sequence=idle_move_stop_pivot start");
            }
            let requested_gait = if (3.2..3.6).contains(&elapsed) {
                Gait::Walking
            } else {
                Gait::Running
            };
            request.locomotion_state = Some(LocomotionState::Grounded);
            request.stance = Some(Stance::Standing);
            request.gait = Some(requested_gait);
            request.rotation_mode = Some(RotationMode::Aiming);
            if (0.75..2.4).contains(&elapsed) {
                input.0.move_dir = Vec2::Y;
            } else if elapsed >= 3.2 {
                input.0.move_dir = Vec2::NEG_Y;
                let horizontal_speed = Vec2::new(velocity.0.x, velocity.0.z).length();
                if !state.p1_pivot_sent && (0.1..2.0).contains(&horizontal_speed) {
                    pivot_notices.write(AlsPivotNotify {
                        character,
                        activated: true,
                    });
                    state.p1_pivot_sent = true;
                    info!(
                        "[MANUAL-INTEGRATION][P1][GROUND] pivot_notice speed={horizontal_speed:.3}"
                    );
                }
            } else {
                input.0.move_dir = Vec2::ZERO;
            }
            if elapsed >= 5.8 {
                begin_phase(&mut state, IntegrationPhase::StateMachineAir);
            }
        }
        IntegrationPhase::TransitionNotify => {
            if entering {
                state.generated_stop_quick_input_started = false;
                state.generated_stop_quick_input_released = false;
                state.generated_stop_quick_count_before = None;
            }
            let (input_started, input_released, should_move) = generated_stop_quick_input_step(
                state.generated_stop_quick_input_started,
                state.generated_stop_quick_input_released,
                standing_graph,
            );
            state.generated_stop_quick_input_started = input_started;
            state.generated_stop_quick_input_released = input_released;
            let stopping = input_released;
            *animation_window = ManualAnimationWindowEvidence {
                label: Some("generated_transition_stop_quick"),
                settled: stopping,
                capture_source_window: stopping,
                requires_pre_rig_motion: true,
                requires_final_pose_motion: true,
                requires_non_reference_root_pose: true,
                requires_source_pose_chain: true,
                source_node: Some("AB_Als_Standing/Standing States"),
                source_state: Some("Move to Idle"),
                source_pose_chain: quick_stop_source_pose_chain(),
                ..default()
            };
            if entering {
                reset_character(
                    &mut transform,
                    &mut position,
                    rotation,
                    &mut velocity,
                    &mut view_yaw,
                    &mut view_pitch,
                    fixture.locomotion_start,
                    fixture.facing_yaw,
                );
                *gait = Gait::Running;
                *overlay = OverlayState::Default;
                *rotation_mode = RotationMode::Aiming;
                request.locomotion_state = Some(LocomotionState::Grounded);
                request.stance = Some(Stance::Standing);
                request.gait = Some(Gait::Running);
                request.rotation_mode = Some(RotationMode::Aiming);
                state.coverage.generated_transition_stop_quick = false;
                rebase_temporal_fixture(&mut state, &mut phase_evidence, "transition_notify");
            }
            input.0.move_dir = if should_move { Vec2::Y } else { Vec2::ZERO };
            if state.coverage.generated_transition_stop_quick
                && stopping
                && manual_gate_exit_ready(visual_progress.as_deref(), false)
            {
                begin_phase(&mut state, IntegrationPhase::Finish);
            } else if elapsed >= 7.0 {
                state.timed_out = true;
                begin_phase(&mut state, IntegrationPhase::Finish);
            }
        }
        IntegrationPhase::StateMachineAir => {
            if entering {
                reset_character(
                    &mut transform,
                    &mut position,
                    rotation,
                    &mut velocity,
                    &mut view_yaw,
                    &mut view_pitch,
                    fixture.locomotion_start,
                    fixture.facing_yaw,
                );
                *gait = Gait::Running;
                *overlay = OverlayState::Default;
                *rotation_mode = RotationMode::ViewDirection;
                request.locomotion_state = Some(LocomotionState::Grounded);
                request.stance = Some(Stance::Standing);
                request.gait = Some(Gait::Running);
                request.rotation_mode = Some(RotationMode::ViewDirection);
                rebase_temporal_fixture(&mut state, &mut phase_evidence, "state_machine_air");
                commands.entity(character).insert(AlsMantleControlSettings {
                    allow_mantling: false,
                    auto_start_in_air: false,
                    ..default()
                });
                info!("[MANUAL-INTEGRATION][P1][AIR] standing_jump start");
            }
            input.0.jump_pressed = (0.5..0.7).contains(&elapsed);
            input.0.jump_held = (0.5..1.0).contains(&elapsed);
            if elapsed >= 5.0 {
                commands
                    .entity(character)
                    .remove::<AlsMantleControlSettings>();
                begin_phase(&mut state, IntegrationPhase::CrouchRecovery);
            }
        }
        IntegrationPhase::Locomotion => {
            if entering {
                reset_character(
                    &mut transform,
                    &mut position,
                    rotation,
                    &mut velocity,
                    &mut view_yaw,
                    &mut view_pitch,
                    fixture.locomotion_start,
                    fixture.facing_yaw,
                );
                *gait = Gait::Running;
                *overlay = OverlayState::Default;
                *rotation_mode = RotationMode::ViewDirection;
                request.locomotion_state = Some(LocomotionState::Grounded);
                request.stance = Some(Stance::Standing);
                state.crouch_recovery_air_crouch_requested = false;
                state.crouch_recovery_standing_requested = false;
                state.coverage.crouch_air_forced_standing = false;
                state.coverage.crouch_air_landed = false;
                state.coverage.crouch_graph_recovered = false;
            }
            if (1.0..2.5).contains(&elapsed) {
                input.0.move_dir = Vec2::NEG_Y;
            } else if (2.5..4.0).contains(&elapsed) {
                input.0.move_dir = Vec2::X;
            } else if (4.0..5.5).contains(&elapsed) {
                input.0.move_dir = Vec2::Y;
            } else if (5.5..7.0).contains(&elapsed) {
                input.0.move_dir = Vec2::Y;
                input.0.sprint_held = true;
            } else if (7.0..10.0).contains(&elapsed) {
                input.0.move_dir = Vec2::Y;
            } else if (10.0..12.0).contains(&elapsed) {
                input.0.move_dir = Vec2::Y;
                input.0.jump_pressed = elapsed < 10.2;
                input.0.jump_held = elapsed < 10.6;
            } else if (12.0..14.0).contains(&elapsed) {
                request.overlay_state = Some(OverlayState::Rifle);
            } else if (14.0..15.0).contains(&elapsed) {
                request.overlay_state = Some(OverlayState::Default);
            }
            input.0.crouch_pressed = locomotion_crouch_pressed(
                state.game_time_seconds - state.phase_started_seconds,
                virtual_time.delta_secs_f64(),
            );
            input.0.look_delta = locomotion_mouse_delta(elapsed, virtual_time.delta_secs());
            if (0.5..1.5).contains(&elapsed) {
                input.0.aim_held = true;
            }
            if elapsed >= 15.0 {
                begin_phase(&mut state, IntegrationPhase::CrouchRecovery);
            }
        }
        IntegrationPhase::CrouchRecovery => {
            if entering {
                let standing_center = state.spawn_translation.unwrap_or(fixture.locomotion_start);
                let recovery_start = params_assets
                    .get(params_handle.id())
                    .map(|params| {
                        stance_center_from_standing_center(
                            params,
                            standing_center,
                            *stance,
                            transform.scale.abs().min_element(),
                        )
                    })
                    .unwrap_or(standing_center);
                reset_character(
                    &mut transform,
                    &mut position,
                    rotation,
                    &mut velocity,
                    &mut view_yaw,
                    &mut view_pitch,
                    recovery_start,
                    fixture.facing_yaw,
                );
                *gait = Gait::Running;
                *overlay = OverlayState::Default;
                *rotation_mode = RotationMode::ViewDirection;
                request.locomotion_state = Some(LocomotionState::Grounded);
                request.stance = Some(Stance::Standing);
                rebase_temporal_fixture(&mut state, &mut phase_evidence, "crouch_recovery");
                commands.entity(character).insert(AlsMantleControlSettings {
                    allow_mantling: false,
                    auto_start_in_air: false,
                    ..default()
                });
                state.crouch_recovery_jump_requested = false;
                state.crouch_recovery_air_crouch_requested = false;
                state.crouch_recovery_standing_requested = false;
                state.coverage.crouch_graph = false;
                state.coverage.crouch_air_forced_standing = false;
                state.coverage.crouch_air_landed = false;
                state.coverage.crouch_graph_recovered = false;
            }
            if !state.crouch_recovery_jump_requested
                && elapsed >= 1.0
                && *stance == Stance::Standing
                && *locomotion == LocomotionState::Grounded
            {
                input.0.jump_pressed = true;
                input.0.jump_held = true;
                state.crouch_recovery_jump_requested = true;
            } else if state.coverage.crouch_air_landed && !state.crouch_recovery_standing_requested
            {
                input.0.crouch_pressed = true;
                state.crouch_recovery_standing_requested = true;
            } else if state.coverage.crouch_graph_recovered {
                commands
                    .entity(character)
                    .remove::<AlsMantleControlSettings>();
                if state.scenario == IntegrationScenario::StateMachine {
                    begin_phase(&mut state, IntegrationPhase::Turn);
                } else {
                    begin_phase(&mut state, IntegrationPhase::CrouchClearance);
                }
            }
            if elapsed >= 12.0 {
                commands
                    .entity(character)
                    .remove::<AlsMantleControlSettings>();
                state.timed_out = true;
                begin_phase(&mut state, IntegrationPhase::Finish);
            }
        }
        IntegrationPhase::CrouchClearance => {
            if entering {
                cleanup_crouch_canopy(&mut commands, &mut state);
                transform.scale = Vec3::splat(2.0);
                let standing_center = state.spawn_translation.unwrap_or(fixture.locomotion_start);
                let clearance_start = params_assets
                    .get(params_handle.id())
                    .map(|params| {
                        let scale = transform.scale.abs().min_element();
                        let standing_center = standing_center
                            + Vec3::Y * params.crouch.standing_capsule_half_height * (scale - 1.0);
                        stance_center_from_standing_center(params, standing_center, *stance, scale)
                    })
                    .unwrap_or(standing_center);
                reset_character(
                    &mut transform,
                    &mut position,
                    rotation,
                    &mut velocity,
                    &mut view_yaw,
                    &mut view_pitch,
                    clearance_start,
                    fixture.facing_yaw,
                );
                *gait = Gait::Running;
                *overlay = OverlayState::Default;
                *rotation_mode = RotationMode::ViewDirection;
                request.locomotion_state = Some(LocomotionState::Grounded);
                request.stance = Some(Stance::Standing);
                rebase_temporal_fixture(&mut state, &mut phase_evidence, "crouch_clearance");
                state.crouch_clearance_stage = CrouchClearanceStage::AwaitStanding;
                state.crouch_clearance_cycles = 0;
                state.crouch_clearance_last_actual = Stance::Standing;
                state.crouch_canopy_prepare_at = None;
                state.crouch_canopy_armed_at = None;
                state.coverage.crouch_cycles = false;
                state.coverage.crouch_scaled = false;
                state.coverage.crouch_canopy_visible = false;
                state.coverage.crouch_canopy_blocked = false;
                state.coverage.crouch_canopy_recovered = false;
                state.coverage.crouch_canopy_cleaned = false;
                state.coverage.crouch_moving = false;
            }
            match state.crouch_clearance_stage {
                CrouchClearanceStage::AwaitStanding => {
                    if *stance == Stance::Standing
                        && *locomotion == LocomotionState::Grounded
                        && !transaction.headroom_blocked
                    {
                        state.crouch_clearance_last_actual = *stance;
                        state.crouch_clearance_stage = CrouchClearanceStage::Cycling;
                        info!(
                            "[MANUAL-INTEGRATION][CROUCH-CLEARANCE-READY] desired={:?} actual={:?} half={:.3}",
                            transaction.desired_stance, stance, transaction.collider_half_height,
                        );
                    }
                }
                CrouchClearanceStage::Cycling => {
                    if *stance == Stance::Standing {
                        if state.crouch_clearance_last_actual == Stance::Crouching {
                            state.crouch_clearance_cycles =
                                state.crouch_clearance_cycles.saturating_add(1);
                            if state.crouch_clearance_cycles % 25 == 0
                                && state.crouch_clearance_cycles < 100
                            {
                                info!(
                                    "[MANUAL-INTEGRATION][CROUCH-CYCLES] pairs={} actual={:?} half={:.3}",
                                    state.crouch_clearance_cycles,
                                    stance,
                                    transaction.collider_half_height,
                                );
                            }
                        }
                        state.crouch_clearance_last_actual = Stance::Standing;
                        if state.crouch_clearance_cycles >= 100 {
                            state.coverage.crouch_cycles = true;
                            state.crouch_clearance_stage =
                                CrouchClearanceStage::PrepareCanopyCrouch;
                            info!(
                                "[MANUAL-INTEGRATION][CROUCH-CYCLES] pairs={} actual={:?} half={:.3}",
                                state.crouch_clearance_cycles,
                                stance,
                                transaction.collider_half_height,
                            );
                        } else {
                            request.stance = Some(Stance::Crouching);
                        }
                    } else {
                        state.crouch_clearance_last_actual = Stance::Crouching;
                        request.stance = Some(Stance::Standing);
                    }
                }
                CrouchClearanceStage::PrepareCanopyCrouch => {
                    rebase_temporal_fixture(&mut state, &mut phase_evidence, "crouch_canopy_reset");
                    let standing_center =
                        state.spawn_translation.unwrap_or(fixture.locomotion_start);
                    let canopy_start = params_assets
                        .get(params_handle.id())
                        .map(|params| {
                            let scale = transform.scale.abs().min_element();
                            let standing_center = standing_center
                                + Vec3::Y
                                    * params.crouch.standing_capsule_half_height
                                    * (scale - 1.0);
                            stance_center_from_standing_center(
                                params,
                                standing_center,
                                *stance,
                                scale,
                            )
                        })
                        .unwrap_or(standing_center);
                    reset_character_without_rotation(
                        &mut transform,
                        &mut position,
                        &mut velocity,
                        &mut view_yaw,
                        &mut view_pitch,
                        canopy_start,
                        fixture.facing_yaw,
                    );
                    *gait = Gait::Walking;
                    *rotation_mode = RotationMode::ViewDirection;
                    request.locomotion_state = Some(LocomotionState::Grounded);
                    request.stance = Some(Stance::Crouching);
                    request.gait = Some(Gait::Walking);
                    request.rotation_mode = Some(RotationMode::ViewDirection);
                    state.crouch_canopy_prepare_at = Some(state.game_time_seconds);
                    state.crouch_clearance_stage = CrouchClearanceStage::StabilizeCanopyCrouch;
                    info!(
                        "[MANUAL-INTEGRATION][CROUCH-CANOPY-STABILIZE] reset=({:.3},{:.3},{:.3})",
                        canopy_start.x, canopy_start.y, canopy_start.z,
                    );
                }
                CrouchClearanceStage::StabilizeCanopyCrouch => {
                    request.locomotion_state = Some(LocomotionState::Grounded);
                    request.stance = Some(Stance::Crouching);
                    request.gait = Some(Gait::Walking);
                    request.rotation_mode = Some(RotationMode::ViewDirection);
                    if *stance == Stance::Crouching
                        && *locomotion == LocomotionState::Grounded
                        && !transaction.headroom_blocked
                        && state
                            .crouch_canopy_prepare_at
                            .is_some_and(|at| state.game_time_seconds - at >= 0.5)
                    {
                        state.crouch_clearance_stage = CrouchClearanceStage::ArmCanopy;
                    }
                }
                CrouchClearanceStage::ArmCanopy => {
                    if let Some(params) = params_assets.get(params_handle.id()) {
                        let crouched_half_height = params.crouch.crouched_capsule_half_height
                            * transform.scale.abs().min_element();
                        let base_y = transform.translation.y - crouched_half_height;
                        let canopy_bottom = base_y + crouched_half_height * 2.0 + 0.08;
                        let canopy_center = Vec3::new(
                            transform.translation.x,
                            canopy_bottom + 0.08,
                            transform.translation.z,
                        );
                        let canopy = commands
                            .spawn((
                                Name::new("manual_integration_crouch_canopy"),
                                RigidBody::Static,
                                CollisionLayers::new(
                                    [AlsCollisionLayer::Default, AlsCollisionLayer::Visibility],
                                    avian3d::prelude::LayerMask::ALL,
                                ),
                                Collider::cuboid(1.2, 0.08, 1.2),
                                Transform::from_translation(canopy_center)
                                    .with_scale(Vec3::new(1.5, 0.1, 1.5)),
                                Mesh3d(fixture.obstacle_mesh.clone()),
                                MeshMaterial3d(fixture.obstacle_material.clone()),
                            ))
                            .id();
                        state.crouch_canopy = Some(canopy);
                        state.crouch_canopy_center = Some(canopy_center);
                        state.crouch_canopy_armed_at = Some(state.game_time_seconds);
                        state.coverage.crouch_canopy_visible = true;
                        state.crouch_clearance_stage = CrouchClearanceStage::AwaitCanopyReady;
                        info!(
                            "[MANUAL-INTEGRATION][CROUCH-CANOPY] center=({:.3},{:.3},{:.3}) bottom={:.3}",
                            canopy_center.x, canopy_center.y, canopy_center.z, canopy_bottom,
                        );
                    }
                }
                CrouchClearanceStage::AwaitCanopyReady => {
                    if state
                        .crouch_canopy_armed_at
                        .is_some_and(|at| state.game_time_seconds - at >= 0.5)
                    {
                        state.crouch_clearance_stage = CrouchClearanceStage::RequestStanding;
                    }
                }
                CrouchClearanceStage::RequestStanding => {
                    if *stance == Stance::Crouching {
                        request.stance = Some(Stance::Standing);
                        state.crouch_clearance_stage = CrouchClearanceStage::AwaitBlocked;
                    }
                }
                CrouchClearanceStage::AwaitBlocked => {
                    if state.coverage.crouch_canopy_blocked {
                        state.crouch_clearance_stage = CrouchClearanceStage::MoveAway;
                    }
                }
                CrouchClearanceStage::MoveAway => {
                    input.0.move_dir = Vec2::X;
                    if state.coverage.crouch_canopy_recovered {
                        state.crouch_clearance_stage = CrouchClearanceStage::AwaitRecovery;
                    }
                }
                CrouchClearanceStage::AwaitRecovery => {
                    if state.coverage.crouch_canopy_recovered {
                        cleanup_crouch_canopy(&mut commands, &mut state);
                        transform.scale = Vec3::ONE;
                        rebase_temporal_fixture(
                            &mut state,
                            &mut phase_evidence,
                            "crouch_scaled_exit",
                        );
                        begin_phase(&mut state, IntegrationPhase::CrouchDirectionRegression);
                    }
                }
            }
            if elapsed >= CROUCH_CLEARANCE_TIMEOUT {
                cleanup_crouch_canopy(&mut commands, &mut state);
                state.timed_out = true;
                begin_phase(&mut state, IntegrationPhase::Finish);
            }
        }
        IntegrationPhase::CrouchDirectionRegression => {
            if entering {
                let crouch_start = state.spawn_translation.unwrap_or(fixture.locomotion_start);
                reset_character_with_rotation(
                    &mut transform,
                    &mut position,
                    rotation.as_deref_mut(),
                    &mut velocity,
                    &mut view_yaw,
                    &mut view_pitch,
                    crouch_start,
                    fixture.facing_yaw,
                );
                *gait = Gait::Running;
                *overlay = OverlayState::Default;
                *rotation_mode = RotationMode::Aiming;
                request.locomotion_state = Some(LocomotionState::Grounded);
                request.stance = Some(Stance::Crouching);
                request.gait = Some(Gait::Running);
                request.rotation_mode = Some(RotationMode::Aiming);
                state.crouch_direction_case_index = usize::MAX;
                state.coverage.crouch_direction_cases = [false; CROUCH_DIRECTION_CASE_COUNT];
                state.coverage.crouch_direction_diagnostics = [false; CROUCH_DIRECTION_CASE_COUNT];
                state.coverage.crouch_direction_state_mask = 0;
                state.coverage.crouch_direction_lean = false;
                state.coverage.crouch_rotate_mask = 0;
                state.coverage.crouch_rotate_source_completion_mask = 0;
                state.coverage.crouch_rotate_completion_mask = 0;
                state.coverage.crouch_turn_active_mask = 0;
                state.coverage.crouch_turn_completion_mask = 0;
                state.crouch_stationary_validation = None;
                state.crouch_stationary_precondition_ready = [false; 2];
                state.crouch_stationary_precondition_frames = [0; 2];
                state.crouch_stationary_base_yaw = [None; 2];
                state.crouch_stationary_started_at = [None; 2];
                state.crouch_turn_epoch_baseline = [None; 2];
                info!("[MANUAL-INTEGRATION][CROUCH-DIRECTION] start");
            }
            let direction_duration =
                CROUCH_DIRECTION_CASE_COUNT as f32 * CROUCH_DIRECTION_CASE_SECONDS;
            request.locomotion_state = Some(LocomotionState::Grounded);
            request.stance = Some(Stance::Crouching);
            request.gait = Some(Gait::Running);
            let pending_case = state
                .coverage
                .crouch_direction_cases
                .iter()
                .position(|covered| !covered);
            if pending_case.is_none() {
                input.0.move_dir = Vec2::ZERO;
            }
            input.0.aim_held = false;
            if let Some(case_index) = pending_case {
                let Some(case) = crouch_direction_regression_case(case_index) else {
                    return;
                };
                *animation_window = ManualAnimationWindowEvidence {
                    actor_yaw_driver: None,
                    label: Some(case.name),
                    index: Some(case_index),
                    settled: state.crouch_direction_case_index == case_index
                        && state.crouch_direction_ready_at.is_some(),
                    capture_source_window: true,
                    capture_every_frame: false,
                    requires_pre_rig_motion: true,
                    requires_final_pose_motion: true,
                    requires_non_reference_root_pose: true,
                    requires_source_pose_chain: true,
                    requires_actor_yaw_motion: false,
                    requires_actor_yaw_convergence: false,
                    expected_actor_yaw_delta_sign: None,
                    expected_actor_yaw_baseline: None,
                    expected_animation_variant: None,
                    source_node: Some(crouch_direction_source_node(case.expected_direction)),
                    source_state: direction_regression_case(case_index)
                        .and_then(|case| case.source_state),
                    source_pose_chain: crouch_direction_source_pose_chain(case.expected_direction),
                    minimum_pre_rig_translation_span: 0.0,
                    minimum_pre_rig_rotation_span_radians: 0.05,
                    minimum_final_translation_span: 0.0,
                    minimum_final_rotation_span_radians: 0.05,
                    minimum_actor_yaw_delta_radians: 0.0,
                    minimum_actor_yaw_convergence_radians: 0.0,
                    minimum_observation_seconds: 0.8,
                    source_sync_group: Some("Movement"),
                    sync_phase_global: Some("sync_groups.movement.phase"),
                    canonical_sync_phase: Some(0.25),
                    maximum_sync_phase_distance: 0.08,
                    canonical_clip_time: None,
                    maximum_clip_time_distance: 0.0,
                    direction_signature_group: matches!(case_index, 0 | 1 | 3 | 7)
                        .then_some("crouch_direction"),
                    required_direction_signature_members: 4,
                    minimum_direction_signature_rotation_radians: 1e-4,
                    expected_direction: Some(movement_direction_name(case.expected_direction)),
                    observed_direction: None,
                    expected_runtime_state: Some(movement_direction_fsm_index(
                        case.expected_direction,
                    )),
                    observed_runtime_state: None,
                    expected_parent_runtime_state: Some(1),
                    expected_terminal_parent_runtime_state: None,
                    expected_terminal_parent_transition_active: None,
                    expected_terminal_requested_parent_transition_active: None,
                    observed_parent_runtime_state: None,
                    observed_parent_transition_active: None,
                    observed_requested_parent_runtime_state: None,
                    observed_requested_parent_transition_active: None,
                    observed_actor_yaw: None,
                    observed_view_yaw: None,
                    observed_animation_yaw_speed_degrees: None,
                    observed_animation_curve_direct_yaw_speed_degrees: None,
                    observed_animation_clip_time: None,
                    observed_animation_weight: None,
                    observed_animation_play_rate: None,
                    observed_animation_curve_stack_weight: None,
                    observed_animation_curve_stack_entries: Vec::new(),
                    observed_animation_variant: None,
                    expected_lifecycle_stages: &[],
                    observed_lifecycle_stages: &[],
                };
                if state.crouch_direction_case_index != case_index {
                    state.crouch_direction_case_index = case_index;
                    state.crouch_direction_case_started_at = elapsed;
                    state.crouch_direction_ready_at = None;
                    let crouch_start = state.spawn_translation.unwrap_or(fixture.locomotion_start);
                    reset_character_with_rotation(
                        &mut transform,
                        &mut position,
                        rotation.as_deref_mut(),
                        &mut velocity,
                        &mut view_yaw,
                        &mut view_pitch,
                        crouch_start,
                        fixture.facing_yaw,
                    );
                    if let Some(avatar) = state.avatar {
                        commands
                            .entity(avatar)
                            .remove::<AlsMovementDirectionState>();
                    }
                    info!(
                        "[MANUAL-INTEGRATION][CROUCH-DIRECTION][START] case={} input=({:.3},{:.3}) expected={:?}",
                        case.name, case.move_dir.x, case.move_dir.y, case.expected_direction,
                    );
                }
                input.0.move_dir = case.move_dir;
                input.0.aim_held = true;
                *rotation_mode = RotationMode::Aiming;
                request.rotation_mode = Some(RotationMode::Aiming);
                view_yaw.0 = fixture.facing_yaw;
            } else if let Some(validation) = next_crouch_stationary_validation(&state.coverage) {
                if state.crouch_stationary_validation != Some(validation) {
                    state.crouch_stationary_validation = Some(validation);
                    state.crouch_stationary_precondition_ready[validation.side_index()] = false;
                    state.crouch_stationary_precondition_frames[validation.side_index()] = 0;
                    state.crouch_stationary_base_yaw[validation.side_index()] = None;
                    state.crouch_stationary_started_at[validation.side_index()] = None;
                    state.crouch_turn_epoch_baseline[validation.side_index()] = None;
                    state.temporal_rebase_epoch = state.temporal_rebase_epoch.wrapping_add(1);
                    phase_evidence.temporal_rebase_epoch = state.temporal_rebase_epoch;
                    info!(
                        "[MANUAL-INTEGRATION][TEMPORAL-REBASE] epoch={} validation={validation:?}",
                        state.temporal_rebase_epoch
                    );
                    info!(
                        "[MANUAL-INTEGRATION][CROUCH-STATIONARY][START] validation={validation:?}"
                    );
                }
                let command_index = match validation {
                    CrouchStationaryValidation::TurnLeft => 0,
                    CrouchStationaryValidation::TurnRight => 1,
                    CrouchStationaryValidation::RotateLeft => 4,
                    CrouchStationaryValidation::RotateRight => 5,
                };
                let command = standing_turn_mouse_command(
                    command_index,
                    state.crouch_stationary_started_at[validation.side_index()]
                        .map(|start| elapsed - start),
                    if validation.is_turn() {
                        state.crouch_stationary_base_yaw[validation.side_index()]
                            .unwrap_or(fixture.facing_yaw)
                    } else {
                        state.crouch_stationary_view_yaw
                    },
                    view_yaw.0,
                    *rotation_mode,
                );
                input.0.move_dir = Vec2::ZERO;
                input.0.look_delta = command.look_delta;
                if validation.is_turn() {
                    input.0.aim_held = crouch_stationary_validation_holds_aim(Some(validation));
                    *rotation_mode = RotationMode::ViewDirection;
                    request.rotation_mode = Some(RotationMode::ViewDirection);
                    *animation_window = crouch_turn_window(validation.is_left(), false, &[]);
                    animation_window.expected_actor_yaw_baseline =
                        state.crouch_stationary_base_yaw[validation.side_index()];
                } else {
                    input.0.aim_held = true;
                    *rotation_mode = RotationMode::Aiming;
                    request.rotation_mode = Some(RotationMode::Aiming);
                    *animation_window = crouch_rotate_window(
                        validation.is_left(),
                        state.crouch_stationary_precondition_ready[validation.side_index()],
                        &[],
                    );
                    animation_window.expected_actor_yaw_baseline =
                        state.crouch_stationary_base_yaw[validation.side_index()];
                }
            } else {
                *animation_window = ManualAnimationWindowEvidence::default();
                state.crouch_stationary_validation = None;
                request.stance = Some(Stance::Standing);
                if state.scenario == IntegrationScenario::Crouching {
                    begin_phase(&mut state, IntegrationPhase::Finish);
                } else {
                    begin_phase(&mut state, IntegrationPhase::Turn);
                }
            }
            if elapsed >= direction_duration + CROUCH_STATIONARY_TIMEOUT_SECONDS {
                state.timed_out = true;
                begin_phase(&mut state, IntegrationPhase::Finish);
            }
        }
        IntegrationPhase::Turn => {
            if entering {
                let turn_start = state.spawn_translation.unwrap_or(transform.translation);
                let turn_yaw = state.spawn_yaw.unwrap_or(fixture.facing_yaw);
                reset_character(
                    &mut transform,
                    &mut position,
                    rotation,
                    &mut velocity,
                    &mut view_yaw,
                    &mut view_pitch,
                    turn_start,
                    turn_yaw,
                );
                *gait = Gait::Walking;
                *overlay = OverlayState::Default;
                *rotation_mode = RotationMode::ViewDirection;
                request.locomotion_state = Some(LocomotionState::Grounded);
                request.stance = Some(Stance::Standing);
                rebase_temporal_fixture(&mut state, &mut phase_evidence, "turn_fixture");
            }
            let body_yaw = world_forward_to_yaw(transform.rotation * Vec3::NEG_Z);
            if state.scenario == IntegrationScenario::StateMachine {
                view_yaw.0 = state_machine_turn_target_yaw(elapsed, body_yaw);
                if state_machine_turn_phase_complete(elapsed) {
                    begin_phase(&mut state, IntegrationPhase::StateMachineRotate);
                }
            } else {
                if elapsed >= 0.5 {
                    view_yaw.0 = body_yaw + 90.0f32.to_radians();
                }
                if elapsed >= 2.5 {
                    begin_phase(&mut state, IntegrationPhase::Mantle);
                }
            }
        }
        IntegrationPhase::StateMachineRotate => {
            if let Some(label) = state_machine_rotation_boundary(elapsed, virtual_time.delta_secs())
            {
                rebase_temporal_fixture(&mut state, &mut phase_evidence, label);
            }
            if entering {
                reset_character_with_rotation(
                    &mut transform,
                    &mut position,
                    rotation.as_deref_mut(),
                    &mut velocity,
                    &mut view_yaw,
                    &mut view_pitch,
                    fixture.locomotion_start,
                    fixture.facing_yaw,
                );
                *gait = Gait::Walking;
                *overlay = OverlayState::Default;
                *rotation_mode = RotationMode::Aiming;
                request.locomotion_state = Some(LocomotionState::Grounded);
                request.stance = Some(Stance::Standing);
                request.gait = Some(Gait::Walking);
                rebase_temporal_fixture(&mut state, &mut phase_evidence, "state_machine_rotate");
                info!("[MANUAL-INTEGRATION][P1][TURN] rotate_modes start");
            }
            input.0.aim_held = elapsed < 2.6;
            let body_yaw = fixture.facing_yaw;
            transform.rotation = Quat::from_rotation_y(body_yaw);
            if let Some(mut rotation) = rotation {
                rotation.0 = transform.rotation;
            }
            request.locomotion_state = Some(LocomotionState::Grounded);
            request.stance = Some(Stance::Standing);
            request.gait = Some(Gait::Walking);
            if elapsed < 1.3 {
                commands.entity(character).remove::<AlsFirstPersonView>();
                *rotation_mode = RotationMode::Aiming;
                request.rotation_mode = Some(RotationMode::Aiming);
                view_yaw.0 = body_yaw - 90.0f32.to_radians();
            } else if elapsed < 2.6 {
                commands.entity(character).remove::<AlsFirstPersonView>();
                *rotation_mode = RotationMode::Aiming;
                request.rotation_mode = Some(RotationMode::Aiming);
                view_yaw.0 = body_yaw + 90.0f32.to_radians();
            } else if elapsed < 3.9 {
                commands.entity(character).insert(AlsFirstPersonView);
                *rotation_mode = RotationMode::ViewDirection;
                request.rotation_mode = Some(RotationMode::ViewDirection);
                view_yaw.0 = body_yaw - 90.0f32.to_radians();
            } else if elapsed < 5.2 {
                commands.entity(character).insert(AlsFirstPersonView);
                *rotation_mode = RotationMode::ViewDirection;
                request.rotation_mode = Some(RotationMode::ViewDirection);
                view_yaw.0 = body_yaw + 90.0f32.to_radians();
            } else {
                commands.entity(character).remove::<AlsFirstPersonView>();
                begin_phase(&mut state, IntegrationPhase::TransitionNotify);
            }
        }
        IntegrationPhase::Mantle => {
            let parent_transform = Transform::from_xyz(5.0, 2.0, -3.0)
                .with_rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2))
                .with_scale(Vec3::splat(1.5));
            show_debug.visible = true;
            show_debug.mantling = true;
            if entering {
                if state.scenario == IntegrationScenario::Base {
                    commands
                        .entity(character)
                        .insert(bevy_als::als_camera::AlsCameraCustomTimeDilation(0.5));
                }
                reset_character(
                    &mut transform,
                    &mut position,
                    rotation,
                    &mut velocity,
                    &mut view_yaw,
                    &mut view_pitch,
                    fixture.mantle_start,
                    fixture.facing_yaw,
                );
                *gait = Gait::Running;
                *overlay = if state.scenario == IntegrationScenario::BowOverlay {
                    OverlayState::Bow
                } else {
                    OverlayState::Default
                };
                if state.scenario == IntegrationScenario::BowOverlay {
                    rebase_temporal_fixture(&mut state, &mut phase_evidence, "bow_mantle_start");
                }
                *rotation_mode = RotationMode::ViewDirection;
                request.locomotion_state = Some(LocomotionState::Grounded);
                request.stance = Some(Stance::Standing);
            }
            if state.scenario == IntegrationScenario::Base {
                if elapsed < 10.0
                    && mantle_for_input
                        .is_some_and(|mantle| mantle.active && mantle.root_motion_ready)
                {
                    if parents.get(character).is_err() {
                        let parent = commands
                            .spawn((Name::new("manual_mantle_parent"), parent_transform))
                            .id();
                        *transform = GlobalTransform::from(*transform)
                            .reparented_to(&GlobalTransform::from(parent_transform));
                        commands.entity(character).insert(ChildOf(parent));
                    }
                } else if let Ok(parent) = parents.get(character) {
                    *transform = parent_transform.mul_transform(*transform);
                    commands.entity(character).remove::<ChildOf>();
                    commands.entity(parent.parent()).despawn();
                }
            }
            let forward = Quat::from_rotation_y(fixture.facing_yaw) * Vec3::NEG_Z;
            let progress = (position.0 - fixture.mantle_start).dot(forward);
            if player_view.is_some_and(|view| {
                bevy_als::als::motion::wrap_yaw_radians(view.0 - fixture.facing_yaw).abs() < 0.001
            }) {
                input.0.move_dir = Vec2::Y;
            }
            input.0.jump_pressed = (5.85..7.1).contains(&progress) && elapsed < 7.0;
            if elapsed >= 10.0 {
                if state.scenario == IntegrationScenario::Base {
                    commands
                        .entity(character)
                        .remove::<bevy_als::als_camera::AlsCameraCustomTimeDilation>();
                }
                if state.scenario == IntegrationScenario::BowOverlay {
                    begin_phase(&mut state, IntegrationPhase::Ragdoll);
                } else if state.scenario == IntegrationScenario::FootIk {
                    begin_phase(&mut state, IntegrationPhase::Roll);
                } else {
                    begin_phase(&mut state, IntegrationPhase::Platform);
                }
            }
        }
        IntegrationPhase::Platform => {
            if entering {
                if let Ok((mut platform_transform, platform_position, mut platform_velocity)) =
                    platforms.get_mut(fixture.platform)
                {
                    let platform_location = fixture.platform_home;
                    platform_transform.translation = platform_location;
                    if let Some(mut platform_position) = platform_position {
                        platform_position.0 = platform_location;
                    }
                    platform_velocity.0 = Vec3::X * 1.5;
                }
                reset_character(
                    &mut transform,
                    &mut position,
                    rotation,
                    &mut velocity,
                    &mut view_yaw,
                    &mut view_pitch,
                    fixture.platform_home + Vec3::Y * 1.17,
                    fixture.facing_yaw,
                );
                *gait = Gait::Running;
                *overlay = OverlayState::Default;
                request.locomotion_state = Some(LocomotionState::Grounded);
            }
            if elapsed >= 6.0 {
                begin_phase(&mut state, IntegrationPhase::Roll);
            }
        }
        IntegrationPhase::Roll => {
            if entering {
                reset_character(
                    &mut transform,
                    &mut position,
                    rotation,
                    &mut velocity,
                    &mut view_yaw,
                    &mut view_pitch,
                    GROUNDED_START.with_y(fixture.roll_start.y),
                    GROUNDED_ROLL_YAW,
                );
                rebase_temporal_fixture(
                    &mut state,
                    &mut phase_evidence,
                    "grounded_original_floor_start",
                );
                *gait = Gait::Running;
                *overlay = OverlayState::Default;
                request.locomotion_state = Some(LocomotionState::Grounded);
                request.stance = Some(Stance::Standing);
            }
            input.0.move_dir = if elapsed < 1.25 { Vec2::Y } else { Vec2::ZERO };
            input.0.roll_pressed = (1.0..1.2).contains(&elapsed);
            if state.scenario == IntegrationScenario::Grounded && input.0.roll_pressed {
                commands
                    .entity(character)
                    .insert(bevy_als::als_camera::AlsCameraCustomTimeDilation(0.5));
            }
            if elapsed >= FOOT_AVATAR_REPLACEMENT_START_SECONDS
                && (state.scenario != IntegrationScenario::Grounded
                    || state.coverage.grounded_roll_completed)
            {
                if state.scenario == IntegrationScenario::FootIk {
                    state.foot_replacement_waited = elapsed - FOOT_AVATAR_REPLACEMENT_START_SECONDS;
                    if state.coverage.foot_bones_replaced && state.coverage.camera_root_rebound {
                        begin_phase(&mut state, IntegrationPhase::Finish);
                    } else if foot_avatar_replacement_timed_out(
                        elapsed,
                        state.coverage.foot_bones_replaced && state.coverage.camera_root_rebound,
                    ) {
                        state.timed_out = true;
                        begin_phase(&mut state, IntegrationPhase::Finish);
                    }
                } else {
                    let next = if state.scenario == IntegrationScenario::Grounded {
                        commands
                            .entity(character)
                            .remove::<bevy_als::als_camera::AlsCameraCustomTimeDilation>();
                        IntegrationPhase::GroundedJoint
                    } else {
                        IntegrationPhase::Finish
                    };
                    begin_phase(&mut state, next);
                }
            }
        }
        IntegrationPhase::GroundedJoint => {
            if entering {
                let index = state.grounded_joint_probe.case_index;
                if index == 0 {
                    reset_character(
                        &mut transform,
                        &mut position,
                        rotation,
                        &mut velocity,
                        &mut view_yaw,
                        &mut view_pitch,
                        GROUNDED_START.with_y(fixture.roll_start.y),
                        SPRINT_YAW,
                    );
                    rebase_temporal_fixture(
                        &mut state,
                        &mut phase_evidence,
                        "grounded_joint_floor_start",
                    );
                }
                state.grounded_joint_probe = GroundedJointProbe {
                    case_index: index,
                    base_yaw: view_yaw.0,
                    valid: true,
                    ..default()
                };
                *gait = Gait::Walking;
                *overlay = OverlayState::Default;
            }
            let index = state.grounded_joint_probe.case_index;
            let (_, target_stance, _, aiming) = GROUNDED_JOINT_CASES[index];
            let trigger_elapsed = state
                .grounded_joint_probe
                .triggered_at
                .map(|start| (state.game_time_seconds - start) as f32);
            input.0 = grounded_joint_command(
                index,
                trigger_elapsed,
                state.grounded_joint_probe.base_yaw,
                view_yaw.0,
            );
            if trigger_elapsed.is_none() {
                request.stance = Some(if aiming {
                    target_stance
                } else if target_stance == Stance::Crouching {
                    Stance::Standing
                } else {
                    Stance::Crouching
                });
                if state
                    .grounded_joint_probe
                    .ready_since
                    .is_some_and(|since| state.game_time_seconds - since >= 0.4)
                {
                    request.stance = None;
                    input.0.crouch_pressed = !aiming && index != 1;
                    input.0.jump_pressed = index == 1;
                    state.grounded_joint_probe.triggered_at = Some(state.game_time_seconds);
                }
            }
            if state.coverage.grounded_joint[index]
                && trigger_elapsed.is_some_and(|elapsed| elapsed >= if aiming { 3.4 } else { 1.8 })
            {
                if index + 1 < GROUNDED_JOINT_CASES.len() {
                    state.grounded_joint_probe.case_index += 1;
                    begin_phase(&mut state, IntegrationPhase::GroundedJoint);
                } else {
                    begin_phase(&mut state, IntegrationPhase::Finish);
                }
            } else if elapsed >= 12.0 {
                state.timed_out = true;
                begin_phase(&mut state, IntegrationPhase::Finish);
            }
        }
        IntegrationPhase::Ragdoll => {
            if entering {
                state.ragdoll_seen = false;
                state.ragdoll_exit_requested = false;
                state.ragdoll_frames = 0;
                state.get_up_fixture_seeded = false;
                state.get_up_jump_requested_at = None;
                state.ragdoll_grounded_at = None;
                state.ragdoll_recovered_at = None;
                state.coverage.ragdoll_entered = false;
                state.coverage.ragdoll_pa_profile = false;
                state.coverage.ragdoll_player_replaced = false;
                state.coverage.ragdoll_debug_render = false;
                state.coverage.ragdoll_follow = false;
                state.coverage.ragdoll_scaled = false;
                state.coverage.ragdoll_joint_drive = false;
                state.coverage.ragdoll_recovered = false;
                let mut spawn_translation =
                    state.spawn_translation.unwrap_or(transform.translation);
                if state.scenario == IntegrationScenario::Ragdoll {
                    transform.scale = Vec3::splat(2.0);
                    let standing_half = params_assets
                        .get(params_handle.id())
                        .map_or(ALS_CHARACTER_CAPSULE_HALF_HEIGHT, |params| {
                            params.crouch.standing_capsule_half_height
                        });
                    spawn_translation.y += standing_half + 9.0;
                    rebase_temporal_fixture(
                        &mut state,
                        &mut phase_evidence,
                        "ragdoll_scaled_start",
                    );
                }
                let spawn_yaw = state.spawn_yaw.unwrap_or(fixture.facing_yaw);
                reset_character(
                    &mut transform,
                    &mut position,
                    rotation,
                    &mut velocity,
                    &mut view_yaw,
                    &mut view_pitch,
                    spawn_translation,
                    spawn_yaw,
                );
                *gait = Gait::Running;
                *overlay = if state.scenario == IntegrationScenario::BowOverlay {
                    OverlayState::Bow
                } else {
                    OverlayState::Default
                };
                if state.scenario == IntegrationScenario::BowOverlay {
                    rebase_temporal_fixture(&mut state, &mut phase_evidence, "bow_ragdoll_start");
                }
                request.locomotion_state = (state.scenario != IntegrationScenario::Ragdoll)
                    .then_some(LocomotionState::Grounded);
                request.stance = Some(Stance::Standing);
                if state.scenario == IntegrationScenario::GetUp {
                    if state.get_up_case == 0 {
                        commands
                            .entity(character)
                            .insert(bevy_als::als_camera::AlsCameraCustomTimeDilation(0.5));
                    } else {
                        commands
                            .entity(character)
                            .remove::<bevy_als::als_camera::AlsCameraCustomTimeDilation>();
                    }
                    rebase_temporal_fixture(
                        &mut state,
                        &mut phase_evidence,
                        "get_up_initial_position",
                    );
                }
            }
            let ready_to_exit = elapsed
                >= if state.scenario == IntegrationScenario::GetUp {
                    0.9
                } else {
                    2.0
                }
                && state.coverage.ragdoll_follow
                && state.coverage.ragdoll_joint_drive
                && ground_follow.is_some_and(|follow| follow.grounded);
            if state.ragdoll_exit_requested {
            } else if state.scenario == IntegrationScenario::GetUp && elapsed < 0.3 {
                request.locomotion_state = Some(LocomotionState::Grounded);
            } else if elapsed < 1.0
                && !matches!(
                    state.scenario,
                    IntegrationScenario::Ragdoll | IntegrationScenario::GetUp
                )
            {
                input.0.move_dir = Vec2::Y;
            } else if !ready_to_exit {
                request.ragdoll_start = state.scenario != IntegrationScenario::Ragdoll;
            } else {
                if state.scenario == IntegrationScenario::GetUp {
                    input.0.jump_pressed = true;
                } else {
                    request.ragdoll_exit = true;
                }
                state.ragdoll_exit_requested = true;
            }
            if state.scenario == IntegrationScenario::GetUp && state.get_up_case == 1 {
                if state.get_up_jump_requested_at.is_none()
                    && get_ups.get(character).is_ok_and(|(get_up, blocked)| {
                        blocked && get_up.pose_ready && get_up.clip_time >= 0.9
                    })
                {
                    input.0.jump_pressed = true;
                    state.get_up_jump_requested_at = Some(state.game_time_seconds);
                    info!("[MANUAL-INTEGRATION][GET-UP] jump requested during input lock");
                }
                input.0.jump_held = state
                    .get_up_jump_requested_at
                    .is_some_and(|at| state.game_time_seconds - at < 0.1);
            }
            if state
                .ragdoll_recovered_at
                .is_some_and(|at| state.game_time_seconds - at >= 1.5)
                && elapsed >= 3.0
                && (state.scenario != IntegrationScenario::GetUp || get_ups.get(character).is_err())
            {
                if state.scenario == IntegrationScenario::GetUp && state.get_up_case == 0 {
                    state.get_up_case = 1;
                    begin_phase(&mut state, IntegrationPhase::Ragdoll);
                } else {
                    begin_phase(&mut state, IntegrationPhase::Finish);
                }
            } else if elapsed >= 20.0 {
                state.timed_out = true;
                begin_phase(&mut state, IntegrationPhase::Finish);
            }
        }
        IntegrationPhase::Camera => {
            show_debug.visible = true;
            show_debug.camera_shapes = true;
            show_debug.camera_traces = true;
            if entering {
                state.coverage.camera_collision = false;
                state.coverage.camera_recovery = false;
                state.camera_obstacles = [None; 3];
                state.camera_obstacle_ignored = false;
                state.camera_blocked_ratio = None;
                state.camera_scenario_armed = false;
                state.camera_last_probe_secs = -1.0;
                state.camera_view_mode_requested = false;
                state.camera_shoulder_requested = false;
                state.camera_movement = default();
                let camera_spawn = state.spawn_translation.unwrap_or(transform.translation);
                let camera_yaw = state.spawn_yaw.unwrap_or(fixture.facing_yaw);
                reset_character(
                    &mut transform,
                    &mut position,
                    rotation,
                    &mut velocity,
                    &mut view_yaw,
                    &mut view_pitch,
                    camera_spawn,
                    camera_yaw,
                );
                *gait = Gait::Running;
                *overlay = OverlayState::Default;
                *rotation_mode = RotationMode::ViewDirection;
            }
            if state.coverage.camera_recovery && !state.camera_movement.passed() {
                input.0.look_delta = Vec2::new(8.0, -4.0);
                input.0.move_dir = Vec2::Y * 0.3;
            }
            if !state.camera_shoulder_requested {
                input.0.switch_shoulder_pressed = true;
                state.camera_shoulder_requested = true;
            }
            if elapsed >= 2.0
                && state.coverage.camera_recovery
                && state.camera_movement.passed()
                && !state.camera_view_mode_requested
            {
                input.0.view_mode_pressed = true;
                state.camera_view_mode_requested = true;
            }
            if (3.0..4.0).contains(&elapsed) {
                input.0.look_delta = Vec2::new(2.0, 5.0);
                if !state.dilation_active {
                    virtual_time.set_relative_speed(0.2);
                    state.dilation_active = true;
                }
            } else if state.dilation_active {
                virtual_time.set_relative_speed(1.0);
                state.dilation_active = false;
            }
            if elapsed >= 4.0 && !state.shake_requested {
                shake_requests.write(AlsCameraShakeRequest::sprint(character, 1.0));
                state.shake_requested = true;
            }
            if elapsed >= 0.5
                && state.camera_obstacles.iter().all(Option::is_none)
                && !state.camera_obstacle_ignored
            {
                if let Some(camera) = state.camera
                    && let Ok(debug) = cameras.get(camera)
                    && debug.valid
                    && !debug.trace_blocking_hit
                    && debug.first_person_override <= 1e-5
                    && debug.trace_start.distance(debug.trace_end) > 0.5
                {
                    let owner_direction =
                        (transform.translation - debug.trace_start).normalize_or_zero();
                    let owner_direction = if owner_direction.length_squared() > 1e-6 {
                        owner_direction
                    } else {
                        Vec3::Y
                    };
                    let trace_direction = (debug.trace_end - debug.trace_start).normalize_or_zero();
                    let lateral = trace_direction.cross(owner_direction).normalize_or_zero();
                    let lateral = if lateral.length_squared() > 1e-6 {
                        lateral
                    } else {
                        Vec3::X
                    };
                    let directions = [
                        owner_direction,
                        (owner_direction + lateral * 0.35).normalize_or_zero(),
                    ];
                    for (index, direction) in directions.into_iter().enumerate() {
                        let direction = if direction.length_squared() > 1e-6 {
                            direction
                        } else {
                            owner_direction
                        };
                        let center = debug.trace_start - direction * 0.1;
                        state.camera_obstacles[index] = Some(
                            commands
                                .spawn((
                                    Name::new(format!(
                                        "manual_integration_camera_obstacle_{index}"
                                    )),
                                    RigidBody::Static,
                                    CollisionLayers::new(
                                        AlsCollisionLayer::Visibility,
                                        avian3d::prelude::LayerMask::NONE,
                                    ),
                                    Collider::cuboid(0.05, 0.6, 0.6),
                                    Transform::from_translation(center)
                                        .with_rotation(Quat::from_rotation_arc(Vec3::X, direction)),
                                    Mesh3d(fixture.obstacle_mesh.clone()),
                                    MeshMaterial3d(fixture.obstacle_material.clone()),
                                ))
                                .id(),
                        );
                    }
                    state.camera_scenario_armed = true;
                    state.camera_obstacles[2] = Some(
                        commands
                            .spawn((
                                Name::new("manual_integration_camera_sweep_blocker"),
                                RigidBody::Static,
                                CollisionLayers::new(
                                    AlsCollisionLayer::Visibility,
                                    avian3d::prelude::LayerMask::NONE,
                                ),
                                Collider::cuboid(0.05, 0.6, 0.6),
                                Transform::from_translation(
                                    debug.trace_start.lerp(debug.trace_end, 0.5),
                                )
                                .with_rotation(Quat::from_rotation_arc(Vec3::X, trace_direction)),
                                Mesh3d(fixture.obstacle_mesh.clone()),
                                MeshMaterial3d(fixture.obstacle_material.clone()),
                            ))
                            .id(),
                    );
                    info!("[MANUAL-INTEGRATION] camera MTD pair and sweep blocker armed");
                }
            }
            if state.coverage.camera_collision && !state.camera_obstacle_ignored {
                for obstacle in state.camera_obstacles.iter_mut().flatten() {
                    commands.entity(*obstacle).insert(CollisionLayers::new(
                        AlsCollisionLayer::Default,
                        avian3d::prelude::LayerMask::NONE,
                    ));
                }
                state.camera_obstacle_ignored = true;
                info!("[MANUAL-INTEGRATION] camera obstacles removed from camera query layer");
            }
            if elapsed >= 9.0 {
                for obstacle in state.camera_obstacles.iter().flatten() {
                    commands.entity(*obstacle).despawn();
                }
                state.camera_obstacles = [None; 3];
                virtual_time.set_relative_speed(1.0);
                state.dilation_active = false;
                begin_phase(&mut state, IntegrationPhase::Finish);
            }
        }
        IntegrationPhase::Interface => {
            if entering {
                state.interface_debug_armed = false;
                state.interface_hud_hidden_seen = false;
                state.interface_hud_restored_seen = false;
                state.interface_slomo_seen = false;
                state.interface_slomo_restored_seen = false;
                state.interface_menu_open_seen = false;
                state.interface_menu_closed_seen = false;
                state.interface_overlay_bow_seen = false;
                state.interface_overlay_default_after_bow_seen = false;
                state.interface_overlay_reentry_seen = false;
                state.interface_parent_instance = None;
                state.interface_parent_preserved = true;
                state.coverage.hud_layout = false;
                state.coverage.hud_toggle = false;
                state.coverage.hud_slomo = false;
                state.coverage.overlay_menu = false;
                state.coverage.overlay_interface_binding = false;
                state.coverage.overlay_interface_fallback = false;
                state.coverage.overlay_interface_reentry = false;
                state.coverage.show_debug = false;
                state.coverage.debug_draw = false;
                for category in DebugCategory::ALL {
                    apply_show_debug_key(
                        &mut show_debug,
                        true,
                        match category {
                            DebugCategory::Curves => KeyCode::Digit1,
                            DebugCategory::State => KeyCode::Digit2,
                            DebugCategory::Shapes => KeyCode::Digit3,
                            DebugCategory::Traces => KeyCode::Digit4,
                            DebugCategory::Mantling => KeyCode::Digit5,
                            DebugCategory::CameraCurves => KeyCode::Digit6,
                            DebugCategory::CameraShapes => KeyCode::Digit7,
                            DebugCategory::CameraTraces => KeyCode::Digit8,
                        },
                    );
                }
                state.interface_debug_armed = true;
                *overlay = OverlayState::Default;
                request.overlay_state = Some(OverlayState::Default);
                info!(
                    "[MANUAL-INTEGRATION][INTERFACE] HUD, overlay menu, and show-debug probes armed"
                );
            }
            hud_layout::drive_controls(&mut keys, elapsed);
            request.overlay_state = Some(
                if (0.6..1.4).contains(&elapsed) || (2.6..3.4).contains(&elapsed) {
                    OverlayState::Bow
                } else {
                    OverlayState::Default
                },
            );
            if elapsed >= 5.0 {
                hud_layout::drive_debug_keys(&mut keys, elapsed);
            }
            if elapsed >= 11.2 {
                keys.release(KeyCode::Tab);
                keys.release(KeyCode::KeyZ);
                keys.release(KeyCode::KeyQ);
                keys.release(KeyCode::ShiftLeft);
                virtual_time.set_relative_speed(1.0);
                begin_phase(&mut state, IntegrationPhase::Finish);
            }
        }
        IntegrationPhase::Notify => {
            if state.scenario == IntegrationScenario::AnimationClock {
                if entering {
                    state.notify_roll_armed = false;
                }
                input.0.move_dir = Vec2::Y;
                request.gait = Some(Gait::Running);
                request.stance = Some(if elapsed < 2.0 {
                    Stance::Standing
                } else {
                    Stance::Crouching
                });
                if let Some(avatar) = state.avatar {
                    let one_frame = (3.0..3.1).contains(&elapsed) && !state.notify_roll_armed;
                    if one_frame {
                        state.notify_roll_armed = true;
                    }
                    let paused = (2.2..3.8).contains(&elapsed);
                    commands.queue(move |world: &mut World| {
                        let Some(entity) = world
                            .get::<AnimatedSceneInstance>(avatar)
                            .map(AnimatedSceneInstance::player_entity)
                        else {
                            return;
                        };
                        let Some(mut player) = world.get_mut::<AnimationGraphPlayer>(entity) else {
                            return;
                        };
                        if one_frame {
                            player.play_one_frame();
                        } else if paused {
                            player.pause();
                        } else {
                            player.resume();
                        }
                    });
                    if (3.8..4.3).contains(&elapsed) {
                        commands
                            .entity(character)
                            .insert(bevy_als::als_camera::AlsCameraCustomTimeDilation(0.5));
                    } else if (4.3..4.6).contains(&elapsed) {
                        commands
                            .entity(character)
                            .insert(bevy_als::als_camera::AlsCameraCustomTimeDilation(0.0));
                    } else if (4.6..5.0).contains(&elapsed) {
                        commands
                            .entity(character)
                            .insert(bevy_als::als_camera::AlsCameraCustomTimeDilation(2.0));
                    } else {
                        commands
                            .entity(character)
                            .remove::<bevy_als::als_camera::AlsCameraCustomTimeDilation>();
                    }
                }
                if elapsed >= 6.0 {
                    begin_phase(&mut state, IntegrationPhase::Finish);
                }
                return;
            }
            if state.scenario == IntegrationScenario::Visibility {
                if entering {
                    state.notify_roll_armed = false;
                    reset_character(
                        &mut transform,
                        &mut position,
                        rotation,
                        &mut velocity,
                        &mut view_yaw,
                        &mut view_pitch,
                        fixture.locomotion_start,
                        fixture.facing_yaw,
                    );
                    request.locomotion_state = Some(LocomotionState::Grounded);
                    request.stance = Some(Stance::Standing);
                }
                if let Some(avatar) = state.avatar {
                    let phase = visibility_case_phase(elapsed);
                    commands.entity(avatar).insert(
                        if matches!(phase, "hidden" | "montage" | "always_pose") {
                            Visibility::Hidden
                        } else {
                            Visibility::Visible
                        },
                    );
                    if phase == "always_pose" {
                        commands.entity(avatar).insert(
                            bevy_als::animation_visibility::AlsAnimationTickOption::AlwaysTickPose,
                        );
                    } else {
                        commands
                            .entity(avatar)
                            .remove::<bevy_als::animation_visibility::AlsAnimationTickOption>();
                    }
                    if phase == "montage" && elapsed >= 8.1 && !state.notify_roll_armed {
                        input.0.roll_pressed = true;
                        state.notify_roll_armed = true;
                    }
                    if phase == "finish" {
                        begin_phase(&mut state, IntegrationPhase::Finish);
                    }
                }
                return;
            }
            if entering {
                state.notify_roll_armed = false;
                state.notify_roll_restarted = false;
                state.coverage.notify_roll_reentry = false;
                state.notify_mantle_armed = false;
                state.coverage.notify_sprint = false;
                state.coverage.notify_sprint_acceleration = false;
                state.coverage.notify_roll = false;
                state.coverage.notify_marker = false;
                state.coverage.notify_grounded_entry = false;
                state.coverage.notify_root_scale = false;
                state.coverage.notify_rolling_action = false;
                state.coverage.notify_mantling_action = false;
                state.coverage.notify_early_blend_out = false;
                reset_character(
                    &mut transform,
                    &mut position,
                    rotation,
                    &mut velocity,
                    &mut view_yaw,
                    &mut view_pitch,
                    fixture.locomotion_start,
                    fixture.facing_yaw,
                );
                *gait = Gait::Running;
                *overlay = OverlayState::Default;
                *rotation_mode = RotationMode::ViewDirection;
                request.locomotion_state = Some(LocomotionState::Grounded);
                request.stance = Some(Stance::Standing);
            }
            if elapsed < 3.0 {
                input.0.move_dir = Vec2::Y;
                input.0.sprint_held = true;
            } else if elapsed < 6.0 {
                if !state.notify_roll_armed {
                    reset_character_without_rotation(
                        &mut transform,
                        &mut position,
                        &mut velocity,
                        &mut view_yaw,
                        &mut view_pitch,
                        fixture.roll_start,
                        fixture.facing_yaw,
                    );
                    *gait = Gait::Running;
                    request.locomotion_state = Some(LocomotionState::Grounded);
                    request.stance = Some(Stance::Standing);
                    state.notify_roll_armed = true;
                }
                input.0.move_dir = Vec2::Y;
                input.0.roll_pressed = (3.5..3.8).contains(&elapsed);
                if roll_tail_reentry_ready(roll_for_input, *locomotion, state.notify_roll_restarted)
                {
                    input.0.roll_pressed = true;
                    state.notify_roll_restarted = true;
                }
            } else {
                if !state.notify_mantle_armed {
                    reset_character_without_rotation(
                        &mut transform,
                        &mut position,
                        &mut velocity,
                        &mut view_yaw,
                        &mut view_pitch,
                        fixture.mantle_start,
                        fixture.facing_yaw,
                    );
                    *gait = Gait::Running;
                    request.locomotion_state = Some(LocomotionState::Grounded);
                    request.stance = Some(Stance::Standing);
                    state.notify_mantle_armed = true;
                }
                let forward = Quat::from_rotation_y(fixture.facing_yaw) * Vec3::NEG_Z;
                let progress = (transform.translation - fixture.mantle_start).dot(forward);
                input.0.move_dir = Vec2::Y;
                input.0.jump_pressed = (5.85..7.1).contains(&progress) && elapsed < 13.0;
            }
            if elapsed >= 17.0 {
                begin_phase(&mut state, IntegrationPhase::Finish);
            }
        }
        IntegrationPhase::Finish => {}
    }
}

fn reset_character(
    transform: &mut Transform,
    position: &mut Position,
    mut rotation: Option<Mut<Rotation>>,
    velocity: &mut LinearVelocity,
    view_yaw: &mut AlsViewYaw,
    view_pitch: &mut AlsViewPitch,
    translation: Vec3,
    yaw: f32,
) {
    reset_character_with_rotation(
        transform,
        position,
        rotation.as_deref_mut(),
        velocity,
        view_yaw,
        view_pitch,
        translation,
        yaw,
    );
}

fn rebase_temporal_fixture(
    state: &mut ManualIntegrationState,
    phase_evidence: &mut ManualRuntimePhaseEvidence,
    label: &str,
) {
    state.temporal_rebase_epoch = state.temporal_rebase_epoch.wrapping_add(1);
    phase_evidence.temporal_rebase_epoch = state.temporal_rebase_epoch;
    info!(
        "[MANUAL-INTEGRATION][TEMPORAL-REBASE] epoch={} fixture={label}",
        state.temporal_rebase_epoch
    );
}

fn reset_fixture_rotation_response_state(
    rotation_response: &mut AlsRotationResponseState,
    aiming_yaw_constraint: &mut AlsAimingYawConstraint,
    rotation_animation: Option<&mut AlsRotationAnimationState>,
) {
    *rotation_response = AlsRotationResponseState::default();
    *aiming_yaw_constraint = AlsAimingYawConstraint::default();
    if let Some(rotation_animation) = rotation_animation {
        *rotation_animation = AlsRotationAnimationState::default();
    }
}

fn reinitialize_fixture_rotation_response(
    mut rotation_response: Option<Mut<AlsRotationResponseState>>,
    aiming_yaw_constraint: &mut AlsAimingYawConstraint,
    mut rotation_animation: Option<Mut<AlsRotationAnimationState>>,
) {
    if let Some(rotation_response) = rotation_response.as_deref_mut() {
        reset_fixture_rotation_response_state(
            rotation_response,
            aiming_yaw_constraint,
            rotation_animation.as_deref_mut(),
        );
    } else {
        *aiming_yaw_constraint = AlsAimingYawConstraint::default();
        if let Some(rotation_animation) = rotation_animation.as_deref_mut() {
            *rotation_animation = AlsRotationAnimationState::default();
        }
    }
}

fn reset_character_with_rotation(
    transform: &mut Transform,
    position: &mut Position,
    rotation: Option<&mut Rotation>,
    velocity: &mut LinearVelocity,
    view_yaw: &mut AlsViewYaw,
    view_pitch: &mut AlsViewPitch,
    translation: Vec3,
    yaw: f32,
) {
    let rotation_value = Quat::from_rotation_y(yaw);
    transform.translation = translation;
    transform.rotation = rotation_value;
    position.0 = translation;
    if let Some(rotation) = rotation {
        rotation.0 = rotation_value;
    }
    velocity.0 = Vec3::ZERO;
    view_yaw.0 = yaw;
    view_pitch.0 = 0.0;
}

fn reset_character_without_rotation(
    transform: &mut Transform,
    position: &mut Position,
    velocity: &mut LinearVelocity,
    view_yaw: &mut AlsViewYaw,
    view_pitch: &mut AlsViewPitch,
    translation: Vec3,
    yaw: f32,
) {
    transform.translation = translation;
    transform.rotation = Quat::from_rotation_y(yaw);
    position.0 = translation;
    velocity.0 = Vec3::ZERO;
    view_yaw.0 = yaw;
    view_pitch.0 = 0.0;
}

fn stance_center_from_standing_center(
    params: &AlsMovementParams,
    standing_center: Vec3,
    stance: Stance,
    effective_scale: f32,
) -> Vec3 {
    let target_half_height = if stance == Stance::Crouching {
        params.crouch.crouched_capsule_half_height
    } else {
        params.crouch.standing_capsule_half_height
    };
    standing_center
        + Vec3::Y
            * (target_half_height - params.crouch.standing_capsule_half_height)
            * effective_scale
}

fn begin_phase(state: &mut ManualIntegrationState, phase: IntegrationPhase) {
    if phase == IntegrationPhase::Mantle {
        state.coverage.mantle_started = false;
        state.mantle_root_released = false;
        state.coverage.mantle_completed = false;
    }
    state.phase = phase;
    state.phase_started_seconds = state.game_time_seconds;
    state.entered = true;
    info!("[MANUAL-INTEGRATION][PHASE] {phase:?}");
}

fn state_machine_rotation_boundary(elapsed: f32, delta: f32) -> Option<&'static str> {
    [
        (1.3, "state_machine_aim_view_flip"),
        (2.6, "state_machine_first_person_enter"),
        (3.9, "state_machine_first_person_view_flip"),
    ]
    .into_iter()
    .find_map(|(boundary, label)| {
        (elapsed >= boundary && elapsed - delta < boundary).then_some(label)
    })
}

fn state_machine_turn_target_yaw(elapsed: f32, body_yaw: f32) -> f32 {
    if (STATE_MACHINE_TURN_FIRST_START..STATE_MACHINE_TURN_FIRST_END).contains(&elapsed) {
        body_yaw + 90.0f32.to_radians()
    } else if (STATE_MACHINE_TURN_SECOND_START..STATE_MACHINE_TURN_SECOND_END).contains(&elapsed) {
        body_yaw - 90.0f32.to_radians()
    } else {
        body_yaw
    }
}

fn state_machine_turn_phase_complete(elapsed: f32) -> bool {
    elapsed >= STATE_MACHINE_TURN_END
}

fn cleanup_crouch_canopy(commands: &mut Commands, state: &mut ManualIntegrationState) {
    if let Some(entity) = state.crouch_canopy.take() {
        commands.entity(entity).despawn();
        state.coverage.crouch_canopy_cleaned = true;
        info!("[MANUAL-INTEGRATION][CROUCH-CANOPY] cleaned entity={entity:?}");
    }
    state.crouch_canopy_center = None;
    state.crouch_canopy_armed_at = None;
}

fn cleanup_foot_ik_terrain(commands: &mut Commands, state: &mut ManualIntegrationState) {
    let mut cleaned = false;
    if let Some(entity) = state.foot_ik_slope.take() {
        commands.entity(entity).despawn();
        cleaned = true;
    }
    if let Some(entity) = state.foot_ik_step.take() {
        commands.entity(entity).despawn();
        cleaned = true;
    }
    if cleaned {
        state.coverage.foot_ik_terrain_cleaned = true;
        info!("[MANUAL-INTEGRATION][FOOT-IK-TERRAIN] cleaned");
    }
}

fn log_foot_ik_stage(stage: &str, spaces: &AlsFootIkSpaceState, apply: &AlsFootIkApplyTelemetry) {
    info!(
        "[MANUAL-INTEGRATION][FOOT-IK-STAGE] stage={stage} left_target=({:.3},{:.3},{:.3}) left_armature=({:.3},{:.3},{:.3}) left_parent=({:.3},{:.3},{:.3}) left_bone=({:.3},{:.3},{:.3}) left_hip=({:.3},{:.3},{:.3}) left_knee=({:.3},{:.3},{:.3}) left_ankle=({:.3},{:.3},{:.3}) left_errors=({:.6},{:.6},{:.6}) left_pole={:.6} left_weight={:.3} right_target=({:.3},{:.3},{:.3}) right_armature=({:.3},{:.3},{:.3}) right_parent=({:.3},{:.3},{:.3}) right_bone=({:.3},{:.3},{:.3}) right_hip=({:.3},{:.3},{:.3}) right_knee=({:.3},{:.3},{:.3}) right_ankle=({:.3},{:.3},{:.3}) right_errors=({:.6},{:.6},{:.6}) right_pole={:.6} right_weight={:.3}",
        spaces.left.resolved_target_world.x,
        spaces.left.resolved_target_world.y,
        spaces.left.resolved_target_world.z,
        spaces.left.target_armature_local.x,
        spaces.left.target_armature_local.y,
        spaces.left.target_armature_local.z,
        spaces.left.target_parent_local.x,
        spaces.left.target_parent_local.y,
        spaces.left.target_parent_local.z,
        spaces.left.target_bone_local.x,
        spaces.left.target_bone_local.y,
        spaces.left.target_bone_local.z,
        apply.left.hip_world.x,
        apply.left.hip_world.y,
        apply.left.hip_world.z,
        apply.left.knee_world.x,
        apply.left.knee_world.y,
        apply.left.knee_world.z,
        apply.left.ankle_world.x,
        apply.left.ankle_world.y,
        apply.left.ankle_world.z,
        apply.left.hip_error,
        apply.left.knee_error,
        apply.left.effector_error,
        apply.left.pole_sign,
        apply.left.applied_weight,
        spaces.right.resolved_target_world.x,
        spaces.right.resolved_target_world.y,
        spaces.right.resolved_target_world.z,
        spaces.right.target_armature_local.x,
        spaces.right.target_armature_local.y,
        spaces.right.target_armature_local.z,
        spaces.right.target_parent_local.x,
        spaces.right.target_parent_local.y,
        spaces.right.target_parent_local.z,
        spaces.right.target_bone_local.x,
        spaces.right.target_bone_local.y,
        spaces.right.target_bone_local.z,
        apply.right.hip_world.x,
        apply.right.hip_world.y,
        apply.right.hip_world.z,
        apply.right.knee_world.x,
        apply.right.knee_world.y,
        apply.right.knee_world.z,
        apply.right.ankle_world.x,
        apply.right.ankle_world.y,
        apply.right.ankle_world.z,
        apply.right.hip_error,
        apply.right.knee_error,
        apply.right.effector_error,
        apply.right.pole_sign,
        apply.right.applied_weight,
    );
}

fn graph_f32(player: &AnimationGraphPlayer, key: &str) -> Option<f32> {
    match player.get_input_data(key.to_owned()) {
        Some(DataValue::F32(value)) if value.is_finite() => Some(value),
        _ => None,
    }
}

fn graph_global_f32(player: &AnimationGraphPlayer, key: &str) -> Option<f32> {
    match player.get_global_input_data(key) {
        Some(DataValue::F32(value)) if value.is_finite() => Some(*value),
        _ => None,
    }
}

fn graph_dynamic_distance_squared_cm(player: &AnimationGraphPlayer, side: &str) -> Option<f64> {
    let target = player
        .get_global_input_data(&format!("feet_state.{side}.target_location"))?
        .as_vec3()
        .ok()?;
    let locked = player
        .get_global_input_data(&format!("feet_state.{side}.lock_location"))?
        .as_vec3()
        .ok()?;
    Some(dynamic_distance_squared_cm(target, locked))
}

const MANTLE_SLOT_NODE: &str = "Slot 'PostLocomotion'\nGroup 'Locomotion'";

fn observe_standing_window(
    window: &mut ManualAnimationWindowEvidence,
    standing: Option<&AlsStandingGraphTelemetry>,
    locomotion: LocomotionState,
) {
    window.settled &= locomotion == LocomotionState::Grounded;
    window.observed_runtime_state = standing.and_then(|value| value.actual_movement_state);
    window.observed_direction = window
        .observed_runtime_state
        .and_then(movement_direction_from_fsm_index)
        .map(movement_direction_name);
    window.observed_parent_runtime_state = standing.and_then(|value| value.actual_standing_state);
    window.observed_parent_transition_active =
        standing.and_then(|value| value.actual_standing_transition_active);
}

fn foot_ik_matches_source(apply: &AlsFootIkApplyTelemetry, curves: [f32; 2]) -> bool {
    [apply.left, apply.right]
        .into_iter()
        .zip(curves)
        .all(|(side, curve)| {
            side.valid
                && (side.applied_weight - curve.clamp(0.0, 1.0)).abs() <= 1e-5
                && side.effector_error <= 0.02
        })
}

fn mantle_foot_ik_matches_source(
    apply: &AlsFootIkApplyTelemetry,
    curves: [f32; 2],
    slot_weight: Option<f32>,
) -> bool {
    slot_weight.is_some_and(|weight| weight >= 1.0 - 1e-5) && foot_ik_matches_source(apply, curves)
}

fn mantle_slot_sample(
    arena: &bevy_animation_graph::core::context::graph_context_arena::GraphContextArena,
    graphs: &Assets<AnimationGraph>,
) -> Option<bevy_als::extensions::animation_context::SourceRelevantAsset> {
    arena.iter_context_ids().find_map(|id| {
        let context = arena.get_context(id)?;
        let graph = graphs.get(context.get_graph_id())?;
        let node = graph
            .nodes
            .values()
            .find(|node| node.name == MANTLE_SLOT_NODE)?;
        let DataValue::Pose(pose) =
            context
                .node_caches
                .get_output_data(node.id, StateKey::Default, "Pose".into())?
        else {
            return None;
        };
        if pose.paths.is_empty() {
            return None;
        }
        let sample = context
            .node_states
            .get::<bevy_als::extensions::AnimationContextState>(node.id, StateKey::Default)
            .ok()?
            .relevant_asset
            .as_ref()?;
        Some(sample.clone())
    })
}

fn observe_mantle(
    state: &mut ManualIntegrationState,
    window: &mut ManualAnimationWindowEvidence,
    mantle: &AlsMantleState,
    montages: Option<&AlsMantleMontages>,
    player: &AnimationGraphPlayer,
    graphs: &Assets<AnimationGraph>,
    locomotion: LocomotionState,
    action: Option<AlsNotifyLocomotionAction>,
    position: Vec3,
) {
    let settings = bevy_als::mantle_drive::mantle_montage_settings(mantle.kind);
    let montage = montages.and_then(|montages| {
        montages
            .entries
            .iter()
            .rev()
            .find(|entry| entry.kind == mantle.kind && entry.pose_ready)
    });
    let sample = player
        .get_context_arena()
        .and_then(|arena| mantle_slot_sample(arena, graphs))
        .filter(|sample| {
            montage.is_some_and(|entry| {
                (sample.position - entry.clip_time).abs() < 1e-5
                    && (sample.duration - settings.sequence_length).abs() < 1e-5
                    && (sample.weight - entry.weight).abs() < 1e-5
            })
        });
    let playing = mantle.active
        && mantle.root_motion_ready
        && sample.is_some()
        && locomotion == LocomotionState::Custom
        && action == Some(AlsNotifyLocomotionAction::Mantling);
    state.coverage.mantle_started |= playing;
    state.mantle_root_released |= state.coverage.mantle_started
        && !mantle.active
        && !mantle.root_motion_ready
        && locomotion == LocomotionState::Grounded
        && action == Some(AlsNotifyLocomotionAction::None)
        && position.y >= mantle.target.y - 0.3;
    state.coverage.mantle_completed |=
        state.mantle_root_released && montages.is_none_or(|montages| montages.entries.is_empty());
    if state.phase != IntegrationPhase::Mantle
        && !(state.phase == IntegrationPhase::MovingBase
            && state.moving_base_probe.mantle_requested)
    {
        return;
    }
    let variant = montage.and_then(|_| settings.source_sequence.rsplit('.').next());
    *window = ManualAnimationWindowEvidence {
        label: Some("mantle"),
        settled: sample.is_some(),
        capture_source_window: true,
        capture_every_frame: true,
        requires_pre_rig_motion: true,
        requires_final_pose_motion: true,
        requires_non_reference_root_pose: true,
        requires_source_pose_chain: true,
        source_node: Some(MANTLE_SLOT_NODE),
        source_state: Some("Mantling"),
        source_pose_chain: vec![ManualAnimationPoseBoundary {
            label: "post_locomotion_slot",
            graph: "ALS/Character/AB_Als/AnimGraph.animgraph.ron",
            node: MANTLE_SLOT_NODE,
            output: "Pose",
            minimum_translation_span: 0.0,
            minimum_rotation_span_radians: 0.0,
        }],
        expected_animation_variant: variant,
        observed_animation_variant: sample.as_ref().and(variant),
        observed_animation_clip_time: sample.as_ref().map(|sample| sample.position),
        observed_animation_weight: sample.as_ref().map(|sample| sample.weight),
        observed_animation_play_rate: sample.as_ref().map(|_| settings.play_rate),
        expected_lifecycle_stages: &["playing", "root_released", "montage_tail", "completed"],
        observed_lifecycle_stages: if state.coverage.mantle_completed {
            &["completed"]
        } else if state.mantle_root_released && sample.is_some() {
            &["root_released", "montage_tail"]
        } else if playing {
            &["playing"]
        } else {
            &[]
        },
        canonical_clip_time: Some((mantle.start_offset + settings.sequence_length) * 0.5),
        maximum_clip_time_distance: 0.06,
        minimum_observation_seconds: 0.15,
        ..default()
    };
}

fn default_overlay_curve_value(name: &str) -> f32 {
    match name {
        "HandLeftIk" | "HandRightIk" | "HipsDirectionLock" | "LayerHandLeft" | "LayerHandRight"
        | "PoseAiming" => 0.0,
        _ => 1.0,
    }
}

fn default_overlay_curves_ready(context: Option<&AlsRootAnimationContext>) -> bool {
    context.is_some_and(|context| {
        ALS_LAYER_CURVE_NAMES.iter().all(|name| {
            let value = context
                .curves
                .get(&bevy_als::extensions::SourceCurveName::from(*name))
                .copied()
                .unwrap_or(0.0);
            (value - default_overlay_curve_value(name)).abs() < 1e-4
        })
    })
}

fn grounded_roll_exit_transition_matches_source(
    phase: IntegrationPhase,
    requested_stance: Option<Stance>,
    roll: Option<&AlsRollState>,
    playback: Option<&AlsTransitionPlayback>,
    slot: Option<&AlsRootTransitionSlotRuntime>,
    grounded: Option<&AlsGroundedGraphTelemetry>,
) -> bool {
    let Some(stance) = requested_stance else {
        return false;
    };
    let expected = AlsTransitionPlayback::roll_to_grounded_right(stance == Stance::Crouching);
    phase == IntegrationPhase::Roll
        && roll.is_some()
        && playback.is_some_and(|playback| {
            playback.active
                && playback.kind == expected.kind
                && playback.index == expected.index
                && (playback.blend_in - STOP_TRANSITION_BLEND_IN).abs() <= 1e-6
                && (playback.blend_out - STOP_TRANSITION_BLEND_OUT).abs() <= 1e-6
                && (playback.play_rate - STOP_TRANSITION_PLAY_RATE).abs() <= 1e-6
                && (playback.start_time - RIGHT_TRANSITION_START_TIME).abs() <= 1e-6
        })
        && slot.is_some_and(|slot| slot.active && slot.pose_ready && slot.weight > 1e-4)
        && grounded.is_some_and(|grounded| {
            grounded.pose_ready == Some(true)
                && (grounded.transition_active == Some(true)
                    || matches!(grounded.actual_state, Some(1 | 2))
                    || (stance == Stance::Crouching && grounded.actual_state == Some(4)))
        })
}

fn grounded_roll_exit_diagnostic(
    stance: Stance,
    desired_stance: AlsDesiredStance,
    roll: Option<&AlsRollState>,
    grounded: Option<&AlsGroundedGraphTelemetry>,
    playback: Option<&AlsTransitionPlayback>,
    slot: Option<&AlsRootTransitionSlotRuntime>,
) -> String {
    let roll = roll.map_or_else(
        || "missing".to_owned(),
        |roll| {
            format!(
                "active={} time={:.6} stance_before={:?}",
                roll.active, roll.time, roll.stance_before_roll
            )
        },
    );
    let grounded = grounded.map_or_else(
        || "missing".to_owned(),
        |grounded| {
            format!(
                "state={:?} transition={:?} pose_ready={:?}",
                grounded.actual_state, grounded.transition_active, grounded.pose_ready
            )
        },
    );
    let playback = playback.map_or_else(
        || "missing".to_owned(),
        |playback| {
            format!(
                "active={} kind={:?} weight={:.6} blend_in={:.6} blend_out={:.6} play_rate={:.6} start_time={:.6}",
                playback.active,
                playback.kind,
                playback.weight,
                playback.blend_in,
                playback.blend_out,
                playback.play_rate,
                playback.start_time,
            )
        },
    );
    let slot = slot.map_or_else(
        || "missing".to_owned(),
        |slot| {
            format!(
                "active={} pose_ready={} weight={:.6}",
                slot.active, slot.pose_ready, slot.weight
            )
        },
    );
    format!(
        "stance={stance:?}; desired_stance={:?}; roll={roll}; grounded={grounded}; transition={playback}; slot={slot}",
        desired_stance.0
    )
}

fn manual_gate_exit_ready(
    visual_progress: Option<&AnimationVisualValidationProgress>,
    timed_out: bool,
) -> bool {
    timed_out || visual_progress.map_or(true, |progress| progress.evidence_ready)
}

fn p1_turn_evidence_mask(drive: &AlsTurnDrive, standing: Option<&AlsStandingGraphTelemetry>) -> u8 {
    let turn = if drive.playing && drive.weight > 1e-4 {
        if drive.lr <= 0.5 { 1 } else { 2 }
    } else {
        0
    };
    let rotate = match standing
        .filter(|graph| {
            graph.actual_standing_pose_ready == Some(true)
                && graph.actual_standing_transition_active == Some(false)
        })
        .and_then(|graph| graph.actual_standing_state)
    {
        Some(3) => 4,
        Some(4) => 8,
        _ => 0,
    };
    turn | rotate
}

fn movement_pose_sample(pose: &Pose, skeletons: &Assets<Skeleton>) -> Vec<serde_json::Value> {
    let Some(skeleton) = skeletons.get(&pose.skeleton) else {
        return Vec::new();
    };
    skeleton.iter_bones().filter_map(|id| {
        let path = skeleton.id_to_path(id)?;
        let name = path.parts.last()?.as_str();
        if !["pelvis", "thigh_l", "calf_l", "foot_l", "thigh_r", "calf_r", "foot_r"].contains(&name) { return None; }
        let bone = pose.get_bone(id)?;
        Some(serde_json::json!({ "name": name, "rotation": bone.rotation.map(|value| value.to_array()), "translation": bone.translation.map(|value| value.to_array()) }))
    }).collect()
}

#[allow(clippy::type_complexity)]
fn observe_manual_integration(
    fixture: Option<Res<ManualIntegrationFixture>>,
    mut observation: ManualObservationResources,
    real_time: Res<Time<Real>>,
    mut animation_events: MessageReader<AlsAnimationEvent>,
    interface: InterfaceProbe,
    core: Query<
        (
            &Transform,
            &LinearVelocity,
            &Stance,
            &Gait,
            &LocomotionState,
            &OverlayState,
            &AlsViewYaw,
            &AlsViewPitch,
            Has<AlsFirstPersonView>,
            Has<AlsCameraLeftShoulder>,
            Option<&StartStopState>,
            Option<&AlsControlRigState>,
            Option<&AlsHeadState>,
            Option<&AlsLayeringState>,
            Option<&AlsOverlayObjectState>,
        ),
        With<AlsCharacter>,
    >,
    mut state_requests: Query<&mut AlsStateRequest, With<AlsCharacter>>,
    control_rig_runtime: Query<(&AlsControlRigExecution, &AlsControlRigInput), With<AlsCharacter>>,
    motion_probe: MotionProbe,
    foot_ik_data: Query<
        (
            &AlsFootIkTelemetry,
            &AlsFootIkSpaceState,
            &AlsFootIkApplyTelemetry,
            Option<&AlsFootLockTeleportTelemetry>,
        ),
        With<AlsCharacter>,
    >,
    action: Query<
        (
            Option<&AlsMantleState>,
            Option<&AlsRollState>,
            Option<&AlsRollSlotRuntime>,
            Option<&AlsFootLockState>,
            Option<&AlsMovementBaseDynamicsState>,
            Option<&AlsRagdollGroundFollow>,
            Option<&AlsRagdollJointDrive>,
            Option<&AlsNotifyLocomotionAction>,
            Option<&AlsNotifyRootMotionScale>,
            Option<&AlsNotifyEarlyBlendOut>,
            Option<&AlsGroundedEntryMode>,
            Option<&AlsTransitionMontageStack>,
            Option<&AlsRootTransitionSlotRuntime>,
            Option<&AlsDesiredStance>,
            Option<&AlsMantleMontages>,
        ),
        With<AlsCharacter>,
    >,
    avatars: Query<(
        &AlsCharacterAvatar,
        &AnimatedSceneInstance,
        Option<&AlsGaitBlend>,
        Option<&AlsStanceBlend>,
        Option<&AlsJumpVariantState>,
        Option<&AlsTurnDrive>,
        Option<&AlsTurnMontageStack>,
        Option<&AlsStandingTurnSlotRuntime>,
        Option<&AlsMovementDirectionState>,
    )>,
    players: Query<(&AnimationGraphPlayer, Option<&AlsRootAnimationContext>)>,
    ragdoll_probe: RagdollPaProbe,
    cameras: Query<(&Transform, &AlsCameraDebugInfo), With<AlsCameraRig>>,
    mut exit: MessageWriter<AppExit>,
) {
    let (state, mut animation_window, gate_report) = (
        &mut observation.state,
        &mut observation.animation_window,
        &mut observation.gate_report,
    );
    if !state.initialized {
        return;
    }
    state.game_time_seconds = interface.virtual_time.elapsed_secs_f64();
    let Some(fixture) = fixture else {
        return;
    };
    let Some(character) = state.character else {
        return;
    };
    let Ok((
        transform,
        velocity,
        stance,
        gait,
        locomotion,
        overlay,
        view_yaw,
        view_pitch,
        first_person,
        left_shoulder,
        start_stop,
        control_rig,
        head,
        layering,
        overlay_object,
    )) = core.get(character)
    else {
        return;
    };
    let rotation_animation = motion_probe
        .rotation_animation
        .get(character)
        .ok()
        .flatten();
    let standing_graph = motion_probe.standing.get(character).ok();
    let crouching_graph = motion_probe.crouching_graph.get(character).ok();
    let grounded_graph = motion_probe.grounded.get(character).ok();
    let crouch_transaction = motion_probe.crouch.get(character).ok();
    state.coverage.crouch_scaled |= state.phase == IntegrationPhase::CrouchClearance
        && state.coverage.crouch_canopy_blocked
        && transform.scale.abs().min_element() > 1.99;
    if matches!(
        state.scenario,
        IntegrationScenario::Base | IntegrationScenario::StateMachine
    ) && state.phase == IntegrationPhase::CrouchRecovery
        && state.phase_elapsed() >= 1.0
        && state.crouch_recovery_jump_requested
        && !state.crouch_recovery_air_crouch_requested
        && *locomotion == LocomotionState::Air
        && let Ok(mut request) = state_requests.get_mut(character)
    {
        request.stance = Some(Stance::Crouching);
        state.crouch_recovery_air_crouch_requested = true;
        info!(
            "[MANUAL-INTEGRATION][CROUCH-AIR-REQUEST] requested=Crouching actual_locomotion={:?}",
            locomotion,
        );
    }
    let Ok((
        mantle,
        roll,
        roll_slot,
        foot_lock,
        movement_base,
        ragdoll_follow,
        joint_drive,
        notify_action,
        notify_root_scale,
        notify_early_blend_out,
        grounded_entry,
        transition_stack,
        root_transition_slot,
        desired_stance,
        mantle_montages,
    )) = action.get(character)
    else {
        return;
    };
    let transition_playback = transition_stack.and_then(AlsTransitionMontageStack::latest);
    let Ok(assembly) = interface.camera_assemblies.get(character) else {
        return;
    };
    let Ok((
        _,
        instance,
        gait_blend,
        stance_blend,
        jump_variant,
        turn_drive,
        turn_stack,
        turn_slot,
        movement_direction,
    )) = avatars.get(assembly.avatar)
    else {
        return;
    };
    let Ok((player, context)) = players.get(instance.player_entity()) else {
        return;
    };
    let speed = Vec2::new(velocity.0.x, velocity.0.z).length();
    let actual_motion = motion_probe.actual.get(character).ok();
    let gait_state = motion_probe.gait.get(character).ok();
    let direction = graph_f32(player, "direction_angle");
    let locomotion_phase = |key| {
        player.get_context_arena().and_then(|arena| {
            bevy_als::extensions::sync_group_phase::source_sync_phase(arena, &motion_probe.graphs, key)
        })
    };
    let jump_phase = locomotion_phase("sync_groups.jump.phase");
    let land_phase = locomotion_phase("sync_groups.land.phase");
    let overlay_tag = motion_probe
        .source_inputs
        .get(instance.player_entity())
        .ok()
        .and_then(|inputs| inputs.gameplay_tag_input("overlay_mode"));
    let source_ready = matches!(player.get_animation_source(), AnimationSource::Graph(_))
        && player.get_error().is_none();
    record_animation_window_trace(
        &mut animation_window,
        transform,
        view_yaw.0,
        rotation_animation,
        crouching_graph,
        turn_drive,
        turn_stack,
        *stance == Stance::Crouching,
        None,
    );
    if matches!(
        state.phase,
        IntegrationPhase::DirectionRegression | IntegrationPhase::StateMachineGrounded
    ) {
        observe_standing_window(&mut animation_window, standing_graph, *locomotion);
    } else if state.phase == IntegrationPhase::CrouchDirectionRegression {
        animation_window.observed_runtime_state =
            crouching_graph.and_then(|value| value.actual_movement_state);
        animation_window.observed_direction = animation_window
            .observed_runtime_state
            .and_then(movement_direction_from_fsm_index)
            .map(movement_direction_name);
        animation_window.observed_parent_runtime_state =
            crouching_graph.and_then(|value| value.actual_outer_state);
        animation_window.observed_parent_transition_active =
            crouching_graph.and_then(|value| value.actual_outer_transition_active);
        animation_window.observed_requested_parent_runtime_state = None;
        animation_window.observed_requested_parent_transition_active = None;
    } else if state.scenario == IntegrationScenario::Grounded
        && state.phase == IntegrationPhase::Roll
    {
        animation_window.observed_runtime_state =
            grounded_graph.and_then(|value| value.actual_state);
    }
    if !source_ready {
        state.coverage.graph = false;
    }
    if state.scenario == IntegrationScenario::LocomotionActions
        && state.phase == IntegrationPhase::LocomotionActions
        && !state.entered
    {
        observe_locomotion_actions(
            state,
            &mut animation_window,
            gate_report,
            player,
            context,
            &motion_probe,
            character,
            transform,
            *locomotion,
            *stance,
        );
    }
    if state.scenario == IntegrationScenario::Grounded && !state.entered {
        let notify = motion_probe.transition_notify.get(character).ok().flatten();
        let mut sequences = Vec::new();
        let mut weights = Vec::new();
        if let Some(arena) = player.get_context_arena() {
            for id in arena.iter_context_ids() {
                let Some(child) = arena.get_context(id) else {
                    continue;
                };
                let Some(graph) = motion_probe.graphs.get(child.get_graph_id()) else {
                    continue;
                };
                for node in graph.nodes.values() {
                    if !child.node_caches.is_updated(node.id, StateKey::Default) {
                        continue;
                    }
                    let variant = match node.name.as_str() {
                        "A_Als_StandToCrouch\nSequence Player" => Some("A_Als_StandToCrouch"),
                        "A_Als_CrouchToStand\nSequence Player" => Some("A_Als_CrouchToStand"),
                        _ => None,
                    };
                    if let Some(variant) = variant
                        && let Some(sequence) = node
                            .try_inner_downcast_ref::<bevy_als::extensions::SourceSequencePlayerNode>()
                        && let Some(clip) = motion_probe.clips.get(&sequence.clip)
                    {
                        let position = child.node_states.get_time(node.id, StateKey::Default);
                        sequences.push((
                            variant,
                            position,
                            clip.duration(),
                            sequence.loop_animation,
                            sequence.play_rate,
                            sequence
                                .curves
                                .sample_all(position)
                                .get(&bevy_als::extensions::SourceCurveName::from("PoseCrouching"))
                                .copied(),
                        ));
                    }
                    if node.name == "Grounded States\nState Machine"
                        && let Some(fsm) =
                            node.try_inner_downcast_ref::<bevy_als::extensions::InterruptibleFsmNode>()
                        && let Some(machine) = motion_probe.machines.get(&fsm.fsm)
                        && let Ok(runtime) = child
                            .node_states
                            .get::<bevy_als::extensions::InterruptibleFsmState>(
                            node.id,
                            StateKey::Default,
                        )
                    {
                        weights.extend(machine.states.values().map(|entry| {
                            (
                                entry.label.clone(),
                                runtime.recorded_state_weight(&entry.id.uuid().to_string()),
                            )
                        }));
                    }
                }
            }
        }
        let avatar_space = state
            .avatar
            .and_then(|avatar| motion_probe.avatar_space.get(avatar).ok());
        let mesh_base_offset = avatar_space
            .zip(crouch_transaction)
            .map(|((local, _), capsule)| local.translation.y + capsule.collider_half_height);
        let actual_state = grounded_graph.and_then(|value| value.actual_state);
        let transition_active = grounded_graph.and_then(|value| value.transition_active);
        state.coverage.grounded_stop_callback |=
            notify.is_some_and(|value| value.stop_animations_count > 0);
        if state.phase == IntegrationPhase::Roll {
            let requested_stance = notify.and_then(|value| value.roll_to_grounded_stance);
            let playback = match state.grounded_roll_playback {
                Some(recorded) => transition_stack.and_then(|stack| {
                    stack
                        .entries
                        .iter()
                        .find(|entry| entry.instance_id == recorded.instance_id)
                }),
                None => transition_playback,
            };
            let active = grounded_roll_exit_transition_matches_source(
                state.phase,
                requested_stance,
                roll,
                playback,
                root_transition_slot,
                grounded_graph,
            );
            if active {
                state.grounded_roll_playback = playback.copied();
            }
            let terminal_stance = desired_stance.map_or(*stance, |value| value.0);
            let terminal_state = if terminal_stance == Stance::Crouching {
                2
            } else {
                1
            };
            let completed = state.grounded_roll_playback.is_some()
                && roll.is_some_and(|value| !value.active)
                && playback.is_none_or(|value| !value.active)
                && root_transition_slot.is_some_and(|slot| !slot.active)
                && actual_state == Some(terminal_state)
                && transition_active == Some(false);
            state.coverage.grounded_roll_completed |= completed;
            if state.scenario == IntegrationScenario::Grounded
                && grounded_roll_exit_timed_out(
                    state.phase_elapsed(),
                    state.coverage.grounded_roll_completed,
                )
            {
                gate_report.direction_trace.push(serde_json::json!({"watchdog": {
                    "exit_reason": "source_transition_completion_timeout", "last_progress": "Roll exit",
                    "waited_game_seconds": state.phase_elapsed(), "no_progress_limit_seconds": 20.0,
                    "current_conditions": state.grounded_roll_exit_diagnostic
                }}));
                state.timed_out = true;
                begin_phase(state, IntegrationPhase::Finish);
            }
            if let Some(recorded) = state.grounded_roll_playback {
                **animation_window = ManualAnimationWindowEvidence {
                    label: Some("grounded_roll_exit"),
                    settled: true,
                    capture_source_window: active || completed,
                    capture_every_frame: true,
                    requires_pre_rig_motion: true,
                    requires_final_pose_motion: true,
                    requires_non_reference_root_pose: true,
                    requires_source_pose_chain: true,
                    source_node: Some("AB_Als_Grounded/PlayRollToGroundedTransitionAnimation"),
                    source_state: Some("Roll exit"),
                    source_pose_chain: vec![ManualAnimationPoseBoundary {
                        label: "root_transition_slot",
                        graph: "ALS/Character/AB_Als/AnimGraph.animgraph.ron",
                        node: "Slot 'Transition'\nGroup 'Grounded'",
                        output: "Pose",
                        minimum_translation_span: 0.0,
                        minimum_rotation_span_radians: 0.0,
                    }],
                    expected_animation_variant: Some(bevy_als::stop_link::source_transition_sequence(
                        &recorded,
                    )),
                    observed_animation_variant: playback
                        .map(bevy_als::stop_link::source_transition_sequence),
                    observed_animation_clip_time: playback
                        .filter(|_| active)
                        .map(|value| value.clip_time),
                    observed_animation_weight: playback.map(|value| value.weight),
                    observed_animation_play_rate: playback.map(|value| value.play_rate),
                    expected_lifecycle_stages: &["playing", "completed", "settled"],
                    observed_lifecycle_stages: if completed {
                        &["completed", "settled"]
                    } else if active {
                        &["playing"]
                    } else {
                        &[]
                    },
                    expected_terminal_parent_runtime_state: Some(terminal_state),
                    expected_terminal_parent_transition_active: Some(false),
                    observed_parent_runtime_state: actual_state,
                    observed_parent_transition_active: transition_active,
                    canonical_clip_time: Some(if recorded.is_crouching() {
                        recorded.start_time + recorded.play_rate * recorded.blend_out
                    } else {
                        0.8
                    }),
                    maximum_clip_time_distance: 0.06,
                    ..default()
                };
            }
        } else if state.phase == IntegrationPhase::GroundedJoint {
            let index = state.grounded_joint_probe.case_index;
            let (label, target, moving, aiming) = GROUNDED_JOINT_CASES[index];
            let target_state = if target == Stance::Crouching { 2 } else { 1 };
            let initial = if aiming {
                target
            } else if target == Stance::Crouching {
                Stance::Standing
            } else {
                Stance::Crouching
            };
            let initial_state = if initial == Stance::Crouching { 2 } else { 1 };
            let native_moving = player
                .get_global_input_data("locomotion_state.b_moving")
                .and_then(|value| value.as_bool().ok());
            let resets = notify.map_or(0, |value| value.grounded_entry_reset_count);
            let game_time = state.game_time_seconds;
            if state.grounded_joint_probe.triggered_at.is_none() {
                let ready = *stance == initial
                    && actual_state == Some(initial_state)
                    && transition_active == Some(false)
                    && native_moving == Some(moving)
                    && if moving { speed > 0.5 } else { speed < 0.01 };
                if ready {
                    state
                        .grounded_joint_probe
                        .ready_since
                        .get_or_insert(game_time);
                } else {
                    state.grounded_joint_probe.ready_since = None;
                }
                state.grounded_joint_probe.reset_start = resets;
            } else {
                let elapsed = (game_time - state.grounded_joint_probe.triggered_at.unwrap()) as f32;
                let probe = &mut state.grounded_joint_probe;
                if let Some(actual) = actual_state {
                    probe.state_mask |= 1 << actual;
                }
                let expected_variant = (index < 2).then_some(if index == 0 {
                    "A_Als_StandToCrouch"
                } else {
                    "A_Als_CrouchToStand"
                });
                let sample = sequences
                    .iter()
                    .find(|value| Some(value.0) == expected_variant);
                probe.record(
                    mesh_base_offset,
                    sample.map(|value| (value.1, value.2, value.3, value.4)),
                    interface.virtual_time.delta_secs(),
                );
                if moving {
                    probe.valid &=
                        sequences.is_empty() && probe.state_mask & ((1 << 3) | (1 << 4)) == 0;
                }
                let terminal = *stance == target
                    && actual_state == Some(target_state)
                    && transition_active == Some(false);
                let yaw = world_forward_to_yaw(*transform.forward());
                let following = motion_probe.rotation_mode.get(character).ok()
                    == Some(&RotationMode::Aiming)
                    && if index >= 6 {
                        elapsed >= 4.5
                            && speed < 0.01
                            && standing_turn_rotation_settled(
                                7,
                                bevy_als::als::motion::wrap_yaw_radians(view_yaw.0 - yaw).to_degrees(),
                                rotation_animation
                                    .map_or(f32::NAN, |value| value.yaw_speed_degrees),
                                turn_drive.is_none_or(|value| value.rotate_active),
                            )
                    } else {
                        speed > 0.5
                            && bevy_als::als::motion::wrap_yaw_radians(yaw - probe.base_yaw)
                                * if index == 4 { -1.0 } else { 1.0 }
                                > 0.5
                            && bevy_als::als::motion::wrap_yaw_radians(view_yaw.0 - yaw).abs()
                                < 3.0_f32.to_radians()
                    };
                let completed = probe.valid
                    && terminal
                    && elapsed >= if aiming { 3.2 } else { 1.5 }
                    && if aiming {
                        following
                    } else {
                        resets > probe.reset_start && (moving || probe.sequence_completed)
                    };
                let base_yaw = probe.base_yaw;
                state.coverage.grounded_joint[index] |= completed;
                let capture = sample.is_some() || moving || (aiming && elapsed >= 0.5) || completed;
                let source_state = if index == 0 {
                    "Standing To Crouching"
                } else if index == 1 {
                    "Crouching To Standing"
                } else if target == Stance::Crouching {
                    "Crouching"
                } else {
                    "Standing"
                };
                **animation_window =
                    ManualAnimationWindowEvidence {
                        label: Some(label),
                        index: Some(index),
                        settled: true,
                        capture_source_window: capture,
                        capture_every_frame: true,
                        requires_pre_rig_motion: true,
                        requires_final_pose_motion: true,
                        requires_non_reference_root_pose: true,
                        requires_source_pose_chain: true,
                        source_node: Some("AB_Als_Grounded/Grounded States"),
                        source_state: Some(source_state),
                        source_pose_chain: grounded_joint_source_chain(),
                        expected_animation_variant: expected_variant,
                        observed_animation_variant: sample.map(|value| value.0),
                        observed_animation_clip_time: sample.map(|value| value.1),
                        observed_animation_weight: sample.and_then(|_| {
                            weights
                                .iter()
                                .find(|(label, _)| label.as_str() == source_state)
                                .map(|(_, weight)| *weight)
                        }),
                        observed_animation_play_rate: sample.map(|value| value.4),
                        expected_runtime_state: if index < 2 {
                            Some(if index == 0 { 3 } else { 4 })
                        } else {
                            Some(target_state)
                        },
                        observed_runtime_state: if index < 2 {
                            sample.and(
                                actual_state
                                    .filter(|value| *value == if index == 0 { 3 } else { 4 }),
                            )
                        } else {
                            actual_state.filter(|value| *value == target_state)
                        },
                        expected_terminal_parent_runtime_state: Some(target_state),
                        expected_terminal_parent_transition_active: Some(false),
                        observed_parent_runtime_state: actual_state,
                        observed_parent_transition_active: transition_active,
                        expected_lifecycle_stages: &["active", "settled"],
                        observed_lifecycle_stages: if completed {
                            &["settled"]
                        } else if capture {
                            &["active"]
                        } else {
                            &[]
                        },
                        canonical_clip_time: (index < 2).then_some(0.4),
                        maximum_clip_time_distance: 0.04,
                        direction_signature_group: (index < 2).then_some("grounded_stance_changes"),
                        required_direction_signature_members: 2,
                        expected_direction: (index < 2).then_some(if target == Stance::Crouching {
                            "Crouching"
                        } else {
                            "Standing"
                        }),
                        observed_direction: sample.map(|_| {
                            if target == Stance::Crouching {
                                "Crouching"
                            } else {
                                "Standing"
                            }
                        }),
                        minimum_direction_signature_rotation_radians: 1e-4,
                        requires_actor_yaw_motion: aiming,
                        actor_yaw_driver: (aiming && index < 6).then_some("locomotion"),
                        observed_animation_yaw_speed_degrees: rotation_animation
                            .map(|value| value.yaw_speed_degrees),
                        expected_actor_yaw_delta_sign: (aiming && index < 6)
                            .then_some(if index == 4 { -1 } else { 1 }),
                        expected_actor_yaw_baseline: aiming.then_some(base_yaw),
                        minimum_actor_yaw_delta_radians: if aiming { 0.5 } else { 0.0 },
                        observed_actor_yaw: Some(yaw),
                        observed_view_yaw: Some(view_yaw.0),
                        ..default()
                    };
            }
        }
        gate_report.direction_trace.push(serde_json::json!({
            "elapsed": state.phase_elapsed(), "phase": format!("{:?}",state.phase), "case": state.grounded_joint_probe.case_index,
            "stance": format!("{stance:?}"), "locomotion": format!("{locomotion:?}"), "desired_stance": desired_stance.map(|value|format!("{:?}",value.0)),
            "actor_position": transform.translation.to_array(), "grounded_entry": grounded_entry.map(|value|format!("{value:?}")),
            "state": actual_state, "transition_active": transition_active, "state_weights": weights, "sequences": sequences,
            "speed": speed, "actor_y": transform.translation.y, "mesh_y": avatar_space.map(|(_,global)|global.translation().y),
            "mesh_local_y": avatar_space.map(|(local,_)|local.translation.y), "capsule_half_height": crouch_transaction.map(|value|value.collider_half_height),
            "mesh_base_offset": mesh_base_offset, "body_yaw": world_forward_to_yaw(*transform.forward()), "view_yaw": view_yaw.0,
            "input": motion_probe.inputs.get(character).ok().and_then(|(_,value)|value).map(|value|(value.0.aim_held,value.0.move_dir.to_array(),value.0.look_delta.to_array())),
            "rotation_mode": motion_probe.rotation_mode.get(character).ok().map(|value|format!("{value:?}")),
            "reset_count": notify.map(|value|value.grounded_entry_reset_count), "stop_count": notify.map(|value|value.stop_animations_count),
            "roll_request_count": notify.map(|value|value.roll_to_grounded_count), "roll_request_stance": notify.and_then(|value|value.roll_to_grounded_stance).map(|value|format!("{value:?}")),
            "root_motion_active": roll.map(AlsRollState::root_motion_active),
            "moving_smooth": player.get_global_input_data("locomotion_state.b_moving_smooth").and_then(|value|value.as_bool().ok()),
            "rotation_yaw_speed": rotation_animation.map(|value|value.yaw_speed_degrees),
            "rotate_active": turn_drive.map(|value|value.rotate_active),
            "roll": roll.map(|value|(value.active,value.time)), "playback": transition_playback.map(|value|(bevy_als::stop_link::source_transition_sequence(value),value.active,value.clip_time,value.weight)),
            "joint_valid": state.grounded_joint_probe.valid, "joint_sequence_completed": state.grounded_joint_probe.sequence_completed,
        }));
    }
    if state.scenario == IntegrationScenario::Head
        && state.phase == IntegrationPhase::Head
        && !state.entered
    {
        let now = state.game_time_seconds;
        let elapsed = state.phase_elapsed();
        let probe = &mut state.head_probe;
        let mut sample = head_graph_sample(player, &motion_probe, probe.previous.as_ref());
        if let Ok(world) = motion_probe.transforms.compute_global_transform(character) {
            if !head_actor_world_inputs_match(player, transform, &world) {
                sample
                    .errors
                    .push("Head parent fixture or actor world inputs differ".into());
            }
        } else {
            sample
                .errors
                .push("Head actor world transform is unavailable".into());
        }
        sample.elapsed_seconds = elapsed;
        let actual_state = bevy_als::head_layer::head_state_from_player(player);
        if head.is_none_or(|head| *head != actual_state) {
            sample
                .errors
                .push("Head Actor projection differs from evaluated instance".into());
        }
        probe.previous = Some((
            sample.frame,
            actual_state,
            sample.refreshed,
            bevy_als::head_layer::head_blend_amount(context),
        ));
        probe.valid &= sample.errors.is_empty();
        for (index, time) in sample.times.iter().enumerate() {
            probe.samples_seen[index] |= time.is_some();
        }
        probe.additive_seen |= sample.additive_effect.is_some_and(|effect| effect > 1e-4);
        let real_delta = real_time.delta_secs();
        let custom_clock = probe.case_index == 5
            && real_delta > 1e-5
            && (sample.inputs[0] - interface.virtual_time.delta_secs() * 0.5).abs() < 1e-4
            && (sample.inputs[1] - real_delta).abs() <= real_delta * 0.25;
        probe.unscaled_pitch_seen |= first_person
            && custom_clock
            && sample.inputs[1] > sample.inputs[0] * 3.5
            && (sample.state[0] - sample.inputs[7]).abs() > 0.01
            && sample.errors.is_empty();
        let index = probe.case_index;
        let branch_ready = match index {
            0..=2 => {
                (sample.state[0] - [-60.0, 0.0, 60.0][index]).abs() < 5.0
                    && !first_person
                    && (index != 2
                        || motion_probe.rotation_mode.get(character).ok()
                            == Some(&RotationMode::Aiming)
                            && motion_probe
                                .inputs
                                .get(character)
                                .ok()
                                .and_then(|(_, input)| input)
                                .is_some_and(|input| !input.0.aim_held))
            }
            3 => sample.modes[1] && sample.modes[2] && speed > 0.5,
            4 => sample.modes[1] && !sample.modes[2] && speed < 0.1,
            5 => first_person && probe.unscaled_pitch_seen,
            _ => !first_person,
        };
        if source_ready
            && sample.ready
            && probe.valid
            && *overlay == OverlayState::Default
            && *stance == Stance::Standing
            && *locomotion == LocomotionState::Grounded
            && branch_ready
        {
            **animation_window = head_window(index, &sample);
            probe.seconds[index] += sample.inputs[0];
            probe.covered[index] |= probe.seconds[index] >= 0.8;
        }
        gate_report.direction_trace.push(serde_json::json!({"case": index, "time": now, "covered": probe.covered, "valid": probe.valid,
            "first_person": first_person, "speed": speed, "unscaled_pitch_seen": probe.unscaled_pitch_seen, "source": sample}));
        probe.sample = sample;
    }
    if state.scenario == IntegrationScenario::Overlays
        && state.phase == IntegrationPhase::Overlays
        && !state.entered
    {
        let rotation = motion_probe
            .rotation_mode
            .get(character)
            .copied()
            .unwrap_or(RotationMode::ViewDirection);
        let probe = &mut state.overlays_probe;
        overlays::observe(
            probe,
            player,
            &motion_probe,
            overlay_object,
            context,
            *stance,
            rotation,
            *locomotion,
            speed,
            &mut animation_window,
        );
        gate_report.direction_trace.push(
            serde_json::json!({"mode": probe.mode, "stage": probe.stage, "sample": probe.sample}),
        );
    }
    if state.scenario == IntegrationScenario::BowOverlay && !state.entered {
        let phase = state.phase;
        bow::observe_object(
            &mut state.bow_probe,
            overlay_object,
            context,
            &motion_probe,
            notify_action,
            mantle,
            phase,
        );
        if phase != IntegrationPhase::BowOverlay {
            gate_report.direction_trace.push(serde_json::json!({"phase": format!("{phase:?}"), "object": state.bow_probe.sample.object,
                "cleared": state.bow_probe.cleared, "restored": state.bow_probe.restored}));
        }
    }
    if state.scenario == IntegrationScenario::BowOverlay
        && state.phase == IntegrationPhase::BowOverlay
        && !state.entered
    {
        let now = state.game_time_seconds;
        let mode = motion_probe
            .rotation_mode
            .get(character)
            .copied()
            .unwrap_or(RotationMode::ViewDirection);
        let notify = motion_probe.transition_notify.get(character).ok().flatten();
        let probe = &mut state.bow_probe;
        bow::observe(
            probe,
            player,
            &motion_probe,
            context,
            *stance,
            mode,
            speed,
            &mut animation_window,
        );
        gate_report.direction_trace.push(serde_json::json!({"case": probe.index, "time": now, "covered": probe.covered,
            "rotation_mode": format!("{mode:?}"), "stance": format!("{stance:?}"), "speed": speed, "source": probe.sample,
            "transition_playback": transition_playback.map(|value| (bevy_als::stop_link::source_transition_sequence(value), value.active, value.clip_time, value.weight)),
            "transition_notifications": notify.map(|value| (value.transition_animation_call_count, value.transition_animation_play_count))}));
    }
    if state.scenario == IntegrationScenario::DefaultOverlay
        && state.phase == IntegrationPhase::DefaultOverlay
        && !state.entered
    {
        let delta = graph_global_f32(player, "animation.delta_seconds").unwrap_or(0.0);
        let now = state.game_time_seconds;
        let probe = &mut state.default_overlay_probe;
        let mut sample =
            default_overlay_graph_sample(player, &motion_probe, probe.prediction_previous);
        sample.layering =
            layering_graph_sample(player, &motion_probe, probe.previous_layering.as_ref());
        if let Some(frame) = player.get_context_arena().map(|arena| arena.frame_index()) {
            probe.previous_layering = Some((
                frame,
                bevy_als::layering_layer::refresh_state_from_curves(context),
            ));
        }
        if let Ok((_, spaces, _, _)) = foot_ik_data.get(character)
            && !default_overlay_foot_targets_ready(spaces)
        {
            sample
                .errors
                .push("foot_trace_and_solver_must_consume_the_same_prepared_rig_target".into());
        }
        let active = *overlay == OverlayState::Default
            && overlay_tag.as_ref() == Some(&overlay.tag())
            && source_ready
            && sample.ready
            && sample.layering.ready;
        if active {
            probe.valid &= sample.errors.is_empty() && sample.layering.errors.is_empty();
            probe.curves_seen |= default_overlay_curves_ready(context);
            probe.additive_seen |= sample.additive_effect.is_some_and(|value| value > 1e-4);
            for (index, time) in sample.evaluator_times.iter().enumerate() {
                probe.evaluators_seen[index] |= time.is_some();
            }
            if let Some(alpha) = sample.prediction_alpha {
                if let Some(previous) = probe.prediction_previous {
                    probe.prediction_increased |= alpha > previous + 1e-4;
                    probe.prediction_decreased |= alpha < previous - 1e-4;
                }
                probe.prediction_previous = Some(alpha);
            }
        }
        let standing = sample.inputs[0].is_some_and(|value| value > 0.99);
        let crouching = sample.inputs[1].is_some_and(|value| value > 0.99);
        let walking = sample.inputs[2];
        let air = sample.inputs[3].is_some_and(|value| value > 0.95);
        let grounded = sample.inputs[3].is_some_and(|value| value < 0.01)
            && *locomotion == LocomotionState::Grounded;
        let index = probe.case_index;
        probe.airborne_seen |=
            probe.jump_started_at.is_some() && *locomotion == LocomotionState::Air;
        probe.jump_completed |= probe.airborne_seen && standing && grounded && speed < 0.1;
        let selection = if !active || !probe.valid {
            None
        } else if index == 0
            && grounded
            && standing
            && speed < 0.1
            && walking.is_some_and(|value| value < 0.01)
        {
            Some(0)
        } else if index == 1
            && grounded
            && standing
            && speed > 0.5
            && walking.is_some_and(|value| value > 0.99)
        {
            Some(1)
        } else if index == 2 && grounded && crouching && speed > 0.1 {
            Some(2)
        } else if air {
            probe.air_window(sample.prediction_alpha)
        } else if index == 5 && standing && grounded && speed < 0.1 {
            Some(5)
        } else {
            None
        };
        if let Some(selected) = selection {
            **animation_window = default_overlay_window(selected, &sample);
            probe.seconds[selected] += delta;
            if selected < 3 {
                probe.canonical_seen[selected] |= sample
                    .idle_phase
                    .is_some_and(|phase| (phase - 0.35).abs() <= 0.04);
            }
            let enough = probe.seconds[selected]
                >= if selected < 3 || selected == 5 {
                    0.5
                } else {
                    0.1
                };
            probe.covered[selected] |= enough && (selected >= 3 || probe.canonical_seen[selected]);
        }
        gate_report.direction_trace.push(serde_json::json!({"case": index, "time": now, "covered": probe.covered, "valid": probe.valid,
            "airborne": probe.airborne_seen, "jump_completed": probe.jump_completed, "prediction_increased": probe.prediction_increased,
            "prediction_decreased": probe.prediction_decreased, "curves": default_overlay_curves_ready(context), "source": sample,
            "standing_graph": standing_graph.map(|value| (value.actual_standing_state, value.actual_standing_transition_active, value.actual_movement_state)),
            "transition_playback": transition_playback.map(|value| (bevy_als::stop_link::source_transition_sequence(value), value.active, value.clip_time, value.weight)),
            "layering": [graph_global_f32(player, "layering_state.pelvis_blend_amount"), graph_global_f32(player, "layering_state.legs_blend_amount")],
            "evaluation_frame": player.get_context_arena().map(|arena| arena.frame_index()),
            "foot_solve_inputs": foot_ik_data.get(character).ok().map(|(_, spaces, _, _)| [spaces.left.solve_input, spaces.right.solve_input])}));
        probe.sample = sample;
    }
    let standing_graph_live = standing_graph.is_some_and(|telemetry| {
        (telemetry.actual_movement_state.is_some()
            || telemetry.actual_movement_transition_active == Some(true))
            && telemetry.actual_movement_pose_ready == Some(true)
            && telemetry.dominant_detail_index.is_some()
            && telemetry
                .dominant_detail_weight
                .is_some_and(|weight| (0.0..=1.0).contains(&weight))
    });
    if *stance == Stance::Standing && speed > 0.5 && source_ready && standing_graph_live {
        if !state.coverage.standing_graph
            && let Some(telemetry) = standing_graph
        {
            info!(
                "[MANUAL-INTEGRATION][STANDING-GRAPH] fsm_state={:?} transition_active={} pose_ready={} dominant_branch={:?} dominant_weight={:.3}",
                telemetry.actual_movement_state,
                telemetry.actual_movement_transition_active.unwrap_or(false),
                telemetry.actual_movement_pose_ready.unwrap_or(false),
                telemetry.dominant_detail_index,
                telemetry.dominant_detail_weight.unwrap_or_default(),
            );
        }
        state.coverage.standing_graph = true;
    }
    if state.phase == IntegrationPhase::CrouchDirectionRegression {
        let elapsed = state.phase_elapsed();
        let case_index = if state.crouch_stationary_validation.is_some() {
            CROUCH_DIRECTION_CASE_COUNT
        } else {
            state.crouch_direction_case_index
        };
        if let Some(graph) =
            crouching_graph.filter(|graph| graph.actual_movement_pose_ready == Some(true))
        {
            state.coverage.crouch_direction_state_mask |= graph.actual_movement_states;
        }
        if case_index < CROUCH_DIRECTION_CASE_COUNT
            && !state.coverage.crouch_direction_cases[case_index]
            && !state.coverage.crouch_direction_diagnostics[case_index]
            && elapsed
                >= state.crouch_direction_case_started_at + CROUCH_DIRECTION_CASE_SETTLE_SECONDS
            && let Some(case) = crouch_direction_regression_case(case_index)
        {
            state.coverage.crouch_direction_diagnostics[case_index] = true;
            info!(
                "[MANUAL-INTEGRATION][CROUCH-DIRECTION][OBSERVE] case={} stance={:?} speed={:.3} source={} runtime={:?} stride={:?} rate={:?} marker={:?} pose_moving={:?} movement_direction={:?} velocity={:?}",
                case.name,
                stance,
                speed,
                source_ready,
                crouching_graph,
                crouching_graph.and_then(|value| value.graph_stride_amount),
                crouching_graph.and_then(|value| value.graph_play_rate),
                crouching_graph.and_then(|value| value.graph_marker_phase),
                crouching_graph.and_then(|value| value.graph_pose_moving),
                movement_direction.map(|value| value.direction),
                actual_motion.map(|value| value.local_velocity),
            );
        }
        if case_index < CROUCH_DIRECTION_CASE_COUNT
            && !state.coverage.crouch_direction_cases[case_index]
            && elapsed
                >= state.crouch_direction_case_started_at + CROUCH_DIRECTION_CASE_SETTLE_SECONDS
            && let Some(case) = crouch_direction_regression_case(case_index)
            && let (Some(motion), Some(telemetry)) = (actual_motion, crouching_graph)
            && let Some(state_index) = telemetry.actual_movement_state
            && state_index < 6
        {
            let expected_world = bevy_als::als::motion::view_relative_move_direction(
                case.move_dir,
                motion_probe
                    .player_views
                    .get(character)
                    .map_or(view_yaw.0, |view| view.0),
            );
            let expected_local = Vec2::new(
                expected_world.dot(transform.rotation * Vec3::NEG_Z),
                expected_world.dot(transform.rotation * Vec3::X),
            )
            .normalize_or_zero();
            let world_alignment = Vec2::new(motion.world_velocity.x, motion.world_velocity.z)
                .normalize_or_zero()
                .dot(Vec2::new(expected_world.x, expected_world.z));
            let local_alignment = motion
                .local_velocity
                .normalize_or_zero()
                .dot(expected_local);
            let graph_matches = telemetry.actual_outer_state == Some(1)
                && motion_probe.rotation_mode.get(character).ok() == Some(&RotationMode::Aiming)
                && telemetry.actual_outer_transition_active == Some(false)
                && telemetry.actual_outer_pose_ready == Some(true)
                && telemetry.actual_movement_transition_active == Some(false)
                && state_index == movement_direction_fsm_index(case.expected_direction)
                && telemetry.actual_movement_pose_ready == Some(true);
            let ready = source_ready
                && motion.horizontal_speed > 0.45
                && world_alignment >= 0.75
                && local_alignment >= 0.75
                && graph_matches;
            if ready {
                state.crouch_direction_ready_at.get_or_insert(elapsed);
            } else {
                state.crouch_direction_ready_at = None;
            }
            if ready && crouch_direction_case_complete(state.crouch_direction_ready_at, elapsed) {
                state.coverage.crouch_direction_cases[case_index] = true;
                state.coverage.crouch_direction_state_mask |= 1 << state_index;
                info!(
                    "[MANUAL-INTEGRATION][CROUCH-DIRECTION] case={} expected={:?} state={:?} pose_ready={:?} branch={:?} velocity=({:.3},{:.3}) world_align={:.3} local_align={:.3}",
                    case.name,
                    case.expected_direction,
                    telemetry.actual_movement_state,
                    telemetry.actual_movement_pose_ready,
                    movement_direction.map(|value| value.direction),
                    motion.local_velocity.x,
                    motion.local_velocity.y,
                    world_alignment,
                    local_alignment,
                );
            }
        }
        if let Some(telemetry) = crouching_graph {
            state.coverage.crouch_direction_lean |= telemetry
                .lean_right_amount
                .zip(telemetry.lean_forward_amount)
                .is_some_and(|(right, forward)| {
                    right.is_finite()
                        && forward.is_finite()
                        && right.abs().max(forward.abs()) > 1e-4
                });
        }
        if let Some(validation) = state.crouch_stationary_validation {
            let expected_parent_state = if validation.is_turn() {
                0
            } else if validation.is_left() {
                2
            } else {
                3
            };
            let side_index = validation.side_index();
            let source_stationary_precondition = crouch_stationary_source_precondition(
                actual_motion,
                crouching_graph,
                turn_drive,
                turn_slot,
            ) && rotation_animation
                .is_none_or(|value| value.yaw_speed_degrees == 0.0);
            state.crouch_stationary_precondition_frames[side_index] =
                advance_crouch_stationary_stability(
                    state.crouch_stationary_precondition_frames[side_index],
                    source_stationary_precondition,
                );
            if state.crouch_stationary_precondition_frames[side_index]
                >= CROUCH_STATIONARY_STABLE_FRAMES
                && !state.crouch_stationary_precondition_ready[side_index]
            {
                state.crouch_stationary_precondition_ready[side_index] = true;
                state.crouch_stationary_started_at[side_index] = Some(state.phase_elapsed());
                state.crouch_stationary_base_yaw[side_index] =
                    Some(world_forward_to_yaw(transform.rotation * Vec3::NEG_Z));
                state.crouch_stationary_view_yaw = view_yaw.0;
                info!(
                    "[MANUAL-INTEGRATION][CROUCH-STATIONARY-READY] side={} speed={:.4} outer={:?}",
                    if validation.is_left() {
                        "left"
                    } else {
                        "right"
                    },
                    actual_motion
                        .map(|motion| motion.horizontal_speed)
                        .unwrap_or_default(),
                    crouching_graph.and_then(|telemetry| telemetry.actual_outer_state),
                );
            }
            if validation.is_turn() {
                let (active, natural_completion, inertialization, settled) =
                    if let (Some(telemetry), Some(drive)) = (crouching_graph, turn_drive) {
                        let turn_precondition_ready =
                            state.crouch_stationary_precondition_ready[side_index];
                        let baseline = state.crouch_turn_epoch_baseline[side_index]
                            .get_or_insert(drive.inertialization_request_epoch);
                        let side_matches = (drive.lr <= 0.5) == validation.is_left();
                        let curve_driven = rotation_animation
                            .is_some_and(|state| state.yaw_speed_degrees.abs() > 1e-3);
                        let slot_active = turn_slot.is_some_and(|slot| {
                            slot.active && slot.pose_ready && slot.weight > 0.05
                        });
                        let active = turn_precondition_ready
                            && telemetry.actual_outer_state == Some(0)
                            && telemetry.actual_outer_transition_active == Some(false)
                            && drive.playing
                            && drive.weight > 0.05
                            && side_matches
                            && curve_driven
                            && slot_active;
                        let natural_completion =
                            drive.inertialization_request_epoch > *baseline + f32::EPSILON;
                        let inertialization = natural_completion
                            && turn_slot.is_some_and(|slot| {
                                slot.inertialization_request_epoch > *baseline + f32::EPSILON
                            });
                        let settled = inertialization
                            && !drive.playing
                            && drive.weight <= 0.05
                            && turn_slot.is_some_and(|slot| !slot.active && slot.weight <= 0.05);
                        (active, natural_completion, inertialization, settled)
                    } else {
                        (false, false, false, false)
                    };
                let lifecycle = match (active, natural_completion, inertialization, settled) {
                    (true, _, _, _) => &["slot_active"][..],
                    (_, true, true, true) => {
                        &["natural_completion", "inertialization", "settled"][..]
                    }
                    (_, true, true, false) => &["natural_completion", "inertialization"][..],
                    (_, true, false, _) => &["natural_completion"][..],
                    (_, false, false, true) => &["settled"][..],
                    _ => &[],
                };
                **animation_window = crouch_turn_window(validation.is_left(), active, lifecycle);
                animation_window.expected_actor_yaw_baseline =
                    state.crouch_stationary_base_yaw[side_index];
                animation_window.observed_parent_runtime_state =
                    crouching_graph.and_then(|value| value.actual_outer_state);
                animation_window.observed_parent_transition_active =
                    crouching_graph.and_then(|value| value.actual_outer_transition_active);
                animation_window.observed_requested_parent_runtime_state = None;
                animation_window.observed_requested_parent_transition_active = None;
                record_animation_window_trace(
                    &mut animation_window,
                    transform,
                    view_yaw.0,
                    rotation_animation,
                    crouching_graph,
                    turn_drive,
                    turn_stack,
                    true,
                    turn_drive
                        .map(|drive| crouch_stationary_animation_variant(drive.lr <= 0.5, true)),
                );
                if let Some(entry) = turn_stack.and_then(|stack| {
                    stack.entries.iter().rev().find(|entry| {
                        entry.crouching && entry.sampling_ready && entry.weight() > 0.0
                    })
                }) {
                    let variant =
                        bevy_als::turn_layer::source_turn_sequence(true, entry.is_180, entry.lr);
                    animation_window.observed_animation_variant = Some(variant);
                    animation_window.observed_animation_clip_time = Some(entry.elapsed);
                    animation_window.observed_animation_play_rate = Some(entry.play_rate);
                    animation_window.observed_animation_weight = Some(entry.weight());
                    animation_window.observed_direction =
                        Some(if entry.lr <= 0.5 { "Left" } else { "Right" });
                    animation_window.observed_animation_curve_direct_yaw_speed_degrees =
                        bevy_als::standing_source::MONTAGES[variant]
                            .curves
                            .sample_all(entry.elapsed)
                            .get(&bevy_als::extensions::SourceCurveName::from(
                                "RotationYawSpeed",
                            ))
                            .copied();
                }
                if active {
                    state.coverage.crouch_turn_active_mask |= 1 << side_index;
                }
                if natural_completion && inertialization && settled {
                    state.coverage.crouch_turn_completion_mask |= 1 << side_index;
                }
            } else {
                let sample = source_sequence_sample(
                    player,
                    &motion_probe.graphs,
                    &format!(
                        "{}\nSequence Player",
                        crouch_stationary_animation_variant(validation.is_left(), false)
                    ),
                );
                let active = state.crouch_stationary_precondition_ready[side_index]
                    && crouching_graph.is_some_and(|telemetry| {
                        telemetry.actual_outer_weights[expected_parent_state as usize] > 1e-5
                    })
                    && sample.is_some_and(|value| value.2)
                    && rotation_animation.is_some_and(|state| state.yaw_speed_degrees.abs() > 1e-3);
                if active {
                    state.coverage.crouch_rotate_mask |= 1 << validation.side_index();
                }
                if state.coverage.crouch_rotate_mask & (1 << side_index) != 0
                    && crouching_graph.is_some_and(|graph| {
                        graph.actual_outer_state == Some(0)
                            && graph.actual_outer_transition_active == Some(false)
                    })
                {
                    state.coverage.crouch_rotate_source_completion_mask |= 1 << side_index;
                }
                let source_completed =
                    state.coverage.crouch_rotate_source_completion_mask & (1 << side_index) != 0;
                let settled = source_rotate_window_completed(
                    state.coverage.crouch_rotate_mask & (1 << side_index) != 0,
                    source_completed,
                    source_stationary_precondition
                        && standing_turn_rotation_settled(
                            if validation.is_left() { 4 } else { 5 },
                            bevy_als::turn_layer::character_turn_angle_from_radians(
                                view_yaw.0,
                                world_forward_to_yaw(*transform.forward()).to_degrees(),
                            ),
                            rotation_animation.map_or(0.0, |value| value.yaw_speed_degrees),
                            turn_drive.is_some_and(|value| value.rotate_active),
                        ),
                );
                if settled {
                    state.coverage.crouch_rotate_completion_mask |= 1 << side_index;
                }
                let lifecycle = match (active, source_completed, settled) {
                    (true, _, _) => &[][..],
                    (_, true, true) => &["sequence_completed", "settled"][..],
                    (_, true, false) => &["sequence_completed"][..],
                    _ => &[],
                };
                **animation_window = crouch_rotate_window(validation.is_left(), active, lifecycle);
                animation_window.expected_actor_yaw_baseline =
                    state.crouch_stationary_base_yaw[side_index];
                animation_window.observed_parent_runtime_state =
                    crouching_graph.and_then(|value| value.actual_outer_state);
                animation_window.observed_parent_transition_active =
                    crouching_graph.and_then(|value| value.actual_outer_transition_active);
                animation_window.observed_requested_parent_runtime_state = None;
                animation_window.observed_requested_parent_transition_active = None;
                record_animation_window_trace(
                    &mut animation_window,
                    transform,
                    view_yaw.0,
                    rotation_animation,
                    crouching_graph,
                    turn_drive,
                    turn_stack,
                    true,
                    turn_drive.map(|drive| {
                        crouch_stationary_animation_variant(drive.rotate_lr <= 0.5, false)
                    }),
                );
                animation_window.observed_animation_clip_time = sample.map(|value| value.0);
                animation_window.observed_animation_curve_direct_yaw_speed_degrees =
                    sample.map(|value| value.1);
                animation_window.observed_animation_play_rate =
                    bevy_als::extensions::source_instance::source_player_property(
                        player,
                        "rotate_in_place_state.play_rate",
                    )
                    .and_then(|value| value.as_f32().ok());
                animation_window.observed_animation_weight = sample
                    .and_then(|_| crouching_graph)
                    .map(|graph| graph.actual_outer_weights[expected_parent_state as usize]);
                animation_window.observed_animation_variant = sample
                    .map(|_| crouch_stationary_animation_variant(validation.is_left(), false));
                animation_window.observed_direction = sample.map(|_| {
                    if validation.is_left() {
                        "Left"
                    } else {
                        "Right"
                    }
                });
            }
        }
    }
    if state.phase == IntegrationPhase::CrouchRecovery
        && let Some(transaction) = crouch_transaction
        && transaction.actual_stance == Stance::Crouching
        && transaction.controller_crouch_active
        && (transaction.headroom_blocked || transaction.collider_half_height < 0.9)
    {
        if !state.coverage.crouch_transaction {
            info!(
                "[MANUAL-INTEGRATION][CROUCH] desired={:?} actual={:?} phase={:?} retries={} half={:.3} radius={:.3} controller={} blocked={}",
                transaction.desired_stance,
                transaction.actual_stance,
                transaction.phase,
                transaction.blocked_retry_count,
                transaction.collider_half_height,
                transaction.collider_radius,
                transaction.controller_crouch_active,
                transaction.headroom_blocked,
            );
        }
        state.coverage.crouch_transaction = true;
    }
    if let (Some(transaction), Some(telemetry)) = (crouch_transaction, grounded_graph)
        && transaction.actual_stance == Stance::Crouching
        && telemetry.actual_state == Some(2)
        && telemetry.transition_active == Some(false)
        && telemetry.pose_ready == Some(true)
    {
        if !state.coverage.crouch_graph {
            info!(
                "[MANUAL-INTEGRATION][CROUCH-GRAPH] desired={:?} actual={:?} forced_standing={} state={} transition_active={} pose_ready={}",
                transaction.desired_stance,
                transaction.actual_stance,
                transaction.forced_standing,
                telemetry.actual_state.unwrap_or_default(),
                telemetry.transition_active.unwrap_or(false),
                telemetry.pose_ready.unwrap_or(false),
            );
        }
        state.coverage.crouch_graph = true;
    }
    if let Some(transaction) = crouch_transaction
        && *locomotion == LocomotionState::Air
        && transaction.desired_stance == Stance::Crouching
        && transaction.actual_stance == Stance::Standing
        && transaction.forced_standing
        && !transaction.controller_crouch_active
    {
        if !state.coverage.crouch_air_forced_standing {
            info!(
                "[MANUAL-INTEGRATION][CROUCH-AIR] desired={:?} actual={:?} phase={:?} controller={} forced_standing={}",
                transaction.desired_stance,
                transaction.actual_stance,
                transaction.phase,
                transaction.controller_crouch_active,
                transaction.forced_standing,
            );
        }
        state.coverage.crouch_air_forced_standing = true;
    }
    if state.phase == IntegrationPhase::CrouchRecovery
        && state.coverage.crouch_air_forced_standing
        && let (Some(transaction), Some(telemetry)) = (crouch_transaction, grounded_graph)
        && *locomotion == LocomotionState::Grounded
        && transaction.desired_stance == Stance::Crouching
        && transaction.actual_stance == Stance::Crouching
        && telemetry.actual_state == Some(2)
        && telemetry.transition_active == Some(false)
        && telemetry.pose_ready == Some(true)
    {
        if !state.coverage.crouch_air_landed {
            info!(
                "[MANUAL-INTEGRATION][CROUCH-LAND] desired={:?} actual={:?} state={} controller={} forced_standing={}",
                transaction.desired_stance,
                transaction.actual_stance,
                telemetry.actual_state.unwrap_or_default(),
                transaction.controller_crouch_active,
                transaction.forced_standing,
            );
        }
        state.coverage.crouch_air_landed = true;
    }
    if state.phase == IntegrationPhase::CrouchRecovery
        && state.crouch_recovery_standing_requested
        && let (Some(transaction), Some(telemetry)) = (crouch_transaction, grounded_graph)
        && transaction.desired_stance == Stance::Standing
        && transaction.actual_stance == Stance::Standing
        && !transaction.controller_crouch_active
        && telemetry.actual_state == Some(1)
        && telemetry.transition_active == Some(false)
        && telemetry.pose_ready == Some(true)
    {
        if !state.coverage.crouch_graph_recovered {
            info!(
                "[MANUAL-INTEGRATION][CROUCH-RECOVERED] desired={:?} actual={:?} state={} controller={}",
                transaction.desired_stance,
                transaction.actual_stance,
                telemetry.actual_state.unwrap_or_default(),
                transaction.controller_crouch_active,
            );
        }
        state.coverage.crouch_graph_recovered = true;
    }
    if state.phase == IntegrationPhase::CrouchClearance
        && *stance == Stance::Crouching
        && speed > 0.1
    {
        if !state.coverage.crouch_moving {
            info!(
                "[MANUAL-INTEGRATION][CROUCH-MOVING] speed={:.3} desired={:?} actual={:?}",
                speed,
                crouch_transaction.map(|transaction| transaction.desired_stance),
                stance,
            );
        }
        state.coverage.crouch_moving = true;
    }
    if state.phase == IntegrationPhase::CrouchClearance
        && state.crouch_canopy.is_some()
        && let (Some(transaction), Some(telemetry)) = (crouch_transaction, grounded_graph)
        && transaction.desired_stance == Stance::Standing
        && transaction.actual_stance == Stance::Crouching
        && transaction.headroom_blocked
        && transaction.blocked_retry_count >= 2
        && telemetry.actual_state == Some(2)
        && telemetry.transition_active == Some(false)
        && telemetry.pose_ready == Some(true)
    {
        if !state.coverage.crouch_canopy_blocked {
            info!(
                "[MANUAL-INTEGRATION][CROUCH-CANOPY-BLOCKED] desired={:?} actual={:?} retries={} half={:.3} state={} controller={}",
                transaction.desired_stance,
                transaction.actual_stance,
                transaction.blocked_retry_count,
                transaction.collider_half_height,
                telemetry.actual_state.unwrap_or_default(),
                transaction.controller_crouch_active,
            );
        }
        state.coverage.crouch_canopy_blocked = true;
    }
    if state.phase == IntegrationPhase::CrouchClearance
        && state.coverage.crouch_canopy_blocked
        && let (Some(transaction), Some(telemetry)) = (crouch_transaction, grounded_graph)
        && transaction.desired_stance == Stance::Standing
        && transaction.actual_stance == Stance::Standing
        && !transaction.headroom_blocked
        && !transaction.controller_crouch_active
        && telemetry.actual_state == Some(1)
        && telemetry.transition_active == Some(false)
        && telemetry.pose_ready == Some(true)
    {
        if !state.coverage.crouch_canopy_recovered {
            info!(
                "[MANUAL-INTEGRATION][CROUCH-CANOPY-RECOVERED] desired={:?} actual={:?} retries={} half={:.3} state={}",
                transaction.desired_stance,
                transaction.actual_stance,
                transaction.blocked_retry_count,
                transaction.collider_half_height,
                telemetry.actual_state.unwrap_or_default(),
            );
        }
        state.coverage.crouch_canopy_recovered = true;
    }
    if state.phase == IntegrationPhase::DirectionRegression
        && let Some(telemetry) = standing_graph
        && telemetry.actual_movement_pose_ready == Some(true)
    {
        state.coverage.direction_source_states |= telemetry.actual_movement_states;
    }
    if state.phase == IntegrationPhase::DirectionRegression
        && state.direction_case_index < DIRECTION_CASE_COUNT
        && !state.coverage.direction_cases[state.direction_case_index]
        && state.phase_elapsed()
            >= state.direction_case_index as f32 * DIRECTION_CASE_SECONDS
                + DIRECTION_CASE_SETTLE_SECONDS
        && let Some(case) = direction_regression_case(state.direction_case_index)
        && let (Some(motion), Some((desired, allowed)), Some(telemetry), Some(direction)) = (
            actual_motion,
            gait_state,
            standing_graph,
            movement_direction,
        )
    {
        let expected_world = bevy_als::als::motion::view_relative_move_direction(
            case.move_dir,
            motion_probe
                .player_views
                .get(character)
                .map_or(view_yaw.0, |view| view.0),
        );
        let expected_local = Vec2::new(
            expected_world.dot(transform.rotation * Vec3::NEG_Z),
            expected_world.dot(transform.rotation * Vec3::X),
        )
        .normalize_or_zero();
        let world_alignment = Vec2::new(motion.world_velocity.x, motion.world_velocity.z)
            .normalize_or_zero()
            .dot(Vec2::new(expected_world.x, expected_world.z));
        let local_alignment = motion
            .local_velocity
            .normalize_or_zero()
            .dot(expected_local);
        let branch_mask = if case.branch_from_actual_local {
            actual_velocity_branch_mask(motion)
        } else {
            case.branch_mask
        };
        let source_state_matches = case.expected_movement_state.is_none_or(|expected| {
            telemetry.actual_movement_state == Some(movement_direction_fsm_index(expected))
        });
        let target_state_matches = case.expected_movement_state.is_some()
            || telemetry.actual_movement_state
                == Some(movement_direction_fsm_index(direction.direction));
        let graph_matches = source_state_matches
            && target_state_matches
            && telemetry.actual_movement_transition_active == Some(false)
            && telemetry.actual_movement_pose_ready == Some(true)
            && direction_branch_matches(telemetry.dominant_detail_index, branch_mask)
            && telemetry
                .dominant_detail_weight
                .is_some_and(|weight| weight >= 0.4);
        let gait_matches = desired.0 == case.desired_gait
            && allowed.0 == case.expected_allowed_gait
            && *gait == case.expected_actual_gait
            && first_person == case.first_person
            && motion_probe.rotation_mode.get(character).ok() == Some(&case.rotation_mode);
        if source_ready
            && motion.horizontal_speed > 0.75
            && world_alignment >= 0.8
            && local_alignment >= 0.8
            && gait_matches
            && graph_matches
        {
            let case_index = state.direction_case_index;
            state.coverage.direction_cases[case_index] = true;
            info!(
                "[MANUAL-INTEGRATION][DIRECTION] case={} source_state={:?} expected_direction={:?} world=({:.3},{:.3}) local=({:.3},{:.3}) direction={:?} gait=({:?},{:?},{:?}) fsm={} branch={} branch_mask={} weight={:.3} world_align={:.3} local_align={:.3}",
                case.name,
                case.source_state,
                case.expected_movement_state,
                motion.world_velocity.x,
                motion.world_velocity.z,
                motion.local_velocity.x,
                motion.local_velocity.y,
                direction.direction,
                desired.0,
                allowed.0,
                gait,
                telemetry.actual_movement_state.unwrap_or_default(),
                telemetry.dominant_detail_index.unwrap_or_default(),
                branch_mask,
                telemetry.dominant_detail_weight.unwrap_or_default(),
                world_alignment,
                local_alignment,
            );
        }
    }
    for event in animation_events
        .read()
        .filter(|event| event.character == character)
    {
        if state.scenario != IntegrationScenario::Notify {
            continue;
        }
        match event.class_name.as_str() {
            "AlsAnimNotify_CameraShake"
                if event.source_asset.contains("A_Als_Sprint_Acceleration") =>
            {
                state.coverage.notify_sprint_acceleration = true;
            }
            "AlsAnimNotify_CameraShake" if event.source_asset.contains("A_Als_Sprint") => {
                state.coverage.notify_sprint = true;
            }
            "AlsAnimNotify_CameraShake" if event.source_asset.contains("A_Als_Roll") => {
                state.coverage.notify_roll = true;
            }
            "AuthoredSyncMarker" => {
                state.coverage.notify_marker = true;
            }
            "AlsAnimNotify_SetGroundedEntryMode" => {
                state.coverage.notify_grounded_entry = true;
            }
            "AlsAnimNotifyState_SetRootMotionScale" => {
                state.coverage.notify_root_scale = true;
            }
            "AlsAnimNotifyState_SetLocomotionAction" => {
                if event.phase == bevy_als::extensions::EventPhase::Active {
                    match event.parameters.get("LocomotionAction") {
                        Some(value)
                            if value
                                .get("__tagged__")
                                .and_then(|value| value.get("TagName"))
                                .and_then(serde_json::Value::as_str)
                                == Some("Als.LocomotionAction.Rolling") =>
                        {
                            state.coverage.notify_rolling_action = true;
                        }
                        Some(value)
                            if value
                                .get("__tagged__")
                                .and_then(|value| value.get("TagName"))
                                .and_then(serde_json::Value::as_str)
                                == Some("Als.LocomotionAction.Mantling") =>
                        {
                            state.coverage.notify_mantling_action = true;
                        }
                        _ => {}
                    }
                }
            }
            "AlsAnimNotifyState_EarlyBlendOut" => {
                state.coverage.notify_early_blend_out = true;
            }
            _ => {}
        }
        info!(
            "[MANUAL-INTEGRATION][NOTIFY] source={} class={} name={} phase={:?} time={:.6} weight={:.3}",
            event.source_asset, event.class_name, event.name, event.phase, event.time, event.weight,
        );
    }
    if state.scenario == IntegrationScenario::Notify {
        state.coverage.notify_roll_reentry |= state.notify_roll_restarted
            && roll.is_some_and(|roll| roll.active && roll.time < 0.1)
            && motion_probe
                .source_inputs
                .get(instance.player_entity())
                .ok()
                .and_then(|inputs| inputs.slot_input_context("roll.pose"))
                .is_some_and(|context| {
                    context
                        .asset_samples
                        .iter()
                        .filter(|sample| {
                            sample.identity.starts_with("roll:") && sample.clock.weight > 0.0
                        })
                        .count()
                        >= 2
                });
        state.coverage.notify_rolling_action |=
            notify_action.is_some_and(|action| *action == AlsNotifyLocomotionAction::Rolling);
        state.coverage.notify_mantling_action |=
            notify_action.is_some_and(|action| *action == AlsNotifyLocomotionAction::Mantling);
        state.coverage.notify_root_scale |=
            notify_root_scale.is_some_and(|scale| (scale.0 - 1.3).abs() < 1e-4);
        state.coverage.notify_early_blend_out |= notify_early_blend_out
            .is_some_and(|state| state.active && state.blend_out_duration > 0.0);
        state.coverage.notify_grounded_entry |=
            grounded_entry.is_some_and(|mode| *mode == AlsGroundedEntryMode::FromRoll);
    }
    if matches!(
        state.scenario,
        IntegrationScenario::StandingDirection
            | IntegrationScenario::StandingResponse
            | IntegrationScenario::StandingStop
            | IntegrationScenario::StandingTurn
            | IntegrationScenario::StandingDynamic
            | IntegrationScenario::Crouching
            | IntegrationScenario::CrouchingActions
    ) && matches!(
        state.phase,
        IntegrationPhase::DirectionRegression
            | IntegrationPhase::StandingResponse
            | IntegrationPhase::StandingStop
            | IntegrationPhase::StandingTurn
            | IntegrationPhase::StandingDynamic
            | IntegrationPhase::CrouchDirectionRegression
    ) && (state.phase != IntegrationPhase::StandingStop
        || standing_stop_observation_active(state))
        && (!matches!(
            state.phase,
            IntegrationPhase::StandingTurn | IntegrationPhase::StandingDynamic
        ) || !state.entered)
    {
        let crouching_actions = state.scenario == IntegrationScenario::CrouchingActions;
        let (parent_state, parent_transition, parent_pose_ready) =
            action_graph_state(crouching_actions, standing_graph, crouching_graph);
        let idle = parent_state == Some(0)
            && parent_transition == Some(false)
            && parent_pose_ready == Some(true)
            && *stance
                == if crouching_actions {
                    Stance::Crouching
                } else {
                    Stance::Standing
                };
        let source_node = if crouching_actions {
            "AB_Als_Crouching/Crouching States"
        } else {
            "AB_Als_Standing/Standing States"
        };
        let mut samples = Vec::new();
        let mut machines = Vec::new();
        let mut rotate_sample = None;
        let arena = player.get_context_arena();
        if let Some(arena) = arena {
            if let Some(ticks) = bevy_als::extensions::sync_group_phase::source_sync_players(
                arena,
                &motion_probe.graphs,
                if state.scenario == IntegrationScenario::Crouching || crouching_actions {
                    "sync_groups.movement.phase"
                } else {
                    "sync_groups.movement.phase"
                },
            ) {
                for tick in ticks {
                    let Some(child) = arena.get_context(tick.identity.0) else {
                        continue;
                    };
                    let Some(graph) = motion_probe.graphs.get(child.get_graph_id()) else {
                        continue;
                    };
                    let Some(node) = graph.nodes.get(&tick.identity.1) else {
                        continue;
                    };
                    let position = match graph
                        .edges_inverted
                        .get(&TargetPin::NodeData(node.id, "position".into()))
                    {
                        Some(SourcePin::NodeData(input, pin)) => child
                            .node_caches
                            .get_output_data(*input, StateKey::Default, pin.clone())
                            .and_then(|value| value.as_vec2().ok()),
                        _ => None,
                    };
                    samples.push(serde_json::json!({
                        "node": node.name, "weight": tick.weight, "phase": tick.phase, "previous_phase": tick.previous_phase,
                        "duration": tick.duration, "play_rate": tick.play_rate, "delta_seconds": tick.delta_seconds,
                        "position": position.map(|value| value.to_array()),
                        "sample_time": child.node_caches.get_output_data(node.id, StateKey::Default, "sample_time".into()).and_then(|value| value.as_f32().ok()).or_else(|| node.try_inner_downcast_ref::<bevy_als::extensions::SourceSequencePlayerNode>().map(|_| child.node_states.get_time(node.id, StateKey::Default))),
                    }));
                }
            }
            for id in arena.iter_context_ids() {
                let Some(child) = arena.get_context(id) else {
                    continue;
                };
                let Some(graph) = motion_probe.graphs.get(child.get_graph_id()) else {
                    continue;
                };
                if state.phase == IntegrationPhase::StandingTurn {
                    for node in graph.nodes.values() {
                        let variant = match node.name.as_str() {
                            "A_Als_Rotate_90_Left\nSequence Player" => "A_Als_Rotate_90_Left",
                            "A_Als_Rotate_90_Right\nSequence Player" => "A_Als_Rotate_90_Right",
                            _ => continue,
                        };
                        let expected = if state.turn_case_index == 7 {
                            if turn_drive.is_some_and(|drive| drive.rotate_lr <= 0.5) {
                                "A_Als_Rotate_90_Left"
                            } else {
                                "A_Als_Rotate_90_Right"
                            }
                        } else {
                            STANDING_TURN_CASES[state.turn_case_index].1
                        };
                        if variant != expected
                            || !child.node_caches.is_updated(node.id, StateKey::Default)
                        {
                            continue;
                        }
                        if let Some(sequence) = node
                            .try_inner_downcast_ref::<bevy_als::extensions::SourceSequencePlayerNode>()
                        {
                            let position = child.node_states.get_time(node.id, StateKey::Default);
                            rotate_sample = Some((
                                variant,
                                position,
                                sequence
                                    .curves
                                    .sample_all(position)
                                    .get(&bevy_als::extensions::SourceCurveName::from(
                                        "RotationYawSpeed",
                                    ))
                                    .copied()
                                    .unwrap_or(0.0),
                            ));
                        }
                    }
                }
                for node in graph.nodes.values().filter(|node| {
                    [
                        "Movement States\nState Machine",
                        "Movement Details States\nState Machine",
                        "Standing States\nState Machine",
                        "Crouching States\nState Machine",
                        "Stop States\nState Machine",
                    ]
                    .contains(&node.name.as_str())
                }) {
                    if !child.node_caches.is_updated(node.id, StateKey::Default) {
                        continue;
                    }
                    let Some(fsm) =
                        node.try_inner_downcast_ref::<bevy_als::extensions::InterruptibleFsmNode>()
                    else {
                        continue;
                    };
                    let Some(machine) = motion_probe.machines.get(&fsm.fsm) else {
                        continue;
                    };
                    let Ok(runtime) = child
                        .node_states
                        .get::<bevy_als::extensions::InterruptibleFsmState>(
                            node.id,
                            StateKey::Default,
                        )
                    else {
                        continue;
                    };
                    let weights = machine
                        .states
                        .values()
                        .map(|state| {
                            (
                                state.label.clone(),
                                runtime.recorded_state_weight(&state.id.uuid().to_string()),
                            )
                        })
                        .collect::<Vec<_>>();
                    let state_label = match &runtime.state {
                        bevy_animation_graph::core::state_machine::low_level::LowLevelStateId::HlState(id) => machine.states.get(id).map(|state| state.label.as_str()),
                        _ => None,
                    };
                    machines.push(serde_json::json!({"node": node.name, "state": format!("{:?}", runtime.state), "state_label": state_label, "weights": weights, "snapshot": runtime.cached_source_pose().is_some()}));
                }
            }
        }
        let inputs = motion_probe.inputs.get(character).ok();
        let pose = player.get_default_output_pose();
        let values = [
            "standing_state.play_rate",
            "standing_state.walk_run_blend_amount",
            "standing_state.stride_blend_amount",
            "crouching_state.stride_blend_amount",
            "crouching_state.play_rate",
            "lean_state.forward_amount",
            "lean_state.right_amount",
            "grounded_state.velocity_blend.forward_amount",
            "grounded_state.velocity_blend.backward_amount",
            "grounded_state.velocity_blend.left_amount",
            "grounded_state.velocity_blend.right_amount",
            "pose_state.unweighted_gait_running_amount",
            "animation.delta_seconds",
        ]
        .map(|key| {
            (
                key,
                bevy_als::extensions::source_instance::source_player_property(player, key)
                    .and_then(|value| value.as_f32().ok())
                    .or_else(|| graph_global_f32(player, key)),
            )
        });
        if state.phase == IntegrationPhase::StandingStop {
            let index = state.stop_case_index;
            let (label, branch, _, mode, right) = stop_cases(crouching_actions)[index];
            let quick = index >= if crouching_actions { 1 } else { 4 };
            state.stop_foot_planted = graph_global_f32(player, "feet_state.foot_planted_amount");
            state.stop_branch_seen |= machines.iter().any(|machine| {
                (machine["node"]
                    == if crouching_actions {
                        "Crouching States\nState Machine"
                    } else {
                        "Stop States\nState Machine"
                    })
                    && machine["state_label"] == branch
            });
            let expected = match (crouching_actions, quick) {
                (true, true) => AlsTransitionKind::CrouchingQuickStop,
                (true, false) => AlsTransitionKind::CrouchingStop,
                (false, true) => AlsTransitionKind::QuickStop,
                (false, false) => AlsTransitionKind::Stop,
            };
            let observed = transition_playback.filter(|playback| {
                state.generated_stop_quick_input_released
                    && playback.kind == expected
                    && playback.index == right
                    && motion_probe.rotation_mode.get(character).ok() == Some(&mode)
            });
            let slot_ready = root_transition_slot
                .is_some_and(|slot| slot.active && slot.pose_ready && slot.weight > 0.05);
            if let Some(playback) = observed.filter(|playback| playback.active && slot_ready) {
                let profile_matches = playback.start_time
                    == if quick {
                        0.3
                    } else if crouching_actions {
                        0.2
                    } else {
                        0.4
                    }
                    && playback.blend_in == if quick { 0.1 } else { 0.2 }
                    && playback.blend_out == 0.2
                    && if quick {
                        if index == if crouching_actions { 1 } else { 4 } {
                            playback.play_rate == 1.75
                        } else {
                            (1.75..=3.035).contains(&playback.play_rate)
                        }
                    } else {
                        playback.play_rate == 1.5
                    };
                if profile_matches {
                    state.stop_playback = Some(*playback);
                    if quick {
                        state.stop_branch_seen = true;
                    }
                }
            }
            let completed = standing_stop_completed(
                state.stop_branch_seen,
                state.stop_playback.as_ref(),
                observed,
                idle,
            );
            state.coverage.standing_stop[index] |= completed;
            let capture = state.stop_playback.is_some();
            let expected_variant = if crouching_actions {
                if right > 0.5 {
                    "A_Als_Crouch_Transition_Right"
                } else {
                    "A_Als_Crouch_Transition_Left"
                }
            } else {
                match (quick, right > 0.5) {
                    (false, false) => "A_Als_Stop_Left",
                    (false, true) => "A_Als_Stop_Right",
                    (true, false) => "A_Als_Stand_Transition_Left",
                    (true, true) => "A_Als_Stand_Transition_Right",
                }
            };
            **animation_window = ManualAnimationWindowEvidence {
                label: Some(label),
                settled: capture,
                capture_source_window: capture,
                capture_every_frame: capture,
                requires_pre_rig_motion: true,
                requires_final_pose_motion: true,
                requires_non_reference_root_pose: true,
                requires_source_pose_chain: true,
                source_node: Some(source_node),
                source_state: Some(branch),
                source_pose_chain: if quick {
                    quick_stop_source_pose_chain()
                } else {
                    generated_stop_source_pose_chain(crouching_actions)
                },
                expected_animation_variant: Some(expected_variant),
                observed_animation_variant: observed
                    .map(bevy_als::stop_link::source_transition_sequence),
                observed_animation_clip_time: observed.map(|playback| playback.clip_time),
                observed_animation_weight: observed.map(|playback| playback.weight),
                observed_animation_play_rate: observed.map(|playback| playback.play_rate),
                expected_lifecycle_stages: &["playing", "completed", "idle", "settled"],
                observed_lifecycle_stages: if completed {
                    &["completed", "idle", "settled"]
                } else if slot_ready {
                    &["playing"]
                } else {
                    &[]
                },
                expected_terminal_parent_runtime_state: Some(0),
                expected_terminal_parent_transition_active: Some(false),
                expected_terminal_requested_parent_transition_active: Some(false),
                observed_parent_runtime_state: parent_state,
                observed_parent_transition_active: parent_transition,
                observed_requested_parent_runtime_state: parent_state,
                observed_requested_parent_transition_active: parent_transition,
                canonical_clip_time: Some(0.8),
                maximum_clip_time_distance: 0.06,
                expected_direction: Some(if right > 0.5 { "Right" } else { "Left" }),
                observed_direction: observed.map(|playback| {
                    if playback.index > 0.5 {
                        "Right"
                    } else {
                        "Left"
                    }
                }),
                direction_signature_group: if crouching_actions {
                    (index >= 2).then_some("crouch_quick_stop_velocity")
                } else {
                    match index {
                        0 | 1 => Some("stop_lock"),
                        2 | 3 => Some("stop_plant"),
                        5 | 6 => Some("quick_stop_velocity"),
                        _ => None,
                    }
                },
                required_direction_signature_members: 2,
                minimum_direction_signature_rotation_radians: 1e-4,
                ..default()
            };
        }
        if state.phase == IntegrationPhase::StandingDynamic {
            let index = state.stop_case_index;
            let (label, right) = STANDING_DYNAMIC_CASES[index];
            let scale =
                graph_global_f32(player, "locomotion_state.scale_world_space").unwrap_or(1.0);
            state.turn_probe.idle_ready = idle
                && speed <= 0.01
                && transition_playback.is_none_or(|playback| !playback.active);
            if let Some(arena) = arena
                && let Ok(source) = arena
                    .get_toplevel()
                    .node_states
                    .get::<SourceInstanceProperties>(NodeId::default(), StateKey::Default)
                && source
                    .write_frames
                    .get("dynamic_transitions.queued_sequence")
                    == Some(&arena.frame_index())
            {
                let distance = ["left", "right"]
                    .map(|side| graph_dynamic_distance_squared_cm(player, side).unwrap_or(0.0));
                let allowed = ["left", "right"].map(|side| {
                    graph_global_f32(player, &format!("feet_state.{side}.lock_amount"))
                        .unwrap_or(0.0)
                        > 1e-5
                });
                let threshold = dynamic_threshold_squared_cm(scale);
                let allowed = [
                    allowed[0] && distance[0] > threshold,
                    allowed[1] && distance[1] > threshold,
                ];
                let expected = if !allowed[0] || (allowed[1] && distance[1] > distance[0]) {
                    1.0
                } else {
                    0.0
                };
                let queued = source
                    .values
                    .get("dynamic_transitions.queued_sequence")
                    .and_then(|value| value.as_f32().ok())
                    .map(|index| dynamic_transition_side(crouching_actions, index));
                state.dynamic_selection_valid &= (allowed[0] || allowed[1])
                    && queued == Some(expected)
                    && player
                        .get_global_input_data("transitions_state.b_transitions_allowed")
                        .and_then(|value| value.as_bool().ok())
                        == Some(true);
                state.stop_branch_seen |=
                    queued == Some(right) && state.turn_probe.started_at.is_some();
            }
            let observed = standing_dynamic_observed(
                transition_stack,
                state.stop_playback.as_ref(),
                if crouching_actions {
                    AlsTransitionKind::CrouchingDynamic
                } else {
                    AlsTransitionKind::StandingDynamic
                },
                right,
            );
            let slot_ready = root_transition_slot
                .is_some_and(|slot| slot.active && slot.pose_ready && slot.weight > 0.05);
            if let Some(playback) =
                observed.filter(|playback| playback.active && slot_ready && state.stop_branch_seen)
            {
                if playback.start_time == 0.0
                    && playback.play_rate == 1.5
                    && playback.blend_in == 0.2
                    && playback.blend_out == 0.2
                {
                    state.stop_playback = Some(*playback);
                    state.coverage.standing_graph |= idle;
                }
            }
            let foot = foot_lock.map(|feet| if right > 0.5 { &feet.right } else { &feet.left });
            state.dynamic_unlocked |=
                state.stop_playback.is_some() && foot.is_some_and(|foot| foot.lock_amount < 0.1);
            let restored = standing_dynamic_foot_restored(state.dynamic_unlocked, foot, scale);
            let completed = state.dynamic_selection_valid
                && restored
                && idle
                && standing_dynamic_stopped(
                    state.stop_branch_seen,
                    state.stop_playback.as_ref(),
                    observed,
                    idle,
                )
                && root_transition_slot.is_some_and(|slot| !slot.active);
            state.coverage.standing_dynamic[index] |= completed;
            let capture = state.stop_playback.is_some() && (observed.is_some() || completed);
            let expected_variant = if crouching_actions {
                if right > 0.5 {
                    "A_Als_Crouch_DynamicTransition_Right"
                } else {
                    "A_Als_Crouch_DynamicTransition_Left"
                }
            } else {
                if right > 0.5 {
                    "A_Als_Stand_DynamicTransition_Right"
                } else {
                    "A_Als_Stand_DynamicTransition_Left"
                }
            };
            let chain = vec![
                ManualAnimationPoseBoundary {
                    label: "root_transition_slot",
                    graph: "ALS/Character/AB_Als/AnimGraph.animgraph.ron",
                    node: "Slot 'Transition'\nGroup 'Grounded'",
                    output: "Pose",
                    minimum_translation_span: 0.0,
                    minimum_rotation_span_radians: 0.05,
                },
                ManualAnimationPoseBoundary {
                    label: "locomotion_output",
                    graph: "ALS/Character/AB_Als/AnimGraph.animgraph.ron",
                    node: "AB_Als_Locomotion\nLinked Anim Graph",
                    output: "Pose",
                    minimum_translation_span: 0.0,
                    minimum_rotation_span_radians: 0.05,
                },
            ];
            **animation_window = ManualAnimationWindowEvidence {
                label: Some(label),
                settled: capture,
                capture_source_window: capture,
                capture_every_frame: capture,
                requires_pre_rig_motion: true,
                requires_final_pose_motion: true,
                requires_non_reference_root_pose: true,
                requires_source_pose_chain: true,
                source_node: Some(if crouching_actions {
                    "AB_Als_Crouching/Idle/RefreshDynamicTransitions"
                } else {
                    "AB_Als_Standing/Idle/RefreshDynamicTransitions"
                }),
                source_state: Some("Idle"),
                source_pose_chain: chain,
                expected_animation_variant: Some(expected_variant),
                observed_animation_variant: observed
                    .map(bevy_als::stop_link::source_transition_sequence),
                observed_animation_clip_time: observed.map(|playback| playback.clip_time),
                observed_animation_weight: observed.map(|playback| playback.weight),
                observed_animation_play_rate: observed.map(|playback| playback.play_rate),
                expected_lifecycle_stages: &["playing", "stopped", "foot_relocked", "settled"],
                observed_lifecycle_stages: if completed {
                    &["stopped", "foot_relocked", "settled"]
                } else if slot_ready {
                    &["playing"]
                } else {
                    &[]
                },
                expected_terminal_parent_runtime_state: Some(0),
                expected_terminal_parent_transition_active: Some(false),
                expected_terminal_requested_parent_transition_active: Some(false),
                observed_parent_runtime_state: parent_state,
                observed_parent_transition_active: parent_transition,
                observed_requested_parent_runtime_state: parent_state,
                observed_requested_parent_transition_active: parent_transition,
                canonical_clip_time: Some(0.4),
                maximum_clip_time_distance: 0.06,
                expected_direction: Some(if right > 0.5 { "Right" } else { "Left" }),
                observed_direction: observed.map(|playback| {
                    if playback.index > 0.5 {
                        "Right"
                    } else {
                        "Left"
                    }
                }),
                direction_signature_group: Some("standing_dynamic"),
                required_direction_signature_members: 2,
                minimum_direction_signature_rotation_radians: 1e-4,
                ..default()
            };
        }
        if state.phase == IntegrationPhase::StandingTurn {
            let index = state.turn_case_index;
            let (mut label, mut expected_variant, yaw_degrees) = STANDING_TURN_CASES[index];
            if crouching_actions {
                expected_variant = if index == 2 {
                    "A_Als_Crouch_Turn_180_Left"
                } else {
                    "A_Als_Crouch_Turn_180_Right"
                };
            }
            let rotating = matches!(index, 4 | 5 | 7);
            let driven_elapsed = state
                .turn_probe
                .started_at
                .map_or(0.0, |started| state.phase_elapsed() - started);
            if index == 7 {
                label = if driven_elapsed < 4.0 {
                    "aim_fast_left"
                } else if driven_elapsed < 8.0 {
                    "aim_fast_right"
                } else {
                    "aim_fast_reversals"
                };
                if state.turn_probe.started_at.is_some() {
                    let yaw = world_forward_to_yaw(*transform.forward());
                    if let Some(previous) = state.turn_probe.previous_yaw {
                        let delta = bevy_als::als::turn_in_place::wrap_angle_degrees(
                            (yaw - previous).to_degrees(),
                        )
                        .to_radians();
                        state.turn_probe.yaw_travel[usize::from(delta < 0.0)] += delta.abs();
                    }
                    state.turn_probe.previous_yaw = Some(yaw);
                }
            }
            let slot_idle = turn_slot.is_some_and(|slot| !slot.active && slot.weight <= 0.05);
            let epoch = turn_drive.map_or(0.0, |drive| drive.inertialization_request_epoch);
            let mode_matches = motion_probe.rotation_mode.get(character).is_ok_and(|mode| {
                *mode
                    == if rotating {
                        RotationMode::Aiming
                    } else {
                        RotationMode::ViewDirection
                    }
            });
            let rotation_settled = standing_turn_rotation_settled(
                index,
                bevy_als::turn_layer::character_turn_angle_from_radians(
                    view_yaw.0,
                    world_forward_to_yaw(*transform.forward()).to_degrees(),
                ),
                rotation_animation.map_or(f32::NAN, |animation| animation.yaw_speed_degrees),
                turn_drive.is_none_or(|drive| drive.rotate_active),
            );
            if state.turn_probe.started_at.is_none() {
                state.turn_probe.idle_ready = mode_matches
                    && idle
                    && slot_idle
                    && speed <= 0.01
                    && rotation_settled
                    && transition_playback.is_none_or(|playback| !playback.active);
                state.turn_probe.epoch_baseline = epoch;
            }
            let entry = turn_stack
                .and_then(|stack| stack.entries.last())
                .filter(|entry| entry.crouching == crouching_actions && entry.sampling_ready);
            let actual_variant = if rotating {
                rotate_sample.map(|sample| sample.0)
            } else {
                entry.map(|entry| {
                    bevy_als::turn_layer::source_turn_sequence(
                        crouching_actions,
                        entry.is_180,
                        entry.lr,
                    )
                })
            };
            let clip_time = if rotating {
                rotate_sample.map(|sample| sample.1)
            } else {
                entry.map(|entry| entry.elapsed)
            };
            let rate = if rotating {
                turn_drive.map(|drive| drive.rotate_rate)
            } else {
                entry.map(|entry| entry.play_rate)
            };
            let expected_state = if rotating {
                if yaw_degrees > 0.0 { 3 } else { 4 }
            } else {
                0
            };
            let active = state.turn_probe.started_at.is_some()
                && mode_matches
                && (parent_pose_ready == Some(true)
                    && if index == 7 {
                        matches!(parent_state, Some(3 | 4)) || parent_transition == Some(true)
                    } else {
                        parent_state == Some(expected_state) && parent_transition == Some(false)
                    })
                && if rotating {
                    rotate_sample.is_some()
                } else {
                    turn_slot
                        .is_some_and(|slot| slot.active && slot.pose_ready && slot.weight > 0.05)
                }
                && actual_variant.is_some_and(|variant| index >= 6 || variant == expected_variant);
            if active {
                state.coverage.standing_graph = true;
                state.turn_probe.active_seen = true;
                state.turn_probe.clip_peak =
                    state.turn_probe.clip_peak.max(clip_time.unwrap_or(0.0));
                state.turn_probe.max_play_rate =
                    state.turn_probe.max_play_rate.max(rate.unwrap_or(0.0));
                state.turn_probe.invalid_rate |= rate.is_none_or(|rate| {
                    if rotating {
                        !(1.15 - 1e-4..=3.0 + 1e-4).contains(&rate)
                    } else {
                        (rate - 1.2).abs() > 1e-6
                    }
                });
                if index == 7 {
                    state.turn_probe.sides_mask |=
                        if actual_variant.is_some_and(|variant| variant.ends_with("Left")) {
                            1
                        } else {
                            2
                        };
                }
                if let Some(entry) = entry.filter(|_| !rotating) {
                    state.turn_probe.sides_mask |= if entry.lr <= 0.5 { 1 } else { 2 };
                }
            }
            let completed = standing_turn_completed(
                index,
                &state.turn_probe,
                idle && rotation_settled,
                slot_idle,
                epoch,
                driven_elapsed,
            );
            state.coverage.standing_turn[index] |= completed;
            let mut chain = if crouching_actions {
                crouch_turn_source_pose_chain()
            } else {
                generated_stop_source_pose_chain(false)
            };
            if !crouching_actions && rotating && index != 7 {
                chain.insert(0, ManualAnimationPoseBoundary {
                    label: "standing_rotate_sequence",
                    graph: if yaw_degrees > 0.0 { "ALS/Character/AnimationInstances/Stances/AB_Als_Standing/AnimGraph/SM6_Standing_States/S6_Rotate_Left.animgraph.ron" }
                        else { "ALS/Character/AnimationInstances/Stances/AB_Als_Standing/AnimGraph/SM6_Standing_States/S5_Rotate_Right.animgraph.ron" },
                    node: if yaw_degrees > 0.0 { "A_Als_Rotate_90_Left\nSequence Player" } else { "A_Als_Rotate_90_Right\nSequence Player" },
                    output: "Pose", minimum_translation_span: 0.0, minimum_rotation_span_radians: 0.05,
                });
            } else if !crouching_actions && !rotating {
                chain.insert(0, ManualAnimationPoseBoundary {
                    label: "standing_turn_slot",
                    graph: "ALS/Character/AnimationInstances/Stances/AB_Als_Standing/AnimGraph/SM6_Standing_States/S0_Idle.animgraph.ron",
                    node: "Slot 'TurnInPlaceStanding'\nGroup 'Grounded'", output: "Pose",
                    minimum_translation_span: 0.0, minimum_rotation_span_radians: 0.05,
                });
            }
            **animation_window = ManualAnimationWindowEvidence {
                label: Some(label),
                settled: active || completed,
                capture_source_window: (active && (index != 7 || driven_elapsed >= 0.75))
                    || completed,
                capture_every_frame: true,
                requires_pre_rig_motion: active,
                requires_final_pose_motion: active,
                requires_non_reference_root_pose: active,
                requires_source_pose_chain: active,
                requires_actor_yaw_motion: active,
                expected_actor_yaw_baseline: Some(state.turn_probe.base_yaw),
                expected_actor_yaw_delta_sign: (index < 6).then_some(if yaw_degrees > 0.0 {
                    1
                } else {
                    -1
                }),
                minimum_actor_yaw_delta_radians: if crouching_actions {
                    3.0
                } else if index >= 6 {
                    0.5
                } else {
                    1.4
                },
                minimum_pre_rig_rotation_span_radians: 0.05,
                minimum_final_rotation_span_radians: 0.05,
                source_node: Some(source_node),
                source_state: Some(if rotating { "Rotate" } else { "Idle" }),
                source_pose_chain: chain,
                expected_animation_variant: (index < 6).then_some(expected_variant),
                expected_parent_runtime_state: (index != 7).then_some(expected_state),
                observed_parent_runtime_state: parent_state,
                observed_parent_transition_active: parent_transition,
                observed_requested_parent_runtime_state: parent_state,
                observed_requested_parent_transition_active: parent_transition,
                expected_terminal_parent_runtime_state: (index != 7 || driven_elapsed >= 8.0)
                    .then_some(0),
                expected_terminal_parent_transition_active: (index != 7 || driven_elapsed >= 8.0)
                    .then_some(false),
                expected_terminal_requested_parent_transition_active: (index != 7
                    || driven_elapsed >= 8.0)
                    .then_some(false),
                expected_lifecycle_stages: if index == 7 && driven_elapsed < 8.0 {
                    &["playing"]
                } else {
                    &["playing", "settled"]
                },
                observed_lifecycle_stages: if completed {
                    &["settled"]
                } else if active {
                    &["playing"]
                } else {
                    &[]
                },
                canonical_clip_time: (index < 6).then_some(if rotating { 0.5 } else { 0.6 }),
                maximum_clip_time_distance: 0.06,
                direction_signature_group: match index {
                    0 | 1 => Some("standing_turn_90"),
                    2 | 3 => Some("standing_turn_180"),
                    4 | 5 => Some("standing_rotate"),
                    _ => None,
                },
                required_direction_signature_members: 2,
                minimum_direction_signature_rotation_radians: 1e-4,
                expected_direction: (index < 6).then_some(if yaw_degrees > 0.0 {
                    "Left"
                } else {
                    "Right"
                }),
                observed_direction: actual_variant.map(|variant| {
                    if variant.ends_with("Left") {
                        "Left"
                    } else {
                        "Right"
                    }
                }),
                ..default()
            };
            record_animation_window_trace(
                &mut animation_window,
                transform,
                view_yaw.0,
                rotation_animation,
                None,
                turn_drive,
                turn_stack,
                crouching_actions,
                actual_variant,
            );
            if let Some(clip_time) = clip_time {
                animation_window.observed_animation_clip_time = Some(clip_time);
            }
            if let Some(rate) = rate {
                animation_window.observed_animation_play_rate = Some(rate);
            }
            animation_window.observed_animation_weight = Some(if rotating {
                if active { 1.0 } else { 0.0 }
            } else {
                entry.map_or(0.0, |entry| entry.weight())
            });
            animation_window.observed_animation_curve_direct_yaw_speed_degrees = if rotating {
                rotate_sample.map(|sample| sample.2)
            } else {
                entry.map(|entry| {
                    bevy_als::standing_source::MONTAGES[bevy_als::turn_layer::source_turn_sequence(
                        crouching_actions,
                        entry.is_180,
                        entry.lr,
                    )]
                    .curves
                    .sample_all(entry.elapsed)
                    .get(&bevy_als::extensions::SourceCurveName::from(
                        "RotationYawSpeed",
                    ))
                    .copied()
                    .unwrap_or(0.0)
                })
            };
        }
        gate_report.direction_trace.push(serde_json::json!({
            "elapsed": state.phase_elapsed(), "case": if state.scenario == IntegrationScenario::StandingResponse { Some(standing_response_step(state.phase_elapsed()).0) } else if state.phase == IntegrationPhase::StandingStop { Some(stop_cases(crouching_actions)[state.stop_case_index].0) } else if state.phase == IntegrationPhase::StandingTurn { Some(STANDING_TURN_CASES[state.turn_case_index].0) } else if state.phase == IntegrationPhase::StandingDynamic { Some(STANDING_DYNAMIC_CASES[state.stop_case_index].0) } else if state.scenario == IntegrationScenario::Crouching { state.crouch_stationary_validation.map(|value| match value { CrouchStationaryValidation::TurnLeft => "crouch_turn_left", CrouchStationaryValidation::TurnRight => "crouch_turn_right", CrouchStationaryValidation::RotateLeft => "crouch_rotate_left", CrouchStationaryValidation::RotateRight => "crouch_rotate_right" }).or_else(|| crouch_direction_regression_case(state.crouch_direction_case_index).map(|case| case.name)) } else { direction_regression_case(state.direction_case_index).map(|case| case.name) },
            "input": inputs.and_then(|(input, _)| input).map(|input| input.move_dir.to_array()),
            "override": inputs.and_then(|(_, input)| input).map(|input| input.0.move_dir.to_array()),
            "keys": ([KeyCode::KeyW, KeyCode::KeyA, KeyCode::KeyS, KeyCode::KeyD].map(|key| motion_probe.keys.pressed(key))),
            "speed": speed, "velocity": velocity.0.to_array(), "view_yaw": view_yaw.0, "body_yaw": world_forward_to_yaw(*transform.forward()),
            "gait": format!("{gait:?}"), "locomotion": format!("{locomotion:?}"), "values": values,
            "rotation_mode": motion_probe.rotation_mode.get(character).ok().map(|mode| format!("{mode:?}")),
            "pose_moving": context.and_then(|context| context.f32("PoseMoving")),
            "pose": pose.map(|pose| movement_pose_sample(pose, &motion_probe.skeletons)),
            "samples": samples, "machines": machines,
            "transition": transition_playback.map(|value| (format!("{:?}", value.kind), value.active, value.clip_time, value.weight)),
            "transition_profile": transition_playback.map(|value| (value.index, value.start_time, value.play_rate, value.blend_in, value.blend_out, value.elapsed, value.stop_elapsed)),
            "foot_planted": graph_global_f32(player, "feet_state.foot_planted_amount"),
            "allow_transitions": context.and_then(|context| context.f32("AllowTransitions")),
            "transitions_allowed": player.get_global_input_data("transitions_state.b_transitions_allowed").and_then(|value| value.as_bool().ok()),
            "slot_weight": graph_global_f32(player, "transition.weight"),
            "turn_stack": turn_stack.map(|stack| stack.entries.iter().map(|entry| serde_json::json!({"id":entry.id,"variant":bevy_als::turn_layer::source_turn_sequence(entry.crouching,entry.is_180,entry.lr),"clip":entry.elapsed,"rate":entry.play_rate,"weight":entry.weight(),"ready":entry.sampling_ready})).collect::<Vec<_>>()),
            "turn_curve_rate": graph_global_f32(player, "turn_in_place_state.play_rate"),
            "rotation_curve": context.and_then(|context| context.f32("RotationYawSpeed")),
            "actor_rotation_curve": rotation_animation.map(|animation| animation.yaw_speed_degrees),
            "rotate_sample": rotate_sample,
            "rotate_rate": turn_drive.map(|drive| drive.rotate_rate),
            "look_delta": inputs.and_then(|(_, input)| input).map(|input| input.0.look_delta.to_array()),
            "turn_yaw_travel": (state.scenario == IntegrationScenario::StandingTurn).then_some(state.turn_probe.yaw_travel),
            "leg_joints": foot_ik_data.get(character).ok().map(|(_, _, apply, _)| [(apply.left.hip_world.to_array(), apply.left.knee_world.to_array(), apply.left.ankle_world.to_array()), (apply.right.hip_world.to_array(), apply.right.knee_world.to_array(), apply.right.ankle_world.to_array())]),
            "foot_locks": foot_lock.map(|feet| (feet.left.lock_amount,feet.left.lock_location.to_array(),feet.right.lock_amount,feet.right.lock_location.to_array())),
            "dynamic_inputs": (["left", "right"].map(|side| (graph_global_f32(player, &format!("feet_state.{side}.lock_amount")), graph_dynamic_distance_squared_cm(player, side).map(|distance| distance * 0.0001)))),
            "dynamic_probe": (state.phase == IntegrationPhase::StandingDynamic).then_some((state.stop_branch_seen, state.dynamic_selection_valid, state.dynamic_unlocked)),
            "dynamic_instance": (state.phase == IntegrationPhase::StandingDynamic).then(|| state.stop_playback.map(|playback|
                (playback.instance_id, playback.clip_time, transition_stack.is_some_and(|stack| stack.entries.iter().any(|entry| entry.instance_id == playback.instance_id))))),
            "slot_allow_transitions": motion_probe.source_inputs.get(instance.player_entity()).ok().and_then(|inputs| inputs.slot_input_context("transition.pose")).and_then(|context| context.curves.get(&bevy_als::extensions::SourceCurveName::from("AllowTransitions")).copied()),
            "native_dynamic": arena.and_then(|arena| arena.get_toplevel().node_states.get::<SourceInstanceProperties>(NodeId::default(), StateKey::Default).ok().map(|source| (
                arena.frame_index(),
                source.values.get("dynamic_transitions_state.frame_delay").and_then(|value| value.as_f32().ok()),
                source.write_frames.get("dynamic_transitions.queued_sequence").copied()
            ))),
            "feet": foot_ik_data.get(character).ok().map(|(_, spaces, _, _)| (spaces.left.trace_target_world.to_array(), spaces.left.target_world.to_array(), spaces.right.trace_target_world.to_array(), spaces.right.target_world.to_array())),
        }));
        if state.scenario == IntegrationScenario::StandingResponse {
            let elapsed = state.phase_elapsed();
            state.coverage.response_reverse |= (4.5..5.5).contains(&elapsed) && speed > 0.1;
            state.coverage.response_cycle |= (8.0..8.5).contains(&elapsed) && speed > 0.1;
            if elapsed >= 11.5 {
                state.coverage.response_idle &=
                    standing_graph.is_some_and(|graph| {
                        graph.actual_standing_state == Some(0)
                            && graph.actual_standing_transition_active == Some(false)
                    }) && context.and_then(|context| context.f32("PoseMoving")) == Some(0.0)
                        && transition_playback.is_none_or(|value| !value.active);
            }
        }
    }
    if state.scenario == IntegrationScenario::Sprint && state.phase == IntegrationPhase::Sprint {
        let native = source_sprint_values(player);
        let arena = player.get_context_arena();
        let phase = arena.and_then(|arena| {
            bevy_als::extensions::sync_group_phase::source_sync_phase(
                arena,
                &motion_probe.graphs,
                "sync_groups.movement.phase",
            )
        });
        let mut weights = [0.0_f32; 2];
        let mut rates = [0.0_f32; 2];
        if let Some(arena) = arena
            && let Some(ticks) = bevy_als::extensions::sync_group_phase::source_sync_players(
                arena,
                &motion_probe.graphs,
                "sync_groups.movement.phase",
            )
        {
            for tick in ticks {
                let node = arena
                    .get_context(tick.identity.0)
                    .and_then(|context| motion_probe.graphs.get(context.get_graph_id()))
                    .and_then(|graph| graph.nodes.get(&tick.identity.1));
                let index = node.and_then(|node| match node.name.as_str() {
                    "A_Als_Sprint\nSequence Player\nSync group Movement" => Some(0),
                    "A_Als_Sprint_Acceleration\nSequence Player\nSync group Movement" => Some(1),
                    _ => None,
                });
                if let Some(index) = index {
                    weights[index] += tick.weight;
                    rates[index] = tick.play_rate;
                }
            }
        }
        gate_report.sprint_trace.push(serde_json::json!({
            "elapsed": state.phase_elapsed(), "native": native,
            "actual_locomotion": format!("{locomotion:?}"), "actual_gait": format!("{gait:?}"),
            "actual_stance": format!("{stance:?}"), "speed_y": velocity.y, "actor_y": transform.translation.y,
            "grounded_state": grounded_graph.and_then(|state| state.actual_state),
            "standing_state": standing_graph.and_then(|state| state.actual_standing_state),
            "standing_transition": standing_graph.and_then(|state| state.actual_standing_transition_active),
            "pose_moving": context.and_then(|value| value.f32("PoseMoving")),
            "pose_grounded": context.and_then(|value| value.f32("PoseGrounded")),
            "moving_smooth": player.get_global_input_data("locomotion_state.b_moving_smooth").and_then(|value| value.as_bool().ok()),
            "gait_flag": graph_global_f32(player, "als_standing_gait_is_sprinting"),
            "phase": phase, "weights": weights, "rates": rates, "speed": speed,
            "play_rate": graph_global_f32(player, "standing_state.play_rate"),
            "stride": graph_global_f32(player, "standing_state.stride_blend_amount"),
            "gait_running": graph_global_f32(player, "pose_state.unweighted_gait_running_amount"),
        }));
        if *gait == Gait::Sprinting
            && let Some((sprint_time, acceleration, true)) = native
            && let Some(phase) = phase
            && let Some(telemetry) = standing_graph
        {
            let accelerating = weights[1] > 1e-5;
            let steady = weights[0] > 1e-5 && !accelerating;
            if accelerating && sprint_time < 0.5 && acceleration > 0.0 {
                state.sprint_signature_phase.get_or_insert(phase);
                state.sprint_samples[0].get_or_insert((sprint_time, acceleration));
            }
            if steady && sprint_time >= 0.5 && acceleration == 0.0 {
                state.sprint_samples[1].get_or_insert((sprint_time, acceleration));
            }
            if (accelerating || steady)
                && let Some(canonical_phase) = state.sprint_signature_phase
            {
                **animation_window = sprint_window(accelerating, canonical_phase, telemetry);
            }
        }
        let standing_global = graph_global_f32(player, "als_standing_curve_pose_moving");
        let final_pose_moving = context.and_then(|value| value.f32("PoseMoving"));
        let final_left_ik = context.and_then(|value| value.f32("FootLeftIk"));
        let final_right_ik = context.and_then(|value| value.f32("FootRightIk"));
        let applied = foot_ik_data
            .get(character)
            .ok()
            .map(|(_, _, apply, _)| (apply.left.applied_weight, apply.right.applied_weight));
        let relay = source_ready
            && *gait == Gait::Sprinting
            && speed > 0.1
            && standing_global.is_some_and(|value| value >= 0.999)
            && final_pose_moving.is_some_and(|value| value >= 0.999)
            && final_left_ik.is_some_and(|value| value >= 0.999)
            && final_right_ik.is_some_and(|value| value >= 0.999)
            && applied.is_some_and(|(left, right)| left >= 0.999 && right >= 0.999);
        if relay && !state.coverage.sprint_curve_relay {
            info!(
                "[MANUAL-INTEGRATION][SPRINT-CURVE-RELAY] global={:.3} final={:.3} left_ik={:.3} right_ik={:.3} applied=({:.3},{:.3}) -> PASS",
                standing_global.unwrap_or_default(),
                final_pose_moving.unwrap_or_default(),
                final_left_ik.unwrap_or_default(),
                final_right_ik.unwrap_or_default(),
                applied.map_or(0.0, |value| value.0),
                applied.map_or(0.0, |value| value.1),
            );
        }
        state.coverage.sprint_curve_relay |= relay;
    }
    state.coverage.locomotion |= speed > 0.25 && direction.is_some();
    if speed > 0.5 && direction.is_some_and(|value| value.abs() > 150.0) {
        state.coverage.backward = true;
    }
    if speed > 0.5 && direction.is_some_and(|value| (60.0..120.0).contains(&value.abs())) {
        state.coverage.sideways = true;
    }
    state.coverage.crouching |= *stance == Stance::Crouching && speed > 0.1;
    state.coverage.sprinting |= *gait == Gait::Sprinting && speed > 0.1;
    if *locomotion == LocomotionState::Air && jump_phase.is_some() {
        state.coverage.airborne = true;
    }
    if state.coverage.airborne
        && *locomotion == LocomotionState::Grounded
        && land_phase.is_some_and(|value| value > 1e-4)
    {
        state.coverage.landed = true;
    }
    state.coverage.overlay |=
        *overlay == OverlayState::Rifle && overlay_tag.as_ref() == Some(&overlay.tag());
    state.coverage.gait_layer |=
        gait_blend.is_some_and(|blend| blend.weights.sprint_w > 1e-4) && *gait == Gait::Sprinting;
    state.coverage.stance_layer |=
        stance_blend.is_some_and(|blend| blend.weight > 1e-4) && *stance == Stance::Crouching;
    state.coverage.jump_variant |=
        jump_variant.is_some_and(|variant| variant.airborne) && *locomotion == LocomotionState::Air;
    state.coverage.turn_in_place |= turn_drive.is_some_and(|drive| {
        (drive.playing && drive.weight > 1e-4)
            || (drive.rotate_active && drive.rotate_weight > 1e-4)
    });
    state.coverage.layering |= *stance == Stance::Crouching
        && layering.is_some()
        && layering_crouching_input_ready(player);
    state.coverage.overlay_object |= overlay_object.is_some_and(|object| {
        object.visible
            && object.applied_overlay == Some(OverlayState::Rifle)
            && object
                .prop
                .is_some_and(|prop| interface.overlay_props.get(prop).is_ok())
    });
    state.coverage.start_stop |= start_stop.is_some_and(|value| {
        matches!(
            value.phase,
            StartStopPhase::Started(_) | StartStopPhase::Stopping(_)
        )
    });
    if matches!(
        state.scenario,
        IntegrationScenario::StateMachine | IntegrationScenario::TransitionNotify
    ) {
        let count = motion_probe
            .transition_notify
            .get(character)
            .ok()
            .flatten()
            .map_or(0, |telemetry| telemetry.stop_quick_count);
        let observed = if state.phase == IntegrationPhase::TransitionNotify {
            let released = state.generated_stop_quick_input_released;
            generated_stop_quick_observed(
                &mut state.generated_stop_quick_count_before,
                released,
                count,
            )
        } else {
            count > 0
        };
        state.coverage.generated_transition_stop_quick |= observed;
        if let Some(telemetry) = standing_graph {
            let previous_mask = state.p1_ground_mask;
            if source_ready && telemetry.outer_state == Some(0) {
                state.p1_ground_mask |= 1 << 0;
            }
            if source_ready
                && telemetry.outer_state == Some(1)
                && telemetry.actual_movement_state.is_some()
                && speed > 0.5
            {
                state.p1_ground_mask |= 1 << 1;
            }
            if source_ready && telemetry.outer_state == Some(2) {
                state.p1_ground_mask |= 1 << 2;
            }
            if source_ready
                && telemetry.outer_state == Some(1)
                && matches!(telemetry.detail_state, Some(2) | Some(5))
            {
                state.p1_ground_mask |= 1 << 3;
            }
            if state.p1_ground_mask != previous_mask {
                info!(
                    "[MANUAL-INTEGRATION][P1][GROUND] mask={:04b} outer={:?} outer_transition={:?} detail={:?} detail_transition={:?} movement={:?} weight={:.3} speed={speed:.3}",
                    state.p1_ground_mask,
                    telemetry.outer_state,
                    telemetry.outer_transition_active,
                    telemetry.detail_state,
                    telemetry.detail_transition_active,
                    telemetry.actual_movement_state,
                    telemetry.dominant_detail_weight.unwrap_or_default(),
                );
            }
        }
        state.coverage.p1_ground = state.p1_ground_mask == 0b1111;

        if let Some((desired, allowed)) = gait_state {
            let previous_mask = state.p1_gait_mask;
            if desired.0 == Gait::Walking && allowed.0 == Gait::Walking && *gait == Gait::Walking {
                state.p1_gait_mask |= 1 << 0;
            }
            if desired.0 == Gait::Running && allowed.0 == Gait::Running && *gait == Gait::Running {
                state.p1_gait_mask |= 1 << 1;
            }
            if desired.0 == Gait::Sprinting
                && allowed.0 == Gait::Sprinting
                && *gait == Gait::Sprinting
            {
                state.p1_gait_mask |= 1 << 2;
            }
            if desired.0 == Gait::Sprinting && allowed.0 == Gait::Running && *gait == Gait::Running
            {
                state.p1_gait_mask |= 1 << 3;
            }
            if state.p1_gait_mask != previous_mask {
                let (local_x, local_y) = actual_motion
                    .map(|motion| (motion.local_velocity.x, motion.local_velocity.y))
                    .unwrap_or_default();
                info!(
                    "[MANUAL-INTEGRATION][P1][GAIT] mask={:04b} desired={:?} allowed={:?} actual={:?} local=({local_x:.3},{local_y:.3}) speed={speed:.3}",
                    state.p1_gait_mask, desired.0, allowed.0, gait,
                );
            }
        }
        state.coverage.p1_gait = state.p1_gait_mask == 0b1111;

        if let (Some(transaction), Some(telemetry)) = (crouch_transaction, grounded_graph) {
            let previous_mask = state.p1_stance_mask;
            if transaction.actual_stance == Stance::Crouching
                && telemetry.actual_state == Some(2)
                && telemetry.pose_ready == Some(true)
            {
                state.p1_stance_mask |= 1 << 0;
            }
            if state.p1_stance_mask & 1 != 0
                && transaction.actual_stance == Stance::Standing
                && telemetry.actual_state == Some(1)
                && telemetry.pose_ready == Some(true)
            {
                state.p1_stance_mask |= 1 << 1;
            }
            if state.p1_stance_mask != previous_mask {
                info!(
                    "[MANUAL-INTEGRATION][P1][STANCE] mask={:02b} desired={:?} actual={:?} phase={:?} collider={:.3} grounded={:?}",
                    state.p1_stance_mask,
                    transaction.desired_stance,
                    transaction.actual_stance,
                    transaction.phase,
                    transaction.collider_half_height,
                    telemetry.actual_state,
                );
            }
        }
        state.coverage.p1_stance = state.p1_stance_mask == 0b11;

        let previous_air_mask = state.p1_air_mask;
        if *locomotion == LocomotionState::Air
            && jump_variant.is_some_and(|variant| variant.airborne)
        {
            state.p1_air_mask |= 1 << 0;
        }
        if *locomotion == LocomotionState::Air && velocity.0.y < -0.05 {
            state.p1_air_mask |= 1 << 1;
        }
        if state.p1_air_mask & 0b11 == 0b11
            && *locomotion == LocomotionState::Grounded
            && land_phase.is_some_and(|value| value > 1e-4)
        {
            state.p1_air_mask |= 1 << 2;
        }
        if state.p1_air_mask != previous_air_mask {
            info!(
                "[MANUAL-INTEGRATION][P1][AIR] mask={:03b} locomotion={:?} vertical={:.3} jump={:?} land={:?}",
                state.p1_air_mask,
                locomotion,
                velocity.0.y,
                jump_variant.map(|variant| variant.airborne),
                land_phase,
            );
        }
        state.coverage.p1_air = state.p1_air_mask == 0b111;

        if let Some(drive) = turn_drive {
            let previous_mask = state.p1_turn_mask;
            state.p1_turn_mask |= p1_turn_evidence_mask(drive, standing_graph);
            if state.p1_turn_mask != previous_mask {
                info!(
                    "[MANUAL-INTEGRATION][P1][TURN] mask={:04b} turn=(playing={},lr={:.1},weight={:.3}) slot=(active={},ready={},weight={:.3},inertia_epoch={:.0}) rotate=(active={},lr={:.1},rate={:.3}) standing_state={:?} standing_pose_ready={:?} first_person={}",
                    state.p1_turn_mask,
                    drive.playing,
                    drive.lr,
                    drive.weight,
                    turn_slot.is_some_and(|slot| slot.active),
                    turn_slot.is_some_and(|slot| slot.pose_ready),
                    turn_slot.map_or(0.0, |slot| slot.weight),
                    turn_slot.map_or(0.0, |slot| slot.inertialization_request_epoch),
                    drive.rotate_active,
                    drive.rotate_lr,
                    drive.rotate_rate,
                    standing_graph.and_then(|graph| graph.actual_standing_state),
                    standing_graph.and_then(|graph| graph.actual_standing_pose_ready),
                    first_person,
                );
            }
        }
        state.coverage.p1_turn = state.p1_turn_mask == 0b1111;
        let turn_slot_ready =
            turn_slot.is_some_and(|slot| slot.active && slot.pose_ready && slot.weight > 0.05);
        if turn_slot_ready && !state.coverage.turn_slot {
            warn!(
                "[MANUAL-INTEGRATION][P1][TURN-SLOT] active=true pose_ready=true weight={:.3} -> PASS",
                turn_slot.map_or(0.0, |slot| slot.weight)
            );
        }
        state.coverage.turn_slot |= turn_slot_ready;
        let turn_inertialized =
            turn_slot.is_some_and(|slot| slot.inertialization_request_epoch >= 1.0);
        if turn_inertialized && !state.coverage.turn_inertialization {
            warn!(
                "[MANUAL-INTEGRATION][P1][TURN-INERTIALIZATION] epoch={:.0} -> PASS",
                turn_slot.map_or(0.0, |slot| slot.inertialization_request_epoch)
            );
        }
        state.coverage.turn_inertialization |= turn_inertialized;
    }
    state.coverage.view_head |= state
        .initial_view_yaw
        .is_some_and(|initial| (view_yaw.0 - initial).abs() > 0.01)
        && state
            .initial_view_pitch
            .is_some_and(|initial| (view_pitch.0 - initial).abs() > 0.01)
        && head.is_some_and(|value| value.pitch_angle.abs() > 0.01 || value.yaw_angle.abs() > 0.01);
    let control_rig_ready = control_rig.is_some_and(|value| {
        value.spine_rotation_allowed
            && value.spine_amount.abs() > 1e-4
            && (state.scenario != IntegrationScenario::ControlRig
                || value.final_yaw_angle.abs() > 1e-4)
    });
    if control_rig_ready && !state.coverage.control_rig {
        if let Some(value) = control_rig {
            info!(
                "[MANUAL-INTEGRATION][CR-ALS-SPINE] allowed={} amount={:.6} final_yaw_degrees={:.6} -> PASS",
                value.spine_rotation_allowed, value.spine_amount, value.final_yaw_angle,
            );
        }
    }
    state.coverage.control_rig |= control_rig_ready;
    if state.scenario == IntegrationScenario::ControlRig
        && !state.coverage.control_rig_actions
        && let Some([left, right, spine]) =
            control_rig_action_evidence(player, &motion_probe.graphs, &motion_probe.skeletons)
    {
        gate_report.direction_trace.push(serde_json::json!({"control_rig_actions": {
            "hand_left_ik_weight": left, "hand_right_ik_weight": right, "spine_local_rotation_delta_radians": spine }}));
        info!(
            "[MANUAL-INTEGRATION][CR-ALS-ACTIONS] hand_left_weight={left:.6} hand_right_weight={right:.6} spine_local_delta_radians={spine:.6} -> PASS"
        );
        state.coverage.control_rig_actions = true;
    }
    if let Ok((construction, input)) = control_rig_runtime.get(character) {
        let input_ready = input.use_hand_ik_bones
            && input.use_foot_ik_bones
            && input.velocity_blend_forward_amount.is_finite()
            && input.velocity_blend_backward_amount.is_finite()
            && input.spine_yaw_angle_degrees.is_finite()
            && input.pelvis_offset_amount.is_finite()
            && input.foot_left_location_armature.is_finite()
            && input.foot_right_location_armature.is_finite()
            && input.foot_left_rotation_armature.is_finite()
            && input.foot_right_rotation_armature.is_finite()
            && (state.scenario != IntegrationScenario::ControlRig
                || input.velocity_blend_forward_amount.abs() > 1e-3
                || input.velocity_blend_backward_amount.abs() > 1e-3
                || input.spine_yaw_angle_degrees.abs() > 1e-3);
        let stage_ready = construction.initialization_count == 1
            && construction.executed_steps > 0
            && construction.frame > 0;
        let foot_targets_ready = foot_ik_data
            .get(character)
            .is_ok_and(|(_, space, apply, _)| {
                construction.foot_height_m.is_finite()
                    && construction.foot_height_m > 0.0
                    && construction.leg_length_m.is_finite()
                    && construction.leg_length_m > 0.0
                    && space.left.target_armature_local.is_finite()
                    && space.right.target_armature_local.is_finite()
                    && space.left.normal_armature_local.is_finite()
                    && space.right.normal_armature_local.is_finite()
                    && apply.left.target_world.is_finite()
                    && apply.right.target_world.is_finite()
            });
        if stage_ready && !state.coverage.control_rig_stages {
            info!(
                "[MANUAL-INTEGRATION][CR-ALS-STAGES] construction_count={} vm_steps={} graph_frame={} -> PASS",
                construction.initialization_count, construction.executed_steps, construction.frame,
            );
        }
        if input_ready && !state.coverage.control_rig_input {
            info!(
                "[MANUAL-INTEGRATION][CR-ALS-INPUT] forward={:.3} backward={:.3} spine_yaw={:.3} pelvis={:.3} left=({:.3},{:.3},{:.3}) right=({:.3},{:.3},{:.3}) -> PASS",
                input.velocity_blend_forward_amount,
                input.velocity_blend_backward_amount,
                input.spine_yaw_angle_degrees,
                input.pelvis_offset_amount,
                input.foot_left_location_armature.x,
                input.foot_left_location_armature.y,
                input.foot_left_location_armature.z,
                input.foot_right_location_armature.x,
                input.foot_right_location_armature.y,
                input.foot_right_location_armature.z,
            );
        }
        if foot_targets_ready && !state.coverage.control_rig_foot_targets {
            info!(
                "[MANUAL-INTEGRATION][CR-ALS-TARGET-TRANSFORM] foot_height_m={:.9} leg_length_m={:.9} -> PASS",
                construction.foot_height_m, construction.leg_length_m,
            );
        }
        state.coverage.control_rig_stages |= stage_ready;
        state.coverage.control_rig_input |= input_ready;
        state.coverage.control_rig_foot_targets |= foot_targets_ready;
    }

    if let Some(mantle) = mantle {
        let actor_world = motion_probe
            .transforms
            .compute_global_transform(character)
            .ok();
        if state.scenario == IntegrationScenario::Base
            && state.phase == IntegrationPhase::Mantle
            && mantle.active
            && mantle.root_motion_ready
        {
            let dilation = ragdoll_probe
                .get_ups
                .get(character)
                .ok()
                .and_then(|(_, _, dilation)| dilation)
                .map_or(1.0, |value| value.0);
            gate_report.direction_trace.push(serde_json::json!({
                "mantle_actor_clock": true, "game_time": state.game_time_seconds,
                "elapsed": mantle.time, "dilation": dilation,
                "mantle_parent_world_valid": actor_world.is_some_and(|world|
                    world.translation().distance(transform.translation) > 0.5
                        && motion_probe.physics.get(character).is_ok_and(|(position, rotation)|
                            world.translation().distance(position.0) < 1e-4
                                && (world.rotation().abs_diff_eq(rotation.0, 1e-5)
                                    || world.rotation().abs_diff_eq(-rotation.0, 1e-5)))),
                "actor_world_position": actor_world.map(|world| world.translation()),
                "actor_local_position": transform.translation
            }));
        }
        if let Some(world) = actor_world {
            observe_mantle(
                state,
                &mut animation_window,
                mantle,
                mantle_montages,
                player,
                &motion_probe.graphs,
                *locomotion,
                notify_action.copied(),
                world.translation(),
            );
        }
    }
    if !state.coverage.ground_prediction_debug
        && interface.debug_stats.ground_prediction_hit_points_drawn > 0
        && let Some(trace) = state
            .avatar
            .and_then(|avatar| interface.ground_debug.get(avatar).ok())
        && let Some((center, point)) = trace.hit
    {
        state.coverage.ground_prediction_debug = true;
        gate_report
            .direction_trace
            .push(serde_json::json!({"ground_prediction_debug_valid":true,
            "start":trace.start,"end":trace.end,"hit_center":center,"impact_point":point,
            "shape_half_extents":trace.shape.shape_scaled().compute_local_aabb().half_extents()}));
    }
    if state.phase == IntegrationPhase::Camera
        && interface.debug_stats.camera_trace_lines_drawn > 0
        && interface.debug_stats.camera_pivot_spheres_drawn == 9
    {
        state.coverage.camera_debug_geometry = true;
    }
    if state.phase == IntegrationPhase::Mantle
        && !state.coverage.mantle_debug
        && let Ok(debug) = interface.motion_debug.get(character)
        && let Some(trace) = debug.0.mantle_probe
        && trace.result == bevy_als::als::mantle::MantleProbeResult::Success
        && interface.debug_stats.cumulative_mantle_probes > 0
        && interface.debug_stats.mantle_probe_lines_drawn >= 2
        && interface.debug_stats.mantle_hit_points_drawn >= 2
    {
        state.coverage.mantle_debug = true;
        gate_report.direction_trace.push(serde_json::json!({"mantle_debug_valid":true,
            "forward_start":trace.forward_start,"forward_end":trace.forward_end,"forward_hit":trace.forward_hit_center,
            "downward_start":trace.downward_start,"downward_end":trace.downward_end,"downward_hit":trace.downward_hit_center,
            "radius":trace.trace_radius,"forward_half_height":trace.forward_half_height,
            "sweeps":interface.debug_stats.mantle_probe_lines_drawn,"hit_points":interface.debug_stats.mantle_hit_points_drawn}));
    }
    if let Ok((_, space, _, _)) = foot_ik_data.get(character) {
        state.coverage.foot_probe |= space.left.has_ground_hit && space.right.has_ground_hit;
    }
    if let Some(lock) = foot_lock {
        state.coverage.foot_lock |= lock.valid;
    }
    if state.scenario == IntegrationScenario::FootIk
        && state.phase == IntegrationPhase::DirectionRegression
        && let Some(bit) = foot_ik_baseline_bit(state.direction_case_index)
        && state.phase_elapsed()
            >= state.direction_case_index as f32 * DIRECTION_CASE_SECONDS
                + DIRECTION_CASE_SETTLE_SECONDS
                + 0.95
        && let Ok((telemetry, space, apply, _)) = foot_ik_data.get(character)
        && telemetry.left.valid
        && telemetry.right.valid
        && space.left.valid
        && space.right.valid
        && apply.left.valid
        && apply.right.valid
        && telemetry.left.target_world.is_finite()
        && telemetry.right.target_world.is_finite()
        && telemetry.left.target_armature_local.is_finite()
        && telemetry.right.target_armature_local.is_finite()
        && telemetry.left.target_parent_local.is_finite()
        && telemetry.right.target_parent_local.is_finite()
        && telemetry.left.reference_chain_error.is_finite()
        && telemetry.right.reference_chain_error.is_finite()
        && telemetry.left.pole_sign.is_finite()
        && telemetry.right.pole_sign.is_finite()
    {
        if state.foot_ik_baseline_mask & bit == 0
            && let Some(case) = direction_regression_case(state.direction_case_index)
        {
            info!(
                "[MANUAL-INTEGRATION][FOOT-IK-BASELINE] case={} left_world=({:.3},{:.3},{:.3}) left_armature=({:.3},{:.3},{:.3}) left_parent=({:.3},{:.3},{:.3}) left_error={:.6} left_pole={:.6} left_weights=({:.3},{:.3}) right_world=({:.3},{:.3},{:.3}) right_armature=({:.3},{:.3},{:.3}) right_parent=({:.3},{:.3},{:.3}) right_error={:.6} right_pole={:.6} right_weights=({:.3},{:.3})",
                case.name,
                telemetry.left.target_world.x,
                telemetry.left.target_world.y,
                telemetry.left.target_world.z,
                telemetry.left.target_armature_local.x,
                telemetry.left.target_armature_local.y,
                telemetry.left.target_armature_local.z,
                telemetry.left.target_parent_local.x,
                telemetry.left.target_parent_local.y,
                telemetry.left.target_parent_local.z,
                telemetry.left.reference_chain_error,
                telemetry.left.pole_sign,
                telemetry.left.ik_weight,
                telemetry.left.pin_weight,
                telemetry.right.target_world.x,
                telemetry.right.target_world.y,
                telemetry.right.target_world.z,
                telemetry.right.target_armature_local.x,
                telemetry.right.target_armature_local.y,
                telemetry.right.target_armature_local.z,
                telemetry.right.target_parent_local.x,
                telemetry.right.target_parent_local.y,
                telemetry.right.target_parent_local.z,
                telemetry.right.reference_chain_error,
                telemetry.right.pole_sign,
                telemetry.right.ik_weight,
                telemetry.right.pin_weight,
            );
            info!(
                "[MANUAL-INTEGRATION][FOOT-IK-SPACE] case={} left_trace=({:.3},{:.3},{:.3}) left_hit=({:.3},{:.3},{:.3}) left_raw={:.6} left_normal_world=({:.3},{:.3},{:.3}) left_normal_armature=({:.3},{:.3},{:.3}) right_trace=({:.3},{:.3},{:.3}) right_hit=({:.3},{:.3},{:.3}) right_raw={:.6} right_normal_world=({:.3},{:.3},{:.3}) right_normal_armature=({:.3},{:.3},{:.3})",
                case.name,
                space.left.trace_target_world.x,
                space.left.trace_target_world.y,
                space.left.trace_target_world.z,
                space.left.hit_world.x,
                space.left.hit_world.y,
                space.left.hit_world.z,
                space.left.raw_offset,
                space.left.normal_world.x,
                space.left.normal_world.y,
                space.left.normal_world.z,
                space.left.normal_armature_local.x,
                space.left.normal_armature_local.y,
                space.left.normal_armature_local.z,
                space.right.trace_target_world.x,
                space.right.trace_target_world.y,
                space.right.trace_target_world.z,
                space.right.hit_world.x,
                space.right.hit_world.y,
                space.right.hit_world.z,
                space.right.raw_offset,
                space.right.normal_world.x,
                space.right.normal_world.y,
                space.right.normal_world.z,
                space.right.normal_armature_local.x,
                space.right.normal_armature_local.y,
                space.right.normal_armature_local.z,
            );
            info!(
                "[MANUAL-INTEGRATION][FOOT-IK-APPLY] case={} left_error={:.6} left_adjust={:.6} left_pole={:.6} left_weight={:.3} right_error={:.6} right_adjust={:.6} right_pole={:.6} right_weight={:.3}",
                case.name,
                apply.left.effector_error,
                space
                    .left
                    .target_world
                    .distance(space.left.resolved_target_world),
                apply.left.pole_sign,
                apply.left.applied_weight,
                apply.right.effector_error,
                space
                    .right
                    .target_world
                    .distance(space.right.resolved_target_world),
                apply.right.pole_sign,
                apply.right.applied_weight,
            );
        }
        state.foot_ik_baseline_mask |= bit;
        state.coverage.foot_ik_space_baseline =
            state.foot_ik_baseline_mask == FOOT_IK_BASELINE_MASK;
    }
    if state.scenario == IntegrationScenario::FootIk
        && state.phase == IntegrationPhase::FootIkNoHit
        && !state.foot_ik_no_hit_logged
        && state.phase_elapsed() >= 0.9
        && let Ok((_, space, _, _)) = foot_ik_data.get(character)
    {
        let left_delta = (space.left.target_armature_local
            - space.left.solver_reference_armature_local)
            .length();
        let right_delta = (space.right.target_armature_local
            - space.right.solver_reference_armature_local)
            .length();
        info!(
            "[MANUAL-INTEGRATION][FOOT-IK-NO-HIT-PROBE] locomotion={:?} left_hit={} right_hit={} left_raw={:.6} right_raw={:.6} left_delta={left_delta:.6} right_delta={right_delta:.6}",
            locomotion,
            space.left.has_ground_hit,
            space.right.has_ground_hit,
            space.left.raw_offset,
            space.right.raw_offset,
        );
        state.foot_ik_no_hit_logged = true;
    }
    if state.scenario == IntegrationScenario::FootIk
        && state.phase == IntegrationPhase::FootIkNoHit
        && state.phase_elapsed() >= 0.8
        && *locomotion == LocomotionState::Air
        && let Ok((_, space, _, _)) = foot_ik_data.get(character)
        && space.left.valid
        && space.right.valid
        && !space.left.has_ground_hit
        && !space.right.has_ground_hit
        && space.left.raw_offset.abs() <= 1e-6
        && space.right.raw_offset.abs() <= 1e-6
    {
        let left_delta = (space.left.target_armature_local
            - space.left.solver_reference_armature_local)
            .length();
        let right_delta = (space.right.target_armature_local
            - space.right.solver_reference_armature_local)
            .length();
        if left_delta <= 0.03 && right_delta <= 0.03 {
            if !state.coverage.foot_ik_no_hit_recovered {
                info!(
                    "[MANUAL-INTEGRATION][FOOT-IK-NO-HIT] left_delta={left_delta:.6} right_delta={right_delta:.6} left_target=({:.3},{:.3},{:.3}) right_target=({:.3},{:.3},{:.3})",
                    space.left.target_world.x,
                    space.left.target_world.y,
                    space.left.target_world.z,
                    space.right.target_world.x,
                    space.right.target_world.y,
                    space.right.target_world.z,
                );
            }
            state.coverage.foot_ik_no_hit_recovered = true;
        }
    }
    if state.scenario == IntegrationScenario::FootIk
        && state.phase == IntegrationPhase::FootIkWeightRamp
        && let Ok((_, _, apply, _)) = foot_ik_data.get(character)
    {
        let elapsed = state.phase_elapsed();
        let sample = if elapsed >= 0.5 && elapsed < 0.75 {
            Some((1 << 0, "off"))
        } else if elapsed >= 1.15 && elapsed < 1.5 {
            Some((1 << 1, "quarter"))
        } else if elapsed >= 1.9 && elapsed < 2.25 {
            Some((1 << 2, "half"))
        } else if elapsed >= 2.65 && elapsed < 3.0 {
            Some((1 << 3, "three_quarters"))
        } else if elapsed >= 3.4 && elapsed < 3.75 {
            Some((1 << 4, "full"))
        } else if elapsed >= 4.35 && elapsed < 4.7 {
            Some((1 << 5, "released"))
        } else {
            None
        };
        if let Some((bit, label)) = sample {
            let weights = (apply.left.applied_weight, apply.right.applied_weight);
            let expected = match bit {
                1 => weights.0 <= 0.05 && weights.1 <= 0.05,
                2 => (0.10..=0.40).contains(&weights.0) && (0.10..=0.40).contains(&weights.1),
                4 => (0.35..=0.65).contains(&weights.0) && (0.35..=0.65).contains(&weights.1),
                8 => (0.60..=0.90).contains(&weights.0) && (0.60..=0.90).contains(&weights.1),
                16 => weights.0 >= 0.95 && weights.1 >= 0.95,
                _ => weights.0 <= 0.05 && weights.1 <= 0.05,
            };
            if expected && state.foot_ik_weight_mask & bit == 0 {
                info!(
                    "[MANUAL-INTEGRATION][FOOT-IK-WEIGHT] phase={label} left={:.3} right={:.3} left_error={:.6} right_error={:.6}",
                    weights.0, weights.1, apply.left.effector_error, apply.right.effector_error,
                );
                state.foot_ik_weight_mask |= bit;
                state.coverage.foot_ik_weight_ramp =
                    state.foot_ik_weight_mask == FOOT_IK_WEIGHT_MASK;
            }
        }
    }
    if state.scenario == IntegrationScenario::FootIk
        && state.phase == IntegrationPhase::FootIkTeleport
        && let Ok((_, spaces, apply, Some(teleport))) = foot_ik_data.get(character)
    {
        if !state.foot_ik_teleport_requested {
            state.foot_ik_teleport_baseline = Some((
                teleport.detected_count,
                teleport.left_rebase_count,
                teleport.right_rebase_count,
            ));
        }
        if state.foot_ik_teleport_requested
            && let Some((detected_before, left_before, right_before)) =
                state.foot_ik_teleport_baseline
            && teleport.detected_count > detected_before
            && teleport.left_rebase_count > left_before
            && teleport.right_rebase_count > right_before
            && spaces.left.valid
            && spaces.right.valid
            && apply.left.valid
            && apply.right.valid
            && !state.coverage.foot_ik_teleport
        {
            info!(
                "[MANUAL-INTEGRATION][FOOT-IK-TELEPORT] delta_m={:.6} detected={} left_rebases={} right_rebases={} left_knee=({:.3},{:.3},{:.3}) right_knee=({:.3},{:.3},{:.3}) left_error={:.6} right_error={:.6}",
                teleport.last_translation_delta_m,
                teleport.detected_count,
                teleport.left_rebase_count,
                teleport.right_rebase_count,
                apply.left.knee_world.x,
                apply.left.knee_world.y,
                apply.left.knee_world.z,
                apply.right.knee_world.x,
                apply.right.knee_world.y,
                apply.right.knee_world.z,
                apply.left.effector_error,
                apply.right.effector_error,
            );
            state.coverage.foot_ik_teleport = true;
        }
    }
    if state.scenario == IntegrationScenario::FootIk
        && let Ok((_, spaces, apply, _)) = foot_ik_data.get(character)
        && spaces.left.valid
        && spaces.right.valid
        && apply.left.valid
        && apply.right.valid
    {
        if state.phase == IntegrationPhase::FootIkTerrain {
            let stable = bevy_als::extensions::root_graph_boundaries::current_control_rig(
                player,
                &motion_probe.graphs,
            )
            .is_some_and(|(_, _, rig)| {
                foot_ik_matches_source(
                    apply,
                    ["FootLeftIk", "FootRightIk"].map(|name| {
                        rig.context
                            .curves
                            .get(&bevy_als::extensions::SourceCurveName::from(name))
                            .copied()
                            .unwrap_or(0.0)
                    }),
                )
            });
            if stable
                && (spaces.left.has_ground_hit || spaces.right.has_ground_hit)
                && (spaces.left.normal_world.y < 0.995 || spaces.right.normal_world.y < 0.995)
                && !state.coverage.foot_ik_slope
            {
                log_foot_ik_stage("slope", spaces, apply);
                state.coverage.foot_ik_slope = true;
            }
            if stable
                && state.phase_elapsed() >= 3.2
                && ((spaces.left.has_ground_hit
                    && spaces.left.hit_world.y > fixture.locomotion_start.y - 0.82)
                    || (spaces.right.has_ground_hit
                        && spaces.right.hit_world.y > fixture.locomotion_start.y - 0.82))
                && !state.coverage.foot_ik_step
            {
                log_foot_ik_stage("step", spaces, apply);
                state.coverage.foot_ik_step = true;
            }
            if stable && *stance == Stance::Crouching && !state.coverage.foot_ik_crouching {
                log_foot_ik_stage("crouching", spaces, apply);
                state.coverage.foot_ik_crouching = true;
            }
            if *locomotion == LocomotionState::Air {
                state.foot_ik_terrain_air_seen = true;
                if stable && !state.coverage.foot_ik_jump_air {
                    log_foot_ik_stage("jump_air", spaces, apply);
                    state.coverage.foot_ik_jump_air = true;
                }
            }
            if state.foot_ik_terrain_air_seen
                && *locomotion == LocomotionState::Grounded
                && stable
                && !state.coverage.foot_ik_landed
            {
                log_foot_ik_stage("landed", spaces, apply);
                state.coverage.foot_ik_landed = true;
            }
            if roll.is_some_and(|roll| roll.active) && stable && !state.coverage.foot_ik_roll {
                log_foot_ik_stage("roll", spaces, apply);
                state.coverage.foot_ik_roll = true;
            }
        }
        if state.phase == IntegrationPhase::Mantle
            && mantle.is_some_and(|mantle| mantle.active)
            && let Some((_, _, rig)) = bevy_als::extensions::root_graph_boundaries::current_control_rig(
                player,
                &motion_probe.graphs,
            )
            && mantle_foot_ik_matches_source(
                apply,
                ["FootLeftIk", "FootRightIk"].map(|name| {
                    rig.context
                        .curves
                        .get(&bevy_als::extensions::SourceCurveName::from(name))
                        .copied()
                        .unwrap_or(0.0)
                }),
                animation_window.observed_animation_weight,
            )
            && !state.coverage.foot_ik_mantle
        {
            log_foot_ik_stage("mantle", spaces, apply);
            state.coverage.foot_ik_mantle = true;
        }
        if state.phase == IntegrationPhase::Roll
            && roll.is_some_and(|roll| roll.active)
            && apply.left.applied_weight >= 0.99
            && apply.right.applied_weight >= 0.99
            && apply.left.effector_error <= 0.02
            && apply.right.effector_error <= 0.02
            && !state.coverage.foot_ik_roll
        {
            log_foot_ik_stage("roll", spaces, apply);
            state.coverage.foot_ik_roll = true;
        }
    }
    if let Some(base) = movement_base {
        state.coverage.movement_base |= base.entity == Some(fixture.platform)
            && base.support_velocity.length() > 0.25
            && base.relative_velocity.length() < 0.75;
    }
    if let Some(roll) = roll {
        state.coverage.roll_started |= roll.active;
        if state.scenario == IntegrationScenario::Grounded
            && state.phase == IntegrationPhase::Roll
            && roll.active
        {
            let dilation = ragdoll_probe
                .get_ups
                .get(character)
                .ok()
                .and_then(|(_, _, dilation)| dilation)
                .map_or(1.0, |value| value.0);
            gate_report.direction_trace.push(serde_json::json!({
                "roll_actor_clock": true, "game_time": state.game_time_seconds,
                "elapsed": roll.time, "dilation": dilation
            }));
        }
    }
    if let Some(slot) = roll_slot {
        state.coverage.roll_slot |= slot.active
            && slot.pose_ready
            && slot.weight > 0.2
            && slot.root_motion_delta.length() > 1e-5;
    }
    if state.phase == IntegrationPhase::Roll {
        state.grounded_roll_exit_diagnostic = grounded_roll_exit_diagnostic(
            *stance,
            desired_stance.copied().unwrap_or(AlsDesiredStance(*stance)),
            roll,
            grounded_graph,
            transition_playback,
            root_transition_slot,
        );
        gate_report.direction_trace.push(serde_json::json!({
            "elapsed": state.phase_elapsed(), "phase": "Roll",
            "diagnostic": state.grounded_roll_exit_diagnostic,
            "notify": motion_probe.transition_notify.get(character).ok().flatten().map(|value| format!("{value:?}")),
        }));
    }
    if grounded_roll_exit_transition_matches_source(
        state.phase,
        motion_probe
            .transition_notify
            .get(character)
            .ok()
            .flatten()
            .and_then(|value| value.roll_to_grounded_stance),
        roll,
        transition_playback,
        root_transition_slot,
        grounded_graph,
    ) && !state.coverage.grounded_roll_exit_transition
    {
        let playback = transition_playback.unwrap();
        info!(
            "[MANUAL-INTEGRATION][GROUNDED-ROLL-EXIT] requested_stance={:?} kind={:?} blend_in={:.3} blend_out={:.3} play_rate={:.3} start_time={:.3} slot_weight={:.3} -> PASS",
            motion_probe
                .transition_notify
                .get(character)
                .ok()
                .flatten()
                .and_then(|value| value.roll_to_grounded_stance),
            playback.kind,
            playback.blend_in,
            playback.blend_out,
            playback.play_rate,
            playback.start_time,
            root_transition_slot.unwrap().weight,
        );
        state.coverage.grounded_roll_exit_transition = true;
    }
    if let Some(spawned) = player.get_spawned_ragdoll() {
        let profile = ragdoll_probe.profiles.get(instance.player_entity()).ok();
        if let Some((applied, source_profile)) = profile {
            state.ragdoll_current_joint_anchor_error_m = applied.current_joint_anchor_error_m;
            state.ragdoll_current_joint_anchor_label = applied
                .current_joint_anchor_profile_index
                .and_then(|index| source_profile.joints.get(index as usize))
                .map(|joint| joint.label.clone());
            state.ragdoll_max_joint_anchor_error_m = applied.max_joint_anchor_error_m;
            state.ragdoll_max_joint_anchor_label = applied
                .max_joint_anchor_profile_index
                .and_then(|index| source_profile.joints.get(index as usize))
                .map(|joint| joint.label.clone());
            state.ragdoll_handoff_locked_joint_anchor_error_m =
                applied.handoff_locked_joint_anchor_error_m;
            state.ragdoll_handoff_locked_joint_anchor_label = applied
                .handoff_locked_joint_anchor_profile_index
                .and_then(|index| source_profile.joints.get(index as usize))
                .map(|joint| joint.label.clone());
            state.ragdoll_handoff_pose_error_m = applied.handoff_animated_body_position_error_m;
            state.ragdoll_handoff_rotation_error_radians =
                applied.handoff_animated_body_rotation_error_radians;
        }
        let profile_valid = profile.is_some_and(|(applied, source)| {
            applied.spawned_root == Some(spawned.root)
                && applied.body_count == 21
                && applied.collider_count == 22
                && applied.joint_count == 20
                && applied.source_joint_count == 19
                && applied.collision_disabled_pair_count == 49
                && applied.handoff_animated_body_count == applied.body_count
                && applied.handoff_animated_body_position_error_m <= 1e-4
                && applied.handoff_animated_body_rotation_error_radians <= 1e-4
                && spawned.colliders.values().all(|collider| {
                    ragdoll_probe
                        .collision_members
                        .get(*collider)
                        .is_ok_and(|member| {
                            let expected = source
                                .bodies
                                .iter()
                                .find(|body| body.source_index == member.body_index)
                                .map(|body| body.source_layers);
                            expected.is_some_and(|expected| {
                                member.source_layers == expected
                                    && member.group == spawned.root
                                    && member.disabled_pairs.len() == 49
                                    && ragdoll_probe.collision_layers.get(*collider).is_ok_and(
                                        |layers| {
                                            *layers
                                                == if notify_action
                                                    == Some(&AlsNotifyLocomotionAction::Ragdolling)
                                                {
                                                    expected
                                                } else {
                                                    CollisionLayers::NONE
                                                }
                                        },
                                    )
                            })
                        })
                })
        });
        state.coverage.ragdoll_player_replaced |= profile_valid
            && ragdoll_probe
                .replacements
                .get(instance.player_entity())
                .is_ok_and(|replacement| {
                    replacement.old_root != spawned.root
                        && !ragdoll_probe.alive.contains(replacement.old_root)
                });
        let mut dynamic = 0usize;
        let mut root_kinematic = false;
        for body in spawned.bodies.values() {
            if ragdoll_probe
                .labels
                .get(*body)
                .is_ok_and(|label| label.0 == "root")
            {
                root_kinematic = ragdoll_probe
                    .body_modes
                    .get(*body)
                    .is_ok_and(|mode| *mode == RigidBody::Kinematic);
            } else if ragdoll_probe
                .body_modes
                .get(*body)
                .is_ok_and(|mode| *mode == RigidBody::Dynamic)
            {
                dynamic += 1;
            }
        }
        state.coverage.ragdoll_pa_profile |= profile_valid
            && (notify_action != Some(&AlsNotifyLocomotionAction::Ragdolling)
                || (dynamic == 20 && root_kinematic));
    }
    if notify_action == Some(&AlsNotifyLocomotionAction::Ragdolling) {
        state.coverage.ragdoll_entered = true;
        state.ragdoll_seen = true;
        state.ragdoll_frames = state.ragdoll_frames.saturating_add(1);
        if let Ok((sockets, actor, collider)) = ragdoll_probe.camera_sockets.get(character) {
            use avian3d::prelude::SimpleCollider;
            let bounds = collider.aabb(actor.translation(), actor.rotation(), 0.0);
            let bottom = actor.translation() - Vec3::Y * bounds.size().y * 0.5;
            state.coverage.ragdoll_camera_pivot |= sockets.valid
                && sockets.root.distance(bottom) < 1e-4
                && cameras.iter().any(|(_, camera)| {
                    camera.pivot_target.distance((bottom + sockets.head) * 0.5) < 1e-4
                });
        }
    }
    if let Some(follow) = ragdoll_follow {
        if state.ragdoll_seen && follow.grounded {
            state.coverage.ragdoll_follow = true;
            if state.scenario == IntegrationScenario::Ragdoll {
                let expected_y = fixture.locomotion_start.y - 0.92
                    + 2.0 * ALS_CHARACTER_CAPSULE_HALF_HEIGHT
                    + ALS_CHARACTER_FLOOR_CLEARANCE;
                let (colliders_scaled, joint_frames_preserved) = player
                    .get_spawned_ragdoll()
                    .map_or((false, false), |spawned| {
                        let colliders_scaled = ragdoll_probe
                            .profiles
                            .get(instance.player_entity())
                            .is_ok_and(|(_, profile)| {
                                spawned.colliders.values().all(|entity| {
                                    ragdoll_probe.colliders.get(*entity).is_ok_and(
                                        |(collider, label)| {
                                            profile
                                                .colliders
                                                .iter()
                                                .find(|source| source.label == label.0)
                                                .is_some_and(|source| {
                                                    let base = source
                                                        .shape
                                                        .avian_collider()
                                                        .shape()
                                                        .compute_local_aabb()
                                                        .half_extents();
                                                    collider
                                                        .shape_scaled()
                                                        .compute_local_aabb()
                                                        .half_extents()
                                                        .abs_diff_eq(base * 2.0, 1e-5)
                                                })
                                        },
                                    )
                                })
                            });
                        let joint_frames_preserved = ragdoll_probe
                            .profiles
                            .get(instance.player_entity())
                            .ok()
                            .and_then(|(_, profile)| {
                                profile.joints.iter().find(|joint| joint.label == "foot_l")
                            })
                            .is_some_and(|source| {
                                spawned.joints.values().any(|entity| {
                                    ragdoll_probe
                                        .joints
                                        .get(*entity)
                                        .is_ok_and(|(joint, label)| {
                                            label.0 == source.label
                                                && joint.parent_frame.translation.abs_diff_eq(
                                                    source.definition.parent_frame.translation,
                                                    1e-4,
                                                )
                                                && joint.child_frame.translation.abs_diff_eq(
                                                    source.definition.child_frame.translation,
                                                    1e-4,
                                                )
                                        })
                                })
                            });
                        (colliders_scaled, joint_frames_preserved)
                    });
                let scaled = transform.scale == Vec3::splat(2.0)
                    && (follow.target.y - expected_y).abs() < 0.12
                    && (transform.translation.y - follow.target.y).abs() < 0.02
                    && colliders_scaled
                    && joint_frames_preserved;
                state.coverage.ragdoll_scaled |= scaled;
                if state.ragdoll_grounded_at.is_none() {
                    info!(
                        "[MANUAL-INTEGRATION][RAGDOLL-SCALED] target_y={:.3} expected_y={expected_y:.3} actor_y={:.3} scale={:?} colliders_scaled={colliders_scaled} joint_frames_preserved={joint_frames_preserved} matched={scaled}",
                        follow.target.y, transform.translation.y, transform.scale,
                    );
                }
            }
            if state.ragdoll_grounded_at.is_none() {
                state.ragdoll_grounded_at = Some(state.game_time_seconds);
                info!("[MANUAL-INTEGRATION] ragdoll ground follow stabilized");
            }
        } else if state.ragdoll_seen {
            state.ragdoll_grounded_at = None;
        }
    }
    if let Some(drive) = joint_drive {
        state.coverage.ragdoll_joint_drive |= state.ragdoll_seen
            && drive.target_joint_count == 19
            && (drive.angular_stiffness - drive.speed_amount * RAGDOLL_ANGULAR_STIFFNESS).abs()
                <= 1e-3
            && drive.angular_damping.abs() <= 1e-6
            && drive.angular_force_limit.abs() <= 1e-6;
    }
    let ragdoll_recovered = state.ragdoll_seen
        && state.ragdoll_exit_requested
        && notify_action != Some(&AlsNotifyLocomotionAction::Ragdolling)
        && *locomotion == LocomotionState::Grounded
        && velocity.0.length() < 0.5;
    if ragdoll_recovered {
        state.coverage.ragdoll_recovered = true;
        if state.ragdoll_recovered_at.is_none() {
            state.ragdoll_recovered_at = Some(state.game_time_seconds);
            info!(
                "[MANUAL-INTEGRATION] ragdoll recovered position=({:.3},{:.3},{:.3}) velocity={:.3}",
                transform.translation.x,
                transform.translation.y,
                transform.translation.z,
                velocity.0.length()
            );
        }
    }
    if state.phase == IntegrationPhase::Ragdoll {
        if let Ok((Some(get_up), blocked, custom_dilation)) = ragdoll_probe.get_ups.get(character) {
            if state.scenario == IntegrationScenario::GetUp && state.get_up_case == 0 {
                gate_report.direction_trace.push(serde_json::json!({
                    "get_up_actor_clock": true, "game_time": state.game_time_seconds,
                    "elapsed": get_up.elapsed, "dilation": custom_dilation.map_or(1.0, |value| value.0)
                }));
            }
            let expected_facing_up = if state.scenario == IntegrationScenario::GetUp {
                state.get_up_case == 1
            } else {
                get_up.facing_up
            };
            let facing_matches = get_up.facing_up == expected_facing_up;
            let active = facing_matches
                && get_up.pose_ready
                && graph_global_f32(player, "post_locomotion.weight")
                    .is_some_and(|weight| weight > 0.0);
            if active {
                state.coverage.get_up_airborne |= notify_action
                    == Some(&AlsNotifyLocomotionAction::GettingUp)
                    && *locomotion != LocomotionState::Grounded;
                state.coverage.get_up_rig_space_mismatch |=
                    bevy_als::extensions::root_graph_boundaries::current_control_rig(
                        player,
                        &motion_probe.graphs,
                    )
                    .zip(
                        motion_probe
                            .transforms
                            .compute_global_transform(instance.player_entity())
                            .ok(),
                    )
                    .is_none_or(|((_, _, rig), mesh)| {
                        !rig.to_world
                            .translation
                            .as_vec3()
                            .abs_diff_eq(mesh.translation(), 1e-5)
                    });
                state.coverage.get_up_front |= !get_up.facing_up;
                state.coverage.get_up_back |= get_up.facing_up;
                state.coverage.get_up_input_locked[usize::from(get_up.facing_up)] |=
                    blocked && notify_action == Some(&AlsNotifyLocomotionAction::GettingUp);
                state.coverage.get_up_input_released[usize::from(get_up.facing_up)] |=
                    !blocked && notify_action == Some(&AlsNotifyLocomotionAction::None);
            }
            if state.scenario == IntegrationScenario::GetUp
                && state.get_up_jump_requested_at.is_some()
            {
                state.coverage.get_up_native_jump_rejected_during_action |= active
                    && blocked
                    && notify_action == Some(&AlsNotifyLocomotionAction::GettingUp)
                    && *locomotion == LocomotionState::Grounded
                    && velocity.y.abs() < 0.02;
            }
            let variant = if get_up.facing_up {
                "GetUpBack"
            } else {
                "GetUpFront"
            };
            **animation_window = ManualAnimationWindowEvidence {
                label: Some(if get_up.facing_up {
                    "get_up_back"
                } else {
                    "get_up_front"
                }),
                index: Some(usize::from(get_up.facing_up)),
                settled: active && (0.4..=0.8).contains(&get_up.clip_time),
                capture_source_window: true,
                requires_pre_rig_motion: true,
                requires_final_pose_motion: true,
                requires_non_reference_root_pose: true,
                source_node: Some("AB_Als/Slot 'PostLocomotion'"),
                source_state: Some("GettingUp"),
                expected_animation_variant: Some(if expected_facing_up {
                    "GetUpBack"
                } else {
                    "GetUpFront"
                }),
                observed_animation_variant: Some(variant),
                observed_animation_clip_time: Some(get_up.clip_time),
                observed_animation_weight: Some(get_up.weight),
                observed_animation_play_rate: Some(
                    bevy_als::get_up::get_up_settings(get_up.facing_up).play_rate,
                ),
                direction_signature_group: (state.scenario == IntegrationScenario::GetUp)
                    .then_some("PostLocomotion GetUp"),
                required_direction_signature_members: 2,
                minimum_direction_signature_rotation_radians: 0.25,
                expected_direction: Some(if expected_facing_up { "Back" } else { "Front" }),
                observed_direction: Some(if get_up.facing_up { "Back" } else { "Front" }),
                canonical_clip_time: Some(0.6),
                maximum_clip_time_distance: 0.1,
                minimum_observation_seconds: 0.1,
                ..default()
            };
        }
    }

    if state.scenario == IntegrationScenario::Interface {
        let parent_instance = player
            .get_context_arena()
            .and_then(|arena| {
                arena
                    .get_toplevel()
                    .node_states
                    .get::<bevy_als::extensions::source_instance::SourceInstanceProperties>(
                        bevy_animation_graph::core::animation_graph::NodeId::default(),
                        StateKey::Default,
                    )
                    .ok()
            })
            .map(|source| source.instance_id);
        if let Some(instance) = parent_instance {
            state.interface_parent_preserved &= state
                .interface_parent_instance
                .is_none_or(|previous| previous == instance);
            state.interface_parent_instance = Some(instance);
        }
        let hud_nodes_hidden = !interface.hud_debug_nodes.is_empty()
            && !interface.hud_input_nodes.is_empty()
            && interface
                .hud_debug_nodes
                .iter()
                .all(|node| node.display == Display::None)
            && interface
                .hud_input_nodes
                .iter()
                .all(|node| node.display == Display::None);
        let hud_nodes_visible = !interface.hud_debug_nodes.is_empty()
            && !interface.hud_input_nodes.is_empty()
            && interface
                .hud_debug_nodes
                .iter()
                .all(|node| node.display != Display::None)
            && interface
                .hud_input_nodes
                .iter()
                .all(|node| node.display != Display::None);
        state.coverage.hud_layout |= !interface.hud_roots.is_empty()
            && !interface.hud_debug_nodes.is_empty()
            && !interface.hud_input_nodes.is_empty();
        state.interface_hud_hidden_seen |= !interface.hud_state.ui_visible && hud_nodes_hidden;
        state.interface_hud_restored_seen |=
            state.interface_hud_hidden_seen && interface.hud_state.ui_visible && hud_nodes_visible;
        state.coverage.hud_toggle =
            state.interface_hud_hidden_seen && state.interface_hud_restored_seen;

        state.interface_slomo_seen |= interface.hud_state.slomo_active
            && (interface.virtual_time.relative_speed() - 0.15).abs() < 1e-4;
        state.interface_slomo_restored_seen |= state.interface_slomo_seen
            && !interface.hud_state.slomo_active
            && (interface.virtual_time.relative_speed() - 1.0).abs() < 1e-4;
        state.coverage.hud_slomo =
            state.interface_slomo_seen && state.interface_slomo_restored_seen;

        state.interface_menu_open_seen |= interface.menu_state.open
            && interface
                .menu_state
                .root
                .and_then(|root| interface.menu_nodes.get(root).ok())
                .is_some_and(|node| node.display == Display::Flex)
            && (interface.virtual_time.relative_speed() - 0.35).abs() < 1e-4;
        state.interface_menu_closed_seen |= state.interface_menu_open_seen
            && !interface.menu_state.open
            && interface
                .menu_state
                .root
                .and_then(|root| interface.menu_nodes.get(root).ok())
                .is_some_and(|node| node.display == Display::None)
            && (interface.virtual_time.relative_speed() - 1.0).abs() < 1e-4;
        state.coverage.overlay_menu =
            state.interface_menu_open_seen && state.interface_menu_closed_seen;

        let bow_bound = *overlay == OverlayState::Bow
            && overlay_tag.as_ref() == Some(&OverlayState::Bow.tag())
            && active_layer_pose(
                player,
                &motion_probe,
                "ALS/Character/AnimationInstances/Overlays/AB_Als_Bow/AnimGraph.animgraph.ron",
            );
        let default_bound = *overlay == OverlayState::Default
            && overlay_tag.as_ref() == Some(&OverlayState::Default.tag())
            && active_layer_pose(
                player,
                &motion_probe,
                "ALS/Character/AnimationInstances/Overlays/AB_Als_Default/AnimGraph.animgraph.ron",
            );
        if bow_bound && !state.interface_overlay_bow_seen {
            info!(
                "[MANUAL-INTEGRATION][ALI] selector=8 state=Bow generated_pose=true binding=PASS"
            );
        }
        if state.interface_overlay_bow_seen
            && default_bound
            && !state.interface_overlay_default_after_bow_seen
        {
            info!(
                "[MANUAL-INTEGRATION][ALI] selector=0 state=Default generated_pose=true fallback=PASS"
            );
        }
        if state.interface_overlay_default_after_bow_seen
            && bow_bound
            && !state.interface_overlay_reentry_seen
        {
            info!("[MANUAL-INTEGRATION][ALI] selector=8 state=Bow reentry=PASS");
        }
        state.interface_overlay_bow_seen |= bow_bound;
        state.interface_overlay_default_after_bow_seen |=
            state.interface_overlay_bow_seen && default_bound;
        state.interface_overlay_reentry_seen |=
            state.interface_overlay_default_after_bow_seen && bow_bound;
        state.coverage.overlay_interface_binding = state.interface_overlay_bow_seen;
        state.coverage.overlay_interface_fallback = state.interface_overlay_default_after_bow_seen;
        state.coverage.overlay_interface_reentry = state.interface_overlay_reentry_seen;
        gate_report.direction_trace.push(serde_json::json!({"time":state.game_time_seconds, "bow_bound":bow_bound, "default_bound":default_bound, "parent_instance":parent_instance, "parent_preserved":state.interface_parent_preserved}));

        let all_debug_categories_active = DebugCategory::ALL
            .into_iter()
            .all(|category| interface.show_debug.is_active(category));
        state.coverage.show_debug |= state.interface_debug_armed
            && all_debug_categories_active
            && !interface.show_debug_roots.is_empty();
        state.coverage.debug_draw |= state.coverage.show_debug && interface.debug_stats.total() > 0;
        if state.phase_elapsed() >= 5.0 {
            hud_layout::observe_debug_keys(
                state.phase_elapsed(),
                &interface.show_debug,
                &interface.show_debug_roots,
                &interface.debug_stats,
                &mut gate_report.direction_trace,
            );
        }
        if state.interface_debug_armed
            && all_debug_categories_active
            && !interface.show_debug_roots.is_empty()
        {
            let expected_context = state
                .avatar
                .and_then(|avatar| motion_probe.object_scenes.get(avatar).ok())
                .and_then(|instance| players.get(instance.player_entity()).ok())
                .and_then(|(_, context)| context);
            let metadata: serde_json::Value = serde_json::from_str(include_str!(
                "../../../assets/ALS/Character/als_character_curves.runtime.json"
            ))
            .unwrap();
            let mut expected: Vec<(String, String)> = metadata["curves"]
                .as_array()
                .unwrap()
                .iter()
                .map(|name| {
                    let name = name.as_str().unwrap();
                    (
                        bevy_als::show_debug_hud::name_to_display_string(name, false),
                        expected_context
                            .map(|context| {
                                let value = context.f32(name).unwrap_or(0.0);
                                let source = if value != 0.0
                                    && ["RotationYawOffset", "RotationYawSpeed"].contains(&name)
                                {
                                    -value
                                } else {
                                    value
                                };
                                format!("{source:.2}")
                            })
                            .unwrap_or_else(|| "N/A".into()),
                    )
                })
                .collect();
            let actual_rotation = motion_probe.rotation_mode.get(character).copied().unwrap();
            let desired_rotation = interface
                .desired_rotation_modes
                .get(character)
                .map_or(actual_rotation, |value| value.0);
            let desired_gait = motion_probe
                .gait
                .get(character)
                .map_or(*gait, |(value, _)| value.0);
            let aiming = motion_probe
                .inputs
                .get(character)
                .ok()
                .and_then(|(input, override_input)| override_input.map(|value| &value.0).or(input))
                .is_some_and(|input| input.aim_held);
            expected.push((
                "View Mode".into(),
                if first_person {
                    "First Person"
                } else {
                    "Third Person"
                }
                .into(),
            ));
            expected.push((
                "Desired Aiming".into(),
                if aiming { "True" } else { "False" }.into(),
            ));
            for (label, tag) in [
                ("Locomotion Mode", locomotion.as_str()),
                ("Desired Rotation Mode", desired_rotation.as_str()),
                ("Rotation Mode", actual_rotation.as_str()),
                (
                    "Desired Stance",
                    desired_stance.map_or(*stance, |value| value.0).as_str(),
                ),
                ("Stance", stance.as_str()),
                ("Desired Gait", desired_gait.as_str()),
                ("Gait", gait.as_str()),
                ("Overlay Mode", overlay.as_str()),
                (
                    "Locomotion Action",
                    notify_action
                        .copied()
                        .unwrap_or(AlsNotifyLocomotionAction::None)
                        .as_str(),
                ),
            ] {
                expected.push((
                    label.into(),
                    if tag.is_empty() {
                        "None".into()
                    } else {
                        bevy_als::show_debug_hud::name_to_display_string(
                            tag.rsplit('.').next().unwrap(),
                            false,
                        )
                    },
                ));
            }
            let source_yaw = |yaw: f32| {
                let positive = (270.0 - yaw).rem_euclid(360.0);
                if positive > 180.0 {
                    positive - 360.0
                } else {
                    positive
                }
            };
            expected.push((
                "Rotation".into(),
                format!(
                    "R: 0.00P: {:.2}Y: {:.2}",
                    view_pitch.0.to_degrees(),
                    source_yaw(view_yaw.0.to_degrees())
                ),
            ));
            let expected_motion = motion_probe.actual.get(character).ok();
            if let Some(motion) = expected_motion {
                expected.push((
                    "Input Yaw Angle".into(),
                    format!("{:.2}", source_yaw(motion.input_yaw_degrees)),
                ));
                expected.push((
                    "Velocity Yaw Angle".into(),
                    format!("{:.2}", source_yaw(motion.velocity_yaw_degrees)),
                ));
                expected.push((
                    "Speed".into(),
                    format!("{:.2}", motion.horizontal_speed * 100.0),
                ));
                let direction = if motion.world_velocity.dot(motion.world_velocity) < 1e-12 {
                    Vec3::ZERO
                } else {
                    motion.world_velocity.normalize_or_zero()
                };
                expected.push((
                    "Velocity Direction".into(),
                    format!(
                        "X: {:.2}Y: {:.2}Z: {:.2}",
                        direction.x, direction.z, direction.y
                    ),
                ));
            }
            if let Ok(response) = interface.rotation_responses.get(character) {
                expected.push((
                    "Smooth Target Yaw Angle".into(),
                    format!("{:.2}", source_yaw(response.smooth_target_yaw.to_degrees())),
                ));
            }
            let camera_source = interface
                .camera_assemblies
                .get(character)
                .ok()
                .and_then(|assembly| assembly.camera)
                .and_then(|view| interface.camera_sources.get(view).ok())
                .map(|source| source.0);
            let camera = camera_source
                .and_then(|source| interface.camera_debug.get(source).ok())
                .filter(|info| info.valid);
            let camera_context = camera_source
                .and_then(|source| motion_probe.object_scenes.get(source).ok())
                .and_then(|instance| players.get(instance.player_entity()).ok())
                .and_then(|(_, context)| context);
            if let Some(context) = camera_context {
                let mapping: std::collections::HashMap<
                    bevy_als::extensions::SourceCurveName,
                    (String, f32),
                > = serde_json::from_str(include_str!(
                    "../../../assets/ALSCamera/als_camera_curves.runtime.json"
                ))
                .unwrap();
                for (name, value) in &context.curves {
                    let (name, value) = mapping
                        .get(name)
                        .map_or((name.as_str(), *value), |(source, scale)| {
                            (source.as_str(), value * scale)
                        });
                    expected.push((
                        bevy_als::show_debug_hud::name_to_display_string(name, false),
                        format!("{:.2}", if value == 0.0 { 0.0 } else { value }),
                    ));
                }
            }
            if let Some(info) = camera {
                for (name, point) in [
                    ("Pivot Target Location", Some(info.pivot_target)),
                    ("Pivot Lag Location", info.pivot_lag),
                    ("Pivot Location", info.pivot),
                ] {
                    expected.push((
                        name.into(),
                        point
                            .map(|point| {
                                format!(
                                    "X: {:.2}Y: {:.2}Z: {:.2}",
                                    point.x * 100.0,
                                    point.z * 100.0,
                                    point.y * 100.0
                                )
                            })
                            .unwrap_or_else(|| "N/A".into()),
                    ));
                }
                expected.push((
                    "Camera Field Of View".into(),
                    format!("{:.2}", info.field_of_view_deg),
                ));
                expected.push(("Right Shoulder".into(), info.right_shoulder.to_string()));
            }
            let mismatches: Vec<_> = expected
                .iter()
                .filter(|(name, value)| {
                    !interface
                        .debug_values
                        .iter()
                        .any(|(key, text)| key.as_str() == name.as_str() && &text.0 == value)
                })
                .cloned()
                .collect();
            gate_report.direction_trace.push(serde_json::json!({"hud_data_valid":expected_context.is_some() && expected_motion.is_some() && camera.is_some() && camera_context.is_some() && mismatches.is_empty(),
                "hud_values_checked":expected.len(),"hud_data_mismatches":mismatches}));
        }
    }

    if state.phase == IntegrationPhase::Camera {
        let view = motion_probe
            .rendered_views
            .iter()
            .find_map(|(camera, transform, owner)| {
                (camera.is_active && owner.character == character)
                    .then(|| transform.rotation.to_euler(EulerRot::YXZ).0)
            });
        let input = motion_probe
            .inputs
            .get(character)
            .ok()
            .and_then(|(input, override_input)| override_input.map(|value| &value.0).or(input))
            .map_or(Vec2::ZERO, |input| input.move_dir);
        let snapshot = motion_probe
            .player_views
            .get(character)
            .ok()
            .map(|view| view.0);
        let desired = motion_probe
            .controllers
            .get(character)
            .ok()
            .map(|controller| controller.basis.desired_motion);
        state
            .camera_movement
            .observe(view, view_yaw.0, input, snapshot, desired);
        gate_report.direction_trace.push(serde_json::json!({"player_view_yaw":snapshot,"rendered_yaw":view,"control_yaw":view_yaw.0,
            "input":input.to_array(),"desired":desired.map(|value| value.to_array()),"movement_samples":state.camera_movement.samples,
            "movement_lagged":state.camera_movement.lagged,"movement_invalid":state.camera_movement.invalid}));
    }
    if let Some(camera) = state.camera
        && let Ok((_camera_transform, debug)) = cameras.get(camera)
    {
        let camera_debug = debug;
        if state.phase == IntegrationPhase::Camera
            || (state.phase == IntegrationPhase::Ragdoll
                && state.scenario == IntegrationScenario::Ragdoll)
        {
            gate_report.camera_trace.push(ManualCameraTraceSample {
                phase_elapsed_seconds: state.phase_elapsed() as f64,
                obstacles_present: state.camera_obstacles.iter().any(Option::is_some),
                obstacles_ignored: state.camera_obstacle_ignored,
                blocked_ratio: state.camera_blocked_ratio,
                first_person_requested: state.camera_view_mode_requested,
                actual_first_person: first_person,
                ragdoll_active: notify_action == Some(&AlsNotifyLocomotionAction::Ragdolling),
                camera: *camera_debug,
            });
        }
        state.coverage.camera_rig |= camera_debug.valid;
        if state.phase == IntegrationPhase::Camera
            && state.camera_scenario_armed
            && state.camera_obstacles.iter().any(Option::is_some)
            && camera_debug.trace_blocking_hit
            && camera_debug.trace_distance_ratio < 0.98
            && camera_debug.trace_adjustment_count >= 2
            && camera_debug.trace_adjustment_resolved
        {
            if !state.coverage.camera_collision {
                info!(
                    "[MANUAL-INTEGRATION][CAMERA][MTD] blocking={} adjustment_count={} resolved={} ratio={:.3}",
                    camera_debug.trace_blocking_hit,
                    camera_debug.trace_adjustment_count,
                    camera_debug.trace_adjustment_resolved,
                    camera_debug.trace_distance_ratio
                );
            }
            state.coverage.camera_collision = true;
            state.camera_blocked_ratio = Some(camera_debug.trace_distance_ratio);
        }
        if state.phase == IntegrationPhase::Camera
            && state.camera_scenario_armed
            && state.camera_obstacles.iter().any(Option::is_some)
        {
            let elapsed = state.phase_elapsed();
            if elapsed - state.camera_last_probe_secs >= 0.5 {
                state.camera_last_probe_secs = elapsed;
                info!(
                    "[MANUAL-INTEGRATION][CAMERA] blocking={} adjusted={} adjustment_count={} resolved={} ratio={:.3} first_person={:.3}",
                    camera_debug.trace_blocking_hit,
                    camera_debug.trace_adjusted_start,
                    camera_debug.trace_adjustment_count,
                    camera_debug.trace_adjustment_resolved,
                    camera_debug.trace_distance_ratio,
                    camera_debug.first_person_override
                );
            }
        }
        if state.camera_obstacle_ignored
            && camera_debug.first_person_override < 1.0 - 1e-5
            && state
                .camera_blocked_ratio
                .is_some_and(|blocked| camera_debug.trace_distance_ratio > blocked + 0.02)
            && !camera_debug.trace_blocking_hit
        {
            state.coverage.camera_recovery = true;
        }
        state.coverage.camera_first_person |=
            first_person && camera_debug.first_person_override > 0.5;
        state.coverage.camera_shoulder |= left_shoulder && !camera_debug.right_shoulder;
        state.coverage.camera_shake |= camera_debug.shake_location_offset.length() > 1e-5
            || camera_debug.shake_rotation_degrees.length() > 1e-5;
        if state.dilation_active && real_time.delta_secs() > 1e-5 {
            let ratio = camera_debug.unscaled_delta_seconds / real_time.delta_secs();
            state.coverage.camera_unscaled_time |= (ratio - 1.0).abs() < 0.25;
        }
    }

    if state.phase == IntegrationPhase::Finish && !state.final_logged {
        state.final_logged = true;
        let grounded_roll_exit_detail = state.grounded_roll_exit_diagnostic.clone();
        let transition_notify_detail = motion_probe
            .transition_notify
            .get(character)
            .ok()
            .flatten()
            .map_or_else(
                || "generated transition telemetry is missing".to_owned(),
                |telemetry| {
                    format!(
                        "stop_quick_count={} observed_named_fsm_state_ids={} active_named_source_state_count={} last_active_named_source_state_id={} active_named_transition_count={} last_active_named_transition_id={}",
                        telemetry.stop_quick_count,
                        telemetry.observed_named_fsm_state_ids.join(","),
                        telemetry.active_named_source_state_count,
                        telemetry.last_active_named_source_state_id,
                        telemetry.active_named_transition_count,
                        telemetry.last_active_named_transition_id
                    )
                },
            );
        let coverage = &state.coverage;
        let direction_regression = coverage.direction_cases.iter().all(|covered| *covered);
        if matches!(
            state.scenario,
            IntegrationScenario::Base | IntegrationScenario::StandingDirection
        ) {
            for index in 0..DIRECTION_CASE_COUNT {
                if !coverage.direction_cases[index]
                    && let Some(case) = direction_regression_case(index)
                {
                    error!(
                        "[MANUAL-INTEGRATION][DIRECTION][NOT_COVERED] case={}",
                        case.name
                    );
                }
            }
        }
        if matches!(
            state.scenario,
            IntegrationScenario::Base | IntegrationScenario::Crouching
        ) {
            for index in 0..CROUCH_DIRECTION_CASE_COUNT {
                if !coverage.crouch_direction_cases[index]
                    && let Some(case) = crouch_direction_regression_case(index)
                {
                    error!(
                        "[MANUAL-INTEGRATION][CROUCH-DIRECTION][NOT_COVERED] case={}",
                        case.name
                    );
                }
            }
        }
        let mut results = match state.scenario {
            IntegrationScenario::LocomotionActions => {
                if state.timed_out {
                    gate_report.direction_trace.push(serde_json::json!({"watchdog": {"exit_reason": "case_progress_timeout", "last_progress": state.locomotion_probe.case_index, "no_progress_limit_seconds": 12.0, "current_conditions": state.locomotion_probe.sample}}));
                }
                let mut results = vec![
                    ("runtime_assembly", coverage.assembly),
                    ("animation_graph", coverage.graph),
                ];
                results.extend(
                    LOCOMOTION_WINDOW_LABELS
                        .iter()
                        .enumerate()
                        .map(|(index, label)| (*label, state.locomotion_probe.covered[index])),
                );
                results.push((
                    "locomotion_lifecycles",
                    state.locomotion_probe.case_index == 6 && state.locomotion_probe.completed,
                ));
                results.push(("locomotion_source_blends", state.locomotion_probe.valid));
                results
            }
            IntegrationScenario::CrouchingActions => {
                let mut results = vec![
                    ("runtime_assembly", coverage.assembly),
                    ("animation_graph", coverage.graph),
                    ("crouching", coverage.crouching),
                ];
                results.extend(
                    CROUCHING_STOP_CASES
                        .iter()
                        .enumerate()
                        .map(|(index, case)| (case.0, coverage.standing_stop[index])),
                );
                results
                    .extend((2..4).map(|index| {
                        (STANDING_TURN_CASES[index].0, coverage.standing_turn[index])
                    }));
                results.extend(
                    STANDING_DYNAMIC_CASES
                        .iter()
                        .enumerate()
                        .map(|(index, case)| (case.0, coverage.standing_dynamic[index])),
                );
                results
            }
            IntegrationScenario::StandingDynamic => {
                let mut results = vec![
                    ("runtime_assembly", coverage.assembly),
                    ("animation_graph", coverage.graph),
                    ("standing_graph", coverage.standing_graph),
                ];
                results.extend(
                    STANDING_DYNAMIC_CASES
                        .iter()
                        .enumerate()
                        .map(|(index, case)| (case.0, coverage.standing_dynamic[index])),
                );
                results
            }
            IntegrationScenario::StandingTurn => {
                let mut results = vec![
                    ("runtime_assembly", coverage.assembly),
                    ("animation_graph", coverage.graph),
                    ("standing_graph", coverage.standing_graph),
                ];
                results.extend(
                    STANDING_TURN_CASES
                        .iter()
                        .enumerate()
                        .map(|(index, case)| (case.0, coverage.standing_turn[index])),
                );
                results
            }
            IntegrationScenario::StandingStop => {
                let mut results = vec![
                    ("runtime_assembly", coverage.assembly),
                    ("animation_graph", coverage.graph),
                    ("standing_graph", coverage.standing_graph),
                ];
                results.extend(
                    STANDING_STOP_CASES
                        .iter()
                        .enumerate()
                        .map(|(index, case)| (case.0, coverage.standing_stop[index])),
                );
                results
            }
            IntegrationScenario::StandingResponse => vec![
                ("runtime_assembly", coverage.assembly),
                ("animation_graph", coverage.graph),
                ("standing_graph", coverage.standing_graph),
                ("response_reverse", coverage.response_reverse),
                ("response_cycle", coverage.response_cycle),
                ("response_idle", coverage.response_idle),
            ],
            IntegrationScenario::StandingDirection => vec![
                ("runtime_assembly", coverage.assembly),
                ("animation_graph", coverage.graph),
                ("standing_graph", coverage.standing_graph),
                ("direction_regression", direction_regression),
                (
                    "direction_source_states",
                    coverage.direction_source_states == 0b11_1111,
                ),
            ],
            IntegrationScenario::Base => vec![
                ("runtime_assembly", coverage.assembly),
                ("animation_graph", coverage.graph),
                ("locomotion", coverage.locomotion),
                ("direction_backward", coverage.backward),
                ("direction_sideways", coverage.sideways),
                ("standing_graph", coverage.standing_graph),
                ("direction_regression", direction_regression),
                ("crouch_transaction", coverage.crouch_transaction),
                ("crouch_graph", coverage.crouch_graph),
                (
                    "crouch_air_forced_standing",
                    coverage.crouch_air_forced_standing,
                ),
                ("crouch_air_landed", coverage.crouch_air_landed),
                ("crouch_graph_recovered", coverage.crouch_graph_recovered),
                ("crouch_cycles_100", coverage.crouch_cycles),
                ("crouch_scaled", coverage.crouch_scaled),
                ("crouch_canopy_visible", coverage.crouch_canopy_visible),
                ("crouch_canopy_blocked", coverage.crouch_canopy_blocked),
                ("crouch_canopy_recovered", coverage.crouch_canopy_recovered),
                ("crouch_canopy_cleaned", coverage.crouch_canopy_cleaned),
                ("crouch_moving", coverage.crouch_moving),
                ("stance_crouching", coverage.crouching),
                (
                    "crouch_direction_six_way",
                    coverage
                        .crouch_direction_cases
                        .iter()
                        .all(|covered| *covered)
                        && coverage.crouch_direction_state_mask == 0b11_1111,
                ),
                ("crouch_direction_lean", coverage.crouch_direction_lean),
                (
                    "crouch_rotate_both_sides",
                    coverage.crouch_rotate_mask == 0b11
                        && coverage.crouch_rotate_completion_mask == 0b11,
                ),
                (
                    "crouch_turn_slot_active_both_sides",
                    coverage.crouch_turn_active_mask == 0b11,
                ),
                (
                    "crouch_turn_completion_both_sides",
                    coverage.crouch_turn_completion_mask == 0b11,
                ),
                ("gait_sprinting", coverage.sprinting),
                ("jump_air", coverage.airborne),
                ("jump_land", coverage.landed),
                ("jump_variant", coverage.jump_variant),
                ("overlay_layer", coverage.overlay),
                ("gait_layer", coverage.gait_layer),
                ("stance_layer", coverage.stance_layer),
                ("turn_in_place", coverage.turn_in_place),
                ("layering", coverage.layering),
                ("overlay_object", coverage.overlay_object),
                ("start_stop", coverage.start_stop),
                ("view_head", coverage.view_head),
                ("control_rig", coverage.control_rig),
                ("control_rig_stages", coverage.control_rig_stages),
                ("control_rig_input", coverage.control_rig_input),
                (
                    "control_rig_foot_targets",
                    coverage.control_rig_foot_targets,
                ),
                ("mantle_start", coverage.mantle_started),
                ("mantle_complete", coverage.mantle_completed),
                ("mantle_debug_geometry", coverage.mantle_debug),
                (
                    "mantle_actor_clock",
                    actor_montage_clock_passed(&gate_report.direction_trace, "mantle_actor_clock"),
                ),
                (
                    "mantle_parent_world",
                    gate_report.direction_trace.iter().any(|row| {
                        row.get("mantle_parent_world_valid")
                            .and_then(serde_json::Value::as_bool)
                            == Some(true)
                    }),
                ),
                (
                    "ground_prediction_debug_geometry",
                    coverage.ground_prediction_debug,
                ),
                ("movement_base", coverage.movement_base),
                ("foot_probe", coverage.foot_probe),
                ("foot_lock", coverage.foot_lock),
                ("roll_start", coverage.roll_started),
                ("roll_slot", coverage.roll_slot),
                (
                    "grounded_roll_exit_transition",
                    coverage.grounded_roll_exit_transition,
                ),
            ],
            IntegrationScenario::Grounded => vec![
                ("runtime_assembly", coverage.assembly),
                ("animation_graph", coverage.graph),
                ("roll_start", coverage.roll_started),
                ("roll_slot", coverage.roll_slot),
                ("grounded_roll_completed", coverage.grounded_roll_completed),
                (
                    "roll_actor_clock",
                    actor_montage_clock_passed(&gate_report.direction_trace, "roll_actor_clock"),
                ),
                ("grounded_stop_callback", coverage.grounded_stop_callback),
                (GROUNDED_JOINT_CASES[0].0, coverage.grounded_joint[0]),
                (GROUNDED_JOINT_CASES[1].0, coverage.grounded_joint[1]),
                (GROUNDED_JOINT_CASES[2].0, coverage.grounded_joint[2]),
                (GROUNDED_JOINT_CASES[3].0, coverage.grounded_joint[3]),
                (GROUNDED_JOINT_CASES[4].0, coverage.grounded_joint[4]),
                (GROUNDED_JOINT_CASES[5].0, coverage.grounded_joint[5]),
                (GROUNDED_JOINT_CASES[6].0, coverage.grounded_joint[6]),
                (GROUNDED_JOINT_CASES[7].0, coverage.grounded_joint[7]),
                (
                    "grounded_roll_exit_transition",
                    coverage.grounded_roll_exit_transition,
                ),
            ],
            IntegrationScenario::StateMachine => vec![
                ("runtime_assembly", coverage.assembly),
                ("animation_graph", coverage.graph),
                ("p1_ground_idle_start_cycle_stop_pivot", coverage.p1_ground),
                ("p1_stance_crouch_and_recover", coverage.p1_stance),
                ("p1_gait_walk_run_sprint_guard", coverage.p1_gait),
                ("p1_air_jump_fall_land", coverage.p1_air),
                ("p1_turn_and_rotate_both_sides", coverage.p1_turn),
                ("p1_turn_slot_pose", coverage.turn_slot),
                (
                    "p1_turn_slot_inertialization",
                    coverage.turn_inertialization,
                ),
                (
                    "generated_stop_quick_transition_notify",
                    coverage.generated_transition_stop_quick,
                ),
            ],
            IntegrationScenario::TransitionNotify => vec![
                ("runtime_assembly", coverage.assembly),
                ("animation_graph", coverage.graph),
                (
                    "generated_stop_quick_transition_notify",
                    coverage.generated_transition_stop_quick,
                ),
            ],
            IntegrationScenario::Crouching => vec![
                ("runtime_assembly", coverage.assembly),
                ("animation_graph", coverage.graph),
                (
                    "crouch_direction_six_way",
                    coverage
                        .crouch_direction_cases
                        .iter()
                        .all(|covered| *covered)
                        && coverage.crouch_direction_state_mask == 0b11_1111,
                ),
                ("crouch_direction_lean", coverage.crouch_direction_lean),
                (
                    "crouch_rotate_both_sides",
                    coverage.crouch_rotate_mask == 0b11
                        && coverage.crouch_rotate_completion_mask == 0b11,
                ),
                (
                    "crouch_turn_slot_active_both_sides",
                    coverage.crouch_turn_active_mask == 0b11,
                ),
                (
                    "crouch_turn_completion_both_sides",
                    coverage.crouch_turn_completion_mask == 0b11,
                ),
            ],
            IntegrationScenario::Sprint => vec![
                ("runtime_assembly", coverage.assembly),
                ("animation_graph", coverage.graph),
                ("standing_graph", coverage.standing_graph),
                ("locomotion", coverage.locomotion),
                ("gait_sprinting", coverage.sprinting),
                ("sprint_curve_relay", coverage.sprint_curve_relay),
                (
                    "sprint_acceleration_branch",
                    state.sprint_samples[0].is_some(),
                ),
                ("sprint_steady_branch", state.sprint_samples[1].is_some()),
            ],
            IntegrationScenario::ControlRig => vec![
                ("runtime_assembly", coverage.assembly),
                ("animation_graph", coverage.graph),
                ("standing_graph", coverage.standing_graph),
                ("control_rig", coverage.control_rig),
                ("control_rig_actions", coverage.control_rig_actions),
                ("control_rig_stages", coverage.control_rig_stages),
                ("control_rig_input", coverage.control_rig_input),
                (
                    "control_rig_foot_targets",
                    coverage.control_rig_foot_targets,
                ),
                ("foot_probe", coverage.foot_probe),
                ("foot_lock", coverage.foot_lock),
            ],
            IntegrationScenario::Head => {
                let probe = &state.head_probe;
                if state.timed_out {
                    gate_report.direction_trace.push(serde_json::json!({"watchdog": {"exit_reason": "case_progress_timeout", "last_progress": probe.case_index,
                        "no_progress_limit_seconds": 12.0, "waited_game_seconds": probe.sample.elapsed_seconds, "current_conditions": probe.sample}}));
                }
                let mut results = vec![
                    ("runtime_assembly", coverage.assembly),
                    ("animation_graph", coverage.graph),
                ];
                results.extend(
                    HEAD_WINDOWS
                        .iter()
                        .enumerate()
                        .map(|(index, label)| (*label, probe.covered[index])),
                );
                results.extend([
                    ("head_source_contract", probe.valid),
                    (
                        "head_look_samples",
                        probe.samples_seen.iter().all(|seen| *seen),
                    ),
                    ("head_mesh_additive", probe.additive_seen),
                    ("head_first_person_clock", probe.unscaled_pitch_seen),
                    ("head_complete", probe.case_index == HEAD_WINDOWS.len()),
                ]);
                results
            }
            IntegrationScenario::Footprint => {
                let probe = &state.footprint_probe;
                if state.timed_out {
                    gate_report.direction_trace.push(serde_json::json!({"watchdog": {
                        "exit_reason":"case_progress_timeout","last_progress":probe.stage,"no_progress_limit_seconds":12.0,
                        "waited_game_seconds":probe.waited,"current_conditions":probe.errors,
                        "gpu_request":probe.gpu.cursor + 1,"waited_gpu_seconds":probe.gpu.last_progress.map(|last| last.elapsed().as_secs_f32())}}));
                }
                let mut results = vec![
                    ("runtime_assembly", coverage.assembly),
                    ("animation_graph", coverage.graph),
                ];
                results.extend(
                    footprints::LABELS
                        .iter()
                        .enumerate()
                        .map(|(index, label)| (*label, probe.covered[index])),
                );
                results.extend([
                    ("footprint_visible", probe.visible),
                    ("footprint_fade", probe.faded),
                    ("footprint_expired", probe.expired),
                    ("footprint_moving_surface", probe.moving),
                    ("footprint_character_receiver", probe.receiver),
                    ("footprint_gpu_pipelines", probe.pipelines),
                    ("footprint_gpu_lifecycle", probe.gpu.complete),
                    ("footprint_moving_gpu", probe.moving_gpu),
                    ("footprint_debug_traces", probe.debug),
                    ("footprint_source_consistency", probe.errors.is_empty()),
                    ("footprint_rotating_ride", state.moving_base_probe.ride),
                ]);
                results
            }
            IntegrationScenario::Overlays => {
                let probe = &state.overlays_probe;
                if state.timed_out {
                    gate_report.direction_trace.push(serde_json::json!({"watchdog":{"exit_reason":"case_progress_timeout","mode":probe.mode,
                        "stage":probe.stage,"waited_game_seconds":probe.waited,"no_progress_limit_seconds":12.0,"current_conditions":probe.sample}}));
                }
                let mut results = vec![
                    ("runtime_assembly", coverage.assembly),
                    ("animation_graph", coverage.graph),
                ];
                for (mode, labels) in overlays::LABELS.iter().enumerate() {
                    results.extend(
                        labels
                            .iter()
                            .enumerate()
                            .map(|(stage, label)| (*label, probe.covered[mode][stage])),
                    );
                }
                results.push((
                    "overlay_gun_states",
                    probe.fsm_states.iter().flatten().all(|value| *value),
                ));
                results.push((
                    "overlay_source_consistency",
                    !gate_report.direction_trace.is_empty()
                        && gate_report.direction_trace.iter().all(|entry| {
                            entry["sample"]["errors"]
                                .as_array()
                                .is_some_and(Vec::is_empty)
                        }),
                ));
                results.push(("overlays_complete", probe.mode == overlays::MODES.len()));
                results
            }
            IntegrationScenario::BowOverlay => {
                if state.timed_out {
                    gate_report.direction_trace.push(serde_json::json!({"watchdog": {"exit_reason": "case_progress_timeout",
                        "last_progress": state.bow_probe.index, "no_progress_limit_seconds": 12.0, "waited_game_seconds": state.bow_probe.waited_seconds,
                        "current_conditions": state.bow_probe.sample}}));
                }
                let mut results = vec![
                    ("runtime_assembly", coverage.assembly),
                    ("animation_graph", coverage.graph),
                ];
                results.extend(
                    bow::LABELS
                        .iter()
                        .enumerate()
                        .map(|(index, label)| (*label, state.bow_probe.covered[index])),
                );
                results.push(("bow_complete", state.bow_probe.index == bow::LABELS.len()));
                results.extend([
                    ("bow_object_material", state.bow_probe.material_seen),
                    ("bow_object_mantle_clear", state.bow_probe.cleared[0]),
                    ("bow_object_mantle_restore", state.bow_probe.restored[0]),
                    ("bow_object_ragdoll_clear", state.bow_probe.cleared[1]),
                    ("bow_object_ragdoll_restore", state.bow_probe.restored[1]),
                    ("bow_mantle_complete", coverage.mantle_completed),
                    ("bow_ragdoll_recovered", coverage.ragdoll_recovered),
                ]);
                results
            }
            IntegrationScenario::DefaultOverlay => {
                let probe = &state.default_overlay_probe;
                if state.timed_out {
                    gate_report.direction_trace.push(serde_json::json!({"watchdog": {"exit_reason": "case_progress_timeout", "last_progress": probe.case_index,
                        "no_progress_limit_seconds": 12.0, "current_conditions": probe.sample}}));
                }
                let mut results = vec![
                    ("runtime_assembly", coverage.assembly),
                    ("animation_graph", coverage.graph),
                ];
                results.extend(
                    DEFAULT_OVERLAY_WINDOWS
                        .iter()
                        .enumerate()
                        .map(|(index, label)| (*label, probe.covered[index])),
                );
                results.extend([
                    (
                        "default_source_poses",
                        probe.valid && probe.evaluators_seen.iter().all(|value| *value),
                    ),
                    ("default_additive_applied", probe.additive_seen),
                    ("default_overlay_curves", probe.curves_seen),
                    ("default_prediction_increase", probe.prediction_increased),
                    ("default_prediction_decrease", probe.prediction_decreased),
                    (
                        "layering_source_regions",
                        probe.sample.layering.boundaries.len() == LAYERING_BOUNDARIES.len(),
                    ),
                    (
                        "layering_curve_operators",
                        probe.sample.layering.curve_operators == 3 && probe.valid,
                    ),
                    (
                        "layering_previous_curve_inputs",
                        probe.sample.layering.ready && probe.valid,
                    ),
                    ("default_complete", probe.case_index == 6),
                ]);
                results
            }
            IntegrationScenario::FootIk => {
                if state.timed_out
                    && !(coverage.foot_bones_replaced && coverage.camera_root_rebound)
                {
                    gate_report.direction_trace.push(serde_json::json!({"watchdog": {
                        "exit_reason": "foot_avatar_replacement_timeout",
                        "last_progress": state.foot_replacement_progress,
                        "waited_game_seconds": state.foot_replacement_waited,
                        "no_progress_limit_seconds": FOOT_AVATAR_REPLACEMENT_NO_PROGRESS_LIMIT_SECONDS,
                        "current_conditions": {"selected": state.avatar == state.foot_replacement_avatar,
                            "covered": coverage.foot_bones_replaced,
                            "camera_root_rebound": coverage.camera_root_rebound,
                            "animation_phase_rebound": coverage.animation_phase_rebound}
                    }}));
                }
                let mut results = vec![
                    ("runtime_assembly", coverage.assembly),
                    ("animation_graph", coverage.graph),
                    ("standing_graph", coverage.standing_graph),
                    ("direction_regression", direction_regression),
                    ("foot_probe", coverage.foot_probe),
                    ("foot_lock", coverage.foot_lock),
                    ("foot_ik_space_baseline", coverage.foot_ik_space_baseline),
                    (
                        "foot_ik_no_hit_recovered",
                        coverage.foot_ik_no_hit_recovered,
                    ),
                    ("foot_ik_weight_ramp", coverage.foot_ik_weight_ramp),
                    ("foot_ik_teleport", coverage.foot_ik_teleport),
                    ("foot_ik_slope", coverage.foot_ik_slope),
                    ("foot_ik_step", coverage.foot_ik_step),
                    ("foot_ik_crouching", coverage.foot_ik_crouching),
                    ("foot_ik_jump_air", coverage.foot_ik_jump_air),
                    ("foot_ik_landed", coverage.foot_ik_landed),
                    ("foot_ik_roll", coverage.foot_ik_roll),
                    ("foot_ik_mantle", coverage.foot_ik_mantle),
                    ("mantle_start", coverage.mantle_started),
                    ("mantle_complete", coverage.mantle_completed),
                    ("foot_ik_terrain_cleaned", coverage.foot_ik_terrain_cleaned),
                    ("foot_bones_replaced", coverage.foot_bones_replaced),
                    ("camera_root_rebound", coverage.camera_root_rebound),
                ];
                if std::env::var_os("ALS_MANUAL_ANIMATION_PHASE_REBUILD").is_some() {
                    results.push(("animation_phase_rebound", coverage.animation_phase_rebound));
                }
                results
            }
            IntegrationScenario::Ragdoll => vec![
                ("runtime_assembly", coverage.assembly),
                ("animation_graph", coverage.graph),
                ("ragdoll_enter", coverage.ragdoll_entered),
                ("ragdoll_pa_profile", coverage.ragdoll_pa_profile),
                ("ragdoll_follow", coverage.ragdoll_follow),
                ("ragdoll_scaled", coverage.ragdoll_scaled),
                ("ragdoll_joint_drive", coverage.ragdoll_joint_drive),
                ("ragdoll_recover", coverage.ragdoll_recovered),
                (
                    "get_up_grounded",
                    !coverage.get_up_airborne && (coverage.get_up_front || coverage.get_up_back),
                ),
                (
                    "get_up_rig_space",
                    !coverage.get_up_rig_space_mismatch
                        && (coverage.get_up_front || coverage.get_up_back),
                ),
                ("ragdoll_camera_pivot", coverage.ragdoll_camera_pivot),
                (
                    "get_up_input_locked",
                    coverage.get_up_input_locked.into_iter().any(|value| value),
                ),
                (
                    "get_up_input_released",
                    coverage
                        .get_up_input_released
                        .into_iter()
                        .any(|value| value),
                ),
            ],
            IntegrationScenario::GetUp => vec![
                ("runtime_assembly", coverage.assembly),
                ("animation_graph", coverage.graph),
                ("ragdoll_pa_profile", coverage.ragdoll_pa_profile),
                ("ragdoll_recover", coverage.ragdoll_recovered),
                (
                    "get_up_grounded",
                    !coverage.get_up_airborne && coverage.get_up_front && coverage.get_up_back,
                ),
                ("ragdoll_camera_pivot", coverage.ragdoll_camera_pivot),
                ("get_up_front", coverage.get_up_front),
                ("get_up_back", coverage.get_up_back),
                ("get_up_front_input_locked", coverage.get_up_input_locked[0]),
                (
                    "get_up_front_input_released",
                    coverage.get_up_input_released[0],
                ),
                ("get_up_back_input_locked", coverage.get_up_input_locked[1]),
                (
                    "get_up_back_input_released",
                    coverage.get_up_input_released[1],
                ),
                (
                    "get_up_native_jump_rejected_during_action",
                    coverage.get_up_native_jump_rejected_during_action,
                ),
                (
                    "get_up_actor_clock",
                    actor_montage_clock_passed(&gate_report.direction_trace, "get_up_actor_clock"),
                ),
            ],
            IntegrationScenario::Camera => vec![
                ("runtime_assembly", coverage.assembly),
                ("animation_graph", coverage.graph),
                ("camera_rig", coverage.camera_rig),
                ("camera_collision", coverage.camera_collision),
                ("camera_collision_recovery", coverage.camera_recovery),
                ("camera_first_person", coverage.camera_first_person),
                ("camera_shoulder", coverage.camera_shoulder),
                ("camera_shake", coverage.camera_shake),
                ("camera_unscaled_time", coverage.camera_unscaled_time),
                (
                    "camera_player_view_movement",
                    state.camera_movement.passed(),
                ),
                ("camera_debug_geometry", coverage.camera_debug_geometry),
            ],
            IntegrationScenario::Stairs => vec![
                ("runtime_assembly", coverage.assembly),
                ("animation_graph", coverage.graph),
                ("stairs_01_up", state.stairs_probe.completed[0]),
                ("stairs_01_down", state.stairs_probe.completed[1]),
                ("stairs_02_up", state.stairs_probe.completed[2]),
                ("stairs_02_down", state.stairs_probe.completed[3]),
                ("stairs_03_up", state.stairs_probe.completed[4]),
                ("stairs_03_down", state.stairs_probe.completed[5]),
                ("stairs_progress", state.stairs_probe.failure.is_none()),
            ],
            IntegrationScenario::UphillJump => vec![
                ("runtime_assembly", coverage.assembly),
                ("animation_graph", coverage.graph),
                (
                    "uphill_01",
                    state.uphill_probe.completed[..6].iter().all(|value| *value),
                ),
                (
                    "uphill_02",
                    state.uphill_probe.completed[6..12]
                        .iter()
                        .all(|value| *value),
                ),
                (
                    "uphill_03",
                    state.uphill_probe.completed[12..]
                        .iter()
                        .all(|value| *value),
                ),
                (
                    "uphill_landing_consistency",
                    state.uphill_probe.failure.is_none(),
                ),
            ],
            IntegrationScenario::MovingBase | IntegrationScenario::RotatingBase => {
                let mut results = vec![
                    ("runtime_assembly", coverage.assembly),
                    ("animation_graph", coverage.graph),
                    ("moving_base_ride", state.moving_base_probe.ride),
                    ("moving_base_jump", state.moving_base_probe.air),
                    ("moving_base_new_support", state.moving_base_probe.landing),
                    ("moving_base_mantle", state.moving_base_probe.moving_mantle),
                    ("mantle_start", coverage.mantle_started),
                    ("mantle_complete", coverage.mantle_completed),
                    (
                        "moving_base_after_mantle",
                        state.moving_base_probe.after_mantle,
                    ),
                    (
                        "moving_base_progress",
                        state.moving_base_probe.failure.is_none(),
                    ),
                ];
                if state.scenario == IntegrationScenario::RotatingBase {
                    results.extend([
                        ("rotating_base_left_step", state.moving_base_probe.steps[0]),
                        ("rotating_base_right_step", state.moving_base_probe.steps[1]),
                    ]);
                }
                results
            }
            IntegrationScenario::Interface => vec![
                ("runtime_assembly", coverage.assembly),
                ("animation_graph", coverage.graph),
                (
                    "overlay_parent_instance_preserved",
                    state.interface_parent_instance.is_some() && state.interface_parent_preserved,
                ),
                ("hud_layout", coverage.hud_layout),
                ("hud_toggle", coverage.hud_toggle),
                ("hud_slomo", coverage.hud_slomo),
                ("overlay_menu", coverage.overlay_menu),
                (
                    "overlay_interface_binding",
                    coverage.overlay_interface_binding,
                ),
                (
                    "overlay_interface_fallback",
                    coverage.overlay_interface_fallback,
                ),
                (
                    "overlay_interface_reentry",
                    coverage.overlay_interface_reentry,
                ),
                ("show_debug", coverage.show_debug),
                ("debug_draw", coverage.debug_draw),
                (
                    "debug_key_lifecycle",
                    hud_layout::debug_keys_passed(&gate_report.direction_trace),
                ),
                (
                    "debug_canvas_layout",
                    hud_layout::passed(&gate_report.direction_trace),
                ),
                (
                    "debug_values",
                    gate_report
                        .direction_trace
                        .iter()
                        .any(|entry| entry.get("hud_data_valid").is_some())
                        && gate_report
                            .direction_trace
                            .iter()
                            .filter_map(|entry| {
                                entry
                                    .get("hud_data_valid")
                                    .and_then(serde_json::Value::as_bool)
                            })
                            .all(|valid| valid),
                ),
            ],
            IntegrationScenario::AnimationClock => {
                let mut results = vec![
                    ("runtime_assembly", coverage.assembly),
                    ("animation_graph", coverage.graph),
                ];
                results.extend(animation_clock_results(&gate_report.direction_trace));
                results
            }
            IntegrationScenario::Visibility => {
                let mut results = vec![
                    ("runtime_assembly", coverage.assembly),
                    ("animation_graph", coverage.graph),
                ];
                results.extend(visibility_case_results(&gate_report.direction_trace));
                results
            }
            IntegrationScenario::Notify => vec![
                ("runtime_assembly", coverage.assembly),
                ("animation_graph", coverage.graph),
                ("notify_sprint", coverage.notify_sprint),
                (
                    "notify_sprint_acceleration",
                    coverage.notify_sprint_acceleration,
                ),
                ("notify_roll", coverage.notify_roll),
                ("notify_roll_reentry", coverage.notify_roll_reentry),
                ("notify_marker", coverage.notify_marker),
                ("notify_grounded_entry", coverage.notify_grounded_entry),
                ("notify_root_scale", coverage.notify_root_scale),
                ("notify_rolling_action", coverage.notify_rolling_action),
                ("notify_mantling_action", coverage.notify_mantling_action),
                ("notify_early_blend_out", coverage.notify_early_blend_out),
                ("camera_shake", coverage.camera_shake),
            ],
        };
        if state.scenario == IntegrationScenario::Ragdoll && state.expect_ragdoll_player_replacement
        {
            results.push((
                "ragdoll_player_replacement",
                coverage.ragdoll_player_replaced,
            ));
        }
        if state.scenario == IntegrationScenario::Ragdoll && state.expect_ragdoll_debug_render {
            results.push(("ragdoll_debug_render", coverage.ragdoll_debug_render));
        }
        if state.scenario == IntegrationScenario::ControlRig
            && state.expect_control_rig_definition_hot_swap
        {
            results.push((
                "control_rig_definition_hot_swap",
                coverage.control_rig_definition_hot_swap,
            ));
        }
        if state.scenario == IntegrationScenario::Footprint && state.expect_decal_receiver_swap {
            results.push(("decal_receiver_swap", coverage.decal_receiver_swap));
        }
        if state.scenario == IntegrationScenario::Base && state.expect_reroute_passthrough {
            results.push(("reroute_passthrough", coverage.reroute_passthrough));
        }
        if state.scenario == IntegrationScenario::Base && state.expect_machine_weight {
            results.extend([
                ("machine_weight_partial", coverage.machine_weight_partial),
                ("machine_weight_full", coverage.machine_weight_full),
                (
                    "machine_weight_low_start",
                    coverage.machine_weight_low_start,
                ),
                (
                    "machine_weight_full_start",
                    coverage.machine_weight_full_start,
                ),
            ]);
        }
        for (name, covered) in &results {
            if *covered {
                info!("[MANUAL-INTEGRATION][EXECUTED] {name}");
            } else {
                error!("[MANUAL-INTEGRATION][NOT_COVERED] {name}");
            }
        }
        let passed = !state.timed_out && results.iter().all(|(_, covered)| *covered);
        let ragdoll_profile_detail = format!(
            "handoff_animated_body_position_error_m={:.6}; handoff_animated_body_rotation_error_radians={:.6}; handoff_locked_joint_anchor_error_m={:.6}; handoff_locked_joint_anchor_label={}; current_joint_anchor_error_m={:.6}; current_joint_anchor_label={}; max_joint_anchor_error_m={:.6}; max_joint_anchor_label={}",
            state.ragdoll_handoff_pose_error_m,
            state.ragdoll_handoff_rotation_error_radians,
            state.ragdoll_handoff_locked_joint_anchor_error_m,
            state
                .ragdoll_handoff_locked_joint_anchor_label
                .as_deref()
                .unwrap_or("none"),
            state.ragdoll_current_joint_anchor_error_m,
            state
                .ragdoll_current_joint_anchor_label
                .as_deref()
                .unwrap_or("none"),
            state.ragdoll_max_joint_anchor_error_m,
            state
                .ragdoll_max_joint_anchor_label
                .as_deref()
                .unwrap_or("none"),
        );
        gate_report.scenario = format!("{:?}", state.scenario);
        gate_report.timed_out = state.timed_out;
        gate_report.completed = true;
        gate_report.passed = Some(passed);
        gate_report.results = results
            .iter()
            .map(|(name, covered)| ManualIntegrationGateResult {
                name: (*name).to_owned(),
                covered: *covered,
                detail: if *name == "grounded_roll_exit_transition" {
                    grounded_roll_exit_detail.clone()
                } else if *name == "generated_stop_quick_transition_notify" {
                    transition_notify_detail.clone()
                } else if *name == "ragdoll_pa_profile" {
                    ragdoll_profile_detail.clone()
                } else if *name == "direction_source_states" {
                    format!(
                        "sampled_state_mask={:06b}; required=111111",
                        coverage.direction_source_states
                    )
                } else if matches!(*name, "sprint_acceleration_branch" | "sprint_steady_branch") {
                    let index = usize::from(*name == "sprint_steady_branch");
                    format!(
                        "source_time_acceleration={:?}; canonical_phase={:?}",
                        state.sprint_samples[index], state.sprint_signature_phase
                    )
                } else if *name == "camera_collision_recovery" {
                    format!(
                        "samples={} last={:?}",
                        gate_report.camera_trace.len(),
                        gate_report.camera_trace.last()
                    )
                } else if *name == "stairs_progress" {
                    state
                        .stairs_probe
                        .failure
                        .clone()
                        .unwrap_or_else(|| "all routes completed without stalls".to_owned())
                } else if *name == "uphill_landing_consistency" {
                    state.uphill_probe.failure.clone().unwrap_or_else(|| {
                        "all uphill landings returned to grounded animation".to_owned()
                    })
                } else if *name == "moving_base_progress" {
                    state
                        .moving_base_probe
                        .failure
                        .clone()
                        .unwrap_or_else(|| "source object traversal completed".to_owned())
                } else if *name == "runtime_assembly" {
                    format!(
                        "game_time_seconds={:.3}; relative_speed={:.3}; slomo_active={}",
                        state.game_time_seconds,
                        interface.virtual_time.relative_speed(),
                        interface.hud_state.slomo_active
                    )
                } else if *covered {
                    "covered".to_owned()
                } else {
                    "not covered".to_owned()
                },
            })
            .collect();
        if passed {
            info!(
                "[MANUAL-INTEGRATION][VERDICT] scenario={:?} real_entry_paths -> PASS awaiting_visual_evidence",
                state.scenario
            );
        } else {
            error!(
                "[MANUAL-INTEGRATION][VERDICT] scenario={:?} real_entry_paths -> FAIL timed_out={} awaiting_visual_evidence",
                state.scenario, state.timed_out
            );
        }
    }
    if state.phase == IntegrationPhase::Finish
        && state.final_logged
        && !state.exit_requested
        && manual_gate_exit_ready(interface.visual_progress.as_deref(), state.timed_out)
    {
        state.exit_requested = true;
        if gate_report.passed == Some(true) {
            exit.write(AppExit::Success);
        } else {
            exit.write(AppExit::Error(NonZeroU8::new(91).unwrap()));
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn browser_review_air_roll_uses_the_configured_mode_change_ragdoll_entry() {
        use bevy_als::als::{
            character::AlsMovementParamsHandle,
            params::AlsMovementParams,
            state::{AlsNotifyLocomotionAction, LocomotionState},
        };
        use avian3d::prelude::{LinearVelocity, Position};
        use bevy::prelude::*;
        for (enabled, already_air, rolling) in [
            (false, false, true),
            (true, false, true),
            (true, true, true),
            (true, false, false),
        ] {
            let (mut app, character, _) = crate::test_support::loaded_character();
            let handle = app
                .world()
                .get::<AlsMovementParamsHandle>(character)
                .unwrap()
                .0
                .clone();
            let mut values = serde_json::to_value(
                app.world()
                    .resource::<Assets<AlsMovementParams>>()
                    .get(&handle)
                    .unwrap(),
            )
            .unwrap();
            values["rolling"] = serde_json::json!({"interrupt_when_in_air": enabled});
            let configured = serde_json::from_value(values).unwrap();
            *app.world_mut()
                .resource_mut::<Assets<AlsMovementParams>>()
                .get_mut(&handle)
                .unwrap() = configured;
            app.world_mut()
                .get_mut::<crate::input::AlsInputCommand>(character)
                .unwrap()
                .roll_pressed = true;
            app.update();
            app.world_mut()
                .get_mut::<crate::input::AlsInputCommand>(character)
                .unwrap()
                .roll_pressed = false;
            if !rolling {
                app.world_mut()
                    .entity_mut(character)
                    .insert(AlsNotifyLocomotionAction::None);
            }
            app.world_mut()
                .entity_mut(character)
                .insert(if already_air {
                    LocomotionState::Air
                } else {
                    LocomotionState::Grounded
                });
            let target = app.world().get::<Position>(character).unwrap().0 + Vec3::Y * 3.0;
            app.world_mut().get_mut::<Position>(character).unwrap().0 = target;
            app.world_mut()
                .get_mut::<Transform>(character)
                .unwrap()
                .translation = target;
            app.world_mut()
                .get_mut::<LinearVelocity>(character)
                .unwrap()
                .0 = Vec3::ZERO;
            app.update();
            let actual = *app
                .world()
                .get::<AlsNotifyLocomotionAction>(character)
                .unwrap();
            let expected = enabled && !already_air && rolling;
            assert_eq!(
                actual == AlsNotifyLocomotionAction::Ragdolling,
                expected,
                "Air roll mode change did not use the source interrupt configuration: enabled={enabled}, already_air={already_air}, rolling={rolling}, actual={actual:?}"
            );
            if expected {
                assert_eq!(
                    *app.world().get::<LocomotionState>(character).unwrap(),
                    LocomotionState::None
                );
                assert_eq!(
                    *app.world().get::<bevy_tnua::TnuaToggle>(character).unwrap(),
                    bevy_tnua::TnuaToggle::Disabled
                );
            }
        }
    }

    #[test]
    fn browser_review_base_midframe_scale_keeps_crouch_cycles_grounded() {
        use bevy_als::als::{
            state::{LocomotionState, Stance},
            transition::AlsStateRequest,
        };
        use avian3d::prelude::{LinearVelocity, Position};
        use bevy::prelude::*;
        for scale in [0.5, 1.0, 2.0] {
            let (mut app, character, _) = crate::test_support::loaded_character();
            let center = Vec3::new(0.0, 0.9 * scale + 0.024, 0.0);
            app.world_mut()
                .get_mut::<Transform>(character)
                .unwrap()
                .scale = Vec3::splat(scale);
            app.world_mut()
                .get_mut::<Transform>(character)
                .unwrap()
                .translation = center;
            app.world_mut().get_mut::<Position>(character).unwrap().0 = center;
            app.world_mut()
                .get_mut::<LinearVelocity>(character)
                .unwrap()
                .0 = Vec3::ZERO;
            let mut previous = Stance::Standing;
            let mut pairs = 0;
            let mut trace = Vec::new();
            for frame in 0..400 {
                let actual = *app.world().get::<Stance>(character).unwrap();
                app.world_mut()
                    .get_mut::<AlsStateRequest>(character)
                    .unwrap()
                    .stance = Some(if actual == Stance::Standing {
                    Stance::Crouching
                } else {
                    Stance::Standing
                });
                app.update();
                let actual = *app.world().get::<Stance>(character).unwrap();
                if previous == Stance::Crouching && actual == Stance::Standing {
                    pairs += 1;
                }
                previous = actual;
                trace.push((
                    frame,
                    actual,
                    *app.world().get::<LocomotionState>(character).unwrap(),
                    app.world().get::<Position>(character).unwrap().0.y,
                    app.world().get::<LinearVelocity>(character).unwrap().0.y,
                ));
                if pairs == 100 {
                    break;
                }
            }
            assert_eq!(
                pairs,
                100,
                "Scaled live actor cannot complete the source crouch cycles: scale={scale}, {:?}",
                &trace[trace.len().saturating_sub(8)..]
            );
            assert_eq!(
                *app.world().get::<LocomotionState>(character).unwrap(),
                LocomotionState::Grounded
            );
        }
    }

    #[test]
    fn locomotion_crouch_toggles_fire_once_at_each_scripted_boundary() {
        for cadence in [
            vec![1.0 / 30.0],
            vec![1.0 / 60.0],
            vec![1.0 / 120.0],
            vec![0.011, 0.027, 0.019, 0.041],
        ] {
            let mut elapsed = 0.0;
            let mut frame = 0;
            let mut pulses = Vec::new();
            while elapsed < 10.0 {
                let delta = cadence[frame % cadence.len()];
                elapsed += delta;
                if locomotion_crouch_pressed(elapsed, delta) {
                    pulses.push(elapsed);
                }
                frame += 1;
            }
            assert_eq!(
                pulses.len(),
                2,
                "Repeated crouch toggles depend on cadence: {pulses:?}"
            );
            for (pulse, boundary) in pulses.into_iter().zip([7.0, 8.5]) {
                assert!(
                    pulse >= boundary
                        && pulse - boundary <= cadence.iter().copied().fold(0.0, f64::max)
                );
            }
        }
    }

    #[test]
    fn locomotion_mouse_window_keeps_the_same_counts_across_cadences() {
        for cadence in [
            vec![1.0_f32 / 30.0],
            vec![1.0 / 60.0],
            vec![1.0 / 120.0],
            vec![0.011, 0.027, 0.019, 0.041],
        ] {
            let mut elapsed = 0.0_f32;
            let mut total = Vec2::ZERO;
            for frame in 0..1000 {
                let delta = cadence[frame % cadence.len()];
                elapsed += delta;
                total += locomotion_mouse_delta(elapsed, delta);
                if elapsed >= 2.0 {
                    break;
                }
            }
            assert!(
                total.abs_diff_eq(Vec2::new(360.0, -180.0), 0.002),
                "Timed locomotion mouse input changed with cadence {cadence:?}: {total:?}"
            );
        }
    }

    #[test]
    fn animation_clock_gate_rejects_pause_drift_and_missing_one_frame() {
        let row = |elapsed, state_elapsed| serde_json::json!({"elapsed":elapsed,"state_elapsed":state_elapsed,"state":"T19"});
        let mut rows = vec![
            row(2.4, 0.1),
            row(2.8, 0.1),
            row(3.02, 0.12),
            row(3.4, 0.12),
            row(3.6, 0.12),
            row(3.9, 0.15),
            row(4.1, 0.25),
            row(4.8, 0.5),
            row(5.5, 1.2),
        ];
        for dilation in [0.0, 0.5, 1.0, 2.0] {
            for _ in 0..3 {
                rows.push(serde_json::json!({"elapsed":6.5,"dilation":dilation,"movement_clock_error":0.0}));
            }
        }
        assert!(
            animation_clock_results(&rows)
                .iter()
                .all(|(_, passed)| *passed)
        );
        let mut drift = rows.clone();
        drift[1]["state_elapsed"] = serde_json::json!(0.2);
        assert!(
            animation_clock_results(&drift)
                .iter()
                .any(|(_, passed)| !passed)
        );
        let mut missing = rows.clone();
        for row in &mut missing {
            if row["elapsed"].as_f64().unwrap() < 3.8 {
                row["state_elapsed"] = serde_json::json!(0.1);
            }
        }
        assert!(
            animation_clock_results(&missing)
                .iter()
                .any(|(_, passed)| !passed)
        );
        let mut movement_drift = rows.clone();
        movement_drift.last_mut().unwrap()["movement_clock_error"] = serde_json::json!(0.01);
        assert!(
            animation_clock_results(&movement_drift)
                .iter()
                .any(|(_, passed)| !passed)
        );
        let no_pause = rows
            .into_iter()
            .filter(|row| row["dilation"].as_f64() != Some(0.0))
            .collect::<Vec<_>>();
        assert!(
            animation_clock_results(&no_pause)
                .iter()
                .any(|(_, passed)| !passed)
        );
    }

    #[test]
    fn visibility_case_gate_rejects_missing_freeze_pending_clock_and_notifications() {
        let row = |phase,
                   elapsed,
                   frame,
                   graph,
                   bones,
                   pending,
                   roll_time,
                   roll_action,
                   shakes,
                   footsteps| serde_json::json!({"visibility_phase":phase,"elapsed":elapsed,"frame":frame,"update_graph":graph,"refresh_bones":bones,"pending":pending,"roll_time":roll_time,"roll_action":roll_action,"roll_slot_relevant":false,"shakes":shakes,"footsteps":footsteps});
        let rows = vec![
            row("visible", 1.0, 10, true, true, false, 0.0, false, 0, 0),
            row("visible", 1.5, 20, true, true, false, 0.0, false, 0, 0),
            row("hidden", 3.5, 30, false, false, true, 0.0, false, 0, 0),
            row("hidden", 4.0, 30, false, false, true, 0.0, false, 0, 0),
            row("resume", 4.6, 31, true, true, true, 0.0, false, 0, 0),
            row("montage", 8.1, 60, false, false, true, 0.1, true, 0, 0),
            row("montage", 9.0, 60, false, false, true, 1.0, false, 0, 0),
            row("always_pose", 9.8, 61, true, false, true, 1.1, false, 0, 0),
            row(
                "always_pose",
                10.5,
                71,
                true,
                false,
                false,
                1.2,
                false,
                0,
                0,
            ),
            row(
                "visible_final",
                12.0,
                81,
                true,
                true,
                false,
                1.2,
                false,
                0,
                0,
            ),
            row(
                "visible_final",
                12.5,
                91,
                true,
                true,
                false,
                1.2,
                false,
                0,
                0,
            ),
        ];
        assert!(
            visibility_case_results(&rows)
                .iter()
                .all(|(_, passed)| *passed)
        );
        let mut landing_before_montage = rows.clone();
        landing_before_montage.push(row("visible", 0.31, 5, true, true, false, 0.0, false, 0, 1));
        assert!(
            visibility_case_results(&landing_before_montage)
                .iter()
                .all(|(_, passed)| *passed),
            "A visible landing notification must not change the hidden montage notification gate"
        );
        for (index, key, value) in [
            (3, "frame", serde_json::json!(31)),
            (4, "pending", serde_json::json!(false)),
            (6, "roll_time", serde_json::json!(0.0)),
            (5, "roll_action", serde_json::json!(false)),
            (6, "roll_action", serde_json::json!(true)),
            (5, "roll_slot_relevant", serde_json::json!(true)),
            (6, "footsteps", serde_json::json!(1)),
            (6, "shakes", serde_json::json!(1)),
            (5, "visibility_phase", serde_json::json!("visible")),
            (8, "frame", serde_json::json!(61)),
        ] {
            let mut changed = rows.clone();
            changed[index][key] = value;
            assert!(
                visibility_case_results(&changed)
                    .iter()
                    .any(|(_, passed)| !passed)
            );
            if matches!(
                key,
                "footsteps" | "shakes" | "visibility_phase" | "roll_action" | "roll_slot_relevant"
            ) {
                assert!(
                    !visibility_case_results(&changed)
                        .iter()
                        .find(|(name, _)| *name == "visibility_montage_notifications")
                        .unwrap()
                        .1,
                    "Montage notification gate must reject queued events without slot relevance and require the branching lifecycle"
                );
            }
        }
        for (time, phase) in [
            (1.99, "visible"),
            (2.0, "hidden"),
            (4.5, "resume"),
            (6.5, "montage"),
            (9.5, "always_pose"),
            (11.5, "visible_final"),
            (13.5, "finish"),
        ] {
            assert_eq!(visibility_case_phase(time), phase);
        }
    }

    #[test]
    fn notify_reentry_driver_waits_for_a_stopped_tail_and_requests_once() {
        for elapsed in [0.4, 0.83, 0.9, 0.95, bevy_als::als::roll::ROLL_DURATION_SECS] {
            let blend_start =
                bevy_als::als::roll::ROLL_DURATION_SECS - bevy_als::als::roll::ROLL_WEIGHT_OUT_SECS;
            let roll = AlsRollState {
                active: true,
                time: elapsed,
                natural_blend_start: (elapsed >= blend_start).then_some(blend_start),
                ..default()
            };
            assert_eq!(
                roll_tail_reentry_ready(Some(&roll), LocomotionState::Grounded, false),
                elapsed >= 0.9 && elapsed < bevy_als::als::roll::ROLL_DURATION_SECS
            );
            assert!(!roll_tail_reentry_ready(
                Some(&roll),
                LocomotionState::Grounded,
                true
            ));
            assert!(!roll_tail_reentry_ready(
                Some(&roll),
                LocomotionState::Air,
                false
            ));
        }
        assert!(!roll_tail_reentry_ready(
            None,
            LocomotionState::Grounded,
            false
        ));
    }

    #[test]
    fn state_machine_rotation_rebases_each_scripted_input_discontinuity_once() {
        for (boundary, label) in [
            (1.3, "state_machine_aim_view_flip"),
            (2.6, "state_machine_first_person_enter"),
            (3.9, "state_machine_first_person_view_flip"),
        ] {
            assert_eq!(
                super::state_machine_rotation_boundary(boundary - 0.01, 0.005),
                None
            );
            assert_eq!(
                super::state_machine_rotation_boundary(boundary, 0.01),
                Some(label)
            );
            assert_eq!(
                super::state_machine_rotation_boundary(boundary + 0.005, 0.01),
                Some(label)
            );
            assert_eq!(
                super::state_machine_rotation_boundary(boundary + 0.02, 0.01),
                None
            );
            assert_eq!(super::state_machine_rotation_boundary(boundary, 0.0), None);
        }
    }

    #[test]
    fn p1_rotation_coverage_reads_actual_source_state_and_pose() {
        use super::*;
        let mut drive = AlsTurnDrive {
            rotate_active: true,
            ..default()
        };
        let mut graph = AlsStandingGraphTelemetry {
            actual_standing_state: Some(3),
            actual_standing_transition_active: Some(false),
            actual_standing_pose_ready: Some(true),
            ..default()
        };
        assert_eq!(p1_turn_evidence_mask(&drive, Some(&graph)), 4);
        graph.actual_standing_state = Some(4);
        assert_eq!(p1_turn_evidence_mask(&drive, Some(&graph)), 8);
        graph.actual_standing_pose_ready = Some(false);
        assert_eq!(p1_turn_evidence_mask(&drive, Some(&graph)), 0);
        graph.actual_standing_pose_ready = Some(true);
        graph.actual_standing_transition_active = Some(true);
        assert_eq!(p1_turn_evidence_mask(&drive, Some(&graph)), 0);
        graph.actual_standing_transition_active = Some(false);
        graph.actual_standing_state = Some(0);
        drive.rotate_weight = 1.0;
        assert_eq!(p1_turn_evidence_mask(&drive, Some(&graph)), 0);
        drive.playing = true;
        drive.weight = 0.5;
        assert_eq!(p1_turn_evidence_mask(&drive, None), 1);
        drive.lr = 1.0;
        assert_eq!(p1_turn_evidence_mask(&drive, None), 2);
    }

    #[test]
    fn foot_avatar_replacement_watchdog_ends_only_after_no_progress_limit() {
        use super::{
            FOOT_AVATAR_REPLACEMENT_NO_PROGRESS_LIMIT_SECONDS,
            FOOT_AVATAR_REPLACEMENT_START_SECONDS, foot_avatar_replacement_timed_out,
        };
        let deadline = FOOT_AVATAR_REPLACEMENT_START_SECONDS
            + FOOT_AVATAR_REPLACEMENT_NO_PROGRESS_LIMIT_SECONDS;
        assert!(!foot_avatar_replacement_timed_out(deadline - 0.01, false));
        assert!(foot_avatar_replacement_timed_out(deadline, false));
        assert!(!foot_avatar_replacement_timed_out(deadline, true));
    }

    #[test]
    fn scaled_crouch_clearance_preserves_floor_and_canopy_gap() {
        use super::stance_center_from_standing_center;
        use bevy_als::als::{params::AlsMovementParams, state::Stance};
        use bevy::prelude::Vec3;
        let params = AlsMovementParams::default();
        let standing_half = params.crouch.standing_capsule_half_height;
        let crouched_half = params.crouch.crouched_capsule_half_height;
        let standing_center = Vec3::Y * (standing_half * 2.0 + 0.02);
        let crouched_center =
            stance_center_from_standing_center(&params, standing_center, Stance::Crouching, 2.0);
        let floor = crouched_center.y - crouched_half * 2.0;
        let canopy_bottom = floor + crouched_half * 4.0 + 0.08;
        assert!((floor - 0.02).abs() < 1e-6);
        assert!((canopy_bottom - (crouched_center.y + crouched_half * 2.0) - 0.08).abs() < 1e-6);
        assert!(canopy_bottom < standing_center.y + standing_half * 2.0);
    }

    #[test]
    fn camera_movement_probe_rejects_control_and_current_frame_substitution() {
        use super::CameraMovementProbe;
        use bevy::prelude::{Quat, Vec2, Vec3};
        for wrong in [None, Some(0.6_f32), Some(1.2)] {
            let mut probe = CameraMovementProbe::default();
            probe.observe(Some(0.4), 1.2, Vec2::ZERO, None, None);
            for _ in 0..8 {
                let desired = Quat::from_rotation_y(wrong.unwrap_or(0.4)) * Vec3::NEG_Z * 0.3;
                probe.observe(
                    Some(0.4),
                    1.2,
                    Vec2::Y * 0.3,
                    Some(wrong.unwrap_or(0.4)),
                    Some(desired),
                );
            }
            assert_eq!(probe.passed(), wrong.is_none());
        }
        let mut missing = CameraMovementProbe::default();
        missing.observe(Some(0.4), 1.2, Vec2::ZERO, None, None);
        missing.observe(Some(0.5), 1.2, Vec2::Y, None, None);
        assert!(missing.invalid && !missing.passed());
    }

    #[test]
    fn foot_ik_terrain_emits_one_press_per_action_at_variable_frame_rates() {
        for deltas in [&[1.0_f32 / 60.0][..], &[0.011, 0.017, 0.008][..]] {
            let mut elapsed = 0.0;
            let mut frame = 0;
            let mut presses = [0; 3];
            let mut crouching = false;
            while elapsed < 10.0 {
                let delta = deltas[frame % deltas.len()];
                elapsed += delta;
                let command = super::foot_ik_terrain_command(elapsed, delta);
                if command.crouch_pressed {
                    presses[0] += 1;
                    crouching = !crouching;
                }
                if command.jump_pressed {
                    presses[1] += 1;
                    assert!(!crouching, "The jump input only stands up from crouch");
                }
                if command.roll_pressed {
                    presses[2] += 1;
                }
                frame += 1;
            }
            assert_eq!(presses, [2, 1, 1]);
        }
    }

    use super::*;

    #[test]
    fn control_rig_action_evidence_requires_actual_hand_weights_and_spine_motion() {
        use bevy_als::extensions::{
            control_rig_program::RigDefinition,
            root_graph_boundaries::{
                ControlRigBoundaryNode, RIG_CAPTURE_KEY, RIG_GAME_WORLD_KEY, RigInputBinding,
            },
        };
        use bevy::{
            ecs::system::RunSystemOnce,
            world_serialization::{WorldAsset, WorldInstanceReady, WorldInstanceSpawner},
        };
        use bevy_animation_graph::{
            AnimationGraphPlugin,
            core::{
                animated_scene::AnimatedSceneHandle, animation_clip::EntityPath,
                animation_node::AnimationNode, edge_data::DataSpec, pose::BonePose,
                systems::animation_player,
            },
        };
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            bevy::mesh::MeshPlugin,
            TransformPlugin,
            bevy::gizmos::GizmoPlugin,
            AnimationGraphPlugin::default(),
            bevy_als::extensions::AlsExtPlugin,
        ))
        .init_asset::<WorldAsset>();
        app.finish();
        app.cleanup();
        let definition: RigDefinition = serde_json::from_str(include_str!(
            "../fixtures/rig/cr_als_program.json"
        ))
        .unwrap();
        let local = |value: bevy_als::extensions::control_rig_hierarchy::RigTransform| Transform {
            translation: value.translation.as_vec3(),
            rotation: value.rotation.as_quat(),
            scale: value.scale.as_vec3(),
        };
        let mut skeleton = Skeleton::default();
        let mut paths: Vec<Vec<String>> = Vec::new();
        for bone in &definition.bones {
            let mut path = bone
                .parent
                .map(|index| paths[index].clone())
                .unwrap_or_default();
            path.push(bone.name.clone());
            skeleton.add_bone(
                EntityPath::from(path.clone()),
                local(bone.initial_local),
                local(bone.initial_global),
            );
            paths.push(path);
        }
        skeleton.set_root(EntityPath::from(paths[0].clone()).id());
        let skeleton = app
            .world_mut()
            .resource_mut::<Assets<Skeleton>>()
            .add(skeleton);
        let mut pose = Pose {
            skeleton: skeleton.clone(),
            ..default()
        };
        for (bone, path) in definition.bones.iter().zip(paths) {
            pose.add_bone(
                BonePose::from_transform(local(bone.initial_local)),
                EntityPath::from(path).id(),
            );
        }
        let mut graph = AnimationGraph::new();
        let source = AnimationNode::new(
            "source",
            bevy_als::extensions::GlobalPoseInputNode {
                key: "source".into(),
                output: "Pose".into(),
            },
        );
        let curves = AnimationNode::new(
            "source curves",
            bevy_als::extensions::source_pose_nodes::SourceCurveModifyNode {
                source_pose_pin: "Source".into(),
                curve_names: vec!["LayerArmLeft".into(), "LayerArmRight".into()],
                pin_defaults: vec![1.0, 1.0],
                alpha: 1.0,
                ..default()
            },
        );
        let rig = AnimationNode::new(
            "rig",
            ControlRigBoundaryNode {
                definition: Some(definition),
                capture_context: true,
                internalize_time: true,
                input_bindings: [0, 1, 2, 5]
                    .map(|index| RigInputBinding {
                        external: "RigInput".into(),
                        path: vec![index],
                        property: format!("input_{index}"),
                    })
                    .into(),
                ..default()
            },
        );
        for node in [&source, &curves, &rig] {
            graph.add_node(node.clone());
        }
        graph.add_node_parameter_edge(source.id, "Pose", curves.id, "Source");
        graph.add_node_parameter_edge(curves.id, "Pose", rig.id, "source_pose");
        graph.add_output_data("pose".into(), DataSpec::Pose);
        graph.add_output_data_edge(rig.id, "Pose", "pose");
        let graph = app
            .world_mut()
            .resource_mut::<Assets<AnimationGraph>>()
            .add(graph);
        let mut player = AnimationGraphPlayer::new(skeleton).with_graph(graph.clone());
        player.set_global_input_data("source", DataValue::Pose(pose));
        player.set_global_input_data("animation.delta_seconds", 1.0_f32 / 60.0);
        player.set_global_input_data(RIG_CAPTURE_KEY, true);
        player.set_global_input_data(RIG_GAME_WORLD_KEY, true);
        for key in ["input_0", "input_1", "input_2"] {
            player.set_global_input_data(key, true);
        }
        player.set_global_input_data("input_5", 0.0_f32);
        let actor = app
            .world_mut()
            .spawn((AlsCharacter, Transform::IDENTITY))
            .id();
        let avatar = app
            .world_mut()
            .spawn((
                AlsCharacterAvatar { character: actor },
                Transform::IDENTITY,
                ChildOf(actor),
                AnimatedSceneHandle::default(),
            ))
            .id();
        let player_entity = app
            .world_mut()
            .spawn((
                player,
                Name::new("root"),
                Transform::IDENTITY,
                ChildOf(avatar),
            ))
            .id();
        app.world_mut().spawn((
            Name::new("pelvis"),
            Transform::IDENTITY,
            ChildOf(player_entity),
        ));
        app.world_mut().trigger(WorldInstanceReady {
            entity: avatar,
            instance_id: WorldInstanceSpawner::default().spawn(Handle::<WorldAsset>::default()),
        });
        app.world_mut().flush();
        app.world_mut().run_system_once(animation_player).unwrap();
        for (active, yaw, expected) in [
            (false, 30.0_f32, false),
            (true, 0.0, false),
            (true, 30.0, true),
            (false, 30.0, false),
        ] {
            run_manual_rig_curve_fixture(app.world_mut(), actor, default(), active);
            app.world_mut()
                .get_mut::<AnimationGraphPlayer>(player_entity)
                .unwrap()
                .set_global_input_data("input_5", yaw);
            app.world_mut().run_system_once(animation_player).unwrap();
            let player = app
                .world()
                .get::<AnimationGraphPlayer>(player_entity)
                .unwrap();
            assert!(player.get_error().is_none(), "{:?}", player.get_error());
            let evidence = control_rig_action_evidence(
                player,
                app.world().resource::<Assets<AnimationGraph>>(),
                app.world().resource::<Assets<Skeleton>>(),
            );
            assert_eq!(
                evidence.is_some(),
                expected,
                "active={active} yaw={yaw} evidence={evidence:?}"
            );
            if let Some([left, right, angle]) = evidence {
                assert_eq!([left, right], [1.0, 1.0]);
                assert!(angle > 0.05);
            }
            if active && expected {
                app.world_mut()
                    .resource_mut::<ManualIntegrationState>()
                    .phase = IntegrationPhase::Finish;
                app.world_mut()
                    .run_system_cached(drive_manual_rig_curves)
                    .unwrap();
                app.world_mut().run_system_once(animation_player).unwrap();
                assert!(
                    control_rig_action_evidence(
                        app.world()
                            .get::<AnimationGraphPlayer>(player_entity)
                            .unwrap(),
                        app.world().resource::<Assets<AnimationGraph>>(),
                        app.world().resource::<Assets<Skeleton>>()
                    )
                    .is_some(),
                    "Finishing the scenario must preserve fixture inputs until pose collection ends"
                );
            }
        }
        app.world_mut()
            .resource_mut::<Assets<AnimationGraph>>()
            .get_mut(&graph)
            .unwrap()
            .add_output_data_edge(source.id, "Pose", "pose");
        app.world_mut().run_system_once(animation_player).unwrap();
        assert!(
            control_rig_action_evidence(
                app.world()
                    .get::<AnimationGraphPlayer>(player_entity)
                    .unwrap(),
                app.world().resource::<Assets<AnimationGraph>>(),
                app.world().resource::<Assets<Skeleton>>()
            )
            .is_none()
        );
    }

    #[test]
    fn control_rig_view_driver_runs_after_physics_before_actor_inputs() {
        use bevy_als::extensions::post_physics_animation::PostPhysicsAnimationSet;
        use bevy::ecs::schedule::ScheduleCleanupPolicy;
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            bevy::asset::AssetPlugin::default(),
            ManualPlaygroundIntegrationPlugin,
        ))
        .init_asset::<StateMachine>()
        .add_message::<AlsCameraShakeRequest>()
        .add_message::<AlsAnimationEvent>()
        .init_resource::<Assets<AnimationGraph>>()
        .init_resource::<AlsFootIkWeightOverride>()
        .configure_sets(
            PostUpdate,
            (
                PostPhysicsAnimationSet::Inputs.after(avian3d::prelude::PhysicsSystems::Last),
                PostPhysicsAnimationSet::Evaluate.after(PostPhysicsAnimationSet::Inputs),
                bevy_animation_graph::core::plugin::AnimationGraphSet::Final
                    .after(PostPhysicsAnimationSet::Evaluate),
            ),
        );
        app.world_mut()
            .schedule_scope(PostUpdate, |world, schedule| {
                assert_eq!(
                    schedule
                        .remove_systems_in_set(
                            prepare_get_up_physics_fixture,
                            world,
                            ScheduleCleanupPolicy::RemoveSystemsOnly
                        )
                        .unwrap(),
                    1
                );
                assert_eq!(
                    schedule
                        .remove_systems_in_set(
                            observe_manual_integration,
                            world,
                            ScheduleCleanupPolicy::RemoveSystemsOnly
                        )
                        .unwrap(),
                    1
                );
            });
        let actor = app
            .world_mut()
            .spawn((AlsCharacter, Transform::IDENTITY, AlsViewYaw(0.0)))
            .id();
        {
            let mut state = app.world_mut().resource_mut::<ManualIntegrationState>();
            state.character = Some(actor);
            state.scenario = IntegrationScenario::ControlRig;
            state.phase = IntegrationPhase::ControlRig;
        }
        app.add_systems(
            PostUpdate,
            (|mut actors: Query<&mut AlsViewYaw>| {
                for mut yaw in &mut actors {
                    yaw.0 = -0.8;
                }
            })
            .in_set(avian3d::prelude::PhysicsSystems::Last),
        );
        let mut frame = 0;
        app.add_systems(
            PostUpdate,
            (move |actors: Query<&AlsViewYaw>| {
                let expected = [60.0_f32, 150.0, -30.0, 60.0][frame].to_radians();
                assert!(
                    (actors.get(actor).unwrap().0 - expected).abs() < 1e-6,
                    "view input must include the current fixture offset"
                );
                frame += 1;
            })
            .in_set(PostPhysicsAnimationSet::Inputs),
        );
        for (index, rotation) in [
            Quat::IDENTITY,
            Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
            Quat::from_rotation_y(-std::f32::consts::FRAC_PI_2),
            Quat::IDENTITY,
        ]
        .into_iter()
        .enumerate()
        {
            if index == 3 {
                app.world_mut()
                    .resource_mut::<ManualIntegrationState>()
                    .phase = IntegrationPhase::Finish;
            }
            app.world_mut()
                .get_mut::<Transform>(actor)
                .unwrap()
                .rotation = rotation;
            app.world_mut().run_schedule(PostUpdate);
        }
    }

    #[test]
    fn head_driver_changes_saved_rotation_intent_through_actual_input() {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            bevy_als::als::transition::AlsStateTransitionPlugin,
        ))
        .add_message::<bevy_als::als::aim::AimStateChanged>()
        .add_message::<AlsCameraShakeRequest>()
        .add_message::<AlsPivotNotify>()
        .init_resource::<ManualIntegrationState>()
        .init_resource::<ManualAnimationWindowEvidence>()
        .init_resource::<ManualRuntimePhaseEvidence>()
        .init_resource::<AlsShowDebugHudState>()
        .init_resource::<AlsFootIkWeightOverride>()
        .init_resource::<Assets<AlsMovementParams>>()
        .init_resource::<ButtonInput<KeyCode>>()
        .insert_resource(ManualIntegrationFixture {
            platform: Entity::PLACEHOLDER,
            platform_home: Vec3::ZERO,
            locomotion_start: Vec3::ZERO,
            mantle_start: Vec3::ZERO,
            roll_start: Vec3::ZERO,
            facing_yaw: 0.0,
            obstacle_mesh: Handle::default(),
            obstacle_material: Handle::default(),
        })
        .add_systems(PreUpdate, drive_manual_integration)
        .add_systems(
            Update,
            bevy_als::als::input_request::input_state_request_system
                .before(bevy_als::als::transition::state_transition_system),
        )
        .add_systems(PostUpdate, bevy_als::als::aim::aim_rotation_mode_system);
        let entity = app
            .world_mut()
            .spawn((
                AlsCharacter,
                crate::input::AlsInputCommand::default(),
                AlsInputOverride::default(),
                AlsMaxAllowedGait(Gait::Walking),
                Stance::Standing,
                LocomotionState::Grounded,
                Gait::Walking,
                RotationMode::ViewDirection,
                OverlayState::Default,
                bevy_als::als::aim::AlsLastFreeRotationMode(RotationMode::ViewDirection),
                AlsStateRequest::default(),
                (
                    Transform::default(),
                    Position::default(),
                    LinearVelocity::default(),
                    AlsViewYaw::default(),
                    AlsViewPitch::default(),
                    AlsMovementParamsHandle::default(),
                    AlsAimingYawConstraint::default(),
                ),
            ))
            .id();
        for (index, expected) in [
            (2, RotationMode::Aiming),
            (3, RotationMode::VelocityDirection),
            (4, RotationMode::VelocityDirection),
            (5, RotationMode::ViewDirection),
            (6, RotationMode::ViewDirection),
        ] {
            let mut state = app.world_mut().resource_mut::<ManualIntegrationState>();
            state.initialized = true;
            state.character = Some(entity);
            state.scenario = IntegrationScenario::Head;
            state.phase = IntegrationPhase::Head;
            state.entered = true;
            state.head_probe.case_index = index;
            state.phase_started_seconds = 0.0;
            app.update();
            assert_eq!(
                *app.world().get::<RotationMode>(entity).unwrap(),
                expected,
                "Source Head branch after owner refresh for case {index}"
            );
            assert_eq!(
                app.world()
                    .get::<bevy_als::als::aim::AlsLastFreeRotationMode>(entity)
                    .unwrap()
                    .0,
                expected,
                "Source Head branch must update saved intent for case {index}"
            );
            app.update();
            assert_eq!(
                *app.world().get::<RotationMode>(entity).unwrap(),
                expected,
                "Source Head branch must remain stable for case {index}"
            );
        }
    }

    #[test]
    fn head_observer_rejects_empty_samples_and_binds_actual_callback_pins() {
        use bevy_animation_graph::core::pose::{BoneId, BonePose};
        let bone = BoneId::default();
        let poses = [0.1, -0.6, 0.8].map(|angle| {
            let mut pose = Pose::default();
            pose.add_bone(
                BonePose {
                    rotation: Some(Quat::from_rotation_x(angle)),
                    ..default()
                },
                bone,
            );
            Some(pose)
        });
        for (pitch, expected) in [
            (-120.0, [0.0, 1.0, 0.0]),
            (-45.0, [0.5, 0.5, 0.0]),
            (0.0, [1.0, 0.0, 0.0]),
            (45.0, [0.5, 0.0, 0.5]),
            (120.0, [0.0, 0.0, 1.0]),
        ] {
            let weights = head_look_weights(pitch);
            assert_eq!(weights, expected);
            assert!(head_weighted_pose(poses.clone(), weights).is_some());
            assert!(head_weighted_pose([None, None, None], weights).is_none());
            assert!(
                head_weighted_pose(
                    [
                        Some(Pose::default()),
                        Some(Pose::default()),
                        Some(Pose::default())
                    ],
                    weights
                )
                .is_none()
            );
        }
        let down = head_weighted_pose(poses.clone(), head_look_weights(-45.0)).unwrap();
        let up = head_weighted_pose(poses, head_look_weights(45.0)).unwrap();
        assert!(default_overlay_pose_error(&down, &up).unwrap() > 0.1);
        let graph = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("assets")
                .join(HEAD_GRAPH),
        )
        .unwrap();
        for (_, node, _) in HEAD_BOUNDARIES {
            assert!(
                graph.contains(&format!("name: {node:?}")),
                "Missing authored Head node {node:?}"
            );
        }
        assert_eq!(HEAD_BOUNDARIES[0].2, "pose");
        for index in 0..HEAD_WINDOWS.len() {
            let window = head_window(index, &HeadGraphSample::default());
            assert!(window.capture_every_frame && window.requires_source_pose_chain);
            assert!(
                window
                    .source_pose_chain
                    .iter()
                    .all(|boundary| !boundary.node.starts_with("additive reference"))
            );
            if index == 4 {
                assert_eq!(window.source_pose_chain.len(), 2);
            }
        }
        assert!(!default_overlay_should_timeout(false, 11.99));
        assert!(default_overlay_should_timeout(false, 12.0));
    }

    #[test]
    fn head_world_guard_rejects_local_or_missing_actor_inputs() {
        let world = GlobalTransform::IDENTITY;
        let local = Transform::from_rotation(Quat::from_rotation_y(-std::f32::consts::FRAC_PI_2));
        let mut player = AnimationGraphPlayer::new(Handle::default());
        assert!(!head_actor_world_inputs_match(&player, &local, &world));
        player.set_global_input_data("locomotion_state.rotation_yaw", 0.0_f32);
        player.set_global_input_data(
            "locomotion_state.rotation_quaternion_world_space",
            Quat::IDENTITY,
        );
        assert!(head_actor_world_inputs_match(&player, &local, &world));
        assert!(!head_actor_world_inputs_match(
            &player,
            &Transform::IDENTITY,
            &world
        ));
        player.set_global_input_data("locomotion_state.rotation_yaw", 90.0_f32);
        assert!(!head_actor_world_inputs_match(&player, &local, &world));
        player.set_global_input_data("locomotion_state.rotation_yaw", 0.0_f32);
        player.set_global_input_data(
            "locomotion_state.rotation_quaternion_world_space",
            local.rotation,
        );
        assert!(!head_actor_world_inputs_match(&player, &local, &world));
    }

    #[test]
    fn head_world_guard_accepts_identical_affine_rotations() {
        let parent = Transform::from_rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2))
            .with_scale(Vec3::splat(1.5));
        let mut player = AnimationGraphPlayer::new(Handle::default());
        for step in 0..3600 {
            let rotation = Quat::from_rotation_y((step as f32 * 0.1).to_radians());
            let local = Transform::from_rotation(parent.rotation.inverse() * rotation)
                .with_scale(Vec3::splat(1.0 / 1.5));
            let world = parent * GlobalTransform::from(local);
            let rotation = world.rotation();
            player.set_global_input_data(
                "locomotion_state.rotation_quaternion_world_space",
                rotation,
            );
            player.set_global_input_data(
                "locomotion_state.rotation_yaw",
                -bevy_als::als::orientation::world_forward_to_yaw(*world.forward()).to_degrees(),
            );
            assert!(
                head_actor_world_inputs_match(&player, &local, &world),
                "World guard rejected identical affine rotation at {step}: {rotation:?}, norm={}",
                rotation.length_squared()
            );
            player.set_global_input_data(
                "locomotion_state.rotation_quaternion_world_space",
                -rotation,
            );
            assert!(
                head_actor_world_inputs_match(&player, &local, &world),
                "Equivalent quaternion sign was rejected at {step}"
            );
        }
    }

    #[test]
    fn head_observer_reads_loaded_graph_samples_and_relevance() {
        use bevy::ecs::system::RunSystemOnce;
        use bevy_animation_graph::{
            builtin_nodes::clip_node::ClipNode,
            core::{
                animation_clip::{GraphClip, loader::GraphClipLoader},
                animation_graph::{GraphInputPin, loader::AnimationGraphLoader},
                animation_node::AnimationNode,
                edge_data::DataSpec,
                skeleton::loader::SkeletonLoader,
                systems::animation_player,
            },
        };
        fn observe(
            In((entity, previous)): In<(Entity, Option<(u64, AlsHeadState, bool, f32)>)>,
            players: Query<&AnimationGraphPlayer>,
            probe: MotionProbe,
        ) -> HeadGraphSample {
            head_graph_sample(players.get(entity).unwrap(), &probe, previous.as_ref())
        }
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            bevy::gltf::GltfPlugin::default(),
        ))
        .init_asset::<bevy::world_serialization::WorldAsset>()
        .init_asset::<bevy::animation::AnimationClip>()
        .init_asset::<bevy::mesh::skinning::SkinnedMeshInverseBindposes>()
        .init_asset::<Mesh>()
        .init_asset::<Image>()
        .init_asset::<GraphClip>()
        .init_asset_loader::<GraphClipLoader>()
        .init_asset::<Skeleton>()
        .init_asset_loader::<SkeletonLoader>()
        .init_asset::<AnimationGraph>()
        .init_asset_loader::<AnimationGraphLoader>()
        .init_asset::<StateMachine>()
        .init_resource::<ButtonInput<KeyCode>>()
        .add_plugins(bevy_als::extensions::AlsExtPlugin)
        .register_type::<ClipNode>();
        app.finish();
        app.cleanup();
        let server = app.world().resource::<AssetServer>();
        let graph: Handle<AnimationGraph> = server.load(HEAD_GRAPH);
        let base: Handle<GraphClip> =
            server.load("ALS/Animations/Overlays/Other/A_Als_Default_Poses.anim.ron");
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut previous_loaded = 0;
        loop {
            app.update();
            let server = app.world().resource::<AssetServer>();
            let loaded = usize::from(server.is_loaded_with_dependencies(graph.id()))
                + usize::from(server.is_loaded_with_dependencies(base.id()));
            if loaded != previous_loaded {
                eprintln!("Head observer assets loaded {loaded}/2");
                previous_loaded = loaded;
            }
            if loaded == 2 {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "Head observer assets did not load within 30 seconds: {loaded}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        let linked = AnimationNode::new(
            "Head",
            bevy_als::extensions::LayeredGraphNode {
                capture_context: true,
                internalize_time: true,
                output_aliases: vec![bevy_als::extensions::layered_graph::OutputDataAlias {
                    child_output: "Result".into(),
                    outer_output: "Pose".into(),
                }],
                ..bevy_als::extensions::LayeredGraphNode::new(graph)
            },
        );
        let mut root = AnimationGraph::new();
        root.add_input_data(
            GraphInputPin::Passthrough("View Input".into()),
            DataSpec::Pose,
        );
        root.add_output_data("pose".into(), DataSpec::Pose);
        root.add_edge(
            SourcePin::InputData(GraphInputPin::Passthrough("View Input".into())),
            TargetPin::NodeData(linked.id, "View Input".into()),
        );
        root.add_edge(
            SourcePin::NodeData(linked.id, "Pose".into()),
            TargetPin::OutputData("pose".into()),
        );
        root.add_node(linked);
        let root = app
            .world_mut()
            .resource_mut::<Assets<AnimationGraph>>()
            .add(root);
        let clip = app
            .world()
            .resource::<Assets<GraphClip>>()
            .get(&base)
            .unwrap();
        let mut player = AnimationGraphPlayer::new(clip.skeleton.clone()).with_graph(root);
        player.set_input_data(
            "View Input",
            bevy_als::extensions::source_sequence_player::sample_pose(clip, 0.0).into(),
        );
        player.set_global_input_data("animation.delta_seconds", 0.1_f32);
        player.set_global_input_data("animation.delta_seconds_without_time_dilation", 0.1_f32);
        let entity = app
            .world_mut()
            .spawn((
                player,
                Name::new("Head observer"),
                Transform::default(),
                GlobalTransform::default(),
            ))
            .id();
        app.world_mut().spawn((Name::new("root"), ChildOf(entity)));
        app.init_resource::<Assets<StandardMaterial>>();
        let mut previous: Option<(u64, AlsHeadState, bool, f32)> = None;
        for (pitch, yaw, alpha) in [
            (-60.0_f32, 40.0_f32, 1.0_f32),
            (60.0, -80.0, 1.0),
            (60.0, -80.0, 0.0),
            (45.0, 120.0, 1.0),
        ] {
            let mut player = app
                .world_mut()
                .get_mut::<AnimationGraphPlayer>(entity)
                .unwrap();
            player.set_global_input_data("view_state.pitch_angle", pitch);
            player.set_global_input_data("view_state.yaw_angle", yaw);
            player.set_global_input_data("view_state.head_blend_amount", alpha);
            if let Some(previous) = previous.as_mut() {
                previous.3 = alpha;
            }
            app.world_mut().run_system_once(animation_player).unwrap();
            assert!(
                app.world()
                    .get::<AnimationGraphPlayer>(entity)
                    .unwrap()
                    .get_error()
                    .is_none(),
                "Loaded Head graph failed: {:?}",
                app.world()
                    .get::<AnimationGraphPlayer>(entity)
                    .unwrap()
                    .get_error()
            );
            let sample = app
                .world_mut()
                .run_system_once_with(observe, (entity, previous))
                .unwrap();
            assert!(sample.errors.is_empty(), "Head observer failed: {sample:?}");
            assert_eq!(
                sample.ready,
                alpha > 1e-5,
                "Head observer did not see authored outputs: {sample:?}"
            );
            assert_eq!(sample.refreshed, alpha > 1e-5);
            if alpha > 1e-5 {
                assert!(
                    sample.additive_effect.is_some_and(|effect| effect > 1e-4),
                    "Head additive effect was not observed: {sample:?}"
                );
            }
            let state = bevy_als::head_layer::head_state_from_player(
                app.world().get::<AnimationGraphPlayer>(entity).unwrap(),
            );
            previous = Some((sample.frame, state, sample.refreshed, alpha));
        }
    }

    #[test]
    fn layering_observer_reads_loaded_layered_blend_curve_context() {
        use bevy::ecs::system::RunSystemOnce;
        use bevy_animation_graph::{
            builtin_nodes::clip_node::ClipNode,
            core::{
                animation_clip::{GraphClip, loader::GraphClipLoader},
                animation_graph::{GraphInputPin, loader::AnimationGraphLoader},
                animation_node::AnimationNode,
                edge_data::DataSpec,
                skeleton::loader::SkeletonLoader,
                systems::animation_player,
            },
        };
        fn observe(
            In((entity, previous)): In<(Entity, Option<(u64, AlsLayeringState)>)>,
            players: Query<&AnimationGraphPlayer>,
            probe: MotionProbe,
        ) -> LayeringGraphSample {
            layering_graph_sample(players.get(entity).unwrap(), &probe, previous.as_ref())
        }
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            bevy::gltf::GltfPlugin::default(),
        ))
        .init_asset::<bevy::world_serialization::WorldAsset>()
        .init_asset::<bevy::animation::AnimationClip>()
        .init_asset::<bevy::mesh::skinning::SkinnedMeshInverseBindposes>()
        .init_asset::<Mesh>()
        .init_asset::<Image>()
        .init_asset::<GraphClip>()
        .init_asset_loader::<GraphClipLoader>()
        .init_asset::<Skeleton>()
        .init_asset_loader::<SkeletonLoader>()
        .init_asset::<AnimationGraph>()
        .init_asset_loader::<AnimationGraphLoader>()
        .init_asset::<StateMachine>()
        .init_resource::<ButtonInput<KeyCode>>()
        .add_plugins(bevy_als::extensions::AlsExtPlugin)
        .register_type::<ClipNode>();
        app.finish();
        app.cleanup();
        let server = app.world().resource::<AssetServer>();
        let graph: Handle<AnimationGraph> = server.load(LAYERING_GRAPH);
        let base: Handle<GraphClip> =
            server.load("ALS/Animations/Overlays/Other/A_Als_Default_Poses.anim.ron");
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut previous_loaded = 0;
        loop {
            app.update();
            let server = app.world().resource::<AssetServer>();
            let loaded = usize::from(server.is_loaded_with_dependencies(graph.id()))
                + usize::from(server.is_loaded_with_dependencies(base.id()));
            if loaded != previous_loaded {
                eprintln!("Layering observer assets loaded {loaded}/2");
                previous_loaded = loaded;
            }
            if loaded == 2 {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "Layering observer assets did not load within 30 seconds: {loaded}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        let linked = AnimationNode::new(
            "Layering",
            bevy_als::extensions::LayeredGraphNode {
                capture_context: true,
                internalize_time: true,
                output_aliases: vec![bevy_als::extensions::layered_graph::OutputDataAlias {
                    child_output: "Result".into(),
                    outer_output: "Pose".into(),
                }],
                ..bevy_als::extensions::LayeredGraphNode::new(graph)
            },
        );
        let mut root = AnimationGraph::new();
        for name in ["Locomotion Input", "Overlay Input"] {
            root.add_input_data(GraphInputPin::Passthrough(name.into()), DataSpec::Pose);
            root.add_edge(
                SourcePin::InputData(GraphInputPin::Passthrough(name.into())),
                TargetPin::NodeData(linked.id, name.into()),
            );
        }
        root.add_output_data("pose".into(), DataSpec::Pose);
        root.add_edge(
            SourcePin::NodeData(linked.id, "Pose".into()),
            TargetPin::OutputData("pose".into()),
        );
        root.add_node(linked);
        let root = app
            .world_mut()
            .resource_mut::<Assets<AnimationGraph>>()
            .add(root);
        let clip = app
            .world()
            .resource::<Assets<GraphClip>>()
            .get(&base)
            .unwrap();
        let mut player = AnimationGraphPlayer::new(clip.skeleton.clone()).with_graph(root);
        for name in ["Locomotion Input", "Overlay Input"] {
            player.set_input_data(
                name,
                bevy_als::extensions::source_sequence_player::sample_pose(clip, 0.0).into(),
            );
        }
        player.set_global_input_data("animation.delta_seconds", 1.0_f32 / 60.0);
        player.set_global_input_data(
            bevy_als::extensions::global_f32_input::ROOT_DEFAULTS_ENABLED_KEY,
            true,
        );
        for (key, value) in
            bevy_als::layering_layer::layering_state_inputs(AlsLayeringState::default())
        {
            player.set_global_input_data(key, value);
        }
        let entity = app
            .world_mut()
            .spawn((
                player,
                Name::new("Layering observer"),
                Transform::default(),
                GlobalTransform::default(),
            ))
            .id();
        app.world_mut().spawn((Name::new("root"), ChildOf(entity)));
        app.init_resource::<Assets<StandardMaterial>>();
        let mut previous = None;
        for _ in 0..2 {
            app.world_mut().run_system_once(animation_player).unwrap();
            assert!(
                app.world()
                    .get::<AnimationGraphPlayer>(entity)
                    .unwrap()
                    .get_error()
                    .is_none()
            );
            let sample = app
                .world_mut()
                .run_system_once_with(observe, (entity, previous))
                .unwrap();
            assert!(
                sample.errors.is_empty(),
                "Layering observer failed: {sample:?}"
            );
            assert_eq!(sample.curve_operators, 3);
            assert_eq!(sample.input_curve_frame, previous.map(|(frame, _)| frame));
            previous = Some((
                sample.evaluation_frame.unwrap(),
                AlsLayeringState::default(),
            ));
        }
    }

    #[test]
    fn default_overlay_observers_compare_the_loaded_animation_poses() {
        use bevy::ecs::system::RunSystemOnce;

        fn observe(
            In(entity): In<Entity>,
            players: Query<&AnimationGraphPlayer>,
            probe: MotionProbe,
        ) -> (DefaultOverlayGraphSample, LayeringGraphSample) {
            let player = players.get(entity).unwrap();
            (
                default_overlay_graph_sample(player, &probe, None),
                layering_graph_sample(player, &probe, None),
            )
        }

        let (mut app, _, avatar) = crate::test_support::loaded_character();
        app.init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<Assets<StandardMaterial>>();
        let player = app
            .world()
            .get::<AnimatedSceneInstance>(avatar)
            .unwrap()
            .player_entity();
        for _ in 0..3 {
            app.update();
        }
        let (overlay, layering) = app
            .world_mut()
            .run_system_once_with(observe, player)
            .unwrap();
        assert!(
            overlay.ready && layering.base_evaluators > 0,
            "{overlay:?} {layering:?}"
        );
        assert!(
            !overlay.errors.iter().any(|error| {
                error.contains("pose mismatch") || error.starts_with("idle additive sequence:")
            }) && layering.errors.is_empty(),
            "{overlay:?} {layering:?}"
        );
        assert!(layering.boundaries.contains(&"layering_locomotion"));
        for (index, locomotion_moves) in [(0, false), (1, true)] {
            let window = default_overlay_window(index, &overlay);
            assert_eq!(
                window
                    .source_pose_chain
                    .iter()
                    .any(|boundary| boundary.label == "layering_locomotion"),
                locomotion_moves
            );
        }
    }

    #[test]
    fn default_overlay_additive_matches_loaded_source_pose_channels() {
        use bevy_animation_graph::core::{
            animation_clip::{GraphClip, loader::GraphClipLoader},
            pose::BonePose,
            skeleton::{Skeleton, loader::SkeletonLoader},
        };
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            bevy::gltf::GltfPlugin::default(),
        ))
        .init_asset::<bevy::world_serialization::WorldAsset>()
        .init_asset::<bevy::animation::AnimationClip>()
        .init_asset::<bevy::mesh::skinning::SkinnedMeshInverseBindposes>()
        .init_asset::<Mesh>()
        .init_asset::<Image>()
        .init_asset::<GraphClip>()
        .init_asset_loader::<GraphClipLoader>()
        .init_asset::<Skeleton>()
        .init_asset_loader::<SkeletonLoader>()
        .add_plugins(bevy_als::extensions::AlsExtPlugin);
        app.finish();
        app.cleanup();
        let server = app.world().resource::<AssetServer>();
        let handles: [Handle<GraphClip>; 3] = [
            "ALS/Animations/Overlays/Other/A_Als_Default_Poses.anim.ron",
            "ALS/Animations/Base/A_Als_Idle.anim.ron",
            "ALS/Animations/Base/A_Als_Stand_Pose.anim.ron",
        ]
        .map(|path| server.load(path));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let mut previous = 0;
        loop {
            app.update();
            let clips = app.world().resource::<Assets<GraphClip>>();
            let skeletons = app.world().resource::<Assets<Skeleton>>();
            let loaded = handles
                .iter()
                .filter(|handle| {
                    clips
                        .get(*handle)
                        .is_some_and(|clip| skeletons.get(&clip.skeleton).is_some())
                })
                .count();
            if loaded != previous {
                eprintln!("Default additive assets loaded {loaded}/3");
                previous = loaded;
            }
            if loaded == handles.len() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "Default additive assets did not load within 30 seconds: {loaded}"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let clips = app.world().resource::<Assets<GraphClip>>();
        let skeletons = app.world().resource::<Assets<Skeleton>>();
        let base_clip = clips.get(&handles[0]).unwrap();
        let idle_clip = clips.get(&handles[1]).unwrap();
        let reference = bevy_als::extensions::source_sequence_player::sample_pose(
            clips.get(&handles[2]).unwrap(),
            0.0,
        );
        let skeleton = skeletons.get(&reference.skeleton).unwrap();
        for base_time in [0.0, base_clip.duration * 0.5] {
            for idle_time in [0.0, 0.8, 1.6] {
                let base =
                    bevy_als::extensions::source_sequence_player::sample_pose(base_clip, base_time);
                let delta = bevy_als::extensions::reference_local_difference::make_local_difference(
                    &reference,
                    &bevy_als::extensions::source_sequence_player::sample_pose(idle_clip, idle_time),
                    skeleton,
                );
                let expected = default_overlay_additive_pose(&base, &delta, 0.75);
                let mut actual = base;
                bevy_als::extensions::mesh_space_additive::apply_local_space_additive(
                    &mut actual,
                    &delta,
                    0.75,
                    skeleton,
                );
                assert!(
                    default_overlay_pose_error(&actual, &expected)
                        .is_some_and(|error| error <= 2e-5),
                    "Default additive channels differ at base={base_time}, idle={idle_time}: actual={}, expected={}, difference={:?}",
                    actual.paths.len(),
                    expected.paths.len(),
                    default_overlay_pose_error(&actual, &expected)
                );
            }
        }
        let bone = skeleton
            .iter_bones()
            .find(|bone| *bone != skeleton.root())
            .unwrap();
        let mut reference = Pose {
            skeleton: base_clip.skeleton.clone(),
            ..default()
        };
        reference.add_bone(
            BonePose::from_transform(skeleton.default_transforms(bone).unwrap().local),
            bone,
        );
        let mut delta = Pose {
            skeleton: base_clip.skeleton.clone(),
            ..default()
        };
        delta.add_bone(
            BonePose {
                translation: Some(Vec3::Y * 0.2),
                rotation: Some(Quat::from_rotation_x(0.4)),
                scale: Some(Vec3::splat(0.1)),
                ..default()
            },
            bone,
        );
        let expected = default_overlay_additive_pose(&reference, &delta, 0.75);
        let mut sparse = Pose {
            skeleton: base_clip.skeleton.clone(),
            ..default()
        };
        bevy_als::extensions::mesh_space_additive::apply_local_space_additive(
            &mut sparse,
            &delta,
            0.75,
            skeleton,
        );
        assert_eq!(sparse.paths.len(), 1);
        assert!(default_overlay_pose_error(&sparse, &expected).is_some_and(|error| error <= 2e-5));
    }

    #[test]
    fn default_overlay_pose_check_rejects_lost_frames_and_additive_weight() {
        use bevy_animation_graph::core::{
            edge_data::bone_mask::BoneMask, interpolation::additive::AdditiveInterpolator,
            pose::BonePose,
        };
        let mut base = Pose::default();
        base.add_bone(
            BonePose {
                translation: Some(Vec3::X),
                rotation: Some(Quat::from_rotation_y(0.4)),
                scale: Some(Vec3::ONE),
                ..default()
            },
            default(),
        );
        let mut additive = Pose::default();
        additive.add_bone(
            BonePose {
                translation: Some(Vec3::Y * 0.2),
                rotation: Some(Quat::from_rotation_x(0.2)),
                scale: Some(Vec3::ZERO),
                ..default()
            },
            default(),
        );
        let mut expected = base.clone();
        AdditiveInterpolator {
            bone_mask: BoneMask::all(),
        }
        .interpolate_pose(&mut expected, &additive, 0.75);
        assert_eq!(default_overlay_pose_error(&expected, &expected), Some(0.0));
        let mut old_weight = base.clone();
        AdditiveInterpolator {
            bone_mask: BoneMask::all(),
        }
        .interpolate_pose(&mut old_weight, &additive, 1.0);
        assert!(default_overlay_pose_error(&old_weight, &expected).unwrap() > 0.01);
        let mut wrong_frame = expected.clone();
        wrong_frame.bones[0].rotation = Some(Quat::IDENTITY);
        assert!(default_overlay_pose_error(&wrong_frame, &expected).unwrap() > 0.01);
        let mut missing = expected.clone();
        missing.bones[0].rotation = None;
        assert_eq!(default_overlay_pose_error(&missing, &expected), None);
        missing.bones[0].rotation = Some(Quat::from_xyzw(f32::NAN, 0.0, 0.0, 1.0));
        assert_eq!(default_overlay_pose_error(&missing, &expected), None);
    }

    #[test]
    fn default_overlay_windows_bind_all_source_frames_and_bound_progress() {
        let mut probe = DefaultOverlayProbe {
            case_index: 3,
            airborne_seen: true,
            ..default()
        };
        for covered in [false, true] {
            probe.covered[3] = covered;
            assert_eq!(probe.air_window(Some(0.0)), Some(3));
            assert_eq!(probe.air_window(Some(0.5)), Some(4));
        }
        assert_eq!(probe.air_window(None), None);
        probe.case_index = 5;
        assert_eq!(probe.air_window(Some(0.0)), None);
        let graph = std::fs::read_to_string(format!("assets/{DEFAULT_OVERLAY_GRAPH}")).unwrap();
        for index in 0..6 {
            let window = default_overlay_window(
                index,
                &DefaultOverlayGraphSample {
                    idle_duration: Some(2.666667),
                    ..default()
                },
            );
            let identity = DEFAULT_OVERLAY_EVALUATORS[if index == 5 { 0 } else { index }];
            assert!(graph.contains(&format!("id: {identity:?}")));
            assert!(window.capture_every_frame && window.requires_source_pose_chain);
            assert_eq!(window.canonical_clip_time.is_some(), index < 3);
            assert_eq!(window.canonical_sync_phase, None);
        }
        assert!(!default_overlay_should_timeout(false, 11.99));
        assert!(default_overlay_should_timeout(false, 12.0));
        assert!(!default_overlay_should_timeout(true, 12.0));
        let mut mapping = bevy_als::extensions::source_pose_nodes::SourceAlphaMapping {
            interpolate_speed_increasing: 20.0,
            interpolate_speed_decreasing: 5.0,
            ..default()
        };
        assert_eq!(
            default_overlay_prediction_alpha(None, 0.8, 0.01, &mapping),
            0.8
        );
        assert!(
            (default_overlay_prediction_alpha(Some(0.0), 1.0, 0.01, &mapping) - 0.2).abs() < 1e-6
        );
        assert!(
            (default_overlay_prediction_alpha(Some(0.8), 0.0, 0.01, &mapping) - 0.76).abs() < 1e-6
        );
        assert!(
            (default_overlay_prediction_alpha(Some(0.8), 0.0, 0.0, &mapping) - 0.8).abs() < 1e-6
        );
        mapping.interpolate_speed_decreasing = 20.0;
        assert!(
            (default_overlay_prediction_alpha(Some(0.8), 0.0, 0.01, &mapping) - 0.76).abs() > 0.1
        );
    }

    #[test]
    fn grounded_roll_reaches_an_existing_playground_wall_before_reentry() {
        let manifest: crate::playground::PlaygroundManifest = ron::from_str(
            include_str!("../../../assets/ALSExtras/Levels/L_Als_Playground/playground_scene.ron"),
        )
        .unwrap();
        let wall = manifest
            .actors
            .iter()
            .find(|actor| {
                actor.bounds_origin == Some([-4000.0, -3000.0, 100.0])
                    && actor.bounds_extent == Some([1000.0, 25.0, 100.0])
            })
            .unwrap();
        let center = crate::playground::ue_point_to_bevy(Vec3::from(
            wall.bounds_origin.unwrap(),
        ));
        let extent = crate::playground::ue_extents_to_bevy(Vec3::from(
            wall.bounds_extent.unwrap(),
        ));
        let forward = Quat::from_rotation_y(GROUNDED_ROLL_YAW) * Vec3::NEG_Z;
        let ahead = GROUNDED_START.with_y(1.0) + forward * 2.0;
        assert!((ahead - center).abs().cmple(extent).all());
        assert!(
            (GROUNDED_START.with_y(1.0) - center)
                .abs()
                .cmpgt(extent + Vec3::splat(0.35))
                .any()
        );
    }

    #[test]
    fn default_overlay_foot_targets_reject_a_separate_current_pose_trace() {
        let mut spaces = AlsFootIkSpaceState::default();
        spaces.right.solve_input = Some(bevy_als::foot_ik_apply::FootIkSolveInput::default());
        spaces.right.trace_target_world = Vec3::new(-37.139523, 0.67141116, -27.877008);
        spaces.right.solver_reference_world = Vec3::new(-37.141247, 0.5346371, -27.864655);
        assert!(!default_overlay_foot_targets_ready(&spaces));
        spaces.right.trace_target_world = spaces.right.solver_reference_world;
        assert!(default_overlay_foot_targets_ready(&spaces));
        spaces.right.trace_target_world.y = f32::NAN;
        assert!(!default_overlay_foot_targets_ready(&spaces));
    }

    #[test]
    fn layering_coverage_reads_the_generated_pose_input() {
        let mut player = AnimationGraphPlayer::new(default());
        for (legacy, source, expected) in [(1.0, 0.0, false), (0.0, 0.4, true)] {
            player.set_global_input_data("als_layering_crouching_amount", legacy);
            player.set_global_input_data("pose_state.crouching_amount", source);
            assert_eq!(layering_crouching_input_ready(&player), expected);
        }
    }

    #[test]
    fn layering_migration_observer_skips_source_irrelevant_stance_inputs() {
        let mut pose = Pose::default();
        pose.add_bone(
            bevy_animation_graph::core::pose::BonePose::from_transform(Transform::from_xyz(
                1.0, 2.0, 3.0,
            )),
            default(),
        );
        let raw = [1.25938095e-6_f32, 0.9987471];
        let weights = raw.map(|weight| weight / raw.iter().sum::<f32>());
        let expected = pose.scalar_mult(weights[1]).normalize_quat();
        let output = default_overlay_stance_pose([None, Some(pose.clone())], weights);
        assert!(
            output
                .as_ref()
                .and_then(|output| default_overlay_pose_error(output, &expected))
                .is_some_and(|error| error < 1e-6)
        );
        assert!(default_overlay_stance_pose([None, Some(pose.clone())], [0.5, 0.5]).is_none());
        assert!(default_overlay_stance_pose([Some(pose), None], [1.0, 0.0]).is_some());
    }

    #[test]
    fn layering_motion_window_excludes_numerically_checked_fixed_frames() {
        for index in 0..6 {
            let window = default_overlay_window(index, &DefaultOverlayGraphSample::default());
            assert!(!window.source_pose_chain.iter().any(|boundary| matches!(
                boundary.label,
                "source_reference_frame" | "layering_base_poses"
            )));
            assert!(
                window
                    .source_pose_chain
                    .iter()
                    .any(|boundary| boundary.label == "layering_output"
                        && boundary.minimum_rotation_span_radians == 0.001)
            );
        }
    }

    #[test]
    fn layering_observer_distinguishes_previous_inputs_and_preserves_curve_presence() {
        let mut previous = AlsRootAnimationContext::default();
        previous.curves.insert("LayerHead".into(), 0.25);
        let previous_state = bevy_als::layering_layer::refresh_state_from_curves(Some(&previous));
        let mut player = AnimationGraphPlayer::new(default());
        for (key, value) in bevy_als::layering_layer::layering_state_inputs(previous_state) {
            player.set_global_input_data(key, value);
        }
        let matches = |pairs: &std::collections::BTreeMap<String, (Option<f32>, f32)>| {
            pairs.values().all(|(actual, expected)| {
                actual.is_some_and(|actual| (actual - expected).abs() <= 1e-5)
            })
        };
        assert!(matches(&layering_input_sample(&player, previous_state)));
        previous.curves.insert("LayerHead".into(), 0.75);
        assert!(!matches(&layering_input_sample(
            &player,
            bevy_als::layering_layer::refresh_state_from_curves(Some(&previous))
        )));
        assert!(!matches(&layering_input_sample(
            &AnimationGraphPlayer::new(default()),
            previous_state
        )));
        let expected = bevy::platform::collections::HashMap::from_iter([
            ("LayerHead".into(), 0.25),
            ("LayerHeadSlot".into(), 0.0),
        ]);
        let mut actual = expected.clone();
        assert_eq!(layering_curve_error(&actual, &expected), Some(0.0));
        actual.insert("LayerHead".into(), 0.75);
        assert_eq!(layering_curve_error(&actual, &expected), Some(0.5));
        actual.remove(&bevy_als::extensions::SourceCurveName::from("LayerHeadSlot"));
        assert_eq!(layering_curve_error(&actual, &expected), None);
        let window = default_overlay_window(1, &DefaultOverlayGraphSample::default());
        for (label, node, output) in LAYERING_BOUNDARIES {
            if label != "layering_base_poses" {
                assert!(
                    window
                        .source_pose_chain
                        .iter()
                        .any(|boundary| boundary.label == label
                            && boundary.node == node
                            && boundary.output == output
                            && boundary.graph == LAYERING_GRAPH)
                );
            }
        }
        let mut caches = bevy_animation_graph::core::context::node_caches::NodeCaches::default();
        let id = NodeId::default();
        let mut cached = Pose::default();
        cached.add_bone(
            bevy_animation_graph::core::pose::BonePose::from_transform(Transform::IDENTITY),
            default(),
        );
        caches.set_output_data(
            id,
            StateKey::Default,
            bevy_als::extensions::source_cached_pose::CACHED_POSE_VALUE.into(),
            DataValue::Pose(cached),
        );
        caches.mark_updated(id, StateKey::Default);
        let saves = LAYERING_BOUNDARIES
            .iter()
            .filter(|(_, node, _)| node.starts_with("Save cached pose"))
            .collect::<Vec<_>>();
        assert_eq!(saves.len(), 7);
        for (label, _, output) in saves {
            assert!(
                caches
                    .get_output_data(id, StateKey::Default, (*output).into())
                    .is_some(),
                "{label}"
            );
        }
        caches.next_frame();
        assert!(
            caches
                .get_output_data(
                    id,
                    StateKey::Default,
                    bevy_als::extensions::source_cached_pose::CACHED_POSE_VALUE.into()
                )
                .is_none()
        );
    }

    #[test]
    fn default_overlay_missing_source_curves_have_unreal_zero_semantics() {
        let mut context = AlsRootAnimationContext {
            curves: [
                "LayerArmLeft",
                "LayerArmLeftAdditive",
                "LayerArmLeftLocalSpace",
                "LayerArmRight",
                "LayerArmRightAdditive",
                "LayerArmRightLocalSpace",
                "LayerHead",
                "LayerHeadAdditive",
                "LayerLegs",
                "LayerPelvis",
                "LayerSpine",
                "LayerSpineAdditive",
            ]
            .map(|name| (name.into(), 1.0))
            .into(),
        };
        assert!(default_overlay_curves_ready(Some(&context)));
        assert!(!default_overlay_curves_ready(None));
        context
            .curves
            .remove(&bevy_als::extensions::SourceCurveName::from("LayerSpine"));
        assert!(!default_overlay_curves_ready(Some(&context)));
        context.curves.insert("LayerSpine".into(), 1.0);
        context.curves.insert("HandLeftIk".into(), 0.5);
        assert!(!default_overlay_curves_ready(Some(&context)));
        context.curves.insert("HandLeftIk".into(), f32::NAN);
        assert!(!default_overlay_curves_ready(Some(&context)));
    }

    #[test]
    fn locomotion_landing_completion_follows_the_observed_source_exit() {
        let mut probe = LocomotionActionProbe {
            landed_since: Some((0.0, 4)),
            land_notify_start: 2,
            ..default()
        };
        let mut actual = LocomotionGraphSample {
            outer: Some(0),
            outer_transition: Some(false),
            ..default()
        };
        assert!(probe.landing_completed(&actual, 1.0, 2));
        probe.landed_since = Some((0.0, 3));
        assert!(!probe.landing_completed(&actual, 1.0, 2));
        assert!(probe.landing_completed(&actual, 1.0, 3));
        actual.outer_transition = Some(true);
        assert!(!probe.landing_completed(&actual, 1.0, 3));
        actual.outer_transition = Some(false);
        actual.outer = Some(3);
        assert!(!probe.landing_completed(&actual, 1.0, 3));
    }

    #[test]
    fn locomotion_fixture_reset_does_not_claim_the_previous_air_action() {
        let mut probe = LocomotionActionProbe::default();
        let actual = LocomotionGraphSample {
            outer: Some(2),
            outer_weights: [0.0, 0.0, 1.0, 0.0, 0.0],
            ..default()
        };
        probe.record_airborne(&actual, LocomotionState::Air);
        assert!(!probe.airborne_seen);
        probe.triggered = true;
        probe.record_airborne(&actual, LocomotionState::Air);
        assert!(probe.airborne_seen);
    }

    #[test]
    fn locomotion_source_windows_preserve_short_samples_and_actual_parent_transition() {
        let sample = LocomotionClipSample {
            index: 9,
            time: 0.2,
            duration: 1.0,
            weight: 1.0,
            phase: Some(0.2),
            play_rate: 1.0,
            looping: false,
        };
        let actual = LocomotionGraphSample {
            outer: Some(3),
            outer_transition: Some(false),
            ..default()
        };
        let window = locomotion_window(10, sample, &actual);
        assert!(window.capture_every_frame);
        assert_eq!(window.observed_parent_transition_active, Some(false));
        assert_eq!(window.observed_animation_clip_time, Some(0.2));
    }

    #[test]
    fn locomotion_source_inputs_and_watchdog_keep_missing_values_visible() {
        let mut player = AnimationGraphPlayer::new(default());
        assert_eq!(
            locomotion_source_scalar(&player, "feet_state.foot_planted_amount"),
            None
        );
        player.set_global_input_data("feet_state.foot_planted_amount", 0.75_f32);
        assert_eq!(
            locomotion_source_scalar(&player, "feet_state.foot_planted_amount"),
            Some(0.75)
        );
        let mut probe = LocomotionActionProbe::default();
        assert!(!locomotion_actions_should_timeout(&probe, 11.99));
        assert!(locomotion_actions_should_timeout(&probe, 12.0));
        probe.completed = true;
        assert!(!locomotion_actions_should_timeout(&probe, 12.0));
        let sample = LocomotionClipSample {
            index: 6,
            time: 0.2,
            duration: 1.0,
            weight: 0.5,
            phase: Some(0.2),
            play_rate: 1.0,
            looping: true,
        };
        let window = locomotion_window(
            6,
            sample,
            &LocomotionGraphSample {
                outer: Some(1),
                ..default()
            },
        );
        assert!(!locomotion_window_canonical(&window, sample));
        assert!(locomotion_window_canonical(
            &window,
            LocomotionClipSample {
                phase: Some(0.65),
                ..sample
            }
        ));
        let land = LocomotionClipSample {
            index: 9,
            looping: false,
            ..sample
        };
        let window = locomotion_window(
            10,
            land,
            &LocomotionGraphSample {
                outer: Some(3),
                ..default()
            },
        );
        probe.record_window(10, &window, land, 0.1);
        assert!(!probe.covered[10]);
        probe.record_window(10, &window, LocomotionClipSample { time: 0.3, ..land }, 0.1);
        assert!(probe.covered[10]);
    }

    #[test]
    fn locomotion_fixture_waits_for_source_feet_and_checks_continuous_jump_weights() {
        for index in 0..4 {
            let speed = if index < 2 { 1.8 } else { 4.2 };
            let foot = if index % 2 == 0 { -1.0 } else { 1.0 };
            assert!(locomotion_jump_ready(index, Some(0), speed, Some(foot)));
            assert!(!locomotion_jump_ready(index, Some(2), speed, Some(foot)));
            assert!(!locomotion_jump_ready(index, Some(0), speed, Some(-foot)));
            assert!(!locomotion_jump_ready(index, Some(0), speed, None));
            let run = ((speed - 2.0) / 3.0_f32).clamp(0.0, 1.0);
            let mut clips = vec![
                LocomotionClipSample {
                    index: index % 2,
                    time: 0.2,
                    duration: 1.0,
                    weight: (1.0 - run) * 0.3,
                    phase: Some(0.35),
                    play_rate: 1.3,
                    looping: false,
                },
                LocomotionClipSample {
                    index: index % 2 + 2,
                    time: 0.2,
                    duration: 1.0,
                    weight: run * 0.3,
                    phase: Some(0.35),
                    play_rate: 1.3,
                    looping: false,
                },
            ];
            assert!(locomotion_jump_weights_match(&clips, [Some(run); 2]));
            assert!(!locomotion_jump_weights_match(&clips, [None; 2]));
            clips[1].weight += 0.1;
            assert!(!locomotion_jump_weights_match(&clips, [Some(run); 2]));
        }
    }

    #[test]
    fn locomotion_jump_weights_keep_source_alpha_interpolation_on_speed_changes() {
        use bevy_als::extensions::source_pose_nodes::{
            SourceAlphaMapping, SourceInputScaleBiasClampState, apply_source_input_scale_bias_clamp,
        };
        let mut mapping = SourceAlphaMapping {
            source_semantics: true,
            map_range: true,
            input_min: 2.0,
            input_max: 5.0,
            interpolate_result: true,
            interpolate_speed_increasing: 5.0,
            interpolate_speed_decreasing: 5.0,
            ..default()
        };
        let mut state = SourceInputScaleBiasClampState::default();
        apply_source_input_scale_bias_clamp(&mut state, 3.75, &mapping, 0.0132349);
        let alpha = apply_source_input_scale_bias_clamp(&mut state, 1.5552317, &mapping, 0.0132349)
            .clamp(0.0, 1.0);
        assert!((alpha - 0.5349207).abs() < 1e-5);
        let clips = [
            LocomotionClipSample {
                index: 0,
                time: 1.1970694,
                duration: 1.6666665,
                weight: 0.12463666,
                phase: Some(0.7182417),
                play_rate: 1.3875,
                looping: false,
            },
            LocomotionClipSample {
                index: 2,
                time: 1.1970694,
                duration: 1.6666665,
                weight: 0.14335346,
                phase: Some(0.7182417),
                play_rate: 1.3875,
                looping: false,
            },
        ];
        assert!(locomotion_jump_weights_match(&clips, [Some(alpha), None]));
        mapping.interpolate_result = false;
        let unfiltered =
            apply_source_input_scale_bias_clamp(&mut state, 1.5552317, &mapping, 0.0132349)
                .clamp(0.0, 1.0);
        assert!(!locomotion_jump_weights_match(
            &clips,
            [Some(unfiltered), None]
        ));
    }

    #[test]
    fn locomotion_observation_binds_both_generated_state_machines_and_all_source_clips() {
        assert_eq!(
            initial_phase(IntegrationScenario::LocomotionActions),
            IntegrationPhase::LocomotionActions
        );
        for (path, title, label, expected) in [
            (
                LOCOMOTION_ROOT,
                "Locomotion States\nState Machine",
                "Land Movement",
                4,
            ),
            (LOCOMOTION_JUMP, "Jump States\nState Machine", "Flail", 3),
        ] {
            let graph = std::fs::read_to_string(format!("assets/{path}")).unwrap();
            assert!(graph.contains(&format!("name: {:?}", title)));
            assert_eq!(locomotion_state_index(title, label), Some(expected));
            assert_eq!(locomotion_state_index("Locomotion States", label), None);
        }
        for (index, (variant, title)) in LOCOMOTION_CLIPS.iter().enumerate() {
            let graph = std::fs::read_to_string(format!("assets/{}", locomotion_clip_graph(index)))
                .unwrap();
            assert!(graph.contains(&format!("name: {:?}", title)), "{variant}");
        }
        let root = include_str!("../../../assets/ALS/Character/AB_Als/AnimGraph.animgraph.ron");
        assert!(root.contains(LOCOMOTION_ROOT));
    }

    #[test]
    fn locomotion_long_air_fixture_stays_above_the_original_floor_for_the_source_transition() {
        use crate::playground::{
            PlaygroundActor, ue_extents_to_bevy, ue_point_to_bevy,
        };
        let actors: Vec<PlaygroundActor> = serde_json::from_str(include_str!(
            "../../../assets/ALSExtras/Levels/L_Als_Playground/playground.json"
        ))
        .unwrap();
        for offset in [Vec3::ZERO, Vec3::X * 12.0] {
            let point = GROUNDED_START + offset;
            assert!(actors.iter().any(|actor| {
                let (Some(origin), Some(extent)) = (actor.bounds_origin, actor.bounds_extent)
                else {
                    return false;
                };
                let center = ue_point_to_bevy(Vec3::from(origin));
                let extent = ue_extents_to_bevy(Vec3::from(extent));
                (center.y + extent.y).abs() < 0.01
                    && (point.x - center.x).abs() + 0.3 < extent.x
                    && (point.z - center.z).abs() + 0.3 < extent.z
            }));
        }
        let params = AlsMovementParams::default();
        let source_duration = 1.666667 / 1.2 + 0.2 + 1.0 + 0.2;
        let fallen = 0.5 * 9.81 * params.jump.gravity_scale * source_duration * source_duration
            - params.jump.jump_z_velocity * source_duration;
        assert!(LOCOMOTION_DROP_HEIGHT > fallen);
        assert!(18.0 < fallen);
    }

    #[test]
    fn bow_mantle_fixture_reaches_the_source_non_low_boundary() {
        use avian3d::prelude::SimpleCollider;
        use bevy::ecs::system::RunSystemOnce;
        for scenario in [IntegrationScenario::Base, IntegrationScenario::BowOverlay] {
            let mut world = World::new();
            world.init_resource::<Assets<Mesh>>();
            world.init_resource::<Assets<StandardMaterial>>();
            world
                .run_system_once(
                    move |mut commands: Commands,
                          mut meshes: ResMut<Assets<Mesh>>,
                          mut materials: ResMut<Assets<StandardMaterial>>| {
                        spawn_manual_integration_fixture(
                            &mut commands,
                            &mut meshes,
                            &mut materials,
                            Vec3::Y * 0.92,
                            0.0,
                            scenario,
                        );
                    },
                )
                .unwrap();
            let mut query = world.query::<(&Name, &Collider, &Transform)>();
            let (_, collider, transform) = query
                .iter(&world)
                .find(|(name, _, _)| name.as_str() == "manual_integration_mantle_ledge")
                .unwrap();
            let top = collider
                .aabb(transform.translation, transform.rotation, 0.0)
                .max
                .y;
            assert_eq!(top > 1.25, scenario == IntegrationScenario::BowOverlay);
            assert!(
                (top - if scenario == IntegrationScenario::BowOverlay {
                    1.6
                } else {
                    0.8
                })
                .abs()
                    < 1e-5
            );
        }
    }

    #[test]
    fn locomotion_platform_physics_matches_its_visible_support_surface() {
        use avian3d::prelude::SimpleCollider;
        use bevy::ecs::system::RunSystemOnce;
        let mut world = World::new();
        world.init_resource::<Assets<Mesh>>();
        world.init_resource::<Assets<StandardMaterial>>();
        world
            .run_system_once(
                |mut commands: Commands,
                 mut meshes: ResMut<Assets<Mesh>>,
                 mut materials: ResMut<Assets<StandardMaterial>>| {
                    spawn_manual_integration_fixture(
                        &mut commands,
                        &mut meshes,
                        &mut materials,
                        Vec3::Y * 0.92,
                        0.0,
                        IntegrationScenario::Base,
                    );
                },
            )
            .unwrap();
        let entity = world.entity(world.resource::<ManualIntegrationFixture>().platform);
        let transform = entity.get::<Transform>().unwrap();
        let aabb =
            entity
                .get::<Collider>()
                .unwrap()
                .aabb(transform.translation, transform.rotation, 0.0);
        let meshes = world.resource::<Assets<Mesh>>();
        let points = meshes
            .get(&entity.get::<Mesh3d>().unwrap().0)
            .unwrap()
            .attribute(Mesh::ATTRIBUTE_POSITION)
            .unwrap()
            .as_float3()
            .unwrap();
        let (min, max) = points
            .iter()
            .map(|point| transform.transform_point(Vec3::from(*point)))
            .fold(
                (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
                |(min, max), point| (min.min(point), max.max(point)),
            );
        assert!(
            aabb.min.distance(min) < 1e-5 && aabb.max.distance(max) < 1e-5,
            "physical={aabb:?} visible=({min:?}, {max:?})"
        );
    }

    #[test]
    fn get_up_fixture_covers_both_source_facing_branches() {
        for yaw in [-0.7, 0.0, 1.2] {
            for facing_up in [false, true] {
                assert_eq!(
                    bevy_als::ragdoll_link::ragdoll_recovery_rotation(get_up_fixture_rotation(
                        facing_up, yaw
                    ))
                    .1,
                    facing_up
                );
            }
        }
    }

    #[derive(Resource, Default)]
    struct BridgeOrderProbe {
        published: u8,
        observed: u8,
    }

    fn publish_bridge_order_probe(mut probe: ResMut<BridgeOrderProbe>) {
        probe.published = 1;
    }

    fn observe_bridge_order_probe(mut probe: ResMut<BridgeOrderProbe>) {
        probe.observed = probe.published;
    }

    #[test]
    fn grounded_scenario_starts_with_the_source_roll_chain() {
        assert_eq!(
            initial_phase(IntegrationScenario::Grounded),
            IntegrationPhase::Roll
        );
    }

    #[test]
    fn grounded_joint_input_holds_aim_before_moving_and_rotating() {
        for index in [4, 5] {
            let first = grounded_joint_command(index, None, 0.0, 0.0);
            assert!(
                first.aim_held && first.move_dir == Vec2::ZERO && first.look_delta == Vec2::ZERO
            );
            let moving = grounded_joint_command(index, Some(0.8), 0.0, 0.0);
            assert!(
                moving.aim_held && moving.move_dir == Vec2::Y && moving.look_delta == Vec2::ZERO
            );
            let rotating = grounded_joint_command(index, Some(1.7), 0.0, 0.0);
            assert!(
                rotating.aim_held
                    && rotating.move_dir == Vec2::Y
                    && rotating.look_delta.x * if index == 4 { -1.0 } else { 1.0 } < 0.0
            );
        }
        for index in [6, 7] {
            let right = grounded_joint_command(index, Some(0.125), 0.0, 0.0);
            let left = grounded_joint_command(index, Some(0.375), 0.0, std::f32::consts::PI);
            let stopped = grounded_joint_command(index, Some(4.5), 0.0, 0.0);
            assert!(right.aim_held && left.aim_held && stopped.aim_held);
            assert!(
                right.look_delta.x < 0.0
                    && left.look_delta.x > 0.0
                    && stopped.look_delta == Vec2::ZERO
            );
            assert_eq!(
                [right.move_dir, left.move_dir, stopped.move_dir],
                [Vec2::ZERO; 3]
            );
        }
        assert!(!grounded_joint_command(1, None, 0.0, 0.0).jump_held);
        assert!(grounded_joint_command(1, Some(0.25), 0.0, 0.0).jump_held);
        assert!(!grounded_joint_command(1, Some(0.6), 0.0, 0.0).jump_held);
        assert_eq!(grounded_joint_command(2, None, 0.0, 0.0).move_dir, Vec2::Y);
        assert_eq!(
            grounded_joint_command(0, None, 0.0, 0.0).move_dir,
            Vec2::ZERO
        );
    }

    #[test]
    fn grounded_joint_probe_rejects_height_drop_looping_and_time_rewind() {
        let mut probe = GroundedJointProbe {
            valid: true,
            ..default()
        };
        probe.record(Some(-0.02), Some((0.4, 0.9, false, 1.2)), 0.016);
        assert!(probe.valid && !probe.sequence_completed);
        probe.record(Some(-0.02), Some((0.89, 0.9, false, 1.2)), 0.016);
        assert!(probe.valid && probe.sequence_completed);
        for (height, position, looping) in [
            (-0.36, 0.89, false),
            (-0.02, 0.89, true),
            (-0.02, 0.01, false),
        ] {
            let mut failed = GroundedJointProbe {
                valid: true,
                last_sequence_time: Some(0.8),
                ..default()
            };
            failed.record(Some(height), Some((position, 0.9, looping, 1.2)), 0.016);
            assert!(!failed.valid);
        }
    }

    #[test]
    fn transition_notify_starts_at_the_source_stop_window() {
        assert_eq!(
            initial_phase(IntegrationScenario::TransitionNotify),
            IntegrationPhase::TransitionNotify
        );
    }

    #[test]
    fn standing_turn_fixture_drives_actual_mouse_and_aim_inputs() {
        for dt in [1.0_f32 / 30.0, 1.0 / 120.0] {
            for (index, expected_degrees) in
                [90.0_f32, -90.0, 160.0, -160.0, 160.0, -160.0, -75.0, 0.0]
                    .into_iter()
                    .enumerate()
            {
                let mut app = App::new();
                let mut time = Time::<()>::default();
                time.advance_by(Duration::from_secs_f32(dt));
                let mut real_time = Time::<Real>::default();
                real_time.advance_by(Duration::from_secs_f32(dt));
                let mut virtual_time = Time::<Virtual>::default();
                virtual_time.advance_by(Duration::from_secs_f32(dt));
                app.insert_resource(time)
                    .insert_resource(real_time)
                    .insert_resource(virtual_time)
                    .add_plugins(bevy_als::als::transition::AlsStateTransitionPlugin)
                    .add_message::<bevy_als::als::aim::AimStateChanged>()
                    .add_systems(
                        Update,
                        (
                            bevy_als::als::input_request::input_state_request_system,
                            bevy_als::als::aim::aim_rotation_mode_system,
                            bevy_als::als::view::apply_view_input_system,
                        )
                            .chain()
                            .before(bevy_als::als::transition::state_transition_system),
                    );
                let entity = app
                    .world_mut()
                    .spawn((
                        AlsCharacter,
                        crate::input::AlsInputCommand::default(),
                        AlsInputOverride::default(),
                        AlsMaxAllowedGait(Gait::Walking),
                        Stance::Standing,
                        LocomotionState::Grounded,
                        Gait::Walking,
                        RotationMode::VelocityDirection,
                        OverlayState::Default,
                        bevy_als::als::aim::AlsLastFreeRotationMode(RotationMode::VelocityDirection),
                        AlsStateRequest::default(),
                        AlsViewYaw(0.3),
                        AlsViewPitch::default(),
                    ))
                    .id();
                let mut travel = [0.0_f32; 2];
                let duration = if index == 7 { 12.5_f32 } else { 1.6 };
                for frame in 0..(duration / dt).round() as usize {
                    let view = app.world().get::<AlsViewYaw>(entity).unwrap().0;
                    let mode = *app.world().get::<RotationMode>(entity).unwrap();
                    let command = standing_turn_mouse_command(
                        index,
                        Some(frame as f32 * dt),
                        0.3,
                        view,
                        mode,
                    );
                    assert_eq!(command.move_dir, Vec2::ZERO);
                    app.world_mut()
                        .get_mut::<AlsInputOverride>(entity)
                        .unwrap()
                        .0 = command;
                    app.update();
                    let delta = app.world().get::<AlsViewYaw>(entity).unwrap().0 - view;
                    travel[usize::from(delta < 0.0)] += delta.abs();
                }
                if index == 7 {
                    assert!(
                        travel
                            .iter()
                            .all(|angle| *angle > 11.0 * std::f32::consts::TAU)
                    );
                }
                let view = app.world().get::<AlsViewYaw>(entity).unwrap().0;
                assert!(
                    bevy_als::als::turn_in_place::wrap_angle_degrees(
                        (view - 0.3).to_degrees() - expected_degrees
                    )
                    .abs()
                        < 1e-3
                );
                assert_eq!(
                    *app.world().get::<RotationMode>(entity).unwrap(),
                    if matches!(index, 4 | 5 | 7) {
                        RotationMode::Aiming
                    } else {
                        RotationMode::ViewDirection
                    }
                );
            }
        }
    }

    #[test]
    fn standing_turn_trace_keeps_the_actual_completed_variant() {
        let mut window = ManualAnimationWindowEvidence::default();
        let drive = AlsTurnDrive {
            lr: 1.0,
            is_180: true,
            play_rate: 1.2,
            ..default()
        };
        record_animation_window_trace(
            &mut window,
            &Transform::IDENTITY,
            0.0,
            Some(&AlsRotationAnimationState::default()),
            None,
            Some(&drive),
            None,
            false,
            None,
        );
        assert_eq!(
            window.observed_animation_variant,
            Some("A_Als_Turn_180_Right")
        );
        assert_eq!(window.observed_animation_clip_time, Some(0.0));
        assert_eq!(window.observed_animation_weight, Some(0.0));
        assert_eq!(window.observed_animation_play_rate, Some(1.2));
    }

    #[test]
    fn standing_turn_settlement_waits_for_curve_consumption_and_pending_turns() {
        assert!(!standing_turn_rotation_settled(6, 74.0, 0.0, false));
        assert!(standing_turn_rotation_settled(6, 44.0, 0.0, false));
        assert!(!standing_turn_rotation_settled(4, 49.0, -0.08568687, false));
        assert!(!standing_turn_rotation_settled(4, 49.0, 0.0, true));
        assert!(!standing_turn_rotation_settled(7, 51.0, 0.0, false));
        assert!(standing_turn_rotation_settled(7, 49.0, 0.0, false));
    }

    #[test]
    fn standing_turn_completion_requires_playback_end_and_actual_idle() {
        let mut probe = StandingTurnProbe {
            started_at: Some(0.0),
            active_seen: true,
            clip_peak: 1.95,
            epoch_baseline: 2.0,
            sides_mask: 3,
            max_play_rate: 3.0,
            yaw_travel: [7.0 * std::f32::consts::TAU; 2],
            ..default()
        };
        for index in 0..STANDING_TURN_CASES.len() {
            assert!(standing_turn_completed(
                index, &probe, true, true, 5.0, 14.0
            ));
            assert!(!standing_turn_completed(
                index, &probe, false, true, 5.0, 14.0
            ));
            assert!(!standing_turn_completed(
                index, &probe, true, false, 5.0, 14.0
            ));
            probe.active_seen = false;
            assert!(!standing_turn_completed(
                index, &probe, true, true, 5.0, 14.0
            ));
            probe.active_seen = true;
        }
        assert!(!standing_turn_completed(0, &probe, true, true, 2.0, 14.0));
        probe.yaw_travel[1] = 5.0 * std::f32::consts::TAU;
        assert!(!standing_turn_completed(7, &probe, true, true, 5.0, 14.0));
        assert!(!standing_turn_completed(6, &probe, true, true, 4.0, 14.0));
        probe.clip_peak = 0.5;
        assert!(!standing_turn_completed(0, &probe, true, true, 5.0, 14.0));
        assert!(!standing_turn_completed(4, &probe, true, true, 5.0, 14.0));
        assert_eq!(
            initial_phase(IntegrationScenario::StandingTurn),
            IntegrationPhase::StandingTurn
        );
    }

    #[test]
    fn standing_stop_fixture_waits_for_each_source_foot_phase() {
        assert_eq!(
            initial_phase(IntegrationScenario::StandingStop),
            IntegrationPhase::StandingStop
        );
        for (index, value) in [-0.9, 0.9, -0.25, 0.25].into_iter().enumerate() {
            assert!(standing_stop_release_phase(index, Some(value)));
            assert!(!standing_stop_release_phase(index, None));
            for other in 0..7 {
                assert_eq!(
                    standing_stop_release_phase(other, Some(value)),
                    index == other
                );
            }
        }
        assert_eq!(STANDING_STOP_CASES[5].2, Vec2::NEG_X);
        assert_eq!(STANDING_STOP_CASES[6].2, Vec2::X);
        let mut playing = AlsTransitionPlayback::stop(0.0);
        playing.advance(0.4);
        let mut finished = playing;
        finished.advance(2.0);
        assert!(finished.active && finished.stop_elapsed.is_some());
        finished.advance(finished.blend_out + 0.01);
        assert!(standing_stop_completed(
            true,
            Some(&playing),
            Some(&finished),
            true
        ));
        assert!(!standing_stop_completed(
            false,
            Some(&playing),
            Some(&finished),
            true
        ));
        assert!(!standing_stop_completed(true, None, Some(&finished), true));
        assert!(!standing_stop_completed(
            true,
            Some(&playing),
            Some(&playing),
            true
        ));
        assert!(!standing_stop_completed(
            true,
            Some(&playing),
            Some(&finished),
            false
        ));
    }

    #[test]
    fn standing_stop_case_handoff_waits_for_its_driver_entry() {
        let mut state = ManualIntegrationState {
            phase: IntegrationPhase::StandingStop,
            entered: false,
            ..default()
        };
        assert!(standing_stop_observation_active(&state));
        state.stop_case_index += 1;
        begin_phase(&mut state, IntegrationPhase::StandingStop);
        assert!(!standing_stop_observation_active(&state));
    }

    #[test]
    fn standing_dynamic_tracks_a_played_instance_until_it_stops() {
        for kind in [
            AlsTransitionKind::StandingDynamic,
            AlsTransitionKind::CrouchingDynamic,
        ] {
            let mut played = AlsTransitionPlayback::standing_dynamic(0.0);
            played.kind = kind;
            played.advance(0.6);
            let mut replacement = AlsTransitionPlayback::standing_dynamic(1.0);
            replacement.kind = kind;
            let mut stack = AlsTransitionMontageStack::default();
            stack.entries = vec![played, replacement];
            let observed =
                standing_dynamic_observed(Some(&stack), Some(&played), kind, 0.0).unwrap();
            assert_eq!(observed.instance_id, played.instance_id);
            assert!(!standing_dynamic_stopped(
                true,
                Some(&played),
                Some(observed),
                true
            ));
            stack.entries.remove(0);
            let observed = standing_dynamic_observed(Some(&stack), Some(&played), kind, 0.0);
            assert!(observed.is_none());
            assert!(standing_dynamic_stopped(
                true,
                Some(&played),
                observed,
                true
            ));
            assert!(!standing_dynamic_stopped(
                false,
                Some(&played),
                observed,
                true
            ));
            assert!(!standing_dynamic_stopped(
                true,
                Some(&played),
                observed,
                false
            ));
            played.clip_time = 0.5;
            assert!(!standing_dynamic_stopped(
                true,
                Some(&played),
                observed,
                true
            ));
        }
    }

    #[test]
    fn standing_dynamic_input_reaches_both_actual_transition_branches() {
        let (mut app, character, avatar) = crate::test_support::loaded_character();
        let player = app
            .world()
            .get::<AnimatedSceneInstance>(avatar)
            .unwrap()
            .player_entity();
        let original = app.world().get::<AnimationGraphPlayer>(player).unwrap();
        let bevy_animation_graph::core::animation_graph_player::AnimationSource::Graph(graph) =
            original.get_animation_source()
        else {
            panic!("Source graph is missing");
        };
        let replacement =
            AnimationGraphPlayer::new(original.skeleton().clone()).with_graph(graph.clone());
        app.world_mut().entity_mut(player).insert(replacement);
        app.world_mut()
            .entity_mut(character)
            .insert(AlsInputOverride::default());
        for index in 0..2 {
            let old_instances = app
                .world()
                .get::<AlsTransitionMontageStack>(character)
                .map(|stack| {
                    stack
                        .entries
                        .iter()
                        .map(|playback| playback.instance_id)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let mut observed = false;
            let mut moved = false;
            let deadline = Instant::now() + Duration::from_secs(25);
            for frame in 0..420 {
                let ready = frame >= 60
                    && app
                        .world()
                        .get::<bevy_als::animation_context_projection::AlsRootAnimationContext>(player)
                        .and_then(|context| context.f32("AllowTransitions"))
                        .is_some_and(bevy_als::als::foot_ik::is_full_animation_weight);
                if ready && !moved {
                    let world = app.world_mut();
                    let mut query = world.query::<(&mut Transform, &mut Position)>();
                    let (mut transform, mut position) = query.get_mut(world, character).unwrap();
                    let forward = *transform.forward();
                    transform.translation -= forward * 0.1;
                    position.0 = transform.translation;
                    moved = true;
                }
                let body_yaw = world_forward_to_yaw(
                    *app.world().get::<Transform>(character).unwrap().forward(),
                );
                let view_yaw = app.world().get::<AlsViewYaw>(character).unwrap().0;
                let mode = *app.world().get::<RotationMode>(character).unwrap();
                app.world_mut()
                    .get_mut::<AlsInputOverride>(character)
                    .unwrap()
                    .0 = dynamic_idle_command(body_yaw, view_yaw, mode);
                app.update();
                observed |= moved
                    && app
                        .world()
                        .get::<AlsTransitionMontageStack>(character)
                        .is_some_and(|stack| {
                            stack.entries.iter().any(|playback| {
                                playback.kind == AlsTransitionKind::StandingDynamic
                                    && playback.active
                                    && !old_instances.contains(&playback.instance_id)
                                    && playback.index == STANDING_DYNAMIC_CASES[index].1
                            })
                        })
                    && app
                        .world()
                        .get::<bevy_als::stop_link::AlsRootTransitionSlotRuntime>(character)
                        .is_some_and(|slot| slot.active && slot.pose_ready && slot.weight > 0.05);
                assert!(
                    Instant::now() < deadline,
                    "Standing dynamic input fixture made no timely progress"
                );
            }
            assert!(
                moved && observed,
                "Standing dynamic input did not reach its source branch: case={index} moved={moved}"
            );
        }
    }

    #[test]
    fn standing_dynamic_inputs_release_aim_and_require_foot_recovery() {
        for current in [
            RotationMode::ViewDirection,
            RotationMode::Aiming,
            RotationMode::VelocityDirection,
        ] {
            let command = dynamic_idle_command(0.4, 0.3, current);
            assert_eq!(command.move_dir, Vec2::ZERO);
            assert!(!command.aim_held);
            assert_eq!(
                command.rotation_mode_pressed,
                current != RotationMode::VelocityDirection
            );
            assert!(
                (0.3 - command.look_delta.x * bevy_als::als::view::LOOK_MOUSE_RADIANS_PER_COUNT - 0.4)
                    .abs()
                    < 1e-5
            );
        }
        let mut foot = bevy_als::als::foot_ik::AlsFootLockFootState {
            lock_amount: 1.0,
            ..default()
        };
        assert!(standing_dynamic_foot_restored(true, Some(&foot), 1.0));
        assert!(!standing_dynamic_foot_restored(false, Some(&foot), 1.0));
        assert!(!standing_dynamic_foot_restored(true, None, 1.0));
        foot.lock_amount = 0.5;
        assert!(!standing_dynamic_foot_restored(true, Some(&foot), 1.0));
        foot.lock_amount = 1.0;
        foot.target_location = bevy::math::DVec3::X * 0.09_f32 as f64;
        assert!(!standing_dynamic_foot_restored(true, Some(&foot), 1.0));
        assert!(standing_dynamic_foot_restored(true, Some(&foot), 2.0));
        let mut playing = AlsTransitionPlayback::standing_dynamic(0.0);
        playing.advance(0.6);
        let mut finished = playing;
        finished.advance(2.0);
        assert!(finished.active && finished.stop_elapsed.is_some());
        finished.advance(finished.blend_out + 0.01);
        assert!(standing_stop_completed(
            true,
            Some(&playing),
            Some(&finished),
            true
        ));
        assert!(!standing_stop_completed(
            true,
            Some(&playing),
            Some(&playing),
            true
        ));
        assert!(!standing_stop_completed(
            false,
            Some(&playing),
            Some(&finished),
            true
        ));
        assert_eq!(
            initial_phase(IntegrationScenario::StandingDynamic),
            IntegrationPhase::StandingDynamic
        );
    }

    #[test]
    fn standing_stop_modes_survive_actual_input_and_aim_systems() {
        for (crouching, index) in [false, true].into_iter().flat_map(|crouching| {
            (0..stop_cases(crouching).len()).map(move |index| (crouching, index))
        }) {
            let mut app = App::new();
            app.init_resource::<Time<Virtual>>();
            app.add_plugins(bevy_als::als::transition::AlsStateTransitionPlugin)
                .add_message::<bevy_als::als::aim::AimStateChanged>()
                .add_systems(
                    Update,
                    (
                        bevy_als::als::input_request::input_state_request_system,
                        bevy_als::als::aim::aim_rotation_mode_system,
                    )
                        .chain()
                        .before(bevy_als::als::transition::state_transition_system),
                );
            let entity = app
                .world_mut()
                .spawn((
                    AlsCharacter,
                    crate::input::AlsInputCommand::default(),
                    AlsInputOverride::default(),
                    AlsMaxAllowedGait(Gait::Running),
                    if crouching {
                        Stance::Crouching
                    } else {
                        Stance::Standing
                    },
                    LocomotionState::Grounded,
                    Gait::Running,
                    RotationMode::ViewDirection,
                    OverlayState::Default,
                    bevy_als::als::aim::AlsLastFreeRotationMode(RotationMode::ViewDirection),
                    AlsStateRequest::default(),
                ))
                .id();
            for moving in [false, true, false] {
                let current = *app.world().get::<RotationMode>(entity).unwrap();
                app.world_mut()
                    .get_mut::<AlsInputOverride>(entity)
                    .unwrap()
                    .0 = standing_stop_command(index, current, moving, crouching);
                app.update();
                assert_eq!(
                    *app.world().get::<RotationMode>(entity).unwrap(),
                    stop_cases(crouching)[index].3
                );
                assert_eq!(
                    *app.world().get::<Stance>(entity).unwrap(),
                    if crouching {
                        Stance::Crouching
                    } else {
                        Stance::Standing
                    }
                );
            }
        }
    }

    #[test]
    fn crouching_dynamic_fixture_displaces_only_the_requested_foot() {
        use bevy_als::als::transitions::{
            TransitionConfig, TransitionSequence, select_dynamic_transition,
        };

        let original =
            Transform::from_xyz(2.0, 1.0, -3.0).with_rotation(Quat::from_rotation_y(0.7));
        let local_feet = [Vec3::new(-0.17, -0.6, 0.03), Vec3::new(0.17, -0.6, -0.03)];
        let locks = local_feet.map(|point| original.transform_point(point).as_dvec3());
        let [left, right] = locks.map(|location| bevy_als::als::foot_ik::AlsFootLockFootState {
            target_location: location,
            lock_location: location,
            lock_amount: 1.0,
            ..default()
        });
        let feet = AlsFootLockState {
            valid: true,
            left,
            right,
            ..default()
        };
        let select = |pose: Transform| {
            let distances = std::array::from_fn(|side| {
                dynamic_distance_squared_cm(pose.transform_point(local_feet[side]), locks[side])
            });
            select_dynamic_transition(
                distances,
                [left.lock_amount, right.lock_amount],
                1.0,
                Stance::Crouching,
                &TransitionConfig::default(),
            )
            .map(|selection| selection.sequence)
        };
        assert_eq!(select(original), None);
        for (side, sequence) in [
            (1.0, TransitionSequence::CrouchingRight),
            (0.0, TransitionSequence::CrouchingLeft),
        ] {
            let pose = crouching_dynamic_fixture_pose(original, &feet, side).unwrap();
            let pivot_index = usize::from(side == 0.0);
            assert!(
                dynamic_distance_squared_cm(
                    pose.transform_point(local_feet[pivot_index]),
                    locks[pivot_index]
                ) < dynamic_threshold_squared_cm(1.0)
            );
            assert_eq!(select(pose), Some(sequence));
        }
    }

    #[test]
    fn crouching_actions_use_the_actual_stance_graph_and_continuous_source_inputs() {
        assert_eq!(
            initial_phase(IntegrationScenario::CrouchingActions),
            IntegrationPhase::StandingStop
        );
        let standing = AlsStandingGraphTelemetry {
            actual_standing_state: Some(0),
            actual_standing_transition_active: Some(false),
            actual_standing_pose_ready: Some(true),
            ..default()
        };
        let crouch = AlsCrouchingGraphTelemetry {
            actual_outer_state: Some(4),
            actual_outer_transition_active: Some(true),
            actual_outer_pose_ready: Some(true),
            ..default()
        };
        assert_eq!(
            action_graph_state(true, Some(&standing), Some(&crouch)),
            (Some(4), Some(true), Some(true))
        );
        assert_eq!(
            action_graph_state(true, Some(&standing), None),
            (None, None, None)
        );
        assert_eq!(
            action_graph_state(false, Some(&standing), Some(&crouch)),
            (Some(0), Some(false), Some(true))
        );
        for side in [0.0, 1.0] {
            assert_eq!(dynamic_transition_side(false, side), side);
            assert_eq!(dynamic_transition_side(true, side + 2.0), side);
        }
        let chain = generated_stop_source_pose_chain(true);
        assert_eq!(chain.len(), 3);
        assert!(
            chain
                .iter()
                .all(|boundary| boundary.graph.contains("AB_Als_Crouching"))
        );
        let mut stop = AlsTransitionPlayback::crouching_stop_right();
        stop.advance(0.6);
        let mut finished = stop;
        finished.advance(3.0);
        assert!(finished.active && finished.stop_elapsed.is_some());
        finished.advance(finished.blend_out + 0.01);
        assert!(!standing_stop_completed(
            true,
            Some(&stop),
            Some(&finished),
            false
        ));
        assert!(standing_stop_completed(
            true,
            Some(&stop),
            Some(&finished),
            true
        ));
        for index in [2, 3] {
            let command = standing_stop_command(index, RotationMode::VelocityDirection, true, true);
            let input_yaw = (command.move_dir != Vec2::ZERO).then(|| {
                bevy_als::als::orientation::world_forward_to_yaw(
                    bevy_als::als::motion::view_relative_move_direction(command.move_dir, 0.3),
                )
                .to_degrees()
            });
            let (side, rate) = bevy_als::stop_link::quick_stop_selection(
                RotationMode::VelocityDirection,
                input_yaw,
                &Transform::from_rotation(Quat::from_rotation_y(0.3)),
                Some(0.3),
            );
            assert_eq!(side, CROUCHING_STOP_CASES[index].4);
            assert!((1.75..1.9).contains(&rate));
        }
        for index in [2, 3] {
            let command = standing_turn_mouse_command(
                index,
                Some(0.0),
                0.3,
                0.3,
                RotationMode::ViewDirection,
            );
            let yaw = -command.look_delta.x * bevy_als::als::view::LOOK_MOUSE_RADIANS_PER_COUNT;
            assert_eq!(yaw.signum(), if index == 2 { 1.0 } else { -1.0 });
            assert!(
                yaw.abs().to_degrees()
                    > bevy_als::als::turn_in_place::TurnInPlaceConfig::default()
                        .turn_180_angle_threshold_degrees
            );
            assert!(!command.aim_held);
        }
    }

    #[test]
    fn transition_notify_releases_on_actual_idle_to_move_transition() {
        assert_eq!(
            generated_stop_quick_input_step(false, false, None),
            (false, false, false)
        );
        let idle = AlsStandingGraphTelemetry {
            actual_standing_state: Some(0),
            actual_standing_pose_ready: Some(true),
            ..default()
        };
        assert_eq!(
            generated_stop_quick_input_step(false, false, Some(&idle)),
            (true, false, true)
        );
        let transitioning = AlsStandingGraphTelemetry {
            actual_standing_transition_active: Some(true),
            ..default()
        };
        assert_eq!(
            generated_stop_quick_input_step(true, false, Some(&transitioning)),
            (true, true, false)
        );
    }

    #[test]
    fn transition_notify_requires_a_new_count_after_the_current_release() {
        let mut baseline = None;
        assert!(!generated_stop_quick_observed(&mut baseline, false, 4));
        assert!(!generated_stop_quick_observed(&mut baseline, true, 4));
        assert!(generated_stop_quick_observed(&mut baseline, true, 5));
        baseline = None;
        assert!(!generated_stop_quick_observed(&mut baseline, true, 5));
    }

    #[test]
    fn manual_exit_waits_for_visual_capture_completion() {
        assert!(!manual_gate_exit_ready(
            Some(&AnimationVisualValidationProgress {
                evidence_ready: false,
            }),
            false,
        ));
        assert!(manual_gate_exit_ready(
            Some(&AnimationVisualValidationProgress {
                evidence_ready: true,
            }),
            false,
        ));
        assert!(manual_gate_exit_ready(None, false));
        assert!(manual_gate_exit_ready(
            Some(&AnimationVisualValidationProgress {
                evidence_ready: false,
            }),
            true,
        ));
    }

    #[test]
    fn grounded_roll_exit_probe_requires_source_transition_and_slot_pose() {
        let roll = AlsRollState::default();
        let playback = AlsTransitionPlayback::standing_transition_right();
        let slot = AlsRootTransitionSlotRuntime {
            active: true,
            weight: 0.5,
            pose_ready: true,
        };
        let grounded = AlsGroundedGraphTelemetry {
            actual_state: Some(1),
            transition_active: Some(false),
            pose_ready: Some(true),
        };
        assert!(grounded_roll_exit_transition_matches_source(
            IntegrationPhase::Roll,
            Some(Stance::Standing),
            Some(&roll),
            Some(&playback),
            Some(&slot),
            Some(&grounded),
        ));
        assert!(!grounded_roll_exit_transition_matches_source(
            IntegrationPhase::Roll,
            Some(Stance::Standing),
            Some(&roll),
            Some(&playback),
            None,
            Some(&grounded),
        ));
        assert!(!grounded_roll_exit_transition_matches_source(
            IntegrationPhase::Locomotion,
            Some(Stance::Standing),
            Some(&roll),
            Some(&playback),
            Some(&slot),
            Some(&grounded),
        ));
        let blending = AlsGroundedGraphTelemetry {
            actual_state: None,
            transition_active: Some(true),
            ..grounded
        };
        assert!(grounded_roll_exit_transition_matches_source(
            IntegrationPhase::Roll,
            Some(Stance::Standing),
            Some(&roll),
            Some(&playback),
            Some(&slot),
            Some(&blending),
        ));
        let crouching = AlsTransitionPlayback::roll_to_grounded_right(true);
        let grounded = AlsGroundedGraphTelemetry {
            actual_state: Some(2),
            ..grounded
        };
        assert!(grounded_roll_exit_transition_matches_source(
            IntegrationPhase::Roll,
            Some(Stance::Crouching),
            Some(&roll),
            Some(&crouching),
            Some(&slot),
            Some(&grounded),
        ));
        let active_roll = AlsRollState {
            active: true,
            ..default()
        };
        assert!(grounded_roll_exit_transition_matches_source(
            IntegrationPhase::Roll,
            Some(Stance::Crouching),
            Some(&active_roll),
            Some(&crouching),
            Some(&slot),
            Some(&grounded),
        ));
        let crouch_to_stand = AlsGroundedGraphTelemetry {
            actual_state: Some(4),
            ..grounded
        };
        assert!(grounded_roll_exit_transition_matches_source(
            IntegrationPhase::Roll,
            Some(Stance::Crouching),
            Some(&active_roll),
            Some(&crouching),
            Some(&slot),
            Some(&crouch_to_stand),
        ));
        assert!(!grounded_roll_exit_transition_matches_source(
            IntegrationPhase::Roll,
            Some(Stance::Standing),
            Some(&active_roll),
            Some(&playback),
            Some(&slot),
            Some(&crouch_to_stand),
        ));
        assert!(!grounded_roll_exit_transition_matches_source(
            IntegrationPhase::Roll,
            Some(Stance::Standing),
            Some(&roll),
            Some(&crouching),
            Some(&slot),
            Some(&grounded),
        ));
        assert!(!grounded_roll_exit_transition_matches_source(
            IntegrationPhase::Roll,
            None,
            Some(&roll),
            Some(&crouching),
            Some(&slot),
            Some(&grounded),
        ));
    }

    #[test]
    fn fixture_rotation_reinitialize_clears_stale_response_state() {
        let mut rotation_response = AlsRotationResponseState {
            initialized: true,
            smooth_target_yaw: 1.25,
            previous_view_yaw: -0.75,
            ..Default::default()
        };
        let mut aiming_yaw_constraint = AlsAimingYawConstraint {
            active_limit_degrees: 70.0,
            applied_last_frame: true,
        };
        let mut rotation_animation = AlsRotationAnimationState {
            yaw_offset_degrees: 12.0,
            yaw_speed_degrees: 90.0,
        };
        reset_fixture_rotation_response_state(
            &mut rotation_response,
            &mut aiming_yaw_constraint,
            Some(&mut rotation_animation),
        );
        assert!(!rotation_response.initialized);
        assert_eq!(rotation_response.smooth_target_yaw, 0.0);
        assert_eq!(rotation_response.previous_view_yaw, 0.0);
        assert_eq!(aiming_yaw_constraint, AlsAimingYawConstraint::default());
        assert_eq!(rotation_animation, AlsRotationAnimationState::default());
    }

    #[test]
    fn slot_lifecycle_observer_runs_after_the_bridge_set() {
        let mut app = App::new();
        app.init_resource::<BridgeOrderProbe>()
            .configure_sets(Update, AlsTurnSlotBridgeSet)
            .add_systems(
                Update,
                (
                    publish_bridge_order_probe.in_set(AlsTurnSlotBridgeSet),
                    observe_bridge_order_probe.after(AlsTurnSlotBridgeSet),
                ),
            );
        app.update();
        assert_eq!(app.world().resource::<BridgeOrderProbe>().observed, 1);
    }

    #[test]
    fn direction_regression_covers_source_state_sequence_and_sprint_guards() {
        let cases: Vec<_> = (0..DIRECTION_CASE_COUNT)
            .map(direction_regression_case)
            .collect();
        assert!(cases.iter().all(Option::is_some));
        assert_eq!(
            cases[0].as_ref().unwrap().branch_mask,
            DIRECTION_BRANCH_FORWARD
        );
        assert_eq!(
            cases[1].as_ref().unwrap().branch_mask,
            DIRECTION_BRANCH_BACKWARD
        );
        assert_eq!(
            cases[2].as_ref().unwrap().branch_mask,
            DIRECTION_BRANCH_RIGHT
        );
        assert_eq!(
            cases[3].as_ref().unwrap().branch_mask,
            DIRECTION_BRANCH_LEFT
        );
        assert_eq!(
            cases[8].as_ref().unwrap().expected_actual_gait,
            Gait::Sprinting
        );
        assert_eq!(
            cases[9].as_ref().unwrap().expected_allowed_gait,
            Gait::Running
        );
        assert!(cases[9].as_ref().unwrap().first_person);
        assert!((cases[10].as_ref().unwrap().yaw_offset.to_degrees() - 90.0).abs() < 1e-6);
        assert!(cases[10].as_ref().unwrap().branch_from_actual_local);
    }

    #[test]
    fn automatic_input_owns_the_actor_before_startup_is_ready() {
        use crate::input::{AlsInputMap, AlsInputPlugin, spawn_als_actions};
        use bevy_als::als::view::AlsViewInputPlugin;
        use bevy::input::{InputPlugin, mouse::MouseMotion};
        use bevy_enhanced_input::prelude::EnhancedInputPlugin;
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            InputPlugin,
            EnhancedInputPlugin,
            AlsInputPlugin,
            AlsViewInputPlugin,
        ));
        app.init_resource::<ManualIntegrationState>()
            .init_resource::<ManualPlaygroundStartupReady>()
            .init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<StandardMaterial>>()
            .init_resource::<AlsFootIkTelemetryEnabled>()
            .init_resource::<AlsFootIkWeightOverride>();
        app.add_systems(
            Update,
            initialize_manual_integration.before(bevy_als::als::view::AlsViewInputSet),
        );
        app.finish();
        let yaw = -45.0_f32.to_radians();
        let actor = app
            .world_mut()
            .spawn((
                AlsCharacter,
                AlsViewYaw(yaw),
                AlsViewPitch::default(),
                Transform::from_rotation(Quat::from_rotation_y(yaw)),
            ))
            .id();
        spawn_als_actions(
            &mut app.world_mut().commands(),
            actor,
            &AlsInputMap::default(),
        );
        app.world_mut().write_message(MouseMotion {
            delta: Vec2::new(-59.53, 8.0),
        });
        app.world_mut().run_schedule(PreUpdate);
        app.world_mut().run_schedule(Update);
        assert_eq!(
            app.world().get::<AlsViewYaw>(actor).unwrap().0,
            yaw,
            "Live mouse input changed the automatic fixture before readiness"
        );
        assert_eq!(app.world().get::<AlsViewPitch>(actor).unwrap().0, 0.0);
        assert!(
            !app.world()
                .resource::<ManualPlaygroundStartupReady>()
                .passed
        );
        assert!(!app.world().resource::<ManualIntegrationState>().initialized);
    }

    #[test]
    fn direction_fixture_keeps_aiming_through_the_actual_aim_system() {
        let case = direction_regression_case(0).unwrap();
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_message::<bevy_als::als::aim::AimStateChanged>()
            .add_message::<bevy_als::als::transition::StateTransitionEvent>()
            .add_systems(Update, bevy_als::als::aim::aim_rotation_mode_system);
        let entity = app
            .world_mut()
            .spawn((
                AlsCharacter,
                crate::input::AlsInputCommand::default(),
                AlsInputOverride(case.command()),
                bevy_als::als::gait::AlsMaxAllowedGait(case.expected_allowed_gait),
                Stance::Standing,
                LocomotionState::Grounded,
                case.rotation_mode,
                bevy_als::als::aim::AlsLastFreeRotationMode(RotationMode::ViewDirection),
                AlsStateRequest {
                    rotation_mode: Some(case.rotation_mode),
                    ..default()
                },
            ))
            .id();
        app.update();
        assert_eq!(
            app.world()
                .get::<AlsStateRequest>(entity)
                .unwrap()
                .rotation_mode,
            Some(RotationMode::Aiming)
        );
    }

    #[test]
    fn a_new_mantle_window_cannot_reuse_an_earlier_completion() {
        let mut state = ManualIntegrationState::default();
        state.coverage.mantle_started = true;
        state.mantle_root_released = true;
        state.coverage.mantle_completed = true;
        begin_phase(&mut state, IntegrationPhase::Mantle);
        let mut window = ManualAnimationWindowEvidence::default();
        let mantle = AlsMantleState {
            active: true,
            root_motion_ready: true,
            target: Vec3::Y,
            ..default()
        };
        observe_mantle(
            &mut state,
            &mut window,
            &mantle,
            None,
            &AnimationGraphPlayer::new(Handle::default()),
            &Assets::default(),
            LocomotionState::Custom,
            Some(AlsNotifyLocomotionAction::Mantling),
            Vec3::Y,
        );
        assert!(
            window.observed_lifecycle_stages.is_empty(),
            "Earlier completion contaminated the new mantle window"
        );
    }

    #[test]
    fn mantle_observer_rejects_actor_only_completion() {
        let mut state = ManualIntegrationState::default();
        state.phase = IntegrationPhase::Mantle;
        let mut window = ManualAnimationWindowEvidence::default();
        let mut mantle = AlsMantleState {
            active: true,
            root_motion_ready: true,
            target: Vec3::Y,
            ..default()
        };
        let player = AnimationGraphPlayer::new(Handle::default());
        let graphs = Assets::default();
        observe_mantle(
            &mut state,
            &mut window,
            &mantle,
            None,
            &player,
            &graphs,
            LocomotionState::Custom,
            Some(AlsNotifyLocomotionAction::Mantling),
            Vec3::Y,
        );
        assert!(
            !state.coverage.mantle_started,
            "Actor state without an evaluated Slot cannot prove animation start"
        );
        assert!(
            window.expected_animation_variant.is_none(),
            "The source variant is unknown before its montage instance exists"
        );
        mantle.active = false;
        mantle.root_motion_ready = false;
        observe_mantle(
            &mut state,
            &mut window,
            &mantle,
            None,
            &player,
            &graphs,
            LocomotionState::Grounded,
            Some(AlsNotifyLocomotionAction::None),
            Vec3::Y,
        );
        assert!(
            !state.coverage.mantle_completed,
            "Actor height without animation lifecycle cannot prove completion"
        );
        state.coverage.mantle_started = true;
        mantle.root_motion_ready = true;
        observe_mantle(
            &mut state,
            &mut window,
            &mantle,
            None,
            &player,
            &graphs,
            LocomotionState::Grounded,
            Some(AlsNotifyLocomotionAction::None),
            Vec3::Y,
        );
        assert!(
            !state.coverage.mantle_completed,
            "Root motion ownership must be released"
        );
        mantle.root_motion_ready = false;
        observe_mantle(
            &mut state,
            &mut window,
            &mantle,
            None,
            &player,
            &graphs,
            LocomotionState::Grounded,
            Some(AlsNotifyLocomotionAction::Mantling),
            Vec3::Y,
        );
        assert!(
            !state.coverage.mantle_completed,
            "The source action notify must end"
        );
        observe_mantle(
            &mut state,
            &mut window,
            &mantle,
            None,
            &player,
            &graphs,
            LocomotionState::Grounded,
            Some(AlsNotifyLocomotionAction::None),
            Vec3::Y,
        );
        assert!(state.coverage.mantle_completed);
        assert_eq!(window.observed_lifecycle_stages, &["completed"]);
        assert_eq!(
            window.expected_lifecycle_stages,
            &["playing", "root_released", "montage_tail", "completed"]
        );
        assert!(
            window.requires_pre_rig_motion
                && window.requires_final_pose_motion
                && window.requires_source_pose_chain
        );
        assert!(window.source_sync_group.is_none() && window.expected_runtime_state.is_none());
    }

    #[test]
    fn foot_ik_gate_preconditions_follow_source_air_curves() {
        let mut apply = AlsFootIkApplyTelemetry {
            left: bevy_als::foot_ik_apply::AlsFootIkApplyTelemetrySide {
                valid: true,
                ..default()
            },
            right: bevy_als::foot_ik_apply::AlsFootIkApplyTelemetrySide {
                valid: true,
                ..default()
            },
        };
        assert!(
            foot_ik_matches_source(&apply, [0.0, 0.0]),
            "Air disables foot IK through the consumed source curves"
        );
        assert!(!foot_ik_matches_source(&apply, [1.0, 1.0]));
        apply.left.valid = false;
        assert!(!foot_ik_matches_source(&apply, [0.0, 0.0]));
        apply.left.valid = true;
        apply.right.effector_error = 0.03;
        assert!(!foot_ik_matches_source(&apply, [0.0, 0.0]));
    }

    #[test]
    fn ragdoll_driver_uses_high_landing_without_direct_start() {
        use bevy::ecs::system::RunSystemOnce;
        let (mut world, character, start) = manual_driver_fixture();
        let mut state = world.resource_mut::<ManualIntegrationState>();
        state.scenario = IntegrationScenario::Ragdoll;
        state.phase = IntegrationPhase::Ragdoll;
        state.entered = true;
        state.spawn_translation = Some(start);
        world.run_system_once(drive_manual_integration).unwrap();
        assert!(
            (world.get::<Position>(character).unwrap().y
                - start.y
                - ALS_CHARACTER_CAPSULE_HALF_HEIGHT
                - 9.0)
                .abs()
                < 1e-5,
            "Ragdoll fixture must reach the source high landing speed through an actual fall"
        );
        for delta in [0.0, 0.4, 1.0] {
            world
                .resource_mut::<Time<Virtual>>()
                .advance_by(Duration::from_secs_f32(delta));
            world.run_system_once(drive_manual_integration).unwrap();
            let request = world.get::<AlsStateRequest>(character).unwrap();
            assert!(
                !request.ragdoll_start,
                "Direct ragdoll start bypasses the source landing consumer"
            );
            assert!(
                request.locomotion_state.is_none(),
                "Grounded override suppresses the actual source fall"
            );
        }
    }

    #[test]
    fn get_up_actor_clock_requires_source_timing_and_the_actual_fixture_input() {
        let rows = |scale: f64, dilation: f64| {
            (0..8)
                .map(|index| {
                    serde_json::json!({
                        "get_up_actor_clock": true, "game_time": index as f64 * 0.02,
                        "elapsed": index as f64 * 0.02 * scale, "dilation": dilation
                    })
                })
                .collect::<Vec<_>>()
        };
        assert!(actor_montage_clock_passed(
            &rows(0.5, 0.5),
            "get_up_actor_clock"
        ));
        assert!(!actor_montage_clock_passed(
            &rows(1.0, 0.5),
            "get_up_actor_clock"
        ));
        assert!(!actor_montage_clock_passed(
            &rows(0.5, 1.0),
            "get_up_actor_clock"
        ));
        assert!(!actor_montage_clock_passed(&[], "get_up_actor_clock"));
    }

    #[test]
    fn fresh_audit_mantle_driver_limits_owner_clock_to_the_base_mantle_phase() {
        use bevy::ecs::system::RunSystemOnce;
        let (mut world, character, _) = manual_driver_fixture();
        {
            let mut state = world.resource_mut::<ManualIntegrationState>();
            state.scenario = IntegrationScenario::Base;
            state.phase = IntegrationPhase::Mantle;
            state.entered = true;
            state.phase_started_seconds = 0.0;
        }
        world.run_system_once(drive_manual_integration).unwrap();
        assert_eq!(
            world
                .get::<bevy_als::als_camera::AlsCameraCustomTimeDilation>(character)
                .unwrap()
                .0,
            0.5
        );
        world
            .resource_mut::<ManualIntegrationState>()
            .phase_started_seconds = -10.1;
        world.run_system_once(drive_manual_integration).unwrap();
        assert_eq!(
            world.resource::<ManualIntegrationState>().phase,
            IntegrationPhase::Platform
        );
        assert!(
            world
                .get::<bevy_als::als_camera::AlsCameraCustomTimeDilation>(character)
                .is_none()
        );
        let rows = (0..8)
            .map(|index| {
                serde_json::json!({
                    "mantle_actor_clock": true, "game_time": index as f64 * 0.02,
                    "elapsed": index as f64 * 0.01, "dilation": 0.5
                })
            })
            .collect::<Vec<_>>();
        assert!(actor_montage_clock_passed(&rows, "mantle_actor_clock"));
        assert!(!actor_montage_clock_passed(&rows, "get_up_actor_clock"));
    }

    #[test]
    fn fresh_audit_roll_driver_limits_owner_clock_to_the_roll_phase() {
        use bevy::ecs::system::RunSystemOnce;
        let (mut world, character, _) = manual_driver_fixture();
        {
            let mut state = world.resource_mut::<ManualIntegrationState>();
            state.scenario = IntegrationScenario::Grounded;
            state.phase = IntegrationPhase::Roll;
            state.entered = false;
            state.phase_started_seconds = -1.1;
        }
        world.run_system_once(drive_manual_integration).unwrap();
        assert_eq!(
            world
                .get::<bevy_als::als_camera::AlsCameraCustomTimeDilation>(character)
                .unwrap()
                .0,
            0.5
        );
        world
            .resource_mut::<ManualIntegrationState>()
            .phase_started_seconds = -(FOOT_AVATAR_REPLACEMENT_START_SECONDS as f64 + 0.1);
        world.run_system_once(drive_manual_integration).unwrap();
        assert_eq!(
            world.resource::<ManualIntegrationState>().phase,
            IntegrationPhase::Roll,
            "Roll driver ended before the source transition completed"
        );
        world
            .resource_mut::<ManualIntegrationState>()
            .coverage
            .grounded_roll_completed = true;
        world.run_system_once(drive_manual_integration).unwrap();
        assert_eq!(
            world.resource::<ManualIntegrationState>().phase,
            IntegrationPhase::GroundedJoint
        );
        assert!(
            world
                .get::<bevy_als::als_camera::AlsCameraCustomTimeDilation>(character)
                .is_none()
        );
        let rows = (0..8)
            .map(|index| {
                serde_json::json!({
                    "roll_actor_clock": true, "game_time": index as f64 * 0.02,
                    "elapsed": index as f64 * 0.01, "dilation": 0.5
                })
            })
            .collect::<Vec<_>>();
        assert!(actor_montage_clock_passed(&rows, "roll_actor_clock"));
        assert!(!actor_montage_clock_passed(&rows, "get_up_actor_clock"));
        assert!(!grounded_roll_exit_timed_out(19.9, false));
        assert!(grounded_roll_exit_timed_out(20.0, false));
        assert!(!grounded_roll_exit_timed_out(21.0, true));
    }

    #[test]
    fn fresh_audit_get_up_driver_uses_owner_clock_and_waits_for_natural_end() {
        use bevy::ecs::system::RunSystemOnce;
        let (mut world, character, _) = manual_driver_fixture();
        {
            let mut state = world.resource_mut::<ManualIntegrationState>();
            state.scenario = IntegrationScenario::GetUp;
            state.phase = IntegrationPhase::Ragdoll;
            state.entered = true;
        }
        world.run_system_once(drive_manual_integration).unwrap();
        assert_eq!(
            world
                .get::<bevy_als::als_camera::AlsCameraCustomTimeDilation>(character)
                .unwrap()
                .0,
            0.5
        );
        world
            .entity_mut(character)
            .insert(bevy_als::get_up::AlsGetUpState::new(false));
        {
            let mut state = world.resource_mut::<ManualIntegrationState>();
            state.entered = false;
            state.phase_started_seconds = -4.0;
            state.ragdoll_recovered_at = Some(-3.0);
            state.ragdoll_exit_requested = true;
        }
        world.run_system_once(drive_manual_integration).unwrap();
        assert_eq!(
            world.resource::<ManualIntegrationState>().get_up_case,
            0,
            "Get-up driver must retain the current montage until its natural end"
        );
        world
            .entity_mut(character)
            .remove::<bevy_als::get_up::AlsGetUpState>();
        world.run_system_once(drive_manual_integration).unwrap();
        assert_eq!(world.resource::<ManualIntegrationState>().get_up_case, 1);
        world.run_system_once(drive_manual_integration).unwrap();
        assert!(
            world
                .get::<bevy_als::als_camera::AlsCameraCustomTimeDilation>(character)
                .is_none()
        );
    }

    #[test]
    fn get_up_driver_exits_ragdoll_through_source_jump_chain() {
        use bevy::ecs::system::RunSystemOnce;
        let (mut world, character, _) = manual_driver_fixture();
        world.entity_mut(character).insert(AlsRagdollGroundFollow {
            grounded: true,
            ..default()
        });
        let mut state = world.resource_mut::<ManualIntegrationState>();
        state.scenario = IntegrationScenario::GetUp;
        state.phase = IntegrationPhase::Ragdoll;
        state.entered = false;
        state.phase_started_seconds = -1.0;
        state.coverage.ragdoll_follow = true;
        state.coverage.ragdoll_joint_drive = true;
        world.run_system_once(drive_manual_integration).unwrap();
        assert!(
            world
                .get::<AlsInputOverride>(character)
                .unwrap()
                .0
                .jump_pressed,
            "Source GetUp fixture must exercise Jump stopRagdolling branch"
        );
        assert!(
            !world
                .get::<AlsStateRequest>(character)
                .unwrap()
                .ragdoll_exit,
            "Direct exit request bypasses the source Jump priority consumer"
        );
    }

    #[test]
    fn get_up_driver_requests_source_jump_once_after_pose_window_while_locked() {
        use bevy::ecs::system::RunSystemOnce;
        let (mut world, character, _) = manual_driver_fixture();
        let mut state = world.resource_mut::<ManualIntegrationState>();
        state.scenario = IntegrationScenario::GetUp;
        state.phase = IntegrationPhase::Ragdoll;
        state.entered = false;
        state.get_up_case = 1;
        state.ragdoll_exit_requested = true;
        for (clip_time, ready, blocked, expected) in [
            (0.8, true, true, false),
            (0.91, false, true, false),
            (0.91, true, false, false),
            (0.91, true, true, true),
            (1.0, true, true, false),
        ] {
            let mut get_up = bevy_als::get_up::AlsGetUpState::new(true);
            get_up.clip_time = clip_time;
            get_up.pose_ready = ready;
            world.entity_mut(character).insert(get_up);
            if blocked {
                world
                    .entity_mut(character)
                    .insert(bevy_als::als::motion::AlsMovementInputBlocked);
            } else {
                world
                    .entity_mut(character)
                    .remove::<bevy_als::als::motion::AlsMovementInputBlocked>();
            }
            world
                .resource_mut::<Time<Virtual>>()
                .advance_by(Duration::from_secs_f32(0.02));
            world.run_system_once(drive_manual_integration).unwrap();
            assert_eq!(
                world
                    .get::<AlsInputOverride>(character)
                    .unwrap()
                    .0
                    .jump_pressed,
                expected,
                "Source GetUp jump driver readiness or repeated pulse differs: clip={clip_time} ready={ready} blocked={blocked}"
            );
        }
        assert!(
            world
                .resource::<ManualIntegrationState>()
                .get_up_jump_requested_at
                .is_some()
        );
    }

    #[test]
    fn camera_probe_stays_stable_and_third_person_until_collision_recovers() {
        use bevy::ecs::system::RunSystemOnce;
        let (mut world, character, _) = manual_driver_fixture();
        let camera = world
            .spawn((
                AlsCameraRig::default(),
                AlsCameraDebugInfo {
                    valid: true,
                    trace_start: Vec3::Y * 1.6,
                    trace_end: Vec3::new(0.0, 1.57, 2.8),
                    ..default()
                },
            ))
            .id();
        for (elapsed, recovered, movement_passed, expected_look, expected_switch) in [
            (0.8, false, false, false, false),
            (2.5, false, false, false, false),
            (1.0, true, false, true, false),
            (2.5, true, false, true, false),
            (2.5, true, true, false, true),
        ] {
            let mut state = ManualIntegrationState {
                character: Some(character),
                camera: Some(camera),
                initialized: true,
                entered: false,
                scenario: IntegrationScenario::Camera,
                phase: IntegrationPhase::Camera,
                phase_started_seconds: -elapsed,
                ..default()
            };
            state.coverage.camera_recovery = recovered;
            state.camera_obstacle_ignored = recovered;
            state.camera_movement.samples = usize::from(movement_passed) * 8;
            state.camera_movement.lagged = movement_passed;
            world.insert_resource(state);
            world.run_system_once(drive_manual_integration).unwrap();
            let input = &world.get::<AlsInputOverride>(character).unwrap().0;
            assert_eq!(
                input.look_delta != Vec2::ZERO,
                expected_look,
                "elapsed={elapsed} recovered={recovered}"
            );
            assert_eq!(
                input.view_mode_pressed, expected_switch,
                "elapsed={elapsed} recovered={recovered}"
            );
            if !recovered {
                assert_eq!(
                    world
                        .resource::<ManualIntegrationState>()
                        .camera_obstacles
                        .iter()
                        .flatten()
                        .count(),
                    3
                );
            }
        }
        for (blocking, first_person_override) in [(true, 0.0), (false, 1.0)] {
            {
                let mut debug = world.get_mut::<AlsCameraDebugInfo>(camera).unwrap();
                debug.trace_blocking_hit = blocking;
                debug.first_person_override = first_person_override;
            }
            world.insert_resource(ManualIntegrationState {
                character: Some(character),
                camera: Some(camera),
                initialized: true,
                entered: false,
                scenario: IntegrationScenario::Camera,
                phase: IntegrationPhase::Camera,
                phase_started_seconds: -0.8,
                ..default()
            });
            world.run_system_once(drive_manual_integration).unwrap();
            assert!(
                world
                    .resource::<ManualIntegrationState>()
                    .camera_obstacles
                    .iter()
                    .all(Option::is_none)
            );
        }
    }

    #[test]
    fn mantle_fixture_preserves_world_pose_and_cleans_its_parent() {
        use bevy::ecs::system::RunSystemOnce;
        let (mut world, character, _) = manual_driver_fixture();
        let mut mantle = AlsMantleState::default();
        mantle.active = true;
        mantle.root_motion_ready = true;
        let before = Transform::from_xyz(1.0, 2.0, 3.0).with_rotation(Quat::from_rotation_y(0.2));
        world.entity_mut(character).insert((
            mantle,
            before,
            Position(before.translation),
            Rotation(before.rotation),
        ));
        world.insert_resource(ManualIntegrationState {
            character: Some(character),
            initialized: true,
            entered: false,
            scenario: IntegrationScenario::Base,
            phase: IntegrationPhase::Mantle,
            ..default()
        });
        world.run_system_once(drive_manual_integration).unwrap();
        let parent = world.get::<ChildOf>(character).unwrap().parent();
        let current = world
            .run_system_once(move |helper: bevy::transform::helper::TransformHelper| {
                helper.compute_global_transform(character).unwrap()
            })
            .unwrap();
        assert!(current.translation().distance(before.translation) < 1e-5);
        assert!(current.rotation().abs_diff_eq(before.rotation, 1e-5));
        assert!(
            current
                .translation()
                .distance(world.get::<Transform>(character).unwrap().translation)
                > 0.5
        );
        world.get_mut::<AlsMantleState>(character).unwrap().active = false;
        world.run_system_once(drive_manual_integration).unwrap();
        assert!(world.get::<ChildOf>(character).is_none());
        assert!(world.get_entity(parent).is_err());
        assert!(
            world
                .get::<Transform>(character)
                .unwrap()
                .translation
                .distance(before.translation)
                < 1e-5
        );
        assert!(
            world
                .get::<Transform>(character)
                .unwrap()
                .rotation
                .abs_diff_eq(before.rotation, 1e-5)
        );
        assert_eq!(
            world.get::<Position>(character).unwrap().0,
            before.translation
        );
    }

    #[test]
    fn mantle_fixture_waits_for_the_player_view_before_approaching() {
        use bevy::ecs::system::RunSystemOnce;
        let (mut world, character, _) = manual_driver_fixture();
        for (yaw, expected) in [(1.0, Vec2::ZERO), (0.0, Vec2::Y)] {
            world
                .entity_mut(character)
                .insert(bevy_als::als::view::AlsPlayerViewYaw(yaw));
            world.insert_resource(ManualIntegrationState {
                character: Some(character),
                initialized: true,
                entered: false,
                scenario: IntegrationScenario::Base,
                phase: IntegrationPhase::Mantle,
                ..default()
            });
            world.run_system_once(drive_manual_integration).unwrap();
            assert_eq!(
                world.get::<AlsInputOverride>(character).unwrap().0.move_dir,
                expected
            );
        }
    }

    fn manual_driver_fixture() -> (World, Entity, Vec3) {
        let mut world = World::new();
        world.init_resource::<ManualAnimationWindowEvidence>();
        world.init_resource::<ManualRuntimePhaseEvidence>();
        world.init_resource::<Time<Virtual>>();
        world.init_resource::<ButtonInput<KeyCode>>();
        world.init_resource::<AlsShowDebugHudState>();
        world.init_resource::<Messages<AlsCameraShakeRequest>>();
        world.init_resource::<Messages<AlsPivotNotify>>();
        world.init_resource::<AlsFootIkWeightOverride>();
        world.init_resource::<Assets<AlsMovementParams>>();
        let character = world
            .spawn((
                AlsCharacter,
                AlsInputOverride::default(),
                Transform::IDENTITY,
                Position::default(),
                LinearVelocity::default(),
                AlsStateRequest::default(),
                Stance::Standing,
                LocomotionState::Grounded,
                Gait::Running,
                OverlayState::Default,
                RotationMode::ViewDirection,
                AlsViewYaw::default(),
                AlsViewPitch::default(),
            ))
            .insert((
                AlsMovementParamsHandle(Handle::default()),
                AlsCrouchTransaction::default(),
                AlsAimingYawConstraint::default(),
            ))
            .id();
        let start = Vec3::new(1.0, 0.9, 2.0);
        world.insert_resource(ManualIntegrationFixture {
            platform: Entity::PLACEHOLDER,
            platform_home: Vec3::ZERO,
            locomotion_start: start,
            mantle_start: Vec3::ZERO,
            roll_start: Vec3::ZERO,
            facing_yaw: 0.0,
            obstacle_mesh: Handle::default(),
            obstacle_material: Handle::default(),
        });
        world.insert_resource(ManualIntegrationState {
            character: Some(character),
            initialized: true,
            entered: true,
            scenario: IntegrationScenario::FootIk,
            phase: IntegrationPhase::FootIkTeleport,
            ..default()
        });
        (world, character, start)
    }

    #[test]
    fn foot_ik_gate_preconditions_keep_the_teleport_driver_idle() {
        use bevy::ecs::system::RunSystemOnce;
        let (mut world, character, start) = manual_driver_fixture();
        for (delta, teleported) in [(0.0, false), (0.5, false), (0.6, true), (0.4, true)] {
            world
                .resource_mut::<Time<Virtual>>()
                .advance_by(Duration::from_secs_f32(delta));
            world.run_system_once(drive_manual_integration).unwrap();
            assert_eq!(
                world.get::<AlsInputOverride>(character).unwrap().0.move_dir,
                Vec2::ZERO,
                "Running releases the source foot locks before teleport"
            );
            assert_eq!(
                world
                    .resource::<ManualIntegrationState>()
                    .foot_ik_teleport_requested,
                teleported
            );
            let expected = start
                + if teleported {
                    Vec3::X * 0.6
                } else {
                    Vec3::ZERO
                };
            assert!(
                world
                    .get::<Position>(character)
                    .unwrap()
                    .0
                    .distance(expected)
                    < 1e-6
            );
        }
    }

    #[test]
    fn foot_ik_gate_preconditions_reject_air_in_standing_windows() {
        let standing = AlsStandingGraphTelemetry {
            actual_movement_state: Some(3),
            actual_standing_state: Some(1),
            actual_standing_transition_active: Some(false),
            ..default()
        };
        for locomotion in [
            LocomotionState::Grounded,
            LocomotionState::Air,
            LocomotionState::Custom,
        ] {
            let mut window = state_machine_grounded_window(1.0);
            observe_standing_window(&mut window, Some(&standing), locomotion);
            assert_eq!(
                window.settled,
                locomotion == LocomotionState::Grounded,
                "Old Standing telemetry cannot make a current Air frame a Grounded sample"
            );
            assert_eq!(window.observed_runtime_state, Some(3));
            assert_eq!(window.observed_parent_runtime_state, Some(1));
            assert!(window.capture_source_window && window.requires_source_pose_chain);
        }
        let mut entering = state_machine_grounded_window(0.0);
        observe_standing_window(&mut entering, Some(&standing), LocomotionState::Grounded);
        assert!(!entering.settled);
    }

    #[test]
    fn mantle_foot_ik_follows_source_curve_weights() {
        let source: serde_json::Value = serde_json::from_str(include_str!(
            "../../../assets/ALS/Animations/Actions/Mantle/AM_Als_Mantle_Low.montage.json"
        ))
        .unwrap();
        let curves: bevy_als::extensions::AlsPoseCurves =
            serde_json::from_value(source["playback"]["montage_curves"].clone()).unwrap();
        let curves = curves.sample_all(0.8);
        let weights = [
            curves[&bevy_als::extensions::SourceCurveName::from("FootLeftIk")],
            curves[&bevy_als::extensions::SourceCurveName::from("FootRightIk")],
        ];
        assert_eq!(weights, [0.0, 1.0]);
        let mut apply = AlsFootIkApplyTelemetry {
            left: bevy_als::foot_ik_apply::AlsFootIkApplyTelemetrySide {
                valid: true,
                applied_weight: weights[0],
                ..default()
            },
            right: bevy_als::foot_ik_apply::AlsFootIkApplyTelemetrySide {
                valid: true,
                applied_weight: weights[1],
                ..default()
            },
        };
        assert!(
            mantle_foot_ik_matches_source(&apply, weights, Some(1.0)),
            "Source Low Mantle has asymmetric foot IK during its action"
        );
        for weight in [None, Some(0.4)] {
            assert!(
                !mantle_foot_ik_matches_source(&apply, weights, weight),
                "A pending or blending Slot cannot prove mantle IK"
            );
        }
        apply.left.applied_weight = 1.0;
        assert!(
            !mantle_foot_ik_matches_source(&apply, weights, Some(1.0)),
            "Both feet at full weight do not match the source action"
        );
        apply.left.applied_weight = 0.0;
        apply.left.valid = false;
        assert!(
            !mantle_foot_ik_matches_source(&apply, weights, Some(1.0)),
            "Missing Rig output must not pass at zero weight"
        );
        apply.left.valid = true;
        apply.right.effector_error = 0.03;
        assert!(
            !mantle_foot_ik_matches_source(&apply, weights, Some(1.0)),
            "Bone output must match the actual Rig result"
        );
    }

    #[test]
    fn mantle_observer_reads_only_the_current_authored_slot_output() {
        use bevy::ecs::system::RunSystemOnce;
        use bevy::platform::collections::HashMap;
        use bevy_animation_graph::core::{
            animation_clip::{EntityPath, GraphClip},
            animation_graph::TimeUpdate,
            animation_node::AnimationNode,
            context::{
                deferred_gizmos::DeferredGizmos, graph_context::QueryOutputTime,
                graph_context_arena::GraphContextArena, new_context::GraphContext,
                system_resources::SystemResources,
            },
            edge_data::DataSpec,
            pose::BonePose,
            utils::testing::TestIoEnv,
        };
        fn evaluate(
            In((handle, entity, pose, weight)): In<(Handle<AnimationGraph>, Entity, Pose, f32)>,
            resources: SystemResources,
            source_inputs: Query<&bevy_als::extensions::SourceAnimationInputs>,
        ) -> GraphContextArena {
            let graph = resources.animation_graph_assets.get(&handle).unwrap();
            let mut arena = GraphContextArena::new(handle.id());
            let inputs = HashMap::from([
                ("base".into(), DataValue::Pose(pose.clone())),
                ("post_locomotion.pose".into(), DataValue::Pose(pose)),
                ("post_locomotion.weight".into(), DataValue::F32(weight)),
                ("post_locomotion.layer_weight".into(), DataValue::F32(1.0)),
            ]);
            let mut gizmos = DeferredGizmos::default();
            let entities = HashMap::default();
            let env = TestIoEnv::default();
            let context = GraphContext::new(
                arena.get_toplevel_id(),
                graph,
                &mut arena,
                &resources,
                &env,
                entity,
                &entities,
                &mut gizmos,
                &inputs,
            );
            source_inputs
                .get(entity)
                .unwrap()
                .evaluate(entity, || {
                    graph.query_with_context(
                        QueryOutputTime::Forced(TimeUpdate::Delta(0.1)),
                        context,
                    )
                })
                .unwrap();
            arena
        }
        let installed = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("assets/ALS/Character/AB_Als/AnimGraph.animgraph.ron"),
        )
        .unwrap();
        assert!(installed.contains(&format!("name: {MANTLE_SLOT_NODE:?}")));
        let mut world = World::new();
        let entity = world.spawn_empty().id();
        let base = AnimationNode::new(
            "base",
            bevy_als::extensions::GlobalPoseInputNode {
                key: "base".into(),
                output: "Pose".into(),
            },
        );
        let slot = AnimationNode::new(
            MANTLE_SLOT_NODE,
            bevy_als::extensions::SourceSlotNode {
                slot_name: "PostLocomotion".into(),
                group_name: "Locomotion".into(),
                pose_key: "post_locomotion.pose".into(),
                weight_key: "post_locomotion.weight".into(),
                layer_weight_key: "post_locomotion.layer_weight".into(),
                ..default()
            },
        );
        let mut graph = AnimationGraph::new();
        graph.add_node(base.clone());
        graph.add_node(slot.clone());
        graph.add_node_parameter_edge(base.id, "Pose", slot.id, "Source");
        graph.add_output_data("Pose".into(), DataSpec::Pose);
        graph.add_output_data_edge(slot.id, "Pose", "Pose");
        let mut graphs = Assets::<AnimationGraph>::default();
        let handle = graphs.add(graph);
        let linked = AnimationNode::new(
            "AB_Als",
            bevy_als::extensions::LayeredGraphNode {
                capture_context: true,
                internalize_time: true,
                ..bevy_als::extensions::LayeredGraphNode::new(handle.clone())
            },
        );
        let mut wrapper = AnimationGraph::new();
        wrapper.add_node(linked.clone());
        wrapper.add_output_data("pose".into(), DataSpec::Pose);
        wrapper.add_output_data_edge(linked.id, "pose", "pose");
        let wrapped_handle = graphs.add(wrapper);
        world.insert_resource(graphs);
        world.insert_resource(Assets::<GraphClip>::default());
        world.insert_resource(Assets::<Skeleton>::default());
        world.insert_resource(Assets::<StateMachine>::default());
        let mut pose = Pose::default();
        pose.add_bone(
            BonePose {
                rotation: Some(Quat::from_rotation_x(0.4)),
                ..default()
            },
            EntityPath::default().child("pelvis").id(),
        );
        world
            .entity_mut(entity)
            .insert(bevy_als::extensions::SourceAnimationInputs::default());
        let mut slot_context = bevy_als::extensions::AnimationContextState::default();
        slot_context.relevant_asset = Some(bevy_als::extensions::animation_context::SourceRelevantAsset {
            position: 0.7,
            duration: 1.5,
            weight: 1.0,
            ..default()
        });
        world
            .get_mut::<bevy_als::extensions::SourceAnimationInputs>(entity)
            .unwrap()
            .set_slot_input_context("post_locomotion.pose", Some(slot_context));
        assert!(
            mantle_slot_sample(&GraphContextArena::new(handle.id()), world.resource()).is_none()
        );
        for (candidate, weight) in [&handle, &wrapped_handle]
            .into_iter()
            .flat_map(|candidate| [1.0, 0.4, 0.0].map(|weight| (candidate, weight)))
        {
            let mut arena = world
                .run_system_once_with(evaluate, (candidate.clone(), entity, pose.clone(), weight))
                .unwrap();
            let sample = mantle_slot_sample(&arena, world.resource());
            assert_eq!(sample.is_some(), weight > 0.0);
            if let Some(sample) = sample {
                assert_eq!(
                    (sample.position, sample.duration, sample.weight),
                    (0.7, 1.5, weight)
                );
            }
            arena.next_frame();
            assert!(
                mantle_slot_sample(&arena, world.resource()).is_none(),
                "A stale evaluated Slot cannot prove this frame"
            );
        }
        let arena = world
            .run_system_once_with(evaluate, (handle.clone(), entity, Pose::default(), 1.0))
            .unwrap();
        assert!(
            mantle_slot_sample(&arena, world.resource()).is_none(),
            "A zero-channel Slot output cannot prove animation"
        );
        world
            .resource_mut::<Assets<AnimationGraph>>()
            .get_mut(&handle)
            .unwrap()
            .nodes
            .get_mut(&slot.id)
            .unwrap()
            .name = "PostLocomotion".into();
        let arena = world
            .run_system_once_with(evaluate, (handle, entity, pose, 1.0))
            .unwrap();
        assert!(
            mantle_slot_sample(&arena, world.resource()).is_none(),
            "A shorthand must not match the authored Slot identity"
        );
        world
            .get_mut::<bevy_als::extensions::SourceAnimationInputs>(entity)
            .unwrap()
            .set_slot_input_context("post_locomotion.pose", None);
    }

    #[test]
    fn generated_standing_pose_chain_uses_the_installed_closure() {
        let chain = generated_standing_source_pose_chain();
        assert_eq!(chain.len(), 9);
        assert_eq!(chain[0].label, "standing_movement_cache");
        assert_eq!(
            chain[0].graph,
            "ALS/Character/AnimationInstances/Stances/AB_Als_Standing/AnimGraph.animgraph.ron"
        );
        assert_eq!(chain[0].node, "Save cached pose 'Movement'");
        assert_eq!(chain[1].node, "Movement Details States\nState Machine");
        assert_eq!(chain[2].node, "Movement States\nState Machine");
        assert_eq!(chain[3].node, "Standing States\nState Machine");
        assert!(
            chain
                .iter()
                .all(|boundary| boundary.node != "Stop States\nState Machine")
        );
        assert_eq!(chain[4].node, "Apply Additive");
        assert_eq!(chain[7].node, "Save cached pose 'Standing'");
        assert_eq!(chain[8].node, "Grounded States\nState Machine");
    }

    #[test]
    fn response_timeline_reverses_without_release_then_waits_for_idle() {
        for (time, expected) in [
            (1.5, Vec2::NEG_X),
            (3.49, Vec2::NEG_X),
            (3.5, Vec2::X),
            (5.49, Vec2::X),
            (5.5, Vec2::Y),
            (5.75, Vec2::NEG_X),
            (6.0, Vec2::NEG_Y),
            (6.25, Vec2::X),
            (8.5, Vec2::ZERO),
        ] {
            assert_eq!(standing_response_step(time).1, expected);
        }
        assert_eq!(standing_response_step(11.5).0, "response_idle_after");
        assert_eq!(
            initial_phase(IntegrationScenario::StandingResponse),
            IntegrationPhase::StandingResponse
        );
    }

    #[test]
    fn movement_pose_reader_keeps_the_actual_leg_transform() {
        use bevy_animation_graph::core::{animation_clip::EntityPath, pose::BonePose};
        let path = EntityPath::default().child("root").child("thigh_r");
        let id = path.id();
        let mut skeleton = Skeleton::default();
        skeleton.set_root(id);
        skeleton.add_bone(path, Transform::IDENTITY, Transform::IDENTITY);
        let mut skeletons = Assets::<Skeleton>::default();
        let mut pose = Pose {
            skeleton: skeletons.add(skeleton),
            ..default()
        };
        pose.add_bone(
            BonePose::from_transform(Transform::from_translation(Vec3::X)),
            id,
        );
        let sampled = movement_pose_sample(&pose, &skeletons);
        assert_eq!(sampled.len(), 1);
        assert_eq!(sampled[0]["name"], "thigh_r");
        assert_eq!(
            sampled[0]["translation"],
            serde_json::json!([1.0, 0.0, 0.0])
        );
        assert_eq!(
            sampled[0]["rotation"],
            serde_json::json!([0.0, 0.0, 0.0, 1.0])
        );
    }

    #[test]
    fn sprint_and_grounded_fixtures_have_continuous_original_floor_without_obstacles() {
        use crate::playground::{
            PlaygroundActor, ue_extents_to_bevy, ue_point_to_bevy,
        };
        let actors: Vec<PlaygroundActor> = serde_json::from_str(include_str!(
            "../../../assets/ALSExtras/Levels/L_Als_Playground/playground.json"
        ))
        .unwrap();
        use bevy::ecs::system::RunSystemOnce;
        let start = actors
            .iter()
            .find(|actor| actor.class == "PlayerStart")
            .unwrap();
        let origin = ue_point_to_bevy(Vec3::from(start.location));
        let yaw = crate::playground::ue_yaw_to_bevy_quat(start.rotation_yaw)
            .to_euler(EulerRot::YXZ)
            .0;
        let mut world = World::new();
        world.init_resource::<Assets<Mesh>>();
        world.init_resource::<Assets<StandardMaterial>>();
        world
            .run_system_once(
                move |mut commands: Commands,
                      mut meshes: ResMut<Assets<Mesh>>,
                      mut materials: ResMut<Assets<StandardMaterial>>| {
                    spawn_manual_integration_fixture(
                        &mut commands,
                        &mut meshes,
                        &mut materials,
                        origin,
                        yaw,
                        IntegrationScenario::Base,
                    );
                },
            )
            .unwrap();
        let extra_bounds: Vec<(Vec3, Vec3)> = world
            .query::<(&Mesh3d, &Transform)>()
            .iter(&world)
            .map(|(mesh, transform)| {
                let meshes = world.resource::<Assets<Mesh>>();
                let points = meshes
                    .get(&mesh.0)
                    .unwrap()
                    .attribute(Mesh::ATTRIBUTE_POSITION)
                    .unwrap()
                    .as_float3()
                    .unwrap();
                points
                    .iter()
                    .map(|point| transform.transform_point(Vec3::from(*point)))
                    .fold(
                        (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
                        |(min, max), point| (min.min(point), max.max(point)),
                    )
            })
            .collect();
        let (mut driver, character, _) = manual_driver_fixture();
        driver
            .resource_mut::<ManualIntegrationFixture>()
            .locomotion_start = world
            .resource::<ManualIntegrationFixture>()
            .locomotion_start;
        driver.resource_mut::<ManualIntegrationFixture>().facing_yaw = yaw;
        driver.insert_resource(ManualIntegrationState {
            character: Some(character),
            initialized: true,
            entered: false,
            phase: IntegrationPhase::DirectionRegression,
            phase_started_seconds: -10.0 * DIRECTION_CASE_SECONDS as f64,
            ..default()
        });
        driver.run_system_once(drive_manual_integration).unwrap();
        let rotated_start = driver.get::<Position>(character).unwrap().0;
        let case = direction_regression_case(10).unwrap();
        let direction =
            bevy_als::als::motion::view_relative_move_direction(case.move_dir, yaw + case.yaw_offset);
        let distance = AlsMovementParams::default().speeds.standing.max_run_speed
            * (DIRECTION_CASE_SECONDS
                + driver.resource::<Time<Virtual>>().max_delta().as_secs_f32());
        let rotated_end = rotated_start + direction * distance;
        let sprint_end =
            SPRINT_START + Quat::from_rotation_y(SPRINT_YAW) * Vec3::NEG_Z * (6.5 * 8.5);
        for (min, max) in [
            (
                SPRINT_START.min(sprint_end) - Vec3::splat(0.35),
                SPRINT_START.max(sprint_end) + Vec3::splat(0.35),
            ),
            (
                Vec3::new(-45.35, 0.0, -26.35),
                Vec3::new(-23.0, 0.0, -25.65),
            ),
            (Vec3::new(-26.0, 0.0, -28.5), Vec3::new(-6.0, 0.0, -21.0)),
            (
                rotated_start.min(rotated_end) - Vec3::new(0.35, 0.0, 0.35),
                rotated_start.max(rotated_end) + Vec3::new(0.35, 0.0, 0.35),
            ),
        ] {
            for (lower, upper) in &extra_bounds {
                assert!(
                    upper.x < min.x || lower.x > max.x || upper.z < min.z || lower.z > max.z,
                    "Movement path intersects an attached integration fixture"
                );
            }
            let mut floors = Vec::new();
            for actor in &actors {
                if actor.mesh_path.is_none()
                    && actor
                        .components
                        .iter()
                        .all(|component| component.mesh_path.is_none())
                {
                    continue;
                }
                let (Some(origin), Some(extent)) = (actor.bounds_origin, actor.bounds_extent)
                else {
                    continue;
                };
                let center = ue_point_to_bevy(Vec3::from(origin));
                let extent = ue_extents_to_bevy(Vec3::from(extent));
                let lower = center - extent;
                let upper = center + extent;
                if upper.y.abs() < 0.01 {
                    floors.push((lower, upper));
                } else if upper.y > 0.01 && lower.y < 1.9 {
                    assert!(
                        upper.x < min.x || lower.x > max.x || upper.z < min.z || lower.z > max.z,
                        "Movement path intersects {}",
                        actor.name
                    );
                }
            }
            for x in 0..=240 {
                for z in 0..=32 {
                    let point =
                        min + (max - min) * Vec3::new(x as f32 / 240.0, 0.0, z as f32 / 32.0);
                    assert!(
                        floors.iter().any(|(min, max)| point.x >= min.x
                            && point.x <= max.x
                            && point.z >= min.z
                            && point.z <= max.z),
                        "Movement path leaves the original floor: {point:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn sprint_windows_follow_native_acceleration_and_share_the_actual_sync_phase() {
        let telemetry = AlsStandingGraphTelemetry {
            actual_movement_state: Some(0),
            actual_standing_state: Some(1),
            ..default()
        };
        let start = sprint_window(true, 0.37, &telemetry);
        let steady = sprint_window(false, 0.37, &telemetry);
        assert_eq!(start.label, Some("sprint_acceleration"));
        assert!(start.capture_every_frame);
        assert_eq!(
            start.source_pose_chain[0].node,
            "A_Als_Sprint_Acceleration\nSequence Player\nSync group Movement"
        );
        assert_eq!(steady.label, Some("sprint_steady"));
        assert!(!steady.capture_every_frame);
        assert_eq!(
            steady.source_pose_chain[0].node,
            "A_Als_Sprint\nSequence Player\nSync group Movement"
        );
        assert_eq!(start.canonical_sync_phase, steady.canonical_sync_phase);
        assert_eq!(steady.observed_runtime_state, Some(0));
        assert_eq!(steady.observed_direction, Some("Forward"));
        assert!(source_sprint_values(&AnimationGraphPlayer::new(default())).is_none());
    }

    #[test]
    fn sprint_reader_keeps_inactive_frame_values_and_marks_them_stale() {
        let mut source = SourceInstanceProperties::default();
        source
            .values
            .insert("standing_state.sprint_time".into(), DataValue::F32(0.25));
        source.values.insert(
            "standing_state.sprint_acceleration_amount".into(),
            DataValue::F32(1.0),
        );
        source
            .write_frames
            .insert("standing_state.sprint_time".into(), 3);
        assert_eq!(
            source_sprint_values_at_frame(&source, 4),
            Some((0.25, 1.0, false))
        );
        assert_eq!(
            source_sprint_values_at_frame(&source, 3),
            Some((0.25, 1.0, true))
        );
    }

    #[test]
    fn generated_stop_pose_chain_uses_the_idle_endpoint_only() {
        let chain = generated_stop_source_pose_chain(false);
        assert_eq!(chain.len(), 4);
        assert_eq!(chain[0].label, "standing_states_fsm");
        assert_eq!(chain[1].label, "standing_graph_output");
        assert_eq!(chain[2].label, "grounded_standing_cache");
        assert_eq!(chain[3].label, "grounded_fsm");
        assert!(chain.iter().all(|boundary| {
            boundary.minimum_translation_span == 0.0
                && boundary.minimum_rotation_span_radians == 0.0
        }));
    }

    #[test]
    fn state_machine_grounded_window_starts_after_the_fixture_boundary() {
        let entering = state_machine_grounded_window(0.0);
        assert!(!entering.settled);
        assert!(!entering.capture_source_window);
        let settled = state_machine_grounded_window(0.5);
        assert!(!settled.settled);
        let before_forward = state_machine_grounded_window(0.75);
        assert!(!before_forward.settled);
        let forward = state_machine_grounded_window(1.0);
        assert!(forward.settled);
        let stop = state_machine_grounded_window(2.4);
        assert!(!stop.settled);
        let before_backward = state_machine_grounded_window(3.2);
        assert!(!before_backward.settled);
        let backward = state_machine_grounded_window(3.5);
        assert!(backward.settled);
        assert!(forward.capture_source_window);
        assert_eq!(settled.source_pose_chain.len(), 9);
        assert_eq!(
            forward.source_pose_chain[0].node,
            "Save cached pose 'Movement'"
        );
        assert_eq!(settled.source_sync_group, Some("Movement"));
        assert_eq!(
            settled.sync_phase_global,
            Some("sync_groups.movement.phase")
        );
    }

    #[test]
    fn source_visible_standing_direction_signatures_have_exact_cases() {
        let expected = [
            (0, AlsMovementDirection::Forward, "Move Forward"),
            (1, AlsMovementDirection::Backward, "Move Backward"),
            (3, AlsMovementDirection::LeftForward, "Move Left Forward"),
            (7, AlsMovementDirection::RightForward, "Move Right Forward"),
        ];
        for (index, direction, source_state) in expected {
            let case = direction_regression_case(index).unwrap();
            assert_eq!(case.expected_movement_state, Some(direction));
            assert_eq!(case.source_state, Some(source_state));
            assert_eq!(
                case.direction_signature_group,
                Some("AB_Als_Standing/Movement States")
            );
            assert_eq!(
                case.expected_movement_state
                    .map(movement_direction_fsm_index),
                Some(movement_direction_fsm_index(direction))
            );
            assert_eq!(
                movement_direction_from_fsm_index(movement_direction_fsm_index(direction)),
                Some(direction)
            );
        }
    }

    #[test]
    fn backward_side_auto_edges_are_not_steady_direction_signatures() {
        let backward = direction_regression_case(5).unwrap();
        let left_forward = direction_regression_case(6).unwrap();
        assert_eq!(
            backward.expected_movement_state,
            Some(AlsMovementDirection::Backward)
        );
        assert_eq!(backward.move_dir, Vec2::NEG_Y);
        assert_eq!(
            left_forward.expected_movement_state,
            Some(AlsMovementDirection::LeftForward)
        );
        assert_eq!(
            left_forward.source_state,
            Some("Move Left Backward to Move Left Forward")
        );
        assert!(left_forward.direction_signature_group.is_none());
        assert_eq!(left_forward.move_dir, Vec2::NEG_X);
        let right_forward = direction_regression_case(2).unwrap();
        assert_eq!(
            right_forward.expected_movement_state,
            Some(AlsMovementDirection::RightForward)
        );
        assert_eq!(
            right_forward.source_state,
            Some("Move Right Backward to Move Right Forward")
        );
        assert!(right_forward.direction_signature_group.is_none());
        assert_eq!(right_forward.move_dir, Vec2::X);
    }

    #[test]
    fn direction_branch_masks_accept_only_declared_axes() {
        assert!(direction_branch_matches(
            Some(0),
            DIRECTION_BRANCH_FORWARD | DIRECTION_BRANCH_RIGHT
        ));
        assert!(direction_branch_matches(
            Some(3),
            DIRECTION_BRANCH_FORWARD | DIRECTION_BRANCH_RIGHT
        ));
        assert!(!direction_branch_matches(
            Some(2),
            DIRECTION_BRANCH_FORWARD | DIRECTION_BRANCH_RIGHT
        ));
        assert!(!direction_branch_matches(None, DIRECTION_BRANCH_FORWARD));
    }

    #[test]
    fn crouch_direction_fixture_waits_for_actual_state_samples() {
        assert!(!crouch_direction_case_complete(None, 10.0));
        assert!(!crouch_direction_case_complete(Some(3.0), 4.0));
        assert!(crouch_direction_case_complete(Some(3.0), 4.5));
    }

    #[test]
    fn crouch_direction_reference_frame_does_not_override_stationary_rotation() {
        let mut cases = [true; CROUCH_DIRECTION_CASE_COUNT];
        assert!(!crouch_direction_reference_frame_active(&cases));
        cases[3] = false;
        assert!(crouch_direction_reference_frame_active(&cases));
    }

    #[test]
    fn crouch_direction_fixture_maps_target_axes_to_source_states() {
        let expected = [
            (0, AlsMovementDirection::Forward),
            (1, AlsMovementDirection::Backward),
            (2, AlsMovementDirection::RightForward),
            (3, AlsMovementDirection::LeftForward),
            (4, AlsMovementDirection::Forward),
            (5, AlsMovementDirection::Backward),
            (6, AlsMovementDirection::LeftForward),
            (7, AlsMovementDirection::RightForward),
        ];
        for (index, direction) in expected {
            let case = crouch_direction_regression_case(index).unwrap();
            assert_eq!(case.expected_direction, direction);
            let world = bevy_als::als::motion::view_relative_move_direction(case.move_dir, 0.0);
            let angle = bevy_als::direction_layer::view_space_direction_angle_from_radians(
                bevy_als::direction_layer::velocity_yaw_deg(world),
                0.0,
            );
            assert_eq!(
                bevy_als::als::movement_direction::update_direction(None, angle),
                direction
            );
        }
    }

    #[test]
    fn crouch_stationary_fixture_holds_aim_only_for_the_source_rotate_actions() {
        assert!(!crouch_stationary_validation_holds_aim(None));
        assert!(!crouch_stationary_validation_holds_aim(Some(
            CrouchStationaryValidation::TurnRight
        )));
        assert!(!crouch_stationary_validation_holds_aim(Some(
            CrouchStationaryValidation::TurnLeft
        )));
        assert!(crouch_stationary_validation_holds_aim(Some(
            CrouchStationaryValidation::RotateRight
        )));
        assert!(crouch_stationary_validation_holds_aim(Some(
            CrouchStationaryValidation::RotateLeft
        )));
    }

    #[test]
    fn source_stationary_fixture_requires_consecutive_stable_frames() {
        let mut frames = 0;
        for _ in 0..CROUCH_STATIONARY_STABLE_FRAMES - 1 {
            frames = advance_crouch_stationary_stability(frames, true);
        }
        assert!(frames < CROUCH_STATIONARY_STABLE_FRAMES);
        frames = advance_crouch_stationary_stability(frames, false);
        assert_eq!(frames, 0);
        for _ in 0..CROUCH_STATIONARY_STABLE_FRAMES {
            frames = advance_crouch_stationary_stability(frames, true);
        }
        assert_eq!(frames, CROUCH_STATIONARY_STABLE_FRAMES);
    }

    #[test]
    fn crouch_stationary_fixture_waits_for_each_completed_source_action() {
        let source = include_str!("mod.rs");
        let branch = source
            .split("else if let Some(validation) = next_crouch_stationary_validation")
            .nth(1)
            .unwrap();
        let setup = branch.split("let command_index").next().unwrap();
        assert!(!setup.contains("reset_character_with_rotation"));
        assert!(!setup.contains("reinitialize_fixture_rotation_response"));
        let mut coverage = IntegrationCoverage::default();
        assert_eq!(
            next_crouch_stationary_validation(&coverage),
            Some(CrouchStationaryValidation::TurnRight)
        );
        coverage.crouch_turn_completion_mask |= 1 << 0;
        assert_eq!(
            next_crouch_stationary_validation(&coverage),
            Some(CrouchStationaryValidation::TurnLeft)
        );
        coverage.crouch_turn_completion_mask |= 1 << 1;
        assert_eq!(
            next_crouch_stationary_validation(&coverage),
            Some(CrouchStationaryValidation::RotateRight)
        );
        coverage.crouch_rotate_mask |= 1 << 0;
        assert_eq!(
            next_crouch_stationary_validation(&coverage),
            Some(CrouchStationaryValidation::RotateRight)
        );
        coverage.crouch_rotate_completion_mask |= 1 << 0;
        assert_eq!(
            next_crouch_stationary_validation(&coverage),
            Some(CrouchStationaryValidation::RotateLeft)
        );
        coverage.crouch_rotate_completion_mask |= 1 << 1;
        assert_eq!(next_crouch_stationary_validation(&coverage), None);
    }

    #[test]
    fn crouch_rotate_fixture_requires_the_source_loop_boundary_and_return_to_stationary() {
        assert!(!source_rotate_window_completed(true, false, true));
        assert!(!source_rotate_window_completed(true, true, false));
        assert!(!source_rotate_window_completed(false, true, true));
        assert!(source_rotate_window_completed(true, true, true));
    }

    #[test]
    fn crouch_visual_boundaries_follow_the_generated_source_tree() {
        let chains = [
            crouch_direction_source_pose_chain(AlsMovementDirection::Forward),
            crouch_rotate_source_pose_chain(true),
            crouch_rotate_source_pose_chain(false),
            crouch_turn_source_pose_chain(),
        ];
        for boundary in chains.into_iter().flatten() {
            assert!(
                boundary.graph.starts_with("ALS/") || boundary.graph.starts_with("ALSCamera/"),
                "{}",
                boundary.graph
            );
            let graph = std::fs::read_to_string(format!("assets/{}", boundary.graph)).unwrap();
            assert!(
                graph.contains(&format!("name: {:?}", boundary.node)),
                "{}: {}",
                boundary.graph,
                boundary.node
            );
        }
        assert_eq!(
            crouch_rotate_window(false, true, &[])
                .expected_terminal_requested_parent_transition_active,
            None
        );
        assert_eq!(
            crouch_turn_window(false, true, &[])
                .expected_terminal_requested_parent_transition_active,
            None
        );
        assert!(!crouch_rotate_window(false, false, &[]).settled);
        assert!(!crouch_turn_window(false, false, &[]).settled);
    }

    #[test]
    fn crouch_source_fixture_uses_transient_side_back_states() {
        for (index, direction, input) in [
            (1, AlsMovementDirection::Backward, Vec2::NEG_Y),
            (2, AlsMovementDirection::RightForward, Vec2::X),
            (6, AlsMovementDirection::LeftForward, Vec2::NEG_X),
        ] {
            let case = crouch_direction_regression_case(index).unwrap();
            assert_eq!(case.expected_direction, direction);
            assert_eq!(case.move_dir, input);
        }
        for chain in [
            crouch_turn_source_pose_chain(),
            crouch_rotate_source_pose_chain(true),
        ] {
            assert!(
                !chain
                    .iter()
                    .any(|boundary| boundary.node == "Apply Additive")
            );
        }
    }

    #[test]
    fn crouch_turn_window_requires_source_measured_yaw_convergence() {
        let window = crouch_turn_window(false, true, &["slot_active"]);
        assert!(window.capture_every_frame);
        assert_eq!(
            window.minimum_actor_yaw_delta_radians,
            CROUCH_TURN_SOURCE_MIN_YAW_CONVERGENCE_RADIANS
        );
        assert_eq!(
            window.minimum_actor_yaw_convergence_radians,
            CROUCH_TURN_SOURCE_MIN_YAW_CONVERGENCE_RADIANS
        );
        assert_eq!(window.expected_actor_yaw_delta_sign, Some(-1));
        assert_eq!(window.minimum_observation_seconds, 0.0);
        assert_eq!(window.expected_terminal_parent_runtime_state, Some(0));
        assert_eq!(
            window.expected_terminal_requested_parent_transition_active,
            None
        );
        assert_eq!(
            crouch_turn_window(true, true, &["slot_active"]).expected_actor_yaw_delta_sign,
            Some(1)
        );
        assert!(
            window
                .expected_lifecycle_stages
                .contains(&"natural_completion")
        );
    }

    #[test]
    fn crouch_rotate_window_uses_source_terminal_angle_for_a_moving_view() {
        let window = crouch_rotate_window(false, true, &[]);
        let live_view = 1.0;
        let command =
            standing_turn_mouse_command(4, Some(0.0), live_view, live_view, RotationMode::Aiming);
        assert_eq!(command.look_delta, Vec2::ZERO);
        let from_body =
            standing_turn_mouse_command(4, Some(0.0), -0.25, live_view, RotationMode::Aiming);
        assert!((from_body.look_delta.x.to_radians() - 1.25).abs() < 1e-6);
        assert_eq!(from_body.look_delta.y, 0.0);
        assert!(window.capture_every_frame);
        assert!(!crouch_rotate_window(false, false, &["sequence_completed"]).capture_source_window);
        assert!(
            crouch_rotate_window(false, false, &["sequence_completed", "settled"])
                .capture_source_window
        );
        assert!(!window.requires_actor_yaw_convergence);
        assert_eq!(
            window.minimum_actor_yaw_delta_radians,
            CROUCH_ROTATE_SOURCE_MIN_YAW_CONVERGENCE_RADIANS
        );
        assert_eq!(window.minimum_actor_yaw_convergence_radians, 0.0);
        assert_eq!(window.expected_actor_yaw_delta_sign, Some(-1));
        assert_eq!(window.expected_terminal_parent_runtime_state, Some(0));
        assert_eq!(
            window.expected_terminal_requested_parent_transition_active,
            None
        );
        assert_eq!(
            crouch_rotate_window(true, true, &[]).expected_actor_yaw_delta_sign,
            Some(1)
        );
        assert!(window.requires_source_pose_chain);
        assert_eq!(
            window.expected_lifecycle_stages,
            &["sequence_completed", "settled"]
        );
    }

    #[test]
    fn crouch_rotate_window_restores_the_common_runtime_trace() {
        let mut window = crouch_rotate_window(true, true, &[]);
        let transform = Transform {
            rotation: Quat::from_rotation_y(0.25),
            ..default()
        };
        let rotation_animation = AlsRotationAnimationState {
            yaw_speed_degrees: 42.0,
            ..default()
        };
        let crouching_graph = AlsCrouchingGraphTelemetry {
            graph_turn_curve_direct_yaw_speed_target: Some(21.0),
            ..default()
        };
        let turn_drive = AlsTurnDrive {
            rotate_active: true,
            rotate_clip_time: 0.4,
            rotate_lr: 0.0,
            rotate_rate: 1.2,
            ..default()
        };
        let turn_stack = AlsTurnMontageStack::default();
        record_animation_window_trace(
            &mut window,
            &transform,
            0.75,
            Some(&rotation_animation),
            Some(&crouching_graph),
            Some(&turn_drive),
            Some(&turn_stack),
            true,
            Some(crouch_stationary_animation_variant(true, false)),
        );
        assert!(window.observed_actor_yaw.is_some());
        assert_eq!(window.observed_view_yaw, Some(0.75));
        assert_eq!(window.observed_animation_yaw_speed_degrees, Some(42.0));
        assert_eq!(
            window.observed_animation_curve_direct_yaw_speed_degrees,
            Some(21.0)
        );
        assert_eq!(window.observed_animation_clip_time, Some(0.4));
        assert_eq!(window.observed_animation_weight, Some(1.0));
        assert_eq!(window.observed_animation_play_rate, Some(1.2));
        assert_eq!(window.observed_animation_curve_stack_weight, Some(0.0));
        assert!(window.observed_animation_curve_stack_entries.is_empty());
        assert_eq!(
            window.observed_animation_variant,
            Some("A_Als_Crouch_Rotate_90_Left")
        );
    }

    #[test]
    fn crouch_stationary_waits_for_the_source_idle_slot_precondition() {
        let mut motion = AlsActualMotion::default();
        let telemetry = AlsCrouchingGraphTelemetry {
            actual_outer_state: Some(0),
            actual_outer_transition_active: Some(false),
            actual_outer_pose_ready: Some(true),
            ..default()
        };
        let mut drive = AlsTurnDrive::default();
        let mut slot = AlsStandingTurnSlotRuntime::default();
        assert!(crouch_stationary_source_precondition(
            Some(&motion),
            Some(&telemetry),
            Some(&drive),
            Some(&slot),
        ));
        motion.horizontal_speed = bevy_als::turn_layer::TURN_GROUND_SPEED_EPS * 2.0;
        assert!(!crouch_stationary_source_precondition(
            Some(&motion),
            Some(&telemetry),
            Some(&drive),
            Some(&slot),
        ));
        motion.horizontal_speed = 0.0;
        drive.rotate_active = true;
        assert!(!crouch_stationary_source_precondition(
            Some(&motion),
            Some(&telemetry),
            Some(&drive),
            Some(&slot),
        ));
        drive.rotate_active = false;
        slot.active = true;
        assert!(!crouch_stationary_source_precondition(
            Some(&motion),
            Some(&telemetry),
            Some(&drive),
            Some(&slot),
        ));
    }

    #[test]
    fn actual_velocity_branch_mask_preserves_dominant_local_axes() {
        let forward = AlsActualMotion {
            velocity_blend: bevy_als::extensions::HorizontalVelocityBlend {
                forward: 1.0,
                ..default()
            },
            ..default()
        };
        let diagonal = AlsActualMotion {
            velocity_blend: bevy_als::extensions::HorizontalVelocityBlend {
                forward: 0.5,
                right: 0.5,
                ..default()
            },
            ..default()
        };
        assert_eq!(
            actual_velocity_branch_mask(&forward),
            DIRECTION_BRANCH_FORWARD
        );
        assert_eq!(
            actual_velocity_branch_mask(&diagonal),
            DIRECTION_BRANCH_FORWARD | DIRECTION_BRANCH_RIGHT
        );
    }

    #[test]
    fn state_machine_rotate_fixture_prescribes_the_physics_yaw_each_frame() {
        use bevy::ecs::system::RunSystemOnce;
        for elapsed in [0.5, 1.8, 3.1, 4.4] {
            let (mut world, character, _) = manual_driver_fixture();
            let yaw = world.resource::<ManualIntegrationFixture>().facing_yaw;
            world
                .entity_mut(character)
                .insert(Rotation(Quat::from_rotation_y(
                    yaw + std::f32::consts::FRAC_PI_2,
                )));
            {
                let mut state = world.resource_mut::<ManualIntegrationState>();
                state.scenario = IntegrationScenario::StateMachine;
                state.phase = IntegrationPhase::StateMachineRotate;
                state.phase_started_seconds = -elapsed;
                state.entered = false;
            }
            world.run_system_once(drive_manual_integration).unwrap();
            let expected = Quat::from_rotation_y(yaw);
            assert!(
                world
                    .get::<Transform>(character)
                    .unwrap()
                    .rotation
                    .abs_diff_eq(expected, 1e-6)
            );
            assert!(
                world
                    .get::<Rotation>(character)
                    .unwrap()
                    .0
                    .abs_diff_eq(expected, 1e-6),
                "The stationary rotate fixture left a different physics yaw: elapsed={elapsed}"
            );
        }
    }

    #[test]
    fn state_machine_turn_fixture_allows_both_source_montages_to_finish() {
        assert!(STATE_MACHINE_TURN_FIRST_END - STATE_MACHINE_TURN_FIRST_START >= 1.0);
        assert!(STATE_MACHINE_TURN_SECOND_END - STATE_MACHINE_TURN_SECOND_START >= 1.0);
        assert_eq!(
            state_machine_turn_target_yaw(STATE_MACHINE_TURN_FIRST_START, 0.0),
            90.0f32.to_radians()
        );
        assert_eq!(state_machine_turn_target_yaw(2.0, 0.0), 0.0);
        assert_eq!(
            state_machine_turn_target_yaw(STATE_MACHINE_TURN_SECOND_START, 0.0),
            -90.0f32.to_radians()
        );
        assert!(!state_machine_turn_phase_complete(
            STATE_MACHINE_TURN_END - 0.01
        ));
        assert!(state_machine_turn_phase_complete(STATE_MACHINE_TURN_END));
    }
}
