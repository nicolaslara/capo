//! Memory packet scaffolding.
//!
//! P9 will add source-linked memory packet artifacts and read-model inspection.

use capo_core::{BoundaryBinding, BoundaryKind, MemoryPacketId, SessionId};

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

    pub fn build_packet(&self, request: FakeMemoryPacketRequest) -> FakeMemoryPacket {
        match self {
            Self::Fake(backend) => backend.build_packet(request),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeMemoryBackend;

impl FakeMemoryBackend {
    pub fn binding(&self) -> BoundaryBinding {
        BoundaryBinding::fake(BoundaryKind::MemoryBackend, "fake-memory")
    }

    pub fn build_packet(&self, request: FakeMemoryPacketRequest) -> FakeMemoryPacket {
        FakeMemoryPacket {
            memory_packet_id: request.memory_packet_id,
            session_id: request.session_id,
            artifact_id: format!("artifact-memory-{}", request.goal_slug),
            purpose: "turn_context".to_string(),
            source_summary: request.summary,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeMemoryPacketRequest {
    pub memory_packet_id: MemoryPacketId,
    pub session_id: SessionId,
    pub goal_slug: String,
    pub summary: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeMemoryPacket {
    pub memory_packet_id: MemoryPacketId,
    pub session_id: SessionId,
    pub artifact_id: String,
    pub purpose: String,
    pub source_summary: String,
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
