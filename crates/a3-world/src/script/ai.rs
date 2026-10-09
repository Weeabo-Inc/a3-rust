//! SQF AI commands: the waypoint queue (`addWaypoint`, `setWaypointType`, ...), the group's
//! modes (`setBehaviour`, `setCombatMode`, ...), the orders a mission gives one unit (`doMove`,
//! `doStop`, ...) and target knowledge (`knowsAbout`, `reveal`, `forgetTarget`) — #129.
//!
//! A waypoint crosses the script boundary in the engine's own form, an array `[group, index]`
//! with the index one-based: the shape `addWaypoint` returns and every `setWaypoint*` takes. The
//! engine's implicit waypoint 0 — the group's start, already completed — exists here as that
//! index alone: `waypoints` reports it, `setCurrentWaypoint` ignores it, deleting it does
//! nothing. The command set and the deviations are in `docs/re/ai.md`.
//!
//! Which machine runs a command does not matter yet: locality gates the AI itself in
//! [`World::perform_ai`], not the script layer, and what a script does is not replicated yet.

use a3_sqf::vm::Ctx;
use a3_sqf::{Registry, Type, TypeSet, Value};
use glam::DVec3;

use super::{
    ARR, BOOL, NOTHING, NUM, OBJ, STR, WorldHost, group_arg, group_value, numbers, object_arg,
    position_or_object, position_value,
};
use crate::{
    Behaviour, CombatMode, EntityId, Formation, GroupId, LoiterType, ObjectRef, SpeedMode,
    Waypoint, WaypointType, World,
};

const GRP: TypeSet = TypeSet::of(Type::Group);
const SIDE: TypeSet = TypeSet::of(Type::Side);

