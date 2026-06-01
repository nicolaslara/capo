# Capo Operator Console

A production-grade web console for supervising Capo's coding agents: a dashboard
plus full **inspect**, **control**, and **chat** surfaces. Terminal-native
aesthetic in light **and** dark.

Stack: Vite + React + TypeScript + Tailwind v4 + Radix + cmdk (bun).
Designed via the `dashboard-design-iteration` workflow; see
`../../workpads/dashboard-webclient/`.

## Screens

Overview · Agents (inspect + control, dispatch pipeline, steer/interrupt/stop) ·
Chat console (inline permission cards) · Goals · Activity/Events ·
Tools/Permissions · Settings. Shared NavRail, ⌘K command palette, IDE status bar.

## Develop (fixtures)

```sh
cd web/app
bun install
bun dev          # http://127.0.0.1:5273  (fixture data, mocked commands)
```

The app auto-detects the live facade at `/api/dashboard`. With the Vite dev
server there's no facade, so it runs on fixtures. Toggle light/dark from the
top-right; `?theme=light|dark` forces a theme (used for deterministic shots).

## Run live (wired to capo-server)

`capo-web` runs an in-process `CapoServer`, so starting the capo server and
starting capo-web are one step. End to end:

```sh
# 1. Build the front-end the facade serves.
cd web/app && bun install && bun run build         # produces web/app/dist

# 2. Start the capo server + HTTP/SSE facade (from the repo root):
CAPO_STATE_ROOT=.capo-dev cargo run -p capo-web    # http://127.0.0.1:4177

# 3. Open the console.
open http://127.0.0.1:4177
```

`capo-web` (`crates/capo-web`) is an axum/tokio HTTP+SSE facade over an
in-process `CapoServer`. Env: `CAPO_WEB_ADDR` (default `127.0.0.1:4177`),
`CAPO_STATE_ROOT` (default `.capo-dev`), `CAPO_WEB_DIST` (default `web/app/dist`).
See `crates/capo-web/README.md` for the endpoint table and the e2e check.

Endpoints: `GET /api/dashboard`, `POST /api/commands`
(`send_task`/`steer_agent`/`interrupt_agent`/`stop_agent`, reply carries the
targeted `sessionId`), `GET /api/thread?session=S&from=N` (the projected
multi-turn thread, ST5), `GET /api/events?from=N&session=S` (the SSE event tail,
ST4).

### How live mode consumes the tail

The app auto-detects the facade (`GET /api/dashboard`) and flips to **live**
mode. It then:

- subscribes to `/api/events` and listens for the contract `event: event` SSE
  frames (not `onmessage`), parsing each `data:` line into a `CapoEvent` via the
  contract types in `src/data/capo-wire.ts` (copied from
  `crates/capo-server/contract/capo-wire.d.ts`);
- projects each committed event into the targeted agent's chat with
  `src/data/live.ts` — so the agent reply **streams in over the tail** rather
  than being a fixture placeholder;
- on a `send_task`/`steer_agent` reply, reads the targeted session's thread once
  (`GET /api/thread`) for history, deduped against the live tail by `event_id`;
- re-polls `GET /api/dashboard` on a slow timer to keep the agent table / lanes
  fresh (the conversation is event-tail driven, not poll driven).

With no facade (e.g. the Vite dev server) it stays on fixtures for offline dev.

## Screenshots

```sh
node web/dashboard/scripts/shoot.mjs --out /tmp/shots "http://127.0.0.1:5273/?theme=dark"
```

Renders any URL/HTML at desktop + mobile via headless Chrome (no extra deps).

## End-to-end check

The live web ↔ capo-web ↔ capo-server path is gated by two deterministic halves:

- **Server side** — `crates/capo-web` test
  `http_facade_serves_the_live_chat_round_trip` drives the real router
  (dashboard → command → thread → SSE events) and asserts the tail emits the
  published contract frame. Runs under `cargo test --workspace`.
- **Client side** — this app compiles against the same contract; `bun run build`
  (`tsc -b && vite build`) passing is the client half.

## Status

Live mode powers Overview, the agent table, the dispatch pipeline, agent
evidence ids, **and the chat surface** (the streamed agent reply + the projected
thread) from real server state over the event tail. Reviews/validations, goals,
tool catalog, and permissions need new `ServerCommand`s / projections and
currently show empty states in live mode (full data in fixture mode).
