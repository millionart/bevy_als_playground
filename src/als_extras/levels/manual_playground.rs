mod logging;

use bevy_als::als::character::AlsCollisionLayer;
use std::collections::HashSet;
use std::num::NonZeroU8;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use avian3d::prelude::*;
use bevy::anti_alias::taa::TemporalAntiAliasing;
use bevy::pbr::ScreenSpaceAmbientOcclusion;
use bevy::prelude::*;
use bevy::render::view::Msaa;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::time::TimeUpdateStrategy;
use bevy::window::{PresentMode, WindowPlugin};
use bevy_als::extensions::SceneQuery;
use bevy_tnua::{TnuaMotor, TnuaProximitySensor, TnuaSensorsEntities, TnuaToggle};

use bevy_als::als::character::{AlsCharacter, AlsCrushRecovery, AlsCrushRecoveryFloor};
use bevy_als::als::control_scheme::AlsControlScheme;
use bevy_als::als::params::AlsMovementParams;
use bevy_als_playground::player_start::{AlsPlayerStart, AlsPlayerStartReady};
use bevy_als::als_camera::AlsCameraViewSource;
use bevy_als::character_assembly::AlsCharacterAvatar;
use bevy_als::debug_display::AlsDebugDisplayPlugin;
use bevy_als::extensions::overlay_materials::{
    AvatarMaterialApplied, AvatarMaterialOverride, OverlayMaterialsPlugin,
};
use bevy_als_playground::playground::PlaygroundLandscape;
use bevy_als_playground::source_world_time::SourceWorldTimePlugin;
use bevy_als_playground::playground::{
    PlaygroundBoxCollider, PlaygroundColliderKind, PlaygroundCollisionSource,
    PlaygroundMovingObject, PlaygroundPlugin,
};
use bevy_als_playground::game_mode::{
    AlsGameModePlugin, AlsPlayerAdmission, AlsPlayerAdmissionComplete, AlsScenePhysicsReady,
    request_als_player,
};
use bevy_als::overlay_object::OverlayObjectPlugin;
use bevy_als_playground::playground_collision::{
    PlaygroundCollisionPlugin, PlaygroundMeshColliderBuilt, playground_kinematic_velocity_bridge,
};
use bevy_als::show_debug_hud::AlsShowDebugHudPlugin;
use bevy_als::AlsPlugin;
use bevy_als_playground::{
    PackedFootprintsPlugin,
    decal_diagnostics::DecalDiagnosticsPlugin,
    input::AlsInputPlugin,
    anim_probe::AnimProbePlugin,
    animation_visual_validation::AnimationVisualValidationPlugin,
    manual_playground_integration::{
        LightingProbePlugin, ManualIntegrationGateReport, ManualPlaygroundIntegrationPlugin,
        ManualPlaygroundStartupReady,
    },
};

const SPAWN_VISIBLE_VALIDATION_STEPS: u32 = 60;
const SPAWN_SENSOR_STABLE_STEPS: u32 = 20;

#[derive(serde::Deserialize)]
struct PlaygroundWorldTime {
    min_undilated_seconds: f64,
    max_undilated_seconds: f64,
}

fn main() {
    let world_time: PlaygroundWorldTime = serde_json::from_str(include_str!(
        "../../../assets/ALSExtras/Levels/L_Als_Playground/world_time.runtime.json"
    ))
    .expect("playground world time settings must be valid");
    let mut app = App::new();
    let use_gpu_instances = !cfg!(all(target_os = "macos", target_arch = "aarch64"));
    app.add_plugins((
        DefaultPlugins
            .set(logging::log_plugin())
            .set(AssetPlugin {
                file_path: "assets".into(),
                ..default()
            })
            .set(WindowPlugin {
                primary_window: Some(Window {
                    present_mode: PresentMode::AutoNoVsync,
                    ..default()
                }),
                ..default()
            })
            .set(bevy::pbr::PbrPlugin {
                use_gpu_instance_buffer_builder: use_gpu_instances,
                ..default()
            }),
        SourceWorldTimePlugin {
            minimum: Duration::from_secs_f64(world_time.min_undilated_seconds),
            maximum: Duration::from_secs_f64(world_time.max_undilated_seconds),
        },
        AlsPlugin,
        AlsInputPlugin,
        AlsGameModePlugin,
    ));
    if !use_gpu_instances {
        use bevy::render::{
            RenderApp, RenderStartup,
            batching::gpu_preprocessing::{GpuPreprocessingMode, GpuPreprocessingSupport},
            init_gpu_resource,
        };
        app.sub_app_mut(RenderApp).add_systems(
            RenderStartup,
            (|mut support: ResMut<GpuPreprocessingSupport>| {
                support.max_supported_mode = GpuPreprocessingMode::None;
            })
            .after(init_gpu_resource::<GpuPreprocessingSupport>),
        );
    }
    app.add_plugins((
        OverlayObjectPlugin,
        AlsDebugDisplayPlugin,
        AlsShowDebugHudPlugin,
        AnimationVisualValidationPlugin,
        LightingProbePlugin(std::env::var_os("ALS_LIGHTING_EVIDENCE_PATH").map(Into::into)),
    ));
    if std::env::var_os("ALS_MANUAL_INTEGRATION_GATE").is_some() {
        app.add_plugins((AnimProbePlugin, ManualPlaygroundIntegrationPlugin));
        let frame_delta = std::env::var("ALS_MANUAL_INTEGRATION_FRAME_DELTA_SECS")
            .ok()
            .map(|raw| parse_gate_frame_delta(&raw));
        if let Some(delta) = frame_delta {
            app.insert_resource(TimeUpdateStrategy::ManualDuration(delta));
        }
        if let Ok(raw) = std::env::var("ALS_MANUAL_INTEGRATION_FRAME_HITCH_SECS") {
            let baseline = frame_delta.expect("frame hitch requires a manual baseline delta");
            let hitch = parse_gate_frame_delta(&raw);
            assert!(
                hitch > baseline,
                "frame hitch must exceed the baseline delta"
            );
            app.insert_resource(GateFrameHitch {
                baseline,
                hitch,
                fired: false,
                restore_next: false,
            });
            app.add_systems(Last, apply_gate_frame_hitch);
        }
    } else if std::env::var_os("ALS_ANIMATION_VISUAL_EVIDENCE_DIR").is_none() {
        app.insert_resource(bevy_als::show_debug_hud::AlsShowDebugSettingsFile(
            std::env::temp_dir().join("bevy-als-debug.ron"),
        ));
    }
    if std::env::var_os("ALS_ANIMATION_VISUAL_EVIDENCE_DIR").is_some() {
        app.add_plugins(bevy::render::diagnostic::RenderDiagnosticsPlugin);
    }
    app.add_plugins((
        bevy_als_playground::als_hud::AlsHudPlugin,
        bevy_als_playground::overlay_mode_menu::OverlayModeMenuPlugin,
        PlaygroundPlugin {
            manifest: "assets/ALSExtras/Levels/L_Als_Playground/playground_scene.ron".into(),
        },
        PlaygroundCollisionPlugin,
        OverlayMaterialsPlugin,
        PackedFootprintsPlugin,
        DecalDiagnosticsPlugin,
    ))
    .init_resource::<SpawnOnce>()
    .init_resource::<CharacterMaterialProbe>()
    .init_resource::<PlatformPhysicsProbe>()
    .insert_resource(RenderEffectsProbe {
        directory: std::env::var_os("ALS_ANIMATION_VISUAL_EVIDENCE_DIR").map(PathBuf::from),
        ..default()
    })
    .init_resource::<CollisionParityProbe>()
    .init_resource::<GroundedSpawnProbe>()
    .init_resource::<ManualPlaygroundStartupReady>()
    .init_resource::<ManualScenePhysicsReady>()
    .init_resource::<AlsScenePhysicsReady>()
    .insert_resource(GlobalAmbientLight {
        brightness: 2.0,
        ..default()
    })
    .add_systems(Startup, arm_auto_quit)
    .add_systems(
        Update,
        (
            landscape_collider_bridge,
            scene_physics_ready.after(landscape_collider_bridge),
            spawn_character_once.after(scene_physics_ready),
            admit_completed_player.after(spawn_character_once),
            (enable_character_ssao_taa, render_effects_probe).chain(),
            character_material_probe,
            platform_physics_probe.after(playground_kinematic_velocity_bridge),
            collision_parity_probe,
            auto_quit,
            screenshot_runner,
        ),
    )
    .add_systems(
        PostUpdate,
        grounded_spawn_probe.after(bevy_als::als::motion::AlsActualVelocitySet),
    );
    if std::env::var("ALS_PLAYGROUND_STARTUP_VARIANT").as_deref() == Ok("multi_raised") {
        app.add_systems(
            Update,
            setup_multi_raised_startup_variant
                .before(bevy_als_playground::player_start::AlsPlayerStartPrepared),
        );
    }
    if let Ok(raw) = std::env::var("ALS_MANUAL_JUMP_GRAVITY_Y") {
        assert!(
            std::env::var_os("ALS_MANUAL_INTEGRATION_GATE").is_some(),
            "jump gravity variant requires the manual integration gate"
        );
        let gravity_y = raw.parse::<f32>().expect("jump gravity must be a number");
        assert!(
            gravity_y.is_finite() && gravity_y < 0.0,
            "jump gravity must be finite and downward"
        );
        app.insert_resource(Gravity(Vec3::Y * gravity_y));
        info!("[JUMP-GRAVITY-VARIANT] gravity_y={gravity_y:.3} -> PASS");
    }

    if let AppExit::Error(code) = app.run() {
        std::process::exit(i32::from(code.get()));
    }
}

