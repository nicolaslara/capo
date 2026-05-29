use capo_core::{BoundaryBinding, BoundaryKind, SessionId, TurnId};

use crate::{NormalizedAdapterEvent, ScriptedMockAgent};

/// Provider-neutral seam every agent provider implements.
///
/// This is the single adapter contract the controller turn loop drives. Codex
/// is the first real implementation; Claude and ACP follow in the `depth`
/// workpad. The trait deliberately speaks provider-neutral vocabulary
/// (`AdapterSessionRequest`, `TurnRequest`, `TurnOutput`, `AdapterSession`) and
/// must not signal "fake-first": `FakeAdapter` and `ScriptedMockAgent` are just
/// the first deterministic implementations behind it.
///
/// Normalized output is expressed as [`NormalizedAdapterEvent`] (via
/// [`AgentAdapter::scripted_turn_events`]) so the trait feeds the existing
/// `apply_normalized_adapter_events_with_turn` controller path rather than a new
/// ingestion route.
pub trait AgentAdapter {
    /// Boundary binding identifying this adapter implementation.
    fn binding(&self) -> BoundaryBinding;

    /// Open a new adapter session for the requested agent.
    fn open_session(&self, request: AdapterSessionRequest) -> AdapterSession;

    /// Send a turn to an open session and observe its output.
    fn send_turn(&self, session: &AdapterSession, request: TurnRequest) -> TurnOutput;

    /// Reattach to an existing external session reference.
    fn attach_session(&self, session_id: SessionId, external_session_ref: String)
    -> AdapterSession;

    /// Interrupt the current turn of a session.
    fn interrupt(&self, session: &AdapterSession, reason: &str) -> TurnOutput;

    /// Stop a session.
    fn stop(&self, session: &AdapterSession, reason: &str) -> TurnOutput;

    /// Normalized adapter events for a scripted turn, when the adapter can
    /// replay deterministic fixtures. Defaults to `None` for adapters that do
    /// not script turns.
    fn scripted_turn_events(&self, _turn_ref: &str) -> Option<Vec<NormalizedAdapterEvent>> {
        None
    }
}

/// Thin dispatch handle over the concrete [`AgentAdapter`] implementations.
///
/// RTL1 keeps this as a dispatch enum over trait impls; RTL2 migrates callers
/// onto the trait. New providers (Codex now; Claude/ACP later) are added as
/// trait implementations, not new vocabulary at the seam.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AgentAdapterHandle {
    Fake(FakeAdapter),
    ScriptedMock(ScriptedMockAgent),
}

impl AgentAdapterHandle {
    pub fn fake() -> Self {
        Self::Fake(FakeAdapter)
    }

    pub fn scripted_mock(script: ScriptedMockAgent) -> Self {
        Self::ScriptedMock(script)
    }

    fn as_adapter(&self) -> &dyn AgentAdapter {
        match self {
            Self::Fake(adapter) => adapter,
            Self::ScriptedMock(agent) => agent,
        }
    }
}

impl AgentAdapter for AgentAdapterHandle {
    fn binding(&self) -> BoundaryBinding {
        self.as_adapter().binding()
    }

    fn open_session(&self, request: AdapterSessionRequest) -> AdapterSession {
        self.as_adapter().open_session(request)
    }

    fn send_turn(&self, session: &AdapterSession, request: TurnRequest) -> TurnOutput {
        self.as_adapter().send_turn(session, request)
    }

    fn attach_session(
        &self,
        session_id: SessionId,
        external_session_ref: String,
    ) -> AdapterSession {
        self.as_adapter()
            .attach_session(session_id, external_session_ref)
    }

    fn interrupt(&self, session: &AdapterSession, reason: &str) -> TurnOutput {
        self.as_adapter().interrupt(session, reason)
    }

    fn stop(&self, session: &AdapterSession, reason: &str) -> TurnOutput {
        self.as_adapter().stop(session, reason)
    }

    fn scripted_turn_events(&self, turn_ref: &str) -> Option<Vec<NormalizedAdapterEvent>> {
        self.as_adapter().scripted_turn_events(turn_ref)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeAdapter;

impl AgentAdapter for FakeAdapter {
    fn binding(&self) -> BoundaryBinding {
        BoundaryBinding::fake(BoundaryKind::AgentAdapter, "fake-adapter")
    }

    fn open_session(&self, request: AdapterSessionRequest) -> AdapterSession {
        AdapterSession {
            session_id: request.session_id,
            external_session_ref: format!("fake-adapter-session-{}", request.agent_name),
            adapter_capability: "fake-streaming-and-tools".to_string(),
        }
    }

    fn send_turn(&self, session: &AdapterSession, request: TurnRequest) -> TurnOutput {
        TurnOutput {
            turn_id: request.turn_id,
            external_session_ref: session.external_session_ref.clone(),
            summary: format!(
                "Fake adapter processed goal for {}: {}",
                request.agent_name, request.goal
            ),
            confidence: 82,
            status: "active".to_string(),
            tool_name: "capo.session_summary".to_string(),
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
            adapter_capability: "fake-streaming-and-tools".to_string(),
        }
    }

    fn interrupt(&self, session: &AdapterSession, reason: &str) -> TurnOutput {
        TurnOutput {
            turn_id: TurnId::new(format!("interrupt-{}", session.session_id)),
            external_session_ref: session.external_session_ref.clone(),
            summary: format!("Fake adapter interrupted session: {reason}"),
            confidence: 70,
            status: "canceled".to_string(),
            tool_name: "capo.session_summary".to_string(),
        }
    }

    fn stop(&self, session: &AdapterSession, reason: &str) -> TurnOutput {
        TurnOutput {
            turn_id: TurnId::new(format!("stop-{}", session.session_id)),
            external_session_ref: session.external_session_ref.clone(),
            summary: format!("Fake adapter stopped session: {reason}"),
            confidence: 70,
            status: "completed".to_string(),
            tool_name: "capo.session_summary".to_string(),
        }
    }
}

/// Provider-neutral request to open an adapter session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdapterSessionRequest {
    pub session_id: SessionId,
    pub agent_name: String,
}

/// Provider-neutral handle to an open adapter session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdapterSession {
    pub session_id: SessionId,
    pub external_session_ref: String,
    pub adapter_capability: String,
}

