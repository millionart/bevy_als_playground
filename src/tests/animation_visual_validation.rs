use bevy::ecs::schedule::ScheduleLabel;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    num::NonZeroU8,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use bevy_als::als::character::AlsCharacter;
use bevy_als::als_camera::AlsCameraSet;
use bevy_als::extensions::{
    InterruptibleFsmState, RagdollPhysicsProfileApplied, player_animation_context_state,
};
use avian3d::prelude::{PhysicsSystems, Position, RigidBody, Rotation};
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy_animation_graph::core::{
    animated_scene::AnimatedSceneInstance,
    animation_graph::{AnimationGraph, NodeId, SourcePin, TargetPin},
    animation_graph_player::{AnimationGraphPlayer, AnimationSource},
    context::node_states::StateKey,
    edge_data::DataValue,
    errors::GraphError,
    plugin::AnimationGraphSet,
    pose::{BonePose, Pose},
    ragdoll::definition::BodyLabel,
    skeleton::Skeleton,
    systems::apply_animation_to_targets,
};
use serde::Serialize;

use bevy_als::{
    animation_context_projection::{AlsRootAnimationContext, AlsRootAnimationContextSet},
    character_assembly::AlsCharacterAvatar,
    control_rig_apply::{AlsControlRigExecution, AlsControlRigInput, AlsControlRigPropagationSet},
    foot_ik_apply::{
        AlsFootIkApplyTelemetry, AlsFootIkApplyTelemetrySide, AlsFootIkSpaceSide,
        AlsFootIkSpaceState, AlsFootLockTeleportTelemetry, FootIkApplySet,
    },
    ragdoll_link::{AlsRagdollGroundFollow, AlsRagdollInitialSpeedLimit},
};

use crate::manual_playground_integration::{
        ManualAnimationPoseBoundary, ManualAnimationWindowEvidence, ManualIntegrationGateReport,
        ManualIntegrationObservationSet, ManualPlaygroundStartupReady, ManualRuntimePhaseEvidence,
        ManualTurnStackEntryEvidence,
    };

const REQUIRED_BONES: &[&str] = &[
    "pelvis",
    "spine_01",
    "spine_02",
    "spine_03",
    "neck_01",
    "head",
    "upperarm_l",
    "lowerarm_l",
    "hand_l",
    "upperarm_r",
    "lowerarm_r",
    "hand_r",
    "thigh_l",
    "calf_l",
    "foot_l",
    "thigh_r",
    "calf_r",
    "foot_r",
];

const SEGMENTS: &[(&str, &str)] = &[
    ("pelvis", "spine_01"),
    ("spine_01", "spine_02"),
    ("spine_02", "spine_03"),
    ("spine_03", "neck_01"),
    ("neck_01", "head"),
    ("upperarm_l", "lowerarm_l"),
    ("lowerarm_l", "hand_l"),
    ("upperarm_r", "lowerarm_r"),
    ("lowerarm_r", "hand_r"),
    ("pelvis", "thigh_l"),
    ("thigh_l", "calf_l"),
    ("calf_l", "foot_l"),
    ("pelvis", "thigh_r"),
    ("thigh_r", "calf_r"),
    ("calf_r", "foot_r"),
];

const MOTION_BONES: &[&str] = &["thigh_l", "calf_l", "foot_l", "thigh_r", "calf_r", "foot_r"];

const LIMB_CHAINS: &[(&str, &str, &str, &str)] = &[
    ("left_leg", "thigh_l", "calf_l", "foot_l"),
    ("right_leg", "thigh_r", "calf_r", "foot_r"),
    ("left_arm", "upperarm_l", "lowerarm_l", "hand_l"),
    ("right_arm", "upperarm_r", "lowerarm_r", "hand_r"),
];

const TEMPORAL_GLOBAL_POSITION_SPEED_METERS_PER_SECOND: f32 = 40.0;
const TEMPORAL_LOCAL_TRANSLATION_JUMP_METERS: f32 = 0.35;
const TEMPORAL_LOCAL_TRANSLATION_SPEED_METERS_PER_SECOND: f32 = 14.0;
const TEMPORAL_LOCAL_ROTATION_JUMP_RADIANS: f32 = 2.2;
const TEMPORAL_LOCAL_ROTATION_SPEED_RADIANS_PER_SECOND: f32 = 80.0;
const TEMPORAL_LOCAL_ROTATION_SPEED_MIN_DELTA_RADIANS: f32 = 0.75;
const TEMPORAL_LARGE_ANGULAR_CHANGE_RATE_RADIANS_PER_SECOND: f32 = 30.0;
const TEMPORAL_SCALE_RATIO: f32 = 1.5;
const TEMPORAL_SEGMENT_RATIO: f32 = 0.35;
const TEMPORAL_SEGMENT_MIN_DELTA_METERS: f32 = 0.05;
const TEMPORAL_LIMB_PLANE_FLIP_DOT: f32 = -0.5;
const ANIMATION_VISUAL_EVIDENCE_FORMAT_SOURCE: &str =
    include_str!("animation_visual_evidence_format.txt");

fn animation_visual_evidence_format() -> &'static str {
    ANIMATION_VISUAL_EVIDENCE_FORMAT_SOURCE.trim()
}
const TEMPORAL_LIMB_BEND_SINE: f32 = 0.08;
const MAX_TEMPORAL_ANOMALIES: usize = 64;
const AUTOMATION_PROGRESS_STALL: Duration = Duration::from_secs(12);

type FinalBone = (Vec3, Quat, Vec3, Vec3);

#[derive(Resource)]
struct AnimationVisualValidationConfig {
    directory: PathBuf,
    case: String,
    capture_count: usize,
    capture_interval: Duration,
    min_pose_samples: usize,
    warmup: Duration,
    max_frame_p95_ms: f64,
    max_frame_ms: f64,
    max_eval_p95_ms: f64,
    max_physics_steps_per_render: f64,
}

impl AnimationVisualValidationConfig {
    fn from_environment() -> Option<Self> {
        let directory = PathBuf::from(std::env::var("ALS_ANIMATION_VISUAL_EVIDENCE_DIR").ok()?);
        fs::create_dir_all(&directory).ok()?;
        Some(Self {
            directory,
            case: std::env::var("ALS_MANUAL_INTEGRATION_CASE")
                .unwrap_or_else(|_| "base".to_owned()),
            capture_count: std::env::var("ALS_ANIMATION_VISUAL_CAPTURE_COUNT")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(0),
            capture_interval: Duration::from_secs_f32(env_f32(
                "ALS_ANIMATION_VISUAL_CAPTURE_INTERVAL_SECS",
                1.5,
            )),
            min_pose_samples: env_usize("ALS_ANIMATION_VISUAL_MIN_POSE_SAMPLES", 24),
            warmup: Duration::from_secs_f32(env_f32("ALS_ANIMATION_VISUAL_WARMUP_SECS", 1.0)),
            max_frame_p95_ms: env_f64("ALS_ANIMATION_VISUAL_MAX_FRAME_P95_MS", 25.0),
            max_frame_ms: env_f64("ALS_ANIMATION_VISUAL_MAX_FRAME_MS", 75.0),
            max_eval_p95_ms: env_f64("ALS_ANIMATION_VISUAL_MAX_EVAL_P95_MS", 4.0),
            max_physics_steps_per_render: env_f64(
                "ALS_ANIMATION_VISUAL_MAX_PHYSICS_STEPS_PER_RENDER",
                3.0,
            ),
        })
    }
}

#[derive(Resource, Default)]
struct AnimationVisualValidationState {
    started: Option<Instant>,
    last_frame: Option<Instant>,
    next_pose_sample: Option<Instant>,
    next_capture: Option<Instant>,
    capture_index: usize,
    fault_captures: usize,
    eval_started: Option<Instant>,
    frame_started: Option<Instant>,
    playable_frame: Option<bool>,
    skip_frame_samples: u8,
    excluded_frame_samples: usize,
    frame_ms: Vec<f64>,
    loading_frame_ms: Vec<f64>,
    cpu_frame_ms: Vec<f64>,
    eval_ms: Vec<f64>,
    pose_samples: Vec<PoseSample>,
    screenshots: Vec<ScreenshotSample>,
    faults: Vec<String>,
    graph_errors: Vec<String>,
    primary_failure: Option<GraphEvaluationFailure>,
    fatal_exit_frames_remaining: Option<u8>,
    temporal_pre_rig_guard: TemporalSkeletonGuard,
    temporal_final_guard: TemporalSkeletonGuard,
    temporal_anomalies: Vec<TemporalSkeletonAnomaly>,
    temporal_context: Option<String>,
    temporal_window_settled: bool,
    temporal_rebase_epoch: Option<u64>,
    captured_direction_cases: BTreeSet<String>,
    motion_windows: BTreeMap<String, MotionWindowTracker>,
    required_motion_windows: BTreeSet<String>,
    last_teleport_count: u32,
    teleport_captures: usize,
    observed_bones: usize,
    watchdog: AutomationProgressWatchdog,
}

impl AnimationVisualValidationState {
    fn performance_ready(&self, warmup: Duration) -> bool {
        self.started.is_some_and(|started| {
            self.playable_frame
                .unwrap_or_else(|| started.elapsed() >= warmup)
        })
    }
}

#[derive(Clone, Default)]
struct AutomationProgressWatchdog {
    last_capture_count: usize,
    last_progress: Option<Instant>,
    triggered: bool,
    stalled_seconds: f64,
}

#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AnimationVisualValidationProgress {
    pub evidence_ready: bool,
}

#[derive(Resource, Default)]
struct AnimationVisualStageTiming {
    physics_started: Option<Instant>,
    physics_step_started: Option<Instant>,
    camera_started: Option<Instant>,
    samples: BTreeMap<String, Vec<f64>>,
    physics_steps_since_render: usize,
    physics_steps_per_render: Vec<f64>,
}

#[derive(Clone, Copy, Default)]
struct LocalBonePose {
    translation: Vec3,
    rotation: Quat,
    scale: Vec3,
}

#[derive(Clone, Copy)]
struct LimbPlane {
    normal: Vec3,
}

#[derive(Default)]
struct TemporalSkeletonGuard {
    previous_relative: BTreeMap<String, Vec3>,
    previous_local: BTreeMap<String, LocalBonePose>,
    previous_segments: BTreeMap<String, f32>,
    previous_limb_planes: BTreeMap<String, LimbPlane>,
}

#[derive(Clone, Debug, PartialEq)]
struct TemporalSkeletonMetric {
    kind: &'static str,
    subject: String,
    previous: f32,
    current: f32,
    threshold: f32,
    trigger: &'static str,
    rate: Option<f32>,
    rate_threshold: Option<f32>,
}

struct TemporalSkeletonObservation {
    metrics: Vec<TemporalSkeletonMetric>,
}

#[derive(Serialize, Clone)]
struct TemporalSkeletonAnomaly {
    stage: String,
    kind: String,
    subject: String,
    previous: f32,
    current: f32,
    threshold: f32,
    trigger: String,
    rate: Option<f32>,
    rate_threshold: Option<f32>,
    elapsed_secs: f64,
    frame_ms: f64,
    animation_delta_seconds: f32,
    direction_case: Option<String>,
    phase: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_boundary: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_selector_inputs: Option<[[f32; 2]; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_physics_readback: Option<ScaledRagdollReadback>,
}

#[derive(Serialize, Clone)]
struct ScaledRagdollReadback {
    mesh_scale: [f32; 3],
    joint_scale: [f32; 3],
    remaining_updates: u8,
    bodies: BTreeMap<String, [f32; 2]>,
}

impl ScaledRagdollReadback {
    fn covers(&self, metric: &TemporalSkeletonMetric) -> bool {
        match metric.kind {
            "global_position_speed" | "local_translation_spike" | "local_rotation_spike" => {
                self.bodies.contains_key(&metric.subject)
            }
            "segment_length_spike" => metric.subject.split_once("->").is_some_and(|(a, b)| {
                self.bodies.contains_key(a) && self.bodies.contains_key(b)
            }),
            _ => false,
        }
    }
}

#[derive(Clone, Copy, Default)]
struct RootPoseActivity {
    output_present: bool,
    bone_count: usize,
    active_channel_count: usize,
}

#[derive(Serialize, Clone)]
struct RootPoseSample {
    output_present: bool,
    bone_count: usize,
    active_channel_count: usize,
}

#[derive(Serialize, Clone)]
struct GraphFailurePin {
    kind: String,
    node_id: Option<String>,
    pin_id: Option<String>,
    graph_context: Option<String>,
}

#[derive(Serialize, Clone)]
struct GraphFailureNodeCandidate {
    graph_path: Option<String>,
    graph_asset_id: String,
    graph_context: Option<String>,
    node_id: String,
    node_name: String,
    node_kind: String,
    active_context: bool,
    context_curves: BTreeMap<String, f32>,
}

#[derive(Serialize, Clone)]
struct GraphFailureContextNode {
    graph_path: Option<String>,
    graph_context: String,
    node_id: String,
    node_name: String,
    node_kind: String,
    context_curves: BTreeMap<String, f32>,
}

#[derive(Serialize, Clone)]
struct GraphEvaluationFailure {
    category: String,
    raw_error: String,
    error_kind: String,
    pin: Option<GraphFailurePin>,
    node_candidates: Vec<GraphFailureNodeCandidate>,
    active_context_nodes: Vec<GraphFailureContextNode>,
    active_node_candidate_count: usize,
    candidate_resolution: String,
    root_graph_path: Option<String>,
    root_graph_recursive_dependencies: String,
    root_pose: RootPoseSample,
    pose_classification: String,
    case: String,
    direction_case: Option<String>,
    phase: Option<String>,
    phase_elapsed_secs: Option<f32>,
    locomotion: Option<String>,
    gait: Option<String>,
    stance: Option<String>,
    rotation_mode: Option<String>,
    screenshot_status: String,
    screenshot_path: Option<String>,
    reported_before_ready: bool,
}

#[derive(Serialize, Clone)]
struct LocalMotionBoneSample {
    translation: [f32; 3],
    rotation: [f32; 4],
}

#[derive(Resource, Default)]
struct AnimationVisualPreRigPose {
    bones: BTreeMap<String, LocalBonePose>,
    global_bones: BTreeMap<String, FinalBone>,
    root_position: Vec3,
    delta_seconds: f32,
    landing_selector: Option<[f32; 2]>,
    physics_readback_active: bool,
    source_selector_inputs: Option<[[f32; 2]; 2]>,
}

impl AnimationVisualPreRigPose {
    fn temporal_delta_seconds(&self) -> f32 {
        self.delta_seconds.max(1e-4)
    }
}

#[derive(Serialize, Clone)]
struct ActorYawTraceSample {
    elapsed_secs: f64,
    actor_yaw_radians: Option<f32>,
    view_yaw_radians: Option<f32>,
    animation_yaw_speed_degrees: Option<f32>,
    animation_curve_direct_yaw_speed_degrees: Option<f32>,
    animation_clip_time: Option<f32>,
    animation_weight: Option<f32>,
    animation_play_rate: Option<f32>,
    animation_curve_stack_weight: Option<f32>,
    animation_curve_stack_entries: Vec<ManualTurnStackEntryEvidence>,
    animation_variant: Option<&'static str>,
    requested_parent_runtime_state: Option<u8>,
    requested_parent_transition_active: Option<bool>,
}

#[derive(Default)]
struct MotionWindowTracker {
    source_node: String,
    source_state: String,
    expected_direction: String,
    observed_directions: BTreeSet<String>,
    expected_runtime_state: Option<u8>,
    observed_runtime_states: BTreeSet<u8>,
    expected_parent_runtime_state: Option<u8>,
    expected_terminal_parent_runtime_state: Option<u8>,
    expected_terminal_parent_transition_active: Option<bool>,
    expected_terminal_requested_parent_transition_active: Option<bool>,
    observed_parent_runtime_states: BTreeSet<u8>,
    observed_parent_transition_active: BTreeSet<bool>,
    observed_requested_parent_runtime_states: BTreeSet<u8>,
    observed_requested_parent_transition_active: BTreeSet<bool>,
    observed_terminal_parent_runtime_states: BTreeSet<u8>,
    observed_terminal_parent_transition_active: BTreeSet<bool>,
    observed_terminal_requested_parent_runtime_states: BTreeSet<u8>,
    observed_terminal_requested_parent_transition_active: BTreeSet<bool>,
    sample_count: usize,
    requires_pre_rig_motion: bool,
    requires_final_pose_motion: bool,
    requires_non_reference_root_pose: bool,
    requires_source_pose_chain: bool,
    requires_actor_yaw_motion: bool,
    requires_actor_yaw_convergence: bool,
    actor_yaw_driver: Option<&'static str>,
    expected_actor_yaw_delta_sign: Option<i8>,
    expected_actor_yaw_baseline: Option<f32>,
    expected_animation_variant: Option<&'static str>,
    minimum_pre_rig_translation_span: f32,
    minimum_pre_rig_rotation_span_radians: f32,
    minimum_final_translation_span: f32,
    minimum_final_rotation_span_radians: f32,
    minimum_actor_yaw_delta_radians: f32,
    minimum_actor_yaw_convergence_radians: f32,
    minimum_observation_seconds: f32,
    observation_seconds: f32,
    first_sample_game_seconds: Option<f64>,
    max_pre_rig_translation_span: f32,
    max_pre_rig_rotation_span_radians: f32,
    max_final_translation_span: f32,
    max_final_rotation_span_radians: f32,
    pre_rig_reference: BTreeMap<String, LocalBonePose>,
    final_reference: BTreeMap<String, LocalBonePose>,
    root_pose_sample_count: usize,
    root_pose_active_samples: usize,
    root_pose_reference_samples: usize,
    root_pose_max_active_channels: usize,
    root_pose_max_translation_span: f32,
    root_pose_max_rotation_span_radians: f32,
    root_pose_reference: Option<Pose>,
    actor_yaw_sample_count: usize,
    actor_yaw_reference: Option<f32>,
    actor_yaw_cumulative_delta: f32,
    actor_yaw_error_reference: Option<f32>,
    actor_yaw_trace: Vec<ActorYawTraceSample>,
    max_actor_yaw_delta_radians: f32,
    max_actor_yaw_signed_delta_radians: f32,
    max_actor_yaw_opposite_delta_radians: f32,
    max_actor_yaw_convergence_radians: f32,
    source_sync_group: String,
    sync_phase_global: String,
    sync_phase_min: Option<f32>,
    sync_phase_max: Option<f32>,
    canonical_sync_phase: Option<f32>,
    maximum_sync_phase_distance: f32,
    canonical_sync_phase_distance: Option<f32>,
    canonical_clip_time: Option<f32>,
    maximum_clip_time_distance: f32,
    canonical_clip_time_distance: Option<f32>,
    direction_signature_group: String,
    required_direction_signature_members: usize,
    minimum_direction_signature_rotation_radians: f32,
    canonical_pre_rig_pose: BTreeMap<String, LocalBonePose>,
    source_pose_chain: Vec<SourcePoseBoundaryTracker>,
    expected_lifecycle_stages: BTreeSet<String>,
    observed_lifecycle_stages: BTreeSet<String>,
}

#[derive(Default)]
struct SourcePoseBoundaryTracker {
    label: String,
    graph: String,
    node: String,
    output: String,
    minimum_translation_span: f32,
    minimum_rotation_span_radians: f32,
    sample_count: usize,
    available_samples: usize,
    unavailable_samples: usize,
    cached_output_samples: usize,
    active_fsm_state_samples: usize,
    last_output_fallback_samples: usize,
    minimum_bone_count: Option<usize>,
    maximum_bone_count: usize,
    minimum_max_local_translation_length: Option<f32>,
    minimum_max_local_translation_elapsed_seconds: Option<f32>,
    maximum_max_local_translation_length: f32,
    trace: Vec<SourcePoseBoundaryTraceSample>,
    max_translation_span: f32,
    max_rotation_span_radians: f32,
    reference: Option<Pose>,
}

#[derive(Serialize, Clone)]
struct MotionWindowSample {
    label: String,
    source_node: String,
    source_state: String,
    expected_direction: String,
    observed_directions: Vec<String>,
    expected_runtime_state: Option<u8>,
    observed_runtime_states: Vec<u8>,
    expected_parent_runtime_state: Option<u8>,
    expected_terminal_parent_runtime_state: Option<u8>,
    expected_terminal_parent_transition_active: Option<bool>,
    expected_terminal_requested_parent_transition_active: Option<bool>,
    observed_parent_runtime_states: Vec<u8>,
    observed_parent_transition_active: Vec<bool>,
    observed_requested_parent_runtime_states: Vec<u8>,
    observed_requested_parent_transition_active: Vec<bool>,
    observed_terminal_parent_runtime_states: Vec<u8>,
    observed_terminal_parent_transition_active: Vec<bool>,
    observed_terminal_requested_parent_runtime_states: Vec<u8>,
    observed_terminal_requested_parent_transition_active: Vec<bool>,
    sample_count: usize,
    requires_pre_rig_motion: bool,
    requires_final_pose_motion: bool,
    requires_non_reference_root_pose: bool,
    requires_source_pose_chain: bool,
    requires_actor_yaw_motion: bool,
    requires_actor_yaw_convergence: bool,
    actor_yaw_driver: Option<&'static str>,
    expected_actor_yaw_delta_sign: Option<i8>,
    expected_actor_yaw_baseline: Option<f32>,
    expected_animation_variant: Option<&'static str>,
    minimum_pre_rig_translation_span: f32,
    minimum_pre_rig_rotation_span_radians: f32,
    minimum_final_translation_span: f32,
    minimum_final_rotation_span_radians: f32,
    minimum_actor_yaw_delta_radians: f32,
    minimum_actor_yaw_convergence_radians: f32,
    minimum_observation_seconds: f32,
    observation_seconds: f32,
    max_pre_rig_translation_span: f32,
    max_pre_rig_rotation_span_radians: f32,
    max_final_translation_span: f32,
    max_final_rotation_span_radians: f32,
    root_pose_sample_count: usize,
    root_pose_active_samples: usize,
    root_pose_reference_samples: usize,
    root_pose_max_active_channels: usize,
    root_pose_max_translation_span: f32,
    root_pose_max_rotation_span_radians: f32,
    actor_yaw_sample_count: usize,
    actor_yaw_trace: Vec<ActorYawTraceSample>,
    max_actor_yaw_delta_radians: f32,
    max_actor_yaw_signed_delta_radians: f32,
    max_actor_yaw_opposite_delta_radians: f32,
    max_actor_yaw_convergence_radians: f32,
    source_sync_group: String,
    sync_phase_global: String,
    sync_phase_min: Option<f32>,
    sync_phase_max: Option<f32>,
    canonical_sync_phase: Option<f32>,
    maximum_sync_phase_distance: f32,
    canonical_sync_phase_distance: Option<f32>,
    canonical_clip_time: Option<f32>,
    maximum_clip_time_distance: f32,
    canonical_clip_time_distance: Option<f32>,
    direction_signature_group: String,
    required_direction_signature_members: usize,
    minimum_direction_signature_rotation_radians: f32,
    canonical_pre_rig_pose: BTreeMap<String, LocalMotionBoneSample>,
    source_pose_chain: Vec<SourcePoseBoundarySample>,
    expected_lifecycle_stages: Vec<String>,
    observed_lifecycle_stages: Vec<String>,
    first_failed_source_pose_boundary: Option<String>,
}

#[derive(Serialize, Clone)]
struct SourcePoseBoundarySample {
    label: String,
    graph: String,
    node: String,
    output: String,
    minimum_translation_span: f32,
    minimum_rotation_span_radians: f32,
    sample_count: usize,
    available_samples: usize,
    unavailable_samples: usize,
    cached_output_samples: usize,
    active_fsm_state_samples: usize,
    last_output_fallback_samples: usize,
    minimum_bone_count: Option<usize>,
    maximum_bone_count: usize,
    minimum_max_local_translation_length: Option<f32>,
    minimum_max_local_translation_elapsed_seconds: Option<f32>,
    maximum_max_local_translation_length: f32,
    trace: Vec<SourcePoseBoundaryTraceSample>,
    max_translation_span: f32,
    max_rotation_span_radians: f32,
}

#[derive(Serialize, Clone)]
struct SourcePoseBoundaryTraceSample {
    elapsed_seconds: f32,
    pose_time: f32,
    bone_count: usize,
    maximum_local_translation_length: f32,
}

#[derive(Serialize, Clone)]
struct BoneSample {
    translation: [f32; 3],
    rotation: [f32; 4],
    scale: [f32; 3],
    local_scale: [f32; 3],
}

#[derive(Serialize, Clone)]
struct ScreenshotSample {
    path: String,
    elapsed_secs: f64,
    kind: String,
    direction_case: Option<String>,
    phase: Option<String>,
    phase_elapsed_secs: Option<f32>,
    locomotion: Option<String>,
    gait: Option<String>,
    stance: Option<String>,
    rotation_mode: Option<String>,
    faults: Vec<String>,
}

#[derive(Serialize, Clone)]
struct PoseSample {
    elapsed_secs: f64,
    frame_ms: f64,
    direction_case: Option<String>,
    phase: Option<String>,
    phase_elapsed_secs: Option<f32>,
    locomotion: Option<String>,
    gait: Option<String>,
    stance: Option<String>,
    rotation_mode: Option<String>,
    bones: BTreeMap<String, BoneSample>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pre_rig_bones: BTreeMap<String, BoneSample>,
    segments: BTreeMap<String, f32>,
    foot_ik: Option<FootIkSample>,
    ragdoll_root_space: Option<RagdollRootSpaceSample>,
    root_pose: RootPoseSample,
}

#[derive(Serialize, Clone)]
struct RagdollRootSpaceSample {
    character_transform_translation: [f32; 3],
    character_position: Option<[f32; 3]>,
    avatar_local_translation: [f32; 3],
    avatar_world_translation: [f32; 3],
    player_local_translation: Option<[f32; 3]>,
    player_world_translation: Option<[f32; 3]>,
    follow_pelvis: Option<[f32; 3]>,
    follow_target: Option<[f32; 3]>,
}

#[derive(Serialize, Clone)]
struct FootIkSideSample {
    valid: bool,
    has_ground_hit: bool,
    trace_target_world: [f32; 3],
    solver_reference_world: [f32; 3],
    target_world: [f32; 3],
    resolved_target_world: [f32; 3],
    pole_location_world: [f32; 3],
    effector_world: [f32; 3],
    hip_world: [f32; 3],
    knee_world: [f32; 3],
    ankle_world: [f32; 3],
    raw_offset: f32,
    applied_weight: f32,
    pole_sign: f32,
    knee_error: f32,
    effector_error: f32,
    solve_input: Option<bevy_als::foot_ik_apply::FootIkSolveInput>,
    previous_solve_input: Option<bevy_als::foot_ik_apply::FootIkSolveInput>,
}

#[derive(Serialize, Clone)]
struct FootLockTeleportSample {
    detected_count: u32,
    left_rebase_count: u32,
    right_rebase_count: u32,
    last_translation_delta_m: f32,
    remaining_seconds: f32,
}

#[derive(Serialize, Clone)]
struct FootIkSample {
    left: FootIkSideSample,
    right: FootIkSideSample,
    rig_inputs: Option<serde_json::Value>,
    source_curves_available: bool,
    pose_moving: f32,
    foot_left_ik_curve: f32,
    foot_right_ik_curve: f32,
    foot_left_lock_curve: f32,
    foot_right_lock_curve: f32,
    standing_pose_moving_global: f32,
    pose_moving_lineage: Vec<ContextCurveSample>,
    source_velocity_blend: [f32; 4],
    source_movement_direction: [bool; 4],
    source_hips_direction_lock_amount: f32,
    source_feet_crossing_amount: f32,
    velocity_blend_forward_amount: f32,
    velocity_blend_backward_amount: f32,
    teleport: FootLockTeleportSample,
}

#[derive(Serialize, Clone)]
struct ContextCurveSample {
    node: String,
    pose_moving: Option<f32>,
    foot_left_ik: Option<f32>,
    foot_right_ik: Option<f32>,
    foot_left_lock: Option<f32>,
    foot_right_lock: Option<f32>,
}

#[derive(Serialize)]
struct TimeStats {
    samples: usize,
    p50_ms: f64,
    p95_ms: f64,
    max_ms: f64,
}

#[derive(Serialize)]
struct CountStats {
    samples: usize,
    p50: f64,
    p95: f64,
    max: f64,
}

#[derive(Serialize)]
struct AutomationWatchdogReport {
    triggered: bool,
    reason: String,
    stalled_seconds: f64,
    capture_count: usize,
    required_capture_count: usize,
}

#[derive(Serialize)]
struct AnimationVisualReport<'a> {
    format: &'static str,
    case: String,
    ready: bool,
    pass: bool,
    observed_bones: usize,
    required_bones: Vec<String>,
    missing_bones: Vec<String>,
    frame_time: TimeStats,
    #[serde(skip_serializing_if = "Option::is_none")]
    loading_frame_time: Option<TimeStats>,
    cpu_frame_time: TimeStats,
    animation_eval_time: TimeStats,
    physics_steps_per_render: CountStats,
    stage_time: BTreeMap<String, TimeStats>,
    excluded_frame_samples: usize,
    pose_samples: Vec<PoseSample>,
    screenshots: Vec<ScreenshotSample>,
    motion_windows: Vec<MotionWindowSample>,
    required_motion_windows: Vec<String>,
    temporal_anomalies: Vec<TemporalSkeletonAnomaly>,
    faults: Vec<String>,
    graph_errors: Vec<String>,
    primary_failure: Option<GraphEvaluationFailure>,
    manual_integration: Option<&'a ManualIntegrationGateReport>,
    automation_watchdog: AutomationWatchdogReport,
    thresholds: BTreeMap<String, f64>,
}

pub struct AnimationVisualValidationPlugin;

