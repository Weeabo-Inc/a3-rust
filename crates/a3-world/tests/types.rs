//! Entity types built from the merged config.

use std::sync::Arc;

use a3_config::{ConfigTree, parse_text};
use a3_world::{Error, Scope, SimulationClass, TypeBank, TypeSource};

const CONFIG: &str = r#"
class CfgVehicles {
    class All { scope = 0; side = 3; simulation = ""; };
    class AllVehicles: All {};
    class Car: AllVehicles { simulation = "car"; displayName = "Car"; };
    class Car_F: Car { simulation = "carx"; model = "\A3\Soft_F\Car.p3d"; };
    class C_Offroad_01_F: Car_F { scope = 2; displayName = "Offroad"; maxSpeed = 140; };
    class Weird_F: All { scope = 2; simulation = "teleporter"; };
    class Abstract_F: All { scope = 2; };
};
class CfgAmmo {
    class Default { simulation = ""; simulationStep = 0.05; };
    class BulletCore: Default { simulation = "shotBullet"; };
    class B_65x39_Caseless: BulletCore { simulationStep = 0.02; model = "\A3\bullet.p3d"; };
};
class CfgNonAIVehicles {
    class EmptyDetector { scope = 2; simulation = "detector"; };
};
"#;

fn bank() -> TypeBank {
    let config = parse_text(CONFIG).unwrap();
    TypeBank::new(Arc::new(ConfigTree::from_config(&config)))
}

#[test]
fn a_vehicle_type_reads_inherited_config_values() {
    let mut bank = bank();

    let ty = bank.get("C_Offroad_01_F").unwrap();

    assert_eq!(ty.name(), "C_Offroad_01_F");
    assert_eq!(ty.source(), TypeSource::Vehicles);
    assert_eq!(ty.simulation(), "carx");
    assert_eq!(ty.class(), SimulationClass::CarX);
    assert_eq!(ty.model(), r"\A3\Soft_F\Car.p3d");
    assert_eq!(ty.scope(), Scope::Public);
    assert_eq!(ty.side(), 3);
    assert_eq!(ty.display_name(), "Offroad");
}

#[test]
fn types_are_looked_up_case_insensitively_and_cached() {
    let mut bank = bank();

    let a = bank.get("c_offroad_01_f").unwrap();
    let b = bank.get("C_OFFROAD_01_F").unwrap();

    assert!(Arc::ptr_eq(&a, &b));
    assert_eq!(a.name(), "C_Offroad_01_F");
}

#[test]
fn family_parameters_stay_reachable_through_the_config() {
    let mut bank = bank();
    let ty = bank.get("C_Offroad_01_F").unwrap();

    assert_eq!(ty.config(bank.config()).get("maxSpeed").number(), 140.0);
}

#[test]
fn ammo_and_non_ai_classes_are_types_too() {
    let mut bank = bank();

    let bullet = bank.get("B_65x39_Caseless").unwrap();
    let trigger = bank.get("EmptyDetector").unwrap();

    assert_eq!(bullet.source(), TypeSource::Ammo);
    assert_eq!(bullet.class(), SimulationClass::ShotBullet);
    assert_eq!(bullet.simulation_step(), 0.02);
    assert_eq!(trigger.source(), TypeSource::NonAiVehicles);
    assert_eq!(trigger.class(), SimulationClass::Detector);
}

#[test]
fn entities_default_to_the_original_fifteenth_of_a_second_step() {
    let mut bank = bank();

    assert_eq!(
        bank.get("C_Offroad_01_F").unwrap().simulation_step(),
        1.0 / 15.0
    );
}

#[test]
fn abstract_types_are_reported_by_scope() {
    let mut bank = bank();

    assert_eq!(bank.get("Car").unwrap().scope(), Scope::Private);
}

#[test]
fn unknown_classes_and_simulations_are_errors() {
    let mut bank = bank();

    assert!(matches!(bank.get("Nope"), Err(Error::UnknownType(n)) if n == "Nope"));
    assert!(matches!(
        bank.get("Weird_F"),
        Err(Error::UnknownSimulation { simulation, .. }) if simulation == "teleporter"
    ));
    assert!(matches!(
        bank.get("Abstract_F"),
        Err(Error::UnknownSimulation { simulation, .. }) if simulation.is_empty()
    ));
}
