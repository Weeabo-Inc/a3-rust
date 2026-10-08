//! Vector math, random-number forms, `parseSimpleArray` and other
//! world-independent helpers.

use super::*;

use crate::lexer::{TokenKind, tokenize};

use crate::vm::Ctx;

fn vector(v: &Value) -> Result<[f32; 3], SqfError> {
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
    Ok(out)
}

fn vec_value(v: [f32; 3]) -> Value {
    Value::array(v.into_iter().map(Value::Number))
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
    vec2!(r, "vectorAdd", ARR, |a, b| vec_value([
        a[0] + b[0],
        a[1] + b[1],
        a[2] + b[2]
    ]));
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
    // seed random x: deterministic for a given seed.
    r.binary("random", NUM, NUM, NUM, |_, a, b| {
        let mut rng = crate::vm::Rng::new(u64::from(num(&a).to_bits()) ^ 0x9E37_79B9_7F4A_7C15);
        rng.next_u64();
        Ok(Value::Number(rng.next_f32() * num(&b)))
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

    r.unary("parseSimpleArray", STR, ARR, |_, a| {
        Ok(parse_simple_array(string(&a)).unwrap_or_else(|| Value::array([])))
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
/// (no expressions), as `parseSimpleArray` does. `None` on any error.
pub(crate) fn parse_simple_array(text: &str) -> Option<Value> {
    let tokens = tokenize(text).ok()?;
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
                let mut out = Vec::new();
                if tokens.get(*pos)?.kind == TokenKind::RBracket {
                    *pos += 1;
                    return Some(Value::array(out));
                }
                loop {
                    out.push(item(tokens, pos, src)?);
                    let t = tokens.get(*pos)?;
                    *pos += 1;
                    match t.kind {
                        TokenKind::Comma => {}
                        TokenKind::RBracket => return Some(Value::array(out)),
                        _ => return None,
                    }
                }
            }
            _ => None,
        }
    }
    if tokens.first()?.kind != TokenKind::LBracket {
        return None;
    }
    let v = item(&tokens, &mut pos, src)?;
    (tokens.get(pos)?.kind == TokenKind::Eof).then_some(v)
}
