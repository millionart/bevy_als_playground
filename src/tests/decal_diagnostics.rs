use bevy::{prelude::*, render::storage::ShaderBuffer};
use std::sync::{Arc, Mutex};

use crate::projected_decal::{
    DECAL_PROBE_SAMPLES, DECAL_PROBE_WORDS, NoDecalReceiver, PackedDecalBuffers,
    PackedDecalPrepareSet, PackedDecalReceiverMaterial,
};

#[derive(Resource, Clone, Copy, Default, Debug, PartialEq)]
pub struct DecalProbeRequest {
    pub id: u32,
    pub receiver: u32,
    pub index: Option<u32>,
    pub plane: Vec4,
}

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct DecalProbeResult {
    pub id: u32,
    pub receiver: u32,
    pub index: Option<u32>,
    pub count: usize,
    pub max_error: f32,
    pub samples: Vec<Vec<f32>>,
    pub plane: [f32; 4],
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct DecalBindingSnapshot {
    pub frame: u32,
    pub logical_buffer: String,
    pub gpu_buffer: String,
    pub bytes: u64,
    pub receiver_bind_groups: Vec<String>,
}

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct DecalPipelines {
    pub decal_ready: bool,
    pub receiver_ready: bool,
    pub clustered_supported: bool,
    pub native_decals: usize,
    pub material_response_samples: usize,
    pub material_response_max_error: f32,
    pub positive_frame: u32,
    pub checked_frame: u32,
    pub probes: Vec<DecalProbeResult>,
    pub probe_errors: Vec<String>,
    pub bindings: Vec<DecalBindingSnapshot>,
    pub errors: Vec<String>,
    pub pipeline_labels: Vec<String>,
}

#[derive(Resource, Clone, Default)]
pub struct DecalRenderStatus(pub Arc<Mutex<DecalPipelines>>);

pub struct DecalDiagnosticsPlugin;

impl Plugin for DecalDiagnosticsPlugin {
    fn build(&self, app: &mut App) {
        let probe_enabled =
            std::env::var("ALS_MANUAL_INTEGRATION_CASE").is_ok_and(|case| case == "footprint");
        let status = DecalRenderStatus::default();
        app.insert_resource(status.clone());
        if probe_enabled {
            app.world_mut()
                .resource_mut::<PackedDecalBuffers>()
                .probe_enabled = true;
            app.init_resource::<DecalProbeRequest>().add_systems(
                PostUpdate,
                prepare_probe_request
                    .after(PackedDecalPrepareSet)
                    .before(bevy::asset::AssetEventSystems),
            );
            app.add_systems(
                Startup,
                |mut commands: Commands, buffers: Res<PackedDecalBuffers>| {
                    commands
                        .spawn(bevy::render::gpu_readback::Readback::buffer(
                            buffers.probe.clone(),
                        ))
                        .observe(read_material_probe);
                },
            );
        }
        if let Some(render) = app.get_sub_app_mut(bevy::render::RenderApp) {
            render.insert_resource(status).add_systems(
                bevy::render::Render,
                capture_decal_pipelines.in_set(bevy::render::RenderSystems::Cleanup),
            );
        }
        if let Some(directory) =
            std::env::var_os("ALS_ANIMATION_VISUAL_EVIDENCE_DIR").map(std::path::PathBuf::from)
        {
            app.add_systems(
                Last,
                move |mut exit: MessageReader<AppExit>,
                      status: Res<DecalRenderStatus>,
                      receivers: Query<
                    Entity,
                    (
                        With<MeshMaterial3d<StandardMaterial>>,
                        With<NoDecalReceiver>,
                    ),
                >,
                      markers: Query<Entity, With<NoDecalReceiver>>| {
                    if exit.read().next().is_none() {
                        return;
                    }
                    let mut report = serde_json::to_value(&*status.0.lock().unwrap()).unwrap();
                    report["receiver_entities"] = receivers.iter().count().into();
                    report["receiver_markers"] = markers.iter().count().into();
                    std::fs::write(
                        directory.join("decal_render.json"),
                        serde_json::to_vec(&report).unwrap(),
                    )
                    .expect("Decal render report must be writable");
                },
            );
        }
    }
}

fn probe_words(enabled: bool, request: &DecalProbeRequest) -> Vec<u32> {
    let mut words = vec![0; DECAL_PROBE_WORDS];
    words[0] = u32::from(enabled);
    words[2] = request.id;
    words[3] = request.index.unwrap_or(u32::MAX);
    words[4] = request.receiver;
    for (word, value) in words[8..12].iter_mut().zip(request.plane.to_array()) {
        *word = value.to_bits();
    }
    words
}

fn prepare_probe_request(
    request: Res<DecalProbeRequest>,
    handles: Res<PackedDecalBuffers>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
) {
    if request.is_changed() {
        let mut probe = buffers.get_mut(&handles.probe).unwrap();
        probe.clear();
        probe.extend_from_slice(&probe_words(true, &request));
    }
}

fn material_probe_error(words: &[u32]) -> Option<(usize, f32)> {
    if words.len() != DECAL_PROBE_WORDS || words[0] != 1 || words[1] == 0 {
        return None;
    }
    let count = (words[1] as usize).min(DECAL_PROBE_SAMPLES);
    let mut error = 0.0_f32;
    for sample in words[12..].chunks_exact(40).take(count) {
        let value: Vec<_> = sample.iter().map(|word| f32::from_bits(*word)).collect();
        if value.iter().any(|value| !value.is_finite()) {
            return Some((count, f32::MAX));
        }
        let alpha = (value[11] * value[15]).clamp(0.0, 1.0);
        let blend = |before: f32, after: f32| before * (1.0 - alpha) + after * alpha;
        let mut expected = [0.0; 8];
        for channel in 0..3 {
            expected[channel] = blend(value[channel], value[12 + channel]);
        }
        expected[3] = blend(value[3], value[9] * value[16]);
        expected[4] = blend(value[4], value[17]);
        for channel in 0..3 {
            expected[5 + channel] = blend(value[5 + channel].powi(2), value[18] * 0.5).sqrt();
        }
        for (actual, expected) in value[20..28].iter().zip(expected) {
            error = error.max((actual - expected).abs());
        }
        if value[28] != value[19] + 1.0 {
            error = f32::MAX;
        }
        if words[3] == u32::MAX {
            if value[29] != -1.0 || value[31] != 0.0 {
                error = f32::MAX;
            }
        } else {
            if value[29] != words[3] as f32 || value[30] <= value[29] || alpha <= 0.0001 {
                error = f32::MAX;
            }
            if (value[32] - value[34])
                .abs()
                .max((value[33] - value[35]).abs())
                > 1e-4
            {
                error = f32::MAX;
            }
        }
    }
    Some((count, error))
}

fn read_material_probe(
    event: On<bevy::render::gpu_readback::ReadbackComplete>,
    status: Res<DecalRenderStatus>,
) {
    let words: Vec<u32> = event.to_shader_type();
    if words.len() != DECAL_PROBE_WORDS || words[0] == 0 {
        return;
    }
    let mut status = status.0.lock().unwrap();
    if words[5] > 0 && status.probe_errors.is_empty() {
        status.probe_errors.push(
            "Packed decal data is stale, out of bounds, or differs from the native projection"
                .into(),
        );
    }
    status.positive_frame = status.positive_frame.max(words[7]);
    status.checked_frame = status.checked_frame.max(words[6]);
    let Some((count, error)) = material_probe_error(&words) else {
        return;
    };
    status.material_response_samples = count;
    status.material_response_max_error = status.material_response_max_error.max(error);
    if words[2] == 0 {
        return;
    }
    let result = DecalProbeResult {
        id: words[2],
        receiver: words[4],
        index: (words[3] != u32::MAX).then_some(words[3]),
        count,
        max_error: error,
        samples: words[12..]
            .chunks_exact(40)
            .take(count)
            .map(|sample| sample.iter().map(|value| f32::from_bits(*value)).collect())
            .collect(),
        plane: std::array::from_fn(|index| f32::from_bits(words[8 + index])),
    };
    if let Some(existing) = status.probes.iter_mut().find(|probe| probe.id == result.id) {
        if existing.count < DECAL_PROBE_SAMPLES {
            *existing = result;
        }
    } else {
        status.probes.push(result);
    }
}

fn capture_decal_pipelines(
    cache: Res<bevy::render::render_resource::PipelineCache>,
    status: Res<DecalRenderStatus>,
    device: Res<bevy::render::renderer::RenderDevice>,
    adapter: Res<bevy::render::renderer::RenderAdapter>,
    decals: Res<bevy::pbr::decal::clustered::RenderClusteredDecals>,
    buffers: Res<PackedDecalBuffers>,
    gpu_buffers: Res<
        bevy::render::render_asset::RenderAssets<bevy::render::storage::GpuShaderBuffer>,
    >,
    material_bindings: Res<bevy::render::material_bind_groups::RenderMaterialBindings>,
    allocators: Res<bevy::render::material_bind_groups::MaterialBindGroupAllocators>,
    frame: Res<bevy::diagnostic::FrameCount>,
) {
    use bevy::render::render_resource::{CachedPipelineState, PipelineDescriptor};
    let mut current = status.0.lock().unwrap();
    current.clustered_supported =
        bevy::pbr::decal::clustered::clustered_decals_are_usable(&device, &adapter);
    current.native_decals = decals.len();
    current.receiver_ready = false;
    current.errors.clear();
    current.pipeline_labels.clear();
    for pipeline in cache.pipelines() {
        let PipelineDescriptor::RenderPipelineDescriptor(descriptor) = &pipeline.descriptor else {
            continue;
        };
        let label = descriptor.label.as_deref().unwrap_or_default();
        if !label.starts_with("als_packed_receiver")
            && !label.starts_with("als_playground_receiver")
        {
            continue;
        }
        current.pipeline_labels.push(label.to_owned());
        match &pipeline.state {
            CachedPipelineState::Ok(_) => {
                current.receiver_ready |= label.ends_with("opaque_mesh_pipeline")
                    || label.ends_with("alpha_to_coverage_mesh_pipeline")
            }
            CachedPipelineState::Err(error) => current.errors.push(format!("{label}: {error}")),
            _ => {}
        }
    }
    if buffers.probe_enabled
        && let Some(gpu) = gpu_buffers.get(&buffers.materials)
    {
        let mut groups = Vec::new();
        for (kind, name) in [
            (
                std::any::TypeId::of::<PackedDecalReceiverMaterial>(),
                "standard",
            ),
            (
                std::any::TypeId::of::<crate::playground_materials::PlaygroundTilesMaterial>(),
                "playground",
            ),
        ] {
            if let Some(allocator) = allocators.get(&kind) {
                for (asset, binding) in material_bindings
                    .iter()
                    .filter(|(asset, _)| asset.type_id() == kind)
                {
                    if let Some(group) = allocator
                        .get(binding.group)
                        .and_then(|slab| slab.bind_group())
                    {
                        groups.push(format!("{name}:{asset:?}:{:?}", group.id()));
                    }
                }
            }
        }
        groups.sort_unstable();
        let snapshot = DecalBindingSnapshot {
            frame: frame.0,
            logical_buffer: format!("{:?}", buffers.materials.id()),
            gpu_buffer: format!("{:?}", gpu.buffer.id()),
            bytes: gpu.buffer.size(),
            receiver_bind_groups: groups,
        };
        if current.bindings.last().is_none_or(|last| {
            last.gpu_buffer != snapshot.gpu_buffer
                || last.receiver_bind_groups != snapshot.receiver_bind_groups
        }) {
            current.bindings.push(snapshot);
        }
    }
    current.decal_ready = current.clustered_supported
        && current.receiver_ready
        && current.positive_frame > 0
        && current.material_response_samples == DECAL_PROBE_SAMPLES
        && current.material_response_max_error <= 1e-5
        && current.probe_errors.is_empty();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projected_decal::{
        PACKED_DECAL_TAG, PackedDecal, PackedDecalMaterial, PackedDecalPlugin,
        install_decal_receiver_bindings,
    };
    use bevy::light::ClusteredDecal;

    #[test]
    fn receiver_dependencies_register_in_the_render_schedule() {
        let mut app = App::new();
        app.insert_sub_app(bevy::render::RenderApp, bevy::app::SubApp::new());
        install_decal_receiver_bindings::<PackedDecalReceiverMaterial>(&mut app);
        install_decal_receiver_bindings::<crate::playground_materials::PlaygroundTilesMaterial>(
            &mut app,
        );
        let render = app.sub_app_mut(bevy::render::RenderApp);
        render.world_mut().run_schedule(bevy::render::Render);
    }

    #[test]
    fn buffer_growth_refreshes_all_receivers_and_removal_preserves_capacity() {
        use crate::playground_materials::{
            PlaygroundMaterialsPlugin, PlaygroundTilesExtension, PlaygroundTilesMaterial,
            PlaygroundTilesSettings,
        };
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<Mesh>()
            .init_asset::<Image>()
            .init_asset::<StandardMaterial>()
            .add_plugins((PackedDecalPlugin, PlaygroundMaterialsPlugin));
        let handles = app.world().resource::<PackedDecalBuffers>().clone();
        let base = app
            .world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial::default());
        let receiver = app
            .world_mut()
            .spawn((Mesh3d::default(), MeshMaterial3d(base)))
            .id();
        let tile = app
            .world_mut()
            .resource_mut::<Assets<PlaygroundTilesMaterial>>()
            .add(PlaygroundTilesMaterial {
                base: default(),
                extension: PlaygroundTilesExtension {
                    decals: handles.materials.clone(),
                    probe: handles.probe.clone(),
                    grid_texture: default(),
                    settings: PlaygroundTilesSettings {
                        side_color_a: Vec4::ONE,
                        top_color_a: Vec4::ONE,
                        side_color_b: Vec4::ONE,
                        top_color_b: Vec4::ONE,
                        tiling_and_mode: Vec4::ONE,
                    },
                },
            });
        let material = app
            .world_mut()
            .resource_mut::<Assets<PackedDecalMaterial>>()
            .add(PackedDecalMaterial {
                color: Vec4::ONE,
                roughness_scale: 0.2,
                metallic: 0.0,
                specular: 0.5,
                texture: default(),
            });
        app.update();
        let receiver = app
            .world()
            .get::<MeshMaterial3d<PackedDecalReceiverMaterial>>(receiver)
            .unwrap()
            .0
            .clone();
        let mut standard_events = app
            .world()
            .resource::<Messages<AssetEvent<PackedDecalReceiverMaterial>>>()
            .get_cursor_current();
        let mut tile_events = app
            .world()
            .resource::<Messages<AssetEvent<PlaygroundTilesMaterial>>>()
            .get_cursor_current();
        let mut alive = Vec::new();
        let element_size = app
            .world()
            .resource::<Assets<ShaderBuffer>>()
            .get(&handles.materials)
            .unwrap()
            .buffer_size();
        let mut previous_size = element_size;
        let mut failures = Vec::new();
        for count in [1, 2, 3, 2, 1, 0, 17, 33, 257, 0] {
            while alive.len() < count {
                alive.push(
                    app.world_mut()
                        .spawn((PackedDecal(material.clone()), GlobalTransform::IDENTITY))
                        .id(),
                );
            }
            while alive.len() > count {
                app.world_mut().despawn(alive.pop().unwrap());
            }
            app.update();
            assert_eq!(
                app.world().resource::<PackedDecalBuffers>().materials,
                handles.materials
            );
            let size = app
                .world()
                .resource::<Assets<ShaderBuffer>>()
                .get(&handles.materials)
                .unwrap()
                .buffer_size();
            assert!(
                size >= element_size * count.max(1) as u64,
                "Live decals were truncated"
            );
            let standard_changed = standard_events
                .read(
                    app.world()
                        .resource::<Messages<AssetEvent<PackedDecalReceiverMaterial>>>(),
                )
                .any(|event| matches!(event, AssetEvent::Modified { id } if *id == receiver.id()));
            let tile_changed = tile_events
                .read(
                    app.world()
                        .resource::<Messages<AssetEvent<PlaygroundTilesMaterial>>>(),
                )
                .any(|event| matches!(event, AssetEvent::Modified { id } if *id == tile.id()));
            if size < previous_size {
                failures.push(format!(
                    "Removing decals shrank the upload at count {count}"
                ));
            }
            if size > previous_size && !(standard_changed && tile_changed) {
                failures.push(format!("Growing the buffer retained receiver bindings at count {count}: standard={standard_changed}, tiles={tile_changed}"));
            }
            if size == previous_size && (standard_changed || tile_changed) {
                failures.push(format!(
                    "Stable buffer capacity rebuilt receiver bindings at count {count}"
                ));
            }
            previous_size = size;
            let mut indices: Vec<_> = alive
                .iter()
                .map(|entity| {
                    app.world().get::<ClusteredDecal>(*entity).unwrap().tag & !PACKED_DECAL_TAG
                })
                .collect();
            indices.sort_unstable();
            assert_eq!(indices, (0..count as u32).collect::<Vec<_>>());
        }
        assert!(failures.is_empty(), "{}", failures.join("; "));
    }

    #[test]
    fn gpu_material_probe_rejects_wrong_blend_values() {
        let mut words = probe_words(
            true,
            &DecalProbeRequest {
                id: 1,
                receiver: 1,
                index: Some(0),
                ..default()
            },
        );
        assert!(material_probe_error(&words).is_none());
        words[1] = 1;
        let values: [f32; 40] = [
            0.8,
            0.4,
            0.2,
            0.9,
            0.6,
            0.9,
            0.7,
            0.5,
            0.1,
            0.8,
            0.3,
            0.5,
            0.0,
            0.0,
            0.0,
            0.6,
            0.2,
            0.0,
            0.5,
            41.0,
            0.56,
            0.28,
            0.14,
            0.678,
            0.42,
            0.642_f32.sqrt(),
            0.418_f32.sqrt(),
            0.5,
            42.0,
            0.0,
            4.0,
            1.0,
            0.3,
            0.4,
            0.3,
            0.4,
            0.0,
            0.0,
            0.0,
            1.0,
        ];
        for (word, value) in words[12..].iter_mut().zip(values) {
            *word = value.to_bits();
        }
        assert!(material_probe_error(&words).unwrap().1 < 1e-6);
        for index in (20..28).chain([19, 28, 30, 32, 33]) {
            let mut invalid = words.clone();
            invalid[12 + index] = 0.0_f32.to_bits();
            assert!(material_probe_error(&invalid).unwrap().1 > 0.1);
        }
        let mut wrong_index = words.clone();
        wrong_index[3] = 1;
        assert!(material_probe_error(&wrong_index).unwrap().1 > 0.1);
    }

    #[test]
    fn gpu_readback_survives_completed_requests_and_reports_late_failures() {
        use bevy::render::gpu_readback::{Readback, ReadbackComplete};
        let mut app = App::new();
        app.init_resource::<DecalRenderStatus>()
            .add_observer(read_material_probe);
        let entity = app
            .world_mut()
            .spawn(Readback::buffer(Handle::default()))
            .id();
        for id in [1, 2] {
            let mut words = probe_words(
                true,
                &DecalProbeRequest {
                    id,
                    receiver: 1,
                    ..default()
                },
            );
            words[1] = DECAL_PROBE_SAMPLES as u32;
            for sample in words[12..].chunks_exact_mut(40) {
                sample[19] = 41.0_f32.to_bits();
                sample[28] = 42.0_f32.to_bits();
                sample[29] = (-1.0_f32).to_bits();
                sample[30] = 1.0_f32.to_bits();
            }
            app.world_mut().trigger(ReadbackComplete {
                entity,
                data: words.into_iter().flat_map(u32::to_le_bytes).collect(),
            });
            app.world_mut().flush();
            assert!(
                app.world().get::<Readback>(entity).is_some(),
                "Readback stopped after the first completed request"
            );
        }
        let mut words = probe_words(true, &DecalProbeRequest { id: 3, ..default() });
        words[5] = 1;
        app.world_mut().trigger(ReadbackComplete {
            entity,
            data: words.into_iter().flat_map(u32::to_le_bytes).collect(),
        });
        app.world_mut().flush();
        let status = app
            .world()
            .resource::<DecalRenderStatus>()
            .0
            .lock()
            .unwrap();
        assert_eq!(status.probes.len(), 2);
        assert_eq!(status.probe_errors.len(), 1);
    }

    #[test]
    fn packed_decals_modify_surface_properties_before_lighting() {
        let decal = include_str!("../../assets/ALSExtras/Environment/Materials/packed_decal.wesl");
        assert!(
            !decal.contains("apply_pbr_lighting("),
            "Decal still computes independent lighting before alpha blending"
        );
        let receiver =
            include_str!("../../assets/ALSExtras/Environment/Materials/playground_tiles.wesl");
        let blend = receiver
            .find("apply_packed_decals(&pbr_input,")
            .expect("Receiving material does not consume packed decals");
        let lighting = receiver.find("apply_pbr_lighting(pbr_input)").unwrap();
        assert!(
            blend < lighting,
            "Decal material properties were blended after lighting"
        );
    }
}
