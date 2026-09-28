#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput
@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var source_sampler: sampler;
@group(0) @binding(2) var<storage, read> exposure: vec4<f32>;

@fragment
fn fragment(i: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let c = textureSample(source,source_sampler,i.uv);
    let xe = max(c.rgb*exposure.x/2.5,vec3<f32>(0.0));
    let t = saturate(1.0-xe);
    let tm = max(xe*0.25+0.75,vec3<f32>(1.0))-t*t;
    let gamma = saturate(sqrt(max(tm*0.5,vec3<f32>(0.0)))*1.41);
    // Bevy's final output attachment performs sRGB encoding. The retail
    // curve already includes gamma; invert sRGB here to avoid encoding twice.
    let linear = select(gamma/12.92,pow((gamma+0.055)/1.055,vec3<f32>(2.4)),gamma>vec3<f32>(0.04045));
    // The reference's final tone pass is opaque. Material coverage has
    // already been resolved; carrying it into the presentation blit would
    // composite foliage a second time against the window's clear colour.
    return vec4<f32>(linear,1.0);
}
