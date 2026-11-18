static SOURCE_LIBRARY: std::sync::LazyLock<bevy_als::overlay_object::OverlayLibrary> =
    std::sync::LazyLock::new(|| {
        serde_json::from_str(include_str!(
            "../../../assets/ALS/Character/overlay_actor.runtime.json"
        ))
        .unwrap()
    });

use super::*;
use crate::input::AlsInputCommand;
use bevy_animation_graph::core::state_machine::low_level::LowLevelStateId;
use std::collections::BTreeMap;

pub(super) const MODES: [OverlayState; 11] = [
    OverlayState::Masculine,
    OverlayState::Feminine,
    OverlayState::Injured,
    OverlayState::HandsTied,
    OverlayState::Rifle,
    OverlayState::PistolOneHanded,
    OverlayState::PistolTwoHanded,
    OverlayState::Torch,
    OverlayState::Binoculars,
    OverlayState::Box,
    OverlayState::Barrel,
];
macro_rules! labels {
    ($name:literal) => {
        [
            concat!($name, "_idle"),
            concat!($name, "_walk"),
            concat!($name, "_crouch"),
            concat!($name, "_aim_up"),
            concat!($name, "_crouch_aim_down"),
            concat!($name, "_release"),
            concat!($name, "_sprint"),
            concat!($name, "_stop"),
            concat!($name, "_jump"),
            concat!($name, "_socket"),
        ]
    };
}
pub(super) const LABELS: [[&str; 10]; 11] = [
    labels!("masculine"),
    labels!("feminine"),
    labels!("injured"),
    labels!("hands_tied"),
    labels!("rifle"),
    labels!("pistol_one"),
    labels!("pistol_two"),
    labels!("torch"),
    labels!("binoculars"),
    labels!("box"),
    labels!("barrel"),
];

#[derive(Default)]
pub(super) struct Probe {
    pub(super) mode: usize,
    pub(super) stage: usize,
    pub(super) covered: [[bool; 10]; 11],
    pub(super) fsm_states: [[bool; 3]; 3],
    pub(super) sample: Sample,
    pub(super) waited: f32,
    seconds: f32,
    parent: Option<u64>,
}

#[derive(Default, Serialize)]
pub(super) struct Sample {
    graph: Option<String>,
    parent: Option<u64>,
    state: Option<String>,
    transition: Option<String>,
    evaluators: usize,
    pitch: Option<f32>,
    curves: BTreeMap<String, f32>,
    object: bool,
    material_slots: Option<u32>,
    materials: Vec<[f32; 4]>,
    errors: Vec<String>,
}

pub(super) fn command(stage: usize) -> AlsInputCommand {
    AlsInputCommand {
        move_dir: if matches!(stage, 1 | 6) {
            Vec2::Y
        } else {
            Vec2::ZERO
        },
        aim_held: matches!(stage, 3 | 4),
        sprint_held: stage == 6,
        ..default()
    }
}

pub(super) fn pitch(stage: usize, elapsed: f32) -> f32 {
    match stage {
        3 => (elapsed * 60.0).min(60.0),
        4 => (60.0 - elapsed * 120.0).max(-60.0),
        5 => (-60.0 + elapsed * 120.0).min(0.0),
        _ => 0.0,
    }
    .to_radians()
}

