use super::*;

#[test]
fn server_live_provider_preflight_gates_codex_and_claude_without_execution() {
    let root = temp_root();
    let server = CapoServer::open(ProjectId::new("project-capo"), &root).expect("server");
    for (agent, adapter, session, run) in [
        (
            "codex-local",
            "codex",
            "session-codex-live-preflight",
            "run-codex-live-preflight",
        ),
        (
            "claude-local",
            "claude",
            "session-claude-live-preflight",
            "run-claude-live-preflight",
        ),
    ] {
        handle(
            &server,
            ServerCommand::RegisterAgent {
                name: agent.to_string(),
                adapter: "fake".to_string(),
            },
        );
        handle(
            &server,
            ServerCommand::StartSession {
                agent_name: agent.to_string(),
                goal: format!("Preflight {adapter} live provider"),
                adapter: adapter.to_string(),
                session_id: Some(session.to_string()),
                run_id: Some(run.to_string()),
            },
        );
        let response = handle(
            &server,
            ServerCommand::PreflightLiveProvider {
                agent_name: agent.to_string(),
                adapter: adapter.to_string(),
                goal: format!("Preflight {adapter} live provider"),
                workspace: "/tmp/capo-workspace".to_string(),
                artifacts: "/tmp/capo-artifacts".to_string(),
                session_id: session.to_string(),
                run_id: run.to_string(),
                turn_id: format!("turn-{adapter}-live-preflight"),
                capability_profile: "trusted-local".to_string(),
                runtime_scope: "local_process_loopback".to_string(),
                credential_scan_policy: "metadata_only_no_secret_read".to_string(),
                raw_prompt_policy: "not_rendered".to_string(),
                raw_output_policy: "artifacts_scanned_redacted".to_string(),
                tool_wrapper_policy: "capo_wrapped_required".to_string(),
                live_provider_opt_in: true,
            },
        );
        let ServerResponsePayload::LiveProviderPreflighted(preflight) = response.payload else {
            panic!("expected live provider preflight response");
        };
        assert!(preflight.provider_cli_execution_allowed);
        assert!(!preflight.provider_cli_executed);
        assert_eq!(preflight.status, "ready_for_live_provider_execution");
        assert_eq!(preflight.reasons, "live_provider_preflight_ready");
        assert_eq!(
            preflight.next_action,
            "run_explicit_live_provider_execution"
        );
    }

    let dashboard = server.dashboard_snapshot().expect("dashboard");
    let codex = dashboard
        .agents
        .iter()
        .find(|agent| agent.name == "codex-local")
        .and_then(|agent| agent.session.as_ref())
        .expect("codex session");
    assert_eq!(
        codex.dispatch_gate_status.as_deref(),
        Some("ready_for_live_provider_execution")
    );
    assert_eq!(
        codex.dispatch_next_action.as_deref(),
        Some("ready_for_explicit_live_provider_run")
    );
    assert_eq!(codex.dispatch_provider_cli_execution_allowed, None);
    assert_eq!(codex.dispatch_provider_cli_executed, None);
}

#[test]
fn server_live_provider_preflight_fails_closed_without_opt_in_or_policies() {
    let root = temp_root();
    let server = CapoServer::open(ProjectId::new("project-capo"), &root).expect("server");
    handle(
        &server,
        ServerCommand::RegisterAgent {
            name: "codex-local".to_string(),
            adapter: "fake".to_string(),
        },
    );
    handle(
        &server,
        ServerCommand::StartSession {
            agent_name: "codex-local".to_string(),
            goal: "Blocked live provider preflight".to_string(),
            adapter: "codex".to_string(),
            session_id: Some("session-codex-live-blocked".to_string()),
            run_id: Some("run-codex-live-blocked".to_string()),
        },
    );

    let response = handle(
        &server,
        ServerCommand::PreflightLiveProvider {
            agent_name: "codex-local".to_string(),
            adapter: "codex".to_string(),
            goal: "Blocked live provider preflight".to_string(),
            workspace: "../outside".to_string(),
            artifacts: "".to_string(),
            session_id: "session-codex-live-blocked".to_string(),
            run_id: "run-codex-live-blocked".to_string(),
            turn_id: "turn-codex-live-blocked".to_string(),
            capability_profile: "read-only".to_string(),
            runtime_scope: "public_network".to_string(),
            credential_scan_policy: "read_provider_session".to_string(),
            raw_prompt_policy: "rendered".to_string(),
            raw_output_policy: "unscanned".to_string(),
            tool_wrapper_policy: "native_tools_unwrapped".to_string(),
            live_provider_opt_in: false,
        },
    );
    let ServerResponsePayload::LiveProviderPreflighted(preflight) = response.payload else {
        panic!("expected live provider preflight response");
    };
    assert!(!preflight.provider_cli_execution_allowed);
    assert!(!preflight.provider_cli_executed);
    assert_eq!(preflight.status, "blocked_by_live_provider_preflight");
    assert_eq!(preflight.credential_scan_policy, "rejected");
    assert_eq!(preflight.raw_prompt_policy, "rejected");
    assert_eq!(preflight.raw_output_policy, "rejected");
    assert_eq!(preflight.tool_wrapper_policy, "rejected");
    for reason in [
        "missing_live_provider_preflight_opt_in",
        "unsafe_runtime_scope",
        "unsafe_workspace_scope",
        "missing_artifact_root_policy",
        "missing_live_capability_profile",
        "credential_handling_policy_not_explicit",
        "raw_prompt_policy_not_redacted",
        "raw_output_policy_missing_artifact_scan",
        "tool_wrapper_instrumentation_missing",
    ] {
        assert!(preflight.reasons.contains(reason), "missing {reason}");
    }
    let state = SqliteStateStore::open(&root).expect("state");
    let events = state
        .recent_events_for_session(&preflight.session_id, 20)
        .expect("session events");
    assert!(events.iter().any(|event| {
        event.kind == "adapter.dispatch_gate_checked"
            && event
                .payload_json
                .contains("\"preflight_kind\":\"live_provider\"")
            && event
                .payload_json
                .contains("\"credential_material_rendered\":false")
            && event
                .payload_json
                .contains("\"credential_scan_policy\":\"rejected\"")
            && !event.payload_json.contains("read_provider_session")
    }));
}

