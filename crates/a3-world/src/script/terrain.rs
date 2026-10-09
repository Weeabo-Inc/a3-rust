//! Terrain queries: heights, water and surface type, surface normals, the ASL/AGL/ATL
//! conversions, ray casts against the terrain, and the world clock and weather.
//!
//! Heights, normals and ray casts work on the height grid the engine samples: heights are
//! interpolated on the triangles the engine splits each height cell into, and the surface normal
//! is the per-sample normal (from the height difference to the sample west and south of it),
//! bilinearly interpolated. Both are verified against the server oracle on Stratis; see
//! `docs/re/landscape.md`.

use a3_environment::{DateTime, sun_position};
use a3_physics::TerrainShape;
use a3_sqf::vm::Ctx;
use a3_sqf::{Registry, Value};
use a3_wrp::Terrain;
use glam::DVec3;

use super::{ARR, BOOL, NUM, STR, WorldHost, null_object, numbers};
use crate::World;

/// Vertical extent of the sun that `sunOrMoon` counts as day (`sunOrMoon` is 1 while the sun is
/// above the horizon).
const SUN_HORIZON: f64 = 0.0;

pub(super) fn register<H: WorldHost>(r: &mut Registry<H>) {
    r.nular("worldSize", NUM, |ctx| {
        Ok(Value::Number(
            ctx.host
                .world()
                .terrain()
                .map_or(0.0, |t| t.world_size() as f32),
        ))
    });
    // The `CfgWorlds` class of the terrain (`worldName`); "" until a loader sets it.
    r.nular("worldName", STR, |ctx| {
        Ok(Value::string(ctx.host.world().world_name().to_owned()))
    });

    // `[x, y]` or `[x, y, z]`: the height is above sea level, the third element is ignored.
    r.unary("getTerrainHeightASL", ARR, NUM, |ctx, a| {
        Ok(Value::Number(position_2d(&a).map_or(0.0, |(x, z)| {
            surface_height(ctx.host.world(), x, z)
        })))
    });
    r.unary("surfaceIsWater", ARR, BOOL, |ctx, a| {
        Ok(Value::Bool(position_2d(&a).is_some_and(|(x, z)| {
            surface_height(ctx.host.world(), x, z) < 0.0
        })))
    });
    r.unary("surfaceNormal", ARR, ARR, |ctx, a| {
        let Some((x, z)) = position_2d(&a) else {
            return Ok(Value::array([]));
        };
        Ok(ctx.host.world().terrain().map_or_else(
            || Value::array([]),
            |t| vector_value(terrain_normal(t, x, z)),
        ))
    });

    // The height conversions. ASL is above sea level, AGL above the surface and ATL above the
    // terrain; AGL and ATL differ only over a roadway, which is not modelled yet.
    for (name, to_sea) in [
        ("ASLToAGL", false),
        ("ASLToATL", false),
        ("AGLToASL", true),
        ("ATLToASL", true),
    ] {
        let f: fn(&mut Ctx<'_, H>, Value) -> Result<Value, a3_sqf::SqfError> = if to_sea {
            |ctx, a| Ok(convert_height(ctx.host.world(), &a, true))
        } else {
            |ctx, a| Ok(convert_height(ctx.host.world(), &a, false))
        };
        r.unary(name, ARR, ARR, f);
    }

    // `terrainIntersect [from, to]` and its ASL form: whether the line crosses the terrain.
    for (name, asl) in [("terrainIntersect", false), ("terrainIntersectASL", true)] {
        let f: fn(&mut Ctx<'_, H>, Value) -> Result<Value, a3_sqf::SqfError> = if asl {
            |ctx, a| Ok(Value::Bool(terrain_intersect(ctx.host.world(), &a, true)))
        } else {
            |ctx, a| Ok(Value::Bool(terrain_intersect(ctx.host.world(), &a, false)))
        };
        r.unary(name, ARR, BOOL, f);
    }
    // `lineIntersects [begASL, endASL, ignore1, ignore2]`: Objects only, never the terrain. It
    // needs the Objects' model geometry through the collision world (issue #117).
    r.unary("lineIntersects", ARR, BOOL, |ctx, a| {
        Ok(Value::Bool(line_intersects(ctx.host.world(), &a)))
    });
    // `lineIntersectsSurfaces [begASL, endASL, ignore1, ignore2, sortMode, maxResults, ...]`,
    // a list of `[position, normal, object, parent, selections, surface]`.
    r.unary("lineIntersectsSurfaces", ARR, ARR, |ctx, a| {
        Ok(line_intersects_surfaces(ctx.host.world(), &a))
    });

    // The world clock and weather.
    r.nular("date", ARR, |ctx| Ok(date_value(ctx.host.world())));
    r.nular("dayTime", NUM, |ctx| {
        Ok(Value::Number(
            ctx.host.world().environment().day_time() as f32
        ))
    });
    r.nular("overcast", NUM, |ctx| {
        Ok(Value::Number(ctx.host.world().environment().overcast))
    });
    r.nular("fog", NUM, |ctx| {
        Ok(Value::Number(ctx.host.world().environment().fog.value))
    });
    r.nular("sunOrMoon", NUM, |ctx| {
        Ok(Value::Number(sun_or_moon(ctx.host.world()) as f32))
    });

    // `sizeOf classname`: the diameter of an Object of that class present in the mission. The
    // model's bounding sphere needs the model geometry (issue #117), so only the "no such object"
    // case is answered.
    r.unary("sizeOf", STR, NUM, |ctx, a| {
        let name = a.as_str().unwrap_or_default().to_owned();
        let present = ctx
            .host
            .world()
            .entities()
            .any(|e| !e.is_deleted() && e.type_name().eq_ignore_ascii_case(&name));
        Ok(Value::Number(if present { 0.0 } else { 0.0 }))
    });

    // `disableSerialization` lives in the game's own registry (`a3-gamedata`), which the
    // function library needs at the main menu too; see #281.
}

