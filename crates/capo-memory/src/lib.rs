//! Memory packet scaffolding.
//!
//! P9 will add source-linked memory packet artifacts and read-model inspection.

/// The first memory backend proves packet provenance without semantic search.
pub const PROTOTYPE_MEMORY_BACKEND: &str = "fake-packet-builder";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prototype_memory_backend_is_fake_packet_builder() {
        assert_eq!(PROTOTYPE_MEMORY_BACKEND, "fake-packet-builder");
    }
}
