use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

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

fn spawn_server(state_root: &Path, max_requests: usize) -> Child {
    Command::new(capo_bin())
        .args([
            "server",
            "serve",
            "--addr",
            "127.0.0.1:0",
            "--max-requests",
            &max_requests.to_string(),
            "--state",
            &state_root.display().to_string(),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn capo server")
}

fn read_server_address(reader: &mut BufReader<std::process::ChildStdout>) -> String {
    let mut first = String::new();
    reader.read_line(&mut first).expect("read listening line");
    assert_eq!(first.trim(), "server_listening=true");
    let mut second = String::new();
    reader.read_line(&mut second).expect("read address line");
    second
        .trim()
        .strip_prefix("server_addr=")
        .expect("server address")
        .to_string()
}

fn capo<const N: usize>(args: [&str; N]) -> String {
    let output = Command::new(capo_bin())
        .args(args)
        .output()
        .expect("run capo");
    assert!(
        output.status.success(),
        "capo failed: status={} stdout={} stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("utf8 stdout")
}

fn capo_bin() -> &'static str {
    env!("CARGO_BIN_EXE_capo")
}

fn temp_root(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    std::env::temp_dir().join(format!("capo-{name}-{nanos}"))
}
