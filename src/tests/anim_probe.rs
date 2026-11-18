use bevy_als::als::character::{AlsCharacter, AlsMovementParamsHandle};
use bevy_als::als::gait::{AlsDesiredGait, AlsMaxAllowedGait};
use bevy_als::als::motion::{AlsCrouchTransaction, AlsViewYaw};
use bevy_als::als::movement_direction::AlsMovementDirectionState;
use bevy_als::als::params::AlsMovementParams;
use bevy_als::als::state::{Gait, LocomotionState, RotationMode, Stance};
use bevy::prelude::*;
use bevy_animation_graph::core::animated_scene::AnimatedSceneInstance;
use bevy_animation_graph::core::animation_graph_player::{AnimationGraphPlayer, AnimationSource};
use bevy_animation_graph::core::edge_data::DataValue;

use bevy_als::actual_motion::AlsActualMotion;
use bevy_als::character_assembly::{AlsCharacterAssembly, AlsCharacterAvatar};
use bevy_als::grounded_layer::AlsGroundedGraphTelemetry;
use bevy_als::standing_layer::AlsStandingGraphTelemetry;

#[derive(Resource, Clone, Copy)]
pub struct AnimProbeConfig {
    pub interval_secs: f32,
    pub facing_mismatch_warn_deg: f32,
}

impl Default for AnimProbeConfig {
    fn default() -> Self {
        Self {
            interval_secs: 0.25,
            facing_mismatch_warn_deg: 90.0,
        }
    }
}

pub struct AnimProbePlugin;

impl Plugin for AnimProbePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AnimProbeConfig>()
            .init_resource::<AnimProbeState>()
            .add_systems(Update, anim_probe_logger);
    }
}

#[derive(Resource, Default)]
struct AnimProbeState {
    last_log: f32,
    last_error_text: Option<String>,
    aposa_warned: bool,
    mismatch_warned: bool,
}

pub fn yaw_deg(v: Vec3) -> f32 {
    bevy_als::als::orientation::world_forward_to_yaw(v).to_degrees()
}

pub fn angle_err(a: f32, b: f32) -> f32 {
    let d = (a - b).abs() % 360.0;
    if d > 180.0 { 360.0 - d } else { d }
}

fn facing_tracks_velocity(rotation_mode: RotationMode, gait: Gait) -> bool {
    rotation_mode == RotationMode::VelocityDirection
        || (rotation_mode == RotationMode::ViewDirection && gait == Gait::Sprinting)
}

fn standing_movement_state_label(value: Option<u8>) -> &'static str {
    match value {
        Some(0) => "Forward",
        Some(1) => "Backward",
        Some(2) => "RightForward",
        Some(3) => "RightBackward",
        Some(4) => "LeftForward",
        Some(5) => "LeftBackward",
        _ => "Inactive",
    }
}

fn movement_detail_label(value: Option<u8>) -> &'static str {
    match value {
        Some(0) => "Forward",
        Some(1) => "Backward",
        Some(2) => "Left",
        Some(3) => "Right",
        _ => "Inactive",
    }
}

fn grounded_state_label(value: Option<u8>) -> &'static str {
    match value {
        Some(0) => "EntryConduit",
        Some(1) => "Standing",
        Some(2) => "Crouching",
        Some(3) => "StandingToCrouching",
        Some(4) => "CrouchingToStanding",
        Some(5) => "Roll",
        _ => "Inactive",
    }
}

type ProbeCharacters = (
    &'static AlsCharacterAssembly,
    &'static Transform,
    &'static AlsActualMotion,
    &'static Stance,
    &'static AlsDesiredGait,
    &'static AlsMaxAllowedGait,
    &'static Gait,
    &'static LocomotionState,
    &'static RotationMode,
    &'static AlsViewYaw,
    Option<&'static AlsMovementParamsHandle>,
    Option<&'static AlsStandingGraphTelemetry>,
    Option<&'static AlsCrouchTransaction>,
    Option<&'static AlsGroundedGraphTelemetry>,
);

type ProbeAvatars = (
    &'static AnimatedSceneInstance,
    Option<&'static AlsMovementDirectionState>,
);

