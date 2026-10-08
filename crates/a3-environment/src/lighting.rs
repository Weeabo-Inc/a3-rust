//! The world's lighting table (`CfgWorlds >> W >> Weather >> LightingNew`).
//!
//! Each entry gives the light at one (height, overcast, sun angle). The engine keys entries
//! by `sin(sunAngle)`, groups them by height and overcast, and interpolates linearly (all
//! colours in linear space): first by sin(sun angle) within an overcast group, then between
//! overcast groups, then between height groups, clamping outside the table
//! (`docs/re/environment.md`).

use a3_config::{ConfigRef, Value};
use glam::Vec3;

use crate::cfg::{classes, number, number_or, value_number};

/// Rec. 601 luma, the engine's luminance for `{colour, EV}` entries.
fn luma(c: Vec3) -> f32 {
    c.x * 0.299 + c.y * 0.587 + c.z * 0.114
}

/// A `{{r, g, b}, ev}` colour: `rgb` rescaled to a luminance of `2^ev`.
pub fn ev_color(rgb: Vec3, ev: f32) -> Vec3 {
    let l = luma(rgb);
    if l == 0.0 { rgb } else { rgb * (ev.exp2() / l) }
}

/// Reads a colour entry: `{r, g, b}`, `{r, g, b, a}` or `{{r, g, b}, ev}`.
fn color(c: &ConfigRef<'_>, name: &str) -> Option<Vec3> {
    let e = c.get(name);
    if !e.is_array() {
        return None;
    }
    let values = e.array();
    let nums = |vs: &[Value]| -> Vec<f32> { vs.iter().filter_map(value_number).collect() };
    match values.as_slice() {
        [Value::Array(rgb), ev] => {
            let rgb = nums(rgb);
            let ev = value_number(ev)?;
            (rgb.len() >= 3).then(|| ev_color(Vec3::new(rgb[0], rgb[1], rgb[2]), ev))
        }
        _ => {
            let n = nums(&values);
            (n.len() >= 3).then(|| Vec3::new(n[0], n[1], n[2]))
        }
    }
}

/// The light at one point of the table.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LightingEntry {
    /// `height`: the group key (0 above water, below 0 under water).
    pub height: f32,
    /// `overcast`: the group key.
    pub overcast: f32,
    /// sin(`sunAngle`): the key within a group.
    pub sin_sun_angle: f32,
    /// `sunOrMoon`: 1 lit by the sun, 0 by the moon.
    pub sun_or_moon: f32,
    /// Direct light (`diffuse`).
    pub diffuse: Vec3,
    /// Direct light with the sun behind clouds (`diffuseCloud`).
    pub diffuse_cloud: Vec3,
    /// Sky light from above (`ambient`).
    pub ambient: Vec3,
    /// `ambientCloud`.
    pub ambient_cloud: Vec3,
    /// Sky light at the horizon (`ambientMid`; default halfway between `ambient` and
    /// `groundReflection`).
    pub ambient_mid: Vec3,
    /// `ambientMidCloud`.
    pub ambient_mid_cloud: Vec3,
    /// Light reflected from the ground (`groundReflection`).
    pub ground_reflection: Vec3,
    /// `groundReflectionCloud`.
    pub ground_reflection_cloud: Vec3,
    /// `bidirect`.
    pub bidirect: Vec3,
    /// `bidirectCloud`.
    pub bidirect_cloud: Vec3,
    /// Sky colour (`sky`).
    pub sky: Vec3,
    /// Sky colour around the sun (`skyAroundSun`).
    pub sky_around_sun: Vec3,
    /// Fog colour (`fogColor`).
    pub fog_color: Vec3,
    /// `apertureMin`.
    pub aperture_min: f32,
    /// `apertureStandard`.
    pub aperture_standard: f32,
    /// `apertureMax`.
    pub aperture_max: f32,
    /// `standardAvgLum`.
    pub standard_avg_lum: f32,
    /// `rayleigh` scattering coefficients.
    pub rayleigh: Vec3,
    /// `mie` scattering coefficients.
    pub mie: Vec3,
    /// `cloudsColor`.
    pub clouds_color: Vec3,
    /// `swBrightness`.
    pub sw_brightness: f32,
}

