use super::*;
use crate::projected_decal::{
    NoDecalReceiver, PackedDecal, PackedDecalMaterial, PackedDecalPrepareSet,
    PackedDecalReceiverMaterial, StandardMaterialSource,
};
use bevy_als::footprint::{Footprint, FootprintDiagnostics, footprint_opacity};

pub(super) const LABELS: [&str; 8] = [
    "footprint_prepare",
    "footprint_walk",
    "footprint_walk_stop",
    "footprint_run",
    "footprint_run_stop",
    "footprint_crouch",
    "footprint_crouch_stop",
    "footprint_cleanup",
];

#[derive(Default)]
pub(super) struct Probe {
    pub(super) stage: usize,
    pub(super) covered: [bool; 8],
    pub(super) previous_spawned: [u64; 2],
    pub(super) visible: bool,
    pub(super) faded: bool,
    pub(super) expired: bool,
    pub(super) moving: bool,
    pub(super) receiver: bool,
    pub(super) pipelines: bool,
    pub(super) errors: Vec<String>,
    pub(super) waited: f32,
    anchors: std::collections::HashMap<Entity, Vec3>,
    last_stage: usize,
    stable_since: Option<f64>,
    pub(super) gpu: GpuProbe,
    gpu_stage_frame: u32,
    moving_gpu_started: Option<u32>,
    pub(super) moving_gpu: bool,
    pub(super) debug: bool,
}

#[derive(Default)]
pub(super) struct GpuProbe {
    root: Option<Entity>,
    library: Handle<bevy_als::footprint::FootprintLibrary>,
    material: Handle<PackedDecalMaterial>,
    decals: Vec<Entity>,
    pub(super) cursor: usize,
    applied_stage: Option<usize>,
    pub(super) last_progress: Option<Instant>,
    pub(super) complete: bool,
    completion_frame: u32,
}

#[derive(Clone, Copy, Debug)]
struct GpuStep {
    stage: usize,
    count: usize,
    receiver: u32,
    index: Option<u32>,
}

fn gpu_step(mut cursor: usize) -> Option<GpuStep> {
    for (stage, count) in [0_usize, 1, 2, 3, 2, 1, 0, 3, 3, 0].into_iter().enumerate() {
        let requests = count.max(1) * 2;
        if cursor < requests {
            return Some(GpuStep {
                stage,
                count,
                receiver: (cursor % 2 + 1) as u32,
                index: (count > 0).then_some((cursor / 2) as u32),
            });
        }
        cursor -= requests;
    }
    None
}

#[derive(Component)]
struct GpuProbeRoot;

#[derive(Component)]
struct ManualReceiverReplacement(Handle<StandardMaterial>);

fn gpu_probe_stalled(waited: f32) -> bool {
    waited > 12.0
}

pub(super) fn install(app: &mut App) {
    app.add_systems(
        PreUpdate,
        drive_gpu
            .after(drive_manual_integration)
            .run_if(|state: Res<ManualIntegrationState>| {
                state.scenario == IntegrationScenario::Footprint
            }),
    );
    app.add_systems(
        PostUpdate,
        observe
            .run_if(|state: Res<ManualIntegrationState>| {
                state.scenario == IntegrationScenario::Footprint
            })
            .after(crate::packed_footprints::PackedFootprintSet)
            .after(AlsCameraSet)
            .after(bevy_als::debug_display::AlsDebugDisplaySet)
            .after(bevy::camera::visibility::VisibilitySystems::CheckVisibility)
            .before(observe_manual_integration)
            .in_set(ManualIntegrationObservationSet),
    );
    if std::env::var("ALS_MANUAL_DECAL_RECEIVER_SWAP").as_deref() == Ok("1") {
        app.add_systems(
            Update,
            replace_receiver_material.before(PackedDecalPrepareSet),
        );
        app.add_systems(
            PostUpdate,
            observe_receiver_material_swap
                .after(PackedDecalPrepareSet)
                .before(observe),
        );
    }
}

