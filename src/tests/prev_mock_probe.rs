use bevy::prelude::*;
use bevy_als_playground::input::{
    AlsInputMap, NextOverlayModeAction, OnFoot, PreviousOverlayModeAction, spawn_als_actions,
};
use bevy_enhanced_input::prelude::*;

#[test]
fn prev_action_mock_fires_events() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        bevy::input::InputPlugin,
        EnhancedInputPlugin,
        bevy_als_playground::input::AlsInputPlugin,
    ));
    app.finish();

    let player = app.world_mut().spawn_empty().id();
    let mut cmds = app.world_mut().commands();
    spawn_als_actions(&mut cmds, player, &AlsInputMap::default());
    app.update();

    app.world_mut()
        .commands()
        .entity(player)
        .mock_once::<OnFoot, NextOverlayModeAction>(TriggerState::Fired, 1.0);
    app.update();
    let next_fired = app
        .world_mut()
        .query_filtered::<&ActionEvents, With<Action<NextOverlayModeAction>>>()
        .single(app.world())
        .unwrap()
        .contains(ActionEvents::START);
    assert!(next_fired, "next mock should fire START");

    app.world_mut()
        .commands()
        .entity(player)
        .mock_once::<OnFoot, PreviousOverlayModeAction>(TriggerState::Fired, 1.0);
    app.update();
    let prev_fired = app
        .world_mut()
        .query_filtered::<&ActionEvents, With<Action<PreviousOverlayModeAction>>>()
        .single(app.world())
        .unwrap()
        .contains(ActionEvents::START);
    assert!(prev_fired, "prev mock should fire START");
}
