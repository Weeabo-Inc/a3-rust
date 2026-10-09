//! Map marker commands: `createMarker`, the `marker*` / `getMarker*` / `setMarker*` families
//! and `allMapMarkers`.
//!
//! The marker table itself lives in [`crate::markers`]. Markers are named, so the marker name is
//! the argument of every accessor.
//!
//! Locality: the plain commands are "effect global" — the original broadcasts the whole marker
//! state to every machine, and once the network layer exists (#131) they will queue a message.
//! The `...Local` variants are "effect local": this machine's marker table only, and they are
//! what the campaigns use in bulk. Both apply to the local table here; the difference is the
//! broadcast.

use a3_sqf::vm::Ctx;
use a3_sqf::{Registry, Value};
use glam::DVec3;

use super::{
    ARR, BOOL, NOTHING, NUM, OBJ, STR, WorldHost, numbers, object_arg, position_value,
    script_position,
};
use crate::markers::Marker;

/// A position argument: a position array, or an Object (its position).
fn marker_position<H: WorldHost>(ctx: &Ctx<'_, H>, value: &Value) -> Option<DVec3> {
    if let Some(r) = object_arg(ctx.host.world(), value) {
        return ctx.host.world().object_position(r);
    }
    script_position(value)
}

/// The elements of an array argument, or empty.
fn items(value: &Value) -> Vec<Value> {
    value
        .as_array()
        .map(|a| a.borrow().clone())
        .unwrap_or_default()
}

/// The marker a name argument refers to, if it exists.
fn marker_of<'w, H: WorldHost>(ctx: &'w Ctx<'_, H>, name: &Value) -> Option<&'w Marker> {
    name.as_str()
        .and_then(|n| ctx.host.world().markers().get(n))
}

/// `createMarker [name, position, channel, creator]`: creates the marker when the name is free
/// and returns the name, or `""` when it is taken (the engine ignores the command then).
fn create_marker<H: WorldHost>(ctx: &mut Ctx<'_, H>, args: &Value, local: bool) -> Value {
    let items = items(args);
    let Some(name) = items.first().and_then(Value::as_str).map(str::to_owned) else {
        return Value::from("");
    };
    let position = items
        .get(1)
        .and_then(|p| marker_position(ctx, p))
        .unwrap_or(DVec3::ZERO);
    let channel = items
        .get(2)
        .and_then(Value::as_number)
        .map_or(-1, |n| n as i32);
    let mut marker = Marker::new(name.clone(), position);
    marker.channel = channel;
    marker.local = local;
    if ctx.host.world_mut().markers_mut().create(marker) {
        Value::from(name)
    } else {
        Value::from("")
    }
}

/// Registers a `marker*` getter, its `setMarker*` setter and the `...Local` variant, over one
/// string field of [`Marker`].
macro_rules! marker_string_field {
    ($r:ident, $field:ident, $get:literal, $set:literal, $set_local:literal) => {
        $r.unary($get, STR, STR, |ctx, a| {
            Ok(Value::from(
                marker_of(ctx, &a).map_or("", |m| m.$field.as_str()),
            ))
        });
        $r.binary($set, STR, STR, NOTHING, |ctx, n, v| {
            if let (Some(name), Some(text)) = (n.as_str(), v.as_str()) {
                if let Some(m) = ctx.host.world_mut().markers_mut().get_mut(name) {
                    m.$field = text.to_owned();
                }
            }
            Ok(Value::Nothing)
        });
        $r.binary($set_local, STR, STR, NOTHING, |ctx, n, v| {
            if let (Some(name), Some(text)) = (n.as_str(), v.as_str()) {
                if let Some(m) = ctx.host.world_mut().markers_mut().get_mut(name) {
                    m.$field = text.to_owned();
                }
            }
            Ok(Value::Nothing)
        });
    };
}

