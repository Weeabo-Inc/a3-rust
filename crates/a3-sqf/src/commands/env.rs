//! Commands that ask the host about its environment (clipboard, files,
//! frame timing, profile saving) and more diagnostics: `diag_scope`,
//! `diag_stacktrace`.

use indexmap::IndexMap;

use super::*;
use crate::symbol::Sym;
use crate::value::{HashKey, HashMap, Namespace};

/// `requiredVersion "2.22"`: whether this build is at least the version.
fn version_at_least(wanted: &str) -> bool {
    let ours = [2u32, 22, 154_103];
    let parts: Vec<u32> = wanted
        .split('.')
        .map(|p| p.trim().parse().unwrap_or(0))
        .collect();
    for (i, want) in parts.iter().enumerate() {
        let have = ours.get(i).copied().unwrap_or(0);
        if have != *want {
            return have > *want;
        }
    }
    true
}

pub(super) fn register<H: Host>(r: &mut Registry<H>) {
    r.nular("copyFromClipboard", STR, |ctx| {
        Ok(Value::from(ctx.host.clipboard()))
    });
    r.unary("fileExists", STR, BOOL, |ctx, a| {
        Ok(Value::Bool(ctx.host.file_exists(string(&a))))
    });
    r.nular("diag_fps", NUM, |ctx| Ok(Value::Number(ctx.host.fps())));
    r.nular("diag_fpsmin", NUM, |ctx| {
        Ok(Value::Number(ctx.host.fps_min()))
    });
    r.nular("diag_deltaTime", NUM, |ctx| {
        Ok(Value::Number(ctx.host.delta_time()))
    });
    r.nular("diag_activeSQSScripts", ARR, |_| Ok(Value::array([])));
    r.nular("diag_activeMissionFSMs", ARR, |_| Ok(Value::array([])));
    r.nular("diag_scope", NUM, |ctx| {
        Ok(Value::Number(ctx.scope_depth() as f32))
    });
    r.nular("diag_stacktrace", ARR, |ctx| {
        let frames = ctx.stack_trace();
        Ok(Value::array(frames.into_iter().map(|f| {
            let mut vars = IndexMap::new();
            for (name, v) in f.locals {
                vars.insert(HashKey::String(name.as_str().into()), v);
            }
            Value::array([
                Value::from(""),
                Value::Number(f.line as f32),
                Value::from(f.scope_name.as_deref().unwrap_or("")),
                Value::HashMap(HashMap::from_map(vars)),
            ])
        })))
    });
    r.unary("requiredVersion", STR, BOOL, |_, a| {
        Ok(Value::Bool(version_at_least(string(&a))))
    });
    r.unary("isLocalized", STR, BOOL, |ctx, a| {
        let key = string(&a);
        let key = key.strip_prefix('$').unwrap_or(key);
        Ok(Value::Bool(ctx.host.localize(key).is_some()))
    });
    for name in ["debugLog", "textLog"] {
        r.unary(name, ANY, NOTHING, |ctx, a| {
            let text = ctx.to_display_string(&a);
            ctx.host.diag_log(&text);
            Ok(Value::Nothing)
        });
    }
    r.nular("saveProfileNamespace", NOTHING, |ctx| {
        let vars = ctx.namespace(Namespace::Profile).clone();
        ctx.host.save_profile_namespace(&vars);
        Ok(Value::Nothing)
    });
    r.nular("saveMissionProfileNamespace", BOOL, |ctx| {
        let vars = ctx.namespace(Namespace::MissionProfile).clone();
        Ok(Value::Bool(ctx.host.save_mission_profile_namespace(&vars)))
    });
    r.nular("isMissionProfileNamespaceLoaded", BOOL, |_| {
        Ok(Value::Bool(false))
    });
    r.binary("isNil", NS, STR, BOOL, |ctx, a, b| {
        let Value::Namespace(ns) = a else {
            unreachable!()
        };
        Ok(Value::Bool(
            ctx.namespace(ns)
                .get(Sym::new(string(&b)))
                .is_none_or(Value::is_nil),
        ))
    });
    r.unary("reverse", STR, STR, |ctx, a| {
        let s = string(&a);
        Ok(Value::from(if ctx.take_unicode() {
            s.chars().rev().collect::<String>()
        } else {
            let mut bytes = s.as_bytes().to_vec();
            bytes.reverse();
            String::from_utf8_lossy(&bytes).into_owned()
        }))
    });
    r.binary("insert", STR, ARR, STR, |ctx, a, b| {
        let args = array(&b);
        let args = args.borrow();
        let at = index(expect_num(args.first().unwrap_or(&Value::Nil))?);
        let piece = expect_str(args.get(1).unwrap_or(&Value::Nil))?.to_string();
        let s = string(&a);
        Ok(Value::from(if ctx.take_unicode() {
            let mut chars: Vec<char> = s.chars().collect();
            let pos = if at < 0 {
                chars.len()
            } else {
                (at as usize).min(chars.len())
            };
            chars.splice(pos..pos, piece.chars());
            chars.into_iter().collect::<String>()
        } else {
            let mut bytes = s.as_bytes().to_vec();
            let pos = if at < 0 {
                bytes.len()
            } else {
                (at as usize).min(bytes.len())
            };
            bytes.splice(pos..pos, piece.bytes());
            String::from_utf8_lossy(&bytes).into_owned()
        }))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert!(version_at_least("2.22"));
        assert!(version_at_least("2.06"));
        assert!(!version_at_least("2.24"));
        assert!(version_at_least("1.98"));
        assert!(!version_at_least("3"));
    }
}
