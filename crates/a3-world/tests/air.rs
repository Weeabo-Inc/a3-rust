//! The air family through `World::simulate`: shipped helicopters and planes with their config
//! classes and models (skipped when `A3_ROOT` is unset), and what an aircraft without a flight
//! model does.

use std::sync::{Arc, OnceLock};

use a3_flight::FlightInput;
use a3_gamedata::{GameData, LoadOptions};
use a3_world::{AircraftBank, ClassState, ClientId, Create, EntityType, SimulationClass, World};
use glam::DVec3;

/// 60 frames a second.
const FRAME: f64 = 1.0 / 60.0;

fn game() -> Option<&'static GameData> {
    static GAME: OnceLock<Option<GameData>> = OnceLock::new();
    GAME.get_or_init(|| {
        let root = std::env::var_os("A3_ROOT")?;
        Some(GameData::load(&LoadOptions::new(root)).expect("game data"))
    })
    .as_ref()
}

/// A World with the shipped aircraft and `class` created at `position`.
fn world_with(class: &str, position: DVec3) -> Option<(World, a3_world::EntityId)> {
    let Some(game) = game() else {
        eprintln!("skipping: A3_ROOT not set");
        return None;
    };
    let mut types = a3_world::TypeBank::new(game.config.clone());
    let mut world = World::new(ClientId::SERVER);
    world.load_aircraft(AircraftBank::new(
        game.config.clone(),
        Arc::new(game.vfs.clone()),
    ));
    let id = world
        .create(Create::new(types.get(class).unwrap(), position))
        .unwrap();
    Some((world, id))
}

fn run(world: &mut World, seconds: f64) {
    for _ in 0..(seconds / FRAME) as usize {
        world.simulate(FRAME);
    }
}

#[test]
fn an_aircraft_without_a_flight_model_stays_put() {
    let mut world = World::new(ClientId::SERVER);
    let ty = Arc::new(EntityType::new("Heli", SimulationClass::HelicopterRtd));
    let id = world
        .create(Create::new(ty, DVec3::new(0.0, 50.0, 0.0)))
        .unwrap();
    run(&mut world, 1.0);
    let e = world.entity(id).unwrap();
    assert_eq!(e.position(), DVec3::new(0.0, 50.0, 0.0));
    assert!(matches!(e.class_state(), ClassState::Air(a) if a.flight.is_none()));
}

#[test]
fn a_piloted_hummingbird_holds_its_height_with_the_keys_released() {
    let Some((mut world, id)) = world_with("B_Heli_Light_01_F", DVec3::new(0.0, 150.0, 0.0)) else {
        return;
    };
    let air = world.air_state_mut(id).unwrap();
    air.flight.as_mut().expect("flight model").take_to_the_air();
    world.set_flight_input(id, Some(FlightInput::default()));
    run(&mut world, 30.0);
    let e = world.entity(id).unwrap();
    eprintln!("after 30 s: {:?}, v {:?}", e.position(), e.velocity());
    assert!((e.position().y - 150.0).abs() < 10.0);
    assert!(e.velocity().length() < 1.0);
    assert!(world.is_engine_on(id));
}

#[test]
fn a_helicopter_with_a_commanded_height_holds_it_with_nobody_flying() {
    let Some((mut world, id)) = world_with("B_Heli_Light_01_F", DVec3::new(0.0, 130.0, 0.0)) else {
        return;
    };
    world
        .air_state_mut(id)
        .unwrap()
        .flight
        .as_mut()
        .expect("flight model")
        .take_to_the_air();
    // `flyInHeight 100`: the AI pilot flies it to 100 m above the ground below (this World has
    // no terrain, so the surface is at 0) with nobody at the controls, and holds it there.
    world.object_state_mut(id).fly_in_height = Some(100.0);

    run(&mut world, 60.0);

    let e = world.entity(id).unwrap();
    eprintln!("after 60 s: {:?}, v {:?}", e.position(), e.velocity());
    assert!(
        (e.position().y - 100.0).abs() < 15.0,
        "holds the commanded altitude: {:?}",
        e.position()
    );
    assert!(
        e.velocity().y.abs() < 5.0,
        "and has stopped climbing or sinking: {:?}",
        e.velocity()
    );
}

#[test]
fn a_parked_hummingbird_starts_its_engine_and_climbs_on_the_collective() {
    let Some((mut world, id)) = world_with("B_Heli_Light_01_F", DVec3::ZERO) else {
        return;
    };
    // Stand it on the ground (the World has no terrain: the surface is at 0).
    let bottom = world
        .air_state(id)
        .and_then(|a| a.flight.as_ref())
        .map(|f| f.data().airframe.bottom())
        .unwrap();
    world
        .entity_mut(id)
        .unwrap()
        .set_position(DVec3::new(0.0, -bottom, 0.0));
    world.set_flight_input(id, Some(FlightInput::default()));
    run(&mut world, 5.0);
    let parked = world.entity(id).unwrap().position();
    assert!(!world.is_engine_on(id));
    assert!(parked.y > 0.0 && parked.y < -bottom + 0.2, "{parked:?}");

    world.set_engine_on(id, true);
    run(&mut world, 21.0);
    world.set_flight_input(
        id,
        Some(FlightInput {
            heli_collective_raise: 1.0,
            ..FlightInput::default()
        }),
    );
    run(&mut world, 8.0);
    let e = world.entity(id).unwrap();
    eprintln!("after the climb: {:?}", e.position());
    assert!(e.position().y > parked.y + 20.0);
    let sources = world
        .air_state(id)
        .and_then(|a| a.flight.as_ref())
        .unwrap()
        .animation_sources(e, &world);
    let rpm = sources.iter().find(|(n, _)| *n == "rpm").unwrap().1;
    assert_eq!(rpm, 1.0);
}

#[test]
fn a_caesar_cruises_at_full_throttle() {
    let Some((mut world, id)) = world_with("C_Plane_Civil_01_F", DVec3::new(0.0, 500.0, 0.0))
    else {
        return;
    };
    world
        .air_state_mut(id)
        .unwrap()
        .flight
        .as_mut()
        .unwrap()
        .take_to_the_air();
    world
        .entity_mut(id)
        .unwrap()
        .set_velocity(DVec3::new(0.0, 0.0, 60.0));
    // Full throttle on the keys for two seconds, then hands off.
    world.set_flight_input(
        id,
        Some(FlightInput {
            heli_up: 1.0,
            ..FlightInput::default()
        }),
    );
    run(&mut world, 2.0);
    world.set_flight_input(id, Some(FlightInput::default()));
    run(&mut world, 10.0);
    let e = world.entity(id).unwrap();
    eprintln!("after 12 s: {:?}, v {:?}", e.position(), e.velocity());
    assert!(e.velocity().length() > 40.0);
    assert!(e.position().y > 300.0);
    assert!(e.position().z > 500.0);
}
