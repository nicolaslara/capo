use capo_state::{AdapterDispatchPlanProjection, AdapterSmokeReportProjection};

use crate::{AdapterDispatchStatus, ProjectDashboard};

impl ProjectDashboard {
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
}
