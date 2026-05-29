# Depth Knowledge

## Objective

Capture decisions for the depth workpad: harden and broaden the working harness
once the loop is real. The live ACP JSON-RPC adapter (incl. the live
`request_permission` round-trip and resume/load reconciliation), Claude as a
second workspace-write provider, a real `MarkdownMemoryBackend` + FTS5 retrieval
path that kills the hardcoded packet strings, a first OS sandbox tier, git
worktree isolation, and an optional OTel exporter. Each task carries its own true
prerequisite so breadth begins the moment that prerequisite lands.

## Scope Decision

Create the `depth` workpad as the LAST workpad in the sequence (Phase 6):

```text
real-turn-loop -> streaming-transport / tools-aci -> safety-gates -> goal-autonomy -> depth
```

These tasks DEEPEN a harness that already works rather than unblocking it. By the
time `depth` runs, the loop is real (`real-turn-loop`), the ACI is real
(`tools-aci`), the loop is gated and verifiable (`safety-gates`), and goals
continue safely (`goal-autonomy`). `depth` then takes the harness from
"works on one provider, one read-only-ish memory packet, no real isolation" to
"works on a second and third provider, with real retrieval memory, real OS
isolation, and optional observability."

Crucially, `depth` does NOT re-architect anything earlier workpads own. It does
not touch the turn loop, the transport protocol, the tool registry, the
permission engine, the goal model, or the web client. It adds breadth and
hardening at the seams those workpads already defined: below the `AgentAdapter`
trait, behind the `MemoryBackend` enum, behind the `RuntimeRunner` boundary, and
as additive observability over existing spans.

The defining structural decision is DIFFERENTIATED per-task prerequisites instead
of a blanket dependency on all earlier phases:

- Live ACP adapter (DP1-DP3), Claude (DP4), and FTS5 memory (DP5-DP6) depend only
  on `real-turn-loop` + `tools-aci`. ACP and memory hardening do not need
  autonomy or checkpoint/rollback.
- The OS sandbox tier (DP7) and git worktree isolation (DP8) depend on
  `safety-gates` (checkpoint/recovery + enforced `PermissionPolicy`), because a
  real isolation boundary must compose with grants, locks, and rollback.
- The worktree-PER-GOAL slice of DP8 additionally depends on `goal-autonomy`,
  because binding a worktree to a goal/attempt needs the goal model.
- OTel (DP9) is additive once its subject surfaces exist; live smokes (DP11) wait
  for their subject tasks.

This lets breadth start as soon as its true prerequisite lands without waiting for
the full earlier sequence to complete or re-opening earlier designs.

## Live ACP Adapter Design

ACP stays an adapter below the `AgentAdapter` trait, never the domain model.
`safety-gates` deliberately scoped its `request_permission` work to the
trait-level round-trip against FAKE/SCRIPTED adapters plus the existing option
mapping; the LIVE `session/request_permission` wire round-trip lands here in
`depth`, not in `safety-gates`. This avoids a hidden cross-phase dependency: a
live permission round-trip cannot exist before a live ACP wire client exists.

The adapter promotes today's fixture-only `AcpAdapter::session_setup_plan`
(capability planning over `ToolDefinition`s) into a real JSON-RPC 2.0 stdio
client driven by the loop. It implements `initialize` (negotiating integer
`protocolVersion`, stable `1` today), `session/new`, `session/prompt`,
`session/cancel`, and ingests `session/update` notifications, with the ACP process
launched through `RuntimeRunner` and attached after start. Client-side
`fs/read_text_file` / `fs/write_text_file` / `terminal/run` calls route through
the existing `wrapper_request_for_client_call` mapping into the `tools-aci`
runtime wrappers, advertising a capability only when its `ToolDefinition` + scope
exist.

Open question carried from the plan: does the live adapter wrap a generic
JSON-RPC ACP client or reuse the existing Codex/Claude connectors? The leaning is
a generic ACP client, because ACP is an interoperability boundary distinct from
the subscription-CLI connectors, and Capo should be able to drive any
ACP-compatible agent. This is recorded as an open question, not a closed
decision.

## ACP Replay And Dedupe Design

Capo never makes replayed ACP `session/update` notifications directly
authoritative for read models. The adapter follows `acp-replay-dedupe.md`:
persist every raw update before normalization, normalize through idempotent
mappers, and project read models only from Capo event sequences.

- `session/resume` is the DEFAULT reconnect path when the agent advertises
  `sessionCapabilities.resume` and Capo already has local history; it creates no
  message/item replay events.
- `session/load` is for foreign import, repair/reconciliation, or resume-less
  agents; it opens an `AcpReplayBatch`, stages candidates in a non-projecting
  workspace, finalizes on the load response, and reconciles.
- Tool calls dedupe cleanly by stable `toolCallId`; plans are full replacements;
  message chunks are the hard case because stable ACP v1 lacks message IDs, so
  Capo finalizes content hashes and records `message_boundary_confidence`.
- Capo restart recovery and ACP replay stay separate phases: recovery establishes
  local event truth first; ACP replay reconciles against it.

## Claude As The Second Write Adapter

