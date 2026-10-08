//! Whether and with which Resolution LOD an object is drawn (`docs/re/render-lod.md`).
//!
//! From the engine (high confidence): the objects-quality setting gives a scene complexity and
//! four coefficients ([`LodCoefficients`]) interpolated over distance; an object's "visible area"
//! `A = size² · density · K` is compared with `T = coef²(d²) · d²` ([`visibility`]): dropped
//! below `0.95² T`, dithered in up to `1.05² T`, drawn above; shadows need `A ≥ 4 T`.
//!
//! Not traced yet, so behind named stand-ins: the global area multiplier `scene+0x8c4`
//! ([`LodSelector::area_scale`]) and the per-LOD value the engine compares to pick the LOD
//! ([`stand_in_lod_index`]).

use glam::Vec3;

/// What LOD selection needs to know about one Resolution LOD.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LodMetrics {
    /// The resolution value (lower is more detailed).
    pub resolution: f32,
    /// Number of faces.
    pub faces: u32,
}

/// The video option `ObjectsQuality` (`CfgVideoOptions`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectsQuality {
    VeryLow,
    Low,
    High,
    VeryHigh,
    Ultra,
    Extreme,
}

impl std::str::FromStr for ObjectsQuality {
    type Err = String;

    /// The `CfgVideoOptions` class name (`VeryLow`, `Low`, `High`, `VeryHigh`, `Ultra`,
    /// `Extreme`), any case.
    fn from_str(s: &str) -> Result<Self, String> {
        let all = [
            ("verylow", ObjectsQuality::VeryLow),
            ("low", ObjectsQuality::Low),
            ("high", ObjectsQuality::High),
            ("veryhigh", ObjectsQuality::VeryHigh),
            ("ultra", ObjectsQuality::Ultra),
            ("extreme", ObjectsQuality::Extreme),
        ];
        all.iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(s))
            .map(|(_, q)| *q)
            .ok_or_else(|| format!("unknown objects quality {s:?}"))
    }
}

impl ObjectsQuality {
    /// The `sceneComplexity` this setting stores.
    pub fn scene_complexity(self) -> f32 {
        match self {
            ObjectsQuality::VeryLow => 100_000.0,
            ObjectsQuality::Low => 600_000.0,
            ObjectsQuality::High => 900_000.0,
            ObjectsQuality::VeryHigh => 1_300_000.0,
            ObjectsQuality::Ultra => 1_800_000.0,
            ObjectsQuality::Extreme => 2_600_000.0,
        }
    }
}

/// The four LOD coefficients at 20, 200, 350 and 1500 m.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LodCoefficients {
    pub c: [f32; 4],
}

impl LodCoefficients {
    /// Knots of [`coef_squared`](Self::coef_squared), metres.
    const KNOTS: [f32; 4] = [20.0, 200.0, 350.0, 1500.0];

    /// The coefficients for a `sceneComplexity` (clamped to 1e4..=1e7 like the engine).
    pub fn from_scene_complexity(scene_complexity: f32) -> Self {
        let sc = scene_complexity.clamp(1.0e4, 1.0e7);
        let c1 = (1.0e6 / sc).sqrt().clamp(1.0, 2.0);
        let c2 = (1.2e6 / sc).powf(0.55).clamp(1.0, 4.0);
        let c3 = (3.0e6 / sc).powf(0.77).max(1.0);
        let c4 = c3.max(6.0);
        LodCoefficients {
            c: [c1, c2, c3, c4],
        }
    }

    /// `coef²` at squared distance `d2`, linear in `d²` between the knots.
    pub fn coef_squared(&self, d2: f32) -> f32 {
        let k = Self::KNOTS.map(|d| d * d);
        let c = self.c.map(|c| c * c);
        if d2 <= k[0] {
            return c[0];
        }
        for i in 1..4 {
            if d2 <= k[i] {
                let t = (d2 - k[i - 1]) / (k[i] - k[i - 1]);
                return c[i - 1] + t * (c[i] - c[i - 1]);
            }
        }
        c[3]
    }
}

/// The engine's object size: the mean half-extent of the bounding box times the object scale.
pub fn object_size(bbox_min: Vec3, bbox_max: Vec3, scale: f32) -> f32 {
    let e = bbox_max - bbox_min;
    (e.x + e.y + e.z) / 6.0 * scale
}

/// What LOD selection needs to know about one placed object.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ObjectBounds {
    /// [`object_size`] in metres.
    pub size: f32,
    /// The model's density factor (`lodDensityCoef`, 1 when unset).
    pub density: f32,
}

/// The camera's projection scale `K = 1/tan(fov_x/2) · 1/tan(fov_y/2)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewScale(pub f32);

