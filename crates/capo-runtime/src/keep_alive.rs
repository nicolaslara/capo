//! DT2: keep-alive across the WHOLE distributed path, on TWO SEPARATE health
//! planes.
//!
//! Capo's distributed deployment (`workpads/distributed-topology/knowledge.md`)
//! has two connectivity legs, and they are NOT the same kind of signal:
//!
//! - **runner <-> server is LOGGED.** Runner liveness affects PROCESS TRUTH (the
//!   server drives agent process lifecycle on the runner), so a missed runner
//!   heartbeat is a legitimately auditable `connectivity.health_changed` event,
//!   and a recovered runner heartbeat re-runs the `runtime-tunnel.md` recovery
//!   sequence (`recover_run` -> `run.recovered` / `run.orphaned` / `run.exited` /
//!   `recovery_pending`). This plane is [`RunnerServerPlane`].
//! - **client <-> server is EPHEMERAL.** A missed client heartbeat is pure
//!   connectivity jitter — it MUST NOT write into the authoritative log, or a
//!   flaky client would spam the truth stream and DT6's byte-for-byte regression
//!   would break. This plane is [`ClientServerPlane`]: an in-memory, server-side
//!   connection state observable via a status query, never an event.
//!
//! Both planes share the CT5 [`crate::connectivity_health::HeartbeatMonitor`] for
//! the deterministic, fake-clock-driven liveness probe — DT2 does NOT re-implement
//! the heartbeat loop. It composes the existing monitor into the two-plane policy,
//! adds the runner-leg event mapping + recovery re-run, and the client-leg
//! ephemeral state machine.
//!
//! ## Inertness in the all-local default (DT6)
//!
//! [`HealthPlanes`] is the gated container. It is constructed ONLY from a
//! [`KeepAliveConfig`] whose endpoints are NON-loopback ([`KeepAliveConfig::for_role`]
//! returns `None` for a fully-loopback / single-box deployment). In the all-local
//! default NEITHER plane is instantiated, so DT2 adds NO events and NO frames: the
//! machinery is structurally unreachable without a non-loopback endpoint in scope.
//! DT6 asserts this two ways (byte-identical snapshots + this structural gate).
//!
//! ## Boundary
//!
//! This module depends ONLY on the [`crate::connectivity_health`] surface and the
//! [`crate::RemoteRunRecovery`] recovery type. It never reads or mutates
//! controller / turn / run read-model state, never owns a process handle, and
//! never logs a credential — heartbeat payloads carry only liveness + handles.

use crate::connectivity_health::{
    HealthTransition, HeartbeatConfig, HeartbeatMonitor, HeartbeatOutcome,
};

/// DT2: whether a configured keep-alive endpoint is loopback (single-box, INERT)
/// or non-loopback (distributed, the planes are LIVE). This is the structural gate
/// that keeps the all-local default free of the heartbeat machinery.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LegEndpoint {
    /// A loopback / same-box leg. The keep-alive plane is NOT constructed for it.
    Loopback,
    /// A non-loopback leg over the tunnel. The keep-alive plane IS constructed.
    NonLoopback,
}

impl LegEndpoint {
    /// Classify a resolved endpoint URI as loopback or non-loopback. A
    /// `tcp://127.0.0.1:..` / `tcp://[::1]:..` / `tcp://localhost:..` is loopback;
    /// anything resolved over the tunnel is not.
    pub fn classify(resolved_uri: &str) -> Self {
        let host = resolved_uri
            .strip_prefix("tcp://")
            .unwrap_or(resolved_uri)
            .rsplit_once(':')
            .map(|(host, _port)| host)
            .unwrap_or(resolved_uri);
        let host = host.trim_start_matches('[').trim_end_matches(']');
        if host == "127.0.0.1" || host == "::1" || host == "localhost" {
            Self::Loopback
        } else {
            Self::NonLoopback
        }
    }

    pub fn is_loopback(self) -> bool {
        matches!(self, Self::Loopback)
    }
}

