use crate::input::SlomoAction;
use crate::input::ToggleUiAction;
use bevy_als::show_debug_hud::{AlsShowDebugHudState, DebugCategory};
use bevy::prelude::*;
use bevy::ui::Display;
use bevy_enhanced_input::prelude::*;

include!("als_hud_generated.rs");
pub const LABEL_GROW: f32 = 1.0;
pub const KEY_CELL_GROW: f32 = 0.7;
pub const DEBUG_KEY_CELL_GROW: f32 = 0.6;
pub const SLMO_TIME_SCALE: f32 = 0.15;

pub const COLOR_ENABLED: Color = Color::linear_rgba(0.0, 1.0, 0.0, 1.0);
pub const COLOR_DISABLED: Color = Color::linear_rgba(0.5, 0.5, 0.5, 1.0);
pub const COLOR_TITLE_ACCENT: Color = Color::linear_rgba(1.0, 0.666_667, 0.0, 1.0);
pub const COLOR_TEXT: Color = Color::linear_rgba(1.0, 1.0, 1.0, 1.0);
pub const COLOR_BORDER_WEAK: Color = Color::linear_rgba(0.015, 0.015, 0.015, 0.25);
pub const COLOR_BORDER_STRONG: Color = Color::linear_rgba(0.015, 0.015, 0.015, 0.5);

#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlsHudState {
    pub ui_visible: bool,
    pub slomo_active: bool,
}

impl Default for AlsHudState {
    fn default() -> Self {
        Self {
            ui_visible: true,
            slomo_active: false,
        }
    }
}

impl AlsHudState {
    pub fn toggle_color(active: bool) -> Color {
        if active {
            COLOR_ENABLED
        } else {
            COLOR_DISABLED
        }
    }
}

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlsHudColorTarget {
    UiToggle,
    Slomo,
    Category(DebugCategory),
}

#[derive(Component)]
pub struct AlsHudDebugSection;

#[derive(Component)]
pub struct AlsHudInputSection;

#[derive(Component)]
pub struct AlsHudRoot;

pub const INPUT_ROWS: [(&str, &str, &str); 14] = [
    ("Movement", "WASD", "Left Stick"),
    ("Camera", "Mouse", "Right Stick"),
    ("Sprint", "Hold Left Shift", "Hold B"),
    ("Roll", "Left Shift", "B"),
    ("Walk", "Left Alt", "RB"),
    ("Crouch", "Left Ctrl", "LSB"),
    ("Jump / Mantle", "Space", "A"),
    ("Aim", "RMB", "LB"),
    ("Ragdoll", "X", "X"),
    ("Rotation Mode", "1", "D-pad Left"),
    ("View Mode", "2", "D-pad Right"),
    ("Switch Shoulder", "C", "RSB"),
    ("Overlay Menu", "Q", "Y"),
    ("Cycle Overlay", "Mouse Wheel", "D-pad U / D"),
];

pub const DEBUG_ROWS: [(&str, &str, Option<DebugCategory>); 10] = [
    ("Toggle Slomo", "Z", None),
    ("Toggle Curves", "Shift + 1", Some(DebugCategory::Curves)),
    ("Toggle State", "Shift + 2", Some(DebugCategory::State)),
    ("Toggle Shapes", "Shift + 3", Some(DebugCategory::Shapes)),
    ("Toggle Traces", "Shift + 4", Some(DebugCategory::Traces)),
    (
        "Toggle Mantling",
        "Shift + 5",
        Some(DebugCategory::Mantling),
    ),
    (
        "Toggle Camera Curves",
        "Shift + 6",
        Some(DebugCategory::CameraCurves),
    ),
    (
        "Toggle Camera Shapes",
        "Shift + 7",
        Some(DebugCategory::CameraShapes),
    ),
    (
        "Toggle Camera Traces",
        "Shift + 8",
        Some(DebugCategory::CameraTraces),
    ),
    ("Toggle Debug", "Shift + 0", None),
];

pub fn slomo_time_scale(slomo_active: bool) -> f32 {
    if slomo_active { SLMO_TIME_SCALE } else { 1.0 }
}

pub struct AlsHudPlugin;

