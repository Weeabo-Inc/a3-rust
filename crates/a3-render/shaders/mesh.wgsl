// Lit, textured mesh instances. Positions are camera-relative (see ADR 0003).

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
}

@group(0) @binding(0) var<uniform> frame: Frame;
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
}

@vertex
fn vs_main(v: VertexIn, i: InstanceIn) -> VertexOut {
    let model = mat4x4<f32>(i.model_0, i.model_1, i.model_2, i.model_3);
    let relative = model * vec4<f32>(v.position, 1.0);
    var out: VertexOut;
    out.clip = frame.view_proj * relative;
    out.normal = (model * vec4<f32>(v.normal, 0.0)).xyz;
    out.tangent = vec4<f32>((model * vec4<f32>(v.tangent.xyz, 0.0)).xyz, v.tangent.w);
    out.uv = v.uv;
    out.color = i.color;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let base = textureSample(base_texture, base_sampler, in.uv) * in.color;
    let n = normalize(in.normal);
    let diffuse = max(dot(n, frame.sun_dir.xyz), 0.0);
    let light = frame.sun_color.rgb * diffuse + vec3<f32>(frame.sun_color.w);
    return vec4<f32>(base.rgb * light, base.a);
}
