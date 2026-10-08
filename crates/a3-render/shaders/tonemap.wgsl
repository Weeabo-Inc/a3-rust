// RV's final HDR pass (docs/re/render-atmosphere.md §3.2): exposed scene plus bloom faded on bright
// pixels, then the tone curve selected by tonemapMethod and a final gamma. Output: display-referred,
// gamma-encoded LDR with luma in alpha (the input of the anti-aliasing pass).

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
@group(0) @binding(1) var hdr: texture_2d<f32>;
@group(0) @binding(2) var<storage, read> state: ExposureState;
@group(0) @binding(3) var bloom_texture: texture_2d<f32>;
@group(0) @binding(4) var bloom_sampler: sampler;

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

// Hable's curve without the E/F offset: h(x) = (x(Ax + BC) + DE) / (x(Ax + B) + DF).
fn hable(x: vec3<f32>) -> vec3<f32> {
    let a = post.filmic_abcd.x;
    let b = post.filmic_abcd.y;
    let c = post.filmic_abcd.z;
    let d = post.filmic_abcd.w;
    let e = post.filmic_efw_bias.x;
    let f = post.filmic_efw_bias.y;
    return (x * (a * x + b * c) + d * e) / (x * (a * x + b) + d * f);
}

fn filmic(color: vec3<f32>) -> vec3<f32> {
    let offset = post.filmic_efw_bias.x / post.filmic_efw_bias.y;
    let white = post.filmic_efw_bias.z;
    let c = color * post.filmic_efw_bias.w;
    return saturate((hable(c) - offset) / (hable(vec3<f32>(white)) - offset));
}

// Extended Reinhard on Rec.709 luminance, preserving hue.
fn reinhard(c: vec3<f32>) -> vec3<f32> {
    let w = post.misc.y;
    let l = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
    return saturate(c * (l * (1.0 + l / (w * w)) / (1.0 + l)) / (l + 0.0001));
}

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let low = c * 12.92;
    let high = 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(high, low, c <= vec3<f32>(0.0031308));
}

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let scene = textureLoad(hdr, vec2<i32>(floor(position.xy)), 0).rgb * state.exposure
        * post.bloom.z;
    let uv = position.xy / vec2<f32>(textureDimensions(hdr));
    let blurred = max(textureSampleLevel(bloom_texture, bloom_sampler, uv, 0.0).rgb, vec3<f32>(0.0));
    let bloom = pow(blurred, vec3<f32>(post.bloom.w)) * post.output.y;
    // Bloom fades out on bright pixels.
    let k = (1.0 - saturate(2.0 * dot(scene, vec3<f32>(0.299, 0.587, 0.114)))) * post.bloom.y;
    let c = max(scene * post.bloom.x + bloom * k, vec3<f32>(0.0));

    var mapped: vec3<f32>;
    let method = u32(post.misc.x + 0.5);
    if method == 0u {
        mapped = c;
    } else if method == 2u {
        mapped = reinhard(c);
    } else {
        mapped = filmic(c);
    }
    let curved = pow(saturate(mapped), vec3<f32>(post.output.x));
    let display = linear_to_srgb(curved);
    let luma = dot(display, vec3<f32>(0.299, 0.587, 0.114));
    return vec4<f32>(display, luma);
}
