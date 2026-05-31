# Streaming Transport References

## Objective

Record the local and external sources that shape the streaming-transport
workpad. Dated claims reflect the observed state on 2026-05-29.

## Local Architecture Sources

- `workpads/architecture/state-model.md`
  - Key facts: SQLite events are append-only and the source of operational
    truth; read models are rebuildable from events plus artifacts; projection
    code tolerates duplicate raw inputs via idempotency keys; streaming
    messages/items/tool calls are SQLite events while runtime stdout/stderr are
    file artifacts plus event pointers (no unbounded blobs in event rows). This
    is the rule the thread read-model projection and the `events_after` tail must
    obey: never stream by re-serializing a full read model; tail discrete
    committed events.
- `workpads/architecture/acp-replay-dedupe.md`
  - Key facts: ACP `session/update` notifications must not be made directly
    authoritative for UI state; raw updates are persisted, normalized into Capo
    events through idempotent mappers, and only Capo event sequences project read
    models. `tool_call_update` fields are partial replacements; plan updates are
    complete replacements; after `session/cancel` agents may still send pending
    updates before responding with stop reason `cancelled`. Informs the
    notification-variant and mid-turn-interrupt semantics (a cancel does not
    immediately silence the stream) and the gap/dup-free backlog-to-live seam.
- `workpads/architecture/boundaries.md`
  - Key facts: Capo is the controller; input surfaces submit commands and render
    read models; the event log is authoritative; UI/voice/mobile must not own
    orchestration state. This is why `capo-web` and the CLI are clients of the
    contract and the transport layer never owns orchestration state.
- `workpads/harness-research/daily-driver-review.md`
  - Key facts (grounded, the transport + chat dimensions): transport scores 1.5
    -- a clean typed `ServerCommand` boundary and a working loopback-TCP CLI
    client, but `handle_stream` is one-line-in/one-line-out per connection on a
    serial single-threaded accept loop, with no notification variant, no
    SSE/WS/streaming, no fan-out, and no tokio/axum/broadcast in any crate
    (verified zero matches at review time; `capo-web` was untracked and not
    counted). Chat scores 1.0 -- default chat is a scripted fake echo
    (`FakeBoundaryController::redirect` writing a canned `latest_summary`); the
    only live path is a blocking one-shot read; no streaming, threads, mid-turn
    interrupt, or branching. The recommended remedy is exactly this workpad:
    JSON-RPC 2.0 with a notification variant, `Subscribe { session_id,
    from_sequence }` tailing the event log via a broadcast channel, a concurrent
    accept loop with timeouts and in-band Cancel, a multi-turn thread read model,
    and Ctrl-C mid-turn interrupt.

## Local Product And Implementation Sources

- `crates/capo-server/src/transport.rs`
  - Key facts: `serve_tcp` enforces a loopback-only listener, opens one
    `CapoServer`, then runs a serial accept loop that calls `handle_stream` once
    per connection. `handle_stream` reads exactly one bounded line
    (`MAX_TRANSPORT_FRAME_BYTES`) and writes exactly one response line, then
    returns -- one frame in, one frame out, no persistence, no push. This is the
    serial single-threaded transport ST2/ST3 replace.
- `crates/capo-server/src/transport/codec.rs`
  - Key facts: the custom newline-JSON codec (`encode_request`/`decode_request`,
    `encode_success_response`/`encode_error_response`) carries `request_id`,
    `origin` (`client_id`/`actor_id`/`input_origin`), and the typed `command`.
    There is no JSON-RPC `id`, no notification shape, and no method dispatch;
    this is the codec layer JSON-RPC 2.0 framing replaces while preserving
    `request_id`/origin propagation.
- `crates/capo-server/src/transport/contract.rs` and
  `crates/capo-server/contract/` -- the published wire contract (ST9)
  - Key facts: `contract.rs` (re-exported as `capo_server::contract`) is the
    single in-code source for the published contract -- `wire_samples()` emits
    real frames through the live `jsonrpc`/`codec`/`EventNotification` path,
    `contract_schema()` is the language-neutral schema, and `sse_frame()` is the
    canonical SSE block. The checked-in `contract/` dir holds
    `jsonrpc-schema.json` (described schema), `snapshots/*.json` (11 enforced
    wire frames), `capo-wire.d.ts` (optional downstream TS types from the
    schema), and `README.md` (cross-team handoff). The regenerate-and-diff
    `tests::contract` cases (run `CAPO_REGENERATE_WIRE_SNAPSHOTS=1` to rewrite)
    fail on any byte-level wire drift, and a compile-enforced exhaustive `match`
    over the command/payload/error enums keeps the schema from lagging the code.
    All verified without a web client.
