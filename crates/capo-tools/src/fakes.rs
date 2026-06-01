//! ACI10: deterministic, scripted fake tool implementations for replayable tests.
//!
//! The real tool surface (`crates/capo-tools/src/runtime_wrappers.rs`,
//! `lib.rs`, `agent_reports.rs`) is exercised end to end through process
//! spawns (`capo.shell_run`, the git wrappers, `capo.test_run`), real
//! filesystem reads/writes (`capo.file_read`, `capo.file_write`,
//! `capo.apply_patch`), and external programs (`rustfmt`, `rg`). Those are not
//! deterministic across machines and cannot be replayed without a live process,
//! so this module provides FAKE/scripted implementations that produce stable
//! outputs for every tool WITHOUT a live provider, a real process, or touching
//! disk.
//!
//! The load-bearing contract (reconciling with `ACI1`): a fake is clearly
//! TEST-ONLY and is never the default in the real controller, AND it emits the
//! SAME event/artifact/projection shape as the real path -- the same
//! `WrapperToolResult` / `CapoToolResult` / `AgentReportRecord` types, the same
//! audit event sequence, schema-valid typed output (validated against the
//! tool's declared `output_schema`), and artifacts carrying a `redaction_state`
//! -- so the controller dispatch, the `ToolInvocation` / `ToolObservation`
//! projections, and replay/projection-rebuild tests run identically against a
//! fake result. The only differences from the real path are deterministic:
//! timing is fixed (`duration_ms`/`started_at`/`completed_at` are pinned), no
//! process is spawned, and no file is touched.
//!
//! The fakes run the REAL authorization phase
//! ([`RuntimeToolWrappers::authorize_tool_call`] /
//! [`CapoToolRegistry::authorize_tool_call`]) against the supplied
//! [`PermissionPolicy`], so a permission DENIAL behaves exactly as it does on
//! the real path (no handler runs, the denial audit/typed-output shape is
//! emitted). Clean and failure paths (a rejected patch hunk, a failing test
//! command, a handler error) are scripted explicitly so a test can exercise
//! both without a live binary.

use capo_core::BoundaryBinding;
use serde_json::Value;

use crate::agent_reports::{AGENT_REPORT_OUTPUT_SCHEMA, EVIDENCE_SOURCE_AGENT_REPORTED};
use crate::runtime_wrapper_types::failed_typed_output;
use crate::{
    AgentReportRecord, AgentReportRegistry, AgentReportRequest, CapoToolRegistry, CapoToolRequest,
    CapoToolResult, PermissionPolicy, RuntimeToolConfig, RuntimeToolWrappers, ToolAuditEvent,
    WrapperArtifact, WrapperToolRequest, WrapperToolResult, content_hash,
};

/// A pinned, deterministic elapsed time (milliseconds) every fake execution
/// reports, so a replayed fake result is byte-identical run to run.
pub const FAKE_DURATION_MS: i64 = 0;

/// A pinned, deterministic wall-clock timestamp (millis-since-epoch) the fake
/// `capo.test_run` reports for `started_at`/`completed_at`. Fixed (not
/// `SystemTime::now`) so the typed test/check record replays identically.
pub const FAKE_EPOCH_MILLIS: i64 = 0;

