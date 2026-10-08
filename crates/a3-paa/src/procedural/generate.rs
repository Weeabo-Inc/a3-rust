//! The engine's procedural texture generators.
//!
//! Every level is generated independently at its own size (not downsampled). All arithmetic
//! is `f32` in the engine's operation order; results are rounded half-to-even (`cvtss2si`)
//! and clamped to 0..=255. The libm functions (`acos`, `sin`, `tan`, `asin`, `pow`, `exp`,
//! `ln`) may differ from the engine's CRT in the last bit, so a value can differ by one.

use std::f32::consts::FRAC_PI_2;

use super::{ColorFormat, Procedural, ProceduralFunction as F, dither, noise};
use crate::{AlphaFlags, Color, Compression, Error, Mip, PixelFormat, Result, Texture};

/// Refractive index of air (1.0002927).
const AIR: f32 = f32::from_bits(0x3f80_0997);
/// Refractive index of water (1.337).
const WATER: f32 = f32::from_bits(0x3fab_22d1);

/// Rounds like the engine (half to even) and clamps to a byte.
fn byte(value: f32) -> u8 {
    (value.round_ties_even() as i32).clamp(0, 255) as u8
}

/// Engine `sincos`: the cosine is evaluated as `sin(x + pi/2)`.
fn sin_cos(x: f32) -> (f32, f32) {
    (x.sin(), (x + FRAC_PI_2).sin())
}

/// Division that returns +-`f32::MAX` (or 1 for 0/0) instead of dividing by a denormal or
/// zero, as the engine's Fresnel code does.
fn safe_div(num: f32, den: f32) -> f32 {
    if den.abs() >= f32::MIN_POSITIVE {
        num / den
    } else if num.abs() >= f32::MIN_POSITIVE {
        if num * den >= 0.0 {
            f32::MAX
        } else {
            -f32::MAX
        }
    } else {
        1.0
    }
}

/// `1 / (n - 1)`, the step that maps `0..n` onto `0..=1`.
fn unit_step(n: usize) -> f32 {
    1.0 / (n as f32 - 1.0)
}

/// One RGBA value per pixel of a level; packed to AI88 as intensity = red, alpha = alpha.
type Pixels = Vec<[u8; 4]>;

/// Conductor Fresnel reflectance lookup, `width` entries for cos(angle) = 0..=1.
fn fresnel_row(width: usize, n: f32, k: f32) -> Vec<u8> {
    let step = unit_step(width);
    (0..width)
        .map(|i| {
            let theta = (i as f32 * step).acos();
            let (sin, cos) = sin_cos(theta);
            let tan = theta.tan();
            let t0 = (n * n - k * k) - sin * sin;
            let ab2 = (k * k * n * (n * 4.0) + t0 * t0).sqrt();
            let a2 = (ab2 + t0) * 0.5;
            let two_a = a2.sqrt() + a2.sqrt();
            let ab2 = (ab2 - t0) * 0.5 + a2;
            let rs = safe_div(
                (ab2 - two_a * cos) + cos * cos,
                two_a * cos + ab2 + cos * cos,
            );
            let rp = if i == 0 {
                rs
            } else {
                let st = two_a * sin * tan;
                let s2t2 = tan * tan * (sin * sin);
                safe_div(((ab2 - st) + s2t2) * rs, st + ab2 + s2t2)
            };
            byte((rs + rp) * 127.5)
        })
        .collect()
}

/// Dielectric (glass) Fresnel reflectance lookup.
fn fresnel_glass_row(width: usize, n: f32) -> Vec<u8> {
    let step = unit_step(width);
    let inv_n = 1.0 / n;
    (0..width)
        .map(|i| {
            let incident = (f64::from((i as f32 * step).acos()) + 1e-6) as f32;
            let refracted = (incident.sin() * AIR * inv_n).asin();
            let (diff, sum) = (incident - refracted, incident + refracted);
            let s = diff.sin() / sum.sin();
            let t = diff.tan() / sum.tan();
            let r = t * t + s * s;
            byte(r / (r * 0.5 + 1.0) * 255.0)
        })
        .collect()
}

/// Air-to-water Fresnel reflectance for cos(angle) = 0..=1.
fn water_fresnel_row(width: usize) -> Vec<u8> {
    // (AIR / WATER)^2, as stored in the executable.
    const RATIO_SQ: f32 = f32::from_bits(0x3f0f_4b8b);
    let step = unit_step(width);
    (0..width)
        .map(|i| {
            let cos_i = i as f32 * step;
            let cos_t = (1.0 - (1.0 - cos_i * cos_i) * RATIO_SQ).sqrt();
            let rs = (cos_i * WATER - cos_t * AIR) / (cos_i * WATER + cos_t * AIR);
            let rp = (cos_i * AIR - cos_t * WATER) / (cos_t * WATER + cos_i * AIR);
            byte((rs * rs + rp * rp) * 127.5)
        })
        .collect()
}