pub(super) fn register<H: WorldHost>(r: &mut Registry<H>) {
    // ---- The waypoint queue ----

    // AG EG. `group addWaypoint [center, radius, index, name]` -> the waypoint `[group, index]`.
    // The random placement inside `radius` needs a seeded source the engine does not have yet
    // (#129 follow-up): the waypoint lands on the center.
    r.binary("addWaypoint", GRP, ARR, ARR, |ctx, g, a| {
        Ok(add_waypoint(ctx, group_arg(ctx.host.world(), &g), &a))
    });
    r.binary("addWaypoint", OBJ, ARR, ARR, |ctx, g, a| {
        Ok(add_waypoint(ctx, group_of_arg(ctx.host.world(), &g), &a))
    });
    // AG EG. `deleteWaypoint [group, index]`; the queue re-indexes at once.
    r.unary("deleteWaypoint", ARR, NOTHING, |ctx, a| {
        let w = ctx.host.world();
        let Some(group) = group_arg(w, &array_at(&a, 0)) else {
            return Ok(Value::Nothing);
        };
        let Some(index) = array_at(&a, 1).as_number().filter(|i| *i >= 1.0) else {
            return Ok(Value::Nothing);
        };
        let _ = ctx
            .host
            .world_mut()
            .delete_waypoint(group, index as usize - 1);
        Ok(Value::Nothing)
    });
    // AL EG.
    for left in [GRP, OBJ] {
        r.binary("setCurrentWaypoint", left, ARR, NOTHING, |ctx, _g, wp| {
            if let Some((group, index)) = waypoint_arg(ctx.host.world(), &wp) {
                let _ = ctx.host.world_mut().set_current_waypoint(group, index);
            }
            Ok(Value::Nothing)
        });
    }
    r.unary("currentWaypoint", GRP, NUM, |ctx, g| {
        Ok(current_waypoint(ctx, &g))
    });
    r.unary("currentWaypoint", OBJ, NUM, |ctx, g| {
        Ok(current_waypoint(ctx, &g))
    });
    // The explicit waypoints, with the engine's implicit waypoint 0 in front of them.
    r.unary("waypoints", GRP, ARR, |ctx, g| Ok(waypoints(ctx, &g)));
    r.unary("waypoints", OBJ, ARR, |ctx, g| Ok(waypoints(ctx, &g)));
    // AL EG. Active waypoints are dropped: the queue becomes one MOVE waypoint.
    r.binary("move", GRP, ARR, NOTHING, |ctx, g, p| {
        move_group(ctx, group_arg(ctx.host.world(), &g), &p);
        Ok(Value::Nothing)
    });
    r.binary("move", OBJ, ARR, NOTHING, |ctx, g, p| {
        move_group(ctx, group_of_arg(ctx.host.world(), &g), &p);
        Ok(Value::Nothing)
    });
    r.binary("move", GRP, OBJ, NOTHING, |ctx, g, p| {
        move_group(ctx, group_arg(ctx.host.world(), &g), &p);
        Ok(Value::Nothing)
    });
    r.binary("move", OBJ, OBJ, NOTHING, |ctx, g, p| {
        move_group(ctx, group_of_arg(ctx.host.world(), &g), &p);
        Ok(Value::Nothing)
    });

    // ---- What a waypoint carries ----

    r.binary("setWaypointType", ARR, STR, NOTHING, |ctx, wp, t| {
        if let Some(t) = t.as_str().and_then(WaypointType::from_config_name) {
            edit_waypoint(ctx.host.world_mut(), &wp, |w| w.waypoint_type = t);
        }
        Ok(Value::Nothing)
    });
    r.unary("waypointType", ARR, STR, |ctx, wp| {
        let t = get_waypoint(ctx.host.world(), &wp).map_or("", |w| w.waypoint_type.name());
        Ok(Value::string(t))
    });
    for name in ["setWaypointPosition", "setWPPos"] {
        r.binary(name, ARR, ARR, NOTHING, |ctx, wp, a| {
            if let Some(position) = position_or_object(ctx.host.world(), &array_at(&a, 0)) {
                edit_waypoint(ctx.host.world_mut(), &wp, |w| w.position = position);
            }
            Ok(Value::Nothing)
        });
    }
    r.unary("waypointPosition", ARR, ARR, |ctx, wp| {
        let p = get_waypoint(ctx.host.world(), &wp).map_or(DVec3::ZERO, |w| w.position);
        Ok(position_value(p))
    });
    // "UNCHANGED" comes back None from the name lookup, which is what the waypoint stores.
    r.binary("setWaypointBehaviour", ARR, STR, NOTHING, |ctx, wp, s| {
        let behaviour = s.as_str().and_then(Behaviour::from_config_name);
        edit_waypoint(ctx.host.world_mut(), &wp, |w| w.behaviour = behaviour);
        Ok(Value::Nothing)
    });
    r.unary("waypointBehaviour", ARR, STR, |ctx, wp| {
        let s = get_waypoint(ctx.host.world(), &wp)
            .and_then(|w| w.behaviour)
            .map_or("", Behaviour::name);
        Ok(Value::string(s))
    });
    r.binary("setWaypointCombatMode", ARR, STR, NOTHING, |ctx, wp, s| {
        let mode = s.as_str().and_then(CombatMode::from_config_name);
        edit_waypoint(ctx.host.world_mut(), &wp, |w| w.combat_mode = mode);
        Ok(Value::Nothing)
    });
    r.unary("waypointCombatMode", ARR, STR, |ctx, wp| {
        let s = get_waypoint(ctx.host.world(), &wp)
            .and_then(|w| w.combat_mode)
            .map_or("", CombatMode::name);
        Ok(Value::string(s))
    });
    r.binary("setWaypointSpeed", ARR, STR, NOTHING, |ctx, wp, s| {
        let speed = s.as_str().and_then(SpeedMode::from_config_name);
        edit_waypoint(ctx.host.world_mut(), &wp, |w| w.speed_mode = speed);
        Ok(Value::Nothing)
    });
    r.unary("waypointSpeed", ARR, STR, |ctx, wp| {
        let s = get_waypoint(ctx.host.world(), &wp)
            .and_then(|w| w.speed_mode)
            .map_or("", SpeedMode::name);
        Ok(Value::string(s))
    });
    r.binary("setWaypointFormation", ARR, STR, NOTHING, |ctx, wp, s| {
        let formation = s.as_str().and_then(Formation::from_config_name);
        edit_waypoint(ctx.host.world_mut(), &wp, |w| w.formation = formation);
        Ok(Value::Nothing)
    });
    r.unary("waypointFormation", ARR, STR, |ctx, wp| {
        let s = get_waypoint(ctx.host.world(), &wp)
            .and_then(|w| w.formation)
            .map_or("", Formation::name);
        Ok(Value::string(s))
    });
    r.binary(
        "setWaypointCompletionRadius",
        ARR,
        NUM,
        NOTHING,
        |ctx, wp, n| {
            if let Some(n) = n.as_number() {
                edit_waypoint(ctx.host.world_mut(), &wp, |w| {
                    w.completion_radius = f64::from(n)
                });
            }
            Ok(Value::Nothing)
        },
    );
    r.unary("waypointCompletionRadius", ARR, NUM, |ctx, wp| {
        let n = get_waypoint(ctx.host.world(), &wp).map_or(0.0, |w| w.completion_radius);
        Ok(Value::Number(n as f32))
    });
    r.binary("setWaypointDescription", ARR, STR, NOTHING, |ctx, wp, s| {
        let text = s.as_str().unwrap_or_default().to_owned();
        edit_waypoint(ctx.host.world_mut(), &wp, |w| w.description = text);
        Ok(Value::Nothing)
    });
    r.unary("waypointDescription", ARR, STR, |ctx, wp| {
        let s = get_waypoint(ctx.host.world(), &wp).map_or("", |w| w.description.as_str());
        Ok(Value::string(s))
    });
    r.binary("setWaypointName", ARR, STR, NOTHING, |ctx, wp, s| {
        let text = s.as_str().unwrap_or_default().to_owned();
        edit_waypoint(ctx.host.world_mut(), &wp, |w| w.name = text);
        Ok(Value::Nothing)
    });
    r.unary("waypointName", ARR, STR, |ctx, wp| {
        let s = get_waypoint(ctx.host.world(), &wp).map_or("", |w| w.name.as_str());
        Ok(Value::string(s))
    });
    r.binary("setWaypointVisible", ARR, BOOL, NOTHING, |ctx, wp, b| {
        if let Some(b) = b.as_bool() {
            edit_waypoint(ctx.host.world_mut(), &wp, |w| w.visible = b);
        }
        Ok(Value::Nothing)
    });
    r.unary("waypointVisible", ARR, BOOL, |ctx, wp| {
        let b = get_waypoint(ctx.host.world(), &wp).is_some_and(|w| w.visible);
        Ok(Value::Bool(b))
    });
    // `setWaypointTimeout [min, mid, max]`; the group moves on after the middle one (no random
    // source yet, `docs/re/ai.md`).
    r.binary("setWaypointTimeout", ARR, ARR, NOTHING, |ctx, wp, a| {
        let Some(times) = numbers(&a) else {
            return Ok(Value::Nothing);
        };
        let mut timeout = [0.0; 3];
        for (slot, value) in timeout.iter_mut().zip(times) {
            *slot = value;
        }
        edit_waypoint(ctx.host.world_mut(), &wp, |w| w.timeout = timeout);
        Ok(Value::Nothing)
    });
    r.unary("waypointTimeout", ARR, ARR, |ctx, wp| {
        let t = get_waypoint(ctx.host.world(), &wp).map_or([0.0; 3], |w| w.timeout);
        Ok(Value::array(t.map(|v| Value::Number(v as f32))))
    });
    // Kept as the source text the mission gave; running it is the VM's job (follow-up).
    r.binary("setWaypointStatements", ARR, ARR, NOTHING, |ctx, wp, a| {
        let items = a.as_array().map(|x| x.borrow().clone()).unwrap_or_default();
        let statements = items
            .chunks(2)
            .map(|pair| {
                let condition = pair.first().and_then(Value::as_str).unwrap_or_default();
                let statement = pair.get(1).and_then(Value::as_str).unwrap_or_default();
                (condition.to_owned(), statement.to_owned())
            })
            .collect();
        edit_waypoint(ctx.host.world_mut(), &wp, |w| w.statements = statements);
        Ok(Value::Nothing)
    });
    r.unary("waypointStatements", ARR, ARR, |ctx, wp| {
        let statements = get_waypoint(ctx.host.world(), &wp)
            .map(|w| w.statements.clone())
            .unwrap_or_default();
        Ok(Value::array(statements.into_iter().flat_map(
            |(condition, statement)| [Value::string(condition), Value::string(statement)],
        )))
    });
    r.binary("setWaypointScript", ARR, STR, NOTHING, |ctx, wp, s| {
        let text = s.as_str().unwrap_or_default().to_owned();
        edit_waypoint(ctx.host.world_mut(), &wp, |w| w.script = text);
        Ok(Value::Nothing)
    });
    r.unary("waypointScript", ARR, STR, |ctx, wp| {
        let s = get_waypoint(ctx.host.world(), &wp).map_or("", |w| w.script.as_str());
        Ok(Value::string(s))
    });
    r.binary(
        "setWaypointHousePosition",
        ARR,
        NUM,
        NOTHING,
        |ctx, wp, n| {
            if let Some(n) = n.as_number() {
                edit_waypoint(ctx.host.world_mut(), &wp, |w| {
                    w.house_position = n.round() as i64
                });
            }
            Ok(Value::Nothing)
        },
    );
    r.unary("waypointHousePosition", ARR, NUM, |ctx, wp| {
        let n = get_waypoint(ctx.host.world(), &wp).map_or(0, |w| w.house_position);
        Ok(Value::Number(n as f32))
    });
    r.binary(
        "setWaypointLoiterRadius",
        ARR,
        NUM,
        NOTHING,
        |ctx, wp, n| {
            if let Some(n) = n.as_number() {
                edit_waypoint(ctx.host.world_mut(), &wp, |w| {
                    w.loiter_radius = f64::from(n)
                });
            }
            Ok(Value::Nothing)
        },
    );
    r.unary("waypointLoiterRadius", ARR, NUM, |ctx, wp| {
        let n = get_waypoint(ctx.host.world(), &wp).map_or(0.0, |w| w.loiter_radius);
        Ok(Value::Number(n as f32))
    });
    r.binary("setWaypointLoiterType", ARR, STR, NOTHING, |ctx, wp, s| {
        if let Some(t) = s.as_str().and_then(LoiterType::from_config_name) {
            edit_waypoint(ctx.host.world_mut(), &wp, |w| w.loiter_type = t);
        }
        Ok(Value::Nothing)
    });
    r.unary("waypointLoiterType", ARR, STR, |ctx, wp| {
        let t = get_waypoint(ctx.host.world(), &wp).map_or(LoiterType::Linear, |w| w.loiter_type);
        Ok(Value::string(t.name()))
    });

    // ---- The group's modes ----

    for left in [GRP, OBJ] {
        r.binary("setBehaviour", left, STR, NOTHING, |ctx, g, s| {
            let group = group_of_arg(ctx.host.world(), &g);
            if let (Some(group), Some(behaviour)) =
                (group, s.as_str().and_then(Behaviour::from_config_name))
            {
                let _ = ctx.host.world_mut().set_group_behaviour(group, behaviour);
            }
            Ok(Value::Nothing)
        });
        r.unary("behaviour", left, STR, |ctx, g| {
            let s = group_of_arg(ctx.host.world(), &g)
                .and_then(|g| ctx.host.world().group_behaviour(g))
                .map_or("", Behaviour::name);
            Ok(Value::string(s))
        });
        r.binary("setCombatMode", left, STR, NOTHING, |ctx, g, s| {
            let group = group_of_arg(ctx.host.world(), &g);
            if let (Some(group), Some(mode)) =
                (group, s.as_str().and_then(CombatMode::from_config_name))
            {
                let _ = ctx.host.world_mut().set_group_combat_mode(group, mode);
            }
            Ok(Value::Nothing)
        });
        r.unary("combatMode", left, STR, |ctx, g| {
            let s = group_of_arg(ctx.host.world(), &g)
                .and_then(|g| ctx.host.world().group_combat_mode(g))
                .map_or("", CombatMode::name);
            Ok(Value::string(s))
        });
        r.binary("setSpeedMode", left, STR, NOTHING, |ctx, g, s| {
            let group = group_of_arg(ctx.host.world(), &g);
            if let (Some(group), Some(speed)) =
                (group, s.as_str().and_then(SpeedMode::from_config_name))
            {
                let _ = ctx.host.world_mut().set_group_speed_mode(group, speed);
            }
            Ok(Value::Nothing)
        });
        r.unary("speedMode", left, STR, |ctx, g| {
            let s = group_of_arg(ctx.host.world(), &g)
                .and_then(|g| ctx.host.world().group_speed_mode(g))
                .map_or("", SpeedMode::name);
            Ok(Value::string(s))
        });
        r.binary("setFormation", left, STR, NOTHING, |ctx, g, s| {
            let group = group_of_arg(ctx.host.world(), &g);
            if let (Some(group), Some(formation)) =
                (group, s.as_str().and_then(Formation::from_config_name))
            {
                let _ = ctx.host.world_mut().set_group_formation(group, formation);
            }
            Ok(Value::Nothing)
        });
        r.unary("formation", left, STR, |ctx, g| {
            let s = group_of_arg(ctx.host.world(), &g)
                .and_then(|g| ctx.host.world().group_formation(g))
                .map_or("", Formation::name);
            Ok(Value::string(s))
        });
    }

    // ---- Orders on one unit ----

    // AL EG. `unit(s) doMove position` and its radio-message twin.
    for name in ["doMove", "commandMove", "moveTo"] {
        for left in [OBJ, ARR] {
            r.binary(name, left, ARR, NOTHING, |ctx, u, p| {
                order_move(ctx, &u, &p);
                Ok(Value::Nothing)
            });
            r.binary(name, left, OBJ, NOTHING, |ctx, u, p| {
                order_move(ctx, &u, &p);
                Ok(Value::Nothing)
            });
        }
    }
    // AG EG. `doStop unit(s)`: out of the formation, where he stands.
    for left in [OBJ, ARR] {
        r.unary("doStop", left, NOTHING, |ctx, u| {
            let w = ctx.host.world();
            let units = units_arg(w, &u);
            let w = ctx.host.world_mut();
            for unit in units {
                w.stop_unit(unit);
            }
            Ok(Value::Nothing)
        });
    }
    r.unary("stopped", OBJ, BOOL, |ctx, u| {
        let w = ctx.host.world();
        let stopped = unit_arg(w, &u).is_some_and(|u| w.unit_stopped(u));
        Ok(Value::Bool(stopped))
    });
    // AL EG. `unit(s) doFollow unitLead`: back into the formation.
    for left in [OBJ, ARR] {
        r.binary("doFollow", left, OBJ, NOTHING, |ctx, u, _lead| {
            let w = ctx.host.world();
            let units = units_arg(w, &u);
            let w = ctx.host.world_mut();
            for unit in units {
                w.follow_unit(unit);
            }
            Ok(Value::Nothing)
        });
    }

    // ---- Target knowledge ----

    r.binary("knowsAbout", OBJ, OBJ, NUM, |ctx, who, target| {
        let w = ctx.host.world();
        let known = unit_arg(w, &who)
            .zip(unit_arg(w, &target))
            .map_or(0.0, |(who, target)| w.knows_about(who, target));
        Ok(Value::Number(known as f32))
    });
    r.binary("knowsAbout", GRP, OBJ, NUM, |ctx, who, target| {
        let w = ctx.host.world();
        let known = group_arg(w, &who)
            .zip(unit_arg(w, &target))
            .map_or(0.0, |(group, target)| w.knows_about_group(group, target));
        Ok(Value::Number(known as f32))
    });
    // A side knows what any of its groups knows.
    r.binary("knowsAbout", SIDE, OBJ, NUM, |ctx, who, target| {
        let Value::Side(side) = who else {
            return Ok(Value::Number(0.0));
        };
        let w = ctx.host.world();
        let Some(target) = unit_arg(w, &target) else {
            return Ok(Value::Number(0.0));
        };
        let known = w
            .all_groups()
            .filter(|g| g.side() == side)
            .map(|g| w.group_targets(g.id()).map_or(0.0, |t| t.knowledge(target)))
            .fold(0.0, f64::max);
        Ok(Value::Number(known as f32))
    });
    // AG EL. `toWhom reveal target` / `toWhom reveal [target, accuracy]`.
    for left in [GRP, OBJ] {
        r.binary("reveal", left, OBJ, NOTHING, |ctx, who, target| {
            reveal(ctx, &who, &target, None);
            Ok(Value::Nothing)
        });
        r.binary("reveal", left, ARR, NOTHING, |ctx, who, a| {
            let target = array_at(&a, 0);
            let accuracy = array_at(&a, 1).as_number().map(f64::from);
            reveal(ctx, &who, &target, accuracy);
            Ok(Value::Nothing)
        });
        r.binary("forgetTarget", left, OBJ, NOTHING, |ctx, who, target| {
            let w = ctx.host.world();
            let known = group_of_arg(w, &who).zip(unit_arg(w, &target));
            if let Some((group, target)) = known {
                ctx.host.world_mut().forget_target(group, target);
            }
            Ok(Value::Nothing)
        });
    }
}

