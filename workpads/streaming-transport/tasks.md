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

Status: done.

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

Evidence:

- The typed mid-turn interrupt plumbing was already in place across
  `crates/capo-server/src/transport.rs` (the `interrupt` JSON-RPC notification
  method, `Frame::Interrupt`, `CancellationToken::interrupt`/`interrupt_reason`,
  `RequestHandler::interrupt`, the `interrupted` error frame, and the
  `interrupt_frame`/`send_interrupt` client seam), `transport/wire.rs`
  (`TransportError::Interrupted` -> `error.data.kind=interrupted`), and
  `crates/capo-server/src/lib.rs` (`CapoServer::interrupt_session`, recording the
  `session.interrupted` turn-aborted event through the same single-writer
  `interrupt_command` point). The gate failure was that the ST6 *verification
  test* was missing, so its helpers (`jsonrpc_interrupt_frame`,
  `ScriptedHandler::interrupts_for_test`, `InterruptLog::entries`) were dead code
  (clippy `-D warnings`), and two long lines in `lib.rs`/`tests/transport.rs`
  tripped `cargo fmt --check`.
- Fix: added the deterministic ST6 transport test
  `in_band_interrupt_aborts_in_flight_turn_and_emits_typed_abort_event`
  (`crates/capo-server/src/tests/transport.rs`, scripted handler, no live
  provider). It holds a live turn (`turn-*`) in flight, sends the typed
  `interrupt` notification on the same open connection, and asserts: a typed
  `error.data.kind=interrupted` frame naming the session + reason (distinct from
  the ST3 `cancelled` kind); `RequestHandler::interrupt` fired once with
  `(session-turn, operator ctrl-c)` so the `session.interrupted` event the thread
  projection renders is recorded; the connection stays open (a follow-up request
  succeeds). The orphan-after-cancel reaping is asserted at the transport level
  via a new `TurnStopObserver`: the in-flight turn observes the interrupt
  *reason* on its `CancellationToken` (`interrupt_reason()`), which is the signal
  a real turn handler drives the runtime process-group kill with -- paired with
  the ST1 runtime test
  `cancel_terminates_descendant_process_group`
  (`crates/capo-runtime/src/async_runner.rs`) that proves no surviving group. Ran
  `cargo fmt` to wrap the two over-long lines.
- Objective gate run from `/Users/nicolas/devel/capo-wt/streaming-transport`:
  `cargo fmt --check` ok (exit 0); `cargo clippy --all-targets --all-features --
  -D warnings` ok (exit 0; the three dead-code errors are gone); `cargo test
  --workspace` ok (0 failures workspace-wide -- capo-server 75 passed/1 ignored
  including the new ST6 case, capo-cli `server_transport` 11 passed, capo-runtime
  38 passed including `cancel_terminates_descendant_process_group`, all other
  crates green). Focused `cargo test -p capo-server --lib transport`: 12 passed,
  0 failed.

## ST7 - Redaction-On-Emit On The Broadcast/SSE Egress Path

Status: done.

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

Evidence:

- Delta path (the ST1 review finding): the tokio runner already redacts each
  `runtime.output_delta` BEFORE `tx.send` rather than the old raw
  `String::from_utf8_lossy(chunk)`. `send_redacted_delta`
  (`crates/capo-runtime/src/async_runner.rs`) applies the run's `RedactionPolicy`
  to every emitted prefix; `take_emittable_prefix` holds the trailing partial
  token across read boundaries so a secret split across a chunk is still scanned
  whole, and the per-chunk redaction is carried into the event detail. The
  durable artifact is independently re-redacted at `wait()` time and its
  `redaction_state` is propagated onto `process.redaction_state`. Covered by
  `streamed_deltas_and_artifact_redact_an_unnamed_secret` and
  `secret_split_across_a_chunk_boundary_is_still_redacted` (a child prints a
  known secret -> the cleartext is in NO delta detail, the artifact is redacted
  with `redaction_state = "redacted"`).
