//! DP1: the live ACP [`AgentAdapter`] that drives the [`AcpWireClient`] over a
//! runtime-spawned process.
//!
//! This is the trait-level seam the controller loop drives: it launches the ACP
//! agent through `RuntimeRunner` (`LocalProcessRunner::spawn_piped_process`, so
//! the runtime owns the process group), attaches the wire client after start,
//! runs `initialize` -> `session/new` -> `session/prompt` (ingesting
//! `session/update` notifications and answering `session/request_permission` on
//! the wire), and reduces the turn transcript to a provider-neutral
//! [`TurnOutput`].
//!
//! Live spawning is gated, fail-closed-fast, behind an explicit opt-in env gate
//! mirroring the Codex live gates: the deterministic transcript tests never
//! spawn a process (they drive the wire client against a scripted transport),
//! and DP11 owns the live smoke. ACP stays strictly an adapter; no
//! `session/update` is authoritative for read models, and Capo never exposes
//! itself as an ACP agent backend.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use capo_core::{BoundaryBinding, BoundaryKind, RunId, SessionId, TurnId};
use capo_runtime::{LocalProcessRunner, PipedRunningProcess};

use crate::acp_wire::{
    AcpTransport, AcpTurnTranscript, AcpWireClient, AcpWireError, PipedProcessTransport,
};
use crate::{
    AcpAdapter, AcpPermissionDecider, AcpSessionSetupPlan, AdapterSession, AdapterSessionRequest,
    AgentAdapter, FailClosedPermissionDecider, NormalizedAdapterEvent, TurnOutput, TurnRequest,
    scan_artifacts_for_sensitive_markers,
};

/// The two opt-in env gates the live ACP path honors, mirroring the Codex live
/// gates (`CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT` + a provider run gate). The live
/// ACP adapter spawns a real agent ONLY when BOTH hold; otherwise it fails
/// closed fast, exactly like the Codex chat path.
pub const ACP_LIVE_PREFLIGHT_OPT_IN_ENV: &str = "CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT";
pub const ACP_LIVE_RUN_OPT_IN_ENV: &str = "CAPO_SERVER_RUN_ACP_LIVE";

/// Whether the live ACP spawn path is open (both gates set to `1`).
pub fn acp_live_gate_open() -> bool {
    env_flag(ACP_LIVE_PREFLIGHT_OPT_IN_ENV) && env_flag(ACP_LIVE_RUN_OPT_IN_ENV)
}

fn env_flag(name: &str) -> bool {
    std::env::var(name).as_deref() == Ok("1")
}

/// A typed error from a live ACP turn.
#[derive(Debug)]
pub enum AcpLiveError {
    /// The live opt-in gate is closed; no ACP agent was spawned.
    GateClosed {
        agent_name: String,
        missing_env: Vec<&'static str>,
    },
    /// Launching or attaching to the ACP process failed.
    Spawn(String),
    /// A protocol/transport failure on the wire.
    Wire(AcpWireError),
    /// The ACP stderr artifact leaked a sensitive marker and was dropped.
    Output(String),
}

impl std::fmt::Display for AcpLiveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::GateClosed {
                agent_name,
                missing_env,
            } => write!(
                f,
                "acp live is fail-closed for agent `{agent_name}`: missing live-provider opt-in \
                 ({})",
                missing_env.join(" and ")
            ),
            Self::Spawn(detail) => write!(f, "acp live spawn failed: {detail}"),
            Self::Wire(error) => write!(f, "acp live wire error: {error}"),
            Self::Output(detail) => write!(f, "acp live output handling failed: {detail}"),
        }
    }
}

impl std::error::Error for AcpLiveError {}

impl From<AcpWireError> for AcpLiveError {
    fn from(error: AcpWireError) -> Self {
        Self::Wire(error)
    }
}

/// The live ACP `AgentAdapter`. Launches a generic ACP JSON-RPC stdio agent
/// through `RuntimeRunner` and drives the wire client.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcpLiveAdapter {
    program: String,
    argv: Vec<String>,
    workspace_root: PathBuf,
    artifact_root: PathBuf,
    setup_plan: AcpSessionSetupPlan,
}

