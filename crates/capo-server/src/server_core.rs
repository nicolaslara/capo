use capo_controller::FakeRunRefs;
use capo_core::{CommandEnvelope, CommandId, CommandIntent, CommandTarget, RunId, SessionId};
use capo_state::{AgentProjection, EventKind, NewEvent, RunProjection, SessionProjection};

use crate::util::{adapter_kind_for_events, command_identity_hash, slug, stable_hash};
use crate::{
    AcpLiveTurnSummary, CapoServer, RecoverySummary, ServerClientOrigin, ServerError,
    ServerResponse, ServerResponsePayload, ServerResult,
};

/// COOPERATIVE CANCEL (B2): RAII guard that deregisters an in-flight live turn
/// from the [`CapoServer`] registry on EVERY exit path of the driving function
/// (normal return, `?`-propagated error, or panic-unwind), so a finished turn can
/// never be left as a dangling cancel target.
struct DeregGuard<'a> {
    server: &'a CapoServer,
    session_id: String,
}

impl Drop for DeregGuard<'_> {
    fn drop(&mut self) {
        self.server.deregister_in_flight(&self.session_id);
    }
}

impl CapoServer {
    pub(crate) fn recover_server(
        &self,
        request_id: &str,
        origin: &ServerClientOrigin,
    ) -> ServerResult<RecoverySummary> {
        let command_hash = command_identity_hash("recover".to_string());
        let command = self.command_envelope(
            request_id,
            origin,
            &command_hash,
            CommandTarget::Project(self.project_id.clone()),
            CommandIntent::Recover,
            None,
        );
        let report = self
            .command_controller()
            .recover_command(&command)
            .map_err(ServerError::State)?;
        self.record_server_request_handled(&command, origin, "recover", None, None)
            .map_err(ServerError::State)?;
        Ok(RecoverySummary {
            recovery_attempt_id: report.recovery_attempt_id,
            recovered_run_count: report.recovered_run_count,
            watermark: report.watermark,
        })
    }

    pub(crate) fn run_refs_for_session_run(
        &self,
        session_id: &SessionId,
        run_id: &RunId,
    ) -> ServerResult<(
        SessionProjection,
        RunProjection,
        AgentProjection,
        FakeRunRefs,
    )> {
        let session = self
            .controller
            .state()
            .session(session_id)
            .map_err(ServerError::State)?
            .ok_or_else(|| ServerError::UnknownSession {
                session_id: session_id.to_string(),
            })?;
        let run = self
            .controller
            .state()
            .run(run_id)
            .map_err(ServerError::State)?
            .ok_or_else(|| ServerError::UnknownSession {
                session_id: session_id.to_string(),
            })?;
        if run.session_id != *session_id {
            return Err(ServerError::RunSessionMismatch {
                session_id: session_id.to_string(),
                run_id: run_id.to_string(),
                actual_session_id: run.session_id.to_string(),
            });
        }
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
        let task_id = session.task_id.clone().ok_or_else(|| {
            ServerError::AdapterFixture(format!("session missing task id: {}", session.session_id))
        })?;
        let refs = FakeRunRefs {
            task_id,
            agent_id: session.agent_id.clone(),
            session_id: session.session_id.clone(),
            run_id: run.run_id.clone(),
            runtime_process_ref: format!("server-session-runtime-{}", run.run_id),
            external_session_ref: format!("server-adapter-session-{}", session.session_id),
        };
        Ok((session, run, agent, refs))
    }

    pub(crate) fn command_envelope(
        &self,
        request_id: &str,
        origin: &ServerClientOrigin,
        command_hash: &str,
        target: CommandTarget,
        intent: CommandIntent,
        text: Option<String>,
    ) -> CommandEnvelope {
        let mut command = CommandEnvelope::new(
            CommandId::new(request_id),
            origin.input_origin.into(),
            origin.actor_id.clone(),
            self.project_id.clone(),
            target,
            intent,
        );
        command.idempotency_key = format!(
            "server:{}:{}:{}:{}",
            origin.client_id, origin.actor_id, request_id, command_hash
        );
        if let Some(text) = text {
            command = command.with_text(text);
        }
        command
    }

    pub(crate) fn response(
        &self,
        request_id: String,
        origin: ServerClientOrigin,
        payload: ServerResponsePayload,
    ) -> ServerResult<ServerResponse> {
        Ok(ServerResponse {
            request_id,
            client_id: origin.client_id,
            actor_id: origin.actor_id,
            input_origin: origin.input_origin,
            payload,
        })
    }

