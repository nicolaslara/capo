use std::io::{BufReader, Read};

use super::support::*;

#[test]
fn cli_talks_to_running_server_process_over_loopback_transport() {
    let state_root = temp_root("transport-state");
    let mut server = spawn_server(&state_root, 4);
    let stdout = server.stdout.take().expect("server stdout");
    let mut reader = BufReader::new(stdout);
    let address = read_server_address(&mut reader);

    let register = capo([
        "server",
        "agent",
        "register",
        "--name",
        "mock-codex",
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
    assert!(register.contains("server_boundary=capo-server"));

    let send = capo([
        "server",
        "task",
        "send",
        "--agent",
        "mock-codex",
        "--goal",
        "Prove process transport",
        "--connect",
        &address,
        "--state",
        &state_root.display().to_string(),
    ]);
    assert!(send.contains("server_task_sent=true"));
    assert!(send.contains("session_id=session-mock-codex"));

    let status = capo([
        "server",
        "agent",
        "status",
        "--agent",
        "mock-codex",
        "--connect",
        &address,
        "--state",
        &state_root.display().to_string(),
    ]);
    assert!(status.contains("run_status=running"));
    assert!(status.contains("tool_calls=1"));

    let dashboard = capo([
        "server",
        "dashboard",
        "--connect",
        &address,
        "--state",
        &state_root.display().to_string(),
    ]);
    assert!(dashboard.contains("server_dashboard=true"));
    assert!(dashboard.contains("agent_count=1"));
    assert!(dashboard.contains("active_session_count=1"));

    let status = server.wait().expect("server wait");
    assert!(status.success(), "server failed: {status}");
    let mut rest = String::new();
    reader.read_to_string(&mut rest).expect("read server rest");
    assert!(rest.contains("server_stopped=true"));
    assert!(rest.contains("requests_served=4"));

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

    let restarted_status = capo([
        "server",
        "agent",
        "status",
        "--agent",
        "mock-codex",
        "--connect",
        &restarted_address,
        "--state",
        &state_root.display().to_string(),
    ]);
    assert!(restarted_status.contains("run_status=exited_unknown"));
    assert!(restarted.wait().expect("restarted wait").success());
}

#[test]
fn cli_uses_running_server_from_default_address_env_without_connect_flags() {
    let state_root = temp_root("transport-default-address-state");
    let mut server = spawn_server(&state_root, 3);
    let stdout = server.stdout.take().expect("server stdout");
    let mut reader = BufReader::new(stdout);
    let address = read_server_address(&mut reader);
    let state = state_root.display().to_string();

    let register = capo_with_env(
        [
            "server",
            "agent",
            "register",
            "--name",
            "mock-human",
            "--state",
            &state,
        ],
        [("CAPO_SERVER_ADDR", address.as_str())],
    );
    assert!(register.contains("server_agent_registered=true"));

    let send = capo_with_env(
        [
            "server",
            "task",
            "send",
            "--agent",
            "mock-human",
            "--goal",
            "Use the normal server commands",
            "--state",
            &state,
        ],
        [("CAPO_SERVER_ADDR", address.as_str())],
    );
    assert!(send.contains("server_task_sent=true"));

    let dashboard = capo_with_env(
        ["server", "dashboard", "--state", &state],
        [("CAPO_SERVER_ADDR", address.as_str())],
    );
    assert!(dashboard.contains("server_dashboard=true"));
    assert!(dashboard.contains("agent_count=1"));
    assert!(server.wait().expect("server wait").success());
}

#[test]
fn control_repl_lists_attaches_and_steers_mock_agent_over_server() {
    let state_root = temp_root("control-repl-state");
    let mut server = spawn_server(&state_root, 10);
    let stdout = server.stdout.take().expect("server stdout");
    let mut reader = BufReader::new(stdout);
    let address = read_server_address(&mut reader);
    let state = state_root.display().to_string();

    let register = capo([
        "server",
        "agent",
        "register",
        "--name",
        "mock-control",
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
        "mock-control",
        "--goal",
        "Start under server control",
        "--connect",
        &address,
        "--state",
        &state,
    ]);
    assert!(send.contains("server_task_sent=true"));

    let script = "\
agents
attach mock-control
agents
status
Please report current status
result
dashboard
quit
";
    let output = capo_with_env_and_stdin(
        [
            "control",
            "--planner",
            "none",
            "--connect",
            &address,
            "--state",
            &state,
        ],
        [],
        script,
    );
    assert!(output.contains("Capo"));
    assert!(output.contains("Agents (1)"));
    assert!(output.contains("Attached to mock-control."));
    assert!(output.contains("- mock-control (attached) - running"));
    assert!(output.contains("Sent to mock-control"));
    assert!(output.contains("goal: Please report current status"));
    assert!(output.contains(
        "reply: Fake adapter processed goal for mock-control: Please report current status"
    ));
    assert!(output.contains("Dashboard"));
    assert!(output.contains("active: 1"));

    assert!(server.wait().expect("server wait").success());
}

#[test]
fn bare_capo_starts_control_and_autostarts_server_when_needed() {
    let state_root = temp_root("control-autostart-state");
    let state = state_root.display().to_string();
    let address = unused_loopback_address();
    let output = capo_with_env_and_stdin(
        ["--state", &state],
        [("CAPO_SERVER_ADDR", address.as_str())],
        "dashboard\nquit\n",
    );

    assert!(output.contains("Capo"));
    assert!(output.contains("Dashboard"));
    assert!(output.contains("agents: 0"));
    assert!(output.contains("bye"));
}

#[test]
fn control_repl_reports_richer_agent_state_and_interrupts_or_stops_agents() {
    let state_root = temp_root("control-rich-state");
    let mut server = spawn_server(&state_root, 12);
    let stdout = server.stdout.take().expect("server stdout");
    let mut reader = BufReader::new(stdout);
    let address = read_server_address(&mut reader);
    let state = state_root.display().to_string();

    for name in ["mock-interrupt", "mock-stop"] {
        let register = capo([
            "server",
            "agent",
            "register",
            "--name",
            name,
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
            name,
            "--goal",
            "Start rich control test",
            "--connect",
            &address,
            "--state",
            &state,
        ]);
        assert!(send.contains("server_task_sent=true"));
    }

    let script = "\
attach mock-interrupt
recent
tools
evidence
reviews
interrupt operator needs to inspect state
attach mock-stop
stop operator completed the task
quit
";
    let output = capo_with_env_and_stdin(
        [
            "control",
            "--planner",
            "none",
            "--connect",
            &address,
            "--state",
            &state,
        ],
        [],
        script,
    );
    assert!(output.contains("mock-interrupt"));
    assert!(output.contains("Tool activity"));
    assert!(output.contains("Evidence"));
    assert!(output.contains("Reviews"));
    assert!(output.contains("interrupted mock-interrupt"));
    assert!(output.contains("stopped mock-stop"));

    assert!(server.wait().expect("server wait").success());
}

#[test]
fn capo_planner_tracks_decisions_as_server_state_and_steers_mock_agent() {
    let state_root = temp_root("control-capo-planner-state");
    let mut server = spawn_server(&state_root, 26);
    let stdout = server.stdout.take().expect("server stdout");
    let mut reader = BufReader::new(stdout);
    let address = read_server_address(&mut reader);
    let state = state_root.display().to_string();

    let register = capo([
        "server",
        "agent",
        "register",
        "--name",
        "mock-control",
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
        "mock-control",
        "--goal",
        "Start Capo planner test",
        "--connect",
        &address,
        "--state",
        &state,
    ]);
    assert!(send.contains("server_task_sent=true"));

    let script = "\
what are my agents doing?
what's up?
status of mock-control
tell mock-control to Please continue under planner control
what is blocked?
summarize mock-control
recent capo-operator
quit
";
    let output = capo_with_env_and_stdin(
        [
            "control",
            "--planner",
            "capo",
            "--connect",
            &address,
            "--state",
            &state,
        ],
        [(
            "CAPO_CONTROL_PLANNER_MOCK_CODEX_JSONL",
            r#"{"type":"item.completed","item":{"id":"planner-item-1","type":"agent_message","text":"{\"action\":\"dashboard\",\"summary\":\"operator asked a casual status question\"}"}}
{"type":"turn.completed"}
"#,
        )],
        script,
    );
    assert!(output.contains("Capo"));
    assert!(output.contains("Dashboard"));
    assert!(output.contains("agents: 2"));
    assert!(output.contains("- capo-operator"));
    assert!(output.contains("Status\n- mock-control"));
    assert!(output.contains("Sent to mock-control"));
    assert!(output.contains("Reviews"));
    assert!(output.contains("capo-operator"));
    assert!(output.contains("capo planner decision input_hash="));

    assert!(server.wait().expect("server wait").success());
}

#[test]
fn control_repl_refuses_to_fake_codex_attached_chat_without_live_opt_in() {
    let state_root = temp_root("control-codex-no-opt-in-state");
    let mut server = spawn_server(&state_root, 4);
    let stdout = server.stdout.take().expect("server stdout");
    let mut reader = BufReader::new(stdout);
    let address = read_server_address(&mut reader);
    let state = state_root.display().to_string();

    let register = capo([
        "server",
        "agent",
        "register",
        "--name",
        "codex-control",
        "--connect",
        &address,
        "--state",
        &state,
    ]);
    assert!(register.contains("server_agent_registered=true"));

    let start = capo([
        "server",
        "session",
        "start",
        "--agent",
        "codex-control",
        "--adapter",
        "codex",
        "--goal",
        "Initial Codex control test",
        "--session",
        "session-codex-control",
        "--run",
        "run-codex-control",
        "--connect",
        &address,
        "--state",
        &state,
    ]);
    assert!(start.contains("server_session_started=true"));

    let output = capo_with_env_and_stdin(
        ["control", "--connect", &address, "--state", &state],
        [],
        "attach codex-control\nsay hi\nquit\n",
    );
    assert!(output.contains("Attached to codex-control."));
    assert!(output.contains("Codex live execution from control requires"));
    assert!(!output.contains("Fake adapter processed goal for codex-control: say hi"));

    assert!(server.wait().expect("server wait").success());
}