- Broadcast/Subscribe egress guard (ST4 egress path): the `RedactionState` guard
  now runs at the single egress funnel `ServerEvent::from_record`
  (`crates/capo-server/src/types.rs`), which both the catch-up backlog
  (`CapoServer::read_subscription_backlog`) and every live broadcast
  notification (`EventStream::next_batch` ->
  `EventNotification::for_event`) build their events through, so no frame leaves
  the process before redaction. `ServerEvent::redacted_for_egress` has two
  layers: (1) an event whose stored `redaction_state` is not persistable-safe
  (`ContainsSensitive` / `Unknown` / unrecognized, via new
  `RedactionState::from_wire` in `crates/capo-state/src/event.rs`) has its raw
  body WITHHELD and replaced with a redacted reference
  (`WITHHELD_PAYLOAD_PLACEHOLDER` + event id + original classification), egress
  state downgraded to `redacted` -- the frame still crosses the boundary (no
  gap) but never the sensitive content; (2) a safe/redacted-labeled event has
  `capo_runtime::RedactionPolicy`'s credential-shape scan run over its payload as
  a backstop, so a secret that slipped into a `safe`-labeled body is scrubbed
  before egress and the state is upgraded to `redacted`. This reuses the
  runner's scanner rather than a bespoke one.
- Tests (`crates/capo-server/src/tests/event_tail.rs`, deterministic, secrets
  seeded directly into the server's own store via a `#[cfg(test)] state_for_test`
  seam so the live broadcast subscriber sees them):
  `subscribe_backlog_withholds_sensitive_event_bodies_and_never_emits_a_secret_raw`
  (a `ContainsSensitive`/`Unknown` event with a raw AWS-key secret is withheld --
  the secret cleartext is on NO backlog wire frame, the body is the withheld
  reference, egress state `redacted`) and
  `live_event_tail_redacts_a_credential_in_a_safe_labeled_payload_before_the_wire`
  (a mislabeled-`safe` event carrying a `ghp_...` token travels the live
  broadcast tail; the actual `to_wire_frame()` JSON-RPC notification never
  contains the secret, the credential placeholder is present, and egress state is
  upgraded to `redacted`).
- Objective gate run from `/Users/nicolas/devel/capo-wt/streaming-transport`:
  `cargo fmt --check` ok; `cargo clippy --all-targets --all-features -- -D
  warnings` ok; `cargo test --workspace` ok (0 failures workspace-wide).

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

Status: done.

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

Evidence:

- The contract is published from one in-code source,
  `crates/capo-server/src/transport/contract.rs` (re-exported as
  `capo_server::contract`): `wire_samples()` builds every frame through the
  SAME `jsonrpc`/`codec`/`EventNotification` path the live transport uses (never
  hand-typed JSON), `contract_schema()` is the language-neutral schema, and
  `sse_frame()` is the canonical SSE `event:`/`data:` block whose data line is
  the verbatim JSON-RPC `event` notification (so SSE and the raw socket carry one
  wire shape; `capo-web`/ST8 is not in this worktree, so the SSE shape is pinned
  here at the contract level and ST8 reuses `contract::sse_frame`).
- Checked-in artifacts under `crates/capo-server/contract/`:
  `jsonrpc-schema.json` (described schema), `snapshots/*.json` (11 enforced wire
  frames -- requests `list_agents`/`subscribe`/`read_thread`, responses
  `agents`/`subscribed`, the `cancelled` error frame pinning
  `error.code=-32603` + `error.data.kind`, the server `event` tail and client
  `cancel`/`interrupt` notifications, and the `sse-event-tail` block),
  `capo-wire.d.ts` (the OPTIONAL downstream TypeScript types, explicitly
  generated FROM the schema and owned web-side -- not the contract), and
  `README.md` (the cross-team handoff + regeneration workflow).
- Schema-representation decision recorded in `knowledge.md`
  ("Schema-representation decision", resolving the second open question):
  hand-authored JSON-Schema-shaped definitions checked by snapshot, with the
  real serialized snapshots as the enforced source of truth (rejecting derived
  JSON Schema and an IDL, with reasons). The snapshots cannot drift silently:
  `tests::contract::wire_snapshots_match_the_checked_in_contract` and
  `jsonrpc_schema_matches_the_checked_in_contract` assert byte-equality against
  the checked-in files (regenerate-and-diff via `CAPO_REGENERATE_WIRE_SNAPSHOTS=1`;
  the default run only reads and asserts). The described schema cannot lag the
  code either: `schema_enumerations_cover_every_wire_variant` uses
  compile-enforced exhaustive `match`es over every
  `ServerCommand`/`ServerResponsePayload`/`ServerError` variant, so a new variant
  is a build error until its wire tag is added to the published schema.
  `every_snapshot_frame_is_valid_json_rpc_2_0` and
  `schema_method_and_notification_names_are_covered_by_snapshots` keep the two
  halves consistent. All verified WITHOUT any web client (frames built from typed
  values through the production codec).
