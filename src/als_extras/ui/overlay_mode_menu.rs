use bevy_als::als::character::AlsCharacter;
use crate::input::OverlayMenuAction;
use bevy_als::als::state::OverlayState;
use bevy_als::als::transition::AlsStateRequest;
use bevy::prelude::*;
use bevy::text::{LetterSpacing, LineHeight};
use bevy_enhanced_input::prelude::*;

use crate::als_hud::AlsHudState;
use bevy_als::character_assembly::AlsPlayerCharacter;

pub fn menu_display_order(tags: &bevy_gameplay_tag::GameplayTagsManager) -> Vec<OverlayState> {
    let parent = "Als.OverlayMode";
    let mut values: Vec<_> = tags
        .tag_map
        .keys()
        .filter(|tag| {
            tag.as_str()
                .get(..parent.len())
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case(parent))
                && tag.as_str().as_bytes().get(parent.len()) == Some(&b'.')
        })
        .map(OverlayState::from_tag)
        .collect();
    let name_key = |name: &str| {
        let prefix = name.trim_end_matches(|value: char| value.is_ascii_digit());
        match name[prefix.len()..].parse::<i64>() {
            Ok(suffix) => (prefix.to_ascii_lowercase(), suffix),
            Err(_) => (name.to_ascii_lowercase(), -1),
        }
    };
    values.sort_by(|a, b| {
        for (a, b) in a.as_str().split('.').zip(b.as_str().split('.')) {
            let order = name_key(b).cmp(&name_key(a));
            if !order.is_eq() {
                return order;
            }
        }
        a.as_str().len().cmp(&b.as_str().len())
    });
    values.dedup();
    values
}

pub const MENU_DILATION: f32 = 0.35;
include!("overlay_mode_menu_generated.rs");

#[derive(Resource, Default)]
pub struct OverlayMenuState {
    pub open: bool,
    pub selected: Option<OverlayState>,
    pub root: Option<Entity>,
    pub options: Vec<OverlayState>,
}

impl OverlayMenuState {
    pub fn selected_index(&self) -> usize {
        self.options
            .iter()
            .position(|value| Some(*value) == self.selected)
            .unwrap_or(usize::MAX)
    }
}

#[derive(Component)]
pub struct OverlayMenuOption {
    pub index: usize,
}

pub fn next_index(current: usize, len: usize) -> usize {
    assert!(len > 0);
    if current == 0 || current >= len {
        len - 1
    } else {
        current - 1
    }
}

pub fn previous_index(current: usize, len: usize) -> usize {
    assert!(len > 0);
    if current >= len - 1 { 0 } else { current + 1 }
}

pub fn option_label(state: OverlayState) -> String {
    bevy_als::show_debug_hud::name_to_display_string(&format!("{state:?}"), false)
}

pub struct OverlayModeMenuPlugin;

impl Plugin for OverlayModeMenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AlsHudState>()
            .init_resource::<OverlayMenuState>()
            .add_systems(Startup, initialize_menu)
            .add_systems(
                Update,
                (
                    menu_control_system.after(bevy_als::als::view::AlsViewInputSet),
                    menu_highlight_system,
                )
                    .chain(),
            )
            .add_systems(
                PostUpdate,
                menu_ellipsis_system
                    .after(bevy::text::load_font_assets_into_font_collection)
                    .before(bevy::text::detect_text_needs_rerender)
                    .run_if(
                        bevy::ecs::schedule::common_conditions::resource_exists::<Assets<Font>>,
                    ),
            );
    }
}

fn initialize_menu(
    mut commands: Commands,
    mut state: ResMut<OverlayMenuState>,
    tags: Res<bevy_gameplay_tag::GameplayTagsManager>,
) {
    state.options = menu_display_order(&tags);
    state.root = Some(spawn_menu(
        &mut commands,
        state.selected_index(),
        &state.options,
    ));
}

