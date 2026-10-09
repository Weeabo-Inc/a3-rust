// Sky dome: gradient from the lighting tables, sun and moon discs, stars and a cloud layer.
// A full-screen triangle at the far plane (reversed-Z depth 0), drawn only where no geometry is.

// Steps of the dome ramp, shared with `crates/a3-render/src/sky.rs`.
const DOME_RAMP_STEPS: u32 = 32u;

struct Frame {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    sun_dir: vec4<f32>,
    sun_color: vec4<f32>,
    sky_zenith: vec4<f32>,
    sky_horizon: vec4<f32>,
    viewport: vec4<f32>,
    params: vec4<f32>,
    ambient_sky: vec4<f32>,
    ambient_mid: vec4<f32>,
    ambient_ground: vec4<f32>,
    fog: vec4<f32>,
    haze: vec4<f32>,
    // x: linear fog end, y: 1 / (end - start); y = 0 disables.
    linear_fog: vec4<f32>,
}

struct Sky {
    zenith: vec4<f32>,
    horizon: vec4<f32>,
    around_sun: vec4<f32>,
    ground: vec4<f32>,
    // xyz: towards the sun, w: cos of the disc radius.
    sun: vec4<f32>,
    // rgb: sun colour, w: disc radiance scale.
    sun_color: vec4<f32>,
    // xyz: towards the moon, w: sin of the disc radius.
    moon: vec4<f32>,
    moon_color: vec4<f32>,
    // rgb: cloud colour, w: cover.
    clouds: vec4<f32>,
    // x: opacity, y: base height (m), z: 1 / noise period (m), w: time (s).
    cloud_params: vec4<f32>,
    // xy: wind (m/s), z: star brightness.
    misc: vec4<f32>,
    star_rotation: mat3x3<f32>,
    // The dome's ramp over elevation, one entry per 90 / (DOME_RAMP_STEPS - 1) degrees; all
    // ones when the World has no `skyTexture`.
    dome_ramp: array<vec4<f32>, DOME_RAMP_STEPS>,
}

@group(0) @binding(0) var<uniform> frame: Frame;
@group(1) @binding(0) var<uniform> sky: Sky;
@group(1) @binding(1) var noise_texture: texture_2d<f32>;
@group(1) @binding(2) var noise_sampler: sampler;

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOut {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    var out: VertexOut;
    // Depth 0: the far plane of the reversed-Z projection.
    out.clip = vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
    return out;
}

fn view_ray(pixel: vec2<f32>) -> vec3<f32> {
    let ndc = vec2<f32>(pixel.x * frame.viewport.z * 2.0 - 1.0, 1.0 - pixel.y * frame.viewport.w * 2.0);
    let p = frame.inv_view_proj * vec4<f32>(ndc, 1.0, 1.0);
    return normalize(p.xyz / p.w);
}

fn hash3(p: vec3<f32>) -> f32 {
    let q = fract(p * vec3<f32>(0.1031, 0.1030, 0.0973));
    let r = q + dot(q, q.yzx + 33.33);
    return fract((r.x + r.y) * r.z);
}

// The sky dome's elevation ramp from the World's `skyTexture`.
//
// The engine's dome shader (`PSHorizon`) shades `tint * texture + sunGlow`: the dome's own
// gradient comes from its `Sky` texture, which stores the zenith first and the horizon last and
// which the shipped dome (`obloha.p3d`) maps to elevation by its UV table
// (docs/re/render-atmosphere.md §4). `sky.dome_ramp` is that texture over elevation, divided by
// its value at the horizon, so this only ever scales the sky the lighting table gives us -- it
// leaves the horizon alone and carries the texture's own ratio and hue towards the zenith.
// One (no change) without a `skyTexture`.
fn dome_ramp(up: f32) -> vec3<f32> {
    let elevation = asin(clamp(up, 0.0, 1.0)) * (2.0 / 3.14159265);
    let x = elevation * f32(DOME_RAMP_STEPS - 1u);
    let i = min(u32(floor(x)), DOME_RAMP_STEPS - 2u);
    return mix(sky.dome_ramp[i].rgb, sky.dome_ramp[i + 1u].rgb, x - floor(x));
}

// Sky gradient with the glow around the sun.
fn gradient(dir: vec3<f32>) -> vec3<f32> {
    let up = saturate(dir.y);
    var color = mix(sky.horizon.rgb, sky.zenith.rgb, pow(up, 0.45));
    color = color * dome_ramp(up);
    let sun_up = saturate(sky.sun.y * 4.0 + 0.5);
    let glow = pow(saturate(dot(dir, sky.sun.xyz) * 0.5 + 0.5), 8.0);
    color = mix(color, sky.around_sun.rgb, glow * 0.7 * sun_up * (1.0 - up * 0.5));
    if dir.y < 0.0 {
        // Below the horizon: fade from the horizon colour to the ground.
        color = mix(color, sky.ground.rgb, saturate(-dir.y * 8.0));
    }
    return color;
}

