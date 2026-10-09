//! Engine-wide state that belongs to no object: the client's network state and the named cut
//! layers (`cutRsc`, `cutText`, `cutObj`, `cutFadeOut`).

use super::{ARR, NOTHING, NUM, OBJ, STR, WorldHost, null_object};
use a3_sqf::{Registry, Value};

pub(super) fn register<H: WorldHost>(r: &mut Registry<H>) {
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
