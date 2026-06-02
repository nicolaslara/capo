//! CT6: anti-sleep when serving locally — an OPT-IN server-lifecycle concern that
//! keeps a laptop awake while it is holding an active non-loopback exposure, kept
//! STRICTLY separate from agent execution and from the tunnel adapter itself.
//!
//! ## Boundary (CT0/CT6)
//!
//! This module shares the [`crate::connectivity_health`] lifecycle home: it is a
//! LEAF module of `capo-runtime` that owns NOTHING but a sleep-inhibitor backend +
//! its own engage/release bookkeeping. The coupling direction is ONE-WAY:
//! `exposure-state -> inhibitor`. The inhibitor NEVER reads exposure/turn/controller
//! state back, is NEVER coupled to `RuntimeRunner` or turn execution, and holds no
//! `RunId`/session/turn handle. CT7's last-revoke may CALL [`AntiSleepController::release`]
//! (the permitted one-way edge); that is the only edge into this module.
//!
//! ## Design
//!
//! - [`SleepInhibitorBackend`] is the INJECTABLE backend (modeled on the
//!   [`crate::TailscaleStatusSource`] / [`crate::connectivity_health::ConnectivityClock`]
//!   injection pattern). [`FakeInhibitorBackend`] is the deterministic test backend
//!   that records acquire/release calls WITHOUT touching the OS — unit tests never
//!   spawn a real power assertion. [`platform_backend`] returns the real
//!   platform backend (macOS IOKit power assertion / Linux `systemd-inhibit` /
//!   no-op elsewhere) for the live, gated path (CT10).
//! - The single rule for the macOS path is IOKit POWER ASSERTIONS; there is NO
//!   `caffeinate` invocation anywhere (the vendored codex `sleep-inhibitor` model
//!   has no `caffeinate` path at all).
//! - [`AntiSleepController`] is OFF by default; it engages only behind the explicit
//!   [`anti_sleep_enabled`] opt-in (`CAPO_SERVER_ANTI_SLEEP=1`). It is driven by the
//!   SERVING lifecycle: [`AntiSleepController::set_active_exposures`] takes the count
//!   of held active non-loopback exposures and engages while > 0, releasing on
//!   shutdown / last-exposure-revoked.
//! - It degrades cleanly: on a platform where sleep cannot be enforced, the backend
//!   reports [`InhibitorCapability::Unsupported`] and the controller records the
//!   limitation in its [`AntiSleepStatus`] WITHOUT claiming the laptop stays awake.
//! - Every engage/release is OBSERVABLE: [`AntiSleepController::set_active_exposures`]
//!   (and `engage`/`release`) return an [`AntiSleepTransition`] the caller records as
//!   a status field / lifecycle event; no transition ever carries a secret.

/// CT6: the explicit opt-in env var. Anti-sleep is OFF unless this is set to a
/// truthy value (`1`/`true`/`yes`/`on`, case-insensitive).
pub const ANTI_SLEEP_ENV: &str = "CAPO_SERVER_ANTI_SLEEP";

/// CT6: whether anti-sleep is opted in via the process environment. OFF by default;
/// any unset/empty/falsey value leaves it disabled (fail-safe: do not hold a power
/// assertion the operator did not ask for).
pub fn anti_sleep_enabled() -> bool {
    std::env::var(ANTI_SLEEP_ENV)
        .map(|value| env_flag_is_truthy(&value))
        .unwrap_or(false)
}

fn env_flag_is_truthy(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

/// CT6: whether the backend can actually enforce sleep prevention on this platform.
/// `Unsupported` means the controller must NOT claim the laptop stays awake (the DP7
/// "do not claim what the OS cannot enforce" discipline).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InhibitorCapability {
    /// The platform backend can hold a real power assertion (macOS IOKit, Linux
    /// `systemd-inhibit`/`gnome-session-inhibit`). Only the deterministic
    /// [`FakeInhibitorBackend::enforced`] reports this today; the real platform
    /// backends report [`InhibitorCapability::Deferred`] until the OS FFI is wired
    /// in CT10.
    Enforced,
    /// The platform CAN enforce sleep prevention, but the OS assertion is NOT yet
    /// wired (the IOKit `IOPMAssertionCreateWithName` FFI / the `systemd-inhibit`
    /// child are deferred to the gated CT10 live path). This is treated EXACTLY like
    /// `Unsupported` for the purpose of [`AntiSleepStatus::keeping_awake`] — the
    /// machine is NOT guaranteed to stay awake — but is a DISTINCT variant so the
    /// recorded limitation says "deferred to CT10" rather than "unsupported
    /// platform", never a false `Enforced` claim while the assertion is a no-op.
    Deferred,
    /// No enforcement is available on this platform; engaging is a recorded no-op
    /// and the limitation is surfaced — Capo does NOT claim the laptop stays awake.
    Unsupported,
}

