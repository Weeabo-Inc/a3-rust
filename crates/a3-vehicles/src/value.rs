//! Reading numbers out of a config: the entry forms the `CfgVehicles` handling classes use,
//! and the small arithmetic language the engine evaluates in them.
//!
//! Shipped configs write numbers in several shapes: `maxBrakeTorque = 2000`, `maxCompression =
//! 0.05`, `width = "0.3"` (text), and `springDamperRate = "1920*2"` (an expression the engine
//! bakes away when it binarises, `docs/re/config.md`). [`number`] reads all of them.

use a3_config::{ConfigRef, Value};
use glam::DVec3;

/// The number an entry holds: a number entry directly, a text or expression entry through
/// [`evaluate`]. `None` for a missing entry, a class, an array, and text that is no number.
pub fn number(entry: &ConfigRef<'_>) -> Option<f64> {
    if entry.is_number() {
        return Some(entry.number() as f64);
    }
    if entry.is_text() {
        return evaluate(&entry.text());
    }
    None
}

/// [`number`], or `default` where the entry is not a number.
pub fn number_or(entry: &ConfigRef<'_>, default: f64) -> f64 {
    number(entry).unwrap_or(default)
}

/// Every number of an array entry, skipping entries that are not numbers.
pub fn numbers(entry: &ConfigRef<'_>) -> Vec<f64> {
    entry.array().iter().filter_map(value_number).collect()
}

/// A `{x, y, z}` array entry.
pub fn vec3(entry: &ConfigRef<'_>) -> Option<DVec3> {
    let v = numbers(entry);
    match v[..] {
        [x, y, z] => Some(DVec3::new(x, y, z)),
        _ => None,
    }
}

/// The `name, value` pairs of an array like `GearboxRatios[] = {"R1", -3.231, "N", 0, ...}`.
/// An odd entry at the end is dropped.
pub fn named_numbers(entry: &ConfigRef<'_>) -> Vec<(String, f64)> {
    let items = entry.array();
    items
        .chunks(2)
        .filter_map(|pair| match pair {
            [Value::String(name), value] | [Value::Expression(name), value] => {
                Some((name.clone(), value_number(value)?))
            }
            _ => None,
        })
        .collect()
}