impl LightingEntry {
    /// Reads one `LightingN` class.
    pub fn from_config(c: &ConfigRef<'_>) -> Self {
        let col = |name: &str| color(c, name).unwrap_or(Vec3::ZERO);
        let ambient = col("ambient");
        let ambient_cloud = col("ambientCloud");
        let ground = col("groundReflection");
        let ground_cloud = col("groundReflectionCloud");
        let sky = col("sky");
        LightingEntry {
            height: number_or(c, "height", 0.0),
            overcast: number_or(c, "overcast", 0.0),
            sin_sun_angle: number_or(c, "sunAngle", 0.0).to_radians().sin(),
            sun_or_moon: number_or(c, "sunOrMoon", 1.0),
            diffuse: col("diffuse"),
            diffuse_cloud: col("diffuseCloud"),
            ambient,
            ambient_cloud,
            ambient_mid: color(c, "ambientMid").unwrap_or((ambient + ground) * 0.5),
            ambient_mid_cloud: color(c, "ambientMidCloud")
                .unwrap_or((ambient_cloud + ground_cloud) * 0.5),
            ground_reflection: ground,
            ground_reflection_cloud: ground_cloud,
            bidirect: col("bidirect"),
            bidirect_cloud: col("bidirectCloud"),
            sky,
            sky_around_sun: color(c, "skyAroundSun").unwrap_or(sky),
            fog_color: color(c, "fogColor").unwrap_or(sky),
            aperture_min: number_or(c, "apertureMin", 1.0),
            aperture_standard: number_or(c, "apertureStandard", 1.0),
            aperture_max: number_or(c, "apertureMax", 1.0),
            standard_avg_lum: number_or(c, "standardAvgLum", 1.0),
            rayleigh: color(c, "rayleigh").unwrap_or(Vec3::new(0.0075, 0.0139, 0.0288)),
            mie: color(c, "mie").unwrap_or(Vec3::splat(0.0046)),
            clouds_color: col("cloudsColor"),
            sw_brightness: number(c, "swBrightness").unwrap_or(1.0),
        }
    }

    /// Linear interpolation `self + (other - self) * t` of every field.
    pub fn lerp(&self, other: &Self, t: f32) -> Self {
        let f = |a: f32, b: f32| a + (b - a) * t;
        let v = |a: Vec3, b: Vec3| a + (b - a) * t;
        LightingEntry {
            height: f(self.height, other.height),
            overcast: f(self.overcast, other.overcast),
            sin_sun_angle: f(self.sin_sun_angle, other.sin_sun_angle),
            sun_or_moon: f(self.sun_or_moon, other.sun_or_moon),
            diffuse: v(self.diffuse, other.diffuse),
            diffuse_cloud: v(self.diffuse_cloud, other.diffuse_cloud),
            ambient: v(self.ambient, other.ambient),
            ambient_cloud: v(self.ambient_cloud, other.ambient_cloud),
            ambient_mid: v(self.ambient_mid, other.ambient_mid),
            ambient_mid_cloud: v(self.ambient_mid_cloud, other.ambient_mid_cloud),
            ground_reflection: v(self.ground_reflection, other.ground_reflection),
            ground_reflection_cloud: v(self.ground_reflection_cloud, other.ground_reflection_cloud),
            bidirect: v(self.bidirect, other.bidirect),
            bidirect_cloud: v(self.bidirect_cloud, other.bidirect_cloud),
            sky: v(self.sky, other.sky),
            sky_around_sun: v(self.sky_around_sun, other.sky_around_sun),
            fog_color: v(self.fog_color, other.fog_color),
            aperture_min: f(self.aperture_min, other.aperture_min),
            aperture_standard: f(self.aperture_standard, other.aperture_standard),
            aperture_max: f(self.aperture_max, other.aperture_max),
            standard_avg_lum: f(self.standard_avg_lum, other.standard_avg_lum),
            rayleigh: v(self.rayleigh, other.rayleigh),
            mie: v(self.mie, other.mie),
            clouds_color: v(self.clouds_color, other.clouds_color),
            sw_brightness: f(self.sw_brightness, other.sw_brightness),
        }
    }
}

/// Entries of one overcast, sorted by sin(sun angle).
type OvercastGroup = (f32, Vec<LightingEntry>);

/// Nested groups: height, then overcast, then entries sorted by sin(sun angle).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LightingTable {
    heights: Vec<(f32, Vec<OvercastGroup>)>,
}

