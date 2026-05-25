//! Tool exposure, instrumentation, and permission policy scaffolding.
//!
//! P8 will add Capo-owned tools and durable instrumentation records. P1 keeps
//! permission policy and tool exposure separate but in the same crate for now.

use capo_core::{BoundaryBinding, BoundaryKind, SessionId, ToolCallId};

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

    pub fn invoke(&self, request: FakeToolRequest) -> FakeToolResult {
        match self {
            Self::Fake(exposure) => exposure.invoke(request),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeToolExposure;

impl FakeToolExposure {
    pub fn binding(&self) -> BoundaryBinding {
        BoundaryBinding::fake(BoundaryKind::ToolExposure, "fake-tools")
    }

    pub fn invoke(&self, request: FakeToolRequest) -> FakeToolResult {
        FakeToolResult {
            tool_call_id: request.tool_call_id,
            tool_name: request.tool_name,
            output_artifact_id: format!("artifact-tool-{}", request.session_id),
            summary: format!("Tool observed session goal: {}", request.input_summary),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeToolRequest {
    pub tool_call_id: ToolCallId,
    pub session_id: SessionId,
    pub tool_name: String,
    pub input_summary: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeToolResult {
    pub tool_call_id: ToolCallId,
    pub tool_name: String,
    pub output_artifact_id: String,
    pub summary: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PermissionPolicy {
    Fake(FakePermissionPolicy),
    TrustedLocal(AllowTrustedLocalProfilePolicy),
}

impl PermissionPolicy {
    pub fn fake() -> Self {
        Self::Fake(FakePermissionPolicy)
    }

    pub fn allow_trusted_local() -> Self {
        Self::TrustedLocal(AllowTrustedLocalProfilePolicy)
    }

    pub fn binding(&self) -> BoundaryBinding {
        match self {
            Self::Fake(policy) => policy.binding(),
            Self::TrustedLocal(policy) => policy.binding(),
        }
    }

    pub fn decide(&self, request: PermissionRequest) -> PermissionDecision {
        match self {
            Self::Fake(policy) => policy.decide(request),
            Self::TrustedLocal(policy) => policy.decide(request),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakePermissionPolicy;

impl FakePermissionPolicy {
    pub fn binding(&self) -> BoundaryBinding {
        BoundaryBinding::fake(BoundaryKind::PermissionPolicy, "fake-permission")
    }

    pub fn decide(&self, request: PermissionRequest) -> PermissionDecision {
        PermissionDecision {
            capability_grant_id: format!("grant-{}", request.session_id),
            capability_profile_id: request.capability_profile_id,
            effect: "allow".to_string(),
            scope_json: request.scope_json,
            subject_json: format!("{{\"session_id\":\"{}\"}}", request.session_id),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AllowTrustedLocalProfilePolicy;

impl AllowTrustedLocalProfilePolicy {
    pub fn binding(&self) -> BoundaryBinding {
        BoundaryBinding {
            kind: BoundaryKind::PermissionPolicy,
            variant: "trusted-local",
            fake: false,
        }
    }

    pub fn decide(&self, request: PermissionRequest) -> PermissionDecision {
        PermissionDecision {
            capability_grant_id: format!("grant-{}", request.session_id),
            capability_profile_id: request.capability_profile_id,
            effect: "allow".to_string(),
            scope_json: request.scope_json,
            subject_json: format!("{{\"session_id\":\"{}\"}}", request.session_id),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PermissionRequest {
    pub session_id: SessionId,
    pub capability_profile_id: String,
    pub scope_json: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PermissionDecision {
    pub capability_grant_id: String,
    pub capability_profile_id: String,
    pub effect: String,
    pub scope_json: String,
    pub subject_json: String,
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

    #[test]
    fn trusted_local_policy_is_explicitly_not_fake() {
        let binding = PermissionPolicy::allow_trusted_local().binding();
        assert_eq!(binding.kind, BoundaryKind::PermissionPolicy);
        assert_eq!(binding.variant, "trusted-local");
        assert!(!binding.fake);
    }
}
