//! Server/control-plane boundary for Capo.
//!
//! This crate owns the typed request/response surface that clients should use
//! before choosing a concrete transport such as a local socket or remote API.

use std::path::Path;

use capo_controller::{FakeBoundaryController, FakeRunRefs};
use capo_core::{
    AgentId, CommandEnvelope, CommandId, CommandIntent, CommandTarget, InputOrigin, ProjectId,
    RunId, SessionId, TaskId,
};
use capo_query::{ProjectDashboardQuery, project_dashboard};
use capo_state::{EventKind, NewEvent};

mod transport;

pub use transport::{TransportError, send_tcp, serve_tcp};

#[derive(Clone, Debug)]
pub struct CapoServer {
    project_id: ProjectId,
    controller: FakeBoundaryController,
}

impl CapoServer {
    pub fn open(project_id: ProjectId, state_root: impl AsRef<Path>) -> ServerResult<Self> {
        let controller = FakeBoundaryController::open(project_id.clone(), state_root)
            .map_err(ServerError::State)?;
        Ok(Self {
            project_id,
            controller,
        })
    }

    pub fn handle(&self, request: ServerRequest) -> ServerResult<ServerResponse> {
        let request_id = request.request_id.clone();
        let origin = request.origin.clone();
        match request.command {
            ServerCommand::RegisterAgent { name } => {
                let command = self.command_envelope(
                    &request_id,
                    &origin,
                    CommandTarget::Project(self.project_id.clone()),
                    CommandIntent::RegisterAgent,
                    Some(name),
                );
                let registration = self
                    .controller
                    .register_agent_command(&command)
                    .map_err(ServerError::State)?;
                self.record_server_request_handled(&command, &origin, "register_agent", None)
                    .map_err(ServerError::State)?;
                self.response(
                    request_id,
                    origin,
                    ServerResponsePayload::AgentRegistered(AgentSummary {
                        agent_id: registration.agent_id,
                        name: registration.agent_name,
                        status: "available".to_string(),
                        current_session_id: None,
                        session: None,
                    }),
                )
            }
            ServerCommand::SendTask {
                agent_name,
                goal,
                scenario,
            } => {
                if let Some(agent) = self.agent_by_name(&agent_name)? {
                    if let Some(session) = agent.session {
                        return Err(ServerError::AgentAlreadyHasSession {
                            agent_name,
                            session_id: session.session_id.to_string(),
                            run_status: session.run_status,
                        });
                    }
                } else {
                    return Err(ServerError::UnknownAgent { agent_name });
                }
                let mut command = self.command_envelope(
                    &request_id,
                    &origin,
                    CommandTarget::Agent(AgentId::new(format!("agent-{agent_name}"))),
                    CommandIntent::SendTask,
                    Some(goal),
                );
                command
                    .structured_args
                    .push(("agent".to_string(), agent_name));
                command
                    .structured_args
                    .push(("scenario".to_string(), scenario));
                let run = self
                    .controller
                    .send_task_command(&command)
                    .map_err(ServerError::State)?;
                self.record_server_request_handled(&command, &origin, "send_task", Some(&run))
                    .map_err(ServerError::State)?;
                self.response(
                    request_id,
                    origin,
                    ServerResponsePayload::TaskSent(TaskRunSummary::from_run_refs(run)),
                )
            }
            ServerCommand::ListAgents => {
                let agents = self.dashboard_snapshot()?.agents;
                self.response(request_id, origin, ServerResponsePayload::Agents(agents))
            }
            ServerCommand::AgentStatus { agent_name } => {
                let agent = self
                    .dashboard_snapshot()?
                    .agents
                    .into_iter()
                    .find(|agent| agent.name == agent_name)
                    .ok_or(ServerError::UnknownAgent { agent_name })?;
                self.response(
                    request_id,
                    origin,
                    ServerResponsePayload::AgentStatus(agent),
                )
            }
            ServerCommand::Dashboard { recent_event_limit } => self.response(
                request_id,
                origin,
                ServerResponsePayload::Dashboard(self.dashboard_with_limit(recent_event_limit)?),
            ),
            ServerCommand::Recover => {
                let command = self.command_envelope(
                    &request_id,
                    &origin,
                    CommandTarget::Project(self.project_id.clone()),
                    CommandIntent::Recover,
                    None,
                );
                let report = self
                    .controller
                    .recover_command(&command)
                    .map_err(ServerError::State)?;
                self.record_server_request_handled(&command, &origin, "recover", None)
                    .map_err(ServerError::State)?;
                self.response(
                    request_id,
                    origin,
                    ServerResponsePayload::Recovery(RecoverySummary {
                        recovery_attempt_id: report.recovery_attempt_id,
                        recovered_run_count: report.recovered_run_count,
                        watermark: report.watermark,
                    }),
                )
            }
        }
    }

