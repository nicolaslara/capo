//! DT1: the three-role configuration and CLI surface (server / runner / client).
//!
//! Capo runs as a DEPLOYMENT TOPOLOGY over its existing single-process
//! boundaries (see `workpads/distributed-topology/knowledge.md`): one
//! authoritative server/controller, zero-or-more remote runners that own agent
//! process lifecycle, and zero-or-more clients that submit commands and tail the
//! event log. This module gives each role an explicit, typed way to start and to
//! point at the others over the tailnet, WITHOUT a second transport: the runner
//! and client both reuse the existing JSON-RPC command transport
//! (`capo_server::send_tcp` / `subscribe_tcp`), and the runner ANNOUNCES itself
//! to the server (DT-D1) via [`capo_server::ServerCommand::RegisterRuntimeTarget`]
//! so the server -- the single authoritative writer -- appends
//! `runtime.target_registered` to the log.
//!
//! Three invariants this module is responsible for:
//!
//! 1. Peers are named by HANDLE, never by an inlined raw address+credential. A
//!    role resolves its peer either by a `connectivity_endpoint_id` (resolved
//!    through [`capo_runtime::ConnectivityTunnel`]) or by an explicit loopback
//!    `--server-endpoint` / `--runner-endpoint` address flag. There is no flag
//!    that accepts an address bundled with a secret.
//! 2. Role configs are validated UP FRONT with a typed [`RoleConfigError`]: a
//!    runner with no server control endpoint, or a client with no server
//!    endpoint, is rejected before any connection is attempted.
//! 3. A `private` / `public` exposure is `blocked_pending_permission` until the
//!    DT5 grant path activates it; only a `loopback` endpoint is reachable with
//!    no grant. This is surfaced via [`ResolvedRolePeer::reachability`], so the
//!    role-start commands refuse to dial an ungranted non-loopback peer rather
//!    than silently failing at connect time.
//!
//! The all-local DEFAULT is structurally untouched: this surface is only entered
//! through the explicit `capo role ...` subcommands. With no role flags the CLI
//! behaves exactly as before (DT6 protects that as an always-on regression).

use capo_runtime::{
    ChannelKind, ConnectivityEndpointConfig, ConnectivityTunnel, EndpointOwner, ExposureScope,
};
use capo_server::{ServerCommand, ServerResponsePayload};

use crate::cli_surface::{ParsedArgs, optional_arg, required_arg};
use crate::runtime_target::parse_runtime_runner_kind;
use crate::server_client::server_role_announce_runtime_target;

/// The three roles a Capo deployment can run, as a closed set. There is no
/// fourth role: every distributed deployment is some composition of these.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RoleKind {
    /// Owns the turn loop, the authoritative event log, and the broadcast hub;
    /// binds a listener (loopback by default).
    Server,
    /// Owns agent process lifecycle behind the `RuntimeRunner` boundary; holds NO
    /// orchestration state; announces itself to the server and reports runtime
    /// events / heartbeat.
    Runner,
    /// Submits commands and tails the event log; holds NO authoritative state.
    Client,
}

impl RoleKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Server => "server",
            Self::Runner => "runner",
            Self::Client => "client",
        }
    }

    pub(crate) fn parse(value: &str) -> Result<Self, RoleConfigError> {
        match value {
            "server" => Ok(Self::Server),
            "runner" => Ok(Self::Runner),
            "client" => Ok(Self::Client),
            other => Err(RoleConfigError::UnknownRole(other.to_string())),
        }
    }
}

/// How a role names one of its peers. Both forms are HANDLES, never a raw
/// address+credential: a `Loopback` address resolves locally (no secret), and an
/// `Endpoint` is a `connectivity_endpoint_id` resolved through the tunnel (the
/// tunnel resolves the handle to a real credential at connect time, never here).
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PeerEndpoint {
    /// An explicit loopback address (e.g. `127.0.0.1:7878`). Used for the
    /// all-local default and deterministic three-process-over-loopback tests.
    Loopback(String),
    /// A `connectivity_endpoint_id` resolved through
    /// [`ConnectivityTunnel::resolve_endpoint`].
    Endpoint(String),
}

