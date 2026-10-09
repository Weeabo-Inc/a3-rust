//! Scripted flights of shipped aircraft (their config classes and models): hover, top speed,
//! rotor start, take-off. Skipped when `A3_ROOT` is unset.
//!
//! The pilot here is a test pilot: it sets the wanted controls the way the player's keys do
//! (`FlightInput`) or, to hold an attitude, steers the wanted cyclic or elevator itself. The
//! numbers each test prints are the model's; the assertions compare them with what the config
//! claims (`maxSpeed`, `landingSpeed`, `startDuration`).

use std::sync::OnceLock;

use a3_flight::heli::{self, HeliState, HeliType};
use a3_flight::plane::{self, PlaneState, PlaneType};
use a3_flight::{Airframe, Environment, FlatGround, FlightInput, RigidBody};
use a3_gamedata::{GameData, LoadOptions};
use glam::DVec3;

const DT: f64 = 1.0 / 15.0;

fn game() -> Option<&'static GameData> {
    static GAME: OnceLock<Option<GameData>> = OnceLock::new();
    GAME.get_or_init(|| {
        let root = std::env::var_os("A3_ROOT")?;
        Some(GameData::load(&LoadOptions::new(root)).expect("game data"))
    })
    .as_ref()
}

fn airframe(game: &GameData, class: &str) -> Airframe {
    let cfg = game.config.root().get("CfgVehicles").get(class);
    let mut path = cfg.get("model").text().trim_start_matches('\\').to_string();
    if !path.to_ascii_lowercase().ends_with(".p3d") {
        path.push_str(".p3d");
    }
    let bytes = game.vfs.open(&path).expect("model");
    let model = a3_p3d::Model::from_bytes(&bytes).expect("odol");
    Airframe::from_model(&model)
}

fn helicopter(class: &str) -> Option<(HeliType, Airframe)> {
    let Some(game) = game() else {
        eprintln!("skipping: A3_ROOT not set");
        return None;
    };
    let cfg = game.config.root().get("CfgVehicles").get(class);
    Some((HeliType::from_config(&cfg), airframe(game, class)))
}

fn airplane(class: &str) -> Option<(PlaneType, Airframe)> {
    let Some(game) = game() else {
        eprintln!("skipping: A3_ROOT not set");
        return None;
    };
    let cfg = game.config.root().get("CfgVehicles").get(class);
    let frame = airframe(game, class);
    let mut ty = PlaneType::from_config(&cfg);
    ty.place_wheels(&frame);
    Some((ty, frame))
}

/// Nose-down pitch, rad.
fn pitch(body: &RigidBody) -> f64 {
    -body.direction().y.asin()
}

/// Left bank, rad.
fn bank(body: &RigidBody) -> f64 {
    body.aside().y.asin()
}

/// Flies a helicopter with the collective keys released (vertical speed hold) and the cyclic
/// steered to hold `nose_down` pitch and wings level, for `seconds`.
fn fly_attitude(
    ty: &HeliType,
    frame: &Airframe,
    state: &mut HeliState,
    body: &mut RigidBody,
    env: &Environment<'_>,
    nose_down: f64,
    seconds: f64,
) {
    let steps = (seconds / DT) as usize;
    for _ in 0..steps {
        state.pilot(&FlightInput::default(), body);
        let w = body.to_model(body.angular_velocity);
        state.controls.cyclic_forward = (nose_down - pitch(body)) * 4.0 - w.x * 2.0;
        state.controls.cyclic_aside = -bank(body) * 4.0 - w.z * 2.0;
        // Left pedal (positive) yaws left (negative about Y).
        state.controls.rudder = (w.y * 2.0).clamp(-1.0, 1.0);
        heli::step(ty, frame, state, body, env, DT);
    }
}

#[test]
fn the_hummingbird_hovers_with_the_keys_released() {
    let Some((ty, frame)) = helicopter("B_Heli_Light_01_F") else {
        return;
    };
    let ground = FlatGround::new(0.0);
    let env = Environment::calm(&ground);
    let mut body = RigidBody::at(DVec3::new(0.0, 200.0, 0.0), 0.0);
    let mut state = HeliState::flying();
    state.collective = 0.32;
    for _ in 0..(60.0 / DT) as usize {
        state.pilot(&FlightInput::default(), &body);
        heli::step(&ty, &frame, &mut state, &mut body, &env, DT);
    }
    let drift = DVec3::new(body.velocity.x, 0.0, body.velocity.z).length();
    eprintln!(
        "hover after 60 s: height {:.2} m, vy {:.3} m/s, drift {:.2} m/s, pitch {:.2}°, bank {:.2}°, collective {:.3}",
        body.position.y,
        body.velocity.y,
        drift,
        pitch(&body).to_degrees(),
        bank(&body).to_degrees(),
        state.collective
    );
    assert!((body.position.y - 200.0).abs() < 5.0);
    assert!(body.velocity.y.abs() < 0.5);
    assert!(drift < 1.0);
    assert!(pitch(&body).abs() < 5f64.to_radians() && bank(&body).abs() < 5f64.to_radians());
}

