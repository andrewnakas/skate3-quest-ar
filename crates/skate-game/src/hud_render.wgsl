#import bevy_sprite::mesh2d_vertex_output::VertexOutput
struct ColorTransform { multiply: vec4<f32>, add: vec4<f32> }
@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> color: ColorTransform;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var atlas: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var atlas_sampler: sampler;
@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    return clamp(textureSample(atlas, atlas_sampler, in.uv) * color.multiply + color.add, vec4(0.0), vec4(1.0));
}
