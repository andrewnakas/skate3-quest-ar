#import bevy_pbr::{forward_io::VertexOutput, mesh_view_bindings as frame}
#import skate_character_lighting::{CharacterParams, shade_character, character_normal, character_albedo}
@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> p: CharacterParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var diffuse: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var diffuse_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var normal_map: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var normal_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(5) var mask_map: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(6) var mask_sampler: sampler;

@fragment
fn fragment(i: VertexOutput) -> @location(0) vec4<f32> {
    let albedo = textureSample(diffuse,diffuse_sampler,i.uv);
    let d=character_albedo(albedo.rgb)*p.tint.rgb;
    let mapped=character_normal(i.world_position,i.world_normal,i.uv,textureSample(normal_map,normal_sampler,i.uv).xy);
    let vn=select(normalize(i.world_normal),mapped,p.options.x>0.0);
    var smask=albedo.a;
    if p.options.y>0.0 { smask=textureSample(mask_map,mask_sampler,i.uv).r; }
    return shade_character(p, vn, i.world_position, d, smask, albedo.a);
}
