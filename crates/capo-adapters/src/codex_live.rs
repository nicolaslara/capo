//! AI2: a real Codex [`AgentAdapter`] for the chat/steer surface.
//!
//! The chat surface (`SendTask` -> controller `send_task`, `SteerAgent` ->
//! controller `redirect`) drives the agent's bound [`AgentAdapterHandle`]
//! through the [`AgentAdapter`] trait. Before AI2 the only handle variants were
//! the deterministic [`crate::FakeAdapter`] and [`crate::ScriptedMockAgent`], so
//! a Codex-profile agent's chat turn still produced a canned fake summary.
//!
//! AI2's CORRECTED design RESPECTS THE AGENT'S ADAPTER BINDING:
//!
//! - A fake/scripted/mock agent keeps its fake/scripted handle and runs
//!   deterministically, exactly as before AI2. Real Codex is NEVER a global
//!   default that replaces the fake adapter for unbound/mock agents.
//! - A Codex-BOUND agent (registered with [`CodexLiveAdapter`]) drives the real,
//!   read-only one-shot Codex execution on its chat turn -- but FAIL-CLOSED-FAST:
//!     - when [`codex_live_chat_gate_open`] is TRUE the turn spawns the real
//!       read-only `codex exec --json` one-shot, waits (bounded), and parses its
//!       JSONL into a provider-neutral [`TurnOutput`];
//!     - when it is FALSE the turn returns an IMMEDIATE typed
//!       [`CodexLiveChatError::GateClosed`] -- NO process spawn, NO blocking, NO
//!       waiting -- mirroring operator-control's fail-closed posture.
//!
//! Because the controller drives chat through the agent's own handle, this is
//! the whole routing story: nothing in the controller has to "default" to Codex.
//! The fallible [`AgentAdapter::try_send_turn`] seam keeps the existing
//! infallible [`AgentAdapter::send_turn`] working unchanged for the fake/scripted
//! handles (its default impl wraps `send_turn` in `Ok`) while giving the Codex
//! handle a typed fail-closed error path the controller propagates.

use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use capo_core::{BoundaryBinding, BoundaryKind, RunId, SessionId, TurnId};
use capo_runtime::LocalProcessRunner;

use crate::{
    AdapterSession, AdapterSessionRequest, AgentAdapter, CodexExecAdapter, NormalizedAdapterEvent,
    TurnOutput, TurnRequest, scan_artifacts_for_sensitive_markers,
};

/// The two opt-in env gates the live-provider dispatch path already honors. Chat
/// against a Codex-bound agent is allowed to spawn the real provider ONLY when
/// BOTH hold, mirroring the dispatch preflight (`CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT`)
/// and the live-write/run gate (`CAPO_SERVER_RUN_CODEX_LIVE`). This is a chat-time
/// fail-closed gate, NOT a new global default: a fake/scripted handle never
/// consults it.
pub const CODEX_LIVE_PREFLIGHT_OPT_IN_ENV: &str = "CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT";
pub const CODEX_LIVE_RUN_OPT_IN_ENV: &str = "CAPO_SERVER_RUN_CODEX_LIVE";

/// Whether the real-Codex chat path is open.
///
/// Returns `true` only when BOTH live-provider opt-in env gates are explicitly
/// set to `1`. When this is `false`, a Codex-bound chat turn fails closed FAST
/// (an immediate typed error) without spawning anything.
pub fn codex_live_chat_gate_open() -> bool {
    env_flag(CODEX_LIVE_PREFLIGHT_OPT_IN_ENV) && env_flag(CODEX_LIVE_RUN_OPT_IN_ENV)
}

fn env_flag(name: &str) -> bool {
    std::env::var(name).as_deref() == Ok("1")
}

/// A typed error from a Codex-bound chat turn.
///
/// `GateClosed` is the fail-closed-fast outcome: it is returned IMMEDIATELY when
/// [`codex_live_chat_gate_open`] is false, before any process spawn or wait, so
/// the chat path can never block the server request handler. The remaining
/// variants are spawn/parse failures that surface as typed errors instead of a
/// fabricated fake summary.
#[derive(Debug)]
pub enum CodexLiveChatError {
    /// The live-provider opt-in gate is closed; no Codex was spawned.
    GateClosed {
        agent_name: String,
        missing_env: Vec<&'static str>,
    },
    /// Spawning or waiting on the Codex process failed.
    Spawn(String),
    /// The Codex process did not exit cleanly (timed out, killed, signalled).
    NonClean { status: String },
    /// Reading or scanning the Codex stdout artifact failed (or it leaked a
    /// sensitive marker and was deleted).
    Output(String),
    /// The Codex stdout produced no normalized events.
    EmptyOutput,
}

