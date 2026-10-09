//! `a3-tools image-diff`: compare a reference screenshot (the real game) with ours.
//!
//! Metrics, all on 8-bit sRGB PNGs of the same size:
//! - mean absolute error per channel in linear light, and its average;
//! - SSIM of the sRGB luma over 8x8 windows with stride 4;
//! - mean and log-average Rec.709 luminance of both images, and their ratio (exposure);
//! - the distance between the two sRGB luma histograms (earth mover's distance over 64 bins,
//!   0 = same histogram, 1 = all black against all white);
//! - the mean linear colour of the top, middle and bottom third (sky against ground).
//!
//! Writes `side.png` (reference left, ours right, half size) and `diff.png` (half size: red
//! where ours is brighter, blue where it is darker, over the dimmed reference).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use clap::Args;

use crate::paa_cmd::{read_png, write_png};

#[derive(Args)]
pub struct ImageDiffArgs {
    /// Reference image (the real game's screenshot).
    reference: PathBuf,
    /// Our image.
    ours: PathBuf,
    /// Folder for `side.png` and `diff.png`; nothing is written without it.
    #[arg(long)]
    out: Option<PathBuf>,
    /// Print the metrics as JSON instead of text.
    #[arg(long)]
    json: bool,
}

/// An RGBA8 image.
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Comparison of a reference image and ours.
#[derive(Debug, Clone, PartialEq)]
pub struct Metrics {
    pub mae_linear_rgb: [f64; 3],
    pub mae_linear: f64,
    pub ssim: f64,
    pub mean_lum: [f64; 2],
    pub log_avg_lum: [f64; 2],
    pub histogram_distance: f64,
    /// Mean linear RGB of the top, middle and bottom third: `[reference, ours]` per band.
    pub bands: [[[f64; 3]; 2]; 3],
}

impl Metrics {
    pub fn lum_ratio(&self) -> f64 {
        self.mean_lum[1] / self.mean_lum[0].max(1e-9)
    }

    pub fn to_json(&self) -> String {
        let v3 = |v: &[f64; 3]| format!("[{:.6}, {:.6}, {:.6}]", v[0], v[1], v[2]);
        let mut s = String::from("{\n");
        let _ = writeln!(s, "  \"mae_linear\": {:.6},", self.mae_linear);
        let _ = writeln!(s, "  \"mae_linear_rgb\": {},", v3(&self.mae_linear_rgb));
        let _ = writeln!(s, "  \"ssim\": {:.6},", self.ssim);
        let _ = writeln!(s, "  \"mean_lum_arma\": {:.6},", self.mean_lum[0]);
        let _ = writeln!(s, "  \"mean_lum_ours\": {:.6},", self.mean_lum[1]);
        let _ = writeln!(s, "  \"lum_ratio\": {:.6},", self.lum_ratio());
        let _ = writeln!(s, "  \"log_avg_lum_arma\": {:.6},", self.log_avg_lum[0]);
        let _ = writeln!(s, "  \"log_avg_lum_ours\": {:.6},", self.log_avg_lum[1]);
        let _ = writeln!(
            s,
            "  \"histogram_distance\": {:.6},",
            self.histogram_distance
        );
        s.push_str("  \"bands\": {\n");
        for (i, (name, band)) in ["top", "middle", "bottom"]
            .iter()
            .zip(&self.bands)
            .enumerate()
        {
            let comma = if i < 2 { "," } else { "" };
            let _ = writeln!(
                s,
                "    \"{name}\": {{\"arma\": {}, \"ours\": {}}}{comma}",
                v3(&band[0]),
                v3(&band[1])
            );
        }
        s.push_str("  }\n}");
        s
    }
}

