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
use std::path::PathBuf;

use capo_core::{BoundaryBinding, BoundaryKind, RunId, SessionId, TurnId};
use capo_runtime::LocalProcessRunner;

use crate::acp_wire::{
    AcpTransport, AcpTurnTranscript, AcpWireClient, AcpWireError, PipedProcessTransport,
};
use crate::{
    AcpAdapter, AcpSessionSetupPlan, AdapterSession, AdapterSessionRequest, AgentAdapter,
    NormalizedAdapterEvent, TurnOutput, TurnRequest, scan_artifacts_for_sensitive_markers,
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

    /// Run ONE live ACP turn against a freshly spawned agent: launch through the
    /// runtime, attach the wire client, `initialize` -> `session/new` ->
    /// `session/prompt`, and tear the process down. Fail-closed-fast when the
    /// gate is off.
    pub fn run_turn(&self, request: &TurnRequest) -> Result<AcpTurnTranscript, AcpLiveError> {
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
        let result = self.drive(transport, &request.goal);

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

    /// Drive the full `initialize` -> `session/new` -> `session/prompt` flow over
    /// an attached transport. Shared by the live spawn path and the deterministic
    /// scripted-transport tests, so the live adapter exercises the IDENTICAL wire
    /// logic the fixtures assert.
    pub fn drive<T: AcpTransport>(
        &self,
        transport: T,
        prompt: &str,
    ) -> Result<AcpTurnTranscript, AcpLiveError> {
        let mut client = AcpWireClient::attach(transport, self.setup_plan.clone());
        client.initialize()?;
        let session_id = client.session_new(self.workspace_root.to_string_lossy().as_ref())?;
        let transcript = client.prompt(&session_id, prompt)?;
        Ok(transcript)
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
