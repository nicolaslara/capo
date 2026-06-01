//! HTTP/SSE facade over the Capo server boundary.
//!
//! Reads come from the same query layer the dashboard uses (`capo-query`);
//! mutations go through the typed server boundary (`CapoServer::handle`). The
//! SQLite-backed query store and `CapoServer` do their per-request work inside
//! `spawn_blocking`; the live event tail keeps that blocking work off the async
//! request handler too (ST8).
//!
//!   GET  /api/dashboard      -> full read model (agents, dispatch, lanes, activity)
//!   POST /api/commands        -> steer / interrupt / stop
//!   GET  /api/events?from=N   -> Server-Sent Events EVENT TAIL: incremental,
//!                                broadcast-backed `ServerEvent` frames (ST4/ST8),
//!                                each frame the published wire contract
//!                                (`event:`/`data:` JSON-RPC notification).
//! and serves the built front-end (web/app/dist) for everything else.

use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use axum::{
    Json, Router,
    extract::{Query, State},
    http::StatusCode,
    response::sse::{Event, KeepAlive, Sse},
    routing::{get, post},
};
use capo_core::ProjectId;
use capo_query::{ProjectDashboardQuery, project_dashboard};
use capo_server::{
    AgentSummary, CapoServer, EventNotification, ServerClientOrigin, ServerCommand,
    ServerDashboardSnapshot, ServerEvent, ServerInputOrigin, ServerRequest, ServerResponsePayload,
    SessionSummary, TailRecvError, contract::sse_frame,
};
use capo_state::SqliteStateStore;
use futures::StreamExt;
use serde::Deserialize;
use serde_json::{Value, json};
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};

const PROJECT_ID: &str = "project-capo";
static REQUEST_SEQ: AtomicU64 = AtomicU64::new(1);

/// How long the blocking tail pump blocks for the next committed event before
/// looping to re-check liveness and run the cross-process catch-up poll. Small
/// enough to surface events committed by *other* processes (whose writes never
/// reach this process's broadcast hub) with low latency, large enough that an
/// idle tail is event-driven rather than a busy spin.
const TAIL_POLL_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Clone)]
struct Config {
    state_root: String,
    addr: String,
    /// One long-lived server handle shared across requests. `CapoServer::handle`
    /// takes `&self`, so the SSE event tail and the command path share it: a
    /// command committed here fans out over the server's own broadcast hub (ST4)
    /// to every live tail with no extra wiring. `CapoServer` is `Send + Sync`
    /// (the same handle `serve_tcp` shares across connection threads).
    server: Arc<CapoServer>,
    /// A store handle on the same db file, used by the event tail's
    /// `events_after` catch-up for events committed by *other* processes (whose
    /// writes never reach this process's broadcast hub).
    store: Arc<SqliteStateStore>,
}

