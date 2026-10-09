//! Whether and with which Resolution LOD an object is drawn (`docs/re/render-lod.md`).
//!
//! The engine's pipeline, per frame:
//! - the objects-quality setting gives a scene complexity and four coefficients
//!   ([`LodCoefficients`]) interpolated over distance;
//! - the draw test ([`LodSelector::draw_test`]) compares an object's screen size
//!   `A = r² · drawImportance · K` with `T = coef²(d²) · d²`: dropped below `0.95² T`, dithered in
//!   up to `1.05² T`, drawn above; the object view distance caps `A`, which fades objects out;
//! - every drawn object's bounding square on screen, in pixels ([`LodSelector::lod_area`]),
//!   rounded to a base-1.5 area class ([`area_class`]), claims a share of a frame-wide budget of
//!   `sceneComplexity` faces ([`LodSelector::assign`]); each object draws the finest LOD that
//!   fits its share ([`pick_lod`]).
//!
//! `K` ([`ViewScale`]) is the render target's pixel count over `tan(fovX/2) · tan(fovY/2)`.

use glam::Vec3;

/// What LOD selection needs to know about one Resolution LOD.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LodMetrics {
    /// The resolution value (lower is more detailed).
    pub resolution: f32,
    /// The LOD's face count as the engine counts it: polygons (a quad is one face), proxy
    /// triangles included. The ODOL LOD summary's `face_count`.
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
    /// Bounding sphere radius around the object position, in metres (scaled).
    pub radius: f32,
    /// The model's `drawImportance` (1 when unset): scales the size in the draw test.
    pub draw_importance: f32,
    /// The model's `lodDensityCoef` (1 when unset): scales the area in the LOD budget.
    pub lod_density: f32,
}

/// The view scale `K`: the render target's pixel count over `tan(fovX/2) · tan(fovY/2)`, so
/// that `K · r² / d²` is the area in pixels of a square of side `2r` at distance `d`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewScale(pub f32);

impl ViewScale {
    /// From RV's `fovTop` (`tan` of half the vertical field of view) and the render target
    /// size in pixels; `fovLeft` follows from the aspect ratio.
    pub fn new(fov_top: f32, width: u32, height: u32) -> Self {
        let (w, h) = (width.max(1) as f32, height.max(1) as f32);
        let fov_left = fov_top * w / h;
        ViewScale(w * h / (fov_left * fov_top))
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

/// The distance the LOD budget and the shadow test use: to the bounding sphere's surface near
/// the object, blending back to the centre distance `d` between 4 and 10 radii, and never
/// nearer than the near plane.
pub fn surface_distance(d: f32, radius: f32, near: f32) -> f32 {
    let mut t = d;
    if radius > 0.0 && d < 10.0 * radius {
        let f = d / radius;
        t = (d - radius).max(0.0);
        if d > 4.0 * radius {
            let s = (f - 4.0) / 6.0;
            t = d * s + (1.0 - s) * t;
        }
    }
    t.max(near)
}

/// Area classes are powers of this ratio.
const CLASS_RATIO: f32 = 1.5;
/// Number of area classes.
const CLASSES: u8 = 40;

/// The base-1.5 area class of `area` (pixels), rounded with a ±0.25 hysteresis against the
/// object's class last frame. `None` (a new object) rounds as if coming from below.
pub fn area_class(area: f32, previous: Option<u8>) -> u8 {
    let area = if area > 1.0e-20 { area } else { 1.0e-20 };
    let x = area.ln() / CLASS_RATIO.ln();
    let lo = (x - 0.5).round();
    let hi = lo + 1.0;
    let prev = previous.map_or(f32::MIN, f32::from);
    let class = if prev < hi {
        if x > hi - 0.25 { hi } else { lo }
    } else if x < lo + 0.25 {
        lo
    } else {
        hi
    };
    class.clamp(0.0, f32::from(CLASSES - 1)) as u8
}

/// The area the LOD budget uses for an area class: `1.5^class` pixels.
pub fn class_area(class: u8) -> f32 {
    CLASS_RATIO.powi(i32::from(class))
}

/// The engine's LOD pick (`0x141273010`): the finest LOD that a budget of `budget` faces
/// affords. LOD `i − 1` replaces `i` once the budget reaches 70 % of the way from `F[i]` to
/// `F[i−1]`; an object that was at `i` or coarser last frame needs 30 % more before it refines.
pub fn pick_lod(lods: &[LodMetrics], budget: f32, previous: Option<usize>) -> Option<usize> {
    if lods.is_empty() {
        return None;
    }
    let mix = |a: u32, b: u32| (a as f32 * 0.7 + b as f32 * 0.3) as i32 as f32;
    for i in (1..lods.len()).rev() {
        let (coarse, fine) = (lods[i].faces, lods[i - 1].faces);
        if budget < mix(coarse, fine) {
            return Some(i);
        }
        if previous.is_some_and(|p| p >= i) && budget < mix(fine, coarse) {
            return Some(i);
        }
    }
    Some(0)
}

/// One group of objects for the frame's LOD budget: instances of one model in one area class.
#[derive(Debug, Clone, PartialEq)]
pub struct LodRequest<'a> {
    /// The model's Resolution LODs, most detailed first.
    pub lods: &'a [LodMetrics],
    /// [`LodSelector::lod_area`] of the group (pixels).
    pub area: f32,
    /// The area class of `area` ([`area_class`]).
    pub class: u8,
    /// How many instances share the request.
    pub instances: u32,
    /// The shadow passes' share of the cost: 1/32 for a shadow-map caster.
    pub shadow_share: f32,
    /// The LOD drawn last frame, for the hysteresis of [`pick_lod`].
    pub previous: Option<usize>,
    /// The result of [`LodSelector::assign`].
    pub lod: Option<usize>,
}

impl<'a> LodRequest<'a> {
    /// A request for one instance at `area` pixels, with its class and LOD last frame.
    pub fn new(lods: &'a [LodMetrics], area: f32, previous_class: Option<u8>) -> Self {
        LodRequest {
            lods,
            area,
            class: area_class(area, previous_class),
            instances: 1,
            shadow_share: 0.0,
            previous: None,
            lod: None,
        }
    }

