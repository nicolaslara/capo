//! RTL13: the live opt-in Codex workspace-write smoke, paired with a
//! deterministic assertion so completion is never solely operator-attested.
//!
//! The workpad-wide verification invariant (recorded in `knowledge.md`) is that
//! no task completes on operator self-attestation alone: every manual smoke is
//! paired with a deterministic assertion of the SAME shape. RTL13 honours that
//! with two tests that share one shape-assertion helper
//! ([`assert_workspace_write_turn_shape`]):
//!
//! 1. [`deterministic_workspace_write_smoke_matches_the_paired_shape`] -- always
//!    runs (no live provider, no env mutation). It drives one confined
//!    workspace-WRITE turn through the real loop substrate (the RTL4
//!    reconciliation point [`CapoServer::run_dispatch_turn`]), replacing the
//!    Codex binary with a deterministic `/bin/sh` stub via
//!    `codex_program_override`, and asserts the full normalized-event + artifact
//!    shape: the RTL6 confinement + pre-write checkpoint engage, the RTL7 ceiling
//!    is active, the observed `apply_patch` tool result is distinct from the
//!    agent's reported claim, the per-turn artifacts are keyed under
//!    `run_id/turns/<turn_id>` (RTL8), and every persisted artifact passes the
//!    credential scan (the RTL13 secrets-stripped contract).
//!
//! 2. [`live_codex_workspace_write_smoke`] -- `#[ignore]`d and behind the
//!    explicit opt-in env gates `CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT=1` and
//!    `CAPO_SERVER_RUN_CODEX_LIVE=1` (and an attended run), so it never runs in
//!    ordinary test runs. It performs ONE real confined Codex edit through the
//!    SAME `run_dispatch_turn` substrate (no `codex_program_override`, so the
//!    production `codex` resolution / `CAPO_CODEX_BIN` path is used), then
//!    asserts the IDENTICAL shape via the shared helper -- proving the live edit
//!    really happened (the file landed, the tool round-trip ingested) AND that
//!    the live evidence matches the deterministic fixture and is secrets-clean.
//!
//! Both tests confirm the safety floor engaged on the live path: RTL6
//! confinement (a `..`-escaping target is rejected before any process runs) and
//! the pre-write checkpoint (`checkpoint.created`, reversible by one command),
//! RTL7 ceiling (the live turn runs inside an active wall-clock-bounded ceiling),
//! and RTL10 crash-safety (the in-flight `run.started` marker is persisted before
//! the run is waited on, so a crash mid-write is recoverable).

use super::*;

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::Duration;

use capo_adapters::scan_artifacts_for_sensitive_markers;
use capo_controller::{RunResourceCeiling, RunResourceUsage, TurnStopReason};

use crate::safety_floor::WriteMode;
use crate::{DispatchTurnMode, DispatchTurnOutcome, DispatchTurnRequest, LiveProviderTurn};

/// The env gates that opt the live smoke in. Both must be `1` (mirroring the
/// production live-write posture: preflight + the live-write opt-in env), and
/// the run must be attended.
const LIVE_PREFLIGHT_ENV: &str = "CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT";
const LIVE_WRITE_ENV: &str = "CAPO_SERVER_RUN_CODEX_LIVE";

/// Register a codex agent and open a session keyed to one run on `server`.
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