#[test]
fn server_live_provider_preflight_changed_policy_does_not_leave_stale_ready_gate() {
    let root = temp_root();
    let server = CapoServer::open(ProjectId::new("project-capo"), &root).expect("server");
    handle(
        &server,
        ServerCommand::RegisterAgent {
            name: "codex-local".to_string(),
            adapter: "fake".to_string(),
        },
    );
    handle(
        &server,
        ServerCommand::StartSession {
            agent_name: "codex-local".to_string(),
            goal: "Changed preflight policy".to_string(),
            adapter: "codex".to_string(),
            session_id: Some("session-codex-live-repeat".to_string()),
            run_id: Some("run-codex-live-repeat".to_string()),
        },
    );

    let ready = handle(
        &server,
        ServerCommand::PreflightLiveProvider {
            agent_name: "codex-local".to_string(),
            adapter: "codex".to_string(),
            goal: "Changed preflight policy".to_string(),
            workspace: "/tmp/capo-workspace".to_string(),
            artifacts: "/tmp/capo-artifacts".to_string(),
            session_id: "session-codex-live-repeat".to_string(),
            run_id: "run-codex-live-repeat".to_string(),
            turn_id: "turn-codex-live-repeat".to_string(),
            capability_profile: "trusted-local".to_string(),
            runtime_scope: "local_process_loopback".to_string(),
            credential_scan_policy: "metadata_only_no_secret_read".to_string(),
            raw_prompt_policy: "not_rendered".to_string(),
            raw_output_policy: "artifacts_scanned_redacted".to_string(),
            tool_wrapper_policy: "capo_wrapped_required".to_string(),
            live_provider_opt_in: true,
        },
    );
    let ServerResponsePayload::LiveProviderPreflighted(ready) = ready.payload else {
        panic!("expected ready preflight");
    };
    assert!(ready.provider_cli_execution_allowed);

    let blocked = handle(
        &server,
        ServerCommand::PreflightLiveProvider {
            agent_name: "codex-local".to_string(),
            adapter: "codex".to_string(),
            goal: "Changed preflight policy".to_string(),
            workspace: "/tmp/capo-workspace".to_string(),
            artifacts: "/tmp/capo-artifacts".to_string(),
            session_id: "session-codex-live-repeat".to_string(),
            run_id: "run-codex-live-repeat".to_string(),
            turn_id: "turn-codex-live-repeat".to_string(),
            capability_profile: "trusted-local".to_string(),
            runtime_scope: "local_process_loopback".to_string(),
            credential_scan_policy: "metadata_only_no_secret_read".to_string(),
            raw_prompt_policy: "not_rendered".to_string(),
            raw_output_policy: "unscanned".to_string(),
            tool_wrapper_policy: "capo_wrapped_required".to_string(),
            live_provider_opt_in: true,
        },
    );
    let ServerResponsePayload::LiveProviderPreflighted(blocked) = blocked.payload else {
        panic!("expected blocked preflight");
    };
    assert!(!blocked.provider_cli_execution_allowed);
    assert_ne!(ready.dispatch_gate_id, blocked.dispatch_gate_id);

    let dashboard = server.dashboard_snapshot().expect("dashboard");
    let session = dashboard.agents[0].session.as_ref().expect("session");
    assert_eq!(
        session.dispatch_gate_status.as_deref(),
        Some("blocked_by_live_provider_preflight")
    );
    assert_eq!(
        session.dispatch_gate_reasons.as_deref(),
        Some("raw_output_policy_missing_artifact_scan")
    );
    assert_eq!(
        session.dispatch_next_action.as_deref(),
        Some("fix_preflight_blockers")
    );
}

#[test]
fn server_live_provider_preflight_default_request_id_does_not_slug_raw_goal() {
    let request = ServerRequest::cli(ServerCommand::PreflightLiveProvider {
        agent_name: "codex-local".to_string(),
        adapter: "codex".to_string(),
        goal: "Secret prompt details should not appear in request ids".to_string(),
        workspace: "/tmp/capo-workspace".to_string(),
        artifacts: "/tmp/capo-artifacts".to_string(),
        session_id: "session-codex-live-request-id".to_string(),
        run_id: "run-codex-live-request-id".to_string(),
        turn_id: "turn-codex-live-request-id".to_string(),
        capability_profile: "trusted-local".to_string(),
        runtime_scope: "local_process_loopback".to_string(),
        credential_scan_policy: "metadata_only_no_secret_read".to_string(),
        raw_prompt_policy: "not_rendered".to_string(),
        raw_output_policy: "artifacts_scanned_redacted".to_string(),
        tool_wrapper_policy: "capo_wrapped_required".to_string(),
        live_provider_opt_in: true,
    });
    assert!(!request.request_id.contains("secret"));
    assert!(!request.request_id.contains("prompt"));
    assert!(request.request_id.contains("session-codex-live-request-id"));
    assert!(request.request_id.contains("turn-codex-live-request-id"));
}