fn replace_receiver_material(
    mut commands: Commands,
    state: Res<ManualIntegrationState>,
    children: Query<&Children>,
    converted: Query<
        (
            &MeshMaterial3d<PackedDecalReceiverMaterial>,
            &StandardMaterialSource,
        ),
        With<Mesh3d>,
    >,
    replacements: Query<Entity, With<ManualReceiverReplacement>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if !state.initialized
        || state.scenario != IntegrationScenario::Footprint
        || gpu_step(state.footprint_probe.gpu.cursor).is_some()
        || !replacements.is_empty()
    {
        return;
    }
    let Some(root) = state.footprint_probe.gpu.root else {
        return;
    };
    let Some(entity) = children
        .iter_descendants(root)
        .find(|entity| converted.get(*entity).is_ok())
    else {
        return;
    };
    let material = materials.add(StandardMaterial {
        base_color: Color::srgba(0.2, 0.7, 0.4, 0.5),
        perceptual_roughness: 0.8,
        metallic: 0.2,
        reflectance: 0.5,
        alpha_mode: AlphaMode::Blend,
        ..default()
    });
    commands.entity(entity).insert((
        MeshMaterial3d(material.clone()),
        ManualReceiverReplacement(material.clone()),
    ));
    info!(
        "[MANUAL-INTEGRATION][DECAL-RECEIVER-SWAP] entity={entity:?} new_material={:?}",
        material.id()
    );
}

fn observe_receiver_material_swap(
    mut state: ResMut<ManualIntegrationState>,
    replacements: Query<
        (
            Entity,
            &ManualReceiverReplacement,
            Option<&MeshMaterial3d<StandardMaterial>>,
            Has<MeshMaterial3d<PackedDecalReceiverMaterial>>,
            Has<StandardMaterialSource>,
        ),
        With<Mesh3d>,
    >,
) {
    if state.scenario != IntegrationScenario::Footprint {
        return;
    }
    let Ok((entity, replacement, current, packed, source)) = replacements.single() else {
        return;
    };
    if current.is_some_and(|material| material.0 == replacement.0) && !packed && !source {
        if !state.coverage.decal_receiver_swap {
            info!(
                "[MANUAL-INTEGRATION][DECAL-RECEIVER-SWAP] entity={entity:?} current_material={:?} packed=false source=false -> PASS",
                replacement.0.id()
            );
        }
        state.coverage.decal_receiver_swap = true;
    }
}

