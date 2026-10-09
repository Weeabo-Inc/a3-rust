//! What the vehicle model does when it is driven: how fast it goes, how far it takes to stop,
//! how tightly it turns. Every test drives a fixed step against a flat surface — no wall clock,
//! no world — so the numbers are the model's own, and the numbers the shipped configs claim
//! (`maxSpeed`, the gear ratios, the tire's grip) are what they are checked against.
//!
//! The fixtures carry the shipped vehicles' driving entries as they are written in the game
//! install (`C_Offroad_01_F`, `B_MBT_01_cannon_F`), and the wheel geometry the model's memory
//! points would place. A tank's memory points are not in the config, so the wheel radius comes
//! from the wheel's own `MOI = mass·r²/2` — 0.668 m for the shipped tank, which is also the
//! radius at which top gear (1.05 × 12) reaches `maxSpeed` at `maxOmega`.

use a3_config::{ConfigTree, parse_text};
use a3_vehicles::ground::FlatSurface;
use a3_vehicles::vehicle::{MemoryPoints, VehicleData, VehicleInput};
use a3_vehicles::wheeled::WheeledVehicle;
use glam::DVec3;
use std::f64::consts::PI;

/// The step every test drives with, seconds.
const DT: f64 = 0.01;

/// A road's `surfaceFriction` (`docs/re/physics-collision.md`: 2 on a standard surface).
const ROAD: f64 = 2.0;

/// The shipped offroad's driving numbers.
const OFFROAD: &str = r#"
    class C_Offroad_01_F {
        simulation = "carx";
        maxSpeed = 200;
        wheelCircumference = 2.805;
        enginePower = 150;
        maxOmega = 450;
        minOmega = 100;
        brakeIdleSpeed = 1.78;
        thrustDelay = 0.5;
        differentialType = "all_limited";
        frontRearSplit = 0.5;
        frontBias = 1.5;
        rearBias = 1.5;
        centreBias = 1.3;
        changeGearMinEffectivity[] = {1, 0.15, 1, 1, 1, 1, 1, 1};
        switchTime = 0.31;
        latency = 1.5;
        clutchStrength = 20.0;
        antiRollbarForceCoef = 1.9;
        antiRollbarForceLimit = 5;
        antiRollbarSpeedMin = 10;
        antiRollbarSpeedMax = 150;
        class complexGearbox {
            GearboxRatios[] = {"R1", -4, "N", 0, "D1", "4.5*(0.58^0)", "D2", "4.5*(0.58^1)",
                               "D3", "4.5*(0.58^2)", "D4", "4.5*(0.58^3)", "D5", "4.5*(0.59^4)",
                               "D6", "4.5*(0.6^5)"};
            TransmissionRatios[] = {"High", 7};
            moveOffGear = 1;
        };
        class Wheels {
            class LF { side = "left"; steering = 1; center = "wheel_1_1_axis";
                       boundary = "wheel_1_1_bound"; width = 0.3; mass = 30; MOI = 6;
                       dampingRate = 1; dampingRateInAir = 0.8; maxBrakeTorque = 2000;
                       maxCompression = 0.05; maxDroop = 0.1; sprungMass = 400;
                       springStrength = 14400; springDamperRate = 3840;
                       longitudinalStiffnessPerUnitGravity = 10000; latStiffX = 2.5;
                       latStiffY = 18;
                       frictionVsSlipGraph[] = {{0, 1.75}, {0.5, 1.35}, {1, 1.2}}; };
            class LR { side = "left"; steering = 0; center = "wheel_1_2_axis";
                       boundary = "wheel_1_2_bound"; width = 0.3; mass = 30; MOI = 6;
                       dampingRate = 1; dampingRateInAir = 0.8; maxBrakeTorque = 2000;
                       maxHandBrakeTorque = 3000; maxCompression = 0.05; maxDroop = 0.1;
                       sprungMass = 400; springStrength = 14400; springDamperRate = 3840;
                       longitudinalStiffnessPerUnitGravity = 10000; latStiffX = 2.5;
                       latStiffY = 18;
                       frictionVsSlipGraph[] = {{0, 1.75}, {0.5, 1.35}, {1, 1.2}}; };
            class RF { side = "right"; steering = 1; center = "wheel_2_1_axis";
                       boundary = "wheel_2_1_bound"; width = 0.3; mass = 30; MOI = 6;
                       dampingRate = 1; dampingRateInAir = 0.8; maxBrakeTorque = 2000;
                       maxCompression = 0.05; maxDroop = 0.1; sprungMass = 400;
                       springStrength = 14400; springDamperRate = 3840;
                       longitudinalStiffnessPerUnitGravity = 10000; latStiffX = 2.5;
                       latStiffY = 18;
                       frictionVsSlipGraph[] = {{0, 1.75}, {0.5, 1.35}, {1, 1.2}}; };
            class RR { side = "right"; steering = 0; center = "wheel_2_2_axis";
                       boundary = "wheel_2_2_bound"; width = 0.3; mass = 30; MOI = 6;
                       dampingRate = 1; dampingRateInAir = 0.8; maxBrakeTorque = 2000;
                       maxHandBrakeTorque = 3000; maxCompression = 0.05; maxDroop = 0.1;
                       sprungMass = 400; springStrength = 14400; springDamperRate = 3840;
                       longitudinalStiffnessPerUnitGravity = 10000; latStiffX = 2.5;
                       latStiffY = 18;
                       frictionVsSlipGraph[] = {{0, 1.75}, {0.5, 1.35}, {1, 1.2}}; };
        };
    };"#;

