use capo_query::{ProjectDashboardQuery, project_dashboard};

use crate::util::{adapter_kind_for_events, turn_ids_for_events};
use crate::{AgentSummary, CapoServer, ServerDashboardSnapshot, ServerResult, SessionSummary};

impl CapoServer {
    pub fn dashboard_snapshot(&self) -> ServerResult<ServerDashboardSnapshot> {
        self.dashboard_with_limit(5)
    }

    pub(crate) fn dashboard_with_limit(
        &self,
        recent_event_limit: usize,
    ) -> ServerResult<ServerDashboardSnapshot> {
        let mut query = ProjectDashboardQuery::new(self.project_id.clone());
        query.recent_event_limit = recent_event_limit;
        let dashboard =
            project_dashboard(self.controller.state(), query).map_err(crate::ServerError::State)?;
        let dispatch_plans = dashboard.adapter_dispatch_plans;
        let dispatch_gates = dashboard.adapter_dispatch_gates;
        let dispatch_executions = dashboard.adapter_dispatch_executions;
        let agents = dashboard
            .agents
            .into_iter()
            .map(|row| {
                let session = row.session.map(|session| {
                    let turn_ids = turn_ids_for_events(&session.recent_events);
                    let latest_plan = dispatch_plans
                        .iter()
                        .rev()
                        .find(|plan| plan.session_id == session.session.session_id);
                    let latest_gate = latest_plan.and_then(|plan| {
                        dispatch_gates
                            .iter()
                            .rev()
                            .find(|gate| gate.dispatch_plan_id == plan.dispatch_plan_id)
                    });
                    let latest_execution = latest_plan.and_then(|plan| {
                        dispatch_executions
                            .iter()
                            .rev()
                            .find(|execution| execution.dispatch_plan_id == plan.dispatch_plan_id)
                    });
                    SessionSummary {
                        session_id: session.session.session_id,
                        status: session.session.status,
                        run_id: session.run.as_ref().map(|run| run.run_id.clone()),
                        run_status: session.run.map(|run| run.status),
                        adapter_kind: adapter_kind_for_events(&session.recent_events)
                            .or_else(|| latest_plan.map(|plan| plan.adapter_kind.clone())),
                        recent_event_count: session.recent_events.len(),
                        evidence_count: session.evidence.len(),
                        evidence_refs: session
                            .evidence
                            .iter()
                            .map(|evidence| evidence.evidence_id.to_string())
                            .collect(),
                        turn_count: turn_ids.len(),
                        turn_ids,
                        latest_dispatch_plan_id: latest_plan
                            .map(|plan| plan.dispatch_plan_id.clone()),
                        latest_dispatch_gate_id: latest_gate
                            .map(|gate| gate.dispatch_gate_id.clone()),
                        latest_dispatch_execution_id: latest_execution
                            .map(|execution| execution.dispatch_execution_id.clone()),
                        dispatch_gate_status: latest_gate.map(|gate| gate.status.clone()),
                        dispatch_gate_reasons: latest_gate.map(|gate| gate.reason_codes.clone()),
                        dispatch_next_action: latest_gate.map(|gate| {
                            if gate.provider_cli_execution_allowed {
                                "ready_for_explicit_live_provider_run".to_string()
                            } else {
                                "fix_preflight_blockers".to_string()
                            }
                        }),
                        dispatch_execution_status: latest_execution
                            .map(|execution| execution.status.clone()),
                        dispatch_runtime_process_ref: latest_execution
                            .and_then(|execution| execution.runtime_process_ref.clone()),
                        dispatch_provider_cli_execution_allowed: latest_execution
                            .map(|execution| execution.provider_cli_execution_allowed),
                        dispatch_provider_cli_executed: latest_execution
                            .map(|execution| execution.provider_cli_executed),
                        dispatch_credential_scan_status: latest_execution
                            .map(|execution| execution.credential_scan_status.clone()),
                        dispatch_raw_prompt_policy: latest_execution
                            .map(|execution| execution.raw_prompt_policy.clone()),
                        dispatch_raw_output_policy: latest_execution
                            .map(|execution| execution.raw_output_policy.clone()),
                        tool_call_count: session.tool_calls.len(),
                        tool_observation_count: session.tool_observations.len(),
                        memory_packet_count: session.memory_packets.len(),
                    }
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

    pub(crate) fn agent_by_name(&self, agent_name: &str) -> ServerResult<Option<AgentSummary>> {
        Ok(self
            .dashboard_snapshot()?
            .agents
            .into_iter()
            .find(|agent| agent.name == agent_name))
    }
}