impl Plugin for AnimationVisualValidationPlugin {
    fn build(&self, app: &mut App) {
        let Some(config) = AnimationVisualValidationConfig::from_environment() else {
            return;
        };
        let physics = app.world().get_resource::<bevy_als::AlsPhysicsSchedule>()
            .map_or_else(|| PostUpdate.intern(), |schedule| schedule.label());
        app.insert_resource(config)
            .insert_resource(bevy_als::foot_ik_apply::AlsFootIkTelemetryEnabled(true))
            .init_resource::<AnimationVisualValidationState>()
            .init_resource::<AnimationVisualStageTiming>()
            .init_resource::<AnimationVisualValidationProgress>()
            .init_resource::<AnimationVisualPreRigPose>()
            .add_systems(First, animation_cpu_frame_start)
            .add_systems(
                physics,
                animation_eval_start
                    .after(bevy_als::extensions::post_physics_animation::PostPhysicsAnimationSet::Inputs)
                    .before(bevy_als::extensions::post_physics_animation::PostPhysicsAnimationSet::Evaluate),
            )
            .add_systems(
                physics,
                animation_eval_end.after(bevy_als::extensions::post_physics_animation::PostPhysicsAnimationSet::Evaluate)
                    .before(AnimationGraphSet::PostPhysics),
            )
            .add_systems(
                PostUpdate,
                capture_animation_visual_pre_rig_pose
                    .after(AnimationGraphSet::Final)
                    .after(apply_animation_to_targets)
                    .before(collect_animation_visual_evidence),
            )
            .add_systems(
                PostUpdate,
                collect_animation_visual_evidence
                    .after(FootIkApplySet::Telemetry)
                    .after(AlsControlRigPropagationSet)
                    .after(AlsCameraSet)
                    .after(AlsRootAnimationContextSet)
                    .after(ManualIntegrationObservationSet),
            )
            .add_systems(
                Last,
                flush_animation_visual_report,
            )
            .add_systems(
                PostUpdate,
                finish_fatal_animation_visual_failure.after(collect_animation_visual_evidence),
            )
            .add_systems(
                PostUpdate,
                update_animation_visual_progress.after(collect_animation_visual_evidence),
            )
            .add_systems(
                PostUpdate,
                abort_stalled_animation_visual_validation.after(update_animation_visual_progress),
            )
            .add_systems(
                physics,
                animation_visual_physics_start.before(PhysicsSystems::First),
            )
            .add_systems(
                physics,
                animation_visual_physics_step_start
                    .after(PhysicsSystems::Prepare)
                    .before(PhysicsSystems::StepSimulation),
            )
            .add_systems(
                physics,
                animation_visual_physics_step_end
                    .after(PhysicsSystems::StepSimulation)
                    .before(PhysicsSystems::Writeback),
            )
            .add_systems(
                physics,
                animation_visual_physics_end.after(PhysicsSystems::Last),
            )
            .add_systems(
                PostUpdate,
                animation_visual_control_rig_time.after(FootIkApplySet::Telemetry)
                    .before(collect_animation_visual_evidence),
            )
            .add_systems(
                PostUpdate,
                animation_visual_camera_start
                    .after(FootIkApplySet::Telemetry)
                    .before(AlsCameraSet),
            )
            .add_systems(PostUpdate, animation_visual_camera_end.after(AlsCameraSet));
    }
}

fn env_f32(name: &str, fallback: f32) -> f32 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<f32>().ok())
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(fallback)
}

fn env_f64(name: &str, fallback: f64) -> f64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(fallback)
}

fn env_usize(name: &str, fallback: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(fallback)
}

fn append_unique(values: &mut Vec<String>, value: String) -> bool {
    if values.len() < 128 && !values.iter().any(|existing| existing == &value) {
        values.push(value);
        return true;
    }
    false
}

fn temporal_scale_ratio(previous: Vec3, current: Vec3) -> f32 {
    let mut ratio = 1.0_f32;
    for (previous, current) in [
        (previous.x.abs(), current.x.abs()),
        (previous.y.abs(), current.y.abs()),
        (previous.z.abs(), current.z.abs()),
    ] {
        if previous <= f32::EPSILON || current <= f32::EPSILON {
            return f32::INFINITY;
        }
        ratio = ratio.max((previous / current).max(current / previous));
    }
    ratio
}

fn limb_plane(
    bones: &BTreeMap<String, FinalBone>,
    first: &str,
    second: &str,
    third: &str,
) -> Option<LimbPlane> {
    let first = bones.get(first)?.0;
    let second = bones.get(second)?.0;
    let third = bones.get(third)?.0;
    let upper = second - first;
    let lower = third - second;
    let lengths = upper.length() * lower.length();
    if !lengths.is_finite() || lengths <= f32::EPSILON {
        return None;
    }
    let normal = upper.cross(lower);
    let bend_sine = normal.length() / lengths;
    if !bend_sine.is_finite() || bend_sine < TEMPORAL_LIMB_BEND_SINE {
        return None;
    }
    Some(LimbPlane {
        normal: normal.normalize(),
    })
}

fn limb_crosses_degenerate_joint_plane(
    previous: &BTreeMap<String, LocalBonePose>,
    current: &BTreeMap<String, LocalBonePose>,
    previous_positions: &BTreeMap<String, Vec3>,
    bones: &BTreeMap<String, FinalBone>,
    first: &str,
    second: &str,
    third: &str,
    frame_seconds: f32,
) -> bool {
    let (Some(old_joint), Some(new_joint), Some(old_tip), Some(new_tip)) = (
        previous.get(second),
        current.get(second),
        previous.get(third),
        current.get(third),
    ) else {
        return false;
    };
    if rotation_delta_radians(old_joint.rotation, new_joint.rotation) / frame_seconds
        >= TEMPORAL_LARGE_ANGULAR_CHANGE_RATE_RADIANS_PER_SECOND
    {
        return false;
    }
    let (Some(old_a), Some(old_b), Some(old_c), Some(new_a), Some(new_b), Some(new_c)) = (
        previous_positions.get(first),
        previous_positions.get(second),
        previous_positions.get(third),
        bones.get(first),
        bones.get(second),
        bones.get(third),
    ) else {
        return false;
    };
    let old_lower = old_joint.rotation * (old_joint.scale * old_tip.translation);
    let new_lower = new_joint.rotation * (new_joint.scale * new_tip.translation);
    for (upper, lower, actual_upper, actual_lower) in [
        (
            old_joint.translation,
            old_lower,
            *old_b - *old_a,
            *old_c - *old_b,
        ),
        (
            new_joint.translation,
            new_lower,
            new_b.0 - new_a.0,
            new_c.0 - new_b.0,
        ),
    ] {
        let local_lengths = upper.length() * lower.length();
        let actual_lengths = actual_upper.length() * actual_lower.length();
        if !local_lengths.is_finite()
            || !actual_lengths.is_finite()
            || local_lengths <= f32::EPSILON
            || actual_lengths <= f32::EPSILON
            || (upper.cross(lower).length() / local_lengths
                - actual_upper.cross(actual_lower).length() / actual_lengths)
                .abs()
                > TEMPORAL_LIMB_BEND_SINE
        {
            return false;
        }
    }
    (1..32).any(|index| {
        let alpha = index as f32 / 32.0;
        let upper = old_joint.translation.lerp(new_joint.translation, alpha);
        let lower = old_joint.rotation.slerp(new_joint.rotation, alpha)
            * (old_joint.scale.lerp(new_joint.scale, alpha)
                * old_tip.translation.lerp(new_tip.translation, alpha));
        let lengths = upper.length() * lower.length();
        lengths.is_finite()
            && lengths > f32::EPSILON
            && upper.cross(lower).length() / lengths < TEMPORAL_LIMB_BEND_SINE
    })
}

fn segment_lengths(bones: &BTreeMap<String, FinalBone>) -> BTreeMap<String, f32> {
    SEGMENTS
        .iter()
        .filter_map(|(first, second)| {
            let first_position = bones.get(*first)?.0;
            let second_position = bones.get(*second)?.0;
            Some((
                segment_name(first, second),
                first_position.distance(second_position),
            ))
        })
        .collect()
}

impl TemporalSkeletonGuard {
    fn reset(&mut self) {
        self.previous_relative.clear();
        self.previous_local.clear();
        self.previous_segments.clear();
        self.previous_limb_planes.clear();
    }

    fn observe(
        &mut self,
        local: &BTreeMap<String, LocalBonePose>,
        bones: &BTreeMap<String, FinalBone>,
        avatar_position: Vec3,
        frame_seconds: f32,
    ) -> TemporalSkeletonObservation {
        let mut metrics = Vec::new();
        for (name, (translation, _, _, _)) in bones {
            let relative = *translation - avatar_position;
            if let Some(previous) = self.previous_relative.get(name) {
                let speed = previous.distance(relative) / frame_seconds;
                if speed > TEMPORAL_GLOBAL_POSITION_SPEED_METERS_PER_SECOND {
                    metrics.push(TemporalSkeletonMetric {
                        kind: "global_position_speed",
                        subject: name.clone(),
                        previous: 0.0,
                        current: speed,
                        threshold: TEMPORAL_GLOBAL_POSITION_SPEED_METERS_PER_SECOND,
                        trigger: "rate",
                        rate: Some(speed),
                        rate_threshold: Some(TEMPORAL_GLOBAL_POSITION_SPEED_METERS_PER_SECOND),
                    });
                }
            }
        }
        for (name, current) in local {
            if let Some(previous) = self.previous_local.get(name).copied() {
                let translation_delta = previous.translation.distance(current.translation);
                let translation_speed = translation_delta / frame_seconds;
                let translation_trigger =
                    if translation_delta > TEMPORAL_LOCAL_TRANSLATION_JUMP_METERS {
                        Some(("jump", TEMPORAL_LOCAL_TRANSLATION_JUMP_METERS))
                    } else if translation_delta > TEMPORAL_LOCAL_TRANSLATION_JUMP_METERS * 0.35
                        && translation_speed > TEMPORAL_LOCAL_TRANSLATION_SPEED_METERS_PER_SECOND
                    {
                        Some(("rate", TEMPORAL_LOCAL_TRANSLATION_SPEED_METERS_PER_SECOND))
                    } else {
                        None
                    };
                if let Some((trigger, threshold)) = translation_trigger {
                    metrics.push(TemporalSkeletonMetric {
                        kind: "local_translation_spike",
                        subject: name.clone(),
                        previous: 0.0,
                        current: translation_delta,
                        threshold,
                        trigger,
                        rate: Some(translation_speed),
                        rate_threshold: Some(TEMPORAL_LOCAL_TRANSLATION_SPEED_METERS_PER_SECOND),
                    });
                }
                let rotation_delta = rotation_delta_radians(previous.rotation, current.rotation);
                let rotation_speed = rotation_delta / frame_seconds;
                let rotation_trigger = if rotation_delta > TEMPORAL_LOCAL_ROTATION_JUMP_RADIANS
                    && rotation_speed > TEMPORAL_LARGE_ANGULAR_CHANGE_RATE_RADIANS_PER_SECOND
                {
                    Some(("jump", TEMPORAL_LOCAL_ROTATION_JUMP_RADIANS))
                } else if rotation_delta > TEMPORAL_LOCAL_ROTATION_SPEED_MIN_DELTA_RADIANS
                    && rotation_speed > TEMPORAL_LOCAL_ROTATION_SPEED_RADIANS_PER_SECOND
                {
                    Some(("rate", TEMPORAL_LOCAL_ROTATION_SPEED_RADIANS_PER_SECOND))
                } else {
                    None
                };
                if let Some((trigger, threshold)) = rotation_trigger {
                    metrics.push(TemporalSkeletonMetric {
                        kind: "local_rotation_spike",
                        subject: name.clone(),
                        previous: 0.0,
                        current: rotation_delta,
                        threshold,
                        trigger,
                        rate: Some(rotation_speed),
                        rate_threshold: Some(if trigger == "jump" {
                            TEMPORAL_LARGE_ANGULAR_CHANGE_RATE_RADIANS_PER_SECOND
                        } else {
                            TEMPORAL_LOCAL_ROTATION_SPEED_RADIANS_PER_SECOND
                        }),
                    });
                }
                let scale_ratio = temporal_scale_ratio(previous.scale, current.scale);
                if scale_ratio > TEMPORAL_SCALE_RATIO {
                    metrics.push(TemporalSkeletonMetric {
                        kind: "local_scale_spike",
                        subject: name.clone(),
                        previous: 1.0,
                        current: scale_ratio,
                        threshold: TEMPORAL_SCALE_RATIO,
                        trigger: "ratio",
                        rate: None,
                        rate_threshold: None,
                    });
                }
            }
        }
        let segments = segment_lengths(bones);
        for (name, length) in &segments {
            if let Some(previous) = self.previous_segments.get(name) {
                let delta = (*length - previous).abs();
                let ratio = delta / previous.abs().max(0.01);
                if delta > TEMPORAL_SEGMENT_MIN_DELTA_METERS && ratio > TEMPORAL_SEGMENT_RATIO {
                    metrics.push(TemporalSkeletonMetric {
                        kind: "segment_length_spike",
                        subject: name.clone(),
                        previous: *previous,
                        current: *length,
                        threshold: TEMPORAL_SEGMENT_RATIO,
                        trigger: "ratio",
                        rate: None,
                        rate_threshold: None,
                    });
                }
            }
            self.previous_segments.insert(name.clone(), *length);
        }
        let mut limb_planes = BTreeMap::new();
        for (name, first, second, third) in LIMB_CHAINS {
            let Some(current) = limb_plane(bones, first, second, third) else {
                continue;
            };
            if let Some(previous) = self.previous_limb_planes.get(*name) {
                let dot = previous.normal.dot(current.normal);
                let plane_rate = dot.clamp(-1.0, 1.0).acos() / frame_seconds;
                if dot < TEMPORAL_LIMB_PLANE_FLIP_DOT
                    && plane_rate > TEMPORAL_LARGE_ANGULAR_CHANGE_RATE_RADIANS_PER_SECOND
                    && !limb_crosses_degenerate_joint_plane(
                        &self.previous_local,
                        local,
                        &self.previous_relative,
                        bones,
                        first,
                        second,
                        third,
                        frame_seconds,
                    )
                {
                    metrics.push(TemporalSkeletonMetric {
                        kind: "limb_plane_flip",
                        subject: (*name).to_owned(),
                        previous: 1.0,
                        current: dot,
                        threshold: TEMPORAL_LIMB_PLANE_FLIP_DOT,
                        trigger: "plane_flip",
                        rate: Some(plane_rate),
                        rate_threshold: Some(TEMPORAL_LARGE_ANGULAR_CHANGE_RATE_RADIANS_PER_SECOND),
                    });
                }
            }
            limb_planes.insert((*name).to_owned(), current);
        }
        self.previous_limb_planes = limb_planes;
        for (name, current) in local {
            self.previous_local.insert(name.clone(), *current);
        }
        for (name, (translation, _, _, _)) in bones {
            self.previous_relative
                .insert(name.clone(), *translation - avatar_position);
        }
        TemporalSkeletonObservation { metrics }
    }
}

fn record_temporal_metrics(
    state: &mut AnimationVisualValidationState,
    metrics: Vec<TemporalSkeletonMetric>,
    stage: &str,
    physics_readback: Option<&ScaledRagdollReadback>,
    elapsed_secs: f64,
    frame_ms: f64,
    animation_delta_seconds: f32,
    direction_case: Option<&str>,
    phase: Option<&str>,
) -> bool {
    let mut detected = false;
    let source_landing = stage == "pre_rig_source_landing_selector";
    let stage = if source_landing { "pre_rig" } else { stage };
    for metric in metrics {
        let physics_readback =
            physics_readback.filter(|readback| stage == "final" && readback.covers(&metric));
        let source_boundary = if physics_readback.is_some() {
            Some("source_live_scaled_joint_readback")
        } else {
            (source_landing
                && metric.kind == "global_position_speed"
                && matches!(
                    metric.subject.as_str(),
                    "hand_l" | "hand_r" | "lowerarm_l" | "lowerarm_r"
                ))
            .then_some("source_fall_heavy_to_light_on_physics_entry")
        };
        let duplicate = state.temporal_anomalies.iter().any(|value| {
            value.kind == metric.kind
                && value.subject == metric.subject
                && value.source_boundary == source_boundary
        });
        if !duplicate && state.temporal_anomalies.len() < MAX_TEMPORAL_ANOMALIES {
            warn!(
                "[ANIMATION-VISUAL][TEMPORAL] stage={} kind={} trigger={} subject={} previous={:.6} current={:.6} threshold={:.6} rate={:?} rate_threshold={:?} elapsed={:.6} frame_ms={:.3}",
                stage,
                metric.kind,
                metric.trigger,
                metric.subject,
                metric.previous,
                metric.current,
                metric.threshold,
                metric.rate,
                metric.rate_threshold,
                elapsed_secs,
                frame_ms,
            );
            state.temporal_anomalies.push(TemporalSkeletonAnomaly {
                stage: stage.to_owned(),
                kind: metric.kind.to_owned(),
                subject: metric.subject.clone(),
                previous: metric.previous,
                current: metric.current,
                threshold: metric.threshold,
                trigger: metric.trigger.to_owned(),
                rate: metric.rate,
                rate_threshold: metric.rate_threshold,
                elapsed_secs,
                frame_ms,
                animation_delta_seconds,
                direction_case: direction_case.map(str::to_owned),
                phase: phase.map(str::to_owned),
                source_boundary,
                source_selector_inputs: None,
                source_physics_readback: physics_readback.cloned(),
            });
        }
        if source_boundary.is_some() {
            continue;
        }
        detected |= append_unique(
            &mut state.faults,
            format!("temporal_{stage}_{}:{}", metric.kind, metric.subject),
        );
    }
    detected
}

fn animation_eval_start(mut state: ResMut<AnimationVisualValidationState>) {
    state.eval_started = Some(Instant::now());
}

fn animation_cpu_frame_start(
    mut state: ResMut<AnimationVisualValidationState>,
    startup: Option<Res<ManualPlaygroundStartupReady>>,
    manual_integration: Option<Res<ManualIntegrationGateReport>>,
) {
    state.frame_started = Some(Instant::now());
    state.playable_frame = manual_integration.and_then(|_| startup.map(|startup| startup.passed));
}

fn record_stage(
    config: &AnimationVisualValidationConfig,
    visual: &AnimationVisualValidationState,
    stages: &mut AnimationVisualStageTiming,
    name: &str,
    started: Option<Instant>,
) {
    if visual.performance_ready(config.warmup)
        && let Some(started) = started
    {
        stages
            .samples
            .entry(name.to_owned())
            .or_default()
            .push(started.elapsed().as_secs_f64() * 1000.0);
    }
}

fn evidence_capture_complete(
    config: &AnimationVisualValidationConfig,
    state: &AnimationVisualValidationState,
    stages: &AnimationVisualStageTiming,
) -> bool {
    state.started.is_some()
        && state.pose_samples.len() >= config.min_pose_samples
        && state.frame_ms.len() >= config.min_pose_samples
        && state.eval_ms.len() >= config.min_pose_samples
        && stages.physics_steps_per_render.len() >= config.min_pose_samples
        && state.screenshots.len() >= config.capture_count
        && state.required_motion_windows.iter().all(|label| {
            state.motion_windows.get(label).is_some_and(|window| {
                window.sample_count >= 2
                    && (!window.requires_non_reference_root_pose
                        || window.root_pose_sample_count >= 2)
            })
        })
        && state
            .screenshots
            .iter()
            .all(|screenshot| Path::new(&screenshot.path).is_file())
        && (config.capture_count == 0
            || state.required_motion_windows.iter().all(|label| {
                state.screenshots.iter().any(|screenshot| {
                    screenshot.kind == "direction"
                        && screenshot.direction_case.as_deref() == Some(label.as_str())
                })
            }))
}

fn update_animation_visual_progress(
    config: Res<AnimationVisualValidationConfig>,
    state: Res<AnimationVisualValidationState>,
    stages: Res<AnimationVisualStageTiming>,
    mut progress: ResMut<AnimationVisualValidationProgress>,
) {
    progress.evidence_ready = evidence_capture_complete(&config, &state, &stages);
}

fn automation_watchdog_should_abort(
    manual_completed: bool,
    evidence_ready: bool,
    capture_progressed: bool,
    stalled_for: Duration,
) -> bool {
    manual_completed
        && !evidence_ready
        && !capture_progressed
        && stalled_for >= AUTOMATION_PROGRESS_STALL
}

fn abort_stalled_animation_visual_validation(
    config: Res<AnimationVisualValidationConfig>,
    progress: Res<AnimationVisualValidationProgress>,
    stages: Res<AnimationVisualStageTiming>,
    mut state: ResMut<AnimationVisualValidationState>,
    manual_integration: Option<Res<ManualIntegrationGateReport>>,
    mut exit: MessageWriter<AppExit>,
) {
    let manual_completed = manual_integration
        .as_deref()
        .is_some_and(|report| report.completed);
    if !manual_completed || progress.evidence_ready || state.watchdog.triggered {
        state.watchdog = AutomationProgressWatchdog::default();
        return;
    }
    let now = Instant::now();
    let capture_count = state.screenshots.len();
    let capture_progressed = state.watchdog.last_progress.is_none()
        || state.watchdog.last_capture_count != capture_count;
    if capture_progressed {
        state.watchdog.last_capture_count = capture_count;
        state.watchdog.last_progress = Some(now);
        return;
    }
    let stalled_for = now.duration_since(state.watchdog.last_progress.unwrap_or(now));
    if automation_watchdog_should_abort(
        manual_completed,
        progress.evidence_ready,
        capture_progressed,
        stalled_for,
    ) {
        state.watchdog.triggered = true;
        state.watchdog.stalled_seconds = stalled_for.as_secs_f64();
        append_unique(
            &mut state.faults,
            "automation_visual_capture_stalled".to_owned(),
        );
        write_report(&config, &state, &stages, manual_integration.as_deref());
        exit.write(AppExit::Error(NonZeroU8::new(94).unwrap()));
    }
}

fn animation_visual_physics_start(mut stages: ResMut<AnimationVisualStageTiming>) {
    stages.physics_started = Some(Instant::now());
}

fn animation_visual_physics_end(
    config: Res<AnimationVisualValidationConfig>,
    visual: Res<AnimationVisualValidationState>,
    mut stages: ResMut<AnimationVisualStageTiming>,
) {
    if visual.started.is_some() {
        stages.physics_steps_since_render += 1;
    }
    let started = stages.physics_started.take();
    record_stage(&config, &visual, &mut stages, "physics", started);
}

fn animation_visual_physics_step_start(mut stages: ResMut<AnimationVisualStageTiming>) {
    stages.physics_step_started = Some(Instant::now());
}

fn animation_visual_physics_step_end(
    config: Res<AnimationVisualValidationConfig>,
    visual: Res<AnimationVisualValidationState>,
    mut stages: ResMut<AnimationVisualStageTiming>,
) {
    let started = stages.physics_step_started.take();
    record_stage(&config, &visual, &mut stages, "physics_step", started);
}

fn animation_visual_control_rig_time(
    config: Res<AnimationVisualValidationConfig>,
    visual: Res<AnimationVisualValidationState>,
    mut stages: ResMut<AnimationVisualStageTiming>,
    executions: Query<&AlsControlRigExecution, With<AlsCharacter>>,
) {
    if visual.performance_ready(config.warmup)
        && let Ok(execution) = executions.single()
        && execution.executed_steps > 0
    {
        stages
            .samples
            .entry("control_rig".into())
            .or_default()
            .push(execution.elapsed_seconds * 1000.0);
    }
}

fn animation_visual_camera_start(mut stages: ResMut<AnimationVisualStageTiming>) {
    stages.camera_started = Some(Instant::now());
}

fn animation_visual_camera_end(
    config: Res<AnimationVisualValidationConfig>,
    visual: Res<AnimationVisualValidationState>,
    mut stages: ResMut<AnimationVisualStageTiming>,
) {
    let started = stages.camera_started.take();
    record_stage(&config, &visual, &mut stages, "camera", started);
}

fn animation_eval_end(
    config: Res<AnimationVisualValidationConfig>,
    mut state: ResMut<AnimationVisualValidationState>,
) {
    if let Some(begin) = state.eval_started.take()
        && state.performance_ready(config.warmup)
    {
        state.eval_ms.push(begin.elapsed().as_secs_f64() * 1000.0);
    }
}

fn collect_bones(
    root: Entity,
    children: &Query<&Children>,
    named_transforms: &Query<(&Name, &GlobalTransform, &Transform)>,
) -> BTreeMap<String, FinalBone> {
    let mut result = BTreeMap::new();
    let mut stack = vec![root];
    while let Some(entity) = stack.pop() {
        if let Ok((name, transform, local)) = named_transforms.get(entity) {
            let label = name.as_str();
            if REQUIRED_BONES.iter().any(|required| *required == label) {
                let (scale, rotation, translation) = transform.to_scale_rotation_translation();
                result.entry(label.to_owned()).or_insert((
                    translation,
                    rotation,
                    scale,
                    local.scale,
                ));
            }
        }
        if let Ok(child_entities) = children.get(entity) {
            stack.extend(child_entities.iter());
        }
    }
    result
}

fn collect_local_motion_bones(
    root: Entity,
    children: &Query<&Children>,
    named_transforms: &Query<(&Name, &GlobalTransform, &Transform)>,
) -> BTreeMap<String, LocalBonePose> {
    let mut result = BTreeMap::new();
    let mut stack = vec![root];
    while let Some(entity) = stack.pop() {
        if let Ok((name, _, local)) = named_transforms.get(entity) {
            let label = name.as_str();
            if REQUIRED_BONES.iter().any(|required| *required == label) {
                result.entry(label.to_owned()).or_insert(LocalBonePose {
                    translation: local.translation,
                    rotation: local.rotation,
                    scale: local.scale,
                });
            }
        }
        if let Ok(child_entities) = children.get(entity) {
            stack.extend(child_entities.iter());
        }
    }
    result
}

fn capture_animation_visual_pre_rig_pose(
    time: Res<Time>,
    mut pre_rig: ResMut<AnimationVisualPreRigPose>,
    avatars: Query<&AnimatedSceneInstance, With<AlsCharacterAvatar>>,
    players: Query<&AnimationGraphPlayer>,
    transforms: bevy::transform::helper::TransformHelper,
    graphs: Res<Assets<AnimationGraph>>,
    skeletons: Res<Assets<Skeleton>>,
    bodies: Query<(
        &avian3d::prelude::RigidBody,
        &bevy_als::extensions::RagdollActiveMode,
    )>,
) {
    let previous_selector = pre_rig.landing_selector.take();
    let previously_active = pre_rig.physics_readback_active;
    pre_rig.physics_readback_active = false;
    pre_rig.source_selector_inputs = None;
    pre_rig.bones.clear();
    pre_rig.global_bones.clear();
    pre_rig.delta_seconds = time.delta_secs();
    let Ok(instance) = avatars.single() else {
        return;
    };
    let Ok(player) = players.get(instance.player_entity()) else {
        return;
    };
    let Ok(player_world) = transforms.compute_global_transform(instance.player_entity()) else {
        return;
    };
    let Some((context, node, _)) =
        bevy_als::extensions::root_graph_boundaries::current_control_rig(player, &graphs)
    else {
        return;
    };
    let rig = node
        .try_inner_downcast_ref::<bevy_als::extensions::root_graph_boundaries::ControlRigBoundaryNode>(
        )
        .unwrap();
    let Some(graph) = graphs.get(context.get_graph_id()) else {
        return;
    };
    let Some(SourcePin::NodeData(source, pin)) = graph
        .edges_inverted
        .get(&TargetPin::NodeData(node.id, rig.pose_input.clone()))
    else {
        return;
    };
    let Some(DataValue::Pose(pose)) =
        context
            .node_caches
            .get_output_data(*source, StateKey::Default, pin.clone())
    else {
        return;
    };
    let Some(skeleton) = skeletons.get(&pose.skeleton) else {
        return;
    };
    if let Some(arena) = player.get_context_arena() {
        for id in arena.iter_context_ids() {
            let Some(context) = arena.get_context(id) else {
                continue;
            };
            let Some(graph) = graphs.get(context.get_graph_id()) else {
                continue;
            };
            let Some(node) = graph
                .nodes
                .values()
                .find(|node| node.id.uuid().to_string() == "10be798e-43e2-1f53-a3b6-6aaf258498ff")
            else {
                continue;
            };
            if !context.node_caches.is_updated(node.id, StateKey::Default) {
                continue;
            }
            let Some(blend) =
                node.try_inner_downcast_ref::<bevy_als::extensions::SourceTwoWayBlendNode>()
            else {
                continue;
            };
            let mapping = &blend.mapping;
            if blend.alpha_key != "in_air_state.vertical_velocity_world_space"
                || !mapping.source_semantics
                || !mapping.map_range
                || mapping.interpolate_result
                || mapping.input_min != -1000.0
                || mapping.input_max != -500.0
                || mapping.output_min != 0.0
                || mapping.output_max != 1.0
                || mapping.scale != 1.0
                || mapping.bias != 0.0
                || mapping.clamp_result
                || blend.alpha_scale != 1.0
                || blend.alpha_bias != 0.0
            {
                continue;
            }
            let Ok(state) = context
                .node_states
                .get::<bevy_als::extensions::source_pose_nodes::SourceTwoWayBlendState>(
                node.id,
                StateKey::Default,
            ) else {
                continue;
            };
            pre_rig.landing_selector = Some([
                (f64::from(state.input_alpha) * 0.01) as f32,
                state.blend_alpha,
            ]);
        }
    }
    pre_rig.physics_readback_active = player
        .get_outputs()
        .get("ragdoll_config")
        .and_then(|value| value.as_ragdoll_config().ok())
        .is_some_and(|config| {
            config.default_readback == Some(true)
                && config.readback_overrides.values().all(|enabled| *enabled)
        })
        && player.get_spawned_ragdoll().is_some_and(|spawned| {
            !spawned.bodies.is_empty()
                && spawned.bodies.values().all(|entity| {
                    bodies.get(*entity).is_ok_and(|(actual, expected)| {
                        *actual == match expected.0 {
                        bevy_animation_graph::core::ragdoll::definition::BodyMode::Dynamic => {
                            avian3d::prelude::RigidBody::Dynamic
                        }
                        bevy_animation_graph::core::ragdoll::definition::BodyMode::Kinematic => {
                            avian3d::prelude::RigidBody::Kinematic
                        }
                    }
                    })
                })
        });
    if source_landing_selector_boundary(
        previous_selector,
        pre_rig.landing_selector,
        previously_active,
        pre_rig.physics_readback_active,
    ) {
        pre_rig.source_selector_inputs = previous_selector
            .zip(pre_rig.landing_selector)
            .map(|(previous, current)| [previous, current]);
    }
    pre_rig.global_bones = pose_global_bones(&pose, skeleton, &player_world);
    pre_rig.root_position = player_world.translation();
    for path in skeleton.iter_paths() {
        let Some(name) = path.parts.last() else {
            continue;
        };
        if !REQUIRED_BONES.contains(&name.as_str()) {
            continue;
        }
        let Some(reference) = skeleton.default_transforms(path.id()) else {
            continue;
        };
        let bone = pose.get_bone(path.id());
        pre_rig.bones.insert(
            name.to_string(),
            LocalBonePose {
                translation: bone
                    .and_then(|bone| bone.translation)
                    .unwrap_or(reference.local.translation),
                rotation: bone
                    .and_then(|bone| bone.rotation)
                    .unwrap_or(reference.local.rotation),
                scale: bone
                    .and_then(|bone| bone.scale)
                    .unwrap_or(reference.local.scale),
            },
        );
    }
}

fn source_landing_selector_boundary(
    previous: Option<[f32; 2]>,
    current: Option<[f32; 2]>,
    previously_active: bool,
    active: bool,
) -> bool {
    !previously_active
        && active
        && previous.zip(current).is_some_and(|(previous, current)| {
            previous[0].is_finite()
                && current[0].is_finite()
                && previous[0] <= -10.0
                && previous[1] == 0.0
                && current[0] >= -5.0
                && current[1] == 1.0
        })
}

