use std::{error as std_error, fmt};

use capo_core::{
    AgentId, EvidenceId, MemoryPacketId, ProjectId, RunId, SessionId, TaskId, ToolCallId,
};
use serde_json::Value;

use crate::codec_adapter::decode_adapter_projection;
use crate::{
    AgentProjection, CapabilityGrantProjection, ConnectivityExposureProjection, EvidenceProjection,
    MemoryPacketProjection, MemoryRecordProjection, MemorySourceProjection,
    PermissionApprovalProjection, ProjectProjection, ProjectionRecord, ReviewFindingProjection,
    RunProjection, RuntimeTargetProjection, SessionProjection, SourceBindingProjection, StateError,
    StateResult, TaskOutcomeReportProjection, TaskProjection, ToolCallProjection,
    ToolCallProvenance, ToolObservationProjection, WorkpadFileProjection,
    WorkpadIndexResetProjection, WorkpadTaskProjection, optional_id,
};

#[derive(Debug)]
pub(crate) struct ProjectionDecodeError(pub(crate) String);

impl fmt::Display for ProjectionDecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std_error::Error for ProjectionDecodeError {}

#[allow(clippy::too_many_arguments)]
pub(crate) fn projection_record_from_row(
    projection_kind: String,
    record_id: String,
    a: Option<String>,
    b: Option<String>,
    c: Option<String>,
    d: Option<String>,
    e: Option<String>,
    f: Option<String>,
    g: Option<String>,
    h: Option<String>,
    payload_json: String,
) -> Result<ProjectionRecord, ProjectionDecodeError> {
    match projection_kind.as_str() {
        "project" => Ok(ProjectionRecord::Project(ProjectProjection {
            project_id: ProjectId::new(record_id),
            name: required_field(&projection_kind, "project", a, "name")?,
            status: required_field(&projection_kind, "project", b, "status")?,
            updated_sequence: 0,
        })),
        "task" => Ok(ProjectionRecord::Task(TaskProjection {
            task_id: TaskId::new(record_id),
            project_id: ProjectId::new(required_field(&projection_kind, "task", a, "project_id")?),
            title: required_field(&projection_kind, "task", b, "title")?,
            capo_execution_status: required_field(
                &projection_kind,
                "task",
                c,
                "capo_execution_status",
            )?,
            active_session_id: optional_id(d),
            latest_summary: e,
            evidence_id: optional_id(f),
            updated_sequence: 0,
        })),
        "agent" => Ok(ProjectionRecord::Agent(AgentProjection {
            agent_id: AgentId::new(record_id),
            project_id: ProjectId::new(required_field(&projection_kind, "agent", a, "project_id")?),
            name: required_field(&projection_kind, "agent", b, "name")?,
            status: required_field(&projection_kind, "agent", c, "status")?,
            current_session_id: optional_id(d),
            updated_sequence: 0,
        })),
        "session" => {
            let payload = parse_projection_payload(&projection_kind, &record_id, &payload_json)?;
            Ok(ProjectionRecord::Session(SessionProjection {
                session_id: SessionId::new(record_id),
                project_id: ProjectId::new(required_field(
                    &projection_kind,
                    "session",
                    a,
                    "project_id",
                )?),
                task_id: optional_id(b),
                agent_id: AgentId::new(required_field(&projection_kind, "session", c, "agent_id")?),
                title: required_field(&projection_kind, "session", d, "title")?,
                status: required_field(&projection_kind, "session", e, "status")?,
                current_goal: required_field(&projection_kind, "session", f, "current_goal")?,
                latest_summary: g,
                latest_confidence: optional_i64(
                    &projection_kind,
                    "session",
                    h,
                    "latest_confidence",
                )?,
                latest_blocker: None,
                external_session_ref: payload_string(&payload, "external_session_ref"),
                updated_sequence: 0,
            }))
        }
        "run" => Ok(ProjectionRecord::Run(RunProjection {
            run_id: RunId::new(record_id),
            session_id: SessionId::new(required_field(&projection_kind, "run", a, "session_id")?),
            status: required_field(&projection_kind, "run", b, "status")?,
            recovery_of_run_id: optional_id(c),
            updated_sequence: 0,
        })),
        "capability_grant" => {
            // SG3: the grant lifecycle timestamps (created_at/expires_at/
            // revoked_at) ride in the payload -- positional slots a..g are taken
            // by the grant body -- so a replay rebuilds revoked/expired state
            // identically from the event log.
            let payload = parse_projection_payload(&projection_kind, &record_id, &payload_json)?;
            Ok(ProjectionRecord::CapabilityGrant(
                CapabilityGrantProjection {
                    capability_grant_id: record_id,
                    capability_profile_id: required_field(
                        &projection_kind,
                        "capability_grant",
                        a,
                        "capability_profile_id",
                    )?,
                    scope_json: required_field(
                        &projection_kind,
                        "capability_grant",
                        b,
                        "scope_json",
                    )?,
                    effect: required_field(&projection_kind, "capability_grant", c, "effect")?,
                    subject_json: required_field(
                        &projection_kind,
                        "capability_grant",
                        d,
                        "subject_json",
                    )?,
                    decision_source: e.unwrap_or_else(|| "unknown".to_string()),
                    persistence: f.unwrap_or_else(|| "unknown".to_string()),
                    explanation: g.unwrap_or_default(),
                    created_at: payload_optional_string(&payload, "created_at"),
                    expires_at: payload_optional_string(&payload, "expires_at"),
                    revoked_at: payload_optional_string(&payload, "revoked_at"),
                    updated_sequence: 0,
                },
            ))
        }
        "permission_approval" => {
            let payload = parse_projection_payload(&projection_kind, &record_id, &payload_json)?;
            Ok(ProjectionRecord::PermissionApproval(
                PermissionApprovalProjection {
                    approval_id: record_id,
                    project_id: ProjectId::new(required_field(
                        &projection_kind,
                        "permission_approval",
                        a,
                        "project_id",
                    )?),
                    session_id: optional_id(b),
                    tool_call_id: optional_id(c),
                    capability_profile_id: required_field(
                        &projection_kind,
                        "permission_approval",
                        d,
                        "capability_profile_id",
                    )?,
                    scope_json: required_field(
                        &projection_kind,
                        "permission_approval",
                        h,
                        "scope_json",
                    )?,
                    subject_json: payload_string(&payload, "subject_json")
                        .unwrap_or_else(|| "{}".to_string()),
                    status: required_field(&projection_kind, "permission_approval", e, "status")?,
                    requested_by: payload_string(&payload, "requested_by")
                        .unwrap_or_else(|| "unknown".to_string()),
                    reason: payload_string(&payload, "reason").unwrap_or_default(),
                    decision: f,
                    capability_grant_id: g,
                    updated_sequence: 0,
                },
            ))
        }
        "connectivity_exposure" => {
            let payload = parse_projection_payload(&projection_kind, &record_id, &payload_json)?;
            Ok(ProjectionRecord::ConnectivityExposure(
                ConnectivityExposureProjection {
                    exposure_id: record_id.clone(),
                    project_id: ProjectId::new(required_field(
                        &projection_kind,
                        "connectivity_exposure",
                        a,
                        "project_id",
                    )?),
                    connectivity_endpoint_id: required_field(
                        &projection_kind,
                        "connectivity_exposure",
                        b,
                        "connectivity_endpoint_id",
                    )?,
                    owner_kind: required_payload_string(
                        &projection_kind,
                        "connectivity_exposure",
                        &payload,
                        "owner_kind",
                    )?,
                    owner_id: required_payload_string(
                        &projection_kind,
                        "connectivity_exposure",
                        &payload,
                        "owner_id",
                    )?,
                    channel_kind: required_payload_string(
                        &projection_kind,
                        "connectivity_exposure",
                        &payload,
                        "channel_kind",
                    )?,
                    exposure: required_payload_string(
                        &projection_kind,
                        "connectivity_exposure",
                        &payload,
                        "exposure",
                    )?,
                    permission_scope: required_field(
                        &projection_kind,
                        "connectivity_exposure",
                        h,
                        "permission_scope",
                    )?,
                    status: required_field(&projection_kind, "connectivity_exposure", c, "status")?,
                    capability_grant_id: d,
                    health_status: required_field(
                        &projection_kind,
                        "connectivity_exposure",
                        e,
                        "health_status",
                    )?,
                    reachable: required_field(
                        &projection_kind,
                        "connectivity_exposure",
                        f,
                        "reachable",
                    )?
                    .parse::<bool>()
                    .map_err(|error| {
                        ProjectionDecodeError(format!(
                            "invalid bool for connectivity_exposure {record_id} reachable: {error}"
                        ))
                    })?,
                    revoked_at: g,
                    updated_sequence: 0,
                },
            ))
        }
        "runtime_target" => {
            let payload = parse_projection_payload(&projection_kind, &record_id, &payload_json)?;
            Ok(ProjectionRecord::RuntimeTarget(RuntimeTargetProjection {
                runtime_target_id: record_id,
                project_id: ProjectId::new(required_field(
                    &projection_kind,
                    "runtime_target",
                    a,
                    "project_id",
                )?),
                name: required_field(&projection_kind, "runtime_target", b, "name")?,
                runner_kind: required_field(&projection_kind, "runtime_target", c, "runner_kind")?,
                workspace_root: required_field(
                    &projection_kind,
                    "runtime_target",
                    g,
                    "workspace_root",
                )?,
                artifact_root: required_field(
                    &projection_kind,
                    "runtime_target",
                    h,
                    "artifact_root",
                )?,
                default_cwd: required_payload_string(
                    &projection_kind,
                    "runtime_target",
                    &payload,
                    "default_cwd",
                )?,
                capability_profile_id: required_field(
                    &projection_kind,
                    "runtime_target",
                    f,
                    "capability_profile_id",
                )?,
                connectivity_endpoint_id: e,
                status: required_field(&projection_kind, "runtime_target", d, "status")?,
                updated_sequence: 0,
            }))
        }
        kind if kind.starts_with("adapter_") => decode_adapter_projection(
            projection_kind,
            record_id,
            a,
            b,
            c,
            d,
            e,
            f,
            g,
            h,
            payload_json,
        ),
        "tool_call" => {
            // ACI7: the per-call provenance + timing rides in the payload; parse
            // it back so a replay rebuilds the queryable chain identically.
            let payload = parse_projection_payload(&projection_kind, &record_id, &payload_json)?;
            let provenance = ToolCallProvenance {
                correlation_id: payload_optional_string(&payload, "correlation_id"),
                permission_decision_id: payload_optional_string(&payload, "permission_decision_id"),
                capability_grant_use_id: payload_optional_string(
                    &payload,
                    "capability_grant_use_id",
                ),
                started_at: payload_optional_i64(
                    &projection_kind,
                    &record_id,
                    &payload,
                    "started_at",
                )?,
                completed_at: payload_optional_i64(
                    &projection_kind,
                    &record_id,
                    &payload,
                    "completed_at",
                )?,
            };
            Ok(ProjectionRecord::ToolCall(ToolCallProjection {
                tool_call_id: ToolCallId::new(record_id),
                session_id: SessionId::new(required_field(
                    &projection_kind,
                    "tool_call",
                    a,
                    "session_id",
                )?),
                turn_id: b,
                tool_name: required_field(&projection_kind, "tool_call", c, "tool_name")?,
                tool_origin: required_field(&projection_kind, "tool_call", d, "tool_origin")?,
                status: required_field(&projection_kind, "tool_call", e, "status")?,
                input_artifact_id: f,
                output_artifact_id: g,
                provenance,
                updated_sequence: 0,
            }))
        }
        "tool_observation" => {
            let payload = parse_projection_payload(&projection_kind, &record_id, &payload_json)?;
            Ok(ProjectionRecord::ToolObservation(
                ToolObservationProjection {
                    tool_observation_id: record_id,
                    session_id: SessionId::new(required_field(
                        &projection_kind,
                        "tool_observation",
                        a,
                        "session_id",
                    )?),
                    tool_call_id: optional_id(b),
                    source: required_field(&projection_kind, "tool_observation", c, "source")?,
                    external_tool_ref: d,
                    tool_name: required_field(
                        &projection_kind,
                        "tool_observation",
                        e,
                        "tool_name",
                    )?,
                    observed_status: required_field(
                        &projection_kind,
                        "tool_observation",
                        f,
                        "observed_status",
                    )?,
                    instrumentation_level: required_field(
                        &projection_kind,
                        "tool_observation",
                        g,
                        "instrumentation_level",
                    )?,
                    confidence: required_field(
                        &projection_kind,
                        "tool_observation",
                        h,
                        "confidence",
                    )?,
                    raw_event_hash: required_payload_string(
                        &projection_kind,
                        "tool_observation",
                        &payload,
                        "raw_event_hash",
                    )?,
                    artifact_id: payload_optional_string(&payload, "artifact_id"),
                    updated_sequence: 0,
                },
            ))
        }
        "memory_packet" => Ok(ProjectionRecord::MemoryPacketRef(MemoryPacketProjection {
            memory_packet_id: MemoryPacketId::new(record_id),
            project_id: ProjectId::new(required_field(
                &projection_kind,
                "memory_packet",
                a,
                "project_id",
            )?),
            task_id: optional_id(b),
            agent_id: optional_id(c),
            session_id: optional_id(d),
            run_id: optional_id(e),
            turn_id: f,
            packet_artifact_id: g,
            purpose: required_field(&projection_kind, "memory_packet", h, "purpose")?,
            updated_sequence: 0,
        })),
        "memory_record" => {
            let payload = parse_projection_payload(&projection_kind, &record_id, &payload_json)?;
            Ok(ProjectionRecord::MemoryRecord(Box::new(
                MemoryRecordProjection {
                    memory_record_id: record_id,
                    project_id: ProjectId::new(required_field(
                        &projection_kind,
                        "memory_record",
                        a,
                        "project_id",
                    )?),
                    scope: required_field(&projection_kind, "memory_record", b, "scope")?,
                    scope_owner_ref: required_field(
                        &projection_kind,
                        "memory_record",
                        c,
                        "scope_owner_ref",
                    )?,
                    subject_ref: d,
                    sensitivity_classification: required_field(
                        &projection_kind,
                        "memory_record",
                        e,
                        "sensitivity_classification",
                    )?,
                    record_kind: required_field(
                        &projection_kind,
                        "memory_record",
                        f,
                        "record_kind",
                    )?,
                    review_state: required_field(
                        &projection_kind,
                        "memory_record",
                        g,
                        "review_state",
                    )?,
                    source_count: required_i64(
                        &projection_kind,
                        "memory_record",
                        h,
                        "source_count",
                    )?,
                    subject: required_payload_string(
                        &projection_kind,
                        "memory_record",
                        &payload,
                        "subject",
                    )?,
                    predicate: required_payload_string(
                        &projection_kind,
                        "memory_record",
                        &payload,
                        "predicate",
                    )?,
                    object: required_payload_string(
                        &projection_kind,
                        "memory_record",
                        &payload,
                        "object",
                    )?,
                    body: required_payload_string(
                        &projection_kind,
                        "memory_record",
                        &payload,
                        "body",
                    )?,
                    confidence: required_payload_string(
                        &projection_kind,
                        "memory_record",
                        &payload,
                        "confidence",
                    )?,
                    valid_from: payload_optional_string(&payload, "valid_from"),
                    valid_until: payload_optional_string(&payload, "valid_until"),
                    supersedes_memory_record_id: payload_optional_string(
                        &payload,
                        "supersedes_memory_record_id",
                    ),
                    revoked_by_memory_record_id: payload_optional_string(
                        &payload,
                        "revoked_by_memory_record_id",
                    ),
                    redaction_state: required_payload_string(
                        &projection_kind,
                        "memory_record",
                        &payload,
                        "redaction_state",
                    )?,
                    invalidated_at: payload_optional_string(&payload, "invalidated_at"),
                    invalidation_reason: payload_optional_string(&payload, "invalidation_reason"),
                    packet_item_ref: payload_optional_string(&payload, "packet_item_ref"),
                    updated_sequence: 0,
                },
            )))
        }
        "memory_source" => {
            let payload = parse_projection_payload(&projection_kind, &record_id, &payload_json)?;
            Ok(ProjectionRecord::MemorySource(MemorySourceProjection {
                memory_source_id: record_id,
                memory_record_id: required_field(
                    &projection_kind,
                    "memory_source",
                    a,
                    "memory_record_id",
                )?,
                source_kind: required_field(&projection_kind, "memory_source", b, "source_kind")?,
                source_event_id: c,
                source_artifact_id: d,
                source_path: e,
                source_anchor: f,
                source_content_hash: g,
                source_sequence: optional_i64(
                    &projection_kind,
                    "memory_source",
                    h,
                    "source_sequence",
                )?,
                quote_artifact_id: payload_optional_string(&payload, "quote_artifact_id"),
                observed_at: payload_optional_string(&payload, "observed_at"),
                updated_sequence: 0,
            }))
        }
        "evidence" => Ok(ProjectionRecord::Evidence(EvidenceProjection {
            evidence_id: EvidenceId::new(record_id),
            project_id: ProjectId::new(required_field(
                &projection_kind,
                "evidence",
                a,
                "project_id",
            )?),
            task_id: optional_id(b),
            session_id: optional_id(c),
            run_id: optional_id(d),
            kind: required_field(&projection_kind, "evidence", e, "kind")?,
            artifact_id: f,
            confidence: required_i64(&projection_kind, "evidence", g, "confidence")?,
            updated_sequence: 0,
        })),
        "task_outcome_report" => {
            let payload = parse_projection_payload(&projection_kind, &record_id, &payload_json)?;
            Ok(ProjectionRecord::TaskOutcomeReport(
                TaskOutcomeReportProjection {
                    task_outcome_report_id: record_id,
                    project_id: ProjectId::new(required_field(
                        &projection_kind,
                        "task_outcome_report",
                        a,
                        "project_id",
                    )?),
                    task_id: TaskId::new(required_field(
                        &projection_kind,
                        "task_outcome_report",
                        b,
                        "task_id",
                    )?),
                    session_id: SessionId::new(required_field(
                        &projection_kind,
                        "task_outcome_report",
                        c,
                        "session_id",
                    )?),
                    run_id: RunId::new(required_field(
                        &projection_kind,
                        "task_outcome_report",
                        d,
                        "run_id",
                    )?),
                    outcome_status: required_field(
                        &projection_kind,
                        "task_outcome_report",
                        e,
                        "outcome_status",
                    )?,
                    started_sequence: required_i64(
                        &projection_kind,
                        "task_outcome_report",
                        f,
                        "started_sequence",
                    )?,
                    completed_sequence: required_i64(
                        &projection_kind,
                        "task_outcome_report",
                        g,
                        "completed_sequence",
                    )?,
                    duration_sequence_span: required_i64(
                        &projection_kind,
                        "task_outcome_report",
                        h,
                        "duration_sequence_span",
                    )?,
                    action_count: required_payload_i64(
                        &projection_kind,
                        "task_outcome_report",
                        &payload,
                        "action_count",
                    )?,
                    tool_call_count: required_payload_i64(
                        &projection_kind,
                        "task_outcome_report",
                        &payload,
                        "tool_call_count",
                    )?,
                    evidence_count: required_payload_i64(
                        &projection_kind,
                        "task_outcome_report",
                        &payload,
                        "evidence_count",
                    )?,
                    memory_packet_count: required_payload_i64(
                        &projection_kind,
                        "task_outcome_report",
                        &payload,
                        "memory_packet_count",
                    )?,
                    confidence: payload_optional_i64(
                        &projection_kind,
                        "task_outcome_report",
                        &payload,
                        "confidence",
                    )?,
                    blocker: payload_optional_string(&payload, "blocker"),
                    review_outcome: required_payload_string(
                        &projection_kind,
                        "task_outcome_report",
                        &payload,
                        "review_outcome",
                    )?,
                    report_artifact_id: payload_optional_string(&payload, "report_artifact_id"),
                    updated_sequence: 0,
                },
            ))
        }
        "review_finding" => {
            let payload = parse_projection_payload(&projection_kind, &record_id, &payload_json)?;
            Ok(ProjectionRecord::ReviewFinding(ReviewFindingProjection {
                review_finding_id: record_id,
                project_id: ProjectId::new(required_field(
                    &projection_kind,
                    "review_finding",
                    a,
                    "project_id",
                )?),
                task_id: TaskId::new(required_field(
                    &projection_kind,
                    "review_finding",
                    b,
                    "task_id",
                )?),
                session_id: SessionId::new(required_field(
                    &projection_kind,
                    "review_finding",
                    c,
                    "session_id",
                )?),
                run_id: optional_id(d),
                tool_call_id: optional_id(e),
                workpad_task_id: f,
                finding_kind: required_field(
                    &projection_kind,
                    "review_finding",
                    g,
                    "finding_kind",
                )?,
                status: required_field(&projection_kind, "review_finding", h, "status")?,
                reviewer: required_payload_string(
                    &projection_kind,
                    "review_finding",
                    &payload,
                    "reviewer",
                )?,
                severity: required_payload_string(
                    &projection_kind,
                    "review_finding",
                    &payload,
                    "severity",
                )?,
                summary: required_payload_string(
                    &projection_kind,
                    "review_finding",
                    &payload,
                    "summary",
                )?,
                evidence_artifact_id: payload_optional_string(&payload, "evidence_artifact_id"),
                follow_up: payload_optional_string(&payload, "follow_up"),
                updated_sequence: 0,
            }))
        }
        "workpad_index_reset" => Ok(ProjectionRecord::WorkpadIndexReset(
            WorkpadIndexResetProjection {
                project_id: ProjectId::new(record_id),
                observed_unix: required_i64(
                    &projection_kind,
                    "workpad_index_reset",
                    a,
                    "observed_unix",
                )?,
                updated_sequence: 0,
            },
        )),
        "source_binding" => Ok(ProjectionRecord::SourceBinding(SourceBindingProjection {
            source_binding_id: record_id,
            project_id: ProjectId::new(required_field(
                &projection_kind,
                "source_binding",
                a,
                "project_id",
            )?),
            task_id: TaskId::new(required_field(
                &projection_kind,
                "source_binding",
                b,
                "task_id",
            )?),
            source_kind: required_field(&projection_kind, "source_binding", c, "source_kind")?,
            source_task_id: required_field(
                &projection_kind,
                "source_binding",
                d,
                "source_task_id",
            )?,
            source_path: required_field(&projection_kind, "source_binding", e, "source_path")?,
            source_anchor: required_field(&projection_kind, "source_binding", f, "source_anchor")?,
            source_hash: required_field(&projection_kind, "source_binding", g, "source_hash")?,
            binding_status: required_field(
                &projection_kind,
                "source_binding",
                h,
                "binding_status",
            )?,
            updated_sequence: 0,
        })),
        "workpad_file" => Ok(ProjectionRecord::WorkpadFile(WorkpadFileProjection {
            path: record_id,
            project_id: ProjectId::new(required_field(
                &projection_kind,
                "workpad_file",
                a,
                "project_id",
            )?),
            content_hash: required_field(&projection_kind, "workpad_file", b, "content_hash")?,
            headings: required_field(&projection_kind, "workpad_file", c, "headings")?,
            objective: d,
            observed_unix: required_i64(&projection_kind, "workpad_file", e, "observed_unix")?,
            updated_sequence: 0,
        })),
        "workpad_task" => Ok(ProjectionRecord::WorkpadTask(WorkpadTaskProjection {
            workpad_task_id: record_id,
            project_id: ProjectId::new(required_field(
                &projection_kind,
                "workpad_task",
                a,
                "project_id",
            )?),
            path: required_field(&projection_kind, "workpad_task", b, "path")?,
            source_anchor: required_field(&projection_kind, "workpad_task", c, "source_anchor")?,
            title: required_field(&projection_kind, "workpad_task", d, "title")?,
            observed_status: required_field(
                &projection_kind,
                "workpad_task",
                e,
                "observed_status",
            )?,
            capo_execution_status: required_field(
                &projection_kind,
                "workpad_task",
                f,
                "capo_execution_status",
            )?,
            observed_unix: required_i64(&projection_kind, "workpad_task", g, "observed_unix")?,
            updated_sequence: 0,
        })),
        other => Err(ProjectionDecodeError(format!(
            "unknown projection kind: {other}"
        ))),
    }
}

