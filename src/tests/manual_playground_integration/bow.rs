use super::*;
use crate::input::AlsInputCommand;
use bevy_animation_graph::core::state_machine::low_level::LowLevelStateId;

const GRAPH: &str = "ALS/Character/AnimationInstances/Overlays/AB_Als_Bow/Overlay.animgraph.ron";
const SYNC: &str = "sync_groups.secondary_motion.phase";
pub(super) const LABELS: [&str; 10] = [
    "bow_relaxed",
    "bow_standing_aim",
    "bow_release",
    "bow_walking_aim",
    "bow_crouching_aim",
    "bow_standing_aim_return",
    "bow_aim_up",
    "bow_aim_down",
    "bow_sprint",
    "bow_in_air",
];

#[derive(Default)]
pub(super) struct Probe {
    pub(super) index: usize,
    pub(super) covered: [bool; LABELS.len()],
    seconds: [f32; LABELS.len()],
    canonical_seen: [bool; 4],
    pub(super) waited_seconds: f32,
    pub(super) sample: Sample,
    pub(super) material_seen: bool,
    pub(super) cleared: [bool; 2],
    pub(super) restored: [bool; 2],
    components: Option<[Entity; 2]>,
}

#[derive(Default, Clone, Serialize)]
pub(super) struct ObjectSample {
    valid: bool,
    draw_amount: Option<f32>,
    pose_time: Option<f32>,
    pose_error: Option<f32>,
    string_local: Option<[f32; 3]>,
    applied_slots: Option<u32>,
    player: Option<String>,
}

#[derive(Default, Serialize)]
pub(super) struct Sample {
    pub(super) object: ObjectSample,
    state: Option<String>,
    transition: Option<String>,
    elapsed: Option<f32>,
    weights: Vec<(String, f32)>,
    phase: Option<f32>,
    pitch: Option<f32>,
    gait: Option<f32>,
    in_air: Option<f32>,
    aiming: [Option<f32>; 2],
    fsm_arms: Vec<(String, [f32; 4])>,
    root_arms: Vec<(String, [f32; 4])>,
    frames: Vec<(f32, f32, f32)>,
    frame_errors: Vec<String>,
}

pub(super) fn observe_object(
    probe: &mut Probe,
    state: Option<&AlsOverlayObjectState>,
    context: Option<&AlsRootAnimationContext>,
    motion: &MotionProbe,
    action: Option<&AlsNotifyLocomotionAction>,
    mantle: Option<&AlsMantleState>,
    phase: IntegrationPhase,
) {
    let mut sample = ObjectSample::default();
    let Some(state) = state else {
        probe.sample.object = sample;
        return;
    };
    let components = *probe.components.get_or_insert(state.components);
    let persistent = components == state.components
        && components
            .iter()
            .all(|entity| motion.object_transforms.contains(*entity));
    let ragdoll = action == Some(&AlsNotifyLocomotionAction::Ragdolling);
    let high_mantle = mantle.is_some_and(|mantle| mantle.active && mantle.kind.is_high());
    if (ragdoll || high_mantle) && state.prop.is_none() && !state.visible && persistent {
        probe.cleared[usize::from(ragdoll)] = true;
    }
    let sample_loaded = |sample: &mut ObjectSample| -> Option<()> {
        let (object, applied) = motion.overlay_objects.get(state.prop?).ok()?;
        let instance = motion.object_scenes.get(object.content).ok()?;
        let player = motion.object_players.get(instance.player_entity()).ok()?;
        sample.player = Some(format!("{:?}", instance.player_entity()));
        sample.draw_amount = graph_global_f32(player, "draw_amount");
        let pose = player.get_default_output_pose()?;
        sample.pose_time = Some(pose.timestamp);
        let clip = motion
            .asset_server
            .get_handle::<bevy_animation_graph::core::animation_clip::GraphClip>(
                "ALS/OverlayObjects/Bow/A_Als_Bow_Draw.anim.ron",
            )?;
        let clip = motion.clips.get(&clip)?;
        let amount = context.map_or(0.0, |context| context.f32("PoseAiming").unwrap_or(0.0));
        let expected = bevy_als::extensions::source_sequence_player::sample_source_clip_pose(
            clip,
            amount.clamp(0.0, clip.duration()),
            &motion.skeletons,
        );
        sample.pose_error = default_overlay_pose_error(pose, &expected);
        let skeleton = motion.skeletons.get(&pose.skeleton)?;
        let string = skeleton.iter_paths().find(|path| {
            path.parts
                .last()
                .is_some_and(|name| name.as_str() == "string_mid")
        })?;
        let actual = motion
            .object_transforms
            .get(player.bone_entity(string.id())?)
            .ok()?
            .translation;
        sample.string_local = Some(actual.to_array());
        let applied = applied?;
        sample.applied_slots = Some(applied.slots);
        sample.valid = persistent
            && player.get_error().is_none()
            && object.overlay == OverlayState::Bow
            && applied.slots == 3
            && applied.source_slot_pattern
            && sample
                .draw_amount
                .is_some_and(|value| (value - amount).abs() < 1e-5)
            && (pose.timestamp - amount.clamp(0.0, clip.duration())).abs() < 1e-5
            && sample.pose_error.is_some_and(|value| value < 2e-5)
            && actual.abs_diff_eq(expected.get_bone(string.id())?.translation?, 1e-5);
        Some(())
    };
    sample_loaded(&mut sample);
    probe.material_seen |= sample.applied_slots == Some(3);
    if !ragdoll && !high_mantle && sample.valid {
        if phase == IntegrationPhase::Mantle {
            probe.restored[0] |= probe.cleared[0];
        }
        if phase == IntegrationPhase::Ragdoll {
            probe.restored[1] |= probe.cleared[1];
        }
    }
    probe.sample.object = sample;
}