pub fn run(args: ImageDiffArgs) -> Result<()> {
    let load = |p: &Path| -> Result<Image> {
        let (width, height, rgba) = read_png(p)?;
        Ok(Image {
            width,
            height,
            rgba,
        })
    };
    let reference = load(&args.reference)?;
    let ours = load(&args.ours)?;
    let metrics = compare(&reference, &ours)?;
    if let Some(dir) = &args.out {
        std::fs::create_dir_all(dir)?;
        let side = side_by_side(&reference, &ours);
        write_png(&dir.join("side.png"), side.width, side.height, &side.rgba)?;
        let diff = difference(&reference, &ours);
        write_png(&dir.join("diff.png"), diff.width, diff.height, &diff.rgba)?;
    }
    if args.json {
        println!("{}", metrics.to_json());
    } else {
        let m = &metrics;
        println!(
            "MAE linear {:.4} (R {:.4} G {:.4} B {:.4})",
            m.mae_linear, m.mae_linear_rgb[0], m.mae_linear_rgb[1], m.mae_linear_rgb[2]
        );
        println!("SSIM {:.4}", m.ssim);
        println!(
            "mean luminance {:.4} vs {:.4} (ratio {:.3}), log-average {:.4} vs {:.4}",
            m.mean_lum[0],
            m.mean_lum[1],
            m.lum_ratio(),
            m.log_avg_lum[0],
            m.log_avg_lum[1]
        );
        println!("histogram distance {:.4}", m.histogram_distance);
    }
    Ok(())
}

/// sRGB-encoded byte to linear light.
fn linear(c: u8) -> f64 {
    let c = f64::from(c) / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_table() -> [f64; 256] {
    std::array::from_fn(|i| linear(i as u8))
}

/// Rec.709 luminance of linear RGB.
fn luminance(rgb: [f64; 3]) -> f64 {
    0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]
}

/// Rec.709 luma of the sRGB-encoded bytes, 0..1.
fn luma(p: &[u8]) -> f64 {
    (0.2126 * f64::from(p[0]) + 0.7152 * f64::from(p[1]) + 0.0722 * f64::from(p[2])) / 255.0
}

pub fn compare(reference: &Image, ours: &Image) -> Result<Metrics> {
    if (reference.width, reference.height) != (ours.width, ours.height) {
        bail!(
            "image sizes differ: {}x{} against {}x{}",
            reference.width,
            reference.height,
            ours.width,
            ours.height
        );
    }
    let (w, h) = (reference.width as usize, reference.height as usize);
    if w == 0 || h == 0 {
        bail!("empty image");
    }
    let lin = linear_table();
    let mut abs = [0.0f64; 3];
    let mut lum = [0.0f64; 2];
    let mut log_lum = [0.0f64; 2];
    let mut bands = [[[0.0f64; 3]; 2]; 3];
    let mut band_pixels = [0usize; 3];
    const BINS: usize = 64;
    let mut hist = [[0u64; BINS]; 2];
    for y in 0..h {
        let band = (y * 3 / h).min(2);
        band_pixels[band] += w;
        for x in 0..w {
            let i = (y * w + x) * 4;
            for (k, img) in [reference, ours].iter().enumerate() {
                let p = &img.rgba[i..i + 4];
                let rgb = [lin[p[0] as usize], lin[p[1] as usize], lin[p[2] as usize]];
                let l = luminance(rgb);
                lum[k] += l;
                log_lum[k] += (l + 1e-4).ln();
                for c in 0..3 {
                    bands[band][k][c] += rgb[c];
                }
                let bin = ((luma(p) * BINS as f64) as usize).min(BINS - 1);
                hist[k][bin] += 1;
            }
            for (c, a) in abs.iter_mut().enumerate() {
                *a += (lin[reference.rgba[i + c] as usize] - lin[ours.rgba[i + c] as usize]).abs();
            }
        }
    }
    let n = (w * h) as f64;
    let mae_linear_rgb = abs.map(|a| a / n);
    for (band, &count) in bands.iter_mut().zip(&band_pixels) {
        for img in band.iter_mut() {
            for c in img.iter_mut() {
                *c /= count.max(1) as f64;
            }
        }
    }
    let (mut cdf, mut emd) = ([0.0f64; 2], 0.0);
    for (a, b) in hist[0].iter().zip(&hist[1]) {
        cdf[0] += *a as f64 / n;
        cdf[1] += *b as f64 / n;
        emd += (cdf[0] - cdf[1]).abs();
    }
    Ok(Metrics {
        mae_linear: mae_linear_rgb.iter().sum::<f64>() / 3.0,
        mae_linear_rgb,
        ssim: ssim(reference, ours),
        mean_lum: lum.map(|l| l / n),
        log_avg_lum: log_lum.map(|l| (l / n).exp()),
        histogram_distance: emd / BINS as f64,
        bands,
    })
}

