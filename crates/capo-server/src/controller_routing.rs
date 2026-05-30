//! RTL11: the single-switch controller cutover.
//!
//! Default chat (`SendTask`) and steer (`SteerAgent`), plus the rest of the
//! command surface (`register`/`interrupt`/`stop`/`recover`), route through
//! ONE typed switch -- [`ControllerSelection`] -- that selects between the
//! default [`FakeBoundaryController`] and the production
//! [`RealBoundaryController`]. There are no scattered booleans: the whole
//! routing decision is this one enum, chosen once at server construction.
//!
//! Why this is the right shape for phase 1:
//!
//! - [`RealBoundaryController`] is a zero-cost view over the SAME orchestration
//!   core ([`FakeBoundaryController`], whose control flow is the real one --
//!   see `real_controller.rs`). Routing a command through the real handle wraps
//!   the server's existing core via [`RealBoundaryController::from_core`] and
//!   persists through the exact same `append_event`/projection path, so the
//!   parity invariant (RTL12) holds by construction: both routings drive the
//!   one store, the one event log, the one projection set.
//! - The `ServerCommand`/`ServerResponse` boundary is untouched. Only the
//!   controller selection changes; clients cannot tell which handle served a
//!   command.
//! - The default is [`ControllerSelection::Fake`]: until RTL12 flips it after
//!   the parity suite passes, default chat keeps routing through the fake
//!   handle and the real path is strictly opt-in (via
//!   [`ControllerSelection::from_env`] / [`CapoServer::open_with_controller`]).
//!
//! Rollback is a one-line revert of the selected value back to
//! [`ControllerSelection::Fake`] (or unsetting the opt-in env). The default
//! routing is recorded in `workpads/real-turn-loop/knowledge.md`.

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
/// Phase 1 default is [`ControllerSelection::Fake`]. The RTL12 cutover flips
/// the default to [`ControllerSelection::Real`] only after the parity suite
/// passes; the documented rollback is to restore this default to `Fake`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ControllerSelection {
    /// Route command handling through [`FakeBoundaryController`] (default until
    /// the RTL12 cutover).
    #[default]
    Fake,
    /// Route command handling through [`RealBoundaryController`] over the same
    /// orchestration core.
    Real,
}

impl ControllerSelection {
    /// Resolve the selection from the [`REAL_CONTROLLER_OPT_IN_ENV`] opt-in
    /// gate. Absent or falsey keeps the default fake routing.
    pub fn from_env() -> Self {
        match std::env::var(REAL_CONTROLLER_OPT_IN_ENV) {
            Ok(value) => Self::from_opt_in(&value),
            Err(_) => Self::Fake,
        }
    }

    /// Interpret an opt-in string: truthy selects the real controller, anything
    /// else (including empty/`0`/`false`) keeps the fake default.
    pub fn from_opt_in(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Self::Real,
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