/// `(i / (n - 1))^power * 255` for `i` in `0..n`.
fn power_ramp(n: usize, power: f32) -> Vec<u8> {
    let step = unit_step(n);
    (0..n)
        .map(|i| byte((i as f32 * step).powf(power) * 255.0))
        .collect()
}

/// `density^(i / (n - 1)) * 255`, or zeros for a non-positive density.
fn density_ramp(n: usize, density: f32) -> Vec<u8> {
    if density <= 0.0 {
        return vec![0; n];
    }
    let ln = density.ln();
    let step = unit_step(n);
    (0..n)
        .map(|i| byte((i as f32 * step * ln).exp() * 255.0))
        .collect()
}

fn grid(w: usize, h: usize, mut pixel: impl FnMut(usize, usize) -> [u8; 4]) -> Pixels {
    (0..h)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .map(|(x, y)| pixel(x, y))
        .collect()
}

/// Generates one level of a non-colour function.
fn level(function: &F, w: usize, h: usize) -> Pixels {
    match *function {
        F::Fresnel { n, k } => {
            let row = fresnel_row(w, n, k);
            grid(w, h, |x, _| [0, 0, 0, row[x]])
        }
        F::FresnelGlass { n } => {
            let row = fresnel_glass_row(w, n);
            grid(w, h, |x, _| [0, 0, 0, row[x]])
        }
        F::Irradiance { power } => {
            let alpha = power_ramp(h, power);
            let step = unit_step(w);
            let ramp: Vec<u8> = (0..w).map(|x| byte(x as f32 * step * 255.0)).collect();
            grid(w, h, |x, y| match ramp[x] {
                0 => [0; 4],
                i => [i, i, i, alpha[y]],
            })
        }
        F::WaterIrradiance { power } => {
            let intensity = power_ramp(h, power);
            let alpha = water_fresnel_row(w);
            grid(w, h, |x, y| {
                let i = intensity[y];
                [i, i, i, alpha[x]]
            })
        }
        F::PerlinNoise {
            x_scale,
            y_scale,
            min,
            max,
        } => {
            let (inv_w, inv_h) = (1.0 / w as f32, 1.0 / h as f32);
            grid(w, h, |x, y| {
                let n = noise::noise(
                    (x as f32 + 0.5) * inv_w * x_scale,
                    (y as f32 + 0.5) * inv_h * y_scale,
                    0.0,
                );
                [byte((max - min) * (n + 1.0) * 127.5 + min * 255.0); 4]
            })
        }
        F::Point => {
            let (sx, sy) = (unit_step(w), unit_step(h));
            grid(w, h, |x, y| {
                let centred = |t: f32| ((t - 0.5) + t) - 0.5;
                let u = centred(x as f32 * sx);
                let v = centred(y as f32 * sy);
                [255, 255, 255, byte((1.0 - (u * u + v * v).sqrt()) * 255.0)]
            })
        }
        F::TreeCrown { density } => {
            let alpha = density_ramp(h, density);
            let intensity = density_ramp(w, density);
            grid(w, h, |x, y| {
                let i = intensity[x];
                [i, i, i, alpha[y]]
            })
        }
        F::TreeCrownAmb { density } => {
            if density <= 0.0 {
                return vec![[0; 4]; w * h];
            }
            let ln = density.ln();
            let (sx, sy) = (unit_step(w), unit_step(h));
            grid(w, h, |x, y| {
                let v = (sy + sy) * y as f32 - 1.0;
                let u = x as f32 * sx;
                let r2 = u * u + v * v;
                let value = if r2 < 1.0 {
                    ((1.0 - r2) * ln).exp()
                } else {
                    1.0
                };
                [byte(value * 255.0); 4]
            })
        }
        F::Dither { a, b } => {
            // Square; the matrix is indexed with the level width as stride.
            dither::matrix(w, a, b)
                .into_iter()
                .map(|v| [v, v, v, v])
                .collect()
        }
        F::Color { .. } | F::Runtime { .. } => unreachable!("handled by generate"),
    }
}

fn pack(format: PixelFormat, pixels: &[[u8; 4]]) -> Vec<u8> {
    match format {
        PixelFormat::Ai88 => pixels.iter().flat_map(|&[i, _, _, a]| [i, a]).collect(),
        _ => pixels
            .iter()
            .flat_map(|&[r, g, b, a]| [b, g, r, a])
            .collect(),
    }
}