pub(super) fn command(index: usize) -> AlsInputCommand {
    AlsInputCommand {
        aim_held: matches!(index, 1 | 3..=7),
        sprint_held: index == 8,
        move_dir: if matches!(index, 3 | 8) {
            Vec2::Y
        } else {
            Vec2::ZERO
        },
        ..default()
    }
}

pub(super) fn pitch(index: usize, elapsed: f32) -> f32 {
    match index {
        6 => (elapsed * 60.0).min(60.0).to_radians(),
        7 => (60.0 - elapsed * 120.0).max(-60.0).to_radians(),
        8 => (-60.0 + elapsed * 120.0).min(0.0).to_radians(),
        _ => 0.0,
    }
}

fn expected_state(index: usize) -> &'static str {
    if matches!(index, 0 | 2 | 8 | 9) {
        "Relaxed"
    } else {
        "Aiming"
    }
}

fn settled(index: usize, stance: Stance, mode: RotationMode, speed: f32, sample: &Sample) -> bool {
    sample.object.valid
        && sample.state.as_deref() == Some(expected_state(index))
        && sample.transition.is_none()
        && stance
            == if index == 4 {
                Stance::Crouching
            } else {
                Stance::Standing
            }
        && (mode == RotationMode::Aiming) == command(index).aim_held
        && if matches!(index, 3 | 8) {
            speed > 0.5
        } else {
            index == 9 || speed < 0.1
        }
        && (index != 6
            || sample
                .pitch
                .is_some_and(|pitch| (pitch - 1.0 / 6.0).abs() < 1e-4))
        && (index != 7
            || sample
                .pitch
                .is_some_and(|pitch| (pitch - 5.0 / 6.0).abs() < 1e-4))
        && (index != 8 || sample.gait.is_some_and(|gait| gait > 2.99))
        && (index != 9 || sample.in_air.is_some_and(|value| value > 0.99))
        && sample.aiming.iter().all(|value| {
            value.is_some_and(|value| (value - command(index).aim_held as u8 as f32).abs() < 1e-4)
        })
        && sample.fsm_arms.len() == 4
        && sample.root_arms.len() == 4
        && sample.frame_errors.is_empty()
}