/// DT2: the keep-alive configuration for a distributed deployment. Built from a
/// resolved [`crate::RoleConfig`]-style topology (the CLI layer passes the resolved
/// leg endpoints + the cadence/threshold). The CRITICAL property is the gate:
/// [`KeepAliveConfig::for_role`] returns `None` when EVERY leg is loopback, so the
/// all-local default never constructs a [`HealthPlanes`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeepAliveConfig {
    /// The runner <-> server leg endpoint classification (the LOGGED plane).
    pub runner_leg: LegEndpoint,
    /// The client <-> server leg endpoint classification (the EPHEMERAL plane).
    pub client_leg: LegEndpoint,
    /// Heartbeat cadence + stall deadline (CT5 bounded config; safe defaults).
    pub heartbeat: HeartbeatConfig,
}

impl KeepAliveConfig {
    /// Build a keep-alive config from the resolved leg endpoints, OR return `None`
    /// when the deployment is single-box (both legs loopback). Returning `None` is
    /// the DT6 inertness gate: the caller constructs NO [`HealthPlanes`] and the
    /// heartbeat machinery is never entered in the all-local default.
    ///
    /// `runner_leg` is `None` when there is no remote runner at all (e.g. a
    /// client-only or server-only process); a leg that is absent is treated as
    /// loopback (inert) for gating purposes.
    pub fn for_role(
        runner_leg: Option<LegEndpoint>,
        client_leg: Option<LegEndpoint>,
        heartbeat: HeartbeatConfig,
    ) -> Option<Self> {
        let runner_leg = runner_leg.unwrap_or(LegEndpoint::Loopback);
        let client_leg = client_leg.unwrap_or(LegEndpoint::Loopback);
        // The gate: if NEITHER leg crosses a machine boundary, this is the all-local
        // default and the planes are inert — construct nothing.
        if runner_leg.is_loopback() && client_leg.is_loopback() {
            return None;
        }
        Some(Self {
            runner_leg,
            client_leg,
            heartbeat,
        })
    }
}

/// DT2: the runner <-> server LOGGED health plane. Wraps the CT5
/// [`HeartbeatMonitor`] and maps each non-`Steady` transition into the controller's
/// `connectivity.health_changed` event family, plus signals when a recovered leg
/// must re-run the `runtime-tunnel.md` recovery sequence.
///
/// This plane NEVER fabricates process liveness: a heartbeat is a LIVENESS signal,
/// not proof a process exists. On reconnect it asks the caller to re-run
/// [`crate::RemoteProcessRunner::recover_run`] (which probes the actual remote and
/// classifies recover/orphan/exit/pending) — the heartbeat itself never asserts the
/// run survived.
#[derive(Debug)]
pub struct RunnerServerPlane {
    monitor: HeartbeatMonitor,
    /// The `runtime_process_ref` handle this plane's liveness pertains to (carried
    /// in the emitted event detail; a handle, never a credential).
    runtime_process_ref: String,
    /// The three-state LOGGED health of the runner leg, layered ON the CT5 binary
    /// reachable/unreachable probe (review finding 1). The CT5 monitor is binary
    /// (reachable | unreachable); the DT2 runner plane owns the
    /// `available -> degraded -> unreachable` vocabulary that
    /// `ConnectivityEndpoint.status` publishes, so a FIRST confirmed miss records
    /// `degraded` and a CONTINUED miss escalates to `unreachable` -- each as its own
    /// recorded `connectivity.health_changed` event.
    state: HealthState,
}

/// DT2: the LOGGED outcome of one runner-leg beat: the heartbeat outcome plus an
/// OPTIONAL recorded event and an OPTIONAL recovery-sequence trigger.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunnerBeat {
    /// The underlying CT5 heartbeat outcome (probed health + timestamp label).
    pub outcome: HeartbeatOutcome,
    /// The `connectivity.health_changed` event to APPEND, or `None` when the beat
    /// was `Steady` (an unchanged tunnel emits nothing — no spurious events).
    pub event: Option<RunnerHealthEvent>,
    /// `true` when this beat is a RECONNECT (`unreachable -> reachable`): the caller
    /// MUST re-run the runtime recovery sequence (`recover_run`) because the leg
    /// returned and the run's survival is now re-probable. Keep-alive never asserts
    /// the run is alive on its own.
    pub must_rerun_recovery: bool,
}

