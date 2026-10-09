//! SQF world commands (ADR 0005): the [`WorldHost`] trait, Object handle conversion and the
//! command registration.
//!
//! Commands follow the original's locality rules, noted per command as
//! `AG/AL` (argument global/local) and `EG/EL` (effect global/local):
//! - a command that needs a local argument does nothing when the Entity is remote;
//! - a server-only command does nothing on clients;
//! - global effects will also queue a network message once the network layer exists (#131).
//!
//! Positions cross the boundary with Y and Z swapped (ADR 0003): script `[x, y, z]` is world
//! `(x, z, y)`.

mod ai;
mod animation;
mod cargo_ops;
mod create;
mod groups;
mod handlers;
mod identity;
mod inventory;
mod markers;
mod object_animation;
mod object_misc;
mod object_state;
mod positions;
mod query;
mod state;
mod terrain;
mod transform;
mod weapons;

use a3_sqf::{Handle, HandleKind, Host, Registry, Type, TypeSet, Value};
use glam::DVec3;

pub use groups::{group_arg, group_value, null_group};
pub use handlers::{dispatch_events, raise_group_event, raise_mission_event, raise_object_event};
pub use terrain::terrain_normal;

use crate::{ClientId, ObjectRef, TypeBank, World};

/// The services world commands need from the engine that embeds the VM.
pub trait WorldHost: Host {
    fn world(&self) -> &World;
    fn world_mut(&mut self) -> &mut World;
    /// Entity types for `createVehicle` and `isKindOf`.
    fn types(&mut self) -> &mut TypeBank;
    /// `missionConfigFile` then `campaignConfigFile`, for lookups the original makes there
    /// before `configFile` (`CfgIdentities` for `setIdentity`). None by default.
    fn mission_configs(&self) -> Vec<std::sync::Arc<a3_config::ConfigTree>> {
        Vec::new()
    }
}

/// Registers every world command implemented so far.
pub fn register_world_commands<H: WorldHost>(r: &mut Registry<H>) {
    ai::register(r);
    animation::register(r);
    create::register(r);
    groups::register(r);
    handlers::register(r);
    markers::register(r);
    identity::register(r);
    inventory::register(r);
    object_state::register(r);
    object_misc::register(r);
    object_animation::register(r);
    cargo_ops::register(r);
    state::register(r);
    transform::register(r);
    positions::register(r);
    query::register(r);
    terrain::register(r);
    weapons::register(r);
}

pub(crate) const OBJ: TypeSet = TypeSet::of(Type::Object);
pub(crate) const NUM: TypeSet = TypeSet::NUMBER;
pub(crate) const STR: TypeSet = TypeSet::of(Type::String);
pub(crate) const BOOL: TypeSet = TypeSet::of(Type::Bool);
pub(crate) const ARR: TypeSet = TypeSet::of(Type::Array);
pub(crate) const NOTHING: TypeSet = TypeSet::of(Type::Nothing);

/// The SQF value for an Object. A promoted Static object keeps the handle of its Static key, so
/// a script sees the same value before and after promotion.
pub fn object_value(world: &World, object: ObjectRef) -> Value {
    let object = match object {
        ObjectRef::Entity(id) => world
            .entity(id)
            .and_then(|e| e.network_id())
            .and_then(crate::StaticKey::from_network_id)
            .map_or(object, ObjectRef::Static),
        ObjectRef::Static(_) => object,
    };
    Value::Handle(Handle {
        kind: HandleKind::Object,
        id: object.to_handle_id(),
    })
}

/// `objNull`.
pub fn null_object() -> Value {
    Value::Handle(Handle::null(HandleKind::Object))
}

/// The Object an SQF value refers to, if it is an Object handle that still exists. A Static
/// object that was promoted resolves to its Entity.
pub fn object_arg(world: &World, value: &Value) -> Option<ObjectRef> {
    match value {
        Value::Handle(h) if h.kind == HandleKind::Object => {
            let object = match ObjectRef::from_handle_id(h.id)? {
                ObjectRef::Static(key) => world.object_ref_of_static(key),
                entity => entity,
            };
            world.exists(object).then_some(object)
        }
        _ => None,
    }
}

/// [`Host::is_null`] for a world host: Object handles of deleted or removed Objects are null.
pub fn is_null_handle(world: &World, handle: Handle) -> bool {
    match handle.kind {
        HandleKind::Object => object_arg(world, &Value::Handle(handle)).is_none(),
        HandleKind::Group => group_arg(world, &Value::Handle(handle)).is_none(),
        _ => handle.is_null(),
    }
}