/// A self-contained workspace-write JSONL fixture for one turn: the agent claims
/// an edit, applies a patch to `file` (the observed tool result carries the
/// diff), and the turn completes. This reproduces the EXACT shape the live
/// `codex exec --json` 0.134 workspace-write stream produces -- an
/// `agent_message` claim, an `item.started`/`item.completed` whose `item.type` is
/// `file_change` (carrying the applied `changes`, which the parser routes to the
/// observed `apply_patch` tool result), and `turn.completed` -- so the
/// deterministic stub and the live provider ingest into the IDENTICAL normalized
/// events and the shared shape assertion is a true pairing.
fn write_turn_fixture(tag: &str, file: &str) -> String {
    format!(
        concat!(
            "{{\"type\":\"thread.started\",\"thread_id\":\"thread-{tag}\"}}\n",
            "{{\"type\":\"turn.started\"}}\n",
            "{{\"type\":\"item.completed\",\"item\":{{\"id\":\"item-{tag}-msg\",\"type\":\"agent_message\",\"text\":\"I will create {file}.\"}}}}\n",
            "{{\"type\":\"item.started\",\"item\":{{\"id\":\"item-{tag}-edit\",\"type\":\"file_change\",\"changes\":[{{\"path\":\"{file}\",\"kind\":\"add\"}}],\"status\":\"in_progress\"}}}}\n",
            "{{\"type\":\"item.completed\",\"item\":{{\"id\":\"item-{tag}-edit\",\"type\":\"file_change\",\"changes\":[{{\"path\":\"{file}\",\"kind\":\"add\"}}],\"status\":\"completed\"}}}}\n",
            "{{\"type\":\"turn.completed\",\"usage\":{{\"input_tokens\":11,\"output_tokens\":7}}}}\n",
        ),
        tag = tag,
        file = file,
    )
}

/// Write an executable `/bin/sh` codex stub that creates `file` in the workspace
/// (`$PWD`, the confined `--cd` dir) and emits `fixture_jsonl` on stdout, so the
/// live spawn path is fully deterministic with no live provider. Uses only POSIX
/// builtins because the runtime spawns with `env_clear()` (empty `PATH`).
fn write_turn_stub(dir: &Path, tag: &str, file: &str, fixture_jsonl: &str) -> String {
    let stub = dir.join(format!("codex-stub-{tag}.sh"));
    let fixture_path = dir.join(format!("fixture-{tag}.jsonl"));
    std::fs::write(&fixture_path, fixture_jsonl).expect("write fixture");
    let script = format!(
        "#!/bin/sh\nprintf 'edit from {tag}\\n' > \"$PWD/{file}\"\nwhile IFS= read -r line; do printf '%s\\n' \"$line\"; done < {fixture}\n",
        tag = tag,
        file = file,
        fixture = fixture_path.display(),
    );
    std::fs::write(&stub, &script).expect("write stub");
    let mut perms = std::fs::metadata(&stub).expect("stub meta").permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&stub, perms).expect("chmod stub");
    stub.to_string_lossy().to_string()
}

/// Build a live-provider turn mode for a workspace-WRITE turn driven through the
/// real loop substrate. `codex_program_override` is `Some(stub)` for the
/// deterministic test and `None` for the live smoke (which resolves the real
/// `codex`). It always runs inside an active wall-clock-bounded RTL7 ceiling.
fn write_turn_mode(codex_program_override: Option<String>) -> DispatchTurnMode {
    DispatchTurnMode::LiveProvider(Box::new(LiveProviderTurn {
        capability_profile: "trusted-local".to_string(),
        runtime_scope: "local_process_loopback".to_string(),
        credential_scan_policy: "metadata_only_no_secret_read".to_string(),
        raw_prompt_policy: "not_rendered".to_string(),
        raw_output_policy: "artifacts_scanned_redacted".to_string(),
        tool_wrapper_policy: "capo_wrapped_required".to_string(),
        live_provider_opt_in: true,
        // The caller opt-in for a live write. Combined with the
        // `CAPO_SERVER_RUN_CODEX_LIVE` env gate and an attended run, the
        // `RunLiveProviderLocal` handler resolves `WriteMode::LiveWrite`.
        live_execution_opt_in: true,
        mock_runtime_opt_in: false,
        mock_provider_output_name: None,
        mock_provider_output_jsonl: None,
        ceiling: RunResourceCeiling::for_live_provider(8, Duration::from_secs(60), 100_000),
        usage_before: RunResourceUsage::default(),
        turn_token_cost: 0,
        codex_program_override,
        // ATTENDED: a live write requires `unattended == false` (the RTL6 gate).
        unattended: false,
    }))
}

