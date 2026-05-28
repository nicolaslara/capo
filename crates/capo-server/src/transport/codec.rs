use capo_core::{AgentId, ProjectId, RunId, SessionId, TaskId};
use serde_json::{Value, json};

use super::{
    TransportError, TransportResult,
    wire::{
        input_origin_name, optional_bool, optional_i64, optional_string, parse_input_origin,
        parse_value, required_bool, required_string, required_string_array, required_usize,
        required_value, transport_error_wire,
    },
};
use crate::{
    AdapterReplaySummary, AgentSummary, DispatchGateSummary, DispatchPlanSummary,
    DispatchRunSummary, LiveProviderPreflightSummary, RecoverySummary, ServerClientOrigin,
    ServerCommand, ServerRequest, ServerResponse, ServerResponsePayload, SessionSummary,
    TaskRunSummary,
};

pub(super) fn encode_request(request: &ServerRequest) -> String {
    json!({
        "request_id": request.request_id,
        "origin": {
            "client_id": request.origin.client_id,
            "actor_id": request.origin.actor_id,
            "input_origin": input_origin_name(request.origin.input_origin),
        },
        "command": encode_command(&request.command),
    })
    .to_string()
}

pub(super) fn decode_request(line: &str) -> TransportResult<ServerRequest> {
    let value = parse_value(line)?;
    let command = value
        .get("command")
        .ok_or_else(|| TransportError::Protocol("missing command".to_string()))
        .and_then(decode_command)?;
    let origin = value
        .get("origin")
        .ok_or_else(|| TransportError::Protocol("missing origin".to_string()))?;
    Ok(ServerRequest {
        request_id: required_string(&value, "request_id")?,
        origin: ServerClientOrigin {
            client_id: required_string(origin, "client_id")?,
            actor_id: required_string(origin, "actor_id")?,
            input_origin: parse_input_origin(&required_string(origin, "input_origin")?)?,
        },
        command,
    })
}

pub(super) fn encode_success_response(response: &ServerResponse) -> String {
    json!({
        "ok": true,
        "response": {
            "request_id": response.request_id,
            "client_id": response.client_id,
            "actor_id": response.actor_id,
            "input_origin": input_origin_name(response.input_origin),
            "payload": encode_payload(&response.payload),
        }
    })
    .to_string()
}

pub(super) fn encode_error_response(error: &TransportError) -> String {
    let (kind, message) = transport_error_wire(error);
    json!({
        "ok": false,
        "error": {
            "kind": kind,
            "message": message,
        }
    })
    .to_string()
}

pub(super) fn decode_transport_response(line: &str) -> TransportResult<ServerResponse> {
    let value = parse_value(line)?;
    if value.get("ok").and_then(Value::as_bool) == Some(false) {
        let error = value
            .get("error")
            .ok_or_else(|| TransportError::Protocol("missing error".to_string()))?;
        return Err(TransportError::Remote {
            kind: required_string(error, "kind")?,
            message: required_string(error, "message")?,
        });
    }
    let response = value
        .get("response")
        .ok_or_else(|| TransportError::Protocol("missing response".to_string()))?;
    let payload = response
        .get("payload")
        .ok_or_else(|| TransportError::Protocol("missing payload".to_string()))
        .and_then(decode_payload)?;
    Ok(ServerResponse {
        request_id: required_string(response, "request_id")?,
        client_id: required_string(response, "client_id")?,
        actor_id: required_string(response, "actor_id")?,
        input_origin: parse_input_origin(&required_string(response, "input_origin")?)?,
        payload,
    })
}

fn encode_command(command: &ServerCommand) -> Value {
    match command {
        ServerCommand::RegisterAgent { name } => json!({
            "type": "register_agent",
            "name": name,
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
        } => json!({
            "type": "run_live_provider_local",
            "dispatch_plan_id": dispatch_plan_id,
            "goal": goal,
            "live_execution_opt_in": live_execution_opt_in,
            "mock_runtime_opt_in": mock_runtime_opt_in,
            "mock_provider_output_name": mock_provider_output_name,
            "mock_provider_output_jsonl": mock_provider_output_jsonl,
            "timeout_seconds": timeout_seconds,
        }),
        ServerCommand::Recover => json!({ "type": "recover" }),
    }
}

fn decode_command(value: &Value) -> TransportResult<ServerCommand> {
    match required_string(value, "type")?.as_str() {
        "register_agent" => Ok(ServerCommand::RegisterAgent {
            name: required_string(value, "name")?,
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
        }),
        "recover" => Ok(ServerCommand::Recover),
        other => Err(TransportError::Protocol(format!(
            "unknown command type: {other}"
        ))),
    }
}

fn encode_payload(payload: &ServerResponsePayload) -> Value {
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
        ServerResponsePayload::DispatchRun(run) => json!({
            "type": "dispatch_run",
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
        }),
        ServerResponsePayload::Recovery(recovery) => json!({
            "type": "recovery",
            "recovery_attempt_id": recovery.recovery_attempt_id,
            "recovered_run_count": recovery.recovered_run_count,
            "watermark": recovery.watermark,
        }),
    }
}

fn decode_payload(value: &Value) -> TransportResult<ServerResponsePayload> {
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
        "dispatch_run" => Ok(ServerResponsePayload::DispatchRun(DispatchRunSummary {
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
        })),
        "recovery" => Ok(ServerResponsePayload::Recovery(RecoverySummary {
            recovery_attempt_id: required_string(value, "recovery_attempt_id")?,
            recovered_run_count: required_usize(value, "recovered_run_count")?,
            watermark: value.get("watermark").and_then(Value::as_i64),
        })),
        other => Err(TransportError::Protocol(format!(
            "unknown payload type: {other}"
        ))),
    }
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
