//! SQF commands on one AI unit's movement: `setUnitPosWeak`, `forceSpeed`, `setFormDir`,
//! `formationDirection`, `formationPosition`, `formationLeader`, `isFormationLeader`,
//! `unitReady`, `moveToCompleted`, `moveToFailed` (handlers in `docs/re/ai.md` §6).

use a3_sqf::{Registry, Type, TypeSet, Value};

use super::ai::{group_of_arg, unit_arg, units_arg};
use super::{ARR, BOOL, NOTHING, NUM, OBJ, STR, WorldHost, object_value, position_value};
use crate::UnitPos;

const GRP: TypeSet = TypeSet::of(Type::Group);

pub(super) fn register<H: WorldHost>(r: &mut Registry<H>) {
    // AL EG. The AI's own ("weak") stance request, as the formation FSM makes it.
    r.binary("setUnitPosWeak", OBJ, STR, NOTHING, |ctx, u, s| {
        let w = ctx.host.world();
        if let (Some(unit), Some(pos)) = (unit_arg(w, &u), s.as_str().and_then(UnitPos::parse))
            && let Some(man) = ctx.host.world_mut().man_mut(unit)
        {
            man.ai.fsm_unit_pos = pos;
        }
        Ok(Value::Nothing)
    });
    // AL EG. `unit forceSpeed metresPerSecond`; negative removes the cap.
    r.binary("forceSpeed", OBJ, NUM, NOTHING, |ctx, u, n| {
        let w = ctx.host.world();
        if let (Some(unit), Some(speed)) = (unit_arg(w, &u), n.as_number()) {
            ctx.host.world_mut().force_speed(unit, f64::from(speed));
        }
        Ok(Value::Nothing)
    });
    // AG EG. `group setFormDir heading`.
    for left in [OBJ, GRP] {
        r.binary("setFormDir", left, NUM, NOTHING, |ctx, g, n| {
            let w = ctx.host.world();
            if let (Some(group), Some(heading)) = (group_of_arg(w, &g), n.as_number()) {
                let _ = ctx
                    .host
                    .world_mut()
                    .set_formation_direction(group, f64::from(heading));
            }
            Ok(Value::Nothing)
        });
    }
    r.unary("formationDirection", OBJ, NUM, |ctx, u| {
        let w = ctx.host.world();
        let heading = group_of_arg(w, &u)
            .and_then(|g| w.formation_direction(g))
            .unwrap_or(0.0);
        Ok(Value::Number(heading as f32))
    });
    // `formationPosition unit`: his slot, above the terrain.
    r.unary("formationPosition", OBJ, ARR, |ctx, u| {
        let w = ctx.host.world();
        let Some(unit) = unit_arg(w, &u) else {
            return Ok(Value::array([]));
        };
        let Some(mut slot) = w.formation_position(unit) else {
            return Ok(Value::array([]));
        };
        slot.y -= w.surface_height(slot.x, slot.z);
        Ok(position_value(slot))
    });
    r.unary("formationLeader", OBJ, OBJ, |ctx, u| {
        let w = ctx.host.world();
        let leader = group_of_arg(w, &u).and_then(|g| w.group(g)?.leader());
        Ok(match leader {
            Some(l) => object_value(w, crate::ObjectRef::Entity(l)),
            None => super::null_object(),
        })
    });
    r.unary("isFormationLeader", OBJ, BOOL, |ctx, u| {
        let w = ctx.host.world();
        let is = unit_arg(w, &u).is_some_and(|unit| {
            w.group_of(unit)
                .and_then(|g| w.group(g)?.leader())
                .is_some_and(|l| l == unit)
        });
        Ok(Value::Bool(is))
    });

    // `unitReady unit(s)`: false only for a leader whose group still has a move to make.
    for arg in [OBJ, ARR] {
        r.unary("unitReady", arg, BOOL, |ctx, u| {
            let w = ctx.host.world();
            Ok(Value::Bool(
                units_arg(w, &u).iter().all(|u| w.unit_ready(*u)),
            ))
        });
    }
    r.unary("moveToCompleted", OBJ, BOOL, |ctx, u| {
        let w = ctx.host.world();
        Ok(Value::Bool(
            unit_arg(w, &u).is_none_or(|u| w.move_to_completed(u)),
        ))
    });
    // Always false in 2.22 (`docs/re/ai.md` §5).
    r.unary("moveToFailed", OBJ, BOOL, |_ctx, _u| Ok(Value::Bool(false)));
}
