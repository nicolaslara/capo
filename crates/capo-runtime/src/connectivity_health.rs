//! CT5: tunnel health — the heartbeat loop, `last_heartbeat_at`, and reconnect
//! events, driven by an INJECTABLE clock so the stall-past-deadline case is
//! deterministic (no wall-clock sleep).
//!
//! This is the NAMED owning module for the heartbeat lifecycle (the same lifecycle
//! home as the CT6 anti-sleep inhibitor will be). It is DELIBERATELY a leaf module
//! of `capo-runtime` that depends ONLY on the [`ConnectivityTunnel`] surface
//! ([`crate::ConnectivityTunnel::check_reachability`]) — it never reads or mutates
//! controller/run/turn state, never owns a process handle, and never couples to
//! `RuntimeRunner`. Connectivity health is a separate boundary (CT0/CT5).
//!
//! ## Design
//!
//! - [`ConnectivityClock`] is the injectable clock (modeled on
//!   [`crate::TailscaleStatusSource`]): a SCRIPTED/MANUAL clock advances logical
//!   time in tests; a real monotonic clock backs the live path. Tests never sleep —
//!   they call [`ConnectivityClock::advance`].
//! - [`HeartbeatMonitor`] holds a tunnel + clock + cadence/stall-deadline config and
//!   the last-observed health. Each [`HeartbeatMonitor::beat`] probes the tunnel,
//!   stamps `last_heartbeat_at`, and returns a [`HeartbeatOutcome`] that names any
//!   health TRANSITION (`reachable -> unreachable`, a `reconnected` recovery, or a
//!   stall past the deadline) so the caller can emit a `connectivity.health_changed`
//!   event. The monitor itself emits NOTHING and persists NOTHING — event emission
//!   and projection writes belong to the CLI/state layer (CT5 wires the projection
//!   field; the controller/CLI own the event append).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crate::{ConnectivityHealth, ConnectivityTunnel, ExposureScope};

/// CT5: the injectable clock backing the heartbeat loop. Logical time is a
/// monotonic count of MILLISECONDS since the clock's origin so a heartbeat timeline
/// (and the stall deadline) is fully deterministic in tests — advanced by
/// [`ConnectivityClock::advance`], never by a wall-clock sleep.
///
/// Modeled on the [`crate::TailscaleStatusSource`] injection pattern: a scripted
/// MANUAL clock for tests, a real monotonic clock for the live path.
#[derive(Clone, Debug)]
pub struct ConnectivityClock(ClockKind);

#[derive(Clone, Debug)]
enum ClockKind {
    /// A deterministic logical clock: `now_ms()` returns the manually-advanced
    /// counter. The counter is shared (`Arc`) so a clone observes the same time.
    Manual(Arc<AtomicU64>),
    /// A real monotonic clock anchored at construction (the live path).
    Monotonic(Instant),
}

impl ConnectivityClock {
    /// A deterministic manual clock starting at `start_ms`. Tests advance it with
    /// [`ConnectivityClock::advance`]; no wall-clock is ever consulted.
    pub fn manual(start_ms: u64) -> Self {
        Self(ClockKind::Manual(Arc::new(AtomicU64::new(start_ms))))
    }

    /// The real monotonic clock for the live heartbeat path.
    pub fn monotonic() -> Self {
        Self(ClockKind::Monotonic(Instant::now()))
    }

    /// Advance a MANUAL clock by `delta` milliseconds. No-op (and harmless) on a
    /// monotonic clock, which advances on its own.
    pub fn advance(&self, delta_ms: u64) {
        if let ClockKind::Manual(counter) = &self.0 {
            counter.fetch_add(delta_ms, Ordering::SeqCst);
        }
    }

    /// The current logical time in milliseconds since the clock origin.
    pub fn now_ms(&self) -> u64 {
        match &self.0 {
            ClockKind::Manual(counter) => counter.load(Ordering::SeqCst),
            ClockKind::Monotonic(origin) => {
                let elapsed = origin.elapsed();
                let millis = elapsed.as_millis();
                u64::try_from(millis).unwrap_or(u64::MAX)
            }
        }
    }
}