/// The deterministic outcome a [`FakeRuntimeToolWrappers`] should script for a
/// runtime-wrapper call.
///
/// The fakes cover BOTH the clean path and the failure paths the real wrappers
/// surface (ACI10): a handler error ([`Self::Failed`]), an `apply_patch` hunk
/// no strategy could locate ([`Self::NoMatch`]), and a `file_write`
/// precondition mismatch ([`Self::PreconditionFailed`]). A permission DENIAL is
/// NOT scripted here: it is produced by the real authorization phase against
/// the policy, exactly as on the real path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScriptedWrapperOutcome {
    /// The tool ran and completed a unit of work. The fake synthesizes a
    /// schema-valid typed output and a deterministic output artifact for the
    /// tool, with the supplied bytes standing in for the captured
    /// output/content. `passed` controls the `exited`-vs-`failed` observed
    /// status for execution wrappers.
    Clean {
        /// The bytes that stand in for the tool's captured output/content (the
        /// process stdout, the read file, the diff). Stored in the fake output
        /// artifact and redacted through the wrapper policy, exactly as the real
        /// path scrubs output, so a redaction test can run against a fake.
        output: Vec<u8>,
        /// Whether the underlying unit "passed" (exit 0 for an execution
        /// wrapper). A clean-but-failing command (`passed:false`) still
        /// COMPLETES the call -- it delivered a full evidence record -- mirroring
        /// the real `capo.shell_run`/`capo.test_run` semantics.
        passed: bool,
    },
    /// The handler errored (e.g. a missing input, an unsupported tool). Emits
    /// the same non-completed `failed` audit shape and schema-valid typed
    /// output as the real failure path.
    Failed { error: String },
    /// An `apply_patch` hunk that no strategy could locate: a STRUCTURED
    /// retryable no-match that wrote nothing. Emits the `no_match` terminal
    /// status (folded to `failed` for dispatch) carrying the rejected hunk
    /// index and reason, exactly like the real `no_match_execution`.
    NoMatch {
        rejected_hunk_index: usize,
        reject_reason: String,
    },
    /// A `file_write` whose expected-precondition hash did not match the
    /// on-disk content: a typed `precondition_failed` that did NOT write. Emits
    /// the same non-completed terminal shape as the real precondition guard.
    PreconditionFailed {
        expected_hash: String,
        actual_hash: String,
    },
}

impl ScriptedWrapperOutcome {
    /// A clean, passing run producing the given output bytes.
    pub fn ok(output: impl Into<Vec<u8>>) -> Self {
        Self::Clean {
            output: output.into(),
            passed: true,
        }
    }

    /// A clean run that completed but whose underlying command did not pass
    /// (non-zero exit / failing test). Still a completed call carrying evidence.
    pub fn ran_but_failed(output: impl Into<Vec<u8>>) -> Self {
        Self::Clean {
            output: output.into(),
            passed: false,
        }
    }
}

/// A deterministic, scripted fake of the runtime wrappers (ACI10).
///
/// Holds the SAME [`RuntimeToolConfig`] as the real [`RuntimeToolWrappers`] (so
/// the descriptor / schema / scope / risk metadata it reports is identical --
/// it delegates `describe_tool`/`authorize_tool_call` to a real
/// `RuntimeToolWrappers`), but its invocation never spawns a process or touches
/// disk: it returns the scripted outcome as a [`WrapperToolResult`] shaped
/// exactly like the real path's result.
///
/// This is TEST-ONLY: it is never installed in [`crate::ToolExposure`] (the real
/// controller always builds the live `RuntimeToolWrappers`), so it cannot become
/// the default for a real `Runtime` dispatch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeRuntimeToolWrappers {
    real: RuntimeToolWrappers,
}

impl FakeRuntimeToolWrappers {
    pub fn new(config: RuntimeToolConfig) -> Self {
        Self {
            real: RuntimeToolWrappers::new(config),
        }
    }

    /// The fake binding: the same `ToolExposure` boundary kind as the real
    /// wrappers, but marked `fake` so it can never masquerade as the live path.
    pub fn binding(&self) -> BoundaryBinding {
        BoundaryBinding::fake(
            capo_core::BoundaryKind::ToolExposure,
            "fake-runtime-wrappers",
        )
    }

    /// The real wrapper descriptors (schema/scope/risk/redaction), unchanged --
    /// the fake reports the SAME tool surface as the live wrappers.
    pub fn describe_tool(&self, tool_id: &str) -> Option<crate::ToolDefinition> {
        self.real.describe_tool(tool_id)
    }

    pub fn list_tools(&self) -> Vec<crate::ToolDefinition> {
        self.real.list_tools()
    }

