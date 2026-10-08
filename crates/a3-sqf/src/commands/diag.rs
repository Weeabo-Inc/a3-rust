//! Diagnostics, introspection and other world-independent helpers:
//! `supportInfo`, `diag_codePerformance`, `productVersion`, `privateAll`/
//! `import`, `breakWith`, `forEachReversed`, date and matrix math.

use std::time::Instant;

use super::*;
use crate::symbol::Sym;
use crate::table::Form;
use crate::vm::{Continuation, ContinuationKind, Ctx, Flow, Invoke, Unwind};

/// `diag_codePerformance`: runs code repeatedly and averages the time.
struct CodePerformance {
    code: Code,
    args: Value,
    cycles: u32,
    done: u32,
    start: Instant,
}

impl<H: Host> Continuation<H> for CodePerformance {
    fn resume(&mut self, _: &mut Ctx<'_, H>, _: Value) -> Result<Flow<H>, SqfError> {
        self.done += 1;
        if self.done >= self.cycles {
            let ms = self.start.elapsed().as_secs_f64() * 1000.0 / f64::from(self.cycles);
            return Ok(Flow::Value(Value::array([
                Value::Number(ms as f32),
                Value::Number(self.cycles as f32),
            ])));
        }
        Ok(Flow::Call(Invoke::with_this(
            self.code.clone(),
            self.args.clone(),
        )))
    }
}

/// `forEachReversed`.
struct ReverseLoop {
    body: Code,
    items: Vec<Value>,
    index: usize,
    last: Value,
}

impl ReverseLoop {
    fn next<H: Host>(&mut self) -> Flow<H> {
        if self.index == 0 {
            return Flow::Value(std::mem::replace(&mut self.last, Value::Nothing));
        }
        self.index -= 1;
        Flow::Call(
            Invoke::new(self.body.clone())
                .local(Sym::X, self.items[self.index].clone())
                .local(Sym::FOR_EACH_INDEX, Value::Number(self.index as f32)),
        )
    }
}

impl<H: Host> Continuation<H> for ReverseLoop {
    fn resume(&mut self, _: &mut Ctx<'_, H>, result: Value) -> Result<Flow<H>, SqfError> {
        self.last = result;
        Ok(self.next())
    }

    fn kind(&self) -> ContinuationKind {
        ContinuationKind::Loop
    }
}

