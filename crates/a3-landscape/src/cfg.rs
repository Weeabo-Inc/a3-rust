//! Small typed accessors over [`ConfigRef`].

use a3_config::{ConfigRef, Value};
use glam::Vec3;

/// The number at `name`, or `None` when missing or not a number-like value.
pub(crate) fn number(c: &ConfigRef<'_>, name: &str) -> Option<f32> {
    let e = c.get(name);
    (!e.is_null() && !e.is_class() && !e.is_array()).then(|| e.number())
}

/// The number at `name`, or `default`.
pub(crate) fn number_or(c: &ConfigRef<'_>, name: &str, default: f32) -> f32 {
    number(c, name).unwrap_or(default)
}

/// The text at `name`, or `None` when missing.
pub(crate) fn text(c: &ConfigRef<'_>, name: &str) -> Option<String> {
    let e = c.get(name);
    (!e.is_null() && !e.is_class() && !e.is_array()).then(|| e.text())
}

/// The text at `name`, or empty.
pub(crate) fn text_or_empty(c: &ConfigRef<'_>, name: &str) -> String {
    text(c, name).unwrap_or_default()
}

pub(crate) fn value_number(v: &Value) -> Option<f32> {
    match v {
        Value::Float(f) => Some(*f),
        Value::Int(i) => Some(*i as f32),
        Value::Int64(i) => Some(*i as f32),
        Value::String(s) | Value::Expression(s) => s.trim().trim_end_matches('f').parse().ok(),
        Value::Array(_) => None,
    }
}

pub(crate) fn value_text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) | Value::Expression(s) => Some(s.clone()),
        _ => None,
    }
}

/// The numbers of the array at `name` (non-numbers skipped).
pub(crate) fn numbers(c: &ConfigRef<'_>, name: &str) -> Vec<f32> {
    c.get(name)
        .array()
        .iter()
        .filter_map(value_number)
        .collect()
}

/// The strings of the array at `name` (non-strings skipped).
pub(crate) fn texts(c: &ConfigRef<'_>, name: &str) -> Vec<String> {
    c.get(name).array().iter().filter_map(value_text).collect()
}

/// The first `N` numbers of the array at `name`, padded with `default`.
pub(crate) fn array_n<const N: usize>(c: &ConfigRef<'_>, name: &str, default: f32) -> [f32; N] {
    let v = numbers(c, name);
    std::array::from_fn(|i| v.get(i).copied().unwrap_or(default))
}

pub(crate) fn vec3(c: &ConfigRef<'_>, name: &str) -> Vec3 {
    Vec3::from_array(array_n::<3>(c, name, 0.0))
}

/// The child classes of `c`, in order (own and inherited).
pub(crate) fn classes<'a>(c: &ConfigRef<'a>) -> Vec<ConfigRef<'a>> {
    c.entries_with_inherited()
        .into_iter()
        .filter(|e| e.is_class())
        .collect()
}