/// The points of an array of `{x, y}` arrays, like `torqueCurve[] =
/// {{0, 0.8}, {0.33, 1}, {1, 0.8}}`. Points that are not two numbers are skipped.
pub fn points(entry: &ConfigRef<'_>) -> Vec<(f64, f64)> {
    entry
        .array()
        .iter()
        .filter_map(|point| match point {
            Value::Array(pair) => match &pair[..] {
                [x, y] => Some((value_number(x)?, value_number(y)?)),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

/// The pairs of an array of `{x, y, z}` arrays or a flat number list taken in pairs, as the
/// `changeGearOmegaRatios[]` variant of the two forms needs. Returns what is there; the caller
/// checks the length.
pub fn number_pairs(entry: &ConfigRef<'_>) -> Vec<(f64, f64)> {
    let items = entry.array();
    if items.iter().all(|i| matches!(i, Value::Array(_))) {
        return items
            .iter()
            .filter_map(|point| match point {
                Value::Array(pair) => match &pair[..] {
                    [x, y] => Some((value_number(x)?, value_number(y)?)),
                    _ => None,
                },
                _ => None,
            })
            .collect();
    }
    items
        .chunks(2)
        .filter_map(|pair| match pair {
            [x, y] => Some((value_number(x)?, value_number(y)?)),
            _ => None,
        })
        .collect()
}

fn value_number(value: &Value) -> Option<f64> {
    match value {
        Value::Float(f) => Some(*f as f64),
        Value::Int(i) => Some(*i as f64),
        Value::Int64(i) => Some(*i as f64),
        Value::String(s) | Value::Expression(s) => evaluate(s),
        Value::Array(_) => None,
    }
}

/// Evaluates an arithmetic expression of the config language: the numbers and operators the
/// shipped vehicle configs use in text entries.
///
/// Grammar: `+ - * / ^`, parentheses, unary minus, and the functions `sqrt`, `abs`, `min`,
/// `max`, `sin`, `cos`, `tan`, `floor`, `ceil`, `round`, `log`, `exp`, with the constants `pi`
/// and `e`; `__EVAL(...)` is unwrapped. This is the config's expression form, not SQF: no
/// variables, no commands. `None` when the text does not evaluate (an SQF expression with
/// identifiers in it, `"true"`, or garbage).
pub fn evaluate(source: &str) -> Option<f64> {
    let text = unwrap_eval(source.trim());
    let mut parser = Parser {
        input: text.as_bytes(),
        at: 0,
    };
    let value = parser.sum()?;
    parser.skip_space();
    if parser.at != parser.input.len() {
        return None;
    }
    value.is_finite().then_some(value)
}

/// `__EVAL(expr)` → `expr`; anything else unchanged.
fn unwrap_eval(text: &str) -> &str {
    let lower = text.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("__eval") {
        let rest = rest.trim_start();
        if let Some(inner) = rest.strip_prefix('(') {
            if let Some(inner) = inner.strip_suffix(')') {
                let start = text.len() - rest.len() + 1;
                return &text[start..start + inner.len()];
            }
        }
    }
    text
}

struct Parser<'a> {
    input: &'a [u8],
    at: usize,
}

impl Parser<'_> {
    fn skip_space(&mut self) {
        while self
            .input
            .get(self.at)
            .is_some_and(|b| b.is_ascii_whitespace())
        {
            self.at += 1;
        }
    }

    fn eat(&mut self, byte: u8) -> bool {
        self.skip_space();
        if self.input.get(self.at) == Some(&byte) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    /// `sum := product (('+' | '-') product)*`
    fn sum(&mut self) -> Option<f64> {
        let mut value = self.product()?;
        loop {
            if self.eat(b'+') {
                value += self.product()?;
            } else if self.eat(b'-') {
                value -= self.product()?;
            } else {
                return Some(value);
            }
        }
    }

    /// `product := power (('*' | '/') power)*`
    fn product(&mut self) -> Option<f64> {
        let mut value = self.power()?;
        loop {
            if self.eat(b'*') {
                value *= self.power()?;
            } else if self.eat(b'/') {
                let divisor = self.power()?;
                if divisor == 0.0 {
                    return None;
                }
                value /= divisor;
            } else {
                return Some(value);
            }
        }
    }

    /// `power := unary ('^' power)?` — right associative.
    fn power(&mut self) -> Option<f64> {
        let base = self.unary()?;
        if self.eat(b'^') {
            let exponent = self.power()?;
            return Some(base.powf(exponent));
        }
        Some(base)
    }

    /// `unary := ('-' | '+')? primary`
    fn unary(&mut self) -> Option<f64> {
        if self.eat(b'-') {
            return Some(-self.unary()?);
        }
        if self.eat(b'+') {
            return self.unary();
        }
        self.primary()
    }

    /// `primary := number | '(' sum ')' | name ('(' args ')')?`
    fn primary(&mut self) -> Option<f64> {
        self.skip_space();
        let byte = *self.input.get(self.at)?;
        if byte == b'(' {
            self.at += 1;
            let value = self.sum()?;
            if !self.eat(b')') {
                return None;
            }
            return Some(value);
        }
        if byte.is_ascii_digit() || byte == b'.' {
            return self.number();
        }
        let name = self.name()?;
        self.skip_space();
        if !self.eat(b'(') {
            return match name.to_ascii_lowercase().as_str() {
                "pi" => Some(std::f64::consts::PI),
                "e" => Some(std::f64::consts::E),
                "true" => Some(1.0),
                "false" => Some(0.0),
                _ => None,
            };
        }
        let mut args = vec![self.sum()?];
        while self.eat(b',') {
            args.push(self.sum()?);
        }
        if !self.eat(b')') {
            return None;
        }
        let name = name.to_ascii_lowercase();
        let (a, b) = (args[0], *args.get(1).unwrap_or(&args[0]));
        match (name.as_str(), args.len()) {
            ("sqrt", 1) => Some(a.sqrt()),
            ("abs", 1) => Some(a.abs()),
            ("sin", 1) => Some(a.sin()),
            ("cos", 1) => Some(a.cos()),
            ("tan", 1) => Some(a.tan()),
            ("floor", 1) => Some(a.floor()),
            ("ceil", 1) => Some(a.ceil()),
            ("round", 1) => Some(a.round()),
            ("log", 1) => Some(a.ln()),
            ("log", 2) => Some(a.log(b)),
            ("exp", 1) => Some(a.exp()),
            ("min", 2) => Some(a.min(b)),
            ("max", 2) => Some(a.max(b)),
            ("pow", 2) => Some(a.powf(b)),
            _ => None,
        }
    }

    fn number(&mut self) -> Option<f64> {
        let start = self.at;
        while self
            .input
            .get(self.at)
            .is_some_and(|b| b.is_ascii_digit() || *b == b'.')
        {
            self.at += 1;
        }
        // An exponent, as in `1.5e-3`.
        if self
            .input
            .get(self.at)
            .is_some_and(|b| *b == b'e' || *b == b'E')
        {
            let after = self.at + 1;
            let digits = self.input.get(after).is_some_and(u8::is_ascii_digit)
                || (self
                    .input
                    .get(after)
                    .is_some_and(|b| *b == b'-' || *b == b'+')
                    && self.input.get(after + 1).is_some_and(u8::is_ascii_digit));
            if digits {
                self.at = after + 1;
                while self.input.get(self.at).is_some_and(u8::is_ascii_digit) {
                    self.at += 1;
                }
            }
        }
        std::str::from_utf8(&self.input[start..self.at])
            .ok()?
            .parse()
            .ok()
    }

    fn name(&mut self) -> Option<String> {
        let start = self.at;
        while self
            .input
            .get(self.at)
            .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
        {
            self.at += 1;
        }
        (self.at > start).then(|| String::from_utf8_lossy(&self.input[start..self.at]).into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use a3_config::parse_text;

    fn tree(text: &str) -> a3_config::ConfigTree {
        a3_config::ConfigTree::from_config(&parse_text(text).unwrap())
    }

    /// Config numbers are `f32`; compare what is read back with that much slack.
    fn close(got: Option<f64>, want: f64) -> bool {
        got.is_some_and(|got| (got - want).abs() < 1e-6)
    }

    #[test]
    fn evaluates_the_forms_the_shipped_configs_use() {
        assert_eq!(evaluate("0.05"), Some(0.05));
        assert_eq!(evaluate("1920*2"), Some(3840.0));
        assert_eq!(evaluate("4.5*(0.58^1)"), Some(2.61));
        assert_eq!(evaluate("1 + 2 * 3"), Some(7.0));
        assert_eq!(evaluate("(1 + 2) * 3"), Some(9.0));
        assert_eq!(evaluate("2^3^2"), Some(512.0));
        assert_eq!(evaluate("-4"), Some(-4.0));
        assert_eq!(evaluate("- 4 * 2"), Some(-8.0));
        assert_eq!(evaluate("sqrt(16)"), Some(4.0));
        assert_eq!(evaluate("max(1, 3)"), Some(3.0));
        assert_eq!(evaluate("1.5e-3"), Some(0.0015));
        assert_eq!(evaluate(" 0.3 "), Some(0.3));
    }

    #[test]
    fn refuses_what_is_not_an_expression() {
        assert_eq!(evaluate("left"), None);
        assert_eq!(evaluate("wheel_1_1_axis"), None);
        assert_eq!(evaluate(""), None);
        assert_eq!(evaluate("1/0"), None);
        assert_eq!(evaluate("1 +"), None);
        assert_eq!(evaluate("(1"), None);
    }

    #[test]
    fn reads_numbers_in_every_entry_form() {
        let tree =
            tree("class V { a = 2000; b = 0.05; c = \"0.3\"; d = \"1920*2\"; e = \"left\"; }");
        let v = tree.root().get("V");
        assert_eq!(number(&v.get("a")), Some(2000.0));
        assert!(close(number(&v.get("b")), 0.05));
        assert!(close(number(&v.get("c")), 0.3));
        assert!(close(number(&v.get("d")), 3840.0));
        assert_eq!(number(&v.get("e")), None);
        assert_eq!(number(&v.get("missing")), None);
        assert_eq!(number_or(&v.get("e"), 7.0), 7.0);
    }

    #[test]
    fn reads_arrays() {
        let tree = tree(
            "class V { pos[] = {1, 2, 3}; ratios[] = {\"R1\", -3, \"N\", 0}; \
             curve[] = {{0, 0.8}, {1, 1.2}}; bad[] = {1, \"x\", 3}; }",
        );
        let v = tree.root().get("V");
        assert_eq!(vec3(&v.get("pos")), Some(DVec3::new(1.0, 2.0, 3.0)));
        assert_eq!(
            named_numbers(&v.get("ratios")),
            vec![("R1".to_string(), -3.0), ("N".to_string(), 0.0)]
        );
        let curve = points(&v.get("curve"));
        assert!(close(curve.first().copied().map(|(_, y)| y), 0.8));
        assert!(close(curve.get(1).copied().map(|(_, y)| y), 1.2));
        assert_eq!(numbers(&v.get("bad")), vec![1.0, 3.0]);
    }
}
