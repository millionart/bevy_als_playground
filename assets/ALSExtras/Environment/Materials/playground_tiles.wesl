import super::packed_decal::apply_packed_decals;
import bevy_pbr::render::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::alpha_discard,
};

@if(PREPASS_PIPELINE)
import bevy_pbr::{
    prepass::io::{VertexOutput, FragmentOutput},
    deferred::functions::deferred_output,
};
@else
import bevy_pbr::render::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
};

struct PlaygroundTilesSettings {
    side_color_a: vec4<f32>,
    top_color_a: vec4<f32>,
    side_color_b: vec4<f32>,
    top_color_b: vec4<f32>,
    tiling_and_mode: vec4<f32>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(100)
var<uniform> playground_tiles: PlaygroundTilesSettings;
@group(constants::MATERIAL_BIND_GROUP) @binding(101)
var playground_grid_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(102)
var playground_grid_sampler: sampler;

@fragment
fn fragment(
    in: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);

    if (playground_tiles.tiling_and_mode.y < 0.5) {
        @if(VERTEX_UVS_A) {
            let uv = in.uv * playground_tiles.tiling_and_mode.x;
            let grid = textureSample(playground_grid_texture, playground_grid_sampler, uv * 2.0);
            let topness = abs(pbr_input.N.y);
            let color_a = mix(playground_tiles.side_color_a.rgb, playground_tiles.top_color_a.rgb, topness);
            let color_b = mix(playground_tiles.side_color_b.rgb, playground_tiles.top_color_b.rgb, topness);
            pbr_input.material.base_color = vec4(mix(color_a, color_b, grid.g) * grid.a, 1.0);
        }
    } else {
        pbr_input.material.base_color.a = clamp(in.world_normal.y * 0.5, 0.0, 1.0);
    }

    if (playground_tiles.tiling_and_mode.y < 0.5) { apply_packed_decals(&pbr_input, 2u); }

    pbr_input.material.base_color = alpha_discard(
        pbr_input.material.flags, pbr_input.material.alpha_cutoff, pbr_input.material.base_color
    );

    @if(PREPASS_PIPELINE)
    return deferred_output(in, pbr_input);
    @else {
        var out: FragmentOutput;
        out.color = apply_pbr_lighting(pbr_input);
        out.color = main_pass_post_lighting_processing(pbr_input, out.color);
        return out;
    }
}
