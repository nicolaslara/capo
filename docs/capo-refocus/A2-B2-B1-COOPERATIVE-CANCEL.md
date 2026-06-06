# A2 (shipped) / B2 (cooperative cancel, landed) / B1 (deferred)

Status as of this change set. All changes are additive and default-off: with no
cancel flag installed, the live ACP turn frames are byte-identical to the
pre-change validated loop (`conductor_live_e2e` and the deterministic suites are
unaffected).

## A2 — detached fan-out: SHIPPED (no code change required)

`crates/capo-server/src/acp_mcp_http.rs` `tool_start_agent` already supports
`detached: true`: it clones the `CapoServer`, spawns the worker turn on a
`std::thread`, and returns `{"status":"running","detached":true}` immediately so
the conductor (L1) stays non-blocking while workers (L2) run. Detached-thread
errors are surfaced via `eprintln!` (an honest "this session is not actually
still running" signal) rather than being swallowed. The conductor system prompt
(`crates/capo-web/src/main.rs`) already instructs detached fan-out +
`collect_results`. Verified by reading; no functional change was made.

### Deferred A2 enhancement: terminal `turn_failed` event on detached error

The optional improvement — appending a terminal `turn_failed` event in the
detached error branch so `review_agent`/`list_agents` don't show a dead session
as forever `running` — was DEFERRED. The detached branch already calls
`server.handle(...)`; appending a typed terminal event would require auditing the
event-recording surface to find a single clean append, and the plan's rule is to
not risk the validated loop for a log-quality improvement. The existing
`eprintln!` visibility (and the in-code comment noting the richer fix) stays.

## B2 — cooperative cancel: LANDED

A shared `Arc<AtomicBool>` cancel flag is threaded from the server's in-flight
registry down to the wire pump, which checks it BETWEEN inbound frames.

### Wire layer — `crates/capo-adapters/src/acp_wire.rs`
- New `AcpWireError::Cancelled { awaiting }` variant (+ `Display`). Only ever
  produced when a cancel flag was installed.
- `AcpWireClient` gains an `Option<Arc<AtomicBool>>` `cancel` field (defaults to
  `None` in `attach`) and a `with_cancel(flag)` builder.
- `pump_until_response` checks the flag at the top of its loop (before the next
  blocking read). When set, it sends a best-effort `session/cancel` NOTIFICATION
  (if the external session id is known) and returns `Cancelled`. When the field
  is `None`, the whole block is skipped → byte-identical frames.

### Adapter layer — `crates/capo-adapters/src/acp_live.rs`
- `drive_with_decider` gains a trailing `cancel: Option<Arc<AtomicBool>>`. When
  `Some`, it is installed onto the wire client. The `Cancelled` sentinel from
  `prompt` is mapped to a TERMINAL transcript with `stop_reason = "cancelled"`
  and `cancelled = true` (matching the existing cancelled status mapping), NOT an
  error, so the controller ingests a clean cancelled turn. `drive` and the
  trait-level `run_turn` path pass `None`.

### Controller layer — `crates/capo-controller/src/acp_live_dispatch.rs`
- `drive_acp_live_turn` gains a trailing `cancel: Option<Arc<AtomicBool>>`,
  forwarded into `drive_with_decider`. All deterministic test callers pass `None`.

### Server layer — `crates/capo-server/src/lib.rs` + `server_core.rs`
- New `InFlightTurn { cancel: Arc<AtomicBool> }` handle and an
  `in_flight: Arc<Mutex<HashMap<String, InFlightTurn>>>` field on `CapoServer`,
  keyed by Capo `session_id`. Because it is an `Arc<Mutex<..>>` field on the
  `#[derive(Clone)]` `CapoServer`, it is shared across every clone (including the
  one moved into the detached worker thread) with no `static` and no cross-test
  bleed.
- Helper trio: `register_in_flight`, `deregister_in_flight`, `cancel_session`
  (returns `true` iff a live turn was found+flagged).
- `run_acp_live_turn_local` and `run_conductor_turn_local` register the turn's
  cancel flag, install an RAII `DeregGuard` (deregisters on normal return,
  `?`-error, or panic), and thread `Some(flag)` into `drive_acp_live_turn`.
- `InterruptAgent` / `StopAgent` handlers call `cancel_session(session_id)` IN
  ADDITION to their existing durable `interrupt_command` / `stop_command` record.
  If a live turn is registered the flag flips (the live win); if not,
  `cancel_session` returns `false` and behavior is byte-identical to the prior
  record-intent-only path (never fakes delivery). The response and recorded event
  are NOT branched on the result, so deterministic-suite assertions stay stable.

### Tests
- Deterministic registry unit tests:
  `crates/capo-server/src/tests/cancel_registry.rs` (register/deregister/cancel,
  shared-across-clones, stale-flag-cleared, no-op-when-absent).
- Deterministic wire tests (scripted transport, no live process):
  `installed_cancel_flag_sends_session_cancel_and_returns_cancelled` and
  `no_cancel_flag_never_cancels_and_sends_no_cancel_frame` in `acp_wire.rs`.
- Gated live e2e: `crates/capo-server/tests/cancel_live_e2e.rs`
  (`#[ignore]`, gated on `CAPO_E2E_LIVE_CANCEL=1` + the live ACP env gate).

### Documented limitation: cancel granularity

The flag is observed BETWEEN recv frames (or at the per-read `read_timeout`
deadline). A worker wedged inside a single blocking `recv_line_within` will not
observe the cancel until the next frame arrives or that deadline elapses. This is
honest and acceptable, not a defect.

## B1 — steer / mid-turn injection: DEFERRED (lifecycle blocker)

`SteerAgent` stays honest record-intent (no code change). ACP is one prompt per
turn — `drive_with_decider` sends exactly one `session/prompt` then pumps to the
terminal response — so there is no supported mid-turn `session/prompt` injection,
and faking it would violate the never-fake-delivery rule. The only honest live
option, enqueuing a follow-up turn, is blocked by the current lifecycle:
`take_transport` (`server_core.rs`) consumes the transport and
`LiveAcpSession::finalize` (`acp_live.rs`) drops it and tears down the process
group at end of turn, leaving no persistent session to enqueue onto. A
follow-up-turn path would need session-persistence (relaxing `finalize`) or a
`session/resume` reconnect plus an enqueue-and-drive scheduler — neither is small
and both risk the validated single-turn loop. Deferred pending that lifecycle
work.

## Verification (real output at landing)

- `cargo build --workspace --tests` — clean.
- `cargo test -p capo-adapters -p capo-controller -p capo-server` — green
  (adapters 82 passed / 2 ignored; controller 201 passed / 4 ignored; server lib
  163 passed / 6 ignored; all integration binaries green). Includes the 5 new
  registry tests and the 2 new wire cancel tests.
- `cargo clippy -p capo-adapters -p capo-controller -p capo-server --tests` —
  clean (no warnings).
- Live tests NOT run (gated).
