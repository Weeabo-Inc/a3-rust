// Terrain (CDLOD heightmap patches) and sea. Positions are camera-relative (ADR 0003).

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
}

struct Terrain {
    // Camera world position (x, y, z) and the sea level.
    camera: vec4<f32>,
    // Height cell edge (m), height samples per axis, world edge (m), land cell edge (m).
    grid: vec4<f32>,
    // Land cells per axis, tile count, sea extent (m), unused.
    misc: vec4<f32>,
    // Per LOD level: morph start (m), morph end (m), 1 / (end - start), unused.
    morph: array<vec4<f32>, 16>,
}

struct Tile {
    // u = u.x * x + u.y * z + u.z (world metres); likewise v.
    u: vec4<f32>,
    v: vec4<f32>,
    // x: texture array layer of the full-resolution satellite tile, or -1.
    slot: vec4<i32>,
}

@group(0) @binding(0) var<uniform> frame: Frame;
@group(1) @binding(0) var heights: texture_2d<f32>;
@group(1) @binding(1) var normals: texture_2d<f32>;
@group(1) @binding(2) var overview: texture_2d<f32>;
@group(1) @binding(3) var linear_clamp: sampler;
@group(1) @binding(4) var<uniform> terrain: Terrain;
@group(1) @binding(5) var cell_tiles: texture_2d<u32>;
@group(1) @binding(6) var<storage, read> tiles: array<Tile>;
@group(1) @binding(7) var near_tiles: texture_2d_array<f32>;
@group(1) @binding(8) var tile_sampler: sampler;

const NO_TILE: u32 = 65535u;

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
    return out;
}

fn overview_color(world_xz: vec2<f32>) -> vec3<f32> {
    let world = terrain.grid.z;
    let uv = vec2<f32>(world_xz.x / world, 1.0 - world_xz.y / world);
    return textureSample(overview, linear_clamp, uv).rgb;
}

fn satellite(world_xz: vec2<f32>) -> vec3<f32> {
    let far = overview_color(world_xz);
    let land = terrain.grid.w;
    let cells = i32(terrain.misc.x);
    let cell = clamp(vec2<i32>(floor(world_xz / land)), vec2<i32>(0), vec2<i32>(cells - 1));
    let tile_index = textureLoad(cell_tiles, cell, 0).r;
    // Gradients from the continuous world position, so tile borders do not change the mip.
    let dx = dpdx(world_xz);
    let dy = dpdy(world_xz);
    if tile_index == NO_TILE || tile_index >= arrayLength(&tiles) {
        return far;
    }
    let tile = tiles[tile_index];
    if tile.slot.x < 0 {
        return far;
    }
    let uv = vec2<f32>(
        dot(tile.u.xy, world_xz) + tile.u.z,
        dot(tile.v.xy, world_xz) + tile.v.z,
    );
    let ddx = vec2<f32>(dot(tile.u.xy, dx), dot(tile.v.xy, dx));
    let ddy = vec2<f32>(dot(tile.u.xy, dy), dot(tile.v.xy, dy));
    return textureSampleGrad(near_tiles, tile_sampler, uv, tile.slot.x, ddx, ddy).rgb;
}

fn surface_normal(world_xz: vec2<f32>) -> vec3<f32> {
    let size = terrain.grid.y;
    let uv = (world_xz / terrain.grid.x + 0.5) / size;
    return normalize(textureSample(normals, linear_clamp, uv).xyz);
}

fn lit(albedo: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    let diffuse = max(dot(n, frame.sun_dir.xyz), 0.0);
    // Hemispherical ambient: sky from above, a darker bounce from below.
    let sky = mix(frame.sky_horizon.rgb, frame.sky_zenith.rgb, 0.5) * frame.sun_color.w * 2.0;
    let ambient = mix(sky * 0.5, sky, n.y * 0.5 + 0.5);
    return albedo * (frame.sun_color.rgb * diffuse + ambient);
}

@fragment
fn fs_terrain(in: TerrainOut) -> @location(0) vec4<f32> {
    let n = surface_normal(in.world_xz);
    let albedo = satellite(in.world_xz);
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
