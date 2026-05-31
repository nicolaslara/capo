//! RTL11/RTL12: the single-switch controller cutover.
//!
//! Default chat (`SendTask`) and steer (`SteerAgent`), plus the rest of the
//! command surface (`register`/`interrupt`/`stop`/`recover`), route through
//! ONE typed switch -- [`ControllerSelection`] -- that selects between the
//! rollback-target [`FakeBoundaryController`] and the now-default production
//! [`RealBoundaryController`]. There are no scattered booleans: the whole
//! routing decision is this one enum, chosen once at server construction. The
//! RTL12 cutover flipped this switch's default to [`ControllerSelection::Real`]
//! after the parity suite passed (see the per-variant docs below).
//!
//! ## Scope: what this switch routes (and what it deliberately does NOT)
//!
//! This switch routes the **command-envelope surface** -- the lightweight
//! `register`/`send`/`steer`/`interrupt`/`stop`/`recover` methods listed on
//! [`ControllerRoute`] below. That is exactly the set the RTL11 acceptance
//! criteria name, and the set whose handle choice clients could otherwise
//! observe through the typed `ServerCommand` boundary.
//!
//! It deliberately does NOT route the RTL3 turn-loop *ingestion* entry points
//! -- `apply_normalized_adapter_events_with_turn` (fixture replay and
//! live-provider ingest), `prepare_local_adapter_dispatch_run` (session start),
//! and `abort_run_for_ceiling`. Those drive the server's one shared
//! orchestration core directly (`self.controller.<method>` in `lib.rs`,
//! `live_provider.rs`, and `turn_orchestration.rs`) and are never wired through
//! [`ControllerSelection`]/[`ControllerRoute`]. This is intentional, not a gap
//! the RTL12 cutover left open: [`RealBoundaryController`] is a zero-cost view
//! over that same orchestration core, so routing loop ingestion through the
//! `Real` handle would be a literal no-op -- it would call the same core methods
//! over the same store. There is no separate "real loop" to flip to; the loop
//! already runs on the one core that the real handle merely views. What the
//! RTL12 cutover actually changed is the *default* of this command-surface
//! switch (now [`ControllerSelection::Real`], see below), gated on the parity
//! suite in `crates/capo-controller/src/tests.rs`. The injection seam that lets
//! a real/scripted adapter back that one core (and therefore both the routed
//! command surface and the loop) is
//! [`crate::CapoServer::open_with_controller_and_adapter`].
//!
//! Why routing only the command surface is the right shape:
//!
//! - [`RealBoundaryController`] is a zero-cost view over the SAME orchestration
//!   core ([`FakeBoundaryController`], whose control flow is the real one --
//!   see `real_controller.rs`). Routing a command through the real handle wraps
//!   the server's existing core via [`RealBoundaryController::from_core`] and
//!   persists through the exact same `append_event`/projection path. For the
//!   command surface this makes the swap invisible by construction; RTL5 owns
//!   the byte-level parity proof over a scripted-mock adapter
//!   (`crates/capo-controller/src/tests.rs`), and RTL12 owns the loop-level
//!   parity criterion. These RTL11 server tests are boundary-wiring/smoke
//!   tests, not the parity authority.
//! - The `ServerCommand`/`ServerResponse` boundary is untouched. Only the
//!   controller selection changes; clients cannot tell which handle served a
//!   command.
//! - The default is [`ControllerSelection::Real`]: the RTL12 cutover flipped it
//!   here after the parity suite passed, so default chat routes through the real
//!   handle. The fake routing is the rollback target, selectable explicitly or
//!   via a falsey opt-in env value (via [`ControllerSelection::from_env`] /
//!   [`CapoServer::open_with_controller`]).
//!
//! Rollback is a one-line revert of the selected value back to
//! [`ControllerSelection::Fake`], or -- without a code change -- a single falsey
//! [`REAL_CONTROLLER_OPT_IN_ENV`] value (`0`/`false`/`no`/`off`). Note that
//! *unsetting* the env no longer rolls back: an absent env now defers to the
//! [`ControllerSelection::Real`] default. The default routing is recorded in
//! `workpads/real-turn-loop/knowledge.md`.