// ---- The helpers the handlers share ----

/// The Entity an SQF Object value refers to.
fn unit_arg(world: &World, value: &Value) -> Option<EntityId> {
    match object_arg(world, value)? {
        ObjectRef::Entity(id) => Some(id),
        ObjectRef::Static(_) => None,
    }
}

/// The group an SQF value refers to: a Group, or a unit of one.
fn group_of_arg(world: &World, value: &Value) -> Option<GroupId> {
    group_arg(world, value).or_else(|| unit_arg(world, value).and_then(|u| world.group_of(u)))
}

/// The units an SQF value refers to: one unit, or an array of them.
fn units_arg(world: &World, value: &Value) -> Vec<EntityId> {
    let Some(items) = value.as_array() else {
        return unit_arg(world, value).into_iter().collect();
    };
    items
        .borrow()
        .iter()
        .filter_map(|v| unit_arg(world, v))
        .collect()
}

/// Element `index` of an SQF array value, or Nil.
fn array_at(value: &Value, index: usize) -> Value {
    value
        .as_array()
        .and_then(|a| a.borrow().get(index).cloned())
        .unwrap_or(Value::Nil)
}

/// The SQF value of a waypoint: `[group, index]`, the index one-based as the engine reports it.
fn waypoint_value(group: GroupId, index: usize) -> Value {
    Value::array([group_value(group), Value::Number(index as f32)])
}

