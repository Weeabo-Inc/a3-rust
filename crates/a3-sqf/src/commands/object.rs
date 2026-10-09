//! Hash map objects (`createHashMapObject`, Arma 3 2.14).
//!
//! A class definition is an array of `[name, value]` pairs or a hash map.
//! Reserved entries: `#create`, `#clone`, `#delete`, `#str` (code), `#flags`
//! (`sealed`, `noCopy`, `unscheduled`), `#base` (a base definition) and
//! `#type`. A base is merged first and the class overwrites its entries;
//! `#create`/`#clone`/`#delete` of base and class run in sequence (base
//! first) and the `#type` values collect into an array. Methods run with
//! `_self` set to the object: `obj call ["method", args]`.
//!
//! Not implemented: running `#delete` when the last reference goes away.

use std::rc::Rc;

use crate::value::HashMapEntries;

use super::*;
use crate::symbol::Sym;
use crate::value::{HashKey, HashMap, ObjectInfo, deep_copy_value};
use crate::vm::{Continuation, Ctx, Flow, Invoke};

fn self_sym() -> Sym {
    Sym::new("_self")
}

/// Pairs of a class definition (array of pairs or hash map).
fn pairs(decl: &Value) -> Result<Vec<(Value, Value)>, SqfError> {
    match decl {
        Value::HashMap(m) => Ok(m
            .borrow()
            .iter()
            .map(|(k, v)| (k.to_value(), v.clone()))
            .collect()),
        Value::Array(a) => {
            let mut out = Vec::new();
            for p in a.borrow().iter() {
                let Value::Array(p) = p else {
                    return Err(SqfError::type_error(p, ARR));
                };
                let p = p.borrow();
                out.push((
                    p.first().cloned().unwrap_or(Value::Nil),
                    p.get(1).cloned().unwrap_or(Value::Nil),
                ));
            }
            Ok(out)
        }
        other => Err(SqfError::type_error(other, ARR | Type::HashMap)),
    }
}

/// Merges a definition (and its bases) into `map`, `info` and `types`.
fn resolve(
    decl: &Value,
    map: &mut HashMapEntries,
    info: &mut ObjectInfo,
    types: &mut Vec<Value>,
    depth: usize,
) -> Result<(), SqfError> {
    if depth > 32 {
        return Err(SqfError::generic("HashMap object base chain too deep"));
    }
    let entries = pairs(decl)?;
    if let Some((_, base)) = entries
        .iter()
        .find(|(k, _)| k.as_str().is_some_and(|s| s.eq_ignore_ascii_case("#base")))
    {
        resolve(base, map, info, types, depth + 1)?;
    }
    for (k, v) in entries {
        let name = k.as_str().map(str::to_ascii_lowercase);
        match name.as_deref() {
            Some("#create") => info.create.push(expect_code(&v)?),
            Some("#clone") => info.clone.push(expect_code(&v)?),
            Some("#delete") => info.delete.push(expect_code(&v)?),
            Some("#str") => info.str_code = Some(expect_code(&v)?),
            Some("#flags") => {
                info.sealed = false;
                info.no_copy = false;
                info.unscheduled = false;
                for f in array(&v).borrow().iter() {
                    match f.as_str().map(str::to_ascii_lowercase).as_deref() {
                        Some("sealed") => info.sealed = true,
                        Some("nocopy") => info.no_copy = true,
                        Some("unscheduled") => info.unscheduled = true,
                        _ => {}
                    }
                }
            }
            Some("#type") => types.push(v.clone()),
            _ => {}
        }
        if name.as_deref() == Some("#type") {
            continue;
        }
        let key = HashKey::from_value(&k).ok_or_else(|| SqfError::type_error(&k, STR))?;
        map.insert(key, v);
    }
    Ok(())
}

/// Runs a list of methods in order with `_self` and `_this`, then yields
/// `result`.
struct RunMethods {
    target: HashMap,
    codes: Vec<Code>,
    index: usize,
    this: Value,
    result: Value,
}

impl RunMethods {
    fn next<H: Host>(&mut self) -> Flow<H> {
        match self.codes.get(self.index) {
            Some(code) => {
                self.index += 1;
                Flow::Call(
                    Invoke::with_this(code.clone(), self.this.clone())
                        .local(self_sym(), Value::HashMap(self.target.clone())),
                )
            }
            None => Flow::Value(self.result.clone()),
        }
    }

    fn start<H: Host>(mut self) -> Flow<H> {
        match self.next::<H>() {
            Flow::Call(inv) => Flow::CallThen(inv, Box::new(self)),
            other => other,
        }
    }
}

