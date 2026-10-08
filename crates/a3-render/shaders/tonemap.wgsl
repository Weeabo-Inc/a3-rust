// Exposure and tonemapping: HDR -> display-referred, gamma-encoded LDR with luma in alpha
// (the input the anti-aliasing pass expects). See docs/re/hdr.md.

struct Post {
    filmic_abcd: vec4<f32>,
    filmic_efw_bias: vec4<f32>,
    exposure: vec4<f32>,
    adaptation: vec4<f32>,
    misc: vec4<f32>,
}

struct ExposureState {
    adapted_luminance: f32,
    exposure: f32,
    average_luminance: f32,
    _pad: f32,
}

@group(0) @binding(0) var<uniform> post: Post;
@group(0) @binding(1) var hdr: texture_2d<f32>;
@group(0) @binding(2) var<storage, read> state: ExposureState;

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

fn hable(x: vec3<f32>) -> vec3<f32> {
    let a = post.filmic_abcd.x;
    let b = post.filmic_abcd.y;
    let c = post.filmic_abcd.z;
    let d = post.filmic_abcd.w;
    let e = post.filmic_efw_bias.x;
    let f = post.filmic_efw_bias.y;
    return (x * (a * x + c * b) + d * e) / (x * (a * x + b) + d * f) - e / f;
}

fn filmic(x: vec3<f32>) -> vec3<f32> {
    let white = post.filmic_efw_bias.z;
    return hable(x) / hable(vec3<f32>(white));
}

fn reinhard(x: vec3<f32>) -> vec3<f32> {
    let w = post.misc.y;
    return x * (1.0 + x / (w * w)) / (1.0 + x);
}

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let low = c * 12.92;
    let high = 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(high, low, c <= vec3<f32>(0.0031308));
}

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let hdr_color = textureLoad(hdr, vec2<i32>(floor(position.xy)), 0).rgb;
    let x = max(hdr_color * state.exposure * post.filmic_efw_bias.w, vec3<f32>(0.0));
    // RV tonemapMethod: 0 none, 1 filmic (Hable), 2 Reinhard.
    var mapped: vec3<f32>;
    let method = u32(post.misc.x + 0.5);
    if method == 0u {
        mapped = x;
    } else if method == 2u {
        mapped = reinhard(x);
    } else {
        mapped = filmic(x);
    }
    let display = linear_to_srgb(clamp(mapped, vec3<f32>(0.0), vec3<f32>(1.0)));
    let luma = dot(display, vec3<f32>(0.299, 0.587, 0.114));
    return vec4<f32>(display, luma);
}
