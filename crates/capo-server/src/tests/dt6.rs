//! DT6 (distributed-topology): the all-local default is the DEFAULT and an
//! always-on regression that proves the single-box path is byte-for-byte
//! unchanged AND structurally inert.
//!
//! The distributed surface (the DT2 keep-alive planes, the DT3 remote-runner
//! attach, the DT4b buffered-event spool, the DT5 exposure gating) is built ONLY
//! when a non-loopback `RoleConfig` is present. In the default single-process
//! path that code is not entered, so it can add NO event and NO wire frame to the
//! all-local deployment. This module pins that two ways, both deterministic and
//! always-on (NOT `#[ignore]`), so a change that alters the single-box path fails
//! the gate:
//!
//! 1. [`all_local_turn_thread_projection_is_byte_for_byte_stable`] -- a
//!    single-process all-local run (server + local runner + client over loopback,
//!    the existing turn loop + `Subscribe`/thread read model) projects an
//!    IDENTICAL thread across two independent runs, and that projection contains
//!    NO distributed (`connectivity.*` / heartbeat / `runtime.remote_*` /
//!    `runtime.target_registered`) event kind. The distributed surface is
//!    therefore structurally inert in the realized log, not merely claimed inert.
//! 2. [`st9_wire_snapshots_are_byte_identical_for_the_single_box_path`] -- the
//!    checked-in `streaming-transport` ST9 contract wire snapshots are byte-for-
//!    byte equal to a fresh generation from the SAME codec the single-box
//!    transport uses, so a single-box wire-shape change fails DT6 (the contract
//!    module owns the regenerate-and-diff vehicle; DT6 re-asserts it as the
//!    single-box regression).
//!
//! The loopback-only bind enforcement (DT6 acceptance: the server still
//! HARD-rejects a non-loopback bind absent a grant) is proven against the real
//! transport guard in [`super::dt5`]
//! (`non_loopback_bind_is_refused_without_a_grant` /
//! `loopback_bind_is_accepted_with_no_grant_default`); DT6 re-affirms the framing
//! here in [`all_local_loopback_bind_is_accepted_with_no_grant`] so the default
//! branch is anchored from the regression module too.

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::*;
use crate::ServerThread;
use crate::contract::{self, WireSample};
use crate::transport::{
    CancellationToken, RequestHandler, ServeConfig, TransportResult, serve_tcp_with_handler,
};

/// The checked-in contract directory: `crates/capo-server/contract/` (mirrors the
/// path the `contract` test module pins, so DT6 reads the SAME fixtures).
fn contract_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("contract")
}

/// The codex-exec replay fixture: a real single-box turn that emits an assistant
/// message, an observed tool round-trip, and a terminal `turn.completed`.
/// Replaying it through the server's production write path commits the SAME
/// normalized events a live single-box turn does (the existing turn loop), so the
/// all-local regression observes a real turn with no live provider.
const CODEX_FIXTURE: &str = include_str!("../../../capo-adapters/fixtures/codex-exec.jsonl");

/// Distributed event-kind markers (wire tags) that MUST NOT appear in an
/// all-local single-box turn. Any of these in the realized log would mean the
/// distributed surface was constructed/entered in the default path, which is
/// exactly the DT6 inertness regression.
///
/// IMPORTANT (resolving review finding 3): these are checked against the RAW
/// COMMITTED EVENT LOG, not only the thread read model. The thread projection only
/// surfaces `Output`/`Tool`/`Terminal` items, so a runtime-layer kind like
/// `runtime.start_requested` (emitted by the local runner in `capo-runtime`) would
/// NEVER appear as a thread item and the old item-only check was vacuous for it. By
/// scanning the raw `events` table we catch a distributed/runtime kind even if it
/// never projects into a thread item.
const DISTRIBUTED_EVENT_KIND_MARKERS: &[&str] = &[
    "connectivity.",
    "runtime.health_changed",
    "runtime.remote_",
    "runtime.target_registered",
    "runtime.start_requested",
];

/// Reopen the all-local store at `root` and return every committed event kind, in
/// sequence order. This is the RAW log (the `events` table) -- the single source of
/// truth the read models are projected from -- so a distributed/runtime event kind
/// that never becomes a thread item is still caught (finding 3).
fn committed_event_kinds(root: &Path) -> Vec<String> {
    let store = capo_state::SqliteStateStore::open(root).expect("reopen the all-local store");
    store
        .events_after(0, 1_000_000)
        .expect("read the committed event log")
        .into_iter()
        .map(|event| event.kind)
        .collect()
}

