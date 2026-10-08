//! String commands.
//!
//! The engine's `select` and `find` on strings work on bytes of the UTF-8
//! text (unless `forceUnicode` is active); `count` counts characters
//! _(uncertain)_. Results that would split a character are repaired with
//! U+FFFD.

use super::*;

fn byte_slice(s: &str, start: usize, len: usize) -> String {
    let bytes = s.as_bytes();
    let start = start.min(bytes.len());
    let end = start.saturating_add(len).min(bytes.len());
    String::from_utf8_lossy(&bytes[start..end]).into_owned()
}

/// C `strtod`-style prefix parse: leading whitespace, optional sign,
/// digits, fraction, exponent. Returns 0 when nothing parses.
pub(crate) fn parse_number_prefix(s: &str) -> f32 {
    let t = s.trim_start();
    let b = t.as_bytes();
    let mut i = 0;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        i += 1;
    }
    if b.len() > i + 1 && b[i] == b'0' && (b[i + 1] == b'x' || b[i + 1] == b'X') {
        let digits = &t[i + 2..];
        let end = digits
            .find(|c: char| !c.is_ascii_hexdigit())
            .unwrap_or(digits.len());
        let v = u64::from_str_radix(&digits[..end], 16).unwrap_or(0) as f32;
        return if b[0] == b'-' { -v } else { v };
    }
    let digits_start = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    if i < b.len() && b[i] == b'.' {
        i += 1;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
    }
    if i == digits_start || (i == digits_start + 1 && b[digits_start] == b'.') {
        return 0.0;
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        let mut j = i + 1;
        if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
            j += 1;
        }
        if j < b.len() && b[j].is_ascii_digit() {
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            i = j;
        }
    }
    t[..i].parse::<f64>().map(|v| v as f32).unwrap_or(0.0)
}

pub(super) fn register<H: Host>(r: &mut Registry<H>) {
    r.unary("count", STR, NUM, |_, a| {
        Ok(Value::Number(string(&a).chars().count() as f32))
    });
    r.unary("toUpper", STR, STR, |_, a| {
        Ok(Value::from(string(&a).to_uppercase()))
    });
    r.unary("toLower", STR, STR, |_, a| {
        Ok(Value::from(string(&a).to_lowercase()))
    });
    r.unary("trim", STR, STR, |_, a| Ok(Value::from(string(&a).trim())));
    r.binary("trim", STR, ARR, STR, |_, a, b| {
        let args = array(&b);
        let args = args.borrow();
        let chars: Vec<char> = match args.first() {
            Some(Value::String(c)) => c.chars().collect(),
            _ => vec![' ', '\t', '\r', '\n'],
        };
        let mode = args.get(1).map(num).unwrap_or(0.0) as i32;
        let s = string(&a);
        let is = |c: char| chars.contains(&c);
        let out = match mode {
            1 => s.trim_start_matches(is),
            2 => s.trim_end_matches(is),
            _ => s.trim_matches(is),
        };
        Ok(Value::from(out))
    });
    r.binary("select", STR, NUM, STR, |_, a, b| {
        let s = string(&a);
        let i = index(num(&b));
        if i < 0 || i as usize >= s.len() {
            return Ok(Value::from(""));
        }
        Ok(Value::from(byte_slice(s, i as usize, 1)))
    });
    r.binary("select", STR, ARR, STR, |_, a, b| {
        let s = string(&a);
        let args = array(&b);
        let args = args.borrow();
        let start = args.first().map(num).unwrap_or(0.0).max(0.0) as usize;
        let len = args
            .get(1)
            .map(|v| num(v).max(0.0) as usize)
            .unwrap_or(usize::MAX);
        Ok(Value::from(byte_slice(s, start, len)))
    });
    r.binary("find", STR, STR, NUM, |_, a, b| {
        Ok(Value::Number(
            string(&a).find(string(&b)).map_or(-1.0, |i| i as f32),
        ))
    });
    r.binary("find", STR, ARR, NUM, |_, a, b| {
        let args = array(&b);
        let args = args.borrow();
        let needle = expect_str(args.first().unwrap_or(&Value::Nil))?;
        let start = args.get(1).map(num).unwrap_or(0.0).max(0.0) as usize;
        let s = string(&a);
        if start > s.len() {
            return Ok(Value::Number(-1.0));
        }
        let hay = &s.as_bytes()[start..];
        let pos = hay
            .windows(needle.len().max(1))
            .position(|w| w == needle.as_bytes());
        Ok(Value::Number(pos.map_or(-1.0, |p| (p + start) as f32)))
    });
    r.binary("in", STR, STR, BOOL, |_, a, b| {
        Ok(Value::Bool(string(&b).contains(string(&a))))
    });
    r.binary("splitString", STR, STR, ARR, |_, a, b| {
        let delims: Vec<char> = string(&b).chars().collect();
        let s = string(&a);
        let parts: Vec<Value> = if delims.is_empty() {
            s.chars().map(|c| Value::from(c.to_string())).collect()
        } else {
            s.split(|c| delims.contains(&c))
                .filter(|p| !p.is_empty())
                .map(Value::from)
                .collect()
        };
        Ok(Value::Array(Array::from_vec(parts)))
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
        Ok(Value::Number(parse_number_prefix(string(&a))))
    });
    r.unary("parseNumber", BOOL, NUM, |_, a| {
        Ok(Value::Number(if boolean(&a) { 1.0 } else { 0.0 }))
    });
}
