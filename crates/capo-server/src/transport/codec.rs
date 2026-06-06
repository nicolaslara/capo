use capo_core::{AgentId, ProjectId, RunId, SessionId, TaskId};
use serde_json::{Value, json};

use super::{
    TransportError, TransportResult,
    wire::{
        input_origin_name, optional_bool, optional_i64, optional_string, parse_input_origin,
        required_bool, required_i64, required_string, required_string_array, required_usize,
        required_value,
    },
};
use crate::{
    AdapterReplaySummary, AgentSummary, DelegatedProviderGoalView, DispatchGateSummary,
    DispatchPlanSummary, DispatchRunSummary, DispatchTurnSummary, GoalContinuationView,
    GoalReportFormat, GoalReportListing, GoalReportRecord, GoalReportRendering, GoalReportView,
    GoalRequirementSpec, GoalRequirementView, GoalSpec, GoalStatusSummary, GoalTimelineEntry,
    GoalTimelineView, GoalView, LiveProviderPreflightSummary, RecoverySummary,
    RequirementStatusRecord, ServerClientOrigin, ServerCommand, ServerEvent, ServerResponse,
    ServerResponsePayload, ServerThread, ServerThreadItem, ServerThreadTurn, SessionSummary,
    SubscriptionBacklog, TaskRunSummary, TurnFinishedSummary,
};

/// GA2 (security egress backstop): scrub credential-shaped tokens out of an
/// agent-supplied free-text field before it crosses the transport boundary on a
/// goal read surface.
///
/// The goal read payloads (`Goals` / `GoalView` / `GoalReports` / `GoalReport`)
/// carry agent-authored text -- report summaries, the goal objective, blocker
/// reasons, the structured `*_json` blobs, and the rendered report body -- that
/// is recorded as `Safe` and so never passes through the `ServerEvent`
/// egress guard ([`ServerEvent::redacted_for_egress`]). This applies the SAME
/// `capo_runtime` credential-shape scanner that guard uses, at the SAME egress
/// point, so a secret an agent pasted into a report summary or a success-criteria
/// blob is scrubbed before it is streamed to every operator/subscriber rather
/// than emitted raw. A field with no credential shape is returned unchanged, so
/// the encode/decode round-trip is unaffected for ordinary content.
fn redact_egress_text(text: &str) -> String {
    let (scanned, _state) = capo_runtime::RedactionPolicy::new(Vec::new()).apply(text.as_bytes());
    String::from_utf8_lossy(&scanned).into_owned()
}

pub(super) fn encode_origin(origin: &ServerClientOrigin) -> Value {
    json!({
        "client_id": origin.client_id,
        "actor_id": origin.actor_id,
        "input_origin": input_origin_name(origin.input_origin),
    })
}

pub(super) fn decode_origin(origin: &Value) -> TransportResult<ServerClientOrigin> {
    Ok(ServerClientOrigin {
        client_id: required_string(origin, "client_id")?,
        actor_id: required_string(origin, "actor_id")?,
        input_origin: parse_input_origin(&required_string(origin, "input_origin")?)?,
    })
}

/// Encode the body of a JSON-RPC `result` object for a successful response.
///
/// This carries the same `client_id`/`actor_id`/`input_origin` origin
/// propagation and typed `payload` the previous codec used; the JSON-RPC `id`
/// (which mirrors `request_id`) is owned by the envelope in [`super::jsonrpc`],
/// so the result body itself does not repeat it.
pub(super) fn encode_response_result(response: &ServerResponse) -> Value {
    json!({
        "client_id": response.client_id,
        "actor_id": response.actor_id,
        "input_origin": input_origin_name(response.input_origin),
        "payload": encode_payload(&response.payload),
    })
}

/// Decode a JSON-RPC `result` body back into a [`ServerResponse`]. The
/// `request_id` is supplied by the envelope (the JSON-RPC `id`).
pub(super) fn decode_response_result(
    request_id: String,
    result: &Value,
) -> TransportResult<ServerResponse> {
    let payload = result
        .get("payload")
        .ok_or_else(|| TransportError::Protocol("missing payload".to_string()))
        .and_then(decode_payload)?;
    Ok(ServerResponse {
        request_id,
        client_id: required_string(result, "client_id")?,
        actor_id: required_string(result, "actor_id")?,
        input_origin: parse_input_origin(&required_string(result, "input_origin")?)?,
        payload,
    })
}

