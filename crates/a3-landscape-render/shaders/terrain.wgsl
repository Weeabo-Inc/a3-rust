// Terrain (CDLOD heightmap patches) and the placeholder sea. Positions are camera-relative
// (ADR 0003). Shading follows the engine's PSTerrainSNX (docs/re/render-terrain.md).

struct Frame {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    sun_dir: vec4<f32>,
    // rgb: sun colour, w: ambient level.
    sun_color: vec4<f32>,
    sky_zenith: vec4<f32>,
    // rgb: horizon/fog colour, w: fog density per metre.
    sky_horizon: vec4<f32>,
    viewport: vec4<f32>,
    params: vec4<f32>,
    // The lighting table's hemisphere ambient (AE, AmbientMid, GE); w = 0 when not set.
    ambient_sky: vec4<f32>,
    ambient_mid: vec4<f32>,
    ambient_ground: vec4<f32>,
}

struct Terrain {
    // Camera world position (x, y, z) and the sea level.
    camera: vec4<f32>,
    // Height cell edge (m), height samples per axis, world edge (m), land cell edge (m).
    grid: vec4<f32>,
    // Land cells per axis, tile count, sea extent (m), detail layers loaded (0 or 1).
    misc: vec4<f32>,
    // fullDetailDist, noDetailDist (m), terrainBlendMaxDarkenCoef, terrainBlendMaxBrightenCoef.
    detail: vec4<f32>,
    // Per LOD level: morph start (m), morph end (m), 1 / (end - start), unused.
    morph: array<vec4<f32>, 16>,
}

struct Tile {
    // u = u.x * x + u.y * z + u.z (world metres); likewise v.
    u: vec4<f32>,
    v: vec4<f32>,
    // x: texture array layer of the full-resolution satellite tile, or -1; y: mask loaded.
    slot: vec4<i32>,
}

@group(0) @binding(0) var<uniform> frame: Frame;
@group(1) @binding(0) var heights: texture_2d<f32>;
@group(1) @binding(1) var normals: texture_2d<f32>;
@group(1) @binding(2) var overview: texture_2d<f32>;
@group(1) @binding(3) var linear_clamp: sampler;
@group(1) @binding(4) var<uniform> terrain: Terrain;
// WRP material index of every land cell.
@group(1) @binding(5) var cell_materials: texture_2d<u32>;
@group(1) @binding(6) var<storage, read> tiles: array<Tile>;
@group(1) @binding(7) var near_tiles: texture_2d_array<f32>;
@group(1) @binding(8) var tile_sampler: sampler;
@group(1) @binding(9) var masks: texture_2d_array<f32>;
@group(1) @binding(10) var detail_color: texture_2d_array<f32>;
@group(1) @binding(11) var detail_normal: texture_2d_array<f32>;
@group(1) @binding(12) var repeat_sampler: sampler;
// Per material: tile, slot0 | slot1 << 16, slot2 | slot3 << 16, slot4 (0xFFFF = none).
@group(1) @binding(13) var<storage, read> materials: array<vec4<u32>>;
// Average sRGB colour of each detail colour texture (its smallest mip).
@group(1) @binding(14) var<storage, read> detail_average: array<vec4<f32>>;

const NONE: u32 = 65535u;

fn load_height(i: i32, j: i32) -> f32 {
    let last = i32(terrain.grid.y) - 1;
    return textureLoad(heights, vec2<i32>(clamp(i, 0, last), clamp(j, 0, last)), 0).r;
}

// Height at grid coordinates `g` (height cells), triangulated like the engine: each cell
// splits along the diagonal from (i + 1, j) to (i, j + 1). Mirrors `HeightField::sample`.
fn height_at(g: vec2<f32>) -> f32 {
    let base = floor(g);
    let f = g - base;
    let i = i32(base.x);
    let j = i32(base.y);
    let h00 = load_height(i, j);
    let h10 = load_height(i + 1, j);
    let h01 = load_height(i, j + 1);
    let h11 = load_height(i + 1, j + 1);
    if f.x + f.y <= 1.0 {
        return h00 + (h10 - h00) * f.x + (h01 - h00) * f.y;
    }
    return (h01 + h10 - h11) + (h11 - h01) * f.x + (h11 - h10) * f.y;
}

