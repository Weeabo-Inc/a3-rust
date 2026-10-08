//! `regexMatch`, `regexFind`, `regexReplace`.
//!
//! The engine uses Boost.Regex (Perl syntax); see `docs/re/sqf-semantics.md`
//! for the decompiled flag handling (`FUN_1408637d0`):
//!
//! - A pattern may end in `/flags` (the last `/`, followed only by flag
//!   letters `g`, `i`, `n`, `o`). Without such a suffix the defaults are
//!   case-insensitive and global. With it, `i` makes the pattern
//!   case-insensitive and `g` global; `pattern/` means case-sensitive, first
//!   match only.
//! - `^` and `$` match at line breaks (Boost's Perl default).
//! - `regexMatch` must match the whole string (`match_all`).
//! - `regexFind [pattern, startOffset]` returns each match as an array of
//!   `[text, offset]` pairs: the whole match, then each group. Offsets are
//!   from the start of the haystack.
//! - `regexReplace [pattern, format]` uses Boost's Perl format syntax: `$&`,
//!   `$0`..`$n`, `${n}`, `` $` ``, `$'`, `$+{name}`, `$$`, and the case
//!   operators `\L`, `\U`, `\E`, `\l`, `\u`.
//!
//! Offsets are byte offsets, or character offsets under `forceUnicode`.
//! Implemented with `fancy-regex` (backreferences, lookaround).

use fancy_regex::{Captures, Regex, RegexBuilder};

use super::*;
use crate::vm::Ctx;

struct Pattern {
    re: Regex,
    global: bool,
}

fn split_flags(pattern: &str) -> (&str, bool, bool) {
    if let Some(i) = pattern.rfind('/') {
        let flags = &pattern[i + 1..];
        if flags.chars().all(|c| matches!(c, 'g' | 'i' | 'n' | 'o')) {
            return (&pattern[..i], flags.contains('i'), flags.contains('g'));
        }
    }
    (pattern, true, true)
}

fn regex_err(e: impl std::fmt::Display) -> SqfError {
    SqfError::generic(format!("Regex error: {e}"))
}

fn compile(pattern: &str, whole: bool) -> Result<Pattern, SqfError> {
    let (body, icase, global) = split_flags(pattern);
    let src = if whole {
        format!(r"(?m)\A(?:{body})\z")
    } else {
        format!("(?m){body}")
    };
    let mut b = RegexBuilder::new(&src);
    b.case_insensitive(icase);
    Ok(Pattern {
        re: b.build().map_err(regex_err)?,
        global,
    })
}

/// Converts a byte offset in `s` to the offset scripts see.
fn offset(s: &str, byte: usize, unicode: bool) -> f32 {
    if unicode {
        s[..byte].chars().count() as f32
    } else {
        byte as f32
    }
}

/// Expands a Boost Perl-style format string for one match.
fn expand(caps: &Captures<'_>, text: &str, prev_end: usize, format: &str) -> String {
    #[derive(Clone, Copy, PartialEq)]
    enum Case {
        Keep,
        Lower,
        Upper,
    }
    let mut out = String::new();
    let mut case = Case::Keep;
    let mut once: Option<Case> = None;
    let whole = caps.get(0).expect("group 0");
    let push = |out: &mut String, s: &str, case: Case, once: &mut Option<Case>| {
        for c in s.chars() {
            let mode = once.take().unwrap_or(case);
            match mode {
                Case::Lower => out.extend(c.to_lowercase()),
                Case::Upper => out.extend(c.to_uppercase()),
                Case::Keep => out.push(c),
            }
        }
    };
    let chars: Vec<char> = format.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '$' && i + 1 < chars.len() {
            let n = chars[i + 1];
            match n {
                '&' => {
                    push(&mut out, whole.as_str(), case, &mut once);
                    i += 2;
                    continue;
                }
                '`' => {
                    push(&mut out, &text[prev_end..whole.start()], case, &mut once);
                    i += 2;
                    continue;
                }
                '\'' => {
                    push(&mut out, &text[whole.end()..], case, &mut once);
                    i += 2;
                    continue;
                }
                '$' => {
                    out.push('$');
                    i += 2;
                    continue;
                }
                '{' => {
                    if let Some(end) = chars[i + 2..].iter().position(|&c| c == '}') {
                        let inner: String = chars[i + 2..i + 2 + end].iter().collect();
                        if let Ok(g) = inner.parse::<usize>() {
                            if let Some(m) = caps.get(g) {
                                push(&mut out, m.as_str(), case, &mut once);
                            }
                            i += 3 + end;
                            continue;
                        }
                    }
                }
                '+' if chars.get(i + 2) == Some(&'{') => {
                    if let Some(end) = chars[i + 3..].iter().position(|&c| c == '}') {
                        let name: String = chars[i + 3..i + 3 + end].iter().collect();
                        if let Some(m) = caps.name(&name) {
                            push(&mut out, m.as_str(), case, &mut once);
                        }
                        i += 4 + end;
                        continue;
                    }
                }
                d if d.is_ascii_digit() => {
                    let mut j = i + 1;
                    while j < chars.len() && chars[j].is_ascii_digit() {
                        j += 1;
                    }
                    let g: usize = chars[i + 1..j]
                        .iter()
                        .collect::<String>()
                        .parse()
                        .unwrap_or(0);
                    if let Some(m) = caps.get(g) {
                        push(&mut out, m.as_str(), case, &mut once);
                    }
                    i = j;
                    continue;
                }
                _ => {}
            }
        }
        if c == '\\' && i + 1 < chars.len() {
            let n = chars[i + 1];
            i += 2;
            match n {
                'L' => case = Case::Lower,
                'U' => case = Case::Upper,
                'E' => case = Case::Keep,
                'l' => once = Some(Case::Lower),
                'u' => once = Some(Case::Upper),
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'r' => out.push('\r'),
                other => push(&mut out, &other.to_string(), case, &mut once),
            }
            continue;
        }
        push(&mut out, &c.to_string(), case, &mut once);
        i += 1;
    }
    out
}

