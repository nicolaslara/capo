//! HTTP/SSE facade over the Capo server boundary.
//!
//! Reads come from the same query layer the dashboard uses (`capo-query`);
//! mutations go through the typed server boundary (`CapoServer::handle`). Both
//! the SQLite-backed query store and `CapoServer` are non-`Send` across awaits,
//! so every request does its work inside `spawn_blocking`.
//!
//!   GET  /api/dashboard   -> full read model (agents, dispatch, lanes, activity)
//!   POST /api/commands     -> steer / interrupt / stop
//!   GET  /api/events       -> Server-Sent Events stream of the read model
//! and serves the built front-end (web/app/dist) for everything else.

use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::sse::{Event, KeepAlive, Sse},
    routing::{get, post},
};
use capo_core::ProjectId;
use capo_query::{ProjectDashboardQuery, project_dashboard};
use capo_server::{
    AgentSummary, CapoServer, ServerClientOrigin, ServerCommand, ServerDashboardSnapshot,
    ServerInputOrigin, ServerRequest, ServerResponsePayload, SessionSummary,
};
use capo_state::SqliteStateStore;
use futures::StreamExt;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio_stream::wrappers::IntervalStream;
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};

const PROJECT_ID: &str = "project-capo";
static REQUEST_SEQ: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
struct Config {
    state_root: String,
    addr: String,
}

#[tokio::main]
async fn main() {
    let addr = std::env::var("CAPO_WEB_ADDR").unwrap_or_else(|_| "127.0.0.1:4177".to_string());
    let state_root = std::env::var("CAPO_STATE_ROOT").unwrap_or_else(|_| ".capo-dev".to_string());
    let dist = std::env::var("CAPO_WEB_DIST").unwrap_or_else(|_| "web/app/dist".to_string());

    let cfg = Arc::new(Config {
        state_root: state_root.clone(),
        addr: addr.clone(),
    });

    let index = format!("{dist}/index.html");
    let static_service = ServeDir::new(&dist).not_found_service(ServeFile::new(index));

    let app = Router::new()
        .route("/api/dashboard", get(dashboard))
        .route("/api/commands", post(commands))
        .route("/api/events", get(events))
        .fallback_service(static_service)
        .layer(CorsLayer::permissive())
        .with_state(cfg);

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| panic!("bind {addr}: {e}"));
    println!("capo-web listening on http://{addr}  (state_root={state_root}, dist={dist})");
    axum::serve(listener, app).await.expect("server");
}

fn api_request(command: ServerCommand) -> ServerRequest {
    let n = REQUEST_SEQ.fetch_add(1, Ordering::Relaxed);
    ServerRequest {
        request_id: format!("capo-web-{n}"),
        origin: ServerClientOrigin {
            client_id: "capo-web".to_string(),
            actor_id: "operator".to_string(),
            input_origin: ServerInputOrigin::Api,
        },
        command,
    }
}

/// Build the full console read model (blocking).
fn build_console(state_root: &str, addr: &str) -> Result<Value, String> {
    let server = CapoServer::open(ProjectId::new(PROJECT_ID), state_root)
        .map_err(|e| format!("open server: {e:?}"))?;
    let snapshot = match server
        .handle(api_request(ServerCommand::Dashboard {
            recent_event_limit: 50,
        }))
        .map_err(|e| format!("handle: {e:?}"))?
        .payload
    {
        ServerResponsePayload::Dashboard(s) => s,
        other => return Err(format!("unexpected payload: {other:?}")),
    };

    let lanes = read_lanes(state_root).unwrap_or_default();
    Ok(map_dashboard(&snapshot, addr, &lanes))
}

#[derive(Default)]
struct Lanes {
    activity: Vec<Value>,
    evidence: Vec<Value>,
    reviews: Vec<Value>,
    validations: Vec<Value>,
}

