//! State store and projection scaffolding.
//!
//! P2 will add SQLite-backed events, projections, artifact metadata, and
//! restart recovery records.

/// Name of the first durable local state backend.
pub const PROTOTYPE_STATE_BACKEND: &str = "sqlite";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prototype_state_backend_is_sqlite() {
        assert_eq!(PROTOTYPE_STATE_BACKEND, "sqlite");
    }
}