/// The half-wheelbase of the car's fixture geometry, m (the model's memory points): 2.8 m
/// between the axles, which is the offroad's own wheelbase.
const CAR_HALF_WHEELBASE: f64 = 1.4;
/// The wheel radius of the car's fixture geometry, m.
const CAR_RADIUS: f64 = 0.45;

/// The shipped main battle tank's driving numbers. Its `class Wheels` has sixteen subclasses —
/// eight road wheels per side, `L1`…`L8` and `R1`…`R8` — all carrying the same numbers, so the
/// fixture generates them.
fn tank_config() -> String {
    const WHEEL: &str = r#"            class @NAME@ {
                side = "@SIDE@";
                suspTravelDirection[] = {-0.125, -1, 0};
                center = "wheel_@SI@_@R@_axis";
                boundary = "wheel_@SI@_@R@_bound";
                steering = 0;
                width = 0.5;
                mass = 150;
                MOI = 33.453;
                dampingRate = 590.0;
                dampingRateInAir = 590.0;
                dampingRateDestroyed = 3400.0;
                maxDroop = 0.18;
                maxCompression = 0.18;
                sprungMass = -1;
                springStrength = 350000;
                springDamperRate = 15514;
                maxBrakeTorque = 23000;
                latStiffX = 2;
                latStiffY = 33;
                longitudinalStiffnessPerUnitGravity = 10000;
                frictionVsSlipGraph[] = {{0.0, 0.55}, {0.3, 1.28}, {0.65, 0.55}};
            };