    pub(crate) fn require_session_adapter(
        &self,
        session_id: &SessionId,
        requested_adapter: &str,
    ) -> ServerResult<()> {
        let session_adapter = self
            .controller
            .state()
            .recent_events_for_session(session_id, 200)
            .map_err(ServerError::State)
            .and_then(|events| {
                adapter_kind_for_events(&events).ok_or_else(|| {
                    ServerError::AdapterFixture(format!(
                        "session missing adapter metadata: {session_id}"
                    ))
                })
            })?;
        if session_adapter != requested_adapter {
            return Err(ServerError::AdapterSessionMismatch {
                session_id: session_id.to_string(),
                session_adapter,
                requested_adapter: requested_adapter.to_string(),
            });
        }
        Ok(())
    }

    pub(crate) fn record_server_request_handled(
        &self,
        command: &CommandEnvelope,
        origin: &ServerClientOrigin,
        command_kind: &str,
        run: Option<&FakeRunRefs>,
        extra_payload: Option<serde_json::Value>,
    ) -> capo_state::StateResult<i64> {
        let event_id = format!(
            "event-server-request-{}-{}",
            slug(command.command_id.as_str()),
            stable_hash(command.idempotency_key.as_bytes())
        );
        let mut event = NewEvent::new(event_id, EventKind::ServerRequestHandled, &origin.actor_id);
        event.project_id = Some(self.project_id.clone());
        event.item_id = Some(command.command_id.to_string());
        event.idempotency_key = Some(command.idempotency_key.clone());
        if let Some(run) = run {
            event.task_id = Some(run.task_id.clone());
            event.agent_id = Some(run.agent_id.clone());
            event.session_id = Some(run.session_id.clone());
            event.run_id = Some(run.run_id.clone());
        }
        let mut payload = serde_json::json!({
            "request_id": command.command_id.to_string(),
            "client_id": origin.client_id,
            "actor_id": origin.actor_id,
            "input_origin": format!("{:?}", origin.input_origin),
            "command_kind": command_kind,
            "idempotency_key": command.idempotency_key,
        });
        if let Some(extra_payload) = extra_payload
            && let (Some(payload), Some(extra_payload)) =
                (payload.as_object_mut(), extra_payload.as_object())
        {
            for (key, value) in extra_payload {
                payload.insert(key.clone(), value.clone());
            }
        }
        event.payload_json = payload.to_string();
        self.controller.state().append_event(event, &[])
    }

    /// SLICE-A: drive ONE live ACP turn through the controller's
    /// `drive_acp_live_turn` seam, confined to a working directory, behind the
    /// existing live ACP env gate. This is the server-level wiring that reaches
    /// the previously test-only `AcpLiveAdapter` + `drive_acp_live_turn` path and
    /// produces an OBSERVED file change in the confined workspace.
    ///
    /// For this slice the agent is a LOCAL stub program spawned through the
    /// runtime (deterministic, `env_clear()`, no network), NOT the live `npx` ACP
    /// bridge. It is spawned via `AcpLiveAdapter::spawn_live_session`, which
    /// self-checks the same gate; we ALSO check the gate up front so a closed gate
    /// fails closed before any work.
    pub(crate) fn run_acp_live_turn_local(
        &self,
        request_id: String,
        origin: ServerClientOrigin,
        req: AcpLiveTurnLocalRequest,
    ) -> ServerResult<ServerResponse> {
        use capo_adapters::{AcpAdapter, AcpLiveAdapter, acp_live_gate_open};
        use capo_core::TurnId;

        // GATE: fail closed unless BOTH the explicit per-command opt-in AND the
        // env gate (`CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT=1` +
        // `CAPO_SERVER_RUN_ACP_LIVE=1`) hold. Default behavior is unchanged.
        if !req.live_acp_opt_in || !acp_live_gate_open() {
            return Err(ServerError::AdapterFixture(
                "acp live turn is fail-closed: set live_acp_opt_in AND \
                 CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT=1 CAPO_SERVER_RUN_ACP_LIVE=1"
                    .to_string(),
            ));
        }

        let session_id = SessionId::new(req.session_id);
        let run_id = RunId::new(req.run_id);
        let (_session, _run, _agent, refs) =
            self.run_refs_for_session_run(&session_id, &run_id)?;

        // The confined working directory: the optional command path (a worktree)
        // or the project-dir default under the server state root.
        let workspace_root = req
            .workspace_root
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| self.codex_chat.acp_workspace_root.clone());
        let artifact_root = self.codex_chat.acp_artifact_root.clone();

