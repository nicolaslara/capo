# Streaming Transport Knowledge

## Objective

Capture the design decisions for turning Capo's synchronous,
one-frame-per-connection transport into a streaming multi-client surface, and
for delivering the server-side web contract a browser can consume. Concretely:
a tokio `capo-runtime` with incremental output and stdin, JSON-RPC 2.0 framing
with a server-initiated notification variant over a persistent bidirectional
connection, an `events_after(since_sequence)` query plus a broadcast channel and
a typed `Subscribe { session_id, from_sequence }` command that tails the
append-only event log, a multi-turn thread read model projected from events,
a typed mid-turn interrupt, redaction-on-emit, and the evolution of the existing
`crates/capo-web` bridge from 1500ms Dashboard polling to a broadcast-backed
event tail. The authoritative deliverable is the JSON-RPC/SSE contract, verified
without a web client.

## Scope Decision

This is a Phase 2 workpad ("Stream it") and depends on `real-turn-loop`. The
loop must already produce real incremental output and a real `TurnFinished`
event before there is anything meaningful to stream; this workpad makes those
events streamable to many clients and re-exposes them to browsers as a contract.

Streaming/transport belongs in a dedicated workpad, not inside `real-turn-loop`
or `operator-control`. `real-turn-loop` produces events; `operator-control` is a
client surface that renders read models and lowers commands. This workpad sits
between them: it makes the loop's events streamable and fans them out, without
owning orchestration state.

### Web Boundary (Critical)

This workpad evolves the Rust server bridge `crates/capo-web` and delivers the
JSON-RPC/SSE event-tail **contract**. It does **not** build the web client.

- `crates/capo-web` (the server-side Rust axum bridge) is IN scope to evolve.
  It already exists as an untracked workspace member: an axum + tokio facade
  over the typed `ServerCommand`/`ServerResponsePayload` boundary exposing
  `/api/dashboard`, `/api/commands`, and `/api/events`
  (`crates/capo-web/src/main.rs`). It must be git-tracked and its poll-SSE path
  replaced.
- `web/app` and `web/dashboard` front-end source are frozen and OUT of scope.
  Another agent owns them. This workpad never edits them; it keeps serving the
  built static assets (`web/app/dist`) unchanged.
- The authoritative contract is a language-neutral JSON-RPC/SSE schema plus
  checked-in wire snapshots, verifiable WITHOUT any web client. TypeScript types
  are an optional downstream convenience generated FROM the schema, owned by the
  web agent, and are not the contract itself.

### Migration Handoff

The current `capo-web` `/api/events` handler is a 1500ms `IntervalStream` that
re-runs `ServerCommand::Dashboard` and re-serializes the full read model as one
SSE event every tick (`crates/capo-web/src/main.rs`, the `events` handler). This
is the exact `latest_summary`-poll antipattern this workpad removes: it never
streams discrete committed events, it re-serializes a whole snapshot on a timer,
and it cannot resume from a sequence watermark.

The handoff so the web agent can switch its front-end from Dashboard-polling to
`Subscribe`-based tailing:

- The server delivers the contract; the web agent adopts it. This workpad
  publishes the JSON-RPC/SSE schema + checked-in wire snapshots (the contract
  vehicle) and a recorded fixture (event sequence plus expected SSE frames) the
  web agent can develop against without a live provider.
- The seam is explicit: this workpad verifies the contract with server-side
  wire-snapshot tests; the web agent verifies front-end rendering. Web-side
  adoption is tracked as a web-agent task, not work performed here.

## Wire Protocol: JSON-RPC 2.0 With A Notification Variant

JSON-RPC 2.0 over a persistent bidirectional connection is the wire protocol;
SSE re-exposes the same typed commands/events for browsers. This replaces the
custom one-line-in/one-line-out newline-JSON codec
(`crates/capo-server/src/transport.rs`,
`crates/capo-server/src/transport/codec.rs`), which reads exactly one bounded
line and writes exactly one line per connection.

Decisions:

- A request/response pair carries a JSON-RPC `id`; the existing
  `request_id`/origin propagation continues to flow through to the handler so
  command-identity idempotency is preserved.
- A **notification** variant (no `id`, server-initiated) lets the server push
  events to a connected client without a prior request. This is what carries the
  event tail and live turn output.
- Every existing typed `ServerCommand` (`crates/capo-server/src/types.rs`) maps
  onto a JSON-RPC method without changing domain command semantics. The wire
  format is the codec layer below the AgentAdapter boundary; it does not become
  the domain model.
- The loopback-only listener constraint already enforced in `serve_tcp`
  (`crates/capo-server/src/transport.rs`) is preserved, as is the bounded-frame
  protection (`MAX_TRANSPORT_FRAME_BYTES`).

