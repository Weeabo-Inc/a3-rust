//! Hash map commands.
//!
//! Keys may be numbers, strings (case-sensitive), booleans, code, sides,
//! namespaces, config entries, or arrays of those; arrays are copied into
//! the key. A map made read-only by `compileFinal` rejects changes.

use indexmap::IndexMap;

use super::*;
use crate::value::{HashKey, HashMap};
use crate::vm::{Continuation, Ctx, Flow, Invoke};

/// Types accepted as keys, for error messages.
fn key_types() -> TypeSet {
    NUM | Type::String
        | Type::Bool
        | Type::Code
        | Type::Side
        | Type::Namespace
        | Type::Config
        | Type::Array
}

fn key(v: &Value) -> Result<HashKey, SqfError> {
    HashKey::from_value(v).ok_or_else(|| SqfError::type_error(v, key_types()))
}

fn map(v: &Value) -> HashMap {
    match v {
        Value::HashMap(m) => m.clone(),
        _ => HashMap::new(),
    }
}

fn writable(m: &HashMap) -> Result<(), SqfError> {
    if m.is_read_only() {
        Err(SqfError::generic("Tried to edit Read-only value"))
    } else {
        Ok(())
    }
}

/// `getOrDefaultCall`: stores the computed default when asked to.
struct StoreDefault {
    map: HashMap,
    key: HashKey,
    store: bool,
}

impl<H: Host> Continuation<H> for StoreDefault {
    fn resume(&mut self, _: &mut Ctx<'_, H>, result: Value) -> Result<Flow<H>, SqfError> {
        if self.store && !result.is_nil() {
            writable(&self.map)?;
            self.map
                .borrow_mut()
                .insert(self.key.clone(), result.clone());
        }
        Ok(Flow::Value(result))
    }
}