impl std::fmt::Display for CodexLiveChatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::GateClosed {
                agent_name,
                missing_env,
            } => write!(
                f,
                "codex live chat is fail-closed for agent `{agent_name}`: missing live-provider \
                 opt-in ({})",
                missing_env.join(" and ")
            ),
            Self::Spawn(detail) => write!(f, "codex live chat spawn failed: {detail}"),
            Self::NonClean { status } => {
                write!(f, "codex live chat process did not exit cleanly: {status}")
            }
            Self::Output(detail) => write!(f, "codex live chat output handling failed: {detail}"),
            Self::EmptyOutput => {
                write!(f, "codex live chat produced no normalized adapter events")
            }
        }
    }
}

impl std::error::Error for CodexLiveChatError {}

/// A real Codex `AgentAdapter` whose chat turn drives the read-only one-shot
/// `codex exec --json` execution (gate-respecting, fail-closed-fast).
///
/// This is a CODEX-BOUND handle: it is installed only for agents explicitly
/// bound to the Codex adapter. The fake/scripted handles are unaffected.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodexLiveAdapter {
    workspace_root: PathBuf,
    artifact_root: PathBuf,
    /// Absolute path to a codex binary to run instead of resolving `codex` from
    /// PATH. Ops set it from `CAPO_CODEX_BIN`; tests pin an executable stub by
    /// absolute path. `None` keeps `codex`. Honored only when absolute, because
    /// the runtime spawns with `env_clear()`.
    codex_program_override: Option<String>,
    /// Bounded wall-clock for the one-shot, so the chat path can never block the
    /// server request handler unbounded even when the gate is open.
    timeout_seconds: u64,
}

impl CodexLiveAdapter {
    /// Open the Codex chat adapter confined to `workspace_root` with artifacts
    /// under `artifact_root` (read-only one-shot; touches nothing in the
    /// workspace).
    pub fn new(workspace_root: impl Into<PathBuf>, artifact_root: impl Into<PathBuf>) -> Self {
        Self {
            workspace_root: workspace_root.into(),
            artifact_root: artifact_root.into(),
            codex_program_override: None,
            timeout_seconds: 300,
        }
    }

    /// Pin an absolute-path codex program override (ops `CAPO_CODEX_BIN`; tests a
    /// deterministic stub). Non-absolute values are ignored at spawn time because
    /// the runtime spawns with `env_clear()`.
    #[must_use]
    pub fn with_codex_program_override(mut self, program: impl Into<String>) -> Self {
        self.codex_program_override = Some(program.into());
        self
    }

    /// Set the bounded wall-clock timeout for the one-shot.
    #[must_use]
    pub fn with_timeout_seconds(mut self, timeout_seconds: u64) -> Self {
        self.timeout_seconds = timeout_seconds;
        self
    }

    /// The provider-neutral boundary binding for the real Codex chat adapter.
    /// `fake: false` -- this is a real provider binding, distinct from the
    /// fake/scripted handles.
    pub fn binding(&self) -> BoundaryBinding {
        BoundaryBinding {
            kind: BoundaryKind::AgentAdapter,
            variant: "codex-live-chat",
            fake: false,
        }
    }

    /// Drive ONE real Codex chat turn, fail-closed-fast.
    ///
    /// When the gate is closed this returns IMMEDIATELY with
    /// [`CodexLiveChatError::GateClosed`] -- no spawn, no wait. When open it spawns
    /// the read-only one-shot `codex exec --json`, waits (bounded), scans + parses
    /// stdout, and reduces it to a provider-neutral [`TurnOutput`].
    pub fn try_send_turn(
        &self,
        session: &AdapterSession,
        request: &TurnRequest,
    ) -> Result<TurnOutput, CodexLiveChatError> {
        // FAIL CLOSED FAST: decide before touching any process. This is the same
        // posture operator-control takes for an attached Codex without the
        // live opt-in.
        if !codex_live_chat_gate_open() {
            let mut missing_env = Vec::new();
            if !env_flag(CODEX_LIVE_PREFLIGHT_OPT_IN_ENV) {
                missing_env.push(CODEX_LIVE_PREFLIGHT_OPT_IN_ENV);
            }
            if !env_flag(CODEX_LIVE_RUN_OPT_IN_ENV) {
                missing_env.push(CODEX_LIVE_RUN_OPT_IN_ENV);
            }
            return Err(CodexLiveChatError::GateClosed {
                agent_name: request.agent_name.clone(),
                missing_env,
            });
        }

        let events = self.run_one_shot(&request.goal, request.turn_id.as_str())?;
        Ok(turn_output_from_events(session, request, &events))
    }