impl Plugin for AlsHudPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<bevy_als::debug_target::AlsDebugTargetPlugin>() {
            app.add_plugins(bevy_als::debug_target::AlsDebugTargetPlugin);
        }
        app.init_resource::<AlsHudState>()
            .init_resource::<UiScale>()
            .add_systems(Startup, spawn_hud)
            .add_systems(
                Update,
                (
                    sync_ui_scale,
                    toggle_ui_system,
                    slomo_system.after(bevy_als::als::view::AlsViewInputSet),
                    refresh_colors_system
                        .after(slomo_system)
                        .after(toggle_ui_system)
                        .after(bevy_als::show_debug_hud::AlsShowDebugHudSet::Input)
                        .after(bevy_als::debug_target::AlsDebugTargetSet),
                ),
            );
    }
}

pub fn viewport_ui_scale(size: UVec2) -> f32 {
    let shortest = size.min_element() as f32;
    let mut value = UI_SCALE_CURVE[0].1;
    for pair in UI_SCALE_CURVE.windows(2) {
        if shortest < pair[0].0 {
            break;
        }
        let alpha = ((shortest - pair[0].0) / (pair[1].0 - pair[0].0)).clamp(0.0, 1.0);
        value = pair[0].1 + (pair[1].1 - pair[0].1) * alpha;
    }
    value.max(0.01)
}

fn sync_ui_scale(
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    mut scale: ResMut<UiScale>,
) {
    if let Ok(window) = windows.single() {
        let value = viewport_ui_scale(window.physical_size()) / window.scale_factor();
        if scale.0 != value {
            scale.0 = value;
        }
    }
}

fn spawn_hud(mut commands: Commands) {
    let root = commands
        .spawn((
            AlsHudRoot,
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(PANEL_RIGHT_MARGIN_PX),
                top: Val::Px(PANEL_TOP_MARGIN_PX),
                width: Val::Px(PANEL_WIDTH_PX),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(SECTION_GAP_PX),
                ..default()
            },
            Pickable::IGNORE,
            GlobalZIndex(10),
        ))
        .id();

    spawn_header_section(&mut commands, root);
    spawn_debug_section(&mut commands, root);
    spawn_input_section(&mut commands, root);
}

fn section_node() -> Node {
    Node {
        flex_direction: FlexDirection::Column,
        ..default()
    }
}

fn row_node(columns: &[f32]) -> Node {
    Node {
        display: Display::Grid,
        grid_template_columns: columns
            .iter()
            .map(|weight| GridTrack::flex(*weight))
            .collect(),
        column_gap: Val::Px(CELL_GAP_PX),
        margin: UiRect::top(Val::Px(ROW_GAP_PX)),
        ..default()
    }
}

fn cell_node(centered: bool) -> Node {
    Node {
        padding: if centered {
            UiRect::ZERO
        } else {
            UiRect::left(Val::Px(CELL_GAP_PX))
        },
        align_items: AlignItems::Center,
        justify_content: if centered {
            JustifyContent::Center
        } else {
            JustifyContent::FlexStart
        },
        ..default()
    }
}

fn text_node(text: &str, size: f32, italic: bool, bold: bool, color: Color) -> impl Bundle {
    (
        Text::new(text),
        TextLayout::default().with_no_wrap(),
        TextFont {
            font_size: FontSize::Px(size),
            weight: if bold {
                FontWeight::BOLD
            } else {
                FontWeight::NORMAL
            },
            style: if italic {
                FontStyle::Italic
            } else {
                FontStyle::Normal
            },
            ..default()
        },
        TextColor(color),
    )
}

