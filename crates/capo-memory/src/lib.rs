//! Memory packet scaffolding.
//!
//! P9 will add source-linked memory packet artifacts and read-model inspection.

use capo_core::{BoundaryBinding, BoundaryKind};

/// The first memory backend proves packet provenance without semantic search.
pub const PROTOTYPE_MEMORY_BACKEND: &str = "fake-packet-builder";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MemoryBackend {
    Fake(FakeMemoryBackend),
}

impl MemoryBackend {
    pub fn fake() -> Self {
        Self::Fake(FakeMemoryBackend)
    }

    pub fn binding(&self) -> BoundaryBinding {
        match self {
            Self::Fake(backend) => backend.binding(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeMemoryBackend;

impl FakeMemoryBackend {
    pub fn binding(&self) -> BoundaryBinding {
        BoundaryBinding::fake(BoundaryKind::MemoryBackend, "fake-memory")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prototype_memory_backend_is_fake_packet_builder() {
        assert_eq!(PROTOTYPE_MEMORY_BACKEND, "fake-packet-builder");
    }

    #[test]
    fn fake_memory_reports_memory_boundary() {
        assert_eq!(
            MemoryBackend::fake().binding().kind,
            BoundaryKind::MemoryBackend
        );
    }
}
