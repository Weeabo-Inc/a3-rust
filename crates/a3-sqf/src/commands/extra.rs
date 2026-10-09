//! Vector math, random-number forms, `parseSimpleArray` and other
//! world-independent helpers.

use super::*;

use crate::lexer::{TokenKind, tokenize};

use crate::vm::Ctx;

fn vector(v: &Value) -> Result<[f32; 3], SqfError> {
    Ok(vector_sized(v)?.0)
}

/// A vector with its declared size: 2 or 3 components, missing ones zero.
fn vector_sized(v: &Value) -> Result<([f32; 3], usize), SqfError> {
    let arr = array(v);
    let items = arr.borrow();
    if items.len() < 2 || items.len() > 3 {
        return Err(SqfError::generic(format!(
            "{} elements provided, 3 expected",
            items.len()
        )));
    }
    let mut out = [0.0; 3];
    for (i, item) in items.iter().enumerate() {
        out[i] = expect_num(item)?;
    }
    Ok((out, items.len()))
}

fn vec_value(v: [f32; 3]) -> Value {
    Value::array(v.into_iter().map(Value::Number))
}

/// A vector of `len` components (2D vectors stay 2D).
fn vec_value_len(v: [f32; 3], len: usize) -> Value {
    Value::array(v.into_iter().take(len).map(Value::Number))
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn magnitude(a: [f32; 3]) -> f32 {
    dot(a, a).sqrt()
}

macro_rules! vec2 {
    ($r:ident, $name:literal, $ret:expr, |$a:ident, $b:ident| $body:expr) => {
        $r.binary($name, ARR, ARR, $ret, |_, x, y| {
            let ($a, $b) = (vector(&x)?, vector(&y)?);
            Ok($body)
        });
    };
}

pub(super) fn register<H: Host>(r: &mut Registry<H>) {
    // The result keeps the size of the longer vector: a 2D plus a 3D vector
    // is 3D, 2D plus 2D stays 2D (server oracle).
    r.binary("vectorAdd", ARR, ARR, ARR, |_, x, y| {
        let ((a, la), (b, lb)) = (vector_sized(&x)?, vector_sized(&y)?);
        Ok(vec_value_len(
            [a[0] + b[0], a[1] + b[1], a[2] + b[2]],
            la.max(lb),
        ))
    });
    vec2!(r, "vectorDiff", ARR, |a, b| vec_value([
        a[0] - b[0],
        a[1] - b[1],
        a[2] - b[2]
    ]));
    vec2!(r, "vectorDotProduct", NUM, |a, b| Value::Number(dot(a, b)));
    vec2!(r, "vectorCrossProduct", ARR, |a, b| vec_value([
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]));
    vec2!(r, "vectorDistance", NUM, |a, b| Value::Number(magnitude([
        a[0] - b[0],
        a[1] - b[1],
        a[2] - b[2]
    ])));
    vec2!(r, "vectorDistanceSqr", NUM, |a, b| {
        let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
        Value::Number(dot(d, d))
    });
    vec2!(r, "vectorCos", NUM, |a, b| {
        let m = magnitude(a) * magnitude(b);
        Value::Number(if m == 0.0 { 0.0 } else { dot(a, b) / m })
    });
    r.binary("vectorMultiply", ARR, NUM, ARR, |_, a, b| {
        let (v, k) = (vector(&a)?, num(&b));
        Ok(vec_value([v[0] * k, v[1] * k, v[2] * k]))
    });
    r.unary("vectorMagnitude", ARR, NUM, |_, a| {
        Ok(Value::Number(magnitude(vector(&a)?)))
    });
    r.unary("vectorMagnitudeSqr", ARR, NUM, |_, a| {
        let v = vector(&a)?;
        Ok(Value::Number(dot(v, v)))
    });
    r.unary("vectorNormalized", ARR, ARR, |_, a| {
        let v = vector(&a)?;
        let m = magnitude(v);
        if m == 0.0 {
            return Ok(vec_value([0.0; 3]));
        }
        Ok(vec_value([v[0] / m, v[1] / m, v[2] / m]))
    });
    r.unary("vectorLinearConversion", ARR, ARR, |_, a| {
        let args = array(&a);
        let args = args.borrow();
        if args.len() < 5 {
            return Err(SqfError::generic(format!(
                "{} elements provided, 5 expected",
                args.len()
            )));
        }
        let (min, max, val) = (
            expect_num(&args[0])?,
            expect_num(&args[1])?,
            expect_num(&args[2])?,
        );
        let (v1, v2) = (vector(&args[3])?, vector(&args[4])?);
        let mut t = (val - min) / (max - min);
        if args.get(5).is_some_and(boolean) {
            t = t.clamp(0.0, 1.0);
        }
        Ok(vec_value([
            v1[0] + (v2[0] - v1[0]) * t,
            v1[1] + (v2[1] - v1[1]) * t,
            v1[2] + (v2[2] - v1[2]) * t,
        ]))
    });

    // random [min, mid, max]: a bell-shaped distribution around `mid`
    // (approximated by the mean of four uniform samples).
    r.unary("random", ARR, NUM, |ctx, a| {
        let args = array(&a);
        let args = args.borrow();
        if args.len() != 3 {
            return Err(SqfError::generic(format!(
                "{} elements provided, 3 expected",
                args.len()
            )));
        }
        let (min, mid, max) = (
            expect_num(&args[0])?,
            expect_num(&args[1])?,
            expect_num(&args[2])?,
        );
        let u = (0..4).map(|_| ctx.random()).sum::<f32>() / 4.0;
        let out = if u < 0.5 {
            min + (mid - min) * (u * 2.0)
        } else {
            mid + (max - mid) * ((u - 0.5) * 2.0)
        };
        Ok(Value::Number(out))
    });
    // seed random x: a hash of the truncated seed, scaled by x.
    r.binary("random", SCALAR, SCALAR, NUM, |_, a, b| {
        let seed = truncate_to_int(num(&a));
        Ok(Value::Number(seeded_random(seed) * num(&b)))
    });
    // seed random [x, y]: 2D noise, a hash of the interleaved bits of
    // x * 100 + seed and y * 100 + seed.
    r.binary("random", SCALAR, ARR, NUM, |_, a, b| {
        let seed = num(&a);
        let items = array(&b);
        let items = items.borrow();
        if items.len() != 2 {
            return Err(SqfError::generic(format!(
                "{} elements provided, 2 expected",
                items.len()
            )));
        }
        let coord = |v: &Value| truncate_to_int(v.as_number().unwrap_or(0.0) * 100.0 + seed);
        let key = interleave_bits(coord(&items[0]), coord(&items[1]));
        Ok(Value::Number(seeded_random(key)))
    });

    r.unary("selectMax", ARR, ANY, |_, a| {
        Ok(select_extreme(&a, f32::gt))
    });
    r.unary("selectMin", ARR, ANY, |_, a| {
        Ok(select_extreme(&a, f32::lt))
    });
    r.binary("selectRandomWeighted", ARR, ARR, ANY, |ctx, a, b| {
        let items = array(&a).borrow().clone();
        let weights: Vec<f32> = array(&b).borrow().iter().map(num).collect();
        Ok(weighted(ctx, &items, &weights))
    });
    r.unary("selectRandomWeighted", ARR, ANY, |ctx, a| {
        let flat = array(&a).borrow().clone();
        let items: Vec<Value> = flat.iter().step_by(2).cloned().collect();
        let weights: Vec<f32> = flat.iter().skip(1).step_by(2).map(num).collect();
        Ok(weighted(ctx, &items, &weights))
    });

    // On bad input the engine reports an error and returns what it parsed so
    // far (server oracle: `parseSimpleArray "[1, b]"` is `[1]`,
    // `parseSimpleArray "[1, 2"` is `[1,2]`).
    r.unary("parseSimpleArray", STR, ARR, |ctx, a| {
        let (value, ok) = parse_simple_array(string(&a));
        if !ok {
            ctx.report(SqfError::generic("parseSimpleArray format error"));
        }
        Ok(value)
    });
}

fn select_extreme(a: &Value, better: fn(&f32, &f32) -> bool) -> Value {
    let arr = array(a);
    let items = arr.borrow();
    let mut best: Option<f32> = None;
    for v in items.iter() {
        if let Value::Number(n) = v {
            if best.is_none_or(|b| better(n, &b)) {
                best = Some(*n);
            }
        }
    }
    best.map_or(Value::Nil, Value::Number)
}

fn weighted<H: Host>(ctx: &mut Ctx<'_, H>, items: &[Value], weights: &[f32]) -> Value {
    let total: f32 = weights.iter().take(items.len()).map(|w| w.max(0.0)).sum();
    if total <= 0.0 {
        return Value::Nil;
    }
    let mut pick = ctx.random() * total;
    for (item, w) in items.iter().zip(weights) {
        let w = w.max(0.0);
        if pick < w {
            return item.clone();
        }
        pick -= w;
    }
    items
        .iter()
        .zip(weights)
        .rev()
        .find(|(_, w)| **w > 0.0)
        .map(|(i, _)| i.clone())
        .unwrap_or(Value::Nil)
}

/// Parses an array literal of numbers, strings, booleans and nested arrays
/// (no expressions), as `parseSimpleArray` does. Returns the value parsed so
/// far and whether the text was a complete array.
pub(crate) fn parse_simple_array(text: &str) -> (Value, bool) {
    let Ok(tokens) = tokenize(text) else {
        return (Value::array([]), false);
    };
    let mut pos = 0;
    let src = text;
    fn item(tokens: &[crate::lexer::Token], pos: &mut usize, src: &str) -> Option<Value> {
        let t = tokens.get(*pos)?;
        *pos += 1;
        match &t.kind {
            TokenKind::Number(n) => Some(Value::Number(*n)),
            TokenKind::String(s) => Some(Value::String(s.clone())),
            TokenKind::Operator if &src[t.span.start as usize..t.span.end as usize] == "-" => {
                match item(tokens, pos, src)? {
                    Value::Number(n) => Some(Value::Number(-n)),
                    _ => None,
                }
            }
            TokenKind::Ident => match src[t.span.start as usize..t.span.end as usize]
                .to_ascii_lowercase()
                .as_str()
            {
                "true" => Some(Value::Bool(true)),
                "false" => Some(Value::Bool(false)),
                _ => None,
            },
            TokenKind::LBracket => {
                let (items, ok) = items(tokens, pos, src);
                ok.then(|| Value::array(items))
            }
            _ => None,
        }
    }
    /// The elements up to the closing `]`, and whether the array ended
    /// properly. A trailing comma before `]` is accepted.
    fn items(tokens: &[crate::lexer::Token], pos: &mut usize, src: &str) -> (Vec<Value>, bool) {
        let mut out = Vec::new();
        if tokens
            .get(*pos)
            .is_some_and(|t| t.kind == TokenKind::RBracket)
        {
            *pos += 1;
            return (out, true);
        }
        loop {
            let Some(v) = item(tokens, pos, src) else {
                return (out, false);
            };
            out.push(v);
            match tokens.get(*pos) {
                Some(t) if t.kind == TokenKind::Comma => {
                    *pos += 1;
                    if tokens
                        .get(*pos)
                        .is_some_and(|t| t.kind == TokenKind::RBracket)
                    {
                        *pos += 1;
                        return (out, true);
                    }
                }
                Some(t) if t.kind == TokenKind::RBracket => {
                    *pos += 1;
                    return (out, true);
                }
                _ => return (out, false),
            }
        }
    }
    if tokens.first().map(|t| &t.kind) != Some(&TokenKind::LBracket) {
        return (Value::array([]), false);
    }
    pos += 1;
    let (items, closed) = items(&tokens, &mut pos, src);
    let complete = closed && tokens.get(pos).is_some_and(|t| t.kind == TokenKind::Eof);
    (Value::array(items), complete)
}

/// `cvttss2si`: truncates toward zero; NaN and out-of-range values give
/// `i32::MIN`.
fn truncate_to_int(x: f32) -> i32 {
    if x.is_nan() || !(-2_147_483_648.0..2_147_483_648.0).contains(&x) {
        i32::MIN
    } else {
        x as i32
    }
}

/// The engine's seeded random number: a Wang-style integer hash of `seed`
/// reduced to 15 bits, in `[0, 1]`.
pub(crate) fn seeded_random(seed: i32) -> f32 {
    let p = seed as u32;
    let mut u = ((((p ^ 0x003d_0000) as i32) >> 16) as u32 ^ p).wrapping_mul(9);
    u = (((u as i32) >> 4) as u32 ^ u).wrapping_mul(0x27d4_eb2d);
    let bits = ((((u as i32) >> 15) as u32) ^ u) & 0x7fff;
    // 1/32767 as the engine stores it (0x38000100).
    bits as f32 * f32::from_bits(0x3800_0100)
}

/// Interleaves the low six bits of `a` and `b` (a5..a0 with b5..b0) the way
/// the 2D form of seeded `random` does.
fn interleave_bits(a: i32, b: i32) -> i32 {
    let bit = |v: i32, n: u32| (v >> n) & 1;
    let mut r = 0;
    for n in 0..5 {
        r = (r << 1) | bit(a, n);
        r = (r << 1) | bit(b, n);
    }
    (r << 2) | bit(b, 5) | ((a >> 4) & 2)
}

#[cfg(test)]
mod seeded_tests {
    use super::*;

    #[test]
    fn seeded_random_matches_the_engine() {
        // 15-bit hashes behind the oracle's `seed random 1` (0 -> 0.573565,
        // 42 -> 0.102329, ...).
        let cases: &[(i32, u16)] = &[
            (0, 18794),
            (1, 11421),
            (2, 13685),
            (3, 28606),
            (4, 12892),
            (42, 3353),
            (-5, 12892),
            (1_000_000, 11934),
        ];
        for (seed, bits) in cases {
            let want = f32::from(*bits) * f32::from_bits(0x3800_0100);
            assert_eq!(seeded_random(*seed), want, "seed {seed}");
        }
    }
}
