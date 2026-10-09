//! Position arithmetic between positions and Objects: `distance`, `distance2D`,
//! `distanceSqr`, `getDir`, the relative form of `getPos`, `inArea`, `inPolygon` and the
//! model/world conversions.
//!
//! A position argument is either an Object (its world position) or an array. An array is a
//! `PositionAGL`: like the original, the value is taken relative to the surface underneath, so
//! the same `[x, y, 0]` means "on the ground" for an Object and for an array alike.

use a3_sqf::vm::Ctx;
use a3_sqf::{Registry, Value};
use glam::DVec3;

use super::{ARR, BOOL, NUM, OBJ, WorldHost, object_arg, position_value, script_position};
use crate::{ObjectRef, World};

/// The engine's degrees-to-radians factor on the `getPos`/`BIS_fnc_relPos` path. It is one f32
/// step above `pi / 180`, which is visible in the last digits of a relative position at a
/// right angle (`[0, 0, 0] getPos [10, 90]` is `[10, -1.62921e-06, 0]`, not `-4.37114e-07`).
const DEG_TO_RAD: f32 = 0.017453294;

pub(super) fn register<H: WorldHost>(r: &mut Registry<H>) {
    for (name, mode) in [
        ("distance", Measure::Distance),
        ("distance2D", Measure::Flat),
        ("distanceSqr", Measure::Squared),
    ] {
        let f: fn(&mut Ctx<'_, H>, Value, Value) -> Result<Value, a3_sqf::SqfError> = match mode {
            Measure::Distance => |ctx, a, b| {
                Ok(Value::Number(measure(
                    ctx.host.world(),
                    &a,
                    &b,
                    Measure::Distance,
                )))
            },
            Measure::Flat => |ctx, a, b| {
                Ok(Value::Number(measure(
                    ctx.host.world(),
                    &a,
                    &b,
                    Measure::Flat,
                )))
            },
            Measure::Squared => |ctx, a, b| {
                Ok(Value::Number(measure(
                    ctx.host.world(),
                    &a,
                    &b,
                    Measure::Squared,
                )))
            },
        };
        for left in [OBJ, ARR] {
            for right in [OBJ, ARR] {
                r.binary(name, left, right, NUM, f);
            }
        }
    }

    // `from getDir to`: the compass direction from one position or Object to another, 0..360.
    // `object getDir` keeps the single-argument form (the Object's own heading).
    let dir: fn(&mut Ctx<'_, H>, Value, Value) -> Result<Value, a3_sqf::SqfError> = |ctx, a, b| {
        Ok(Value::Number(
            match (point(ctx.host.world(), &a), point(ctx.host.world(), &b)) {
                (Some(from), Some(to)) => direction(from, to),
                _ => 0.0,
            },
        ))
    };
    for left in [OBJ, ARR] {
        for right in [OBJ, ARR] {
            r.binary("getDir", left, right, NUM, dir);
        }
    }

    // `origin getPos [distance, heading]`: a position that far away in that compass direction.
    // The result is a `PositionAGLS` with `z` = 0 over land and the (negative) terrain height
    // over water.
    let relative: fn(&mut Ctx<'_, H>, Value, Value) -> Result<Value, a3_sqf::SqfError> =
        |ctx, a, b| {
            let Some(from) = point(ctx.host.world(), &a) else {
                return Ok(Value::array([]));
            };
            let Some(args) = numbers_of(&b, 2) else {
                return Ok(Value::array([]));
            };
            Ok(get_pos_relative(ctx.host.world(), from, args[0], args[1]))
        };
    for left in [OBJ, ARR] {
        r.binary("getPos", left, ARR, ARR, relative);
        r.binary("position", left, ARR, ARR, relative);
    }

    // `position inArea [center, a, b, angle, isRectangle, c, usePosWorld]`.
    let in_area: fn(&mut Ctx<'_, H>, Value, Value) -> Result<Value, a3_sqf::SqfError> =
        |ctx, a, b| {
            let w = ctx.host.world();
            let (Some(p), Some(area)) = (point(w, &a), area_of(w, &b)) else {
                return Ok(Value::Bool(false));
            };
            Ok(Value::Bool(area.contains(p)))
        };
    for left in [ARR, OBJ] {
        r.binary("inArea", left, ARR, BOOL, in_area);
    }

    // `position inPolygon polygon`: 2D point-in-polygon (the z values are ignored).
    r.binary("inPolygon", ARR, ARR, BOOL, |ctx, a, b| {
        let w = ctx.host.world();
        let (Some(p), Some(polygon)) = (point(w, &a), area_of_polygon(w, &b)) else {
            return Ok(Value::Bool(false));
        };
        Ok(Value::Bool(polygon.contains(p)))
    });

    // `object modelToWorld offset` / `object worldToModel position`. The model axes are
    // x = right, y = forward, z = up.
    r.binary("modelToWorld", OBJ, ARR, ARR, |ctx, a, b| {
        let w = ctx.host.world();
        let Some(local) = vector(&b) else {
            return Ok(Value::array([]));
        };
        let Some(id) = entity(w, &a) else {
            return Ok(Value::array([]));
        };
        let e = w.entity(id).expect("exists");
        let offset = e.orientation() * model_to_world_vector(local);
        Ok(position_value(e.position() + offset))
    });
    r.binary("worldToModel", OBJ, ARR, ARR, |ctx, a, b| {
        let w = ctx.host.world();
        let Some(world_point) = point(w, &b) else {
            return Ok(Value::array([]));
        };
        let Some(id) = entity(w, &a) else {
            return Ok(Value::array([]));
        };
        let e = w.entity(id).expect("exists");
        let local = e.orientation().inverse() * (world_point - e.position());
        Ok(vector_value_of(world_to_model_vector(local)))
    });
}

/// The world point a `position` argument names: an Object's position, or an array as a position
/// above the surface (ADR 0003: script `[x, y, z]` is world `(x, z, y)`).
fn point(world: &World, value: &Value) -> Option<DVec3> {
    if let Some(object) = object_arg(world, value) {
        return world.object_position(object);
    }
    let mut p = script_position(value)?;
    p.y += world.surface_height(p.x, p.z);
    Some(p)
}

/// The Entity behind a value, if it is one.
fn entity(world: &World, value: &Value) -> Option<crate::EntityId> {
    match object_arg(world, value)? {
        ObjectRef::Entity(id) => Some(id),
        ObjectRef::Static(_) => None,
    }
}

/// The numbers of an array position with at least `n` elements.
fn numbers_of(value: &Value, n: usize) -> Option<Vec<f64>> {
    let numbers = super::numbers(value)?;
    (numbers.len() >= n).then_some(numbers)
}

/// A model-space vector from a 3-element array.
fn vector(value: &Value) -> Option<DVec3> {
    match numbers_of(value, 3)?.as_slice() {
        [x, y, z, ..] => Some(DVec3::new(*x, *y, *z)),
        _ => None,
    }
}

/// Model space (`x` right, `y` forward, `z` up) to world space (`x` east, `y` up, `z` north).
fn model_to_world_vector(v: DVec3) -> DVec3 {
    DVec3::new(v.y, v.z, -v.x)
}

/// World space to model space.
fn world_to_model_vector(v: DVec3) -> DVec3 {
    DVec3::new(-v.z, v.x, v.y)
}

/// A vector as a script array (`[x, y, z]`, model space order).
fn vector_value_of(v: DVec3) -> Value {
    Value::array([v.x, v.y, v.z].map(|n| Value::Number(n as f32)))
}

/// Which of the three distance commands [`measure`] computes.
#[derive(Clone, Copy)]
enum Measure {
    /// 3D distance (`distance`).
    Distance,
    /// 2D distance, the height ignored (`distance2D`).
    Flat,
    /// Squared 3D distance (`distanceSqr`).
    Squared,
}

/// `a distance b` and its two variants.
fn measure(world: &World, a: &Value, b: &Value, mode: Measure) -> f32 {
    let (Some(p), Some(q)) = (point(world, a), point(world, b)) else {
        return 0.0;
    };
    match mode {
        Measure::Distance => p.distance(q) as f32,
        Measure::Flat => DVec3::new(p.x, 0.0, p.z).distance(DVec3::new(q.x, 0.0, q.z)) as f32,
        Measure::Squared => p.distance_squared(q) as f32,
    }
}

/// The compass direction from `from` to `to`, in degrees clockwise from north.
fn direction(from: DVec3, to: DVec3) -> f32 {
    let (dx, dz) = ((to.x - from.x) as f32, (to.z - from.z) as f32);
    dx.atan2(dz).to_degrees().rem_euclid(360.0)
}

/// `origin getPos [distance, heading]`.
fn get_pos_relative(world: &World, from: DVec3, distance: f64, heading: f64) -> Value {
    let radians = (heading as f32) * DEG_TO_RAD;
    let (sin, cos) = (radians.sin(), radians.cos());
    let x = (from.x + distance * f64::from(sin)) as f32;
    let y = (from.z + distance * f64::from(cos)) as f32;
    // The height is the terrain level: 0 over land, the (negative) terrain height over water.
    let surface = world.surface_height(f64::from(x), f64::from(y)) as f32;
    Value::array([
        Value::Number(x),
        Value::Number(y),
        Value::Number(surface.min(0.0)),
    ])
}

/// An area for `inArea`: `[center, a, b, angle, isRectangle, c]`, or a plain radius form
/// `[center, radius, radius]`.
struct Area {
    center: DVec3,
    /// Half extent along the area's x axis.
    a: f64,
    /// Half extent along the area's y axis.
    b: f64,
    /// Vertical half extent; negative means no limit.
    c: f64,
    /// Clockwise rotation of the area, in degrees.
    angle: f64,
    rectangle: bool,
}

impl Area {
    fn contains(&self, p: DVec3) -> bool {
        if self.c > 0.0 && (p.y - self.center.y).abs() > self.c {
            return false;
        }
        let (dx, dy) = (p.x - self.center.x, p.z - self.center.z);
        let (sin, cos) = (-self.angle.to_radians()).sin_cos();
        let (x, y) = (dx * cos - dy * sin, dx * sin + dy * cos);
        if self.rectangle {
            x.abs() <= self.a && y.abs() <= self.b
        } else if self.a < 0.0 && self.b < 0.0 {
            // Negative extents make a hexagon, which this does not model yet.
            false
        } else {
            let (a, b) = (
                self.a.abs().max(f64::MIN_POSITIVE),
                self.b.abs().max(f64::MIN_POSITIVE),
            );
            (x / a).powi(2) + (y / b).powi(2) <= 1.0
        }
    }
}

/// The area an `inArea` argument describes.
fn area_of(world: &World, value: &Value) -> Option<Area> {
    let items = value.as_array().map(|a| a.borrow().clone())?;
    let center = items.first().and_then(|c| point(world, c))?;
    let a = items.get(1).and_then(Value::as_number)?;
    let b = items.get(2).and_then(Value::as_number).unwrap_or(a);
    Some(Area {
        center,
        a: f64::from(a),
        b: f64::from(b),
        c: items
            .get(5)
            .and_then(Value::as_number)
            .map_or(-1.0, f64::from),
        angle: items
            .get(3)
            .and_then(Value::as_number)
            .map_or(0.0, f64::from),
        rectangle: items.get(4).and_then(Value::as_bool).unwrap_or(false),
    })
}

/// A polygon for `inPolygon`, in 2D.
struct Polygon {
    points: Vec<(f64, f64)>,
}

impl Polygon {
    /// Whether `p` is inside, by the even-odd rule (points on an edge are inside).
    fn contains(&self, p: DVec3) -> bool {
        let (x, y) = (p.x, p.z);
        let mut inside = false;
        let n = self.points.len();
        for i in 0..n {
            let (x1, y1) = self.points[i];
            let (x2, y2) = self.points[(i + 1) % n];
            if on_segment((x, y), (x1, y1), (x2, y2)) {
                return true;
            }
            if (y1 > y) != (y2 > y) {
                let t = (y - y1) / (y2 - y1);
                if x < x1 + t * (x2 - x1) {
                    inside = !inside;
                }
            }
        }
        inside
    }
}

/// Whether `p` lies on the closed segment `a`..`b`.
fn on_segment(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> bool {
    let cross = (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0);
    if cross.abs() > 1e-9 {
        return false;
    }
    let dot = (p.0 - a.0) * (b.0 - a.0) + (p.1 - a.1) * (b.1 - a.1);
    let len = (b.0 - a.0).powi(2) + (b.1 - a.1).powi(2);
    dot >= 0.0 && dot <= len
}

/// The polygon an `inPolygon` argument describes.
fn area_of_polygon(world: &World, value: &Value) -> Option<Polygon> {
    let items = value.as_array().map(|a| a.borrow().clone())?;
    let points: Vec<(f64, f64)> = items
        .iter()
        .filter_map(|p| point(world, p))
        .map(|p| (p.x, p.z))
        .collect();
    (points.len() >= 3).then_some(Polygon { points })
}
