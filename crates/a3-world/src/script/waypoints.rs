//! Group waypoint commands: `addWaypoint`, `waypoints`, `deleteWaypoint`, `copyWaypoints`,
//! `currentWaypoint` and the `waypoint*` / `setWaypoint*` families.
//!
//! The waypoint state lives in [`crate::waypoints`]. Scripts address a waypoint as the
//! two-element array `[group, index]` the engine hands out, so every accessor takes that array
//! and the command that creates one returns it.
//!
//! The accessors of a waypoint that does not exist read as empty (0, `""`, `[]`, `false`), as
//! the engine does; the setters do nothing.
//!
//! Driving a group along its waypoints is the AI's job (#129); these commands only maintain the
//! list. `move` (which orders a group to a position without a waypoint) belongs with that.

use a3_sqf::vm::Ctx;
use a3_sqf::{Registry, Value};
use glam::DVec3;

use super::groups::GRP;
use super::{
    ARR, BOOL, NOTHING, NUM, OBJ, STR, WorldHost, group_arg, group_value, numbers, object_arg,
    position_value, script_position,
};
use crate::waypoints::Waypoint;
use crate::{GroupId, ObjectRef, World};

/// The `[group, index]` value scripts use for a waypoint.
fn waypoint_value(group: GroupId, index: usize) -> Value {
    Value::array([group_value(group), Value::Number(index as f32)])
}

/// The elements of an array argument, or empty.
fn items(value: &Value) -> Vec<Value> {
    value
        .as_array()
        .map(|a| a.borrow().clone())
        .unwrap_or_default()
}

/// The group and index a waypoint argument (`[group, index]`) refers to. The index must be a
/// non-negative number; an out-of-range one still decodes and the accessors then read empty.
fn waypoint_arg(world: &World, value: &Value) -> Option<(GroupId, usize)> {
    let arr = value.as_array()?;
    let arr = arr.borrow();
    let group = group_arg(world, arr.first()?)?;
    let index = arr.get(1).and_then(Value::as_number)?;
    (index >= 0.0).then_some((group, index as usize))
}

/// The waypoint a `[group, index]` argument refers to, if it exists.
fn waypoint_of<'w, H: WorldHost>(ctx: &'w Ctx<'_, H>, value: &Value) -> Option<&'w Waypoint> {
    let (group, index) = waypoint_arg(ctx.host.world(), value)?;
    ctx.host.world().waypoint(group, index)
}

/// Runs `f` over the waypoint a `[group, index]` argument refers to.
fn with_waypoint<H: WorldHost>(ctx: &mut Ctx<'_, H>, value: &Value, f: impl FnOnce(&mut Waypoint)) {
    let world = ctx.host.world_mut();
    if let Some((group, index)) = waypoint_arg(world, value) {
        if let Some(point) = world.waypoint_mut(group, index) {
            f(point);
        }
    }
}

/// The group a group-or-unit argument names: a Group, or the group of a unit.
fn group_or_unit_arg(world: &World, value: &Value) -> Option<GroupId> {
    if let Some(group) = group_arg(world, value) {
        return Some(group);
    }
    match object_arg(world, value)? {
        ObjectRef::Entity(id) => world.group_of(id),
        ObjectRef::Static(_) => None,
    }
}

/// A waypoint position argument: a position array, or an Object (its position).
fn waypoint_position<H: WorldHost>(ctx: &Ctx<'_, H>, value: &Value) -> Option<DVec3> {
    if let Some(r) = object_arg(ctx.host.world(), value) {
        return ctx.host.world().object_position(r);
    }
    script_position(value)
}

/// Registers a `waypoint*` getter and its `setWaypoint*` setter over one string field.
macro_rules! waypoint_string_field {
    ($r:ident, $field:ident, $get:literal, $set:literal) => {
        $r.unary($get, ARR, STR, |ctx, a| {
            Ok(Value::from(
                waypoint_of(ctx, &a).map_or("", |w| w.$field.as_str()),
            ))
        });
        $r.binary($set, ARR, STR, NOTHING, |ctx, wp, v| {
            with_waypoint(ctx, &wp, |w| {
                if let Some(text) = v.as_str() {
                    w.$field = text.to_owned();
                }
            });
            Ok(Value::Nothing)
        });
    };
}

/// Registers the same pair for a numeric field.
macro_rules! waypoint_number_field {
    ($r:ident, $field:ident, $get:literal, $set:literal) => {
        $r.unary($get, ARR, NUM, |ctx, a| {
            Ok(Value::Number(
                waypoint_of(ctx, &a).map_or(0.0, |w| w.$field),
            ))
        });
        $r.binary($set, ARR, NUM, NOTHING, |ctx, wp, v| {
            with_waypoint(ctx, &wp, |w| {
                if let Some(number) = v.as_number() {
                    w.$field = number;
                }
            });
            Ok(Value::Nothing)
        });
    };
}