/// The group and the queue index (zero-based) an SQF Waypoint array names. Index 0 — the
/// engine's implicit start waypoint — names no waypoint of ours, and neither does a missing one.
fn waypoint_arg(world: &World, value: &Value) -> Option<(GroupId, usize)> {
    let group = group_arg(world, &array_at(value, 0))?;
    let index = array_at(value, 1).as_number()?.floor();
    (index >= 1.0).then(|| (group, index as usize - 1))
}

/// The waypoint an SQF Waypoint array names, if it exists.
fn get_waypoint<'a>(world: &'a World, value: &Value) -> Option<&'a Waypoint> {
    let (group, index) = waypoint_arg(world, value)?;
    world.waypoint(group, index)
}

/// Changes the waypoint an SQF Waypoint array names, when it exists.
fn edit_waypoint(world: &mut World, value: &Value, edit: impl FnOnce(&mut Waypoint)) {
    let Some((group, index)) = waypoint_arg(world, value) else {
        return;
    };
    if let Some(waypoint) = world.waypoint_mut(group, index) {
        edit(waypoint);
    }
}

fn add_waypoint<H: WorldHost>(ctx: &mut Ctx<'_, H>, group: Option<GroupId>, args: &Value) -> Value {
    let Some(group) = group else {
        return Value::Nil;
    };
    let Some(position) = position_or_object(ctx.host.world(), &array_at(args, 0)) else {
        return Value::Nil;
    };
    let mut waypoint = Waypoint::new(WaypointType::Move, position);
    waypoint.name = array_at(args, 3).as_str().unwrap_or_default().to_owned();
    // `index` inserts before that waypoint; missing or invalid appends (the engine's -1).
    let index = array_at(args, 2).as_number().filter(|i| *i >= 1.0);
    let world = ctx.host.world_mut();
    let added = match index {
        Some(index) => world.insert_waypoint(group, index as usize - 1, waypoint),
        None => world.add_waypoint(group, waypoint),
    };
    match added {
        Ok(index) => waypoint_value(group, index + 1),
        Err(_) => Value::Nil,
    }
}