/// Registers the same pair for a numeric field, with an optional clamp.
macro_rules! marker_number_field {
    ($r:ident, $field:ident, $get:literal, $set:literal, $set_local:literal) => {
        $r.unary($get, STR, NUM, |ctx, a| {
            Ok(Value::Number(marker_of(ctx, &a).map_or(0.0, |m| m.$field)))
        });
        $r.binary($set, STR, NUM, NOTHING, |ctx, n, v| {
            if let (Some(name), Some(value)) = (n.as_str(), v.as_number()) {
                if let Some(m) = ctx.host.world_mut().markers_mut().get_mut(name) {
                    m.$field = value;
                }
            }
            Ok(Value::Nothing)
        });
        $r.binary($set_local, STR, NUM, NOTHING, |ctx, n, v| {
            if let (Some(name), Some(value)) = (n.as_str(), v.as_number()) {
                if let Some(m) = ctx.host.world_mut().markers_mut().get_mut(name) {
                    m.$field = value;
                }
            }
            Ok(Value::Nothing)
        });
    };
}

pub(super) fn register<H: WorldHost>(r: &mut Registry<H>) {
    // AG EG: `createMarker [name, position, channel, creator]`.
    r.unary("createMarker", ARR, STR, |ctx, a| {
        Ok(create_marker(ctx, &a, false))
    });
    // AG EL.
    r.unary("createMarkerLocal", ARR, STR, |ctx, a| {
        Ok(create_marker(ctx, &a, true))
    });

    r.unary("deleteMarker", STR, NOTHING, |ctx, a| {
        if let Some(name) = a.as_str() {
            ctx.host.world_mut().markers_mut().delete(name);
        }
        Ok(Value::Nothing)
    });
    r.unary("deleteMarkerLocal", STR, NOTHING, |ctx, a| {
        if let Some(name) = a.as_str() {
            ctx.host.world_mut().markers_mut().delete(name);
        }
        Ok(Value::Nothing)
    });

    r.nular("allMapMarkers", ARR, |ctx| {
        Ok(Value::array(
            ctx.host.world().marker_names().into_iter().map(Value::from),
        ))
    });

    // `getMarkerPos name` and `getMarkerPos [name, preserveElevation]`.
    r.unary("getMarkerPos", STR, ARR, |ctx, a| {
        Ok(marker_pos(ctx, &a, false))
    });
    r.unary("markerPos", STR, ARR, |ctx, a| {
        Ok(marker_pos(ctx, &a, false))
    });
    r.unary("getMarkerPos", ARR, ARR, |ctx, a| {
        Ok(marker_pos_array(ctx, &a))
    });
    r.unary("markerPos", ARR, ARR, |ctx, a| {
        Ok(marker_pos_array(ctx, &a))
    });

    r.binary("setMarkerPos", STR, OBJ.union(ARR), NOTHING, |ctx, n, p| {
        set_marker_pos(ctx, &n, &p);
        Ok(Value::Nothing)
    });
    r.binary(
        "setMarkerPosLocal",
        STR,
        OBJ.union(ARR),
        NOTHING,
        |ctx, n, p| {
            set_marker_pos(ctx, &n, &p);
            Ok(Value::Nothing)
        },
    );

    r.binary("setMarkerDrawPriority", STR, NUM, NOTHING, |ctx, n, p| {
        if let (Some(name), Some(priority)) = (n.as_str(), p.as_number()) {
            ctx.host
                .world_mut()
                .markers_mut()
                .set_draw_priority(name, priority);
        }
        Ok(Value::Nothing)
    });
    r.unary("markerDrawPriority", STR, NUM, |ctx, a| {
        Ok(Value::Number(
            marker_of(ctx, &a).map_or(0.0, |m| m.draw_priority),
        ))
    });

    // The string fields (the two `getMarker*` aliases the engine table has).
    marker_string_field!(
        r,
        marker_type,
        "markerType",
        "setMarkerType",
        "setMarkerTypeLocal"
    );
    marker_string_field!(r, text, "markerText", "setMarkerText", "setMarkerTextLocal");
    marker_string_field!(
        r,
        color,
        "markerColor",
        "setMarkerColor",
        "setMarkerColorLocal"
    );
    r.unary("getMarkerColor", STR, STR, |ctx, a| {
        Ok(Value::from(
            marker_of(ctx, &a).map_or("", |m| m.color.as_str()),
        ))
    });
    r.unary("getMarkerType", STR, STR, |ctx, a| {
        Ok(Value::from(
            marker_of(ctx, &a).map_or("", |m| m.marker_type.as_str()),
        ))
    });

    // The shape is stored uppercased, as `markerShape` reports it.
    r.unary("markerShape", STR, STR, |ctx, a| {
        Ok(Value::from(
            marker_of(ctx, &a).map_or("", |m| m.shape.as_str()),
        ))
    });
    r.binary("setMarkerShape", STR, STR, NOTHING, |ctx, n, v| {
        if let (Some(name), Some(shape)) = (n.as_str(), v.as_str()) {
            if let Some(m) = ctx.host.world_mut().markers_mut().get_mut(name) {
                m.shape = shape.to_ascii_uppercase();
            }
        }
        Ok(Value::Nothing)
    });
    r.binary("setMarkerShapeLocal", STR, STR, NOTHING, |ctx, n, v| {
        if let (Some(name), Some(shape)) = (n.as_str(), v.as_str()) {
            if let Some(m) = ctx.host.world_mut().markers_mut().get_mut(name) {
                m.shape = shape.to_ascii_uppercase();
            }
        }
        Ok(Value::Nothing)
    });
    marker_string_field!(
        r,
        brush,
        "markerBrush",
        "setMarkerBrush",
        "setMarkerBrushLocal"
    );

    // Size.
    r.unary("markerSize", STR, ARR, |ctx, a| Ok(marker_size(ctx, &a)));
    r.unary("getMarkerSize", STR, ARR, |ctx, a| Ok(marker_size(ctx, &a)));
    r.binary("setMarkerSize", STR, ARR, NOTHING, |ctx, n, s| {
        set_marker_size(ctx, &n, &s);
        Ok(Value::Nothing)
    });
    r.binary("setMarkerSizeLocal", STR, ARR, NOTHING, |ctx, n, s| {
        set_marker_size(ctx, &n, &s);
        Ok(Value::Nothing)
    });

    // Direction, alpha, channel and the polyline.
    marker_number_field!(
        r,
        direction,
        "markerDir",
        "setMarkerDir",
        "setMarkerDirLocal"
    );
    r.unary("markerAlpha", STR, NUM, |ctx, a| {
        Ok(Value::Number(marker_of(ctx, &a).map_or(0.0, |m| m.alpha)))
    });
    r.binary("setMarkerAlpha", STR, NUM, NOTHING, |ctx, n, v| {
        set_marker_alpha(ctx, &n, &v);
        Ok(Value::Nothing)
    });
    r.binary("setMarkerAlphaLocal", STR, NUM, NOTHING, |ctx, n, v| {
        set_marker_alpha(ctx, &n, &v);
        Ok(Value::Nothing)
    });
    r.unary("markerChannel", STR, NUM, |ctx, a| {
        Ok(Value::Number(
            marker_of(ctx, &a).map_or(-1.0, |m| m.channel as f32),
        ))
    });
    r.unary("markerPolyline", STR, ARR, |ctx, a| {
        Ok(marker_polyline(ctx, &a))
    });
    r.binary("setMarkerPolyline", STR, ARR, NOTHING, |ctx, n, p| {
        set_marker_polyline(ctx, &n, &p);
        Ok(Value::Nothing)
    });
    r.binary("setMarkerPolylineLocal", STR, ARR, NOTHING, |ctx, n, p| {
        set_marker_polyline(ctx, &n, &p);
        Ok(Value::Nothing)
    });

    // Shadow (a bool).
    r.unary("markerShadow", STR, BOOL, |ctx, a| {
        Ok(Value::Bool(marker_of(ctx, &a).is_some_and(|m| m.shadow)))
    });
    r.binary("setMarkerShadow", STR, BOOL, NOTHING, |ctx, n, v| {
        set_marker_shadow(ctx, &n, &v);
        Ok(Value::Nothing)
    });
    r.binary("setMarkerShadowLocal", STR, BOOL, NOTHING, |ctx, n, v| {
        set_marker_shadow(ctx, &n, &v);
        Ok(Value::Nothing)
    });
}

