//! Type inspection, conversion, compilation, logging, time, sides and null
//! values.

use super::*;
use crate::value::{Handle, HandleKind, Side};
use crate::vm::Ctx;

pub(super) fn register<H: Host>(r: &mut Registry<H>) {
    r.unary("typeName", ANY, STR, |_, a| {
        Ok(Value::from(a.ty().type_name()))
    });
    r.unary("str", ANY, STR, |ctx, a| {
        Ok(Value::from(ctx.to_sqf_string(&a)))
    });
    r.unary("format", ARR, STR, |ctx, a| {
        Ok(Value::from(format(ctx, &a)?))
    });

    r.unary("compile", STR, CODE, |ctx, a| {
        let code = ctx
            .compile("", string(&a))
            .map_err(|e| SqfError::Generic(e.message))?;
        Ok(Value::Code(code))
    });
    r.unary("compileFinal", STR, CODE, |ctx, a| {
        let code = ctx
            .compile("", string(&a))
            .map_err(|e| SqfError::Generic(e.message))?;
        Ok(Value::Code(code.to_final()))
    });
    r.unary("compileFinal", CODE, CODE, |_, a| {
        Ok(Value::Code(expect_code(&a)?.to_final()))
    });
    r.unary("isFinal", CODE, BOOL, |_, a| {
        Ok(Value::Bool(expect_code(&a)?.is_final()))
    });
    r.unary("isFinal", STR, BOOL, |ctx, a| {
        let v = ctx.get_var(crate::symbol::Sym::new(string(&a)));
        Ok(Value::Bool(matches!(v, Value::Code(c) if c.is_final())))
    });
    r.unary("toString", CODE, STR, |_, a| {
        Ok(Value::from(expect_code(&a)?.source()))
    });
    r.unary("toString", ARR, STR, |_, a| {
        let mut s = String::new();
        for v in array(&a).borrow().iter() {
            let n = expect_num(v)? as u32;
            s.push(char::from_u32(n).unwrap_or('\u{FFFD}'));
        }
        Ok(Value::from(s))
    });

    r.unary("diag_log", ANY, NOTHING, |ctx, a| {
        let text = ctx.to_display_string(&a);
        ctx.host.diag_log(&text);
        Ok(Value::Nothing)
    });
    r.unary("systemChat", STR, NOTHING, |ctx, a| {
        ctx.host.system_chat(string(&a));
        Ok(Value::Nothing)
    });
    for name in ["hint", "hintSilent"] {
        r.unary(name, STR, NOTHING, |ctx, a| {
            ctx.host.hint(string(&a));
            Ok(Value::Nothing)
        });
    }
    r.unary("copyToClipboard", STR, NOTHING, |ctx, a| {
        ctx.host.copy_to_clipboard(string(&a));
        Ok(Value::Nothing)
    });
    r.unary("scriptName", STR, NOTHING, |ctx, a| {
        ctx.set_script_name(string(&a));
        Ok(Value::Nothing)
    });

    r.nular("time", NUM, |ctx| Ok(Value::Number(ctx.host.time())));
    r.nular("serverTime", NUM, |ctx| {
        Ok(Value::Number(ctx.host.server_time()))
    });
    r.nular("diag_tickTime", NUM, |ctx| {
        Ok(Value::Number(ctx.host.tick_time()))
    });
    r.nular("diag_frameNo", NUM, |ctx| {
        Ok(Value::Number(ctx.host.frame_no() as f32))
    });

    r.unary("preprocessFile", STR, STR, |ctx, a| {
        let text = ctx
            .host
            .preprocess_file(string(&a), false)
            .unwrap_or_default();
        Ok(Value::from(text))
    });
    r.unary("preprocessFileLineNumbers", STR, STR, |ctx, a| {
        let text = ctx
            .host
            .preprocess_file(string(&a), true)
            .unwrap_or_default();
        Ok(Value::from(text))
    });
    r.unary("loadFile", STR, STR, |ctx, a| {
        Ok(Value::from(
            ctx.host.load_file(string(&a)).unwrap_or_default(),
        ))
    });

    // Sides.
    for (name, side) in [
        ("west", Side::West),
        ("blufor", Side::West),
        ("east", Side::East),
        ("opfor", Side::East),
        ("resistance", Side::Independent),
        ("independent", Side::Independent),
        ("civilian", Side::Civilian),
        ("sideUnknown", Side::Unknown),
        ("sideEnemy", Side::Enemy),
        ("sideFriendly", Side::Friendly),
        ("sideLogic", Side::Logic),
        ("sideEmpty", Side::Empty),
        ("sideAmbientLife", Side::AmbientLife),
    ] {
        register_side(r, name, side);
    }

    // Null values of host handles.
    for (name, kind) in [
        ("objNull", HandleKind::Object),
        ("grpNull", HandleKind::Group),
        ("controlNull", HandleKind::Control),
        ("displayNull", HandleKind::Display),
        ("locationNull", HandleKind::Location),
        ("taskNull", HandleKind::Task),
        ("teamMemberNull", HandleKind::TeamMember),
        ("diaryRecordNull", HandleKind::DiaryRecord),
        ("configNull", HandleKind::Config),
    ] {
        register_null(r, name, kind);
    }
    let handles = [
        Type::Object,
        Type::Group,
        Type::Control,
        Type::Display,
        Type::Location,
        Type::Task,
        Type::TeamMember,
        Type::DiaryRecord,
        Type::Config,
        Type::NetObject,
    ]
    .into_iter()
    .fold(TypeSet::EMPTY, |acc, t| acc | t);
    r.unary("isNull", handles, BOOL, |ctx, a| {
        let Value::Handle(h) = a else { unreachable!() };
        Ok(Value::Bool(ctx.host.is_null(h)))
    });
    r.unary("isNull", SCRIPT, BOOL, |ctx, a| {
        let Value::Script(h) = a else { unreachable!() };
        Ok(Value::Bool(h.0 == 0 || ctx.scheduler().is_done(h)))
    });
}