struct NodeIn {
    // Node south-west corner minus camera, metres (x, z).
    @location(1) rel_origin: vec2<f32>,
    // Node south-west corner in height cells.
    @location(2) grid_origin: vec2<u32>,
    // x: height cells per patch quad (2^level), y: level.
    @location(3) step_level: vec2<u32>,
}

struct TerrainOut {
    @builtin(position) clip: vec4<f32>,
    // World x, z in metres.
    @location(0) world_xz: vec2<f32>,
    // Camera-relative position.
    @location(1) rel: vec3<f32>,
    // Detail texture coordinates: 5 repeats per height cell (the engine's TexGen1/2 on the
    // integer grid index), counted from the node origin so they stay small.
    @location(2) detail_uv: vec2<f32>,
}

@vertex
fn vs_terrain(@location(0) local: vec2<u32>, node: NodeIn) -> TerrainOut {
    let cell = terrain.grid.x;
    let step = f32(node.step_level.x);
    let morph = terrain.morph[node.step_level.y];
    let origin = vec2<f32>(node.grid_origin);

    // Unmorphed vertex: exactly on a height sample.
    let local_f = vec2<f32>(local);
    let g0 = origin + local_f * step;
    let rel0 = node.rel_origin + local_f * step * cell;
    let h0 = load_height(i32(g0.x), i32(g0.y));
    let distance = length(vec3<f32>(rel0.x, h0 - terrain.camera.y, rel0.y));
    let k = clamp((distance - morph.x) * morph.z, 0.0, 1.0);

    // Morph odd vertices onto the parent level's grid.
    let odd = vec2<f32>(local % vec2<u32>(2u));
    let morphed = local_f - odd * k;
    let g = origin + morphed * step;
    let h = height_at(g);
    let rel_xz = node.rel_origin + morphed * step * cell;

    var out: TerrainOut;
    out.rel = vec3<f32>(rel_xz.x, h - terrain.camera.y, rel_xz.y);
    out.clip = frame.view_proj * vec4<f32>(out.rel, 1.0);
    out.world_xz = g * cell;
    out.detail_uv = morphed * step * 5.0;
    return out;
}

fn overview_color(world_xz: vec2<f32>) -> vec3<f32> {
    let world = terrain.grid.z;
    let uv = vec2<f32>(world_xz.x / world, 1.0 - world_xz.y / world);
    return textureSample(overview, linear_clamp, uv).rgb;
}

fn surface_normal(world_xz: vec2<f32>) -> vec3<f32> {
    let size = terrain.grid.y;
    let uv = (world_xz / terrain.grid.x + 0.5) / size;
    return normalize(textureSample(normals, linear_clamp, uv).xyz);
}

fn lit(albedo: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    let diffuse = max(dot(n, frame.sun_dir.xyz), 0.0);
    return albedo * (frame.sun_color.rgb * diffuse + ambient(n.y));
}

// Terrain ambient: the engine's hemisphere ambient (AE, AmbientMid, GE on the normal's y,
// docs/re/render-terrain.md §3.6 and render-materials.md §3.1). Without a lighting table the
// sky colours stand in: sky from above, a darker bounce from below.
fn ambient(y: f32) -> vec3<f32> {
    if frame.ambient_sky.w > 0.0 {
        if y > 0.0 {
            return mix(frame.ambient_mid.rgb, frame.ambient_sky.rgb, saturate(y));
        }
        return mix(frame.ambient_ground.rgb, frame.ambient_mid.rgb, saturate(1.0 + y));
    }
    let sky = mix(frame.sky_horizon.rgb, frame.sky_zenith.rgb, 0.5) * frame.sun_color.w * 2.0;
    return mix(sky * 0.5, sky, y * 0.5 + 0.5);
}

