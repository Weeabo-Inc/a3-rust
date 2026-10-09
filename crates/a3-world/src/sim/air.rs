//! Aircraft and parachutes (`PlaneOrHeli`, `Parachute`). Issue #126: the basic helicopter and
//! `airplanex` flight models of `a3-flight` (`docs/re/sim-air.md`); parachutes and RotorLib's
//! advanced model are not flown.
//!
//! Called once per simulation step of each Entity of this family, on every machine. Do the
//! authoritative work (forces, damage, decisions) only when `entity.is_local()`; a remote
//! Entity only advances from its last received state.

use std::sync::Arc;

use a3_flight::heli::{self, HeliState};
use a3_flight::plane::{self, PlaneState};
use a3_flight::{Environment, FlightInput, Ground, RigidBody};
use glam::DVec3;

use crate::aircraft::{FlightData, FlightType};
use crate::{ClassState, Entity, EntityType, World};

use super::StepContext;

/// Class-specific state of this family (`ClassState`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AirState {
    /// The flight model, once the World's aircraft bank gave the type one.
    pub flight: Option<Flight>,
    /// The player's flight actions, while a player pilots it; `None` when nobody does.
    pub input: Option<FlightInput>,
    /// Angular velocity, World space, rad/s.
    pub angular_velocity: DVec3,
}

/// The hit point indices the flight models read (`HitHRotor`, `HitVRotor`, `HitEngine`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct HitIndices {
    pub main_rotor: Option<usize>,
    pub tail_rotor: Option<usize>,
    pub engine: Option<usize>,
}

/// An aircraft's flight model and its state.
// One per aircraft, stored inline in its AirState; boxing the larger variant would only add a
// pointer chase per step.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub enum Flight {
    Heli {
        data: Arc<FlightData>,
        state: HeliState,
        hits: HitIndices,
    },
    Plane {
        data: Arc<FlightData>,
        state: PlaneState,
        hits: HitIndices,
    },
}

impl Flight {
    /// A parked aircraft of `ty` (engine off, plane gear down).
    pub(crate) fn new(data: Arc<FlightData>, ty: &EntityType) -> Flight {
        let damage = ty.damage();
        let hits = HitIndices {
            main_rotor: damage.hit_point_index("HitHRotor"),
            tail_rotor: damage.hit_point_index("HitVRotor"),
            engine: damage.hit_point_index("HitEngine"),
        };
        match data.kind {
            FlightType::Heli(_) => Flight::Heli {
                data,
                state: HeliState::new(),
                hits,
            },
            FlightType::Plane(_) => Flight::Plane {
                data,
                state: PlaneState::new(),
                hits,
            },
        }
    }

    /// Puts the aircraft in the air: engine running, rotor at speed or plane cruising with the
    /// gear up (what `createVehicle` with `"FLY"` does).
    pub fn take_to_the_air(&mut self) {
        match self {
            Flight::Heli { state, .. } => *state = HeliState::flying(),
            Flight::Plane { state, .. } => *state = PlaneState::flying(0.8),
        }
    }

    pub fn engine_on(&self) -> bool {
        match self {
            Flight::Heli { state, .. } => state.engine_on,
            Flight::Plane { state, .. } => state.engine_on,
        }
    }

    pub fn set_engine_on(&mut self, on: bool) {
        match self {
            Flight::Heli { state, .. } => state.engine_on = on,
            Flight::Plane { state, .. } => state.engine_on = on,
        }
    }

    /// The flight data of the type.
    pub fn data(&self) -> &FlightData {
        match self {
            Flight::Heli { data, .. } | Flight::Plane { data, .. } => data,
        }
    }

