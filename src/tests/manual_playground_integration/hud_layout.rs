use super::*;
use crate::als_hud::{AlsHudRoot, COLOR_TEXT, viewport_ui_scale};

fn viewport(elapsed: f32) -> (u32, UVec2, f32) {
    if elapsed < 1.3 {
        (0, UVec2::new(1280, 720), 1.0)
    } else if elapsed < 3.3 {
        (1, UVec2::new(960, 540), 1.0)
    } else {
        (2, UVec2::new(960, 540), 2.0)
    }
}

pub(super) fn install(app: &mut App) {
    app.add_systems(PreUpdate, resize.after(drive_manual_integration));
    app.add_systems(
        PostUpdate,
        observe
            .run_if(|state: Res<ManualIntegrationState>| {
                state.scenario == IntegrationScenario::Interface
            })
            .after(bevy::ui::UiSystems::PostLayout)
            .after(bevy_als::show_debug_hud::AlsShowDebugHudSet::Sync)
            .before(observe_manual_integration),
    );
}

fn resize(
    state: Res<ManualIntegrationState>,
    mut windows: Query<&mut Window, With<bevy::window::PrimaryWindow>>,
) {
    if state.scenario != IntegrationScenario::Interface
        || state.phase != IntegrationPhase::Interface
    {
        return;
    }
    let Ok(mut window) = windows.single_mut() else {
        return;
    };
    let (_, size, dpi) = viewport(state.phase_elapsed());
    if window.resolution.scale_factor_override() != Some(dpi) {
        window.resolution.set_scale_factor_override(Some(dpi));
    }
    if window.physical_size() != size {
        window.resolution.set_physical_resolution(size.x, size.y);
    }
}

