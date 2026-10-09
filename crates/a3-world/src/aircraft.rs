//! What the air family flies with: each aircraft type's flight model (`a3-flight`), read once
//! from its config class and its model, and the World operations a pilot uses.
//!
//! The host installs an [`AircraftBank`] ([`World::load_aircraft`]); every helicopter or plane
//! created afterwards gets its flight model at creation. Without a bank, or for a type whose
//! model cannot be read, the Entity has no flight model and stays where it is put.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use a3_config::ConfigTree;
use a3_flight::heli::HeliType;
use a3_flight::plane::PlaneType;
use a3_flight::{Airframe, FlightInput};
use a3_physics::FileSource;

use crate::sim::{AirState, Flight};
use crate::{ClassState, EntityClass, EntityId, EntityType, World};

/// Which flight model an aircraft type flies with, configured.
#[derive(Debug, Clone, PartialEq)]
pub enum FlightType {
    /// The basic helicopter model (`helicopterrtd`, `helicopter`).
    Heli(HeliType),
    /// The `airplanex` model (`airplane` too).
    Plane(PlaneType),
}

/// One aircraft type's flight model and airframe, shared by every Entity of the type.
#[derive(Debug, Clone, PartialEq)]
pub struct FlightData {
    pub kind: FlightType,
    pub airframe: Airframe,
}

/// Reads and caches [`FlightData`] per type name, from the merged config and the model files.
pub struct AircraftBank {
    config: Arc<ConfigTree>,
    files: Arc<dyn FileSource>,
    cache: HashMap<String, Option<Arc<FlightData>>>,
}

impl fmt::Debug for AircraftBank {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AircraftBank")
            .field("types", &self.cache.len())
            .finish()
    }
}

impl AircraftBank {
    /// A bank over the merged config and the game files (models are read through `files`).
    pub fn new(config: Arc<ConfigTree>, files: Arc<dyn FileSource>) -> AircraftBank {
        AircraftBank {
            config,
            files,
            cache: HashMap::new(),
        }
    }

    /// The flight data of `ty`, `None` for a type that is no aircraft or whose model is missing.
    pub fn get(&mut self, ty: &EntityType) -> Option<Arc<FlightData>> {
        let key = ty.name().to_ascii_lowercase();
        if let Some(data) = self.cache.get(&key) {
            return data.clone();
        }
        let data = self.read(ty).map(Arc::new);
        self.cache.insert(key, data.clone());
        data
    }

    fn read(&self, ty: &EntityType) -> Option<FlightData> {
        let class = ty.class().engine_class();
        let heli = class.is_kind_of(EntityClass::Helicopter);
        let plane = class.is_kind_of(EntityClass::Airplane);
        if !heli && !plane {
            return None;
        }
        let mut path = ty.model().trim_start_matches(['\\', '/']).to_string();
        if !path.to_ascii_lowercase().ends_with(".p3d") {
            path.push_str(".p3d");
        }
        let bytes = self.files.read(&path)?;
        let model = a3_p3d::Model::from_bytes(&bytes).ok()?;
        let airframe = Airframe::from_model(&model);
        let cfg = ty.config(&self.config);
        let kind = if heli {
            FlightType::Heli(HeliType::from_config(&cfg))
        } else {
            let mut plane = PlaneType::from_config(&cfg);
            plane.place_wheels(&airframe);
            FlightType::Plane(plane)
        };
        Some(FlightData { kind, airframe })
    }
}

impl World {
    /// Installs the aircraft bank: helicopters and planes created from now on fly.
    pub fn load_aircraft(&mut self, bank: AircraftBank) {
        self.aircraft = Some(bank);
    }

    /// Gives a just-created air Entity its flight model, when the bank has one for its type.
    pub(crate) fn init_flight(&mut self, id: EntityId) {
        let Some(entity) = self.entity(id) else {
            return;
        };
        if !matches!(entity.class_state(), ClassState::Air(_)) {
            return;
        }
        let ty = entity.entity_type().clone();
        let Some(data) = self.aircraft.as_mut().and_then(|bank| bank.get(&ty)) else {
            return;
        };
        let flight = Flight::new(data, &ty);
        if let Some(ClassState::Air(air)) = self.entity_mut(id).map(|e| e.class_state_mut()) {
            air.flight = Some(flight);
        }
    }

    /// The air state of an aircraft.
    pub fn air_state(&self, id: EntityId) -> Option<&AirState> {
        match self.entity(id)?.class_state() {
            ClassState::Air(air) => Some(air),
            _ => None,
        }
    }

    /// The air state of an aircraft, to change.
    pub fn air_state_mut(&mut self, id: EntityId) -> Option<&mut AirState> {
        match self.entity_mut(id)?.class_state_mut() {
            ClassState::Air(air) => Some(air),
            _ => None,
        }
    }

    /// The player's flight actions for an aircraft he pilots, this frame. `None` hands the
    /// controls back (no pilot).
    pub fn set_flight_input(&mut self, id: EntityId, input: Option<FlightInput>) {
        if let Some(air) = self.air_state_mut(id) {
            air.input = input;
        }
    }

    /// `engineOn`: starts or stops an aircraft's engine.
    pub fn set_engine_on(&mut self, id: EntityId, on: bool) {
        if let Some(flight) = self.air_state_mut(id).and_then(|a| a.flight.as_mut()) {
            flight.set_engine_on(on);
        }
    }

    /// `isEngineOn`.
    pub fn is_engine_on(&self, id: EntityId) -> bool {
        self.air_state(id)
            .and_then(|a| a.flight.as_ref())
            .is_some_and(Flight::engine_on)
    }
}
