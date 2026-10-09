//! Hash map commands.
//!
//! Keys may be numbers, strings (case-sensitive), booleans, code, sides,
//! namespaces, config entries, or arrays of those; arrays are copied into
//! the key. A map made read-only by `compileFinal` rejects changes.

use crate::value::HashMapEntries;

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

/// Refuses adding `k` to a sealed hash map object.
fn may_add(m: &HashMap, entries: &HashMapEntries, k: &HashKey) -> Result<(), SqfError> {
    if m.is_sealed() && !entries.contains_key(k) {
        Err(SqfError::generic("Tried to add key to sealed HashMap"))
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
            let mut entries = self.map.borrow_mut();
            may_add(&self.map, &entries, &self.key)?;
            entries.insert(self.key.clone(), result.clone());
        }
        Ok(Flow::Value(result))
    }
}

pub(super) fn register<H: Host>(r: &mut Registry<H>) {
    r.nular("createHashMap", HASH, |_| {
        Ok(Value::HashMap(HashMap::new()))
    });
    r.unary("createHashMapFromArray", ARR, HASH, |_, a| {
        let mut out = HashMapEntries::default();
        for pair in array(&a).borrow().iter() {
            let Value::Array(pair) = pair else {
                return Err(SqfError::type_error(pair, ARR));
            };
            let pair = pair.borrow();
            let k = key(pair.first().unwrap_or(&Value::Nil))?;
            out.insert(k, pair.get(1).cloned().unwrap_or(Value::Nothing));
        }
        Ok(Value::HashMap(HashMap::from_map(out)))
    });
    r.binary("createHashMapFromArray", ARR, ARR, HASH, |_, a, b| {
        let (keys, values) = (array(&a), array(&b));
        let (keys, values) = (keys.borrow(), values.borrow());
        let mut out = HashMapEntries::default();
        for (i, k) in keys.iter().enumerate() {
            out.insert(key(k)?, values.get(i).cloned().unwrap_or(Value::Nothing));
        }
        Ok(Value::HashMap(HashMap::from_map(out)))
    });
    // `set [key, value, onlyIfNotExists]` returns whether an existing value
    // was overwritten: true when the key was already there and `insertOnly`
    // was not asked for (server oracle: new key -> false, existing key ->
    // true, existing key with insertOnly -> false).
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
            may_add(&m, &entries, &k)?;
            entries.insert(k, v);
        }
        Ok(Value::Bool(existed && !insert_only))
    });
    r.binary("get", HASH, ANY, ANY, |_, a, b| {
        let k = key(&b)?;
        Ok(map(&a).borrow().get(&k).cloned().unwrap_or(Value::Nothing))
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
            let mut entries = m.borrow_mut();
            may_add(&m, &entries, &k)?;
            entries.insert(k, default.clone());
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
        if m.is_sealed() && m.borrow().contains_key(&k) {
            return Err(SqfError::generic("Tried to remove key from sealed HashMap"));
        }
        Ok(m.borrow_mut().shift_remove(&k).unwrap_or(Value::Nothing))
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
    r.binary("toArray", HASH, BOOL, ARR, |_, a, b| {
        let m = map(&a);
        let m = m.borrow();
        Ok(if boolean(&b) {
            Value::array([
                Value::array(m.keys().map(HashKey::to_value)),
                Value::array(m.values().cloned()),
            ])
        } else {
            Value::array(
                m.iter()
                    .map(|(k, v)| Value::array([k.to_value(), v.clone()])),
            )
        })
    });
    r.binary("isNil", HASH, STR, BOOL, |_, a, b| {
        let k = HashKey::String(string(&b).into());
        Ok(Value::Bool(
            map(&a).borrow().get(&k).is_none_or(Value::is_nil),
        ))
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
        // `insert [[k, v], ...]`, or `insert [split, data]` where `split`
        // true means `data` is `[[keys], [values]]` and false a pair list.
        let pairs: Vec<(Value, Value)> = match args.first() {
            Some(Value::Bool(true)) => {
                let data = array(args.get(1).unwrap_or(&Value::Nil));
                let data = data.borrow();
                let (ks, vs) = (
                    array(data.first().unwrap_or(&Value::Nil)),
                    array(data.get(1).unwrap_or(&Value::Nil)),
                );
                let (ks, vs) = (ks.borrow(), vs.borrow());
                ks.iter()
                    .enumerate()
                    .map(|(i, k)| (k.clone(), vs.get(i).cloned().unwrap_or(Value::Nil)))
                    .collect()
            }
            first => {
                let list = match first {
                    Some(Value::Bool(false)) => {
                        array(args.get(1).unwrap_or(&Value::Nil)).borrow().clone()
                    }
                    _ => args.clone(),
                };
                let mut out = Vec::new();
                for pair in &list {
                    let Value::Array(pair) = pair else {
                        return Err(SqfError::type_error(pair, ARR));
                    };
                    let pair = pair.borrow();
                    out.push((
                        pair.first().cloned().unwrap_or(Value::Nil),
                        pair.get(1).cloned().unwrap_or(Value::Nothing),
                    ));
                }
                out
            }
        };
        let mut entries = m.borrow_mut();
        for (k, v) in pairs {
            let k = key(&k)?;
            may_add(&m, &entries, &k)?;
            entries.insert(k, v);
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
            may_add(target, &dst, k)?;
            dst.insert(k.clone(), v.clone());
        }
    }
    Ok(())
}
