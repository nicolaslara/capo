//! SG2: the `AgentAdapter` permission round-trip + ACP option mapping, against
//! fake/scripted adapters.
//!
//! A fake/scripted adapter raises an [`AdapterPermissionRequest`] (the requested
//! scope + the ACP `PermissionOption[]` it offers). The controller DECIDES it and
//! returns the chosen outcome to the adapter as an [`AdapterPermissionResponse`]
//! -- the ACP outcome plus the selected `optionId` -- using the provider-neutral
//! adapter types (NOT `Fake*`-named structs). The lifecycle steps from
//! `capability-permissions.md` are persisted: `permission.requested` ->
//! evaluate -> `permission.decided` (with the ACP option list + chosen option ID
//! recorded as `adapter_options`/`adapter_response`) -> on an allow with
//! non-observational persistence, `capability.grant_created`.
//!
//! The ACP option mapping is the design's table (`capability-permissions.md`
//! lines 383-397), implemented in [`capo_adapters::map_acp_options_trusted_local`]:
//! `allow_once` -> allow once/turn-scoped; `allow_always` -> allow downscoped to
//! `until_session_end` under TrustedLocal; `reject_once`/`reject_always` ->
//! reject with the correct returned `optionId`; cancellation -> `cancelled`
//! outcome plus a `permission.decided` with `decision = cancel`. When no
//! selectable option exists, that is an adapter error: record `cancel` and fail
//! the adapter request rather than inventing an ACP outcome.
//!
//! This is fixture/option-mapping ONLY. The live ACP JSON-RPC wire round-trip is
//! explicitly out of scope and lands in the depth workpad.

use capo_adapters::{
    AcpOptionMapping, AcpPermissionOutcome, AdapterPermissionCancelReason,
    AdapterPermissionRequest, AdapterPermissionResponse, map_acp_options_trusted_local,
};
use capo_core::SessionId;
use capo_tools::{PermissionPolicy, PermissionRequest};

use super::*;

/// Where one adapter permission round-trip hangs on the loop's scope tree, so the
/// persisted `permission.*`/`capability.*` events carry the same
/// task/agent/session/run/turn provenance the rest of the loop uses.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PermissionRoundTripScope {
    pub task_id: TaskId,
    pub agent_id: AgentId,
    pub session_id: SessionId,
    pub run_id: RunId,
    pub turn_id: TurnId,
    /// A stable id for THIS round-trip, used to key the persisted events.
    pub request_ref: String,
}

/// Whether a permission round-trip was canceled by the operator or by an adapter
/// error (no selectable option), to drive the controller's cancel handling.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PermissionCancellation {
    /// The prompt turn / permission request was actually canceled.
    OperatorCancelled,
}

impl FakeBoundaryController {
    /// SG2: decide one adapter permission round-trip and return the ACP outcome.
    ///
    /// Runs `PermissionPolicy::decide` over the requested scope, maps the offered
    /// ACP options onto a Capo decision/persistence via the
    /// `capability-permissions.md` table, persists the lifecycle
    /// (`permission.requested` -> `permission.decided` -> on allow
    /// `capability.grant_created`), and returns the chosen `optionId` (or a
    /// `cancelled` outcome) to the adapter.
    ///
    /// The FINAL effect is the conjunction of policy AND option: an allow option
    /// only allows when the policy also permits the scope; a policy deny is
    /// reflected even if the adapter offered an allow option (the adapter cannot
    /// over-rule the policy). A reject option records a Capo reject regardless.
    pub fn decide_adapter_permission(
        &self,
        scope: &PermissionRoundTripScope,
        request: &AdapterPermissionRequest,
    ) -> StateResult<AdapterPermissionResponse> {
        // Step 1: map the offered ACP options onto a Capo decision/persistence.
        // An empty option list (or all-unknown kinds) yields the
        // no-selectable-option adapter-error mapping (cancel).
        let mapping = map_acp_options_trusted_local(&request.options);
        self.record_round_trip(scope, request, mapping)
    }