fn pose_global_bones(
    pose: &Pose,
    skeleton: &Skeleton,
    root: &GlobalTransform,
) -> BTreeMap<String, FinalBone> {
    let mut output = BTreeMap::new();
    for path in skeleton.iter_paths() {
        let Some(name) = path.parts.last() else {
            continue;
        };
        if !REQUIRED_BONES.contains(&name.as_str()) {
            continue;
        }
        let mut chain = Vec::new();
        let mut current = Some(path.id());
        for _ in 0..path.parts.len() {
            let Some(id) = current else {
                break;
            };
            chain.push(id);
            current = skeleton.parent(&id);
        }
        let mut matrix = root.to_matrix();
        let mut local_scale = Vec3::ONE;
        for id in chain.into_iter().rev() {
            let Some(reference) = skeleton.default_transforms(id) else {
                continue;
            };
            let local = pose.get_bone(id).map_or(reference.local, |bone| {
                bone.to_transform_with_base(reference.local)
            });
            matrix *= local.to_matrix();
            local_scale = local.scale;
        }
        let (scale, rotation, translation) = matrix.to_scale_rotation_translation();
        output.insert(
            name.to_string(),
            (translation, rotation, scale, local_scale),
        );
    }
    output
}

fn rotation_delta_radians(previous: Quat, current: Quat) -> f32 {
    if !previous.is_finite()
        || !current.is_finite()
        || previous.length_squared() <= f32::EPSILON
        || current.length_squared() <= f32::EPSILON
    {
        return f32::INFINITY;
    }
    (2.0 * previous
        .normalize()
        .dot(current.normalize())
        .abs()
        .clamp(0.0, 1.0)
        .acos())
    .max(0.0)
}

fn root_pose_activity_from_pose(pose: &Pose) -> RootPoseActivity {
    let mut activity = RootPoseActivity {
        output_present: true,
        bone_count: pose.paths.len(),
        ..default()
    };
    for BonePose {
        translation,
        rotation,
        scale,
        weights,
    } in pose
        .paths
        .values()
        .filter_map(|index| pose.bones.get(*index))
    {
        activity.active_channel_count += translation.is_some() as usize;
        activity.active_channel_count += rotation.is_some() as usize;
        activity.active_channel_count += scale.is_some() as usize;
        activity.active_channel_count += weights.is_some() as usize;
    }
    activity
}

fn root_pose_activity(player: &AnimationGraphPlayer) -> RootPoseActivity {
    match player.get_outputs().get("pose") {
        Some(DataValue::Pose(pose)) => root_pose_activity_from_pose(pose),
        _ => RootPoseActivity::default(),
    }
}

fn root_pose_has_animation_channels(activity: RootPoseActivity) -> bool {
    activity.output_present && activity.active_channel_count > 0
}

fn graph_dependency_state(asset_server: &AssetServer, player: &AnimationGraphPlayer) -> String {
    match player.get_animation_source() {
        AnimationSource::Graph(handle) => asset_server
            .get_recursive_dependency_load_state(handle.id())
            .map(|state| format!("{state:?}"))
            .unwrap_or_else(|| "Unknown".to_owned()),
        _ => "NotGraph".to_owned(),
    }
}

fn root_pose_sample(activity: RootPoseActivity) -> RootPoseSample {
    RootPoseSample {
        output_present: activity.output_present,
        bone_count: activity.bone_count,
        active_channel_count: activity.active_channel_count,
    }
}

fn source_pin_failure_pin(pin: &SourcePin) -> GraphFailurePin {
    match pin {
        SourcePin::NodeData(node, pin) => GraphFailurePin {
            kind: "node_data".to_owned(),
            node_id: Some(format!("{node:?}")),
            pin_id: Some(pin.clone()),
            graph_context: None,
        },
        SourcePin::InputData(pin) => GraphFailurePin {
            kind: "input_data".to_owned(),
            node_id: None,
            pin_id: Some(format!("{pin:?}")),
            graph_context: None,
        },
        SourcePin::NodeTime(node) => GraphFailurePin {
            kind: "node_time".to_owned(),
            node_id: Some(format!("{node:?}")),
            pin_id: None,
            graph_context: None,
        },
        SourcePin::InputTime(pin) => GraphFailurePin {
            kind: "input_time".to_owned(),
            node_id: None,
            pin_id: Some(format!("{pin:?}")),
            graph_context: None,
        },
    }
}

fn target_pin_failure_pin(pin: &TargetPin) -> GraphFailurePin {
    match pin {
        TargetPin::NodeData(node, pin) => GraphFailurePin {
            kind: "node_data".to_owned(),
            node_id: Some(format!("{node:?}")),
            pin_id: Some(pin.clone()),
            graph_context: None,
        },
        TargetPin::OutputData(pin) => GraphFailurePin {
            kind: "output_data".to_owned(),
            node_id: None,
            pin_id: Some(pin.clone()),
            graph_context: None,
        },
        TargetPin::NodeTime(node, pin) => GraphFailurePin {
            kind: "node_time".to_owned(),
            node_id: Some(format!("{node:?}")),
            pin_id: Some(pin.clone()),
            graph_context: None,
        },
        TargetPin::OutputTime => GraphFailurePin {
            kind: "output_time".to_owned(),
            node_id: None,
            pin_id: None,
            graph_context: None,
        },
    }
}

fn graph_error_kind(error: &GraphError) -> String {
    format!("{error:?}")
        .split(['(', '{', ' '])
        .next()
        .unwrap_or("GraphError")
        .to_owned()
}

fn graph_error_pin(error: &GraphError) -> Option<GraphFailurePin> {
    match error {
        GraphError::MissingEdgeToSource(pin)
        | GraphError::DurationMissing(pin)
        | GraphError::TimeUpdateMissingFwd(pin) => Some(source_pin_failure_pin(pin)),
        GraphError::MissingEdgeToTarget(pin) | GraphError::TimeUpdateMissingBack(pin) => {
            Some(target_pin_failure_pin(pin))
        }
        GraphError::OutputMissing { graph, node, pin }
        | GraphError::ExtraTimeUpdateNotAvailable { graph, node, pin } => Some(GraphFailurePin {
            kind: "node_output".to_owned(),
            node_id: Some(format!("{node:?}")),
            pin_id: Some(pin.clone()),
            graph_context: Some(format!("{graph:?}")),
        }),
        GraphError::UnknownErrorNode { graph, node, .. } => Some(GraphFailurePin {
            kind: "node".to_owned(),
            node_id: Some(format!("{node:?}")),
            pin_id: None,
            graph_context: Some(format!("{graph:?}")),
        }),
        _ => None,
    }
}

fn graph_error_node_id(error: &GraphError) -> Option<NodeId> {
    match error {
        GraphError::MissingEdgeToSource(SourcePin::NodeData(node, _))
        | GraphError::MissingEdgeToSource(SourcePin::NodeTime(node))
        | GraphError::DurationMissing(SourcePin::NodeData(node, _))
        | GraphError::DurationMissing(SourcePin::NodeTime(node))
        | GraphError::TimeUpdateMissingFwd(SourcePin::NodeData(node, _))
        | GraphError::TimeUpdateMissingFwd(SourcePin::NodeTime(node))
        | GraphError::MissingEdgeToTarget(TargetPin::NodeData(node, _))
        | GraphError::MissingEdgeToTarget(TargetPin::NodeTime(node, _))
        | GraphError::TimeUpdateMissingBack(TargetPin::NodeData(node, _))
        | GraphError::TimeUpdateMissingBack(TargetPin::NodeTime(node, _))
        | GraphError::OutputMissing { node, .. }
        | GraphError::ExtraTimeUpdateNotAvailable { node, .. }
        | GraphError::UnknownErrorNode { node, .. } => Some(*node),
        _ => None,
    }
}

fn add_graph_failure_node_candidate(
    candidates: &mut Vec<GraphFailureNodeCandidate>,
    asset_server: &AssetServer,
    graph_id: bevy::asset::AssetId<AnimationGraph>,
    graph: &AnimationGraph,
    node_id: NodeId,
    graph_context: Option<String>,
    active_context: bool,
    context_curves: BTreeMap<String, f32>,
) {
    let Some(node) = graph.nodes.get(&node_id) else {
        return;
    };
    let graph_asset_id = format!("{graph_id:?}");
    if let Some(existing) = candidates.iter_mut().find(|candidate| {
        candidate.graph_asset_id == graph_asset_id && candidate.node_id == format!("{node_id:?}")
    }) {
        existing.active_context |= active_context;
        if existing.graph_context.is_none() {
            existing.graph_context = graph_context;
        }
        if existing.context_curves.is_empty() {
            existing.context_curves = context_curves;
        }
        return;
    }
    candidates.push(GraphFailureNodeCandidate {
        graph_path: asset_server
            .get_path(graph_id)
            .map(|path| path.path().to_string_lossy().into_owned()),
        graph_asset_id,
        graph_context,
        node_id: format!("{node_id:?}"),
        node_name: node.name.clone(),
        node_kind: node.inner_ref().display_name(),
        active_context,
        context_curves,
    });
}

fn graph_failure_node_candidates(
    player: &AnimationGraphPlayer,
    asset_server: &AssetServer,
    graphs: &Assets<AnimationGraph>,
    node_id: NodeId,
) -> Vec<GraphFailureNodeCandidate> {
    let mut candidates = Vec::new();
    if let Some(arena) = player.get_context_arena() {
        for context_id in arena.iter_context_ids() {
            let Some(context) = arena.get_context(context_id) else {
                continue;
            };
            let graph_id = context.get_graph_id();
            let Some(graph) = graphs.get(graph_id) else {
                continue;
            };
            let context_curves = bevy_als::extensions::animation_context::node_context_state(
                &context.node_states,
                StateKey::Default,
                node_id,
            )
            .map(|state| state.curves.clone())
            .unwrap_or_default()
            .into_iter()
            .filter(|(_, value)| value.is_finite())
            .map(|(name, value)| (name.to_string(), value))
            .collect();
            add_graph_failure_node_candidate(
                &mut candidates,
                asset_server,
                graph_id,
                graph,
                node_id,
                Some(format!("{context_id:?}")),
                true,
                context_curves,
            );
        }
    }
    for (graph_id, graph) in graphs.iter() {
        add_graph_failure_node_candidate(
            &mut candidates,
            asset_server,
            graph_id,
            graph,
            node_id,
            None,
            false,
            BTreeMap::new(),
        );
    }
    candidates.sort_by(|left, right| {
        right
            .active_context
            .cmp(&left.active_context)
            .then_with(|| left.graph_path.cmp(&right.graph_path))
            .then_with(|| left.graph_asset_id.cmp(&right.graph_asset_id))
    });
    candidates
}

fn graph_failure_context_nodes(
    player: &AnimationGraphPlayer,
    asset_server: &AssetServer,
    graphs: &Assets<AnimationGraph>,
) -> Vec<GraphFailureContextNode> {
    let Some(arena) = player.get_context_arena() else {
        return Vec::new();
    };
    let mut nodes = Vec::new();
    for context_id in arena.iter_context_ids() {
        let Some(context) = arena.get_context(context_id) else {
            continue;
        };
        let graph_id = context.get_graph_id();
        let Some(graph) = graphs.get(graph_id) else {
            continue;
        };
        for node in graph.nodes.values() {
            let context_curves = bevy_als::extensions::animation_context::node_context_state(
                &context.node_states,
                StateKey::Default,
                node.id,
            )
            .map(|state| state.curves.clone());
            let Some(context_curves) = context_curves else {
                continue;
            };
            nodes.push(GraphFailureContextNode {
                graph_path: asset_server
                    .get_path(graph_id)
                    .map(|path| path.path().to_string_lossy().into_owned()),
                graph_context: format!("{context_id:?}"),
                node_id: format!("{:?}", node.id),
                node_name: node.name.clone(),
                node_kind: node.inner_ref().display_name(),
                context_curves: context_curves
                    .into_iter()
                    .filter(|(_, value)| value.is_finite())
                    .map(|(name, value)| (name.to_string(), value))
                    .collect(),
            });
        }
    }
    nodes.sort_by(|left, right| {
        left.graph_context
            .cmp(&right.graph_context)
            .then_with(|| left.node_name.cmp(&right.node_name))
    });
    nodes.truncate(128);
    nodes
}

fn graph_error_pose_classification(root_pose: RootPoseActivity) -> String {
    if !root_pose.output_present {
        "missing_root_pose_likely_reference_or_a_pose".to_owned()
    } else if !root_pose_has_animation_channels(root_pose) {
        "reference_root_pose_likely_a_pose".to_owned()
    } else {
        "root_pose_channels_present_graph_error".to_owned()
    }
}

fn graph_evaluation_failure(
    config: &AnimationVisualValidationConfig,
    error: Option<&GraphError>,
    player: &AnimationGraphPlayer,
    asset_server: &AssetServer,
    graphs: &Assets<AnimationGraph>,
    root_pose: RootPoseActivity,
    direction_case: Option<String>,
    phase: Option<String>,
    phase_elapsed_secs: Option<f32>,
    locomotion: Option<String>,
    gait: Option<String>,
    stance: Option<String>,
    rotation_mode: Option<String>,
    reported_before_ready: bool,
) -> GraphEvaluationFailure {
    let node_candidates = error
        .and_then(graph_error_node_id)
        .map(|node_id| graph_failure_node_candidates(player, asset_server, graphs, node_id))
        .unwrap_or_default();
    let active_context_nodes = graph_failure_context_nodes(player, asset_server, graphs);
    let active_node_candidate_count = node_candidates
        .iter()
        .filter(|candidate| candidate.active_context)
        .count();
    let candidate_resolution = if error.and_then(graph_error_node_id).is_none() {
        "not_applicable".to_owned()
    } else if active_node_candidate_count == 1 {
        "resolved_active_context".to_owned()
    } else if active_node_candidate_count > 1 {
        "ambiguous_active_context".to_owned()
    } else {
        "not_resolved_from_active_context".to_owned()
    };
    let root_graph_path = match player.get_animation_source() {
        AnimationSource::Graph(handle) => asset_server
            .get_path(handle.id())
            .map(|path| path.path().to_string_lossy().into_owned()),
        _ => None,
    };
    GraphEvaluationFailure {
        category: if error.is_some() { "graph_evaluation_error" } else { "root_pose_contract_error" }.to_owned(),
        raw_error: error.map(|error| format!("{error:?}")).unwrap_or_else(|| format!(
            "Root pose contract failed after graph evaluation: output_present={}, bone_count={}, active_channel_count={}",
            root_pose.output_present, root_pose.bone_count, root_pose.active_channel_count)),
        error_kind: error.map(graph_error_kind).unwrap_or_else(|| if root_pose.output_present { "EmptyRootPose" } else { "MissingRootPose" }.to_owned()),
        pin: error.and_then(graph_error_pin),
        node_candidates,
        active_context_nodes,
        active_node_candidate_count,
        candidate_resolution,
        root_graph_path,
        root_graph_recursive_dependencies: graph_dependency_state(asset_server, player),
        root_pose: root_pose_sample(root_pose),
        pose_classification: graph_error_pose_classification(root_pose),
        case: config.case.clone(),
        direction_case,
        phase,
        phase_elapsed_secs,
        locomotion,
        gait,
        stance,
        rotation_mode,
        screenshot_status: "not_attempted".to_owned(),
        screenshot_path: None,
        reported_before_ready,
    }
}

fn refresh_graph_failure_screenshot_status(
    state: &mut AnimationVisualValidationState,
    exiting: bool,
) {
    let Some(failure) = state.primary_failure.as_mut() else {
        return;
    };
    let Some(path) = failure.screenshot_path.as_deref() else {
        return;
    };
    if Path::new(path).is_file() {
        failure.screenshot_status = "captured".to_owned();
    } else if exiting && failure.screenshot_status == "capture_scheduled" {
        failure.screenshot_status = "capture_not_materialized_before_exit".to_owned();
    }
}

fn source_pose_motion_span(reference: &Pose, current: &Pose) -> (f32, f32) {
    let mut translation_span = 0.0_f32;
    let mut rotation_span = 0.0_f32;
    for (reference_bone, current_bone) in reference.bones.iter().zip(&current.bones) {
        if let (Some(reference), Some(current)) =
            (reference_bone.translation, current_bone.translation)
        {
            translation_span = translation_span.max(reference.distance(current));
        }
        if let (Some(reference), Some(current)) = (reference_bone.rotation, current_bone.rotation) {
            rotation_span = rotation_span.max(rotation_delta_radians(reference, current));
        }
    }
    (translation_span, rotation_span)
}

enum SourcePoseOutputOrigin {
    Cached,
    ActiveFsmState,
    LastFsmOutput,
}

fn source_pose_node(graph: &AnimationGraph, identity: &str) -> Option<NodeId> {
    graph
        .nodes
        .values()
        .find(|node| node.name == identity || node.id.uuid().to_string() == identity)
        .map(|node| node.id)
}

fn source_pose_boundary_output(
    player: &AnimationGraphPlayer,
    asset_server: &AssetServer,
    graphs: &Assets<AnimationGraph>,
    boundary: &ManualAnimationPoseBoundary,
) -> Option<(Pose, SourcePoseOutputOrigin)> {
    let arena = player.get_context_arena()?;
    for context_id in arena.iter_context_ids() {
        let context = arena.get_context(context_id)?;
        let Some(path) = asset_server.get_path(context.get_graph_id()) else {
            continue;
        };
        if path.path() != Path::new(boundary.graph) {
            continue;
        }
        let Some(graph) = graphs.get(context.get_graph_id()) else {
            continue;
        };
        let Some(node) = source_pose_node(graph, boundary.node).and_then(|id| graph.nodes.get(&id))
        else {
            continue;
        };
        if boundary.output == "Pose"
            && let Ok(mut states) = context
                .node_states
                .get_all_upcoming_states::<InterruptibleFsmState>(node.id)
            && let Some(pose) = states.find_map(|state| state.last_output_pose().cloned())
        {
            return Some((pose, SourcePoseOutputOrigin::ActiveFsmState));
        }
        if let Some(DataValue::Pose(pose)) = context.node_caches.get_output_data(
            node.id,
            StateKey::Default,
            boundary.output.to_owned(),
        ) {
            return Some((pose, SourcePoseOutputOrigin::Cached));
        }
        if boundary.output == "Pose"
            && let Ok(state) = context
                .node_states
                .get::<InterruptibleFsmState>(node.id, StateKey::Default)
            && let Some(pose) = state.last_output_pose()
        {
            return Some((pose.clone(), SourcePoseOutputOrigin::LastFsmOutput));
        }
    }
    None
}

fn source_pose_boundary_tracker(
    boundary: &ManualAnimationPoseBoundary,
) -> SourcePoseBoundaryTracker {
    SourcePoseBoundaryTracker {
        label: boundary.label.to_owned(),
        graph: boundary.graph.to_owned(),
        node: boundary.node.to_owned(),
        output: boundary.output.to_owned(),
        minimum_translation_span: boundary.minimum_translation_span.max(0.0),
        minimum_rotation_span_radians: boundary.minimum_rotation_span_radians.max(0.0),
        ..default()
    }
}

fn record_source_pose_bone_count(boundary: &mut SourcePoseBoundaryTracker, pose: &Pose) {
    let bone_count = pose.paths.len();
    boundary.minimum_bone_count = Some(
        boundary
            .minimum_bone_count
            .map_or(bone_count, |minimum| minimum.min(bone_count)),
    );
    boundary.maximum_bone_count = boundary.maximum_bone_count.max(bone_count);
}

fn record_source_pose_local_translation_extent(
    boundary: &mut SourcePoseBoundaryTracker,
    pose: &Pose,
    elapsed_seconds: f32,
) {
    let extent = pose
        .paths
        .values()
        .filter_map(|index| pose.bones.get(*index))
        .filter_map(|bone| bone.translation)
        .map(Vec3::length)
        .fold(0.0, f32::max);
    if boundary
        .minimum_max_local_translation_length
        .is_none_or(|minimum| extent < minimum)
    {
        boundary.minimum_max_local_translation_length = Some(extent);
        boundary.minimum_max_local_translation_elapsed_seconds = Some(elapsed_seconds);
    }
    boundary.maximum_max_local_translation_length =
        boundary.maximum_max_local_translation_length.max(extent);
    if boundary.trace.len() < 64 {
        boundary.trace.push(SourcePoseBoundaryTraceSample {
            elapsed_seconds,
            pose_time: pose.timestamp,
            bone_count: pose.paths.len(),
            maximum_local_translation_length: extent,
        });
    }
}

fn record_source_pose_chain(
    tracker: &mut MotionWindowTracker,
    evidence: &ManualAnimationWindowEvidence,
    player: &AnimationGraphPlayer,
    asset_server: &AssetServer,
    graphs: &Assets<AnimationGraph>,
    elapsed_seconds: f32,
) {
    if tracker.source_pose_chain.is_empty() {
        tracker.source_pose_chain = evidence
            .source_pose_chain
            .iter()
            .map(source_pose_boundary_tracker)
            .collect();
    }
    for (definition, boundary) in evidence
        .source_pose_chain
        .iter()
        .zip(&mut tracker.source_pose_chain)
    {
        boundary.sample_count += 1;
        let Some((pose, origin)) =
            source_pose_boundary_output(player, asset_server, graphs, definition)
        else {
            boundary.unavailable_samples += 1;
            continue;
        };
        boundary.available_samples += 1;
        match origin {
            SourcePoseOutputOrigin::Cached => boundary.cached_output_samples += 1,
            SourcePoseOutputOrigin::ActiveFsmState => boundary.active_fsm_state_samples += 1,
            SourcePoseOutputOrigin::LastFsmOutput => boundary.last_output_fallback_samples += 1,
        }
        record_source_pose_bone_count(boundary, &pose);
        record_source_pose_local_translation_extent(boundary, &pose, elapsed_seconds);
        if let Some(reference) = boundary.reference.as_ref() {
            let (translation_span, rotation_span) = source_pose_motion_span(reference, &pose);
            boundary.max_translation_span = boundary.max_translation_span.max(translation_span);
            boundary.max_rotation_span_radians =
                boundary.max_rotation_span_radians.max(rotation_span);
        } else {
            boundary.reference = Some(pose.clone());
        }
    }
}

fn first_source_pose_boundary_failure(boundaries: &[SourcePoseBoundaryTracker]) -> Option<String> {
    for boundary in boundaries {
        if boundary.sample_count < 2 || boundary.available_samples < 2 {
            return Some(format!("{}: unavailable", boundary.label));
        }
        if boundary.unavailable_samples > 0 {
            return Some(format!("{}: intermittently unavailable", boundary.label));
        }
        if boundary.max_translation_span <= f32::EPSILON
            && boundary.max_rotation_span_radians <= f32::EPSILON
        {
            return Some(format!("{}: static", boundary.label));
        }
        if (boundary.minimum_translation_span > f32::EPSILON
            || boundary.minimum_rotation_span_radians > f32::EPSILON)
            && (boundary.minimum_translation_span <= f32::EPSILON
                || boundary.max_translation_span < boundary.minimum_translation_span)
            && (boundary.minimum_rotation_span_radians <= f32::EPSILON
                || boundary.max_rotation_span_radians < boundary.minimum_rotation_span_radians)
        {
            return Some(format!("{}: below source floor", boundary.label));
        }
    }
    None
}

fn local_motion_pose_sample(
    bones: &BTreeMap<String, LocalBonePose>,
) -> BTreeMap<String, LocalMotionBoneSample> {
    MOTION_BONES
        .iter()
        .filter_map(|name| {
            let pose = bones.get(*name)?;
            Some((
                (*name).to_owned(),
                LocalMotionBoneSample {
                    translation: [
                        safe(pose.translation.x),
                        safe(pose.translation.y),
                        safe(pose.translation.z),
                    ],
                    rotation: [
                        safe(pose.rotation.x),
                        safe(pose.rotation.y),
                        safe(pose.rotation.z),
                        safe(pose.rotation.w),
                    ],
                },
            ))
        })
        .collect()
}

fn local_motion_span(
    reference: &BTreeMap<String, LocalBonePose>,
    current: &BTreeMap<String, LocalBonePose>,
) -> (f32, f32) {
    let mut translation_span = 0.0_f32;
    let mut rotation_span = 0.0_f32;
    for bone in MOTION_BONES {
        let Some(reference) = reference.get(*bone) else {
            continue;
        };
        let Some(value) = current.get(*bone) else {
            continue;
        };
        translation_span = translation_span.max(reference.translation.distance(value.translation));
        rotation_span =
            rotation_span.max(rotation_delta_radians(reference.rotation, value.rotation));
    }
    (translation_span, rotation_span)
}

fn wrapped_phase_distance(left: f32, right: f32) -> f32 {
    if !left.is_finite() || !right.is_finite() {
        return f32::INFINITY;
    }
    let difference = (left.rem_euclid(1.0) - right.rem_euclid(1.0)).abs();
    difference.min((1.0 - difference).abs())
}

fn wrapped_yaw_distance(left: f32, right: f32) -> f32 {
    if !left.is_finite() || !right.is_finite() {
        return f32::INFINITY;
    }
    wrapped_yaw_delta(left, right).abs()
}

fn wrapped_yaw_delta(reference: f32, current: f32) -> f32 {
    if !reference.is_finite() || !current.is_finite() {
        return f32::INFINITY;
    }
    (current - reference + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)
        - std::f32::consts::PI
}

fn record_actor_yaw_delta(tracker: &mut MotionWindowTracker, actor_yaw: f32) {
    tracker.actor_yaw_sample_count += 1;
    if let Some(reference) = tracker.actor_yaw_reference {
        tracker.actor_yaw_cumulative_delta += wrapped_yaw_delta(reference, actor_yaw);
        let delta = tracker.actor_yaw_cumulative_delta;
        tracker.max_actor_yaw_delta_radians = tracker.max_actor_yaw_delta_radians.max(delta.abs());
        if let Some(expected_sign) = tracker.expected_actor_yaw_delta_sign {
            let signed_delta = delta * f32::from(expected_sign);
            tracker.max_actor_yaw_signed_delta_radians = tracker
                .max_actor_yaw_signed_delta_radians
                .max(signed_delta.max(0.0));
            tracker.max_actor_yaw_opposite_delta_radians = tracker
                .max_actor_yaw_opposite_delta_radians
                .max((-signed_delta).max(0.0));
        }
    }
    tracker.actor_yaw_reference = Some(actor_yaw);
}

fn append_actor_yaw_trace(
    tracker: &mut MotionWindowTracker,
    evidence: &ManualAnimationWindowEvidence,
    actor_yaw: Option<f32>,
    view_yaw: Option<f32>,
    elapsed_seconds: f64,
) {
    if tracker.actor_yaw_trace.len() >= 512 {
        return;
    }
    tracker.actor_yaw_trace.push(ActorYawTraceSample {
        elapsed_secs: elapsed_seconds,
        actor_yaw_radians: actor_yaw.filter(|value| value.is_finite()),
        view_yaw_radians: view_yaw.filter(|value| value.is_finite()),
        animation_yaw_speed_degrees: evidence
            .observed_animation_yaw_speed_degrees
            .filter(|value| value.is_finite()),
        animation_curve_direct_yaw_speed_degrees: evidence
            .observed_animation_curve_direct_yaw_speed_degrees
            .filter(|value| value.is_finite()),
        animation_clip_time: evidence
            .observed_animation_clip_time
            .filter(|value| value.is_finite()),
        animation_weight: evidence
            .observed_animation_weight
            .filter(|value| value.is_finite()),
        animation_play_rate: evidence
            .observed_animation_play_rate
            .filter(|value| value.is_finite()),
        animation_curve_stack_weight: evidence
            .observed_animation_curve_stack_weight
            .filter(|value| value.is_finite()),
        animation_curve_stack_entries: evidence.observed_animation_curve_stack_entries.clone(),
        animation_variant: evidence.observed_animation_variant,
        requested_parent_runtime_state: evidence.observed_requested_parent_runtime_state,
        requested_parent_transition_active: evidence.observed_requested_parent_transition_active,
    });
}

fn record_motion_window_metadata(
    state: &mut AnimationVisualValidationState,
    evidence: &ManualAnimationWindowEvidence,
) -> Option<String> {
    let label = evidence.label?;
    let tracker = state.motion_windows.entry(label.to_owned()).or_default();
    if tracker.source_node.is_empty() {
        tracker.source_node = evidence.source_node.unwrap_or_default().to_owned();
    }
    if tracker.source_state.is_empty() {
        tracker.source_state = evidence.source_state.unwrap_or_default().to_owned();
    }
    tracker.actor_yaw_driver = evidence.actor_yaw_driver;
    if tracker.expected_actor_yaw_delta_sign.is_none() {
        tracker.expected_actor_yaw_delta_sign = evidence.expected_actor_yaw_delta_sign;
    }
    if tracker.expected_animation_variant.is_none() {
        tracker.expected_animation_variant = evidence.expected_animation_variant;
    }
    if tracker.expected_terminal_parent_runtime_state.is_none() {
        tracker.expected_terminal_parent_runtime_state =
            evidence.expected_terminal_parent_runtime_state;
    }
    if tracker.expected_terminal_parent_transition_active.is_none() {
        tracker.expected_terminal_parent_transition_active =
            evidence.expected_terminal_parent_transition_active;
    }
    if tracker
        .expected_terminal_requested_parent_transition_active
        .is_none()
    {
        tracker.expected_terminal_requested_parent_transition_active =
            evidence.expected_terminal_requested_parent_transition_active;
    }
    tracker.expected_lifecycle_stages.extend(
        evidence
            .expected_lifecycle_stages
            .iter()
            .map(|value| (*value).to_owned()),
    );
    if !evidence.capture_source_window {
        return None;
    }
    tracker.observed_lifecycle_stages.extend(
        evidence
            .observed_lifecycle_stages
            .iter()
            .map(|value| (*value).to_owned()),
    );
    Some(label.to_owned())
}