- `crates/capo-server/src/types.rs` -- `ServerCommand` and `ServerResponsePayload`
  - Key facts: `ServerCommand` is a closed enum (`SteerAgent`, `StopAgent`,
    `Dashboard { recent_event_limit }`, `PlanDispatch`/`GateDispatch`/
    `RunDispatchLocal`/`RunLiveProviderLocal`, `Recover`, etc.) with no
    `Subscribe` variant. `ServerResponsePayload` has `Dashboard`, `DispatchRun`,
    `Recovery`, etc., but no streaming/notification variant. `SessionSummary`
    exposes a single `latest_summary: Option<String>` -- the polled field the
    multi-turn thread read model replaces. ST4/ST5 add the `Subscribe` command,
    a notification payload, and the thread projection here.
- `crates/capo-web/src/main.rs` -- the poll-SSE antipattern
  - Key facts: an axum + tokio facade over the typed boundary
    (`/api/dashboard`, `/api/commands`, `/api/events`). The `events` handler is
    an `IntervalStream` on a 1500ms `tokio::time::interval` that re-runs
    `ServerCommand::Dashboard { recent_event_limit: 50 }` inside `spawn_blocking`
    and emits the whole re-serialized read model as one SSE event per tick --
    the exact `latest_summary`-poll antipattern this workpad deletes. The module
    doc states `CapoServer` and the SQLite query store are non-`Send` across
    awaits, so every handler opens a fresh `CapoServer` inside `spawn_blocking`;
    this is the `!Send`-across-await constraint ST8 resolves. It serves the
    frozen `web/app/dist` static assets via `ServeDir`. The crate is a live
    workspace member but untracked (ST8 git-tracks it).
- `crates/capo-runtime/src/lib.rs`
  - Key facts: the local runner is synchronous -- `spawn_process` writes
    stdout/stderr to files and the run is buffered then capped after exit via
    `capped_output`, which returns `Err(OutputLimitExceeded)` on overflow so a
    long successful run is misclassified as an error. There is no stdin path.
    Descendant reaping is real: `process_group(0)` on spawn and
    `terminate_process_group` sending `-TERM` to the group. `RemoteProcessRunner`
    and the `Fake` runner are preserved shapes. ST1 moves this to tokio,
    streams output, adds stdin, fixes the success-classification bug, and ports
    the process-group kill regression test plus a new orphan-after-cancel test.
- `crates/capo-state/src/lib.rs` and `crates/capo-state/src/queries.rs` --
  sequence and watermarks
  - Key facts: `append_event` writes to a single append-only `events` table and
    takes `sequence = last_insert_rowid()`, then updates the `default`
    projection watermark in-transaction; `queries.rs` exposes `watermark(name)`
    and `recent_events_for_session` (selecting `sequence, event_id, kind, ...,
    payload_json, redaction_state` from `events`). A monotonic sequence and a
    durable watermark already exist; ST4 adds `events_after(since_sequence)` as a
    forward read and feeds a broadcast channel from the append path. Recovery
    today is the blunt `mark_active_runs_exited_unknown`; liveness-aware reattach
    is a `safety-gates` concern, not this workpad.
- `crates/capo-state/src/projections.rs` -- the redaction guard
  - Key facts: emission is gated on `RedactionState::ContainsSensitive` /
    `RedactionState::Unknown` and `sensitivity_classification == "secret_derived"`
    (the artifact-safety check). ST7 reuses this guard on the broadcast/SSE
    egress path so no frame leaves the process unredacted.

## External Sources

- https://www.jsonrpc.org/specification
  - Observed 2026-05-29.
  - Key facts: JSON-RPC 2.0 defines request objects with `jsonrpc`/`method`/
    `params`/`id`; a Notification is a request with the `id` omitted and MUST NOT
    be replied to; responses carry the matching `id` plus `result` or `error`.
    This is the framing ST2 adopts -- request/response keyed by `id` for typed
    commands, and the id-less Notification as the server-initiated event/push
    variant carrying the event tail and live turn output.
- https://docs.rs/axum/latest/axum/
  - Observed 2026-05-29.
  - Key facts: axum's `response::sse` (`Sse`, `Event`, `KeepAlive`) wraps a
    `Stream` of `Event`s as a Server-Sent Events response; this is the API
    `capo-web` already uses for `/api/events` and that ST8 keeps while swapping
    the underlying stream from a polling `IntervalStream` to the broadcast tail.
- ACP `session/update` + JSON-RPC and other harness/transport comparisons:
  inherited via `workpads/harness-research/references.md` (dated source links
  for ACP, Codex app-server, OpenCode, Claude Code, Codex CLI, Cursor, Cline,
  and peers). Those external sources are not re-fetched here; the ACP-specific
  replay/streaming facts this workpad relies on are captured in
  `workpads/architecture/acp-replay-dedupe.md` above. The live ACP JSON-RPC wire
  adapter itself is a `depth` concern, not this workpad.
