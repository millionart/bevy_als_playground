use bevy::ecs::spawn::{SpawnWith, WithRelated};
use bevy::input::gamepad::GamepadSettings;
use bevy::prelude::*;
use bevy_enhanced_input::prelude::Press as EIPress;
use bevy_enhanced_input::prelude::Release as EIRelease;
use bevy_enhanced_input::prelude::*;
use bevy_als::character_assembly::AlsPlayerCharacter;
pub use bevy_als::als::input::{AlsInputCommand, OnFoot};

pub const SPRINT_HOLD_SECS: f32 = 0.1;
pub const ROLL_TAP_SECS: f32 = 0.3;

#[derive(InputAction)]
#[action_output(Vec2)]
pub struct MoveAction;

#[derive(InputAction)]
#[action_output(Vec2)]
pub struct LookAction;

#[derive(InputAction)]
#[action_output(Vec2)]
pub struct LookMouseAction;

#[derive(InputAction)]
#[action_output(Vec2)]
struct StickAction;

#[derive(InputAction)]
#[action_output(bool)]
struct DebugShiftAction;

#[derive(Component)]
struct DebugShiftContext;

#[derive(Component, Debug, Default)]
struct GamePauseBlocker(bool);

impl InputCondition for GamePauseBlocker {
    fn evaluate(
        &mut self,
        _actions: &ActionsQuery,
        _time: &ContextTime,
        _value: ActionValue,
    ) -> TriggerState {
        if self.0 {
            TriggerState::None
        } else {
            TriggerState::Fired
        }
    }

    fn kind(&self) -> ConditionKind {
        ConditionKind::Blocker
    }
}

fn sync_game_pause_blockers(time: Res<Time<Virtual>>, mut blockers: Query<&mut GamePauseBlocker>) {
    for mut blocker in &mut blockers {
        blocker.0 = time.is_paused();
    }
}

#[derive(Component, Debug)]
struct StickValue(Entity);

impl InputModifier for StickValue {
    fn transform(
        &mut self,
        actions: &ActionsQuery,
        _time: &ContextTime,
        value: ActionValue,
    ) -> ActionValue {
        actions.get(self.0).map_or(value, |(value, ..)| *value)
    }
}

#[derive(InputAction)]
#[action_output(bool)]
pub struct JumpAction;

#[derive(InputAction)]
#[action_output(bool)]
pub struct CrouchAction;

#[derive(InputAction)]
#[action_output(bool)]
pub struct SprintAction;

#[derive(InputAction)]
#[action_output(bool)]
pub struct AimAction;

#[derive(InputAction)]
#[action_output(bool)]
pub struct RollAction;

#[derive(InputAction)]
#[action_output(bool)]
pub struct WalkAction;

#[derive(InputAction)]
#[action_output(bool)]
pub struct RagdollAction;

#[derive(InputAction)]
#[action_output(bool)]
pub struct RotationModeAction;

#[derive(InputAction)]
#[action_output(bool)]
pub struct ViewModeAction;

#[derive(InputAction)]
#[action_output(bool)]
pub struct SwitchShoulderAction;

#[derive(InputAction)]
#[action_output(bool)]
pub struct OverlayMenuAction;

#[derive(InputAction)]
#[action_output(f32)]
pub struct NextOverlayModeAction;

#[derive(InputAction)]
#[action_output(f32)]
pub struct PreviousOverlayModeAction;

#[derive(InputAction)]
#[action_output(bool)]
pub struct SlomoAction;

#[derive(InputAction)]
#[action_output(bool)]
pub struct ToggleUiAction;

pub fn roll_pressed_from_events(events: ActionEvents) -> bool {
    events.contains(ActionEvents::FIRE)
}

#[derive(Debug, Clone)]
pub struct AlsInputMap {
    pub move_north: Binding,
    pub move_south: Binding,
    pub move_west: Binding,
    pub move_east: Binding,
    pub move_stick: [Binding; 2],
    pub look_mouse: Binding,
    pub look_stick: [Binding; 2],
    pub look_left: Binding,
    pub look_right: Binding,
    pub look_up: Binding,
    pub look_down: Binding,
    pub jump: Vec<Binding>,
    pub crouch: Vec<Binding>,
    pub sprint: Vec<Binding>,
    pub aim: Vec<Binding>,
    pub roll: Vec<Binding>,
    pub walk: Vec<Binding>,
    pub ragdoll: Vec<Binding>,
    pub rotation_mode: Vec<Binding>,
    pub view_mode: Vec<Binding>,
    pub switch_shoulder: Vec<Binding>,
    pub overlay_menu: Vec<Binding>,
    pub slomo: Vec<Binding>,
    pub toggle_ui: Vec<Binding>,
}

impl Default for AlsInputMap {
    fn default() -> Self {
        Self {
            move_north: KeyCode::KeyW.into(),
            move_south: KeyCode::KeyS.into(),
            move_west: KeyCode::KeyA.into(),
            move_east: KeyCode::KeyD.into(),
            move_stick: [
                GamepadAxis::LeftStickX.into(),
                GamepadAxis::LeftStickY.into(),
            ],
            look_mouse: Binding::mouse_motion(),
            look_stick: [
                GamepadAxis::RightStickX.into(),
                GamepadAxis::RightStickY.into(),
            ],
            look_left: KeyCode::ArrowLeft.into(),
            look_right: KeyCode::ArrowRight.into(),
            look_up: KeyCode::ArrowUp.into(),
            look_down: KeyCode::ArrowDown.into(),
            jump: vec![KeyCode::Space.into(), GamepadButton::South.into()],
            crouch: vec![KeyCode::ControlLeft.into(), GamepadButton::LeftThumb.into()],
            sprint: vec![KeyCode::ShiftLeft.into(), GamepadButton::East.into()],
            aim: vec![MouseButton::Right.into(), GamepadButton::LeftTrigger.into()],
            roll: vec![KeyCode::ShiftLeft.into(), GamepadButton::East.into()],
            walk: vec![KeyCode::AltLeft.into(), GamepadButton::RightTrigger.into()],
            ragdoll: vec![KeyCode::KeyX.into(), GamepadButton::West.into()],
            rotation_mode: vec![KeyCode::Digit1.into(), GamepadButton::DPadLeft.into()],
            view_mode: vec![KeyCode::Digit2.into(), GamepadButton::DPadRight.into()],
            switch_shoulder: vec![KeyCode::KeyC.into(), GamepadButton::RightThumb.into()],
            overlay_menu: vec![KeyCode::KeyQ.into(), GamepadButton::North.into()],
            slomo: vec![KeyCode::KeyZ.into(), GamepadButton::Start.into()],
            toggle_ui: vec![KeyCode::Tab.into(), GamepadButton::Select.into()],
        }
    }
}

pub fn spawn_als_actions(cmds: &mut Commands, player: Entity, map: &AlsInputMap) {
    let debug_shift = cmds
        .spawn((
            Action::<DebugShiftAction>::new(),
            Bindings::spawn(vec![
                Binding::from(KeyCode::ShiftLeft),
                Binding::from(KeyCode::ShiftRight),
            ]),
        ))
        .id();
    let mode_bindings = |inputs: Vec<Binding>| {
        Bindings::spawn(SpawnWith(move |bindings: &mut BindingSpawner| {
            for binding in inputs {
                if matches!(
                    binding,
                    Binding::Keyboard {
                        key: KeyCode::Digit1 | KeyCode::Digit2,
                        ..
                    }
                ) {
                    bindings.spawn((binding, BlockBy::single(debug_shift)));
                } else {
                    bindings.spawn(binding);
                }
            }
        }))
    };
    let move_stick = cmds
        .spawn((
            Action::<StickAction>::new(),
            DeadZone::default(),
            Bindings::spawn(Axial::new(map.move_stick[0], map.move_stick[1])),
        ))
        .id();
    let look_stick = cmds
        .spawn((
            Action::<StickAction>::new(),
            DeadZone::default(),
            Bindings::spawn(Axial::new(map.look_stick[0], map.look_stick[1])),
        ))
        .id();
    cmds.entity(player).insert((
        DebugShiftContext,
        ContextPriority::<DebugShiftContext>::new(1),
        Actions::<DebugShiftContext>::spawn(WithRelated([debug_shift].into_iter())),
        OnFoot,
        AlsPlayerCharacter,
        AlsInputCommand::default(),
        Actions::<OnFoot>::spawn((
            WithRelated([move_stick, look_stick].into_iter()),
            bevy::ecs::recursive_spawn!(
                (
                    Action::<MoveAction>::new(),
                    ActionSettings {
                        accumulation: Accumulation::MaxAbs,
                        ..default()
                    },
                    Bindings::spawn((
                        Spawn((Binding::None, StickValue(move_stick))),
                        Spawn(map.move_east),
                        Spawn((map.move_south, SwizzleAxis::YXZ, Negate::all())),
                        Spawn((map.move_west, Negate::all())),
                        Spawn((map.move_north, SwizzleAxis::YXZ)),
                    )),
                ),
                (
                    Action::<LookMouseAction>::new(),
                    Bindings::spawn(vec![map.look_mouse]),
                ),
                (
                    Action::<LookAction>::new(),
                    ActionSettings {
                        accumulation: Accumulation::MaxAbs,
                        ..default()
                    },
                    Bindings::spawn((
                        Spawn((Binding::None, StickValue(look_stick))),
                        Spawn(map.look_right),
                        Spawn((map.look_down, SwizzleAxis::YXZ)),
                        Spawn((map.look_left, Negate::all())),
                        Spawn((map.look_up, SwizzleAxis::YXZ, Negate::all())),
                    )),
                ),
                (
                    Action::<JumpAction>::new(),
                    EIPress::default(),
                    EIRelease::default(),
                    GamePauseBlocker::default(),
                    Bindings::spawn(map.jump.clone()),
                ),
                (
                    Action::<CrouchAction>::new(),
                    EIPress::default(),
                    Bindings::spawn(map.crouch.clone()),
                ),
                (
                    Action::<SprintAction>::new(),
                    Hold::new(SPRINT_HOLD_SECS).one_shot(true),
                    EIRelease::default(),
                    GamePauseBlocker::default(),
                    Bindings::spawn(map.sprint.clone()),
                ),
                (
                    Action::<AimAction>::new(),
                    EIPress::default(),
                    EIRelease::default(),
                    GamePauseBlocker::default(),
                    Bindings::spawn(map.aim.clone()),
                ),
                (
                    Action::<RollAction>::new(),
                    Tap::new(ROLL_TAP_SECS),
                    GamePauseBlocker::default(),
                    Bindings::spawn(map.roll.clone()),
                ),
                (
                    Action::<WalkAction>::new(),
                    EIPress::default(),
                    Bindings::spawn(map.walk.clone()),
                ),
                (
                    Action::<RagdollAction>::new(),
                    EIPress::default(),
                    Bindings::spawn(map.ragdoll.clone()),
                ),
                (
                    Action::<RotationModeAction>::new(),
                    EIPress::default(),
                    mode_bindings(map.rotation_mode.clone()),
                ),
                (
                    Action::<ViewModeAction>::new(),
                    EIPress::default(),
                    mode_bindings(map.view_mode.clone()),
                ),
                (
                    Action::<SwitchShoulderAction>::new(),
                    EIPress::default(),
                    Bindings::spawn(map.switch_shoulder.clone()),
                ),
                (
                    Action::<OverlayMenuAction>::new(),
                    Down::default(),
                    Bindings::spawn(map.overlay_menu.clone()),
                ),
                (
                    Action::<NextOverlayModeAction>::new(),
                    Down::default(),
                    Clamp::pos(),
                    bindings![
                        GamepadButton::DPadUp,
                        (Binding::mouse_wheel(), SwizzleAxis::YXZ),
                    ],
                ),
                (
                    Action::<PreviousOverlayModeAction>::new(),
                    Down::default(),
                    Clamp::pos(),
                    bindings![
                        GamepadButton::DPadDown,
                        (Binding::mouse_wheel(), SwizzleAxis::YXZ, Negate::x()),
                    ],
                ),
                (
                    Action::<SlomoAction>::new(),
                    EIPress::default(),
                    Bindings::spawn(map.slomo.clone()),
                ),
                (
                    Action::<ToggleUiAction>::new(),
                    EIPress::default(),
                    Bindings::spawn(map.toggle_ui.clone()),
                )
            ),
        )),
    ));
}

