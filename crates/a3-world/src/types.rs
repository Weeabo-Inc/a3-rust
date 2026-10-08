//! Entity types: what a config class (CfgVehicles, CfgAmmo, CfgNonAIVehicles) says about the
//! Entities created from it. The original builds one `EntityType` subclass per class, chosen by
//! its `simulation` value (`docs/re/world-object-model.md`).

use std::collections::HashMap;
use std::sync::Arc;

use a3_config::{ConfigRef, ConfigTree, NodeId};

use crate::{Error, SimulationClass};

/// The step an Entity simulates at unless its class changes it: the original's `Entity`
/// constructor sets 1/15 s (`+0x1bc`); classes adjust it at run time.
pub const DEFAULT_SIMULATION_STEP: f32 = 1.0 / 15.0;

/// Which config root a type comes from. Searched in this order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TypeSource {
    /// `CfgVehicles`.
    Vehicles,
    /// `CfgAmmo`.
    Ammo,
    /// `CfgNonAIVehicles`.
    NonAiVehicles,
}

impl TypeSource {
    pub const ALL: [TypeSource; 3] = [
        TypeSource::Vehicles,
        TypeSource::Ammo,
        TypeSource::NonAiVehicles,
    ];

    /// The config class name of the root.
    pub fn root_name(self) -> &'static str {
        match self {
            TypeSource::Vehicles => "CfgVehicles",
            TypeSource::Ammo => "CfgAmmo",
            TypeSource::NonAiVehicles => "CfgNonAIVehicles",
        }
    }
}

/// The config `scope`: who may create the type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Scope {
    /// 0: an abstract base; the engine refuses to create it.
    Private,
    /// 1: creatable by scripts, hidden in editors.
    Protected,
    /// 2: public.
    Public,
}

impl Scope {
    fn from_number(n: f32) -> Scope {
        if n >= 2.0 {
            Scope::Public
        } else if n >= 1.0 {
            Scope::Protected
        } else {
            Scope::Private
        }
    }
}

/// One Entity type.
#[derive(Debug, Clone, PartialEq)]
pub struct EntityType {
    name: String,
    source: TypeSource,
    simulation: String,
    class: SimulationClass,
    model: String,
    scope: Scope,
    side: i32,
    display_name: String,
    simulation_step: f32,
    config_path: Vec<NodeId>,
}

impl EntityType {
    /// A type with defaults and no config behind it, for tests and engine-made objects.
    pub fn new(name: impl Into<String>, class: SimulationClass) -> Self {
        Self {
            name: name.into(),
            source: TypeSource::Vehicles,
            simulation: String::new(),
            class,
            model: String::new(),
            scope: Scope::Public,
            side: 3,
            display_name: String::new(),
            simulation_step: DEFAULT_SIMULATION_STEP,
            config_path: Vec::new(),
        }
    }

    fn from_config(source: TypeSource, cfg: &ConfigRef<'_>) -> Result<Self, Error> {
        let simulation = cfg.get("simulation").text().to_ascii_lowercase();
        let class = SimulationClass::from_simulation(&simulation).ok_or_else(|| {
            Error::UnknownSimulation {
                type_name: cfg.name().to_owned(),
                simulation: simulation.clone(),
            }
        })?;
        let step = cfg.get("simulationStep");
        let simulation_step =
            if source == TypeSource::Ammo && step.is_number() && step.number() > 0.0 {
                step.number()
            } else {
                DEFAULT_SIMULATION_STEP
            };
        Ok(Self {
            name: cfg.name().to_owned(),
            source,
            simulation,
            class,
            model: cfg.get("model").text(),
            scope: Scope::from_number(cfg.get("scope").number()),
            side: cfg.get("side").number() as i32,
            display_name: cfg.get("displayName").text(),
            simulation_step,
            config_path: cfg.node_path().to_vec(),
        })
    }

    /// The config class name in its original case.
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn source(&self) -> TypeSource {
        self.source
    }

    /// The `simulation` value, lower case.
    pub fn simulation(&self) -> &str {
        &self.simulation
    }

    pub fn class(&self) -> SimulationClass {
        self.class
    }

    /// The `model` path as written in config.
    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn scope(&self) -> Scope {
        self.scope
    }

    /// The `side` number (0 east, 1 west, 2 independent, 3 civilian, ...).
    pub fn side(&self) -> i32 {
        self.side
    }

    /// `displayName`, not localised.
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    /// The initial simulation step in seconds: `simulationStep` for ammo, otherwise
    /// [`DEFAULT_SIMULATION_STEP`].
    pub fn simulation_step(&self) -> f32 {
        self.simulation_step
    }

    /// The type's config class, for the parameters a family module reads itself. Null for types
    /// made with [`EntityType::new`].
    pub fn config<'a>(&self, tree: &'a ConfigTree) -> ConfigRef<'a> {
        tree.from_node_path(&self.config_path)
    }
}

/// Builds and caches [`EntityType`]s from the merged config, one per class name.
#[derive(Debug)]
pub struct TypeBank {
    config: Arc<ConfigTree>,
    cache: HashMap<String, Arc<EntityType>>,
}

impl TypeBank {
    pub fn new(config: Arc<ConfigTree>) -> Self {
        Self {
            config,
            cache: HashMap::new(),
        }
    }

    /// The merged config the types are read from.
    pub fn config(&self) -> &ConfigTree {
        &self.config
    }

    /// The type of config class `name` (case-insensitive), looked up in CfgVehicles, then
    /// CfgAmmo, then CfgNonAIVehicles.
    pub fn get(&mut self, name: &str) -> Result<Arc<EntityType>, Error> {
        let key = name.to_ascii_lowercase();
        if let Some(ty) = self.cache.get(&key) {
            return Ok(ty.clone());
        }
        let root = self.config.root();
        let (source, cfg) = TypeSource::ALL
            .iter()
            .map(|&s| (s, root.get(s.root_name()).get(name)))
            .find(|(_, cfg)| cfg.is_class())
            .ok_or_else(|| Error::UnknownType(name.to_owned()))?;
        let ty = Arc::new(EntityType::from_config(source, &cfg)?);
        self.cache.insert(key, ty.clone());
        Ok(ty)
    }
}