        // Build the ACP adapter exactly as the DP11 path does: a read-only-local
        // policy (the controller is the wire permission authority), the runtime
        // tool wrappers' tool list, confined to `workspace_root`/`artifact_root`.
        let wrappers = capo_tools::RuntimeToolWrappers::new(
            capo_tools::RuntimeToolConfig::local_workspace(
                workspace_root.clone(),
                artifact_root.clone(),
            ),
        );
        // Default (the `/bin/sh` stub path, `acp_session_mode == None`): a
        // read-only-local policy that does NOT advertise `fs.writeTextFile` -- the
        // stub writes the file itself, no wire callback. The LIVE bridge profile
        // (`acp_session_mode == Some`) instead uses a trusted-local policy that
        // advertises `fs.writeTextFile` (so the agent's Write tool routes the write
        // back over the wire) and carries the session mode the live driver switches
        // to before prompting (so the real bridge emits an on-wire write callback
        // rather than simulating the tool in its default mode).
        let policy = if req.acp_session_mode.is_some() {
            capo_tools::PermissionPolicy::allow_trusted_local()
        } else {
            capo_tools::PermissionPolicy::static_read_only_local()
        };
        let mut setup_plan = AcpAdapter::session_setup_plan(
            &wrappers.list_tools(),
            &policy,
            SessionId::new(format!("acp-setup-{session_id}")),
        );
        if let Some(mode) = req.acp_session_mode.clone() {
            // The live bridge delegates its Write/Read tools to the client over
            // the wire and only ACKs them once the client performs the fs op, so
            // the wire client must EXECUTE the write/read confined to the
            // workspace. The stub path leaves this unset (the stub writes its own
            // disk).
            setup_plan = setup_plan
                .with_session_mode(mode)
                .with_workspace_root(workspace_root.clone());
        }
        // Optional worker-result channel: forward capo's in-process MCP endpoint
        // into the worker's `session/new` so it can call `report_result`. Default
        // (`None`) advertises NO MCP server to the worker, keeping the validated
        // file-only worker loop byte-identical (mirrors the conductor turn's
        // `with_http_mcp_server`).
        if let Some(mcp_url) = req.mcp_url.clone() {
            setup_plan = setup_plan.with_http_mcp_server(mcp_url, req.mcp_headers.clone());
        }
        let adapter = AcpLiveAdapter::new(
            req.acp_program,
            req.acp_argv,
            workspace_root.clone(),
            artifact_root,
            setup_plan,
        );

        let turn_id = TurnId::new(req.turn_id.clone());
        // Spawn the LOCAL ACP agent through the runtime (gate self-checked).
        let mut session = adapter
            .spawn_live_session(&turn_id)
            .map_err(|error| ServerError::AdapterFixture(format!("acp live spawn: {error}")))?;
        let transport = session
            .take_transport()
            .ok_or_else(|| ServerError::AdapterFixture("acp live transport already taken".into()))?;

        // Register this turn under the Capo session_id (the SAME key
        // InterruptAgent/StopAgent/SteerAgent resolve). A POSITIVE steer window
        // makes the session PERSISTENT + steerable (register the steer channel);
        // a 0 window keeps the one-shot path (no steer channel). The RAII guard
        // deregisters on every exit path. With no command arriving, the cancel
        // flag stays false and the initial prompt is byte-identical to the
        // pre-steering drive.
        let (steer_tx, steer_rx) = std::sync::mpsc::channel::<crate::SteerSignal>();
        let steer_window = std::time::Duration::from_secs(req.steer_window_secs);
        let in_flight = if steer_window.is_zero() {
            self.register_in_flight(session_id.as_str())
        } else {
            self.register_in_flight_steerable(session_id.as_str(), steer_tx)
        };
        let _guard = DeregGuard {
            server: self,
            session_id: session_id.to_string(),
        };

        // LIVE STEERING: attach a PERSISTENT session and drive the INITIAL prompt
        // through the controller's ingest seam under `turn-acp-live-{turn_id}` —
        // IDENTICAL to `drive_acp_live_turn`, so the zero-window path is
        // byte-identical. Then, while the steer window is open, loop: a `Steer`
        // signal cancels any in-flight prompt and re-prompts the SAME session (the
        // ACP multi-turn continuation); `Stop` / window-timeout / channel-close
        // ends the session.
        let mut psession = self
            .controller
            .attach_persistent_acp_session(
                &refs,
                &adapter,
                transport,
                &turn_id,
                Some(in_flight.cancel.clone()),
            )
            .map_err(ServerError::State)?;

