//! Variables and namespaces: `getVariable`, `setVariable`, `private`,
//! `params`, `param`, the namespace commands.
//!
//! `params` and `param` report a wrong type or element count as an error
//! but do not abort the script: the default value is used and `params`
//! returns `false`.

use super::*;
use crate::symbol::Sym;
use crate::value::Namespace;
use crate::vm::Ctx;

pub(super) fn register<H: Host>(r: &mut Registry<H>) {
    r.nular("missionNamespace", NS, |_| {
        Ok(Value::Namespace(Namespace::Mission))
    });
    r.nular("uiNamespace", NS, |_| Ok(Value::Namespace(Namespace::Ui)));
    r.nular("profileNamespace", NS, |_| {
        Ok(Value::Namespace(Namespace::Profile))
    });
    r.nular("parsingNamespace", NS, |_| {
        Ok(Value::Namespace(Namespace::Parsing))
    });
    r.nular("localNamespace", NS, |_| {
        Ok(Value::Namespace(Namespace::Local))
    });
    r.nular("missionProfileNamespace", NS, |_| {
        Ok(Value::Namespace(Namespace::MissionProfile))
    });
    r.nular("serverNamespace", NS, |_| {
        Ok(Value::Namespace(Namespace::Server))
    });
    r.nular("currentNamespace", NS, |ctx| {
        Ok(Value::Namespace(ctx.current_namespace()))
    });

    r.binary("getVariable", NS, STR, ANY, |ctx, a, b| {
        let Value::Namespace(ns) = a else {
            unreachable!()
        };
        Ok(ctx
            .namespace(ns)
            .get(Sym::new(string(&b)))
            .cloned()
            .unwrap_or(Value::Nil))
    });
    r.binary("getVariable", NS, ARR, ANY, |ctx, a, b| {
        let Value::Namespace(ns) = a else {
            unreachable!()
        };
        let args = array(&b);
        let args = args.borrow();
        let name = expect_str(args.first().unwrap_or(&Value::Nil))?;
        let default = args.get(1).cloned().unwrap_or(Value::Nil);
        Ok(ctx
            .namespace(ns)
            .get(Sym::new(name))
            .cloned()
            .unwrap_or(default))
    });
    r.binary("setVariable", NS, ARR, NOTHING, |ctx, a, b| {
        let Value::Namespace(ns) = a else {
            unreachable!()
        };
        let args = array(&b);
        let args = args.borrow();
        let name = Sym::new(expect_str(args.first().unwrap_or(&Value::Nil))?);
        let value = args.get(1).cloned().unwrap_or(Value::Nil);
        let vars = ctx.namespace_mut(ns);
        if let Some(Value::Code(c)) = vars.get(name) {
            if c.is_final() {
                return Err(SqfError::generic(format!(
                    "Attempt to override final function - {name}"
                )));
            }
        }
        vars.set(name, value);
        Ok(Value::Nothing)
    });
    r.unary("allVariables", NS, ARR, |ctx, a| {
        let Value::Namespace(ns) = a else {
            unreachable!()
        };
        let mut names: Vec<&str> = ctx.namespace(ns).iter().map(|(k, _)| k.as_str()).collect();
        names.sort_unstable();
        Ok(Value::array(names.into_iter().map(Value::from)))
    });

    r.unary("private", STR, NOTHING, |ctx, a| {
        declare_private(ctx, &a)?;
        Ok(Value::Nothing)
    });
    r.unary("private", ARR, NOTHING, |ctx, a| {
        for v in array(&a).borrow().iter() {
            declare_private(ctx, v)?;
        }
        Ok(Value::Nothing)
    });

    r.unary("params", ARR, BOOL, |ctx, a| {
        let this = ctx.get_var(Sym::THIS);
        params(ctx, this, &a)
    });
    r.binary("params", ANY, ARR, BOOL, |ctx, a, b| params(ctx, a, &b));
    r.unary("param", ARR, ANY, |ctx, a| {
        let this = ctx.get_var(Sym::THIS);
        param(ctx, this, &a)
    });
    r.binary("param", ANY, ARR, ANY, |ctx, a, b| param(ctx, a, &b));
}

fn declare_private<H: Host>(ctx: &mut Ctx<'_, H>, name: &Value) -> Result<(), SqfError> {
    let name = expect_str(name)?;
    if !name.starts_with('_') {
        // The engine logs this and goes on: the variable is not declared.
        ctx.report(SqfError::generic("Local variable in global space"));
        return Ok(());
    }
    ctx.declare_private(Sym::new(name));
    Ok(())
}

