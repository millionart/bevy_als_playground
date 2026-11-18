use bevy::asset::{embedded_asset, load_embedded_asset};
use bevy::core_pipeline::{
    FullscreenShader,
    schedule::{Core3d, Core3dSystems},
    tonemapping::tonemapping,
};
use bevy::prelude::*;
use bevy::render::camera::ExtractedCamera;
use bevy::render::diagnostic::RecordDiagnostics;
use bevy::render::extract_component::{ExtractComponent, ExtractComponentPlugin};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::{binding_types::*, *};
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery};
use bevy::render::texture::{CachedTexture, GpuImage, TextureCache};
use bevy::render::view::ViewTarget;
use bevy::render::{RenderApp, RenderStartup};
use serde::Deserialize;

#[derive(Component, Clone, Deserialize, ExtractComponent)]
#[extract_app(RenderApp)]
#[require(bevy::camera::Hdr)]
pub struct SourceGaussianBloom {
    pub intensity: f32,
    pub size_scale: f32,
    pub sizes: [f32; 6],
    pub tints: [Vec3; 6],
    pub filter_size_scale: f32,
    pub fast_blur_threshold: f32,
    pub kernel_exponent: f32,
    pub sample_count_max: u32,
}

#[derive(Component, Clone, ExtractComponent)]
#[extract_app(RenderApp)]
pub struct SourceBloomProbes(pub Vec<(Handle<Image>, Handle<Image>)>);

pub fn gaussian_kernel(radius: f32, settings: &SourceGaussianBloom) -> Vec<Vec2> {
    let sigma = radius.clamp(0.00001, settings.sample_count_max as f32 - 1.0);
    let integer = (radius * settings.filter_size_scale.clamp(0.1, 10.0))
        .clamp(0.00001, settings.sample_count_max as f32 - 1.0)
        .ceil() as i32;
    let mut taps = Vec::new();
    let mut sum = 0.0;
    for sample in (-integer..=integer).step_by(2) {
        let first = (settings.kernel_exponent * (sample as f32 / sigma).powi(2)).exp();
        let second = if sample == integer {
            0.0
        } else {
            (settings.kernel_exponent * ((sample + 1) as f32 / sigma).powi(2)).exp()
        };
        let weight = first + second;
        if weight > 0.0 {
            taps.push(Vec2::new(sample as f32 + second / weight, weight));
            sum += weight;
        }
    }
    for tap in &mut taps {
        tap.y /= sum;
    }
    taps
}

pub struct SourceGaussianBloomPlugin;

impl Plugin for SourceGaussianBloomPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "source_gaussian_bloom.wesl");
        app.add_plugins((
            ExtractComponentPlugin::<SourceGaussianBloom>::default(),
            ExtractComponentPlugin::<SourceBloomProbes>::default(),
        ));
        if let Some(render) = app.get_sub_app_mut(RenderApp) {
            render
                .add_systems(RenderStartup, initialize_pipeline)
                .add_systems(
                    Core3d,
                    render_bloom
                        .before(tonemapping)
                        .in_set(Core3dSystems::PostProcess),
                );
        }
    }
}

#[derive(Clone, ShaderType)]
struct FilterSettings {
    mapping: Vec4,
    tint: Vec4,
    control: UVec4,
    taps: [Vec4; 32],
}

impl Default for FilterSettings {
    fn default() -> Self {
        Self {
            mapping: Vec4::new(1.0, 1.0, 0.0, 0.0),
            tint: Vec4::ONE,
            control: UVec4::ZERO,
            taps: [Vec4::ZERO; 32],
        }
    }
}

#[derive(Resource)]
struct BloomPipeline {
    layout: BindGroupLayoutDescriptor,
    pipeline: CachedRenderPipelineId,
    sampler: Sampler,
}

fn initialize_pipeline(
    mut commands: Commands,
    device: Res<RenderDevice>,
    cache: Res<PipelineCache>,
    fullscreen: Res<FullscreenShader>,
    assets: Res<AssetServer>,
) {
    let layout = BindGroupLayoutDescriptor::new(
        "source_gaussian_bloom",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_2d(TextureSampleType::Float { filterable: true }),
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                uniform_buffer::<FilterSettings>(false),
            ),
        ),
    );
    let pipeline = cache.queue_render_pipeline(RenderPipelineDescriptor {
        label: Some("source_gaussian_bloom".into()),
        layout: vec![layout.clone()],
        vertex: fullscreen.to_vertex_state(),
        fragment: Some(FragmentState {
            shader: load_embedded_asset!(assets.as_ref(), "source_gaussian_bloom.wesl"),
            entry_point: Some("fragment".into()),
            targets: vec![Some(ColorTargetState {
                format: TextureFormat::Rgba16Float,
                blend: None,
                write_mask: ColorWrites::ALL,
            })],
            ..default()
        }),
        ..default()
    });
    commands.insert_resource(BloomPipeline {
        layout,
        pipeline,
        sampler: device.create_sampler(&SamplerDescriptor {
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            ..default()
        }),
    });
}