pub(super) fn encode_command(command: &ServerCommand) -> Value {
    match command {
        ServerCommand::RegisterAgent { name, adapter } => json!({
            "type": "register_agent",
            "name": name,
            "adapter": adapter,
        }),
        ServerCommand::RegisterRuntimeTarget {
            runtime_target_id,
            name,
            runner_kind,
            workspace_root,
            artifact_root,
            default_cwd,
            capability_profile_id,
            connectivity_endpoint_id,
            status,
        } => json!({
            "type": "register_runtime_target",
            "runtime_target_id": runtime_target_id,
            "name": name,
            "runner_kind": runner_kind,
            "workspace_root": workspace_root,
            "artifact_root": artifact_root,
            "default_cwd": default_cwd,
            "capability_profile_id": capability_profile_id,
            "connectivity_endpoint_id": connectivity_endpoint_id,
            "status": status,
        }),
        ServerCommand::SendTask {
            agent_name,
            goal,
            scenario,
        } => json!({
            "type": "send_task",
            "agent_name": agent_name,
            "goal": goal,
            "scenario": scenario,
        }),
        ServerCommand::SteerAgent { agent_name, goal } => json!({
            "type": "steer_agent",
            "agent_name": agent_name,
            "goal": goal,
        }),
        ServerCommand::InterruptAgent { agent_name, reason } => json!({
            "type": "interrupt_agent",
            "agent_name": agent_name,
            "reason": reason,
        }),
        ServerCommand::StopAgent { agent_name, reason } => json!({
            "type": "stop_agent",
            "agent_name": agent_name,
            "reason": reason,
        }),
        ServerCommand::ListAgents => json!({ "type": "list_agents" }),
        ServerCommand::AgentStatus { agent_name } => json!({
            "type": "agent_status",
            "agent_name": agent_name,
        }),
        ServerCommand::Dashboard { recent_event_limit } => json!({
            "type": "dashboard",
            "recent_event_limit": recent_event_limit,
        }),
        ServerCommand::StartSession {
            agent_name,
            goal,
            adapter,
            session_id,
            run_id,
        } => json!({
            "type": "start_session",
            "agent_name": agent_name,
            "goal": goal,
            "adapter": adapter,
            "session_id": session_id,
            "run_id": run_id,
        }),
        ServerCommand::ReplayAdapterFixture {
            adapter,
            session_id,
            run_id,
            turn_id,
            fixture_name,
            fixture_jsonl,
        } => json!({
            "type": "replay_adapter_fixture",
            "adapter": adapter,
            "session_id": session_id,
            "run_id": run_id,
            "turn_id": turn_id,
            "fixture_name": fixture_name,
            "fixture_jsonl": fixture_jsonl,
        }),
        ServerCommand::PlanDispatch {
            agent_name,
            adapter,
            goal,
            workspace,
            artifacts,
            session_id,
            run_id,
            turn_id,
            deterministic_opt_in,
        } => json!({
            "type": "plan_dispatch",
            "agent_name": agent_name,
            "adapter": adapter,
            "goal": goal,
            "workspace": workspace,
            "artifacts": artifacts,
            "session_id": session_id,
            "run_id": run_id,
            "turn_id": turn_id,
            "deterministic_opt_in": deterministic_opt_in,
        }),
        ServerCommand::PreflightLiveProvider {
            agent_name,
            adapter,
            goal,
            workspace,
            artifacts,
            session_id,
            run_id,
            turn_id,
            capability_profile,
            runtime_scope,
            credential_scan_policy,
            raw_prompt_policy,
            raw_output_policy,
            tool_wrapper_policy,
            live_provider_opt_in,
        } => json!({
            "type": "preflight_live_provider",
            "agent_name": agent_name,
            "adapter": adapter,
            "goal": goal,
            "workspace": workspace,
            "artifacts": artifacts,
            "session_id": session_id,
            "run_id": run_id,
            "turn_id": turn_id,
            "capability_profile": capability_profile,
            "runtime_scope": runtime_scope,
            "credential_scan_policy": credential_scan_policy,
            "raw_prompt_policy": raw_prompt_policy,
            "raw_output_policy": raw_output_policy,
            "tool_wrapper_policy": tool_wrapper_policy,
            "live_provider_opt_in": live_provider_opt_in,
        }),
        ServerCommand::GateDispatch { dispatch_plan_id } => json!({
            "type": "gate_dispatch",
            "dispatch_plan_id": dispatch_plan_id,
        }),
        ServerCommand::RunDispatchLocal {
            dispatch_plan_id,
            fixture_name,
            fixture_jsonl,
        } => json!({
            "type": "run_dispatch_local",
            "dispatch_plan_id": dispatch_plan_id,
            "fixture_name": fixture_name,
            "fixture_jsonl": fixture_jsonl,
        }),
        ServerCommand::RunLiveProviderLocal {
            dispatch_plan_id,
            goal,
            live_execution_opt_in,
            mock_runtime_opt_in,
            mock_provider_output_name,
            mock_provider_output_jsonl,
            timeout_seconds,
            codex_program_override,
            unattended,
        } => json!({
            "type": "run_live_provider_local",
            "dispatch_plan_id": dispatch_plan_id,
            "goal": goal,
            "live_execution_opt_in": live_execution_opt_in,
            "mock_runtime_opt_in": mock_runtime_opt_in,
            "mock_provider_output_name": mock_provider_output_name,
            "mock_provider_output_jsonl": mock_provider_output_jsonl,
            "timeout_seconds": timeout_seconds,
            "codex_program_override": codex_program_override,
            "unattended": unattended,
        }),
        ServerCommand::RunDispatchTurn {
            agent_name,
            adapter,
            goal,
            workspace,
            artifacts,
            session_id,
            run_id,
            turn_id,
            capability_profile,
            runtime_scope,
            credential_scan_policy,
            raw_prompt_policy,
            raw_output_policy,
            tool_wrapper_policy,
            live_provider_opt_in,
            live_execution_opt_in,
            mock_runtime_opt_in,
            mock_provider_output_name,
            mock_provider_output_jsonl,
            timeout_seconds,
            max_turns,
            max_token_cost,
            turns_taken_before,
            token_cost_before,
            turn_token_cost,
            unattended,
        } => json!({
            "type": "run_dispatch_turn",
            "agent_name": agent_name,
            "adapter": adapter,
            "goal": goal,
            "workspace": workspace,
            "artifacts": artifacts,
            "session_id": session_id,
            "run_id": run_id,
            "turn_id": turn_id,
            "capability_profile": capability_profile,
            "runtime_scope": runtime_scope,
            "credential_scan_policy": credential_scan_policy,
            "raw_prompt_policy": raw_prompt_policy,
            "raw_output_policy": raw_output_policy,
            "tool_wrapper_policy": tool_wrapper_policy,
            "live_provider_opt_in": live_provider_opt_in,
            "live_execution_opt_in": live_execution_opt_in,
            "mock_runtime_opt_in": mock_runtime_opt_in,
            "mock_provider_output_name": mock_provider_output_name,
            "mock_provider_output_jsonl": mock_provider_output_jsonl,
            "timeout_seconds": timeout_seconds,
            "max_turns": max_turns,
            "max_token_cost": max_token_cost,
            "turns_taken_before": turns_taken_before,
            "token_cost_before": token_cost_before,
            "turn_token_cost": turn_token_cost,
            "unattended": unattended,
        }),
        ServerCommand::RunAcpLiveTurnLocal {
            session_id,
            run_id,
            goal,
            turn_id,
            acp_program,
            acp_argv,
            workspace_root,
            live_acp_opt_in,
            acp_session_mode,
            mcp_url,
            mcp_headers,
            steer_window_secs,
        } => json!({
            "type": "run_acp_live_turn_local",
            "session_id": session_id,
            "run_id": run_id,
            "goal": goal,
            "turn_id": turn_id,
            "acp_program": acp_program,
            "acp_argv": acp_argv,
            "workspace_root": workspace_root,
            "live_acp_opt_in": live_acp_opt_in,
            "acp_session_mode": acp_session_mode,
            "mcp_url": mcp_url,
            "mcp_headers": mcp_headers
                .iter()
                .map(|(name, value)| json!({ "name": name, "value": value }))
                .collect::<Vec<_>>(),
            "steer_window_secs": steer_window_secs,
        }),
        ServerCommand::RunConductorTurnLocal {
            session_id,
            run_id,
            turn_id,
            user_message,
            conductor_goal,
            mcp_url,
            mcp_headers,
            acp_program,
            acp_argv,
            acp_session_mode,
            live_acp_opt_in,
            conductor_lockdown,
        } => json!({
            "type": "run_conductor_turn_local",
            "session_id": session_id,
            "run_id": run_id,
            "turn_id": turn_id,
            "user_message": user_message,
            "conductor_goal": conductor_goal,
            "mcp_url": mcp_url,
            "mcp_headers": mcp_headers
                .iter()
                .map(|(name, val)| json!({ "name": name, "value": val }))
                .collect::<Vec<_>>(),
            "acp_program": acp_program,
            "acp_argv": acp_argv,
            "acp_session_mode": acp_session_mode,
            "live_acp_opt_in": live_acp_opt_in,
            "conductor_lockdown": conductor_lockdown,
        }),
        ServerCommand::Recover => json!({ "type": "recover" }),
        ServerCommand::Subscribe {
            session_id,
            from_sequence,
        } => json!({
            "type": "subscribe",
            "session_id": session_id,
            "from_sequence": from_sequence,
        }),
        ServerCommand::ReadThread {
            session_id,
            from_sequence,
        } => json!({
            "type": "read_thread",
            "session_id": session_id,
            "from_sequence": from_sequence,
        }),
        ServerCommand::SetGoal { spec } => json!({
            "type": "set_goal",
            "spec": encode_goal_spec(spec),
        }),
        ServerCommand::PauseGoal { goal_id } => json!({
            "type": "pause_goal",
            "goal_id": goal_id,
        }),
        ServerCommand::ResumeGoal { goal_id } => json!({
            "type": "resume_goal",
            "goal_id": goal_id,
        }),
        ServerCommand::BlockGoal { goal_id, reason } => json!({
            "type": "block_goal",
            "goal_id": goal_id,
            "reason": reason,
        }),
        ServerCommand::ClearGoal { goal_id, reason } => json!({
            "type": "clear_goal",
            "goal_id": goal_id,
            "reason": reason,
        }),
        ServerCommand::SetRequirementStatus { record } => json!({
            "type": "set_requirement_status",
            "requirement_id": record.requirement_id,
            "goal_id": record.goal_id,
            "summary": record.summary,
            "status": record.status,
            "source": record.source,
        }),
        ServerCommand::RecordGoalReport { report } => json!({
            "type": "record_goal_report",
            "goal_report_id": report.goal_report_id,
            "goal_id": report.goal_id,
            "session_id": report.session_id,
            "requirement_id": report.requirement_id,
            "report_kind": report.report_kind,
            "source": report.source,
            "confidence": report.confidence,
            "summary": report.summary,
            "body_artifact_id": report.body_artifact_id,
            "evidence_id": report.evidence_id,
        }),
        ServerCommand::MarkGoalComplete { goal_id } => json!({
            "type": "mark_goal_complete",
            "goal_id": goal_id,
        }),
        ServerCommand::ListGoals => json!({ "type": "list_goals" }),
        ServerCommand::ViewGoal { goal_id } => json!({
            "type": "view_goal",
            "goal_id": goal_id,
        }),
        ServerCommand::GoalStory { goal_id } => json!({
            "type": "goal_story",
            "goal_id": goal_id,
        }),
        ServerCommand::GoalTimeline { goal_id } => json!({
            "type": "goal_timeline",
            "goal_id": goal_id,
        }),
        ServerCommand::GoalEvidence { goal_id } => json!({
            "type": "goal_evidence",
            "goal_id": goal_id,
        }),
        ServerCommand::GoalValidations { goal_id } => json!({
            "type": "goal_validations",
            "goal_id": goal_id,
        }),
        ServerCommand::GoalReviews { goal_id } => json!({
            "type": "goal_reviews",
            "goal_id": goal_id,
        }),
        ServerCommand::GoalRisks { goal_id } => json!({
            "type": "goal_risks",
            "goal_id": goal_id,
        }),
        ServerCommand::GoalReport { goal_id, format } => json!({
            "type": "goal_report",
            "goal_id": goal_id,
            "format": format.as_str(),
        }),
        ServerCommand::ContinueGoal {
            goal_id,
            continuation_id,
            conditions,
            turn,
        } => json!({
            "type": "continue_goal",
            "goal_id": goal_id,
            "continuation_id": continuation_id,
            "conditions": encode_continue_goal_conditions(conditions),
            "turn": encode_continue_goal_turn(turn),
        }),
        ServerCommand::ReplayRunnerEvents { frames } => json!({
            "type": "replay_runner_events",
            "frames": frames
                .iter()
                .map(encode_runner_replay_frame)
                .collect::<Vec<_>>(),
        }),
    }
}

fn encode_runner_replay_frame(frame: &crate::RunnerReplayFrame) -> Value {
    json!({
        "event_id": frame.event_id,
        "kind": frame.kind,
        "session_id": frame.session_id,
        "idempotency_key": frame.idempotency_key,
        "payload_json": frame.payload_json,
        "redaction_state": frame.redaction_state,
    })
}

fn decode_runner_replay_frame(value: &Value) -> TransportResult<crate::RunnerReplayFrame> {
    Ok(crate::RunnerReplayFrame {
        event_id: required_string(value, "event_id")?,
        kind: required_string(value, "kind")?,
        session_id: required_string(value, "session_id")?,
        idempotency_key: required_string(value, "idempotency_key")?,
        payload_json: required_string(value, "payload_json")?,
        redaction_state: required_string(value, "redaction_state")?,
    })
}

fn encode_continue_goal_conditions(c: &crate::ContinueGoalConditions) -> Value {
    json!({
        "enabled": c.enabled,
        "runtime_idle": c.runtime_idle,
        "session_idle": c.session_idle,
        "user_input_queued": c.user_input_queued,
        "permission_pending": c.permission_pending,
        "capability_profile_valid": c.capability_profile_valid,
        "next_step_writes_source": c.next_step_writes_source,
        "checkpoint_boundary_available": c.checkpoint_boundary_available,
        "verification_runner_available": c.verification_runner_available,
        "last_continuation_made_no_progress": c.last_continuation_made_no_progress,
        "strategy_changed_since_suppression": c.strategy_changed_since_suppression,
        "budget_max_turns": c.budget_max_turns,
        "budget_timeout_seconds": c.budget_timeout_seconds,
        "budget_max_token_cost": c.budget_max_token_cost,
        "budget_turns_taken": c.budget_turns_taken,
        "budget_token_cost": c.budget_token_cost,
    })
}

fn encode_continue_goal_turn(t: &crate::ContinueGoalTurn) -> Value {
    json!({
        "agent_name": t.agent_name,
        "adapter": t.adapter,
        "goal": t.goal,
        "workspace": t.workspace,
        "artifacts": t.artifacts,
        "session_id": t.session_id,
        "run_id": t.run_id,
        "turn_id": t.turn_id,
        "capability_profile": t.capability_profile,
        "runtime_scope": t.runtime_scope,
        "credential_scan_policy": t.credential_scan_policy,
        "raw_prompt_policy": t.raw_prompt_policy,
        "raw_output_policy": t.raw_output_policy,
        "tool_wrapper_policy": t.tool_wrapper_policy,
        "live_provider_opt_in": t.live_provider_opt_in,
        "live_execution_opt_in": t.live_execution_opt_in,
        "mock_runtime_opt_in": t.mock_runtime_opt_in,
        "mock_provider_output_name": t.mock_provider_output_name,
        "mock_provider_output_jsonl": t.mock_provider_output_jsonl,
        "timeout_seconds": t.timeout_seconds,
        "max_turns": t.max_turns,
        "max_token_cost": t.max_token_cost,
        "turns_taken_before": t.turns_taken_before,
        "token_cost_before": t.token_cost_before,
        "turn_token_cost": t.turn_token_cost,
        "unattended": t.unattended,
    })
}

