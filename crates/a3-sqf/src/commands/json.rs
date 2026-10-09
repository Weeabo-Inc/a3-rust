//! `toJSON` and `fromJSON` (Arma 3 2.18).
//!
//! `toJSON` writes booleans, numbers, strings, arrays and hash maps (string
//! keys only; other keys are skipped). Unsupported values inside a
//! container become `null`; an unsupported value on its own gives `""`.
//! `fromJSON` reads objects as hash maps and `null` as nil.

use crate::value::HashMapEntries;

use super::*;
use crate::value::{HashKey, HashMap};

fn number(out: &mut String, n: f32) {
    if !n.is_finite() {
        out.push_str("null");
    } else if n.fract() == 0.0 && n.abs() < 1e15 {
        out.push_str(&format!("{}", n as i64));
    } else {
        out.push_str(&format!("{n}"));
    }
}

fn string_lit(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Writes `v`; returns false when the type is unsupported.
fn write(out: &mut String, v: &Value, depth: usize) -> bool {
    if depth > 64 {
        out.push_str("null");
        return true;
    }
    match v {
        Value::Nil => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => number(out, *n),
        Value::String(s) => string_lit(out, s),
        Value::Array(a) => {
            out.push('[');
            for (i, item) in a.borrow().iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                if !write(out, item, depth + 1) {
                    out.push_str("null");
                }
            }
            out.push(']');
        }
        Value::HashMap(m) => {
            out.push('{');
            let mut first = true;
            for (k, item) in m.borrow().iter() {
                let HashKey::String(key) = k else {
                    continue;
                };
                if !first {
                    out.push(',');
                }
                first = false;
                string_lit(out, key);
                out.push(':');
                if !write(out, item, depth + 1) {
                    out.push_str("null");
                }
            }
            out.push('}');
        }
        _ => return false,
    }
    true
}

struct Reader<'a> {
    s: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn ws(&mut self) {
        while self.pos < self.s.len() && self.s[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }

    fn eat(&mut self, lit: &str) -> bool {
        if self.s[self.pos..].starts_with(lit.as_bytes()) {
            self.pos += lit.len();
            true
        } else {
            false
        }
    }

    fn value(&mut self) -> Option<Value> {
        self.ws();
        let c = *self.s.get(self.pos)?;
        match c {
            b'n' if self.eat("null") => Some(Value::Nil),
            b't' if self.eat("true") => Some(Value::Bool(true)),
            b'f' if self.eat("false") => Some(Value::Bool(false)),
            b'"' => self.string().map(Value::from),
            b'[' => {
                self.pos += 1;
                let mut items = Vec::new();
                self.ws();
                if self.eat("]") {
                    return Some(Value::array(items));
                }
                loop {
                    items.push(self.value()?);
                    self.ws();
                    if self.eat(",") {
                        continue;
                    }
                    if self.eat("]") {
                        return Some(Value::array(items));
                    }
                    return None;
                }
            }
            b'{' => {
                self.pos += 1;
                let mut map = HashMapEntries::default();
                self.ws();
                if self.eat("}") {
                    return Some(Value::HashMap(HashMap::from_map(map)));
                }
                loop {
                    self.ws();
                    let key = self.string()?;
                    self.ws();
                    if !self.eat(":") {
                        return None;
                    }
                    let v = self.value()?;
                    map.insert(HashKey::String(key.into()), v);
                    self.ws();
                    if self.eat(",") {
                        continue;
                    }
                    if self.eat("}") {
                        return Some(Value::HashMap(HashMap::from_map(map)));
                    }
                    return None;
                }
            }
            _ => {
                let start = self.pos;
                while self.pos < self.s.len()
                    && matches!(
                        self.s[self.pos],
                        b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9'
                    )
                {
                    self.pos += 1;
                }
                let text = std::str::from_utf8(&self.s[start..self.pos]).ok()?;
                text.parse::<f64>().ok().map(|n| Value::Number(n as f32))
            }
        }
    }

    fn string(&mut self) -> Option<String> {
        if !self.eat("\"") {
            return None;
        }
        let mut out = Vec::new();
        loop {
            let c = *self.s.get(self.pos)?;
            self.pos += 1;
            match c {
                b'"' => return String::from_utf8(out).ok(),
                b'\\' => {
                    let e = *self.s.get(self.pos)?;
                    self.pos += 1;
                    match e {
                        b'n' => out.push(b'\n'),
                        b't' => out.push(b'\t'),
                        b'r' => out.push(b'\r'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'u' => {
                            let hex =
                                std::str::from_utf8(self.s.get(self.pos..self.pos + 4)?).ok()?;
                            self.pos += 4;
                            let mut code = u32::from_str_radix(hex, 16).ok()?;
                            if (0xD800..0xDC00).contains(&code) && self.eat("\\u") {
                                let hex2 = std::str::from_utf8(self.s.get(self.pos..self.pos + 4)?)
                                    .ok()?;
                                self.pos += 4;
                                let low = u32::from_str_radix(hex2, 16).ok()?;
                                code = 0x10000 + ((code - 0xD800) << 10) + (low - 0xDC00);
                            }
                            let ch = char::from_u32(code).unwrap_or('\u{FFFD}');
                            let mut buf = [0; 4];
                            out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                        }
                        other => out.push(other),
                    }
                }
                other => out.push(other),
            }
        }
    }
}

pub(super) fn register<H: Host>(r: &mut Registry<H>) {
    r.unary("toJSON", ANY, STR, |_, a| {
        if a.is_nil() {
            // The engine does not run toJSON on a bare nil.
            return Ok(Value::Nil);
        }
        let mut out = String::new();
        if !write(&mut out, &a, 0) {
            out.clear();
        }
        Ok(Value::from(out))
    });
    r.unary("fromJSON", STR, ANY, |_, a| {
        let mut rd = Reader {
            s: string(&a).as_bytes(),
            pos: 0,
        };
        let v = rd.value();
        rd.ws();
        match v {
            Some(v) if rd.pos == rd.s.len() => Ok(v),
            _ => Err(SqfError::generic("Invalid JSON")),
        }
    });
}
