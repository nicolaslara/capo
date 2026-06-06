//! SLICE-A / Layer 1: the in-process STATELESS HTTP MCP server that exposes
//! "capo tools" (start_agent, list_agents, review_agent, steer_agent, set_mode)
//! to a claude-code-acp conductor session.
//!
//! Per DESIGN-A: a small `axum` router with a single `POST /mcp` JSON-RPC route
//! and a `GET /mcp` -> 405 (the empirically-required statelessness switch — a
//! session-id-issuing transport makes claude-code-acp's MCP client report
//! "Failed to connect"). The server NEVER issues an `Mcp-Session-Id`. Each tool
//! call dispatches in-process to `CapoServer::handle` (the same entrypoint the
//! smoke tests use). The `start_agent` tool reproduces the proven 3-step worker
//! drive (RegisterAgent -> StartSession -> RunAcpLiveTurnLocal) — the L1 -> L2
//! hop.
//!
//! This module is transport-only and deterministic-testable via
//! `tower::ServiceExt::oneshot` (no socket). The live wiring (forwarding the
//! URL through `session/new`'s `mcpServers`) is Layer 2.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::{
    Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use serde_json::{Value, json};

use crate::{CapoServer, ServerCommand, ServerRequest, ServerResponsePayload};

/// The worker-turn dispatch config a `start_agent` call uses to drive an ACP
/// worker turn. In Layer-1 tests this points at a deterministic `/bin/sh` ACP
/// stub; in ops it is `npx -y @zed-industries/claude-code-acp`.
#[derive(Clone, Debug)]
pub struct AcpWorkerToolConfig {
    /// The ACP agent program the worker turn spawns.
    pub acp_program: String,
    /// Args for that program.
    pub acp_argv: Vec<String>,
    /// The default confined workspace the worker turn runs in when the tool call
    /// carries no `worktree`. `None` lets the server use its project-dir default.
    pub default_workspace_root: Option<String>,
    /// The ACP session mode the worker turn switches to before prompting. The
    /// deterministic stub path uses `None`; the live bridge uses
    /// `Some("bypassPermissions")` so its Write tool round-trips an on-wire write.
    pub acp_session_mode: Option<String>,
}

/// Conductor-session-local routing state (`set_mode`). There is no server
/// command for interaction scope; it lives here, owned by the MCP listener for
/// the duration of a conductor turn.
#[derive(Clone, Debug, Default)]
struct ConductorMode {
    /// `"one"` | `"all"`; `None` until the conductor sets it.
    scope: Option<String>,
    /// The target agent when `scope == "one"`.
    agent_id: Option<String>,
}

/// The shared state behind the router: the in-process `CapoServer` tool calls
/// dispatch into, the worker-turn config, the bearer token, and the
/// conductor-local mode.
#[derive(Clone)]
pub struct McpState {
    server: CapoServer,
    worker: AcpWorkerToolConfig,
    /// Defense-in-depth bearer token; every request must carry it. Empty string
    /// disables the check (Layer-1 oneshot tests may omit auth).
    bearer_token: String,
    mode: Arc<Mutex<ConductorMode>>,
    /// Observable invocation log of `tools/call` dispatches. The Layer-3 live E2E
    /// reads this (out of band) to prove the conductor actually CALLED a capo
    /// tool (e.g. `start_agent`) over the localhost MCP endpoint.
    invocations: Arc<Mutex<Vec<ToolInvocation>>>,
    /// The conductor's long-lived session/run identity. Each `tools/call`
    /// dispatch emits a capo SESSION EVENT tagged to these ids (F1) so the
    /// conductor's tool activity surfaces on `/api/events` + the chat feed.
    conductor_session_id: String,
    conductor_run_id: String,
    /// Structured worker results reported via `report_result(key, value)`. Keyed
    /// by the same `key` the conductor passes to `collect_results` (typically the
    /// result filename), so a reported value takes precedence over reading the
    /// file. Shared across `CapoServer`/`McpState` clones (incl. detached worker
    /// threads) because all sessions hit this one in-process MCP endpoint.
    reports: Arc<Mutex<HashMap<String, String>>>,
    /// When set, `start_agent` forwards this MCP endpoint (+ `worker_mcp_headers`
    /// bearer) into each WORKER's `session/new`, so workers can call
    /// `report_result`. `None` (the default) advertises NO MCP server to workers,
    /// keeping the validated file-only worker loop byte-identical.
    worker_mcp_url: Option<String>,
    worker_mcp_headers: Vec<(String, String)>,
}

/// One observed `tools/call` dispatch through the in-process MCP server.
#[derive(Clone, Debug)]
pub struct ToolInvocation {
    /// The tool name (`start_agent`, `list_agents`, ...).
    pub name: String,
    /// The raw `arguments` object the conductor passed.
    pub arguments: Value,
    /// Whether the dispatch reported `isError:true`.
    pub is_error: bool,
}

impl McpState {
    pub fn new(server: CapoServer, worker: AcpWorkerToolConfig, bearer_token: String) -> Self {
        Self {
            server,
            worker,
            bearer_token,
            mode: Arc::new(Mutex::new(ConductorMode::default())),
            invocations: Arc::new(Mutex::new(Vec::new())),
            // Default to the capo-web conductor identity (the single live
            // consumer). `with_conductor_identity` overrides this when the
            // caller knows the bootstrap-assigned ids.
            conductor_session_id: "session-conductor-web".to_string(),
            conductor_run_id: "run-conductor-web".to_string(),
            reports: Arc::new(Mutex::new(HashMap::new())),
            worker_mcp_url: None,
            worker_mcp_headers: Vec::new(),
        }
    }

