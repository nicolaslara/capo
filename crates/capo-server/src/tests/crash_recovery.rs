//! RTL10 tests: crash-safe in-flight runs -- persist the pid/process-group
//! reference before the spawn returns, and reap the orphaned process group on
//! restart.
//!
//! These are deterministic (no live provider): a real `/bin/sh` process group
//! with a backgrounded descendant stands in for an in-flight Codex run that
//! outlived a controller crash. The descendant would write a marker after a
//! delay; reaping the persisted process group on "restart" must kill it before
//! it writes, leave the thread read model consistent (no half-open run), and be
//! idempotent across repeated restarts.

use super::*;

use capo_core::RunId;
use capo_runtime::{LocalProcessConfig, LocalProcessRequest, LocalProcessRunner};
use capo_state::{EventKind, NewEvent, ProjectionRecord, RedactionState, RunProjection};
use std::collections::HashMap;
use std::time::Duration;

/// Register an agent and start a codex session/run on `server`. The run is left
/// `running` (active-looking) -- exactly the in-flight state a crash interrupts.
fn register_and_start(server: &CapoServer, agent: &str, goal: &str, session: &str, run: &str) {
    handle(
        server,
        ServerCommand::RegisterAgent {
            name: agent.to_string(),
        },
    );
    handle(
        server,
        ServerCommand::StartSession {
            agent_name: agent.to_string(),
            goal: goal.to_string(),
            adapter: "codex".to_string(),
            session_id: Some(session.to_string()),
            run_id: Some(run.to_string()),
        },
    );
}

/// Persist the in-flight marker the live spawn path records the instant the
/// spawn returns: a `run.started` event carrying the `external_pid` and
/// process-group reference, keeping the run `running`.
fn persist_inflight_marker(
    server: &CapoServer,
    project: &ProjectId,
    session: &str,
    run: &str,
    external_pid: u32,
) {
    let session_id = SessionId::new(session.to_string());
    let run_id = RunId::new(run.to_string());
    server
        .controller
        .state()
        .append_event(
            NewEvent {
                event_id: format!("event-run-started-inflight-{run}-{external_pid}"),
                kind: EventKind::RunStarted,
                actor: "capo-server".to_string(),
                project_id: Some(project.clone()),
                task_id: None,
                agent_id: None,
                session_id: Some(session_id.clone()),
                run_id: Some(run_id.clone()),
                turn_id: Some("turn-crash".to_string()),
                item_id: Some(format!("local-process-{run}")),
                payload_json: serde_json::json!({
                    "status": "running",
                    "runtime_process_ref": format!("local-process-{run}"),
                    "external_pid": external_pid,
                    "marker": "start_requested_inflight",
                })
                .to_string(),
                idempotency_key: Some(format!("server-run-started-inflight:{run}:{external_pid}")),
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::Run(RunProjection {
                run_id,
                session_id,
                status: "running".to_string(),
                recovery_of_run_id: None,
                updated_sequence: 0,
            })],
        )
        .expect("persist in-flight marker");
}

#[cfg(unix)]
#[test]
fn restart_mid_turn_reaps_the_orphaned_process_group_and_leaves_a_consistent_read_model() {
    let root = temp_root();
    let workspace = root.join("workspace");
    let artifacts = root.join("artifacts");
    std::fs::create_dir_all(&workspace).expect("workspace");
    let marker = workspace.join("orphan-survived.txt");

    let project = ProjectId::new("project-capo");
    let server = CapoServer::open(project.clone(), &root).expect("server");
    register_and_start(
        &server,
        "codex-local",
        "crash mid-turn",
        "session-crash",
        "run-crash",
    );

    // Spawn a real in-flight process group with a backgrounded descendant that
    // would write the marker after a delay -- the orphan a crash would leave
    // running. We persist its pid as the live path would, then drop our handle
    // to simulate the controller dying mid-run.
    let runner = LocalProcessRunner::new(LocalProcessConfig::for_test(
        workspace.clone(),
        artifacts.clone(),
    ));
    let running = runner
        .spawn_process(LocalProcessRequest {
            run_id: RunId::new("run-crash"),
            turn_id: Some("turn-crash".to_string()),
            program: "/bin/sh".to_string(),
            argv: vec![
                "-c".to_string(),
                format!("(sleep 2; printf survived > {}) &", marker.display()),
            ],
            cwd: workspace,
            env: HashMap::new(),
        })
        .expect("spawn in-flight orphan");
    let pid = running.process.external_pid.expect("pid recorded");
    persist_inflight_marker(&server, &project, "session-crash", "run-crash", pid);

    // The crash: the parent `/bin/sh` exits immediately after backgrounding the
    // descendant, the descendant keeps sleeping, and Capo no longer owns the
    // child handle -- only the persisted pid survives. Dropping the running
    // handle stands in for the controller dying mid-run.
    std::thread::sleep(Duration::from_millis(100));
    drop(running);

    // The run still looks live in the read model (a crashed in-flight run).
    let run_id = RunId::new("run-crash");
    assert_eq!(
        server
            .controller
            .state()
            .run(&run_id)
            .unwrap()
            .expect("run")
            .status,
        "running"
    );

    // Restart: reap the orphaned process group by the persisted pid and record
    // the outcome.
    let recovered = server
        .reap_orphaned_runs_on_restart("recovery-1")
        .expect("reap orphans on restart");
    assert_eq!(recovered, 1, "the one in-flight run is recovered");

    // The reaped descendant never gets to write its marker.
    std::thread::sleep(Duration::from_millis(2200));
    assert!(
        !marker.exists(),
        "reaping the process group must kill the orphaned descendant"
    );

    // The thread read model is consistent: the run is terminal (recovered), not
    // a half-open `running`, and is no longer active-looking.
    assert_eq!(
        server
            .controller
            .state()
            .run(&run_id)
            .unwrap()
            .expect("run")
            .status,
        "recovered"
    );
    assert!(
        server
            .controller
            .state()
            .active_looking_runs()
            .unwrap()
            .is_empty()
    );

    // The recovery events the state model prescribes were recorded.
    let kinds: Vec<String> = server
        .controller
        .state()
        .recent_events_for_session(&SessionId::new("session-crash"), 32)
        .unwrap()
        .into_iter()
        .map(|event| event.kind)
        .collect();
    assert!(kinds.iter().any(|kind| kind == "run.orphaned"), "{kinds:?}");
    assert!(kinds.iter().any(|kind| kind == "run.exited"), "{kinds:?}");
    assert!(
        kinds.iter().any(|kind| kind == "run.recovered"),
        "{kinds:?}"
    );

    // Idempotent across repeated restarts: a second restart that observes the
    // same runtime state (the now-gone pid) appends no new recovery events and
    // leaves the run terminal.
    let event_count_before = server.controller.state().event_count().unwrap();
    let recovered_again = server
        .reap_orphaned_runs_on_restart("recovery-2")
        .expect("reap orphans on restart again");
    // The run is no longer active-looking, so a second restart finds nothing to
    // reap and records nothing.
    assert_eq!(recovered_again, 0);
    assert_eq!(
        server.controller.state().event_count().unwrap(),
        event_count_before
    );

    // Replay rebuilds the recovered run identically.
    server
        .controller
        .state()
        .rebuild_projections()
        .expect("rebuild projections");
    assert_eq!(
        server
            .controller
            .state()
            .run(&run_id)
            .unwrap()
            .expect("run")
            .status,
        "recovered"
    );
}