    /// Authorize a wrapper call against the policy (the REAL authorization
    /// phase) and either deny it -- exactly as the real path does -- or emit the
    /// scripted outcome as a schema-valid [`WrapperToolResult`].
    pub fn authorize_and_invoke(
        &self,
        request: WrapperToolRequest,
        policy: &PermissionPolicy,
        outcome: ScriptedWrapperOutcome,
    ) -> WrapperToolResult {
        let authorization = self.real.authorize_tool_call(&request, policy);
        if authorization.permission.effect != "allow" {
            // The real deny audit shape: `tool.call_canceled` after the decision,
            // no handler run, schema-valid denied typed output.
            let mut events = authorization.events;
            events.push(ToolAuditEvent::new(
                "tool.call_canceled",
                "permission_denied",
            ));
            return WrapperToolResult::denied(
                request,
                authorization.definition,
                authorization.permission,
                events,
            );
        }

        let mut events = authorization.events;
        events.extend([
            ToolAuditEvent::new("capability.grant_used", "used"),
            ToolAuditEvent::new("tool.invocation_started", "running"),
        ]);
        let input_artifact = self.fake_input_artifact(&request);
        let output_schema = authorization.definition.output_schema.as_str();

        match outcome {
            ScriptedWrapperOutcome::Clean { output, passed } => {
                let artifact = self.fake_output_artifact(&request, &output);
                let observed_status = self.completed_status(&request.tool_id, passed);
                let typed_output =
                    self.clean_typed_output(&request.tool_id, &observed_status, passed, &artifact);
                events.extend([
                    ToolAuditEvent::new("tool.output_artifact_recorded", "safe"),
                    ToolAuditEvent::new("tool.output_observed", observed_status.clone()),
                    ToolAuditEvent::new("tool.call_completed", "completed"),
                    ToolAuditEvent::new("tool.result_delivered", "delivered"),
                ]);
                WrapperToolResult {
                    tool_call_id: request.tool_call_id,
                    tool_id: request.tool_id,
                    status: observed_status.clone(),
                    summary: format!("fake {observed_status} ({} byte(s))", artifact.size_bytes),
                    typed_output,
                    input_artifact: Some(input_artifact),
                    output_artifacts: vec![artifact],
                    permission_decision: authorization.permission,
                    events,
                }
            }
            ScriptedWrapperOutcome::NoMatch {
                rejected_hunk_index,
                reject_reason,
            } => {
                // A structured retryable no-match: wrote nothing, produced no
                // artifact, so it flows through the NON-completed audit shape
                // (no `tool.output_artifact_recorded`/`tool.call_completed`),
                // exactly like the real `apply_patch` no-match.
                // The optional `nearest_line`/`nearest_preview` fields are
                // OMITTED rather than set to `null` here: a fake near-miss has
                // no nearest candidate, and the declared `integer?`/`string?`
                // optionals validate by ABSENCE, not by a null value.
                let typed_output = serde_json::json!({
                    "status": "no_match",
                    "path": fake_path(&request),
                    "hunks_total": 0,
                    "hunks_applied": 0,
                    "hunks_rejected": 1,
                    "changed_line_ranges": Vec::<Value>::new(),
                    "output_artifact_id": "none",
                    "lint_status": "skipped",
                    "lint_findings": Vec::<Value>::new(),
                    "rejected_hunk_index": rejected_hunk_index as i64,
                    "reject_reason": reject_reason.clone(),
                });
                events.extend([
                    ToolAuditEvent::new("tool.output_observed", "no_match"),
                    ToolAuditEvent::new("tool.call_failed", "no_match"),
                ]);
                WrapperToolResult {
                    tool_call_id: request.tool_call_id,
                    tool_id: request.tool_id,
                    status: "no_match".to_string(),
                    summary: format!(
                        "fake apply_patch hunk {rejected_hunk_index} did not match ({reject_reason})"
                    ),
                    typed_output,
                    input_artifact: Some(input_artifact),
                    output_artifacts: Vec::new(),
                    permission_decision: authorization.permission,
                    events,
                }
            }
            ScriptedWrapperOutcome::PreconditionFailed {
                expected_hash,
                actual_hash,
            } => {
                let typed_output = serde_json::json!({
                    "status": "precondition_failed",
                    "path": fake_path(&request),
                    "mode": "precondition",
                    "before_hash": actual_hash.clone(),
                    "after_hash": actual_hash.clone(),
                    "bytes_written": 0,
                    "output_artifact_id": "none",
                    "expected_hash": expected_hash.clone(),
                    "actual_hash": actual_hash.clone(),
                });
                events.extend([
                    ToolAuditEvent::new("tool.output_observed", "precondition_failed"),
                    ToolAuditEvent::new("tool.call_failed", "precondition_failed"),
                ]);
                WrapperToolResult {
                    tool_call_id: request.tool_call_id,
                    tool_id: request.tool_id,
                    status: "precondition_failed".to_string(),
                    summary: format!(
                        "fake file_write precondition failed: expected {expected_hash} but on-disk is {actual_hash}"
                    ),
                    typed_output,
                    input_artifact: Some(input_artifact),
                    output_artifacts: Vec::new(),
                    permission_decision: authorization.permission,
                    events,
                }
            }
            ScriptedWrapperOutcome::Failed { error } => {
                events.extend([
                    ToolAuditEvent::new("tool.output_observed", "failed"),
                    ToolAuditEvent::new("tool.call_failed", "failed"),
                ]);
                let typed_output = failed_typed_output(output_schema, &error);
                WrapperToolResult {
                    tool_call_id: request.tool_call_id,
                    tool_id: request.tool_id,
                    status: "failed".to_string(),
                    summary: error,
                    typed_output,
                    input_artifact: Some(input_artifact),
                    output_artifacts: Vec::new(),
                    permission_decision: authorization.permission,
                    events,
                }
            }
        }
    }