/// Lanes come from the read-model query, which exposes evidence / reviews /
/// validations / recent events that the server snapshot omits.
fn read_lanes(state_root: &str) -> Option<Lanes> {
    let store = SqliteStateStore::open(state_root).ok()?;
    let mut query = ProjectDashboardQuery::new(ProjectId::new(PROJECT_ID));
    query.recent_event_limit = 50;
    let pd = project_dashboard(&store, query).ok()?;

    let mut session_agent: HashMap<String, String> = HashMap::new();
    for row in &pd.agents {
        if let Some(s) = &row.session {
            session_agent.insert(s.session.session_id.to_string(), row.agent.name.clone());
        }
    }
    let target_of = |sid: &str| {
        session_agent
            .get(sid)
            .cloned()
            .unwrap_or_else(|| sid.to_string())
    };

    // Activity: flatten + dedupe recent events, newest first.
    let mut events = Vec::new();
    for row in &pd.agents {
        if let Some(s) = &row.session {
            for e in &s.recent_events {
                events.push(e);
            }
        }
    }
    events.sort_by(|a, b| b.sequence.cmp(&a.sequence));
    let mut seen = std::collections::HashSet::new();
    let activity: Vec<Value> = events
        .into_iter()
        .filter(|e| seen.insert(e.event_id.clone()))
        .take(50)
        .map(|e| {
            let agent = e
                .agent_id
                .as_ref()
                .map(|a| a.to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| e.actor.clone());
            let agent = agent.trim_start_matches("agent-").to_string();
            json!({
                "id": e.event_id,
                "sequence": e.sequence,
                "time": "",
                "agent": agent,
                "kind": kind_label(&e.kind),
                "text": summarize(&e.payload_json, &e.kind),
            })
        })
        .collect();

    let evidence: Vec<Value> = pd
        .project_evidence
        .iter()
        .map(|e| {
            let sid = e
                .session_id
                .as_ref()
                .map(|s| s.to_string())
                .unwrap_or_default();
            json!({
                "id": e.evidence_id.to_string(),
                "kind": e.kind,
                "status": confidence_status(e.confidence),
                "agent": target_of(&sid),
            })
        })
        .collect();

    let reviews: Vec<Value> = pd
        .review_findings
        .iter()
        .map(|r| {
            json!({
                "id": r.review_finding_id,
                "status": r.status,
                "target": target_of(&r.session_id.to_string()),
            })
        })
        .collect();

    let validations: Vec<Value> = pd
        .task_outcome_reports
        .iter()
        .map(|t| {
            json!({
                "id": t.task_outcome_report_id,
                "status": t.outcome_status,
                "target": target_of(&t.session_id.to_string()),
            })
        })
        .collect();

    Some(Lanes {
        activity,
        evidence,
        reviews,
        validations,
    })
}

async fn dashboard(State(cfg): State<Arc<Config>>) -> Result<Json<Value>, (StatusCode, String)> {
    let state_root = cfg.state_root.clone();
    let addr = cfg.addr.clone();
    let value = tokio::task::spawn_blocking(move || build_console(&state_root, &addr))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("join: {e}")))?
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(value))
}

#[derive(Deserialize)]
struct CommandBody {
    kind: String,
    agent: String,
    #[serde(default)]
    message: String,
    #[serde(default)]
    reason: String,
    #[serde(default)]
    goal: String,
}

async fn commands(
    State(cfg): State<Arc<Config>>,
    Json(body): Json<CommandBody>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let command = match body.kind.as_str() {
        "steer_agent" => ServerCommand::SteerAgent {
            agent_name: body.agent,
            goal: if body.message.is_empty() {
                body.goal
            } else {
                body.message
            },
        },
        "interrupt_agent" => ServerCommand::InterruptAgent {
            agent_name: body.agent,
            reason: if body.reason.is_empty() {
                "operator interrupt".to_string()
            } else {
                body.reason
            },
        },
        "stop_agent" => ServerCommand::StopAgent {
            agent_name: body.agent,
            reason: if body.reason.is_empty() {
                "operator stop".to_string()
            } else {
                body.reason
            },
        },
        other => {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("unsupported command: {other}"),
            ));
        }
    };
    let state_root = cfg.state_root.clone();
    tokio::task::spawn_blocking(move || {
        let server = CapoServer::open(ProjectId::new(PROJECT_ID), &state_root)
            .map_err(|e| format!("open: {e:?}"))?;
        server
            .handle(api_request(command))
            .map_err(|e| format!("handle: {e:?}"))
    })
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("join: {e}")))?
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(json!({ "ok": true })))
}

