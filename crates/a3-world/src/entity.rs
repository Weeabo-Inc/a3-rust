//! An Entity: identity, type, transform, simulation bookkeeping and class-specific state.

use std::sync::Arc;

use glam::{DQuat, DVec3};

use crate::sim::ClassState;
use crate::{ClientId, EntityClass, EntityId, EntityType, NetworkId, SimulationClass};

/// Whether this machine owns an Entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Locality {
    /// This machine simulates the Entity authoritatively and sends its updates.
    Local,
    /// Another machine owns it. The server always knows the owner; a client usually does not
    /// (the original's `owner` command returns 0 on clients).
    Remote { owner: Option<ClientId> },
}

impl Locality {
    pub fn is_local(self) -> bool {
        matches!(self, Locality::Local)
    }
}

/// The World list an Entity is simulated in, after the original's World containers
/// (`docs/re/world-object-model.md`, "World containers").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ListKind {
    /// AI-capable Entities: people, vehicles, buildings and things (`EntityAI`). The original's
    /// "vehicles".
    Vehicles = 0,
    /// Shots and other projectiles. The original's "fast vehicles", simulated in parallel jobs.
    Projectiles = 1,
    /// Every other Entity or Object created at run time (triggers, cameras, lamps, proxies).
    Slow = 2,
    /// Static objects promoted to Entities.
    Static = 3,
}

impl ListKind {
    pub(crate) const COUNT: usize = 4;
    pub const ALL: [ListKind; 4] = [
        ListKind::Vehicles,
        ListKind::Projectiles,
        ListKind::Slow,
        ListKind::Static,
    ];

    /// The list a created Entity of `class` goes to.
    pub fn for_class(class: SimulationClass) -> ListKind {
        if class.is_kind_of(EntityClass::Shot) {
            ListKind::Projectiles
        } else if class.is_kind_of(EntityClass::EntityAi) {
            ListKind::Vehicles
        } else {
            ListKind::Slow
        }
    }
}

/// Where an Entity was at its last two simulation steps, so the renderer can interpolate
/// between steps (the original keeps a history of visual states per Entity).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VisualState {
    pub previous_position: DVec3,
    pub previous_orientation: DQuat,
    /// Length of the last step in seconds.
    pub last_step: f64,
    /// Time since the last step in seconds.
    pub since_step: f64,
}

/// A simulated Object in the World.
#[derive(Debug, Clone, PartialEq)]
pub struct Entity {
    pub(crate) id: EntityId,
    pub(crate) network_id: Option<NetworkId>,
    pub(crate) locality: Locality,
    pub(crate) entity_type: Arc<EntityType>,
    pub(crate) list: ListKind,
    pub(crate) deleted: bool,
    // Transform and motion (world space, ADR 0003).
    pub(crate) position: DVec3,
    pub(crate) orientation: DQuat,
    pub(crate) velocity: DVec3,
    pub(crate) visual: VisualState,
    // Simulation bookkeeping (original: `+0x1bc` step, `+0x1c4` accumulator, `+0x1cd` flags).
    pub(crate) simulation_step: f64,
    pub(crate) accumulated: f64,
    pub(crate) simulation_enabled: bool,
    pub(crate) dynamically_frozen: bool,
    pub(crate) steps: u64,
    pub(crate) simulated_time: f64,
    pub(crate) class_state: ClassState,
    // Object state scripts see.
    pub(crate) damage: f32,
    pub(crate) hidden: bool,
    pub(crate) attachment: Option<Attachment>,
}

/// An `attachTo` link: the Entity follows `to` at `offset` in `to`'s model space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Attachment {
    pub to: EntityId,
    pub offset: DVec3,
}

impl Entity {
    pub(crate) fn new(
        id: EntityId,
        network_id: Option<NetworkId>,
        locality: Locality,
        entity_type: Arc<EntityType>,
        position: DVec3,
        list: ListKind,
    ) -> Self {
        let simulation_step = f64::from(entity_type.simulation_step());
        let class_state = ClassState::for_class(entity_type.class());
        Self {
            id,
            network_id,
            locality,
            entity_type,
            list,
            deleted: false,
            position,
            orientation: DQuat::IDENTITY,
            velocity: DVec3::ZERO,
            visual: VisualState {
                previous_position: position,
                previous_orientation: DQuat::IDENTITY,
                last_step: 0.0,
                since_step: 0.0,
            },
            simulation_step,
            accumulated: 0.0,
            simulation_enabled: true,
            dynamically_frozen: false,
            steps: 0,
            simulated_time: 0.0,
            class_state,
            damage: 0.0,
            hidden: false,
            attachment: None,
        }
    }

    pub fn id(&self) -> EntityId {
        self.id
    }

