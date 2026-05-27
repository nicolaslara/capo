use capo_core::{BoundaryBinding, BoundaryKind, SessionId};
use serde_json::Value;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PermissionPolicy {
    Fake(FakePermissionPolicy),
    TrustedLocal(AllowTrustedLocalProfilePolicy),
    Static(StaticPolicy),
}

impl PermissionPolicy {
    pub fn fake() -> Self {
        Self::Fake(FakePermissionPolicy)
    }

    pub fn allow_trusted_local() -> Self {
        Self::TrustedLocal(AllowTrustedLocalProfilePolicy)
    }

    pub fn static_read_only_local() -> Self {
        Self::Static(StaticPolicy::read_only_local())
    }

    pub fn static_reviewer() -> Self {
        Self::Static(StaticPolicy::reviewer())
    }

    pub fn binding(&self) -> BoundaryBinding {
        match self {
            Self::Fake(policy) => policy.binding(),
            Self::TrustedLocal(policy) => policy.binding(),
            Self::Static(policy) => policy.binding(),
        }
    }

    pub fn decide(&self, request: PermissionRequest) -> PermissionDecision {
        match self {
            Self::Fake(policy) => policy.decide(request),
            Self::TrustedLocal(policy) => policy.decide(request),
            Self::Static(policy) => policy.decide(request),
        }
    }

    pub fn default_profile_id(&self) -> &'static str {
        match self {
            Self::Fake(_) => "fake",
            Self::TrustedLocal(_) => "trusted-local-dev",
            Self::Static(policy) => policy.profile_id(),
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
            capability_grant_id: scoped_grant_id(&request, "allow"),
            capability_profile_id: request.capability_profile_id,
            effect: "allow".to_string(),
            scope_json: request.scope_json,
            subject_json: format!("{{\"session_id\":\"{}\"}}", request.session_id),
            decision_source: "fake".to_string(),
            persistence: "once".to_string(),
            explanation: "fake policy allows all requests".to_string(),
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
            capability_grant_id: scoped_grant_id(&request, "allow"),
            capability_profile_id: request.capability_profile_id,
            effect: "allow".to_string(),
            scope_json: request.scope_json,
            subject_json: format!("{{\"session_id\":\"{}\"}}", request.session_id),
            decision_source: "allow_trusted_local_profile".to_string(),
            persistence: "until_session_end".to_string(),
            explanation: "trusted local profile allows audited local prototype request".to_string(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StaticPolicy {
    profile_id: String,
    allowed_scopes: Vec<String>,
}

impl StaticPolicy {
    pub fn read_only_local() -> Self {
        Self {
            profile_id: "read-only-local".to_string(),
            allowed_scopes: [
                "tool:invoke:capo.task_status",
                "tool:invoke:capo.agent_status",
                "tool:invoke:capo.session_summary",
                "tool:invoke:capo.project_memory_read",
                "tool:invoke:capo.workpad_read",
                "tool:invoke:capo.file_read",
                "tool:invoke:capo.git_status",
                "tool:invoke:capo.git_diff",
                "state:read:task",
                "state:read:agent",
                "state:read:session",
                "state:read:runtime",
                "state:read:provider",
                "state:read:tool",
                "state:read:evidence",
                "state:read:permission_queue",
                "filesystem:read:workspace",
                "git:status:workspace",
                "git:diff:workspace",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
        }
    }

    pub fn reviewer() -> Self {
        Self {
            profile_id: "reviewer".to_string(),
            allowed_scopes: [
                "tool:invoke:capo.task_status",
                "tool:invoke:capo.agent_status",
                "tool:invoke:capo.session_summary",
                "tool:invoke:capo.project_memory_read",
                "tool:invoke:capo.workpad_read",
                "tool:invoke:capo.file_read",
                "tool:invoke:capo.git_status",
                "tool:invoke:capo.git_diff",
                "state:read:task",
                "state:read:agent",
                "state:read:session",
                "state:read:runtime",
                "state:read:provider",
                "state:read:tool",
                "state:read:evidence",
                "state:read:permission_queue",
                "state:read:capability",
                "filesystem:read:workspace",
                "git:status:workspace",
                "git:diff:workspace",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
        }
    }

    pub fn binding(&self) -> BoundaryBinding {
        BoundaryBinding {
            kind: BoundaryKind::PermissionPolicy,
            variant: "static",
            fake: false,
        }
    }

    pub fn profile_id(&self) -> &'static str {
        match self.profile_id.as_str() {
            "read-only-local" => "read-only-local",
            "reviewer" => "reviewer",
            _ => "static",
        }
    }

    pub fn decide(&self, request: PermissionRequest) -> PermissionDecision {
        let requested_scopes = match scope_items(&request.scope_json) {
            Ok(scopes) => scopes,
            Err(explanation) => {
                return PermissionDecision {
                    capability_grant_id: scoped_grant_id(&request, "deny"),
                    capability_profile_id: request.capability_profile_id,
                    effect: "deny".to_string(),
                    scope_json: request.scope_json,
                    subject_json: format!("{{\"session_id\":\"{}\"}}", request.session_id),
                    decision_source: format!("static_policy:{}", self.profile_id),
                    persistence: "once".to_string(),
                    explanation,
                };
            }
        };
        let missing_scopes = requested_scopes
            .iter()
            .filter(|scope| !self.allowed_scopes.iter().any(|allowed| allowed == *scope))
            .cloned()
            .collect::<Vec<_>>();
        let allowed = !requested_scopes.is_empty() && missing_scopes.is_empty();
        PermissionDecision {
            capability_grant_id: scoped_grant_id(&request, if allowed { "allow" } else { "deny" }),
            capability_profile_id: request.capability_profile_id,
            effect: if allowed { "allow" } else { "deny" }.to_string(),
            scope_json: request.scope_json,
            subject_json: format!("{{\"session_id\":\"{}\"}}", request.session_id),
            decision_source: format!("static_policy:{}", self.profile_id),
            persistence: "once".to_string(),
            explanation: if allowed {
                format!(
                    "static profile `{}` allows all requested scopes",
                    self.profile_id
                )
            } else if requested_scopes.is_empty() {
                "static policy rejected request with no parseable scopes".to_string()
            } else {
                format!(
                    "static profile `{}` rejects missing scopes: {}",
                    self.profile_id,
                    missing_scopes.join(",")
                )
            },
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
    pub decision_source: String,
    pub persistence: String,
    pub explanation: String,
}

fn scope_items(scope_json: &str) -> Result<Vec<String>, String> {
    let value = serde_json::from_str::<Value>(scope_json)
        .map_err(|error| format!("static policy rejected malformed scope json: {error}"))?;
    let Value::Array(items) = value else {
        return Err("static policy rejected non-array scope json".to_string());
    };
    let mut scopes = Vec::with_capacity(items.len());
    for item in items {
        let Value::String(scope) = item else {
            return Err("static policy rejected non-string scope item".to_string());
        };
        scopes.push(scope);
    }
    Ok(scopes)
}

fn scoped_grant_id(request: &PermissionRequest, effect: &str) -> String {
    format!(
        "grant-{}-{}-{}",
        request.session_id,
        effect,
        stable_hash(&format!(
            "{}:{}:{}",
            request.capability_profile_id, request.scope_json, effect
        ))
    )
}

fn stable_hash(value: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}
