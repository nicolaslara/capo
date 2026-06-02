//! Server/control-plane boundary for Capo.
//!
//! This crate owns the typed request/response surface that clients should use
//! before choosing a concrete transport such as a local socket or remote API.
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use capo_adapters::{AgentAdapterHandle, ClaudeCodeLiveAdapter, CodexLiveAdapter};
use capo_controller::{FakeBoundaryController, LocalAdapterDispatchRunStart};
use capo_core::{AgentId, CommandIntent, CommandTarget, ProjectId, RunId, SessionId, TaskId};
use capo_state::{
    AdapterDispatchPlanProjection, AdapterDispatchPromptSourceProjection, EventKind, NewEvent,
    ProjectionRecord, RedactionState,
};

mod controller_routing;
mod dashboard;
mod dispatch;
mod event_tail;
mod goal_commands;
mod live_provider;
mod safety_floor;
mod server_core;
mod transport;
mod turn_orchestration;
mod types;
mod util;

use controller_routing::ControllerRoute;
pub use controller_routing::{ControllerSelection, REAL_CONTROLLER_OPT_IN_ENV};
use dispatch::DispatchExecutionOutcome;
pub use event_tail::{EventStream, TailRecvError};
use live_provider::{LiveProviderLocalRunRequest, LiveProviderPreflightRequest};
pub use safety_floor::{
    LIVE_WRITE_OPT_IN_ENV, RunTurnRef, WorkspaceCheckpoint, WorkspaceWriteOutcome,
    WorkspaceWriteRequest, WriteMode, resolve_write_mode, resolve_write_mode_with_env,
};
pub use transport::contract;
pub use transport::{
    CancellationToken, EVENT_TAIL_METHOD, EventNotification, SubscribeStream, TransportError,
    interrupt_frame, send_interrupt, send_tcp, serve_tcp, subscribe_tcp,
};
#[cfg(test)]
pub(crate) use transport::{jsonrpc_request_roundtrip, jsonrpc_response_roundtrip};
pub use turn_orchestration::{
    DispatchTurnMode, DispatchTurnOutcome, DispatchTurnRequest, LiveProviderTurn,
};
pub use types::*;
use util::{
    adapter_label, command_identity_hash, parse_adapter_events, provider_kind_for_adapter, slug,
    stable_hash,
};

const MAX_ADAPTER_FIXTURE_BYTES: usize = 256 * 1024;

/// Maximum events returned in a single subscription catch-up backlog page (ST4).
/// A subscriber reconnecting against a long log reads a bounded page rather than
/// the entire history in one query; it pages by advancing `from_sequence` to the
/// backlog's `next_sequence` and re-subscribing. Generous enough that an ordinary
/// session's whole history fits in one page.
const EVENT_TAIL_BACKLOG_LIMIT: usize = 4096;

#[derive(Clone, Debug)]
pub struct CapoServer {
    project_id: ProjectId,
    controller: FakeBoundaryController,
    controller_selection: ControllerSelection,
    /// AI2: the per-agent Codex chat binding registry + the config a Codex-bound
    /// turn needs.
    codex_chat: CodexChatBindings,
}

/// AI2: the server's record of which registered agents are bound to the real
/// Codex chat adapter, plus the workspace/artifact roots and (optional) program
/// override a Codex-bound `SendTask`/`SteerAgent` turn needs.
///
/// Binding is PER-AGENT, recorded at `RegisterAgent` time, and consulted at chat
/// time. A non-Codex agent never appears here and keeps the shared (fake/default)
/// adapter, so Codex is never a global chat default. The bound set is shared
/// across the server's cloned views (register and chat arrive on different
/// connections of the same server process) behind a [`Mutex`]. The Codex chat
/// turn itself is fail-closed-fast: even a bound agent's turn returns an immediate
/// typed error (no spawn) when [`capo_adapters::codex_live_chat_gate_open`] is off.
/// The real-provider chat binding the per-agent route should use.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RealChatBinding {
    Codex,
    Claude,
}

#[derive(Clone, Debug)]
struct CodexChatBindings {
    /// Codex-bound agent names (AI2).
    bound_agents: Arc<Mutex<HashSet<String>>>,
    /// DP4: Claude-bound agent names (the second real provider).
    claude_bound_agents: Arc<Mutex<HashSet<String>>>,
    codex_workspace_root: PathBuf,
    codex_artifact_root: PathBuf,
    claude_workspace_root: PathBuf,
    claude_artifact_root: PathBuf,
    /// Absolute path to a codex binary (ops `CAPO_CODEX_BIN`; tests a stub). Only
    /// honored when absolute -- the runtime spawns with `env_clear()`.
    program_override: Option<String>,
    /// Absolute path to a claude binary (ops `CAPO_CLAUDE_BIN`; tests a stub).
    /// Only honored when absolute -- the runtime spawns with `env_clear()`.
    claude_program_override: Option<String>,
}

impl CodexChatBindings {
    /// Derive the chat config from the server `state_root`: the Codex one-shot
    /// runs confined to `<state_root>/codex-chat/{workspace,artifacts}`; the
    /// Claude workspace-write one-shot under `<state_root>/claude-chat/...`. An
    /// absolute `CAPO_CODEX_BIN` / `CAPO_CLAUDE_BIN` pins the respective program;
    /// a relative value is ignored (env_clear spawn).
    fn from_state_root(state_root: &Path) -> Self {
        let codex_base = state_root.join("codex-chat");
        let claude_base = state_root.join("claude-chat");
        let program_override = std::env::var("CAPO_CODEX_BIN")
            .ok()
            .filter(|path| Path::new(path).is_absolute());
        let claude_program_override = std::env::var("CAPO_CLAUDE_BIN")
            .ok()
            .filter(|path| Path::new(path).is_absolute());
        Self {
            bound_agents: Arc::new(Mutex::new(HashSet::new())),
            claude_bound_agents: Arc::new(Mutex::new(HashSet::new())),
            codex_workspace_root: codex_base.join("workspace"),
            codex_artifact_root: codex_base.join("artifacts"),
            claude_workspace_root: claude_base.join("workspace"),
            claude_artifact_root: claude_base.join("artifacts"),
            program_override,
            claude_program_override,
        }
    }

    /// Record that `agent_name` is bound to the Codex chat adapter.
    fn bind(&self, agent_name: &str) {
        self.bound_agents
            .lock()
            .expect("codex chat binding lock")
            .insert(agent_name.to_string());
    }

    /// DP4: record that `agent_name` is bound to the Claude chat adapter.
    fn bind_claude(&self, agent_name: &str) {
        self.claude_bound_agents
            .lock()
            .expect("claude chat binding lock")
            .insert(agent_name.to_string());
    }

    /// The real-provider chat binding for `agent_name`, if any.
    fn binding_for(&self, agent_name: &str) -> Option<RealChatBinding> {
        if self
            .bound_agents
            .lock()
            .expect("codex chat binding lock")
            .contains(agent_name)
        {
            return Some(RealChatBinding::Codex);
        }
        if self
            .claude_bound_agents
            .lock()
            .expect("claude chat binding lock")
            .contains(agent_name)
        {
            return Some(RealChatBinding::Claude);
        }
        None
    }

    /// Build the per-agent Codex chat handle. The handle drives the real one-shot
    /// only when the live-provider gate is open; otherwise it fails closed-fast.
    fn codex_handle(&self) -> AgentAdapterHandle {
        let mut adapter = CodexLiveAdapter::new(
            self.codex_workspace_root.clone(),
            self.codex_artifact_root.clone(),
        );
        if let Some(program) = self.program_override.as_deref() {
            adapter = adapter.with_codex_program_override(program);
        }
        AgentAdapterHandle::codex(adapter)
    }

    /// DP4: build the per-agent Claude chat handle. The handle drives the real
    /// workspace-write one-shot only when the Claude live gate is open; otherwise
    /// it fails closed-fast.
    fn claude_handle(&self) -> AgentAdapterHandle {
        let mut adapter = ClaudeCodeLiveAdapter::new(
            self.claude_workspace_root.clone(),
            self.claude_artifact_root.clone(),
        );
        if let Some(program) = self.claude_program_override.as_deref() {
            adapter = adapter.with_claude_program_override(program);
        }
        AgentAdapterHandle::claude(adapter)
    }
}

