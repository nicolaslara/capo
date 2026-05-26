//! Controller orchestration for Capo.
//!
//! P3 keeps this intentionally fake-only, but the control flow is real: the
//! controller calls each boundary, persists Capo-owned events/projections, and
//! answers inspection requests from SQLite read models.

use std::path::{Path, PathBuf};

use capo_adapters::{
    AdapterToolObservation, AgentAdapter, ClaudeCodeAdapter, CodexExecAdapter,
    FakeAdapterSessionRequest, FakeAdapterTurnRequest, LocalAdapterLaunchPlan,
    NormalizedAdapterEvent, ProviderConnector, ScriptedMockAgent, ScriptedMockTurn,
};
use capo_core::{
    AgentId, CommandEnvelope, CommandIntent, EvidenceId, MemoryPacketId, ProjectId, RunId,
    SessionId, TaskId, ToolCallId, TurnId,
};
use capo_memory::{
    MemoryBackend, MemoryCandidate, MemoryReviewState, MemorySensitivity, MemorySourceKind,
    MemorySourceRef, SourceLinkedMemoryPacketRequest,
};
use capo_runtime::{FakeRuntimeStartRequest, RuntimeRunner};
use capo_state::{
    AgentProjection, ArtifactRecord, EventKind, EventRecord, NewEvent, ProjectProjection,
    ProjectionRecord, RedactionState, RunProjection, SessionProjection, SqliteStateStore,
    StateError, StateResult, TaskProjection,
};
use capo_tools::{
    FakeToolRequest, PermissionDecision, PermissionPolicy, PermissionRequest, ToolExposure,
};

mod adapter_replay;
mod fake_session;
mod local_dispatch;

pub use local_dispatch::LocalAdapterDispatchRunStart;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeBoundaryController {
    project_id: ProjectId,
    state: SqliteStateStore,
    adapter: AgentAdapter,
    runtime: RuntimeRunner,
    provider: ProviderConnector,
    permission_policy: PermissionPolicy,
    tools: ToolExposure,
    memory: MemoryBackend,
}

impl FakeBoundaryController {
    pub fn open(project_id: ProjectId, state_root: impl AsRef<Path>) -> StateResult<Self> {
        Self::open_with_permission_policy(
            project_id,
            state_root,
            PermissionPolicy::allow_trusted_local(),
        )
    }

    pub fn open_with_permission_policy(
        project_id: ProjectId,
        state_root: impl AsRef<Path>,
        permission_policy: PermissionPolicy,
    ) -> StateResult<Self> {
        Ok(Self {
            project_id,
            state: SqliteStateStore::open(state_root)?,
            adapter: AgentAdapter::fake(),
            runtime: RuntimeRunner::fake(),
            provider: ProviderConnector::fake(),
            permission_policy,
            tools: ToolExposure::fake(),
            memory: MemoryBackend::fake(),
        })
    }

    pub fn state(&self) -> &SqliteStateStore {
        &self.state
    }

    pub fn initialize(&self, command: &CommandEnvelope) -> StateResult<ControllerInit> {
        require_intent(command, CommandIntent::InitializeProject);
        Ok(ControllerInit {
            command_id: command.command_id.to_string(),
            state_db_path: self.state.db_path().display().to_string(),
        })
    }

    pub fn register_agent_command(
        &self,
        command: &CommandEnvelope,
    ) -> StateResult<FakeAgentRegistration> {
        require_intent(command, CommandIntent::RegisterAgent);
        let name = command
            .text
            .as_deref()
            .ok_or_else(|| missing_read_model("command.text", &command.command_id))?;
        self.register_agent(name)
    }

    pub fn spawn_agent_command(
        &self,
        command: &CommandEnvelope,
    ) -> StateResult<FakeAgentRegistration> {
        require_intent(command, CommandIntent::RegisterAgent);
        let name = command
            .text
            .as_deref()
            .ok_or_else(|| missing_read_model("command.text", &command.command_id))?;
        self.register_agent(name)
    }

