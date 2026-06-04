//! DP4: a real Claude Code [`AgentAdapter`] as a SECOND real provider.
//!
//! This lifts Claude out of the no-tools `--permission-mode plan` profile (the
//! `ClaudeCodeAdapter::local_launch_plan`) into a workspace-write profile and
//! validates the [`AgentAdapter`] trait against a second real provider beside
//! [`crate::CodexLiveAdapter`] -- breadth, not a re-architecture of the loop.
//!
//! The design mirrors the Codex chat adapter exactly so the trait seam stays
//! provider-neutral:
//!
//! - A Claude-BOUND agent (registered with [`ClaudeCodeLiveAdapter`]) drives the
//!   real, confined `claude -p --output-format stream-json --verbose` one-shot on
//!   its chat turn -- but FAIL-CLOSED-FAST:
//!     - when [`claude_live_chat_gate_open`] is TRUE the turn spawns the real
//!       workspace-write `claude` one-shot, waits (bounded), scans + parses its
//!       `stream-json` into a provider-neutral [`TurnOutput`];
//!     - when it is FALSE the turn returns an IMMEDIATE typed
//!       [`CodexLiveChatError::GateClosed`] -- NO process spawn, NO blocking, NO
//!       waiting -- mirroring the Codex adapter's fail-closed posture.
//!
//! The gate mirrors `CAPO_SERVER_RUN_CODEX_LIVE` with a Claude-specific
//! [`CLAUDE_LIVE_RUN_OPT_IN_ENV`] (`CAPO_SERVER_RUN_CLAUDE_LIVE`) AND the shared
//! live-provider preflight (`CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT`); BOTH must hold
//! for the live write to spawn. The typed error reuses [`CodexLiveChatError`] so
//! the trait's `try_send_turn` seam stays one provider-neutral error type across
//! both real providers (the message carries the agent + the missing Claude env).

use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use capo_core::{BoundaryBinding, BoundaryKind, RunId, SessionId, TurnId};
use capo_runtime::LocalProcessRunner;

use crate::codex_live::{CODEX_LIVE_PREFLIGHT_OPT_IN_ENV, CodexLiveChatError};
use crate::{
    AdapterSession, AdapterSessionRequest, AgentAdapter, ClaudeCodeAdapter, NormalizedAdapterEvent,
    TurnOutput, TurnRequest, scan_artifacts_for_sensitive_markers,
};

/// The Claude live-write opt-in gate, mirroring `CAPO_SERVER_RUN_CODEX_LIVE`.
/// Claude's live write spawns only when BOTH this and the shared live-provider
/// preflight gate are set to `1`.
pub const CLAUDE_LIVE_RUN_OPT_IN_ENV: &str = "CAPO_SERVER_RUN_CLAUDE_LIVE";

/// Whether the real-Claude chat path is open.
///
/// Returns `true` only when BOTH the shared live-provider preflight opt-in
/// (`CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT`) AND the Claude live-write opt-in
/// (`CAPO_SERVER_RUN_CLAUDE_LIVE`) are explicitly set to `1`. When this is
/// `false`, a Claude-bound chat turn fails closed FAST (an immediate typed
/// error) without spawning anything.
pub fn claude_live_chat_gate_open() -> bool {
    env_flag(CODEX_LIVE_PREFLIGHT_OPT_IN_ENV) && env_flag(CLAUDE_LIVE_RUN_OPT_IN_ENV)
}

fn env_flag(name: &str) -> bool {
    std::env::var(name).as_deref() == Ok("1")
}

/// A real Claude Code `AgentAdapter` whose chat turn drives the confined
/// workspace-write `claude -p --output-format stream-json --verbose` one-shot
/// (gate-respecting, fail-closed-fast).
///
/// This is a CLAUDE-BOUND handle: it is installed only for agents explicitly
/// bound to the Claude adapter. The fake/scripted/Codex handles are unaffected.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaudeCodeLiveAdapter {
    workspace_root: PathBuf,
    artifact_root: PathBuf,
    /// Absolute path to a `claude` binary to run instead of resolving `claude`
    /// from PATH. Ops set it from `CAPO_CLAUDE_BIN`; tests pin an executable stub
    /// by absolute path. `None` keeps `claude`. Honored only when absolute,
    /// because the runtime spawns with `env_clear()`.
    claude_program_override: Option<String>,
    /// Bounded wall-clock for the one-shot, so the chat path can never block the
    /// server request handler unbounded even when the gate is open.
    timeout_seconds: u64,
}

