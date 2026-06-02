//! DT3: remote runner attach over the tunnel.
//!
//! This module is the DT1/DT3 SEAM the substrate update names: it wires the
//! server's runtime endpoint resolution through a [`ConnectivityTunnel`] and binds
//! the resulting reachability channel to a [`RemoteProcessRunner`], so the server
//! drives an agent process on a REMOTE runner device through the EXISTING
//! `RuntimeRunner` boundary — no loop change, no new transport protocol.
//!
//! It deliberately does NOT reimplement the RR8 remote runner: it COMPOSES the
//! in-tree boundaries. RR8's [`SshRemoteProcessRunner`] resolved its target via a
//! DIRECT `SshRemoteConfig` (host/key); DT3 resolves it instead via the tunnel so
//! reachability stays SEPARATE from execution (the connectivity boundary resolves +
//! opens the channel; the runner owns the process group). The transport itself is
//! injected by the caller so the deterministic suite binds a `FakeRemoteChannel`
//! (NO network) while the live DT7 smoke binds the real `SshRemoteChannel`.
//!
//! Safety boundary (DT3): the runner-side redaction-before-transit pass already
//! lives in [`RemoteProcessRunner::stream_output`] (it redacts each delta BEFORE it
//! becomes an event/artifact, on the leg Capo controls); this seam only resolves
//! reachability and never carries a raw credential — identity is the derived
//! channel fingerprint (a HANDLE), never a key/token.

use crate::{
    ChannelKind, ConnectivityResult, ConnectivityTunnel, EndpointOwner, OpenChannel, RemoteChannel,
    RemoteProcessConfig, RemoteProcessRunner,
};

/// DT3: the resolved attach of a remote runner over the tunnel.
///
/// Carries the [`RemoteProcessRunner`] the server drives plus the [`OpenChannel`]
/// reachability handle (so a later revoke / teardown can be matched to this attach,
/// per the CT7 close-channel discipline). The channel handle carries NO secret —
/// its identity is the derived fingerprint.
#[derive(Clone, Debug)]
pub struct RemoteRunnerAttach {
    runner: RemoteProcessRunner,
    channel: OpenChannel,
}

impl RemoteRunnerAttach {
    /// Resolve the runner's runtime endpoint over `tunnel` and bind a
    /// [`RemoteProcessRunner`] to the opened reachability channel.
    ///
    /// `owner` names the runtime target this attach is for; `channel_kind` is the
    /// execution channel (`Stdio` for an agent process). `build_transport` receives
    /// the OPENED channel (identity = its fingerprint) and returns the
    /// [`RemoteChannel`] transport bound to it — the deterministic suite returns a
    /// `FakeRemoteChannel`, the live DT7 path an `SshRemoteChannel`. Resolution and
    /// channel-open happen on the CONNECTIVITY boundary; the runner that comes back
    /// owns ONLY the process group (the one-way coupling `runtime-tunnel.md` pins).
    ///
    /// A resolution that requires permission (a `private`/`public` exposure with no
    /// active grant) propagates the tunnel's typed error here, so the attach is
    /// `blocked_pending_permission` until the DT5 grant — it never silently opens a
    /// non-loopback channel.
    pub fn resolve<F>(
        tunnel: &ConnectivityTunnel,
        owner: EndpointOwner,
        channel_kind: ChannelKind,
        build_transport: F,
    ) -> ConnectivityResult<Self>
    where
        F: FnOnce(&OpenChannel) -> RemoteChannel,
    {
        let resolved = tunnel.resolve_endpoint(owner, channel_kind)?;
        let channel = tunnel.open_channel(&resolved)?;
        let transport = build_transport(&channel);
        let runner = RemoteProcessRunner::new(RemoteProcessConfig::with_transport(
            channel.clone(),
            transport,
        ));
        Ok(Self { runner, channel })
    }

    /// The remote runner the server drives. Borrowed so the caller dispatches the
    /// existing `start_process` / `stream_output` / `interrupt` / `terminate` /
    /// `recover_orphan` surface unchanged — DT3 adds no new control verbs.
    pub fn runner(&self) -> &RemoteProcessRunner {
        &self.runner
    }

    /// The opened reachability channel handle (HANDLE only, no secret), so a CT7
    /// teardown / DT5 revoke can be matched to this attach.
    pub fn channel(&self) -> &OpenChannel {
        &self.channel
    }

    /// HONESTY: whether this attach rode a LOOPBACK/fake transport (the
    /// deterministic suite) rather than crossing a real machine boundary. A
    /// realness guard reads this, never a bare flag.
    pub fn is_loopback(&self) -> bool {
        self.runner.is_loopback()
    }
}
