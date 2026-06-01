use super::*;

#[test]
fn server_dispatch_plan_gate_and_run_local_ingest_codex_fixture_idempotently() {
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
            goal: "Run Codex fixture through server dispatch".to_string(),
            adapter: "codex".to_string(),
            session_id: Some("session-codex-dispatch".to_string()),
            run_id: Some("run-codex-dispatch".to_string()),
        },
    );

    let planned = handle(
        &server,
        ServerCommand::PlanDispatch {
            agent_name: "codex-local".to_string(),
            adapter: "codex".to_string(),
            goal: "Run Codex fixture through server dispatch".to_string(),
            workspace: "/tmp/capo-workspace".to_string(),
            artifacts: "/tmp/capo-artifacts".to_string(),
            session_id: "session-codex-dispatch".to_string(),
            run_id: "run-codex-dispatch".to_string(),
            turn_id: "turn-codex-dispatch".to_string(),
            deterministic_opt_in: true,
        },
    );
    let ServerResponsePayload::DispatchPlanned(plan) = planned.payload else {
        panic!("expected dispatch planned response");
    };
    assert_eq!(plan.adapter, "codex_exec");
    assert_eq!(plan.agent_name, "codex-local");
    assert_eq!(plan.session_id.as_str(), "session-codex-dispatch");
    assert_eq!(plan.run_id.as_str(), "run-codex-dispatch");
    assert_eq!(plan.runtime_program, "deterministic-fixture-runtime");
    assert_eq!(plan.runtime_prompt_policy, "not_rendered");
    assert_eq!(plan.raw_prompt_policy, "not_rendered");
    assert!(!plan.provider_cli_executed);
    assert_eq!(plan.status, "planned");

    let gated = handle(
        &server,
        ServerCommand::GateDispatch {
            dispatch_plan_id: plan.dispatch_plan_id.clone(),
        },
    );
    let ServerResponsePayload::DispatchGated(gate) = gated.payload else {
        panic!("expected dispatch gated response");
    };
    assert_eq!(gate.dispatch_plan_id, plan.dispatch_plan_id);
    assert!(gate.provider_cli_execution_allowed);
    assert!(!gate.provider_cli_executed);
    assert_eq!(gate.status, "ready_for_deterministic_execution");
    assert_eq!(gate.reasons, "deterministic_fixture_dispatch_allowed");
    assert_eq!(gate.raw_prompt_policy, "not_rendered");

    let run = server
        .handle(ServerRequest::local_cli(
            "run-codex-dispatch-once",
            ServerCommand::RunDispatchLocal {
                dispatch_plan_id: plan.dispatch_plan_id.clone(),
                fixture_name: "crates/capo-adapters/fixtures/codex-exec.jsonl".to_string(),
                fixture_jsonl: include_str!("../../../capo-adapters/fixtures/codex-exec.jsonl")
                    .to_string(),
            },
        ))
        .expect("dispatch run-local");
    let ServerResponsePayload::DispatchRun(run) = run.payload else {
        panic!("expected dispatch run response");
    };
    assert_eq!(run.dispatch_plan_id, plan.dispatch_plan_id);
    assert_eq!(run.adapter, "codex_exec");
    assert_eq!(run.session_id.as_str(), "session-codex-dispatch");
    assert_eq!(run.run_id.as_str(), "run-codex-dispatch");
    assert!(run.provider_cli_execution_allowed);
    assert!(!run.provider_cli_executed);
    assert_eq!(run.status, "exited");
    assert_eq!(run.credential_scan_status, "not_applicable_fixture");
    assert_eq!(run.raw_prompt_policy, "not_rendered");
    assert_eq!(run.raw_output_policy, "content_hashed_not_rendered");
    assert_eq!(run.input_event_count, 5);
    assert_eq!(run.appended_event_count, 6);
    assert_eq!(run.tool_event_count, 2);
    assert_eq!(run.completed_turn_count, 1);

    let dashboard = server.dashboard_snapshot().expect("dashboard");
    let agent = dashboard
        .agents
        .iter()
        .find(|agent| agent.name == "codex-local")
        .expect("codex agent");
    let session = agent.session.as_ref().expect("dispatch session");
    assert_eq!(
        session.latest_dispatch_plan_id.as_deref(),
        Some(plan.dispatch_plan_id.as_str())
    );
    assert_eq!(
        session.latest_dispatch_gate_id.as_deref(),
        Some(gate.dispatch_gate_id.as_str())
    );
    assert_eq!(
        session.latest_dispatch_execution_id.as_deref(),
        Some(run.dispatch_execution_id.as_str())
    );
    assert_eq!(session.dispatch_execution_status.as_deref(), Some("exited"));
    assert_eq!(
        session.dispatch_credential_scan_status.as_deref(),
        Some("not_applicable_fixture")
    );
    assert_eq!(session.dispatch_provider_cli_execution_allowed, Some(true));
    assert_eq!(session.dispatch_provider_cli_executed, Some(false));
    assert_eq!(
        session.dispatch_raw_prompt_policy.as_deref(),
        Some("not_rendered")
    );
    assert_eq!(
        session.dispatch_raw_output_policy.as_deref(),
        Some("content_hashed_not_rendered")
    );
    assert_eq!(session.turn_ids, vec!["turn-codex-dispatch"]);
    assert_eq!(session.run_status.as_deref(), Some("exited"));
    assert_eq!(session.tool_call_count, 1);
    assert_eq!(session.tool_observation_count, 1);
    assert_eq!(session.evidence_count, 1);

    let state = SqliteStateStore::open(&root).expect("state");
    let event_count_before_repeat = state.event_count().expect("event count before repeat");
    let repeated = server
        .handle(ServerRequest::local_cli(
            "run-codex-dispatch-once",
            ServerCommand::RunDispatchLocal {
                dispatch_plan_id: plan.dispatch_plan_id.clone(),
                fixture_name: "crates/capo-adapters/fixtures/codex-exec.jsonl".to_string(),
                fixture_jsonl: include_str!("../../../capo-adapters/fixtures/codex-exec.jsonl")
                    .to_string(),
            },
        ))
        .expect("repeat dispatch run-local");
    let ServerResponsePayload::DispatchRun(repeated) = repeated.payload else {
        panic!("expected repeated dispatch run response");
    };
    assert_eq!(repeated.dispatch_execution_id, run.dispatch_execution_id);
    assert_eq!(repeated.appended_event_count, 0);
    assert_eq!(repeated.tool_event_count, 0);
    assert_eq!(repeated.completed_turn_count, 0);
    let dashboard_after_repeat = server.dashboard_snapshot().expect("dashboard after repeat");
    let session_after_repeat = dashboard_after_repeat.agents[0]
        .session
        .as_ref()
        .expect("session after repeat");
    assert_eq!(session_after_repeat.tool_call_count, 1);
    assert_eq!(session_after_repeat.tool_observation_count, 1);
    assert_eq!(session_after_repeat.turn_ids, vec!["turn-codex-dispatch"]);
    assert_eq!(
        state.event_count().expect("event count after repeat"),
        event_count_before_repeat,
        "the same execution request should be idempotent and avoid duplicate process/projection/audit state"
    );

    let changed_fixture = format!(
        "{}\n",
        include_str!("../../../capo-adapters/fixtures/codex-exec.jsonl")
    );
    let changed_error = server
        .handle(ServerRequest::local_cli(
            "run-codex-dispatch-changed-fixture",
            ServerCommand::RunDispatchLocal {
                dispatch_plan_id: plan.dispatch_plan_id.clone(),
                fixture_name: "codex-exec-with-extra-newline.jsonl".to_string(),
                fixture_jsonl: changed_fixture,
            },
        ))
        .expect_err("changed fixture should be rejected after first dispatch run");
    assert!(
        matches!(changed_error, ServerError::AdapterFixture(message) if message.contains("already ran with fixture hash"))
    );
}

