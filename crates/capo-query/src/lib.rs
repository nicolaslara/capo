//! Reusable read-model queries for Capo operator surfaces.
//!
//! This crate owns aggregation over state projections. CLI, dashboards, voice,
//! mobile, and web surfaces should render these structs instead of stitching
//! SQLite read models together independently.

use capo_core::{ProjectId, SessionId};
use capo_state::{
    AdapterDispatchPlanProjection, AdapterReadinessProjection, AdapterSmokeReportProjection,
    AgentProjection, ConnectivityExposureProjection, EventRecord, EvidenceProjection,
    MemoryPacketProjection, RunProjection, SessionProjection, SqliteStateStore, StateResult,
    ToolCallProjection, WorkpadTaskProjection,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectDashboard {
    pub project_id: ProjectId,
    pub agents: Vec<AgentDashboardRow>,
    pub connectivity_exposures: Vec<ConnectivityExposureProjection>,
    pub adapter_readiness: Vec<AdapterReadinessProjection>,
    pub adapter_smoke_reports: Vec<AdapterSmokeReportProjection>,
    pub adapter_dispatch_plans: Vec<AdapterDispatchPlanProjection>,
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
    pub memory_packets: Vec<MemoryPacketProjection>,
    pub recent_events: Vec<EventRecord>,
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
    let adapter_readiness = state.adapter_readiness(&query.project_id)?;
    let adapter_smoke_reports = state.adapter_smoke_reports(&query.project_id)?;
    let adapter_dispatch_plans = state.adapter_dispatch_plans(&query.project_id)?;
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
        connectivity_exposures,
        adapter_readiness,
        adapter_smoke_reports,
        adapter_dispatch_plans,
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
        memory_packets: state.memory_packets_for_session(session_id)?,
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
        WorkpadTaskProjection,
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
            session.memory_packets[0].memory_packet_id,
            MemoryPacketId::new("packet-demo")
        );
        assert_eq!(session.recent_events[0].kind, "session.started");
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
    fn project_dashboard_includes_adapter_dispatch_plans() {
        let root = temp_root("query-dashboard-adapter-dispatch");
        let state = SqliteStateStore::open(&root).expect("state");
        let project_id = ProjectId::new("project-capo");
        append_agent(&state, &project_id, "agent-idle", None);
        append_adapter_dispatch_plan(&state, &project_id);

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

    fn append_connectivity_exposure(state: &SqliteStateStore, project_id: &ProjectId) {
        state
            .append_event(
                NewEvent {
                    event_id: "event-connectivity-exposure".to_string(),
                    kind: EventKind::ConnectivityExposureRequested,
                    actor: "test".to_string(),
                    project_id: Some(project_id.clone()),
                    task_id: None,
                    agent_id: None,
                    session_id: None,
                    run_id: None,
                    turn_id: None,
                    item_id: Some("exposure-private-control".to_string()),
                    payload_json: "{}".to_string(),
                    idempotency_key: None,
                    redaction_state: RedactionState::Safe,
                },
                &[ProjectionRecord::ConnectivityExposure(
                    ConnectivityExposureProjection {
                        exposure_id: "exposure-private-control".to_string(),
                        project_id: project_id.clone(),
                        connectivity_endpoint_id: "endpoint-private-1".to_string(),
                        owner_kind: "runtime_target".to_string(),
                        owner_id: "remote-target-1".to_string(),
                        channel_kind: "control".to_string(),
                        exposure: "private".to_string(),
                        permission_scope: "network:connect:private_tunnel".to_string(),
                        status: "blocked_pending_permission".to_string(),
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
        state
            .append_event(
                NewEvent {
                    event_id: "event-adapter-dispatch-plan-codex".to_string(),
                    kind: EventKind::AdapterDispatchPlanned,
                    actor: "test".to_string(),
                    project_id: Some(project_id.clone()),
                    task_id: None,
                    agent_id: Some(AgentId::new("agent-codex")),
                    session_id: Some(SessionId::new("session-codex")),
                    run_id: Some(RunId::new("run-codex")),
                    turn_id: None,
                    item_id: Some("adapter-dispatch-plan-codex".to_string()),
                    payload_json: "{}".to_string(),
                    idempotency_key: None,
                    redaction_state: RedactionState::Safe,
                },
                &[ProjectionRecord::AdapterDispatchPlan(
                    AdapterDispatchPlanProjection {
                        dispatch_plan_id: "adapter-dispatch-plan-codex".to_string(),
                        project_id: project_id.clone(),
                        adapter_kind: "codex_exec".to_string(),
                        provider_kind: "codex_subscription".to_string(),
                        credential_scope: "user_local_subscription".to_string(),
                        agent_id: AgentId::new("agent-codex"),
                        agent_name: "codex".to_string(),
                        session_id: SessionId::new("session-codex"),
                        run_id: RunId::new("run-codex"),
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