    fn class_area(&self) -> f32 {
        class_area(self.class)
    }
}

/// The share of the budget per pixel never exceeds this many faces.
const MAX_FACES_PER_PIXEL: f32 = 0.3;
/// Nor drops below this.
const MIN_FACES_PER_PIXEL: f32 = 0.03;

/// The LOD policy.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LodSelector {
    pub coefficients: LodCoefficients,
    /// `sceneComplexity`: the frame's face budget.
    pub scene_complexity: f32,
    /// The overall view distance (metres); reserves `(0.02 · d)²` faces of the budget.
    pub view_distance: f32,
    /// The object view distance (metres): objects fade out as they reach it.
    pub object_view_distance: f32,
}

impl Default for LodSelector {
    fn default() -> Self {
        LodSelector::new(ObjectsQuality::High)
    }
}

impl LodSelector {
    pub fn new(quality: ObjectsQuality) -> Self {
        let scene_complexity = quality.scene_complexity();
        LodSelector {
            coefficients: LodCoefficients::from_scene_complexity(scene_complexity),
            scene_complexity,
            view_distance: 1600.0,
            object_view_distance: 1600.0,
        }
    }

    /// The draw test of an object at `distance` metres.
    pub fn draw_test(&self, object: ObjectBounds, distance: f32, view: ViewScale) -> Visibility {
        let d2 = distance * distance;
        let coef2 = self.coefficients.coef_squared(d2);
        let t = coef2 * d2;
        if t <= 0.0 {
            return Visibility::Full;
        }
        let reach = object.radius + self.object_view_distance;
        let a = (object.size * object.size * object.draw_importance * view.0)
            .min(reach * reach * coef2 / 1.1025);
        visibility(a, t)
    }

    /// The object's bounding square on screen in pixels, times its `lodDensityCoef`, at
    /// [`surface_distance`] `d_surface`; at most twice the screen.
    pub fn lod_area(&self, object: ObjectBounds, d_surface: f32, view: ViewScale) -> f32 {
        let r2 = object.size * object.size;
        let d2 = d_surface * d_surface;
        let area = if r2 >= 2.0 * d2 {
            2.0 * view.0
        } else {
            view.0 * r2 / d2
        };
        area * object.lod_density
    }

    /// Whether the object casts a sun shadow, at [`surface_distance`] `d_surface`: its screen
    /// size must reach four times the draw threshold.
    pub fn casts_shadow(&self, object: ObjectBounds, d_surface: f32, view: ViewScale) -> bool {
        let d2 = d_surface * d_surface;
        view.0 * object.size * object.size >= 4.0 * self.coefficients.coef_squared(d2) * d2
    }