    pub fn dashboard_snapshot(&self) -> ServerResult<ServerDashboardSnapshot> {
        self.dashboard_with_limit(5)
    }

    fn dashboard_with_limit(
        &self,
        recent_event_limit: usize,
    ) -> ServerResult<ServerDashboardSnapshot> {
        let mut query = ProjectDashboardQuery::new(self.project_id.clone());
        query.recent_event_limit = recent_event_limit;
        let dashboard =
            project_dashboard(self.controller.state(), query).map_err(ServerError::State)?;
        let agents = dashboard
            .agents
            .into_iter()
            .map(|row| {
                let session = row.session.map(|session| SessionSummary {
                    session_id: session.session.session_id,
                    status: session.session.status,
                    run_id: session.run.as_ref().map(|run| run.run_id.clone()),
                    run_status: session.run.map(|run| run.status),
                    recent_event_count: session.recent_events.len(),
                    tool_call_count: session.tool_calls.len(),
                    tool_observation_count: session.tool_observations.len(),
                    memory_packet_count: session.memory_packets.len(),
                });
                AgentSummary {
                    agent_id: row.agent.agent_id,
                    name: row.agent.name,
                    status: row.agent.status,
                    current_session_id: row.agent.current_session_id,
                    session: None,
                }
                .with_session(session)
            })
            .collect::<Vec<_>>();
        Ok(ServerDashboardSnapshot {
            project_id: dashboard.project_id,
            agent_count: agents.len(),
            active_session_count: agents
                .iter()
                .filter(|agent| {
                    agent
                        .session
                        .as_ref()
                        .map(|session| session.run_status == Some("running".to_string()))
                        .unwrap_or(false)
                })
                .count(),
            agents,
        })
    }

    fn agent_by_name(&self, agent_name: &str) -> ServerResult<Option<AgentSummary>> {
        Ok(self
            .dashboard_snapshot()?
            .agents
            .into_iter()
            .find(|agent| agent.name == agent_name))
    }

    fn command_envelope(
        &self,
        request_id: &str,
        origin: &ServerClientOrigin,
        target: CommandTarget,
        intent: CommandIntent,
        text: Option<String>,
    ) -> CommandEnvelope {
        let mut command = CommandEnvelope::new(
            CommandId::new(request_id),
            origin.input_origin.into(),
            origin.actor_id.clone(),
            self.project_id.clone(),
            target,
            intent,
        );
        command.idempotency_key = format!(
            "server:{}:{}:{}",
            origin.client_id, origin.actor_id, request_id
        );
        if let Some(text) = text {
            command = command.with_text(text);
        }
        command
    }

    fn response(
        &self,
        request_id: String,
        origin: ServerClientOrigin,
        payload: ServerResponsePayload,
    ) -> ServerResult<ServerResponse> {
        Ok(ServerResponse {
            request_id,
            client_id: origin.client_id,
            actor_id: origin.actor_id,
            input_origin: origin.input_origin,
            payload,
        })
    }

    fn record_server_request_handled(
        &self,
        command: &CommandEnvelope,
        origin: &ServerClientOrigin,
        command_kind: &str,
        run: Option<&FakeRunRefs>,
    ) -> capo_state::StateResult<i64> {
        let event_id = format!(
            "event-server-request-{}-{}",
            slug(command.command_id.as_str()),
            stable_hash(command.idempotency_key.as_bytes())
        );
        let mut event = NewEvent::new(event_id, EventKind::ServerRequestHandled, &origin.actor_id);
        event.project_id = Some(self.project_id.clone());
        event.item_id = Some(command.command_id.to_string());
        event.idempotency_key = Some(command.idempotency_key.clone());
        if let Some(run) = run {
            event.task_id = Some(run.task_id.clone());
            event.agent_id = Some(run.agent_id.clone());
            event.session_id = Some(run.session_id.clone());
            event.run_id = Some(run.run_id.clone());
        }
        event.payload_json = serde_json::json!({
            "request_id": command.command_id.to_string(),
            "client_id": origin.client_id,
            "actor_id": origin.actor_id,
            "input_origin": format!("{:?}", origin.input_origin),
            "command_kind": command_kind,
            "idempotency_key": command.idempotency_key,
        })
        .to_string();
        self.controller.state().append_event(event, &[])
    }
}

pub type ServerResult<T> = Result<T, ServerError>;