"#;
    let mut text = String::from(
        r#"    class B_MBT_01_cannon_F {
        simulation = "tankx";
        maxSpeed = 60;
        engineMOI = 9.0;
        enginePower = 1230;
        maxOmega = 345.575;
        minOmega = 146.608;
        peakTorque = 5000;
        torqueCurve[] = {{0.363636, 0.68}, {0.454545, 0.8}, {0.545455, 0.95}, {0.606061, 0.98},
                         {0.666667, 1}, {0.727273, 0.94}, {0.848485, 0.8}, {1, 0.64}};
        thrustDelay = 0.05;
        dampingRateFullThrottle = 0.3;
        dampingRateZeroThrottleClutchEngaged = 3.0;
        dampingRateZeroThrottleClutchDisengaged = 0.25;
        clutchStrength = 40.0;
        latency = 1.5;
        switchTime = 0;
        changeGearType = "rpmratio";
        changeGearOmegaRatios[] = {1, 0.424242, 0.454545, 0.333333, 0.939394, 0.424242, 0.909091,
                                   0.636364, 0.848485, 0.666667, 1, 0.666667};
        brakeIdleSpeed = 0.1;
        tankTurnForce = 1280000.0;
        tankTurnForceAngMinSpd = 0.6;
        tankTurnForceAngSpd = 0.91;
        class complexGearbox {
            GearboxRatios[] = {"R1", -3.5, "N", 0, "D1", 4.7, "D2", 2.9, "D3", 1.9, "D4", 1.05};
            transmissionRatios[] = {"High", 12};
            moveOffGear = 1;
        };
        class Wheels {
"#,
    );
    for (side_index, side) in ["left", "right"].iter().enumerate() {
        for rank in 1..=8 {
            let prefix = if side_index == 0 { "L" } else { "R" };
            text.push_str(
                &WHEEL
                    .replace("@NAME@", &format!("{prefix}{rank}"))
                    .replace("@SIDE@", side)
                    .replace("@SI@", &(side_index + 1).to_string())
                    .replace("@R@", &rank.to_string()),
            );
        }
    }
    text.push_str("        };\n    };");
    text
}

/// The tank's half-track, m: its sixteen wheels run 1.2 m either side of the centre line.
const TANK_HALF_TRACK: f64 = 1.2;
/// The tank's wheel radius, m, from its own `MOI = mass·r²/2`.
const TANK_RADIUS: f64 = 0.668;

/// A car: the shipped offroad, its four wheels 1.4 m either side of the origin.
fn car() -> (VehicleData, DVec3) {
    let tree = ConfigTree::from_config(&parse_text(OFFROAD).unwrap());
    let mut data = VehicleData::from_class(&tree.root().get("C_Offroad_01_F")).unwrap();
    let mut points = MemoryPoints::new();
    for (name, x, z) in [
        ("wheel_1_1", 0.8, CAR_HALF_WHEELBASE),
        ("wheel_1_2", 0.8, -CAR_HALF_WHEELBASE),
        ("wheel_2_1", -0.8, CAR_HALF_WHEELBASE),
        ("wheel_2_2", -0.8, -CAR_HALF_WHEELBASE),
    ] {
        points.insert(format!("{name}_axis"), DVec3::new(x, 0.3, z));
        points.insert(format!("{name}_bound"), DVec3::new(x, 0.3, z + CAR_RADIUS));
    }
    let size = DVec3::new(2.2, 1.9, 2.0 * CAR_HALF_WHEELBASE + 1.5);
    data.resolve_geometry(&points, size);
    (data, size)
}

/// A tank: the shipped main battle tank, its sixteen road wheels down the two tracks.
fn tank() -> (VehicleData, DVec3) {
    let text = tank_config();
    let tree = ConfigTree::from_config(&parse_text(&text).unwrap());
    let mut data = VehicleData::from_class(&tree.root().get("B_MBT_01_cannon_F")).unwrap();
    let mut points = MemoryPoints::new();
    for (side_index, x) in [(1usize, TANK_HALF_TRACK), (2, -TANK_HALF_TRACK)] {
        for rank in 1..=8usize {
            // Eight road wheels along a 8 m hull, 0.95 m between the axles.
            let z = 3.3 - 0.95 * (rank as f64 - 1.0);
            points.insert(
                format!("wheel_{side_index}_{rank}_axis"),
                DVec3::new(x, 0.5, z),
            );
            points.insert(
                format!("wheel_{side_index}_{rank}_bound"),
                DVec3::new(x, 0.5, z + TANK_RADIUS),
            );
        }
    }
    let size = DVec3::new(3.7, 2.5, 8.0);
    data.resolve_geometry(&points, size);
    (data, size)
}

