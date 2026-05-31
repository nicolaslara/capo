//! Controller orchestration for Capo.
//!
//! P3 keeps this intentionally fake-only, but the control flow is real: the
//! controller calls each boundary, persists Capo-owned events/projections, and
//! answers inspection requests from SQLite read models.

use std::path::{Path, PathBuf};

use capo_adapters::{
    AdapterSessionRequest, AdapterToolObservation, AgentAdapter, AgentAdapterHandle,
    ClaudeCodeAdapter, CodexExecAdapter, LocalAdapterLaunchPlan, NormalizedAdapterEvent,
    ProviderConnector, ScriptedMockAgent, ScriptedMockTurn, TurnRequest,
};
use capo_core::{
    AgentId, CommandEnvelope, CommandIntent, EvidenceId, MemoryPacketId, ProjectId, RunId,
    SessionId, TaskId, ToolCallId, TurnId,
};
use capo_memory::{
    MemoryBackend, MemoryCandidate, MemoryReviewState, MemorySensitivity, MemorySourceKind,
    MemorySourceRef, SourceLinkedMemoryPacketRequest,
};
use capo_runtime::{FakeRuntimeStartRequest, LocalProcessRunner, RuntimeRunner};
use capo_state::{
    AgentProjection, ArtifactRecord, EventKind, EventRecord, NewEvent, ProjectProjection,
    ProjectedTurnOutcome, ProjectionRecord, RedactionState, RunProjection, RunReapKind,
    RunReapObservation, SessionProjection, SqliteStateStore, StateError, StateResult,
    TaskProjection,
};
use capo_tools::{
    FakeToolRequest, PermissionDecision, PermissionPolicy, PermissionRequest, ToolExposure,
};

mod adapter_replay;
mod fake_session;
mod grant_lifecycle;
mod local_dispatch;
mod permission_round_trip;
mod real_controller;
mod resource_ceiling;
mod session_control;
mod tool_dispatch;
mod turn_loop;
mod verification;
mod workspace_lock;

