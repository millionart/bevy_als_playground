use bevy::ecs::schedule::ScheduleLabel;
use super::*;
use bevy_als::extensions::{InterruptibleFsmNode, InterruptibleFsmState};

const STANDING_GRAPH: &str =
    "ALS/Character/AnimationInstances/Stances/AB_Als_Standing/AnimGraph.animgraph.ron";
const GROUNDED_GRAPH: &str =
    "ALS/Character/AnimationInstances/AB_Als_Grounded/AnimGraph.animgraph.ron";

pub(super) fn install(app: &mut App) {
    if std::env::var("ALS_MANUAL_MACHINE_WEIGHT").as_deref() == Ok("1") {
        let physics = app.world().get_resource::<bevy_als::AlsPhysicsSchedule>()
            .map_or_else(|| PostUpdate.intern(), |schedule| schedule.label());
        app.add_systems(
            physics,
            drive_running
                .after(bevy_als::gait_layer::AlsGaitLayerSet)
                .before(bevy_als::ab_als_root::AlsRootGraphSet),
        );
        app.add_systems(
            physics,
            observe.after(bevy_als::extensions::post_physics_animation::PostPhysicsAnimationSet::Evaluate),
        );
    }
}

fn drive_running(
    state: Res<ManualIntegrationState>,
    mut logged: Local<bool>,
    mut characters: Query<(&Stance, &mut Gait), With<AlsCharacter>>,
) {
    if state.scenario != IntegrationScenario::Base
        || !matches!(
            state.phase,
            IntegrationPhase::CrouchRecovery | IntegrationPhase::CrouchClearance
        )
        || state.machine_weight_partial_phase != Some(state.phase)
        || state.coverage.machine_weight_low_start
    {
        return;
    }
    let Some(character) = state.character else {
        return;
    };
    let Ok((stance, mut gait)) = characters.get_mut(character) else {
        return;
    };
    if *stance == Stance::Standing {
        *gait = Gait::Running;
        if !*logged {
            info!(
                "[MANUAL-INTEGRATION][MACHINE-WEIGHT-INPUT] phase={:?} gait=Running",
                state.phase
            );
            *logged = true;
        }
    }
}

fn machine_sample(
    player: &AnimationGraphPlayer,
    asset_server: &AssetServer,
    graphs: &Assets<AnimationGraph>,
    machines: &Assets<StateMachine>,
    path: &str,
    title: &str,
    labels: &[&str],
) -> Option<(f32, Vec<f32>)> {
    let arena = player.get_context_arena()?;
    arena.iter_context_ids().find_map(|id| {
        let context = arena.get_context(id)?;
        if asset_server.get_path(context.get_graph_id())?.path() != std::path::Path::new(path) {
            return None;
        }
        let graph = graphs.get(context.get_graph_id())?;
        let node = graph.nodes.values().find(|node| node.name == title)?;
        let fsm = node.try_inner_downcast_ref::<InterruptibleFsmNode>()?;
        if !context.node_caches.is_updated(node.id, StateKey::Default) {
            return None;
        }
        let state = context
            .node_states
            .get_all_upcoming_states::<InterruptibleFsmState>(node.id)
            .ok()?
            .find(|state| state.last_output_pose().is_some())?;
        let machine = machines.get(&fsm.fsm)?;
        let weights = labels
            .iter()
            .map(|label| {
                let id = machine
                    .states
                    .values()
                    .find(|state| state.label == *label)?
                    .id;
                Some(state.recorded_state_weight(&id.uuid().to_string()))
            })
            .collect::<Option<Vec<_>>>()?;
        Some((state.recorded_machine_weight(), weights))
    })
}

fn observe(
    mut state: ResMut<ManualIntegrationState>,
    avatars: Query<(&AlsCharacterAvatar, &AnimatedSceneInstance)>,
    players: Query<&AnimationGraphPlayer>,
    asset_server: Res<AssetServer>,
    graphs: Res<Assets<AnimationGraph>>,
    machines: Res<Assets<StateMachine>>,
) {
    if !state.initialized || state.scenario != IntegrationScenario::Base {
        return;
    }
    let (Some(character), Some(avatar)) = (state.character, state.avatar) else {
        return;
    };
    let Ok((owner, instance)) = avatars.get(avatar) else {
        return;
    };
    if owner.character != character {
        return;
    }
    let Ok(player) = players.get(instance.player_entity()) else {
        return;
    };
    let Some((weight, _)) = machine_sample(
        player,
        &asset_server,
        &graphs,
        &machines,
        STANDING_GRAPH,
        "Standing States\nState Machine",
        &[],
    ) else {
        return;
    };
    let Some((_, grounded)) = machine_sample(
        player,
        &asset_server,
        &graphs,
        &machines,
        GROUNDED_GRAPH,
        "Grounded States\nState Machine",
        &["Standing"],
    ) else {
        return;
    };
    let Some((_, detail)) = machine_sample(
        player,
        &asset_server,
        &graphs,
        &machines,
        STANDING_GRAPH,
        "Movement Details States\nState Machine",
        &["Run Start", "Run Start From Walk"],
    ) else {
        return;
    };
    let partial = grounded[0] > 0.01 && grounded[0] < 0.99 && (weight - grounded[0]).abs() < 0.03;
    let full = grounded[0] >= 0.999 && weight >= 0.999;
    if partial
        && matches!(
            state.phase,
            IntegrationPhase::CrouchRecovery | IntegrationPhase::CrouchClearance
        )
    {
        state.machine_weight_partial_phase = Some(state.phase);
    }
    let previous = (
        state.coverage.machine_weight_partial,
        state.coverage.machine_weight_full,
        state.coverage.machine_weight_low_start,
        state.coverage.machine_weight_full_start,
    );
    state.coverage.machine_weight_partial |= partial;
    state.coverage.machine_weight_full |= full;
    state.coverage.machine_weight_low_start |= partial && detail[0] > 0.01;
    state.coverage.machine_weight_full_start |= full && detail[1] > 0.01;
    let current = (
        state.coverage.machine_weight_partial,
        state.coverage.machine_weight_full,
        state.coverage.machine_weight_low_start,
        state.coverage.machine_weight_full_start,
    );
    if current != previous {
        info!(
            "[MANUAL-INTEGRATION][MACHINE-WEIGHT] phase={:?} standing={weight:.3} grounded={:.3} run_start={:.3} run_start_from_walk={:.3} coverage={current:?}",
            state.phase, grounded[0], detail[0], detail[1]
        );
    }
}