#[derive(Debug)]
pub enum ServerError {
    State(capo_state::StateError),
    UnknownAgent {
        agent_name: String,
    },
    AgentAlreadyHasSession {
        agent_name: String,
        session_id: String,
        run_status: Option<String>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerRequest {
    pub request_id: String,
    pub origin: ServerClientOrigin,
    pub command: ServerCommand,
}

impl ServerRequest {
    pub fn cli(command: ServerCommand) -> Self {
        Self::local_cli(default_request_id(&command), command)
    }

    pub fn local_cli(request_id: impl Into<String>, command: ServerCommand) -> Self {
        Self {
            request_id: request_id.into(),
            origin: ServerClientOrigin {
                client_id: "local-cli".to_string(),
                actor_id: "local-user".to_string(),
                input_origin: ServerInputOrigin::Cli,
            },
            command,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerClientOrigin {
    pub client_id: String,
    pub actor_id: String,
    pub input_origin: ServerInputOrigin,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServerInputOrigin {
    Cli,
    Dashboard,
    Mobile,
    Voice,
    Api,
    System,
}

impl From<ServerInputOrigin> for InputOrigin {
    fn from(value: ServerInputOrigin) -> Self {
        match value {
            ServerInputOrigin::Cli => Self::Cli,
            ServerInputOrigin::Dashboard => Self::Dashboard,
            ServerInputOrigin::Mobile => Self::Mobile,
            ServerInputOrigin::Voice => Self::Voice,
            ServerInputOrigin::Api => Self::Api,
            ServerInputOrigin::System => Self::System,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ServerCommand {
    RegisterAgent {
        name: String,
    },
    SendTask {
        agent_name: String,
        goal: String,
        scenario: String,
    },
    ListAgents,
    AgentStatus {
        agent_name: String,
    },
    Dashboard {
        recent_event_limit: usize,
    },
    Recover,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerResponse {
    pub request_id: String,
    pub client_id: String,
    pub actor_id: String,
    pub input_origin: ServerInputOrigin,
    pub payload: ServerResponsePayload,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ServerResponsePayload {
    AgentRegistered(AgentSummary),
    TaskSent(TaskRunSummary),
    Agents(Vec<AgentSummary>),
    AgentStatus(AgentSummary),
    Dashboard(ServerDashboardSnapshot),
    Recovery(RecoverySummary),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerDashboardSnapshot {
    pub project_id: ProjectId,
    pub agent_count: usize,
    pub active_session_count: usize,
    pub agents: Vec<AgentSummary>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentSummary {
    pub agent_id: AgentId,
    pub name: String,
    pub status: String,
    pub current_session_id: Option<SessionId>,
    pub session: Option<SessionSummary>,
}

impl AgentSummary {
    fn with_session(mut self, session: Option<SessionSummary>) -> Self {
        self.session = session;
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionSummary {
    pub session_id: SessionId,
    pub status: String,
    pub run_id: Option<RunId>,
    pub run_status: Option<String>,
    pub recent_event_count: usize,
    pub tool_call_count: usize,
    pub tool_observation_count: usize,
    pub memory_packet_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskRunSummary {
    pub task_id: TaskId,
    pub agent_id: AgentId,
    pub session_id: SessionId,
    pub run_id: RunId,
    pub runtime_process_ref: String,
    pub external_session_ref: String,
}

impl TaskRunSummary {
    fn from_run_refs(run: FakeRunRefs) -> Self {
        Self {
            task_id: run.task_id,
            agent_id: run.agent_id,
            session_id: run.session_id,
            run_id: run.run_id,
            runtime_process_ref: run.runtime_process_ref,
            external_session_ref: run.external_session_ref,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoverySummary {
    pub recovery_attempt_id: String,
    pub recovered_run_count: usize,
    pub watermark: Option<i64>,
}

fn default_request_id(command: &ServerCommand) -> String {
    match command {
        ServerCommand::RegisterAgent { name } => {
            format!("server-agent-register-{}", slug(name))
        }
        ServerCommand::SendTask {
            agent_name, goal, ..
        } => {
            format!("server-task-send-{}-{}", slug(agent_name), slug(goal))
        }
        ServerCommand::ListAgents => "server-agent-list".to_string(),
        ServerCommand::AgentStatus { agent_name } => {
            format!("server-agent-status-{}", slug(agent_name))
        }
        ServerCommand::Dashboard { .. } => "server-dashboard".to_string(),
        ServerCommand::Recover => "server-recover".to_string(),
    }
}

fn slug(value: &str) -> String {
    let mut slug = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    slug.trim_matches('-').to_string()
}

fn stable_hash(value: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in value {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests;
