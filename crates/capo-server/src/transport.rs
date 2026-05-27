use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::path::Path;

use capo_core::{AgentId, ProjectId, RunId, SessionId, TaskId};
use serde_json::{Value, json};

use crate::{
    AgentSummary, CapoServer, RecoverySummary, ServerClientOrigin, ServerCommand, ServerError,
    ServerInputOrigin, ServerRequest, ServerResponse, ServerResponsePayload, SessionSummary,
    TaskRunSummary,
};

pub fn serve_tcp(
    listener: TcpListener,
    project_id: ProjectId,
    state_root: impl AsRef<Path>,
    max_requests: Option<usize>,
) -> TransportResult<usize> {
    let server = CapoServer::open(project_id, state_root).map_err(TransportError::Server)?;
    let mut served = 0;
    while max_requests.map(|max| served < max).unwrap_or(true) {
        let (stream, _) = listener.accept().map_err(TransportError::Io)?;
        handle_stream(&server, stream)?;
        served += 1;
    }
    Ok(served)
}

pub fn send_tcp(
    address: impl ToSocketAddrs,
    request: &ServerRequest,
) -> TransportResult<ServerResponse> {
    let mut stream = TcpStream::connect(address).map_err(TransportError::Io)?;
    let request_json = encode_request(request);
    stream
        .write_all(request_json.as_bytes())
        .and_then(|_| stream.write_all(b"\n"))
        .and_then(|_| stream.flush())
        .map_err(TransportError::Io)?;
    let mut line = String::new();
    BufReader::new(stream)
        .read_line(&mut line)
        .map_err(TransportError::Io)?;
    decode_transport_response(&line)
}

pub type TransportResult<T> = Result<T, TransportError>;

#[derive(Debug)]
pub enum TransportError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Protocol(String),
    Server(ServerError),
    Remote { kind: String, message: String },
}

fn handle_stream(server: &CapoServer, mut stream: TcpStream) -> TransportResult<()> {
    let mut line = String::new();
    {
        let mut reader = BufReader::new(&mut stream);
        reader.read_line(&mut line).map_err(TransportError::Io)?;
    }
    let response_line = match decode_request(&line).and_then(|request| {
        server
            .handle(request)
            .map_err(TransportError::Server)
            .map(|response| encode_success_response(&response))
    }) {
        Ok(response) => response,
        Err(error) => encode_error_response(&error),
    };
    stream
        .write_all(response_line.as_bytes())
        .and_then(|_| stream.write_all(b"\n"))
        .and_then(|_| stream.flush())
        .map_err(TransportError::Io)
}

fn encode_request(request: &ServerRequest) -> String {
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

fn decode_request(line: &str) -> TransportResult<ServerRequest> {
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

fn encode_success_response(response: &ServerResponse) -> String {
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

fn encode_error_response(error: &TransportError) -> String {
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

fn decode_transport_response(line: &str) -> TransportResult<ServerResponse> {
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
        ServerCommand::ListAgents => json!({ "type": "list_agents" }),
        ServerCommand::AgentStatus { agent_name } => json!({
            "type": "agent_status",
            "agent_name": agent_name,
        }),
        ServerCommand::Dashboard { recent_event_limit } => json!({
            "type": "dashboard",
            "recent_event_limit": recent_event_limit,
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
        "list_agents" => Ok(ServerCommand::ListAgents),
        "agent_status" => Ok(ServerCommand::AgentStatus {
            agent_name: required_string(value, "agent_name")?,
        }),
        "dashboard" => Ok(ServerCommand::Dashboard {
            recent_event_limit: required_usize(value, "recent_event_limit")?,
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
        "recent_event_count": session.recent_event_count,
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
        recent_event_count: required_usize(value, "recent_event_count")?,
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

fn input_origin_name(origin: ServerInputOrigin) -> &'static str {
    match origin {
        ServerInputOrigin::Cli => "cli",
        ServerInputOrigin::Dashboard => "dashboard",
        ServerInputOrigin::Mobile => "mobile",
        ServerInputOrigin::Voice => "voice",
        ServerInputOrigin::Api => "api",
        ServerInputOrigin::System => "system",
    }
}

fn parse_input_origin(value: &str) -> TransportResult<ServerInputOrigin> {
    match value {
        "cli" => Ok(ServerInputOrigin::Cli),
        "dashboard" => Ok(ServerInputOrigin::Dashboard),
        "mobile" => Ok(ServerInputOrigin::Mobile),
        "voice" => Ok(ServerInputOrigin::Voice),
        "api" => Ok(ServerInputOrigin::Api),
        "system" => Ok(ServerInputOrigin::System),
        other => Err(TransportError::Protocol(format!(
            "unknown input origin: {other}"
        ))),
    }
}

fn parse_value(line: &str) -> TransportResult<Value> {
    serde_json::from_str(line).map_err(TransportError::Json)
}

fn required_value<'a>(value: &'a Value, key: &str) -> TransportResult<&'a Value> {
    value
        .get(key)
        .ok_or_else(|| TransportError::Protocol(format!("missing {key}")))
}

fn required_string(value: &Value, key: &str) -> TransportResult<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .ok_or_else(|| TransportError::Protocol(format!("missing {key} string")))
}

fn optional_string(value: &Value, key: &str) -> TransportResult<Option<String>> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_str()
            .map(|value| Some(value.to_string()))
            .ok_or_else(|| TransportError::Protocol(format!("{key} must be a string"))),
    }
}

fn required_usize(value: &Value, key: &str) -> TransportResult<usize> {
    let number = value
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| TransportError::Protocol(format!("missing {key} integer")))?;
    usize::try_from(number).map_err(|_| TransportError::Protocol(format!("{key} is too large")))
}

fn transport_error_wire(error: &TransportError) -> (&'static str, String) {
    match error {
        TransportError::Io(error) => ("io", error.to_string()),
        TransportError::Json(error) => ("json", error.to_string()),
        TransportError::Protocol(message) => ("protocol", message.clone()),
        TransportError::Server(error) => server_error_wire(error),
        TransportError::Remote { kind, message } => ("remote", format!("{kind}: {message}")),
    }
}

fn server_error_wire(error: &ServerError) -> (&'static str, String) {
    match error {
        ServerError::State(error) => ("state", format!("{error:?}")),
        ServerError::UnknownAgent { agent_name } => {
            ("unknown_agent", format!("unknown agent: {agent_name}"))
        }
        ServerError::AgentAlreadyHasSession {
            agent_name,
            session_id,
            run_status,
        } => (
            "agent_already_has_session",
            format!(
                "agent {agent_name} already has session {session_id} with run_status={}",
                run_status.as_deref().unwrap_or("none")
            ),
        ),
    }
}