fn observe(
    state: Res<ManualIntegrationState>,
    mut report: ResMut<ManualIntegrationGateReport>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    nodes: Query<(
        &Name,
        &ComputedNode,
        &bevy::ui::UiGlobalTransform,
        &bevy::ui::ComputedUiRenderTargetInfo,
    )>,
    roots: Query<Entity, With<AlsHudRoot>>,
    menu: Res<OverlayMenuState>,
    instances: Query<&AnimatedSceneInstance>,
    contexts: Query<&AlsRootAnimationContext>,
    backgrounds: Query<&BackgroundColor>,
    controls: Query<(
        &ComputedNode,
        &bevy::ui::UiGlobalTransform,
        Option<&Children>,
        Option<&Text>,
        Option<&TextColor>,
    )>,
) {
    if state.scenario != IntegrationScenario::Interface
        || state.phase != IntegrationPhase::Interface
        || !state.interface_debug_armed
        || state.phase_elapsed() >= 5.0
    {
        return;
    }
    let Ok(window) = windows.single() else {
        return;
    };
    let (stage, size, dpi) = viewport(state.phase_elapsed());
    let umg_scale = viewport_ui_scale(size);
    if window.physical_size() != size
        || (window.scale_factor() - dpi).abs() > 1e-5
        || nodes.iter().any(|(_, _, _, target)| {
            target.physical_size() != size || (target.scale_factor() - umg_scale).abs() > 1e-5
        })
    {
        report.direction_trace.push(serde_json::json!({"hud_layout_pending":stage,"hud_viewport":window.physical_size().to_array(),
            "hud_dpi":window.scale_factor(),"hud_requested_viewport":size.to_array(),"hud_requested_dpi":dpi}));
        return;
    }
    let scale = (size.x as f32 / 1280.0).min(size.y as f32 / 720.0);
    let Some(camera_curve_count) = state
        .camera
        .and_then(|camera| instances.get(camera).ok())
        .and_then(|instance| contexts.get(instance.player_entity()).ok())
        .map(|context| context.curves.len())
    else {
        return;
    };
    let camera_height = (31.0 + 12.0 * camera_curve_count as f32).max(158.0);
    let mut failures = Vec::new();
    for prefix in [
        "ENTITY ",
        "Location (cm): ",
        "Owner: ",
        "INPUT CONTEXT ",
        "STATE ",
    ] {
        if !controls
            .iter()
            .any(|(_, _, _, text, _)| text.is_some_and(|text| text.0.starts_with(prefix)))
        {
            failures.push(serde_json::json!({"field":"HostDiagnostic","missing":prefix}));
        }
    }
    for (name, position) in [
        (
            "DebugHeader:Als.CameraCurves (Shift + 6)",
            Vec2::new(5.0, 5.0),
        ),
        (
            "DebugHeader:Als.CameraShapes (Shift + 7)",
            Vec2::new(205.0, 5.0),
        ),
        ("Camera Offset X", Vec2::new(150.0, 24.0)),
        (
            "DebugHeader:Debug mode is enabled! Press (Shift + 0) to disable.",
            Vec2::new(5.0, camera_height + 5.0),
        ),
        (
            "DebugHeader:Als.State (Shift + 2)",
            Vec2::new(205.0, camera_height + 32.0),
        ),
        ("Allow Transitions", Vec2::new(150.0, camera_height + 51.0)),
        ("View Block", Vec2::new(150.0, camera_height + 555.0)),
    ] {
        let actual = nodes
            .iter()
            .find(|(key, _, _, _)| key.as_str() == name)
            .filter(|(_, node, _, _)| node.size().min_element() > 0.0)
            .map(|(_, node, transform, _)| transform.affine().translation - node.size() * 0.5);
        let expected = Vec2::new(
            position.x * scale,
            50.0 * dpi + 29.9 * (size.x as f32 / 1920.0).max(dpi) + (position.y - 5.0) * scale,
        );
        if actual.is_none_or(|actual| !actual.abs_diff_eq(expected, 0.001)) {
            failures.push(serde_json::json!({"field":name,"expected":expected.to_array(),"actual":actual.map(|point| point.to_array())}));
        }
    }
    for (name, position) in [
        ("DebugHostTarget", Vec2::new(4.0 * dpi, 50.0 * dpi)),
        (
            "DebugHostHelp",
            Vec2::new(
                4.0 * dpi,
                50.0 * dpi + 14.95 * (size.x as f32 / 1920.0).max(dpi),
            ),
        ),
    ] {
        let actual = nodes
            .iter()
            .find(|(key, _, _, _)| key.as_str() == name)
            .map(|(_, node, transform, _)| transform.affine().translation - node.size() * 0.5);
        if actual.is_none_or(|actual| !actual.abs_diff_eq(position, 0.001)) {
            failures.push(serde_json::json!({"field":name,"expected":position.to_array(),"actual":actual.map(|point| point.to_array())}));
        }
    }
    if let Ok(root) = roots.single() {
        let (root_node, root_transform, sections, _, _) = controls.get(root).unwrap();
        let origin = root_transform.affine().translation - root_node.size() * 0.5;
        let expected = Vec2::new(size.x as f32 - 365.0 * umg_scale, 25.0 * umg_scale);
        if !origin.abs_diff_eq(expected, 1.0)
            || (root_node.size().x - 340.0 * umg_scale).abs() > 1.0
        {
            failures.push(serde_json::json!({"field":"ControlPanel","expected":expected.to_array(),"actual":origin.to_array(),"width":root_node.size().x}));
        }
        for section in sections.into_iter().flatten() {
            let (section_node, _, rows, _, _) = controls.get(*section).unwrap();
            if section_node.size().y == 0.0 {
                continue;
            }
            for (row_index, row) in rows.into_iter().flatten().enumerate() {
                let (node, transform, cells, _, _) = controls.get(*row).unwrap();
                let position = transform.affine().translation - node.size() * 0.5;
                if (position.x - origin.x).abs() > 1.0
                    || (node.size().x - root_node.size().x).abs() > 1.0
                {
                    failures.push(serde_json::json!({"field":"ControlRow","row":row_index,"position":position.to_array(),"width":node.size().x}));
                }
                if row_index == 0 {
                    continue;
                }
                let mut end = origin.x;
                for (index, cell) in cells.into_iter().flatten().enumerate() {
                    let (cell_node, transform, _, _, _) = controls.get(*cell).unwrap();
                    let position = transform.affine().translation - cell_node.size() * 0.5;
                    let gap = if index == 0 { 0.0 } else { 4.0 * umg_scale };
                    if (position.x - end - gap).abs() > 1.0 {
                        failures.push(serde_json::json!({"field":"ControlCell","row":row_index,"column":index,"gap":position.x-end,"expected":gap}));
                    }
                    end = position.x + cell_node.size().x;
                }
                if (end - origin.x - root_node.size().x).abs() > 1.0 {
                    failures.push(
                        serde_json::json!({"field":"ControlRowEnd","row":row_index,"end":end}),
                    );
                }
            }
        }
        if !controls.iter().any(|(_, _, _, text, color)| {
            text.is_some_and(|text| text.0 == "Shift + 0")
                && color.is_some_and(|color| color.0 == COLOR_TEXT)
        }) {
            failures.push(serde_json::json!({"field":"MasterKeyColor"}));
        }
    } else {
        failures.push(serde_json::json!({"field":"ControlPanelMissing"}));
    }
    if menu.open {
        if let Some(root) = menu.root
            && let Ok((node, transform, children, _, _)) = controls.get(root)
        {
            let origin = transform.affine().translation - node.size() * 0.5;
            let expected = Vec2::new(size.x as f32 - 573.0 * umg_scale, 25.0 * umg_scale);
            let height = (menu.options.len() as f32 * 29.0 - 4.0) * umg_scale;
            if !origin.abs_diff_eq(expected, 1.0)
                || !node
                    .size()
                    .abs_diff_eq(Vec2::new(200.0 * umg_scale, height), 1.0)
                || backgrounds.get(root).map_or(true, |color| {
                    color.0 != crate::overlay_mode_menu::COLOR_MENU_BACKGROUND
                })
            {
                failures.push(serde_json::json!({"field":"MenuPanel","position":origin.to_array(),"size":node.size().to_array()}));
            }
            for (index, child) in children.into_iter().flatten().enumerate() {
                let (node, transform, labels, _, _) = controls.get(*child).unwrap();
                let position = transform.affine().translation - node.size() * 0.5;
                let expected = origin + Vec2::Y * index as f32 * 29.0 * umg_scale;
                let color = if index == menu.selected_index() {
                    crate::overlay_mode_menu::COLOR_OPTION_SELECTED
                } else {
                    crate::overlay_mode_menu::COLOR_OPTION_NOT_SELECTED
                };
                if !position.abs_diff_eq(expected, 1.0)
                    || !node
                        .size()
                        .abs_diff_eq(Vec2::new(200.0, 25.0) * umg_scale, 1.0)
                    || backgrounds
                        .get(*child)
                        .map_or(true, |background| background.0 != color)
                {
                    failures.push(serde_json::json!({"field":"MenuOption","index":index,"position":position.to_array(),"size":node.size().to_array()}));
                }
                let expected_label = crate::overlay_mode_menu::option_label(menu.options[index]);
                let label = labels
                    .and_then(|labels| labels.first())
                    .and_then(|entity| controls.get(*entity).ok())
                    .and_then(|(_, _, _, text, _)| text);
                if label.is_none_or(|text| text.0 != expected_label) {
                    failures.push(serde_json::json!({"field":"MenuLabel","index":index,"expected":expected_label,"actual":label.map(|text| &text.0)}));
                }
            }
        } else {
            failures.push(serde_json::json!({"field":"MenuPanelMissing"}));
        }
    }
    report.direction_trace.push(
        serde_json::json!({"hud_layout_stage":stage,"hud_canvas_valid":failures.is_empty(),
        "hud_camera_curve_count":camera_curve_count,
        "hud_menu_valid":menu.open && failures.is_empty(),
        "hud_viewport":size.to_array(),"hud_dpi":dpi,"hud_layout_failures":failures}),
    );
}