    /// Spawn the read-only one-shot Codex, wait (bounded), scan + parse stdout.
    fn run_one_shot(
        &self,
        goal: &str,
        turn_id: &str,
    ) -> Result<Vec<NormalizedAdapterEvent>, CodexLiveChatError> {
        let mut launch_plan = CodexExecAdapter::local_launch_plan(
            self.workspace_root.clone(),
            self.artifact_root.clone(),
            goal.to_string(),
        );
        // The chat one-shot runs against a fresh, confined, non-git workspace
        // (the server's `workspace_root`), so Codex's trusted-directory/git-repo
        // guard otherwise refuses to run with "Not inside a trusted directory and
        // --skip-git-repo-check was not specified" and exits non-clean. Mirror the
        // read-only `local_smoke_plan` and `local_workspace_write_launch_plan`,
        // which add `--skip-git-repo-check` for the SAME reason, by inserting it
        // before the positional `--cd <workspace> <prompt>` args. This stays
        // read-only (`--sandbox read-only --ephemeral`) and confined (`--cd`); the
        // flag only skips the git-repo guard, it does not relax the sandbox.
        if let Some(cd_index) = launch_plan.argv.iter().position(|arg| arg == "--cd") {
            launch_plan
                .argv
                .insert(cd_index, "--skip-git-repo-check".to_string());
        }
        // Test/ops seam: an absolute override runs THAT binary (a stub for tests,
        // `CAPO_CODEX_BIN` for ops) instead of resolving `codex` from PATH. The
        // runtime spawns with `env_clear()`, so only an absolute path is honored.
        if let Some(program) = self
            .codex_program_override
            .as_deref()
            .filter(|path| std::path::Path::new(path).is_absolute())
        {
            launch_plan.program = program.to_string();
        }
        launch_plan
            .assert_subscription_safe()
            .map_err(CodexLiveChatError::Spawn)?;
        fs::create_dir_all(&launch_plan.workspace_root).map_err(|error| {
            CodexLiveChatError::Spawn(format!("failed to create chat workspace: {error}"))
        })?;
        fs::create_dir_all(&launch_plan.artifact_root).map_err(|error| {
            CodexLiveChatError::Spawn(format!("failed to create chat artifact root: {error}"))
        })?;

        let runner = LocalProcessRunner::new(launch_plan.runtime_config());
        let run_id = RunId::new(format!("codex-live-chat-{turn_id}"));
        let mut process = runner
            .spawn_process(launch_plan.runtime_request_for_turn(run_id, turn_id))
            .map_err(|error| CodexLiveChatError::Spawn(format!("{error:?}")))?;
        let outcome = runner
            .wait_running_with_timeout(&mut process, Duration::from_secs(self.timeout_seconds))
            .map_err(|error| CodexLiveChatError::Spawn(format!("wait failed: {error:?}")))?;

        if scan_artifacts_for_sensitive_markers([&outcome.stdout.path, &outcome.stderr.path])
            .is_err()
        {
            let _ = fs::remove_file(&outcome.stdout.path);
            let _ = fs::remove_file(&outcome.stderr.path);
            return Err(CodexLiveChatError::Output(
                "codex chat artifact failed the sensitive-marker scan".to_string(),
            ));
        }
        if outcome.process.status != "exited" {
            return Err(CodexLiveChatError::NonClean {
                status: outcome.process.status,
            });
        }
        let stdout = fs::read_to_string(&outcome.stdout.path)
            .map_err(|error| CodexLiveChatError::Output(format!("read stdout failed: {error}")))?;
        let parse = CodexExecAdapter::parse_jsonl(&stdout)
            .map_err(|error| CodexLiveChatError::Output(format!("{error:?}")))?;
        let events = parse.events;
        if events.is_empty() {
            return Err(CodexLiveChatError::EmptyOutput);
        }
        Ok(events)
    }
}