        in_flight
            .cancel
            .store(false, std::sync::atomic::Ordering::Relaxed);
        let ingest_turn_0 = format!("turn-acp-live-{}", req.turn_id);
        let mut last_transcript = psession
            .prompt(&req.goal)
            .map_err(|error| ServerError::AdapterFixture(format!("acp live prompt: {error}")))?;
        let mut last_ingest = self
            .controller
            .ingest_acp_prompt(&refs, &last_transcript, &ingest_turn_0)
            .map_err(ServerError::State)?;

        if !steer_window.is_zero() {
            // Loop while STEER signals arrive within the window; exit on Stop, an
            // idle timeout, or all senders dropped (any non-`Steer` recv result
            // fails the `while let` pattern and ends the session).
            let mut steer_seq = 0u64;
            while let Ok(crate::SteerSignal::Steer(message)) = steer_rx.recv_timeout(steer_window) {
                steer_seq += 1;
                // The cancel flag may have aborted the prior prompt; reset it so
                // the continuation prompt runs on the same session.
                in_flight
                    .cancel
                    .store(false, std::sync::atomic::Ordering::Relaxed);
                last_transcript = psession.prompt(&message).map_err(|error| {
                    ServerError::AdapterFixture(format!("acp steer prompt: {error}"))
                })?;
                let ingest_turn = format!("turn-acp-live-{}-steer-{steer_seq}", req.turn_id);
                last_ingest = self
                    .controller
                    .ingest_acp_prompt(&refs, &last_transcript, &ingest_turn)
                    .map_err(ServerError::State)?;
            }
        }

        // Drop the persistent session (closes the agent's stdin) BEFORE finalize
        // tears down the process group and scans stderr for credential markers.
        drop(psession);
        session
            .finalize("server acp live turn complete")
            .map_err(|error| ServerError::AdapterFixture(format!("acp live finalize: {error}")))?;

        let summary = AcpLiveTurnSummary {
            session_id: session_id.to_string(),
            run_id: run_id.to_string(),
            turn_id: req.turn_id.clone(),
            workspace_root: workspace_root.to_string_lossy().to_string(),
            // The summary reflects the LAST prompt (the only one for a one-shot
            // worker; the final continuation for a steered worker).
            event_count: last_transcript.events.len(),
            appended_event_count: last_ingest.appended_event_count,
            stop_reason: last_transcript.stop_reason.clone(),
            reply_text: agent_reply_text(&last_transcript.events),
        };

        let command_hash = command_identity_hash(format!(
            "run_acp_live_turn_local:{}:{}:{}:{}",
            session_id,
            run_id,
            req.turn_id,
            stable_hash(req.goal.as_bytes())
        ));
        let command = self.command_envelope(
            &request_id,
            &origin,
            &command_hash,
            CommandTarget::Session(session_id.clone()),
            CommandIntent::SendTask,
            Some(req.goal),
        );
        self.record_server_request_handled(
            &command,
            &origin,
            "run_acp_live_turn_local",
            Some(&refs),
            Some(serde_json::json!({
                "turn_id": summary.turn_id,
                "workspace_root": summary.workspace_root,
                "event_count": summary.event_count,
                "appended_event_count": summary.appended_event_count,
                "stop_reason": summary.stop_reason,
            })),
        )
        .map_err(ServerError::State)?;

