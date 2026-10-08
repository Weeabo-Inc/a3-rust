//! The SQF value model, mirroring the engine's `GameValue`.
//!
//! # Numbers are `f32`
//!
//! SQF numbers are IEEE-754 single precision: `16777217` reads back as
//! `16777216`, and `str 0.1` prints six significant digits through the C
//! `%g` format of the float widened to double. [`Value::Number`] therefore
//! holds an `f32` and all arithmetic is done in `f32`.
//!
//! # Arrays and hash maps are shared by reference
//!
//! An [`Array`] is a reference to one mutable vector. Assigning an array to
//! another variable, passing it to a function or storing it in another array
//! copies the reference, so `pushBack` through one reference is visible
//! through all of them. The unary `+` command and `+` between arrays build
//! new arrays. [`HashMap`] values behave the same way.
//!
//! # Values are not `Send`
//!
//! The VM runs on one thread (the simulation thread), as the engine's does,
//! so shared values use `Rc`/`RefCell`.

use std::cell::{Cell, Ref, RefCell, RefMut};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use indexmap::IndexMap;

use crate::code::Code;
use crate::symbol::Sym;
use crate::types::Type;

/// One SQF value.
#[derive(Clone, Default)]
pub enum Value {
    /// `nil`: an undefined variable or the `nil` command. `typeName` is `ANY`.
    #[default]
    Nil,
    /// The result of a command that returns nothing. `typeName` is `NOTHING`.
    Nothing,
    Bool(bool),
    Number(f32),
    String(Rc<str>),
    Array(Array),
    HashMap(HashMap),
    Code(Code),
    Namespace(Namespace),
    Side(Side),
    Script(ScriptHandle),
    /// An engine object owned by the host (object, group, control, ...).
    Handle(Handle),
    /// `if cond`: the evaluated condition.
    If(bool),
    /// `while {cond}`: the condition code.
    While(Code),
    /// `for ...`: the loop description being built.
    For(Rc<ForSpec>),
    /// `switch value`: the value being matched, or a `case` result inside a
    /// switch block.
    Switch(Rc<SwitchState>),
    /// `with namespace`.
    With(Namespace),
    /// `try {code}`: the code to run under `catch`.
    Exception(Code),
}

impl Value {
    /// The SQF type of this value.
    pub fn ty(&self) -> Type {
        match self {
            Value::Nil => Type::Any,
            Value::Nothing => Type::Nothing,
            Value::Bool(_) => Type::Bool,
            Value::Number(n) if n.is_nan() => Type::NaN,
            Value::Number(_) => Type::Number,
            Value::String(_) => Type::String,
            Value::Array(_) => Type::Array,
            Value::HashMap(_) => Type::HashMap,
            Value::Code(_) => Type::Code,
            Value::Namespace(_) => Type::Namespace,
            Value::Side(_) => Type::Side,
            Value::Script(_) => Type::Script,
            Value::Handle(h) => h.kind.ty(),
            Value::If(_) => Type::If,
            Value::While(_) => Type::While,
            Value::For(_) => Type::For,
            Value::Switch(_) => Type::Switch,
            Value::With(_) => Type::With,
            Value::Exception(_) => Type::Exception,
        }
    }

    /// Whether this is `nil`.
    pub fn is_nil(&self) -> bool {
        matches!(self, Value::Nil)
    }

    /// Builds a string value.
    pub fn string(s: impl AsRef<str>) -> Value {
        Value::String(Rc::from(s.as_ref()))
    }

    /// Builds a new array value from `items`.
    pub fn array(items: impl IntoIterator<Item = Value>) -> Value {
        Value::Array(Array::from_vec(items.into_iter().collect()))
    }