fn menu_control_system(
    mut commands: Commands,
    mut state: ResMut<OverlayMenuState>,
    hud_state: Res<AlsHudState>,
    mut time: ResMut<Time<Virtual>>,
    menu_held: Query<&Action<OverlayMenuAction>>,
    next_states: Query<&TriggerState, With<Action<crate::input::NextOverlayModeAction>>>,
    prev_states: Query<&TriggerState, With<Action<crate::input::PreviousOverlayModeAction>>>,
    characters: Query<(Entity, &OverlayState), (With<AlsCharacter>, With<AlsPlayerCharacter>)>,
    mut requests: Query<&mut AlsStateRequest>,
    mut nodes: Query<&mut Node>,
    mut prev_held: Local<bool>,
) {
    let held = menu_held.iter().any(|action| **action);
    let pressed = held && !*prev_held;
    let released = !held && *prev_held;
    *prev_held = held;
    if time.is_paused() {
        return;
    }

    let next = next_states
        .iter()
        .any(|state| *state == TriggerState::Fired);
    let prev = prev_states
        .iter()
        .any(|state| *state == TriggerState::Fired);

    if state.open && (next || prev) && !state.options.is_empty() {
        let len = state.options.len();
        let mut selected_index = state.selected_index();
        if next {
            selected_index = next_index(selected_index, len);
        }
        if prev {
            selected_index = previous_index(selected_index, len);
        }
        state.selected = Some(state.options[selected_index]);
    }

    if pressed && !state.open {
        state.open = true;
        if let Ok((_, overlay)) = characters.single() {
            state.selected = Some(*overlay);
        }
        if !hud_state.slomo_active {
            time.set_relative_speed(MENU_DILATION);
        }
    }

    if released {
        if let Ok((entity, _)) = characters.single() {
            let selected = state.selected.unwrap_or(OverlayState::new("").unwrap());
            match requests.get_mut(entity) {
                Ok(mut request) => request.overlay_state = Some(selected),
                Err(_) => {
                    commands.entity(entity).insert(AlsStateRequest {
                        overlay_state: Some(selected),
                        ..default()
                    });
                }
            }
        }
        state.open = false;
        if !hud_state.slomo_active {
            time.set_relative_speed(1.0);
        }
    }
    if let Some(root) = state.root
        && let Ok(mut node) = nodes.get_mut(root)
    {
        let display = if state.open {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != display {
            node.display = display;
        }
    }
}

fn spawn_menu(
    commands: &mut Commands,
    selected_index: usize,
    display_order: &[OverlayState],
) -> Entity {
    let root = commands
        .spawn((
            Node {
                display: Display::None,
                position_type: PositionType::Absolute,
                right: Val::Px(
                    crate::als_hud::PANEL_RIGHT_MARGIN_PX
                        + crate::als_hud::PANEL_WIDTH_PX
                        + crate::als_hud::SECTION_GAP_PX,
                ),
                top: Val::Px(crate::als_hud::PANEL_TOP_MARGIN_PX),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(OPTION_GAP_PX),
                ..default()
            },
            BackgroundColor(COLOR_MENU_BACKGROUND),
            GlobalZIndex(11),
        ))
        .id();

    for (index, overlay) in display_order.iter().enumerate() {
        let selected = index == selected_index;
        let option = commands
            .spawn((
                OverlayMenuOption { index },
                Node {
                    width: Val::Px(OPTION_WIDTH_PX),
                    height: Val::Px(OPTION_HEIGHT_PX),
                    padding: UiRect::horizontal(Val::Px(OPTION_HORIZONTAL_PADDING_PX)),
                    overflow: Overflow::clip(),
                    overflow_clip_margin: OverflowClipMargin::content_box(),
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    ..default()
                },
                BackgroundColor(if selected {
                    COLOR_OPTION_SELECTED
                } else {
                    COLOR_OPTION_NOT_SELECTED
                }),
            ))
            .with_children(|parent| {
                parent.spawn((
                    Text::new(option_label(*overlay)),
                    Node {
                        min_width: Val::Px(0.0),
                        ..default()
                    },
                    TextLayout::default().with_no_wrap(),
                    TextFont {
                        font_size: FontSize::Px(OPTION_FONT_SIZE_PX),
                        ..default()
                    },
                    TextColor(COLOR_MENU_TEXT),
                ));
            })
            .id();
        commands.entity(root).add_child(option);
    }

    root
}

fn menu_ellipsis_system(
    menu: Res<OverlayMenuState>,
    fonts: Res<Assets<Font>>,
    mut font_events: MessageReader<AssetEvent<Font>>,
    mut pipeline: ResMut<bevy::text::TextPipeline>,
    mut font_cx: ResMut<bevy::text::FontCx>,
    mut layout_cx: ResMut<bevy::text::LayoutCx>,
    rem: Res<bevy::text::RemSize>,
    options: Query<(&OverlayMenuOption, &Children)>,
    mut texts: Query<(
        Entity,
        &mut Text,
        &mut Node,
        &TextFont,
        &LineHeight,
        &LetterSpacing,
    )>,
) {
    let fonts_changed = font_events.read().count() > 0;
    if !menu.is_added() && !fonts_changed {
        return;
    }
    let width = OPTION_WIDTH_PX - 2.0 * OPTION_HORIZONTAL_PADDING_PX;
    for (option, children) in &options {
        let Ok((entity, mut text, mut node, font, line_height, spacing)) =
            texts.get_mut(children[0])
        else {
            continue;
        };
        let original = option_label(menu.options[option.index]);
        let mut computed = bevy::text::ComputedTextBlock::default();
        let result = crate::text_overflow::ellipsize(&original, width, |value| {
            pipeline
                .create_text_measure(
                    entity,
                    &fonts,
                    std::iter::once((
                        entity,
                        0,
                        bevy::text::TextElement::Text {
                            text: value,
                            font,
                            color: COLOR_MENU_TEXT,
                            line_height: *line_height,
                            letter_spacing: *spacing,
                        },
                    )),
                    1.0,
                    &TextLayout::no_wrap(),
                    &mut computed,
                    &mut font_cx,
                    &mut layout_cx,
                    Vec2::ZERO,
                    *rem,
                )
                .map(|_| computed.buffer().full_width())
        });
        if let Ok((displayed, natural_width)) = result {
            let desired_width = if displayed == original {
                Val::Auto
            } else {
                Val::Px(natural_width)
            };
            if node.width != desired_width {
                node.width = desired_width;
            }
            if text.0 != displayed {
                text.0 = displayed;
            }
        }
    }
}

fn menu_highlight_system(
    state: Res<OverlayMenuState>,
    mut options: Query<(&OverlayMenuOption, &mut BackgroundColor)>,
) {
    if !state.open {
        return;
    }
    for (option, mut background) in &mut options {
        let selected = option.index == state.selected_index();
        let color = if selected {
            COLOR_OPTION_SELECTED
        } else {
            COLOR_OPTION_NOT_SELECTED
        };
        if background.0 != color {
            background.0 = color;
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    #[test]
    fn browser_review_tag_menu_uses_case_insensitive_descendants_and_unique_identity() {
        let tags = tag_manager(&[
            "als.overlaymode.browsercase",
            "ALS.OVERLAYMODE.BROWSERCASE",
            "als.overlaymode.rifle",
            "Als.OverlayMode2.Other",
        ]);
        let order = menu_display_order(&tags);
        assert_eq!(
            order
                .iter()
                .filter(|tag| tag
                    .as_str()
                    .eq_ignore_ascii_case("Als.OverlayMode.BrowserCase"))
                .count(),
            1,
            "Source hierarchy lost the registered case aliases"
        );
        assert_eq!(
            order
                .iter()
                .filter(|tag| **tag == OverlayState::Rifle)
                .count(),
            1
        );
        assert_eq!(order.len(), 14);
    }

    use super::*;

    pub(crate) fn tag_manager(extra: &[&str]) -> bevy_gameplay_tag::GameplayTagsManager {
        let mut rows: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../assets/ALS/Data/als_gameplay_tags.json"
        ))
        .unwrap();
        rows.extend(
            extra
                .iter()
                .map(|name| serde_json::json!({"tag_name": name, "description": "Test overlay"})),
        );
        let mut world = World::new();
        world.insert_resource(bevy_gameplay_tag::GameplayTagsSettings {
            json_data: serde_json::to_string(&rows).unwrap(),
            data_path: None,
        });
        bevy_gameplay_tag::GameplayTagsManager::from_world(&mut world)
    }

    #[test]
    fn paused_menu_events_preserve_source_open_selection_and_commit_state() {
        use crate::input::{AlsInputMap, AlsInputPlugin, spawn_als_actions};
        use bevy::input::gamepad::{RawGamepadButtonChangedEvent, RawGamepadEvent};
        for gamepad_menu in [false, true] {
            let mut app = App::new();
            app.add_plugins((
                MinimalPlugins,
                bevy::input::InputPlugin,
                EnhancedInputPlugin,
                AlsInputPlugin,
            ))
            .init_resource::<AlsHudState>()
            .insert_resource(OverlayMenuState {
                options: menu_display_order(&tag_manager(&[])),
                ..default()
            })
            .add_systems(Update, menu_control_system);
            app.finish();
            let player = app
                .world_mut()
                .spawn((
                    AlsCharacter,
                    AlsPlayerCharacter,
                    OverlayState::Default,
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
            if gamepad_menu {
                app.world_mut().write_message(RawGamepadEvent::Button(
                    RawGamepadButtonChangedEvent::new(gamepad, GamepadButton::North, 1.0),
                ));
            } else {
                app.world_mut()
                    .resource_mut::<ButtonInput<KeyCode>>()
                    .press(KeyCode::KeyQ);
            }
            app.update();
            assert!(app.world().resource::<OverlayMenuState>().open);
            let selected = app.world().resource::<OverlayMenuState>().selected;
            app.world_mut().resource_mut::<Time<Virtual>>().pause();
            for button in [GamepadButton::DPadUp, GamepadButton::DPadDown] {
                app.world_mut().write_message(RawGamepadEvent::Button(
                    RawGamepadButtonChangedEvent::new(gamepad, button, 1.0),
                ));
                app.update();
                assert_eq!(
                    app.world().resource::<OverlayMenuState>().selected,
                    selected,
                    "Source paused Down selection must preserve the open menu selection"
                );
                app.world_mut().write_message(RawGamepadEvent::Button(
                    RawGamepadButtonChangedEvent::new(gamepad, button, 0.0),
                ));
            }
            if gamepad_menu {
                app.world_mut().write_message(RawGamepadEvent::Button(
                    RawGamepadButtonChangedEvent::new(gamepad, GamepadButton::North, 0.0),
                ));
            } else {
                app.world_mut()
                    .resource_mut::<ButtonInput<KeyCode>>()
                    .release(KeyCode::KeyQ);
            }
            app.update();
            assert!(
                app.world().resource::<OverlayMenuState>().open,
                "Source paused menu Released event must not close or approve the menu"
            );
            assert_eq!(
                app.world()
                    .get::<AlsStateRequest>(player)
                    .unwrap()
                    .overlay_state,
                None
            );
            assert_eq!(
                app.world().resource::<Time<Virtual>>().relative_speed(),
                MENU_DILATION
            );
            app.world_mut().resource_mut::<Time<Virtual>>().unpause();
            app.update();
            assert!(
                app.world().resource::<OverlayMenuState>().open,
                "Source menu release history must not replay approval after resume"
            );
            if gamepad_menu {
                app.world_mut().write_message(RawGamepadEvent::Button(
                    RawGamepadButtonChangedEvent::new(gamepad, GamepadButton::North, 1.0),
                ));
            } else {
                app.world_mut()
                    .resource_mut::<ButtonInput<KeyCode>>()
                    .press(KeyCode::KeyQ);
            }
            app.update();
            if gamepad_menu {
                app.world_mut().write_message(RawGamepadEvent::Button(
                    RawGamepadButtonChangedEvent::new(gamepad, GamepadButton::North, 0.0),
                ));
            } else {
                app.world_mut()
                    .resource_mut::<ButtonInput<KeyCode>>()
                    .release(KeyCode::KeyQ);
            }
            app.update();
            assert!(!app.world().resource::<OverlayMenuState>().open);
            assert_eq!(
                app.world()
                    .get::<AlsStateRequest>(player)
                    .unwrap()
                    .overlay_state,
                selected
            );
            assert_eq!(
                app.world().resource::<Time<Virtual>>().relative_speed(),
                1.0
            );
        }
    }
    #[test]
    fn first_menu_release_after_paused_press_approves_source_empty_selection() {
        use crate::input::{AlsInputMap, AlsInputPlugin, spawn_als_actions};
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            bevy::input::InputPlugin,
            EnhancedInputPlugin,
            AlsInputPlugin,
        ))
        .init_resource::<AlsHudState>()
        .insert_resource(OverlayMenuState {
            options: menu_display_order(&tag_manager(&[])),
            ..default()
        })
        .add_systems(Update, menu_control_system);
        app.finish();
        let player = app
            .world_mut()
            .spawn((
                AlsCharacter,
                AlsPlayerCharacter,
                OverlayState::Default,
                AlsStateRequest::default(),
            ))
            .id();
        spawn_als_actions(
            &mut app.world_mut().commands(),
            player,
            &AlsInputMap::default(),
        );
        app.update();
        app.world_mut().resource_mut::<Time<Virtual>>().pause();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyQ);
        app.update();
        assert!(!app.world().resource::<OverlayMenuState>().open);
        app.world_mut().resource_mut::<Time<Virtual>>().unpause();
        app.update();
        assert!(!app.world().resource::<OverlayMenuState>().open);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .release(KeyCode::KeyQ);
        app.update();
        assert_eq!(
            app.world()
                .get::<AlsStateRequest>(player)
                .unwrap()
                .overlay_state,
            Some(OverlayState::new("").unwrap()),
            "Source Released callback must approve the default empty Tag before first Construct"
        );
        assert_eq!(
            app.world().resource::<Time<Virtual>>().relative_speed(),
            1.0
        );
    }

    #[test]
    fn menu_uses_registered_descendants_and_source_tree_order() {
        let tags = tag_manager(&[
            "Als.OverlayMode.Custom2",
            "Als.OverlayMode.Custom10.Detail",
            "Als.Unrelated.Custom",
        ]);
        let order = menu_display_order(&tags);
        assert_eq!(order.len(), 16);
        assert_eq!(order[0], OverlayState::Torch);
        assert_eq!(*order.last().unwrap(), OverlayState::Barrel);
        let custom = order
            .iter()
            .position(|tag| tag.as_str() == "Als.OverlayMode.Custom10")
            .unwrap();
        assert_eq!(
            order[custom + 1].as_str(),
            "Als.OverlayMode.Custom10.Detail"
        );
        assert_eq!(order[custom + 2].as_str(), "Als.OverlayMode.Custom2");
        let baseline_order = menu_display_order(&tag_manager(&[]));
        assert_eq!(baseline_order.len(), 13);
        assert_eq!(baseline_order[8], OverlayState::Default);
        assert_eq!(next_index(0, order.len()), order.len() - 1);
        assert_eq!(previous_index(order.len() - 1, order.len()), 0);
        assert_eq!(next_index(usize::MAX, order.len()), order.len() - 1);
        assert_eq!(previous_index(usize::MAX, order.len()), 0);
        let empty = bevy_gameplay_tag::GameplayTagsManager {
            root: Entity::PLACEHOLDER,
            tag_map: default(),
        };
        assert!(menu_display_order(&empty).is_empty());
    }

    #[test]
    fn menu_commits_to_the_owning_pawn_and_keeps_the_initialized_dictionary() {
        let custom = OverlayState::new("Als.OverlayMode.CustomRifle").unwrap();
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(tag_manager(&[custom.as_str()]))
            .add_plugins(OverlayModeMenuPlugin);
        let decoy = app
            .world_mut()
            .spawn((
                AlsCharacter,
                OverlayState::Rifle,
                AlsStateRequest::default(),
            ))
            .id();
        let character = app
            .world_mut()
            .spawn((AlsCharacter, custom, AlsStateRequest::default()))
            .id();
        let action = app
            .world_mut()
            .spawn(Action::<OverlayMenuAction>::new())
            .id();
        app.update();
        app.world_mut()
            .entity_mut(character)
            .insert(bevy_als::character_assembly::AlsPlayerCharacter);
        **app
            .world_mut()
            .get_mut::<Action<OverlayMenuAction>>(action)
            .unwrap() = true;
        app.update();
        let state = app.world().resource::<OverlayMenuState>();
        assert!(state.open);
        assert_eq!(state.selected, Some(custom));
        assert_eq!(
            app.world_mut()
                .query::<&OverlayMenuOption>()
                .iter(app.world())
                .count(),
            14
        );
        app.insert_resource(tag_manager(&[]));
        **app
            .world_mut()
            .get_mut::<Action<OverlayMenuAction>>(action)
            .unwrap() = false;
        app.update();
        assert_eq!(
            app.world()
                .get::<AlsStateRequest>(character)
                .unwrap()
                .overlay_state,
            Some(custom)
        );
        assert_eq!(
            app.world()
                .get::<AlsStateRequest>(decoy)
                .unwrap()
                .overlay_state,
            None
        );
        assert!(!app.world().resource::<OverlayMenuState>().open);
        assert_eq!(
            app.world().resource::<Time<Virtual>>().relative_speed(),
            1.0
        );
        app.world_mut()
            .resource_mut::<bevy_gameplay_tag::GameplayTagsManager>()
            .tag_map
            .clear();
        **app
            .world_mut()
            .get_mut::<Action<OverlayMenuAction>>(action)
            .unwrap() = true;
        app.update();
        assert_eq!(app.world().resource::<OverlayMenuState>().options.len(), 14);
        assert_eq!(
            app.world().resource::<OverlayMenuState>().selected,
            Some(custom)
        );
        **app
            .world_mut()
            .get_mut::<Action<OverlayMenuAction>>(action)
            .unwrap() = false;
        app.update();
        assert!(!app.world().resource::<OverlayMenuState>().open);
        assert_eq!(
            app.world()
                .get::<AlsStateRequest>(character)
                .unwrap()
                .overlay_state,
            Some(custom)
        );
        app.world_mut()
            .entity_mut(character)
            .remove::<AlsPlayerCharacter>();
        **app
            .world_mut()
            .get_mut::<Action<OverlayMenuAction>>(action)
            .unwrap() = true;
        app.update();
        assert_eq!(
            app.world().resource::<OverlayMenuState>().selected,
            Some(custom)
        );
    }

    #[test]
    fn empty_menu_ignores_cycle_input() {
        let mut app = App::new();
        let mut tags = tag_manager(&[]);
        tags.tag_map.clear();
        app.add_plugins(MinimalPlugins)
            .insert_resource(tags)
            .add_plugins(OverlayModeMenuPlugin);
        let mut action = Action::<OverlayMenuAction>::new();
        *action = true;
        app.world_mut().spawn(action);
        app.update();
        app.world_mut().spawn((
            Action::<crate::input::NextOverlayModeAction>::new(),
            TriggerState::Fired,
        ));
        app.update();
        let menu = app.world().resource::<OverlayMenuState>();
        assert!(menu.open && menu.options.is_empty());
        assert!(menu.selected.is_none());
    }

    #[test]
    fn cycle_input_preserves_both_source_action_callbacks() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(tag_manager(&[]))
            .add_plugins(OverlayModeMenuPlugin);
        app.world_mut()
            .spawn((AlsCharacter, AlsPlayerCharacter, OverlayState::Rifle));
        let mut action = Action::<OverlayMenuAction>::new();
        *action = true;
        app.world_mut().spawn(action);
        app.update();
        app.world_mut().spawn((
            Action::<crate::input::NextOverlayModeAction>::new(),
            TriggerState::Fired,
        ));
        app.world_mut().spawn((
            Action::<crate::input::PreviousOverlayModeAction>::new(),
            TriggerState::Fired,
        ));
        app.update();
        assert_eq!(
            app.world().resource::<OverlayMenuState>().selected,
            Some(OverlayState::Rifle)
        );
    }

    #[test]
    fn menu_widget_is_initialized_once_and_reused_between_viewport_entries() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(tag_manager(&[]))
            .add_plugins(OverlayModeMenuPlugin);
        let action = app
            .world_mut()
            .spawn(Action::<OverlayMenuAction>::new())
            .id();
        app.update();
        let root = app
            .world()
            .resource::<OverlayMenuState>()
            .root
            .expect("The controller creates the menu during initialization");
        let options: Vec<_> = app.world().get::<Children>(root).unwrap().iter().collect();
        for held in [true, false, true, false] {
            **app
                .world_mut()
                .get_mut::<Action<OverlayMenuAction>>(action)
                .unwrap() = held;
            app.update();
            assert_eq!(app.world().resource::<OverlayMenuState>().root, Some(root));
            assert_eq!(
                app.world().get::<Node>(root).unwrap().display,
                if held { Display::Flex } else { Display::None }
            );
            assert_eq!(
                app.world()
                    .get::<Children>(root)
                    .unwrap()
                    .iter()
                    .collect::<Vec<_>>(),
                options
            );
        }
    }

    #[test]
    fn menu_widget_uses_full_backdrop_native_padding_and_font_units() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(tag_manager(&[]))
            .add_plugins(OverlayModeMenuPlugin);
        let mut action = Action::<OverlayMenuAction>::new();
        *action = true;
        app.world_mut().spawn(action);
        app.update();
        let root = app.world().resource::<OverlayMenuState>().root.unwrap();
        assert_eq!(
            app.world()
                .get::<BackgroundColor>(root)
                .map(|color| color.0),
            Some(crate::als_hud::COLOR_BORDER_WEAK)
        );
        for (node, children) in app
            .world_mut()
            .query_filtered::<(&Node, &Children), With<OverlayMenuOption>>()
            .iter(app.world())
        {
            assert_eq!(node.width, Val::Px(200.0));
            assert_eq!(node.height, Val::Px(25.0));
            assert_eq!(node.padding, UiRect::horizontal(Val::Px(4.0)));
            let font = app.world().get::<TextFont>(children[0]).unwrap();
            assert_eq!(font.font_size, FontSize::Px(56.0 / 3.0));
            assert_eq!(
                app.world()
                    .get::<TextLayout>(children[0])
                    .unwrap()
                    .linebreak,
                LineBreak::NoWrap
            );
        }
    }

    #[test]
    fn long_menu_labels_clip_to_the_padded_option() {
        use bevy::ecs::system::RunSystemOnce;
        let mut app = crate::als_hud::tests::hud_layout_app();
        app.init_resource::<OverlayMenuState>();
        app.insert_resource(tag_manager(&[
            "Als.OverlayMode.ExtremelyLongCustomOverlayModeThatExceedsTheMenuWidth",
        ]));
        app.world_mut().run_system_once(initialize_menu).unwrap();
        let root = app.world().resource::<OverlayMenuState>().root.unwrap();
        app.world_mut().get_mut::<Node>(root).unwrap().display = Display::Flex;
        app.add_systems(
            PostUpdate,
            bevy::ui::update::update_clipping_system.after(bevy::ui::ui_layout_system),
        );
        app.update();
        let long = app
            .world_mut()
            .query::<(Entity, &Text)>()
            .iter(app.world())
            .find(|(_, text)| text.0.starts_with("Extremely"))
            .unwrap()
            .0;
        app.world_mut().get_mut::<Node>(long).unwrap().width = Val::Auto;
        app.world_mut()
            .entity_mut(long)
            .insert(bevy::ui::ContentSize::fixed_size(Vec2::new(400.0, 20.0)));
        app.update();
        let mut found = false;
        for (_, children) in app
            .world_mut()
            .query::<(&OverlayMenuOption, &Children)>()
            .iter(app.world())
        {
            let text = app.world().get::<Text>(children[0]).unwrap();
            if !text.0.starts_with("Extremely") {
                continue;
            }
            found = true;
            let node = app.world().get::<ComputedNode>(children[0]).unwrap();
            assert!(node.size().x <= 192.0);
            let clip = app
                .world()
                .get::<CalculatedClip>(children[0])
                .expect("The source text uses on-demand clipping");
            assert!(
                clip.rects()
                    .unwrap()
                    .iter()
                    .any(|rect| rect.rect.width() <= 192.0)
            );
        }
        assert!(found);
    }

    #[test]
    fn menu_overflow_uses_native_font_measurement_and_an_ellipsis() {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            bevy::asset::AssetPlugin::default(),
            bevy::text::TextPlugin,
        ))
        .insert_resource(tag_manager(&[
            "Als.OverlayMode.ExtremelyLongCustomOverlayModeThatExceedsTheMenuWidth",
        ]))
        .add_plugins(OverlayModeMenuPlugin);
        for _ in 0..3 {
            app.update();
        }
        let label = app
            .world_mut()
            .query::<&Text>()
            .iter(app.world())
            .find(|text| text.0.starts_with("Extremely"))
            .unwrap();
        assert!(
            label.0.ends_with('\u{2026}'),
            "Overflow text must end with the source ellipsis"
        );
        assert!(
            label.0.len()
                < option_label(
                    *app.world()
                        .resource::<OverlayMenuState>()
                        .options
                        .iter()
                        .find(|tag| tag.as_str().contains("Extremely"))
                        .unwrap()
                )
                .len()
        );
        app.insert_resource(UiScale(0.4995));
        app.update();
        assert!(
            app.world_mut()
                .query::<&Text>()
                .iter(app.world())
                .any(|text| text.0 == "Pistol One Handed"),
            "A fitting label must survive fractional DPI without rounding its glyph advance upward"
        );
    }

    #[test]
    fn option_labels_match_ue_name_display() {
        assert_eq!(option_label(OverlayState::Default), "Default");
        assert_eq!(
            option_label(OverlayState::PistolOneHanded),
            "Pistol One Handed"
        );
        assert_eq!(
            option_label(OverlayState::PistolTwoHanded),
            "Pistol Two Handed"
        );
        assert_eq!(option_label(OverlayState::HandsTied), "Hands Tied");
    }

    #[test]
    fn selection_background_preserves_ue_linear_color_contrast() {
        let selected = COLOR_OPTION_SELECTED.to_linear();
        let unselected = COLOR_OPTION_NOT_SELECTED.to_linear();
        assert!((selected.red - 0.15).abs() < 1e-6);
        assert!((unselected.red - 0.015).abs() < 1e-6);
        assert!((selected.alpha - 0.5).abs() < 1e-6);
        assert!((unselected.alpha - 0.5).abs() < 1e-6);
        assert!(selected.red / unselected.red >= 9.9);
    }
}