impl InhibitorCapability {
    /// Whether the backend ACTUALLY holds an OS-enforced assertion. False for both
    /// `Unsupported` (no OS support) and `Deferred` (OS support exists but the FFI is
    /// not wired yet, CT10) — so Capo never claims to keep the laptop awake when it
    /// holds no real assertion.
    pub fn is_enforced(self) -> bool {
        matches!(self, InhibitorCapability::Enforced)
    }

    /// Whether engaging on this backend is a recorded NO-OP (no real assertion held):
    /// true for both `Unsupported` and `Deferred`.
    pub fn is_noop(self) -> bool {
        !self.is_enforced()
    }
}

/// CT6: the INJECTABLE sleep-inhibitor backend. The real platform backends
/// (macOS IOKit / Linux `systemd-inhibit`) implement this for the live path; the
/// deterministic [`FakeInhibitorBackend`] implements it for unit tests so no real
/// power assertion is ever taken in a test.
///
/// The backend is a PURE SINK: it receives `acquire`/`release` and reports its
/// platform capability. It never reads exposure/turn/controller state — the one-way
/// dependency is enforced structurally by giving the backend no such inputs.
pub trait SleepInhibitorBackend: std::fmt::Debug + Send {
    /// Acquire (or re-affirm) a sleep-prevention assertion. Idempotent: a backend
    /// that is already holding an assertion does nothing.
    fn acquire(&mut self);
    /// Release any held assertion. Idempotent.
    fn release(&mut self);
    /// Whether this platform backend can actually enforce sleep prevention.
    fn capability(&self) -> InhibitorCapability;
}

/// CT6: the deterministic test backend. Records the number of acquire/release calls
/// and the current held state WITHOUT touching the OS, so tests assert the state
/// machine (off-by-default / engage / release / no-op-on-unsupported) with no real
/// power assertion and no spawned process.
#[derive(Clone, Debug)]
pub struct FakeInhibitorBackend {
    capability: InhibitorCapability,
    held: bool,
    acquire_calls: usize,
    release_calls: usize,
}

impl FakeInhibitorBackend {
    /// A fake backend that reports `Enforced` (simulates a supported platform).
    pub fn enforced() -> Self {
        Self::with_capability(InhibitorCapability::Enforced)
    }

    /// A fake backend that reports `Unsupported` (simulates an unsupported platform):
    /// acquire is a recorded no-op and the assertion is never held.
    pub fn unsupported() -> Self {
        Self::with_capability(InhibitorCapability::Unsupported)
    }

    /// A fake backend that reports `Deferred` (the platform CAN enforce but the OS
    /// FFI is not wired yet — the CT6 state of the real platform backends): acquire
    /// is a recorded no-op and the assertion is never held, exactly like
    /// `Unsupported`, but the recorded limitation says "deferred".
    pub fn deferred() -> Self {
        Self::with_capability(InhibitorCapability::Deferred)
    }

    pub fn with_capability(capability: InhibitorCapability) -> Self {
        Self {
            capability,
            held: false,
            acquire_calls: 0,
            release_calls: 0,
        }
    }

    /// Whether the fake is currently HOLDING an assertion. Always `false` on an
    /// unsupported backend (it cannot enforce, so it never claims to hold one).
    pub fn is_held(&self) -> bool {
        self.held
    }

    /// How many times `acquire` transitioned not-held -> held (effective acquires).
    pub fn acquire_calls(&self) -> usize {
        self.acquire_calls
    }

    /// How many times `release` transitioned held -> not-held (effective releases).
    pub fn release_calls(&self) -> usize {
        self.release_calls
    }
}

