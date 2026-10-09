//! Groups and sides: `createGroup`, `createUnit`, `join`, `leader`, `units`, `side`,
//! `setFriend`, group locality and ids.

use a3_sqf::vm::Ctx;
use a3_sqf::{Handle, HandleKind, Registry, Type, TypeSet, Value};

use super::{
    ARR, BOOL, NOTHING, NUM, OBJ, STR, WorldHost, is_server, null_object, object_arg, object_value,
    script_position,
};
use crate::{ClientId, Create, EntityId, GroupId, Locality, NetworkId, ObjectRef, Side, World};

pub(crate) const GRP: TypeSet = TypeSet::of(Type::Group);
const SIDE: TypeSet = TypeSet::of(Type::Side);

/// The SQF value for a group.
pub fn group_value(id: GroupId) -> Value {
    Value::Handle(Handle {
        kind: HandleKind::Group,
        id: id.to_handle_id(),
    })
}

/// `grpNull`.
pub fn null_group() -> Value {
    Value::Handle(Handle::null(HandleKind::Group))
}

/// The group an SQF value refers to, if it is a Group handle that still exists.
pub fn group_arg(world: &World, value: &Value) -> Option<GroupId> {
    match value {
        Value::Handle(h) if h.kind == HandleKind::Group => {
            GroupId::from_handle_id(h.id).filter(|&g| world.group(g).is_some())
        }
        _ => None,
    }
}

/// The letter `str` puts before a group name.
pub(crate) fn side_letter(side: Side) -> &'static str {
    match side {
        Side::West => "B",
        Side::East => "O",
        Side::Independent => "I",
        Side::Civilian => "C",
        Side::Logic => "L",
        _ => "U",
    }
}

/// `str group`: side letter and name, `"B Alpha 1-1"`.
pub(crate) fn format_group(world: &World, id: GroupId) -> String {
    let g = world.group(id).expect("exists");
    format!("{} {}", side_letter(g.side()), g.name())
}

/// `str unit` for a unit in a group: `"B Alpha 1-1:2"` (position in the group, 1-based).
pub(crate) fn format_unit(world: &World, unit: EntityId) -> Option<String> {
    let g = world.group(world.group_of(unit)?)?;
    let n = g.units().iter().position(|&u| u == unit)? + 1;
    Some(format!("{}:{n}", format_group(world, g.id())))
}

fn unit_arg(world: &World, value: &Value) -> Option<EntityId> {
    match object_arg(world, value)? {
        ObjectRef::Entity(id) => Some(id),
        ObjectRef::Static(_) => None,
    }
}

fn unit_values(world: &World, units: impl IntoIterator<Item = EntityId>) -> Value {
    Value::array(
        units
            .into_iter()
            .map(|u| object_value(world, ObjectRef::Entity(u))),
    )
}

