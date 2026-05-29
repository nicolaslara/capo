//! Agent adapter and provider connector scaffolding.
//!
//! P6 adds fixture parsers for Codex, Claude Code, and ACP streams. The
//! parsers preserve provider-specific records as adapter facts and emit
//! normalized adapter events for the controller pipeline.

mod acp_client;
mod adapter;
mod event;
mod local_subscription;
mod provider_parsers;
mod scripted_mock_agent;

pub use acp_client::{AcpClientCall, AcpSessionSetupPlan};
pub use adapter::{
    AdapterSession, AdapterSessionRequest, AgentAdapter, AgentAdapterHandle, FakeAdapter,
    FakeProviderConnector, FakeProviderInfo, ProviderConnector, TurnOutput, TurnRequest,
};
pub use event::{
    AdapterFixtureParse, AdapterParseError, AdapterParseResult, AdapterTimelineConfidence,
    AdapterToolObservation, NormalizedAdapterEvent, NormalizedAdapterKind,
};
pub use local_subscription::{
    LocalAdapterLaunchPlan, LocalAdapterSmokeError, LocalAdapterSmokePlan, LocalAdapterSmokeResult,
    LocalAdapterSmokeRunner, scan_artifacts_for_sensitive_markers,
};
pub use provider_parsers::{AcpAdapter, ClaudeCodeAdapter, CodexExecAdapter};
pub use scripted_mock_agent::{ScriptedMockAgent, ScriptedMockEvent, ScriptedMockTurn};

/// Initial adapter variants named by the architecture.
pub const PLANNED_ADAPTERS: &[&str] =
    &["fake", "scripted-mock", "codex-exec", "claude-code", "acp"];

#[cfg(test)]
mod tests;