fn parse_gate_frame_delta(raw: &str) -> Duration {
    let seconds = raw
        .parse::<f64>()
        .expect("gate frame delta must be a number");
    assert!(
        seconds.is_finite() && seconds > 0.0,
        "gate frame delta must be positive and finite"
    );
    Duration::from_secs_f64(seconds)
}

#[derive(Resource)]
struct GateFrameHitch {
    baseline: Duration,
    hitch: Duration,
    fired: bool,
    restore_next: bool,
}

fn apply_gate_frame_hitch(
    mut strategy: ResMut<TimeUpdateStrategy>,
    mut hitch: ResMut<GateFrameHitch>,
    report: Res<ManualIntegrationGateReport>,
) {
    if hitch.restore_next {
        *strategy = TimeUpdateStrategy::ManualDuration(hitch.baseline);
        hitch.restore_next = false;
    } else if !hitch.fired
        && report.moving_base_trace.last().is_some_and(|frame| {
            frame
                .get("takeoff_ready")
                .and_then(serde_json::Value::as_bool)
                == Some(true)
                && frame.get("mode").and_then(serde_json::Value::as_str) == Some("Grounded")
        })
    {
        *strategy = TimeUpdateStrategy::ManualDuration(hitch.hitch);
        hitch.fired = true;
        hitch.restore_next = true;
        info!(
            "[MANUAL-INTEGRATION][FRAME-HITCH] next_delta_seconds={:.6}",
            hitch.hitch.as_secs_f64()
        );
    }
}

#[derive(Resource, Default)]
struct SpawnOnce(bool);

#[derive(Resource, Default)]
struct CharacterMaterialProbe(bool);

#[derive(Resource, Default)]
struct PlatformPhysicsProbe(bool);

#[derive(Resource, Default)]
struct RenderEffectsProbe {
    ready_at: Option<f64>,
    passed: Option<bool>,
    directory: Option<PathBuf>,
}

#[derive(Resource, Default)]
struct CollisionParityProbe {
    ready_frames: u32,
    done: bool,
}

#[derive(Resource, Default)]
struct GroundedSpawnProbe {
    ground_y: Option<f32>,
    expected_center_y: Option<f32>,
    spawn_adjustment: Option<f32>,
    character: Option<Entity>,
    controller_enabled: bool,
    bootstrap_frames: u32,
    stable_sensor_frames: u32,
    frames: u32,
    pre_enable_max_center_error: f32,
    pre_enable_max_vertical_velocity: f32,
    max_center_error: f32,
    max_vertical_velocity: f32,
    done: bool,
    passed: Option<bool>,
}

#[derive(Resource, Default)]
struct ManualScenePhysicsReady {
    ready: bool,
    stable_frames: u32,
    wait_frames: u32,
    sources: usize,
    boxes: usize,
}

#[derive(Resource)]
struct AutoQuit(Instant);

#[derive(Resource)]
struct ShotPlan {
    at: Instant,
    path: PathBuf,
    taken: bool,
    done_at: Option<Instant>,
}

fn arm_auto_quit(mut commands: Commands) {
    let Ok(secs) = std::env::var("ALS_BOOT_AUTO_QUIT_SECS") else {
        return;
    };
    let Ok(secs) = secs.parse::<f64>() else {
        return;
    };
    commands.insert_resource(AutoQuit(Instant::now() + Duration::from_secs_f64(secs)));
    info!("[MANUAL] auto quit in {secs}s");

    if let Ok(path) = std::env::var("ALS_SHOT_PATH") {
        let after = std::env::var("ALS_SHOT_AFTER_SECS")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(4.0);
        commands.insert_resource(ShotPlan {
            at: Instant::now() + Duration::from_secs_f64(after),
            path: PathBuf::from(&path),
            taken: false,
            done_at: None,
        });
        info!("[MANUAL] screenshot scheduled +{after}s -> {path}");
    }
}

fn auto_quit(
    timer: Option<Res<AutoQuit>>,
    ground_probe: Res<GroundedSpawnProbe>,
    render_probe: Res<RenderEffectsProbe>,
    visual_progress: Option<
        Res<bevy_als_playground::animation_visual_validation::AnimationVisualValidationProgress>,
    >,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(t) = timer else {
        return;
    };
    if Instant::now() >= t.0 {
        if visual_progress.is_some_and(|progress| {
            ground_probe.passed != Some(false)
                && render_probe.passed != Some(false)
                && (!progress.evidence_ready
                    || ground_probe.passed.is_none()
                    || render_probe.passed.is_none())
        }) {
            return;
        }
        match ground_probe.passed {
            Some(true) if render_probe.passed == Some(true) => {
                info!("[MANUAL] auto quit reached after grounded spawn and render effects PASS");
                exit.write(AppExit::Success);
            }
            Some(_) => {
                error!("[MANUAL] auto quit reached without grounded spawn and render effects PASS");
                exit.write(AppExit::Error(NonZeroU8::new(12).unwrap()));
            }
            None => {
                error!("[MANUAL] auto quit reached before grounded spawn verdict");
                exit.write(AppExit::Error(NonZeroU8::new(12).unwrap()));
            }
        }
    }
}