fn record_motion_window(
    state: &mut AnimationVisualValidationState,
    evidence: &ManualAnimationWindowEvidence,
    pre_rig: &AnimationVisualPreRigPose,
    final_pose: &BTreeMap<String, LocalBonePose>,
    actor_yaw: Option<f32>,
    view_yaw: Option<f32>,
    root_pose: RootPoseActivity,
    root_pose_value: Option<&Pose>,
    player: &AnimationGraphPlayer,
    asset_server: &AssetServer,
    graphs: &Assets<AnimationGraph>,
    elapsed_seconds: f64,
    game_elapsed_seconds: f64,
) -> bool {
    let sync_phase = evidence.sync_phase_global.and_then(|key| {
        if key.starts_with("sync_groups.") {
            player.get_context_arena().and_then(|arena| {
                bevy_als::extensions::sync_group_phase::source_sync_phase(arena, graphs, key)
            })
        } else {
            player
                .get_global_input_data(key)
                .and_then(|value| value.as_f32().ok())
        }
        .filter(|value| value.is_finite())
    });
    let Some(label) = record_motion_window_metadata(state, evidence) else {
        return false;
    };
    {
        let tracker = state.motion_windows.entry(label.clone()).or_default();
        if tracker.expected_actor_yaw_baseline.is_none() {
            tracker.expected_actor_yaw_baseline = evidence
                .expected_actor_yaw_baseline
                .filter(|value| value.is_finite());
        }
        if evidence.expected_actor_yaw_delta_sign.is_some() {
            if let Some(baseline) = tracker.expected_actor_yaw_baseline {
                tracker.actor_yaw_reference.get_or_insert(baseline);
                if let Some(view_yaw) = view_yaw.filter(|value| value.is_finite()) {
                    tracker
                        .actor_yaw_error_reference
                        .get_or_insert(wrapped_yaw_distance(baseline, view_yaw));
                }
                tracker
                    .first_sample_game_seconds
                    .get_or_insert(game_elapsed_seconds);
            }
        }
        if evidence.expected_actor_yaw_delta_sign.is_some()
            || evidence.observed_animation_clip_time.is_some()
        {
            append_actor_yaw_trace(tracker, evidence, actor_yaw, view_yaw, elapsed_seconds);
        }
    }
    if !evidence.settled
        || !(evidence.requires_pre_rig_motion
            || evidence.requires_final_pose_motion
            || evidence.requires_non_reference_root_pose
            || evidence.requires_source_pose_chain
            || evidence.requires_actor_yaw_motion
            || evidence.requires_actor_yaw_convergence
            || !evidence.expected_lifecycle_stages.is_empty())
    {
        return false;
    }
    let tracker = state.motion_windows.entry(label).or_default();
    if tracker.expected_direction.is_empty() {
        tracker.expected_direction = evidence.expected_direction.unwrap_or_default().to_owned();
    }
    if tracker.expected_runtime_state.is_none() {
        tracker.expected_runtime_state = evidence.expected_runtime_state;
    }
    if let Some(direction) = evidence.observed_direction {
        tracker.observed_directions.insert(direction.to_owned());
    }
    if let Some(runtime_state) = evidence.observed_runtime_state {
        tracker.observed_runtime_states.insert(runtime_state);
    }
    if tracker.expected_parent_runtime_state.is_none() {
        tracker.expected_parent_runtime_state = evidence.expected_parent_runtime_state;
    }
    if terminal_parent_state_sample(evidence) {
        if let Some(runtime_state) = evidence.observed_parent_runtime_state {
            tracker
                .observed_terminal_parent_runtime_states
                .insert(runtime_state);
        }
        if let Some(active) = evidence.observed_parent_transition_active {
            tracker
                .observed_terminal_parent_transition_active
                .insert(active);
        }
        if let Some(runtime_state) = evidence.observed_requested_parent_runtime_state {
            tracker
                .observed_terminal_requested_parent_runtime_states
                .insert(runtime_state);
        }
        if let Some(active) = evidence.observed_requested_parent_transition_active {
            tracker
                .observed_terminal_requested_parent_transition_active
                .insert(active);
        }
    } else {
        if let Some(runtime_state) = evidence.observed_parent_runtime_state {
            tracker.observed_parent_runtime_states.insert(runtime_state);
        }
        if let Some(active) = evidence.observed_parent_transition_active {
            tracker.observed_parent_transition_active.insert(active);
        }
        if let Some(runtime_state) = evidence.observed_requested_parent_runtime_state {
            tracker
                .observed_requested_parent_runtime_states
                .insert(runtime_state);
        }
        if let Some(active) = evidence.observed_requested_parent_transition_active {
            tracker
                .observed_requested_parent_transition_active
                .insert(active);
        }
    }
    tracker.requires_pre_rig_motion |= evidence.requires_pre_rig_motion;
    tracker.requires_final_pose_motion |= evidence.requires_final_pose_motion;
    tracker.requires_non_reference_root_pose |= evidence.requires_non_reference_root_pose;
    tracker.requires_source_pose_chain |= evidence.requires_source_pose_chain;
    tracker.requires_actor_yaw_motion |= evidence.requires_actor_yaw_motion;
    tracker.requires_actor_yaw_convergence |= evidence.requires_actor_yaw_convergence;
    tracker.minimum_pre_rig_translation_span = tracker
        .minimum_pre_rig_translation_span
        .max(evidence.minimum_pre_rig_translation_span.max(0.0));
    tracker.minimum_pre_rig_rotation_span_radians = tracker
        .minimum_pre_rig_rotation_span_radians
        .max(evidence.minimum_pre_rig_rotation_span_radians.max(0.0));
    tracker.minimum_final_translation_span = tracker
        .minimum_final_translation_span
        .max(evidence.minimum_final_translation_span.max(0.0));
    tracker.minimum_final_rotation_span_radians = tracker
        .minimum_final_rotation_span_radians
        .max(evidence.minimum_final_rotation_span_radians.max(0.0));
    tracker.minimum_actor_yaw_delta_radians = tracker
        .minimum_actor_yaw_delta_radians
        .max(evidence.minimum_actor_yaw_delta_radians.max(0.0));
    tracker.minimum_actor_yaw_convergence_radians = tracker
        .minimum_actor_yaw_convergence_radians
        .max(evidence.minimum_actor_yaw_convergence_radians.max(0.0));
    tracker.minimum_observation_seconds = tracker
        .minimum_observation_seconds
        .max(evidence.minimum_observation_seconds.max(0.0));
    if tracker.source_sync_group.is_empty() {
        tracker.source_sync_group = evidence.source_sync_group.unwrap_or_default().to_owned();
    }
    if tracker.sync_phase_global.is_empty() {
        tracker.sync_phase_global = evidence.sync_phase_global.unwrap_or_default().to_owned();
    }
    if let Some(sync_phase) = sync_phase {
        tracker.sync_phase_min = Some(
            tracker
                .sync_phase_min
                .map_or(sync_phase, |value| value.min(sync_phase)),
        );
        tracker.sync_phase_max = Some(
            tracker
                .sync_phase_max
                .map_or(sync_phase, |value| value.max(sync_phase)),
        );
    }
    tracker.canonical_sync_phase = evidence
        .canonical_sync_phase
        .filter(|value| value.is_finite());
    tracker.maximum_sync_phase_distance = evidence.maximum_sync_phase_distance.max(0.0);
    tracker.canonical_clip_time = evidence.canonical_clip_time;
    tracker.maximum_clip_time_distance = evidence.maximum_clip_time_distance;
    if tracker.direction_signature_group.is_empty() {
        tracker.direction_signature_group = evidence
            .direction_signature_group
            .unwrap_or_default()
            .to_owned();
    }
    tracker.required_direction_signature_members = evidence.required_direction_signature_members;
    tracker.minimum_direction_signature_rotation_radians = evidence
        .minimum_direction_signature_rotation_radians
        .max(0.0);
    let pre_rig_current = pre_rig.bones.clone();
    if tracker.pre_rig_reference.is_empty() {
        tracker.pre_rig_reference = pre_rig_current.clone();
    }
    if tracker.final_reference.is_empty() {
        tracker.final_reference = final_pose.clone();
    }
    let first_sample_game_seconds = tracker
        .first_sample_game_seconds
        .get_or_insert(game_elapsed_seconds);
    tracker.observation_seconds =
        (game_elapsed_seconds - *first_sample_game_seconds).max(0.0) as f32;
    let (pre_rig_translation_span, pre_rig_rotation_span) =
        local_motion_span(&tracker.pre_rig_reference, &pre_rig_current);
    tracker.max_pre_rig_translation_span = tracker
        .max_pre_rig_translation_span
        .max(pre_rig_translation_span);
    tracker.max_pre_rig_rotation_span_radians = tracker
        .max_pre_rig_rotation_span_radians
        .max(pre_rig_rotation_span);
    let (final_translation_span, final_rotation_span) =
        local_motion_span(&tracker.final_reference, final_pose);
    tracker.max_final_translation_span = tracker
        .max_final_translation_span
        .max(final_translation_span);
    tracker.max_final_rotation_span_radians = tracker
        .max_final_rotation_span_radians
        .max(final_rotation_span);
    if let Some(actor_yaw) = actor_yaw.filter(|value| value.is_finite()) {
        record_actor_yaw_delta(tracker, actor_yaw);
        if let Some(view_yaw) = view_yaw.filter(|value| value.is_finite()) {
            let error = wrapped_yaw_distance(actor_yaw, view_yaw);
            if let Some(reference) = tracker.actor_yaw_error_reference {
                tracker.max_actor_yaw_convergence_radians = tracker
                    .max_actor_yaw_convergence_radians
                    .max((reference - error).max(0.0));
            } else {
                tracker.actor_yaw_error_reference = Some(error);
            }
        }
    }
    tracker.root_pose_sample_count += 1;
    tracker.root_pose_max_active_channels = tracker
        .root_pose_max_active_channels
        .max(root_pose.active_channel_count);
    if root_pose.output_present && root_pose.active_channel_count > 0 {
        tracker.root_pose_active_samples += 1;
    } else {
        tracker.root_pose_reference_samples += 1;
    }
    if let Some(root_pose_value) = root_pose_value {
        if let Some(reference) = tracker.root_pose_reference.as_ref() {
            let (translation_span, rotation_span) =
                source_pose_motion_span(reference, root_pose_value);
            tracker.root_pose_max_translation_span =
                tracker.root_pose_max_translation_span.max(translation_span);
            tracker.root_pose_max_rotation_span_radians = tracker
                .root_pose_max_rotation_span_radians
                .max(rotation_span);
        } else {
            tracker.root_pose_reference = Some(root_pose_value.clone());
        }
    }
    let canonical_sample = if let Some(target_time) = tracker.canonical_clip_time {
        evidence.observed_animation_clip_time.map(|time| {
            (
                (time - target_time).abs(),
                tracker.maximum_clip_time_distance,
                &mut tracker.canonical_clip_time_distance,
            )
        })
    } else {
        tracker
            .canonical_sync_phase
            .zip(sync_phase)
            .map(|(target, phase)| {
                (
                    wrapped_phase_distance(phase, target),
                    tracker.maximum_sync_phase_distance,
                    &mut tracker.canonical_sync_phase_distance,
                )
            })
    };
    if let Some((distance, maximum_distance, selected_distance)) = canonical_sample
        && distance.is_finite()
        && distance <= maximum_distance
        && selected_distance.is_none_or(|current| distance < current)
    {
        *selected_distance = Some(distance);
        tracker.canonical_pre_rig_pose = pre_rig_current;
    }
    if evidence.requires_source_pose_chain {
        record_source_pose_chain(
            tracker,
            evidence,
            player,
            asset_server,
            graphs,
            elapsed_seconds as f32,
        );
    }
    tracker.sample_count += 1;
    tracker.requires_non_reference_root_pose
        && tracker.root_pose_reference_samples >= 2
        && tracker.root_pose_active_samples == 0
}

fn stats(values: &[f64]) -> TimeStats {
    if values.is_empty() {
        return TimeStats {
            samples: 0,
            p50_ms: 0.0,
            p95_ms: 0.0,
            max_ms: 0.0,
        };
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|left, right| left.total_cmp(right));
    let p50_index = (sorted.len() - 1) / 2;
    let p95_index = ((sorted.len() as f64 * 0.95).ceil() as usize)
        .saturating_sub(1)
        .min(sorted.len() - 1);
    TimeStats {
        samples: sorted.len(),
        p50_ms: sorted[p50_index],
        p95_ms: sorted[p95_index],
        max_ms: *sorted.last().unwrap_or(&0.0),
    }
}

fn count_stats(values: &[f64]) -> CountStats {
    let stats = stats(values);
    CountStats {
        samples: stats.samples,
        p50: stats.p50_ms,
        p95: stats.p95_ms,
        max: stats.max_ms,
    }
}

fn safe(value: f32) -> f32 {
    value.is_finite().then_some(value).unwrap_or(0.0)
}

fn scheduled_capture_allowed(_direction_settled: bool, teleport_detected: bool) -> bool {
    !teleport_detected
}

fn direction_capture_allowed(direction_settled: bool, teleport_detected: bool) -> bool {
    direction_settled && !teleport_detected
}

fn nonfault_capture_allowed(fault_detected: bool, capture_allowed: bool) -> bool {
    !fault_detected && capture_allowed
}

fn terminal_lifecycle_sample_required(evidence: &ManualAnimationWindowEvidence) -> bool {
    evidence.capture_source_window
        && evidence.observed_lifecycle_stages.iter().any(|stage| {
            matches!(
                *stage,
                "sequence_completed" | "natural_completion" | "inertialization" | "settled"
            )
        })
}

fn pose_sample_due(
    now: Instant,
    next: Instant,
    evidence: Option<&ManualAnimationWindowEvidence>,
) -> bool {
    now >= next
        || evidence.is_some_and(|evidence| {
            evidence.settled && evidence.capture_source_window && evidence.capture_every_frame
        })
}

fn terminal_parent_state_sample(evidence: &ManualAnimationWindowEvidence) -> bool {
    evidence.capture_source_window
        && evidence
            .observed_lifecycle_stages
            .iter()
            .any(|stage| *stage == "settled")
}

fn temporal_guard_should_rebase(
    previous_context: Option<&str>,
    context: &str,
    previous_window_settled: bool,
    window_settled: bool,
    previous_rebase_epoch: Option<u64>,
    rebase_epoch: u64,
) -> bool {
    previous_rebase_epoch != Some(rebase_epoch)
        || previous_context != Some(context)
        || (window_settled && !previous_window_settled)
}

fn sample_vec3(value: Vec3) -> [f32; 3] {
    [safe(value.x), safe(value.y), safe(value.z)]
}

fn ragdoll_root_space_sample(
    character_transform: &Transform,
    character_position: Option<&Position>,
    avatar_local: &Transform,
    avatar_world: &GlobalTransform,
    player_local: Option<&Transform>,
    player_world: Option<&GlobalTransform>,
    follow: Option<&AlsRagdollGroundFollow>,
) -> RagdollRootSpaceSample {
    RagdollRootSpaceSample {
        character_transform_translation: sample_vec3(character_transform.translation),
        character_position: character_position.map(|position| sample_vec3(position.0)),
        avatar_local_translation: sample_vec3(avatar_local.translation),
        avatar_world_translation: sample_vec3(avatar_world.translation()),
        player_local_translation: player_local.map(|transform| sample_vec3(transform.translation)),
        player_world_translation: player_world
            .map(|transform| sample_vec3(transform.translation())),
        follow_pelvis: follow.map(|follow| sample_vec3(follow.pelvis)),
        follow_target: follow.map(|follow| sample_vec3(follow.target)),
    }
}

fn foot_ik_side_sample(
    space: &AlsFootIkSpaceSide,
    apply: &AlsFootIkApplyTelemetrySide,
) -> FootIkSideSample {
    FootIkSideSample {
        valid: space.valid && apply.valid,
        has_ground_hit: space.has_ground_hit,
        trace_target_world: sample_vec3(space.trace_target_world),
        solver_reference_world: sample_vec3(space.solver_reference_world),
        target_world: sample_vec3(space.target_world),
        resolved_target_world: sample_vec3(space.resolved_target_world),
        pole_location_world: sample_vec3(space.pole_location_world),
        effector_world: sample_vec3(apply.effector_world),
        hip_world: sample_vec3(apply.hip_world),
        knee_world: sample_vec3(apply.knee_world),
        ankle_world: sample_vec3(apply.ankle_world),
        raw_offset: safe(space.raw_offset),
        applied_weight: safe(apply.applied_weight),
        pole_sign: safe(apply.pole_sign),
        knee_error: safe(apply.knee_error),
        effector_error: safe(apply.effector_error),
        solve_input: space.solve_input,
        previous_solve_input: space.previous_solve_input,
    }
}

fn rotation_rate_matches_source_ik(
    metric: &TemporalSkeletonMetric,
    previous: &bevy_als::foot_ik_apply::FootIkSolveInput,
    current: &bevy_als::foot_ik_apply::FootIkSolveInput,
    definition: &bevy_als::extensions::control_rig_program::RigDefinition,
    bones: &BTreeMap<String, FinalBone>,
) -> bool {
    if metric.kind != "local_rotation_spike" || metric.trigger != "rate" {
        return false;
    }
    let names = match metric.subject.as_str() {
        "calf_l" => ["thigh_l", "calf_l", "foot_l"],
        "calf_r" => ["thigh_r", "calf_r", "foot_r"],
        _ => return false,
    };
    let reference = |input: &bevy_als::foot_ik_apply::FootIkSolveInput| -> Option<([Quat; 2], Quat)> {
        let armature = Transform {
            translation: Vec3::from_array(input.armature_world_trs.0),
            rotation: Quat::from_array(input.armature_world_trs.1),
            scale: Vec3::from_array(input.armature_world_trs.2),
        };
        let inverse = armature.rotation.inverse();
        let point =
            |value| inverse * (Vec3::from_array(value) - armature.translation) / armature.scale;
        let rotation = |value| inverse * Quat::from_array(value);
        let hip = rotation(input.thigh_world_trs.1);
        let calf = rotation(input.calf_world_trs.1);
        let mut lengths = [input.upper_length, input.lower_length];
        for index in 0..2 {
            if lengths[index] > 0.0 {
                continue;
            }
            let a = definition
                .bones
                .iter()
                .find(|bone| bone.name == names[index])?
                .initial_global;
            let b = definition
                .bones
                .iter()
                .find(|bone| bone.name == names[index + 1])?
                .initial_global;
            let scale = Vec3::from_array(input.thigh_world_trs.2) / armature.scale;
            lengths[index] = bevy_als::extensions::ik::scaled_initial_bone_length(
                (a.translation - b.translation).as_vec3(),
                a.scale.as_vec3(),
                scale,
            );
        }
        let solved = bevy_als::extensions::ik::solve_basic_two_bone_ik(
            point(input.thigh_world_trs.0),
            hip,
            point(input.calf_world_trs.0),
            point(input.target_world),
            point(input.pole_world),
            lengths[0],
            lengths[1],
            Vec3::from_array(input.primary_axis),
            Vec3::from_array(input.secondary_axis),
            1.0,
        )?;
        let hip = hip.slerp(solved.hip_rotation_world, input.weight);
        let calf = calf.slerp(solved.knee_rotation_world, input.weight);
        Some((
            [armature.rotation * hip, armature.rotation * calf],
            hip.inverse() * calf,
        ))
    };
    let Some((_, previous_local)) = reference(previous) else {
        return false;
    };
    let Some((world, current_local)) = reference(current) else {
        return false;
    };
    if (rotation_delta_radians(previous_local, current_local) - metric.current).abs() > 0.0001 {
        return false;
    }
    names[..2].iter().zip(world).all(|(name, expected)| {
        bones.get(*name).is_some_and(|(_, actual, _, _)| {
            actual.is_finite()
                && expected.is_finite()
                && (actual
                    .to_array()
                    .into_iter()
                    .zip(expected.to_array())
                    .map(|(a, b)| (a - b).abs())
                    .fold(0.0_f32, f32::max)
                    .min(
                        actual
                            .to_array()
                            .into_iter()
                            .zip(expected.to_array())
                            .map(|(a, b)| (a + b).abs())
                            .fold(0.0_f32, f32::max),
                    )
                    < 0.00005)
        })
    })
}

fn foot_ik_sample(
    spaces: &AlsFootIkSpaceState,
    apply: &AlsFootIkApplyTelemetry,
    input: &AlsControlRigInput,
    teleport: Option<&AlsFootLockTeleportTelemetry>,
    context: Option<&AlsRootAnimationContext>,
    standing_pose_moving_global: f32,
    pose_moving_lineage: Vec<ContextCurveSample>,
    source_velocity_blend: [f32; 4],
    source_movement_direction: [bool; 4],
    source_hips_direction_lock_amount: f32,
    source_feet_crossing_amount: f32,
) -> FootIkSample {
    let teleport = teleport.copied().unwrap_or_default();
    let pose_moving = context.and_then(|value| value.f32("PoseMoving"));
    let foot_left_ik_curve = context.and_then(|value| value.f32("FootLeftIk"));
    let foot_right_ik_curve = context.and_then(|value| value.f32("FootRightIk"));
    let foot_left_lock_curve = context.and_then(|value| value.f32("FootLeftLock"));
    let foot_right_lock_curve = context.and_then(|value| value.f32("FootRightLock"));
    FootIkSample {
        left: foot_ik_side_sample(&spaces.left, &apply.left),
        right: foot_ik_side_sample(&spaces.right, &apply.right),
        rig_inputs: None,
        source_curves_available: pose_moving.is_some()
            && foot_left_ik_curve.is_some()
            && foot_right_ik_curve.is_some()
            && foot_left_lock_curve.is_some()
            && foot_right_lock_curve.is_some(),
        pose_moving: safe(pose_moving.unwrap_or_default()),
        foot_left_ik_curve: safe(foot_left_ik_curve.unwrap_or_default()),
        foot_right_ik_curve: safe(foot_right_ik_curve.unwrap_or_default()),
        foot_left_lock_curve: safe(foot_left_lock_curve.unwrap_or_default()),
        foot_right_lock_curve: safe(foot_right_lock_curve.unwrap_or_default()),
        standing_pose_moving_global: safe(standing_pose_moving_global),
        pose_moving_lineage,
        source_velocity_blend: source_velocity_blend.map(safe),
        source_movement_direction,
        source_hips_direction_lock_amount: safe(source_hips_direction_lock_amount),
        source_feet_crossing_amount: safe(source_feet_crossing_amount),
        velocity_blend_forward_amount: safe(input.velocity_blend_forward_amount),
        velocity_blend_backward_amount: safe(input.velocity_blend_backward_amount),
        teleport: FootLockTeleportSample {
            detected_count: teleport.detected_count,
            left_rebase_count: teleport.left_rebase_count,
            right_rebase_count: teleport.right_rebase_count,
            last_translation_delta_m: safe(teleport.last_translation_delta_m),
            remaining_seconds: safe(teleport.remaining_seconds),
        },
    }
}

fn rig_inputs_sample(
    definition: &bevy_als::extensions::control_rig_program::RigDefinition,
    state: &bevy_als::extensions::root_graph_boundaries::ControlRigNodeState,
) -> serde_json::Value {
    use bevy_als::extensions::control_rig_program::RigInstruction;
    let calls: Vec<_> = state
        .calls
        .iter()
        .filter_map(|call| {
            let RigInstruction::Call { function, .. } =
                definition.program.instructions.get(call.instruction)?
            else {
                return None;
            };
            if !matches!(
                function.as_str(),
                "FAlsRigUnit_ApplyFootOffsetLocation::Execute"
                    | "FRigUnit_TwoBoneIKSimplePerItem::Execute"
                    | "FAlsRigUnit_ApplyFootOffsetRotation::Execute"
                    | "FRigUnit_SetRotation::Execute"
            ) {
                return None;
            }
            Some(
                serde_json::json!({"instruction": call.instruction, "function": function,
            "arguments": call.arguments, "bones_before": call.bones_before}),
            )
        })
        .collect();
    serde_json::json!({"elapsed_seconds": state.elapsed_seconds, "initialization_count": state.initialization_count, "calls": calls})
}

fn pose_moving_lineage(
    player: &AnimationGraphPlayer,
    graph: &AnimationGraph,
) -> Vec<ContextCurveSample> {
    [
        "AB_Als_Standing",
        "AB_Als_Grounded",
        "Slot 'Transition'",
        "AB_Als_Locomotion",
        "Slot 'PostLocomotion'",
        "ALI_Overlay - Overlay",
        "AB_Als_Layering",
        "AB_Als_Head",
        "ControlRig CR_Als",
        "AB_Als_Ragdolling",
    ]
    .into_iter()
    .map(|node_name| {
        let context = graph
            .nodes
            .values()
            .find(|node| node.name == node_name)
            .and_then(|node| player_animation_context_state(player, node.id))
            .map(|state| &state.curves);
        let curve = |name| {
            context
                .and_then(|curves| curves.get(&bevy_als::extensions::SourceCurveName::from(name)))
                .copied()
                .filter(|value| value.is_finite())
        };
        let pose_moving = curve("PoseMoving");
        ContextCurveSample {
            node: node_name.to_owned(),
            pose_moving,
            foot_left_ik: curve("FootLeftIk"),
            foot_right_ik: curve("FootRightIk"),
            foot_left_lock: curve("FootLeftLock"),
            foot_right_lock: curve("FootRightLock"),
        }
    })
    .collect()
}

fn segment_name(first: &str, second: &str) -> String {
    format!("{first}->{second}")
}

fn bone_samples(bones: &BTreeMap<String, FinalBone>) -> BTreeMap<String, BoneSample> {
    bones
        .iter()
        .map(|(name, (translation, rotation, scale, local_scale))| {
            (
                name.clone(),
                BoneSample {
                    translation: translation.to_array().map(safe),
                    rotation: rotation.to_array().map(safe),
                    scale: scale.to_array().map(safe),
                    local_scale: local_scale.to_array().map(safe),
                },
            )
        })
        .collect()
}

fn snapshot(
    elapsed_secs: f64,
    frame_ms: f64,
    bones: &BTreeMap<String, (Vec3, Quat, Vec3, Vec3)>,
    direction_case: Option<&str>,
    phase: Option<&str>,
    phase_elapsed_secs: Option<f32>,
    locomotion: Option<&str>,
    gait: Option<&str>,
    stance: Option<&str>,
    rotation_mode: Option<&str>,
    foot_ik: Option<&FootIkSample>,
    ragdoll_root_space: Option<&RagdollRootSpaceSample>,
    root_pose: RootPoseActivity,
) -> PoseSample {
    let bones = bone_samples(bones);
    let segments = SEGMENTS
        .iter()
        .filter_map(|(first, second)| {
            let first_position = bones.get(*first)?;
            let second_position = bones.get(*second)?;
            let first_translation = Vec3::from_array(first_position.translation);
            let second_translation = Vec3::from_array(second_position.translation);
            Some((
                segment_name(first, second),
                first_translation.distance(second_translation),
            ))
        })
        .collect();
    PoseSample {
        elapsed_secs,
        frame_ms,
        direction_case: direction_case.map(str::to_owned),
        phase: phase.map(str::to_owned),
        phase_elapsed_secs,
        locomotion: locomotion.map(str::to_owned),
        gait: gait.map(str::to_owned),
        stance: stance.map(str::to_owned),
        rotation_mode: rotation_mode.map(str::to_owned),
        bones,
        pre_rig_bones: BTreeMap::new(),
        segments,
        foot_ik: foot_ik.cloned(),
        ragdoll_root_space: ragdoll_root_space.cloned(),
        root_pose: root_pose_sample(root_pose),
    }
}

