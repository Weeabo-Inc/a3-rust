//! Comparisons. `==` compares strings without regard to ASCII case and only
//! accepts two values of the same comparable type; `isEqualTo` accepts
//! anything and compares exactly (case-sensitive, deep for arrays).

use super::*;

/// Types `==` and `!=` accept (both sides of the same type).
const EQ_TYPES: [Type; 16] = [
    Type::Number,
    Type::Bool,
    Type::NetObject,
    Type::String,
    Type::Side,
    Type::Namespace,
    Type::Object,
    Type::Group,
    Type::Text,
    Type::Config,
    Type::Display,
    Type::Control,
    Type::TeamMember,
    Type::Task,
    Type::Location,
    Type::DiaryRecord,
];

fn loose_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::String(x), Value::String(y)) => x.eq_ignore_ascii_case(y),
        _ => a.is_equal_to(b),
    }
}

pub(super) fn register<H: Host>(r: &mut Registry<H>) {
    for ty in EQ_TYPES {
        let t = TypeSet::of(ty);
        r.binary("==", t, t, BOOL, |_, a, b| {
            Ok(Value::Bool(loose_equal(&a, &b)))
        });
        r.binary("!=", t, t, BOOL, |_, a, b| {
            Ok(Value::Bool(!loose_equal(&a, &b)))
        });
    }
    r.binary("<", NUM, NUM, BOOL, |_, a, b| {
        Ok(Value::Bool(num(&a) < num(&b)))
    });
    r.binary(">", NUM, NUM, BOOL, |_, a, b| {
        Ok(Value::Bool(num(&a) > num(&b)))
    });
    r.binary("<=", NUM, NUM, BOOL, |_, a, b| {
        Ok(Value::Bool(num(&a) <= num(&b)))
    });
    r.binary(">=", NUM, NUM, BOOL, |_, a, b| {
        Ok(Value::Bool(num(&a) >= num(&b)))
    });
    r.binary("isEqualTo", ANY, ANY, BOOL, |_, a, b| {
        Ok(Value::Bool(a.is_equal_to(&b)))
    });
    r.binary("isNotEqualTo", ANY, ANY, BOOL, |_, a, b| {
        Ok(Value::Bool(!a.is_equal_to(&b)))
    });
    r.binary("isEqualRef", ANY, ANY, BOOL, |_, a, b| {
        Ok(Value::Bool(match (&a, &b) {
            (Value::Array(x), Value::Array(y)) => x.ptr_eq(y),
            (Value::HashMap(x), Value::HashMap(y)) => x.ptr_eq(y),
            (Value::Code(x), Value::Code(y)) => x.ptr_eq(y),
            _ => a.is_equal_to(&b),
        }))
    });
    r.binary("isEqualType", ANY, ANY, BOOL, |_, a, b| {
        Ok(Value::Bool(same_type(&a, &b)))
    });
    r.binary("isEqualTypeAll", ARR, ANY, BOOL, |_, a, b| {
        let arr = array(&a);
        let items = arr.borrow();
        Ok(Value::Bool(
            !items.is_empty() && items.iter().all(|v| same_type(v, &b)),
        ))
    });
    r.unary("isEqualTypeAll", ARR, BOOL, |_, a| {
        let arr = array(&a);
        let items = arr.borrow();
        Ok(Value::Bool(match items.first() {
            Some(first) => items.iter().all(|v| same_type(v, first)),
            None => false,
        }))
    });
    r.binary("isEqualTypeAny", ANY, ARR, BOOL, |_, a, b| {
        let arr = array(&b);
        let types = arr.borrow();
        Ok(Value::Bool(types.iter().any(|t| same_type(&a, t))))
    });
    r.binary("isEqualTypeArray", ARR, ARR, BOOL, |_, a, b| {
        let (a, b) = (array(&a), array(&b));
        let (a, b) = (a.borrow(), b.borrow());
        Ok(Value::Bool(
            a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| same_type(x, y)),
        ))
    });
    r.binary("isEqualTypeParams", ANY, ARR, BOOL, |_, a, b| {
        let template = array(&b);
        let template = template.borrow();
        let Value::Array(values) = &a else {
            return Ok(Value::Bool(false));
        };
        let values = values.borrow();
        Ok(Value::Bool(
            values.len() >= template.len()
                && template
                    .iter()
                    .zip(values.iter())
                    .all(|(t, v)| t.is_nil() || same_type(v, t)),
        ))
    });
}

/// Same type for `isEqualType`: NaN counts as a number.
fn same_type(a: &Value, b: &Value) -> bool {
    let norm = |t: Type| if t == Type::NaN { Type::Number } else { t };
    norm(a.ty()) == norm(b.ty())
}