impl AcpLiveAdapter {
    /// Build a live ACP adapter for the given generic ACP agent binary,
    /// confined to `workspace_root` with artifacts under `artifact_root`, using
    /// `setup_plan` to decide which client capabilities to advertise and how to
    /// map `request_permission` options.
    pub fn new(
        program: impl Into<String>,
        argv: Vec<String>,
        workspace_root: impl Into<PathBuf>,
        artifact_root: impl Into<PathBuf>,
        setup_plan: AcpSessionSetupPlan,
    ) -> Self {
        Self {
            program: program.into(),
            argv,
            workspace_root: workspace_root.into(),
            artifact_root: artifact_root.into(),
            setup_plan,
        }
    }

    pub fn binding(&self) -> BoundaryBinding {
        BoundaryBinding {
            kind: BoundaryKind::AgentAdapter,
            variant: "acp-live",
            fake: false,
        }
    }

    /// Run ONE live ACP turn against a freshly spawned agent, routing any inbound
    /// `session/request_permission` through `decider` (the controller's
    /// `PermissionPolicy`-backed seam). See [`Self::run_turn`].
    pub fn run_turn_with_decider<'d>(
        &self,
        request: &TurnRequest,
        decider: Box<dyn AcpPermissionDecider + 'd>,
    ) -> Result<AcpTurnTranscript, AcpLiveError> {
        self.run_turn_inner(request, decider)
    }

    /// Run ONE live ACP turn against a freshly spawned agent: launch through the
    /// runtime, attach the wire client, `initialize` -> `session/new` ->
    /// `session/prompt`, and tear the process down. Fail-closed-fast when the
    /// gate is off.
    ///
    /// SAFETY: without an injected controller decider this uses the fail-closed
    /// default ([`FailClosedPermissionDecider`]) -- it cancels every permission
    /// request rather than self-authorizing. The controller path uses
    /// [`Self::run_turn_with_decider`] to route through `PermissionPolicy`.
    pub fn run_turn(&self, request: &TurnRequest) -> Result<AcpTurnTranscript, AcpLiveError> {
        self.run_turn_inner(request, Box::new(FailClosedPermissionDecider))
    }

    fn run_turn_inner<'d>(
        &self,
        request: &TurnRequest,
        decider: Box<dyn AcpPermissionDecider + 'd>,
    ) -> Result<AcpTurnTranscript, AcpLiveError> {
        if !acp_live_gate_open() {
            let mut missing_env = Vec::new();
            if !env_flag(ACP_LIVE_PREFLIGHT_OPT_IN_ENV) {
                missing_env.push(ACP_LIVE_PREFLIGHT_OPT_IN_ENV);
            }
            if !env_flag(ACP_LIVE_RUN_OPT_IN_ENV) {
                missing_env.push(ACP_LIVE_RUN_OPT_IN_ENV);
            }
            return Err(AcpLiveError::GateClosed {
                agent_name: request.agent_name.clone(),
                missing_env,
            });
        }

        let launch_plan = AcpAdapter::local_launch_plan(
            self.program.clone(),
            self.argv.clone(),
            self.workspace_root.clone(),
            self.artifact_root.clone(),
        );
        launch_plan
            .assert_subscription_safe()
            .map_err(AcpLiveError::Spawn)?;
        fs::create_dir_all(&launch_plan.workspace_root)
            .map_err(|error| AcpLiveError::Spawn(format!("workspace: {error}")))?;
        fs::create_dir_all(&launch_plan.artifact_root)
            .map_err(|error| AcpLiveError::Spawn(format!("artifacts: {error}")))?;

        // Launch through the runtime so the RUNTIME owns the process group; the
        // adapter only borrows the pipe handles.
        let runner = LocalProcessRunner::new(launch_plan.runtime_config());
        let run_id = RunId::new(format!("acp-live-{}", request.turn_id.as_str()));
        let mut process = runner
            .spawn_piped_process(
                launch_plan.runtime_request_for_turn(run_id, request.turn_id.as_str()),
            )
            .map_err(|error| AcpLiveError::Spawn(format!("{error:?}")))?;
        let stdin = process
            .take_stdin()
            .ok_or_else(|| AcpLiveError::Spawn("missing stdin pipe".to_string()))?;
        let stdout = process
            .take_stdout()
            .ok_or_else(|| AcpLiveError::Spawn("missing stdout pipe".to_string()))?;
        let stderr_path = process.stderr_path().to_path_buf();

        let transport = PipedProcessTransport::new(stdin, stdout);
        // No cancel flag on this trait-level path (byte-identical to pre-cancel).
        let result = self.drive_with_decider(transport, &request.goal, decider, None);

        let shutdown = process.shutdown("acp live turn complete");
        debug_assert_eq!(shutdown.process.status, "exited");

        // The agent's stderr diagnostics must pass the credential scan before
        // retention; drop the artifact if it leaked a marker.
        if scan_artifacts_for_sensitive_markers([&stderr_path]).is_err() {
            let _ = fs::remove_file(&stderr_path);
            return Err(AcpLiveError::Output(
                "acp agent stderr failed the sensitive-marker scan".to_string(),
            ));
        }

        result
    }

    /// DP11: spawn a REAL ACP agent through the runtime and hand back a
    /// [`LiveAcpSession`] whose [`AcpTransport`] a caller (e.g. the controller's
    /// `drive_acp_live_turn`) can drive directly -- so the live wire round-trip
    /// runs inside the controller's `PermissionPolicy` seam rather than the
    /// adapter's fail-closed default, while ACP stays strictly an adapter.
    ///
    /// Fail-closed-fast: returns [`AcpLiveError::GateClosed`] unless BOTH live
    /// opt-in env gates are set. The runtime owns the spawned process group; the
    /// returned session borrows the pipe handles and MUST be finalized with
    /// [`LiveAcpSession::finalize`] to tear the process down and scan the agent's
    /// stderr artifact for credential markers before retention.
    pub fn spawn_live_session(&self, turn: &TurnId) -> Result<LiveAcpSession, AcpLiveError> {
        if !acp_live_gate_open() {
            let mut missing_env = Vec::new();
            if !env_flag(ACP_LIVE_PREFLIGHT_OPT_IN_ENV) {
                missing_env.push(ACP_LIVE_PREFLIGHT_OPT_IN_ENV);
            }
            if !env_flag(ACP_LIVE_RUN_OPT_IN_ENV) {
                missing_env.push(ACP_LIVE_RUN_OPT_IN_ENV);
            }
            return Err(AcpLiveError::GateClosed {
                agent_name: self.program.clone(),
                missing_env,
            });
        }

        let launch_plan = AcpAdapter::local_launch_plan(
            self.program.clone(),
            self.argv.clone(),
            self.workspace_root.clone(),
            self.artifact_root.clone(),
        );
        launch_plan
            .assert_subscription_safe()
            .map_err(AcpLiveError::Spawn)?;
        fs::create_dir_all(&launch_plan.workspace_root)
            .map_err(|error| AcpLiveError::Spawn(format!("workspace: {error}")))?;
        fs::create_dir_all(&launch_plan.artifact_root)
            .map_err(|error| AcpLiveError::Spawn(format!("artifacts: {error}")))?;

        let runner = LocalProcessRunner::new(launch_plan.runtime_config());
        let run_id = RunId::new(format!("acp-live-{}", turn.as_str()));
        let mut process = runner
            .spawn_piped_process(launch_plan.runtime_request_for_turn(run_id, turn.as_str()))
            .map_err(|error| AcpLiveError::Spawn(format!("{error:?}")))?;
        let stdin = process
            .take_stdin()
            .ok_or_else(|| AcpLiveError::Spawn("missing stdin pipe".to_string()))?;
        let stdout = process
            .take_stdout()
            .ok_or_else(|| AcpLiveError::Spawn("missing stdout pipe".to_string()))?;
        let transport = PipedProcessTransport::new(stdin, stdout);
        Ok(LiveAcpSession {
            process,
            transport: Some(transport),
        })
    }

    /// Drive the full `initialize` -> `session/new` -> `session/prompt` flow over
    /// an attached transport. Shared by the live spawn path and the deterministic
    /// scripted-transport tests, so the live adapter exercises the IDENTICAL wire
    /// logic the fixtures assert.
    pub fn drive<T: AcpTransport>(
        &self,
        transport: T,
        prompt: &str,
    ) -> Result<AcpTurnTranscript, AcpLiveError> {
        self.drive_with_decider(transport, prompt, Box::new(FailClosedPermissionDecider), None)
    }

    /// Drive the full flow, routing inbound `session/request_permission` through
    /// `decider` (the controller's `PermissionPolicy`-backed seam). The wire client
    /// writes back ONLY the decider's outcome, so the wire client is never the
    /// policy authority.
    ///
    /// COOPERATIVE CANCEL (B2): the trailing `cancel` is an OPTIONAL shared flag
    /// installed onto the wire client. `None` (every existing caller via
    /// [`Self::drive`], the deterministic suites, and the validated live loop)
    /// means no cancel check and byte-identical frames. When `Some` and the flag
    /// flips during the prompt pump, the wire client sends a best-effort
    /// `session/cancel` and the prompt returns [`AcpWireError::Cancelled`], which
    /// is mapped HERE to a terminal transcript with `stop_reason = "cancelled"`
    /// (matching the existing cancelled status mapping) rather than an error, so
    /// the controller ingests a clean cancelled turn.
    pub fn drive_with_decider<'d, T: AcpTransport>(
        &self,
        transport: T,
        prompt: &str,
        decider: Box<dyn AcpPermissionDecider + 'd>,
        cancel: Option<Arc<AtomicBool>>,
    ) -> Result<AcpTurnTranscript, AcpLiveError> {
        let mut client = AcpWireClient::attach(transport, self.setup_plan.clone())
            .with_permission_decider(decider);
        // Additive: install the cancel flag only when supplied. With `None` the
        // client carries `cancel: None` and the pump is byte-identical to today.
        if let Some(cancel) = cancel {
            client = client.with_cancel(cancel);
        }
        client.initialize()?;
        let session_id = client.session_new(self.workspace_root.to_string_lossy().as_ref())?;
        // If the setup plan selected a session mode (the live file-write profile
        // uses a permission-bypassing mode so the real bridge emits an on-wire
        // write callback instead of simulating the tool in its default mode),
        // switch to it AFTER session/new and BEFORE prompting. The deterministic
        // stub/scripted plans leave this `None`, so nothing extra is sent there.
        if let Some(mode_id) = self.setup_plan.session_mode.clone() {
            client.session_set_mode(&session_id, &mode_id)?;
        }
        match client.prompt(&session_id, prompt) {
            Ok(transcript) => Ok(transcript),
            // COOPERATIVE CANCEL: map the cancelled sentinel to a terminal
            // cancelled transcript (NOT an error) so the controller ingests a
            // clean `stopReason: cancelled` turn. Any pre-cancel `session/update`
            // events already ingested by the pump are lost on the error path here;
            // that is acceptable for a cancelled turn (the worker is being torn
            // down). This arm is only reachable when a cancel flag was installed.
            Err(AcpWireError::Cancelled { .. }) => Ok(AcpTurnTranscript {
                stop_reason: Some("cancelled".to_string()),
                cancelled: true,
                ..AcpTurnTranscript::default()
            }),
            Err(other) => Err(AcpLiveError::Wire(other)),
        }
    }

    /// LIVE STEERING: attach a PERSISTENT session — `initialize` + `session/new`
    /// (+ optional `session/set_mode`) performed ONCE — then `prompt` may be
    /// called REPEATEDLY on the same session id to CONTINUE the conversation
    /// (the ACP spec's multi-turn "once a prompt turn completes, the Client may
    /// send another `session/prompt`"). This is the steerable-worker path; the
    /// one-shot [`Self::drive_with_decider`] above is unchanged.
    ///
    /// The returned [`PersistentAcpSession`] borrows nothing from `self`; it owns
    /// the wire client (and its `!Send` decider), so it stays pinned to the
    /// thread that drives it — exactly how the worker actor uses it.
    pub fn attach_persistent_session<'d, T: AcpTransport>(
        &self,
        transport: T,
        decider: Box<dyn AcpPermissionDecider + 'd>,
        cancel: Option<Arc<AtomicBool>>,
    ) -> Result<PersistentAcpSession<'d, T>, AcpLiveError> {
        let mut client = AcpWireClient::attach(transport, self.setup_plan.clone())
            .with_permission_decider(decider);
        if let Some(cancel) = cancel {
            client = client.with_cancel(cancel);
        }
        client.initialize()?;
        let session_id = client.session_new(self.workspace_root.to_string_lossy().as_ref())?;
        if let Some(mode_id) = self.setup_plan.session_mode.clone() {
            client.session_set_mode(&session_id, &mode_id)?;
        }
        Ok(PersistentAcpSession { client, session_id })
    }
}