    /// Model animation source values (`docs/re/sim-air.md` §2.7): name (lower case) and value.
    pub fn animation_sources(&self, entity: &Entity, world: &World) -> Vec<(&'static str, f64)> {
        let body = body_of(entity, DVec3::ZERO);
        let mut sources = common_sources(entity, &body, world);
        match self {
            Flight::Heli { data, state, .. } => {
                let FlightType::Heli(ty) = &data.kind else {
                    return sources;
                };
                let turn = std::f64::consts::TAU;
                sources.extend([
                    (
                        "rotorh",
                        state.main_rotor_angle / turn * ty.main_rotor_speed,
                    ),
                    (
                        "rotorv",
                        state.tail_rotor_angle / turn * ty.back_rotor_speed,
                    ),
                    ("rpm", state.rotor),
                    ("cyclicforward", state.cyclic_forward),
                    ("cyclicaside", state.cyclic_aside),
                    ("collectivertd", state.collective_lever),
                    ("rudderrtd", state.pedal),
                ]);
            }
            Flight::Plane { data, state, .. } => {
                let FlightType::Plane(ty) = &data.kind else {
                    return sources;
                };
                sources.extend([
                    // _Inferred_: the engine's rpm state `engine·10·(thrust + 0.4)`, scaled
                    // to 1 at full thrust.
                    ("rpm", state.engine * (state.thrust + 0.4) / 1.4),
                    ("elevator", state.elevator),
                    ("aileron", state.aileron),
                    ("rudder", state.rudder),
                    ("flap", state.flaps),
                    ("gear", state.gear),
                    ("thrust", state.lever(ty)),
                ]);
            }
        }
        sources
    }
}

/// The sources every aircraft instrument panel reads.
fn common_sources(entity: &Entity, body: &RigidBody, world: &World) -> Vec<(&'static str, f64)> {
    let p = entity.position;
    let dir = body.direction();
    let aside = body.aside();
    vec![
        ("altbaro", p.y),
        ("altradar", p.y - world.surface_height(p.x, p.z)),
        ("speed", entity.velocity.length()),
        ("vertspeed", entity.velocity.y),
        ("horizonbank", aside.y.asin()),
        ("horizondive", -dir.y.asin()),
        ("direction", dir.x.atan2(dir.z)),
    ]
}

/// The World's surface as `a3-flight` asks for it: the terrain, and the sea at 0 where there is
/// a terrain.
struct WorldGround<'a>(&'a World);

impl Ground for WorldGround<'_> {
    fn height(&self, x: f64, z: f64) -> f64 {
        self.0.surface_height(x, z)
    }

    fn water_level(&self, _x: f64, _z: f64) -> f64 {
        if self.0.terrain().is_some() {
            0.0
        } else {
            f64::NEG_INFINITY
        }
    }
}

fn body_of(entity: &Entity, angular_velocity: DVec3) -> RigidBody {
    RigidBody {
        position: entity.position,
        orientation: entity.orientation,
        velocity: entity.velocity,
        angular_velocity,
    }
}

pub(crate) fn simulate(entity: &mut Entity, ctx: &mut StepContext<'_>, dt: f64) {
    if !entity.is_local() {
        return;
    }
    let destroyed = entity.is_destroyed();
    let damage =
        |index: Option<usize>| index.map_or(0.0, |i| f64::from(entity.hit_point_damage(i)));
    let ClassState::Air(air) = &entity.class_state else {
        return;
    };
    let Some(flight) = &air.flight else {
        return;
    };
    let mut flight = flight.clone();
    let input = air.input;
    let mut body = body_of(entity, air.angular_velocity);
    let ground = WorldGround(ctx.world());
    let env = Environment::calm(&ground);
    match &mut flight {
        Flight::Heli { data, state, hits } => {
            let FlightType::Heli(ty) = &data.kind else {
                return;
            };
            state.damage.main_rotor = damage(hits.main_rotor);
            state.damage.tail_rotor = damage(hits.tail_rotor);
            state.damage.engine = damage(hits.engine);
            state.damage.destroyed = destroyed;
            match input {
                Some(input) => state.pilot(&input, &body),
                None => state.no_pilot(),
            }
            heli::step(ty, &data.airframe, state, &mut body, &env, dt);
        }
        Flight::Plane { data, state, hits } => {
            let FlightType::Plane(ty) = &data.kind else {
                return;
            };
            state.damage.engine = damage(hits.engine);
            state.damage.destroyed = destroyed;
            if let Some(input) = input {
                state.pilot(ty, &input, dt);
            }
            plane::step(ty, &data.airframe, state, &mut body, &env, dt);
        }
    }
    entity.position = body.position;
    entity.orientation = body.orientation;
    entity.velocity = body.velocity;
    if let ClassState::Air(air) = &mut entity.class_state {
        air.angular_velocity = body.angular_velocity;
        air.flight = Some(flight);
    }
}
