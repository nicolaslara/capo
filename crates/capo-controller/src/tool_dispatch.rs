//! ACI1: real tool dispatch wired into the loop.
//!
//! This is the seam the `tools-aci` workpad's first task lands: a tool-invoking
//! turn DRIVES the existing execution substrate (the same `append_event` /
//! projection path the loop already uses) instead of forking a second
//! pipeline. The controller takes a typed [`capo_tools::ToolExposureRequest`],
//! routes it through [`capo_tools::ToolExposure::authorize_and_invoke`] -- which
//! calls the REAL [`capo_tools::CapoToolRegistry::authorize_and_invoke`] /
//! [`capo_tools::RuntimeToolWrappers::authorize_and_invoke`], never the fake
//! summary shim -- and then NORMALIZES the resulting typed audit events onto the
//! canonical `tool.*`/`permission.*`/`capability.*` event kinds, keyed to the
//! turn.
//!
//! It deliberately reuses the loop's existing primitives (`scoped_event`,
//! `append_event`, `ToolCallProjection`) and does NOT call
//! `append_dispatch_run_exit`: a tool call annotates the in-flight run with its
//! observed evidence; it does not duplicate run-completion semantics.

use std::time::{SystemTime, UNIX_EPOCH};

use capo_tools::{
    AgentReportRecord, CapoToolResult, EVIDENCE_SOURCE_RUNTIME_OUTPUT, PermissionDecision,
    ToolAuditEvent, ToolExposure, ToolExposureRequest, ToolExposureResult, WrapperToolResult,
};

use super::*;

/// Wall-clock millis-since-epoch for the per-call `started_at`/`completed_at`
/// timing (ACI7), consistent with the ACI6 typed-test timing fields. A clock
/// before the epoch (impossible in practice) is clamped to 0.
fn epoch_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

/// Where a dispatched tool call hangs on the loop's scope tree, so the persisted
/// events carry the same task/agent/session/run/turn provenance the rest of the
/// loop uses.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolDispatchScope {
    pub task_id: TaskId,
    pub agent_id: AgentId,
    pub session_id: SessionId,
    pub run_id: RunId,
    pub turn_id: TurnId,
    pub tool_call_id: ToolCallId,
}

/// The persisted outcome of one real tool dispatch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolDispatchOutcome {
    pub tool_call_id: ToolCallId,
    pub tool_name: String,
    pub tool_origin: String,
    pub status: String,
    pub output_artifact_id: Option<String>,
    /// The canonical event kinds appended for this call, in order, so a test can
    /// assert the real audit event sequence.
    pub observed_event_kinds: Vec<String>,
    /// SG1: the typed decide outcome recorded for this call before the tool ran
    /// (or was blocked). Carries the policy `PermissionDecision`'s
    /// `decision_source`/`persistence`/`explanation` so the loop has a structured
    /// decide outcome (not a raw error string) even when everything is allowed,
    /// and so a `deny` surfaces an agent-readable refusal the loop can reflect on.
    pub decide: PermissionDecideOutcome,
    /// The raw typed result, so callers can inspect the narrow output.
    pub result: ToolExposureResult,
}

/// SG1: the typed permission-decide outcome the real loop's decide step records
/// before any tool invocation or workspace write proceeds.
///
/// Every dispatch -- allow OR deny -- carries one of these so the audit trail is
/// complete even when the decision is allow, and a denied write surfaces as a
/// structured, agent-readable refusal (not a raw error string) the loop can
/// reflect on rather than silently continuing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PermissionDecideOutcome {
    /// `true` when the policy allowed the request (`effect == "allow"`).
    pub allowed: bool,
    /// The raw policy effect (`allow`/`deny`/`cancel`).
    pub effect: String,
    /// The capability grant id the decision issued (an allow grant authorizes a
    /// later request; a deny grant blocks it).
    pub capability_grant_id: String,
    /// `decision_source` from the [`PermissionDecision`] (e.g.
    /// `allow_trusted_local_profile`, `static_policy:read-only-local`), recorded
    /// on every decide outcome so the audit trail names the policy that decided.
    pub decision_source: String,
    /// `persistence` from the [`PermissionDecision`] (`once`/`until_session_end`/
    /// ...). Drives whether the allow path materializes a durable grant.
    pub persistence: String,
    /// The human/agent-readable explanation from the [`PermissionDecision`].
    pub explanation: String,
    /// `true` when a `capability.grant_created` event was appended for this
    /// decision (allow or deny) with non-observational persistence.
    pub grant_created: bool,
    /// The structured, agent-readable refusal for a denied call, or `None` when
    /// allowed. The loop reflects on this rather than a raw error string.
    pub refusal: Option<ToolRefusal>,
}

