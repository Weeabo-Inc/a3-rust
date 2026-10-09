// Lit, textured mesh instances with sun shadows. Positions are camera-relative (see ADR 0003).

struct Frame {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    // xyz: unit vector towards the sun.
    sun_dir: vec4<f32>,
    // rgb: sun colour, w: ambient level.
    sun_color: vec4<f32>,
    sky_zenith: vec4<f32>,
    // rgb: horizon/fog colour, w: fog density per metre.
    sky_horizon: vec4<f32>,
    // width, height, 1/width, 1/height.
    viewport: vec4<f32>,
    // near plane, time in seconds, unused, unused.
    params: vec4<f32>,
    // Hemisphere ambient: from above, from the horizon, from below.
    ambient_sky: vec4<f32>,
    ambient_mid: vec4<f32>,
    ambient_ground: vec4<f32>,
    // x: fog extinction at sea level, y: fog height decay, z: camera world height,
    // w: 1 when the built-in sky is drawn.
    fog: vec4<f32>,
    // rgb: haze extinction per metre.
    haze: vec4<f32>,
    // x: linear fog end, y: 1 / (end - start); y = 0 disables.
    linear_fog: vec4<f32>,
}

// Must match shadow.rs (ShadowUniforms).
struct Shadows {
    cascades: array<mat4x4<f32>, 4>,
    splits: vec4<f32>,
    // xyz: camera forward, w: cascade count (0 = off).
    view_forward: vec4<f32>,
    texel_sizes: vec4<f32>,
    // normal bias in texels, map size, 1 / map size, fade start.
    params: vec4<f32>,
}

@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(1) var shadow_maps: texture_depth_2d_array;
@group(0) @binding(2) var shadow_sampler: sampler_comparison;
@group(0) @binding(3) var<uniform> shadows: Shadows;
@group(1) @binding(0) var base_texture: texture_2d<f32>;
@group(1) @binding(1) var base_sampler: sampler;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) tangent: vec4<f32>,
}

struct InstanceIn {
    @location(4) model_0: vec4<f32>,
    @location(5) model_1: vec4<f32>,
    @location(6) model_2: vec4<f32>,
    @location(7) model_3: vec4<f32>,
    @location(8) color: vec4<f32>,
}

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) tangent: vec4<f32>,
    @location(4) relative: vec3<f32>,
}

fn model_of(i: InstanceIn) -> mat4x4<f32> {
    return mat4x4<f32>(i.model_0, i.model_1, i.model_2, i.model_3);
}

@vertex
fn vs_main(v: VertexIn, i: InstanceIn) -> VertexOut {
    let model = model_of(i);
    let relative = model * vec4<f32>(v.position, 1.0);
    var out: VertexOut;
    out.clip = frame.view_proj * relative;
    out.relative = relative.xyz;
    out.normal = (model * vec4<f32>(v.normal, 0.0)).xyz;
    out.tangent = vec4<f32>((model * vec4<f32>(v.tangent.xyz, 0.0)).xyz, v.tangent.w);
    out.uv = v.uv;
    out.color = i.color;
    return out;
}

// Depth-only pass into a shadow cascade; `frame.view_proj` is the cascade's light matrix.
@vertex
fn vs_shadow(v: VertexIn, i: InstanceIn) -> @builtin(position) vec4<f32> {
    return frame.view_proj * (model_of(i) * vec4<f32>(v.position, 1.0));
}

// 1 = fully lit, 0 = in shadow.
fn sun_visibility(relative: vec3<f32>, normal: vec3<f32>) -> f32 {
    let count = u32(shadows.view_forward.w + 0.5);
    if count == 0u {
        return 1.0;
    }
    let depth = dot(relative, shadows.view_forward.xyz);
    var cascade = count;
    for (var c = 0u; c < count; c += 1u) {
        if depth <= shadows.splits[c] {
            cascade = c;
            break;
        }
    }
    if cascade >= count {
        return 1.0;
    }
    // Push the receiver off its surface by a few texels against self-shadowing.
    let offset = normal * shadows.texel_sizes[cascade] * shadows.params.x;
    let clip = shadows.cascades[cascade] * vec4<f32>(relative + offset, 1.0);
    let uv = vec2<f32>(clip.x * 0.5 + 0.5, 0.5 - clip.y * 0.5);
    if any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) || clip.z > 1.0 {
        return 1.0;
    }
    // 3x3 hardware-filtered PCF.
    var lit = 0.0;
    for (var y = -1; y <= 1; y += 1) {
        for (var x = -1; x <= 1; x += 1) {
            let shifted = uv + vec2<f32>(f32(x), f32(y)) * shadows.params.z;
            lit += textureSampleCompareLevel(shadow_maps, shadow_sampler, shifted, i32(cascade), clip.z);
        }
    }
    lit /= 9.0;
    // Fade out towards the end of the shadow distance.
    let last = shadows.splits[count - 1u];
    let fade = clamp((depth - last * shadows.params.w) / (last * (1.0 - shadows.params.w)), 0.0, 1.0);
    return mix(lit, 1.0, fade);
}

// RV's hemisphere ambient (docs/re/render-materials.md): mid at the horizon, towards the
// sky colour above and the ground colour below.
fn hemisphere_ambient(n: vec3<f32>) -> vec3<f32> {
    if frame.ambient_sky.w < 0.5 {
        // No hemisphere set: even ambient.
        return vec3<f32>(frame.sun_color.w);
    }
    if n.y > 0.0 {
        return mix(frame.ambient_mid.rgb, frame.ambient_sky.rgb, saturate(n.y));
    }
    return mix(frame.ambient_ground.rgb, frame.ambient_mid.rgb, saturate(1.0 + n.y));
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let base = textureSample(base_texture, base_sampler, in.uv) * in.color;
    let n = normalize(in.normal);
    let n_dot_l = dot(n, frame.sun_dir.xyz);
    var diffuse = max(n_dot_l, 0.0);
    if diffuse > 0.0 {
        diffuse *= sun_visibility(in.relative, n);
    }
    let light = frame.sun_color.rgb * diffuse + hemisphere_ambient(n);
    return vec4<f32>(base.rgb * light, base.a);
}