    /// `None` for Entities created local-only (`createVehicleLocal`).
    pub fn network_id(&self) -> Option<NetworkId> {
        self.network_id
    }

    pub fn locality(&self) -> Locality {
        self.locality
    }

    pub fn is_local(&self) -> bool {
        self.locality.is_local()
    }

    pub fn entity_type(&self) -> &Arc<EntityType> {
        &self.entity_type
    }

    /// The config class name (`typeOf`).
    pub fn type_name(&self) -> &str {
        self.entity_type.name()
    }

    pub fn class(&self) -> SimulationClass {
        self.entity_type.class()
    }

    /// The simulation list it lives in.
    pub fn list(&self) -> ListKind {
        self.list
    }

    /// Scheduled for deletion; it disappears at the end of the step.
    pub fn is_deleted(&self) -> bool {
        self.deleted
    }

    pub fn position(&self) -> DVec3 {
        self.position
    }

    pub fn set_position(&mut self, position: DVec3) {
        self.position = position;
    }

    pub fn orientation(&self) -> DQuat {
        self.orientation
    }

    pub fn set_orientation(&mut self, orientation: DQuat) {
        self.orientation = orientation;
    }

    /// World-space velocity in m/s.
    pub fn velocity(&self) -> DVec3 {
        self.velocity
    }

    pub fn set_velocity(&mut self, velocity: DVec3) {
        self.velocity = velocity;
    }

    /// The transform at the last two steps, for interpolated rendering.
    pub fn visual_state(&self) -> VisualState {
        self.visual
    }

    /// The position to draw: interpolated from the previous step's position towards the
    /// current one by the time elapsed since the last step.
    pub fn render_position(&self) -> DVec3 {
        let v = &self.visual;
        if v.last_step <= 0.0 {
            return self.position;
        }
        let t = (v.since_step / v.last_step).clamp(0.0, 1.0);
        v.previous_position.lerp(self.position, t)
    }

    /// Seconds between simulation steps (original default 1/15 s; projectiles use their
    /// `simulationStep`). Classes may change it at run time, as the original does.
    pub fn simulation_step(&self) -> f64 {
        self.simulation_step
    }

    pub fn set_simulation_step(&mut self, step: f64) {
        self.simulation_step = step;
    }

    /// `simulationEnabled`.
    pub fn simulation_enabled(&self) -> bool {
        self.simulation_enabled
    }

    /// `enableSimulation`.
    pub fn set_simulation_enabled(&mut self, enabled: bool) {
        self.simulation_enabled = enabled;
    }

    /// Frozen by the dynamic simulation system (far from players).
    pub fn dynamically_frozen(&self) -> bool {
        self.dynamically_frozen
    }

    pub fn set_dynamically_frozen(&mut self, frozen: bool) {
        self.dynamically_frozen = frozen;
    }

    /// Whether `World::simulate` steps this Entity at all.
    pub fn is_simulated(&self) -> bool {
        self.simulation_enabled && !self.dynamically_frozen && !self.deleted
    }

    /// Number of simulation steps run so far.
    pub fn steps(&self) -> u64 {
        self.steps
    }

    /// Total time covered by simulation steps so far, in seconds.
    pub fn simulated_time(&self) -> f64 {
        self.simulated_time
    }

    /// Total damage, 0 (intact) to 1 (destroyed). Hit points come with #128.
    pub fn damage(&self) -> f32 {
        self.damage
    }

    pub fn set_damage(&mut self, damage: f32) {
        self.damage = damage.clamp(0.0, 1.0);
    }

    /// `alive`: not destroyed and not scheduled for deletion.
    pub fn is_alive(&self) -> bool {
        self.damage < 1.0 && !self.deleted
    }

    /// `isObjectHidden`.
    pub fn is_hidden(&self) -> bool {
        self.hidden
    }

    /// `hideObject`.
    pub fn set_hidden(&mut self, hidden: bool) {
        self.hidden = hidden;
    }

    /// `attachedTo`.
    pub fn attachment(&self) -> Option<Attachment> {
        self.attachment
    }

    /// Heading in degrees clockwise from north (`getDir`).
    pub fn heading(&self) -> f64 {
        let dir = self.orientation * DVec3::Z;
        dir.x.atan2(dir.z).to_degrees().rem_euclid(360.0)
    }

    /// Sets the heading (`setDir`), keeping the Entity upright.
    pub fn set_heading(&mut self, degrees: f64) {
        self.orientation = DQuat::from_rotation_y(degrees.to_radians());
    }

    /// The class-specific state, owned by the family module that simulates it.
    pub fn class_state(&self) -> &ClassState {
        &self.class_state
    }

    pub fn class_state_mut(&mut self) -> &mut ClassState {
        &mut self.class_state
    }
}