/// A live ACP session kept open across multiple prompts so a worker can be
/// STEERED: the conductor cancels the in-flight prompt (B2 cooperative cancel)
/// and a follow-up [`Self::prompt`] continues the SAME session. Owns the wire
/// client; not `Send` (the permission decider isn't), so it lives on the
/// worker's driving thread.
pub struct PersistentAcpSession<'d, T: AcpTransport> {
    client: AcpWireClient<'d, T>,
    session_id: String,
}

impl<'d, T: AcpTransport> PersistentAcpSession<'d, T> {
    /// The external ACP session id (`session/new` result) every prompt continues.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Drive ONE `session/prompt` on the persistent session. A cooperative cancel
    /// is mapped to a terminal `cancelled` transcript (same mapping as
    /// [`AcpLiveAdapter::drive_with_decider`]), NOT an error — so a steered turn
    /// ends cleanly and the next prompt can continue. The caller resets the
    /// cancel flag between prompts.
    pub fn prompt(&mut self, prompt: &str) -> Result<AcpTurnTranscript, AcpLiveError> {
        match self.client.prompt(&self.session_id, prompt) {
            Ok(transcript) => Ok(transcript),
            Err(AcpWireError::Cancelled { .. }) => Ok(AcpTurnTranscript {
                stop_reason: Some("cancelled".to_string()),
                cancelled: true,
                ..AcpTurnTranscript::default()
            }),
            Err(other) => Err(AcpLiveError::Wire(other)),
        }
    }
}