pub use grant_lifecycle::{
    GrantReadBackDecision, GrantReadBackSource, GrantRevocation, GrantRevocationScope,
};
pub use local_dispatch::LocalAdapterDispatchRunStart;
pub use permission_round_trip::{
    PermissionCancellation, PermissionRoundTripOutcome, PermissionRoundTripScope,
};
pub use real_controller::{
    RealAgentRegistration, RealBoundaryController, RealReadModelObservation, RealRunRefs,
};
pub use resource_ceiling::{
    CeilingBreach, CeilingTurnOutcome, RunResourceCeiling, RunResourceUsage,
};
pub use tool_dispatch::{
    PermissionDecideOutcome, ToolDispatchOutcome, ToolDispatchScope, ToolRefusal,
};
pub use turn_loop::{TurnFinished, TurnStopReason};
pub use verification::{
    TestRunRecord, VERIFICATION_EVIDENCE_ACTOR, VERIFICATION_EVIDENCE_SOURCE, VerificationCommand,
    VerificationKind, VerificationOutcome, VerificationScope,
};
pub use workspace_lock::{
    WorkspaceLeaseScope, WorkspaceLockConflict, WorkspaceWriteGate, WorkspaceWriteLeaseOutcome,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeBoundaryController {
    project_id: ProjectId,
    state: SqliteStateStore,
    adapter: AgentAdapterHandle,
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
        Self::open_with_permission_policy_and_adapter(
            project_id,
            state_root,
            permission_policy,
            AgentAdapterHandle::fake(),
        )
    }

    /// Open the controller over an injected adapter handle.
    ///
    /// The controller drives the adapter purely through the [`AgentAdapter`]
    /// trait (`open_session`/`send_turn`/`attach_session`/`interrupt`/`stop`),
    /// so the concrete implementation behind [`AgentAdapterHandle`] is
    /// substitutable. The default constructors inject
    /// [`AgentAdapterHandle::fake`]; the scripted-mock handle is the explicit
    /// deterministic fallback used by the parity suites.
    pub fn open_with_adapter(
        project_id: ProjectId,
        state_root: impl AsRef<Path>,
        adapter: AgentAdapterHandle,
    ) -> StateResult<Self> {
        Self::open_with_permission_policy_and_adapter(
            project_id,
            state_root,
            PermissionPolicy::allow_trusted_local(),
            adapter,
        )
    }

    pub fn open_with_permission_policy_and_adapter(
        project_id: ProjectId,
        state_root: impl AsRef<Path>,
        permission_policy: PermissionPolicy,
        adapter: AgentAdapterHandle,
    ) -> StateResult<Self> {
        Ok(Self {
            project_id,
            state: SqliteStateStore::open(state_root)?,
            adapter,
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

    /// AI2: return a clone of this core with its chat adapter handle swapped.
    ///
    /// The core's [`SqliteStateStore`] is a path handle, so the clone shares one
    /// database with the source -- swapping the adapter changes ONLY which handle
    /// drives the chat turn (`send_task`/`redirect`), never the persisted store.
    /// The server uses this to drive a Codex-BOUND agent's chat turn through the
    /// [`AgentAdapterHandle::codex`] handle while leaving the shared default
    /// (fake) core untouched for every other agent. This is the binding-respecting
    /// seam: it is a per-agent view, not a new global default.
    #[must_use]
    pub fn with_adapter(mut self, adapter: AgentAdapterHandle) -> Self {
        self.adapter = adapter;
        self
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

    /// Restart recovery: reap orphaned in-flight process groups and reconcile
    /// the read model, framed as a single recovery attempt (RTL10).
    ///
    /// This is the one production restart-recovery seam (driven by
    /// `ServerCommand::Recover`). It replaces the blunt
    /// `mark_active_runs_exited_unknown` -- which marked *every* live-looking run
    /// `exited_unknown` and left any still-running children orphaned -- with the
    /// crash-safe reaper: for each in-flight run it loads the PID/boot-id its
    /// spawn persisted before returning, probes that process group, reaps it if
    /// still alive within the same boot, and records
    /// `run.orphaned`/`run.exited`/`run.recovered`. The whole sweep stays inside
    /// the `begin_recovery`/`complete_recovery` bracket the state model's Restart
    /// Recovery order requires, so it projects into `recovery_attempts` exactly
    /// like before. Phase 1 reaps and records; it does not reattach (full
    /// liveness-probe reattach stays in `safety-gates`).
    pub fn recover_command(&self, command: &CommandEnvelope) -> StateResult<RecoveryReport> {
        require_intent(command, CommandIntent::Recover);
        let recovery_attempt_id = format!(
            "recovery-{}-after-{}",
            command.command_id,
            self.state.last_sequence()?
        );
        let started = self.state.begin_recovery(&recovery_attempt_id)?;
        self.state.rebuild_projections()?;
        let recovered_runs = self.reap_orphaned_runs(&recovery_attempt_id)?;
        let completed = self.state.complete_recovery(&recovery_attempt_id)?;
        Ok(RecoveryReport {
            recovery_attempt_id,
            started_sequence: started.started_sequence,
            completed_sequence: completed.completed_sequence.unwrap_or_default(),
            watermark: self.state.watermark("default")?,
            recovered_run_count: recovered_runs.len(),
        })
    }

    /// Probe (and, if alive within the same boot, reap) every in-flight run's
    /// persisted process group, then record the per-run recovery outcome.
    ///
    /// Returns the reconciled `Run` projections. Must run inside a recovery
    /// attempt bracket ([`Self::recover_command`]); exposed separately so tests
    /// can drive a restart sweep deterministically.
    pub fn reap_orphaned_runs(&self, recovery_attempt_id: &str) -> StateResult<Vec<RunProjection>> {
        let inflight = self.state.inflight_runs_for_project(&self.project_id)?;
        let observations: Vec<RunReapObservation> = inflight
            .into_iter()
            .map(|run| {
                let (kind, observed_runtime_state_hash) = match run.external_pid {
                    Some(pid) => {
                        let reap = LocalProcessRunner::reap_orphan_process_group(
                            pid,
                            run.boot_id.as_deref(),
                        );
                        let kind = if reap.reaped {
                            RunReapKind::AliveReaped
                        } else {
                            RunReapKind::AlreadyGone
                        };
                        (kind, reap.observed_runtime_state_hash)
                    }
                    None => (
                        RunReapKind::NoProcess,
                        stable_hash(format!("no-process:{}", run.run_id).as_bytes()),
                    ),
                };
                RunReapObservation {
                    run_id: run.run_id,
                    session_id: run.session_id,
                    previous_status: run.status,
                    kind,
                    external_pid: run.external_pid,
                    observed_runtime_state_hash,
                }
            })
            .collect();
        self.state
            .reap_orphaned_runs(&self.project_id, recovery_attempt_id, &observations)
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

    /// AI3: the production `send_task` command path that routes the per-turn
    /// summary tool through the REAL dispatch seam (the supplied live Capo
    /// `exposure`'s `authorize_and_invoke`) instead of the fake summary shim.
    /// `RealBoundaryController::send_task_command` calls this with its own real
    /// Capo exposure so a real chat turn's tool call is a real dispatched result.
    pub(crate) fn send_task_command_with_real_tools(
        &self,
        command: &CommandEnvelope,
        exposure: &ToolExposure,
    ) -> StateResult<FakeRunRefs> {
        require_intent(command, CommandIntent::SendTask);
        let agent_name = required_structured_arg(command, "agent")?;
        let goal = command
            .text
            .as_deref()
            .ok_or_else(|| missing_read_model("command.text", &command.command_id))?;
        let registration = self.registration_for_agent_name(agent_name)?;
        let task_id = match optional_structured_arg(command, "task_id") {
            Some(task_id) => TaskId::new(task_id),
            None => TaskId::new(format!("task-{}", slug(goal))),
        };
        self.send_task_with_real_tools(&registration, task_id, goal, exposure)
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

#[derive(Clone, Debug, Default, Eq, PartialEq)]
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
    fn with_item(self, item_id: impl ToString) -> Self;
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

    fn with_item(mut self, item_id: impl ToString) -> Self {
        self.item_id = Some(item_id.to_string());
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