impl AgentAdapter for CodexLiveAdapter {
    fn binding(&self) -> BoundaryBinding {
        CodexLiveAdapter::binding(self)
    }

    fn open_session(&self, request: AdapterSessionRequest) -> AdapterSession {
        AdapterSession {
            session_id: request.session_id,
            external_session_ref: format!("codex-live-chat-session-{}", request.agent_name),
            adapter_capability: "codex-live-chat-readonly-one-shot".to_string(),
        }
    }

    /// Infallible trait entry. The Codex chat turn is fail-closed and can spawn a
    /// real process, so the fallible [`AgentAdapter::try_send_turn`] is the real
    /// seam; this infallible shim exists only to satisfy the trait and surfaces
    /// the failure inside the [`TurnOutput`] status. Callers on the chat path use
    /// `try_send_turn` and propagate the typed error.
    fn send_turn(&self, session: &AdapterSession, request: TurnRequest) -> TurnOutput {
        match self.try_send_turn(session, &request) {
            Ok(output) => output,
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

    fn try_send_turn(
        &self,
        session: &AdapterSession,
        request: &TurnRequest,
    ) -> Result<TurnOutput, CodexLiveChatError> {
        CodexLiveAdapter::try_send_turn(self, session, request)
    }

    fn attach_session(
        &self,
        session_id: SessionId,
        external_session_ref: String,
    ) -> AdapterSession {
        AdapterSession {
            session_id,
            external_session_ref,
            adapter_capability: "codex-live-chat-readonly-one-shot".to_string(),
        }
    }

    fn interrupt(&self, session: &AdapterSession, reason: &str) -> TurnOutput {
        TurnOutput {
            turn_id: TurnId::new(format!("interrupt-{}", session.session_id)),
            external_session_ref: session.external_session_ref.clone(),
            summary: format!("Codex live chat interrupted: {reason}"),
            confidence: 0,
            status: "canceled".to_string(),
            tool_name: "capo.session_summary".to_string(),
        }
    }

    fn stop(&self, session: &AdapterSession, reason: &str) -> TurnOutput {
        TurnOutput {
            turn_id: TurnId::new(format!("stop-{}", session.session_id)),
            external_session_ref: session.external_session_ref.clone(),
            summary: format!("Codex live chat stopped: {reason}"),
            confidence: 0,
            status: "completed".to_string(),
            tool_name: "capo.session_summary".to_string(),
        }
    }
}

/// Reduce a parsed Codex turn's normalized events to a provider-neutral
/// [`TurnOutput`] the controller chat path consumes.
fn turn_output_from_events(
    session: &AdapterSession,
    request: &TurnRequest,
    events: &[NormalizedAdapterEvent],
) -> TurnOutput {
    let summary = events
        .iter()
        .rev()
        .find_map(|event| event.content.clone())
        .unwrap_or_else(|| format!("Codex live chat accepted goal: {}", request.goal));
    let status = events
        .iter()
        .rev()
        .find_map(|event| {
            (event.kind == "adapter.turn_completed")
                .then(|| event.status.clone())
                .flatten()
        })
        .unwrap_or_else(|| "active".to_string());
    let tool_name = events
        .iter()
        .find_map(|event| event.tool_name.clone())
        .unwrap_or_else(|| "capo.session_summary".to_string());
    let external_session_ref = events
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

/// Test-only re-export of the Codex chat reduction so the Claude adapter's CS2
/// parity test can assert its `TurnOutput` shape equals the Codex reduction for
/// the same logical turn. Takes a fallback `external_session_ref` string so the
/// caller need not construct a full `AdapterSession`.
#[cfg(test)]
pub(crate) fn turn_output_from_events_for_test(
    fallback_session_ref: &str,
    request: &TurnRequest,
    events: &[NormalizedAdapterEvent],
) -> TurnOutput {
    let session = AdapterSession {
        session_id: SessionId::new("codex-parity"),
        external_session_ref: fallback_session_ref.to_string(),
        adapter_capability: "codex-live-chat-one-shot".to_string(),
    };
    turn_output_from_events(&session, request, events)
}
