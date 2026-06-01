# Capo wire contract (JSON-RPC 2.0 + event stream)

This directory is the **authoritative, web-client-independent contract** for the
Capo server transport: the JSON-RPC 2.0 request/response framing, the
server-initiated notification variant that carries the live event tail, the
client-initiated `cancel`/`interrupt` notifications, and the SSE re-exposure a
browser bridge consumes. It is the ST9 deliverable of the
`streaming-transport` workpad.

It is verified **without any web client**: every artifact here is produced by
serializing typed `ServerRequest` / `ServerResponse` / `ServerEvent` values
through the *same* codec the live transport uses
(`crates/capo-server/src/transport/contract.rs`), and a `cargo test` enforces
that the checked-in copies never drift.

## Files

| File | Role |
| --- | --- |
| `jsonrpc-schema.json` | The language-neutral schema (the *described* contract). |
| `snapshots/*.json` | Real serialized wire frames (the *enforced* contract). |
| `capo-wire.d.ts` | Optional TypeScript types generated FROM the schema. Not the contract. |

### Snapshots

Each `snapshots/<name>.json` is one exact frame plus a trailing newline:

- `request-*` -- client -> server JSON-RPC requests (the `id` mirrors the
  `request_id`; the command fields plus a reserved `origin` object are `params`).
- `response-agents`, `response-subscribed` -- successful JSON-RPC responses
  (`result.payload` is tagged by `type`).
- `response-error-cancelled` -- a JSON-RPC error frame: `error.code` is always
  `-32603` (Internal error) and the precise machine-readable kind is in
  `error.data.kind` (here `cancelled`).
- `notification-event-tail` -- the server-initiated `event` notification (no
  `id`); `params.event` is the shared committed-event shape.
- `notification-cancel`, `notification-interrupt` -- the client-initiated
  notifications (no `id`): request-id-scoped cancel and session-scoped typed
  mid-turn interrupt.
- `sse-event-tail` -- the SSE re-exposure: `event: event\ndata: <frame>\n\n`,
  where the `data` line is the verbatim JSON-RPC `event` notification.

## How a web agent adopts this

The web front-end (`web/app`, `web/dashboard`) is frozen and owned by a separate
agent; this contract is the seam between the two teams.

1. Open a `subscribe` request (`request-subscribe.json`) with `from_sequence`
   set to the last `sequence` you durably processed (or `0` to catch up fully).
2. The server replies with a `subscribed` backlog (`response-subscribed.json`):
   every committed event strictly after `from_sequence`, in order. Render it and
   advance your watermark to `next_sequence`.
3. Then consume live `event` notifications (`notification-event-tail.json`) on
   the same connection -- or, over the SSE bridge, the `sse-event-tail.json`
   block whose `data` line parses to the same notification. There is no gap and
   no duplicate at the backlog-to-live seam (the server seeds the live watermark
   from the backlog's `next_sequence`).
4. To abort, send a `cancel` notification (one in-flight request) or an
   `interrupt` notification (a session's mid-turn generation).

The event log stays authoritative: clients render the projected read models and
never author thread ordering. Web-side adoption is tracked as a web-agent task.

## Regenerating (intentional contract changes)

The fixtures are checked in. With no env var set (the default, including CI), the
`contract` test only *reads* them and asserts byte-equality, so an unintended
wire-shape change fails the build. To intentionally evolve the contract:

```sh
CAPO_REGENERATE_WIRE_SNAPSHOTS=1 cargo test -p capo-server --lib contract
```

then review the diff and commit. Run it again without the env var to confirm the
checked-in copies match.