impl PeerEndpoint {
    /// Resolve the flags `--<peer>-endpoint` (a connectivity endpoint id) /
    /// `--<peer>-addr` (an explicit loopback address) into a typed peer handle.
    /// Exactly one of the two may be present; an address that carries an inlined
    /// credential (`user:pass@`) is rejected so credentials never enter config.
    fn from_flags(args: &[String], peer: &str) -> Result<Option<Self>, RoleConfigError> {
        let endpoint = optional_arg(args, &format!("--{peer}-endpoint"));
        let addr = optional_arg(args, &format!("--{peer}-addr"));
        match (endpoint, addr) {
            (Some(_), Some(_)) => Err(RoleConfigError::ConflictingPeerFlags {
                peer: peer.to_string(),
            }),
            (Some(endpoint), None) => Ok(Some(Self::Endpoint(endpoint))),
            (None, Some(addr)) => {
                if addr.contains('@') {
                    return Err(RoleConfigError::InlinedCredential {
                        peer: peer.to_string(),
                    });
                }
                Ok(Some(Self::Loopback(addr)))
            }
            (None, None) => Ok(None),
        }
    }
}

/// The reachability verdict for a resolved peer. A non-loopback exposure is
/// `BlockedPendingPermission` until the DT5 grant path activates it; only a
/// loopback peer is reachable with no grant.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PeerReachability {
    /// Loopback endpoint: reachable now, no grant required.
    Reachable,
    /// A `private` / `public` exposure that needs a DT5 grant first.
    BlockedPendingPermission,
}

impl PeerReachability {
    fn as_str(self) -> &'static str {
        match self {
            Self::Reachable => "reachable",
            Self::BlockedPendingPermission => "blocked_pending_permission",
        }
    }
}

/// A peer endpoint after it has been resolved through the tunnel. Carries the
/// resolved URI (loopback or tunnel) plus the exposure-derived reachability
/// verdict. The URI is reachability metadata, never a credential.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ResolvedRolePeer {
    pub(crate) endpoint_ref: String,
    pub(crate) resolved_uri: String,
    pub(crate) exposure: &'static str,
    pub(crate) reachability: PeerReachability,
}

/// A typed, validated three-role configuration. Built by [`RoleConfig::parse`]
/// from the CLI flags for one role; validation rejects a config that names no
/// required peer before any connection is attempted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RoleConfig {
    pub(crate) role: RoleKind,
    /// The server's own bind endpoint (server role) or the server control
    /// endpoint a runner/client points at.
    pub(crate) server_endpoint: Option<PeerEndpoint>,
    /// The runner's own reachability endpoint (runner role), by handle.
    pub(crate) runner_endpoint: Option<PeerEndpoint>,
}

impl RoleConfig {
    /// Parse + validate the role config for `role` from its flags.
    ///
    /// Validation (the DT1 "validated up front" criterion):
    /// - a `runner` MUST name a server control endpoint (it has nothing to
    ///   announce to otherwise);
    /// - a `client` MUST name a server endpoint (it has nothing to tail
    ///   otherwise);
    /// - a `server` MAY name its own bind endpoint (defaults to loopback).
    pub(crate) fn parse(role: RoleKind, args: &[String]) -> Result<Self, RoleConfigError> {
        let server_endpoint = PeerEndpoint::from_flags(args, "server")?;
        let runner_endpoint = PeerEndpoint::from_flags(args, "runner")?;
        let config = Self {
            role,
            server_endpoint,
            runner_endpoint,
        };
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), RoleConfigError> {
        match self.role {
            RoleKind::Runner if self.server_endpoint.is_none() => {
                Err(RoleConfigError::MissingPeer {
                    role: self.role,
                    peer: "server-endpoint",
                })
            }
            RoleKind::Client if self.server_endpoint.is_none() => {
                Err(RoleConfigError::MissingPeer {
                    role: self.role,
                    peer: "server-endpoint",
                })
            }
            _ => Ok(()),
        }
    }
}