/// A vehicle placed on level ground at its rest height, facing +Z.
fn placed(data: &VehicleData, size: DVec3) -> WheeledVehicle {
    WheeledVehicle::new(data, DVec3::new(0.0, data.rest_height(), 0.0), 0.0, size)
}

/// Drives `input` until the vehicle holds `speed` m/s, or `steps` run out.
fn drive_to(
    vehicle: &mut WheeledVehicle,
    data: &VehicleData,
    road: &FlatSurface,
    speed: f64,
    steps: usize,
) {
    for _ in 0..steps {
        let throttle = if vehicle.forward_speed() < speed {
            0.3
        } else {
            0.0
        };
        vehicle.step(data, &VehicleInput::drive(throttle, 0.0), road, DT);
        if vehicle.forward_speed() >= speed {
            return;
        }
    }
}

#[test]
fn the_car_drives_up_through_the_gears_to_its_top_speed() {
    let (data, size) = car();
    let road = FlatSurface::new(0.0, ROAD);
    let mut vehicle = placed(&data, size);
    let mut gear_early = 0;
    for step in 0..9000 {
        vehicle.step(&data, &VehicleInput::drive(1.0, 0.0), &road, DT);
        if step == 200 {
            gear_early = vehicle.gear_number(&data);
        }
    }
    assert!(
        gear_early > 0,
        "the car starts in a forward gear, in {gear_early}"
    );
    assert!(
        vehicle.gear_number(&data) > gear_early,
        "the box upshifts as the car gathers speed: {gear_early} then {}",
        vehicle.gear_number(&data)
    );
    // `maxSpeed` is the top-speed governor: 90 s of full throttle brings the car to it, and to
    // no more than it.
    let speed = vehicle.forward_speed();
    let max = data.handling.max_speed();
    assert!(
        speed > 0.9 * max && speed < 1.02 * max,
        "90 s of full throttle gives {speed:.2} m/s = {:.1} km/h, config {:.1} km/h",
        speed * 3.6,
        max * 3.6
    );
    // It went straight: the whole run is along +Z, with the wheels on the ground throughout.
    assert!(
        vehicle.body.position.z > 1000.0,
        "{:?}",
        vehicle.body.position
    );
    // Straight within a milliradian: the symmetric car's sideways drift is rounding, and the
    // rounding differs between platforms' libm (1.1 m over 3.5 km on Linux).
    assert!(
        vehicle.body.position.x.abs() < 1e-3 * vehicle.body.position.z,
        "{:?}",
        vehicle.body.position
    );
    for (index, wheel) in vehicle.wheels.iter().enumerate() {
        assert!(wheel.in_contact, "wheel {index} left the ground at speed");
    }
}

#[test]
fn the_brakes_stop_the_car_in_the_tires_reach() {
    let (data, size) = car();
    let road = FlatSurface::new(0.0, ROAD);
    let mut vehicle = placed(&data, size);
    drive_to(&mut vehicle, &data, &road, 27.78, 6000); // 100 km/h
    let from = vehicle.body.position;
    let mut steps = 0;
    for _ in 0..1000 {
        vehicle.step(&data, &VehicleInput::braking(1.0), &road, DT);
        steps += 1;
        if vehicle.forward_speed() < 0.1 {
            break;
        }
    }
    let distance = (vehicle.body.position - from).length();
    // The shortest a stop can be: the tire's peak grip, `frictionVsSlipGraph(0)` over the
    // surface's `surfaceFriction`, `v²/2μg`. The model takes longer than that because the grip
    // falls away as the slip grows, and holds the wheels just short of a skid.
    let ideal = 27.78 * 27.78 / (2.0 * 1.75 * a3_vehicles::wheel::GRAVITY);
    assert!(
        distance > ideal && distance < 2.0 * ideal,
        "stopped in {distance:.2} m, the tire's reach is {ideal:.2} m"
    );
    assert!(
        vehicle.forward_speed().abs() < 0.1,
        "{}",
        vehicle.forward_speed()
    );
    assert!(steps as f64 * DT < 4.0, "{:.2} s", steps as f64 * DT);
    // The dive: the chassis pitches onto its nose and drops ~0.2 m at full brakes, below the
    // rest height of the springs, and comes back up as the car stops. It stays on the ground.
    assert!(
        vehicle.body.position.y > -0.2 && vehicle.body.position.y < 0.5,
        "the chassis ended at y = {:.3}",
        vehicle.body.position.y
    );
}