fn marker_pos<H: WorldHost>(ctx: &Ctx<'_, H>, name: &Value, preserve_elevation: bool) -> Value {
    let Some(p) = marker_of(ctx, name).map(|m| m.position) else {
        return Value::array([0.0f32, 0.0, 0.0].map(Value::Number));
    };
    // `getMarkerPos` reports [x, y, 0] unless the elevation is asked for.
    let p = if preserve_elevation {
        p
    } else {
        DVec3::new(p.x, 0.0, p.z)
    };
    position_value(p)
}

/// `getMarkerPos [name, preserveElevation]`.
fn marker_pos_array<H: WorldHost>(ctx: &Ctx<'_, H>, args: &Value) -> Value {
    let items = items(args);
    let preserve = items.get(1).and_then(Value::as_bool).unwrap_or(false);
    match items.first() {
        Some(name) => marker_pos(ctx, name, preserve),
        None => Value::array([0.0f32, 0.0, 0.0].map(Value::Number)),
    }
}

/// `marker setMarkerPos positionOrObject`.
fn set_marker_pos<H: WorldHost>(ctx: &mut Ctx<'_, H>, name: &Value, pos: &Value) {
    let (Some(name), Some(position)) = (name.as_str(), marker_position(ctx, pos)) else {
        return;
    };
    if let Some(m) = ctx.host.world_mut().markers_mut().get_mut(name) {
        m.position = position;
    }
}