fn settled(
    stage: usize,
    stance: Stance,
    rotation: RotationMode,
    locomotion: LocomotionState,
    speed: f32,
    sample: &Sample,
    gun: bool,
    context: Option<&AlsRootAnimationContext>,
) -> bool {
    sample.graph.is_some()
        && sample.parent.is_some()
        && sample.evaluators > 0
        && sample.object
        && sample.errors.is_empty()
        && stance
            == if matches!(stage, 2 | 4) {
                Stance::Crouching
            } else {
                Stance::Standing
            }
        && (rotation == RotationMode::Aiming) == command(stage).aim_held
        && (if matches!(stage, 1 | 6) {
            speed > 0.5
        } else {
            stage == 8 || speed < 0.1
        })
        && (locomotion
            == if stage == 8 {
                LocomotionState::Air
            } else {
                LocomotionState::Grounded
            })
        && (stage != 6
            || context
                .and_then(|c| c.f32("PoseGait"))
                .is_some_and(|value| value > 2.99))
        && (stage != 3
            || sample
                .pitch
                .is_some_and(|value| (value - 1.0 / 6.0).abs() < 1e-4))
        && (stage != 4
            || sample
                .pitch
                .is_some_and(|value| (value - 5.0 / 6.0).abs() < 1e-4))
        && (stage != 5 || sample.pitch.is_some_and(|value| (value - 0.5).abs() < 1e-4))
        && (!gun
            || (sample.transition.is_none()
                && sample.state.as_deref()
                    == Some(if command(stage).aim_held {
                        "Aiming"
                    } else {
                        "Relaxed"
                    })))
}

fn window(mode: usize, stage: usize, ready: bool) -> ManualAnimationWindowEvidence {
    if !ready && stage != 0 {
        return default();
    }
    let label = LABELS[mode][stage];
    ManualAnimationWindowEvidence {
        label: Some(label),
        index: Some(mode * LABELS[0].len() + stage),
        settled: ready,
        capture_source_window: ready,
        capture_every_frame: true,
        requires_non_reference_root_pose: true,
        requires_source_pose_chain: true,
        expected_direction: Some(label),
        observed_direction: ready.then_some(label),
        source_node: Some("Output Pose\nAnimGraph"),
        minimum_observation_seconds: if stage == 8 { 0.1 } else { 0.25 },
        source_pose_chain: vec![
            ManualAnimationPoseBoundary {
                label: "overlay",
                graph: &SOURCE_LIBRARY.0[&MODES[mode]].graph,
                node: "Output Pose\nAnimGraph",
                output: "Pose",
                minimum_translation_span: 0.0,
                minimum_rotation_span_radians: 0.0,
            },
            ManualAnimationPoseBoundary {
                label: "layering",
                graph: "ALS/Character/AB_Als/AnimGraph.animgraph.ron",
                node: "AB_Als_Layering\nLinked Anim Graph",
                output: "Pose",
                minimum_translation_span: 0.0,
                minimum_rotation_span_radians: 0.0,
            },
        ],
        ..default()
    }
}