/// Mean SSIM of the sRGB luma over 8x8 windows with stride 4.
fn ssim(a: &Image, b: &Image) -> f64 {
    const WIN: usize = 8;
    const STRIDE: usize = 4;
    const C1: f64 = 0.01 * 0.01;
    const C2: f64 = 0.03 * 0.03;
    let (w, h) = (a.width as usize, a.height as usize);
    let la: Vec<f64> = a.rgba.chunks_exact(4).map(luma).collect();
    let lb: Vec<f64> = b.rgba.chunks_exact(4).map(luma).collect();
    if w < WIN || h < WIN {
        return if la == lb { 1.0 } else { 0.0 };
    }
    let (mut total, mut count) = (0.0, 0usize);
    let mut y = 0;
    while y + WIN <= h {
        let mut x = 0;
        while x + WIN <= w {
            let (mut sa, mut sb, mut saa, mut sbb, mut sab) = (0.0, 0.0, 0.0, 0.0, 0.0);
            for dy in 0..WIN {
                let row = (y + dy) * w + x;
                for i in row..row + WIN {
                    let (p, q) = (la[i], lb[i]);
                    sa += p;
                    sb += q;
                    saa += p * p;
                    sbb += q * q;
                    sab += p * q;
                }
            }
            let k = (WIN * WIN) as f64;
            let (ma, mb) = (sa / k, sb / k);
            let va = saa / k - ma * ma;
            let vb = sbb / k - mb * mb;
            let cov = sab / k - ma * mb;
            total += ((2.0 * ma * mb + C1) * (2.0 * cov + C2))
                / ((ma * ma + mb * mb + C1) * (va + vb + C2));
            count += 1;
            x += STRIDE;
        }
        y += STRIDE;
    }
    total / count as f64
}

/// Half-size box downsample.
fn half(img: &Image) -> Image {
    let (w, h) = ((img.width / 2).max(1), (img.height / 2).max(1));
    let src_w = img.width as usize;
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h as usize {
        for x in 0..w as usize {
            for c in 0..4 {
                let mut sum = 0u32;
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let sx = (2 * x + dx).min(img.width as usize - 1);
                    let sy = (2 * y + dy).min(img.height as usize - 1);
                    sum += u32::from(img.rgba[(sy * src_w + sx) * 4 + c]);
                }
                rgba.push((sum / 4) as u8);
            }
        }
    }
    Image {
        width: w,
        height: h,
        rgba,
    }
}

/// Reference on the left, ours on the right, each at half size.
pub fn side_by_side(reference: &Image, ours: &Image) -> Image {
    let (a, b) = (half(reference), half(ours));
    let (w, h) = (a.width + b.width, a.height.max(b.height));
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    for (img, x0) in [(&a, 0), (&b, a.width)] {
        for y in 0..img.height {
            let src = (y * img.width * 4) as usize;
            let dst = ((y * w + x0) * 4) as usize;
            let len = (img.width * 4) as usize;
            rgba[dst..dst + len].copy_from_slice(&img.rgba[src..src + len]);
        }
    }
    Image {
        width: w,
        height: h,
        rgba,
    }
}