impl PermissionDecideOutcome {
    fn from_decision(decision: &PermissionDecision, tool_name: &str, grant_created: bool) -> Self {
        let allowed = decision.effect == "allow";
        let refusal = (!allowed).then(|| ToolRefusal {
            tool_name: tool_name.to_string(),
            decision_source: decision.decision_source.clone(),
            scope_json: decision.scope_json.clone(),
            reason: decision.explanation.clone(),
        });
        Self {
            allowed,
            effect: decision.effect.clone(),
            capability_grant_id: decision.capability_grant_id.clone(),
            decision_source: decision.decision_source.clone(),
            persistence: decision.persistence.clone(),
            explanation: decision.explanation.clone(),
            grant_created,
            refusal,
        }
    }
}

/// SG1: a structured, agent-readable refusal for a denied tool call.
///
/// Maps a policy `deny` for a (potentially write) tool onto a typed value the
/// loop can reflect on and surface back to the agent -- the tool that was
/// refused, which policy refused it, the scope it asked for, and the policy's
/// own explanation -- rather than a raw error string.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolRefusal {
    pub tool_name: String,
    pub decision_source: String,
    pub scope_json: String,
    pub reason: String,
}

impl ToolRefusal {
    /// A single agent-readable line summarizing the refusal, for the loop to
    /// reflect back to the model.
    pub fn agent_message(&self) -> String {
        format!(
            "Permission denied for tool `{}` by `{}`: {} (requested scope: {})",
            self.tool_name, self.decision_source, self.reason, self.scope_json
        )
    }
}

