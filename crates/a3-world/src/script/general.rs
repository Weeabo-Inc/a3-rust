//! Engine-wide state that belongs to no object: the client's network state and the named cut
//! layers (`cutRsc`, `cutText`, `cutObj`, `cutFadeOut`).

use super::{ARR, BOOL, NOTHING, NUM, OBJ, STR, WorldHost, null_object};
use a3_sqf::{Registry, TypeSet, Value};
use std::sync::atomic::{AtomicI32, Ordering};

/// Handles handed out by [`ppEffectCreate`]. We render no post-process effects, so a handle is
/// only an identity the caller can pass back; the counter keeps two creations distinct the way
/// the engine's priorities do.
static PP_HANDLE: AtomicI32 = AtomicI32::new(1);

pub(super) fn register<H: WorldHost>(r: &mut Registry<H>) {
    // `onTeamSwitch code`: the code the engine runs when the player switches team. We have no
    // team switching, so nothing runs it. Recorded as `stub`.
    r.unary("onTeamSwitch", TypeSet::ANYTHING, NOTHING, |_, _| {
        Ok(Value::Nothing)
    });
    r.unary("enableTeamSwitch", BOOL, NOTHING, |_, _| Ok(Value::Nothing));

    // `fadeSound time, volume`: the engine's sound-fade bus, which no mixer bus is wired to yet
    // (`stub`).
    r.binary("fadeSound", NUM, NUM, NOTHING, |_, _, _| Ok(Value::Nothing));

    // `enableSaving`: there is no save system, so nothing changes (`stub`).
    r.unary("enableSaving", BOOL, NOTHING, |_, _| Ok(Value::Nothing));
    r.unary("enableSaving", ARR, NOTHING, |_, _| Ok(Value::Nothing));

    // `cutText`: we render no cut text; the layer is still reported (`stub`).
    r.unary("cutText", ARR, NOTHING, |_, _| Ok(Value::Nothing));
    r.binary("cutText", NUM, ARR, NOTHING, |_, _, _| Ok(Value::Nothing));
    r.binary("cutText", STR, ARR, NUM, |_, _, _| Ok(Value::Number(0.0)));

    // `enableMimics`: facial animation, which we do not drive (`stub`).
    r.binary("enableMimics", OBJ, BOOL, NOTHING, |_, _, _| {
        Ok(Value::Nothing)
    });

    // `a disableCollisionWith b`: the engine tells the physics world to ignore the pair. Our
    // collision world has no per-pair filter yet, so this is a no-op (`stub`).
    r.binary("disableCollisionWith", OBJ, OBJ, NOTHING, |_, _, _| {
        Ok(Value::Nothing)
    });

    // `driver vehicle`: the unit in the driver's seat. Nobody is ever inside a vehicle here (no
    // crew model), so the answer is the null object.
    r.unary("driver", OBJ, OBJ, |_, _| Ok(null_object()));

    // `UAVControl uav`: `[operator, vehicleRole]`. Nobody controls a UAV here, which the engine
    // reports as `[objNull, ""]`. The alternative `[uav, option]` form answers the same.
    r.unary("UAVControl", OBJ, ARR, |_, _| {
        Ok(Value::array([null_object(), Value::string("")]))
    });
    r.unary("UAVControl", ARR, ARR, |_, _| {
        Ok(Value::array([null_object(), Value::string("")]))
    });

    // The post-process effect family. We render no post-process effects, so `ppEffectCreate`
    // hands back a handle nothing draws and the rest of the family is inert. The argument and
    // return contracts are the engine's (`docs/re/sqf-commands.tsv`); the whole family is
    // recorded as `stub` in `docs/fidelity/sqf-verified.tsv`.
    r.unary("ppEffectCreate", ARR, TypeSet::ANYTHING, |_, _| {
        let handle = PP_HANDLE.fetch_add(1, Ordering::Relaxed);
        Ok(Value::Number(handle as f32))
    });
    for left in [NUM, ARR] {
        r.unary("ppEffectDestroy", left, NOTHING, |_, _| Ok(Value::Nothing));
        r.binary("ppEffectEnable", left, BOOL, NOTHING, |_, _, _| {
            Ok(Value::Nothing)
        });
    }
    for left in [NUM, STR] {
        r.unary("ppEffectCommitted", left, BOOL, |_, _| {
            Ok(Value::Bool(true))
        });
        r.unary("ppEffectEnabled", left, BOOL, |_, _| Ok(Value::Bool(true)));
    }
    r.binary("ppEffectEnable", STR, BOOL, NOTHING, |_, _, _| {
        Ok(Value::Nothing)
    });
    r.binary("ppEffectAdjust", NUM, ARR, NOTHING, |_, _, _| {
        Ok(Value::Nothing)
    });
    r.binary("ppEffectAdjust", STR, ARR, NOTHING, |_, _, _| {
        Ok(Value::Nothing)
    });
    r.binary("ppEffectCommit", NUM, NUM, NOTHING, |_, _, _| {
        Ok(Value::Nothing)
    });
    r.binary("ppEffectCommit", ARR, NUM, NOTHING, |_, _, _| {
        Ok(Value::Nothing)
    });
    r.binary("ppEffectCommit", STR, NUM, NOTHING, |_, _, _| {
        Ok(Value::Nothing)
    });
    r.binary("ppEffectForceInNVG", NUM, BOOL, NOTHING, |_, _, _| {
        Ok(Value::Nothing)
    });
    r.unary("ppEffectCommitted", STR, BOOL, |_, _| Ok(Value::Bool(true)));
    r.unary("ppEffectEnabled", STR, BOOL, |_, _| Ok(Value::Bool(true)));

    // `getConnectedUAV`: the UAV a unit is connected to, `objNull` when it is connected to none.
    // We have no UAV terminal yet, so the answer is always the null object (#126 follow-up).
    r.unary("getConnectedUAV", OBJ, OBJ, |_, _| Ok(null_object()));

    // `layer cutFadeOut duration` / `"name" cutFadeOut duration`: hide a cut layer. We render no
    // cut layers yet, so hiding one does nothing and the named form still has to hand back a
    // layer id — `0` is the engine's "no layer" id. Recorded as `stub`.
    r.binary("cutFadeOut", NUM, NUM, NOTHING, |_, _, _| {
        Ok(Value::Nothing)
    });
    r.binary("cutFadeOut", STR, NUM, NUM, |_, _, _| {
        Ok(Value::Number(0.0))
    });

    // `getClientState`: the client's state in a network game, `"NONE"` when there is no client
    // (singleplayer and a dedicated server with nobody connected). The engine's table is
    // `NONE` 0, `CREATED` 1, `CONNECTED` 2, `LOGGED IN` 3, `MISSION SELECTED` 4, ... `NONE` is
    // what we can report until the server side of the protocol exists (#7).
    r.nular("getClientState", STR, |_ctx| Ok(Value::string("NONE")));

    // `allCutLayers`: every layer named by a cut command. We have no cut rendering yet, so the
    // list is empty — the engine's answer when none has been used.
    r.nular("allCutLayers", ARR, |_ctx| {
        Ok(Value::array(std::iter::empty()))
    });
}
