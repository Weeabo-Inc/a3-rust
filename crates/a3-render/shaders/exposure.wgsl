// Eye adaptation: a log-luminance histogram of the HDR image, then the adapted luminance
// ("aperture") and exposure for this frame. See docs/re/hdr.md.

struct Post {
    // Hable A, B, C, D.
    filmic_abcd: vec4<f32>,
    // Hable E, F, W, exposure bias.
    filmic_efw_bias: vec4<f32>,
    // log2 of min luminance, log2 range, key, fixed exposure (<= 0: automatic).
    exposure: vec4<f32>,
    // speed towards brighter, speed towards darker, dt, reset (1 = jump to target).
    adaptation: vec4<f32>,
    // tonemap method (RV numbering: 0 none, 1 filmic, 2 Reinhard), Reinhard white, output is
    // sRGB, FXAA on.
    misc: vec4<f32>,
}

struct ExposureState {
    adapted_luminance: f32,
    exposure: f32,
    average_luminance: f32,
    _pad: f32,
}

const BINS: u32 = 256u;

@group(0) @binding(0) var<uniform> post: Post;
@group(0) @binding(1) var hdr: texture_2d<f32>;
@group(0) @binding(2) var<storage, read_write> histogram: array<atomic<u32>, 256>;
@group(0) @binding(3) var<storage, read_write> state: ExposureState;

var<workgroup> local_bins: array<atomic<u32>, 256>;
var<workgroup> weighted: array<f32, 256>;
var<workgroup> counted: array<f32, 256>;

fn luminance(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// Bin 0 holds black pixels and is ignored; bins 1..255 span the log2 range.
fn bin_of(lum: f32) -> u32 {
    if lum < 1e-7 {
        return 0u;
    }
    let t = clamp((log2(lum) - post.exposure.x) / post.exposure.y, 0.0, 1.0);
    return u32(t * 254.0 + 1.0);
}

@compute @workgroup_size(16, 16)
fn build_histogram(
    @builtin(global_invocation_id) id: vec3<u32>,
    @builtin(local_invocation_index) index: u32,
) {
    atomicStore(&local_bins[index], 0u);
    workgroupBarrier();
    let size = textureDimensions(hdr);
    if id.x < size.x && id.y < size.y {
        let c = textureLoad(hdr, vec2<i32>(id.xy), 0).rgb;
        atomicAdd(&local_bins[bin_of(luminance(c))], 1u);
    }
    workgroupBarrier();
    atomicAdd(&histogram[index], atomicLoad(&local_bins[index]));
}

@compute @workgroup_size(256)
fn adapt(@builtin(local_invocation_index) index: u32) {
    let count = f32(atomicLoad(&histogram[index]));
    // Clear for the next frame.
    atomicStore(&histogram[index], 0u);
    if index == 0u {
        weighted[index] = 0.0;
        counted[index] = 0.0;
    } else {
        weighted[index] = count * f32(index);
        counted[index] = count;
    }
    workgroupBarrier();
    for (var stride = BINS / 2u; stride > 0u; stride = stride / 2u) {
        if index < stride {
            weighted[index] += weighted[index + stride];
            counted[index] += counted[index + stride];
        }
        workgroupBarrier();
    }
    if index != 0u {
        return;
    }
    let min_log = post.exposure.x;
    let range = post.exposure.y;
    var average = exp2(min_log);
    if counted[0] > 0.0 {
        let mean_bin = weighted[0] / counted[0];
        average = exp2((mean_bin - 1.0) / 254.0 * range + min_log);
    }
    var adapted = average;
    if post.adaptation.w < 0.5 {
        let previous = state.adapted_luminance;
        let speed = select(post.adaptation.y, post.adaptation.x, average > previous);
        adapted = previous + (average - previous) * (1.0 - exp(-post.adaptation.z * speed));
    }
    adapted = clamp(adapted, exp2(min_log), exp2(min_log + range));
    state.adapted_luminance = adapted;
    state.average_luminance = average;
    if post.exposure.w > 0.0 {
        state.exposure = post.exposure.w;
    } else {
        state.exposure = post.exposure.z / adapted;
    }
}