    pub fn send_task_command(&self, command: &CommandEnvelope) -> StateResult<FakeRunRefs> {
        require_intent(command, CommandIntent::SendTask);
        let agent_name = required_structured_arg(command, "agent")?;
        let goal = command
            .text
            .as_deref()
            .ok_or_else(|| missing_read_model("command.text", &command.command_id))?;
        if let Some(task_id) = optional_structured_arg(command, "task_id") {
            self.send_task_to_agent_name_with_task_id(agent_name, TaskId::new(task_id), goal)
        } else {
            self.send_task_to_agent_name(agent_name, goal)
        }
    }

    pub fn redirect_command(
        &self,
        command: &CommandEnvelope,
    ) -> StateResult<FakeReadModelObservation> {
        require_intent(command, CommandIntent::RedirectSession);
        let agent_name = required_structured_arg(command, "agent")?;
        let goal = command
            .text
            .as_deref()
            .ok_or_else(|| missing_read_model("command.text", &command.command_id))?;
        self.redirect_agent_name(agent_name, goal)
    }

    pub fn interrupt_command(
        &self,
        command: &CommandEnvelope,
    ) -> StateResult<FakeReadModelObservation> {
        require_intent(command, CommandIntent::InterruptSession);
        let agent_name = required_structured_arg(command, "agent")?;
        let reason = command.text.as_deref().unwrap_or("interrupt requested");
        self.interrupt_agent_name(agent_name, reason)
    }

    pub fn stop_command(&self, command: &CommandEnvelope) -> StateResult<FakeReadModelObservation> {
        require_intent(command, CommandIntent::InterruptSession);
        let agent_name = required_structured_arg(command, "agent")?;
        let reason = command.text.as_deref().unwrap_or("stop requested");
        self.stop_agent_name(agent_name, reason)
    }

    pub fn recover_command(&self, command: &CommandEnvelope) -> StateResult<RecoveryReport> {
        require_intent(command, CommandIntent::Recover);
        let recovery_attempt_id = format!(
            "recovery-{}-after-{}",
            command.command_id,
            self.state.last_sequence()?
        );
        let started = self.state.begin_recovery(&recovery_attempt_id)?;
        self.state.rebuild_projections()?;
        let recovered_runs = self
            .state
            .mark_active_runs_exited_unknown(&self.project_id, &recovery_attempt_id)?;
        let completed = self.state.complete_recovery(&recovery_attempt_id)?;
        Ok(RecoveryReport {
            recovery_attempt_id,
            started_sequence: started.started_sequence,
            completed_sequence: completed.completed_sequence.unwrap_or_default(),
            watermark: self.state.watermark("default")?,
            recovered_run_count: recovered_runs.len(),
        })
    }

    pub fn register_agent(&self, agent_name: &str) -> StateResult<FakeAgentRegistration> {
        let agent_id = AgentId::new(format!("agent-{agent_name}"));
        let provider = self.provider.describe_provider();

        self.state.append_event(
            event(
                &format!("event-register-{}", registration_slug(agent_name)),
                EventKind::AgentRegistered,
                &self.project_id,
            )
            .with_payload(format!(
                "{{\"provider_kind\":\"{}\",\"auth_mode\":\"{}\"}}",
                provider.provider_kind, provider.auth_mode
            )),
            &[
                ProjectionRecord::Project(ProjectProjection {
                    project_id: self.project_id.clone(),
                    name: "Capo".to_string(),
                    status: "active".to_string(),
                    updated_sequence: 0,
                }),
                ProjectionRecord::Agent(AgentProjection {
                    agent_id: agent_id.clone(),
                    project_id: self.project_id.clone(),
                    name: agent_name.to_string(),
                    status: "available".to_string(),
                    current_session_id: None,
                    updated_sequence: 0,
                }),
            ],
        )?;

        Ok(FakeAgentRegistration {
            agent_id,
            agent_name: agent_name.to_string(),
        })
    }

    pub fn registration_for_agent_name(
        &self,
        agent_name: &str,
    ) -> StateResult<FakeAgentRegistration> {
        let agent = self
            .state
            .agent_by_name(agent_name)?
            .ok_or_else(|| missing_read_model("agent.name", &agent_name))?;
        Ok(FakeAgentRegistration {
            agent_id: agent.agent_id,
            agent_name: agent.name,
        })
    }

