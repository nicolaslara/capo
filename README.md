# Capo

Capo is a local-first controller and harness for coding-agent sessions. It is a
durable server/control plane that owns the orchestration loop for tracked coding
agents, plus clients (a CLI/operator REPL and a web console) that submit commands
and render read models. Capo's job is to make agent work observable, steerable,
resumable, and auditable: it owns the turn loop, the event-sourced state, the
permission and verification gates, and the project-memory context it injects into
prompts — while concrete providers (Codex, Claude, ACP) live below an adapter
boundary.

## Status (honest)

The daily-driver harness is **implemented and tested, but not yet dogfooded.** The
controller turn loop is real (a single `run_dispatch_turn` production path that
drives the dispatch primitives), the `AgentAdapter` trait seam is in place, real
Codex chat is live-proven end to end behind explicit gates, the tool/ACI layer is
wired into the loop, permission/verification/checkpoint gates enforce, goal
autonomy (continuation + evidence-gated completion auditor) exists, and the web
console streams live chat from real server state over the event tail. Roughly
**127K lines of Rust across 14 `capo-*` crates with ~790 tests, all green on
`main`.**

It is **not** a polished daily driver yet. Capo has not been turned on itself for
its own work, several real subsystems are built but not yet the default wiring
(see "What's missing"), only Codex is live (others are gated), memory is
FTS5-only, there is no remote runtime or tunnel, and the agent-built code in this
tree is pending a human audit. Treat it as a working harness under active
construction, not a finished product.

## Architecture

Capo is a boundary-model controller. Clients submit commands and render read
models; the server/controller owns the orchestration loop and authoritative
state; every external system (provider, runtime, tool) sits behind an adapter
boundary. The event log is the source of truth, and all read models rebuild from
it.

```text
  Clients (CLI / operator REPL, web console)
      |  typed ServerCommand / ServerResponse  (JSON-RPC 2.0 + SSE event tail)
      v
  CapoServer  ──►  CapoController              owns the turn loop
      |                run_dispatch_turn        observe → decide → emit
      |                (single production path; drives the dispatch
      |                 plan → gate/preflight → run primitives, then
      |                 annotates the run with TurnFinished)
      |
      +── PermissionPolicy / ToolExposure       gates in the decide step
      +── VerificationRunner / score_run         verdict over observed evidence
      +── checkpoint / rollback (shadow-git)
      +── MemoryBackend (markdown + SQLite FTS5)  → sourced context packet
      |
      v
  AgentAdapter trait  (provider-neutral seam)
      |
      +── CodexLiveAdapter        (live, gated)
      +── Claude adapter          (gated)
      +── AcpAdapter              (live JSON-RPC client + fixtures)
      +── Fake / ScriptedMock     (deterministic tests / fallback)
      |
      v
  RuntimeRunner                 only LocalProcessRunner today (+ Fake)
      |  spawn / stdin / process-group kill / orphan reaping
      v
  Event-sourced SQLite  (capo.sqlite + file artifacts)
      append-only event log  ──►  read models (projections, rebuildable)
```

Key invariants: there is exactly one orchestration path (the loop drives dispatch
rather than running beside it); no provider runs without passing the existing
gate/preflight; the controller routing is a single typed switch
(`ControllerSelection`, default `Real`) so the real handle is a zero-cost view
over the same orchestration core; and Capo owns context selection — providers do
not reach memory directly.

### Crate map

| Crate | Role |
| --- | --- |
| `capo-core` | Shared IDs, primitives, and common types. |
| `capo-state` | Event-sourced SQLite store, event kinds, projections, restart recovery. |
| `capo-controller` | The turn loop: observe → decide → emit, permission/verification/score_run/checkpoint gates, real and fake boundary controllers. |
| `capo-server` | Typed `ServerCommand`/`ServerResponse` boundary, dispatch state machine, live-provider preflight, JSON-RPC/SSE transport + wire contract. |
| `capo-adapters` | `AgentAdapter` trait and impls: Codex (live), Claude, ACP, Fake/ScriptedMock; provider-neutral permission round-trip types. |
| `capo-runtime` | `LocalProcessRunner` (tokio): spawn, incremental output, stdin, process-group kill, orphan reaping; OS sandbox tiers. |
| `capo-tools` | Tool registry + runtime wrappers (edit/patch/search/test, file/git/shell), typed I/O, provenance, redaction, `PermissionPolicy`, path containment. |
| `capo-memory` | Markdown + SQLite FTS5 memory backend, extraction/staleness jobs, sourced context-packet assembly. |
| `capo-query` | Read-model query surface used by the CLI dashboard and web facade. |
| `capo-eval` | Evidence/evaluation roll-ups and reporting over the event log. |
| `capo-voice` | Voice-transcript command intake (transitional surface). |
| `capo-workpads` | Markdown project-memory indexing (`project memory` + transitional `workpad` commands). |
| `capo-web` | axum/tokio HTTP+SSE facade over an in-process `CapoServer` (`/api/dashboard`, `/api/commands`, `/api/thread`, `/api/events`). |
| `capo-cli` | The `capo` binary: command surface, operator control REPL, server client. |