fn write_report(
    config: &AnimationVisualValidationConfig,
    state: &AnimationVisualValidationState,
    stages: &AnimationVisualStageTiming,
    manual_integration: Option<&ManualIntegrationGateReport>,
) {
    let observed = state
        .pose_samples
        .last()
        .map(|sample| sample.bones.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    let missing_bones = REQUIRED_BONES
        .iter()
        .filter(|name| !observed.iter().any(|observed_name| observed_name == **name))
        .map(|name| (*name).to_owned())
        .collect::<Vec<_>>();
    let frame_time = stats(&state.frame_ms);
    let cpu_frame_time = stats(&state.cpu_frame_ms);
    let animation_eval_time = stats(&state.eval_ms);
    let physics_steps_per_render = count_stats(&stages.physics_steps_per_render);
    let stage_time = stages
        .samples
        .iter()
        .map(|(name, values)| (name.clone(), stats(values)))
        .collect();
    let ready = state.pose_samples.len() >= config.min_pose_samples
        && state.screenshots.len() >= config.capture_count
        && frame_time.samples >= config.min_pose_samples
        && animation_eval_time.samples >= config.min_pose_samples
        && physics_steps_per_render.samples >= config.min_pose_samples;
    let pass = ready
        && missing_bones.is_empty()
        && state.faults.is_empty()
        && state.graph_errors.is_empty()
        && state.primary_failure.is_none()
        && frame_time.p95_ms <= config.max_frame_p95_ms
        && frame_time.max_ms <= config.max_frame_ms
        && animation_eval_time.p95_ms <= config.max_eval_p95_ms
        && physics_steps_per_render.max <= config.max_physics_steps_per_render;
    let thresholds = BTreeMap::from([
        ("max_frame_p95_ms".to_owned(), config.max_frame_p95_ms),
        ("max_frame_ms".to_owned(), config.max_frame_ms),
        ("max_eval_p95_ms".to_owned(), config.max_eval_p95_ms),
        (
            "max_physics_steps_per_render".to_owned(),
            config.max_physics_steps_per_render,
        ),
        (
            "temporal_global_position_speed_mps".to_owned(),
            TEMPORAL_GLOBAL_POSITION_SPEED_METERS_PER_SECOND as f64,
        ),
        (
            "temporal_local_translation_jump_m".to_owned(),
            TEMPORAL_LOCAL_TRANSLATION_JUMP_METERS as f64,
        ),
        (
            "temporal_local_rotation_jump_radians".to_owned(),
            TEMPORAL_LOCAL_ROTATION_JUMP_RADIANS as f64,
        ),
        (
            "temporal_local_rotation_speed_radps".to_owned(),
            TEMPORAL_LOCAL_ROTATION_SPEED_RADIANS_PER_SECOND as f64,
        ),
        (
            "temporal_large_angular_change_rate_radps".to_owned(),
            TEMPORAL_LARGE_ANGULAR_CHANGE_RATE_RADIANS_PER_SECOND as f64,
        ),
        (
            "temporal_scale_ratio".to_owned(),
            TEMPORAL_SCALE_RATIO as f64,
        ),
        (
            "temporal_segment_ratio".to_owned(),
            TEMPORAL_SEGMENT_RATIO as f64,
        ),
        (
            "temporal_limb_plane_flip_dot".to_owned(),
            TEMPORAL_LIMB_PLANE_FLIP_DOT as f64,
        ),
    ]);
    let report = AnimationVisualReport {
        format: animation_visual_evidence_format(),
        case: config.case.clone(),
        ready,
        pass,
        observed_bones: state.observed_bones,
        required_bones: REQUIRED_BONES
            .iter()
            .map(|name| (*name).to_owned())
            .collect(),
        missing_bones,
        frame_time,
        loading_frame_time: state.playable_frame.map(|_| stats(&state.loading_frame_ms)),
        cpu_frame_time,
        animation_eval_time,
        physics_steps_per_render,
        stage_time,
        excluded_frame_samples: state.excluded_frame_samples,
        pose_samples: state.pose_samples.clone(),
        screenshots: state.screenshots.clone(),
        motion_windows: state
            .motion_windows
            .iter()
            .map(|(label, tracker)| MotionWindowSample {
                label: label.clone(),
                source_node: tracker.source_node.clone(),
                source_state: tracker.source_state.clone(),
                expected_direction: tracker.expected_direction.clone(),
                observed_directions: tracker.observed_directions.iter().cloned().collect(),
                expected_runtime_state: tracker.expected_runtime_state,
                observed_runtime_states: tracker.observed_runtime_states.iter().copied().collect(),
                expected_parent_runtime_state: tracker.expected_parent_runtime_state,
                expected_terminal_parent_runtime_state: tracker
                    .expected_terminal_parent_runtime_state,
                expected_terminal_parent_transition_active: tracker
                    .expected_terminal_parent_transition_active,
                expected_terminal_requested_parent_transition_active: tracker
                    .expected_terminal_requested_parent_transition_active,
                observed_parent_runtime_states: tracker
                    .observed_parent_runtime_states
                    .iter()
                    .copied()
                    .collect(),
                observed_parent_transition_active: tracker
                    .observed_parent_transition_active
                    .iter()
                    .copied()
                    .collect(),
                observed_requested_parent_runtime_states: tracker
                    .observed_requested_parent_runtime_states
                    .iter()
                    .copied()
                    .collect(),
                observed_requested_parent_transition_active: tracker
                    .observed_requested_parent_transition_active
                    .iter()
                    .copied()
                    .collect(),
                observed_terminal_parent_runtime_states: tracker
                    .observed_terminal_parent_runtime_states
                    .iter()
                    .copied()
                    .collect(),
                observed_terminal_parent_transition_active: tracker
                    .observed_terminal_parent_transition_active
                    .iter()
                    .copied()
                    .collect(),
                observed_terminal_requested_parent_runtime_states: tracker
                    .observed_terminal_requested_parent_runtime_states
                    .iter()
                    .copied()
                    .collect(),
                observed_terminal_requested_parent_transition_active: tracker
                    .observed_terminal_requested_parent_transition_active
                    .iter()
                    .copied()
                    .collect(),
                sample_count: tracker.sample_count,
                requires_pre_rig_motion: tracker.requires_pre_rig_motion,
                requires_final_pose_motion: tracker.requires_final_pose_motion,
                requires_non_reference_root_pose: tracker.requires_non_reference_root_pose,
                requires_source_pose_chain: tracker.requires_source_pose_chain,
                requires_actor_yaw_motion: tracker.requires_actor_yaw_motion,
                requires_actor_yaw_convergence: tracker.requires_actor_yaw_convergence,
                actor_yaw_driver: tracker.actor_yaw_driver,
                expected_actor_yaw_delta_sign: tracker.expected_actor_yaw_delta_sign,
                expected_actor_yaw_baseline: tracker.expected_actor_yaw_baseline,
                expected_animation_variant: tracker.expected_animation_variant,
                minimum_pre_rig_translation_span: tracker.minimum_pre_rig_translation_span,
                minimum_pre_rig_rotation_span_radians: tracker
                    .minimum_pre_rig_rotation_span_radians,
                minimum_final_translation_span: tracker.minimum_final_translation_span,
                minimum_final_rotation_span_radians: tracker.minimum_final_rotation_span_radians,
                minimum_actor_yaw_delta_radians: tracker.minimum_actor_yaw_delta_radians,
                minimum_actor_yaw_convergence_radians: tracker
                    .minimum_actor_yaw_convergence_radians,
                minimum_observation_seconds: tracker.minimum_observation_seconds,
                observation_seconds: tracker.observation_seconds,
                max_pre_rig_translation_span: tracker.max_pre_rig_translation_span,
                max_pre_rig_rotation_span_radians: tracker.max_pre_rig_rotation_span_radians,
                max_final_translation_span: tracker.max_final_translation_span,
                max_final_rotation_span_radians: tracker.max_final_rotation_span_radians,
                root_pose_sample_count: tracker.root_pose_sample_count,
                root_pose_active_samples: tracker.root_pose_active_samples,
                root_pose_reference_samples: tracker.root_pose_reference_samples,
                root_pose_max_active_channels: tracker.root_pose_max_active_channels,
                root_pose_max_translation_span: tracker.root_pose_max_translation_span,
                root_pose_max_rotation_span_radians: tracker.root_pose_max_rotation_span_radians,
                actor_yaw_sample_count: tracker.actor_yaw_sample_count,
                actor_yaw_trace: tracker.actor_yaw_trace.clone(),
                max_actor_yaw_delta_radians: tracker.max_actor_yaw_delta_radians,
                max_actor_yaw_signed_delta_radians: tracker.max_actor_yaw_signed_delta_radians,
                max_actor_yaw_opposite_delta_radians: tracker.max_actor_yaw_opposite_delta_radians,
                max_actor_yaw_convergence_radians: tracker.max_actor_yaw_convergence_radians,
                source_sync_group: tracker.source_sync_group.clone(),
                sync_phase_global: tracker.sync_phase_global.clone(),
                sync_phase_min: tracker.sync_phase_min,
                sync_phase_max: tracker.sync_phase_max,
                canonical_sync_phase: tracker.canonical_sync_phase,
                maximum_sync_phase_distance: tracker.maximum_sync_phase_distance,
                canonical_sync_phase_distance: tracker.canonical_sync_phase_distance,
                canonical_clip_time: tracker.canonical_clip_time,
                maximum_clip_time_distance: tracker.maximum_clip_time_distance,
                canonical_clip_time_distance: tracker.canonical_clip_time_distance,
                direction_signature_group: tracker.direction_signature_group.clone(),
                required_direction_signature_members: tracker.required_direction_signature_members,
                minimum_direction_signature_rotation_radians: tracker
                    .minimum_direction_signature_rotation_radians,
                canonical_pre_rig_pose: local_motion_pose_sample(&tracker.canonical_pre_rig_pose),
                source_pose_chain: tracker
                    .source_pose_chain
                    .iter()
                    .map(|boundary| SourcePoseBoundarySample {
                        label: boundary.label.clone(),
                        graph: boundary.graph.clone(),
                        node: boundary.node.clone(),
                        output: boundary.output.clone(),
                        minimum_translation_span: boundary.minimum_translation_span,
                        minimum_rotation_span_radians: boundary.minimum_rotation_span_radians,
                        sample_count: boundary.sample_count,
                        available_samples: boundary.available_samples,
                        unavailable_samples: boundary.unavailable_samples,
                        cached_output_samples: boundary.cached_output_samples,
                        active_fsm_state_samples: boundary.active_fsm_state_samples,
                        last_output_fallback_samples: boundary.last_output_fallback_samples,
                        minimum_bone_count: boundary.minimum_bone_count,
                        maximum_bone_count: boundary.maximum_bone_count,
                        minimum_max_local_translation_length: boundary
                            .minimum_max_local_translation_length,
                        minimum_max_local_translation_elapsed_seconds: boundary
                            .minimum_max_local_translation_elapsed_seconds,
                        maximum_max_local_translation_length: boundary
                            .maximum_max_local_translation_length,
                        trace: boundary.trace.clone(),
                        max_translation_span: boundary.max_translation_span,
                        max_rotation_span_radians: boundary.max_rotation_span_radians,
                    })
                    .collect(),
                expected_lifecycle_stages: tracker
                    .expected_lifecycle_stages
                    .iter()
                    .cloned()
                    .collect(),
                observed_lifecycle_stages: tracker
                    .observed_lifecycle_stages
                    .iter()
                    .cloned()
                    .collect(),
                first_failed_source_pose_boundary: tracker
                    .requires_source_pose_chain
                    .then(|| first_source_pose_boundary_failure(&tracker.source_pose_chain))
                    .flatten(),
            })
            .collect(),
        required_motion_windows: state.required_motion_windows.iter().cloned().collect(),
        temporal_anomalies: state.temporal_anomalies.clone(),
        faults: state.faults.clone(),
        graph_errors: state.graph_errors.clone(),
        primary_failure: state.primary_failure.clone(),
        manual_integration,
        automation_watchdog: AutomationWatchdogReport {
            triggered: state.watchdog.triggered,
            reason: if state.watchdog.triggered {
                "capture_progress_stalled".to_owned()
            } else {
                String::new()
            },
            stalled_seconds: state.watchdog.stalled_seconds,
            capture_count: state.screenshots.len(),
            required_capture_count: config.capture_count,
        },
        thresholds,
    };
    let path = config.directory.join("report.json");
    if let Ok(serialized) = serde_json::to_vec_pretty(&report) {
        let _ = fs::write(path, serialized);
    }
}

#[allow(clippy::too_many_arguments)]
fn record_graph_evaluation_failure(
    commands: &mut Commands,
    config: &AnimationVisualValidationConfig,
    state: &mut AnimationVisualValidationState,
    stages: &AnimationVisualStageTiming,
    manual_integration: Option<&ManualIntegrationGateReport>,
    error: Option<&GraphError>,
    player: &AnimationGraphPlayer,
    asset_server: &AssetServer,
    graphs: &Assets<AnimationGraph>,
    root_pose: RootPoseActivity,
    direction_case: Option<&str>,
    phase: Option<&str>,
    phase_elapsed_secs: Option<f32>,
    locomotion: Option<&str>,
    gait: Option<&str>,
    stance: Option<&str>,
    rotation_mode: Option<&str>,
    foot_ik: Option<&FootIkSample>,
    bones: &BTreeMap<String, FinalBone>,
    has_camera: bool,
) {
    let elapsed_secs = state
        .started
        .map(|started| started.elapsed().as_secs_f64())
        .unwrap_or_default();
    let mut failure = graph_evaluation_failure(
        config,
        error,
        player,
        asset_server,
        graphs,
        root_pose,
        direction_case.map(str::to_owned),
        phase.map(str::to_owned),
        phase_elapsed_secs,
        locomotion.map(str::to_owned),
        gait.map(str::to_owned),
        stance.map(str::to_owned),
        rotation_mode.map(str::to_owned),
        state.started.is_none(),
    );
    if config.capture_count == 0 {
        failure.screenshot_status = "disabled_by_configuration".to_owned();
    } else if has_camera {
        let path = config.directory.join("graph_error.png");
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path.clone()));
        let path = path.to_string_lossy().into_owned();
        state.screenshots.push(ScreenshotSample {
            path: path.clone(),
            elapsed_secs,
            kind: "graph_error".to_owned(),
            direction_case: direction_case.map(str::to_owned),
            phase: phase.map(str::to_owned),
            phase_elapsed_secs,
            locomotion: locomotion.map(str::to_owned),
            gait: gait.map(str::to_owned),
            stance: stance.map(str::to_owned),
            rotation_mode: rotation_mode.map(str::to_owned),
            faults: vec![format!("graph_evaluation_error:{}", failure.error_kind)],
        });
        failure.screenshot_status = "capture_scheduled".to_owned();
        failure.screenshot_path = Some(path);
    } else {
        failure.screenshot_status = "unavailable_no_camera".to_owned();
    }
    append_unique(
        &mut state.graph_errors,
        format!(
            "{}; root_graph_recursive_dependencies={}",
            failure.raw_error, failure.root_graph_recursive_dependencies
        ),
    );
    state.observed_bones = state.observed_bones.max(bones.len());
    let mut sample = snapshot(
        elapsed_secs,
        0.0,
        bones,
        direction_case,
        phase,
        phase_elapsed_secs,
        locomotion,
        gait,
        stance,
        rotation_mode,
        foot_ik,
        None,
        root_pose,
    );
    sample.segments = segment_lengths(bones);
    state.pose_samples.push(sample);
    state.primary_failure = Some(failure);
    state.fatal_exit_frames_remaining = Some(2);
    write_report(config, state, stages, manual_integration);
}

fn finish_fatal_animation_visual_failure(
    config: Res<AnimationVisualValidationConfig>,
    mut state: ResMut<AnimationVisualValidationState>,
    stages: Res<AnimationVisualStageTiming>,
    manual_integration: Option<Res<ManualIntegrationGateReport>>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(remaining) = state.fatal_exit_frames_remaining else {
        return;
    };
    let exiting = remaining <= 1;
    refresh_graph_failure_screenshot_status(&mut state, exiting);
    write_report(&config, &state, &stages, manual_integration.as_deref());
    if exiting {
        state.fatal_exit_frames_remaining = None;
        exit.write(AppExit::Error(NonZeroU8::new(92).unwrap()));
    } else {
        state.fatal_exit_frames_remaining = Some(remaining - 1);
    }
}

fn flush_animation_visual_report(
    config: Res<AnimationVisualValidationConfig>,
    state: Res<AnimationVisualValidationState>,
    stages: Res<AnimationVisualStageTiming>,
    manual_integration: Option<Res<ManualIntegrationGateReport>>,
    mut exit: MessageReader<AppExit>,
) {
    if !exit.is_empty() {
        exit.clear();
        write_report(&config, &state, &stages, manual_integration.as_deref());
    }
}

