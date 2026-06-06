//! Agent adapter and provider connector scaffolding.
//!
//! P6 adds fixture parsers for Codex, Claude Code, and ACP streams. The
//! parsers preserve provider-specific records as adapter facts and emit
//! normalized adapter events for the controller pipeline.

mod acp_client;
mod acp_live;
mod acp_replay;
mod acp_wire;
mod adapter;
mod claude_live;
mod codex_live;
mod event;
mod local_subscription;
mod permission_request;
mod provider_parsers;
mod scripted_mock_agent;

pub use acp_client::{
    AcpClientCall, AcpHttpMcpServer, AcpPermissionProfile, AcpSessionLockdown, AcpSessionSetupPlan,
};
pub use acp_live::{
    ACP_LIVE_PREFLIGHT_OPT_IN_ENV, ACP_LIVE_RUN_OPT_IN_ENV, AcpLiveAdapter, AcpLiveError,
    LiveAcpSession, PersistentAcpSession, acp_live_gate_open, turn_output_from_transcript,
};
pub use acp_replay::{
    AcpDedupeConfidence, AcpImportConfidence, AcpRawUpdateRecord, AcpReconcileDecision,
    AcpReconciledCandidate, AcpReplayEngine, AcpReplayPlan, AcpReplaySource, AcpTimelineKeyRecord,
    AcpTimelineKind, ExistingItemFingerprint,
};
pub use acp_wire::{
    ACP_PROTOCOL_VERSION, ACP_PUMP_READ_TIMEOUT, AcpClientCallRecord, AcpPermissionRoundTrip,
    AcpResumeOutcome, AcpTransport, AcpTurnTranscript, AcpWireClient, AcpWireError,
    PipedProcessTransport, RecvOutcome, ScriptedAcpTransport, ScriptedServerFrame,
};
pub use adapter::{
    AdapterSession, AdapterSessionRequest, AgentAdapter, AgentAdapterHandle, FakeAdapter,
    FakeProviderConnector, FakeProviderInfo, PermissionDeliveryAck, ProviderConnector, TurnOutput,
    TurnRequest,
};
pub use claude_live::{
    CLAUDE_LIVE_RUN_OPT_IN_ENV, ClaudeCodeLiveAdapter, claude_live_chat_gate_open,
};
pub use codex_live::{
    CODEX_LIVE_PREFLIGHT_OPT_IN_ENV, CODEX_LIVE_RUN_OPT_IN_ENV, CodexLiveAdapter,
    CodexLiveChatError, codex_live_chat_gate_open,
};
pub use event::{
    AcpExternalRef, AdapterFixtureParse, AdapterParseError, AdapterParseResult,
    AdapterTerminalOutcome, AdapterTimelineConfidence, AdapterToolObservation,
    NormalizedAdapterEvent, NormalizedAdapterKind,
};
pub use local_subscription::{
    LocalAdapterLaunchPlan, LocalAdapterSmokeError, LocalAdapterSmokePlan, LocalAdapterSmokeResult,
    LocalAdapterSmokeRunner, scan_artifacts_for_sensitive_markers,
};
pub use permission_request::{
    AcpOptionMapping, AcpPermissionDecider, AcpPermissionOption, AcpPermissionOptionKind,
    AcpPermissionOutcome, AdapterPermissionCancelReason, AdapterPermissionRequest,
    AdapterPermissionResponse, FailClosedPermissionDecider, map_acp_options_trusted_local,
};
pub use provider_parsers::{AcpAdapter, ClaudeCodeAdapter, CodexExecAdapter};
pub use scripted_mock_agent::{ScriptedMockAgent, ScriptedMockEvent, ScriptedMockTurn};

/// Initial adapter variants named by the architecture.
pub const PLANNED_ADAPTERS: &[&str] =
    &["fake", "scripted-mock", "codex-exec", "claude-code", "acp"];

#[cfg(test)]
mod tests;