        self.response(
            request_id,
            origin,
            ServerResponsePayload::AcpLiveTurn(summary),
        )
    }

    /// SLICE-A (DESIGN-B Layer 2): drive ONE CONDUCTOR turn -- a near-clone of
    /// [`Self::run_acp_live_turn_local`] with TWO deltas, both at the setup-plan
    /// stage: (1) capo's in-process HTTP MCP endpoint (`mcp_url`/`mcp_headers`)
    /// is forwarded into `session/new` via `with_http_mcp_server`, so the
    /// conductor can dial the "capo tools" directly; (2) the prompt is composed
    /// as `"{conductor_goal}\n\n[user]: {user_message}"`. Everything else --
    /// the gate, `run_refs_for_session_run`, the adapter build, `spawn_live_session`,
    /// `drive_acp_live_turn` (so every tool call still round-trips through the
    /// `ControllerAcpDecider` permission seam), and `finalize` -- is reused
    /// unchanged.
    pub(crate) fn run_conductor_turn_local(
        &self,
        request_id: String,
        origin: ServerClientOrigin,
        req: ConductorTurnLocalRequest,
    ) -> ServerResult<ServerResponse> {
        use capo_adapters::{AcpAdapter, AcpLiveAdapter, TurnRequest, acp_live_gate_open};
        use capo_core::TurnId;

        // GATE: same fail-closed contract as run_acp_live_turn_local.
        if !req.live_acp_opt_in || !acp_live_gate_open() {
            return Err(ServerError::AdapterFixture(
                "conductor turn is fail-closed: set live_acp_opt_in AND \
                 CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT=1 CAPO_SERVER_RUN_ACP_LIVE=1"
                    .to_string(),
            ));
        }

        let session_id = SessionId::new(req.session_id);
        let run_id = RunId::new(req.run_id);
        let (_session, _run, _agent, refs) =
            self.run_refs_for_session_run(&session_id, &run_id)?;

        // The conductor runs in the project dir by default (it delegates real
        // work to workers, which carry their own worktrees).
        let workspace_root = self.codex_chat.acp_workspace_root.clone();
        let artifact_root = self.codex_chat.acp_artifact_root.clone();

        let wrappers = capo_tools::RuntimeToolWrappers::new(
            capo_tools::RuntimeToolConfig::local_workspace(
                workspace_root.clone(),
                artifact_root.clone(),
            ),
        );
        let policy = if req.acp_session_mode.is_some() {
            capo_tools::PermissionPolicy::allow_trusted_local()
        } else {
            capo_tools::PermissionPolicy::static_read_only_local()
        };
        let mut setup_plan = AcpAdapter::session_setup_plan(
            &wrappers.list_tools(),
            &policy,
            SessionId::new(format!("conductor-setup-{session_id}")),
        );
        // DELTA 1: forward capo's in-process HTTP MCP endpoint into session/new.
        setup_plan = setup_plan.with_http_mcp_server(req.mcp_url, req.mcp_headers);
        // Slice-0 (fork-free Path-1): opt-in conductor lockdown. When set, render
        // the proven `claude-code-acp` recipe into `session/new`'s
        // `_meta.claudeCode.options` so the conductor is confined to capo-only MCP
        // tools (capo re-supplies file/shell/search as capo_read/write/bash/search).
        // Default false ⇒ NO `_meta` is added, so the existing flow is byte-identical.
        if req.conductor_lockdown {
            setup_plan =
                setup_plan.with_session_lockdown(capo_adapters::AcpSessionLockdown::conductor_default());
        }
        if let Some(mode) = req.acp_session_mode.clone() {
            setup_plan = setup_plan
                .with_session_mode(mode)
                .with_workspace_root(workspace_root.clone());
        }
        let adapter = AcpLiveAdapter::new(
            req.acp_program,
            req.acp_argv,
            workspace_root.clone(),
            artifact_root,
            setup_plan,
        );

        let turn_id = TurnId::new(req.turn_id.clone());
        let mut session = adapter
            .spawn_live_session(&turn_id)
            .map_err(|error| ServerError::AdapterFixture(format!("conductor spawn: {error}")))?;
        let transport = session
            .take_transport()
            .ok_or_else(|| ServerError::AdapterFixture("conductor transport already taken".into()))?;

        // DELTA 2: compose the conductor prompt.
        let goal = format!("{}\n\n[user]: {}", req.conductor_goal, req.user_message);
        // COOPERATIVE CANCEL (B2): register + RAII deregister + thread the flag,
        // identical to run_acp_live_turn_local. Conductor turns are also
        // cancellable via InterruptAgent/StopAgent on the conductor's session.
        let in_flight = self.register_in_flight(session_id.as_str());
        let _guard = DeregGuard {
            server: self,
            session_id: session_id.to_string(),
        };
        let outcome = self
            .controller
            .drive_acp_live_turn(
                &refs,
                &adapter,
                transport,
                &TurnRequest {
                    turn_id: turn_id.clone(),
                    agent_name: "conductor".to_string(),
                    goal: goal.clone(),
                },
                Some(in_flight.cancel.clone()),
            )
            .map_err(ServerError::State)?;

        session
            .finalize("server conductor turn complete")
            .map_err(|error| ServerError::AdapterFixture(format!("conductor finalize: {error}")))?;

        let summary = AcpLiveTurnSummary {
            session_id: session_id.to_string(),
            run_id: run_id.to_string(),
            turn_id: req.turn_id.clone(),
            workspace_root: workspace_root.to_string_lossy().to_string(),
            event_count: outcome.transcript.events.len(),
            appended_event_count: outcome.ingest.appended_event_count,
            stop_reason: outcome.transcript.stop_reason.clone(),
            reply_text: agent_reply_text(&outcome.transcript.events),
        };

        let command_hash = command_identity_hash(format!(
            "run_conductor_turn_local:{}:{}:{}:{}",
            session_id,
            run_id,
            req.turn_id,
            stable_hash(goal.as_bytes())
        ));
        let command = self.command_envelope(
            &request_id,
            &origin,
            &command_hash,
            CommandTarget::Session(session_id.clone()),
            CommandIntent::SendTask,
            Some(goal),
        );
        self.record_server_request_handled(
            &command,
            &origin,
            "run_conductor_turn_local",
            Some(&refs),
            Some(serde_json::json!({
                "turn_id": summary.turn_id,
                "workspace_root": summary.workspace_root,
                "event_count": summary.event_count,
                "appended_event_count": summary.appended_event_count,
                "stop_reason": summary.stop_reason,
            })),
        )
        .map_err(ServerError::State)?;

        self.response(
            request_id,
            origin,
            ServerResponsePayload::AcpLiveTurn(summary),
        )
    }
}