impl ClaudeCodeLiveAdapter {
    /// Open the Claude chat adapter confined to `workspace_root` with artifacts
    /// under `artifact_root`.
    pub fn new(workspace_root: impl Into<PathBuf>, artifact_root: impl Into<PathBuf>) -> Self {
        Self {
            workspace_root: workspace_root.into(),
            artifact_root: artifact_root.into(),
            claude_program_override: None,
            timeout_seconds: 300,
        }
    }

    /// Pin an absolute-path claude program override (ops `CAPO_CLAUDE_BIN`; tests
    /// a deterministic stub). Non-absolute values are ignored at spawn time
    /// because the runtime spawns with `env_clear()`.
    #[must_use]
    pub fn with_claude_program_override(mut self, program: impl Into<String>) -> Self {
        self.claude_program_override = Some(program.into());
        self
    }

    /// Set the bounded wall-clock timeout for the one-shot.
    #[must_use]
    pub fn with_timeout_seconds(mut self, timeout_seconds: u64) -> Self {
        self.timeout_seconds = timeout_seconds;
        self
    }

    /// The provider-neutral boundary binding for the real Claude chat adapter.
    /// `fake: false` -- this is a real provider binding, distinct from the
    /// fake/scripted handles and from the Codex binding.
    pub fn binding(&self) -> BoundaryBinding {
        BoundaryBinding {
            kind: BoundaryKind::AgentAdapter,
            variant: "claude-live",
            fake: false,
        }
    }

    /// Drive ONE real Claude chat turn, fail-closed-fast.
    ///
    /// When the gate is closed this returns IMMEDIATELY with
    /// [`CodexLiveChatError::GateClosed`] -- no spawn, no wait. When open it spawns
    /// the confined workspace-write one-shot, waits (bounded), scans + parses
    /// stdout `stream-json`, and reduces it to a provider-neutral [`TurnOutput`].
    pub fn try_send_turn(
        &self,
        session: &AdapterSession,
        request: &TurnRequest,
    ) -> Result<TurnOutput, CodexLiveChatError> {
        // FAIL CLOSED FAST: decide before touching any process, mirroring the
        // Codex adapter's posture.
        if !claude_live_chat_gate_open() {
            let mut missing_env = Vec::new();
            if !env_flag(CODEX_LIVE_PREFLIGHT_OPT_IN_ENV) {
                missing_env.push(CODEX_LIVE_PREFLIGHT_OPT_IN_ENV);
            }
            if !env_flag(CLAUDE_LIVE_RUN_OPT_IN_ENV) {
                missing_env.push(CLAUDE_LIVE_RUN_OPT_IN_ENV);
            }
            return Err(CodexLiveChatError::GateClosed {
                agent_name: request.agent_name.clone(),
                missing_env,
            });
        }

        let events = self.run_one_shot(&request.goal, request.turn_id.as_str())?;
        Ok(turn_output_from_events(session, request, &events))
    }

    /// Spawn the confined workspace-write Claude one-shot, wait (bounded), scan +
    /// parse stdout `stream-json`.
    fn run_one_shot(
        &self,
        goal: &str,
        turn_id: &str,
    ) -> Result<Vec<NormalizedAdapterEvent>, CodexLiveChatError> {
        let mut launch_plan = ClaudeCodeAdapter::local_workspace_write_launch_plan(
            self.workspace_root.clone(),
            self.artifact_root.clone(),
            goal.to_string(),
        );
        // Test/ops seam: an absolute override runs THAT binary (a stub for tests,
        // `CAPO_CLAUDE_BIN` for ops) instead of resolving `claude` from PATH. The
        // runtime spawns with `env_clear()`, so only an absolute path is honored.
        if let Some(program) = self
            .claude_program_override
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
        let run_id = RunId::new(format!("claude-live-chat-{turn_id}"));
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
                "claude chat artifact failed the sensitive-marker scan".to_string(),
            ));
        }
        if outcome.process.status != "exited" {
            return Err(CodexLiveChatError::NonClean {
                status: outcome.process.status,
            });
        }
        let stdout = fs::read_to_string(&outcome.stdout.path)
            .map_err(|error| CodexLiveChatError::Output(format!("read stdout failed: {error}")))?;
        let parse = ClaudeCodeAdapter::parse_stream_json(&stdout)
            .map_err(|error| CodexLiveChatError::Output(format!("{error:?}")))?;
        let events = parse.events;
        if events.is_empty() {
            return Err(CodexLiveChatError::EmptyOutput);
        }
        Ok(events)
    }
}

