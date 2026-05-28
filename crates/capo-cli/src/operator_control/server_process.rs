use std::io::{BufRead, Read};
use std::net::{TcpListener, ToSocketAddrs};
use std::process::{Child, Command, Stdio};

use crate::cli_surface::ParsedArgs;
use crate::debug_error;
use crate::server_client::DEFAULT_SERVER_ADDR;

pub(super) fn server_address(args: &[String]) -> Result<String, String> {
    optional_value(args, "--connect")?
        .or_else(|| {
            std::env::var("CAPO_SERVER_ADDR")
                .ok()
                .filter(|value| !value.trim().is_empty())
        })
        .or_else(|| Some(DEFAULT_SERVER_ADDR.to_string()))
        .ok_or_else(|| "missing server address".to_string())
}

pub(super) fn require_loopback_address(address: &str) -> Result<(), String> {
    let resolved = address
        .to_socket_addrs()
        .map_err(debug_error)?
        .collect::<Vec<_>>();
    if resolved.is_empty() {
        return Err(format!("server address did not resolve: {address}"));
    }
    if !resolved.iter().all(|address| address.ip().is_loopback()) {
        return Err(format!(
            "server address must resolve only to loopback addresses, got {address}"
        ));
    }
    Ok(())
}

pub(super) struct AutoServer {
    child: Child,
}

impl Drop for AutoServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub(super) fn ensure_server_running(
    address: &str,
    parsed: &ParsedArgs,
) -> Result<Option<AutoServer>, String> {
    if server_port_is_bound(address)? {
        return Ok(None);
    }
    let mut child = Command::new(std::env::current_exe().map_err(debug_error)?)
        .args([
            "server",
            "serve",
            "--addr",
            address,
            "--state",
            &parsed.state_root.display().to_string(),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(debug_error)?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "failed to capture started server stdout".to_string())?;
    let mut reader = std::io::BufReader::new(stdout);
    let mut first = String::new();
    reader.read_line(&mut first).map_err(debug_error)?;
    let mut second = String::new();
    reader.read_line(&mut second).map_err(debug_error)?;
    if first.trim() != "server_listening=true" {
        let mut stderr = String::new();
        if let Some(mut child_stderr) = child.stderr.take() {
            let _ = child_stderr.read_to_string(&mut stderr);
        }
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!(
            "failed to start Capo server at {address}: {}",
            stderr.trim()
        ));
    }
    let reported = second
        .trim()
        .strip_prefix("server_addr=")
        .ok_or_else(|| "started server did not report server_addr".to_string())?;
    if reported != address {
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!(
            "started server reported unexpected address: {reported}; expected {address}"
        ));
    }
    Ok(Some(AutoServer { child }))
}

fn server_port_is_bound(address: &str) -> Result<bool, String> {
    match TcpListener::bind(address) {
        Ok(listener) => {
            drop(listener);
            Ok(false)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => Ok(true),
        Err(error) => Err(debug_error(error)),
    }
}

fn optional_value(args: &[String], key: &str) -> Result<Option<String>, String> {
    let Some(index) = args.iter().position(|arg| arg == key) else {
        return Ok(None);
    };
    let Some(value) = args.get(index + 1) else {
        return Err(format!("{key} requires a value"));
    };
    if value.starts_with("--") {
        return Err(format!("{key} requires a value"));
    }
    Ok(Some(value.clone()))
}
