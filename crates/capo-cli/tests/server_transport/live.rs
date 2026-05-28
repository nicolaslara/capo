use std::io::BufReader;
use std::path::PathBuf;

use super::support::*;

#[test]
fn cli_preflights_live_codex_and_claude_through_running_server_process() {
    let state_root = temp_root("transport-live-preflight-state");
    let mut server = spawn_server(&state_root, 7);
    let stdout = server.stdout.take().expect("server stdout");
    let mut reader = BufReader::new(stdout);
    let address = read_server_address(&mut reader);

    for (agent, adapter, session, run) in [
        (
            "codex-live",
            "codex",
            "session-codex-live",
            "run-codex-live",
        ),
        (
            "claude-live",
            "claude",
            "session-claude-live",
            "run-claude-live",
        ),
    ] {
        let register = capo([
            "server",
            "agent",
            "register",
            "--name",
            agent,
            "--adapter",
            "fake",
            "--runtime",
            "fake",
            "--connect",
            &address,
            "--state",
            &state_root.display().to_string(),
        ]);
        assert!(register.contains("server_agent_registered=true"));

        let start = capo([
            "server",
            "session",
            "start",
            "--agent",
            agent,
            "--adapter",
            adapter,
            "--goal",
            "Preflight live provider",
            "--session",
            session,
            "--run",
            run,
            "--connect",
            &address,
            "--state",
            &state_root.display().to_string(),
        ]);
        assert!(start.contains("server_session_started=true"));

        let preflight = capo_with_env(
            [
                "server",
                "dispatch",
                "live-preflight",
                "--agent",
                agent,
                "--adapter",
                adapter,
                "--goal",
                "Preflight live provider",
                "--session",
                session,
                "--run",
                run,
                "--turn",
                "turn-live-preflight",
                "--workspace",
                ".",
                "--artifacts",
                ".capo-artifacts",
                "--connect",
                &address,
                "--state",
                &state_root.display().to_string(),
            ],
            [("CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT", "1")],
        );
        assert!(preflight.contains("server_dispatch_live_preflight=true"));
        assert!(preflight.contains("provider_cli_execution_allowed=true"));
        assert!(preflight.contains("provider_cli_executed=false"));
        assert!(preflight.contains("status=ready_for_live_provider_execution"));
        assert!(preflight.contains("next_action=run_explicit_live_provider_execution"));
    }

    let dashboard = capo([
        "server",
        "dashboard",
        "--connect",
        &address,
        "--state",
        &state_root.display().to_string(),
    ]);
    assert!(dashboard.contains("dispatch_gate_status=ready_for_live_provider_execution"));
    assert!(dashboard.contains("dispatch_next_action=ready_for_explicit_live_provider_run"));
    assert!(dashboard.contains("dispatch_provider_cli_executed=none"));
    assert!(server.wait().expect("server wait").success());
}

#[test]
fn cli_live_runs_codex_mock_output_through_running_server_process() {
    let state_root = temp_root("transport-live-run-state");
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../capo-adapters/fixtures/codex-exec.jsonl");
    let mut server = spawn_server(&state_root, 5);
    let stdout = server.stdout.take().expect("server stdout");
    let mut reader = BufReader::new(stdout);
    let address = read_server_address(&mut reader);

    let register = capo([
        "server",
        "agent",
        "register",
        "--name",
        "codex-live-run",
        "--adapter",
        "fake",
        "--runtime",
        "fake",
        "--connect",
        &address,
        "--state",
        &state_root.display().to_string(),
    ]);
    assert!(register.contains("server_agent_registered=true"));

    let start = capo([
        "server",
        "session",
        "start",
        "--agent",
        "codex-live-run",
        "--adapter",
        "codex",
        "--goal",
        "Run Codex live provider through server transport",
        "--session",
        "session-codex-live-run",
        "--run",
        "run-codex-live-run",
        "--connect",
        &address,
        "--state",
        &state_root.display().to_string(),
    ]);
    assert!(start.contains("server_session_started=true"));

    let preflight = capo_with_env(
        [
            "server",
            "dispatch",
            "live-preflight",
            "--agent",
            "codex-live-run",
            "--adapter",
            "codex",
            "--goal",
            "Run Codex live provider through server transport",
            "--session",
            "session-codex-live-run",
            "--run",
            "run-codex-live-run",
            "--turn",
            "turn-codex-live-run",
            "--workspace",
            ".",
            "--artifacts",
            ".capo-artifacts",
            "--connect",
            &address,
            "--state",
            &state_root.display().to_string(),
        ],
        [("CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT", "1")],
    );
    assert!(preflight.contains("server_dispatch_live_preflight=true"));
    assert!(preflight.contains("status=ready_for_live_provider_execution"));
    let dispatch_plan_id = output_value(&preflight, "dispatch_plan_id");

    let run = capo_with_env(
        [
            "server",
            "dispatch",
            "live-run-local",
            "--dispatch-plan",
            &dispatch_plan_id,
            "--goal",
            "Run Codex live provider through server transport",
            "--mock-fixture",
            &fixture.display().to_string(),
            "--connect",
            &address,
            "--state",
            &state_root.display().to_string(),
        ],
        [("CAPO_SERVER_MOCK_LIVE_PROVIDER_RUNTIME", "1")],
    );
    assert!(run.contains("server_dispatch_live_run_local=true"));
    assert!(run.contains("adapter=codex_exec"));
    assert!(run.contains("provider_cli_execution_allowed=true"));
    assert!(run.contains("provider_cli_executed=false"));
    assert!(run.contains("mock_runtime_opt_in=true"));
    assert!(run.contains("status=mocked_live_provider_output_ingested"));
    assert!(run.contains("credential_scan_status=not_applicable_mock"));
    assert!(run.contains("raw_output_policy=content_hashed_not_rendered"));
    assert!(run.contains("tool_events=2"));
    assert!(!run.contains("Codex fixture response."));

    let dashboard = capo([
        "server",
        "dashboard",
        "--connect",
        &address,
        "--state",
        &state_root.display().to_string(),
    ]);
    assert!(dashboard.contains("server_dashboard=true"));
    assert!(dashboard.contains("dispatch_execution_status=mocked_live_provider_output_ingested"));
    assert!(dashboard.contains("dispatch_provider_cli_executed=false"));
    assert!(dashboard.contains("run_status=exited"));
    assert!(dashboard.contains("turn_ids=turn-codex-live-run"));
    assert!(server.wait().expect("server wait").success());
}