/// DP11: a live ACP agent spawned through the runtime (the runtime owns the
/// process group). The caller takes its [`AcpTransport`] to drive one live turn
/// (e.g. through the controller seam) and then [`finalize`](Self::finalize)s it,
/// which tears the process down and scans the agent's stderr artifact for
/// credential markers, dropping it (and failing) if it leaked one.
pub struct LiveAcpSession {
    process: PipedRunningProcess,
    transport: Option<PipedProcessTransport<std::process::ChildStdin>>,
}

impl LiveAcpSession {
    /// Take the live wire transport exactly once so a caller can drive the turn.
    pub fn take_transport(&mut self) -> Option<PipedProcessTransport<std::process::ChildStdin>> {
        self.transport.take()
    }

    /// The agent's captured stderr artifact path (for evidence after the scan).
    pub fn stderr_path(&self) -> &Path {
        self.process.stderr_path()
    }

    /// Tear the spawned agent down and enforce the secrets-stripped contract on
    /// its stderr artifact: if the agent's stderr leaked a credential marker, the
    /// artifact is dropped and this fails closed.
    pub fn finalize(mut self, reason: &str) -> Result<(), AcpLiveError> {
        // Drop the transport first so the agent sees EOF on stdin and can exit.
        self.transport = None;
        let stderr_path = self.process.stderr_path().to_path_buf();
        let _ = self.process.shutdown(reason);
        if scan_artifacts_for_sensitive_markers([&stderr_path]).is_err() {
            let _ = fs::remove_file(&stderr_path);
            return Err(AcpLiveError::Output(
                "acp agent stderr failed the sensitive-marker scan".to_string(),
            ));
        }
        Ok(())
    }
}