#[test]
fn server_live_provider_local_run_requires_ready_codex_preflight_and_mock_opt_in() {
    let root = temp_root();
    let server = CapoServer::open(ProjectId::new("project-capo"), &root).expect("server");
    let goal = "Run Codex live provider through server";
    handle(
        &server,
        ServerCommand::RegisterAgent {
            name: "codex-local".to_string(),
            adapter: "fake".to_string(),
        },
    );
    handle(
        &server,
        ServerCommand::StartSession {
            agent_name: "codex-local".to_string(),
            goal: goal.to_string(),
            adapter: "codex".to_string(),
            session_id: Some("session-codex-live-run".to_string()),
            run_id: Some("run-codex-live-run".to_string()),
        },
    );
    let preflight = handle(
        &server,
        ServerCommand::PreflightLiveProvider {
            agent_name: "codex-local".to_string(),
            adapter: "codex".to_string(),
            goal: goal.to_string(),
            workspace: "/tmp/capo-workspace".to_string(),
            artifacts: "/tmp/capo-artifacts".to_string(),
            session_id: "session-codex-live-run".to_string(),
            run_id: "run-codex-live-run".to_string(),
            turn_id: "turn-codex-live-run".to_string(),
            capability_profile: "trusted-local".to_string(),
            runtime_scope: "local_process_loopback".to_string(),
            credential_scan_policy: "metadata_only_no_secret_read".to_string(),
            raw_prompt_policy: "not_rendered".to_string(),
            raw_output_policy: "artifacts_scanned_redacted".to_string(),
            tool_wrapper_policy: "capo_wrapped_required".to_string(),
            live_provider_opt_in: true,
        },
    );
    let ServerResponsePayload::LiveProviderPreflighted(preflight) = preflight.payload else {
        panic!("expected live provider preflight response");
    };

    let blocked = handle(
        &server,
        ServerCommand::RunLiveProviderLocal {
            dispatch_plan_id: preflight.dispatch_plan_id.clone(),
            goal: goal.to_string(),
            live_execution_opt_in: false,
            mock_runtime_opt_in: false,
            mock_provider_output_name: None,
            mock_provider_output_jsonl: None,
            timeout_seconds: 1,
            codex_program_override: None,
            unattended: true,
        },
    );
    let ServerResponsePayload::DispatchRun(blocked) = blocked.payload else {
        panic!("expected blocked live run response");
    };
    assert!(!blocked.provider_cli_executed);
    assert_eq!(blocked.status, "blocked_by_live_provider_execution_gate");
    assert!(
        blocked
            .reason_codes
            .contains("missing_live_provider_execution_opt_in")
    );

    let fixture = include_str!("../../../capo-adapters/fixtures/codex-exec.jsonl");
    let stale_prompt = handle(
        &server,
        ServerCommand::RunLiveProviderLocal {
            dispatch_plan_id: preflight.dispatch_plan_id.clone(),
            goal: "Changed live run prompt".to_string(),
            live_execution_opt_in: false,
            mock_runtime_opt_in: true,
            mock_provider_output_name: Some("codex-exec.jsonl".to_string()),
            mock_provider_output_jsonl: Some(fixture.to_string()),
            timeout_seconds: 1,
            codex_program_override: None,
            unattended: true,
        },
    );
    let ServerResponsePayload::DispatchRun(stale_prompt) = stale_prompt.payload else {
        panic!("expected stale prompt live run response");
    };
    assert!(!stale_prompt.provider_cli_executed);
    assert_eq!(
        stale_prompt.status,
        "blocked_by_live_provider_execution_gate"
    );
    assert!(stale_prompt.reason_codes.contains("prompt_hash_mismatch"));

    let run = handle(
        &server,
        ServerCommand::RunLiveProviderLocal {
            dispatch_plan_id: preflight.dispatch_plan_id,
            goal: goal.to_string(),
            live_execution_opt_in: false,
            mock_runtime_opt_in: true,
            mock_provider_output_name: Some("codex-exec.jsonl".to_string()),
            mock_provider_output_jsonl: Some(fixture.to_string()),
            timeout_seconds: 1,
            codex_program_override: None,
            unattended: true,
        },
    );
    let ServerResponsePayload::DispatchRun(run) = run.payload else {
        panic!("expected live run response");
    };
    assert!(!run.provider_cli_executed);
    assert_eq!(run.status, "mocked_live_provider_output_ingested");
    assert_eq!(run.credential_scan_status, "not_applicable_mock");
    assert_eq!(run.raw_output_policy, "content_hashed_not_rendered");
    assert_eq!(run.input_event_count, 5);
    assert_eq!(run.tool_event_count, 2);
    let state = SqliteStateStore::open(&root).expect("state");
    let events_after_run = state
        .recent_events_for_session(&SessionId::new("session-codex-live-run"), 40)
        .expect("events after mocked live run");
    assert!(events_after_run.iter().any(|event| {
        event.kind == "run.exited"
            && event
                .payload_json
                .contains("\"provider_cli_executed\":false")
            && event
                .payload_json
                .contains("mock_live_provider_output_ingested_without_provider_cli")
    }));
    assert!(events_after_run.iter().any(|event| {
        event.kind == "adapter.dispatch_replayed"
            && event
                .payload_json
                .contains("\"provider_cli_executed\":false")
            && event
                .payload_json
                .contains("\"raw_content_policy\":\"content_hashed_not_rendered\"")
    }));
    let event_count_before_repeat = state.event_count().expect("event count before repeat");
    let repeat = handle(
        &server,
        ServerCommand::RunLiveProviderLocal {
            dispatch_plan_id: run.dispatch_plan_id.clone(),
            goal: goal.to_string(),
            live_execution_opt_in: false,
            mock_runtime_opt_in: true,
            mock_provider_output_name: Some("codex-exec.jsonl".to_string()),
            mock_provider_output_jsonl: Some(fixture.to_string()),
            timeout_seconds: 1,
            codex_program_override: None,
            unattended: true,
        },
    );
    let ServerResponsePayload::DispatchRun(repeat) = repeat.payload else {
        panic!("expected repeated live run response");
    };
    assert_eq!(repeat.dispatch_execution_id, run.dispatch_execution_id);
    assert_eq!(
        state.event_count().expect("event count after repeat"),
        event_count_before_repeat,
        "repeating the same mocked live-run must not duplicate stream, replay, execution, or audit events"
    );

    let dashboard = server.dashboard_snapshot().expect("dashboard");
    let session = dashboard.agents[0].session.as_ref().expect("session");
    assert_eq!(
        session.dispatch_execution_status.as_deref(),
        Some("mocked_live_provider_output_ingested")
    );
    assert_eq!(session.dispatch_provider_cli_executed, Some(false));
    assert_eq!(session.run_status.as_deref(), Some("exited"));
    assert_eq!(session.tool_call_count, 1);
}

