# Web Console References

## Objective

Record the in-repo files, modules, and docs the `web-console` workpad touches or
builds on, plus the cross-workpad contracts it consumes. The workpad owns only
`web/app` and CONSUMES the `crates/capo-web` facade (owned by the
streaming-transport / harness track per `AGENTS.md:61` / `TASKS.md:11,63`). Dated
facts reflect the tree as of 2026-06-02.

## Local Client Sources (web/app — OWNED by this workpad)

- `web/app/src/data/store.tsx`
  - Key facts: the `StoreProvider` is the live/fixture switch. It auto-detects the
    facade (`fetch('/api/dashboard')`), flips `liveRef`, re-polls every 4s with a
    `keepChats` guard, and subscribes via a bare `new EventSource('/api/events')`
    (no `from` watermark, no app-controlled reconnect). `seenEventIds` dedupes by
    event id; `chatKeyForEvent` resolves an event to an agent by `session_id` then
    `agent-<name>`. WC1 adds the endpoint/auth helper here; WC2 adds the contiguous
    watermark + app-controlled reconnect-with-resume; WC6 drives connect/disconnect
    from Settings.
- `web/app/src/data/live.ts`
  - Key facts: `fetchThread` (`/api/thread?session=&from=`), `threadToChatMessages`,
    `eventToChatMessage`, `parseEventFrame` (parses the `event: event` SSE data line
    into a `CapoEvent`), and `classifyEventKind`
    (`session.summary_updated`→agent, `tool.*`→tool, terminal kinds→system). Flat
    message projection today (surfaces `redactionState`, ~line 28); WC3 adds turn
    grouping + streaming indicator + redaction-aware rendering.
- `web/app/src/data/capo-wire.ts`
  - Key facts: the client copy of the published wire contract
    (`crates/capo-server/contract/capo-wire.d.ts`): `CapoEvent`,
    `EventNotification`, `SSE_EVENT_NAME`. The client compiles against this; the
    `bun run build` is the client half of the e2e gate. The wire carries
    `redaction_state` (all checked-in snapshots are `"safe"`, so WC3 ships a
    non-safe fixture).
- `web/app/src/data/{fixtures.ts,types.ts}`
  - Key facts: `fixtureData` (the offline `ConsoleData`) and the `ConsoleData` /
    `ChatMessage` / agent / permission / tool types. The offline fallback (WC1) and
    the honest-empty-state contrast (WC4/WC5) build on these. WC3's non-safe
    redaction fixture lands here.
- `web/app/src/screens/*` (`Overview`, `Agents`, `Chat`, `Goals`, `Tools`,
  `Activity`, `Settings`, `Placeholder`)
  - Key facts: the seven screens (+ `Placeholder`). `Settings.tsx` has the disabled
    "Live server — coming next" toggle + "polling (SSE when wired)" placeholder (WC6
    replaces). `Chat.tsx` hosts the chat surface + `PermissionCard` (WC3/WC4).
    `Agents.tsx` hosts the dispatch pipeline + steer/interrupt/stop (WC5). WC5
    removes/restricts `Placeholder` so NavRail exposes exactly seven.
- `web/app/src/components/*` (`AppShell`, `NavRail`, `Topbar`, `StatusBar`,
  `CommandPalette`, `DispatchPipeline`, `PermissionCard`, `ui.tsx`, `nav.ts`)
  - Key facts: shared shell (NavRail, ⌘K palette, IDE status bar). `StatusBar`
    surfaces connection state (WC2 "reconnecting…", WC6 advisory exposure warning).
    `PermissionCard` is the inline permission UI (WC4). `nav.ts` defines the seven
    nav entries.
- `web/app/src/lib/theme.tsx`
  - Key facts: light/dark theme provider; `?theme=light|dark` forces a theme. WC5
    verifies light+dark parity through it as a computed-style assertion (no
    token-bypassing hard-coded colors), not a screenshot.
- `web/app/package.json`, `eslint.config.js`, `vite.config.ts`, `tsconfig*.json`
  - Key facts: `bun run build` = `tsc -b && vite build`; `bun run lint` = eslint.
    These are the client gate (WC8).
- `web/app/README.md`
  - Key facts: documents fixture dev (`bun dev`), the live run path, the endpoint
    table, and the current "how live mode consumes the tail" behavior + status (chat
    live; goals/tools/permissions empty in live). Updated by WC1/WC6/WC8.

## Local Facade + Tooling Sources (CONSUMED — NOT modified by this workpad)

