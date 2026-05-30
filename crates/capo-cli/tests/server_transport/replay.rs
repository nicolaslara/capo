use std::io::BufReader;
use std::path::PathBuf;

use capo_core::SessionId;
use capo_state::SqliteStateStore;

use super::support::*;

#[test]
fn cli_replays_codex_fixture_through_running_server_process() {
    let state_root = temp_root("transport-codex-state");
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../capo-adapters/fixtures/codex-exec.jsonl");
    let mut server = spawn_server(&state_root, 4);
    let stdout = server.stdout.take().expect("server stdout");
    let mut reader = BufReader::new(stdout);
    let address = read_server_address(&mut reader);

    let register = capo([
        "server",
        "agent",
        "register",
        "--name",
        "codex-local",
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
        "codex-local",
        "--adapter",
        "codex",
        "--goal",
        "Replay Codex through server transport",
        "--session",
        "session-codex-local",
        "--run",
        "run-codex-local",
        "--connect",
        &address,
        "--state",
        &state_root.display().to_string(),
    ]);
    assert!(start.contains("server_session_started=true"));
    assert!(start.contains("session_id=session-codex-local"));
    assert!(start.contains("provider_cli_executed=false"));

    let replay = capo([
        "server",
        "adapter",
        "replay-fixture",
        "--adapter",
        "codex",
        "--fixture",
        &fixture.display().to_string(),
        "--session",
        "session-codex-local",
        "--run",
        "run-codex-local",
        "--turn",
        "turn-codex-local",
        "--connect",
        &address,
        "--state",
        &state_root.display().to_string(),
    ]);
    assert!(replay.contains("server_adapter_replayed=true"));
    assert!(replay.contains("server_boundary=capo-server"));
    assert!(replay.contains("adapter=codex_exec"));
    assert!(replay.contains("run_id=run-codex-local"));
    assert!(replay.contains("turn_id=turn-codex-local"));
    assert!(replay.contains("provider_cli_executed=false"));
    assert!(replay.contains("raw_content_policy=content_hashed_not_rendered"));
    assert!(replay.contains("tool_events=2"));
    assert!(!replay.contains("Codex fixture response."));

    let dashboard = capo([
        "server",
        "dashboard",
        "--connect",
        &address,
        "--state",
        &state_root.display().to_string(),
    ]);
    assert!(dashboard.contains("server_dashboard=true"));
    assert!(dashboard.contains("agent=codex-local status=running"));
    assert!(dashboard.contains("adapter_kind=codex_exec"));
    assert!(dashboard.contains("evidence_count=1"));
    assert!(dashboard.contains("turn_ids=turn-codex-local"));
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
    assert!(recover.contains("recovered_run_count=1"));

    let status = capo([
        "server",
        "agent",
        "status",
        "--agent",
        "codex-local",
        "--connect",
        &restarted_address,
        "--state",
        &state_root.display().to_string(),
    ]);
    // RTL10: restart recovery reaps the orphaned in-flight run and records a
    // terminal `run.recovered`, so the reconciled run reports `recovered`.
    assert!(status.contains("run_status=recovered"));
    assert!(status.contains("adapter_kind=codex_exec"));
    assert!(status.contains("evidence_count=1"));
    assert!(status.contains("turn_ids=turn-codex-local"));
    assert!(status.contains("tool_calls=1"));
    let state = SqliteStateStore::open(&state_root).expect("state");
    let events = state
        .recent_events_for_session(&SessionId::new("session-codex-local"), 40)
        .expect("session events");
    let replay_audit = events
        .iter()
        .find(|event| {
            event.kind == "server.request_handled"
                && event.payload_json.contains("replay_adapter_fixture")
        })
        .expect("server replay audit survives restart");
    assert!(
        replay_audit
            .payload_json
            .contains("\"provider_cli_executed\":false")
    );
    assert!(
        replay_audit
            .payload_json
            .contains("\"raw_content_policy\":\"content_hashed_not_rendered\"")
    );
    assert!(
        replay_audit
            .payload_json
            .contains("\"raw_fixture_body_persisted\":false")
    );
    assert!(replay_audit.payload_json.contains("\"fixture_hash\":"));
    assert!(
        !replay_audit
            .payload_json
            .contains("Codex fixture response.")
    );
    assert!(restarted.wait().expect("restarted wait").success());
}