/// Run one deterministic all-local turn through the production write path and
/// return the server-projected thread read model (over the in-process handler,
/// the same path `ReadThread` serves). No role flags, loopback only.
fn run_all_local_turn(session: &str, run: &str, turn: &str) -> (ServerThread, PathBuf) {
    let root = temp_root();
    let project_id = ProjectId::new("project-dt6-all-local");
    let server = CapoServer::open(project_id, &root).expect("server opens for the all-local box");

    // Register a local agent and replay one real turn — the single-box default:
    // server + local runner + client all in one process over loopback.
    let registered = handle(
        &server,
        ServerCommand::RegisterAgent {
            name: "dt6-local-codex".to_string(),
            adapter: "fake".to_string(),
        },
    );
    assert_agent_registered(&registered, "dt6-local-codex");

    let started = handle(
        &server,
        ServerCommand::StartSession {
            agent_name: "dt6-local-codex".to_string(),
            goal: "prove the all-local single-box turn is unchanged".to_string(),
            adapter: "codex".to_string(),
            session_id: Some(session.to_string()),
            run_id: Some(run.to_string()),
        },
    );
    let ServerResponsePayload::SessionStarted(started) = started.payload else {
        panic!("expected a session-started response for the all-local turn");
    };
    assert_eq!(started.session_id.as_str(), session);

    let replayed = handle(
        &server,
        ServerCommand::ReplayAdapterFixture {
            adapter: "codex".to_string(),
            session_id: session.to_string(),
            run_id: run.to_string(),
            turn_id: turn.to_string(),
            fixture_name: "crates/capo-adapters/fixtures/codex-exec.jsonl".to_string(),
            fixture_jsonl: CODEX_FIXTURE.to_string(),
        },
    );
    let ServerResponsePayload::AdapterFixtureReplayed(replay) = replayed.payload else {
        panic!("expected an adapter replay response for the all-local turn");
    };
    assert_eq!(
        replay.completed_turn_count, 1,
        "the all-local turn must complete exactly one turn"
    );

    let read = handle(
        &server,
        ServerCommand::ReadThread {
            session_id: session.to_string(),
            from_sequence: 0,
        },
    );
    let ServerResponsePayload::Thread(thread) = read.payload else {
        panic!("expected a thread payload for the all-local turn");
    };
    (thread, root)
}

#[test]
fn all_local_turn_thread_projection_is_byte_for_byte_stable() {
    // Two independent single-process all-local runs with the SAME session/run/turn
    // identifiers must project the IDENTICAL thread read model. The projection is
    // derived purely from the committed event log + the existing `Subscribe`/thread
    // read model, so equality proves the single-box turn loop is deterministic and
    // unchanged by the distributed surface.
    let (first, first_root) = run_all_local_turn("session-dt6", "run-dt6", "turn-dt6");
    let (second, _second_root) = run_all_local_turn("session-dt6", "run-dt6", "turn-dt6");
    assert_eq!(
        first, second,
        "the all-local thread projection must be byte-for-byte stable across runs \
         (a divergence means the single-box path changed)"
    );

    // The turn must actually carry the real single-box turn shape (so the equality
    // above is not vacuously true on an empty thread): one completed turn with at
    // least the assistant-summary + terminal items.
    assert_eq!(
        first.turns.len(),
        1,
        "the all-local run must project exactly one turn"
    );
    let turn = &first.turns[0];
    assert_eq!(turn.status, "completed", "the single-box turn completes");
    assert!(
        turn.items.len() >= 2,
        "the single-box turn must project its incremental output + terminal items, got {}",
        turn.items.len()
    );

    // Structural inertness in the REALIZED log: no distributed event kind is
    // reachable in the all-local default. The keep-alive planes (DT2), the remote
    // runner path (DT3), the spool (DT4b), and the exposure gating (DT5) are
    // constructed only for a non-loopback RoleConfig, so NONE of their event kinds
    // can appear here.
    //
    // We scan the RAW COMMITTED EVENT LOG (resolving review finding 3): a
    // runtime-layer kind such as `runtime.start_requested` (emitted by the local
    // runner) is NEVER projected into a thread item, so the prior item-only check was
    // vacuous for it. The raw log is the source of truth the read models project
    // from, so a distributed/runtime kind is caught here even when it never becomes a
    // thread item.
    let committed_kinds = committed_event_kinds(&first_root);
    assert!(
        !committed_kinds.is_empty(),
        "the all-local turn must commit events (so the marker scan is not vacuous)"
    );
    for kind in &committed_kinds {
        for marker in DISTRIBUTED_EVENT_KIND_MARKERS {
            assert!(
                !kind.starts_with(marker),
                "the all-local default committed a distributed/runtime event kind `{kind}` \
                 (marker `{marker}`); the distributed surface must be structurally inert \
                 without a non-loopback endpoint"
            );
        }
    }

    // Belt-and-braces: the projected thread items must also be marker-free (the read
    // model can only surface a subset of kinds, but if a distributed kind ever did
    // project, this catches it too).
    for turn in &first.turns {
        for item in &turn.items {
            for marker in DISTRIBUTED_EVENT_KIND_MARKERS {
                assert!(
                    !item.event_kind.starts_with(marker),
                    "the all-local default projected a distributed event kind `{}` (marker `{}`)",
                    item.event_kind,
                    marker,
                );
            }
        }
    }
}