pub(super) fn passed(trace: &[serde_json::Value]) -> bool {
    (1..3).all(|stage| {
        trace.iter().any(|entry| {
            entry["hud_layout_stage"].as_u64() == Some(stage) && entry["hud_menu_valid"] == true
        })
    }) && (0..3).all(|stage| {
        trace.iter().any(|entry| {
            entry["hud_layout_stage"].as_u64() == Some(stage) && entry["hud_canvas_valid"] == true
        })
    }) && trace
        .iter()
        .filter_map(|entry| {
            entry
                .get("hud_canvas_valid")
                .and_then(serde_json::Value::as_bool)
        })
        .all(|valid| valid)
}

const DEBUG_KEYS: [KeyCode; 10] = [
    KeyCode::Digit0,
    KeyCode::Digit1,
    KeyCode::Digit2,
    KeyCode::Digit3,
    KeyCode::Digit4,
    KeyCode::Digit5,
    KeyCode::Digit6,
    KeyCode::Digit7,
    KeyCode::Digit8,
    KeyCode::Digit9,
];

pub(super) fn drive_controls(keys: &mut ButtonInput<KeyCode>, elapsed: f32) {
    drive_key_windows(keys, KeyCode::Tab, elapsed, &[(0.25, 0.55), (0.95, 1.25)]);
    drive_key_windows(keys, KeyCode::KeyZ, elapsed, &[(1.65, 1.95), (2.25, 2.55)]);
    drive_key_windows(keys, KeyCode::KeyQ, elapsed, &[(2.9, 3.9)]);
}

fn drive_key_windows(
    keys: &mut ButtonInput<KeyCode>,
    key: KeyCode,
    elapsed: f32,
    windows: &[(f32, f32)],
) {
    if windows
        .iter()
        .any(|(start, end)| (*start..*end).contains(&elapsed))
    {
        keys.press(key);
    } else {
        keys.release(key);
    }
}

pub(super) fn drive_debug_keys(keys: &mut ButtonInput<KeyCode>, elapsed: f32) {
    keys.press(KeyCode::ShiftLeft);
    for (index, key) in DEBUG_KEYS.into_iter().enumerate() {
        let start = 5.0 + index as f32 * 0.6;
        drive_key_windows(
            keys,
            key,
            elapsed,
            &[(start, start + 0.1), (start + 0.3, start + 0.4)],
        );
    }
}