fn is_leap(y: i32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn days_in_month(y: i32, m: i32) -> i32 {
    match m {
        2 if is_leap(y) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// Glob-style match with `*` (any run) and `?` (one character), ignoring
/// ASCII case.
fn wildcard(pattern: &str, text: &str) -> bool {
    let (p, t): (Vec<char>, Vec<char>) = (
        pattern.to_ascii_lowercase().chars().collect(),
        text.to_ascii_lowercase().chars().collect(),
    );
    let (mut pi, mut ti, mut star, mut mark) = (0, 0, None, 0);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == '*')
}

/// The `supportInfo` entries of the command table as `(kind, name, text)`:
/// `t:TYPE`, `n:name`, `u:name RIGHT`, `b:LEFT name RIGHT`.
fn support_entries(table: &crate::table::CommandTable) -> Vec<(char, String, String)> {
    let mut out: Vec<(char, String, String)> = Type::ALL
        .iter()
        .filter(|t| !matches!(t, Type::Any | Type::Nothing))
        .map(|t| {
            (
                't',
                t.type_name().to_string(),
                format!("t:{}", t.type_name()),
            )
        })
        .collect();
    let first = |s: TypeSet| s.iter().next().map(Type::type_name).unwrap_or("ANY");
    for (_, info) in table.iter() {
        let name = &info.name;
        if info.has_form(Form::Nular) {
            out.push(('n', name.clone(), format!("n:{name}")));
        }
        for s in &info.unary {
            if s.right == TypeSet::ANYTHING {
                out.push(('u', name.clone(), format!("u:{name} ANY")));
                continue;
            }
            for t in s.right.iter().filter(|t| *t != Type::NaN) {
                out.push(('u', name.clone(), format!("u:{name} {}", t.type_name())));
            }
        }
        for s in &info.binary {
            out.push((
                'b',
                name.clone(),
                format!("b:{} {name} {}", first(s.left), first(s.right)),
            ));
        }
    }
    out
}

/// Whether a `supportInfo` mask (`kind:name`, wildcards allowed in both
/// parts) selects an entry.
fn support_match(mask: &str, kind: char, name: &str) -> bool {
    let (k, n) = mask.split_once(':').unwrap_or(("*", mask));
    wildcard(k, &kind.to_string()) && wildcard(n, name)
}
pub(super) fn register<H: Host>(r: &mut Registry<H>) {
    r.unary_flow("breakWith", ANY, NOTHING, |_, a| {
        Ok(Flow::Unwind(Unwind::BreakWith(a)))
    });
    r.binary_flow("forEachReversed", CODE, ARR, ANY, |_, a, b| {
        let items = array(&b).borrow().clone();
        let mut l = ReverseLoop {
            body: expect_code(&a)?,
            index: items.len(),
            items,
            last: Value::Nothing,
        };
        match l.next::<H>() {
            Flow::Call(inv) => Ok(Flow::CallThen(inv, Box::new(l))),
            other => Ok(other),
        }
    });
    r.nular("privateAll", NOTHING, |ctx| {
        ctx.set_private_all();
        Ok(Value::Nothing)
    });
    r.unary("import", STR, NOTHING, |ctx, a| {
        import(ctx, &a)?;
        Ok(Value::Nothing)
    });
    r.unary("import", ARR, NOTHING, |ctx, a| {
        for v in array(&a).borrow().iter() {
            import(ctx, v)?;
        }
        Ok(Value::Nothing)
    });
    r.unary("assert", BOOL, BOOL, |ctx, a| {
        if !boolean(&a) {
            ctx.report(SqfError::generic("Assertion failed"));
        }
        Ok(a)
    });
    r.unary("comment", STR, NOTHING, |_, _| Ok(Value::Nothing));
    r.nular("endl", STR, |_| Ok(Value::from("\r\n")));
    r.binary("isNotEqualRef", ANY, ANY, BOOL, |_, a, b| {
        Ok(Value::Bool(!match (&a, &b) {
            (Value::Array(x), Value::Array(y)) => x.ptr_eq(y),
            (Value::HashMap(x), Value::HashMap(y)) => x.ptr_eq(y),
            (Value::Code(x), Value::Code(y)) => x.ptr_eq(y),
            _ => a.is_equal_to(&b),
        }))
    });
    r.unary("toLowerANSI", STR, STR, |_, a| {
        Ok(Value::from(string(&a).to_ascii_lowercase()))
    });
    r.unary("toUpperANSI", STR, STR, |_, a| {
        Ok(Value::from(string(&a).to_ascii_uppercase()))
    });
    r.nular("productVersion", ARR, |_| {
        Ok(Value::array([
            Value::from("Arma 3"),
            Value::from("Arma3"),
            Value::Number(222.0),
            Value::Number(154_103.0),
            Value::from("Stable"),
            Value::Bool(false),
            Value::from(if cfg!(windows) { "Windows" } else { "Linux" }),
            Value::from("x64"),
        ]))
    });
    r.nular("systemTimeUTC", ARR, |_| Ok(system_time()));
    r.nular("systemTime", ARR, |_| Ok(system_time()));
    r.nular("uiTime", NUM, |ctx| Ok(Value::Number(ctx.host.tick_time())));
    r.unary_flow("diag_codePerformance", ARR, ARR, |_, a| {
        let args = array(&a);
        let args = args.borrow();
        let code = expect_code(args.first().unwrap_or(&Value::Nil))?;
        let this = args.get(1).cloned().unwrap_or(Value::Nil);
        let cycles = args.get(2).map(num).unwrap_or(1.0).max(1.0) as u32;
        Ok(Flow::CallThen(
            Invoke::with_this(code.clone(), this.clone()),
            Box::new(CodePerformance {
                code,
                args: this,
                cycles,
                done: 0,
                start: Instant::now(),
            }),
        ))
    });
    r.nular("diag_activeSQFScripts", ARR, |ctx| {
        Ok(Value::array(ctx.scheduled_scripts().into_iter().map(
            |(_, name)| {
                Value::array([
                    Value::from(name.as_deref().unwrap_or("")),
                    Value::from(""),
                    Value::Bool(true),
                    Value::Number(0.0),
                ])
            },
        )))
    });
    r.nular("diag_activeScripts", ARR, |ctx| {
        let n = ctx.scheduled_scripts().len() as f32;
        Ok(Value::array([n, 0.0, 0.0, 0.0].map(Value::Number)))
    });
    r.unary("supportInfo", STR, ARR, |ctx, a| {
        let mask = string(&a);
        let mask = if mask.is_empty() { "*:*" } else { mask };
        Ok(Value::array(
            support_entries(ctx.table())
                .into_iter()
                .filter(|(k, n, _)| support_match(mask, *k, n))
                .map(|(_, _, text)| Value::from(text)),
        ))
    });
    r.unary("dateToNumber", ARR, NUM, |_, a| {
        let d: Vec<f32> = array(&a).borrow().iter().map(num).collect();
        let g = |i: usize| d.get(i).copied().unwrap_or(0.0);
        let (y, m, day) = (g(0) as i32, (g(1) as i32).clamp(1, 12), g(2));
        let year_days = if is_leap(y) { 366.0 } else { 365.0 };
        let before: i32 = (1..m).map(|mm| days_in_month(y, mm)).sum();
        let t = before as f32 + (day - 1.0) + (g(3) + g(4) / 60.0) / 24.0;
        Ok(Value::Number(t / year_days))
    });
    r.unary("numberToDate", ARR, ARR, |_, a| {
        let d: Vec<f32> = array(&a).borrow().iter().map(num).collect();
        let y = d.first().copied().unwrap_or(2035.0) as i32;
        let year_days = if is_leap(y) { 366.0 } else { 365.0 };
        let mut t = f64::from(d.get(1).copied().unwrap_or(0.0)).rem_euclid(1.0) * year_days;
        let mut m = 1;
        while m < 12 && t >= f64::from(days_in_month(y, m)) {
            t -= f64::from(days_in_month(y, m));
            m += 1;
        }
        let day = t.floor();
        let minutes = ((t - day) * 24.0 * 60.0).round() as i32;
        Ok(Value::array([
            Value::Number(y as f32),
            Value::Number(m as f32),
            Value::Number(day as f32 + 1.0),
            Value::Number((minutes / 60) as f32),
            Value::Number((minutes % 60) as f32),
        ]))
    });
    r.binary("matrixMultiply", ARR, ARR, ARR, |_, a, b| {
        let (x, y) = (matrix(&a)?, matrix(&b)?);
        let inner = y.len();
        if x.iter().any(|row| row.len() != inner) {
            return Err(SqfError::generic("Matrix dimensions do not match"));
        }
        let cols = y.first().map_or(0, Vec::len);
        let out = x.iter().map(|row| {
            Value::array((0..cols).map(|c| {
                Value::Number(
                    (0..inner)
                        .map(|k| row[k] * y[k].get(c).copied().unwrap_or(0.0))
                        .sum(),
                )
            }))
        });
        Ok(Value::array(out))
    });
    r.unary("matrixTranspose", ARR, ARR, |_, a| {
        let m = matrix(&a)?;
        let cols = m.first().map_or(0, Vec::len);
        Ok(Value::array((0..cols).map(|c| {
            Value::array(
                m.iter()
                    .map(|row| Value::Number(row.get(c).copied().unwrap_or(0.0))),
            )
        })))
    });
    r.binary("vectorFromTo", ARR, ARR, ARR, |_, a, b| {
        let (p, q) = (numbers(&a)?, numbers(&b)?);
        let d: Vec<f32> = (0..3)
            .map(|i| q.get(i).copied().unwrap_or(0.0) - p.get(i).copied().unwrap_or(0.0))
            .collect();
        let len = d.iter().map(|v| v * v).sum::<f32>().sqrt();
        Ok(Value::array(d.into_iter().map(|v| {
            Value::Number(if len == 0.0 { 0.0 } else { v / len })
        })))
    });
    r.binary("bezierInterpolation", NUM, ARR, ARR, |_, a, b| {
        let t = num(&a).clamp(0.0, 1.0);
        let mut pts = matrix(&b)?;
        if pts.is_empty() {
            return Ok(Value::array([]));
        }
        while pts.len() > 1 {
            pts = pts
                .windows(2)
                .map(|w| {
                    w[0].iter()
                        .zip(w[1].iter())
                        .map(|(p, q)| p + (q - p) * t)
                        .collect()
                })
                .collect();
        }
        Ok(Value::array(pts[0].iter().map(|v| Value::Number(*v))))
    });
    r.unary("spawn", CODE, SCRIPT, |ctx, a| {
        Ok(Value::Script(ctx.spawn(expect_code(&a)?, Value::Nil, None)))
    });
}

fn numbers(v: &Value) -> Result<Vec<f32>, SqfError> {
    array(v).borrow().iter().map(expect_num).collect()
}

fn matrix(v: &Value) -> Result<Vec<Vec<f32>>, SqfError> {
    array(v)
        .borrow()
        .iter()
        .map(|row| match row {
            Value::Array(_) => numbers(row),
            other => Err(SqfError::type_error(other, ARR)),
        })
        .collect()
}

fn import<H: Host>(ctx: &mut Ctx<'_, H>, name: &Value) -> Result<(), SqfError> {
    let sym = Sym::new(expect_str(name)?);
    let v = ctx.get_local_through_barrier(sym).unwrap_or(Value::Nil);
    ctx.set_private(sym, v);
    Ok(())
}

fn system_time() -> Value {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs() as i64;
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    let rem = secs.rem_euclid(86_400);
    Value::array(
        [
            y as f32,
            m as f32,
            d as f32,
            (rem / 3600) as f32,
            ((rem % 3600) / 60) as f32,
            (rem % 60) as f32,
            now.subsec_millis() as f32,
        ]
        .map(Value::Number),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcards() {
        assert!(support_match("b:select*", 'b', "select"));
        assert!(support_match("b:select*", 'b', "selectMax"));
        assert!(!support_match("u:select*", 'b', "select"));
        assert!(support_match("n:pixelgrid", 'n', "pixelGrid"));
        assert!(support_match("*:*", 't', "SCALAR"));
    }

    #[test]
    fn civil_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
    }
}
