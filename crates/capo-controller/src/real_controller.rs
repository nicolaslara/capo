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
//! - It coexists with [`FakeBoundaryController`]; the RTL12 cutover flipped the
//!   default routing to this handle (the rollback is the one falsey
//!   `CAPO_SERVER_REAL_CONTROLLER` value). Neither handle is deleted.
//!
//! The typed return values are re-exported under real-controller names
//! ([`RealRunRefs`], [`RealReadModelObservation`], [`RealAgentRegistration`])
//! so call sites can read as production code; they are aliases of the shared
//! structs so the read models stay one shape.

use std::path::Path;

use capo_adapters::{AgentAdapterHandle, NormalizedAdapterEvent};
use capo_core::{CommandEnvelope, TaskId, TurnId};
use capo_state::{SqliteStateStore, StateResult};
use capo_tools::{
    CapoToolRegistry, PermissionPolicy, RuntimeToolConfig, ToolExposure, ToolExposureRequest,
};

use crate::{
    ControllerInit, FakeAgentRegistration, FakeBoundaryController, FakeReadModelObservation,
    FakeRunRefs, ProjectId, RecoveryReport, ToolDispatchOutcome, ToolDispatchScope, TurnFinished,
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
    /// The REAL tool exposures the loop dispatches through (ACI1). `capo` is the
    /// Capo-owned registry; `runtime` is the workspace-confined runtime wrappers
    /// once a workspace config is wired. Both are non-fake by construction --
    /// the test-only [`ToolExposure::fake`] is never installed here, so a real
    /// loop turn cannot silently default to the fake summary shim.
    tools: RealToolExposures,
}

/// The real, non-fake tool exposures the production controller dispatches
/// through. Kept as a pair because the `ToolExposure` enum is single-variant per
/// handle and the loop dispatches Capo-owned and runtime-wrapper tools through
/// distinct registries.
#[derive(Clone, Debug, Eq, PartialEq)]
struct RealToolExposures {
    capo: ToolExposure,
    runtime: Option<ToolExposure>,
    /// ACI8: the `GO2` agent-reporting / evidence tool surface, always live (like
    /// the Capo registry) and never the fake default.
    agent_reports: ToolExposure,
}

impl RealToolExposures {
    /// The default real exposures: the Capo registry and the agent-report
    /// surface are always live; the runtime wrappers are wired only once a
    /// workspace config is provided
    /// ([`RealBoundaryController::with_runtime_tools`]).
    fn default_real() -> Self {
        Self {
            capo: ToolExposure::capo(),
            runtime: None,
            agent_reports: ToolExposure::agent_reports(),
        }
    }
}