fn decode_continue_goal_conditions(
    value: &Value,
) -> TransportResult<crate::ContinueGoalConditions> {
    Ok(crate::ContinueGoalConditions {
        enabled: required_bool(value, "enabled")?,
        runtime_idle: required_bool(value, "runtime_idle")?,
        session_idle: required_bool(value, "session_idle")?,
        user_input_queued: required_bool(value, "user_input_queued")?,
        permission_pending: required_bool(value, "permission_pending")?,
        capability_profile_valid: required_bool(value, "capability_profile_valid")?,
        next_step_writes_source: required_bool(value, "next_step_writes_source")?,
        checkpoint_boundary_available: required_bool(value, "checkpoint_boundary_available")?,
        verification_runner_available: required_bool(value, "verification_runner_available")?,
        last_continuation_made_no_progress: required_bool(
            value,
            "last_continuation_made_no_progress",
        )?,
        strategy_changed_since_suppression: required_bool(
            value,
            "strategy_changed_since_suppression",
        )?,
        budget_max_turns: required_usize(value, "budget_max_turns")? as u32,
        budget_timeout_seconds: required_usize(value, "budget_timeout_seconds")? as u64,
        budget_max_token_cost: required_usize(value, "budget_max_token_cost")? as u64,
        budget_turns_taken: required_usize(value, "budget_turns_taken")? as u32,
        budget_token_cost: required_usize(value, "budget_token_cost")? as u64,
    })
}

fn decode_continue_goal_turn(value: &Value) -> TransportResult<crate::ContinueGoalTurn> {
    Ok(crate::ContinueGoalTurn {
        agent_name: required_string(value, "agent_name")?,
        adapter: required_string(value, "adapter")?,
        goal: required_string(value, "goal")?,
        workspace: required_string(value, "workspace")?,
        artifacts: required_string(value, "artifacts")?,
        session_id: required_string(value, "session_id")?,
        run_id: required_string(value, "run_id")?,
        turn_id: required_string(value, "turn_id")?,
        capability_profile: required_string(value, "capability_profile")?,
        runtime_scope: required_string(value, "runtime_scope")?,
        credential_scan_policy: required_string(value, "credential_scan_policy")?,
        raw_prompt_policy: required_string(value, "raw_prompt_policy")?,
        raw_output_policy: required_string(value, "raw_output_policy")?,
        tool_wrapper_policy: required_string(value, "tool_wrapper_policy")?,
        live_provider_opt_in: required_bool(value, "live_provider_opt_in")?,
        live_execution_opt_in: required_bool(value, "live_execution_opt_in")?,
        mock_runtime_opt_in: required_bool(value, "mock_runtime_opt_in")?,
        mock_provider_output_name: optional_string(value, "mock_provider_output_name")?,
        mock_provider_output_jsonl: optional_string(value, "mock_provider_output_jsonl")?,
        timeout_seconds: required_usize(value, "timeout_seconds")? as u64,
        max_turns: required_usize(value, "max_turns")? as u32,
        max_token_cost: required_usize(value, "max_token_cost")? as u64,
        turns_taken_before: required_usize(value, "turns_taken_before")? as u32,
        token_cost_before: required_usize(value, "token_cost_before")? as u64,
        turn_token_cost: required_usize(value, "turn_token_cost")? as u64,
        unattended: optional_bool(value, "unattended")?.unwrap_or(true),
    })
}

pub(super) fn decode_command(value: &Value) -> TransportResult<ServerCommand> {
    match required_string(value, "type")?.as_str() {
        "register_agent" => Ok(ServerCommand::RegisterAgent {
            name: required_string(value, "name")?,
            // Back-compat: an older client that omits `adapter` defaults to the
            // fake chat adapter, so a missing field never binds Codex by surprise.
            adapter: optional_string(value, "adapter")?.unwrap_or_else(|| "fake".to_string()),
        }),
        "register_runtime_target" => Ok(ServerCommand::RegisterRuntimeTarget {
            runtime_target_id: required_string(value, "runtime_target_id")?,
            name: required_string(value, "name")?,
            runner_kind: required_string(value, "runner_kind")?,
            workspace_root: required_string(value, "workspace_root")?,
            artifact_root: required_string(value, "artifact_root")?,
            default_cwd: required_string(value, "default_cwd")?,
            capability_profile_id: required_string(value, "capability_profile_id")?,
            connectivity_endpoint_id: optional_string(value, "connectivity_endpoint_id")?,
            status: required_string(value, "status")?,
        }),
        "send_task" => Ok(ServerCommand::SendTask {
            agent_name: required_string(value, "agent_name")?,
            goal: required_string(value, "goal")?,
            scenario: required_string(value, "scenario")?,
        }),
        "steer_agent" => Ok(ServerCommand::SteerAgent {
            agent_name: required_string(value, "agent_name")?,
            goal: required_string(value, "goal")?,
        }),
        "interrupt_agent" => Ok(ServerCommand::InterruptAgent {
            agent_name: required_string(value, "agent_name")?,
            reason: required_string(value, "reason")?,
        }),
        "stop_agent" => Ok(ServerCommand::StopAgent {
            agent_name: required_string(value, "agent_name")?,
            reason: required_string(value, "reason")?,
        }),
        "list_agents" => Ok(ServerCommand::ListAgents),
        "agent_status" => Ok(ServerCommand::AgentStatus {
            agent_name: required_string(value, "agent_name")?,
        }),
        "dashboard" => Ok(ServerCommand::Dashboard {
            recent_event_limit: required_usize(value, "recent_event_limit")?,
        }),
        "start_session" => Ok(ServerCommand::StartSession {
            agent_name: required_string(value, "agent_name")?,
            goal: required_string(value, "goal")?,
            adapter: required_string(value, "adapter")?,
            session_id: optional_string(value, "session_id")?,
            run_id: optional_string(value, "run_id")?,
        }),
        "replay_adapter_fixture" => Ok(ServerCommand::ReplayAdapterFixture {
            adapter: required_string(value, "adapter")?,
            session_id: required_string(value, "session_id")?,
            run_id: required_string(value, "run_id")?,
            turn_id: required_string(value, "turn_id")?,
            fixture_name: required_string(value, "fixture_name")?,
            fixture_jsonl: required_string(value, "fixture_jsonl")?,
        }),
        "plan_dispatch" => Ok(ServerCommand::PlanDispatch {
            agent_name: required_string(value, "agent_name")?,
            adapter: required_string(value, "adapter")?,
            goal: required_string(value, "goal")?,
            workspace: required_string(value, "workspace")?,
            artifacts: required_string(value, "artifacts")?,
            session_id: required_string(value, "session_id")?,
            run_id: required_string(value, "run_id")?,
            turn_id: required_string(value, "turn_id")?,
            deterministic_opt_in: required_bool(value, "deterministic_opt_in")?,
        }),
        "preflight_live_provider" => Ok(ServerCommand::PreflightLiveProvider {
            agent_name: required_string(value, "agent_name")?,
            adapter: required_string(value, "adapter")?,
            goal: required_string(value, "goal")?,
            workspace: required_string(value, "workspace")?,
            artifacts: required_string(value, "artifacts")?,
            session_id: required_string(value, "session_id")?,
            run_id: required_string(value, "run_id")?,
            turn_id: required_string(value, "turn_id")?,
            capability_profile: required_string(value, "capability_profile")?,
            runtime_scope: required_string(value, "runtime_scope")?,
            credential_scan_policy: required_string(value, "credential_scan_policy")?,
            raw_prompt_policy: required_string(value, "raw_prompt_policy")?,
            raw_output_policy: required_string(value, "raw_output_policy")?,
            tool_wrapper_policy: required_string(value, "tool_wrapper_policy")?,
            live_provider_opt_in: required_bool(value, "live_provider_opt_in")?,
        }),
        "gate_dispatch" => Ok(ServerCommand::GateDispatch {
            dispatch_plan_id: required_string(value, "dispatch_plan_id")?,
        }),
        "run_dispatch_local" => Ok(ServerCommand::RunDispatchLocal {
            dispatch_plan_id: required_string(value, "dispatch_plan_id")?,
            fixture_name: required_string(value, "fixture_name")?,
            fixture_jsonl: required_string(value, "fixture_jsonl")?,
        }),
        "run_live_provider_local" => Ok(ServerCommand::RunLiveProviderLocal {
            dispatch_plan_id: required_string(value, "dispatch_plan_id")?,
            goal: required_string(value, "goal")?,
            live_execution_opt_in: required_bool(value, "live_execution_opt_in")?,
            mock_runtime_opt_in: required_bool(value, "mock_runtime_opt_in")?,
            mock_provider_output_name: optional_string(value, "mock_provider_output_name")?,
            mock_provider_output_jsonl: optional_string(value, "mock_provider_output_jsonl")?,
            timeout_seconds: required_usize(value, "timeout_seconds")? as u64,
            codex_program_override: optional_string(value, "codex_program_override")?,
            // Safe default: a turn with no explicit `unattended` flag is treated
            // as unattended, which forces the read-only dry-run profile (RTL6/9).
            unattended: optional_bool(value, "unattended")?.unwrap_or(true),
        }),
        "run_dispatch_turn" => Ok(ServerCommand::RunDispatchTurn {
            agent_name: required_string(value, "agent_name")?,
            adapter: required_string(value, "adapter")?,
            goal: required_string(value, "goal")?,
            workspace: required_string(value, "workspace")?,
            artifacts: required_string(value, "artifacts")?,
            session_id: required_string(value, "session_id")?,
            run_id: required_string(value, "run_id")?,
            turn_id: required_string(value, "turn_id")?,
            capability_profile: required_string(value, "capability_profile")?,
            runtime_scope: required_string(value, "runtime_scope")?,
            credential_scan_policy: required_string(value, "credential_scan_policy")?,
            raw_prompt_policy: required_string(value, "raw_prompt_policy")?,
            raw_output_policy: required_string(value, "raw_output_policy")?,
            tool_wrapper_policy: required_string(value, "tool_wrapper_policy")?,
            live_provider_opt_in: required_bool(value, "live_provider_opt_in")?,
            live_execution_opt_in: required_bool(value, "live_execution_opt_in")?,
            mock_runtime_opt_in: required_bool(value, "mock_runtime_opt_in")?,
            mock_provider_output_name: optional_string(value, "mock_provider_output_name")?,
            mock_provider_output_jsonl: optional_string(value, "mock_provider_output_jsonl")?,
            timeout_seconds: required_usize(value, "timeout_seconds")? as u64,
            max_turns: required_usize(value, "max_turns")? as u32,
            max_token_cost: required_usize(value, "max_token_cost")? as u64,
            turns_taken_before: required_usize(value, "turns_taken_before")? as u32,
            token_cost_before: required_usize(value, "token_cost_before")? as u64,
            turn_token_cost: required_usize(value, "turn_token_cost")? as u64,
            // Safe default: a turn with no explicit `unattended` flag is treated
            // as unattended, which forces the read-only dry-run profile (RTL6/9).
            unattended: optional_bool(value, "unattended")?.unwrap_or(true),
        }),
        "run_acp_live_turn_local" => Ok(ServerCommand::RunAcpLiveTurnLocal {
            session_id: required_string(value, "session_id")?,
            run_id: required_string(value, "run_id")?,
            goal: required_string(value, "goal")?,
            turn_id: required_string(value, "turn_id")?,
            acp_program: required_string(value, "acp_program")?,
            acp_argv: value
                .get("acp_argv")
                .and_then(|v| v.as_array())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.as_str().map(str::to_string))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default(),
            workspace_root: optional_string(value, "workspace_root")?,
            live_acp_opt_in: required_bool(value, "live_acp_opt_in")?,
            acp_session_mode: optional_string(value, "acp_session_mode")?,
            mcp_url: optional_string(value, "mcp_url")?,
            mcp_headers: value
                .get("mcp_headers")
                .and_then(|v| v.as_array())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| {
                            let name = item.get("name")?.as_str()?.to_string();
                            let val = item.get("value")?.as_str()?.to_string();
                            Some((name, val))
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default(),
            steer_window_secs: value
                .get("steer_window_secs")
                .and_then(Value::as_u64)
                .unwrap_or(0),
        }),
        "run_conductor_turn_local" => Ok(ServerCommand::RunConductorTurnLocal {
            session_id: required_string(value, "session_id")?,
            run_id: required_string(value, "run_id")?,
            turn_id: required_string(value, "turn_id")?,
            user_message: required_string(value, "user_message")?,
            conductor_goal: required_string(value, "conductor_goal")?,
            mcp_url: required_string(value, "mcp_url")?,
            mcp_headers: value
                .get("mcp_headers")
                .and_then(|v| v.as_array())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| {
                            let name = item.get("name")?.as_str()?.to_string();
                            let val = item.get("value")?.as_str()?.to_string();
                            Some((name, val))
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default(),
            acp_program: required_string(value, "acp_program")?,
            acp_argv: value
                .get("acp_argv")
                .and_then(|v| v.as_array())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.as_str().map(str::to_string))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default(),
            acp_session_mode: optional_string(value, "acp_session_mode")?,
            live_acp_opt_in: required_bool(value, "live_acp_opt_in")?,
            conductor_lockdown: optional_bool(value, "conductor_lockdown")?.unwrap_or(false),
        }),
        "recover" => Ok(ServerCommand::Recover),
        "subscribe" => Ok(ServerCommand::Subscribe {
            session_id: optional_string(value, "session_id")?,
            from_sequence: required_i64(value, "from_sequence")?,
        }),
        "read_thread" => Ok(ServerCommand::ReadThread {
            session_id: required_string(value, "session_id")?,
            from_sequence: required_i64(value, "from_sequence")?,
        }),
        "set_goal" => Ok(ServerCommand::SetGoal {
            spec: decode_goal_spec(required_value(value, "spec")?)?,
        }),
        "pause_goal" => Ok(ServerCommand::PauseGoal {
            goal_id: required_string(value, "goal_id")?,
        }),
        "resume_goal" => Ok(ServerCommand::ResumeGoal {
            goal_id: required_string(value, "goal_id")?,
        }),
        "block_goal" => Ok(ServerCommand::BlockGoal {
            goal_id: required_string(value, "goal_id")?,
            reason: required_string(value, "reason")?,
        }),
        "clear_goal" => Ok(ServerCommand::ClearGoal {
            goal_id: required_string(value, "goal_id")?,
            reason: required_string(value, "reason")?,
        }),
        "set_requirement_status" => Ok(ServerCommand::SetRequirementStatus {
            record: RequirementStatusRecord {
                requirement_id: required_string(value, "requirement_id")?,
                goal_id: required_string(value, "goal_id")?,
                summary: required_string(value, "summary")?,
                status: required_string(value, "status")?,
                source: required_string(value, "source")?,
            },
        }),
        "record_goal_report" => Ok(ServerCommand::RecordGoalReport {
            report: GoalReportRecord {
                goal_report_id: required_string(value, "goal_report_id")?,
                goal_id: required_string(value, "goal_id")?,
                session_id: optional_string(value, "session_id")?,
                requirement_id: optional_string(value, "requirement_id")?,
                report_kind: required_string(value, "report_kind")?,
                source: required_string(value, "source")?,
                confidence: optional_i64(value, "confidence")?,
                summary: required_string(value, "summary")?,
                body_artifact_id: optional_string(value, "body_artifact_id")?,
                evidence_id: optional_string(value, "evidence_id")?,
            },
        }),
        "mark_goal_complete" => Ok(ServerCommand::MarkGoalComplete {
            goal_id: required_string(value, "goal_id")?,
        }),
        "list_goals" => Ok(ServerCommand::ListGoals),
        "view_goal" => Ok(ServerCommand::ViewGoal {
            goal_id: required_string(value, "goal_id")?,
        }),
        "goal_story" => Ok(ServerCommand::GoalStory {
            goal_id: required_string(value, "goal_id")?,
        }),
        "goal_timeline" => Ok(ServerCommand::GoalTimeline {
            goal_id: required_string(value, "goal_id")?,
        }),
        "goal_evidence" => Ok(ServerCommand::GoalEvidence {
            goal_id: required_string(value, "goal_id")?,
        }),
        "goal_validations" => Ok(ServerCommand::GoalValidations {
            goal_id: required_string(value, "goal_id")?,
        }),
        "goal_reviews" => Ok(ServerCommand::GoalReviews {
            goal_id: required_string(value, "goal_id")?,
        }),
        "goal_risks" => Ok(ServerCommand::GoalRisks {
            goal_id: required_string(value, "goal_id")?,
        }),
        "goal_report" => Ok(ServerCommand::GoalReport {
            goal_id: required_string(value, "goal_id")?,
            format: decode_goal_report_format(&required_string(value, "format")?)?,
        }),
        "continue_goal" => Ok(ServerCommand::ContinueGoal {
            goal_id: required_string(value, "goal_id")?,
            continuation_id: required_string(value, "continuation_id")?,
            conditions: decode_continue_goal_conditions(required_value(value, "conditions")?)?,
            turn: Box::new(decode_continue_goal_turn(required_value(value, "turn")?)?),
        }),
        "replay_runner_events" => Ok(ServerCommand::ReplayRunnerEvents {
            frames: required_value(value, "frames")?
                .as_array()
                .ok_or_else(|| TransportError::Protocol("frames must be an array".to_string()))?
                .iter()
                .map(decode_runner_replay_frame)
                .collect::<TransportResult<Vec<_>>>()?,
        }),
        other => Err(TransportError::Protocol(format!(
            "unknown command type: {other}"
        ))),
    }
}

