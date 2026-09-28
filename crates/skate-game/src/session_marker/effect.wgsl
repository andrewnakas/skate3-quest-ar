#import bevy_sprite::mesh2d_vertex_output::VertexOutput

struct Params { scroll: vec4<f32>, first: vec4<f32>, second: vec4<f32>, fade: vec4<f32> };
@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: Params;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var noise_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var noise_sampler: sampler;

// Noise contribution of postfx_basictex_fisheye_noisePS, instructions3..13,
//17..18,27..28. Scene blending uses native f_NoiseFade=(1-p,p,0,0).
@fragment fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let first = textureSample(noise_texture, noise_sampler,
        in.uv * vec2<f32>(10., 5.625) + params.scroll.xy);
    let second = textureSample(noise_texture, noise_sampler,
        in.uv * vec2<f32>(11., 6.625) + params.scroll.zw);
    let value = clamp(2. * (dot(first, params.first) + dot(second, params.second)) - 2., 0., 1.);
    return vec4<f32>(vec3<f32>(value), params.fade.x);
}
