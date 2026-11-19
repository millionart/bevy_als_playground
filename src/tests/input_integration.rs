use bevy::prelude::*;

#[test]
fn paused_sprint_cancel_preserves_the_actual_source_max_allowed_gait() {
    use crate::input::{AlsInputMap, AlsInputPlugin, spawn_als_actions};
    use bevy::input::gamepad::{RawGamepadButtonChangedEvent, RawGamepadEvent};
    use bevy::time::TimeUpdateStrategy;
    use bevy_als::als::character::AlsMovementParamsHandle;
    use bevy_als::als::gait::{
        AlsDesiredGait, AlsGaitAmount, AlsMaxAllowedGait, refresh_gait_system,
    };
    use bevy_als::als::motion::{AlsActualVelocity, AlsViewYaw};
    use bevy_als::als::transition::{AlsStateRequest, StateTransitionEvent};
    use bevy_als::als::{Gait, LocomotionState, RotationMode, Stance};
    use bevy_als::{AlsCharacter, AlsMovementParams};
    for gamepad_input in [false, true] {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::input::InputPlugin, AlsInputPlugin))
            .init_resource::<Assets<AlsMovementParams>>()
            .add_message::<StateTransitionEvent>()
            .insert_resource(TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_millis(20),
            ))
            .add_systems(
                Update,
                bevy_als::als::input_request::input_state_request_system,
            )
            .add_systems(PostUpdate, refresh_gait_system);
        app.finish();
        let handle = app
            .world_mut()
            .resource_mut::<Assets<AlsMovementParams>>()
            .add(AlsMovementParams::default());
        let character = app
            .world_mut()
            .spawn((
                AlsCharacter,
                LocomotionState::Grounded,
                Stance::Standing,
                RotationMode::VelocityDirection,
                AlsViewYaw(0.0),
                AlsDesiredGait(Gait::Running),
                AlsMaxAllowedGait(Gait::Running),
                AlsGaitAmount::default(),
                Gait::Walking,
                AlsActualVelocity::default(),
                AlsMovementParamsHandle(handle),
                AlsStateRequest::default(),
                bevy_als::actual_motion::AlsActualMotion {
                    has_input: true,
                    ..default()
                },
            ))
            .id();
        let gamepad = app.world_mut().spawn(Gamepad::default()).id();
        spawn_als_actions(
            &mut app.world_mut().commands(),
            character,
            &AlsInputMap::default(),
        );
        app.update();
        if gamepad_input {
            app.world_mut().write_message(RawGamepadEvent::Button(
                RawGamepadButtonChangedEvent::new(gamepad, GamepadButton::East, 1.0),
            ));
        } else {
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(KeyCode::ShiftLeft);
        }
        for _ in 0..2 {
            app.update();
        }
        assert_eq!(
            app.world().get::<AlsDesiredGait>(character).unwrap().0,
            Gait::Running
        );
        app.world_mut().resource_mut::<Time<Virtual>>().pause();
        app.update();
        assert_eq!(
            app.world().get::<AlsDesiredGait>(character).unwrap().0,
            Gait::Sprinting,
            "Source Sprint Canceled callback preserves its actual positive value during pause"
        );
        assert_eq!(
            app.world().get::<AlsMaxAllowedGait>(character).unwrap().0,
            Gait::Running,
            "Source paused Character must retain its last refreshed max allowed gait"
        );
        app.world_mut().resource_mut::<Time<Virtual>>().unpause();
        app.update();
        assert_eq!(
            app.world().get::<AlsMaxAllowedGait>(character).unwrap().0,
            Gait::Sprinting,
            "Source resumed Character must consume its saved desired gait"
        );
    }
}