impl AgentAdapter for AcpLiveAdapter {
    fn binding(&self) -> BoundaryBinding {
        AcpLiveAdapter::binding(self)
    }

    fn open_session(&self, request: AdapterSessionRequest) -> AdapterSession {
        AdapterSession {
            session_id: request.session_id,
            external_session_ref: format!("acp-live-session-{}", request.agent_name),
            adapter_capability: "acp-jsonrpc-stdio".to_string(),
        }
    }

    fn send_turn(&self, session: &AdapterSession, request: TurnRequest) -> TurnOutput {
        match self.run_turn(&request) {
            Ok(transcript) => turn_output_from_transcript(session, &request, &transcript),
            Err(error) => TurnOutput {
                turn_id: request.turn_id,
                external_session_ref: session.external_session_ref.clone(),
                summary: error.to_string(),
                confidence: 0,
                status: "blocked".to_string(),
                tool_name: "capo.session_summary".to_string(),
            },
        }
    }

    fn attach_session(
        &self,
        session_id: SessionId,
        external_session_ref: String,
    ) -> AdapterSession {
        AdapterSession {
            session_id,
            external_session_ref,
            adapter_capability: "acp-jsonrpc-stdio".to_string(),
        }
    }

    fn interrupt(&self, session: &AdapterSession, reason: &str) -> TurnOutput {
        TurnOutput {
            turn_id: TurnId::new(format!("interrupt-{}", session.session_id)),
            external_session_ref: session.external_session_ref.clone(),
            summary: format!("ACP live interrupted: {reason}"),
            confidence: 0,
            status: "canceled".to_string(),
            tool_name: "capo.session_summary".to_string(),
        }
    }