pub(super) fn encode_payload(payload: &ServerResponsePayload) -> Value {
    match payload {
        ServerResponsePayload::AgentRegistered(agent) => json!({
            "type": "agent_registered",
            "agent": encode_agent(agent),
        }),
        ServerResponsePayload::TaskSent(run) => json!({
            "type": "task_sent",
            "run": encode_run(run),
        }),
        ServerResponsePayload::Agents(agents) => json!({
            "type": "agents",
            "agents": agents.iter().map(encode_agent).collect::<Vec<_>>(),
        }),
        ServerResponsePayload::AgentStatus(agent) => json!({
            "type": "agent_status",
            "agent": encode_agent(agent),
        }),
        ServerResponsePayload::RuntimeTargetRegistered(target) => json!({
            "type": "runtime_target_registered",
            "runtime_target_id": target.runtime_target_id,
            "name": target.name,
            "runner_kind": target.runner_kind,
            "status": target.status,
            "connectivity_endpoint_id": target.connectivity_endpoint_id,
            "sequence": target.sequence,
        }),
        ServerResponsePayload::Dashboard(snapshot) => json!({
            "type": "dashboard",
            "project_id": snapshot.project_id.to_string(),
            "agent_count": snapshot.agent_count,
            "active_session_count": snapshot.active_session_count,
            "agents": snapshot.agents.iter().map(encode_agent).collect::<Vec<_>>(),
        }),
        ServerResponsePayload::SessionStarted(run) => json!({
            "type": "session_started",
            "run": encode_run(run),
        }),
        ServerResponsePayload::AdapterFixtureReplayed(replay) => json!({
            "type": "adapter_fixture_replayed",
            "adapter": replay.adapter,
            "fixture_name": replay.fixture_name,
            "fixture_hash": replay.fixture_hash,
            "agent_name": replay.agent_name,
            "task_id": replay.task_id.to_string(),
            "session_id": replay.session_id.to_string(),
            "run_id": replay.run_id.to_string(),
            "turn_id": replay.turn_id,
            "provider_cli_executed": replay.provider_cli_executed,
            "raw_content_policy": replay.raw_content_policy,
            "input_event_count": replay.input_event_count,
            "appended_event_count": replay.appended_event_count,
            "tool_event_count": replay.tool_event_count,
            "summary_event_count": replay.summary_event_count,
            "completed_turn_count": replay.completed_turn_count,
        }),
        ServerResponsePayload::DispatchPlanned(plan) => json!({
            "type": "dispatch_planned",
            "dispatch_plan_id": plan.dispatch_plan_id,
            "prompt_source_id": plan.prompt_source_id,
            "adapter": plan.adapter,
            "agent_name": plan.agent_name,
            "session_id": plan.session_id.to_string(),
            "run_id": plan.run_id.to_string(),
            "runtime_program": plan.runtime_program,
            "runtime_prompt_policy": plan.runtime_prompt_policy,
            "raw_prompt_policy": plan.raw_prompt_policy,
            "provider_cli_executed": plan.provider_cli_executed,
            "status": plan.status,
        }),
        ServerResponsePayload::LiveProviderPreflighted(preflight) => json!({
            "type": "live_provider_preflighted",
            "dispatch_plan_id": preflight.dispatch_plan_id,
            "dispatch_gate_id": preflight.dispatch_gate_id,
            "execution_request_id": preflight.execution_request_id,
            "adapter": preflight.adapter,
            "provider_kind": preflight.provider_kind,
            "agent_name": preflight.agent_name,
            "session_id": preflight.session_id.to_string(),
            "run_id": preflight.run_id.to_string(),
            "capability_profile": preflight.capability_profile,
            "runtime_scope": preflight.runtime_scope,
            "credential_scan_policy": preflight.credential_scan_policy,
            "raw_prompt_policy": preflight.raw_prompt_policy,
            "raw_output_policy": preflight.raw_output_policy,
            "tool_wrapper_policy": preflight.tool_wrapper_policy,
            "provider_cli_execution_allowed": preflight.provider_cli_execution_allowed,
            "provider_cli_executed": preflight.provider_cli_executed,
            "status": preflight.status,
            "reasons": preflight.reasons,
            "next_action": preflight.next_action,
        }),
        ServerResponsePayload::DispatchGated(gate) => json!({
            "type": "dispatch_gated",
            "dispatch_plan_id": gate.dispatch_plan_id,
            "dispatch_gate_id": gate.dispatch_gate_id,
            "execution_request_id": gate.execution_request_id,
            "materialization_id": gate.materialization_id,
            "adapter": gate.adapter,
            "provider_cli_execution_allowed": gate.provider_cli_execution_allowed,
            "provider_cli_executed": gate.provider_cli_executed,
            "status": gate.status,
            "reasons": gate.reasons,
            "raw_prompt_policy": gate.raw_prompt_policy,
        }),
        ServerResponsePayload::DispatchRun(run) => {
            let mut value = encode_dispatch_run(run);
            value["type"] = json!("dispatch_run");
            value
        }
        ServerResponsePayload::DispatchTurn(turn) => {
            let mut obj = encode_dispatch_turn_body(turn);
            obj.insert("type".to_string(), json!("dispatch_turn"));
            Value::Object(obj)
        }
        ServerResponsePayload::ContinuationEvaluated(summary) => json!({
            "type": "continuation_evaluated",
            "goal_id": summary.goal_id,
            "continuation_id": summary.continuation_id,
            "decision": summary.decision,
            "reason": summary.reason,
            "dispatched": summary
                .dispatched
                .as_ref()
                .map(|turn| Value::Object(encode_dispatch_turn_body(turn))),
        }),
        ServerResponsePayload::RunnerEventsReplayed(summary) => json!({
            "type": "runner_events_replayed",
            "appended_sequences": summary.appended_sequences,
        }),
        ServerResponsePayload::AcpLiveTurn(summary) => json!({
            "type": "acp_live_turn",
            "session_id": summary.session_id,
            "run_id": summary.run_id,
            "turn_id": summary.turn_id,
            "workspace_root": summary.workspace_root,
            "event_count": summary.event_count,
            "appended_event_count": summary.appended_event_count,
            "stop_reason": summary.stop_reason,
            "reply_text": summary.reply_text,
        }),
        ServerResponsePayload::Recovery(recovery) => json!({
            "type": "recovery",
            "recovery_attempt_id": recovery.recovery_attempt_id,
            "recovered_run_count": recovery.recovered_run_count,
            "watermark": recovery.watermark,
        }),
        ServerResponsePayload::Subscribed(backlog) => json!({
            "type": "subscribed",
            "session_id": backlog.session_id,
            "from_sequence": backlog.from_sequence,
            "next_sequence": backlog.next_sequence,
            "events": backlog.events.iter().map(encode_event).collect::<Vec<_>>(),
        }),
        ServerResponsePayload::Thread(thread) => json!({
            "type": "thread",
            "session_id": thread.session_id,
            "from_sequence": thread.from_sequence,
            "next_sequence": thread.next_sequence,
            "turns": thread.turns.iter().map(encode_thread_turn).collect::<Vec<_>>(),
        }),
        ServerResponsePayload::Goals(goals) => json!({
            "type": "goals",
            "goals": goals.iter().map(encode_goal_summary).collect::<Vec<_>>(),
        }),
        ServerResponsePayload::GoalView(view) => json!({
            "type": "goal_view",
            "view": encode_goal_view(view),
        }),
        ServerResponsePayload::GoalReports(listing) => json!({
            "type": "goal_reports",
            "goal_id": listing.goal_id,
            "surface": listing.surface,
            "blocker_reason": redact_egress_text(&listing.blocker_reason),
            "reports": listing.reports.iter().map(encode_goal_report_view).collect::<Vec<_>>(),
        }),
        ServerResponsePayload::GoalTimeline(timeline) => json!({
            "type": "goal_timeline",
            "goal_id": timeline.goal_id,
            "entries": timeline.entries.iter().map(|entry| json!({
                "sequence": entry.sequence,
                "event_id": entry.event_id,
                "kind": entry.kind,
                "actor": entry.actor,
                "redaction_state": entry.redaction_state,
            })).collect::<Vec<_>>(),
        }),
        ServerResponsePayload::GoalReport(rendering) => json!({
            "type": "goal_report",
            "goal_id": rendering.goal_id,
            "format": rendering.format,
            // The rendered body concatenates every agent-authored field
            // (objective, blocker, report summaries) -- scan it as the last
            // egress backstop so a credential pasted into any of them is scrubbed.
            "body": redact_egress_text(&rendering.body),
            "degraded": rendering.degraded,
        }),
    }
}

