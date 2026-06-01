use std::io::{BufRead, Read};
use std::net::{TcpListener, ToSocketAddrs};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use crate::cli_surface::ParsedArgs;
use crate::debug_error;
use crate::server_client::DEFAULT_SERVER_ADDR;

/// How long autostart keeps trying to claim an env/default address before it
/// concludes a real server already owns the port and connects instead. This
/// rides out a port that is only transiently busy (a peer loopback server under
/// parallel tests, or one lingering in `TIME_WAIT`) while staying short enough
/// not to delay connecting to a genuinely running server.
const SERVER_BIND_DEADLINE: Duration = Duration::from_secs(2);
/// Pause between bind attempts while a transiently held port clears.
const SERVER_BIND_RETRY_INTERVAL: Duration = Duration::from_millis(20);

/// The resolved server address plus how it was chosen. `explicit` is `true` only
/// when the operator pointed us at an already-running server with `--connect`;
/// in that case we never autostart and never probe (which would consume a
/// budgeted request). Otherwise the address comes from `CAPO_SERVER_ADDR`/the
/// default and we own its lifecycle.
pub(super) struct ServerAddress {
    pub(super) address: String,
    pub(super) explicit: bool,
}

pub(super) fn server_address(args: &[String]) -> Result<ServerAddress, String> {
    if let Some(address) = optional_value(args, "--connect")? {
        return Ok(ServerAddress {
            address,
            explicit: true,
        });
    }
    let address = std::env::var("CAPO_SERVER_ADDR")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_SERVER_ADDR.to_string());
    Ok(ServerAddress {
        address,
        explicit: false,
    })
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
    // Held only so the server's stdout pipe stays open for the session; if it
    // were dropped the child could block (or be signalled) on a full pipe.
    _stdout: std::io::BufReader<std::process::ChildStdout>,
}

impl Drop for AutoServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub(super) fn ensure_server_running(
    address: &ServerAddress,
    parsed: &ParsedArgs,
) -> Result<Option<AutoServer>, String> {
    // With an explicit `--connect`, the operator owns the target server: never
    // autostart, and never probe it (a probe would consume a budgeted request).
    if address.explicit {
        return Ok(None);
    }
    let address = address.address.as_str();
    // We own this env/default address, so prefer to autostart our own server.
    // A bound port may be a genuinely running server (which stays bound) or a
    // peer that is about to free it (a short-lived loopback test server, or a
    // socket lingering in `TIME_WAIT`). Retry the bind until the deadline:
    //
    // - free now  -> spawn our own server,
    // - frees later -> spawn once it clears,
    // - stays bound past the deadline -> connect to the already running server.
    //
    // A bare bind probe never consumes a `--max-requests` budget, unlike a
    // protocol round-trip, so this stays safe even against budgeted servers.
    let deadline = Instant::now() + SERVER_BIND_DEADLINE;
    loop {
        if server_port_is_bound(address)? {
            if Instant::now() >= deadline {
                return Ok(None);
            }
            std::thread::sleep(SERVER_BIND_RETRY_INTERVAL);
            continue;
        }
        match try_spawn_server(address, parsed)? {
            SpawnOutcome::Started(server) => return Ok(Some(server)),
            SpawnOutcome::AddressBusy if Instant::now() < deadline => {
                // A peer grabbed the port between our probe and the child bind.
                std::thread::sleep(SERVER_BIND_RETRY_INTERVAL);
            }
            SpawnOutcome::AddressBusy => {
                if server_port_is_bound(address)? {
                    return Ok(None);
                }
                return Err(format!(
                    "failed to start Capo server at {address}: address stayed in use"
                ));
            }
        }
    }
}

enum SpawnOutcome {
    Started(AutoServer),
    AddressBusy,
}

fn try_spawn_server(address: &str, parsed: &ParsedArgs) -> Result<SpawnOutcome, String> {
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
        let stderr = stderr.trim();
        if address_in_use_error(stderr) {
            return Ok(SpawnOutcome::AddressBusy);
        }
        return Err(format!(
            "failed to start Capo server at {address}: {stderr}"
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
    // Keep the stdout pipe attached so the child never blocks on a full,
    // abandoned pipe buffer during the session.
    Ok(SpawnOutcome::Started(AutoServer {
        child,
        _stdout: reader,
    }))
}

fn address_in_use_error(stderr: &str) -> bool {
    let stderr = stderr.to_ascii_lowercase();
    stderr.contains("address in use")
        || stderr.contains("addrinuse")
        || stderr.contains("address already in use")
}

/// Returns `true` when the loopback `address` is already bound. This is a pure
/// bind probe, so unlike a protocol round-trip it never consumes a server's
/// `--max-requests` budget.
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
