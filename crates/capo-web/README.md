# capo-web

An axum/tokio **HTTP + SSE facade** over the Capo server boundary. It is the
server-side half of the streaming-transport contract (ST8): it re-exposes the
typed `CapoServer` boundary as the four routes a browser console (`web/app`)
consumes, and re-publishes the live committed-event tail (ST4) as
Server-Sent Events using the **published wire contract**
(`crates/capo-server/contract/`).

It owns no orchestration state. Reads come from the same query layer the
dashboard uses (`capo-query`); mutations go through `CapoServer::handle`; the
live tail rides `CapoServer::subscribe`.

## Endpoints

| Method + path | Role |
| --- | --- |
| `GET  /api/dashboard` | Full live read model (agents, dispatch pipeline, lanes). The client boots from this. `project.mode = "live"`. |
| `POST /api/commands` | `send_task` / `steer_agent` / `interrupt_agent` / `stop_agent`. The reply carries the targeted `sessionId` so the client can read its thread and tail its streaming reply. |
| `GET  /api/thread?session=S&from=N` | The session's projected multi-turn conversation thread (ST5), incrementally from sequence `N`. The client renders it once, then extends it from the live tail at `nextSequence`. |
| `GET  /api/events?from=N&session=S` | The **event tail** (ST4/ST8): incremental, broadcast-backed `ServerEvent` frames. Each SSE block is the wire contract verbatim — `event: event` + a `data:` line carrying the JSON-RPC `event` notification. The streaming agent reply arrives here. |

Anything else is served from the built front-end (`CAPO_WEB_DIST`, default
`web/app/dist`).

## Run path (server, facade, browser)

capo-web runs an in-process `CapoServer`, so "start the capo server" and "start
capo-web" are the same process. The end-to-end live path is:

```sh
# 1. Build the front-end the facade serves.
cd web/app && bun install && bun run build        # -> web/app/dist

# 2. Start capo-web (the in-process Capo server + the HTTP/SSE facade).
#    From the repo root:
CAPO_STATE_ROOT=.capo-dev cargo run -p capo-web    # http://127.0.0.1:4177

# 3. Open the console.
open http://127.0.0.1:4177
```

The console auto-detects the facade: `GET /api/dashboard` answering flips it from
fixture mode to **live** mode, where it subscribes to `/api/events` and projects
each committed `CapoEvent` into the targeted agent's chat — the agent reply
*streams in* over the tail rather than being a fixture placeholder. With no
facade (e.g. the Vite dev server) the console stays on fixtures for offline dev.

Env: `CAPO_WEB_ADDR` (default `127.0.0.1:4177`), `CAPO_STATE_ROOT` (default
`.capo-dev`), `CAPO_WEB_DIST` (default `web/app/dist`).

## End-to-end check

The live path is gated by two deterministic halves, both run by the project
gate (`cargo test --workspace` + `web/app` `bun run build`):

- **Server side** — `http_facade_serves_the_live_chat_round_trip` (in
  `src/main.rs` tests) drives the **real axum router** through its full
  request/response stack (`tower::ServiceExt::oneshot`, no socket, no clock):
  `GET /api/dashboard` → `POST /api/commands(send_task)` → `GET /api/thread` →
  `GET /api/events`. It asserts the SSE response is `text/event-stream` and that
  the first frame is the published contract (`event: event` + a `data:` line
  whose JSON parses to an `EventNotification` carrying a `CapoEvent` strictly
  after the pre-turn watermark). The companion tests cover the incremental
  tail, the thread mapping, the targeted-session reply, and a byte-equal SSE
  wire-shape snapshot (`tests/snapshots/sse-event-tail.json`).
- **Client side** — `web/app` compiles against this contract: it consumes the
  SSE tail via the contract types in `web/app/src/data/capo-wire.ts` (copied
  from `crates/capo-server/contract/capo-wire.d.ts`) and the projection logic in
  `web/app/src/data/live.ts`. `bun run build` (`tsc -b && vite build`) proving
  it compiles is the client half of the e2e check.

The wire contract itself is owned by `crates/capo-server/contract/` and enforced
there by the `capo-server` `contract` test.
