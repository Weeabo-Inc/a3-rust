// ODOL model sections: instanced, camera-relative (ADR 0003), one material per draw.
//
// Skinned models (a Skeleton, vertices with bone weights) draw through the `*_skinned` entry
// points, which blend up to four bone matrices from the palette buffer (group 2) and transform
// the position, normal and tangent by the blended matrix before the instance transform.
//
// Follows docs/re/render-materials.md (the engine's own shaders). Families (params.x):
//   0 Basic  - Normal / Detail etc.: colour map, lit per vertex normal
//   1 Super  - Super, NormalMap*, Skin: colour, macro, detail, normal, smdi, AS, fresnel, env
//   2 Multi  - four colour/dtsmdi/normal layers blended by a mask, macro, AS
//   3 Tree   - TreeAdv*, Grass: colour x _mca, wrapped lighting; alpha-tested, two-sided
//   4 Glass  - colour, fresnel, env; alpha-blended
// Texture bindings t0..t14 follow material.rs `Slot::binding`: Multi uses all 15, the other
// families t0..t7 (t0 colour, t1 normal, t2 smdi, t3 AS, t4 macro, t5 detail, t6 fresnel, t7 env).

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
    // The lighting table's hemisphere ambient (AE, AmbientMid, GE); w = 0 when not set.
    ambient_sky: vec4<f32>,
    ambient_mid: vec4<f32>,
    ambient_ground: vec4<f32>,
}

const FAMILY_BASIC: u32 = 0u;
const FAMILY_SUPER: u32 = 1u;
const FAMILY_MULTI: u32 = 2u;
const FAMILY_TREE: u32 = 3u;
const FAMILY_GLASS: u32 = 4u;
const ALPHA_TEST: u32 = 1u;
const ALPHA_BLEND: u32 = 2u;

struct Material {
    ambient: vec4<f32>,
    diffuse: vec4<f32>,
    emissive: vec4<f32>,
    // rgb: specular colour, w: specular power.
    specular: vec4<f32>,
    // x: family, y: alpha mode (0 opaque, 1 test, 2 blend), z, w: unused.
    params: vec4<u32>,
    // Per binding two rows of the UV transform; w of the first row selects the UV set.
    uv: array<vec4<f32>, 30>,
}

// Must match a3-render's shadow.rs (ShadowUniforms).
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
@group(1) @binding(0) var<uniform> material: Material;
@group(1) @binding(1) var t0: texture_2d<f32>;
@group(1) @binding(2) var t1: texture_2d<f32>;
@group(1) @binding(3) var t2: texture_2d<f32>;
@group(1) @binding(4) var t3: texture_2d<f32>;
@group(1) @binding(5) var t4: texture_2d<f32>;
@group(1) @binding(6) var t5: texture_2d<f32>;
@group(1) @binding(7) var t6: texture_2d<f32>;
@group(1) @binding(8) var t7: texture_2d<f32>;
@group(1) @binding(9) var t8: texture_2d<f32>;
@group(1) @binding(10) var t9: texture_2d<f32>;
@group(1) @binding(11) var t10: texture_2d<f32>;
@group(1) @binding(12) var t11: texture_2d<f32>;
@group(1) @binding(13) var t12: texture_2d<f32>;
@group(1) @binding(14) var t13: texture_2d<f32>;
@group(1) @binding(15) var t14: texture_2d<f32>;
@group(1) @binding(16) var s_material: sampler;
// Bone palette: the frame's skinning matrices, one block per instance. An instance's block
// starts at its `palette` base; slot `b` of the block holds Skeleton bone `b`'s skinning matrix
// (rest pose -> posed model), and the block is padded with the identity, the rest pose that
// vertices without influences are bound to. Slot 0 of the buffer is the shared identity block
// that unposed instances read.
@group(2) @binding(0) var<storage, read> palette: array<mat4x4<f32>>;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv0: vec2<f32>,
    @location(3) uv1: vec2<f32>,
    @location(4) tangent: vec4<f32>,
}

