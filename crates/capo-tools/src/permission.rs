use capo_core::{BoundaryBinding, BoundaryKind, SessionId};
use serde_json::Value;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PermissionPolicy {
    Fake(FakePermissionPolicy),
    TrustedLocal(AllowTrustedLocalProfilePolicy),
    Static(StaticPolicy),
    /// SG3: a durable-grant authority. The controller's decide-step grant
    /// read-back consults the durable grant store BEFORE the configured policy; a
    /// hit (a valid subject+scope allow grant, or a standing `reject_always` deny
    /// grant) becomes this one-shot policy so the SAME `authorize_and_invoke`
    /// dispatch path enforces the durable verdict -- one decide path, not a
    /// parallel API. It is never a configured controller default; it is minted per
    /// dispatch from a read-back hit.
    DurableGrant(DurableGrantPolicy),
}

impl PermissionPolicy {
    pub fn fake() -> Self {
        Self::Fake(FakePermissionPolicy)
    }

    pub fn allow_trusted_local() -> Self {
        Self::TrustedLocal(AllowTrustedLocalProfilePolicy::new())
    }

    /// SG4: TrustedLocal with an explicit set of granted critical scopes.
    ///
    /// `allow_trusted_local()` denies every critical scope (source-write outside
    /// the workspace, network egress, secret/credential read/write, raw voice
    /// transcript read, external memory sync/export, remote browser control,
    /// arbitrary shell); this constructor re-admits ONLY the critical scopes named
    /// in `granted`, so a reviewed profile can opt back into a specific critical
    /// capability without reopening the blanket-allow hole. Non-critical
    /// TrustedLocal behavior is unchanged either way.
    ///
    /// NOTE (SG4 scope): this profile-level grant set is test-only scaffolding for
    /// the policy unit tests. The PRODUCTION re-admission surface in the running
    /// loop is the SG3 durable-grant read-back (`decide_with_grant_read_back` /
    /// the dispatch gate's `read_back_effective_policy`): a reviewed durable allow
    /// grant re-admits a critical scope the configured TrustedLocal policy denies.
    /// That production path is what the controller-level SG4 test exercises with a
    /// critical scope (`sg4_default_trusted_local_controller_denies_each_critical_
    /// scope_until_durable_grant`); no production caller builds this constructor.
    pub fn allow_trusted_local_with_grants(granted: impl IntoIterator<Item = String>) -> Self {
        Self::TrustedLocal(AllowTrustedLocalProfilePolicy::with_granted_critical_scopes(granted))
    }

    pub fn static_read_only_local() -> Self {
        Self::Static(StaticPolicy::read_only_local())
    }

    pub fn static_reviewer() -> Self {
        Self::Static(StaticPolicy::reviewer())
    }

    /// SG3: a one-shot policy that returns the supplied durable-grant verdict for
    /// this dispatch. Used by the controller's decide step to enforce a grant
    /// read-back hit through the same `authorize_and_invoke` gate the policy path
    /// uses, so a valid durable allow grant authorizes a call the configured
    /// policy would deny, and a standing deny grant blocks a call it would allow.
    pub fn durable_grant(decision: PermissionDecision) -> Self {
        Self::DurableGrant(DurableGrantPolicy { decision })
    }

    pub fn binding(&self) -> BoundaryBinding {
        match self {
            Self::Fake(policy) => policy.binding(),
            Self::TrustedLocal(policy) => policy.binding(),
            Self::Static(policy) => policy.binding(),
            Self::DurableGrant(policy) => policy.binding(),
        }
    }

    pub fn decide(&self, request: PermissionRequest) -> PermissionDecision {
        match self {
            Self::Fake(policy) => policy.decide(request),
            Self::TrustedLocal(policy) => policy.decide(request),
            Self::Static(policy) => policy.decide(request),
            Self::DurableGrant(policy) => policy.decide(request),
        }
    }

    pub fn default_profile_id(&self) -> &'static str {
        match self {
            Self::Fake(_) => "fake",
            Self::TrustedLocal(_) => "trusted-local-dev",
            Self::Static(policy) => policy.profile_id(),
            Self::DurableGrant(_) => "durable-grant",
        }
    }
}

