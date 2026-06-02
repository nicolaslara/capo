# Web Console Tasks

## Objective

Polish and complete the Capo operator console (`web/app`) into a finished
seven-screen, terminal-native (light + dark) supervision surface; tighten the
live streaming-chat UX over the real ST4/ST5 event-tail + thread contract; make
the web CLIENT remote-aware so it can connect to a Capo server reached over the
connectivity-tunnel (a configurable endpoint + auth handle) and survive
keep-alive/reconnect by resuming the event tail via `from_sequence`; and preserve
the offline fixture fallback throughout. The surface is the `web/app` React
console ONLY. Per `AGENTS.md` and `TASKS.md`, the server-side `crates/capo-web`
facade is owned by the streaming-transport / harness track, NOT by this workpad;
`web-console` is a CLIENT of the facade and the wire contract, and records its
facade asks as CROSS-WORKPAD DEPENDENCIES rather than implementing them.

This workpad runs FULLY IN PARALLEL with the entire daily-driver harness track
and the connectivity-tunnel track. It stays strictly on the CLIENT side of the
boundary map (`InputSurface` in `boundaries.md`): it submits commands and renders
read models, and it never owns orchestration state, the loop, the transport
protocol, the permission model, the goal model, or the server-side facade. It
coordinates WITH the connectivity-tunnel exposure/auth model but implements none
of the tunnel itself.

## Status

Planned. All tasks pending.

## Feature Set

- A complete, finished seven-screen console (Overview / Agents / Chat / Goals /
  Tools / Activity / Settings) with terminal-native light + dark parity, no dead
  placeholders, and honest empty states where live data is not yet projected.
- A tightened streaming-chat UX over the real ST4 event tail + ST5 thread:
  visible streaming/typing state, turn boundaries, tool/terminal item rendering,
  inline permission cards, error/retry, and gap-free resume.
- A remote-aware client: a configurable server endpoint (origin/base URL) + an
  auth handle (by reference, never raw), so the console connects to a Capo server
  reachable over the connectivity-tunnel instead of being hard-wired to
  same-origin `/api/*`.
- Keep-alive / reconnect survival: the SSE event tail and the dashboard poll
  reconnect with app-controlled backoff after a drop and RESUME from the last
  delivered `sequence` via `from_sequence` so the chat thread never gaps or
  duplicates.
- A preserved, first-class offline fixture fallback selectable independently of
  server reachability.
- `web/app` builds and lints clean (`bun run build` = `tsc -b && vite build`;
  `bun run lint`). The `crates/capo-web` cargo gate is the OWNING track's
  responsibility; `web-console` only compiles the client against the published
  wire contract.

## WC0 - Workpad, Boundary Scope, And Verification Invariant

Status: pending.

Scope:

- Establish `web-console` as an INDEPENDENT workpad on the `InputSurface` (client)
  side of `boundaries.md`, runnable in parallel with the whole harness track and
  the connectivity-tunnel track.

Acceptance criteria:

- Record the boundaries this workpad OWNS: the `web/app` React console UX (the
  seven screens, theming, chat UX), the client-side data layer
  (`web/app/src/data/{store.tsx,live.ts,fixtures.ts,types.ts,capo-wire.ts}`), and
  the client endpoint/auth/reconnect/resume logic. The console treats
  `crates/capo-web` as a thin client-facing HTTP/SSE adapter it CONSUMES, not
  edits.
- Record the OWNERSHIP DECISION that resolves the `AGENTS.md`/`TASKS.md` web
  boundary: `AGENTS.md:61` and `TASKS.md:11,63` assign all server-side
  `crates/capo-web` evolution to the streaming-transport / harness track and put
  the browser client in a separate agent's hands. This workpad takes ONLY the
  browser-client half (`web/app`) and makes NO server-side `crates/capo-web`
  change. Every facade capability this client needs that does not exist today (an
  exposure/health hint, optional auth validation, a CORS allowlist) is recorded as
  a CROSS-WORKPAD DEPENDENCY on the facade-owning track, never built here. If a
  future explicit authorization hands `crates/capo-web` server changes to
  `web-console`, that authorization is cited in this file before any such task is
  opened.