/// Registers the same pair for a boolean field.
macro_rules! waypoint_bool_field {
    ($r:ident, $field:ident, $get:literal, $set:literal) => {
        $r.unary($get, ARR, BOOL, |ctx, a| {
            Ok(Value::Bool(waypoint_of(ctx, &a).is_some_and(|w| w.$field)))
        });
        $r.binary($set, ARR, BOOL, NOTHING, |ctx, wp, v| {
            with_waypoint(ctx, &wp, |w| {
                if let Some(flag) = v.as_bool() {
                    w.$field = flag;
                }
            });
            Ok(Value::Nothing)
        });
    };
}

pub(super) fn register<H: WorldHost>(r: &mut Registry<H>) {
    // AG EG. `group addWaypoint [center, radius, index, name]`; returns the new waypoint.
    r.binary("addWaypoint", GRP, ARR, ARR, |ctx, g, a| {
        Ok(add_waypoint(ctx, &g, &a))
    });
    // AG EG. `deleteWaypoint [group, index]`.
    r.unary("deleteWaypoint", ARR, NOTHING, |ctx, a| {
        let world = ctx.host.world_mut();
        if let Some((group, index)) = waypoint_arg(world, &a) {
            world.delete_waypoint(group, index);
        }
        Ok(Value::Nothing)
    });
    // AG EG. `groupTo copyWaypoints groupFrom`.
    r.binary("copyWaypoints", GRP, GRP, NOTHING, |ctx, to, from| {
        let world = ctx.host.world_mut();
        if let (Some(to), Some(from)) = (group_arg(world, &to), group_arg(world, &from)) {
            world.copy_waypoints(from, to);
        }
        Ok(Value::Nothing)
    });

    // The waypoint list and the active index.
    r.unary("waypoints", GRP, ARR, |ctx, a| Ok(waypoints_value(ctx, &a)));
    r.unary("waypoints", OBJ, ARR, |ctx, a| Ok(waypoints_value(ctx, &a)));
    r.unary("currentWaypoint", GRP, NUM, |ctx, a| {
        let world = ctx.host.world();
        Ok(Value::Number(
            group_or_unit_arg(world, &a)
                .and_then(|g| world.current_waypoint(g))
                .map_or(0.0, |i| i as f32),
        ))
    });
    // AL EG. `group setCurrentWaypoint waypoint`.
    r.binary("setCurrentWaypoint", GRP, ARR, NOTHING, |ctx, g, w| {
        let world = ctx.host.world_mut();
        let (Some(group), Some((_, index))) = (group_arg(world, &g), waypoint_arg(world, &w))
        else {
            return Ok(Value::Nothing);
        };
        // Unlike an accessor, the engine ignores an index past the end.
        if index < world.waypoints(group).map_or(0, <[Waypoint]>::len) {
            world.set_current_waypoint(group, index);
        }
        Ok(Value::Nothing)
    });

    // Position: `waypointPosition`, its `getWPPos` alias and their setters.
    r.unary("waypointPosition", ARR, ARR, |ctx, a| {
        Ok(waypoint_pos(ctx, &a))
    });
    r.unary("getWPPos", ARR, ARR, |ctx, a| Ok(waypoint_pos(ctx, &a)));
    // `waypoint setWaypointPosition [center, radius]`; a negative radius asks for exact
    // placement, which is what this places at the centre anyway.
    r.binary("setWaypointPosition", ARR, ARR, NOTHING, |ctx, wp, a| {
        let args = items(&a);
        let position = args.first().and_then(|p| waypoint_position(ctx, p));
        let radius = args.get(1).and_then(Value::as_number);
        if let Some(position) = position {
            with_waypoint(ctx, &wp, |w| {
                w.position = position;
                if let Some(radius) = radius {
                    w.radius = radius;
                }
            });
        }
        Ok(Value::Nothing)
    });
    r.binary("setWPPos", ARR, ARR, NOTHING, |ctx, wp, p| {
        let position = waypoint_position(ctx, &p);
        if let Some(position) = position {
            with_waypoint(ctx, &wp, |w| w.position = position);
        }
        Ok(Value::Nothing)
    });

    // The string fields.
    waypoint_string_field!(r, waypoint_type, "waypointType", "setWaypointType");
    waypoint_string_field!(r, speed, "waypointSpeed", "setWaypointSpeed");
    waypoint_string_field!(
        r,
        combat_mode,
        "waypointCombatMode",
        "setWaypointCombatMode"
    );
    waypoint_string_field!(r, formation, "waypointFormation", "setWaypointFormation");
    waypoint_string_field!(r, behaviour, "waypointBehaviour", "setWaypointBehaviour");
    waypoint_string_field!(
        r,
        description,
        "waypointDescription",
        "setWaypointDescription"
    );
    waypoint_string_field!(r, name, "waypointName", "setWaypointName");
    waypoint_string_field!(r, script, "waypointScript", "setWaypointScript");
    waypoint_string_field!(
        r,
        loiter_type,
        "waypointLoiterType",
        "setWaypointLoiterType"
    );
    waypoint_string_field!(r, show, "waypointShow", "showWaypoint");

    // The numeric and boolean fields.
    waypoint_number_field!(
        r,
        completion_radius,
        "waypointCompletionRadius",
        "setWaypointCompletionRadius"
    );
    waypoint_number_field!(
        r,
        loiter_radius,
        "waypointLoiterRadius",
        "setWaypointLoiterRadius"
    );
    waypoint_number_field!(
        r,
        house_position,
        "waypointHousePosition",
        "setWaypointHousePosition"
    );
    waypoint_bool_field!(
        r,
        force_behaviour,
        "waypointForceBehaviour",
        "setWaypointForceBehaviour"
    );
    // `waypointVisible` reports a number (0 for an invalid waypoint), though its setter takes
    // a boolean.
    r.unary("waypointVisible", ARR, NUM, |ctx, a| {
        Ok(Value::Number(
            if waypoint_of(ctx, &a).is_some_and(|w| w.visible) {
                1.0
            } else {
                0.0
            },
        ))
    });
    r.binary("setWaypointVisible", ARR, BOOL, NOTHING, |ctx, wp, v| {
        with_waypoint(ctx, &wp, |w| {
            if let Some(flag) = v.as_bool() {
                w.visible = flag;
            }
        });
        Ok(Value::Nothing)
    });

    // `waypointStatements` / `setWaypointStatements [condition, statement]`.
    r.unary("waypointStatements", ARR, ARR, |ctx, a| {
        let statements = waypoint_of(ctx, &a).map(|w| w.statements.clone());
        Ok(match statements {
            Some([condition, statement]) => {
                Value::array([Value::from(condition), Value::from(statement)])
            }
            None => Value::array([]),
        })
    });
    r.binary("setWaypointStatements", ARR, ARR, NOTHING, |ctx, wp, v| {
        if let Some(statements) = statements_arg(&v) {
            with_waypoint(ctx, &wp, |w| w.statements = statements);
        }
        Ok(Value::Nothing)
    });

    // `waypointTimeout` / `setWaypointTimeout [min, mid, max]`.
    r.unary("waypointTimeout", ARR, ARR, |ctx, a| {
        let timeout = waypoint_of(ctx, &a).map(|w| w.timeout);
        Ok(match timeout {
            Some(t) => Value::array(t.map(Value::Number)),
            None => Value::array([]),
        })
    });
    r.binary("setWaypointTimeout", ARR, ARR, NOTHING, |ctx, wp, v| {
        if let Some(timeout) = timeout_arg(&v) {
            with_waypoint(ctx, &wp, |w| w.timeout = timeout);
        }
        Ok(Value::Nothing)
    });
}

