//! Core domain vocabulary for Capo.
//!
//! This crate starts intentionally small. P1 will add typed IDs, command
//! envelopes, lifecycle records, and static dispatch boundary enums.

/// Product name used by CLI/help surfaces.
pub const PRODUCT_NAME: &str = "Capo";

/// Boundary crates that make up the first prototype scaffold.
pub const BOUNDARY_CRATES: &[&str] = &[
    "capo-core",
    "capo-state",
    "capo-adapters",
    "capo-runtime",
    "capo-tools",
    "capo-memory",
    "capo-eval",
];

/// Returns a stable, human-readable scaffold summary.
pub fn scaffold_summary() -> String {
    format!(
        "{PRODUCT_NAME} prototype scaffold: {} boundary crates",
        BOUNDARY_CRATES.len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaffold_summary_names_product() {
        assert!(scaffold_summary().contains(PRODUCT_NAME));
    }

    #[test]
    fn boundary_crates_include_core_boundaries() {
        assert!(BOUNDARY_CRATES.contains(&"capo-state"));
        assert!(BOUNDARY_CRATES.contains(&"capo-adapters"));
        assert!(BOUNDARY_CRATES.contains(&"capo-runtime"));
        assert!(BOUNDARY_CRATES.contains(&"capo-tools"));
        assert!(BOUNDARY_CRATES.contains(&"capo-memory"));
    }
}