/// SLICE-A: extract the agent's verbatim assistant prose from a live turn's
/// transcript events. capo content-hashes raw provider output in the persisted
/// event log (so the thread readback only carries a redacted LABEL), but the
/// live transcript's `NormalizedAdapterEvent.content` still holds the literal
/// streamed text. The assistant's prose rides on the agent-message kinds
/// (`adapter.item_delta` / `adapter.item_completed`) with `role == "assistant"`;
/// tool results carry the same kinds with other roles, so we filter on role to
/// avoid pulling tool output into the reply. Returns `None` when no prose exists.
fn agent_reply_text(
    events: &[capo_adapters::NormalizedAdapterEvent],
) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    for event in events {
        let is_agent_message =
            event.kind == "adapter.item_delta" || event.kind == "adapter.item_completed";
        let is_assistant = event.role.as_deref() == Some("assistant");
        if is_agent_message
            && is_assistant
            && let Some(text) = event.content.as_deref()
            && !text.is_empty()
        {
            parts.push(text);
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.concat())
    }
}

/// SLICE-A (DESIGN-B Layer 2): the flat inputs for
/// [`CapoServer::run_conductor_turn_local`].
pub(crate) struct ConductorTurnLocalRequest {
    pub session_id: String,
    pub run_id: String,
    pub turn_id: String,
    pub user_message: String,
    pub conductor_goal: String,
    pub mcp_url: String,
    pub mcp_headers: Vec<(String, String)>,
    pub acp_program: String,
    pub acp_argv: Vec<String>,
    pub acp_session_mode: Option<String>,
    pub live_acp_opt_in: bool,
    /// Slice-0 (fork-free Path-1): opt-in conductor session lockdown. Default
    /// false ⇒ the existing conductor flow is byte-identical.
    pub conductor_lockdown: bool,
}

/// SLICE-A: the flat inputs for [`CapoServer::run_acp_live_turn_local`].
pub(crate) struct AcpLiveTurnLocalRequest {
    pub session_id: String,
    pub run_id: String,
    pub goal: String,
    pub turn_id: String,
    pub acp_program: String,
    pub acp_argv: Vec<String>,
    pub workspace_root: Option<String>,
    pub live_acp_opt_in: bool,
    pub acp_session_mode: Option<String>,
    /// Optional capo MCP endpoint forwarded into the worker's `session/new` so
    /// the worker can call `report_result`. `None` ⇒ no MCP server advertised to
    /// the worker (validated file-only loop unchanged). See the command field.
    pub mcp_url: Option<String>,
    pub mcp_headers: Vec<(String, String)>,
    /// LIVE STEERING: seconds to keep the session alive after the turn for
    /// steer/stop signals. `0` ⇒ one-shot (finalize immediately), byte-identical
    /// to the pre-steering path. See the command field.
    pub steer_window_secs: u64,
}