- Objective gate run from `/Users/nicolas/devel/capo-wt/streaming-transport`:
  `cargo fmt --check` ok; `cargo clippy --all-targets --all-features -- -D
  warnings` ok; `cargo test --workspace` ok (0 failures workspace-wide;
  capo-server 82 passed/1 ignored including the 5 new `tests::contract` cases).
  `git diff --check` clean.

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

Status: done. (The `capo-web` SSE wire-snapshot bullet is delivered as a
contract-level SSE-sequence fixture because `capo-web` is not a member of this
worktree -- ST8 owns git-tracking `capo-web`; the SSE shape for a scripted event
sequence is pinned here at the published-contract level and ST8 reuses
`contract::sse_event_sequence`/`sse_frame`.)

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

Evidence:

- Live server-pushed event tail wired into the persistent connection: a
  `Subscribe` now puts the connection into tailing mode
  (`crates/capo-server/src/transport.rs` `start_event_tail` + `TailHandle`),
  writing the catch-up `Subscribed` backlog as the response and then pumping live
  `event` notifications on the same socket through a detached pump that blocks on
  the broadcast with a bounded poll (`EventStream::recv_batch` /
  `TailRecvError` in `crates/capo-server/src/event_tail.rs`). The write half is an
  `Arc<Mutex<TcpStream>>` so the pump and the main loop never interleave bytes; a
  `cancel` against a live tail (no request in flight) ends the tail with a typed
  `cancelled` frame, and an `interrupt` while tailing still records the
  turn-aborted event. `RequestHandler` gained `subscribe`/`supports_subscription`
  (default off; `CapoServerHandler` delegates to `CapoServer::subscribe`, no
  second writer).
- Client seam: `subscribe_tcp` + `SubscribeStream`
  (`crates/capo-server/src/transport.rs`, re-exported from `lib.rs`) open one
  persistent connection, read the typed `Subscribed` backlog, then yield each live
  committed event (`next_event` / `next_event_frame`, the latter returning the
  verbatim wire bytes for snapshot assertions).
- Deterministic stream wire-snapshot tests (fake/scripted agent, no live
  provider) in `crates/capo-server/src/tests/stream.rs`:
  `scripted_turn_streams_exact_backlog_and_live_event_frames` asserts the EXACT
  JSON-RPC `event` notification wire bytes for a scripted turn;
  `subscribe_tail_has_no_gap_and_no_duplicate_at_the_backlog_to_live_seam_over_the_wire`;
  `in_band_cancel_on_the_subscribe_connection_ends_the_tail` (typed `cancelled`
  frame); `mid_turn_interrupt_on_the_subscribe_connection_records_the_typed_abort`;
  `live_tail_withholds_a_sensitive_event_body_and_never_emits_a_secret_raw`
  (redaction-on-emit over the live wire);
  `live_tail_streams_real_committed_events_from_write_bearing_commands` (the real
  `CapoServerHandler` path); and the restart/replay
  `thread_projection_and_subscriber_resume_identically_after_a_server_restart`
  (the thread projection rebuilds byte-identically and a `from_sequence`
  subscriber replays identically after reopening the store on the same root).
- SSE wire-snapshot: `contract::sse_event_sequence` builds the canonical SSE
  byte stream for a scripted three-event turn from `contract::sse_frame`, checked
  in at `crates/capo-server/contract/snapshots/sse-event-sequence.txt` and
  enforced by `tests::contract::sse_event_sequence_matches_the_checked_in_fixture`
  (regenerate-and-diff via `CAPO_REGENERATE_WIRE_SNAPSHOTS=1`). `capo-web` is not a
  member of this worktree, so this is the published-contract vehicle the web agent
  (ST10) develops against; ST8 reuses it when it git-tracks `capo-web`.
- Incremental CLI: `crates/capo-cli/src/operator_control.rs` `thread` no longer
  one-shots `ReadThread { from_sequence: 0 }`. It drives the sequence watermark
  contract -- `subscribe_tcp` from the session's last-rendered watermark, folds the
  catch-up backlog and any live events (`drain_live_thread_events`), renders the
  projected multi-turn read model plus an incremental tail of new events
  (`render_thread_tail`), and advances a per-session `thread_watermarks` entry so a
  repeated `thread` shows only what is new since the last read. Covered by CLI
  integration tests in `crates/capo-cli/tests/server_transport/stream.rs`
  (`control_thread_renders_incrementally_via_subscribe_over_the_persistent_connection`
  and `control_thread_streams_new_events_committed_between_reads`, driving a real
  server process).
