//! Animation source values.

use std::collections::HashMap;

/// Values of animation sources by name. Names compare ignoring ASCII case; a source that was
/// never set reads as `0.0`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Sources {
    values: HashMap<String, f32>,
}

impl Sources {
    /// No sources set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets `name` to `value`.
    pub fn set(&mut self, name: &str, value: f32) {
        self.values.insert(name.to_ascii_lowercase(), value);
    }

    /// Builder form of [`Sources::set`].
    pub fn with(mut self, name: &str, value: f32) -> Self {
        self.set(name, value);
        self
    }

    /// The value of `name`, `0.0` when unset.
    pub fn get(&self, name: &str) -> f32 {
        self.values
            .get(&name.to_ascii_lowercase())
            .copied()
            .unwrap_or(0.0)
    }
}
