//! RTL10 tests: crash-safe in-flight runs -- persist the pid/process-group
//! reference before the spawn returns, and reap the orphaned process group on
//! restart through the production recovery path.
//!
//! These are deterministic (no live provider):
//!
//! - `live_path_persists_the_in_flight_start_marker_before_completion` drives the
//!   real live run path (`run_live_provider_local` with a codex stub) and asserts
//!   the production `append_run_started_inflight` recorded the in-flight marker
//!   (production event-id/idempotency-key shape, recorded `external_pid` and
//!   `boot_id`). If that producer or its `live_provider.rs` call site regressed,
//!   this test fails -- the marker is no longer self-attested by a test helper.
//!
//! - `restart_mid_turn_reaps_the_orphaned_process_group_and_leaves_a_consistent_read_model`
//!   reaches the in-flight state by persisting the marker through that SAME
//!   production producer (a real `/bin/sh` process group with a backgrounded
//!   descendant stands in for an in-flight Codex run that outlived a controller
//!   crash), then drives the PRODUCTION restart-recovery seam
//!   (`ServerCommand::Recover` -> `recover_server` -> `recover_command`). The
//!   recovery must reap the orphaned descendant before its delayed marker, leave
//!   the thread read model consistent (no half-open run), and be idempotent
//!   across repeated restarts.

use super::*;

use capo_core::RunId;
use capo_runtime::{
    LocalProcessConfig, LocalProcessRequest, LocalProcessRunner, LocalRuntimeProcessRef,
};
use std::collections::HashMap;
use std::os::unix::fs::PermissionsExt;
use std::time::Duration;

use crate::live_provider::LiveProviderLocalRunRequest;
use crate::safety_floor::WriteMode;

fn system_origin() -> ServerClientOrigin {
    ServerClientOrigin {
        client_id: "test-client".to_string(),
        actor_id: "test-actor".to_string(),
        input_origin: ServerInputOrigin::System,
    }
}

