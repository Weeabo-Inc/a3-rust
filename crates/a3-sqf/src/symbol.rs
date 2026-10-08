//! Interned, case-insensitive names for variables.
//!
//! SQF variable names compare without regard to ASCII case (`_X` and `_x`
//! are the same variable). A [`Sym`] is the interned lower-case form, so the
//! VM compares variables by integer.
//!
//! The interner is process-wide and never frees names. Names come from
//! identifiers in compiled source and from strings passed to
//! `getVariable`/`setVariable`; the engine keeps such names alive too.

use std::collections::HashMap;
use std::fmt;
use std::sync::{OnceLock, RwLock};

const LOCAL_BIT: u32 = 1 << 31;

/// An interned variable name (lower-cased).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Sym(u32);

struct Interner {
    map: HashMap<&'static str, u32>,
    names: Vec<&'static str>,
}

fn interner() -> &'static RwLock<Interner> {
    static INTERNER: OnceLock<RwLock<Interner>> = OnceLock::new();
    INTERNER.get_or_init(|| {
        RwLock::new(Interner {
            map: HashMap::new(),
            names: Vec::new(),
        })
    })
}

impl Sym {
    /// Interns `name`, ignoring ASCII case.
    pub fn new(name: &str) -> Sym {
        let lower_owned;
        let lower: &str = if name.bytes().any(|b| b.is_ascii_uppercase()) {
            lower_owned = name.to_ascii_lowercase();
            &lower_owned
        } else {
            name
        };
        {
            let guard = interner().read().unwrap_or_else(|e| e.into_inner());
            if let Some(&id) = guard.map.get(lower) {
                return Sym(id);
            }
        }
        let mut guard = interner().write().unwrap_or_else(|e| e.into_inner());
        if let Some(&id) = guard.map.get(lower) {
            return Sym(id);
        }
        let leaked: &'static str = Box::leak(lower.to_string().into_boxed_str());
        let mut id = guard.names.len() as u32;
        if leaked.starts_with('_') {
            id |= LOCAL_BIT;
        }
        guard.names.push(leaked);
        guard.map.insert(leaked, id);
        Sym(id)
    }

    /// The lower-case name.
    pub fn as_str(self) -> &'static str {
        let guard = interner().read().unwrap_or_else(|e| e.into_inner());
        guard.names[(self.0 & !LOCAL_BIT) as usize]
    }

    /// Whether this names a local variable (starts with `_`).
    pub fn is_local(self) -> bool {
        self.0 & LOCAL_BIT != 0
    }
}

impl fmt::Debug for Sym {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Sym({})", self.as_str())
    }
}

impl fmt::Display for Sym {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interning_ignores_ascii_case() {
        assert_eq!(Sym::new("_myVar"), Sym::new("_MYVAR"));
        assert_ne!(Sym::new("_a"), Sym::new("_b"));
        assert_eq!(Sym::new("Foo").as_str(), "foo");
        assert!(Sym::new("_a").is_local());
        assert!(!Sym::new("a").is_local());
    }
}
