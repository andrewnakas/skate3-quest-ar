#import bevy_ui::ui_vertex_output::UiVertexOutput
@group(1) @binding(0) var hud: texture_2d<f32>;
@group(1) @binding(1) var hud_sampler: sampler;
@fragment
fn fragment(in: UiVertexOutput) -> @location(0) vec4<f32> {
    return textureSample(hud, hud_sampler, in.uv);
}
