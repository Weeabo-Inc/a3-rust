//! `regexMatch`, `regexFind`, `regexReplace`.
//!
//! The engine uses Boost.Regex (Perl syntax). A pattern may end in
//! `/flags`; the flag `i` makes it case-insensitive, other flags are
//! accepted and ignored. Matching is case-sensitive without `/i`
//! _(uncertain: two help examples in the binary imply case-insensitive
//! defaults, the regexMatch example uses `/i` explicitly)_. Implemented with
//! `fancy-regex`, which supports the Perl features scripts use
//! (backreferences, lookaround).
//!
//! - `regexMatch` is true when the pattern matches the whole string
//!   _(uncertain: Boost `regex_match` semantics assumed from the name)_.
//! - `regexFind [pattern, startOffset]` returns every match as an array of
//!   `[text, offset]` pairs, the whole match first, then each group.
//! - `regexReplace [pattern, replacement]` replaces every match; `$&` is
//!   the whole match and `$1`.. are groups.

use fancy_regex::{Regex, RegexBuilder};

use super::*;

fn compile(pattern: &str) -> Result<Regex, SqfError> {
    let (body, flags) = strip_flags(pattern);
    build(body, flags)
}

fn build(body: &str, flags: &str) -> Result<Regex, SqfError> {
    let mut b = RegexBuilder::new(body);
    b.case_insensitive(flags.contains('i'));
    b.build()
        .map_err(|e| SqfError::generic(format!("Regex error: {e}")))
}
/// Converts Boost/ECMAScript replacement syntax to fancy-regex's.
fn replacement(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '$' {
            match chars.peek() {
                Some('&') => {
                    chars.next();
                    out.push_str("${0}");
                }
                Some('$') => {
                    chars.next();
                    out.push_str("$$");
                }
                Some(d) if d.is_ascii_digit() => {
                    let mut n = String::new();
                    while let Some(d) = chars.peek().filter(|d| d.is_ascii_digit()) {
                        n.push(*d);
                        chars.next();
                    }
                    out.push_str(&format!("${{{n}}}"));
                }
                _ => out.push_str("$$"),
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn regex_err(e: fancy_regex::Error) -> SqfError {
    SqfError::generic(format!("Regex error: {e}"))
}

pub(super) fn register<H: Host>(r: &mut Registry<H>) {
    r.binary("regexMatch", STR, STR, BOOL, |_, a, b| {
        let (body, flags) = strip_flags(string(&b));
        let re = build(&format!("^(?:{body})$"), flags)?;
        Ok(Value::Bool(re.is_match(string(&a)).map_err(regex_err)?))
    });
    r.binary("regexFind", STR, ARR, ARR, |_, a, b| {
        let args = array(&b);
        let args = args.borrow();
        let re = compile(expect_str(args.first().unwrap_or(&Value::Nil))?)?;
        let start = args.get(1).map(num).unwrap_or(0.0).max(0.0) as usize;
        let s = string(&a);
        if start > s.len() || !s.is_char_boundary(start) {
            return Ok(Value::array([]));
        }
        let mut out = Vec::new();
        for caps in re.captures_iter(&s[start..]) {
            let caps = caps.map_err(regex_err)?;
            let groups = caps.iter().map(|g| match g {
                Some(m) => Value::array([
                    Value::from(m.as_str()),
                    Value::Number((m.start() + start) as f32),
                ]),
                None => Value::array([Value::from(""), Value::Number(-1.0)]),
            });
            out.push(Value::array(groups));
        }
        Ok(Value::Array(Array::from_vec(out)))
    });
    r.binary("regexReplace", STR, ARR, STR, |_, a, b| {
        let args = array(&b);
        let args = args.borrow();
        let re = compile(expect_str(args.first().unwrap_or(&Value::Nil))?)?;
        let with = replacement(expect_str(args.get(1).unwrap_or(&Value::Nil))?);
        let out = re
            .try_replacen(string(&a), 0, with.as_str())
            .map_err(regex_err)?;
        Ok(Value::from(out.into_owned()))
    });
}

fn strip_flags(pattern: &str) -> (&str, &str) {
    match pattern.rfind('/') {
        Some(i) if i > 0 && pattern[i + 1..].chars().all(|c| c.is_ascii_alphabetic()) => {
            (&pattern[..i], &pattern[i + 1..])
        }
        _ => (pattern, ""),
    }
}
