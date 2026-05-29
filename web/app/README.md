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

```sh
cd web/app && bun run build               # produces web/app/dist
# from the repo root:
CAPO_STATE_ROOT=.capo-dev cargo run -p capo-web   # http://127.0.0.1:4177
```

`capo-web` (`crates/capo-web`) is an axum/tokio HTTP+SSE facade over an
in-process `CapoServer`. Env: `CAPO_WEB_ADDR` (default `127.0.0.1:4177`),
`CAPO_STATE_ROOT` (default `.capo-dev`), `CAPO_WEB_DIST` (default `web/app/dist`).

Endpoints: `GET /api/dashboard`, `POST /api/commands`
(`steer_agent`/`interrupt_agent`/`stop_agent`), `GET /api/events` (SSE).

## Screenshots

```sh
node web/dashboard/scripts/shoot.mjs --out /tmp/shots "http://127.0.0.1:5273/?theme=dark"
```

Renders any URL/HTML at desktop + mobile via headless Chrome (no extra deps).

## Status

Live mode powers Overview, the agent table, the dispatch pipeline, and agent
evidence ids from real server state. Activity, reviews/validations, goals, tool
catalog, permissions, and chat need new `ServerCommand`s and currently show
empty states in live mode (full data in fixture mode).