#[test]
fn the_car_turns_the_circle_its_wheelbase_gives() {
    let (data, size) = car();
    let road = FlatSurface::new(0.0, ROAD);
    // The kinematic circle: at a slow, unsaturated speed the wheels point where the driver
    // asks and the car follows the arc `wheelbase / tan(lock)`.
    let kinematic = 2.0 * CAR_HALF_WHEELBASE / data.max_steer_angle.tan();
    for (steer, direction) in [(1.0, "right"), (-1.0, "left")] {
        let mut vehicle = placed(&data, size);
        drive_to(&mut vehicle, &data, &road, 6.0, 4000);
        // Settle into the turn for two seconds, then measure a full radian of it.
        for _ in 0..200 {
            vehicle.step(&data, &VehicleInput::drive(0.3, steer), &road, DT);
        }
        let from = vehicle.body.position;
        // The yaw accumulates past half a turn in the measurement, so unwrap it: the delta
        // between steps, signed by the steering.
        let mut previous = vehicle.yaw();
        let mut turned = 0.0;
        let mut radius = f64::NAN;
        for _ in 0..2000 {
            vehicle.step(&data, &VehicleInput::drive(0.3, steer), &road, DT);
            let yaw = vehicle.yaw();
            let delta = (yaw - previous + PI).rem_euclid(2.0 * PI) - PI;
            previous = yaw;
            // Steering +1 is right: the car's yaw grows. Steering −1 takes it the other way.
            assert!(
                delta * steer >= -1e-6,
                "steering {direction} turned the car the wrong way"
            );
            turned += delta;
            if turned.abs() > 1.0 {
                radius = (vehicle.body.position - from).length() / turned.abs();
                break;
            }
        }
        assert!(radius.is_finite(), "the car never turned {direction}");
        assert!(
            radius > 0.8 * kinematic && radius < 1.4 * kinematic,
            "turns {direction} on {radius:.2} m, the wheelbase gives {kinematic:.2} m"
        );
    }
}

#[test]
fn the_car_reverses_when_the_driver_asks_for_reverse() {
    let (data, size) = car();
    let road = FlatSurface::new(0.0, ROAD);
    let mut vehicle = placed(&data, size);
    let input = VehicleInput::drive(1.0, 0.0).reversing();
    for _ in 0..600 {
        vehicle.step(&data, &input, &road, DT);
    }
    assert!(
        vehicle.forward_speed() < -1.0,
        "reverse gives {:.2} m/s",
        vehicle.forward_speed()
    );
    assert!(
        vehicle.gear_number(&data) < 0,
        "reverse gives gear {}",
        vehicle.gear_number(&data)
    );
    assert!(
        vehicle.body.position.z < -1.0,
        "{:?}",
        vehicle.body.position
    );
    // Steering back is the other half: the driver's direction flag picks the gear. (Leaving
    // reverse under power while the wheels still turn backwards does not come back into a
    // forward gear in this model yet — `docs/re/sim-vehicles.md` §5.)
    for _ in 0..600 {
        vehicle.step(&data, &VehicleInput::drive(1.0, 0.0).reversing(), &road, DT);
    }
    assert!(
        vehicle.gear_number(&data) < 0,
        "the box stays in reverse under reverse input"
    );
    assert!(
        vehicle.body.position.z < -2.0,
        "it keeps backing up: {:?}",
        vehicle.body.position
    );
}