#[test]
fn paused_move_events_do_not_refresh_source_actual_character_input() {
    use crate::input::{AlsInputMap, AlsInputPlugin, spawn_als_actions};
    use avian3d::prelude::PhysicsPlugins;
    use avian3d::prelude::{Collider, RigidBody};
    use bevy::asset::AssetEvent;
    use bevy::input::gamepad::{RawGamepadAxisChangedEvent, RawGamepadEvent};
    use bevy::time::TimeUpdateStrategy;
    use bevy_als::AlsMovementParams;
    use bevy_als::als::character::{AlsCharacterBundle, AlsCharacterPlugin};
    use bevy_als::als::control_scheme::{AlsControlScheme, AlsControlSchemeConfig};
    use bevy_als::als::motion::{
        AlsActualVelocitySet, AlsMotionPlugin, initial_als_control_scheme_config,
    };
    use bevy_als::als::transition::AlsStateTransitionPlugin;
    use bevy_tnua::{TnuaController, TnuaControllerPlugin};
    use bevy_tnua_avian3d::TnuaAvian3dPlugin;
    for (initial, gamepad_input) in [
        (Vec2::ZERO, false),
        (Vec2::Y, false),
        (Vec2::ZERO, true),
        (Vec2::Y, true),
    ] {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            TransformPlugin,
            bevy::input::InputPlugin,
            AlsInputPlugin,
            PhysicsPlugins::new(PostUpdate),
            TnuaAvian3dPlugin::new(Update),
            TnuaControllerPlugin::<AlsControlScheme>::new(Update),
            AlsStateTransitionPlugin,
            AlsCharacterPlugin,
            AlsMotionPlugin,
            bevy_als::actual_motion::AlsActualMotionPlugin,
        ))
        .configure_sets(
            PostUpdate,
            bevy_als::actual_motion::AlsActualMotionSet.after(AlsActualVelocitySet),
        )
        .init_resource::<Assets<Mesh>>()
        .add_message::<AssetEvent<Mesh>>()
        .insert_resource(TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f32(1.0 / 60.0),
        ));
        app.finish();
        let params = AlsMovementParams::default();
        let handle = app
            .world_mut()
            .resource_mut::<Assets<AlsMovementParams>>()
            .add(params.clone());
        let config = initial_als_control_scheme_config(&params);
        let mut bundle = AlsCharacterBundle::new(
            handle,
            &mut app
                .world_mut()
                .resource_mut::<Assets<AlsControlSchemeConfig>>(),
            config,
        );
        bundle.transform.translation = Vec3::new(0.0, 0.924, 0.0);
        let character = app.world_mut().spawn(bundle).id();
        let gamepad = app.world_mut().spawn(Gamepad::default()).id();
        spawn_als_actions(
            &mut app.world_mut().commands(),
            character,
            &AlsInputMap::default(),
        );
        app.world_mut().spawn((
            RigidBody::Static,
            Collider::cuboid(20.0, 0.2, 20.0),
            Transform::from_xyz(0.0, -0.1, 0.0),
        ));
        for _ in 0..30 {
            app.update();
        }
        if initial == Vec2::Y {
            if gamepad_input {
                app.world_mut().write_message(RawGamepadEvent::Axis(
                    RawGamepadAxisChangedEvent::new(gamepad, GamepadAxis::LeftStickY, -1.0),
                ));
            } else {
                app.world_mut()
                    .resource_mut::<ButtonInput<KeyCode>>()
                    .press(KeyCode::KeyW);
            }
        }
        for _ in 0..5 {
            app.update();
        }
        let before = *app
            .world()
            .get::<bevy_als::actual_motion::AlsActualMotion>(character)
            .unwrap();
        app.world_mut().resource_mut::<Time<Virtual>>().pause();
        if gamepad_input {
            for (axis, value) in [
                (GamepadAxis::LeftStickY, 0.0),
                (GamepadAxis::LeftStickX, 1.0),
            ] {
                app.world_mut().write_message(RawGamepadEvent::Axis(
                    RawGamepadAxisChangedEvent::new(gamepad, axis, value),
                ));
            }
        } else {
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keys.release(KeyCode::KeyW);
            keys.press(KeyCode::KeyD);
        }
        for _ in 0..3 {
            app.update();
        }
        let actual = app
            .world()
            .get::<bevy_als::actual_motion::AlsActualMotion>(character)
            .unwrap();
        assert_eq!(
            (actual.has_input, actual.input_yaw_degrees),
            (before.has_input, before.input_yaw_degrees),
            "Source paused Character owner must preserve its last refreshed input for initial={initial:?}"
        );
        assert_eq!(
            *actual, before,
            "Source paused owner must retain its complete motion snapshot"
        );
        app.world_mut().resource_mut::<Time<Virtual>>().unpause();
        app.update();
        let resumed = app
            .world()
            .get::<bevy_als::actual_motion::AlsActualMotion>(character)
            .unwrap();
        assert!(
            resumed.has_input && (resumed.input_yaw_degrees + 90.0).abs() < 1e-5,
            "Source resumed Character must refresh the current held movement for device={gamepad_input}"
        );
        let retained_yaw = resumed.input_yaw_degrees;
        if gamepad_input {
            app.world_mut()
                .write_message(RawGamepadEvent::Axis(RawGamepadAxisChangedEvent::new(
                    gamepad,
                    GamepadAxis::LeftStickX,
                    0.0,
                )));
        } else {
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .release(KeyCode::KeyD);
        }
        for _ in 0..3 {
            app.update();
            let released = app
                .world()
                .get::<bevy_als::actual_motion::AlsActualMotion>(character)
                .unwrap();
            assert!(
                !released.has_input && released.input_yaw_degrees == retained_yaw,
                "Source Move release must consume zero each tick without clearing its retained direction"
            );
            assert_eq!(
                app.world()
                    .get::<TnuaController<AlsControlScheme>>(character)
                    .unwrap()
                    .basis
                    .desired_motion,
                Vec3::ZERO
            );
        }
        if gamepad_input {
            app.world_mut()
                .write_message(RawGamepadEvent::Axis(RawGamepadAxisChangedEvent::new(
                    gamepad,
                    GamepadAxis::LeftStickX,
                    1.0,
                )));
        } else {
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(KeyCode::KeyD);
        }
        app.update();
        assert!(
            app.world()
                .get::<bevy_als::actual_motion::AlsActualMotion>(character)
                .unwrap()
                .has_input
        );
        app.world_mut().resource_mut::<Time<Virtual>>().pause();
        if gamepad_input {
            app.world_mut()
                .write_message(RawGamepadEvent::Axis(RawGamepadAxisChangedEvent::new(
                    gamepad,
                    GamepadAxis::LeftStickX,
                    0.0,
                )));
        } else {
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .release(KeyCode::KeyD);
        }
        app.update();
        assert!(
            app.world()
                .get::<bevy_als::actual_motion::AlsActualMotion>(character)
                .unwrap()
                .has_input
        );
        app.world_mut().resource_mut::<Time<Virtual>>().unpause();
        for _ in 0..3 {
            app.update();
            assert!(
                !app.world()
                    .get::<bevy_als::actual_motion::AlsActualMotion>(character)
                    .unwrap()
                    .has_input,
                "Source Move released while paused must not leave pending input on resume"
            );
            assert_eq!(
                app.world()
                    .get::<TnuaController<AlsControlScheme>>(character)
                    .unwrap()
                    .basis
                    .desired_motion,
                Vec3::ZERO
            );
        }
    }
}