fn drive_gpu(
    mut commands: Commands,
    mut state: ResMut<ManualIntegrationState>,
    fixture: Option<Res<ManualIntegrationFixture>>,
    server: Res<AssetServer>,
    libraries: Res<Assets<bevy_als::footprint::FootprintLibrary>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut standard: ResMut<Assets<StandardMaterial>>,
    mut tiles: ResMut<Assets<crate::playground_materials::PlaygroundTilesMaterial>>,
    tile_library: Res<crate::playground_materials::PlaygroundMaterialLibrary>,
    mut materials: ResMut<Assets<PackedDecalMaterial>>,
    mut request: ResMut<crate::decal_diagnostics::DecalProbeRequest>,
    render: Res<crate::decal_diagnostics::DecalRenderStatus>,
    time: Res<Time>,
    frame: Res<bevy::diagnostic::FrameCount>,
    mut roots: Query<&mut Transform, With<GpuProbeRoot>>,
) {
    if !state.initialized
        || state.phase != IntegrationPhase::Footprints
        || state.footprint_probe.stage != 0
        || state.footprint_probe.gpu.complete
    {
        return;
    }
    let Some(fixture) = fixture else {
        return;
    };
    let gpu = &mut state.footprint_probe.gpu;
    let waited = gpu
        .last_progress
        .get_or_insert_with(Instant::now)
        .elapsed()
        .as_secs_f32();
    if gpu_probe_stalled(waited) {
        let cursor = gpu.cursor;
        state.footprint_probe.errors.push(format!(
            "Footprint GPU request {} made no progress for {waited:.3} seconds",
            cursor + 1
        ));
        state.timed_out = true;
        begin_phase(&mut state, IntegrationPhase::Finish);
        return;
    }
    if gpu.root.is_none() {
        gpu.library = server.load("ALS/Data/Footstep/footprint.runtime.json");
        let Some(library) = libraries.get(&gpu.library) else {
            return;
        };
        let Some(definition) = library.definitions.iter().find(|definition| {
            definition.source_settings == "/ALS/ALS/Data/Footstep/FES_Als_Default.FES_Als_Default"
        }) else {
            return;
        };
        let Some(source) = definition
            .effects
            .first()
            .and_then(|effect| effect.material.as_ref())
            .and_then(|material| definition.materials.get(material))
        else {
            return;
        };
        let Some(grid) = tile_library.tiles_grid.clone() else {
            return;
        };
        gpu.material = materials.add(PackedDecalMaterial {
            color: Vec4::new(
                source.base_color,
                source.base_color,
                source.base_color,
                source.opacity,
            ),
            roughness_scale: source.roughness_scale,
            metallic: source.metallic,
            specular: source.specular,
            texture: source.image.clone(),
        });
        let base = StandardMaterial {
            base_color: Color::linear_rgb(0.8, 0.7, 0.6),
            perceptual_roughness: 0.8,
            metallic: 0.1,
            reflectance: 0.7,
            ..default()
        };
        let plain = standard.add(base.clone());
        use crate::playground_materials::{
            PlaygroundTilesExtension, PlaygroundTilesMaterial, PlaygroundTilesSettings,
        };
        let patterned = tiles.add(PlaygroundTilesMaterial {
            base,
            extension: PlaygroundTilesExtension {
                decals: tile_library.decal_buffers.materials.clone(),
                probe: tile_library.decal_buffers.probe.clone(),
                grid_texture: grid,
                settings: PlaygroundTilesSettings {
                    side_color_a: Vec4::new(0.8, 0.7, 0.6, 1.0),
                    top_color_a: Vec4::new(0.8, 0.7, 0.6, 1.0),
                    side_color_b: Vec4::new(0.8, 0.7, 0.6, 1.0),
                    top_color_b: Vec4::new(0.8, 0.7, 0.6, 1.0),
                    tiling_and_mode: Vec4::new(1.0, 0.0, 0.0, 0.0),
                },
            },
        });
        let facing = Quat::from_rotation_y(SPRINT_YAW);
        let center = GROUNDED_START.with_y(fixture.locomotion_start.y - 0.92 + 0.04)
            + facing * Vec3::new(1.0, 0.0, -2.0);
        let root = commands
            .spawn((
                Name::new("Footprint GPU receiver fixture"),
                GpuProbeRoot,
                Transform::from_translation(center).with_rotation(facing),
                Visibility::default(),
            ))
            .id();
        let mesh = meshes.add(Cuboid::new(1.4, 0.02, 3.4));
        commands.spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(plain),
            Transform::from_xyz(-0.7, 0.0, 0.0),
            ChildOf(root),
        ));
        commands.spawn((
            Mesh3d(mesh),
            MeshMaterial3d(patterned),
            Transform::from_xyz(0.7, 0.0, 0.0),
            ChildOf(root),
        ));
        gpu.root = Some(root);
        gpu.last_progress = Some(Instant::now());
    }
    let Some(step) = gpu_step(gpu.cursor) else {
        if let Some(root) = gpu.root.take() {
            commands.entity(root).despawn();
        }
        gpu.decals.clear();
        gpu.complete = true;
        gpu.completion_frame = frame.0.wrapping_add(1);
        return;
    };
    if gpu.applied_stage != Some(step.stage) {
        while gpu.decals.len() > step.count {
            commands.entity(gpu.decals.pop().unwrap()).despawn();
        }
        while gpu.decals.len() < step.count {
            let index = gpu.decals.len();
            gpu.decals.push(
                commands
                    .spawn((
                        PackedDecal(gpu.material.clone()),
                        ChildOf(gpu.root.unwrap()),
                        Transform::from_xyz(0.0, 0.03, index as f32 - 1.0)
                            .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2))
                            .with_scale(Vec3::new(0.7, 0.8, 0.4)),
                    ))
                    .id(),
            );
        }
        if step.stage == 8 {
            materials.get_mut(&gpu.material).unwrap().color.w *= 0.5;
        }
        gpu.applied_stage = Some(step.stage);
    }
    if step.stage == 7
        && let Ok(mut root) = roots.get_mut(gpu.root.unwrap())
    {
        root.translation.x += time.delta_secs() * 0.4;
    }
    let Ok(root) = roots.get(gpu.root.unwrap()) else {
        return;
    };
    let next = crate::decal_diagnostics::DecalProbeRequest {
        id: gpu.cursor as u32 + 1,
        receiver: step.receiver,
        index: step.index,
        plane: Vec4::new(
            root.translation.y + 0.01,
            root.translation.x,
            root.translation.z,
            1.6,
        ),
    };
    if *request != next {
        *request = next;
    }
    let status = render.0.lock().unwrap();
    if let Some(error) = status.probe_errors.first() {
        state.footprint_probe.errors.push(error.clone());
        begin_phase(&mut state, IntegrationPhase::Finish);
        return;
    }
    if let Some(result) = status.probes.iter().find(|result| {
        result.id == next.id
            && result.count == crate::projected_decal::DECAL_PROBE_SAMPLES
    }) {
        if result.max_error > 1e-5 || !status.probe_errors.is_empty() {
            state.footprint_probe.errors.push(format!(
                "Footprint GPU request {} failed: error={}",
                next.id, result.max_error
            ));
            begin_phase(&mut state, IntegrationPhase::Finish);
        } else {
            gpu.cursor += 1;
            gpu.last_progress = Some(Instant::now());
        }
    }
}