impl FakeBoundaryController {
    /// Dispatch a real tool call through `exposure.authorize_and_invoke` and
    /// persist the canonical tool-call event sequence keyed to the turn.
    ///
    /// `exposure` is the REAL registry/wrappers handle (the production
    /// [`RealBoundaryController`] holds one; the test-only fake exposure is never
    /// the default). The typed result's audit events are normalized onto the
    /// loop's existing event kinds, so the persisted sequence is the same one the
    /// `tool-exposure.md` design specifies and the loop's
    /// `reconstruct_turn_finished` already consumes.
    pub fn dispatch_tool_call(
        &self,
        exposure: &ToolExposure,
        scope: &ToolDispatchScope,
        request: ToolExposureRequest,
    ) -> StateResult<ToolDispatchOutcome> {
        // ACI7: capture wall-clock timing around the real authorize+invoke so the
        // persisted `ToolCall` projection carries `started_at`/`completed_at` for
        // later evaluation.
        let started_at = epoch_millis();
        let result = exposure.authorize_and_invoke(request, &self.permission_policy);
        let completed_at = epoch_millis();
        let normalized = NormalizedToolResult::from_result(&result);
        // ACI7: the per-call provenance that ties command -> turn -> permission ->
        // tool -> artifact into one queryable chain. The `correlation_id` is the
        // turn-scoped identity already stamped on every event's item ref (the
        // tool_call_id), and the permission-decision / grant-use ids pin the
        // authorization that allowed (or denied) the call.
        let provenance = capo_state::ToolCallProvenance {
            correlation_id: Some(dispatch_correlation_id(scope)),
            permission_decision_id: normalized.permission_decision_id.clone(),
            capability_grant_use_id: normalized
                .capability_grant_id
                .as_ref()
                .map(|grant_id| dispatch_grant_use_id(scope, grant_id)),
            started_at: Some(started_at),
            completed_at: Some(completed_at),
        };
        // The persistable canonical kinds for this dispatch, in order. Audit
        // events with no loop counterpart (`tool.call_canceled`/`tool.call_failed`)
        // are dropped here, but their terminal STATUS is still applied to the
        // projection below so a denied/failed call never sticks at "requested".
        let persistable: Vec<(usize, EventKind, &ToolAuditEvent)> = normalized
            .events
            .iter()
            .enumerate()
            .filter_map(|(index, audit_event)| {
                tool_audit_event_kind(audit_event).map(|kind| (index, kind, audit_event))
            })
            .collect();
        // Whether the registry/wrappers reached a `tool.call_completed`. If not
        // (deny/fail), the loop never runs the `ToolCallCompleted` projection
        // branch, so we must stamp the terminal projection onto the LAST
        // persisted event instead -- otherwise the persisted projection (read by
        // `tool_calls_for_session`, dashboards, recovery) stays at "requested"
        // even though the dispatch outcome is "denied"/"failed".
        let reaches_completed = persistable
            .iter()
            .any(|(_, kind, _)| matches!(kind, EventKind::ToolCallCompleted));
        let last_index = persistable.len().saturating_sub(1);
        // SG1: the decide step materializes a `CapabilityGrant` (the lifecycle
        // step 5 `capability.grant_created`) when the recorded decision has
        // non-observational persistence. The tool layer's audit stream goes
        // straight from `permission.decided` to `capability.grant_used`, so the
        // grant-creation event is injected here, in the loop's decide step,
        // immediately after the `permission.decided` event is recorded -- the
        // tool/runtime layer's later `capability.grant_used`/invocation events
        // proceed only after that decision is on the log.
        let emit_grant_created = decision_creates_grant(
            &normalized.permission_decision,
            &normalized.capability_grant_id,
        );
        let mut grant_created = false;
        let mut observed_event_kinds = Vec::with_capacity(persistable.len());
        for (position, (index, kind, audit_event)) in persistable.iter().enumerate() {
            let kind = *kind;
            let mut projections =
                self.dispatch_event_projections(kind, scope, &normalized, &provenance);
            // Terminal projection for the non-completed paths: attach the final
            // `ToolCall` projection (status == normalized.status, i.e.
            // "denied"/"failed") to the dispatch's last persisted event so every
            // path drives the projection to its true terminal status.
            if !reaches_completed && position == last_index {
                projections.push(terminal_tool_call_projection(
                    scope,
                    &normalized,
                    &provenance,
                ));
            }
            self.state.append_event(
                scoped_event(
                    &format!(
                        "event-tool-dispatch-{}-{}-{}",
                        scope.session_id,
                        scope.tool_call_id,
                        dispatch_event_suffix(kind, *index)
                    ),
                    kind,
                    &self.project_id,
                    &scope.task_id,
                    &scope.agent_id,
                    &scope.session_id,
                    &scope.run_id,
                )
                .with_turn(scope.turn_id.to_string())
                // Stamp a shared item ref (the tool_call_id) across every event of
                // this one tool call so `persisted_turn_ref`/`reconstruct_turn_finished`
                // dedup collapse them to a SINGLE observed tool ref per call,
                // matching the loop's documented replay-identity invariant.
                .with_item(scope.tool_call_id.to_string())
                .with_payload(dispatch_event_payload(
                    kind,
                    scope,
                    &normalized,
                    audit_event,
                )),
                &projections,
            )?;
            observed_event_kinds.push(kind.as_str().to_string());

            // SG1: record `capability.grant_created` directly after the decision
            // is on the log (lifecycle steps 4 -> 5). The projection carries the
            // full decision record (`decision_source`/`persistence`/`explanation`)
            // so the durable grant store has a row to read back (SG3).
            if matches!(kind, EventKind::PermissionDecided) && emit_grant_created {
                self.append_capability_grant_created(scope, &normalized.permission_decision)?;
                observed_event_kinds.push(EventKind::CapabilityGrantCreated.as_str().to_string());
                grant_created = true;
            }
        }
        let decide = PermissionDecideOutcome::from_decision(
            &normalized.permission_decision,
            &normalized.tool_name,
            grant_created,
        );
        Ok(ToolDispatchOutcome {
            tool_call_id: scope.tool_call_id.clone(),
            tool_name: normalized.tool_name,
            tool_origin: normalized.tool_origin,
            status: normalized.status,
            output_artifact_id: normalized.output_artifact_id,
            decide,
            observed_event_kinds,
            result,
        })
    }