- Record the boundaries it explicitly DEFERS / does NOT change: the server turn
  loop, transport protocol, permission model, and goal model (the harness track);
  the server-side `crates/capo-web` facade routes, auth, and CORS (the
  streaming-transport / harness track — this workpad consumes them); the
  `ConnectivityTunnel`/`RemoteProcessRunner`/`ExposurePolicy` implementation (the
  connectivity-tunnel track — this workpad is a tunnel CONSUMER, not an
  implementer); new `ServerCommand`s / projections that goals / tools /
  permissions live data would require (out of scope unless a server agent lands
  them).
- Record the coordination seam with connectivity-tunnel: the client treats the
  configured server endpoint as a `ResolvedEndpoint` reachable over a
  `ConnectivityTunnel` whose `exposure` is `loopback` / `private` / `public`, and
  references credentials by an `auth_ref` HANDLE only — mirroring
  `ConnectivityEndpoint.auth_ref` in `runtime-tunnel.md`.
- Record the SAFETY-BOUNDARY invariant as a first-class acceptance criterion for
  every task: the client NEVER stores or logs raw API keys, subscription/OAuth
  tokens, cookies, session files, or transcripts-with-secrets; an auth handle is
  referenced by name and resolved from a non-persistent/in-memory source; the
  client honors `redaction_state` on every rendered event/item; remote-control
  affordances (which endpoint, which auth handle, connect/disconnect) are visible
  and revocable in the UI. This invariant is enforced by an automated test where
  stated (WC1), never by an unaudited code-review checklist alone.
- Record the VERIFICATION invariant: no task completes on operator
  self-attestation; deterministic offline/fixture and contract tests land before
  any live-server-dependent behavior; the live `web ↔ capo-web ↔ capo-server` path
  stays paired with the existing deterministic halves (the facade-owning track's
  `http_facade_serves_the_live_chat_round_trip` server test + the `bun run build`
  client compile against the shared contract), and any live remote smoke is opt-in
  / skips cleanly when no server is reachable.

Verification:

- `workpads/web-console/tasks.md`, `knowledge.md`, `references.md` exist and
  record the above, including the ownership decision and the facade-as-dependency
  framing.
- No code changes.

Dependencies:

- Intra: none (gates the rest of WC).
- Cross: none to start. Coordinates with connectivity-tunnel (endpoint/auth/
  exposure vocabulary), with the streaming-transport / harness track (owner of
  `crates/capo-web`), and with any parallel web agent (keep tasks modular).

## WC1 - Client Endpoint + Auth-Handle Config (Remote-Aware Base, Offline-Preserving)

Status: pending.

Scope:

- Replace the hard-wired same-origin `/api/*` assumption with a configurable
  server endpoint and an auth-handle reference, while preserving the offline
  fixture path. Resolve, before this task is actionable, HOW a browser resolves an
  auth handle to a token without persisting or URL-embedding it (the WC0 safety
  boundary).

Acceptance criteria:

- AUTH-RESOLUTION DECISION (was an open question; now a binding criterion): the
  token is held ONLY in memory for the session and is obtained by an explicit
  in-memory mechanism — a one-shot prompt OR a future server-issued short-lived
  session token via a login round-trip — NEVER from localStorage and NEVER from a
  URL query string. The persisted/URL-sourced config may carry the `authRef`
  HANDLE NAME only; it MUST NOT carry token/secret material. `?auth=` carrying
  secret material is forbidden outright; `?auth=` may carry only a handle NAME that
  selects which in-memory credential to prompt for.
- Introduce a single client connection config resolved at startup: a `baseUrl`
  (default empty = same-origin `/api`, the current behavior) plus an optional
  `authRef` (a credential HANDLE NAME, never raw secret material). Sources, in
  precedence order: URL query param (e.g. `?server=`/`?auth=`, handle-name-only),
  a persisted setting (localStorage, handle-name-only), then a build-time default;
  documented in `knowledge.md`.