impl ViewScale {
    /// From RV's `fovTop` and the viewport aspect ratio (width / height).
    pub fn new(fov_top: f32, aspect: f32) -> Self {
        ViewScale(1.0 / (fov_top * aspect) / fov_top)
    }
}

/// The outcome of the draw test.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Visibility {
    Hidden,
    /// Dithered in: 0 just appearing, 1 fully drawn.
    Fade(f32),
    Full,
}

/// The engine's draw test for visible area `a` against threshold `t`.
pub fn visibility(a: f32, t: f32) -> Visibility {
    let (lo, hi) = (0.9025 * t, 1.1025 * t);
    if a < lo {
        Visibility::Hidden
    } else if a >= hi {
        Visibility::Full
    } else {
        Visibility::Fade((a - lo) / (hi - lo))
    }
}

/// The LOD index for an object whose visible area is `relative_size` times the draw threshold.
///
/// **Stand-in** for the engine's per-LOD comparison (`render-lod.md` §5, not decoded): draw the
/// most detailed LOD whose face count fits `relative_size · faces_per_unit`. The doc's suggested
/// `(size / resolution)²` value is not used: compared the way §5 describes, it would pick coarser
/// LODs for nearer objects, so its sign or meaning is still unclear.
pub fn stand_in_lod_index(lods: &[LodMetrics], relative_size: f32, faces_per_unit: f32) -> usize {
    let budget = relative_size * faces_per_unit;
    lods.iter()
        .position(|l| l.faces as f32 <= budget)
        .unwrap_or(lods.len().saturating_sub(1))
}

/// The LOD policy.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LodSelector {
    pub coefficients: LodCoefficients,
    /// Stand-in for the engine's global area multiplier (`scene+0x8c4`, not traced): chosen so
    /// that a small house stays visible to about 1.5 km and a 20 cm stone to about 200 m.
    pub area_scale: f32,
    /// Faces an object may draw per unit of [`stand_in_lod_index`]'s relative size.
    pub faces_per_unit: f32,
}

impl Default for LodSelector {
    fn default() -> Self {
        LodSelector::new(ObjectsQuality::High)
    }
}

impl LodSelector {
    pub fn new(quality: ObjectsQuality) -> Self {
        LodSelector {
            coefficients: LodCoefficients::from_scene_complexity(quality.scene_complexity()),
            area_scale: 4.0e6,
            faces_per_unit: 50.0,
        }
    }

    fn area(&self, object: ObjectBounds, view: ViewScale) -> f32 {
        object.size * object.size * object.density * view.0 * self.area_scale
    }

    fn threshold(&self, distance: f32) -> f32 {
        let d2 = distance * distance;
        self.coefficients.coef_squared(d2) * d2
    }

    /// The LOD index (into `lods`, most detailed first) and fade of an object at `distance`
    /// metres, or `None` when it is not drawn.
    pub fn select(
        &self,
        lods: &[LodMetrics],
        object: ObjectBounds,
        distance: f32,
        view: ViewScale,
    ) -> Option<(usize, Visibility)> {
        if lods.is_empty() {
            return None;
        }
        let a = self.area(object, view);
        let t = self.threshold(distance);
        if t <= 0.0 {
            return Some((0, Visibility::Full));
        }
        match visibility(a, t) {
            Visibility::Hidden => None,
            v => Some((stand_in_lod_index(lods, a / t, self.faces_per_unit), v)),
        }
    }

    /// Whether the object casts a sun shadow at `distance` (needs `A ≥ 4 T`).
    pub fn casts_shadow(&self, object: ObjectBounds, distance: f32, view: ViewScale) -> bool {
        self.area(object, view) >= 4.0 * self.threshold(distance)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() <= 1e-3 * b.abs().max(1.0)
    }

    #[test]
    fn scene_complexity_gives_the_four_coefficients() {
        // sceneComplexity 1e6: c1 = 1, c2 = 1.2^0.55, c3 = 3^0.77, c4 = 6.
        let c = LodCoefficients::from_scene_complexity(1.0e6);
        assert!(close(c.c[0], 1.0));
        assert!(close(c.c[1], 1.10548));
        assert!(close(c.c[2], 2.33013));
        assert!(close(c.c[3], 6.0));
        // Very low complexity hits the clamps: c1 <= 2, c2 <= 4; c3 and c4 are unbounded.
        let low = LodCoefficients::from_scene_complexity(1.0e4);
        assert!(close(low.c[0], 2.0));
        assert!(close(low.c[1], 4.0));
        assert!(close(low.c[2], 80.83));
        assert!(close(low.c[3], 80.83));
        assert!(close(ObjectsQuality::High.scene_complexity(), 900_000.0));
    }