// The layer weights of PSTerrainSNX: each present layer (`present`, 0 or 1 per slot) paints
// over the earlier ones with coverage saturate(3 * channel); slot 1 follows red, 2 green,
// 3 blue, 4 blue where alpha is low. Mirrors `detail::layer_weights`.
fn layer_weights(m: vec4<f32>, l0: f32, l: vec4<f32>) -> array<f32, 5> {
    var w = array<f32, 5>(saturate(3.0 * l0), 0.0, 0.0, 0.0, 0.0);
    var r = min(1.0 - l0, 1.0);
    let channels = vec4<f32>(m.r, m.g, m.b, m.b * saturate(2.0 * (1.0 - m.a)));
    for (var k = 1; k < 5; k++) {
        let t = max(r, channels[k - 1]) * l[k - 1];
        r = min(r, 1.0 - l[k - 1]);
        let s = saturate(3.0 * t);
        for (var j = 0; j < k; j++) {
            w[j] *= 1.0 - s;
        }
        w[k] = s;
    }
    return w;
}

fn to_gamma(c: vec3<f32>) -> vec3<f32> {
    return pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2));
}

fn to_linear(c: vec3<f32>) -> vec3<f32> {
    return pow(max(c, vec3<f32>(0.0)), vec3<f32>(2.2));
}

@fragment
fn fs_terrain(in: TerrainOut) -> @location(0) vec4<f32> {
    let n0 = surface_normal(in.world_xz);
    let far = overview_color(in.world_xz);
    // Gradients from continuous coordinates, so tile and node borders do not change the mip.
    let dx = dpdx(in.world_xz);
    let dy = dpdy(in.world_xz);
    let ddx = dpdx(in.detail_uv);
    let ddy = dpdy(in.detail_uv);

    let cells = i32(terrain.misc.x);
    let cell = clamp(vec2<i32>(floor(in.world_xz / terrain.grid.w)), vec2<i32>(0), vec2<i32>(cells - 1));
    let material_index = min(textureLoad(cell_materials, cell, 0).r, arrayLength(&materials) - 1u);
    let material = materials[material_index];
    let tile_index = material.x;
    if tile_index == NONE || tile_index >= arrayLength(&tiles) {
        return vec4<f32>(lit(far, n0), 1.0);
    }
    let tile = tiles[tile_index];
    if tile.slot.x < 0 {
        return vec4<f32>(lit(far, n0), 1.0);
    }
    let uv = vec2<f32>(dot(tile.u.xy, in.world_xz) + tile.u.z, dot(tile.v.xy, in.world_xz) + tile.v.z);
    let uv_dx = vec2<f32>(dot(tile.u.xy, dx), dot(tile.v.xy, dx));
    let uv_dy = vec2<f32>(dot(tile.u.xy, dy), dot(tile.v.xy, dy));
    // Satellite colour S; the base colour far away (stage 2 is constant grey: base = S).
    let satellite = textureSampleGrad(near_tiles, tile_sampler, uv, tile.slot.x, uv_dx, uv_dy).rgb;

    // Detail weight: 1 up to fullDetailDist, 0 from noDetailDist.
    let distance = length(in.rel);
    let detail_weight = saturate((terrain.detail.y - distance) / (terrain.detail.y - terrain.detail.x));
    if detail_weight <= 0.01 || tile.slot.y == 0 || terrain.misc.w == 0.0 {
        return vec4<f32>(lit(satellite, n0), 1.0);
    }

    let mask = textureSampleGrad(masks, tile_sampler, uv, tile.slot.x, uv_dx, uv_dy);
    let ids = array<u32, 5>(
        material.y & 0xffffu, material.y >> 16u, material.z & 0xffffu, material.z >> 16u, material.w & 0xffffu,
    );
    var present = vec4<f32>(0.0);
    for (var k = 1; k < 5; k++) {
        present[k - 1] = select(0.0, 1.0, ids[k] != NONE);
    }
    let weights = layer_weights(mask, select(0.0, 1.0, ids[0] != NONE), present);

    var d = vec3<f32>(0.0);
    var a = vec3<f32>(0.0);
    var detail_n = vec3<f32>(0.0);
    for (var k = 0; k < 5; k++) {
        let w = weights[k];
        if w > 0.001 && ids[k] != NONE {
            let layer = i32(ids[k]);
            // Detail colour sampled linear (sRGB texture); the blend works on stored values.
            d += w * to_gamma(textureSampleGrad(detail_color, repeat_sampler, in.detail_uv, layer, ddx, ddy).rgb);
            a += w * detail_average[ids[k]].rgb;
            detail_n += w * (textureSampleGrad(detail_normal, repeat_sampler, in.detail_uv, layer, ddx, ddy).rgb * 2.0 - 1.0);
        }
    }

    // PSC_TerrainBlend = (10, maxDarken, maxBrighten): the satellite relative to "satellite +
    // average detail" darkens or brightens the detail beyond the configured limits.
    let s = to_gamma(satellite);
    let kk = clamp(1.0 / (s + a + vec3<f32>(0.001)), vec3<f32>(1.0), vec3<f32>(10.0));
    let ks = kk * s;
    let lo = max(ks, vec3<f32>(terrain.detail.z));
    let hi = min(ks, vec3<f32>(terrain.detail.w));
    let res = lo * d * (1.0 - d) + d * (1.0 - (1.0 - d) * (1.0 - hi));
    let albedo = mix(satellite, to_linear(res), detail_weight);

    // Detail normal in the terrain's tangent frame: u east, v north.
    let t = normalize(vec3<f32>(1.0, 0.0, 0.0) - n0 * n0.x);
    let b = cross(t, n0);
    let dn = normalize(detail_n + vec3<f32>(0.0, 0.0, 0.001));
    let n = normalize(mix(n0, t * dn.x + b * dn.y + n0 * dn.z, detail_weight));
    return vec4<f32>(lit(albedo, n), 1.0);
}