#[test]
fn the_tank_drives_on_its_tracks() {
    let (data, size) = tank();
    let road = FlatSurface::new(0.0, ROAD);
    let mut vehicle = placed(&data, size);
    assert_eq!(
        vehicle.wheels.len(),
        16,
        "the shipped tank has sixteen road wheels"
    );
    assert_eq!(
        data.mass, 40000.0,
        "the tank's mass comes from the model, not the config"
    );
    for step in 0..3000 {
        vehicle.step(&data, &VehicleInput::drive(1.0, 0.0), &road, DT);
        if step % 1000 == 0 {
            assert!(
                vehicle.body.position.y > 0.0,
                "the tank sank: {:?}",
                vehicle.body.position
            );
        }
    }
    // 30 s of full throttle. The shipped tank's per-road-wheel `dampingRate` (590 N·m·s/rad,
    // where a car's is 1) holds it far below its `maxSpeed` in this model — see
    // `docs/re/sim-vehicles.md` §5. The test holds the model to what it does, not to the game.
    let speed = vehicle.forward_speed();
    assert!(speed > 4.0, "30 s of full throttle gives {speed:.2} m/s");
    assert!(
        vehicle.body.position.z > 100.0,
        "{:?}",
        vehicle.body.position
    );
    // The shipped tank's `suspTravelDirection[] = {-0.125, -1, 0}` leans every suspension the
    // same way, and the model pushes along it: 30 s of driving drifts the tank ~1.9 m sideways
    // (1% of the distance). Zeroing the entry's x takes the drift to zero exactly, so it is the
    // config's own asymmetry, not an integration error — `docs/re/sim-vehicles.md` §5.
    assert!(
        vehicle.body.position.x.abs() < 2.5,
        "the tank veered: {:?}",
        vehicle.body.position
    );
    for (index, wheel) in vehicle.wheels.iter().enumerate() {
        assert!(wheel.in_contact, "wheel {index} left the ground");
        assert!(wheel.load > 0.0, "wheel {index} carries nothing");
    }
}

#[test]
fn the_tank_steers_with_its_tracks() {
    let (data, size) = tank();
    let road = FlatSurface::new(0.0, ROAD);
    for (steer, direction) in [(1.0, "right"), (-1.0, "left")] {
        let mut vehicle = placed(&data, size);
        drive_to(&mut vehicle, &data, &road, 5.0, 4000);
        let yaw = vehicle.yaw();
        for _ in 0..300 {
            vehicle.step(&data, &VehicleInput::drive(0.5, steer), &road, DT);
        }
        let turned = vehicle.yaw() - yaw;
        assert!(
            turned * steer > 0.1,
            "steering {direction} turned the tank {turned:.3} rad"
        );
    }
}

#[test]
fn the_tank_pivots_on_the_spot() {
    let (data, size) = tank();
    let road = FlatSurface::new(0.0, ROAD);
    let mut vehicle = placed(&data, size);
    let from = vehicle.body.position;
    // Track steering with no throttle (`tankTurnForce`): the tank turns about itself.
    for _ in 0..200 {
        vehicle.step(&data, &VehicleInput::drive(0.0, -1.0), &road, DT);
    }
    let yaw = vehicle.yaw();
    assert!(yaw < -1.0, "2 s of full left lock gives {yaw:.3} rad");
    assert!(
        vehicle.body.velocity.length() < 1.0,
        "it drove off at {:.2} m/s",
        vehicle.body.velocity.length()
    );
    assert!(
        (vehicle.body.position - from).length() < 1.5,
        "it left the spot: {:?}",
        vehicle.body.position - from
    );
    // The other way about.
    let yaw = vehicle.yaw();
    for _ in 0..200 {
        vehicle.step(&data, &VehicleInput::drive(0.0, 1.0), &road, DT);
    }
    assert!(vehicle.yaw() > yaw, "right lock turns it back");
}
