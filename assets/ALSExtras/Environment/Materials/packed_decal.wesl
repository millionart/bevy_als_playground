import bevy_pbr::render::{pbr_types::PbrInput, mesh_view_bindings};

@if(CLUSTERED_DECALS_ARE_USABLE)
import bevy_pbr::{
    render::clustered_forward,
    decal::clustered::{clustered_decal_iterator_new, clustered_decal_iterator_next, get_view_z, view_is_orthographic},
};

struct PackedDecalData {
    local_from_world: mat4x4<f32>,
    color: vec4<f32>,
    material: vec4<f32>,
}

struct DecalMaterialProbe {
    enabled: u32,
    count: atomic<u32>,
    request: u32,
    target_index: u32,
    receiver: u32,
    errors: atomic<u32>,
    latest_frame: atomic<u32>,
    positive_frame: atomic<u32>,
    plane: vec4<f32>,
    values: array<vec4<f32>, 160>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(110)
var<storage, read> packed_decals: array<PackedDecalData>;
@group(constants::MATERIAL_BIND_GROUP) @binding(111)
var<storage, read_write> decal_probe: DecalMaterialProbe;

fn probe_pixel(position: vec2<f32>) -> bool {
    return (u32(position.x) & 7u) == 0u && (u32(position.y) & 7u) == 0u;
}

fn record_material_response(before_color_roughness: vec4<f32>, before_material: vec4<f32>, packed: vec4<f32>, color: vec4<f32>, material: vec4<f32>, after_color_roughness: vec4<f32>, after_material: vec4<f32>, index_value: u32, receiver: u32, uv: vec4<f32>, world: vec4<f32>, applied: u32) {
    if (decal_probe.enabled == 0u || atomicLoad(&decal_probe.count) >= 16u || decal_probe.receiver != receiver || decal_probe.target_index != index_value) { return; }
    if (decal_probe.plane.w > 0.0 && (abs(world.y - decal_probe.plane.x) > 0.0001 || distance(world.xz, decal_probe.plane.yz) > decal_probe.plane.w)) { return; }
    if (index_value != 0xffffffffu && packed.a * color.a <= 0.0001) { return; }
    let index = atomicAdd(&decal_probe.count, 1u);
    if (index >= 16u) { return; }
    let offset = index * 10u;
    decal_probe.values[offset] = before_color_roughness;
    decal_probe.values[offset + 1u] = before_material;
    decal_probe.values[offset + 2u] = packed;
    decal_probe.values[offset + 3u] = color;
    decal_probe.values[offset + 4u] = material;
    decal_probe.values[offset + 5u] = after_color_roughness;
    decal_probe.values[offset + 6u] = after_material;
    decal_probe.values[offset + 7u] = vec4(f32(mesh_view_bindings::globals.frame_count), select(f32(index_value), -1.0, index_value == 0xffffffffu), f32(arrayLength(&packed_decals)), f32(applied));
    decal_probe.values[offset + 8u] = uv;
    decal_probe.values[offset + 9u] = world;
}

fn apply_packed_decals(pbr_input: ptr<function, PbrInput>, receiver: u32) {
    @if(CLUSTERED_DECALS_ARE_USABLE) {
        let initial_color = vec4((*pbr_input).material.base_color.rgb, (*pbr_input).material.perceptual_roughness);
        let initial_material = vec4((*pbr_input).material.metallic, (*pbr_input).material.reflectance);
        let world = (*pbr_input).world_position;
        let world_dx = dpdx(world.xyz);
        let world_dy = dpdy(world.xyz);
        let cluster = clustered_forward::view_fragment_cluster_index((*pbr_input).frag_coord.xy, get_view_z(world.xyz), view_is_orthographic());
        var ranges = clustered_forward::unpack_clusterable_object_index_ranges(cluster);
        var cursor = 0u;
        var applied = 0u;
        let probe = decal_probe.enabled != 0u && probe_pixel((*pbr_input).frag_coord.xy);
        loop {
            var selected = 0xffffffffu;
            var texture_index = -1i;
            var uv = vec2(0.0);
            var iterator = clustered_decal_iterator_new(world.xyz, &ranges);
            while (clustered_decal_iterator_next(&iterator)) {
                let index = iterator.tag & 0x7fffffffu;
                if (probe && (iterator.tag & 0x80000000u) != 0u && iterator.metallic_roughness_texture_index >= 0 && index >= arrayLength(&packed_decals)) {
                    atomicAdd(&decal_probe.errors, 1u);
                }
                if ((iterator.tag & 0x80000000u) != 0u && index >= cursor && index < selected && index < arrayLength(&packed_decals) && iterator.metallic_roughness_texture_index >= 0) {
                    selected = index;
                    texture_index = iterator.metallic_roughness_texture_index;
                    uv = iterator.uv;
                }
            }
            if (selected == 0xffffffffu) { break; }
            cursor = selected + 1u;
            let decal = packed_decals[selected];
            let local = (decal.local_from_world * world).xyz;
            let packed_uv = local.xy * vec2(1.0, -1.0) + vec2(0.5);
            let current_data = decal.material.w + 1.0 == f32(mesh_view_bindings::globals.frame_count) && all(abs(uv - packed_uv) <= vec2(0.0001));
            if (probe) {
                atomicMax(&decal_probe.latest_frame, mesh_view_bindings::globals.frame_count);
                if (!current_data) {
                    atomicAdd(&decal_probe.errors, 1u);
                }
            }
            let uv_dx = (decal.local_from_world * vec4(world_dx, 0.0)).xy * vec2(1.0, -1.0);
            let uv_dy = (decal.local_from_world * vec4(world_dy, 0.0)).xy * vec2(1.0, -1.0);
            let packed = textureSampleGrad(mesh_view_bindings::clustered_decal_textures[texture_index], mesh_view_bindings::clustered_decal_sampler, uv, uv_dx, uv_dy);
            let color = vec4(decal.color.rgb, decal.color.a * clamp(4.0 - 8.0 * abs(local.z), 0.0, 1.0));
            let alpha = clamp(packed.a * color.a, 0.0, 1.0);
            let before_color_roughness = vec4((*pbr_input).material.base_color.rgb, (*pbr_input).material.perceptual_roughness);
            let before_material = vec4((*pbr_input).material.metallic, (*pbr_input).material.reflectance);
            (*pbr_input).material.base_color = vec4(mix(before_color_roughness.rgb, color.rgb, alpha), (*pbr_input).material.base_color.a);
            (*pbr_input).material.perceptual_roughness = mix(before_color_roughness.a, packed.g * decal.material.x, alpha);
            (*pbr_input).material.metallic = mix(before_material.x, decal.material.y, alpha);
            (*pbr_input).material.reflectance = sqrt(mix(before_material.yzw * before_material.yzw, vec3(decal.material.z * 0.5), alpha));
            applied += 1u;
            let after_color = vec4((*pbr_input).material.base_color.rgb, (*pbr_input).material.perceptual_roughness);
            let after_material = vec4((*pbr_input).material.metallic, (*pbr_input).material.reflectance);
            if (decal_probe.enabled != 0u && current_data && alpha > 0.0001 && (any(abs(after_color - before_color_roughness) > vec4(0.00001)) || any(abs(after_material - before_material) > vec4(0.00001)))) {
                if (atomicLoad(&decal_probe.positive_frame) != mesh_view_bindings::globals.frame_count) { atomicMax(&decal_probe.positive_frame, mesh_view_bindings::globals.frame_count); }
            }
            record_material_response(before_color_roughness, before_material, packed, color, decal.material,
                after_color, after_material, selected, receiver, vec4(uv, packed_uv), world, applied);
        }
        if (decal_probe.enabled != 0u && decal_probe.target_index == 0xffffffffu) {
            record_material_response(initial_color, initial_material, vec4(0.0), vec4(0.0), vec4(0.0, 0.0, 0.0, packed_decals[0].material.w),
                vec4((*pbr_input).material.base_color.rgb, (*pbr_input).material.perceptual_roughness),
                vec4((*pbr_input).material.metallic, (*pbr_input).material.reflectance), 0xffffffffu, receiver, vec4(0.0), world, applied);
        }
    }
}