#[test]
fn st9_wire_snapshots_are_byte_identical_for_the_single_box_path() {
    // DT6 acceptance: the checked-in streaming-transport ST9 contract wire
    // snapshots are UNCHANGED for the single-box path. The contract module owns the
    // regenerate-and-diff vehicle (`contract::wire_samples` produces frames from the
    // SAME codec the live transport uses); DT6 re-asserts byte-equality against the
    // checked-in fixtures as the always-on single-box regression, so a wire-shape
    // change in the default path fails here too.
    let snapshots = contract_dir().join("snapshots");
    let mut checked = 0usize;
    for WireSample {
        name,
        description,
        frame,
    } in contract::wire_samples()
    {
        assert!(!description.is_empty(), "snapshot {name} must be described");
        let path = snapshots.join(format!("{name}.json"));
        let on_disk = std::fs::read_to_string(&path).unwrap_or_else(|error| {
            panic!(
                "missing checked-in ST9 contract fixture {}: {error}.\n\
                 Regenerate with CAPO_REGENERATE_WIRE_SNAPSHOTS=1 cargo test -p capo-server --lib contract",
                path.display()
            )
        });
        let expected = format!("{frame}\n");
        assert_eq!(
            on_disk,
            expected,
            "ST9 wire snapshot {name} drifted for the single-box path ({}); \
             the all-local contract must stay byte-for-byte unchanged",
            path.display()
        );
        checked += 1;
    }
    assert!(
        checked >= 1,
        "the ST9 contract must pin at least one wire snapshot for the single-box regression"
    );
}

/// A trivial handler for the bind-guard re-affirmation: the guard runs before the
/// accept loop, so `handle` is unreachable.
struct NoopHandler;

impl RequestHandler for NoopHandler {
    fn handle(
        &self,
        _request: ServerRequest,
        _cancel: &CancellationToken,
    ) -> TransportResult<ServerResponse> {
        unreachable!("the DT6 bind-guard re-affirmation never accepts a connection")
    }
}

#[test]
fn all_local_loopback_bind_is_accepted_with_no_grant() {
    // DT6 acceptance: the all-local path stays loopback-only by default — a
    // loopback bind passes with NO grant (today's enforcement preserved). The
    // non-loopback HARD-rejection without a grant is proven against the same real
    // transport guard in `super::dt5::non_loopback_bind_is_refused_without_a_grant`;
    // DT6 anchors the default (loopback) branch from the regression module.
    let listener =
        TcpListener::bind("127.0.0.1:0").expect("loopback listener for the all-local box");
    let accepted = serve_tcp_with_handler(
        listener,
        Arc::new(NoopHandler),
        Some(0),
        ServeConfig::default(),
    )
    .expect("the all-local loopback bind is accepted with no grant");
    assert_eq!(
        accepted, 0,
        "no connection is accepted (max_connections = 0); the guard ran and passed"
    );
}
