// The sea: the engine's VSWater / PSWater (docs/re/render-water.md). Positions are
// camera-relative (ADR 0003); waves and texture coordinates use world metres.

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

// Must match sea.rs (SeaUniforms).
struct Sea {
    // A, A, 2pi XScale A, 2pi ZScale A (VSC_WaveHeight).
    wave: vec4<f32>,
    // u scale, v scale, radial phase, angular phase.
    wave_uv: vec4<f32>,
    // 1 / WaterGrid, map centre in water cells, depth lattice points per side, sea level.
    grid: vec4<f32>,
    // time (s), camera world height, 1 / sqrt(P00 P11), P00.
    misc: vec4<f32>,
    // camera forward (unit), near plane.
    forward: vec4<f32>,
    // camera world x, z, view distance, unused.
    camera: vec4<f32>,
    // PSC_CalmWaterPars1[0], [1], [3], [4].
    cwp0: vec4<f32>,
    cwp1: vec4<f32>,
    cwp3: vec4<f32>,
    cwp4: vec4<f32>,
    // PSC_WaterAdditionalPars[0..2].
    ap0: vec4<f32>,
    ap1: vec4<f32>,
    ap2: vec4<f32>,
    // PSC_WaterSSReflectionPars[0..1].
    ssr0: vec4<f32>,
    ssr1: vec4<f32>,
    // Foam colour (PSC_WaveColor).
    wave_color: vec4<f32>,
    // Sun glint colour (PSC_Specular).
    specular: vec4<f32>,
    // Direction the light travels.
    light: vec4<f32>,
    // Sky reflection tint (PSC_GlassEnvColor); w: blend towards the second sky texture.
    env: vec4<f32>,
    // Water fog colour; w: extinction per metre.
    water_fog: vec4<f32>,
    // Water fog colour scale looking down, at the horizon, up.
    water_gradient: vec4<f32>,
}

struct Exposure {
    adapted_luminance: f32,
    exposure: f32,
    average_luminance: f32,
    pad: f32,
}

@group(0) @binding(0) var<uniform> frame: Frame;
@group(1) @binding(0) var<uniform> sea: Sea;
// Terrain height at every water-grid vertex of the map (the patches' VSC_WaterDepth).
@group(1) @binding(1) var lattice: texture_2d<f32>;
@group(1) @binding(2) var wave_normals: texture_2d<f32>;
@group(1) @binding(3) var detail_normals: texture_2d<f32>;
@group(1) @binding(4) var foam_texture: texture_2d<f32>;
// The overcast sample's sky reflection textures (t5, then t4).
@group(1) @binding(5) var sky_lower: texture_2d<f32>;
@group(1) @binding(6) var sky_upper: texture_2d<f32>;
@group(1) @binding(7) var repeat_sampler: sampler;
@group(1) @binding(8) var clamp_sampler: sampler;
@group(1) @binding(9) var<storage, read> exposure: Exposure;
// The opaque scene (Phase::Water group 2).
@group(2) @binding(0) var scene_color: texture_2d<f32>;
@group(2) @binding(1) var scene_depth: texture_depth_2d;
@group(2) @binding(2) var scene_sampler: sampler;

const PI: f32 = 3.14159265;
const TAU: f32 = 6.28318531;

// The engine's wave sine and cosine: of 2 pi frac(x) - pi.
fn wave_sin(x: f32) -> f32 {
    return sin(fract(x) * TAU - PI);
}

fn wave_cos(x: f32) -> f32 {
    return cos(fract(x) * TAU - PI);
}

fn lattice_height(i: i32, j: i32) -> f32 {
    let last = i32(sea.grid.z) - 1;
    return textureLoad(lattice, vec2<i32>(clamp(i, 0, last), clamp(j, 0, last)), 0).r;
}

