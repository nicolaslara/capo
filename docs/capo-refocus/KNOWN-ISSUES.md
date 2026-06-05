# Known issues — `slice-a-acp-wiring` (from the overnight self-review)

> Adversarial self-review of the branch diff (correctness/concurrency, security,
> ACP-protocol, quality). Findings were verified (false alarms dropped). NONE of
> these break the proven single-user dogfood loop — they are concurrency,
> robustness, and fidelity hardening. **M1 (detached failures swallowed) has been
> partially addressed** (failures are now logged; the richer fix — appending a
> terminal `turn_failed` event — remains). The rest are documented here for a
> supervised fix session, prioritized.

# REVIEW REPORT — `slice-a-acp-wiring` (capo)

Branch: `slice-a-acp-wiring` (10 commits ahead of `main`). Scope: ACP client wiring — capo-web `/api/chat` → conductor `claude-code-acp` session → in-process HTTP MCP → worker ACP turns, all on subscription (no API key).

---

## Blockers

### B1 — Concurrent `/api/chat` requests race the single boot-time conductor session
`crates/capo-web/src/main.rs:326-467` (and boot registration at 90-92, 350-351)
**Problem:** The `chat` handler holds no serialization. Every request drives `RunConductorTurnLocal` against the *same* `conductor.session_id`/`run_id`. Two concurrent POSTs interleave two ACP turns on one session, write concurrently to the same SQLite session/run, and corrupt per-turn slicing: `pre_watermark` (353-359) and `inv_before` (361-366) are captured pre-turn, but the `[pre_watermark..]` `ReadThread` slice (418-436) and `skip(inv_before)` invocation slice (439-454) are turn-*global*, not turn-scoped — so replies and `toolCalls` cross-attribute between tabs/users.
**Fix:** Hold a one-slot `tokio::sync::Mutex` across steps 2-6 so only one conductor turn runs at a time; or allocate a fresh session per turn and slice by `run_id`/`turn_id` instead of a shared watermark.
**Effort:** ~0.5 day (mutex) / ~2 days (per-turn session + scoped slicing).

---

## Majors

### M1 — Detached `start_agent` swallows ALL worker-turn failures → phantom "running" forever
`crates/capo-server/src/acp_mcp_http.rs:382-394`; root cause `crates/capo-server/src/server_core.rs:228-326`
**Problem:** Detached path returns `status:"running"` immediately and runs the turn as `let _ = server.handle(RunAcpLiveTurnLocal{...})` (383). In `run_acp_live_turn_local`, *every* failure path (gate closed 232-238, `spawn_live_session` 303, `drive_acp_live_turn` 320, `finalize` 325) returns `Err` *before* `record_server_request_handled` (352) — so on any failure no event is appended and the session stays in its initial state. The discarded `Err` (plus `panic=unwind` isolating a panicking thread) makes a later `review_agent`/`list_agents` see a session indistinguishable from "still running." The conductor polls forever.
**Fix:** In the detached closure, on `Err(e)` (and via `catch_unwind`) append a terminal/`turn_failed` event so projections reflect it; at minimum log it.
**Effort:** ~0.5 day.

### M2 — Non-detached `start_agent` blocks a tokio worker thread for the entire nested worker turn
`crates/capo-server/src/acp_mcp_http.rs:128-178, 263-294, 408-421` (capo-web spawn at main.rs:248)
**Problem:** `handle_post` is `async`; for `tools/call` it synchronously calls `handle_tools_call` → `tool_start_agent`, which (non-detached) calls `server.handle(RunAcpLiveTurnLocal)` (408) — a multi-minute blocking stdio drive — directly on the async task with no `spawn_blocking`. This blocks a runtime worker thread serving the MCP endpoint. A few concurrent non-detached calls starve the executor.
**Fix:** Offload the blocking drive via `tokio::task::spawn_blocking` (tool fns are sync; refactor so `handle_post` can offload).
**Effort:** ~0.5 day.

### M3 — MCP `set_mode` tool writes state that is never read — silently ignored
`crates/capo-server/src/acp_mcp_http.rs:556-573` (write at 569-571; field at 70)
**Problem:** `tool_set_mode` writes `state.mode` (`Arc<Mutex<ConductorMode>>`), but `McpState.mode` is read nowhere in capo-server. The mode that actually drives goal composition is a *different* mutex — `ConductorChat.mode` in capo-web (main.rs:339-348) — updated only from the `ChatBody.mode` HTTP field (331), never from the MCP tool. So `set_mode("one", …)` returns `isError:false` and changes nothing observable.
**Fix:** Inject capo-web's `ChatMode` mutex into `McpState` so the tool mutates the state `conductor_goal` reads, or remove `set_mode` and document mode as client-only.
**Effort:** ~0.5 day.