fn observe(
    mut state: ResMut<ManualIntegrationState>,
    mut report: ResMut<ManualIntegrationGateReport>,
    mut window: ResMut<ManualAnimationWindowEvidence>,
    time: Res<Time>,
    characters: Query<(
        &FootprintDiagnostics,
        &LocomotionState,
        &Stance,
        &AlsActualMotion,
        &AlsStandingGraphTelemetry,
        &AlsCrouchingGraphTelemetry,
    )>,
    footprints: Query<(
        Entity,
        &Footprint,
        &Transform,
        &GlobalTransform,
        Option<&ChildOf>,
        &PackedDecal,
        &ViewVisibility,
    )>,
    globals: Query<&GlobalTransform>,
    owners: Query<&avian3d::prelude::ColliderOf>,
    bodies: Query<&RigidBody>,
    materials: Res<Assets<PackedDecalMaterial>>,
    no_receivers: Query<&MeshMaterial3d<StandardMaterial>, With<NoDecalReceiver>>,
    images: Res<Assets<Image>>,
    render: (
        Res<crate::decal_diagnostics::DecalRenderStatus>,
        Res<bevy::diagnostic::FrameCount>,
    ),
    debug: Res<bevy_als::debug_display::AlsDebugDisplayStats>,
) {
    let (render, frame) = render;
    let Some(character) = state.character else {
        return;
    };
    let Ok((diagnostics, mode, stance, motion, standing, crouching)) = characters.get(character)
    else {
        return;
    };
    let elapsed = state.phase_elapsed();
    let phase = state.phase;
    let probe = &mut state.footprint_probe;
    probe.debug |= debug.footstep_trace_lines_drawn > 0
        && debug.footstep_axis_lines_drawn == 3 * debug.footstep_trace_lines_drawn
        && debug.footstep_hit_points_drawn > 0;
    let pipelines = render.0.lock().unwrap().clone();
    probe.pipelines =
        pipelines.decal_ready && pipelines.receiver_ready && pipelines.errors.is_empty();
    if diagnostics.invalid > 0
        && !probe
            .errors
            .iter()
            .any(|error| error == "Footstep inputs or final bones were invalid")
    {
        probe
            .errors
            .push("Footstep inputs or final bones were invalid".into());
    }
    probe.receiver |= no_receivers.iter().count() >= 15;
    probe.expired |= diagnostics.expired > 0;
    let mut alive = 0;
    let mut visible_meshes = 0;
    let mut loaded_images = 0;
    for (entity, mark, local, world, parent, material, visible) in &footprints {
        if mark.character != character {
            continue;
        }
        alive += 1;
        let age = time.elapsed_secs_f64() - mark.created_at;
        let alpha = mark.opacity * footprint_opacity(age, mark.lifetime);
        let material = materials.get(&material.0);
        if material.is_none_or(|material| (material.color.w - alpha).abs() > 1e-5) {
            if !probe
                .errors
                .iter()
                .any(|error| error == "Footprint opacity differs from source lifetime")
            {
                probe
                    .errors
                    .push("Footprint opacity differs from source lifetime".into());
            }
        }
        probe.faded |= alpha > 0.0 && alpha < mark.opacity - 0.01;
        let image_loaded = material.is_some_and(|material| {
            images
                .get(&material.texture)
                .is_some_and(|image| image.size() == UVec2::splat(512))
        });
        visible_meshes += usize::from(visible.get());
        loaded_images += usize::from(image_loaded);
        let valid_parent = parent.is_some_and(|parent| parent.parent() == mark.surface);
        if !valid_parent {
            probe
                .errors
                .push("Footprint is not attached to its hit component".into());
        }
        if let Ok(parent) = globals.get(mark.surface) {
            let expected = parent.mul_transform(*local);
            if expected.translation().distance(world.translation()) > 1e-4 {
                probe
                    .errors
                    .push("Footprint world location did not follow its hit component".into());
            }
        }
        let body = owners
            .get(mark.surface)
            .map_or(mark.surface, |owner| owner.body);
        if bodies
            .get(body)
            .is_ok_and(|body| *body == RigidBody::Kinematic)
        {
            let start = *probe.anchors.entry(entity).or_insert(world.translation());
            probe.moving |= start.distance(world.translation()) > 0.05;
        }
    }
    probe
        .anchors
        .retain(|entity, _| footprints.contains(*entity));
    probe.visible |= probe.gpu.complete && pipelines.positive_frame > probe.gpu.completion_frame;
    if phase == IntegrationPhase::MovingBase {
        let start = *probe
            .moving_gpu_started
            .get_or_insert(frame.0.wrapping_add(1));
        probe.moving_gpu |= probe.moving && pipelines.positive_frame > start;
    }
    for error in &pipelines.probe_errors {
        if !probe.errors.contains(error) {
            probe.errors.push(error.clone());
        }
    }
    if phase == IntegrationPhase::Footprints {
        let stage = probe.stage;
        if probe.last_stage != stage {
            probe.previous_spawned = diagnostics.spawned;
            probe.last_stage = stage;
            probe.stable_since = None;
            probe.gpu_stage_frame = pipelines.positive_frame;
        }
        let moving = matches!(stage, 1 | 3 | 5);
        let (actual, transition, pose) = if stage == 5 {
            (
                crouching.actual_outer_state,
                crouching.actual_outer_transition_active,
                crouching.actual_outer_pose_ready,
            )
        } else {
            (
                standing.actual_standing_state,
                standing.actual_standing_transition_active,
                standing.actual_standing_pose_ready,
            )
        };
        let expected = u8::from(moving);
        let stable = *mode == LocomotionState::Grounded
            && *stance
                == if stage == 5 {
                    Stance::Crouching
                } else {
                    Stance::Standing
                }
            && actual == Some(expected)
            && transition == Some(false)
            && pose == Some(true)
            && if moving {
                motion.horizontal_speed > 0.5
            } else {
                motion.horizontal_speed < 0.1
            };
        let observed = if stable {
            time.elapsed_secs_f64() - *probe.stable_since.get_or_insert(time.elapsed_secs_f64())
        } else {
            probe.stable_since = None;
            0.0
        };
        let footprint_pair =
            (0..2).all(|side| diagnostics.spawned[side] > probe.previous_spawned[side]);
        probe.covered[stage] |= stable
            && observed >= 0.3
            && if stage == 7 {
                elapsed > 8.5 && alive == 0
            } else if moving {
                elapsed > 2.0 && footprint_pair && pipelines.positive_frame > probe.gpu_stage_frame
            } else {
                elapsed > 0.5
            };
        *window = ManualAnimationWindowEvidence {
            label: Some(LABELS[stage]),
            index: Some(stage),
            settled: stable,
            source_node: Some(if stage == 5 {
                "AB_Als_Crouching/AnimGraph"
            } else {
                "AB_Als_Standing/AnimGraph"
            }),
            source_state: Some(if moving { "Move" } else { "Idle" }),
            expected_runtime_state: Some(expected),
            observed_runtime_state: actual,
            capture_source_window: stable,
            capture_every_frame: true,
            requires_non_reference_root_pose: true,
            requires_source_pose_chain: moving,
            minimum_observation_seconds: 0.25,
            source_pose_chain: if moving {
                vec![ManualAnimationPoseBoundary {
                    label: "locomotion",
                    graph: LOCOMOTION_ROOT,
                    node: "Output Pose\nAnimGraph",
                    output: "Pose",
                    minimum_translation_span: 0.0,
                    minimum_rotation_span_radians: 0.0,
                }]
            } else {
                Vec::new()
            },
            ..default()
        };
    }
    report.direction_trace.push(serde_json::json!({"footprint_stage": probe.stage, "time": time.elapsed_secs_f64(), "alive": alive,
        "notifications":diagnostics.notifications,"spawned":diagnostics.spawned,"expired":diagnostics.expired,
        "missed":diagnostics.missed,"angle":diagnostics.rejected_angle,"invalid":diagnostics.invalid,
        "visible":probe.visible,"visible_meshes":visible_meshes,"loaded_images":loaded_images,"moving":probe.moving,"receiver":probe.receiver,"pipelines":probe.pipelines,
        "pipeline_errors":pipelines.errors,"errors":probe.errors,"gpu_request":probe.gpu.cursor,"gpu_complete":probe.gpu.complete,
        "gpu_positive_frame":pipelines.positive_frame,"gpu_moving":probe.moving_gpu,"gpu_errors":pipelines.probe_errors,
        "debug_traces":debug.footstep_trace_lines_drawn,"debug_axes":debug.footstep_axis_lines_drawn,"debug_points":debug.footstep_hit_points_drawn}));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gpu_plan_covers_each_receiver_and_index_through_growth_removal_motion_and_fade() {
        let steps: Vec<_> = (0..).map_while(gpu_step).collect();
        for (stage, count) in [0, 1, 2, 3, 2, 1, 0, 3, 3, 0].into_iter().enumerate() {
            let actual: Vec<_> = steps
                .iter()
                .filter(|step| step.stage == stage)
                .map(|step| (step.receiver, step.index))
                .collect();
            assert_eq!(actual.len(), count.max(1) * 2);
            for receiver in [1, 2] {
                for index in 0..count.max(1) {
                    assert!(actual.contains(&(receiver, (count > 0).then_some(index as u32))));
                }
            }
        }
    }

    #[test]
    fn gpu_probe_ends_after_its_fixed_no_progress_limit() {
        assert!(!gpu_probe_stalled(12.0));
        assert!(gpu_probe_stalled(12.01));
    }
}