#[test]
fn server_live_provider_local_run_rechecks_prompt_after_existing_real_execution() {
    let root = temp_root();
    let server = CapoServer::open(ProjectId::new("project-capo"), &root).expect("server");
    let goal = "Existing real execution should not bypass prompt hash";
    handle(
        &server,
        ServerCommand::RegisterAgent {
            name: "codex-local".to_string(),
            adapter: "fake".to_string(),
        },
    );
    handle(
        &server,
        ServerCommand::StartSession {
            agent_name: "codex-local".to_string(),
            goal: goal.to_string(),
            adapter: "codex".to_string(),
            session_id: Some("session-codex-live-existing-real".to_string()),
            run_id: Some("run-codex-live-existing-real".to_string()),
        },
    );
    let preflight = handle(
        &server,
        ServerCommand::PreflightLiveProvider {
            agent_name: "codex-local".to_string(),
            adapter: "codex".to_string(),
            goal: goal.to_string(),
            workspace: "/tmp/capo-workspace".to_string(),
            artifacts: "/tmp/capo-artifacts".to_string(),
            session_id: "session-codex-live-existing-real".to_string(),
            run_id: "run-codex-live-existing-real".to_string(),
            turn_id: "turn-codex-live-existing-real".to_string(),
            capability_profile: "trusted-local".to_string(),
            runtime_scope: "local_process_loopback".to_string(),
            credential_scan_policy: "metadata_only_no_secret_read".to_string(),
            raw_prompt_policy: "not_rendered".to_string(),
            raw_output_policy: "artifacts_scanned_redacted".to_string(),
            tool_wrapper_policy: "capo_wrapped_required".to_string(),
            live_provider_opt_in: true,
        },
    );
    let ServerResponsePayload::LiveProviderPreflighted(preflight) = preflight.payload else {
        panic!("expected live provider preflight response");
    };
    let (plan, _prompt_source) = server
        .dispatch_plan_with_prompt(&preflight.dispatch_plan_id)
        .expect("dispatch plan");
    let execution_request = server
        .latest_execution_request(&preflight.dispatch_plan_id)
        .expect("execution request");
    let existing_real_execution = server.dispatch_execution_projection(
        &plan,
        &execution_request,
        crate::dispatch::DispatchExecutionOutcome {
            status: "exited",
            provider_cli_executed: true,
            runtime_process_ref: Some("local-process-existing-real".to_string()),
            exit_code: Some(0),
            stdout_artifact_id: Some("stdout-existing-real".to_string()),
            stderr_artifact_id: Some("stderr-existing-real".to_string()),
            credential_scan_status: "clean",
            raw_output_policy: "bounded_redacted_artifacts",
            reason_codes: "provider_cli_executed_and_artifacts_scanned",
        },
    );
    server
        .append_dispatch_execution(
            &ServerClientOrigin {
                client_id: "test-client".to_string(),
                actor_id: "test-actor".to_string(),
                input_origin: ServerInputOrigin::System,
            },
            &plan,
            &existing_real_execution,
        )
        .expect("append existing real execution");

    let stale = handle(
        &server,
        ServerCommand::RunLiveProviderLocal {
            dispatch_plan_id: preflight.dispatch_plan_id,
            goal: "Changed live run prompt after existing execution".to_string(),
            live_execution_opt_in: true,
            mock_runtime_opt_in: false,
            mock_provider_output_name: None,
            mock_provider_output_jsonl: None,
            timeout_seconds: 1,
            codex_program_override: None,
            unattended: true,
        },
    );
    let ServerResponsePayload::DispatchRun(stale) = stale.payload else {
        panic!("expected stale prompt response");
    };
    assert!(!stale.provider_cli_executed);
    assert_eq!(stale.status, "blocked_by_live_provider_execution_gate");
    assert!(stale.reason_codes.contains("prompt_hash_mismatch"));
}