#[tokio::main]
async fn main() {
    let addr = std::env::var("CAPO_WEB_ADDR").unwrap_or_else(|_| "127.0.0.1:4177".to_string());
    let state_root = std::env::var("CAPO_STATE_ROOT").unwrap_or_else(|_| ".capo-dev".to_string());
    let dist = std::env::var("CAPO_WEB_DIST").unwrap_or_else(|_| "web/app/dist".to_string());

    let server = Arc::new(
        CapoServer::open(ProjectId::new(PROJECT_ID), &state_root)
            .unwrap_or_else(|e| panic!("open server {state_root}: {e:?}")),
    );
    let store = Arc::new(
        SqliteStateStore::open(&state_root)
            .unwrap_or_else(|e| panic!("open state store {state_root}: {e:?}")),
    );

    let cfg = Arc::new(Config {
        state_root: state_root.clone(),
        addr: addr.clone(),
        server,
        store,
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

#[derive(Deserialize)]
struct EventsQuery {
    /// Resume watermark (ST4): the highest `sequence` the client already has.
    /// The tail yields only events strictly after this. Omitted means "tail
    /// from now" -- the current end of the log -- so a fresh subscriber is not
    /// flooded with the whole backlog (the initial snapshot is GET /api/dashboard).
    from: Option<i64>,
    /// Optional single-session filter, matching `Subscribe { session_id }`.
    session: Option<String>,
}

/// GET /api/events -- the broadcast-backed EVENT TAIL (ST4/ST8).
///
/// Replaces the old 1500ms full-dashboard re-poll: instead of re-projecting the
/// entire read model on a timer, this subscribes to the committed-event log and
/// emits *incremental* `ServerEvent` frames as they commit. Each SSE block is
/// the published wire contract -- `event: event` + a `data:` line carrying the
/// verbatim JSON-RPC `event` notification (see
/// `contract/snapshots/sse-event-tail.json`).
///
/// It rides the server's public `Subscribe` boundary ([`CapoServer::subscribe`]),
/// so two sources feed one gap-free, duplicate-free, sequence-ordered tail:
///
///  * the broadcast hub, for events committed in this process; and
///  * a `Subscribe` catch-up read against the same db file, for events committed
///    by *other* processes (whose writes never reach this process's broadcast
///    hub).
///
/// A shared delivery watermark across both sources dedupes the seam.
async fn events(
    State(cfg): State<Arc<Config>>,
    Query(q): Query<EventsQuery>,
) -> Sse<impl futures::Stream<Item = Result<Event, Infallible>>> {
    // Buffer comfortably so a burst of commits is never dropped between the
    // blocking pump and the async SSE writer; the pump applies back-pressure by
    // blocking on `send` when the client is slow.
    let (tx, rx) = tokio::sync::mpsc::channel::<Event>(256);
    let server = cfg.server.clone();
    let store = cfg.store.clone();
    let session = q.session.clone();
    let from = q.from;

    // The whole tail -- the public `CapoServer::subscribe` (catch-up backlog +
    // live `EventStream`) and the live pump -- runs on a blocking thread.
    // `CapoServer` is `Send + Sync` and `recv_batch` is a synchronous blocking
    // primitive, so this keeps that blocking work off the async runtime without a
    // per-event `spawn_blocking` round-trip (ST8).
    tokio::task::spawn_blocking(move || run_event_tail(server, store, session, from, tx));

    let stream = tokio_stream::wrappers::ReceiverStream::new(rx).map(Ok::<Event, Infallible>);
    Sse::new(stream).keep_alive(KeepAlive::default())
}

/// Drive one client's event tail to completion on a blocking thread.
///
/// Uses the server's public `Subscribe` boundary (ST4): one
/// [`CapoServer::subscribe`] subscribes to the broadcast hub *before* reading the
/// catch-up backlog (so a seam event is captured live, not lost) and hands back
/// both the backlog and the live [`EventStream`]. The loop then emits incremental
/// SSE frames until the client disconnects (the channel `send` fails) or the
/// broadcast hub is gone.
fn run_event_tail(
    server: Arc<CapoServer>,
    store: Arc<SqliteStateStore>,
    session: Option<String>,
    from: Option<i64>,
    tx: tokio::sync::mpsc::Sender<Event>,
) {
    // "Tail from now" when the client gives no watermark: resume from the
    // current end of the log so a fresh subscriber sees only *new* events. Read
    // off the shared store handle on the same db file.
    let from_sequence = match from {
        Some(seq) => seq,
        None => store.last_sequence().unwrap_or(0),
    };

    // Subscribe through the server boundary: this subscribes to the broadcast
    // *before* snapshotting the backlog, so an event committed in the seam is
    // captured live rather than lost (ST4).
    let (backlog, mut stream) = match server.subscribe(session.clone(), from_sequence) {
        Ok(pair) => pair,
        Err(_) => return,
    };
    // Highest sequence delivered across every source; the live stream and the
    // cross-process catch-up both dedupe against it so the seam yields no gap and
    // no duplicate. The backlog's `next_sequence` already seeds the stream's own
    // watermark.
    let mut delivered_through = backlog.next_sequence;
    for event in backlog.events {
        if emit_event(&tx, &event).is_err() {
            return;
        }
    }

    loop {
        match stream.recv_batch(TAIL_POLL_INTERVAL) {
            Ok(batch) => {
                for event in batch {
                    delivered_through = delivered_through.max(event.sequence);
                    if emit_event(&tx, &event).is_err() {
                        return;
                    }
                }
            }
            // No in-process commit this window. Fall through to the cross-process
            // catch-up below, then loop. (A `Timeout` is the normal idle path.)
            Err(TailRecvError::Timeout) => {}
            // The broadcast hub and every store handle were dropped: no further
            // in-process events can arrive. The catch-up below still covers any
            // last cross-process writes, but the stream is effectively done.
            Err(TailRecvError::Disconnected) => return,
        }

        // Catch-up: pick up events committed by other processes, which never
        // reach this process's broadcast hub. Dedupe against `delivered_through`
        // (the live stream may have already delivered some) and advance it.
        match read_catchup(&server, &session, delivered_through) {
            Ok(events) => {
                for event in events {
                    if event.sequence <= delivered_through {
                        continue;
                    }
                    delivered_through = event.sequence;
                    if emit_event(&tx, &event).is_err() {
                        return;
                    }
                }
            }
            Err(_) => return,
        }
    }
}

/// Read the catch-up backlog (events strictly after `from_sequence`, optionally
/// one session) as egress-shaped `ServerEvent`s -- the same forward read of the
/// append-only log the server's `Subscribe` backlog uses. Issued through the
/// public `Subscribe` boundary; the throwaway live stream is dropped (and so
/// unsubscribed) immediately, leaving only the catch-up `events`.
fn read_catchup(
    server: &CapoServer,
    session: &Option<String>,
    from_sequence: i64,
) -> Result<Vec<ServerEvent>, ()> {
    let (backlog, _stream) = server
        .subscribe(session.clone(), from_sequence)
        .map_err(|_| ())?;
    Ok(backlog.events)
}

/// Encode one committed event as a contract SSE frame and hand it to the SSE
/// writer. Returns `Err` once the client has disconnected (the channel closed).
fn emit_event(tx: &tokio::sync::mpsc::Sender<Event>, event: &ServerEvent) -> Result<(), ()> {
    let frame = sse_frame(&EventNotification::for_event(event));
    // `sse_frame` is the full `event: <name>\ndata: <json>\n\n` block; axum's
    // `Event` re-adds framing, so split it back into the named event + data line
    // and let axum re-serialize the identical block.
    let event = parse_sse_frame(&frame);
    tx.blocking_send(event).map_err(|_| ())
}

/// Split the canonical `event: <name>\ndata: <json>\n\n` contract block back
/// into an axum [`Event`] carrying the same event name and data line, so the
/// re-serialized SSE block is byte-for-byte the published contract frame.
fn parse_sse_frame(frame: &str) -> Event {
    let mut name = None;
    let mut data = None;
    for line in frame.lines() {
        if let Some(rest) = line.strip_prefix("event: ") {
            name = Some(rest.to_string());
        } else if let Some(rest) = line.strip_prefix("data: ") {
            data = Some(rest.to_string());
        }
    }
    let mut event = Event::default();
    if let Some(name) = name {
        event = event.event(name);
    }
    event.data(data.unwrap_or_default())
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

#[cfg(test)]
mod tests {
    //! ST8: the `/api/events` SSE bridge re-exposes the server's event tail (ST4)
    //! as *incremental* frames -- a newly-committed event reaches a live
    //! subscriber without any full dashboard re-poll, and each frame is the
    //! published wire contract verbatim.
    //!
    //! Both tests are deterministic: events are produced by scripted server
    //! commands (`RegisterAgent`), never a live provider or a clock.
    //!
    //! The wire-shape fixture lives at `tests/snapshots/sse-event-tail.json`. As
    //! with the server contract tests it is regenerate-and-diff: with
    //! `CAPO_REGENERATE_WIRE_SNAPSHOTS` set the test rewrites it, and unset (the
    //! default, including CI) the test reads it back and asserts byte-equality so
    //! the SSE wire shape cannot drift silently.

    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    use capo_server::{EventNotification, ServerEvent, contract::sse_frame};

    use super::*;

    static TEMP_ROOT_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn temp_root() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let counter = TEMP_ROOT_COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("capo-web-{nanos}-{counter}"))
    }

    fn open_server(root: &Path) -> Arc<CapoServer> {
        Arc::new(
            CapoServer::open(ProjectId::new(PROJECT_ID), root).expect("open server in temp root"),
        )
    }

    fn register(server: &CapoServer, name: &str) {
        server
            .handle(api_request(ServerCommand::RegisterAgent {
                name: name.to_string(),
                adapter: "fake".to_string(),
            }))
            .unwrap_or_else(|error| panic!("register {name}: {error:?}"));
    }

    /// A fixed, deterministic `ServerEvent` so the SSE frame fixture is stable
    /// across runs and machines (no clock, no randomness, no live provider).
    fn fixed_event() -> ServerEvent {
        ServerEvent {
            sequence: 44,
            event_id: "event-0000000044".to_string(),
            kind: "session.summary_updated".to_string(),
            actor: "local-user".to_string(),
            project_id: Some("project-capo".to_string()),
            task_id: Some("task-demo".to_string()),
            agent_id: Some("agent-demo".to_string()),
            session_id: Some("session-demo".to_string()),
            run_id: Some("run-demo".to_string()),
            turn_id: Some("turn-2".to_string()),
            item_id: Some("item-7".to_string()),
            payload_json: "{\"summary\":\"inspected workspace state\"}".to_string(),
            redaction_state: "safe".to_string(),
        }
    }

    fn snapshot_path() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("snapshots")
            .join("sse-event-tail.json")
    }

    /// Read the on-disk fixture, or (re)write it when regenerating, mirroring the
    /// server's `contract` snapshot tests so the two stay in lockstep.
    fn checked_in_or_regenerated(path: &Path, expected: &str) -> String {
        if std::env::var_os("CAPO_REGENERATE_WIRE_SNAPSHOTS").is_some() {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("create snapshot dir");
            }
            std::fs::write(path, expected).expect("write fixture");
            return expected.to_string();
        }
        std::fs::read_to_string(path).unwrap_or_else(|error| {
            panic!(
                "missing checked-in SSE fixture {}: {error}.\n\
                 Regenerate with CAPO_REGENERATE_WIRE_SNAPSHOTS=1 cargo test -p capo-web",
                path.display()
            )
        })
    }

    /// The SSE frame `/api/events` emits is byte-for-byte the published wire
    /// contract: an `event: event` line plus the verbatim JSON-RPC `event`
    /// notification, produced by the same `sse_frame` codec the raw transport
    /// uses (ST4/ST8/ST9).
    #[test]
    fn sse_event_frame_matches_the_checked_in_contract() {
        let frame = sse_frame(&EventNotification::for_event(&fixed_event()));
        let path = snapshot_path();
        let on_disk = checked_in_or_regenerated(&path, &frame);
        assert_eq!(
            on_disk,
            frame,
            "the /api/events SSE wire shape drifted from {}.\n\
             If this change is intentional, regenerate with \
             CAPO_REGENERATE_WIRE_SNAPSHOTS=1 cargo test -p capo-web",
            path.display()
        );

        // Sanity: the frame our emit path round-trips through `parse_sse_frame`
        // names the contract event type, so axum re-serializes the same block.
        let event = parse_sse_frame(&frame);
        let rebuilt = format!("{:?}", event);
        assert!(
            rebuilt.contains("event"),
            "round-tripped Event must carry the contract event name: {rebuilt}"
        );
    }

    /// A newly-committed event reaches a live `/api/events` subscriber as an
    /// incremental frame -- the tail surfaces it on its own, with no full
    /// dashboard re-poll. This is the ST8 replacement for the old 1500ms
    /// IntervalStream(re-run build_console) tail.
    #[test]
    fn events_stream_surfaces_incremental_event_without_repoll() {
        let root = temp_root();
        let server = open_server(&root);
        let store = Arc::new(SqliteStateStore::open(&root).expect("open store"));

        // Baseline write so the log is non-empty, then "tail from now": the
        // subscriber must NOT be flooded with this backlog.
        register(&server, "alpha");
        let baseline = store.last_sequence().expect("last_sequence");

        let (tx, mut rx) = tokio::sync::mpsc::channel::<Event>(16);
        let tail_server = server.clone();
        let tail_store = store.clone();
        // Run the real production tail (blocking) on its own OS thread, resuming
        // strictly after the baseline so only *new* events are delivered.
        let handle = std::thread::spawn(move || {
            run_event_tail(tail_server, tail_store, None, Some(baseline), tx);
        });

        // Nothing should arrive before a new commit: the backlog is suppressed by
        // the "tail from now" watermark.
        assert!(
            rx.try_recv().is_err(),
            "tail-from-now must not replay the pre-subscription backlog"
        );

        // Commit a brand-new event AFTER the subscription is live.
        register(&server, "beta");

        // The new event must surface on the tail incrementally. Poll the channel
        // (the tail pump wakes within TAIL_POLL_INTERVAL) up to a generous bound.
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut received = 0usize;
        while Instant::now() < deadline {
            match rx.try_recv() {
                Ok(_frame) => {
                    received += 1;
                    break;
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => break,
            }
        }
        assert!(
            received >= 1,
            "a newly-committed event must reach the live SSE tail without a full re-poll"
        );

        // The new event is strictly after the baseline -- an incremental frame,
        // not a re-projected snapshot. Verify directly against the tail's own
        // catch-up read (the same `Subscribe` boundary the live pump uses).
        let after = read_catchup(&server, &None, baseline).expect("catch-up read");
        assert!(
            after.iter().all(|event| event.sequence > baseline),
            "catch-up after the baseline must yield only strictly-newer events: {:?}",
            after.iter().map(|e| e.sequence).collect::<Vec<_>>()
        );
        assert!(
            !after.is_empty(),
            "the post-baseline commit must be visible to the tail"
        );

        // Dropping the receiver makes the next `blocking_send` fail, so the tail
        // thread returns on its own.
        drop(rx);
        let _ = handle.join();
    }
}
