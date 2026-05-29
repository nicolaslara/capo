# Capo Daily-Driver Readiness Review — Lead Synthesis

**Question:** Are Capo's design decisions conducive to something that could become a daily-driver coding-agent harness (used in place of Claude Code / Codex / OpenCode / Cursor / Cline)? If not, what should change?

## Verdict: NO (not yet a daily driver; the foundations are promising)

Capo has unusually disciplined bones — a typed, audited command boundary; a genuinely real event-sourced SQLite state core; a real permission decision engine; an honest runtime; and the best-articulated autonomy design among all peers reviewed. But the loop that actually makes a coding-agent harness usable every day is **not built**. Weighing what is implemented over what is designed:

- **Chat today is a scripted fake.** The default "chat" routes SteerAgent into `FakeBoundaryController::redirect`, which writes a canned `latest_summary`. The controller's own doc-comment says it is "intentionally fake-only." Confirmed in `crates/capo-controller/src/lib.rs:1-5` and `session_control.rs`.
- **No streaming anywhere.** The transport reads one line and writes one line per connection, on a serial single-threaded accept loop (`crates/capo-server/src/transport.rs:28-32, 79-106`). There is no notification variant, no SSE/WS, no tokio/axum in any crate (verified: zero matches).
- **The only live path is read-only and one-shot.** Codex runs `--sandbox read-only --ephemeral`, parsed after exit; Claude is forced into no-tools plan mode. No agent can complete a multi-step edit through Capo.
- **No goal continuation, no real verification.** Zero `Goal`/`continuation`/`scheduler`/`auditor` symbols in code (verified). Smoke `--status passed` is taken on faith; no `cargo test`/`clippy`/`Command::new` runs any verification in `capo-eval`/`adapter_smoke.rs` (verified).
- **The web UI is 100% fixtures.** No HTTP/SSE bridge exists in any Rust crate; the CLI is the only real client.

The design docs are thorough and frequently ahead of peers conceptually, but thoroughness of design is not capability. On the dimensions that define daily use, Capo sits below every named peer.

## Scorecard (post-verification adjusted scores, /5)

| Dimension | Score | One-line |
|---|---|---|
| Server transport & multi-client surface | 1.5 | Clean typed command boundary + working loopback-TCP CLI client, but synchronous serial one-frame-per-connection; no streaming/push/fan-out. |
| Interactive chat / steering model (core loop) | 1.0 | Default chat is a scripted fake echo; only live path is a blocking one-shot Codex read; no streaming, threads, interrupt, or branching. |
| State model, event log, recovery & checkpoint/rollback | 2.5 | Genuinely real event-sourced SQLite store with idempotency + rebuild + recovery bookkeeping; but no workspace checkpoint/rollback, blunt recovery. |
| Protocol & provider adapters (ACP, Codex, Claude) | 1.5 | Thoughtful boundary, mostly fixtures; read-only one-shot Codex only; ACP fixture-only; Claude no-tools; no streaming/resume/tool round-trip. |
| Permission policy & capability model | 2.0 | Real scope engine + path containment + durable grant store with ACP mapping; but not wired into the loop, grants write-only, no revoke/sandbox. |
| Goal lifecycle & continuation loop | 1.0 | Best-articulated design among peers, but essentially zero implementation; controller is an admitted fake event synthesizer. |
| Project memory & context assembly | 1.5 | Real non-destructive indexer + strict SQL eligibility filter, but the live packet is 4 hardcoded strings; zero retrieval/search. |
| Runtime supervision, sandboxing & connectivity | 1.5 | Honest, real process-group-aware local runner wired into two live paths; but synchronous, no stdin/stream, no real sandbox/tunnel, recovery unwired. |
| Evaluation / outcome scoring & observability | 1.5 | Auditable event log captures tool decisions, but the only artifact is a descriptive roll-up; all verification is operator-asserted; no timing/OTel. |

**Weighted read:** the two most daily-driver-central dimensions (chat loop, transport streaming) are the lowest-scoring (1.0–1.5), which is why the overall verdict is *no* rather than *not-yet* despite a solid 2.5 on state.

## Top Blockers (ordered by impact)

1. **Default chat loop is a fake-adapter echo; no real model-driven turn loop.** *(Chat / steering)* — Build a genuine controller turn loop (observe normalized events -> update projections -> emit TurnFinished) on a real non-fake adapter, replacing the scripted send_task/redirect path. This is the substrate everything else attaches to.
2. **No streaming or server-push; serial one-frame-per-connection transport.** *(Transport)* — Move to a persistent bidirectional connection (JSON-RPC 2.0) with a notification variant and a `Subscribe { session_id, from_sequence }` that tails the event log via a broadcast channel; make the accept loop concurrent with timeouts and in-band Cancel.
3. **No working agent can actually edit code end-to-end.** *(Adapters)* — Extract the designed adapter contract into a real trait, implement it for Codex with a workspace-write profile and live tool-result round-trip, then lift Claude out of no-tools plan mode.
4. **Permission engine + durable grants exist but are inert.** *(Permissions)* — Wire PermissionPolicy/ToolExposure into the real loop, handle ACP `session/request_permission`, add grant read-back and revoke/expiry. The hard part is built; it just isn't called.
5. **No verification loop and no computed outcome score.** *(Evaluation)* — Build a VerificationRunner that runs project test/lint commands and emits evidence with real exit-status pass/fail; implement `score_run` to compare acceptance criteria to that evidence; add wall-clock timing.
6. **No goal continuation loop or evidence-gated completion auditor in code.** *(Goal lifecycle)* — After the real loop lands, implement the goal model, the safe-boundary scheduler, and the evidence-gated auditor (agents propose completion, never assert it).
7. **No web UI connected to the real server; both surfaces are fixtures.** *(Web UX)* — Add an axum HTTP/SSE adapter reusing existing ServerRequest/ServerResponse types, generate shared TS types, build a real streaming chat console before more screens.