    fn dispatch_event_projections(
        &self,
        kind: EventKind,
        scope: &ToolDispatchScope,
        normalized: &NormalizedToolResult,
        provenance: &capo_state::ToolCallProvenance,
    ) -> Vec<ProjectionRecord> {
        match kind {
            EventKind::ToolCallRequested => {
                vec![ProjectionRecord::ToolCall(capo_state::ToolCallProjection {
                    tool_call_id: scope.tool_call_id.clone(),
                    session_id: scope.session_id.clone(),
                    turn_id: Some(scope.turn_id.to_string()),
                    tool_name: normalized.tool_name.clone(),
                    tool_origin: normalized.tool_origin.clone(),
                    status: "requested".to_string(),
                    input_artifact_id: normalized.input_artifact_id.clone(),
                    output_artifact_id: None,
                    provenance: provenance.clone(),
                    updated_sequence: 0,
                })]
            }
            EventKind::ToolObservationRecorded => {
                // ACI8: persist the agent report as a DISTINCT observation class
                // tagged `source=agent_reported` (carrying confidence), separate
                // from observed runtime/adapter evidence, so completion is never
                // reachable by agent assertion alone. Observed tools (Capo
                // registry / runtime wrappers) carry no `agent_report`, so this
                // projection is emitted ONLY for the reporting surface.
                match &normalized.agent_report {
                    Some(report) => vec![ProjectionRecord::ToolObservation(
                        capo_state::ToolObservationProjection {
                            tool_observation_id: format!("agent-report-obs-{}", scope.tool_call_id),
                            session_id: scope.session_id.clone(),
                            tool_call_id: Some(scope.tool_call_id.clone()),
                            source: report.source.clone(),
                            external_tool_ref: None,
                            tool_name: normalized.tool_name.clone(),
                            observed_status: "reported".to_string(),
                            instrumentation_level: "structured_observed".to_string(),
                            confidence: report.confidence.to_string(),
                            raw_event_hash: format!("agent-report:{}", scope.tool_call_id),
                            artifact_id: None,
                            updated_sequence: 0,
                        },
                    )],
                    None => Vec::new(),
                }
            }
            EventKind::ToolOutputObserved => {
                // ACI9: normalize an OBSERVED tool result (Capo registry / runtime
                // wrappers) into the `ToolObservation` projection too, tagged
                // `source=runtime_output` -- a DISTINCT class from the
                // `agent_reported` claim above. Without this row a query over
                // `tool_observations_for_session` would surface only agent reports
                // for locally-dispatched tools, leaving observed evidence with no
                // observation projection; the two must be co-queryable and
                // distinguishable. The reporting surface carries no
                // `observed_evidence` (it is a claim, not observed proof), so this
                // projection is emitted ONLY for observed tools.
                match &normalized.observed_evidence {
                    Some(observed) => vec![ProjectionRecord::ToolObservation(
                        capo_state::ToolObservationProjection {
                            tool_observation_id: format!("runtime-obs-{}", scope.tool_call_id),
                            session_id: scope.session_id.clone(),
                            tool_call_id: Some(scope.tool_call_id.clone()),
                            source: observed.source.clone(),
                            external_tool_ref: None,
                            tool_name: normalized.tool_name.clone(),
                            observed_status: observed.observed_status.clone(),
                            instrumentation_level: observed.instrumentation_level.clone(),
                            confidence: "observed".to_string(),
                            raw_event_hash: format!("runtime-observed:{}", scope.tool_call_id),
                            artifact_id: normalized.output_artifact_id.clone(),
                            updated_sequence: 0,
                        },
                    )],
                    None => Vec::new(),
                }
            }
            EventKind::ToolCallCompleted => {
                vec![terminal_tool_call_projection(scope, normalized, provenance)]
            }
            _ => Vec::new(),
        }
    }

    /// SG1: append the lifecycle step-5 `capability.grant_created` event and its
    /// durable [`CapabilityGrantProjection`], keyed to the turn.
    ///
    /// Called by the decide step immediately after the `permission.decided`
    /// event, for any decision with non-observational persistence (allow OR a
    /// `reject_always`-style deny grant). The persisted grant row carries the full
    /// decision record (`effect`/`decision_source`/`persistence`/`explanation`)
    /// so the durable grant store has a queryable, read-backable row -- this is
    /// the grant SG3 reads back to authorize a later request, and the deny grant
    /// that blocks one. The event payload uses `serde_json` so every interpolated
    /// field is escaped for the full JSON grammar.
    fn append_capability_grant_created(
        &self,
        scope: &ToolDispatchScope,
        decision: &PermissionDecision,
    ) -> StateResult<()> {
        let payload = serde_json::json!({
            "tool_call_id": scope.tool_call_id.to_string(),
            "capability_grant_id": decision.capability_grant_id,
            "effect": decision.effect,
            "decision_source": decision.decision_source,
            "persistence": decision.persistence,
            "explanation": decision.explanation,
        })
        .to_string();
        self.state.append_event(
            scoped_event(
                &format!(
                    "event-tool-dispatch-{}-{}-{}",
                    scope.session_id,
                    scope.tool_call_id,
                    dispatch_event_suffix(EventKind::CapabilityGrantCreated, 0)
                ),
                EventKind::CapabilityGrantCreated,
                &self.project_id,
                &scope.task_id,
                &scope.agent_id,
                &scope.session_id,
                &scope.run_id,
            )
            .with_turn(scope.turn_id.to_string())
            .with_item(scope.tool_call_id.to_string())
            .with_payload(payload),
            &[ProjectionRecord::CapabilityGrant(
                capo_state::CapabilityGrantProjection {
                    capability_grant_id: decision.capability_grant_id.clone(),
                    capability_profile_id: decision.capability_profile_id.clone(),
                    scope_json: decision.scope_json.clone(),
                    effect: decision.effect.clone(),
                    subject_json: decision.subject_json.clone(),
                    decision_source: decision.decision_source.clone(),
                    persistence: decision.persistence.clone(),
                    explanation: decision.explanation.clone(),
                    updated_sequence: 0,
                },
            )],
        )?;
        Ok(())
    }
}