/// DT2: the LOGGED runner-leg health-changed event payload. Maps to the in-tree
/// `EventKind::ConnectivityHealthChanged` ("connectivity.health_changed"). It
/// carries ONLY liveness/health + the `runtime_process_ref` handle and the
/// `last_heartbeat_at` logical label — never a credential or transcript content.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunnerHealthEvent {
    /// The event kind token, always `"connectivity.health_changed"`.
    pub kind: &'static str,
    /// The new health status: `available` / `degraded` / `unreachable`.
    pub status: &'static str,
    /// The named transition detail (`initial` / `lost` / `reconnected` / `stalled`).
    pub detail: &'static str,
    /// The `runtime_process_ref` handle the liveness pertains to.
    pub runtime_process_ref: String,
    /// The replay-stable logical heartbeat instant label.
    pub last_heartbeat_at: String,
}

/// DT2: the three-state health vocabulary, matching `ConnectivityEndpoint.status`.
/// A beat advances through these; only a CHANGE produces a logged event (runner
/// leg) or an observable transition (client leg).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HealthState {
    Available,
    Degraded,
    Unreachable,
}

impl HealthState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::Degraded => "degraded",
            Self::Unreachable => "unreachable",
        }
    }
}

impl RunnerServerPlane {
    pub fn new(monitor: HeartbeatMonitor, runtime_process_ref: impl Into<String>) -> Self {
        Self {
            monitor,
            runtime_process_ref: runtime_process_ref.into(),
            state: HealthState::Available,
        }
    }

    /// Take one runner-leg heartbeat. Advances the three-state LOGGED health
    /// (`available -> degraded -> unreachable` and back) and maps each CHANGE into a
    /// `connectivity.health_changed` event; an unchanged state emits `None`. Flags
    /// whether the caller must re-run the runtime recovery sequence (on reconnect).
    ///
    /// Review finding 1: a confirmed miss does NOT jump straight to `unreachable`.
    /// The first miss records the intermediate `degraded` (the leg is impaired but
    /// not yet declared down); a continued miss escalates to `unreachable`. Both are
    /// recorded events, matching the `ConnectivityEndpoint.status` three-state
    /// contract the spec names for the LOGGED plane.
    pub fn beat(&mut self) -> RunnerBeat {
        let outcome = self.monitor.beat();
        let prev = self.state;

        // Map the CT5 binary probe + this plane's prior state onto the three-state
        // vocabulary. A reachable beat is `available`. An unreachable beat
        // (`Lost`/`Stalled`) is `degraded` on the FIRST miss (prev was available)
        // and `unreachable` once the leg is already impaired.
        let next = if outcome.reachable {
            HealthState::Available
        } else if prev == HealthState::Available {
            HealthState::Degraded
        } else {
            HealthState::Unreachable
        };
        self.state = next;

        // A recovered leg (`unreachable`/`degraded` -> `available`) is the reconnect
        // that re-runs recovery. We key this off the three-state delta (not just the
        // CT5 `Reconnected` marker) so a `degraded -> available` flap also re-probes.
        let recovered = prev != HealthState::Available && next == HealthState::Available;
        let must_rerun_recovery = recovered;

        // The audit `detail` names the cause of a DOWN-edge: a stall past the
        // deadline vs a probe failure. Both the `degraded` and the `unreachable`
        // edges carry it, so the trail preserves WHY the leg is impaired (the
        // three-state value is the `status` field; this is the `detail`). The
        // recovery edge is `reconnected`.
        let down_cause = if matches!(outcome.transition, HealthTransition::Stalled) {
            "stalled"
        } else {
            "lost"
        };
        let detail = match (prev, next) {
            (HealthState::Available, HealthState::Degraded)
            | (HealthState::Degraded, HealthState::Unreachable)
            | (HealthState::Available, HealthState::Unreachable) => down_cause,
            _ if recovered => "reconnected",
            _ => outcome.transition.detail(),
        };

        // Emit ONLY on a real state change. The initial beat establishes the
        // baseline and is recorded so the timeline has an origin.
        let is_initial = matches!(outcome.transition, HealthTransition::Initial);
        let event = if is_initial || prev != next {
            Some(RunnerHealthEvent {
                kind: "connectivity.health_changed",
                status: next.as_str(),
                detail: if is_initial { "initial" } else { detail },
                runtime_process_ref: self.runtime_process_ref.clone(),
                last_heartbeat_at: outcome.last_heartbeat_at.clone(),
            })
        } else {
            None
        };

        RunnerBeat {
            outcome,
            event,
            must_rerun_recovery,
        }
    }

