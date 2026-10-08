//! Shared engine primitives used across the a3-rust workspace.
//!
//! This crate holds small, dependency-light types that every other crate may
//! need (versions, identifiers, common math aliases as they appear).

/// Version of the original Arma 3 build this reimplementation targets.
pub const GAME_VERSION: &str = "2.22.0.154103";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn game_version_has_four_numeric_parts() {
        let parts: Vec<_> = GAME_VERSION.split('.').collect();
        assert_eq!(parts.len(), 4);
        assert!(parts.iter().all(|p| p.parse::<u32>().is_ok()));
    }
}