pub(super) fn register<H: WorldHost>(r: &mut Registry<H>) {
    // AL EG.
    r.unary("createGroup", SIDE, GRP, |ctx, a| {
        let Value::Side(side) = a else {
            return Ok(null_group());
        };
        Ok(group_value(ctx.host.world_mut().create_group(side, false)))
    });
    r.unary("createGroup", ARR, GRP, |ctx, a| {
        let items = a.as_array().map(|x| x.borrow().clone()).unwrap_or_default();
        let Some(Value::Side(side)) = items.first().cloned() else {
            return Ok(null_group());
        };
        let delete_when_empty = items.get(1).and_then(Value::as_bool).unwrap_or(false);
        Ok(group_value(
            ctx.host.world_mut().create_group(side, delete_when_empty),
        ))
    });
    // AG EG. `group createUnit [type, position, markers, placement, special]`.
    r.binary("createUnit", GRP, ARR, OBJ, |ctx, g, a| {
        let items = a.as_array().map(|x| x.borrow().clone()).unwrap_or_default();
        let get = |i: usize| items.get(i).cloned().unwrap_or(Value::Nil);
        let group = group_arg(ctx.host.world(), &g);
        Ok(create_unit(ctx, &get(0), &get(1), group))
    });
    // AG EG. `type createUnit [position, group, init, skill, rank]`: returns nothing. The init
    // code, skill and rank come with AI (#129).
    r.binary("createUnit", STR, ARR, NOTHING, |ctx, t, a| {
        let items = a.as_array().map(|x| x.borrow().clone()).unwrap_or_default();
        let get = |i: usize| items.get(i).cloned().unwrap_or(Value::Nil);
        let group = group_arg(ctx.host.world(), &get(1));
        create_unit(ctx, &t, &get(0), group);
        Ok(Value::Nothing)
    });

    // AG EG.
    for name in ["join", "joinSilent"] {
        r.binary(name, ARR, GRP, NOTHING, |ctx, a, g| {
            let target = group_arg(ctx.host.world(), &g);
            join(ctx, &a, target);
            Ok(Value::Nothing)
        });
        r.binary(name, ARR, OBJ, NOTHING, |ctx, a, u| {
            let w = ctx.host.world();
            let target = unit_arg(w, &u).and_then(|u| w.group_of(u));
            join(ctx, &a, target);
            Ok(Value::Nothing)
        });
    }

    r.unary("group", OBJ, GRP, |ctx, a| {
        let w = ctx.host.world();
        Ok(unit_arg(w, &a)
            .and_then(|u| w.group_of(u))
            .map_or_else(null_group, group_value))
    });
    r.unary("leader", GRP, OBJ, |ctx, a| {
        let w = ctx.host.world();
        Ok(group_arg(w, &a)
            .and_then(|g| w.group(g).expect("exists").leader())
            .map_or_else(null_object, |u| object_value(w, ObjectRef::Entity(u))))
    });
    r.unary("leader", OBJ, OBJ, |ctx, a| {
        let w = ctx.host.world();
        let Some(unit) = unit_arg(w, &a) else {
            return Ok(null_object());
        };
        // A unit without a group leads itself.
        let leader = w
            .group_of(unit)
            .and_then(|g| w.group(g).expect("exists").leader())
            .unwrap_or(unit);
        Ok(object_value(w, ObjectRef::Entity(leader)))
    });
    // AL EG.
    r.binary("selectLeader", GRP, OBJ, NOTHING, |ctx, g, u| {
        let w = ctx.host.world_mut();
        if let (Some(g), Some(u)) = (group_arg(w, &g), unit_arg(w, &u)) {
            let _ = w.set_leader(g, u);
        }
        Ok(Value::Nothing)
    });
    r.unary("units", GRP, ARR, |ctx, a| {
        let w = ctx.host.world();
        let units = group_arg(w, &a)
            .map(|g| w.group(g).expect("exists").units().to_vec())
            .unwrap_or_default();
        Ok(unit_values(w, units))
    });
    r.unary("units", OBJ, ARR, |ctx, a| {
        let w = ctx.host.world();
        let units = match unit_arg(w, &a) {
            Some(u) => match w.group_of(u) {
                Some(g) => w.group(g).expect("exists").units().to_vec(),
                None => vec![u],
            },
            None => Vec::new(),
        };
        Ok(unit_values(w, units))
    });
    r.unary("units", SIDE, ARR, |ctx, a| {
        let w = ctx.host.world();
        let Value::Side(side) = a else {
            return Ok(Value::array([]));
        };
        let units: Vec<EntityId> = w
            .all_groups()
            .filter(|g| g.side() == side)
            .flat_map(|g| g.units().to_vec())
            .collect();
        Ok(unit_values(w, units))
    });
    r.nular("allGroups", ARR, |ctx| {
        let w = ctx.host.world();
        Ok(Value::array(w.all_groups().map(|g| group_value(g.id()))))
    });
    // Alive units of every group.
    r.nular("allUnits", ARR, |ctx| {
        let w = ctx.host.world();
        let units: Vec<EntityId> = w
            .all_groups()
            .flat_map(|g| g.units().to_vec())
            .filter(|&u| w.entity(u).is_some_and(|e| e.is_alive()))
            .collect();
        Ok(unit_values(w, units))
    });
    r.unary("side", OBJ, SIDE, |ctx, a| {
        let w = ctx.host.world();
        let side = unit_arg(w, &a).and_then(|u| {
            // A captive unit counts as civilian (`setCaptive`); its group keeps its side.
            if w.captive(u) > 0 {
                Some(Side::Civilian)
            } else {
                w.object_side(u)
            }
        });
        Ok(Value::Side(side.unwrap_or(Side::Unknown)))
    });
    r.unary("side", GRP, SIDE, |ctx, a| {
        let w = ctx.host.world();
        Ok(Value::Side(group_arg(w, &a).map_or(Side::Unknown, |g| {
            w.group(g).expect("exists").side()
        })))
    });
    r.unary("groupId", GRP, STR, |ctx, a| {
        let w = ctx.host.world();
        Ok(Value::string(
            group_arg(w, &a).map_or("", |g| w.group(g).expect("exists").name()),
        ))
    });
    // AG EG.
    r.binary("setGroupId", GRP, ARR, NOTHING, |ctx, g, a| {
        let name = a.as_array().and_then(|x| {
            x.borrow()
                .first()
                .and_then(|v| v.as_str().map(str::to_owned))
        });
        let w = ctx.host.world_mut();
        if let (Some(g), Some(name)) = (group_arg(w, &g), name) {
            let _ = w.set_group_name(g, name);
        }
        Ok(Value::Nothing)
    });
    // AL EG. Only empty groups are deleted.
    r.unary("deleteGroup", GRP, NOTHING, |ctx, a| {
        let w = ctx.host.world_mut();
        if let Some(g) = group_arg(w, &a) {
            if w.group(g).expect("exists").is_local() {
                let _ = w.delete_group(g);
            }
        }
        Ok(Value::Nothing)
    });

    r.unary("local", GRP, BOOL, |ctx, a| {
        let w = ctx.host.world();
        Ok(Value::Bool(
            group_arg(w, &a).is_some_and(|g| w.group(g).expect("exists").is_local()),
        ))
    });
    // Server only: clients get 0.
    r.unary("groupOwner", GRP, NUM, |ctx, a| {
        let w = ctx.host.world();
        let owner = match group_arg(w, &a) {
            Some(g) if is_server(w) => match w.group(g).expect("exists").locality() {
                Locality::Local => ClientId::SERVER.0,
                Locality::Remote { owner } => owner.map_or(0, |c| c.0),
            },
            _ => 0,
        };
        Ok(Value::Number(owner as f32))
    });
    // Server only. Moves the group and its AI units; returns whether locality changed. The
    // message to the clients comes with the network layer (#131).
    r.binary("setGroupOwner", GRP, NUM, BOOL, |ctx, g, n| {
        let w = ctx.host.world_mut();
        let (Some(g), Some(n)) = (group_arg(w, &g), n.as_number()) else {
            return Ok(Value::Bool(false));
        };
        if !is_server(w) {
            return Ok(Value::Bool(false));
        }
        let client = ClientId(n as u32);
        let locality = if client == ClientId::SERVER {
            Locality::Local
        } else {
            Locality::Remote {
                owner: Some(client),
            }
        };
        if w.group(g).expect("exists").locality() == locality {
            return Ok(Value::Bool(false));
        }
        Ok(Value::Bool(w.set_group_locality(g, locality).is_ok()))
    });
    r.unary("netId", GRP, STR, |ctx, a| {
        let w = ctx.host.world();
        Ok(Value::string(group_arg(w, &a).map_or(String::new(), |g| {
            w.group(g).expect("exists").network_id().to_string()
        })))
    });
    r.unary("groupFromNetId", STR, GRP, |ctx, a| {
        let w = ctx.host.world();
        Ok(a.as_str()
            .and_then(|s| s.parse::<NetworkId>().ok())
            .and_then(|n| w.resolve_group(n))
            .map_or_else(null_group, group_value))
    });

    // Side relations. AG EG.
    r.binary("setFriend", SIDE, ARR, NOTHING, |ctx, a, b| {
        let items = b.as_array().map(|x| x.borrow().clone()).unwrap_or_default();
        if let (Value::Side(a), Some(Value::Side(b)), Some(v)) = (
            a,
            items.first().cloned(),
            items.get(1).and_then(Value::as_number),
        ) {
            ctx.host.world_mut().set_friendship(a, b, v);
        }
        Ok(Value::Nothing)
    });
    r.binary("getFriend", SIDE, SIDE, NUM, |ctx, a, b| {
        let (Value::Side(a), Value::Side(b)) = (a, b) else {
            return Ok(Value::Number(0.0));
        };
        Ok(Value::Number(ctx.host.world().friendship(a, b)))
    });
}

fn create_unit<H: WorldHost>(
    ctx: &mut Ctx<'_, H>,
    type_name: &Value,
    position: &Value,
    group: Option<GroupId>,
) -> Value {
    let (Some(name), Some(pos), Some(group)) =
        (type_name.as_str(), script_position(position), group)
    else {
        return null_object();
    };
    let Ok(ty) = ctx.host.types().get(name) else {
        return null_object();
    };
    // A unit is created with its class's weapons and magazines.
    super::weapons::ensure_config(ctx);
    let w = ctx.host.world_mut();
    let Ok(unit) = w.create(Create::new(ty, pos).on_surface()) else {
        return null_object();
    };
    let _ = w.join(unit, group);
    let object = object_value(w, ObjectRef::Entity(unit));
    super::handlers::dispatch_events_in(ctx);
    object
}

fn join<H: WorldHost>(ctx: &mut Ctx<'_, H>, units: &Value, group: Option<GroupId>) {
    let Some(group) = group else { return };
    let items = units
        .as_array()
        .map(|x| x.borrow().clone())
        .unwrap_or_default();
    let w = ctx.host.world_mut();
    for item in items {
        if let Some(unit) = unit_arg(w, &item) {
            let _ = w.join(unit, group);
        }
    }
}