/// Typed role-config validation errors. Every variant is raised BEFORE any
/// connection is attempted, so an invalid topology never opens a socket.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RoleConfigError {
    UnknownRole(String),
    MissingPeer { role: RoleKind, peer: &'static str },
    ConflictingPeerFlags { peer: String },
    InlinedCredential { peer: String },
}

impl std::fmt::Display for RoleConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownRole(role) => write!(
                f,
                "unknown role: {role}; expected one of server, runner, client"
            ),
            Self::MissingPeer { role, peer } => write!(
                f,
                "role {} requires --{peer} (a connectivity endpoint id) or --{}-addr (a loopback address)",
                role.as_str(),
                peer.strip_suffix("-endpoint").unwrap_or(peer)
            ),
            Self::ConflictingPeerFlags { peer } => write!(
                f,
                "--{peer}-endpoint and --{peer}-addr are mutually exclusive; name the peer by exactly one handle"
            ),
            Self::InlinedCredential { peer } => write!(
                f,
                "--{peer}-addr must not inline a credential (no `user:pass@`); reference credentials by a connectivity endpoint handle instead"
            ),
        }
    }
}

impl From<RoleConfigError> for String {
    fn from(error: RoleConfigError) -> Self {
        error.to_string()
    }
}

/// Resolve a [`PeerEndpoint`] through a [`ConnectivityTunnel`] into a
/// [`ResolvedRolePeer`], deriving the reachability verdict from the resolved
/// exposure. A loopback endpoint resolves through the loopback tunnel and is
/// reachable; an endpoint id resolves through the provided tunnel and is
/// `blocked_pending_permission` whenever its exposure requires a grant.
///
/// `tunnel` is injected so tests use `ConnectivityTunnel::fake()` /
/// `endpoint_stub(...)` and the live path uses `Tailscale`; the resolution path
/// is identical.
pub(crate) fn resolve_role_peer(
    peer: &PeerEndpoint,
    owner: EndpointOwner,
    channel_kind: ChannelKind,
    tunnel: &ConnectivityTunnel,
) -> Result<ResolvedRolePeer, String> {
    match peer {
        PeerEndpoint::Loopback(addr) => Ok(ResolvedRolePeer {
            endpoint_ref: addr.clone(),
            resolved_uri: format!("tcp://{addr}"),
            exposure: ExposureScope::Loopback.as_str(),
            reachability: PeerReachability::Reachable,
        }),
        PeerEndpoint::Endpoint(endpoint_id) => {
            let resolved = tunnel
                .resolve_endpoint(owner, channel_kind)
                .map_err(|error| format!("endpoint resolution failed: {error}"))?;
            let reachability = if resolved.permission_required {
                PeerReachability::BlockedPendingPermission
            } else {
                PeerReachability::Reachable
            };
            Ok(ResolvedRolePeer {
                endpoint_ref: endpoint_id.clone(),
                resolved_uri: resolved.resolved_uri,
                exposure: resolved.exposure.as_str(),
                reachability,
            })
        }
    }
}