fn mip(width: usize, height: usize, data: Vec<u8>) -> Mip {
    Mip {
        width: width as u16,
        height: height as u16,
        data,
        compression: Compression::Lzss,
    }
}

impl Procedural {
    /// The pixel format of the generated texture: AI88 for `ai`/`a`/`i`, ARGB8888 for
    /// `argb`/`rgb`.
    pub fn pixel_format(&self) -> PixelFormat {
        if self.format.is_ai88() {
            PixelFormat::Ai88
        } else {
            PixelFormat::Argb8888
        }
    }

    /// The sizes of the levels the engine generates. `color` is always one 1x1 level;
    /// `fresnel` and `fresnelGlass` one `width` x 1 level; `dither` a square of the larger
    /// side halving down to 2x2; everything else halves from the declared size for the
    /// declared number of levels, stopping after the first level with a side below 2.
    pub fn level_sizes(&self) -> Vec<(usize, usize)> {
        let (w, h) = (usize::from(self.width), usize::from(self.height));
        match self.function {
            F::Color { .. } => vec![(1, 1)],
            F::Fresnel { .. } | F::FresnelGlass { .. } => vec![(w, 1)],
            F::Dither { .. } => {
                let size = w.max(h);
                let levels = (size.ilog2() as usize).max(1);
                (0..levels).map(|i| (size >> i, size >> i)).collect()
            }
            _ => {
                let mut out = Vec::new();
                for i in 0..usize::from(self.mip_count) {
                    let (lw, lh) = (w >> i, h >> i);
                    if lw == 0 || lh == 0 {
                        break;
                    }
                    out.push((lw, lh));
                    if lw < 2 || lh < 2 {
                        break;
                    }
                }
                out
            }
        }
    }

    /// Generates the texture as the engine does (see `docs/re/paa.md` for each algorithm).
    ///
    /// Runtime sources (`r2t`, `text`, `ui`, `uiex`, `extension`) and `dither` in an ARGB
    /// format return [`Error::Procedural`].
    pub fn generate(&self) -> Result<Texture> {
        let fail = |reason: &str| {
            Err(Error::Procedural {
                text: self.to_string(),
                reason: reason.to_owned(),
            })
        };
        let format = self.pixel_format();
        let mips = match &self.function {
            F::Runtime { .. } => return fail("rendered by the engine at run time"),
            F::Dither { .. } if format != PixelFormat::Ai88 => {
                return fail("dither supports only the ai formats");
            }
            F::Color { rgba, .. } => vec![mip(1, 1, color_pixel(self.format, *rgba))],
            function => self
                .level_sizes()
                .into_iter()
                .map(|(w, h)| mip(w, h, pack(format, &level(function, w, h))))
                .collect(),
        };
        let mut texture = Texture {
            format,
            procedural: Some(self.to_string()),
            mips,
            ..Texture::default()
        };
        summarise(&mut texture);
        Ok(texture)
    }
}

/// The single pixel of a `color()` texture.
fn color_pixel(format: ColorFormat, rgba: [f32; 4]) -> Vec<u8> {
    let [r, g, b, a] = rgba;
    if format.is_ai88() {
        let luma = g * 149.685 + r * 76.245 + b * 29.07;
        vec![byte(luma), byte(a * 255.0)]
    } else {
        let c = |v: f32| byte(v.clamp(0.0, 1.0) * 255.0);
        vec![c(b), c(g), c(r), c(a)]
    }
}

/// Fills `AVGC`, `MAXC` and `FLAG` from the top level, as for any converted texture.
fn summarise(texture: &mut Texture) {
    let Ok(rgba) = crate::decode_rgba8(texture.format, &texture.mips[0]) else {
        return;
    };
    let n = (rgba.len() / 4).max(1) as u64;
    let mut sum = [0u64; 4];
    for px in rgba.chunks_exact(4) {
        for (s, &v) in sum.iter_mut().zip(px) {
            *s += u64::from(v);
        }
    }
    let [r, g, b, a] = sum.map(|s| ((s + n / 2) / n) as u8);
    texture.average_color = Some(Color::rgba(r, g, b, a));
    texture.max_color = Some(Color::WHITE);
    let alphas = || rgba.chunks_exact(4).map(|p| p[3]);
    texture.flags = if alphas().all(|a| a == 255) {
        None
    } else if alphas().all(|a| a == 0 || a == 255) {
        Some(AlphaFlags(AlphaFlags::BINARY))
    } else {
        Some(AlphaFlags(AlphaFlags::INTERPOLATED))
    };
}