- Every existing fetch / EventSource call in `web/app/src/data/{store.tsx,live.ts}`
  (`/api/dashboard`, `/api/commands`, `/api/thread`, `/api/events`) routes through
  one `apiUrl(path)` / `apiFetch(path, init)` helper that prefixes `baseUrl` and,
  when an in-memory token has been resolved for the configured `authRef`, attaches
  it as an `Authorization` header — the token value is NEVER written to
  localStorage, NEVER logged, and NEVER placed in a query string.
- A `mode` selector with three states recorded in the store: `live` (facade
  reachable), `fixtures` (explicit offline), and `connecting/unreachable`; the
  offline fixture fallback remains reachable explicitly (e.g. `?mode=fixtures`) AND
  as the automatic fallback when `GET {baseUrl}/api/dashboard` does not answer —
  preserving today's auto-detect.
- Cross-origin requests already work against a remote facade because
  `crates/capo-web` ships `CorsLayer::permissive()` (`main.rs:125`); this task only
  CONFIRMS cross-origin fetch + SSE behavior against that existing layer and
  records the confirmation. Any CORS TIGHTENING (an allowlist) is a cross-workpad
  ask on the facade-owning track, not work done here (see WC7-DEP).
- The token/secret-handling rules from WC0 hold and are ENFORCED BY TEST, not by a
  checklist: a deterministic test simulates the auth flow and asserts the resolved
  token string never appears in localStorage, in any `window.location`/URL, or in a
  captured console log.

Verification:

- Deterministic (no server): a test/harness asserting `apiUrl` composition for
  empty vs set `baseUrl`, that `apiFetch` attaches the resolved auth header only
  when an in-memory token exists, and a leak-scan test asserting no code path
  persists, logs, or URL-embeds the raw token.
- `bun run build` and `bun run lint` clean.
- Cross-origin confirmation recorded against the existing permissive CORS layer (no
  server change in this workpad).

Dependencies:

- Intra: WC0.
- Cross: connectivity-tunnel for the `auth_ref` / exposure vocabulary (consumed,
  not implemented). A facade-side auth validation + CORS allowlist, IF wanted, is
  the facade-owning track's WC7-DEP (below) — this client works open-on-loopback
  today and attaches a bearer header only when a token is resolved.

## WC2 - Resilient Event Tail: Reconnect-With-Backoff + Resume Via from_sequence

Status: pending.

Scope:

- Make the live event-tail subscription survive transient drops and server/tunnel
  reconnects without gapping or duplicating the chat thread. The real work is
  RESUME-on-reconnect (carrying the watermark), because the native `EventSource`
  already auto-reconnects but does NOT resume by application sequence.

Acceptance criteria:

- Track a CONTIGUOUS resume watermark: the highest event `sequence` such that every
  sequence at or below it has been delivered (NOT merely the max observed). Today
  the store subscribes with a bare `new EventSource('/api/events')` and records no
  watermark. The server delivers in committed-sequence order
  (`run_event_tail`/`delivered_through`), so in practice contiguous == max; this
  task STATES that dependency explicitly and the test below pins it so an
  out-of-order frame can never silently advance the watermark past a gap.
- Mechanism: the native `EventSource` does NOT let the app carry a `from` watermark
  into its built-in retry, so on `error`/close the client CLOSES the native source
  and CONSTRUCTS a new one at `/api/events?from=<watermark>` under
  APP-CONTROLLED bounded exponential backoff + jitter, capped at a documented
  ceiling, stopping when the store unmounts or the user switches to fixtures. The
  server replays only events strictly after `from` (the `from` contract already
  implemented in `crates/capo-web` `events()` / `run_event_tail`).
- Preserve gap-free + duplicate-free delivery: the existing `seenEventIds` dedupe
  set continues to guard re-delivered frames (dedupe key is `event_id`), and the
  contiguous resume watermark (a `sequence`) guarantees no committed event between
  the drop and the resubscribe is lost — verified deterministically, including the
  watermark/dedupe-key reconciliation.
- Surface a visible "reconnecting…" state in the StatusBar during the backoff
  window.
- On reconnect, re-run the dashboard poll once immediately (so agent/lane state is
  fresh) without wiping accumulated `chats` (the existing `keepChats` path).