    pub fn send_task_to_agent_name(
        &self,
        agent_name: &str,
        goal: &str,
    ) -> StateResult<FakeRunRefs> {
        let registration = self.registration_for_agent_name(agent_name)?;
        self.send_task(&registration, goal)
    }

    pub fn send_task_to_agent_name_with_task_id(
        &self,
        agent_name: &str,
        task_id: TaskId,
        goal: &str,
    ) -> StateResult<FakeRunRefs> {
        let registration = self.registration_for_agent_name(agent_name)?;
        self.send_task_with_task_id(&registration, task_id, goal)
    }

    pub fn refs_for_agent_name(&self, agent_name: &str) -> StateResult<FakeRunRefs> {
        let agent = self
            .state
            .agent_by_name(agent_name)?
            .ok_or_else(|| missing_read_model("agent.name", &agent_name))?;
        let session_id = agent
            .current_session_id
            .clone()
            .ok_or_else(|| missing_read_model("agent.current_session_id", &agent.agent_id))?;
        let session = self
            .state
            .session(&session_id)?
            .ok_or_else(|| missing_read_model("session", &session_id))?;
        let run = self
            .state
            .run_for_session(&session_id)?
            .ok_or_else(|| missing_read_model("run.session_id", &session_id))?;
        Ok(FakeRunRefs {
            task_id: session
                .task_id
                .ok_or_else(|| missing_read_model("session.task_id", &session_id))?,
            agent_id: agent.agent_id,
            session_id,
            run_id: run.run_id,
            runtime_process_ref: format!("fake-runtime-process-{agent_name}"),
            external_session_ref: format!("fake-adapter-session-{agent_name}"),
        })
    }

    pub fn observe_agent_name(&self, agent_name: &str) -> StateResult<FakeReadModelObservation> {
        let refs = self.refs_for_agent_name(agent_name)?;
        self.observe(&refs)
    }

    pub fn redirect_agent_name(
        &self,
        agent_name: &str,
        goal: &str,
    ) -> StateResult<FakeReadModelObservation> {
        let registration = self.registration_for_agent_name(agent_name)?;
        let refs = self.refs_for_agent_name(agent_name)?;
        self.redirect(&registration, &refs, goal)
    }

    pub fn redirect(
        &self,
        registration: &FakeAgentRegistration,
        refs: &FakeRunRefs,
        goal: &str,
    ) -> StateResult<FakeReadModelObservation> {
        let session = self
            .state
            .session(&refs.session_id)?
            .ok_or_else(|| missing_read_model("session", &refs.session_id))?;
        let task = self
            .state
            .task(&refs.task_id)?
            .ok_or_else(|| missing_read_model("task", &refs.task_id))?;
        let adapter_session = self
            .adapter
            .attach_session(refs.session_id.clone(), refs.external_session_ref.clone());
        let turn_id = TurnId::new(format!("redirect-{}", refs.session_id));
        let adapter_output = self.adapter.send_turn(
            &adapter_session,
            FakeAdapterTurnRequest {
                turn_id: turn_id.clone(),
                agent_name: registration.agent_name.clone(),
                goal: goal.to_string(),
            },
        );

        self.state.append_event(
            scoped_event(
                &format!(
                    "event-session-redirected-{}-{}",
                    refs.session_id,
                    stable_hash(goal.as_bytes())
                ),
                EventKind::SessionRedirected,
                &self.project_id,
                &refs.task_id,
                &registration.agent_id,
                &refs.session_id,
                &refs.run_id,
            )
            .with_turn(format!("{turn_id}-{}", stable_hash(goal.as_bytes())))
            .with_payload(format!(
                "{{\"goal\":\"{}\",\"adapter_summary\":\"{}\"}}",
                escape_json(goal),
                escape_json(&adapter_output.summary)
            )),
            &[
                ProjectionRecord::Task(TaskProjection {
                    task_id: refs.task_id.clone(),
                    project_id: self.project_id.clone(),
                    title: session.title.clone(),
                    capo_execution_status: "active".to_string(),
                    active_session_id: Some(refs.session_id.clone()),
                    latest_summary: Some(adapter_output.summary.clone()),
                    evidence_id: task.evidence_id,
                    updated_sequence: 0,
                }),
                ProjectionRecord::Agent(AgentProjection {
                    agent_id: refs.agent_id.clone(),
                    project_id: self.project_id.clone(),
                    name: registration.agent_name.clone(),
                    status: "running".to_string(),
                    current_session_id: Some(refs.session_id.clone()),
                    updated_sequence: 0,
                }),
                ProjectionRecord::Session(SessionProjection {
                    session_id: refs.session_id.clone(),
                    project_id: self.project_id.clone(),
                    task_id: Some(refs.task_id.clone()),
                    agent_id: refs.agent_id.clone(),
                    title: session.title,
                    status: adapter_output.status,
                    current_goal: goal.to_string(),
                    latest_summary: Some(adapter_output.summary),
                    latest_confidence: Some(78),
                    latest_blocker: None,
                    updated_sequence: 0,
                }),
            ],
        )?;

        self.observe(refs)
    }