#[test]
fn roll_paused_tap_release_does_not_start_the_actual_roll_driver() {
    use crate::input::{AlsInputCommand, AlsInputMap, spawn_als_actions};
    use avian3d::prelude::LinearVelocity;
    use bevy::input::gamepad::{RawGamepadButtonChangedEvent, RawGamepadEvent};
    use bevy::time::TimeUpdateStrategy;
    use bevy_als::als::motion::AlsViewYaw;
    use bevy_als::als::roll::AlsRollState;
    use bevy_als::{LocomotionState, Stance};
    let cases = [
        vec![
            (false, true, 2, false),
            (true, true, 1, false),
            (true, false, 2, false),
            (false, false, 1, false),
            (false, true, 2, false),
            (false, false, 1, true),
        ],
        vec![
            (true, true, 2, false),
            (true, false, 2, false),
            (false, false, 1, false),
        ],
        vec![
            (true, true, 2, false),
            (false, true, 1, false),
            (false, false, 1, true),
        ],
        vec![
            (false, true, 2, false),
            (true, true, 20, false),
            (false, true, 1, false),
            (false, false, 1, false),
        ],
        vec![
            (false, true, 2, false),
            (true, true, 2, false),
            (false, false, 1, true),
        ],
    ];
    for gamepad_input in [false, true] {
        for speed in [1.0, 0.25] {
            for (case_index, steps) in cases.iter().enumerate() {
                let mut app = crate::test_support::headless_app(
                    bevy_als::AlsPhysicsSchedule::SourceFrameTiming,
                );
                app.insert_resource(TimeUpdateStrategy::ManualDuration(
                    std::time::Duration::from_millis(20),
                ));
                app.finish();
                app.cleanup();
                let character = app
                    .world_mut()
                    .spawn((
                        bevy_als::als::character::AlsCharacter,
                        AlsRollState::default(),
                        LocomotionState::Grounded,
                        Stance::Standing,
                        AlsViewYaw(0.0),
                        Transform::IDENTITY,
                        LinearVelocity::ZERO,
                    ))
                    .id();
                let gamepad = app.world_mut().spawn(Gamepad::default()).id();
                spawn_als_actions(
                    &mut app.world_mut().commands(),
                    character,
                    &AlsInputMap::default(),
                );
                app.update();
                app.world_mut()
                    .resource_mut::<Time<Virtual>>()
                    .set_relative_speed(speed);
                for (step_index, &(paused, held, frames, expected)) in steps.iter().enumerate() {
                    let mut time = app.world_mut().resource_mut::<Time<Virtual>>();
                    if paused {
                        time.pause();
                    } else {
                        time.unpause();
                    }
                    if gamepad_input {
                        app.world_mut().write_message(RawGamepadEvent::Button(
                            RawGamepadButtonChangedEvent::new(
                                gamepad,
                                GamepadButton::East,
                                if held { 1.0 } else { 0.0 },
                            ),
                        ));
                    } else {
                        let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
                        if held {
                            keys.press(KeyCode::ShiftLeft);
                        } else {
                            keys.release(KeyCode::ShiftLeft);
                        }
                    }
                    for _ in 0..frames {
                        app.update();
                    }
                    assert_eq!(
                        app.world().get::<AlsRollState>(character).unwrap().active,
                        expected,
                        "Source actual Roll activation for device={gamepad_input}, speed={speed}, case={case_index}, step={step_index}"
                    );
                    if paused {
                        assert!(
                            !app.world()
                                .get::<AlsInputCommand>(character)
                                .unwrap()
                                .roll_pressed,
                            "Source paused Tap release has no Triggered callback"
                        );
                    }
                }
                app.update();
                assert!(
                    !app.world()
                        .get::<AlsInputCommand>(character)
                        .unwrap()
                        .roll_pressed,
                    "Source Tap callback is a single-frame pulse"
                );
                assert_eq!(
                    app.world().get::<AlsRollState>(character).unwrap().active,
                    steps.last().unwrap().3,
                    "Source actual Roll activation must survive its pulse without replaying a paused release"
                );
            }
        }
    }
}