impl CapoServer {
    /// Open the server with the default routing.
    ///
    /// After the RTL12 cutover the default is [`ControllerSelection::Real`]: the
    /// real controller passes the parity suite, so default chat/steer now route
    /// through it. The [`REAL_CONTROLLER_OPT_IN_ENV`] env gate is honored here as
    /// the single rollback knob -- setting `CAPO_SERVER_REAL_CONTROLLER=0` forces
    /// the fake routing back on without scattering the decision across call
    /// sites.
    pub fn open(project_id: ProjectId, state_root: impl AsRef<Path>) -> ServerResult<Self> {
        Self::open_with_controller(project_id, state_root, ControllerSelection::from_env())
    }

    /// Open the server with an explicit [`ControllerSelection`] -- the single
    /// typed switch (RTL11) that routes `SendTask`/`SteerAgent` and the rest of
    /// the command surface through either the fake or the real controller. The
    /// orchestration core is one [`FakeBoundaryController`]; the real routing is
    /// a zero-cost view over it (see `controller_routing.rs`).
    pub fn open_with_controller(
        project_id: ProjectId,
        state_root: impl AsRef<Path>,
        controller_selection: ControllerSelection,
    ) -> ServerResult<Self> {
        let codex_chat = CodexChatBindings::from_state_root(state_root.as_ref());
        let controller = FakeBoundaryController::open(project_id.clone(), state_root)
            .map_err(ServerError::State)?;
        Ok(Self {
            project_id,
            controller,
            controller_selection,
            codex_chat,
        })
    }

    /// Open the server with an explicit [`ControllerSelection`] and an injected
    /// [`AgentAdapterHandle`].
    ///
    /// This is the adapter-injection seam for RTL12/RTL13: the default
    /// `open`/`open_with_controller` build the core with the default
    /// ([`AgentAdapterHandle::fake`]) adapter, so with that core the `Real`
    /// selection cannot observably differ from `Fake` -- both views drive the
    /// same fake-backed core. This constructor instead builds the one shared
    /// orchestration core over `adapter`, so a scripted-mock handle backs the
    /// deterministic parity suites and a real Codex/Claude/ACP handle plugs in
    /// unchanged. Because the core is the real control flow and the `Real`
    /// routing is a view over it, the injected adapter backs BOTH the routed
    /// command surface and the (shared-core) loop ingestion, giving RTL12's
    /// parity suite and RTL13's live smoke a server-level seam to drive a
    /// genuinely-real controller through the switch.
    pub fn open_with_controller_and_adapter(
        project_id: ProjectId,
        state_root: impl AsRef<Path>,
        controller_selection: ControllerSelection,
        adapter: AgentAdapterHandle,
    ) -> ServerResult<Self> {
        let codex_chat = CodexChatBindings::from_state_root(state_root.as_ref());
        let controller =
            FakeBoundaryController::open_with_adapter(project_id.clone(), state_root, adapter)
                .map_err(ServerError::State)?;
        Ok(Self {
            project_id,
            controller,
            controller_selection,
            codex_chat,
        })
    }

    /// The controller routing in effect (the RTL11 single-switch value).
    pub fn controller_selection(&self) -> ControllerSelection {
        self.controller_selection
    }

