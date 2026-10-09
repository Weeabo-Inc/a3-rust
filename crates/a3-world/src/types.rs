//! Entity types: what a config class (CfgVehicles, CfgAmmo, CfgNonAIVehicles) says about the
//! Entities created from it. The original builds one `EntityType` subclass per class, chosen by
//! its `simulation` value (`docs/re/world-object-model.md`).

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use a3_config::{ConfigRef, ConfigTree, NodeId};

use crate::{DamageModel, Error, SimulationClass};

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
    damage: DamageModel,
    ai: AiParams,
}

/// What the AI reads from a type's config (`EntityAIType` in the engine; `docs/re/ai.md`).
/// Missing keys keep the values the shipped config gives the type's family; a type without
/// config takes them all from its family.
#[derive(Debug, Clone, PartialEq)]
pub struct AiParams {
    /// `formationX`: the formation unit sideways, in metres (Man 5).
    pub formation_x: f64,
    /// `formationZ`: the formation unit along the formation direction, in metres (Man 5).
    pub formation_z: f64,
    /// `formationTime`: seconds of the unit ahead's trail a follower keeps behind (Man 5).
    pub formation_time: f64,
    /// `precision`: how close counts as there, in metres (Man 1).
    pub precision: f64,
    /// `maxSpeed`, in metres per second (config km/h; Man 24 km/h).
    pub max_speed: f64,
    /// `limitedSpeedCoef`: the LIMITED speed mode's share of `maxSpeed` (0.22).
    pub limited_speed_coef: f64,
    /// `sensitivity`: how well its eyes spot (Man 1.75).
    pub sensitivity: f64,
    /// `sensitivityEar`: how well its ears hear (Man 0.125).
    pub sensitivity_ear: f64,
    /// `camouflage`: how visible it is (Man 2, `B_Soldier_base_F` 1.4).
    pub camouflage: f64,
    /// `audible`: how loud it is (Man 0.05).
    pub audible: f64,
    /// `accuracy`: the knowledge above which an observer tells this class from its parent.
    pub accuracy: f64,
    /// `fsmFormation`: the unit's formation FSM: a CfgFSMs class, a `.fsm` path, or empty.
    pub fsm_formation: String,
    /// `fsmDanger`: the unit's danger FSM; `"-"` switches danger handling off.
    pub fsm_danger: String,
}

impl AiParams {
    /// The shipped values of `CfgVehicles >> All`.
    pub fn all() -> Self {
        Self {
            formation_x: 10.0,
            formation_z: 20.0,
            formation_time: 5.0,
            precision: 5.0,
            max_speed: 80.0 / 3.6,
            limited_speed_coef: 0.22,
            sensitivity: 2.5,
            sensitivity_ear: 0.0075,
            camouflage: 2.0,
            audible: 1.0,
            accuracy: 0.0,
            fsm_formation: String::new(),
            fsm_danger: String::new(),
        }
    }

    /// The shipped values of a rifleman: `CAManBase`, with `B_Soldier_base_F`'s camouflage and
    /// accuracy.
    pub fn soldier() -> Self {
        Self {
            formation_x: 5.0,
            formation_z: 5.0,
            formation_time: 5.0,
            precision: 1.0,
            max_speed: 24.0 / 3.6,
            limited_speed_coef: 0.22,
            sensitivity: 1.75,
            sensitivity_ear: 0.125,
            camouflage: 1.4,
            audible: 0.05,
            accuracy: 2.3,
            fsm_formation: "Formation".into(),
            fsm_danger: "-".into(),
        }
    }

    fn for_class(class: SimulationClass) -> Self {
        if class == SimulationClass::Soldier {
            Self::soldier()
        } else {
            Self::all()
        }
    }

    fn from_config(cfg: &ConfigRef<'_>, class: SimulationClass) -> Self {
        let d = Self::for_class(class);
        let number = |key: &str, default: f64| {
            let v = cfg.get(key);
            if v.is_number() {
                f64::from(v.number())
            } else {
                default
            }
        };
        let text = |key: &str, default: &str| {
            let v = cfg.get(key);
            if v.is_text() {
                v.text()
            } else {
                default.to_owned()
            }
        };
        Self {
            formation_x: number("formationX", d.formation_x),
            formation_z: number("formationZ", d.formation_z),
            formation_time: number("formationTime", d.formation_time),
            precision: number("precision", d.precision),
            max_speed: number("maxSpeed", d.max_speed * 3.6) / 3.6,
            limited_speed_coef: number("limitedSpeedCoef", d.limited_speed_coef),
            sensitivity: number("sensitivity", d.sensitivity),
            sensitivity_ear: number("sensitivityEar", d.sensitivity_ear),
            camouflage: number("camouflage", d.camouflage),
            audible: number("audible", d.audible),
            accuracy: number("accuracy", d.accuracy),
            fsm_formation: text("fsmFormation", &d.fsm_formation),
            fsm_danger: text("fsmDanger", &d.fsm_danger),
        }
    }
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
            damage: DamageModel::new(),
            ai: AiParams::for_class(class),
        }
    }

    /// The same type with another model path, for a type the engine makes up (a ruin built from
    /// a `DestructionEffects` entry).
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    pub(crate) fn from_config(source: TypeSource, cfg: &ConfigRef<'_>) -> Result<Self, Error> {
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
            damage: DamageModel::from_config(cfg, class),
            ai: AiParams::from_config(cfg, class),
        })
    }

    /// What the AI reads from the type's config.
    pub fn ai(&self) -> &AiParams {
        &self.ai
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

    /// What the type's config says about taking damage: hit points, armor and ruins.
    pub fn damage(&self) -> &DamageModel {
        &self.damage
    }

    /// The type's config class, for the parameters a family module reads itself. Null for types
    /// made with [`EntityType::new`].
    pub fn config<'a>(&self, tree: &'a ConfigTree) -> ConfigRef<'a> {
        tree.from_node_path(&self.config_path)
    }
}