#[test]
fn server_live_provider_run_exit_audit_distinguishes_mock_and_real_metadata() {
    let root = temp_root();
    let server = CapoServer::open(ProjectId::new("project-capo"), &root).expect("server");
    let goal = "Run-exit audit metadata must differ";
    handle(
        &server,
        ServerCommand::RegisterAgent {
            name: "codex-local".to_string(),
            adapter: "fake".to_string(),
        },
    );
    handle(
        &server,
        ServerCommand::StartSession {
            agent_name: "codex-local".to_string(),
            goal: goal.to_string(),
            adapter: "codex".to_string(),
            session_id: Some("session-codex-live-run-exit-audit".to_string()),
            run_id: Some("run-codex-live-run-exit-audit".to_string()),
        },
    );
    let preflight = handle(
        &server,
        ServerCommand::PreflightLiveProvider {
            agent_name: "codex-local".to_string(),
            adapter: "codex".to_string(),
            goal: goal.to_string(),
            workspace: "/tmp/capo-workspace".to_string(),
            artifacts: "/tmp/capo-artifacts".to_string(),
            session_id: "session-codex-live-run-exit-audit".to_string(),
            run_id: "run-codex-live-run-exit-audit".to_string(),
            turn_id: "turn-codex-live-run-exit-audit".to_string(),
            capability_profile: "trusted-local".to_string(),
            runtime_scope: "local_process_loopback".to_string(),
            credential_scan_policy: "metadata_only_no_secret_read".to_string(),
            raw_prompt_policy: "not_rendered".to_string(),
            raw_output_policy: "artifacts_scanned_redacted".to_string(),
            tool_wrapper_policy: "capo_wrapped_required".to_string(),
            live_provider_opt_in: true,
        },
    );
    let ServerResponsePayload::LiveProviderPreflighted(preflight) = preflight.payload else {
        panic!("expected live provider preflight response");
    };
    let (plan, _prompt_source) = server
        .dispatch_plan_with_prompt(&preflight.dispatch_plan_id)
        .expect("dispatch plan");
    let (_session, run_projection, _agent, _run) = server
        .run_refs_for_session_run(&plan.session_id, &plan.run_id)
        .expect("run refs");
    let origin = ServerClientOrigin {
        client_id: "test-client".to_string(),
        actor_id: "test-actor".to_string(),
        input_origin: ServerInputOrigin::System,
    };

    server
        .append_dispatch_run_exit_with_metadata(
            &origin,
            &plan,
            &run_projection,
            false,
            "mock_live_provider_output_ingested_without_provider_cli",
        )
        .expect("append mocked run exit");
    server
        .append_dispatch_run_exit_with_metadata(
            &origin,
            &plan,
            &run_projection,
            true,
            "provider_cli_executed_and_artifacts_scanned",
        )
        .expect("append real run exit");

    let state = SqliteStateStore::open(&root).expect("state");
    let events = state
        .recent_events_for_session(&plan.session_id, 40)
        .expect("session events");
    assert!(events.iter().any(|event| {
        event.kind == "run.exited"
            && event
                .payload_json
                .contains("\"provider_cli_executed\":false")
            && event
                .payload_json
                .contains("mock_live_provider_output_ingested_without_provider_cli")
    }));
    assert!(events.iter().any(|event| {
        event.kind == "run.exited"
            && event
                .payload_json
                .contains("\"provider_cli_executed\":true")
            && event
                .payload_json
                .contains("provider_cli_executed_and_artifacts_scanned")
    }));
}

/// CS5 (a): the dispatch executor allow-list now ADMITS `claude_code`. A Claude
/// dispatch reaches `live_execution_blockers` carrying `adapter_kind ==
/// "claude_code"` (stamped by the preflight via `adapter_label`) and is no longer
/// hard-blocked; its mock `stream-json` output ingests into the run's read models
/// through `parse_adapter_events("claude_code", ..)`. This short-circuits at
/// `ingest_mock_live_provider_output` BEFORE the spawn arm, so it proves only the
/// allow-list widening + Claude ingestion, not the spawn arm (see CS5 (b)).
#[test]
fn server_live_provider_local_run_admits_claude_and_ingests_mock_stream_json() {
    let root = temp_root();
    let server = CapoServer::open(ProjectId::new("project-capo"), &root).expect("server");
    let goal = "Run Claude live provider through server";
    handle(
        &server,
        ServerCommand::RegisterAgent {
            name: "claude-local".to_string(),
            adapter: "fake".to_string(),
        },
    );
    handle(
        &server,
        ServerCommand::StartSession {
            agent_name: "claude-local".to_string(),
            goal: goal.to_string(),
            adapter: "claude".to_string(),
            session_id: Some("session-claude-live-run".to_string()),
            run_id: Some("run-claude-live-run".to_string()),
        },
    );
    let preflight = handle(
        &server,
        ServerCommand::PreflightLiveProvider {
            agent_name: "claude-local".to_string(),
            adapter: "claude".to_string(),
            goal: goal.to_string(),
            workspace: "/tmp/capo-workspace".to_string(),
            artifacts: "/tmp/capo-artifacts".to_string(),
            session_id: "session-claude-live-run".to_string(),
            run_id: "run-claude-live-run".to_string(),
            turn_id: "turn-claude-live-run".to_string(),
            capability_profile: "trusted-local".to_string(),
            runtime_scope: "local_process_loopback".to_string(),
            credential_scan_policy: "metadata_only_no_secret_read".to_string(),
            raw_prompt_policy: "not_rendered".to_string(),
            raw_output_policy: "artifacts_scanned_redacted".to_string(),
            tool_wrapper_policy: "capo_wrapped_required".to_string(),
            live_provider_opt_in: true,
        },
    );
    let ServerResponsePayload::LiveProviderPreflighted(preflight) = preflight.payload else {
        panic!("expected live provider preflight response");
    };
    // The plan carries `claude_code`, proving the allow-list is exercised on a
    // real Claude plan, not a tolerated string.
    let (plan, _prompt) = server
        .dispatch_plan_with_prompt(&preflight.dispatch_plan_id)
        .expect("dispatch plan");
    assert_eq!(plan.adapter_kind, "claude_code");

    let run = handle(
        &server,
        ServerCommand::RunLiveProviderLocal {
            dispatch_plan_id: preflight.dispatch_plan_id,
            goal: goal.to_string(),
            live_execution_opt_in: false,
            mock_runtime_opt_in: true,
            mock_provider_output_name: Some("claude-code-stream.jsonl".to_string()),
            mock_provider_output_jsonl: Some(
                include_str!("../../../capo-adapters/fixtures/claude-code-stream.jsonl")
                    .to_string(),
            ),
            timeout_seconds: 1,
            codex_program_override: None,
            unattended: true,
        },
    );
    let ServerResponsePayload::DispatchRun(run) = run.payload else {
        panic!("expected live run response");
    };
    // No longer blocked by the first-live-slice allow-list; the Claude
    // `stream-json` ingested.
    assert!(
        !run.reason_codes
            .contains("provider_not_enabled_for_first_live_slice"),
        "claude_code must be admitted by the live executor allow-list, got: {}",
        run.reason_codes
    );
    assert!(!run.provider_cli_executed);
    assert_eq!(run.status, "mocked_live_provider_output_ingested");
    assert!(
        run.input_event_count >= 1,
        "Claude stream-json must ingest normalized events"
    );

    let dashboard = server.dashboard_snapshot().expect("dashboard");
    let session = dashboard.agents[0].session.as_ref().expect("session");
    assert_eq!(
        session.dispatch_execution_status.as_deref(),
        Some("mocked_live_provider_output_ingested")
    );
    assert_eq!(session.run_status.as_deref(), Some("exited"));
}

