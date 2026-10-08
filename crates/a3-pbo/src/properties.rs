use a3_core::VfsPath;

/// The key/value header properties of a PBO, in file order.
///
/// Shipped PBOs use `prefix`, `product` and `version`; third-party tools add their own keys
/// (for example `Mikero`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Properties(Vec<(String, String)>);

impl Properties {
    /// The value of the first property named `key`, compared case-insensitively.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_str())
    }

    /// The `prefix` property, normalised; `None` when absent or empty.
    pub fn prefix(&self) -> Option<VfsPath> {
        self.get("prefix")
            .map(VfsPath::new)
            .filter(|prefix| !prefix.is_root())
    }

    /// Iterates the properties as `(key, value)` pairs in file order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// Number of properties.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns `true` when there are no properties.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Appends a property.
    pub fn push(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.0.push((key.into(), value.into()));
    }
}
