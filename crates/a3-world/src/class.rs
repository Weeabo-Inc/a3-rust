//! The engine's class tree and the config `simulation` values that select a class.
//!
//! See `docs/re/world-object-model.md` ("Class tree", "`simulation` → C++ class").

/// A node of the original engine's Object class tree, for the engine-level kind questions
/// ("is this a Transport?"). Config inheritance (`isKindOf`) is a separate question answered by
/// the config tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntityClass {
    Object,
    Entity,
    Shot,
    EntityAi,
    Building,
    Thing,
    FlagCarrier,
    Target,
    EntityAiFull,
    Person,
    Man,
    Animal,
    InvisibleVehicle,
    Transport,
    TankOrCar,
    Car,
    Motorcycle,
    Tank,
    Ship,
    PlaneOrHeli,
    Helicopter,
    Airplane,
    Parachute,
}

impl EntityClass {
    /// The primary base class, `None` for the root.
    pub fn parent(self) -> Option<EntityClass> {
        use EntityClass::*;
        Some(match self {
            Object => return None,
            Entity => Object,
            Shot | EntityAi => Entity,
            Building | Thing | FlagCarrier | Target | EntityAiFull => EntityAi,
            Person | Transport => EntityAiFull,
            Man | Animal | InvisibleVehicle => Person,
            TankOrCar | PlaneOrHeli | Parachute => Transport,
            Car | Motorcycle | Tank | Ship => TankOrCar,
            Helicopter | Airplane => PlaneOrHeli,
        })
    }

    /// Whether `self` is `other` or derives from it.
    pub fn is_kind_of(self, other: EntityClass) -> bool {
        let mut class = Some(self);
        while let Some(c) = class {
            if c == other {
                return true;
            }
            class = c.parent();
        }
        false
    }
}

/// The concrete engine class an Entity type uses, chosen by its config `simulation` value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SimulationClass {
    Soldier,
    UavPilot,
    Animal,
    Invisible,
    Curator,
    HeadlessClient,
    Car,
    CarX,
    Motorcycle,
    Tank,
    TankX,
    Ship,
    ShipX,
    HovercraftX,
    SubmarineX,
    Helicopter,
    HelicopterX,
    HelicopterRtd,
    Airplane,
    AirplaneX,
    Parachute,
    Paraglide,
    House,
    Church,
    Fountain,
    Fire,
    Airport,
    FlagCarrier,
    Thing,
    ThingX,
    ThingEffect,
    BreakableHousePart,
    LaserTarget,
    NvMarker,
    ArtilleryMarker,
    SuppressTarget,
    SeaGull,
}

/// `simulation` value (lower case) → class. Pairs confirmed in the factory are marked in the RE
/// doc; `tankx`, `shipx`, `house`, `church`, `animal` are by name (issue #117).
const SIMULATIONS: &[(&str, SimulationClass)] = &[
    ("soldier", SimulationClass::Soldier),
    ("uavpilot", SimulationClass::UavPilot),
    ("animal", SimulationClass::Animal),
    ("invisible", SimulationClass::Invisible),
    ("curator", SimulationClass::Curator),
    ("headlessclient", SimulationClass::HeadlessClient),
    ("car", SimulationClass::Car),
    ("carx", SimulationClass::CarX),
    ("motorcycle", SimulationClass::Motorcycle),
    ("tank", SimulationClass::Tank),
    ("tankx", SimulationClass::TankX),
    ("ship", SimulationClass::Ship),
    ("shipx", SimulationClass::ShipX),
    ("hovercraftx", SimulationClass::HovercraftX),
    ("submarinex", SimulationClass::SubmarineX),
    ("helicopter", SimulationClass::Helicopter),
    ("helicopterx", SimulationClass::HelicopterX),
    ("helicopterrtd", SimulationClass::HelicopterRtd),
    ("airplane", SimulationClass::Airplane),
    ("airplanex", SimulationClass::AirplaneX),
    ("parachute", SimulationClass::Parachute),
    ("paraglide", SimulationClass::Paraglide),
    ("house", SimulationClass::House),
    ("housesimulated", SimulationClass::House),
    ("breakablehouseanimated", SimulationClass::House),
    ("church", SimulationClass::Church),
    ("fountain", SimulationClass::Fountain),
    ("fire", SimulationClass::Fire),
    ("airport", SimulationClass::Airport),
    ("flagcarrier", SimulationClass::FlagCarrier),
    ("thing", SimulationClass::Thing),
    ("thingx", SimulationClass::ThingX),
    ("thingeffect", SimulationClass::ThingEffect),
    ("breakablehousepart", SimulationClass::BreakableHousePart),
    (
        "breakablehouseanimatedpart",
        SimulationClass::BreakableHousePart,
    ),
    ("lasertarget", SimulationClass::LaserTarget),
    ("nvmarker", SimulationClass::NvMarker),
    ("artillerymarker", SimulationClass::ArtilleryMarker),
    ("suppresstarget", SimulationClass::SuppressTarget),
    ("seagull", SimulationClass::SeaGull),
];

impl SimulationClass {
    /// The class for a config `simulation` value (case-insensitive), or `None` if the value is
    /// not (yet) known.
    pub fn from_simulation(value: &str) -> Option<SimulationClass> {
        SIMULATIONS
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(value))
            .map(|&(_, class)| class)
    }

    /// The node of the engine class tree this class sits under.
    pub fn engine_class(self) -> EntityClass {
        use SimulationClass as S;
        match self {
            S::Soldier | S::UavPilot => EntityClass::Man,
            S::Animal => EntityClass::Animal,
            S::Invisible | S::Curator | S::HeadlessClient => EntityClass::InvisibleVehicle,
            S::Car | S::CarX => EntityClass::Car,
            S::Motorcycle => EntityClass::Motorcycle,
            S::Tank | S::TankX => EntityClass::Tank,
            S::Ship | S::ShipX | S::HovercraftX | S::SubmarineX => EntityClass::Ship,
            S::Helicopter | S::HelicopterX | S::HelicopterRtd => EntityClass::Helicopter,
            S::Airplane | S::AirplaneX => EntityClass::Airplane,
            S::Parachute | S::Paraglide => EntityClass::Parachute,
            S::House | S::Church | S::Fountain | S::Fire | S::Airport => EntityClass::Building,
            S::FlagCarrier => EntityClass::FlagCarrier,
            S::Thing | S::ThingX | S::ThingEffect | S::BreakableHousePart => EntityClass::Thing,
            S::LaserTarget | S::NvMarker | S::ArtilleryMarker | S::SuppressTarget => {
                EntityClass::Target
            }
            S::SeaGull => EntityClass::Entity,
        }
    }

    /// Whether this class is `other` or derives from it in the engine class tree.
    pub fn is_kind_of(self, other: EntityClass) -> bool {
        self.engine_class().is_kind_of(other)
    }

    /// Whether the original drives this class with PhysX (`*EPE` classes, the `...x` values).
    pub fn is_physx(self) -> bool {
        use SimulationClass as S;
        matches!(
            self,
            S::CarX
                | S::TankX
                | S::ShipX
                | S::HovercraftX
                | S::SubmarineX
                | S::HelicopterX
                | S::AirplaneX
                | S::ThingX
        )
    }
}
