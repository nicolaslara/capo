# Streaming Transport Tasks

## Objective

Turn Capo's synchronous one-frame-per-connection transport into a streaming
multi-client surface: a tokio runtime with incremental output, JSON-RPC 2.0
framing with a notification variant over a persistent bidirectional connection,
`events_after(since_sequence)` plus a broadcast channel and a typed `Subscribe`
command, a multi-turn thread read model projected from events, typed mid-turn
interrupt, redaction-on-emit, and the evolution of the existing `crates/capo-web`
bridge from 1500ms Dashboard polling to a broadcast-backed event tail. The
deliverable is the server-side web contract the out-of-scope web client consumes,
verified without a web client.

## Status

Planned. Phase 2 - Stream it (interactive loop + server-side web contract).
Depends on `real-turn-loop`: real turns must produce incremental output and a
real `TurnFinished` event to stream. `ST0` defines routing, scope, and the
web-boundary statement. All implementation tasks remain pending.

## Feature Set

- tokio-based `capo-runtime` with incremental stdout/stderr streaming and stdin,
  preserving provable process-group descendant reaping.
- JSON-RPC 2.0 request/response plus a notification variant over a persistent
  bidirectional connection.
- Concurrent accept loop with per-connection timeouts and in-band typed `Cancel`.
- `events_after(since_sequence)` query, a broadcast channel, and a typed
  `Subscribe { session_id, from_sequence }` command that tails the event log.
- A multi-turn thread read model projected from events, replacing
  `latest_summary` polling.
- Typed mid-turn interrupt wired through the stream to Ctrl-C.
- Redaction-on-emit applied to the broadcast/SSE egress path.
- The existing `crates/capo-web` bridge evolved from Dashboard polling to an
  event tail and git-tracked.
- A published language-neutral JSON-RPC/SSE schema with checked-in wire
  snapshots, plus a documented cross-team handoff for the web agent.

## ST0 - Workpad, Routing, Scope, And Web-Boundary Statement

Status: pending.

Acceptance:

- Decide and record that streaming/transport belongs in a dedicated workpad, not
  inside `real-turn-loop` or `operator-control`: the loop produces events, this
  workpad makes them streamable to many clients.
- State the web boundary explicitly: `crates/capo-web` (the server-side Rust
  axum bridge) is IN scope to evolve; `web/app` and `web/dashboard` front-end
  source are frozen and out of scope and are not edited by this workpad.
- Record the key decision that the HTTP/SSE bridge already exists as `capo-web`;
  this workpad makes it tail the event log instead of polling Dashboard, it does
  not build a new bridge.
- Record the boundary that the event log stays authoritative, `Subscribe` tails
  it via broadcast, read models rebuild from it, and the transport layer never
  owns orchestration state.
- List the excluded concerns and their owners: permission-card content and
  enforcement (`safety-gates`), verification progress semantics
  (`safety-gates`), goal projections (`goal-autonomy`), and any web UI.
- Add the verification invariant: no task in this workpad completes on operator
  self-attestation alone; every manual smoke is paired with a deterministic
  assertion (wire snapshot, exit status, or replay).

Verification:

- `workpads/streaming-transport/tasks.md`, `knowledge.md`, and `references.md`
  exist and follow the conventional format.
- Scope decision reviewed against `workpads/architecture/boundaries.md` and
  `workpads/architecture/state-model.md`.
- `git diff --check`.

Must not do:

- Do not edit `web/app`, `web/dashboard`, `TASKS.md`, `WORKPADS.md`,
  `AGENTS.md`, or `WORKING.md` from this workpad.

## ST1 - tokio capo-runtime With Incremental Streaming And Stdin

Status: pending.

Acceptance:

- Move `crates/capo-runtime` to a tokio async runner so a live run emits stdout
  and stderr incrementally instead of buffer-then-cap-after-exit
  (`crates/capo-runtime/src/lib.rs:315-349`).
- Expose a way to write to a run's stdin so the controller can talk to a process
  mid-flight; today there is no stdin path.
- Preserve provable descendant-process reaping: keep the process-group
  setup/kill escalation (`process_group(0)` /
  `terminate_process_group`, `lib.rs:341,594`) and port the existing
  process-group kill regression test to the tokio runner unchanged in intent.
- Add a new orphan-after-cancel reaping test: a cancelled run with a spawned
  child leaves no surviving process group.
