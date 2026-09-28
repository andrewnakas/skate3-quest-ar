#import bevy_pbr::{forward_io::Vertex, mesh_view_bindings::view}
struct SkyParams { settings: vec4<f32>, sun_direction: vec4<f32> }
@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> p: SkyParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var panorama: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var panorama_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var sun_gradient: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var sun_sampler: sampler;
struct SkyOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) dome_position: vec3<f32>,
}
@vertex
fn vertex(v: Vertex) -> SkyOut {
    var out: SkyOut;
    let world = v.position + vec3<f32>(view.world_position.x,p.settings.x,view.world_position.z);
    out.position = view.clip_from_world*vec4<f32>(world,1.0);
    // Reverse-Z far plane: the authored dome is larger than the camera's far distance.
    out.position.z = 0.0000001*out.position.w;
    out.uv = v.uv;
    out.dome_position = v.position;
    return out;
}
@fragment
fn fragment(i: SkyOut) -> @location(0) vec4<f32> {
    let d = textureSample(panorama,panorama_sampler,i.uv).rgb;
    var linear = d*d;
    if p.settings.w > 0.0 {
        // Native sky_defaultPS: sine-angle lookup in the material's specular
        // gradient, alpha controls the HDR core. World position minus the sky
        // anchor is exactly the local dome position used here.
        let dot_pl = saturate(dot(normalize(i.dome_position),p.sun_direction.xyz));
        let sin_angle = sqrt(saturate(1.0-dot_pl*dot_pl));
        let sun = textureSample(sun_gradient,sun_sampler,vec2<f32>(sin_angle/p.settings.w,0.5/16.0));
        linear += sun.rgb*sun.rgb/saturate(sun.a+0.01);
    }
    return vec4<f32>(linear*p.settings.y*p.settings.z,1.0);
}