- Objective gate run from `/Users/nicolas/devel/capo-wt/streaming-transport`:
  `cargo fmt --check` ok (exit 0); `cargo clippy --all-targets --all-features --
  -D warnings` ok (exit 0); `cargo test --workspace` ok (0 failures workspace-wide
  -- capo-server 90 passed/1 ignored including the 7 new `stream` cases and the new
  contract SSE-sequence case, capo-cli `server_transport` 13 passed including the 2
  new CLI thread-stream cases, capo-runtime 38, capo-state 47, all other crates
  green). `capo-web` is not a member of this worktree, so its focused `cargo test`
  is N/A here (owned by ST8).

## ST12 - Live Opt-In Streaming Smoke Paired With Deterministic Assertions

Status: done.

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

Evidence:

- New `crates/capo-server/src/tests/e2e_gate.rs` (module registered in
  `crates/capo-server/src/tests.rs`) delivers both halves of ST12 sharing ONE
  shape-assertion helper (`assert_streamed_turn_shape`), so the live smoke is a
  true pairing and never operator-attested:
  - `streaming_e2e_gate_covers_the_full_contract` -- the always-on deterministic
    E2E gate, run over the REAL production transport (`serve_tcp` +
    `send_tcp` + `subscribe_tcp` on a loopback listener, bounded accept loop sized
    to the exact connection count). It opens a live `subscribe_tcp` tail BEFORE
    the turn; drives ONE real turn through the production write path
    (`ServerCommand::ReplayAdapterFixture` over the wire, the codex-exec fixture
    -> `session.summary_updated` incremental output, observed `exec_command`
    tool round-trip, and a terminal `evidence.recorded` = `TurnFinished`); observes
    the deltas LIVE over the socket with no gap/dup/reorder (strictly increasing
    sequence) and every frame a well-formed JSON-RPC `event` notification; asserts
    the projected multi-turn thread read model renders the streamed turn
    (`ReadThread`); proves redaction-on-emit on the real wire egress (a seeded
    `ContainsSensitive` event's body is WITHHELD with the
    `[REDACTED:withheld]` placeholder and downgraded to `redacted` in a
    reconnecting subscriber's backlog -- the secret cleartext never crosses the
    socket); fires a typed mid-turn `send_interrupt` over the persistent connection
    and asserts the thread projects an `interrupted` turn (the durable
    `session.interrupted` event); proves restart-resume (reopening the store on the
    same root rebuilds the thread projection byte-identically and a `from_sequence`
    subscriber replays the same events strictly after the watermark); and pins the
    `capo-web` SSE re-exposure shape via `contract::sse_frame` over the same
    committed events (the SSE `data:` line is the verbatim JSON-RPC `event`
    notification, and redaction holds on the SSE path too).
  - `live_streaming_smoke` -- `#[ignore]`d AND gated behind the explicit opt-in env
    var `CAPO_SERVER_RUN_STREAMING_LIVE` (mirroring the `CAPO_SERVER_RUN_CODEX_LIVE`
    convention; it also skips cleanly when unset, so it never fails for non-opted-in
    runs). It connects a live socket client (`subscribe_tcp`) to the loopback
    JSON-RPC server, `Subscribe`-s a session, drives ONE real turn, observes the
    live incremental deltas + terminal `TurnFinished` over the wire, prints a
    secrets-stripped transcript an operator can attach, and asserts the IDENTICAL
    shape via `assert_streamed_turn_shape` (secrets-stripped over the live wire).
    Run with `CAPO_SERVER_RUN_STREAMING_LIVE=1 cargo test -p capo-server --
    --ignored live_streaming_smoke`; verified passing opted-in (1 passed) with the
    live transcript emitted, and ignored in ordinary runs.
- Objective gate run from `/Users/nicolas/devel/capo-wt/streaming-transport`:
  `cargo fmt --check` ok; `cargo clippy --all-targets --all-features -- -D warnings`
  ok; `cargo test --workspace` ok (0 failures workspace-wide; capo-server includes
  the new always-on `streaming_e2e_gate_covers_the_full_contract` case and the
  `#[ignore]`d `live_streaming_smoke`). `git diff --check` clean.