/// SG1: whether a recorded permission decision materializes a durable
/// `CapabilityGrant` (lifecycle step 5).
///
/// A grant is created for any decision whose persistence is NOT purely
/// observational and that issued a grant id. Every persistence value the
/// prototype policies emit today (`once`, `until_turn_end`, `until_session_end`,
/// `until_revoked`, `until_time`) is non-observational and creates a grant
/// (an allow grant authorizes a later request; a `reject_always` deny grant
/// blocks one); a future `observational` persistence would skip grant creation.
fn decision_creates_grant(decision: &PermissionDecision, grant_id: &Option<String>) -> bool {
    grant_id.is_some() && !persistence_is_observational(&decision.persistence)
}

/// Whether a persistence value is purely observational (records a decision but
/// creates no durable grant). The lifecycle's "persistence is not purely
/// observational" condition for emitting `capability.grant_created`.
fn persistence_is_observational(persistence: &str) -> bool {
    matches!(persistence, "observational" | "none")
}

/// The stable correlation id for one tool dispatch (ACI7): a turn-scoped value
/// that ties the command -> turn -> permission -> tool -> artifact chain. The
/// tool_call_id is the join key already stamped on every event's item ref; the
/// session/run/turn prefix keeps the id self-describing for cross-projection
/// queries.
fn dispatch_correlation_id(scope: &ToolDispatchScope) -> String {
    format!(
        "corr-{}-{}-{}-{}",
        scope.session_id, scope.run_id, scope.turn_id, scope.tool_call_id
    )
}

/// The per-invocation capability-grant-use id (ACI7): the grant the permission
/// decision issued, scoped to THIS tool call, so two calls that reuse the same
/// grant still carry distinct grant-use ids.
fn dispatch_grant_use_id(scope: &ToolDispatchScope, grant_id: &str) -> String {
    format!("grant-use-{}-{}", scope.tool_call_id, grant_id)
}

/// The terminal `ToolCall` projection for a dispatch: status == the normalized
/// dispatch status (`completed`/`denied`/`failed`). Used both for the
/// `tool.call_completed` (allow+success) branch AND, for the deny/fail paths that
/// carry no `tool.call_completed` event, attached to the last persisted event so
/// the persisted projection always reaches its true terminal status.
fn terminal_tool_call_projection(
    scope: &ToolDispatchScope,
    normalized: &NormalizedToolResult,
    provenance: &capo_state::ToolCallProvenance,
) -> ProjectionRecord {
    ProjectionRecord::ToolCall(capo_state::ToolCallProjection {
        tool_call_id: scope.tool_call_id.clone(),
        session_id: scope.session_id.clone(),
        turn_id: Some(scope.turn_id.to_string()),
        tool_name: normalized.tool_name.clone(),
        tool_origin: normalized.tool_origin.clone(),
        status: normalized.status.clone(),
        input_artifact_id: normalized.input_artifact_id.clone(),
        output_artifact_id: normalized.output_artifact_id.clone(),
        provenance: provenance.clone(),
        updated_sequence: 0,
    })
}

/// A variant-erased view of the typed tool result, so the canonical normalization
/// is one code path regardless of whether the tool was Capo-owned or a runtime
/// wrapper.
struct NormalizedToolResult {
    tool_name: String,
    tool_origin: String,
    status: String,
    input_artifact_id: Option<String>,
    output_artifact_id: Option<String>,
    /// The capability grant the permission decision issued for this call (ACI7).
    /// `None` only for a degenerate result with no decision.
    capability_grant_id: Option<String>,
    /// The permission-decision id pinned to this call (ACI7), derived from the
    /// issued grant so the projection can join back to the authorization.
    permission_decision_id: Option<String>,
    /// SG1: the full policy [`PermissionDecision`] for this call, so the dispatch
    /// decide step can (a) enrich the persisted `permission.decided` event with
    /// `decision_source`/`persistence`/`explanation`, (b) materialize a
    /// `capability.grant_created` event + projection on a non-observational
    /// decision, and (c) surface a typed allow/deny decide outcome.
    permission_decision: PermissionDecision,
    /// ACI8: the agent-report classification for a `GO2` reporting tool, carrying
    /// the `agent_reported` source tag and the agent's self-declared confidence.
    /// `None` for observed tools (Capo registry / runtime wrappers), so a report
    /// is never persisted indistinguishably from observed evidence.
    agent_report: Option<AgentReportObservation>,
    /// ACI9: the observed-evidence classification for an OBSERVED tool (Capo
    /// registry / runtime wrappers), tagged `source=runtime_output` and carrying
    /// the observed terminal status + instrumentation level. `None` for the
    /// reporting surface (a claim, not observed proof), so observed evidence and
    /// agent reports remain a distinct class in the `ToolObservation` projection.
    observed_evidence: Option<ObservedEvidence>,
    events: Vec<ToolAuditEvent>,
}