## Phased Roadmap (respects the boundary model: server/controller owns the loop, ACP is an adapter, CLI/dashboard are clients)

### Phase 0 — Make the loop real (unblocks everything)
- Replace/augment FakeBoundaryController with a real observe->decide->emit turn loop behind the existing boundary.
- Extract the adapter contract into a real Rust trait; implement it for Codex first as a working workspace-write, tool-result-round-trip adapter.
- Persist per-turn artifacts keyed by turn_id (fix the stdout.txt overwrite bug).

### Phase 1 — Stream it (the interactive daily-driver loop)
- Rewrite capo-runtime on tokio: incremental output streaming + stdin; keep process-group kill escalation.
- JSON-RPC 2.0 framing with a notification variant, persistent bidirectional connection, concurrent accept loop, timeouts, in-band Cancel.
- `events_after(since_sequence)` + broadcast channel + `Subscribe` command; convert dashboard/CLI to incremental updates.
- First-class multi-turn thread read model projected from events; render the thread, not latest_summary.
- Typed mid-turn interrupt wired to Ctrl-C.

### Phase 2 — Make it safe (gate the loop, verify outcomes, enable rollback)
- Wire PermissionPolicy + ToolExposure into the real loop; handle ACP `request_permission`; inline permission cards over the stream.
- Grant read-back in `decide` + created_at/expires_at/revoked_at + revoke/expire events + revoke CLI; fix TrustedLocal critical-scope exclusion.
- VerificationRunner that actually runs check/lint/test and emits real pass/fail evidence.
- Workspace checkpoint/rollback as controller-owned shadow-git (checkpoint.created/restored + Restore command).
- Liveness-aware restart recovery (persist start_requested, probe health, run.recovered/orphaned/exited, reattach).

### Phase 3 — Make it autonomous (Capo's differentiator)
- Goal domain model + EventKinds/projections with idempotency + rebuild test.
- Safe-boundary continuation scheduler as a pure state machine, opt-in.
- Evidence-gated completion auditor as the only path to goal-complete (agents propose, never assert).
- Reattach-after-compaction: re-inject objective + audit contract on restart.

### Phase 4 — Deepen memory, providers, and the web app
- Wire the real memory packet path (kill the 4 hardcoded strings); MarkdownMemoryBackend + FTS5; extraction + staleness MemoryJob; index the user's repo.
- Live ACP JSON-RPC adapter (initialize/session.new/prompt/update/cancel/request_permission) + resume/load.
- axum HTTP/SSE bridge + generated TS types + real streaming chat/screens in web/app; freeze web/dashboard.
- First OS sandbox tier (seatbelt/landlock+bwrap) + git worktree isolation + optional OTel exporter.

## Strengths Worth Preserving
- **Event-sourced state core** (append-only events, tested idempotency, in-transaction projections, replayable rebuild, watermarks) — genuinely real and on par with codex/OpenHands. The right foundation for streaming, recovery, and autonomy.
- **Typed, audited command boundary** with origin propagation and command-identity idempotency — exactly the multi-surface discipline a harness needs.
- **Real permission decision engine** with scope matching, filesystem path containment, and a durable SQLite grant store with correct ACP option mapping — well ahead of a pure UI-prompt model; just inert.
- **Honest, real runtime** with provable descendant-process reaping and credential scanning, and explicit refusal to overclaim sandboxing.
- **Best-articulated autonomy design among all peers**: server-owned outer loop, safe-boundary continuation, no-progress guard, reattach-after-compaction, and completion gated by an external evidence ledger rather than model confidence — a genuine conceptual edge over codex /goal. Preserve and build it.
- **Provenance-first memory model** with source-hash drift detection and a strict SQL eligibility filter.
- **Disciplined web component craft** (real Cmd-K palette, persisted theme, clean shell) ready to pay off once a live data path exists.

## Peer Comparison
On the daily-driver essentials Capo is behind every named peer: ACP, codex app-server, opencode, Claude Code, Codex CLI, Cursor, and Cline all ship live streaming, multi-turn threads, mid-turn interruption, and inline approvals that users depend on today; Capo ships a fake echo plus a blocking one-shot Codex read and latest_summary polling. On safety, codex (seatbelt/landlock+seccomp+execpolicy) and OpenHands (containers) enforce real isolation and cline/cursor ship checkpoint/rollback; Capo has path-prefix checks only and no rollback. On verification, the incumbents and SWE-bench discipline run real tests as the score; Capo's verification is operator-asserted. Capo reaches **parity** on its event-sourced state model and conceptually on permission/ACP-option mapping. Capo is uniquely **ahead** only in the *design* of an evidence-ledger-gated completion auditor — which is unbuilt. Net: a well-architected prototype whose design ambitions exceed every peer while its shipped capability sits below all of them on daily-driver basics.