    /// The faces per pixel the budget left after `fixed` faces affords over `total` pixels.
    fn ratio(&self, fixed: f32, total: f32) -> f32 {
        ((self.scene_complexity - fixed) / total.max(1.0e-20))
            .clamp(MIN_FACES_PER_PIXEL, MAX_FACES_PER_PIXEL)
    }

    /// Shares the frame's face budget between `requests` and sets each one's
    /// [`LodRequest::lod`] (`0x141260900`). Returns the final faces-per-pixel ratio, which
    /// [`pick`](Self::pick) applies to objects outside the budget.
    pub fn assign(&self, requests: &mut [LodRequest<'_>]) -> f32 {
        let reserved = (self.view_distance * 0.02).powi(2).round();
        let mut fixed = reserved;
        let mut total = 0.0;
        for r in requests.iter_mut() {
            r.lod = None;
            let Some(coarsest) = r.lods.last() else {
                continue;
            };
            let n = r.instances as f32;
            // Not even the coarsest LOD fits the most generous share: it is forced.
            if r.area * MAX_FACES_PER_PIXEL < coarsest.faces as f32 {
                r.lod = Some(r.lods.len() - 1);
                fixed += n * coarsest.faces as f32;
            } else {
                total += n * r.class_area() * (1.0 + r.shadow_share);
            }
        }
        let initial = total;
        let mut ratio = self.ratio(fixed, total);
        // Pass 1: the groups that get their finest LOD leave the budget at that cost.
        for r in requests.iter_mut().filter(|r| r.lod.is_none()) {
            if pick_lod(r.lods, r.class_area() * ratio, r.previous) == Some(0) {
                let n = r.instances as f32;
                r.lod = Some(0);
                fixed += n * r.lods[0].faces as f32;
                total -= n * r.class_area();
            }
        }
        if total > initial * 1.0e-4 {
            ratio = self.ratio(fixed, total);
        }
        // Pass 2: the rest share what is left.
        for r in requests.iter_mut().filter(|r| r.lod.is_none()) {
            r.lod = pick_lod(r.lods, r.class_area() * ratio, r.previous);
        }
        ratio
    }

    /// The LOD for an object outside the budget (a proxy), at the frame's `ratio`.
    pub fn pick(
        &self,
        lods: &[LodMetrics],
        area: f32,
        ratio: f32,
        previous: Option<usize>,
    ) -> Option<usize> {
        pick_lod(lods, class_area(area_class(area, None)) * ratio, previous)
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

    #[test]
    fn the_view_scale_is_pixels_over_the_field_of_view() {
        // 90 degrees vertically (tan 45 = 1) on a square 1000 x 1000 target: K = 1e6. A 2 m
        // square at 10 m spans 0.2 of the 2-unit-wide view plane, 100 x 100 pixels:
        // K · 1² / 10² = 1e4.
        let k = ViewScale::new(1.0, 1000, 1000);
        assert!(close(k.0, 1.0e6));
        // The same field of view on a wider target: fovLeft grows with the aspect ratio.
        let wide = ViewScale::new(1.0, 2000, 1000);
        assert!(close(wide.0, 1.0e6));
        // Zooming in (smaller tangent) raises K quadratically.
        assert!(close(ViewScale::new(0.5, 1000, 1000).0, 4.0e6));
    }

    #[test]
    fn surface_distance_measures_to_the_bounding_sphere_near_the_object() {
        // Inside the sphere: the near plane.
        assert!(close(surface_distance(1.0, 2.0, 0.1), 0.1));
        // Within 4 radii: to the surface.
        assert!(close(surface_distance(6.0, 2.0, 0.1), 4.0));
        // Between 4 and 10 radii it blends back to the centre distance: at 7 radii halfway.
        assert!(close(surface_distance(14.0, 2.0, 0.1), 13.0));
        // From 10 radii on, the centre distance.
        assert!(close(surface_distance(20.0, 2.0, 0.1), 20.0));
        assert!(close(surface_distance(50.0, 2.0, 0.1), 50.0));
    }

    #[test]
    fn area_classes_are_powers_of_one_and_a_half_with_hysteresis() {
        assert!(close(class_area(0), 1.0));
        assert!(close(class_area(10), 1.5f32.powi(10)));
        // x = 10.6: a new object rounds up only past 10.75.
        let a = 1.5f32.powf(10.6);
        assert_eq!(area_class(a, None), 10);
        assert_eq!(area_class(1.5f32.powf(10.8), None), 11);
        // Coming from above (last frame 11), it stays at 11 down to 10.25.
        assert_eq!(area_class(a, Some(11)), 11);
        assert_eq!(area_class(1.5f32.powf(10.2), Some(11)), 10);
        // Clamped to 0..=39.
        assert_eq!(area_class(0.0, None), 0);
        assert_eq!(area_class(1.0e30, None), 39);
    }

    /// A model with 8000, 2000, 500 and 100 faces in its resolution LODs.
    fn lods() -> Vec<LodMetrics> {
        [(1.0, 8000), (2.0, 2000), (3.0, 500), (4.0, 100)]
            .map(|(resolution, faces)| LodMetrics { resolution, faces })
            .to_vec()
    }

    #[test]
    fn pick_lod_refines_at_seventy_percent_and_falls_back_at_thirty() {
        let l = lods();
        // From LOD 3 (100) to 2 (500): 0.7*100 + 0.3*500 = 220.
        assert_eq!(pick_lod(&l, 219.0, None), Some(3));
        assert_eq!(pick_lod(&l, 220.0, None), Some(2));
        // From 1 (2000) to 0 (8000): 0.7*2000 + 0.3*8000 = 3800.
        assert_eq!(pick_lod(&l, 3799.0, None), Some(1));
        assert_eq!(pick_lod(&l, 3800.0, None), Some(0));
        // Last frame at LOD 1 or coarser: LOD 0 needs 0.7*8000 + 0.3*2000 = 6200.
        assert_eq!(pick_lod(&l, 6199.0, Some(1)), Some(1));
        assert_eq!(pick_lod(&l, 6200.0, Some(1)), Some(0));
        // Last frame finer: no hysteresis on the way down to 1.
        assert_eq!(pick_lod(&l, 5000.0, Some(0)), Some(0));
        assert_eq!(pick_lod(&[], 1.0e9, None), None);
        assert_eq!(pick_lod(&l[..1], 0.0, None), Some(0));
    }

    fn request(lods: &[LodMetrics], area: f32) -> LodRequest<'_> {
        LodRequest::new(lods, area, None)
    }

    #[test]
    fn a_light_scene_gets_point_three_faces_per_pixel() {
        let l = lods();
        let s = LodSelector::default();
        // One object of class area 1.5^20 = 3325 pixels: 997 faces at 0.3 per pixel. LOD 1
        // needs 0.7·500 + 0.3·2000 = 950, LOD 0 needs 3800: LOD 1.
        let mut r = [request(&l, class_area(20))];
        let ratio = s.assign(&mut r);
        assert!(close(ratio, 0.3));
        assert_eq!(r[0].lod, Some(1));
    }

    #[test]
    fn a_crowded_scene_shares_the_budget() {
        let l = lods();
        let mut s = LodSelector::new(ObjectsQuality::VeryLow); // 100 000 faces
        s.view_distance = 0.0;
        // 100 objects of 1.5^20 pixels each: 332 500 pixels want 99 750 faces at 0.3, less
        // than the budget; 1000 of them must make do with 0.03..0.3.
        let mut few: Vec<_> = (0..100).map(|_| request(&l, class_area(20))).collect();
        assert!(close(s.assign(&mut few), 0.3));
        let mut many: Vec<_> = (0..1000).map(|_| request(&l, class_area(20))).collect();
        let ratio = s.assign(&mut many);
        assert!(
            close(ratio, 100_000.0 / (1000.0 * class_area(20))),
            "{ratio}"
        );
        // 3325 * 0.03 = 99.8 faces: below the 220 LOD 2 needs.
        assert!(many.iter().all(|r| r.lod == Some(3)));
    }

    #[test]
    fn objects_at_their_finest_lod_leave_the_budget_to_the_others() {
        let l = lods();
        let mut s = LodSelector::new(ObjectsQuality::VeryLow);
        s.view_distance = 0.0;
        // A huge object (class 35: 1.5^35 pixels) affords LOD 0 at any ratio; it pays 8000
        // faces and its area no longer dilutes the others' share.
        let mut r = vec![request(&l, class_area(35))];
        r.extend((0..20).map(|_| request(&l, class_area(18))));
        let ratio = s.assign(&mut r);
        assert_eq!(r[0].lod, Some(0));
        let expected = ((100_000.0 - 8000.0) / (20.0 * class_area(18))).clamp(0.03, 0.3);
        assert!(close(ratio, expected), "{ratio} vs {expected}");
    }

    #[test]
    fn the_coarsest_lod_is_forced_when_even_it_does_not_fit() {
        let l = lods();
        let s = LodSelector::default();
        // 300 pixels * 0.3 = 90 faces < 100: LOD 3 regardless of the budget.
        let mut r = [request(&l, 300.0)];
        s.assign(&mut r);
        assert_eq!(r[0].lod, Some(3));
    }

    fn house() -> ObjectBounds {
        ObjectBounds {
            size: 6.0,
            radius: 10.0,
            draw_importance: 1.0,
            lod_density: 1.0,
        }
    }

    #[test]
    fn near_objects_get_fine_lods_and_far_ones_coarse_lods() {
        let s = LodSelector::default();
        let view = ViewScale::new(0.75, 1920, 1080);
        let l = lods();
        let mut previous = 0;
        for d in [5.0, 30.0, 80.0, 200.0, 500.0, 1200.0] {
            let area = s.lod_area(house(), surface_distance(d, 10.0, 0.1), view);
            let mut r = [request(&l, area)];
            s.assign(&mut r);
            let lod = r[0].lod.expect("drawn");
            assert!(lod >= previous, "LOD went back to {lod} at {d} m");
            previous = lod;
        }
        assert_eq!(previous, 3);
    }

    #[test]
    fn small_far_objects_are_not_drawn_and_objects_fade_at_the_object_view_distance() {
        let s = LodSelector::default();
        let view = ViewScale::new(0.75, 1920, 1080);
        let pebble = ObjectBounds {
            size: 0.1,
            radius: 0.2,
            ..house()
        };
        assert_ne!(s.draw_test(pebble, 5.0, view), Visibility::Hidden);
        assert_eq!(s.draw_test(pebble, 600.0, view), Visibility::Hidden);
        // A big house is full until (R + 1600) / 1.1025 and gone past (R + 1600) * 1.0025.
        let big = ObjectBounds {
            size: 30.0,
            ..house()
        };
        assert_eq!(s.draw_test(big, 1400.0, view), Visibility::Full);
        assert!(matches!(
            s.draw_test(big, 1550.0, view),
            Visibility::Fade(_)
        ));
        assert_eq!(s.draw_test(big, 1700.0, view), Visibility::Hidden);
    }

    #[test]
    fn draw_importance_scales_the_draw_test_but_not_the_lod() {
        let s = LodSelector::default();
        let view = ViewScale::new(0.75, 1920, 1080);
        let stone = ObjectBounds {
            size: 0.3,
            radius: 0.4,
            ..house()
        };
        let important = ObjectBounds {
            draw_importance: 4.0,
            ..stone
        };
        let d = (10..2000)
            .map(|d| d as f32)
            .find(|&d| s.draw_test(stone, d, view) == Visibility::Hidden)
            .unwrap();
        assert_ne!(s.draw_test(important, d, view), Visibility::Hidden);
        assert_eq!(
            s.lod_area(stone, 20.0, view),
            s.lod_area(important, 20.0, view)
        );
        let dense = ObjectBounds {
            lod_density: 2.0,
            ..stone
        };
        assert!(close(
            s.lod_area(dense, 20.0, view),
            2.0 * s.lod_area(stone, 20.0, view)
        ));
    }

    #[test]
    fn shadows_need_twice_the_screen_size() {
        let s = LodSelector::default();
        let view = ViewScale::new(0.75, 1920, 1080);
        let size = ObjectBounds {
            size: 1.0,
            radius: 1.0,
            ..house()
        };
        let d_draw = (1..20_000)
            .map(|d| d as f32)
            .find(|&d| s.draw_test(size, d, view) == Visibility::Hidden)
            .unwrap();
        let d_shadow = (1..20_000)
            .map(|d| d as f32)
            .find(|&d| !s.casts_shadow(size, d, view))
            .unwrap();
        // Four times the area is about half the distance; the coefficient growing with
        // distance moves it out a little.
        assert!(d_shadow < d_draw * 0.65, "{d_shadow} vs {d_draw}");
    }
}