/// Build the resolution tunnel for a peer handle. A loopback peer never needs a
/// tunnel here (it is resolved inline); an endpoint id is resolved through a
/// tunnel built from the endpoint's exposure. Tests can override `exposure` to
/// drive the `blocked_pending_permission` branch deterministically with no live
/// tailnet (mirroring `connectivity.rs`'s `endpoint_stub` usage).
pub(crate) fn role_resolution_tunnel(
    endpoint_id: &str,
    exposure: ExposureScope,
) -> ConnectivityTunnel {
    match exposure {
        ExposureScope::Loopback => ConnectivityTunnel::local_loopback(),
        ExposureScope::Private => ConnectivityTunnel::endpoint_stub(
            ConnectivityEndpointConfig::stub_private(endpoint_id, format!("private:{endpoint_id}")),
        ),
        ExposureScope::Public => ConnectivityTunnel::endpoint_stub(
            ConnectivityEndpointConfig::stub_public(endpoint_id, format!("public:{endpoint_id}")),
        ),
    }
}

// ----------------------------------------------------------------------------
// CLI handlers: `capo role server|runner|client ...`
// ----------------------------------------------------------------------------

/// `capo role server [--server-addr ADDR | --server-endpoint ID] [--exposure
/// loopback|private|public]`
///
/// Resolve (and report) the server's own bind endpoint. The bind itself reuses
/// the existing `server serve` listener (loopback by default); a non-loopback
/// bind requires the DT5 grant path and is reported `blocked_pending_permission`
/// here, NOT silently bound. This command does not start the long-lived
/// listener; it validates + resolves the role config so an operator can confirm
/// the topology before `capo server serve`.
pub(crate) fn role_server(_parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let config = RoleConfig::parse(RoleKind::Server, args)?;
    let exposure = parse_exposure(args)?;
    let peer = config
        .server_endpoint
        .clone()
        .unwrap_or_else(|| PeerEndpoint::Loopback(default_loopback_addr()));
    let resolved = resolve_for(
        &peer,
        exposure,
        EndpointOwner::capo_server("server"),
        ChannelKind::Control,
    )?;
    // DT2: report whether the keep-alive planes would be CONSTRUCTED for this
    // server bind. The bind itself is the runner/client legs' server endpoint, so a
    // non-loopback bind is what makes the two health planes live; a loopback bind
    // keeps them inert (the all-local default). This is reported, not started — the
    // long-lived listener + heartbeat loop are wired by `capo server serve`.
    let keep_alive = keep_alive_config_for(Some(&resolved), Some(&resolved));
    let mut output = String::new();
    output.push_str("role=server\n");
    output.push_str(&render_resolved("server_bind", &resolved));
    output.push_str(&format!(
        "keep_alive_planes={}\n",
        if keep_alive.is_some() {
            "live"
        } else {
            "inert"
        }
    ));
    output.push_str(&format!(
        "all_local_default={}\nnext_action={}\n",
        matches!(peer, PeerEndpoint::Loopback(_)) && exposure == ExposureScope::Loopback,
        match resolved.reachability {
            PeerReachability::Reachable => "capo server serve",
            PeerReachability::BlockedPendingPermission =>
                "capo connectivity expose-stub + request-approval + activate-exposure (DT5)",
        }
    ));
    Ok(output)
}