- Keep-alive: rely on the facade's `KeepAlive::default()` SSE comments to hold the
  connection. Because the browser `EventSource` API does NOT surface SSE comment
  frames to application code (they keep the socket warm and fire no event/error),
  the client cannot and need not "treat" a keep-alive gap as anything; the only
  testable assertion is that an idle period carrying only keep-alive comments does
  NOT trigger the app-controlled reconnect path. This is verified against the mock
  `EventSource`, and the actual SSE-comment-suppression behavior is confirmed
  against the WHATWG HTML living standard at implementation time (do not assume).

Verification:

- Deterministic client test (fixture/mock `EventSource`): simulate a drop after
  delivering up to sequence N, then a reconnect; assert the resubscribe URL carries
  `from=N`, that an event at N is not re-appended (dedupe), and that an event at
  N+1 delivered post-reconnect is appended exactly once.
- Contiguity test: deliver N, then N+2 before N+1; assert the watermark stays at N
  (not N+2) so a resubscribe replays N+1 — proving the watermark is contiguous, not
  max-observed.
- A backoff unit test asserting the delay schedule, the jitter bound, and the
  unmount/disable cancellation; and an idle-keep-alive test asserting no reconnect
  fires during a comment-only idle window.
- `bun run build`, `bun run lint` clean.
- Server side unchanged; the `from`-resume + catch-up behavior is already covered
  by the facade-owning track's `events_stream_surfaces_incremental_event_without_repoll`
  and e2e tests (consumed, not re-run here).

Dependencies:

- Intra: WC0, WC1.
- Cross: none for the client work (the `from_sequence` resume contract already
  exists server-side).

## WC3 - Chat Console UX: Streaming Reply, Turn Boundaries, Tool/Terminal Items, Errors

Status: pending.

Scope:

- Tighten the Chat screen so the real streamed reply over the tail reads as a
  coherent multi-turn conversation, not a flat append.

Acceptance criteria:

- Group the projected messages into TURNS using the ST5 thread `turnId` /
  `firstSequence` / `lastSequence` (and the live tail's `turn_id`) so operator
  prompt, agent summary, tool calls, and the turn-closing terminal note render as
  one visually-bounded turn; today `live.ts` projects a flat message list.
- Show a STREAMING/typing indicator for the active turn: from command-send until
  the turn's terminal event (`run.exited` / `session.interrupted` /
  `session.stopped`) arrives on the tail, the agent bubble shows an in-progress
  state; the indicator clears deterministically on the terminal event.
- Render `tool` items (name + result) and `system`/terminal notes distinctly from
  `agent`/`operator` messages, reusing the existing `ChatMessage` role taxonomy and
  the `classifyEventKind` mapping; honor `redactionState` (a redacted item shows a
  redacted placeholder, never raw content).
- Error + retry UX: a failed `send_task`/`steer_agent`/`interrupt`/`stop` shows the
  humanized error (the existing `humanizeErr`) inline with a retry affordance; an
  unreachable server shows a clear "server unreachable" state distinct from a
  command rejection.
- The composer disables/duplicate-guards while a turn is in flight for that agent
  (no double-send), and the optimistic operator message reconciles with the
  committed thread on hydrate (no duplicate operator bubble).

Verification:

- Deterministic component/unit tests over `live.ts` projection helpers
  (`threadToChatMessages`, `eventToChatMessage`) feeding a fixture thread + a
  fixture tail: assert turn grouping, the streaming-indicator lifecycle (set on
  send, cleared on the terminal event), tool/system rendering, and redaction
  placeholder behavior. Because EVERY checked-in wire snapshot carries
  `redaction_state: "safe"`, the redaction test MUST include at least one fixture
  item with a NON-safe `redaction_state` so the placeholder path is actually
  exercised — no live server.
- `bun run build`, `bun run lint` clean.

Dependencies:

- Intra: WC0, WC2 (resume) for gap-free history.
- Cross: none (ST4/ST5 contract already implemented in `crates/capo-web`).

## WC4 - Inline Permission Cards In Chat + Tools/Permissions Screen (Honest Live/Empty States)

Status: pending.

Scope:

