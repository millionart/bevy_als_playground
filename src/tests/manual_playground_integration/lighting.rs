use crate::source_gaussian_bloom::{SourceBloomProbes, SourceGaussianBloom};
use bevy::app::AppExit;
use bevy::core_pipeline::{schedule::Core3d, tonemapping::tonemapping};
use bevy::post_process::auto_exposure::AutoExposure;
use bevy::prelude::*;
use bevy::render::gpu_readback::{Readback, ReadbackComplete};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::{BufferUsages, ShaderType, encase};
use bevy::render::renderer::{RenderContext, ViewQuery};
use bevy::render::storage::{GpuShaderBuffer, ShaderBuffer};
use bevy::render::view::{ViewUniform, ViewUniformOffset, ViewUniforms};
use bevy::render::{RenderApp, RenderStartup, init_gpu_resource};

pub struct LightingProbePlugin(pub Option<std::path::PathBuf>);

#[derive(Resource, Clone)]
struct ViewReadback(Handle<ShaderBuffer>);

#[derive(Resource)]
struct LightingSamples {
    path: std::path::PathBuf,
    samples: usize,
    minimum_ev: f32,
    maximum_ev: f32,
    last_ev: f32,
    physical_exposure: f32,
    invalid_samples: usize,
    environment_maximums: [Option<f32>; 2],
    display_lut_maximum: Option<f32>,
    bloom_probe_errors: [Option<f32>; 3],
}

fn exposure_passed(
    samples: usize,
    minimum: f32,
    maximum: f32,
    physical: f32,
    invalid: usize,
) -> bool {
    samples >= 2
        && invalid == 0
        && minimum.is_finite()
        && maximum.is_finite()
        && maximum.abs().max(minimum.abs()) > 1e-5
        && (physical - 1.0).abs() < 1e-5
}

impl Plugin for LightingProbePlugin {
    fn build(&self, app: &mut App) {
        let Some(path) = self.0.clone() else {
            return;
        };
        let handle =
            app.world_mut()
                .resource_mut::<Assets<ShaderBuffer>>()
                .add(ShaderBuffer::from(vec![
                    0u32;
                    ViewUniform::min_size().get()
                        as usize
                        / 4
                ]));
        let readback = ViewReadback(handle.clone());
        app.insert_resource(LightingSamples {
            path,
            samples: 0,
            minimum_ev: f32::INFINITY,
            maximum_ev: f32::NEG_INFINITY,
            last_ev: 0.0,
            physical_exposure: 0.0,
            invalid_samples: 0,
            environment_maximums: [None; 2],
            display_lut_maximum: None,
            bloom_probe_errors: [None; 3],
        })
        .add_systems(Last, save_lighting_evidence)
        .add_systems(
            PostUpdate,
            (prepare_environment_readback, prepare_bloom_probes),
        );
        app.world_mut().spawn(Readback::buffer(handle)).observe(
            |event: On<ReadbackComplete>, mut samples: ResMut<LightingSamples>| {
                if event.data.len() < ViewUniform::min_size().get() as usize {
                    return;
                }
                let view = encase::UniformBuffer::new(event.data.as_slice())
                    .create::<ViewUniform>()
                    .expect("Lighting readback must match the native view layout");
                if !view.color_grading.exposure.is_finite() || !view.exposure.is_finite() {
                    samples.invalid_samples += 1;
                    return;
                }
                samples.samples += 1;
                samples.minimum_ev = samples.minimum_ev.min(view.color_grading.exposure);
                samples.maximum_ev = samples.maximum_ev.max(view.color_grading.exposure);
                samples.last_ev = view.color_grading.exposure;
                samples.physical_exposure = view.exposure;
            },
        );
        let render = app.sub_app_mut(RenderApp);
        render
            .insert_resource(readback)
            .add_systems(
                RenderStartup,
                configure_view_copy.after(init_gpu_resource::<ViewUniforms>),
            )
            .add_systems(Core3d, copy_view_buffer.after(tonemapping));
    }
}