/// AI1/AI5: encode a [`DispatchTurnSummary`] body (no `type` tag). Shared by the
/// standalone `dispatch_turn` payload and the embedded `dispatched` turn of the
/// AI5 `continuation_evaluated` payload so both emit one identical turn shape.
fn encode_dispatch_turn_body(turn: &DispatchTurnSummary) -> serde_json::Map<String, Value> {
    let mut map = serde_json::Map::new();
    map.insert("run".to_string(), encode_dispatch_run(&turn.run));
    map.insert(
        "finished".to_string(),
        json!({
            "turn_id": turn.finished.turn_id,
            "stop_reason": turn.finished.stop_reason,
            "observed_terminal_event": turn.finished.observed_terminal_event,
            "summary_refs": turn.finished.summary_refs,
            "observed_tool_refs": turn.finished.observed_tool_refs,
        }),
    );
    map.insert(
        "ceiling_breach_code".to_string(),
        json!(turn.ceiling_breach_code),
    );
    map
}

/// AI1/AI5: decode a [`DispatchTurnSummary`] from its wire object (the inverse of
/// [`encode_dispatch_turn_body`]). Shared by the `dispatch_turn` payload and the
/// `dispatched` field of `continuation_evaluated`.
fn decode_dispatch_turn_summary(value: &Value) -> TransportResult<DispatchTurnSummary> {
    let finished = required_value(value, "finished")?;
    Ok(DispatchTurnSummary {
        run: decode_dispatch_run(required_value(value, "run")?)?,
        finished: TurnFinishedSummary {
            turn_id: required_string(finished, "turn_id")?,
            stop_reason: required_string(finished, "stop_reason")?,
            observed_terminal_event: required_bool(finished, "observed_terminal_event")?,
            summary_refs: required_string_array(finished, "summary_refs")?,
            observed_tool_refs: required_string_array(finished, "observed_tool_refs")?,
        },
        ceiling_breach_code: optional_string(value, "ceiling_breach_code")?,
    })
}

/// AI1: encode a [`DispatchRunSummary`] to its wire object. Shared by the
/// `dispatch_run` payload and the `dispatch_turn` payload (whose `run` field is
/// the same run summary) so the two paths emit one identical run shape. The
/// caller sets the top-level `type` tag for the standalone `dispatch_run`
/// payload; the embedded `run` object inside `dispatch_turn` carries no tag.
fn encode_dispatch_run(run: &DispatchRunSummary) -> Value {
    json!({
        "dispatch_plan_id": run.dispatch_plan_id,
        "dispatch_execution_id": run.dispatch_execution_id,
        "adapter": run.adapter,
        "session_id": run.session_id.to_string(),
        "run_id": run.run_id.to_string(),
        "provider_cli_execution_allowed": run.provider_cli_execution_allowed,
        "provider_cli_executed": run.provider_cli_executed,
        "status": run.status,
        "runtime_process_ref": run.runtime_process_ref,
        "credential_scan_status": run.credential_scan_status,
        "raw_prompt_policy": run.raw_prompt_policy,
        "raw_output_policy": run.raw_output_policy,
        "reason_codes": run.reason_codes,
        "input_event_count": run.input_event_count,
        "appended_event_count": run.appended_event_count,
        "tool_event_count": run.tool_event_count,
        "summary_event_count": run.summary_event_count,
        "completed_turn_count": run.completed_turn_count,
        "observed_token_cost": run.observed_token_cost,
    })
}

/// AI1: decode a [`DispatchRunSummary`] from its wire object. The inverse of
/// [`encode_dispatch_run`]; shared by the `dispatch_run` payload and the `run`
/// field of the `dispatch_turn` payload so both decode one identical run shape.
fn decode_dispatch_run(value: &Value) -> TransportResult<DispatchRunSummary> {
    Ok(DispatchRunSummary {
        dispatch_plan_id: required_string(value, "dispatch_plan_id")?,
        dispatch_execution_id: required_string(value, "dispatch_execution_id")?,
        adapter: required_string(value, "adapter")?,
        session_id: SessionId::new(required_string(value, "session_id")?),
        run_id: RunId::new(required_string(value, "run_id")?),
        provider_cli_execution_allowed: required_bool(value, "provider_cli_execution_allowed")?,
        provider_cli_executed: required_bool(value, "provider_cli_executed")?,
        status: required_string(value, "status")?,
        runtime_process_ref: optional_string(value, "runtime_process_ref")?,
        credential_scan_status: required_string(value, "credential_scan_status")?,
        raw_prompt_policy: required_string(value, "raw_prompt_policy")?,
        raw_output_policy: required_string(value, "raw_output_policy")?,
        reason_codes: required_string(value, "reason_codes")?,
        input_event_count: required_usize(value, "input_event_count")?,
        appended_event_count: required_usize(value, "appended_event_count")?,
        tool_event_count: required_usize(value, "tool_event_count")?,
        summary_event_count: required_usize(value, "summary_event_count")?,
        completed_turn_count: required_usize(value, "completed_turn_count")?,
        observed_token_cost: value.get("observed_token_cost").and_then(Value::as_u64),
    })
}

fn encode_thread_turn(turn: &ServerThreadTurn) -> Value {
    json!({
        "turn_id": turn.turn_id,
        "status": turn.status,
        "first_sequence": turn.first_sequence,
        "last_sequence": turn.last_sequence,
        "items": turn.items.iter().map(encode_thread_item).collect::<Vec<_>>(),
    })
}

fn encode_thread_item(item: &ServerThreadItem) -> Value {
    json!({
        "sequence": item.sequence,
        "event_id": item.event_id,
        "kind": item.kind,
        "event_kind": item.event_kind,
        "item_ref": item.item_ref,
        "text": item.text,
        "redaction_state": item.redaction_state,
    })
}

