# Worker-result channel (3-tier) — `report_result` + file + reply-text fallback

Decided with the owner (2026-06-06): replace the file-only fan-out hack with a
layered worker→conductor result channel, **keeping the file as a fallback**.

## How the conductor resolves a worker's result (in order)
1. **`report_result(key, value)` — primary, structured.** A new capo MCP tool.
   The worker calls it once when done; the value is stored under `key` (the
   result identifier the conductor assigned, typically the result filename).
   `collect_results` returns it **in preference to reading the file** — no file
   dance, no prose-parsing.
2. **Result file — fallback.** If no value was reported for a key,
   `collect_results` blocks (as before) until the relative file has non-empty
   content and returns that. The conductor still tells workers to write the file
   too, so a worker that never calls the tool still works.
3. **Reply-text extraction — last resort.** If a worker neither reported nor
   wrote a file (`null` after retry), the conductor (which IS an LLM) falls back
   to `review_agent` on that worker and reads the result from its reply text.
   This is realized at the conductor-orchestration layer (the prompt), not in the
   MCP server, which has no model handle — and the conductor is told to say when
   it used this fallback.

## Wiring (all additive; validated loop byte-identical)
- `report_result` tool added to the capo MCP server (`acp_mcp_http.rs`); stores
  into a process-wide `Arc<Mutex<HashMap<key,value>>>` on `McpState`, shared
  across clones (incl. detached worker threads) — so a worker's report reaches
  the conductor turn that calls `collect_results`.
- Workers don't have a capo MCP channel by default. `start_agent` now forwards
  capo's **same** in-process MCP endpoint into each worker's `session/new`
  (new `mcp_url`/`mcp_headers` on `RunAcpLiveTurnLocal`, set from
  `McpState::with_worker_mcp`, wired in capo-web's `spawn_mcp_server`). When
  `mcp_url` is `None` (every existing test + the deterministic path) NO MCP
  server is advertised to the worker → the validated file-only loop is unchanged.
- `collect_results` resolution order updated to reported-value → file.
- Conductor prompt updated to instruct workers to `report_result` (and still
  write the file as fallback), and to use the `review_agent` last-resort.

## Tests
- `report_result_is_preferred_by_collect_results` (deterministic): a reported
  value is returned with no file on disk; an unreported key is `null`/not-ready.
- Codec round-trips the new `RunAcpLiveTurnLocal` fields; full capo-server (163)
  + capo-web (6) suites green; clippy clean.

## Keying note
`key` is confined to a bare name (a worker can't key another worker's slot via a
path), mirroring `collect_results`' file-name confinement. The conductor assigns
distinct keys per worker (the result filenames it already assigns), so the tool
and file tiers share one namespace.