    /// SG2: cancel a pending adapter permission round-trip.
    ///
    /// Records `permission.requested` -> `permission.decided` with
    /// `decision = cancel` and returns the ACP `cancelled` outcome. This is the
    /// operator-cancel path (`capability-permissions.md` cancellation section),
    /// distinct from the no-selectable-option adapter error.
    pub fn cancel_adapter_permission(
        &self,
        scope: &PermissionRoundTripScope,
        request: &AdapterPermissionRequest,
        _cancellation: PermissionCancellation,
    ) -> StateResult<AdapterPermissionResponse> {
        self.record_round_trip(scope, request, AcpOptionMapping::cancelled())
    }

    /// Persist the round-trip lifecycle and build the adapter response.
    ///
    /// The mapping's `cancel_reason` distinguishes the operator-cancel path (no
    /// adapter error) from the no-selectable-option adapter error; both record
    /// `cancel` but the response's `adapter_error` flag and the persisted reason
    /// differ.
    fn record_round_trip(
        &self,
        scope: &PermissionRoundTripScope,
        request: &AdapterPermissionRequest,
        mapping: AcpOptionMapping,
    ) -> StateResult<AdapterPermissionResponse> {
        // Build the policy request from the adapter's requested scope. The scope
        // string is wrapped in the array shape the StaticPolicy parses.
        let scope_json = serde_json::json!([request.scope]).to_string();
        let policy_request = PermissionRequest {
            session_id: scope.session_id.clone(),
            capability_profile_id: request.capability_profile_id.clone(),
            scope_json: scope_json.clone(),
        };
        let policy_decision = self.permission_policy.decide(policy_request);

        // Step 2: combine policy + option into the FINAL Capo decision. The
        // policy is the authority on whether the scope is permitted; the ACP
        // option chooses persistence/grant behavior. A policy deny overrides an
        // allow option; a cancel mapping stays a cancel.
        let resolved = resolve_decision(&self.permission_policy, &policy_decision, &mapping);

        let permission_decision_id = format!("decision-round-trip-{}", scope.request_ref);
        let capability_grant_id = resolved
            .creates_grant
            .then(|| format!("grant-round-trip-{}-{}", scope.request_ref, resolved.effect));

        // Lifecycle step 2: append `permission.requested` (records the requested
        // scope + the offered ACP option list).
        self.append_round_trip_event(
            scope,
            EventKind::PermissionRequested,
            "requested",
            round_trip_requested_payload(scope, request, &permission_decision_id),
        )?;

        // Lifecycle step 4: append `permission.decided` (records the Capo
        // decision, the chosen ACP option / cancel, decision_source/persistence/
        // explanation, and the adapter_options/adapter_response).
        self.append_round_trip_event(
            scope,
            EventKind::PermissionDecided,
            "decided",
            round_trip_decided_payload(
                scope,
                request,
                &mapping,
                &resolved,
                &permission_decision_id,
                capability_grant_id.as_deref(),
            ),
        )?;

        // Lifecycle step 5: on an allow (or a durable `reject_always` deny) with
        // non-observational persistence, materialize the durable grant.
        if let Some(grant_id) = capability_grant_id.as_deref() {
            self.append_round_trip_grant(
                scope,
                request,
                &resolved,
                &permission_decision_id,
                grant_id,
            )?;
        }

        let adapter_error = matches!(
            mapping.cancel_reason,
            Some(AdapterPermissionCancelReason::NoSelectableOption)
        );

        Ok(AdapterPermissionResponse {
            outcome: mapping.outcome,
            capo_decision: resolved.effect.to_string(),
            capo_persistence: resolved.persistence.map(str::to_string),
            permission_decision_id,
            capability_grant_id,
            adapter_error,
        })
    }

    fn append_round_trip_event(
        &self,
        scope: &PermissionRoundTripScope,
        kind: EventKind,
        suffix: &str,
        payload: String,
    ) -> StateResult<()> {
        self.state.append_event(
            scoped_event(
                &format!(
                    "event-permission-round-trip-{}-{}-{}",
                    scope.session_id, scope.request_ref, suffix
                ),
                kind,
                &self.project_id,
                &scope.task_id,
                &scope.agent_id,
                &scope.session_id,
                &scope.run_id,
            )
            .with_turn(scope.turn_id.to_string())
            .with_item(scope.request_ref.clone())
            .with_payload(payload),
            &[],
        )?;
        Ok(())
    }