/// Encode a single tail event (ST4). The same shape is used both inside a
/// `subscribed` backlog and as the `params.event` of a live JSON-RPC
/// notification, so a client decodes a backlog event and a live event with one
/// code path.
pub(crate) fn encode_event(event: &ServerEvent) -> Value {
    json!({
        "sequence": event.sequence,
        "event_id": event.event_id,
        "kind": event.kind,
        "actor": event.actor,
        "project_id": event.project_id,
        "task_id": event.task_id,
        "agent_id": event.agent_id,
        "session_id": event.session_id,
        "run_id": event.run_id,
        "turn_id": event.turn_id,
        "item_id": event.item_id,
        "payload_json": event.payload_json,
        "redaction_state": event.redaction_state,
    })
}

pub(crate) fn decode_event(value: &Value) -> TransportResult<ServerEvent> {
    Ok(ServerEvent {
        sequence: required_i64(value, "sequence")?,
        event_id: required_string(value, "event_id")?,
        kind: required_string(value, "kind")?,
        actor: required_string(value, "actor")?,
        project_id: optional_string(value, "project_id")?,
        task_id: optional_string(value, "task_id")?,
        agent_id: optional_string(value, "agent_id")?,
        session_id: optional_string(value, "session_id")?,
        run_id: optional_string(value, "run_id")?,
        turn_id: optional_string(value, "turn_id")?,
        item_id: optional_string(value, "item_id")?,
        payload_json: required_string(value, "payload_json")?,
        redaction_state: required_string(value, "redaction_state")?,
    })
}

fn decode_thread_turn(value: &Value) -> TransportResult<ServerThreadTurn> {
    Ok(ServerThreadTurn {
        turn_id: required_string(value, "turn_id")?,
        status: required_string(value, "status")?,
        first_sequence: required_i64(value, "first_sequence")?,
        last_sequence: required_i64(value, "last_sequence")?,
        items: required_value(value, "items")?
            .as_array()
            .ok_or_else(|| TransportError::Protocol("items must be an array".to_string()))?
            .iter()
            .map(decode_thread_item)
            .collect::<TransportResult<Vec<_>>>()?,
    })
}

fn decode_thread_item(value: &Value) -> TransportResult<ServerThreadItem> {
    Ok(ServerThreadItem {
        sequence: required_i64(value, "sequence")?,
        event_id: required_string(value, "event_id")?,
        kind: required_string(value, "kind")?,
        event_kind: required_string(value, "event_kind")?,
        item_ref: optional_string(value, "item_ref")?,
        text: optional_string(value, "text")?,
        redaction_state: required_string(value, "redaction_state")?,
    })
}

pub(super) fn decode_payload(value: &Value) -> TransportResult<ServerResponsePayload> {
    match required_string(value, "type")?.as_str() {
        "agent_registered" => Ok(ServerResponsePayload::AgentRegistered(decode_agent(
            required_value(value, "agent")?,
        )?)),
        "task_sent" => Ok(ServerResponsePayload::TaskSent(decode_run(
            required_value(value, "run")?,
        )?)),
        "agents" => Ok(ServerResponsePayload::Agents(decode_agents(
            value, "agents",
        )?)),
        "agent_status" => Ok(ServerResponsePayload::AgentStatus(decode_agent(
            required_value(value, "agent")?,
        )?)),
        "runtime_target_registered" => Ok(ServerResponsePayload::RuntimeTargetRegistered(
            crate::ServerRuntimeTargetSummary {
                runtime_target_id: required_string(value, "runtime_target_id")?,
                name: required_string(value, "name")?,
                runner_kind: required_string(value, "runner_kind")?,
                status: required_string(value, "status")?,
                connectivity_endpoint_id: optional_string(value, "connectivity_endpoint_id")?,
                sequence: required_i64(value, "sequence")?,
            },
        )),
        "dashboard" => Ok(ServerResponsePayload::Dashboard(
            crate::ServerDashboardSnapshot {
                project_id: ProjectId::new(required_string(value, "project_id")?),
                agent_count: required_usize(value, "agent_count")?,
                active_session_count: required_usize(value, "active_session_count")?,
                agents: decode_agents(value, "agents")?,
            },
        )),
        "session_started" => Ok(ServerResponsePayload::SessionStarted(decode_run(
            required_value(value, "run")?,
        )?)),
        "adapter_fixture_replayed" => Ok(ServerResponsePayload::AdapterFixtureReplayed(
            AdapterReplaySummary {
                adapter: required_string(value, "adapter")?,
                fixture_name: required_string(value, "fixture_name")?,
                fixture_hash: required_string(value, "fixture_hash")?,
                agent_name: required_string(value, "agent_name")?,
                task_id: TaskId::new(required_string(value, "task_id")?),
                session_id: SessionId::new(required_string(value, "session_id")?),
                run_id: RunId::new(required_string(value, "run_id")?),
                turn_id: required_string(value, "turn_id")?,
                provider_cli_executed: required_bool(value, "provider_cli_executed")?,
                raw_content_policy: required_string(value, "raw_content_policy")?,
                input_event_count: required_usize(value, "input_event_count")?,
                appended_event_count: required_usize(value, "appended_event_count")?,
                tool_event_count: required_usize(value, "tool_event_count")?,
                summary_event_count: required_usize(value, "summary_event_count")?,
                completed_turn_count: required_usize(value, "completed_turn_count")?,
            },
        )),
        "dispatch_planned" => Ok(ServerResponsePayload::DispatchPlanned(
            DispatchPlanSummary {
                dispatch_plan_id: required_string(value, "dispatch_plan_id")?,
                prompt_source_id: required_string(value, "prompt_source_id")?,
                adapter: required_string(value, "adapter")?,
                agent_name: required_string(value, "agent_name")?,
                session_id: SessionId::new(required_string(value, "session_id")?),
                run_id: RunId::new(required_string(value, "run_id")?),
                runtime_program: required_string(value, "runtime_program")?,
                runtime_prompt_policy: required_string(value, "runtime_prompt_policy")?,
                raw_prompt_policy: required_string(value, "raw_prompt_policy")?,
                provider_cli_executed: required_bool(value, "provider_cli_executed")?,
                status: required_string(value, "status")?,
            },
        )),
        "live_provider_preflighted" => Ok(ServerResponsePayload::LiveProviderPreflighted(
            LiveProviderPreflightSummary {
                dispatch_plan_id: required_string(value, "dispatch_plan_id")?,
                dispatch_gate_id: required_string(value, "dispatch_gate_id")?,
                execution_request_id: required_string(value, "execution_request_id")?,
                adapter: required_string(value, "adapter")?,
                provider_kind: required_string(value, "provider_kind")?,
                agent_name: required_string(value, "agent_name")?,
                session_id: SessionId::new(required_string(value, "session_id")?),
                run_id: RunId::new(required_string(value, "run_id")?),
                capability_profile: required_string(value, "capability_profile")?,
                runtime_scope: required_string(value, "runtime_scope")?,
                credential_scan_policy: required_string(value, "credential_scan_policy")?,
                raw_prompt_policy: required_string(value, "raw_prompt_policy")?,
                raw_output_policy: required_string(value, "raw_output_policy")?,
                tool_wrapper_policy: required_string(value, "tool_wrapper_policy")?,
                provider_cli_execution_allowed: required_bool(
                    value,
                    "provider_cli_execution_allowed",
                )?,
                provider_cli_executed: required_bool(value, "provider_cli_executed")?,
                status: required_string(value, "status")?,
                reasons: required_string(value, "reasons")?,
                next_action: required_string(value, "next_action")?,
            },
        )),
        "dispatch_gated" => Ok(ServerResponsePayload::DispatchGated(DispatchGateSummary {
            dispatch_plan_id: required_string(value, "dispatch_plan_id")?,
            dispatch_gate_id: required_string(value, "dispatch_gate_id")?,
            execution_request_id: required_string(value, "execution_request_id")?,
            materialization_id: required_string(value, "materialization_id")?,
            adapter: required_string(value, "adapter")?,
            provider_cli_execution_allowed: required_bool(value, "provider_cli_execution_allowed")?,
            provider_cli_executed: required_bool(value, "provider_cli_executed")?,
            status: required_string(value, "status")?,
            reasons: required_string(value, "reasons")?,
            raw_prompt_policy: required_string(value, "raw_prompt_policy")?,
        })),
        "dispatch_run" => Ok(ServerResponsePayload::DispatchRun(decode_dispatch_run(
            value,
        )?)),
        "dispatch_turn" => Ok(ServerResponsePayload::DispatchTurn(
            decode_dispatch_turn_summary(value)?,
        )),
        "continuation_evaluated" => Ok(ServerResponsePayload::ContinuationEvaluated(
            crate::ContinuationEvaluatedSummary {
                goal_id: required_string(value, "goal_id")?,
                continuation_id: required_string(value, "continuation_id")?,
                decision: required_string(value, "decision")?,
                reason: required_string(value, "reason")?,
                dispatched: match value.get("dispatched") {
                    Some(Value::Null) | None => None,
                    Some(turn) => Some(decode_dispatch_turn_summary(turn)?),
                },
            },
        )),
        "runner_events_replayed" => Ok(ServerResponsePayload::RunnerEventsReplayed(
            crate::RunnerEventsReplayedSummary {
                appended_sequences: required_value(value, "appended_sequences")?
                    .as_array()
                    .ok_or_else(|| {
                        TransportError::Protocol("appended_sequences must be an array".to_string())
                    })?
                    .iter()
                    .map(|seq| {
                        seq.as_i64().ok_or_else(|| {
                            TransportError::Protocol(
                                "appended_sequences entries must be integers".to_string(),
                            )
                        })
                    })
                    .collect::<TransportResult<Vec<_>>>()?,
            },
        )),
        "acp_live_turn" => Ok(ServerResponsePayload::AcpLiveTurn(
            crate::AcpLiveTurnSummary {
                session_id: required_string(value, "session_id")?,
                run_id: required_string(value, "run_id")?,
                turn_id: required_string(value, "turn_id")?,
                workspace_root: required_string(value, "workspace_root")?,
                event_count: required_usize(value, "event_count")?,
                appended_event_count: required_usize(value, "appended_event_count")?,
                stop_reason: optional_string(value, "stop_reason")?,
                reply_text: optional_string(value, "reply_text")?,
            },
        )),
        "recovery" => Ok(ServerResponsePayload::Recovery(RecoverySummary {
            recovery_attempt_id: required_string(value, "recovery_attempt_id")?,
            recovered_run_count: required_usize(value, "recovered_run_count")?,
            watermark: value.get("watermark").and_then(Value::as_i64),
        })),
        "subscribed" => Ok(ServerResponsePayload::Subscribed(SubscriptionBacklog {
            session_id: optional_string(value, "session_id")?,
            from_sequence: required_i64(value, "from_sequence")?,
            next_sequence: required_i64(value, "next_sequence")?,
            events: required_value(value, "events")?
                .as_array()
                .ok_or_else(|| TransportError::Protocol("events must be an array".to_string()))?
                .iter()
                .map(decode_event)
                .collect::<TransportResult<Vec<_>>>()?,
        })),
        "thread" => Ok(ServerResponsePayload::Thread(ServerThread {
            session_id: required_string(value, "session_id")?,
            from_sequence: required_i64(value, "from_sequence")?,
            next_sequence: required_i64(value, "next_sequence")?,
            turns: required_value(value, "turns")?
                .as_array()
                .ok_or_else(|| TransportError::Protocol("turns must be an array".to_string()))?
                .iter()
                .map(decode_thread_turn)
                .collect::<TransportResult<Vec<_>>>()?,
        })),
        "goals" => Ok(ServerResponsePayload::Goals(
            required_value(value, "goals")?
                .as_array()
                .ok_or_else(|| TransportError::Protocol("goals must be an array".to_string()))?
                .iter()
                .map(decode_goal_summary)
                .collect::<TransportResult<Vec<_>>>()?,
        )),
        "goal_view" => Ok(ServerResponsePayload::GoalView(Box::new(decode_goal_view(
            required_value(value, "view")?,
        )?))),
        "goal_reports" => Ok(ServerResponsePayload::GoalReports(GoalReportListing {
            goal_id: required_string(value, "goal_id")?,
            surface: required_string(value, "surface")?,
            blocker_reason: required_string(value, "blocker_reason")?,
            reports: required_value(value, "reports")?
                .as_array()
                .ok_or_else(|| TransportError::Protocol("reports must be an array".to_string()))?
                .iter()
                .map(decode_goal_report_view)
                .collect::<TransportResult<Vec<_>>>()?,
        })),
        "goal_timeline" => Ok(ServerResponsePayload::GoalTimeline(GoalTimelineView {
            goal_id: required_string(value, "goal_id")?,
            entries: required_value(value, "entries")?
                .as_array()
                .ok_or_else(|| TransportError::Protocol("entries must be an array".to_string()))?
                .iter()
                .map(|entry| {
                    Ok(GoalTimelineEntry {
                        sequence: required_i64(entry, "sequence")?,
                        event_id: required_string(entry, "event_id")?,
                        kind: required_string(entry, "kind")?,
                        actor: required_string(entry, "actor")?,
                        redaction_state: required_string(entry, "redaction_state")?,
                    })
                })
                .collect::<TransportResult<Vec<_>>>()?,
        })),
        "goal_report" => Ok(ServerResponsePayload::GoalReport(GoalReportRendering {
            goal_id: required_string(value, "goal_id")?,
            format: required_string(value, "format")?,
            body: required_string(value, "body")?,
            degraded: required_bool(value, "degraded")?,
        })),
        other => Err(TransportError::Protocol(format!(
            "unknown payload type: {other}"
        ))),
    }
}