fn marker_size<H: WorldHost>(ctx: &Ctx<'_, H>, name: &Value) -> Value {
    let size = marker_of(ctx, name).map_or([0.0, 0.0], |m| m.size);
    Value::array([Value::Number(size[0]), Value::Number(size[1])])
}

/// `[a-axis, b-axis]`; a one-element array uses the same value for both.
fn size_arg(value: &Value) -> Option<[f32; 2]> {
    let n = numbers(value)?;
    Some([*n.first()? as f32, *n.get(1).unwrap_or(&n[0]) as f32])
}

fn set_marker_size<H: WorldHost>(ctx: &mut Ctx<'_, H>, name: &Value, size: &Value) {
    let (Some(name), Some(size)) = (name.as_str(), size_arg(size)) else {
        return;
    };
    if let Some(m) = ctx.host.world_mut().markers_mut().get_mut(name) {
        m.size = size;
    }
}

/// The points of a polyline argument: an array of `[x, y]` / `[x, y, z]` positions, or one flat
/// position array.
fn polyline_arg(value: &Value) -> Option<Vec<DVec3>> {
    let points = items(value);
    if points.len() <= 3 && numbers(value).is_some() {
        return Some(vec![script_position(value)?]);
    }
    points.iter().map(script_position).collect()
}

fn marker_polyline<H: WorldHost>(ctx: &Ctx<'_, H>, name: &Value) -> Value {
    let points = marker_of(ctx, name)
        .map(|m| m.polyline.clone())
        .unwrap_or_default();
    Value::array(points.into_iter().map(position_value))
}

fn set_marker_polyline<H: WorldHost>(ctx: &mut Ctx<'_, H>, name: &Value, points: &Value) {
    let (Some(name), Some(points)) = (name.as_str(), polyline_arg(points)) else {
        return;
    };
    if let Some(m) = ctx.host.world_mut().markers_mut().get_mut(name) {
        m.polyline = points;
    }
}

fn set_marker_shadow<H: WorldHost>(ctx: &mut Ctx<'_, H>, name: &Value, value: &Value) {
    let (Some(name), Some(shadow)) = (name.as_str(), value.as_bool()) else {
        return;
    };
    if let Some(m) = ctx.host.world_mut().markers_mut().get_mut(name) {
        m.shadow = shadow;
    }
}

fn set_marker_alpha<H: WorldHost>(ctx: &mut Ctx<'_, H>, name: &Value, value: &Value) {
    let (Some(name), Some(alpha)) = (name.as_str(), value.as_number()) else {
        return;
    };
    if let Some(m) = ctx.host.world_mut().markers_mut().get_mut(name) {
        m.alpha = alpha.clamp(0.0, 1.0);
    }
}