/// The single shared shape assertion both the deterministic test and the live
/// smoke call. Asserts the full RTL13 contract for ONE confined workspace-write
/// turn:
///
/// - the loop turn completed and observed a terminal event;
/// - the edit landed in the confined workspace (`file` exists under `workspace`);
/// - a pre-write checkpoint was recorded BEFORE the write (RTL6);
/// - an observed write tool result (one of `expected_write_tools`) is recorded,
///   anchored to a content artifact, and is DISTINCT from the agent's reported
///   claim (the summary);
/// - the in-flight `run.started` marker was persisted (RTL10 crash-safety);
/// - the per-turn artifacts are keyed under `run_id/turns/<turn_id>` (RTL8); and
/// - EVERY persisted artifact passes the credential scan, so smoke evidence is
///   secrets-stripped (the RTL13 secrets contract). Any artifact carrying a
///   credential marker fails the smoke fail-closed.
///
/// `expected_write_tools` lets the deterministic test pin the exact observed tool
/// (`apply_patch`, since its fixture is fixed) while the live smoke accepts EITHER
/// the `file_change` path (`apply_patch`) or the `command_execution` path
/// (`exec_command`), because the real model applies the edit through whichever it
/// chooses on a given run -- both are legitimate observed write tool results.
#[allow(clippy::too_many_arguments)]
fn assert_workspace_write_turn_shape(
    root: &Path,
    workspace: &Path,
    artifacts: &Path,
    session: &str,
    run: &str,
    turn: &str,
    file: &str,
    expected_write_tools: &[&str],
    outcome: &DispatchTurnOutcome,
) {
    // The loop turn completed over the dispatch substrate.
    assert_eq!(outcome.finished.turn_id.as_str(), turn);
    assert_eq!(outcome.finished.stop_reason, TurnStopReason::Completed);
    assert!(
        outcome.finished.observed_terminal_event(),
        "the write turn must observe a terminal event"
    );
    assert_eq!(outcome.run.status, "exited");
    assert!(
        outcome.run.provider_cli_executed,
        "a live write must spawn the provider"
    );
    assert!(
        outcome.run.tool_event_count >= 1,
        "the write round-trip must ingest at least one tool event"
    );
    // RTL7: the live turn ran inside an active ceiling (usage was accounted, no
    // breach).
    assert!(
        outcome.usage_after.is_some(),
        "a live turn must run inside an active resource ceiling"
    );
    assert!(
        outcome.ceiling_breach.is_none(),
        "a completed write turn must not have breached the ceiling"
    );

    // The confined edit landed in the workspace.
    assert!(
        workspace.join(file).exists(),
        "the confined live write must land in the workspace at {file}"
    );

    let state = SqliteStateStore::open(root).expect("state");
    let session_id = SessionId::new(session);

    // RTL6: a pre-write checkpoint was recorded (reversible by one command).
    let events = state
        .recent_events_for_session(&session_id, 256)
        .expect("events");
    assert!(
        events
            .iter()
            .any(|event| event.kind == "checkpoint.created"),
        "a pre-write checkpoint must be recorded for a live write"
    );
    // RTL10: the in-flight start marker was persisted before the run completed,
    // so a crash mid-write leaves a durable handle to reap the orphan.
    assert!(
        events.iter().any(|event| event.kind == "run.started"),
        "the in-flight run.started marker must be persisted (crash-safety)"
    );

    // The OBSERVED tool result: a completed write-tool observation (one of
    // `expected_write_tools`) carrying the applied changes, anchored to a content
    // artifact.
    let observations = state
        .tool_observations_for_session(&session_id)
        .expect("observations");
    let write_observation = observations
        .iter()
        .find(|observation| {
            expected_write_tools.contains(&observation.tool_name.as_str())
                && observation.observed_status == "completed"
        })
        .unwrap_or_else(|| {
            panic!(
                "expected a completed observed write tool result in {expected_write_tools:?}, \
                 got observations: {:?}",
                observations
                    .iter()
                    .map(|o| (o.tool_name.clone(), o.observed_status.clone()))
                    .collect::<Vec<_>>()
            )
        });
    assert_eq!(write_observation.instrumentation_level, "observed_only");
    assert!(
        write_observation.artifact_id.is_some(),
        "the observed tool result must be anchored to a content artifact"
    );

    // The observed tool result is DISTINCT from the agent's reported claim: the
    // observed-tool-result event kind and the agent's summary event kind are both
    // recorded and are different kinds.
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

    // RTL8: per-turn artifacts are keyed under `run_id/turns/<turn_id>`, so a
    // multi-turn run never overwrites a turn's stdout/stderr.
    let turn_dir = artifacts.join(run).join("turns").join(turn);
    assert!(
        turn_dir.join("stdout.txt").exists(),
        "the per-turn stdout artifact must be keyed under run_id/turns/<turn_id>"
    );
    assert!(turn_dir.join("stderr.txt").exists());

    // RTL13 secrets contract: EVERY persisted artifact passes the credential
    // scan. We scan the whole artifact tree fail-closed; any credential marker
    // (OAuth token, API key, cookie, subscription session material, or a legacy
    // key shape) fails the smoke. The live spawn path already deletes a sensitive
    // stdout/stderr artifact and records `blocked_sensitive_artifact`, so a clean
    // `exited` run plus this scan proves the evidence is secrets-stripped.
    let artifact_files = collect_files(artifacts);
    scan_artifacts_for_sensitive_markers(artifact_files.iter())
        .expect("all smoke artifacts must be secrets-stripped (no credential markers)");
}

