//! Array commands. Arrays are shared by reference: `pushBack`, `set`,
//! `append`, `resize`, `reverse`, `sort`, `deleteAt`, `deleteRange` and
//! `insert` modify the array in place.
//!
//! `select` and `#` with an index past the end raise "N elements provided,
//! M expected" (an index equal to the size returns `nil`); negative indices
//! count from the end. A `[start, count]` range outside the array is empty.

use std::cmp::Ordering;

use super::*;
use crate::vm::Ctx;

fn element_at(a: &Value, i: i64) -> Option<Value> {
    if i < 0 {
        return None;
    }
    array(a).borrow().get(i as usize).cloned()
}

/// `array select index` and `array # index`: a negative index counts from
/// the end; an index equal to the size gives nil; anything further out is
/// the engine's "N elements provided, M expected" error.
fn select_index(a: &Value, b: &Value) -> Result<Value, SqfError> {
    let len = array(a).len() as i64;
    let mut i = index(num(b));
    if i < 0 {
        i += len;
    }
    if i < 0 || i >= len {
        if i == len {
            return Ok(Value::Nil);
        }
        return Err(SqfError::generic(format!(
            "{len} elements provided, {} expected",
            i + 1
        )));
    }
    Ok(element_at(a, i).unwrap_or(Value::Nil))
}

fn check_not_self(target: &Array, v: &Value) -> Result<(), SqfError> {
    if let Value::Array(inner) = v {
        if inner.contains_ref(target) {
            return Err(SqfError::generic("Recursive array"));
        }
    }
    Ok(())
}