## What it can do

**Server + transport.** Durable local server (`capo server serve`, loopback
`127.0.0.1:7878`), JSON-RPC 2.0 framing with a server-initiated notification
variant, an `events_after(since_sequence)` + broadcast `Subscribe` event tail that
tails the append-only log, a projected multi-turn thread read model, and a typed
mid-turn interrupt. The wire contract is checked in and snapshot-verified under
`crates/capo-server/contract/`.

**Real turn loop.** One production path (`run_dispatch_turn`) that drives
plan → gate/preflight → run and annotates the run with a `TurnFinished` derived
from the same normalized batch — no second run-completion model. A
provider-neutral `AgentAdapter` trait sits below the loop.

**Providers.** Codex live and proven end to end (read-only one-shot today),
Claude gated, a live ACP JSON-RPC adapter plus fixture replay, and
Fake/ScriptedMock for deterministic tests.

**Tools / ACI.** Typed edit/patch/search/test tools with narrow validated output
and structured retryable errors, file/git/shell wrappers, provenance and
input-and-output redaction, and the agent-reporting/evidence tools tagged
`source=agent_reported` (claims, never proof).

**Permissions / capabilities.** `PermissionPolicy` enforced in the loop's decide
step, durable grants with read-back / revoke / expiry, the `TrustedLocal`
critical-scope denial fix (no blanket allow on writes outside the workspace,
network egress, secret read, arbitrary shell), and the ACP permission
option-mapping round-trip.

**Verification.** A real `VerificationRunner` that executes configured
check/lint/test commands and keys pass/fail off true exit status, plus `score_run`
computed over observed evidence only (agent claims never raise the score).

