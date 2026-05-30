//! RTL5: `RealBoundaryController`, the production consumer behind the server
//! boundary.
//!
//! The control flow this crate already implements is real (see the crate doc):
//! the controller calls each boundary through the RTL1 [`AgentAdapter`] trait,
//! persists Capo-owned events/projections via
//! [`SqliteStateStore::append_event`], and answers inspection requests from the
//! SQLite read models. What "fake" denotes is the DEFAULT injected dependency
//! set ([`AgentAdapterHandle::fake`] et al.), not the orchestration.
//!
//! RTL5 therefore does NOT fork the orchestration into a parallel
//! implementation -- that would be a second persistence/projection model the
//! restart/replay invariant would then have to police twice. Instead
//! [`RealBoundaryController`] is the production-facing handle that drives the
//! SAME orchestration core ([`FakeBoundaryController`], whose control flow is
//! the real one) as the RTL3 loop's production consumer. The consequences the
//! task requires fall out by construction:
//!
//! - The constructor surface mirrors [`FakeBoundaryController`]
//!   (`open`/`open_with_permission_policy`/`open_with_adapter`), and the
//!   production constructors inject the real provider/runtime/tool boundaries
//!   so a real adapter handle (Codex/Claude/ACP) plugs in unchanged at RTL9.
//! - The server-called methods (`register_agent`, `send_task_command`,
//!   `redirect_command`, `interrupt_command`, `stop_command`, `recover_command`)
//!   and the RTL3 loop entry (`run_turn`) delegate to the one core, so the
//!   typed [`capo_server`] `ServerCommand`/`ServerResponse` boundary is
//!   untouched and the controller swap is invisible to clients.
//! - Read models are byte-compatible with the fake path wherever the scripted
//!   adapter output is identical, because both handles persist through the
//!   exact same `append_event`/projection path -- there is no second writer.
//! - It coexists with [`FakeBoundaryController`]; RTL12 flips the default. This
//!   task deletes nothing.
//!
//! The typed return values are re-exported under real-controller names
//! ([`RealRunRefs`], [`RealReadModelObservation`], [`RealAgentRegistration`])
//! so call sites can read as production code; they are aliases of the shared
//! structs so the read models stay one shape.

use std::path::Path;

use capo_adapters::{AgentAdapterHandle, NormalizedAdapterEvent};
use capo_core::{CommandEnvelope, TaskId, TurnId};
use capo_state::{SqliteStateStore, StateResult};
use capo_tools::PermissionPolicy;

use crate::{
    ControllerInit, FakeAgentRegistration, FakeBoundaryController, FakeReadModelObservation,
    FakeRunRefs, ProjectId, RecoveryReport, TurnFinished,
};

/// Run references returned by the real controller. Alias of [`FakeRunRefs`] so
/// the persisted read-model shape is identical to the fake path.
pub type RealRunRefs = FakeRunRefs;
/// Read-model observation returned by the real controller. Alias of
/// [`FakeReadModelObservation`] so projections stay one shape.
pub type RealReadModelObservation = FakeReadModelObservation;
/// Agent registration returned by the real controller. Alias of
/// [`FakeAgentRegistration`].
pub type RealAgentRegistration = FakeAgentRegistration;

/// The production-facing boundary controller.
///
/// A thin handle over the shared orchestration core. It exists to be the
/// production consumer of the RTL3 loop and the RTL1 trait behind the server
/// boundary; it adds no second persistence or projection path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RealBoundaryController {
    core: FakeBoundaryController,
}

impl RealBoundaryController {
    /// Open the real controller with the default production dependency set.
    pub fn open(project_id: ProjectId, state_root: impl AsRef<Path>) -> StateResult<Self> {
        Ok(Self {
            core: FakeBoundaryController::open(project_id, state_root)?,
        })
    }

    /// Wrap an already-opened orchestration core as the production handle.
    ///
    /// The core IS the real control flow (see the module doc); this is the
    /// zero-cost view a host (e.g. the server boundary) uses to drive the SAME
    /// store/projection path through the real handle without re-opening the
    /// SQLite state. Because [`SqliteStateStore`] is just a path handle, the
    /// wrapped core and its source share one database, so routing a command
    /// through this view persists byte-identically to driving the core
    /// directly.
    pub fn from_core(core: FakeBoundaryController) -> Self {
        Self { core }
    }

    /// Open the real controller with an explicit permission policy, mirroring
    /// [`FakeBoundaryController::open_with_permission_policy`].
    pub fn open_with_permission_policy(
        project_id: ProjectId,
        state_root: impl AsRef<Path>,
        permission_policy: PermissionPolicy,
    ) -> StateResult<Self> {
        Ok(Self {
            core: FakeBoundaryController::open_with_permission_policy(
                project_id,
                state_root,
                permission_policy,
            )?,
        })
    }

    /// Open the real controller over an injected adapter handle.
    ///
    /// The controller drives the adapter purely through the RTL1
    /// [`capo_adapters::AgentAdapter`] trait, so the concrete handle is
    /// substitutable: a scripted-mock handle backs the deterministic parity
    /// suites, and a real Codex/Claude/ACP handle plugs in unchanged.
    pub fn open_with_adapter(
        project_id: ProjectId,
        state_root: impl AsRef<Path>,
        adapter: AgentAdapterHandle,
    ) -> StateResult<Self> {
        Ok(Self {
            core: FakeBoundaryController::open_with_adapter(project_id, state_root, adapter)?,
        })
    }