- `crates/capo-web/src/main.rs` (owned by the streaming-transport / harness track)
  - Key facts: the axum facade. `build_router` wires `GET /api/dashboard`,
    `POST /api/commands`, `GET /api/thread`, `GET /api/events`, a static fallback,
    and `CorsLayer::permissive()` (`main.rs:125` — cross-origin already works).
    `events()` / `run_event_tail` implement the `from`-resume tail (default "tail
    from now" via `last_sequence()`), the broadcast-hub + cross-process catch-up
    dedupe against `delivered_through`, and `KeepAlive::default()`. `commands()`
    surfaces the targeted `sessionId`. The `dashboard.project` block
    (`main.rs:660-667`) reports `addr` + hardcoded `"mode": "live"` and NO exposure
    field; live mode returns `goals: []`/`permissions: []`/`tools: []`
    (`main.rs:675-677`). Tests include `http_facade_serves_the_live_chat_round_trip`
    and `events_stream_surfaces_incremental_event_without_repoll`. The
    `web-console` workpad does NOT edit this file; the `/api/health` + exposure hint,
    optional auth, and CORS allowlist are recorded as WC7-DEP asks on the owning
    track.
- `crates/capo-web/README.md` (owned by the facade track)
  - Key facts: the endpoint table, the run path, the "owns no orchestration state"
    statement, and the env vars (`CAPO_WEB_ADDR`, `CAPO_STATE_ROOT`, `CAPO_WEB_DIST`).
    A `CAPO_WEB_ALLOW_ORIGIN` allowlist + any auth would be documented here BY THE
    OWNING TRACK if/when WC7-DEP lands.
- `crates/capo-web/tests/snapshots/sse-event-tail.json` (owned by the facade track)
  - Key facts: the regenerate-and-diff SSE wire-shape fixture; this workpad consumes
    its shape and never drifts it.
- `web/dashboard/scripts/shoot.mjs`
  - Key facts: headless-Chrome screenshot tool (desktop + mobile, any URL/HTML)
    targeting the LEGACY `web/dashboard` tree. WC5's primary parity proof is a
    computed-style test, not a screenshot; if screenshots are produced as
    supplementary evidence, WC5 FIRST verifies (and records here) whether `shoot.mjs`
    can drive the `web/app` Vite dev server at `?theme=`, and adds a `web/app`-local
    shoot script if it cannot. (Not yet verified — flagged as a WC5 sub-task.)

## Cross-Workpad Contracts Consumed (Not Modified)

- `crates/capo-server/contract/` (the published wire contract: `capo-wire.d.ts`,
  `sse_frame`, `EventNotification`, `ServerEvent`)
  - Key facts: the JSON-RPC `event` notification shape the SSE tail emits and the
    client parses; carries `redaction_state`. The client compiles against the `.d.ts`
    copy; the server test pins the byte-shape. Owned by streaming-transport; consumed
    here.
- `crates/capo-server` (`CapoServer::handle`, `CapoServer::subscribe`,
  `ServerCommand`, `ServerThread`, the ST5 thread projection)
  - Key facts: the typed server boundary the facade adapts. `Subscribe { session,
    from_sequence }` is the resume contract WC2 relies on. Owned by
    server/streaming-transport; consumed here.
- `workpads/architecture/boundaries.md`
  - Key facts: the `InputSurface` contract — surfaces submit `CommandEnvelope`s and
    render read models; they do NOT own session truth, hold runtime handles, mutate
    state without the controller, or bypass policy. Failure modes include duplicate
    command submission after reconnect and stale UI approving an old permission —
    both directly relevant to WC2 (resume dedupe) and WC4 (permission cards).
- `workpads/architecture/runtime-tunnel.md`
  - Key facts: `ConnectivityTunnel { LocalLoopback, Ssh, Tailscale, Reverse, Fake }`;
    `ConnectivityEndpoint.exposure` ∈ `loopback`/`private`/`public`;
    `ConnectivityEndpoint.auth_ref` points to a secret HANDLE, never raw material;
    `ResolvedEndpoint` with `channel_kind = dashboard` is the owner shape for a web
    surface; `auth_ref`/`exposure`/endpoint resolution live INSIDE the tunnel,
    separate from execution/controller state. This is the endpoint/exposure/auth
    VOCABULARY the remote-aware client (WC1/WC6) consumes, and the home of any
    who-may-reach-the-endpoint AUTH decision (WC7-DEP) — the tunnel and the facade
    auth are NOT this workpad's to implement.
- `crates/capo-cli/src/connectivity.rs`
  - Key facts: today a stub (`expose_connectivity_stub`) that resolves a
    loopback/private/public endpoint through `ConnectivityTunnel` and reports
    `blocked_pending_permission` for exposures requiring permission. It models the
    exposure/permission vocabulary the client surfaces in Settings (WC6); the client
    does not call it.

## Ownership / Boundary Sources (the BLOCKER resolution)

- `AGENTS.md` (line 61) + `TASKS.md` (lines 11, 29, 34, 63)
  - Key facts: "The web UI (`web/app`, `web/dashboard`) is owned by a separate agent
    and is out of scope for the harness track; those workpads deliver only the
    server-side streaming contract (evolving `crates/capo-web`)." This is the source
    of truth that assigns server-side `crates/capo-web` to the streaming-transport /
    harness track and the browser client to a separate (web) agent. `web-console` is
    that web-client workpad; it owns `web/app` only and records facade asks as
    WC7-DEP. `TASKS.md:34` (streaming-transport) explicitly "evolves
    `crates/capo-web`; does not build the web client," and `TASKS.md:63` documents
    the Dashboard-poll → Subscribe migration handoff for the web agent.
- `AGENTS.md` — Safety Boundary section
  - Key facts: never log API keys / subscription tokens / OAuth tokens / cookies /
    session files / transcripts-with-secrets; treat subscription-backed agent access
    as a privileged connector; keep tunnel/connectivity concerns SEPARATE from agent
    execution and controller state; make remote-control capabilities auditable and
    revocable. This is the first-class acceptance criterion threaded through WC0, WC1
    (in-memory token + leak-scan test), WC6 (revocable, fail-loud exposure), and WC8
    (fail-closed credential scan); it is ALSO the reason facade request-auth is NOT
    pulled into this client workpad (WC7-DEP).

## External References

- Server-Sent Events (`EventSource`) / WHATWG HTML living standard
  - Key facts: `EventSource` auto-reconnects but the browser default does NOT resume
    by application sequence, and does NOT let the application carry a `from` watermark
    into its built-in retry — so WC2 closes the native source on error and
    constructs a new one at `from=<watermark>` under app-controlled backoff.
    Separately, SSE comment frames (`: keep-alive`) are NOT surfaced to application
    code (they keep the socket warm, fire no event/error), so the client cannot and
    need not treat a keep-alive gap as a disconnect — WC2 only asserts (via mock)
    that a comment-only idle window does not trigger the app reconnect path.
  - To verify both behaviors against the current standard at implementation time (do
    not assume).