impl AgentAdapter for ClaudeCodeLiveAdapter {
    fn binding(&self) -> BoundaryBinding {
        ClaudeCodeLiveAdapter::binding(self)
    }

    fn open_session(&self, request: AdapterSessionRequest) -> AdapterSession {
        AdapterSession {
            session_id: request.session_id,
            external_session_ref: format!("claude-live-session-{}", request.agent_name),
            adapter_capability: "claude-live-workspace-write-one-shot".to_string(),
        }
    }

    /// Infallible trait entry. The Claude chat turn is fail-closed and can spawn a
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
        ClaudeCodeLiveAdapter::try_send_turn(self, session, request)
    }

    fn attach_session(
        &self,
        session_id: SessionId,
        external_session_ref: String,
    ) -> AdapterSession {
        AdapterSession {
            session_id,
            external_session_ref,
            adapter_capability: "claude-live-workspace-write-one-shot".to_string(),
        }
    }

    fn interrupt(&self, session: &AdapterSession, reason: &str) -> TurnOutput {
        TurnOutput {
            turn_id: TurnId::new(format!("interrupt-{}", session.session_id)),
            external_session_ref: session.external_session_ref.clone(),
            summary: format!("Claude live chat interrupted: {reason}"),
            confidence: 0,
            status: "canceled".to_string(),
            tool_name: "capo.session_summary".to_string(),
        }
    }

    fn stop(&self, session: &AdapterSession, reason: &str) -> TurnOutput {
        TurnOutput {
            turn_id: TurnId::new(format!("stop-{}", session.session_id)),
            external_session_ref: session.external_session_ref.clone(),
            summary: format!("Claude live chat stopped: {reason}"),
            confidence: 0,
            status: "completed".to_string(),
            tool_name: "capo.session_summary".to_string(),
        }
    }
}

