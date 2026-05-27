use capo_core::SessionId;
use capo_state::{AgentProjection, SqliteStateStore, StateResult, ToolObservationProjection};

use crate::{
    AgentDashboardRow, ProjectDashboard, ProjectDashboardQuery, SessionDashboardRow,
    adapter_dogfood_gate,
};

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
    let source_bindings = state.source_bindings(&query.project_id)?;
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
        source_bindings,
        workpad_tasks,
    })
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
        tool_observations: tool_observations_for_session(state, session_id)?,
        memory_packets: state.memory_packets_for_session(session_id)?,
        review_findings: state.review_findings_for_session(session_id)?,
        task_outcome_reports: state.task_outcome_reports_for_session(session_id)?,
        recent_events: state.recent_events_for_session(session_id, recent_event_limit)?,
        session,
    })
}

fn tool_observations_for_session(
    state: &SqliteStateStore,
    session_id: &SessionId,
) -> StateResult<Vec<ToolObservationProjection>> {
    state.tool_observations_for_session(session_id)
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