/// ACI8: the distinct classification an agent report carries onto the persisted
/// `tool.observation_recorded` projection: `source=agent_reported` (never an
/// observed-evidence source) plus the agent's self-declared confidence.
struct AgentReportObservation {
    source: String,
    confidence: i64,
}

/// ACI9: the distinct classification an OBSERVED tool result carries onto the
/// persisted `tool.output_observed` -> `ToolObservation` projection:
/// `source=runtime_output` (an observed-evidence source, never `agent_reported`)
/// plus the observed terminal status and the instrumentation level.
struct ObservedEvidence {
    source: String,
    observed_status: String,
    instrumentation_level: String,
}

impl NormalizedToolResult {
    fn from_result(result: &ToolExposureResult) -> Self {
        match result {
            ToolExposureResult::Capo(result) => Self::from_capo(result),
            ToolExposureResult::Runtime(result) => Self::from_runtime(result),
            ToolExposureResult::AgentReport(result) => Self::from_agent_report(result),
            ToolExposureResult::Fake(_) => {
                // The fake summary shim is not a real dispatch result; ACI1's
                // real path never routes it here.
                panic!(
                    "dispatch_tool_call received a fake tool result; the real loop \
                     dispatches Capo/Runtime/AgentReport tools only"
                )
            }
        }
    }

    fn from_agent_report(result: &AgentReportRecord) -> Self {
        let status = if result.accepted {
            "completed".to_string()
        } else {
            "denied".to_string()
        };
        let grant_id = result.permission_decision.capability_grant_id.clone();
        Self {
            tool_name: result.tool_id.clone(),
            // ACI8: a Capo-owned reporting tool, but tagged distinctly through
            // the agent-report observation below so a report's persisted
            // observation reads `source=agent_reported`, not observed evidence.
            tool_origin: "capo".to_string(),
            status,
            input_artifact_id: None,
            output_artifact_id: None,
            permission_decision_id: Some(permission_decision_id(&grant_id)),
            capability_grant_id: Some(grant_id),
            permission_decision: result.permission_decision.clone(),
            agent_report: Some(AgentReportObservation {
                source: result.source.clone(),
                confidence: result.confidence,
            }),
            // ACI9: a report is a CLAIM, not observed proof, so it carries no
            // observed-evidence classification -- it persists ONLY the
            // `agent_reported` observation, never a `runtime_output` one.
            observed_evidence: None,
            events: result.events.clone(),
        }
    }

    fn from_capo(result: &CapoToolResult) -> Self {
        let allowed = result.permission_decision.effect == "allow";
        let status = if allowed {
            "completed".to_string()
        } else {
            "denied".to_string()
        };
        let grant_id = result.permission_decision.capability_grant_id.clone();
        Self {
            tool_name: result.tool_id.clone(),
            tool_origin: "capo".to_string(),
            status: status.clone(),
            input_artifact_id: None,
            output_artifact_id: artifact_or_none(&result.output_artifact_id),
            permission_decision_id: Some(permission_decision_id(&grant_id)),
            capability_grant_id: Some(grant_id),
            permission_decision: result.permission_decision.clone(),
            agent_report: None,
            // ACI9: an allowed Capo tool produces OBSERVED runtime evidence; the
            // `tool.output_observed` event drives the `runtime_output` observation
            // projection. A denied call emits no `tool.output_observed` event, so
            // the observation row is never created even though the field is set.
            observed_evidence: Some(ObservedEvidence {
                source: EVIDENCE_SOURCE_RUNTIME_OUTPUT.to_string(),
                observed_status: status,
                instrumentation_level: "full".to_string(),
            }),
            events: result.events.clone(),
        }
    }