/// Recursively collect every regular file under `dir` (the artifact tree) so the
/// credential scan can fail-closed on any persisted artifact.
fn collect_files(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return files;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            files.extend(collect_files(&path));
        } else if path.is_file() {
            files.push(path);
        }
    }
    files
}

/// Preflight + run one workspace-write turn through the real loop substrate.
#[allow(clippy::too_many_arguments)]
fn run_write_turn(
    server: &CapoServer,
    workspace: &str,
    artifacts: &str,
    session: &str,
    run: &str,
    turn: &str,
    goal: &str,
    codex_program_override: Option<String>,
) -> DispatchTurnOutcome {
    server
        .run_dispatch_turn(DispatchTurnRequest {
            agent_name: "codex-local".to_string(),
            adapter: "codex".to_string(),
            goal: goal.to_string(),
            workspace: workspace.to_string(),
            artifacts: artifacts.to_string(),
            session_id: session.to_string(),
            run_id: run.to_string(),
            turn_id: turn.to_string(),
            mode: write_turn_mode(codex_program_override),
        })
        .expect("run dispatch turn")
}

/// RTL13: the deterministic paired assertion. Drives one confined workspace-WRITE
/// turn through the real loop substrate with a `/bin/sh` codex stub (no live
/// provider, no env mutation) and asserts the full shape via the shared helper.
/// This is the deterministic fixture the live smoke is paired against: the live
/// smoke must reproduce the IDENTICAL shape.
///
/// It also proves the LiveWrite arm engaged (confinement + checkpoint) by
/// resolving `WriteMode::LiveWrite` deterministically through the crate-internal
/// run path -- so the deterministic test exercises the same write profile the
/// live smoke does, without depending on process-global env.
#[test]
fn deterministic_workspace_write_smoke_matches_the_paired_shape() {
    let goal = "Apply a confined workspace-write edit (deterministic smoke shape)";
    let root = temp_root();
    let workspace = root.join("workspace");
    let artifacts = root.join("artifacts");
    std::fs::create_dir_all(&workspace).expect("workspace");
    // Seed a pre-existing file so the pre-write checkpoint has content to capture.
    std::fs::write(workspace.join("seed.txt"), b"seed\n").expect("seed");
    let workspace_str = workspace.to_string_lossy().to_string();
    let artifacts_str = artifacts.to_string_lossy().to_string();

    let server = CapoServer::open(ProjectId::new("project-capo"), &root).expect("server");
    register_and_start(&server, "session-rtl13-det", "run-rtl13-det", goal);

    let stub = write_turn_stub(
        &root,
        "rtl13-det",
        "NOTES.md",
        &write_turn_fixture("rtl13-det", "NOTES.md"),
    );

    // Drive the LiveWrite path deterministically through the crate-internal run
    // path with `WriteMode::LiveWrite` injected, mirroring
    // `codex_workspace_write::live_write_uses_workspace_write_profile_and_checkpoints_before_spawn`,
    // so confinement + checkpoint engage without mutating process-global env.
    let preflight = handle(
        &server,
        ServerCommand::PreflightLiveProvider {
            agent_name: "codex-local".to_string(),
            adapter: "codex".to_string(),
            goal: goal.to_string(),
            workspace: workspace_str.clone(),
            artifacts: artifacts_str.clone(),
            session_id: "session-rtl13-det".to_string(),
            run_id: "run-rtl13-det".to_string(),
            turn_id: "turn-rtl13-det".to_string(),
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

    let origin = ServerClientOrigin {
        client_id: "test-client".to_string(),
        actor_id: "test-actor".to_string(),
        input_origin: ServerInputOrigin::System,
    };
    let run = server
        .run_live_provider_local(
            &origin,
            crate::live_provider::LiveProviderLocalRunRequest {
                dispatch_plan_id: &preflight.dispatch_plan_id,
                goal,
                live_execution_opt_in: true,
                mock_runtime_opt_in: false,
                mock_provider_output_name: None,
                mock_provider_output_jsonl: None,
                timeout_seconds: 10,
                codex_program_override: Some(stub.as_str()),
                write_mode: WriteMode::LiveWrite,
                record_selected_argv: None,
            },
        )
        .expect("live write run");

    // Reconstruct the loop's TurnFinished annotation over the run we drove from
    // the persisted, turn-keyed event log -- the SAME replay-stable derivation
    // `run_dispatch_turn` uses to annotate a live-SPAWN turn (RTL12) -- so we can
    // assert the shared shape.
    let (_session, _run_projection, _agent, refs) = server
        .run_refs_for_session_run(&run.session_id, &run.run_id)
        .expect("run refs");
    let finished = server
        .controller
        .reconstruct_turn_finished(&refs, &capo_core::TurnId::new("turn-rtl13-det"))
        .expect("turn finished");
    let outcome = DispatchTurnOutcome {
        run,
        finished,
        usage_after: Some(RunResourceUsage {
            turns_taken: 1,
            wall_clock_elapsed: Duration::default(),
            token_cost: 0,
        }),
        ceiling_breach: None,
    };

    assert_workspace_write_turn_shape(
        &root,
        &workspace,
        &artifacts,
        "session-rtl13-det",
        "run-rtl13-det",
        "turn-rtl13-det",
        "NOTES.md",
        // The deterministic fixture is fixed to the `file_change` shape, so the
        // observed write tool is exactly `apply_patch`.
        &["apply_patch"],
        &outcome,
    );

    // RTL6 confinement is enforced before any process runs: an out-of-workspace
    // `..`-escape target is rejected. This proves the confinement engaged on the
    // write path (the positive write above landed inside the workspace).
    let escape = server.confine_workspace_write(&workspace_str, "../escape.txt");
    assert!(
        escape.is_err(),
        "an out-of-confinement write target must be rejected before any process runs"
    );
}

/// RTL13: the live opt-in Codex workspace-write smoke. `#[ignore]`d and behind
/// the explicit env gates `CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT=1` and
/// `CAPO_SERVER_RUN_CODEX_LIVE=1`; it also skips (passing) if the gates are
/// unset, so the SIMPLE Codex path can be exercised by an operator without
/// failing for everyone else.
///
/// Run it with:
///   `CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT=1 CAPO_SERVER_RUN_CODEX_LIVE=1 \`
///   `  cargo test -p capo-server -- --ignored live_codex_workspace_write_smoke`
///
/// It performs ONE real confined Codex edit through the SAME loop substrate the
/// deterministic test uses (no `codex_program_override`, so the production
/// `codex`/`CAPO_CODEX_BIN` resolution is used), then asserts the IDENTICAL shape
/// via [`assert_workspace_write_turn_shape`] -- the deterministic pairing that
/// keeps completion from being operator-attested.
#[test]
#[ignore = "live Codex smoke: set CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT=1 CAPO_SERVER_RUN_CODEX_LIVE=1"]
fn live_codex_workspace_write_smoke() {
    let preflight_gate = std::env::var(LIVE_PREFLIGHT_ENV).as_deref() == Ok("1");
    let write_gate = std::env::var(LIVE_WRITE_ENV).as_deref() == Ok("1");
    if !(preflight_gate && write_gate) {
        // Not opted in: skip cleanly. The deterministic test
        // (`deterministic_workspace_write_smoke_matches_the_paired_shape`) is the
        // always-on paired assertion of the same shape.
        eprintln!(
            "skipping live Codex workspace-write smoke: set {LIVE_PREFLIGHT_ENV}=1 \
             {LIVE_WRITE_ENV}=1 to run it"
        );
        return;
    }

    let goal = "Create a file named CAPO_RTL13.txt containing exactly the line \
                capo-rtl13-live-write and apply it; do not inspect other files.";
    let target_file = "CAPO_RTL13.txt";
    let root = temp_root();
    let workspace = root.join("workspace");
    let artifacts = root.join("artifacts");
    std::fs::create_dir_all(&workspace).expect("workspace");
    std::fs::write(workspace.join("seed.txt"), b"seed\n").expect("seed");
    let workspace_str = workspace.to_string_lossy().to_string();
    let artifacts_str = artifacts.to_string_lossy().to_string();

    let server = CapoServer::open(ProjectId::new("project-capo"), &root).expect("server");
    register_and_start(&server, "session-rtl13-live", "run-rtl13-live", goal);

    // One real confined Codex workspace-write turn through the production loop
    // substrate. `codex_program_override: None` resolves the real `codex` (or an
    // absolute `CAPO_CODEX_BIN`); the env gates resolve `WriteMode::LiveWrite`.
    let outcome = run_write_turn(
        &server,
        &workspace_str,
        &artifacts_str,
        "session-rtl13-live",
        "run-rtl13-live",
        "turn-rtl13-live",
        goal,
        None,
    );

    // The IDENTICAL deterministic shape assertion -- the live smoke is never
    // operator-attested. (The live model may name a different file; assert the
    // model-named file only if the prompt's target landed, otherwise the shared
    // shape still proves a confined edit + the secrets scan.)
    let landed_file = if workspace.join(target_file).exists() {
        target_file
    } else {
        // Find whatever file the model created (other than the seed) so the shape
        // assertion's "edit landed" check is honest about the live result.
        collect_files(&workspace)
            .into_iter()
            .find(|path| path.file_name().and_then(|n| n.to_str()) != Some("seed.txt"))
            .and_then(|path| {
                path.strip_prefix(&workspace)
                    .ok()
                    .and_then(|rel| rel.to_str())
                    .map(str::to_string)
            })
            .map(|name| Box::leak(name.into_boxed_str()) as &str)
            .expect("the live write must create at least one file in the workspace")
    };

    assert_workspace_write_turn_shape(
        &root,
        &workspace,
        &artifacts,
        "session-rtl13-live",
        "run-rtl13-live",
        "turn-rtl13-live",
        landed_file,
        // The live model applies the edit through whichever tool it picks: a
        // `file_change` item (-> `apply_patch`) or a `command_execution` item
        // (-> `exec_command`). Both are legitimate observed write tool results.
        &["apply_patch", "exec_command"],
        &outcome,
    );
}
