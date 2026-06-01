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

/// AI2 END-TO-END (CLI process path): a user can `capo server agent register
/// --adapter codex` against a RUNNING server and `capo server task send` to get
/// REAL Codex output back -- here a deterministic absolute-path stub pinned via
/// `CAPO_CODEX_BIN`, with the live-provider gate opened in the SERVER process.
///
/// This is the reachability the AI2 wiring closed: before it,
/// `capo server agent register --adapter codex` was rejected client-side and the
/// server only ever bound the fake adapter, so real-Codex chat via SendTask was
/// unreachable by a user.
#[cfg(unix)]
#[test]
fn cli_registers_codex_agent_and_gets_real_stub_chat_through_running_server() {
    let state_root = temp_root("transport-codex-chat-e2e-state");
    let stub = write_codex_stub(
        &state_root.join("stub"),
        "{\"type\":\"thread.started\",\"thread_id\":\"cli-codex-thread\"}\n\
{\"type\":\"item.completed\",\"item\":{\"id\":\"item-1\",\"type\":\"agent_message\",\"text\":\"CLI_CODEX_STUB_CHAT\"}}\n\
{\"type\":\"turn.completed\"}\n",
    );

    // The server reads CAPO_CODEX_BIN at open time and the live gate at chat time,
    // so they are set in the SERVER process (the gate is a server-side concern).
    let mut server = spawn_server_with_env(
        &state_root,
        3,
        &[
            ("CAPO_CODEX_BIN", stub.as_str()),
            ("CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT", "1"),
            ("CAPO_SERVER_RUN_CODEX_LIVE", "1"),
        ],
    );
    let stdout = server.stdout.take().expect("server stdout");
    let mut reader = BufReader::new(stdout);
    let address = read_server_address(&mut reader);
    let state = state_root.display().to_string();

    // The relaxed `--adapter codex` is now ACCEPTED by the client and binds the
    // agent's chat adapter on the server.
    let register = capo([
        "server",
        "agent",
        "register",
        "--name",
        "codex-chat",
        "--adapter",
        "codex",
        "--connect",
        &address,
        "--state",
        &state,
    ]);
    assert!(register.contains("server_agent_registered=true"));

    let send = capo([
        "server",
        "task",
        "send",
        "--agent",
        "codex-chat",
        "--goal",
        "Summarize through real Codex",
        "--connect",
        &address,
        "--state",
        &state,
    ]);
    assert!(send.contains("server_task_sent=true"));

    let status = capo([
        "server",
        "agent",
        "status",
        "--agent",
        "codex-chat",
        "--connect",
        &address,
        "--state",
        &state,
    ]);
    // The rendered session summary is the STUB's parsed Codex agent_message text,
    // proving REAL (stub) chat output flowed back, not a fake summary.
    assert!(
        status.contains("latest_summary=CLI_CODEX_STUB_CHAT"),
        "expected real codex stub summary in status output:\n{status}"
    );
    assert!(!status.contains("Fake adapter processed goal"));

    assert!(server.wait().expect("server wait").success());
}

/// AI2: a `--adapter` value other than `fake`/`codex` is rejected by the CLI
/// before it ever reaches the server.
#[test]
fn cli_rejects_unsupported_chat_adapter_on_register() {
    let state_root = temp_root("transport-codex-bad-adapter-state");
    let mut server = spawn_server(&state_root, 1);
    let stdout = server.stdout.take().expect("server stdout");
    let mut reader = BufReader::new(stdout);
    let address = read_server_address(&mut reader);
    let state = state_root.display().to_string();

    let output = std::process::Command::new(capo_bin())
        .args([
            "server",
            "agent",
            "register",
            "--name",
            "bad-adapter",
            "--adapter",
            "claude",
            "--connect",
            &address,
            "--state",
            &state,
        ])
        .output()
        .expect("run capo");
    assert!(
        !output.status.success(),
        "register with an unsupported chat adapter must fail"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("supports `fake` (default) or `codex`"),
        "expected the chat-adapter rejection message, got: {stderr}"
    );

    // The server never received a register, so it is still waiting; send one
    // request to let it reach its budget and exit cleanly.
    let _ = capo([
        "server",
        "agent",
        "register",
        "--name",
        "ok-agent",
        "--connect",
        &address,
        "--state",
        &state,
    ]);
    assert!(server.wait().expect("server wait").success());
}