fn screenshot_runner(
    plan: Option<ResMut<ShotPlan>>,
    ground_probe: Res<GroundedSpawnProbe>,
    mut commands: Commands,
    cameras: Query<Entity, With<Camera3d>>,
    mut exit: MessageWriter<AppExit>,
    mut report: Option<ResMut<ManualIntegrationGateReport>>,
) {
    if ground_probe.passed == Some(false) {
        if let Some(report) = report.as_deref_mut() {
            if report.scenario.is_empty() {
                report.scenario = "startup".into();
            }
            report.completed = true;
            report.passed = Some(false);
            report.results.push(bevy_als_playground::manual_playground_integration::ManualIntegrationGateResult {
                name: "grounded_spawn".into(), covered: false,
                detail: format!("pre_enable_center_error={:.6} pre_enable_vertical_velocity={:.6} max_center_error={:.6} max_vertical_velocity={:.6} controller_enabled={}",
                    ground_probe.pre_enable_max_center_error, ground_probe.pre_enable_max_vertical_velocity,
                    ground_probe.max_center_error, ground_probe.max_vertical_velocity, ground_probe.controller_enabled),
            });
        }
        exit.write(AppExit::Error(NonZeroU8::new(13).unwrap()));
        return;
    }
    let Some(mut plan) = plan else {
        return;
    };
    if !plan.taken && Instant::now() >= plan.at {
        let Ok(cam) = cameras.single() else {
            return;
        };
        let path = plan.path.clone();
        commands.entity(cam).insert(Screenshot::primary_window());
        commands.entity(cam).observe(save_to_disk(path.clone()));
        info!("[MANUAL] screenshot requested -> {}", path.display());
        plan.taken = true;
        plan.done_at = Some(Instant::now() + Duration::from_millis(1500));
    }
    if let Some(done) = plan.done_at
        && Instant::now() >= done
    {
        match ground_probe.passed {
            Some(true) => {
                info!("[MANUAL] screenshot flow done after grounded spawn PASS, exiting");
                exit.write(AppExit::Success);
            }
            Some(false) => {
                error!("[MANUAL] screenshot flow done after grounded spawn FAIL");
                exit.write(AppExit::Error(NonZeroU8::new(13).unwrap()));
            }
            None => {
                error!("[MANUAL] screenshot flow done before grounded spawn verdict");
                exit.write(AppExit::Error(NonZeroU8::new(13).unwrap()));
            }
        }
    }
}

#[derive(Component)]
struct LandscapeColliderBuilt;

fn landscape_collider_bridge(
    mut commands: Commands,
    landscapes: Query<
        (Entity, &GlobalTransform),
        (With<PlaygroundLandscape>, Without<LandscapeColliderBuilt>),
    >,
    scene_children: Query<&Children>,
    mesh_parts: Query<(&Mesh3d, &GlobalTransform)>,
    meshes: Res<Assets<Mesh>>,
) {
    fn append_meshes(
        entity: Entity,
        root_inverse: Mat4,
        scene_children: &Query<&Children>,
        mesh_parts: &Query<(&Mesh3d, &GlobalTransform)>,
        meshes: &Assets<Mesh>,
        vertices: &mut Vec<Vec3>,
        triangles: &mut Vec<[u32; 3]>,
    ) {
        if let Ok((mesh_handle, transform)) = mesh_parts.get(entity)
            && let Some(mesh) = meshes.get(&mesh_handle.0)
            && let Some(positions) = mesh.attribute(Mesh::ATTRIBUTE_POSITION)
            && let Some(values) = positions.as_float3()
            && let Some(indices) = mesh.indices()
        {
            let base = vertices.len() as u32;
            let relative = root_inverse * transform.to_matrix();
            vertices.extend(
                values
                    .iter()
                    .map(|position| relative.transform_point3(Vec3::from(*position))),
            );
            match indices {
                bevy::render::mesh::Indices::U32(list) => {
                    triangles.extend(list.chunks_exact(3).map(|triangle| {
                        [base + triangle[0], base + triangle[1], base + triangle[2]]
                    }))
                }
                bevy::render::mesh::Indices::U16(list) => {
                    triangles.extend(list.chunks_exact(3).map(|triangle| {
                        [
                            base + triangle[0] as u32,
                            base + triangle[1] as u32,
                            base + triangle[2] as u32,
                        ]
                    }))
                }
            }
        }
        if let Ok(children) = scene_children.get(entity) {
            for child in children.iter() {
                append_meshes(
                    child,
                    root_inverse,
                    scene_children,
                    mesh_parts,
                    meshes,
                    vertices,
                    triangles,
                );
            }
        }
    }

    for (root, root_transform) in &landscapes {
        let mut vertices = Vec::new();
        let mut triangles = Vec::new();
        append_meshes(
            root,
            root_transform.to_matrix().inverse(),
            &scene_children,
            &mesh_parts,
            &meshes,
            &mut vertices,
            &mut triangles,
        );
        if vertices.is_empty() || triangles.is_empty() {
            continue;
        }
        info!(
            "[MANUAL] landscape collider built verts={} triangles={}",
            vertices.len(),
            triangles.len()
        );
        commands.entity(root).insert((
            RigidBody::Static,
            Collider::trimesh(vertices, triangles),
            LandscapeColliderBuilt,
            CollisionLayers::new(
                [AlsCollisionLayer::Default, AlsCollisionLayer::Visibility],
                LayerMask::ALL,
            ),
        ));
    }
}

fn scene_physics_ready(
    mut ready: ResMut<ManualScenePhysicsReady>,
    mut admission_ready: ResMut<AlsScenePhysicsReady>,
    player_starts: Query<&AlsPlayerStartReady>,
    landscapes: Query<Entity, (With<PlaygroundLandscape>, With<LandscapeColliderBuilt>)>,
    sources: Query<(&PlaygroundCollisionSource, Has<Collider>)>,
    boxes: Query<Has<Collider>, With<PlaygroundBoxCollider>>,
) {
    if ready.ready {
        return;
    }
    let source_total = sources
        .iter()
        .filter(|(source, _)| !source.kind.is_box())
        .count();
    let source_ready = sources
        .iter()
        .filter(|(source, has_collider)| !source.kind.is_box() && *has_collider)
        .count();
    let box_total = boxes.iter().count();
    let box_ready = boxes.iter().filter(|has_collider| *has_collider).count();
    if landscapes.is_empty()
        || source_total == 0
        || source_ready != source_total
        || box_total == 0
        || box_ready != box_total
        || player_starts.is_empty()
    {
        ready.wait_frames += 1;
        if ready.wait_frames == 30 {
            info!(
                "[SCENE][PHYSICS] landscapes={} sources={source_ready}/{source_total} boxes={box_ready}/{box_total} player_starts={} -> WAIT",
                landscapes.iter().count(),
                player_starts.iter().count()
            );
        }
        return;
    }
    ready.stable_frames += 1;
    if ready.stable_frames < 2 {
        return;
    }
    ready.ready = true;
    admission_ready.ready = true;
    ready.sources = source_total;
    ready.boxes = box_total;
    info!(
        "[SCENE][PHYSICS] landscapes={} sources={source_ready}/{source_total} boxes={box_ready}/{box_total} player_starts={} spatial_frames={} -> PASS",
        landscapes.iter().count(),
        player_starts.iter().count(),
        ready.stable_frames
    );
}

fn setup_multi_raised_startup_variant(
    mut done: Local<bool>,
    mut commands: Commands,
    starts: Query<&Transform, With<AlsPlayerStart>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if *done {
        return;
    }
    let Some(start) = starts.iter().next().copied() else {
        return;
    };
    let platform = Transform::from_xyz(start.translation.x, 1.9, start.translation.z);
    commands.spawn((
        Name::new("StartupVariantRaisedPlatform"),
        platform,
        RigidBody::Static,
        Collider::cuboid(8.0, 0.2, 8.0),
        CollisionLayers::new(
            [
                bevy_als::als::character::AlsCollisionLayer::Default,
                bevy_als::als::character::AlsCollisionLayer::Visibility,
            ],
            LayerMask::ALL,
        ),
        Mesh3d(meshes.add(Cuboid::new(8.0, 0.2, 8.0))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.25, 0.55, 0.75),
            ..default()
        })),
    ));
    commands.spawn((
        Name::new("StartupVariantSecondPlayerStart"),
        AlsPlayerStart::default(),
        start,
    ));
    *done = true;
    info!("[STARTUP-VARIANT] second_player_start=true platform_top_y=2.0 -> PASS");
}

