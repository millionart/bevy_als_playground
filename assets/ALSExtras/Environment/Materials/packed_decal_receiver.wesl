import bevy_pbr::{
    render::{pbr_fragment::pbr_input_from_standard_material, pbr_functions::alpha_discard},
    decal::clustered::apply_decals,
};
import super::packed_decal::apply_packed_decals;

@if(PREPASS_PIPELINE)
import bevy_pbr::{prepass::io::{VertexOutput, FragmentOutput}, deferred::functions::deferred_output};
@else
import bevy_pbr::render::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
    pbr_types::STANDARD_MATERIAL_FLAGS_UNLIT_BIT,
};

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    pbr_input.material.base_color = alpha_discard(
        pbr_input.material.flags, pbr_input.material.alpha_cutoff, pbr_input.material.base_color
    );
    apply_decals(&pbr_input);
    apply_packed_decals(&pbr_input, 1u);
    @if(PREPASS_PIPELINE)
    return deferred_output(in, pbr_input);
    @else {
        var out: FragmentOutput;
        if ((pbr_input.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u) {
            out.color = apply_pbr_lighting(pbr_input);
        } else { out.color = pbr_input.material.base_color; }
        out.color = main_pass_post_lighting_processing(pbr_input, out.color);
        return out;
    }
}
