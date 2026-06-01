use capo_core::{BoundaryBinding, BoundaryKind, SessionId, TurnId};

use crate::acp_live::AcpLiveAdapter;
use crate::claude_live::ClaudeCodeLiveAdapter;
use crate::codex_live::{CodexLiveAdapter, CodexLiveChatError};
use crate::{
    AdapterPermissionRequest, AdapterPermissionResponse, NormalizedAdapterEvent, ScriptedMockAgent,
};

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

    /// Fallible send seam (AI2).
    ///
    /// The deterministic fake/scripted adapters can never fail to produce a turn,
    /// so this defaults to wrapping [`AgentAdapter::send_turn`] in `Ok`. The real
    /// Codex chat adapter ([`CodexLiveAdapter`]) overrides it so a Codex-bound
    /// chat turn can FAIL CLOSED FAST (an immediate typed
    /// [`CodexLiveChatError::GateClosed`]) when the live-provider opt-in gate is
    /// off -- no process spawn, no blocking -- rather than fabricating a fake
    /// summary. The controller chat path drives THIS method and propagates the
    /// typed error.
    fn try_send_turn(
        &self,
        session: &AdapterSession,
        request: &TurnRequest,
    ) -> Result<TurnOutput, CodexLiveChatError> {
        Ok(self.send_turn(session, request.clone()))
    }

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

    /// SG2: the adapter permission round-trip RAISE side.
    ///
    /// An interactive provider (ACP, and the fake/scripted adapters that stand in
    /// for it) raises a permission request -- the requested scope plus the ACP
    /// `PermissionOption[]` it is offering -- which the controller decides. The
    /// closing leg (returning the chosen `optionId`/`cancelled` to the adapter) is
    /// [`AgentAdapter::deliver_permission_response`]. Adapters that never prompt
    /// for permission default to `None`. This is fixture/option-mapping only: it
    /// does NOT speak the live ACP JSON-RPC wire (that is the depth workpad).
    fn scripted_permission_request(&self, _request_ref: &str) -> Option<AdapterPermissionRequest> {
        None
    }

    /// SG2: the adapter permission round-trip DELIVER side (the closing leg).
    ///
    /// After the controller decides a raised request, the loop delivers the
    /// [`AdapterPermissionResponse`] back THROUGH this seam, and the adapter
    /// reports whether it would proceed with the tool call. The default derives
    /// the ack purely from the response (proceed iff the policy allowed AND the
    /// `must_not_proceed` halt signal is clear) -- so a fake/scripted adapter
    /// honors a Capo deny/cancel/adapter-error by NOT proceeding, even when the
    /// raw ACP `outcome` carries a selected option id. The depth ACP adapter
    /// overrides this to write the real ACP wire frame; the proceed/halt contract
    /// is identical.
    fn deliver_permission_response(
        &self,
        response: &AdapterPermissionResponse,
    ) -> PermissionDeliveryAck {
        PermissionDeliveryAck {
            proceeded: response.may_proceed(),
            adapter_error: response.adapter_error,
        }
    }
}

/// SG2: the adapter's acknowledgement of a delivered [`AdapterPermissionResponse`]
/// -- the closing leg of the permission round-trip.
///
/// `proceeded` is the single safety-relevant bit: `true` only when the adapter
/// would go ahead with the requested tool call. A Capo deny (including a policy
/// deny over-ruling an offered allow option), a cancel, or the adapter-error path
/// all yield `proceeded = false`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PermissionDeliveryAck {
    /// `true` iff the adapter would proceed with the requested tool call.
    pub proceeded: bool,
    /// `true` when the request failed as an adapter error (no selectable option).
    pub adapter_error: bool,
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
    /// AI2: a real Codex chat handle. Installed ONLY for agents explicitly bound
    /// to the Codex adapter; it never replaces the fake adapter as a global
    /// default for unbound/mock agents.
    Codex(CodexLiveAdapter),
    /// DP4: a real live Claude Code adapter handle (the SECOND real provider),
    /// bound ONLY for agents explicitly routed to the Claude workspace-write
    /// adapter. Like the Codex handle it is a real provider binding
    /// (`is_real()`), never a default for unbound/mock agents; the live write
    /// spawn stays fail-closed behind the Claude live opt-in gate
    /// (`CAPO_SERVER_RUN_CLAUDE_LIVE` + the shared live-provider preflight).
    Claude(ClaudeCodeLiveAdapter),
    /// DP1: a real live ACP JSON-RPC adapter handle, bound ONLY for agents
    /// explicitly routed to the ACP wire client. Like the Codex handle it is a
    /// real provider binding (`is_real()`), never a default for unbound/mock
    /// agents; the live spawn stays fail-closed behind the ACP live opt-in gate.
    ///
    /// Boxed because `AcpLiveAdapter` is by far the largest adapter variant
    /// (wire-client + transport state); inlining it would bloat every
    /// `AgentAdapterHandle` (`clippy::large_enum_variant`).
    Acp(Box<AcpLiveAdapter>),
}