pub(super) fn observe_debug_keys(
    elapsed: f32,
    state: &AlsShowDebugHudState,
    roots: &Query<(&Node, &Visibility), With<AlsShowDebugHudRoot>>,
    stats: &AlsDebugDisplayStats,
    trace: &mut Vec<serde_json::Value>,
) {
    let index = ((elapsed - 5.0) / 0.6).floor() as usize;
    if index >= 10 {
        return;
    }
    let phase = elapsed - 5.0 - index as f32 * 0.6;
    let enabled = index == 9 || phase >= 0.3;
    let visible = index != 0 || enabled;
    let root = roots.iter().any(|(node, visibility)| {
        node.display != Display::None && *visibility != Visibility::Hidden
    });
    let valid = state.visible == visible
        && root == visible
        && stats.bounds_boxes_drawn == u32::from(visible)
        && DebugCategory::ALL
            .into_iter()
            .enumerate()
            .all(|(category_index, category)| {
                state.is_active(category) == (visible && (index != category_index + 1 || enabled))
            });
    trace.push(serde_json::json!({"debug_key":index,"debug_key_enabled":enabled,"debug_key_valid":valid,
        "debug_key_master":state.visible,"debug_key_root_visible":root,"debug_target_bounds":stats.bounds_boxes_drawn}));
}

pub(super) fn debug_keys_passed(trace: &[serde_json::Value]) -> bool {
    (0..10).all(|key| {
        [false, true]
            .into_iter()
            .filter(|enabled| key != 9 || *enabled)
            .all(|enabled| {
                trace
                    .iter()
                    .any(|entry| entry["debug_key"] == key && entry["debug_key_enabled"] == enabled)
            })
    }) && trace
        .iter()
        .filter_map(|entry| entry["debug_key_valid"].as_bool())
        .all(|valid| valid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interface_key_windows_produce_exact_press_counts() {
        let mut keys = ButtonInput::<KeyCode>::default();
        let mut presses = [0; 3];
        for step in 0..100 {
            keys.clear();
            drive_controls(&mut keys, step as f32 * 0.05);
            for (index, key) in [KeyCode::Tab, KeyCode::KeyZ, KeyCode::KeyQ]
                .into_iter()
                .enumerate()
            {
                presses[index] += u32::from(keys.just_pressed(key));
            }
        }
        assert_eq!(presses, [2, 2, 1]);
    }

    #[test]
    fn debug_key_gate_reads_hidden_root_display_state() {
        use bevy::ecs::system::RunSystemOnce;
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy_als::show_debug_hud::AlsShowDebugHudPlugin))
            .init_resource::<ButtonInput<KeyCode>>();
        app.update();
        let trace = app
            .world_mut()
            .run_system_once(
                |roots: Query<(&Node, &Visibility), With<AlsShowDebugHudRoot>>| {
                    assert_eq!(roots.iter().count(), 1);
                    let mut trace = Vec::new();
                    observe_debug_keys(
                        5.1,
                        &AlsShowDebugHudState::default(),
                        &roots,
                        &AlsDebugDisplayStats::default(),
                        &mut trace,
                    );
                    trace
                },
            )
            .unwrap();
        assert_eq!(trace[0]["debug_key_valid"], true);
    }

    #[test]
    fn debug_key_gate_requires_all_edges_and_rejects_late_failure() {
        let mut trace: Vec<_> = (0..10).flat_map(|key| [false, true].into_iter().filter(move |enabled| key != 9 || *enabled)
            .map(move |enabled| serde_json::json!({"debug_key":key,"debug_key_enabled":enabled,"debug_key_valid":true}))).collect();
        assert!(debug_keys_passed(&trace));
        assert!(!debug_keys_passed(&trace[..trace.len() - 1]));
        trace.push(serde_json::json!({"debug_key_valid":false}));
        assert!(!debug_keys_passed(&trace));
    }

    #[test]
    fn viewport_gate_requires_every_size_and_rejects_late_layout_failures() {
        let mut trace: Vec<_> = (0..3)
            .map(|stage| serde_json::json!({"hud_layout_stage":stage,"hud_canvas_valid":true,"hud_menu_valid":stage > 0}))
            .collect();
        assert!(passed(&trace));
        assert!(!passed(&trace[..2]));
        trace[2]["hud_menu_valid"] = false.into();
        assert!(!passed(&trace));
        trace[2]["hud_menu_valid"] = true.into();
        trace.push(serde_json::json!({"hud_layout_stage":2,"hud_canvas_valid":false}));
        assert!(!passed(&trace));
        assert_eq!(viewport(0.0).0, 0);
        assert_eq!(viewport(2.0).0, 1);
        assert_eq!(viewport(4.0).0, 2);
    }
}