    pub fn interrupt_agent_name(
        &self,
        agent_name: &str,
        reason: &str,
    ) -> StateResult<FakeReadModelObservation> {
        let registration = self.registration_for_agent_name(agent_name)?;
        let refs = self.refs_for_agent_name(agent_name)?;
        self.interrupt(&registration, &refs, reason)
    }

    pub fn stop_agent_name(
        &self,
        agent_name: &str,
        reason: &str,
    ) -> StateResult<FakeReadModelObservation> {
        let registration = self.registration_for_agent_name(agent_name)?;
        let refs = self.refs_for_agent_name(agent_name)?;
        self.stop(&registration, &refs, reason)
    }

    pub fn interrupt(
        &self,
        registration: &FakeAgentRegistration,
        refs: &FakeRunRefs,
        reason: &str,
    ) -> StateResult<FakeReadModelObservation> {
        let session = self
            .state
            .session(&refs.session_id)?
            .ok_or_else(|| missing_read_model("session", &refs.session_id))?;
        let task = self
            .state
            .task(&refs.task_id)?
            .ok_or_else(|| missing_read_model("task", &refs.task_id))?;
        let runtime_process = self
            .runtime
            .attach_process(refs.run_id.clone(), refs.runtime_process_ref.clone());
        let interrupted_process = self.runtime.interrupt(&runtime_process, reason);
        let adapter_session = self
            .adapter
            .attach_session(refs.session_id.clone(), refs.external_session_ref.clone());
        let adapter_output = self.adapter.interrupt(&adapter_session, reason);

        self.state.append_event(
            scoped_event(
                &format!("event-session-interrupted-{}", refs.session_id),
                EventKind::SessionInterrupted,
                &self.project_id,
                &refs.task_id,
                &registration.agent_id,
                &refs.session_id,
                &refs.run_id,
            )
            .with_payload(format!(
                "{{\"reason\":\"{}\",\"adapter_summary\":\"{}\"}}",
                escape_json(reason),
                escape_json(&adapter_output.summary)
            )),
            &[
                ProjectionRecord::Task(TaskProjection {
                    task_id: refs.task_id.clone(),
                    project_id: self.project_id.clone(),
                    title: session.title.clone(),
                    capo_execution_status: "canceled".to_string(),
                    active_session_id: Some(refs.session_id.clone()),
                    latest_summary: Some(adapter_output.summary),
                    evidence_id: task.evidence_id,
                    updated_sequence: 0,
                }),
                ProjectionRecord::Agent(AgentProjection {
                    agent_id: refs.agent_id.clone(),
                    project_id: self.project_id.clone(),
                    name: registration.agent_name.clone(),
                    status: "available".to_string(),
                    current_session_id: None,
                    updated_sequence: 0,
                }),
                ProjectionRecord::Session(SessionProjection {
                    session_id: refs.session_id.clone(),
                    project_id: self.project_id.clone(),
                    task_id: Some(refs.task_id.clone()),
                    agent_id: refs.agent_id.clone(),
                    title: session.title,
                    status: "canceled".to_string(),
                    current_goal: session.current_goal,
                    latest_summary: Some(format!("Interrupted: {reason}")),
                    latest_confidence: Some(70),
                    latest_blocker: None,
                    updated_sequence: 0,
                }),
                ProjectionRecord::Run(RunProjection {
                    run_id: refs.run_id.clone(),
                    session_id: refs.session_id.clone(),
                    status: interrupted_process.status,
                    recovery_of_run_id: None,
                    updated_sequence: 0,
                }),
            ],
        )?;

        self.observe(refs)
    }