    #[test]
    fn the_coefficient_is_linear_in_squared_distance_between_knots() {
        let c = LodCoefficients {
            c: [1.0, 2.0, 3.0, 6.0],
        };
        assert!(close(c.coef_squared(10.0 * 10.0), 1.0));
        assert!(close(c.coef_squared(20.0 * 20.0), 1.0));
        assert!(close(c.coef_squared(200.0 * 200.0), 4.0));
        // Halfway between 20^2 and 200^2.
        let mid = (400.0 + 40_000.0) / 2.0;
        assert!(close(c.coef_squared(mid), 2.5));
        assert!(close(c.coef_squared(350.0 * 350.0), 9.0));
        assert!(close(c.coef_squared(5_000.0 * 5_000.0), 36.0));
    }

    #[test]
    fn object_size_is_the_mean_half_extent_times_scale() {
        let r = object_size(Vec3::new(-1.0, 0.0, -2.0), Vec3::new(1.0, 6.0, 2.0), 2.0);
        // (2 + 6 + 4) / 6 * 2 = 4.
        assert!(close(r, 4.0));
    }

    #[test]
    fn draw_test_drops_fades_and_draws_around_the_threshold() {
        assert_eq!(visibility(0.90, 1.0), Visibility::Hidden);
        assert_eq!(visibility(1.2, 1.0), Visibility::Full);
        let Visibility::Fade(f) = visibility(1.0, 1.0) else {
            panic!("between 0.95^2 and 1.05^2 the object fades");
        };
        assert!(close(f, 0.4875));
    }

    /// A model with 8000, 2000, 500 and 100 faces in its resolution LODs.
    fn lods() -> Vec<LodMetrics> {
        [(1.0, 8000), (2.0, 2000), (3.0, 500), (4.0, 100)]
            .map(|(resolution, faces)| LodMetrics { resolution, faces })
            .to_vec()
    }

    fn house() -> ObjectBounds {
        ObjectBounds {
            size: 6.0,
            density: 1.0,
        }
    }

    #[test]
    fn near_objects_get_fine_lods_and_far_ones_coarse_lods() {
        let s = LodSelector::default();
        let view = ViewScale::new(0.75, 16.0 / 9.0);
        let mut previous = 0;
        for d in [5.0, 30.0, 80.0, 200.0, 500.0, 1200.0] {
            let (lod, _) = s.select(&lods(), house(), d, view).expect("drawn");
            assert!(lod >= previous, "LOD went back to {lod} at {d} m");
            previous = lod;
        }
        assert_eq!(s.select(&lods(), house(), 5.0, view).unwrap().0, 0);
        assert_eq!(previous, 3);
    }

    #[test]
    fn small_far_objects_are_not_drawn() {
        let s = LodSelector::default();
        let view = ViewScale::new(0.75, 16.0 / 9.0);
        let pebble = ObjectBounds {
            size: 0.1,
            density: 1.0,
        };
        assert!(s.select(&lods(), pebble, 5.0, view).is_some());
        assert!(s.select(&lods(), pebble, 600.0, view).is_none());
        assert!(s.select(&[], house(), 5.0, view).is_none());
    }

    #[test]
    fn higher_scene_complexity_keeps_detail_longer() {
        let view = ViewScale::new(0.75, 16.0 / 9.0);
        let low = LodSelector::new(ObjectsQuality::Low);
        let high = LodSelector::new(ObjectsQuality::Ultra);
        let at = |s: &LodSelector| s.select(&lods(), house(), 150.0, view).unwrap().0;
        assert!(at(&high) <= at(&low));
        let tiny = ObjectBounds {
            size: 0.3,
            density: 1.0,
        };
        // Some distance where Low drops the object but Ultra still draws it.
        let d = (10..2000)
            .map(|d| d as f32)
            .find(|&d| low.select(&lods(), tiny, d, view).is_none())
            .unwrap();
        assert!(high.select(&lods(), tiny, d, view).is_some());
    }

    #[test]
    fn shadows_need_twice_the_screen_size() {
        let s = LodSelector::default();
        let view = ViewScale::new(0.75, 16.0 / 9.0);
        let size = house();
        let d_draw = (1..20_000)
            .map(|d| d as f32)
            .find(|&d| s.select(&lods(), size, d, view).is_none())
            .unwrap();
        let d_shadow = (1..20_000)
            .map(|d| d as f32)
            .find(|&d| !s.casts_shadow(size, d, view))
            .unwrap();
        assert!(d_shadow < d_draw * 0.6, "{d_shadow} vs {d_draw}");
    }
}