fn current_waypoint<H: WorldHost>(ctx: &mut Ctx<'_, H>, who: &Value) -> Value {
    let w = ctx.host.world();
    let current = group_of_arg(w, who).map_or(0, |g| {
        if w.waypoints(g).is_empty() {
            0
        } else {
            w.current_waypoint(g).unwrap_or(0) + 1
        }
    });
    Value::Number(current as f32)
}

fn waypoints<H: WorldHost>(ctx: &mut Ctx<'_, H>, who: &Value) -> Value {
    let w = ctx.host.world();
    let Some(group) = group_of_arg(w, who) else {
        return Value::array([]);
    };
    // The engine's implicit waypoint 0 comes first; the mission's waypoints follow it.
    let count = w.waypoints(group).len();
    Value::array((0..=count).map(|index| waypoint_value(group, index)))
}

fn move_group<H: WorldHost>(ctx: &mut Ctx<'_, H>, group: Option<GroupId>, position: &Value) {
    let Some(group) = group else {
        return;
    };
    let Some(position) = position_or_object(ctx.host.world(), position) else {
        return;
    };
    let _ = ctx.host.world_mut().move_group(group, position);
}

fn order_move<H: WorldHost>(ctx: &mut Ctx<'_, H>, who: &Value, position: &Value) {
    let Some(position) = position_or_object(ctx.host.world(), position) else {
        return;
    };
    let units = units_arg(ctx.host.world(), who);
    let world = ctx.host.world_mut();
    for unit in units {
        world.order_move(unit, position);
    }
}

fn reveal<H: WorldHost>(ctx: &mut Ctx<'_, H>, who: &Value, target: &Value, accuracy: Option<f64>) {
    let w = ctx.host.world();
    let known = group_of_arg(w, who).zip(unit_arg(w, target));
    if let Some((group, target)) = known {
        ctx.host.world_mut().reveal_group(group, target, accuracy);
    }
}
