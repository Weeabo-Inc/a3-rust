//! The in-memory form of one config file (one config.cpp / config.bin), before merging.
//!
//! Names keep their original case; every lookup compares names ASCII-case-insensitively, as the
//! engine does.

/// One whole config file: the anonymous root class and the enum table.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Config {
    /// The root class. Its `base` is always `None` in well-formed files.
    pub root: ConfigClass,
    /// `enum { NAME = value, ... };` constants, in file order.
    pub enums: Vec<EnumEntry>,
}

/// One constant of a config `enum` block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumEntry {
    pub name: String,
    pub value: i32,
}

/// The body of a config class: an optional base class name and its ordered entries.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ConfigClass {
    /// Name of the class this one inherits from (`class X: Base`), resolved by scope at lookup.
    pub base: Option<String>,
    pub entries: Vec<Entry>,
}

/// A named member of a config class.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub name: String,
    pub kind: EntryKind,
}

/// What a class member is.
#[derive(Debug, Clone, PartialEq)]
pub enum EntryKind {
    /// `class Name: Base { ... };`
    Class(ConfigClass),
    /// `class Name;` — a forward/external reference to a class defined elsewhere.
    External,
    /// `delete Name;` — removes a class from the merged config.
    Delete,
    /// `name = value;` or `name[] = {...};`
    Value(Value),
    /// `name[] += {...};` — appends to the inherited or previously defined array.
    ArrayAppend(Vec<Value>),
}

/// A config value: a scalar or a (possibly nested) array.
#[derive(Debug, Clone)]
pub enum Value {
    String(String),
    Float(f32),
    Int(i32),
    Int64(i64),
    /// A rapified "expression"/variable value (rap subtype 4), kept verbatim.
    Expression(String),
    Array(Vec<Value>),
}

impl PartialEq for Value {
    /// Structural equality; floats compare by bit pattern so `NaN == NaN` and `0.0 != -0.0`.
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::String(a), Self::String(b)) | (Self::Expression(a), Self::Expression(b)) => {
                a == b
            }
            (Self::Float(a), Self::Float(b)) => a.to_bits() == b.to_bits(),
            (Self::Int(a), Self::Int(b)) => a == b,
            (Self::Int64(a), Self::Int64(b)) => a == b,
            (Self::Array(a), Self::Array(b)) => a == b,
            _ => false,
        }
    }
}

impl Entry {
    pub fn new(name: impl Into<String>, kind: EntryKind) -> Self {
        Self {
            name: name.into(),
            kind,
        }
    }

    /// Shorthand for a scalar or array value entry.
    pub fn value(name: impl Into<String>, value: Value) -> Self {
        Self::new(name, EntryKind::Value(value))
    }

    /// Shorthand for a class entry.
    pub fn class(name: impl Into<String>, class: ConfigClass) -> Self {
        Self::new(name, EntryKind::Class(class))
    }
}

impl ConfigClass {
    pub fn new(base: Option<&str>, entries: Vec<Entry>) -> Self {
        Self {
            base: base.map(str::to_owned),
            entries,
        }
    }

    /// The first entry with this name (ASCII case-insensitive), ignoring inheritance.
    pub fn get(&self, name: &str) -> Option<&Entry> {
        self.entries
            .iter()
            .find(|e| e.name.eq_ignore_ascii_case(name))
    }

    /// The child class body with this name, if the entry exists and is a class.
    pub fn class(&self, name: &str) -> Option<&ConfigClass> {
        match &self.get(name)?.kind {
            EntryKind::Class(c) => Some(c),
            _ => None,
        }
    }
}