- Fix the output-cap-discards-success classification: a successful run that
  exceeds the output cap is NOT classified as failed; output is
  streamed-and-truncated with truncation recorded as artifact metadata.
- Keep `RemoteProcessRunner` and the `Fake` runner shapes intact so deterministic
  tests do not require a tokio reactor where they did not before.

Verification:

- Ported process-group kill regression test passes on the tokio runner.
- New deterministic `>cap-successful-run` test asserts success classification
  plus a truncation marker.
- `cargo fmt` and focused `cargo test -p capo-runtime`.

Must not do:

- Do not weaken or remove process-group reaping to simplify the async migration.

## ST2 - JSON-RPC 2.0 Framing With A Notification Variant

Status: done.

Acceptance:

- Define JSON-RPC 2.0 request/response framing over a persistent bidirectional
  connection, replacing the one-line-in/one-line-out newline-JSON codec
  (`crates/capo-server/src/transport.rs:79-106`,
  `crates/capo-server/src/transport/codec.rs`).
- Add a notification variant (no `id`, server-initiated) so the server can push
  events to a connected client without a prior request.
- Map every existing typed `ServerCommand` (`crates/capo-server/src/types.rs:107`)
  onto a JSON-RPC method without changing the domain command semantics.
- Preserve request-identity idempotency: a JSON-RPC `id` and the existing
  `request_id`/origin propagation continue to flow through to the server handler.
- Keep the loopback-only listener constraint already enforced in
  `serve_tcp` (`transport.rs:13-33`).
- Document that the wire format is the codec layer below the AgentAdapter
  boundary; it does not become the domain model.

Verification:

- Deterministic codec round-trip tests for request, response, and notification
  framing against checked-in wire fixtures.
- `cargo fmt` and focused `cargo test -p capo-server`.

Evidence:

- The JSON-RPC 2.0 codec (`crates/capo-server/src/transport.rs`,
  `transport/jsonrpc.rs`, `transport/codec.rs`) replaced the legacy
  `{"ok":true/false,...}` newline-JSON envelope. The bounded-frame test
  `tcp_transport_rejects_oversized_frames_before_json_decode`
  (`crates/capo-server/src/tests/transport.rs`) still asserted the removed
  `"ok":false` shape; it now asserts the JSON-RPC error frame
  (`jsonrpc:2.0`, `id:null`, `error.data.kind=protocol`, message
  `"request frame is too large"`) so it verifies the same pre-JSON-decode
  rejection against the new wire contract.
- Objective gate run from `/Users/nicolas/devel/capo-wt/streaming-transport`:
  `cargo fmt --check` ok; `cargo clippy --all-targets --all-features -- -D warnings`
  ok; `cargo test --workspace` ok (no failures; the previously failing
  `tests::transport::tcp_transport_rejects_oversized_frames_before_json_decode`
  now passes: 1 passed, 0 failed, 60 filtered out in the focused run).

## ST3 - Concurrent Accept Loop With Timeouts And In-Band Cancel

Status: done.

Acceptance:

- Replace the serial single-threaded accept loop (`transport.rs:28-32`) with a
  concurrent accept loop that handles multiple persistent connections.
- Add per-connection read/idle timeouts so a stalled or abandoned client does
  not hold resources indefinitely.
- Add an in-band typed `Cancel` frame that a client can send on its open
  connection to abort an in-flight request/turn, distinct from closing the
  socket.
- Keep the bounded-frame protection (`MAX_TRANSPORT_FRAME_BYTES`,
  `transport.rs:11`) and loopback-only enforcement.
- Ensure concurrency does not introduce a second writer to the event log:
  document that concurrent writers are unsupported until the `safety-gates`
  write lock lands, and serialize server writes behind the existing handler.

Verification:

- Deterministic test with two concurrent connections receiving independent
  responses.
- Deterministic timeout test for an idle/stalled connection.
- In-band `Cancel` test asserting an in-flight request aborts without dropping
  the connection.
- `cargo fmt` and focused `cargo test -p capo-server`.

Evidence:

- `crates/capo-server/src/transport.rs` replaced the serial accept loop
  (accept-one / `handle_stream` / loop) with a concurrent, task-per-connection
  accept loop (`serve_tcp_with_handler`, thread-per-connection) over a
  persistent per-connection read loop (`handle_connection`). The read side runs
  on its own thread feeding a single `ConnEvent` channel (`Incoming` /
  `Closed` / `Result`), so the loop blocks on one `recv` yet reacts the instant
  either an in-band `Cancel` arrives or the handler completes (no polling). The
  public `serve_tcp`/`send_tcp` signatures are unchanged; `max_requests` keeps
  its meaning (connections accepted) so the ST2 round-trip/recovery test is
  untouched.