    /// The observed terminal status for a completed fake execution.
    ///
    /// Execution wrappers (`shell_run`, the git wrappers, `test_run`) carry the
    /// runtime's `exited`/`failed` observed status (a non-zero exit is observed
    /// as `failed` but the call still COMPLETED); the read/write/patch/search
    /// wrappers always complete as `completed`.
    fn completed_status(&self, tool_id: &str, passed: bool) -> String {
        match tool_id {
            "capo.shell_run" | "capo.git_status" | "capo.git_diff" | "capo.git_commit"
            | "capo.test_run" => if passed { "exited" } else { "failed" }.to_string(),
            _ => "completed".to_string(),
        }
    }

    /// Build the schema-valid typed output for a clean fake execution, matching
    /// the per-tool `output_schema` the real wrapper declares.
    fn clean_typed_output(
        &self,
        tool_id: &str,
        observed_status: &str,
        passed: bool,
        artifact: &WrapperArtifact,
    ) -> Value {
        let artifact_id = artifact.artifact_id.as_str();
        match tool_id {
            "capo.shell_run" | "capo.git_status" | "capo.git_diff" | "capo.git_commit" => {
                serde_json::json!({
                    "status": observed_status,
                    "exit_status": if passed { 0 } else { 1 },
                    "passed": passed,
                    "duration_ms": FAKE_DURATION_MS,
                    "output_artifact_id": artifact_id,
                    "truncated": false,
                })
            }
            "capo.file_write" => serde_json::json!({
                "status": "completed",
                "path": "fake/path",
                "mode": "overwrite",
                "before_hash": "fnv1a64:0000000000000000",
                "after_hash": artifact.content_hash,
                "bytes_written": artifact.size_bytes,
                "output_artifact_id": artifact_id,
            }),
            "capo.apply_patch" => serde_json::json!({
                "status": "completed",
                "path": "fake/path",
                "hunks_total": 1,
                "hunks_applied": 1,
                "hunks_rejected": 0,
                "changed_line_ranges": ["1-1"],
                "output_artifact_id": artifact_id,
                "lint_status": "passed",
                "lint_findings": Vec::<Value>::new(),
            }),
            "capo.search" => serde_json::json!({
                "status": "completed",
                "query": "fake-query",
                "matches": Vec::<Value>::new(),
                "returned_matches": 0,
                "total_matches": 0,
                "truncated": false,
                "truncation_reason": "none",
                "duration_ms": FAKE_DURATION_MS,
            }),
            "capo.test_run" => serde_json::json!({
                "status": observed_status,
                "command": "fake-command",
                "exit_status": if passed { 0 } else { 1 },
                "passed": passed,
                "failing_items": if passed {
                    Vec::<Value>::new()
                } else {
                    vec![Value::String("fake::failing_test".to_string())]
                },
                "failing_items_total": if passed { 0 } else { 1 },
                "failing_items_truncated": false,
                "duration_ms": FAKE_DURATION_MS,
                "started_at": FAKE_EPOCH_MILLIS,
                "completed_at": FAKE_EPOCH_MILLIS,
                "output_artifact_id": artifact_id,
            }),
            // file_read and the read-only workpad/project-memory aliases.
            _ => serde_json::json!({
                "status": "completed",
                "path": "fake/path",
                "bytes_read": artifact.size_bytes,
                "content_hash": artifact.content_hash,
                "output_artifact_id": artifact_id,
            }),
        }
    }

