use capo_core::{BoundaryBinding, BoundaryKind, SessionId, TurnId};
use serde_json::json;

use crate::{
    AdapterTimelineConfidence, FakeAdapterSession, FakeAdapterSessionRequest,
    FakeAdapterTurnOutput, FakeAdapterTurnRequest, NormalizedAdapterEvent, NormalizedAdapterKind,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScriptedMockAgent {
    external_session_ref: String,
    turns: Vec<ScriptedMockTurn>,
}

impl ScriptedMockAgent {
    pub fn new(external_session_ref: impl Into<String>) -> Self {
        Self {
            external_session_ref: external_session_ref.into(),
            turns: Vec::new(),
        }
    }

    pub fn with_turn(mut self, turn: ScriptedMockTurn) -> Self {
        self.turns.push(turn);
        self
    }

    pub fn turns(&self) -> &[ScriptedMockTurn] {
        &self.turns
    }

    pub fn turn_events(&self, turn_ref: &str) -> Option<Vec<NormalizedAdapterEvent>> {
        self.turns
            .iter()
            .find(|turn| turn.turn_ref == turn_ref)
            .map(|turn| turn.normalized_events(&self.external_session_ref))
    }

    pub fn binding(&self) -> BoundaryBinding {
        BoundaryBinding::fake(BoundaryKind::AgentAdapter, "scripted-mock-agent")
    }

    pub fn open_session(&self, request: FakeAdapterSessionRequest) -> FakeAdapterSession {
        FakeAdapterSession {
            session_id: request.session_id,
            external_session_ref: self.external_session_ref.clone(),
            adapter_capability: "scripted-mock-events".to_string(),
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
            adapter_capability: "scripted-mock-events".to_string(),
        }
    }

    pub fn send_turn(
        &self,
        session: &FakeAdapterSession,
        request: FakeAdapterTurnRequest,
    ) -> FakeAdapterTurnOutput {
        let events = self
            .turn_events(request.turn_id.as_str())
            .or_else(|| {
                self.turns
                    .first()
                    .map(|turn| turn.normalized_events(&session.external_session_ref))
            })
            .unwrap_or_default();
        let summary = events
            .iter()
            .rev()
            .find_map(|event| event.content.clone())
            .unwrap_or_else(|| format!("Scripted mock accepted goal: {}", request.goal));
        let status = events
            .iter()
            .rev()
            .find_map(|event| event.status.clone())
            .unwrap_or_else(|| "active".to_string());
        let tool_name = events
            .iter()
            .find_map(|event| event.tool_name.clone())
            .unwrap_or_else(|| "capo.session_summary".to_string());
        FakeAdapterTurnOutput {
            turn_id: request.turn_id,
            external_session_ref: session.external_session_ref.clone(),
            summary,
            confidence: 88,
            status,
            tool_name,
        }
    }

    pub fn interrupt(&self, session: &FakeAdapterSession, reason: &str) -> FakeAdapterTurnOutput {
        FakeAdapterTurnOutput {
            turn_id: TurnId::new(format!("interrupt-{}", session.session_id)),
            external_session_ref: session.external_session_ref.clone(),
            summary: format!("Scripted mock interrupted session: {reason}"),
            confidence: 80,
            status: "canceled".to_string(),
            tool_name: "capo.session_summary".to_string(),
        }
    }

    pub fn stop(&self, session: &FakeAdapterSession, reason: &str) -> FakeAdapterTurnOutput {
        FakeAdapterTurnOutput {
            turn_id: TurnId::new(format!("stop-{}", session.session_id)),
            external_session_ref: session.external_session_ref.clone(),
            summary: format!("Scripted mock stopped session: {reason}"),
            confidence: 80,
            status: "completed".to_string(),
            tool_name: "capo.session_summary".to_string(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScriptedMockTurn {
    turn_ref: String,
    events: Vec<ScriptedMockEvent>,
}

impl ScriptedMockTurn {
    pub fn new(turn_ref: impl Into<String>) -> Self {
        Self {
            turn_ref: turn_ref.into(),
            events: Vec::new(),
        }
    }

    pub fn turn_ref(&self) -> &str {
        &self.turn_ref
    }

    pub fn message_delta(
        mut self,
        item_ref: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        self.events.push(ScriptedMockEvent::MessageDelta {
            item_ref: item_ref.into(),
            role: "assistant".to_string(),
            content: content.into(),
        });
        self
    }

    pub fn message_completed(
        mut self,
        item_ref: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        self.events.push(ScriptedMockEvent::MessageCompleted {
            item_ref: item_ref.into(),
            role: "assistant".to_string(),
            content: content.into(),
        });
        self
    }

    pub fn tool_requested(
        mut self,
        item_ref: impl Into<String>,
        tool_name: impl Into<String>,
    ) -> Self {
        self.events.push(ScriptedMockEvent::ToolRequested {
            item_ref: item_ref.into(),
            tool_name: tool_name.into(),
        });
        self
    }

    pub fn tool_completed(
        mut self,
        item_ref: impl Into<String>,
        tool_name: impl Into<String>,
        output_summary: impl Into<String>,
    ) -> Self {
        self.events.push(ScriptedMockEvent::ToolCompleted {
            item_ref: item_ref.into(),
            tool_name: tool_name.into(),
            output_summary: output_summary.into(),
        });
        self
    }

    pub fn permission_requested(
        mut self,
        item_ref: impl Into<String>,
        scope: impl Into<String>,
    ) -> Self {
        self.events.push(ScriptedMockEvent::PermissionRequested {
            item_ref: item_ref.into(),
            scope: scope.into(),
        });
        self
    }

    pub fn failed(mut self, item_ref: impl Into<String>, reason: impl Into<String>) -> Self {
        self.events.push(ScriptedMockEvent::Failed {
            item_ref: item_ref.into(),
            reason: reason.into(),
        });
        self
    }

    pub fn interrupted(mut self, item_ref: impl Into<String>, reason: impl Into<String>) -> Self {
        self.events.push(ScriptedMockEvent::Interrupted {
            item_ref: item_ref.into(),
            reason: reason.into(),
        });
        self
    }

    pub fn turn_completed(mut self, item_ref: impl Into<String>) -> Self {
        self.events.push(ScriptedMockEvent::TurnCompleted {
            item_ref: item_ref.into(),
        });
        self
    }

    pub fn normalized_events(&self, external_session_ref: &str) -> Vec<NormalizedAdapterEvent> {
        self.events
            .iter()
            .enumerate()
            .map(|(index, event)| {
                event.normalized_event(external_session_ref, &self.turn_ref, index)
            })
            .collect()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScriptedMockEvent {
    MessageDelta {
        item_ref: String,
        role: String,
        content: String,
    },
    MessageCompleted {
        item_ref: String,
        role: String,
        content: String,
    },
    ToolRequested {
        item_ref: String,
        tool_name: String,
    },
    ToolCompleted {
        item_ref: String,
        tool_name: String,
        output_summary: String,
    },
    PermissionRequested {
        item_ref: String,
        scope: String,
    },
    Failed {
        item_ref: String,
        reason: String,
    },
    Interrupted {
        item_ref: String,
        reason: String,
    },
    TurnCompleted {
        item_ref: String,
    },
}

impl ScriptedMockEvent {
    fn normalized_event(
        &self,
        external_session_ref: &str,
        turn_ref: &str,
        index: usize,
    ) -> NormalizedAdapterEvent {
        let (kind, provider_event_kind, item_ref, operation) = match self {
            Self::MessageDelta { item_ref, .. } => (
                "adapter.item_delta",
                "mock.message_delta",
                item_ref.as_str(),
                "delta",
            ),
            Self::MessageCompleted { item_ref, .. } => (
                "adapter.item_completed",
                "mock.message_completed",
                item_ref.as_str(),
                "completed",
            ),
            Self::ToolRequested { item_ref, .. } => (
                "adapter.tool_call_requested",
                "mock.tool_requested",
                item_ref.as_str(),
                "requested",
            ),
            Self::ToolCompleted { item_ref, .. } => (
                "adapter.tool_call_completed",
                "mock.tool_completed",
                item_ref.as_str(),
                "completed",
            ),
            Self::PermissionRequested { item_ref, .. } => (
                "adapter.permission_requested",
                "mock.permission_requested",
                item_ref.as_str(),
                "permission",
            ),
            Self::Failed { item_ref, .. } => (
                "adapter.turn_failed",
                "mock.failed",
                item_ref.as_str(),
                "failed",
            ),
            Self::Interrupted { item_ref, .. } => (
                "adapter.turn_interrupted",
                "mock.interrupted",
                item_ref.as_str(),
                "interrupted",
            ),
            Self::TurnCompleted { item_ref } => (
                "adapter.turn_completed",
                "mock.turn_completed",
                item_ref.as_str(),
                "completed",
            ),
        };
        let raw = json!({
            "source": "scripted_mock_agent",
            "external_session_ref": external_session_ref,
            "turn_ref": turn_ref,
            "item_ref": item_ref,
            "index": index,
            "event": event_name(self),
        });
        let mut normalized = NormalizedAdapterEvent::new(
            NormalizedAdapterKind::Mock,
            kind,
            provider_event_kind,
            &raw,
        )
        .with_timeline(
            Some(external_session_ref.to_string()),
            Some(item_ref.to_string()),
            format!("mock:{external_session_ref}:{turn_ref}:{item_ref}:{index}"),
            AdapterTimelineConfidence::Stable,
            operation,
        );
        match self {
            Self::MessageDelta { role, content, .. }
            | Self::MessageCompleted { role, content, .. } => {
                normalized.role = Some(role.clone());
                normalized.content = Some(content.clone());
                normalized.status = Some(operation.to_string());
            }
            Self::ToolRequested { tool_name, .. } => {
                normalized.tool_name = Some(tool_name.clone());
                normalized.status = Some("requested".to_string());
            }
            Self::ToolCompleted {
                tool_name,
                output_summary,
                ..
            } => {
                normalized.tool_name = Some(tool_name.clone());
                normalized.content = Some(output_summary.clone());
                normalized.status = Some("completed".to_string());
            }
            Self::PermissionRequested { scope, .. } => {
                normalized.tool_name = Some("capo.permission_request".to_string());
                normalized.content = Some(scope.clone());
                normalized.status = Some("waiting_for_permission".to_string());
            }
            Self::Failed { reason, .. } => {
                normalized.content = Some(reason.clone());
                normalized.status = Some("failed".to_string());
            }
            Self::Interrupted { reason, .. } => {
                normalized.content = Some(reason.clone());
                normalized.status = Some("interrupted".to_string());
            }
            Self::TurnCompleted { .. } => {
                normalized.status = Some("completed".to_string());
            }
        }
        normalized
    }
}

fn event_name(event: &ScriptedMockEvent) -> &'static str {
    match event {
        ScriptedMockEvent::MessageDelta { .. } => "message_delta",
        ScriptedMockEvent::MessageCompleted { .. } => "message_completed",
        ScriptedMockEvent::ToolRequested { .. } => "tool_requested",
        ScriptedMockEvent::ToolCompleted { .. } => "tool_completed",
        ScriptedMockEvent::PermissionRequested { .. } => "permission_requested",
        ScriptedMockEvent::Failed { .. } => "failed",
        ScriptedMockEvent::Interrupted { .. } => "interrupted",
        ScriptedMockEvent::TurnCompleted { .. } => "turn_completed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripted_mock_agent_emits_stable_normalized_events() {
        let script = ScriptedMockAgent::new("mock-session").with_turn(
            ScriptedMockTurn::new("turn-1")
                .message_delta("msg-1", "working")
                .message_delta("msg-1", "still working")
                .tool_requested("tool-1", "capo.agent_status")
                .tool_completed("tool-1", "capo.agent_status", "agent is running")
                .turn_completed("done-1"),
        );

        let events = script.turn_events("turn-1").expect("turn events");

        assert_eq!(events.len(), 5);
        assert!(events.iter().all(|event| {
            event.adapter_kind == NormalizedAdapterKind::Mock
                && event.external_session_ref.as_deref() == Some("mock-session")
                && event.timeline_confidence == AdapterTimelineConfidence::Stable
                && event.idempotency_key.is_some()
        }));
        let delta_keys = events
            .iter()
            .filter(|event| event.kind == "adapter.item_delta")
            .filter_map(|event| event.idempotency_key.clone())
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(delta_keys.len(), 2);
        assert!(events.iter().any(|event| {
            event.kind == "adapter.tool_call_completed"
                && event.tool_name.as_deref() == Some("capo.agent_status")
                && event.status.as_deref() == Some("completed")
        }));
    }

    #[test]
    fn scripted_mock_agent_routes_through_static_adapter_dispatch() {
        let adapter = crate::AgentAdapter::scripted_mock(
            ScriptedMockAgent::new("mock-session").with_turn(
                ScriptedMockTurn::new("turn-mock-worker")
                    .message_completed("msg-1", "scripted turn completed"),
            ),
        );

        let session = adapter.open_session(FakeAdapterSessionRequest {
            session_id: SessionId::new("session-mock"),
            agent_name: "mock-worker".to_string(),
        });
        let output = adapter.send_turn(
            &session,
            FakeAdapterTurnRequest {
                turn_id: TurnId::new("turn-mock-worker"),
                agent_name: "mock-worker".to_string(),
                goal: "run scripted turn".to_string(),
            },
        );

        assert_eq!(adapter.binding().variant, "scripted-mock-agent");
        assert_eq!(output.external_session_ref, "mock-session");
        assert_eq!(output.summary, "scripted turn completed");
        assert_eq!(
            adapter
                .scripted_turn_events("turn-mock-worker")
                .expect("scripted events")
                .len(),
            1
        );
    }
}