/// `capo role runner --target ID --name NAME --runner local-process|remote-process|container
/// --workspace PATH --artifacts PATH [--cwd PATH] [--capability-profile PROFILE]
/// [--endpoint RUNNER_ENDPOINT_ID] --server-addr ADDR | --server-endpoint ID
/// [--exposure ...] [--connect ADDR]`
///
/// Validate the runner role config, resolve the server control endpoint, and
/// ANNOUNCE this runtime target to the server over the JSON-RPC transport so the
/// SERVER (single writer) appends `runtime.target_registered` (DT-D1). This is
/// new code, not the legacy local-store write: the runner can be on a different
/// device and still has the server own the log entry.
pub(crate) fn role_runner(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let config = RoleConfig::parse(RoleKind::Runner, args)?;
    let exposure = parse_exposure(args)?;
    let server_peer = config
        .server_endpoint
        .clone()
        .ok_or(RoleConfigError::MissingPeer {
            role: RoleKind::Runner,
            peer: "server-endpoint",
        })?;
    let server_resolved = resolve_for(
        &server_peer,
        exposure,
        EndpointOwner::capo_server("server"),
        ChannelKind::Control,
    )?;

    // A runner cannot announce to a peer it is not permitted to reach. Refuse
    // here (typed, before any socket) rather than failing opaquely at connect.
    if server_resolved.reachability == PeerReachability::BlockedPendingPermission {
        return Err(format!(
            "runner cannot announce: server control endpoint {} is blocked_pending_permission (exposure={}); activate the DT5 grant first",
            server_resolved.endpoint_ref, server_resolved.exposure
        ));
    }

    let runtime_target_id = required_arg(args, "--target")?;
    let name = required_arg(args, "--name")?;
    let runner_kind = parse_runtime_runner_kind(&required_arg(args, "--runner")?)?;
    let workspace_root = required_arg(args, "--workspace")?;
    let artifact_root = required_arg(args, "--artifacts")?;
    let default_cwd = optional_arg(args, "--cwd").unwrap_or_else(|| workspace_root.clone());
    let capability_profile_id =
        optional_arg(args, "--capability-profile").unwrap_or_else(|| "read-only-local".to_string());
    // The runner's own reachability endpoint id, by handle. Prefer an explicit
    // `--endpoint`; otherwise carry the typed `--runner-endpoint` handle.
    let connectivity_endpoint_id = optional_arg(args, "--endpoint").or_else(|| {
        config.runner_endpoint.as_ref().map(|peer| match peer {
            PeerEndpoint::Endpoint(id) => id.clone(),
            PeerEndpoint::Loopback(addr) => addr.clone(),
        })
    });

    let command = ServerCommand::RegisterRuntimeTarget {
        runtime_target_id: runtime_target_id.clone(),
        name: name.clone(),
        runner_kind: runner_kind.clone(),
        workspace_root,
        artifact_root,
        default_cwd,
        capability_profile_id,
        connectivity_endpoint_id: connectivity_endpoint_id.clone(),
        status: "available".to_string(),
    };

    let connect_addr = announce_address(args, &server_resolved)?;
    let response = server_role_announce_runtime_target(parsed, args, &connect_addr, command)?;
    let ServerResponsePayload::RuntimeTargetRegistered(summary) = response else {
        return Err("server returned unexpected response for runner announce".to_string());
    };

    let mut output = String::new();
    output.push_str("role=runner\n");
    output.push_str(&render_resolved("server_control", &server_resolved));
    output.push_str(&format!(
        "runner_announced=true\nannounce_source=runner_jsonrpc\nruntime_target={}\nname={}\nrunner_kind={}\nstatus={}\nrunner_endpoint={}\nappended_by=server\nsequence={}\n",
        summary.runtime_target_id,
        summary.name,
        summary.runner_kind,
        summary.status,
        summary
            .connectivity_endpoint_id
            .as_deref()
            .unwrap_or("none"),
        summary.sequence,
    ));
    Ok(output)
}

/// `capo role client --server-addr ADDR | --server-endpoint ID [--exposure ...]`
///
/// Validate the client role config and resolve the server endpoint into a
/// `subscribe_tcp` tail target. The client holds NO authoritative state; it
/// resolves the endpoint by handle and reports the tail/command target. The
/// actual tail is the existing `capo control` / `subscribe_tcp` path,
/// parameterized by the resolved endpoint rather than hardcoded loopback.
pub(crate) fn role_client(_parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let config = RoleConfig::parse(RoleKind::Client, args)?;
    let exposure = parse_exposure(args)?;
    let server_peer = config
        .server_endpoint
        .clone()
        .ok_or(RoleConfigError::MissingPeer {
            role: RoleKind::Client,
            peer: "server-endpoint",
        })?;
    let resolved = resolve_for(
        &server_peer,
        exposure,
        EndpointOwner::capo_server("server"),
        ChannelKind::Control,
    )?;
    let mut output = String::new();
    output.push_str("role=client\n");
    output.push_str(&render_resolved("server_tail", &resolved));
    output.push_str(&format!(
        "holds_authoritative_state=false\nnext_action={}\n",
        match resolved.reachability {
            PeerReachability::Reachable => "capo control --connect <addr> (subscribe_tcp tail)",
            PeerReachability::BlockedPendingPermission =>
                "activate the DT5 grant before tailing the private endpoint",
        }
    ));
    Ok(output)
}