/// GA2: encode the structured [`GoalSpec`] a `SetGoal` carries. Every structured
/// field is preserved verbatim so the round-trip reproduces the spec exactly --
/// the goal is durable, rebuildable state, not transcript text.
fn encode_goal_spec(spec: &GoalSpec) -> Value {
    json!({
        "goal_id": spec.goal_id,
        "objective": spec.objective,
        "task_id": spec.task_id,
        "agent_id": spec.agent_id,
        "session_id": spec.session_id,
        "parent_goal_id": spec.parent_goal_id,
        "attempt_run_id": spec.attempt_run_id,
        "requirements": spec.requirements.iter().map(|requirement| json!({
            "requirement_id": requirement.requirement_id,
            "summary": requirement.summary,
        })).collect::<Vec<_>>(),
        "success_criteria_json": spec.success_criteria_json,
        "constraints_json": spec.constraints_json,
        "verification_surface_json": spec.verification_surface_json,
        "budget_json": spec.budget_json,
        "stop_conditions_json": spec.stop_conditions_json,
    })
}

/// GA2: decode a [`GoalSpec`]; the inverse of [`encode_goal_spec`].
fn decode_goal_spec(value: &Value) -> TransportResult<GoalSpec> {
    Ok(GoalSpec {
        goal_id: required_string(value, "goal_id")?,
        objective: required_string(value, "objective")?,
        task_id: optional_string(value, "task_id")?,
        agent_id: optional_string(value, "agent_id")?,
        session_id: optional_string(value, "session_id")?,
        parent_goal_id: optional_string(value, "parent_goal_id")?,
        attempt_run_id: optional_string(value, "attempt_run_id")?,
        requirements: required_value(value, "requirements")?
            .as_array()
            .ok_or_else(|| TransportError::Protocol("requirements must be an array".to_string()))?
            .iter()
            .map(|requirement| {
                Ok(GoalRequirementSpec {
                    requirement_id: required_string(requirement, "requirement_id")?,
                    summary: required_string(requirement, "summary")?,
                })
            })
            .collect::<TransportResult<Vec<_>>>()?,
        success_criteria_json: required_string(value, "success_criteria_json")?,
        constraints_json: required_string(value, "constraints_json")?,
        verification_surface_json: required_string(value, "verification_surface_json")?,
        budget_json: required_string(value, "budget_json")?,
        stop_conditions_json: required_string(value, "stop_conditions_json")?,
    })
}

/// GA2: decode the historical-report [`GoalReportFormat`] from its wire string.
fn decode_goal_report_format(value: &str) -> TransportResult<GoalReportFormat> {
    match value {
        "markdown" => Ok(GoalReportFormat::Markdown),
        "json" => Ok(GoalReportFormat::Json),
        other => Err(TransportError::Protocol(format!(
            "unknown goal report format: {other}"
        ))),
    }
}

/// GA2: encode a concise [`GoalStatusSummary`] for the `goals` listing.
fn encode_goal_summary(summary: &GoalStatusSummary) -> Value {
    json!({
        "goal_id": summary.goal_id,
        "objective": redact_egress_text(&summary.objective),
        "status": summary.status,
        "parent_goal_id": summary.parent_goal_id,
        "attempt_run_id": summary.attempt_run_id,
        "requirement_count": summary.requirement_count,
        "requirements_supported": summary.requirements_supported,
        "blocked_requirement_count": summary.blocked_requirement_count,
        "contradicted_requirement_count": summary.contradicted_requirement_count,
        "report_count": summary.report_count,
        "blocker_reason": redact_egress_text(&summary.blocker_reason),
        "updated_sequence": summary.updated_sequence,
    })
}

/// GA2: decode a [`GoalStatusSummary`]; the inverse of [`encode_goal_summary`].
fn decode_goal_summary(value: &Value) -> TransportResult<GoalStatusSummary> {
    Ok(GoalStatusSummary {
        goal_id: required_string(value, "goal_id")?,
        objective: required_string(value, "objective")?,
        status: required_string(value, "status")?,
        parent_goal_id: optional_string(value, "parent_goal_id")?,
        attempt_run_id: optional_string(value, "attempt_run_id")?,
        requirement_count: required_usize(value, "requirement_count")?,
        requirements_supported: required_usize(value, "requirements_supported")?,
        blocked_requirement_count: required_usize(value, "blocked_requirement_count")?,
        contradicted_requirement_count: required_usize(value, "contradicted_requirement_count")?,
        report_count: required_usize(value, "report_count")?,
        blocker_reason: required_string(value, "blocker_reason")?,
        updated_sequence: required_i64(value, "updated_sequence")?,
    })
}

/// GA2: encode one requirement-ledger row for the goal view.
fn encode_goal_requirement_view(requirement: &GoalRequirementView) -> Value {
    json!({
        "requirement_id": requirement.requirement_id,
        "summary": redact_egress_text(&requirement.summary),
        "status": requirement.status,
        "last_status_source": requirement.last_status_source,
        "observed": requirement.observed,
    })
}

/// GA2: decode one requirement-ledger row for the goal view.
fn decode_goal_requirement_view(value: &Value) -> TransportResult<GoalRequirementView> {
    Ok(GoalRequirementView {
        requirement_id: required_string(value, "requirement_id")?,
        summary: required_string(value, "summary")?,
        status: required_string(value, "status")?,
        last_status_source: required_string(value, "last_status_source")?,
        observed: required_bool(value, "observed")?,
    })
}

/// GA2: encode one report/evidence/review/validation row. Observed-vs-reported is
/// carried on the wire so a client never reconstructs it from prose.
fn encode_goal_report_view(report: &GoalReportView) -> Value {
    json!({
        "goal_report_id": report.goal_report_id,
        "requirement_id": report.requirement_id,
        "report_kind": report.report_kind,
        "source": report.source,
        "observed": report.observed,
        "confidence": report.confidence,
        "summary": redact_egress_text(&report.summary),
        "body_artifact_id": report.body_artifact_id,
        "evidence_id": report.evidence_id,
    })
}

/// GA2: decode one report row; the inverse of [`encode_goal_report_view`].
fn decode_goal_report_view(value: &Value) -> TransportResult<GoalReportView> {
    Ok(GoalReportView {
        goal_report_id: required_string(value, "goal_report_id")?,
        requirement_id: optional_string(value, "requirement_id")?,
        report_kind: required_string(value, "report_kind")?,
        source: required_string(value, "source")?,
        observed: required_bool(value, "observed")?,
        confidence: optional_i64(value, "confidence")?,
        summary: required_string(value, "summary")?,
        body_artifact_id: optional_string(value, "body_artifact_id")?,
        evidence_id: optional_string(value, "evidence_id")?,
    })
}

/// GA2: encode one continuation-decision row for the goal view.
fn encode_goal_continuation_view(continuation: &GoalContinuationView) -> Value {
    json!({
        "continuation_id": continuation.continuation_id,
        "decision": continuation.decision,
        "reason": redact_egress_text(&continuation.reason),
        "attempt_run_id": continuation.attempt_run_id,
    })
}

/// GA2: decode one continuation-decision row for the goal view.
fn decode_goal_continuation_view(value: &Value) -> TransportResult<GoalContinuationView> {
    Ok(GoalContinuationView {
        continuation_id: required_string(value, "continuation_id")?,
        decision: required_string(value, "decision")?,
        reason: required_string(value, "reason")?,
        attempt_run_id: optional_string(value, "attempt_run_id")?,
    })
}