    /// Forward this MCP endpoint into each worker's `session/new` so workers can
    /// call `report_result`. Pass capo's own in-process `/mcp` URL + the bearer
    /// header. Leaving this unset keeps the validated file-only worker loop
    /// byte-identical (no MCP server advertised to workers).
    pub fn with_worker_mcp(
        mut self,
        worker_mcp_url: impl Into<String>,
        worker_mcp_headers: Vec<(String, String)>,
    ) -> Self {
        self.worker_mcp_url = Some(worker_mcp_url.into());
        self.worker_mcp_headers = worker_mcp_headers;
        self
    }

    /// Override the conductor session/run identity emitted tool events are
    /// tagged with (F1). Use the ids from `bootstrap_conductor` as the single
    /// source of truth.
    pub fn with_conductor_identity(
        mut self,
        conductor_session_id: impl Into<String>,
        conductor_run_id: impl Into<String>,
    ) -> Self {
        self.conductor_session_id = conductor_session_id.into();
        self.conductor_run_id = conductor_run_id.into();
        self
    }

    /// A cloneable handle to the invocation log so a test (or operator) can
    /// observe tool calls dispatched through this server out of band.
    pub fn invocation_log(&self) -> Arc<Mutex<Vec<ToolInvocation>>> {
        Arc::clone(&self.invocations)
    }
}

/// Build the stateless MCP router. `POST /mcp` handles JSON-RPC; `GET /mcp`
/// returns 405 (no server-initiated SSE stream — the statelessness switch).
pub fn router(state: McpState) -> Router {
    Router::new()
        .route("/mcp", post(handle_post).get(handle_get))
        .with_state(state)
}

/// `GET /mcp` -> 405. This is the exact switch that flips claude-code-acp's MCP
/// client from "Failed to connect" to "✓ Connected".
async fn handle_get() -> Response {
    StatusCode::METHOD_NOT_ALLOWED.into_response()
}

/// The protocol version capo answers `initialize` with unless the client
/// offered one we recognize, in which case we echo it.
const DEFAULT_PROTOCOL_VERSION: &str = "2025-06-18";
const SUPPORTED_PROTOCOL_VERSIONS: [&str; 3] = ["2025-03-26", "2025-06-18", "2025-11-25"];

/// `POST /mcp` — one JSON-RPC message per request. A request (`id` present)
/// gets a JSON-RPC response with HTTP 200 + `application/json`; a notification
/// (no `id`) gets HTTP 202 with empty body.
async fn handle_post(
    State(state): State<McpState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    // Bearer auth (defense-in-depth; loopback-only endpoint). Skipped when the
    // configured token is empty.
    if !state.bearer_token.is_empty() {
        let ok = headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .map(|v| v == format!("Bearer {}", state.bearer_token))
            .unwrap_or(false);
        if !ok {
            return StatusCode::UNAUTHORIZED.into_response();
        }
    }

    let request: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => {
            return json_response(error_response(
                Value::Null,
                -32700,
                "Parse error: invalid JSON",
            ));
        }
    };

    let id = request.get("id").cloned();
    let method = request.get("method").and_then(Value::as_str).unwrap_or("");
    let params = request.get("params").cloned().unwrap_or(Value::Null);

    // Notifications carry no `id` -> 202, no JSON-RPC response.
    if id.is_none() {
        // e.g. notifications/initialized
        return StatusCode::ACCEPTED.into_response();
    }
    let id = id.unwrap();

    match method {
        "initialize" => json_response(handle_initialize(id, &params)),
        "tools/list" => json_response(handle_tools_list(id)),
        "tools/call" => json_response(handle_tools_call(&state, id, &params)),
        other => json_response(error_response(
            id,
            -32601,
            &format!("Method not found: {other}"),
        )),
    }
}

fn handle_initialize(id: Value, params: &Value) -> Value {
    let offered = params.get("protocolVersion").and_then(Value::as_str);
    let version = match offered {
        Some(v) if SUPPORTED_PROTOCOL_VERSIONS.contains(&v) => v,
        _ => DEFAULT_PROTOCOL_VERSION,
    };
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "protocolVersion": version,
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": { "name": "capo-conductor-tools", "version": "0.1.0" }
        }
    })
}

fn handle_tools_list(id: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": { "tools": tool_schemas() }
    })
}