#[allow(clippy::too_many_arguments)]
pub fn als_input_bridge_system(
    virtual_time: Res<Time<Virtual>>,
    mut receivers: Query<(&mut AlsInputCommand, &Actions<OnFoot>)>,
    moves: Query<&Action<MoveAction>>,
    mouse_looks: Query<&Action<LookMouseAction>>,
    looks: Query<&Action<LookAction>>,
    jumps: Query<(&Action<JumpAction>, &ActionEvents)>,
    sprints: Query<(&Action<SprintAction>, &ActionEvents)>,
    aims: Query<(&Action<AimAction>, &ActionEvents)>,
    crouches: Query<&ActionEvents, With<Action<CrouchAction>>>,
    rolls: Query<&ActionEvents, With<Action<RollAction>>>,
    walks: Query<&ActionEvents, With<Action<WalkAction>>>,
    ragdolls: Query<&ActionEvents, With<Action<RagdollAction>>>,
    rotation_modes: Query<&ActionEvents, With<Action<RotationModeAction>>>,
    view_modes: Query<&ActionEvents, With<Action<ViewModeAction>>>,
    switch_shoulders: Query<&ActionEvents, With<Action<SwitchShoulderAction>>>,

) {
    for (mut cmd, actions) in &mut receivers {
        let saved_aim = cmd.aim_held;
        let saved_sprint = cmd.sprint_held;
        cmd.reset();
        cmd.aim_held = saved_aim;
        cmd.sprint_held = saved_sprint;

        if let Some(mv) = moves.iter_many(actions).find_map(Result::ok) {
            cmd.move_dir = **mv;
        }

        if let Some(mouse) = mouse_looks.iter_many(actions).find_map(Result::ok) {
            cmd.look_delta = **mouse;
        }

        if let Some(lk) = looks.iter_many(actions).find_map(Result::ok) {
            cmd.look_rate = **lk;
        }

        if let Some((value, events)) = jumps.iter_many(actions).find_map(Result::ok) {
            cmd.jump_held = **value;
            cmd.jump_pressed =
                **value && events.intersects(ActionEvents::FIRE | ActionEvents::CANCEL);
        }

        if let Some((value, events)) = sprints.iter_many(actions).find_map(Result::ok) {
            if events.intersects(ActionEvents::FIRE | ActionEvents::CANCEL) {
                cmd.sprint_held = **value;
            }
        }

        if let Some((value, events)) = aims.iter_many(actions).find_map(Result::ok) {
            if events.intersects(ActionEvents::FIRE | ActionEvents::CANCEL) {
                cmd.aim_held = **value;
            }
        }

        if let Some(events) = crouches.iter_many(actions).find_map(Result::ok) {
            cmd.crouch_pressed = !virtual_time.is_paused() && events.contains(ActionEvents::START);
        }

        if let Some(events) = rolls.iter_many(actions).find_map(Result::ok) {
            cmd.roll_pressed = roll_pressed_from_events(*events);
        }

        if let Some(events) = walks.iter_many(actions).find_map(Result::ok) {
            cmd.walk_pressed = !virtual_time.is_paused() && events.contains(ActionEvents::START);
        }

        if let Some(events) = ragdolls.iter_many(actions).find_map(Result::ok) {
            cmd.ragdoll_pressed = !virtual_time.is_paused() && events.contains(ActionEvents::START);
        }

        if let Some(events) = rotation_modes.iter_many(actions).find_map(Result::ok) {
            cmd.rotation_mode_pressed =
                !virtual_time.is_paused() && events.contains(ActionEvents::START);
        }

        if let Some(events) = view_modes.iter_many(actions).find_map(Result::ok) {
            cmd.view_mode_pressed =
                !virtual_time.is_paused() && events.contains(ActionEvents::START);
        }

        if let Some(events) = switch_shoulders.iter_many(actions).find_map(Result::ok) {
            cmd.switch_shoulder_pressed =
                !virtual_time.is_paused() && events.contains(ActionEvents::START);
        }
    }
}

/// Default keyboard, mouse, and gamepad adapter for the Playground.
pub struct AlsInputPlugin;