impl SleepInhibitorBackend for FakeInhibitorBackend {
    fn acquire(&mut self) {
        // A non-enforcing backend (Unsupported OR Deferred) can never hold an
        // assertion: acquire is a no-op so the controller cannot falsely claim the
        // laptop will stay awake.
        if self.capability.is_noop() {
            return;
        }
        if !self.held {
            self.held = true;
            self.acquire_calls += 1;
        }
    }

    fn release(&mut self) {
        if self.held {
            self.held = false;
            self.release_calls += 1;
        }
    }

    fn capability(&self) -> InhibitorCapability {
        self.capability
    }
}

/// CT6: the observable engage/release transition a lifecycle update produced. The
/// caller records a non-`Unchanged` transition as a status field / lifecycle event
/// so anti-sleep is auditable; no transition ever carries a secret.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AntiSleepTransition {
    /// No change to the engaged state.
    Unchanged,
    /// not-engaged -> engaged: a power assertion was acquired (Enforced backend).
    Engaged,
    /// engaged -> not-engaged: the assertion was released.
    Released,
    /// A non-loopback exposure is held and anti-sleep is opted in, but the platform
    /// CANNOT enforce sleep prevention. This is recorded so the operator knows the
    /// laptop is NOT guaranteed to stay awake — never a silent false "Engaged".
    EngageUnsupported,
}

impl AntiSleepTransition {
    /// Whether this transition should be recorded as a lifecycle event (everything
    /// except `Unchanged`).
    pub fn is_event(self) -> bool {
        !matches!(self, AntiSleepTransition::Unchanged)
    }

    /// A stable, secret-free audit label for the transition.
    pub fn detail(self) -> &'static str {
        match self {
            AntiSleepTransition::Unchanged => "unchanged",
            AntiSleepTransition::Engaged => "engaged",
            AntiSleepTransition::Released => "released",
            AntiSleepTransition::EngageUnsupported => "engage_unsupported",
        }
    }
}

/// CT6: the auditable anti-sleep status snapshot. Secret-free: it carries only the
/// opt-in/engaged/enforced booleans, the held-exposure count, and a human limitation
/// string — never a credential.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AntiSleepStatus {
    /// Whether anti-sleep was opted in (`CAPO_SERVER_ANTI_SLEEP=1`).
    pub enabled: bool,
    /// Whether the controller currently INTENDS to keep the machine awake (an active
    /// non-loopback exposure is held AND anti-sleep is enabled).
    pub engaged: bool,
    /// Whether the platform backend can actually ENFORCE the intent.
    pub enforced: bool,
    /// The count of held active non-loopback exposures driving the state (the ONLY
    /// input — the one-way `exposure-state -> inhibitor` edge).
    pub active_exposures: usize,
    /// A recorded limitation when engagement is intended but unenforceable, so Capo
    /// does NOT claim the laptop stays awake. `None` when there is nothing to qualify.
    pub limitation: Option<String>,
}

impl AntiSleepStatus {
    /// Whether the machine is genuinely being kept awake: intended AND enforceable.
    pub fn keeping_awake(&self) -> bool {
        self.engaged && self.enforced
    }
}

/// CT6: the opt-in anti-sleep controller. OFF by default; engages a sleep-inhibitor
/// backend while the SERVING lifecycle reports a held active non-loopback exposure,
/// releasing on shutdown / last-exposure-revoked.
///
/// The ONLY input is the active-exposure count (the one-way `exposure-state ->
/// inhibitor` edge): there is NO controller/run/turn/`RuntimeRunner` handle in scope,
/// so the inhibitor structurally CANNOT read exposure/turn state back.
#[derive(Debug)]
pub struct AntiSleepController {
    enabled: bool,
    backend: Box<dyn SleepInhibitorBackend>,
    /// Held active non-loopback exposures — the sole lifecycle input. Plain `usize`:
    /// every mutator is `&mut self`-gated, so no shared-ownership indirection is
    /// needed.
    active_exposures: usize,
    engaged: bool,
}

impl AntiSleepController {
    /// Build a controller with an explicit enabled flag and an injected backend.
    /// Used directly by tests (with a [`FakeInhibitorBackend`]); production uses
    /// [`AntiSleepController::from_env`].
    pub fn new(enabled: bool, backend: Box<dyn SleepInhibitorBackend>) -> Self {
        Self {
            enabled,
            backend,
            active_exposures: 0,
            engaged: false,
        }
    }