/// The five capo tools advertised to the conductor (DESIGN-A §3).
fn tool_schemas() -> Value {
    json!([
        {
            "name": "start_agent",
            "description": "Register a worker agent, start its acp session, and drive one live ACP worker turn on the project (or a worktree). Returns the worker's ids and turn outcome. Long-running.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "task": {"type": "string", "description": "The goal/instruction for the worker agent."},
                    "worktree": {"type": "string", "description": "Optional absolute path to a confined worktree to run in; defaults to the project dir."},
                    "name": {"type": "string", "description": "Optional worker name; auto-derived from the task if omitted."},
                    "detached": {"type": "boolean", "description": "When true, start the worker and return immediately (status:running) instead of blocking on its turn, so you (the conductor) stay responsive; poll progress with review_agent/list_agents. Default false."}
                },
                "required": ["task"]
            }
        },
        {
            "name": "capo_read",
            "description": "Read a file from the confined workspace via capo's supervised file tool. Use this INSTEAD of a native Read tool (which is unavailable in a locked-down session).",
            "inputSchema": {
                "type": "object",
                "properties": {"path": {"type": "string", "description": "Path to read (confined to the workspace)."}},
                "required": ["path"]
            }
        },
        {
            "name": "capo_write",
            "description": "Write a file in the confined workspace via capo's supervised file tool. Use this INSTEAD of a native Write/Edit tool (which is unavailable in a locked-down session).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Path to write (confined to the workspace)."},
                    "content": {"type": "string", "description": "The new file contents."}
                },
                "required": ["path", "content"]
            }
        },
        {
            "name": "capo_bash",
            "description": "Run a shell command in the confined workspace via capo's supervised shell tool. Use this INSTEAD of a native Bash tool (which is unavailable in a locked-down session).",
            "inputSchema": {
                "type": "object",
                "properties": {"command": {"type": "string", "description": "The shell command line to run."}},
                "required": ["command"]
            }
        },
        {
            "name": "capo_search",
            "description": "Search the confined workspace via capo's supervised search tool. Use this INSTEAD of a native Grep/Glob tool (which is unavailable in a locked-down session).",
            "inputSchema": {
                "type": "object",
                "properties": {"query": {"type": "string", "description": "The search query (ripgrep pattern)."}},
                "required": ["query"]
            }
        },
        {
            "name": "list_agents",
            "description": "List all worker agents capo is managing, with their session/run status.",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false}
        },
        {
            "name": "review_agent",
            "description": "Read one worker agent's current state and recent conversation thread.",
            "inputSchema": {
                "type": "object",
                "properties": {"agent_id": {"type": "string", "description": "The worker's id or name."}},
                "required": ["agent_id"]
            }
        },
        {
            "name": "steer_agent",
            "description": "Send a steering instruction to a running worker agent.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "agent_id": {"type": "string"},
                    "msg": {"type": "string", "description": "The steering instruction."}
                },
                "required": ["agent_id", "msg"]
            }
        },
        {
            "name": "set_mode",
            "description": "Set the conductor's interaction scope: talk to one specific agent or to all agents.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "scope": {"type": "string", "enum": ["one", "all"]},
                    "agent_id": {"type": "string", "description": "Required when scope=one."}
                },
                "required": ["scope"]
            }
        },
        {
            "name": "report_result",
            "description": "Return your final structured result to the conductor. Call this ONCE when your task is done, with `key` set to the exact result identifier the conductor gave you (usually the result filename, e.g. \"result-fruit-1.txt\") and `value` set to your result. This is the PRIMARY way to hand a result back — it is read directly by the conductor's collect_results, so it is preferred over (and need not duplicate) writing a result file.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "key": {"type": "string", "description": "The result identifier the conductor assigned (typically the result filename you were told to use)."},
                    "value": {"type": "string", "description": "Your final result/answer."}
                },
                "required": ["key", "value"]
            }
        },
        {
            "name": "collect_results",
            "description": "Wait for and read worker results. For each key/filename, returns a REPORTED value (if a worker called report_result with that key) in preference to reading the file; otherwise BLOCKS server-side (up to timeout_secs) until the file has non-empty content. Use this to aggregate fan-out results instead of reading them yourself. The returned `results` are ground truth — never invent worker results.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "files": {"type": "array", "items": {"type": "string"}, "description": "Relative result filenames the workers write, e.g. [\"result-fruit-1.txt\",\"result-fruit-2.txt\"]."},
                    "timeout_secs": {"type": "integer", "description": "Max seconds to wait for all files to be non-empty (default 75)."}
                },
                "required": ["files"]
            }
        }
    ])
}

fn handle_tools_call(state: &McpState, id: Value, params: &Value) -> Value {
    let name = match params.get("name").and_then(Value::as_str) {
        Some(n) => n,
        None => return error_response(id, -32602, "Invalid params: missing tool name"),
    };
    let args = params.get("arguments").cloned().unwrap_or(json!({}));

    let outcome = match name {
        "start_agent" => tool_start_agent(state, &args),
        "capo_read" => tool_capo_read(state, &args),
        "capo_write" => tool_capo_write(state, &args),
        "capo_bash" => tool_capo_bash(state, &args),
        "capo_search" => tool_capo_search(state, &args),
        "list_agents" => tool_list_agents(state),
        "review_agent" => tool_review_agent(state, &args),
        "steer_agent" => tool_steer_agent(state, &args),
        "set_mode" => tool_set_mode(state, &args),
        "report_result" => tool_report_result(state, &args),
        "collect_results" => tool_collect_results(state, &args),
        other => Err(format!("unknown tool: {other}")),
    };

    // Record the dispatch in the observable invocation log (Layer-3 proof seam).
    if let Ok(mut log) = state.invocations.lock() {
        log.push(ToolInvocation {
            name: name.to_string(),
            arguments: args.clone(),
            is_error: outcome.is_err(),
        });
    }

    // F1: emit a capo SESSION EVENT for this tools/call, tagged to the
    // conductor's session/run, so the conductor's tool activity surfaces on
    // `/api/events` + the chat feed as `→ <name>(args)`. Best-effort: a store
    // error must never break the tool reply.
    emit_conductor_tool_event(state, name, &args, outcome.is_err());

    match outcome {
        Ok(text) => tool_result(id, &text, false),
        // MCP convention: tool-execution failures are reported in-result with
        // isError:true, NOT as a JSON-RPC error.
        Err(message) => tool_result(id, &message, true),
    }
}