// Rows of the camera-relative model matrix: world = (dot(row0, p), dot(row1, p), dot(row2, p)).
struct InstanceIn {
    @location(5) row0: vec4<f32>,
    @location(6) row1: vec4<f32>,
    @location(7) row2: vec4<f32>,
    // x: fade, 1 = fully drawn; objects near the drop-out distance dither in; yzw unused.
    @location(8) params: vec4<f32>,
    // Palette slot the instance's bone matrices start at (0 = the identity block, the rest pose).
    @location(11) palette: u32,
}

// A skinned vertex's bone influences: up to four palette slots (Skeleton bones), with the
// weights summing to 1.
struct SkinIn {
    @location(9) bones: vec4<u32>,
    @location(10) weights: vec4<f32>,
}

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) relative: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) tangent: vec4<f32>,
    @location(3) uv0: vec2<f32>,
    @location(4) uv1: vec2<f32>,
    @location(5) @interpolate(flat) fade: f32,
}

fn transform_point(i: InstanceIn, p: vec3<f32>) -> vec3<f32> {
    let h = vec4<f32>(p, 1.0);
    return vec3<f32>(dot(i.row0, h), dot(i.row1, h), dot(i.row2, h));
}

fn transform_vector(i: InstanceIn, v: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(dot(i.row0.xyz, v), dot(i.row1.xyz, v), dot(i.row2.xyz, v));
}

// The vertex's skinning matrix: its bones' palette matrices blended by weight. Weights sum to
// 1, so the blend of the affine matrices is the affine matrix of the blend.
fn skin_matrix(s: SkinIn, base: u32) -> mat4x4<f32> {
    return s.weights.x * palette[base + s.bones.x]
        + s.weights.y * palette[base + s.bones.y]
        + s.weights.z * palette[base + s.bones.z]
        + s.weights.w * palette[base + s.bones.w];
}

fn skin_point(m: mat4x4<f32>, p: vec3<f32>) -> vec3<f32> {
    return (m * vec4<f32>(p, 1.0)).xyz;
}

// Directions take the blended matrix's rotation (the engine skins normals with the bone matrix
// itself, then renormalises). A blend that collapses the vertex — its bones are hidden — would
// give the zero vector; the vertex is degenerate there, so keep the rest direction.
fn skin_vector(m: mat4x4<f32>, v: vec3<f32>) -> vec3<f32> {
    let skinned = (m * vec4<f32>(v, 0.0)).xyz;
    if dot(skinned, skinned) < 1e-12 {
        return v;
    }
    return skinned;
}

// The vertex's output, from its model-space position, normal and tangent.
fn vertex_out(position: vec3<f32>, normal: vec3<f32>, tangent: vec4<f32>, uv0: vec2<f32>, uv1: vec2<f32>, i: InstanceIn) -> VertexOut {
    let relative = transform_point(i, position);
    var out: VertexOut;
    out.clip = frame.view_proj * vec4<f32>(relative, 1.0);
    out.relative = relative;
    out.normal = transform_vector(i, normal);
    out.tangent = vec4<f32>(transform_vector(i, tangent.xyz), tangent.w);
    out.uv0 = uv0;
    out.uv1 = uv1;
    out.fade = i.params.x;
    return out;
}

@vertex
fn vs_main(v: VertexIn, i: InstanceIn) -> VertexOut {
    return vertex_out(v.position, v.normal, v.tangent, v.uv0, v.uv1, i);
}

@vertex
fn vs_main_skinned(v: VertexIn, i: InstanceIn, s: SkinIn) -> VertexOut {
    let m = skin_matrix(s, i.palette);
    return vertex_out(
        skin_point(m, v.position),
        skin_vector(m, v.normal),
        vec4<f32>(skin_vector(m, v.tangent.xyz), v.tangent.w),
        v.uv0,
        v.uv1,
        i,
    );
}

struct ShadowOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv0: vec2<f32>,
}

// Depth-only pass into a sun shadow cascade; `frame.view_proj` is the cascade's light matrix.
@vertex
fn vs_shadow(v: VertexIn, i: InstanceIn) -> ShadowOut {
    var out: ShadowOut;
    out.clip = frame.view_proj * vec4<f32>(transform_point(i, v.position), 1.0);
    out.uv0 = v.uv0;
    return out;
}