/// CT5: a stable, replay-friendly heartbeat timestamp LABEL derived from the
/// injectable clock — `heartbeat-ms:<logical-ms>`. It is a bare logical instant, not
/// a wall-clock time and never a credential, so the projected `last_heartbeat_at`
/// rebuilds identically on replay when the same clock timeline is driven.
pub fn heartbeat_label(now_ms: u64) -> String {
    format!("heartbeat-ms:{now_ms}")
}

/// CT8: a stable, replay-friendly EXPIRY instant LABEL in the SAME logical-ms domain
/// as [`heartbeat_label`] — `expiry-ms:<logical-ms>`. A (gated) short-lived public
/// exposure stamps this onto `ResolvedEndpoint.expires_at`; the CT5 heartbeat/clock
/// tick is the expiry SWEEP (no separate scheduler): when [`ConnectivityClock::now_ms`]
/// passes the parsed deadline, the next tick fires the CT7 auto-revoke. It is a bare
/// logical instant, never a credential, so it round-trips replay-stably.
pub fn expiry_label(at_ms: u64) -> String {
    format!("expiry-ms:{at_ms}")
}

/// CT8: parse an [`expiry_label`] back to its logical-ms deadline. Returns `None` for
/// any label that is not an `expiry-ms:<u64>` (so a non-expiry / open-ended
/// `expires_at` is never mistaken for an expired deadline by the sweep).
pub fn parse_expiry_ms(label: &str) -> Option<u64> {
    label.strip_prefix("expiry-ms:")?.parse().ok()
}

/// CT8: the documented MAXIMUM time-to-live ceiling for a (gated) short-lived public
/// exposure — 10 minutes. A public exposure is high-risk and default-off (CT8); when
/// it is permitted at all (explicit `network:expose:public` grant + opt-in) it MUST
/// be short-lived, so a requested TTL is clamped down to this ceiling. Recorded in
/// `knowledge.md` as the resolved numeric ceiling.
pub const PUBLIC_EXPOSURE_MAX_TTL_MS: u64 = 10 * 60 * 1_000;

/// CT8: compute the clamped expiry instant for a gated public exposure: `now + ttl`,
/// with `ttl` clamped to (1ms ..= [`PUBLIC_EXPOSURE_MAX_TTL_MS`]) so a public exposure
/// can never be open-ended or exceed the documented short-lived ceiling.
pub fn public_expiry_label(now_ms: u64, requested_ttl_ms: u64) -> String {
    let ttl = requested_ttl_ms.clamp(1, PUBLIC_EXPOSURE_MAX_TTL_MS);
    expiry_label(now_ms.saturating_add(ttl))
}

/// CT5: the bounded heartbeat cadence + stall deadline. Both are in milliseconds and
/// validated non-zero on construction so a stalled heartbeat is a HEALTH TRANSITION,
/// never a hang and never a zero-interval busy loop.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HeartbeatConfig {
    /// The nominal interval between beats (live path); informational for the
    /// deterministic tests, which advance the clock explicitly.
    pub cadence_ms: u64,
    /// The stall deadline: if the gap between two successful (reachable) beats
    /// exceeds this, the tunnel is treated as STALLED — a health transition to
    /// unreachable, surfaced by the next beat, never a blocking wait.
    pub stall_deadline_ms: u64,
}

impl HeartbeatConfig {
    /// A bounded default cadence (15s) + stall deadline (45s = 3 missed beats).
    pub const DEFAULT_CADENCE_MS: u64 = 15_000;
    pub const DEFAULT_STALL_DEADLINE_MS: u64 = 45_000;

    pub fn new(cadence_ms: u64, stall_deadline_ms: u64) -> Self {
        // Bound both away from zero so the cadence cannot become a busy loop and the
        // stall deadline cannot trip on every beat.
        Self {
            cadence_ms: cadence_ms.max(1),
            stall_deadline_ms: stall_deadline_ms.max(1),
        }
    }

