use std::io::Write;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) fn spawn_server(state_root: &Path, max_requests: usize) -> Child {
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