/// Allowlisted `arguments` keys that are safe to surface in a tool-event
/// payload. Anything outside this set (e.g. arbitrary/secret args) is dropped,
/// keeping credential redaction intact. The capo tools' args (task/path/...)
/// are not credential-shaped, but the allowlist + length cap is defense-in-depth.
const TOOL_ARG_ALLOWLIST: &[&str] = &[
    "task", "goal", "path", "query", "command", "name", "agent_id", "mode", "detached", "worktree",
];

/// F1: append a `tool.call_requested` SESSION EVENT for one conductor `tools/call`
/// dispatch. Tagged to the conductor's session/run + actor `agent-conductor` so
/// the chat feed labels it "conductor" and renders `→ <name>(args)` via
/// `legibleLine`. Best-effort — any store error is swallowed so the tool reply
/// is never broken.
fn emit_conductor_tool_event(state: &McpState, name: &str, args: &Value, is_error: bool) {
    use capo_core::{RunId, SessionId};
    use capo_state::{EventKind, NewEvent, RedactionState};

    let mut payload = serde_json::Map::new();
    payload.insert("tool_name".to_string(), json!(name));
    payload.insert(
        "status".to_string(),
        json!(if is_error { "failed" } else { "completed" }),
    );
    payload.insert("source".to_string(), json!("conductor_mcp"));
    // Surface only allowlisted args, clipped — never dump arbitrary args.
    if let Some(obj) = args.as_object() {
        for key in TOOL_ARG_ALLOWLIST {
            if let Some(v) = obj.get(*key) {
                let clipped = match v {
                    Value::String(s) => json!(s.chars().take(120).collect::<String>()),
                    other => other.clone(),
                };
                payload.insert((*key).to_string(), clipped);
            }
        }
    }

    let event_id = format!(
        "event-conductor-tool-{}",
        crate::util::stable_hash(format!("{name}:{args}").as_bytes())
    );
    let mut ev = NewEvent::new(event_id, EventKind::ToolCallRequested, "agent-conductor");
    ev.session_id = Some(SessionId::new(state.conductor_session_id.clone()));
    ev.run_id = Some(RunId::new(state.conductor_run_id.clone()));
    ev.payload_json = Value::Object(payload).to_string();
    ev.redaction_state = RedactionState::Safe;
    let _ = state.server.append_event(ev, &[]);
}

