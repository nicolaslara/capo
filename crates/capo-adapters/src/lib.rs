//! Agent adapter and provider connector scaffolding.
//!
//! Concrete Codex, Claude Code, and ACP implementations are deferred. P1 only
//! installs the static dispatch shape and fake variants used by controller
//! wiring tests.

use capo_core::{BoundaryBinding, BoundaryKind};

/// Initial adapter variants named by the architecture.
pub const PLANNED_ADAPTERS: &[&str] = &["fake", "codex-exec", "claude-code", "acp"];

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AgentAdapter {
    Fake(FakeAdapter),
}

impl AgentAdapter {
    pub fn fake() -> Self {
        Self::Fake(FakeAdapter)
    }

    pub fn binding(&self) -> BoundaryBinding {
        match self {
            Self::Fake(adapter) => adapter.binding(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeAdapter;

impl FakeAdapter {
    pub fn binding(&self) -> BoundaryBinding {
        BoundaryBinding::fake(BoundaryKind::AgentAdapter, "fake-adapter")
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProviderConnector {
    Fake(FakeProviderConnector),
}

impl ProviderConnector {
    pub fn fake() -> Self {
        Self::Fake(FakeProviderConnector)
    }

    pub fn binding(&self) -> BoundaryBinding {
        match self {
            Self::Fake(connector) => connector.binding(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeProviderConnector;

impl FakeProviderConnector {
    pub fn binding(&self) -> BoundaryBinding {
        BoundaryBinding::fake(BoundaryKind::ProviderConnector, "fake-provider")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planned_adapters_include_fake_and_first_real_targets() {
        assert!(PLANNED_ADAPTERS.contains(&"fake"));
        assert!(PLANNED_ADAPTERS.contains(&"codex-exec"));
        assert!(PLANNED_ADAPTERS.contains(&"claude-code"));
    }

    #[test]
    fn fake_adapter_reports_adapter_boundary() {
        assert_eq!(
            AgentAdapter::fake().binding().kind,
            BoundaryKind::AgentAdapter
        );
    }

    #[test]
    fn fake_provider_reports_provider_boundary() {
        assert_eq!(
            ProviderConnector::fake().binding().kind,
            BoundaryKind::ProviderConnector
        );
    }
}
