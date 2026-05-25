//! State store and projection scaffolding.
//!
//! P2 will add SQLite-backed events, projections, artifact metadata, and
//! restart recovery records. P1 only exposes a fake store boundary.

use capo_core::{BoundaryBinding, BoundaryKind};

/// Name of the first durable local state backend.
pub const PROTOTYPE_STATE_BACKEND: &str = "sqlite";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StateStore {
    Fake(FakeStateStore),
}

impl StateStore {
    pub fn fake() -> Self {
        Self::Fake(FakeStateStore)
    }

    pub fn binding(&self) -> BoundaryBinding {
        match self {
            Self::Fake(store) => store.binding(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeStateStore;

impl FakeStateStore {
    pub fn binding(&self) -> BoundaryBinding {
        BoundaryBinding::fake(BoundaryKind::StateStore, "fake-state")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prototype_state_backend_is_sqlite() {
        assert_eq!(PROTOTYPE_STATE_BACKEND, "sqlite");
    }

    #[test]
    fn fake_store_reports_state_boundary() {
        assert_eq!(StateStore::fake().binding().kind, BoundaryKind::StateStore);
    }
}