    /// Build the production controller: enabled iff [`anti_sleep_enabled`] (the
    /// `CAPO_SERVER_ANTI_SLEEP=1` opt-in) and backed by the real [`platform_backend`].
    pub fn from_env() -> Self {
        Self::new(anti_sleep_enabled(), platform_backend())
    }

    /// Whether anti-sleep was opted in.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Whether the controller currently intends to keep the machine awake.
    pub fn is_engaged(&self) -> bool {
        self.engaged
    }

    /// The platform enforcement capability of the backing inhibitor.
    pub fn capability(&self) -> InhibitorCapability {
        self.backend.capability()
    }

    /// CT6 serving-lifecycle driver: report the count of HELD active non-loopback
    /// exposures. Anti-sleep engages while the count is > 0 (and opted in) and
    /// releases when it returns to 0 (last-exposure-revoked) — the one-way
    /// `exposure-state -> inhibitor` edge. Returns the observable transition.
    pub fn set_active_exposures(&mut self, count: usize) -> AntiSleepTransition {
        self.active_exposures = count;
        if count > 0 {
            self.engage()
        } else {
            self.release()
        }
    }

    /// Engage anti-sleep (acquire the assertion) if enabled. A no-op transition when
    /// already engaged or disabled. On an unsupported platform the intent is recorded
    /// but reported as `EngageUnsupported` — never a false `Engaged`.
    pub fn engage(&mut self) -> AntiSleepTransition {
        if !self.enabled {
            // Disabled: ensure nothing is held and report no change.
            return self.release();
        }
        let unenforceable = self.backend.capability().is_noop();
        if self.engaged {
            // Already engaged; re-affirm the assertion idempotently (a no-op on the
            // backend) and report no new event.
            self.backend.acquire();
            return AntiSleepTransition::Unchanged;
        }
        self.engaged = true;
        self.backend.acquire();
        if unenforceable {
            AntiSleepTransition::EngageUnsupported
        } else {
            AntiSleepTransition::Engaged
        }
    }

    /// Release anti-sleep (drop the assertion). Idempotent: a no-op transition when
    /// not engaged. Called on shutdown / last-exposure-revoked (CT7).
    pub fn release(&mut self) -> AntiSleepTransition {
        if !self.engaged {
            // Defensive: ensure the backend holds nothing even if state drifted.
            self.backend.release();
            return AntiSleepTransition::Unchanged;
        }
        self.engaged = false;
        self.backend.release();
        AntiSleepTransition::Released
    }

    /// The current secret-free, auditable status snapshot.
    pub fn status(&self) -> AntiSleepStatus {
        let capability = self.backend.capability();
        let enforced = capability.is_enforced();
        let active = self.active_exposures;
        let limitation = if self.engaged && !enforced {
            Some(match capability {
                // The OS assertion exists but is deferred to the gated CT10 live path.
                InhibitorCapability::Deferred => "sleep prevention is not yet wired on \
                     this platform (deferred to CT10); the machine is not guaranteed \
                     to stay awake"
                    .to_string(),
                // No OS support at all on this platform.
                _ => "sleep prevention is not enforceable on this platform; \
                     the machine is not guaranteed to stay awake"
                    .to_string(),
            })
        } else {
            None
        };
        AntiSleepStatus {
            enabled: self.enabled,
            engaged: self.engaged,
            enforced,
            active_exposures: active,
            limitation,
        }
    }
}

/// CT6: the real platform backend for the live (CT10) path.
///
/// - macOS: an IOKit `PreventUserIdleSystemSleep` power assertion (NO `caffeinate`).
/// - Linux: a held `systemd-inhibit`/`gnome-session-inhibit` child process.
/// - Other: a no-op backend reporting `Unsupported`.
///
/// Unit tests NEVER construct this — they inject a [`FakeInhibitorBackend`] — so no
/// real assertion/process is taken in the deterministic suite.
pub fn platform_backend() -> Box<dyn SleepInhibitorBackend> {
    #[cfg(target_os = "macos")]
    {
        Box::new(macos_backend::MacosPowerAssertion::new())
    }
    #[cfg(target_os = "linux")]
    {
        Box::new(linux_backend::SystemdInhibit::new())
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        Box::new(UnsupportedBackend)
    }
}