/// CS5 (b): the unblocked SPAWN ARM, driven through an absolute-path Claude stub
/// via the new `claude_program_override`/`CAPO_CLAUDE_BIN` seam. This is the test
/// that actually proves the executor unblock through the spawn arm (the mock path
/// short-circuits before it). With the caller opt-in + a `LiveWrite` write mode,
/// the run builds `ClaudeCodeAdapter::local_workspace_write_launch_plan`, swaps in
/// the stub program, confines + checkpoints, spawns, scans, and ingests the
/// stub's `stream-json` through `apply_normalized_adapter_events_with_turn`.
#[cfg(unix)]
#[test]
fn server_live_provider_claude_spawn_arm_ingests_stub_stream_json_through_override() {
    use std::os::unix::fs::PermissionsExt;

    use crate::live_provider::LiveProviderLocalRunRequest;
    use crate::safety_floor::WriteMode;

    let root = temp_root();
    let server = CapoServer::open(ProjectId::new("project-capo"), &root).expect("server");
    let goal = "Apply a confined Claude workspace-write edit live".to_string();

    let workspace = root.join("workspace");
    let artifacts = root.join("artifacts");
    std::fs::create_dir_all(&workspace).expect("workspace");
    std::fs::write(workspace.join("seed.txt"), b"seed\n").expect("seed");
    let workspace_str = workspace.to_string_lossy().to_string();
    let artifacts_str = artifacts.to_string_lossy().to_string();

    handle(
        &server,
        ServerCommand::RegisterAgent {
            name: "claude-local".to_string(),
            adapter: "fake".to_string(),
        },
    );
    handle(
        &server,
        ServerCommand::StartSession {
            agent_name: "claude-local".to_string(),
            goal: goal.clone(),
            adapter: "claude".to_string(),
            session_id: Some("session-claude-spawn".to_string()),
            run_id: Some("run-claude-spawn".to_string()),
        },
    );
    let preflight = handle(
        &server,
        ServerCommand::PreflightLiveProvider {
            agent_name: "claude-local".to_string(),
            adapter: "claude".to_string(),
            goal: goal.clone(),
            workspace: workspace_str.clone(),
            artifacts: artifacts_str.clone(),
            session_id: "session-claude-spawn".to_string(),
            run_id: "run-claude-spawn".to_string(),
            turn_id: "turn-claude-spawn".to_string(),
            capability_profile: "trusted-local".to_string(),
            runtime_scope: "local_process_loopback".to_string(),
            credential_scan_policy: "metadata_only_no_secret_read".to_string(),
            raw_prompt_policy: "not_rendered".to_string(),
            raw_output_policy: "artifacts_scanned_redacted".to_string(),
            tool_wrapper_policy: "capo_wrapped_required".to_string(),
            live_provider_opt_in: true,
        },
    );
    let ServerResponsePayload::LiveProviderPreflighted(preflight) = preflight.payload else {
        panic!("expected preflight response");
    };

    // An executable Claude stub that ignores argv, applies an edit in the confined
    // workspace, and emits the Claude `stream-json` fixture on stdout. Uses only
    // POSIX shell builtins because the runtime spawns with `env_clear()` (no PATH).
    let stub = root.join("claude-stub.sh");
    let fixture_path = root.join("claude-stream-fixture.jsonl");
    std::fs::write(
        &fixture_path,
        include_str!("../../../capo-adapters/fixtures/claude-code-stream.jsonl"),
    )
    .expect("write fixture");
    let script = format!(
        "#!/bin/sh\nprintf 'edited by claude\\n' > \"$PWD/NOTES.md\"\nwhile IFS= read -r line; do printf '%s\\n' \"$line\"; done < {fixture}\n",
        fixture = fixture_path.display(),
    );
    std::fs::write(&stub, script).expect("write stub");
    let mut perms = std::fs::metadata(&stub).expect("stub meta").permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&stub, perms).expect("chmod stub");

    let origin = ServerClientOrigin {
        client_id: "test-client".to_string(),
        actor_id: "test-actor".to_string(),
        input_origin: ServerInputOrigin::System,
    };
    let selected_argv = std::cell::RefCell::new(Vec::new());
    let run = server
        .run_live_provider_local(
            &origin,
            LiveProviderLocalRunRequest {
                dispatch_plan_id: &preflight.dispatch_plan_id,
                goal: &goal,
                live_execution_opt_in: true,
                mock_runtime_opt_in: false,
                mock_provider_output_name: None,
                mock_provider_output_jsonl: None,
                timeout_seconds: 10,
                codex_program_override: None,
                // The Claude DISPATCH override pins the stub program; the Codex
                // override is irrelevant for a `claude_code` plan and must be
                // ignored.
                claude_program_override: Some(stub.to_string_lossy().as_ref()),
                write_mode: WriteMode::LiveWrite,
                record_selected_argv: Some(&selected_argv),
            },
        )
        .expect("claude spawn-arm run");

    // The spawn arm selected the Claude workspace-write profile (acceptEdits +
    // --add-dir), not a Codex profile.
    let argv = selected_argv.borrow();
    assert!(
        argv.windows(2)
            .any(|pair| pair == ["--permission-mode", "acceptEdits"]),
        "LiveWrite claude_code must select the workspace-write profile, got argv: {argv:?}"
    );
    assert!(
        argv.iter().any(|arg| arg == "--add-dir"),
        "claude workspace-write profile must carry --add-dir, got argv: {argv:?}"
    );
    assert!(
        !argv.iter().any(|arg| arg == "--sandbox"),
        "claude profile must not carry a Codex --sandbox flag, got argv: {argv:?}"
    );
    drop(argv);

    // The stub spawned and the Claude `stream-json` ingested through the SAME
    // ingestion route Codex uses.
    assert!(
        run.provider_cli_executed,
        "the spawn arm must spawn the (stub) provider"
    );
    assert_eq!(run.status, "exited");
    assert!(run.input_event_count >= 1);

    // The confined live write landed and a pre-write checkpoint was recorded.
    assert!(
        workspace.join("NOTES.md").exists(),
        "the confined live write must land in the workspace"
    );
    let state = SqliteStateStore::open(&root).expect("state");
    let events = state
        .recent_events_for_session(&SessionId::new("session-claude-spawn"), 64)
        .expect("events");
    assert!(
        events
            .iter()
            .any(|event| event.kind == "checkpoint.created"),
        "a pre-write checkpoint must be recorded for a live Claude write"
    );
}