    pub fn as_number(&self) -> Option<f32> {
        match self {
            Value::Number(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&Array> {
        match self {
            Value::Array(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_code(&self) -> Option<&Code> {
        match self {
            Value::Code(c) => Some(c),
            _ => None,
        }
    }

    pub fn as_hashmap(&self) -> Option<&HashMap> {
        match self {
            Value::HashMap(m) => Some(m),
            _ => None,
        }
    }

    /// The engine's `isEqualTo`: same type and same content, strings
    /// compared case-sensitively, arrays element-wise, references for
    /// hash maps and handles.
    pub fn is_equal_to(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Nil, Value::Nil) | (Value::Nothing, Value::Nothing) => true,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Number(a), Value::Number(b)) => a == b,
            (Value::String(a), Value::String(b)) => a == b,
            (Value::Array(a), Value::Array(b)) => {
                if a.ptr_eq(b) {
                    return true;
                }
                let (a, b) = (a.borrow(), b.borrow());
                a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| x.is_equal_to(y))
            }
            (Value::HashMap(a), Value::HashMap(b)) => {
                if a.ptr_eq(b) {
                    return true;
                }
                let (a, b) = (a.borrow(), b.borrow());
                a.len() == b.len()
                    && a.iter()
                        .all(|(k, v)| b.get(k).is_some_and(|w| v.is_equal_to(w)))
            }
            (Value::Code(a), Value::Code(b)) => a.source() == b.source(),
            (Value::Namespace(a), Value::Namespace(b)) => a == b,
            (Value::Side(a), Value::Side(b)) => a == b,
            (Value::Script(a), Value::Script(b)) => a == b,
            (Value::Handle(a), Value::Handle(b)) => a == b,
            (Value::If(a), Value::If(b)) => a == b,
            (Value::With(a), Value::With(b)) => a == b,
            _ => false,
        }
    }

    /// Formats the value as the `str` command does (strings quoted).
    pub fn to_sqf_string(&self) -> String {
        self.to_sqf_string_with(&|h| h.to_string())
    }

    /// Like [`to_sqf_string`](Self::to_sqf_string), formatting host handles
    /// with `handle`.
    pub fn to_sqf_string_with(&self, handle: &dyn Fn(Handle) -> String) -> String {
        let mut out = String::new();
        self.write_sqf(&mut out, true, handle);
        out
    }

    /// Formats the value as `format "%1"` does: like `str` but a top-level
    /// string is not quoted.
    pub fn to_display_string(&self) -> String {
        match self {
            Value::String(s) => s.to_string(),
            _ => self.to_sqf_string(),
        }
    }

    /// Like [`to_display_string`](Self::to_display_string), formatting host
    /// handles with `handle`.
    pub fn to_display_string_with(&self, handle: &dyn Fn(Handle) -> String) -> String {
        match self {
            Value::String(s) => s.to_string(),
            _ => self.to_sqf_string_with(handle),
        }
    }

    fn write_sqf(&self, out: &mut String, quote: bool, handle: &dyn Fn(Handle) -> String) {
        match self {
            Value::Nil => out.push_str("any"),
            Value::Nothing => out.push_str("nothing"),
            Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Value::Number(n) => out.push_str(&format_number(*n)),
            Value::String(s) => {
                if quote {
                    out.push('"');
                    for c in s.chars() {
                        if c == '"' {
                            out.push('"');
                        }
                        out.push(c);
                    }
                    out.push('"');
                } else {
                    out.push_str(s);
                }
            }
            Value::Array(a) => {
                out.push('[');
                // Guard against self-referencing arrays.
                match a.0.try_borrow() {
                    Ok(items) => {
                        for (i, v) in items.iter().enumerate() {
                            if i > 0 {
                                out.push(',');
                            }
                            v.write_sqf(out, true, handle);
                        }
                    }
                    Err(_) => out.push_str("..."),
                }
                out.push(']');
            }
            Value::HashMap(m) => {
                out.push('[');
                match m.try_borrow() {
                    Ok(map) => {
                        for (i, (k, v)) in map.iter().enumerate() {
                            if i > 0 {
                                out.push(',');
                            }
                            out.push('[');
                            k.to_value().write_sqf(out, true, handle);
                            out.push(',');
                            v.write_sqf(out, true, handle);
                            out.push(']');
                        }
                    }
                    Err(_) => out.push_str("..."),
                }
                out.push(']');
            }
            Value::Code(c) => {
                out.push('{');
                out.push_str(c.source());
                out.push('}');
            }
            Value::Namespace(ns) => out.push_str(ns.name()),
            Value::Side(s) => out.push_str(s.name()),
            Value::Script(h) => {
                if h.0 == 0 {
                    out.push_str("<NULL-script>");
                } else {
                    out.push_str(&format!("<script {}>", h.0));
                }
            }
            Value::Handle(h) => out.push_str(&handle(*h)),
            Value::If(_) => out.push_str("if"),
            Value::While(_) => out.push_str("while"),
            Value::For(_) => out.push_str("for"),
            Value::Switch(_) => out.push_str("switch"),
            Value::With(_) => out.push_str("with"),
            Value::Exception(_) => out.push_str("try"),
        }
    }
}

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_sqf_string())
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_sqf_string())
    }
}