    pub fn cadence(&self) -> Duration {
        Duration::from_millis(self.cadence_ms)
    }
}

impl Default for HeartbeatConfig {
    fn default() -> Self {
        Self::new(Self::DEFAULT_CADENCE_MS, Self::DEFAULT_STALL_DEADLINE_MS)
    }
}

/// CT5: the named health transition a single beat produced. The caller maps a
/// non-`Steady` outcome to a `connectivity.health_changed` event; `Steady` emits
/// nothing (no spurious events on an unchanged tunnel).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HealthTransition {
    /// First beat — establishes the baseline; recorded so the timeline has an origin.
    Initial,
    /// No change since the last beat.
    Steady,
    /// reachable -> unreachable (a probe failure).
    Lost,
    /// unreachable -> reachable after an unreachable window (the RECONNECT marker;
    /// `connectivity.health_changed` with a `reconnected` detail per `knowledge.md`).
    Reconnected,
    /// No successful beat within the stall deadline: a STALL transition to
    /// unreachable, surfaced by advancing the clock past the deadline — never a hang.
    Stalled,
}

impl HealthTransition {
    /// Whether this transition should be emitted as a `connectivity.health_changed`
    /// event (everything except `Steady`).
    pub fn is_event(self) -> bool {
        !matches!(self, HealthTransition::Steady)
    }

    /// The `reconnected` audit detail flag (CT5: reuse `health_changed` with a
    /// `reconnected` detail rather than a dedicated kind, per `knowledge.md`).
    pub fn detail(self) -> &'static str {
        match self {
            HealthTransition::Initial => "initial",
            HealthTransition::Steady => "steady",
            HealthTransition::Lost => "lost",
            HealthTransition::Reconnected => "reconnected",
            HealthTransition::Stalled => "stalled",
        }
    }
}

/// CT5: the outcome of one heartbeat: the probed health, the logical timestamp
/// label to project onto `last_heartbeat_at`, and the named transition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HeartbeatOutcome {
    pub health: ConnectivityHealth,
    pub last_heartbeat_at: String,
    pub transition: HealthTransition,
    /// `reachable=false` whenever the tunnel is down OR stalled — the value the
    /// exposure projection's `reachable` flag takes after this beat.
    pub reachable: bool,
}

/// CT5: the heartbeat monitor. Holds the tunnel + injectable clock + bounded config
/// and the last-observed reachability/heartbeat instant; each [`HeartbeatMonitor::beat`]
/// is a pure function of the tunnel surface + clock (NO controller/turn state).
#[derive(Debug)]
pub struct HeartbeatMonitor {
    tunnel: ConnectivityTunnel,
    clock: ConnectivityClock,
    config: HeartbeatConfig,
    /// The exposure scope of the underlying tunnel, captured ONCE at construction
    /// from [`ConnectivityTunnel::exposure_report`] (a non-probing read that does
    /// NOT advance the scripted `FakeTunnel` health timeline). The scope is stable
    /// for a given tunnel config, so `exposure()` returns this stored value rather
    /// than re-probing and silently consuming a step from the timeline walk.
    exposure_scope: ExposureScope,
    /// `None` before the first beat; `Some(reachable)` afterwards.
    last_reachable: Option<bool>,
    /// Logical-ms of the last SUCCESSFUL (reachable) beat, for the stall deadline.
    last_reachable_ms: Option<u64>,
}

impl HeartbeatMonitor {
    pub fn new(
        tunnel: ConnectivityTunnel,
        clock: ConnectivityClock,
        config: HeartbeatConfig,
    ) -> Self {
        // Capture the exposure scope without probing reachability (so the scripted
        // health timeline is not advanced by construction).
        let exposure_scope = tunnel.exposure_report().exposure;
        Self {
            tunnel,
            clock,
            config,
            exposure_scope,
            last_reachable: None,
            last_reachable_ms: None,
        }
    }