- Per-connection idle timeout via `TcpStream::set_read_timeout`
  (`ServeConfig`, default 300s) folds a `WouldBlock`/`TimedOut` read into a
  clean connection close so a stalled/abandoned client cannot hold a connection
  thread. The bounded-frame `MAX_TRANSPORT_FRAME_BYTES` protection and
  loopback-only listener enforcement are preserved (oversized frame still
  yields the ST2 `error.data.kind=protocol` "request frame is too large" frame).
- In-band typed `Cancel`: a JSON-RPC `cancel` notification (no `id`,
  `params.request_id`) aborts the matching in-flight request and emits a typed
  `error.data.kind=cancelled` frame while keeping the connection open; a
  generation tag discards the worker's later result. A `RequestHandler` seam
  hands each request a `CancellationToken` for cooperative stop (the production
  `CapoServerHandler` routes through the single `CapoServer::handle`
  serialization point and does not add a second writer; the module doc records
  that concurrent writers stay unsupported until the `safety-gates` write lock).
- Tests added in `crates/capo-server/src/tests/transport.rs` (scripted handler,
  no live provider): `two_concurrent_connections_receive_independent_responses`
  (a `Barrier(2)` that only completes if the accept loop is truly concurrent),
  `idle_connection_is_closed_after_the_read_timeout`, and
  `in_band_cancel_aborts_in_flight_request_without_dropping_connection` (asserts
  the typed `cancelled` frame, then a follow-up request on the same connection
  still succeeds).
- Objective gate run from `/Users/nicolas/devel/capo-wt/streaming-transport`:
  `cargo fmt --check` ok; `cargo clippy --all-targets --all-features -- -D
  warnings` ok (exit 0); `cargo test --workspace` ok (capo-server: 63 passed, 0
  failed, 1 ignored; capo-cli integration `server_transport`: 11 passed; all
  other crates green; 0 failures workspace-wide). Focused
  `cargo test -p capo-server --lib transport`: 7 passed, 0 failed.

## ST4 - events_after, Broadcast Channel, And Typed Subscribe

Status: done. (The `capo-web` `/api/events` conversion bullet is deferred to ST8,
which owns git-tracking `capo-web` and is the bullet's stated home -- "full
delete covered in ST8"; `capo-web` does not exist in this worktree yet, so the
core query/broadcast/Subscribe contract landed here and ST8 wires the SSE tail
onto it.)

Acceptance:

- Add `events_after(since_sequence)` to the state query surface
  (`crates/capo-state/src/queries.rs`) returning ordered events strictly after a
  caller-supplied sequence watermark.
- Add a broadcast channel fed by the append path so newly committed events are
  fanned out to all live subscribers without polling.
- Add a typed `Subscribe { session_id, from_sequence }` `ServerCommand` variant
  and a streaming response/notification variant in `ServerResponsePayload`
  (`types.rs:107-221`, which today have no `Subscribe` or notification variant).
- A subscriber first receives the catch-up backlog via `events_after(from_sequence)`,
  then live events from the broadcast channel, with no gap and no duplicate at
  the seam (gap/dup proven by a sequence-continuity test).
- Never stream by re-serializing a full read model per interval; the broadcast
  carries discrete committed events keyed by sequence.
- `capo-web` `/api/events` is converted from polling `ServerCommand::Dashboard`
  (`crates/capo-web/src/main.rs:150-165`) to a broadcast-backed tail keyed by
  `from_sequence`; the poll loop is deleted (full delete covered in ST8).

Verification:

- Deterministic test that `events_after(n)` returns only events with
  `sequence > n` in order.
- Sequence-continuity test across the backlog-to-live seam (no gap, no
  duplicate).
- `cargo fmt` and focused `cargo test -p capo-state -p capo-server`.

Must not do:

- Do not stream by re-serializing a full read model per interval (the poll
  antipattern this workpad removes).

Evidence:

- `events_after(since_sequence, limit)` (and a session-scoped
  `events_after_for_session`) added to `crates/capo-state/src/queries.rs`:
  `SELECT ... WHERE sequence > ?1 ORDER BY sequence ASC LIMIT ?2`, a forward read
  of the existing monotonic `last_insert_rowid()` sequence. It returns events
  strictly after the watermark (never the watermark event itself), so pairing it
  with a broadcast resume from the same watermark is gap- and dup-free.