/// Structural equality for tests and host code: [`Value::is_equal_to`].
impl PartialEq for Value {
    fn eq(&self, other: &Value) -> bool {
        self.is_equal_to(other)
    }
}

impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Value::Bool(b)
    }
}
impl From<f32> for Value {
    fn from(n: f32) -> Self {
        Value::Number(n)
    }
}
impl From<i32> for Value {
    fn from(n: i32) -> Self {
        Value::Number(n as f32)
    }
}
impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::string(s)
    }
}
impl From<String> for Value {
    fn from(s: String) -> Self {
        Value::String(Rc::from(s))
    }
}
impl From<Vec<Value>> for Value {
    fn from(v: Vec<Value>) -> Self {
        Value::Array(Array::from_vec(v))
    }
}

/// Formats a number as the engine does: C `printf("%g")` of the float
/// widened to double, with the three-digit exponent of the MSVC runtime
/// (`str 1e6` is `"1e+006"`).
///
/// _(uncertain: the spelling of infinities and NaN.)_
pub fn format_number(n: f32) -> String {
    format_g(f64::from(n), 6)
}

fn format_g(v: f64, precision: usize) -> String {
    if v.is_nan() {
        return "-1.#IND".to_string();
    }
    if v.is_infinite() {
        return if v > 0.0 { "1.#INF" } else { "-1.#INF" }.to_string();
    }
    if v == 0.0 {
        return if v.is_sign_negative() { "-0" } else { "0" }.to_string();
    }
    let sci = format!("{:.*e}", precision - 1, v);
    let (mantissa, exp) = sci.split_once('e').expect("exponent format");
    let exp: i32 = exp.parse().expect("exponent digits");
    if exp < -4 || exp >= precision as i32 {
        let mantissa = strip_fraction_zeros(mantissa);
        let sign = if exp < 0 { '-' } else { '+' };
        format!("{mantissa}e{sign}{:03}", exp.unsigned_abs())
    } else {
        let decimals = (precision as i32 - 1 - exp).max(0) as usize;
        strip_fraction_zeros(&format!("{v:.decimals$}")).to_string()
    }
}

fn strip_fraction_zeros(s: &str) -> &str {
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.')
    } else {
        s
    }
}

/// A shared, mutable SQF array.
#[derive(Clone, Default)]
pub struct Array(Rc<RefCell<Vec<Value>>>);

impl Array {
    pub fn new() -> Array {
        Array::default()
    }

    pub fn from_vec(items: Vec<Value>) -> Array {
        Array(Rc::new(RefCell::new(items)))
    }

    pub fn borrow(&self) -> Ref<'_, Vec<Value>> {
        self.0.borrow()
    }

    pub fn borrow_mut(&self) -> RefMut<'_, Vec<Value>> {
        self.0.borrow_mut()
    }

    pub fn len(&self) -> usize {
        self.0.borrow().len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.borrow().is_empty()
    }

    /// Whether both references point at the same array.
    pub fn ptr_eq(&self, other: &Array) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }

    /// A shallow copy: a new array holding the same element values (nested
    /// arrays are still shared).
    pub fn shallow_copy(&self) -> Array {
        Array::from_vec(self.borrow().clone())
    }

    /// A deep copy, as the unary `+` command makes: nested arrays are copied
    /// too.
    pub fn deep_copy(&self) -> Array {
        Array::from_vec(self.borrow().iter().map(deep_copy_value).collect())
    }

    /// Whether `needle` is this array or is reachable from it through nested
    /// arrays. Used to refuse inserting an array into itself.
    pub fn contains_ref(&self, needle: &Array) -> bool {
        if self.ptr_eq(needle) {
            return true;
        }
        self.borrow()
            .iter()
            .any(|v| matches!(v, Value::Array(a) if a.contains_ref(needle)))
    }
}

