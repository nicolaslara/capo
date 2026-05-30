use capo_controller::FakeRunRefs;
use capo_core::{AgentId, InputOrigin, ProjectId, RunId, SessionId, TaskId};
use capo_state::{
    AdapterDispatchExecutionProjection, AdapterDispatchPlanProjection,
    AdapterDispatchPromptSourceProjection,
};

use crate::util::{slug, stable_hash};

pub type ServerResult<T> = Result<T, ServerError>;

#[derive(Debug)]
pub enum ServerError {
    State(capo_state::StateError),
    AdapterFixture(String),
    UnknownAgent {
        agent_name: String,
    },
    AgentHasNoActiveSession {
        agent_name: String,
    },
    AgentAlreadyHasSession {
        agent_name: String,
        session_id: String,
        run_status: Option<String>,
    },
    SessionAlreadyExists {
        session_id: String,
    },
    RunAlreadyExists {
        run_id: String,
    },
    UnknownDispatchPlan {
        dispatch_plan_id: String,
    },
    UnknownSession {
        session_id: String,
    },
    RunSessionMismatch {
        session_id: String,
        run_id: String,
        actual_session_id: String,
    },
    AdapterSessionMismatch {
        session_id: String,
        session_adapter: String,
        requested_adapter: String,
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
    SteerAgent {
        agent_name: String,
        goal: String,
    },
    InterruptAgent {
        agent_name: String,
        reason: String,
    },
    StopAgent {
        agent_name: String,
        reason: String,
    },
    ListAgents,
    AgentStatus {
        agent_name: String,
    },
    Dashboard {
        recent_event_limit: usize,
    },
    StartSession {
        agent_name: String,
        goal: String,
        adapter: String,
        session_id: Option<String>,
        run_id: Option<String>,
    },
    ReplayAdapterFixture {
        adapter: String,
        session_id: String,
        run_id: String,
        turn_id: String,
        fixture_name: String,
        fixture_jsonl: String,
    },
    PlanDispatch {
        agent_name: String,
        adapter: String,
        goal: String,
        workspace: String,
        artifacts: String,
        session_id: String,
        run_id: String,
        turn_id: String,
        deterministic_opt_in: bool,
    },
    PreflightLiveProvider {
        agent_name: String,
        adapter: String,
        goal: String,
        workspace: String,
        artifacts: String,
        session_id: String,
        run_id: String,
        turn_id: String,
        capability_profile: String,
        runtime_scope: String,
        credential_scan_policy: String,
        raw_prompt_policy: String,
        raw_output_policy: String,
        tool_wrapper_policy: String,
        live_provider_opt_in: bool,
    },
    GateDispatch {
        dispatch_plan_id: String,
    },
    RunDispatchLocal {
        dispatch_plan_id: String,
        fixture_name: String,
        fixture_jsonl: String,
    },
    RunLiveProviderLocal {
        dispatch_plan_id: String,
        goal: String,
        live_execution_opt_in: bool,
        mock_runtime_opt_in: bool,
        mock_provider_output_name: Option<String>,
        mock_provider_output_jsonl: Option<String>,
        timeout_seconds: u64,
        /// Absolute path to a codex binary to run on the spawn path instead of
        /// resolving `codex` from PATH. Ops set it from `CAPO_CODEX_BIN`; tests
        /// pass a stub so the spawn path is deterministic. `None`/relative keeps
        /// `codex`.
        codex_program_override: Option<String>,
        /// RTL9: whether this turn is running unattended. An unattended turn can
        /// never reach a live workspace write here (that is `goal-autonomy`
        /// territory); the handler resolves the write mode via the RTL6 gate
        /// (`live_execution_opt_in` AND `CAPO_SERVER_RUN_CODEX_LIVE` AND
        /// `!unattended`), so a `true` here forces the read-only dry-run profile.
        unattended: bool,
    },
    Recover,
}

impl ServerCommand {
    /// Whether this command only reads the store (never appends an event or
    /// mutates a projection). Read-only commands need not be serialized behind
    /// the transport's single-writer lock, so they can run concurrently with
    /// each other and (under WAL) alongside an in-flight write. Every other
    /// command is treated as write-bearing and serialized; defaulting unknown or
    /// future variants to write-bearing keeps the single-writer guarantee safe
    /// by construction.
    pub fn is_read_only(&self) -> bool {
        matches!(
            self,
            ServerCommand::ListAgents
                | ServerCommand::AgentStatus { .. }
                | ServerCommand::Dashboard { .. }
        )
    }
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
    SessionStarted(TaskRunSummary),
    AdapterFixtureReplayed(AdapterReplaySummary),
    DispatchPlanned(DispatchPlanSummary),
    LiveProviderPreflighted(LiveProviderPreflightSummary),
    DispatchGated(DispatchGateSummary),
    DispatchRun(DispatchRunSummary),
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
    pub(crate) fn with_session(mut self, session: Option<SessionSummary>) -> Self {
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
    pub adapter_kind: Option<String>,
    pub current_goal: String,
    pub latest_summary: Option<String>,
    pub latest_blocker: Option<String>,
    pub latest_confidence: Option<i64>,
    pub recent_event_count: usize,
    pub evidence_count: usize,
    pub evidence_refs: Vec<String>,
    pub review_finding_count: usize,
    pub task_outcome_report_count: usize,
    pub turn_count: usize,
    pub turn_ids: Vec<String>,
    pub latest_dispatch_plan_id: Option<String>,
    pub latest_dispatch_gate_id: Option<String>,
    pub latest_dispatch_execution_id: Option<String>,
    pub dispatch_gate_status: Option<String>,
    pub dispatch_gate_reasons: Option<String>,
    pub dispatch_next_action: Option<String>,
    pub dispatch_execution_status: Option<String>,
    pub dispatch_runtime_process_ref: Option<String>,
    pub dispatch_provider_cli_execution_allowed: Option<bool>,
    pub dispatch_provider_cli_executed: Option<bool>,
    pub dispatch_credential_scan_status: Option<String>,
    pub dispatch_raw_prompt_policy: Option<String>,
    pub dispatch_raw_output_policy: Option<String>,
    pub tool_call_count: usize,
    pub tool_observation_count: usize,
    pub memory_packet_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LiveProviderPreflightSummary {
    pub dispatch_plan_id: String,
    pub dispatch_gate_id: String,
    pub execution_request_id: String,
    pub adapter: String,
    pub provider_kind: String,
    pub agent_name: String,
    pub session_id: SessionId,
    pub run_id: RunId,
    pub capability_profile: String,
    pub runtime_scope: String,
    pub credential_scan_policy: String,
    pub raw_prompt_policy: String,
    pub raw_output_policy: String,
    pub tool_wrapper_policy: String,
    pub provider_cli_execution_allowed: bool,
    pub provider_cli_executed: bool,
    pub status: String,
    pub reasons: String,
    pub next_action: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DispatchPlanSummary {
    pub dispatch_plan_id: String,
    pub prompt_source_id: String,
    pub adapter: String,
    pub agent_name: String,
    pub session_id: SessionId,
    pub run_id: RunId,
    pub runtime_program: String,
    pub runtime_prompt_policy: String,
    pub raw_prompt_policy: String,
    pub provider_cli_executed: bool,
    pub status: String,
}

impl DispatchPlanSummary {
    pub(crate) fn from_projection(
        plan: &AdapterDispatchPlanProjection,
        prompt_source: &AdapterDispatchPromptSourceProjection,
    ) -> Self {
        Self {
            dispatch_plan_id: plan.dispatch_plan_id.clone(),
            prompt_source_id: prompt_source.prompt_source_id.clone(),
            adapter: plan.adapter_kind.clone(),
            agent_name: plan.agent_name.clone(),
            session_id: plan.session_id.clone(),
            run_id: plan.run_id.clone(),
            runtime_program: plan.runtime_program.clone(),
            runtime_prompt_policy: plan.runtime_prompt_policy.clone(),
            raw_prompt_policy: prompt_source.raw_prompt_policy.clone(),
            provider_cli_executed: plan.provider_cli_executed,
            status: plan.status.clone(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DispatchGateSummary {
    pub dispatch_plan_id: String,
    pub dispatch_gate_id: String,
    pub execution_request_id: String,
    pub materialization_id: String,
    pub adapter: String,
    pub provider_cli_execution_allowed: bool,
    pub provider_cli_executed: bool,
    pub status: String,
    pub reasons: String,
    pub raw_prompt_policy: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DispatchRunSummary {
    pub dispatch_plan_id: String,
    pub dispatch_execution_id: String,
    pub adapter: String,
    pub session_id: SessionId,
    pub run_id: RunId,
    pub provider_cli_execution_allowed: bool,
    pub provider_cli_executed: bool,
    pub status: String,
    pub runtime_process_ref: Option<String>,
    pub credential_scan_status: String,
    pub raw_prompt_policy: String,
    pub raw_output_policy: String,
    pub reason_codes: String,
    pub input_event_count: usize,
    pub appended_event_count: usize,
    pub tool_event_count: usize,
    pub summary_event_count: usize,
    pub completed_turn_count: usize,
    /// Observed post-turn provider token/cost, in provider cost units, when the
    /// provider reports it. `None` when no token source is available (phase-1
    /// mock/live has none), in which case the RTL7 ceiling accounting falls back
    /// to the caller's pre-turn estimate. The real Codex token round-trip wires
    /// this in RTL9.
    pub observed_token_cost: Option<u64>,
}

impl DispatchRunSummary {
    pub(crate) fn from_execution(
        execution: &AdapterDispatchExecutionProjection,
        input_event_count: usize,
        appended_event_count: usize,
        tool_event_count: usize,
        summary_event_count: usize,
        completed_turn_count: usize,
    ) -> Self {
        Self {
            dispatch_plan_id: execution.dispatch_plan_id.clone(),
            dispatch_execution_id: execution.dispatch_execution_id.clone(),
            adapter: execution.adapter_kind.clone(),
            session_id: execution.session_id.clone(),
            run_id: execution.run_id.clone(),
            provider_cli_execution_allowed: execution.provider_cli_execution_allowed,
            provider_cli_executed: execution.provider_cli_executed,
            status: execution.status.clone(),
            runtime_process_ref: execution.runtime_process_ref.clone(),
            credential_scan_status: execution.credential_scan_status.clone(),
            raw_prompt_policy: execution.raw_prompt_policy.clone(),
            raw_output_policy: execution.raw_output_policy.clone(),
            reason_codes: execution.reason_codes.clone(),
            input_event_count,
            appended_event_count,
            tool_event_count,
            summary_event_count,
            completed_turn_count,
            // No provider token source on the deterministic/mock/live phase-1
            // path; RTL9 wires the real Codex token round-trip here.
            observed_token_cost: None,
        }
    }
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdapterReplaySummary {
    pub adapter: String,
    pub fixture_name: String,
    pub fixture_hash: String,
    pub agent_name: String,
    pub task_id: TaskId,
    pub session_id: SessionId,
    pub run_id: RunId,
    pub turn_id: String,
    pub provider_cli_executed: bool,
    pub raw_content_policy: String,
    pub input_event_count: usize,
    pub appended_event_count: usize,
    pub tool_event_count: usize,
    pub summary_event_count: usize,
    pub completed_turn_count: usize,
}

impl TaskRunSummary {
    pub(crate) fn from_run_refs(run: FakeRunRefs) -> Self {
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
        ServerCommand::SteerAgent { agent_name, goal } => {
            format!(
                "server-agent-steer-{}-{}",
                slug(agent_name),
                stable_hash(goal.as_bytes())
            )
        }
        ServerCommand::InterruptAgent { agent_name, reason } => {
            format!(
                "server-agent-interrupt-{}-{}",
                slug(agent_name),
                stable_hash(reason.as_bytes())
            )
        }
        ServerCommand::StopAgent { agent_name, reason } => {
            format!(
                "server-agent-stop-{}-{}",
                slug(agent_name),
                stable_hash(reason.as_bytes())
            )
        }
        ServerCommand::ListAgents => "server-agent-list".to_string(),
        ServerCommand::AgentStatus { agent_name } => {
            format!("server-agent-status-{}", slug(agent_name))
        }
        ServerCommand::Dashboard { .. } => "server-dashboard".to_string(),
        ServerCommand::StartSession {
            adapter,
            agent_name,
            goal,
            session_id,
            run_id,
            ..
        } => {
            format!(
                "server-session-start-{}-{}-{}-{}-{}",
                slug(adapter),
                slug(agent_name),
                session_id
                    .as_deref()
                    .map(slug)
                    .unwrap_or_else(|| "auto-session".to_string()),
                run_id
                    .as_deref()
                    .map(slug)
                    .unwrap_or_else(|| "auto-run".to_string()),
                stable_hash(goal.as_bytes())
            )
        }
        ServerCommand::ReplayAdapterFixture {
            adapter,
            session_id,
            run_id,
            turn_id,
            fixture_name,
            ..
        } => {
            format!(
                "server-adapter-replay-{}-{}-{}-{}-{}",
                slug(adapter),
                slug(session_id),
                slug(run_id),
                slug(turn_id),
                slug(fixture_name)
            )
        }
        ServerCommand::PlanDispatch {
            adapter,
            agent_name,
            goal,
            ..
        } => {
            format!(
                "server-dispatch-plan-{}-{}-{}",
                slug(adapter),
                slug(agent_name),
                slug(goal)
            )
        }
        ServerCommand::PreflightLiveProvider {
            adapter,
            agent_name,
            goal,
            session_id,
            run_id,
            turn_id,
            ..
        } => {
            format!(
                "server-dispatch-live-preflight-{}-{}-{}-{}-{}-{}",
                slug(adapter),
                slug(agent_name),
                slug(session_id),
                slug(run_id),
                slug(turn_id),
                stable_hash(goal.as_bytes())
            )
        }
        ServerCommand::GateDispatch { dispatch_plan_id } => {
            format!("server-dispatch-gate-{}", slug(dispatch_plan_id))
        }
        ServerCommand::RunDispatchLocal {
            dispatch_plan_id,
            fixture_name,
            ..
        } => {
            format!(
                "server-dispatch-run-local-{}-{}",
                slug(dispatch_plan_id),
                slug(fixture_name)
            )
        }
        ServerCommand::RunLiveProviderLocal {
            dispatch_plan_id,
            goal,
            mock_provider_output_name,
            ..
        } => {
            format!(
                "server-dispatch-live-run-local-{}-{}-{}",
                slug(dispatch_plan_id),
                stable_hash(goal.as_bytes()),
                mock_provider_output_name
                    .as_deref()
                    .map(slug)
                    .unwrap_or_else(|| "provider".to_string())
            )
        }
        ServerCommand::Recover => "server-recover".to_string(),
    }
}