pub(super) fn register<H: Host>(r: &mut Registry<H>) {
    r.unary("count", ARR, NUM, |_, a| {
        Ok(Value::Number(array(&a).len() as f32))
    });
    r.binary("select", ARR, NUM, ANY, |_, a, b| select_index(&a, &b));
    r.binary("select", ARR, BOOL, ANY, |_, a, b| {
        Ok(element_at(&a, i64::from(boolean(&b))).unwrap_or(Value::Nil))
    });
    r.binary("select", ARR, ARR, ARR, |_, a, b| {
        let args = array(&b);
        let args = args.borrow();
        let items = array(&a);
        let items = items.borrow();
        let start = args.first().map(|v| index(num(v))).unwrap_or(0);
        if start < 0 || start as usize >= items.len() {
            return Ok(Value::array([]));
        }
        let start = start as usize;
        let count = args
            .get(1)
            .map(|v| index(num(v)).max(0) as usize)
            .unwrap_or(usize::MAX);
        let end = start.saturating_add(count).min(items.len());
        Ok(Value::array(items[start..end].iter().cloned()))
    });
    r.binary("#", ARR, NUM, ANY, |_, a, b| select_index(&a, &b));
    r.binary("pushBack", ARR, ANY, NUM, |_, a, b| {
        let arr = array(&a);
        check_not_self(&arr, &b)?;
        let mut items = arr.borrow_mut();
        items.push(b);
        Ok(Value::Number((items.len() - 1) as f32))
    });
    r.binary("pushBackUnique", ARR, ANY, NUM, |_, a, b| {
        let arr = array(&a);
        check_not_self(&arr, &b)?;
        let mut items = arr.borrow_mut();
        if items.iter().any(|v| v.is_equal_to(&b)) {
            return Ok(Value::Number(-1.0));
        }
        items.push(b);
        Ok(Value::Number((items.len() - 1) as f32))
    });
    r.binary("append", ARR, ARR, NOTHING, |_, a, b| {
        let extra = array(&b).borrow().clone();
        let arr = array(&a);
        for v in &extra {
            check_not_self(&arr, v)?;
        }
        arr.borrow_mut().extend(extra);
        Ok(Value::Nothing)
    });
    r.binary("set", ARR, ARR, NOTHING, |_, a, b| {
        let args = array(&b);
        let args = args.borrow();
        let i = index(expect_num(args.first().unwrap_or(&Value::Nil))?);
        if i < 0 {
            return Err(SqfError::ZeroDivisor);
        }
        let v = args.get(1).cloned().unwrap_or(Value::Nil);
        let arr = array(&a);
        check_not_self(&arr, &v)?;
        let mut items = arr.borrow_mut();
        let i = i as usize;
        if i >= items.len() {
            items.resize(i + 1, Value::Nil);
        }
        items[i] = v;
        Ok(Value::Nothing)
    });
    r.binary("resize", ARR, NUM, NOTHING, |_, a, b| {
        let n = num(&b).max(0.0) as usize;
        array(&a).borrow_mut().resize(n, Value::Nil);
        Ok(Value::Nothing)
    });
    r.binary("resize", ARR, ARR, NOTHING, |_, a, b| {
        let args = array(&b);
        let args = args.borrow();
        let n = expect_num(args.first().unwrap_or(&Value::Nil))?.max(0.0) as usize;
        let fill = args.get(1).cloned().unwrap_or(Value::Nil);
        array(&a).borrow_mut().resize(n, fill);
        Ok(Value::Nothing)
    });
    r.unary("reverse", ARR, NOTHING, |_, a| {
        array(&a).borrow_mut().reverse();
        Ok(Value::Nothing)
    });
    r.binary("deleteAt", ARR, NUM, ANY, |_, a, b| {
        let i = index(num(&b));
        let arr = array(&a);
        let mut items = arr.borrow_mut();
        if i < 0 || i as usize >= items.len() {
            return Ok(Value::Nil);
        }
        Ok(items.remove(i as usize))
    });
    r.binary("deleteRange", ARR, ARR, NOTHING, |_, a, b| {
        let args = array(&b);
        let args = args.borrow();
        let from = args.first().map(num).unwrap_or(0.0).max(0.0) as usize;
        let count = args.get(1).map(num).unwrap_or(0.0).max(0.0) as usize;
        let arr = array(&a);
        let mut items = arr.borrow_mut();
        let from = from.min(items.len());
        let to = from.saturating_add(count).min(items.len());
        items.drain(from..to);
        Ok(Value::Nothing)
    });
    r.binary("insert", ARR, ARR, NUM, |_, a, b| {
        let args = array(&b);
        let args = args.borrow();
        let at = index(expect_num(args.first().unwrap_or(&Value::Nil))?);
        let new = match args.get(1) {
            Some(Value::Array(x)) => x.borrow().clone(),
            Some(other) => return Err(SqfError::type_error(other, ARR)),
            None => Vec::new(),
        };
        let unique = args.get(2).is_some_and(boolean);
        let arr = array(&a);
        for v in &new {
            check_not_self(&arr, v)?;
        }
        let mut items = arr.borrow_mut();
        let mut pos = if at < 0 {
            items.len()
        } else {
            (at as usize).min(items.len())
        };
        let mut inserted = 0;
        for v in new {
            if unique && items.iter().any(|x| x.is_equal_to(&v)) {
                continue;
            }
            items.insert(pos, v);
            pos += 1;
            inserted += 1;
        }
        Ok(Value::Number(inserted as f32))
    });
    r.binary("find", ARR, ANY, NUM, |_, a, b| {
        let pos = array(&a).borrow().iter().position(|v| v.is_equal_to(&b));
        Ok(Value::Number(pos.map_or(-1.0, |p| p as f32)))
    });
    r.binary("findAny", ARR, ARR, NUM, |_, a, b| {
        let needles = array(&b);
        let needles = needles.borrow();
        let pos = array(&a)
            .borrow()
            .iter()
            .position(|v| needles.iter().any(|n| n.is_equal_to(v)));
        Ok(Value::Number(pos.map_or(-1.0, |p| p as f32)))
    });
    r.binary("in", ANY, ARR, BOOL, |_, a, b| {
        Ok(Value::Bool(
            array(&b).borrow().iter().any(|v| v.is_equal_to(&a)),
        ))
    });
    r.binary("arrayIntersect", ARR, ARR, ARR, |_, a, b| {
        let other = array(&b);
        let other = other.borrow();
        let mut out: Vec<Value> = Vec::new();
        for v in array(&a).borrow().iter() {
            if other.iter().any(|o| o.is_equal_to(v)) && !out.iter().any(|o| o.is_equal_to(v)) {
                out.push(v.clone());
            }
        }
        Ok(Value::Array(Array::from_vec(out)))
    });
    r.binary("sort", ARR, BOOL, NOTHING, sort);
    r.unary("flatten", ARR, ARR, |_, a| {
        fn walk(arr: &Array, out: &mut Vec<Value>) {
            for v in arr.borrow().iter() {
                match v {
                    Value::Array(inner) => walk(inner, out),
                    other => out.push(other.clone()),
                }
            }
        }
        let mut out = Vec::new();
        walk(&array(&a), &mut out);
        Ok(Value::Array(Array::from_vec(out)))
    });
    r.unary("selectRandom", ARR, ANY, |ctx, a| {
        let arr = array(&a);
        let len = arr.len();
        if len == 0 {
            return Ok(Value::Nil);
        }
        let i = ((ctx.random() * len as f32) as usize).min(len - 1);
        Ok(arr.borrow()[i].clone())
    });
}

fn compare_values(a: &Value, b: &Value) -> Result<Ordering, SqfError> {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => Ok(x.partial_cmp(y).unwrap_or(Ordering::Equal)),
        (Value::String(x), Value::String(y)) => Ok(x.cmp(y)),
        (Value::Bool(x), Value::Bool(y)) => Ok(x.cmp(y)),
        (Value::Array(x), Value::Array(y)) => {
            let (x, y) = (x.borrow(), y.borrow());
            for (p, q) in x.iter().zip(y.iter()) {
                let o = compare_values(p, q)?;
                if o != Ordering::Equal {
                    return Ok(o);
                }
            }
            Ok(x.len().cmp(&y.len()))
        }
        _ => Err(SqfError::type_error(b, a.ty())),
    }
}

/// `array sort ascending`: numbers, strings or arrays (compared element by
/// element), all of one type; sorts in place.
fn sort<H: Host>(_: &mut Ctx<'_, H>, a: Value, b: Value) -> Result<Value, SqfError> {
    let arr = array(&a);
    let mut items = arr.borrow().clone();
    let mut err = None;
    items.sort_by(|x, y| match compare_values(x, y) {
        Ok(o) => o,
        Err(e) => {
            err.get_or_insert(e);
            Ordering::Equal
        }
    });
    if let Some(e) = err {
        return Err(e);
    }
    if !boolean(&b) {
        items.reverse();
    }
    *arr.borrow_mut() = items;
    Ok(Value::Nothing)
}