/// Copies arrays recursively; other values are shared.
pub fn deep_copy_value(v: &Value) -> Value {
    match v {
        Value::Array(a) => Value::Array(a.deep_copy()),
        other => other.clone(),
    }
}

impl fmt::Debug for Array {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", Value::Array(self.clone()))
    }
}

/// A shared, mutable SQF hash map.
#[derive(Clone, Default)]
pub struct HashMap(Rc<HashMapInner>);

#[derive(Default)]
struct HashMapInner {
    map: RefCell<IndexMap<HashKey, Value>>,
    read_only: Cell<bool>,
}

impl HashMap {
    pub fn new() -> HashMap {
        HashMap::default()
    }

    pub fn from_map(map: IndexMap<HashKey, Value>) -> HashMap {
        HashMap(Rc::new(HashMapInner {
            map: RefCell::new(map),
            read_only: Cell::new(false),
        }))
    }

    pub fn borrow(&self) -> Ref<'_, IndexMap<HashKey, Value>> {
        self.0.map.borrow()
    }

    pub fn borrow_mut(&self) -> RefMut<'_, IndexMap<HashKey, Value>> {
        self.0.map.borrow_mut()
    }

    pub fn ptr_eq(&self, other: &HashMap) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }

    pub fn len(&self) -> usize {
        self.0.map.borrow().len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.map.borrow().is_empty()
    }

    /// Whether the map is read-only (`compileFinal` of a hash map).
    pub fn is_read_only(&self) -> bool {
        self.0.read_only.get()
    }

    pub fn set_read_only(&self) {
        self.0.read_only.set(true);
    }

    fn try_borrow(&self) -> Result<Ref<'_, IndexMap<HashKey, Value>>, std::cell::BorrowError> {
        self.0.map.try_borrow()
    }
}

impl fmt::Debug for HashMap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", Value::HashMap(self.clone()))
    }
}

/// A hash map key: the subset of values the engine accepts as keys
/// (numbers, strings, booleans, code, sides, namespaces, config entries and
/// arrays of those). Strings compare case-sensitively. An array key is an
/// immutable snapshot of the array.
#[derive(Clone, Debug)]
pub enum HashKey {
    Bool(bool),
    Number(f32),
    String(Rc<str>),
    Code(Code),
    Side(Side),
    Namespace(Namespace),
    Handle(Handle),
    Array(Rc<[HashKey]>),
}

impl HashKey {
    /// Converts a value to a key, or `None` for a type that cannot be a key.
    pub fn from_value(v: &Value) -> Option<HashKey> {
        Some(match v {
            Value::Bool(b) => HashKey::Bool(*b),
            Value::Number(n) => HashKey::Number(*n),
            Value::String(s) => HashKey::String(s.clone()),
            Value::Code(c) => HashKey::Code(c.clone()),
            Value::Side(s) => HashKey::Side(*s),
            Value::Namespace(n) => HashKey::Namespace(*n),
            Value::Handle(h) if h.kind == HandleKind::Config => HashKey::Handle(*h),
            Value::Array(a) => {
                let items = a.borrow();
                let mut keys = Vec::with_capacity(items.len());
                for item in items.iter() {
                    keys.push(HashKey::from_value(item)?);
                }
                HashKey::Array(keys.into())
            }
            _ => return None,
        })
    }

    /// The key as a value (array keys become new arrays).
    pub fn to_value(&self) -> Value {
        match self {
            HashKey::Bool(b) => Value::Bool(*b),
            HashKey::Number(n) => Value::Number(*n),
            HashKey::String(s) => Value::String(s.clone()),
            HashKey::Code(c) => Value::Code(c.clone()),
            HashKey::Side(s) => Value::Side(*s),
            HashKey::Namespace(n) => Value::Namespace(*n),
            HashKey::Handle(h) => Value::Handle(*h),
            HashKey::Array(items) => Value::array(items.iter().map(HashKey::to_value)),
        }
    }
}