    fn from_runtime(result: &WrapperToolResult) -> Self {
        let grant_id = result.permission_decision.capability_grant_id.clone();
        Self {
            tool_name: result.tool_id.clone(),
            tool_origin: "runtime".to_string(),
            // Normalize onto the dispatch terminal-status vocabulary
            // (`completed`/`failed`/`denied`) that downstream consumers
            // (dashboards, safety-gates score_run, goal-autonomy evidence) match
            // on. A wrapper may carry finer-grained terminal statuses for the
            // loop (e.g. `precondition_failed`, or the runtime's `exited`), but
            // the persisted dispatch status must stay in the shared vocabulary so
            // a non-completed outcome is never mis-bucketed as a completion. The
            // process-runner's `exited` is NOT itself a pass/fail discriminator
            // (a non-zero exit also reports `exited`), so it is folded using the
            // wrapper's own `passed` signal: `exited` + passed -> `completed`,
            // `exited` + !passed -> `failed`.
            status: normalize_runtime_status(&result.status, wrapper_passed(result)),
            input_artifact_id: result
                .input_artifact
                .as_ref()
                .map(|artifact| artifact.artifact_id.clone()),
            output_artifact_id: result
                .output_artifacts
                .first()
                .map(|artifact| artifact.artifact_id.clone()),
            permission_decision_id: Some(permission_decision_id(&grant_id)),
            capability_grant_id: Some(grant_id),
            permission_decision: result.permission_decision.clone(),
            agent_report: None,
            // ACI9: a runtime wrapper that ran produces OBSERVED evidence; the
            // `tool.output_observed` event drives the `runtime_output` observation
            // projection, carrying the wrapper's OWN observed status (`exited` /
            // `failed` / `precondition_failed` / `no_match`) rather than the folded
            // dispatch terminal status, so the observation records what was
            // actually observed. A denied call emits no `tool.output_observed`
            // event, so no observation row is created even though the field is set.
            observed_evidence: Some(ObservedEvidence {
                source: EVIDENCE_SOURCE_RUNTIME_OUTPUT.to_string(),
                observed_status: result.status.clone(),
                instrumentation_level: "full".to_string(),
            }),
            events: result.events.clone(),
        }
    }
}

/// Derive the stable permission-decision id from the issued capability grant
/// (ACI7). The grant id is the decision's identity; the `decision-` prefix marks
/// it as the decision-projection join key rather than the raw grant.
fn permission_decision_id(grant_id: &str) -> String {
    format!("decision-{grant_id}")
}

/// The wrapper's own pass signal: the process-runner wrappers (`shell_run`,
/// `test_run`, `git_*`) compute `passed = exit_code == Some(0)` and carry it on
/// the typed output. This is the authoritative pass/fail discriminator for an
/// `exited` process; the raw runner status (`exited`) does not encode it.
/// Defaults to `false` (treat as failure) when the field is absent or non-bool,
/// so an unknown shape is never optimistically bucketed as a completion.
fn wrapper_passed(result: &WrapperToolResult) -> bool {
    result
        .typed_output
        .get("passed")
        .and_then(|passed| passed.as_bool())
        .unwrap_or(false)
}

/// Fold a runtime wrapper's terminal status onto the dispatch terminal-status
/// vocabulary downstream consumers match on.
///
/// Most wrapper statuses are already canonical (`completed`/`failed`/`denied`).
/// Two classes need folding:
///
/// 1. The process-runner's `exited` is NOT a pass/fail discriminator on its own
///    -- a successful AND a non-zero-exit `shell_run`/`test_run`/`git_*` both
///    report `exited`. Folding it through unchanged would persist
///    `status="exited"` and make a non-zero exit indistinguishable from success
///    at the projection level, dropping the discriminator the safety-gates
///    `score_run` consumes. We fold it using the wrapper's own `passed` signal
///    (`exited` + passed -> `completed`, else -> `failed`). The raw `exited`
///    detail still survives on `observed_evidence.observed_status`.
/// 2. The non-completing guards (`precondition_failed` for a stale file_write,
///    `no_match` for an apply_patch hunk no strategy could locate) are real
///    terminal FAILURES for dispatch/projection purposes (no write, no
///    artifact), so they are folded onto `failed`; the precise semantics
///    (expected/actual hashes, rejected hunk index, nearest candidate) still
///    travel on the wrapper result's own status and typed output for the loop to
///    reflect and retry.
fn normalize_runtime_status(status: &str, passed: bool) -> String {
    match status {
        "exited" if passed => "completed".to_string(),
        "exited" | "precondition_failed" | "no_match" => "failed".to_string(),
        other => other.to_string(),
    }
}

fn artifact_or_none(artifact_id: &str) -> Option<String> {
    if artifact_id == "none" {
        None
    } else {
        Some(artifact_id.to_string())
    }
}