/// [`Host::format_handle`] for a world host, in the original's style: units in a group print
/// `"B Alpha 1-1:1"`, other Objects `"<address># <id>: <model>"`, groups `"B Alpha 1-1"`;
/// null and deleted ones print `<NULL-object>` / `<NULL-group>`.
pub fn format_handle(world: &World, handle: Handle) -> String {
    if is_null_handle(world, handle) {
        return handle.to_string();
    }
    if handle.kind == HandleKind::Group {
        return match group_arg(world, &Value::Handle(handle)) {
            Some(g) => groups::format_group(world, g),
            None => handle.to_string(),
        };
    }
    if handle.kind != HandleKind::Object {
        return handle.to_string();
    }
    let Some(object) = object_arg(world, &Value::Handle(handle)) else {
        return handle.to_string();
    };
    let (id, model) = match object {
        ObjectRef::Entity(id) => {
            // The original's `EntityAI` debug name: the vehicle variable name, else the unit.
            let ai = world
                .entity(id)
                .is_some_and(|e| e.class().is_kind_of(crate::EntityClass::EntityAi));
            if ai && !world.var_name(id).is_empty() {
                return world.var_name(id).to_owned();
            }
            if let Some(unit) = groups::format_unit(world, id) {
                return unit;
            }
            let e = world.entity(id).expect("checked");
            let model = e.entity_type().model();
            let file = model.rsplit(['\\', '/']).next().unwrap_or(model);
            let file = if file.is_empty() { e.type_name() } else { file };
            (-1i64, file.to_ascii_lowercase())
        }
        ObjectRef::Static(key) => {
            let o = world.static_object(key).expect("checked");
            let model = world
                .static_model(key)
                .and_then(|m| m.file_name().map(str::to_owned))
                .unwrap_or_default();
            (i64::from(o.object_id), model)
        }
    };
    format!("{:x}# {id}: {model}", handle.id)
}

/// Script `[x, y, z]` (east, north, height) to world space.
pub(crate) fn to_world(x: f64, y: f64, z: f64) -> DVec3 {
    DVec3::new(x, z, y)
}

/// World space to a script position array.
pub(crate) fn position_value(p: DVec3) -> Value {
    Value::array([p.x, p.z, p.y].map(|v| Value::Number(v as f32)))
}

/// World-space direction to a script vector.
pub(crate) fn vector_value(v: DVec3) -> Value {
    position_value(v)
}

/// Numbers of an array value.
pub(crate) fn numbers(value: &Value) -> Option<Vec<f64>> {
    let arr = value.as_array()?;
    arr.borrow()
        .iter()
        .map(|v| v.as_number().map(f64::from))
        .collect()
}

/// A script position `[x, y]` or `[x, y, z]` as world space, with `z` as given (no surface
/// conversion).
pub(crate) fn script_position(value: &Value) -> Option<DVec3> {
    match numbers(value)?.as_slice() {
        [x, y] => Some(to_world(*x, *y, 0.0)),
        [x, y, z, ..] => Some(to_world(*x, *y, *z)),
        _ => None,
    }
}

/// A position argument that may also be an Object (its position), as most queries accept.
/// Array positions are taken as above-terrain heights.
pub(crate) fn position_or_object(world: &World, value: &Value) -> Option<DVec3> {
    if let Some(r) = object_arg(world, value) {
        return world.object_position(r);
    }
    let mut p = script_position(value)?;
    p.y += world.surface_height(p.x, p.z);
    Some(p)
}

/// Whether this machine is the server (server-only commands do nothing elsewhere).
pub(crate) fn is_server(world: &World) -> bool {
    world.local_client() == ClientId::SERVER
}

/// A self-contained world host: a [`World`] and its [`TypeBank`], with `time`, `isNull` and
/// `str` wired to the World. For tools, tests and headless runs.
#[derive(Debug)]
pub struct ScriptWorld {
    pub world: World,
    pub types: TypeBank,
}

impl ScriptWorld {
    pub fn new(world: World, types: TypeBank) -> Self {
        Self { world, types }
    }
}

impl Host for ScriptWorld {
    fn time(&self) -> f32 {
        self.world.time() as f32
    }

    fn is_null(&self, handle: Handle) -> bool {
        is_null_handle(&self.world, handle)
    }

    fn format_handle(&self, handle: Handle) -> String {
        format_handle(&self.world, handle)
    }
}

impl WorldHost for ScriptWorld {
    fn world(&self) -> &World {
        &self.world
    }

    fn world_mut(&mut self) -> &mut World {
        &mut self.world
    }

    fn types(&mut self) -> &mut TypeBank {
        &mut self.types
    }
}