## Per-Dimension Detail

### Server transport & multi-client surface — 1.5
Clean typed `ServerCommand` boundary and a working loopback-TCP CLI client, but `handle_stream` is one-line-in/one-line-out per connection on a serial single-threaded accept loop; no notification variant, no streaming, no fan-out, no concurrency. The web client is fixture-backed; the CLI is the only real client. Custom newline-JSON codec with no published schema/SDK. Loopback-only (intentional). Verified: no tokio/axum/broadcast/notification in any crate.

### Interactive chat / steering model — 1.0 (most verdict-critical)
Default chat = `SteerAgent` -> `FakeBoundaryController::redirect` writing a canned `latest_summary` (verified in session_control.rs). Only live path is a blocking one-shot `codex exec` parsed after exit, reusing one stdout.txt that overwrites prior turns. No token streaming, no multi-turn thread (only latest_summary polling), no mid-turn interruption of a real generation, no inline permission/question round-trip in the loop, no branching/undo. The rich chat UI exists only as fixtures. Note: the CLI permission code is real (correcting an overstated gap), but there is no live mid-turn round-trip.

### State model, event log, recovery & checkpoint/rollback — 2.5 (the strongest dimension)
Genuinely real: single append-only events table, idempotency-key dedupe (tested), ~30 in-transaction projections, replayable projection_records, watermarks, full rebuild, 54 event kinds, redaction guard, and tested restart recovery bookkeeping. Missing: workspace checkpoint/rollback (no shadow-git, designed-only), liveness-aware recovery (recovery bluntly marks all live-looking runs exited_unknown), event subscription/tail, ACP raw-update reconciliation, and projection failure isolation. Solid bones, not yet safe for higher autonomy.

### Protocol & provider adapters — 1.5
Real but thin: only Fake/ScriptedMock are dispatchable; CodexExec/ClaudeCode/Acp are unit structs exposing fixture parsers. The one live path is a read-only one-shot Codex spawn; Claude is forced into no-tools plan mode; ACP is fixture-only (no live JSON-RPC). Strong: thoughtful JSONL normalizers with timeline/idempotency keys, and genuinely-built subscription-credential hygiene. Note (correcting overstatement): `acp_client.rs` does real capability planning, and the AgentAdapter enum does expose send_turn/attach for the fake variants — the accurate gap is that the real adapters aren't wired into dispatch.

### Permission policy & capability model — 2.0
Real allow/deny scope engine (StaticPolicy), scoped tool registry, true filesystem path containment, and a durable SQLite approval/grant store with correct ACP allow_once/always/reject mapping and allow_always down-scoping. But the live controller is FakeBoundaryController; session_control never consults the policy; the ACP server ignores request_permission; grants are write-only (never read back to authorize); no revoke/expiry; TrustedLocal default is allow-all; no OS sandbox or per-command shell policy. Built machinery that is currently inert.

### Goal lifecycle & continuation loop — 1.0
The best-articulated design among peers (evidence-gated completion auditor, event-driven safe-boundary scheduler, no-progress guard, reattach-after-compaction) — and essentially nothing is implemented (verified: zero goal/continuation/scheduler/auditor symbols). The controller is an admitted fake event synthesizer with hardcoded confidence values. Daily-driver readiness here is near-zero, but the design is a real asset to build against.

### Project memory & context assembly — 1.5
Two genuinely real pieces: a non-destructive markdown indexer with source-hash drift detection, and a strict SQL packet-eligibility filter. But the live context packet is built from 4 hardcoded strings that never touch the store, the eligibility filter is dead in production, and there is zero retrieval/search (no FTS/embedding/ranking). Token "budget" is a literal int sum. The only real ingestion path is the voice summary.

### Runtime supervision, sandboxing & connectivity — 1.5
Honest and real where it counts: a process-group-aware local runner (provably reaps descendants), env scrubbing, credential scanning, wired into two live paths. But it is fully synchronous, non-streaming, has no stdin (can't talk to a run mid-flight), no real sandbox of its own (delegates to the CLI's flags), RemoteProcessRunner and tunnels are stubs, output is buffer-then-cap (a long successful run is discarded as an error), and the detailed restart-recovery is designed + unit-tested but not wired (the controller uses RuntimeRunner::fake()).

### Evaluation / outcome scoring & observability — 1.5
Auditable event log genuinely captures tool decisions/outputs, but the only eval artifact is a descriptive markdown roll-up whose "duration" is an event-sequence delta. Every verification signal (smoke/test/review) is operator-asserted (verified: no test/lint/build runner exists), and the dogfood readiness gate keys off self-attestation — exactly the "automated score overstates completion" failure the design warns about. No computed score, no wall-clock timing, no OTel.