@vertex
fn vs_shadow_skinned(v: VertexIn, i: InstanceIn, s: SkinIn) -> ShadowOut {
    let p = skin_point(skin_matrix(s, i.palette), v.position);
    var out: ShadowOut;
    out.clip = frame.view_proj * vec4<f32>(transform_point(i, p), 1.0);
    out.uv0 = v.uv0;
    return out;
}

// Alpha-tested casters (foliage, fences) drop their transparent texels.
@fragment
fn fs_shadow_alpha_test(in: ShadowOut) {
    let r0 = material.uv[0];
    let r1 = material.uv[1];
    let h = vec3<f32>(in.uv0, 1.0);
    let uv = vec2<f32>(dot(r0.xyz, h), dot(r1.xyz, h));
    if textureSample(t0, s_material, uv).a < 0.5 {
        discard;
    }
}

// 1 = fully lit, 0 = in shadow. The same lookup as a3-render's mesh.wgsl.
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

fn slot_uv(binding: u32, in: VertexOut) -> vec2<f32> {
    let r0 = material.uv[binding * 2u];
    let r1 = material.uv[binding * 2u + 1u];
    let base = select(in.uv0, in.uv1, r0.w > 0.5);
    let h = vec3<f32>(base, 1.0);
    return vec2<f32>(dot(r0.xyz, h), dot(r1.xyz, h));
}

// Tangent-space normal from a normal-map texel, as the engine decodes it: works for `_nohq`
// (X stored as 1 - alpha, red 0) and for plain RGB (alpha 1).
fn decode_normal(c: vec4<f32>) -> vec3<f32> {
    return vec3<f32>(2.0 * (c.r - c.a) + 1.0, 2.0 * c.g - 1.0, 2.0 * c.b - 1.0);
}

// Ambient occlusion from an `_as` texel: (ambient, sun).
fn ambient_shadow(as_: vec4<f32>) -> vec2<f32> {
    let ambient = mix(pow(as_.a, 2.2), as_.g, as_.r);
    let sun = mix(as_.g, as_.b, as_.r);
    return vec2<f32>(ambient, sun);
}

struct Surface {
    albedo: vec3<f32>,
    alpha: f32,
    // Tangent-space normal.
    normal: vec3<f32>,
    // Specular intensity (smdi green) and gloss (smdi blue).
    specular: f32,
    gloss: f32,
    ao_ambient: f32,
    ao_sun: f32,
}

// Super, NormalMap*, Glass, Skin, Normal: colour, macro, detail, normal, smdi, AS.
fn sample_super(in: VertexOut) -> Surface {
    var s: Surface;
    let d = textureSample(t0, s_material, slot_uv(0u, in));
    let macro_map = textureSample(t4, s_material, slot_uv(4u, in));
    let detail = textureSample(t5, s_material, slot_uv(5u, in));
    s.albedo = mix(d.rgb, macro_map.rgb, macro_map.a) * 2.0 * detail.rgb;
    s.alpha = d.a;
    s.normal = decode_normal(textureSample(t1, s_material, slot_uv(1u, in)));
    let smdi = textureSample(t2, s_material, slot_uv(2u, in));
    s.specular = smdi.g;
    s.gloss = smdi.b;
    let ao = ambient_shadow(textureSample(t3, s_material, slot_uv(3u, in)));
    s.ao_ambient = ao.x;
    s.ao_sun = ao.y;
    return s;
}

// TreeAdv / TreeAdvTrunk: colour times the `_mca` crown map (x 2^2.2), AO in its alpha.
fn sample_tree(in: VertexOut) -> Surface {
    var s: Surface;
    let d = textureSample(t0, s_material, slot_uv(0u, in));
    let mca = textureSample(t4, s_material, slot_uv(4u, in));
    s.albedo = d.rgb * mca.rgb * 4.5947;
    s.alpha = d.a;
    s.normal = decode_normal(textureSample(t1, s_material, slot_uv(1u, in)));
    s.specular = 0.0;
    s.gloss = 0.0;
    s.ao_ambient = mca.a;
    s.ao_sun = 1.0;
    return s;
}