    fn append_round_trip_grant(
        &self,
        scope: &PermissionRoundTripScope,
        request: &AdapterPermissionRequest,
        resolved: &ResolvedDecision,
        permission_decision_id: &str,
        grant_id: &str,
    ) -> StateResult<()> {
        let scope_json = serde_json::json!([request.scope]).to_string();
        let subject_json =
            serde_json::json!({ "session_id": scope.session_id.to_string() }).to_string();
        let persistence = resolved.persistence.unwrap_or("once").to_string();
        let payload = serde_json::json!({
            "request_ref": scope.request_ref,
            "permission_decision_id": permission_decision_id,
            "capability_grant_id": grant_id,
            "effect": resolved.effect,
            "decision_source": resolved.decision_source,
            "persistence": persistence,
            "explanation": resolved.explanation,
        })
        .to_string();
        self.state.append_event(
            scoped_event(
                &format!(
                    "event-permission-round-trip-{}-{}-grant",
                    scope.session_id, scope.request_ref
                ),
                EventKind::CapabilityGrantCreated,
                &self.project_id,
                &scope.task_id,
                &scope.agent_id,
                &scope.session_id,
                &scope.run_id,
            )
            .with_turn(scope.turn_id.to_string())
            .with_item(scope.request_ref.clone())
            .with_payload(payload),
            &[ProjectionRecord::CapabilityGrant(
                capo_state::CapabilityGrantProjection {
                    capability_grant_id: grant_id.to_string(),
                    capability_profile_id: request.capability_profile_id.clone(),
                    scope_json,
                    effect: resolved.effect.to_string(),
                    subject_json,
                    decision_source: resolved.decision_source.clone(),
                    persistence,
                    explanation: resolved.explanation.clone(),
                    updated_sequence: 0,
                },
            )],
        )?;
        Ok(())
    }
}

/// The resolved Capo decision after combining the policy decision with the ACP
/// option mapping.
struct ResolvedDecision {
    /// `allow` / `deny` / `cancel`.
    effect: &'static str,
    /// The grant persistence the decision downscoped to, or `None` on cancel /
    /// transient reject.
    persistence: Option<&'static str>,
    decision_source: String,
    explanation: String,
    /// Whether a durable `capability.grant_created` should be materialized.
    creates_grant: bool,
}

/// Combine the policy decision with the ACP option mapping into the final Capo
/// decision (the documented step-3 evaluate + step-5 grant rule).
///
/// - A cancel mapping (operator cancel or no-selectable-option) stays a cancel:
///   no grant, no persistence.
/// - An allow option allows ONLY when the policy also allows the scope; a policy
///   deny is reflected (the adapter cannot over-rule the policy), and the
///   downscoped persistence is dropped (a denied allow creates no grant).
/// - A reject option records a Capo `deny`: `reject_once` is transient (no
///   grant), `reject_always` is a durable deny grant (`until_revoked`).
fn resolve_decision(
    policy: &PermissionPolicy,
    policy_decision: &capo_tools::PermissionDecision,
    mapping: &AcpOptionMapping,
) -> ResolvedDecision {
    let decision_source = policy_decision.decision_source.clone();
    match mapping.capo_decision {
        "cancel" => ResolvedDecision {
            effect: "cancel",
            persistence: None,
            decision_source,
            explanation: cancel_explanation(mapping),
            creates_grant: false,
        },
        "allow" => {
            let policy_allows = policy_decision.effect == "allow";
            if policy_allows {
                let persistence = mapping.capo_persistence.unwrap_or("until_turn_end");
                ResolvedDecision {
                    effect: "allow",
                    persistence: Some(persistence),
                    decision_source,
                    explanation: format!(
                        "policy `{}` permits scope; ACP `{}` mapped to allow ({})",
                        policy.default_profile_id(),
                        mapping
                            .selected
                            .as_ref()
                            .map(|option| option.kind.as_str())
                            .unwrap_or("allow"),
                        persistence
                    ),
                    creates_grant: true,
                }
            } else {
                // The adapter offered an allow option but the policy denies the
                // scope: the policy wins. No grant.
                ResolvedDecision {
                    effect: "deny",
                    persistence: None,
                    decision_source,
                    explanation: format!(
                        "policy denied the scope ({}); the ACP allow option cannot over-rule it",
                        policy_decision.explanation
                    ),
                    creates_grant: false,
                }
            }
        }
        // A reject option records a Capo deny. Durable (`reject_always`) creates a
        // standing deny grant; transient (`reject_once`) records the rejection
        // only.
        _reject => {
            let durable = matches!(mapping.capo_persistence, Some("until_revoked"));
            ResolvedDecision {
                effect: "deny",
                persistence: mapping.capo_persistence,
                decision_source,
                explanation: format!(
                    "ACP `{}` mapped to a Capo reject{}",
                    mapping
                        .selected
                        .as_ref()
                        .map(|option| option.kind.as_str())
                        .unwrap_or("reject"),
                    if durable {
                        " (durable deny grant)"
                    } else {
                        " (transient, no grant)"
                    }
                ),
                creates_grant: durable,
            }
        }
    }
}