fn resolve_for(
    peer: &PeerEndpoint,
    exposure: ExposureScope,
    owner: EndpointOwner,
    channel_kind: ChannelKind,
) -> Result<ResolvedRolePeer, String> {
    match peer {
        PeerEndpoint::Loopback(_) => resolve_role_peer(
            peer,
            owner,
            channel_kind,
            &ConnectivityTunnel::local_loopback(),
        ),
        PeerEndpoint::Endpoint(endpoint_id) => {
            let tunnel = role_resolution_tunnel(endpoint_id, exposure);
            resolve_role_peer(peer, owner, channel_kind, &tunnel)
        }
    }
}

/// The address a runner dials to announce. The two flags have DISTINCT, narrow
/// roles (review finding 6):
/// - `--server-addr` / `--server-endpoint` drive endpoint RESOLUTION and the
///   `blocked_pending_permission` reachability check (what the topology points
///   at);
/// - `--connect` is the tunnel-local DIAL address the announce socket actually
///   opens (where the loopback-or-DT5-granted tunnel terminates locally).
///
/// For a loopback peer these coincide, so they MUST NOT diverge: if both
/// `--connect` and a loopback `--server-addr` are present and differ, the config
/// is rejected -- two flags silently describing "the server address" with
/// different values is an operator footgun (and a security seam once DT3/DT5
/// extend this to non-loopback tunnels). For an endpoint handle the resolved URI
/// is not a dialable loopback address, so `--connect` supplies the tunnel-local
/// dial (defaulting to the loopback server address in DT1).
fn announce_address(args: &[String], server: &ResolvedRolePeer) -> Result<String, String> {
    let connect = optional_arg(args, "--connect");
    let is_loopback_peer = server.exposure == ExposureScope::Loopback.as_str();
    match (connect, is_loopback_peer) {
        (Some(addr), true) if addr != server.endpoint_ref => Err(format!(
            "--connect {addr} disagrees with the resolved loopback server endpoint {} -- for a loopback peer the dial address must match the resolved address; pass only one, or use --server-endpoint + --connect for a tunnel-local dial",
            server.endpoint_ref
        )),
        (Some(addr), _) => Ok(addr),
        (None, true) => Ok(server.endpoint_ref.clone()),
        (None, false) => Ok(default_loopback_addr()),
    }
}

fn parse_exposure(args: &[String]) -> Result<ExposureScope, String> {
    match optional_arg(args, "--exposure").as_deref() {
        None | Some("loopback") => Ok(ExposureScope::Loopback),
        Some("private") => Ok(ExposureScope::Private),
        Some("public") => Ok(ExposureScope::Public),
        Some(other) => Err(format!(
            "unsupported exposure: {other}; expected loopback, private, or public"
        )),
    }
}

fn render_resolved(label: &str, resolved: &ResolvedRolePeer) -> String {
    format!(
        "{label}_endpoint={} {label}_uri={} {label}_exposure={} {label}_reachability={}\n",
        resolved.endpoint_ref,
        resolved.resolved_uri,
        resolved.exposure,
        resolved.reachability.as_str()
    )
}

fn default_loopback_addr() -> String {
    crate::server_client::DEFAULT_SERVER_ADDR.to_string()
}