/// The fastest level flight of a helicopter: the nose is held lower and lower (1° at a time)
/// with the collective keys released, until the vertical speed hold runs out of collective.
fn top_level_speed(ty: &HeliType, frame: &Airframe) -> f64 {
    let ground = FlatGround::new(0.0);
    let env = Environment::calm(&ground);
    let mut best: f64 = 0.0;
    for degrees in 2..=40 {
        let mut body = RigidBody::at(DVec3::new(0.0, 500.0, 0.0), 0.0);
        let mut state = HeliState::flying();
        state.collective = 0.3;
        fly_attitude(
            ty,
            frame,
            &mut state,
            &mut body,
            &env,
            f64::from(degrees).to_radians(),
            60.0,
        );
        if body.velocity.y.abs() > 1.0 {
            break;
        }
        best = best.max(body.velocity.length());
    }
    best
}

#[test]
fn helicopters_top_out_near_their_max_speed() {
    let classes = [
        "B_Heli_Light_01_F",
        "O_Heli_Light_02_dynamicLoadout_F",
        "I_Heli_light_03_dynamicLoadout_F",
        "B_Heli_Transport_01_F",
        "I_Heli_Transport_02_F",
        "B_Heli_Transport_03_F",
        "O_Heli_Transport_04_F",
        "B_Heli_Attack_01_dynamicLoadout_F",
        "O_Heli_Attack_02_dynamicLoadout_F",
    ];
    let mut failed = Vec::new();
    for class in classes {
        let Some((ty, frame)) = helicopter(class) else {
            return;
        };
        let top = top_level_speed(&ty, &frame);
        let ratio = top / ty.max_speed;
        eprintln!(
            "{class:<36} top level speed {:>6.1} km/h, maxSpeed {:>6.1} km/h ({:>5.1} %), mass {:>6.0} kg",
            top * 3.6,
            ty.max_speed * 3.6,
            ratio * 100.0,
            frame.mass
        );
        // The rotor-dive helicopters (Blackfoot, Kajman) reach about 71 %: the player's keys
        // leave the rotor dive at 0, which only the autopilot moves (`docs/re/sim-air.md` §5).
        if !(0.65..=1.1).contains(&ratio) {
            failed.push(class);
        }
    }
    assert!(failed.is_empty(), "{failed:?}");
}

#[test]
fn the_hummingbird_spins_up_on_the_ground_and_lifts_off() {
    let Some((ty, frame)) = helicopter("B_Heli_Light_01_F") else {
        return;
    };
    let ground = FlatGround::new(10.0);
    let env = Environment::calm(&ground);
    let mut body = RigidBody::at(DVec3::new(0.0, 10.0 - frame.bottom(), 0.0), 0.0);
    let mut state = HeliState::new();
    state.engine_on = true;
    let mut seconds = 0.0;
    while state.rotor < 1.0 && seconds < 60.0 {
        state.pilot(&FlightInput::default(), &body);
        heli::step(&ty, &frame, &mut state, &mut body, &env, DT);
        seconds += DT;
    }
    let rest = body.position;
    eprintln!(
        "rotor at full speed after {seconds:.1} s, still at {rest:?}, on land {}",
        state.touch.land
    );
    assert!((seconds - ty.start_duration).abs() < 1.0);
    assert!(state.touch.land);
    let raise = FlightInput {
        heli_collective_raise: 1.0,
        ..FlightInput::default()
    };
    for _ in 0..(10.0 / DT) as usize {
        state.pilot(&raise, &body);
        heli::step(&ty, &frame, &mut state, &mut body, &env, DT);
    }
    eprintln!(
        "after 10 s of collective: height {:.1} m above the start, climbing {:.2} m/s",
        body.position.y - rest.y,
        body.velocity.y
    );
    assert!(body.position.y - rest.y > 20.0);
    assert!(!state.touch.land);
}