/// GA2: encode one delegated-provider goal row (observed, never authoritative).
fn encode_delegated_provider_goal_view(delegated: &DelegatedProviderGoalView) -> Value {
    json!({
        "delegated_goal_id": delegated.delegated_goal_id,
        "provider_kind": delegated.provider_kind,
        "provider_goal_ref": delegated.provider_goal_ref,
        "provider_state": delegated.provider_state,
        "source": delegated.source,
    })
}

/// GA2: decode one delegated-provider goal row.
fn decode_delegated_provider_goal_view(
    value: &Value,
) -> TransportResult<DelegatedProviderGoalView> {
    Ok(DelegatedProviderGoalView {
        delegated_goal_id: required_string(value, "delegated_goal_id")?,
        provider_kind: required_string(value, "provider_kind")?,
        provider_goal_ref: optional_string(value, "provider_goal_ref")?,
        provider_state: required_string(value, "provider_state")?,
        source: required_string(value, "source")?,
    })
}

/// GA2: encode a full [`GoalView`], assembled from the goal, requirement-ledger,
/// story, continuation, and delegated-provider projections.
fn encode_goal_view(view: &GoalView) -> Value {
    json!({
        "summary": encode_goal_summary(&view.summary),
        "success_criteria_json": redact_egress_text(&view.success_criteria_json),
        "constraints_json": redact_egress_text(&view.constraints_json),
        "verification_surface_json": redact_egress_text(&view.verification_surface_json),
        "budget_json": redact_egress_text(&view.budget_json),
        "stop_conditions_json": redact_egress_text(&view.stop_conditions_json),
        "task_id": view.task_id,
        "agent_id": view.agent_id,
        "session_id": view.session_id,
        "requirements": view.requirements.iter().map(encode_goal_requirement_view).collect::<Vec<_>>(),
        "reports": view.reports.iter().map(encode_goal_report_view).collect::<Vec<_>>(),
        "continuations": view.continuations.iter().map(encode_goal_continuation_view).collect::<Vec<_>>(),
        "delegated_provider_goals": view.delegated_provider_goals.iter().map(encode_delegated_provider_goal_view).collect::<Vec<_>>(),
    })
}

/// GA2: decode a full [`GoalView`]; the inverse of [`encode_goal_view`].
fn decode_goal_view(value: &Value) -> TransportResult<GoalView> {
    Ok(GoalView {
        summary: decode_goal_summary(required_value(value, "summary")?)?,
        success_criteria_json: required_string(value, "success_criteria_json")?,
        constraints_json: required_string(value, "constraints_json")?,
        verification_surface_json: required_string(value, "verification_surface_json")?,
        budget_json: required_string(value, "budget_json")?,
        stop_conditions_json: required_string(value, "stop_conditions_json")?,
        task_id: optional_string(value, "task_id")?,
        agent_id: optional_string(value, "agent_id")?,
        session_id: optional_string(value, "session_id")?,
        requirements: required_value(value, "requirements")?
            .as_array()
            .ok_or_else(|| TransportError::Protocol("requirements must be an array".to_string()))?
            .iter()
            .map(decode_goal_requirement_view)
            .collect::<TransportResult<Vec<_>>>()?,
        reports: required_value(value, "reports")?
            .as_array()
            .ok_or_else(|| TransportError::Protocol("reports must be an array".to_string()))?
            .iter()
            .map(decode_goal_report_view)
            .collect::<TransportResult<Vec<_>>>()?,
        continuations: required_value(value, "continuations")?
            .as_array()
            .ok_or_else(|| TransportError::Protocol("continuations must be an array".to_string()))?
            .iter()
            .map(decode_goal_continuation_view)
            .collect::<TransportResult<Vec<_>>>()?,
        delegated_provider_goals: required_value(value, "delegated_provider_goals")?
            .as_array()
            .ok_or_else(|| {
                TransportError::Protocol("delegated_provider_goals must be an array".to_string())
            })?
            .iter()
            .map(decode_delegated_provider_goal_view)
            .collect::<TransportResult<Vec<_>>>()?,
    })
}

fn encode_agent(agent: &AgentSummary) -> Value {
    json!({
        "agent_id": agent.agent_id.to_string(),
        "name": agent.name,
        "status": agent.status,
        "current_session_id": agent.current_session_id.as_ref().map(ToString::to_string),
        "session": agent.session.as_ref().map(encode_session),
    })
}

fn decode_agent(value: &Value) -> TransportResult<AgentSummary> {
    Ok(AgentSummary {
        agent_id: AgentId::new(required_string(value, "agent_id")?),
        name: required_string(value, "name")?,
        status: required_string(value, "status")?,
        current_session_id: optional_string(value, "current_session_id")?.map(SessionId::new),
        session: value
            .get("session")
            .filter(|value| !value.is_null())
            .map(decode_session)
            .transpose()?,
    })
}

fn encode_session(session: &SessionSummary) -> Value {
    json!({
        "session_id": session.session_id.to_string(),
        "status": session.status,
        "run_id": session.run_id.as_ref().map(ToString::to_string),
        "run_status": session.run_status,
        "adapter_kind": session.adapter_kind,
        "current_goal": session.current_goal,
        "latest_summary": session.latest_summary,
        "latest_blocker": session.latest_blocker,
        "latest_confidence": session.latest_confidence,
        "recent_event_count": session.recent_event_count,
        "evidence_count": session.evidence_count,
        "evidence_refs": session.evidence_refs,
        "review_finding_count": session.review_finding_count,
        "task_outcome_report_count": session.task_outcome_report_count,
        "turn_count": session.turn_count,
        "turn_ids": session.turn_ids,
        "latest_dispatch_plan_id": session.latest_dispatch_plan_id,
        "latest_dispatch_gate_id": session.latest_dispatch_gate_id,
        "latest_dispatch_execution_id": session.latest_dispatch_execution_id,
        "dispatch_gate_status": session.dispatch_gate_status,
        "dispatch_gate_reasons": session.dispatch_gate_reasons,
        "dispatch_next_action": session.dispatch_next_action,
        "dispatch_execution_status": session.dispatch_execution_status,
        "dispatch_runtime_process_ref": session.dispatch_runtime_process_ref,
        "dispatch_provider_cli_execution_allowed": session.dispatch_provider_cli_execution_allowed,
        "dispatch_provider_cli_executed": session.dispatch_provider_cli_executed,
        "dispatch_credential_scan_status": session.dispatch_credential_scan_status,
        "dispatch_raw_prompt_policy": session.dispatch_raw_prompt_policy,
        "dispatch_raw_output_policy": session.dispatch_raw_output_policy,
        "tool_call_count": session.tool_call_count,
        "tool_observation_count": session.tool_observation_count,
        "memory_packet_count": session.memory_packet_count,
    })
}

fn decode_session(value: &Value) -> TransportResult<SessionSummary> {
    Ok(SessionSummary {
        session_id: SessionId::new(required_string(value, "session_id")?),
        status: required_string(value, "status")?,
        run_id: optional_string(value, "run_id")?.map(RunId::new),
        run_status: optional_string(value, "run_status")?,
        adapter_kind: optional_string(value, "adapter_kind")?,
        current_goal: required_string(value, "current_goal")?,
        latest_summary: optional_string(value, "latest_summary")?,
        latest_blocker: optional_string(value, "latest_blocker")?,
        latest_confidence: optional_i64(value, "latest_confidence")?,
        recent_event_count: required_usize(value, "recent_event_count")?,
        evidence_count: required_usize(value, "evidence_count")?,
        evidence_refs: required_string_array(value, "evidence_refs")?,
        review_finding_count: required_usize(value, "review_finding_count")?,
        task_outcome_report_count: required_usize(value, "task_outcome_report_count")?,
        turn_count: required_usize(value, "turn_count")?,
        turn_ids: required_string_array(value, "turn_ids")?,
        latest_dispatch_plan_id: optional_string(value, "latest_dispatch_plan_id")?,
        latest_dispatch_gate_id: optional_string(value, "latest_dispatch_gate_id")?,
        latest_dispatch_execution_id: optional_string(value, "latest_dispatch_execution_id")?,
        dispatch_gate_status: optional_string(value, "dispatch_gate_status")?,
        dispatch_gate_reasons: optional_string(value, "dispatch_gate_reasons")?,
        dispatch_next_action: optional_string(value, "dispatch_next_action")?,
        dispatch_execution_status: optional_string(value, "dispatch_execution_status")?,
        dispatch_runtime_process_ref: optional_string(value, "dispatch_runtime_process_ref")?,
        dispatch_provider_cli_execution_allowed: optional_bool(
            value,
            "dispatch_provider_cli_execution_allowed",
        )?,
        dispatch_provider_cli_executed: optional_bool(value, "dispatch_provider_cli_executed")?,
        dispatch_credential_scan_status: optional_string(value, "dispatch_credential_scan_status")?,
        dispatch_raw_prompt_policy: optional_string(value, "dispatch_raw_prompt_policy")?,
        dispatch_raw_output_policy: optional_string(value, "dispatch_raw_output_policy")?,
        tool_call_count: required_usize(value, "tool_call_count")?,
        tool_observation_count: required_usize(value, "tool_observation_count")?,
        memory_packet_count: required_usize(value, "memory_packet_count")?,
    })
}

fn encode_run(run: &TaskRunSummary) -> Value {
    json!({
        "task_id": run.task_id.to_string(),
        "agent_id": run.agent_id.to_string(),
        "session_id": run.session_id.to_string(),
        "run_id": run.run_id.to_string(),
        "runtime_process_ref": run.runtime_process_ref,
        "external_session_ref": run.external_session_ref,
    })
}

fn decode_run(value: &Value) -> TransportResult<TaskRunSummary> {
    Ok(TaskRunSummary {
        task_id: TaskId::new(required_string(value, "task_id")?),
        agent_id: AgentId::new(required_string(value, "agent_id")?),
        session_id: SessionId::new(required_string(value, "session_id")?),
        run_id: RunId::new(required_string(value, "run_id")?),
        runtime_process_ref: required_string(value, "runtime_process_ref")?,
        external_session_ref: required_string(value, "external_session_ref")?,
    })
}

fn decode_agents(value: &Value, key: &str) -> TransportResult<Vec<AgentSummary>> {
    let agents = value
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| TransportError::Protocol(format!("missing {key} array")))?;
    agents.iter().map(decode_agent).collect()
}