/// Register an agent and start a codex session/run on `server`. The run is left
/// `running` (active-looking) -- exactly the in-flight state a crash interrupts.
fn register_and_start(server: &CapoServer, agent: &str, goal: &str, session: &str, run: &str) {
    handle(
        server,
        ServerCommand::RegisterAgent {
            name: agent.to_string(),
            adapter: "fake".to_string(),
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

/// Preflight the live provider against the real confined workspace/artifacts and
/// return the ready dispatch plan id.
#[allow(clippy::too_many_arguments)]
fn preflight(
    server: &CapoServer,
    agent: &str,
    goal: &str,
    workspace: &str,
    artifacts: &str,
    session: &str,
    run: &str,
    turn: &str,
) -> String {
    let response = handle(
        server,
        ServerCommand::PreflightLiveProvider {
            agent_name: agent.to_string(),
            adapter: "codex".to_string(),
            goal: goal.to_string(),
            workspace: workspace.to_string(),
            artifacts: artifacts.to_string(),
            session_id: session.to_string(),
            run_id: run.to_string(),
            turn_id: turn.to_string(),
            capability_profile: "trusted-local".to_string(),
            runtime_scope: "local_process_loopback".to_string(),
            credential_scan_policy: "metadata_only_no_secret_read".to_string(),
            raw_prompt_policy: "not_rendered".to_string(),
            raw_output_policy: "artifacts_scanned_redacted".to_string(),
            tool_wrapper_policy: "capo_wrapped_required".to_string(),
            live_provider_opt_in: true,
        },
    );
    let ServerResponsePayload::LiveProviderPreflighted(preflight) = response.payload else {
        panic!("expected live provider preflight response");
    };
    preflight.dispatch_plan_id
}

/// Write an executable codex stub that ignores its argv and emits the read-only
/// `codex-exec.jsonl` fixture on stdout, so the read-only live spawn path is
/// deterministic with no live provider (the marker we assert is persisted right
/// after the spawn returns, before the output is parsed). Uses only shell
/// builtins because the runtime spawns with `env_clear()` (empty `PATH`).
fn write_noop_codex_stub(dir: &std::path::Path) -> std::path::PathBuf {
    let stub = dir.join("codex-noop-stub.sh");
    let fixture_path = dir.join("codex-exec-readonly-fixture.jsonl");
    std::fs::write(
        &fixture_path,
        include_str!("../../../capo-adapters/fixtures/codex-exec.jsonl"),
    )
    .expect("write read-only fixture");
    // `read`/`printf` are POSIX builtins, so this needs no `PATH`. The fixture is
    // passed by absolute path and streamed verbatim to stdout.
    let script = format!(
        "#!/bin/sh\nwhile IFS= read -r line; do printf '%s\\n' \"$line\"; done < {fixture}\n",
        fixture = fixture_path.display(),
    );
    std::fs::write(&stub, script).expect("write stub");
    let mut perms = std::fs::metadata(&stub).expect("stub meta").permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&stub, perms).expect("chmod stub");
    stub
}

/// RTL10 core acceptance: the PRODUCTION live run path persists the in-flight
/// start marker (start-requested + pid + boot id) before the run completes.
///
/// This drives the real `run_live_provider_local` path -- which calls
/// `append_run_started_inflight` from `live_provider.rs` right after the spawn
/// returns -- and then asserts exactly one `run.started` in-flight marker exists
/// in the durable event log with the production event-id / idempotency-key shape,
/// a recorded `external_pid`, and the persisted boot id. The marker is no longer
/// proven only by a divergent test helper.
#[cfg(unix)]
#[test]
fn live_path_persists_the_in_flight_start_marker_before_completion() {
    let root = temp_root();
    let workspace = root.join("workspace");
    let artifacts = root.join("artifacts");
    std::fs::create_dir_all(&workspace).expect("workspace");
    let workspace_str = workspace.to_string_lossy().to_string();
    let artifacts_str = artifacts.to_string_lossy().to_string();

    let goal = "Persist the in-flight start marker before the run completes".to_string();
    let project = ProjectId::new("project-capo");
    let server = CapoServer::open(project.clone(), &root).expect("server");
    register_and_start(
        &server,
        "codex-local",
        &goal,
        "session-inflight",
        "run-inflight",
    );
    let dispatch_plan_id = preflight(
        &server,
        "codex-local",
        &goal,
        &workspace_str,
        &artifacts_str,
        "session-inflight",
        "run-inflight",
        "turn-inflight",
    );

    let stub = write_noop_codex_stub(&root);
    let origin = system_origin();
    let run = server
        .run_live_provider_local(
            &origin,
            LiveProviderLocalRunRequest {
                dispatch_plan_id: &dispatch_plan_id,
                goal: &goal,
                live_execution_opt_in: true,
                mock_runtime_opt_in: false,
                mock_provider_output_name: None,
                mock_provider_output_jsonl: None,
                timeout_seconds: 10,
                codex_program_override: Some(stub.to_string_lossy().as_ref()),
                write_mode: WriteMode::DryRun,
                record_selected_argv: None,
            },
        )
        .expect("live read-only run");
    assert!(
        run.provider_cli_executed,
        "the live path must spawn the stub provider"
    );

    // Exactly one in-flight `run.started` marker, with the production shape: an
    // event-id of `event-server-run-started-inflight-<hash(run_id)>-<pid>`, an
    // idempotency key of `server-run-started-inflight:<run_id>:<pid>`, a recorded
    // `external_pid`, and the persisted `boot_id`.
    let markers: Vec<_> = server
        .controller
        .state()
        .recent_events_for_session(&SessionId::new("session-inflight"), 64)
        .expect("events")
        .into_iter()
        .filter(|event| {
            event.kind == "run.started"
                && event
                    .payload_json
                    .contains("\"marker\":\"start_requested_inflight\"")
        })
        .collect();
    assert_eq!(
        markers.len(),
        1,
        "exactly one in-flight start marker must be persisted, got {markers:?}"
    );
    let marker = &markers[0];
    let payload: serde_json::Value =
        serde_json::from_str(&marker.payload_json).expect("marker payload json");
    let pid = payload
        .get("external_pid")
        .and_then(|pid| pid.as_u64())
        .expect("the marker records the spawned external_pid");
    assert!(pid > 1, "the recorded pid must be a real process pid");
    assert!(
        payload.get("boot_id").is_some(),
        "the marker records a boot id for cross-reboot reap safety: {payload}"
    );
    let run_id = RunId::new("run-inflight");
    let expected_event_id = format!(
        "event-server-run-started-inflight-{}-{}",
        crate::util::stable_hash(run_id.as_str().as_bytes()),
        pid
    );
    assert_eq!(
        marker.event_id, expected_event_id,
        "the marker must use the production event-id shape"
    );
    assert_eq!(
        marker.idempotency_key.as_deref(),
        Some(format!("server-run-started-inflight:run-inflight:{pid}").as_str()),
        "the marker must use the production idempotency-key shape"
    );
}

/// Persist the in-flight marker exactly as the live spawn path does, by calling
/// the PRODUCTION producer `CapoServer::append_run_started_inflight` with the
/// real spawned process ref. This reaches the in-flight state through production
/// code (not a divergent hand-rolled event), keeping the run `running`.
fn persist_inflight_marker_via_production(
    server: &CapoServer,
    origin: &ServerClientOrigin,
    dispatch_plan_id: &str,
    turn_id: &str,
    process: &LocalRuntimeProcessRef,
) {
    let (plan, _prompt_source) = server
        .dispatch_plan_with_prompt(dispatch_plan_id)
        .expect("dispatch plan");
    server
        .append_run_started_inflight(origin, &plan, turn_id, process)
        .expect("persist in-flight marker via production path");
}

#[cfg(unix)]
#[test]
fn restart_mid_turn_reaps_the_orphaned_process_group_and_leaves_a_consistent_read_model() {
    let root = temp_root();
    let workspace = root.join("workspace");
    let artifacts = root.join("artifacts");
    std::fs::create_dir_all(&workspace).expect("workspace");
    let workspace_str = workspace.to_string_lossy().to_string();
    let artifacts_str = artifacts.to_string_lossy().to_string();
    let marker = workspace.join("orphan-survived.txt");

    let goal = "crash mid-turn".to_string();
    let project = ProjectId::new("project-capo");
    let server = CapoServer::open(project.clone(), &root).expect("server");
    register_and_start(&server, "codex-local", &goal, "session-crash", "run-crash");
    let dispatch_plan_id = preflight(
        &server,
        "codex-local",
        &goal,
        &workspace_str,
        &artifacts_str,
        "session-crash",
        "run-crash",
        "turn-crash",
    );

    // Spawn a real in-flight process group with a backgrounded descendant that
    // would write the marker after a delay -- the orphan a crash would leave
    // running. We persist its pid through the PRODUCTION marker producer as the
    // live path would, then drop our handle to simulate the controller dying
    // mid-run.
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
    running
        .process
        .external_pid
        .expect("pid recorded for the in-flight orphan");
    let origin = system_origin();
    persist_inflight_marker_via_production(
        &server,
        &origin,
        &dispatch_plan_id,
        "turn-crash",
        &running.process,
    );

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

    // Restart: drive the PRODUCTION recovery seam (the `recover` server command
    // -> `recover_server` -> `recover_command`), which reaps the orphaned process
    // group by the persisted pid and records the outcome inside a framed recovery
    // attempt.
    let recovery = handle(&server, ServerCommand::Recover);
    let ServerResponsePayload::Recovery(recovery) = recovery.payload else {
        panic!("expected recovery response");
    };
    assert_eq!(
        recovery.recovered_run_count, 1,
        "the one in-flight run is recovered"
    );

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

    // The reap ran inside a framed recovery attempt: the production
    // `recover_command` brackets it with `begin_recovery`/`complete_recovery`, so
    // the summary carries a recovery_attempt_id and a watermark.
    assert!(
        !recovery.recovery_attempt_id.is_empty(),
        "the reap must run inside a framed recovery attempt"
    );
    assert!(recovery.watermark.is_some());

    // The per-run recovery events the state model prescribes were recorded.
    let kinds: Vec<String> = server
        .controller
        .state()
        .recent_events_for_session(&SessionId::new("session-crash"), 64)
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

    // Idempotent across repeated restarts: a second recovery that observes the
    // same runtime state (the run no longer active-looking) records no new
    // per-run recovery events and leaves the run terminal.
    let run_started_before = run_recovery_event_count(&server, &SessionId::new("session-crash"));
    let recovery_again = handle(&server, ServerCommand::Recover);
    let ServerResponsePayload::Recovery(recovery_again) = recovery_again.payload else {
        panic!("expected recovery response");
    };
    assert_eq!(recovery_again.recovered_run_count, 0);
    assert_eq!(
        run_recovery_event_count(&server, &SessionId::new("session-crash")),
        run_started_before,
        "a second restart that observes the same state appends no new per-run recovery events"
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

/// Count the per-run recovery events (`run.orphaned`/`run.exited`/`run.recovered`)
/// for a session -- the events whose idempotency must hold across restarts.
fn run_recovery_event_count(server: &CapoServer, session: &SessionId) -> usize {
    server
        .controller
        .state()
        .recent_events_for_session(session, 128)
        .unwrap()
        .into_iter()
        .filter(|event| {
            matches!(
                event.kind.as_str(),
                "run.orphaned" | "run.exited" | "run.recovered"
            )
        })
        .count()
}