// Multi: four layers blended by the mask (red, green, blue weight layers 1, 2, 3).
fn sample_multi(in: VertexOut) -> Surface {
    var s: Surface;
    let m = textureSample(t5, s_material, slot_uv(5u, in));
    let uv0 = slot_uv(0u, in);
    let uv1 = slot_uv(6u, in);
    let uv2 = slot_uv(7u, in);
    let uv3 = slot_uv(8u, in);
    var c = textureSample(t0, s_material, uv0).rgb;
    c = mix(c, textureSample(t6, s_material, uv1).rgb, m.r);
    c = mix(c, textureSample(t7, s_material, uv2).rgb, m.g);
    c = mix(c, textureSample(t8, s_material, uv3).rgb, m.b);
    // The same blend of each layer's average colour (its smallest mip).
    var avg = textureSampleLevel(t0, s_material, uv0, 20.0).rgb;
    avg = mix(avg, textureSampleLevel(t6, s_material, uv1, 20.0).rgb, m.r);
    avg = mix(avg, textureSampleLevel(t7, s_material, uv2, 20.0).rgb, m.g);
    avg = mix(avg, textureSampleLevel(t8, s_material, uv3, 20.0).rgb, m.b);
    let macro_map = textureSample(t4, s_material, slot_uv(4u, in));
    c = mix(c, c * clamp(macro_map.rgb / max(avg, vec3<f32>(1e-3)), vec3<f32>(0.0), vec3<f32>(2.0)), macro_map.a);
    var ds = textureSample(t2, s_material, slot_uv(2u, in));
    ds = mix(ds, textureSample(t12, s_material, slot_uv(12u, in)), m.r);
    ds = mix(ds, textureSample(t13, s_material, slot_uv(13u, in)), m.g);
    ds = mix(ds, textureSample(t14, s_material, slot_uv(14u, in)), m.b);
    var n = textureSample(t1, s_material, slot_uv(1u, in));
    n = mix(n, textureSample(t9, s_material, slot_uv(9u, in)), m.r);
    n = mix(n, textureSample(t10, s_material, slot_uv(10u, in)), m.g);
    n = mix(n, textureSample(t11, s_material, slot_uv(11u, in)), m.b);
    s.albedo = 2.0 * ds.r * c;
    s.alpha = 1.0;
    s.normal = decode_normal(n);
    s.specular = ds.g;
    s.gloss = ds.b;
    let ao = ambient_shadow(textureSample(t3, s_material, slot_uv(3u, in)));
    s.ao_ambient = ao.x;
    s.ao_sun = ao.y;
    return s;
}

// The hemisphere ambient on the world normal's y (docs/re/render-materials.md §3.1):
// lerp(AmbientMid, AE, y) above the horizon, lerp(GE, AmbientMid, 1 + y) below, from the
// lighting table's ambient, ambientMid and groundReflection. Without a lighting table (model
// viewer, test scene) the sky colours stand in.
fn hemisphere(y: f32) -> vec3<f32> {
    if frame.ambient_sky.w > 0.0 {
        if y > 0.0 {
            return mix(frame.ambient_mid.rgb, frame.ambient_sky.rgb, saturate(y));
        }
        return mix(frame.ambient_ground.rgb, frame.ambient_mid.rgb, saturate(1.0 + y));
    }
    let level = frame.sun_color.w;
    let sky = frame.sky_zenith.rgb * level * 1.6;
    let mid = frame.sky_horizon.rgb * level;
    let ground = frame.sky_horizon.rgb * level * 0.45;
    if y > 0.0 {
        return mix(mid, sky, saturate(y));
    }
    return mix(ground, mid, saturate(1.0 + y));
}

// Screen-door fade: a 4x4 ordered-dither threshold in 1/16 steps.
fn dither_threshold(pixel: vec2<f32>) -> f32 {
    let bayer = array<f32, 16>(0.0, 8.0, 2.0, 10.0, 12.0, 4.0, 14.0, 6.0, 3.0, 11.0, 1.0, 9.0, 15.0, 7.0, 13.0, 5.0);
    let p = vec2<u32>(pixel) % vec2<u32>(4u);
    return (bayer[p.y * 4u + p.x] + 0.5) / 16.0;
}

