//! Scripted FSM commands: `execFSM`, `completedFSM`, `getFSMVariable`, `setFSMVariable`,
//! `diag_activeMissionFSMs` (handlers in `docs/re/sqf-commands.tsv`: 0x48ecf0 / 0x48eda0 /
//! 0x48eea0 / 0x48f020, 0x48fa30, 0x48f5d0, 0x48fb20, 0x8a5f00). The runtime: `crate::fsm`.

use std::rc::Rc;

use a3_fsm::Fsm;

use super::{ANY, ARR, BOOL, NOTHING, NUM, STR, array, boolean, num, string};
use crate::error::SqfError;
use crate::fsm::CompiledFsm;
use crate::host::Host;
use crate::registry::Registry;
use crate::types::TypeSet;
use crate::value::Value;
use crate::vm::Ctx;

const STR_ARR: TypeSet = STR.union(ARR);

pub(crate) fn register<H: Host>(r: &mut Registry<H>) {
    r.unary("execFSM", STR, NUM, |ctx, a| exec_fsm(ctx, Value::Nil, &a));
    r.unary("execFSM", ARR, NUM, |ctx, a| exec_fsm(ctx, Value::Nil, &a));
    r.binary("execFSM", ANY, STR, NUM, |ctx, a, b| exec_fsm(ctx, a, &b));
    r.binary("execFSM", ANY, ARR, NUM, |ctx, a, b| exec_fsm(ctx, a, &b));
    r.unary("completedFSM", NUM, BOOL, |ctx, a| {
        Ok(Value::Bool(ctx.vm.fsms.is_completed(handle(&a))))
    });
    r.binary("getFSMVariable", NUM, STR_ARR, ANY, |ctx, a, b| {
        let (name, default) = match &b {
            Value::Array(items) => {
                let items = items.borrow();
                (
                    items
                        .first()
                        .map(|v| string(v).to_owned())
                        .unwrap_or_default(),
                    items.get(1).cloned(),
                )
            }
            other => (string(other).to_owned(), None),
        };
        Ok(match ctx.vm.fsms.variable(handle(&a), &name) {
            Some(value) if !matches!(value, Value::Nil) => value,
            _ => default.unwrap_or(Value::Nil),
        })
    });
    r.binary("setFSMVariable", NUM, ARR, NOTHING, |ctx, a, b| {
        let items = array(&b);
        let items = items.borrow();
        if let Some(name) = items.first() {
            let value = items.get(1).cloned().unwrap_or(Value::Nil);
            ctx.vm.fsms.set_variable(handle(&a), string(name), value);
        }
        Ok(Value::Nothing)
    });
    r.nular("diag_activeMissionFSMs", ARR, |ctx| {
        Ok(Value::array(ctx.vm.fsms.active().into_iter().map(
            |(name, state)| {
                Value::array([
                    Value::from(&*name),
                    Value::from(state.as_str()),
                    Value::Number(0.0),
                ])
            },
        )))
    });
}

fn handle(value: &Value) -> u32 {
    num(value).max(0.0) as u32
}

/// `execFSM`: loads (once) and starts the FSM file; the handle, or 0 when the file cannot be
/// read or is not an FSM. `[path, allowTermination]` is accepted; termination of FSMs by
/// `terminate` is not modelled.
fn exec_fsm<H: Host>(ctx: &mut Ctx<'_, H>, this: Value, file: &Value) -> Result<Value, SqfError> {
    let path = match file {
        Value::Array(items) => {
            let items = items.borrow();
            let path = items
                .first()
                .map(|v| string(v).to_owned())
                .unwrap_or_default();
            let _allow_termination = items.get(1).is_some_and(boolean);
            path
        }
        other => string(other).to_owned(),
    };
    let Some(compiled) = load_fsm(ctx, &path) else {
        return Ok(Value::Number(0.0));
    };
    Ok(Value::Number(
        ctx.vm.fsms.start(compiled, this, &path) as f32
    ))
}

/// The compiled FSM of a file, read and compiled on first use.
pub(crate) fn load_fsm<H: Host>(ctx: &mut Ctx<'_, H>, path: &str) -> Option<Rc<CompiledFsm>> {
    if let Some(compiled) = ctx.vm.fsms.cached(path) {
        return Some(compiled);
    }
    let text = match ctx.host.preprocess_file(path, false) {
        Ok(text) => text,
        Err(e) => {
            ctx.report(SqfError::Generic(e));
            return None;
        }
    };
    let loaded = match Fsm::parse_scripted(&text) {
        Ok(loaded) => loaded,
        Err(e) => {
            ctx.report(SqfError::Generic(format!("{path}: {e}")));
            return None;
        }
    };
    let compiled = CompiledFsm::new(loaded.fsm, path, ctx.table());
    for error in loaded.warnings.iter().chain(compiled.errors()) {
        ctx.report(SqfError::Generic(error.clone()));
    }
    let compiled = Rc::new(compiled);
    ctx.vm.fsms.cache(path, compiled.clone());
    Some(compiled)
}