use capo_adapters::AgentAdapterHandle;
use capo_controller::{
    FakeAgentRegistration, FakeBoundaryController, FakeReadModelObservation, FakeRunRefs,
    RealBoundaryController, RecoveryReport,
};
use capo_core::CommandEnvelope;
use capo_state::StateResult;

/// Opt-in environment gate selecting the real controller.
///
/// Mirrors the live-provider opt-in posture: absent/empty/`0`/`false` keeps the
/// default fake routing; any truthy value selects the real controller. Reading
/// the switch from a single env var keeps the cutover a one-value decision,
/// never a constellation of booleans.
pub const REAL_CONTROLLER_OPT_IN_ENV: &str = "CAPO_SERVER_REAL_CONTROLLER";

/// The single typed switch that selects the boundary controller.
///
/// The RTL12 cutover flipped the default to [`ControllerSelection::Real`], now
/// that the parity suite passes (the deterministic `send`/`steer`/`interrupt`/
/// `stop` + restart/replay suite and the parity-equivalence test in
/// `crates/capo-controller/src/tests.rs` and the multi-turn-edit suite in
/// `crates/capo-server/src/tests/multi_turn_edit.rs`). The documented rollback is
/// a one-value revert: restore this default to `Fake` (or set the
/// [`REAL_CONTROLLER_OPT_IN_ENV`] opt-in to a falsey value, e.g.
/// `CAPO_SERVER_REAL_CONTROLLER=0`). No event schema, projection, or
/// `ServerCommand` change is involved, so rollback needs no data migration.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ControllerSelection {
    /// Route command handling through [`FakeBoundaryController`]. No longer the
    /// default after the RTL12 cutover; it remains the documented rollback target
    /// and is still selectable explicitly or via a falsey opt-in env value.
    Fake,
    /// Route command handling through [`RealBoundaryController`] over the same
    /// orchestration core. Default since the RTL12 cutover.
    #[default]
    Real,
}

impl ControllerSelection {
    /// Resolve the selection from the [`REAL_CONTROLLER_OPT_IN_ENV`] gate.
    ///
    /// After the RTL12 cutover the default is [`ControllerSelection::Real`], so
    /// an absent env var resolves to the (real) default; the env var is now the
    /// rollback knob: a falsey value (`0`/`false`/`no`/`off`) forces the fake
    /// routing back on, a truthy value pins the real routing explicitly.
    pub fn from_env() -> Self {
        match std::env::var(REAL_CONTROLLER_OPT_IN_ENV) {
            Ok(value) => Self::from_opt_in(&value),
            Err(_) => Self::default(),
        }
    }

    /// Interpret the [`REAL_CONTROLLER_OPT_IN_ENV`] value: a falsey string
    /// (`0`/`false`/`no`/`off`, the documented rollback) forces the fake routing;
    /// a truthy string pins the real routing; an empty string defers to the
    /// post-cutover default ([`ControllerSelection::Real`]).
    pub fn from_opt_in(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Self::Real,
            "0" | "false" | "no" | "off" => Self::Fake,
            "" => Self::default(),
            // Any other unexpected value keeps the fake routing -- the
            // conservative choice for an unparsable rollback knob.
            _ => Self::Fake,
        }
    }

    /// Whether this selection routes through the real controller.
    pub fn is_real(self) -> bool {
        matches!(self, Self::Real)
    }
}