fn find<H: Host>(ctx: &mut Ctx<'_, H>, a: &Value, b: &Value) -> Result<Value, SqfError> {
    let args = array(b);
    let args = args.borrow();
    let p = compile(expect_str(args.first().unwrap_or(&Value::Nil))?, false)?;
    let unicode = ctx.take_unicode();
    let s = string(a);
    let start = index(args.get(1).map(num).unwrap_or(0.0)).max(0) as usize;
    let start = if unicode {
        s.char_indices().nth(start).map_or(s.len(), |(i, _)| i)
    } else {
        start
    };
    if start > s.len() || !s.is_char_boundary(start) {
        return Ok(Value::array([]));
    }
    let mut out = Vec::new();
    let mut pos = start;
    while pos <= s.len() {
        let Some(caps) = p.re.captures_from_pos(s, pos).map_err(regex_err)? else {
            break;
        };
        let whole = caps.get(0).expect("group 0");
        let groups = caps.iter().map(|g| match g {
            Some(m) => Value::array([
                Value::from(m.as_str()),
                Value::Number(offset(s, m.start(), unicode)),
            ]),
            None => Value::array([Value::from(""), Value::Number(-1.0)]),
        });
        out.push(Value::array(groups));
        if !p.global {
            break;
        }
        pos = if whole.end() > whole.start() {
            whole.end()
        } else {
            match s[whole.end()..].chars().next() {
                Some(c) => whole.end() + c.len_utf8(),
                None => break,
            }
        };
    }
    Ok(Value::Array(Array::from_vec(out)))
}

fn replace<H: Host>(ctx: &mut Ctx<'_, H>, a: &Value, b: &Value) -> Result<Value, SqfError> {
    let args = array(b);
    let args = args.borrow();
    let p = compile(expect_str(args.first().unwrap_or(&Value::Nil))?, false)?;
    let format = expect_str(args.get(1).unwrap_or(&Value::Nil))?.to_string();
    ctx.take_unicode();
    let s = string(a);
    let mut out = String::with_capacity(s.len());
    let mut last = 0;
    let mut pos = 0;
    while pos <= s.len() {
        let Some(caps) = p.re.captures_from_pos(s, pos).map_err(regex_err)? else {
            break;
        };
        let whole = caps.get(0).expect("group 0");
        out.push_str(&s[last..whole.start()]);
        out.push_str(&expand(&caps, s, last, &format));
        last = whole.end();
        if !p.global {
            break;
        }
        pos = if whole.end() > whole.start() {
            whole.end()
        } else {
            match s[whole.end()..].chars().next() {
                Some(c) => {
                    out.push(c);
                    last = whole.end() + c.len_utf8();
                    whole.end() + c.len_utf8()
                }
                None => break,
            }
        };
    }
    out.push_str(&s[last.min(s.len())..]);
    Ok(Value::from(out))
}

pub(super) fn register<H: Host>(r: &mut Registry<H>) {
    r.binary("regexMatch", STR, STR, BOOL, |ctx, a, b| {
        ctx.take_unicode();
        let p = compile(string(&b), true)?;
        Ok(Value::Bool(p.re.is_match(string(&a)).map_err(regex_err)?))
    });
    r.binary("regexFind", STR, ARR, ARR, |ctx, a, b| find(ctx, &a, &b));
    r.binary("regexReplace", STR, ARR, STR, |ctx, a, b| {
        replace(ctx, &a, &b)
    });
}