/// Signed luma difference at half size: red where ours is brighter, blue where darker (x4),
/// over the reference dimmed to a quarter.
pub fn difference(reference: &Image, ours: &Image) -> Image {
    let (a, b) = (half(reference), half(ours));
    let rgba = a
        .rgba
        .chunks_exact(4)
        .zip(b.rgba.chunks_exact(4))
        .flat_map(|(p, q)| {
            let d = luma(q) - luma(p);
            let base = luma(p) * 0.25;
            let to_byte = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
            [
                to_byte(base + (4.0 * d).max(0.0)),
                to_byte(base),
                to_byte(base + (-4.0 * d).max(0.0)),
                255,
            ]
        })
        .collect();
    Image {
        width: a.width,
        height: a.height,
        rgba,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, rgb: [u8; 3]) -> Image {
        Image {
            width: w,
            height: h,
            rgba: (0..w * h)
                .flat_map(|_| [rgb[0], rgb[1], rgb[2], 255])
                .collect(),
        }
    }

    fn noise(w: u32, h: u32, seed: u32) -> Image {
        let mut state = seed;
        let rgba = (0..w * h)
            .flat_map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let v = (state >> 24) as u8;
                [v, v / 2, 255 - v, 255]
            })
            .collect();
        Image {
            width: w,
            height: h,
            rgba,
        }
    }

    #[test]
    fn identical_images_match_perfectly() {
        let img = noise(64, 48, 7);
        let m = compare(&img, &img).unwrap();
        assert_eq!(m.mae_linear, 0.0);
        assert!((m.ssim - 1.0).abs() < 1e-12);
        assert_eq!(m.histogram_distance, 0.0);
        assert!((m.lum_ratio() - 1.0).abs() < 1e-12);
        assert_eq!(m.bands[0][0], m.bands[0][1]);
    }

    #[test]
    fn black_against_white_is_the_largest_difference() {
        let m = compare(&solid(16, 16, [0; 3]), &solid(16, 16, [255; 3])).unwrap();
        assert!((m.mae_linear - 1.0).abs() < 1e-12);
        assert!(m.ssim < 0.01);
        // All mass moves across 63 of the 64 bins.
        assert!((m.histogram_distance - 63.0 / 64.0).abs() < 1e-12);
        assert!((m.mean_lum[1] - 1.0).abs() < 1e-12);
    }

    #[test]
    fn errors_are_measured_in_linear_light_per_channel() {
        // sRGB 188 is linear ~0.5: a red-only difference shows up in R alone.
        let m = compare(&solid(8, 8, [0, 10, 10]), &solid(8, 8, [188, 10, 10])).unwrap();
        assert!((m.mae_linear_rgb[0] - 0.5029).abs() < 1e-3, "{m:?}");
        assert_eq!(m.mae_linear_rgb[1], 0.0);
        assert!((m.mae_linear - m.mae_linear_rgb[0] / 3.0).abs() < 1e-12);
        let gb = (0.7152 + 0.0722) * linear(10);
        let expected = (0.2126 * linear(188) + gb) / gb;
        assert!((m.lum_ratio() - expected).abs() < 1e-9, "{m:?}");
    }

    #[test]
    fn a_brighter_copy_keeps_structure_but_not_exposure() {
        let a = noise(64, 64, 3);
        let b = Image {
            rgba: a.rgba.iter().map(|&v| v / 2).collect(),
            ..a
        };
        let a = noise(64, 64, 3);
        let m = compare(&a, &b).unwrap();
        assert!(m.lum_ratio() < 0.5);
        assert!(
            m.ssim > 0.3,
            "structure survives a brightness change: {}",
            m.ssim
        );
        assert!(m.histogram_distance > 0.1);
    }

    #[test]
    fn bands_split_sky_from_ground() {
        let mut img = solid(4, 6, [0; 3]);
        img.rgba[..2 * 4 * 4].fill(255);
        let m = compare(&img, &solid(4, 6, [0; 3])).unwrap();
        assert_eq!(m.bands[0][0], [1.0; 3]);
        assert_eq!(m.bands[1][0], [0.0; 3]);
        assert_eq!(m.bands[2][1], [0.0; 3]);
    }

    #[test]
    fn different_sizes_are_an_error() {
        assert!(compare(&solid(4, 4, [0; 3]), &solid(4, 5, [0; 3])).is_err());
    }

    #[test]
    fn side_by_side_and_difference_are_half_size() {
        let a = solid(8, 6, [200; 3]);
        let b = solid(8, 6, [100; 3]);
        let side = side_by_side(&a, &b);
        assert_eq!((side.width, side.height), (8, 3));
        assert_eq!(&side.rgba[..3], &[200; 3]);
        assert_eq!(&side.rgba[4 * 4..4 * 4 + 3], &[100; 3]);
        let diff = difference(&a, &b);
        assert_eq!((diff.width, diff.height), (4, 3));
        // Ours is darker: blue dominates.
        assert!(diff.rgba[2] > diff.rgba[0]);
    }

    #[test]
    fn metrics_serialise_as_json() {
        let img = solid(8, 8, [128; 3]);
        let json = compare(&img, &img).unwrap().to_json();
        assert!(json.starts_with('{') && json.ends_with('}'));
        assert!(json.contains("\"ssim\": 1.000000"));
        assert!(json.contains("\"bottom\": {\"arma\": ["));
    }
}