impl PartialEq for HashKey {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (HashKey::Bool(a), HashKey::Bool(b)) => a == b,
            // -0 and 0 are the same key; bit patterns otherwise.
            (HashKey::Number(a), HashKey::Number(b)) => number_key_bits(*a) == number_key_bits(*b),
            (HashKey::String(a), HashKey::String(b)) => a == b,
            (HashKey::Code(a), HashKey::Code(b)) => a.source() == b.source(),
            (HashKey::Side(a), HashKey::Side(b)) => a == b,
            (HashKey::Namespace(a), HashKey::Namespace(b)) => a == b,
            (HashKey::Handle(a), HashKey::Handle(b)) => a == b,
            (HashKey::Array(a), HashKey::Array(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for HashKey {}

impl Hash for HashKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            HashKey::Bool(b) => b.hash(state),
            HashKey::Number(n) => number_key_bits(*n).hash(state),
            HashKey::String(s) => s.hash(state),
            HashKey::Code(c) => c.source().hash(state),
            HashKey::Side(s) => s.hash(state),
            HashKey::Namespace(n) => n.hash(state),
            HashKey::Handle(h) => h.hash(state),
            HashKey::Array(items) => items.hash(state),
        }
    }
}

fn number_key_bits(n: f32) -> u32 {
    if n == 0.0 { 0 } else { n.to_bits() }
}

/// A variable namespace.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Namespace {
    Mission,
    Ui,
    Profile,
    Parsing,
    Local,
    MissionProfile,
    Server,
}

impl Namespace {
    pub const ALL: [Namespace; 7] = [
        Namespace::Mission,
        Namespace::Ui,
        Namespace::Profile,
        Namespace::Parsing,
        Namespace::Local,
        Namespace::MissionProfile,
        Namespace::Server,
    ];

    /// The command that returns this namespace.
    pub fn name(self) -> &'static str {
        match self {
            Namespace::Mission => "missionNamespace",
            Namespace::Ui => "uiNamespace",
            Namespace::Profile => "profileNamespace",
            Namespace::Parsing => "parsingNamespace",
            Namespace::Local => "localNamespace",
            Namespace::MissionProfile => "missionProfileNamespace",
            Namespace::Server => "serverNamespace",
        }
    }
}

/// A side (faction alignment).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    West,
    East,
    Independent,
    Civilian,
    Unknown,
    Enemy,
    Friendly,
    Logic,
    Empty,
    AmbientLife,
}

impl Side {
    /// The string `str side` returns.
    pub fn name(self) -> &'static str {
        match self {
            Side::West => "WEST",
            Side::East => "EAST",
            Side::Independent => "GUER",
            Side::Civilian => "CIV",
            Side::Unknown => "UNKNOWN",
            Side::Enemy => "ENEMY",
            Side::Friendly => "FRIENDLY",
            Side::Logic => "LOGIC",
            Side::Empty => "EMPTY",
            Side::AmbientLife => "AMBIENT LIFE",
        }
    }
}

/// A script instance handle as returned by `spawn`/`execVM`; `0` is
/// `scriptNull`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct ScriptHandle(pub u32);

/// The kind of host-owned engine object a [`Handle`] refers to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HandleKind {
    Object,
    Group,
    Control,
    Display,
    Location,
    Task,
    TeamMember,
    DiaryRecord,
    Config,
    Text,
    NetObject,
    Target,
    SubGroup,
}

impl HandleKind {
    pub fn ty(self) -> Type {
        match self {
            HandleKind::Object => Type::Object,
            HandleKind::Group => Type::Group,
            HandleKind::Control => Type::Control,
            HandleKind::Display => Type::Display,
            HandleKind::Location => Type::Location,
            HandleKind::Task => Type::Task,
            HandleKind::TeamMember => Type::TeamMember,
            HandleKind::DiaryRecord => Type::DiaryRecord,
            HandleKind::Config => Type::Config,
            HandleKind::Text => Type::Text,
            HandleKind::NetObject => Type::NetObject,
            HandleKind::Target => Type::Target,
            HandleKind::SubGroup => Type::SubGroup,
        }
    }
}

/// An opaque reference to an engine object owned by the host (the world,
/// UI or config subsystem). The host interprets `id`; `id == 0` is the null
/// value of the kind (`objNull`, `grpNull`, `controlNull`, ...).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Handle {
    pub kind: HandleKind,
    pub id: u64,
}

