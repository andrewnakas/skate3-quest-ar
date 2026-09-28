// StandardMaterial entry point; the added block only composes skin-stamp albedo.
// Bevy 0.18.1 pbr.wgsl, MIT/Apache-2.0, copyright Bevy contributors.
struct SkinStamp {
    transform: vec4<f32>,
    rectangle: vec4<f32>,
    enabled: vec4<f32>,
}
@group(#{MATERIAL_BIND_GROUP}) @binding(100) var stamp_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var stamp_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(103) var hair_opacity: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(104) var hair_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var<uniform> stamp: SkinStamp;
#import bevy_pbr::{
    pbr_types,
    pbr_functions::alpha_discard,
    pbr_fragment::pbr_input_from_standard_material,
    decal::clustered::apply_decals,
}

#ifdef PREPASS_PIPELINE
#import bevy_pbr::{
    prepass_io::{VertexOutput, FragmentOutput},
    pbr_deferred_functions::deferred_output,
}
#else
#import skate_character_lighting::{CharacterParams, shade_character, character_normal, character_albedo}
#import bevy_pbr::pbr_bindings
@group(#{MATERIAL_BIND_GROUP}) @binding(105) var retail_mask: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(106) var retail_mask_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(107) var<uniform> retail: CharacterParams;
#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
    pbr_types::STANDARD_MATERIAL_FLAGS_UNLIT_BIT,
}
#endif

#ifdef VISIBILITY_RANGE_DITHER
#import bevy_pbr::pbr_functions::visibility_range_dither;
#endif

#ifdef MESHLET_MESH_MATERIAL_PASS
#import bevy_pbr::meshlet_visibility_buffer_resolve::resolve_vertex_output
#endif

#ifdef OIT_ENABLED
#import bevy_core_pipeline::oit::oit_draw
#endif // OIT_ENABLED

#ifdef FORWARD_DECAL
#import bevy_pbr::decal::forward::get_forward_decal_info
#endif

@fragment
fn fragment(
#ifdef MESHLET_MESH_MATERIAL_PASS
    @builtin(position) frag_coord: vec4<f32>,
#else
    vertex_output: VertexOutput,
    @builtin(front_facing) is_front: bool,
#endif
) -> FragmentOutput {
#ifdef MESHLET_MESH_MATERIAL_PASS
    let vertex_output = resolve_vertex_output(frag_coord);
    let is_front = true;
#endif

    var in = vertex_output;

    // If we're in the crossfade section of a visibility range, conditionally
    // discard the fragment according to the visibility pattern.
#ifdef VISIBILITY_RANGE_DITHER
    visibility_range_dither(in.position, in.visibility_range_dither);
#endif

#ifdef FORWARD_DECAL
    let forward_decal_info = get_forward_decal_info(in);
    in.world_position = forward_decal_info.world_position;
    in.uv = forward_decal_info.uv;
#endif

    // generate a PbrInput struct from the StandardMaterial bindings
    var pbr_input = pbr_input_from_standard_material(in, is_front);
#ifndef PREPASS_PIPELINE
    // Preserve authored alpha as specular data before opacity/tattoos/discard.
    let retail_albedo=textureSample(pbr_bindings::base_color_texture,pbr_bindings::base_color_sampler,in.uv);
    let retail_mapped=character_normal(in.world_position,in.world_normal,in.uv,
        textureSample(pbr_bindings::normal_map_texture,pbr_bindings::normal_map_sampler,in.uv).xy);
    let retail_n=select(normalize(in.world_normal),retail_mapped,retail.options.x>0.0);
    if retail.light.w>0.0 && retail.tint.w>0.0 {
        pbr_input.material.base_color=vec4<f32>(character_albedo(retail_albedo.rgb)*retail.tint.rgb,retail_albedo.a);
    }
#endif



#ifdef VERTEX_UVS_B
    // The diffuse alpha is NOT coverage. Hair has a separate red-channel
    // opacity map on secondary UVs (cac_hair_defaultVS/PS).
    let hair_alpha = textureSample(hair_opacity, hair_sampler, in.uv_b).r;
    pbr_input.material.base_color.a = mix(pbr_input.material.base_color.a, hair_alpha, stamp.enabled.y);
    // cacstamp_skin_defaultPS: skin * (1 + (decal.rgb - 1) * decal.a).
    // Its VS transforms TEXCOORD1 by the two i_customGraphic rows.
    let stamp_uv = in.uv_b * stamp.transform.xy + stamp.transform.zw;
    let decal = textureSample(stamp_texture, stamp_sampler, stamp_uv);
    let inside = all(in.uv_b >= stamp.rectangle.xy) && all(in.uv_b <= stamp.rectangle.zw);
    let opacity = decal.a * stamp.enabled.x * select(0.0, 1.0, inside);
    pbr_input.material.base_color = vec4<f32>(
        pbr_input.material.base_color.rgb * mix(vec3<f32>(1.0), decal.rgb, opacity),
        pbr_input.material.base_color.a);
#endif

    // alpha discard
    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);

    // clustered decals
    apply_decals(&pbr_input);

#ifdef PREPASS_PIPELINE
    // write the gbuffer, lighting pass id, and optionally normal and motion_vector textures
    let out = deferred_output(in, pbr_input);
#else
    // in forward mode, we calculate the lit color immediately, and then apply some post-lighting effects here.
    // in deferred mode the lit color and these effects will be calculated in the deferred lighting shader
    var out: FragmentOutput;
    if retail.light.w > 0.0 && retail.tint.w > 0.0 {
        let mask = textureSample(retail_mask, retail_mask_sampler, in.uv).r;
        out.color = shade_character(retail, retail_n, pbr_input.world_position,
            pbr_input.material.base_color.rgb,
            select(retail_albedo.a, mask, retail.options.y > 0.0),
            pbr_input.material.base_color.a);
    } else if (pbr_input.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u {
        out.color = apply_pbr_lighting(pbr_input);
    } else {
        out.color = pbr_input.material.base_color;
    }

    // apply in-shader post processing (fog, alpha-premultiply, and also tonemapping, debanding if the camera is non-hdr)
    // note this does not include fullscreen postprocessing effects like bloom.
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
#endif

#ifdef OIT_ENABLED
    let alpha_mode = pbr_input.material.flags & pbr_types::STANDARD_MATERIAL_FLAGS_ALPHA_MODE_RESERVED_BITS;
    if alpha_mode != pbr_types::STANDARD_MATERIAL_FLAGS_ALPHA_MODE_OPAQUE {
        // The fragments will only be drawn during the oit resolve pass.
        oit_draw(in.position, out.color);
        discard;
    }
#endif // OIT_ENABLED

#ifdef FORWARD_DECAL
        out.color.a = min(forward_decal_info.alpha, out.color.a);
#endif

        return out;
}
