//! RTL9 tests: the Codex workspace-write adapter and its live tool-result
//! round-trip.
//!
//! Three properties are proven deterministically (no live provider):
//!
//! 1. The workspace-write round-trip ingests into an OBSERVED tool-result event
//!    (`tool.observation_recorded` for the `apply_patch` tool) that is distinct
//!    from the agent's own reported claim (`session.summary_updated` /
//!    `tool.observation_recorded` carries the applied diff/output, the summary
//!    carries the agent message). This is the RTL9 contract: an observed
//!    tool-result distinct from any agent-reported claim.
//! 2. The ingested write turn rebuilds identically after a restart/replay.
//! 3. The live spawn arm selects the workspace-WRITE profile and engages the
//!    RTL6 confinement + pre-write checkpoint BEFORE any process runs only when
//!    the resolved write mode is `LiveWrite`; the default (`DryRun`) stays
//!    read-only and takes no checkpoint. Driven through a deterministic codex
//!    stub via `codex_program_override`, so no live provider is needed.

use super::*;

use std::os::unix::fs::PermissionsExt;

use crate::live_provider::LiveProviderLocalRunRequest;
use crate::safety_floor::WriteMode;

const WRITE_FIXTURE: &str =
    include_str!("../../../capo-adapters/fixtures/codex-exec-workspace-write.jsonl");

fn system_origin() -> ServerClientOrigin {
    ServerClientOrigin {
        client_id: "test-client".to_string(),
        actor_id: "test-actor".to_string(),
        input_origin: ServerInputOrigin::System,
    }
}

/// Register a codex agent and open a session on `server`.
fn register_and_start(server: &CapoServer, session: &str, run: &str, goal: &str) {
    handle(
        server,
        ServerCommand::RegisterAgent {
            name: "codex-local".to_string(),
            adapter: "fake".to_string(),
        },
    );
    handle(
        server,
        ServerCommand::StartSession {
            agent_name: "codex-local".to_string(),
            goal: goal.to_string(),
            adapter: "codex".to_string(),
            session_id: Some(session.to_string()),
            run_id: Some(run.to_string()),
        },
    );
}