/// The planes of the shipped game, one per airframe.
const PLANES: [&str; 8] = [
    "C_Plane_Civil_01_F",
    "B_Plane_CAS_01_dynamicLoadout_F",
    "O_Plane_CAS_02_dynamicLoadout_F",
    "I_Plane_Fighter_03_dynamicLoadout_F",
    "B_Plane_Fighter_01_F",
    "O_Plane_Fighter_02_F",
    "I_Plane_Fighter_04_F",
    "B_UAV_02_dynamicLoadout_F",
];

/// A take-off run on a flat runway: full throttle on the keys, the stick pulled back from 90 %
/// of the landing speed. Returns the speed at which the wheels left the ground for good.
fn take_off(ty: &PlaneType, frame: &Airframe) -> Option<f64> {
    let ground = FlatGround::new(0.0);
    let env = Environment::calm(&ground);
    let lowest_wheel = ty
        .wheels
        .iter()
        .filter(|w| w.is_placed())
        .map(|w| w.position.y - w.radius)
        .fold(frame.bottom(), f64::min);
    let mut body = RigidBody::at(DVec3::new(0.0, -lowest_wheel, 0.0), 0.0);
    let mut state = PlaneState::new();
    let mut airborne_for = 0.0;
    let mut lift_off = None;
    let mut seconds = 0.0;
    while seconds < 180.0 {
        let rotate = body.model_speed().z > ty.landing_speed * 0.9;
        let input = FlightInput {
            heli_up: 1.0,
            heli_back: if rotate { 1.0 } else { 0.0 },
            ..FlightInput::default()
        };
        state.pilot(ty, &input, DT);
        plane::step(ty, frame, &mut state, &mut body, &env, DT);
        seconds += DT;
        if state.touch.land {
            airborne_for = 0.0;
        } else {
            if airborne_for == 0.0 {
                lift_off = Some(body.model_speed().z);
            }
            airborne_for += DT;
            if airborne_for > 2.0 {
                return lift_off;
            }
        }
    }
    None
}

/// Level flight at full throttle for four minutes: the elevator holds the climb rate at 0, the
/// ailerons the wings level. Returns the speed reached.
fn level_speed(ty: &PlaneType, frame: &Airframe) -> f64 {
    let ground = FlatGround::new(0.0);
    let env = Environment::calm(&ground);
    let mut body = RigidBody::at(DVec3::new(0.0, 1000.0, 0.0), 0.0);
    body.velocity = DVec3::new(0.0, 0.0, ty.max_speed * 0.7);
    let mut state = PlaneState::flying(1.0);
    for _ in 0..(240.0 / DT) as usize {
        let w = body.to_model(body.angular_velocity);
        state.controls.elevator = (body.velocity.y * 0.05 - w.x).clamp(-1.0, 1.0);
        state.controls.aileron = (-bank(&body) * 2.0 - w.z).clamp(-1.0, 1.0);
        // Rudder left (positive) yaws left (negative about Y).
        state.controls.rudder = w.y.clamp(-1.0, 1.0);
        plane::step(ty, frame, &mut state, &mut body, &env, DT);
    }
    body.model_speed().z
}

#[test]
fn planes_take_off_near_their_landing_speed() {
    let mut failed = Vec::new();
    for class in PLANES {
        let Some((ty, frame)) = airplane(class) else {
            return;
        };
        let speed = take_off(&ty, &frame);
        let ratio = speed.unwrap_or(0.0) / ty.landing_speed;
        eprintln!(
            "{class:<36} lift-off {:>6.1} km/h, landingSpeed {:>6.1} km/h ({:>5.1} %), wheels {}/{}, mass {:>6.0} kg",
            speed.unwrap_or(0.0) * 3.6,
            ty.landing_speed * 3.6,
            ratio * 100.0,
            ty.wheels.iter().filter(|w| w.is_placed()).count(),
            ty.wheels.len(),
            frame.mass
        );
        if !(0.85..=1.15).contains(&ratio) {
            failed.push(class);
        }
    }
    assert!(failed.is_empty(), "{failed:?}");
}

#[test]
fn planes_top_out_near_their_max_speed_in_level_flight() {
    let mut failed = Vec::new();
    for class in PLANES {
        let Some((ty, frame)) = airplane(class) else {
            return;
        };
        let speed = level_speed(&ty, &frame);
        let ratio = speed / ty.max_speed;
        eprintln!(
            "{class:<36} level speed {:>7.1} km/h, maxSpeed {:>7.1} km/h ({:>5.1} %)",
            speed * 3.6,
            ty.max_speed * 3.6,
            ratio * 100.0
        );
        if !(0.8..=1.15).contains(&ratio) {
            failed.push(class);
        }
    }
    assert!(failed.is_empty(), "{failed:?}");
}