impl AgentAdapterHandle {
    pub fn fake() -> Self {
        Self::Fake(FakeAdapter)
    }

    pub fn scripted_mock(script: ScriptedMockAgent) -> Self {
        Self::ScriptedMock(script)
    }

    /// Bind a real Codex chat adapter handle (AI2). Fail-closed-fast on chat when
    /// the live-provider opt-in gate is off.
    pub fn codex(adapter: CodexLiveAdapter) -> Self {
        Self::Codex(adapter)
    }

    /// Bind a real live Claude adapter handle (DP4). The live write spawn is
    /// fail-closed behind the Claude live opt-in gate; with the gate off
    /// `try_send_turn` returns an immediate typed error rather than spawning.
    pub fn claude(adapter: ClaudeCodeLiveAdapter) -> Self {
        Self::Claude(adapter)
    }

    /// Bind a real live ACP adapter handle (DP1). The live spawn is fail-closed
    /// behind the ACP live opt-in gate; with the gate off `send_turn` reports a
    /// blocked turn rather than spawning a process.
    pub fn acp(adapter: AcpLiveAdapter) -> Self {
        Self::Acp(Box::new(adapter))
    }

    /// Whether this handle drives a real (non-fake) provider. Fake/scripted are
    /// deterministic test handles; the Codex and ACP handles are real provider
    /// bindings.
    pub fn is_real(&self) -> bool {
        matches!(self, Self::Codex(_) | Self::Claude(_) | Self::Acp(_))
    }

    fn as_adapter(&self) -> &dyn AgentAdapter {
        match self {
            Self::Fake(adapter) => adapter,
            Self::ScriptedMock(agent) => agent,
            Self::Codex(adapter) => adapter,
            Self::Claude(adapter) => adapter,
            Self::Acp(adapter) => adapter.as_ref(),
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

    fn try_send_turn(
        &self,
        session: &AdapterSession,
        request: &TurnRequest,
    ) -> Result<TurnOutput, CodexLiveChatError> {
        self.as_adapter().try_send_turn(session, request)
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

    fn scripted_permission_request(&self, request_ref: &str) -> Option<AdapterPermissionRequest> {
        self.as_adapter().scripted_permission_request(request_ref)
    }

    fn deliver_permission_response(
        &self,
        response: &AdapterPermissionResponse,
    ) -> PermissionDeliveryAck {
        self.as_adapter().deliver_permission_response(response)
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
    fn deliver_permission_response_default_honors_halt_signal() {
        let adapter = FakeAdapter;
        // An allow with no halt signal proceeds.
        let allow = AdapterPermissionResponse {
            outcome: crate::AcpPermissionOutcome::Selected {
                option_id: "opt-allow_once".to_string(),
            },
            capo_decision: "allow".to_string(),
            capo_persistence: Some("until_turn_end".to_string()),
            permission_decision_id: "decision-1".to_string(),
            capability_grant_id: Some("grant-1".to_string()),
            adapter_error: false,
            must_not_proceed: false,
        };
        let ack = adapter.deliver_permission_response(&allow);
        assert!(ack.proceeded);
        assert!(!ack.adapter_error);

        // A policy deny that over-rules an allow option still carries a `selected`
        // outcome string in some shapes, but `must_not_proceed` halts the adapter.
        let deny = AdapterPermissionResponse {
            outcome: crate::AcpPermissionOutcome::Cancelled,
            capo_decision: "deny".to_string(),
            capo_persistence: None,
            permission_decision_id: "decision-2".to_string(),
            capability_grant_id: None,
            adapter_error: false,
            must_not_proceed: true,
        };
        let ack = adapter.deliver_permission_response(&deny);
        assert!(!ack.proceeded, "a halt signal must stop the adapter");
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