fn cancel_explanation(mapping: &AcpOptionMapping) -> String {
    match mapping.cancel_reason {
        Some(AdapterPermissionCancelReason::NoSelectableOption) => {
            "no selectable ACP option offered; recorded cancel and failed the adapter request"
                .to_string()
        }
        _ => "permission request canceled".to_string(),
    }
}

/// The `permission.requested` payload: records the requested scope and the
/// offered ACP option list (`adapter_options`).
fn round_trip_requested_payload(
    scope: &PermissionRoundTripScope,
    request: &AdapterPermissionRequest,
    permission_decision_id: &str,
) -> String {
    let adapter_options = request
        .options
        .iter()
        .map(|option| {
            serde_json::json!({
                "option_id": option.option_id,
                "name": option.name,
                "kind": option.kind.as_str(),
            })
        })
        .collect::<Vec<_>>();
    serde_json::json!({
        "request_ref": scope.request_ref,
        "permission_decision_id": permission_decision_id,
        "tool": request.tool_name,
        "scope": request.scope,
        "capability_profile_id": request.capability_profile_id,
        "adapter_options": adapter_options,
    })
    .to_string()
}

/// The `permission.decided` payload: records the Capo decision, the chosen ACP
/// option / cancel (`adapter_response`), the full decision_source/persistence/
/// explanation, and the offered `adapter_options` so the decision record is
/// self-describing.
fn round_trip_decided_payload(
    scope: &PermissionRoundTripScope,
    request: &AdapterPermissionRequest,
    mapping: &AcpOptionMapping,
    resolved: &ResolvedDecision,
    permission_decision_id: &str,
    capability_grant_id: Option<&str>,
) -> String {
    let decision = match resolved.effect {
        // Capo's decision vocabulary uses `reject` for a deny decision in the
        // ACP-mapping table; the durable record keeps `deny` as the grant effect
        // but the decision is recorded as `reject`/`allow`/`cancel`.
        "deny" => "reject",
        other => other,
    };
    let adapter_response = match &mapping.outcome {
        AcpPermissionOutcome::Selected { option_id } => serde_json::json!({
            "outcome": "selected",
            "option_id": option_id,
        }),
        AcpPermissionOutcome::Cancelled => serde_json::json!({
            "outcome": "cancelled",
            "reason": mapping
                .cancel_reason
                .map(|reason| reason.as_str())
                .unwrap_or("cancelled"),
        }),
    };
    let adapter_options = request.option_ids();
    serde_json::json!({
        "request_ref": scope.request_ref,
        "permission_decision_id": permission_decision_id,
        "tool": request.tool_name,
        "scope": request.scope,
        "decision": decision,
        "effect": resolved.effect,
        "capability_grant_id": capability_grant_id,
        "decision_source": resolved.decision_source,
        "persistence": resolved.persistence,
        "explanation": resolved.explanation,
        "adapter_options": adapter_options,
        "adapter_response": adapter_response,
    })
    .to_string()
}