    /// The current three-state LOGGED health of the runner leg (a status query; the
    /// transitions themselves are the audited events).
    pub fn state(&self) -> HealthState {
        self.state
    }

    /// The handle this plane's liveness pertains to (non-probing).
    pub fn runtime_process_ref(&self) -> &str {
        &self.runtime_process_ref
    }
}

/// DT2: the client <-> server EPHEMERAL health plane. This is an in-memory,
/// server-side connection state — it is NEVER an authoritative log entry. Client
/// connectivity jitter must not be able to write into the truth log (this protects
/// DT6's byte-for-byte regression: a flaky client can flap this state freely and
/// the event log is byte-identical to a run with no client jitter).
///
/// A miss advances `available -> degraded -> unreachable`; a recovery returns to
/// `available`. The current state is observable via [`ClientServerPlane::state`]
/// (a status query), NEVER emitted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClientServerPlane {
    /// Consecutive missed beats before `degraded` (1) then `unreachable` (>= 2).
    state: HealthState,
    consecutive_misses: u32,
    /// The miss threshold at which the state escalates to `unreachable`.
    unreachable_after: u32,
}

impl ClientServerPlane {
    /// A fresh client plane, starting `available`. `unreachable_after` is the number
    /// of consecutive missed beats at which the state becomes `unreachable`
    /// (`degraded` is reached on the first miss).
    pub fn new(unreachable_after: u32) -> Self {
        Self {
            state: HealthState::Available,
            consecutive_misses: 0,
            unreachable_after: unreachable_after.max(2),
        }
    }

    /// Record a successful client heartbeat: resets to `available`. Returns `true`
    /// if the state CHANGED (so a status query observes a recovery), but emits
    /// NOTHING — there is no event on this plane.
    pub fn observe_beat(&mut self) -> bool {
        let changed = self.state != HealthState::Available;
        self.state = HealthState::Available;
        self.consecutive_misses = 0;
        changed
    }

    /// Record a MISSED client heartbeat: escalate `available -> degraded ->
    /// unreachable`. Returns `true` if the state CHANGED. Emits NOTHING and writes
    /// NOTHING to the authoritative log — this is the whole point of the ephemeral
    /// plane.
    pub fn observe_miss(&mut self) -> bool {
        self.consecutive_misses = self.consecutive_misses.saturating_add(1);
        let next = if self.consecutive_misses >= self.unreachable_after {
            HealthState::Unreachable
        } else {
            HealthState::Degraded
        };
        let changed = self.state != next;
        self.state = next;
        changed
    }

    /// The current ephemeral connection state (the status-query view).
    pub fn state(&self) -> HealthState {
        self.state
    }
}

/// DT2: the gated two-plane container. It is constructed ONLY when a
/// [`KeepAliveConfig`] exists (i.e. a non-loopback deployment); the all-local
/// default never builds it. A plane is present for a leg ONLY when that leg is
/// non-loopback, so e.g. a server with a remote runner but a loopback client has a
/// `runner` plane and NO `client` plane.
#[derive(Debug)]
pub struct HealthPlanes {
    runner: Option<RunnerServerPlane>,
    client: Option<ClientServerPlane>,
}

impl HealthPlanes {
    /// Build the planes for a configured distributed deployment. `runner_monitor` is
    /// the CT5 heartbeat monitor for the runner leg (built by the caller from the
    /// resolved tunnel + clock); it is consumed ONLY when the runner leg is
    /// non-loopback. `client_unreachable_after` parameterizes the ephemeral plane.
    pub fn new(
        config: &KeepAliveConfig,
        runner_monitor: Option<HeartbeatMonitor>,
        runtime_process_ref: impl Into<String>,
        client_unreachable_after: u32,
    ) -> Self {
        let runner = match (config.runner_leg, runner_monitor) {
            (LegEndpoint::NonLoopback, Some(monitor)) => {
                Some(RunnerServerPlane::new(monitor, runtime_process_ref))
            }
            _ => None,
        };
        let client = match config.client_leg {
            LegEndpoint::NonLoopback => Some(ClientServerPlane::new(client_unreachable_after)),
            LegEndpoint::Loopback => None,
        };
        Self { runner, client }
    }