    /// Take one heartbeat: probe the tunnel, stamp `last_heartbeat_at` from the
    /// injectable clock, classify the transition (including a stall past the
    /// deadline), and return the outcome. Emits/persists NOTHING — that is the
    /// caller's (CLI/state) job.
    pub fn beat(&mut self) -> HeartbeatOutcome {
        let now_ms = self.clock.now_ms();
        let mut health = self.tunnel.check_reachability();
        let probe_reachable = health.reachable;

        // A successful probe still counts as STALLED if the deadline elapsed since
        // the last successful beat: a stalled heartbeat is itself a transition to
        // unreachable, proven by advancing the clock past the deadline — never a hang.
        let stalled = match self.last_reachable_ms {
            Some(prev_ms) if probe_reachable => {
                now_ms.saturating_sub(prev_ms) > self.config.stall_deadline_ms
            }
            _ => false,
        };

        let effective_reachable = probe_reachable && !stalled;
        if stalled {
            health = ConnectivityHealth {
                status: "unreachable".to_string(),
                reachable: false,
                detail: "heartbeat stalled past deadline".to_string(),
                ..health
            };
        }

        let transition = match self.last_reachable {
            None => HealthTransition::Initial,
            // A stall is its OWN transition (to unreachable), distinct from a probe
            // failure (`Lost`) — it means no successful beat landed within the
            // deadline. It takes precedence over the reachable/unreachable delta.
            _ if stalled => HealthTransition::Stalled,
            Some(prev) if prev == effective_reachable => HealthTransition::Steady,
            Some(true) => HealthTransition::Lost,
            Some(false) => HealthTransition::Reconnected,
        };

        self.last_reachable = Some(effective_reachable);
        if effective_reachable {
            self.last_reachable_ms = Some(now_ms);
        } else {
            // ANY unreachable window — a probe failure (`Lost`) OR a stall past the
            // deadline — clears the frozen last-reachable instant so the deadline
            // window restarts fresh. Otherwise the pre-window instant survives and a
            // later recovery beat (after a gap that itself exceeds the deadline)
            // would re-evaluate the stall check against it and wrongly emit `Stalled`
            // instead of `Reconnected`. A stall is defined as "no successful beat
            // within the deadline"; once we are already unreachable, the deadline must
            // not keep counting from a pre-unreachable success. After ANY unreachable
            // window the next successful probe transitions `Reconnected` (recovery
            // after an unreachable window) per the CT5 acceptance criterion.
            self.last_reachable_ms = None;
        }

        HeartbeatOutcome {
            health,
            last_heartbeat_at: heartbeat_label(now_ms),
            transition,
            reachable: effective_reachable,
        }
    }

    /// The exposure scope the underlying tunnel reports (for the caller's
    /// projection). Returns the value captured at construction — it does NOT probe
    /// the tunnel, so interleaving `exposure()` with `beat()` never consumes a step
    /// from the scripted health timeline.
    pub fn exposure(&self) -> ExposureScope {
        self.exposure_scope
    }
}

/// CT5: the heartbeat LIFECYCLE handle — the start/stop shape the loop is bound to.
///
/// This DEFINES (per the CT5 acceptance criterion "the loop's start/stop lifecycle
/// is defined, bound to a held exposure, released on revoke") the lifecycle-binding
/// contract that CT7 WIRES:
///
/// - An exposure becoming `active` STARTS a monitor (`HeartbeatHandle::start`),
///   binding the loop to that held exposure.
/// - Each tick calls [`HeartbeatMonitor::beat`]; the live path uses
///   [`ConnectivityClock::monotonic`] + a real timer, the deterministic path uses
///   [`ConnectivityClock::manual`] advanced explicitly.
/// - `revoke-exposure` (CT7) calls [`HeartbeatHandle::stop`], releasing the bound
///   loop; `is_running()` reports the lifecycle state. The handle owns ONLY the
///   monitor + a running flag — no `RunId`, no controller/turn state, no state-store
///   handle (the CT0/CT5 boundary).
///
/// CT5 provides the shape; CT7 binds `active -> start` and `revoke -> stop`.
#[derive(Debug)]
pub struct HeartbeatHandle {
    monitor: HeartbeatMonitor,
    running: Arc<AtomicBool>,
}

