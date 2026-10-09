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

fn sin_d(x: f64) -> f64 {
    x.to_radians().sin()
}

fn cos_d(x: f64) -> f64 {
    x.to_radians().cos()
}

/// Julian centuries since J2000.0 for a local time.
fn centuries(observer: &Observer, dt: &DateTime) -> (f64, f64) {
    let jd = dt.julian_day(observer.utc_offset_hours);
    (jd, (jd - 2_451_545.0) / 36_525.0)
}

/// Mean obliquity of the ecliptic in degrees.
fn obliquity(t: f64) -> f64 {
    23.0 + (26.0 + (21.448 - t * (46.815 + t * (0.000_59 - t * 0.001_813))) / 60.0) / 60.0
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

/// The sun's position (NOAA solar position algorithm).
pub fn sun_position(observer: &Observer, dt: &DateTime) -> Horizontal {
    let (_, t) = centuries(observer, dt);
    let mean_long = (280.466_46 + t * (36_000.769_83 + t * 0.000_303_2)).rem_euclid(360.0);
    let mean_anom = 357.529_11 + t * (35_999.050_29 - 0.000_153_7 * t);
    let ecc = 0.016_708_634 - t * (0.000_042_037 + 0.000_000_126_7 * t);
    let center = sin_d(mean_anom) * (1.914_602 - t * (0.004_817 + 0.000_014 * t))
        + sin_d(2.0 * mean_anom) * (0.019_993 - 0.000_101 * t)
        + sin_d(3.0 * mean_anom) * 0.000_289;
    let omega = 125.04 - 1934.136 * t;
    let apparent_long = mean_long + center - 0.005_69 - 0.004_78 * sin_d(omega);
    let oblique = obliquity(t) + 0.002_56 * cos_d(omega);
    let declination = (sin_d(oblique) * sin_d(apparent_long)).asin().to_degrees();

    let y = (oblique / 2.0).to_radians().tan().powi(2);
    let l2 = 2.0 * mean_long.to_radians();
    let m = mean_anom.to_radians();
    let eq_time = 4.0
        * (y * l2.sin() - 2.0 * ecc * m.sin() + 4.0 * ecc * y * m.sin() * l2.cos()
            - 0.5 * y * y * (2.0 * l2).sin()
            - 1.25 * ecc * ecc * (2.0 * m).sin())
        .to_degrees();
    let minutes = dt.hours * 60.0;
    let true_solar =
        minutes + eq_time + 4.0 * observer.longitude - 60.0 * observer.utc_offset_hours;
    let hour_angle = true_solar / 4.0 - 180.0;
    horizontal(observer.latitude, declination, hour_angle)
}

/// The moon's position (truncated lunar theory, geocentric, about 0.3° precise).
pub fn moon_position(observer: &Observer, dt: &DateTime) -> Horizontal {
    let (jd, t) = centuries(observer, dt);
    let longitude = 218.32 + 481_267.881 * t + 6.29 * sin_d(134.9 + 477_198.85 * t)
        - 1.27 * sin_d(259.2 - 413_335.38 * t)
        + 0.66 * sin_d(235.7 + 890_534.23 * t)
        + 0.21 * sin_d(269.9 + 954_397.70 * t)
        - 0.19 * sin_d(357.5 + 35_999.05 * t)
        - 0.11 * sin_d(186.6 + 966_404.05 * t);
    let latitude = 5.13 * sin_d(93.3 + 483_202.03 * t) + 0.28 * sin_d(228.2 + 960_400.87 * t)
        - 0.28 * sin_d(318.3 + 6_003.18 * t)
        - 0.17 * sin_d(217.6 - 407_332.20 * t);
    let eps = obliquity(t);
    // Ecliptic to equatorial.
    let (l, b, e) = (
        longitude.to_radians(),
        latitude.to_radians(),
        eps.to_radians(),
    );
    let ra = (l.sin() * e.cos() - b.tan() * e.sin())
        .atan2(l.cos())
        .to_degrees();
    let declination = (b.sin() * e.cos() + b.cos() * e.sin() * l.sin())
        .asin()
        .to_degrees();
    let hour_angle = sidereal_time(jd, t) + observer.longitude - ra;
    let mut h = horizontal(observer.latitude, declination, hour_angle);
    // Topocentric parallax (about 0.95° at the horizon).
    h.elevation -= 0.95 * cos_d(h.elevation);
    h
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
