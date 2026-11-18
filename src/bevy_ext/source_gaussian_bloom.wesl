import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;

struct FilterSettings {
    mapping: vec4<f32>,
    tint: vec4<f32>,
    control: vec4<u32>,
    taps: array<vec4<f32>, 32>,
}

@group(0) @binding(0) var input_texture: texture_2d<f32>;
@group(0) @binding(1) var additive_texture: texture_2d<f32>;
@group(0) @binding(2) var input_sampler: sampler;
@group(0) @binding(3) var<uniform> settings: FilterSettings;

@fragment
fn fragment(input: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let uv = input.uv * settings.mapping.xy + settings.mapping.zw;
    if settings.control.x == 0u {
        let texel = 1.0 / vec2<f32>(textureDimensions(input_texture));
        let a = textureSampleLevel(input_texture, input_sampler, uv + texel * vec2(-1.0, -1.0), 0.0).rgb;
        let b = textureSampleLevel(input_texture, input_sampler, uv + texel * vec2(1.0, -1.0), 0.0).rgb;
        let c = textureSampleLevel(input_texture, input_sampler, uv + texel * vec2(-1.0, 1.0), 0.0).rgb;
        let d = textureSampleLevel(input_texture, input_sampler, uv + texel * vec2(1.0, 1.0), 0.0).rgb;
        return vec4(max((a + b + c + d) * 0.25, vec3(0.0)), 1.0);
    }
    if settings.control.x == 2u {
        let scene = textureSampleLevel(input_texture, input_sampler, uv, 0.0);
        let bloom = textureSampleLevel(additive_texture, input_sampler, input.uv, 0.0).rgb;
        return vec4(scene.rgb + bloom, scene.a);
    }
    var color = vec3(0.0);
    for (var index = 0u; index < settings.control.y; index += 1u) {
        let tap = settings.taps[index];
        color += textureSampleLevel(input_texture, input_sampler, uv + tap.xy, 0.0).rgb * tap.z;
    }
    color *= settings.tint.rgb;
    if settings.control.z != 0u {
        color += textureSampleLevel(additive_texture, input_sampler, input.uv, 0.0).rgb;
    }
    return vec4(color, 1.0);
}
