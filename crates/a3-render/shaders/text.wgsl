// Screen-space bitmap text: one instanced quad per glyph.

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

@group(0) @binding(0) var<uniform> frame: Frame;
@group(1) @binding(0) var font_atlas: texture_2d<f32>;

// Must match font.rs.
const CELL: vec2<f32> = vec2<f32>(6.0, 8.0);
const GLYPH: vec2<f32> = vec2<f32>(5.0, 7.0);
const COLUMNS: u32 = 16u;

struct GlyphIn {
    // Top-left corner in pixels.
    @location(0) position: vec2<f32>,
    @location(1) glyph: u32,
    @location(2) scale: f32,
    @location(3) color: vec4<f32>,
}

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    // Atlas texel coordinate.
    @location(0) texel: vec2<f32>,
    @location(1) color: vec4<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) index: u32, g: GlyphIn) -> VertexOut {
    // Two triangles: corners (0,0) (1,0) (0,1) / (0,1) (1,0) (1,1).
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
    );
    let corner = corners[index];
    let pixel = g.position + corner * GLYPH * g.scale;
    let ndc = vec2<f32>(pixel.x * frame.viewport.z * 2.0 - 1.0, 1.0 - pixel.y * frame.viewport.w * 2.0);
    let cell = vec2<f32>(f32(g.glyph % COLUMNS), f32(g.glyph / COLUMNS));
    var out: VertexOut;
    out.clip = vec4<f32>(ndc, 0.0, 1.0);
    out.texel = cell * CELL + corner * GLYPH;
    out.color = g.color;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let coverage = textureLoad(font_atlas, vec2<i32>(floor(in.texel)), 0).r;
    if coverage < 0.5 {
        discard;
    }
    return in.color;
}
