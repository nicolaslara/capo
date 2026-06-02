//! CS3: Claude `stream-json` normalizes through the SAME ingestion route the
//! loop already uses (`parse_adapter_events("claude_code", ..)` ->
//! `apply_normalized_adapter_events_with_turn`), never a parallel Claude-only
//! route, and a Claude turn lands as the SAME read-model row SHAPE a Codex turn
//! produces.
//!
//! This mirrors the DP1 ACP loop-route proof and the Codex fixture replay
//! (`e2e_gate.rs`/`sessions.rs`): both providers are driven through the IDENTICAL
//! production write path -- `ServerCommand::ReplayAdapterFixture` ->
//! `parse_adapter_events(<adapter>, ..)` ->
//! `apply_normalized_adapter_events_with_turn` -- which is the single, ungated,
//! provider-neutral ingestion route the loop uses (NOT a Claude-only route, and
//! NOT the dispatch live-execution gate, which CS5 unblocks). The two providers
//! are then asserted to produce the SAME projected read-model row shape for the
//! same logical turn:
//!
//! - exactly one COMPLETED turn ingested,
//! - a session `latest_summary` carrying the assistant's reported message,
//! - exactly one OBSERVED tool result (`instrumentation_level = "observed_only"`)
//!   recorded as `tool.observation_recorded`, distinct from the agent's claim,
//! - a recorded tool call and a `session.summary_updated` event,
//! - equal replay ingestion counts (tool/summary/turn) across providers.
//!
//! No live provider is used.

use super::*;

const CLAUDE_FIXTURE: &str =
    include_str!("../../../capo-adapters/fixtures/claude-code-stream.jsonl");
const CODEX_FIXTURE: &str =
    include_str!("../../../capo-adapters/fixtures/codex-exec-workspace-write.jsonl");

/// A summary of the projected read-model rows a turn produces, captured in a
/// provider-neutral shape so a Claude turn and a Codex turn for the same logical
/// turn can be compared row-for-row.
#[derive(Debug, Eq, PartialEq)]
struct TurnRowShape {
    has_summary: bool,
    tool_observation_count: usize,
    observed_only_count: usize,
    tool_call_count: usize,
    has_summary_updated_event: bool,
    has_observation_event: bool,
    completed_turn_count: usize,
    replay_tool_event_count: usize,
    replay_summary_event_count: usize,
}

fn register_and_start(server: &CapoServer, adapter: &str, session: &str, run: &str, goal: &str) {
    handle(
        server,
        ServerCommand::RegisterAgent {
            name: "loop-route-agent".to_string(),
            adapter: "fake".to_string(),
        },
    );
    handle(
        server,
        ServerCommand::StartSession {
            agent_name: "loop-route-agent".to_string(),
            goal: goal.to_string(),
            adapter: adapter.to_string(),
            session_id: Some(session.to_string()),
            run_id: Some(run.to_string()),
        },
    );
}

/// Drive ONE provider's `stream-json`/JSONL fixture through the shared
/// `ReplayAdapterFixture` ingestion route and capture the projected read-model
/// row shape.
fn ingest_turn(
    adapter: &str,
    fixture_name: &str,
    fixture_jsonl: &str,
    goal: &str,
) -> (TurnRowShape, String) {
    let session = format!("session-cs3-{adapter}");
    let run = format!("run-cs3-{adapter}");
    let turn = format!("turn-cs3-{adapter}");
    let root = temp_root();
    let server = CapoServer::open(ProjectId::new("project-capo"), &root).expect("server");
    register_and_start(&server, adapter, &session, &run, goal);

    let replayed = handle(
        &server,
        ServerCommand::ReplayAdapterFixture {
            adapter: adapter.to_string(),
            session_id: session.clone(),
            run_id: run.clone(),
            turn_id: turn.clone(),
            fixture_name: fixture_name.to_string(),
            fixture_jsonl: fixture_jsonl.to_string(),
        },
    );
    let ServerResponsePayload::AdapterFixtureReplayed(replay) = replayed.payload else {
        panic!("expected adapter fixture replayed response for {adapter}");
    };
    assert!(
        !replay.provider_cli_executed,
        "the {adapter} fixture replay ingests parsed events, it never spawns a provider"
    );

    let state = SqliteStateStore::open(&root).expect("state");
    let session_id = SessionId::new(session.clone());

    let session_projection = state
        .session(&session_id)
        .expect("session")
        .expect("session present");
    let summary = session_projection.latest_summary.unwrap_or_default();

    let observations = state
        .tool_observations_for_session(&session_id)
        .expect("observations");
    let observed_only_count = observations
        .iter()
        .filter(|observation| observation.instrumentation_level == "observed_only")
        .count();

    let tool_calls = state
        .tool_calls_for_session(&session_id)
        .expect("tool calls");

    let events = state
        .recent_events_for_session(&session_id, 128)
        .expect("events");

    let shape = TurnRowShape {
        has_summary: !summary.is_empty(),
        tool_observation_count: observations.len(),
        observed_only_count,
        tool_call_count: tool_calls.len(),
        has_summary_updated_event: events
            .iter()
            .any(|event| event.kind == "session.summary_updated"),
        has_observation_event: events
            .iter()
            .any(|event| event.kind == "tool.observation_recorded"),
        completed_turn_count: replay.completed_turn_count,
        replay_tool_event_count: replay.tool_event_count,
        replay_summary_event_count: replay.summary_event_count,
    };
    (shape, summary)
}

/// CS3 LOOP-ROUTE REUSE: a Claude `stream-json` turn lands as the SAME projected
/// read-model row shape a Codex turn produces, proving Claude rides the shared
/// `parse_adapter_events` + `apply_normalized_adapter_events_with_turn` ingestion
/// route rather than a parallel Claude-only route.
#[test]
fn claude_turn_lands_same_read_model_row_shape_as_codex_through_shared_ingestion_route() {
    let (claude_shape, claude_summary) = ingest_turn(
        "claude",
        "crates/capo-adapters/fixtures/claude-code-stream.jsonl",
        CLAUDE_FIXTURE,
        "Run the workspace edit through Claude",
    );
    let (codex_shape, _codex_summary) = ingest_turn(
        "codex",
        "crates/capo-adapters/fixtures/codex-exec-workspace-write.jsonl",
        CODEX_FIXTURE,
        "Run the workspace edit through Codex",
    );

    // The row SHAPE (what kinds of rows the loop projected) is identical for the
    // same logical turn: one completed turn, an assistant summary, exactly one
    // observed-only tool result distinct from the agent's claim, a recorded tool
    // call, the summary-updated + observation-recorded events, and equal replay
    // ingestion counts.
    assert_eq!(
        claude_shape, codex_shape,
        "Claude and Codex must project the same read-model row shape for the same logical turn"
    );

    // Positive pins on the Claude side (the equality above proves Codex matches).
    assert_eq!(claude_shape.completed_turn_count, 1);
    assert!(claude_shape.has_summary);
    assert_eq!(claude_shape.observed_only_count, 1);
    assert!(claude_shape.has_observation_event);
    assert!(claude_shape.has_summary_updated_event);

    // The Claude summary is the agent's REPORTED assistant claim projected
    // through the SAME content-hashed loop route Codex uses (raw text is
    // content-hashed, not rendered -- the connector retention policy), so it
    // names the `claude_code` adapter and carries the content-hash anchor, NOT a
    // fabricated fake-adapter summary and NOT the raw observed tool result.
    assert!(
        claude_summary.contains("claude_code") && claude_summary.contains("content_hash="),
        "the Claude summary must be the content-hashed assistant claim from the shared route, got: {claude_summary}"
    );
}
