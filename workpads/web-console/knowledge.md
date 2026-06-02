# Web Console Knowledge

## Objective

Capture decisions for the `web-console` workpad: complete the seven-screen
operator console, tighten the live streaming-chat UX over the real ST4/ST5
contract, make the web client remote-aware (configurable endpoint + auth handle)
and reconnect/resume-safe, and preserve the offline fixture fallback — all
strictly on the CLIENT side of the boundary, and entirely within `web/app`,
runnable fully in parallel with the harness and connectivity-tunnel tracks.

## Scope Decision (And The Ownership Resolution)

`web-console` is an INDEPENDENT, parallel-safe workpad that DEEPENS the client
surface without touching anything the harness track owns. The defining structural
decision is that this workpad lives entirely on the `InputSurface` side of
`boundaries.md` AND entirely within the BROWSER CLIENT (`web/app`): it submits
commands and renders read models, and it never owns orchestration state, the
controller turn loop, the streaming transport protocol, the permission engine, the
goal model, or the server-side `crates/capo-web` facade.

### The injected ownership decision (resolves the AGENTS.md / TASKS.md web boundary)

`AGENTS.md:61` and `TASKS.md:11,63` are explicit: "The web UI (`web/app`,
`web/dashboard`) is owned by a separate agent and is out of scope for the harness
track; those workpads deliver only the server-side streaming contract (evolving
`crates/capo-web`)." That is, server-side `crates/capo-web` evolution belongs to
the streaming-transport / harness track, and the browser client belongs to a
separate (web) agent. `web-console` IS that web-client agent's workpad. Therefore:

- `web-console` OWNS only `web/app` (the React console, its data layer, its
  endpoint/auth/reconnect logic).
- `web-console` makes NO server-side change to `crates/capo-web`. It CONSUMES the
  facade and the published wire contract.
- The draft's WC7 (adding `/api/health`, bearer-auth validation, and a CORS
  allowlist INSIDE `crates/capo-web`) is REMOVED, because it would have a
  client-side workpad mutate a crate another track owns, contradicting the source
  of truth. Those facade capabilities are now recorded as a cross-workpad
  DEPENDENCY (WC7-DEP) on the facade-owning track. If a future explicit
  authorization reassigns `crates/capo-web` server changes to `web-console`, that
  authorization will be cited in `tasks.md` before such a task is opened.

This mirrors how `depth` deepens at existing seams without re-architecting earlier
phases, and it is why `web-console` can run in parallel with every other track: it
consumes contracts (`capo-wire`, the four `/api/*` routes, the `from_sequence`
resume) that are already implemented and stable, and it consumes the
connectivity-tunnel's endpoint/exposure/auth vocabulary WITHOUT implementing the
tunnel or the facade.

### Why facade auth is NOT pulled into this workpad

The draft proposed facade-side bearer-token validation. Two boundary reasons reject
it here. First, ownership (above): `crates/capo-web` is the harness track's. Second,
the AGENTS.md safety boundary requires keeping tunnel/connectivity concerns SEPARATE
from agent execution and controller state, and `runtime-tunnel.md` keeps
`auth_ref`/`exposure`/endpoint resolution INSIDE `ConnectivityTunnel`. Who-may-reach
the endpoint is the connectivity-tunnel track's concern; request authentication
added to the execution-facing facade would put access-control policy adjacent to the
permission model this workpad explicitly defers. The client therefore stays
open-on-loopback and attaches a bearer header only when a token is resolved
(client-side, in memory); any facade-side auth is an explicitly-authorized task for
the facade/connectivity owners, scoped (if a stopgap) to a single env-configured
shared-secret check logged as a `WORKING.md` workaround with a follow-up review.

## Current State (Verified In-Tree, 2026-06-02)

- The client is hard-wired to same-origin `/api/*`: `store.tsx` calls
  `fetch('/api/dashboard')`, `fetch('/api/commands')`, and `new
  EventSource('/api/events')` with no base URL and no auth. There is no endpoint
  config and no auth handle.