/// `waypoints group`: every waypoint of the group (or of a unit's group).
fn waypoints_value<H: WorldHost>(ctx: &Ctx<'_, H>, value: &Value) -> Value {
    let world = ctx.host.world();
    let Some(group) = group_or_unit_arg(world, value) else {
        return Value::array([]);
    };
    let count = world.waypoints(group).map_or(0, <[Waypoint]>::len);
    Value::array((0..count).map(|i| waypoint_value(group, i)))
}

fn waypoint_pos<H: WorldHost>(ctx: &Ctx<'_, H>, value: &Value) -> Value {
    let position = waypoint_of(ctx, value).map_or(DVec3::ZERO, |w| w.position);
    position_value(position)
}

/// `addWaypoint [center, radius, index, name]`: inserts at `index`, or appends when it is
/// missing, negative or past the end, and returns the new waypoint.
fn add_waypoint<H: WorldHost>(ctx: &mut Ctx<'_, H>, group: &Value, args: &Value) -> Value {
    let args = items(args);
    let Some(group) = group_arg(ctx.host.world(), group) else {
        return Value::array([]);
    };
    let position = args
        .first()
        .and_then(|p| waypoint_position(ctx, p))
        .unwrap_or(DVec3::ZERO);
    let radius = args.get(1).and_then(Value::as_number).unwrap_or(0.0);
    let index = args
        .get(2)
        .and_then(Value::as_number)
        .filter(|n| *n >= 0.0)
        .map(|n| n as usize);
    let mut waypoint = Waypoint::new(position);
    waypoint.radius = radius;
    waypoint.name = args
        .get(3)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    match ctx.host.world_mut().add_waypoint(group, index, waypoint) {
        Some(index) => waypoint_value(group, index),
        None => Value::array([]),
    }
}

/// `[condition, statement]`; either half may be missing.
fn statements_arg(value: &Value) -> Option<[String; 2]> {
    let arr = value.as_array()?;
    let arr = arr.borrow();
    let text = |i: usize| arr.get(i).and_then(Value::as_str).unwrap_or("").to_owned();
    Some([text(0), text(1)])
}

/// `[min, mid, max]`; a missing middle or maximum repeats the middle (the engine's default).
fn timeout_arg(value: &Value) -> Option<[f32; 3]> {
    let n = numbers(value)?;
    let min = *n.first()? as f32;
    let mid = *n.get(1).unwrap_or(&n[0]) as f32;
    Some([min, mid, *n.get(2).unwrap_or(&(mid as f64)) as f32])
}