Claude is breadth: a second real workspace-write provider that validates the
`AgentAdapter` trait against a second implementation. The Claude write adapter was
deliberately MOVED out of `real-turn-loop` (one real write provider, Codex, is
enough to make the loop real) into `depth`. Claude launches through
`RuntimeRunner` as `claude -p --output-format stream-json --verbose`, scrubs
unrelated `ANTHROPIC_API_KEY` / `ANTHROPIC_AUTH_TOKEN`, and rides the existing
live-provider preflight (Claude is already a supported preflight kind). Live
writes are gated behind an explicit opt-in env gate AND the `real-turn-loop`
safety floor; if native tool-result delivery is unsupported in the observed CLI,
results are recorded observed-only.

## Real Memory Retrieval: FTS5-First

Memory retrieval is FTS5-first; vector/embeddings/graph are deferred unless real
dogfood traces prove FTS insufficient (no vector DB for the first retrieval path).
The live packet today is built from four hardcoded strings that never touch the
store, the strict eligibility filter
(`packet_eligible_memory_records` / `is_packet_eligible`) is dead in production,
and there is zero retrieval/search. `depth` fixes all three:

- `MarkdownMemoryBackend` supplies workpad/source pointers with content hashes
  (non-destructive indexer, human truth stays in markdown).
- `SqliteFtsMemoryBackend` implements `search(MemoryQuery, MemoryBudget)` over
  FTS5 with ranking, filtering out invalidated/rejected/superseded/secret/
  unauthorized/redacted records.
- The eligibility filter is wired into packet candidate selection so the live
  turn-context packet derives from retrieved, filtered sources, not literals,
  while preserving the existing per-item inclusion reasons and excluded-reason
  decisions and keeping the packet artifact replayable.
- `MemoryJob`s (`extract_facts` / `index_fts` / `invalidate` / `rebuild`) index
  the working repo and detect staleness via source-hash drift; extracted facts
  stay `generated` (untrusted) until promoted, and secrets/credentials/raw voice
  transcripts are never valid memory sources.

## Runtime Boundary: Sandbox And Worktrees Are Swappable

Sandbox and worktrees live behind the `RuntimeRunner` boundary and are swappable,
exactly as `runtime-tunnel.md` keeps remote runners and tunnels in the type model
without forcing them into the first prototype. Capo never claims hard sandboxing
unless the selected runtime actually enforces it through OS mechanisms and tests
prove it; today Capo delegates to the provider CLI's own `--sandbox` flag and has
only path-prefix checks.

- The first OS sandbox tier enforces real filesystem/network confinement via
  macOS seatbelt (dev box) and linux landlock+bwrap (CI), modeled after the codex
  `sandboxing` crate. It composes with the `real-turn-loop` path confinement and
  pre-write checkpoint as an additional enforcement layer.
- Git worktree isolation runs a session's write run in a dedicated worktree rather
  than the operator's live tree, composing with the `safety-gates` single-writer
  lock and checkpoint/rollback. The worktree-per-goal slice binds a worktree to a
  goal/attempt (gated on `goal-autonomy`) without changing the goal model.

Open question carried from the plan: which sandbox tier gates first, macOS
seatbelt or linux landlock+bwrap? The likely answer is seatbelt on the dev box
for fast iteration and landlock+bwrap for CI enforcement; recorded as an open
question pending the implementation environment.

## Optional Observability: OTel Off By Default

OTel is optional and OFF by default. No spans leave the process unless explicitly
enabled. It is additive observability over the existing spans across the loop,
tools, and runtime, modeled after the codex `otel` crate. It adds real wall-clock
timing alongside the existing event-sequence-delta duration in `capo-eval`, and
span attributes pass the existing redaction guard before export. Disabling OTel
changes nothing about event-sourced truth or read models; spans are observability,
not state.

## Verification Discipline

Deterministic-tests-before-live-providers holds across every task: ACP transcript
replay, FTS5 retrieval, and sandbox refusal-mode fixtures must pass with no live
provider and no real OS network before any live work. Every manual smoke is paired
with a deterministic assertion (wire snapshot, exit status, or restart/replay), so
nothing completes on operator self-attestation. Live ACP/Claude and real-sandbox
smokes stay behind explicit opt-in env gates mirroring `CAPO_SERVER_RUN_CODEX_LIVE`
/ `CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT`, with secrets stripped from all evidence.

## Non-Goals

- No web client (the web agent owns that surface).
- Do not change the goal model, transport protocol, or permission engine here;
  earlier workpads own those.
- Do not require a vector DB, embeddings, or graph memory for the first retrieval
  path.
- Do not make ACP `session/update` notifications directly authoritative for read
  models, and do not expose Capo as an ACP agent backend.
- Do not claim sandboxing on a platform where Capo cannot enforce it.
- Keep OTel optional and off by default, and never let telemetry carry authoritative state (the event log remains the source of truth). Using the exporter crate itself is encouraged.

## Open Questions

- Does the live ACP adapter reuse the existing CodexExec/ClaudeCode connectors or
  wrap a generic JSON-RPC ACP client? (Leaning: a generic ACP client, since ACP is
  an interoperability boundary distinct from the subscription connectors.)
- Is the first sandbox tier macOS seatbelt (dev box) or linux landlock+bwrap (CI),
  and which gates first?
- How aggressively should FTS5 retrieval rank/snippet before vector search is
  justified, and what dogfood signal would prove FTS insufficient?
- When a worktree-per-goal is bound, what is the merge-back/review point before a
  child worktree can satisfy a parent goal requirement?
- Should OTel spans ever be retained as artifacts, or only exported live to an
  external collector?