    /// The command-routing view bound to the selected controller. Command
    /// handling (`register`/`send`/`steer`/`interrupt`/`stop`/`recover`) flows
    /// through this; state/dispatch/projection helpers continue to use the one
    /// orchestration core directly, since those persist identically regardless
    /// of which handle drove the command.
    fn command_controller(&self) -> ControllerRoute<'_> {
        ControllerRoute::new(self.controller_selection, &self.controller)
    }

    /// AI2: the chat-routing view for `agent_name`'s `SendTask`/`SteerAgent`.
    ///
    /// If the agent registered with `--adapter codex` it is in the per-agent
    /// Codex binding registry, so its chat turn routes through a Codex-bound view
    /// of the shared core (the real read-only one-shot when the live-provider gate
    /// is open; an immediate fail-closed-fast typed error when it is off). Every
    /// other (fake/default) agent routes through the ordinary
    /// [`Self::command_controller`], so Codex is never a global chat default.
    fn chat_controller(&self, agent_name: &str) -> ControllerRoute<'_> {
        match self.codex_chat.binding_for(agent_name) {
            Some(RealChatBinding::Codex) => ControllerRoute::new_codex_bound(
                self.controller_selection,
                &self.controller,
                self.codex_chat.codex_handle(),
            ),
            // DP4: a Claude-bound agent routes through the SAME binding-respecting
            // chat route (generic over the per-agent `AgentAdapterHandle`), so its
            // chat turn drives the real Claude workspace-write handle while every
            // other agent keeps the shared adapter; fail-closed-fast when the gate
            // is off, identical to Codex.
            Some(RealChatBinding::Claude) => ControllerRoute::new_codex_bound(
                self.controller_selection,
                &self.controller,
                self.codex_chat.claude_handle(),
            ),
            None => self.command_controller(),
        }
    }

    pub fn handle(&self, request: ServerRequest) -> ServerResult<ServerResponse> {
        let request_id = request.request_id.clone();
        let origin = request.origin.clone();
        match request.command {
            ServerCommand::RegisterAgent { name, adapter } => {
                // AI2/DP4: resolve the agent's chat adapter binding. `fake`
                // (default) keeps the deterministic adapter; `codex` binds the
                // real Codex chat handle and `claude` binds the real Claude
                // workspace-write handle for THIS agent only (fail-closed-fast on
                // chat). Anything else is rejected before the agent is created.
                let chat_binding = match adapter.as_str() {
                    "fake" => None,
                    "codex" => Some(RealChatBinding::Codex),
                    "claude" => Some(RealChatBinding::Claude),
                    other => {
                        return Err(ServerError::UnsupportedChatAdapter {
                            adapter: other.to_string(),
                        });
                    }
                };
                let command_hash =
                    command_identity_hash(format!("register_agent:{name}:{adapter}"));
                let command = self.command_envelope(
                    &request_id,
                    &origin,
                    &command_hash,
                    CommandTarget::Project(self.project_id.clone()),
                    CommandIntent::RegisterAgent,
                    Some(name),
                );
                let registration = self
                    .command_controller()
                    .register_agent_command(&command)
                    .map_err(ServerError::State)?;
                match chat_binding {
                    Some(RealChatBinding::Codex) => {
                        self.codex_chat.bind(&registration.agent_name);
                    }
                    Some(RealChatBinding::Claude) => {
                        self.codex_chat.bind_claude(&registration.agent_name);
                    }
                    None => {}
                }
                self.record_server_request_handled(&command, &origin, "register_agent", None, None)
                    .map_err(ServerError::State)?;
                self.response(
                    request_id,
                    origin,
                    ServerResponsePayload::AgentRegistered(AgentSummary {
                        agent_id: registration.agent_id,
                        name: registration.agent_name,
                        status: "available".to_string(),
                        current_session_id: None,
                        session: None,
                    }),
                )
            }
            ServerCommand::RegisterRuntimeTarget {
                runtime_target_id,
                name,
                runner_kind,
                workspace_root,
                artifact_root,
                default_cwd,
                capability_profile_id,
                connectivity_endpoint_id,
                status,
            } => {
                // DT1 (DT-D1): the runner ANNOUNCED over this transport; the
                // server (single writer) appends `runtime.target_registered`. The
                // endpoint is carried by handle only -- no raw address/credential
                // ever reaches the log. Idempotent on the target id, so a
                // re-announce on reconnect does not duplicate the target.
                //
                // Review finding 6: this command can arrive from a remote runner
                // speaking JSON-RPC directly, bypassing the CLI's
                // `parse_runtime_runner_kind` / `parse_runtime_target_status`. The
                // server is the single writer to the authoritative log, so it
                // re-validates `runner_kind` / `status` against the SAME closed
                // vocabularies here, BEFORE appending -- a raw-TCP caller cannot
                // inject an arbitrary string into `runtime.target_registered`.
                if !matches!(
                    runner_kind.as_str(),
                    "local-process" | "remote-process" | "container"
                ) {
                    return Err(ServerError::InvalidRuntimeTargetField {
                        field: "runner_kind",
                        value: runner_kind,
                        expected: "local-process, remote-process, or container",
                    });
                }
                if !matches!(status.as_str(), "available" | "disabled" | "unhealthy") {
                    return Err(ServerError::InvalidRuntimeTargetField {
                        field: "status",
                        value: status,
                        expected: "available, disabled, or unhealthy",
                    });
                }
                let (target, sequence) = self
                    .controller
                    .register_runtime_target(
                        &runtime_target_id,
                        &name,
                        &runner_kind,
                        &workspace_root,
                        &artifact_root,
                        &default_cwd,
                        &capability_profile_id,
                        connectivity_endpoint_id.as_deref(),
                        &status,
                    )
                    .map_err(ServerError::State)?;
                let command_hash = command_identity_hash(format!(
                    "register_runtime_target:{runtime_target_id}:{runner_kind}"
                ));
                let command = self.command_envelope(
                    &request_id,
                    &origin,
                    &command_hash,
                    CommandTarget::Project(self.project_id.clone()),
                    CommandIntent::RegisterAgent,
                    Some(target.name.clone()),
                );
                self.record_server_request_handled(
                    &command,
                    &origin,
                    "register_runtime_target",
                    None,
                    Some(serde_json::json!({
                        "runtime_target_id": target.runtime_target_id,
                        "runner_kind": target.runner_kind,
                        "connectivity_endpoint_id": target.connectivity_endpoint_id,
                        "announce_source": "runner_jsonrpc",
                    })),
                )
                .map_err(ServerError::State)?;
                self.response(
                    request_id,
                    origin,
                    ServerResponsePayload::RuntimeTargetRegistered(ServerRuntimeTargetSummary {
                        runtime_target_id: target.runtime_target_id,
                        name: target.name,
                        runner_kind: target.runner_kind,
                        status: target.status,
                        connectivity_endpoint_id: target.connectivity_endpoint_id,
                        sequence,
                    }),
                )
            }
            ServerCommand::SendTask {
                agent_name,
                goal,
                scenario,
            } => {
                let command_hash =
                    command_identity_hash(format!("send_task:{agent_name}:{goal}:{scenario}"));
                if let Some(agent) = self.agent_by_name(&agent_name)? {
                    if let Some(session) = agent.session {
                        return Err(ServerError::AgentAlreadyHasSession {
                            agent_name,
                            session_id: session.session_id.to_string(),
                            run_status: session.run_status,
                        });
                    }
                } else {
                    return Err(ServerError::UnknownAgent { agent_name });
                }
                let mut command = self.command_envelope(
                    &request_id,
                    &origin,
                    &command_hash,
                    CommandTarget::Agent(AgentId::new(format!("agent-{agent_name}"))),
                    CommandIntent::SendTask,
                    Some(goal),
                );
                // AI2: pick the chat route by the agent's binding BEFORE moving the
                // name into the command args -- a Codex-bound agent drives the real
                // Codex handle; every other agent keeps the default adapter.
                let chat_controller = self.chat_controller(&agent_name);
                command
                    .structured_args
                    .push(("agent".to_string(), agent_name));
                command
                    .structured_args
                    .push(("scenario".to_string(), scenario));
                let run = chat_controller
                    .send_task_command(&command)
                    .map_err(ServerError::State)?;
                self.record_server_request_handled(
                    &command,
                    &origin,
                    "send_task",
                    Some(&run),
                    None,
                )
                .map_err(ServerError::State)?;
                self.response(
                    request_id,
                    origin,
                    ServerResponsePayload::TaskSent(TaskRunSummary::from_run_refs(run)),
                )
            }
            ServerCommand::SteerAgent { agent_name, goal } => {
                let agent =
                    self.agent_by_name(&agent_name)?
                        .ok_or_else(|| ServerError::UnknownAgent {
                            agent_name: agent_name.clone(),
                        })?;
                let session =
                    agent
                        .session
                        .ok_or_else(|| ServerError::AgentHasNoActiveSession {
                            agent_name: agent_name.clone(),
                        })?;
                let run_id =
                    session
                        .run_id
                        .ok_or_else(|| ServerError::AgentHasNoActiveSession {
                            agent_name: agent_name.clone(),
                        })?;
                let (_, _, _, refs) =
                    self.run_refs_for_session_run(&session.session_id, &run_id)?;
                let goal_hash = stable_hash(goal.as_bytes());
                let command_hash =
                    command_identity_hash(format!("steer_agent:{agent_name}:{goal_hash}"));
                let mut command = self.command_envelope(
                    &request_id,
                    &origin,
                    &command_hash,
                    CommandTarget::Agent(AgentId::new(format!("agent-{agent_name}"))),
                    CommandIntent::RedirectSession,
                    Some(goal),
                );
                command
                    .structured_args
                    .push(("agent".to_string(), agent_name.clone()));
                // AI2: SteerAgent routes by the agent's binding too -- a Codex-bound
                // agent's steer turn drives the real Codex handle (fail-closed-fast
                // when the live-provider gate is off); others keep the default.
                self.chat_controller(&agent_name)
                    .redirect_command(&command)
                    .map_err(ServerError::State)?;
                self.record_server_request_handled(
                    &command,
                    &origin,
                    "steer_agent",
                    Some(&refs),
                    Some(serde_json::json!({
                        "goal_hash": goal_hash,
                        "raw_goal_policy": "not_rendered"
                    })),
                )
                .map_err(ServerError::State)?;
                let agent = self
                    .agent_by_name(&agent_name)?
                    .ok_or(ServerError::UnknownAgent { agent_name })?;
                self.response(
                    request_id,
                    origin,
                    ServerResponsePayload::AgentStatus(agent),
                )
            }
            ServerCommand::InterruptAgent { agent_name, reason } => {
                let agent =
                    self.agent_by_name(&agent_name)?
                        .ok_or_else(|| ServerError::UnknownAgent {
                            agent_name: agent_name.clone(),
                        })?;
                let session =
                    agent
                        .session
                        .ok_or_else(|| ServerError::AgentHasNoActiveSession {
                            agent_name: agent_name.clone(),
                        })?;
                let run_id =
                    session
                        .run_id
                        .ok_or_else(|| ServerError::AgentHasNoActiveSession {
                            agent_name: agent_name.clone(),
                        })?;
                let (_, _, _, refs) =
                    self.run_refs_for_session_run(&session.session_id, &run_id)?;
                let reason_hash = stable_hash(reason.as_bytes());
                let command_hash =
                    command_identity_hash(format!("interrupt_agent:{agent_name}:{reason_hash}"));
                let mut command = self.command_envelope(
                    &request_id,
                    &origin,
                    &command_hash,
                    CommandTarget::Agent(AgentId::new(format!("agent-{agent_name}"))),
                    CommandIntent::InterruptSession,
                    Some(reason),
                );
                command
                    .structured_args
                    .push(("agent".to_string(), agent_name.clone()));
                self.command_controller()
                    .interrupt_command(&command)
                    .map_err(ServerError::State)?;
                self.record_server_request_handled(
                    &command,
                    &origin,
                    "interrupt_agent",
                    Some(&refs),
                    Some(serde_json::json!({
                        "reason_hash": reason_hash,
                        "raw_reason_policy": "not_rendered"
                    })),
                )
                .map_err(ServerError::State)?;
                let agent = self
                    .agent_by_name(&agent_name)?
                    .ok_or(ServerError::UnknownAgent { agent_name })?;
                self.response(
                    request_id,
                    origin,
                    ServerResponsePayload::AgentStatus(agent),
                )
            }
            ServerCommand::StopAgent { agent_name, reason } => {
                let agent =
                    self.agent_by_name(&agent_name)?
                        .ok_or_else(|| ServerError::UnknownAgent {
                            agent_name: agent_name.clone(),
                        })?;
                let session =
                    agent
                        .session
                        .ok_or_else(|| ServerError::AgentHasNoActiveSession {
                            agent_name: agent_name.clone(),
                        })?;
                let run_id =
                    session
                        .run_id
                        .ok_or_else(|| ServerError::AgentHasNoActiveSession {
                            agent_name: agent_name.clone(),
                        })?;
                let (_, _, _, refs) =
                    self.run_refs_for_session_run(&session.session_id, &run_id)?;
                let reason_hash = stable_hash(reason.as_bytes());
                let command_hash =
                    command_identity_hash(format!("stop_agent:{agent_name}:{reason_hash}"));
                let mut command = self.command_envelope(
                    &request_id,
                    &origin,
                    &command_hash,
                    CommandTarget::Agent(AgentId::new(format!("agent-{agent_name}"))),
                    CommandIntent::InterruptSession,
                    Some(reason),
                );
                command
                    .structured_args
                    .push(("agent".to_string(), agent_name.clone()));
                self.command_controller()
                    .stop_command(&command)
                    .map_err(ServerError::State)?;
                self.record_server_request_handled(
                    &command,
                    &origin,
                    "stop_agent",
                    Some(&refs),
                    Some(serde_json::json!({
                        "reason_hash": reason_hash,
                        "raw_reason_policy": "not_rendered"
                    })),
                )
                .map_err(ServerError::State)?;
                let agent = self
                    .agent_by_name(&agent_name)?
                    .ok_or(ServerError::UnknownAgent { agent_name })?;
                self.response(
                    request_id,
                    origin,
                    ServerResponsePayload::AgentStatus(agent),
                )
            }
            ServerCommand::ListAgents => {
                let agents = self.dashboard_snapshot()?.agents;
                self.response(request_id, origin, ServerResponsePayload::Agents(agents))
            }
            ServerCommand::AgentStatus { agent_name } => {
                let agent = self
                    .dashboard_snapshot()?
                    .agents
                    .into_iter()
                    .find(|agent| agent.name == agent_name)
                    .ok_or(ServerError::UnknownAgent { agent_name })?;
                self.response(
                    request_id,
                    origin,
                    ServerResponsePayload::AgentStatus(agent),
                )
            }
            ServerCommand::Dashboard { recent_event_limit } => self.response(
                request_id,
                origin,
                ServerResponsePayload::Dashboard(self.dashboard_with_limit(recent_event_limit)?),
            ),
            ServerCommand::StartSession {
                agent_name,
                goal,
                adapter,
                session_id,
                run_id,
            } => {
                let command_hash = command_identity_hash(format!(
                    "start_session:{agent_name}:{goal}:{adapter}:{}:{}",
                    session_id.as_deref().unwrap_or(""),
                    run_id.as_deref().unwrap_or("")
                ));
                let agent = self
                    .controller
                    .state()
                    .agent_by_name(&agent_name)
                    .map_err(ServerError::State)?
                    .ok_or_else(|| ServerError::UnknownAgent {
                        agent_name: agent_name.clone(),
                    })?;
                if let Some(current_session_id) = agent.current_session_id.as_ref()
                    && let Some(current_run) = self
                        .controller
                        .state()
                        .run_for_session(current_session_id)
                        .map_err(ServerError::State)?
                    && current_run.status == "running"
                {
                    return Err(ServerError::AgentAlreadyHasSession {
                        agent_name: agent_name.clone(),
                        session_id: current_session_id.to_string(),
                        run_status: Some(current_run.status),
                    });
                }
                let adapter_label = adapter_label(&adapter)?.to_string();
                let session_id = session_id.map(SessionId::new).unwrap_or_else(|| {
                    SessionId::new(format!(
                        "session-{}-{}",
                        slug(&agent_name),
                        stable_hash(format!("{request_id}:{goal}").as_bytes())
                    ))
                });
                if self
                    .controller
                    .state()
                    .session(&session_id)
                    .map_err(ServerError::State)?
                    .is_some()
                {
                    return Err(ServerError::SessionAlreadyExists {
                        session_id: session_id.to_string(),
                    });
                }
                let run_id = run_id
                    .map(RunId::new)
                    .unwrap_or_else(|| RunId::new(format!("run-{}", session_id)));
                if self
                    .controller
                    .state()
                    .run(&run_id)
                    .map_err(ServerError::State)?
                    .is_some()
                {
                    return Err(ServerError::RunAlreadyExists {
                        run_id: run_id.to_string(),
                    });
                }
                let task_id = TaskId::new(format!("task-{}", session_id));
                let external_session_ref = format!("server-adapter-session-{}", session_id);
                let runtime_process_ref = format!("server-session-runtime-{}", run_id);
                let goal_hash = stable_hash(goal.as_bytes());
                let stored_goal_ref = format!("goal_hash:{goal_hash};raw_policy:not_rendered");
                let run = self
                    .controller
                    .prepare_local_adapter_dispatch_run(LocalAdapterDispatchRunStart {
                        agent_name: agent_name.clone(),
                        task_id,
                        session_id: session_id.clone(),
                        run_id: run_id.clone(),
                        goal: stored_goal_ref.clone(),
                        runtime_process_ref,
                        external_session_ref,
                        provider_cli_executed: false,
                        adapter_kind: adapter_label.clone(),
                    })
                    .map_err(ServerError::State)?;
                let command = self.command_envelope(
                    &request_id,
                    &origin,
                    &command_hash,
                    CommandTarget::Agent(agent.agent_id),
                    CommandIntent::StartSession,
                    Some(stored_goal_ref),
                );
                self.record_server_request_handled(
                    &command,
                    &origin,
                    "start_session",
                    Some(&run),
                    Some(serde_json::json!({
                        "adapter": adapter_label,
                        "provider_cli_executed": false,
                        "session_start_kind": "server_native",
                        "goal_hash": goal_hash,
                        "raw_goal_policy": "not_rendered",
                    })),
                )
                .map_err(ServerError::State)?;
                self.response(
                    request_id,
                    origin,
                    ServerResponsePayload::SessionStarted(TaskRunSummary::from_run_refs(run)),
                )
            }
            ServerCommand::ReplayAdapterFixture {
                adapter,
                session_id,
                run_id,
                turn_id,
                fixture_name,
                fixture_jsonl,
            } => {
                if fixture_jsonl.len() > MAX_ADAPTER_FIXTURE_BYTES {
                    return Err(ServerError::AdapterFixture(format!(
                        "adapter fixture is too large: {} bytes > {} bytes",
                        fixture_jsonl.len(),
                        MAX_ADAPTER_FIXTURE_BYTES
                    )));
                }
                let requested_adapter = adapter_label(&adapter)?.to_string();
                let fixture_hash = stable_hash(fixture_jsonl.as_bytes());
                let command_hash = command_identity_hash(format!(
                    "replay_adapter_fixture:{requested_adapter}:{session_id}:{run_id}:{turn_id}:{fixture_name}:{fixture_hash}"
                ));
                let session_id = SessionId::new(session_id);
                let run_id = RunId::new(run_id);
                let (session, run_projection, agent, run) =
                    self.run_refs_for_session_run(&session_id, &run_id)?;
                self.require_session_adapter(&session.session_id, &requested_adapter)?;
                let adapter_events = parse_adapter_events(&requested_adapter, &fixture_jsonl)
                    .map_err(ServerError::AdapterFixture)?;
                if adapter_events.is_empty() {
                    return Err(ServerError::AdapterFixture(
                        "adapter fixture produced no normalized events".to_string(),
                    ));
                }
                let command = self.command_envelope(
                    &request_id,
                    &origin,
                    &command_hash,
                    CommandTarget::Agent(agent.agent_id.clone()),
                    CommandIntent::SendTask,
                    Some(session.current_goal.clone()),
                );
                let report = self
                    .controller
                    .apply_normalized_adapter_events_with_turn(
                        &run,
                        &adapter_events,
                        Some(&turn_id),
                    )
                    .map_err(ServerError::State)?;
                self.record_server_request_handled(
                    &command,
                    &origin,
                    "replay_adapter_fixture",
                    Some(&run),
                    Some(serde_json::json!({
                        "adapter": requested_adapter,
                        "fixture_name": fixture_name,
                        "fixture_hash": fixture_hash,
                        "provider_cli_executed": false,
                        "raw_content_policy": "content_hashed_not_rendered",
                        "raw_fixture_body_persisted": false,
                        "raw_fixture_transport_scope": "local_loopback_request_only",
                        "target_session_id": run.session_id.to_string(),
                        "target_run_id": run.run_id.to_string(),
                        "target_turn_id": turn_id,
                        "run_status_before_replay": run_projection.status,
                    })),
                )
                .map_err(ServerError::State)?;
                self.response(
                    request_id,
                    origin,
                    ServerResponsePayload::AdapterFixtureReplayed(AdapterReplaySummary {
                        adapter: requested_adapter,
                        fixture_name,
                        fixture_hash,
                        agent_name: agent.name,
                        task_id: run.task_id,
                        session_id: run.session_id,
                        run_id: run.run_id,
                        turn_id,
                        provider_cli_executed: false,
                        raw_content_policy: "content_hashed_not_rendered".to_string(),
                        input_event_count: report.input_event_count,
                        appended_event_count: report.appended_event_count,
                        tool_event_count: report.tool_event_count,
                        summary_event_count: report.summary_event_count,
                        completed_turn_count: report.completed_turn_count,
                    }),
                )
            }
            ServerCommand::PlanDispatch {
                agent_name,
                adapter,
                goal,
                workspace,
                artifacts,
                session_id,
                run_id,
                turn_id,
                deterministic_opt_in,
            } => {
                let adapter_label = adapter_label(&adapter)?.to_string();
                let session_id = SessionId::new(session_id);
                let run_id = RunId::new(run_id);
                let (_session, _run_projection, agent, _run) =
                    self.run_refs_for_session_run(&session_id, &run_id)?;
                self.require_session_adapter(&session_id, &adapter_label)?;
                if agent.name != agent_name {
                    return Err(ServerError::AdapterFixture(format!(
                        "dispatch agent mismatch: session belongs to {}, requested {}",
                        agent.name, agent_name
                    )));
                }
                let goal_hash = stable_hash(goal.as_bytes());
                let target_hash = stable_hash(
                    format!("{agent_name}:{adapter_label}:{session_id}:{run_id}:{turn_id}:{workspace}:{artifacts}")
                        .as_bytes(),
                );
                let dispatch_plan_id =
                    format!("server-dispatch-plan-{adapter_label}-{goal_hash}-{target_hash}");
                let prompt_source_id = format!(
                    "server-dispatch-prompt-source-{}",
                    stable_hash(dispatch_plan_id.as_bytes())
                );
                let plan = AdapterDispatchPlanProjection {
                    dispatch_plan_id: dispatch_plan_id.clone(),
                    project_id: self.project_id.clone(),
                    adapter_kind: adapter_label.clone(),
                    provider_kind: provider_kind_for_adapter(&adapter_label).to_string(),
                    credential_scope: "subscription_cli".to_string(),
                    agent_id: agent.agent_id.clone(),
                    agent_name: agent.name.clone(),
                    session_id: session_id.clone(),
                    run_id: run_id.clone(),
                    runtime_program: "deterministic-fixture-runtime".to_string(),
                    runtime_arg_count: 1,
                    runtime_prompt_policy: "not_rendered".to_string(),
                    runtime_cwd: workspace.clone(),
                    artifact_root: artifacts.clone(),
                    request_env_count: usize::from(deterministic_opt_in) as i64,
                    env_allowlist_count: usize::from(deterministic_opt_in) as i64,
                    redaction_rule_count: 1,
                    stdout_format: "jsonl".to_string(),
                    stderr_policy: "bounded_redacted_artifact".to_string(),
                    provider_cli_executed: false,
                    status: "planned".to_string(),
                    updated_sequence: 0,
                };
                let prompt_source = AdapterDispatchPromptSourceProjection {
                    prompt_source_id: prompt_source_id.clone(),
                    project_id: self.project_id.clone(),
                    dispatch_plan_id: dispatch_plan_id.clone(),
                    prompt_hash: goal_hash.clone(),
                    source_kind: "server_inline_goal".to_string(),
                    source_ref: Some(format!("server-dispatch-turn:{turn_id}")),
                    source_hash: Some(goal_hash.clone()),
                    materialization_status: "server_replayable_goal_hash".to_string(),
                    raw_prompt_policy: "not_rendered".to_string(),
                    updated_sequence: 0,
                };
                let event = NewEvent {
                    event_id: format!(
                        "event-server-dispatch-plan-{}",
                        stable_hash(dispatch_plan_id.as_bytes())
                    ),
                    kind: EventKind::AdapterDispatchPlanned,
                    actor: origin.actor_id.clone(),
                    project_id: Some(self.project_id.clone()),
                    task_id: None,
                    agent_id: Some(agent.agent_id.clone()),
                    session_id: Some(session_id.clone()),
                    run_id: Some(run_id.clone()),
                    turn_id: Some(turn_id.clone()),
                    item_id: Some(dispatch_plan_id.clone()),
                    payload_json: serde_json::json!({
                        "dispatch_plan_id": dispatch_plan_id,
                        "adapter": adapter_label,
                        "agent": agent.name,
                        "target_turn_id": turn_id,
                        "runtime_prompt_policy": "not_rendered",
                        "provider_cli_executed": false,
                        "raw_prompt_policy": "not_rendered",
                        "deterministic_opt_in": deterministic_opt_in,
                    })
                    .to_string(),
                    idempotency_key: Some(format!(
                        "server-dispatch-plan:{}:{}:{}:{}:{}",
                        self.project_id, session_id, run_id, turn_id, target_hash
                    )),
                    redaction_state: RedactionState::Safe,
                };
                self.controller
                    .state()
                    .append_event(
                        event,
                        &[
                            ProjectionRecord::AdapterDispatchPlan(plan.clone()),
                            ProjectionRecord::AdapterDispatchPromptSource(prompt_source.clone()),
                        ],
                    )
                    .map_err(ServerError::State)?;
                let command_hash =
                    command_identity_hash(format!("plan_dispatch:{dispatch_plan_id}"));
                let command = self.command_envelope(
                    &request_id,
                    &origin,
                    &command_hash,
                    CommandTarget::Session(session_id),
                    CommandIntent::SendTask,
                    Some(goal),
                );
                self.record_server_request_handled(
                    &command,
                    &origin,
                    "plan_dispatch",
                    None,
                    Some(serde_json::json!({
                        "dispatch_plan_id": plan.dispatch_plan_id,
                        "prompt_source_id": prompt_source.prompt_source_id,
                        "target_run_id": run_id.to_string(),
                        "target_turn_id": turn_id,
                        "provider_cli_executed": false,
                        "deterministic_opt_in": deterministic_opt_in,
                    })),
                )
                .map_err(ServerError::State)?;
                self.response(
                    request_id,
                    origin,
                    ServerResponsePayload::DispatchPlanned(DispatchPlanSummary::from_projection(
                        &plan,
                        &prompt_source,
                    )),
                )
            }
            ServerCommand::PreflightLiveProvider {
                agent_name,
                adapter,
                goal,
                workspace,
                artifacts,
                session_id,
                run_id,
                turn_id,
                capability_profile,
                runtime_scope,
                credential_scan_policy,
                raw_prompt_policy,
                raw_output_policy,
                tool_wrapper_policy,
                live_provider_opt_in,
            } => {
                let summary = self.preflight_live_provider(
                    &origin,
                    LiveProviderPreflightRequest {
                        request_id: &request_id,
                        agent_name: &agent_name,
                        adapter: &adapter,
                        goal: &goal,
                        workspace: &workspace,
                        artifacts: &artifacts,
                        session_id: &session_id,
                        run_id: &run_id,
                        turn_id: &turn_id,
                        capability_profile: &capability_profile,
                        runtime_scope: &runtime_scope,
                        credential_scan_policy: &credential_scan_policy,
                        raw_prompt_policy: &raw_prompt_policy,
                        raw_output_policy: &raw_output_policy,
                        tool_wrapper_policy: &tool_wrapper_policy,
                        live_provider_opt_in,
                    },
                )?;
                self.response(
                    request_id,
                    origin,
                    ServerResponsePayload::LiveProviderPreflighted(summary),
                )
            }
            ServerCommand::RunLiveProviderLocal {
                dispatch_plan_id,
                goal,
                live_execution_opt_in,
                mock_runtime_opt_in,
                mock_provider_output_name,
                mock_provider_output_jsonl,
                timeout_seconds,
                codex_program_override,
                unattended,
            } => {
                // Spawn-path codex-binary override: prefer the explicit command
                // field (threaded in-process by the loop / tests); otherwise fall
                // back to an absolute `CAPO_CODEX_BIN` so ops can pin an exact
                // codex build for a live smoke. A bare/relative value is ignored
                // downstream (the runtime spawns with `env_clear()`).
                let codex_program_override = codex_program_override.or_else(|| {
                    std::env::var("CAPO_CODEX_BIN")
                        .ok()
                        .filter(|path| std::path::Path::new(path.trim()).is_absolute())
                });
                // CS5: the dispatch-side Claude spawn override. Like the Codex
                // one, ops pin an absolute `CAPO_CLAUDE_BIN` so a live Claude
                // smoke can run an exact `claude` build; a bare/relative value is
                // ignored downstream (the runtime spawns with `env_clear()`). This
                // dispatch seam is distinct from the chat-side `CAPO_CLAUDE_BIN`
                // consumed by `claude_live.rs`.
                let claude_program_override = std::env::var("CAPO_CLAUDE_BIN")
                    .ok()
                    .filter(|path| std::path::Path::new(path.trim()).is_absolute());
                // RTL9: resolve the write mode through the RTL6 gate. A live
                // workspace write requires the caller opt-in AND
                // `CAPO_SERVER_RUN_CODEX_LIVE` AND an attended run; anything short
                // of all three stays read-only/dry-run. The mock-output path never
                // spawns a provider, so its profile is irrelevant.
                let write_mode = resolve_write_mode(live_execution_opt_in, unattended);
                let summary = self.run_live_provider_local(
                    &origin,
                    LiveProviderLocalRunRequest {
                        dispatch_plan_id: &dispatch_plan_id,
                        goal: &goal,
                        live_execution_opt_in,
                        mock_runtime_opt_in,
                        mock_provider_output_name: mock_provider_output_name.as_deref(),
                        mock_provider_output_jsonl: mock_provider_output_jsonl.as_deref(),
                        timeout_seconds,
                        codex_program_override: codex_program_override.as_deref().map(str::trim),
                        claude_program_override: claude_program_override.as_deref().map(str::trim),
                        write_mode,
                        record_selected_argv: None,
                    },
                )?;
                let command_hash = command_identity_hash(format!(
                    "run_live_provider_local:{}:{}:{}",
                    dispatch_plan_id,
                    stable_hash(goal.as_bytes()),
                    summary.dispatch_execution_id
                ));
                let command = self.command_envelope(
                    &request_id,
                    &origin,
                    &command_hash,
                    CommandTarget::Session(summary.session_id.clone()),
                    CommandIntent::SendTask,
                    Some(goal),
                );
                self.record_server_request_handled(
                    &command,
                    &origin,
                    "run_live_provider_local",
                    None,
                    Some(serde_json::json!({
                        "dispatch_plan_id": summary.dispatch_plan_id,
                        "dispatch_execution_id": summary.dispatch_execution_id,
                        "provider_cli_execution_allowed": summary.provider_cli_execution_allowed,
                        "provider_cli_executed": summary.provider_cli_executed,
                        "status": summary.status,
                        "credential_scan_status": summary.credential_scan_status,
                        "raw_prompt_policy": summary.raw_prompt_policy,
                        "raw_output_policy": summary.raw_output_policy,
                        "reason_codes": summary.reason_codes,
                    })),
                )
                .map_err(ServerError::State)?;
                self.response(
                    request_id,
                    origin,
                    ServerResponsePayload::DispatchRun(summary),
                )
            }
            ServerCommand::GateDispatch { dispatch_plan_id } => {
                let (plan, prompt_source) = self.dispatch_plan_with_prompt(&dispatch_plan_id)?;
                let gate = self.dispatch_gate_for_plan(&plan);
                self.append_dispatch_gate(&origin, &plan, &gate)?;
                let materialization = self.dispatch_prompt_materialization(&prompt_source);
                self.append_prompt_materialization(&origin, &plan, &materialization)?;
                let execution_request = self.dispatch_execution_request(&plan, &gate);
                self.append_execution_request(&origin, &plan, &execution_request)?;
                let command_hash =
                    command_identity_hash(format!("gate_dispatch:{dispatch_plan_id}"));
                let command = self.command_envelope(
                    &request_id,
                    &origin,
                    &command_hash,
                    CommandTarget::Session(plan.session_id.clone()),
                    CommandIntent::SendTask,
                    None,
                );
                self.record_server_request_handled(
                    &command,
                    &origin,
                    "gate_dispatch",
                    None,
                    Some(serde_json::json!({
                        "dispatch_plan_id": plan.dispatch_plan_id,
                        "dispatch_gate_id": gate.dispatch_gate_id,
                        "execution_request_id": execution_request.execution_request_id,
                        "materialization_id": materialization.materialization_id,
                        "provider_cli_execution_allowed": gate.provider_cli_execution_allowed,
                        "provider_cli_executed": false,
                    })),
                )
                .map_err(ServerError::State)?;
                self.response(
                    request_id,
                    origin,
                    ServerResponsePayload::DispatchGated(DispatchGateSummary {
                        dispatch_plan_id: gate.dispatch_plan_id,
                        dispatch_gate_id: gate.dispatch_gate_id,
                        execution_request_id: execution_request.execution_request_id,
                        materialization_id: materialization.materialization_id,
                        adapter: gate.adapter_kind,
                        provider_cli_execution_allowed: gate.provider_cli_execution_allowed,
                        provider_cli_executed: false,
                        status: gate.status,
                        reasons: gate.reason_codes,
                        raw_prompt_policy: materialization.raw_prompt_policy,
                    }),
                )
            }
            ServerCommand::RunDispatchLocal {
                dispatch_plan_id,
                fixture_name,
                fixture_jsonl,
            } => {
                if fixture_jsonl.len() > MAX_ADAPTER_FIXTURE_BYTES {
                    return Err(ServerError::AdapterFixture(format!(
                        "adapter fixture is too large: {} bytes > {} bytes",
                        fixture_jsonl.len(),
                        MAX_ADAPTER_FIXTURE_BYTES
                    )));
                }
                let (plan, _prompt_source) = self.dispatch_plan_with_prompt(&dispatch_plan_id)?;
                let gate = self.latest_dispatch_gate(&dispatch_plan_id)?;
                if !gate.provider_cli_execution_allowed {
                    let execution_request = self.latest_execution_request(&dispatch_plan_id)?;
                    let execution = self.dispatch_execution_projection(
                        &plan,
                        &execution_request,
                        DispatchExecutionOutcome {
                            status: "blocked_by_preflight",
                            provider_cli_executed: false,
                            runtime_process_ref: None,
                            exit_code: None,
                            stdout_artifact_id: None,
                            stderr_artifact_id: None,
                            credential_scan_status: "not_run",
                            raw_output_policy: "not_captured",
                            reason_codes: &gate.reason_codes,
                        },
                    );
                    self.append_dispatch_execution(&origin, &plan, &execution)?;
                    return self.response(
                        request_id,
                        origin,
                        ServerResponsePayload::DispatchRun(DispatchRunSummary::from_execution(
                            &execution, 0, 0, 0, 0, 0,
                        )),
                    );
                }
                let turn_id = self.dispatch_plan_turn_id(&plan)?.unwrap_or_else(|| {
                    format!("turn-{}", stable_hash(plan.dispatch_plan_id.as_bytes()))
                });
                let fixture_hash = stable_hash(fixture_jsonl.as_bytes());
                self.reject_changed_dispatch_fixture(&plan.dispatch_plan_id, &fixture_hash)?;
                let adapter_events = parse_adapter_events(&plan.adapter_kind, &fixture_jsonl)
                    .map_err(ServerError::AdapterFixture)?;
                if adapter_events.is_empty() {
                    return Err(ServerError::AdapterFixture(
                        "dispatch fixture produced no normalized events".to_string(),
                    ));
                }
                let (_session, run_projection, _agent, run) =
                    self.run_refs_for_session_run(&plan.session_id, &plan.run_id)?;
                let report = self
                    .controller
                    .apply_normalized_adapter_events_with_turn(
                        &run,
                        &adapter_events,
                        Some(&turn_id),
                    )
                    .map_err(ServerError::State)?;
                let execution_request = self.latest_execution_request(&dispatch_plan_id)?;
                let runtime_process_ref =
                    format!("deterministic-fixture-ingest-{}", plan.dispatch_plan_id);
                let execution = self.dispatch_execution_projection(
                    &plan,
                    &execution_request,
                    DispatchExecutionOutcome {
                        status: "exited",
                        provider_cli_executed: false,
                        runtime_process_ref: Some(runtime_process_ref.clone()),
                        exit_code: None,
                        stdout_artifact_id: None,
                        stderr_artifact_id: None,
                        credential_scan_status: "not_applicable_fixture",
                        raw_output_policy: "content_hashed_not_rendered",
                        reason_codes: "deterministic_fixture_ingested_without_provider_cli",
                    },
                );
                self.append_dispatch_execution(&origin, &plan, &execution)?;
                self.append_dispatch_run_exit(&origin, &plan, &run_projection)?;
                let replay = self.dispatch_replay_projection(
                    &plan,
                    &gate,
                    &fixture_name,
                    &fixture_hash,
                    &report,
                );
                self.append_dispatch_replay(&origin, &plan, &replay)?;
                let command_hash = command_identity_hash(format!(
                    "run_dispatch:{dispatch_plan_id}:{fixture_hash}"
                ));
                let command = self.command_envelope(
                    &request_id,
                    &origin,
                    &command_hash,
                    CommandTarget::Session(plan.session_id.clone()),
                    CommandIntent::SendTask,
                    None,
                );
                self.record_server_request_handled(
                    &command,
                    &origin,
                    "run_dispatch_local",
                    Some(&run),
                    Some(serde_json::json!({
                        "dispatch_plan_id": plan.dispatch_plan_id,
                        "dispatch_gate_id": gate.dispatch_gate_id,
                        "execution_request_id": execution.execution_request_id,
                        "dispatch_execution_id": execution.dispatch_execution_id,
                        "dispatch_replay_id": replay.dispatch_replay_id,
                        "runtime_process_ref": runtime_process_ref,
                        "fixture_hash": fixture_hash,
                        "provider_cli_executed": false,
                        "raw_prompt_policy": execution.raw_prompt_policy,
                        "raw_output_policy": execution.raw_output_policy,
                        "credential_scan_status": execution.credential_scan_status,
                        "target_turn_id": turn_id,
                    })),
                )
                .map_err(ServerError::State)?;
                self.response(
                    request_id,
                    origin,
                    ServerResponsePayload::DispatchRun(DispatchRunSummary::from_execution(
                        &execution,
                        report.input_event_count,
                        report.appended_event_count,
                        report.tool_event_count,
                        report.summary_event_count,
                        report.completed_turn_count,
                    )),
                )
            }
            ServerCommand::RunDispatchTurn {
                agent_name,
                adapter,
                goal,
                workspace,
                artifacts,
                session_id,
                run_id,
                turn_id,
                capability_profile,
                runtime_scope,
                credential_scan_policy,
                raw_prompt_policy,
                raw_output_policy,
                tool_wrapper_policy,
                live_provider_opt_in,
                live_execution_opt_in,
                mock_runtime_opt_in,
                mock_provider_output_name,
                mock_provider_output_jsonl,
                timeout_seconds,
                max_turns,
                max_token_cost,
                turns_taken_before,
                token_cost_before,
                turn_token_cost,
                unattended,
            } => {
                // AI1: the single production orchestration path. The loop DRIVES
                // the existing preflight/run dispatch primitives and ANNOTATES the
                // run with a `TurnFinished`; the operator/live-run flow issues this
                // command instead of hand-sequencing those primitives beside the
                // loop, so the path the design claims is the path that executes.
                //
                // A live turn must run inside a wall-clock-bounded ceiling
                // (`run_dispatch_turn` rejects a turn whose ceiling does not bound
                // wall-clock); a zero timeout cannot satisfy that, so reject it
                // here with a clear message rather than letting the loop reject a
                // degenerate ceiling.
                if timeout_seconds == 0 {
                    return Err(ServerError::AdapterFixture(
                        "RunDispatchTurn requires a non-zero wall-clock timeout (the live turn \
                         runs inside a wall-clock-bounded resource ceiling)"
                            .to_string(),
                    ));
                }
                let ceiling = capo_controller::RunResourceCeiling::for_live_provider(
                    max_turns,
                    std::time::Duration::from_secs(timeout_seconds),
                    max_token_cost,
                );
                let usage_before = capo_controller::RunResourceUsage {
                    turns_taken: turns_taken_before,
                    wall_clock_elapsed: std::time::Duration::ZERO,
                    token_cost: token_cost_before,
                };
                let outcome = self.run_dispatch_turn(DispatchTurnRequest {
                    agent_name,
                    adapter,
                    goal,
                    workspace,
                    artifacts,
                    session_id,
                    run_id,
                    turn_id,
                    mode: DispatchTurnMode::LiveProvider(Box::new(LiveProviderTurn {
                        capability_profile,
                        runtime_scope,
                        credential_scan_policy,
                        raw_prompt_policy,
                        raw_output_policy,
                        tool_wrapper_policy,
                        live_provider_opt_in,
                        live_execution_opt_in,
                        mock_runtime_opt_in,
                        mock_provider_output_name,
                        mock_provider_output_jsonl,
                        ceiling,
                        usage_before,
                        turn_token_cost,
                        // The spawn-path codex binary is resolved server-side from
                        // `CAPO_CODEX_BIN`; the command carries no explicit override.
                        codex_program_override: None,
                        unattended,
                    })),
                })?;
                let summary = DispatchTurnSummary {
                    run: outcome.run,
                    finished: TurnFinishedSummary::from_finished(&outcome.finished),
                    ceiling_breach_code: outcome
                        .ceiling_breach
                        .map(|breach| breach.code().to_string()),
                };
                self.response(
                    request_id,
                    origin,
                    ServerResponsePayload::DispatchTurn(summary),
                )
            }
            ServerCommand::Recover => {
                let recovery = self.recover_server(&request_id, &origin)?;
                self.response(
                    request_id,
                    origin,
                    ServerResponsePayload::Recovery(recovery),
                )
            }
            ServerCommand::Subscribe {
                session_id,
                from_sequence,
            } => {
                // The request/response transport returns the catch-up backlog
                // here; the live tail is delivered as JSON-RPC notifications
                // through the persistent connection (the broadcast subscription
                // is obtained via `CapoServer::subscribe`). `Subscribe` is
                // read-only: it reads the log and registers a subscriber, never
                // appending an event.
                let backlog = self.read_subscription_backlog(session_id, from_sequence)?;
                self.response(
                    request_id,
                    origin,
                    ServerResponsePayload::Subscribed(backlog),
                )
            }
            ServerCommand::ReadThread {
                session_id,
                from_sequence,
            } => {
                // ReadThread is read-only: it projects the multi-turn thread read
                // model from the event log strictly after `from_sequence`, never
                // appending an event. The projection rebuilds identically from
                // the durable log, so a read after restart reconstructs the same
                // thread, and its `next_sequence` watermark composes with a
                // `Subscribe` tail over the same watermark.
                let thread = self.read_thread(session_id, from_sequence)?;
                self.response(request_id, origin, ServerResponsePayload::Thread(thread))
            }
            // GA2 (goal-orchestration GO4/GO6): the typed goal lifecycle mutations
            // all flow through the server/controller boundary here.
            ServerCommand::SetGoal { spec } => {
                self.handle_goal_command_set(request_id, origin, spec)
            }
            ServerCommand::PauseGoal { goal_id } => self.handle_goal_lifecycle(
                request_id,
                origin,
                goal_id,
                capo_state::GoalProjection::PAUSED,
                "",
                EventKind::GoalPaused,
                "goal.paused",
            ),
            ServerCommand::ResumeGoal { goal_id } => self.handle_goal_lifecycle(
                request_id,
                origin,
                goal_id,
                capo_state::GoalProjection::ACTIVE,
                "",
                EventKind::GoalResumed,
                "goal.resumed",
            ),
            ServerCommand::BlockGoal { goal_id, reason } => self.handle_goal_lifecycle(
                request_id,
                origin,
                goal_id,
                capo_state::GoalProjection::BLOCKED,
                &reason,
                EventKind::GoalBlocked,
                "goal.blocked",
            ),
            ServerCommand::ClearGoal { goal_id, reason } => self.handle_goal_lifecycle(
                request_id,
                origin,
                goal_id,
                capo_state::GoalProjection::CLEARED,
                &reason,
                EventKind::GoalCleared,
                "goal.cleared",
            ),
            ServerCommand::SetRequirementStatus { record } => {
                self.handle_set_requirement_status(request_id, origin, record)
            }
            ServerCommand::RecordGoalReport { report } => {
                self.handle_record_goal_report(request_id, origin, report)
            }
            // GA2 (goal-orchestration GO9): a direct mark-complete is rejected by
            // construction -- completion is the GA5 auditor's verdict.
            ServerCommand::MarkGoalComplete { goal_id } => self.reject_mark_goal_complete(&goal_id),
            ServerCommand::ListGoals => self.handle_list_goals(request_id, origin),
            ServerCommand::ViewGoal { goal_id } => {
                self.handle_view_goal(request_id, origin, goal_id)
            }
            ServerCommand::GoalStory { goal_id } => self.handle_goal_report_listing(
                request_id,
                origin,
                goal_id,
                goal_commands::GoalReportSurface::Story,
            ),
            ServerCommand::GoalEvidence { goal_id } => self.handle_goal_report_listing(
                request_id,
                origin,
                goal_id,
                goal_commands::GoalReportSurface::Evidence,
            ),
            ServerCommand::GoalValidations { goal_id } => self.handle_goal_report_listing(
                request_id,
                origin,
                goal_id,
                goal_commands::GoalReportSurface::Validations,
            ),
            ServerCommand::GoalReviews { goal_id } => self.handle_goal_report_listing(
                request_id,
                origin,
                goal_id,
                goal_commands::GoalReportSurface::Reviews,
            ),
            ServerCommand::GoalRisks { goal_id } => self.handle_goal_report_listing(
                request_id,
                origin,
                goal_id,
                goal_commands::GoalReportSurface::Risks,
            ),
            ServerCommand::GoalTimeline { goal_id } => {
                self.handle_goal_timeline(request_id, origin, goal_id)
            }
            ServerCommand::GoalReport { goal_id, format } => {
                self.handle_goal_report_rendering(request_id, origin, goal_id, format)
            }
            ServerCommand::ContinueGoal {
                goal_id,
                continuation_id,
                conditions,
                turn,
            } => self.handle_continue_goal(
                request_id,
                origin,
                goal_id,
                continuation_id,
                conditions,
                *turn,
            ),
        }
    }

    /// Test-only access to the underlying event store, so a test can append a
    /// seeded event through the *same* broadcaster the server's `subscribe`
    /// reads from (a separately-opened store would have its own broadcast hub).
    #[cfg(test)]
    pub(crate) fn state_for_test(&self) -> &capo_state::SqliteStateStore {
        self.controller.state()
    }

    /// Open an event tail (ST4): the catch-up backlog plus a live [`EventStream`]
    /// over newly-committed events.
    ///
    /// The broadcast subscription is taken **before** the backlog snapshot is
    /// read, so no event committed between the snapshot and the first live poll
    /// is missed (no gap). The returned [`SubscriptionBacklog::next_sequence`]
    /// seeds the stream's delivery watermark, so a live event already present in
    /// the backlog is dropped at the seam (no duplicate). A `None` `session_id`
    /// tails every committed event; `Some(id)` tails one session.
    pub fn subscribe(
        &self,
        session_id: Option<String>,
        from_sequence: i64,
    ) -> ServerResult<(SubscriptionBacklog, EventStream)> {
        // Subscribe first, then snapshot the backlog: any event committed after
        // this point is captured live, and the seam watermark below drops the
        // overlap.
        let subscription = self.controller.state().event_broadcaster().subscribe();
        let backlog = self.read_subscription_backlog(session_id.clone(), from_sequence)?;
        let stream = EventStream::new(subscription, backlog.next_sequence, session_id);
        Ok((backlog, stream))
    }

    /// Read the catch-up backlog for a subscription: every committed event
    /// strictly after `from_sequence` (optionally one session), in order, plus
    /// the watermark the live tail resumes from.
    fn read_subscription_backlog(
        &self,
        session_id: Option<String>,
        from_sequence: i64,
    ) -> ServerResult<SubscriptionBacklog> {
        let records = match &session_id {
            Some(session_id) => self
                .controller
                .state()
                .events_after_for_session(
                    &SessionId::new(session_id.clone()),
                    from_sequence,
                    EVENT_TAIL_BACKLOG_LIMIT,
                )
                .map_err(ServerError::State)?,
            None => self
                .controller
                .state()
                .events_after(from_sequence, EVENT_TAIL_BACKLOG_LIMIT)
                .map_err(ServerError::State)?,
        };
        // The live tail resumes strictly after the highest backlog sequence;
        // with an empty backlog it resumes from the caller's watermark.
        let next_sequence = records
            .last()
            .map(|record| record.sequence)
            .unwrap_or(from_sequence);
        let events = records.into_iter().map(ServerEvent::from_record).collect();
        Ok(SubscriptionBacklog {
            session_id,
            from_sequence,
            next_sequence,
            events,
        })
    }

    /// Read a session's multi-turn conversation thread (ST5) incrementally from
    /// `from_sequence`.
    ///
    /// This delegates to the pure `capo_state::SqliteStateStore::session_thread`
    /// projection over the durable event log (the same forward read the ST4
    /// backlog uses), so the thread is a rebuildable read model and composes
    /// gap-free with a `Subscribe` resuming from the returned `next_sequence`.
    fn read_thread(&self, session_id: String, from_sequence: i64) -> ServerResult<ServerThread> {
        let thread = self
            .controller
            .state()
            .session_thread(
                &SessionId::new(session_id),
                from_sequence,
                EVENT_TAIL_BACKLOG_LIMIT,
            )
            .map_err(ServerError::State)?;
        Ok(ServerThread::from_thread(thread))
    }

    /// Abort the live turn for a session by a typed mid-turn interrupt (ST6).
    ///
    /// This is the server handler the transport's in-band `interrupt` frame
    /// drives (via [`transport::RequestHandler::interrupt`]). It is distinct from
    /// the coarse `StopAgent` command: it records the turn-keyed
    /// `session.interrupted` event through the existing
    /// `FakeBoundaryController::interrupt_command` (the SAME mechanism
    /// `ServerCommand::InterruptAgent` uses), so the event is keyed to the
    /// session's active turn and the thread read model renders that turn as
    /// `Interrupted` -- on the SAME serialization point as every other write, so
    /// the interrupt never opens a second writer.
    ///
    /// The runtime process-group kill that reaps descendants is driven by the
    /// transport signaling the in-flight request's [`transport::CancellationToken`]
    /// as interrupted; this method records the durable abort truth that pairs
    /// with that kill.
    pub fn interrupt_session(&self, session_id: &str, reason: &str) -> ServerResult<()> {
        let session_id = SessionId::new(session_id.to_string());
        let session = self
            .controller
            .state()
            .session(&session_id)
            .map_err(ServerError::State)?
            .ok_or_else(|| ServerError::UnknownSession {
                session_id: session_id.to_string(),
            })?;
        let agent = self
            .controller
            .state()
            .agent(&session.agent_id)
            .map_err(ServerError::State)?
            .ok_or_else(|| {
                ServerError::AdapterFixture(format!(
                    "missing agent for session: {}",
                    session.agent_id
                ))
            })?;
        let reason_hash = stable_hash(reason.as_bytes());
        let command_hash =
            command_identity_hash(format!("interrupt_session:{}:{reason_hash}", session_id));
        let origin = ServerClientOrigin {
            client_id: "local-cli".to_string(),
            actor_id: "local-user".to_string(),
            input_origin: ServerInputOrigin::Cli,
        };
        let mut command = self.command_envelope(
            &format!(
                "server-interrupt-session-{}-{reason_hash}",
                slug(session_id.as_str())
            ),
            &origin,
            &command_hash,
            CommandTarget::Agent(agent.agent_id.clone()),
            CommandIntent::InterruptSession,
            Some(reason.to_string()),
        );
        command
            .structured_args
            .push(("agent".to_string(), agent.name.clone()));
        self.command_controller()
            .interrupt_command(&command)
            .map_err(ServerError::State)?;
        self.record_server_request_handled(
            &command,
            &origin,
            "interrupt_session",
            None,
            Some(serde_json::json!({
                "session_id": session_id.to_string(),
                "reason_hash": reason_hash,
                "raw_reason_policy": "not_rendered",
                "interrupt_kind": "typed_mid_turn",
            })),
        )
        .map_err(ServerError::State)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
