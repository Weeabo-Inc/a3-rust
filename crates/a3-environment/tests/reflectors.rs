//! The shipped lamps: `CfgVehicles >> Reflectors` and the memory points of their p3d models.
//! Skipped when `A3_ROOT` is unset.

use std::path::Path;

use a3_environment::{lamp_classes, normalise_path, placed_lights};
use a3_gamedata::{GameData, LoadOptions};
use glam::{DAffine3, Vec3};

#[test]
fn the_shipped_lamps_decode_and_their_memory_points_resolve() {
    let Some(root) = std::env::var_os("A3_ROOT") else {
        eprintln!("skipping: A3_ROOT not set");
        return;
    };
    let data = GameData::load(&LoadOptions::new(Path::new(&root))).unwrap();
    let classes = lamp_classes(&data.config.root());
    eprintln!("{} CfgVehicles classes carry Reflectors", classes.len());
    assert!(classes.len() > 100, "{} lamp classes", classes.len());

    // The street lamp the render oracle's Kavala shots are full of.
    let street = classes
        .iter()
        .find(|c| c.class == "Land_LampStreet_F")
        .expect("Land_LampStreet_F");
    assert_eq!(street.model, "a3/structures_f/civ/lamps/lampstreet_f.p3d");
    assert_eq!(street.reflectors.len(), 1);
    let light = &street.reflectors[0];
    assert_eq!(light.colour, Vec3::new(1200.0, 600.0, 300.0));
    assert_eq!(light.ambient, Vec3::new(12.0, 6.0, 3.0));
    assert_eq!(light.intensity, 7.0);
    assert!(!light.is_spot(), "outerAngle 180 is a point light");
    assert_eq!(light.position, "Light_1_pos");
    assert_eq!(light.direction, "Light_1_dir");
    assert_eq!(light.selection, "Light_1_hide");
    assert_eq!(light.attenuation.quadratic, 0.3);
    assert_eq!(light.attenuation.hard_limit_end, 60.0);
    assert!((light.attenuation.fade_scale() - 0.05).abs() < 1e-6);

    // One more class, a cone this time, so both shapes are covered.
    let harbour = classes
        .iter()
        .find(|c| c.class == "Land_LampHarbour_F")
        .expect("Land_LampHarbour_F");
    eprintln!(
        "Land_LampHarbour_F: model {}, {:?}, outerAngle {}",
        harbour.model, harbour.reflectors[0].colour, harbour.reflectors[0].outer_angle
    );
    assert!(harbour.reflectors[0].is_spot());

    // The memory points of one shipped lamp model: the head is metres above the origin.
    let path = normalise_path("A3\\Structures_F\\Civ\\Lamps\\LampStreet_F.p3d");
    let bytes = data.vfs.open(&path).expect("lamp model");
    let model = a3_p3d::Model::from_bytes(&bytes).expect("parse lamp model");
    let position = model
        .memory_point("Light_1_pos")
        .unwrap_or_else(|| panic!("{path} has no Light_1_pos"));
    // The direction is a point next to the position, not a vector (see placed_lights).
    let direction = model.memory_point("Light_1_dir").expect("Light_1_dir");
    let down = (direction - position).normalize();
    eprintln!("LampStreet_F: Light_1_pos {position:?}, Light_1_dir {direction:?}, down {down:?}");
    assert!(position.y > 3.0 && position.y < 12.0, "{position:?}");
    assert!(
        position.x.abs() < 2.0 && position.z.abs() < 2.0,
        "the head hangs off the pole, not off the map: {position:?}"
    );
    assert!(down.y < -0.8, "the head points down: {down:?}");

    // And the whole chain on a synthetic placement: one object, one light in the world.
    let models = vec!["a3\\structures_f\\civ\\lamps\\lampstreet_f.p3d".to_owned()];
    let placed = vec![(
        0u32,
        DAffine3::from_translation(glam::DVec3::new(3000.0, 40.0, 12000.0)),
    )];
    let lights = placed_lights(&classes, &models, &placed, |model, point| {
        a3_p3d::Model::from_bytes(&data.vfs.open(model).ok()?)
            .ok()?
            .memory_point(point)
    });
    assert_eq!(lights.len(), 1, "{lights:#?}");
    eprintln!("placed at {:?}", lights[0].position);
    assert!((lights[0].position.x - 3000.0).abs() < 2.0);
    assert!(lights[0].position.y > 40.0, "the head is above the ground");
    assert!((lights[0].position.z - 12000.0).abs() < 2.0);
}