pub(super) fn register<H: Host>(r: &mut Registry<H>) {
    r.nular("createHashMap", HASH, |_| {
        Ok(Value::HashMap(HashMap::new()))
    });
    r.unary("createHashMapFromArray", ARR, HASH, |_, a| {
        let mut out = IndexMap::new();
        for pair in array(&a).borrow().iter() {
            let Value::Array(pair) = pair else {
                return Err(SqfError::type_error(pair, ARR));
            };
            let pair = pair.borrow();
            let k = key(pair.first().unwrap_or(&Value::Nil))?;
            out.insert(k, pair.get(1).cloned().unwrap_or(Value::Nil));
        }
        Ok(Value::HashMap(HashMap::from_map(out)))
    });
    r.binary("createHashMapFromArray", ARR, ARR, HASH, |_, a, b| {
        let (keys, values) = (array(&a), array(&b));
        let (keys, values) = (keys.borrow(), values.borrow());
        let mut out = IndexMap::new();
        for (i, k) in keys.iter().enumerate() {
            out.insert(key(k)?, values.get(i).cloned().unwrap_or(Value::Nil));
        }
        Ok(Value::HashMap(HashMap::from_map(out)))
    });
    r.binary("set", HASH, ARR, BOOL, |_, a, b| {
        let m = map(&a);
        writable(&m)?;
        let args = array(&b);
        let args = args.borrow();
        let k = key(args.first().unwrap_or(&Value::Nil))?;
        let v = args.get(1).cloned().unwrap_or(Value::Nil);
        let insert_only = args.get(2).is_some_and(boolean);
        let mut entries = m.borrow_mut();
        let existed = entries.contains_key(&k);
        if !(insert_only && existed) {
            entries.insert(k, v);
        }
        Ok(Value::Bool(existed))
    });
    r.binary("get", HASH, ANY, ANY, |_, a, b| {
        let k = key(&b)?;
        Ok(map(&a).borrow().get(&k).cloned().unwrap_or(Value::Nil))
    });
    r.binary("getOrDefault", HASH, ARR, ANY, |_, a, b| {
        let m = map(&a);
        let args = array(&b);
        let args = args.borrow();
        let k = key(args.first().unwrap_or(&Value::Nil))?;
        if let Some(v) = m.borrow().get(&k) {
            return Ok(v.clone());
        }
        let default = args.get(1).cloned().unwrap_or(Value::Nil);
        if args.get(2).is_some_and(boolean) && !default.is_nil() {
            writable(&m)?;
            m.borrow_mut().insert(k, default.clone());
        }
        Ok(default)
    });
    r.binary_flow("getOrDefaultCall", HASH, ARR, ANY, |_, a, b| {
        let m = map(&a);
        let args = array(&b);
        let args = args.borrow();
        let k = key(args.first().unwrap_or(&Value::Nil))?;
        if let Some(v) = m.borrow().get(&k) {
            return Ok(Flow::Value(v.clone()));
        }
        let code = expect_code(args.get(1).unwrap_or(&Value::Nil))?;
        let store = args.get(2).is_some_and(boolean);
        Ok(Flow::CallThen(
            Invoke::new(code),
            Box::new(StoreDefault {
                map: m,
                key: k,
                store,
            }),
        ))
    });
    r.binary("deleteAt", HASH, ANY, ANY, |_, a, b| {
        let m = map(&a);
        writable(&m)?;
        let k = key(&b)?;
        Ok(m.borrow_mut().shift_remove(&k).unwrap_or(Value::Nil))
    });
    r.binary("in", ANY, HASH, BOOL, |_, a, b| {
        let Some(k) = HashKey::from_value(&a) else {
            return Ok(Value::Bool(false));
        };
        Ok(Value::Bool(map(&b).borrow().contains_key(&k)))
    });
    r.unary("keys", HASH, ARR, |_, a| {
        Ok(Value::array(map(&a).borrow().keys().map(HashKey::to_value)))
    });
    r.unary("values", HASH, ARR, |_, a| {
        Ok(Value::array(map(&a).borrow().values().cloned()))
    });
    r.unary("count", HASH, NUM, |_, a| {
        Ok(Value::Number(map(&a).len() as f32))
    });
    r.unary("toArray", HASH, ARR, |_, a| {
        let m = map(&a);
        let m = m.borrow();
        Ok(Value::array([
            Value::array(m.keys().map(HashKey::to_value)),
            Value::array(m.values().cloned()),
        ]))
    });
    r.binary("merge", HASH, HASH, NOTHING, |_, a, b| {
        merge(&map(&a), &map(&b), false)?;
        Ok(Value::Nothing)
    });
    r.binary("merge", HASH, ARR, NOTHING, |_, a, b| {
        let args = array(&b);
        let args = args.borrow();
        let Some(Value::HashMap(other)) = args.first() else {
            return Err(SqfError::type_error(
                args.first().unwrap_or(&Value::Nil),
                HASH,
            ));
        };
        merge(&map(&a), other, args.get(1).is_some_and(boolean))?;
        Ok(Value::Nothing)
    });
    r.binary("insert", HASH, ARR, BOOL, |_, a, b| {
        let m = map(&a);
        writable(&m)?;
        let args = array(&b);
        let args = args.borrow();
        let (insert_only, pairs) = match args.first() {
            Some(Value::Bool(flag)) => (*flag, args.get(1).cloned().unwrap_or(Value::Nil)),
            _ => (false, Value::Array(Array::from_vec(args.clone()))),
        };
        let mut entries = m.borrow_mut();
        for pair in array(&pairs).borrow().iter() {
            let Value::Array(pair) = pair else {
                return Err(SqfError::type_error(pair, ARR));
            };
            let pair = pair.borrow();
            let k = key(pair.first().unwrap_or(&Value::Nil))?;
            if insert_only && entries.contains_key(&k) {
                continue;
            }
            entries.insert(k, pair.get(1).cloned().unwrap_or(Value::Nil));
        }
        Ok(Value::Bool(true))
    });
    r.unary("compileFinal", HASH, HASH, |_, a| {
        let m = map(&a);
        let copy = HashMap::from_map(m.borrow().clone());
        copy.set_read_only();
        Ok(Value::HashMap(copy))
    });
    r.unary("isFinal", HASH, BOOL, |_, a| {
        Ok(Value::Bool(map(&a).is_read_only()))
    });
}

fn merge(target: &HashMap, other: &HashMap, overwrite: bool) -> Result<(), SqfError> {
    writable(target)?;
    if target.ptr_eq(other) {
        return Ok(());
    }
    let src = other.borrow();
    let mut dst = target.borrow_mut();
    for (k, v) in src.iter() {
        if overwrite || !dst.contains_key(k) {
            dst.insert(k.clone(), v.clone());
        }
    }
    Ok(())
}