fn sun_disc(dir: vec3<f32>) -> vec3<f32> {
    let c = dot(dir, sky.sun.xyz);
    let cos_r = sky.sun.w;
    let edge = (1.0 - cos_r) * 0.15;
    let disc = smoothstep(cos_r - edge, cos_r + edge, c);
    // Sets below the horizon (the terrain or sea hides the lower part of the disc).
    let visible = smoothstep(-0.02, 0.0, sky.sun.y);
    let halo = pow(saturate(c), 2000.0) * 0.05 + pow(saturate(c), 200.0) * 0.01;
    return sky.sun_color.rgb * (disc * sky.sun_color.w + halo * sky.sun_color.w * 0.02) * visible;
}

fn moon_disc(dir: vec3<f32>) -> vec3<f32> {
    let m = sky.moon.xyz;
    let c = dot(dir, m);
    if c <= 0.0 {
        return vec3<f32>(0.0);
    }
    // Position on the disc in units of its radius.
    let offset = (dir - m * c) / sky.moon.w;
    let r2 = dot(offset, offset);
    if r2 >= 1.0 {
        return vec3<f32>(0.0);
    }
    // Sphere normal facing us, lit by the sun.
    let normal = normalize(offset - m * sqrt(1.0 - r2));
    let lit = saturate(dot(normal, sky.sun.xyz) * 4.0);
    let rim = smoothstep(1.0, 0.85, r2);
    let visible = smoothstep(-0.02, 0.0, m.y);
    return sky.moon_color.rgb * (lit + 0.004) * rim * visible;
}

fn stars(dir: vec3<f32>) -> vec3<f32> {
    if sky.misc.z <= 0.0 || dir.y <= 0.0 {
        return vec3<f32>(0.0);
    }
    // Fixed to the celestial sphere.
    let e = transpose(sky.star_rotation) * dir;
    let n = 260.0;
    let cell = floor(e * n);
    let h = hash3(cell);
    if h < 0.985 {
        return vec3<f32>(0.0);
    }
    let centre = (cell + 0.5) / n;
    let d = length(e - normalize(centre)) * n;
    let point = saturate(1.0 - d * 1.8);
    let magnitude = pow((h - 0.985) / 0.015, 6.0);
    let tint = mix(vec3<f32>(0.8, 0.85, 1.0), vec3<f32>(1.0, 0.9, 0.75), hash3(cell + 7.0));
    // Fade into the horizon haze.
    return tint * point * magnitude * sky.misc.z * saturate(dir.y * 6.0);
}

// Cloud layer: a plane at the cloud base, textured with tiling noise octaves.
fn clouds(dir: vec3<f32>) -> vec4<f32> {
    if sky.clouds.w <= 0.0 || dir.y <= 0.002 {
        return vec4<f32>(0.0);
    }
    let distance = sky.cloud_params.y / dir.y;
    let p = dir.xz * distance + sky.misc.xy * sky.cloud_params.w;
    let uv = p * sky.cloud_params.z;
    let lod = clamp(log2(distance * sky.cloud_params.z * 64.0), 0.0, 6.0);
    let a = textureSampleLevel(noise_texture, noise_sampler, uv, lod).r;
    let b = textureSampleLevel(noise_texture, noise_sampler, uv * 2.7 + 0.31, lod).g;
    let c = textureSampleLevel(noise_texture, noise_sampler, uv * 7.3 + 0.67, lod).b;
    let n = a * 0.55 + b * 0.3 + c * 0.15;
    let cover = sky.clouds.w;
    let threshold = 0.78 - cover * 0.6;
    let density = smoothstep(threshold - 0.06, threshold + 0.16, n);
    // Thin out towards the horizon where the layer is seen edge-on and far away.
    let fade = smoothstep(0.0, 0.12, dir.y);
    let alpha = density * sky.cloud_params.x * fade;
    // Lit side towards the sun, darker cores in thick cloud.
    let sun_side = 0.75 + 0.25 * saturate(dot(dir, sky.sun.xyz));
    let shade = mix(1.0, 0.55, density * cover);
    let colour = mix(sky.horizon.rgb, sky.clouds.rgb * sun_side * shade, fade);
    return vec4<f32>(colour, alpha);
}

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let dir = view_ray(position.xy);
    var color = gradient(dir) + stars(dir) + moon_disc(dir) + sun_disc(dir);
    let cloud = clouds(dir);
    color = mix(color, cloud.rgb, cloud.a);
    return vec4<f32>(color, 1.0);
}