    /// A deterministic fake INPUT artifact: the request input, redacted through
    /// the real wrapper policy so input redaction is exercised on the fake path.
    fn fake_input_artifact(&self, request: &WrapperToolRequest) -> WrapperArtifact {
        let payload = format!(
            "{{\"tool_id\":\"{}\",\"input\":{}}}",
            request.tool_id, request.input
        );
        self.redacted_artifact(request, "input", "fake wrapper input", payload.as_bytes())
    }

    /// A deterministic fake OUTPUT artifact carrying the scripted output bytes,
    /// redacted through the real wrapper policy so OUTPUT redaction is exercised
    /// on the fake path (ACI7 contract holds for fakes too).
    fn fake_output_artifact(&self, request: &WrapperToolRequest, output: &[u8]) -> WrapperArtifact {
        self.redacted_artifact(request, "fake_output", "fake wrapper output", output)
    }

    /// Build a fake artifact without touching disk: redact the bytes through the
    /// SAME policy the real wrappers use (so a secret in fake output is scrubbed
    /// and the recorded `redaction_state` is honest), hash the redacted bytes,
    /// and synthesize the same artifact-id shape the real path uses. The `uri`
    /// is a `fake://` URI rather than an on-disk path -- nothing is written.
    fn redacted_artifact(
        &self,
        request: &WrapperToolRequest,
        kind: &str,
        summary: &str,
        bytes: &[u8],
    ) -> WrapperArtifact {
        let (redacted, redaction_state) = self.real.redact_bytes_with_state_for_fake(bytes);
        WrapperArtifact {
            artifact_id: format!("artifact-wrapper-{}-{kind}", request.tool_call_id),
            kind: kind.to_string(),
            uri: format!("fake://{}/{kind}.txt", request.tool_call_id),
            content_hash: content_hash(&redacted),
            size_bytes: redacted.len() as i64,
            redaction_state,
            summary: summary.to_string(),
        }
    }
}

/// The fake path for a scripted no-match / precondition outcome: the request's
/// declared `path` input if present, else a stable placeholder. Keeps the typed
/// output schema-valid without resolving a real workspace path.
fn fake_path(request: &WrapperToolRequest) -> String {
    request
        .input
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or("fake/path")
        .to_string()
}

/// A deterministic, scripted fake of the Capo-owned tool registry (ACI10).
///
/// The real [`CapoToolRegistry`] is already deterministic (it renders from an
/// in-memory context), but it pulls its rendered output from a caller-supplied
/// [`crate::CapoToolContext`]. This fake makes the scripted contract explicit
/// for replay tests: it runs the real authorization phase against the policy
/// (so denial behaves identically) and returns a stable [`CapoToolResult`] with
/// the same audit event sequence and the same `output`/`output_artifact_id`
/// shape, without depending on the controller assembling a live context.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeCapoToolRegistry;

