//! String commands.
//!
//! Like the engine, `count`, `select`, `find`, `in`, `splitString` and `trim`
//! work on the bytes of the UTF-8 text unless `forceUnicode` is active, in
//! which case they work on characters (`count` then counts UTF-16 units, as
//! the engine's `MultiByteToWideChar` does). A byte range that splits a
//! character is repaired with U+FFFD. See `docs/re/sqf-semantics.md`.

use super::*;
use crate::vm::Ctx;

/// A string seen as bytes or as characters.
enum Units<'a> {
    Bytes(&'a [u8]),
    Chars(Vec<char>),
}

impl<'a> Units<'a> {
    fn new(s: &'a str, unicode: bool) -> Units<'a> {
        if unicode {
            Units::Chars(s.chars().collect())
        } else {
            Units::Bytes(s.as_bytes())
        }
    }

    fn len(&self) -> usize {
        match self {
            Units::Bytes(b) => b.len(),
            Units::Chars(c) => c.len(),
        }
    }

    fn slice(&self, start: usize, end: usize) -> String {
        let start = start.min(self.len());
        let end = end.clamp(start, self.len());
        match self {
            Units::Bytes(b) => String::from_utf8_lossy(&b[start..end]).into_owned(),
            Units::Chars(c) => c[start..end].iter().collect(),
        }
    }

    fn find(&self, needle: &Units<'_>, from: usize) -> Option<usize> {
        if from > self.len() {
            return None;
        }
        match (self, needle) {
            (Units::Bytes(h), Units::Bytes(n)) => find_slice(&h[from..], n).map(|p| p + from),
            (Units::Chars(h), Units::Chars(n)) => find_slice(&h[from..], n).map(|p| p + from),
            _ => None,
        }
    }
}

fn find_slice<T: PartialEq>(hay: &[T], needle: &[T]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

fn unicode<H: Host>(ctx: &mut Ctx<'_, H>) -> bool {
    ctx.take_unicode()
}

/// Splits `s` at any of the delimiter units, dropping empty parts.
fn split(s: &str, delims: &str, unicode: bool) -> Vec<Value> {
    if unicode || delims.is_ascii() {
        let d: Vec<char> = delims.chars().collect();
        if d.is_empty() {
            return s.chars().map(|c| Value::from(c.to_string())).collect();
        }
        return s
            .split(|c| d.contains(&c))
            .filter(|p| !p.is_empty())
            .map(Value::from)
            .collect();
    }
    let d = delims.as_bytes();
    s.as_bytes()
        .split(|b| d.contains(b))
        .filter(|p| !p.is_empty())
        .map(|p| Value::from(String::from_utf8_lossy(p).into_owned()))
        .collect()
}

/// Trims units in `set` from the start (`mode` 1), end (2) or both (0).
fn trim_units(s: &str, set: &str, mode: i32, unicode: bool) -> String {
    let u = Units::new(s, unicode);
    let is_trim = |i: usize| match &u {
        Units::Bytes(b) => set.as_bytes().contains(&b[i]),
        Units::Chars(c) => set.contains(c[i]),
    };
    let mut start = 0;
    let mut end = u.len();
    if mode != 2 {
        while start < end && is_trim(start) {
            start += 1;
        }
    }
    if mode != 1 {
        while end > start && is_trim(end - 1) {
            end -= 1;
        }
    }
    u.slice(start, end)
}

const WHITESPACE: &str = " \t\r\n";

pub(super) fn register<H: Host>(r: &mut Registry<H>) {
    r.unary("count", STR, NUM, |ctx, a| {
        let s = string(&a);
        let n = if unicode(ctx) {
            s.chars().map(char::len_utf16).sum()
        } else {
            s.len()
        };
        Ok(Value::Number(n as f32))
    });
    r.unary("forceUnicode", NUM, NOTHING, |ctx, a| {
        ctx.set_unicode_mode(num(&a).round_ties_even().clamp(-1.0, 1.0) as i8);
        Ok(Value::Nothing)
    });
    r.unary("toUpper", STR, STR, |_, a| {
        Ok(Value::from(string(&a).to_uppercase()))
    });
    r.unary("toLower", STR, STR, |_, a| {
        Ok(Value::from(string(&a).to_lowercase()))
    });
    r.unary("trim", STR, STR, |ctx, a| {
        let u = unicode(ctx);
        Ok(Value::from(trim_units(string(&a), WHITESPACE, 0, u)))
    });
    r.binary("trim", STR, ARR, STR, |ctx, a, b| {
        let args = array(&b);
        let args = args.borrow();
        let set = match args.first() {
            Some(Value::String(c)) => c.to_string(),
            _ => WHITESPACE.to_string(),
        };
        let mode = args.get(1).map(num).unwrap_or(0.0) as i32;
        let u = unicode(ctx);
        Ok(Value::from(trim_units(string(&a), &set, mode, u)))
    });
    r.binary("select", STR, ARR, STR, |ctx, a, b| {
        let args = array(&b);
        let args = args.borrow();
        let u = Units::new(string(&a), unicode(ctx));
        let start = index(args.first().map(num).unwrap_or(0.0)).max(0) as usize;
        let len = args
            .get(1)
            .map(|v| index(num(v)).max(0) as usize)
            .unwrap_or(usize::MAX);
        Ok(Value::from(u.slice(start, start.saturating_add(len))))
    });
    r.binary("find", STR, STR, NUM, |ctx, a, b| {
        let uni = unicode(ctx);
        let (h, n) = (Units::new(string(&a), uni), Units::new(string(&b), uni));
        Ok(Value::Number(h.find(&n, 0).map_or(-1.0, |i| i as f32)))
    });
    r.binary("find", STR, ARR, NUM, |ctx, a, b| {
        let args = array(&b);
        let args = args.borrow();
        let needle = expect_str(args.first().unwrap_or(&Value::Nil))?.to_string();
        let start = index(args.get(1).map(num).unwrap_or(0.0)).max(0) as usize;
        let uni = unicode(ctx);
        let (h, n) = (Units::new(string(&a), uni), Units::new(&needle, uni));
        Ok(Value::Number(h.find(&n, start).map_or(-1.0, |i| i as f32)))
    });
    r.binary("in", STR, STR, BOOL, |ctx, a, b| {
        unicode(ctx);
        Ok(Value::Bool(string(&b).contains(string(&a))))
    });
    r.binary("splitString", STR, STR, ARR, |ctx, a, b| {
        let u = unicode(ctx);
        Ok(Value::Array(Array::from_vec(split(
            string(&a),
            string(&b),
            u,
        ))))
    });
    r.binary("joinString", ARR, STR, STR, |ctx, a, b| {
        let sep = string(&b);
        let parts: Vec<String> = array(&a)
            .borrow()
            .iter()
            .map(|v| ctx.to_display_string(v))
            .collect();
        Ok(Value::from(parts.join(sep)))
    });
    r.unary("toArray", STR, ARR, |_, a| {
        Ok(Value::array(
            string(&a).chars().map(|c| Value::Number(c as u32 as f32)),
        ))
    });
    r.unary("parseNumber", STR, NUM, |_, a| {
        Ok(Value::Number(crate::number::parse_prefix(string(&a))))
    });
    r.unary("parseNumber", BOOL, NUM, |_, a| {
        Ok(Value::Number(if boolean(&a) { 1.0 } else { 0.0 }))
    });
}
