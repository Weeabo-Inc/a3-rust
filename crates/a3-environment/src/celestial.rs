//! Sun and moon positions.
//!
//! Arma 3 hands its sky to trueSKY, passing the world's `latitude` negated (the config's
//! `latitude = -35.152` means 35.152° north), its `longitude`, and a time zone of
//! `longitude / 15` hours; the engine reads the sun and moon directions back from it
//! (`docs/re/environment.md`). trueSKY uses real ephemerides, so this module does too: the
//! NOAA solar position algorithm and a truncated lunar theory (Meeus, *Astronomical
//! Algorithms*, ch. 47 low-precision terms), both good to a fraction of a degree.

use glam::Vec3;

use crate::DateTime;

/// Where on Earth the World lies and which time zone its clock runs in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Observer {
    /// Geographic latitude in degrees, north positive.
    pub latitude: f64,
    /// Geographic longitude in degrees, east positive.
    pub longitude: f64,
    /// Hours the local clock is ahead of UT.
    pub utc_offset_hours: f64,
}

impl Observer {
    /// The observer for a world config's `latitude` and `longitude` entries, as the engine
    /// passes them to the sky: latitude negated, time zone `longitude / 15` hours (so the
    /// clock is local mean solar time).
    pub fn from_world_config(latitude: f32, longitude: f32) -> Self {
        let longitude = f64::from(longitude);
        Observer {
            latitude: -f64::from(latitude),
            longitude,
            utc_offset_hours: longitude / 15.0,
        }
    }
}

/// A direction in the observer's sky.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Horizontal {
    /// Degrees clockwise from north (90 = east).
    pub azimuth: f64,
    /// Degrees above the horizon (negative below), without refraction.
    pub elevation: f64,
}

impl Horizontal {
    /// Unit vector in world axes: x east, y up, z north.
    pub fn direction(&self) -> Vec3 {
        let (az, el) = (self.azimuth.to_radians(), self.elevation.to_radians());
        Vec3::new(
            (el.cos() * az.sin()) as f32,
            el.sin() as f32,
            (el.cos() * az.cos()) as f32,
        )
    }
}

/// Julian centuries since J2000.0 for a local time.
fn centuries(observer: &Observer, dt: &DateTime) -> (f64, f64) {
    let jd = dt.julian_day(observer.utc_offset_hours);
    (jd, (jd - 2_451_545.0) / 36_525.0)
}

/// Altitude and azimuth from declination and hour angle (degrees).
fn horizontal(latitude: f64, declination: f64, hour_angle: f64) -> Horizontal {
    let (lat, dec, ha) = (
        latitude.to_radians(),
        declination.to_radians(),
        hour_angle.to_radians(),
    );
    let sin_el = lat.sin() * dec.sin() + lat.cos() * dec.cos() * ha.cos();
    let el = sin_el.clamp(-1.0, 1.0).asin();
    // Azimuth from north, clockwise.
    let y = -ha.sin() * dec.cos();
    let x = dec.sin() * lat.cos() - dec.cos() * lat.sin() * ha.cos();
    let az = y.atan2(x).to_degrees().rem_euclid(360.0);
    Horizontal {
        azimuth: az,
        elevation: el.to_degrees(),
    }
}

/// Greenwich mean sidereal time in degrees.
fn sidereal_time(jd: f64, t: f64) -> f64 {
    (280.460_618_37 + 360.985_647_366_29 * (jd - 2_451_545.0) + 0.000_387_933 * t * t)
        .rem_euclid(360.0)
}

// --------------------------------------------------------------- the engine's own model
//
// Arma builds the sun and moon directions itself in `FUN_1410813c0`: a 23° axial tilt, a yearly
// and a daily rotation, the observer's latitude, and for the moon a 5° orbit tilt on an angle
// that runs 81.90581 radians per year from 1985.0082. `FUN_1410851a0` (the light the renderer
// shades with) and the `moonPhase` command both call it, so it is the engine's sky, not the
// real ephemeris (`docs/re/environment.md`). It is coarse, but it is what the game shows: the
// oracle's own client logs `moonPhase` 0.704879 for 2035-06-06 23:30, where the real Moon is
// new, and that night's Moon is 43° up in this model and so lights the ground.

/// `FUN_14035c410`: a rotation about X by `-a`.
fn rot_x_neg(a: f64) -> [f64; 9] {
    let (s, c) = a.sin_cos();
    [1.0, 0.0, 0.0, 0.0, c, s, 0.0, -s, c]
}

/// `FUN_14035c560`: a rotation about Y by `a`.
fn rot_y(a: f64) -> [f64; 9] {
    let (s, c) = a.sin_cos();
    [c, 0.0, s, 0.0, 1.0, 0.0, -s, 0.0, c]
}

/// `FUN_14035c6b0`: a rotation about Z by `a`.
fn rot_z(a: f64) -> [f64; 9] {
    let (s, c) = a.sin_cos();
    [c, -s, 0.0, s, c, 0.0, 0.0, 0.0, 1.0]
}

