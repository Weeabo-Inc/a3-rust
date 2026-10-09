//! Arithmetic and math commands. Angles are in degrees, as in the engine.
//! All arithmetic is single precision.

use super::*;
use crate::value::deep_copy_value;
use crate::vm::Ctx;

pub(super) fn register<H: Host>(r: &mut Registry<H>) {
    r.binary("+", NUM, NUM, NUM, |_, a, b| Ok(number(num(&a) + num(&b))));
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
    r.binary("-", NUM, NUM, NUM, |_, a, b| Ok(number(num(&a) - num(&b))));
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
    r.binary("*", NUM, NUM, NUM, |_, a, b| Ok(number(num(&a) * num(&b))));
    r.binary("/", NUM, NUM, NUM, |_, a, b| {
        let d = num(&b);
        if d == 0.0 {
            return Err(SqfError::ZeroDivisor);
        }
        Ok(number(num(&a) / d))
    });
    for name in ["%", "mod"] {
        r.binary(name, NUM, NUM, NUM, |_, a, b| {
            let d = num(&b);
            if d == 0.0 {
                return Err(SqfError::ZeroDivisor);
            }
            Ok(number(num(&a) % d))
        });
    }
    r.binary("^", NUM, NUM, NUM, |_, a, b| {
        Ok(number(num(&a).powf(num(&b))))
    });
    // `a < b ? a : b` and `a > b ? a : b`: with a NaN operand the result is
    // the right-hand one.
    r.binary("min", NUM, NUM, NUM, |_, a, b| {
        let (x, y) = (num(&a), num(&b));
        Ok(Value::Number(if x < y { x } else { y }))
    });
    r.binary("max", NUM, NUM, NUM, |_, a, b| {
        let (x, y) = (num(&a), num(&b));
        Ok(Value::Number(if x > y { x } else { y }))
    });
    r.binary("atan2", NUM, NUM, NUM, |_, a, b| {
        Ok(Value::Number(num(&a).atan2(num(&b)).to_degrees()))
    });

    r.unary("abs", NUM, NUM, |_, a| Ok(Value::Number(num(&a).abs())));
    register_unary_math(r);
    // floor(x + 0.5) in single precision: halves round up (`round -2.5` is
    // -2), and 0.49999997 rounds to 1.
    r.unary("round", NUM, NUM, |_, a| {
        Ok(number((num(&a) + 0.5).floor()))
    });
    r.unary("random", NUM, NUM, |ctx, a| {
        let x = num(&a);
        Ok(Value::Number(ctx.random() * x))
    });
    r.nular("pi", NUM, |_| Ok(Value::Number(std::f32::consts::PI)));
    r.unary("finite", NUM, BOOL, |_, a| {
        Ok(Value::Bool(num(&a).is_finite()))
    });
    r.unary("linearConversion", ARR, NUM, linear_conversion);
    // Only finite numbers: infinity or NaN on the left fails the type check.
    r.binary("toFixed", SCALAR, NUM, STR, |_, a, b| {
        let digits = round_half_even(num(&b)).clamp(0, 20) as usize;
        Ok(Value::from(crate::number::format_fixed(num(&a), digits)))
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
                Ok(number(f(num(&a))))
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
    // A source range narrower than 1e-6 maps everything to `minTo`.
    if (max_from - min_from).abs() < 1e-6 {
        return Ok(items[3].clone());
    }
    let clip = items.get(5).is_some_and(boolean);
    let mut out = ((max_to - min_to) * (value - min_from)) / (max_from - min_from) + min_to;
    if clip {
        if max_to <= min_to {
            if out <= max_to {
                out = max_to;
            }
            if min_to <= out {
                out = min_to;
            }
        } else {
            if out <= min_to {
                out = min_to;
            }
            if max_to <= out {
                out = max_to;
            }
        }
    }
    Ok(number(out))
}

/// A number result with subnormals flushed to zero (the engine runs with
/// SSE flush-to-zero).
pub(crate) fn number(x: f32) -> Value {
    Value::Number(crate::number::ftz(x))
}

/// `cvtss2si`: rounds to the nearest integer, ties to even; out of range
/// and NaN give `i32::MIN`.
pub(crate) fn round_half_even(x: f32) -> i32 {
    if !x.is_finite() || x.abs() >= 2_147_483_648.0 {
        return i32::MIN;
    }
    x.round_ties_even() as i32
}
