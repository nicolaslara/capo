use capo_controller::FakeRunRefs;
use capo_core::{CommandEnvelope, CommandId, CommandIntent, CommandTarget, RunId, SessionId};
use capo_state::{AgentProjection, EventKind, NewEvent, RunProjection, SessionProjection};

use crate::util::{adapter_kind_for_events, command_identity_hash, slug, stable_hash};
use crate::{
    CapoServer, RecoverySummary, ServerClientOrigin, ServerError, ServerResponse,
    ServerResponsePayload, ServerResult,
};

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
            .controller
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
}
