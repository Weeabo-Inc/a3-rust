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
    Detector,
    Camera,
    StreetLamp,
    WindSock,
    RopeSegment,
    EntityAi,
    Building,
    Thing,
    FlagCarrier,
    Target,
    Rope,
    Vasi,
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
            Shot | Detector | Camera | StreetLamp | WindSock | RopeSegment | EntityAi => Entity,
            Building | Thing | FlagCarrier | Target | Rope | Vasi | EntityAiFull => EntityAi,
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
    // People and logic (CfgVehicles)
    Soldier,
    UavPilot,
    Animal,
    Invisible,
    Curator,
    HeadlessClient,
    // Vehicles (CfgVehicles)
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
    // Buildings and things (CfgVehicles)
    House,
    Church,
    Fountain,
    Fire,
    Airport,
    FlagCarrier,
    Vasi,
    Thing,
    ThingX,
    ThingEffect,
    BreakableHousePart,
    Rope,
    LaserTarget,
    NvMarker,
    ArtilleryMarker,
    SuppressTarget,
    // Projectiles (CfgAmmo)
    ShotBullet,
    ShotShell,
    ShotSpread,
    ShotMissile,
    ShotRocket,
    ShotGrenade,
    ShotSmoke,
    ShotSmokeX,
    ShotIlluminating,
    ShotCm,
    ShotMine,
    ShotTimeBomb,
    ShotDirectionalBomb,
    ShotBoundingMine,
    ShotDeploy,
    ShotSubmunitions,
    ShotLaser,
    ShotNvgMarker,
    LaserDesignate,
    // Non-AI objects (CfgNonAIVehicles)
    Detector,
    Camera,
    SeaGull,
    StreetLamp,
    WindSock,
    RopeSegment,
    Road,
    Proxy,
    Plain,
}

/// `simulation` value (lower case) → class. Pairs confirmed in the factory are marked in the RE
/// doc; the rest are by name (issue #117).
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
    ("vasi", SimulationClass::Vasi),
    ("thing", SimulationClass::Thing),
    ("thingx", SimulationClass::ThingX),
    ("thingeffect", SimulationClass::ThingEffect),
    ("breakablehousepart", SimulationClass::BreakableHousePart),
    (
        "breakablehouseanimatedpart",
        SimulationClass::BreakableHousePart,
    ),
    ("rope", SimulationClass::Rope),
    ("lasertarget", SimulationClass::LaserTarget),
    ("nvmarker", SimulationClass::NvMarker),
    ("artillerymarker", SimulationClass::ArtilleryMarker),
    ("suppresstarget", SimulationClass::SuppressTarget),
    ("shotbullet", SimulationClass::ShotBullet),
    ("shotshell", SimulationClass::ShotShell),
    ("shotspread", SimulationClass::ShotSpread),
    ("shotmissile", SimulationClass::ShotMissile),
    ("shotrocket", SimulationClass::ShotRocket),
    ("shotgrenade", SimulationClass::ShotGrenade),
    ("shotsmoke", SimulationClass::ShotSmoke),
    ("shotsmokex", SimulationClass::ShotSmokeX),
    ("shotilluminating", SimulationClass::ShotIlluminating),
    ("shotcm", SimulationClass::ShotCm),
    ("shotmine", SimulationClass::ShotMine),
    ("shottimebomb", SimulationClass::ShotTimeBomb),
    ("shotdirectionalbomb", SimulationClass::ShotDirectionalBomb),
    ("shotboundingmine", SimulationClass::ShotBoundingMine),
    ("shotdeploy", SimulationClass::ShotDeploy),
    ("shotsubmunitions", SimulationClass::ShotSubmunitions),
    ("shotlaser", SimulationClass::ShotLaser),
    ("shotnvgmarker", SimulationClass::ShotNvgMarker),
    ("laserdesignate", SimulationClass::LaserDesignate),
    ("detector", SimulationClass::Detector),
    ("camera", SimulationClass::Camera),
    ("camconstruct", SimulationClass::Camera),
    ("camcurator", SimulationClass::Camera),
    ("editcursor", SimulationClass::Camera),
    ("objview", SimulationClass::Camera),
    ("seagull", SimulationClass::SeaGull),
    ("streetlamp", SimulationClass::StreetLamp),
    ("windsock", SimulationClass::WindSock),
    ("ropesegment", SimulationClass::RopeSegment),
    ("road", SimulationClass::Road),
    ("alwayshide", SimulationClass::Proxy),
    ("alwaysshow", SimulationClass::Proxy),
    ("flag", SimulationClass::Proxy),
    ("magazine", SimulationClass::Proxy),
    ("maverickweapon", SimulationClass::Proxy),
    ("pylonpod", SimulationClass::Proxy),
    ("proxycrew", SimulationClass::Proxy),
    ("proxyheadgear", SimulationClass::Proxy),
    ("proxyinventoryold", SimulationClass::Proxy),
    ("proxyradio", SimulationClass::Proxy),
    ("proxyretex", SimulationClass::Proxy),
    ("proxysecweapon", SimulationClass::Proxy),
    ("proxysubpart", SimulationClass::Proxy),
    ("proxyweapon", SimulationClass::Proxy),
    ("proxyhandgun", SimulationClass::Proxy),
    ("randomshape", SimulationClass::Plain),
    ("temp", SimulationClass::Plain),
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
            S::Vasi => EntityClass::Vasi,
            S::Thing | S::ThingX | S::ThingEffect | S::BreakableHousePart => EntityClass::Thing,
            S::Rope => EntityClass::Rope,
            S::LaserTarget | S::NvMarker | S::ArtilleryMarker | S::SuppressTarget => {
                EntityClass::Target
            }
            S::ShotBullet
            | S::ShotShell
            | S::ShotSpread
            | S::ShotMissile
            | S::ShotRocket
            | S::ShotGrenade
            | S::ShotSmoke
            | S::ShotSmokeX
            | S::ShotIlluminating
            | S::ShotCm
            | S::ShotMine
            | S::ShotTimeBomb
            | S::ShotDirectionalBomb
            | S::ShotBoundingMine
            | S::ShotDeploy
            | S::ShotSubmunitions
            | S::ShotLaser
            | S::ShotNvgMarker
            | S::LaserDesignate => EntityClass::Shot,
            S::Detector => EntityClass::Detector,
            S::Camera | S::SeaGull => EntityClass::Camera,
            S::StreetLamp => EntityClass::StreetLamp,
            S::WindSock => EntityClass::WindSock,
            S::RopeSegment => EntityClass::RopeSegment,
            S::Road | S::Proxy | S::Plain => EntityClass::Object,
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
