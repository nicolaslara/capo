use std::io::Write;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) fn spawn_server(state_root: &Path, max_requests: usize) -> Child {
    spawn_server_with_env(state_root, max_requests, &[])
}

/// Spawn `capo server serve` with extra environment variables, so a test can open
/// the server with the Codex live-chat gate (and an absolute `CAPO_CODEX_BIN`
/// stub) set in the SERVER process -- the gate/codex-bin are read by the server,
/// not the client.
pub(crate) fn spawn_server_with_env(
    state_root: &Path,
    max_requests: usize,
    envs: &[(&str, &str)],
) -> Child {
    let mut command = Command::new(capo_bin());
    command.args([
        "server",
        "serve",
        "--addr",
        "127.0.0.1:0",
        "--max-requests",
        &max_requests.to_string(),
        "--state",
        &state_root.display().to_string(),
    ]);
    for (key, value) in envs {
        command.env(key, value);
    }
    command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn capo server")
}

/// Write an executable absolute-path `codex` stub that streams `fixture_jsonl` to
/// stdout using only POSIX builtins (the runtime spawns with `env_clear()`).
/// Returns the absolute stub path.
#[cfg(unix)]
pub(crate) fn write_codex_stub(dir: &Path, fixture_jsonl: &str) -> String {
    use std::os::unix::fs::PermissionsExt;

    std::fs::create_dir_all(dir).expect("stub dir");
    let fixture = dir.join("codex-output.jsonl");
    std::fs::write(&fixture, fixture_jsonl).expect("write fixture");
    let stub = dir.join("codex-stub.sh");
    let script = format!(
        "#!/bin/sh\nwhile IFS= read -r line; do printf '%s\\n' \"$line\"; done < '{}'\n",
        fixture.display()
    );
    std::fs::write(&stub, &script).expect("write stub");
    let mut perms = std::fs::metadata(&stub).expect("stub meta").permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&stub, perms).expect("chmod stub");
    stub.to_string_lossy().to_string()
}

/// Write an executable absolute-path `claude` stub that streams a Claude
/// `stream-json` turn (a `system` start, an `assistant` message carrying
/// `assistant_text`, a terminal `result`) to stdout using only POSIX builtins (the
/// runtime spawns with `env_clear()`). Returns the absolute stub path. Mirrors
/// `write_codex_stub` for the Claude chat seam (CS6 blocker-2 CLI E2E).
#[cfg(unix)]
pub(crate) fn write_claude_chat_stub(dir: &Path, assistant_text: &str) -> String {
    use std::os::unix::fs::PermissionsExt;

    std::fs::create_dir_all(dir).expect("stub dir");
    let fixture = dir.join("claude-output.jsonl");
    let fixture_jsonl = format!(
        "{{\"type\":\"system\",\"session_id\":\"cli-claude-sess\"}}\n\
{{\"type\":\"assistant\",\"session_id\":\"cli-claude-sess\",\"message\":{{\"id\":\"msg-1\",\"role\":\"assistant\",\"content\":[{{\"type\":\"text\",\"text\":\"{assistant_text}\"}}]}}}}\n\
{{\"type\":\"result\",\"session_id\":\"cli-claude-sess\",\"subtype\":\"success\"}}\n"
    );
    std::fs::write(&fixture, fixture_jsonl).expect("write fixture");
    let stub = dir.join("claude-stub.sh");
    let script = format!(
        "#!/bin/sh\nwhile IFS= read -r line; do printf '%s\\n' \"$line\"; done < '{}'\n",
        fixture.display()
    );
    std::fs::write(&stub, &script).expect("write stub");
    let mut perms = std::fs::metadata(&stub).expect("stub meta").permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&stub, perms).expect("chmod stub");
    stub.to_string_lossy().to_string()
}

pub(crate) fn read_server_address(reader: &mut BufReader<std::process::ChildStdout>) -> String {
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

pub(crate) fn output_value(output: &str, key: &str) -> String {
    output
        .lines()
        .find_map(|line| line.strip_prefix(&format!("{key}=")))
        .unwrap_or_else(|| panic!("missing {key} in output:\n{output}"))
        .to_string()
}

pub(crate) fn capo<const N: usize>(args: [&str; N]) -> String {
    capo_with_env(args, [])
}

pub(crate) fn capo_with_env<const N: usize, const M: usize>(
    args: [&str; N],
    envs: [(&str, &str); M],
) -> String {
    let mut command = Command::new(capo_bin());
    command.args(args);
    for (key, value) in envs {
        command.env(key, value);
    }
    let output = command.output().expect("run capo");
    assert!(
        output.status.success(),
        "capo failed: status={} stdout={} stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("utf8 stdout")
}

pub(crate) fn capo_with_env_and_stdin<const N: usize, const M: usize>(
    args: [&str; N],
    envs: [(&str, &str); M],
    stdin: &str,
) -> String {
    let mut command = Command::new(capo_bin());
    command.args(args);
    for (key, value) in envs {
        command.env(key, value);
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run capo");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(stdin.as_bytes())
        .expect("write stdin");
    let output = child.wait_with_output().expect("capo output");
    assert!(
        output.status.success(),
        "capo failed: status={} stdout={} stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("utf8 stdout")
}

/// Run `capo` expecting a NON-zero exit, returning combined stdout+stderr. Used
/// by the DT1 role-config validation tests where a missing/invalid role config
/// must be rejected before any connection is attempted.
pub(crate) fn capo_failure<const N: usize>(args: [&str; N]) -> String {
    let output = Command::new(capo_bin())
        .args(args)
        .output()
        .expect("run capo");
    assert!(
        !output.status.success(),
        "expected capo to fail but it succeeded: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
    combined.push_str(&String::from_utf8_lossy(&output.stderr));
    combined
}

pub(crate) fn capo_bin() -> &'static str {
    env!("CARGO_BIN_EXE_capo")
}

pub(crate) fn temp_root(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    std::env::temp_dir().join(format!("capo-{name}-{nanos}"))
}

pub(crate) fn unused_loopback_address() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind unused address");
    listener.local_addr().expect("local address").to_string()
}
