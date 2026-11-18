use super::*;

pub(super) fn install(app: &mut App) {
    if std::env::var("ALS_MANUAL_RAGDOLL_DEBUG_RENDER").as_deref() == Ok("1") {
        app.add_systems(
            Update,
            verify
                .after(drive_manual_integration)
                .after(bevy_als::debug_target::AlsDebugTargetSet),
        );
    }
}

fn verify(
    mut state: ResMut<ManualIntegrationState>,
    mut detached_since: Local<Option<(Entity, f64)>>,
    time: Res<Time>,
    actions: Query<&AlsNotifyLocomotionAction, With<AlsCharacter>>,
    avatars: Query<(&AlsCharacterAvatar, &AnimatedSceneInstance)>,
    detached: Query<&bevy_als::ragdoll_link::AlsRagdollDetachedPlayer>,
    meshes: Query<(Entity, &ViewVisibility), With<bevy::mesh::skinning::SkinnedMesh>>,
    parents: Query<&ChildOf>,
    stamps: Query<&bevy_als::debug_target::AlsLastDebugRender>,
) {
    if !state.initialized
        || state.scenario != IntegrationScenario::Ragdoll
        || state.phase != IntegrationPhase::Ragdoll
        || state.coverage.ragdoll_debug_render
    {
        return;
    }
    let (Some(character), Some(avatar)) = (state.character, state.avatar) else {
        return;
    };
    if actions.get(character).ok() != Some(&AlsNotifyLocomotionAction::Ragdolling) {
        return;
    }
    let Ok((owner, instance)) = avatars.get(avatar) else {
        return;
    };
    if owner.character != character {
        return;
    }
    let player = instance.player_entity();
    if !detached
        .get(player)
        .is_ok_and(|marker| marker.character == character)
    {
        return;
    }
    let now = time.elapsed_secs_f64();
    let threshold = 0.2f64.max(time.delta_secs_f64() + 0.0001);
    let since = match *detached_since {
        Some((observed, since)) if observed == player => since,
        _ => {
            *detached_since = Some((player, now));
            return;
        }
    };
    if now - since <= threshold {
        return;
    }
    let visible = meshes.iter().any(|(mesh, visibility)| {
        visibility.get()
            && (mesh == player
                || parents
                    .iter_ancestors(mesh)
                    .any(|ancestor| ancestor == player))
    });
    if !visible {
        return;
    }
    let Ok(stamp) = stamps.get(character) else {
        return;
    };
    let age = now - stamp.0;
    if age <= threshold {
        state.coverage.ragdoll_debug_render = true;
        info!(
            "[MANUAL-INTEGRATION][RAGDOLL-DEBUG-RENDER] player={player:?} detached={:.3} age={age:.3} -> PASS",
            now - since
        );
    }
}