// ----------------------------------------------------------------------------
// DT2: keep-alive gating — derive the two-plane KeepAliveConfig from the resolved
// role peers. The gate IS the resolved exposure: a fully-loopback (all-local)
// deployment yields `None`, so the DT2 heartbeat machinery is never constructed
// (the DT6 inertness guarantee, anchored to the SAME resolution path the role
// commands use, not a second classifier).
// ----------------------------------------------------------------------------

/// Classify a resolved role peer's leg as loopback or non-loopback for DT2 gating.
/// A peer reachable on a loopback exposure is an inert leg; any tunnel-resolved
/// (`private`/`public`) peer is a live leg whose keep-alive plane is constructed.
fn leg_for(resolved: &ResolvedRolePeer) -> capo_runtime::LegEndpoint {
    if resolved.exposure == ExposureScope::Loopback.as_str() {
        capo_runtime::LegEndpoint::classify(&resolved.resolved_uri)
    } else {
        // A private/public exposure is, by definition, a non-loopback leg.
        capo_runtime::LegEndpoint::NonLoopback
    }
}

/// DT2 inertness gate, anchored to the role resolution: build a
/// [`capo_runtime::KeepAliveConfig`] from the resolved runner/client legs, or
/// `None` when the deployment is all-loopback (single box). The all-local default
/// therefore constructs NO keep-alive planes — the DT6 structural inertness
/// guarantee, expressed against the exact resolution the role commands run.
pub(crate) fn keep_alive_config_for(
    runner_leg: Option<&ResolvedRolePeer>,
    client_leg: Option<&ResolvedRolePeer>,
) -> Option<capo_runtime::KeepAliveConfig> {
    capo_runtime::KeepAliveConfig::for_role(
        runner_leg.map(leg_for),
        client_leg.map(leg_for),
        capo_runtime::HeartbeatConfig::default(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flags(pairs: &[&str]) -> Vec<String> {
        pairs.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn server_role_defaults_to_loopback_reachable() {
        let config = RoleConfig::parse(RoleKind::Server, &[]).expect("server config");
        assert_eq!(config.role, RoleKind::Server);
        assert!(config.server_endpoint.is_none());
    }

    #[test]
    fn runner_without_server_endpoint_is_rejected_up_front() {
        let error = RoleConfig::parse(
            RoleKind::Runner,
            &flags(&["--target", "t1", "--name", "n1"]),
        )
        .expect_err("runner must name a server endpoint");
        assert_eq!(
            error,
            RoleConfigError::MissingPeer {
                role: RoleKind::Runner,
                peer: "server-endpoint",
            }
        );
    }

    #[test]
    fn client_without_server_endpoint_is_rejected_up_front() {
        let error = RoleConfig::parse(RoleKind::Client, &[])
            .expect_err("client must name a server endpoint");
        assert_eq!(
            error,
            RoleConfigError::MissingPeer {
                role: RoleKind::Client,
                peer: "server-endpoint",
            }
        );
    }

    #[test]
    fn loopback_server_endpoint_is_accepted_and_reachable() {
        let config = RoleConfig::parse(
            RoleKind::Client,
            &flags(&["--server-addr", "127.0.0.1:7878"]),
        )
        .expect("loopback client config");
        let peer = config.server_endpoint.expect("server peer");
        assert_eq!(peer, PeerEndpoint::Loopback("127.0.0.1:7878".to_string()));
        let resolved = resolve_for(
            &peer,
            ExposureScope::Loopback,
            EndpointOwner::capo_server("server"),
            ChannelKind::Control,
        )
        .expect("resolve loopback");
        assert_eq!(resolved.reachability, PeerReachability::Reachable);
    }

    #[test]
    fn conflicting_peer_flags_are_rejected() {
        let error = RoleConfig::parse(
            RoleKind::Client,
            &flags(&[
                "--server-endpoint",
                "ep-1",
                "--server-addr",
                "127.0.0.1:7878",
            ]),
        )
        .expect_err("conflicting flags");
        assert_eq!(
            error,
            RoleConfigError::ConflictingPeerFlags {
                peer: "server".to_string(),
            }
        );
    }

    #[test]
    fn inlined_credential_address_is_rejected() {
        let error = RoleConfig::parse(
            RoleKind::Client,
            &flags(&["--server-addr", "user:pass@10.0.0.2:7878"]),
        )
        .expect_err("inlined credential");
        assert_eq!(
            error,
            RoleConfigError::InlinedCredential {
                peer: "server".to_string(),
            }
        );
    }

    #[test]
    fn private_endpoint_is_blocked_pending_permission() {
        let config = RoleConfig::parse(
            RoleKind::Client,
            &flags(&["--server-endpoint", "ep-private", "--exposure", "private"]),
        )
        .expect("private client config");
        let peer = config.server_endpoint.expect("server peer");
        assert_eq!(peer, PeerEndpoint::Endpoint("ep-private".to_string()));
        let resolved = resolve_for(
            &peer,
            ExposureScope::Private,
            EndpointOwner::capo_server("server"),
            ChannelKind::Control,
        )
        .expect("resolve private");
        assert_eq!(
            resolved.reachability,
            PeerReachability::BlockedPendingPermission
        );
        assert_eq!(resolved.exposure, ExposureScope::Private.as_str());
    }

    #[test]
    fn public_endpoint_is_blocked_pending_permission() {
        let resolved = resolve_for(
            &PeerEndpoint::Endpoint("ep-public".to_string()),
            ExposureScope::Public,
            EndpointOwner::capo_server("server"),
            // A public exposure fronts the dashboard channel (the stub's only
            // allowed channel); it is still blocked_pending_permission.
            ChannelKind::Dashboard,
        )
        .expect("resolve public");
        assert_eq!(
            resolved.reachability,
            PeerReachability::BlockedPendingPermission
        );
    }

    #[test]
    fn dt2_keep_alive_is_inert_for_all_local_default() {
        // DT6 inertness anchored to the DT1 resolution: a loopback server + loopback
        // runner leg (the single-box default) builds NO keep-alive config, so the DT2
        // heartbeat machinery is never constructed.
        let loopback = resolve_for(
            &PeerEndpoint::Loopback("127.0.0.1:7878".to_string()),
            ExposureScope::Loopback,
            EndpointOwner::capo_server("server"),
            ChannelKind::Control,
        )
        .expect("resolve loopback");
        let config = keep_alive_config_for(Some(&loopback), Some(&loopback));
        assert!(
            config.is_none(),
            "the all-local default must construct no keep-alive plane"
        );
    }

    #[test]
    fn dt2_keep_alive_is_live_for_non_loopback_runner_leg() {
        // A private (tunnel-resolved) runner leg makes the keep-alive plane LIVE; the
        // loopback client leg stays inert within the same distributed deployment.
        let runner_leg = resolve_for(
            &PeerEndpoint::Endpoint("ep-private".to_string()),
            ExposureScope::Private,
            EndpointOwner::capo_server("server"),
            ChannelKind::Control,
        )
        .expect("resolve private runner leg");
        let client_leg = resolve_for(
            &PeerEndpoint::Loopback("127.0.0.1:7878".to_string()),
            ExposureScope::Loopback,
            EndpointOwner::capo_server("server"),
            ChannelKind::Control,
        )
        .expect("resolve loopback client leg");
        let config = keep_alive_config_for(Some(&runner_leg), Some(&client_leg))
            .expect("a non-loopback runner leg builds a keep-alive config");
        assert_eq!(config.runner_leg, capo_runtime::LegEndpoint::NonLoopback);
        assert_eq!(config.client_leg, capo_runtime::LegEndpoint::Loopback);
    }

    #[test]
    fn unknown_role_is_rejected() {
        assert_eq!(
            RoleKind::parse("orchestrator"),
            Err(RoleConfigError::UnknownRole("orchestrator".to_string()))
        );
    }
}
