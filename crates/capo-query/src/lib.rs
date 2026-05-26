//! Reusable read-model queries for Capo operator surfaces.
//!
//! This crate owns aggregation over state projections. CLI, dashboards, voice,
//! mobile, and web surfaces should render these structs instead of stitching
//! SQLite read models together independently.

use capo_core::{ProjectId, SessionId};
use capo_state::{
    AdapterDispatchExecutionProjection, AdapterDispatchExecutionRequestProjection,
    AdapterDispatchGateProjection, AdapterDispatchPlanProjection,
    AdapterDispatchPromptMaterializationProjection, AdapterDispatchPromptSourceProjection,
    AdapterDispatchReplayProjection, AdapterReadinessProjection, AdapterSmokeReportProjection,
    AgentProjection, ConnectivityExposureProjection, EventRecord, EvidenceProjection,
    MemoryPacketProjection, ReviewFindingProjection, RunProjection, RuntimeTargetProjection,
    SessionProjection, SqliteStateStore, StateResult, TaskOutcomeReportProjection,
    ToolCallProjection, ToolObservationProjection, WorkpadTaskProjection,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectDashboard {
    pub project_id: ProjectId,
    pub agents: Vec<AgentDashboardRow>,
    pub project_evidence: Vec<EvidenceProjection>,
    pub review_findings: Vec<ReviewFindingProjection>,
    pub task_outcome_reports: Vec<TaskOutcomeReportProjection>,
    pub runtime_targets: Vec<RuntimeTargetProjection>,
    pub connectivity_exposures: Vec<ConnectivityExposureProjection>,
    pub adapter_readiness: Vec<AdapterReadinessProjection>,
    pub adapter_smoke_reports: Vec<AdapterSmokeReportProjection>,
    pub adapter_dispatch_plans: Vec<AdapterDispatchPlanProjection>,
    pub adapter_dispatch_gates: Vec<AdapterDispatchGateProjection>,
    pub adapter_dispatch_replays: Vec<AdapterDispatchReplayProjection>,
    pub adapter_dispatch_execution_requests: Vec<AdapterDispatchExecutionRequestProjection>,
    pub adapter_dispatch_executions: Vec<AdapterDispatchExecutionProjection>,
    pub adapter_dispatch_prompt_sources: Vec<AdapterDispatchPromptSourceProjection>,
    pub adapter_dispatch_prompt_materializations:
        Vec<AdapterDispatchPromptMaterializationProjection>,
    pub adapter_dogfood_gate: AdapterDogfoodGate,
    pub workpad_tasks: Vec<WorkpadTaskProjection>,
}

impl ProjectDashboard {
    pub fn active_session_count(&self) -> usize {
        self.agents
            .iter()
            .filter(|agent| agent.session.is_some())
            .count()
    }

    pub fn dogfood_readiness(&self) -> ProjectDogfoodReadiness {
        project_dogfood_readiness(self)
    }

    pub fn tool_activity_summary(&self, agent_name: Option<&str>) -> ToolActivitySummary {
        let mut summary = ToolActivitySummary {
            agent_count: 0,
            active_session_count: 0,
            tool_call_count: 0,
            tool_observation_count: 0,
        };
        for row in self.agents.iter().filter(|row| {
            agent_name
                .map(|name| row.agent.name == name)
                .unwrap_or(true)
        }) {
            summary.agent_count += 1;
            if let Some(session_row) = &row.session {
                summary.active_session_count += 1;
                summary.tool_call_count += session_row.tool_calls.len();
                summary.tool_observation_count += session_row.tool_observations.len();
            }
        }
        summary
    }

    pub fn next_workpad_task(&self) -> Option<&WorkpadTaskProjection> {
        self.workpad_tasks
            .iter()
            .filter(|task| actionable_workpad_status_rank(&task.observed_status).is_some())
            .filter(|task| task.capo_execution_status == "observed_only")
            .min_by(|left, right| {
                actionable_workpad_status_rank(&left.observed_status)
                    .cmp(&actionable_workpad_status_rank(&right.observed_status))
                    .then_with(|| left.path.cmp(&right.path))
                    .then_with(|| left.source_anchor.cmp(&right.source_anchor))
                    .then_with(|| left.workpad_task_id.cmp(&right.workpad_task_id))
            })
    }

    pub fn next_workpad_candidate_count(&self) -> usize {
        self.workpad_tasks
            .iter()
            .filter(|task| actionable_workpad_status_rank(&task.observed_status).is_some())
            .filter(|task| task.capo_execution_status == "observed_only")
            .count()
    }

    pub fn adapter_dispatch_status(&self, dispatch_plan_id: &str) -> Option<AdapterDispatchStatus> {
        let plan = self
            .adapter_dispatch_plans
            .iter()
            .find(|plan| plan.dispatch_plan_id == dispatch_plan_id)?;
        let latest_gate = self
            .adapter_dispatch_gates
            .iter()
            .rev()
            .find(|gate| gate.dispatch_plan_id == plan.dispatch_plan_id);
        let latest_replay = self
            .adapter_dispatch_replays
            .iter()
            .rev()
            .find(|replay| replay.dispatch_plan_id == plan.dispatch_plan_id);
        let latest_execution = self
            .adapter_dispatch_executions
            .iter()
            .rev()
            .find(|execution| execution.dispatch_plan_id == plan.dispatch_plan_id);

        let next_action = if latest_execution
            .map(|execution| execution.provider_cli_executed)
            .unwrap_or(false)
        {
            "inspect_execution_artifacts_and_export_evidence"
        } else if latest_replay.is_some() {
            "inspect_replay_or_prepare_real_execution"
        } else if latest_execution.is_some() {
            "resolve_latest_execution_blocker"
        } else if latest_gate
            .map(|gate| gate.provider_cli_execution_allowed && gate.status == "ready_for_execution")
            .unwrap_or(false)
        {
            "replay_dispatch_fixture_or_run_provider_execution_after_explicit_opt_in"
        } else if self.adapter_dogfood_gate.ready {
            "record_ready_dispatch_gate"
        } else {
            "record_clean_real_smoke_evidence"
        };

        Some(AdapterDispatchStatus {
            dispatch_plan_id: plan.dispatch_plan_id.clone(),
            adapter_kind: plan.adapter_kind.clone(),
            agent_name: plan.agent_name.clone(),
            session_id: plan.session_id.to_string(),
            run_id: plan.run_id.to_string(),
            plan_status: plan.status.clone(),
            provider_kind: plan.provider_kind.clone(),
            credential_scope: plan.credential_scope.clone(),
            runtime_program: plan.runtime_program.clone(),
            runtime_arg_count: plan.runtime_arg_count,
            runtime_prompt_policy: plan.runtime_prompt_policy.clone(),
            provider_cli_executed: plan.provider_cli_executed,
            dogfood_gate_status: self.adapter_dogfood_gate.status.clone(),
            latest_dispatch_gate_id: latest_gate
                .map(|gate| gate.dispatch_gate_id.clone())
                .unwrap_or_else(|| "none".to_string()),
            latest_gate_status: latest_gate
                .map(|gate| gate.status.clone())
                .unwrap_or_else(|| "missing".to_string()),
            latest_gate_provider_cli_execution_allowed: latest_gate
                .map(|gate| gate.provider_cli_execution_allowed)
                .unwrap_or(false),
            latest_gate_reasons: latest_gate
                .map(|gate| gate.reason_codes.clone())
                .unwrap_or_else(|| "recorded_dispatch_gate_missing".to_string()),
            latest_dispatch_replay_id: latest_replay
                .map(|replay| replay.dispatch_replay_id.clone())
                .unwrap_or_else(|| "none".to_string()),
            latest_replay_appended_events: latest_replay
                .map(|replay| replay.appended_event_count)
                .unwrap_or(0),
            latest_replay_raw_content_policy: latest_replay
                .map(|replay| replay.raw_content_policy.clone())
                .unwrap_or_else(|| "none".to_string()),
            latest_dispatch_execution_id: latest_execution
                .map(|execution| execution.dispatch_execution_id.clone())
                .unwrap_or_else(|| "none".to_string()),
            latest_execution_status: latest_execution
                .map(|execution| execution.status.clone())
                .unwrap_or_else(|| "missing".to_string()),
            latest_execution_provider_cli_execution_allowed: latest_execution
                .map(|execution| execution.provider_cli_execution_allowed)
                .unwrap_or(false),
            latest_execution_provider_cli_executed: latest_execution
                .map(|execution| execution.provider_cli_executed)
                .unwrap_or(false),
            latest_execution_credential_scan_status: latest_execution
                .map(|execution| execution.credential_scan_status.clone())
                .unwrap_or_else(|| "none".to_string()),
            latest_execution_stdout_artifact_id: latest_execution
                .and_then(|execution| execution.stdout_artifact_id.clone())
                .unwrap_or_else(|| "none".to_string()),
            latest_execution_stderr_artifact_id: latest_execution
                .and_then(|execution| execution.stderr_artifact_id.clone())
                .unwrap_or_else(|| "none".to_string()),
            latest_execution_reasons: latest_execution
                .map(|execution| execution.reason_codes.clone())
                .unwrap_or_else(|| "none".to_string()),
            next_action: next_action.to_string(),
        })
    }

    pub fn latest_adapter_dispatch_status(
        &self,
        agent_name: Option<&str>,
    ) -> Option<AdapterDispatchStatus> {
        self.adapter_dispatch_plans
            .iter()
            .filter(|plan| {
                agent_name
                    .map(|name| plan.agent_name == name)
                    .unwrap_or(true)
            })
            .max_by(|left, right| {
                self.adapter_dispatch_activity_sequence(left)
                    .cmp(&self.adapter_dispatch_activity_sequence(right))
                    .then_with(|| left.dispatch_plan_id.cmp(&right.dispatch_plan_id))
            })
            .and_then(|plan| self.adapter_dispatch_status(&plan.dispatch_plan_id))
    }

    fn adapter_dispatch_activity_sequence(&self, plan: &AdapterDispatchPlanProjection) -> i64 {
        let latest_gate_sequence = self
            .adapter_dispatch_gates
            .iter()
            .filter(|gate| gate.dispatch_plan_id == plan.dispatch_plan_id)
            .map(|gate| gate.updated_sequence)
            .max()
            .unwrap_or(0);
        let latest_replay_sequence = self
            .adapter_dispatch_replays
            .iter()
            .filter(|replay| replay.dispatch_plan_id == plan.dispatch_plan_id)
            .map(|replay| replay.updated_sequence)
            .max()
            .unwrap_or(0);
        let latest_execution_sequence = self
            .adapter_dispatch_executions
            .iter()
            .filter(|execution| execution.dispatch_plan_id == plan.dispatch_plan_id)
            .map(|execution| execution.updated_sequence)
            .max()
            .unwrap_or(0);

        plan.updated_sequence
            .max(latest_gate_sequence)
            .max(latest_replay_sequence)
            .max(latest_execution_sequence)
    }

    pub fn runtime_target_status(
        &self,
        runtime_target_id: &str,
    ) -> Option<&RuntimeTargetProjection> {
        self.runtime_targets
            .iter()
            .rev()
            .find(|target| target.runtime_target_id == runtime_target_id)
    }

    pub fn connectivity_exposure_status(
        &self,
        exposure_id: &str,
    ) -> Option<&ConnectivityExposureProjection> {
        self.connectivity_exposures
            .iter()
            .rev()
            .find(|exposure| exposure.exposure_id == exposure_id)
    }

    pub fn latest_connectivity_exposure(
        &self,
        owner_kind: Option<&str>,
        owner_id: Option<&str>,
        channel_kind: Option<&str>,
    ) -> Option<&ConnectivityExposureProjection> {
        self.connectivity_exposures
            .iter()
            .filter(|exposure| {
                owner_kind
                    .map(|kind| exposure.owner_kind == kind)
                    .unwrap_or(true)
            })
            .filter(|exposure| owner_id.map(|id| exposure.owner_id == id).unwrap_or(true))
            .filter(|exposure| {
                channel_kind
                    .map(|channel| exposure.channel_kind == channel)
                    .unwrap_or(true)
            })
            .max_by(|left, right| {
                left.updated_sequence
                    .cmp(&right.updated_sequence)
                    .then_with(|| left.exposure_id.cmp(&right.exposure_id))
            })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentDashboardRow {
    pub agent: AgentProjection,
    pub session: Option<SessionDashboardRow>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionDashboardRow {
    pub session: SessionProjection,
    pub run: Option<RunProjection>,
    pub evidence: Vec<EvidenceProjection>,
    pub tool_calls: Vec<ToolCallProjection>,
    pub tool_observations: Vec<ToolObservationProjection>,
    pub memory_packets: Vec<MemoryPacketProjection>,
    pub review_findings: Vec<ReviewFindingProjection>,
    pub task_outcome_reports: Vec<TaskOutcomeReportProjection>,
    pub recent_events: Vec<EventRecord>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolActivitySummary {
    pub agent_count: usize,
    pub active_session_count: usize,
    pub tool_call_count: usize,
    pub tool_observation_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdapterDogfoodGate {
    pub ready: bool,
    pub status: String,
    pub required_adapters: Vec<String>,
    pub proven_adapters: Vec<String>,
    pub blocked_adapters: Vec<String>,
    pub reasons: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdapterDispatchStatus {
    pub dispatch_plan_id: String,
    pub adapter_kind: String,
    pub agent_name: String,
    pub session_id: String,
    pub run_id: String,
    pub plan_status: String,
    pub provider_kind: String,
    pub credential_scope: String,
    pub runtime_program: String,
    pub runtime_arg_count: i64,
    pub runtime_prompt_policy: String,
    pub provider_cli_executed: bool,
    pub dogfood_gate_status: String,
    pub latest_dispatch_gate_id: String,
    pub latest_gate_status: String,
    pub latest_gate_provider_cli_execution_allowed: bool,
    pub latest_gate_reasons: String,
    pub latest_dispatch_replay_id: String,
    pub latest_replay_appended_events: i64,
    pub latest_replay_raw_content_policy: String,
    pub latest_dispatch_execution_id: String,
    pub latest_execution_status: String,
    pub latest_execution_provider_cli_execution_allowed: bool,
    pub latest_execution_provider_cli_executed: bool,
    pub latest_execution_credential_scan_status: String,
    pub latest_execution_stdout_artifact_id: String,
    pub latest_execution_stderr_artifact_id: String,
    pub latest_execution_reasons: String,
    pub next_action: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectDogfoodReadiness {
    pub ready: bool,
    pub status: String,
    pub real_agent_connector_ready: bool,
    pub workpad_bridge_ready: bool,
    pub dispatch_chain_ready: bool,
    pub workpad_task_count: usize,
    pub observed_workpad_task_count: usize,
    pub imported_workpad_task_count: usize,
    pub dispatch_plan_count: usize,
    pub ready_dispatch_gate_count: usize,
    pub dispatch_replay_count: usize,
    pub dispatch_execution_count: usize,
    pub connector_evidence_refs: Vec<String>,
    pub workpad_task_refs: Vec<String>,
    pub dispatch_chain_refs: Vec<String>,
    pub project_evidence_refs: Vec<String>,
    pub blockers: Vec<String>,
    pub next_actions: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectDashboardQuery {
    pub project_id: ProjectId,
    pub session_id: Option<SessionId>,
    pub status: Option<String>,
    pub workpad_path: Option<String>,
    pub workpad_status: Option<String>,
    pub recent_event_limit: usize,
}

impl ProjectDashboardQuery {
    pub fn new(project_id: ProjectId) -> Self {
        Self {
            project_id,
            session_id: None,
            status: None,
            workpad_path: None,
            workpad_status: None,
            recent_event_limit: 5,
        }
    }

    pub fn with_session_id(mut self, session_id: SessionId) -> Self {
        self.session_id = Some(session_id);
        self
    }

    pub fn with_status(mut self, status: impl Into<String>) -> Self {
        self.status = Some(status.into());
        self
    }

    pub fn with_workpad_path(mut self, path: impl Into<String>) -> Self {
        self.workpad_path = Some(path.into());
        self
    }

    pub fn with_workpad_status(mut self, status: impl Into<String>) -> Self {
        self.workpad_status = Some(status.into());
        self
    }
}

pub fn project_dashboard(
    state: &SqliteStateStore,
    query: ProjectDashboardQuery,
) -> StateResult<ProjectDashboard> {
    let mut rows = Vec::new();
    for agent in state.agents()? {
        if agent.project_id != query.project_id {
            continue;
        }
        let session = agent
            .current_session_id
            .as_ref()
            .map(|session_id| session_dashboard(state, session_id, query.recent_event_limit))
            .transpose()?;
        if let Some(session_id) = &query.session_id
            && session.as_ref().map(|row| &row.session.session_id) != Some(session_id)
        {
            continue;
        }
        if let Some(status) = &query.status
            && !dashboard_row_matches_status(&agent, session.as_ref(), status)
        {
            continue;
        }
        rows.push(AgentDashboardRow { agent, session });
    }
    let connectivity_exposures = state.connectivity_exposures(&query.project_id)?;
    let runtime_targets = state.runtime_targets(&query.project_id)?;
    let project_evidence = state.project_evidence(&query.project_id)?;
    let review_findings = state.review_findings(&query.project_id)?;
    let task_outcome_reports = state.task_outcome_reports(&query.project_id)?;
    let adapter_readiness = state.adapter_readiness(&query.project_id)?;
    let adapter_smoke_reports = state.adapter_smoke_reports(&query.project_id)?;
    let adapter_dispatch_plans = state.adapter_dispatch_plans(&query.project_id)?;
    let adapter_dispatch_gates = state.adapter_dispatch_gates(&query.project_id)?;
    let adapter_dispatch_replays = state.adapter_dispatch_replays(&query.project_id)?;
    let adapter_dispatch_execution_requests =
        state.adapter_dispatch_execution_requests(&query.project_id)?;
    let adapter_dispatch_executions = state.adapter_dispatch_executions(&query.project_id)?;
    let adapter_dispatch_prompt_sources =
        state.adapter_dispatch_prompt_sources(&query.project_id)?;
    let adapter_dispatch_prompt_materializations =
        state.adapter_dispatch_prompt_materializations(&query.project_id)?;
    let adapter_dogfood_gate = adapter_dogfood_gate(&adapter_smoke_reports);
    let workpad_tasks = state
        .workpad_tasks(&query.project_id)?
        .into_iter()
        .filter(|task| {
            query
                .workpad_path
                .as_ref()
                .map(|path| &task.path == path)
                .unwrap_or(true)
        })
        .filter(|task| {
            query
                .workpad_status
                .as_ref()
                .map(|status| {
                    &task.observed_status == status || &task.capo_execution_status == status
                })
                .unwrap_or(true)
        })
        .collect();
    Ok(ProjectDashboard {
        project_id: query.project_id,
        agents: rows,
        project_evidence,
        review_findings,
        task_outcome_reports,
        runtime_targets,
        connectivity_exposures,
        adapter_readiness,
        adapter_smoke_reports,
        adapter_dispatch_plans,
        adapter_dispatch_gates,
        adapter_dispatch_replays,
        adapter_dispatch_execution_requests,
        adapter_dispatch_executions,
        adapter_dispatch_prompt_sources,
        adapter_dispatch_prompt_materializations,
        adapter_dogfood_gate,
        workpad_tasks,
    })
}

pub fn adapter_dogfood_gate(smoke_reports: &[AdapterSmokeReportProjection]) -> AdapterDogfoodGate {
    let required_adapters = vec!["codex_exec".to_string()];
    let proven_adapters = required_adapters
        .iter()
        .filter(|adapter| {
            smoke_reports.iter().any(|report| {
                &report.adapter_kind == *adapter
                    && report.smoke_status == "passed"
                    && report.credential_scan_status == "clean"
                    && report.marker_found
                    && report.dogfood_readiness_effect == "real_agent_connector_proven"
            })
        })
        .cloned()
        .collect::<Vec<_>>();
    let blocked_adapters = required_adapters
        .iter()
        .filter(|adapter| !proven_adapters.contains(adapter))
        .cloned()
        .collect::<Vec<_>>();
    let ready = blocked_adapters.is_empty();
    let reasons = if ready {
        vec!["required_real_smoke_evidence_recorded".to_string()]
    } else {
        blocked_adapters
            .iter()
            .map(|adapter| format!("{adapter}:real_subscription_smoke_not_recorded"))
            .collect()
    };
    AdapterDogfoodGate {
        ready,
        status: if ready {
            "ready_for_first_real_agent_dogfood".to_string()
        } else {
            "blocked_pending_real_smoke".to_string()
        },
        required_adapters,
        proven_adapters,
        blocked_adapters,
        reasons,
    }
}

pub fn project_dogfood_readiness(dashboard: &ProjectDashboard) -> ProjectDogfoodReadiness {
    let real_agent_connector_ready = dashboard.adapter_dogfood_gate.ready;
    let workpad_task_count = dashboard.workpad_tasks.len();
    let observed_workpad_task_count = dashboard
        .workpad_tasks
        .iter()
        .filter(|task| task.capo_execution_status == "observed_only")
        .count();
    let imported_workpad_task_count = dashboard
        .workpad_tasks
        .iter()
        .filter(|task| task.capo_execution_status == "imported")
        .count();
    let workpad_bridge_ready = workpad_task_count > 0;
    let dispatch_plan_count = dashboard.adapter_dispatch_plans.len();
    let ready_dispatch_gate_count = dashboard
        .adapter_dispatch_gates
        .iter()
        .filter(|gate| gate.provider_cli_execution_allowed && gate.status == "ready_for_execution")
        .count();
    let dispatch_replay_count = dashboard.adapter_dispatch_replays.len();
    let dispatch_execution_count = dashboard.adapter_dispatch_executions.len();
    let dispatch_chain_ready = dispatch_plan_count > 0
        && (ready_dispatch_gate_count > 0
            || dispatch_replay_count > 0
            || dispatch_execution_count > 0);
    let connector_evidence_refs = dashboard
        .adapter_smoke_reports
        .iter()
        .map(|report| report.smoke_report_id.clone())
        .collect::<Vec<_>>();
    let workpad_task_refs = dashboard
        .workpad_tasks
        .iter()
        .map(|task| task.workpad_task_id.clone())
        .collect::<Vec<_>>();
    let dispatch_chain_refs = dashboard
        .adapter_dispatch_plans
        .iter()
        .map(|plan| plan.dispatch_plan_id.clone())
        .chain(
            dashboard
                .adapter_dispatch_replays
                .iter()
                .map(|replay| replay.dispatch_replay_id.clone()),
        )
        .chain(
            dashboard
                .adapter_dispatch_executions
                .iter()
                .map(|execution| execution.dispatch_execution_id.clone()),
        )
        .collect::<Vec<_>>();
    let project_evidence_refs = dashboard
        .project_evidence
        .iter()
        .map(|evidence| evidence.evidence_id.to_string())
        .collect::<Vec<_>>();
    let mut blockers = Vec::new();
    let mut next_actions = Vec::new();
    if !real_agent_connector_ready {
        blockers.push("real_agent_connector_not_proven".to_string());
        next_actions.push("record_clean_codex_smoke_evidence".to_string());
    }
    if !workpad_bridge_ready {
        blockers.push("workpad_index_missing".to_string());
        next_actions.push("run_workpad_index".to_string());
    }
    if !dispatch_chain_ready {
        blockers.push("dispatch_chain_missing".to_string());
        next_actions.push("record_or_replay_workpad_dispatch_plan".to_string());
    }
    let ready = blockers.is_empty();
    ProjectDogfoodReadiness {
        ready,
        status: if ready {
            "ready_for_first_dogfood".to_string()
        } else {
            "blocked_pending_dogfood_prerequisites".to_string()
        },
        real_agent_connector_ready,
        workpad_bridge_ready,
        dispatch_chain_ready,
        workpad_task_count,
        observed_workpad_task_count,
        imported_workpad_task_count,
        dispatch_plan_count,
        ready_dispatch_gate_count,
        dispatch_replay_count,
        dispatch_execution_count,
        connector_evidence_refs,
        workpad_task_refs,
        dispatch_chain_refs,
        project_evidence_refs,
        blockers,
        next_actions,
    }
}

fn actionable_workpad_status_rank(status: &str) -> Option<u8> {
    match status {
        "in_progress" => Some(0),
        "pending" => Some(1),
        "ready" => Some(2),
        "waiting_on_opt_in" => Some(3),
        _ => None,
    }
}

fn session_dashboard(
    state: &SqliteStateStore,
    session_id: &SessionId,
    recent_event_limit: usize,
) -> StateResult<SessionDashboardRow> {
    let session =
        state
            .session(session_id)?
            .ok_or_else(|| capo_state::StateError::MissingReadModel {
                kind: "session",
                id: session_id.to_string(),
            })?;
    Ok(SessionDashboardRow {
        run: state.run_for_session(session_id)?,
        evidence: state.evidence_for_session(session_id)?,
        tool_calls: state.tool_calls_for_session(session_id)?,
        tool_observations: state.tool_observations_for_session(session_id)?,
        memory_packets: state.memory_packets_for_session(session_id)?,
        review_findings: state.review_findings_for_session(session_id)?,
        task_outcome_reports: state.task_outcome_reports_for_session(session_id)?,
        recent_events: state.recent_events_for_session(session_id, recent_event_limit)?,
        session,
    })
}

fn dashboard_row_matches_status(
    agent: &AgentProjection,
    session: Option<&SessionDashboardRow>,
    status: &str,
) -> bool {
    agent.status == status
        || session
            .map(|row| {
                row.session.status == status
                    || row.run.as_ref().is_some_and(|run| run.status == status)
            })
            .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;
    use capo_core::{AgentId, EvidenceId, MemoryPacketId, RunId, TaskId, ToolCallId};
    use capo_state::{
        AdapterSmokeReportProjection, AgentProjection, ConnectivityExposureProjection, EventKind,
        EvidenceProjection, MemoryPacketProjection, NewEvent, ProjectionRecord, RedactionState,
        RunProjection, SessionProjection, TaskProjection, ToolCallProjection,
        ToolObservationProjection, WorkpadTaskProjection,
    };

    #[test]
    fn project_dashboard_aggregates_agents_sessions_runs_evidence_and_events() {
        let root = temp_root("query-dashboard");
        let state = SqliteStateStore::open(&root).expect("state");
        let project_id = ProjectId::new("project-capo");
        let task_id = TaskId::new("task-demo");
        let agent_id = AgentId::new("agent-demo");
        let session_id = SessionId::new("session-demo");
        let run_id = RunId::new("run-demo");
        let evidence_id = EvidenceId::new("evidence-demo");

        state
            .append_event(
                NewEvent {
                    event_id: "event-dashboard-demo".to_string(),
                    kind: EventKind::SessionStarted,
                    actor: "test".to_string(),
                    project_id: Some(project_id.clone()),
                    task_id: Some(task_id.clone()),
                    agent_id: Some(agent_id.clone()),
                    session_id: Some(session_id.clone()),
                    run_id: Some(run_id.clone()),
                    turn_id: None,
                    item_id: None,
                    payload_json: "{}".to_string(),
                    idempotency_key: None,
                    redaction_state: RedactionState::Safe,
                },
                &[
                    ProjectionRecord::Task(TaskProjection {
                        task_id: task_id.clone(),
                        project_id: project_id.clone(),
                        title: "Demo".to_string(),
                        capo_execution_status: "active".to_string(),
                        active_session_id: Some(session_id.clone()),
                        latest_summary: None,
                        evidence_id: Some(evidence_id.clone()),
                        updated_sequence: 0,
                    }),
                    ProjectionRecord::Agent(AgentProjection {
                        agent_id: agent_id.clone(),
                        project_id: project_id.clone(),
                        name: "demo".to_string(),
                        status: "running".to_string(),
                        current_session_id: Some(session_id.clone()),
                        updated_sequence: 0,
                    }),
                    ProjectionRecord::Session(SessionProjection {
                        session_id: session_id.clone(),
                        project_id: project_id.clone(),
                        task_id: Some(task_id.clone()),
                        agent_id,
                        title: "Demo session".to_string(),
                        status: "active".to_string(),
                        current_goal: "prove query".to_string(),
                        latest_summary: Some("working".to_string()),
                        latest_confidence: Some(80),
                        latest_blocker: None,
                        updated_sequence: 0,
                    }),
                    ProjectionRecord::Run(RunProjection {
                        run_id: run_id.clone(),
                        session_id: session_id.clone(),
                        status: "running".to_string(),
                        recovery_of_run_id: None,
                        updated_sequence: 0,
                    }),
                    ProjectionRecord::Evidence(EvidenceProjection {
                        evidence_id: evidence_id.clone(),
                        project_id: project_id.clone(),
                        task_id: Some(task_id.clone()),
                        session_id: Some(session_id.clone()),
                        run_id: Some(run_id.clone()),
                        kind: "summary".to_string(),
                        artifact_id: Some("artifact-demo".to_string()),
                        confidence: 80,
                        updated_sequence: 0,
                    }),
                    ProjectionRecord::ToolCall(ToolCallProjection {
                        tool_call_id: ToolCallId::new("tool-demo"),
                        session_id: session_id.clone(),
                        turn_id: Some("turn-demo".to_string()),
                        tool_name: "capo.session_summary".to_string(),
                        tool_origin: "capo".to_string(),
                        status: "completed".to_string(),
                        input_artifact_id: None,
                        output_artifact_id: Some("artifact-tool-demo".to_string()),
                        updated_sequence: 0,
                    }),
                    ProjectionRecord::ToolObservation(ToolObservationProjection {
                        tool_observation_id: "tool-observation-demo".to_string(),
                        session_id: session_id.clone(),
                        tool_call_id: Some(ToolCallId::new("tool-demo")),
                        source: "adapter_event".to_string(),
                        external_tool_ref: Some("provider-tool-1".to_string()),
                        tool_name: "provider.native_search".to_string(),
                        observed_status: "completed".to_string(),
                        instrumentation_level: "observed_only".to_string(),
                        confidence: "high".to_string(),
                        raw_event_hash: "hash-demo".to_string(),
                        artifact_id: Some("artifact-observation-demo".to_string()),
                        updated_sequence: 0,
                    }),
                    ProjectionRecord::MemoryPacketRef(MemoryPacketProjection {
                        memory_packet_id: MemoryPacketId::new("packet-demo"),
                        project_id: project_id.clone(),
                        task_id: Some(task_id),
                        agent_id: Some(AgentId::new("agent-demo")),
                        session_id: Some(session_id.clone()),
                        run_id: Some(run_id),
                        turn_id: Some("turn-demo".to_string()),
                        packet_artifact_id: Some("artifact-memory-demo".to_string()),
                        purpose: "turn_context".to_string(),
                        updated_sequence: 0,
                    }),
                ],
            )
            .expect("append dashboard source event");

        let dashboard =
            project_dashboard(&state, ProjectDashboardQuery::new(project_id)).expect("dashboard");

        assert_eq!(dashboard.agents.len(), 1);
        assert_eq!(dashboard.active_session_count(), 1);
        let row = &dashboard.agents[0];
        assert_eq!(row.agent.name, "demo");
        let session = row.session.as_ref().expect("session row");
        assert_eq!(session.session.current_goal, "prove query");
        let tool_activity = dashboard.tool_activity_summary(None);
        assert_eq!(
            tool_activity,
            ToolActivitySummary {
                agent_count: 1,
                active_session_count: 1,
                tool_call_count: 1,
                tool_observation_count: 1,
            }
        );
        assert_eq!(dashboard.tool_activity_summary(Some("demo")), tool_activity);
        assert_eq!(
            dashboard.tool_activity_summary(Some("missing-agent")),
            ToolActivitySummary {
                agent_count: 0,
                active_session_count: 0,
                tool_call_count: 0,
                tool_observation_count: 0,
            }
        );
        assert_eq!(
            session.run.as_ref().map(|run| run.status.as_str()),
            Some("running")
        );
        assert_eq!(session.evidence[0].evidence_id, evidence_id);
        assert_eq!(
            session.tool_calls[0].tool_call_id,
            ToolCallId::new("tool-demo")
        );
        assert_eq!(
            session.tool_observations[0].tool_observation_id,
            "tool-observation-demo"
        );
        assert_eq!(
            session.tool_observations[0].instrumentation_level,
            "observed_only"
        );
        assert_eq!(
            session.memory_packets[0].memory_packet_id,
            MemoryPacketId::new("packet-demo")
        );
        assert_eq!(session.recent_events[0].kind, "session.started");
    }

    #[test]
    fn project_dashboard_includes_project_level_evidence() {
        let root = temp_root("query-dashboard-project-evidence");
        let state = SqliteStateStore::open(&root).expect("state");
        let project_id = ProjectId::new("project-capo");
        append_agent(&state, &project_id, "agent-idle", None);
        state
            .append_event(
                NewEvent {
                    event_id: "event-project-evidence".to_string(),
                    kind: EventKind::EvidenceRecorded,
                    actor: "test".to_string(),
                    project_id: Some(project_id.clone()),
                    task_id: None,
                    agent_id: None,
                    session_id: None,
                    run_id: None,
                    turn_id: None,
                    item_id: Some("evidence-dogfood-readiness".to_string()),
                    payload_json: "{}".to_string(),
                    idempotency_key: None,
                    redaction_state: RedactionState::Safe,
                },
                &[ProjectionRecord::Evidence(EvidenceProjection {
                    evidence_id: EvidenceId::new("evidence-dogfood-readiness"),
                    project_id: project_id.clone(),
                    task_id: None,
                    session_id: None,
                    run_id: None,
                    kind: "dogfood_readiness".to_string(),
                    artifact_id: Some("artifact-dogfood-readiness".to_string()),
                    confidence: 65,
                    updated_sequence: 0,
                })],
            )
            .expect("append project evidence");

        let dashboard =
            project_dashboard(&state, ProjectDashboardQuery::new(project_id)).expect("dashboard");

        assert_eq!(dashboard.project_evidence.len(), 1);
        assert_eq!(
            dashboard.project_evidence[0].evidence_id,
            EvidenceId::new("evidence-dogfood-readiness")
        );
        assert_eq!(dashboard.project_evidence[0].kind, "dogfood_readiness");
        assert!(dashboard.project_evidence[0].session_id.is_none());
    }

    #[test]
    fn project_dashboard_includes_review_findings() {
        let root = temp_root("query-dashboard-review-findings");
        let state = SqliteStateStore::open(&root).expect("state");
        let project_id = ProjectId::new("project-capo");
        append_agent(&state, &project_id, "agent-active", Some("session-active"));
        append_minimal_session(&state, &project_id, "agent-active", "session-active");
        append_review_finding(&state, &project_id, "review-finding-blocker");

        let dashboard =
            project_dashboard(&state, ProjectDashboardQuery::new(project_id)).expect("dashboard");

        assert_eq!(dashboard.review_findings.len(), 1);
        assert_eq!(
            dashboard.review_findings[0].review_finding_id,
            "review-finding-blocker"
        );
        assert_eq!(dashboard.review_findings[0].finding_kind, "blocker");
        let session = dashboard.agents[0].session.as_ref().expect("session row");
        assert_eq!(session.review_findings.len(), 1);
        assert_eq!(session.review_findings[0].severity, "high");
    }

    #[test]
    fn project_dashboard_includes_task_outcome_reports() {
        let root = temp_root("query-dashboard-task-outcome-reports");
        let state = SqliteStateStore::open(&root).expect("state");
        let project_id = ProjectId::new("project-capo");
        append_agent(&state, &project_id, "agent-active", Some("session-active"));
        append_minimal_session(&state, &project_id, "agent-active", "session-active");
        append_task_outcome_report(&state, &project_id, "task-outcome-report-demo");

        let dashboard =
            project_dashboard(&state, ProjectDashboardQuery::new(project_id)).expect("dashboard");

        assert_eq!(dashboard.task_outcome_reports.len(), 1);
        assert_eq!(
            dashboard.task_outcome_reports[0].task_outcome_report_id,
            "task-outcome-report-demo"
        );
        assert_eq!(
            dashboard.task_outcome_reports[0].review_outcome,
            "reviewed_with_findings"
        );
        let session = dashboard.agents[0].session.as_ref().expect("session row");
        assert_eq!(session.task_outcome_reports.len(), 1);
        assert_eq!(session.task_outcome_reports[0].tool_call_count, 2);
    }

    #[test]
    fn project_dashboard_includes_connectivity_exposures() {
        let root = temp_root("query-dashboard-connectivity");
        let state = SqliteStateStore::open(&root).expect("state");
        let project_id = ProjectId::new("project-capo");
        append_agent(&state, &project_id, "agent-idle", None);
        append_connectivity_exposure(&state, &project_id);

        let dashboard =
            project_dashboard(&state, ProjectDashboardQuery::new(project_id)).expect("dashboard");

        assert_eq!(dashboard.connectivity_exposures.len(), 1);
        let exposure = &dashboard.connectivity_exposures[0];
        assert_eq!(exposure.exposure_id, "exposure-private-control");
        assert_eq!(exposure.status, "blocked_pending_permission");
        assert_eq!(exposure.permission_scope, "network:connect:private_tunnel");
        assert_eq!(exposure.health_status, "unknown");
        assert!(!exposure.reachable);
    }

    #[test]
    fn project_dashboard_selects_runtime_target_status() {
        let root = temp_root("query-dashboard-runtime-target-status");
        let state = SqliteStateStore::open(&root).expect("state");
        let project_id = ProjectId::new("project-capo");
        append_runtime_target(&state, &project_id, "remote-target-1", "disabled");
        append_runtime_target(&state, &project_id, "remote-target-1", "available");

        let dashboard =
            project_dashboard(&state, ProjectDashboardQuery::new(project_id)).expect("dashboard");

        let target = dashboard
            .runtime_target_status("remote-target-1")
            .expect("runtime target status");
        assert_eq!(target.runtime_target_id, "remote-target-1");
        assert_eq!(target.status, "available");
        assert!(
            dashboard
                .runtime_target_status("missing-runtime-target")
                .is_none()
        );
    }

    #[test]
    fn project_dashboard_selects_latest_connectivity_exposure() {
        let root = temp_root("query-dashboard-latest-connectivity");
        let state = SqliteStateStore::open(&root).expect("state");
        let project_id = ProjectId::new("project-capo");
        append_connectivity_exposure(&state, &project_id);
        append_connectivity_exposure_with(
            &state,
            &project_id,
            "exposure-dashboard",
            "capo_server",
            "capo-server-1",
            "dashboard",
            "public",
            "network:expose:public",
            "blocked_pending_permission",
        );
        append_connectivity_exposure_with(
            &state,
            &project_id,
            "exposure-runtime-logs",
            "runtime_target",
            "remote-target-1",
            "logs",
            "private",
            "network:connect:private_tunnel",
            "active",
        );

        let dashboard =
            project_dashboard(&state, ProjectDashboardQuery::new(project_id)).expect("dashboard");

        let latest = dashboard
            .latest_connectivity_exposure(None, None, None)
            .expect("latest exposure");
        assert_eq!(latest.exposure_id, "exposure-runtime-logs");
        let latest_dashboard = dashboard
            .latest_connectivity_exposure(Some("capo_server"), None, Some("dashboard"))
            .expect("latest dashboard exposure");
        assert_eq!(latest_dashboard.exposure_id, "exposure-dashboard");
        let exact = dashboard
            .connectivity_exposure_status("exposure-private-control")
            .expect("exact exposure");
        assert_eq!(exact.owner_kind, "runtime_target");
        assert!(
            dashboard
                .latest_connectivity_exposure(Some("runtime_target"), Some("missing"), None)
                .is_none()
        );
    }

    #[test]
    fn project_dashboard_includes_adapter_dogfood_gate() {
        let root = temp_root("query-dashboard-adapter-gate");
        let state = SqliteStateStore::open(&root).expect("state");
        let project_id = ProjectId::new("project-capo");
        append_agent(&state, &project_id, "agent-idle", None);

        let blocked = project_dashboard(&state, ProjectDashboardQuery::new(project_id.clone()))
            .expect("blocked dashboard");
        assert!(!blocked.adapter_dogfood_gate.ready);
        assert_eq!(
            blocked.adapter_dogfood_gate.status,
            "blocked_pending_real_smoke"
        );
        assert_eq!(
            blocked.adapter_dogfood_gate.blocked_adapters,
            vec!["codex_exec"]
        );

        append_adapter_smoke_report(
            &state,
            &project_id,
            "adapter-smoke-codex-clean",
            "codex_exec",
            "passed",
            "clean",
            true,
        );
        let ready = project_dashboard(&state, ProjectDashboardQuery::new(project_id))
            .expect("ready dashboard");
        assert!(ready.adapter_dogfood_gate.ready);
        assert_eq!(
            ready.adapter_dogfood_gate.status,
            "ready_for_first_real_agent_dogfood"
        );
        assert_eq!(
            ready.adapter_dogfood_gate.proven_adapters,
            vec!["codex_exec"]
        );
    }

    #[test]
    fn project_dogfood_readiness_reports_blockers_and_ready_counts() {
        let root = temp_root("query-dogfood-readiness");
        let state = SqliteStateStore::open(&root).expect("state");
        let project_id = ProjectId::new("project-capo");
        append_agent(&state, &project_id, "agent-idle", None);

        let blocked_dashboard =
            project_dashboard(&state, ProjectDashboardQuery::new(project_id.clone()))
                .expect("blocked dashboard");
        let blocked = blocked_dashboard.dogfood_readiness();
        assert!(!blocked.ready);
        assert_eq!(blocked.status, "blocked_pending_dogfood_prerequisites");
        assert_eq!(
            blocked.blockers,
            vec![
                "real_agent_connector_not_proven",
                "workpad_index_missing",
                "dispatch_chain_missing"
            ]
        );

        append_adapter_smoke_report(
            &state,
            &project_id,
            "adapter-smoke-codex-clean",
            "codex_exec",
            "passed",
            "clean",
            true,
        );
        append_workpad_task(
            &state,
            &project_id,
            "workpads:features:tasks.md#f1",
            "workpads/features/tasks.md",
            "in_progress",
            "observed_only",
        );
        append_adapter_dispatch_plan(&state, &project_id);
        append_adapter_dispatch_replay(&state, &project_id);
        state
            .append_event(
                NewEvent {
                    event_id: "event-dogfood-readiness-evidence".to_string(),
                    kind: EventKind::EvidenceRecorded,
                    actor: "test".to_string(),
                    project_id: Some(project_id.clone()),
                    task_id: None,
                    agent_id: None,
                    session_id: None,
                    run_id: None,
                    turn_id: None,
                    item_id: Some("evidence-dogfood-readiness".to_string()),
                    payload_json: "{}".to_string(),
                    idempotency_key: None,
                    redaction_state: RedactionState::Safe,
                },
                &[ProjectionRecord::Evidence(EvidenceProjection {
                    evidence_id: EvidenceId::new("evidence-dogfood-readiness"),
                    project_id: project_id.clone(),
                    task_id: None,
                    session_id: None,
                    run_id: None,
                    kind: "dogfood_readiness".to_string(),
                    artifact_id: Some("artifact-dogfood-readiness".to_string()),
                    confidence: 90,
                    updated_sequence: 0,
                })],
            )
            .expect("append project evidence");

        let ready_dashboard =
            project_dashboard(&state, ProjectDashboardQuery::new(project_id)).expect("dashboard");
        let ready = ready_dashboard.dogfood_readiness();
        assert!(ready.ready);
        assert_eq!(ready.status, "ready_for_first_dogfood");
        assert!(ready.real_agent_connector_ready);
        assert!(ready.workpad_bridge_ready);
        assert!(ready.dispatch_chain_ready);
        assert_eq!(ready.workpad_task_count, 1);
        assert_eq!(ready.observed_workpad_task_count, 1);
        assert_eq!(ready.dispatch_plan_count, 1);
        assert_eq!(ready.dispatch_replay_count, 1);
        assert_eq!(
            ready.connector_evidence_refs,
            vec!["adapter-smoke-codex-clean"]
        );
        assert_eq!(
            ready.workpad_task_refs,
            vec!["workpads:features:tasks.md#f1"]
        );
        assert_eq!(
            ready.dispatch_chain_refs,
            vec![
                "adapter-dispatch-plan-codex",
                "adapter-dispatch-replay-codex"
            ]
        );
        assert_eq!(
            ready.project_evidence_refs,
            vec!["evidence-dogfood-readiness"]
        );
        assert!(ready.blockers.is_empty());
        assert!(ready.next_actions.is_empty());
    }

    #[test]
    fn project_dashboard_includes_adapter_dispatch_plans() {
        let root = temp_root("query-dashboard-adapter-dispatch");
        let state = SqliteStateStore::open(&root).expect("state");
        let project_id = ProjectId::new("project-capo");
        append_agent(&state, &project_id, "agent-idle", None);
        append_adapter_dispatch_plan(&state, &project_id);
        append_adapter_dispatch_gate(&state, &project_id);
        append_adapter_dispatch_replay(&state, &project_id);
        append_adapter_dispatch_execution_request(&state, &project_id);
        append_adapter_dispatch_prompt_source(&state, &project_id);
        append_adapter_dispatch_prompt_materialization(&state, &project_id);

        let dashboard =
            project_dashboard(&state, ProjectDashboardQuery::new(project_id)).expect("dashboard");

        assert_eq!(dashboard.adapter_dispatch_plans.len(), 1);
        let plan = &dashboard.adapter_dispatch_plans[0];
        assert_eq!(plan.dispatch_plan_id, "adapter-dispatch-plan-codex");
        assert_eq!(plan.adapter_kind, "codex_exec");
        assert_eq!(plan.credential_scope, "user_local_subscription");
        assert_eq!(plan.runtime_prompt_policy, "not_rendered");
        assert!(!plan.provider_cli_executed);
        assert_eq!(plan.status, "planned");
        assert_eq!(dashboard.adapter_dispatch_gates.len(), 1);
        let gate = &dashboard.adapter_dispatch_gates[0];
        assert_eq!(gate.dispatch_plan_id, "adapter-dispatch-plan-codex");
        assert_eq!(gate.adapter_kind, "codex_exec");
        assert_eq!(gate.status, "blocked");
        assert!(!gate.provider_cli_execution_allowed);
        assert!(!gate.provider_cli_executed);
        assert_eq!(dashboard.adapter_dispatch_replays.len(), 1);
        let replay = &dashboard.adapter_dispatch_replays[0];
        assert_eq!(replay.dispatch_plan_id, "adapter-dispatch-plan-codex");
        assert_eq!(replay.dispatch_gate_id, "adapter-dispatch-gate-codex");
        assert_eq!(replay.adapter_kind, "codex_exec");
        assert_eq!(replay.input_event_count, 4);
        assert!(!replay.provider_cli_executed);
        assert_eq!(replay.raw_content_policy, "content_hashed_not_rendered");
        assert_eq!(dashboard.adapter_dispatch_execution_requests.len(), 1);
        let request = &dashboard.adapter_dispatch_execution_requests[0];
        assert_eq!(request.dispatch_plan_id, "adapter-dispatch-plan-codex");
        assert_eq!(request.dispatch_gate_id, "adapter-dispatch-gate-codex");
        assert_eq!(request.status, "waiting_on_explicit_provider_opt_in");
        assert_eq!(request.opt_in_env, "CAPO_RUN_CODEX_LOCAL_DISPATCH");
        assert!(request.provider_cli_execution_allowed);
        assert!(!request.provider_cli_executed);
        assert_eq!(dashboard.adapter_dispatch_prompt_sources.len(), 1);
        let source = &dashboard.adapter_dispatch_prompt_sources[0];
        assert_eq!(source.dispatch_plan_id, "adapter-dispatch-plan-codex");
        assert_eq!(source.source_kind, "workpad_task");
        assert_eq!(
            source.materialization_status,
            "replayable_if_source_hash_matches"
        );
        assert_eq!(source.raw_prompt_policy, "not_rendered");
        assert_eq!(dashboard.adapter_dispatch_prompt_materializations.len(), 1);
        let materialization = &dashboard.adapter_dispatch_prompt_materializations[0];
        assert_eq!(
            materialization.dispatch_plan_id,
            "adapter-dispatch-plan-codex"
        );
        assert_eq!(materialization.status, "ready_without_rendering_prompt");
        assert_eq!(materialization.raw_prompt_policy, "not_rendered");
    }

    #[test]
    fn project_dashboard_summarizes_adapter_dispatch_status() {
        let root = temp_root("query-adapter-dispatch-status");
        let state = SqliteStateStore::open(&root).expect("state");
        let project_id = ProjectId::new("project-capo");
        append_adapter_smoke_report(
            &state,
            &project_id,
            "adapter-smoke-codex-clean",
            "codex_exec",
            "passed",
            "clean",
            true,
        );
        append_adapter_dispatch_plan(&state, &project_id);
        append_adapter_dispatch_gate(&state, &project_id);
        append_adapter_dispatch_replay(&state, &project_id);
        append_adapter_dispatch_execution(&state, &project_id);

        let dashboard =
            project_dashboard(&state, ProjectDashboardQuery::new(project_id)).expect("dashboard");
        let status = dashboard
            .adapter_dispatch_status("adapter-dispatch-plan-codex")
            .expect("dispatch status");

        assert_eq!(status.dispatch_plan_id, "adapter-dispatch-plan-codex");
        assert_eq!(status.adapter_kind, "codex_exec");
        assert_eq!(status.provider_kind, "codex_subscription");
        assert_eq!(status.credential_scope, "user_local_subscription");
        assert_eq!(
            status.dogfood_gate_status,
            "ready_for_first_real_agent_dogfood"
        );
        assert_eq!(
            status.latest_dispatch_gate_id,
            "adapter-dispatch-gate-codex"
        );
        assert_eq!(
            status.latest_dispatch_replay_id,
            "adapter-dispatch-replay-codex"
        );
        assert_eq!(
            status.latest_dispatch_execution_id,
            "adapter-dispatch-execution-codex"
        );
        assert_eq!(status.latest_execution_status, "completed");
        assert!(status.latest_execution_provider_cli_executed);
        assert_eq!(
            status.latest_execution_stdout_artifact_id,
            "artifact-dispatch-stdout"
        );
        assert_eq!(
            status.next_action,
            "inspect_execution_artifacts_and_export_evidence"
        );
        assert!(
            dashboard
                .adapter_dispatch_status("missing-dispatch-plan")
                .is_none()
        );
    }

    #[test]
    fn project_dashboard_selects_latest_adapter_dispatch_status() {
        let root = temp_root("query-latest-adapter-dispatch-status");
        let state = SqliteStateStore::open(&root).expect("state");
        let project_id = ProjectId::new("project-capo");
        append_adapter_dispatch_plan(&state, &project_id);
        append_adapter_dispatch_plan_named(
            &state,
            &project_id,
            "adapter-dispatch-plan-reviewer",
            "reviewer",
            "session-reviewer",
            "run-reviewer",
        );
        append_adapter_dispatch_execution_named(
            &state,
            &project_id,
            "adapter-dispatch-plan-codex",
            "adapter-dispatch-execution-codex",
            false,
        );

        let dashboard =
            project_dashboard(&state, ProjectDashboardQuery::new(project_id)).expect("dashboard");
        let latest = dashboard
            .latest_adapter_dispatch_status(None)
            .expect("latest dispatch status");
        assert_eq!(latest.dispatch_plan_id, "adapter-dispatch-plan-codex");
        assert_eq!(latest.latest_execution_status, "blocked_missing_opt_in");
        assert_eq!(latest.next_action, "resolve_latest_execution_blocker");

        let reviewer = dashboard
            .latest_adapter_dispatch_status(Some("reviewer"))
            .expect("reviewer dispatch status");
        assert_eq!(reviewer.dispatch_plan_id, "adapter-dispatch-plan-reviewer");
        assert_eq!(reviewer.agent_name, "reviewer");
        assert!(
            dashboard
                .latest_adapter_dispatch_status(Some("missing-agent"))
                .is_none()
        );
    }

    #[test]
    fn project_dashboard_includes_workpad_tasks() {
        let root = temp_root("query-dashboard-workpad-tasks");
        let state = SqliteStateStore::open(&root).expect("state");
        let project_id = ProjectId::new("project-capo");
        append_agent(&state, &project_id, "agent-idle", None);
        append_workpad_task(
            &state,
            &project_id,
            "workpads:features:tasks.md#f2",
            "workpads/features/tasks.md",
            "in_progress",
            "observed_only",
        );

        let dashboard =
            project_dashboard(&state, ProjectDashboardQuery::new(project_id)).expect("dashboard");

        assert_eq!(dashboard.workpad_tasks.len(), 1);
        assert_eq!(
            dashboard.workpad_tasks[0].workpad_task_id,
            "workpads:features:tasks.md#f2"
        );
        assert_eq!(dashboard.workpad_tasks[0].observed_status, "in_progress");
        assert_eq!(
            dashboard.workpad_tasks[0].capo_execution_status,
            "observed_only"
        );
    }

    #[test]
    fn project_dashboard_filters_workpad_tasks_without_filtering_agents() {
        let root = temp_root("query-dashboard-workpad-filter");
        let state = SqliteStateStore::open(&root).expect("state");
        let project_id = ProjectId::new("project-capo");
        append_agent(&state, &project_id, "agent-active", Some("session-active"));
        append_minimal_session(&state, &project_id, "agent-active", "session-active");
        append_workpad_task(
            &state,
            &project_id,
            "workpads:features:tasks.md#f2",
            "workpads/features/tasks.md",
            "in_progress",
            "observed_only",
        );
        append_workpad_task(
            &state,
            &project_id,
            "workpads:features:dashboard.md#ds3",
            "workpads/features/dashboard.md",
            "completed",
            "imported",
        );

        let dashboard = project_dashboard(
            &state,
            ProjectDashboardQuery::new(project_id.clone())
                .with_workpad_path("workpads/features/tasks.md"),
        )
        .expect("dashboard by workpad path");
        assert_eq!(dashboard.agents.len(), 1);
        assert_eq!(dashboard.workpad_tasks.len(), 1);
        assert_eq!(
            dashboard.workpad_tasks[0].workpad_task_id,
            "workpads:features:tasks.md#f2"
        );

        let imported_dashboard = project_dashboard(
            &state,
            ProjectDashboardQuery::new(project_id).with_workpad_status("imported"),
        )
        .expect("dashboard by workpad status");
        assert_eq!(imported_dashboard.agents.len(), 1);
        assert_eq!(imported_dashboard.workpad_tasks.len(), 1);
        assert_eq!(
            imported_dashboard.workpad_tasks[0].workpad_task_id,
            "workpads:features:dashboard.md#ds3"
        );
    }

    #[test]
    fn project_dashboard_selects_next_actionable_workpad_task() {
        let root = temp_root("query-dashboard-next-workpad-task");
        let state = SqliteStateStore::open(&root).expect("state");
        let project_id = ProjectId::new("project-capo");
        append_workpad_task(
            &state,
            &project_id,
            "workpads:features:remote-runtime.md#rr7",
            "workpads/features/remote-runtime.md",
            "waiting_on_opt_in",
            "observed_only",
        );
        append_workpad_task(
            &state,
            &project_id,
            "workpads:features:voice.md#v7",
            "workpads/features/voice.md",
            "pending",
            "observed_only",
        );
        append_workpad_task(
            &state,
            &project_id,
            "workpads:features:tasks.md#f1",
            "workpads/features/tasks.md",
            "in_progress",
            "imported",
        );
        append_workpad_task(
            &state,
            &project_id,
            "workpads:features:tasks.md#f6",
            "workpads/features/tasks.md",
            "completed",
            "observed_only",
        );

        let dashboard =
            project_dashboard(&state, ProjectDashboardQuery::new(project_id)).expect("dashboard");

        assert_eq!(dashboard.next_workpad_candidate_count(), 2);
        let next = dashboard.next_workpad_task().expect("next workpad task");
        assert_eq!(next.workpad_task_id, "workpads:features:voice.md#v7");
        assert_eq!(next.observed_status, "pending");
        assert_eq!(next.capo_execution_status, "observed_only");
    }

    #[test]
    fn project_dashboard_filters_project_and_keeps_idle_agents() {
        let root = temp_root("query-dashboard-filter");
        let state = SqliteStateStore::open(&root).expect("state");
        let project_id = ProjectId::new("project-capo");
        let other_project_id = ProjectId::new("project-other");

        append_agent(&state, &project_id, "agent-active", Some("session-active"));
        append_minimal_session(&state, &project_id, "agent-active", "session-active");
        append_agent(&state, &project_id, "agent-idle", None);
        append_agent(&state, &other_project_id, "agent-other", None);

        let dashboard =
            project_dashboard(&state, ProjectDashboardQuery::new(project_id)).expect("dashboard");

        assert_eq!(dashboard.agents.len(), 2);
        assert_eq!(dashboard.active_session_count(), 1);
        assert!(
            dashboard
                .agents
                .iter()
                .any(|row| { row.agent.name == "agent-active" && row.session.is_some() })
        );
        assert!(
            dashboard
                .agents
                .iter()
                .any(|row| { row.agent.name == "agent-idle" && row.session.is_none() })
        );
        assert!(
            !dashboard
                .agents
                .iter()
                .any(|row| row.agent.name == "agent-other")
        );
    }

    #[test]
    fn project_dashboard_honors_recent_event_limit() {
        let root = temp_root("query-dashboard-limit");
        let state = SqliteStateStore::open(&root).expect("state");
        let project_id = ProjectId::new("project-capo");
        let session_id = SessionId::new("session-limited");

        append_agent(
            &state,
            &project_id,
            "agent-limited",
            Some(session_id.as_str()),
        );
        append_minimal_session(&state, &project_id, "agent-limited", session_id.as_str());
        for index in 0..4 {
            append_session_event(&state, &project_id, &session_id, index);
        }

        let mut query = ProjectDashboardQuery::new(project_id);
        query.recent_event_limit = 2;
        let dashboard = project_dashboard(&state, query).expect("dashboard");
        let recent_events = &dashboard.agents[0]
            .session
            .as_ref()
            .expect("session")
            .recent_events;

        assert_eq!(recent_events.len(), 2);
        assert_eq!(recent_events[0].event_id, "event-extra-2");
        assert_eq!(recent_events[1].event_id, "event-extra-3");
    }

    #[test]
    fn project_dashboard_filters_by_session_and_status() {
        let root = temp_root("query-dashboard-session-filter");
        let state = SqliteStateStore::open(&root).expect("state");
        let project_id = ProjectId::new("project-capo");

        append_agent(&state, &project_id, "agent-active", Some("session-active"));
        append_minimal_session(&state, &project_id, "agent-active", "session-active");
        append_agent(&state, &project_id, "agent-idle", None);

        let by_session = project_dashboard(
            &state,
            ProjectDashboardQuery::new(project_id.clone())
                .with_session_id(SessionId::new("session-active")),
        )
        .expect("dashboard by session");
        assert_eq!(by_session.agents.len(), 1);
        assert_eq!(by_session.agents[0].agent.name, "agent-active");

        let by_agent_status = project_dashboard(
            &state,
            ProjectDashboardQuery::new(project_id.clone()).with_status("available"),
        )
        .expect("dashboard by agent status");
        assert_eq!(by_agent_status.agents.len(), 1);
        assert_eq!(by_agent_status.agents[0].agent.name, "agent-idle");

        let by_session_status = project_dashboard(
            &state,
            ProjectDashboardQuery::new(project_id).with_status("active"),
        )
        .expect("dashboard by session status");
        assert_eq!(by_session_status.agents.len(), 1);
        assert_eq!(by_session_status.agents[0].agent.name, "agent-active");
    }

    #[test]
    fn project_dashboard_fails_closed_on_missing_current_session() {
        let root = temp_root("query-dashboard-missing-session");
        let state = SqliteStateStore::open(&root).expect("state");
        let project_id = ProjectId::new("project-capo");

        append_agent(&state, &project_id, "agent-stale", Some("session-missing"));

        let error = project_dashboard(&state, ProjectDashboardQuery::new(project_id))
            .expect_err("missing session should fail closed");

        assert!(matches!(
            error,
            capo_state::StateError::MissingReadModel {
                kind: "session",
                ..
            }
        ));
    }

    fn append_agent(
        state: &SqliteStateStore,
        project_id: &ProjectId,
        name: &str,
        current_session_id: Option<&str>,
    ) {
        let agent_id = AgentId::new(name);
        state
            .append_event(
                NewEvent {
                    event_id: format!("event-agent-{name}"),
                    kind: EventKind::AgentRegistered,
                    actor: "test".to_string(),
                    project_id: Some(project_id.clone()),
                    task_id: None,
                    agent_id: Some(agent_id.clone()),
                    session_id: current_session_id.map(SessionId::new),
                    run_id: None,
                    turn_id: None,
                    item_id: None,
                    payload_json: "{}".to_string(),
                    idempotency_key: None,
                    redaction_state: RedactionState::Safe,
                },
                &[ProjectionRecord::Agent(AgentProjection {
                    agent_id,
                    project_id: project_id.clone(),
                    name: name.to_string(),
                    status: if current_session_id.is_some() {
                        "running".to_string()
                    } else {
                        "available".to_string()
                    },
                    current_session_id: current_session_id.map(SessionId::new),
                    updated_sequence: 0,
                })],
            )
            .expect("append agent");
    }

    fn append_minimal_session(
        state: &SqliteStateStore,
        project_id: &ProjectId,
        agent_name: &str,
        session_id: &str,
    ) {
        let task_id = TaskId::new(format!("task-{agent_name}"));
        let run_id = RunId::new(format!("run-{agent_name}"));
        let session_id = SessionId::new(session_id);
        state
            .append_event(
                NewEvent {
                    event_id: format!("event-session-{session_id}"),
                    kind: EventKind::SessionStarted,
                    actor: "test".to_string(),
                    project_id: Some(project_id.clone()),
                    task_id: Some(task_id.clone()),
                    agent_id: Some(AgentId::new(agent_name)),
                    session_id: Some(session_id.clone()),
                    run_id: Some(run_id.clone()),
                    turn_id: None,
                    item_id: None,
                    payload_json: "{}".to_string(),
                    idempotency_key: None,
                    redaction_state: RedactionState::Safe,
                },
                &[
                    ProjectionRecord::Session(SessionProjection {
                        session_id: session_id.clone(),
                        project_id: project_id.clone(),
                        task_id: Some(task_id),
                        agent_id: AgentId::new(agent_name),
                        title: "Session".to_string(),
                        status: "active".to_string(),
                        current_goal: "prove query".to_string(),
                        latest_summary: None,
                        latest_confidence: None,
                        latest_blocker: None,
                        updated_sequence: 0,
                    }),
                    ProjectionRecord::Run(RunProjection {
                        run_id,
                        session_id,
                        status: "running".to_string(),
                        recovery_of_run_id: None,
                        updated_sequence: 0,
                    }),
                ],
            )
            .expect("append session");
    }

    fn append_session_event(
        state: &SqliteStateStore,
        project_id: &ProjectId,
        session_id: &SessionId,
        index: usize,
    ) {
        state
            .append_event(
                NewEvent {
                    event_id: format!("event-extra-{index}"),
                    kind: EventKind::SessionSummaryUpdated,
                    actor: "test".to_string(),
                    project_id: Some(project_id.clone()),
                    task_id: None,
                    agent_id: None,
                    session_id: Some(session_id.clone()),
                    run_id: None,
                    turn_id: None,
                    item_id: None,
                    payload_json: "{}".to_string(),
                    idempotency_key: None,
                    redaction_state: RedactionState::Safe,
                },
                &[],
            )
            .expect("append session event");
    }

    fn append_review_finding(state: &SqliteStateStore, project_id: &ProjectId, finding_id: &str) {
        state
            .append_event(
                NewEvent {
                    event_id: format!("event-{finding_id}"),
                    kind: EventKind::ReviewFindingRecorded,
                    actor: "test".to_string(),
                    project_id: Some(project_id.clone()),
                    task_id: Some(TaskId::new("task-agent-active")),
                    agent_id: Some(AgentId::new("agent-active")),
                    session_id: Some(SessionId::new("session-active")),
                    run_id: Some(RunId::new("run-agent-active")),
                    turn_id: None,
                    item_id: Some(finding_id.to_string()),
                    payload_json: "{}".to_string(),
                    idempotency_key: None,
                    redaction_state: RedactionState::Safe,
                },
                &[ProjectionRecord::ReviewFinding(ReviewFindingProjection {
                    review_finding_id: finding_id.to_string(),
                    project_id: project_id.clone(),
                    task_id: TaskId::new("task-agent-active"),
                    session_id: SessionId::new("session-active"),
                    run_id: Some(RunId::new("run-agent-active")),
                    tool_call_id: None,
                    workpad_task_id: Some("ME3".to_string()),
                    reviewer: "focused-review".to_string(),
                    finding_kind: "blocker".to_string(),
                    severity: "high".to_string(),
                    summary: "Review blocker needs follow-up.".to_string(),
                    status: "open".to_string(),
                    evidence_artifact_id: Some("artifact-review-finding-blocker".to_string()),
                    follow_up: Some("Create follow-up workpad task.".to_string()),
                    updated_sequence: 0,
                })],
            )
            .expect("append review finding");
    }

    fn append_task_outcome_report(
        state: &SqliteStateStore,
        project_id: &ProjectId,
        report_id: &str,
    ) {
        state
            .append_event(
                NewEvent {
                    event_id: format!("event-{report_id}"),
                    kind: EventKind::TaskOutcomeReportGenerated,
                    actor: "test".to_string(),
                    project_id: Some(project_id.clone()),
                    task_id: Some(TaskId::new("task-agent-active")),
                    agent_id: Some(AgentId::new("agent-active")),
                    session_id: Some(SessionId::new("session-active")),
                    run_id: Some(RunId::new("run-agent-active")),
                    turn_id: None,
                    item_id: Some(report_id.to_string()),
                    payload_json: "{}".to_string(),
                    idempotency_key: None,
                    redaction_state: RedactionState::Safe,
                },
                &[ProjectionRecord::TaskOutcomeReport(
                    TaskOutcomeReportProjection {
                        task_outcome_report_id: report_id.to_string(),
                        project_id: project_id.clone(),
                        task_id: TaskId::new("task-agent-active"),
                        session_id: SessionId::new("session-active"),
                        run_id: RunId::new("run-agent-active"),
                        outcome_status: "completed".to_string(),
                        started_sequence: 10,
                        completed_sequence: 20,
                        duration_sequence_span: 10,
                        action_count: 7,
                        tool_call_count: 2,
                        evidence_count: 3,
                        memory_packet_count: 1,
                        confidence: Some(82),
                        blocker: None,
                        review_outcome: "reviewed_with_findings".to_string(),
                        report_artifact_id: Some("artifact-task-outcome-report-demo".to_string()),
                        updated_sequence: 0,
                    },
                )],
            )
            .expect("append task outcome report");
    }

    fn append_connectivity_exposure(state: &SqliteStateStore, project_id: &ProjectId) {
        append_connectivity_exposure_with(
            state,
            project_id,
            "exposure-private-control",
            "runtime_target",
            "remote-target-1",
            "control",
            "private",
            "network:connect:private_tunnel",
            "blocked_pending_permission",
        );
    }

    fn append_runtime_target(
        state: &SqliteStateStore,
        project_id: &ProjectId,
        runtime_target_id: &str,
        status: &str,
    ) {
        state
            .append_event(
                NewEvent {
                    event_id: format!("event-runtime-target-{runtime_target_id}-{status}"),
                    kind: EventKind::RuntimeTargetRegistered,
                    actor: "test".to_string(),
                    project_id: Some(project_id.clone()),
                    task_id: None,
                    agent_id: None,
                    session_id: None,
                    run_id: None,
                    turn_id: None,
                    item_id: Some(runtime_target_id.to_string()),
                    payload_json: "{}".to_string(),
                    idempotency_key: None,
                    redaction_state: RedactionState::Safe,
                },
                &[ProjectionRecord::RuntimeTarget(RuntimeTargetProjection {
                    runtime_target_id: runtime_target_id.to_string(),
                    project_id: project_id.clone(),
                    name: "remote target".to_string(),
                    runner_kind: "remote_process".to_string(),
                    workspace_root: "/tmp/capo-runtime-workspace".to_string(),
                    artifact_root: "/tmp/capo-runtime-artifacts".to_string(),
                    default_cwd: "/tmp/capo-runtime-workspace".to_string(),
                    capability_profile_id: "read-only-local".to_string(),
                    connectivity_endpoint_id: Some("endpoint-runtime-1".to_string()),
                    status: status.to_string(),
                    updated_sequence: 0,
                })],
            )
            .expect("append runtime target");
    }

    #[allow(clippy::too_many_arguments)]
    fn append_connectivity_exposure_with(
        state: &SqliteStateStore,
        project_id: &ProjectId,
        exposure_id: &str,
        owner_kind: &str,
        owner_id: &str,
        channel_kind: &str,
        exposure_scope: &str,
        permission_scope: &str,
        status: &str,
    ) {
        state
            .append_event(
                NewEvent {
                    event_id: format!("event-{exposure_id}"),
                    kind: EventKind::ConnectivityExposureRequested,
                    actor: "test".to_string(),
                    project_id: Some(project_id.clone()),
                    task_id: None,
                    agent_id: None,
                    session_id: None,
                    run_id: None,
                    turn_id: None,
                    item_id: Some(exposure_id.to_string()),
                    payload_json: "{}".to_string(),
                    idempotency_key: None,
                    redaction_state: RedactionState::Safe,
                },
                &[ProjectionRecord::ConnectivityExposure(
                    ConnectivityExposureProjection {
                        exposure_id: exposure_id.to_string(),
                        project_id: project_id.clone(),
                        connectivity_endpoint_id: format!("endpoint-{exposure_id}"),
                        owner_kind: owner_kind.to_string(),
                        owner_id: owner_id.to_string(),
                        channel_kind: channel_kind.to_string(),
                        exposure: exposure_scope.to_string(),
                        permission_scope: permission_scope.to_string(),
                        status: status.to_string(),
                        capability_grant_id: None,
                        health_status: "unknown".to_string(),
                        reachable: false,
                        revoked_at: None,
                        updated_sequence: 0,
                    },
                )],
            )
            .expect("append connectivity exposure");
    }

    fn append_adapter_smoke_report(
        state: &SqliteStateStore,
        project_id: &ProjectId,
        smoke_report_id: &str,
        adapter_kind: &str,
        smoke_status: &str,
        credential_scan_status: &str,
        marker_found: bool,
    ) {
        state
            .append_event(
                NewEvent {
                    event_id: format!("event-{smoke_report_id}"),
                    kind: EventKind::AdapterSmokeRecorded,
                    actor: "test".to_string(),
                    project_id: Some(project_id.clone()),
                    task_id: None,
                    agent_id: None,
                    session_id: None,
                    run_id: None,
                    turn_id: None,
                    item_id: Some(smoke_report_id.to_string()),
                    payload_json: "{}".to_string(),
                    idempotency_key: None,
                    redaction_state: RedactionState::Safe,
                },
                &[ProjectionRecord::AdapterSmokeReport(
                    AdapterSmokeReportProjection {
                        smoke_report_id: smoke_report_id.to_string(),
                        project_id: project_id.clone(),
                        adapter_kind: adapter_kind.to_string(),
                        smoke_status: smoke_status.to_string(),
                        credential_scan_status: credential_scan_status.to_string(),
                        marker_found,
                        artifact_root: None,
                        reason: "test smoke evidence".to_string(),
                        dogfood_readiness_effect: if smoke_status == "passed"
                            && credential_scan_status == "clean"
                            && marker_found
                        {
                            "real_agent_connector_proven".to_string()
                        } else {
                            "real_subscription_smoke_not_recorded".to_string()
                        },
                        updated_sequence: 0,
                    },
                )],
            )
            .expect("append adapter smoke report");
    }

    fn append_adapter_dispatch_plan(state: &SqliteStateStore, project_id: &ProjectId) {
        append_adapter_dispatch_plan_named(
            state,
            project_id,
            "adapter-dispatch-plan-codex",
            "codex",
            "session-codex",
            "run-codex",
        );
    }

    fn append_adapter_dispatch_plan_named(
        state: &SqliteStateStore,
        project_id: &ProjectId,
        dispatch_plan_id: &str,
        agent_name: &str,
        session_id: &str,
        run_id: &str,
    ) {
        state
            .append_event(
                NewEvent {
                    event_id: format!("event-{dispatch_plan_id}"),
                    kind: EventKind::AdapterDispatchPlanned,
                    actor: "test".to_string(),
                    project_id: Some(project_id.clone()),
                    task_id: None,
                    agent_id: Some(AgentId::new(format!("agent-{agent_name}"))),
                    session_id: Some(SessionId::new(session_id)),
                    run_id: Some(RunId::new(run_id)),
                    turn_id: None,
                    item_id: Some(dispatch_plan_id.to_string()),
                    payload_json: "{}".to_string(),
                    idempotency_key: None,
                    redaction_state: RedactionState::Safe,
                },
                &[ProjectionRecord::AdapterDispatchPlan(
                    AdapterDispatchPlanProjection {
                        dispatch_plan_id: dispatch_plan_id.to_string(),
                        project_id: project_id.clone(),
                        adapter_kind: "codex_exec".to_string(),
                        provider_kind: "codex_subscription".to_string(),
                        credential_scope: "user_local_subscription".to_string(),
                        agent_id: AgentId::new(format!("agent-{agent_name}")),
                        agent_name: agent_name.to_string(),
                        session_id: SessionId::new(session_id),
                        run_id: RunId::new(run_id),
                        runtime_program: "codex".to_string(),
                        runtime_arg_count: 9,
                        runtime_prompt_policy: "not_rendered".to_string(),
                        runtime_cwd: "/tmp/capo-workspace".to_string(),
                        artifact_root: "/tmp/capo-artifacts".to_string(),
                        request_env_count: 0,
                        env_allowlist_count: 7,
                        redaction_rule_count: 6,
                        stdout_format: "jsonl".to_string(),
                        stderr_policy: "logs_redacted".to_string(),
                        provider_cli_executed: false,
                        status: "planned".to_string(),
                        updated_sequence: 0,
                    },
                )],
            )
            .expect("append adapter dispatch plan");
    }

    fn append_adapter_dispatch_gate(state: &SqliteStateStore, project_id: &ProjectId) {
        state
            .append_event(
                NewEvent {
                    event_id: "event-adapter-dispatch-gate-codex".to_string(),
                    kind: EventKind::AdapterDispatchGateChecked,
                    actor: "test".to_string(),
                    project_id: Some(project_id.clone()),
                    task_id: None,
                    agent_id: Some(AgentId::new("agent-codex")),
                    session_id: Some(SessionId::new("session-codex")),
                    run_id: Some(RunId::new("run-codex")),
                    turn_id: None,
                    item_id: Some("adapter-dispatch-gate-codex".to_string()),
                    payload_json: "{}".to_string(),
                    idempotency_key: None,
                    redaction_state: RedactionState::Safe,
                },
                &[ProjectionRecord::AdapterDispatchGate(
                    AdapterDispatchGateProjection {
                        dispatch_gate_id: "adapter-dispatch-gate-codex".to_string(),
                        project_id: project_id.clone(),
                        dispatch_plan_id: "adapter-dispatch-plan-codex".to_string(),
                        adapter_kind: "codex_exec".to_string(),
                        provider_cli_execution_allowed: false,
                        status: "blocked".to_string(),
                        required_dogfood_gate: "blocked_pending_real_smoke".to_string(),
                        reason_codes: "codex_exec:real_subscription_smoke_not_recorded".to_string(),
                        provider_cli_executed: false,
                        runtime_prompt_policy: "not_rendered".to_string(),
                        updated_sequence: 0,
                    },
                )],
            )
            .expect("append adapter dispatch gate");
    }

    fn append_adapter_dispatch_replay(state: &SqliteStateStore, project_id: &ProjectId) {
        state
            .append_event(
                NewEvent {
                    event_id: "event-adapter-dispatch-replay-codex".to_string(),
                    kind: EventKind::AdapterDispatchReplayed,
                    actor: "test".to_string(),
                    project_id: Some(project_id.clone()),
                    task_id: None,
                    agent_id: Some(AgentId::new("agent-codex")),
                    session_id: Some(SessionId::new("session-codex")),
                    run_id: Some(RunId::new("run-codex")),
                    turn_id: None,
                    item_id: Some("adapter-dispatch-replay-codex".to_string()),
                    payload_json: "{}".to_string(),
                    idempotency_key: None,
                    redaction_state: RedactionState::Safe,
                },
                &[ProjectionRecord::AdapterDispatchReplay(
                    AdapterDispatchReplayProjection {
                        dispatch_replay_id: "adapter-dispatch-replay-codex".to_string(),
                        project_id: project_id.clone(),
                        dispatch_plan_id: "adapter-dispatch-plan-codex".to_string(),
                        dispatch_gate_id: "adapter-dispatch-gate-codex".to_string(),
                        adapter_kind: "codex_exec".to_string(),
                        session_id: SessionId::new("session-codex"),
                        run_id: RunId::new("run-codex"),
                        fixture_path: "fixtures/codex-exec.jsonl".to_string(),
                        fixture_hash: "fixture-hash".to_string(),
                        input_event_count: 4,
                        appended_event_count: 4,
                        tool_event_count: 2,
                        summary_event_count: 1,
                        completed_turn_count: 1,
                        provider_cli_executed: false,
                        raw_content_policy: "content_hashed_not_rendered".to_string(),
                        updated_sequence: 0,
                    },
                )],
            )
            .expect("append adapter dispatch replay");
    }

    fn append_adapter_dispatch_execution_request(state: &SqliteStateStore, project_id: &ProjectId) {
        state
            .append_event(
                NewEvent {
                    event_id: "event-adapter-dispatch-execution-request-codex".to_string(),
                    kind: EventKind::AdapterDispatchExecutionRequested,
                    actor: "test".to_string(),
                    project_id: Some(project_id.clone()),
                    task_id: None,
                    agent_id: Some(AgentId::new("agent-codex")),
                    session_id: Some(SessionId::new("session-codex")),
                    run_id: Some(RunId::new("run-codex")),
                    turn_id: None,
                    item_id: Some("adapter-dispatch-execution-request-codex".to_string()),
                    payload_json: "{}".to_string(),
                    idempotency_key: None,
                    redaction_state: RedactionState::Safe,
                },
                &[ProjectionRecord::AdapterDispatchExecutionRequest(
                    AdapterDispatchExecutionRequestProjection {
                        execution_request_id: "adapter-dispatch-execution-request-codex"
                            .to_string(),
                        project_id: project_id.clone(),
                        dispatch_plan_id: "adapter-dispatch-plan-codex".to_string(),
                        dispatch_gate_id: "adapter-dispatch-gate-codex".to_string(),
                        adapter_kind: "codex_exec".to_string(),
                        provider_cli_execution_allowed: true,
                        provider_cli_executed: false,
                        status: "waiting_on_explicit_provider_opt_in".to_string(),
                        opt_in_env: "CAPO_RUN_CODEX_LOCAL_DISPATCH".to_string(),
                        runtime_prompt_policy: "not_rendered".to_string(),
                        reason_codes: "explicit_provider_execution_opt_in_required".to_string(),
                        updated_sequence: 0,
                    },
                )],
            )
            .expect("append adapter dispatch execution request");
    }

    fn append_adapter_dispatch_prompt_source(state: &SqliteStateStore, project_id: &ProjectId) {
        state
            .append_event(
                NewEvent {
                    event_id: "event-adapter-dispatch-prompt-source-codex".to_string(),
                    kind: EventKind::AdapterDispatchPromptSourceRecorded,
                    actor: "test".to_string(),
                    project_id: Some(project_id.clone()),
                    task_id: None,
                    agent_id: Some(AgentId::new("agent-codex")),
                    session_id: Some(SessionId::new("session-codex")),
                    run_id: Some(RunId::new("run-codex")),
                    turn_id: None,
                    item_id: Some("adapter-dispatch-prompt-source-codex".to_string()),
                    payload_json: "{}".to_string(),
                    idempotency_key: None,
                    redaction_state: RedactionState::Safe,
                },
                &[ProjectionRecord::AdapterDispatchPromptSource(
                    AdapterDispatchPromptSourceProjection {
                        prompt_source_id: "adapter-dispatch-prompt-source-codex".to_string(),
                        project_id: project_id.clone(),
                        dispatch_plan_id: "adapter-dispatch-plan-codex".to_string(),
                        prompt_hash: "prompt-hash".to_string(),
                        source_kind: "workpad_task".to_string(),
                        source_ref: Some("workpads/features/tasks.md#f1".to_string()),
                        source_hash: Some("source-hash".to_string()),
                        materialization_status: "replayable_if_source_hash_matches".to_string(),
                        raw_prompt_policy: "not_rendered".to_string(),
                        updated_sequence: 0,
                    },
                )],
            )
            .expect("append adapter dispatch prompt source");
    }

    fn append_adapter_dispatch_prompt_materialization(
        state: &SqliteStateStore,
        project_id: &ProjectId,
    ) {
        state
            .append_event(
                NewEvent {
                    event_id: "event-adapter-dispatch-prompt-materialization-codex".to_string(),
                    kind: EventKind::AdapterDispatchPromptMaterialized,
                    actor: "test".to_string(),
                    project_id: Some(project_id.clone()),
                    task_id: None,
                    agent_id: None,
                    session_id: None,
                    run_id: None,
                    turn_id: None,
                    item_id: Some("adapter-dispatch-prompt-materialization-codex".to_string()),
                    payload_json: "{}".to_string(),
                    idempotency_key: None,
                    redaction_state: RedactionState::Safe,
                },
                &[ProjectionRecord::AdapterDispatchPromptMaterialization(
                    AdapterDispatchPromptMaterializationProjection {
                        materialization_id: "adapter-dispatch-prompt-materialization-codex"
                            .to_string(),
                        project_id: project_id.clone(),
                        dispatch_plan_id: "adapter-dispatch-plan-codex".to_string(),
                        prompt_source_id: "adapter-dispatch-prompt-source-codex".to_string(),
                        source_kind: "workpad_task".to_string(),
                        source_ref: Some("workpads/features/tasks.md#f1".to_string()),
                        expected_source_hash: Some("source-hash".to_string()),
                        observed_source_hash: Some("source-hash".to_string()),
                        expected_prompt_hash: "prompt-hash".to_string(),
                        materialized_prompt_hash: Some("prompt-hash".to_string()),
                        status: "ready_without_rendering_prompt".to_string(),
                        raw_prompt_policy: "not_rendered".to_string(),
                        reason_codes: "prompt_hash_matches_source".to_string(),
                        updated_sequence: 0,
                    },
                )],
            )
            .expect("append adapter dispatch prompt materialization");
    }

    fn append_adapter_dispatch_execution(state: &SqliteStateStore, project_id: &ProjectId) {
        append_adapter_dispatch_execution_named(
            state,
            project_id,
            "adapter-dispatch-plan-codex",
            "adapter-dispatch-execution-codex",
            true,
        );
    }

    fn append_adapter_dispatch_execution_named(
        state: &SqliteStateStore,
        project_id: &ProjectId,
        dispatch_plan_id: &str,
        dispatch_execution_id: &str,
        provider_cli_executed: bool,
    ) {
        state
            .append_event(
                NewEvent {
                    event_id: format!("event-{dispatch_execution_id}"),
                    kind: EventKind::AdapterDispatchExecuted,
                    actor: "test".to_string(),
                    project_id: Some(project_id.clone()),
                    task_id: None,
                    agent_id: Some(AgentId::new("agent-codex")),
                    session_id: Some(SessionId::new("session-codex")),
                    run_id: Some(RunId::new("run-codex")),
                    turn_id: None,
                    item_id: Some(dispatch_execution_id.to_string()),
                    payload_json: "{}".to_string(),
                    idempotency_key: None,
                    redaction_state: RedactionState::Safe,
                },
                &[ProjectionRecord::AdapterDispatchExecution(
                    AdapterDispatchExecutionProjection {
                        dispatch_execution_id: dispatch_execution_id.to_string(),
                        project_id: project_id.clone(),
                        dispatch_plan_id: dispatch_plan_id.to_string(),
                        execution_request_id: "adapter-dispatch-execution-request-codex"
                            .to_string(),
                        adapter_kind: "codex_exec".to_string(),
                        session_id: SessionId::new("session-codex"),
                        run_id: RunId::new("run-codex"),
                        provider_cli_execution_allowed: true,
                        provider_cli_executed,
                        status: if provider_cli_executed {
                            "completed".to_string()
                        } else {
                            "blocked_missing_opt_in".to_string()
                        },
                        exit_code: provider_cli_executed.then_some(0),
                        runtime_process_ref: provider_cli_executed
                            .then(|| "runtime-process-codex".to_string()),
                        stdout_artifact_id: provider_cli_executed
                            .then(|| "artifact-dispatch-stdout".to_string()),
                        stderr_artifact_id: provider_cli_executed
                            .then(|| "artifact-dispatch-stderr".to_string()),
                        artifact_root: "/tmp/capo-artifacts".to_string(),
                        credential_scan_status: if provider_cli_executed {
                            "clean".to_string()
                        } else {
                            "not_run".to_string()
                        },
                        raw_prompt_policy: "not_rendered".to_string(),
                        raw_output_policy: "artifacts_scanned_redacted".to_string(),
                        reason_codes: if provider_cli_executed {
                            "provider_cli_executed_with_clean_artifacts".to_string()
                        } else {
                            "explicit_provider_execution_opt_in_required".to_string()
                        },
                        updated_sequence: 0,
                    },
                )],
            )
            .expect("append adapter dispatch execution");
    }

    fn append_workpad_task(
        state: &SqliteStateStore,
        project_id: &ProjectId,
        workpad_task_id: &str,
        path: &str,
        observed_status: &str,
        capo_execution_status: &str,
    ) {
        state
            .append_event(
                NewEvent {
                    event_id: format!("event-{workpad_task_id}"),
                    kind: EventKind::WorkpadIndexed,
                    actor: "test".to_string(),
                    project_id: Some(project_id.clone()),
                    task_id: None,
                    agent_id: None,
                    session_id: None,
                    run_id: None,
                    turn_id: None,
                    item_id: Some(workpad_task_id.to_string()),
                    payload_json: "{}".to_string(),
                    idempotency_key: None,
                    redaction_state: RedactionState::Safe,
                },
                &[ProjectionRecord::WorkpadTask(WorkpadTaskProjection {
                    workpad_task_id: workpad_task_id.to_string(),
                    project_id: project_id.clone(),
                    path: path.to_string(),
                    source_anchor: "F2 - Workpad Dogfood Bridge".to_string(),
                    title: "Workpad Dogfood Bridge".to_string(),
                    observed_status: observed_status.to_string(),
                    capo_execution_status: capo_execution_status.to_string(),
                    observed_unix: 123,
                    updated_sequence: 0,
                })],
            )
            .expect("append workpad task");
    }

    fn temp_root(name: &str) -> std::path::PathBuf {
        let mut root = std::env::temp_dir();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        root.push(format!("capo-{name}-{nanos}"));
        root
    }
}