/// SG3: a one-shot policy carrying a precomputed durable-grant verdict.
///
/// Read-back is the authority here, not a profile: the controller mints this from
/// a durable grant-store hit and hands it to `authorize_and_invoke` for a single
/// dispatch so the durable verdict (allow or `reject_always` deny) is enforced on
/// the SAME gate the configured policy uses. `decide` returns the precomputed
/// decision verbatim, re-stamped onto the live request's scope so it lines up with
/// the dispatch's own `permission.requested`/`decided` audit events.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DurableGrantPolicy {
    decision: PermissionDecision,
}

impl DurableGrantPolicy {
    pub fn binding(&self) -> BoundaryBinding {
        BoundaryBinding {
            kind: BoundaryKind::PermissionPolicy,
            variant: "durable-grant",
            fake: false,
        }
    }

    pub fn decide(&self, request: PermissionRequest) -> PermissionDecision {
        PermissionDecision {
            capability_grant_id: self.decision.capability_grant_id.clone(),
            capability_profile_id: request.capability_profile_id,
            effect: self.decision.effect.clone(),
            scope_json: request.scope_json,
            subject_json: format!("{{\"session_id\":\"{}\"}}", request.session_id),
            decision_source: self.decision.decision_source.clone(),
            persistence: self.decision.persistence.clone(),
            explanation: self.decision.explanation.clone(),
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

/// SG4: the critical-scope categories TrustedLocal must NOT blanket-allow.
///
/// `capability-permissions.md` (Design Rules / `trusted-local-dev` v0 profile)
/// excludes these from the trusted-local audit-only allow unless the selected
/// profile explicitly includes them. They are the scopes that escape the local
/// workspace sandbox or touch credential material:
///
/// - source-write outside the workspace (`filesystem:write:path`),
/// - network egress / remote exposure (`network:connect:internet`,
///   `network:expose:public`, `network:connect:private_tunnel`),
/// - secret/credential read or write (`secret:read:credential_material`,
///   `secret:write:credential_material`),
/// - raw voice transcript read (`voice:read:raw_transcript`),
/// - external memory sync/export (`memory:export:project`,
///   `memory:sync:external`),
/// - browser automation against a remote page with persisted session state
///   (`browser:control:remote_page`),
/// - arbitrary shell outside the workspace (`shell:execute:path`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CriticalScope {
    /// `filesystem:write:path` -- a source write addressed by path, not confined
    /// to the workspace root.
    SourceWriteOutsideWorkspace,
    /// `network:connect:internet`, `network:expose:public`, or
    /// `network:connect:private_tunnel` -- network egress / public or private
    /// tunnel exposure (the runtime's own remote/egress exposure scope set, see
    /// `ExposureScope::permission_scope`).
    NetworkEgress,
    /// `secret:read:credential_material` -- reading raw credential material.
    SecretRead,
    /// `secret:write:credential_material` -- writing/persisting raw credential
    /// material (the doc marks credential material critical for read AND write).
    SecretWrite,
    /// `voice:read:raw_transcript` -- reading a raw (non-summarized) voice
    /// transcript.
    RawVoiceTranscriptRead,
    /// `memory:export:project` or `memory:sync:external` -- external memory
    /// sync/export off the local machine.
    ExternalMemorySync,
    /// `browser:control:remote_page` -- browser automation against a remote page,
    /// which can drive persisted authenticated session state.
    RemoteBrowserControl,
    /// `shell:execute:path` -- arbitrary shell at a path outside the workspace.
    ArbitraryShell,
}

impl CriticalScope {
    fn label(self) -> &'static str {
        match self {
            Self::SourceWriteOutsideWorkspace => "source-write outside workspace",
            Self::NetworkEgress => "network egress",
            Self::SecretRead => "secret/credential read",
            Self::SecretWrite => "secret/credential write",
            Self::RawVoiceTranscriptRead => "raw voice transcript read",
            Self::ExternalMemorySync => "external memory sync/export",
            Self::RemoteBrowserControl => "remote browser control",
            Self::ArbitraryShell => "arbitrary shell",
        }
    }
}

/// SG4: classify a scope string as critical (and which category) or non-critical.
///
/// Returns `Some(_)` for exactly the enumerated critical scopes and `None` for
/// every ordinary workspace scope (workspace read/write, `git:status`/`git:diff`,
/// Capo tool invocation, etc.), so non-critical TrustedLocal behavior is
/// untouched. Matching is on the full `{domain}:{action}:{resource}` scope string
/// used for matching/display.
///
/// The set mirrors the `trusted-local-dev` v0 exclusion list in
/// `capability-permissions.md` ("Excludes credential-material read/write, ...
/// raw voice transcript read, public tunnel exposure, remote runtime execution,
/// external memory sync/export, browser automation with persisted session
/// state"). Private-tunnel exposure is included because the runtime emits it as a
/// remote/egress scope (`ExposureScope::Private -> network:connect:private_tunnel`,
/// `requires_permission() == true`).
pub fn critical_scope_kind(scope: &str) -> Option<CriticalScope> {
    match scope {
        "filesystem:write:path" => Some(CriticalScope::SourceWriteOutsideWorkspace),
        "network:connect:internet" | "network:expose:public" | "network:connect:private_tunnel" => {
            Some(CriticalScope::NetworkEgress)
        }
        "secret:read:credential_material" => Some(CriticalScope::SecretRead),
        "secret:write:credential_material" => Some(CriticalScope::SecretWrite),
        "voice:read:raw_transcript" => Some(CriticalScope::RawVoiceTranscriptRead),
        "memory:export:project" | "memory:sync:external" => Some(CriticalScope::ExternalMemorySync),
        "browser:control:remote_page" => Some(CriticalScope::RemoteBrowserControl),
        "shell:execute:path" => Some(CriticalScope::ArbitraryShell),
        _ => None,
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct AllowTrustedLocalProfilePolicy {
    /// SG4: critical scopes the selected profile EXPLICITLY grants. Empty by
    /// default (the blanket-allow hole is closed): a critical scope not listed
    /// here is denied even under TrustedLocal. A critical scope listed here is
    /// re-admitted (an explicit grant is present), so the same request allows.
    granted_critical_scopes: Vec<String>,
}

impl AllowTrustedLocalProfilePolicy {
    pub fn new() -> Self {
        Self::default()
    }

    /// SG4: TrustedLocal that explicitly grants the named critical scopes.
    pub fn with_granted_critical_scopes(granted: impl IntoIterator<Item = String>) -> Self {
        Self {
            granted_critical_scopes: granted.into_iter().collect(),
        }
    }

    pub fn binding(&self) -> BoundaryBinding {
        BoundaryBinding {
            kind: BoundaryKind::PermissionPolicy,
            variant: "trusted-local",
            fake: false,
        }
    }

    /// SG4: a critical scope is admitted only when the profile explicitly grants
    /// it.
    fn critical_scope_is_granted(&self, scope: &str) -> bool {
        self.granted_critical_scopes
            .iter()
            .any(|granted| granted == scope)
    }

    pub fn decide(&self, request: PermissionRequest) -> PermissionDecision {
        // SG4: enumerate the requested critical scopes that lack an explicit
        // grant. TrustedLocal is audit-only-allow for ordinary local work, but it
        // is NO LONGER blanket-allow on critical scopes: an un-granted
        // source-write outside the workspace, network egress, secret read, or
        // arbitrary shell request is DENIED. Malformed scope json fails closed.
        let ungranted_critical: Vec<(String, CriticalScope)> =
            match scope_items(&request.scope_json) {
                Ok(scopes) => scopes
                    .into_iter()
                    .filter_map(|scope| {
                        critical_scope_kind(&scope).and_then(|kind| {
                            (!self.critical_scope_is_granted(&scope)).then_some((scope, kind))
                        })
                    })
                    .collect(),
                Err(explanation) => {
                    return PermissionDecision {
                        capability_grant_id: scoped_grant_id(&request, "deny"),
                        capability_profile_id: request.capability_profile_id,
                        effect: "deny".to_string(),
                        scope_json: request.scope_json,
                        subject_json: format!("{{\"session_id\":\"{}\"}}", request.session_id),
                        decision_source: "allow_trusted_local_profile".to_string(),
                        persistence: "once".to_string(),
                        explanation,
                    };
                }
            };

        if !ungranted_critical.is_empty() {
            let denied = ungranted_critical
                .iter()
                .map(|(scope, kind)| format!("{scope} ({})", kind.label()))
                .collect::<Vec<_>>()
                .join(", ");
            return PermissionDecision {
                capability_grant_id: scoped_grant_id(&request, "deny"),
                capability_profile_id: request.capability_profile_id,
                effect: "deny".to_string(),
                scope_json: request.scope_json,
                subject_json: format!("{{\"session_id\":\"{}\"}}", request.session_id),
                decision_source: "allow_trusted_local_profile".to_string(),
                persistence: "once".to_string(),
                explanation: format!(
                    "trusted local profile denies critical scope without an explicit grant: {denied}"
                ),
            };
        }

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
