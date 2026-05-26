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

    pub fn adapter_smoke_report_status(
        &self,
        smoke_report_id: &str,
    ) -> Option<&AdapterSmokeReportProjection> {
        self.adapter_smoke_reports
            .iter()
            .rev()
            .find(|report| report.smoke_report_id == smoke_report_id)
    }

    pub fn latest_adapter_smoke_report(
        &self,
        adapter_kind: Option<&str>,
    ) -> Option<&AdapterSmokeReportProjection> {
        self.adapter_smoke_reports
            .iter()
            .filter(|report| {
                adapter_kind
                    .map(|kind| report.adapter_kind == kind)
                    .unwrap_or(true)
            })
            .max_by(|left, right| {
                left.updated_sequence
                    .cmp(&right.updated_sequence)
                    .then_with(|| left.smoke_report_id.cmp(&right.smoke_report_id))
            })
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

    pub fn latest_runtime_target(
        &self,
        runner_kind: Option<&str>,
        status: Option<&str>,
    ) -> Option<&RuntimeTargetProjection> {
        self.runtime_targets
            .iter()
            .filter(|target| {
                runner_kind
                    .map(|kind| runtime_runner_kind_matches(&target.runner_kind, kind))
                    .unwrap_or(true)
            })
            .filter(|target| status.map(|value| target.status == value).unwrap_or(true))
            .max_by(|left, right| {
                left.updated_sequence
                    .cmp(&right.updated_sequence)
                    .then_with(|| left.runtime_target_id.cmp(&right.runtime_target_id))
            })
    }

    pub fn runtime_target_control_readiness(
        &self,
        runtime_target_id: &str,
    ) -> Option<RuntimeTargetControlReadiness> {
        let target = self.runtime_target_status(runtime_target_id)?;
        let latest_control_exposure = self.latest_connectivity_exposure(
            Some("runtime_target"),
            Some(runtime_target_id),
            Some("control"),
        );
        let exposure_ready = latest_control_exposure
            .map(|exposure| exposure.status == "active" && exposure.reachable)
            .unwrap_or(false);
        let target_ready = target.status == "available";
        let ready = target_ready && exposure_ready;
        let mut blockers = Vec::new();
        if !target_ready {
            blockers.push(format!("runtime_target_status_{}", target.status));
        }
        match latest_control_exposure {
            Some(exposure) if exposure.status != "active" => {
                blockers.push(format!("control_exposure_status_{}", exposure.status));
            }
            Some(exposure) if !exposure.reachable => {
                blockers.push("control_exposure_unreachable".to_string());
            }
            Some(_) => {}
            None => blockers.push("control_exposure_missing".to_string()),
        }
        let next_action = if ready {
            "use_runtime_target_for_remote_control"
        } else if !target_ready {
            "set_runtime_target_available"
        } else if latest_control_exposure.is_none() {
            "record_control_connectivity_exposure"
        } else if latest_control_exposure
            .map(|exposure| exposure.status == "blocked_pending_permission")
            .unwrap_or(false)
        {
            "request_or_grant_control_exposure_permission"
        } else {
            "repair_or_replace_control_exposure"
        };

        Some(RuntimeTargetControlReadiness {
            runtime_target_id: target.runtime_target_id.clone(),
            runner_kind: target.runner_kind.clone(),
            target_status: target.status.clone(),
            target_ready,
            control_exposure_ready: exposure_ready,
            control_exposure_id: latest_control_exposure
                .map(|exposure| exposure.exposure_id.clone())
                .unwrap_or_else(|| "none".to_string()),
            control_exposure_status: latest_control_exposure
                .map(|exposure| exposure.status.clone())
                .unwrap_or_else(|| "missing".to_string()),
            control_exposure_scope: latest_control_exposure
                .map(|exposure| exposure.exposure.clone())
                .unwrap_or_else(|| "none".to_string()),
            control_exposure_permission_scope: latest_control_exposure
                .map(|exposure| exposure.permission_scope.clone())
                .unwrap_or_else(|| "none".to_string()),
            control_exposure_reachable: latest_control_exposure
                .map(|exposure| exposure.reachable)
                .unwrap_or(false),
            ready,
            blockers: if blockers.is_empty() {
                "none".to_string()
            } else {
                blockers.join(",")
            },
            next_action: next_action.to_string(),
        })
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

fn runtime_runner_kind_matches(stored: &str, requested: &str) -> bool {
    stored == requested
        || stored.replace('_', "-") == requested
        || stored.replace('-', "_") == requested
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentDashboardRow {
    pub agent: AgentProjection,
    pub session: Option<SessionDashboardRow>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeTargetControlReadiness {
    pub runtime_target_id: String,
    pub runner_kind: String,
    pub target_status: String,
    pub target_ready: bool,
    pub control_exposure_ready: bool,
    pub control_exposure_id: String,
    pub control_exposure_status: String,
    pub control_exposure_scope: String,
    pub control_exposure_permission_scope: String,
    pub control_exposure_reachable: bool,
    pub ready: bool,
    pub blockers: String,
    pub next_action: String,
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
    pub runtime_target_ready: bool,
    pub workpad_bridge_ready: bool,
    pub dispatch_chain_ready: bool,
    pub runtime_target_count: usize,
    pub available_runtime_target_count: usize,
    pub workpad_task_count: usize,
    pub observed_workpad_task_count: usize,
    pub imported_workpad_task_count: usize,
    pub dispatch_plan_count: usize,
    pub ready_dispatch_gate_count: usize,
    pub dispatch_replay_count: usize,
    pub dispatch_execution_count: usize,
    pub connector_evidence_refs: Vec<String>,
    pub runtime_target_refs: Vec<String>,
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
    let runtime_target_count = dashboard.runtime_targets.len();
    let available_runtime_target_count = dashboard
        .runtime_targets
        .iter()
        .filter(|target| target.status == "available")
        .count();
    let runtime_target_ready = available_runtime_target_count > 0;
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
    let runtime_target_refs = dashboard
        .runtime_targets
        .iter()
        .map(|target| target.runtime_target_id.clone())
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
    if !runtime_target_ready {
        blockers.push("available_runtime_target_missing".to_string());
        next_actions.push("register_available_runtime_target".to_string());
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
        runtime_target_ready,
        workpad_bridge_ready,
        dispatch_chain_ready,
        runtime_target_count,
        available_runtime_target_count,
        workpad_task_count,
        observed_workpad_task_count,
        imported_workpad_task_count,
        dispatch_plan_count,
        ready_dispatch_gate_count,
        dispatch_replay_count,
        dispatch_execution_count,
        connector_evidence_refs,
        runtime_target_refs,
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
mod tests;