/// CT6: the no-op backend for unsupported platforms — degrades cleanly and reports
/// the limitation rather than claiming enforcement.
#[derive(Debug, Default)]
pub struct UnsupportedBackend;

impl SleepInhibitorBackend for UnsupportedBackend {
    fn acquire(&mut self) {}
    fn release(&mut self) {}
    fn capability(&self) -> InhibitorCapability {
        InhibitorCapability::Unsupported
    }
}

#[cfg(target_os = "macos")]
mod macos_backend {
    use super::{InhibitorCapability, SleepInhibitorBackend};

    /// CT6 macOS live path: a native IOKit `PreventUserIdleSystemSleep` power
    /// assertion. The single rule is IOKit power assertions — there is NO
    /// `caffeinate` invocation. The IOKit binding is created lazily at `acquire`
    /// time on the live path (CT10); the deterministic suite injects the fake
    /// backend and never reaches here.
    ///
    /// NOTE: the actual `IOPMAssertionCreateWithName` FFI is wired in the gated CT10
    /// live path (it requires the IOKit/CoreFoundation link the deterministic build
    /// does not pull in). Until then this backend reports `Deferred` capability — the
    /// platform CAN enforce via IOKit, but NO assertion is actually held — so
    /// `keeping_awake()` stays false and the status records the deferral rather than
    /// a false `Enforced` claim. It models the vendored codex `sleep-inhibitor` IOKit
    /// assertion shape and explicitly does NOT spawn `caffeinate`.
    ///
    /// No `held` flag is tracked here: until the IOKit FFI is wired there is no
    /// assertion id to bookkeep, and a `held` flag that no callee reads would be
    /// incoherent dead state that contradicts the `Deferred` capability. The
    /// eventual live implementation will store the real `IOPMAssertionID` instead.
    #[derive(Debug, Default)]
    pub struct MacosPowerAssertion;

    impl MacosPowerAssertion {
        pub fn new() -> Self {
            Self
        }
    }

    impl SleepInhibitorBackend for MacosPowerAssertion {
        fn acquire(&mut self) {
            // Live CT10: create an IOKit PreventUserIdleSystemSleep assertion here and
            // flip capability() to Enforced. NEVER spawn `caffeinate`. Until then this
            // is a genuine no-op so no false enforcement is claimed and no dead `held`
            // state is tracked.
        }

        fn release(&mut self) {
            // Live CT10: release the held IOKit assertion id here.
        }

        fn capability(&self) -> InhibitorCapability {
            // Deferred (not Enforced) until the IOKit FFI is wired in CT10.
            InhibitorCapability::Deferred
        }
    }
}

#[cfg(target_os = "linux")]
mod linux_backend {
    use super::{InhibitorCapability, SleepInhibitorBackend};

    /// CT6 Linux live path: a held `systemd-inhibit` (or `gnome-session-inhibit`)
    /// child process for the duration of the assertion. The process is spawned ONLY
    /// on the live path (CT10) — the deterministic suite injects the fake backend and
    /// never reaches here, so no child process is spawned in unit tests. Until the
    /// child is wired in CT10 this reports `Deferred` (not `Enforced`): no process is
    /// held, so `keeping_awake()` stays false and no false enforcement is claimed.
    ///
    /// No `held` flag is tracked here: until the `systemd-inhibit` child is wired
    /// there is no process handle to bookkeep, and a `held` flag that no callee
    /// reads would be incoherent dead state that contradicts the `Deferred`
    /// capability. The eventual live implementation will store the real child
    /// process handle instead.
    #[derive(Debug, Default)]
    pub struct SystemdInhibit;

    impl SystemdInhibit {
        pub fn new() -> Self {
            Self
        }
    }

    impl SleepInhibitorBackend for SystemdInhibit {
        fn acquire(&mut self) {
            // Live CT10: spawn `systemd-inhibit --what=idle:sleep ... sleep infinity`
            // (or `gnome-session-inhibit`), hold its handle, and flip capability() to
            // Enforced. Until then this is a genuine no-op (no spawned process) and no
            // dead `held` state is tracked.
        }

        fn release(&mut self) {
            // Live CT10: kill the held inhibitor child here.
        }