#[derive(Component)]
struct EnvironmentReadbackInstalled;

#[derive(Component)]
struct BloomProbesInstalled;

fn bloom_probe_error(bytes: &[u8], template: &Image, expected: &[(u32, u32, f32)]) -> Option<f32> {
    if bytes.len()
        != template.texture_descriptor.size.width as usize
            * template.texture_descriptor.size.height as usize
            * 8
    {
        return None;
    }
    let mut image = template.clone();
    image.data = Some(bytes.to_vec());
    let mut error = 0.0_f32;
    for &(x, y, expected) in expected {
        let pixel = image.get_color_at(x, y).ok()?.to_linear().to_f32_array();
        if pixel.iter().any(|value| !value.is_finite()) || pixel[3] != 1.0 {
            return None;
        }
        for value in &pixel[..3] {
            let difference = (value - expected).abs() / (4e-6 + expected.abs() * 0.005);
            if difference > 1.0 {
                warn!(
                    "Bloom response differs at ({x}, {y}): expected {expected}, actual {value}, normalized_error {difference}"
                );
            }
            error = error.max(difference);
        }
    }
    Some(error)
}

fn prepare_bloom_probes(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    cameras: Query<(Entity, &SourceGaussianBloom), Without<BloomProbesInstalled>>,
) {
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages};
    if cameras.is_empty() {
        return;
    }
    let source: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/render/bloom_kernels.json")).unwrap();
    let width = source["spatial"]["width"].as_u64().unwrap() as u32;
    let size = Extent3d {
        width,
        height: width,
        depth_or_array_layers: 1,
    };
    for (camera, _) in &cameras {
        let mut probes = Vec::new();
        for kind in 0..3 {
            let value = if kind == 0 { 1.0 } else { 110.0 };
            let mut pixel = Image::new_fill(
                Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                TextureDimension::D2,
                &[0; 8],
                TextureFormat::Rgba16Float,
                bevy::asset::RenderAssetUsages::all(),
            );
            pixel
                .set_color_at(0, 0, Color::linear_rgb(value, value, value))
                .unwrap();
            let mut input = Image::new_fill(
                size,
                TextureDimension::D2,
                if kind == 2 {
                    &[0, 0, 0, 0, 0, 0, 0, 60]
                } else {
                    pixel.data.as_ref().unwrap()
                },
                TextureFormat::Rgba16Float,
                bevy::asset::RenderAssetUsages::all(),
            );
            if kind == 2 {
                let rectangle: Vec<u32> = source["spatial"]["impulse_rectangle"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|value| value.as_u64().unwrap() as u32)
                    .collect();
                for y in rectangle[1]..rectangle[3] {
                    for x in rectangle[0]..rectangle[2] {
                        input
                            .set_color_at(x, y, Color::linear_rgb(value, value, value))
                            .unwrap();
                    }
                }
            }
            let mut output = Image::new_fill(
                size,
                TextureDimension::D2,
                &[0; 8],
                TextureFormat::Rgba16Float,
                bevy::asset::RenderAssetUsages::all(),
            );
            output.texture_descriptor.usage |=
                TextureUsages::COPY_SRC | TextureUsages::RENDER_ATTACHMENT;
            let template = output.clone();
            let input = images.add(input);
            let output = images.add(output);
            let expected: Vec<_> = source["spatial"]["samples"]
                .as_array()
                .unwrap()
                .iter()
                .map(|sample| {
                    (
                        sample[0].as_u64().unwrap() as u32,
                        sample[1].as_u64().unwrap() as u32,
                        if kind == 2 {
                            sample[2].as_f64().unwrap() as f32
                        } else {
                            value * (1.0 + source["uniform_output_gain"].as_f64().unwrap() as f32)
                        },
                    )
                })
                .collect();
            commands.spawn(Readback::texture(output.clone())).observe(
                move |event: On<ReadbackComplete>,
                      mut samples: ResMut<LightingSamples>,
                      mut commands: Commands| {
                    if let Some(error) = bloom_probe_error(&event.data, &template, &expected) {
                        samples.bloom_probe_errors[kind] = Some(error);
                        commands.entity(event.entity).try_despawn();
                        if samples.bloom_probe_errors.iter().all(Option::is_some) {
                            commands.entity(camera).remove::<SourceBloomProbes>();
                        }
                    }
                },
            );
            probes.push((input, output));
        }
        commands
            .entity(camera)
            .insert((SourceBloomProbes(probes), BloomProbesInstalled));
    }
}