fn arms(pose: &Pose, skeletons: &Assets<Skeleton>) -> Vec<(String, [f32; 4])> {
    let Some(skeleton) = skeletons.get(&pose.skeleton) else {
        return Vec::new();
    };
    skeleton
        .iter_bones()
        .filter_map(|id| {
            let path = skeleton.id_to_path(id)?;
            let name = path.parts.last()?.as_str();
            ["upperarm_l", "lowerarm_l", "upperarm_r", "lowerarm_r"]
                .contains(&name)
                .then(|| Some((name.to_owned(), pose.get_bone(id)?.rotation?.to_array())))
                .flatten()
        })
        .collect()
}

fn sample(
    player: &AnimationGraphPlayer,
    probe: &MotionProbe,
    root_context: Option<&AlsRootAnimationContext>,
) -> Sample {
    let mut sample = Sample::default();
    sample.pitch = graph_global_f32(player, "view_state.pitch_amount");
    sample.gait = root_context.and_then(|context| context.f32("PoseGait"));
    sample.in_air = root_context.and_then(|context| context.f32("PoseInAir"));
    sample.aiming[1] = root_context.map(|context| context.f32("PoseAiming").unwrap_or(0.0));
    if let Some(DataValue::Pose(pose)) = player.get_outputs().get("pose") {
        sample.root_arms = arms(pose, &probe.skeletons);
    }
    let Some(arena) = player.get_context_arena() else {
        return sample;
    };
    sample.phase =
        bevy_als::extensions::sync_group_phase::source_sync_phase(arena, &probe.graphs, SYNC);
    for id in arena.iter_context_ids() {
        let Some(context) = arena.get_context(id) else {
            continue;
        };
        let Some(path) = probe.asset_server.get_path(context.get_graph_id()) else {
            continue;
        };
        if !path
            .path()
            .starts_with("ALS/Character/AnimationInstances/Overlays/AB_Als_Bow")
        {
            continue;
        }
        let Some(graph) = probe.graphs.get(context.get_graph_id()) else {
            continue;
        };
        for node in graph.nodes.values() {
            if !context.node_caches.is_updated(node.id, StateKey::Default) {
                continue;
            }
            let Some(sequence) =
                node.try_inner_downcast_ref::<bevy_als::extensions::SourceSequenceEvaluatorNode>()
            else {
                continue;
            };
            if !sequence.use_explicit_frame {
                continue;
            }
            let Some(clip) = probe.clips.get(&sequence.clip) else {
                continue;
            };
            let Some(DataValue::Pose(actual)) =
                context
                    .node_caches
                    .get_output_data(node.id, StateKey::Default, "Pose".into())
            else {
                continue;
            };
            let time = ((sequence.explicit_value as f64 / sequence.frames_per_second) as f32)
                .clamp(0.0, clip.duration());
            sample
                .frames
                .push((sequence.explicit_value, actual.timestamp, time));
            let at = if sequence.step_interpolation && time > 0.0 {
                time.next_up().min(clip.duration())
            } else {
                time
            };
            let expected = bevy_als::extensions::source_sequence_player::sample_source_clip_pose(
                clip,
                at,
                &probe.skeletons,
            );
            let pose_error = default_overlay_pose_error(&actual, &expected);
            if actual.timestamp != time || pose_error.is_none_or(|error| error > 2e-5) {
                sample.frame_errors.push(format!(
                    "frame {} sampled {} instead of {} with pose error {pose_error:?}",
                    sequence.explicit_value, actual.timestamp, time
                ));
            }
        }
        if path.path() != std::path::Path::new(GRAPH) {
            continue;
        }
        let Some(node) = graph
            .nodes
            .values()
            .find(|node| node.name == "Bow States\nState Machine")
        else {
            continue;
        };
        if !context.node_caches.is_updated(node.id, StateKey::Default) {
            continue;
        }
        let Some(fsm) = node.try_inner_downcast_ref::<bevy_als::extensions::InterruptibleFsmNode>()
        else {
            continue;
        };
        let Some(machine) = probe.machines.get(&fsm.fsm) else {
            continue;
        };
        let Ok(runtime) = context
            .node_states
            .get::<bevy_als::extensions::InterruptibleFsmState>(node.id, StateKey::Default)
        else {
            continue;
        };
        sample.state = match &runtime.state {
            LowLevelStateId::HlState(id) => machine.states.get(id).map(|state| state.label.clone()),
            LowLevelStateId::HlTransition(_) => None,
        };
        sample.transition = runtime.active_direct_transition_id();
        sample.elapsed = Some(runtime.current_state_elapsed_time());
        sample.aiming[0] = Some(
            runtime
                .context
                .curves
                .get(&bevy_als::extensions::SourceCurveName::from("PoseAiming"))
                .copied()
                .unwrap_or(0.0),
        );
        sample.weights = machine
            .states
            .values()
            .map(|state| {
                (
                    state.label.clone(),
                    runtime.recorded_state_weight(&state.id.uuid().to_string()),
                )
            })
            .collect();
        if let Some(pose) = runtime.last_output_pose() {
            sample.fsm_arms = arms(pose, &probe.skeletons);
        }
    }
    sample
}