/// `start_agent(task, worktree?, name?)` — the L1 -> L2 hop: RegisterAgent ->
/// StartSession -> RunAcpLiveTurnLocal, reproducing the 3-step proven worker
/// drive.
fn tool_start_agent(state: &McpState, args: &Value) -> Result<String, String> {
    let task = args
        .get("task")
        .and_then(Value::as_str)
        .ok_or("start_agent requires `task`")?;
    let worktree = args.get("worktree").and_then(Value::as_str);
    // Depth discipline: when detached, the conductor (L1) is NOT blocked on the
    // worker turn (L2) — start_agent returns immediately with status:running.
    let detached = args.get("detached").and_then(Value::as_bool).unwrap_or(false);
    let name = args
        .get("name")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| derive_agent_name(task));

    // Step 1: register (skip if it already exists — register is name-keyed).
    let reg = state.server.handle(ServerRequest::cli(ServerCommand::RegisterAgent {
        name: name.clone(),
        adapter: "acp".to_string(),
    }));
    // If the agent already exists, registration may error; tolerate that and
    // continue to start a fresh session.
    let agent_id = match reg {
        Ok(resp) => match resp.payload {
            ServerResponsePayload::AgentRegistered(summary) => summary.agent_id.to_string(),
            other => return Err(format!("unexpected RegisterAgent payload: {other:?}")),
        },
        Err(_) => format!("agent-{name}"),
    };

    // Step 2: start the session + run (fresh ids per call).
    let suffix = crate::util::stable_hash(format!("{name}:{task}").as_bytes());
    let session_id = format!("session-conductor-{}-{suffix}", crate::util::slug(&name));
    let run_id = format!("run-conductor-{}-{suffix}", crate::util::slug(&name));
    let turn_id = format!("turn-conductor-{}-{suffix}", crate::util::slug(&name));

    let started = state
        .server
        .handle(ServerRequest::cli(ServerCommand::StartSession {
            agent_name: name.clone(),
            goal: task.to_string(),
            adapter: "acp".to_string(),
            session_id: Some(session_id.clone()),
            run_id: Some(run_id.clone()),
        }))
        .map_err(|e| format!("StartSession failed: {e:?}"))?;
    match started.payload {
        ServerResponsePayload::SessionStarted(_) => {}
        other => return Err(format!("unexpected StartSession payload: {other:?}")),
    }

    // Step 3: drive ONE live ACP worker turn through the controller seam.
    let workspace_root = worktree
        .map(str::to_string)
        .or_else(|| state.worker.default_workspace_root.clone());
    // Steer the worker onto the ON-WIRE file tool. The real `claude-code-acp`
    // bridge round-trips its `Write`/`Edit`/`Read` tools to capo over the ACP wire
    // (`fs/write_text_file` -> capo's confined `file_write`, under the controller's
    // permission decider). If left to its own devices, the worker may instead
    // delegate filesystem work to a `Task` sub-agent whose Bash ops run INSIDE the
    // nested `claude` and are SIMULATED -- they never cross the wire, so capo never
    // performs (or supervises) the write and nothing lands on disk. Prefixing the
    // task with an explicit directive keeps file work on the supervised on-wire
    // path. This is additive guidance around the operator's `task`, not a
    // replacement for it.
    let worker_goal = format!(
        "Perform the following task by editing files DIRECTLY with your built-in \
         Write/Edit/Read file tools (NOT via the Task tool, a sub-agent, or shell \
         commands like cat/printf/echo). Use the file tools so the host can apply \
         and supervise every change.\n\nTask: {task}"
    );
    // DETACHED (depth discipline): spawn the worker turn on a background thread and
    // return immediately so the conductor stays responsive. The worker turn drives
    // through the same RunAcpLiveTurnLocal seam and appends to the event log as
    // usual; the conductor polls via review_agent/list_agents. Default behavior
    // (the synchronous drive below) is unchanged.
    if detached {
        let server = state.server.clone();
        let (sid, rid, tid) = (session_id.clone(), run_id.clone(), turn_id.clone());
        let program = state.worker.acp_program.clone();
        let argv = state.worker.acp_argv.clone();
        let mode = state.worker.acp_session_mode.clone();
        let ws = workspace_root.clone();
        let wmcp_url = state.worker_mcp_url.clone();
        let wmcp_headers = state.worker_mcp_headers.clone();
        let log_session = session_id.clone();
        std::thread::spawn(move || {
            // Surface failures instead of swallowing them: a discarded Err here
            // would leave the session looking indefinitely "running" to a later
            // review_agent/list_agents. (A richer fix appends a terminal
            // `turn_failed` event; logging is the minimal visibility.)
            if let Err(error) = server.handle(ServerRequest::cli(ServerCommand::RunAcpLiveTurnLocal {
                session_id: sid,
                run_id: rid,
                goal: worker_goal,
                turn_id: tid,
                acp_program: program,
                acp_argv: argv,
                workspace_root: ws,
                live_acp_opt_in: true,
                acp_session_mode: mode,
                mcp_url: wmcp_url,
                mcp_headers: wmcp_headers,
            })) {
                eprintln!("capo: detached worker turn failed (session={log_session}): {error:?}");
            }
        });
        return Ok(json!({
            "agent_id": agent_id,
            "name": name,
            "session_id": session_id,
            "run_id": run_id,
            "turn_id": turn_id,
            "status": "running",
            "detached": true,
            "workspace_root": workspace_root,
        })
        .to_string());
    }

    let resp = state
        .server
        .handle(ServerRequest::cli(ServerCommand::RunAcpLiveTurnLocal {
            session_id: session_id.clone(),
            run_id: run_id.clone(),
            goal: worker_goal,
            turn_id: turn_id.clone(),
            acp_program: state.worker.acp_program.clone(),
            acp_argv: state.worker.acp_argv.clone(),
            workspace_root,
            live_acp_opt_in: true,
            acp_session_mode: state.worker.acp_session_mode.clone(),
            mcp_url: state.worker_mcp_url.clone(),
            mcp_headers: state.worker_mcp_headers.clone(),
        }))
        .map_err(|e| format!("RunAcpLiveTurnLocal failed: {e:?}"))?;

    let summary = match resp.payload {
        ServerResponsePayload::AcpLiveTurn(summary) => summary,
        other => return Err(format!("unexpected AcpLiveTurn payload: {other:?}")),
    };

    Ok(json!({
        "agent_id": agent_id,
        "name": name,
        "session_id": session_id,
        "run_id": run_id,
        "turn_id": turn_id,
        "stop_reason": summary.stop_reason,
        "event_count": summary.event_count,
        "appended_event_count": summary.appended_event_count,
        "workspace_root": summary.workspace_root,
    })
    .to_string())
}

// --- Slice-0 (fork-free Path-1): capo's OWN confined file/shell/search MCP
// tools. A locked-down conductor session has NO native tools (the proven
// recipe removes the bridge's built-ins), so capo re-supplies file/shell/search
// here as capo MCP tools wired to the SAME confined `RuntimeToolWrappers`
// (`capo.file_read`/`capo.file_write`/`capo.shell_run`/`capo.search`) that
// `run_acp_live_turn_local` uses. Confinement to the worker's `workspace_root`
// is inherited from the wrapper config; the trusted-local policy authorizes the
// supervised call. ---

/// Build the confined wrapper bundle (wrappers + policy + the confined session
/// identity) for a capo I/O tool call, rooted at the worker's workspace.
fn capo_io_wrappers(
    state: &McpState,
) -> Result<
    (
        capo_tools::RuntimeToolWrappers,
        capo_tools::PermissionPolicy,
        capo_core::SessionId,
        capo_core::RunId,
    ),
    String,
> {
    let workspace_root = std::path::PathBuf::from(
        state
            .worker
            .default_workspace_root
            .clone()
            .ok_or("capo I/O tools require a configured worker workspace")?,
    );
    // Keep capo's tool artifacts inside the workspace so the call is fully
    // self-contained and confined (mirrors the live-turn artifact root, which
    // is workspace-derived).
    let artifact_root = workspace_root.join(".capo-conductor-artifacts");
    let wrappers = capo_tools::RuntimeToolWrappers::new(
        capo_tools::RuntimeToolConfig::local_workspace(workspace_root, artifact_root),
    );
    let policy = capo_tools::PermissionPolicy::allow_trusted_local();
    Ok((
        wrappers,
        policy,
        capo_core::SessionId::new("session-conductor-capo-io"),
        capo_core::RunId::new("run-conductor-capo-io"),
    ))
}