// Bilinear terrain height over the water-grid lattice at world `p`.
fn terrain_at(p: vec2<f32>) -> f32 {
    let g = p * sea.grid.x;
    let base = floor(g);
    let f = g - base;
    let i = i32(base.x);
    let j = i32(base.y);
    let a = mix(lattice_height(i, j), lattice_height(i + 1, j), f.x);
    let b = mix(lattice_height(i, j + 1), lattice_height(i + 1, j + 1), f.x);
    return mix(a, b, f.y);
}

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    // Camera-relative position.
    @location(0) rel: vec3<f32>,
    @location(1) world_xz: vec2<f32>,
    // Wave normal (VSWater o3).
    @location(2) normal: vec3<f32>,
    // Normal map coordinates: 500 m and 50 m octaves (o4).
    @location(3) uv_a: vec4<f32>,
    // xy: 15 m octave; z, w: octave weights by distance (o5).
    @location(4) uv_b: vec4<f32>,
    // Shallowness 0 (deeper than 40 m) .. 1 (shallower than 8 m) (o6.x).
    @location(5) shallow: f32,
    // Crest foam coordinates: uv_a.xy * 50.1, uv_a.zw * 10.2 and uv_b.xy * 0.11, each kept
    // small on its own so the patches stay seamless.
    @location(6) crest_a: vec4<f32>,
    @location(7) crest_b: vec2<f32>,
}

// World `origin` * `scale` + `scroll`, kept small: the fraction of the patch origin's
// coordinate plus the vertex offset (textures repeat).
fn scrolled(origin: vec2<f32>, local: vec2<f32>, scale: f32, scroll: f32) -> vec2<f32> {
    return fract(origin * scale + vec2<f32>(scroll)) + local * scale;
}

@vertex
fn vs_sea(
    @location(0) local: vec2<f32>,
    @location(1) rel_origin: vec2<f32>,
    @location(2) world_origin: vec2<f32>,
) -> VsOut {
    let world = world_origin + local;
    let rel_xz = rel_origin + local;
    let sea_level = sea.grid.w;
    let camera_y = sea.misc.y;

    // Rings around the map centre (docs/re/render-water.md section 3.1).
    let g = world * sea.grid.x - vec2<f32>(sea.grid.y);
    let angle = atan2(g.x, g.y) * 79.577472;
    let u = length(g) * sea.wave_uv.x + sea.wave_uv.z;
    let v = angle * sea.wave_uv.y + sea.wave_uv.w;
    let u2 = u + 0.2 * wave_sin(0.25 * v) + 0.12 * wave_sin(0.375 * v);
    var h = wave_sin(u2) * sea.wave.x + wave_sin(v) * sea.wave.y;
    // Waves only near the camera (zoom widens the range).
    let d = length(vec3<f32>(rel_xz.x, sea_level - camera_y, rel_xz.y));
    h *= 1.0 - saturate((d - 50.0) * sea.misc.z * 0.01);
    let depth = terrain_at(world) - sea_level;
    let peak = saturate((depth + 8.0) / 7.0);
    let y = sea_level + h * (1.0 - peak) * (1.0 - peak);

    var out: VsOut;
    out.rel = vec3<f32>(rel_xz.x, y - camera_y, rel_xz.y);
    out.clip = frame.view_proj * vec4<f32>(out.rel, 1.0);
    out.world_xz = world;
    out.shallow = saturate(depth / 32.0 + (40.0 - sea_level) / 32.0);
    let slope = vec3<f32>(-wave_cos(u2) * sea.wave.z, 10.0, -wave_cos(v) * sea.wave.w);
    let flat = min(peak - saturate(1.25 - 0.00125 * length(out.rel)) + 1.0, 1.0);
    out.normal = normalize(mix(slope, vec3<f32>(0.0, 1.0, 0.0), flat));
    let t = sea.misc.x;
    out.uv_a = vec4<f32>(
        scrolled(world_origin, local, 0.002, t * 0.004),
        scrolled(world_origin, local, 0.02, t * 0.003),
    );
    let height = abs(camera_y - sea_level);
    out.uv_b = vec4<f32>(
        scrolled(world_origin, local, 0.0666667, t * 0.002),
        saturate(1.4285714 - d / (700.0 * (1.0 + 0.02 * height))) * 1.6,
        saturate(1.75 - d / (20.0 * (1.0 + height / 30.0))) * 1.2,
    );
    out.crest_a = vec4<f32>(
        scrolled(world_origin, local, 0.002 * 50.1, t * 0.004 * 50.1),
        scrolled(world_origin, local, 0.02 * 10.2, t * 0.003 * 10.2),
    );
    out.crest_b = scrolled(world_origin, local, 0.0666667 * 0.11, t * 0.002 * 0.11);
    return out;
}