- Live mode is auto-detected: if `GET /api/dashboard` answers, the store flips
  `liveRef`, re-polls the dashboard every 4s (preserving accumulated `chats`), and
  subscribes to the SSE tail; otherwise it stays on `fixtureData`. This auto-detect
  + offline fallback must be preserved.
- The SSE subscription is NOT reconnect/resume-safe: it is a bare
  `new EventSource('/api/events')` that "tails from now" and records no delivered
  watermark, so a drop loses any events committed during the gap. WC2 adds a
  contiguous watermark + app-controlled reconnect-with-resume
  (`from=<lastDeliveredSequence>`).
- The server side already supports resume: `crates/capo-web` `events()` /
  `run_event_tail` honor `from` (default "tail from now" via `last_sequence()`),
  dedupe across the broadcast hub + cross-process catch-up against
  `delivered_through`, and apply `KeepAlive::default()`. The `from_sequence` resume
  contract the client needs is implemented and tested by the OWNING track.
- The facade ships `CorsLayer::permissive()` (`main.rs:125`), so cross-origin fetch
  + SSE already work; tightening to an allowlist is hardening owned by the facade
  track, not enablement here.
- The `dashboard.project` block (`main.rs:660-667`) reports `addr` + a hardcoded
  `"mode": "live"` and NO exposure field; live mode returns `goals: []`,
  `permissions: []`, `tools: []` (`main.rs:675-677`, with the comment "need new
  ServerCommands / projections"). WC4/WC5 keep these HONEST rather than faking data;
  WC6 infers exposure from `baseUrl` until a facade hint exists.
- Chat is a flat message list: `live.ts` projects `threadToChatMessages` /
  `eventToChatMessage` into a single ordered list; turn grouping, a streaming
  indicator, and redaction-aware rendering are the WC3 work. `classifyEventKind`
  already maps `session.summary_updated` → agent, `tool.*` → tool, terminal kinds →
  system. Every checked-in wire snapshot is `redaction_state: "safe"`, so WC3 must
  add a NON-safe fixture to actually exercise the redaction placeholder.
- Settings shows a DISABLED "Live server — coming next" toggle and an "Updates:
  polling (SSE when wired)" placeholder — both stale (SSE is wired); WC6 replaces
  them with a functional, auditable remote-connection control.

## Remote-Awareness Design

The client becomes remote-aware by introducing one connection config (`baseUrl` +
optional `authRef` HANDLE NAME) resolved at startup (query param → persisted
setting → default) and routing every request through a single `apiUrl`/`apiFetch`
helper. This keeps the change surface small and centralizes the safety-boundary
rules.

### Auth-resolution decision (was an open question; now binding for WC1)

The persisted/URL-sourced config carries the `authRef` HANDLE NAME ONLY — never
token/secret material. The token is resolved to an IN-MEMORY value for the session
by an explicit mechanism (a one-shot prompt now; a server-issued short-lived session
token via a login round-trip later) and is attached only as an `Authorization`
header. It is NEVER written to localStorage, NEVER logged, and NEVER placed in a URL
query string. `?auth=` carrying secret material is forbidden; `?auth=` may carry only
a handle name. This is enforced by a deterministic leak-scan test (WC1), not by an
unaudited checklist — the draft's "documented code-review checklist" is explicitly
NOT sufficient under the no-self-attestation invariant.

Coordination with connectivity-tunnel is by VOCABULARY, not by code: the configured
server is conceptually a `ResolvedEndpoint` reached over a `ConnectivityTunnel` whose
`exposure` is `loopback` / `private` / `public` (`runtime-tunnel.md`). The client
references credentials by an `auth_ref` HANDLE exactly as
`ConnectivityEndpoint.auth_ref` does — "a secret handle or OS/vendor credential
location, never raw credential material." The client never opens a tunnel, never
resolves an endpoint server-side, and never implements SSH/Tailscale/reverse
transport; it only points at a URL the tunnel makes reachable.

## Reconnect / Resume Design

The native browser `EventSource` auto-reconnects but does NOT resume by application
sequence and does NOT let the app carry a `from` watermark into its built-in retry.
The real work is therefore RESUME, implemented by app-controlled reconnect: on
`error`/close the client CLOSES the native source and CONSTRUCTS a new one at
`/api/events?from=<watermark>` under bounded exponential backoff + jitter, capped at
a documented ceiling, stopping on unmount or a switch to fixtures.

The watermark is the CONTIGUOUS high-water sequence — the highest sequence with no
gap below it — NOT the max observed. The server delivers in committed-sequence order
(`run_event_tail`/`delivered_through`), so contiguous == max in practice; this is
stated as an explicit DEPENDENCY and pinned by a test (deliver N, then N+2 before
N+1, assert the watermark stays at N) so an out-of-order frame can never advance the
watermark past a gap. Because the server replays only events strictly after `from`
and the client keeps its `seenEventIds` (event-id) dedupe set, the seam across a
reconnect is gap-free and duplicate-free. The watermark is a `sequence`; the dedupe
key is `event_id`; the two are reconciled by test, not assumption.

Keep-alive: the browser `EventSource` API does NOT surface SSE comment frames
(`: keep-alive`) to application code — they keep the socket warm and fire no event or
error. So there is nothing for the client to "treat" as a disconnect; the only
testable assertion (against the mock `EventSource`) is that a comment-only idle
window does NOT trigger the app-controlled reconnect path. The actual
comment-suppression behavior is confirmed against the WHATWG HTML living standard at
implementation time rather than assumed.

## Streaming-Chat UX Design

Chat is grouped into turns using the ST5 thread's `turnId` / `firstSequence` /
`lastSequence` and the live tail's `turn_id`. A turn is "in progress" (showing a
streaming/typing indicator) from command-send until its terminal event
(`run.exited` / `session.interrupted` / `session.stopped`) arrives on the tail, at
which point the indicator clears deterministically. Tool and terminal items render
distinctly, and `redaction_state` is honored on every item (a redacted item shows a
placeholder, never raw content) — consistent with the cross-cutting redaction
discipline and the AGENTS.md safety boundary. Because every checked-in snapshot is
`"safe"`, the redaction test ships a NON-safe fixture so the placeholder path is
proven, not nominal. The inline `PermissionCard` (WC4) is layered on top of the turn
grouping, so WC4 sequences after WC3 on the Chat surface to avoid re-doing placement.

