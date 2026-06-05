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
        }
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
                    "name": {"type": "string", "description": "Optional worker name; auto-derived from the task if omitted."}
                },
                "required": ["task"]
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
        "list_agents" => tool_list_agents(state),
        "review_agent" => tool_review_agent(state, &args),
        "steer_agent" => tool_steer_agent(state, &args),
        "set_mode" => tool_set_mode(state, &args),
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

    match outcome {
        Ok(text) => tool_result(id, &text, false),
        // MCP convention: tool-execution failures are reported in-result with
        // isError:true, NOT as a JSON-RPC error.
        Err(message) => tool_result(id, &message, true),
    }
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
    let resp = state
        .server
        .handle(ServerRequest::cli(ServerCommand::RunAcpLiveTurnLocal {
            session_id: session_id.clone(),
            run_id: run_id.clone(),
            goal: task.to_string(),
            turn_id: turn_id.clone(),
            acp_program: state.worker.acp_program.clone(),
            acp_argv: state.worker.acp_argv.clone(),
            workspace_root,
            live_acp_opt_in: true,
            acp_session_mode: state.worker.acp_session_mode.clone(),
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
