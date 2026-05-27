use capo_state::AdapterSmokeReportProjection;

use crate::{AdapterDogfoodGate, ProjectDashboard, ProjectDogfoodReadiness};

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
    let bound_source_task_count = dashboard
        .workpad_tasks
        .iter()
        .filter(|task| task.capo_execution_status != "observed_only")
        .count();
    let source_task_count = workpad_task_count;
    let observed_source_task_count = observed_workpad_task_count;
    let project_memory_ready = source_task_count > 0;
    let workpad_bridge_ready = project_memory_ready;
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
    let source_task_refs = workpad_task_refs.clone();
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
    let mut compatibility_blockers = Vec::new();
    let mut compatibility_next_actions = Vec::new();
    if !real_agent_connector_ready {
        blockers.push("real_agent_connector_not_proven".to_string());
        next_actions.push("record_clean_codex_smoke_evidence".to_string());
    }
    if !runtime_target_ready {
        blockers.push("available_runtime_target_missing".to_string());
        next_actions.push("register_available_runtime_target".to_string());
    }
    if !workpad_bridge_ready {
        blockers.push("project_memory_index_missing".to_string());
        next_actions.push("run_project_memory_index".to_string());
        compatibility_blockers.push("workpad_index_missing".to_string());
        compatibility_next_actions.push("run_workpad_index".to_string());
    }
    if !dispatch_chain_ready {
        blockers.push("source_task_dispatch_chain_missing".to_string());
        next_actions.push("record_or_replay_source_task_dispatch_plan".to_string());
        compatibility_blockers.push("dispatch_chain_missing".to_string());
        compatibility_next_actions.push("record_or_replay_workpad_dispatch_plan".to_string());
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
        project_memory_ready,
        workpad_bridge_ready,
        dispatch_chain_ready,
        runtime_target_count,
        available_runtime_target_count,
        source_task_count,
        observed_source_task_count,
        bound_source_task_count,
        workpad_task_count,
        observed_workpad_task_count,
        imported_workpad_task_count,
        dispatch_plan_count,
        ready_dispatch_gate_count,
        dispatch_replay_count,
        dispatch_execution_count,
        connector_evidence_refs,
        runtime_target_refs,
        source_task_refs,
        workpad_task_refs,
        dispatch_chain_refs,
        project_evidence_refs,
        blockers,
        next_actions,
        compatibility_blockers,
        compatibility_next_actions,
    }
}