    pub fn stop(
        &self,
        registration: &FakeAgentRegistration,
        refs: &FakeRunRefs,
        reason: &str,
    ) -> StateResult<FakeReadModelObservation> {
        let session = self
            .state
            .session(&refs.session_id)?
            .ok_or_else(|| missing_read_model("session", &refs.session_id))?;
        let task = self
            .state
            .task(&refs.task_id)?
            .ok_or_else(|| missing_read_model("task", &refs.task_id))?;
        let runtime_process = self
            .runtime
            .attach_process(refs.run_id.clone(), refs.runtime_process_ref.clone());
        let stopped_process = self.runtime.stop(&runtime_process, reason);
        let adapter_session = self
            .adapter
            .attach_session(refs.session_id.clone(), refs.external_session_ref.clone());
        let adapter_output = self.adapter.stop(&adapter_session, reason);

        self.state.append_event(
            scoped_event(
                &format!("event-session-stopped-{}", refs.session_id),
                EventKind::SessionStopped,
                &self.project_id,
                &refs.task_id,
                &registration.agent_id,
                &refs.session_id,
                &refs.run_id,
            )
            .with_payload(format!(
                "{{\"reason\":\"{}\",\"adapter_summary\":\"{}\"}}",
                escape_json(reason),
                escape_json(&adapter_output.summary)
            )),
            &[
                ProjectionRecord::Task(TaskProjection {
                    task_id: refs.task_id.clone(),
                    project_id: self.project_id.clone(),
                    title: session.title.clone(),
                    capo_execution_status: "completed".to_string(),
                    active_session_id: Some(refs.session_id.clone()),
                    latest_summary: Some(adapter_output.summary),
                    evidence_id: task.evidence_id,
                    updated_sequence: 0,
                }),
                ProjectionRecord::Agent(AgentProjection {
                    agent_id: refs.agent_id.clone(),
                    project_id: self.project_id.clone(),
                    name: registration.agent_name.clone(),
                    status: "available".to_string(),
                    current_session_id: None,
                    updated_sequence: 0,
                }),
                ProjectionRecord::Session(SessionProjection {
                    session_id: refs.session_id.clone(),
                    project_id: self.project_id.clone(),
                    task_id: Some(refs.task_id.clone()),
                    agent_id: refs.agent_id.clone(),
                    title: session.title,
                    status: "completed".to_string(),
                    current_goal: session.current_goal,
                    latest_summary: Some(format!("Stopped: {reason}")),
                    latest_confidence: Some(70),
                    latest_blocker: None,
                    updated_sequence: 0,
                }),
                ProjectionRecord::Run(RunProjection {
                    run_id: refs.run_id.clone(),
                    session_id: refs.session_id.clone(),
                    status: stopped_process.status,
                    recovery_of_run_id: None,
                    updated_sequence: 0,
                }),
            ],
        )?;

        self.observe(refs)
    }