fn shade(in: VertexOut, front: bool) -> vec4<f32> {
    let family = material.params.x;
    let alpha_mode = material.params.y;
    var s: Surface;
    if family == FAMILY_MULTI {
        s = sample_multi(in);
    } else if family == FAMILY_TREE {
        s = sample_tree(in);
    } else {
        s = sample_super(in);
    }
    // Alpha tests use the texture alpha (the engine scales it by tree/VS constants not traced
    // yet); blending also applies the material's diffuse alpha.
    let alpha = select(s.alpha, s.alpha * material.diffuse.a, alpha_mode == ALPHA_BLEND);
    if in.fade < 1.0 && in.fade <= dither_threshold(in.clip.xy) {
        discard;
    }
    if alpha_mode == ALPHA_TEST && alpha < 0.5 {
        discard;
    }
    if alpha_mode == ALPHA_BLEND && alpha < 1.0 / 255.0 {
        discard;
    }

    // World normal. ODOL's S and T point along -dP/du and -dP/dv: +X of the map follows -S,
    // +Y (green up, Direct3D convention) follows T.
    var n = normalize(in.normal);
    if !front {
        n = -n;
    }
    var t = in.tangent.xyz - n * dot(n, in.tangent.xyz);
    let t_len = length(t);
    if family != FAMILY_BASIC && t_len > 1e-5 {
        t = t / t_len;
        let b = cross(n, t) * in.tangent.w;
        let nt = s.normal;
        let mapped = -t * nt.x + b * nt.y + n * nt.z;
        if dot(mapped, mapped) > 1e-8 {
            n = normalize(mapped);
        }
    }

    let l = frame.sun_dir.xyz;
    let v = normalize(-in.relative);
    let h = normalize(l + v);
    let n_dot_l = dot(n, l);
    let n_dot_h = dot(n, h);
    let geometric = select(-normalize(in.normal), normalize(in.normal), front);
    let sun_vis = saturate(s.ao_sun * sun_visibility(in.relative, geometric));
    let sun = frame.sun_color.rgb;

    if family == FAMILY_TREE {
        // Foliage is thin and translucent: wrapped diffuse, ambient on n.y * 0.5 + 0.5.
        let wrapped = saturate(n_dot_l * 0.5 + 0.5);
        let amb = hemisphere(n.y * 0.5 + 0.5) * material.ambient.rgb * s.ao_ambient;
        let sun_d = sun * material.diffuse.rgb * wrapped;
        return vec4<f32>(s.albedo * (amb + sun_d * sun_vis), 1.0);
    }

    let power = max(material.specular.w * s.gloss, 1.0);
    var spec = 0.0;
    if n_dot_l >= 0.0 && n_dot_h >= 0.0 {
        spec = min(pow(n_dot_h, power), 1.0);
    }
    var reflectivity = s.specular;
    var env = vec3<f32>(0.0);
    if family == FAMILY_SUPER || family == FAMILY_GLASS {
        let n_dot_v = saturate(dot(n, v));
        let fresnel = textureSampleLevel(t6, s_material, vec2<f32>(n_dot_v, 0.5), 0.0).a;
        reflectivity = select(s.specular * fresnel, fresnel, family == FAMILY_GLASS);
        let r = reflect(-v, n);
        let mip = 8.0 * pow(1.0 - saturate(power * 0.001), 10.0);
        let e = textureSampleLevel(t7, s_material, vec2<f32>(r.x * 0.5 + 0.5, -r.y * 0.5 + 0.5), mip);
        // GlassEnvColor x GlassMatSpecular are engine constants not traced yet; the sky
        // ambient and the material specular stand in.
        let tint = frame.sun_color.w * material.specular.rgb;
        env = e.rgb * exp2(4.0 * (1.0 - e.a)) * tint * reflectivity * 2.0;
    }

    var amb = hemisphere(n.y) * material.ambient.rgb + material.emissive.rgb;
    amb = amb * s.ao_ambient;
    let sun_d = sun * material.diffuse.rgb * max(n_dot_l, 0.0) * (1.0 - reflectivity);
    let sun_s = sun * material.specular.rgb * spec * reflectivity;
    // Premultiplied alpha, like the engine: colour scales with alpha, highlights do not.
    let a = select(1.0, alpha, alpha_mode == ALPHA_BLEND);
    let rgb = s.albedo * (amb + sun_d * sun_vis) * a + sun_s * sun_vis + env;
    return vec4<f32>(rgb, a);
}

@fragment
fn fs_main(in: VertexOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    return shade(in, front);
}
