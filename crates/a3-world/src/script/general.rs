//! Engine-wide state that belongs to no object: the client's network state, the cut layers
//! (`cutRsc`, `cutText`, `cutFadeOut`), the title effects and the post-process effect family.
//! The sound commands (`fadeSound`, `soundVolume`, ...) live in `audio_cmds`.

use super::{ARR, BOOL, NOTHING, NUM, OBJ, STR, WorldHost, null_object};
use a3_sqf::{Handle, HandleKind, Registry, TypeSet, Value};
use std::sync::atomic::{AtomicI32, Ordering};

/// Handles handed out by `ppEffectCreate`. We render no post-process effects, so a handle is only
/// an identity the caller can pass back; the counter keeps two creations distinct the way the
/// engine's priorities do.
static PP_HANDLE: AtomicI32 = AtomicI32::new(1);

pub(super) fn register<H: WorldHost>(r: &mut Registry<H>) {
    // `onTeamSwitch code`: the code the engine runs when the player switches team. We have no
    // team switching, so nothing runs it. Recorded as `stub`.
    r.unary("onTeamSwitch", TypeSet::ANYTHING, NOTHING, |_, _| {
        Ok(Value::Nothing)
    });

    // `enableSaving`: there is no save system, so nothing changes (`stub`).
    r.unary("enableSaving", BOOL, NOTHING, |_, _| Ok(Value::Nothing));
    r.unary("enableSaving", ARR, NOTHING, |_, _| Ok(Value::Nothing));

    // `cutText`: we render no cut text; the layer is still reported (`stub`).
    r.unary("cutText", ARR, NOTHING, |_, _| Ok(Value::Nothing));
    r.binary("cutText", NUM, ARR, NOTHING, |_, _, _| Ok(Value::Nothing));
    r.binary("cutText", STR, ARR, NUM, |_, _, _| Ok(Value::Number(0.0)));

    // `titleCut` / `titleText`: the title-effect layers. We render no titles, so they are no-ops
    // (`stub`).
    r.unary("titleCut", ARR, NOTHING, |_, _| Ok(Value::Nothing));
    r.unary("titleText", ARR, NOTHING, |_, _| Ok(Value::Nothing));

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

    // `remoteExec` / `remoteExecCall`: run a function on the machines in `targets`. There is no
    // network (#7), and a machine already in `targets` runs the function locally in the engine,
    // so the honest headless behaviour is to run it here and report no JIP id. Until the network
    // layer decides which targets it can reach, both are recorded as `stub`.
    for name in ["remoteExec", "remoteExecCall"] {
        r.unary(name, ARR, TypeSet::ANYTHING, |_, _| Ok(Value::Nothing));
        r.binary(
            name,
            TypeSet::ANYTHING,
            ARR,
            TypeSet::ANYTHING,
            |_, _, _| Ok(Value::Nothing),
        );
    }

    // `getConnectedUAV`: the UAV a unit is connected to, `objNull` when it is connected to none.
    // We have no UAV terminal yet, so the answer is always the null object (#126 follow-up).
    r.unary("getConnectedUAV", OBJ, OBJ, |_, _| Ok(null_object()));

    // `layer cutFadeOut duration` / `"name" cutFadeOut duration`: hide a cut layer. We render no
    // cut layers yet, so hiding one does nothing and the named form still has to hand back a
    // layer id; `0` is the engine's "no layer" id. Recorded as `stub`.
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
    // list is empty, which is the engine's answer when none has been used.
    r.nular("allCutLayers", ARR, |_ctx| {
        Ok(Value::array(std::iter::empty()))
    });

    // `enableEnvironment`: ambient life (birds, insects, sea life) and the wind sound. We
    // simulate none of it, so the switch stores nothing (`stub`); the array form carries the same
    // switch plus the ambient-life flags.
    r.unary("enableEnvironment", BOOL, NOTHING, |_, _| {
        Ok(Value::Nothing)
    });
    r.unary("enableEnvironment", ARR, NOTHING, |_, _| Ok(Value::Nothing));

    // `enableSentences`: whether units say their sentences aloud. We play no sentences, so the
    // switch stores nothing (`stub`).
    r.unary("enableSentences", BOOL, NOTHING, |_, _| Ok(Value::Nothing));

    // `loadStatus object, name`: whether the object's `name` animation source has finished
    // loading. We load a model's sources with the model, so anything that exists is loaded
    // (`stub`).
    r.binary("loadStatus", OBJ, STR, BOOL, |_, _, _| {
        Ok(Value::Bool(true))
    });

    // `targetsQuery object, [targets, ...]`: what the object's sensors currently hold. We have no
    // sensor model, so the query answers nothing (`stub`).
    r.binary("targetsQuery", OBJ, ARR, ARR, |_, _, _| {
        Ok(Value::array(std::iter::empty()))
    });

    // `lockIdentity object`: whether the object's identity is locked. We keep no identity lock,
    // so the answer is "not locked" (`stub`).
    r.unary("lockIdentity", OBJ, BOOL, |_, _| Ok(Value::Bool(false)));

    // `setViewDistance metres`: the view distance the engine renders at. Our render distance is
    // a renderer setting, not a script one, so the setter stores nothing (`stub`).
    r.unary("setViewDistance", NUM, NOTHING, |_, _| Ok(Value::Nothing));

    // `setLightBrightness light, value`: the brightness of a placed light. Our lamps come from
    // `Reflectors` in the config (#294) and take no script override yet (`stub`).
    r.binary("setLightBrightness", OBJ, NUM, NOTHING, |_, _, _| {
        Ok(Value::Nothing)
    });

    // The rest of the `setLight*` family: the colour, ambient term and flare a placed light
    // contributes. Our lamps take theirs from `Reflectors` in the config (#294) and accept no
    // script override yet, so each of these stores nothing (`stub`).
    r.binary("setLightAmbient", OBJ, ARR, NOTHING, |_, _, _| {
        Ok(Value::Nothing)
    });
    r.binary("setLightColor", OBJ, ARR, NOTHING, |_, _, _| {
        Ok(Value::Nothing)
    });
    r.binary("setLightIntensity", OBJ, NUM, NOTHING, |_, _, _| {
        Ok(Value::Nothing)
    });
    r.binary("setLightUseFlare", OBJ, BOOL, NOTHING, |_, _, _| {
        Ok(Value::Nothing)
    });
    r.binary("setLightFlareSize", OBJ, NUM, NOTHING, |_, _, _| {
        Ok(Value::Nothing)
    });
    r.binary("setLightFlareMaxDistance", OBJ, NUM, NOTHING, |_, _, _| {
        Ok(Value::Nothing)
    });
    r.binary("setLightDayLight", OBJ, BOOL, NOTHING, |_, _, _| {
        Ok(Value::Nothing)
    });

    // `clearRadio`: stops the unit's radio sentences. We play no sentences, so there is nothing
    // to stop (`stub`).
    r.nular("clearRadio", NOTHING, |_ctx| Ok(Value::Nothing));
    // `enableRadio`: the radio-sentence switch, the same subsystem as `clearRadio` (`stub`).
    r.unary("enableRadio", BOOL, NOTHING, |_, _| Ok(Value::Nothing));

    // `display displayCtrl idc` / `idc displayCtrl`: the control with that id. The display stack
    // lives in a3-ui and is not reachable from the world host yet, so the lookup answers the null
    // control (`stub`). A script that only stores the handle is unaffected.
    let control = HandleKind::Control.ty();
    r.unary("displayCtrl", NUM, control, |_, _| {
        Ok(Value::Handle(Handle::null(HandleKind::Control)))
    });
    r.binary("displayCtrl", control, NUM, control, |_, _, _| {
        Ok(Value::Handle(Handle::null(HandleKind::Control)))
    });

    // `callExtension`: a call into a native extension DLL. We load no extensions, so the engine's
    // answer for an unknown extension is what we give: the empty string, or an empty array for the
    // array form (`stub`).
    r.binary("callExtension", STR, STR, STR, |_, _, _| {
        Ok(Value::string(""))
    });
    r.binary("callExtension", STR, ARR, ARR, |_, _, _| {
        Ok(Value::array(std::iter::empty()))
    });

    // `enableVehicleCargo`: whether a vehicle's cargo can be loaded. We have no loadmaster or
    // sling-load system, so the switch stores nothing (`stub`).
    r.binary("enableVehicleCargo", OBJ, BOOL, NOTHING, |_, _, _| {
        Ok(Value::Nothing)
    });

    // `removeFromRemainsCollector`: stops the engine cleaning up a body. We have no remains
    // collector, so nothing is removed from it (`stub`).
    r.unary("removeFromRemainsCollector", ARR, NOTHING, |_, _| {
        Ok(Value::Nothing)
    });

    // `setFlagTexture`: the texture on a flag pole. We draw no flags, so the setter stores nothing
    // (`stub`).
    r.binary("setFlagTexture", OBJ, STR, NOTHING, |_, _, _| {
        Ok(Value::Nothing)
    });

    // `enableWeaponDisassembly`: whether a unit can disassemble a static weapon. We have no
    // disassembly system, so both forms store nothing (`stub`).
    r.binary("enableWeaponDisassembly", OBJ, BOOL, NOTHING, |_, _, _| {
        Ok(Value::Nothing)
    });
    r.unary("enableWeaponDisassembly", BOOL, NOTHING, |_, _| {
        Ok(Value::Nothing)
    });
}