fn spawn_header_section(commands: &mut Commands, root: Entity) {
    let section = commands
        .spawn((section_node(), BackgroundColor(COLOR_BORDER_WEAK)))
        .id();

    let title_border = commands
        .spawn((
            Node {
                justify_content: JustifyContent::Center,
                ..default()
            },
            BackgroundColor(COLOR_BORDER_STRONG),
        ))
        .with_children(|parent| {
            parent
                .spawn(Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    ..default()
                })
                .with_children(|row| {
                    row.spawn(text_node(
                        "Advanced Locomotion System ",
                        FONT_SIZE_DEFAULT_PX,
                        true,
                        true,
                        COLOR_TEXT,
                    ));
                    row.spawn(text_node(
                        "Refactored",
                        FONT_SIZE_DEFAULT_PX,
                        true,
                        false,
                        COLOR_TITLE_ACCENT,
                    ));
                });
        })
        .id();

    let platform_row = commands
        .spawn(row_node(&[LABEL_GROW, KEY_CELL_GROW, KEY_CELL_GROW]))
        .with_children(|row| {
            row.spawn((cell_node(false), BackgroundColor(COLOR_BORDER_STRONG)));
            row.spawn((cell_node(true), BackgroundColor(COLOR_BORDER_STRONG)))
                .with_children(|cell| {
                    cell.spawn(text_node(
                        "PC",
                        FONT_SIZE_DEFAULT_PX,
                        true,
                        false,
                        COLOR_TEXT,
                    ));
                });
            row.spawn((cell_node(true), BackgroundColor(COLOR_BORDER_STRONG)))
                .with_children(|cell| {
                    cell.spawn(text_node(
                        "Xbox",
                        FONT_SIZE_DEFAULT_PX,
                        true,
                        false,
                        COLOR_TEXT,
                    ));
                });
        })
        .id();

    let toggle_row = commands
        .spawn(row_node(&[LABEL_GROW, KEY_CELL_GROW, KEY_CELL_GROW]))
        .with_children(|row| {
            row.spawn((cell_node(false), BackgroundColor(COLOR_BORDER_STRONG)))
                .with_children(|cell| {
                    cell.spawn(text_node(
                        "Toggle UI",
                        FONT_SIZE_DEFAULT_PX,
                        false,
                        false,
                        COLOR_TEXT,
                    ));
                });
            row.spawn((cell_node(true), BackgroundColor(COLOR_BORDER_STRONG)))
                .with_children(|cell| {
                    cell.spawn((
                        text_node("Tab", FONT_SIZE_DEFAULT_PX, true, false, COLOR_ENABLED),
                        AlsHudColorTarget::UiToggle,
                    ));
                });
            row.spawn((cell_node(true), BackgroundColor(COLOR_BORDER_STRONG)))
                .with_children(|cell| {
                    cell.spawn((
                        text_node("View", FONT_SIZE_DEFAULT_PX, false, false, COLOR_ENABLED),
                        AlsHudColorTarget::UiToggle,
                    ));
                });
        })
        .id();

    commands
        .entity(section)
        .add_child(title_border)
        .add_child(platform_row)
        .add_child(toggle_row);
    commands.entity(root).add_child(section);
}