fn collect_animation_visual_evidence(
    mut commands: Commands,
    config: Res<AnimationVisualValidationConfig>,
    mut state: ResMut<AnimationVisualValidationState>,
    mut stages: ResMut<AnimationVisualStageTiming>,
    animation_window: Option<Res<ManualAnimationWindowEvidence>>,
    phase_evidence: Option<Res<ManualRuntimePhaseEvidence>>,
    manual_integration: Option<Res<ManualIntegrationGateReport>>,
    pose_state: (
        Res<AnimationVisualPreRigPose>,
        Res<Time<Virtual>>,
        Query<(&BodyLabel, &RigidBody, &Position, &Rotation)>,
        Query<&RagdollPhysicsProfileApplied>,
        Query<&AlsRagdollInitialSpeedLimit, With<AlsCharacter>>,
    ),
    asset_server: Res<AssetServer>,
    graphs: Res<Assets<AnimationGraph>>,
    avatars: Query<
        (Entity, &AnimatedSceneInstance, &GlobalTransform, &Transform),
        With<AlsCharacterAvatar>,
    >,
    players: Query<(
        &AnimationGraphPlayer,
        Option<&AlsRootAnimationContext>,
        Option<&GlobalTransform>,
        Option<&Transform>,
    )>,
    foot_ik: Query<
        (
            &AlsFootIkSpaceState,
            &AlsFootIkApplyTelemetry,
            &AlsControlRigInput,
            Option<&AlsFootLockTeleportTelemetry>,
            &Transform,
            Option<&Position>,
            Option<&AlsRagdollGroundFollow>,
        ),
        With<AlsCharacter>,
    >,
    children: Query<&Children>,
    named_transforms: Query<(&Name, &GlobalTransform, &Transform)>,
    cameras: (
        Query<Entity, With<Camera3d>>,
        Query<(
            &bevy_als::camera_component::AlsSourceCameraComponent,
            &AnimatedSceneInstance,
        )>,
        Query<&bevy_als::extensions::overlay_materials::AvatarMaterialApplied>,
        Query<&bevy_als::overlay_object::AlsOverlayObject>,
        Query<&AnimatedSceneInstance>,
    ),
) {
    let (pre_rig, game_time, ragdoll_bodies, ragdoll_profiles, ragdoll_limit) = pose_state;
    if state.primary_failure.is_some() {
        return;
    }
    if cameras.2.iter().any(|applied| !applied.source_slot_pattern) {
        append_unique(
            &mut state.faults,
            "mesh_material_source_slot_pattern_mismatch".to_owned(),
        );
    }
    let direction_case = animation_window
        .as_deref()
        .and_then(|evidence| evidence.label)
        .map(str::to_owned);
    let phase = phase_evidence
        .as_deref()
        .and_then(|evidence| (!evidence.phase.is_empty()).then_some(evidence.phase.as_str()));
    let phase_elapsed_secs = phase_evidence
        .as_deref()
        .map(|evidence| evidence.elapsed_seconds)
        .filter(|value| value.is_finite() && *value >= 0.0);
    let locomotion = phase_evidence.as_deref().and_then(|evidence| {
        (!evidence.locomotion.is_empty()).then_some(evidence.locomotion.as_str())
    });
    let gait = phase_evidence
        .as_deref()
        .and_then(|evidence| (!evidence.gait.is_empty()).then_some(evidence.gait.as_str()));
    let stance = phase_evidence
        .as_deref()
        .and_then(|evidence| (!evidence.stance.is_empty()).then_some(evidence.stance.as_str()));
    let rotation_mode = phase_evidence.as_deref().and_then(|evidence| {
        (!evidence.rotation_mode.is_empty()).then_some(evidence.rotation_mode.as_str())
    });
    let body_has_error = avatars
        .single()
        .ok()
        .and_then(|(_, instance, _, _)| players.get(instance.player_entity()).ok())
        .is_some_and(|(player, _, _, _)| player.get_error().is_some());
    if !body_has_error {
        let secondary_players = cameras
            .1
            .iter()
            .filter(|(component, _)| component.is_active())
            .map(|(_, instance)| instance.player_entity())
            .chain(cameras.3.iter().filter_map(|object| {
                cameras
                    .4
                    .get(object.content)
                    .ok()
                    .map(AnimatedSceneInstance::player_entity)
            }));
        for entity in secondary_players {
            let Ok((player, _, _, _)) = players.get(entity) else {
                continue;
            };
            let error = player.get_error();
            let root_pose = root_pose_activity(player);
            let evaluated = player
                .get_context_arena()
                .is_some_and(|arena| arena.frame_index() > 0);
            if error.is_none() && (!evaluated || root_pose_has_animation_channels(root_pose)) {
                continue;
            }
            record_graph_evaluation_failure(
                &mut commands,
                &config,
                &mut state,
                &stages,
                manual_integration.as_deref(),
                error.as_ref(),
                player,
                &asset_server,
                &graphs,
                root_pose,
                direction_case.as_deref(),
                phase,
                phase_elapsed_secs,
                locomotion,
                gait,
                stance,
                rotation_mode,
                None,
                &BTreeMap::new(),
                cameras.0.iter().next().is_some(),
            );
            return;
        }
    }
    let Ok((_avatar, instance, avatar_transform, avatar_local)) = avatars.single() else {
        return;
    };
    let Ok((player, context, player_world, player_local)) = players.get(instance.player_entity())
    else {
        return;
    };
    let graph_asset = match player.get_animation_source() {
        AnimationSource::Graph(handle) => graphs.get(handle),
        _ => None,
    };
    let graph_ready = graph_asset.is_some();
    let graph_error = player.get_error();
    let root_pose = root_pose_activity(player);
    let root_pose_value = player.get_default_output_pose();
    let direction_settled = animation_window
        .as_deref()
        .is_some_and(|evidence| evidence.settled);
    if let Some(evidence) = animation_window.as_deref()
        && let Some(label) = evidence.label
        && (evidence.requires_pre_rig_motion
            || evidence.requires_final_pose_motion
            || evidence.requires_non_reference_root_pose
            || evidence.requires_source_pose_chain
            || evidence.requires_actor_yaw_motion
            || !evidence.expected_lifecycle_stages.is_empty())
    {
        state.required_motion_windows.insert(label.to_owned());
    }
    let temporal_rebase_epoch = phase_evidence
        .as_deref()
        .map(|evidence| evidence.temporal_rebase_epoch)
        .unwrap_or_default();
    let temporal_window_requires = animation_window.as_deref().is_some_and(|evidence| {
        evidence.requires_pre_rig_motion
            || evidence.requires_final_pose_motion
            || evidence.requires_non_reference_root_pose
            || evidence.requires_source_pose_chain
            || evidence.requires_actor_yaw_motion
            || !evidence.expected_lifecycle_stages.is_empty()
    });
    let temporal_observable = !temporal_window_requires || direction_settled;
    let temporal_context = format!(
        "{}|{}|{}",
        phase.unwrap_or_default(),
        direction_case.as_deref().unwrap_or_default(),
        stance.unwrap_or_default(),
    );
    let temporal_rebase = temporal_guard_should_rebase(
        state.temporal_context.as_deref(),
        &temporal_context,
        state.temporal_window_settled,
        direction_settled,
        state.temporal_rebase_epoch,
        temporal_rebase_epoch,
    );
    if temporal_rebase {
        state.temporal_pre_rig_guard.reset();
        state.temporal_final_guard.reset();
    }
    state.temporal_context = Some(temporal_context);
    state.temporal_window_settled = direction_settled;
    state.temporal_rebase_epoch = Some(temporal_rebase_epoch);
    let standing_pose_moving_global = player
        .get_global_input_data("als_standing_curve_pose_moving")
        .and_then(|value| value.as_f32().ok())
        .unwrap_or_default();
    let lineage = graph_asset
        .map(|graph| pose_moving_lineage(player, graph))
        .unwrap_or_default();
    let source_velocity_blend = [
        "forward_amount",
        "backward_amount",
        "left_amount",
        "right_amount",
    ]
    .map(|direction| {
        bevy_als::extensions::source_instance::source_player_property(
            player,
            &format!("grounded_state.velocity_blend.{direction}"),
        )
        .and_then(|value| value.as_f32().ok())
        .unwrap_or_default()
    });
    let source_movement_direction =
        ["b_forward", "b_backward", "b_left", "b_right"].map(|direction| {
            player
                .get_global_input_data(format!("grounded_state.movement_direction.{direction}"))
                .and_then(|value| value.as_bool().ok())
                .unwrap_or(false)
        });
    let source_hips_direction_lock_amount = player
        .get_global_input_data("grounded_state.hips_direction_lock_amount")
        .and_then(|value| value.as_f32().ok())
        .unwrap_or_default();
    let source_feet_crossing_amount = player
        .get_global_input_data("feet_state.feet_crossing_amount")
        .and_then(|value| value.as_f32().ok())
        .unwrap_or_default();
    let foot_ik_data = foot_ik.single().ok();
    let ragdoll_root_space = foot_ik_data.as_ref().map(
        |(_, _, _, _, character_transform, character_position, follow)| {
            ragdoll_root_space_sample(
                character_transform,
                *character_position,
                avatar_local,
                avatar_transform,
                player_local,
                player_world,
                *follow,
            )
        },
    );
    let mut foot_ik_sample = foot_ik_data.map(|(spaces, apply, input, teleport, _, _, _)| {
        foot_ik_sample(
            spaces,
            apply,
            input,
            teleport,
            context,
            standing_pose_moving_global,
            lineage,
            source_velocity_blend,
            source_movement_direction,
            source_hips_direction_lock_amount,
            source_feet_crossing_amount,
        )
    });
    if let Some(sample) = foot_ik_sample.as_mut()
        && let Some((_, node, state)) = bevy_als::extensions::root_graph_boundaries::current_control_rig(player, &graphs)
        && let Some(definition) = node.try_inner_downcast_ref::<bevy_als::extensions::root_graph_boundaries::ControlRigBoundaryNode>()
            .and_then(|node| node.definition.as_ref())
    {
        sample.rig_inputs = Some(rig_inputs_sample(definition, state));
    }
    if let Some(error) = graph_error.as_ref() {
        let bones = collect_bones(instance.player_entity(), &children, &named_transforms);
        record_graph_evaluation_failure(
            &mut commands,
            &config,
            &mut state,
            &stages,
            manual_integration.as_deref(),
            Some(error),
            player,
            &asset_server,
            &graphs,
            root_pose,
            direction_case.as_deref(),
            phase,
            phase_elapsed_secs,
            locomotion,
            gait,
            stance,
            rotation_mode,
            foot_ik_sample.as_ref(),
            &bones,
            cameras.0.iter().next().is_some(),
        );
        return;
    }
    if state.started.is_none() {
        if !graph_ready {
            return;
        }
        let now = Instant::now();
        state.started = Some(now);
        state.last_frame = Some(now);
        state.next_pose_sample = Some(now);
        state.next_capture = Some(now + config.capture_interval);
        stages.physics_steps_since_render = 0;
        stages.physics_steps_per_render.clear();
    }
    let now = Instant::now();
    let started = state.started.unwrap_or(now);
    let elapsed = started.elapsed();
    let terminal_lifecycle_sample = animation_window
        .as_deref()
        .is_some_and(terminal_lifecycle_sample_required);
    if let Some(evidence) = animation_window.as_deref() {
        let _ = record_motion_window_metadata(&mut state, evidence);
    }
    let frame_ms = state
        .last_frame
        .replace(now)
        .map(|previous| now.duration_since(previous).as_secs_f64() * 1000.0)
        .unwrap_or(0.0);
    let cpu_frame_ms = state
        .frame_started
        .map(|started| now.duration_since(started).as_secs_f64() * 1000.0)
        .unwrap_or(0.0);
    let performance_ready = state.performance_ready(config.warmup);
    if frame_ms > 0.0 {
        if state.playable_frame == Some(false) {
            state.loading_frame_ms.push(frame_ms);
        } else if performance_ready {
            if state.skip_frame_samples > 0 {
                state.skip_frame_samples -= 1;
                state.excluded_frame_samples += 1;
            } else {
                state.frame_ms.push(frame_ms);
                if cpu_frame_ms > 0.0 {
                    state.cpu_frame_ms.push(cpu_frame_ms);
                }
            }
        }
    }
    if performance_ready {
        let physics_steps_since_render = stages.physics_steps_since_render as f64;
        stages
            .physics_steps_per_render
            .push(physics_steps_since_render);
    }
    let physics_advanced = stages.physics_steps_since_render > 0;
    stages.physics_steps_since_render = 0;
    let mut fault_detected = false;
    if elapsed >= config.warmup && !root_pose_has_animation_channels(root_pose) {
        fault_detected |= append_unique(
            &mut state.faults,
            format!(
                "startup_root_pose:{}",
                graph_error_pose_classification(root_pose)
            ),
        );
    }
    let teleport_detected = foot_ik_sample.as_ref().is_some_and(|sample| {
        let detected = sample.teleport.detected_count > state.last_teleport_count;
        state.last_teleport_count = state
            .last_teleport_count
            .max(sample.teleport.detected_count);
        detected
    });
    if !graph_ready {
        fault_detected |= append_unique(&mut state.faults, "animation_source_not_graph".to_owned());
    }
    let bones = collect_bones(instance.player_entity(), &children, &named_transforms);
    let final_pose =
        collect_local_motion_bones(instance.player_entity(), &children, &named_transforms);
    state.observed_bones = state.observed_bones.max(bones.len());
    if elapsed >= Duration::from_secs(2) {
        for required in REQUIRED_BONES {
            if !bones.contains_key(*required) {
                fault_detected |=
                    append_unique(&mut state.faults, format!("missing_bone:{required}"));
            }
        }
    }
    let avatar_position = avatar_transform.translation();
    let mesh_position = player_world.map_or(avatar_position, GlobalTransform::translation);
    let frame_seconds = pre_rig.temporal_delta_seconds();
    let segments = segment_lengths(&bones);
    let (pre_rig_metrics, mut final_metrics) = if temporal_observable {
        let pre_rig = state.temporal_pre_rig_guard.observe(
            &pre_rig.bones,
            &pre_rig.global_bones,
            pre_rig.root_position,
            frame_seconds,
        );
        let final_pose =
            state
                .temporal_final_guard
                .observe(&final_pose, &bones, mesh_position, frame_seconds);
        (pre_rig.metrics, final_pose.metrics)
    } else {
        (Vec::new(), Vec::new())
    };
    if let Some(sample) = &foot_ik_sample
        && let Some((_, node, _)) = bevy_als::extensions::root_graph_boundaries::current_control_rig(player, &graphs)
        && let Some(definition) = node.try_inner_downcast_ref::<bevy_als::extensions::root_graph_boundaries::ControlRigBoundaryNode>()
            .and_then(|node| node.definition.as_ref())
    {
        final_metrics.retain(|metric| {
            let side = match metric.subject.as_str() { "calf_l" => &sample.left, "calf_r" => &sample.right, _ => return true };
            let (Some(previous), Some(current)) = (&side.previous_solve_input, &side.solve_input) else { return true; };
            let source_motion = side.valid && rotation_rate_matches_source_ik(metric, previous, current, definition, &bones);
            if source_motion { debug!("[ANIMATION-VISUAL][SOURCE-MOTION] subject={} rotation_delta={:.6}", metric.subject, metric.current); }
            !source_motion
        });
    }
    if elapsed >= config.warmup && temporal_observable {
        fault_detected |= record_temporal_metrics(
            &mut state,
            pre_rig_metrics,
            if pre_rig.source_selector_inputs.is_some() {
                "pre_rig_source_landing_selector"
            } else {
                "pre_rig"
            },
            None,
            elapsed.as_secs_f64(),
            frame_ms,
            frame_seconds,
            direction_case.as_deref(),
            phase,
        );
        if pre_rig.source_selector_inputs.is_some() {
            for metric in &mut state.temporal_anomalies {
                if metric.source_boundary == Some("source_fall_heavy_to_light_on_physics_entry")
                    && metric.source_selector_inputs.is_none()
                {
                    metric.source_selector_inputs = pre_rig.source_selector_inputs;
                }
            }
        }
        let scaled_readback = if physics_advanced
            && pre_rig.physics_readback_active
            && !final_metrics.is_empty()
            && let Ok(limit) = ragdoll_limit.single()
            && limit.remaining_updates > 0
            && limit.remaining_updates
                < bevy_als::get_up::RAGDOLL_SETTINGS.initial_speed_limit_refresh_frames
            && let Ok(profile) = ragdoll_profiles.get(instance.player_entity())
            && profile.scale.cmpgt(profile.joint_scale).all()
            && let Some(spawned) = player.get_spawned_ragdoll()
            && profile.spawned_root == Some(spawned.root)
        {
            let bodies = spawned
                .bodies
                .values()
                .filter_map(|entity| ragdoll_bodies.get(*entity).ok())
                .filter_map(|(label, body, position, rotation)| {
                    let (published_position, published_rotation, _, _) = bones.get(&label.0)?;
                    let position_error = published_position.distance(position.0);
                    let published = Vec4::from_array(published_rotation.to_array());
                    let physical = Vec4::from_array(rotation.0.to_array());
                    let rotation_error = (published - physical)
                        .abs()
                        .max_element()
                        .min((published + physical).abs().max_element());
                    (body.is_dynamic() && position_error <= 1e-4 && rotation_error <= 1e-4)
                        .then(|| (label.0.clone(), [position_error, rotation_error]))
                })
                .collect();
            Some(ScaledRagdollReadback {
                mesh_scale: profile.scale.to_array(),
                joint_scale: profile.joint_scale.to_array(),
                remaining_updates: limit.remaining_updates,
                bodies,
            })
        } else {
            None
        };
        fault_detected |= record_temporal_metrics(
            &mut state,
            final_metrics,
            "final",
            scaled_readback.as_ref(),
            elapsed.as_secs_f64(),
            frame_ms,
            frame_seconds,
            direction_case.as_deref(),
            phase,
        );
    }
    for (name, (translation, rotation, scale, local_scale)) in &bones {
        let scale_abs = scale.abs();
        let relative = *translation - avatar_position;
        if !translation.is_finite()
            || !rotation.is_finite()
            || !scale.is_finite()
            || rotation.length_squared() < 0.5
        {
            fault_detected |=
                append_unique(&mut state.faults, format!("non_finite_transform:{name}"));
        }
        if scale_abs.min_element() < 0.01 || scale_abs.max_element() > 10.0 {
            fault_detected |=
                append_unique(&mut state.faults, format!("scale_out_of_range:{name}"));
        }
        if !local_scale.is_finite()
            || local_scale.abs().min_element() < 0.01
            || local_scale.abs().max_element() > 10.0
        {
            fault_detected |= append_unique(
                &mut state.faults,
                format!("local_scale_out_of_range:{name}"),
            );
        }
        if relative.length() > 8.0 {
            fault_detected |= append_unique(
                &mut state.faults,
                format!("bone_radius_out_of_range:{name}"),
            );
        }
    }
    for (name, length) in &segments {
        if !length.is_finite() || !(0.015..=3.0).contains(length) {
            fault_detected |=
                append_unique(&mut state.faults, format!("segment_out_of_range:{name}"));
        }
    }
    let mut sampled = false;
    if let Some(next_pose_sample) = state.next_pose_sample {
        if pose_sample_due(now, next_pose_sample, animation_window.as_deref())
            || fault_detected
            || teleport_detected
            || terminal_lifecycle_sample
        {
            let mut sample = snapshot(
                elapsed.as_secs_f64(),
                frame_ms,
                &bones,
                direction_case.as_deref(),
                phase,
                phase_elapsed_secs,
                locomotion,
                gait,
                stance,
                rotation_mode,
                foot_ik_sample.as_ref(),
                ragdoll_root_space.as_ref(),
                root_pose,
            );
            sample.segments = segments.clone();
            sample.pre_rig_bones = bone_samples(&pre_rig.global_bones);
            state.pose_samples.push(sample);
            if let Some(evidence) = animation_window.as_deref() {
                if record_motion_window(
                    &mut state,
                    evidence,
                    &pre_rig,
                    &final_pose,
                    evidence.observed_actor_yaw,
                    evidence.observed_view_yaw,
                    root_pose,
                    root_pose_value,
                    player,
                    &asset_server,
                    &graphs,
                    elapsed.as_secs_f64(),
                    game_time.elapsed_secs_f64(),
                ) {
                    fault_detected |= append_unique(
                        &mut state.faults,
                        format!("reference_root_pose:{}", evidence.label.unwrap_or_default()),
                    );
                }
            }
            state.next_pose_sample = Some(now + Duration::from_millis(100));
            sampled = true;
        }
    }
    if (fault_detected || teleport_detected) && !sampled {
        let mut sample = snapshot(
            elapsed.as_secs_f64(),
            frame_ms,
            &bones,
            direction_case.as_deref(),
            phase,
            phase_elapsed_secs,
            locomotion,
            gait,
            stance,
            rotation_mode,
            foot_ik_sample.as_ref(),
            ragdoll_root_space.as_ref(),
            root_pose,
        );
        sample.segments = segments.clone();
        state.pose_samples.push(sample);
        if let Some(evidence) = animation_window.as_deref() {
            if record_motion_window(
                &mut state,
                evidence,
                &pre_rig,
                &final_pose,
                evidence.observed_actor_yaw,
                evidence.observed_view_yaw,
                root_pose,
                root_pose_value,
                player,
                &asset_server,
                &graphs,
                elapsed.as_secs_f64(),
                game_time.elapsed_secs_f64(),
            ) {
                fault_detected |= append_unique(
                    &mut state.faults,
                    format!("reference_root_pose:{}", evidence.label.unwrap_or_default()),
                );
            }
        }
    }
    if config.capture_count > 0
        && nonfault_capture_allowed(
            fault_detected,
            scheduled_capture_allowed(direction_settled, teleport_detected),
        )
        && state.capture_index < config.capture_count
        && state
            .next_capture
            .is_some_and(|next_capture| now >= next_capture)
        && let Some(camera) = cameras.0.iter().next()
    {
        let path = config
            .directory
            .join(format!("frame_{:02}.png", state.capture_index));
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path.clone()));
        state.screenshots.push(ScreenshotSample {
            path: path.to_string_lossy().into_owned(),
            elapsed_secs: elapsed.as_secs_f64(),
            kind: "scheduled".to_owned(),
            direction_case: direction_case.clone(),
            phase: phase.map(str::to_owned),
            phase_elapsed_secs,
            locomotion: locomotion.map(str::to_owned),
            gait: gait.map(str::to_owned),
            stance: stance.map(str::to_owned),
            rotation_mode: rotation_mode.map(str::to_owned),
            faults: Vec::new(),
        });
        state.capture_index += 1;
        state.next_capture = Some(now + config.capture_interval);
        state.skip_frame_samples = state.skip_frame_samples.max(3);
        let _ = camera;
    }
    if config.capture_count > 0
        && nonfault_capture_allowed(
            fault_detected,
            direction_capture_allowed(direction_settled, teleport_detected),
        )
        && let Some(direction_case) = direction_case.as_ref()
        && state
            .captured_direction_cases
            .insert(direction_case.clone())
        && let Some(camera) = cameras.0.iter().next()
    {
        let path = config
            .directory
            .join(format!("direction_{direction_case}.png"));
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path.clone()));
        state.screenshots.push(ScreenshotSample {
            path: path.to_string_lossy().into_owned(),
            elapsed_secs: elapsed.as_secs_f64(),
            kind: "direction".to_owned(),
            direction_case: Some(direction_case.clone()),
            phase: phase.map(str::to_owned),
            phase_elapsed_secs,
            locomotion: locomotion.map(str::to_owned),
            gait: gait.map(str::to_owned),
            stance: stance.map(str::to_owned),
            rotation_mode: rotation_mode.map(str::to_owned),
            faults: Vec::new(),
        });
        state.skip_frame_samples = state.skip_frame_samples.max(3);
        let _ = camera;
    }
    if config.capture_count > 0
        && nonfault_capture_allowed(fault_detected, teleport_detected)
        && state.teleport_captures < 4
        && let Some(camera) = cameras.0.iter().next()
    {
        let path = config
            .directory
            .join(format!("teleport_{:02}.png", state.teleport_captures));
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path.clone()));
        state.screenshots.push(ScreenshotSample {
            path: path.to_string_lossy().into_owned(),
            elapsed_secs: elapsed.as_secs_f64(),
            kind: "teleport".to_owned(),
            direction_case: direction_case.clone(),
            phase: phase.map(str::to_owned),
            phase_elapsed_secs,
            locomotion: locomotion.map(str::to_owned),
            gait: gait.map(str::to_owned),
            stance: stance.map(str::to_owned),
            rotation_mode: rotation_mode.map(str::to_owned),
            faults: Vec::new(),
        });
        state.teleport_captures += 1;
        state.skip_frame_samples = state.skip_frame_samples.max(3);
        let _ = camera;
    }
    if config.capture_count > 0 && fault_detected && state.fault_captures < 4 {
        let path = config
            .directory
            .join(format!("fault_{:02}.png", state.fault_captures));
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path.clone()));
        let faults = state.faults.clone();
        state.screenshots.push(ScreenshotSample {
            path: path.to_string_lossy().into_owned(),
            elapsed_secs: elapsed.as_secs_f64(),
            kind: "fault".to_owned(),
            direction_case,
            phase: phase.map(str::to_owned),
            phase_elapsed_secs,
            locomotion: locomotion.map(str::to_owned),
            gait: gait.map(str::to_owned),
            stance: stance.map(str::to_owned),
            rotation_mode: rotation_mode.map(str::to_owned),
            faults,
        });
        state.fault_captures += 1;
        state.skip_frame_samples = state.skip_frame_samples.max(3);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pre_rig_global_motion_includes_current_ancestor_rotations() {
        use bevy_animation_graph::core::animation_clip::EntityPath;
        let root_path = EntityPath::from(vec!["pelvis".to_owned()]);
        let spine_path = EntityPath::from(vec!["pelvis".to_owned(), "spine_01".to_owned()]);
        let hand_path = EntityPath::from(vec![
            "pelvis".to_owned(),
            "spine_01".to_owned(),
            "hand_l".to_owned(),
        ]);
        let mut skeleton = Skeleton::default();
        skeleton.add_bone(root_path.clone(), Transform::IDENTITY, Transform::IDENTITY);
        skeleton.add_bone(
            spine_path.clone(),
            Transform::from_translation(Vec3::X),
            Transform::from_translation(Vec3::X),
        );
        skeleton.add_bone(
            hand_path.clone(),
            Transform::from_translation(Vec3::X),
            Transform::from_translation(Vec3::X * 2.0),
        );
        skeleton.set_root(root_path.id());
        let world = GlobalTransform::from(
            Transform::from_xyz(8.0, 2.0, -4.0)
                .with_rotation(Quat::from_rotation_y(0.3))
                .with_scale(Vec3::splat(1.2)),
        );
        let mut pose = Pose::default();
        let before = pose_global_bones(&pose, &skeleton, &world);
        assert!(
            before["hand_l"]
                .0
                .abs_diff_eq(world.transform_point(Vec3::X * 2.0), 1e-5)
        );
        let rotation = Quat::from_rotation_z(0.7);
        pose.add_bone(
            BonePose {
                rotation: Some(rotation),
                ..default()
            },
            root_path.id(),
        );
        pose.add_bone(
            BonePose {
                rotation: Some(rotation),
                ..default()
            },
            spine_path.id(),
        );
        let after = pose_global_bones(&pose, &skeleton, &world);
        let expected = rotation * Vec3::X + rotation * rotation * Vec3::X;
        assert!(
            after["hand_l"]
                .0
                .abs_diff_eq(world.transform_point(expected), 1e-5)
        );
        let mut guard = TemporalSkeletonGuard::default();
        guard.observe(&BTreeMap::new(), &before, world.translation(), 0.04);
        let metrics = guard
            .observe(&BTreeMap::new(), &after, world.translation(), 0.04)
            .metrics;
        assert!(
            metrics
                .iter()
                .any(|metric| metric.kind == "global_position_speed" && metric.subject == "hand_l")
        );
        let sample = bone_samples(&after);
        assert_eq!(sample["hand_l"].translation, after["hand_l"].0.to_array());
    }

    #[test]
    fn uphill_landing_source_cadence_keeps_the_pre_rig_leg_plane_continuous() {
        use bevy_als::als::{Gait, LocomotionState, RotationMode};
        use bevy::ecs::system::RunSystemOnce;
        let (mut app, character, _) = crate::test_support::loaded_character();
        crate::playground_collision::PlaygroundCollisionPlugin.build(&mut app);
        let model = bevy_gltf::gltf::binary::Glb::from_slice(include_bytes!(
            "../../assets/ALSExtras/Environment/Meshes/SM_Als_Stairs_01.glb"
        ))
        .unwrap();
        let data: serde_json::Value = serde_json::from_slice(&model.json).unwrap();
        let bytes = model.bin.unwrap();
        let primitive = &data["meshes"][0]["primitives"][0];
        let accessor_bytes = |index: usize, width: usize| {
            let accessor = &data["accessors"][index];
            let view = &data["bufferViews"][accessor["bufferView"].as_u64().unwrap() as usize];
            let offset = view["byteOffset"].as_u64().unwrap_or(0) as usize
                + accessor["byteOffset"].as_u64().unwrap_or(0) as usize;
            &bytes[offset..offset + accessor["count"].as_u64().unwrap() as usize * width]
        };
        let positions = accessor_bytes(
            primitive["attributes"]["POSITION"].as_u64().unwrap() as usize,
            12,
        )
        .chunks_exact(12)
        .map(|point| {
            std::array::from_fn::<_, 3, _>(|i| {
                f32::from_le_bytes(point[i * 4..i * 4 + 4].try_into().unwrap())
            })
        })
        .collect::<Vec<_>>();
        let indices = accessor_bytes(primitive["indices"].as_u64().unwrap() as usize, 2)
            .chunks_exact(2)
            .map(|index| u16::from_le_bytes(index.try_into().unwrap()) as u32)
            .collect();
        let mut mesh = Mesh::new(
            bevy::render::render_resource::PrimitiveTopology::TriangleList,
            default(),
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
        mesh.insert_indices(bevy::render::mesh::Indices::U32(indices));
        let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(mesh);
        let source = Transform::from_xyz(-3.3333, 0.18, -30.0);
        let stairs = app
            .world_mut()
            .spawn((
                crate::playground::PlaygroundCollisionSource::new("SM_Als_Stairs_01"),
                source,
                GlobalTransform::from(source),
            ))
            .id();
        app.world_mut().spawn((
            ChildOf(stairs),
            Mesh3d(mesh),
            Transform::IDENTITY,
            GlobalTransform::from(source),
        ));
        let start = Vec3::new(0.001699686, 0.924, -29.0);
        app.world_mut().entity_mut(character).insert((
            avian3d::prelude::Position(start),
            Transform::from_translation(start),
            avian3d::prelude::LinearVelocity::ZERO,
        ));
        for _ in 0..30 {
            app.update();
        }
        assert!(
            app.world()
                .entity(stairs)
                .contains::<crate::playground_collision::PlaygroundMeshColliderBuilt>()
        );
        app.init_resource::<AnimationVisualPreRigPose>();
        app.world_mut()
            .get_mut::<bevy_als::als::gait::AlsDesiredGait>(character)
            .unwrap()
            .0 = Gait::Sprinting;
        app.world_mut().entity_mut(character).insert((
            RotationMode::VelocityDirection,
            bevy_als::als::aim::AlsLastFreeRotationMode(RotationMode::VelocityDirection),
        ));
        app.world_mut()
            .get_mut::<crate::input::AlsInputCommand>(character)
            .unwrap()
            .move_dir = Vec2::Y;
        for _ in 0..60 {
            if start.z
                - app
                    .world()
                    .get::<avian3d::prelude::Position>(character)
                    .unwrap()
                    .z
                >= 0.85
            {
                break;
            }
            app.update();
        }
        let mut guard = TemporalSkeletonGuard::default();
        let cadence = [
            0.1136718_f32,
            0.0810643,
            0.0513584,
            0.054793,
            0.0254989,
            0.055838,
            0.048991,
            0.0456682,
        ];
        let mut airborne = false;
        let mut landed = false;
        let mut failed = Vec::new();
        for frame in 0..80 {
            let delta = cadence[frame % cadence.len()];
            app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_secs_f32(delta),
            ));
            app.world_mut()
                .get_mut::<crate::input::AlsInputCommand>(character)
                .unwrap()
                .jump_pressed = frame == 0;
            app.update();
            app.world_mut()
                .run_system_once(capture_animation_visual_pre_rig_pose)
                .unwrap();
            let mode = *app.world().get::<LocomotionState>(character).unwrap();
            airborne |= mode == LocomotionState::Air;
            landed |= airborne && mode == LocomotionState::Grounded;
            let capture = app.world().resource::<AnimationVisualPreRigPose>();
            if !capture.global_bones.is_empty() {
                let observed = guard.observe(
                    &capture.bones,
                    &capture.global_bones,
                    capture.root_position,
                    capture.temporal_delta_seconds(),
                );
                failed.extend(
                    observed
                        .metrics
                        .into_iter()
                        .filter(|metric| metric.kind == "limb_plane_flip"),
                );
            }
        }
        assert!(airborne && landed);
        assert!(
            failed.is_empty(),
            "Source landing pre-rig pose must preserve leg-plane continuity under the observed cadence: {failed:?}"
        );
    }

    #[test]
    fn high_landing_pre_rig_hands_keep_the_source_stage_space() {
        use bevy::ecs::system::RunSystemOnce;
        let (mut app, character, _) = crate::test_support::loaded_character();
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f32(0.0122101),
        ));
        app.init_resource::<AnimationVisualPreRigPose>();
        let position = Vec3::new(-15.0, 10.824, -15.0);
        let transform = Transform::from_translation(position)
            .with_scale(Vec3::splat(2.0))
            .with_rotation(Quat::from_rotation_y(-std::f32::consts::FRAC_PI_4));
        app.world_mut().entity_mut(character).insert((
            avian3d::prelude::Position(position),
            transform,
            GlobalTransform::from(transform),
            avian3d::prelude::LinearVelocity::ZERO,
        ));
        let mut guard = TemporalSkeletonGuard::default();
        let mut state = AnimationVisualValidationState::default();
        let mut saw_ragdoll = false;
        for _ in 0..130 {
            app.update();
            app.world_mut()
                .run_system_once(capture_animation_visual_pre_rig_pose)
                .unwrap();
            let capture = app.world().resource::<AnimationVisualPreRigPose>();
            if !capture.global_bones.is_empty() {
                let observed = guard.observe(
                    &capture.bones,
                    &capture.global_bones,
                    capture.root_position,
                    capture.temporal_delta_seconds(),
                );
                let failed = record_temporal_metrics(
                    &mut state,
                    observed.metrics,
                    if capture.source_selector_inputs.is_some() {
                        "pre_rig_source_landing_selector"
                    } else {
                        "pre_rig"
                    },
                    None,
                    0.0,
                    12.2101,
                    capture.temporal_delta_seconds(),
                    None,
                    None,
                );
                assert!(
                    !failed,
                    "Actual high landing has an unclassified temporal fault: {:?}",
                    state.faults
                );
            }
            saw_ragdoll |= app
                .world()
                .get::<bevy_als::animation_events::AlsNotifyLocomotionAction>(character)
                == Some(&bevy_als::animation_events::AlsNotifyLocomotionAction::Ragdolling);
            if saw_ragdoll {
                break;
            }
        }
        assert!(saw_ragdoll);
        assert_eq!(state.temporal_anomalies.len(), 2);
        assert!(
            state
                .temporal_anomalies
                .iter()
                .all(|metric| metric.source_boundary.is_some()
                    && metric.current > metric.threshold
                    && metric.threshold == 40.0)
        );
        assert!(state.faults.is_empty());
    }

    #[test]
    fn source_landing_selector_classification_rejects_wrong_input_stage_and_physics() {
        let previous = Some([-13.295677, 0.0]);
        let current = Some([0.0, 1.0]);
        assert!(source_landing_selector_boundary(
            previous, current, false, true
        ));
        for (previous, current, was_active, active) in [
            (previous, current, false, false),
            (previous, current, true, true),
            (None, current, false, true),
            (previous, None, false, true),
            (Some([-9.999, 0.0]), current, false, true),
            (previous, Some([-5.001, 1.0]), false, true),
            (Some([-13.295677, 0.2]), current, false, true),
            (previous, Some([0.0, 0.8]), false, true),
            (Some([f32::NAN, 0.0]), current, false, true),
        ] {
            assert!(!source_landing_selector_boundary(
                previous, current, was_active, active
            ));
        }
        let metric = |kind, subject: &str| TemporalSkeletonMetric {
            kind,
            subject: subject.into(),
            previous: 0.0,
            current: 46.0,
            threshold: 40.0,
            trigger: "rate",
            rate: Some(46.0),
            rate_threshold: Some(40.0),
        };
        for (stage, kind, subject, expected_failure) in [
            (
                "pre_rig_source_landing_selector",
                "global_position_speed",
                "hand_l",
                false,
            ),
            (
                "pre_rig_source_landing_selector",
                "global_position_speed",
                "hand_r",
                false,
            ),
            (
                "pre_rig_source_landing_selector",
                "global_position_speed",
                "lowerarm_l",
                false,
            ),
            (
                "pre_rig_source_landing_selector",
                "global_position_speed",
                "lowerarm_r",
                false,
            ),
            ("pre_rig", "global_position_speed", "hand_l", true),
            ("final", "global_position_speed", "hand_l", true),
            ("final", "global_position_speed", "lowerarm_l", true),
            (
                "pre_rig_source_landing_selector",
                "global_position_speed",
                "head",
                true,
            ),
            (
                "pre_rig_source_landing_selector",
                "local_rotation_spike",
                "hand_l",
                true,
            ),
        ] {
            let mut state = AnimationVisualValidationState::default();
            assert_eq!(
                record_temporal_metrics(
                    &mut state,
                    vec![metric(kind, subject)],
                    stage,
                    None,
                    1.0,
                    12.2101,
                    0.0122101,
                    None,
                    None
                ),
                expected_failure
            );
            assert_eq!(state.temporal_anomalies.len(), 1);
            assert_eq!(
                state.temporal_anomalies[0].source_boundary.is_none(),
                expected_failure
            );
            assert_eq!(!state.faults.is_empty(), expected_failure);
            assert_eq!(
                state.temporal_anomalies[0].stage,
                if stage == "final" { "final" } else { "pre_rig" }
            );
        }
    }

    #[test]
    fn scaled_ragdoll_observation_requires_final_readback_of_each_affected_bone() {
        let readback = ScaledRagdollReadback {
            mesh_scale: [2.0; 3],
            joint_scale: [1.0; 3],
            remaining_updates: 7,
            bodies: BTreeMap::from([
                ("calf_l".to_owned(), [0.00001, 0.00001]),
                ("foot_l".to_owned(), [0.00001, 0.00001]),
            ]),
        };
        for (stage, kind, subject, evidence, expected_failure) in [
            (
                "final",
                "global_position_speed",
                "foot_l",
                Some(&readback),
                false,
            ),
            (
                "final",
                "segment_length_spike",
                "calf_l->foot_l",
                Some(&readback),
                false,
            ),
            (
                "final",
                "segment_length_spike",
                "calf_r->foot_l",
                Some(&readback),
                true,
            ),
            (
                "final",
                "local_scale_spike",
                "foot_l",
                Some(&readback),
                true,
            ),
            (
                "pre_rig",
                "global_position_speed",
                "foot_l",
                Some(&readback),
                true,
            ),
            ("final", "global_position_speed", "foot_l", None, true),
        ] {
            let mut state = AnimationVisualValidationState::default();
            let metric = TemporalSkeletonMetric {
                kind,
                subject: subject.to_owned(),
                previous: 0.0,
                current: 91.25,
                threshold: 40.0,
                trigger: "rate",
                rate: Some(91.25),
                rate_threshold: Some(40.0),
            };
            assert_eq!(
                record_temporal_metrics(
                    &mut state,
                    vec![metric],
                    stage,
                    evidence,
                    1.0,
                    8.0,
                    0.006985542,
                    None,
                    None,
                ),
                expected_failure
            );
            assert_eq!(
                state.temporal_anomalies[0]
                    .source_physics_readback
                    .is_none(),
                expected_failure
            );
        }
    }

    #[test]
    fn source_landing_classification_preserves_later_final_fault_evidence() {
        let directory =
            std::env::temp_dir().join(format!("als_visual_landing_{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let config = AnimationVisualValidationConfig {
            directory: directory.clone(),
            case: "ragdoll".into(),
            capture_count: 0,
            capture_interval: Duration::from_secs(1),
            min_pose_samples: 1,
            warmup: Duration::ZERO,
            max_frame_p95_ms: 25.0,
            max_frame_ms: 75.0,
            max_eval_p95_ms: 4.0,
            max_physics_steps_per_render: 3.0,
        };
        let bones = REQUIRED_BONES
            .iter()
            .map(|name| {
                (
                    name.to_string(),
                    (Vec3::ZERO, Quat::IDENTITY, Vec3::ONE, Vec3::ONE),
                )
            })
            .collect();
        let mut state = AnimationVisualValidationState {
            frame_ms: vec![12.21],
            eval_ms: vec![2.0],
            pose_samples: vec![snapshot(
                1.0,
                12.21,
                &bones,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                RootPoseActivity::default(),
            )],
            ..default()
        };
        let stages = AnimationVisualStageTiming {
            physics_steps_per_render: vec![1.0],
            ..default()
        };
        for (stage, expected_failure) in
            [("pre_rig_source_landing_selector", false), ("final", true)]
        {
            let metric = TemporalSkeletonMetric {
                kind: "global_position_speed",
                subject: "hand_l".into(),
                previous: 0.0,
                current: 46.0,
                threshold: 40.0,
                trigger: "rate",
                rate: Some(46.0),
                rate_threshold: Some(40.0),
            };
            assert_eq!(
                record_temporal_metrics(
                    &mut state,
                    vec![metric],
                    stage,
                    None,
                    1.0,
                    12.21,
                    0.01221,
                    None,
                    None
                ),
                expected_failure
            );
            if !expected_failure {
                state.temporal_anomalies[0].source_selector_inputs =
                    Some([[-13.25, 0.0], [0.0, 1.0]]);
            }
            write_report(&config, &state, &stages, None);
            let report: serde_json::Value =
                serde_json::from_slice(&fs::read(directory.join("report.json")).unwrap()).unwrap();
            assert_eq!(report["pass"], !expected_failure);
        }
        assert_eq!(state.temporal_anomalies.len(), 2);
        assert!(state.temporal_anomalies[0].source_boundary.is_some());
        assert_eq!(state.temporal_anomalies[1].stage, "final");
        assert!(state.temporal_anomalies[1].source_boundary.is_none());
        assert_eq!(
            state.faults,
            ["temporal_final_global_position_speed:hand_l"]
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn pre_rig_capture_uses_current_graph_input_and_skeleton_defaults() {
        use bevy_als::extensions::{
            control_rig_program::RigDefinition,
            root_graph_boundaries::{ControlRigBoundaryNode, RIG_GAME_WORLD_KEY, RigInputBinding},
        };
        use bevy::{
            ecs::system::RunSystemOnce,
            world_serialization::{WorldAsset, WorldInstanceReady, WorldInstanceSpawner},
        };
        use bevy_animation_graph::{
            AnimationGraphPlugin,
            core::{
                animated_scene::AnimatedSceneHandle, animation_clip::EntityPath,
                animation_node::AnimationNode, edge_data::DataSpec, skeleton::Skeleton,
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
        .init_asset::<WorldAsset>()
        .init_resource::<AnimationVisualPreRigPose>();
        app.finish();
        app.cleanup();
        let definition: RigDefinition =
            serde_json::from_str(include_str!("fixtures/rig/cr_als_program.json"))
                .unwrap();
        let reference = Transform::from_xyz(0.0, 0.4, 0.0)
            .with_rotation(Quat::from_rotation_y(0.3))
            .with_scale(Vec3::splat(1.2));
        let mut skeleton = Skeleton::default();
        let mut paths: Vec<Vec<String>> = Vec::new();
        let local = |value: bevy_als::extensions::control_rig_hierarchy::RigTransform| Transform {
            translation: value.translation.as_vec3(),
            rotation: value.rotation.as_quat(),
            scale: value.scale.as_vec3(),
        };
        for bone in &definition.bones {
            let mut path = bone
                .parent
                .map(|index| paths[index].clone())
                .unwrap_or_default();
            path.push(bone.name.clone());
            skeleton.add_bone(
                EntityPath::from(path.clone()),
                if bone.name == "foot_l" {
                    reference
                } else {
                    local(bone.initial_local)
                },
                local(bone.initial_global),
            );
            paths.push(path);
        }
        skeleton.set_root(EntityPath::from(paths[0].clone()).id());
        let foot = EntityPath::from(
            paths[definition
                .bones
                .iter()
                .position(|bone| bone.name == "foot_l")
                .unwrap()]
            .clone(),
        );
        let spine = EntityPath::from(
            paths[definition
                .bones
                .iter()
                .position(|bone| bone.name == "spine_01")
                .unwrap()]
            .clone(),
        );
        let spine_reference = skeleton
            .default_transforms(spine.id())
            .unwrap()
            .local
            .rotation;
        let skeleton = app
            .world_mut()
            .resource_mut::<Assets<Skeleton>>()
            .add(skeleton);
        let mut pose = Pose {
            skeleton: skeleton.clone(),
            ..default()
        };
        pose.add_bone(
            BonePose {
                translation: Some(Vec3::new(0.2, 0.4, 0.0)),
                ..default()
            },
            foot.id(),
        );
        let mut graph = AnimationGraph::new();
        let source = AnimationNode::new(
            "source",
            bevy_als::extensions::GlobalPoseInputNode {
                key: "source".into(),
                output: "Pose".into(),
            },
        );
        let rig = AnimationNode::new(
            "rig",
            ControlRigBoundaryNode {
                definition: Some(definition),
                internalize_time: true,
                input_bindings: vec![RigInputBinding {
                    external: "RigInput".into(),
                    path: vec![5],
                    property: "yaw".into(),
                }],
                ..default()
            },
        );
        graph.add_node(source.clone());
        graph.add_node(rig.clone());
        graph.add_node_parameter_edge(source.id, "Pose", rig.id, "source_pose");
        graph.add_output_data("pose".into(), DataSpec::Pose);
        graph.add_output_data_edge(rig.id, "Pose", "pose");
        let graph = app
            .world_mut()
            .resource_mut::<Assets<AnimationGraph>>()
            .add(graph);
        let mut player = AnimationGraphPlayer::new(skeleton).with_graph(graph.clone());
        player.set_global_input_data("source", DataValue::Pose(pose));
        player.set_global_input_data("animation.delta_seconds", 1.0_f32 / 60.0);
        player.set_global_input_data(RIG_GAME_WORLD_KEY, true);
        player.set_global_input_data("yaw", 30.0_f32);
        let avatar = app
            .world_mut()
            .spawn((
                AlsCharacterAvatar {
                    character: Entity::PLACEHOLDER,
                },
                Transform::IDENTITY,
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
            Name::new("foot_l"),
            Transform::from_xyz(9.0, 8.0, 7.0),
            ChildOf(player_entity),
        ));
        app.world_mut().trigger(WorldInstanceReady {
            entity: avatar,
            instance_id: WorldInstanceSpawner::default().spawn(Handle::<WorldAsset>::default()),
        });
        app.world_mut().flush();
        app.world_mut().run_system_once(animation_player).unwrap();
        assert!(
            app.world()
                .get::<AnimationGraphPlayer>(player_entity)
                .unwrap()
                .get_error()
                .is_none(),
            "{:?}",
            app.world()
                .get::<AnimationGraphPlayer>(player_entity)
                .unwrap()
                .get_error()
        );
        app.world_mut()
            .run_system_once(capture_animation_visual_pre_rig_pose)
            .unwrap();
        let captured = &app.world().resource::<AnimationVisualPreRigPose>().bones["foot_l"];
        assert_eq!(captured.translation, Vec3::new(0.2, 0.4, 0.0));
        assert_eq!(captured.rotation, reference.rotation);
        assert_eq!(captured.scale, reference.scale);
        assert!(
            app.world()
                .resource::<AnimationVisualPreRigPose>()
                .global_bones
                .contains_key("hand_l")
        );
        assert_eq!(
            app.world().resource::<AnimationVisualPreRigPose>().bones["spine_01"].rotation,
            spine_reference
        );
        let output = app
            .world()
            .get::<AnimationGraphPlayer>(player_entity)
            .unwrap()
            .get_default_output_pose()
            .unwrap();
        assert!(
            rotation_delta_radians(
                spine_reference,
                output.get_bone(spine.id()).unwrap().rotation.unwrap()
            ) > 0.05
        );
        let previous = app
            .world()
            .resource::<AnimationVisualPreRigPose>()
            .global_bones["hand_l"]
            .0;
        let moved = Transform::from_xyz(6.0, 2.0, -3.0)
            .with_rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2));
        app.world_mut().entity_mut(avatar).insert(moved);
        app.world_mut()
            .run_system_once(capture_animation_visual_pre_rig_pose)
            .unwrap();
        let captured = app.world().resource::<AnimationVisualPreRigPose>();
        assert!(
            captured.global_bones["hand_l"]
                .0
                .abs_diff_eq(moved.transform_point(previous), 1e-5)
        );
        assert!(captured.root_position.abs_diff_eq(moved.translation, 1e-5));
        app.world_mut()
            .get_mut::<AnimationGraphPlayer>(player_entity)
            .unwrap()
            .start(graph);
        app.world_mut()
            .run_system_once(capture_animation_visual_pre_rig_pose)
            .unwrap();
        assert!(
            app.world()
                .resource::<AnimationVisualPreRigPose>()
                .bones
                .is_empty()
        );
        assert!(
            app.world()
                .resource::<AnimationVisualPreRigPose>()
                .global_bones
                .is_empty()
        );
    }

    #[test]
    fn visual_rig_timing_reads_vm_duration_and_skips_unevaluated_frames() {
        use bevy::ecs::system::RunSystemOnce;
        let mut app = App::new();
        app.insert_resource(AnimationVisualValidationConfig {
            directory: default(),
            case: "fixture".into(),
            capture_count: 0,
            capture_interval: Duration::ZERO,
            min_pose_samples: 1,
            warmup: Duration::ZERO,
            max_frame_p95_ms: 25.0,
            max_frame_ms: 75.0,
            max_eval_p95_ms: 4.0,
            max_physics_steps_per_render: 3.0,
        })
        .insert_resource(AnimationVisualValidationState {
            started: Some(Instant::now()),
            ..default()
        })
        .init_resource::<AnimationVisualStageTiming>();
        let actor = app
            .world_mut()
            .spawn((
                AlsCharacter,
                AlsControlRigExecution {
                    executed_steps: 153,
                    elapsed_seconds: 0.0125,
                    ..default()
                },
            ))
            .id();
        app.world_mut()
            .run_system_once(animation_visual_control_rig_time)
            .unwrap();
        assert_eq!(
            app.world().resource::<AnimationVisualStageTiming>().samples["control_rig"],
            [12.5]
        );
        app.world_mut()
            .get_mut::<AlsControlRigExecution>(actor)
            .unwrap()
            .executed_steps = 0;
        app.world_mut()
            .run_system_once(animation_visual_control_rig_time)
            .unwrap();
        assert_eq!(
            app.world().resource::<AnimationVisualStageTiming>().samples["control_rig"],
            [12.5]
        );
    }
    use bevy::ecs::system::In;

    #[test]
    fn visual_report_waits_for_exit_and_includes_the_final_samples() {
        let directory =
            std::env::temp_dir().join(format!("als_visual_exit_{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let report_path = directory.join("report.json");
        if report_path.exists() {
            fs::remove_file(&report_path).unwrap();
        }
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_message::<AppExit>()
            .insert_resource(AnimationVisualValidationConfig {
                directory: directory.clone(),
                case: "fixture".into(),
                capture_count: 0,
                capture_interval: Duration::from_secs(1),
                min_pose_samples: 1,
                warmup: Duration::ZERO,
                max_frame_p95_ms: 25.0,
                max_frame_ms: 75.0,
                max_eval_p95_ms: 4.0,
                max_physics_steps_per_render: 3.0,
            })
            .insert_resource(AnimationVisualValidationState {
                started: Some(Instant::now() - Duration::from_secs(2)),
                ..default()
            })
            .init_resource::<ManualIntegrationGateReport>()
            .init_resource::<AnimationVisualStageTiming>()
            .add_systems(Last, flush_animation_visual_report);
        app.world_mut().run_schedule(Last);
        assert!(
            !report_path.exists(),
            "running capture must not serialize the accumulated report"
        );
        for exit in [AppExit::Success, AppExit::Error(NonZeroU8::new(7).unwrap())] {
            app.world_mut()
                .resource_mut::<AnimationVisualValidationState>()
                .frame_ms
                .push(12.0);
            let samples = app
                .world()
                .resource::<AnimationVisualValidationState>()
                .frame_ms
                .len();
            let completed = matches!(exit, AppExit::Success);
            {
                let mut manual = app
                    .world_mut()
                    .resource_mut::<ManualIntegrationGateReport>();
                manual.completed = completed;
                manual
                    .direction_trace
                    .push(serde_json::json!({"final_sample": samples}));
                manual.camera_trace.push(
                    crate::manual_playground_integration::ManualCameraTraceSample {
                        phase_elapsed_seconds: samples as f64,
                        obstacles_present: false,
                        obstacles_ignored: true,
                        blocked_ratio: Some(0.3),
                        first_person_requested: false,
                        actual_first_person: false,
                        ragdoll_active: false,
                        camera: bevy_als::als_camera::AlsCameraDebugInfo {
                            trace_distance_ratio: 0.8,
                            valid: true,
                            ..default()
                        },
                    },
                );
            }
            app.world_mut().write_message(exit);
            app.world_mut().run_schedule(Last);
            let report: serde_json::Value =
                serde_json::from_slice(&fs::read(&report_path).unwrap()).unwrap();
            assert_eq!(report["frame_time"]["samples"], samples);
            assert_eq!(report["manual_integration"]["completed"], completed);
            assert_eq!(
                report["manual_integration"]["direction_trace"][samples - 1]["final_sample"],
                samples
            );
            assert_eq!(report["manual_integration"]["camera_trace"][samples - 1]["camera"]["trace_distance_ratio"].as_f64().unwrap() as f32, 0.8_f32);
            fs::remove_file(&report_path).unwrap();
            app.world_mut().run_schedule(Last);
            assert!(!report_path.exists());
        }
        fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn temporal_rate_guard_recognizes_source_equivalent_near_extended_ik() {
        let (previous, current, hip, calf) = include!("fixtures/rig/ik_rate_limits.rs");
        let definition =
            serde_json::from_str(include_str!("fixtures/rig/cr_als_program.json"))
                .unwrap();
        let bones = BTreeMap::from([
            (
                "thigh_r".to_owned(),
                (Vec3::ZERO, Quat::from_array(hip), Vec3::ONE, Vec3::ONE),
            ),
            (
                "calf_r".to_owned(),
                (Vec3::ZERO, Quat::from_array(calf), Vec3::ONE, Vec3::ONE),
            ),
        ]);
        let metric = TemporalSkeletonMetric {
            kind: "local_rotation_spike",
            subject: "calf_r".to_owned(),
            previous: 0.0,
            current: 1.3006219,
            threshold: 80.0,
            trigger: "rate",
            rate: Some(87.05111),
            rate_threshold: Some(80.0),
        };
        assert!(
            rotation_rate_matches_source_ik(&metric, &previous, &current, &definition, &bones),
            "Native IK produces this same calf motion; a raw rate cannot classify it as a porting fault"
        );
        let mut wrong_output = bones.clone();
        wrong_output.get_mut("calf_r").unwrap().1 =
            Quat::from_rotation_x(0.2) * Quat::from_array(calf);
        assert!(!rotation_rate_matches_source_ik(
            &metric,
            &previous,
            &current,
            &definition,
            &wrong_output
        ));
        let mut wrong_previous = previous;
        wrong_previous.target_world[2] += 0.2;
        assert!(!rotation_rate_matches_source_ik(
            &metric,
            &wrong_previous,
            &current,
            &definition,
            &bones
        ));
        let mut wrong_metric = metric.clone();
        wrong_metric.current = 0.9;
        assert!(!rotation_rate_matches_source_ik(
            &wrong_metric,
            &previous,
            &current,
            &definition,
            &bones
        ));
        let mut wrong_metric = metric.clone();
        wrong_metric.subject = "head".to_owned();
        assert!(!rotation_rate_matches_source_ik(
            &wrong_metric,
            &previous,
            &current,
            &definition,
            &bones
        ));
        let mut wrong_metric = metric;
        wrong_metric.trigger = "jump";
        assert!(!rotation_rate_matches_source_ik(
            &wrong_metric,
            &previous,
            &current,
            &definition,
            &bones
        ));
    }

    #[test]
    fn foot_ik_sample_preserves_adjacent_actual_solver_inputs() {
        let previous = bevy_als::foot_ik_apply::FootIkSolveInput {
            weight: 0.46678212,
            target_world: [1.0, 0.4, 2.0],
            ..default()
        };
        let current = bevy_als::foot_ik_apply::FootIkSolveInput {
            weight: 0.20929879,
            target_world: [1.0, 0.7, 2.0],
            ..previous
        };
        let space = AlsFootIkSpaceSide {
            solve_input: Some(current),
            previous_solve_input: Some(previous),
            ..default()
        };
        let sample = serde_json::to_value(foot_ik_side_sample(&space, &default())).unwrap();
        assert_eq!(
            sample["solve_input"],
            serde_json::to_value(current).unwrap()
        );
        assert_eq!(
            sample["previous_solve_input"],
            serde_json::to_value(previous).unwrap()
        );
        let skipped = AlsFootIkSpaceSide {
            solve_input: None,
            ..space
        };
        assert!(
            serde_json::to_value(foot_ik_side_sample(&skipped, &default())).unwrap()["solve_input"]
                .is_null()
        );
    }

    #[test]
    fn rig_inputs_sample_preserves_double_precision_and_call_inputs() {
        use bevy_als::extensions::{
            control_rig_hierarchy::RigTransform,
            control_rig_math::RigValue,
            control_rig_program::{RigDefinition, RigInstruction},
            root_graph_boundaries::{ControlRigNodeState, RigCallSample},
        };
        let mut definition: RigDefinition =
            serde_json::from_str(include_str!("fixtures/rig/cr_als_program.json"))
                .unwrap();
        definition.program.instructions = [
            "FRigUnit_TwoBoneIKSimplePerItem::Execute",
            "FRigUnit_GetTransform::Execute",
        ]
        .map(|function| RigInstruction::Call {
            function: function.into(),
            arguments: Vec::new(),
            outputs: Vec::new(),
        })
        .to_vec();
        let value = 1.0_f64 + 1e-10;
        let transform = RigTransform {
            translation: bevy::math::DVec3::splat(value),
            ..default()
        };
        let mut state = ControlRigNodeState::default();
        state.initialization_count = 2;
        state.calls = (0..2)
            .map(|instruction| RigCallSample {
                instruction,
                arguments: vec![RigValue::F64(value)],
                bones_before: vec![(3, transform)],
            })
            .collect();
        let sample = rig_inputs_sample(&definition, &state);
        assert_eq!(sample["initialization_count"], 2);
        assert_eq!(sample["calls"].as_array().unwrap().len(), 1);
        assert_eq!(
            sample["calls"][0]["arguments"][0]["F64"].as_f64(),
            Some(value)
        );
        assert_eq!(
            sample["calls"][0]["bones_before"][0][1]["translation"][0].as_f64(),
            Some(value)
        );
        assert_ne!(Some(value), Some(value as f32 as f64));
    }

    #[test]
    fn source_pose_node_selects_repeated_titles_by_source_identity() {
        use bevy_animation_graph::core::animation_node::AnimationNode;
        let mut graph = AnimationGraph::new();
        let nodes = [0.0, 1.0].map(|explicit_value| {
            AnimationNode::new(
                "Sequence Evaluator",
                bevy_als::extensions::SourceSequenceEvaluatorNode {
                    explicit_value,
                    ..default()
                },
            )
        });
        for node in &nodes {
            graph.add_node(node.clone());
        }
        for node in &nodes {
            assert_eq!(
                source_pose_node(&graph, &node.id.uuid().to_string()),
                Some(node.id)
            );
        }
        assert!(source_pose_node(&graph, "Sequence Evaluator").is_some());
        assert_eq!(source_pose_node(&graph, "absent"), None);
    }

    #[test]
    fn motion_observation_duration_uses_game_time_without_changing_wall_traces() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()));
        let player = AnimationGraphPlayer::new(default());
        let graphs = Assets::<AnimationGraph>::default();
        let asset_server = app.world().resource::<AssetServer>();
        for (game_span, wall_span) in [(0.7, 0.1166511), (0.06, 1.0), (0.0, 1.0), (0.3, 0.3)] {
            let mut state = AnimationVisualValidationState::default();
            let evidence = ManualAnimationWindowEvidence {
                label: Some("clock_fixture"),
                settled: true,
                capture_source_window: true,
                requires_pre_rig_motion: true,
                minimum_observation_seconds: 0.12,
                observed_animation_clip_time: Some(0.0),
                ..default()
            };
            let pose = AnimationVisualPreRigPose::default();
            for (game, wall) in [(10.0, 20.0), (10.0 + game_span, 20.0 + wall_span)] {
                record_motion_window(
                    &mut state,
                    &evidence,
                    &pose,
                    &pose.bones,
                    None,
                    None,
                    RootPoseActivity::default(),
                    None,
                    &player,
                    asset_server,
                    &graphs,
                    wall,
                    game,
                );
            }
            let tracker = &state.motion_windows["clock_fixture"];
            assert!(
                (f64::from(tracker.observation_seconds) - game_span).abs() < 1e-6,
                "game span {game_span} used wall span {wall_span}: {}",
                tracker.observation_seconds
            );
            assert_eq!(
                tracker.observation_seconds >= tracker.minimum_observation_seconds,
                game_span >= 0.12
            );
            assert!(
                (tracker.actor_yaw_trace[1].elapsed_secs
                    - tracker.actor_yaw_trace[0].elapsed_secs
                    - wall_span)
                    .abs()
                    < 1e-9
            );
        }
    }

    #[test]
    fn canonical_clip_signature_uses_nearest_real_clip_sample_without_fsm_or_sync_group() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()));
        let mut state = AnimationVisualValidationState::default();
        let player = AnimationGraphPlayer::new(default());
        let graphs = Assets::<AnimationGraph>::default();
        let asset_server = app.world().resource::<AssetServer>();
        for (frame, time) in [0.51, 0.61, 0.9, f32::NAN].into_iter().enumerate() {
            let evidence = ManualAnimationWindowEvidence {
                label: Some("front"),
                settled: true,
                capture_source_window: true,
                requires_pre_rig_motion: true,
                direction_signature_group: None,
                canonical_clip_time: Some(0.6),
                maximum_clip_time_distance: 0.1,
                observed_animation_clip_time: Some(time),
                expected_animation_variant: Some("FixtureClip"),
                observed_animation_variant: Some("FixtureClip"),
                observed_animation_weight: Some(1.0),
                observed_animation_play_rate: Some(1.25),
                ..default()
            };
            let pose = AnimationVisualPreRigPose {
                bones: BTreeMap::from([(
                    "thigh_l".to_owned(),
                    LocalBonePose {
                        translation: Vec3::X * time,
                        rotation: Quat::IDENTITY,
                        scale: Vec3::ONE,
                    },
                )]),
                ..default()
            };
            record_motion_window(
                &mut state,
                &evidence,
                &pose,
                &pose.bones,
                None,
                None,
                RootPoseActivity::default(),
                None,
                &player,
                asset_server,
                &graphs,
                frame as f64 * 0.1,
                frame as f64 * 0.1,
            );
        }
        let tracker = &state.motion_windows["front"];
        assert!((tracker.canonical_clip_time_distance.unwrap() - 0.01).abs() < 1e-5);
        assert_eq!(
            tracker.canonical_pre_rig_pose["thigh_l"].translation.x,
            0.61
        );
        assert!(tracker.canonical_sync_phase_distance.is_none());
        assert!(tracker.observed_runtime_states.is_empty());
        let trace = serde_json::to_value(&tracker.actor_yaw_trace).unwrap();
        assert_eq!(
            trace.as_array().unwrap().len(),
            4,
            "A clip without actor yaw still requires its actual playback trace"
        );
        assert!(trace[0]["actor_yaw_radians"].is_null());
        assert_eq!(trace[0]["animation_variant"], "FixtureClip");
        assert_eq!(trace[0]["animation_weight"], 1.0);
        assert_eq!(trace[0]["animation_play_rate"], 1.25);
        assert!(trace[3]["animation_clip_time"].is_null());
    }

    fn collected_bone_count(
        In(root): In<Entity>,
        children: Query<&Children>,
        named_transforms: Query<(&Name, &GlobalTransform, &Transform)>,
    ) -> usize {
        collect_bones(root, &children, &named_transforms).len()
    }

    #[test]
    fn detached_player_remains_the_visual_bone_collection_root() {
        let mut world = World::new();
        let avatar = world.spawn_empty().id();
        let player = world
            .spawn((
                Name::new("player"),
                Transform::IDENTITY,
                GlobalTransform::IDENTITY,
            ))
            .id();
        let pelvis = world
            .spawn((
                Name::new("pelvis"),
                Transform::IDENTITY,
                GlobalTransform::IDENTITY,
            ))
            .id();
        world.entity_mut(avatar).add_child(player);
        world.entity_mut(player).add_child(pelvis);
        world.entity_mut(player).remove::<ChildOf>();
        assert_eq!(
            world
                .run_system_cached_with(collected_bone_count, avatar)
                .unwrap(),
            0
        );
        assert_eq!(
            world
                .run_system_cached_with(collected_bone_count, player)
                .unwrap(),
            1
        );
    }

    #[test]
    fn ragdoll_root_space_sample_records_parent_and_follow_coordinates() {
        let character_transform = Transform::from_xyz(2.0, 1.0, -3.0);
        let character_position = Position(Vec3::new(2.0, 1.0, -3.0));
        let avatar_local = Transform::from_xyz(0.0, -0.92, 0.0);
        let avatar_world = GlobalTransform::from(Transform::from_xyz(2.0, 0.08, -3.0));
        let player_local = Transform::from_xyz(0.0, 0.0, 0.0);
        let player_world = GlobalTransform::from(Transform::from_xyz(2.0, 0.08, -3.0));
        let follow = AlsRagdollGroundFollow {
            pelvis: Vec3::new(4.0, 0.5, -2.0),
            target: Vec3::new(4.0, 1.0, -2.0),
            ..default()
        };
        let sample = ragdoll_root_space_sample(
            &character_transform,
            Some(&character_position),
            &avatar_local,
            &avatar_world,
            Some(&player_local),
            Some(&player_world),
            Some(&follow),
        );
        assert_eq!(sample.character_transform_translation, [2.0, 1.0, -3.0]);
        assert_eq!(sample.character_position, Some([2.0, 1.0, -3.0]));
        assert_eq!(sample.avatar_world_translation, [2.0, 0.08, -3.0]);
        assert_eq!(sample.player_world_translation, Some([2.0, 0.08, -3.0]));
        assert_eq!(sample.follow_pelvis, Some([4.0, 0.5, -2.0]));
        assert_eq!(sample.follow_target, Some([4.0, 1.0, -2.0]));
    }

    #[test]
    fn visual_evidence_format_is_loaded_from_the_shared_contract() {
        assert_eq!(
            animation_visual_evidence_format(),
            "als_animation_visual_evidence_v24"
        );
    }

    #[test]
    fn graph_error_diagnostic_preserves_node_time_and_a_pose_risk() {
        let node = NodeId::default();
        let error = GraphError::MissingEdgeToSource(SourcePin::NodeTime(node));
        let pin = graph_error_pin(&error).unwrap();
        assert_eq!(graph_error_kind(&error), "MissingEdgeToSource");
        assert_eq!(pin.kind, "node_time");
        assert_eq!(pin.node_id, Some(format!("{node:?}")));
        assert_eq!(graph_error_node_id(&error), Some(node));
        assert_eq!(
            graph_error_pose_classification(RootPoseActivity::default()),
            "missing_root_pose_likely_reference_or_a_pose"
        );
    }

    #[test]
    fn overlay_object_graph_failure_uses_the_public_failure_report() {
        let (mut app, object, player_entity) = crate::test_support::loaded_bow_scene();
        app.world_mut()
            .spawn(bevy_als::overlay_object::AlsOverlayObject {
                character: Entity::PLACEHOLDER,
                content: object,
                overlay: bevy_als::als::state::OverlayState::Bow,
                definition: crate::test_support::SOURCE_LIBRARY.0
                    [&bevy_als::als::state::OverlayState::Bow]
                    .object
                    .clone()
                    .unwrap(),
                attachment_bone: bevy_als::overlay_object::LEFT_GUN_BONE.into(),
            });
        let directory =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/validation/object_fault");
        fs::create_dir_all(&directory).unwrap();
        let report_path = directory.join("report.json");
        if report_path.exists() {
            fs::remove_file(&report_path).unwrap();
        }
        app.insert_resource(AnimationVisualValidationConfig {
            directory,
            case: "bow".into(),
            capture_count: 0,
            capture_interval: Duration::from_secs(1),
            min_pose_samples: 1,
            warmup: Duration::ZERO,
            max_frame_p95_ms: 25.0,
            max_frame_ms: 75.0,
            max_eval_p95_ms: 4.0,
            max_physics_steps_per_render: 3.0,
        })
        .init_resource::<AnimationVisualValidationState>()
        .init_resource::<AnimationVisualStageTiming>()
        .init_resource::<AnimationVisualPreRigPose>()
        .insert_resource(ManualRuntimePhaseEvidence {
            phase: "BowOverlay".into(),
            ..default()
        })
        .add_systems(
            PostUpdate,
            (
                collect_animation_visual_evidence,
                finish_fatal_animation_visual_failure,
            )
                .chain()
                .after(AlsCameraSet),
        );
        let handle = match app
            .world()
            .get::<AnimationGraphPlayer>(player_entity)
            .unwrap()
            .get_animation_source()
        {
            AnimationSource::Graph(handle) => handle.clone(),
            _ => panic!("Object must use its generated graph"),
        };
        let mut graph = AnimationGraph::new();
        graph.add_output_data(
            "pose".into(),
            bevy_animation_graph::core::edge_data::DataSpec::Pose,
        );
        *app.world_mut()
            .resource_mut::<Assets<AnimationGraph>>()
            .get_mut(&handle)
            .unwrap() = graph;
        app.world_mut()
            .get_mut::<AnimationGraphPlayer>(player_entity)
            .unwrap()
            .start(handle);
        app.update();
        let report: serde_json::Value = serde_json::from_slice(
            &fs::read(report_path)
                .expect("Object graph failure must write a report on its first frame"),
        )
        .unwrap();
        let failure = &report["primary_failure"];
        assert_eq!(
            failure["root_graph_path"],
            "ALS/OverlayObjects/Bow/AB_Als_Bow.runtime.animgraph.ron"
        );
        assert_eq!(failure["phase"], "BowOverlay");
        assert_eq!(failure["category"], "graph_evaluation_error");
        assert_eq!(failure["pin"]["pin_id"], "pose");
        assert!(failure["raw_error"].as_str().unwrap().contains("pose"));
        assert_eq!(failure["screenshot_status"], "disabled_by_configuration");
        let mut exits = app.world().resource::<Messages<AppExit>>().get_cursor();
        app.update();
        assert!(
            exits
                .read(app.world().resource::<Messages<AppExit>>())
                .any(|event| matches!(event, AppExit::Error(code) if code.get() == 92))
        );
    }

    #[test]
    fn camera_graph_failure_is_reported_before_body_ready_and_exits() {
        use bevy::ecs::system::RunSystemOnce;
        use bevy_animation_graph::core::animation_node::AnimationNode;
        for failure_case in ["edge", "missing", "empty"] {
            let (mut app, _, component, _) = crate::test_support::loaded_camera_scene();
            let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("target/validation/camera")
                .join(format!("c4c_camera_fault_{failure_case}"));
            fs::create_dir_all(&directory).unwrap();
            let report_path = directory.join("report.json");
            if report_path.exists() {
                fs::remove_file(&report_path).unwrap();
            }
            app.insert_resource(AnimationVisualValidationConfig {
                directory,
                case: "camera".into(),
                capture_count: 0,
                capture_interval: Duration::from_secs(1),
                min_pose_samples: 1,
                warmup: Duration::ZERO,
                max_frame_p95_ms: 25.0,
                max_frame_ms: 75.0,
                max_eval_p95_ms: 4.0,
                max_physics_steps_per_render: 3.0,
            })
            .init_resource::<AnimationVisualValidationState>()
            .init_resource::<AnimationVisualStageTiming>()
            .init_resource::<AnimationVisualPreRigPose>()
            .insert_resource(ManualRuntimePhaseEvidence {
                phase: "Camera".into(),
                gait: "Walking".into(),
                ..default()
            })
            .add_systems(
                PostUpdate,
                (
                    collect_animation_visual_evidence,
                    finish_fatal_animation_visual_failure,
                )
                    .chain()
                    .after(AlsCameraSet),
            );
            let player_entity = app
                .world()
                .get::<AnimatedSceneInstance>(component)
                .unwrap()
                .player_entity();
            let handle = match app
                .world()
                .get::<AnimationGraphPlayer>(player_entity)
                .unwrap()
                .get_animation_source()
            {
                AnimationSource::Graph(handle) => handle.clone(),
                _ => panic!("Camera must use its source graph"),
            };
            app.world_mut()
                .run_system_once(collect_animation_visual_evidence)
                .unwrap();
            assert!(
                !report_path.exists(),
                "Valid camera reference pose was rejected"
            );
            if failure_case == "edge" {
                let material_fault = app
                    .world_mut()
                    .spawn(
                        bevy_als::extensions::overlay_materials::AvatarMaterialApplied {
                            slots: 16,
                            body_slots: 15,
                            main_slots: 1,
                            source_slot_pattern: false,
                        },
                    )
                    .id();
                app.world_mut()
                    .run_system_once(collect_animation_visual_evidence)
                    .unwrap();
                assert!(
                    app.world()
                        .resource::<AnimationVisualValidationState>()
                        .faults
                        .iter()
                        .any(|fault| fault == "mesh_material_source_slot_pattern_mismatch")
                );
                app.world_mut().entity_mut(material_fault).despawn();
                app.world_mut()
                    .resource_mut::<AnimationVisualValidationState>()
                    .faults
                    .clear();
            }
            let skeleton = app
                .world()
                .get::<AnimationGraphPlayer>(player_entity)
                .unwrap()
                .skeleton()
                .clone();
            let mut fresh_player = AnimationGraphPlayer::new(skeleton);
            fresh_player.start(handle.clone());
            app.world_mut()
                .entity_mut(player_entity)
                .insert(fresh_player);
            app.world_mut()
                .run_system_once(collect_animation_visual_evidence)
                .unwrap();
            assert!(!report_path.exists(), "Unevaluated camera was rejected");
            let mut graph = AnimationGraph::new();
            if failure_case != "missing" {
                graph.add_output_data(
                    "pose".into(),
                    bevy_animation_graph::core::edge_data::DataSpec::Pose,
                );
            }
            if failure_case == "empty" {
                let node = AnimationNode::new(
                    "Empty camera pose fixture",
                    bevy_als::extensions::LocalRefPoseNode {
                        internalize_time: true,
                    },
                );
                graph.add_output_data_edge(node.id, "pose", "pose");
                graph.add_node(node);
            }
            *app.world_mut()
                .resource_mut::<Assets<AnimationGraph>>()
                .get_mut(&handle)
                .unwrap() = graph;
            app.world_mut()
                .get_mut::<AnimationGraphPlayer>(player_entity)
                .unwrap()
                .start(handle);
            app.world_mut()
                .get_mut::<bevy_als::camera_component::AlsSourceCameraComponent>(component)
                .unwrap()
                .deactivate();
            app.update();
            assert!(
                !report_path.exists(),
                "Inactive camera was evaluated as a failure"
            );
            app.world_mut()
                .get_mut::<bevy_als::camera_component::AlsSourceCameraComponent>(component)
                .unwrap()
                .activate(true);
            app.update();
            assert!(
                report_path.exists(),
                "Camera {failure_case} failure was hidden by missing body Avatar"
            );
            let report: serde_json::Value =
                serde_json::from_slice(&fs::read(&report_path).unwrap()).unwrap();
            let failure = &report["primary_failure"];
            assert_eq!(failure["case"], "camera");
            assert_eq!(failure["phase"], "Camera");
            assert_eq!(failure["reported_before_ready"], true);
            assert_eq!(failure["screenshot_status"], "disabled_by_configuration");
            assert!(failure["screenshot_path"].is_null());
            assert_eq!(
                failure["root_graph_path"],
                "ALSCamera/AB_Als_Camera.runtime.animgraph.ron"
            );
            if failure_case == "edge" {
                assert_eq!(failure["category"], "graph_evaluation_error");
                assert!(failure["raw_error"].as_str().unwrap().contains("pose"));
                assert_eq!(failure["pin"]["pin_id"], "pose");
            } else {
                assert!(
                    app.world()
                        .get::<AnimationGraphPlayer>(player_entity)
                        .unwrap()
                        .get_error()
                        .is_none()
                );
                assert_eq!(failure["category"], "root_pose_contract_error");
                assert_eq!(
                    failure["error_kind"],
                    if failure_case == "missing" {
                        "MissingRootPose"
                    } else {
                        "EmptyRootPose"
                    }
                );
            }
            let mut exits = app.world().resource::<Messages<AppExit>>().get_cursor();
            assert!(
                exits
                    .read(app.world().resource::<Messages<AppExit>>())
                    .next()
                    .is_none()
            );
            app.update();
            assert!(
                exits
                    .read(app.world().resource::<Messages<AppExit>>())
                    .any(|event| matches!(event, AppExit::Error(code) if code.get() == 92))
            );
            let final_report: serde_json::Value =
                serde_json::from_slice(&fs::read(&report_path).unwrap()).unwrap();
            assert_eq!(final_report["primary_failure"], *failure);
        }
    }

    #[test]
    fn stats_uses_ordered_percentiles() {
        let value = stats(&[4.0, 1.0, 3.0, 2.0, 5.0]);
        assert_eq!(value.samples, 5);
        assert_eq!(value.p50_ms, 3.0);
        assert_eq!(value.p95_ms, 5.0);
        assert_eq!(value.max_ms, 5.0);
    }

    #[test]
    fn actor_yaw_direction_tracks_multiple_turns_and_still_rejects_reversal() {
        for direction in [-1.0_f32, 1.0] {
            let mut tracker = MotionWindowTracker {
                expected_actor_yaw_delta_sign: Some(1),
                actor_yaw_reference: Some(0.0),
                ..default()
            };
            for step in 0..=48 {
                record_actor_yaw_delta(
                    &mut tracker,
                    wrapped_yaw_delta(0.0, direction * (step as f32 * 15.0).to_radians()),
                );
            }
            let (expected, opposite) = if direction > 0.0 {
                (
                    tracker.max_actor_yaw_signed_delta_radians,
                    tracker.max_actor_yaw_opposite_delta_radians,
                )
            } else {
                (
                    tracker.max_actor_yaw_opposite_delta_radians,
                    tracker.max_actor_yaw_signed_delta_radians,
                )
            };
            assert!((expected - 2.0 * std::f32::consts::TAU).abs() < 1e-5);
            assert!(opposite < 1e-6);
        }
    }

    #[test]
    fn wrapped_yaw_delta_preserves_turn_side() {
        assert!(
            (wrapped_yaw_delta(0.0, 90.0_f32.to_radians()) - 90.0_f32.to_radians()).abs() < 1e-6
        );
        assert!(
            (wrapped_yaw_delta(0.0, -90.0_f32.to_radians()) + 90.0_f32.to_radians()).abs() < 1e-6
        );
        assert!(
            (wrapped_yaw_delta(170.0_f32.to_radians(), -170.0_f32.to_radians())
                - 20.0_f32.to_radians())
            .abs()
                < 1e-6
        );
    }

    #[test]
    fn lifecycle_pulse_is_recorded_without_a_pose_sample() {
        let mut state = AnimationVisualValidationState::default();
        let evidence = ManualAnimationWindowEvidence {
            label: Some("turn"),
            capture_source_window: true,
            expected_actor_yaw_baseline: Some(0.5),
            expected_lifecycle_stages: &["slot_active", "natural_completion", "settled"],
            observed_lifecycle_stages: &["natural_completion", "settled"],
            ..default()
        };
        assert_eq!(
            record_motion_window_metadata(&mut state, &evidence).as_deref(),
            Some("turn")
        );
        let tracker = state.motion_windows.get("turn").unwrap();
        assert_eq!(tracker.sample_count, 0);
        assert!(tracker.expected_lifecycle_stages.contains("slot_active"));
        assert!(
            tracker
                .observed_lifecycle_stages
                .contains("natural_completion")
        );
        assert!(tracker.observed_lifecycle_stages.contains("settled"));
        assert!(tracker.expected_actor_yaw_baseline.is_none());
        assert!(tracker.actor_yaw_reference.is_none());
    }

    #[test]
    fn precondition_window_cannot_record_lifecycle_or_yaw_baseline() {
        let mut state = AnimationVisualValidationState::default();
        let evidence = ManualAnimationWindowEvidence {
            label: Some("turn"),
            capture_source_window: false,
            expected_actor_yaw_baseline: Some(0.5),
            expected_lifecycle_stages: &["slot_active"],
            observed_lifecycle_stages: &["slot_active"],
            ..default()
        };
        assert!(record_motion_window_metadata(&mut state, &evidence).is_none());
        let tracker = state.motion_windows.get("turn").unwrap();
        assert!(tracker.expected_lifecycle_stages.contains("slot_active"));
        assert!(tracker.observed_lifecycle_stages.is_empty());
        assert!(tracker.expected_actor_yaw_baseline.is_none());
        assert!(tracker.actor_yaw_reference.is_none());
    }

    #[test]
    fn terminal_lifecycle_forces_a_pose_sample() {
        let active = ManualAnimationWindowEvidence {
            capture_source_window: true,
            observed_lifecycle_stages: &["slot_active"],
            ..default()
        };
        assert!(!terminal_lifecycle_sample_required(&active));
        let completion = ManualAnimationWindowEvidence {
            capture_source_window: true,
            observed_lifecycle_stages: &["natural_completion", "inertialization"],
            ..default()
        };
        assert!(terminal_lifecycle_sample_required(&completion));
        let settled = ManualAnimationWindowEvidence {
            capture_source_window: true,
            observed_lifecycle_stages: &["sequence_completed", "settled"],
            ..default()
        };
        assert!(terminal_lifecycle_sample_required(&settled));
        let precondition = ManualAnimationWindowEvidence {
            capture_source_window: false,
            observed_lifecycle_stages: &["settled"],
            ..default()
        };
        assert!(!terminal_lifecycle_sample_required(&precondition));
    }

    #[test]
    fn settled_lifecycle_routes_parent_state_to_the_terminal_contract() {
        let active = ManualAnimationWindowEvidence {
            capture_source_window: true,
            observed_lifecycle_stages: &["slot_active"],
            ..default()
        };
        assert!(!terminal_parent_state_sample(&active));
        let completion = ManualAnimationWindowEvidence {
            capture_source_window: true,
            observed_lifecycle_stages: &["natural_completion", "inertialization"],
            ..default()
        };
        assert!(!terminal_parent_state_sample(&completion));
        let settled = ManualAnimationWindowEvidence {
            capture_source_window: true,
            observed_lifecycle_stages: &["sequence_completed", "settled"],
            ..default()
        };
        assert!(terminal_parent_state_sample(&settled));
    }

    #[test]
    fn segment_name_is_stable() {
        assert_eq!(segment_name("pelvis", "spine_01"), "pelvis->spine_01");
    }

    fn temporal_local_pose(calf_rotation: Quat) -> BTreeMap<String, LocalBonePose> {
        BTreeMap::from([
            (
                "thigh_l".to_owned(),
                LocalBonePose {
                    scale: Vec3::ONE,
                    ..default()
                },
            ),
            (
                "calf_l".to_owned(),
                LocalBonePose {
                    rotation: calf_rotation,
                    scale: Vec3::ONE,
                    ..default()
                },
            ),
            (
                "foot_l".to_owned(),
                LocalBonePose {
                    scale: Vec3::ONE,
                    ..default()
                },
            ),
        ])
    }

    fn temporal_bones(knee_x: f32) -> BTreeMap<String, FinalBone> {
        BTreeMap::from([
            (
                "thigh_l".to_owned(),
                (
                    Vec3::new(0.0, 1.0, 0.0),
                    Quat::IDENTITY,
                    Vec3::ONE,
                    Vec3::ONE,
                ),
            ),
            (
                "calf_l".to_owned(),
                (
                    Vec3::new(knee_x, 0.5, 0.0),
                    Quat::IDENTITY,
                    Vec3::ONE,
                    Vec3::ONE,
                ),
            ),
            (
                "foot_l".to_owned(),
                (Vec3::ZERO, Quat::IDENTITY, Vec3::ONE, Vec3::ONE),
            ),
        ])
    }

    #[test]
    fn temporal_guard_accepts_native_uphill_landing_motion() {
        let mut rejected = Vec::new();
        for (name, previous, current, delta) in [
            (
                "hand_l",
                Vec3::new(0.5178596, 1.4902093, 0.566879),
                Vec3::new(0.46625707, 1.1090595, 0.687198),
                0.0148442,
            ),
            (
                "calf_l",
                Vec3::new(-0.1704133, 0.4692102, -0.2130673),
                Vec3::new(-0.15643789, 0.39046794, 0.07042244),
                0.0100415,
            ),
            (
                "hand_r",
                Vec3::new(-0.66575725, 1.2844432, 0.060162455),
                Vec3::new(-0.6196818, 1.1758767, 0.5142271),
                0.0141097,
            ),
        ] {
            let bones = |position| {
                BTreeMap::from([(
                    name.to_owned(),
                    (position, Quat::IDENTITY, Vec3::ONE, Vec3::ONE),
                )])
            };
            let mut guard = TemporalSkeletonGuard::default();
            guard.observe(&BTreeMap::new(), &bones(previous), Vec3::ZERO, delta);
            rejected.extend(
                guard
                    .observe(&BTreeMap::new(), &bones(current), Vec3::ZERO, delta)
                    .metrics,
            );
        }
        for (rotations, delta) in [
            (
                [
                    Quat::from_xyzw(-0.0029632404, -0.24226911, 0.007246189, 0.9701775),
                    Quat::from_xyzw(-0.00900114, -0.61373883, 0.02740959, 0.78898185),
                ],
                0.0159982,
            ),
            (
                [
                    Quat::from_xyzw(-0.006461881, -0.40865546, 0.017139463, 0.9125049),
                    Quat::from_xyzw(-0.012386095, -0.73553616, 0.040971845, 0.676132),
                ],
                0.0122713,
            ),
        ] {
            let poses = rotations.map(|rotation| {
                BTreeMap::from([(
                    "calf_r".to_owned(),
                    LocalBonePose {
                        rotation,
                        scale: Vec3::ONE,
                        ..default()
                    },
                )])
            });
            let mut guard = TemporalSkeletonGuard::default();
            guard.observe(&poses[0], &BTreeMap::new(), Vec3::ZERO, delta);
            rejected.extend(
                guard
                    .observe(&poses[1], &BTreeMap::new(), Vec3::ZERO, delta)
                    .metrics,
            );
        }
        assert!(
            rejected.is_empty(),
            "reference motion was rejected: {:?}",
            rejected
                .iter()
                .map(|metric| (metric.subject.as_str(), metric.current))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn temporal_guard_accepts_source_ik_ten_fps_and_rejects_a_full_flip() {
        let local = temporal_local_pose(Quat::IDENTITY);
        let bones = temporal_bones(0.25);
        let mut source_bones = temporal_bones(-0.710_181_8 * 0.25);
        source_bones.get_mut("calf_l").unwrap().0.z = (1.0 - 0.710_181_8_f32.powi(2)).sqrt() * 0.25;
        let mut guard = TemporalSkeletonGuard::default();
        guard.observe(&local, &bones, Vec3::ZERO, 0.1);
        let source = guard.observe(
            &temporal_local_pose(Quat::from_rotation_x(2.447_425_6)),
            &source_bones,
            Vec3::ZERO,
            0.1,
        );
        assert!(
            !source
                .metrics
                .iter()
                .any(|metric| matches!(metric.kind, "local_rotation_spike" | "limb_plane_flip"))
        );

        let mut guard = TemporalSkeletonGuard::default();
        guard.observe(&local, &bones, Vec3::ZERO, 0.1);
        let flip = guard.observe(
            &temporal_local_pose(Quat::from_rotation_x(std::f32::consts::PI)),
            &temporal_bones(-0.25),
            Vec3::ZERO,
            0.1,
        );
        assert!(
            flip.metrics
                .iter()
                .any(|metric| metric.kind == "local_rotation_spike")
        );
        assert!(
            flip.metrics
                .iter()
                .any(|metric| metric.kind == "limb_plane_flip")
        );
    }

    #[test]
    fn temporal_guard_uses_the_animation_step_after_a_collection_stall() {
        for (angle, animation_delta, expected_spike) in
            [(0.76698923, 0.0854492, false), (1.2263747, 0.0148859, true)]
        {
            let pre_rig = AnimationVisualPreRigPose {
                delta_seconds: animation_delta,
                ..default()
            };
            let delta = pre_rig.temporal_delta_seconds();
            let mut guard = TemporalSkeletonGuard::default();
            let bones = temporal_bones(0.25);
            guard.observe(
                &temporal_local_pose(Quat::IDENTITY),
                &bones,
                Vec3::ZERO,
                delta,
            );
            let observation = guard.observe(
                &temporal_local_pose(Quat::from_rotation_x(angle)),
                &bones,
                Vec3::ZERO,
                delta,
            );
            assert_eq!(
                observation
                    .metrics
                    .iter()
                    .any(|metric| metric.kind == "local_rotation_spike"),
                expected_spike
            );
        }
    }

    #[test]
    fn temporal_guard_rejects_an_instant_local_leg_twist() {
        let mut guard = TemporalSkeletonGuard::default();
        let bones = temporal_bones(0.25);
        assert!(
            guard
                .observe(
                    &temporal_local_pose(Quat::IDENTITY),
                    &bones,
                    Vec3::ZERO,
                    1.0 / 60.0,
                )
                .metrics
                .is_empty()
        );
        let observation = guard.observe(
            &temporal_local_pose(Quat::from_rotation_x(std::f32::consts::PI)),
            &bones,
            Vec3::ZERO,
            1.0 / 60.0,
        );
        assert!(
            observation
                .metrics
                .iter()
                .any(|value| value.kind == "local_rotation_spike" && value.subject == "calf_l")
        );
    }

    #[test]
    fn temporal_guard_accepts_source_additive_joint_motion_through_a_degenerate_plane() {
        let mut guard = TemporalSkeletonGuard::default();
        for (thigh, calf) in [
            (
                Quat::from_xyzw(0.092057794, 0.34182903, 0.018209398, 0.93506515),
                Quat::from_xyzw(-0.05224237, -0.82909095, 0.040767312, 0.555173),
            ),
            (
                Quat::from_xyzw(0.14291288, 0.78061044, 0.005640864, 0.60843354),
                Quat::from_xyzw(0.018322527, -0.99552083, 0.083647355, -0.04007089),
            ),
        ] {
            let local = BTreeMap::from([
                (
                    "thigh_l".to_owned(),
                    LocalBonePose {
                        rotation: thigh.normalize(),
                        scale: Vec3::ONE,
                        ..default()
                    },
                ),
                (
                    "calf_l".to_owned(),
                    LocalBonePose {
                        translation: Vec3::NEG_X * 0.422,
                        rotation: calf.normalize(),
                        scale: Vec3::ONE,
                    },
                ),
                (
                    "foot_l".to_owned(),
                    LocalBonePose {
                        translation: Vec3::NEG_X * 0.402,
                        rotation: Quat::IDENTITY,
                        scale: Vec3::ONE,
                    },
                ),
            ]);
            let thigh = Transform::from_rotation(thigh.normalize());
            let knee = thigh
                * Transform::from_translation(Vec3::NEG_X * 0.422).with_rotation(calf.normalize());
            let ankle = knee * Transform::from_translation(Vec3::NEG_X * 0.402);
            let bones = BTreeMap::from([
                (
                    "thigh_l".to_owned(),
                    (thigh.translation, thigh.rotation, Vec3::ONE, Vec3::ONE),
                ),
                (
                    "calf_l".to_owned(),
                    (knee.translation, knee.rotation, Vec3::ONE, Vec3::ONE),
                ),
                (
                    "foot_l".to_owned(),
                    (ankle.translation, ankle.rotation, Vec3::ONE, Vec3::ONE),
                ),
            ]);
            let metrics = guard.observe(&local, &bones, Vec3::ZERO, 0.0810643).metrics;
            assert!(
                !metrics
                    .iter()
                    .any(|metric| metric.kind == "limb_plane_flip"),
                "Source-equivalent additive motion crosses a collinear knee configuration without an instantaneous joint flip: {metrics:?}"
            );
        }
    }

    #[test]
    fn temporal_guard_rejects_a_global_flip_with_unchanged_valid_local_joint_geometry() {
        let local = BTreeMap::from([
            (
                "thigh_l".to_owned(),
                LocalBonePose {
                    rotation: Quat::IDENTITY,
                    scale: Vec3::ONE,
                    ..default()
                },
            ),
            (
                "calf_l".to_owned(),
                LocalBonePose {
                    translation: Vec3::new(0.25, -0.5, 0.0),
                    rotation: Quat::IDENTITY,
                    scale: Vec3::ONE,
                },
            ),
            (
                "foot_l".to_owned(),
                LocalBonePose {
                    translation: Vec3::new(-0.25, -0.5, 0.0),
                    rotation: Quat::IDENTITY,
                    scale: Vec3::ONE,
                },
            ),
        ]);
        let mut guard = TemporalSkeletonGuard::default();
        guard.observe(&local, &temporal_bones(0.25), Vec3::ZERO, 1.0 / 60.0);
        let metrics = guard
            .observe(&local, &temporal_bones(-0.25), Vec3::ZERO, 1.0 / 60.0)
            .metrics;
        assert!(
            metrics
                .iter()
                .any(|metric| metric.kind == "limb_plane_flip"),
            "A global limb flip cannot be excused by unchanged or inconsistent local joint data: {metrics:?}"
        );
    }

    #[test]
    fn temporal_guard_rejects_an_instant_limb_plane_flip() {
        let mut guard = TemporalSkeletonGuard::default();
        let local = temporal_local_pose(Quat::IDENTITY);
        assert!(
            guard
                .observe(&local, &temporal_bones(0.25), Vec3::ZERO, 1.0 / 60.0)
                .metrics
                .is_empty()
        );
        let observation = guard.observe(&local, &temporal_bones(-0.25), Vec3::ZERO, 1.0 / 60.0);
        assert!(
            observation
                .metrics
                .iter()
                .any(|value| value.kind == "limb_plane_flip" && value.subject == "left_leg")
        );
    }

    #[test]
    fn temporal_guard_rejects_an_instant_local_scale_spike() {
        let mut guard = TemporalSkeletonGuard::default();
        let bones = temporal_bones(0.25);
        assert!(
            guard
                .observe(
                    &temporal_local_pose(Quat::IDENTITY),
                    &bones,
                    Vec3::ZERO,
                    1.0 / 60.0,
                )
                .metrics
                .is_empty()
        );
        let mut scaled = temporal_local_pose(Quat::IDENTITY);
        scaled.get_mut("calf_l").unwrap().scale = Vec3::splat(2.0);
        let observation = guard.observe(&scaled, &bones, Vec3::ZERO, 1.0 / 60.0);
        assert!(
            observation
                .metrics
                .iter()
                .any(|value| value.kind == "local_scale_spike" && value.subject == "calf_l")
        );
    }

    #[test]
    fn temporal_guard_rejects_an_instant_local_translation_spike() {
        let mut guard = TemporalSkeletonGuard::default();
        let bones = temporal_bones(0.25);
        let baseline = temporal_local_pose(Quat::IDENTITY);
        assert!(
            guard
                .observe(&baseline, &bones, Vec3::ZERO, 1.0 / 60.0)
                .metrics
                .is_empty()
        );
        let mut displaced = baseline;
        displaced.get_mut("calf_l").unwrap().translation = Vec3::new(1.0, 0.0, 0.0);
        let observation = guard.observe(&displaced, &bones, Vec3::ZERO, 1.0 / 60.0);
        assert!(
            observation
                .metrics
                .iter()
                .any(|value| value.kind == "local_translation_spike" && value.subject == "calf_l")
        );
    }

    #[test]
    fn temporal_guard_rejects_an_instant_segment_and_position_spike() {
        let mut guard = TemporalSkeletonGuard::default();
        let local = temporal_local_pose(Quat::IDENTITY);
        assert!(
            guard
                .observe(&local, &temporal_bones(0.25), Vec3::ZERO, 1.0 / 60.0)
                .metrics
                .is_empty()
        );
        let observation = guard.observe(&local, &temporal_bones(1.5), Vec3::ZERO, 1.0 / 60.0);
        assert!(observation.metrics.iter().any(
            |value| value.kind == "segment_length_spike" && value.subject == "thigh_l->calf_l"
        ));
        assert!(
            observation
                .metrics
                .iter()
                .any(|value| value.kind == "global_position_speed" && value.subject == "calf_l")
        );
    }

    #[test]
    fn temporal_guard_ablation_without_local_history_misses_a_twist() {
        let mut guard = TemporalSkeletonGuard::default();
        let bones = temporal_bones(0.25);
        let baseline = temporal_local_pose(Quat::IDENTITY);
        let _ = guard.observe(&baseline, &bones, Vec3::ZERO, 1.0 / 60.0);
        guard.previous_local.clear();
        let observation = guard.observe(
            &temporal_local_pose(Quat::from_rotation_x(std::f32::consts::PI)),
            &bones,
            Vec3::ZERO,
            1.0 / 60.0,
        );
        assert!(
            !observation
                .metrics
                .iter()
                .any(|value| value.kind == "local_rotation_spike" && value.subject == "calf_l")
        );
    }

    #[test]
    fn temporal_guard_reset_rebases_only_the_first_new_pose() {
        let mut guard = TemporalSkeletonGuard::default();
        let bones = temporal_bones(0.25);
        let baseline = temporal_local_pose(Quat::IDENTITY);
        let _ = guard.observe(&baseline, &bones, Vec3::ZERO, 1.0 / 60.0);
        guard.reset();
        let turned = temporal_local_pose(Quat::from_rotation_x(std::f32::consts::PI));
        assert!(
            !guard
                .observe(&turned, &bones, Vec3::ZERO, 1.0 / 60.0)
                .metrics
                .iter()
                .any(|value| value.kind == "local_rotation_spike")
        );
        assert!(
            guard
                .observe(&baseline, &bones, Vec3::ZERO, 1.0 / 60.0)
                .metrics
                .iter()
                .any(|value| value.kind == "local_rotation_spike")
        );
    }

    #[test]
    fn source_pose_chain_reports_the_first_static_boundary() {
        let boundaries = vec![
            SourcePoseBoundaryTracker {
                label: "sequence_player".to_owned(),
                sample_count: 2,
                available_samples: 2,
                max_rotation_span_radians: 0.1,
                ..default()
            },
            SourcePoseBoundaryTracker {
                label: "cache_producer".to_owned(),
                sample_count: 2,
                available_samples: 2,
                ..default()
            },
            SourcePoseBoundaryTracker {
                label: "movement_fsm".to_owned(),
                sample_count: 2,
                available_samples: 2,
                max_rotation_span_radians: 0.1,
                ..default()
            },
        ];
        assert_eq!(
            first_source_pose_boundary_failure(&boundaries),
            Some("cache_producer: static".to_owned())
        );
    }

    #[test]
    fn source_pose_boundary_records_empty_and_populated_channel_counts() {
        let mut boundary = SourcePoseBoundaryTracker::default();
        record_source_pose_bone_count(&mut boundary, &Pose::default());
        let mut populated = Pose::default();
        populated.add_bone(BonePose::default(), default());
        record_source_pose_bone_count(&mut boundary, &populated);
        assert_eq!(boundary.minimum_bone_count, Some(0));
        assert_eq!(boundary.maximum_bone_count, 1);
    }

    #[test]
    fn source_pose_boundary_records_local_translation_extent() {
        let mut boundary = SourcePoseBoundaryTracker::default();
        let mut full = Pose::default();
        full.add_bone(
            BonePose {
                translation: Some(Vec3::new(0.0, 1.0, 0.0)),
                ..default()
            },
            default(),
        );
        let mut collapsed = Pose::default();
        collapsed.add_bone(
            BonePose {
                translation: Some(Vec3::new(0.0, 0.2, 0.0)),
                ..default()
            },
            default(),
        );
        record_source_pose_local_translation_extent(&mut boundary, &full, 1.0);
        record_source_pose_local_translation_extent(&mut boundary, &collapsed, 2.0);
        assert_eq!(boundary.minimum_max_local_translation_length, Some(0.2));
        assert_eq!(
            boundary.minimum_max_local_translation_elapsed_seconds,
            Some(2.0)
        );
        assert_eq!(boundary.maximum_max_local_translation_length, 1.0);
        assert_eq!(boundary.trace.len(), 2);
        assert_eq!(boundary.trace[1].elapsed_seconds, 2.0);
        assert_eq!(boundary.trace[1].maximum_local_translation_length, 0.2);
    }

    #[test]
    fn driven_performance_follows_playground_readiness() {
        use bevy::ecs::system::RunSystemOnce;

        let mut world = World::new();
        world.init_resource::<ManualPlaygroundStartupReady>();
        world.init_resource::<ManualIntegrationGateReport>();
        world.insert_resource(AnimationVisualValidationConfig {
            directory: PathBuf::new(),
            case: "ragdoll".to_owned(),
            capture_count: 0,
            capture_interval: Duration::from_secs(1),
            min_pose_samples: 1,
            warmup: Duration::from_secs(1),
            max_frame_p95_ms: 25.0,
            max_frame_ms: 75.0,
            max_eval_p95_ms: 4.0,
            max_physics_steps_per_render: 3.0,
        });
        world.insert_resource(AnimationVisualValidationState {
            started: Some(Instant::now() - Duration::from_secs(2)),
            ..default()
        });

        let sampled = |world: &mut World| {
            world.run_system_once(animation_cpu_frame_start).unwrap();
            world.run_system_once(animation_eval_start).unwrap();
            world.run_system_once(animation_eval_end).unwrap();
            world.resource::<AnimationVisualValidationState>().eval_ms.len()
        };
        assert_eq!(sampled(&mut world), 0);

        world.resource_mut::<ManualPlaygroundStartupReady>().passed = true;
        world.resource_mut::<AnimationVisualValidationState>().started = Some(Instant::now());
        assert_eq!(sampled(&mut world), 1);

        world.remove_resource::<ManualIntegrationGateReport>();
        assert_eq!(sampled(&mut world), 1);
    }

    #[test]
    fn physics_step_stage_is_recorded_after_warmup() {
        let config = AnimationVisualValidationConfig {
            directory: PathBuf::new(),
            case: "fixture".to_owned(),
            capture_count: 1,
            capture_interval: Duration::from_secs(1),
            min_pose_samples: 1,
            warmup: Duration::ZERO,
            max_frame_p95_ms: 1.0,
            max_frame_ms: 1.0,
            max_eval_p95_ms: 1.0,
            max_physics_steps_per_render: 1.0,
        };
        let visual = AnimationVisualValidationState {
            started: Some(Instant::now()),
            ..default()
        };
        let mut stages = AnimationVisualStageTiming::default();
        record_stage(
            &config,
            &visual,
            &mut stages,
            "physics_step",
            Some(Instant::now()),
        );
        assert_eq!(stages.samples["physics_step"].len(), 1);
    }

    #[test]
    fn zero_screenshot_readiness_still_requires_numeric_samples() {
        let mut config = AnimationVisualValidationConfig {
            directory: PathBuf::new(),
            case: "fixture".to_owned(),
            capture_count: 0,
            capture_interval: Duration::from_secs(1),
            min_pose_samples: 1,
            warmup: Duration::ZERO,
            max_frame_p95_ms: 1.0,
            max_frame_ms: 1.0,
            max_eval_p95_ms: 1.0,
            max_physics_steps_per_render: 1.0,
        };
        let mut state = AnimationVisualValidationState {
            started: Some(Instant::now()),
            frame_ms: vec![1.0],
            eval_ms: vec![1.0],
            required_motion_windows: BTreeSet::from(["fixture".to_owned()]),
            ..default()
        };
        state.pose_samples.push(snapshot(
            1.0,
            1.0,
            &BTreeMap::new(),
            Some("fixture"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            RootPoseActivity::default(),
        ));
        let mut stages = AnimationVisualStageTiming::default();
        assert!(!evidence_capture_complete(&config, &state, &stages));
        stages.physics_steps_per_render.push(1.0);
        assert!(!evidence_capture_complete(&config, &state, &stages));
        state.motion_windows.insert(
            "fixture".to_owned(),
            MotionWindowTracker {
                requires_non_reference_root_pose: true,
                sample_count: 1,
                ..default()
            },
        );
        assert!(!evidence_capture_complete(&config, &state, &stages));
        state
            .motion_windows
            .get_mut("fixture")
            .unwrap()
            .sample_count = 2;
        assert!(!evidence_capture_complete(&config, &state, &stages));
        state
            .motion_windows
            .get_mut("fixture")
            .unwrap()
            .root_pose_sample_count = 2;
        assert!(evidence_capture_complete(&config, &state, &stages));
        config.capture_count = 1;
        assert!(!evidence_capture_complete(&config, &state, &stages));
        config.capture_count = 0;
        state.pose_samples.clear();
        assert!(!evidence_capture_complete(&config, &state, &stages));
    }

    #[test]
    fn teleport_capture_excludes_competing_window_captures() {
        assert!(!scheduled_capture_allowed(false, true));
        assert!(!direction_capture_allowed(true, true));
        assert!(scheduled_capture_allowed(false, false));
        assert!(scheduled_capture_allowed(true, false));
        assert!(direction_capture_allowed(true, false));
    }

    #[test]
    fn automation_watchdog_stops_completed_scenarios_without_capture_progress() {
        assert!(!automation_watchdog_should_abort(
            true,
            false,
            true,
            AUTOMATION_PROGRESS_STALL,
        ));
        assert!(!automation_watchdog_should_abort(
            true,
            true,
            false,
            AUTOMATION_PROGRESS_STALL,
        ));
        assert!(!automation_watchdog_should_abort(
            false,
            false,
            false,
            AUTOMATION_PROGRESS_STALL,
        ));
        assert!(automation_watchdog_should_abort(
            true,
            false,
            false,
            AUTOMATION_PROGRESS_STALL,
        ));
    }

    #[test]
    fn fault_capture_preempts_competing_capture_requests() {
        assert!(!nonfault_capture_allowed(true, true));
        assert!(!nonfault_capture_allowed(true, false));
        assert!(nonfault_capture_allowed(false, true));
    }

    #[test]
    fn temporal_guard_rebases_at_declared_window_boundaries_only() {
        assert!(temporal_guard_should_rebase(
            None,
            "phase|case|stance",
            false,
            false,
            None,
            0,
        ));
        assert!(temporal_guard_should_rebase(
            Some("phase|case|stance"),
            "phase|next_case|stance",
            true,
            false,
            Some(0),
            0,
        ));
        assert!(temporal_guard_should_rebase(
            Some("phase|case|stance"),
            "phase|case|stance",
            false,
            true,
            Some(0),
            0,
        ));
        assert!(!temporal_guard_should_rebase(
            Some("phase|case|stance"),
            "phase|case|stance",
            true,
            true,
            Some(0),
            0,
        ));
        assert!(temporal_guard_should_rebase(
            Some("phase|case|stance"),
            "phase|case|stance",
            true,
            true,
            Some(0),
            1,
        ));
    }

    #[test]
    fn root_pose_activity_distinguishes_reference_and_animated_channels() {
        let reference = root_pose_activity_from_pose(&Pose::default());
        assert!(reference.output_present);
        assert_eq!(reference.bone_count, 0);
        assert_eq!(reference.active_channel_count, 0);
        assert!(!root_pose_has_animation_channels(reference));
        let mut animated = Pose::default();
        animated.add_bone(
            BonePose {
                rotation: Some(Quat::IDENTITY),
                ..default()
            },
            bevy_animation_graph::core::animation_clip::EntityPath::from_slashed_string(
                "root/pelvis".to_owned(),
            )
            .id(),
        );
        let activity = root_pose_activity_from_pose(&animated);
        assert!(activity.output_present);
        assert_eq!(activity.bone_count, 1);
        assert_eq!(activity.active_channel_count, 1);
        assert!(root_pose_has_animation_channels(activity));
    }

    #[test]
    fn root_pose_activity_counts_only_bones_referenced_by_the_pose() {
        let bone = bevy_animation_graph::core::animation_clip::EntityPath::from_slashed_string(
            "root/pelvis".to_owned(),
        )
        .id();
        let mut pose = Pose::default();
        pose.add_bone(
            BonePose {
                translation: Some(Vec3::ONE),
                ..default()
            },
            bone,
        );
        pose.add_bone(
            BonePose {
                rotation: Some(Quat::IDENTITY),
                ..default()
            },
            bone,
        );
        let activity = root_pose_activity_from_pose(&pose);
        assert_eq!(activity.bone_count, 1);
        assert_eq!(activity.active_channel_count, 1);
        let mut boundary = SourcePoseBoundaryTracker::default();
        record_source_pose_bone_count(&mut boundary, &pose);
        record_source_pose_local_translation_extent(&mut boundary, &pose, 1.0);
        assert_eq!(boundary.maximum_bone_count, 1);
        assert_eq!(boundary.trace[0].bone_count, 1);
        assert_eq!(boundary.trace[0].maximum_local_translation_length, 0.0);
        pose.paths.clear();
        assert!(!root_pose_has_animation_channels(
            root_pose_activity_from_pose(&pose)
        ));
    }

    #[test]
    fn sync_phase_distance_wraps_at_cycle_boundary() {
        assert!(wrapped_phase_distance(0.98, 0.02) < 0.05);
        assert!((wrapped_phase_distance(0.25, 0.5) - 0.25).abs() < 1e-6);
    }
}
#[test]
fn short_source_windows_are_sampled_on_each_active_frame() {
    let now = Instant::now();
    let next = now + Duration::from_millis(100);
    let mut evidence = ManualAnimationWindowEvidence {
        settled: true,
        capture_source_window: true,
        capture_every_frame: true,
        ..default()
    };
    for elapsed in [0, 16, 32] {
        assert!(pose_sample_due(
            now + Duration::from_millis(elapsed),
            next,
            Some(&evidence)
        ));
    }
    evidence.settled = false;
    assert!(!pose_sample_due(now, next, Some(&evidence)));
    assert!(!pose_sample_due(now, next, None));
    assert!(pose_sample_due(next, next, None));
}