    fn stop(&self, session: &AdapterSession, reason: &str) -> TurnOutput {
        TurnOutput {
            turn_id: TurnId::new(format!("stop-{}", session.session_id)),
            external_session_ref: session.external_session_ref.clone(),
            summary: format!("ACP live stopped: {reason}"),
            confidence: 0,
            status: "completed".to_string(),
            tool_name: "capo.session_summary".to_string(),
        }
    }
}

/// Reduce a driven ACP turn transcript to a provider-neutral [`TurnOutput`].
///
/// The summary is the last message/thought content the agent streamed; the
/// status reflects the prompt `stopReason` (mapped to the loop's vocabulary).
pub fn turn_output_from_transcript(
    session: &AdapterSession,
    request: &TurnRequest,
    transcript: &AcpTurnTranscript,
) -> TurnOutput {
    let summary = transcript
        .events
        .iter()
        .rev()
        .find_map(|event| event.content.clone())
        .unwrap_or_else(|| format!("ACP live accepted goal: {}", request.goal));
    let status = match transcript.stop_reason.as_deref() {
        Some("cancelled") => "canceled".to_string(),
        Some("end_turn") | Some("completed") => "completed".to_string(),
        Some(other) => other.to_string(),
        None if transcript.cancelled => "canceled".to_string(),
        None => "active".to_string(),
    };
    let tool_name = transcript
        .events
        .iter()
        .find_map(|event: &NormalizedAdapterEvent| event.tool_name.clone())
        .unwrap_or_else(|| "capo.session_summary".to_string());
    let external_session_ref = transcript
        .events
        .iter()
        .find_map(|event| event.external_session_ref.clone())
        .unwrap_or_else(|| session.external_session_ref.clone());
    TurnOutput {
        turn_id: request.turn_id.clone(),
        external_session_ref,
        summary,
        confidence: 80,
        status,
        tool_name,
    }
}
