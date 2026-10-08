// Bloom source: the exposed HDR scene at quarter resolution, blurred with a separable Gaussian.
// RV's bloom generation passes are not decoded yet (docs/re/render-atmosphere.md covers only the
// final mix), so this is our approximation; the final pass mixes it in as RV does.

// Must match post.rs (PostUniforms).
struct Post {
    filmic_abcd: vec4<f32>,
    filmic_efw_bias: vec4<f32>,
    exposure: vec4<f32>,
    adaptation: vec4<f32>,
    misc: vec4<f32>,
    bloom: vec4<f32>,
    output: vec4<f32>,
}

struct ExposureState {
    adapted_luminance: f32,
    exposure: f32,
    average_luminance: f32,
    _pad: f32,
}

@group(0) @binding(0) var<uniform> post: Post;
@group(0) @binding(1) var source: texture_2d<f32>;
@group(0) @binding(2) var<storage, read> state: ExposureState;

const WEIGHTS: array<f32, 5> = array<f32, 5>(0.2270270, 0.1945946, 0.1216216, 0.0540541, 0.0162162);

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

fn load_clamped(p: vec2<i32>) -> vec3<f32> {
    let size = vec2<i32>(textureDimensions(source)) - vec2<i32>(1);
    return textureLoad(source, clamp(p, vec2<i32>(0), size), 0).rgb;
}

// Average of the 4x4 full-resolution block under this quarter-resolution pixel, exposed.
@fragment
fn fs_down(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let base = vec2<i32>(floor(position.xy)) * 4;
    var sum = vec3<f32>(0.0);
    for (var y = 0; y < 4; y += 1) {
        for (var x = 0; x < 4; x += 1) {
            sum += load_clamped(base + vec2<i32>(x, y));
        }
    }
    return vec4<f32>(sum / 16.0 * state.exposure * post.bloom.z, 1.0);
}

fn blur(position: vec4<f32>, step: vec2<i32>) -> vec4<f32> {
    let p = vec2<i32>(floor(position.xy));
    var sum = load_clamped(p) * WEIGHTS[0];
    for (var i = 1; i < 5; i += 1) {
        sum += (load_clamped(p + step * i) + load_clamped(p - step * i)) * WEIGHTS[i];
    }
    return vec4<f32>(sum, 1.0);
}

@fragment
fn fs_blur_h(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    return blur(position, vec2<i32>(1, 0));
}

@fragment
fn fs_blur_v(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    return blur(position, vec2<i32>(0, 1));
}
