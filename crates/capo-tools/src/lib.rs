//! Tool exposure, instrumentation, and permission policy scaffolding.
//!
//! P8 will add Capo-owned tools and durable instrumentation records. P1 keeps
//! permission policy and tool exposure separate but in the same crate for now.

use capo_core::{BoundaryBinding, BoundaryKind};

/// First Capo-owned tools selected by the architecture.
pub const CAPO_OWNED_TOOLS: &[&str] = &[
    "capo.task_status",
    "capo.agent_status",
    "capo.session_summary",
    "capo.workpad_read",
    "capo.evidence_record",
    "capo.capability_request",
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ToolExposure {
    Fake(FakeToolExposure),
}

impl ToolExposure {
    pub fn fake() -> Self {
        Self::Fake(FakeToolExposure)
    }

    pub fn binding(&self) -> BoundaryBinding {
        match self {
            Self::Fake(exposure) => exposure.binding(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeToolExposure;

impl FakeToolExposure {
    pub fn binding(&self) -> BoundaryBinding {
        BoundaryBinding::fake(BoundaryKind::ToolExposure, "fake-tools")
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PermissionPolicy {
    Fake(FakePermissionPolicy),
}

impl PermissionPolicy {
    pub fn fake() -> Self {
        Self::Fake(FakePermissionPolicy)
    }

    pub fn binding(&self) -> BoundaryBinding {
        match self {
            Self::Fake(policy) => policy.binding(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakePermissionPolicy;

impl FakePermissionPolicy {
    pub fn binding(&self) -> BoundaryBinding {
        BoundaryBinding::fake(BoundaryKind::PermissionPolicy, "fake-permission")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_tool_set_supports_status_and_evidence() {
        assert!(CAPO_OWNED_TOOLS.contains(&"capo.task_status"));
        assert!(CAPO_OWNED_TOOLS.contains(&"capo.evidence_record"));
    }

    #[test]
    fn fake_tool_and_permission_are_separate_boundaries() {
        assert_eq!(
            ToolExposure::fake().binding().kind,
            BoundaryKind::ToolExposure
        );
        assert_eq!(
            PermissionPolicy::fake().binding().kind,
            BoundaryKind::PermissionPolicy
        );
    }
}