### M4 — `fs/read_text_file` ignores ACP `line`/`limit` and returns the whole file
`crates/capo-adapters/src/acp_wire.rs:755-759`
**Problem:** The ACP `fs/read_text_file` contract carries optional `line` (1-based start) and `limit`; the bridge issues them. The impl returns full file content regardless. When the bridge requested a slice, the agent gets differently-offset content, desyncing line-numbered Read/Edit reasoning — a correctness hazard for the exact on-wire write flow this branch exists to enable.
**Fix:** Honor `line`/`limit` from `input` when present, returning only the requested window.
**Effort:** ~0.5 day.

### M5 — tool_call `content`/`diff` blocks are silently dropped on the real 0.16.2 bridge
`crates/capo-adapters/src/provider_parsers.rs:437-445` via `text_from_content_array` (`crates/capo-adapters/src/event.rs:371-391`)
**Problem:** The comment claims `text_from_content_array` reaches into the real array shape `[{type:"content",content:{...}},{type:"diff",...}]`. It does not. In the `Value::Array` arm (event.rs:374-385) each item tries `string_at` for `text`/`content`/`input`; for a `content` block `item.content` is an *object*, and `string_at`'s terminal match (355-360) returns `None` for non-scalars, so `content.text` is never reached. A `diff` block has no string key at all → dropped. Net: on the real bridge every observed tool result normalizes to `content = None`, breaking the "observed result distinct from the agent's claim" invariant the design relies on. Tests pass only because fixtures use the legacy single-object shape (`Value::Object` arm).
**Fix:** In the array arm, recurse into the ACP wrapper (`text_from_content_array(item.get("content"))`) and special-case `diff` (emit `newText`, optional `path:` prefix). Add a fixture with the real array shape + a diff block.
**Effort:** ~1 day (incl. fixture).

### M6 — Unbounded MCP invocation log grows for the process lifetime
`crates/capo-server/src/acp_mcp_http.rs:74, 280-286`; `crates/capo-web/src/main.rs:89, 439-454`
**Problem:** Every `tools/call` pushes a `ToolInvocation` (incl. full `arguments` Value) into an `Arc<Mutex<Vec<…>>>` that is never drained or capped; in capo-web it lives the whole process and is shared across all chat turns. Each `/api/chat` re-locks and `skip(inv_before)`s the tail, so all prior entries are pure dead weight — unbounded memory growth + ever-growing lock-hold.
**Fix:** Cap with a ring buffer, or truncate to `inv_before` at the start of each turn (capo-web owns the watermark and never needs older entries).
**Effort:** ~0.5 day.

### M7 — Synchronous conductor turn has no timeout and pins a blocking-pool thread indefinitely
`crates/capo-web/src/main.rs:368-404` (author comment at 371-372: "No axum-side timeout; live turns are slow")
**Problem:** `RunConductorTurnLocal` drives a real nested `claude-code-acp` + nested `claude` synchronously inside `spawn_blocking`. A hung worker/model pins a blocking-pool thread with no cap and no client-cancellation path; the HTTP client can disconnect but the turn keeps running. With the default 512-thread blocking pool, enough stuck turns starve every other `spawn_blocking` endpoint (`/api/dashboard`, `/api/thread`, `/api/commands`).
**Fix:** Wrap in a bounded `tokio::time::timeout` and/or a cancellation token tied to disconnect; surface timeout as 504 and tear down the spawned process group.
**Effort:** ~1 day.

---

## Minors

### m1 — `start_agent` ids are a pure function of `(name, task)` → identical re-invocations collide
`crates/capo-server/src/acp_mcp_http.rs:330-333` (misleading comment at 329)
**Problem:** `session_id`/`run_id`/`turn_id` are `stable_hash(name:task)`. Calling `start_agent` twice with the same name+task yields identical ids; the second `StartSession` collides, and `command_identity_hash` (server_core.rs:337-343) is identical so dedup may swallow it. A conductor retrying the same task gets aliasing, not a fresh worker.
**Fix:** Mix a per-call nonce (uuid or `turn_seq`) into the run/turn ids.
**Effort:** ~0.25 day.