/// A `[x, y]` (or `[x, y, z]`) position, with the third element ignored.
fn position_2d(value: &Value) -> Option<(f64, f64)> {
    match numbers(value)?.as_slice() {
        [x, z, ..] => Some((*x, *z)),
        _ => None,
    }
}

/// The terrain height (ASL) at `(x, z)`, 0 without a terrain.
fn surface_height(world: &World, x: f64, z: f64) -> f32 {
    world.surface_height(x, z) as f32
}

/// A script vector `[x, y, z]` from a world-space vector (ADR 0003).
fn vector_value(v: DVec3) -> Value {
    Value::array([v.x, v.z, v.y].map(|n| Value::Number(n as f32)))
}

/// A script position `[x, y, z]` from a world-space position.
fn position_value(p: DVec3) -> Value {
    vector_value(p)
}

/// `ASLToAGL`/`ASLToATL` (`to_sea` false) and `AGLToASL`/`ATLToASL` (`to_sea` true).
fn convert_height(world: &World, position: &Value, to_sea: bool) -> Value {
    let Some(p) = numbers(position)
        .filter(|n| n.len() >= 3)
        .map(|n| (n[0], n[1], n[2]))
    else {
        return Value::array([]);
    };
    let surface = f64::from(surface_height(world, p.0, p.1));
    let y = if to_sea { p.2 + surface } else { p.2 - surface };
    Value::array([p.0, p.1, y].map(|n| Value::Number(n as f32)))
}

/// The terrain's surface normal at `(x, z)`.
///
/// Each height sample carries a normal from the height difference to the sample west and south
/// of it; the normal at a position is the bilinear blend of the four samples around it. Verified
/// against the oracle at grid vertices on Stratis (`surfaceNormal [3500, 4500]`), where the four
/// blends collapse to the sample's own normal; the interior blend is a hypothesis (**medium**,
/// `docs/re/landscape.md`).
pub fn terrain_normal(terrain: &Terrain, x: f64, z: f64) -> DVec3 {
    let cell = f64::from(terrain.terrain_cell_size());
    if cell <= 0.0 {
        return DVec3::Y;
    }
    let (gx, gz) = (x / cell, z / cell);
    let (fi, fj) = (gx.floor(), gz.floor());
    let (fx, fz) = (gx - fi, gz - fj);
    let (i, j) = (fi as i64, fj as i64);
    let blend = |a: DVec3, b: DVec3, t: f64| a * (1.0 - t) + b * t;
    let north = blend(
        sample_normal(terrain, i, j, cell),
        sample_normal(terrain, i + 1, j, cell),
        fx,
    );
    let south = blend(
        sample_normal(terrain, i, j + 1, cell),
        sample_normal(terrain, i + 1, j + 1, cell),
        fx,
    );
    blend(north, south, fz).normalize_or(DVec3::Y)
}

/// The normal of the height sample `(i, j)`: the surface through the sample and its west and
/// south neighbours.
fn sample_normal(terrain: &Terrain, i: i64, j: i64, cell: f64) -> DVec3 {
    let h = |i: i64, j: i64| f64::from(terrain.grid_height(i, j));
    let h0 = h(i, j);
    // At the map edge the difference falls back to the sample on the other side.
    let (dx, dz) = if i > 0 && j > 0 {
        (h0 - h(i - 1, j), h0 - h(i, j - 1))
    } else if i > 0 {
        (h0 - h(i - 1, j), h(i, j + 1) - h0)
    } else if j > 0 {
        (h(i + 1, j) - h0, h0 - h(i, j - 1))
    } else {
        (h(i + 1, j) - h0, h(i, j + 1) - h0)
    };
    DVec3::new(-dx, cell, -dz).normalize_or(DVec3::Y)
}