/// A model path in the form the index compares: lower case, `\` separators, no leading
/// separator.
fn normalize_model(model: &str) -> String {
    model
        .replace('/', "\\")
        .trim_start_matches('\\')
        .to_ascii_lowercase()
}

/// Builds and caches [`EntityType`]s from the merged config, one per class name.
#[derive(Debug)]
pub struct TypeBank {
    config: Arc<ConfigTree>,
    cache: HashMap<String, Arc<EntityType>>,
    /// `CfgVehicles` classes by normalised model path, built on first use.
    by_model: Option<HashMap<String, String>>,
}

impl TypeBank {
    pub fn new(config: Arc<ConfigTree>) -> Self {
        Self {
            config,
            cache: HashMap::new(),
            by_model: None,
        }
    }

    /// The merged config the types are read from.
    pub fn config(&self) -> &ConfigTree {
        &self.config
    }

    /// The merged config, shared (for [`World::set_config`](crate::World::set_config)).
    pub fn config_arc(&self) -> Arc<ConfigTree> {
        self.config.clone()
    }

    /// Whether config class `name` is `base` or inherits from it (`isKindOf`), searching the
    /// same roots as [`get`](Self::get). Case-insensitive; unknown names are not kinds of anything.
    pub fn is_kind_of(&self, name: &str, base: &str) -> bool {
        let root = self.config.root();
        let Some(cfg) = TypeSource::ALL
            .iter()
            .map(|s| root.get(s.root_name()).get(name))
            .find(|c| c.is_class())
        else {
            return false;
        };
        cfg.name().eq_ignore_ascii_case(base)
            || cfg
                .bases()
                .iter()
                .any(|b| b.name().eq_ignore_ascii_case(base))
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

    /// The name of the config class whose `model` is this model path, or `None`.
    ///
    /// The original resolves a model path back to its class (that is how a `DestructionEffects`
    /// ruin `type`, a model path, finds its class, `docs/re/sim-damage.md` §7.2). Searched in the
    /// same order as [`get`](Self::get) (CfgVehicles, CfgAmmo, CfgNonAIVehicles), matching
    /// case-insensitively and ignoring `/` versus `\` and a leading separator; where several
    /// classes share a model the first in config order wins. The index is built on first use.
    pub fn class_of_model(&mut self, model: &str) -> Option<String> {
        if self.by_model.is_none() {
            let mut index = HashMap::new();
            for source in TypeSource::ALL {
                for entry in self.config.root().get(source.root_name()).entries() {
                    let model = entry.get("model").text();
                    if !model.is_empty() {
                        index
                            .entry(normalize_model(&model))
                            .or_insert_with(|| entry.name().to_owned());
                    }
                }
            }
            self.by_model = Some(index);
        }
        self.by_model
            .as_ref()
            .and_then(|index| index.get(&normalize_model(model)))
            .cloned()
    }

    /// The type of the class whose `model` is this model path, if the config knows it and its
    /// class has a known simulation.
    pub fn for_model(&mut self, model: &str) -> Option<Arc<EntityType>> {
        let name = self.class_of_model(model)?;
        self.get(&name).ok()
    }

    /// A [`ModelTypeResolver`] over the same config, for the host's one-line wiring:
    ///
    /// ```ignore
    /// world.set_model_type_resolver(Some(types.resolver()));
    /// ```
    ///
    /// A `World` owns no config, so the resolver is a second bank over the same `Arc<ConfigTree>`
    /// (its own cache; the config itself is shared, not copied).
    pub fn resolver(&self) -> Box<dyn ModelTypeResolver> {
        Box::new(Self::new(self.config.clone()))
    }
}

impl ModelTypeResolver for TypeBank {
    fn resolve(&mut self, model: &str) -> Option<Arc<EntityType>> {
        self.for_model(model)
    }
}

/// The model path → [`EntityType`] lookup a Static object's model is resolved with: a hit on a
/// Static object promotes it with the type of its config class (so a house gets its hit points,
/// its armor and its ruin), and a destroyed building's ruin `type` finds its class with it
/// (`docs/re/sim-damage.md` §7.2). `World` owns no config, so a host that does installs one with
/// [`World::set_model_type_resolver`](crate::World::set_model_type_resolver); without one an
/// object is promoted as the plain type named after its model.
///
/// [`TypeBank`] implements it (`TypeBank::for_model`).
pub trait ModelTypeResolver: fmt::Debug + Send {
    /// The type of the config class whose `model` is this path, or `None`.
    fn resolve(&mut self, model: &str) -> Option<Arc<EntityType>>;
}