        fn capability(&self) -> InhibitorCapability {
            // Deferred (not Enforced) until the systemd-inhibit child is wired in CT10.
            InhibitorCapability::Deferred
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ct6_env_flag_parsing() {
        assert!(env_flag_is_truthy("1"));
        assert!(env_flag_is_truthy("true"));
        assert!(env_flag_is_truthy("TRUE"));
        assert!(env_flag_is_truthy(" yes "));
        assert!(env_flag_is_truthy("on"));
        assert!(!env_flag_is_truthy("0"));
        assert!(!env_flag_is_truthy(""));
        assert!(!env_flag_is_truthy("false"));
        assert!(!env_flag_is_truthy("nope"));
    }

    #[test]
    fn ct6_off_by_default_takes_no_assertion() {
        // CT6 core: disabled (the default), an active exposure must NOT acquire any
        // assertion — anti-sleep is opt-in.
        let backend = FakeInhibitorBackend::enforced();
        let mut controller = AntiSleepController::new(/*enabled*/ false, Box::new(backend));
        assert!(!controller.is_enabled());

        let transition = controller.set_active_exposures(1);
        assert_eq!(transition, AntiSleepTransition::Unchanged);
        assert!(!controller.is_engaged());

        let status = controller.status();
        assert!(!status.enabled);
        assert!(!status.engaged);
        assert!(!status.keeping_awake());
    }

    #[test]
    fn ct6_engages_on_active_exposure_releases_on_revoke() {
        // CT6 core: enabled + an active non-loopback exposure engages; the count
        // returning to 0 (last-exposure-revoked) releases. The transitions are the
        // observable engage/release lifecycle events.
        let mut controller =
            AntiSleepController::new(true, Box::new(FakeInhibitorBackend::enforced()));

        // First active exposure -> Engaged.
        assert_eq!(
            controller.set_active_exposures(1),
            AntiSleepTransition::Engaged
        );
        assert!(controller.is_engaged());
        assert!(controller.status().keeping_awake());

        // A second exposure while already engaged -> no new event.
        assert_eq!(
            controller.set_active_exposures(2),
            AntiSleepTransition::Unchanged
        );
        assert!(controller.is_engaged());

        // Down to one held exposure -> still engaged, no event.
        assert_eq!(
            controller.set_active_exposures(1),
            AntiSleepTransition::Unchanged
        );
        assert!(controller.is_engaged());

        // Last exposure revoked (count 0) -> Released.
        assert_eq!(
            controller.set_active_exposures(0),
            AntiSleepTransition::Released
        );
        assert!(!controller.is_engaged());
        assert!(!controller.status().keeping_awake());
    }

    #[test]
    fn ct6_explicit_release_on_shutdown_is_idempotent() {
        // CT7/shutdown may CALL release directly (the permitted one-way edge).
        let mut controller =
            AntiSleepController::new(true, Box::new(FakeInhibitorBackend::enforced()));
        controller.set_active_exposures(1);
        assert!(controller.is_engaged());

        assert_eq!(controller.release(), AntiSleepTransition::Released);
        assert!(!controller.is_engaged());
        // Idempotent: a second release is a no-op.
        assert_eq!(controller.release(), AntiSleepTransition::Unchanged);
    }

    #[test]
    fn ct6_unsupported_platform_reports_limitation_not_a_false_claim() {
        // CT6 clean degradation: on an unsupported platform an engaged intent is
        // recorded as EngageUnsupported, the assertion is NEVER held, and the status
        // records the limitation WITHOUT claiming the laptop stays awake.
        let mut controller =
            AntiSleepController::new(true, Box::new(FakeInhibitorBackend::unsupported()));

        let transition = controller.set_active_exposures(1);
        assert_eq!(transition, AntiSleepTransition::EngageUnsupported);
        assert!(controller.is_engaged(), "intent is recorded");

        let status = controller.status();
        assert!(status.enabled);
        assert!(status.engaged);
        assert!(!status.enforced);
        assert!(
            !status.keeping_awake(),
            "must NOT claim the laptop stays awake on an unsupported platform"
        );
        assert!(
            status.limitation.is_some(),
            "the limitation must be recorded"
        );
    }

    #[test]
    fn ct6_unsupported_backend_never_holds_an_assertion() {
        // The fake unsupported backend must never transition to held, even when
        // acquire is called — so Capo cannot falsely claim enforcement.
        let mut backend = FakeInhibitorBackend::unsupported();
        backend.acquire();
        assert!(!backend.is_held());
        assert_eq!(backend.acquire_calls(), 0);
    }

    #[test]
    fn ct6_enforced_backend_acquire_release_is_idempotent() {
        // The fake enforced backend counts only EFFECTIVE transitions, modeling the
        // codex inhibitor's idempotent acquire/release.
        let mut backend = FakeInhibitorBackend::enforced();
        backend.acquire();
        backend.acquire();
        backend.acquire();
        assert!(backend.is_held());
        assert_eq!(backend.acquire_calls(), 1, "acquire is idempotent");

        backend.release();
        backend.release();
        assert!(!backend.is_held());
        assert_eq!(backend.release_calls(), 1, "release is idempotent");
    }

    #[test]
    fn ct6_coupling_is_one_way_inhibitor_has_no_exposure_state_input() {
        // CT6 one-way dependency: the inhibitor backend is a pure SINK. Its trait has
        // NO method that returns or reads exposure/turn/controller state — the only
        // inputs are acquire/release/capability. This test pins that the controller's
        // ONLY state input is the active-exposure count: re-driving the SAME count
        // sequence on a fresh controller reproduces the identical transition + status,
        // proving no hidden back-read of exposure/turn/run state.
        let drive = |counts: &[usize]| {
            let mut controller =
                AntiSleepController::new(true, Box::new(FakeInhibitorBackend::enforced()));
            let mut transitions = Vec::new();
            for &count in counts {
                transitions.push(controller.set_active_exposures(count));
            }
            (transitions, controller.status())
        };

        let counts = [1usize, 2, 1, 0, 1, 0];
        let (t1, s1) = drive(&counts);
        let (t2, s2) = drive(&counts);
        assert_eq!(t1, t2, "transitions are a pure function of the count input");
        assert_eq!(s1, s2, "status is a pure function of the count input");
        assert_eq!(
            t1,
            vec![
                AntiSleepTransition::Engaged,
                AntiSleepTransition::Unchanged,
                AntiSleepTransition::Unchanged,
                AntiSleepTransition::Released,
                AntiSleepTransition::Engaged,
                AntiSleepTransition::Released,
            ]
        );
        // Ends released after the final last-exposure-revoked.
        assert!(!s1.engaged);
    }

    #[test]
    fn ct6_disabled_controller_releases_a_stale_engagement() {
        // If a controller is disabled mid-flight, driving exposures must keep it
        // released (defense-in-depth: never hold an assertion the operator disabled).
        let mut controller =
            AntiSleepController::new(false, Box::new(FakeInhibitorBackend::enforced()));
        assert_eq!(
            controller.set_active_exposures(3),
            AntiSleepTransition::Unchanged
        );
        assert!(!controller.is_engaged());
        assert_eq!(controller.engage(), AntiSleepTransition::Unchanged);
        assert!(!controller.is_engaged());
    }

    #[test]
    fn ct6_status_is_secret_free_and_renders_count() {
        let mut controller =
            AntiSleepController::new(true, Box::new(FakeInhibitorBackend::enforced()));
        controller.set_active_exposures(2);
        let status = controller.status();
        assert_eq!(status.active_exposures, 2);
        // The status carries only booleans + a count + a human limitation string.
        // There is no field that could hold a credential; the debug render is safe.
        let rendered = format!("{status:?}");
        assert!(rendered.contains("active_exposures: 2"));
    }

    /// Serializes the env-var tests in this module: `CAPO_SERVER_ANTI_SLEEP` is
    /// process-wide, so concurrent set/remove from parallel tests would race. Held for
    /// the duration of each env-sensitive test.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Run `f` with `CAPO_SERVER_ANTI_SLEEP` set to `value` (or removed when `None`),
    /// restoring the prior value afterwards, under the shared env lock.
    fn with_anti_sleep_env<R>(value: Option<&str>, f: impl FnOnce() -> R) -> R {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let prior = std::env::var(ANTI_SLEEP_ENV).ok();
        // SAFETY: env access is serialized by ENV_LOCK within this test module.
        unsafe {
            match value {
                Some(v) => std::env::set_var(ANTI_SLEEP_ENV, v),
                None => std::env::remove_var(ANTI_SLEEP_ENV),
            }
        }
        let out = f();
        unsafe {
            match prior {
                Some(v) => std::env::set_var(ANTI_SLEEP_ENV, v),
                None => std::env::remove_var(ANTI_SLEEP_ENV),
            }
        }
        out
    }

    #[test]
    fn ct6_anti_sleep_enabled_reads_the_env_opt_in() {
        // The env-var parsing path itself: unset/empty/falsey => disabled; truthy =>
        // enabled. Serialized so the process-wide env var is not raced.
        with_anti_sleep_env(None, || assert!(!anti_sleep_enabled()));
        with_anti_sleep_env(Some(""), || assert!(!anti_sleep_enabled()));
        with_anti_sleep_env(Some("0"), || assert!(!anti_sleep_enabled()));
        with_anti_sleep_env(Some("1"), || assert!(anti_sleep_enabled()));
        with_anti_sleep_env(Some("true"), || assert!(anti_sleep_enabled()));
    }

    #[test]
    fn ct6_from_env_is_disabled_when_unset() {
        // With the opt-in UNSET, from_env() builds a DISABLED controller — exercising
        // the actual env-var parsing path (not a `false` literal). Serialized via the
        // env lock; injects a fake backend is not possible through from_env (it uses
        // platform_backend by design), but a disabled controller takes no assertion
        // regardless of backend, so the platform backend here is never acquired.
        with_anti_sleep_env(None, || {
            let controller = AntiSleepController::from_env();
            assert!(
                !controller.is_enabled(),
                "from_env is disabled when the opt-in is unset"
            );
        });
    }

    #[test]
    fn ct6_from_env_is_enabled_when_opted_in() {
        // With the opt-in set, from_env() builds an ENABLED controller via the real
        // env-var path. We assert only is_enabled() — we do NOT drive exposures here,
        // so the platform backend's acquire is never called (no OS side effect).
        with_anti_sleep_env(Some("1"), || {
            let controller = AntiSleepController::from_env();
            assert!(
                controller.is_enabled(),
                "from_env is enabled when CAPO_SERVER_ANTI_SLEEP=1"
            );
        });
    }

    #[test]
    fn ct6_deferred_backend_records_limitation_not_a_false_claim() {
        // The real platform backends report `Deferred` at CT6 (OS FFI not yet wired):
        // engaging is recorded but the assertion is NEVER held, keeping_awake() stays
        // false, and the limitation says the support is deferred — never a false
        // `Enforced`/`keeping_awake` claim. Uses the injected FakeInhibitorBackend to
        // satisfy the module invariant (unit tests never construct platform_backend).
        let mut controller =
            AntiSleepController::new(true, Box::new(FakeInhibitorBackend::deferred()));
        let transition = controller.set_active_exposures(1);
        assert_eq!(transition, AntiSleepTransition::EngageUnsupported);
        assert!(controller.is_engaged(), "intent is recorded");
        assert!(!controller.capability().is_enforced());

        let status = controller.status();
        assert!(status.engaged);
        assert!(!status.enforced);
        assert!(
            !status.keeping_awake(),
            "a Deferred backend must NOT claim the laptop stays awake"
        );
        let limitation = status.limitation.expect("limitation recorded");
        assert!(
            limitation.contains("deferred"),
            "the limitation distinguishes Deferred from Unsupported: {limitation}"
        );
    }

    #[test]
    fn ct6_platform_backend_is_not_enforced_until_ct10() {
        // The real platform backend must NOT claim Enforced at CT6 — it is Deferred
        // (or Unsupported on other OSes) so keeping_awake() can never be true while no
        // OS assertion is wired. This constructs platform_backend() ONLY to read its
        // capability (no acquire/release, so no OS side effect).
        let backend = platform_backend();
        assert!(
            !backend.capability().is_enforced(),
            "platform backend must not claim Enforced until the CT10 FFI is wired"
        );
    }

    #[test]
    fn ct6_transition_detail_labels_are_stable() {
        assert_eq!(AntiSleepTransition::Engaged.detail(), "engaged");
        assert_eq!(AntiSleepTransition::Released.detail(), "released");
        assert_eq!(
            AntiSleepTransition::EngageUnsupported.detail(),
            "engage_unsupported"
        );
        assert_eq!(AntiSleepTransition::Unchanged.detail(), "unchanged");
        assert!(AntiSleepTransition::Engaged.is_event());
        assert!(!AntiSleepTransition::Unchanged.is_event());
    }
}