## Event Tail: events_after + Broadcast + Subscribe

The event log stays authoritative. Subscription tails it; it never re-serializes
a full read model per interval.

Decisions:

- Add `events_after(since_sequence)` to the state query surface
  (`crates/capo-state/src/queries.rs`) returning events strictly after a
  caller-supplied sequence watermark, in order. The store already exposes a
  monotonic `sequence` (`last_insert_rowid()` in `append_event`) and a `default`
  projection watermark (`crates/capo-state/src/lib.rs`), so the watermark
  semantics already exist; this surfaces a forward read of them.
- Add a broadcast channel fed by the append path so newly committed events fan
  out to all live subscribers without polling.
- Add a typed `Subscribe { session_id, from_sequence }` `ServerCommand` variant
  and a corresponding streaming/notification variant in `ServerResponsePayload`
  (today neither exists: `ServerResponsePayload` has `Dashboard`, `DispatchRun`,
  etc., but no `Subscribe` and no notification).
- A subscriber first receives the catch-up backlog via
  `events_after(from_sequence)`, then live events from the broadcast channel,
  with no gap and no duplicate at the seam. The broadcast carries discrete
  committed events keyed by sequence, never a re-serialized snapshot.

## Concurrent Accept Loop, Timeouts, And In-Band Cancel

The current accept loop is serial and single-threaded: `serve_tcp` accepts one
connection, calls `handle_stream`, and loops
(`crates/capo-server/src/transport.rs`). A streaming multi-client surface needs
concurrency.

Decisions:

- Replace the serial accept loop with a concurrent loop, task-per-connection,
  handling multiple persistent connections.
- Add per-connection read/idle timeouts so a stalled or abandoned client does
  not hold resources indefinitely.
- Add an in-band typed `Cancel` frame a client can send on its open connection
  to abort an in-flight request/turn, distinct from closing the socket.
- Concurrency must not introduce a second writer to the event log. Server writes
  stay serialized behind the existing handler; concurrent writers are
  unsupported until the `safety-gates` single-writer workspace lock lands. This
  is documented as a known constraint, not silently interleaved.

## tokio Runtime: Incremental Output And stdin

`crates/capo-runtime` is synchronous today: a local process is spawned, run to
exit, then its stdout/stderr are buffered and capped after exit
(`spawn_process`/`capped_output` in `crates/capo-runtime/src/lib.rs`). There is
no stdin path, so the controller cannot talk to a process mid-flight.

Decisions:

- Move the runner to tokio so a live run emits stdout/stderr incrementally
  instead of buffer-then-cap-after-exit.
- Expose a way to write to a run's stdin so the controller can talk to a process
  mid-flight.
- Preserve provable descendant-process reaping: keep the process-group
  setup/kill escalation (`process_group(0)` and `terminate_process_group`,
  which sends `-TERM` to the process group in `crates/capo-runtime/src/lib.rs`)
  and port the existing process-group kill regression test to the tokio runner
  unchanged in intent. Add a new orphan-after-cancel reaping test: a cancelled
  run leaves no surviving process group.
- Fix the output-cap-discards-success classification. Today `capped_output`
  returns `Err(OutputLimitExceeded)` and the run discards artifacts on overflow,
  so a long **successful** run is misclassified as an error. Under streaming,
  output is streamed-and-truncated with truncation recorded as artifact
  metadata; a successful run that exceeds the cap is NOT classified as failed.
- Keep `RemoteProcessRunner` and the `Fake` runner shapes intact so deterministic
  tests do not require a tokio reactor where they did not before.

## Multi-Turn Thread Read Model

Today the dashboard and `capo-web` render `latest_summary` (a single
`Option<String>` on `SessionSummary` in `crates/capo-server/src/types.rs`),
polled per interval. A daily-driver chat surface needs an ordered multi-turn
thread.

Decisions:

- Add a thread read-model projection that reconstructs an ordered multi-turn
  conversation (turns, incremental output items, tool observations,
  `TurnFinished`) from events, replacing `latest_summary` polling.
- The thread is a projected read model, never client-owned state. Clients render
  it; they never author thread ordering.
- The projection rebuilds identically from the event log on restart/replay,
  matching the rebuildable-read-models rule in
  `workpads/architecture/state-model.md`.
- Per-turn items key by `turn_id` so distinct turns do not collapse (depends on
  `real-turn-loop` per-turn artifact keying that fixes the single-`stdout.txt`
  overwrite).
- Add a query command to read a session's thread incrementally by sequence,
  composable with `Subscribe`.