// A tangent-space normal stored in green and alpha.
fn normal_xy(t: texture_2d<f32>, uv: vec2<f32>) -> vec2<f32> {
    let c = textureSample(t, repeat_sampler, uv);
    return vec2<f32>(c.g, c.a) * 2.0 - 1.0;
}

// View depth of the opaque scene at a texel; a large number where nothing was drawn.
fn scene_z(texel: vec2<i32>) -> f32 {
    let size = vec2<i32>(textureDimensions(scene_depth));
    let d = textureLoad(scene_depth, clamp(texel, vec2<i32>(0), size - 1), 0);
    return select(1e6, sea.forward.w / d, d > 0.0);
}

fn texel_of(uv: vec2<f32>) -> vec2<i32> {
    return vec2<i32>(floor(uv * vec2<f32>(textureDimensions(scene_depth))));
}

// Lowest view depth of a five-tap cross around `uv`; w: 1 when a tap saw nothing (sky).
fn cross_min(uv: vec2<f32>) -> vec2<f32> {
    let c = texel_of(uv);
    var m = 1e6;
    var sky = 0.0;
    for (var k = 0; k < 5; k++) {
        var o = vec2<i32>(0, 0);
        if k == 1 { o = vec2<i32>(1, 0); }
        if k == 2 { o = vec2<i32>(-1, 0); }
        if k == 3 { o = vec2<i32>(0, 1); }
        if k == 4 { o = vec2<i32>(0, -1); }
        let z = scene_z(c + o);
        if z >= 1e6 {
            sky = 1.0;
        }
        m = min(m, z);
    }
    return vec2<f32>(m, sky);
}

fn scene_rgb(uv: vec2<f32>) -> vec3<f32> {
    return textureSampleLevel(scene_color, scene_sampler, uv, 0.0).rgb;
}

// Screen position and reversed-Z depth of a camera-relative point.
fn project(p: vec3<f32>) -> vec3<f32> {
    let c = frame.view_proj * vec4<f32>(p, 1.0);
    let ndc = c.xyz / max(c.w, 1e-6);
    return vec3<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5, ndc.z);
}

// One screen-space reflection ray (the first pass of PSWater): `fallback` where it misses.
fn reflect_screen(p: vec3<f32>, r: vec3<f32>, fallback: vec3<f32>) -> vec3<f32> {
    let max_distance = sea.ssr0.y;
    let rz = dot(r, sea.forward.xyz);
    let pz = dot(p, sea.forward.xyz);
    if rz <= 0.0 || max_distance - pz <= 0.0 {
        return fallback;
    }
    let s = project(p);
    let e = project(p + r * ((max_distance - pz) / rz));
    var delta = e.xy - s.xy;
    // Clip the ray to the screen.
    var t = 1.0;
    if delta.x > 0.0 { t = min(t, (1.0 - s.x) / delta.x); }
    if delta.x < 0.0 { t = min(t, s.x / -delta.x); }
    if delta.y > 0.0 { t = min(t, (1.0 - s.y) / delta.y); }
    if delta.y < 0.0 { t = min(t, s.y / -delta.y); }
    t = max(t, 0.0);
    delta *= t;
    let depth_span = (e.z - s.z) * t;
    let tolerance = abs(depth_span) * 0.125;
    let near_fade = min(0.75 * max_distance, 100.0);
    for (var k = 0; k < 8; k++) {
        let f = 0.0125 + 0.125 * f32(k);
        let uv = s.xy + delta * f;
        let ray_depth = s.z + depth_span * f;
        let texel = texel_of(uv);
        let size = vec2<i32>(textureDimensions(scene_depth));
        let scene = textureLoad(scene_depth, clamp(texel, vec2<i32>(0), size - 1), 0);
        if scene > 0.0 && abs(ray_depth - scene) < tolerance {
            let border = max(abs(uv.x - 0.5), abs(uv.y - 0.5)) * 2.0;
            let edge = 1.0 - min(pow(border, sea.ssr1.x), 1.0);
            let z = sea.forward.w / scene;
            let far = 1.0 - saturate((z - near_fade) / (max_distance - near_fade));
            let fade = edge * pow(far, sea.ssr1.y);
            return mix(fallback, scene_rgb(uv), fade);
        }
    }
    return fallback;
}