## Theme Parity As A Test, Not A Screenshot

Light + dark parity (WC5) is a DETERMINISTIC computed-style assertion over both
`?theme=light` and `?theme=dark`: no rendered element resolves to a hard-coded color
outside the theme tokens. This makes parity falsifiable in CI and auditable, rather
than resting on a gitignored screenshot a reviewer cannot see (which the WC0
no-self-attestation invariant forbids). If screenshots are produced as supplementary
evidence, the task first confirms whether `web/dashboard/scripts/shoot.mjs` can drive
the `web/app` dev server at `?theme=` (it targets the legacy `web/dashboard` tree;
this is verified, not assumed), adds a `web/app`-local shoot script if not, and
commits any retained shots as canonical evidence rather than gitignoring them.

## Exposure Indicator Is Advisory And Fail-Loud

The WC6 connection-exposure indicator is ADVISORY, not a security control. Its
primary path infers loopback-vs-remote from `baseUrl`
(`localhost`/`127.0.0.1`/`::1` → loopback). Because a tunnel can map a remote
endpoint onto `localhost:PORT`, a naive heuristic would FAIL SILENTLY — the worst
outcome for a safety affordance — so when exposure is uncertain the client DEFAULTS
TO WARNING (treats it as non-loopback). The authoritative `loopback`/`private`/
`public` value is consumed only if/when the facade surfaces it (WC7-DEP); until
then the fail-loud inference holds.