/// Reduce a parsed Claude turn's normalized events to a provider-neutral
/// [`TurnOutput`] the controller chat path consumes.
///
/// This is the SAME reduction shape as the Codex adapter's
/// `turn_output_from_events`: the last item content becomes the summary, the
/// `adapter.turn_completed` status (the Claude `result.subtype`) becomes the
/// status, the first observed tool name and the Claude `session-id`
/// (`external_session_ref`) carry through. Keeping the shape identical is the
/// trait-seam parity DP4 requires: a second provider feeds the loop through the
/// exact same provider-neutral [`TurnOutput`] without any new vocabulary.
fn turn_output_from_events(
    session: &AdapterSession,
    request: &TurnRequest,
    events: &[NormalizedAdapterEvent],
) -> TurnOutput {
    let summary = events
        .iter()
        .rev()
        .find_map(|event| event.content.clone())
        .unwrap_or_else(|| format!("Claude live chat accepted goal: {}", request.goal));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CodexExecAdapter;

    /// Serializes the process-global env-gate mutation across the gated tests in
    /// this module so concurrent test threads never observe a half-set gate.
    static CLAUDE_LIVE_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn session(agent: &str) -> AdapterSession {
        ClaudeCodeLiveAdapter::new("/tmp/capo-claude-ws", "/tmp/capo-claude-art").open_session(
            AdapterSessionRequest {
                session_id: SessionId::new(format!("session-{agent}")),
                agent_name: agent.to_string(),
            },
        )
    }

    fn turn(agent: &str, goal: &str) -> TurnRequest {
        TurnRequest {
            turn_id: TurnId::new("turn-claude-1"),
            agent_name: agent.to_string(),
            goal: goal.to_string(),
        }
    }

    /// A deterministic Claude `stream-json` fixture covering a `system`
    /// session-start, an `assistant` message (with usage), a `tool_use`, and a
    /// terminal `result` -- the same record families the Codex parser maps.
    const CLAUDE_STREAM_JSON: &str = concat!(
        "{\"type\":\"system\",\"session_id\":\"claude-sess-1\"}\n",
        "{\"type\":\"assistant\",\"session_id\":\"claude-sess-1\",\"message\":{\"id\":\"msg-1\",\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"applied the workspace edit\"}],\"usage\":{\"input_tokens\":11,\"output_tokens\":7}}}\n",
        "{\"type\":\"tool_use\",\"session_id\":\"claude-sess-1\",\"id\":\"tool-1\",\"name\":\"Edit\"}\n",
        "{\"type\":\"result\",\"session_id\":\"claude-sess-1\",\"subtype\":\"success\",\"usage\":{\"input_tokens\":11,\"output_tokens\":7}}\n",
    );

    #[test]
    fn claude_live_adapter_reports_real_provider_binding() {
        let adapter = ClaudeCodeLiveAdapter::new("/tmp/ws", "/tmp/art");
        let binding = adapter.binding();
        assert_eq!(binding.kind, BoundaryKind::AgentAdapter);
        assert_eq!(binding.variant, "claude-live");
        assert!(!binding.fake, "claude-live is a real provider binding");
    }

    /// CS2 gate-OFF fail-closed-fast (KEPT, landed under DP4): with neither gate
    /// set, `try_send_turn` returns `GateClosed { missing_env: [PREFLIGHT,
    /// RUN_CLAUDE_LIVE] }` IMMEDIATELY and spawns nothing (the program override
    /// points at a nonexistent binary that must never run), and the infallible
    /// `send_turn` shim surfaces a `blocked` turn with `confidence: 0`. This is
    /// CONFIRMED/KEPT, not new CS2 work.
    #[test]
    fn claude_send_turn_fails_closed_fast_when_gate_off() {
        let _guard = CLAUDE_LIVE_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        unsafe {
            std::env::remove_var(CODEX_LIVE_PREFLIGHT_OPT_IN_ENV);
            std::env::remove_var(CLAUDE_LIVE_RUN_OPT_IN_ENV);
        }
        // Pin a program that must never spawn; the gate is off, so it never does.
        let adapter = ClaudeCodeLiveAdapter::new("/tmp/ws", "/tmp/art")
            .with_claude_program_override("/nonexistent/claude-must-never-spawn");
        let session = session("worker");
        let error = adapter
            .try_send_turn(&session, &turn("worker", "do the thing"))
            .expect_err("gate off must fail closed");
        match error {
            CodexLiveChatError::GateClosed {
                agent_name,
                missing_env,
            } => {
                assert_eq!(agent_name, "worker");
                assert!(missing_env.contains(&CODEX_LIVE_PREFLIGHT_OPT_IN_ENV));
                assert!(missing_env.contains(&CLAUDE_LIVE_RUN_OPT_IN_ENV));
            }
            other => panic!("expected GateClosed, got {other:?}"),
        }
        // The infallible shim surfaces the failure as a blocked turn, never a
        // fabricated summary.
        let blocked = adapter.send_turn(&session, turn("worker", "do the thing"));
        assert_eq!(blocked.status, "blocked");
        assert_eq!(blocked.confidence, 0);
    }

    #[test]
    fn claude_normalized_events_match_codex_trait_seam_shape() {
        // DP4 trait-seam parity: a Claude `stream-json` turn and a Codex `--json`
        // turn that describe the SAME logical turn (session start, an assistant
        // message, a tool call, a completed turn) normalize to the SAME ordered
        // `NormalizedAdapterEvent` KINDS, so the loop ingests a second provider
        // through the identical provider-neutral seam.
        let codex_jsonl = concat!(
            "{\"type\":\"thread.started\",\"thread_id\":\"codex-sess-1\"}\n",
            "{\"type\":\"item.completed\",\"item\":{\"id\":\"item-1\",\"type\":\"agent_message\",\"text\":\"applied the workspace edit\"}}\n",
            "{\"type\":\"tool_call.started\",\"call_id\":\"call-1\",\"tool_name\":\"apply_patch\"}\n",
            "{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":11,\"output_tokens\":7}}\n",
        );

        let claude_events = ClaudeCodeAdapter::parse_stream_json(CLAUDE_STREAM_JSON)
            .expect("claude parse")
            .events;
        let codex_events = CodexExecAdapter::parse_jsonl(codex_jsonl)
            .expect("codex parse")
            .events;

        let claude_kinds: Vec<&str> = claude_events.iter().map(|e| e.kind.as_str()).collect();
        let codex_kinds: Vec<&str> = codex_events.iter().map(|e| e.kind.as_str()).collect();
        assert_eq!(
            claude_kinds, codex_kinds,
            "Claude and Codex must normalize the same logical turn to the same event kinds"
        );
        assert_eq!(
            claude_kinds,
            vec![
                "adapter.session_started",
                "adapter.item_completed",
                "adapter.tool_call_started",
                "adapter.turn_completed",
            ]
        );

        // Both reduce to a provider-neutral TurnOutput with the same shape.
        let session = session("parity");
        let request = turn("parity", "edit the file");
        let claude_out = turn_output_from_events(&session, &request, &claude_events);
        assert_eq!(claude_out.summary, "applied the workspace edit");
        assert_eq!(claude_out.status, "success");
        assert_eq!(claude_out.tool_name, "Edit");
        assert_eq!(claude_out.external_session_ref, "claude-sess-1");
        assert_eq!(claude_out.confidence, 80);
        assert_eq!(claude_out.turn_id.as_str(), "turn-claude-1");
    }

    /// CS2: drive `try_send_turn` (gate ON) through a pinned absolute-path stub
    /// spawned by `LocalProcessRunner`, and assert the reduced `TurnOutput` shape
    /// equals the Codex `turn_output_from_events` reduction for the SAME logical
    /// turn. This proves the real chat path (gate -> spawn -> parse -> reduce)
    /// end-to-end against a deterministic binary, not a fabricated summary.
    #[test]
    fn claude_try_send_turn_stub_matches_codex_turn_output_reduction() {
        let _guard = CLAUDE_LIVE_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _env = GateGuard::open();

        let root = temp_root("claude-cs2-try-send");
        std::fs::create_dir_all(&root).expect("root");
        // A stub that ignores its args (it receives the workspace-write profile
        // argv ending in the prompt) and prints the fixture `stream-json` on
        // stdout. The runtime spawns with `env_clear()`, so the stub uses only
        // POSIX builtins and an absolute fixture path.
        let fixture = root.join("stream.jsonl");
        std::fs::write(&fixture, CLAUDE_STREAM_JSON).expect("fixture");
        let stub = root.join("claude-stub.sh");
        std::fs::write(
            &stub,
            format!(
                "#!/bin/sh\nwhile IFS= read -r line; do printf '%s\\n' \"$line\"; done < '{}'\n",
                fixture.display()
            ),
        )
        .expect("stub");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&stub).expect("meta").permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&stub, perms).expect("chmod");
        }

        let adapter = ClaudeCodeLiveAdapter::new(root.join("ws"), root.join("art"))
            .with_claude_program_override(stub.to_string_lossy().to_string());
        let session = session("worker");
        let request = turn("worker", "edit the file");
        let out = adapter
            .try_send_turn(&session, &request)
            .expect("gate open: stub turn must succeed");

        // The SAME reduction the Codex chat adapter performs on the SAME logical
        // turn: summary = last item content, status = result.subtype, first tool,
        // session-id as external_session_ref, confidence 80.
        assert_eq!(out.summary, "applied the workspace edit");
        assert_eq!(out.status, "success");
        assert_eq!(out.tool_name, "Edit");
        assert_eq!(out.external_session_ref, "claude-sess-1");
        assert_eq!(out.confidence, 80);
        assert_eq!(out.turn_id.as_str(), "turn-claude-1");

        // Pin equality against the Codex reduction directly: parse the equivalent
        // Codex turn and reduce it through the SAME provider-neutral shape.
        let codex_jsonl = concat!(
            "{\"type\":\"thread.started\",\"thread_id\":\"claude-sess-1\"}\n",
            "{\"type\":\"item.completed\",\"item\":{\"id\":\"item-1\",\"type\":\"agent_message\",\"text\":\"applied the workspace edit\"}}\n",
            "{\"type\":\"tool_call.started\",\"call_id\":\"call-1\",\"tool_name\":\"Edit\"}\n",
            "{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":11,\"output_tokens\":7}}\n",
        );
        let codex_events = CodexExecAdapter::parse_jsonl(codex_jsonl)
            .expect("codex parse")
            .events;
        let codex_out = crate::codex_live::turn_output_from_events_for_test(
            &session.external_session_ref,
            &request,
            &codex_events,
        );
        // Same provider-neutral fields the loop consumes (status differs only by
        // the providers' native subtype vocabulary, which both map identically
        // here: Claude `result.subtype=success`, Codex `turn.completed`).
        assert_eq!(out.summary, codex_out.summary);
        assert_eq!(out.tool_name, codex_out.tool_name);
        assert_eq!(out.external_session_ref, codex_out.external_session_ref);
        assert_eq!(out.confidence, codex_out.confidence);
        assert_eq!(out.turn_id.as_str(), codex_out.turn_id.as_str());
    }

    /// CS2 argv parity: pin the EXACT profile the live chat adapter invokes
    /// (`local_workspace_write_launch_plan`) and assert the SEPARATE read-bounded
    /// `local_launch_plan` profile exists and is distinct, so the two are never
    /// conflated.
    #[test]
    fn claude_launch_profiles_pin_exact_argv() {
        let ws = PathBuf::from("/tmp/capo-cs2-ws");
        let art = PathBuf::from("/tmp/capo-cs2-art");

        // The profile the LIVE chat adapter and the dispatch write arm share.
        let write = ClaudeCodeAdapter::local_workspace_write_launch_plan(
            ws.clone(),
            art.clone(),
            "edit the file",
        );
        assert_eq!(write.program, "claude");
        assert_eq!(
            write.argv,
            vec![
                "-p",
                "--output-format",
                "stream-json",
                "--verbose",
                "--permission-mode",
                "acceptEdits",
                "--no-session-persistence",
                "--disable-slash-commands",
                "--mcp-config",
                "/dev/null",
                "--strict-mcp-config",
                "--add-dir",
                "/tmp/capo-cs2-ws",
                "edit the file",
            ]
        );

        // The SEPARATE read-bounded profile: `plan` mode, no tools, NO `--add-dir`.
        let plan = ClaudeCodeAdapter::local_launch_plan(ws, art, "edit the file");
        assert_eq!(
            plan.argv,
            vec![
                "-p",
                "--output-format",
                "stream-json",
                "--verbose",
                "--permission-mode",
                "plan",
                "--no-session-persistence",
                "--disable-slash-commands",
                "--tools",
                "",
                "--disallowedTools",
                "*",
                "--mcp-config",
                "/dev/null",
                "--strict-mcp-config",
                "edit the file",
            ]
        );
        // The two profiles are distinct (mode + `--add-dir` presence).
        assert!(write.argv.iter().any(|a| a == "acceptEdits"));
        assert!(write.argv.iter().any(|a| a == "--add-dir"));
        assert!(plan.argv.iter().any(|a| a == "plan"));
        assert!(!plan.argv.iter().any(|a| a == "--add-dir"));
    }

    fn temp_root(name: &str) -> capo_tmptest::TempRoot {
        capo_tmptest::TempRoot::new(&format!("capo-adapter-{name}"))
    }

    /// A Drop guard that opens BOTH chat gates for the duration of a test and
    /// restores the prior env on drop -- even on a panic mid-test -- so the
    /// process-global gate never leaks into other tests in this binary.
    struct GateGuard {
        prev_preflight: Option<String>,
        prev_run: Option<String>,
    }

    impl GateGuard {
        fn open() -> Self {
            let prev_preflight = std::env::var(CODEX_LIVE_PREFLIGHT_OPT_IN_ENV).ok();
            let prev_run = std::env::var(CLAUDE_LIVE_RUN_OPT_IN_ENV).ok();
            unsafe {
                std::env::set_var(CODEX_LIVE_PREFLIGHT_OPT_IN_ENV, "1");
                std::env::set_var(CLAUDE_LIVE_RUN_OPT_IN_ENV, "1");
            }
            Self {
                prev_preflight,
                prev_run,
            }
        }
    }

    impl Drop for GateGuard {
        fn drop(&mut self) {
            unsafe {
                match &self.prev_preflight {
                    Some(v) => std::env::set_var(CODEX_LIVE_PREFLIGHT_OPT_IN_ENV, v),
                    None => std::env::remove_var(CODEX_LIVE_PREFLIGHT_OPT_IN_ENV),
                }
                match &self.prev_run {
                    Some(v) => std::env::set_var(CLAUDE_LIVE_RUN_OPT_IN_ENV, v),
                    None => std::env::remove_var(CLAUDE_LIVE_RUN_OPT_IN_ENV),
                }
            }
        }
    }

    #[test]
    fn claude_turn_completed_carries_usage_tokens_and_session_ref() {
        let events = ClaudeCodeAdapter::parse_stream_json(CLAUDE_STREAM_JSON)
            .expect("claude parse")
            .events;
        let completed = events
            .iter()
            .find(|e| e.kind == "adapter.turn_completed")
            .expect("turn completed event");
        assert_eq!(
            completed.external_session_ref.as_deref(),
            Some("claude-sess-1")
        );
        assert_eq!(completed.input_tokens, Some(11));
        assert_eq!(completed.output_tokens, Some(7));
    }
}