pub(crate) fn required_field(
    projection_kind: &str,
    record_id: &str,
    value: Option<String>,
    field: &str,
) -> Result<String, ProjectionDecodeError> {
    value.ok_or_else(|| {
        ProjectionDecodeError(format!("{projection_kind}.{record_id} missing {field}"))
    })
}

pub(crate) fn parse_projection_payload(
    projection_kind: &str,
    record_id: &str,
    payload_json: &str,
) -> Result<Value, ProjectionDecodeError> {
    serde_json::from_str(payload_json).map_err(|error| {
        ProjectionDecodeError(format!(
            "{projection_kind}.{record_id} invalid payload_json: {error}"
        ))
    })
}

pub(crate) fn payload_string(payload: &Value, key: &str) -> Option<String> {
    match payload.get(key)? {
        Value::Null => None,
        Value::String(value) => Some(value.clone()),
        value => Some(value.to_string()),
    }
}

pub(crate) fn payload_optional_string(payload: &Value, key: &str) -> Option<String> {
    payload_string(payload, key)
}

pub(crate) fn required_payload_string(
    projection_kind: &str,
    record_id: &str,
    payload: &Value,
    key: &str,
) -> Result<String, ProjectionDecodeError> {
    payload_string(payload, key).ok_or_else(|| {
        ProjectionDecodeError(format!("{projection_kind}.{record_id} missing {key}"))
    })
}