/// A command-routing view bound to the chosen [`ControllerSelection`].
///
/// Holds a borrow of the server's orchestration core for the fake routing and,
/// for the real routing, a [`RealBoundaryController`] wrapping a clone of that
/// same core (a path-only `SqliteStateStore` clone, so both share one DB). The
/// command methods forward to whichever handle the switch selected; the
/// persisted effect is identical because there is only one store.
pub(crate) enum ControllerRoute<'a> {
    Fake(&'a FakeBoundaryController),
    // Boxed: the real handle owns a clone of the orchestration core, which is
    // far larger than the fake borrow. The box keeps the routing view small;
    // the clone is a path-only `SqliteStateStore`, so the allocation is cheap
    // and the box adds one transient indirection per command.
    Real(Box<RealBoundaryController>),
}

impl<'a> ControllerRoute<'a> {
    pub(crate) fn new(selection: ControllerSelection, core: &'a FakeBoundaryController) -> Self {
        match selection {
            ControllerSelection::Fake => Self::Fake(core),
            ControllerSelection::Real => {
                Self::Real(Box::new(RealBoundaryController::from_core(core.clone())))
            }
        }
    }

    /// AI2: a route whose chat handle is the supplied per-agent adapter (a
    /// Codex-bound [`AgentAdapterHandle::codex`]), built over a CLONE of the
    /// shared core.
    ///
    /// This is the binding-respecting chat route: the server uses it ONLY for a
    /// Codex-bound agent's `SendTask`/`SteerAgent`, so that agent's chat turn
    /// drives the real Codex handle while every other agent keeps the shared
    /// (fake/default) adapter through [`Self::new`]. The core clone shares the one
    /// SQLite store (a path handle), so swapping the adapter changes only which
    /// handle drives the turn, never the persisted store/projection path. A bound
    /// agent's turn still fails CLOSED-FAST (immediate typed error, no spawn) when
    /// the live-provider gate is off. Codex chat respects the same routing switch:
    /// the `Fake` selection (the documented rollback) keeps the fake adapter even
    /// for a bound agent, so a single env value can disable the real path entirely.
    pub(crate) fn new_codex_bound(
        selection: ControllerSelection,
        core: &'a FakeBoundaryController,
        adapter: AgentAdapterHandle,
    ) -> Self {
        match selection {
            // Rollback (the documented `Fake` selection / falsey opt-in): keep the
            // shared fake adapter even for a Codex-bound agent, so a single env
            // value disables the real chat path without any code change.
            ControllerSelection::Fake => Self::Fake(core),
            ControllerSelection::Real => Self::Real(Box::new(
                RealBoundaryController::from_core(core.clone()).with_adapter(adapter),
            )),
        }
    }

    pub(crate) fn register_agent_command(
        &self,
        command: &CommandEnvelope,
    ) -> StateResult<FakeAgentRegistration> {
        match self {
            Self::Fake(core) => core.register_agent_command(command),
            Self::Real(real) => real.register_agent_command(command),
        }
    }

    pub(crate) fn send_task_command(&self, command: &CommandEnvelope) -> StateResult<FakeRunRefs> {
        match self {
            Self::Fake(core) => core.send_task_command(command),
            Self::Real(real) => real.send_task_command(command),
        }
    }

    pub(crate) fn redirect_command(
        &self,
        command: &CommandEnvelope,
    ) -> StateResult<FakeReadModelObservation> {
        match self {
            Self::Fake(core) => core.redirect_command(command),
            Self::Real(real) => real.redirect_command(command),
        }
    }

    pub(crate) fn interrupt_command(
        &self,
        command: &CommandEnvelope,
    ) -> StateResult<FakeReadModelObservation> {
        match self {
            Self::Fake(core) => core.interrupt_command(command),
            Self::Real(real) => real.interrupt_command(command),
        }
    }

    pub(crate) fn stop_command(
        &self,
        command: &CommandEnvelope,
    ) -> StateResult<FakeReadModelObservation> {
        match self {
            Self::Fake(core) => core.stop_command(command),
            Self::Real(real) => real.stop_command(command),
        }
    }

    pub(crate) fn recover_command(&self, command: &CommandEnvelope) -> StateResult<RecoveryReport> {
        match self {
            Self::Fake(core) => core.recover_command(command),
            Self::Real(real) => real.recover_command(command),
        }
    }
}
