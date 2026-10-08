//! Arithmetic and math commands. Angles are in degrees, as in the engine.
//! All arithmetic is single precision.

use super::*;
use crate::value::{HashMap, deep_copy_value};
use crate::vm::Ctx;

pub(super) fn register<H: Host>(r: &mut Registry<H>) {
    r.binary("+", NUM, NUM, NUM, |_, a, b| {
        Ok(Value::Number(num(&a) + num(&b)))
    });
    r.binary("+", STR, STR, STR, |_, a, b| {
        let mut s = String::with_capacity(string(&a).len() + string(&b).len());
        s.push_str(string(&a));
        s.push_str(string(&b));
        Ok(Value::from(s))
    });
    r.binary("+", ARR, ARR, ARR, |_, a, b| {
        let mut items = array(&a).borrow().clone();
        items.extend(array(&b).borrow().iter().cloned());
        Ok(Value::Array(Array::from_vec(items)))
    });
    r.unary("+", NUM, NUM, |_, a| Ok(a));
    r.unary("+", ARR, ARR, |_, a| Ok(deep_copy_value(&a)));
    r.unary("+", HASH, HASH, |_, a| {
        let Value::HashMap(m) = a else { unreachable!() };
        let copy = HashMap::new();
        {
            let src = m.borrow();
            let mut dst = copy.borrow_mut();
            for (k, v) in src.iter() {
                dst.insert(k.clone(), deep_copy_value(v));
            }
        }
        Ok(Value::HashMap(copy))
    });
    r.binary("-", NUM, NUM, NUM, |_, a, b| {
        Ok(Value::Number(num(&a) - num(&b)))
    });
    r.binary("-", ARR, ARR, ARR, |_, a, b| {
        let remove = array(&b);
        let remove = remove.borrow();
        let kept: Vec<Value> = array(&a)
            .borrow()
            .iter()
            .filter(|v| !remove.iter().any(|r| r.is_equal_to(v)))
            .cloned()
            .collect();
        Ok(Value::Array(Array::from_vec(kept)))
    });
    r.unary("-", NUM, NUM, |_, a| Ok(Value::Number(-num(&a))));
    r.binary("*", NUM, NUM, NUM, |_, a, b| {
        Ok(Value::Number(num(&a) * num(&b)))
    });
    r.binary("/", NUM, NUM, NUM, |_, a, b| {
        let d = num(&b);
        if d == 0.0 {
            return Err(SqfError::ZeroDivisor);
        }
        Ok(Value::Number(num(&a) / d))
    });
    for name in ["%", "mod"] {
        r.binary(name, NUM, NUM, NUM, |_, a, b| {
            let d = num(&b);
            if d == 0.0 {
                return Err(SqfError::ZeroDivisor);
            }
            Ok(Value::Number(num(&a) % d))
        });
    }
    r.binary("^", NUM, NUM, NUM, |_, a, b| {
        Ok(Value::Number(num(&a).powf(num(&b))))
    });
    r.binary("min", NUM, NUM, NUM, |_, a, b| {
        Ok(Value::Number(num(&a).min(num(&b))))
    });
    r.binary("max", NUM, NUM, NUM, |_, a, b| {
        Ok(Value::Number(num(&a).max(num(&b))))
    });
    r.binary("atan2", NUM, NUM, NUM, |_, a, b| {
        Ok(Value::Number(num(&a).atan2(num(&b)).to_degrees()))
    });

    r.unary("abs", NUM, NUM, |_, a| Ok(Value::Number(num(&a).abs())));
    register_unary_math(r);
    r.unary("round", NUM, NUM, |_, a| Ok(Value::Number(num(&a).round())));
    r.unary("random", NUM, NUM, |ctx, a| {
        let x = num(&a);
        Ok(Value::Number(ctx.random() * x))
    });
    r.nular("pi", NUM, |_| Ok(Value::Number(std::f32::consts::PI)));
    r.unary("finite", NUM, BOOL, |_, a| {
        Ok(Value::Bool(num(&a).is_finite()))
    });
    r.unary("linearConversion", ARR, NUM, linear_conversion);
    r.binary("toFixed", NUM, NUM, STR, |_, a, b| {
        let digits = num(&b).clamp(0.0, 20.0) as usize;
        Ok(Value::from(format!("{:.*}", digits, f64::from(num(&a)))))
    });
    r.binary("bitAnd", NUM, NUM, NUM, |_, a, b| {
        Ok(bits(&a, &b, |x, y| x & y))
    });
    r.binary("bitOr", NUM, NUM, NUM, |_, a, b| {
        Ok(bits(&a, &b, |x, y| x | y))
    });
    r.binary("bitXor", NUM, NUM, NUM, |_, a, b| {
        Ok(bits(&a, &b, |x, y| x ^ y))
    });
    r.binary("bitShiftLeft", NUM, NUM, NUM, |_, a, b| {
        Ok(bits(&a, &b, |x, y| x.checked_shl(y).unwrap_or(0)))
    });
    r.binary("bitShiftRight", NUM, NUM, NUM, |_, a, b| {
        Ok(bits(&a, &b, |x, y| x.checked_shr(y).unwrap_or(0)))
    });
    r.unary("bitNot", NUM, NUM, |_, a| {
        Ok(Value::Number((!(num(&a) as u32)) as f32))
    });
}

fn bits(a: &Value, b: &Value, f: fn(u32, u32) -> u32) -> Value {
    Value::Number(f(num(a) as u32, num(b) as u32) as f32)
}

macro_rules! unary_math {
    ($r:ident, $($name:literal => $f:expr),* $(,)?) => {
        $(
            $r.unary($name, NUM, NUM, |_, a| {
                let f: fn(f32) -> f32 = $f;
                Ok(Value::Number(f(num(&a))))
            });
        )*
    };
}

fn register_unary_math<H: Host>(r: &mut Registry<H>) {
    unary_math!(r,
        "sqrt" => f32::sqrt,
        "sin" => |x| x.to_radians().sin(),
        "cos" => |x| x.to_radians().cos(),
        "tan" => |x| x.to_radians().tan(),
        "asin" => |x| x.asin().to_degrees(),
        "acos" => |x| x.acos().to_degrees(),
        "atan" => |x| x.atan().to_degrees(),
        "atg" => |x| x.atan().to_degrees(),
        "exp" => f32::exp,
        "ln" => f32::ln,
        "log" => f32::log10,
        "floor" => f32::floor,
        "ceil" => f32::ceil,
        "deg" => f32::to_degrees,
        "rad" => f32::to_radians,
    );
}

fn linear_conversion<H: Host>(_: &mut Ctx<'_, H>, a: Value) -> Result<Value, SqfError> {
    let arr = array(&a);
    let items = arr.borrow();
    if items.len() < 5 {
        return Err(SqfError::generic(format!(
            "{} elements provided, 5 expected",
            items.len()
        )));
    }
    let n = |i: usize| expect_num(&items[i]);
    let (min_from, max_from, value, min_to, max_to) = (n(0)?, n(1)?, n(2)?, n(3)?, n(4)?);
    let clip = items.get(5).is_some_and(boolean);
    let t = (value - min_from) / (max_from - min_from);
    let mut out = min_to + t * (max_to - min_to);
    if clip {
        let (lo, hi) = if min_to <= max_to {
            (min_to, max_to)
        } else {
            (max_to, min_to)
        };
        out = out.clamp(lo, hi);
    }
    Ok(Value::Number(out))
}