impl RealBoundaryController {
    /// Open the real controller with the default production dependency set.
    pub fn open(project_id: ProjectId, state_root: impl AsRef<Path>) -> StateResult<Self> {
        Ok(Self {
            core: FakeBoundaryController::open(project_id, state_root)?,
            tools: RealToolExposures::default_real(),
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
        Self {
            core,
            tools: RealToolExposures::default_real(),
        }
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
            tools: RealToolExposures::default_real(),
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
            tools: RealToolExposures::default_real(),
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
            tools: RealToolExposures::default_real(),
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

    /// Wire the workspace-confined runtime wrappers (ACI1).
    ///
    /// The Capo registry is always live; the runtime wrappers
    /// (`capo.shell_run`/`capo.file_read`/...) need a workspace + artifact root,
    /// so they are wired here. The installed exposure is the REAL
    /// [`RuntimeToolWrappers`](capo_tools::RuntimeToolWrappers), never the fake.
    #[must_use]
    pub fn with_runtime_tools(mut self, config: RuntimeToolConfig) -> Self {
        self.tools.runtime = Some(ToolExposure::runtime_wrappers(config));
        self
    }

    /// Whether the Capo-owned tool exposure routes through the real registry
    /// rather than the test-only fake shim. ACI1 invariant: this is always
    /// `true` for the production controller.
    pub fn capo_tools_are_real(&self) -> bool {
        !self.tools.capo.binding().fake
    }

    /// Whether a real runtime-wrapper exposure has been wired (and, if so, that
    /// it is non-fake).
    pub fn runtime_tools_are_real(&self) -> bool {
        self.tools
            .runtime
            .as_ref()
            .map(|exposure| !exposure.binding().fake)
            .unwrap_or(false)
    }

    /// Dispatch a real tool call through the loop (ACI1).
    ///
    /// Routes the typed request to the matching REAL exposure
    /// (`ToolExposureRequest::Capo` -> the Capo registry,
    /// `ToolExposureRequest::Runtime` -> the runtime wrappers), runs
    /// `authorize_and_invoke`, and persists the canonical tool-call event
    /// sequence keyed to the turn via the core's
    /// [`FakeBoundaryController::dispatch_tool_call`]. The fake summary shim is
    /// unreachable from this path.
    ///
    /// Scope (ACI1): this is the REAL dispatch SEAM and is the only entrypoint
    /// into real tool execution. The autonomous observe->decide->emit turn loop
    /// does not yet auto-select/auto-invoke tools on a model's behalf -- the
    /// loop's per-turn memory-packet summary still uses `ToolExposure::fake()`.
    /// Promoting this seam into the loop's decision step is owned by the later
    /// ACI tasks + `safety-gates`; ACI1 proves the seam is real and driveable.
    pub fn dispatch_tool_call(
        &self,
        scope: &ToolDispatchScope,
        request: ToolExposureRequest,
    ) -> StateResult<ToolDispatchOutcome> {
        let exposure = match &request {
            ToolExposureRequest::Capo(_) => &self.tools.capo,
            ToolExposureRequest::Runtime(_) => self.tools.runtime.as_ref().expect(
                "runtime tool exposure not wired; call RealBoundaryController::with_runtime_tools",
            ),
            ToolExposureRequest::AgentReport(_) => &self.tools.agent_reports,
            ToolExposureRequest::Fake(_) => panic!(
                "RealBoundaryController::dispatch_tool_call refuses fake tool requests; \
                 the real loop dispatches Capo/Runtime/AgentReport tools only"
            ),
        };
        self.core.dispatch_tool_call(exposure, scope, request)
    }

    /// Borrow the Capo registry behind the real exposure, for callers that need
    /// the tool definitions (schemas/scopes) directly.
    pub fn capo_registry(&self) -> Option<&CapoToolRegistry> {
        match &self.tools.capo {
            ToolExposure::Capo(registry) => Some(registry),
            _ => None,
        }
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

    // --- The session-control convenience surface ---------------------------
    //
    // RTL12 parity criterion: the real handle exposes the SAME
    // `redirect`/`interrupt`/`stop` surface the fake handle does, so the
    // identical deterministic suite (`send`/`steer`/`interrupt`/`stop`,
    // restart/replay) runs over both handles. Each delegates to the one core, so
    // a sequence driven through the real handle persists byte-identically to the
    // same sequence driven through the fake handle (proven by
    // `real_controller_passes_the_identical_send_steer_interrupt_stop_suite`).

    pub fn redirect(
        &self,
        registration: &RealAgentRegistration,
        refs: &RealRunRefs,
        goal: &str,
    ) -> StateResult<RealReadModelObservation> {
        self.core.redirect(registration, refs, goal)
    }

    pub fn redirect_agent_name(
        &self,
        agent_name: &str,
        goal: &str,
    ) -> StateResult<RealReadModelObservation> {
        self.core.redirect_agent_name(agent_name, goal)
    }

    pub fn interrupt(
        &self,
        registration: &RealAgentRegistration,
        refs: &RealRunRefs,
        reason: &str,
    ) -> StateResult<RealReadModelObservation> {
        self.core.interrupt(registration, refs, reason)
    }

    pub fn interrupt_agent_name(
        &self,
        agent_name: &str,
        reason: &str,
    ) -> StateResult<RealReadModelObservation> {
        self.core.interrupt_agent_name(agent_name, reason)
    }

    pub fn stop(
        &self,
        registration: &RealAgentRegistration,
        refs: &RealRunRefs,
        reason: &str,
    ) -> StateResult<RealReadModelObservation> {
        self.core.stop(registration, refs, reason)
    }

    pub fn stop_agent_name(
        &self,
        agent_name: &str,
        reason: &str,
    ) -> StateResult<RealReadModelObservation> {
        self.core.stop_agent_name(agent_name, reason)
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