    /// The LOGGED runner plane, if the runner leg is non-loopback.
    pub fn runner_mut(&mut self) -> Option<&mut RunnerServerPlane> {
        self.runner.as_mut()
    }

    /// The EPHEMERAL client plane, if the client leg is non-loopback.
    pub fn client_mut(&mut self) -> Option<&mut ClientServerPlane> {
        self.client.as_mut()
    }

    /// Whether ANY plane is present. Used by DT6's structural assertion: in the
    /// all-local default `HealthPlanes` is never constructed at all, but even a
    /// partially-loopback config never builds a plane for a loopback leg.
    pub fn any_plane_present(&self) -> bool {
        self.runner.is_some() || self.client.is_some()
    }
}

/// DT2: a credential/transcript scan over a runner health event, used by the
/// "heartbeat carries no secrets" test. A health event carries ONLY the kind,
/// status, transition detail, a `runtime_process_ref` handle, and a logical
/// timestamp label — none of which is a credential. This function makes that a
/// CHECKABLE property: it returns `true` iff the rendered frame contains none of the
/// seeded credential / transcript markers.
pub fn runner_health_event_is_clean(event: &RunnerHealthEvent, markers: &[&str]) -> bool {
    let frame = format!(
        "{}|{}|{}|{}|{}",
        event.kind, event.status, event.detail, event.runtime_process_ref, event.last_heartbeat_at,
    );
    !markers.iter().any(|marker| frame.contains(marker))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connectivity_health::ConnectivityClock;
    use crate::{ConnectivityTunnel, FakeTunnelScript};

    fn scripted(timeline: Vec<bool>) -> ConnectivityTunnel {
        ConnectivityTunnel::fake_scripted(
            FakeTunnelScript::private_matching("dt2-endpoint", "trusted-runner")
                .with_health_timeline(timeline),
        )
    }

    fn runner_plane(timeline: Vec<bool>, clock: &ConnectivityClock) -> RunnerServerPlane {
        let monitor = HeartbeatMonitor::new(
            scripted(timeline),
            clock.clone(),
            HeartbeatConfig::default(),
        );
        RunnerServerPlane::new(monitor, "runtime-proc:dt2")
    }

    #[test]
    fn config_gate_is_inert_for_all_loopback() {
        // DT6 inertness gate: both legs loopback -> NO config, so NO planes are ever
        // constructed in the all-local default.
        let config = KeepAliveConfig::for_role(
            Some(LegEndpoint::Loopback),
            Some(LegEndpoint::Loopback),
            HeartbeatConfig::default(),
        );
        assert!(config.is_none(), "all-loopback deployment builds no planes");

        // Absent legs are treated as loopback (inert) too.
        let none = KeepAliveConfig::for_role(None, None, HeartbeatConfig::default());
        assert!(none.is_none());
    }

    #[test]
    fn config_gate_is_live_when_any_leg_non_loopback() {
        let config = KeepAliveConfig::for_role(
            Some(LegEndpoint::NonLoopback),
            Some(LegEndpoint::Loopback),
            HeartbeatConfig::default(),
        )
        .expect("a non-loopback runner leg builds a config");
        assert_eq!(config.runner_leg, LegEndpoint::NonLoopback);
        assert_eq!(config.client_leg, LegEndpoint::Loopback);
    }

    #[test]
    fn leg_endpoint_classifies_loopback_uris() {
        assert!(LegEndpoint::classify("tcp://127.0.0.1:7878").is_loopback());
        assert!(LegEndpoint::classify("tcp://localhost:7878").is_loopback());
        assert!(LegEndpoint::classify("tcp://[::1]:7878").is_loopback());
        assert!(!LegEndpoint::classify("tcp://100.64.0.3:7878").is_loopback());
        assert!(!LegEndpoint::classify("tunnel://capo-worker.ts.net").is_loopback());
    }

    #[test]
    fn planes_only_built_for_non_loopback_legs() {
        // A runner-non-loopback / client-loopback config builds a runner plane and
        // NO client plane (the loopback leg is inert even within a distributed run).
        let config = KeepAliveConfig::for_role(
            Some(LegEndpoint::NonLoopback),
            Some(LegEndpoint::Loopback),
            HeartbeatConfig::default(),
        )
        .unwrap();
        let clock = ConnectivityClock::manual(0);
        let monitor =
            HeartbeatMonitor::new(scripted(vec![true]), clock, HeartbeatConfig::default());
        let mut planes = HealthPlanes::new(&config, Some(monitor), "runtime-proc:dt2", 2);
        assert!(planes.runner_mut().is_some());
        assert!(planes.client_mut().is_none());
        assert!(planes.any_plane_present());
    }

    #[test]
    fn runner_leg_logs_three_state_transitions_and_reruns_recovery_on_reconnect() {
        // DT2 core (LOGGED plane, review finding 1): a missed runner heartbeat
        // transitions available -> DEGRADED (a recorded event), a CONTINUED miss
        // escalates degraded -> unreachable (a second recorded event), and a
        // recovered heartbeat emits the reconnect event AND flags that the recovery
        // sequence must re-run. The full `available -> degraded -> unreachable`
        // vocabulary the spec names for the LOGGED plane is recorded, each
        // transition its own event. Driven by a fake clock — no wall-clock sleep.
        let clock = ConnectivityClock::manual(0);
        // true, then TWO misses (degraded, then unreachable), then recover.
        let mut plane = runner_plane(vec![true, false, false, true], &clock);

        let b0 = plane.beat();
        assert_eq!(b0.outcome.transition, HealthTransition::Initial);
        // The initial beat is an event (establishes the baseline) and is `available`.
        let e0 = b0.event.expect("initial transition is logged");
        assert_eq!(e0.kind, "connectivity.health_changed");
        assert_eq!(e0.status, "available");
        assert_eq!(e0.detail, "initial");
        assert_eq!(e0.runtime_process_ref, "runtime-proc:dt2");
        assert!(!b0.must_rerun_recovery);
        assert_eq!(plane.state(), HealthState::Available);

        clock.advance(15_000);
        let b1 = plane.beat();
        let e1 = b1.event.expect("first miss is logged as degraded");
        assert_eq!(e1.status, "degraded");
        // status is the three-state vocabulary; detail carries the CAUSE (a probe
        // failure here -> `lost`; a stall would be `stalled`).
        assert_eq!(e1.detail, "lost");
        assert!(!b1.must_rerun_recovery);
        assert_eq!(plane.state(), HealthState::Degraded);

        clock.advance(15_000);
        let b2 = plane.beat();
        let e2 = b2.event.expect("continued miss escalates to unreachable");
        assert_eq!(e2.status, "unreachable");
        assert_eq!(e2.detail, "lost");
        assert!(!b2.must_rerun_recovery);
        assert_eq!(plane.state(), HealthState::Unreachable);

        clock.advance(15_000);
        let b3 = plane.beat();
        let e3 = b3.event.expect("reconnect transition is logged");
        assert_eq!(e3.status, "available");
        assert_eq!(e3.detail, "reconnected");
        assert!(
            b3.must_rerun_recovery,
            "a recovered runner leg must re-run the runtime recovery sequence"
        );

        // Steady afterwards: no event, no recovery re-run.
        clock.advance(15_000);
        let b4 = plane.beat();
        assert!(b4.event.is_none(), "steady beat emits nothing");
        assert!(!b4.must_rerun_recovery);
    }

    #[test]
    fn runner_degraded_recovers_directly_to_available_and_reruns_recovery() {
        // A degraded leg (one miss) that recovers BEFORE escalating to unreachable
        // returns straight to available, and that recovery still re-runs the runtime
        // recovery sequence (the leg flapped; the run's survival must be re-probed).
        let clock = ConnectivityClock::manual(0);
        let mut plane = runner_plane(vec![true, false, true], &clock);
        plane.beat();
        clock.advance(15_000);
        let degraded = plane.beat();
        assert_eq!(degraded.event.unwrap().status, "degraded");
        clock.advance(15_000);
        let recovered = plane.beat();
        let e = recovered.event.expect("degraded -> available is logged");
        assert_eq!(e.status, "available");
        assert_eq!(e.detail, "reconnected");
        assert!(recovered.must_rerun_recovery);
    }

    #[test]
    fn runner_stall_past_deadline_logs_degraded_with_stalled_cause() {
        // A reachable probe past the stall deadline is a confirmed miss: it records
        // `degraded` with the `stalled` cause detail (the three-state status carries
        // the cause separately) — proven by advancing the clock, never a hang. The
        // degraded -> unreachable escalation on a continued miss is covered by
        // `runner_leg_logs_three_state_transitions_and_reruns_recovery_on_reconnect`.
        let clock = ConnectivityClock::manual(0);
        let monitor = HeartbeatMonitor::new(
            scripted(vec![true]),
            clock.clone(),
            HeartbeatConfig::new(10_000, 30_000),
        );
        let mut plane = RunnerServerPlane::new(monitor, "runtime-proc:dt2");
        plane.beat();
        clock.advance(60_000);
        let stalled = plane.beat();
        let event = stalled.event.expect("stall is logged");
        assert_eq!(event.status, "degraded");
        assert_eq!(event.detail, "stalled");
        assert!(!stalled.must_rerun_recovery);
        assert_eq!(plane.state(), HealthState::Degraded);
    }

    #[test]
    fn client_leg_is_ephemeral_and_emits_no_event() {
        // DT2 core (EPHEMERAL plane): a missed client heartbeat degrades the in-memory
        // state and a recovery restores it, with NO event ever produced. The plane
        // exposes only a status query.
        let mut plane = ClientServerPlane::new(2);
        assert_eq!(plane.state(), HealthState::Available);

        assert!(plane.observe_miss(), "first miss changes state");
        assert_eq!(plane.state(), HealthState::Degraded);

        assert!(plane.observe_miss(), "second miss escalates");
        assert_eq!(plane.state(), HealthState::Unreachable);

        // A further miss stays unreachable (no change).
        assert!(!plane.observe_miss());
        assert_eq!(plane.state(), HealthState::Unreachable);

        assert!(plane.observe_beat(), "recovery changes state back");
        assert_eq!(plane.state(), HealthState::Available);
        assert!(!plane.observe_beat(), "steady beat is no change");
    }

    #[test]
    fn client_jitter_produces_no_logged_event_type() {
        // The structural guarantee behind DT6: the client plane's API has NO way to
        // produce a logged event — `observe_miss`/`observe_beat` return only a
        // changed-bool, never an event. We pin that the type carries no event field by
        // flapping it hard and asserting the only observable is the status query.
        let mut plane = ClientServerPlane::new(3);
        for _ in 0..10 {
            plane.observe_miss();
            plane.observe_beat();
        }
        // Still just a state — no log entry was ever returned to append.
        assert_eq!(plane.state(), HealthState::Available);
    }

    #[test]
    fn heartbeat_event_carries_no_credentials() {
        // DT2 safety: a runner health event carries only liveness + handles. A seeded
        // credential marker never appears in the rendered frame.
        let clock = ConnectivityClock::manual(0);
        let mut plane = runner_plane(vec![true], &clock);
        let event = plane.beat().event.expect("initial event");
        let markers = [
            "sk-ant-",
            "ANTHROPIC_API_KEY",
            "oauth-token-",
            "BEGIN OPENSSH PRIVATE KEY",
        ];
        assert!(
            runner_health_event_is_clean(&event, &markers),
            "heartbeat frame must contain no seeded credential marker"
        );

        // The clean assertion above is only meaningful if the scan ACTUALLY reads
        // the event's fields — including `runtime_process_ref`, the one field that
        // could in principle carry an opaque handle. Prove the scan is invoked over
        // that field: an event whose `runtime_process_ref` carries a seeded marker
        // is reported NOT clean. This falsifies the "trivially clean because the
        // fields are hard-coded safe" reading of the test (review finding 5): the
        // scan covers `runtime_process_ref`, so a leak there would be caught.
        let mut tainted = event.clone();
        tainted.runtime_process_ref = "runtime-proc:sk-ant-SEEDED".to_string();
        assert!(
            !runner_health_event_is_clean(&tainted, &markers),
            "the scan must read runtime_process_ref: a seeded marker there must be caught"
        );
        // And the original, untainted event stays clean (the taint above is the
        // only difference, so the scan is field-sensitive, not all-or-nothing).
        assert!(runner_health_event_is_clean(&event, &markers));
    }
}