pub(super) fn observe(
    probe: &mut Probe,
    player: &AnimationGraphPlayer,
    motion: &MotionProbe,
    object_state: Option<&AlsOverlayObjectState>,
    context: Option<&AlsRootAnimationContext>,
    stance: Stance,
    rotation: RotationMode,
    locomotion: LocomotionState,
    speed: f32,
    window: &mut ManualAnimationWindowEvidence,
) {
    let mode = MODES[probe.mode];
    let definition = &SOURCE_LIBRARY.0[&mode];
    let prefix = std::path::Path::new(&definition.graph).parent().unwrap();
    let mut sample = Sample::default();
    sample.pitch = graph_global_f32(player, "view_state.pitch_amount");
    let Some(arena) = player.get_context_arena() else {
        probe.sample = sample;
        return;
    };
    sample.parent = arena
        .get_toplevel()
        .node_states
        .get::<bevy_als::extensions::source_instance::SourceInstanceProperties>(
            NodeId::default(),
            StateKey::Default,
        )
        .ok()
        .map(|p| p.instance_id);
    if let Some(parent) = sample.parent {
        if *probe.parent.get_or_insert(parent) != parent {
            sample
                .errors
                .push("Parent animation instance changed".into());
        }
    }
    for id in arena.iter_context_ids() {
        let Some(ctx) = arena.get_context(id) else {
            continue;
        };
        let Some(path) = motion.asset_server.get_path(ctx.get_graph_id()) else {
            continue;
        };
        if !path.path().starts_with(prefix) {
            continue;
        }
        let Some(graph) = motion.graphs.get(ctx.get_graph_id()) else {
            continue;
        };
        for node in graph.nodes.values() {
            if !ctx.node_caches.is_updated(node.id, StateKey::Default) {
                continue;
            }
            if path.path() == std::path::Path::new(&definition.graph)
                && node.name == "Output Pose\nAnimGraph"
            {
                if let Some(DataValue::Pose(pose)) =
                    ctx.node_caches
                        .get_output_data(node.id, StateKey::Default, "Pose".into())
                {
                    if pose.bones.is_empty() {
                        sample.errors.push("Generated overlay pose is empty".into());
                    } else {
                        sample.graph = Some(definition.graph.clone());
                    }
                }
                if let Some(context) = bevy_als::extensions::animation_context::node_context_state(
                    &ctx.node_states,
                    StateKey::Default,
                    node.id,
                ) {
                    sample.curves = context
                        .curves
                        .iter()
                        .map(|(name, value)| (name.to_string(), *value))
                        .collect();
                }
            }
            if let Some(fsm) =
                node.try_inner_downcast_ref::<bevy_als::extensions::InterruptibleFsmNode>()
            {
                if let Some(machine) = motion.machines.get(&fsm.fsm)
                    && let Ok(runtime) = ctx
                        .node_states
                        .get::<bevy_als::extensions::InterruptibleFsmState>(node.id, StateKey::Default)
                {
                    sample.state = match &runtime.state {
                        LowLevelStateId::HlState(id) => {
                            machine.states.get(id).map(|state| state.label.clone())
                        }
                        _ => None,
                    };
                    sample.transition = runtime.active_direct_transition_id();
                    if (4..=6).contains(&probe.mode) {
                        for state in machine.states.values() {
                            if let Some(index) = ["Relaxed", "Ready", "Aiming"]
                                .iter()
                                .position(|label| *label == state.label)
                                && runtime.recorded_state_weight(&state.id.uuid().to_string())
                                    > 1e-4
                            {
                                probe.fsm_states[probe.mode - 4][index] = true;
                            }
                        }
                    }
                }
            }
            let Some(sequence) =
                node.try_inner_downcast_ref::<bevy_als::extensions::SourceSequenceEvaluatorNode>()
            else {
                continue;
            };
            let Some(clip) = motion.clips.get(&sequence.clip) else {
                sample.errors.push("Evaluator clip is missing".into());
                continue;
            };
            let Some(DataValue::Pose(actual)) =
                ctx.node_caches
                    .get_output_data(node.id, StateKey::Default, "Pose".into())
            else {
                continue;
            };
            let pin = graph.edges_inverted.get(&TargetPin::NodeData(
                node.id,
                sequence.explicit_value_pin.clone(),
            ));
            let value = pin
                .and_then(|pin| match pin {
                    SourcePin::NodeData(node, pin) => ctx
                        .node_caches
                        .get_output_data(*node, StateKey::Default, pin.clone())
                        .and_then(|value| value.as_f32().ok()),
                    _ => None,
                })
                .or_else(|| {
                    (!sequence.explicit_value_key.is_empty())
                        .then(|| graph_global_f32(player, &sequence.explicit_value_key))
                        .flatten()
                })
                .unwrap_or(sequence.explicit_value);
            let time = (if sequence.use_explicit_frame {
                (value.max(0.0) as f64 / sequence.frames_per_second) as f32
            } else {
                value
            })
            .clamp(0.0, clip.duration());
            let intervals = (clip.duration() as f64 * sequence.frames_per_second)
                .round()
                .max(1.0) as f32;
            let key_time = if sequence.step_interpolation {
                ((time * sequence.frames_per_second as f32)
                    .floor()
                    .clamp(0.0, intervals)
                    * (clip.duration() / intervals))
                    .min(clip.duration())
            } else {
                time
            };
            let mut expected =
                bevy_als::extensions::source_sequence_player::sample_pose(clip, key_time);
            if let Some(reference) = &sequence.additive_reference {
                let Some(reference) = motion.clips.get(reference) else {
                    sample.errors.push("Additive reference is missing".into());
                    continue;
                };
                let reference = bevy_als::extensions::source_sequence_player::sample_pose(
                    reference,
                    sequence.additive_reference_position,
                );
                let Some(skeleton) = motion.skeletons.get(&reference.skeleton) else {
                    continue;
                };
                expected = if sequence.additive_mesh_space {
                    bevy_als::extensions::mesh_space_difference::make_mesh_space_difference(
                        &reference, &expected, skeleton,
                    )
                } else {
                    bevy_als::extensions::reference_local_difference::make_local_difference(
                        &reference, &expected, skeleton,
                    )
                };
            }
            expected.timestamp = time;
            sample.evaluators += 1;
            let pose_error = default_overlay_pose_error(&actual, &expected);
            let source_curves = sequence.curves.sample_all(time);
            let curves_match = bevy_als::extensions::animation_context::node_context_state(
                &ctx.node_states,
                StateKey::Default,
                node.id,
            )
            .is_some_and(|state| {
                source_curves.len() == state.curves.len()
                    && source_curves.iter().all(|(name, value)| {
                        state
                            .curves
                            .get(name)
                            .is_some_and(|actual| (*actual - value).abs() < 1e-5)
                    })
            });
            if (actual.timestamp - time).abs() > 1e-6
                || pose_error.is_none_or(|error| error > 2e-5)
                || !curves_match
            {
                sample.errors.push(format!("Evaluator {} differs: time={} expected={} pose={pose_error:?} curves={curves_match}", node.name, actual.timestamp, time));
            }
        }
    }
    sample.object = bevy_als::overlay_object::overlay_object_spec(&SOURCE_LIBRARY, mode).map_or_else(
        || object_state.is_some_and(|state| !state.visible && state.prop.is_none()),
        |spec| {
            object_state
                .and_then(|state| state.prop)
                .is_some_and(|prop| {
                    let Ok((object, material)) = motion.overlay_objects.get(prop) else {
                        return false;
                    };
                    let expected = definition.object.as_ref().unwrap();
                    let mut pending = vec![object.content];
                    let mut materials_match = true;
                    while let Some(entity) = pending.pop() {
                        if let Ok(children) = motion.object_children.get(entity) {
                            pending.extend(children.iter().rev());
                        }
                        if let Ok((material, source)) = motion.object_mesh_materials.get(entity)
                            && let Some(handle) = material
                                .map(|material| &material.0)
                                .or_else(|| source.map(|source| &source.0))
                            && let Some(actual) = motion.object_material_assets.get(handle)
                        {
                            let slot = sample.materials.len();
                            sample
                                .materials
                                .push(actual.base_color.to_linear().to_f32_array());
                            materials_match &=
                                expected.materials.get(slot).is_some_and(|expected| {
                                    sample.materials[slot] == expected.linear_rgba
                                        && actual.metallic == expected.metallic
                                        && actual.perceptual_roughness == expected.roughness
                                        && actual.reflectance == expected.reflectance
                                });
                        }
                    }
                    let transform_matches =
                        motion.object_transforms.get(prop).is_ok_and(|transform| {
                            let expected = if probe.stage == 9 {
                                Transform {
                                    translation: expected.socket_translation,
                                    rotation: expected.socket_rotation,
                                    scale: expected.socket_scale,
                                }
                            } else {
                                Transform::IDENTITY
                            };
                            transform
                                .translation
                                .abs_diff_eq(expected.translation, 1e-6)
                                && transform.rotation.abs_diff_eq(expected.rotation, 1e-6)
                                && transform.scale.abs_diff_eq(expected.scale, 1e-6)
                        });
                    sample.material_slots = material.map(|value| value.slots);
                    object.overlay == mode
                        && object.attachment_bone
                            == bevy_als::overlay_object::attachment_bone(spec, probe.stage != 9)
                        && material
                            .is_some_and(|value| value.source_slot_pattern && value.slots > 0)
                        && materials_match
                        && sample.materials.len() == expected.materials.len()
                        && transform_matches
                })
        },
    );
    let ready = settled(
        probe.stage,
        stance,
        rotation,
        locomotion,
        speed,
        &sample,
        (4..=6).contains(&probe.mode),
        context,
    );
    *window = self::window(probe.mode, probe.stage, ready);
    if ready {
        probe.seconds += graph_global_f32(player, "animation.delta_seconds").unwrap_or(0.0);
        probe.covered[probe.mode][probe.stage] |=
            probe.seconds >= if probe.stage == 8 { 0.15 } else { 0.35 };
    } else {
        probe.seconds = 0.0;
    }
    probe.sample = sample;
}