/// Dispatch one confined wrapper tool call and render its result as a JSON
/// string, mapping a non-`completed` status (denied / failed / precondition)
/// onto a tool error so the conductor sees the real outcome.
fn run_capo_io_tool(
    state: &McpState,
    tool_id: &str,
    call_label: &str,
    input: Value,
) -> Result<String, String> {
    let (wrappers, policy, session_id, run_id) = capo_io_wrappers(state)?;
    let request = capo_tools::WrapperToolRequest {
        tool_call_id: capo_core::ToolCallId::new(format!("conductor-{call_label}")),
        session_id,
        run_id,
        tool_id: tool_id.to_string(),
        capability_profile_id: policy.default_profile_id().to_string(),
        input,
    };
    let result = wrappers.authorize_and_invoke(request, &policy);
    // SLICE-A LEGIBILITY: a wrapper signals it completed a unit of work with the
    // authoritative `tool.call_completed` audit event, NOT a fixed status STRING:
    // a successful `shell_run`/`git_command` reports the PROCESS status (e.g.
    // `exited`) in `result.status`, while a `file_read`/`search`/`file_write`
    // reports `completed`. Gating only on `status == "completed"` therefore
    // rejected every successful shell/command run (capo_bash output was lost).
    // Gate on the completion AUDIT EVENT so a successful command -- even a
    // non-zero exit -- surfaces its real outcome (stdout/exit) to the conductor.
    let reached_completion = result
        .events
        .iter()
        .any(|event| event.kind == "tool.call_completed");
    if !reached_completion {
        return Err(format!(
            "{call_label} did not complete (status={}): {}",
            result.status, result.summary
        ));
    }
    // SLICE-A LEGIBILITY (acceptance #4): surface the ACTUAL captured content
    // inline so a LOCKED conductor can SEE and reason over file/command output
    // rather than an opaque `{bytes_read, content_hash}`. The artifacts on disk
    // are ALREADY credential-redacted (`write_redacted_artifact` /
    // `redact_bytes`) and the runner is byte-bounded, so inlining them preserves
    // the confinement contract -- we surface CONTENT, never CREDENTIALS. We read
    // the redacted artifact bytes back (capped) and attach them to the typed
    // output; the hash/artifact-id stay for provenance.
    let mut output = result.typed_output.clone();
    enrich_capo_io_output(tool_id, &mut output, &result.output_artifacts);
    Ok(json!({
        "status": result.status,
        "summary": result.summary,
        "output": output,
    })
    .to_string())
}

/// Inline cap for surfaced tool I/O content (per artifact). Keeps a huge file or
/// command output from blowing the MCP response; `*_truncated` flags report it.
const CAPO_IO_INLINE_CAP: usize = 64 * 1024;

/// Read a redacted artifact file back as (capped) UTF-8 text plus a truncated
/// flag. The artifact was already credential-scrubbed on write; reading it back
/// surfaces CONTENT (not credentials) to the confined conductor.
fn read_artifact_text(
    art: &capo_tools::WrapperArtifact,
    cap: usize,
) -> (String, bool) {
    let bytes = std::fs::read(&art.uri).unwrap_or_default();
    let truncated = bytes.len() > cap;
    let slice = &bytes[..bytes.len().min(cap)];
    (String::from_utf8_lossy(slice).into_owned(), truncated)
}

/// Attach the already-captured, already-redacted content inline to the typed
/// output for the capo I/O tools that otherwise return only a hash.
///
/// - `capo.file_read`: artifact[0] is the read bytes -> `content` + `truncated`.
/// - `capo.shell_run`: artifacts are `[stdout, stderr]` -> `stdout`/`stderr` text.
/// - `capo.search`: `matches`/`total_matches` are ALREADY inline in the typed
///   output (no artifact), so nothing to add here.
fn enrich_capo_io_output(
    tool_id: &str,
    output: &mut Value,
    artifacts: &[capo_tools::WrapperArtifact],
) {
    let Some(map) = output.as_object_mut() else {
        return;
    };
    match tool_id {
        "capo.file_read" => {
            if let Some(art) = artifacts.first() {
                let (text, truncated) = read_artifact_text(art, CAPO_IO_INLINE_CAP);
                map.insert("content".to_string(), Value::String(text));
                map.insert("content_truncated".to_string(), Value::Bool(truncated));
                map.insert(
                    "redacted".to_string(),
                    Value::Bool(art.redaction_state == "redacted"),
                );
            }
        }
        "capo.shell_run" | "capo.git_command" => {
            if let Some(stdout) = artifacts.first() {
                let (text, truncated) = read_artifact_text(stdout, CAPO_IO_INLINE_CAP);
                map.insert("stdout".to_string(), Value::String(text));
                map.insert("stdout_truncated".to_string(), Value::Bool(truncated));
            }
            if let Some(stderr) = artifacts.get(1) {
                let (text, truncated) = read_artifact_text(stderr, CAPO_IO_INLINE_CAP);
                map.insert("stderr".to_string(), Value::String(text));
                map.insert("stderr_truncated".to_string(), Value::Bool(truncated));
            }
        }
        _ => {}
    }
}

/// `capo_read(path)` -> confined `capo.file_read`.
fn tool_capo_read(state: &McpState, args: &Value) -> Result<String, String> {
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .ok_or("capo_read requires `path`")?;
    run_capo_io_tool(state, "capo.file_read", "capo_read", json!({ "path": path }))
}

/// `capo_write(path, content)` -> confined `capo.file_write`.
fn tool_capo_write(state: &McpState, args: &Value) -> Result<String, String> {
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .ok_or("capo_write requires `path`")?;
    let content = args
        .get("content")
        .and_then(Value::as_str)
        .ok_or("capo_write requires `content`")?;
    run_capo_io_tool(
        state,
        "capo.file_write",
        "capo_write",
        json!({ "path": path, "content": content }),
    )
}

