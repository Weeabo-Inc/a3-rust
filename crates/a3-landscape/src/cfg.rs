//! Small typed accessors over [`ConfigRef`].

use a3_config::{ConfigRef, Value};
use glam::Vec3;

/// The number at `name`, or `None` when missing or not a number-like value.
pub(crate) fn number(c: &ConfigRef<'_>, name: &str) -> Option<f32> {
    let e = c.get(name);
    (!e.is_null() && !e.is_class() && !e.is_array()).then(|| e.number())
}

/// The number at `name`, or `default`.
pub(crate) fn number_or(c: &ConfigRef<'_>, name: &str, default: f32) -> f32 {
    number(c, name).unwrap_or(default)
}

/// The number at `name` like [`number`], but a text entry is evaluated as an arithmetic
/// expression (`"2.0/50"`), as the engine does for numeric config entries. `None` when missing
/// or not a number.
pub(crate) fn expr_number(c: &ConfigRef<'_>, name: &str) -> Option<f32> {
    let e = c.get(name);
    if e.is_null() || e.is_class() || e.is_array() {
        return None;
    }
    if e.is_text() {
        eval_arithmetic(&e.text())
    } else {
        Some(e.number())
    }
}

/// Evaluates `+ - * /` with parentheses and unary signs over decimal numbers (an optional `f`
/// suffix is allowed). `None` on anything else.
pub(crate) fn eval_arithmetic(text: &str) -> Option<f32> {
    struct P<'a> {
        s: &'a [u8],
        i: usize,
    }
    impl P<'_> {
        fn skip(&mut self) {
            while self.s.get(self.i).is_some_and(u8::is_ascii_whitespace) {
                self.i += 1;
            }
        }
        fn peek(&mut self) -> Option<u8> {
            self.skip();
            self.s.get(self.i).copied()
        }
        fn sum(&mut self) -> Option<f64> {
            let mut v = self.product()?;
            while let Some(op @ (b'+' | b'-')) = self.peek() {
                self.i += 1;
                let r = self.product()?;
                v = if op == b'+' { v + r } else { v - r };
            }
            Some(v)
        }
        fn product(&mut self) -> Option<f64> {
            let mut v = self.unary()?;
            while let Some(op @ (b'*' | b'/')) = self.peek() {
                self.i += 1;
                let r = self.unary()?;
                v = if op == b'*' { v * r } else { v / r };
            }
            Some(v)
        }
        fn unary(&mut self) -> Option<f64> {
            match self.peek()? {
                b'-' => {
                    self.i += 1;
                    Some(-self.unary()?)
                }
                b'+' => {
                    self.i += 1;
                    self.unary()
                }
                b'(' => {
                    self.i += 1;
                    let v = self.sum()?;
                    (self.peek()? == b')').then(|| self.i += 1)?;
                    Some(v)
                }
                _ => {
                    let start = self.i;
                    while self
                        .s
                        .get(self.i)
                        .is_some_and(|c| c.is_ascii_digit() || matches!(c, b'.' | b'e' | b'E'))
                    {
                        self.i += 1;
                    }
                    let v = std::str::from_utf8(&self.s[start..self.i])
                        .ok()?
                        .parse()
                        .ok()?;
                    if self.s.get(self.i).is_some_and(|c| matches!(c, b'f' | b'F')) {
                        self.i += 1;
                    }
                    Some(v)
                }
            }
        }
    }
    let mut p = P {
        s: text.as_bytes(),
        i: 0,
    };
    let v = p.sum()?;
    (p.peek().is_none() && v.is_finite()).then_some(v as f32)
}

/// The text at `name`, or `None` when missing.
pub(crate) fn text(c: &ConfigRef<'_>, name: &str) -> Option<String> {
    let e = c.get(name);
    (!e.is_null() && !e.is_class() && !e.is_array()).then(|| e.text())
}

/// The text at `name`, or empty.
pub(crate) fn text_or_empty(c: &ConfigRef<'_>, name: &str) -> String {
    text(c, name).unwrap_or_default()
}

pub(crate) fn value_number(v: &Value) -> Option<f32> {
    match v {
        Value::Float(f) => Some(*f),
        Value::Int(i) => Some(*i as f32),
        Value::Int64(i) => Some(*i as f32),
        Value::String(s) | Value::Expression(s) => s.trim().trim_end_matches('f').parse().ok(),
        Value::Array(_) => None,
    }
}

pub(crate) fn value_text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) | Value::Expression(s) => Some(s.clone()),
        _ => None,
    }
}

/// The numbers of the array at `name` (non-numbers skipped).
pub(crate) fn numbers(c: &ConfigRef<'_>, name: &str) -> Vec<f32> {
    c.get(name)
        .array()
        .iter()
        .filter_map(value_number)
        .collect()
}

/// The strings of the array at `name` (non-strings skipped).
pub(crate) fn texts(c: &ConfigRef<'_>, name: &str) -> Vec<String> {
    c.get(name).array().iter().filter_map(value_text).collect()
}

/// The first `N` numbers of the array at `name`, padded with `default`.
pub(crate) fn array_n<const N: usize>(c: &ConfigRef<'_>, name: &str, default: f32) -> [f32; N] {
    let v = numbers(c, name);
    std::array::from_fn(|i| v.get(i).copied().unwrap_or(default))
}

pub(crate) fn vec3(c: &ConfigRef<'_>, name: &str) -> Vec3 {
    Vec3::from_array(array_n::<3>(c, name, 0.0))
}

/// The child classes of `c`, in order (own and inherited).
pub(crate) fn classes<'a>(c: &ConfigRef<'a>) -> Vec<ConfigRef<'a>> {
    c.entries_with_inherited()
        .into_iter()
        .filter(|e| e.is_class())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::eval_arithmetic;

    #[test]
    fn evaluates_config_arithmetic() {
        assert_eq!(eval_arithmetic("2.0/50"), Some(0.04));
        assert_eq!(eval_arithmetic(" 1 + 2 * 3 "), Some(7.0));
        assert_eq!(eval_arithmetic("-(1 + 1) * 0.5f"), Some(-1.0));
        assert_eq!(eval_arithmetic("15.0f"), Some(15.0));
        assert_eq!(eval_arithmetic("1e3"), Some(1000.0));
        assert_eq!(eval_arithmetic("sea"), None);
        assert_eq!(eval_arithmetic("1/0"), None);
        assert_eq!(eval_arithmetic("1 +"), None);
    }
}
