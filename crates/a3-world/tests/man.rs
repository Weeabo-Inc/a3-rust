//! The Man family: the ground he walks on, his motion, the moves state machine and the input
//! a player controller fills in.
//!
//! Ground-query and motion tests use a synthetic [`GroundQuery`] (a closure) or a synthetic
//! WRP terrain, so they run everywhere.

use std::sync::Arc;

use a3_world::{GroundContact, GroundQuery, Motion};
use a3_wrp::{Terrain, TerrainBuilder};
use glam::DVec3;

/// A level terrain of `height` metres above sea level: 4 land cells of 50 m, a 8x8 height grid.
fn flat_terrain(height: f32) -> Terrain {
    TerrainBuilder::new(4, 8, 50.0).heights(|_, _| height).build()
}

/// Ground from a closure returning the surface height at `(x, z)`, always level.
struct Level<F>(F);

impl<F: Fn(f64, f64) -> f64> GroundQuery for Level<F> {
    fn ground(
        &self,
        x: f64,
        z: f64,
        _from_y: f64,
    ) -> Option<GroundContact> {
        Some(GroundContact {
            height: (self.0)(x, z),
            normal: DVec3::Y,
        })
    }
}

/// A surface query with nothing to stand on.
struct Void;

impl GroundQuery for Void {
    fn ground(&self, _: f64, _: f64, _: f64) -> Option<GroundContact> {
        None
    }
}

#[test]
fn terrain_is_the_ground_of_a_flat_map() {
    let terrain = flat_terrain(100.0);

    let contact = terrain.ground(75.0, 125.0, 100.0).expect("flat terrain");

    assert_eq!(contact.height, 100.0);
    assert_eq!(contact.normal, DVec3::Y);
}

#[test]
fn a_slope_tilts_the_ground_normal() {
    // Heights rise 1 m per 50 m cell eastwards: a 1:50 gradient.
    let terrain = TerrainBuilder::new(4, 4, 50.0)
        .heights(|i, _| i as f32)
        .build();

    let contact = terrain.ground(60.0, 60.0, 0.0).expect("terrain");

    assert!((contact.height - 1.2).abs() < 1e-6, "{}", contact.height);
    // The surface rises eastwards, so its normal leans west.
    let expected = DVec3::new(-1.0, 50.0, 0.0).normalize();
    assert!(contact.normal.abs_diff_eq(expected, 1e-6), "{:?}", contact.normal);
}

#[test]
fn a_man_standing_still_keeps_his_feet_on_the_ground() {
    let ground = Level(|_, _| 100.0);
    let mut motion = Motion::default();

    let feet = motion.step(DVec3::new(10.0, 100.0, 20.0), DVec3::ZERO, &ground, 1.0 / 15.0);

    assert_eq!(feet, DVec3::new(10.0, 100.0, 20.0));
    assert!(motion.on_ground);
    assert_eq!(motion.vertical_speed, 0.0);
}

#[test]
fn walking_moves_the_feet_and_follows_the_ground() {
    // A 10 % slope upwards to the north (+Z).
    let ground = Level(|_, z| 100.0 + z * 0.1);
    let mut motion = Motion::default();

    let feet = motion.step(
        DVec3::new(0.0, 100.0, 0.0),
        DVec3::new(0.0, 0.0, 1.5),
        &ground,
        1.0,
    );

    assert!((feet.z - 1.5).abs() < 1e-9, "{feet:?}");
    assert!((feet.y - (100.0 + 0.15)).abs() < 1e-9, "{feet:?}");
    assert!(motion.on_ground);
}

#[test]
fn walking_off_a_ledge_starts_a_fall() {
    // The ground stops at z = 0: past it there is a 10 m drop.
    let ground = Level(|_, z| if z > 0.0 { 90.0 } else { 100.0 });
    let mut motion = Motion::default();
    let feet = DVec3::new(0.0, 100.0, 0.0);

    let feet = motion.step(feet, DVec3::new(0.0, 0.0, 1.0), &ground, 1.0);

    assert!(!motion.on_ground, "a drop taller than a step starts a fall");
    assert_eq!(feet.y, 100.0, "he keeps his height while the fall starts");
    assert!(feet.z > 0.0, "he walked past the edge: {feet:?}");
}

#[test]
fn a_falling_man_accelerates_with_gravity() {
    let mut motion = Motion {
        on_ground: false,
        vertical_speed: 0.0,
    };

    let feet = motion.step(DVec3::new(0.0, 100.0, 0.0), DVec3::ZERO, &Void, 1.0);

    assert!(motion.vertical_speed < 0.0);
    assert!(feet.y < 100.0);
    assert!(!motion.on_ground);
}

#[test]
fn a_falling_man_lands_on_the_ground() {
    let ground = Level(|_, _| 100.0);
    let mut motion = Motion {
        on_ground: false,
        vertical_speed: -2.0,
    };

    // 2 m above the ground, falling at 2 m/s: the step of 1 s takes him through the surface.
    let feet = motion.step(DVec3::new(0.0, 102.0, 0.0), DVec3::ZERO, &ground, 1.0);

    assert!(motion.on_ground);
    assert_eq!(motion.vertical_speed, 0.0);
    assert_eq!(feet.y, 100.0);
}

#[test]
fn a_step_within_reach_is_climbed() {
    // A 0.3 m kerb at z = 1.
    let ground = Level(|_, z| if z >= 1.0 { 100.3 } else { 100.0 });
    let mut motion = Motion::default();

    let feet = motion.step(
        DVec3::new(0.0, 100.0, 0.5),
        DVec3::new(0.0, 0.0, 1.0),
        &ground,
        1.0,
    );

    assert!(motion.on_ground);
    assert_eq!(feet.z, 1.5);
    assert_eq!(feet.y, 100.3);
}

#[test]
fn a_wall_too_high_to_step_onto_stops_him() {
    // A 3 m wall at z = 1.
    let ground = Level(|_, z| if z >= 1.0 { 103.0 } else { 100.0 });
    let mut motion = Motion::default();

    let feet = motion.step(
        DVec3::new(0.0, 100.0, 0.5),
        DVec3::new(0.0, 0.0, 1.0),
        &ground,
        1.0,
    );

    assert!(motion.on_ground);
    assert_eq!(feet, DVec3::new(0.0, 100.0, 0.5), "he stays at the wall");
}

#[test]
fn a_man_who_has_no_ground_under_him_falls() {
    let mut motion = Motion::default();

    // The step that loses the ground only enters the fall; gravity acts from the next one.
    let feet = motion.step(DVec3::new(0.0, 100.0, 0.0), DVec3::ZERO, &Void, 0.1);
    assert!(!motion.on_ground);
    assert_eq!(feet.y, 100.0);

    let feet = motion.step(feet, DVec3::ZERO, &Void, 0.1);
    assert!(motion.vertical_speed < 0.0);
    assert!(feet.y < 100.0);
}

/// The terrain under a level map is what a Man stands on through [`a3_wrp::Terrain`]'s own
/// [`GroundQuery`] implementation, not only through his own copy.
#[test]
fn a_terrain_can_be_asked_as_the_ground_of_a_man() {
    let terrain: Arc<Terrain> = Arc::new(flat_terrain(42.0));
    let ground: &dyn GroundQuery = terrain.as_ref();
    let mut motion = Motion::default();

    let feet = motion.step(DVec3::new(10.0, 42.0, 10.0), DVec3::ZERO, ground, 0.1);

    assert_eq!(feet.y, 42.0);
    assert!(motion.on_ground);
}
