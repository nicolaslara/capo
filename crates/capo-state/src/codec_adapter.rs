use capo_core::{AgentId, ProjectId, RunId, SessionId};

use crate::codec::{
    ProjectionDecodeError, parse_projection_payload, payload_optional_i64, payload_optional_string,
    required_field, required_payload_i64, required_payload_string,
};
use crate::{
    AdapterDispatchExecutionProjection, AdapterDispatchExecutionRequestProjection,
    AdapterDispatchGateProjection, AdapterDispatchPlanProjection,
    AdapterDispatchPromptMaterializationProjection, AdapterDispatchPromptSourceProjection,
    AdapterDispatchReplayProjection, AdapterRawUpdateProjection, AdapterReadinessProjection,
    AdapterReplayBatchProjection, AdapterSmokeReportProjection, AdapterTimelineKeyProjection,
    ProjectionRecord,
};

#[allow(clippy::too_many_arguments)]
pub(crate) fn decode_adapter_projection(
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
        "adapter_readiness" => {
            let payload = parse_projection_payload(&projection_kind, &record_id, &payload_json)?;
            Ok(ProjectionRecord::AdapterReadiness(
                AdapterReadinessProjection {
                    adapter_kind: record_id,
                    project_id: ProjectId::new(required_field(
                        &projection_kind,
                        "adapter_readiness",
                        a,
                        "project_id",
                    )?),
                    program: required_field(&projection_kind, "adapter_readiness", b, "program")?,
                    opt_in_env: required_field(
                        &projection_kind,
                        "adapter_readiness",
                        c,
                        "opt_in_env",
                    )?,
                    opted_in: required_field(&projection_kind, "adapter_readiness", d, "opted_in")?
                        .parse::<bool>()
                        .map_err(|error| {
                            ProjectionDecodeError(format!(
                                "invalid bool for adapter_readiness opted_in: {error}"
                            ))
                        })?,
                    smoke_status: required_field(
                        &projection_kind,
                        "adapter_readiness",
                        e,
                        "smoke_status",
                    )?,
                    credential_policy: required_field(
                        &projection_kind,
                        "adapter_readiness",
                        f,
                        "credential_policy",
                    )?,
                    expected_marker: required_field(
                        &projection_kind,
                        "adapter_readiness",
                        g,
                        "expected_marker",
                    )?,
                    env_allowlist_count: required_payload_i64(
                        &projection_kind,
                        "adapter_readiness",
                        &payload,
                        "env_allowlist_count",
                    )?,
                    redaction_rule_count: required_payload_i64(
                        &projection_kind,
                        "adapter_readiness",
                        &payload,
                        "redaction_rule_count",
                    )?,
                    output_limit_bytes: required_payload_i64(
                        &projection_kind,
                        "adapter_readiness",
                        &payload,
                        "output_limit_bytes",
                    )?,
                    dogfood_blocker: h,
                    updated_sequence: 0,
                },
            ))
        }
        "adapter_smoke_report" => {
            let payload = parse_projection_payload(&projection_kind, &record_id, &payload_json)?;
            Ok(ProjectionRecord::AdapterSmokeReport(
                AdapterSmokeReportProjection {
                    smoke_report_id: record_id,
                    project_id: ProjectId::new(required_field(
                        &projection_kind,
                        "adapter_smoke_report",
                        a,
                        "project_id",
                    )?),
                    adapter_kind: required_field(
                        &projection_kind,
                        "adapter_smoke_report",
                        b,
                        "adapter_kind",
                    )?,
                    smoke_status: required_field(
                        &projection_kind,
                        "adapter_smoke_report",
                        c,
                        "smoke_status",
                    )?,
                    credential_scan_status: required_field(
                        &projection_kind,
                        "adapter_smoke_report",
                        d,
                        "credential_scan_status",
                    )?,
                    marker_found: required_field(
                        &projection_kind,
                        "adapter_smoke_report",
                        e,
                        "marker_found",
                    )?
                    .parse::<bool>()
                    .map_err(|error| {
                        ProjectionDecodeError(format!(
                            "invalid bool for adapter_smoke_report marker_found: {error}"
                        ))
                    })?,
                    artifact_root: f,
                    reason: required_payload_string(
                        &projection_kind,
                        "adapter_smoke_report",
                        &payload,
                        "reason",
                    )?,
                    dogfood_readiness_effect: required_field(
                        &projection_kind,
                        "adapter_smoke_report",
                        g,
                        "dogfood_readiness_effect",
                    )?,
                    updated_sequence: 0,
                },
            ))
        }
        "adapter_dispatch_plan" => {
            let payload = parse_projection_payload(&projection_kind, &record_id, &payload_json)?;
            Ok(ProjectionRecord::AdapterDispatchPlan(
                AdapterDispatchPlanProjection {
                    dispatch_plan_id: record_id,
                    project_id: ProjectId::new(required_field(
                        &projection_kind,
                        "adapter_dispatch_plan",
                        a,
                        "project_id",
                    )?),
                    adapter_kind: required_field(
                        &projection_kind,
                        "adapter_dispatch_plan",
                        b,
                        "adapter_kind",
                    )?,
                    provider_kind: required_payload_string(
                        &projection_kind,
                        "adapter_dispatch_plan",
                        &payload,
                        "provider_kind",
                    )?,
                    credential_scope: required_payload_string(
                        &projection_kind,
                        "adapter_dispatch_plan",
                        &payload,
                        "credential_scope",
                    )?,
                    agent_id: AgentId::new(required_field(
                        &projection_kind,
                        "adapter_dispatch_plan",
                        c,
                        "agent_id",
                    )?),
                    agent_name: required_payload_string(
                        &projection_kind,
                        "adapter_dispatch_plan",
                        &payload,
                        "agent_name",
                    )?,
                    session_id: SessionId::new(required_field(
                        &projection_kind,
                        "adapter_dispatch_plan",
                        d,
                        "session_id",
                    )?),
                    run_id: RunId::new(required_field(
                        &projection_kind,
                        "adapter_dispatch_plan",
                        e,
                        "run_id",
                    )?),
                    runtime_program: required_payload_string(
                        &projection_kind,
                        "adapter_dispatch_plan",
                        &payload,
                        "runtime_program",
                    )?,
                    runtime_arg_count: required_payload_i64(
                        &projection_kind,
                        "adapter_dispatch_plan",
                        &payload,
                        "runtime_arg_count",
                    )?,
                    runtime_prompt_policy: required_field(
                        &projection_kind,
                        "adapter_dispatch_plan",
                        h,
                        "runtime_prompt_policy",
                    )?,
                    runtime_cwd: required_payload_string(
                        &projection_kind,
                        "adapter_dispatch_plan",
                        &payload,
                        "runtime_cwd",
                    )?,
                    artifact_root: required_payload_string(
                        &projection_kind,
                        "adapter_dispatch_plan",
                        &payload,
                        "artifact_root",
                    )?,
                    request_env_count: required_payload_i64(
                        &projection_kind,
                        "adapter_dispatch_plan",
                        &payload,
                        "request_env_count",
                    )?,
                    env_allowlist_count: required_payload_i64(
                        &projection_kind,
                        "adapter_dispatch_plan",
                        &payload,
                        "env_allowlist_count",
                    )?,
                    redaction_rule_count: required_payload_i64(
                        &projection_kind,
                        "adapter_dispatch_plan",
                        &payload,
                        "redaction_rule_count",
                    )?,
                    stdout_format: required_payload_string(
                        &projection_kind,
                        "adapter_dispatch_plan",
                        &payload,
                        "stdout_format",
                    )?,
                    stderr_policy: required_payload_string(
                        &projection_kind,
                        "adapter_dispatch_plan",
                        &payload,
                        "stderr_policy",
                    )?,
                    provider_cli_executed: required_field(
                        &projection_kind,
                        "adapter_dispatch_plan",
                        g,
                        "provider_cli_executed",
                    )?
                    .parse::<bool>()
                    .map_err(|error| {
                        ProjectionDecodeError(format!(
                            "invalid bool for adapter_dispatch_plan provider_cli_executed: {error}"
                        ))
                    })?,
                    status: required_field(&projection_kind, "adapter_dispatch_plan", f, "status")?,
                    updated_sequence: 0,
                },
            ))
        }
        "adapter_dispatch_gate" => {
            let payload = parse_projection_payload(&projection_kind, &record_id, &payload_json)?;
            Ok(ProjectionRecord::AdapterDispatchGate(
                AdapterDispatchGateProjection {
                    dispatch_gate_id: record_id,
                    project_id: ProjectId::new(required_field(
                        &projection_kind,
                        "adapter_dispatch_gate",
                        a,
                        "project_id",
                    )?),
                    dispatch_plan_id: required_field(
                        &projection_kind,
                        "adapter_dispatch_gate",
                        b,
                        "dispatch_plan_id",
                    )?,
                    adapter_kind: required_field(
                        &projection_kind,
                        "adapter_dispatch_gate",
                        c,
                        "adapter_kind",
                    )?,
                    provider_cli_execution_allowed: required_field(
                        &projection_kind,
                        "adapter_dispatch_gate",
                        d,
                        "provider_cli_execution_allowed",
                    )?
                    .parse::<bool>()
                    .map_err(|error| {
                        ProjectionDecodeError(format!(
                            "invalid bool for adapter_dispatch_gate provider_cli_execution_allowed: {error}"
                        ))
                    })?,
                    status: required_field(&projection_kind, "adapter_dispatch_gate", e, "status")?,
                    required_dogfood_gate: required_field(
                        &projection_kind,
                        "adapter_dispatch_gate",
                        f,
                        "required_dogfood_gate",
                    )?,
                    reason_codes: required_payload_string(
                        &projection_kind,
                        "adapter_dispatch_gate",
                        &payload,
                        "reason_codes",
                    )?,
                    provider_cli_executed: required_field(
                        &projection_kind,
                        "adapter_dispatch_gate",
                        g,
                        "provider_cli_executed",
                    )?
                    .parse::<bool>()
                    .map_err(|error| {
                        ProjectionDecodeError(format!(
                            "invalid bool for adapter_dispatch_gate provider_cli_executed: {error}"
                        ))
                    })?,
                    runtime_prompt_policy: required_field(
                        &projection_kind,
                        "adapter_dispatch_gate",
                        h,
                        "runtime_prompt_policy",
                    )?,
                    updated_sequence: 0,
                },
            ))
        }
        "adapter_dispatch_replay" => {
            let payload = parse_projection_payload(&projection_kind, &record_id, &payload_json)?;
            Ok(ProjectionRecord::AdapterDispatchReplay(
                AdapterDispatchReplayProjection {
                    dispatch_replay_id: record_id,
                    project_id: ProjectId::new(required_field(
                        &projection_kind,
                        "adapter_dispatch_replay",
                        a,
                        "project_id",
                    )?),
                    dispatch_plan_id: required_field(
                        &projection_kind,
                        "adapter_dispatch_replay",
                        b,
                        "dispatch_plan_id",
                    )?,
                    dispatch_gate_id: required_field(
                        &projection_kind,
                        "adapter_dispatch_replay",
                        c,
                        "dispatch_gate_id",
                    )?,
                    adapter_kind: required_field(
                        &projection_kind,
                        "adapter_dispatch_replay",
                        d,
                        "adapter_kind",
                    )?,
                    session_id: SessionId::new(required_field(
                        &projection_kind,
                        "adapter_dispatch_replay",
                        e,
                        "session_id",
                    )?),
                    run_id: RunId::new(required_field(
                        &projection_kind,
                        "adapter_dispatch_replay",
                        f,
                        "run_id",
                    )?),
                    fixture_path: required_payload_string(
                        &projection_kind,
                        "adapter_dispatch_replay",
                        &payload,
                        "fixture_path",
                    )?,
                    fixture_hash: required_payload_string(
                        &projection_kind,
                        "adapter_dispatch_replay",
                        &payload,
                        "fixture_hash",
                    )?,
                    input_event_count: required_payload_i64(
                        &projection_kind,
                        "adapter_dispatch_replay",
                        &payload,
                        "input_event_count",
                    )?,
                    appended_event_count: required_payload_i64(
                        &projection_kind,
                        "adapter_dispatch_replay",
                        &payload,
                        "appended_event_count",
                    )?,
                    tool_event_count: required_payload_i64(
                        &projection_kind,
                        "adapter_dispatch_replay",
                        &payload,
                        "tool_event_count",
                    )?,
                    summary_event_count: required_payload_i64(
                        &projection_kind,
                        "adapter_dispatch_replay",
                        &payload,
                        "summary_event_count",
                    )?,
                    completed_turn_count: required_payload_i64(
                        &projection_kind,
                        "adapter_dispatch_replay",
                        &payload,
                        "completed_turn_count",
                    )?,
                    provider_cli_executed: required_field(
                        &projection_kind,
                        "adapter_dispatch_replay",
                        g,
                        "provider_cli_executed",
                    )?
                    .parse::<bool>()
                    .map_err(|error| {
                        ProjectionDecodeError(format!(
                            "invalid bool for adapter_dispatch_replay provider_cli_executed: {error}"
                        ))
                    })?,
                    raw_content_policy: required_field(
                        &projection_kind,
                        "adapter_dispatch_replay",
                        h,
                        "raw_content_policy",
                    )?,
                    updated_sequence: 0,
                },
            ))
        }
        "adapter_dispatch_execution_request" => {
            let payload = parse_projection_payload(&projection_kind, &record_id, &payload_json)?;
            Ok(ProjectionRecord::AdapterDispatchExecutionRequest(
                AdapterDispatchExecutionRequestProjection {
                    execution_request_id: record_id,
                    project_id: ProjectId::new(required_field(
                        &projection_kind,
                        "adapter_dispatch_execution_request",
                        a,
                        "project_id",
                    )?),
                    dispatch_plan_id: required_field(
                        &projection_kind,
                        "adapter_dispatch_execution_request",
                        b,
                        "dispatch_plan_id",
                    )?,
                    dispatch_gate_id: required_field(
                        &projection_kind,
                        "adapter_dispatch_execution_request",
                        c,
                        "dispatch_gate_id",
                    )?,
                    adapter_kind: required_field(
                        &projection_kind,
                        "adapter_dispatch_execution_request",
                        d,
                        "adapter_kind",
                    )?,
                    provider_cli_execution_allowed: required_field(
                        &projection_kind,
                        "adapter_dispatch_execution_request",
                        e,
                        "provider_cli_execution_allowed",
                    )?
                    .parse::<bool>()
                    .map_err(|error| {
                        ProjectionDecodeError(format!(
                            "invalid bool for adapter_dispatch_execution_request provider_cli_execution_allowed: {error}"
                        ))
                    })?,
                    provider_cli_executed: required_field(
                        &projection_kind,
                        "adapter_dispatch_execution_request",
                        g,
                        "provider_cli_executed",
                    )?
                    .parse::<bool>()
                    .map_err(|error| {
                        ProjectionDecodeError(format!(
                            "invalid bool for adapter_dispatch_execution_request provider_cli_executed: {error}"
                        ))
                    })?,
                    status: required_field(
                        &projection_kind,
                        "adapter_dispatch_execution_request",
                        f,
                        "status",
                    )?,
                    opt_in_env: required_payload_string(
                        &projection_kind,
                        "adapter_dispatch_execution_request",
                        &payload,
                        "opt_in_env",
                    )?,
                    runtime_prompt_policy: required_field(
                        &projection_kind,
                        "adapter_dispatch_execution_request",
                        h,
                        "runtime_prompt_policy",
                    )?,
                    reason_codes: required_payload_string(
                        &projection_kind,
                        "adapter_dispatch_execution_request",
                        &payload,
                        "reason_codes",
                    )?,
                    updated_sequence: 0,
                },
            ))
        }
        "adapter_dispatch_execution" => {
            let payload = parse_projection_payload(&projection_kind, &record_id, &payload_json)?;
            Ok(ProjectionRecord::AdapterDispatchExecution(
                AdapterDispatchExecutionProjection {
                    dispatch_execution_id: record_id,
                    project_id: ProjectId::new(required_field(
                        &projection_kind,
                        "adapter_dispatch_execution",
                        a,
                        "project_id",
                    )?),
                    dispatch_plan_id: required_field(
                        &projection_kind,
                        "adapter_dispatch_execution",
                        b,
                        "dispatch_plan_id",
                    )?,
                    execution_request_id: required_field(
                        &projection_kind,
                        "adapter_dispatch_execution",
                        c,
                        "execution_request_id",
                    )?,
                    adapter_kind: required_field(
                        &projection_kind,
                        "adapter_dispatch_execution",
                        d,
                        "adapter_kind",
                    )?,
                    session_id: SessionId::new(required_field(
                        &projection_kind,
                        "adapter_dispatch_execution",
                        e,
                        "session_id",
                    )?),
                    run_id: RunId::new(required_field(
                        &projection_kind,
                        "adapter_dispatch_execution",
                        f,
                        "run_id",
                    )?),
                    provider_cli_execution_allowed: required_payload_string(
                        &projection_kind,
                        "adapter_dispatch_execution",
                        &payload,
                        "provider_cli_execution_allowed",
                    )?
                    .parse::<bool>()
                    .map_err(|error| {
                        ProjectionDecodeError(format!(
                            "invalid bool for adapter_dispatch_execution provider_cli_execution_allowed: {error}"
                        ))
                    })?,
                    provider_cli_executed: required_field(
                        &projection_kind,
                        "adapter_dispatch_execution",
                        g,
                        "provider_cli_executed",
                    )?
                    .parse::<bool>()
                    .map_err(|error| {
                        ProjectionDecodeError(format!(
                            "invalid bool for adapter_dispatch_execution provider_cli_executed: {error}"
                        ))
                    })?,
                    status: required_field(
                        &projection_kind,
                        "adapter_dispatch_execution",
                        h,
                        "status",
                    )?,
                    exit_code: payload_optional_i64(
                        &projection_kind,
                        "adapter_dispatch_execution",
                        &payload,
                        "exit_code",
                    )?,
                    runtime_process_ref: payload_optional_string(&payload, "runtime_process_ref"),
                    stdout_artifact_id: payload_optional_string(&payload, "stdout_artifact_id"),
                    stderr_artifact_id: payload_optional_string(&payload, "stderr_artifact_id"),
                    artifact_root: required_payload_string(
                        &projection_kind,
                        "adapter_dispatch_execution",
                        &payload,
                        "artifact_root",
                    )?,
                    credential_scan_status: required_payload_string(
                        &projection_kind,
                        "adapter_dispatch_execution",
                        &payload,
                        "credential_scan_status",
                    )?,
                    raw_prompt_policy: required_payload_string(
                        &projection_kind,
                        "adapter_dispatch_execution",
                        &payload,
                        "raw_prompt_policy",
                    )?,
                    raw_output_policy: required_payload_string(
                        &projection_kind,
                        "adapter_dispatch_execution",
                        &payload,
                        "raw_output_policy",
                    )?,
                    reason_codes: required_payload_string(
                        &projection_kind,
                        "adapter_dispatch_execution",
                        &payload,
                        "reason_codes",
                    )?,
                    updated_sequence: 0,
                },
            ))
        }
        "adapter_dispatch_prompt_source" => Ok(ProjectionRecord::AdapterDispatchPromptSource(
            AdapterDispatchPromptSourceProjection {
                prompt_source_id: record_id,
                project_id: ProjectId::new(required_field(
                    &projection_kind,
                    "adapter_dispatch_prompt_source",
                    a,
                    "project_id",
                )?),
                dispatch_plan_id: required_field(
                    &projection_kind,
                    "adapter_dispatch_prompt_source",
                    b,
                    "dispatch_plan_id",
                )?,
                prompt_hash: required_field(
                    &projection_kind,
                    "adapter_dispatch_prompt_source",
                    c,
                    "prompt_hash",
                )?,
                source_kind: required_field(
                    &projection_kind,
                    "adapter_dispatch_prompt_source",
                    d,
                    "source_kind",
                )?,
                source_ref: e,
                source_hash: f,
                materialization_status: required_field(
                    &projection_kind,
                    "adapter_dispatch_prompt_source",
                    g,
                    "materialization_status",
                )?,
                raw_prompt_policy: required_field(
                    &projection_kind,
                    "adapter_dispatch_prompt_source",
                    h,
                    "raw_prompt_policy",
                )?,
                updated_sequence: 0,
            },
        )),
        "adapter_dispatch_prompt_materialization" => {
            let payload = parse_projection_payload(&projection_kind, &record_id, &payload_json)?;
            Ok(ProjectionRecord::AdapterDispatchPromptMaterialization(
                AdapterDispatchPromptMaterializationProjection {
                    materialization_id: record_id,
                    project_id: ProjectId::new(required_field(
                        &projection_kind,
                        "adapter_dispatch_prompt_materialization",
                        a,
                        "project_id",
                    )?),
                    dispatch_plan_id: required_field(
                        &projection_kind,
                        "adapter_dispatch_prompt_materialization",
                        b,
                        "dispatch_plan_id",
                    )?,
                    prompt_source_id: required_field(
                        &projection_kind,
                        "adapter_dispatch_prompt_materialization",
                        c,
                        "prompt_source_id",
                    )?,
                    source_kind: required_field(
                        &projection_kind,
                        "adapter_dispatch_prompt_materialization",
                        d,
                        "source_kind",
                    )?,
                    source_ref: payload_optional_string(&payload, "source_ref"),
                    expected_source_hash: payload_optional_string(&payload, "expected_source_hash"),
                    observed_source_hash: payload_optional_string(&payload, "observed_source_hash"),
                    expected_prompt_hash: required_field(
                        &projection_kind,
                        "adapter_dispatch_prompt_materialization",
                        e,
                        "expected_prompt_hash",
                    )?,
                    materialized_prompt_hash: f,
                    status: required_field(
                        &projection_kind,
                        "adapter_dispatch_prompt_materialization",
                        g,
                        "status",
                    )?,
                    raw_prompt_policy: required_field(
                        &projection_kind,
                        "adapter_dispatch_prompt_materialization",
                        h,
                        "raw_prompt_policy",
                    )?,
                    reason_codes: required_payload_string(
                        &projection_kind,
                        "adapter_dispatch_prompt_materialization",
                        &payload,
                        "reason_codes",
                    )?,
                    updated_sequence: 0,
                },
            ))
        }
        "adapter_replay_batch" => {
            let payload = parse_projection_payload(&projection_kind, &record_id, &payload_json)?;
            Ok(ProjectionRecord::AdapterReplayBatch(
                AdapterReplayBatchProjection {
                    acp_replay_batch_id: record_id,
                    session_id: SessionId::new(required_field(
                        &projection_kind,
                        "adapter_replay_batch",
                        a,
                        "session_id",
                    )?),
                    project_id: ProjectId::new(required_field(
                        &projection_kind,
                        "adapter_replay_batch",
                        b,
                        "project_id",
                    )?),
                    external_session_ref: required_field(
                        &projection_kind,
                        "adapter_replay_batch",
                        c,
                        "external_session_ref",
                    )?,
                    source: required_field(&projection_kind, "adapter_replay_batch", d, "source")?,
                    status: required_field(&projection_kind, "adapter_replay_batch", e, "status")?,
                    load_request_id: f,
                    prompt_request_id: g,
                    recovery_attempt_id: h,
                    raw_update_count: required_payload_i64(
                        &projection_kind,
                        "adapter_replay_batch",
                        &payload,
                        "raw_update_count",
                    )?,
                    imported_count: required_payload_i64(
                        &projection_kind,
                        "adapter_replay_batch",
                        &payload,
                        "imported_count",
                    )?,
                    duplicate_count: required_payload_i64(
                        &projection_kind,
                        "adapter_replay_batch",
                        &payload,
                        "duplicate_count",
                    )?,
                    ambiguous_count: required_payload_i64(
                        &projection_kind,
                        "adapter_replay_batch",
                        &payload,
                        "ambiguous_count",
                    )?,
                    normalized_sequence_start: payload_optional_i64(
                        &projection_kind,
                        "adapter_replay_batch",
                        &payload,
                        "normalized_sequence_start",
                    )?,
                    normalized_sequence_end: payload_optional_i64(
                        &projection_kind,
                        "adapter_replay_batch",
                        &payload,
                        "normalized_sequence_end",
                    )?,
                    started_at: payload_optional_string(&payload, "started_at"),
                    completed_at: payload_optional_string(&payload, "completed_at"),
                    updated_sequence: 0,
                },
            ))
        }
        "adapter_raw_update" => {
            let payload = parse_projection_payload(&projection_kind, &record_id, &payload_json)?;
            let batch_index =
                required_field(&projection_kind, "adapter_raw_update", d, "batch_index")?
                    .parse::<i64>()
                    .map_err(|error| {
                        ProjectionDecodeError(format!(
                            "invalid i64 for adapter_raw_update batch_index: {error}"
                        ))
                    })?;
            Ok(ProjectionRecord::AdapterRawUpdate(
                AdapterRawUpdateProjection {
                    acp_raw_update_id: record_id,
                    acp_replay_batch_id: required_field(
                        &projection_kind,
                        "adapter_raw_update",
                        a,
                        "acp_replay_batch_id",
                    )?,
                    project_id: ProjectId::new(required_field(
                        &projection_kind,
                        "adapter_raw_update",
                        b,
                        "project_id",
                    )?),
                    external_session_ref: required_field(
                        &projection_kind,
                        "adapter_raw_update",
                        c,
                        "external_session_ref",
                    )?,
                    batch_index,
                    jsonrpc_method: required_field(
                        &projection_kind,
                        "adapter_raw_update",
                        e,
                        "jsonrpc_method",
                    )?,
                    session_update_kind: f,
                    external_item_ref: g,
                    acp_timeline_key: h,
                    payload_hash: required_payload_string(
                        &projection_kind,
                        "adapter_raw_update",
                        &payload,
                        "payload_hash",
                    )?,
                    payload_artifact_id: payload_optional_string(&payload, "payload_artifact_id"),
                    replay_source: required_payload_string(
                        &projection_kind,
                        "adapter_raw_update",
                        &payload,
                        "replay_source",
                    )?,
                    dedupe_confidence: required_payload_string(
                        &projection_kind,
                        "adapter_raw_update",
                        &payload,
                        "dedupe_confidence",
                    )?,
                    observed_at: payload_optional_string(&payload, "observed_at"),
                    updated_sequence: 0,
                },
            ))
        }
        "adapter_timeline_key" => {
            let payload = parse_projection_payload(&projection_kind, &record_id, &payload_json)?;
            Ok(ProjectionRecord::AdapterTimelineKey(
                AdapterTimelineKeyProjection {
                    adapter_timeline_key_id: record_id,
                    session_id: SessionId::new(required_field(
                        &projection_kind,
                        "adapter_timeline_key",
                        a,
                        "session_id",
                    )?),
                    project_id: ProjectId::new(required_field(
                        &projection_kind,
                        "adapter_timeline_key",
                        b,
                        "project_id",
                    )?),
                    external_session_ref: required_field(
                        &projection_kind,
                        "adapter_timeline_key",
                        c,
                        "external_session_ref",
                    )?,
                    kind: required_field(&projection_kind, "adapter_timeline_key", d, "kind")?,
                    stable_ref: e,
                    synthetic_ref: f,
                    confidence: required_field(
                        &projection_kind,
                        "adapter_timeline_key",
                        g,
                        "confidence",
                    )?,
                    first_sequence: payload_optional_i64(
                        &projection_kind,
                        "adapter_timeline_key",
                        &payload,
                        "first_sequence",
                    )?,
                    last_sequence: payload_optional_i64(
                        &projection_kind,
                        "adapter_timeline_key",
                        &payload,
                        "last_sequence",
                    )?,
                    updated_sequence: 0,
                },
            ))
        }
        other => Err(ProjectionDecodeError(format!(
            "unknown projection kind: {other}"
        ))),
    }
}
