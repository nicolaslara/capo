use std::io::BufReader;
use std::path::PathBuf;

use super::support::*;

#[test]
fn cli_dispatches_codex_fixture_through_running_server_process() {
    let state_root = temp_root("transport-dispatch-state");
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../capo-adapters/fixtures/codex-exec.jsonl");
    let mut server = spawn_server(&state_root, 6);
    let stdout = server.stdout.take().expect("server stdout");
    let mut reader = BufReader::new(stdout);
    let address = read_server_address(&mut reader);

    let register = capo([
        "server",
        "agent",
        "register",
        "--name",
        "codex-dispatch",
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
        "codex-dispatch",
        "--adapter",
        "codex",
        "--goal",
        "Dispatch Codex through server transport",
        "--session",
        "session-codex-dispatch",
        "--run",
        "run-codex-dispatch",
        "--connect",
        &address,
        "--state",
        &state_root.display().to_string(),
    ]);
    assert!(start.contains("server_session_started=true"));
    assert!(start.contains("session_id=session-codex-dispatch"));

    let plan = capo_with_env(
        [
            "server",
            "dispatch",
            "plan",
            "--agent",
            "codex-dispatch",
            "--adapter",
            "codex",
            "--goal",
            "Dispatch Codex through server transport",
            "--session",
            "session-codex-dispatch",
            "--run",
            "run-codex-dispatch",
            "--turn",
            "turn-codex-dispatch",
            "--workspace",
            ".",
            "--artifacts",
            ".capo-artifacts",
            "--connect",
            &address,
            "--state",
            &state_root.display().to_string(),
        ],
        [("CAPO_SERVER_DETERMINISTIC_DISPATCH", "1")],
    );
    assert!(plan.contains("server_dispatch_planned=true"));
    assert!(plan.contains("adapter=codex_exec"));
    assert!(plan.contains("runtime_program=deterministic-fixture-runtime"));
    assert!(plan.contains("raw_prompt_policy=not_rendered"));
    assert!(plan.contains("provider_cli_executed=false"));
    let dispatch_plan_id = output_value(&plan, "dispatch_plan_id");

    let gate = capo([
        "server",
        "dispatch",
        "gate",
        "--dispatch-plan",
        &dispatch_plan_id,
        "--connect",
        &address,
        "--state",
        &state_root.display().to_string(),
    ]);
    assert!(gate.contains("server_dispatch_gated=true"));
    assert!(gate.contains("provider_cli_execution_allowed=true"));
    assert!(gate.contains("provider_cli_executed=false"));
    assert!(gate.contains("reasons=deterministic_fixture_dispatch_allowed"));

    let run = capo([
        "server",
        "dispatch",
        "run-local",
        "--dispatch-plan",
        &dispatch_plan_id,
        "--fixture",
        &fixture.display().to_string(),
        "--connect",
        &address,
        "--state",
        &state_root.display().to_string(),
    ]);
    assert!(run.contains("server_dispatch_run_local=true"));
    assert!(run.contains("adapter=codex_exec"));
    assert!(run.contains("provider_cli_execution_allowed=true"));
    assert!(run.contains("provider_cli_executed=false"));
    assert!(run.contains("status=exited"));
    assert!(run.contains("credential_scan_status=not_applicable_fixture"));
    assert!(run.contains("raw_prompt_policy=not_rendered"));
    assert!(run.contains("raw_output_policy=content_hashed_not_rendered"));
    assert!(run.contains("tool_events=2"));
    assert!(!run.contains("Codex fixture response."));
    let dispatch_execution_id = output_value(&run, "dispatch_execution_id");

    let dashboard = capo([
        "server",
        "dashboard",
        "--connect",
        &address,
        "--state",
        &state_root.display().to_string(),
    ]);
    assert!(dashboard.contains("server_dashboard=true"));
    assert!(dashboard.contains("agent=codex-dispatch status=running"));
    assert!(dashboard.contains("adapter_kind=codex_exec"));
    assert!(dashboard.contains(&format!("latest_dispatch_plan={dispatch_plan_id}")));
    assert!(dashboard.contains(&format!(
        "latest_dispatch_execution={dispatch_execution_id}"
    )));
    assert!(dashboard.contains("dispatch_execution_status=exited"));
    assert!(dashboard.contains("run_status=exited"));
    assert!(dashboard.contains("dispatch_provider_cli_execution_allowed=true"));
    assert!(dashboard.contains("dispatch_provider_cli_executed=false"));
    assert!(dashboard.contains("dispatch_credential_scan_status=not_applicable_fixture"));
    assert!(dashboard.contains("dispatch_raw_output_policy=content_hashed_not_rendered"));
    assert!(dashboard.contains("turn_ids=turn-codex-dispatch"));
    assert!(dashboard.contains("tool_calls=1"));

    assert!(server.wait().expect("server wait").success());

    let mut restarted = spawn_server(&state_root, 2);
    let restarted_stdout = restarted.stdout.take().expect("restarted stdout");
    let mut restarted_reader = BufReader::new(restarted_stdout);
    let restarted_address = read_server_address(&mut restarted_reader);
    let recover = capo([
        "server",
        "recover",
        "--connect",
        &restarted_address,
        "--state",
        &state_root.display().to_string(),
    ]);
    assert!(recover.contains("server_recovered=true"));
    assert!(recover.contains("recovered_run_count=0"));

    let status = capo([
        "server",
        "agent",
        "status",
        "--agent",
        "codex-dispatch",
        "--connect",
        &restarted_address,
        "--state",
        &state_root.display().to_string(),
    ]);
    assert!(status.contains("run_status=exited"));
    assert!(status.contains(&format!("latest_dispatch_plan={dispatch_plan_id}")));
    assert!(status.contains(&format!(
        "latest_dispatch_execution={dispatch_execution_id}"
    )));
    assert!(status.contains("dispatch_execution_status=exited"));
    assert!(status.contains("dispatch_provider_cli_execution_allowed=true"));
    assert!(status.contains("dispatch_provider_cli_executed=false"));
    assert!(status.contains("dispatch_credential_scan_status=not_applicable_fixture"));
    assert!(status.contains("dispatch_raw_prompt_policy=not_rendered"));
    assert!(status.contains("turn_ids=turn-codex-dispatch"));
    assert!(status.contains("tool_calls=1"));
    assert!(restarted.wait().expect("restarted wait").success());
}