/// The input of `params`/`param`: an array, or a single value wrapped in
/// one.
fn input_items(input: &Value) -> Vec<Value> {
    match input {
        Value::Array(a) => a.borrow().clone(),
        Value::Nil => Vec::new(),
        other => vec![other.clone()],
    }
}

/// Checks a value against `params` type and count rules. Returns the error
/// to report, if any.
fn check_value(value: &Value, types: Option<&Value>, counts: Option<&Value>) -> Option<SqfError> {
    if let Some(Value::Array(types)) = types {
        let types = types.borrow();
        if !types.is_empty() && !value.is_nil() {
            let ty = value.ty();
            let ok = types.iter().any(|t| t.ty() == ty);
            if !ok {
                let expected = types
                    .iter()
                    .fold(TypeSet::EMPTY, |acc, t| acc.union(TypeSet::exactly(t.ty())));
                return Some(SqfError::Generic(format!(
                    "Params: Type {}, expected {}",
                    ty,
                    expected.display_list()
                )));
            }
        }
    }
    if let (Some(counts), Value::Array(arr)) = (counts, value) {
        let n = arr.len();
        let allowed: Vec<usize> = match counts {
            Value::Number(c) => vec![*c as usize],
            Value::Array(cs) => cs.borrow().iter().map(|c| num(c) as usize).collect(),
            _ => Vec::new(),
        };
        if !allowed.is_empty() && !allowed.contains(&n) {
            let list = allowed
                .iter()
                .map(|c| c.to_string())
                .collect::<Vec<_>>()
                .join(",");
            return Some(SqfError::Generic(format!(
                "{n} elements provided, {list} expected"
            )));
        }
    }
    None
}

/// Resolves one `params` element spec against the input value at `i`.
/// Returns `(name, value, valid)`.
fn resolve<H: Host>(
    ctx: &mut Ctx<'_, H>,
    spec: &Value,
    input: &[Value],
    i: usize,
) -> Result<(Option<Sym>, Value, bool), SqfError> {
    let given = input.get(i).cloned().unwrap_or(Value::Nil);
    match spec {
        Value::String(name) => {
            let name = (!name.is_empty()).then(|| Sym::new(name));
            Ok((name, given, true))
        }
        Value::Array(parts) => {
            let parts = parts.borrow();
            let name = match parts.first() {
                Some(Value::String(s)) if !s.is_empty() => Some(Sym::new(s)),
                Some(Value::String(_)) => None,
                Some(other) => return Err(SqfError::type_error(other, STR)),
                None => None,
            };
            let default = parts.get(1).cloned().unwrap_or(Value::Nil);
            if given.is_nil() {
                return Ok((name, default, true));
            }
            match check_value(&given, parts.get(2), parts.get(3)) {
                None => Ok((name, given, true)),
                Some(err) => {
                    ctx.report(err);
                    Ok((name, default, false))
                }
            }
        }
        other => Err(SqfError::type_error(other, STR | Type::Array)),
    }
}

fn params<H: Host>(ctx: &mut Ctx<'_, H>, input: Value, specs: &Value) -> Result<Value, SqfError> {
    let input = input_items(&input);
    let specs = array(specs).borrow().clone();
    let mut all_valid = true;
    for (i, spec) in specs.iter().enumerate() {
        let (name, value, valid) = resolve(ctx, spec, &input, i)?;
        all_valid &= valid;
        if let Some(name) = name {
            if !name.is_local() {
                // Logged, not fatal: the rest of the block still runs
                // (server oracle: `[1] call {params ["a"]; 1}` is 1).
                ctx.report(SqfError::generic("Local variable in global space"));
                all_valid = false;
                continue;
            }
            ctx.set_private(name, value);
        }
    }
    Ok(Value::Bool(all_valid))
}

fn param<H: Host>(ctx: &mut Ctx<'_, H>, input: Value, spec: &Value) -> Result<Value, SqfError> {
    let input = input_items(&input);
    let spec = array(spec);
    let spec = spec.borrow();
    let index = spec.first().map(num).unwrap_or(0.0).max(0.0) as usize;
    let default = spec.get(1).cloned().unwrap_or(Value::Nil);
    let given = input.get(index).cloned().unwrap_or(Value::Nil);
    if given.is_nil() {
        return Ok(default);
    }
    match check_value(&given, spec.get(2), spec.get(3)) {
        None => Ok(given),
        Some(err) => {
            ctx.report(err);
            Ok(default)
        }
    }
}