/// Preflight the live provider and return the ready dispatch plan id.
#[allow(clippy::too_many_arguments)]
fn preflight(server: &CapoServer, session: &str, run: &str, turn: &str, goal: &str) -> String {
    let preflight = handle(
        server,
        ServerCommand::PreflightLiveProvider {
            agent_name: "codex-local".to_string(),
            adapter: "codex".to_string(),
            goal: goal.to_string(),
            workspace: "/tmp/capo-workspace".to_string(),
            artifacts: "/tmp/capo-artifacts".to_string(),
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
    let ServerResponsePayload::LiveProviderPreflighted(preflight) = preflight.payload else {
        panic!("expected live provider preflight response");
    };
    preflight.dispatch_plan_id
}

/// Write an executable codex stub that ignores its argv and emits the
/// workspace-write fixture JSONL on stdout (and applies the edit the fixture
/// describes), so the spawn path is deterministic with no live provider. Uses
/// only shell builtins because the runtime spawns with `env_clear()` (empty
/// `PATH`).
fn write_codex_stub(dir: &std::path::Path, write_target_rel: &str) -> std::path::PathBuf {
    let stub = dir.join("codex-stub.sh");
    // `read`/`printf` are POSIX builtins, so this needs no `PATH`. The fixture
    // path and the workspace-write file are passed by absolute path. The stub
    // touches the workspace so the live write is observable on disk.
    let fixture_path = dir.join("write-fixture.jsonl");
    std::fs::write(&fixture_path, WRITE_FIXTURE).expect("write fixture");
    let script = format!(
        "#!/bin/sh\nprintf 'hello from codex\\n' > \"$PWD/{write_target_rel}\"\nwhile IFS= read -r line; do printf '%s\\n' \"$line\"; done < {fixture}\n",
        write_target_rel = write_target_rel,
        fixture = fixture_path.display(),
    );
    std::fs::write(&stub, script).expect("write stub");
    let mut perms = std::fs::metadata(&stub).expect("stub meta").permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&stub, perms).expect("chmod stub");
    stub
}

/// RTL9: the workspace-write round-trip records an OBSERVED tool result distinct
/// from the agent's reported claim, fully testable via the mock-output path.
#[test]
fn workspace_write_mock_round_trip_records_observed_tool_result_distinct_from_agent_claim() {
    let goal = "Apply a workspace-write edit to NOTES.md";
    let root = temp_root();
    let server = CapoServer::open(ProjectId::new("project-capo"), &root).expect("server");
    register_and_start(&server, "session-rtl9-mock", "run-rtl9-mock", goal);
    let plan_id = preflight(
        &server,
        "session-rtl9-mock",
        "run-rtl9-mock",
        "turn-rtl9-mock",
        goal,
    );

    let run = handle(
        &server,
        ServerCommand::RunLiveProviderLocal {
            dispatch_plan_id: plan_id,
            goal: goal.to_string(),
            live_execution_opt_in: false,
            mock_runtime_opt_in: true,
            mock_provider_output_name: Some("codex-exec-workspace-write.jsonl".to_string()),
            mock_provider_output_jsonl: Some(WRITE_FIXTURE.to_string()),
            timeout_seconds: 1,
            codex_program_override: None,
            unattended: true,
        },
    );
    let ServerResponsePayload::DispatchRun(run) = run.payload else {
        panic!("expected dispatch run response");
    };
    assert_eq!(run.status, "mocked_live_provider_output_ingested");
    assert!(!run.provider_cli_executed);
    // The fixture's tool round-trip yields a tool event (begin+end dedup to the
    // completed observation).
    assert!(run.tool_event_count >= 1);

    let state = SqliteStateStore::open(&root).expect("state");
    let session = SessionId::new("session-rtl9-mock");

    // The OBSERVED tool result: an `apply_patch` observation carrying the applied
    // diff/output, recorded as `tool.observation_recorded`.
    let observations = state
        .tool_observations_for_session(&session)
        .expect("observations");
    let write_observation = observations
        .iter()
        .find(|observation| observation.tool_name == "apply_patch")
        .expect("observed apply_patch tool result");
    assert_eq!(write_observation.observed_status, "completed");
    assert_eq!(write_observation.instrumentation_level, "observed_only");
    // The observed result is anchored to the artifact carrying the applied diff.
    assert!(write_observation.artifact_id.is_some());

    // The observed tool result is DISTINCT from the agent's reported claim: the
    // session summary records the agent message, not the tool's applied output.
    let session_projection = state.session(&session).expect("session").expect("present");
    let summary = session_projection.latest_summary.unwrap_or_default();
    // SLICE-A LEGIBILITY: the summary now carries the agent's REAL message prose
    // (legible chat/feed), and is still DISTINCT from the tool's applied output
    // (the NOTES.md diff / "hello from codex" the apply_patch observation holds).
    assert!(
        summary.contains("I will add a greeting to NOTES.md."),
        "agent claim summary should be the legible assistant message prose, got: {summary}"
    );
    assert!(
        !summary.contains("hello from codex"),
        "the summary is the agent's message, NOT the tool's applied output, got: {summary}"
    );

    // The observed tool-result event kind is recorded and is not the summary kind.
    let events = state
        .recent_events_for_session(&session, 64)
        .expect("events");
    assert!(
        events
            .iter()
            .any(|event| event.kind == "tool.observation_recorded"),
        "an observed tool-result event must be recorded"
    );
    assert!(
        events
            .iter()
            .any(|event| event.kind == "session.summary_updated"),
        "the agent's reported claim is a separate summary event"
    );
}

/// RTL9: the ingested write turn rebuilds identically after a restart/replay.
#[test]
fn ingested_write_turn_rebuilds_identically_after_restart_replay() {
    let goal = "Apply a workspace-write edit, then replay";
    let root = temp_root();
    let server = CapoServer::open(ProjectId::new("project-capo"), &root).expect("server");
    register_and_start(&server, "session-rtl9-replay", "run-rtl9-replay", goal);
    let plan_id = preflight(
        &server,
        "session-rtl9-replay",
        "run-rtl9-replay",
        "turn-rtl9-replay",
        goal,
    );
    handle(
        &server,
        ServerCommand::RunLiveProviderLocal {
            dispatch_plan_id: plan_id,
            goal: goal.to_string(),
            live_execution_opt_in: false,
            mock_runtime_opt_in: true,
            mock_provider_output_name: Some("codex-exec-workspace-write.jsonl".to_string()),
            mock_provider_output_jsonl: Some(WRITE_FIXTURE.to_string()),
            timeout_seconds: 1,
            codex_program_override: None,
            unattended: true,
        },
    );

    let session = SessionId::new("session-rtl9-replay");
    let snapshot = |store: &SqliteStateStore| {
        let observations = store
            .tool_observations_for_session(&session)
            .expect("observations")
            .into_iter()
            .map(|observation| {
                (
                    observation.tool_name,
                    observation.observed_status,
                    observation.artifact_id,
                )
            })
            .collect::<Vec<_>>();
        let tool_calls = store
            .tool_calls_for_session(&session)
            .expect("tool calls")
            .into_iter()
            .map(|call| (call.tool_name, call.status, call.turn_id))
            .collect::<Vec<_>>();
        let session_projection = store.session(&session).expect("session").expect("present");
        let event_count = store.event_count().expect("event count");
        (observations, tool_calls, session_projection, event_count)
    };

    let state = SqliteStateStore::open(&root).expect("state");
    let before = snapshot(&state);

    let reopened = SqliteStateStore::open(&root).expect("reopen");
    reopened.rebuild_projections().expect("rebuild");
    let after = snapshot(&reopened);

    assert_eq!(
        before, after,
        "ingested write turn must rebuild identically"
    );
    // The observed apply_patch result survives the replay.
    assert!(after.0.iter().any(|(tool_name, status, artifact)| {
        tool_name == "apply_patch" && status == "completed" && artifact.is_some()
    }));
}

/// RTL9: a `LiveWrite` spawn uses the workspace-write profile and engages the
/// RTL6 confinement + pre-write checkpoint BEFORE the provider runs; a `DryRun`
/// does neither. Both driven through a deterministic codex stub.
#[test]
fn live_write_uses_workspace_write_profile_and_checkpoints_before_spawn() {
    let goal = "Apply a confined workspace-write edit live".to_string();
    let root = temp_root();
    let server = CapoServer::open(ProjectId::new("project-capo"), &root).expect("server");

    let workspace = root.join("workspace");
    let artifacts = root.join("artifacts");
    std::fs::create_dir_all(&workspace).expect("workspace");
    // Seed a pre-existing file so the pre-write checkpoint has content to capture.
    std::fs::write(workspace.join("seed.txt"), b"seed\n").expect("seed");
    let workspace_str = workspace.to_string_lossy().to_string();
    let artifacts_str = artifacts.to_string_lossy().to_string();

    register_and_start(&server, "session-rtl9-live", "run-rtl9-live", &goal);
    // Preflight with the REAL confined workspace/artifacts so the plan's
    // runtime_cwd/artifact_root are the confined dirs the spawn uses.
    let preflight = handle(
        &server,
        ServerCommand::PreflightLiveProvider {
            agent_name: "codex-local".to_string(),
            adapter: "codex".to_string(),
            goal: goal.clone(),
            workspace: workspace_str.clone(),
            artifacts: artifacts_str.clone(),
            session_id: "session-rtl9-live".to_string(),
            run_id: "run-rtl9-live".to_string(),
            turn_id: "turn-rtl9-live".to_string(),
            capability_profile: "trusted-local".to_string(),
            runtime_scope: "local_process_loopback".to_string(),
            credential_scan_policy: "metadata_only_no_secret_read".to_string(),
            raw_prompt_policy: "not_rendered".to_string(),
            raw_output_policy: "artifacts_scanned_redacted".to_string(),
            tool_wrapper_policy: "capo_wrapped_required".to_string(),
            live_provider_opt_in: true,
        },
    );
    let ServerResponsePayload::LiveProviderPreflighted(preflight) = preflight.payload else {
        panic!("expected preflight response");
    };

    let stub = write_codex_stub(&root, "NOTES.md");
    let origin = system_origin();

    // LiveWrite: drives the workspace-write profile + checkpoint + spawn. We call
    // the crate-internal run path directly with `WriteMode::LiveWrite` so the
    // live-write decision is exercised without mutating the process-global
    // `CAPO_SERVER_RUN_CODEX_LIVE` env. The `record_selected_argv` seam captures
    // the argv of the launch plan the run path ACTUALLY selected, so we can prove
    // the write mode drove the workspace-write profile (not just that some plan
    // with that content exists at the adapter layer).
    let selected_argv = std::cell::RefCell::new(Vec::new());
    let run = server
        .run_live_provider_local(
            &origin,
            LiveProviderLocalRunRequest {
                dispatch_plan_id: &preflight.dispatch_plan_id,
                goal: &goal,
                live_execution_opt_in: true,
                mock_runtime_opt_in: false,
                mock_provider_output_name: None,
                mock_provider_output_jsonl: None,
                timeout_seconds: 10,
                codex_program_override: Some(stub.to_string_lossy().as_ref()),
                claude_program_override: None,
                write_mode: WriteMode::LiveWrite,
                record_selected_argv: Some(&selected_argv),
            },
        )
        .expect("live write run");

    // CORE ACCEPTANCE: the LiveWrite arm selected the workspace-write profile,
    // i.e. it spawns Codex with `--sandbox workspace-write` and NOT
    // `--sandbox read-only`. Asserting the selected argv (not just the adapter
    // plan's content) is what makes swapping this arm to the read-only profile a
    // test failure.
    let argv = selected_argv.borrow();
    assert!(
        argv.windows(2)
            .any(|pair| pair == ["--sandbox", "workspace-write"]),
        "LiveWrite must select the workspace-write profile, got argv: {argv:?}"
    );
    assert!(
        !argv.iter().any(|arg| arg == "read-only"),
        "LiveWrite must not select the read-only profile, got argv: {argv:?}"
    );

    // The provider actually executed (the stub ran) and the round-trip ingested.
    assert!(
        run.provider_cli_executed,
        "live write must spawn a provider"
    );
    assert_eq!(run.status, "exited");
    assert!(run.tool_event_count >= 1);

    // The pre-write checkpoint was recorded BEFORE the write, and the write
    // landed inside the confined workspace.
    let state = SqliteStateStore::open(&root).expect("state");
    let events = state
        .recent_events_for_session(&SessionId::new("session-rtl9-live"), 64)
        .expect("events");
    assert!(
        events
            .iter()
            .any(|event| event.kind == "checkpoint.created"),
        "a pre-write checkpoint must be recorded for a live write"
    );
    assert!(
        workspace.join("NOTES.md").exists(),
        "the confined live write must land in the workspace"
    );

    // The observed apply_patch tool result is recorded (the live round-trip).
    let observations = state
        .tool_observations_for_session(&SessionId::new("session-rtl9-live"))
        .expect("observations");
    assert!(
        observations
            .iter()
            .any(|observation| observation.tool_name == "apply_patch"),
        "the live write round-trip must record the observed apply_patch result"
    );
}

/// RTL9: without the live-write opt-in the run stays on the read-only profile
/// and takes no checkpoint -- the dry-run default.
#[test]
fn default_run_stays_read_only_and_takes_no_checkpoint() {
    let goal = "A default run must not write or checkpoint".to_string();
    let root = temp_root();
    let server = CapoServer::open(ProjectId::new("project-capo"), &root).expect("server");

    let workspace = root.join("workspace");
    let artifacts = root.join("artifacts");
    std::fs::create_dir_all(&workspace).expect("workspace");
    let workspace_str = workspace.to_string_lossy().to_string();
    let artifacts_str = artifacts.to_string_lossy().to_string();

    register_and_start(&server, "session-rtl9-dry", "run-rtl9-dry", &goal);
    let preflight = handle(
        &server,
        ServerCommand::PreflightLiveProvider {
            agent_name: "codex-local".to_string(),
            adapter: "codex".to_string(),
            goal: goal.clone(),
            workspace: workspace_str.clone(),
            artifacts: artifacts_str.clone(),
            session_id: "session-rtl9-dry".to_string(),
            run_id: "run-rtl9-dry".to_string(),
            turn_id: "turn-rtl9-dry".to_string(),
            capability_profile: "trusted-local".to_string(),
            runtime_scope: "local_process_loopback".to_string(),
            credential_scan_policy: "metadata_only_no_secret_read".to_string(),
            raw_prompt_policy: "not_rendered".to_string(),
            raw_output_policy: "artifacts_scanned_redacted".to_string(),
            tool_wrapper_policy: "capo_wrapped_required".to_string(),
            live_provider_opt_in: true,
        },
    );
    let ServerResponsePayload::LiveProviderPreflighted(preflight) = preflight.payload else {
        panic!("expected preflight response");
    };

    let stub = write_codex_stub(&root, "NOTES.md");
    let origin = system_origin();

    let selected_argv = std::cell::RefCell::new(Vec::new());
    let run = server
        .run_live_provider_local(
            &origin,
            LiveProviderLocalRunRequest {
                dispatch_plan_id: &preflight.dispatch_plan_id,
                goal: &goal,
                live_execution_opt_in: true,
                mock_runtime_opt_in: false,
                mock_provider_output_name: None,
                mock_provider_output_jsonl: None,
                timeout_seconds: 10,
                codex_program_override: Some(stub.to_string_lossy().as_ref()),
                claude_program_override: None,
                // The DRY-RUN default: no live write, even with the caller opt-in,
                // because the env gate / attended conditions did not resolve to a
                // live write upstream.
                write_mode: WriteMode::DryRun,
                record_selected_argv: Some(&selected_argv),
            },
        )
        .expect("dry-run run");

    // The DryRun default selected the read-only profile (the mirror of the
    // LiveWrite assertion): it spawns with `--sandbox read-only` and never
    // `--sandbox workspace-write`.
    let argv = selected_argv.borrow();
    assert!(
        argv.windows(2)
            .any(|pair| pair == ["--sandbox", "read-only"]),
        "DryRun must select the read-only profile, got argv: {argv:?}"
    );
    assert!(
        !argv
            .windows(2)
            .any(|pair| pair == ["--sandbox", "workspace-write"]),
        "DryRun must not select the workspace-write profile, got argv: {argv:?}"
    );
    drop(argv);

    // It still ran the (read-only) provider, but took NO checkpoint.
    assert!(run.provider_cli_executed);
    let state = SqliteStateStore::open(&root).expect("state");
    let events = state
        .recent_events_for_session(&SessionId::new("session-rtl9-dry"), 64)
        .expect("events");
    assert!(
        !events
            .iter()
            .any(|event| event.kind == "checkpoint.created"),
        "a dry-run default must not take a pre-write checkpoint"
    );
}
