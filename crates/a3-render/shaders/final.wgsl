// Final pass: FXAA-style anti-aliasing (or a plain copy) from the gamma-encoded LDR image into
// the output target. If the output format is sRGB, values are decoded so the hardware's sRGB
// encode restores them.

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

@group(0) @binding(0) var<uniform> post: Post;
@group(0) @binding(1) var ldr: texture_2d<f32>;
@group(0) @binding(2) var linear_clamp: sampler;

// Edges with less local contrast than this (relative to the brightest neighbour) stay as they
// are; very dark regions below the absolute minimum too.
const EDGE_THRESHOLD: f32 = 0.125;
const EDGE_THRESHOLD_MIN: f32 = 0.0312;
// Limits of the blend direction estimate.
const REDUCE_MUL: f32 = 0.125;
const REDUCE_MIN: f32 = 0.0078125;
const SPAN_MAX: f32 = 8.0;

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let low = c / 12.92;
    let high = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(high, low, c <= vec3<f32>(0.04045));
}

fn output(c: vec3<f32>) -> vec4<f32> {
    if post.misc.z > 0.5 {
        return vec4<f32>(srgb_to_linear(c), 1.0);
    }
    return vec4<f32>(c, 1.0);
}

fn luma_at(uv: vec2<f32>) -> f32 {
    return textureSampleLevel(ldr, linear_clamp, uv, 0.0).a;
}

fn rgb_at(uv: vec2<f32>) -> vec3<f32> {
    return textureSampleLevel(ldr, linear_clamp, uv, 0.0).rgb;
}

@fragment
fn fs_copy(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    return output(textureLoad(ldr, vec2<i32>(floor(position.xy)), 0).rgb);
}

@fragment
fn fs_fxaa(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let size = vec2<f32>(textureDimensions(ldr));
    let texel = 1.0 / size;
    let uv = position.xy * texel;

    let centre = textureSampleLevel(ldr, linear_clamp, uv, 0.0);
    // Diagonal neighbours: the edge direction comes from their luma gradient.
    let nw = luma_at(uv + vec2<f32>(-1.0, -1.0) * texel);
    let ne = luma_at(uv + vec2<f32>(1.0, -1.0) * texel);
    let sw = luma_at(uv + vec2<f32>(-1.0, 1.0) * texel);
    let se = luma_at(uv + vec2<f32>(1.0, 1.0) * texel);
    let m = centre.a;

    let luma_min = min(m, min(min(nw, ne), min(sw, se)));
    let luma_max = max(m, max(max(nw, ne), max(sw, se)));
    if luma_max - luma_min < max(EDGE_THRESHOLD_MIN, luma_max * EDGE_THRESHOLD) {
        return output(centre.rgb);
    }

    // Gradient across the edge; blend along it (perpendicular to the gradient).
    var dir = vec2<f32>(-((nw + ne) - (sw + se)), (nw + sw) - (ne + se));
    let reduce = max((nw + ne + sw + se) * 0.25 * REDUCE_MUL, REDUCE_MIN);
    let scale = 1.0 / (min(abs(dir.x), abs(dir.y)) + reduce);
    dir = clamp(dir * scale, vec2<f32>(-SPAN_MAX), vec2<f32>(SPAN_MAX)) * texel;

    let inner = 0.5 * (rgb_at(uv + dir * (1.0 / 3.0 - 0.5)) + rgb_at(uv + dir * (2.0 / 3.0 - 0.5)));
    let outer = inner * 0.5 + 0.25 * (rgb_at(uv - dir * 0.5) + rgb_at(uv + dir * 0.5));
    let outer_luma = dot(outer, vec3<f32>(0.299, 0.587, 0.114));
    // The wide blend may cross into a different feature; fall back to the narrow one then.
    if outer_luma < luma_min || outer_luma > luma_max {
        return output(inner);
    }
    return output(outer);
}