/// CS5: the unblock is PROVIDER-SCOPED, not blanket. An unsupported adapter
/// (`acp`) cannot even reach the live executor: the live-provider preflight
/// rejects it up front, so no `claude_code`/`codex_exec`-style plan is ever
/// minted for it.
#[test]
fn server_live_provider_preflight_rejects_unsupported_acp_adapter() {
    let root = temp_root();
    let server = CapoServer::open(ProjectId::new("project-capo"), &root).expect("server");
    let goal = "Attempt acp live provider".to_string();
    handle(
        &server,
        ServerCommand::RegisterAgent {
            name: "acp-local".to_string(),
            adapter: "fake".to_string(),
        },
    );
    handle(
        &server,
        ServerCommand::StartSession {
            agent_name: "acp-local".to_string(),
            goal: goal.clone(),
            adapter: "acp".to_string(),
            session_id: Some("session-acp-live".to_string()),
            run_id: Some("run-acp-live".to_string()),
        },
    );
    let response = server.handle(ServerRequest::cli(ServerCommand::PreflightLiveProvider {
        agent_name: "acp-local".to_string(),
        adapter: "acp".to_string(),
        goal: goal.clone(),
        workspace: "/tmp/capo-workspace".to_string(),
        artifacts: "/tmp/capo-artifacts".to_string(),
        session_id: "session-acp-live".to_string(),
        run_id: "run-acp-live".to_string(),
        turn_id: "turn-acp-live".to_string(),
        capability_profile: "trusted-local".to_string(),
        runtime_scope: "local_process_loopback".to_string(),
        credential_scan_policy: "metadata_only_no_secret_read".to_string(),
        raw_prompt_policy: "not_rendered".to_string(),
        raw_output_policy: "artifacts_scanned_redacted".to_string(),
        tool_wrapper_policy: "capo_wrapped_required".to_string(),
        live_provider_opt_in: true,
    }));
    let error = response.expect_err("acp live preflight must be rejected");
    assert!(
        format!("{error:?}").contains("acp"),
        "acp must be rejected from the live provider preflight, got: {error:?}"
    );
}