/// Map a typed tool-audit event onto the canonical loop [`EventKind`], or `None`
/// for audit events that have no persisted-loop counterpart (e.g. the
/// `tool.call_canceled` denial marker, surfaced through the projected status
/// instead).
fn tool_audit_event_kind(event: &ToolAuditEvent) -> Option<EventKind> {
    match event.kind.as_str() {
        "tool.call_requested" => Some(EventKind::ToolCallRequested),
        "permission.requested" => Some(EventKind::PermissionRequested),
        "permission.decided" => Some(EventKind::PermissionDecided),
        "capability.grant_used" => Some(EventKind::CapabilityGrantUsed),
        "tool.invocation_started" => Some(EventKind::ToolInvocationStarted),
        "tool.output_artifact_recorded" => Some(EventKind::ToolOutputArtifactRecorded),
        "tool.output_observed" => Some(EventKind::ToolOutputObserved),
        // ACI8: an agent report records a distinct `agent_reported` observation,
        // not observed runtime/adapter evidence.
        "tool.observation_recorded" => Some(EventKind::ToolObservationRecorded),
        "tool.call_completed" => Some(EventKind::ToolCallCompleted),
        "tool.result_delivered" => Some(EventKind::ToolResultDelivered),
        _ => None,
    }
}

/// A short, stable event-id suffix per canonical kind. The index disambiguates
/// the rare case where a kind repeats within one call.
fn dispatch_event_suffix(kind: EventKind, index: usize) -> String {
    format!("{}-{index}", kind.as_str().replace('.', "-"))
}

/// Serialize one string field to a complete JSON string literal (surrounding
/// quotes included) via `serde_json`, so EVERY interpolated value is escaped for
/// the full JSON grammar -- control chars, newlines, quotes, backslashes -- not
/// just the `\`/`"` subset the legacy `escape_json` handled. Serializing a `str`
/// is infallible, so the `unwrap_or_default` fallback is never taken in practice
/// (a degenerate empty fragment is still safe inside the assembled object).
fn json_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

/// Build the persisted event payload as JSON. The values are serialized through
/// `serde_json` (`json_string`) rather than interpolated raw, so the payload is
/// ALWAYS valid JSON regardless of what a tool name / status / artifact id
/// contains. The key order is preserved verbatim (matching the prior `format!`
/// layout) so the output stays byte-identical for the current well-formed inputs.
fn dispatch_event_payload(
    kind: EventKind,
    scope: &ToolDispatchScope,
    normalized: &NormalizedToolResult,
    audit_event: &ToolAuditEvent,
) -> String {
    let tool_call_id = json_string(scope.tool_call_id.as_str());
    let output_artifact_id =
        || json_string(normalized.output_artifact_id.as_deref().unwrap_or("none"));
    let tool = || json_string(&normalized.tool_name);
    let status = || json_string(&audit_event.status);
    match kind {
        EventKind::ToolOutputArtifactRecorded => format!(
            "{{\"tool_call_id\":{},\"output_artifact_id\":{},\"redaction_state\":{}}}",
            tool_call_id,
            output_artifact_id(),
            status()
        ),
        EventKind::ToolOutputObserved => format!(
            "{{\"tool_call_id\":{},\"tool\":{},\"status\":{}}}",
            tool_call_id,
            tool(),
            status()
        ),
        // ACI8: the agent-report observation payload records its distinct
        // `source` (the audit event's status carries `agent_reported`) so the
        // persisted event is classifiable without re-deriving from the tool name.
        EventKind::ToolObservationRecorded => format!(
            "{{\"tool_call_id\":{},\"tool\":{},\"source\":{}}}",
            tool_call_id,
            tool(),
            status()
        ),
        EventKind::ToolCallCompleted => format!(
            "{{\"tool_call_id\":{},\"tool\":{},\"output_artifact_id\":{}}}",
            tool_call_id,
            tool(),
            output_artifact_id()
        ),
        // SG1: the decide-step events carry the FULL `PermissionDecision` record so
        // the audit trail is complete even when everything is allowed. The
        // requested event records the scope; the decided event records the
        // effect/grant/decision_source/persistence/explanation. Built through
        // `serde_json` so every field is escaped for the full JSON grammar.
        EventKind::PermissionRequested => {
            let decision = &normalized.permission_decision;
            serde_json::json!({
                "tool_call_id": scope.tool_call_id.to_string(),
                "tool": normalized.tool_name,
                "capability_profile_id": decision.capability_profile_id,
                "scope_json": decision.scope_json,
            })
            .to_string()
        }
        EventKind::PermissionDecided => {
            let decision = &normalized.permission_decision;
            serde_json::json!({
                "tool_call_id": scope.tool_call_id.to_string(),
                "tool": normalized.tool_name,
                "effect": decision.effect,
                "capability_grant_id": decision.capability_grant_id,
                "decision_source": decision.decision_source,
                "persistence": decision.persistence,
                "explanation": decision.explanation,
            })
            .to_string()
        }
        _ => format!(
            "{{\"tool_call_id\":{},\"tool\":{},\"status\":{}}}",
            tool_call_id,
            tool(),
            status()
        ),
    }
}