pub(crate) fn required_payload_i64(
    projection_kind: &str,
    record_id: &str,
    payload: &Value,
    key: &str,
) -> Result<i64, ProjectionDecodeError> {
    payload_optional_i64(projection_kind, record_id, payload, key)?.ok_or_else(|| {
        ProjectionDecodeError(format!("{projection_kind}.{record_id} missing {key}"))
    })
}

pub(crate) fn payload_optional_i64(
    projection_kind: &str,
    record_id: &str,
    payload: &Value,
    key: &str,
) -> Result<Option<i64>, ProjectionDecodeError> {
    match payload.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(value)) => value.as_i64().map(Some).ok_or_else(|| {
            ProjectionDecodeError(format!(
                "{projection_kind}.{record_id} invalid {key}: not an i64"
            ))
        }),
        Some(Value::String(value)) => value.parse::<i64>().map(Some).map_err(|error| {
            ProjectionDecodeError(format!(
                "{projection_kind}.{record_id} invalid {key}: {error}"
            ))
        }),
        Some(_) => Err(ProjectionDecodeError(format!(
            "{projection_kind}.{record_id} invalid {key}: not a number"
        ))),
    }
}

pub(crate) fn validate_projection_json(
    kind: &'static str,
    id: &str,
    field: &'static str,
    value: &str,
) -> StateResult<()> {
    serde_json::from_str::<Value>(value)
        .map(|_| ())
        .map_err(|error| StateError::InvalidProjectionJson {
            kind,
            id: id.to_string(),
            field,
            error: error.to_string(),
        })
}

fn optional_i64(
    projection_kind: &str,
    record_id: &str,
    value: Option<String>,
    field: &str,
) -> Result<Option<i64>, ProjectionDecodeError> {
    value
        .map(|value| {
            value.parse::<i64>().map_err(|error| {
                ProjectionDecodeError(format!(
                    "{projection_kind}.{record_id} invalid {field}: {error}"
                ))
            })
        })
        .transpose()
}

fn required_i64(
    projection_kind: &str,
    record_id: &str,
    value: Option<String>,
    field: &str,
) -> Result<i64, ProjectionDecodeError> {
    let value = required_field(projection_kind, record_id, value, field)?;
    value.parse::<i64>().map_err(|error| {
        ProjectionDecodeError(format!(
            "{projection_kind}.{record_id} invalid {field}: {error}"
        ))
    })
}