fn read_f32(player: &AnimationGraphPlayer, pin: &str) -> Option<f32> {
    match player.get_input_data(pin.to_string()) {
        Some(DataValue::F32(v)) => Some(v),
        _ => None,
    }
}

fn anim_probe_logger(
    time: Res<Time>,
    config: Res<AnimProbeConfig>,
    mut state: ResMut<AnimProbeState>,
    characters: Query<ProbeCharacters, With<AlsCharacter>>,
    avatars: Query<ProbeAvatars, With<AlsCharacterAvatar>>,
    params_assets: Res<Assets<AlsMovementParams>>,
    players: Query<&AnimationGraphPlayer>,
) {
    let t = time.elapsed_secs();
    let Ok((
        assembly,
        tf,
        motion,
        stance,
        desired_gait,
        allowed_gait,
        gait,
        loco,
        rot,
        view_yaw,
        params_h,
        standing_telemetry,
        crouch_transaction,
        grounded_telemetry,
    )) = characters.single()
    else {
        return;
    };
    let Ok((instance, dir_state)) = avatars.get(assembly.avatar) else {
        return;
    };
    let Ok(player) = players.get(instance.player_entity()) else {
        return;
    };

    let capsule_h = params_h
        .and_then(|h| params_assets.get(&h.0))
        .map(|p| match *stance {
            Stance::Crouching => p.crouch.crouched_capsule_half_height,
            _ => p.crouch.standing_capsule_half_height,
        });
    let hspeed = motion.horizontal_speed;
    let face = yaw_deg(*tf.forward());
    let velyaw = if hspeed > 1e-3 {
        yaw_deg(motion.world_velocity)
    } else {
        f32::NAN
    };
    let dir_angle = dir_state.map(|s| s.angle).unwrap_or(f32::NAN);
    let dir_name = dir_state
        .map(|s| format!("{:?}", s.direction))
        .unwrap_or_else(|| "None".into());
    let fsm_state = standing_movement_state_label(
        standing_telemetry.and_then(|telemetry| telemetry.actual_movement_state),
    );
    let fsm_detail = movement_detail_label(
        standing_telemetry.and_then(|telemetry| telemetry.dominant_detail_index),
    );
    let fsm_transition_active = standing_telemetry
        .and_then(|telemetry| telemetry.actual_movement_transition_active)
        .map(|value| value.to_string())
        .unwrap_or_else(|| "?".into());
    let fsm_pose_ready = standing_telemetry
        .and_then(|telemetry| telemetry.actual_movement_pose_ready)
        .map(|value| value.to_string())
        .unwrap_or_else(|| "?".into());
    let dominant_weight = standing_telemetry
        .and_then(|telemetry| telemetry.dominant_detail_weight)
        .map(|value| format!("{value:.2}"))
        .unwrap_or_else(|| "?".into());
    let crouch_transaction_text = crouch_transaction
        .map(|transaction| {
            format!(
                "desired={:?},actual={:?},phase={:?},retries={},controller={},half={:.2},radius={:.2},blocked={},forced={}",
                transaction.desired_stance,
                transaction.actual_stance,
                transaction.phase,
                transaction.blocked_retry_count,
                transaction.controller_crouch_active,
                transaction.collider_half_height,
                transaction.collider_radius,
                transaction.headroom_blocked,
                transaction.forced_standing,
            )
        })
        .unwrap_or_else(|| "?".into());
    let grounded_state =
        grounded_state_label(grounded_telemetry.and_then(|telemetry| telemetry.actual_state));
    let grounded_transition_active = grounded_telemetry
        .and_then(|telemetry| telemetry.transition_active)
        .map(|value| value.to_string())
        .unwrap_or_else(|| "?".into());
    let grounded_pose_ready = grounded_telemetry
        .and_then(|telemetry| telemetry.pose_ready)
        .map(|value| value.to_string())
        .unwrap_or_else(|| "?".into());

    let (src_name, is_graph) = match player.get_animation_source() {
        AnimationSource::Graph(_) => ("Graph", true),
        AnimationSource::Pose(_) => ("Pose(APOSE)", false),
        AnimationSource::None => ("None(APOSE)", false),
    };
    let err_text = player
        .get_error()
        .map(|e| format!("{e:?}"))
        .unwrap_or_else(|| "none".into());

    let fmt_pin = |name: &str| {
        read_f32(player, name)
            .map(|f| format!("{f:.2}"))
            .unwrap_or_else(|| "?".into())
    };

    if t - state.last_log >= config.interval_secs {
        state.last_log = t;
        info!(
            "[PROBE] t={t:.2} pos=({:.2},{:.2},{:.2}) cap_h={} stance={stance:?} crouch_tx=({crouch_transaction_text}) gait=(desired={:?},allowed={:?},actual={gait:?}) loco={loco:?} rot={rot:?} face={face:.0} view={:.0} velyaw={velyaw:.0} hspd={hspeed:.2} world_v=({:.2},{:.2},{:.2}) local_v=({:.2},{:.2}) local_ang={:.1} blend=({:.2},{:.2},{:.2},{:.2}) dir={dir_name} dir_ang={dir_angle:.0} grounded_fsm={grounded_state} grounded_transition_active={grounded_transition_active} grounded_pose_ready={grounded_pose_ready} movement_fsm={fsm_state} transition_active={fsm_transition_active} pose_ready={fsm_pose_ready} dominant_branch={fsm_detail} dominant_weight={dominant_weight} src={src_name} graph_err={err_text} pins(speed={},rate={},dir_ang={},stance_w={},air_w={},land_w={})",
            tf.translation.x,
            tf.translation.y,
            tf.translation.z,
            capsule_h
                .map(|h| format!("{h:.2}"))
                .unwrap_or_else(|| "?".into()),
            desired_gait.0,
            allowed_gait.0,
            view_yaw.0,
            motion.world_velocity.x,
            motion.world_velocity.y,
            motion.world_velocity.z,
            motion.local_velocity.x,
            motion.local_velocity.y,
            motion.signed_direction_radians.to_degrees(),
            motion.velocity_blend.forward,
            motion.velocity_blend.backward,
            motion.velocity_blend.left,
            motion.velocity_blend.right,
            fmt_pin("speed"),
            fmt_pin("play_rate"),
            fmt_pin("direction_angle"),
            fmt_pin("stance_weight"),
            fmt_pin("air_weight"),
            fmt_pin("land_weight"),
        );
    }

    if !is_graph && !state.aposa_warned {
        state.aposa_warned = true;
        warn!("[PROBE][APOSE_RISK] animation source is {src_name}; avatar will show bind pose");
    }
    if err_text != "none" && state.last_error_text.as_deref() != Some(err_text.as_str()) {
        state.last_error_text = Some(err_text.clone());
        warn!("[PROBE][GRAPH_ERROR] {err_text}");
    }
    if facing_tracks_velocity(*rot, *gait)
        && hspeed > 1.0
        && angle_err(face, velyaw) > config.facing_mismatch_warn_deg
    {
        if !state.mismatch_warned {
            state.mismatch_warned = true;
            warn!(
                "[PROBE][FACING_MISMATCH] face={face:.0} velyaw={velyaw:.0} err={:.0}deg while hspd={hspeed:.2}",
                angle_err(face, velyaw)
            );
        }
    } else if hspeed <= 1.0 || !facing_tracks_velocity(*rot, *gait) {
        state.mismatch_warned = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facing_probe_only_tracks_source_velocity_facing_modes() {
        for (direction, yaw) in [
            (Vec3::NEG_Z, 0.0),
            (Vec3::X, -90.0),
            (Vec3::Z, 180.0),
            (Vec3::NEG_X, 90.0),
        ] {
            assert!(angle_err(yaw_deg(direction), yaw) < 1e-4);
        }
        assert!(facing_tracks_velocity(
            RotationMode::VelocityDirection,
            Gait::Running,
        ));
        assert!(facing_tracks_velocity(
            RotationMode::ViewDirection,
            Gait::Sprinting,
        ));
        assert!(!facing_tracks_velocity(
            RotationMode::ViewDirection,
            Gait::Running,
        ));
        assert!(!facing_tracks_velocity(
            RotationMode::Aiming,
            Gait::Sprinting
        ));
    }
}