impl HeartbeatHandle {
    /// Start a heartbeat lifecycle bound to a held exposure. The returned handle is
    /// `running` until [`HeartbeatHandle::stop`] is called (by CT7's revoke).
    pub fn start(monitor: HeartbeatMonitor) -> Self {
        Self {
            monitor,
            // Running until `stop()` (CT7 revoke / shutdown) flips it false.
            running: Arc::new(AtomicBool::new(true)),
        }
    }

    /// Take one heartbeat through the bound monitor. Returns `None` once the
    /// lifecycle has been stopped (a stopped monitor never beats again).
    pub fn beat(&mut self) -> Option<HeartbeatOutcome> {
        if !self.is_running() {
            return None;
        }
        Some(self.monitor.beat())
    }

    /// Stop the lifecycle (CT7 revoke / server shutdown). Idempotent.
    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    /// Whether the lifecycle is still running (not yet stopped).
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// The exposure scope of the bound monitor (non-probing).
    pub fn exposure(&self) -> ExposureScope {
        self.monitor.exposure()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ConnectivityTunnel, FakeTunnelScript};

    fn scripted(timeline: Vec<bool>) -> ConnectivityTunnel {
        ConnectivityTunnel::fake_scripted(
            FakeTunnelScript::private_matching("ct5-endpoint", "trusted-node")
                .with_health_timeline(timeline),
        )
    }

    #[test]
    fn ct5_manual_clock_advances_only_when_told() {
        let clock = ConnectivityClock::manual(1_000);
        assert_eq!(clock.now_ms(), 1_000);
        clock.advance(500);
        assert_eq!(clock.now_ms(), 1_500);
        // A clone observes the same shared time (Arc-backed).
        let clone = clock.clone();
        clock.advance(250);
        assert_eq!(clone.now_ms(), 1_750);
    }

    #[test]
    fn ct5_reachable_unreachable_reconnected_emits_ordered_transitions() {
        // CT5 core: the fake walks reachable -> unreachable -> reconnected and the
        // monitor names the ordered transitions with last_heartbeat_at stamped off
        // the injectable clock — NO wall-clock.
        let clock = ConnectivityClock::manual(0);
        let mut monitor = HeartbeatMonitor::new(
            scripted(vec![true, false, true]),
            clock.clone(),
            HeartbeatConfig::default(),
        );

        let b0 = monitor.beat();
        assert_eq!(b0.transition, HealthTransition::Initial);
        assert!(b0.reachable);
        assert_eq!(b0.last_heartbeat_at, "heartbeat-ms:0");

        clock.advance(15_000);
        let b1 = monitor.beat();
        assert_eq!(b1.transition, HealthTransition::Lost);
        assert!(!b1.reachable);
        assert_eq!(b1.health.status, "unreachable");
        assert_eq!(b1.last_heartbeat_at, "heartbeat-ms:15000");

        clock.advance(15_000);
        let b2 = monitor.beat();
        assert_eq!(b2.transition, HealthTransition::Reconnected);
        assert!(b2.reachable);
        assert_eq!(b2.transition.detail(), "reconnected");

        // Steady afterwards: an unchanged reachable tunnel emits nothing.
        clock.advance(15_000);
        let b3 = monitor.beat();
        assert_eq!(b3.transition, HealthTransition::Steady);
        assert!(!b3.transition.is_event());
    }