// Caps a colour at 1 in exposed units, as the engine's saturate on light values that carry
// the exposure.
fn cap_exposed(c: vec3<f32>) -> vec3<f32> {
    let e = select(1.0, exposure.exposure, exposure.exposure > 0.0);
    return min(c * e, vec3<f32>(1.0)) / e;
}

@fragment
fn fs_sea(in: VsOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    // Front faces look up: the back face is the surface seen from below.
    let below = !front;
    let distance = length(in.rel);
    let view = -in.rel / (distance + 1e-4);
    let view_z = max(dot(in.rel, sea.forward.xyz), sea.forward.w);
    let x = sea.cwp4.x;

    // Normal (section 4.2).
    let s = 0.8 * x + 0.4;
    let w6 = s * sea.cwp0 * in.uv_b.z;
    let w7 = s * sea.cwp1 * in.uv_b.w;
    let n_a = normal_xy(wave_normals, in.uv_a.xy);
    let n_b = normal_xy(wave_normals, in.uv_a.zw);
    let fine = n_b * w6.x
        + normal_xy(wave_normals, in.uv_a.zw + 0.25) * w6.y
        + normal_xy(wave_normals, in.uv_a.zw + 0.5) * w6.z
        + normal_xy(wave_normals, in.uv_a.zw + 0.75) * w6.w
        + normal_xy(detail_normals, in.uv_b.xy) * w7.x
        + normal_xy(detail_normals, in.uv_b.xy + 0.25) * w7.y
        + normal_xy(detail_normals, in.uv_b.xy + 0.5) * w7.z
        + normal_xy(detail_normals, in.uv_b.xy + 0.75) * w7.w;
    let a = n_a + n_b;
    let steep = inverseSqrt(dot(2.0 * a, 2.0 * a) + 1.0);
    let tn = normalize(vec3<f32>(1.8 * a + fine, sea.cwp3.y));
    let nv = normalize(in.normal);
    let tangent = normalize(cross(vec3<f32>(1.0, 0.0, 0.0), nv));
    let bitangent = cross(nv, tangent);
    let n = normalize(-tangent * tn.x + bitangent * tn.y + nv * tn.z);

    // Fresnel.
    var fresnel = 0.02 + 0.98 * pow(1.0 - max(dot(n, view), 0.0), 5.0);
    if below {
        fresnel = min(pow(1.512146 * (1.0 - max(dot(-n, view), 0.0)), 4.0), 1.0);
    }

    // Sky reflection.
    let r = reflect(-view, n);
    let rxz = r.xz;
    let sky_uv = rxz * dot(rxz, rxz) * 0.45 + 0.5;
    let lower = textureSample(sky_lower, clamp_sampler, sky_uv).rgb;
    let upper = textureSample(sky_upper, clamp_sampler, sky_uv).rgb;
    var reflection = mix(lower, upper, sea.env.w) * sea.env.rgb;
    if below {
        reflection = sea.water_fog.rgb;
    }
    // Screen-space reflection.
    let n_ssr = normalize(mix(nv, n, sea.ssr0.w));
    let r_ssr = reflect(-view, n_ssr);
    let enabled = (saturate(-100.0 * r_ssr.y) + select(1.0, 0.0, below)) * sea.ssr0.x;
    if enabled > 0.001 {
        let weight = saturate((dot(r_ssr, sea.forward.xyz) - 0.2) * 5.0);
        if weight >= 0.01 {
            let traced = reflect_screen(in.rel, r_ssr, reflection);
            reflection = mix(reflection, traced, weight * sea.ssr0.x);
        }
    }

    // Refraction (section 4.3).
    let uv_s = in.clip.xy / vec2<f32>(textureDimensions(scene_depth));
    let z_s = scene_z(texel_of(uv_s));
    let thick = z_s - view_z;
    let coef = clamp(10.0 * sea.misc.w / view_z, 0.1, 1.0);
    let border = max(abs(uv_s.x - 0.5), abs(uv_s.y - 0.5));
    var k = mix(sea.ap0.x, sea.ap0.y, saturate(thick * sea.ap0.z)) * coef;
    k *= saturate((thick + 0.01) * 100000.0) * saturate((0.5 - border) * 10.0);
    var offset = saturate(thick * 5.0) * k;
    var refracted = scene_rgb(uv_s);
    var z_used = z_s;
    if offset >= 0.001 {
        let shift = vec2<f32>(n.x, n.z);
        let first = cross_min(uv_s + shift * offset);
        let thick1 = first.x - view_z;
        let k1 = mix(sea.ap0.x, sea.ap0.y, saturate(thick1 * sea.ap0.z)) * coef
            * saturate((thick1 + 0.01) * 100000.0);
        offset = min(offset, k1);
        let uv2 = uv_s + shift * offset;
        let second = cross_min(uv2);
        if second.x - view_z >= 0.0 {
            if second.y < 0.5 {
                refracted = scene_rgb(uv2);
            }
            z_used = second.x;
        }
    }
    let water_color = sea.water_fog.rgb;
    let behind = max(z_used - view_z, 0.0);
    if !below {
        // Our post pass fogs only the air: fog the refracted underwater scene here with the
        // water fog its own shaders would have applied (render-atmosphere.md section 2).
        let under = select(behind * distance / view_z, 10000.0, z_used >= 1e6);
        let transmit = exp(-under * sea.water_fog.w);
        let dir_y = -view.y;
        let g = sea.water_gradient;
        var shade = g.y + dir_y * (g.z - g.y);
        if dir_y < 0.0 {
            shade = g.x + (1.0 + dir_y) * (1.0 + dir_y) * (g.y - g.x);
        }
        refracted = refracted * transmit + shade * water_color * (1.0 - transmit);
    }
    let tint = (1.0 - exp(-0.1 * min(behind, 10000.0))) * max(1.0 - 4.0 * abs(view.y), 0.0);
    refracted = mix(refracted, water_color, tint);

    // Foam along shores and around objects.
    var edge = 1.0;
    var shore_foam = 0.0;
    if sea.ap1.x > 0.01 {
        let rel = behind * max(0.2, abs(sea.misc.y)) / view_z;
        let foam_uv = vec2<f32>(n.x, n.z) * sea.ap1.y + in.world_xz * sea.ap1.z + sea.ap2.xy;
        let f = textureSample(foam_texture, repeat_sampler, foam_uv).r;
        let fade = (rel - 0.1) * sea.ap1.w;
        let foam_tex = saturate(f - (saturate(fade * 0.5) + 0.2));
        edge = saturate((rel - 0.05) * 10.0);
        let fade_out = 1.0 - saturate(fade);
        shore_foam = saturate(fade_out * fade_out * foam_tex * edge * sea.ap1.x);
    }
    refracted = mix(refracted, water_color, sea.ap2.w);

    // Foam on wave crests in stormy, shallow water.
    let c = saturate((steep - 0.7) * 5.0);
    let crest_shape = (1.0 - c) * (1.5 - c);
    let shallow2 = in.shallow * in.shallow;
    let crest_texture = saturate(
        textureSample(foam_texture, repeat_sampler, in.crest_a.xy).r
            + textureSample(foam_texture, repeat_sampler, in.crest_a.zw).r
            + textureSample(foam_texture, repeat_sampler, in.crest_b).r
            - 2.4,
    );
    let crest = crest_shape * (shallow2 * shallow2 + 0.1) * saturate(0.5 - x) * crest_texture;
    let foam = max(shore_foam, crest);

    var color = mix(refracted, reflection, fresnel);

    // Sun glint.
    let n_spec = normalize(mix(n, normalize(vec3<f32>(n.x, 0.6, n.z)), sea.ap2.z));
    let glint = pow(saturate(dot(reflect(sea.light.xyz, n_spec), view)), sea.cwp3.w);
    var specular = cap_exposed(fresnel * glint * sea.specular.rgb * sea.cwp3.z);

    color = mix(color, cap_exposed(sea.wave_color.rgb), foam);
    specular *= (1.0 - foam) * edge;

    // Output (section 4.4): premultiplied, alpha fades at the shoreline. Above water the
    // light extinction split cancels; below, the post pass fogs.
    let alpha = edge;
    let split = 0.6 * x + 0.2;
    let rgb = color * (1.0 - edge + edge * split) * alpha + color * (1.0 - split) * edge + specular;
    return vec4<f32>(rgb, alpha);
}