- Finish the permission UX on the Chat surface and the Tools/Permissions screen,
  with honest states given that live permission data is not yet projected by the
  server.

Acceptance criteria:

- The existing `PermissionCard` renders inline in Chat for any pending permission
  request scoped to that agent, mapping the `allow_once` / `allow_always` /
  `reject_once` decisions onto the store's `decidePermission(once|always|reject)`
  (matching the ACP option vocabulary in `capability-permissions.md`); in fixture
  mode it drives the fixture permission queue (today's behavior). The card's
  placement is built ON TOP of the WC3 turn grouping so it is not reworked when
  turns land (WC4 sequences after WC3 on the Chat surface; see Dependencies).
- In LIVE mode, where the server does not yet project a permission queue
  (`map_dashboard` returns `permissions: []` at `main.rs:676`), the
  Tools/Permissions screen shows an HONEST empty state labelled as such (not a fake
  card), and records the open dependency on a server-side permission projection /
  decision command.
- The Tools screen renders the tool catalog from `data.tools` in fixture mode and
  an honest empty state in live mode (`tools: []` at `main.rs:677`), consistent
  with the README's stated status.
- No decision affordance in live mode silently no-ops: a live decision either
  routes through a real command path (if/when one exists) or is clearly DISABLED
  with the reason shown — never a button that pretends to work.

Verification:

- Deterministic component tests: a fixture pending permission renders a card and
  the three decisions update the fixture queue + activity; a live-mode empty
  `permissions`/`tools` renders the honest empty state and the disabled affordance
  WITH its reason string asserted.
- `bun run build`, `bun run lint` clean.

Dependencies:

- Intra: WC0; WC3 (turn grouping) for the in-Chat card placement so WC4 does not
  re-do work WC3 restructures.
- Cross: a future server-side permission/tool projection + decision command (out of
  scope here; recorded as an open dependency).

## WC5 - Complete The Remaining Screens: Overview, Agents, Goals, Activity (Light+Dark Parity)

Status: pending.

Scope:

- Bring every non-chat screen to a finished state with terminal-native light + dark
  parity, honest live data, and no dead placeholders.

Acceptance criteria:

- Overview: summary tiles (agents / active / blocked / evidence / reviews /
  validations) and recent activity render from live `data.summary` +
  `data.activity` and from fixtures offline; the blocked-tone warning path (already
  mapped in `crates/capo-web` `map_dashboard`) is visible.
- Agents: the agent table + per-agent inspect (dispatch pipeline, evidence ids,
  confidence, blocker) renders from live `map_agent`/`map_dispatch` output and
  fixtures; steer / interrupt / stop affordances route through the store command
  path (live) or the fixture path (offline), with disabled states honest per agent
  status.
- Goals + Activity: render from live data where projected; Goals shows an honest
  empty state in live mode (server returns `goals: []`) and full data in fixtures;
  Activity renders the deduped recent-event lane.
- Light + dark parity is verified as a DETERMINISTIC TEST, not a screenshot a
  reviewer cannot audit: a computed-style assertion over both `?theme=light` and
  `?theme=dark` asserts no rendered element resolves to a hard-coded color outside
  the theme tokens (no token-bypassing hex), so "parity" is falsifiable in CI.
  Screenshots, if produced, are supplementary evidence only.
- The `Placeholder` screen is removed or only used where a screen is intentionally
  not-yet-built, and the NavRail exposes exactly the seven screens.

Verification:

- The deterministic light+dark parity test above (both themes, no token-bypassing
  hard-coded colors), runnable with no live server.
- Optional supplementary screenshots: if `web/dashboard/scripts/shoot.mjs` is used,
  this task FIRST confirms and records in `references.md` whether `shoot.mjs` can
  drive the `web/app` Vite dev server at `?theme=`; if it cannot, a small
  `web/app`-local shoot script is added as a sub-task. Any retained shots are
  committed as canonical evidence (not gitignored) or replaced entirely by the
  parity test.
- `bun run build`, `bun run lint` clean.

Dependencies:

- Intra: WC0; WC1 for live/fixture mode plumbing.
- Cross: none for the rendered slices; Goals-live depends on a future goal
  projection (honest empty state until then).

## WC6 - Settings: Remote Connection Control + Revocable, Auditable Remote-Control Surface

Status: pending.

Scope:

- Make Settings the operator's remote-control surface: pick/connect/disconnect a
  server endpoint, reference an auth handle, and see connection/exposure status —
  replacing today's disabled "Live server — coming next" toggle.

Acceptance criteria:

- Settings exposes the WC1 connection config: the current `baseUrl` (or
  same-origin), the resolved `mode` (`live` / `fixtures` / `connecting`), the
  configured `authRef` HANDLE NAME (never the secret), and a connection-exposure
  indicator; the now-functional connect/disconnect (live↔fixtures) toggle replaces
  the disabled one.
- Remote-control affordances are AUDITABLE and REVOCABLE (the AGENTS.md safety
  boundary): changing the endpoint or auth handle is reflected immediately in the
  StatusBar, a Disconnect action drops the live tail + poll and returns to
  fixtures, and clearing the auth handle removes the in-memory token — all without
  logging or persisting raw secret material.
- The exposure indicator is FAIL-LOUD and explicitly ADVISORY, not a security
  control. Its PRIMARY path is a best-effort inference from `baseUrl`
  (`localhost`/`127.0.0.1`/`::1` → loopback, else remote), marked "best-effort, may
  under-warn" in the UI; because a tunnel can map a remote endpoint onto
  `localhost:PORT`, when the exposure is uncertain the client DEFAULTS TO WARNING
  (treats it as non-loopback) rather than silently suppressing the warning. The
  authoritative `loopback`/`private`/`public` value is used only if/when the facade
  surfaces it (today the `dashboard.project` block at `main.rs:660-667` reports
  `addr` + a hardcoded `"mode": "live"` and NO exposure field — see WC7-DEP).
- The Updates row honestly reflects the transport ("SSE event tail" in live,
  "fixtures" offline) instead of the current "polling (SSE when wired)"
  placeholder.

Verification:

- Deterministic component test: toggling live↔fixtures flips `mode`, drops/creates
  the tail subscription, and the auth-handle clear removes the in-memory token; the
  fail-loud non-loopback warning renders for a remote `baseUrl` AND for an
  uncertain/tunneled-to-localhost case (warning shown when uncertain), and the
  indicator is labelled advisory.
- `bun run build`, `bun run lint` clean.

Dependencies:

- Intra: WC0, WC1, WC2.
- Cross: connectivity-tunnel / the facade-owning track for an authoritative
  exposure value (WC7-DEP); until that exists the client uses the fail-loud
  `baseUrl` inference and records the open dependency.

## WC7-DEP - Facade Asks Recorded As A Cross-Workpad Dependency (NOT Built Here)

Status: pending (a recorded DEPENDENCY, not an implementation task).

Scope:

- Record — without writing any `crates/capo-web` code — the facade capabilities the
  remote-aware client would benefit from, as an explicit ask on the
  streaming-transport / harness track that OWNS `crates/capo-web` per
  `AGENTS.md:61` / `TASKS.md:11,63`. This replaces the draft's WC7, which would have
  had `web-console` mutate a crate another track owns and add a request-auth surface
  adjacent to the permission model — both forbidden by the boundary.

Acceptance criteria:

- Record the ASK, with the boundary rationale, in `knowledge.md` and
  `references.md`:
  - A liveness/exposure HINT (a tiny `GET /api/health`, or an `exposure` field on
    the `dashboard.project` block) reporting liveness, bound `addr`, and a
    `loopback`/`private`/`public` hint so the WC6 indicator can be authoritative
    instead of inferring from `baseUrl`.
  - Optional request authentication: if remote exposure needs access control, who
    may reach the endpoint is the CONNECTIVITY-TUNNEL track's concern
    (`runtime-tunnel.md` keeps `auth_ref`/`exposure`/endpoint resolution inside
    `ConnectivityTunnel`, separate from execution/controller state per the AGENTS.md
    safety boundary). Any bearer-token check in the facade is therefore an
    explicitly-authorized task for the facade/connectivity owners, NOT for
    `web-console`; if it ever lands as a stopgap, it must be scoped to a single
    env-configured shared-secret check and logged as a `WORKING.md` workaround with a
    follow-up review task. The client remains open-on-loopback and attaches a bearer
    header only when a token is resolved (WC1).
  - A CORS allowlist (`CAPO_WEB_ALLOW_ORIGIN`) for non-loopback origins. The
    existing `CorsLayer::permissive()` (`main.rs:125`) already satisfies the
    cross-origin FUNCTIONAL need; an allowlist is HARDENING owned by the facade
    track, not enablement owned here.
- Record that NONE of these are blockers for WC1–WC6: the client degrades to
  `baseUrl` inference (exposure), open-on-loopback (auth), and the existing
  permissive CORS (cross-origin) until the owning track lands them.

Verification:

- `knowledge.md` / `references.md` record the ask, the owner, and the
  client-side degradation path for each.
- No `crates/capo-web` change in this workpad.

Dependencies:

- Intra: WC0.
- Cross: the streaming-transport / harness track (owner of `crates/capo-web`) and
  the connectivity-tunnel track (owner of `auth_ref`/`exposure`/endpoint auth).

## WC8 - Green Gate + Live Remote Smoke (Opt-In) Paired With The Deterministic Halves

Status: pending.

Scope:

- Close the workpad on a fully green client build/lint/test gate plus an opt-in
  live remote smoke that is paired with the existing deterministic contract halves
  and skips cleanly when no server is reachable.

Acceptance criteria:

- The full CLIENT gate passes: `bun run build` (`tsc -b && vite build`) and `bun
  run lint` clean; the new WC1 endpoint/auth + leak-scan tests, the WC2 reconnect +
  contiguity + backoff tests, the WC3 projection + redaction tests, the WC4
  permission/empty-state tests, and the WC5 light+dark parity test all pass with no
  live server. The `crates/capo-web` cargo gate is the OWNING track's
  responsibility; this workpad relies on the published wire contract and the
  client compile against it.
- The deterministic e2e halves stay green and are the authoritative proof: the
  facade-owning track's server-side `http_facade_serves_the_live_chat_round_trip`
  test and the client-side compile against the shared `capo-wire` contract; plus
  the WC2/WC3/WC4/WC5 client tests above.
- An OPT-IN live remote smoke (documented runbook, env-gated, NOT part of the
  default gate): build the front-end, run the OWNING track's `capo-web`, point a
  second client/origin at it over a non-loopback endpoint (or loopback), send one
  real chat turn, drop the connection mid-turn, and confirm the tail RESUMES from
  `from_sequence` with no gap/duplicate — paired with the deterministic WC2
  reconnect+contiguity assertion that pins the same resume shape, so completion is
  never operator-attested alone. The smoke SKIPS cleanly when no server is
  reachable.
- Secrets stripped from all smoke evidence by a CONCRETE, FAIL-CLOSED credential
  scan: a script that greps every retained artifact (screenshots-as-text-where
  applicable, console logs, HAR/network captures) for the known test token plus
  common token shapes (e.g. `Bearer `, `sk-`, long base64/hex runs, `Authorization:`
  headers) and FAILS the smoke if any match is found. The same scan runs
  deterministically over a captured-log fixture as a unit so its detection is
  itself tested. No auth token, endpoint secret, or transcript-with-secrets appears
  in any retained artifact.
- A short review note in `knowledge.md`: client stays on the `InputSurface` side;
  no server loop/transport/permission/goal model and no `crates/capo-web`
  server-side code changed by this workpad; coordination points with the
  facade-owning track, connectivity-tunnel, and any parallel web agent recorded;
  decision on whether to deepen further or close `web-console`.

Verification:

- The client gate above, run and recorded.
- The opt-in live remote smoke runbook + its paired deterministic assertion + the
  fail-closed credential scan (run live and as a unit over a fixture).
- `git diff --check` clean.

Dependencies:

- Intra: WC1–WC6, WC7-DEP.
- Cross: the facade-owning track for the running `capo-web`; connectivity-tunnel if
  the live smoke uses a real tunnel endpoint (loopback is sufficient and
  tunnel-independent for the deterministic pairing).
