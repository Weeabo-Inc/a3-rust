//! Structured text: `text`, `parseText`, `composeText`, `formatText`,
//! `lineBreak`, `image`.
//!
//! A structured text value keeps its markup. Plain strings put into it are
//! XML-escaped, so `composeText ["a", lineBreak]` equals `parseText
//! "a<br/>"` (the wiki's `lineBreak` and `image` examples).

use super::*;
use crate::value::{StructuredText, escape_xml};
use crate::vm::Ctx;

const TEXT: TypeSet = TypeSet::of(Type::Text);

fn markup_of<H: Host>(ctx: &Ctx<'_, H>, v: &Value) -> String {
    match v {
        Value::Text(t) => t.markup().to_string(),
        other => escape_xml(&ctx.to_display_string(other)),
    }
}

pub(super) fn register<H: Host>(r: &mut Registry<H>) {
    r.unary("text", STR, TEXT, |_, a| {
        Ok(Value::Text(StructuredText::from_plain(string(&a))))
    });
    r.unary("parseText", STR, TEXT, |_, a| {
        Ok(Value::Text(StructuredText::from_markup(string(&a))))
    });
    r.nular("lineBreak", TEXT, |_| {
        Ok(Value::Text(StructuredText::from_markup("<br/>")))
    });
    r.unary("image", STR, TEXT, |_, a| {
        Ok(Value::Text(StructuredText::from_markup(&format!(
            "<img image='{}'/>",
            escape_xml(string(&a))
        ))))
    });
    r.unary("composeText", ARR, TEXT, |ctx, a| {
        let mut out = String::new();
        for v in array(&a).borrow().iter() {
            out.push_str(&markup_of(ctx, v));
        }
        Ok(Value::Text(StructuredText::from_markup(&out)))
    });
    r.unary("formatText", ARR, TEXT, |ctx, a| {
        let arr = array(&a);
        let items = arr.borrow();
        let Some(fmt) = items.first() else {
            return Ok(Value::Text(StructuredText::from_markup("")));
        };
        let fmt = expect_str(fmt)?;
        let mut out = String::new();
        let mut rest = fmt;
        while let Some(pos) = rest.find('%') {
            out.push_str(&escape_xml(&rest[..pos]));
            let after = &rest[pos + 1..];
            let digits = after.bytes().take_while(u8::is_ascii_digit).count();
            if digits == 0 {
                out.push('%');
                rest = after;
                continue;
            }
            let n: usize = after[..digits].parse().unwrap_or(0);
            if let Some(v) = items.get(n).filter(|_| n > 0) {
                out.push_str(&markup_of(ctx, v));
            }
            rest = &after[digits..];
        }
        out.push_str(&escape_xml(rest));
        Ok(Value::Text(StructuredText::from_markup(&out)))
    });
    r.unary("hint", TEXT, NOTHING, |ctx, a| {
        if let Value::Text(t) = &a {
            ctx.host.hint(&t.plain());
        }
        Ok(Value::Nothing)
    });
    r.unary("hintSilent", TEXT, NOTHING, |ctx, a| {
        if let Value::Text(t) = &a {
            ctx.host.hint_silent(&t.plain());
        }
        Ok(Value::Nothing)
    });
}