/// `capo_bash(command)` -> confined `capo.shell_run` (`sh -c <command>`).
fn tool_capo_bash(state: &McpState, args: &Value) -> Result<String, String> {
    let command = args
        .get("command")
        .and_then(Value::as_str)
        .ok_or("capo_bash requires `command`")?;
    run_capo_io_tool(
        state,
        "capo.shell_run",
        "capo_bash",
        json!({ "program": "sh", "argv": ["-c", command] }),
    )
}

/// `capo_search(query)` -> confined `capo.search`.
fn tool_capo_search(state: &McpState, args: &Value) -> Result<String, String> {
    let query = args
        .get("query")
        .and_then(Value::as_str)
        .ok_or("capo_search requires `query`")?;
    run_capo_io_tool(state, "capo.search", "capo_search", json!({ "query": query }))
}

/// `list_agents()` -> Dashboard projection of each managed agent.
fn tool_list_agents(state: &McpState) -> Result<String, String> {
    let resp = state
        .server
        .handle(ServerRequest::cli(ServerCommand::Dashboard {
            recent_event_limit: 16,
        }))
        .map_err(|e| format!("Dashboard failed: {e:?}"))?;
    let snapshot = match resp.payload {
        ServerResponsePayload::Dashboard(snapshot) => snapshot,
        other => return Err(format!("unexpected Dashboard payload: {other:?}")),
    };
    let agents: Vec<Value> = snapshot
        .agents
        .iter()
        .map(|a| {
            json!({
                "agent_id": a.agent_id.to_string(),
                "name": a.name,
                "status": a.status,
                "current_session_id": a.current_session_id.as_ref().map(ToString::to_string),
                "session": a.session.as_ref().map(|s| json!({
                    "status": s.status,
                    "run_status": s.run_status,
                    "current_goal": s.current_goal,
                    "latest_summary": s.latest_summary,
                    "turn_count": s.turn_count,
                })),
            })
        })
        .collect();
    Ok(json!({
        "agent_count": snapshot.agent_count,
        "active_session_count": snapshot.active_session_count,
        "agents": agents,
    })
    .to_string())
}

/// `review_agent(agent_id)` -> the agent summary + its session thread.
fn tool_review_agent(state: &McpState, args: &Value) -> Result<String, String> {
    let agent_ref = args
        .get("agent_id")
        .and_then(Value::as_str)
        .ok_or("review_agent requires `agent_id`")?;
    let snapshot = dashboard_snapshot(state)?;
    let agent = snapshot
        .agents
        .iter()
        .find(|a| a.name == agent_ref || a.agent_id.to_string() == agent_ref)
        .ok_or_else(|| format!("unknown agent: {agent_ref}"))?;

    let thread = if let Some(session_id) = agent.current_session_id.as_ref() {
        let resp = state
            .server
            .handle(ServerRequest::cli(ServerCommand::ReadThread {
                session_id: session_id.to_string(),
                from_sequence: 0,
            }))
            .map_err(|e| format!("ReadThread failed: {e:?}"))?;
        match resp.payload {
            ServerResponsePayload::Thread(thread) => Some(json!({
                "session_id": thread.session_id,
                "turns": thread.turns.iter().map(|t| json!({
                    "turn_id": t.turn_id,
                    "status": t.status,
                    "items": t.items.iter().map(|i| json!({
                        "kind": i.kind,
                        "text": i.text,
                    })).collect::<Vec<_>>(),
                })).collect::<Vec<_>>(),
            })),
            other => return Err(format!("unexpected ReadThread payload: {other:?}")),
        }
    } else {
        None
    };

    Ok(json!({
        "agent_id": agent.agent_id.to_string(),
        "name": agent.name,
        "status": agent.status,
        "thread": thread,
    })
    .to_string())
}

/// `steer_agent(agent_id, msg)` -> SteerAgent (resolving id -> name).
fn tool_steer_agent(state: &McpState, args: &Value) -> Result<String, String> {
    let agent_ref = args
        .get("agent_id")
        .and_then(Value::as_str)
        .ok_or("steer_agent requires `agent_id`")?;
    let msg = args
        .get("msg")
        .and_then(Value::as_str)
        .ok_or("steer_agent requires `msg`")?;
    let agent_name = resolve_agent_name(state, agent_ref)?;
    let resp = state
        .server
        .handle(ServerRequest::cli(ServerCommand::SteerAgent {
            agent_name: agent_name.clone(),
            goal: msg.to_string(),
        }))
        .map_err(|e| format!("SteerAgent failed: {e:?}"))?;
    let status = match resp.payload {
        ServerResponsePayload::AgentStatus(summary) => summary.status,
        other => return Err(format!("unexpected SteerAgent payload: {other:?}")),
    };
    Ok(json!({ "agent_name": agent_name, "status": status, "steered": true }).to_string())
}

/// `set_mode(scope, agent_id?)` -> conductor-local routing state (no server
/// command). `scope=one` requires `agent_id`.
fn tool_set_mode(state: &McpState, args: &Value) -> Result<String, String> {
    let scope = args
        .get("scope")
        .and_then(Value::as_str)
        .ok_or("set_mode requires `scope`")?;
    if scope != "one" && scope != "all" {
        return Err(format!("invalid scope: {scope} (expected one|all)"));
    }
    let agent_id = args.get("agent_id").and_then(Value::as_str);
    if scope == "one" && agent_id.is_none() {
        return Err("set_mode scope=one requires `agent_id`".to_string());
    }
    {
        let mut mode = state.mode.lock().expect("conductor mode lock");
        mode.scope = Some(scope.to_string());
        mode.agent_id = agent_id.map(str::to_string);
    }
    Ok(json!({ "scope": scope, "agent_id": agent_id }).to_string())
}