fn temporary_texture(
    size: UVec2,
    cache: &mut TextureCache,
    device: &RenderDevice,
) -> CachedTexture {
    cache.get(
        device,
        TextureDescriptor {
            label: Some("source_bloom_texture"),
            size: Extent3d {
                width: size.x,
                height: size.y,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba16Float,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        },
    )
}

fn filter_pass(
    context: &mut RenderContext,
    queue: &RenderQueue,
    pipeline: &BloomPipeline,
    cache: &PipelineCache,
    input: &TextureView,
    additive: &TextureView,
    destination: &TextureView,
    settings: FilterSettings,
    viewport: Option<(UVec2, UVec2)>,
) {
    let mut uniform = UniformBuffer::from(settings);
    uniform.write_buffer(context.render_device(), queue);
    let group = context.render_device().create_bind_group(
        "source_bloom_pass",
        &cache.get_bind_group_layout(&pipeline.layout),
        &BindGroupEntries::sequential((
            input,
            additive,
            &pipeline.sampler,
            uniform.binding().unwrap(),
        )),
    );
    let mut pass = context
        .command_encoder()
        .begin_render_pass(&RenderPassDescriptor {
            label: Some("source_bloom_pass"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: destination,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Load,
                    store: StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    pass.set_pipeline(cache.get_render_pipeline(pipeline.pipeline).unwrap());
    pass.set_bind_group(0, &group, &[]);
    if let Some((position, size)) = viewport {
        pass.set_viewport(
            position.x as f32,
            position.y as f32,
            size.x as f32,
            size.y as f32,
            0.0,
            1.0,
        );
    }
    pass.draw(0..3, 0..1);
}

fn render_bloom(
    view: ViewQuery<(
        &ViewTarget,
        &ExtractedCamera,
        &SourceGaussianBloom,
        Option<&SourceBloomProbes>,
    )>,
    pipeline: Res<BloomPipeline>,
    cache: Res<PipelineCache>,
    mut textures: ResMut<TextureCache>,
    queue: Res<RenderQueue>,
    images: Res<RenderAssets<GpuImage>>,
    mut context: RenderContext,
) {
    let (target, camera, settings, probes) = view.into_inner();
    let Some(view_size) = camera.physical_viewport_size else {
        return;
    };
    if view_size.min_element() == 0
        || settings.intensity == 0.0
        || cache.get_render_pipeline(pipeline.pipeline).is_none()
    {
        return;
    }
    let target_size = camera.physical_target_size.unwrap_or(view_size);
    let origin = camera
        .viewport
        .as_ref()
        .map_or(UVec2::ZERO, |viewport| viewport.physical_position);
    let mapping = Vec4::new(
        view_size.x as f32 / target_size.x as f32,
        view_size.y as f32 / target_size.y as f32,
        origin.x as f32 / target_size.x as f32,
        origin.y as f32 / target_size.y as f32,
    );
    let diagnostics = context.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    let span = diagnostics.time_span(context.command_encoder(), "source_gaussian_bloom");
    let output = target.post_process_write();
    bloom_chain(
        &mut context,
        &queue,
        &pipeline,
        &cache,
        &mut textures,
        settings,
        output.source,
        output.destination,
        view_size,
        mapping,
        Some((origin, view_size)),
    );
    if let Some(probes) = probes {
        for (input, output) in &probes.0 {
            if let (Some(input), Some(output)) = (images.get(input), images.get(output)) {
                bloom_chain(
                    &mut context,
                    &queue,
                    &pipeline,
                    &cache,
                    &mut textures,
                    settings,
                    &input.texture_view,
                    &output.texture_view,
                    UVec2::new(
                        input.texture_descriptor.size.width,
                        input.texture_descriptor.size.height,
                    ),
                    FilterSettings::default().mapping,
                    None,
                );
            }
        }
    }
    span.end(context.command_encoder());
}

fn bloom_chain(
    context: &mut RenderContext,
    queue: &RenderQueue,
    pipeline: &BloomPipeline,
    cache: &PipelineCache,
    textures: &mut TextureCache,
    settings: &SourceGaussianBloom,
    scene: &TextureView,
    destination: &TextureView,
    view_size: UVec2,
    mapping: Vec4,
    viewport: Option<(UVec2, UVec2)>,
) {
    let mut levels: Vec<(UVec2, CachedTexture)> = Vec::new();
    let mut size = view_size;
    for level in 0..6 {
        size = UVec2::new(size.x.div_ceil(2), size.y.div_ceil(2)).max(UVec2::ONE);
        let texture = temporary_texture(size, textures, context.render_device());
        let input = levels
            .last()
            .map_or(scene, |(_, previous)| &previous.default_view);
        filter_pass(
            context,
            queue,
            pipeline,
            cache,
            input,
            input,
            &texture.default_view,
            FilterSettings {
                mapping: if level == 0 {
                    mapping
                } else {
                    FilterSettings::default().mapping
                },
                ..default()
            },
            None,
        );
        levels.push((size, texture));
    }
    let mut combined: Option<CachedTexture> = None;
    for level in (0..6).rev() {
        let (size, input) = &levels[level];
        let radius = size.x as f32 * settings.sizes[level] * settings.size_scale * 0.005;
        let taps = gaussian_kernel(radius, settings);
        let horizontal_size = UVec2::new(
            if radius >= settings.fast_blur_threshold {
                size.x.div_ceil(2)
            } else {
                size.x
            },
            size.y,
        );
        let horizontal = temporary_texture(horizontal_size, textures, context.render_device());
        let mut horizontal_settings = FilterSettings {
            control: UVec4::new(1, taps.len() as u32, 0, 0),
            ..default()
        };
        for (output, tap) in horizontal_settings.taps.iter_mut().zip(&taps) {
            *output = Vec4::new(tap.x / size.x as f32, 0.0, tap.y, 0.0);
        }
        filter_pass(
            context,
            queue,
            pipeline,
            cache,
            &input.default_view,
            &input.default_view,
            &horizontal.default_view,
            horizontal_settings,
            None,
        );
        let vertical = temporary_texture(*size, textures, context.render_device());
        let mut vertical_settings = FilterSettings {
            tint: (settings.tints[level] * (settings.intensity / 6.0)).extend(1.0),
            control: UVec4::new(1, taps.len() as u32, u32::from(combined.is_some()), 0),
            ..default()
        };
        for (output, tap) in vertical_settings.taps.iter_mut().zip(&taps) {
            *output = Vec4::new(0.0, tap.x / size.y as f32, tap.y, 0.0);
        }
        let additive = combined
            .as_ref()
            .map_or(&horizontal.default_view, |texture| &texture.default_view);
        filter_pass(
            context,
            queue,
            pipeline,
            cache,
            &horizontal.default_view,
            additive,
            &vertical.default_view,
            vertical_settings,
            None,
        );
        combined = Some(vertical);
    }
    filter_pass(
        context,
        queue,
        pipeline,
        cache,
        scene,
        &combined.unwrap().default_view,
        destination,
        FilterSettings {
            mapping,
            control: UVec4::new(2, 0, 0, 0),
            ..default()
        },
        viewport,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gaussian_filters_preserve_source_shape_gain_and_resolution() {
        let data: serde_json::Value = serde_json::from_str(include_str!(
            "../../assets/ALSExtras/Environment/Sky/playground_display.runtime.json"
        ))
        .unwrap();
        let settings: SourceGaussianBloom = serde_json::from_value(data["bloom"].clone()).unwrap();
        let source: serde_json::Value = serde_json::from_str(include_str!(
            "../tests/fixtures/render/bloom_kernels.json"
        ))
        .unwrap();
        for row in source["kernels"].as_array().unwrap() {
            let stage = row["stage"].as_u64().unwrap() as usize;
            let mut width = row["width"].as_u64().unwrap() as u32;
            for _ in 0..=stage {
                width = width.div_ceil(2);
            }
            let radius = width as f32 * settings.sizes[stage] * settings.size_scale * 0.005;
            assert!((f64::from(radius) - row["radius"].as_f64().unwrap()).abs() < 1e-5);
            let taps = gaussian_kernel(radius, &settings);
            let expected = row["taps"].as_array().unwrap();
            assert_eq!(taps.len(), expected.len());
            for (tap, expected) in taps.iter().zip(expected) {
                assert!((f64::from(tap.x) - expected[0].as_f64().unwrap()).abs() < 4e-6);
                assert!((f64::from(tap.y) - expected[1].as_f64().unwrap()).abs() < 2e-7);
            }
            let horizontal = if radius >= settings.fast_blur_threshold {
                width.div_ceil(2)
            } else {
                width
            };
            assert_eq!(horizontal as u64, row["horizontal_width"].as_u64().unwrap());
            assert!((taps.iter().map(|tap| tap.y).sum::<f32>() - 1.0).abs() < 1e-6);
        }
        let gain: f32 = settings
            .tints
            .iter()
            .map(|tint| tint.x * settings.intensity / 6.0)
            .sum();
        for input in [1.0, 110.0] {
            assert!(
                (input * gain - input * source["uniform_output_gain"].as_f64().unwrap() as f32)
                    .abs()
                    < 2e-6
            );
        }
        let coarse_gain = settings.tints[5].x * settings.intensity / 6.0;
        assert!((coarse_gain - 0.0068625).abs() < 1e-8);
    }
}