### m2 — `register`-error fallback fabricates `agent-{name}` instead of resolving the real id
`crates/capo-server/src/acp_mcp_http.rs:321-327`
**Problem:** On `RegisterAgent` error (e.g. already-exists), `agent_id` is fabricated as `format!("agent-{name}")` (326) rather than read back. If the stored id differs, the returned id is wrong; downstream works only because resolvers also match on name (`resolve_agent_name`, 592). Brittle.
**Fix:** On register error, do a Dashboard lookup to resolve the real id (the existing `resolve_agent_name`/`dashboard_snapshot` helpers already do this).
**Effort:** ~0.25 day.

### m3 — `spawn_mcp_server` serve-task panic is isolated and silent
`crates/capo-web/src/main.rs:248-249`
**Problem:** `axum::serve(...).await.expect("serve MCP endpoint")` runs in a detached `tokio::spawn` whose `JoinHandle` is dropped. If the endpoint dies, the panic is swallowed and every later conductor turn fails to reach its tools with no surfaced cause.
**Fix:** Log the error / propagate a process-level shutdown.
**Effort:** ~0.25 day.

### m4 — Read path uses `confine_write_path`; doc references a non-existent `confine_read_path`, and the write-confine rejects legit reads
`crates/capo-adapters/src/acp_client.rs:142` (doc) vs `crates/capo-adapters/src/acp_wire.rs:736` (call), rule at acp_wire.rs:722
**Problem:** The doc references `confine_read_path` as the read confinement, but `execute_client_call_fs` calls `confine_write_path` for both read and write. Write-confine is the safe direction, but its "reject credential-like components" rule also rejects *legitimate reads* of `.env`-style/dotfile paths an agent may need. Doc is wrong and reads are over-restricted.
**Fix:** Reconcile the comment; consider a dedicated read-confine that permits reads a write would refuse.
**Effort:** ~0.25 day.

### m5 — New tests mutate process-wide ACP-gate env vars without restoring them
`crates/capo-web/src/main.rs:1781-1784` (same pattern in `crates/capo-server/tests/{acp_mcp_http_smoke,conductor_turn_smoke,acp_live_bridge_smoke,acp_dispatch_smoke}.rs`)
**Problem:** `set_var("CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT"/"CAPO_SERVER_RUN_ACP_LIVE", "1")` is process-global, set unconditionally with no `remove_var` and no `serial_test`/RAII guard. Cargo runs a binary's tests in parallel, so this can leak the gate "open" into a sibling test expecting it closed (the fail-closed gate tests depend on it being off).
**Fix:** Use a serialized guard (`serial_test`) or a RAII env guard that restores prior values.
**Effort:** ~0.25 day.

### NIT — Stale `bypassPermissions` doc contradicts the shipped `"default"` mode
`crates/capo-server/src/acp_mcp_http.rs:43-46`
**Problem:** The `acp_session_mode` doc (45) says the live bridge uses `Some("bypassPermissions")` "so its Write tool round-trips an on-wire write." The shipped wiring uses `Some("default")` precisely because `bypassPermissions` lets the worker delegate writes to a simulated sub-agent that never crosses the wire. The comment misleads about the on-wire path this branch depends on.
**Fix:** Update the doc to describe `default` as the on-wire mode.
**Effort:** ~0.1 day.

---

## Overall assessment

The branch is structurally sound and largely does what it sets out to do, but it is **not yet ready to hand to a human reviewer without flagging must-fixes**. There is one true blocker (B1) and three correctness/robustness majors that undermine the validated-loop premise this slice exists to prove:

- **Must-fix before review:** B1 (concurrent-chat cross-attribution — silently mixes users' replies/tool calls), M1 (failed worker turns become invisible phantom-running sessions the conductor polls forever), and M5 (real-bridge tool results normalize to `content = None`, defeating the core "observed result vs. agent claim" invariant — and hidden because no fixture exercises the real array/diff shape).
- **Should-fix soon:** M2/M7 (executor and blocking-pool starvation under modest concurrency) and M3 (a shipped MCP tool that is a silent no-op). M4 and M6 are real correctness/resource issues but lower-blast-radius.
- The minors and NIT are cheap and worth clearing in the same pass; several are misleading comments (m1, m4, NIT) that will actively mislead the next reader.

Recommend fixing B1, M1, and M5 (plus adding the M5 real-shape fixture) before handing off; the rest can be tracked as review follow-ups. Note that M5 in particular means the current green test suite is giving false confidence — its fixtures don't cover the production wire shape.