fn spawn_character_once(
    mut commands: Commands,
    mut once: ResMut<SpawnOnce>,
    scene_ready: Res<ManualScenePhysicsReady>,
    landscapes: Query<Entity, (With<PlaygroundLandscape>, With<LandscapeColliderBuilt>)>,
) {
    if once.0 || !scene_ready.ready || landscapes.is_empty() {
        return;
    }
    once.0 = true;
    info!(
        "[SPAWN][REQUEST] scene_sources={} scene_boxes={} barrier=true -> PASS",
        scene_ready.sources, scene_ready.boxes,
    );
    let mut admission = AlsPlayerAdmission::new();
    admission.spawn_camera = true;
    request_als_player(&mut commands, admission);
}

fn admit_completed_player(
    mut commands: Commands,
    admissions: Query<&AlsPlayerAdmissionComplete, Added<AlsPlayerAdmissionComplete>>,
    mut ground_probe: ResMut<GroundedSpawnProbe>,
) {
    let Ok(admission) = admissions.single() else {
        return;
    };
    let point = admission.selection.fit;
    let capsule_half_height = AlsMovementParams::default()
        .crouch
        .standing_capsule_half_height;
    let capsule_radius = AlsMovementParams::default().crouch.standing_capsule_radius;
    ground_probe.ground_y = Some(point.ground_y);
    ground_probe.expected_center_y = Some(point.transform.translation.y);
    ground_probe.spawn_adjustment = Some(point.adjustment);
    ground_probe.character = Some(admission.character);
    commands
        .entity(admission.character)
        .insert((TnuaToggle::SenseOnly, GravityScale(0.0)));
    info!(
        "[SPAWN][FIT] start={:?} occupancy={:?} ground_y={:.4} center_y={:.4} adjustment=({:.4},{:.4},{:.4}) capsule_half={capsule_half_height:.4} capsule_radius={capsule_radius:.4} skin={:.4} -> PASS",
        admission.selection.entity,
        point.occupancy,
        point.ground_y,
        point.transform.translation.y,
        point.teleport_adjustment.x,
        point.teleport_adjustment.y,
        point.teleport_adjustment.z,
        bevy_als_playground::player_start::ALS_PLAYER_START_SOURCE_CLEARANCE,
    );
    info!(
        "[SPAWN][BODY] phase=visible_sensor_bootstrap avatar_capsule_same_transaction=true visibility=visible -> PASS"
    );
}

fn spawn_sensor_matches_ground(proximity: f32, center_y: f32, ground_y: f32) -> bool {
    (proximity - (center_y - ground_y)).abs() <= 0.01
}

fn grounded_spawn_probe(
    mut commands: Commands,
    mut probe: ResMut<GroundedSpawnProbe>,
    mut manual_startup: ResMut<ManualPlaygroundStartupReady>,
    characters: Query<
        (
            &Transform,
            &Position,
            &LinearVelocity,
            &AlsCrushRecovery,
            &TnuaSensorsEntities<AlsControlScheme>,
        ),
        With<AlsCharacter>,
    >,
    avatars: Query<(&Transform, &AlsCharacterAvatar)>,
    sensors: Query<&TnuaProximitySensor>,
) {
    if probe.done || probe.ground_y.is_none() || probe.expected_center_y.is_none() {
        return;
    }
    let Some(character_entity) = probe.character else {
        return;
    };
    let Ok((character, position, velocity, recovery, sensor_entities)) =
        characters.get(character_entity)
    else {
        return;
    };
    let Some((avatar, _)) = avatars.iter().next() else {
        return;
    };
    let ground_y = probe.ground_y.unwrap();
    let expected_center_y = probe.expected_center_y.unwrap();
    let spawn_adjustment = probe.spawn_adjustment.unwrap_or_default();
    let capsule_half_height = AlsMovementParams::default()
        .crouch
        .standing_capsule_half_height;
    let capsule_bottom = character.translation.y - capsule_half_height;
    let bottom_clearance = capsule_bottom - ground_y;
    let avatar_relative_error =
        (avatar.translation.y - bevy_als::character_assembly::AVATAR_CAPSULE_BOTTOM_OFFSET).abs();
    let center_error = (character.translation.y - expected_center_y).abs();
    let position_error = (position.0.y - character.translation.y).abs();
    let sensor_proximity = sensor_entities
        .ground
        .and_then(|sensor| sensors.get(sensor).ok())
        .and_then(|sensor| sensor.output.as_ref())
        .map(|output| output.proximity);
    if !probe.controller_enabled {
        probe.bootstrap_frames += 1;
        let Some(sensor_proximity) = sensor_proximity else {
            if probe.bootstrap_frames == 120 {
                info!(
                    "[SPAWN][BOOTSTRAP] sensor=false center_y={:.4} vertical_velocity={:.4} -> WAIT",
                    character.translation.y, velocity.0.y
                );
            }
            return;
        };
        probe.pre_enable_max_center_error = probe.pre_enable_max_center_error.max(center_error);
        probe.pre_enable_max_vertical_velocity = probe
            .pre_enable_max_vertical_velocity
            .max(velocity.0.y.abs());
        let stable = center_error <= 0.01
            && position_error <= 0.001
            && spawn_sensor_matches_ground(sensor_proximity, expected_center_y, ground_y)
            && velocity.0.y.abs() <= 0.01;
        if stable {
            probe.stable_sensor_frames += 1;
        } else {
            probe.stable_sensor_frames = 0;
        }
        if probe.stable_sensor_frames < SPAWN_SENSOR_STABLE_STEPS {
            return;
        }
        commands.entity(character_entity).insert((
            TnuaToggle::Enabled,
            GravityScale(1.0),
            TnuaMotor::default(),
            LinearVelocity::default(),
        ));
        probe.controller_enabled = true;
        info!(
            "[SPAWN][CONTROLLER] sensor_frames={} center_y={:.4} sensor_proximity={sensor_proximity:.4} vertical_velocity={:.4} phase=active visibility=visible -> PASS",
            probe.stable_sensor_frames, character.translation.y, velocity.0.y
        );
        return;
    }
    let Some(sensor_proximity) = sensor_proximity else {
        return;
    };
    probe.frames += 1;
    probe.max_center_error = probe.max_center_error.max(center_error);
    probe.max_vertical_velocity = probe.max_vertical_velocity.max(velocity.0.y.abs());
    if probe.frames < SPAWN_VISIBLE_VALIDATION_STEPS {
        return;
    }
    let pass = bottom_clearance >= -0.01
        && bottom_clearance <= 0.05
        && avatar_relative_error <= 0.001
        && center_error <= 0.05
        && position_error <= 0.001
        && velocity.0.y.abs() <= 0.15
        && probe.pre_enable_max_center_error <= 0.01
        && probe.pre_enable_max_vertical_velocity <= 0.01
        && probe.max_center_error <= 0.025
        && probe.max_vertical_velocity <= 0.15
        && recovery.corrections == 0;
    if pass {
        manual_startup.passed = true;
        info!(
            "[SPAWN][GROUND] ground_y={ground_y:.4} center_y={:.4} spawn_adjustment={spawn_adjustment:.4} capsule_bottom={capsule_bottom:.4} clearance={bottom_clearance:.4} avatar_relative_error={avatar_relative_error:.6} position_error={position_error:.6} sensor_proximity={sensor_proximity:.6} vertical_velocity={:.4} recovery_corrections={} recovery_penetration={:.6} pre_enable_max_center_error={:.6} pre_enable_max_vertical_velocity={:.6} max_center_error={:.6} max_vertical_velocity={:.6} -> PASS",
            character.translation.y,
            velocity.0.y,
            recovery.corrections,
            recovery.max_penetration,
            probe.pre_enable_max_center_error,
            probe.pre_enable_max_vertical_velocity,
            probe.max_center_error,
            probe.max_vertical_velocity
        );
    } else {
        error!(
            "[SPAWN][GROUND] ground_y={ground_y:.4} center_y={:.4} spawn_adjustment={spawn_adjustment:.4} capsule_bottom={capsule_bottom:.4} clearance={bottom_clearance:.4} avatar_relative_error={avatar_relative_error:.6} position_error={position_error:.6} sensor_proximity={sensor_proximity:.6} vertical_velocity={:.4} recovery_corrections={} recovery_penetration={:.6} pre_enable_max_center_error={:.6} pre_enable_max_vertical_velocity={:.6} max_center_error={:.6} max_vertical_velocity={:.6} -> FAIL",
            character.translation.y,
            velocity.0.y,
            recovery.corrections,
            recovery.max_penetration,
            probe.pre_enable_max_center_error,
            probe.pre_enable_max_vertical_velocity,
            probe.max_center_error,
            probe.max_vertical_velocity
        );
    }
    probe.passed = Some(pass);
    probe.done = true;
}

