import bevy_pbr::render::{forward_io::{VertexOutput, FragmentOutput}, mesh_view_bindings::view};

struct PlaygroundSkySettings {
    zenith: vec4<f32>, horizon: vec4<f32>, cloud: vec4<f32>, overall: vec4<f32>,
    sun_color: vec4<f32>, light_direction: vec4<f32>, animation: vec4<f32>, shape: vec4<f32>,
    origin: vec4<f32>, constants: vec4<f32>, texture_settings: vec4<f32>, intensity: vec4<f32>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> sky: PlaygroundSkySettings;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var sky_clouds: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var sky_clouds_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(3) var sky_blue: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(4) var sky_blue_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(5) var sky_stars: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(6) var sky_stars_sampler: sampler;

fn source_power(base: f32, exponent: f32) -> f32 {
    return select(pow(max(base, 0.00000002980233), exponent), 0.0, base <= 0.00000002980233);
}

fn sky_color(camera_vector: vec3<f32>, position: vec3<f32>, uv: vec2<f32>, uv_b: vec2<f32>) -> vec3<f32> {
    let upness = clamp(-camera_vector.y, 0.0, 1.0);
    let horizon = clamp(source_power(1.0 - upness, sky.shape.x), 0.0, 1.0);
    let light = dot(camera_vector, normalize(sky.light_direction.xyz));
    let disc = clamp(1.0 - abs(light - 1.0) / max(0.00001, sky.shape.z), 0.0, 1.0);
    let glow = clamp(source_power(clamp(1.0 - abs(light - 1.0) / max(0.00001, sky.constants.z), 0.0, 1.0), sky.constants.w), 0.0, 1.0);
    let clock = sky.animation.w * sky.animation.x;
    let blue = textureSample(sky_blue, sky_blue_sampler, uv + vec2(clock * sky.texture_settings.x, 0.0));
    let clouds = textureSample(sky_clouds, sky_clouds_sampler, uv + vec2(clock * sky.texture_settings.y, 0.0));
    let noise = textureSample(sky_clouds, sky_clouds_sampler, uv * sky.texture_settings.z);
    let stars = textureSample(sky_stars, sky_stars_sampler, uv_b * sky.texture_settings.w);
    let height = 1.0 - clamp((position.y - sky.origin.y) / (sky.shape.w * sky.intensity.z), 0.0, 1.0);
    let cloud = source_power(mix(blue.r, clouds.r, upness) * height * sky.animation.y, mix(sky.constants.x, sky.constants.y, noise.r));
    let coverage = cloud * cloud;
    let clear = mix(sky.zenith.rgb + stars.rgb * sky.animation.z * sky.light_direction.w, sky.horizon.rgb, horizon) + disc * sky.sun_color.rgb * sky.shape.y;
    let cloudy = sky.cloud.rgb * cloud + glow * sky.sun_color.rgb * coverage * sky.intensity.y;
    return mix(clear, cloudy, clamp(coverage, 0.0, 1.0)) * sky.overall.rgb * sky.intensity.x;
}

@fragment
fn fragment(in: VertexOutput) -> FragmentOutput {
    var out: FragmentOutput;
    out.color = vec4(sky_color(normalize(view.world_position - in.world_position.xyz), in.world_position.xyz, in.uv, in.uv_b), 1.0);
    return out;
}