fn spawn_debug_section(commands: &mut Commands, root: Entity) {
    let section = commands
        .spawn((
            section_node(),
            BackgroundColor(COLOR_BORDER_WEAK),
            AlsHudDebugSection,
        ))
        .id();

    let header = commands
        .spawn((
            Node {
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(COLOR_BORDER_STRONG),
        ))
        .with_children(|parent| {
            parent.spawn(text_node(
                "Debug",
                FONT_SIZE_DEFAULT_PX,
                false,
                true,
                COLOR_TEXT,
            ));
        })
        .id();
    commands.entity(section).add_child(header);

    for (label, key, category) in DEBUG_ROWS {
        let row = commands
            .spawn(row_node(if label == "Toggle Slomo" {
                &[LABEL_GROW, KEY_CELL_GROW, KEY_CELL_GROW]
            } else {
                &[LABEL_GROW, DEBUG_KEY_CELL_GROW]
            }))
            .id();
        commands.entity(row).with_children(|row_parent| {
            row_parent
                .spawn((cell_node(false), BackgroundColor(COLOR_BORDER_STRONG)))
                .with_children(|cell| {
                    cell.spawn(text_node(
                        label,
                        FONT_SIZE_DEFAULT_PX,
                        false,
                        false,
                        COLOR_TEXT,
                    ));
                });
            if label == "Toggle Slomo" {
                row_parent
                    .spawn((cell_node(true), BackgroundColor(COLOR_BORDER_STRONG)))
                    .with_children(|cell| {
                        cell.spawn((
                            text_node(key, FONT_SIZE_DEFAULT_PX, true, false, COLOR_DISABLED),
                            AlsHudColorTarget::Slomo,
                        ));
                    });
                row_parent
                    .spawn((cell_node(true), BackgroundColor(COLOR_BORDER_STRONG)))
                    .with_children(|cell| {
                        cell.spawn((
                            text_node("Menu", FONT_SIZE_DEFAULT_PX, false, false, COLOR_DISABLED),
                            AlsHudColorTarget::Slomo,
                        ));
                    });
            } else {
                row_parent
                    .spawn((cell_node(true), BackgroundColor(COLOR_BORDER_STRONG)))
                    .with_children(|cell| {
                        let mut text = cell.spawn(text_node(
                            key,
                            FONT_SIZE_DEFAULT_PX,
                            true,
                            false,
                            COLOR_TEXT,
                        ));
                        if let Some(category) = category {
                            text.insert(AlsHudColorTarget::Category(category));
                        }
                    });
            }
        });
        commands.entity(section).add_child(row);
    }

    commands.entity(root).add_child(section);
}

fn spawn_input_section(commands: &mut Commands, root: Entity) {
    let section = commands
        .spawn((
            section_node(),
            BackgroundColor(COLOR_BORDER_WEAK),
            AlsHudInputSection,
        ))
        .id();

    let header = commands
        .spawn((
            Node {
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(COLOR_BORDER_STRONG),
        ))
        .with_children(|parent| {
            parent.spawn(text_node(
                "Input",
                FONT_SIZE_DEFAULT_PX,
                false,
                true,
                COLOR_TEXT,
            ));
        })
        .id();
    commands.entity(section).add_child(header);

    for (label, pc, xbox) in INPUT_ROWS {
        let row = commands
            .spawn(row_node(&[LABEL_GROW, KEY_CELL_GROW, KEY_CELL_GROW]))
            .with_children(|row_parent| {
                row_parent
                    .spawn((cell_node(false), BackgroundColor(COLOR_BORDER_STRONG)))
                    .with_children(|cell| {
                        cell.spawn(text_node(
                            label,
                            FONT_SIZE_DEFAULT_PX,
                            false,
                            false,
                            COLOR_TEXT,
                        ));
                    });
                row_parent
                    .spawn((cell_node(true), BackgroundColor(COLOR_BORDER_STRONG)))
                    .with_children(|cell| {
                        cell.spawn(text_node(pc, FONT_SIZE_VALUE_PX, false, false, COLOR_TEXT));
                    });
                row_parent
                    .spawn((cell_node(true), BackgroundColor(COLOR_BORDER_STRONG)))
                    .with_children(|cell| {
                        cell.spawn(text_node(
                            xbox,
                            FONT_SIZE_VALUE_PX,
                            false,
                            false,
                            COLOR_TEXT,
                        ));
                    });
            })
            .id();
        commands.entity(section).add_child(row);
    }

    commands.entity(root).add_child(section);
}

fn toggle_ui_system(
    time: Res<Time<Virtual>>,
    events: Query<&ActionEvents, With<Action<ToggleUiAction>>>,
    mut state: ResMut<AlsHudState>,
    mut debug_sections: Query<&mut Node, (With<AlsHudDebugSection>, Without<AlsHudInputSection>)>,
    mut input_sections: Query<&mut Node, With<AlsHudInputSection>>,
) {
    let pressed = !time.is_paused()
        && events
            .iter()
            .any(|events| events.contains(ActionEvents::START));
    if pressed {
        state.ui_visible = !state.ui_visible;
    }
    let display = if state.ui_visible {
        Display::Flex
    } else {
        Display::None
    };
    for mut node in &mut debug_sections {
        if node.display != display {
            node.display = display;
        }
    }
    for mut node in &mut input_sections {
        if node.display != display {
            node.display = display;
        }
    }
}

fn slomo_system(
    events: Query<&ActionEvents, With<Action<SlomoAction>>>,
    mut state: ResMut<AlsHudState>,
    mut time: ResMut<Time<Virtual>>,
) {
    let pressed = !time.is_paused()
        && events
            .iter()
            .any(|events| events.contains(ActionEvents::START));
    if !pressed {
        return;
    }
    state.slomo_active = !state.slomo_active;
    time.set_relative_speed(slomo_time_scale(state.slomo_active));
}

fn refresh_colors_system(
    state: Res<AlsHudState>,
    show_debug: Option<Res<AlsShowDebugHudState>>,
    owned_target: Query<
        (),
        (
            With<bevy_als::debug_target::AlsDebugTarget>,
            With<bevy_als::character_assembly::AlsPlayerCharacter>,
        ),
    >,
    mut targets: Query<(&AlsHudColorTarget, &mut TextColor)>,
) {
    for (target, mut color) in &mut targets {
        let active = match *target {
            AlsHudColorTarget::UiToggle => state.ui_visible,
            AlsHudColorTarget::Slomo => state.slomo_active,
            AlsHudColorTarget::Category(category) => show_debug
                .as_ref()
                .map(|debug| debug.is_active(category) && !owned_target.is_empty())
                .unwrap_or(false),
        };
        color.0 = AlsHudState::toggle_color(active);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    #[test]
    fn owning_player_legend_does_not_claim_another_targets_categories() {
        use bevy_als::{AlsCharacter, character_assembly::AlsPlayerCharacter, debug_target::AlsLastDebugRender};
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, crate::als_hud::AlsHudPlugin))
            .init_resource::<ButtonInput<KeyCode>>()
            .insert_resource(AlsShowDebugHudState {
                visible: true,
                state: true,
                ..default()
            });
        app.world_mut()
            .spawn((AlsCharacter, AlsPlayerCharacter, AlsLastDebugRender(0.0)));
        app.world_mut()
            .spawn((AlsCharacter, AlsLastDebugRender(0.0)));
        app.update();
        let state_color = |world: &mut World| {
            world
                .query::<(&crate::als_hud::AlsHudColorTarget, &TextColor)>()
                .iter(world)
                .find(|(target, _)| {
                    **target
                        == crate::als_hud::AlsHudColorTarget::Category(
                            bevy_als::show_debug_hud::DebugCategory::State,
                        )
                })
                .unwrap()
                .1
                .0
        };
        assert_eq!(state_color(app.world_mut()), crate::als_hud::COLOR_ENABLED);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::PageUp);
        app.update();
        assert_eq!(state_color(app.world_mut()), crate::als_hud::COLOR_DISABLED);
    }

    use super::*;

    pub(crate) fn hud_layout_app() -> App {
        use bevy::{
            app::{HierarchyPropagatePlugin, PropagateSet},
            ui::{
                ComputedUiRenderTargetInfo, ComputedUiTargetCamera, ui_layout_system,
                update::propagate_ui_target_cameras,
            },
        };
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            TransformPlugin,
            AlsHudPlugin,
            HierarchyPropagatePlugin::<ComputedUiTargetCamera>::new(PostUpdate),
            HierarchyPropagatePlugin::<ComputedUiRenderTargetInfo>::new(PostUpdate),
        ))
        .init_resource::<AlsShowDebugHudState>()
        .init_resource::<UiScale>()
        .init_resource::<bevy::ui::ui_surface::UiSurface>()
        .init_resource::<bevy::text::RemSize>()
        .init_resource::<bevy::text::TextPipeline>()
        .init_resource::<bevy::text::FontCx>()
        .init_resource::<bevy::text::ScaleCx>()
        .add_systems(
            PostUpdate,
            (propagate_ui_target_cameras, ui_layout_system).chain(),
        )
        .configure_sets(
            PostUpdate,
            (
                PropagateSet::<ComputedUiTargetCamera>::default(),
                PropagateSet::<ComputedUiRenderTargetInfo>::default(),
            )
                .after(propagate_ui_target_cameras)
                .before(ui_layout_system),
        )
        .add_systems(
            Update,
            |mut texts: Query<(&Text, &mut Node), Added<Text>>| {
                for (text, mut node) in &mut texts {
                    node.width = Val::Px(text.0.len() as f32 * 6.0);
                    node.height = Val::Px(16.0);
                }
            },
        );
        let mut camera = Camera::default();
        camera.computed.target_info = Some(bevy::camera::RenderTargetInfo {
            physical_size: UVec2::new(1280, 720),
            scale_factor: 1.0,
        });
        camera.viewport = Some(bevy::camera::Viewport {
            physical_size: UVec2::new(1280, 720),
            ..default()
        });
        app.world_mut().spawn((Camera2d, camera));
        app.update();
        app
    }

    #[test]
    fn paused_ui_pressed_actions_preserve_source_hud_and_time_state() {
        use crate::input::{AlsInputMap, AlsInputPlugin, spawn_als_actions};
        use bevy::input::gamepad::{RawGamepadButtonChangedEvent, RawGamepadEvent};
        for gamepad_input in [false, true] {
            let mut app = App::new();
            app.add_plugins((
                MinimalPlugins,
                bevy::input::InputPlugin,
                EnhancedInputPlugin,
                AlsInputPlugin,
            ))
            .init_resource::<AlsHudState>()
            .add_systems(Update, (toggle_ui_system, slomo_system));
            app.finish();
            let player = app.world_mut().spawn_empty().id();
            let gamepad = app.world_mut().spawn(Gamepad::default()).id();
            let debug = app
                .world_mut()
                .spawn((Node::default(), AlsHudDebugSection))
                .id();
            let input = app
                .world_mut()
                .spawn((Node::default(), AlsHudInputSection))
                .id();
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
                    (KeyCode::Tab, GamepadButton::Select),
                    (KeyCode::KeyZ, GamepadButton::Start),
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
                assert_eq!(
                    *app.world().resource::<AlsHudState>(),
                    AlsHudState {
                        ui_visible: !expected,
                        slomo_active: expected
                    },
                    "Source paused UI Pressed actions must preserve HUD and Slomo state"
                );
                assert_eq!(
                    app.world().resource::<Time<Virtual>>().relative_speed(),
                    if expected { SLMO_TIME_SCALE } else { 1.0 }
                );
                for section in [debug, input] {
                    assert_eq!(
                        app.world().get::<Node>(section).unwrap().display,
                        if expected {
                            Display::None
                        } else {
                            Display::Flex
                        }
                    );
                }
                if paused {
                    app.world_mut().resource_mut::<Time<Virtual>>().unpause();
                    app.update();
                    assert_eq!(
                        *app.world().resource::<AlsHudState>(),
                        AlsHudState::default(),
                        "Source paused UI Pressed history must not replay after resume"
                    );
                }
            }
            app.update();
            assert_eq!(
                *app.world().resource::<AlsHudState>(),
                AlsHudState {
                    ui_visible: false,
                    slomo_active: true
                },
                "Source held UI Pressed must not toggle again"
            );
        }
    }
    #[test]
    fn source_ui_bindings_reach_hud_and_virtual_time_once_per_press() {
        use crate::input::{AlsInputMap, AlsInputPlugin, spawn_als_actions};
        for custom in [false, true] {
            for (button, key, toggle_ui) in [
                (
                    Some(if custom {
                        GamepadButton::Start
                    } else {
                        GamepadButton::Select
                    }),
                    None,
                    true,
                ),
                (
                    Some(if custom {
                        GamepadButton::Select
                    } else {
                        GamepadButton::Start
                    }),
                    None,
                    false,
                ),
                (None, Some(KeyCode::Tab), true),
                (None, Some(KeyCode::KeyZ), false),
            ] {
                let mut app = App::new();
                app.add_plugins((
                    MinimalPlugins,
                    bevy::input::InputPlugin,
                    EnhancedInputPlugin,
                    AlsInputPlugin,
                ))
                .init_resource::<AlsHudState>()
                .add_systems(Update, (toggle_ui_system, slomo_system));
                app.finish();
                let gamepad = app.world_mut().spawn(Gamepad::default()).id();
                let player = app.world_mut().spawn_empty().id();
                let debug = app
                    .world_mut()
                    .spawn((Node::default(), AlsHudDebugSection))
                    .id();
                let input = app
                    .world_mut()
                    .spawn((Node::default(), AlsHudInputSection))
                    .id();
                let mut map = AlsInputMap::default();
                if custom {
                    map.toggle_ui = vec![KeyCode::Tab.into(), GamepadButton::Start.into()];
                    map.slomo = vec![KeyCode::KeyZ.into(), GamepadButton::Select.into()];
                }
                spawn_als_actions(&mut app.world_mut().commands(), player, &map);
                app.update();
                app.update();
                for (frame, pressed) in [true, true, false, true].into_iter().enumerate() {
                    if let Some(button) = button {
                        app.world_mut()
                            .get_mut::<Gamepad>(gamepad)
                            .unwrap()
                            .analog_mut()
                            .set(button, if pressed { 1.0 } else { 0.0 });
                    }
                    if let Some(key) = key {
                        let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
                        if pressed {
                            keys.press(key);
                        } else {
                            keys.release(key);
                        }
                    }
                    app.update();
                    let state = *app.world().resource::<AlsHudState>();
                    let active = frame < 3;
                    assert_eq!(
                        state.ui_visible,
                        !(toggle_ui && active),
                        "custom={custom} button={button:?} key={key:?} frame={frame}: {state:?}"
                    );
                    assert_eq!(state.slomo_active, !toggle_ui && active);
                    assert_eq!(
                        app.world().resource::<Time<Virtual>>().relative_speed(),
                        if !toggle_ui && active {
                            SLMO_TIME_SCALE
                        } else {
                            1.0
                        }
                    );
                    for section in [debug, input] {
                        assert_eq!(
                            app.world().get::<Node>(section).unwrap().display,
                            if toggle_ui && active {
                                Display::None
                            } else {
                                Display::Flex
                            }
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn widget_slots_fill_headers_and_keep_padding_inside_the_first_cell() {
        let mut app = hud_layout_app();
        let root = app
            .world_mut()
            .query_filtered::<Entity, With<AlsHudRoot>>()
            .single(app.world())
            .unwrap();
        let rectangle = |world: &World, entity: Entity| {
            let size = world.get::<ComputedNode>(entity).unwrap().size();
            let center = world
                .get::<UiGlobalTransform>(entity)
                .unwrap()
                .affine()
                .translation;
            (center - size * 0.5, size)
        };
        let (origin, root_size) = rectangle(app.world(), root);
        assert_eq!(root_size.x, 340.0);
        let mut row_index = 0;
        let mut section_end = origin.y - SECTION_GAP_PX;
        for section in app.world().get::<Children>(root).unwrap().iter() {
            let (section_position, section_size) = rectangle(app.world(), section);
            assert_eq!(section_position.y - section_end, SECTION_GAP_PX);
            section_end = section_position.y + section_size.y;
            let rows = app.world().get::<Children>(section).unwrap();
            let (header_position, header_size) = rectangle(app.world(), rows[0]);
            assert_eq!(
                header_position.x, origin.x,
                "Header background must begin at the panel edge"
            );
            assert_eq!(
                header_size.x, root_size.x,
                "Header background must fill its section"
            );
            let mut row_end = header_position.y + header_size.y;
            for row in rows.iter().skip(1) {
                let (row_position, row_size) = rectangle(app.world(), row);
                assert_eq!(row_position.y - row_end, ROW_GAP_PX);
                row_end = row_position.y + row_size.y;
                let cells = app.world().get::<Children>(row).unwrap();
                let contract = ROW_SLOT_CONTRACT[row_index];
                row_index += 1;
                assert_eq!(cells.len(), contract.len());
                let available = root_size.x - contract.iter().map(|cell| cell.1).sum::<f32>();
                let weights = contract.iter().map(|cell| cell.0).sum::<f32>();
                for (cell, source) in cells.iter().zip(contract) {
                    assert!(
                        (rectangle(app.world(), cell).1.x - available * source.0 / weights).abs()
                            <= 1.0,
                        "Column width differs from the reference slot allocation"
                    );
                }
                let mut previous_end = origin.x;
                for (index, cell) in cells.iter().enumerate() {
                    let (position, size) = rectangle(app.world(), cell);
                    assert_eq!(
                        position.x - previous_end,
                        if index == 0 { 0.0 } else { 4.0 },
                        "Only adjacent cells have an outer gap"
                    );
                    if index == 0
                        && let Some(text) = app
                            .world()
                            .get::<Children>(cell)
                            .and_then(|children| children.first())
                    {
                        assert_eq!(
                            rectangle(app.world(), *text).0.x - position.x,
                            4.0,
                            "Label inset belongs inside the border"
                        );
                    }
                    previous_end = position.x + size.x;
                }
                assert_eq!(previous_end, origin.x + root_size.x);
            }
        }
        assert_eq!(row_index, ROW_SLOT_CONTRACT.len());
    }

    #[test]
    fn widget_master_key_is_static_and_visibility_tracks_the_controller_state() {
        let mut app = hud_layout_app();
        for active in [false, true, false] {
            app.world_mut()
                .resource_mut::<AlsShowDebugHudState>()
                .visible = active;
            app.world_mut().resource_mut::<AlsHudState>().ui_visible = active;
            app.update();
            let color = app
                .world_mut()
                .query::<(&Text, &TextColor)>()
                .iter(app.world())
                .find(|(text, _)| text.0 == "Shift + 0")
                .unwrap()
                .1
                .0;
            assert_eq!(
                color, COLOR_TEXT,
                "The master key has no color delegate in W_Als_Hud"
            );
            for node in app
                .world_mut()
                .query_filtered::<&Node, Or<(With<AlsHudDebugSection>, With<AlsHudInputSection>)>>()
                .iter(app.world())
            {
                assert_eq!(
                    node.display,
                    if active { Display::Flex } else { Display::None }
                );
            }
        }
    }

    #[test]
    fn complete_text_tree_matches_native_widget_styles_and_order() {
        fn collect(world: &World, entity: Entity, result: &mut Vec<(String, f32, bool, bool)>) {
            if let Some(text) = world.get::<Text>(entity) {
                let font = world.get::<TextFont>(entity).unwrap();
                let FontSize::Px(size) = font.font_size else {
                    panic!("Expected pixel size")
                };
                assert_eq!(
                    world.get::<TextLayout>(entity).unwrap().linebreak,
                    LineBreak::NoWrap
                );
                result.push((
                    text.0.clone(),
                    size,
                    font.style == FontStyle::Italic,
                    font.weight == FontWeight::BOLD,
                ));
            }
            if let Some(children) = world.get::<Children>(entity) {
                for child in children.iter() {
                    collect(world, child, result);
                }
            }
        }
        let mut app = hud_layout_app();
        let root = app
            .world_mut()
            .query_filtered::<Entity, With<AlsHudRoot>>()
            .single(app.world())
            .unwrap();
        let mut actual = Vec::new();
        collect(app.world(), root, &mut actual);
        let expected: Vec<_> = TEXT_STYLE_CONTRACT
            .iter()
            .map(|(text, size, italic, bold)| (text.to_string(), *size, *italic, *bold))
            .collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn umg_scale_uses_physical_short_side_and_cancels_platform_dpi() {
        let mut app = App::new();
        app.init_resource::<UiScale>()
            .add_systems(Update, sync_ui_scale);
        let window = app
            .world_mut()
            .spawn((Window::default(), bevy::window::PrimaryWindow))
            .id();
        for (size, expected) in [
            (UVec2::new(1920, 1080), 1.0),
            (UVec2::new(1280, 720), 0.666),
            (UVec2::new(540, 960), 0.4995),
            (UVec2::new(320, 240), 0.444),
            (UVec2::splat(9000), 8.0),
        ] {
            for dpi in [1.0, 2.0] {
                let mut window = app.world_mut().get_mut::<Window>(window).unwrap();
                window.resolution.set_scale_factor_override(Some(dpi));
                window.resolution.set_physical_resolution(size.x, size.y);
                app.update();
                assert!((app.world().resource::<UiScale>().0 * dpi - expected).abs() < 1e-6);
            }
        }
    }

    #[test]
    fn slomo_scale_matches_ue_dilation() {
        assert_eq!(slomo_time_scale(false), 1.0);
        assert_eq!(slomo_time_scale(true), SLMO_TIME_SCALE);
        assert_eq!(SLMO_TIME_SCALE, 0.15);
    }

    #[test]
    fn toggle_color_matches_ue_enabled_disabled() {
        assert_eq!(AlsHudState::toggle_color(true), COLOR_ENABLED);
        assert_eq!(AlsHudState::toggle_color(false), COLOR_DISABLED);
    }

    #[test]
    fn source_colors_remain_linear() {
        let disabled = COLOR_DISABLED.to_linear();
        let accent = COLOR_TITLE_ACCENT.to_linear();
        let weak = COLOR_BORDER_WEAK.to_linear();
        let strong = COLOR_BORDER_STRONG.to_linear();
        assert!((disabled.red - 0.5).abs() < 1e-6);
        assert!((accent.green - 0.666_667).abs() < 1e-6);
        assert!((weak.red - 0.015).abs() < 1e-6);
        assert!((weak.alpha - 0.25).abs() < 1e-6);
        assert!((strong.red - 0.015).abs() < 1e-6);
        assert!((strong.alpha - 0.5).abs() < 1e-6);
    }

    #[test]
    fn default_state_matches_ue_player_controller_defaults() {
        let state = AlsHudState::default();
        assert!(state.ui_visible);
        assert!(!state.slomo_active);
    }

    #[test]
    fn debug_rows_cover_all_categories_plus_master_and_slomo() {
        let categories: Vec<_> = DEBUG_ROWS.iter().filter_map(|(_, _, c)| *c).collect();
        assert_eq!(categories.len(), 8);
        for category in DebugCategory::ALL {
            assert!(categories.contains(&category), "{category:?} missing");
        }
        assert_eq!(DEBUG_ROWS.len(), 10);
        assert_eq!(DEBUG_ROWS[0].0, "Toggle Slomo");
        assert_eq!(DEBUG_ROWS[9].0, "Toggle Debug");
    }

    #[test]
    fn input_rows_match_ue_panel() {
        assert_eq!(INPUT_ROWS.len(), 14);
        assert_eq!(INPUT_ROWS[0], ("Movement", "WASD", "Left Stick"));
        assert_eq!(
            INPUT_ROWS[13],
            ("Cycle Overlay", "Mouse Wheel", "D-pad U / D")
        );
    }
}
