/**
 * Capo JSON-RPC 2.0 + event-stream wire contract -- TypeScript types.
 *
 * OPTIONAL DOWNSTREAM CONVENIENCE, NOT THE CONTRACT.
 *
 * The authoritative contract is `jsonrpc-schema.json` (described) plus the
 * `snapshots/*.json` wire frames (enforced by the capo-server `contract` test).
 * These types are a hand-maintained convenience generated FROM that schema for
 * the web agent; they are owned web-side and are not the source of truth. If
 * they disagree with the schema/snapshots, the schema/snapshots win.
 *
 * Mirrors `crates/capo-server/src/transport/contract.rs` (v1).
 */

/** Every frame on the persistent connection (and inside an SSE data line). */
export type JsonRpcVersion = "2.0";

export type InputOrigin =
  | "cli"
  | "dashboard"
  | "mobile"
  | "voice"
  | "api"
  | "system";

/** The reserved origin object carried in every request's params. */
export interface Origin {
  client_id: string;
  actor_id: string;
  input_origin: InputOrigin;
}

/** The ServerCommand discriminant a request's `method` is one of. */
export type CommandMethod =
  | "register_agent"
  | "send_task"
  | "steer_agent"
  | "interrupt_agent"
  | "stop_agent"
  | "list_agents"
  | "agent_status"
  | "dashboard"
  | "start_session"
  | "replay_adapter_fixture"
  | "plan_dispatch"
  | "preflight_live_provider"
  | "gate_dispatch"
  | "run_dispatch_local"
  | "run_live_provider_local"
  | "recover"
  | "subscribe"
  | "read_thread";

/** Client -> server. `id` mirrors the request_id (idempotency key). */
export interface JsonRpcRequest {
  jsonrpc: JsonRpcVersion;
  id: string;
  method: CommandMethod;
  params: { origin: Origin } & Record<string, unknown>;
}

/** The tag on a successful response's `result.payload`. */
export type PayloadType =
  | "agent_registered"
  | "task_sent"
  | "agents"
  | "agent_status"
  | "dashboard"
  | "session_started"
  | "adapter_fixture_replayed"
  | "dispatch_planned"
  | "live_provider_preflighted"
  | "dispatch_gated"
  | "dispatch_run"
  | "recovery"
  | "subscribed"
  | "thread";

export interface JsonRpcSuccessResponse {
  jsonrpc: JsonRpcVersion;
  id: string;
  result: {
    client_id: string;
    actor_id: string;
    input_origin: InputOrigin;
    payload: { type: PayloadType } & Record<string, unknown>;
  };
}

/** Machine-readable Capo error kind clients branch on (`error.data.kind`). */
export type ErrorKind =
  | "io"
  | "json"
  | "protocol"
  | "state"
  | "adapter_fixture"
  | "unknown_agent"
  | "agent_has_no_active_session"
  | "agent_already_has_session"
  | "session_already_exists"
  | "run_already_exists"
  | "unknown_dispatch_plan"
  | "unknown_session"
  | "run_session_mismatch"
  | "adapter_session_mismatch"
  | "remote"
  | "cancelled"
  | "interrupted";

export interface JsonRpcErrorResponse {
  jsonrpc: JsonRpcVersion;
  /** `null` when the request could not be parsed (no recoverable id). */
  id: string | null;
  error: {
    /** Always JSON-RPC Internal error; the precise kind is in `data.kind`. */
    code: -32603;
    message: string;
    data: { kind: ErrorKind };
  };
}

/** One committed event: identical in a Subscribed backlog and a live `event`. */
export interface CapoEvent {
  /** Monotonic commit watermark; resume a Subscribe from here. */
  sequence: number;
  event_id: string;
  kind: string;
  actor: string;
  project_id: string | null;
  task_id: string | null;
  agent_id: string | null;
  session_id: string | null;
  run_id: string | null;
  turn_id: string | null;
  item_id: string | null;
  /** The event body, already redacted on egress (ST7). */
  payload_json: string;
  /** A withheld/sensitive body is downgraded to `redacted`. */
  redaction_state: string;
}

/** Server -> client: one committed event on the live tail (method `event`). */
export interface EventNotification {
  jsonrpc: JsonRpcVersion;
  method: "event";
  params: { event: CapoEvent };
}

/** Client -> server: abort one in-flight request by request_id. */
export interface CancelNotification {
  jsonrpc: JsonRpcVersion;
  method: "cancel";
  params: { request_id: string };
}

/** Client -> server: typed mid-turn interrupt of a session. */
export interface InterruptNotification {
  jsonrpc: JsonRpcVersion;
  method: "interrupt";
  params: { session_id: string; reason: string };
}

export type ServerNotification = EventNotification;
export type ClientNotification = CancelNotification | InterruptNotification;

/** The catch-up backlog returned by a `subscribe` request's `subscribed` payload. */
export interface SubscriptionBacklog {
  type: "subscribed";
  session_id: string | null;
  from_sequence: number;
  next_sequence: number;
  events: CapoEvent[];
}

/**
 * SSE re-exposure: each `event` notification is delivered as an
 * `event: event\ndata: <JsonRpcNotification frame>\n\n` block. The `data` line
 * is the verbatim JSON-RPC notification frame, so `JSON.parse(ev.data)` yields
 * an `EventNotification`.
 */
export const SSE_EVENT_NAME = "event" as const;