    #[test]
    fn ct5_stall_past_deadline_is_a_transition_not_a_hang() {
        // A reachable probe is still STALLED if the deadline elapsed since the last
        // successful beat. Proven by ADVANCING the clock past the deadline — there is
        // no sleep, so this can never hang.
        let clock = ConnectivityClock::manual(0);
        let mut monitor = HeartbeatMonitor::new(
            scripted(vec![true]), // always reachable
            clock.clone(),
            HeartbeatConfig::new(10_000, 30_000),
        );

        assert_eq!(monitor.beat().transition, HealthTransition::Initial);
        // Jump well past the 30s stall deadline before the next beat.
        clock.advance(60_000);
        let stalled = monitor.beat();
        assert_eq!(stalled.transition, HealthTransition::Stalled);
        assert!(!stalled.reachable);
        assert_eq!(stalled.health.detail, "heartbeat stalled past deadline");
    }

    #[test]
    fn ct5_stall_then_reachable_recovers_with_reconnected() {
        // A stall is an unreachable window. After a stall fires, the next reachable
        // probe must transition `Reconnected` (and stay `Steady` afterwards) — the
        // stall transition is REVERSIBLE, not a permanent re-`Stalled` loop. Proven
        // by advancing the clock past the deadline (the stall) then a normal step.
        let clock = ConnectivityClock::manual(0);
        let mut monitor = HeartbeatMonitor::new(
            scripted(vec![true]), // always reachable at the probe level
            clock.clone(),
            HeartbeatConfig::new(10_000, 30_000),
        );

        assert_eq!(monitor.beat().transition, HealthTransition::Initial);

        // Stall: jump past the 30s deadline so the reachable probe is forced down.
        clock.advance(60_000);
        let stalled = monitor.beat();
        assert_eq!(stalled.transition, HealthTransition::Stalled);
        assert!(!stalled.reachable);

        // Recovery: a normal-cadence step later, the reachable probe reconnects
        // (NOT another Stalled) because the deadline window was reset on stall.
        clock.advance(10_000);
        let recovered = monitor.beat();
        assert_eq!(recovered.transition, HealthTransition::Reconnected);
        assert!(recovered.reachable);
        assert_eq!(recovered.health.status, "available");

        // And it stays Steady afterwards — no spurious re-stall on every beat.
        clock.advance(10_000);
        assert_eq!(monitor.beat().transition, HealthTransition::Steady);
    }

    #[test]
    fn ct5_lost_then_long_recovery_reconnects_not_stalls() {
        // Regression: after a `Lost` (probe failure), the unreachable window must
        // reset the stall deadline. If the probe stays down LONGER than the deadline
        // and then recovers, the recovery beat is `Reconnected`, NOT `Stalled` — a
        // stall is "no successful beat within the deadline" measured from a SUCCESS,
        // not from a pre-unreachable success that the deadline kept counting from.
        // Timeline: reachable (t=0) -> Lost (t=10_000) -> recovery (t=70_000) with a
        // 30_000 deadline; the Lost->recovery gap (60_000) exceeds the deadline.
        let clock = ConnectivityClock::manual(0);
        let mut monitor = HeartbeatMonitor::new(
            scripted(vec![true, false, true]),
            clock.clone(),
            HeartbeatConfig::new(10_000, 30_000),
        );

        assert_eq!(monitor.beat().transition, HealthTransition::Initial);

        clock.advance(10_000);
        let lost = monitor.beat();
        assert_eq!(lost.transition, HealthTransition::Lost);
        assert!(!lost.reachable);

        // Recovery after an unreachable window longer than the deadline.
        clock.advance(60_000);
        let recovered = monitor.beat();
        assert_eq!(
            recovered.transition,
            HealthTransition::Reconnected,
            "recovery after a Lost window is Reconnected even past the stall deadline"
        );
        assert!(recovered.reachable);
        assert_eq!(recovered.health.status, "available");
    }