async fn events(
    State(cfg): State<Arc<Config>>,
) -> Sse<impl futures::Stream<Item = Result<Event, Infallible>>> {
    let stream =
        IntervalStream::new(tokio::time::interval(Duration::from_millis(1500))).then(move |_| {
            let cfg = cfg.clone();
            async move {
                let value =
                    tokio::task::spawn_blocking(move || build_console(&cfg.state_root, &cfg.addr))
                        .await
                        .unwrap_or_else(|e| Err(format!("join: {e}")))
                        .unwrap_or_else(|e| json!({ "error": e }));
                Ok(Event::default()
                    .json_data(value)
                    .unwrap_or_else(|_| Event::default().data("{}")))
            }
        });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

// ---------------------------------------------------------------------------
// Mapping: typed snapshot + query lanes -> the console's JSON read model
// ---------------------------------------------------------------------------

fn map_dashboard(s: &ServerDashboardSnapshot, addr: &str, lanes: &Lanes) -> Value {
    let agents: Vec<Value> = s.agents.iter().map(map_agent).collect();
    let blocked = s
        .agents
        .iter()
        .filter(|a| matches!(agent_status(a), "blocked" | "timed out"))
        .count();

    let blocked_tone = if blocked > 0 { "warn" } else { "default" };
    let blocked_hint = if blocked > 0 { "needs attention" } else { "" };
    let summary = json!([
        { "key": "agents", "label": "Agents", "value": s.agent_count },
        { "key": "active", "label": "Active", "value": s.active_session_count, "tone": "info" },
        { "key": "blocked", "label": "Blocked", "value": blocked, "tone": blocked_tone, "hint": blocked_hint },
        { "key": "evidence", "label": "Evidence", "value": lanes.evidence.len() },
        { "key": "reviews", "label": "Reviews", "value": lanes.reviews.len() },
        { "key": "validations", "label": "Validations", "value": lanes.validations.len(), "tone": "good" }
    ]);

    json!({
        "project": {
            "id": s.project_id.to_string(),
            "name": "Capo",
            "server": "capo-server (live)",
            "mode": "live",
            "addr": addr,
            "updatedAt": ""
        },
        "summary": summary,
        "agents": agents,
        "activity": lanes.activity,
        "evidence": lanes.evidence,
        "reviews": lanes.reviews,
        "validations": lanes.validations,
        // Goals, tools, permissions, and chat history need new ServerCommands / projections.
        "goals": [],
        "permissions": [],
        "tools": [],
        "chats": {}
    })
}

fn agent_status(a: &AgentSummary) -> &'static str {
    let raw = a
        .session
        .as_ref()
        .and_then(|s| s.run_status.as_deref())
        .map(str::to_ascii_lowercase)
        .or_else(|| Some(a.status.to_ascii_lowercase()))
        .unwrap_or_default();
    match raw.as_str() {
        s if s.contains("run") || s.contains("progress") || s.contains("active") => "running",
        s if s.contains("finish")
            || s.contains("complete")
            || s.contains("done")
            || s.contains("succeed") =>
        {
            "finished"
        }
        s if s.contains("timeout") || s.contains("timed") => "timed out",
        s if s.contains("block") => "blocked",
        s if s.contains("pause") => "paused",
        _ if a.session.is_some() => "running",
        _ => "available",
    }
}

