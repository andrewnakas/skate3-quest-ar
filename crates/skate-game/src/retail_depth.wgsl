#import bevy_pbr::prepass_io::VertexOutput
#ifdef PREPASS_FRAGMENT
#import bevy_pbr::prepass_io::FragmentOutput
#endif
#ifdef MOTION_VECTOR_PREPASS
#import bevy_pbr::pbr_prepass_functions::calculate_motion_vector
#endif
#import skate_retail::material_bindings as bindings
#ifdef BINDLESS
#import bevy_pbr::mesh_bindings::mesh
#endif
@fragment
fn fragment(i: VertexOutput)
#ifdef PREPASS_FRAGMENT
    -> FragmentOutput
#endif
{
#ifdef BINDLESS
    let index = bindings::indices[mesh[i.instance_index].material_and_lightmap_bind_group_slot & 0xffffu];
    let alpha = textureSample(bindings::textures[index.diffuse],bindings::samplers[index.diffuse_sampler],i.uv).a;
    let cutoff = bindings::params[index.params].mode.z;
#else
    let alpha = textureSample(bindings::diffuse,bindings::diffuse_sampler,i.uv).a;
    let cutoff = bindings::p.mode.z;
#endif
    if alpha < cutoff { discard; }
#ifdef PREPASS_FRAGMENT
    var out: FragmentOutput;
#ifdef NORMAL_PREPASS
    out.normal = vec4<f32>(normalize(i.world_normal)*0.5+0.5,1.0);
#endif
#ifdef MOTION_VECTOR_PREPASS
    out.motion_vector = calculate_motion_vector(i.world_position,i.previous_world_position);
#endif
#ifdef UNCLIPPED_DEPTH_ORTHO_EMULATION
    out.frag_depth = i.unclipped_depth;
#endif
    return out;
#endif
}