/// Provider-neutral request to send one turn to a session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TurnRequest {
    pub turn_id: TurnId,
    pub agent_name: String,
    pub goal: String,
}

/// Provider-neutral observed output of one turn.
///
/// The shape carries `turn_id`, `external_session_ref`, `summary`,
/// `confidence`, `status`, and the observed `tool_name` so the existing
/// controller projection wiring keeps working unchanged.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TurnOutput {
    pub turn_id: TurnId,
    pub external_session_ref: String,
    pub summary: String,
    pub confidence: i64,
    pub status: String,
    pub tool_name: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProviderConnector {
    Fake(FakeProviderConnector),
}

impl ProviderConnector {
    pub fn fake() -> Self {
        Self::Fake(FakeProviderConnector)
    }

    pub fn binding(&self) -> BoundaryBinding {
        match self {
            Self::Fake(connector) => connector.binding(),
        }
    }

    pub fn describe_provider(&self) -> FakeProviderInfo {
        match self {
            Self::Fake(connector) => connector.describe_provider(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeProviderConnector;

impl FakeProviderConnector {
    pub fn binding(&self) -> BoundaryBinding {
        BoundaryBinding::fake(BoundaryKind::ProviderConnector, "fake-provider")
    }

    pub fn describe_provider(&self) -> FakeProviderInfo {
        FakeProviderInfo {
            provider_kind: "fake".to_string(),
            auth_mode: "none".to_string(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeProviderInfo {
    pub provider_kind: String,
    pub auth_mode: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_session(agent_name: &str) -> AdapterSession {
        FakeAdapter.open_session(AdapterSessionRequest {
            session_id: SessionId::new(format!("session-{agent_name}")),
            agent_name: agent_name.to_string(),
        })
    }

    #[test]
    fn fake_adapter_implements_provider_neutral_trait() {
        let adapter = FakeAdapter;
        let session = fake_session("planner");

        assert_eq!(session.external_session_ref, "fake-adapter-session-planner");

        let output = adapter.send_turn(
            &session,
            TurnRequest {
                turn_id: TurnId::new("turn-1"),
                agent_name: "planner".to_string(),
                goal: "draft a plan".to_string(),
            },
        );

        assert_eq!(output.turn_id.as_str(), "turn-1");
        assert_eq!(output.confidence, 82);
        assert_eq!(output.status, "active");
        assert_eq!(output.tool_name, "capo.session_summary");
        assert_eq!(
            output.summary,
            "Fake adapter processed goal for planner: draft a plan"
        );
        assert!(adapter.scripted_turn_events("turn-1").is_none());
    }

    #[test]
    fn handle_dispatches_through_the_trait() {
        let handle = AgentAdapterHandle::fake();
        assert_eq!(handle.binding().kind, BoundaryKind::AgentAdapter);
        assert_eq!(handle.binding().variant, "fake-adapter");

        let session = handle.open_session(AdapterSessionRequest {
            session_id: SessionId::new("session-worker"),
            agent_name: "worker".to_string(),
        });
        let interrupted = handle.interrupt(&session, "operator paused");
        assert_eq!(interrupted.status, "canceled");
        let stopped = handle.stop(&session, "done");
        assert_eq!(stopped.status, "completed");
    }

    #[test]
    fn scripted_mock_routes_through_handle_and_trait() {
        use crate::{ScriptedMockAgent, ScriptedMockTurn};

        let handle = AgentAdapterHandle::scripted_mock(
            ScriptedMockAgent::new("mock-session").with_turn(
                ScriptedMockTurn::new("turn-mock")
                    .message_completed("msg-1", "scripted turn completed"),
            ),
        );

        let session = handle.open_session(AdapterSessionRequest {
            session_id: SessionId::new("session-mock"),
            agent_name: "mock-worker".to_string(),
        });
        let output = handle.send_turn(
            &session,
            TurnRequest {
                turn_id: TurnId::new("turn-mock"),
                agent_name: "mock-worker".to_string(),
                goal: "run scripted turn".to_string(),
            },
        );

        assert_eq!(handle.binding().variant, "scripted-mock-agent");
        assert_eq!(output.external_session_ref, "mock-session");
        assert_eq!(output.summary, "scripted turn completed");
        assert_eq!(
            handle
                .scripted_turn_events("turn-mock")
                .expect("scripted events")
                .len(),
            1
        );
    }
}