// --- helpers ---

fn dashboard_snapshot(state: &McpState) -> Result<crate::ServerDashboardSnapshot, String> {
    let resp = state
        .server
        .handle(ServerRequest::cli(ServerCommand::Dashboard {
            recent_event_limit: 16,
        }))
        .map_err(|e| format!("Dashboard failed: {e:?}"))?;
    match resp.payload {
        ServerResponsePayload::Dashboard(snapshot) => Ok(snapshot),
        other => Err(format!("unexpected Dashboard payload: {other:?}")),
    }
}

/// Resolve an `agent_id` (which may be an id or a name) to the agent's name.
fn resolve_agent_name(state: &McpState, agent_ref: &str) -> Result<String, String> {
    let snapshot = dashboard_snapshot(state)?;
    snapshot
        .agents
        .iter()
        .find(|a| a.name == agent_ref || a.agent_id.to_string() == agent_ref)
        .map(|a| a.name.clone())
        .ok_or_else(|| format!("unknown agent: {agent_ref}"))
}

/// `report_result(key, value)` — a worker returns its final structured result to
/// the conductor over capo's MCP wire (the primary worker-result channel). The
/// value is stored under `key` (the result identifier the conductor assigned,
/// typically the result filename) and read back, in preference to the file, by
/// `collect_results`. Shared across `McpState` clones, so a detached worker's
/// report reaches the conductor turn that calls `collect_results`.
fn tool_report_result(state: &McpState, args: &Value) -> Result<String, String> {
    let key = args
        .get("key")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or("report_result requires a non-empty `key`")?
        // Confine to a bare name so a worker can't key another worker's slot via
        // a path; mirrors collect_results' file-name confinement.
        .trim()
        .to_string();
    let value = args
        .get("value")
        .and_then(Value::as_str)
        .ok_or("report_result requires `value`")?
        .to_string();
    state
        .reports
        .lock()
        .map_err(|_| "report_result: reports map poisoned".to_string())?
        .insert(key.clone(), value);
    Ok(json!({ "ok": true, "key": key }).to_string())
}

/// `collect_results(files, timeout_secs?)` — block server-side until each named
/// result file (relative to the worker workspace) has non-empty content, then
/// return the REAL contents. Removes the conductor-vs-slow-detached-worker timing
/// race (and the temptation to hallucinate) by handing the conductor ground-truth
/// file contents in one tool result.
fn tool_collect_results(state: &McpState, args: &Value) -> Result<String, String> {
    let files: Vec<String> = args
        .get("files")
        .and_then(Value::as_array)
        .ok_or("collect_results requires `files` (array of relative filenames)")?
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();
    if files.is_empty() {
        return Err("collect_results: `files` must be a non-empty array".to_string());
    }
    let timeout_secs = args
        .get("timeout_secs")
        .and_then(Value::as_u64)
        .unwrap_or(75)
        .min(180);
    let root = std::path::PathBuf::from(
        state
            .worker
            .default_workspace_root
            .clone()
            .ok_or("collect_results: no worker workspace configured")?,
    );
    // Confine to the workspace: only the file NAME component is honored.
    let resolve = |f: &str| -> std::path::PathBuf {
        let name = std::path::Path::new(f).file_name().unwrap_or_default();
        root.join(name)
    };
    let read_nonempty = |p: &std::path::Path| -> Option<String> {
        match std::fs::read_to_string(p) {
            Ok(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
            _ => None,
        }
    };

    // Tier 1: a value reported via `report_result(key, value)` is ground truth
    // and takes precedence over the file (no wait needed). Snapshot the map each
    // poll so a value reported mid-wait is picked up.
    let reported = |key: &str| -> Option<String> {
        state
            .reports
            .lock()
            .ok()
            .and_then(|m| m.get(key).cloned())
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.trim().to_string())
    };

    let start = std::time::Instant::now();
    let deadline = std::time::Duration::from_secs(timeout_secs);
    loop {
        let mut results = serde_json::Map::new();
        let mut all_ready = true;
        for f in &files {
            // Tier 1 (reported value) → Tier 2 (result file). Tier 3 (LLM extract
            // from the worker's reply text) is handled by the conductor layer, not
            // here, since this server has no model handle.
            match reported(f).or_else(|| read_nonempty(&resolve(f))) {
                Some(content) => {
                    results.insert(f.clone(), Value::String(content));
                }
                None => {
                    all_ready = false;
                    results.insert(f.clone(), Value::Null);
                }
            }
        }
        if all_ready || start.elapsed() >= deadline {
            return Ok(json!({
                "ready": all_ready,
                "results": results,
                "waited_secs": start.elapsed().as_secs(),
            })
            .to_string());
        }
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
}

fn derive_agent_name(task: &str) -> String {
    let slugged = crate::util::slug(task);
    let short: String = slugged.chars().take(24).collect();
    if short.is_empty() {
        format!("worker-{}", crate::util::stable_hash(task.as_bytes()))
    } else {
        format!("worker-{short}")
    }
}

fn tool_result(id: Value, text: &str, is_error: bool) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "content": [{ "type": "text", "text": text }],
            "isError": is_error
        }
    })
}

fn error_response(id: Value, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message }
    })
}

fn json_response(body: Value) -> Response {
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}