impl Handle {
    pub const fn null(kind: HandleKind) -> Handle {
        Handle { kind, id: 0 }
    }

    pub const fn is_null(self) -> bool {
        self.id == 0
    }
}

impl fmt::Display for Handle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_null() {
            let name = match self.kind {
                HandleKind::Object => "<NULL-object>",
                HandleKind::Group => "<NULL-group>",
                HandleKind::Control => "No control",
                HandleKind::Display => "No display",
                HandleKind::Location => "No location",
                HandleKind::Task => "No task",
                HandleKind::TeamMember => "<NULL-team member>",
                HandleKind::DiaryRecord => "No diary record",
                HandleKind::Config => "",
                HandleKind::Text => "",
                HandleKind::NetObject => "<NULL-netobject>",
                HandleKind::Target => "<NULL-target>",
                HandleKind::SubGroup => "<NULL-subgroup>",
            };
            f.write_str(name)
        } else {
            write!(f, "<{:?} {}>", self.kind, self.id)
        }
    }
}

/// The state of a `for` loop under construction.
#[derive(Clone, Debug)]
pub enum ForSpec {
    /// `for "_i" from a to b step s`.
    Range {
        var: Sym,
        from: f32,
        to: f32,
        step: f32,
    },
    /// `for [{init}, {cond}, {step}]`.
    Code { init: Code, cond: Code, step: Code },
}

/// The state of a `switch`: the value being matched and, inside the switch
/// block, whether a `case` already matched and which code to run.
#[derive(Debug, Default)]
pub struct SwitchState {
    pub value: Value,
    pub matched: RefCell<bool>,
    pub selected: RefCell<Option<Code>>,
    pub default: RefCell<Option<Code>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_format_like_printf_g() {
        assert_eq!(format_number(7.0), "7");
        assert_eq!(format_number(0.1), "0.1");
        assert_eq!(format_number(-2.5), "-2.5");
        assert_eq!(format_number(123456.0), "123456");
        assert_eq!(format_number(1234567.0), "1.23457e+006");
        assert_eq!(format_number(1e6), "1e+006");
        assert_eq!(format_number(0.0001), "0.0001");
        assert_eq!(format_number(0.00001), "1e-005");
        assert_eq!(format_number(1.0 / 3.0), "0.333333");
    }

    #[test]
    fn numbers_are_single_precision() {
        let v = Value::Number(16_777_217.0);
        assert_eq!(v.as_number(), Some(16_777_216.0));
    }

    #[test]
    fn arrays_are_shared_by_reference() {
        let a = Array::from_vec(vec![Value::Number(1.0)]);
        let b = a.clone();
        b.borrow_mut().push(Value::Number(2.0));
        assert_eq!(a.len(), 2);
        let c = a.deep_copy();
        c.borrow_mut().push(Value::Number(3.0));
        assert_eq!(a.len(), 2);
    }

    #[test]
    fn str_quotes_and_doubles_quotes() {
        let v = Value::array([Value::from(1), Value::from("a\"b"), Value::array([])]);
        assert_eq!(v.to_sqf_string(), r#"[1,"a""b",[]]"#);
        assert_eq!(Value::from("x").to_display_string(), "x");
    }

    #[test]
    fn is_equal_to_is_case_sensitive_and_deep() {
        assert!(Value::from("a").is_equal_to(&Value::from("a")));
        assert!(!Value::from("a").is_equal_to(&Value::from("A")));
        let a = Value::array([Value::from(1), Value::array([Value::from(2)])]);
        let b = Value::array([Value::from(1), Value::array([Value::from(2)])]);
        assert!(a.is_equal_to(&b));
        assert!(!Value::from(1).is_equal_to(&Value::from("1")));
    }

    #[test]
    fn hash_keys_treat_negative_zero_as_zero() {
        let a = HashKey::from_value(&Value::Number(0.0)).unwrap();
        let b = HashKey::from_value(&Value::Number(-0.0)).unwrap();
        assert_eq!(a, b);
        assert!(HashKey::from_value(&Value::Nil).is_none());
    }
}
