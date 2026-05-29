# Depth References

## Objective

Record the local and external sources that shape the depth workpad: the live ACP
JSON-RPC adapter, Claude as a second write adapter, real `MarkdownMemoryBackend` +
FTS5 retrieval, the first OS sandbox tier, git worktree isolation, and optional
OTel. Dated claims reflect 2026-05-29.

## Local Architecture Sources

- `workpads/architecture/memory-architecture.md`
  - Key facts: prototype `MemoryBackend` order is `MarkdownMemoryBackend` ->
    `FakeMemoryBackend` -> `SqliteFtsMemoryBackend`; FTS5 is the first retrieval
    backend, embeddings/vector/graph are deferred; operational truth stays in
    SQLite events/projections/artifacts and human truth in markdown; every memory
    record carries source provenance + content hash; `search` filters out
    invalidated/rejected/superseded/unauthorized/redacted records; packets are
    fractional, source-linked, replayable from the packet artifact; secrets/voice
    transcripts are never valid memory sources; `MemoryJob` kinds include
    `extract_facts`/`index_fts`/`invalidate`/`rebuild` with idempotent source
    ranges.
- `workpads/architecture/runtime-tunnel.md`
  - Key facts: `RuntimeRunner` owns process lifecycle, `ConnectivityTunnel` owns
    reachability, separate boundaries; Capo never claims hard sandboxing unless the
    runtime actually enforces it through OS/container/VM mechanisms and tests prove
    it; Linux sandbox profiles are explicitly deferred until Capo can distinguish
    audited local process control from enforced sandboxing; runner/tunnel enums are
    static-dispatch and swappable; start ordering is append-first
    (`runtime.start_requested` before spawn) with orphan reaping on recovery.
- `workpads/architecture/protocol-provider.md`
  - Key facts: `AgentAdapter` enum is `CodexExec`/`ClaudeCode`/`Acp`/`Fake`; ACP is
    stdio JSON-RPC 2.0 with Capo as the client; adapter agent-call surface is
    `initialize`/`session/new`/`session/prompt`/`session/cancel` plus
    `session/load`/`session/resume` when advertised; Capo implements
    `session/request_permission`, `fs/*`, and `terminal/*` client handlers only
    when wrapper tools + scopes exist; Claude runs `claude -p --output-format
    stream-json --verbose` (observed `2.1.150`) and scrubs unrelated
    `ANTHROPIC_API_KEY`/`ANTHROPIC_AUTH_TOKEN`; Codex/Claude subscription connectors
    record auth mode only and never read credential material; adapters never own
    process groups (the runtime does).
- `workpads/architecture/acp-replay-dedupe.md`
  - Key facts: integer `protocolVersion` (stable `1`); `session/resume` is the
    default reconnect (no replay events) and `session/load` is import/repair only;
    raw updates persisted before normalization and never authoritative for read
    models; `toolCallId` is a stable timeline key, plans/`tool_call_update`
    content/locations are replacements, message chunks lack stable IDs so Capo
    finalizes content hashes + `message_boundary_confidence`; idempotency-key shape
    `acp:{session}:{event_family}:{timeline_key}:{operation}:{operation_version}`;
    `adapter_replay_batches`/`adapter_raw_updates`/`adapter_timeline_keys` tables
    and `adapter.replay_*`/`adapter.attach_*` event kinds; cancel-while-pending
    answers the permission `cancelled` and accepts late updates.
- `workpads/harness-research/daily-driver-review.md`
  - Key facts: memory dimension scored 1.5 because the live packet is four
    hardcoded strings, the eligibility filter is dead in production, and there is
    zero retrieval/search; adapters scored 1.5 (ACP fixture-only, Claude no-tools,
    no live JSON-RPC/resume/tool round-trip); runtime scored 1.5 (no real sandbox
    of its own, delegates to CLI flags); eval/observability scored 1.5 (no
    wall-clock timing, no OTel); the Phase 4 roadmap names exactly this workpad's
    scope: kill the four hardcoded strings, `MarkdownMemoryBackend` + FTS5,
    extraction/staleness `MemoryJob`, live ACP adapter + resume/load, first OS
    sandbox tier + git worktree isolation + optional OTel.

## Local Product And Implementation Sources

- `crates/capo-adapters/src/acp_client.rs`
  - Key facts: `AcpAdapter::session_setup_plan` is fixture-only capability planning
    over `ToolDefinition`s (negotiated `protocol_version: 1`, advertised
    capabilities, `runtime_started: false`, `provider_cli_executed: false`);
    `wrapper_request_for_client_call` already maps `fs/read_text_file` ->
    `capo.file_read`, `fs/write_text_file` -> `capo.file_write`, `terminal/run` ->
    `capo.shell_run`, rejecting un-advertised capabilities. DP1 promotes this to a
    real JSON-RPC wire client.