impl FakeCapoToolRegistry {
    pub fn binding(&self) -> BoundaryBinding {
        BoundaryBinding::fake(capo_core::BoundaryKind::ToolExposure, "fake-capo-registry")
    }

    /// Authorize a Capo tool call against the policy and return a scripted
    /// [`CapoToolResult`]: a stable `output` string on the clean path, or the
    /// real denial shape when the policy denies the call.
    pub fn authorize_and_invoke(
        &self,
        request: CapoToolRequest,
        policy: &PermissionPolicy,
        scripted_output: &str,
    ) -> CapoToolResult {
        let real = CapoToolRegistry;
        let authorization = real.authorize_tool_call(&request, policy);
        if authorization.permission.effect != "allow" {
            let mut events = authorization.events;
            events.push(ToolAuditEvent::new(
                "tool.call_canceled",
                "permission_denied",
            ));
            return CapoToolResult {
                tool_call_id: request.tool_call_id,
                tool_id: request.tool_id,
                output: authorization.permission.explanation.clone(),
                output_artifact_id: "none".to_string(),
                mutates_state: authorization.definition.mutates_state,
                permission_decision: authorization.permission,
                events,
            };
        }
        let output_artifact_id = format!(
            "artifact-{}-{}",
            request.tool_call_id,
            request.tool_id.replace('.', "-")
        );
        let mut events = authorization.events;
        events.extend([
            ToolAuditEvent::new("capability.grant_used", "used"),
            ToolAuditEvent::new("tool.invocation_started", "running"),
            ToolAuditEvent::new("tool.output_artifact_recorded", "safe"),
            ToolAuditEvent::new("tool.output_observed", "observed"),
            ToolAuditEvent::new("tool.call_completed", "completed"),
            ToolAuditEvent::new("tool.result_delivered", "delivered"),
        ]);
        CapoToolResult {
            tool_call_id: request.tool_call_id,
            tool_id: request.tool_id,
            output: scripted_output.to_string(),
            output_artifact_id,
            mutates_state: authorization.definition.mutates_state,
            permission_decision: authorization.permission,
            events,
        }
    }
}

/// A deterministic, scripted fake of the `GO2` agent-reporting surface (ACI10).
///
/// The real [`AgentReportRegistry`] is ALREADY fully deterministic and replayable
/// (no process/disk/clock dependency), and [`crate::AgentReportLedger`] is the
/// replayable fake ledger that dedupes by idempotency key. This thin wrapper
/// exists for SYMMETRY with the other fakes: a single fake surface a replay test
/// can drive for every tool class, delegating to the real (already-deterministic)
/// report emission so the `agent_reported` classification, confidence, and
/// idempotency key are identical to the real path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeAgentReportRegistry {
    real: AgentReportRegistry,
}

impl FakeAgentReportRegistry {
    pub fn new() -> Self {
        Self {
            real: AgentReportRegistry,
        }
    }

    pub fn binding(&self) -> BoundaryBinding {
        BoundaryBinding::fake(
            capo_core::BoundaryKind::ToolExposure,
            "fake-capo-agent-reports",
        )
    }

    /// Emit an agent report deterministically. Delegates to the real registry
    /// (which is already deterministic) so the persisted-shape `AgentReportRecord`
    /// -- `source=agent_reported`, confidence, idempotency key, accepted flag --
    /// is identical to the real path.
    pub fn authorize_and_invoke(
        &self,
        request: AgentReportRequest,
        policy: &PermissionPolicy,
    ) -> AgentReportRecord {
        let record = self.real.authorize_and_invoke(request, policy);
        // Invariant the fake upholds: a report is ALWAYS a claim, never observed
        // evidence, and the output schema constants stay coherent. These are
        // asserted at construction so a future divergence is caught immediately.
        debug_assert_eq!(record.source, EVIDENCE_SOURCE_AGENT_REPORTED);
        debug_assert!(AGENT_REPORT_OUTPUT_SCHEMA.contains("idempotency_key"));
        record
    }
}

impl Default for FakeAgentReportRegistry {
    fn default() -> Self {
        Self::new()
    }
}