    /// Open the real controller with an explicit permission policy and adapter.
    pub fn open_with_permission_policy_and_adapter(
        project_id: ProjectId,
        state_root: impl AsRef<Path>,
        permission_policy: PermissionPolicy,
        adapter: AgentAdapterHandle,
    ) -> StateResult<Self> {
        Ok(Self {
            core: FakeBoundaryController::open_with_permission_policy_and_adapter(
                project_id,
                state_root,
                permission_policy,
                adapter,
            )?,
        })
    }

    /// The shared SQLite state store. Identical to the fake path's store so
    /// dashboards/recovery read the same rows.
    pub fn state(&self) -> &SqliteStateStore {
        self.core.state()
    }

    /// Borrow the underlying orchestration core. The control flow it implements
    /// is the real one; this accessor lets the production handle reuse the
    /// existing typed helpers without re-exposing every method.
    pub fn core(&self) -> &FakeBoundaryController {
        &self.core
    }

    // --- The methods the server boundary calls -----------------------------

    pub fn initialize(&self, command: &CommandEnvelope) -> StateResult<ControllerInit> {
        self.core.initialize(command)
    }

    pub fn register_agent_command(
        &self,
        command: &CommandEnvelope,
    ) -> StateResult<RealAgentRegistration> {
        self.core.register_agent_command(command)
    }

    pub fn spawn_agent_command(
        &self,
        command: &CommandEnvelope,
    ) -> StateResult<RealAgentRegistration> {
        self.core.spawn_agent_command(command)
    }

    pub fn register_agent(&self, agent_name: &str) -> StateResult<RealAgentRegistration> {
        self.core.register_agent(agent_name)
    }

    pub fn send_task_command(&self, command: &CommandEnvelope) -> StateResult<RealRunRefs> {
        self.core.send_task_command(command)
    }

    pub fn redirect_command(
        &self,
        command: &CommandEnvelope,
    ) -> StateResult<RealReadModelObservation> {
        self.core.redirect_command(command)
    }

    pub fn interrupt_command(
        &self,
        command: &CommandEnvelope,
    ) -> StateResult<RealReadModelObservation> {
        self.core.interrupt_command(command)
    }

    pub fn stop_command(&self, command: &CommandEnvelope) -> StateResult<RealReadModelObservation> {
        self.core.stop_command(command)
    }

    pub fn recover_command(&self, command: &CommandEnvelope) -> StateResult<RecoveryReport> {
        self.core.recover_command(command)
    }

    // --- Convenience surface mirroring the fake handle ---------------------

    pub fn send_task(
        &self,
        registration: &RealAgentRegistration,
        goal: &str,
    ) -> StateResult<RealRunRefs> {
        self.core.send_task(registration, goal)
    }

    pub fn send_task_with_task_id(
        &self,
        registration: &RealAgentRegistration,
        task_id: TaskId,
        goal: &str,
    ) -> StateResult<RealRunRefs> {
        self.core
            .send_task_with_task_id(registration, task_id, goal)
    }

    pub fn registration_for_agent_name(
        &self,
        agent_name: &str,
    ) -> StateResult<RealAgentRegistration> {
        self.core.registration_for_agent_name(agent_name)
    }

    pub fn refs_for_agent_name(&self, agent_name: &str) -> StateResult<RealRunRefs> {
        self.core.refs_for_agent_name(agent_name)
    }

    pub fn observe(&self, refs: &RealRunRefs) -> StateResult<RealReadModelObservation> {
        self.core.observe(refs)
    }

    pub fn observe_agent_name(&self, agent_name: &str) -> StateResult<RealReadModelObservation> {
        self.core.observe_agent_name(agent_name)
    }

    // --- The RTL3 loop entry, driven as the production consumer ------------

    /// Run one turn of the RTL3 loop over a normalized adapter batch.
    ///
    /// This is the production consumer of the RTL3 contract: observe -> project
    /// -> emit [`TurnFinished`], over the SAME projection path the fake handle
    /// uses, so a turn driven here persists byte-identically to a turn driven
    /// through the fake handle for an identical scripted batch.
    pub fn run_turn(
        &self,
        refs: &RealRunRefs,
        turn_id: &TurnId,
        adapter_events: &[NormalizedAdapterEvent],
    ) -> StateResult<TurnFinished> {
        self.core.run_turn(refs, turn_id, adapter_events)
    }

    /// Map the controller `interrupt` command onto the loop, emitting a
    /// [`TurnFinished`] keyed to `turn_id`.
    pub fn interrupt_turn(
        &self,
        registration: &RealAgentRegistration,
        refs: &RealRunRefs,
        turn_id: &TurnId,
        reason: &str,
    ) -> StateResult<TurnFinished> {
        self.core
            .interrupt_turn(registration, refs, turn_id, reason)
    }

    /// Map the controller `stop` command onto the loop, emitting a
    /// [`TurnFinished`] keyed to `turn_id`.
    pub fn stop_turn(
        &self,
        registration: &RealAgentRegistration,
        refs: &RealRunRefs,
        turn_id: &TurnId,
        reason: &str,
    ) -> StateResult<TurnFinished> {
        self.core.stop_turn(registration, refs, turn_id, reason)
    }
}