/// Registers a nular returning a side. A macro-free way to get one fn
/// pointer per side.
fn register_side<H: Host>(r: &mut Registry<H>, name: &str, side: Side) {
    let f: crate::registry::NularFn<H> = match side {
        Side::West => |_| Ok(Value::Side(Side::West)),
        Side::East => |_| Ok(Value::Side(Side::East)),
        Side::Independent => |_| Ok(Value::Side(Side::Independent)),
        Side::Civilian => |_| Ok(Value::Side(Side::Civilian)),
        Side::Unknown => |_| Ok(Value::Side(Side::Unknown)),
        Side::Enemy => |_| Ok(Value::Side(Side::Enemy)),
        Side::Friendly => |_| Ok(Value::Side(Side::Friendly)),
        Side::Logic => |_| Ok(Value::Side(Side::Logic)),
        Side::Empty => |_| Ok(Value::Side(Side::Empty)),
        Side::AmbientLife => |_| Ok(Value::Side(Side::AmbientLife)),
    };
    r.nular(name, SIDE, f);
}

fn register_null<H: Host>(r: &mut Registry<H>, name: &str, kind: HandleKind) {
    macro_rules! null {
        ($k:expr) => {
            |_| Ok(Value::Handle(Handle::null($k)))
        };
    }
    let f: crate::registry::NularFn<H> = match kind {
        HandleKind::Object => null!(HandleKind::Object),
        HandleKind::Group => null!(HandleKind::Group),
        HandleKind::Control => null!(HandleKind::Control),
        HandleKind::Display => null!(HandleKind::Display),
        HandleKind::Location => null!(HandleKind::Location),
        HandleKind::Task => null!(HandleKind::Task),
        HandleKind::TeamMember => null!(HandleKind::TeamMember),
        HandleKind::DiaryRecord => null!(HandleKind::DiaryRecord),
        HandleKind::Config => null!(HandleKind::Config),
        HandleKind::Text => null!(HandleKind::Text),
        HandleKind::NetObject => null!(HandleKind::NetObject),
        HandleKind::Target => null!(HandleKind::Target),
        HandleKind::SubGroup => null!(HandleKind::SubGroup),
    };
    r.nular(name, TypeSet::of(kind.ty()), f);
}

/// `format [fmt, args...]`: `%1`..`%N` are replaced by the arguments as
/// `str` shows them, except that strings are not quoted. A `%` not followed
/// by a digit is kept.
fn format<H: Host>(ctx: &Ctx<'_, H>, a: &Value) -> Result<String, SqfError> {
    let arr = array(a);
    let items = arr.borrow();
    let Some(fmt) = items.first() else {
        return Ok(String::new());
    };
    let fmt = expect_str(fmt)?;
    let mut out = String::with_capacity(fmt.len());
    let mut rest = fmt;
    while let Some(pos) = rest.find('%') {
        out.push_str(&rest[..pos]);
        let after = &rest[pos + 1..];
        let digits = after.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            out.push('%');
            rest = after;
            continue;
        }
        let n: usize = after[..digits].parse().unwrap_or(0);
        if let Some(v) = items.get(n).filter(|_| n > 0) {
            out.push_str(&ctx.to_display_string(v));
        }
        rest = &after[digits..];
    }
    out.push_str(rest);
    Ok(out)
}
