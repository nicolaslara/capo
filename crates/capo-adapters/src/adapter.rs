use capo_core::{BoundaryBinding, BoundaryKind, SessionId, TurnId};

use crate::{NormalizedAdapterEvent, ScriptedMockAgent};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AgentAdapter {
    Fake(FakeAdapter),
    ScriptedMock(ScriptedMockAgent),
}

impl AgentAdapter {
    pub fn fake() -> Self {
        Self::Fake(FakeAdapter)
    }

    pub fn scripted_mock(script: ScriptedMockAgent) -> Self {
        Self::ScriptedMock(script)
    }

    pub fn binding(&self) -> BoundaryBinding {
        match self {
            Self::Fake(adapter) => adapter.binding(),
            Self::ScriptedMock(agent) => agent.binding(),
        }
    }

    pub fn open_session(&self, request: FakeAdapterSessionRequest) -> FakeAdapterSession {
        match self {
            Self::Fake(adapter) => adapter.open_session(request),
            Self::ScriptedMock(agent) => agent.open_session(request),
        }
    }

    pub fn send_turn(
        &self,
        session: &FakeAdapterSession,
        request: FakeAdapterTurnRequest,
    ) -> FakeAdapterTurnOutput {
        match self {
            Self::Fake(adapter) => adapter.send_turn(session, request),
            Self::ScriptedMock(agent) => agent.send_turn(session, request),
        }
    }

    pub fn attach_session(
        &self,
        session_id: SessionId,
        external_session_ref: String,
    ) -> FakeAdapterSession {
        match self {
            Self::Fake(adapter) => adapter.attach_session(session_id, external_session_ref),
            Self::ScriptedMock(agent) => agent.attach_session(session_id, external_session_ref),
        }
    }

    pub fn interrupt(&self, session: &FakeAdapterSession, reason: &str) -> FakeAdapterTurnOutput {
        match self {
            Self::Fake(adapter) => adapter.interrupt(session, reason),
            Self::ScriptedMock(agent) => agent.interrupt(session, reason),
        }
    }

    pub fn stop(&self, session: &FakeAdapterSession, reason: &str) -> FakeAdapterTurnOutput {
        match self {
            Self::Fake(adapter) => adapter.stop(session, reason),
            Self::ScriptedMock(agent) => agent.stop(session, reason),
        }
    }

    pub fn scripted_turn_events(&self, turn_ref: &str) -> Option<Vec<NormalizedAdapterEvent>> {
        match self {
            Self::Fake(_) => None,
            Self::ScriptedMock(agent) => agent.turn_events(turn_ref),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeAdapter;

impl FakeAdapter {
    pub fn binding(&self) -> BoundaryBinding {
        BoundaryBinding::fake(BoundaryKind::AgentAdapter, "fake-adapter")
    }

    pub fn open_session(&self, request: FakeAdapterSessionRequest) -> FakeAdapterSession {
        FakeAdapterSession {
            session_id: request.session_id,
            external_session_ref: format!("fake-adapter-session-{}", request.agent_name),
            adapter_capability: "fake-streaming-and-tools".to_string(),
        }
    }

    pub fn send_turn(
        &self,
        session: &FakeAdapterSession,
        request: FakeAdapterTurnRequest,
    ) -> FakeAdapterTurnOutput {
        FakeAdapterTurnOutput {
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

    pub fn attach_session(
        &self,
        session_id: SessionId,
        external_session_ref: String,
    ) -> FakeAdapterSession {
        FakeAdapterSession {
            session_id,
            external_session_ref,
            adapter_capability: "fake-streaming-and-tools".to_string(),
        }
    }

    pub fn interrupt(&self, session: &FakeAdapterSession, reason: &str) -> FakeAdapterTurnOutput {
        FakeAdapterTurnOutput {
            turn_id: TurnId::new(format!("interrupt-{}", session.session_id)),
            external_session_ref: session.external_session_ref.clone(),
            summary: format!("Fake adapter interrupted session: {reason}"),
            confidence: 70,
            status: "canceled".to_string(),
            tool_name: "capo.session_summary".to_string(),
        }
    }

    pub fn stop(&self, session: &FakeAdapterSession, reason: &str) -> FakeAdapterTurnOutput {
        FakeAdapterTurnOutput {
            turn_id: TurnId::new(format!("stop-{}", session.session_id)),
            external_session_ref: session.external_session_ref.clone(),
            summary: format!("Fake adapter stopped session: {reason}"),
            confidence: 70,
            status: "completed".to_string(),
            tool_name: "capo.session_summary".to_string(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeAdapterSessionRequest {
    pub session_id: SessionId,
    pub agent_name: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeAdapterSession {
    pub session_id: SessionId,
    pub external_session_ref: String,
    pub adapter_capability: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeAdapterTurnRequest {
    pub turn_id: TurnId,
    pub agent_name: String,
    pub goal: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeAdapterTurnOutput {
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