- Broadcast channel: new `crates/capo-state/src/broadcast.rs`
  (`EventBroadcaster` / `EventSubscription`), a small `std::sync::mpsc` fan-out
  (no tokio reactor, keeping capo-state reactor-free and tests deterministic).
  `SqliteStateStore` carries an `Arc<EventBroadcaster>` (shared across clones;
  manual `Debug`/`Eq`/`PartialEq` keep path-identity and exclude the runtime
  side-channel, so all existing derives' semantics hold). The append path
  publishes the committed `EventRecord` *after* `transaction.commit()`
  (`append_event` and `decide_permission_approval`), so no event is fanned out
  ahead of its durable watermark. An idempotent no-op append publishes nothing.
- Typed `ServerCommand::Subscribe { session_id: Option<String>, from_sequence }`
  plus `ServerResponsePayload::Subscribed(SubscriptionBacklog)` and a
  `ServerEvent` wire shape added in `crates/capo-server/src/types.rs`, with
  JSON-RPC encode/decode in `transport/codec.rs` (+`required_i64` in
  `transport/wire.rs`). Live events ride the ST2 server-initiated notification:
  `EventNotification::for_event` / `decode_event` and `EVENT_TAIL_METHOD`
  ("event") in `transport.rs`; `Subscribe` is classified read-only
  (no second writer). The broadcast carries discrete committed events keyed by
  sequence -- never a re-serialized read-model snapshot.
- Backlog-to-live seam: `CapoServer::subscribe` (`lib.rs`) subscribes to the
  broadcast *before* snapshotting the backlog and seeds the live `EventStream`
  (`crates/capo-server/src/event_tail.rs`) watermark from the backlog's
  `next_sequence`; `EventStream::next_batch` drops any live event with
  `sequence <= delivered_through`, so the seam has no gap and no duplicate.
- Deterministic tests (scripted commands / no live provider):
  capo-state `events_after_returns_only_events_strictly_after_the_watermark_in_order`
  and `committed_events_fan_out_to_live_subscribers_after_append`
  (`crates/capo-state/src/tests.rs`); capo-server
  `subscribe_backlog_returns_only_events_after_the_watermark_in_order`,
  `event_tail_has_no_gap_and_no_duplicate_across_the_backlog_to_live_seam`,
  `session_scoped_subscribe_tails_only_the_named_session`,
  `subscribe_command_and_subscribed_payload_round_trip_on_the_wire`, and
  `live_event_notification_frame_round_trips`
  (`crates/capo-server/src/tests/event_tail.rs`).
- Objective gate run from `/Users/nicolas/devel/capo-wt/streaming-transport`:
  `cargo fmt --check` ok (exit 0); `cargo clippy --all-targets --all-features --
  -D warnings` ok (exit 0); `cargo test --workspace` ok (exit 0; 0 failures
  workspace-wide -- capo-state 41 passed, capo-server 72 passed/1 ignored,
  capo-cli `server_transport` 11 passed, all other crates green). Focused
  `cargo test -p capo-server --lib event_tail`: 5 passed, 0 failed.

## ST5 - Multi-Turn Thread Read Model Projected From Events

Status: done.

Acceptance:

- Add a thread read-model projection that reconstructs an ordered multi-turn
  conversation (turns, incremental output items, tool observations,
  `TurnFinished`) from events, replacing `latest_summary` polling
  (`crates/capo-web/src/main.rs:258`, dashboard mapping).
- The thread is a projected read model, never client-owned state; clients render
  it and never author thread ordering.
- The projection rebuilds identically from the event log on restart/replay
  (matches the rebuildable-read-models rule in
  `workpads/architecture/state-model.md`).
- Per-turn items key by `turn_id` so distinct turns do not collapse onto one
  another (depends on `real-turn-loop` per-turn artifact keying).
- Add a query command to read a session's thread incrementally by sequence,
  composable with `Subscribe`.

Verification:

- Projection rebuild test asserting a scripted multi-turn event sequence rebuilds
  to an identical thread.
- Idempotency test for duplicate/replayed turn events.
- `cargo fmt` and focused `cargo test -p capo-state -p capo-server`.

Evidence:

- Gate fix only (no behavior change): in
  `crates/capo-state/src/thread.rs`, collapsed the `turns.last_mut().expect(...)`
  method chain back onto one line so `cargo fmt --check` is clean, and collapsed
  the nested `if let Some(text) = ... { if !text.is_empty() { ... } }` into a
  single `if let ... && !text.is_empty()` let-chain to satisfy clippy's
  `collapsible_if` under `-D warnings`. The projection logic, item/turn keying,
  and `item_text` field-priority semantics are unchanged.
- Objective gate run from `/Users/nicolas/devel/capo-wt/streaming-transport`:
  `cargo fmt --check` ok (exit 0); `cargo clippy --all-targets --all-features --
  -D warnings` ok (exit 0); `cargo test --workspace` ok (exit 0; 0 failures
  workspace-wide -- capo-state 46 passed including the 4 `thread::tests`
  projection/rebuild/idempotency/incremental-read cases, capo-server 72
  passed/1 ignored, capo-cli `server_transport` 11 passed, all other crates
  green).

## ST6 - Typed Mid-Turn Interrupt Wired To Ctrl-C

Status: pending.

Acceptance:

- Add a typed mid-turn interrupt that travels over the persistent connection and
  aborts the live generation/run for a session, distinct from `StopAgent`
  (`types.rs:124`) which is a coarser stop.
- Wire the CLI Ctrl-C to emit the interrupt frame on the open connection rather
  than killing the client process.
- The interrupt drives the runtime process-group kill (ST1) so descendants are
  reaped, and emits a typed interrupt/turn-aborted event that the thread
  projection renders.
- Add an orphan-after-cancel reaping assertion at the transport level: an
  interrupted turn leaves no surviving runtime process group.

Verification:

- Deterministic test that an interrupt frame aborts an in-flight scripted turn
  and emits the typed abort event.
- Orphan-after-cancel reaping test (paired with ST1).
- `cargo fmt` and focused `cargo test -p capo-server -p capo-runtime`.

## ST7 - Redaction-On-Emit On The Broadcast/SSE Egress Path

Status: pending.

Acceptance:

- Apply the existing `RedactionState` guard (`crates/capo-state/src/projections.rs:386-400`,
  used by the artifact-safety checks) to the broadcast/SSE emit path so no frame
  leaves the process unredacted.
- Redaction runs before any frame is written to a JSON-RPC notification or an SSE
  `Event`, at the egress point, not only at the tool/ACI boundary.
- Frames carrying artifacts or output classified `ContainsSensitive`/`Unknown`
  are withheld or replaced with a redacted reference, never streamed raw.
- Reuse `capo-runtime`'s credential-shape scanning where output redaction is
  needed, rather than a new bespoke scanner.

Verification:

- Deterministic test asserting a known secret seeded into an event never appears
  on the broadcast/SSE wire.
- Deterministic test asserting an `Unknown`/`ContainsSensitive`-classified
  artifact frame is withheld or referenced, not emitted raw.
- `cargo fmt` and focused `cargo test -p capo-server -p capo-state`.

## ST8 - Evolve capo-web From Poll-SSE To Broadcast Event Tail

Status: pending.

Acceptance:

- Git-track `crates/capo-web` (currently a live workspace member but untracked:
  `Cargo.toml` members include `crates/capo-web`, `git ls-files` returns
  nothing).
- Replace the 1500ms Dashboard-poll SSE (`crates/capo-web/src/main.rs:150-165`,
  `IntervalStream(1500ms).then(run(Dashboard{...}))`) with the broadcast-backed
  event tail from ST4 keyed by `from_sequence`; delete the poll path entirely.
- Reuse the existing typed integration (`ServerRequest`,
  `ServerResponsePayload`, `ServerCommand`, `main.rs:27-30`) and the in-process
  `CapoServer` rather than introducing a parallel facade.
- Resolve the `CapoServer` `!Send`-across-await constraint
  (`main.rs:10-12,89-101`, currently worked around via per-request
  `spawn_blocking`) so a long-lived subscriber can hold a tail without
  re-opening the server per tick; either make the server `Send` or wrap it in an
  actor/channel handle.
- Keep serving the frozen front-end static assets (`web/app/dist`) unchanged;
  do not modify front-end source.
- Apply redaction-on-emit (ST7) on the `capo-web` SSE path.

Verification:

- Deterministic test asserting `/api/events` emits broadcast-tail SSE frames (not
  Dashboard snapshots) for a scripted event sequence.
- Assertion that the poll path is removed (no `IntervalStream`/`Dashboard`
  poll in the events handler).
- `cargo fmt` and focused `cargo test -p capo-web`.

Must not do:

- Do not build a second HTTP/SSE bridge; evolve `capo-web` in place.
- Do not edit `web/app` or `web/dashboard` source.

## ST9 - Publish The JSON-RPC/SSE Schema And Checked-In Wire Snapshots

Status: pending.

Acceptance:

- Publish a language-neutral schema for the JSON-RPC methods, the notification
  variant, and the SSE event-tail frames as the authoritative web contract.
- Check in wire snapshots (request, response, notification, and SSE frame
  samples) generated from real serialization, verified without any web client.
- The schema and snapshots are the source of truth for the contract; TypeScript
  types are an optional downstream convenience generated FROM the schema, not the
  contract itself.
- Snapshot tests fail on an unintended wire-shape change so the contract cannot
  drift silently.
- Document the schema decision in `knowledge.md` (JSON Schema vs IDL vs
  hand-authored typed definitions checked by snapshot).

Verification:

- Snapshot tests over the checked-in wire fixtures (regenerate-and-diff).
- `cargo fmt` and focused `cargo test -p capo-server`.
- `git diff --check`.

## ST10 - Cross-Team Handoff For The Web Agent

Status: pending.

Acceptance:

- Provide a documented migration path so the web agent can switch its front-end
  from Dashboard-polling to `Subscribe`-based event tailing against the published
  schema (ST9).
- Provide a fixture/example (recorded event sequence plus expected SSE frames)
  the web agent can develop against without a live provider.
- Track web-side adoption as a web-agent task, not work this workpad performs;
  state that `capo-web` server-side delivery (ST8) is the contract vehicle and
  the web front-end is frozen here.
- Record the seam explicitly: this workpad verifies the contract via server-side
  wire-snapshot tests; the web agent verifies front-end rendering.

Verification:

- The handoff doc and fixture exist and reference the ST9 schema and snapshots.
- Server-side wire-snapshot test referenced by the handoff passes.
- `git diff --check`.

## ST11 - Deterministic Stream/SSE Wire-Snapshot Tests And Incremental CLI

Status: pending.

Acceptance:

- Add deterministic stream tests driven by a fake/scripted agent (no live
  provider) that exercise: subscribe-with-backlog, live tail, in-band cancel,
  mid-turn interrupt, and redaction-on-emit.
- Add SSE wire-snapshot tests asserting the `capo-web` event-tail frames match
  checked-in fixtures for a scripted event sequence.
- Convert the CLI from one-shot request/response rendering to incremental
  updates over the persistent connection, rendering the thread read model (ST5)
  as it streams.
- Restart/replay test proving the thread projection and subscriber resume
  identically from a `from_sequence` after a server restart.
- Every manual step in this workpad is paired with one of these deterministic
  assertions; nothing completes on self-attestation alone.

Verification:

- Focused `cargo test` over `capo-server`, `capo-state`, `capo-runtime`,
  `capo-web`, and `capo-cli`.
- Restart/replay test passes.
- `cargo fmt` and `git diff --check`.

## ST12 - Live Opt-In Streaming Smoke Paired With Deterministic Assertions

Status: pending.

Acceptance:

- Add a live streaming smoke (eventsource/curl against `capo-web`, or a live CLI
  tail) behind an explicit opt-in env gate mirroring the existing
  `CAPO_SERVER_RUN_CODEX_LIVE` convention, separate from ordinary test runs.
- The live smoke streams a real turn's incremental output and a real
  `TurnFinished` event (depends on `real-turn-loop`).
- Capture smoke evidence with secrets stripped (redaction-on-emit, ST7, must
  hold on the live path too).
- Pair the live smoke with a deterministic assertion (wire snapshot or replay) so
  completion is never solely operator-attested.
- Add an E2E gate: a full scripted path covering subscribe, incremental tail,
  interrupt, redaction, restart-resume, and the `capo-web` SSE tail.

Verification:

- E2E deterministic gate test passes.
- Live smoke transcript attached with secrets stripped, paired with a wire
  snapshot or replay assertion.
- `cargo fmt`, focused `cargo test` for changed crates (widening to `cargo test`
  if transport/state behavior changes broadly), and `git diff --check`.

Must not do:

- Do not let a live smoke stand as the only evidence for any task.