// ---------------------------------------------------------------------------------------
// Placeholder sea until the ocean renderer exists: one camera-centred quad at sea level,
// blended over the terrain below it by water depth.

struct SeaOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) rel: vec3<f32>,
}

@vertex
fn vs_sea(@builtin(vertex_index) index: u32) -> SeaOut {
    // Two clockwise triangles (seen from above) spanning +-extent around the camera.
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, -1.0),
        vec2<f32>(1.0, -1.0), vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, 1.0),
    );
    let c = corners[index] * terrain.misc.z;
    var out: SeaOut;
    out.rel = vec3<f32>(c.x, terrain.camera.w - terrain.camera.y, c.y);
    out.clip = frame.view_proj * vec4<f32>(out.rel, 1.0);
    return out;
}

@fragment
fn fs_sea(in: SeaOut) -> @location(0) vec4<f32> {
    let world_xz = terrain.camera.xz + in.rel.xz;
    let world = terrain.grid.z;
    var depth = 1000.0;
    if all(world_xz >= vec2<f32>(0.0)) && all(world_xz <= vec2<f32>(world)) {
        depth = terrain.camera.w - height_at(world_xz / terrain.grid.x);
    }
    let view = normalize(-in.rel);
    // Schlick fresnel for water (F0 = 0.02).
    let fresnel = 0.02 + 0.98 * pow(1.0 - max(view.y, 0.0), 5.0);
    let deep = vec3<f32>(0.01, 0.05, 0.08);
    let shallow = vec3<f32>(0.05, 0.22, 0.24);
    let body = mix(shallow, deep, clamp(depth / 25.0, 0.0, 1.0));
    let water = lit(body, vec3<f32>(0.0, 1.0, 0.0));
    let reflection = mix(frame.sky_horizon.rgb, frame.sky_zenith.rgb, 0.25);
    let half_vector = normalize(view + frame.sun_dir.xyz);
    let glint = pow(max(half_vector.y, 0.0), 400.0) * 4.0;
    let color = mix(water, reflection, fresnel) + frame.sun_color.rgb * glint;
    // Clear in the shallows, opaque from a few metres down.
    let alpha = clamp(max(1.0 - exp(-depth * 0.35), fresnel), 0.0, 1.0);
    return vec4<f32>(color, alpha);
}