impl Plugin for AlsInputPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<EnhancedInputPlugin>() {
            app.add_plugins(EnhancedInputPlugin);
        }
        app.add_input_context::<OnFoot>()
            .add_input_context::<DebugShiftContext>()
            .add_input_modifier::<StickValue>()
            .add_input_condition::<GamePauseBlocker>()
            .add_systems(
                PreUpdate,
                sync_game_pause_blockers.before(EnhancedInputSystems::Update),
            )
            .add_observer(
                |gamepad: On<Add<Gamepad>>, mut settings: Query<&mut GamepadSettings>| {
                    let mut settings = settings.get_mut(gamepad.entity).unwrap();
                    for axis in [
                        GamepadAxis::LeftStickX,
                        GamepadAxis::LeftStickY,
                        GamepadAxis::RightStickX,
                        GamepadAxis::RightStickY,
                    ] {
                        let mut axis_settings = settings.get_axis_settings(axis).clone();
                        axis_settings.set_threshold(0.0);
                        settings.axis_settings.insert(axis, axis_settings);
                    }
                },
            )
            .add_systems(
                PreUpdate,
                als_input_bridge_system.after(EnhancedInputSystems::Apply),
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::{MinimalPlugins, ecs::system::RunSystemOnce};

    #[test]
    fn default_command_is_zeroed() {
        let cmd = AlsInputCommand::default();
        assert_eq!(cmd.move_dir, Vec2::ZERO);
        assert_eq!(cmd.look_delta, Vec2::ZERO);
        assert_eq!(cmd.look_rate, Vec2::ZERO);
        assert!(!cmd.jump_held);
        assert!(!cmd.jump_pressed);
        assert!(!cmd.crouch_pressed);
        assert!(!cmd.roll_pressed);
        assert!(!cmd.sprint_held);
        assert!(!cmd.aim_held);
        assert!(!cmd.walk_pressed);
        assert!(!cmd.ragdoll_pressed);
        assert!(!cmd.rotation_mode_pressed);
        assert!(!cmd.view_mode_pressed);
        assert!(!cmd.switch_shoulder_pressed);
    }

    #[test]
    fn reset_clears_continuous_and_edge_fields() {
        let mut cmd = AlsInputCommand {
            move_dir: Vec2::ONE,
            look_delta: Vec2::NEG_ONE,
            look_rate: Vec2::ONE,
            jump_held: true,
            jump_pressed: true,
            crouch_pressed: true,
            roll_pressed: true,
            sprint_held: true,
            aim_held: true,
            walk_pressed: true,
            ragdoll_pressed: true,
            rotation_mode_pressed: true,
            view_mode_pressed: true,
            switch_shoulder_pressed: true,
        };
        cmd.reset();
        assert_eq!(
            cmd,
            AlsInputCommand::default(),
            "reset should produce the default value"
        );
    }

    #[test]
    fn roll_requires_the_tap_fire_event_not_the_initial_press_event() {
        let press = ActionEvents::START | ActionEvents::ONGOING;
        let release = ActionEvents::FIRE;
        assert!(!roll_pressed_from_events(press));
        assert!(roll_pressed_from_events(release));
    }

    #[test]
    fn bridge_tolerates_missing_actions_and_entities() {
        let mut world = World::new();
        world.insert_resource(ButtonInput::<KeyCode>::default());
        world.insert_resource(Time::<Virtual>::default());

        world.run_system_once(als_input_bridge_system).unwrap();

        let player = world
            .spawn((
                AlsInputCommand {
                    move_dir: Vec2::ONE,
                    jump_pressed: true,
                    sprint_held: true,
                    ..Default::default()
                },
                Actions::<OnFoot>::default(),
            ))
            .id();
        world.run_system_once(als_input_bridge_system).unwrap();

        let cmd = world.get::<AlsInputCommand>(player).unwrap();
        assert_eq!(
            *cmd,
            AlsInputCommand {
                sprint_held: true,
                ..default()
            },
            "Source saved intent has no callback when actions are absent"
        );
    }

    #[test]
    fn spawn_creates_eighteen_actions_and_two_stick_groups_with_bindings() {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            bevy::input::InputPlugin,
            EnhancedInputPlugin,
            AlsInputPlugin,
        ));
        app.finish();

        let player = app.world_mut().spawn_empty().id();
        let mut cmds = app.world_mut().commands();
        spawn_als_actions(&mut cmds, player, &AlsInputMap::default());
        app.update();

        let world = app.world_mut();
        assert_eq!(world.get::<Actions<OnFoot>>(player).unwrap().len(), 20);

        for action in world.get::<Actions<OnFoot>>(player).unwrap().iter() {
            let bindings = world.get::<Bindings>(action).unwrap();
            assert!(
                !bindings.is_empty(),
                "action {action:?} should have bindings"
            );
        }

        let (sprint_one_shot, sprint_secs) = {
            let sprint = world
                .query_filtered::<&Hold, With<Action<SprintAction>>>()
                .single(world)
                .unwrap();
            (sprint.one_shot, sprint.timer().duration().as_secs_f32())
        };
        let roll_secs = {
            let roll = world
                .query_filtered::<&Tap, With<Action<RollAction>>>()
                .single(world)
                .unwrap();
            roll.timer().duration().as_secs_f32()
        };
        let next_actuation = {
            let next_overlay = world
                .query_filtered::<&Down, With<Action<NextOverlayModeAction>>>()
                .single(world)
                .unwrap();
            next_overlay.actuation
        };
        let previous_actuation = {
            let previous_overlay = world
                .query_filtered::<&Down, With<Action<PreviousOverlayModeAction>>>()
                .single(world)
                .unwrap();
            previous_overlay.actuation
        };
        assert!(sprint_one_shot);
        assert!((sprint_secs - SPRINT_HOLD_SECS).abs() < 1e-6);
        assert!((roll_secs - ROLL_TAP_SECS).abs() < 1e-6);
        assert!((next_actuation - 0.5).abs() < f32::EPSILON);
        assert!((previous_actuation - 0.5).abs() < f32::EPSILON);

        let mut cmds = app.world_mut().commands();
        spawn_als_actions(&mut cmds, player, &AlsInputMap::default());
        app.update();
    }

    #[test]
    fn default_input_map_matches_the_als_context() {
        let map = AlsInputMap::default();
        assert_eq!(map.move_north, KeyCode::KeyW.into());
        assert_eq!(map.move_south, KeyCode::KeyS.into());
        assert_eq!(map.move_west, KeyCode::KeyA.into());
        assert_eq!(map.move_east, KeyCode::KeyD.into());
        assert_eq!(
            map.move_stick,
            [
                GamepadAxis::LeftStickX.into(),
                GamepadAxis::LeftStickY.into()
            ]
        );
        assert_eq!(map.look_mouse, Binding::mouse_motion());
        assert_eq!(
            map.look_stick,
            [
                GamepadAxis::RightStickX.into(),
                GamepadAxis::RightStickY.into()
            ]
        );
        assert_eq!(map.look_left, KeyCode::ArrowLeft.into());
        assert_eq!(map.look_right, KeyCode::ArrowRight.into());
        assert_eq!(map.look_up, KeyCode::ArrowUp.into());
        assert_eq!(map.look_down, KeyCode::ArrowDown.into());
        assert_eq!(
            map.jump,
            vec![KeyCode::Space.into(), GamepadButton::South.into()]
        );
        assert_eq!(
            map.crouch,
            vec![KeyCode::ControlLeft.into(), GamepadButton::LeftThumb.into()]
        );
        assert_eq!(
            map.sprint,
            vec![KeyCode::ShiftLeft.into(), GamepadButton::East.into()]
        );
        assert_eq!(
            map.aim,
            vec![MouseButton::Right.into(), GamepadButton::LeftTrigger.into()]
        );
        assert_eq!(
            map.roll,
            vec![KeyCode::ShiftLeft.into(), GamepadButton::East.into()]
        );
        assert_eq!(
            map.walk,
            vec![KeyCode::AltLeft.into(), GamepadButton::RightTrigger.into()]
        );
        assert_eq!(
            map.ragdoll,
            vec![KeyCode::KeyX.into(), GamepadButton::West.into()]
        );
        assert_eq!(
            map.rotation_mode,
            vec![KeyCode::Digit1.into(), GamepadButton::DPadLeft.into()]
        );
        assert_eq!(
            map.view_mode,
            vec![KeyCode::Digit2.into(), GamepadButton::DPadRight.into()]
        );
        assert_eq!(
            map.switch_shoulder,
            vec![KeyCode::KeyC.into(), GamepadButton::RightThumb.into()]
        );
        assert_eq!(
            map.overlay_menu,
            vec![KeyCode::KeyQ.into(), GamepadButton::North.into()]
        );
        assert_eq!(
            map.slomo,
            vec![KeyCode::KeyZ.into(), GamepadButton::Start.into()]
        );
        assert_eq!(
            map.toggle_ui,
            vec![KeyCode::Tab.into(), GamepadButton::Select.into()]
        );
    }

    #[test]
    fn shift_suppresses_digit_key_actions() {
        for shift in [KeyCode::ShiftLeft, KeyCode::ShiftRight] {
            let mut app = App::new();
            app.add_plugins((
                MinimalPlugins,
                bevy::input::InputPlugin,
                EnhancedInputPlugin,
                AlsInputPlugin,
            ));
            app.finish();
            let player = app.world_mut().spawn_empty().id();
            spawn_als_actions(
                &mut app.world_mut().commands(),
                player,
                &AlsInputMap::default(),
            );
            app.update();
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keys.press(shift);
            keys.press(KeyCode::Digit1);
            keys.press(KeyCode::Digit2);
            app.update();
            let command = app.world().get::<AlsInputCommand>(player).unwrap();
            assert!(
                !command.rotation_mode_pressed && !command.view_mode_pressed,
                "User-approved Shift debug digits must not actuate mode bindings"
            );
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .release_all();
            app.update();
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keys.press(KeyCode::Digit1);
            keys.press(KeyCode::Digit2);
            app.update();
            let command = app.world().get::<AlsInputCommand>(player).unwrap();
            assert!(
                command.rotation_mode_pressed && command.view_mode_pressed,
                "Source bare mode digit bindings must remain active"
            );
        }
    }
    #[test]
    fn movement_keys_remain_live_while_holding_and_changing_direction() {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            bevy::input::InputPlugin,
            EnhancedInputPlugin,
            AlsInputPlugin,
        ));
        app.finish();
        let player = app.world_mut().spawn_empty().id();
        spawn_als_actions(
            &mut app.world_mut().commands(),
            player,
            &AlsInputMap::default(),
        );
        app.update();
        for (frame, (pressed, expected)) in [
            (vec![KeyCode::KeyW], Vec2::Y),
            (vec![KeyCode::KeyW], Vec2::Y),
            (vec![KeyCode::KeyW, KeyCode::KeyD], Vec2::ONE),
            (vec![KeyCode::KeyD], Vec2::X),
            (vec![KeyCode::KeyS], Vec2::NEG_Y),
            (vec![KeyCode::KeyA], Vec2::NEG_X),
            (vec![], Vec2::ZERO),
        ]
        .into_iter()
        .enumerate()
        {
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keys.release_all();
            for key in pressed {
                keys.press(key);
            }
            app.update();
            let actual = app.world().get::<AlsInputCommand>(player).unwrap().move_dir;
            assert_eq!(actual, expected, "movement input frame={frame}");
        }
    }

    #[test]
    fn full_chain_from_action_mock_to_command() {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            bevy::input::InputPlugin,
            EnhancedInputPlugin,
            AlsInputPlugin,
        ));
        app.finish();

        let player = app.world_mut().spawn_empty().id();
        let mut cmds = app.world_mut().commands();
        spawn_als_actions(&mut cmds, player, &AlsInputMap::default());
        app.update();

        app.world_mut()
            .commands()
            .entity(player)
            .mock_once::<OnFoot, JumpAction>(TriggerState::Fired, true)
            .mock_once::<OnFoot, CrouchAction>(TriggerState::Fired, true)
            .mock_once::<OnFoot, RollAction>(TriggerState::Fired, true)
            .mock::<OnFoot, SprintAction>(TriggerState::Fired, true, MockSpan::Updates(50))
            .mock::<OnFoot, AimAction>(TriggerState::Fired, true, MockSpan::Updates(50))
            .mock::<OnFoot, MoveAction>(
                TriggerState::Fired,
                Vec2::new(1.0, 0.5),
                MockSpan::Updates(50),
            )
            .mock::<OnFoot, LookMouseAction>(
                TriggerState::Fired,
                Vec2::new(4.0, -3.0),
                MockSpan::Updates(50),
            )
            .mock::<OnFoot, LookAction>(
                TriggerState::Fired,
                Vec2::new(0.5, -0.25),
                MockSpan::Updates(50),
            )
            .mock::<OnFoot, OverlayMenuAction>(TriggerState::Fired, true, MockSpan::Updates(50))
            .mock_once::<OnFoot, NextOverlayModeAction>(TriggerState::Fired, 1.0)
            .mock_once::<OnFoot, PreviousOverlayModeAction>(TriggerState::Fired, 1.0)
            .mock_once::<OnFoot, SlomoAction>(TriggerState::Fired, true)
            .mock_once::<OnFoot, ToggleUiAction>(TriggerState::Fired, true);

        app.update();
        let cmd = *app.world().get::<AlsInputCommand>(player).unwrap();
        assert!(cmd.jump_pressed && cmd.jump_held, "jump pressed and held");
        assert!(cmd.crouch_pressed, "crouch toggle pressed");
        assert!(cmd.roll_pressed, "roll pressed");
        assert!(cmd.sprint_held, "sprint held");
        assert!(cmd.aim_held, "aim held");
        assert_eq!(cmd.look_delta, Vec2::new(4.0, -3.0));
        assert_eq!(cmd.look_rate, Vec2::new(0.5, -0.25));
        assert!(
            (cmd.move_dir - Vec2::new(1.0, 0.5)).length() < 1e-5,
            "move_dir should equal mocked value, got {:?}",
            cmd.move_dir
        );

        app.update();
        let cmd = *app.world().get::<AlsInputCommand>(player).unwrap();
        assert!(!cmd.jump_pressed);
        assert!(!cmd.jump_held);
        assert!(!cmd.crouch_pressed);
        assert!(!cmd.roll_pressed);
        assert!(cmd.sprint_held);
        assert!(cmd.aim_held);
        assert_eq!(cmd.look_delta, Vec2::new(4.0, -3.0));
        assert_eq!(cmd.look_rate, Vec2::new(0.5, -0.25));
        assert!(
            (cmd.move_dir - Vec2::new(1.0, 0.5)).length() < 1e-5,
            "move_dir should persist within hold span"
        );

        app.world_mut()
            .commands()
            .entity(player)
            .mock_once::<OnFoot, AimAction>(TriggerState::Fired, false)
            .mock_once::<OnFoot, SprintAction>(TriggerState::Fired, false);
        app.update();
        let actions = app
            .world()
            .get::<Actions<OnFoot>>(player)
            .unwrap()
            .iter()
            .collect::<Vec<_>>();
        for action in actions {
            if let Some(mut mock) = app.world_mut().get_mut::<ActionMock>(action) {
                mock.enabled = false;
            }
        }
        app.update();
        let cmd = *app.world().get::<AlsInputCommand>(player).unwrap();
        assert_eq!(cmd, AlsInputCommand::default());
    }

    #[test]
    fn mouse_motion_reaches_source_degree_angles_through_the_actual_action_bridge() {
        use bevy_als::als::character::AlsCharacter;
        use bevy_als::als::motion::AlsViewYaw;
        use bevy_als::als::view::{AlsViewInputPlugin, AlsViewPitch};
        use bevy::input::{InputPlugin, mouse::MouseMotion};
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            InputPlugin,
            EnhancedInputPlugin,
            AlsInputPlugin,
            AlsViewInputPlugin,
        ));
        app.finish();
        let character = app
            .world_mut()
            .spawn((AlsCharacter, AlsViewYaw(0.0), AlsViewPitch(0.0)))
            .id();
        spawn_als_actions(
            &mut app.world_mut().commands(),
            character,
            &AlsInputMap::default(),
        );
        app.update();
        app.world_mut().write_message(MouseMotion {
            delta: Vec2::new(12.0, -8.0),
        });
        app.update();
        assert_eq!(
            app.world()
                .get::<AlsInputCommand>(character)
                .unwrap()
                .look_delta,
            Vec2::new(12.0, -8.0)
        );
        assert!(
            (app.world().get::<AlsViewYaw>(character).unwrap().0 + 12.0_f32.to_radians()).abs()
                < 1e-6
        );
        assert!(
            (app.world().get::<AlsViewPitch>(character).unwrap().0 - 8.0_f32.to_radians()).abs()
                < 1e-6
        );
        app.update();
        assert_eq!(
            app.world()
                .get::<AlsInputCommand>(character)
                .unwrap()
                .look_delta,
            Vec2::ZERO
        );
        assert!(
            (app.world().get::<AlsViewYaw>(character).unwrap().0 + 12.0_f32.to_radians()).abs()
                < 1e-6
        );
    }

    #[test]
    fn mouse_pitch_preserves_source_camera_limits_and_winding() {
        use bevy_als::als::character::AlsCharacter;
        use bevy_als::als::motion::AlsViewYaw;
        use bevy_als::als::view::{AlsViewInputPlugin, AlsViewPitch};
        use bevy::input::{InputPlugin, mouse::MouseMotion};
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            InputPlugin,
            EnhancedInputPlugin,
            AlsInputPlugin,
            AlsViewInputPlugin,
        ));
        app.finish();
        let character = app
            .world_mut()
            .spawn((AlsCharacter, AlsViewYaw(0.0), AlsViewPitch(0.0)))
            .id();
        spawn_als_actions(
            &mut app.world_mut().commands(),
            character,
            &AlsInputMap::default(),
        );
        app.update();
        for (degrees, expected) in [
            (87.0, 87.0_f32),
            (-87.0, -87.0),
            (89.9, 89.9),
            (-89.9, -89.9),
            (90.0, 89.9),
            (-90.0, -89.9),
            (179.0, 89.9),
            (-179.0, -89.9),
            (180.0, 89.9),
            (-180.0, 89.9),
            (200.0, -89.9),
            (-200.0, 89.9),
            (360.0, 0.0),
            (400.0, 40.0),
            (-400.0, -40.0),
            (720.0, 0.0),
        ] {
            app.world_mut()
                .get_mut::<AlsViewPitch>(character)
                .unwrap()
                .0 = 0.0;
            app.world_mut().write_message(MouseMotion {
                delta: Vec2::new(0.0, -degrees),
            });
            app.update();
            let actual = app.world().get::<AlsViewPitch>(character).unwrap().0;
            assert!(
                (actual - expected.to_radians()).abs() < 1e-6,
                "Source pitch for {degrees} degrees: expected {expected}, actual {}",
                actual.to_degrees()
            );
        }
        app.update();
        assert_eq!(
            app.world()
                .get::<AlsInputCommand>(character)
                .unwrap()
                .look_delta,
            Vec2::ZERO
        );
    }

    #[test]
    fn opposing_keys_preserve_source_maximum_magnitude_and_mapping_order() {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            bevy::input::InputPlugin,
            EnhancedInputPlugin,
            AlsInputPlugin,
        ));
        app.finish();
        let player = app.world_mut().spawn_empty().id();
        spawn_als_actions(
            &mut app.world_mut().commands(),
            player,
            &AlsInputMap::default(),
        );
        app.update();
        for (keys, movement, look) in [
            (vec![KeyCode::KeyW, KeyCode::KeyS], Vec2::NEG_Y, Vec2::ZERO),
            (vec![KeyCode::KeyA, KeyCode::KeyD], Vec2::X, Vec2::ZERO),
            (
                vec![KeyCode::KeyW, KeyCode::KeyA, KeyCode::KeyS, KeyCode::KeyD],
                Vec2::new(1.0, -1.0),
                Vec2::ZERO,
            ),
            (
                vec![KeyCode::ArrowUp, KeyCode::ArrowDown],
                Vec2::ZERO,
                Vec2::Y,
            ),
            (
                vec![KeyCode::ArrowLeft, KeyCode::ArrowRight],
                Vec2::ZERO,
                Vec2::X,
            ),
            (
                vec![
                    KeyCode::ArrowUp,
                    KeyCode::ArrowLeft,
                    KeyCode::ArrowDown,
                    KeyCode::ArrowRight,
                ],
                Vec2::ZERO,
                Vec2::ONE,
            ),
            (vec![], Vec2::ZERO, Vec2::ZERO),
        ] {
            let mut pressed = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            pressed.release_all();
            for key in &keys {
                pressed.press(*key);
            }
            app.update();
            let command = app.world().get::<AlsInputCommand>(player).unwrap();
            assert_eq!(
                command.move_dir, movement,
                "Source movement priority for {keys:?}"
            );
            assert_eq!(command.look_rate, look, "Source look priority for {keys:?}");
        }
    }

    #[test]
    fn gamepad_and_keyboard_preserve_source_component_winners_and_ties() {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            bevy::input::InputPlugin,
            EnhancedInputPlugin,
            AlsInputPlugin,
        ));
        app.finish();
        let player = app.world_mut().spawn_empty().id();
        let gamepad = app.world_mut().spawn(Gamepad::default()).id();
        spawn_als_actions(
            &mut app.world_mut().commands(),
            player,
            &AlsInputMap::default(),
        );
        app.update();
        for (stick, expected) in [(-0.5, Vec2::Y), (-1.0, Vec2::NEG_Y), (0.5, Vec2::Y)] {
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keys.release_all();
            keys.press(KeyCode::KeyW);
            keys.press(KeyCode::ArrowUp);
            let mut device = app.world_mut().get_mut::<Gamepad>(gamepad).unwrap();
            device.analog_mut().set(GamepadAxis::LeftStickY, stick);
            device.analog_mut().set(GamepadAxis::RightStickY, -stick);
            app.update();
            let command = app.world().get::<AlsInputCommand>(player).unwrap();
            assert_eq!(
                command.move_dir, expected,
                "Source move component winner for stick={stick}"
            );
            assert_eq!(
                command.look_rate, -expected,
                "Source look component winner for stick={stick}"
            );
        }
    }

    #[test]
    fn stick_dead_zone_uses_the_source_two_dimensional_magnitude() {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            bevy::input::InputPlugin,
            EnhancedInputPlugin,
            AlsInputPlugin,
        ));
        app.finish();
        let player = app.world_mut().spawn_empty().id();
        let gamepad = app.world_mut().spawn(Gamepad::default()).id();
        spawn_als_actions(
            &mut app.world_mut().commands(),
            player,
            &AlsInputMap::default(),
        );
        app.update();
        for (raw, expected) in [
            (Vec2::splat(0.15), Vec2::splat(0.010723304)),
            (Vec2::splat(0.5), Vec2::splat(0.4482233)),
            (Vec2::ONE, Vec2::splat(std::f32::consts::FRAC_1_SQRT_2)),
            (Vec2::new(0.3, 0.4), Vec2::new(0.225, 0.3)),
            (Vec2::new(-0.3, 0.4), Vec2::new(-0.225, 0.3)),
            (Vec2::new(0.5, 0.0), Vec2::new(0.375, 0.0)),
            (Vec2::new(0.0, -0.5), Vec2::new(0.0, -0.375)),
            (Vec2::new(0.2, 0.0), Vec2::ZERO),
            (Vec2::splat(0.1), Vec2::ZERO),
            (Vec2::ZERO, Vec2::ZERO),
        ] {
            let mut device = app.world_mut().get_mut::<Gamepad>(gamepad).unwrap();
            device.analog_mut().set(GamepadAxis::LeftStickX, raw.x);
            device.analog_mut().set(GamepadAxis::LeftStickY, raw.y);
            device.analog_mut().set(GamepadAxis::RightStickX, raw.x);
            device.analog_mut().set(GamepadAxis::RightStickY, raw.y);
            app.update();
            let command = app.world().get::<AlsInputCommand>(player).unwrap();
            assert!(
                (command.move_dir - expected).length() < 1e-6,
                "Source radial move for {raw}: expected {expected}, actual {}",
                command.move_dir
            );
            assert!(
                (command.look_rate - expected).length() < 1e-6,
                "Source radial look magnitude for {raw}: expected {expected}, actual {}",
                command.look_rate
            );
        }
        let mut device = app.world_mut().get_mut::<Gamepad>(gamepad).unwrap();
        device.analog_mut().set(GamepadAxis::LeftStickX, 0.3);
        device.analog_mut().set(GamepadAxis::LeftStickY, 0.4);
        device.analog_mut().set(GamepadAxis::RightStickX, 0.3);
        device.analog_mut().set(GamepadAxis::RightStickY, 0.4);
        let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        keys.press(KeyCode::KeyD);
        keys.press(KeyCode::ArrowLeft);
        app.update();
        let command = app.world().get::<AlsInputCommand>(player).unwrap();
        assert!(
            (command.move_dir - Vec2::new(1.0, 0.3)).length() < 1e-6,
            "Source keyboard competition occurs after the radial mapping: {}",
            command.move_dir
        );
        assert!(
            (command.look_rate - Vec2::new(-1.0, 0.3)).length() < 1e-6,
            "Source look keyboard competition occurs after the radial mapping: {}",
            command.look_rate
        );
    }

    #[test]
    fn right_stick_preserves_source_final_view_direction_without_extra_negation() {
        use bevy_als::als::character::AlsCharacter;
        use bevy_als::als::motion::AlsViewYaw;
        use bevy_als::als::view::{AlsViewInputPlugin, AlsViewPitch};
        use bevy::time::TimeUpdateStrategy;
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            bevy::input::InputPlugin,
            EnhancedInputPlugin,
            AlsInputPlugin,
            AlsViewInputPlugin,
        ))
        .insert_resource(TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f32(0.1),
        ));
        app.finish();
        let character = app
            .world_mut()
            .spawn((AlsCharacter, AlsViewYaw(0.0), AlsViewPitch(0.0)))
            .id();
        let gamepad = app.world_mut().spawn(Gamepad::default()).id();
        spawn_als_actions(
            &mut app.world_mut().commands(),
            character,
            &AlsInputMap::default(),
        );
        app.update();
        for (raw, source_degrees) in [
            (Vec2::new(0.0, 0.5), Vec2::new(0.0, -3.375)),
            (Vec2::new(0.0, -0.5), Vec2::new(0.0, 3.375)),
            (Vec2::new(0.5, 0.0), Vec2::new(9.0, 0.0)),
            (Vec2::new(-0.5, 0.0), Vec2::new(-9.0, 0.0)),
            (Vec2::new(0.3, 0.4), Vec2::new(5.4, -2.7)),
            (Vec2::new(-0.3, -0.4), Vec2::new(-5.4, 2.7)),
        ] {
            app.world_mut()
                .entity_mut(character)
                .insert((AlsViewYaw(0.0), AlsViewPitch(0.0)));
            let mut device = app.world_mut().get_mut::<Gamepad>(gamepad).unwrap();
            device.analog_mut().set(GamepadAxis::RightStickX, raw.x);
            device.analog_mut().set(GamepadAxis::RightStickY, raw.y);
            app.update();
            let yaw = app.world().get::<AlsViewYaw>(character).unwrap().0;
            let pitch = app.world().get::<AlsViewPitch>(character).unwrap().0;
            let source_yaw = source_degrees.x.to_radians();
            let source_pitch = source_degrees.y.to_radians();
            assert!(
                (yaw + source_yaw).abs() < 1e-6 && (pitch - source_pitch).abs() < 1e-6,
                "Source final stick angles for {raw}: actual yaw={yaw}, pitch={pitch}"
            );
            let expected = Vec3::new(
                source_pitch.cos() * source_yaw.sin(),
                source_pitch.sin(),
                -source_pitch.cos() * source_yaw.cos(),
            );
            let actual = Quat::from_euler(EulerRot::YXZ, yaw, pitch, 0.0) * Vec3::NEG_Z;
            assert!(
                (actual - expected).length() < 1e-6,
                "Source physical view direction for {raw}: expected {expected}, actual {actual}"
            );
        }
    }

    #[test]
    fn raw_stick_events_preserve_source_changes_below_platform_filter_thresholds() {
        use bevy::input::gamepad::{RawGamepadAxisChangedEvent, RawGamepadEvent};
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            bevy::input::InputPlugin,
            EnhancedInputPlugin,
            AlsInputPlugin,
        ));
        app.finish();
        let player = app.world_mut().spawn_empty().id();
        let gamepad = app.world_mut().spawn(Gamepad::default()).id();
        spawn_als_actions(
            &mut app.world_mut().commands(),
            player,
            &AlsInputMap::default(),
        );
        app.update();
        for device in [GamepadDevice::Any, GamepadDevice::Single(gamepad)] {
            app.world_mut().entity_mut(player).insert(device);
            for (raw, expected) in [
                (Vec2::new(0.5, 0.0), Vec2::new(0.375, 0.0)),
                (Vec2::new(0.505, 0.0), Vec2::new(0.38125, 0.0)),
                (Vec2::new(0.5, 0.004), Vec2::new(0.375008, 0.003000064)),
                (Vec2::new(0.5, -0.004), Vec2::new(0.375008, -0.003000064)),
                (Vec2::new(0.0, 0.5), Vec2::new(0.0, 0.375)),
                (Vec2::new(0.0, 0.505), Vec2::new(0.0, 0.38125)),
                (Vec2::ZERO, Vec2::ZERO),
            ] {
                for (axis, value) in [
                    (GamepadAxis::LeftStickX, raw.x),
                    (GamepadAxis::LeftStickY, raw.y),
                    (GamepadAxis::RightStickX, raw.x),
                    (GamepadAxis::RightStickY, raw.y),
                ] {
                    app.world_mut().write_message(RawGamepadEvent::Axis(
                        RawGamepadAxisChangedEvent::new(gamepad, axis, value),
                    ));
                }
                app.update();
                let command = app.world().get::<AlsInputCommand>(player).unwrap();
                assert!(
                    (command.move_dir - expected).length() < 1e-6,
                    "Source raw stick move for {raw}: expected {expected}, actual {}",
                    command.move_dir
                );
                assert!(
                    (command.look_rate - expected).length() < 1e-6,
                    "Source raw stick look for {raw}: expected {expected}, actual {}",
                    command.look_rate
                );
            }
        }
        let settings = app.world().get::<GamepadSettings>(gamepad).unwrap();
        let mut expected = bevy::input::gamepad::AxisSettings::default();
        expected.set_threshold(0.0);
        for axis in [
            GamepadAxis::LeftStickX,
            GamepadAxis::LeftStickY,
            GamepadAxis::RightStickX,
            GamepadAxis::RightStickY,
        ] {
            assert_eq!(
                settings.get_axis_settings(axis),
                &expected,
                "Source stick event configuration must change only the threshold for {axis:?}"
            );
        }
        assert_eq!(
            settings.get_axis_settings(GamepadAxis::LeftZ),
            &bevy::input::gamepad::AxisSettings::default()
        );
        let reconnected = app.world_mut().spawn(Gamepad::default()).id();
        assert_eq!(
            app.world()
                .get::<GamepadSettings>(reconnected)
                .unwrap()
                .get_axis_settings(GamepadAxis::RightStickX),
            &expected
        );
    }
    #[test]
    fn sprint_hold_pause_cancel_updates_the_actual_source_desired_gait() {
        use bevy_als::als::character::AlsCharacter;
        use bevy_als::als::gait::AlsDesiredGait;
        use bevy_als::als::state::{Gait, RotationMode, Stance};
        use bevy_als::als::transition::AlsStateRequest;
        use bevy::input::gamepad::{RawGamepadButtonChangedEvent, RawGamepadEvent};
        use bevy::time::TimeUpdateStrategy;
        let cases = [
            vec![
                (false, true, 8, true),
                (true, true, 1, true),
                (true, false, 2, true),
                (false, false, 1, true),
                (false, true, 8, true),
                (false, false, 1, false),
            ],
            vec![
                (true, true, 8, false),
                (false, true, 2, false),
                (false, false, 1, false),
                (false, true, 8, true),
                (false, false, 1, false),
            ],
            vec![
                (false, true, 2, false),
                (true, true, 8, true),
                (false, true, 2, true),
                (false, false, 1, false),
            ],
            vec![
                (false, true, 8, true),
                (true, false, 2, false),
                (false, false, 1, false),
            ],
        ];
        for gamepad_input in [false, true] {
            for speed in [1.0, 0.25] {
                for (case_index, steps) in cases.iter().enumerate() {
                    let mut app = App::new();
                    app.add_plugins((
                        MinimalPlugins,
                        bevy::input::InputPlugin,
                        EnhancedInputPlugin,
                        AlsInputPlugin,
                    ))
                    .insert_resource(TimeUpdateStrategy::ManualDuration(
                        std::time::Duration::from_millis(20),
                    ))
                    .add_systems(
                        Update,
                        bevy_als::als::input_request::input_state_request_system,
                    );
                    app.finish();
                    let player = app
                        .world_mut()
                        .spawn((
                            AlsCharacter,
                            Stance::Standing,
                            RotationMode::ViewDirection,
                            AlsDesiredGait(Gait::Running),
                            AlsStateRequest::default(),
                        ))
                        .id();
                    let gamepad = app.world_mut().spawn(Gamepad::default()).id();
                    spawn_als_actions(
                        &mut app.world_mut().commands(),
                        player,
                        &AlsInputMap::default(),
                    );
                    app.update();
                    app.world_mut()
                        .resource_mut::<Time<Virtual>>()
                        .set_relative_speed(speed);
                    for (step_index, &(paused, held, frames, expected)) in steps.iter().enumerate()
                    {
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
                            app.world()
                                .get::<AlsInputCommand>(player)
                                .unwrap()
                                .sprint_held,
                            expected,
                            "Source saved Sprint intent for device={gamepad_input}, speed={speed}, case={case_index}, step={step_index}"
                        );
                        assert_eq!(
                            app.world().get::<AlsDesiredGait>(player).unwrap().0,
                            if expected {
                                Gait::Sprinting
                            } else {
                                Gait::Running
                            },
                            "Source saved DesiredGait for device={gamepad_input}, speed={speed}, case={case_index}, step={step_index}"
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn shared_sprint_roll_key_preserves_source_tap_after_hold_waiting() {
        use bevy::input::gamepad::{RawGamepadButtonChangedEvent, RawGamepadEvent};
        use bevy::time::TimeUpdateStrategy;
        for gamepad_input in [false, true] {
            for frames in [2, 8, 20] {
                for speed in [1.0, 0.25] {
                    let mut app = App::new();
                    app.add_plugins((
                        MinimalPlugins,
                        bevy::input::InputPlugin,
                        EnhancedInputPlugin,
                        AlsInputPlugin,
                    ))
                    .insert_resource(TimeUpdateStrategy::ManualDuration(
                        std::time::Duration::from_millis(20),
                    ));
                    app.finish();
                    let player = app.world_mut().spawn_empty().id();
                    let gamepad = app.world_mut().spawn(Gamepad::default()).id();
                    spawn_als_actions(
                        &mut app.world_mut().commands(),
                        player,
                        &AlsInputMap::default(),
                    );
                    app.update();
                    app.world_mut()
                        .resource_mut::<Time<Virtual>>()
                        .set_relative_speed(speed);
                    if gamepad_input {
                        app.world_mut().write_message(RawGamepadEvent::Button(
                            RawGamepadButtonChangedEvent::new(gamepad, GamepadButton::East, 1.0),
                        ));
                    } else {
                        app.world_mut()
                            .resource_mut::<ButtonInput<KeyCode>>()
                            .press(KeyCode::ShiftLeft);
                    }
                    for _ in 0..frames {
                        app.update();
                        assert!(
                            !app.world()
                                .get::<AlsInputCommand>(player)
                                .unwrap()
                                .roll_pressed,
                            "Source Tap must wait for release"
                        );
                    }
                    assert_eq!(
                        app.world()
                            .get::<AlsInputCommand>(player)
                            .unwrap()
                            .sprint_held,
                        frames >= 8,
                        "Source one-shot Hold intent for gamepad={gamepad_input}, frames={frames}, speed={speed}"
                    );
                    if gamepad_input {
                        app.world_mut().write_message(RawGamepadEvent::Button(
                            RawGamepadButtonChangedEvent::new(gamepad, GamepadButton::East, 0.0),
                        ));
                    } else {
                        app.world_mut()
                            .resource_mut::<ButtonInput<KeyCode>>()
                            .release(KeyCode::ShiftLeft);
                    }
                    app.update();
                    let released = app.world().get::<AlsInputCommand>(player).unwrap();
                    assert_eq!(
                        released.roll_pressed,
                        frames < 20,
                        "Source shared Sprint/Roll Tap release for gamepad={gamepad_input}, frames={frames}, speed={speed}"
                    );
                    assert!(
                        !released.sprint_held,
                        "Source Released trigger must clear Sprint intent"
                    );
                    app.update();
                    assert!(
                        !app.world()
                            .get::<AlsInputCommand>(player)
                            .unwrap()
                            .roll_pressed
                    );
                }
            }
        }
    }
    #[test]
    fn shift_debug_digits_do_not_block_source_gamepad_mode_actions() {
        use bevy::input::gamepad::{RawGamepadButtonChangedEvent, RawGamepadEvent};
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            bevy::input::InputPlugin,
            EnhancedInputPlugin,
            AlsInputPlugin,
        ));
        app.finish();
        let player = app.world_mut().spawn_empty().id();
        let gamepad = app.world_mut().spawn(Gamepad::default()).id();
        spawn_als_actions(
            &mut app.world_mut().commands(),
            player,
            &AlsInputMap::default(),
        );
        app.update();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::ShiftLeft);
        for button in [GamepadButton::DPadLeft, GamepadButton::DPadRight] {
            app.world_mut().write_message(RawGamepadEvent::Button(
                RawGamepadButtonChangedEvent::new(gamepad, button, 1.0),
            ));
        }
        app.update();
        let command = app.world().get::<AlsInputCommand>(player).unwrap();
        assert!(
            command.rotation_mode_pressed && command.view_mode_pressed,
            "Source gamepad mode presses must remain active while the keyboard debug Shift is held"
        );
        app.update();
        let command = app.world().get::<AlsInputCommand>(player).unwrap();
        assert!(
            !command.rotation_mode_pressed && !command.view_mode_pressed,
            "Source mode Pressed trigger must not repeat while held"
        );
    }

    #[test]
    fn shift_policy_preserves_custom_mode_keys_and_current_frame_digit_chords() {
        for digits in [false, true] {
            let mut app = App::new();
            app.add_plugins((
                MinimalPlugins,
                bevy::input::InputPlugin,
                EnhancedInputPlugin,
                AlsInputPlugin,
            ));
            app.finish();
            let player = app.world_mut().spawn_empty().id();
            let keys = if digits {
                [KeyCode::Digit1, KeyCode::Digit2]
            } else {
                [KeyCode::KeyR, KeyCode::KeyF]
            };
            let mut map = AlsInputMap::default();
            map.rotation_mode = vec![keys[0].with_mod_keys(ModKeys::CONTROL)];
            map.view_mode = vec![keys[1].with_mod_keys(ModKeys::CONTROL)];
            spawn_als_actions(&mut app.world_mut().commands(), player, &map);
            app.update();
            let mut pressed = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            for key in [KeyCode::ShiftLeft, KeyCode::ControlLeft, keys[0], keys[1]] {
                pressed.press(key);
            }
            app.update();
            let command = app.world().get::<AlsInputCommand>(player).unwrap();
            assert_eq!(
                command.rotation_mode_pressed, !digits,
                "Current-frame debug Shift must apply only to numeric mode bindings"
            );
            assert_eq!(
                command.view_mode_pressed, !digits,
                "Current-frame debug Shift must preserve custom nonnumeric mode bindings"
            );
        }
    }

    #[test]
    fn paused_pressed_actions_do_not_change_source_saved_intent() {
        use bevy_als::als::character::AlsCharacter;
        use bevy_als::als::gait::AlsDesiredGait;
        use bevy_als::als::state::{Gait, RotationMode, Stance};
        use bevy_als::als::transition::AlsStateRequest;
        use bevy::input::gamepad::{RawGamepadButtonChangedEvent, RawGamepadEvent};
        for gamepad_input in [false, true] {
            let mut app = App::new();
            app.add_plugins((
                MinimalPlugins,
                bevy::input::InputPlugin,
                EnhancedInputPlugin,
                AlsInputPlugin,
            ))
            .add_systems(
                Update,
                bevy_als::als::input_request::input_state_request_system,
            );
            app.finish();
            let player = app
                .world_mut()
                .spawn((
                    AlsCharacter,
                    Stance::Standing,
                    RotationMode::ViewDirection,
                    AlsDesiredGait(Gait::Running),
                    AlsStateRequest::default(),
                ))
                .id();
            let gamepad = app.world_mut().spawn(Gamepad::default()).id();
            spawn_als_actions(
                &mut app.world_mut().commands(),
                player,
                &AlsInputMap::default(),
            );
            app.update();
            app.world_mut().resource_mut::<Time<Virtual>>().pause();
            for value in [1.0, 0.0, 1.0] {
                if value == 0.0 {
                    app.world_mut().resource_mut::<Time<Virtual>>().unpause();
                }
                for (key, button) in [
                    (KeyCode::AltLeft, GamepadButton::RightTrigger),
                    (KeyCode::ControlLeft, GamepadButton::LeftThumb),
                    (KeyCode::KeyX, GamepadButton::West),
                    (KeyCode::KeyC, GamepadButton::RightThumb),
                ] {
                    if gamepad_input {
                        app.world_mut().write_message(RawGamepadEvent::Button(
                            RawGamepadButtonChangedEvent::new(gamepad, button, value),
                        ));
                    } else {
                        let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
                        if value > 0.0 {
                            keys.press(key);
                        } else {
                            keys.release(key);
                        }
                    }
                }
                app.update();
                let command = *app.world().get::<AlsInputCommand>(player).unwrap();
                let paused = app.world().resource::<Time<Virtual>>().is_paused();
                let expected = value > 0.0 && !paused;
                assert_eq!(
                    [
                        command.walk_pressed,
                        command.crouch_pressed,
                        command.ragdoll_pressed,
                        command.switch_shoulder_pressed
                    ],
                    [expected; 4],
                    "Source paused Pressed events for gamepad={gamepad_input}, value={value}, paused={paused}"
                );
                let request = app.world().get::<AlsStateRequest>(player).unwrap();
                assert_eq!(request.stance, expected.then_some(Stance::Crouching));
                assert_eq!(request.ragdoll_start, expected);
                assert_eq!(
                    app.world()
                        .get::<bevy_als::als::input_request::AlsCameraLeftShoulder>(player)
                        .is_some(),
                    expected
                );
                assert_eq!(
                    app.world().get::<AlsDesiredGait>(player).unwrap().0,
                    if expected {
                        Gait::Walking
                    } else {
                        Gait::Running
                    },
                    "Source paused Walk trigger must not change saved gait intent"
                );
                if paused {
                    app.world_mut().resource_mut::<Time<Virtual>>().unpause();
                    app.update();
                    let command = app.world().get::<AlsInputCommand>(player).unwrap();
                    assert_eq!(
                        [
                            command.walk_pressed,
                            command.crouch_pressed,
                            command.ragdoll_pressed,
                            command.switch_shoulder_pressed
                        ],
                        [false; 4],
                        "Source Pressed history must advance during pause without replay on resume"
                    );
                    assert_eq!(
                        app.world().get::<AlsDesiredGait>(player).unwrap().0,
                        Gait::Running
                    );
                }
            }
            app.update();
            let command = app.world().get::<AlsInputCommand>(player).unwrap();
            assert_eq!(
                [
                    command.walk_pressed,
                    command.crouch_pressed,
                    command.ragdoll_pressed,
                    command.switch_shoulder_pressed
                ],
                [false; 4],
                "Source held Pressed must not repeat"
            );
        }
    }
    #[test]
    fn paused_mode_pressed_actions_preserve_source_saved_modes() {
        use bevy_als::als::aim::AlsLastFreeRotationMode;
        use bevy_als::als::character::AlsCharacter;
        use bevy_als::als::gait::AlsDesiredGait;
        use bevy_als::als::state::{Gait, RotationMode, Stance};
        use bevy_als::als::transition::AlsStateRequest;
        use bevy::input::gamepad::{RawGamepadButtonChangedEvent, RawGamepadEvent};
        for gamepad_input in [false, true] {
            let mut app = App::new();
            app.add_plugins((
                MinimalPlugins,
                bevy::input::InputPlugin,
                EnhancedInputPlugin,
                AlsInputPlugin,
            ))
            .add_systems(
                Update,
                bevy_als::als::input_request::input_state_request_system,
            );
            app.finish();
            let player = app
                .world_mut()
                .spawn((
                    AlsCharacter,
                    Stance::Standing,
                    RotationMode::ViewDirection,
                    AlsLastFreeRotationMode(RotationMode::ViewDirection),
                    AlsDesiredGait(Gait::Running),
                    AlsStateRequest::default(),
                ))
                .id();
            let gamepad = app.world_mut().spawn(Gamepad::default()).id();
            spawn_als_actions(
                &mut app.world_mut().commands(),
                player,
                &AlsInputMap::default(),
            );
            app.update();
            app.world_mut().resource_mut::<Time<Virtual>>().pause();
            for value in [1.0, 0.0, 1.0] {
                if value == 0.0 {
                    app.world_mut().resource_mut::<Time<Virtual>>().unpause();
                }
                for (key, button) in [
                    (KeyCode::Digit1, GamepadButton::DPadLeft),
                    (KeyCode::Digit2, GamepadButton::DPadRight),
                ] {
                    if gamepad_input {
                        app.world_mut().write_message(RawGamepadEvent::Button(
                            RawGamepadButtonChangedEvent::new(gamepad, button, value),
                        ));
                    } else {
                        let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
                        if value > 0.0 {
                            keys.press(key);
                        } else {
                            keys.release(key);
                        }
                    }
                }
                app.update();
                let paused = app.world().resource::<Time<Virtual>>().is_paused();
                let expected = value > 0.0 && !paused;
                let command = app.world().get::<AlsInputCommand>(player).unwrap();
                assert_eq!(
                    [command.rotation_mode_pressed, command.view_mode_pressed],
                    [expected; 2],
                    "Source paused mode events for gamepad={gamepad_input}, value={value}, paused={paused}"
                );
                assert_eq!(
                    app.world()
                        .get::<AlsLastFreeRotationMode>(player)
                        .unwrap()
                        .0,
                    if expected {
                        RotationMode::VelocityDirection
                    } else {
                        RotationMode::ViewDirection
                    },
                    "Source paused RotationMode trigger must preserve saved rotation intent"
                );
                assert_eq!(
                    app.world()
                        .get::<bevy_als::als::input_request::AlsFirstPersonView>(player)
                        .is_some(),
                    expected
                );
                if paused {
                    app.world_mut().resource_mut::<Time<Virtual>>().unpause();
                    app.update();
                    let command = app.world().get::<AlsInputCommand>(player).unwrap();
                    assert_eq!(
                        [command.rotation_mode_pressed, command.view_mode_pressed],
                        [false; 2],
                        "Source paused Pressed mode input must not replay on resume"
                    );
                }
            }
            app.update();
            let command = app.world().get::<AlsInputCommand>(player).unwrap();
            assert!(!command.rotation_mode_pressed && !command.view_mode_pressed);
        }
    }
    #[test]
    fn aim_pressed_during_pause_preserves_source_saved_aim_until_a_fresh_trigger() {
        use bevy_als::als::aim::{AlsAimPlugin, AlsLastFreeRotationMode};
        use bevy_als::als::character::AlsCharacter;
        use bevy_als::als::gait::AlsMaxAllowedGait;
        use bevy_als::als::state::{Gait, LocomotionState, RotationMode, Stance};
        use bevy::input::gamepad::{RawGamepadButtonChangedEvent, RawGamepadEvent};
        let cases = [
            vec![
                (true, true, false, false),
                (false, true, false, false),
                (false, false, false, false),
                (false, true, true, true),
                (false, true, true, true),
                (true, true, true, true),
                (true, false, true, true),
                (false, false, true, true),
                (false, true, true, true),
                (false, true, true, true),
                (false, false, false, false),
            ],
            vec![
                (false, true, true, true),
                (false, true, true, true),
                (true, false, false, true),
                (false, false, false, false),
            ],
            vec![
                (false, true, true, true),
                (true, false, true, true),
                (false, false, true, true),
                (false, true, true, true),
                (false, true, true, true),
                (false, false, false, false),
            ],
        ];
        for gamepad_input in [false, true] {
            for (case_index, steps) in cases.iter().enumerate() {
                let mut app = App::new();
                app.add_plugins((
                    MinimalPlugins,
                    bevy::input::InputPlugin,
                    EnhancedInputPlugin,
                    AlsInputPlugin,
                    AlsAimPlugin,
                ));
                app.finish();
                let player = app
                    .world_mut()
                    .spawn((
                        AlsCharacter,
                        Stance::Standing,
                        LocomotionState::Grounded,
                        RotationMode::ViewDirection,
                        AlsLastFreeRotationMode(RotationMode::ViewDirection),
                        AlsMaxAllowedGait(Gait::Running),
                    ))
                    .id();
                let gamepad = app.world_mut().spawn(Gamepad::default()).id();
                spawn_als_actions(
                    &mut app.world_mut().commands(),
                    player,
                    &AlsInputMap::default(),
                );
                app.update();
                for (step_index, &(paused, held, expected_aim, expected_mode)) in
                    steps.iter().enumerate()
                {
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
                                GamepadButton::LeftTrigger,
                                if held { 1.0 } else { 0.0 },
                            ),
                        ));
                    } else {
                        let mut buttons =
                            app.world_mut().resource_mut::<ButtonInput<MouseButton>>();
                        if held {
                            buttons.press(MouseButton::Right);
                        } else {
                            buttons.release(MouseButton::Right);
                        }
                    }
                    app.update();
                    assert_eq!(
                        app.world().get::<AlsInputCommand>(player).unwrap().aim_held,
                        expected_aim,
                        "Source Aim saved intent for device={gamepad_input}, case={case_index}, step={step_index}"
                    );
                    assert_eq!(
                        *app.world().get::<RotationMode>(player).unwrap(),
                        if expected_mode {
                            RotationMode::Aiming
                        } else {
                            RotationMode::ViewDirection
                        },
                        "Source Aim owner phase for device={gamepad_input}, case={case_index}, step={step_index}"
                    );
                }
            }
        }
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            bevy::input::InputPlugin,
            EnhancedInputPlugin,
            AlsInputPlugin,
        ));
        app.finish();
        let player = app.world_mut().spawn_empty().id();
        spawn_als_actions(
            &mut app.world_mut().commands(),
            player,
            &AlsInputMap::default(),
        );
        app.update();
        app.world_mut()
            .resource_mut::<Time<Virtual>>()
            .set_relative_speed(0.0);
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Right);
        app.update();
        assert!(
            app.world().get::<AlsInputCommand>(player).unwrap().aim_held,
            "Source zero game-time rate must not be mistaken for game pause"
        );
    }
    #[test]
    fn paused_jump_press_does_not_dispatch_source_action_chain_or_replay_on_resume() {
        use bevy::input::gamepad::{RawGamepadButtonChangedEvent, RawGamepadEvent};
        let cases = [
            vec![
                (true, true, false),
                (false, true, false),
                (false, false, false),
                (false, true, true),
                (false, true, false),
                (false, false, false),
            ],
            vec![
                (false, true, true),
                (false, true, false),
                (true, true, true),
                (true, true, false),
                (true, false, false),
                (false, false, false),
            ],
            vec![
                (false, true, true),
                (true, true, false),
                (false, true, false),
                (false, false, false),
            ],
            vec![
                (false, true, true),
                (false, true, false),
                (true, false, false),
                (false, false, false),
            ],
        ];
        for gamepad_input in [false, true] {
            for (case_index, steps) in cases.iter().enumerate() {
                let mut app = App::new();
                app.add_plugins((
                    MinimalPlugins,
                    bevy::input::InputPlugin,
                    EnhancedInputPlugin,
                    AlsInputPlugin,
                ));
                app.finish();
                let player = app.world_mut().spawn_empty().id();
                let gamepad = app.world_mut().spawn(Gamepad::default()).id();
                spawn_als_actions(
                    &mut app.world_mut().commands(),
                    player,
                    &AlsInputMap::default(),
                );
                app.update();
                for (step_index, &(paused, held, expected_dispatch)) in steps.iter().enumerate() {
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
                                GamepadButton::South,
                                if held { 1.0 } else { 0.0 },
                            ),
                        ));
                    } else {
                        let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
                        if held {
                            keys.press(KeyCode::Space);
                        } else {
                            keys.release(KeyCode::Space);
                        }
                    }
                    app.update();
                    let command = app.world().get::<AlsInputCommand>(player).unwrap();
                    assert_eq!(
                        command.jump_pressed, expected_dispatch,
                        "Source Jump positive Triggered/Canceled dispatch for device={gamepad_input}, case={case_index}, step={step_index}"
                    );
                    assert_eq!(
                        command.jump_held, held,
                        "Existing raw physical Jump hold must remain available for separate motor validation"
                    );
                }
            }
        }
    }
    #[test]
    fn paused_look_events_preserve_actual_view_and_do_not_replay_mouse_deltas() {
        use bevy_als::als::character::AlsCharacter;
        use bevy_als::als::motion::AlsViewYaw;
        use bevy_als::als::view::{AlsViewInputPlugin, AlsViewPitch};
        use bevy::input::{
            gamepad::{RawGamepadAxisChangedEvent, RawGamepadEvent},
            mouse::MouseMotion,
        };
        use bevy::time::TimeUpdateStrategy;
        for gamepad_input in [false, true] {
            let mut app = App::new();
            app.add_plugins((
                MinimalPlugins,
                bevy::input::InputPlugin,
                EnhancedInputPlugin,
                AlsInputPlugin,
                AlsViewInputPlugin,
            ))
            .insert_resource(TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_millis(20),
            ));
            app.finish();
            let character = app
                .world_mut()
                .spawn((AlsCharacter, AlsViewYaw(0.0), AlsViewPitch(0.0)))
                .id();
            let gamepad = app.world_mut().spawn(Gamepad::default()).id();
            spawn_als_actions(
                &mut app.world_mut().commands(),
                character,
                &AlsInputMap::default(),
            );
            app.update();
            let mut expected = Vec2::ZERO;
            for (paused, held, mouse) in [
                (true, true, Vec2::new(8.0, -4.0)),
                (false, true, Vec2::ZERO),
                (true, false, Vec2::new(7.0, -3.0)),
                (false, false, Vec2::ZERO),
                (false, true, Vec2::new(2.0, -3.0)),
                (true, true, Vec2::new(-7.0, 4.0)),
                (false, true, Vec2::ZERO),
                (false, false, Vec2::ZERO),
            ] {
                let mut time = app.world_mut().resource_mut::<Time<Virtual>>();
                if paused {
                    time.pause();
                } else {
                    time.unpause();
                }
                if gamepad_input {
                    app.world_mut().write_message(RawGamepadEvent::Axis(
                        RawGamepadAxisChangedEvent::new(
                            gamepad,
                            GamepadAxis::RightStickX,
                            if held { 1.0 } else { 0.0 },
                        ),
                    ));
                } else {
                    let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
                    if held {
                        keys.press(KeyCode::ArrowRight);
                    } else {
                        keys.release(KeyCode::ArrowRight);
                    }
                }
                if mouse != Vec2::ZERO {
                    app.world_mut().write_message(MouseMotion { delta: mouse });
                }
                app.update();
                if !paused {
                    expected -= Vec2::new(mouse.x + if held { 4.8 } else { 0.0 }, mouse.y)
                        * std::f32::consts::PI
                        / 180.0;
                }
                assert!(
                    (app.world().get::<AlsViewYaw>(character).unwrap().0 - expected.x).abs() < 1e-6
                        && (app.world().get::<AlsViewPitch>(character).unwrap().0 - expected.y)
                            .abs()
                            < 1e-6,
                    "Source paused view output for device={gamepad_input}, paused={paused}, held={held}, mouse={mouse:?}"
                );
            }
        }
    }
    #[test]
    fn look_rate_uses_source_nondilated_time_and_rejects_paused_actions() {
        use bevy_als::als::character::AlsCharacter;
        use bevy_als::als::motion::AlsViewYaw;
        use bevy_als::als::view::{AlsViewInputPlugin, AlsViewPitch};
        use bevy::input::{InputPlugin, mouse::MouseMotion};
        use bevy::time::TimeUpdateStrategy;
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            InputPlugin,
            EnhancedInputPlugin,
            AlsInputPlugin,
            AlsViewInputPlugin,
        ))
        .insert_resource(TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f32(0.1),
        ));
        app.finish();
        let character = app
            .world_mut()
            .spawn((AlsCharacter, AlsViewYaw(0.0), AlsViewPitch(0.0)))
            .id();
        spawn_als_actions(
            &mut app.world_mut().commands(),
            character,
            &AlsInputMap::default(),
        );
        app.update();
        for speed in [1.0, 0.15, 2.0, 0.0] {
            app.world_mut()
                .resource_mut::<Time<Virtual>>()
                .set_relative_speed(speed);
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(KeyCode::ArrowRight);
            app.world_mut().write_message(MouseMotion {
                delta: Vec2::new(4.0, -2.0),
            });
            app.world_mut()
                .entity_mut(character)
                .insert((AlsViewYaw(0.0), AlsViewPitch(0.0)));
            app.update();
            assert!(
                (app.world().get::<AlsViewYaw>(character).unwrap().0 + 28.0_f32.to_radians()).abs()
                    < 1e-6,
                "Source view Delta for speed={speed}"
            );
            assert!(
                (app.world().get::<AlsViewPitch>(character).unwrap().0 - 2.0_f32.to_radians())
                    .abs()
                    < 1e-6
            );
        }
        {
            let mut clock = app.world_mut().resource_mut::<Time<Virtual>>();
            clock.set_relative_speed(0.15);
            clock.set_max_delta(std::time::Duration::from_micros(7500));
        }
        app.world_mut()
            .entity_mut(character)
            .insert((AlsViewYaw(0.0), AlsViewPitch(0.0)));
        app.world_mut().write_message(MouseMotion {
            delta: Vec2::new(4.0, -2.0),
        });
        app.update();
        assert_eq!(
            app.world().resource::<Time<Virtual>>().delta(),
            std::time::Duration::from_micros(7500)
        );
        assert!(
            (app.world().get::<AlsViewYaw>(character).unwrap().0 + 16.0_f32.to_radians()).abs()
                < 1e-6,
            "Source nondilated input must preserve the effective frame clamp"
        );
        app.world_mut().resource_mut::<Time<Virtual>>().pause();
        app.world_mut()
            .entity_mut(character)
            .insert((AlsViewYaw(0.0), AlsViewPitch(0.0)));
        app.world_mut().write_message(MouseMotion {
            delta: Vec2::new(4.0, -2.0),
        });
        app.update();
        assert_eq!(app.world().get::<AlsViewYaw>(character).unwrap().0, 0.0);
        assert_eq!(app.world().get::<AlsViewPitch>(character).unwrap().0, 0.0);
    }
}
