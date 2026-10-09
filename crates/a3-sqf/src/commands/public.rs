//! `publicVariable` and its family: publishing a `missionNamespace`
//! variable to the network, and the handlers for broadcasts that arrive.
//!
//! The four commands are thin wrappers over the engine's network manager
//! (`tools/oracle/probes/15_publicvariable.probes` is the recorded contract,
//! and the handlers are `FUN_140547370`, `FUN_1405474b0`, `FUN_1405473f0`
//! and `FUN_1401809f0`):
//!
//! - All four return `Nothing`, and all four hoist their `String` argument,
//!   substituting the empty string when it has no data.
//! - Publishing an undefined variable is **not** an error: the manager
//!   resolves the name itself, so `publicVariable "never_set"` is a no-op.
//! - An empty name is the error `Reserved variable in expression`
//!   (code BAD_VAR, logged, the script goes on).
//! - `publicVariableClient` looks the client up by rounded id and does
//!   nothing at all when there is no such client; a bad id is not an error.
//! - The event handler fires only for a broadcast this machine *receives*,
//!   never on the machine that published.
//! - A wrong argument type fails the VM's type check, which ends the script
//!   (the messages in the probe file are the engine's).

use super::*;
use crate::host::PublicTarget;
use crate::vm::Ctx;

/// The engine's "Reserved variable in expression" (error code BAD_VAR, 5).
const RESERVED: &str = "Reserved variable in expression";

/// `publicVariable name`: publish to every machine, including the server.
fn publish<H: Host>(
    ctx: &mut Ctx<'_, H>,
    name: &str,
    target: PublicTarget,
) -> Result<Value, SqfError> {
    if name.is_empty() {
        // Logged, and the command still yields Nothing (server oracle:
        // `publicVariable ""` continues with "ok").
        ctx.report(SqfError::generic(RESERVED));
        return Ok(Value::Nothing);
    }
    ctx.host.publish_variable(name, target);
    Ok(Value::Nothing)
}

pub(super) fn register<H: Host>(r: &mut Registry<H>) {
    r.unary("publicVariable", STR, NOTHING, |ctx, a| {
        publish(ctx, string(&a), PublicTarget::All)
    });
    r.unary("publicVariableServer", STR, NOTHING, |ctx, a| {
        publish(ctx, string(&a), PublicTarget::Server)
    });
    // `clientID publicVariableClient varName`: the id is rounded before the
    // lookup (`ROUND` in FUN_1405473f0); an unknown client is a silent no-op.
    r.binary("publicVariableClient", SCALAR, STR, NOTHING, |ctx, a, b| {
        let id = num(&a).round() as i32;
        publish(ctx, string(&b), PublicTarget::Client(id))
    });
    // `varName addPublicVariableEventHandler code`.
    r.binary(
        "addPublicVariableEventHandler",
        STR,
        CODE,
        NOTHING,
        |ctx, a, b| {
            let code = expect_code(&b)?;
            ctx.register_public_handler(string(&a), None, code);
            Ok(Value::Nothing)
        },
    );
    // `varName addPublicVariableEventHandler [target, code]`.
    r.binary(
        "addPublicVariableEventHandler",
        STR,
        ARR,
        NOTHING,
        |ctx, a, b| {
            let args = array(&b);
            let args = args.borrow().clone();
            let target = args.first().cloned().unwrap_or(Value::Nothing);
            // The engine checks the target first: an array whose first
            // element is not an object, group or namespace reports
            // "Type code, expected Object, Group, Namespace" for `[{1}]`.
            if !matches!(
                target.ty(),
                Type::Object | Type::Group | Type::Namespace | Type::TeamMember
            ) {
                ctx.report(SqfError::type_error(
                    &target,
                    Type::Object | Type::Group | Type::Namespace,
                ));
                return Ok(Value::Nothing);
            }
            let code = match args.get(1) {
                Some(v) => expect_code(v)?,
                None => {
                    ctx.report(SqfError::type_error(&Value::Nothing, CODE));
                    return Ok(Value::Nothing);
                }
            };
            ctx.register_public_handler(string(&a), Some(target), code);
            Ok(Value::Nothing)
        },
    );
}
