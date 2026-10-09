// The UI pass: batched textured quads in screen pixels, drawn over the finished frame.
//
// Colours in the draw list are sRGB (as config authors them) and the output target is sRGB, so
// this shader decodes the colour and the texture (sampled through an sRGB view) to linear
// values; the hardware's sRGB encode on store restores the stored bytes. Opaque pixels are
// therefore byte-exact, while blending happens in linear space — see docs/re/ui.md.

// Must match renderer.rs (FrameUniforms); the trailing fields are unused here.
struct Frame {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    sun_dir: vec4<f32>,
    sun_color: vec4<f32>,
    sky_zenith: vec4<f32>,
    sky_horizon: vec4<f32>,
    viewport: vec4<f32>,
    params: vec4<f32>,
}

@group(0) @binding(0) var<uniform> frame: Frame;
@group(1) @binding(0) var ui_texture: texture_2d<f32>;
@group(1) @binding(1) var ui_sampler: sampler;

struct VertexIn {
    // Position in pixels from the top-left of the target.
    @location(0) position: vec2<f32>,
    @location(1) uv: vec2<f32>,
    // sRGB, straight alpha.
    @location(2) color: vec4<f32>,
}

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
}

// Must match the convention of final.wgsl.
fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let low = c / 12.92;
    let high = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(high, low, c <= vec3<f32>(0.04045));
}

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    let ndc = vec2<f32>(
        in.position.x * frame.viewport.z * 2.0 - 1.0,
        1.0 - in.position.y * frame.viewport.w * 2.0,
    );
    var out: VertexOut;
    out.clip = vec4<f32>(ndc, 0.0, 1.0);
    out.uv = in.uv;
    out.color = in.color;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let texel = textureSample(ui_texture, ui_sampler, in.uv);
    let alpha = in.color.a * texel.a;
    if alpha <= 0.0 {
        discard;
    }
    return vec4<f32>(srgb_to_linear(in.color.rgb) * texel.rgb, alpha);
}
