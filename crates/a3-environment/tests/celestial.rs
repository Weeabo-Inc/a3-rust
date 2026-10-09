//! Sun and moon positions. Reference values come from published almanac data (solstice and
//! equinox geometry, lunar phase dates), not from this implementation.

use a3_environment::{DateTime, Observer, moon_phase, moon_position, sun_position};

fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

/// Altis as the engine passes it to the sky: config latitude -35.152 means 35.152 N.
fn altis() -> Observer {
    Observer::from_world_config(-35.152, 16.661)
}

#[test]
fn the_world_config_latitude_is_negated_and_the_zone_follows_the_longitude() {
    let o = altis();
    assert!(close(o.latitude, 35.152, 1e-6));
    assert!(close(o.longitude, 16.661, 1e-6));
    assert!(close(o.utc_offset_hours, 16.661 / 15.0, 1e-6));
}

#[test]
fn midsummer_noon_on_altis_is_high_in_the_south() {
    // Sun declination on 24 June is about +23.4 degrees: elevation 90 - 35.15 + 23.4.
    let sun = sun_position(&altis(), &DateTime::new(2035, 6, 24, 12.0));
    assert!(close(sun.elevation, 78.25, 0.5), "{sun:?}");
    assert!(close(sun.azimuth, 180.0, 5.0), "{sun:?}");
}

#[test]
fn equinox_sun_at_the_equator_rises_in_the_east_and_culminates_overhead() {
    let equator = Observer {
        latitude: 0.0,
        longitude: 0.0,
        utc_offset_hours: 0.0,
    };
    // 2035 March equinox is on 20 March; the equation of time is about -7.5 minutes.
    let noon = sun_position(&equator, &DateTime::new(2035, 3, 20, 12.125));
    assert!(noon.elevation > 89.0, "{noon:?}");
    let morning = sun_position(&equator, &DateTime::new(2035, 3, 20, 6.125));
    assert!(close(morning.elevation, 0.0, 1.0), "{morning:?}");
    assert!(close(morning.azimuth, 90.0, 1.0), "{morning:?}");
}

#[test]
fn midsummer_day_on_altis_lasts_about_fourteen_and_a_half_hours() {
    let o = altis();
    let up = |h: f64| sun_position(&o, &DateTime::new(2035, 6, 21, h)).elevation > -0.833;
    let mut sunrise = 0.0;
    let mut sunset = 0.0;
    let mut h = 0.0;
    while h < 24.0 {
        if !up(h) && up(h + 0.01) {
            sunrise = h;
        }
        if up(h) && !up(h + 0.01) {
            sunset = h;
        }
        h += 0.01;
    }
    assert!(close(sunset - sunrise, 14.5, 0.25), "{sunrise} {sunset}");
}

#[test]
fn direction_uses_world_axes_x_east_y_up_z_north() {
    let o = altis();
    let noon = sun_position(&o, &DateTime::new(2035, 6, 24, 12.0)).direction();
    assert!(noon.y > 0.95 && noon.z < 0.0, "{noon:?}");
    let evening = sun_position(&o, &DateTime::new(2035, 6, 24, 19.0)).direction();
    assert!(
        evening.x < -0.5,
        "the summer sun sets in the west: {evening:?}"
    );
    assert!((noon.length() - 1.0).abs() < 1e-5);
}

#[test]
fn moon_phase_follows_published_full_and_new_moons() {
    let utc = Observer {
        latitude: 35.0,
        longitude: 0.0,
        utc_offset_hours: 0.0,
    };
    // Full moon 25 January 2024 17:54 UT, new moon 11 January 2024 11:57 UT.
    let full = DateTime::new(2024, 1, 25, 17.9);
    let new = DateTime::new(2024, 1, 11, 11.95);
    let phase = |dt: &DateTime| {
        moon_phase(
            sun_position(&utc, dt).direction(),
            moon_position(&utc, dt).direction(),
        )
    };
    // The moon may pass up to 5 degrees off the ecliptic, so new and full are not exact.
    assert!(phase(&full) > 0.95, "{}", phase(&full));
    assert!(phase(&new) < 0.05, "{}", phase(&new));
    let first_quarter = DateTime::new(2024, 1, 18, 3.9);
    assert!(close(f64::from(phase(&first_quarter)), 0.5, 0.05));
}

#[test]
fn the_moon_stays_within_its_declination_band() {
    // Elevation at upper culmination can't exceed 90 - |lat| + 28.6 degrees.
    let o = altis();
    let mut highest: f64 = -90.0;
    let mut dt = DateTime::new(2035, 1, 1, 0.0);
    for _ in 0..(24 * 30) {
        highest = highest.max(moon_position(&o, &dt).elevation);
        dt.skip_hours(1.0);
    }
    assert!(
        highest < 90.0 - 35.152 + 29.0 && highest > 60.0,
        "{highest}"
    );
}

#[test]
fn the_star_sphere_turns_about_the_celestial_pole() {
    use a3_environment::star_rotation;
    use glam::Vec3;
    let o = altis();
    let lat = 35.152f32.to_radians();
    let pole = Vec3::new(0.0, lat.sin(), lat.cos());
    for hour in [0.0, 6.0, 13.5] {
        let r = star_rotation(&o, &DateTime::new(2035, 6, 24, hour));
        // Equatorial z is the north celestial pole.
        assert!((r * Vec3::Z - pole).length() < 1e-3, "{hour}");
        // A rotation: orthonormal columns.
        assert!((r.x_axis.length() - 1.0).abs() < 1e-4);
        assert!(r.x_axis.dot(r.y_axis).abs() < 1e-4);
    }
    // The celestial equator crosses the meridian due south at elevation 90 - latitude.
    let r = star_rotation(&o, &DateTime::new(2035, 6, 24, 0.0));
    let highest = (0..360)
        .map(|deg| {
            let a = (deg as f32).to_radians();
            r * Vec3::new(a.cos(), a.sin(), 0.0)
        })
        .max_by(|a, b| a.y.total_cmp(&b.y))
        .unwrap();
    assert!((highest.y.asin().to_degrees() - (90.0 - 35.152)).abs() < 1.0);
    assert!(highest.z < 0.0 && highest.x.abs() < 0.05, "{highest:?}");
}