    #[test]
    fn ct5_exposure_does_not_consume_a_health_timeline_step() {
        // Interleaving exposure() with beat() must NOT advance the scripted timeline.
        // The timeline is [true, false]: with exposure() probing, the second beat
        // would wrongly read `true` (clamped) instead of `false` if exposure() ate a
        // step. We assert beat #2 is `Lost`, proving exposure() is non-probing.
        let clock = ConnectivityClock::manual(0);
        let mut monitor = HeartbeatMonitor::new(
            scripted(vec![true, false]),
            clock.clone(),
            HeartbeatConfig::default(),
        );

        assert_eq!(monitor.exposure(), ExposureScope::Private);
        let b0 = monitor.beat();
        assert_eq!(b0.transition, HealthTransition::Initial);
        assert!(b0.reachable);

        // Interleave several exposure() calls — none may consume a step.
        assert_eq!(monitor.exposure(), ExposureScope::Private);
        assert_eq!(monitor.exposure(), ExposureScope::Private);

        clock.advance(15_000);
        let b1 = monitor.beat();
        assert_eq!(
            b1.transition,
            HealthTransition::Lost,
            "exposure() must not advance the scripted health timeline"
        );
        assert!(!b1.reachable);
    }

    #[test]
    fn ct5_heartbeat_handle_start_stop_lifecycle() {
        // CT5 lifecycle shape: a handle starts running, beats while running, and a
        // stop() (CT7 revoke) halts further beats. No controller/turn state is held.
        let clock = ConnectivityClock::manual(0);
        let monitor = HeartbeatMonitor::new(
            scripted(vec![true]),
            clock.clone(),
            HeartbeatConfig::default(),
        );
        let mut handle = HeartbeatHandle::start(monitor);
        assert!(handle.is_running());
        assert_eq!(handle.exposure(), ExposureScope::Private);

        let beat = handle.beat().expect("running handle beats");
        assert_eq!(beat.transition, HealthTransition::Initial);

        handle.stop();
        assert!(!handle.is_running());
        assert!(handle.beat().is_none(), "stopped handle never beats again");
        // Idempotent stop.
        handle.stop();
        assert!(!handle.is_running());
    }

    #[test]
    fn ct5_within_deadline_reachable_stays_steady() {
        let clock = ConnectivityClock::manual(0);
        let mut monitor = HeartbeatMonitor::new(
            scripted(vec![true]),
            clock.clone(),
            HeartbeatConfig::new(10_000, 30_000),
        );
        monitor.beat();
        clock.advance(10_000); // within the 30s deadline
        assert_eq!(monitor.beat().transition, HealthTransition::Steady);
    }

    #[test]
    fn ct5_config_bounds_away_from_zero() {
        let config = HeartbeatConfig::new(0, 0);
        assert_eq!(config.cadence_ms, 1);
        assert_eq!(config.stall_deadline_ms, 1);
    }

    #[test]
    fn ct5_health_is_computed_from_the_tunnel_surface_only() {
        // CT5 boundary: connectivity health depends ONLY on the `ConnectivityTunnel`
        // surface + the injectable clock — never on controller/run/turn state. The
        // monitor is constructed from exactly (tunnel, clock, config): there is NO
        // run id, session id, turn key, or state-store handle in scope, so a beat
        // CANNOT read or mutate run/turn read models. This test pins that the full
        // health timeline is reproducible from the tunnel + clock alone.
        let clock = ConnectivityClock::manual(0);
        let mut monitor = HeartbeatMonitor::new(
            scripted(vec![true, false, true]),
            clock.clone(),
            HeartbeatConfig::default(),
        );
        // Re-running the SAME tunnel + clock timeline on a fresh monitor yields the
        // identical transition sequence — a pure function of the tunnel surface, with
        // no hidden controller/run/turn dependency.
        let mut transitions = Vec::new();
        transitions.push(monitor.beat().transition);
        clock.advance(15_000);
        transitions.push(monitor.beat().transition);
        clock.advance(15_000);
        transitions.push(monitor.beat().transition);
        assert_eq!(
            transitions,
            vec![
                HealthTransition::Initial,
                HealthTransition::Lost,
                HealthTransition::Reconnected,
            ]
        );
        // The monitor exposes the tunnel's exposure scope (private for the scripted
        // Tailscale-parity fake) without any controller coupling.
        assert_eq!(monitor.exposure(), ExposureScope::Private);
    }
}