/// The terrain shape of the World, for the ray casts.
fn terrain_shape(world: &World) -> Option<TerrainShape> {
    world.terrain().map(|t| TerrainShape::new(t.clone()))
}

/// `terrainIntersect` (`asl` false, the positions are AGL) and `terrainIntersectASL`.
fn terrain_intersect(world: &World, segment: &Value, asl: bool) -> bool {
    let Some((from, to)) = segment_ends(world, segment, asl) else {
        return false;
    };
    let Some(shape) = terrain_shape(world) else {
        return false;
    };
    // A hit along the segment, or along the reversed segment when the start is below the surface
    // (the engine cannot see out of the ground, `lineIntersectsSurfaces` documents the same).
    shape
        .cast_ray(from, to - from, 1.0)
        .is_some_and(|hit| hit.distance >= 0.0)
        || shape
            .cast_ray(to, from - to, 1.0)
            .is_some_and(|hit| hit.distance >= 0.0)
}

/// `lineIntersects`: Objects only. Without the Objects' model geometry in the collision world
/// (issue #117) nothing intersects.
fn line_intersects(world: &World, _segment: &Value) -> bool {
    world.collision_world().is_some_and(|_| false)
}

/// `lineIntersectsSurfaces`: the terrain crossings of the segment, nearest first.
fn line_intersects_surfaces(world: &World, arguments: &Value) -> Value {
    let items = arguments.as_array().map(|a| a.borrow().clone());
    let Some(from) = items.as_ref().and_then(|i| i.first()).and_then(asl_point) else {
        return Value::array([]);
    };
    let Some(to) = items.as_ref().and_then(|i| i.get(1)).and_then(asl_point) else {
        return Value::array([]);
    };
    let hits: Vec<Value> = terrain_shape(world)
        .and_then(|shape| shape.cast_ray(from, to - from, 1.0))
        .map(|hit| {
            let normal = world.terrain().map_or(DVec3::Y, |t| {
                terrain_normal(t, hit.position.x, hit.position.z)
            });
            vec![surface_hit(hit.position, normal)]
        })
        .unwrap_or_default();
    Value::array(hits)
}

/// One `lineIntersectsSurfaces` result for the terrain: the position, its normal, and no Object
/// (terrain hits report `objNull`). The selection list and the surface path are empty.
fn surface_hit(position: DVec3, normal: DVec3) -> Value {
    Value::array([
        position_value(position),
        vector_value(normal),
        null_object(),
        null_object(),
        Value::array([]),
        Value::string(""),
    ])
}

/// The two ends of a segment argument, in world space. AGL positions (`asl` false) are raised by
/// the surface height.
fn segment_ends(world: &World, segment: &Value, asl: bool) -> Option<(DVec3, DVec3)> {
    let items = segment.as_array().map(|a| a.borrow().clone())?;
    let mut ends = [items.first()?, items.get(1)?].into_iter().map(|p| {
        let n = numbers(&p.clone())?;
        match n.as_slice() {
            [x, y, z, ..] => Some(DVec3::new(*x, *z, *y)),
            _ => None,
        }
    });
    let (from, to) = (ends.next()??, ends.next()??);
    if asl {
        return Some((from, to));
    }
    let raise = |p: DVec3| DVec3::new(p.x, p.y + world.surface_height(p.x, p.z), p.z);
    Some((raise(from), raise(to)))
}

/// An ASL position from a script array.
fn asl_point(value: &Value) -> Option<DVec3> {
    let n = numbers(value)?;
    match n.as_slice() {
        [x, y, z, ..] => Some(DVec3::new(*x, *z, *y)),
        _ => None,
    }
}

/// `date` as `[year, month, day, hour, minute]`.
fn date_value(world: &World) -> Value {
    let dt: DateTime = world.environment().date_time;
    Value::array([
        Value::Number(dt.year as f32),
        Value::Number(dt.month as f32),
        Value::Number(dt.day as f32),
        Value::Number(dt.hours.floor() as f32),
        Value::Number(((dt.hours.fract() * 60.0).round() % 60.0) as f32),
    ])
}

/// `sunOrMoon`: 1 while the sun is above the horizon, 0 while it is below. Without the world's
/// observer (no `CfgWorlds >> latitude`) the date alone cannot place the sun, so the equator is
/// assumed.
fn sun_or_moon(world: &World) -> f64 {
    let date = world.environment().date_time;
    let height = match world.observer() {
        Some(observer) => sun_position(observer, &date).elevation,
        None => {
            let observer = a3_environment::Observer::from_world_config(0.0, 0.0);
            sun_position(&observer, &date).elevation
        }
    };
    if height > SUN_HORIZON { 1.0 } else { 0.0 }
}