fn filtered_environment_maximum(bytes: &[u8], template: &Image) -> Option<f32> {
    let size = template.texture_descriptor.size;
    let row = size.width as usize * 8;
    let aligned_row = row.div_ceil(256) * 256;
    let rows = size.height as usize * size.depth_or_array_layers as usize;
    if bytes.len() < aligned_row * rows {
        return None;
    }
    let data: Vec<u8> = bytes
        .chunks_exact(aligned_row)
        .take(rows)
        .flat_map(|bytes| bytes[..row].iter().copied())
        .collect();
    if template
        .data
        .as_ref()
        .is_some_and(|expected| expected.len() >= data.len() && expected[..data.len()] != data)
    {
        return None;
    }
    let mut image = template.clone();
    image.data = Some(data);
    let mut maximum = 0.0_f32;
    for layer in 0..size.depth_or_array_layers {
        for y in 0..size.height {
            for x in 0..size.width {
                let pixel = image
                    .get_color_at_3d(x, y, layer)
                    .ok()?
                    .to_linear()
                    .to_f32_array();
                if pixel.iter().any(|channel| !channel.is_finite()) || pixel[3] != 1.0 {
                    return None;
                }
                maximum = maximum.max(pixel[0]).max(pixel[1]).max(pixel[2]);
            }
        }
    }
    (maximum > 0.0).then_some(maximum)
}

fn prepare_environment_readback(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    luts: Option<Res<bevy::core_pipeline::tonemapping::TonemappingLuts>>,
    environments: Query<
        (
            Entity,
            &bevy::light::EnvironmentMapLight,
            Option<&bevy::core_pipeline::tonemapping::Tonemapping>,
        ),
        (With<Camera3d>, Without<EnvironmentReadbackInstalled>),
    >,
) {
    for (entity, environment, tone) in &environments {
        let mut handles = vec![
            environment.diffuse_map.clone(),
            environment.specular_map.clone(),
        ];
        if matches!(
            tone,
            Some(bevy::core_pipeline::tonemapping::Tonemapping::TonyMcMapface)
        ) {
            if let Some(luts) = &luts {
                handles.push(luts.tony_mc_mapface.clone());
            }
        }
        for (kind, handle) in handles.into_iter().enumerate() {
            let mut image = images
                .get_mut(&handle)
                .expect("Native environment output must exist before extraction");
            image.texture_descriptor.usage |=
                bevy::render::render_resource::TextureUsages::COPY_SRC;
            let template = image.clone();
            commands.spawn(Readback::texture(handle)).observe(
                move |event: On<ReadbackComplete>,
                      mut samples: ResMut<LightingSamples>,
                      mut commands: Commands| {
                    if let Some(maximum) = filtered_environment_maximum(&event.data, &template) {
                        if kind == 2 {
                            samples.display_lut_maximum = Some(maximum);
                        } else {
                            samples.environment_maximums[kind] = Some(maximum);
                        }
                        commands.entity(event.entity).try_despawn();
                    }
                },
            );
        }
        commands.entity(entity).insert(EnvironmentReadbackInstalled);
    }
}

fn configure_view_copy(mut uniforms: ResMut<ViewUniforms>) {
    uniforms.uniforms.add_usages(BufferUsages::COPY_SRC);
}

