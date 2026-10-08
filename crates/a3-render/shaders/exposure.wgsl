// Eye adaptation as RV does it (docs/re/render-atmosphere.md §3.3): a log-average (geometric
// mean) luminance meter, then the "assumed luminance" step towards key / measured with a soft
// step near the target, per-frame change limits and clamps.

// Must match post.rs (PostUniforms).
struct Post {
    // Hable A, B, C, D.
    filmic_abcd: vec4<f32>,
    // Hable E, F, W, exposure bias.
    filmic_efw_bias: vec4<f32>,
    // min exposure, max exposure, key, fixed exposure (<= 0: automatic).
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
    let measured = max(min(exp(mean_ln), 1000.0), 1e-4);
    // RV adapts an exposure-like value: target = key / measured.
    var wanted = post.exposure.z / measured;
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
    state.adapted_luminance = post.exposure.z / exposure;
    state.average_luminance = measured;
}