fn map_agent(a: &AgentSummary) -> Value {
    let s = a.session.as_ref();
    let confidence = s
        .and_then(|x| x.latest_confidence)
        .map(map_confidence)
        .unwrap_or("medium");
    json!({
        "id": a.name,
        "name": a.name,
        "status": agent_status(a),
        "adapter": s.and_then(|x| x.adapter_kind.clone()).unwrap_or_else(|| "unknown".to_string()),
        "goal": s.map(|x| x.current_goal.clone()).unwrap_or_default(),
        "result": s.and_then(|x| x.latest_summary.clone()).unwrap_or_default(),
        "confidence": confidence,
        "evidence": s.map(|x| x.evidence_refs.clone()).unwrap_or_default(),
        "reviews": s.map(|x| x.review_finding_count).unwrap_or(0),
        "validations": s.map(|x| x.task_outcome_report_count).unwrap_or(0),
        "tools": s.map(|x| x.tool_call_count).unwrap_or(0),
        "memory": s.map(|x| x.memory_packet_count).unwrap_or(0),
        "blocker": s.and_then(|x| x.latest_blocker.clone()),
        "updatedAt": "",
        "sessionId": a.current_session_id.as_ref().map(|x| x.to_string()),
        "runId": s.and_then(|x| x.run_id.as_ref().map(|r| r.to_string())),
        "rawOutputPolicy": s.and_then(|x| x.dispatch_raw_output_policy.clone()),
        "rawPromptPolicy": s.and_then(|x| x.dispatch_raw_prompt_policy.clone()),
        "dispatch": s.map(map_dispatch),
    })
}

fn map_dispatch(s: &SessionSummary) -> Value {
    let plan = if s.latest_dispatch_plan_id.is_some() {
        "done"
    } else {
        "none"
    };
    let gate = match s.dispatch_gate_status.as_deref() {
        Some(v) if v.contains("approv") || v.contains("gated") || v.contains("ready") => "done",
        Some(v) if v.contains("block") || v.contains("reject") => "blocked",
        Some(_) => "pending",
        None => "none",
    };
    let preflight = if s.dispatch_gate_status.is_some() || s.latest_dispatch_gate_id.is_some() {
        "done"
    } else if plan == "done" {
        "pending"
    } else {
        "none"
    };
    let run = match s.dispatch_execution_status.as_deref() {
        Some(v) if v.contains("complete") || v.contains("succeed") || v.contains("done") => "done",
        Some(v) if v.contains("run") || v.contains("active") => "active",
        Some(v) if v.contains("fail") || v.contains("error") || v.contains("timeout") => "failed",
        Some(_) => "pending",
        None => "none",
    };
    json!({
        "plan": plan,
        "preflight": preflight,
        "gate": gate,
        "run": run,
        "gateStatus": s.dispatch_gate_status,
        "nextAction": s.dispatch_next_action,
        "credentialScan": s.dispatch_credential_scan_status,
        "providerCliExecuted": s.dispatch_provider_cli_executed.unwrap_or(false),
        "planId": s.latest_dispatch_plan_id,
        "gateId": s.latest_dispatch_gate_id,
        "executionId": s.latest_dispatch_execution_id,
    })
}

fn map_confidence(c: i64) -> &'static str {
    match c {
        x if x >= 3 || x >= 70 => "high",
        x if x >= 1 => "medium",
        _ => "low",
    }
}

fn confidence_status(c: i64) -> &'static str {
    match c {
        x if x >= 3 || x >= 70 => "validated",
        x if x >= 1 => "partial",
        _ => "pending",
    }
}

fn kind_label(kind: &str) -> String {
    // e.g. "session.goal_updated" -> "goal updated"; keep a short, human label.
    let tail = kind.rsplit('.').next().unwrap_or(kind);
    tail.replace('_', " ")
}

fn summarize(payload_json: &str, kind: &str) -> String {
    if let Ok(Value::Object(map)) = serde_json::from_str::<Value>(payload_json) {
        for key in [
            "summary", "goal", "text", "message", "reason", "content", "title", "detail", "note",
        ] {
            if let Some(Value::String(s)) = map.get(key)
                && !s.is_empty()
            {
                return truncate(s, 160);
            }
        }
    }
    kind.replace('_', " ").replace('.', " · ")
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max).collect();
        out.push('…');
        out
    }
}