    pub fn observe(&self, refs: &FakeRunRefs) -> StateResult<FakeReadModelObservation> {
        Ok(FakeReadModelObservation {
            task: self
                .state
                .task(&refs.task_id)?
                .ok_or_else(|| missing_read_model("task", &refs.task_id))?,
            agent: self
                .state
                .agent(&refs.agent_id)?
                .ok_or_else(|| missing_read_model("agent", &refs.agent_id))?,
            session: self
                .state
                .session(&refs.session_id)?
                .ok_or_else(|| missing_read_model("session", &refs.session_id))?,
            run: self
                .state
                .run(&refs.run_id)?
                .ok_or_else(|| missing_read_model("run", &refs.run_id))?,
            recent_events: self.state.recent_events_for_session(&refs.session_id, 16)?,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeAgentRegistration {
    pub agent_id: AgentId,
    pub agent_name: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeRunRefs {
    pub task_id: TaskId,
    pub agent_id: AgentId,
    pub session_id: SessionId,
    pub run_id: RunId,
    pub runtime_process_ref: String,
    pub external_session_ref: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeReadModelObservation {
    pub task: TaskProjection,
    pub agent: AgentProjection,
    pub session: SessionProjection,
    pub run: RunProjection,
    pub recent_events: Vec<EventRecord>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ControllerInit {
    pub command_id: String,
    pub state_db_path: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveryReport {
    pub recovery_attempt_id: String,
    pub started_sequence: i64,
    pub completed_sequence: i64,
    pub watermark: Option<i64>,
    pub recovered_run_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdapterReplayReport {
    pub input_event_count: usize,
    pub appended_event_count: usize,
    pub tool_event_count: usize,
    pub summary_event_count: usize,
    pub completed_turn_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalAdapterDispatchPlan {
    pub project_id: ProjectId,
    pub agent_id: AgentId,
    pub agent_name: String,
    pub session_id: SessionId,
    pub run_id: RunId,
    pub launch_plan: LocalAdapterLaunchPlan,
    pub runtime_program: String,
    pub runtime_arg_count: usize,
    pub runtime_cwd: PathBuf,
    pub request_env_count: usize,
}

fn event(event_id: &str, kind: EventKind, project_id: &ProjectId) -> NewEvent {
    let mut event = NewEvent::new(event_id, kind, "capo-controller");
    event.project_id = Some(project_id.clone());
    event.idempotency_key = Some(event_id.to_string());
    event
}

fn scoped_event(
    event_id: &str,
    kind: EventKind,
    project_id: &ProjectId,
    task_id: &TaskId,
    agent_id: &AgentId,
    session_id: &SessionId,
    run_id: &RunId,
) -> NewEvent {
    let mut event = event(event_id, kind, project_id);
    event.task_id = Some(task_id.clone());
    event.agent_id = Some(agent_id.clone());
    event.session_id = Some(session_id.clone());
    event.run_id = Some(run_id.clone());
    event
}

trait EventBuilder {
    fn with_payload(self, payload_json: String) -> Self;
    fn with_turn(self, turn_id: impl ToString) -> Self;
}

impl EventBuilder for NewEvent {
    fn with_payload(mut self, payload_json: String) -> Self {
        self.payload_json = payload_json;
        self
    }

    fn with_turn(mut self, turn_id: impl ToString) -> Self {
        self.turn_id = Some(turn_id.to_string());
        self
    }
}

fn slug(value: &str) -> String {
    let slug = value
        .chars()
        .filter_map(|ch| {
            if ch.is_ascii_alphanumeric() {
                Some(ch.to_ascii_lowercase())
            } else if ch.is_ascii_whitespace() || ch == '-' || ch == '_' {
                Some('-')
            } else {
                None
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();

    if slug.is_empty() {
        "work".to_string()
    } else {
        slug
    }
}

fn registration_slug(value: &str) -> String {
    slug(value)
}

fn missing_read_model(kind: &'static str, id: &impl ToString) -> StateError {
    StateError::MissingReadModel {
        kind,
        id: id.to_string(),
    }
}

fn require_intent(command: &CommandEnvelope, expected: CommandIntent) {
    assert_eq!(
        command.intent, expected,
        "controller command intent did not match handler"
    );
}

fn required_structured_arg<'a>(command: &'a CommandEnvelope, key: &str) -> StateResult<&'a str> {
    optional_structured_arg(command, key)
        .ok_or_else(|| missing_read_model("command.structured_args", &key))
}

fn optional_structured_arg<'a>(command: &'a CommandEnvelope, key: &str) -> Option<&'a str> {
    command
        .structured_args
        .iter()
        .find_map(|(candidate, value)| (candidate == key).then_some(value.as_str()))
}

fn escape_json(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn stable_hash(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("fnv1a64:{hash:016x}")
}

#[cfg(test)]
mod tests;