## Typed Mid-Turn Interrupt

Decisions:

- Add a typed mid-turn interrupt that travels over the persistent connection and
  aborts the live generation/run for a session, distinct from the coarser
  `StopAgent` (`crates/capo-server/src/types.rs`).
- Wire the CLI Ctrl-C to emit the interrupt frame on the open connection rather
  than killing the client process.
- The interrupt drives the runtime process-group kill so descendants are reaped,
  and emits a typed interrupt/turn-aborted event the thread projection renders.
- An interrupted turn leaves no surviving runtime process group
  (orphan-after-cancel assertion at the transport level, paired with the runtime
  reaping test).

## Redaction-On-Emit

A multi-client stream is a new secret-egress surface, so redaction must guard
the egress point, not only the tool/ACI boundary.

Decisions:

- Apply the existing `RedactionState` guard
  (`crates/capo-state/src/projections.rs`, which already gates emission on
  `ContainsSensitive`/`Unknown`/`secret_derived`) to the broadcast/SSE emit path
  so no frame leaves the process unredacted.
- Redaction runs before any frame is written to a JSON-RPC notification or an SSE
  `Event`, at the egress point.
- Frames carrying artifacts/output classified `ContainsSensitive` or `Unknown`
  are withheld or replaced with a redacted reference, never streamed raw.
- Reuse `capo-runtime`'s existing credential-shape scanning where output
  redaction is needed, rather than a new bespoke scanner.

## Evolving capo-web And The CapoServer !Send Constraint

`capo-web` works around a `CapoServer` `!Send`-across-await constraint by opening
a fresh `CapoServer` per request inside `tokio::task::spawn_blocking` (its module
doc states the SQLite-backed query store and `CapoServer` are non-`Send` across
awaits; the `dashboard`, `commands`, and `events` handlers each
`spawn_blocking`). A long-lived subscriber cannot hold a tail this way because it
re-opens the server per tick.

Decisions:

- Git-track `crates/capo-web`, replace the 1500ms Dashboard-poll SSE with the
  broadcast-backed event tail keyed by `from_sequence`, and delete the poll path
  entirely. Reuse the existing typed integration (`ServerRequest`,
  `ServerResponsePayload`, `ServerCommand`) and the in-process `CapoServer`
  rather than introducing a parallel facade.
- Resolve the `!Send`-across-await constraint either by making the server `Send`
  or by wrapping it in an actor/channel handle, so a subscriber holds a tail
  without re-opening the server per tick. (This is the first open question.)
- Apply redaction-on-emit on the `capo-web` SSE path too.

## Authoritative Contract: Schema + Wire Snapshots

The authoritative web contract is a language-neutral JSON-RPC/SSE schema plus
checked-in wire snapshots (request, response, notification, and SSE-frame
samples) generated from real serialization and verified without any web client.
Snapshot tests fail on an unintended wire-shape change so the contract cannot
drift silently. The schema-representation choice (JSON Schema vs an IDL vs
hand-authored typed definitions checked by snapshot) is the second open
question; whichever is chosen, the checked-in snapshots remain the enforced
source of truth.

## Non-Goals

- Do not build any web UI. `web/app` and `web/dashboard` are owned by another
  agent and frozen here; this workpad never edits them.
- Do not stream by re-serializing a full read model per interval (the poll
  antipattern this workpad removes).
- Do not own orchestration state in the transport layer; the event log stays
  authoritative and read models rebuild from it.
- Do not define permission/verification semantics: permission-card content and
  enforcement and verification progress semantics belong to `safety-gates`; the
  stream only carries them. Goal projections belong to `goal-autonomy`.
- Do not own or adopt TypeScript types in the front end; TS types are an
  optional downstream artifact the web agent generates from the schema.
- Do not support concurrent writers: document that concurrent writers are
  rejected (not interleaved) until the `safety-gates` write lock lands.
- Do not let a live streaming smoke stand as the only evidence for any task;
  every manual smoke is paired with a deterministic assertion.

## Open Questions

- How to resolve the `CapoServer` `!Send`-across-await constraint: make the
  server `Send`, or wrap it in an actor/channel handle? `capo-web` currently
  works around it via per-request `spawn_blocking`, which cannot hold a
  long-lived tail.
- Does the JSON-RPC schema get published as JSON Schema, an IDL, or
  hand-authored typed definitions checked by snapshot tests?
- Where exactly is the broadcast channel fed from so a committed event is never
  fanned out before it is durably appended (avoiding a live event ahead of the
  watermark a reconnecting subscriber would later miss)?
- What is the per-connection idle/read timeout default, and how does it interact
  with a long-idle but legitimately open subscriber holding a tail?