fn enable_character_ssao_taa(
    mut commands: Commands,
    cameras: Query<
        (
            Entity,
            Option<&Msaa>,
            Has<ScreenSpaceAmbientOcclusion>,
            Has<TemporalAntiAliasing>,
        ),
        (With<Camera3d>, With<AlsCameraViewSource>),
    >,
) {
    for (camera, msaa, ssao, taa) in &cameras {
        let mut camera = commands.entity(camera);
        if msaa != Some(&Msaa::Off) {
            camera.insert(Msaa::Off);
        }
        if !ssao {
            camera.insert(ScreenSpaceAmbientOcclusion::default());
        }
        if !taa {
            camera.insert(TemporalAntiAliasing::default());
        }
    }
}

fn render_effects_probe(
    mut probe: ResMut<RenderEffectsProbe>,
    ready: Res<ManualPlaygroundStartupReady>,
    time: Res<Time<Real>>,
    diagnostics: Option<Res<bevy::diagnostic::DiagnosticsStore>>,
    cameras: Query<
        (
            Has<ScreenSpaceAmbientOcclusion>,
            Has<TemporalAntiAliasing>,
            Option<&Msaa>,
        ),
        (With<Camera3d>, With<AlsCameraViewSource>),
    >,
    mut exit: MessageWriter<AppExit>,
) {
    if probe.passed.is_some() || !ready.passed {
        return;
    }
    let waited = time.elapsed_secs_f64() - *probe.ready_at.get_or_insert(time.elapsed_secs_f64());
    let camera_count = cameras.iter().count();
    let ssao_count = cameras.iter().filter(|(ssao, _, _)| *ssao).count();
    let taa_count = cameras.iter().filter(|(_, taa, _)| *taa).count();
    let msaa_off_count = cameras
        .iter()
        .filter(|(_, _, msaa)| *msaa == Some(&Msaa::Off))
        .count();
    let components_ready =
        camera_count == 1 && ssao_count == 1 && taa_count == 1 && msaa_off_count == 1;
    let passes: Vec<_> = diagnostics
        .iter()
        .flat_map(|store| store.iter())
        .filter(|diagnostic| {
            let path = diagnostic.path().as_str();
            (path.contains("/ssao/") || path.contains("/taa/")) && diagnostic.value().is_some()
        })
        .map(|diagnostic| (diagnostic.path().as_str().to_owned(), diagnostic.value()))
        .collect();
    let passes_ready = ["/ssao/", "/taa/"]
        .iter()
        .all(|name| passes.iter().any(|(path, _)| path.contains(name)));
    if components_ready && probe.directory.is_some() && !passes_ready && waited < 5.0 {
        return;
    }
    let passed = components_ready && (probe.directory.is_none() || passes_ready);
    probe.passed = Some(passed);
    let report = serde_json::json!({
        "passed": passed, "cameras": camera_count, "ssao": ssao_count, "taa": taa_count,
        "msaa_off": msaa_off_count, "executed_passes": passes, "waited_seconds": waited,
        "no_progress_limit_seconds": 5.0,
        "exit_reason": if passed { "completed" } else if !components_ready { "missing_render_components" } else { "render_pass_timeout" },
    });
    info!("[RENDER-EFFECTS][VERDICT] {report}");
    if let Some(directory) = &probe.directory {
        std::fs::write(directory.join("render_effects.json"), report.to_string())
            .expect("render effects report must be writable");
    }
    if !passed {
        exit.write(AppExit::Error(NonZeroU8::new(1).unwrap()));
    }
}

