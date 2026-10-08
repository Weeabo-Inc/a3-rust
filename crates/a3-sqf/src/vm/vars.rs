//! Global variable storage.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

use crate::symbol::Sym;
use crate::value::{Namespace, Value};

/// Multiplicative hasher for [`Sym`] keys (already unique integers).
#[derive(Default)]
pub struct SymHasher(u64);

impl Hasher for SymHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0.rotate_left(8) ^ u64::from(b)).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        }
    }

    fn write_u32(&mut self, n: u32) {
        self.0 = (self.0 ^ u64::from(n)).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    }
}

type SymMap<V> = HashMap<Sym, V, BuildHasherDefault<SymHasher>>;

/// The variables of one namespace. Setting a variable to `nil` deletes it.
#[derive(Default, Clone)]
pub struct Variables {
    map: SymMap<Value>,
}

impl Variables {
    pub fn get(&self, name: Sym) -> Option<&Value> {
        self.map.get(&name)
    }

    /// Sets a variable; `nil` removes it.
    pub fn set(&mut self, name: Sym, value: Value) {
        if value.is_nil() {
            self.map.remove(&name);
        } else {
            self.map.insert(name, value);
        }
    }

    pub fn remove(&mut self, name: Sym) -> Option<Value> {
        self.map.remove(&name)
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// All variables, in no particular order.
    pub fn iter(&self) -> impl Iterator<Item = (Sym, &Value)> {
        self.map.iter().map(|(k, v)| (*k, v))
    }
}

/// The variables of every namespace.
#[derive(Default, Clone)]
pub struct Namespaces {
    spaces: [Variables; 7],
}

impl Namespaces {
    fn index(ns: Namespace) -> usize {
        match ns {
            Namespace::Mission => 0,
            Namespace::Ui => 1,
            Namespace::Profile => 2,
            Namespace::Parsing => 3,
            Namespace::Local => 4,
            Namespace::MissionProfile => 5,
            Namespace::Server => 6,
        }
    }

    pub fn get(&self, ns: Namespace) -> &Variables {
        &self.spaces[Self::index(ns)]
    }

    pub fn get_mut(&mut self, ns: Namespace) -> &mut Variables {
        &mut self.spaces[Self::index(ns)]
    }
}