impl Probe {
    pub(super) fn advance(&mut self) -> bool {
        self.seconds = 0.0;
        self.stage += 1;
        if self.stage == LABELS[0].len() {
            self.stage = 0;
            self.mode += 1;
        }
        self.mode == MODES.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn overlay_class_fixture_prepares_before_temporal_observation() {
        for mode in 0..MODES.len() {
            let preparing = window(mode, 0, false);
            assert!(
                preparing.requires_source_pose_chain
                    && !preparing.settled
                    && !preparing.capture_source_window
            );
            let ready = window(mode, 0, true);
            assert_eq!(preparing.label, ready.label);
            assert!(ready.settled && ready.capture_source_window);
            for stage in 1..LABELS[mode].len() {
                assert!(!window(mode, stage, false).requires_source_pose_chain);
                assert!(window(mode, stage, true).settled);
            }
        }
    }
    #[test]
    fn overlay_sprint_exit_matches_source_local_additive_chain() {
        use bevy::ecs::system::RunSystemOnce;
        use bevy_animation_graph::core::{
            animation_clip::GraphClip, animation_graph::GraphInputPin,
            animation_node::AnimationNode, edge_data::DataSpec, systems::animation_player,
        };
        let (mut app, _, _, _) = crate::test_support::loaded_camera_scene();
        let server = app.world().resource::<AssetServer>();
        let relaxed: Handle<AnimationGraph> = server.load("ALS/Character/AnimationInstances/Overlays/AB_Als_PistolTwoHanded/Overlay/SM0_Pistol_States/S2_Relaxed.animgraph.ron");
        let layering: Handle<AnimationGraph> = server.load(LAYERING_GRAPH);
        let sprint: Handle<GraphClip> =
            server.load("ALS/Animations/Grounded/Sprint/A_Als_Sprint.anim.ron");
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            app.update();
            let server = app.world().resource::<AssetServer>();
            if server.is_loaded_with_dependencies(relaxed.id())
                && server.is_loaded_with_dependencies(layering.id())
                && server.is_loaded_with_dependencies(sprint.id())
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "Overlay source-chain assets did not load within 30 seconds"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        let layer = |name, graph| {
            AnimationNode::new(
                name,
                bevy_als::extensions::LayeredGraphNode {
                    capture_context: true,
                    internalize_time: true,
                    output_aliases: vec![bevy_als::extensions::layered_graph::OutputDataAlias {
                        child_output: "Result".into(),
                        outer_output: "Pose".into(),
                    }],
                    ..bevy_als::extensions::LayeredGraphNode::new(graph)
                },
            )
        };
        let overlay = layer("Overlay", relaxed);
        let layered = layer("Layering", layering.clone());
        let mut root = AnimationGraph::new();
        root.add_input_data(
            GraphInputPin::Passthrough("Locomotion Input".into()),
            DataSpec::Pose,
        );
        root.add_edge(
            SourcePin::InputData(GraphInputPin::Passthrough("Locomotion Input".into())),
            TargetPin::NodeData(layered.id, "Locomotion Input".into()),
        );
        root.add_node_parameter_edge(overlay.id, "Pose", layered.id, "Overlay Input");
        root.add_output_data("pose".into(), DataSpec::Pose);
        root.add_output_data_edge(layered.id, "Pose", "pose");
        root.add_node(overlay);
        root.add_node(layered);
        let root = app
            .world_mut()
            .resource_mut::<Assets<AnimationGraph>>()
            .add(root);
        let clip = app
            .world()
            .resource::<Assets<GraphClip>>()
            .get(&sprint)
            .unwrap();
        let locomotion = bevy_als::extensions::source_sequence_player::sample_pose(clip, 0.2);
        let skeleton = app
            .world()
            .resource::<Assets<Skeleton>>()
            .get(&clip.skeleton)
            .unwrap();
        let hand = skeleton
            .iter_bones()
            .find(|bone| {
                skeleton.id_to_path(*bone).is_some_and(|path| {
                    path.parts
                        .last()
                        .is_some_and(|name| name.as_str() == "hand_l")
                })
            })
            .unwrap();
        let mut player = AnimationGraphPlayer::new(clip.skeleton.clone()).with_graph(root);
        player.set_input_data("Locomotion Input", locomotion.into());
        player.set_global_input_data(
            bevy_als::extensions::global_f32_input::ROOT_DEFAULTS_ENABLED_KEY,
            true,
        );
        for (key, value) in [
            ("animation.delta_seconds", 0.0123616_f32),
            ("pose_state.standing_amount", 1.0),
            ("pose_state.gait_walking_amount", 1.0),
            ("pose_state.crouching_amount", 0.0),
            ("layering_state.arm_left_blend_amount", 1.0),
            ("layering_state.arm_left_local_space_blend_amount", 1.0),
        ] {
            player.set_global_input_data(key, value);
        }
        let entity = app
            .world_mut()
            .spawn((
                player,
                Name::new("Overlay source chain"),
                Transform::default(),
                GlobalTransform::default(),
            ))
            .id();
        app.world_mut().spawn((Name::new("root"), ChildOf(entity)));
        let mut previous_alpha = 1.0_f32;
        let mut previous_rotation: Option<Quat> = None;
        for alpha in [1.0_f32, 0.890038, 0.447047, 0.0] {
            let mut player = app
                .world_mut()
                .get_mut::<AnimationGraphPlayer>(entity)
                .unwrap();
            player.set_global_input_data("pose_state.gait_sprinting_amount", alpha);
            player.set_global_input_data(
                "layering_state.arm_left_additive_blend_amount",
                previous_alpha,
            );
            app.world_mut().run_system_once(animation_player).unwrap();
            let player = app.world().get::<AnimationGraphPlayer>(entity).unwrap();
            assert!(player.get_error().is_none(), "{:?}", player.get_error());
            let arena = player.get_context_arena().unwrap();
            let context = arena
                .iter_context_ids()
                .filter_map(|id| arena.get_context(id))
                .find(|context| context.get_graph_id() == layering.id())
                .unwrap();
            let graph = app
                .world()
                .resource::<Assets<AnimationGraph>>()
                .get(&layering)
                .unwrap();
            let node = graph
                .nodes
                .values()
                .find(|node| {
                    node.try_inner_downcast_ref::<bevy_als::extensions::SourceApplyAdditiveNode>()
                        .is_some_and(|node| {
                            node.alpha_key == "layering_state.arm_left_additive_blend_amount"
                        })
                })
                .unwrap();
            let input = |pin: &str| {
                let SourcePin::NodeData(id, output) =
                    &graph.edges_inverted[&TargetPin::NodeData(node.id, pin.into())]
                else {
                    panic!("Missing source pose edge");
                };
                context
                    .node_caches
                    .get_output_data(*id, StateKey::Default, output.clone())
                    .unwrap()
                    .into_pose()
                    .unwrap()
                    .get_bone(hand)
                    .unwrap()
                    .rotation
                    .unwrap()
            };
            let base = input("Base");
            let delta = input("Additive");
            let expected =
                (Quat::IDENTITY.lerp(delta, previous_alpha).normalize() * base).normalize();
            let actual = player
                .get_default_output_pose()
                .unwrap()
                .get_bone(hand)
                .unwrap()
                .rotation
                .unwrap();
            assert!(
                (actual - expected)
                    .length()
                    .min((actual + expected).length())
                    < 2e-5,
                "Source hand composition differs: {alpha}"
            );
            for id in arena.iter_context_ids() {
                let context = arena.get_context(id).unwrap();
                let graphs = app.world().resource::<Assets<AnimationGraph>>();
                let graph = graphs.get(context.get_graph_id()).unwrap();
                for node in graph.nodes.values() {
                    if node
                        .try_inner_downcast_ref::<bevy_als::extensions::SourceTwoWayBlendNode>()
                        .is_none()
                        || !context.node_caches.is_updated(node.id, StateKey::Default)
                    {
                        continue;
                    }
                    let state = context
                        .node_states
                        .get::<bevy_als::extensions::source_pose_nodes::SourceTwoWayBlendState>(
                            node.id,
                            StateKey::Default,
                        )
                        .unwrap();
                    if !(1e-5..1.0 - 1e-5).contains(&state.blend_alpha) {
                        continue;
                    }
                    let rotation = |pin: &str| {
                        let SourcePin::NodeData(id, output) =
                            &graph.edges_inverted[&TargetPin::NodeData(node.id, pin.into())]
                        else {
                            panic!("Missing source blend edge");
                        };
                        context
                            .node_caches
                            .get_output_data(*id, StateKey::Default, output.clone())
                            .unwrap()
                            .into_pose()
                            .unwrap()
                            .get_bone(hand)
                            .unwrap()
                            .rotation
                            .unwrap()
                    };
                    let expected = rotation("A")
                        .lerp(rotation("B"), state.blend_alpha)
                        .normalize();
                    let actual = context
                        .node_caches
                        .get_output_data(node.id, StateKey::Default, "Pose".into())
                        .unwrap()
                        .into_pose()
                        .unwrap()
                        .get_bone(hand)
                        .unwrap()
                        .rotation
                        .unwrap();
                    assert!(
                        (actual - expected)
                            .length()
                            .min((actual + expected).length())
                            < 2e-5,
                        "Source quaternion blend differs: {}",
                        node.name
                    );
                }
            }
            if let Some(previous) = previous_rotation {
                let change = 2.0 * previous.dot(actual).abs().clamp(0.0, 1.0).acos();
                if alpha == 0.0 {
                    assert!(
                        change > 1.0,
                        "Source compound transition no longer exercises the reported boundary"
                    );
                }
                eprintln!(
                    "Source sprint exit alpha={alpha} previous_alpha={previous_alpha} hand_delta={change}"
                );
            }
            previous_rotation = Some(actual);
            previous_alpha = alpha;
        }
    }
    #[test]
    fn overlay_fixture_requires_real_pose_object_and_source_state() {
        let sample = Sample {
            graph: Some("generated".into()),
            parent: Some(1),
            evaluators: 1,
            object: true,
            state: Some("Relaxed".into()),
            ..default()
        };
        assert!(settled(
            0,
            Stance::Standing,
            RotationMode::ViewDirection,
            LocomotionState::Grounded,
            0.0,
            &sample,
            true,
            None
        ));
        assert!(!settled(
            3,
            Stance::Standing,
            RotationMode::Aiming,
            LocomotionState::Grounded,
            0.0,
            &sample,
            true,
            None
        ));
        assert!(settled(
            7,
            Stance::Standing,
            RotationMode::ViewDirection,
            LocomotionState::Grounded,
            0.0,
            &sample,
            true,
            None
        ));
        assert!(!settled(
            7,
            Stance::Standing,
            RotationMode::ViewDirection,
            LocomotionState::Grounded,
            2.0,
            &sample,
            true,
            None
        ));
        assert!(!settled(
            7,
            Stance::Standing,
            RotationMode::ViewDirection,
            LocomotionState::Air,
            0.0,
            &sample,
            true,
            None
        ));
        assert!(!settled(
            0,
            Stance::Standing,
            RotationMode::ViewDirection,
            LocomotionState::Grounded,
            0.0,
            &Sample {
                object: false,
                ..sample
            },
            false,
            None
        ));
        for stage in 3..=6 {
            assert!((pitch(stage, 0.0) - pitch(stage - 1, 4.0)).abs() < 1e-6);
        }
        let mut probe = Probe::default();
        for _ in 1..MODES.len() * LABELS[0].len() {
            assert!(!probe.advance());
        }
        assert!(probe.advance());
    }
}
