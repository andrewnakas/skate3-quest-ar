// Native multiplicative evaluator (827F0D00) and centre weighting (827F0B78).
// Portable meter: 16x16 bilinear HDR samples, not the console resolve surface.
@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var source_sampler: sampler;
struct Settings { tuning: vec4<f32>, timing: vec4<f32> }
@group(0) @binding(2) var<uniform> settings: Settings;
@group(0) @binding(3) var<storage, read_write> state: vec4<f32>;
var<workgroup> sums: array<f32, 256>;
@compute @workgroup_size(256)
fn meter(@builtin(local_invocation_index) id: u32) {
    if id == 0u && state.z != settings.timing.y {
        state.x = clamp(2.5, settings.tuning.y, settings.tuning.z);
        state.z = settings.timing.y;
    }
    storageBarrier();
    let uv=(vec2<f32>(f32(id%16u),f32(id/16u))+0.5)/16.0;
    let xy=uv*2.0-1.0;
    let weight=pow(1.0-abs(xy.x*xy.y),2.0);
    // Material output currently includes baseline exposure 2.5. Meter the
    // current adapted image, with a portable Rec.709 luminance conversion.
    let c=max(textureSampleLevel(source,source_sampler,uv,0.0).rgb,vec3<f32>(0.0))*state.x/2.5;
    sums[id]=min(dot(c,vec3<f32>(0.2126,0.7152,0.0722)),16.0)*weight;
    workgroupBarrier();
    for(var stride=128u;stride>0u;stride/=2u) {
        if id<stride { sums[id]+=sums[id+stride]; }
        workgroupBarrier();
    }
    if id==0u {
        let average=sums[0]/256.0*2.515;
        let factor=max(0.001,1.0+settings.tuning.w*(settings.tuning.x-average));
        // Native evaluator ticks per frame; adapt to a 30 Hz time basis here.
        state.x=clamp(state.x*pow(factor,settings.timing.x*30.0),settings.tuning.y,settings.tuning.z);
        state.y=average;
    }
}