**Checkpoint / rollback.** Controller-owned per-turn shadow-git checkpoints under
the state root (the workspace's own `.git` is never touched) with a one-command
restore, so a real write is reversible.

**Liveness recovery.** Crash-safe in-flight runs (start-requested + pid persisted
before spawn), orphan-process-group reaping, and liveness-aware restart recovery
classifying runs as recovered / orphaned / exited.

**Goal autonomy.** A durable goal/requirement/evidence model, an opt-in
safe-boundary continuation scheduler, an evidence-gated completion auditor (the
only path to goal-complete — agents propose, they never assert), no-progress
suppression, and reattach-after-compaction from persisted goal state.

**Runtime isolation.** OS sandbox tiers (macOS seatbelt enforced on dev; Linux
landlock+bwrap as the CI tier), git worktree isolation, per-run resource ceilings
(max turns / wall-clock / token-cost), environment scrubbing, and process-group
orphan reaping.

**Memory.** Markdown-backed records indexed into `capo.sqlite` (rows point at repo
`.md` via `source_path` + content hash) with FTS5 retrieval, extraction/staleness
jobs, a sourced context packet Capo injects into the turn prompt, and a governed
`capo.project_memory_read` tool — Capo owns context selection, providers don't
reach memory directly.

**Observability.** The event log itself, plus an optional OpenTelemetry exporter
(off by default; spans never carry authoritative state).

**Clients.** A CLI / operator REPL with planner modes (`--planner none`
deterministic, `--planner capo` a tracked Codex-backed operator agent), and the
web operator console streaming live chat over the event tail.

## How to use it

All commands run from the repository root. Use a scratch state dir while
experimenting:

```sh
export CAPO_STATE=.capo-dev/readme-demo
```

### Build & verify

```sh
cargo build -p capo-cli --bin capo
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace
```

CI enforces the deterministic subset of that gate — `cargo fmt --check`, `cargo
clippy --all-targets --all-features -D warnings`, `cargo test --workspace`, `git
diff --check`, and a dashboard static smoke — with `TMPDIR=/tmp`. Live-provider
smokes stay out of unattended CI behind their opt-in env gates.

### Run the server

Terminal 1 starts the local server (loopback `127.0.0.1:7878` by default):

```sh
cargo run -p capo-cli --bin capo -- server serve
```

Terminal 2 talks to it automatically when it is running:

```sh
cargo run -p capo-cli --bin capo -- server agent register --name demo
cargo run -p capo-cli --bin capo -- server task send \
  --agent demo --goal "Inspect the project and summarize the current state"
cargo run -p capo-cli --bin capo -- server dashboard
```

### Operator control loop

Bare `capo` enters the control loop (aliases to `capo control --planner none`),
starting a loopback server if one is not already running:

```sh
cargo run -p capo-cli --bin capo --
```

After `attach`, ordinary text is sent to the attached agent; `status`, `result`,
`details`, `tools`, `detach`, and `quit` stay commands. It scripts over stdin:

```sh
printf '%s\n' \
  'agents' \
  'attach demo' \
  'Please report current status and wait for the next instruction' \
  'result' 'status' 'tools' 'evidence' 'reviews' 'details' 'dashboard' \
  'quit' \
  | cargo run -p capo-cli --bin capo --
```

`--planner capo` runs a tracked Codex-backed operator agent that maps free-form
intent into validated server-backed actions and audits its choices as a
`capo-operator` session:

```sh
printf '%s\n' \
  'what are my agents doing?' "what's up?" 'what is blocked?' \
  'summarize demo' 'tell demo to Please summarize the latest state' \
  'recent capo-operator' 'quit' \
  | cargo run -p capo-cli --bin capo -- control --planner capo
```

### Real Codex chat (behind live gates)

Live Codex is per-agent and gated; it never becomes a global default. Bind the
Codex chat adapter and drive a turn with both live-provider gates set:

```sh
CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT=1 CAPO_SERVER_RUN_CODEX_LIVE=1 \
  cargo run -p capo-cli --bin capo -- server agent register \
    --name codex-demo --adapter codex

CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT=1 CAPO_SERVER_RUN_CODEX_LIVE=1 \
  cargo run -p capo-cli --bin capo -- server task send \
    --agent codex-demo --goal "Say CAPO_REAL_CODEX_OK and nothing else"
```

From the control REPL, `new codex <name> <goal>` does the same with both gates
enabled. Without the gates the live path fails closed rather than faking output.
Claude live execution is intentionally still blocked.

### Web console

`capo-web` runs an in-process `CapoServer`, so the server and the HTTP/SSE facade
start in one step. Build the front-end once, then serve it:

```sh
cd web/app && bun install && bun run build      # produces web/app/dist
CAPO_STATE_ROOT=.capo-dev cargo run -p capo-web  # http://127.0.0.1:4177
```

The console auto-detects the live facade (`GET /api/dashboard`): with it present
it runs **live** (streaming the agent reply over `/api/events`); with the Vite dev
server (`bun dev`, fixtures) it runs offline on fixtures. Endpoints:
`GET /api/dashboard`, `POST /api/commands`, `GET /api/thread?session=S&from=N`,
`GET /api/events?from=N&session=S`. See `crates/capo-web/README.md` and
`web/app/README.md`.

### Project memory

```sh
cargo run -p capo-cli --bin capo -- project memory index --root .
cargo run -p capo-cli --bin capo -- project memory next
cargo run -p capo-cli --bin capo -- project memory start-next --agent demo
```

Read memory through the governed wrapper tool:

```sh
mkdir -p .capo-dev/readme-artifacts
cargo run -p capo-cli --bin capo -- tool run-wrapper \
  --tool project_memory_read --workspace . \
  --artifacts .capo-dev/readme-artifacts --path project.md
```

The transitional `capo workpad ...` commands and `--workpad-*` flag aliases still
exist for older scripts and repo migration; new work should use the
`project`/`memory`/`source-*` product language.

## What's missing / not yet a daily driver

- **Not dogfooded.** Capo has not yet been turned on its own work; the harness is
  proven by tests, not by running real Capo development through Capo.
- **Real-but-unwired edges.** The OS sandbox is not the default spawn wrapper;
  autonomous continuation is opt-in and off by default; some Linux bwrap preflight
  paths are pending; isolation/sandbox layers exist behind their seams but are not
  yet the production default.
- **Provider breadth.** Only Codex is live (read-only one-shot); Claude and the
  live ACP wire round-trip are gated or fixture-bounded.
- **Web UI is functional, not polished.** Live mode powers Overview, the agent
  table, the dispatch pipeline, and the chat surface; goals, tool catalog,
  reviews/validations, and permissions still show empty states in live mode and
  carry full data only on fixtures.
- **FTS-only memory.** Retrieval is FTS5; no vector/embeddings/graph memory.
- **No remote runtime or tunnel.** Only `LocalProcessRunner` exists; remote/
  container runners and tunnels are modeled in the type system but unbuilt.
- **Agent-built code pending human audit.**

## Repo layout / learn more

- [`AGENTS.md`](./AGENTS.md) — orchestration brain and primary instruction surface.
- [`project.md`](./project.md) — product goal, thesis, and desired features.
- [`WORKING.md`](./WORKING.md) — agent workflow, verification, and CI contract.
- [`workpads/`](./workpads/) — per-area design and build history; start with
  `workpads/architecture/boundaries.md` and `state-model.md`, then the harness
  workpads (`real-turn-loop`, `streaming-transport`, `tools-aci`, `safety-gates`,
  `goal-autonomy`, `depth`).
- [`crates/capo-server/contract/`](./crates/capo-server/contract/) — the
  authoritative JSON-RPC/SSE wire contract.
