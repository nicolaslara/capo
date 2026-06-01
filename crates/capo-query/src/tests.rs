use std::time::{SystemTime, UNIX_EPOCH};

use super::*;
use capo_core::{
    AgentId, EvidenceId, MemoryPacketId, ProjectId, RunId, SessionId, TaskId, ToolCallId,
};
use capo_state::{
    AdapterDispatchExecutionProjection, AdapterDispatchExecutionRequestProjection,
    AdapterDispatchGateProjection, AdapterDispatchPlanProjection,
    AdapterDispatchPromptMaterializationProjection, AdapterDispatchPromptSourceProjection,
    AdapterDispatchReplayProjection, AdapterSmokeReportProjection, AgentProjection,
    ConnectivityExposureProjection, EventKind, EvidenceProjection, MemoryPacketProjection,
    NewEvent, ProjectionRecord, RedactionState, ReviewFindingProjection, RunProjection,
    RuntimeTargetProjection, SessionProjection, SourceBindingProjection, SqliteStateStore,
    TaskOutcomeReportProjection, TaskProjection, ToolCallProjection, ToolObservationProjection,
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
                    external_session_ref: Some("adapter-session-demo".to_string()),
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
                    provenance: Default::default(),
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
    let latest = dashboard
        .latest_runtime_target(Some("remote-process"), Some("available"))
        .expect("latest available remote target");
    assert_eq!(latest.runtime_target_id, "remote-target-1");
    assert!(
        dashboard
            .runtime_target_status("missing-runtime-target")
            .is_none()
    );
    assert!(
        dashboard
            .latest_runtime_target(Some("container"), Some("available"))
            .is_none()
    );

    let blocked_readiness = dashboard
        .runtime_target_control_readiness("remote-target-1")
        .expect("blocked runtime target control readiness");
    assert!(!blocked_readiness.ready);
    assert!(blocked_readiness.target_ready);
    assert_eq!(blocked_readiness.control_exposure_status, "missing");
    assert_eq!(blocked_readiness.blockers, "control_exposure_missing");
    assert_eq!(
        blocked_readiness.next_action,
        "record_control_connectivity_exposure"
    );
    assert!(
        dashboard
            .runtime_target_control_readiness("missing-runtime-target")
            .is_none()
    );

    append_connectivity_exposure_with_reachability(
        &state,
        &ProjectId::new("project-capo"),
        "exposure-remote-control",
        "runtime_target",
        "remote-target-1",
        "control",
        "private",
        "network:connect:private_tunnel",
        "active",
        true,
    );
    let dashboard = project_dashboard(
        &state,
        ProjectDashboardQuery::new(ProjectId::new("project-capo")),
    )
    .expect("updated dashboard");
    let ready = dashboard
        .runtime_target_control_readiness("remote-target-1")
        .expect("ready runtime target control readiness");
    assert!(ready.ready);
    assert!(ready.control_exposure_ready);
    assert_eq!(ready.control_exposure_id, "exposure-remote-control");
    assert_eq!(ready.control_exposure_scope, "private");
    assert_eq!(ready.blockers, "none");
    assert_eq!(ready.next_action, "use_runtime_target_for_remote_control");
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
    let ready =
        project_dashboard(&state, ProjectDashboardQuery::new(project_id)).expect("ready dashboard");
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
fn project_dashboard_selects_adapter_smoke_report_status() {
    let root = temp_root("query-dashboard-smoke-report-status");
    let state = SqliteStateStore::open(&root).expect("state");
    let project_id = ProjectId::new("project-capo");
    append_adapter_smoke_report(
        &state,
        &project_id,
        "adapter-smoke-codex",
        "codex_exec",
        "skipped",
        "not_run",
        false,
    );

    let dashboard = project_dashboard(&state, ProjectDashboardQuery::new(project_id.clone()))
        .expect("dashboard");

    let report = dashboard
        .adapter_smoke_report_status("adapter-smoke-codex")
        .expect("adapter smoke report status");
    assert_eq!(report.adapter_kind, "codex_exec");
    assert_eq!(report.smoke_status, "skipped");
    assert_eq!(report.credential_scan_status, "not_run");
    assert!(
        dashboard
            .adapter_smoke_report_status("missing-smoke-report")
            .is_none()
    );
    let latest_any = dashboard
        .latest_adapter_smoke_report(None)
        .expect("latest smoke report");
    assert_eq!(latest_any.smoke_report_id, "adapter-smoke-codex");
    assert!(
        dashboard
            .latest_adapter_smoke_report(Some("claude_code"))
            .is_none()
    );

    append_adapter_smoke_report(
        &state,
        &project_id,
        "adapter-smoke-claude",
        "claude_code",
        "failed",
        "blocked",
        false,
    );
    let updated =
        project_dashboard(&state, ProjectDashboardQuery::new(project_id)).expect("dashboard");
    let latest_claude = updated
        .latest_adapter_smoke_report(Some("claude_code"))
        .expect("latest claude smoke report");
    assert_eq!(latest_claude.smoke_report_id, "adapter-smoke-claude");
    assert_eq!(latest_claude.credential_scan_status, "blocked");
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
            "available_runtime_target_missing",
            "project_memory_index_missing",
            "source_task_dispatch_chain_missing"
        ]
    );
    assert_eq!(
        blocked.compatibility_blockers,
        vec!["workpad_index_missing", "dispatch_chain_missing"]
    );
    assert_eq!(
        blocked.next_actions,
        vec![
            "record_clean_codex_smoke_evidence",
            "register_available_runtime_target",
            "run_project_memory_index",
            "record_or_replay_source_task_dispatch_plan"
        ]
    );
    assert_eq!(
        blocked.compatibility_next_actions,
        vec![
            "run_workpad_index",
            "record_or_replay_workpad_dispatch_plan"
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
    append_runtime_target(&state, &project_id, "runtime-target-local-1", "available");
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
    assert!(ready.runtime_target_ready);
    assert!(ready.project_memory_ready);
    assert!(ready.workpad_bridge_ready);
    assert!(ready.dispatch_chain_ready);
    assert_eq!(ready.runtime_target_count, 1);
    assert_eq!(ready.available_runtime_target_count, 1);
    assert_eq!(ready.source_task_count, 1);
    assert_eq!(ready.observed_source_task_count, 1);
    assert_eq!(ready.bound_source_task_count, 0);
    assert_eq!(ready.workpad_task_count, 1);
    assert_eq!(ready.observed_workpad_task_count, 1);
    assert_eq!(ready.dispatch_plan_count, 1);
    assert_eq!(ready.dispatch_replay_count, 1);
    assert_eq!(
        ready.connector_evidence_refs,
        vec!["adapter-smoke-codex-clean"]
    );
    assert_eq!(ready.runtime_target_refs, vec!["runtime-target-local-1"]);
    assert_eq!(
        ready.source_task_refs,
        vec!["workpads:features:tasks.md#f1"]
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
    assert!(ready.compatibility_blockers.is_empty());
    assert!(ready.compatibility_next_actions.is_empty());
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

    let source_tasks = dashboard.source_tasks();
    assert_eq!(source_tasks.len(), 1);
    assert_eq!(
        source_tasks[0].source_task_id,
        "workpads:features:tasks.md#f2"
    );
    assert_eq!(source_tasks[0].source_path, "workpads/features/tasks.md");
    assert_eq!(source_tasks[0].observed_source_status, "in_progress");
    assert_eq!(source_tasks[0].capo_binding_status, "observed_only");
    assert_eq!(
        source_tasks[0].compatibility_workpad_task_id,
        "workpads:features:tasks.md#f2"
    );
}

#[test]
fn project_dashboard_includes_source_bindings() {
    let root = temp_root("query-dashboard-source-bindings");
    let state = SqliteStateStore::open(&root).expect("state");
    let project_id = ProjectId::new("project-capo");
    let task_id = TaskId::new("task-workpad-workpads-features-tasks-md-f2");
    append_source_binding(
        &state,
        &project_id,
        &task_id,
        "workpads:features:tasks.md#f2",
        "workpads/features/tasks.md",
        "hash-f2",
    );

    let dashboard =
        project_dashboard(&state, ProjectDashboardQuery::new(project_id)).expect("dashboard");

    assert_eq!(dashboard.source_bindings.len(), 1);
    let binding = &dashboard.source_bindings[0];
    assert_eq!(binding.task_id, task_id);
    assert_eq!(binding.source_kind, "markdown");
    assert_eq!(binding.source_task_id, "workpads:features:tasks.md#f2");
    assert_eq!(binding.source_path, "workpads/features/tasks.md");
    assert_eq!(binding.source_hash, "hash-f2");
    assert_eq!(binding.binding_status, "active");
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

    assert_eq!(dashboard.next_source_task_candidate_count(), 2);
    let source_next = dashboard.next_source_task().expect("next source task");
    assert_eq!(source_next.source_task_id, "workpads:features:voice.md#v7");
    assert_eq!(
        source_next.compatibility_workpad_task_id,
        "workpads:features:voice.md#v7"
    );
    assert_eq!(source_next.source_path, "workpads/features/voice.md");
    assert_eq!(source_next.observed_source_status, "pending");
    assert_eq!(source_next.capo_binding_status, "observed_only");
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
                    external_session_ref: None,
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

fn append_task_outcome_report(state: &SqliteStateStore, project_id: &ProjectId, report_id: &str) {
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
    append_connectivity_exposure_with_reachability(
        state,
        project_id,
        exposure_id,
        owner_kind,
        owner_id,
        channel_kind,
        exposure_scope,
        permission_scope,
        status,
        false,
    );
}

#[allow(clippy::too_many_arguments)]
fn append_connectivity_exposure_with_reachability(
    state: &SqliteStateStore,
    project_id: &ProjectId,
    exposure_id: &str,
    owner_kind: &str,
    owner_id: &str,
    channel_kind: &str,
    exposure_scope: &str,
    permission_scope: &str,
    status: &str,
    reachable: bool,
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
                    health_status: if reachable { "healthy" } else { "unknown" }.to_string(),
                    reachable,
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
                    execution_request_id: "adapter-dispatch-execution-request-codex".to_string(),
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
                    materialization_id: "adapter-dispatch-prompt-materialization-codex".to_string(),
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
                    execution_request_id: "adapter-dispatch-execution-request-codex".to_string(),
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

fn append_source_binding(
    state: &SqliteStateStore,
    project_id: &ProjectId,
    task_id: &TaskId,
    source_task_id: &str,
    source_path: &str,
    source_hash: &str,
) {
    state
        .append_event(
            NewEvent {
                event_id: format!("event-source-binding-{task_id}"),
                kind: EventKind::WorkpadTaskImported,
                actor: "test".to_string(),
                project_id: Some(project_id.clone()),
                task_id: Some(task_id.clone()),
                agent_id: None,
                session_id: None,
                run_id: None,
                turn_id: None,
                item_id: Some(format!("source-binding-{task_id}")),
                payload_json: "{}".to_string(),
                idempotency_key: None,
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::SourceBinding(SourceBindingProjection {
                source_binding_id: format!("source-binding-{task_id}"),
                project_id: project_id.clone(),
                task_id: task_id.clone(),
                source_kind: "markdown".to_string(),
                source_task_id: source_task_id.to_string(),
                source_path: source_path.to_string(),
                source_anchor: "F2 - Workpad Dogfood Bridge".to_string(),
                source_hash: source_hash.to_string(),
                binding_status: "active".to_string(),
                updated_sequence: 0,
            })],
        )
        .expect("append source binding");
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