## Safety Boundary (First-Class Acceptance Criterion)

The AGENTS.md safety boundary is an acceptance criterion for every task:

- A subscription-backed agent driven through the console is a PRIVILEGED CONNECTOR.
  The UI must make it visible when the operator is driving a remote (non-loopback)
  server, and remote-control affordances (endpoint, auth handle, connect/disconnect)
  must be auditable and revocable.
- The client NEVER stores or logs raw API keys, subscription/OAuth tokens, cookies,
  session files, or transcripts-with-secrets. An auth token is referenced by HANDLE
  NAME, held only in memory when resolved, attached only as an `Authorization`
  header, and never written to localStorage, the console, or a URL query string —
  enforced by the WC1 leak-scan test.
- The client honors `redaction_state` on every rendered event/item, proven against a
  non-safe fixture.
- Tunnel/connectivity and request-auth concerns are kept SEPARATE from this
  client-side workpad: facade auth, if any, is the facade/connectivity owners' task.
- The WC8 live-smoke credential scan is concrete and fail-closed (greps artifacts for
  the known token + common token shapes, also unit-tested over a captured-log
  fixture), so "secrets stripped" is falsifiable rather than asserted.

## Verification Discipline

Deterministic-first holds across every task. Client projection, reconnect/resume,
endpoint/auth composition + leak scan, theme parity, and screen rendering are tested
offline against fixtures and mock transports with no live server. The live `web ↔
capo-web ↔ capo-server` path stays gated by the two existing deterministic halves —
the OWNING track's `http_facade_serves_the_live_chat_round_trip` server test and the
client `bun run build` compile against the shared `capo-wire` contract — plus the new
WC1/WC2/WC3/WC4/WC5 client tests. Any live remote smoke is opt-in, paired with a
deterministic assertion that pins the same resume/projection shape, gated by a
fail-closed credential scan, and skips cleanly when no server is reachable. Nothing
completes on operator self-attestation.

## Non-Goals

- No server-side `crates/capo-web` change (the streaming-transport / harness track
  owns the facade; WC7-DEP records the asks). No new `/api` routes, no facade auth,
  no facade CORS allowlist built here.
- No changes to the server turn loop, streaming transport protocol, permission
  engine, or goal model (the harness track owns those).
- No implementation of the `ConnectivityTunnel` / `RemoteProcessRunner` /
  `ExposurePolicy` or any SSH/Tailscale/reverse transport (the connectivity-tunnel
  track owns those; this workpad is a consumer of their vocabulary).
- No fabricated live data: goals / tools / permissions / reviews / validations show
  HONEST empty states in live mode until a server-side projection lands.
- No new `ServerCommand`s or projections; if a live permission decision or goal view
  needs one, it is recorded as an open dependency, not built here.
- No raw-secret storage, logging, or URL embedding anywhere in the client; no
  request-auth policy added to the execution-facing facade by this workpad.

## Open Questions

- Which in-memory auth-resolution mechanism ships first in WC1 — a one-shot session
  prompt or a server-issued short-lived session token via a login round-trip? (Both
  satisfy the binding rule that no token touches localStorage or a URL; the choice is
  UX/timing, not a boundary question. Leaning: one-shot prompt now, session token
  when the facade owners expose a login endpoint.)
- When will the facade-owning track land the WC7-DEP exposure/health hint so WC6 can
  show an authoritative `loopback`/`private`/`public` value instead of the fail-loud
  `baseUrl` inference?
- Does a parallel web agent also touch `web/app`? Tasks are kept modular (per-file,
  additive) to avoid collisions; coordinate on `store.tsx`/`live.ts` ownership if
  both are active.
- Should live-mode permission decisions be wired to a real server command now, or
  remain a documented honest-disabled state until the server projects a permission
  queue? (Leaning: honest-disabled until the projection exists.)