#[test]
fn server_live_provider_local_run_rejects_credential_like_paths() {
    let root = temp_root();
    let server = CapoServer::open(ProjectId::new("project-capo"), &root).expect("server");
    let goal = "Reject unsafe live provider paths";
    handle(
        &server,
        ServerCommand::RegisterAgent {
            name: "codex-local".to_string(),
            adapter: "fake".to_string(),
        },
    );
    handle(
        &server,
        ServerCommand::StartSession {
            agent_name: "codex-local".to_string(),
            goal: goal.to_string(),
            adapter: "codex".to_string(),
            session_id: Some("session-codex-live-unsafe-path".to_string()),
            run_id: Some("run-codex-live-unsafe-path".to_string()),
        },
    );
    let preflight = handle(
        &server,
        ServerCommand::PreflightLiveProvider {
            agent_name: "codex-local".to_string(),
            adapter: "codex".to_string(),
            goal: goal.to_string(),
            workspace: "/tmp/capo-workspace".to_string(),
            artifacts: "/tmp/.codex/capo-artifacts".to_string(),
            session_id: "session-codex-live-unsafe-path".to_string(),
            run_id: "run-codex-live-unsafe-path".to_string(),
            turn_id: "turn-codex-live-unsafe-path".to_string(),
            capability_profile: "trusted-local".to_string(),
            runtime_scope: "local_process_loopback".to_string(),
            credential_scan_policy: "metadata_only_no_secret_read".to_string(),
            raw_prompt_policy: "not_rendered".to_string(),
            raw_output_policy: "artifacts_scanned_redacted".to_string(),
            tool_wrapper_policy: "capo_wrapped_required".to_string(),
            live_provider_opt_in: true,
        },
    );
    let ServerResponsePayload::LiveProviderPreflighted(preflight) = preflight.payload else {
        panic!("expected live provider preflight response");
    };
    assert!(preflight.provider_cli_execution_allowed);

    let run = handle(
        &server,
        ServerCommand::RunLiveProviderLocal {
            dispatch_plan_id: preflight.dispatch_plan_id,
            goal: goal.to_string(),
            live_execution_opt_in: false,
            mock_runtime_opt_in: true,
            mock_provider_output_name: Some("codex-exec.jsonl".to_string()),
            mock_provider_output_jsonl: Some(
                include_str!("../../../capo-adapters/fixtures/codex-exec.jsonl").to_string(),
            ),
            timeout_seconds: 1,
            codex_program_override: None,
            unattended: true,
        },
    );
    let ServerResponsePayload::DispatchRun(run) = run.payload else {
        panic!("expected live run response");
    };
    assert!(!run.provider_cli_executed);
    assert_eq!(run.status, "blocked_by_live_provider_execution_gate");
    assert!(run.reason_codes.contains("unsafe artifact root"));
}

#[cfg(unix)]
#[test]
fn server_live_provider_local_run_rejects_symlinked_credential_paths() {
    use std::os::unix::fs::symlink;

    let root = temp_root();
    let credential_like_artifacts = root.join(".codex");
    std::fs::create_dir_all(&credential_like_artifacts).expect("credential-like dir");
    let artifact_link = root.join("artifact-link");
    symlink(&credential_like_artifacts, &artifact_link).expect("artifact symlink");

    let server = CapoServer::open(ProjectId::new("project-capo"), &root).expect("server");
    let goal = "Reject symlinked unsafe live provider paths";
    handle(
        &server,
        ServerCommand::RegisterAgent {
            name: "codex-local".to_string(),
            adapter: "fake".to_string(),
        },
    );
    handle(
        &server,
        ServerCommand::StartSession {
            agent_name: "codex-local".to_string(),
            goal: goal.to_string(),
            adapter: "codex".to_string(),
            session_id: Some("session-codex-live-symlink-path".to_string()),
            run_id: Some("run-codex-live-symlink-path".to_string()),
        },
    );
    let preflight = handle(
        &server,
        ServerCommand::PreflightLiveProvider {
            agent_name: "codex-local".to_string(),
            adapter: "codex".to_string(),
            goal: goal.to_string(),
            workspace: root.display().to_string(),
            artifacts: artifact_link.display().to_string(),
            session_id: "session-codex-live-symlink-path".to_string(),
            run_id: "run-codex-live-symlink-path".to_string(),
            turn_id: "turn-codex-live-symlink-path".to_string(),
            capability_profile: "trusted-local".to_string(),
            runtime_scope: "local_process_loopback".to_string(),
            credential_scan_policy: "metadata_only_no_secret_read".to_string(),
            raw_prompt_policy: "not_rendered".to_string(),
            raw_output_policy: "artifacts_scanned_redacted".to_string(),
            tool_wrapper_policy: "capo_wrapped_required".to_string(),
            live_provider_opt_in: true,
        },
    );
    let ServerResponsePayload::LiveProviderPreflighted(preflight) = preflight.payload else {
        panic!("expected live provider preflight response");
    };
    assert!(preflight.provider_cli_execution_allowed);

    let run = handle(
        &server,
        ServerCommand::RunLiveProviderLocal {
            dispatch_plan_id: preflight.dispatch_plan_id,
            goal: goal.to_string(),
            live_execution_opt_in: false,
            mock_runtime_opt_in: true,
            mock_provider_output_name: Some("codex-exec.jsonl".to_string()),
            mock_provider_output_jsonl: Some(
                include_str!("../../../capo-adapters/fixtures/codex-exec.jsonl").to_string(),
            ),
            timeout_seconds: 1,
            codex_program_override: None,
            unattended: true,
        },
    );
    let ServerResponsePayload::DispatchRun(run) = run.payload else {
        panic!("expected live run response");
    };
    assert!(!run.provider_cli_executed);
    assert_eq!(run.status, "blocked_by_live_provider_execution_gate");
    assert!(run.reason_codes.contains("unsafe artifact root"));
}