impl<H: Host> Continuation<H> for RunMethods {
    fn resume(&mut self, _: &mut Ctx<'_, H>, _: Value) -> Result<Flow<H>, SqfError> {
        Ok(self.next())
    }
}

/// Runs `codes` with `_self`, unscheduled when the object asks for it and
/// the caller is scheduled; otherwise as continuations.
fn run_methods<H: Host>(
    ctx: &mut Ctx<'_, H>,
    target: HashMap,
    codes: Vec<Code>,
    this: Value,
    result: Value,
    unscheduled: bool,
) -> Result<Flow<H>, SqfError> {
    if unscheduled && ctx.is_scheduled() {
        let mut last = Value::Nothing;
        for code in &codes {
            last = ctx
                .call_unscheduled_with_locals(
                    code,
                    Some(this.clone()),
                    vec![(self_sym(), Value::HashMap(target.clone()))],
                )
                .map_err(|e| e.error)?;
        }
        let _ = last;
        return Ok(Flow::Value(result));
    }
    Ok(RunMethods {
        target,
        codes,
        index: 0,
        this,
        result,
    }
    .start())
}

pub(super) fn register<H: Host>(r: &mut Registry<H>) {
    r.unary_flow("createHashMapObject", ARR, HASH, |ctx, a| {
        let args = array(&a).borrow().clone();
        let decl = args.first().cloned().unwrap_or(Value::Nil);
        let this = args.get(1).cloned().unwrap_or(Value::Nothing);
        let mut map = HashMapEntries::default();
        let mut info = ObjectInfo::default();
        let mut types = Vec::new();
        resolve(&decl, &mut map, &mut info, &mut types, 0)?;
        if !types.is_empty() {
            map.insert(HashKey::String("#type".into()), Value::array(types));
        }
        let object = HashMap::from_map(map);
        let codes = info.create.clone();
        let unscheduled = info.unscheduled;
        object.set_object(Rc::new(info));
        let result = Value::HashMap(object.clone());
        run_methods(ctx, object, codes, this, result, unscheduled)
    });
    r.binary_flow("call", HASH, ARR, ANY, |ctx, a, b| {
        let Value::HashMap(object) = a else {
            unreachable!()
        };
        let args = array(&b).borrow().clone();
        let name = expect_str(args.first().unwrap_or(&Value::Nil))?.to_string();
        let method = object
            .borrow()
            .get(&HashKey::String(name.as_str().into()))
            .cloned();
        let Some(Value::Code(code)) = method else {
            return Err(SqfError::generic(format!("Method {name} not found")));
        };
        let this = args.get(1).cloned().unwrap_or(Value::Nothing);
        let unscheduled = object.object().is_some_and(|o| o.unscheduled);
        if unscheduled && ctx.is_scheduled() {
            let v = ctx
                .call_unscheduled_with_locals(
                    &code,
                    Some(this),
                    vec![(self_sym(), Value::HashMap(object))],
                )
                .map_err(|e| e.error)?;
            return Ok(Flow::Value(v));
        }
        Ok(Flow::Call(
            Invoke::with_this(code, this).local(self_sym(), Value::HashMap(object)),
        ))
    });
    // `+object`: copy, then run `#clone` on the copy.
    r.unary_flow("+", HASH, HASH, |ctx, a| {
        let Value::HashMap(m) = a else { unreachable!() };
        let info = m.object();
        if info.as_ref().is_some_and(|o| o.no_copy) {
            return Err(SqfError::generic(
                "Copy of noCopy HashMap object is not allowed",
            ));
        }
        let copy = HashMap::new();
        {
            let src = m.borrow();
            let mut dst = copy.borrow_mut();
            for (k, v) in src.iter() {
                dst.insert(k.clone(), deep_copy_value(v));
            }
        }
        let Some(info) = info else {
            return Ok(Flow::Value(Value::HashMap(copy)));
        };
        copy.set_object(info.clone());
        let result = Value::HashMap(copy.clone());
        run_methods(
            ctx,
            copy,
            info.clone.clone(),
            Value::Nothing,
            result,
            info.unscheduled,
        )
    });
}

/// `str` of a hash map object with a `#str` method.
pub(crate) fn object_str<H: Host>(
    ctx: &mut Ctx<'_, H>,
    v: &Value,
) -> Option<Result<String, SqfError>> {
    let Value::HashMap(m) = v else {
        return None;
    };
    let code = m.object()?.str_code.clone()?;
    Some(
        ctx.call_unscheduled_with_locals(
            &code,
            None,
            vec![(self_sym(), Value::HashMap(m.clone()))],
        )
        .map_err(|e| e.error)
        .and_then(|r| match r {
            Value::String(s) => Ok(s.to_string()),
            other => Err(SqfError::type_error(&other, STR)),
        }),
    )
}