/// Keys within this distance are the same group (the engine's tolerance).
const SAME_KEY: f32 = 1e-4;

/// The bracketing indices of `x` among ascending `keys` and the blend factor, as the engine
/// computes them: below the first key and above the last the end entry is used.
fn bracket(keys: &[f32], x: f32) -> (usize, usize, f32) {
    let above = keys.iter().position(|&k| x < k).unwrap_or(keys.len());
    let lo = above.saturating_sub(1);
    let hi = above.min(keys.len() - 1);
    let span = keys[hi] - keys[lo];
    let t = if lo != hi && span > 0.0 {
        ((x - keys[lo]) / span).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (lo, hi, t)
}

/// Picks or blends two values the way the engine does: below 0.0001 the lower, above 0.9999
/// the upper, otherwise a lerp.
fn blend<T>(
    lo: usize,
    hi: usize,
    t: f32,
    get: impl Fn(usize) -> T,
    lerp: impl Fn(&T, &T, f32) -> T,
) -> T {
    if t < 0.0001 {
        get(lo)
    } else if t > 0.9999 {
        get(hi)
    } else {
        lerp(&get(lo), &get(hi), t)
    }
}

impl LightingTable {
    /// Reads the `LightingNew` class (each child class one entry).
    pub fn from_config(c: &ConfigRef<'_>) -> Self {
        let mut table = LightingTable::default();
        for entry in classes(c) {
            table.insert(LightingEntry::from_config(&entry));
        }
        table
    }

    /// Adds an entry into its height and overcast group, keeping all levels sorted.
    pub fn insert(&mut self, entry: LightingEntry) {
        let heights = &mut self.heights;
        let h = match heights
            .iter()
            .position(|(k, _)| (k - entry.height).abs() <= SAME_KEY)
        {
            Some(i) => i,
            None => {
                let at = heights
                    .iter()
                    .position(|(k, _)| entry.height < *k)
                    .unwrap_or(heights.len());
                heights.insert(at, (entry.height, Vec::new()));
                at
            }
        };
        let overcasts = &mut heights[h].1;
        let o = match overcasts
            .iter()
            .position(|(k, _)| (k - entry.overcast).abs() <= SAME_KEY)
        {
            Some(i) => i,
            None => {
                let at = overcasts
                    .iter()
                    .position(|(k, _)| entry.overcast < *k)
                    .unwrap_or(overcasts.len());
                overcasts.insert(at, (entry.overcast, Vec::new()));
                at
            }
        };
        let entries = &mut overcasts[o].1;
        let at = entries
            .iter()
            .position(|e| entry.sin_sun_angle < e.sin_sun_angle)
            .unwrap_or(entries.len());
        entries.insert(at, entry);
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.heights
            .iter()
            .flat_map(|(_, o)| o.iter().map(|(_, e)| e.len()))
            .sum()
    }

    /// `true` without entries.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The light at `height` (0 above water), `overcast` (the weather's lighting overcast)
    /// and `sin_sun_angle`. A default (dark) entry for an empty table.
    pub fn sample(&self, height: f32, overcast: f32, sin_sun_angle: f32) -> LightingEntry {
        if self.is_empty() {
            return LightingEntry::default();
        }
        let hkeys: Vec<f32> = self.heights.iter().map(|(k, _)| *k).collect();
        let (lo, hi, t) = bracket(&hkeys, height);
        blend(
            lo,
            hi,
            t,
            |i| sample_overcast(&self.heights[i].1, overcast, sin_sun_angle),
            LightingEntry::lerp,
        )
    }
}

fn sample_overcast(groups: &[OvercastGroup], overcast: f32, sin_sun_angle: f32) -> LightingEntry {
    let keys: Vec<f32> = groups.iter().map(|(k, _)| *k).collect();
    let (lo, hi, t) = bracket(&keys, overcast);
    blend(
        lo,
        hi,
        t,
        |i| sample_sun(&groups[i].1, sin_sun_angle),
        LightingEntry::lerp,
    )
}

fn sample_sun(entries: &[LightingEntry], sin_sun_angle: f32) -> LightingEntry {
    let keys: Vec<f32> = entries.iter().map(|e| e.sin_sun_angle).collect();
    let (lo, hi, t) = bracket(&keys, sin_sun_angle);
    blend(lo, hi, t, |i| entries[i], LightingEntry::lerp)
}