fn copy_view_buffer(
    view: ViewQuery<(&ViewUniformOffset, &AutoExposure)>,
    uniforms: Res<ViewUniforms>,
    handle: Res<ViewReadback>,
    buffers: Res<RenderAssets<GpuShaderBuffer>>,
    mut context: RenderContext,
) {
    let Some(source) = uniforms.uniforms.buffer() else {
        return;
    };
    let Some(destination) = buffers.get(handle.0.id()) else {
        return;
    };
    let (offset, _) = view.into_inner();
    context.command_encoder().copy_buffer_to_buffer(
        source,
        u64::from(offset.offset),
        &destination.buffer,
        0,
        ViewUniform::min_size().get(),
    );
}

fn save_lighting_evidence(
    mut exits: MessageReader<AppExit>,
    samples: Res<LightingSamples>,
    diagnostics: Option<Res<bevy::diagnostic::DiagnosticsStore>>,
    environments: Query<
        (
            &bevy::light::EnvironmentMapLight,
            Has<bevy::light::GeneratedEnvironmentMapLight>,
        ),
        With<Camera3d>,
    >,
    ambient: Option<Res<GlobalAmbientLight>>,
) {
    if exits.read().next().is_none() {
        return;
    }
    let exposure_passed = exposure_passed(
        samples.samples,
        samples.minimum_ev,
        samples.maximum_ev,
        samples.physical_exposure,
        samples.invalid_samples,
    );
    let rendered_passes: Vec<_> = diagnostics
        .iter()
        .flat_map(|store| store.iter())
        .filter(|diagnostic| {
            diagnostic.path().as_str().ends_with("/elapsed_gpu")
                && diagnostic
                    .value()
                    .is_some_and(|value| value.is_finite() && value > 0.0)
        })
        .map(|diagnostic| diagnostic.path().as_str())
        .collect();
    let environment_ready_count = environments
        .iter()
        .filter(|(environment, generated)| {
            !generated && environment.intensity == 2.0 && environment.rotation == Quat::IDENTITY
        })
        .count();
    let bloom_rendered = rendered_passes
        .iter()
        .any(|path| path.ends_with("/source_gaussian_bloom/elapsed_gpu"));
    let environment_rendered = samples.environment_maximums.iter().all(Option::is_some);
    let ambient_brightness = ambient.map(|ambient| ambient.brightness);
    let bloom_response_passed = samples
        .bloom_probe_errors
        .iter()
        .all(|error| error.is_some_and(|error| error <= 1.0));
    let passed = exposure_passed
        && bloom_rendered
        && environment_rendered
        && samples.display_lut_maximum.is_some()
        && bloom_response_passed
        && environment_ready_count == 1
        && ambient_brightness == Some(0.0);
    let report = serde_json::json!({"format":"playground_lighting_evidence_v1", "pass":passed,
        "exposure_passed":exposure_passed,"bloom_rendered":bloom_rendered,"environment_rendered":environment_rendered,
        "bloom_response_passed":bloom_response_passed,"bloom_probe_normalized_errors":samples.bloom_probe_errors,
        "display_lut_maximum":samples.display_lut_maximum,"environment_maximums":samples.environment_maximums,"environment_ready_count":environment_ready_count,"ambient_brightness":ambient_brightness,
        "samples":samples.samples,"minimum_ev":samples.minimum_ev,"maximum_ev":samples.maximum_ev,
        "rendered_passes":rendered_passes,"last_ev":samples.last_ev,"physical_exposure":samples.physical_exposure,"invalid_samples":samples.invalid_samples,"images":[]});
    std::fs::write(&samples.path, serde_json::to_vec_pretty(&report).unwrap())
        .expect("Lighting evidence must be saved");
    info!(
        "[LIGHTING][VERDICT] samples={} min={} max={} last={} physical={} pass={passed}",
        samples.samples,
        samples.minimum_ev,
        samples.maximum_ev,
        samples.last_ev,
        samples.physical_exposure
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposure_readback_rejects_missing_fixed_and_non_finite_results() {
        assert!(exposure_passed(20, 0.0, 2.0, 1.0, 0));
        assert!(!exposure_passed(0, 0.0, 2.0, 1.0, 0));
        assert!(!exposure_passed(20, 0.0, 0.0, 1.0, 0));
        assert!(!exposure_passed(20, f32::NAN, 2.0, 1.0, 0));
        assert!(!exposure_passed(20, 0.0, 2.0, 0.20833333, 0));
        assert!(!exposure_passed(20, 0.0, 2.0, 1.0, 1));
    }

    #[test]
    fn environment_readback_rejects_pending_and_non_finite_pixels() {
        use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
        let size = Extent3d {
            width: 32,
            height: 32,
            depth_or_array_layers: 6,
        };
        let image = Image::new_fill(
            size,
            TextureDimension::D2,
            &[0, 60, 0, 0, 0, 0, 0, 60],
            TextureFormat::Rgba16Float,
            bevy::asset::RenderAssetUsages::all(),
        );
        assert_eq!(
            filtered_environment_maximum(image.data.as_ref().unwrap(), &image),
            Some(1.0)
        );
        assert_eq!(filtered_environment_maximum(&[], &image), None);
        assert_eq!(
            filtered_environment_maximum(&vec![0; 32 * 32 * 6 * 8], &image),
            None
        );
        let mut invalid = image.data.clone().unwrap();
        invalid[..2].copy_from_slice(&[0, 126]);
        assert_eq!(filtered_environment_maximum(&invalid, &image), None);
    }

    #[test]
    fn display_readback_removes_native_row_padding() {
        use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
        let image = Image::new_fill(
            Extent3d {
                width: 48,
                height: 48,
                depth_or_array_layers: 48,
            },
            TextureDimension::D3,
            &[0, 60, 0, 0, 0, 0, 0, 60],
            TextureFormat::Rgba16Float,
            bevy::asset::RenderAssetUsages::all(),
        );
        let mut padded = Vec::new();
        for row in image.data.as_ref().unwrap().chunks_exact(48 * 8) {
            padded.extend_from_slice(row);
            padded.extend_from_slice(&[255; 128]);
        }
        assert_eq!(filtered_environment_maximum(&padded, &image), Some(1.0));
        padded[0] = 1;
        assert_eq!(filtered_environment_maximum(&padded, &image), None);
    }

    #[test]
    fn bloom_response_rejects_pending_missing_halo_and_hdr_compression() {
        use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
        let mut image = Image::new_fill(
            Extent3d {
                width: 32,
                height: 32,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            &[0; 8],
            TextureFormat::Rgba16Float,
            bevy::asset::RenderAssetUsages::all(),
        );
        let expected = [(16, 16, 119.83936), (0, 0, 0.002)];
        assert_eq!(
            bloom_probe_error(image.data.as_ref().unwrap(), &image, &expected),
            None
        );
        for &(x, y, value) in &expected {
            image
                .set_color_at(x, y, Color::linear_rgb(value, value, value))
                .unwrap();
        }
        assert!(bloom_probe_error(image.data.as_ref().unwrap(), &image, &expected).unwrap() <= 1.0);
        image
            .set_color_at(0, 0, Color::linear_rgb(0.0, 0.0, 0.0))
            .unwrap();
        assert!(bloom_probe_error(image.data.as_ref().unwrap(), &image, &expected).unwrap() > 1.0);
        image
            .set_color_at(0, 0, Color::linear_rgb(0.002, 0.002, 0.002))
            .unwrap();
        image
            .set_color_at(16, 16, Color::linear_rgb(16.123, 16.123, 16.123))
            .unwrap();
        assert!(bloom_probe_error(image.data.as_ref().unwrap(), &image, &expected).unwrap() > 1.0);
    }

    #[test]
    fn bloom_probes_use_the_camera_settings_and_are_created_once() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<Assets<Image>>()
            .add_systems(PostUpdate, prepare_bloom_probes);
        let source: serde_json::Value = serde_json::from_str(include_str!(
            "../../../assets/ALSExtras/Environment/Sky/playground_display.runtime.json"
        ))
        .unwrap();
        let settings: SourceGaussianBloom =
            serde_json::from_value(source["bloom"].clone()).unwrap();
        let camera = app.world_mut().spawn(settings).id();
        app.update();
        assert_eq!(
            app.world()
                .get::<SourceBloomProbes>(camera)
                .unwrap()
                .0
                .len(),
            3
        );
        assert_eq!(app.world().resource::<Assets<Image>>().len(), 6);
        assert_eq!(
            app.world_mut()
                .query::<&Readback>()
                .iter(app.world())
                .count(),
            3
        );
        app.update();
        assert_eq!(app.world().resource::<Assets<Image>>().len(), 6);
        assert_eq!(
            app.world_mut()
                .query::<&Readback>()
                .iter(app.world())
                .count(),
            3
        );
    }

    #[test]
    fn environment_readback_is_installed_before_native_gpu_assets_once() {
        use bevy::render::render_resource::{
            Extent3d, TextureDimension, TextureFormat, TextureUsages,
        };
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<Assets<ShaderBuffer>>()
            .init_resource::<Assets<Image>>();
        app.insert_sub_app(RenderApp, bevy::app::SubApp::new());
        app.add_plugins(LightingProbePlugin(Some(
            std::env::temp_dir().join("als_environment_probe_unused.json"),
        )));
        app.add_systems(Update, bevy::pbr::generate::generate_environment_map_light);
        let image = Image::new_fill(
            Extent3d {
                width: 128,
                height: 128,
                depth_or_array_layers: 6,
            },
            TextureDimension::D2,
            &[0; 8],
            TextureFormat::Rgba16Float,
            bevy::asset::RenderAssetUsages::all(),
        );
        let source = app.world_mut().resource_mut::<Assets<Image>>().add(image);
        let camera = app
            .world_mut()
            .spawn((
                Camera3d::default(),
                bevy::light::GeneratedEnvironmentMapLight {
                    environment_map: source,
                    intensity: 2.0,
                    ..default()
                },
            ))
            .id();
        app.update();
        let maps = app
            .world()
            .get::<bevy::light::EnvironmentMapLight>(camera)
            .unwrap();
        let images = app.world().resource::<Assets<Image>>();
        for handle in [&maps.diffuse_map, &maps.specular_map] {
            assert!(
                images
                    .get(handle)
                    .unwrap()
                    .texture_descriptor
                    .usage
                    .contains(TextureUsages::COPY_SRC),
                "Readback must be configured before the reference GPU output is prepared"
            );
        }
        assert_eq!(
            app.world_mut()
                .query::<&Readback>()
                .iter(app.world())
                .count(),
            3
        );
        app.update();
        assert_eq!(
            app.world_mut()
                .query::<&Readback>()
                .iter(app.world())
                .count(),
            3,
            "Static reference outputs require only one diagnostic request each"
        );
    }

    #[test]
    fn lighting_saves_the_final_sample_on_post_update_exit() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("als_lighting_{}_{stamp}.json", std::process::id()));
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<Assets<ShaderBuffer>>()
            .init_resource::<Assets<Image>>();
        app.insert_sub_app(RenderApp, bevy::app::SubApp::new());
        app.add_plugins(LightingProbePlugin(Some(path.clone())));
        {
            let mut samples = app.world_mut().resource_mut::<LightingSamples>();
            samples.samples = 20;
            samples.minimum_ev = 0.0;
            samples.maximum_ev = 2.0;
            samples.last_ev = 2.0;
            samples.physical_exposure = 1.0;
        }
        app.add_systems(PostUpdate, |mut exits: MessageWriter<AppExit>| {
            exits.write(AppExit::Success);
        });
        app.update();
        let bytes = std::fs::read(&path).expect("Lighting evidence must include the exit frame");
        let report: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(report["pass"], false);
        assert_eq!(report["exposure_passed"], true);
        assert_eq!(report["environment_ready_count"], 0);
        assert_eq!(report["last_ev"], 2.0);
        std::fs::remove_file(path).unwrap();
    }
}