/// Row-major 3x3 product.
fn mat_mul(a: &[f64; 9], b: &[f64; 9]) -> [f64; 9] {
    let mut out = [0.0; 9];
    for r in 0..3 {
        for c in 0..3 {
            out[r * 3 + c] = (0..3).map(|k| a[r * 3 + k] * b[k * 3 + c]).sum();
        }
    }
    out
}

/// `FUN_14035bc40`: the transpose.
fn mat_transpose(a: &[f64; 9]) -> [f64; 9] {
    [a[0], a[3], a[6], a[1], a[4], a[7], a[2], a[5], a[8]]
}

/// `FUN_141088ab0`: the length of the 0-based month `index`, February taking the leap day.
fn days_in_month(year: i32, index: usize) -> i32 {
    const DAYS: [i32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let leap = index == 1 && year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    DAYS[index.min(11)] + i32::from(leap)
}

/// The engine's sun and moon directions in world axes (x east, y up, z north).
///
/// `latitude` is in degrees north-positive (the config's `latitude = -35.152` means 35.152° N),
/// and the clock is the world's local time, as the engine's own builder takes it. The two
/// vectors are the engine's model vectors negated, which is how `FUN_1410851a0` maps them onto
/// our axes: it leaves the model's `(east, north, up)` frame with x, y and z all flipped.
pub fn legacy_directions(observer: &Observer, dt: &DateTime) -> (Vec3, Vec3) {
    const TILT: f64 = 0.401_42; // 23.0 degrees
    const ORBIT_TILT: f64 = 0.087_266_5; // 5.0 degrees
    const MOON_RATE: f64 = 81.905_81; // radians per year
    const MOON_EPOCH: f64 = 1985.0082;
    const DAYS_PER_YEAR: f64 = 0.002_739_726; // 1 / 365

    // Day of year as the builder's own day counter: the day of the month minus one plus the
    // months before it.
    let mut day = dt.day as i32 - 1;
    for i in 0..(dt.month.max(1) as usize - 1) {
        day += days_in_month(dt.year, i);
    }
    let fraction = (f64::from(day) + dt.hours / 24.0) * DAYS_PER_YEAR;

    let daily = rot_y(dt.hours / 24.0 * std::f64::consts::TAU);
    let yearly = rot_y((fraction - 0.030_136_986) * std::f64::consts::TAU);
    let tilt = rot_x_neg(TILT);
    let half_pi = rot_x_neg(-std::f64::consts::FRAC_PI_2);
    // The builder is handed the config latitude in radians, which is the negated one.
    let latitude = rot_x_neg(-observer.latitude.to_radians());

    let frame = mat_mul(&yearly, &mat_mul(&daily, &tilt));
    let axes = mat_mul(&mat_transpose(&mat_mul(&latitude, &frame)), &half_pi);

    let orbit = rot_z(ORBIT_TILT);
    let moon_turn =
        rot_y((f64::from(dt.year) + fraction - MOON_EPOCH) * MOON_RATE + std::f64::consts::PI);
    // The moon's row of its own rotation, carried through the orbit tilt.
    let moon = [moon_turn[6], moon_turn[7], moon_turn[8]];
    let moon_ray = [
        moon[0] * orbit[0] + moon[1] * orbit[3] + moon[2] * orbit[6],
        moon[0] * orbit[1] + moon[1] * orbit[4] + moon[2] * orbit[7],
        moon[0] * orbit[2] + moon[1] * orbit[5] + moon[2] * orbit[8],
    ];
    // The sun is the same row of the yearly rotation.
    let sun_ray = [yearly[6], yearly[7], yearly[8]];

    // The builder writes the axes' columns into the model vector with x and z negated, and the
    // engine's (east, north, up) frame is the model's, negated as a whole.
    let through = |v: [f64; 3]| {
        let dot = |column: usize| (0..3).map(|k| axes[k * 3 + column] * v[k]).sum::<f64>();
        [-dot(0), dot(1), -dot(2)]
    };
    let to_world = |v: [f64; 3]| -Vec3::new(v[0] as f32, v[1] as f32, v[2] as f32);
    (to_world(through(sun_ray)), to_world(through(moon_ray)))
}

/// A direction as an azimuth and an elevation.
fn horizontal_of(direction: Vec3) -> Horizontal {
    let d = direction.normalize_or_zero();
    Horizontal {
        azimuth: f64::from(d.x)
            .atan2(f64::from(d.z))
            .to_degrees()
            .rem_euclid(360.0),
        elevation: f64::from(d.y).clamp(-1.0, 1.0).asin().to_degrees(),
    }
}

/// The sun's position, from the engine's own model.
pub fn sun_position(observer: &Observer, dt: &DateTime) -> Horizontal {
    horizontal_of(legacy_directions(observer, dt).0)
}

/// The moon's position, from the engine's own model.
pub fn moon_position(observer: &Observer, dt: &DateTime) -> Horizontal {
    horizontal_of(legacy_directions(observer, dt).1)
}

/// The rotation from equatorial coordinates (x towards right ascension 0h, y towards 6h, z
/// the north celestial pole) to world axes (x east, y up, z north) at this time: for drawing
/// the stars.
pub fn star_rotation(observer: &Observer, dt: &DateTime) -> glam::Mat3 {
    let (jd, t) = centuries(observer, dt);
    let lst = sidereal_time(jd, t) + observer.longitude;
    let column = |ra: f64, dec: f64| horizontal(observer.latitude, dec, lst - ra).direction();
    glam::Mat3::from_cols(column(0.0, 0.0), column(90.0, 0.0), column(0.0, 90.0))
}

/// The engine's `moonPhase`: the sun-moon elongation over 180°, 0 at new moon and 1 at full
/// moon. `sun` and `moon` are unit directions towards them.
pub fn moon_phase(sun: Vec3, moon: Vec3) -> f32 {
    sun.dot(moon).clamp(-1.0, 1.0).acos() / std::f32::consts::PI
}

/// The illuminated fraction of the moon's disc for a phase from [`moon_phase`].
pub fn moon_illumination(phase: f32) -> f32 {
    (1.0 - (phase * std::f32::consts::PI).cos()) * 0.5
}

#[cfg(test)]
mod tests {
    use super::*;

    fn altis() -> Observer {
        Observer::from_world_config(-35.152, 16.661)
    }

    /// The `moonPhase` the oracle's own client logged for `date` (its scenario prints it). The
    /// SQF command and the renderer's light both come from the same builder, and the command is
    /// handed a time of day of zero, so the hour does not enter these two.
    #[test]
    fn moon_phase_matches_the_client() {
        let observer = altis();
        for (year, month, day, want) in [(2035, 6, 6, 0.704_879_f32), (2035, 6, 24, 0.107_916_f32)]
        {
            let dt = DateTime::new(year, month, day, 0.0);
            let (sun, moon) = legacy_directions(&observer, &dt);
            let phase = moon_phase(sun, moon);
            assert!(
                (phase - want).abs() < 0.001,
                "{year}-{month}-{day}: {phase} against the client's {want}"
            );
        }
    }

    /// The night shot of the render oracle: the engine's moon is up and well lit, so the ground
    /// is moonlit. The real Moon that night was new, which is why our own ephemeris left the
    /// scene dark (#293).
    #[test]
    fn the_night_shot_has_a_moon_above_the_horizon() {
        let observer = altis();
        let dt = DateTime::new(2035, 6, 6, 23.5);
        let sun = sun_position(&observer, &dt);
        let moon = moon_position(&observer, &dt);
        assert!(sun.elevation < -30.0, "the sun is down: {sun:?}");
        assert!(moon.elevation > 30.0, "the moon is up: {moon:?}");
        let phase = moon_phase(sun.direction(), moon.direction());
        assert!(phase > 0.6, "the moon is well lit: {phase}");
    }

    /// The engine's sun still tracks the real one closely enough that the day shots keep their
    /// shadows: the oracle's client renders the noon sun 78.2° over Altis, at 177°.
    #[test]
    fn the_sun_tracks_the_real_one() {
        let observer = altis();
        let noon = sun_position(&observer, &DateTime::new(2035, 6, 24, 12.0));
        assert!(
            (noon.elevation - 78.2).abs() < 3.0,
            "noon elevation {}",
            noon.elevation
        );
        assert!(
            (noon.azimuth - 177.2f64).abs() < 12.0,
            "noon azimuth {}",
            noon.azimuth
        );
        // East in the morning, west in the evening, and down at midnight.
        let morning = sun_position(&observer, &DateTime::new(2035, 6, 24, 8.0));
        let evening = sun_position(&observer, &DateTime::new(2035, 6, 24, 17.0));
        assert!(morning.elevation > 20.0 && evening.elevation > 20.0);
        assert!(morning.azimuth < 180.0, "{}", morning.azimuth);
        assert!(evening.azimuth > 180.0, "{}", evening.azimuth);
        let midnight = sun_position(&observer, &DateTime::new(2035, 6, 24, 0.0));
        assert!(midnight.elevation < -30.0, "{}", midnight.elevation);
    }

    #[test]
    fn directions_are_unit_and_the_calendar_has_leap_days() {
        let observer = altis();
        let (sun, moon) = legacy_directions(&observer, &DateTime::new(2035, 6, 24, 12.0));
        assert!((sun.length() - 1.0).abs() < 1e-5, "{sun:?}");
        assert!((moon.length() - 1.0).abs() < 1e-5, "{moon:?}");
        assert_eq!(days_in_month(2036, 1), 29);
        assert_eq!(days_in_month(2035, 1), 28);
        assert_eq!(days_in_month(2035, 11), 31);
    }
}
