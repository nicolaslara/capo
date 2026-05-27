//! Server/control-plane boundary for Capo.
//!
//! This crate owns the typed request/response surface that clients should use
//! before choosing a concrete transport such as a local socket or remote API.

use std::path::Path;

use capo_controller::{FakeBoundaryController, FakeRunRefs};
use capo_core::{AgentId, ProjectId, RunId, SessionId, TaskId};
use capo_query::{ProjectDashboardQuery, project_dashboard};

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
        match request.command {
            ServerCommand::RegisterAgent { name } => {
                let registration = self
                    .controller
                    .register_agent(&name)
                    .map_err(ServerError::State)?;
                Ok(ServerResponse::AgentRegistered(AgentSummary {
                    agent_id: registration.agent_id,
                    name: registration.agent_name,
                    status: "available".to_string(),
                    current_session_id: None,
                    session: None,
                }))
            }
            ServerCommand::SendTask { agent_name, goal } => {
                let run = self
                    .controller
                    .send_task_to_agent_name(&agent_name, &goal)
                    .map_err(ServerError::State)?;
                Ok(ServerResponse::TaskSent(TaskRunSummary::from_run_refs(run)))
            }
            ServerCommand::ListAgents => {
                Ok(ServerResponse::Agents(self.dashboard_snapshot()?.agents))
            }
            ServerCommand::Dashboard { recent_event_limit } => Ok(ServerResponse::Dashboard(
                self.dashboard_with_limit(recent_event_limit)?,
            )),
            ServerCommand::Recover => {
                let report = self
                    .controller
                    .recover_command(&capo_core::CommandEnvelope::new(
                        capo_core::CommandId::new("command-server-recover"),
                        capo_core::InputOrigin::Api,
                        request.origin.actor_id,
                        self.project_id.clone(),
                        capo_core::CommandTarget::Project(self.project_id.clone()),
                        capo_core::CommandIntent::Recover,
                    ))
                    .map_err(ServerError::State)?;
                Ok(ServerResponse::Recovery(RecoverySummary {
                    recovery_attempt_id: report.recovery_attempt_id,
                    recovered_run_count: report.recovered_run_count,
                    watermark: report.watermark,
                }))
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
}

pub type ServerResult<T> = Result<T, ServerError>;

#[derive(Debug)]
pub enum ServerError {
    State(capo_state::StateError),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerRequest {
    pub origin: ServerClientOrigin,
    pub command: ServerCommand,
}

impl ServerRequest {
    pub fn cli(command: ServerCommand) -> Self {
        Self {
            origin: ServerClientOrigin {
                client_id: "local-cli".to_string(),
                actor_id: "local-user".to_string(),
            },
            command,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerClientOrigin {
    pub client_id: String,
    pub actor_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ServerCommand {
    RegisterAgent { name: String },
    SendTask { agent_name: String, goal: String },
    ListAgents,
    Dashboard { recent_event_limit: usize },
    Recover,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ServerResponse {
    AgentRegistered(AgentSummary),
    TaskSent(TaskRunSummary),
    Agents(Vec<AgentSummary>),
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

#[cfg(test)]
mod tests;
