//! Calendar date and time of day.

use a3_environment::DateTime;

#[test]
fn parses_the_world_config_start_date_and_time() {
    let dt = DateTime::from_config("24/6/2035", "12:00").unwrap();
    assert_eq!((dt.year, dt.month, dt.day), (2035, 6, 24));
    assert_eq!(dt.hours, 12.0);
    let dt = DateTime::from_config("1/1/2000", "05:30").unwrap();
    assert_eq!(dt.hours, 5.5);
    assert!(DateTime::from_config("24-6-2035", "12:00").is_none());
    assert!(DateTime::from_config("31/2/2035", "12:00").is_none());
}

#[test]
fn day_of_year_counts_leap_days() {
    assert_eq!(DateTime::new(2035, 1, 1, 0.0).day_of_year(), 1);
    assert_eq!(DateTime::new(2035, 12, 31, 0.0).day_of_year(), 365);
    assert_eq!(DateTime::new(2036, 12, 31, 0.0).day_of_year(), 366);
    assert_eq!(DateTime::new(2100, 3, 1, 0.0).day_of_year(), 60);
}

#[test]
fn skipping_time_rolls_over_days_months_and_years() {
    let mut dt = DateTime::new(2035, 12, 31, 23.0);
    dt.skip_hours(2.5);
    assert_eq!((dt.year, dt.month, dt.day), (2036, 1, 1));
    assert!((dt.hours - 1.5).abs() < 1e-9);
    dt.skip_hours(-2.0);
    assert_eq!((dt.year, dt.month, dt.day), (2035, 12, 31));
    assert!((dt.hours - 23.5).abs() < 1e-9);
    let mut dt = DateTime::new(2036, 2, 28, 12.0);
    dt.skip_hours(24.0);
    assert_eq!((dt.month, dt.day), (2, 29));
}

#[test]
fn julian_day_of_a_known_instant() {
    // J2000.0 is 2000-01-01 12:00 UT = JD 2451545.0.
    let dt = DateTime::new(2000, 1, 1, 12.0);
    assert!((dt.julian_day(0.0) - 2_451_545.0).abs() < 1e-9);
    // Local time two hours ahead of UT.
    assert!((dt.julian_day(2.0) - (2_451_545.0 - 2.0 / 24.0)).abs() < 1e-9);
}
