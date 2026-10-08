// Post-process: sky where nothing was drawn, distance fog elsewhere, then output.
// Reads the HDR scene colour and the reversed-Z depth buffer.

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
@group(1) @binding(0) var scene_color: texture_2d<f32>;
@group(1) @binding(1) var scene_depth: texture_depth_2d;

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    // One triangle covering the screen.
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

fn view_ray(pixel: vec2<f32>) -> vec3<f32> {
    let ndc = vec2<f32>(pixel.x * frame.viewport.z * 2.0 - 1.0, 1.0 - pixel.y * frame.viewport.w * 2.0);
    // Any depth > 0 lies on the ray; 1 is the near plane.
    let p = frame.inv_view_proj * vec4<f32>(ndc, 1.0, 1.0);
    return normalize(p.xyz / p.w);
}

fn sky(dir: vec3<f32>) -> vec3<f32> {
    let up = clamp(dir.y, 0.0, 1.0);
    var color = mix(frame.sky_horizon.rgb, frame.sky_zenith.rgb, pow(up, 0.5));
    let sun = max(dot(dir, frame.sun_dir.xyz), 0.0);
    color += frame.sun_color.rgb * pow(sun, 800.0) * 4.0;
    return color;
}

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let texel = vec2<i32>(floor(position.xy));
    let depth = textureLoad(scene_depth, texel, 0);
    let dir = view_ray(position.xy);
    if depth <= 0.0 {
        return vec4<f32>(sky(dir), 1.0);
    }
    let color = textureLoad(scene_color, texel, 0).rgb;
    // Reversed infinite projection: view-space z = near / depth.
    let view_z = frame.params.x / depth;
    let forward = normalize((frame.inv_view_proj * vec4<f32>(0.0, 0.0, 1.0, 1.0)).xyz);
    let distance = view_z / max(dot(dir, forward), 1e-4);
    let fog = 1.0 - exp(-distance * frame.sky_horizon.w);
    return vec4<f32>(mix(color, frame.sky_horizon.rgb, fog), 1.0);
}
