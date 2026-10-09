// Eye adaptation as RV does it (docs/re/render-atmosphere.md §3.3): a log-average (geometric
// mean) luminance meter, then the aperture stage of the lighting entry (`1 / aperture²` at the
// measured luminance, `0x14175ac10`), stepped towards its target with the soft step near the
// target and the per-frame change limits of `PSC_AssumedLuminancePars2.zw`.
//
// The engine runs the aperture stage on the CPU from the luminance read back off the GPU, divided
// by the exposure it was rendered with. We meter the unexposed HDR buffer, so `measured` below is
// already that absolute scene luminance; the same stage therefore runs in this pass and the
// per-frame GPU-to-CPU read-back is not needed.

// Must match post.rs (PostUniforms).
struct Post {
    // Hable A, B, C, D.
    filmic_abcd: vec4<f32>,
    // Hable E, F, W, exposure bias.
    filmic_efw_bias: vec4<f32>,
    // exposure range (min, max), key of the assumed-luminance fallback, fixed exposure
    // (<= 0: automatic).
    exposure: vec4<f32>,
    // lowest per-frame ratio, highest per-frame ratio, dt, reset (1 = jump to target).
    adaptation: vec4<f32>,
    // tonemap method (RV numbering: 0 none, 1 filmic, 2 Reinhard), Reinhard white, output is
    // sRGB, FXAA on.
    misc: vec4<f32>,
    // bloomImageScale, bloomScale, scene scale, bloomExponent.
    bloom: vec4<f32>,
    // final gamma (PSC_RgbEyeCoef.w), bloom on, unused, unused.
    output: vec4<f32>,
    // apertureMin, apertureStandard, apertureMax, standardAvgLum of the lighting entry.
    aperture: vec4<f32>,
    // apertureRatioMin, apertureRatioMax, minAperture, maxAperture.
    aperture_ratios: vec4<f32>,
}

struct ExposureState {
    adapted_luminance: f32,
    exposure: f32,
    average_luminance: f32,
    _pad: f32,
}

@group(0) @binding(0) var<uniform> post: Post;
@group(0) @binding(1) var hdr: texture_2d<f32>;
// Per workgroup of `measure`: (sum of ln luminance, sample count).
@group(0) @binding(2) var<storage, read_write> partials: array<f32>;
@group(0) @binding(3) var<storage, read_write> state: ExposureState;

var<workgroup> sums: array<f32, 256>;
var<workgroup> counts: array<f32, 256>;

fn reduce(index: u32) {
    for (var stride = 128u; stride > 0u; stride = stride / 2u) {
        if index < stride {
            sums[index] += sums[index + stride];
            counts[index] += counts[index + stride];
        }
        workgroupBarrier();
    }
}

// Each invocation takes 4 taps (a 2x2 block) like PSPostProcessGlowNewLuminanceInit:
// ln(Rec.601 luma + 0.001).
@compute @workgroup_size(16, 16)
fn measure(
    @builtin(workgroup_id) group: vec3<u32>,
    @builtin(num_workgroups) groups: vec3<u32>,
    @builtin(local_invocation_id) local: vec3<u32>,
    @builtin(local_invocation_index) index: u32,
) {
    let size = textureDimensions(hdr);
    let base = (group.xy * 16u + local.xy) * 2u;
    var sum = 0.0;
    var count = 0.0;
    for (var dy = 0u; dy < 2u; dy += 1u) {
        for (var dx = 0u; dx < 2u; dx += 1u) {
            let p = base + vec2<u32>(dx, dy);
            if p.x < size.x && p.y < size.y {
                let c = textureLoad(hdr, vec2<i32>(p), 0).rgb;
                sum += log(dot(c, vec3<f32>(0.299, 0.587, 0.114)) + 0.001);
                count += 1.0;
            }
        }
    }
    sums[index] = sum;
    counts[index] = count;
    workgroupBarrier();
    reduce(index);
    if index == 0u {
        let slot = (group.y * groups.x + group.x) * 2u;
        partials[slot] = sums[0];
        partials[slot + 1u] = counts[0];
    }
}

// The aperture the lighting entry asks for at an absolute scene luminance (`0x14175ac10`): the
// standard aperture at `standardAvgLum`, moving linearly towards apertureMin / apertureMax as the
// luminance falls / rises, reaching them at `standardAvgLum / apertureRatioMin` and
// `standardAvgLum * apertureRatioMax`. The dark branch is the engine's literal one, so the curve
// jumps at the standard luminance when apertureStandard != apertureMin. Clamped to
// `[minAperture, maxAperture]` like the engine.
fn aperture_at(luminance: f32) -> f32 {
    let p = post.aperture;
    let r = post.aperture_ratios;
    var ap: f32;
    if p.y <= 0.0 || p.w <= 0.0 {
        return 1.0;
    }
    if luminance <= p.w {
        let x = -p.w / max(luminance, 1e-6);
        ap = select(p.x + (x + r.x) / (1.0 + r.x) * (p.y - p.x), p.x, r.x <= 1.0 || x < -r.x);
    } else {
        let y = luminance / p.w;
        ap = select(p.y + (y - 1.0) / (r.y - 1.0) * (p.z - p.y), p.z, r.y <= 1.0 || y > r.y);
    }
    return clamp(ap, r.z, max(r.w, r.z));
}

// RV's aperture stage output: the exposure the scene is rendered with.
fn aperture_exposure(luminance: f32) -> f32 {
    let ap = max(aperture_at(luminance), 1e-12);
    return 1.0 / (ap * ap);
}

@compute @workgroup_size(256)
fn adapt(@builtin(local_invocation_index) index: u32) {
    let slots = arrayLength(&partials) / 2u;
    var sum = 0.0;
    var count = 0.0;
    for (var i = index; i < slots; i += 256u) {
        sum += partials[i * 2u];
        count += partials[i * 2u + 1u];
    }
    sums[index] = sum;
    counts[index] = count;
    workgroupBarrier();
    reduce(index);
    if index != 0u {
        return;
    }
    let mean_ln = min(sums[0] / max(counts[0], 1.0), 1637.6);
    // The meter's own cap (`min(t0.x, 1000)` in PSPostProcessAssumedLuminance) is not applied:
    // the engine meters the already exposed HDR buffer, while this meter reads the unexposed
    // scene, so its value is absolute luminance and that bound would clamp every daylit frame.
    let measured = max(exp(mean_ln), 1e-4);
    // RV adapts the aperture stage's exposure: 1 / aperture² at the measured luminance. Entries
    // without a usable aperture stage fall back to the assumed-luminance step key / measured.
    var wanted = aperture_exposure(measured);
    if post.aperture.y <= 0.0 || post.aperture.w <= 0.0 {
        wanted = post.exposure.z / measured;
    }
    let previous = state.exposure;
    if post.adaptation.w < 0.5 && previous > 0.0 {
        var ratio = wanted / previous;
        let l = log2(ratio);
        // Soft step when close to the target (|l| < 1 stop).
        ratio = min(ratio, exp2(select(abs(l), l * l, abs(l) < 1.0)));
        ratio = clamp(ratio, post.adaptation.x, post.adaptation.y);
        wanted = ratio * previous;
    }
    var exposure = clamp(wanted, post.exposure.x, post.exposure.y);
    if post.exposure.w > 0.0 {
        exposure = post.exposure.w;
    }
    state.exposure = exposure;
    state.adapted_luminance = measured;
    state.average_luminance = measured;
}