#[test]
fn server_dispatch_gate_blocks_without_deterministic_opt_in() {
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
            goal: "Plan without deterministic opt-in".to_string(),
            adapter: "codex".to_string(),
            session_id: Some("session-codex-no-opt".to_string()),
            run_id: Some("run-codex-no-opt".to_string()),
        },
    );
    let planned = handle(
        &server,
        ServerCommand::PlanDispatch {
            agent_name: "codex-local".to_string(),
            adapter: "codex".to_string(),
            goal: "Plan without deterministic opt-in".to_string(),
            workspace: "/tmp/capo-workspace".to_string(),
            artifacts: "/tmp/capo-artifacts".to_string(),
            session_id: "session-codex-no-opt".to_string(),
            run_id: "run-codex-no-opt".to_string(),
            turn_id: "turn-codex-no-opt".to_string(),
            deterministic_opt_in: false,
        },
    );
    let ServerResponsePayload::DispatchPlanned(plan) = planned.payload else {
        panic!("expected dispatch planned response");
    };
    let gated = handle(
        &server,
        ServerCommand::GateDispatch {
            dispatch_plan_id: plan.dispatch_plan_id.clone(),
        },
    );
    let ServerResponsePayload::DispatchGated(gate) = gated.payload else {
        panic!("expected dispatch gated response");
    };
    assert!(!gate.provider_cli_execution_allowed);
    assert_eq!(gate.status, "blocked");
    assert!(
        gate.reasons
            .contains("missing_deterministic_fixture_opt_in")
    );

    let blocked = handle(
        &server,
        ServerCommand::RunDispatchLocal {
            dispatch_plan_id: plan.dispatch_plan_id.clone(),
            fixture_name: "crates/capo-adapters/fixtures/codex-exec.jsonl".to_string(),
            fixture_jsonl: include_str!("../../../capo-adapters/fixtures/codex-exec.jsonl")
                .to_string(),
        },
    );
    let ServerResponsePayload::DispatchRun(blocked) = blocked.payload else {
        panic!("expected blocked dispatch run response");
    };
    assert_eq!(blocked.status, "blocked_by_preflight");
    assert!(!blocked.provider_cli_executed);
    assert_eq!(blocked.credential_scan_status, "not_run");
    assert_eq!(blocked.appended_event_count, 0);
    let dashboard = server.dashboard_snapshot().expect("dashboard");
    let session = dashboard.agents[0].session.as_ref().expect("session");
    assert_eq!(session.run_status.as_deref(), Some("running"));
    assert_eq!(session.dispatch_provider_cli_execution_allowed, Some(false));
}
