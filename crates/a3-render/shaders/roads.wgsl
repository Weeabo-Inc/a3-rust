// Roads: textured strips draped on the terrain, alpha-blended over it. Positions are relative
// to a chunk origin; the instance carries the chunk origin relative to the camera (ADR 0003).

struct Frame {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    sun_dir: vec4<f32>,
    sun_color: vec4<f32>,
    sky_zenith: vec4<f32>,
    sky_horizon: vec4<f32>,
    viewport: vec4<f32>,
    params: vec4<f32>,
    // The lighting table's hemisphere ambient (AE, AmbientMid, GE); w = 0 when not set.
    ambient_sky: vec4<f32>,
    ambient_mid: vec4<f32>,
    ambient_ground: vec4<f32>,
}

// Must match shadow.rs (ShadowUniforms).
struct Shadows {
    cascades: array<mat4x4<f32>, 4>,
    splits: vec4<f32>,
    view_forward: vec4<f32>,
    texel_sizes: vec4<f32>,
    params: vec4<f32>,
}

@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(1) var shadow_maps: texture_depth_2d_array;
@group(0) @binding(2) var shadow_sampler: sampler_comparison;
@group(0) @binding(3) var<uniform> shadows: Shadows;
@group(1) @binding(0) var road_texture: texture_2d<f32>;
@group(1) @binding(1) var road_sampler: sampler;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    // Chunk origin minus camera position.
    @location(3) offset: vec3<f32>,
}

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) relative: vec3<f32>,
}

@vertex
fn vs_main(v: VertexIn) -> VertexOut {
    let relative = v.position + v.offset;
    var out: VertexOut;
    out.clip = frame.view_proj * vec4<f32>(relative, 1.0);
    out.relative = relative;
    out.normal = v.normal;
    out.uv = v.uv;
    return out;
}

// 1 = fully lit, 0 = in shadow (same lookup as mesh.wgsl).
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
    let offset = normal * shadows.texel_sizes[cascade] * shadows.params.x;
    let clip = shadows.cascades[cascade] * vec4<f32>(relative + offset, 1.0);
    let uv = vec2<f32>(clip.x * 0.5 + 0.5, 0.5 - clip.y * 0.5);
    if any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) || clip.z > 1.0 {
        return 1.0;
    }
    var lit = 0.0;
    for (var y = -1; y <= 1; y += 1) {
        for (var x = -1; x <= 1; x += 1) {
            let shifted = uv + vec2<f32>(f32(x), f32(y)) * shadows.params.z;
            lit += textureSampleCompareLevel(shadow_maps, shadow_sampler, shifted, i32(cascade), clip.z);
        }
    }
    lit /= 9.0;
    let last = shadows.splits[count - 1u];
    let fade = clamp((depth - last * shadows.params.w) / (last * (1.0 - shadows.params.w)), 0.0, 1.0);
    return mix(lit, 1.0, fade);
}

// The Road shader's hemisphere ambient (docs/re/render-materials.md §3.1); without a lighting
// table, the flat ambient level.
fn ambient(y: f32) -> vec3<f32> {
    if frame.ambient_sky.w > 0.0 {
        if y > 0.0 {
            return mix(frame.ambient_mid.rgb, frame.ambient_sky.rgb, saturate(y));
        }
        return mix(frame.ambient_ground.rgb, frame.ambient_mid.rgb, saturate(1.0 + y));
    }
    return vec3<f32>(frame.sun_color.w);
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    // Road shader: albedo x 2 x detail; the shipped detail stage is a constant 0.5 grey, so the
    // albedo is used as is.
    let albedo = textureSample(road_texture, road_sampler, in.uv);
    let n = normalize(in.normal);
    var diffuse = max(dot(n, frame.sun_dir.xyz), 0.0);
    if diffuse > 0.0 {
        diffuse *= sun_visibility(in.relative, n);
    }
    let light = frame.sun_color.rgb * diffuse + ambient(n.y);
    return vec4<f32>(albedo.rgb * light, albedo.a);
}