- `crates/capo-adapters/src/adapter.rs`
  - Key facts: today the `AgentAdapter` enum is only `Fake`/`ScriptedMock` with
    concrete `FakeAdapterSessionRequest`/`FakeAdapterTurnOutput`/`FakeAdapterSession`
    signatures; `real-turn-loop` replaces these with a provider-neutral
    `AgentAdapter` trait + types that DP1 (ACP) and DP4 (Claude) implement;
    `NormalizedAdapterEvent` and `scripted_turn_events` are the existing ingestion
    vocabulary the live adapters must reuse.
- `crates/capo-memory/src/lib.rs`
  - Key facts: `MemoryBackend` is currently only `Fake(FakeMemoryBackend)`;
    `build_source_linked_packet` already enforces secret-exclusion,
    `review_state != Reviewed` exclusion, and token-budget exclusion with
    per-item inclusion/exclusion reasons and replayable packet/explanation
    markdown; DP5 adds `Markdown`/`SqliteFts` variants and real retrieval while
    preserving these provenance semantics.
- `crates/capo-state` (`queries.rs`, `projections.rs`)
  - Key facts: `SqliteQueries::packet_eligible_memory_records`
    (`queries.rs:846`) and `MemoryRecordProjection::is_packet_eligible`
    (`projections.rs:394`) implement the strict eligibility filter that is dead in
    production; DP5 wires them into packet candidate selection. State also hosts the
    event kinds/tables ACP replay (DP2) and memory jobs (DP5-DP6) add.
- `crates/capo-runtime/src/lib.rs`
  - Key facts: `RuntimeRunner` enum includes `LocalProcess`/`RemoteProcess`/`Fake`;
    `LocalProcessRunner` is process-group-aware (`command.process_group(0)`,
    `terminate_process_group`) with bounded output (`capped_output` /
    `OutputLimitExceeded`) and `run_dir = artifact_root/run_id`; no real OS sandbox
    exists (DP7 adds one behind this boundary) and no worktree isolation exists
    (DP8 adds one).
- `crates/capo-server/src/live_provider.rs` (+ `util.rs`)
  - Key facts: live execution is gated by `live_execution_opt_in` and the
    `CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT` opt-in env (`:245`), with
    `mock_provider_output_jsonl` for deterministic tests; preflight already supports
    `codex` and `claude` (`:70`, `util.rs:39`); DP4 rides this preflight for the
    live Claude write path and DP11 mirrors `CAPO_SERVER_RUN_CODEX_LIVE` for the
    live smokes.
- `crates/capo-eval/src/lib.rs`
  - Key facts: `TaskOutcomeReport` derives `duration_sequence_span` from an
    event-sequence delta (`:123`), not wall-clock; there is no OTel and no real
    timing; DP9 adds wall-clock timing alongside the sequence span and the optional
    OTel exporter.

## External Sources

- Agent Client Protocol repository clone (`workpads/references/repos/agent-client-protocol`)
  - Observed 2026-05-29.
  - Key facts: ACP docs under `docs/protocol/` and `schema/schema.json` define
    `initialize`/`session/new`/`session/prompt`/`session/cancel`/`session/update`,
    the `loadSession`-gated `session/load` (full conversation replay) and stabilized
    `session/resume`, `request_permission`, integer `protocolVersion` (stable `1`),
    stable `toolCallId`, replacement `tool_call_update`/plan semantics, and the
    Message ID RFD noting stable message IDs are gated behind `unstable_message_id`
    (so dedupe of ID-less chunks needs content hashes). This is the wire contract
    DP1-DP3 implement against.
- OpenAI codex `sandboxing` / `linux-sandbox` / `bwrap` crates
  (`workpads/references/repos/openai-codex/codex-rs/{sandboxing,linux-sandbox,bwrap}`)
  - Observed 2026-05-29.
  - Key facts: the `sandboxing` crate gates `seatbelt` behind `target_os = "macos"`
    (with `.sbpl` base + network policy files) and `landlock` behind
    `target_os = "linux"`, exposing a `SandboxManager`/`SandboxExecRequest`/
    `SandboxCommand` surface with `policy_transforms`; `linux-sandbox` provides the
    bwrap launcher (`bundled_bwrap`/`bazel_bwrap`/`bwrap.rs`) and landlock
    enforcement; `bwrap` bundles the bubblewrap binary. This is the model for DP7's
    macOS-seatbelt / linux-landlock+bwrap tiers behind the runtime boundary.
- OpenAI codex `otel` crate
  (`workpads/references/repos/openai-codex/codex-rs/otel`)
  - Observed 2026-05-29.
  - Key facts: the crate is structured as `config` / `otlp` / `provider` /
    `trace_context` / `metrics` / `events` / `targets`, i.e. an opt-in OTLP exporter
    with configurable targets and trace-context propagation. This is the model for
    DP9's optional, off-by-default span/timing exporter across loop/tools/runtime.
- OpenHands docker runtime
  - Observed 2026-05-29 (rechecked from prior harness research; product surfaces
    change quickly).
  - Key facts: OpenHands enforces isolation by running agent actions inside a Docker
    container runtime (one container per session/workspace) rather than path-prefix
    checks, and supports git-based workspace isolation. This is the peer baseline
    DP7 (OS sandbox tier) and DP8 (git worktree isolation) move toward without
    requiring a full container runtime for the first tier.
