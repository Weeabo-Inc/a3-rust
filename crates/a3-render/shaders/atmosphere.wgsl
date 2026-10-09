// Atmosphere: sky where nothing was drawn, distance fog elsewhere, into the HDR target.
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
    // x: water height, y: water fog extinction per metre, z: 1 when there is water.
    water: vec4<f32>,
    water_fog_color: vec4<f32>,
    // Water fog colour scale looking down, at the horizon and up.
    water_fog_gradient: vec4<f32>,
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

// Optical depth of the height fog `beta0 * e^(-k * h)` along `distance` metres of the ray
// `dir` from the camera.
fn fog_optical_depth(dir: vec3<f32>, distance: f32) -> f32 {
    let beta0 = frame.sky_horizon.w;
    let k = frame.fog.y;
    let at_camera = beta0 * exp(-k * frame.fog.z);
    let rise = k * dir.y * distance;
    if abs(rise) < 1e-4 {
        return at_camera * distance;
    }
    return at_camera * distance * (1.0 - exp(-rise)) / rise;
}

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let texel = vec2<i32>(floor(position.xy));
    let depth = textureLoad(scene_depth, texel, 0);
    let dir = view_ray(position.xy);
    let color = textureLoad(scene_color, texel, 0).rgb;
    let under_water = frame.water.z > 0.5 && frame.fog.z < frame.water.x;
    if depth <= 0.0 && !under_water {
        if frame.fog.w > 0.5 {
            return vec4<f32>(sky(dir), 1.0);
        }
        // A render feature drew the sky.
        return vec4<f32>(color, 1.0);
    }
    if depth <= 0.0 {
        // Below the water, open sky past the edge of the sea is lost in the water fog.
        let g = frame.water_fog_gradient;
        var shade = g.y + dir.y * (g.z - g.y);
        if dir.y < 0.0 {
            shade = g.x + (1.0 + dir.y) * (1.0 + dir.y) * (g.y - g.x);
        }
        return vec4<f32>(shade * frame.water_fog_color.rgb, 1.0);
    }
    // Reversed infinite projection: view-space z = near / depth.
    let view_z = frame.params.x / depth;
    let forward = normalize((frame.inv_view_proj * vec4<f32>(0.0, 0.0, 1.0, 1.0)).xyz);
    let distance = view_z / max(dot(dir, forward), 1e-4);
    // RV fogs the part of the ray below the water with the water fog (render-atmosphere.md
    // section 2). Seen from above, the water surface hides everything below it and the sea
    // shader fogs what it refracts, so only a camera below the water splits rays here.
    var under = 0.0;
    let camera_y = frame.fog.z;
    if frame.water.z > 0.5 && camera_y < frame.water.x {
        let rise = dir.y * distance;
        if rise > 1e-4 {
            under = distance * saturate((frame.water.x - camera_y) / rise);
        } else {
            under = distance;
        }
    }
    let in_air = distance - under;
    var transmittance = exp(-(fog_optical_depth(dir, in_air) + frame.haze.rgb * in_air));
    if frame.linear_fog.y > 0.0 {
        transmittance *= saturate((frame.linear_fog.x - distance) * frame.linear_fog.y);
    }
    if under > 0.0 {
        let water = exp(-under * frame.water.y);
        let g = frame.water_fog_gradient;
        var shade = g.y + dir.y * (g.z - g.y);
        if dir.y < 0.0 {
            shade = g.x + (1.0 + dir.y) * (1.0 + dir.y) * (g.y - g.x);
        }
        // Camera below water: scene*Ta*Tw + airFog*(1 - Ta) + waterFog*Ta*(1 - Tw).
        let lit = color * transmittance * water + frame.sky_horizon.rgb * (1.0 - transmittance);
        let fogged = shade * frame.water_fog_color.rgb * transmittance * (1.0 - water);
        return vec4<f32>(lit + fogged, 1.0);
    }
    return vec4<f32>(mix(frame.sky_horizon.rgb, color, transmittance), 1.0);
}
