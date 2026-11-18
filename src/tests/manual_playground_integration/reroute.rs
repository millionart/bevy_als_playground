use super::*;
use bevy_als::extensions::{SourceScalarDataType, SourceScalarExpressionNode, SourceScalarPin};
use bevy::platform::collections::HashMap;
use bevy_animation_graph::core::{
    animation_graph::{GraphInputPin, TimeUpdate},
    animation_node::AnimationNode,
    context::{
        deferred_gizmos::DeferredGizmos, graph_context::QueryOutputTime,
        graph_context_arena::GraphContextArena, new_context::GraphContext,
        system_resources::SystemResources,
    },
    edge_data::DataSpec,
    utils::testing::TestIoEnv,
};

pub(super) fn install(app: &mut App) {
    if std::env::var("ALS_MANUAL_REROUTE_PASSTHROUGH").as_deref() == Ok("1") {
        app.add_systems(
            PostUpdate,
            verify
                .after(AlsCameraSet)
                .before(observe_manual_integration),
        );
    }
}

fn query(
    In(handle): In<Handle<AnimationGraph>>,
    resources: SystemResources,
) -> Result<(f32, bool), String> {
    let graph = resources
        .animation_graph_assets
        .get(&handle)
        .ok_or("missing reroute graph")?;
    let mut arena = GraphContextArena::new(handle.id());
    let mut gizmos = DeferredGizmos::default();
    let entities = HashMap::new();
    let inputs = HashMap::new();
    let env = TestIoEnv::default();
    let context = GraphContext::new(
        arena.get_toplevel_id(),
        graph,
        &mut arena,
        &resources,
        &env,
        Entity::PLACEHOLDER,
        &entities,
        &mut gizmos,
        &inputs,
    );
    let output = graph
        .query_with_context(QueryOutputTime::Forced(TimeUpdate::Delta(0.02)), context)
        .map_err(|error| format!("{error:?}"))?;
    Ok((
        output
            .get("Float")
            .ok_or("missing float output")?
            .as_f32()
            .map_err(|error| format!("{error:?}"))?,
        output
            .get("Bool")
            .ok_or("missing bool output")?
            .as_bool()
            .map_err(|error| format!("{error:?}"))?,
    ))
}

fn verify(world: &mut World, mut attempted: Local<bool>) {
    if *attempted {
        return;
    }
    let state = world.resource::<ManualIntegrationState>();
    if !state.initialized || state.scenario != IntegrationScenario::Base {
        return;
    }
    *attempted = true;

    let mut graph = AnimationGraph::new();
    for (name, data_type, value, spec) in [
        (
            "Float",
            SourceScalarDataType::F32,
            DataValue::F32(2.5),
            DataSpec::F32,
        ),
        (
            "Bool",
            SourceScalarDataType::Bool,
            DataValue::Bool(true),
            DataSpec::Bool,
        ),
    ] {
        let node = AnimationNode::new(
            name,
            SourceScalarExpressionNode {
                operation: "passthrough".into(),
                input_pins: vec![SourceScalarPin {
                    name: "InputPin".into(),
                    data_type: data_type.clone(),
                    ..default()
                }],
                output_pins: vec![SourceScalarPin {
                    name: "OutputPin".into(),
                    data_type,
                    ..default()
                }],
                ..default()
            },
        );
        graph.add_node(node.clone());
        graph.set_default_data(GraphInputPin::passthrough(name), value);
        graph.add_input_data_edge(GraphInputPin::passthrough(name), node.id, "InputPin");
        graph.add_output_data(name.into(), spec);
        graph.add_output_data_edge(node.id, "OutputPin", name);
    }
    let handle = world.resource_mut::<Assets<AnimationGraph>>().add(graph);
    let result = world.run_system_cached_with(query, handle);
    let passed = matches!(&result, Ok(Ok((value, condition))) if *value == 2.5 && *condition);
    world
        .resource_mut::<ManualIntegrationState>()
        .coverage
        .reroute_passthrough = passed;
    info!(
        "[MANUAL-INTEGRATION][REROUTE] result={result:?} -> {}",
        if passed { "PASS" } else { "FAIL" }
    );
}
