use capo_state::WorkpadTaskProjection;

use crate::{
    ProjectDashboard, ProjectDogfoodReadiness, SourceTaskProjection, ToolActivitySummary,
    project_dogfood_readiness,
};

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

    pub fn source_tasks(&self) -> Vec<SourceTaskProjection> {
        self.workpad_tasks
            .iter()
            .map(SourceTaskProjection::from_workpad_task)
            .collect()
    }

    pub fn next_source_task(&self) -> Option<SourceTaskProjection> {
        self.next_workpad_task()
            .map(SourceTaskProjection::from_workpad_task)
    }

    pub fn next_source_task_candidate_count(&self) -> usize {
        self.next_workpad_candidate_count()
    }
}

pub(crate) fn actionable_workpad_status_rank(status: &str) -> Option<u8> {
    match status {
        "in_progress" => Some(0),
        "pending" => Some(1),
        "ready" => Some(2),
        "waiting_on_opt_in" => Some(3),
        _ => None,
    }
}