pub(super) fn observe(
    probe: &mut Probe,
    player: &AnimationGraphPlayer,
    motion: &MotionProbe,
    context: Option<&AlsRootAnimationContext>,
    stance: Stance,
    mode: RotationMode,
    speed: f32,
    window: &mut ManualAnimationWindowEvidence,
) {
    let object = probe.sample.object.clone();
    probe.sample = sample(player, motion, context);
    probe.sample.object = object;
    let index = probe.index;
    if !settled(index, stance, mode, speed, &probe.sample) {
        return;
    }
    let canonical = matches!(index, 0 | 1 | 6 | 7);
    let state = if expected_state(index) == "Aiming" {
        1
    } else {
        0
    };
    *window = ManualAnimationWindowEvidence {
        label: Some(LABELS[index]),
        index: Some(index),
        settled: true,
        capture_source_window: true,
        capture_every_frame: true,
        requires_non_reference_root_pose: true,
        requires_source_pose_chain: true,
        source_node: Some("Bow States\nState Machine"),
        source_state: Some(expected_state(index)),
        expected_runtime_state: Some(state),
        observed_runtime_state: Some(state),
        minimum_observation_seconds: if index == 9 { 0.15 } else { 0.3 },
        source_sync_group: Some("Secondary Motion"),
        sync_phase_global: Some(SYNC),
        canonical_sync_phase: canonical.then_some(0.35),
        maximum_sync_phase_distance: 0.04,
        direction_signature_group: if index < 2 {
            Some("bow_standing_pose")
        } else if canonical {
            Some("bow_aim_pitch")
        } else {
            None
        },
        required_direction_signature_members: if canonical { 2 } else { 0 },
        minimum_direction_signature_rotation_radians: if canonical { 0.01 } else { 0.0 },
        expected_direction: Some(LABELS[index]),
        observed_direction: Some(LABELS[index]),
        source_pose_chain: ["Bow States\nState Machine", "Output Pose\nOverlay"]
            .map(|node| ManualAnimationPoseBoundary {
                label: node,
                graph: GRAPH,
                node,
                output: "Pose",
                minimum_translation_span: 0.0,
                minimum_rotation_span_radians: 0.0,
            })
            .to_vec(),
        ..default()
    };
    probe.seconds[index] += graph_global_f32(player, "animation.delta_seconds").unwrap_or(0.0);
    let canonical_index = if index < 2 {
        index
    } else {
        index.saturating_sub(4)
    };
    if canonical {
        probe.canonical_seen[canonical_index] |= probe
            .sample
            .phase
            .is_some_and(|phase| (phase - 0.35).abs() <= 0.04);
    }
    probe.covered[index] |= probe.seconds[index] >= if index == 9 { 0.2 } else { 0.5 }
        && (!canonical || probe.canonical_seen[canonical_index]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bow_pitch_fixture_is_continuous_between_windows() {
        for index in 6..=9 {
            assert!(
                (pitch(index, 0.0) - pitch(index - 1, 4.0)).abs() < 1e-6,
                "Pitch fixture jumped at window {index}"
            );
            assert!((pitch(index, 0.01) - pitch(index, 0.0)).abs() <= 1.21_f32.to_radians());
        }
        assert!((pitch(6, 4.0) - 60.0_f32.to_radians()).abs() < 1e-6);
        assert!((pitch(7, 4.0) + 60.0_f32.to_radians()).abs() < 1e-6);
        assert_eq!(pitch(8, 4.0), 0.0);
    }

    #[test]
    fn bow_fixture_requires_actual_state_curve_and_normal_held_input() {
        assert!(!command(0).aim_held && command(1).aim_held && !command(2).aim_held);
        assert_eq!(command(3).move_dir, Vec2::Y);
        assert_eq!(command(4).move_dir, Vec2::ZERO);
        let mut sample = Sample {
            object: ObjectSample {
                valid: true,
                ..default()
            },
            state: Some("Aiming".into()),
            aiming: [Some(1.0); 2],
            fsm_arms: vec![(String::new(), [0.0; 4]); 4],
            root_arms: vec![(String::new(), [0.0; 4]); 4],
            ..default()
        };
        assert!(settled(
            1,
            Stance::Standing,
            RotationMode::Aiming,
            0.0,
            &sample
        ));
        sample.object.valid = false;
        assert!(!settled(
            1,
            Stance::Standing,
            RotationMode::Aiming,
            0.0,
            &sample
        ));
        sample.object.valid = true;
        sample.pitch = Some(1.0 / 6.0);
        assert!(settled(
            6,
            Stance::Standing,
            RotationMode::Aiming,
            0.0,
            &sample
        ));
        assert!(!settled(
            7,
            Stance::Standing,
            RotationMode::Aiming,
            0.0,
            &sample
        ));
        sample.pitch = Some(5.0 / 6.0);
        assert!(settled(
            7,
            Stance::Standing,
            RotationMode::Aiming,
            0.0,
            &sample
        ));
        let mut moving = Sample {
            object: ObjectSample {
                valid: true,
                ..default()
            },
            state: Some("Relaxed".into()),
            aiming: [Some(0.0); 2],
            gait: Some(3.0),
            in_air: Some(1.0),
            fsm_arms: vec![(String::new(), [0.0; 4]); 4],
            root_arms: vec![(String::new(), [0.0; 4]); 4],
            ..default()
        };
        assert!(command(8).sprint_held && !command(8).aim_held);
        assert!(settled(
            8,
            Stance::Standing,
            RotationMode::ViewDirection,
            6.0,
            &moving
        ));
        moving.gait = Some(2.0);
        assert!(!settled(
            8,
            Stance::Standing,
            RotationMode::ViewDirection,
            6.0,
            &moving
        ));
        assert!(settled(
            9,
            Stance::Standing,
            RotationMode::ViewDirection,
            0.0,
            &moving
        ));
        moving.in_air = Some(0.0);
        assert!(!settled(
            9,
            Stance::Standing,
            RotationMode::ViewDirection,
            0.0,
            &moving
        ));
        sample.state = Some("Ready".into());
        assert!(!settled(
            1,
            Stance::Standing,
            RotationMode::Aiming,
            0.0,
            &sample
        ));
        sample.state = Some("Aiming".into());
        sample.aiming[1] = Some(0.0);
        assert!(!settled(
            1,
            Stance::Standing,
            RotationMode::Aiming,
            0.0,
            &sample
        ));
        sample.aiming[1] = Some(1.0);
        sample
            .frame_errors
            .push("previous step key was sampled".into());
        assert!(!settled(
            1,
            Stance::Standing,
            RotationMode::Aiming,
            0.0,
            &sample
        ));
        assert!(!settled(
            4,
            Stance::Standing,
            RotationMode::Aiming,
            0.0,
            &sample
        ));
        assert!(!default_overlay_should_timeout(false, 11.9));
        assert!(default_overlay_should_timeout(false, 12.0));
        assert!(!default_overlay_should_timeout(true, 12.0));
    }
}