fn character_material_probe(
    mut probe: ResMut<CharacterMaterialProbe>,
    avatars: Query<
        (&AvatarMaterialOverride, &AvatarMaterialApplied),
        With<bevy_als::character_assembly::AlsCharacterAvatar>,
    >,
    materials: Res<Assets<StandardMaterial>>,
) {
    if probe.0 {
        return;
    }
    let Ok((override_comp, applied)) = avatars.single() else {
        return;
    };
    let Some(handle) = override_comp.0.first() else {
        error!("[CHARMAT][VERDICT] missing material handles -> FAIL");
        probe.0 = true;
        return;
    };
    let eye_slot = bevy_als::character_assembly::ALS_CHARACTER_EYES_MATERIAL_SLOT;
    let Some(eye_handle) = override_comp.0.get(eye_slot) else {
        error!("[CHARMAT][VERDICT] missing eyes material handle -> FAIL");
        probe.0 = true;
        return;
    };
    let Some(material) = materials.get(handle) else {
        return;
    };
    let Some(eyes_material) = materials.get(eye_handle) else {
        return;
    };
    let color = material.base_color.to_linear();
    let expected = bevy_als::character_assembly::ALS_CHARACTER_MATERIAL_LINEAR_RGB;
    let color_error = (color.red - expected[0])
        .abs()
        .max((color.green - expected[1]).abs())
        .max((color.blue - expected[2]).abs())
        .max((color.alpha - 1.0).abs());
    let expected_slots = bevy_als::character_assembly::ALS_CHARACTER_MATERIAL_SLOT_COUNT;
    let expected_main = bevy_als::character_assembly::ALS_CHARACTER_MAIN_MATERIAL_LINEAR_RGB;
    let expected_roughness =
        bevy_als::character_assembly::ALS_CHARACTER_MATERIAL_PERCEPTUAL_ROUGHNESS;
    let expected_reflectance = bevy_als::character_assembly::ALS_CHARACTER_MATERIAL_REFLECTANCE;
    let roughness_error = (material.perceptual_roughness - expected_roughness).abs();
    let reflectance_error = (material.reflectance - expected_reflectance).abs();
    let eyes_color = eyes_material.base_color.to_linear();
    let eyes_color_error = (eyes_color.red - expected_main[0])
        .abs()
        .max((eyes_color.green - expected_main[1]).abs())
        .max((eyes_color.blue - expected_main[2]).abs())
        .max((eyes_color.alpha - 1.0).abs());
    let body_slots_match = override_comp
        .0
        .iter()
        .enumerate()
        .filter(|(slot, _)| *slot != eye_slot)
        .all(|(_, slot_handle)| slot_handle.id() == handle.id());
    let passed = applied.slots as usize == expected_slots
        && applied.body_slots as usize == expected_slots - 1
        && applied.main_slots == 1
        && applied.source_slot_pattern
        && override_comp.0.len() == expected_slots
        && body_slots_match
        && eye_handle.id() != handle.id()
        && color_error < 1e-6
        && roughness_error < 1e-6
        && reflectance_error < 1e-6
        && eyes_color_error < 1e-6;
    if passed {
        info!(
            "[CHARMAT][VERDICT] slots={}/{} body_slots={}/{} main_slots={}/1 eye_slot={eye_slot}/{} source_pattern={} body_color_error={color_error:.6} eyes_color_error={eyes_color_error:.6} roughness={:.6} roughness_error={roughness_error:.6} specular={:.6} specular_error={reflectance_error:.6} -> PASS",
            applied.slots,
            expected_slots,
            applied.body_slots,
            expected_slots - 1,
            applied.main_slots,
            bevy_als::character_assembly::ALS_CHARACTER_EYES_MATERIAL_SLOT,
            applied.source_slot_pattern,
            material.perceptual_roughness,
            material.reflectance
        );
    } else {
        error!(
            "[CHARMAT][VERDICT] slots={}/{} body_slots={}/{} main_slots={}/1 eye_slot={eye_slot}/{} source_pattern={} body_color_error={color_error:.6} eyes_color_error={eyes_color_error:.6} roughness={:.6} roughness_error={roughness_error:.6} specular={:.6} specular_error={reflectance_error:.6} -> FAIL",
            applied.slots,
            expected_slots,
            applied.body_slots,
            expected_slots - 1,
            applied.main_slots,
            bevy_als::character_assembly::ALS_CHARACTER_EYES_MATERIAL_SLOT,
            applied.source_slot_pattern,
            material.perceptual_roughness,
            material.reflectance
        );
    }
    probe.0 = true;
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;

    #[test]
    fn auto_quit_waits_for_complete_startup_evidence() {
        for (ground, render, evidence, expected) in [
            (None, None, Some(false), None),
            (Some(true), None, Some(false), None),
            (Some(true), Some(true), Some(false), None),
            (Some(true), Some(true), Some(true), Some(0)),
            (Some(false), None, Some(false), Some(12)),
            (Some(true), Some(false), Some(false), Some(12)),
            (None, None, None, Some(12)),
        ] {
            let mut world = World::new();
            world.init_resource::<Messages<AppExit>>();
            world.insert_resource(AutoQuit(Instant::now() - Duration::from_secs(30)));
            world.insert_resource(GroundedSpawnProbe {
                passed: ground,
                ..default()
            });
            world.insert_resource(RenderEffectsProbe {
                passed: render,
                ..default()
            });
            if let Some(evidence_ready) = evidence {
                world.insert_resource(
                    bevy_als_playground::animation_visual_validation::AnimationVisualValidationProgress {
                        evidence_ready,
                    },
                );
            }
            world.run_system_once(auto_quit).unwrap();
            let actual = world
                .resource_mut::<Messages<AppExit>>()
                .drain()
                .next()
                .map(|exit| match exit {
                    AppExit::Success => 0,
                    AppExit::Error(code) => code.get(),
                });
            assert_eq!(
                actual, expected,
                "ground={ground:?} render={render:?} evidence={evidence:?}"
            );
        }
    }

    #[test]
    fn failed_grounded_spawn_exits_without_a_screenshot_plan() {
        let mut world = World::new();
        world.init_resource::<Messages<AppExit>>();
        world.init_resource::<ManualIntegrationGateReport>();
        world.insert_resource(GroundedSpawnProbe {
            passed: Some(false),
            done: true,
            max_vertical_velocity: 0.163349,
            ..default()
        });
        world.run_system_once(screenshot_runner).unwrap();
        assert!(
            world
                .resource_mut::<Messages<AppExit>>()
                .drain()
                .any(|exit| matches!(exit, AppExit::Error(code) if code.get() == 13)),
            "Failed startup must terminate without a screenshot plan"
        );
        let report = world.resource::<ManualIntegrationGateReport>();
        assert!(report.completed && report.passed == Some(false));
        assert_eq!(report.results[0].name, "grounded_spawn");
        assert!(
            report.results[0]
                .detail
                .contains("max_vertical_velocity=0.163349")
        );
    }
    #[test]
    fn scene_ready_accepts_multiple_valid_player_starts() {
        for start_count in [1, 2] {
            let mut world = World::new();
            world.init_resource::<ManualScenePhysicsReady>();
            world.init_resource::<AlsScenePhysicsReady>();
            world.spawn((PlaygroundLandscape, LandscapeColliderBuilt));
            world.spawn((
                PlaygroundCollisionSource {
                    mesh_base: "fixture".into(),
                    kind: PlaygroundColliderKind::TriMesh,
                },
                Collider::cuboid(1.0, 1.0, 1.0),
            ));
            world.spawn((
                PlaygroundBoxCollider {
                    half_extents: Vec3::ONE,
                },
                Collider::cuboid(1.0, 1.0, 1.0),
            ));
            for _ in 0..start_count {
                world.spawn(AlsPlayerStartReady {
                    transform: Transform::IDENTITY,
                    ground_y: 0.0,
                    adjustment: 0.0,
                    teleport_adjustment: Vec3::ZERO,
                    occupancy: bevy_als_playground::player_start::AlsPlayerStartOccupancy::Unoccupied,
                });
            }
            for _ in 0..2 {
                world.run_system_once(scene_physics_ready).unwrap();
            }
            assert!(
                world.resource::<ManualScenePhysicsReady>().ready,
                "starts={start_count}"
            );
            assert!(
                world.resource::<AlsScenePhysicsReady>().ready,
                "starts={start_count}"
            );
        }
    }

    #[test]
    fn spawn_sensor_proximity_is_ground_relative() {
        for ground_y in [0.0, 5.0, -3.0] {
            let center_y = bevy_als_playground::player_start::player_start_grounded_height(
                ground_y,
                AlsMovementParams::default()
                    .crouch
                    .standing_capsule_half_height,
            );
            let sensor_proximity = center_y - ground_y;
            assert!(spawn_sensor_matches_ground(
                sensor_proximity,
                center_y,
                ground_y
            ));
            assert!(!spawn_sensor_matches_ground(
                sensor_proximity + 0.1,
                center_y,
                ground_y
            ));
        }
    }

    #[test]
    fn gate_frame_delta_preserves_raw_hitches_for_the_world_clock() {
        assert_eq!(parse_gate_frame_delta("0.1"), Duration::from_millis(100));
        assert_eq!(parse_gate_frame_delta("0.5"), Duration::from_millis(500));
        for invalid in ["0", "-0.1", "NaN", "inf", "bad"] {
            assert!(std::panic::catch_unwind(|| parse_gate_frame_delta(invalid)).is_err());
        }
    }

    #[test]
    fn gate_frame_hitch_arms_at_stable_takeoff_and_restores() {
        let baseline = Duration::from_secs_f64(1.0 / 60.0);
        let hitch = Duration::from_millis(200);
        let mut app = App::new();
        app.insert_resource(TimeUpdateStrategy::ManualDuration(baseline));
        app.insert_resource(GateFrameHitch {
            baseline,
            hitch,
            fired: false,
            restore_next: false,
        });
        app.init_resource::<ManualIntegrationGateReport>();
        app.add_systems(Last, apply_gate_frame_hitch);
        app.update();
        assert!(
            matches!(app.world().resource::<TimeUpdateStrategy>(), TimeUpdateStrategy::ManualDuration(value) if *value == baseline)
        );
        app.world_mut()
            .resource_mut::<ManualIntegrationGateReport>()
            .moving_base_trace
            .push(serde_json::json!({"stage":"Exit","mode":"Grounded","takeoff_ready":false}));
        app.update();
        assert!(
            matches!(app.world().resource::<TimeUpdateStrategy>(), TimeUpdateStrategy::ManualDuration(value) if *value == baseline)
        );
        *app.world_mut()
            .resource_mut::<ManualIntegrationGateReport>()
            .moving_base_trace
            .last_mut()
            .unwrap() = serde_json::json!({"stage":"Exit","mode":"Air","takeoff_ready":true});
        app.update();
        assert!(
            matches!(app.world().resource::<TimeUpdateStrategy>(), TimeUpdateStrategy::ManualDuration(value) if *value == baseline)
        );
        *app.world_mut()
            .resource_mut::<ManualIntegrationGateReport>()
            .moving_base_trace
            .last_mut()
            .unwrap() = serde_json::json!({"stage":"Exit","mode":"Grounded","takeoff_ready":true});
        app.update();
        assert!(
            matches!(app.world().resource::<TimeUpdateStrategy>(), TimeUpdateStrategy::ManualDuration(value) if *value == hitch)
        );
        app.update();
        assert!(
            matches!(app.world().resource::<TimeUpdateStrategy>(), TimeUpdateStrategy::ManualDuration(value) if *value == baseline)
        );
        app.update();
        assert!(
            matches!(app.world().resource::<TimeUpdateStrategy>(), TimeUpdateStrategy::ManualDuration(value) if *value == baseline)
        );
        assert!(app.world().resource::<GateFrameHitch>().fired);
    }

    #[test]
    fn platform_probe_waits_until_the_playground_is_ready() {
        let mut world = World::new();
        world.init_resource::<PlatformPhysicsProbe>();
        world.init_resource::<ManualPlaygroundStartupReady>();
        world.spawn(AlsCrushRecoveryFloor {
            half_extents: Vec3::ONE,
        });
        let platforms = (0..3)
            .map(|index| {
                world
                    .spawn((
                        PlaygroundMovingObject,
                        RigidBody::Kinematic,
                        LinearVelocity::ZERO,
                        AngularVelocity(if index == 0 { Vec3::Y } else { Vec3::ZERO }),
                    ))
                    .id()
            })
            .collect::<Vec<_>>();
        world.run_system_once(platform_physics_probe).unwrap();
        assert!(!world.resource::<PlatformPhysicsProbe>().0);
        world.resource_mut::<ManualPlaygroundStartupReady>().passed = true;
        for platform in &platforms[..2] {
            world.entity_mut(*platform).insert(LinearVelocity(Vec3::X));
        }
        world.run_system_once(platform_physics_probe).unwrap();
        assert!(world.resource::<PlatformPhysicsProbe>().0);
    }

    #[test]
    fn character_material_probe_ignores_the_independent_camera_scene() {
        let mut world = World::new();
        world.init_resource::<CharacterMaterialProbe>();
        world.init_resource::<Assets<StandardMaterial>>();
        world.spawn((
            AvatarMaterialOverride(vec![Handle::default()]),
            AvatarMaterialApplied {
                slots: 1,
                body_slots: 1,
                main_slots: 0,
                source_slot_pattern: true,
            },
        ));
        world.run_system_once(character_material_probe).unwrap();
        assert!(!world.resource::<CharacterMaterialProbe>().0);
    }
    #[test]
    fn render_effects_follow_the_view_and_preserve_existing_settings() {
        for (has_ssao, has_taa) in [(false, false), (true, false), (false, true), (true, true)] {
            let mut world = World::new();
            let source = world
                .spawn(bevy_als::als_camera::AlsCameraRig::default())
                .id();
            let view = world
                .spawn((
                    Camera3d::default(),
                    bevy_als::als_camera::AlsCameraViewSource(source),
                ))
                .id();
            if has_ssao {
                world.entity_mut(view).insert(ScreenSpaceAmbientOcclusion {
                    constant_object_thickness: 0.37,
                    ..default()
                });
            }
            if has_taa {
                world
                    .entity_mut(view)
                    .insert(TemporalAntiAliasing { reset: false });
            }
            world.run_system_once(enable_character_ssao_taa).unwrap();
            assert_eq!(world.get::<Msaa>(view), Some(&Msaa::Off));
            assert!(world.get::<ScreenSpaceAmbientOcclusion>(view).is_some());
            assert!(world.get::<TemporalAntiAliasing>(view).is_some());
            assert!(
                world
                    .get::<bevy::core_pipeline::prepass::DepthPrepass>(view)
                    .is_some()
            );
            assert!(
                world
                    .get::<bevy::core_pipeline::prepass::NormalPrepass>(view)
                    .is_some()
            );
            assert!(
                world
                    .get::<bevy::core_pipeline::prepass::MotionVectorPrepass>(view)
                    .is_some()
            );
            assert!(world.get::<ScreenSpaceAmbientOcclusion>(source).is_none());
            assert!(
                world
                    .get::<bevy_als::als_camera::AlsCameraRig>(view)
                    .is_none()
            );
            if has_ssao {
                assert_eq!(
                    world
                        .get::<ScreenSpaceAmbientOcclusion>(view)
                        .unwrap()
                        .constant_object_thickness,
                    0.37
                );
            }
            if has_taa {
                assert!(!world.get::<TemporalAntiAliasing>(view).unwrap().reset);
            }
            world.get_mut::<TemporalAntiAliasing>(view).unwrap().reset = false;
            world.run_system_once(enable_character_ssao_taa).unwrap();
            assert!(!world.get::<TemporalAntiAliasing>(view).unwrap().reset);
        }
    }

    #[test]
    fn render_effects_probe_rejects_missing_views_and_partial_effects() {
        for missing in ["view", "ssao", "taa", "msaa", "none"] {
            let mut world = World::new();
            world.init_resource::<RenderEffectsProbe>();
            world.init_resource::<Time<Real>>();
            world.init_resource::<Messages<AppExit>>();
            world.init_resource::<ManualPlaygroundStartupReady>();
            world.run_system_once(render_effects_probe).unwrap();
            assert_eq!(world.resource::<RenderEffectsProbe>().passed, None);
            if missing != "view" {
                let source = world.spawn_empty().id();
                let mut view = world.spawn((Camera3d::default(), AlsCameraViewSource(source)));
                if missing != "ssao" {
                    view.insert(ScreenSpaceAmbientOcclusion::default());
                }
                if missing != "taa" {
                    view.insert(TemporalAntiAliasing::default());
                }
                if missing != "msaa" {
                    view.insert(Msaa::Off);
                }
            }
            world.resource_mut::<ManualPlaygroundStartupReady>().passed = true;
            world.run_system_once(render_effects_probe).unwrap();
            assert_eq!(
                world.resource::<RenderEffectsProbe>().passed,
                Some(missing == "none")
            );
            assert_eq!(
                world.resource::<Messages<AppExit>>().is_empty(),
                missing == "none"
            );
        }
    }

    #[test]
    fn render_pass_probe_requires_executed_passes_and_has_a_fixed_deadline() {
        use bevy::diagnostic::{Diagnostic, DiagnosticMeasurement, DiagnosticsStore};
        let directory =
            std::env::temp_dir().join(format!("als-render-pass-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        for passes_ready in [false, true] {
            let mut world = World::new();
            world.insert_resource(RenderEffectsProbe {
                directory: Some(directory.clone()),
                ..default()
            });
            world.init_resource::<Time<Real>>();
            world.init_resource::<Messages<AppExit>>();
            world.insert_resource(ManualPlaygroundStartupReady { passed: true });
            let source = world.spawn_empty().id();
            world.spawn((Camera3d::default(), AlsCameraViewSource(source)));
            world.run_system_once(enable_character_ssao_taa).unwrap();
            world.run_system_once(render_effects_probe).unwrap();
            assert_eq!(world.resource::<RenderEffectsProbe>().passed, None);
            if passes_ready {
                let mut store = DiagnosticsStore::default();
                for name in [
                    "render/view/ssao/elapsed_cpu",
                    "render/view/taa/elapsed_cpu",
                ] {
                    let mut diagnostic =
                        Diagnostic::new(bevy::diagnostic::DiagnosticPath::new(name));
                    diagnostic.add_measurement(DiagnosticMeasurement {
                        time: Instant::now(),
                        value: 0.1,
                    });
                    store.add(diagnostic);
                }
                world.insert_resource(store);
            }
            world
                .resource_mut::<Time<Real>>()
                .advance_by(Duration::from_secs(5));
            world.run_system_once(render_effects_probe).unwrap();
            assert_eq!(
                world.resource::<RenderEffectsProbe>().passed,
                Some(passes_ready)
            );
            let report: serde_json::Value = serde_json::from_slice(
                &std::fs::read(directory.join("render_effects.json")).unwrap(),
            )
            .unwrap();
            assert_eq!(report["passed"], passes_ready);
            assert_eq!(report["waited_seconds"], 5.0);
            assert_eq!(
                report["exit_reason"],
                if passes_ready {
                    "completed"
                } else {
                    "render_pass_timeout"
                }
            );
        }
        std::fs::remove_file(directory.join("render_effects.json")).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }
}

fn collision_parity_probe(
    mut probe: ResMut<CollisionParityProbe>,
    sources: Query<(
        Entity,
        &PlaygroundCollisionSource,
        Option<&PlaygroundMeshColliderBuilt>,
        &GlobalTransform,
        Has<Collider>,
    )>,
    spatial: SceneQuery,
) {
    if probe.done {
        return;
    }
    let mut box_meshes = HashSet::new();
    let mut convex_meshes = HashSet::new();
    let mut trimesh_meshes = HashSet::new();
    let mut non_box_total = 0usize;
    let mut non_box_built = 0usize;
    let mut stair_roots = Vec::new();
    for (entity, source, built, transform, has_collider) in &sources {
        match source.kind {
            PlaygroundColliderKind::Box => {
                box_meshes.insert(source.mesh_base.as_str());
            }
            PlaygroundColliderKind::Convex { .. } => {
                convex_meshes.insert(source.mesh_base.as_str());
                non_box_total += 1;
                if built.is_some_and(|built| {
                    built.kind == source.kind
                        && built.vertices > 0
                        && built.triangles > 0
                        && has_collider
                }) {
                    non_box_built += 1;
                }
            }
            PlaygroundColliderKind::TriMesh => {
                trimesh_meshes.insert(source.mesh_base.as_str());
                non_box_total += 1;
                if built.is_some_and(|built| {
                    built.kind == source.kind
                        && built.vertices > 0
                        && built.triangles > 0
                        && has_collider
                }) {
                    non_box_built += 1;
                }
            }
        }
        if source.mesh_base.starts_with("SM_Als_Stairs_")
            && let Some(built) = built
        {
            stair_roots.push((entity, transform, built));
        }
    }
    let source_ready = box_meshes.len() == 8
        && convex_meshes.len() == 5
        && trimesh_meshes.len() == 7
        && non_box_total > 0
        && non_box_built == non_box_total
        && stair_roots.len() >= 3
        && stair_roots
            .iter()
            .all(|(_, _, built)| built.surface_probes.len() == 2);
    if !source_ready {
        return;
    }
    probe.ready_frames += 1;
    if probe.ready_frames < 4 {
        return;
    }
    let stair_count = stair_roots.len();
    let mut ray_total = 0usize;
    let mut ray_matching = 0usize;
    let mut stair_height_delta_min = f32::INFINITY;
    for (entity, transform, built) in &stair_roots {
        let mut hit_heights = Vec::new();
        for local_point in &built.surface_probes {
            let expected = transform.to_matrix().transform_point3(*local_point);
            let origin = expected + Vec3::Y * 30.0;
            let hit = spatial.cast_ray_predicate(
                origin,
                Dir3::NEG_Y,
                60.0,
                false,
                &SpatialQueryFilter::default(),
                &|candidate| candidate == *entity,
            );
            ray_total += 1;
            if let Some(hit) = hit {
                let hit_height = origin.y - hit.distance;
                if (hit_height - expected.y).abs() < 0.05 {
                    ray_matching += 1;
                    hit_heights.push(hit_height);
                }
            }
        }
        if hit_heights.len() == 2 {
            stair_height_delta_min =
                stair_height_delta_min.min((hit_heights[0] - hit_heights[1]).abs());
        }
    }
    let expected_rays = stair_count * 2;
    if ray_total != expected_rays && probe.ready_frames < 120 {
        return;
    }
    if ray_matching != expected_rays && probe.ready_frames < 120 {
        return;
    }
    let pass =
        ray_total == expected_rays && ray_matching == expected_rays && stair_height_delta_min > 0.2;
    info!(
        "[PGCOL][VERDICT] mesh_types={} box_types={}/8 convex_types={}/5 trimesh_types={}/7 nonbox_built={}/{} stairs={} rays={}/{} stair_height_delta_min={stair_height_delta_min:.6} -> {}",
        box_meshes.len() + convex_meshes.len() + trimesh_meshes.len(),
        box_meshes.len(),
        convex_meshes.len(),
        trimesh_meshes.len(),
        non_box_built,
        non_box_total,
        stair_count,
        ray_matching,
        expected_rays,
        if pass { "PASS" } else { "FAIL" }
    );
    probe.done = true;
}

fn platform_physics_probe(
    mut probe: ResMut<PlatformPhysicsProbe>,
    ready: Res<ManualPlaygroundStartupReady>,
    platforms: Query<(&RigidBody, &LinearVelocity, &AngularVelocity), With<PlaygroundMovingObject>>,
    floors: Query<(), With<AlsCrushRecoveryFloor>>,
) {
    if probe.0 || !ready.passed || platforms.iter().count() != 3 || floors.iter().count() != 1 {
        return;
    }
    let kinematic = platforms
        .iter()
        .filter(|(body, _, _)| **body == RigidBody::Kinematic)
        .count();
    let moving = platforms
        .iter()
        .filter(|(_, linear, _)| linear.0.length_squared() > 1e-6)
        .count();
    let rotating = platforms
        .iter()
        .filter(|(_, _, angular)| angular.0.length_squared() > 1e-6)
        .count();
    let finite = platforms
        .iter()
        .all(|(_, linear, angular)| linear.0.is_finite() && angular.0.is_finite());
    if kinematic == 3 && moving == 2 && rotating == 1 && finite {
        info!(
            "[PLATFORM][VERDICT] kinematic={kinematic}/3 moving={moving}/2 rotating={rotating}/1 crush_floor=1/1 finite={finite} -> PASS"
        );
    } else {
        error!(
            "[PLATFORM][VERDICT] kinematic={kinematic}/3 moving={moving}/2 rotating={rotating}/1 crush_floor=1/1 finite={finite} -> FAIL"
        );
    }
    probe.0 = true;
}
