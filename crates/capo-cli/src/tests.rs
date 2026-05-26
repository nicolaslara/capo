use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use super::*;
use crate::adapter_dispatch_run::scan_dispatch_artifacts_or_delete;
use capo_adapters::LocalAdapterSmokeError;
use capo_state::ConnectivityExposureProjection;

#[test]
fn help_mentions_command_envelopes_and_no_credentials() {
    assert!(HELP.contains("command envelopes"));
    assert!(HELP.contains("does not read provider credentials"));
    assert!(HELP.contains("adapter readiness"));
    assert!(HELP.contains("adapter plan-launch"));
    assert!(HELP.contains("adapter dispatch-gate"));
    assert!(HELP.contains("adapter dispatch-status"));
    assert!(HELP.contains("adapter dispatch-evidence"));
    assert!(HELP.contains("adapter smoke-report status"));
    assert!(HELP.contains("adapter smoke-report evidence"));
    assert!(HELP.contains("adapter execution-request"));
    assert!(HELP.contains("adapter materialize-prompt"));
    assert!(HELP.contains("adapter run-preflight"));
    assert!(HELP.contains("adapter run-local"));
    assert!(HELP.contains("adapter replay-dispatch"));
    assert!(HELP.contains("adapter dogfood-gate"));
    assert!(HELP.contains("dogfood readiness"));
    assert!(HELP.contains("runtime target register"));
    assert!(HELP.contains("runtime target set-status"));
    assert!(HELP.contains("runtime target status"));
    assert!(HELP.contains("runtime target evidence"));
    assert!(HELP.contains("runtime target list"));
    assert!(HELP.contains("connectivity expose-stub"));
    assert!(HELP.contains("connectivity request-approval"));
    assert!(HELP.contains("connectivity activate-exposure"));
    assert!(HELP.contains("connectivity revoke-exposure"));
    assert!(HELP.contains("connectivity exposure-status"));
    assert!(HELP.contains("connectivity exposure-evidence"));
    assert!(HELP.contains("workpad index"));
    assert!(HELP.contains("workpad next"));
    assert!(HELP.contains("workpad plan-next"));
    assert!(HELP.contains("workpad start-next"));
    assert!(HELP.contains("workpad propose"));
    assert!(HELP.contains("workpad apply"));
    assert!(HELP.contains("tool run-wrapper"));
}

#[test]
fn tool_run_wrapper_exposes_governed_runtime_wrappers_without_providers() {
    let workspace = temp_root("cli-tool-wrapper-workspace");
    let artifacts = temp_root("cli-tool-wrapper-artifacts");
    let state_root = temp_root("cli-tool-wrapper-state");
    fs::create_dir_all(&workspace).expect("workspace");
    Command::new("git")
        .args(["init"])
        .current_dir(&workspace)
        .output()
        .expect("git init");
    fs::write(workspace.join("tracked.txt"), "tracked\n").expect("tracked");

    let read_only_status = run_cli(vec![
        "tool".to_string(),
        "run-wrapper".to_string(),
        "--tool".to_string(),
        "git_status".to_string(),
        "--workspace".to_string(),
        workspace.display().to_string(),
        "--artifacts".to_string(),
        artifacts.display().to_string(),
        "--record".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("read-only git status");
    assert!(read_only_status.contains("wrapper_tool_run=true"));
    assert!(read_only_status.contains("tool=capo.git_status"));
    assert!(read_only_status.contains("tool_call=cli-wrapper-"));
    assert!(read_only_status.contains("session_id=session-cli-wrapper-"));
    assert!(read_only_status.contains("policy=read-only-local"));
    assert!(read_only_status.contains("permission_effect=allow"));
    assert!(read_only_status.contains("recorded=true"));
    assert!(read_only_status.contains("recorded_sequence="));
    assert!(read_only_status.contains("input_artifact=artifact-wrapper-"));
    assert!(read_only_status.contains("output_artifacts=2"));
    assert!(read_only_status.contains("output_artifact="));
    assert!(read_only_status.contains("audit_event=tool.invocation_started"));
    assert!(!read_only_status.contains("provider_cli_executed=true"));
    let dashboard = run_cli(vec![
        "dashboard".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("dashboard after recorded wrapper");
    assert!(dashboard.contains("agent=cli-wrapper"));
    assert!(dashboard.contains("tool_calls=1"));
    assert!(dashboard.contains("tool_call=cli-wrapper-"));
    assert!(dashboard.contains("tool=capo.git_status"));
    assert!(dashboard.contains("tool_origin=capo_wrapper"));
    assert!(dashboard.contains("input_artifact=artifact-wrapper-"));

    Command::new("git")
        .args(["add", "tracked.txt"])
        .current_dir(&workspace)
        .output()
        .expect("git add");
    let denied_commit = run_cli(vec![
        "tool".to_string(),
        "run-wrapper".to_string(),
        "--tool".to_string(),
        "git_commit".to_string(),
        "--workspace".to_string(),
        workspace.display().to_string(),
        "--artifacts".to_string(),
        artifacts.display().to_string(),
        "--message".to_string(),
        "Denied wrapper commit".to_string(),
    ])
    .expect("denied commit");
    assert!(denied_commit.contains("tool=capo.git_commit"));
    assert!(denied_commit.contains("policy=read-only-local"));
    assert!(denied_commit.contains("status=denied"));
    assert!(denied_commit.contains("permission_effect=deny"));
    assert!(denied_commit.contains("git:commit:workspace"));
    assert!(!denied_commit.contains("audit_event=tool.invocation_started"));

    let trusted_commit = run_cli(vec![
        "tool".to_string(),
        "run-wrapper".to_string(),
        "--tool".to_string(),
        "git_commit".to_string(),
        "--workspace".to_string(),
        workspace.display().to_string(),
        "--artifacts".to_string(),
        artifacts.display().to_string(),
        "--policy".to_string(),
        "trusted-local".to_string(),
        "--message".to_string(),
        "Trusted wrapper commit".to_string(),
    ])
    .expect("trusted commit");
    assert!(trusted_commit.contains("status=exited"));
    assert!(trusted_commit.contains("permission_effect=allow"));
    assert!(trusted_commit.contains("output_artifacts=2"));
    assert!(trusted_commit.contains("kind=git_commit_stdout"));
    assert!(trusted_commit.contains("kind=git_commit_stderr"));
    let log = Command::new("git")
        .args(["log", "--oneline", "-1"])
        .current_dir(&workspace)
        .output()
        .expect("git log");
    assert!(String::from_utf8_lossy(&log.stdout).contains("Trusted wrapper commit"));
}

#[test]
fn dispatch_artifact_scan_deletes_sensitive_outputs_on_failure() {
    let artifact_root = temp_root("dispatch-sensitive-artifacts");
    fs::create_dir_all(&artifact_root).expect("artifact root");
    let stdout = artifact_root.join("stdout.txt");
    let stderr = artifact_root.join("stderr.txt");
    fs::write(&stdout, "Authorization: leaked\n").expect("stdout");
    fs::write(&stderr, "ordinary stderr\n").expect("stderr");

    let error = scan_dispatch_artifacts_or_delete([&stdout, &stderr])
        .expect_err("sensitive marker should fail scan");
    assert!(matches!(
        error,
        LocalAdapterSmokeError::SensitiveArtifact { .. }
    ));
    assert!(!stdout.exists());
    assert!(!stderr.exists());
}

#[test]
fn adapter_plan_launch_builds_dispatch_contract_without_running_provider_cli() {
    let state_root = temp_root("adapter-plan-launch-state");
    let workspace = temp_root("adapter-plan-launch-workspace");
    let artifacts = temp_root("adapter-plan-launch-artifacts");
    let output = run_cli(vec![
        "adapter".to_string(),
        "plan-launch".to_string(),
        "--adapter".to_string(),
        "codex".to_string(),
        "--agent".to_string(),
        "codex-worker".to_string(),
        "--goal".to_string(),
        "Summarize this workpad without printing this prompt.".to_string(),
        "--workspace".to_string(),
        workspace.display().to_string(),
        "--artifacts".to_string(),
        artifacts.display().to_string(),
        "--record".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("adapter plan launch");

    assert!(output.contains("adapter_launch_planned=true"));
    assert!(output.contains("adapter=codex_exec"));
    assert!(output.contains("provider_kind=codex_subscription"));
    assert!(output.contains("credential_scope=user_local_subscription"));
    assert!(output.contains("runtime_program=codex"));
    assert!(output.contains("runtime_prompt_policy=not_rendered"));
    assert!(output.contains("runtime_prompt_source_kind=inline_cli_prompt"));
    assert!(output.contains("runtime_prompt_materialization=manual_prompt_not_replayable"));
    assert!(output.contains("request_env_count=0"));
    assert!(output.contains("subscription_safe=true"));
    assert!(output.contains("provider_cli_executed=false"));
    assert!(output.contains("recorded=true"));
    assert!(output.contains(&format!("runtime_cwd={}", workspace.display())));
    assert!(output.contains(&format!("artifact_root={}", artifacts.display())));
    assert!(!output.contains("Summarize this workpad"));
    assert!(!workspace.exists());
    assert!(!artifacts.exists());
    let plans = SqliteStateStore::open(&state_root)
        .expect("state")
        .adapter_dispatch_plans(&project_id())
        .expect("dispatch plans");
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].adapter_kind, "codex_exec");
    assert_eq!(plans[0].runtime_prompt_policy, "not_rendered");
    assert!(!plans[0].provider_cli_executed);
    let prompt_sources = SqliteStateStore::open(&state_root)
        .expect("state")
        .adapter_dispatch_prompt_sources(&project_id())
        .expect("dispatch prompt sources");
    assert_eq!(prompt_sources.len(), 1);
    assert_eq!(prompt_sources[0].source_kind, "inline_cli_prompt");
    assert_eq!(
        prompt_sources[0].materialization_status,
        "manual_prompt_not_replayable"
    );
    assert_eq!(prompt_sources[0].raw_prompt_policy, "not_rendered");
    let materialize = run_cli(vec![
        "adapter".to_string(),
        "materialize-prompt".to_string(),
        "--dispatch-plan".to_string(),
        plans[0].dispatch_plan_id.clone(),
        "--record".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("materialize inline prompt");
    assert!(materialize.contains("adapter_dispatch_prompt_materialization=true"));
    assert!(materialize.contains("status=blocked_non_replayable_prompt"));
    assert!(materialize.contains("raw_prompt_policy=not_rendered"));
    assert!(!materialize.contains("Summarize this workpad"));
    let dashboard = run_cli(vec![
        "dashboard".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("dashboard");
    assert!(dashboard.contains("adapter_dispatch_plans=1"));
    assert!(dashboard.contains("adapter_dispatch_prompt_sources=1"));
    assert!(dashboard.contains("adapter_dispatch_prompt_materializations=1"));
    assert!(dashboard.contains("status=blocked_non_replayable_prompt"));
    assert!(dashboard.contains("source_kind=inline_cli_prompt"));
    assert!(dashboard.contains("adapter_dispatch_plan=adapter-dispatch-plan-codex_exec"));
    assert!(!dashboard.contains("Summarize this workpad"));
}

#[test]
fn adapter_plan_launch_records_distinct_prompt_identities_without_prompt_text() {
    let state_root = temp_root("adapter-plan-launch-distinct-state");
    let workspace = temp_root("adapter-plan-launch-distinct-workspace");
    let artifacts = temp_root("adapter-plan-launch-distinct-artifacts");
    for goal in ["First sensitive-ish prompt", "Second sensitive-ish prompt"] {
        run_cli(vec![
            "adapter".to_string(),
            "plan-launch".to_string(),
            "--adapter".to_string(),
            "codex".to_string(),
            "--agent".to_string(),
            "codex-worker".to_string(),
            "--goal".to_string(),
            goal.to_string(),
            "--workspace".to_string(),
            workspace.display().to_string(),
            "--artifacts".to_string(),
            artifacts.display().to_string(),
            "--record".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("record dispatch plan");
    }

    let plans = SqliteStateStore::open(&state_root)
        .expect("state")
        .adapter_dispatch_plans(&project_id())
        .expect("dispatch plans");
    assert_eq!(plans.len(), 2);
    assert_ne!(plans[0].dispatch_plan_id, plans[1].dispatch_plan_id);
    let prompt_sources = SqliteStateStore::open(&state_root)
        .expect("state")
        .adapter_dispatch_prompt_sources(&project_id())
        .expect("dispatch prompt sources");
    assert_eq!(prompt_sources.len(), 2);
    assert_ne!(prompt_sources[0].prompt_hash, prompt_sources[1].prompt_hash);
    let dashboard = run_cli(vec![
        "dashboard".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("dashboard");
    assert!(dashboard.contains("adapter_dispatch_plans=2"));
    assert!(!dashboard.contains("First sensitive-ish prompt"));
    assert!(!dashboard.contains("Second sensitive-ish prompt"));
}

#[test]
fn adapter_plan_launch_rejects_unknown_adapter() {
    let state_root = temp_root("adapter-plan-launch-unknown-state");
    let error = run_cli(vec![
        "adapter".to_string(),
        "plan-launch".to_string(),
        "--adapter".to_string(),
        "unknown".to_string(),
        "--agent".to_string(),
        "worker".to_string(),
        "--goal".to_string(),
        "Do work.".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .unwrap_err();

    assert!(error.contains("unsupported local adapter dispatch plan"));
    let state = SqliteStateStore::open(&state_root).expect("state");
    assert!(
        state
            .agent_by_name("worker")
            .expect("agent lookup after failed plan")
            .is_none()
    );
}

#[test]
fn adapter_dispatch_gate_blocks_until_real_smoke_evidence_is_recorded() {
    let state_root = temp_root("adapter-dispatch-gate-state");
    let workspace = temp_root("adapter-dispatch-gate-workspace");
    let artifacts = temp_root("adapter-dispatch-gate-artifacts");
    run_cli(vec![
        "adapter".to_string(),
        "plan-launch".to_string(),
        "--adapter".to_string(),
        "codex".to_string(),
        "--agent".to_string(),
        "codex-worker".to_string(),
        "--goal".to_string(),
        "Do not render this dispatch prompt.".to_string(),
        "--workspace".to_string(),
        workspace.display().to_string(),
        "--artifacts".to_string(),
        artifacts.display().to_string(),
        "--record".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("record dispatch plan");
    let plans = SqliteStateStore::open(&state_root)
        .expect("state")
        .adapter_dispatch_plans(&project_id())
        .expect("dispatch plans");
    let dispatch_plan_id = plans[0].dispatch_plan_id.clone();

    let blocked = run_cli(vec![
        "adapter".to_string(),
        "dispatch-gate".to_string(),
        "--dispatch-plan".to_string(),
        dispatch_plan_id.clone(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("blocked dispatch gate");
    assert!(blocked.contains("adapter_dispatch_gate=true"));
    assert!(blocked.contains("provider_cli_execution_allowed=false"));
    assert!(blocked.contains("status=blocked"));
    assert!(blocked.contains("required_dogfood_gate=blocked_pending_real_smoke"));
    assert!(blocked.contains("provider_cli_executed=false"));
    assert!(blocked.contains("runtime_prompt_policy=not_rendered"));
    assert!(blocked.contains("codex_exec:real_subscription_smoke_not_recorded"));
    assert!(blocked.contains("recorded=false"));
    assert!(!blocked.contains("Do not render this dispatch prompt"));
    assert!(!workspace.exists());
    assert!(!artifacts.exists());
    let blocked_status = run_cli(vec![
        "adapter".to_string(),
        "dispatch-status".to_string(),
        "--dispatch-plan".to_string(),
        dispatch_plan_id.clone(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("blocked dispatch status");
    assert!(blocked_status.contains("adapter_dispatch_status=true"));
    assert!(blocked_status.contains("latest_dispatch_gate=none"));
    assert!(blocked_status.contains("latest_gate_status=missing"));
    assert!(blocked_status.contains("latest_dispatch_replay=none"));
    assert!(blocked_status.contains("latest_dispatch_execution=none"));
    assert!(blocked_status.contains("latest_execution_status=missing"));
    assert!(blocked_status.contains("next_action=record_clean_real_smoke_evidence"));
    assert!(!blocked_status.contains("Do not render this dispatch prompt"));
    let blocked_latest_status = run_cli(vec![
        "adapter".to_string(),
        "dispatch-status".to_string(),
        "--latest".to_string(),
        "--agent".to_string(),
        "codex-worker".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("latest blocked dispatch status");
    assert!(blocked_latest_status.contains(&format!("dispatch_plan={dispatch_plan_id}")));
    assert!(blocked_latest_status.contains("agent=codex-worker"));
    assert!(blocked_latest_status.contains("next_action=record_clean_real_smoke_evidence"));
    assert!(!blocked_latest_status.contains("Do not render this dispatch prompt"));
    let blocked_execution_request = run_cli(vec![
        "adapter".to_string(),
        "execution-request".to_string(),
        "--dispatch-plan".to_string(),
        dispatch_plan_id.clone(),
        "--record".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("blocked execution request");
    assert!(blocked_execution_request.contains("adapter_dispatch_execution_request=true"));
    assert!(blocked_execution_request.contains("provider_cli_execution_allowed=false"));
    assert!(blocked_execution_request.contains("provider_cli_executed=false"));
    assert!(blocked_execution_request.contains("status=blocked_missing_ready_gate"));
    assert!(blocked_execution_request.contains("recorded=true"));
    assert!(!blocked_execution_request.contains("Do not render this dispatch prompt"));
    let fixture = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../capo-adapters/fixtures/codex-exec.jsonl"
    ));
    let blocked_replay = run_cli(vec![
        "adapter".to_string(),
        "replay-dispatch".to_string(),
        "--dispatch-plan".to_string(),
        dispatch_plan_id.clone(),
        "--fixture".to_string(),
        fixture.display().to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect_err("replay should require a recorded ready gate");
    assert!(blocked_replay.contains("has no recorded ready dispatch gate"));

    let artifact_root = temp_root("adapter-dispatch-gate-smoke-artifacts");
    fs::create_dir_all(&artifact_root).expect("artifact dir");
    fs::write(artifact_root.join("stdout.txt"), "CAPO_CODEX_SMOKE_OK\n").expect("artifact");
    run_cli(vec![
        "adapter".to_string(),
        "smoke-report".to_string(),
        "record".to_string(),
        "--adapter".to_string(),
        "codex".to_string(),
        "--status".to_string(),
        "passed".to_string(),
        "--credential-scan".to_string(),
        "clean".to_string(),
        "--marker-found".to_string(),
        "--artifact-root".to_string(),
        artifact_root.display().to_string(),
        "--reason".to_string(),
        "operator recorded clean opt-in smoke".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("record passed smoke");

    let ready = run_cli(vec![
        "adapter".to_string(),
        "dispatch-gate".to_string(),
        "--dispatch-plan".to_string(),
        dispatch_plan_id,
        "--record".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("ready dispatch gate");
    assert!(ready.contains("provider_cli_execution_allowed=true"));
    assert!(ready.contains("status=ready_for_execution"));
    assert!(ready.contains("required_dogfood_gate=ready_for_first_real_agent_dogfood"));
    assert!(ready.contains("reasons=required_real_smoke_evidence_recorded"));
    assert!(ready.contains("recorded=true"));
    let gates = SqliteStateStore::open(&state_root)
        .expect("state")
        .adapter_dispatch_gates(&project_id())
        .expect("dispatch gates");
    assert_eq!(gates.len(), 1);
    assert_eq!(gates[0].adapter_kind, "codex_exec");
    assert_eq!(gates[0].status, "ready_for_execution");
    assert!(gates[0].provider_cli_execution_allowed);
    assert!(!gates[0].provider_cli_executed);
    assert_eq!(gates[0].runtime_prompt_policy, "not_rendered");
    let dashboard = run_cli(vec![
        "dashboard".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("dashboard");
    assert!(dashboard.contains("adapter_dispatch_gates=1"));
    assert!(dashboard.contains("gate_status=ready_for_execution"));
    assert!(!dashboard.contains("Do not render this dispatch prompt"));
    let ready_status = run_cli(vec![
        "adapter".to_string(),
        "dispatch-status".to_string(),
        "--dispatch-plan".to_string(),
        gates[0].dispatch_plan_id.clone(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("ready dispatch status");
    assert!(ready_status.contains("latest_gate_status=ready_for_execution"));
    assert!(ready_status.contains("latest_gate_provider_cli_execution_allowed=true"));
    assert!(ready_status.contains("latest_dispatch_replay=none"));
    assert!(ready_status.contains(
        "next_action=replay_dispatch_fixture_or_run_provider_execution_after_explicit_opt_in"
    ));
    assert!(!ready_status.contains("Do not render this dispatch prompt"));
    let ready_execution_request = run_cli(vec![
        "adapter".to_string(),
        "execution-request".to_string(),
        "--dispatch-plan".to_string(),
        gates[0].dispatch_plan_id.clone(),
        "--record".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("ready execution request");
    assert!(ready_execution_request.contains("provider_cli_execution_allowed=true"));
    assert!(ready_execution_request.contains("provider_cli_executed=false"));
    assert!(ready_execution_request.contains("status=waiting_on_explicit_provider_opt_in"));
    assert!(ready_execution_request.contains("opt_in_env=CAPO_RUN_CODEX_LOCAL_DISPATCH"));
    assert!(
        ready_execution_request.contains("reasons=explicit_provider_execution_opt_in_required")
    );
    assert!(!ready_execution_request.contains("Do not render this dispatch prompt"));
    let preflight_without_materialization = run_cli(vec![
        "adapter".to_string(),
        "run-preflight".to_string(),
        "--dispatch-plan".to_string(),
        gates[0].dispatch_plan_id.clone(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("dispatch preflight without materialization");
    assert!(preflight_without_materialization.contains("adapter_dispatch_run_preflight=true"));
    assert!(
        preflight_without_materialization.contains("status=blocked_missing_prompt_materialization")
    );
    assert!(
        preflight_without_materialization
            .contains("reasons=recorded_prompt_materialization_missing")
    );
    assert!(preflight_without_materialization.contains("provider_cli_execution_allowed=false"));
    assert!(preflight_without_materialization.contains("provider_cli_executed=false"));
    assert!(!preflight_without_materialization.contains("Do not render this dispatch prompt"));
    let run_local_without_materialization = run_cli(vec![
        "adapter".to_string(),
        "run-local".to_string(),
        "--dispatch-plan".to_string(),
        gates[0].dispatch_plan_id.clone(),
        "--record".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("local dispatch runner blocks without materialization");
    assert!(run_local_without_materialization.contains("adapter_dispatch_run_local=true"));
    assert!(
        run_local_without_materialization.contains("status=blocked_missing_prompt_materialization")
    );
    assert!(run_local_without_materialization.contains("provider_cli_executed=false"));
    assert!(run_local_without_materialization.contains("recorded=true"));
    assert!(!run_local_without_materialization.contains("Do not render this dispatch prompt"));
    let executions = SqliteStateStore::open(&state_root)
        .expect("state")
        .adapter_dispatch_executions(&project_id())
        .expect("dispatch executions");
    assert_eq!(executions.len(), 1);
    assert_eq!(
        executions[0].status,
        "blocked_missing_prompt_materialization"
    );
    assert!(!executions[0].provider_cli_executed);
    assert_eq!(executions[0].credential_scan_status, "not_run");
    let execution_dashboard = run_cli(vec![
        "dashboard".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("dashboard after blocked local run");
    assert!(execution_dashboard.contains("adapter_dispatch_executions=1"));
    assert!(
        execution_dashboard.contains("execution_status=blocked_missing_prompt_materialization")
    );
    let blocked_execution_status = run_cli(vec![
        "adapter".to_string(),
        "dispatch-status".to_string(),
        "--dispatch-plan".to_string(),
        gates[0].dispatch_plan_id.clone(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("dispatch status after blocked local run");
    assert!(blocked_execution_status.contains("latest_dispatch_execution="));
    assert!(
        blocked_execution_status
            .contains("latest_execution_status=blocked_missing_prompt_materialization")
    );
    assert!(blocked_execution_status.contains("latest_execution_provider_cli_executed=false"));
    assert!(blocked_execution_status.contains("latest_execution_credential_scan_status=not_run"));
    assert!(blocked_execution_status.contains("next_action=resolve_latest_execution_blocker"));
    assert!(!blocked_execution_status.contains("Do not render this dispatch prompt"));
    let latest_after_blocked_execution = run_cli(vec![
        "adapter".to_string(),
        "dispatch-status".to_string(),
        "--latest".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("latest status after blocked local run");
    assert!(
        latest_after_blocked_execution
            .contains(&format!("dispatch_plan={}", gates[0].dispatch_plan_id))
    );
    assert!(
        latest_after_blocked_execution
            .contains("latest_execution_status=blocked_missing_prompt_materialization")
    );
    assert!(
        latest_after_blocked_execution.contains("next_action=resolve_latest_execution_blocker")
    );
    assert!(!latest_after_blocked_execution.contains("Do not render this dispatch prompt"));
    let inline_materialization = run_cli(vec![
        "adapter".to_string(),
        "materialize-prompt".to_string(),
        "--dispatch-plan".to_string(),
        gates[0].dispatch_plan_id.clone(),
        "--record".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("materialize inline prompt");
    assert!(inline_materialization.contains("status=blocked_non_replayable_prompt"));
    assert!(!inline_materialization.contains("Do not render this dispatch prompt"));
    let preflight_with_blocked_materialization = run_cli(vec![
        "adapter".to_string(),
        "run-preflight".to_string(),
        "--dispatch-plan".to_string(),
        gates[0].dispatch_plan_id.clone(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("dispatch preflight with blocked materialization");
    assert!(
        preflight_with_blocked_materialization
            .contains("status=blocked_prompt_materialization_not_ready")
    );
    assert!(preflight_with_blocked_materialization.contains("blocked_non_replayable_prompt"));
    assert!(preflight_with_blocked_materialization.contains("raw_prompt_policy=not_rendered"));
    assert!(!preflight_with_blocked_materialization.contains("Do not render this dispatch prompt"));
    let execution_requests = SqliteStateStore::open(&state_root)
        .expect("state")
        .adapter_dispatch_execution_requests(&project_id())
        .expect("dispatch execution requests");
    assert_eq!(execution_requests.len(), 2);
    assert!(
        execution_requests
            .iter()
            .any(|request| request.status == "blocked_missing_ready_gate")
    );
    assert!(
        execution_requests
            .iter()
            .any(|request| request.status == "waiting_on_explicit_provider_opt_in")
    );
    let evidence_dir = temp_root("adapter-dispatch-gate-replay-evidence");
    let replay = run_cli(vec![
        "adapter".to_string(),
        "replay-dispatch".to_string(),
        "--dispatch-plan".to_string(),
        gates[0].dispatch_plan_id.clone(),
        "--fixture".to_string(),
        fixture.display().to_string(),
        "--out".to_string(),
        evidence_dir.display().to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("replay dispatch fixture");
    assert!(replay.contains("adapter_dispatch_replayed=true"));
    assert!(replay.contains("adapter=codex_exec"));
    assert!(replay.contains("raw_content_policy=content_hashed_not_rendered"));
    assert!(replay.contains("provider_cli_executed=false"));
    assert!(replay.contains("tool_events=2"));
    assert!(replay.contains("summary_events=1"));
    assert!(replay.contains("completed_turns=1"));
    assert!(replay.contains("evidence_exported=true"));
    assert!(!replay.contains("Do not render this dispatch prompt"));
    assert!(!replay.contains("Codex fixture response."));
    assert!(!replay.contains("cargo test"));
    let replays = SqliteStateStore::open(&state_root)
        .expect("state")
        .adapter_dispatch_replays(&project_id())
        .expect("dispatch replays");
    assert_eq!(replays.len(), 1);
    assert_eq!(replays[0].dispatch_plan_id, gates[0].dispatch_plan_id);
    assert_eq!(replays[0].dispatch_gate_id, gates[0].dispatch_gate_id);
    assert_eq!(replays[0].adapter_kind, "codex_exec");
    assert_eq!(replays[0].tool_event_count, 2);
    assert!(!replays[0].provider_cli_executed);
    assert_eq!(replays[0].raw_content_policy, "content_hashed_not_rendered");
    let replay_tool_observations = SqliteStateStore::open(&state_root)
        .expect("state")
        .tool_observations_for_session(&replays[0].session_id)
        .expect("dispatch replay tool observations");
    assert!(replay_tool_observations.iter().any(|observation| {
        observation.tool_name == "exec_command"
            && observation.source == "adapter_event:codex_exec"
            && observation.instrumentation_level == "observed_only"
            && observation.confidence == "high"
    }));
    let replay_dashboard = run_cli(vec![
        "dashboard".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("dashboard after replay");
    assert!(replay_dashboard.contains("adapter_dispatch_replays=1"));
    assert!(replay_dashboard.contains("adapter_dispatch_execution_requests=2"));
    assert!(replay_dashboard.contains("execution_status=waiting_on_explicit_provider_opt_in"));
    assert!(replay_dashboard.contains("raw_content_policy=content_hashed_not_rendered"));
    assert!(!replay_dashboard.contains("Codex fixture response."));
    assert!(!replay_dashboard.contains("cargo test"));
    let replay_status = run_cli(vec![
        "adapter".to_string(),
        "dispatch-status".to_string(),
        "--dispatch-plan".to_string(),
        replays[0].dispatch_plan_id.clone(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("replay dispatch status");
    assert!(replay_status.contains("latest_gate_status=ready_for_execution"));
    assert!(replay_status.contains("latest_replay_raw_content_policy=content_hashed_not_rendered"));
    assert!(
        replay_status.contains("latest_execution_status=blocked_missing_prompt_materialization")
    );
    assert!(replay_status.contains("latest_replay_appended_events="));
    assert!(replay_status.contains("next_action=inspect_replay_or_prepare_real_execution"));
    assert!(!replay_status.contains("Do not render this dispatch prompt"));
    assert!(!replay_status.contains("Codex fixture response."));
    assert!(!replay_status.contains("cargo test"));
    let dispatch_evidence_dir = temp_root("adapter-dispatch-chain-evidence");
    let dispatch_evidence = run_cli(vec![
        "adapter".to_string(),
        "dispatch-evidence".to_string(),
        "--dispatch-plan".to_string(),
        replays[0].dispatch_plan_id.clone(),
        "--out".to_string(),
        dispatch_evidence_dir.display().to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("export dispatch evidence");
    assert!(dispatch_evidence.contains("adapter_dispatch_evidence_exported=true"));
    assert!(dispatch_evidence.contains("evidence_id="));
    assert!(dispatch_evidence.contains("artifact_id=artifact-adapter-dispatch-evidence-"));
    assert!(dispatch_evidence.contains(&format!("dispatch_plan={}", replays[0].dispatch_plan_id)));
    let dispatch_evidence_path = dispatch_evidence
        .lines()
        .find_map(|line| line.strip_prefix("path="))
        .map(PathBuf::from)
        .expect("dispatch evidence path");
    let dispatch_evidence_markdown =
        fs::read_to_string(&dispatch_evidence_path).expect("read dispatch evidence");
    assert!(dispatch_evidence_markdown.starts_with("<!-- capo:adapter-dispatch-evidence -->"));
    assert!(dispatch_evidence_markdown.contains("## Dispatch Plan"));
    assert!(dispatch_evidence_markdown.contains("## Latest Dispatch Gate"));
    assert!(dispatch_evidence_markdown.contains("## Latest Fixture Replay"));
    assert!(dispatch_evidence_markdown.contains("## Latest Local Execution"));
    assert!(dispatch_evidence_markdown.contains("## Observed Tool Activity"));
    assert!(dispatch_evidence_markdown.contains("name=`exec_command`"));
    assert!(dispatch_evidence_markdown.contains("source=`adapter_event:codex_exec`"));
    assert!(dispatch_evidence_markdown.contains("instrumentation=`observed_only`"));
    assert!(dispatch_evidence_markdown.contains("confidence=`high`"));
    assert!(dispatch_evidence_markdown.contains("Raw dispatch prompts are not rendered"));
    assert!(
        dispatch_evidence_markdown.contains("Status: `blocked_missing_prompt_materialization`")
    );
    assert!(!dispatch_evidence_markdown.contains("Do not render this dispatch prompt"));
    assert!(!dispatch_evidence_markdown.contains("Codex fixture response."));
    assert!(!dispatch_evidence_markdown.contains("cargo test"));
    let latest_dispatch_evidence_dir = temp_root("adapter-dispatch-chain-latest-evidence");
    let latest_dispatch_evidence = run_cli(vec![
        "adapter".to_string(),
        "dispatch-evidence".to_string(),
        "--latest".to_string(),
        "--agent".to_string(),
        "codex-worker".to_string(),
        "--out".to_string(),
        latest_dispatch_evidence_dir.display().to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("export latest dispatch evidence");
    assert!(latest_dispatch_evidence.contains("adapter_dispatch_evidence_exported=true"));
    assert!(
        latest_dispatch_evidence
            .contains(&format!("dispatch_plan={}", replays[0].dispatch_plan_id))
    );
    let latest_dispatch_evidence_path = latest_dispatch_evidence
        .lines()
        .find_map(|line| line.strip_prefix("path="))
        .map(PathBuf::from)
        .expect("latest dispatch evidence path");
    let latest_dispatch_evidence_markdown =
        fs::read_to_string(&latest_dispatch_evidence_path).expect("read latest evidence");
    assert!(
        latest_dispatch_evidence_markdown.starts_with("<!-- capo:adapter-dispatch-evidence -->")
    );
    assert!(latest_dispatch_evidence_markdown.contains("## Latest Fixture Replay"));
    assert!(latest_dispatch_evidence_markdown.contains("## Observed Tool Activity"));
    assert!(latest_dispatch_evidence_markdown.contains("instrumentation=`observed_only`"));
    assert!(latest_dispatch_evidence_markdown.contains("Raw dispatch prompts are not rendered"));
    assert!(!latest_dispatch_evidence.contains("Do not render this dispatch prompt"));
    assert!(!latest_dispatch_evidence_markdown.contains("Do not render this dispatch prompt"));
    assert!(!latest_dispatch_evidence_markdown.contains("Codex fixture response."));
    assert!(!latest_dispatch_evidence_markdown.contains("cargo test"));
    let dispatch_evidence_rows = SqliteStateStore::open(&state_root)
        .expect("state")
        .evidence_for_session(&replays[0].session_id)
        .expect("dispatch evidence rows");
    assert!(
        dispatch_evidence_rows
            .iter()
            .any(|evidence| evidence.kind == "adapter_dispatch_evidence")
    );
    let runtime_workspace = temp_root("dogfood-readiness-runtime-workspace");
    let runtime_artifacts = temp_root("dogfood-readiness-runtime-artifacts");
    let runtime_target = run_cli(vec![
        "runtime".to_string(),
        "target".to_string(),
        "register".to_string(),
        "--target".to_string(),
        "runtime-target-local-dogfood".to_string(),
        "--name".to_string(),
        "local dogfood runtime".to_string(),
        "--runner".to_string(),
        "local-process".to_string(),
        "--workspace".to_string(),
        runtime_workspace.display().to_string(),
        "--artifacts".to_string(),
        runtime_artifacts.display().to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("register dogfood runtime target");
    assert!(runtime_target.contains("runtime_target_registered=true"));
    let readiness = run_cli(vec![
        "dogfood".to_string(),
        "readiness".to_string(),
        "--out".to_string(),
        dispatch_evidence_dir.display().to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("dogfood readiness");
    assert!(readiness.contains("dogfood_readiness=true"));
    assert!(readiness.contains("ready=false"));
    assert!(readiness.contains("real_agent_connector_ready=true"));
    assert!(readiness.contains("runtime_target_ready=true"));
    assert!(readiness.contains("dispatch_chain_ready=true"));
    assert!(readiness.contains("workpad_bridge_ready=false"));
    assert!(readiness.contains("runtime_targets=1"));
    assert!(readiness.contains("runtime_targets_available=1"));
    assert!(readiness.contains("dispatch_plans=1"));
    assert!(readiness.contains("dispatch_replays=1"));
    assert!(readiness.contains("dispatch_executions=1"));
    assert!(readiness.contains("connector_evidence_refs=adapter-smoke-codex"));
    assert!(readiness.contains("runtime_target_refs=runtime-target-local-dogfood"));
    assert!(readiness.contains("dispatch_chain_refs=adapter-dispatch-plan-"));
    assert!(readiness.contains("adapter-dispatch-replay-"));
    assert!(readiness.contains("adapter-dispatch-execution-"));
    assert!(readiness.contains("project_evidence_refs=none"));
    assert!(readiness.contains("blockers=workpad_index_missing"));
    assert!(readiness.contains("next_actions=run_workpad_index"));
    assert!(readiness.contains("dogfood_readiness_evidence_exported=true"));
    assert!(readiness.contains("artifact_id=artifact-dogfood-readiness-"));
    let readiness_path = readiness
        .lines()
        .find_map(|line| line.strip_prefix("path="))
        .map(PathBuf::from)
        .expect("dogfood readiness evidence path");
    let readiness_markdown =
        fs::read_to_string(&readiness_path).expect("read dogfood readiness evidence");
    assert!(readiness_markdown.starts_with("<!-- capo:dogfood-readiness -->"));
    assert!(readiness_markdown.contains("## Summary"));
    assert!(readiness_markdown.contains("## Counts"));
    assert!(readiness_markdown.contains("## Component Refs"));
    assert!(readiness_markdown.contains("Connector evidence refs: `adapter-smoke-codex"));
    assert!(readiness_markdown.contains("Runtime target refs: `runtime-target-local-dogfood`"));
    assert!(readiness_markdown.contains("Dispatch chain refs: `adapter-dispatch-plan-"));
    assert!(readiness_markdown.contains("adapter-dispatch-replay-"));
    assert!(readiness_markdown.contains("adapter-dispatch-execution-"));
    assert!(readiness_markdown.contains("`workpad_index_missing`"));
    assert!(readiness_markdown.contains("does not run provider CLIs"));
    assert!(!readiness_markdown.contains("Do not render this dispatch prompt"));
    assert!(!readiness_markdown.contains("Codex fixture response."));
    assert!(!readiness_markdown.contains("cargo test"));
    let dashboard_after_readiness = run_cli(vec![
        "dashboard".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("dashboard after dogfood readiness evidence");
    assert!(dashboard_after_readiness.contains("project_evidence=1"));
    assert!(dashboard_after_readiness.contains("kind=dogfood_readiness"));
    assert!(dashboard_after_readiness.contains("artifact=artifact-dogfood-readiness-"));
    assert!(dashboard_after_readiness.contains("project_dogfood_readiness=false"));
    assert!(dashboard_after_readiness.contains("status=blocked_pending_dogfood_prerequisites"));
    assert!(dashboard_after_readiness.contains("real_agent_connector_ready=true"));
    assert!(dashboard_after_readiness.contains("runtime_target_ready=true"));
    assert!(dashboard_after_readiness.contains("workpad_bridge_ready=false"));
    assert!(dashboard_after_readiness.contains("dispatch_chain_ready=true"));
    assert!(dashboard_after_readiness.contains("connector_evidence_refs=adapter-smoke-codex"));
    assert!(dashboard_after_readiness.contains("runtime_target_refs=runtime-target-local-dogfood"));
    assert!(dashboard_after_readiness.contains("workpad_task_refs=none"));
    assert!(dashboard_after_readiness.contains("dispatch_chain_refs=adapter-dispatch-plan-"));
    assert!(dashboard_after_readiness.contains("adapter-dispatch-replay-"));
    assert!(dashboard_after_readiness.contains("adapter-dispatch-execution-"));
    assert!(
        dashboard_after_readiness
            .contains("project_evidence_refs=evidence-artifact-dogfood-readiness-")
    );
    assert!(dashboard_after_readiness.contains("blockers=workpad_index_missing"));
    assert_text_absent_in_tree(&state_root, "Do not render this dispatch prompt");
    assert_text_absent_in_tree(&state_root, "Codex fixture response.");
    assert_text_absent_in_tree(&state_root, "cargo test");
    assert_text_absent_in_tree(&evidence_dir, "Codex fixture response.");
    assert_text_absent_in_tree(&evidence_dir, "cargo test");
    assert_text_absent_in_tree(&dispatch_evidence_dir, "Do not render this dispatch prompt");
    assert_text_absent_in_tree(&dispatch_evidence_dir, "Codex fixture response.");
    assert_text_absent_in_tree(&dispatch_evidence_dir, "cargo test");
    assert!(!workspace.exists());
    assert!(!artifacts.exists());
}

#[test]
fn adapter_readiness_reports_opt_in_gates_without_running_provider_clis() {
    let state_root = temp_root("adapter-readiness-state");
    let output = run_cli(vec![
        "adapter".to_string(),
        "readiness".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("adapter readiness");

    assert!(output.contains("adapter_readiness=true"));
    assert!(output.contains("credential_policy=not_inspected"));
    assert!(output.contains("adapter=codex_exec"));
    assert!(output.contains("opt_in_env=CAPO_RUN_CODEX_LOCAL_SMOKE"));
    assert!(output.contains("adapter=claude_code"));
    assert!(output.contains("opt_in_env=CAPO_RUN_CLAUDE_LOCAL_SMOKE"));
    assert!(output.contains("ready_for_real_agent_dogfood=false"));
    assert!(output.contains("blocked_reason=real_subscription_smoke_not_recorded"));
    assert!(output.contains("recorded=false"));
    assert!(!state_root.join("adapter-readiness").exists());

    let recorded = run_cli(vec![
        "adapter".to_string(),
        "readiness".to_string(),
        "--record".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("record adapter readiness");
    assert!(recorded.contains("recorded=true"));
    assert!(recorded.contains("recorded_sequence="));

    let state = SqliteStateStore::open(&state_root).expect("state");
    let readiness = state
        .adapter_readiness(&project_id())
        .expect("adapter readiness rows");
    assert_eq!(readiness.len(), 2);
    assert!(readiness.iter().any(|row| row.adapter_kind == "codex_exec"
        && row.smoke_status == "waiting_on_opt_in"
        && row.credential_policy == "not_inspected"));

    let dashboard = run_cli(vec![
        "dashboard".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("dashboard");
    assert!(dashboard.contains("adapter_readiness=2"));
    assert!(dashboard.contains("adapter_readiness_row=codex_exec"));
    assert!(dashboard.contains("dogfood_blocker=real_subscription_smoke_not_recorded"));
}

#[test]
fn adapter_smoke_report_records_skipped_and_blocks_invalid_pass() {
    let state_root = temp_root("adapter-smoke-report-state");
    let artifact_root = temp_root("adapter-smoke-report-artifacts");
    fs::create_dir_all(&artifact_root).expect("artifact dir");
    fs::write(
        artifact_root.join("stdout.txt"),
        "CAPO_CODEX_SMOKE_OK\nAuthorization: [REDACTED]\n",
    )
    .expect("clean artifact");
    let skipped = run_cli(vec![
        "adapter".to_string(),
        "smoke-report".to_string(),
        "record".to_string(),
        "--adapter".to_string(),
        "codex".to_string(),
        "--status".to_string(),
        "skipped".to_string(),
        "--credential-scan".to_string(),
        "not_run".to_string(),
        "--reason".to_string(),
        "waiting for explicit opt-in".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("record skipped smoke");
    assert!(skipped.contains("adapter_smoke_report_recorded=true"));
    assert!(skipped.contains("dogfood_readiness_effect=real_subscription_smoke_not_recorded"));

    let invalid_pass = run_cli(vec![
        "adapter".to_string(),
        "smoke-report".to_string(),
        "record".to_string(),
        "--adapter".to_string(),
        "codex".to_string(),
        "--status".to_string(),
        "passed".to_string(),
        "--credential-scan".to_string(),
        "not_run".to_string(),
        "--reason".to_string(),
        "bad pass".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect_err("passed report requires clean scan and marker");
    assert!(invalid_pass.contains("passed smoke reports require"));

    let passed = run_cli(vec![
        "adapter".to_string(),
        "smoke-report".to_string(),
        "record".to_string(),
        "--adapter".to_string(),
        "codex".to_string(),
        "--status".to_string(),
        "passed".to_string(),
        "--credential-scan".to_string(),
        "clean".to_string(),
        "--marker-found".to_string(),
        "--artifact-root".to_string(),
        artifact_root.display().to_string(),
        "--reason".to_string(),
        "clean opt-in smoke artifacts".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("record passed smoke");
    assert!(passed.contains("dogfood_readiness_effect=real_agent_connector_proven"));

    let exact_status = run_cli(vec![
        "adapter".to_string(),
        "smoke-report".to_string(),
        "status".to_string(),
        "--smoke-report".to_string(),
        output_value(&passed, "smoke_report_id"),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("adapter smoke report status");
    assert!(exact_status.contains("adapter_smoke_report_status=true"));
    assert!(exact_status.contains("adapter=codex_exec"));
    assert!(exact_status.contains("smoke_status=passed"));
    assert!(exact_status.contains("credential_scan_status=clean"));
    assert!(exact_status.contains("provider_cli_executed=false"));
    assert!(exact_status.contains("credential_material_rendered=false"));
    assert!(exact_status.contains("state_mutated=false"));

    let latest_status = run_cli(vec![
        "adapter".to_string(),
        "smoke-report".to_string(),
        "status".to_string(),
        "--latest".to_string(),
        "--adapter".to_string(),
        "codex".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("latest adapter smoke report status");
    assert!(latest_status.contains("adapter_smoke_report_status=true"));
    assert_eq!(
        output_value(&latest_status, "smoke_report_id"),
        output_value(&passed, "smoke_report_id")
    );

    let evidence_dir = temp_root("adapter-smoke-report-evidence");
    let evidence = run_cli(vec![
        "adapter".to_string(),
        "smoke-report".to_string(),
        "evidence".to_string(),
        "--smoke-report".to_string(),
        output_value(&passed, "smoke_report_id"),
        "--out".to_string(),
        evidence_dir.display().to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("export adapter smoke evidence");
    assert!(evidence.contains("adapter_smoke_report_evidence_exported=true"));
    assert!(evidence.contains("evidence_id=evidence-artifact-adapter-smoke-evidence-"));
    let evidence_path = output_value(&evidence, "path");
    let markdown = fs::read_to_string(&evidence_path).expect("read adapter smoke evidence");
    assert!(markdown.starts_with("<!-- capo:adapter-smoke-evidence -->"));
    assert!(markdown.contains("## Smoke Report"));
    assert!(markdown.contains("- Adapter: `codex_exec`"));
    assert!(markdown.contains("- Smoke status: `passed`"));
    assert!(markdown.contains("- Credential scan status: `clean`"));
    assert!(markdown.contains("does not render stdout"));
    assert!(!markdown.contains("CAPO_CODEX_SMOKE_OK"));
    assert!(!markdown.contains("Authorization:"));

    let latest_evidence = run_cli(vec![
        "adapter".to_string(),
        "smoke-report".to_string(),
        "evidence".to_string(),
        "--latest".to_string(),
        "--adapter".to_string(),
        "codex".to_string(),
        "--out".to_string(),
        evidence_dir.display().to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("export latest adapter smoke evidence");
    assert!(latest_evidence.contains("adapter_smoke_report_evidence_exported=true"));
    assert_eq!(
        output_value(&latest_evidence, "smoke_report_id"),
        output_value(&passed, "smoke_report_id")
    );
    let missing_latest_evidence = run_cli(vec![
        "adapter".to_string(),
        "smoke-report".to_string(),
        "evidence".to_string(),
        "--latest".to_string(),
        "--adapter".to_string(),
        "claude".to_string(),
        "--out".to_string(),
        evidence_dir.display().to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect_err("missing latest adapter smoke evidence");
    assert!(
        missing_latest_evidence
            .contains("no recorded adapter smoke reports matching adapter=claude_code")
    );
    let missing_evidence = run_cli(vec![
        "adapter".to_string(),
        "smoke-report".to_string(),
        "evidence".to_string(),
        "--smoke-report".to_string(),
        "missing-smoke-report".to_string(),
        "--out".to_string(),
        evidence_dir.display().to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect_err("missing adapter smoke evidence");
    assert!(missing_evidence.contains("missing adapter smoke report: missing-smoke-report"));
    let missing_status = run_cli(vec![
        "adapter".to_string(),
        "smoke-report".to_string(),
        "status".to_string(),
        "--smoke-report".to_string(),
        "missing-smoke-report".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect_err("missing adapter smoke report status");
    assert!(missing_status.contains("missing adapter smoke report: missing-smoke-report"));

    let dashboard = run_cli(vec![
        "dashboard".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("dashboard");
    assert!(dashboard.contains("adapter_smoke_reports=2"));
    assert!(dashboard.contains("latest_adapter_smoke_report_any=adapter-smoke-codex_exec"));
    assert!(dashboard.contains("latest_adapter_smoke_report_codex=adapter-smoke-codex_exec"));
    assert!(dashboard.contains("latest_adapter_smoke_report_claude=none"));
    assert!(dashboard.contains("adapter_smoke_report=adapter-smoke-codex_exec"));
    assert!(dashboard.contains("credential_scan_status=not_run"));
    assert!(dashboard.contains("project_evidence=1"));
    assert!(dashboard.contains("kind=adapter_smoke_evidence"));
}

#[test]
fn adapter_smoke_artifact_scan_blocks_raw_secret_markers() {
    let clean_root = temp_root("adapter-clean-artifacts");
    fs::create_dir_all(clean_root.join("nested")).expect("clean artifact dir");
    fs::write(
        clean_root.join("nested").join("stdout.txt"),
        "Cookie: [REDACTED]\n",
    )
    .expect("clean artifact");
    let clean = run_cli(vec![
        "adapter".to_string(),
        "smoke-report".to_string(),
        "scan".to_string(),
        "--artifact-root".to_string(),
        clean_root.display().to_string(),
    ])
    .expect("clean scan");
    assert!(clean.contains("adapter_smoke_artifact_scan=true"));
    assert!(clean.contains("credential_scan_status=clean"));
    assert!(clean.contains("files_scanned=1"));

    let blocked_root = temp_root("adapter-blocked-artifacts");
    fs::create_dir_all(&blocked_root).expect("blocked artifact dir");
    fs::write(
        blocked_root.join("stderr.txt"),
        "Authorization: Bearer secret\n",
    )
    .expect("blocked artifact");
    let blocked = run_cli(vec![
        "adapter".to_string(),
        "smoke-report".to_string(),
        "scan".to_string(),
        "--artifact-root".to_string(),
        blocked_root.display().to_string(),
    ])
    .expect_err("raw secret marker should block scan");
    assert!(blocked.contains("credential scan blocked artifact"));
    assert!(blocked.contains("authorization:"));

    let state_root = temp_root("adapter-blocked-report-state");
    let blocked_report = run_cli(vec![
        "adapter".to_string(),
        "smoke-report".to_string(),
        "record".to_string(),
        "--adapter".to_string(),
        "codex".to_string(),
        "--status".to_string(),
        "passed".to_string(),
        "--credential-scan".to_string(),
        "clean".to_string(),
        "--marker-found".to_string(),
        "--artifact-root".to_string(),
        blocked_root.display().to_string(),
        "--reason".to_string(),
        "should fail scan".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect_err("passed report should enforce scan");
    assert!(blocked_report.contains("credential scan blocked artifact"));
}

#[test]
fn adapter_smoke_artifact_scan_refuses_symlinks() {
    let artifact_root = temp_root("adapter-symlink-artifacts");
    fs::create_dir_all(&artifact_root).expect("artifact dir");
    fs::write(artifact_root.join("stdout.txt"), "CAPO_CODEX_SMOKE_OK\n").expect("artifact");
    let outside = temp_root("adapter-symlink-outside");
    fs::create_dir_all(&outside).expect("outside dir");
    fs::write(
        outside.join("session.txt"),
        "Authorization: Bearer secret\n",
    )
    .expect("outside secret");
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        outside.join("session.txt"),
        artifact_root.join("session-link"),
    )
    .expect("symlink");
    #[cfg(windows)]
    std::os::windows::fs::symlink_file(
        outside.join("session.txt"),
        artifact_root.join("session-link"),
    )
    .expect("symlink");

    let blocked = run_cli(vec![
        "adapter".to_string(),
        "smoke-report".to_string(),
        "scan".to_string(),
        "--artifact-root".to_string(),
        artifact_root.display().to_string(),
    ])
    .expect_err("symlink should be refused");
    assert!(blocked.contains("artifact scan refuses symlink path"));
}

#[test]
fn adapter_dogfood_gate_requires_passed_codex_smoke_report() {
    let state_root = temp_root("adapter-dogfood-gate-state");
    let artifact_root = temp_root("adapter-dogfood-gate-artifacts");
    fs::create_dir_all(&artifact_root).expect("artifact dir");
    fs::write(artifact_root.join("stdout.txt"), "CAPO_CODEX_SMOKE_OK\n").expect("artifact");
    let blocked = run_cli(vec![
        "adapter".to_string(),
        "dogfood-gate".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("blocked gate");
    assert!(blocked.contains("adapter_dogfood_gate=true"));
    assert!(blocked.contains("ready_for_first_real_agent_dogfood=false"));
    assert!(blocked.contains("blocked_adapters=codex_exec"));
    let evidence_dir = temp_root("adapter-dogfood-gate-evidence");
    let blocked_evidence = run_cli(vec![
        "adapter".to_string(),
        "dogfood-gate".to_string(),
        "evidence".to_string(),
        "--out".to_string(),
        evidence_dir.display().to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("blocked gate evidence");
    assert!(blocked_evidence.contains("adapter_dogfood_gate_evidence_exported=true"));
    assert!(blocked_evidence.contains("ready_for_first_real_agent_dogfood=false"));
    assert!(blocked_evidence.contains("provider_cli_executed=false"));
    let blocked_evidence_path = output_value(&blocked_evidence, "path");
    let blocked_markdown =
        fs::read_to_string(&blocked_evidence_path).expect("read blocked gate evidence");
    assert!(blocked_markdown.starts_with("<!-- capo:adapter-dogfood-gate-evidence -->"));
    assert!(blocked_markdown.contains("- Blocked adapters: `codex_exec`"));
    assert!(blocked_markdown.contains("does not render stdout"));

    run_cli(vec![
        "adapter".to_string(),
        "smoke-report".to_string(),
        "record".to_string(),
        "--adapter".to_string(),
        "codex".to_string(),
        "--status".to_string(),
        "passed".to_string(),
        "--credential-scan".to_string(),
        "clean".to_string(),
        "--marker-found".to_string(),
        "--artifact-root".to_string(),
        artifact_root.display().to_string(),
        "--reason".to_string(),
        "operator recorded clean opt-in smoke".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("record passed smoke");

    let ready = run_cli(vec![
        "adapter".to_string(),
        "dogfood-gate".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("ready gate");
    assert!(ready.contains("ready_for_first_real_agent_dogfood=true"));
    assert!(ready.contains("status=ready_for_first_real_agent_dogfood"));
    assert!(ready.contains("proven_adapters=codex_exec"));
    let ready_evidence = run_cli(vec![
        "adapter".to_string(),
        "dogfood-gate".to_string(),
        "evidence".to_string(),
        "--out".to_string(),
        evidence_dir.display().to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("ready gate evidence");
    assert!(ready_evidence.contains("adapter_dogfood_gate_evidence_exported=true"));
    assert!(ready_evidence.contains("ready_for_first_real_agent_dogfood=true"));
    let ready_evidence_path = output_value(&ready_evidence, "path");
    let ready_markdown =
        fs::read_to_string(&ready_evidence_path).expect("read ready gate evidence");
    assert!(ready_markdown.contains("- Proven adapters: `codex_exec`"));
    assert!(ready_markdown.contains("adapter-smoke-codex_exec"));
    assert!(!ready_markdown.contains("CAPO_CODEX_SMOKE_OK"));

    let dashboard = run_cli(vec![
        "dashboard".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("dashboard");
    assert!(dashboard.contains("adapter_dogfood_gate=true"));
    assert!(dashboard.contains("ready_for_first_real_agent_dogfood=true"));
    assert!(dashboard.contains("project_evidence=2"));
    assert!(dashboard.contains("kind=adapter_dogfood_gate_evidence"));
}

#[test]
fn workpad_index_imports_markdown_refs_without_modifying_sources() {
    let state_root = temp_root("workpad-index-state");
    let project_root = temp_root("workpad-index-project");
    fs::create_dir_all(project_root.join("workpads/features")).expect("feature dir");
    fs::write(
            project_root.join("TASKS.md"),
            "# Project Task Queue\n\n## Objective\n\nRoute work.\n\n## F2 - Workpad Dogfood Bridge\n\nStatus: pending\n",
        )
        .expect("write tasks");
    fs::write(
        project_root.join("project.md"),
        "# Capo\n\n## Objective\n\nBuild Capo.\n",
    )
    .expect("write project");
    fs::write(
            project_root.join("workpads/features/tasks.md"),
            "# Feature Tasks\n\n## Objective\n\nSplit work.\n\n## F1 - Real Local Agent Connector Proof\n\nStatus: pending\n\n## F2 - Workpad Dogfood Bridge\n\nStatus: in_progress\n",
        )
        .expect("write feature tasks");
    let before =
        fs::read_to_string(project_root.join("workpads/features/tasks.md")).expect("read before");

    let output = run_cli(vec![
        "workpad".to_string(),
        "index".to_string(),
        "--root".to_string(),
        project_root.display().to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("index workpads");

    assert!(output.contains("workpads_indexed=true"));
    assert!(output.contains("files=3"));
    assert!(output.contains("tasks=3"));
    let state = SqliteStateStore::open(&state_root).expect("state");
    state
        .rebuild_projections()
        .expect("rebuild workpad projections");
    let files = state.workpad_files(&project_id()).expect("workpad files");
    let tasks = state.workpad_tasks(&project_id()).expect("workpad tasks");
    assert_eq!(files.len(), 3);
    assert!(files.iter().any(|file| file.path == "TASKS.md"));
    assert!(files.iter().any(|file| {
        file.path == "workpads/features/tasks.md"
            && file.objective.as_deref() == Some("Split work.")
    }));
    assert_eq!(
        tasks
            .iter()
            .find(|task| task.workpad_task_id == "workpads:features:tasks.md#f2")
            .map(|task| {
                (
                    task.observed_status.as_str(),
                    task.capo_execution_status.as_str(),
                )
            }),
        Some(("in_progress", "observed_only"))
    );
    assert_eq!(
        fs::read_to_string(project_root.join("workpads/features/tasks.md")).expect("read after"),
        before
    );
    let next_output = run_cli(vec![
        "workpad".to_string(),
        "next".to_string(),
        "--path".to_string(),
        "workpads/features/tasks.md".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("select next indexed workpad task");
    assert!(next_output.contains("workpad_next_found=true"));
    assert!(next_output.contains("workpad_task_id=workpads:features:tasks.md#f2"));
    assert!(next_output.contains("observed_status=in_progress"));
    assert!(next_output.contains("capo_execution_status=observed_only"));
    assert!(next_output.contains("default_task_id=task-workpad-workpads-features-tasks-md-f2"));
    let dashboard_after_index = run_cli(vec![
        "dashboard".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("dashboard after workpad index");
    assert!(dashboard_after_index.contains("workpad_tasks=3"));
    assert!(dashboard_after_index.contains("workpad_task=workpads:features:tasks.md#f2"));
    assert!(dashboard_after_index.contains("capo_execution_status=observed_only"));
    let dashboard_by_workpad = run_cli(vec![
        "dashboard".to_string(),
        "--workpad-path".to_string(),
        "workpads/features/tasks.md".to_string(),
        "--workpad-status".to_string(),
        "in_progress".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("dashboard filtered by workpad task");
    assert!(dashboard_by_workpad.contains("workpad_tasks=1"));
    assert!(dashboard_by_workpad.contains("workpad_task=workpads:features:tasks.md#f2"));
    assert!(!dashboard_by_workpad.contains("workpad_task=TASKS.md#f2"));
    let plan_next = run_cli(vec![
        "workpad".to_string(),
        "plan-next".to_string(),
        "--agent".to_string(),
        "codex-dogfood".to_string(),
        "--adapter".to_string(),
        "codex".to_string(),
        "--path".to_string(),
        "workpads/features/tasks.md".to_string(),
        "--record".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("plan next workpad task for adapter");
    assert!(plan_next.contains("workpad_next_planned=true"));
    assert!(plan_next.contains("adapter=codex_exec"));
    assert!(plan_next.contains("workpad_task_id=workpads:features:tasks.md#f2"));
    assert!(plan_next.contains("runtime_prompt_policy=not_rendered"));
    assert!(plan_next.contains("runtime_prompt_source_kind=workpad_task"));
    assert!(plan_next.contains("runtime_prompt_materialization=replayable_if_source_hash_matches"));
    assert!(plan_next.contains("provider_cli_executed=false"));
    assert!(plan_next.contains("recorded=true"));
    assert!(!plan_next.contains("Work on Workpad Dogfood Bridge"));
    let plans = state
        .adapter_dispatch_plans(&project_id())
        .expect("dispatch plans");
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].adapter_kind, "codex_exec");
    assert_eq!(plans[0].runtime_prompt_policy, "not_rendered");
    assert!(!plans[0].provider_cli_executed);
    let prompt_sources = state
        .adapter_dispatch_prompt_sources(&project_id())
        .expect("dispatch prompt sources");
    assert_eq!(prompt_sources.len(), 1);
    assert_eq!(prompt_sources[0].source_kind, "workpad_task");
    assert_eq!(
        prompt_sources[0].source_ref.as_deref(),
        Some("workpads/features/tasks.md#F2 - Workpad Dogfood Bridge")
    );
    assert_eq!(
        prompt_sources[0].materialization_status,
        "replayable_if_source_hash_matches"
    );
    let materialize = run_cli(vec![
        "adapter".to_string(),
        "materialize-prompt".to_string(),
        "--dispatch-plan".to_string(),
        plans[0].dispatch_plan_id.clone(),
        "--record".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("materialize workpad prompt");
    assert!(materialize.contains("adapter_dispatch_prompt_materialization=true"));
    assert!(materialize.contains("status=ready_without_rendering_prompt"));
    assert!(materialize.contains("reasons=prompt_hash_matches_source"));
    assert!(materialize.contains("raw_prompt_policy=not_rendered"));
    assert!(!materialize.contains("Work on Workpad Dogfood Bridge"));
    let artifact_root = temp_root("workpad-plan-dispatch-smoke-artifacts");
    fs::create_dir_all(&artifact_root).expect("artifact dir");
    fs::write(artifact_root.join("stdout.txt"), "CAPO_CODEX_SMOKE_OK\n").expect("artifact");
    run_cli(vec![
        "adapter".to_string(),
        "smoke-report".to_string(),
        "record".to_string(),
        "--adapter".to_string(),
        "codex".to_string(),
        "--status".to_string(),
        "passed".to_string(),
        "--credential-scan".to_string(),
        "clean".to_string(),
        "--marker-found".to_string(),
        "--artifact-root".to_string(),
        artifact_root.display().to_string(),
        "--reason".to_string(),
        "operator recorded clean opt-in smoke".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("record passed smoke");
    let ready_gate = run_cli(vec![
        "adapter".to_string(),
        "dispatch-gate".to_string(),
        "--dispatch-plan".to_string(),
        plans[0].dispatch_plan_id.clone(),
        "--record".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("record ready dispatch gate");
    assert!(ready_gate.contains("status=ready_for_execution"));
    let execution_request = run_cli(vec![
        "adapter".to_string(),
        "execution-request".to_string(),
        "--dispatch-plan".to_string(),
        plans[0].dispatch_plan_id.clone(),
        "--record".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("record execution request");
    assert!(execution_request.contains("status=waiting_on_explicit_provider_opt_in"));
    let run_preflight = run_cli(vec![
        "adapter".to_string(),
        "run-preflight".to_string(),
        "--dispatch-plan".to_string(),
        plans[0].dispatch_plan_id.clone(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("run dispatch preflight");
    assert!(run_preflight.contains("adapter_dispatch_run_preflight=true"));
    assert!(run_preflight.contains("status=blocked_missing_explicit_provider_opt_in"));
    assert!(run_preflight.contains("provider_cli_execution_allowed=false"));
    assert!(run_preflight.contains("provider_cli_executed=false"));
    assert!(run_preflight.contains("opt_in_env=CAPO_RUN_CODEX_LOCAL_DISPATCH"));
    assert!(run_preflight.contains("CAPO_RUN_CODEX_LOCAL_DISPATCH=1_required"));
    assert!(run_preflight.contains("raw_prompt_policy=not_rendered"));
    assert!(!run_preflight.contains("Work on Workpad Dogfood Bridge"));
    let planned_workpad_task = state
        .workpad_task(&project_id(), "workpads:features:tasks.md#f2")
        .expect("planned workpad task query")
        .expect("planned workpad task");
    assert_eq!(planned_workpad_task.capo_execution_status, "observed_only");
    let dashboard_after_plan = run_cli(vec![
        "dashboard".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("dashboard after plan-next");
    assert!(dashboard_after_plan.contains("adapter_dispatch_plans=1"));
    assert!(dashboard_after_plan.contains("adapter_dispatch_prompt_sources=1"));
    assert!(dashboard_after_plan.contains("adapter_dispatch_prompt_materializations=1"));
    assert!(dashboard_after_plan.contains("status=ready_without_rendering_prompt"));
    assert!(dashboard_after_plan.contains("source_kind=workpad_task"));
    assert!(!dashboard_after_plan.contains("Work on Workpad Dogfood Bridge"));
    let source_hash = files
        .iter()
        .find(|file| file.path == "workpads/features/tasks.md")
        .expect("feature tasks file")
        .content_hash
        .clone();

    let import_output = run_cli(vec![
        "workpad".to_string(),
        "import".to_string(),
        "--workpad-task".to_string(),
        "workpads:features:tasks.md#f2".to_string(),
        "--expected-hash".to_string(),
        source_hash.clone(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("import workpad task");
    assert!(import_output.contains("workpad_task_imported=true"));
    assert!(import_output.contains("task_id=task-workpad-workpads-features-tasks-md-f2"));
    assert!(import_output.contains(&format!("source_hash={source_hash}")));
    let imported_task = state
        .task(&TaskId::new("task-workpad-workpads-features-tasks-md-f2"))
        .expect("imported task query")
        .expect("imported task");
    assert_eq!(imported_task.capo_execution_status, "ready");
    assert!(
        imported_task
            .latest_summary
            .as_deref()
            .is_some_and(|summary| summary
                .contains("workpads/features/tasks.md#F2 - Workpad Dogfood Bridge")
                && summary.contains(&format!("hash={source_hash}"))
                && summary.contains("observed_status=in_progress"))
    );
    let imported_workpad_task = state
        .workpad_task(&project_id(), "workpads:features:tasks.md#f2")
        .expect("workpad task query")
        .expect("workpad task");
    assert_eq!(imported_workpad_task.observed_status, "in_progress");
    assert_eq!(imported_workpad_task.capo_execution_status, "imported");
    let next_after_import = run_cli(vec![
        "workpad".to_string(),
        "next".to_string(),
        "--path".to_string(),
        "workpads/features/tasks.md".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("select next after imported task");
    assert!(next_after_import.contains("workpad_next_found=true"));
    assert!(next_after_import.contains("workpad_task_id=workpads:features:tasks.md#f1"));
    assert!(next_after_import.contains("observed_status=pending"));
    assert!(next_after_import.contains("capo_execution_status=observed_only"));
    let missing_agent_start = run_cli(vec![
        "workpad".to_string(),
        "start-next".to_string(),
        "--agent".to_string(),
        "missing".to_string(),
        "--path".to_string(),
        "workpads/features/tasks.md".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect_err("missing agent should fail before import");
    assert!(missing_agent_start.contains("missing registered agent"));
    let next_after_missing_agent = run_cli(vec![
        "workpad".to_string(),
        "next".to_string(),
        "--path".to_string(),
        "workpads/features/tasks.md".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("missing agent should not consume next task");
    assert!(next_after_missing_agent.contains("workpad_task_id=workpads:features:tasks.md#f1"));
    assert!(next_after_missing_agent.contains("capo_execution_status=observed_only"));
    run_cli(vec![
        "agent".to_string(),
        "register".to_string(),
        "--name".to_string(),
        "dogfood".to_string(),
        "--adapter".to_string(),
        "fake".to_string(),
        "--runtime".to_string(),
        "fake".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("register dogfood agent");
    let started = run_cli(vec![
        "workpad".to_string(),
        "start-next".to_string(),
        "--agent".to_string(),
        "dogfood".to_string(),
        "--path".to_string(),
        "workpads/features/tasks.md".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("start next workpad task");
    assert!(started.contains("workpad_next_started=true"));
    assert!(started.contains("workpad_task_id=workpads:features:tasks.md#f1"));
    assert!(started.contains("task_id=task-workpad-workpads-features-tasks-md-f1"));
    assert!(started.contains("capo_execution_status=active"));
    let started_task = state
        .task(&TaskId::new("task-workpad-workpads-features-tasks-md-f1"))
        .expect("started task query")
        .expect("started task");
    assert_eq!(started_task.capo_execution_status, "active");
    assert_eq!(
        fs::read_to_string(project_root.join("workpads/features/tasks.md"))
            .expect("read source after start-next"),
        before
    );
    let proposal_dir = temp_root("workpad-proposal");
    let proposal_output = run_cli(vec![
        "workpad".to_string(),
        "propose".to_string(),
        "--workpad-task".to_string(),
        "workpads:features:tasks.md#f2".to_string(),
        "--expected-hash".to_string(),
        source_hash.clone(),
        "--out".to_string(),
        proposal_dir.display().to_string(),
        "--summary".to_string(),
        "Mark DB3 reviewed artifacts complete after verification.".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("write proposal");
    assert!(proposal_output.contains("workpad_proposal_written=true"));
    assert!(proposal_output.contains("source_hash="));
    let proposal_path = proposal_output
        .lines()
        .find_map(|line| line.strip_prefix("path="))
        .map(PathBuf::from)
        .expect("proposal path");
    let proposal = fs::read_to_string(&proposal_path).expect("read proposal");
    assert!(proposal.starts_with("<!-- capo:workpad-proposal -->"));
    assert!(proposal.contains("## Apply Policy"));
    assert!(proposal.contains("Automated source writeback is disabled"));
    assert!(proposal.contains("## Rollback And Fallback"));
    assert!(proposal.contains("Mark DB3 reviewed artifacts complete"));
    assert_eq!(
        fs::read_to_string(project_root.join("workpads/features/tasks.md"))
            .expect("read source after proposal"),
        before
    );
    let apply_without_confirm = run_cli(vec![
        "workpad".to_string(),
        "apply".to_string(),
        "--proposal".to_string(),
        proposal_path.display().to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect_err("apply should require confirmation");
    assert!(apply_without_confirm.contains("explicit --confirm is required"));
    let apply_with_confirm = run_cli(vec![
        "workpad".to_string(),
        "apply".to_string(),
        "--proposal".to_string(),
        proposal_path.display().to_string(),
        "--confirm".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("confirmed apply is guarded no-op in DB3");
    assert!(apply_with_confirm.contains("workpad_apply_supported=false"));
    assert!(apply_with_confirm.contains("source_modified=false"));
    assert_eq!(
        fs::read_to_string(project_root.join("workpads/features/tasks.md"))
            .expect("read source after apply"),
        before
    );
    let second_proposal_output = run_cli(vec![
        "workpad".to_string(),
        "propose".to_string(),
        "--workpad-task".to_string(),
        "workpads:features:tasks.md#f2".to_string(),
        "--expected-hash".to_string(),
        source_hash.clone(),
        "--out".to_string(),
        proposal_dir.display().to_string(),
        "--summary".to_string(),
        "A different reviewed proposal body gets a distinct artifact.".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("write second proposal");
    let second_proposal_path = second_proposal_output
        .lines()
        .find_map(|line| line.strip_prefix("path="))
        .map(PathBuf::from)
        .expect("second proposal path");
    assert_ne!(proposal_path, second_proposal_path);
    assert!(proposal_path.exists());
    assert!(second_proposal_path.exists());
    fs::write(
        &second_proposal_path,
        format!("{proposal}\nmanual review note\n"),
    )
    .expect("mutate Capo proposal");
    let changed_proposal_overwrite = run_cli(vec![
        "workpad".to_string(),
        "propose".to_string(),
        "--workpad-task".to_string(),
        "workpads:features:tasks.md#f2".to_string(),
        "--expected-hash".to_string(),
        source_hash.clone(),
        "--out".to_string(),
        proposal_dir.display().to_string(),
        "--summary".to_string(),
        "A different reviewed proposal body gets a distinct artifact.".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect_err("proposal should not overwrite changed Capo file");
    assert!(changed_proposal_overwrite.contains("refusing to overwrite changed Capo"));
    fs::write(&proposal_path, "# user-authored proposal\n").expect("replace proposal");
    let proposal_overwrite = run_cli(vec![
        "workpad".to_string(),
        "propose".to_string(),
        "--workpad-task".to_string(),
        "workpads:features:tasks.md#f2".to_string(),
        "--expected-hash".to_string(),
        source_hash.clone(),
        "--out".to_string(),
        proposal_dir.display().to_string(),
        "--summary".to_string(),
        "Mark DB3 reviewed artifacts complete after verification.".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect_err("proposal should not overwrite foreign file");
    assert!(proposal_overwrite.contains("refusing to overwrite non-Capo"));

    let conflicting_task_id = TaskId::new("task-existing-active");
    state
        .append_event(
            NewEvent {
                event_id: "event-existing-active-task".to_string(),
                kind: EventKind::TaskDiscovered,
                actor: "test".to_string(),
                project_id: Some(project_id()),
                task_id: Some(conflicting_task_id.clone()),
                agent_id: None,
                session_id: None,
                run_id: None,
                turn_id: None,
                item_id: None,
                payload_json: "{}".to_string(),
                idempotency_key: None,
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::Task(capo_state::TaskProjection {
                task_id: conflicting_task_id.clone(),
                project_id: project_id(),
                title: "Existing active task".to_string(),
                capo_execution_status: "active".to_string(),
                active_session_id: None,
                latest_summary: Some("unrelated task".to_string()),
                evidence_id: None,
                updated_sequence: 0,
            })],
        )
        .expect("existing active task");
    let collision_error = run_cli(vec![
        "workpad".to_string(),
        "import".to_string(),
        "--workpad-task".to_string(),
        "workpads:features:tasks.md#f2".to_string(),
        "--task".to_string(),
        conflicting_task_id.to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect_err("import should not overwrite existing Capo task");
    assert!(collision_error.contains("refusing to overwrite existing Capo task"));

    let event_count = state.event_count().expect("event count before re-import");
    run_cli(vec![
        "workpad".to_string(),
        "import".to_string(),
        "--workpad-task".to_string(),
        "workpads:features:tasks.md#f2".to_string(),
        "--expected-hash".to_string(),
        source_hash.clone(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("idempotent import");
    assert_eq!(
        state.event_count().expect("event count unchanged"),
        event_count
    );

    run_cli(vec![
        "workpad".to_string(),
        "index".to_string(),
        "--root".to_string(),
        project_root.display().to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("re-index workpads without source change");
    let imported_workpad_task = state
        .workpad_task(&project_id(), "workpads:features:tasks.md#f2")
        .expect("workpad task after re-index")
        .expect("workpad task after re-index");
    assert_eq!(imported_workpad_task.observed_status, "in_progress");
    assert_eq!(imported_workpad_task.capo_execution_status, "imported");

    fs::write(
            project_root.join("workpads/features/tasks.md"),
            "# Feature Tasks\n\n## Objective\n\nSplit work updated.\n\n## F1 - Real Local Agent Connector Proof\n\nStatus: pending\n\n## F2 - Workpad Dogfood Bridge\n\nStatus: in_progress\n",
        )
        .expect("drift feature tasks");
    run_cli(vec![
        "workpad".to_string(),
        "index".to_string(),
        "--root".to_string(),
        project_root.display().to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("re-index drifted workpads");
    let drift_error = run_cli(vec![
        "workpad".to_string(),
        "import".to_string(),
        "--workpad-task".to_string(),
        "workpads:features:tasks.md#f2".to_string(),
        "--expected-hash".to_string(),
        source_hash,
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect_err("old hash should detect source drift");
    assert!(drift_error.contains("source drift detected"));

    fs::write(
            project_root.join("workpads/features/tasks.md"),
            "# Feature Tasks\n\n## Objective\n\nSplit work.\n\n## F1 - Real Local Agent Connector Proof\n\nStatus: pending\n",
        )
        .expect("remove f2 from feature tasks");
    run_cli(vec![
        "workpad".to_string(),
        "index".to_string(),
        "--root".to_string(),
        project_root.display().to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("re-index workpads");
    state.rebuild_projections().expect("rebuild after re-index");
    let tasks = state
        .workpad_tasks(&project_id())
        .expect("tasks after re-index");
    assert!(
        !tasks
            .iter()
            .any(|task| task.workpad_task_id == "workpads:features:tasks.md#f2")
    );

    fs::write(project_root.join("workpads/features/tasks.md"), before)
        .expect("restore original feature tasks");
    run_cli(vec![
        "workpad".to_string(),
        "index".to_string(),
        "--root".to_string(),
        project_root.display().to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("re-index restored workpads");
    let tasks = state
        .workpad_tasks(&project_id())
        .expect("tasks after source recurrence");
    assert!(
        tasks
            .iter()
            .any(|task| task.workpad_task_id == "workpads:features:tasks.md#f2"),
        "restored source task should reappear after A-B-A fingerprint recurrence"
    );
}

#[test]
fn permission_approval_queue_maps_decisions_to_scoped_grants() {
    let state_root = temp_root("permission-approval-state");

    let request_output = run_cli(vec![
        "permission".to_string(),
        "request".to_string(),
        "--approval".to_string(),
        "approval-evidence-record".to_string(),
        "--profile".to_string(),
        "trusted-local-dev".to_string(),
        "--session".to_string(),
        "session-test".to_string(),
        "--tool-call".to_string(),
        "tool-call-evidence".to_string(),
        "--scope-json".to_string(),
        "[\"tool:invoke:capo.evidence_record\",\"state:write:evidence\"]".to_string(),
        "--subject-json".to_string(),
        "{\"actor\":\"local-user\",\"agent\":\"codex\"}".to_string(),
        "--reason".to_string(),
        "record evidence".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("request approval");
    assert!(request_output.contains("permission_approval_queued=true"));
    assert!(request_output.contains("status=pending"));

    let pending = run_cli(vec![
        "permission".to_string(),
        "list".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("list pending approvals");
    assert!(pending.contains("permission_approvals=1"));
    assert!(pending.contains("approval=approval-evidence-record status=pending"));

    let decide_output = run_cli(vec![
        "permission".to_string(),
        "decide".to_string(),
        "--approval".to_string(),
        "approval-evidence-record".to_string(),
        "--decision".to_string(),
        "allow_once".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("decide approval");
    assert!(decide_output.contains("permission_approval_decided=true"));
    assert!(decide_output.contains("effect=allow"));
    assert!(decide_output.contains("persistence=once"));
    let grant_id = decide_output
        .lines()
        .find_map(|line| line.strip_prefix("capability_grant_id="))
        .expect("grant id")
        .to_string();

    let decided = run_cli(vec![
        "permission".to_string(),
        "list".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("list decided approvals");
    assert!(decided.contains("approval=approval-evidence-record status=decided"));
    assert!(decided.contains("decision=allow_once"));
    assert!(decided.contains(&format!("grant={grant_id}")));

    let second_decision = run_cli(vec![
        "permission".to_string(),
        "decide".to_string(),
        "--approval".to_string(),
        "approval-evidence-record".to_string(),
        "--decision".to_string(),
        "reject_once".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect_err("decided approval cannot be decided again");
    assert!(second_decision.contains("approval is not pending"));

    let state = SqliteStateStore::open(&state_root).expect("state");
    state
        .rebuild_projections()
        .expect("rebuild approval projections");
    let approval = state
        .permission_approval(&project_id(), "approval-evidence-record")
        .expect("approval query")
        .expect("approval read model");
    assert_eq!(approval.status, "decided");
    assert_eq!(approval.decision.as_deref(), Some("allow_once"));
    assert_eq!(
        approval.capability_grant_id.as_deref(),
        Some(grant_id.as_str())
    );
    let grant = state
        .capability_grants()
        .expect("grant query")
        .into_iter()
        .find(|grant| grant.capability_grant_id == grant_id)
        .expect("grant");
    assert_eq!(grant.effect, "allow");
    assert_eq!(grant.persistence, "once");
    assert_eq!(grant.decision_source, "user");

    run_cli(vec![
        "permission".to_string(),
        "request".to_string(),
        "--approval".to_string(),
        "approval-shell".to_string(),
        "--scope-json".to_string(),
        "[\"tool:invoke:shell\"]".to_string(),
        "--reason".to_string(),
        "run shell".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("request second approval");
    run_cli(vec![
        "permission".to_string(),
        "decide".to_string(),
        "--approval".to_string(),
        "approval-shell".to_string(),
        "--decision".to_string(),
        "reject_always".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("reject second approval");
    let denied = state
        .capability_grants()
        .expect("grant query")
        .into_iter()
        .find(|grant| {
            grant
                .explanation
                .contains("reject_always for approval-shell")
        })
        .expect("deny grant");
    assert_eq!(denied.effect, "deny");
    assert_eq!(denied.persistence, "until_revoked");

    run_cli(vec![
        "permission".to_string(),
        "request".to_string(),
        "--approval".to_string(),
        "approval-reject-once".to_string(),
        "--scope-json".to_string(),
        "[\"tool:invoke:shell\"]".to_string(),
        "--reason".to_string(),
        "reject one shell request".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("request reject-once approval");
    let reject_once = run_cli(vec![
        "permission".to_string(),
        "decide".to_string(),
        "--approval".to_string(),
        "approval-reject-once".to_string(),
        "--decision".to_string(),
        "reject_once".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("reject once");
    assert!(reject_once.contains("capability_grant_id=none"));
    let reject_once_approval = state
        .permission_approval(&project_id(), "approval-reject-once")
        .expect("reject-once approval query")
        .expect("reject-once approval");
    assert_eq!(
        reject_once_approval.decision.as_deref(),
        Some("reject_once")
    );
    assert!(reject_once_approval.capability_grant_id.is_none());

    run_cli(vec![
        "permission".to_string(),
        "request".to_string(),
        "--approval".to_string(),
        "approval-broad-always".to_string(),
        "--scope-json".to_string(),
        "[\"tool:invoke:shell\"]".to_string(),
        "--reason".to_string(),
        "broad remembered allow".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("request broad allow-always approval");
    let broad_always = run_cli(vec![
        "permission".to_string(),
        "decide".to_string(),
        "--approval".to_string(),
        "approval-broad-always".to_string(),
        "--decision".to_string(),
        "allow_always".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect_err("broad remembered allow is rejected");
    assert!(broad_always.contains("allow_always is restricted"));

    let bad_scope = run_cli(vec![
        "permission".to_string(),
        "request".to_string(),
        "--approval".to_string(),
        "approval-bad".to_string(),
        "--scope-json".to_string(),
        "{\"scope\":\"tool:invoke:shell\"}".to_string(),
        "--reason".to_string(),
        "bad scope".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect_err("object scope is rejected");
    assert!(bad_scope.contains("JSON array of strings"));
}

#[test]
fn cli_drives_fake_controller_and_exports_evidence() {
    let state_root = temp_root("cli-state");
    let evidence_dir = temp_root("cli-evidence");

    assert!(
        run_cli(vec![
            "init".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .unwrap()
        .contains("initialized=true")
    );

    run_cli(vec![
        "agent".to_string(),
        "register".to_string(),
        "--name".to_string(),
        "fake-codex".to_string(),
        "--adapter".to_string(),
        "fake".to_string(),
        "--runtime".to_string(),
        "fake".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .unwrap();

    let send = run_cli(vec![
        "task".to_string(),
        "send".to_string(),
        "--agent".to_string(),
        "fake-codex".to_string(),
        "--goal".to_string(),
        "Inspect the project and write a short status summary".to_string(),
        "--scenario".to_string(),
        "tool-memory".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .unwrap();
    assert!(send.contains("session_id=session-fake-codex"));

    let status = run_cli(vec![
        "session".to_string(),
        "status".to_string(),
        "--agent".to_string(),
        "fake-codex".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .unwrap();
    assert!(status.contains("current_goal=Inspect the project"));
    assert!(status.contains("kind=tool.call_completed"));
    assert!(status.contains("evidence_refs=evidence-fake-codex"));

    let interrupted = run_cli(vec![
        "session".to_string(),
        "interrupt".to_string(),
        "--agent".to_string(),
        "fake-codex".to_string(),
        "--reason".to_string(),
        "smoke interrupt".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .unwrap();
    assert!(interrupted.contains("status=canceled"));

    assert!(
        run_cli(vec![
            "recover".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .unwrap()
        .contains("recovered=true")
    );

    let export = run_cli(vec![
        "evidence".to_string(),
        "export".to_string(),
        "--session".to_string(),
        "session-fake-codex".to_string(),
        "--out".to_string(),
        evidence_dir.display().to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .unwrap();
    assert!(export.contains("evidence_exported=true"));
    let evidence_path = evidence_dir.join("session-fake-codex.md");
    let exported = fs::read_to_string(&evidence_path).expect("read evidence export");
    assert!(exported.starts_with("<!-- capo:evidence-export -->"));
    assert!(exported.contains("## State Refs"));
    assert!(exported.contains("- Session status: `canceled`"));
    assert!(exported.contains("- Run status: `exited_unknown`"));
    assert!(exported.contains("- `evidence-fake-codex`"));
    assert!(exported.contains("artifact=`artifact-tool-session-fake-codex`"));
    assert!(exported.contains("## Tool Calls"));
    assert!(exported.contains("origin=`capo` status=`completed`"));
    assert!(exported.contains("## Memory Packets"));
    assert!(exported.contains("artifact=`artifact-memory-packet-packet-fake-codex`"));
    assert!(exported.contains("session.interrupted"));
    assert!(!exported.contains("OPENAI_API_KEY"));
    assert!(!exported.contains("ANTHROPIC_API_KEY"));
    let state = SqliteStateStore::open(&state_root).expect("open state");
    let review_recorded = run_cli(vec![
        "review".to_string(),
        "record".to_string(),
        "--session".to_string(),
        "session-fake-codex".to_string(),
        "--reviewer".to_string(),
        "focused-review".to_string(),
        "--kind".to_string(),
        "no_blockers".to_string(),
        "--summary".to_string(),
        "No blockers in exported fake controller evidence.".to_string(),
        "--out".to_string(),
        evidence_dir.display().to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("record no-blockers review");
    assert!(review_recorded.contains("review_finding_recorded=true"));

    let outcome = run_cli(vec![
        "eval".to_string(),
        "task-outcome".to_string(),
        "--session".to_string(),
        "session-fake-codex".to_string(),
        "--out".to_string(),
        evidence_dir.display().to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .unwrap();
    assert!(outcome.contains("task_outcome_report_exported=true"));
    assert!(outcome.contains("artifact_id=artifact-task-outcome-"));
    let reports = state
        .task_outcome_reports_for_task(&TaskId::new(
            "task-inspect-the-project-and-write-a-short-status-summary",
        ))
        .expect("task outcome reports");
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].review_outcome, "reviewed_no_blockers");
    assert!(reports[0].tool_call_count >= 1);
    assert!(reports[0].evidence_count >= 1);
    let report_path = evidence_dir.join(format!(
        "{}.md",
        reports[0]
            .report_artifact_id
            .as_deref()
            .expect("report artifact")
    ));
    let report = fs::read_to_string(report_path).expect("read task outcome report");
    assert!(report.starts_with("<!-- capo:task-outcome-report -->"));
    assert!(report.contains("Review outcome: `reviewed_no_blockers`"));
    assert!(report.contains("## Event Trace"));
    let rerun = run_cli(vec![
        "eval".to_string(),
        "task-outcome".to_string(),
        "--session".to_string(),
        "session-fake-codex".to_string(),
        "--out".to_string(),
        evidence_dir.display().to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .unwrap();
    assert!(rerun.contains("task_outcome_report_exported=true"));
    assert!(
        rerun.contains(
            reports[0]
                .report_artifact_id
                .as_deref()
                .expect("stable report artifact")
        )
    );
    assert_eq!(
        state
            .task_outcome_reports_for_task(&TaskId::new(
                "task-inspect-the-project-and-write-a-short-status-summary",
            ))
            .expect("task outcome reports")
            .len(),
        1
    );
    state
        .append_event(
            NewEvent::new(
                "event-workpad-me3-follow-up",
                EventKind::WorkpadIndexed,
                "test",
            ),
            &[ProjectionRecord::WorkpadTask(WorkpadTaskProjection {
                workpad_task_id: "ME3".to_string(),
                project_id: project_id(),
                path: "workpads/features/memory-eval.md".to_string(),
                source_anchor: "ME3 - Review Feedback Loop".to_string(),
                title: "Review Feedback Loop".to_string(),
                observed_status: "pending".to_string(),
                capo_execution_status: "observed_only".to_string(),
                observed_unix: 1,
                updated_sequence: 0,
            })],
        )
        .expect("append ME3 follow-up workpad task");

    let blocker_review = run_cli(vec![
        "review".to_string(),
        "record".to_string(),
        "--session".to_string(),
        "session-fake-codex".to_string(),
        "--reviewer".to_string(),
        "focused-review".to_string(),
        "--kind".to_string(),
        "blocker".to_string(),
        "--summary".to_string(),
        "Tool output needs follow-up workpad handling.".to_string(),
        "--tool-call".to_string(),
        "tool-fake-codex".to_string(),
        "--follow-up-workpad-task".to_string(),
        "ME3".to_string(),
        "--out".to_string(),
        evidence_dir.display().to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("record blocker review");
    assert!(blocker_review.contains("review_finding_recorded=true"));
    let blocker_outcome = run_cli(vec![
        "eval".to_string(),
        "task-outcome".to_string(),
        "--session".to_string(),
        "session-fake-codex".to_string(),
        "--out".to_string(),
        evidence_dir.display().to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .unwrap();
    assert!(blocker_outcome.contains("task_outcome_report_exported=true"));
    let blocker_reports = state
        .task_outcome_reports_for_task(&TaskId::new(
            "task-inspect-the-project-and-write-a-short-status-summary",
        ))
        .expect("task outcome reports after blocker review");
    assert!(
        blocker_reports
            .iter()
            .any(|report| report.review_outcome == "reviewed_with_findings")
    );
    let findings = state
        .review_findings_for_session(&SessionId::new("session-fake-codex"))
        .expect("review findings");
    assert_eq!(findings.len(), 2);
    let blocker = findings
        .iter()
        .find(|finding| finding.finding_kind == "blocker")
        .expect("blocker finding");
    assert_eq!(
        blocker
            .tool_call_id
            .as_ref()
            .map(ToString::to_string)
            .as_deref(),
        Some("tool-fake-codex")
    );
    assert_eq!(blocker.workpad_task_id.as_deref(), Some("ME3"));
    let review_artifact = fs::read_to_string(
        evidence_dir
            .join(
                blocker
                    .evidence_artifact_id
                    .as_ref()
                    .expect("review artifact"),
            )
            .with_extension("md"),
    )
    .expect("read review artifact");
    assert!(review_artifact.starts_with("<!-- capo:review-finding -->"));
    assert!(review_artifact.contains("Follow-up workpad task: `ME3`"));
}

#[test]
fn dashboard_rejects_malformed_filters() {
    let state_root = temp_root("cli-dashboard-filters");

    let missing_session = run_cli(vec![
        "dashboard".to_string(),
        "--session".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .unwrap_err();
    assert!(missing_session.contains("--session requires a value"));

    let missing_status = run_cli(vec![
        "dashboard".to_string(),
        "--status".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .unwrap_err();
    assert!(missing_status.contains("--status requires a value"));

    let unknown = run_cli(vec![
        "dashboard".to_string(),
        "--agent".to_string(),
        "fake-codex".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .unwrap_err();
    assert!(unknown.contains("unknown dashboard filter: --agent"));

    let missing_workpad_path = run_cli(vec![
        "dashboard".to_string(),
        "--workpad-path".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .unwrap_err();
    assert!(missing_workpad_path.contains("--workpad-path requires a value"));

    let missing_workpad_status = run_cli(vec![
        "dashboard".to_string(),
        "--workpad-status".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .unwrap_err();
    assert!(missing_workpad_status.contains("--workpad-status requires a value"));
}

#[test]
fn dashboard_renders_review_findings_from_shared_query() {
    let state_root = temp_root("cli-dashboard-review-findings");
    let evidence_dir = temp_root("cli-dashboard-review-finding-evidence");
    seed_running_agent(&state_root, "fake-codex", "Inspect the project");

    let review = run_cli(vec![
        "review".to_string(),
        "record".to_string(),
        "--session".to_string(),
        "session-fake-codex".to_string(),
        "--reviewer".to_string(),
        "focused-review".to_string(),
        "--kind".to_string(),
        "blocker".to_string(),
        "--summary".to_string(),
        "Dashboard must expose review blockers.".to_string(),
        "--out".to_string(),
        evidence_dir.display().to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("record review finding");
    assert!(review.contains("review_finding_recorded=true"));
    let review_finding_id = output_value(&review, "review_finding_id");

    let dashboard = run_cli(vec![
        "dashboard".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("dashboard with review finding");

    assert!(dashboard.contains("review_findings=1"));
    assert!(dashboard.contains("session_review_findings=1"));
    assert!(dashboard.contains(&format!("project_review_finding={review_finding_id}")));
    assert!(dashboard.contains(&format!("review_finding={review_finding_id}")));
    assert!(dashboard.contains("kind=blocker"));
    assert!(dashboard.contains("severity=high"));
    assert!(dashboard.contains("status=open"));
    assert!(dashboard.contains("reviewer=focused-review"));
    assert!(dashboard.contains("summary=Dashboard must expose review blockers."));
}

#[test]
fn dashboard_renders_task_outcome_reports_from_shared_query() {
    let state_root = temp_root("cli-dashboard-task-outcome");
    seed_running_agent(&state_root, "fake-codex", "Inspect the project");
    let report_id = "task-outcome-report-dashboard";
    let artifact_id = "artifact-task-outcome-dashboard";
    SqliteStateStore::open(&state_root)
        .expect("state")
        .append_event(
            NewEvent {
                event_id: "event-cli-dashboard-task-outcome".to_string(),
                kind: EventKind::TaskOutcomeReportGenerated,
                actor: "test".to_string(),
                project_id: Some(project_id()),
                task_id: Some(TaskId::new("task-inspect-the-project")),
                agent_id: Some(AgentId::new("agent-fake-codex")),
                session_id: Some(SessionId::new("session-fake-codex")),
                run_id: Some(RunId::new("run-fake-codex")),
                turn_id: None,
                item_id: Some(report_id.to_string()),
                payload_json: "{}".to_string(),
                idempotency_key: None,
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::TaskOutcomeReport(
                capo_state::TaskOutcomeReportProjection {
                    task_outcome_report_id: report_id.to_string(),
                    project_id: project_id(),
                    task_id: TaskId::new("task-inspect-the-project"),
                    session_id: SessionId::new("session-fake-codex"),
                    run_id: RunId::new("run-fake-codex"),
                    outcome_status: "completed".to_string(),
                    started_sequence: 1,
                    completed_sequence: 10,
                    duration_sequence_span: 9,
                    action_count: 5,
                    tool_call_count: 1,
                    evidence_count: 2,
                    memory_packet_count: 1,
                    confidence: Some(82),
                    blocker: None,
                    review_outcome: "not_reviewed".to_string(),
                    report_artifact_id: Some(artifact_id.to_string()),
                    updated_sequence: 0,
                },
            )],
        )
        .expect("append task outcome report");

    let dashboard = run_cli(vec![
        "dashboard".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("dashboard with task outcome report");

    assert!(dashboard.contains("task_outcome_reports=1"));
    assert!(dashboard.contains("session_task_outcome_reports=1"));
    assert!(dashboard.contains(&format!("project_task_outcome_report={report_id}")));
    assert!(dashboard.contains(&format!("task_outcome_report={report_id}")));
    assert!(dashboard.contains("outcome_status=completed"));
    assert!(dashboard.contains("review_outcome=not_reviewed"));
    assert!(dashboard.contains(&format!("artifact={artifact_id}")));
}

#[test]
fn dashboard_renders_connectivity_exposure_state() {
    let state_root = temp_root("cli-dashboard-connectivity");
    let state = SqliteStateStore::open(&state_root).expect("state");
    state
        .append_event(
            NewEvent {
                event_id: "event-cli-connectivity-exposure".to_string(),
                kind: EventKind::ConnectivityExposureRequested,
                actor: "test".to_string(),
                project_id: Some(project_id()),
                task_id: None,
                agent_id: None,
                session_id: None,
                run_id: None,
                turn_id: None,
                item_id: Some("exposure-private-control".to_string()),
                payload_json: "{}".to_string(),
                idempotency_key: None,
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::ConnectivityExposure(
                ConnectivityExposureProjection {
                    exposure_id: "exposure-private-control".to_string(),
                    project_id: project_id(),
                    connectivity_endpoint_id: "endpoint-private-1".to_string(),
                    owner_kind: "runtime_target".to_string(),
                    owner_id: "remote-target-1".to_string(),
                    channel_kind: "control".to_string(),
                    exposure: "private".to_string(),
                    permission_scope: "network:connect:private_tunnel".to_string(),
                    status: "blocked_pending_permission".to_string(),
                    capability_grant_id: None,
                    health_status: "unknown".to_string(),
                    reachable: false,
                    revoked_at: None,
                    updated_sequence: 0,
                },
            )],
        )
        .expect("append exposure");

    let dashboard = run_cli(vec![
        "dashboard".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("dashboard");

    assert!(dashboard.contains("connectivity_exposures=1"));
    assert!(dashboard.contains("connectivity_exposure=exposure-private-control"));
    assert!(dashboard.contains("exposure_status=blocked_pending_permission"));
    assert!(dashboard.contains("permission_scope=network:connect:private_tunnel"));
    assert!(dashboard.contains("grant=none"));
}

#[test]
fn runtime_target_register_lists_dashboard_metadata_without_execution() {
    let state_root = temp_root("cli-runtime-target-register");
    let workspace = temp_root("runtime-target-workspace");
    let artifacts = temp_root("runtime-target-artifacts");
    let registered = run_cli(vec![
        "runtime".to_string(),
        "target".to_string(),
        "register".to_string(),
        "--target".to_string(),
        "runtime-target-local-1".to_string(),
        "--name".to_string(),
        "local dev box".to_string(),
        "--runner".to_string(),
        "local-process".to_string(),
        "--workspace".to_string(),
        workspace.display().to_string(),
        "--artifacts".to_string(),
        artifacts.display().to_string(),
        "--capability-profile".to_string(),
        "read-only-local".to_string(),
        "--endpoint".to_string(),
        "endpoint-loopback-1".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("register runtime target");

    assert!(registered.contains("runtime_target_registered=true"));
    assert!(registered.contains("runtime_target=runtime-target-local-1"));
    assert!(registered.contains("runner=local-process"));
    assert!(registered.contains("capability_profile=read-only-local"));
    assert!(registered.contains("endpoint=endpoint-loopback-1"));
    assert!(registered.contains("provider_cli_executed=false"));
    assert!(registered.contains("tunnel_opened=false"));

    let list = run_cli(vec![
        "runtime".to_string(),
        "target".to_string(),
        "list".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("list runtime targets");
    assert!(list.contains("runtime_targets=1"));
    assert!(list.contains("runtime_target=runtime-target-local-1"));
    assert!(list.contains("status=available"));

    let status = run_cli(vec![
        "runtime".to_string(),
        "target".to_string(),
        "status".to_string(),
        "--target".to_string(),
        "runtime-target-local-1".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("runtime target status");
    assert!(status.contains("runtime_target_status_found=true"));
    assert!(status.contains("runtime_target=runtime-target-local-1"));
    assert!(status.contains("status=available"));
    assert!(status.contains("provider_cli_executed=false"));
    assert!(status.contains("tunnel_opened=false"));
    assert!(status.contains("runtime_process_started=false"));
    assert!(status.contains("state_mutated=false"));
    assert!(status.contains("runtime_target_selector=exact"));

    let latest_status = run_cli(vec![
        "runtime".to_string(),
        "target".to_string(),
        "status".to_string(),
        "--latest".to_string(),
        "--runner".to_string(),
        "local-process".to_string(),
        "--status".to_string(),
        "available".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("latest runtime target status");
    assert!(latest_status.contains("runtime_target_status_found=true"));
    assert!(latest_status.contains("runtime_target_selector=latest"));
    assert!(latest_status.contains("runtime_target_filter_runner=local-process"));
    assert!(latest_status.contains("runtime_target_filter_status=available"));
    assert!(latest_status.contains("runtime_target=runtime-target-local-1"));
    assert!(latest_status.contains("provider_cli_executed=false"));
    assert!(latest_status.contains("tunnel_opened=false"));
    assert!(latest_status.contains("runtime_process_started=false"));
    assert!(latest_status.contains("state_mutated=false"));

    let latest_missing = run_cli(vec![
        "runtime".to_string(),
        "target".to_string(),
        "status".to_string(),
        "--latest".to_string(),
        "--runner".to_string(),
        "remote-process".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect_err("missing latest runtime target status");
    assert!(latest_missing.contains("no recorded runtime targets matching runner=remote-process"));

    let voice_status = run_cli(vec![
        "voice".to_string(),
        "submit".to_string(),
        "--transcript".to_string(),
        "What is the runtime target status for runtime target local 1?".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("voice runtime target status");
    assert!(voice_status.contains("voice_plan=runtime_target_status"));
    assert!(voice_status.contains("read_scope=project_runtime_target_status"));
    assert!(voice_status.contains("spoken_runtime_target=runtime-target-local-1"));
    assert!(voice_status.contains("spoken_runner=local-process"));
    assert!(voice_status.contains("spoken_capability_profile=read-only-local"));
    assert!(voice_status.contains("spoken_endpoint=endpoint-loopback-1"));
    assert!(voice_status.contains("spoken_runtime_status=available"));
    assert!(voice_status.contains("mutation_applied=false"));
    assert!(voice_status.contains("raw_transcript_retained=false"));

    let latest_voice_status = run_cli(vec![
        "voice".to_string(),
        "submit".to_string(),
        "--transcript".to_string(),
        "What is the latest runtime target status for available local process?".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("voice latest runtime target status");
    assert!(latest_voice_status.contains("voice_plan=runtime_target_status"));
    assert!(latest_voice_status.contains("read_scope=project_latest_runtime_target_status"));
    assert!(latest_voice_status.contains("spoken_runtime_target=runtime-target-local-1"));
    assert!(latest_voice_status.contains("spoken_runner=local-process"));
    assert!(latest_voice_status.contains("spoken_runtime_status=available"));
    assert!(latest_voice_status.contains("mutation_applied=false"));
    assert!(latest_voice_status.contains("raw_transcript_retained=false"));

    let latest_voice_missing = run_cli(vec![
        "voice".to_string(),
        "submit".to_string(),
        "--transcript".to_string(),
        "What is the latest runtime target status for remote process?".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("voice missing latest runtime target status");
    assert!(latest_voice_missing.contains("voice_plan=runtime_target_status"));
    assert!(latest_voice_missing.contains("spoken_latest_runtime_target_missing=true"));
    assert!(
        latest_voice_missing.contains("spoken_latest_runtime_target_filter_runner=remote-process")
    );

    let missing_voice_status = run_cli(vec![
        "voice".to_string(),
        "submit".to_string(),
        "--transcript".to_string(),
        "What is the runtime target status for missing runtime target?".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("voice missing runtime target status");
    assert!(missing_voice_status.contains("voice_plan=runtime_target_status"));
    assert!(missing_voice_status.contains("spoken_runtime_target_missing=missing-runtime-target"));

    let missing_status = run_cli(vec![
        "runtime".to_string(),
        "target".to_string(),
        "status".to_string(),
        "--target".to_string(),
        "missing-runtime-target".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect_err("missing runtime target status");
    assert!(missing_status.contains("missing runtime target: missing-runtime-target"));

    let malformed_status_filter = run_cli(vec![
        "runtime".to_string(),
        "target".to_string(),
        "status".to_string(),
        "--status".to_string(),
        "available".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect_err("status filter without latest");
    assert!(
        malformed_status_filter
            .contains("runtime target status --runner/--status filters require --latest")
    );

    let evidence_dir = temp_root("runtime-target-evidence");
    let evidence = run_cli(vec![
        "runtime".to_string(),
        "target".to_string(),
        "evidence".to_string(),
        "--target".to_string(),
        "runtime-target-local-1".to_string(),
        "--out".to_string(),
        evidence_dir.display().to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("runtime target evidence");
    assert!(evidence.contains("runtime_target_evidence_exported=true"));
    assert!(evidence.contains("runtime_target=runtime-target-local-1"));
    assert!(evidence.contains("evidence_id=evidence-artifact-runtime-target-evidence-"));
    let evidence_path = output_value(&evidence, "path");
    let markdown = fs::read_to_string(&evidence_path).expect("read runtime target evidence");
    assert!(markdown.starts_with("<!-- capo:runtime-target-evidence -->"));
    assert!(markdown.contains("## Runtime Target"));
    assert!(markdown.contains("- Status: `available`"));
    assert!(markdown.contains("- Connectivity endpoint: `endpoint-loopback-1`"));
    assert!(markdown.contains("does not launch runtimes"));

    let latest_evidence_dir = temp_root("runtime-target-latest-evidence");
    let latest_evidence = run_cli(vec![
        "runtime".to_string(),
        "target".to_string(),
        "evidence".to_string(),
        "--latest".to_string(),
        "--runner".to_string(),
        "local-process".to_string(),
        "--status".to_string(),
        "available".to_string(),
        "--out".to_string(),
        latest_evidence_dir.display().to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("latest runtime target evidence");
    assert!(latest_evidence.contains("runtime_target_evidence_exported=true"));
    assert!(latest_evidence.contains("runtime_target_selector=latest"));
    assert!(latest_evidence.contains("runtime_target_filter_runner=local-process"));
    assert!(latest_evidence.contains("runtime_target_filter_status=available"));
    assert!(latest_evidence.contains("runtime_target=runtime-target-local-1"));
    assert!(latest_evidence.contains("provider_cli_executed=false"));
    assert!(latest_evidence.contains("tunnel_opened=false"));
    assert!(latest_evidence.contains("runtime_process_started=false"));
    assert!(latest_evidence.contains("state_mutated=false"));
    let latest_evidence_path = output_value(&latest_evidence, "path");
    let latest_markdown =
        fs::read_to_string(&latest_evidence_path).expect("read latest runtime target evidence");
    assert!(latest_markdown.starts_with("<!-- capo:runtime-target-evidence -->"));
    assert!(latest_markdown.contains("- Runtime target: `runtime-target-local-1`"));

    let latest_evidence_missing = run_cli(vec![
        "runtime".to_string(),
        "target".to_string(),
        "evidence".to_string(),
        "--latest".to_string(),
        "--runner".to_string(),
        "remote-process".to_string(),
        "--out".to_string(),
        latest_evidence_dir.display().to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect_err("missing latest runtime target evidence");
    assert!(
        latest_evidence_missing
            .contains("no recorded runtime targets matching runner=remote-process")
    );

    let malformed_evidence_filter = run_cli(vec![
        "runtime".to_string(),
        "target".to_string(),
        "evidence".to_string(),
        "--status".to_string(),
        "available".to_string(),
        "--out".to_string(),
        latest_evidence_dir.display().to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect_err("evidence filter without latest");
    assert!(
        malformed_evidence_filter
            .contains("runtime target evidence --runner/--status filters require --latest")
    );

    let missing_evidence = run_cli(vec![
        "runtime".to_string(),
        "target".to_string(),
        "evidence".to_string(),
        "--target".to_string(),
        "missing-runtime-target".to_string(),
        "--out".to_string(),
        evidence_dir.display().to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect_err("missing runtime target evidence");
    assert!(missing_evidence.contains("missing runtime target: missing-runtime-target"));

    let dashboard = run_cli(vec![
        "dashboard".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("dashboard");
    assert!(dashboard.contains("runtime_targets=1"));
    assert!(dashboard.contains("runtime_target=runtime-target-local-1"));
    assert!(dashboard.contains("runner=local-process"));
    assert!(dashboard.contains("target_ready=true"));
    assert!(dashboard.contains("control_exposure_status=missing"));
    assert!(dashboard.contains("ready=false"));
    assert!(dashboard.contains("next_action=record_control_connectivity_exposure"));
    assert!(dashboard.contains("project_evidence=1"));
    assert!(dashboard.contains("kind=runtime_target_evidence"));
}

#[test]
fn connectivity_expose_stub_records_blocked_private_exposure_without_runtime_execution() {
    let state_root = temp_root("cli-connectivity-expose-stub");
    let blocked_unknown_target = run_cli(vec![
        "connectivity".to_string(),
        "expose-stub".to_string(),
        "--endpoint".to_string(),
        "endpoint-private-1".to_string(),
        "--owner-kind".to_string(),
        "runtime_target".to_string(),
        "--owner-id".to_string(),
        "remote-target-1".to_string(),
        "--channel".to_string(),
        "control".to_string(),
        "--exposure".to_string(),
        "private".to_string(),
        "--record".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect_err("record private exposure with unknown runtime target");
    assert!(blocked_unknown_target.contains("unknown runtime target"));

    run_cli(vec![
        "runtime".to_string(),
        "target".to_string(),
        "register".to_string(),
        "--target".to_string(),
        "remote-target-1".to_string(),
        "--name".to_string(),
        "remote target 1".to_string(),
        "--runner".to_string(),
        "remote-process".to_string(),
        "--workspace".to_string(),
        "/tmp/capo-remote-workspace".to_string(),
        "--artifacts".to_string(),
        "/tmp/capo-remote-artifacts".to_string(),
        "--endpoint".to_string(),
        "endpoint-private-1".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("register runtime target");

    let mismatched_endpoint = run_cli(vec![
        "connectivity".to_string(),
        "expose-stub".to_string(),
        "--endpoint".to_string(),
        "endpoint-private-2".to_string(),
        "--owner-kind".to_string(),
        "runtime_target".to_string(),
        "--owner-id".to_string(),
        "remote-target-1".to_string(),
        "--channel".to_string(),
        "control".to_string(),
        "--exposure".to_string(),
        "private".to_string(),
        "--record".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect_err("record private exposure with mismatched registered endpoint");
    assert!(mismatched_endpoint.contains("runtime target endpoint mismatch"));

    run_cli(vec![
        "runtime".to_string(),
        "target".to_string(),
        "register".to_string(),
        "--target".to_string(),
        "remote-target-disabled".to_string(),
        "--name".to_string(),
        "disabled remote target".to_string(),
        "--runner".to_string(),
        "remote-process".to_string(),
        "--workspace".to_string(),
        "/tmp/capo-disabled-workspace".to_string(),
        "--artifacts".to_string(),
        "/tmp/capo-disabled-artifacts".to_string(),
        "--endpoint".to_string(),
        "endpoint-disabled-1".to_string(),
        "--status".to_string(),
        "disabled".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("register disabled runtime target");
    let disabled_target = run_cli(vec![
        "connectivity".to_string(),
        "expose-stub".to_string(),
        "--endpoint".to_string(),
        "endpoint-disabled-1".to_string(),
        "--owner-kind".to_string(),
        "runtime_target".to_string(),
        "--owner-id".to_string(),
        "remote-target-disabled".to_string(),
        "--channel".to_string(),
        "control".to_string(),
        "--exposure".to_string(),
        "private".to_string(),
        "--record".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect_err("record private exposure for disabled target");
    assert!(disabled_target.contains("runtime target is not available"));

    let enabled_target = run_cli(vec![
        "runtime".to_string(),
        "target".to_string(),
        "set-status".to_string(),
        "--target".to_string(),
        "remote-target-disabled".to_string(),
        "--status".to_string(),
        "available".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("enable runtime target");
    assert!(enabled_target.contains("runtime_target_status_updated=true"));
    assert!(enabled_target.contains("status=available"));
    assert!(enabled_target.contains("provider_cli_executed=false"));
    assert!(enabled_target.contains("tunnel_opened=false"));
    let enabled_exposure = run_cli(vec![
        "connectivity".to_string(),
        "expose-stub".to_string(),
        "--endpoint".to_string(),
        "endpoint-disabled-1".to_string(),
        "--owner-kind".to_string(),
        "runtime_target".to_string(),
        "--owner-id".to_string(),
        "remote-target-disabled".to_string(),
        "--channel".to_string(),
        "control".to_string(),
        "--exposure".to_string(),
        "private".to_string(),
        "--record".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("record private exposure after enabling target");
    assert!(enabled_exposure.contains("connectivity_exposure_planned=true"));
    assert!(enabled_exposure.contains("recorded=true"));

    let planned = run_cli(vec![
        "connectivity".to_string(),
        "expose-stub".to_string(),
        "--endpoint".to_string(),
        "endpoint-private-1".to_string(),
        "--owner-kind".to_string(),
        "runtime_target".to_string(),
        "--owner-id".to_string(),
        "remote-target-1".to_string(),
        "--channel".to_string(),
        "control".to_string(),
        "--exposure".to_string(),
        "private".to_string(),
        "--record".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("record private exposure");

    assert!(planned.contains("connectivity_exposure_planned=true"));
    assert!(planned.contains("permission_required=true"));
    assert!(planned.contains("permission_scope=network:connect:private_tunnel"));
    assert!(planned.contains("status=blocked_pending_permission"));
    assert!(planned.contains("recorded=true"));

    let dashboard = run_cli(vec![
        "dashboard".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("dashboard");
    assert!(dashboard.contains("connectivity_exposures=2"));
    assert!(dashboard.contains("endpoint=endpoint-private-1"));
    assert!(dashboard.contains("endpoint=endpoint-disabled-1"));
    assert!(dashboard.contains("owner=runtime_target:remote-target-1"));
    assert!(dashboard.contains("owner=runtime_target:remote-target-disabled"));
    assert!(dashboard.contains("exposure_status=blocked_pending_permission"));
    assert!(dashboard.contains("permission_scope=network:connect:private_tunnel"));

    let denied = run_cli(vec![
        "connectivity".to_string(),
        "expose-stub".to_string(),
        "--endpoint".to_string(),
        "endpoint-public-1".to_string(),
        "--owner-kind".to_string(),
        "capo_server".to_string(),
        "--owner-id".to_string(),
        "server-1".to_string(),
        "--channel".to_string(),
        "control".to_string(),
        "--exposure".to_string(),
        "public".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .unwrap_err();
    assert!(denied.contains("connectivity endpoint resolution failed"));
    assert!(denied.contains("ChannelNotAllowed"));
}

#[test]
fn connectivity_exposure_approval_activates_only_with_matching_grant() {
    let state_root = temp_root("cli-connectivity-exposure-approval");
    run_cli(vec![
        "runtime".to_string(),
        "target".to_string(),
        "register".to_string(),
        "--target".to_string(),
        "remote-target-1".to_string(),
        "--name".to_string(),
        "remote target 1".to_string(),
        "--runner".to_string(),
        "remote-process".to_string(),
        "--workspace".to_string(),
        "/tmp/capo-remote-workspace".to_string(),
        "--artifacts".to_string(),
        "/tmp/capo-remote-artifacts".to_string(),
        "--endpoint".to_string(),
        "endpoint-private-1".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("register runtime target");

    let planned = run_cli(vec![
        "connectivity".to_string(),
        "expose-stub".to_string(),
        "--endpoint".to_string(),
        "endpoint-private-1".to_string(),
        "--owner-kind".to_string(),
        "runtime_target".to_string(),
        "--owner-id".to_string(),
        "remote-target-1".to_string(),
        "--channel".to_string(),
        "control".to_string(),
        "--exposure".to_string(),
        "private".to_string(),
        "--record".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("record private exposure");
    let exposure_id = output_value(&planned, "exposure");

    let blocked = run_cli(vec![
        "connectivity".to_string(),
        "activate-exposure".to_string(),
        "--exposure".to_string(),
        exposure_id.clone(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .unwrap_err();
    assert!(blocked.contains("missing allow grant for connectivity exposure"));

    let readiness_before_grant = run_cli(vec![
        "runtime".to_string(),
        "target".to_string(),
        "readiness".to_string(),
        "--target".to_string(),
        "remote-target-1".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("runtime target readiness before grant");
    assert!(readiness_before_grant.contains("runtime_target_control_readiness_found=true"));
    assert!(readiness_before_grant.contains("runtime_target=remote-target-1"));
    assert!(readiness_before_grant.contains("target_ready=true"));
    assert!(readiness_before_grant.contains("control_exposure_ready=false"));
    assert!(readiness_before_grant.contains("control_exposure_status=blocked_pending_permission"));
    assert!(readiness_before_grant.contains("ready=false"));
    assert!(
        readiness_before_grant
            .contains("blockers=control_exposure_status_blocked_pending_permission")
    );
    assert!(
        readiness_before_grant.contains("next_action=request_or_grant_control_exposure_permission")
    );
    assert!(readiness_before_grant.contains("provider_cli_executed=false"));
    assert!(readiness_before_grant.contains("tunnel_opened=false"));
    assert!(readiness_before_grant.contains("runtime_process_started=false"));
    assert!(readiness_before_grant.contains("state_mutated=false"));

    let approval = run_cli(vec![
        "connectivity".to_string(),
        "request-approval".to_string(),
        "--exposure".to_string(),
        exposure_id.clone(),
        "--approval".to_string(),
        "approval-private-control".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("request connectivity approval");
    assert!(approval.contains("connectivity_exposure_approval_requested=true"));
    assert!(approval.contains("approval=approval-private-control"));
    assert!(approval.contains("permission_scope=network:connect:private_tunnel"));

    let decided = run_cli(vec![
        "permission".to_string(),
        "decide".to_string(),
        "--approval".to_string(),
        "approval-private-control".to_string(),
        "--decision".to_string(),
        "allow_once".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("allow connectivity approval");
    assert!(decided.contains("permission_approval_decided=true"));
    assert!(decided.contains("decision=allow_once"));

    let activated = run_cli(vec![
        "connectivity".to_string(),
        "activate-exposure".to_string(),
        "--exposure".to_string(),
        exposure_id,
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("activate exposure");
    assert!(activated.contains("connectivity_exposure_activated=true"));
    assert!(activated.contains("status=active"));
    assert!(activated.contains("grant=grant-approval-"));

    let readiness_after_grant = run_cli(vec![
        "runtime".to_string(),
        "target".to_string(),
        "readiness".to_string(),
        "--target".to_string(),
        "remote-target-1".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("runtime target readiness after grant");
    assert!(readiness_after_grant.contains("runtime_target_control_readiness_found=true"));
    assert!(readiness_after_grant.contains("control_exposure_ready=true"));
    assert!(readiness_after_grant.contains("control_exposure_status=active"));
    assert!(readiness_after_grant.contains("control_exposure_scope=private"));
    assert!(
        readiness_after_grant
            .contains("control_exposure_permission_scope=network:connect:private_tunnel")
    );
    assert!(readiness_after_grant.contains("control_exposure_reachable=true"));
    assert!(readiness_after_grant.contains("ready=true"));
    assert!(readiness_after_grant.contains("blockers=none"));
    assert!(readiness_after_grant.contains("next_action=use_runtime_target_for_remote_control"));
    let latest_readiness = run_cli(vec![
        "runtime".to_string(),
        "target".to_string(),
        "readiness".to_string(),
        "--latest".to_string(),
        "--runner".to_string(),
        "remote-process".to_string(),
        "--status".to_string(),
        "available".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("latest runtime target readiness after grant");
    assert!(latest_readiness.contains("runtime_target_control_readiness_found=true"));
    assert!(latest_readiness.contains("runtime_target_selector=latest"));
    assert!(latest_readiness.contains("runtime_target_filter_runner=remote-process"));
    assert!(latest_readiness.contains("runtime_target_filter_status=available"));
    assert!(latest_readiness.contains("runtime_target=remote-target-1"));
    assert!(latest_readiness.contains("ready=true"));
    assert!(latest_readiness.contains("provider_cli_executed=false"));
    assert!(latest_readiness.contains("tunnel_opened=false"));
    assert!(latest_readiness.contains("runtime_process_started=false"));
    assert!(latest_readiness.contains("state_mutated=false"));
    let latest_readiness_missing = run_cli(vec![
        "runtime".to_string(),
        "target".to_string(),
        "readiness".to_string(),
        "--latest".to_string(),
        "--runner".to_string(),
        "container".to_string(),
        "--status".to_string(),
        "available".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect_err("missing latest runtime target readiness");
    assert!(
        latest_readiness_missing
            .contains("no recorded runtime targets matching runner=container status=available")
    );

    let before_voice_readiness_sequence = SqliteStateStore::open(&state_root)
        .expect("state")
        .last_sequence()
        .expect("sequence before voice readiness");
    let voice_readiness = run_cli(vec![
        "voice".to_string(),
        "submit".to_string(),
        "--transcript".to_string(),
        "Is runtime target remote target 1 ready for remote control?".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("voice runtime target readiness");
    assert!(voice_readiness.contains("voice_plan=runtime_target_readiness"));
    assert!(voice_readiness.contains("read_scope=project_runtime_target_control_readiness"));
    assert!(voice_readiness.contains("spoken_runtime_target=remote-target-1"));
    assert!(voice_readiness.contains("spoken_target_ready=true"));
    assert!(voice_readiness.contains("spoken_control_exposure_ready=true"));
    assert!(voice_readiness.contains("spoken_runtime_target_ready_for_control=true"));
    assert!(voice_readiness.contains("spoken_blockers=none"));
    assert!(voice_readiness.contains("spoken_next_action=use_runtime_target_for_remote_control"));
    assert!(voice_readiness.contains("mutation_applied=false"));
    assert!(voice_readiness.contains("raw_transcript_retained=false"));
    assert!(
        !voice_readiness.contains("Is runtime target remote target 1 ready for remote control")
    );
    assert_eq!(
        SqliteStateStore::open(&state_root)
            .expect("state")
            .last_sequence()
            .expect("sequence after voice readiness"),
        before_voice_readiness_sequence
    );

    let dashboard = run_cli(vec![
        "dashboard".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("dashboard");
    assert!(dashboard.contains("connectivity_exposures=1"));
    assert!(dashboard.contains("exposure_status=active"));
    assert!(dashboard.contains("grant=grant-approval-"));
    assert!(dashboard.contains("runtime_target=remote-target-1"));
    assert!(dashboard.contains("control_exposure_status=active"));
    assert!(dashboard.contains("control_exposure_reachable=true"));
    assert!(dashboard.contains("ready=true"));
    assert!(dashboard.contains("next_action=use_runtime_target_for_remote_control"));

    let readiness_evidence_dir = temp_root("cli-runtime-target-readiness-evidence");
    let readiness_evidence = run_cli(vec![
        "runtime".to_string(),
        "target".to_string(),
        "readiness-evidence".to_string(),
        "--target".to_string(),
        "remote-target-1".to_string(),
        "--out".to_string(),
        readiness_evidence_dir.display().to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("runtime target readiness evidence");
    assert!(readiness_evidence.contains("runtime_target_readiness_evidence_exported=true"));
    assert!(readiness_evidence.contains("runtime_target=remote-target-1"));
    assert!(readiness_evidence.contains("ready=true"));
    assert!(readiness_evidence.contains("runtime_target_selector=exact"));
    assert!(
        readiness_evidence
            .contains("evidence_id=evidence-artifact-runtime-target-readiness-evidence-")
    );
    assert!(readiness_evidence.contains("provider_cli_executed=false"));
    assert!(readiness_evidence.contains("tunnel_opened=false"));
    assert!(readiness_evidence.contains("runtime_process_started=false"));
    let readiness_evidence_path = output_value(&readiness_evidence, "path");
    let readiness_markdown =
        fs::read_to_string(&readiness_evidence_path).expect("read readiness evidence");
    assert!(readiness_markdown.starts_with("<!-- capo:runtime-target-readiness-evidence -->"));
    assert!(readiness_markdown.contains("## Runtime Target Control Readiness"));
    assert!(readiness_markdown.contains("- Runtime target: `remote-target-1`"));
    assert!(readiness_markdown.contains("- Ready for control: `true`"));
    assert!(readiness_markdown.contains("- Blockers: `none`"));
    assert!(readiness_markdown.contains("does not launch runtimes"));

    let latest_readiness_evidence = run_cli(vec![
        "runtime".to_string(),
        "target".to_string(),
        "readiness-evidence".to_string(),
        "--latest".to_string(),
        "--runner".to_string(),
        "remote-process".to_string(),
        "--status".to_string(),
        "available".to_string(),
        "--out".to_string(),
        readiness_evidence_dir.display().to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("latest runtime target readiness evidence");
    assert!(latest_readiness_evidence.contains("runtime_target_readiness_evidence_exported=true"));
    assert!(latest_readiness_evidence.contains("runtime_target_selector=latest"));
    assert!(latest_readiness_evidence.contains("runtime_target_filter_runner=remote-process"));
    assert!(latest_readiness_evidence.contains("runtime_target_filter_status=available"));
    assert!(latest_readiness_evidence.contains("runtime_target=remote-target-1"));
    assert!(latest_readiness_evidence.contains("ready=true"));
    assert!(latest_readiness_evidence.contains("provider_cli_executed=false"));
    assert!(latest_readiness_evidence.contains("tunnel_opened=false"));
    assert!(latest_readiness_evidence.contains("runtime_process_started=false"));
    let missing_latest_readiness_evidence = run_cli(vec![
        "runtime".to_string(),
        "target".to_string(),
        "readiness-evidence".to_string(),
        "--latest".to_string(),
        "--runner".to_string(),
        "container".to_string(),
        "--status".to_string(),
        "available".to_string(),
        "--out".to_string(),
        readiness_evidence_dir.display().to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect_err("missing latest runtime target readiness evidence");
    assert!(
        missing_latest_readiness_evidence
            .contains("no recorded runtime targets matching runner=container status=available")
    );

    let exact_status = run_cli(vec![
        "connectivity".to_string(),
        "exposure-status".to_string(),
        "--exposure".to_string(),
        output_value(&activated, "exposure"),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("exact exposure status");
    assert!(exact_status.contains("connectivity_exposure_status=true"));
    assert!(exact_status.contains("status=active"));
    assert!(exact_status.contains("owner=runtime_target:remote-target-1"));

    let latest_status = run_cli(vec![
        "connectivity".to_string(),
        "exposure-status".to_string(),
        "--latest".to_string(),
        "--owner-kind".to_string(),
        "runtime_target".to_string(),
        "--owner-id".to_string(),
        "remote-target-1".to_string(),
        "--channel".to_string(),
        "control".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("latest exposure status");
    assert!(latest_status.contains("connectivity_exposure_status=true"));
    assert!(latest_status.contains("status=active"));
    assert_eq!(
        output_value(&latest_status, "exposure"),
        output_value(&activated, "exposure")
    );
    let before_voice_sequence = SqliteStateStore::open(&state_root)
        .expect("state")
        .last_sequence()
        .expect("sequence before voice");
    let voice_latest = run_cli(vec![
        "voice".to_string(),
        "submit".to_string(),
        "--transcript".to_string(),
        "What is the latest connectivity exposure status for runtime target remote-target-1?"
            .to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("voice latest connectivity exposure status");
    assert!(voice_latest.contains("voice_plan=connectivity_status"));
    assert!(voice_latest.contains("mutation_applied=false"));
    assert!(voice_latest.contains("raw_transcript_retained=false"));
    assert!(voice_latest.contains("read_scope=project_latest_connectivity_exposure"));
    assert!(voice_latest.contains("spoken_connectivity_exposure="));
    assert!(voice_latest.contains("spoken_owner=runtime_target:remote-target-1"));
    assert!(voice_latest.contains("spoken_channel=control"));
    assert!(voice_latest.contains("spoken_exposure_status=active"));
    assert!(voice_latest.contains("spoken_permission_scope=network:connect:private_tunnel"));
    assert!(!voice_latest.contains("What is the latest connectivity exposure status"));
    assert_eq!(
        SqliteStateStore::open(&state_root)
            .expect("state")
            .last_sequence()
            .expect("sequence after voice"),
        before_voice_sequence
    );

    let revoked = run_cli(vec![
        "connectivity".to_string(),
        "revoke-exposure".to_string(),
        "--exposure".to_string(),
        output_value(&activated, "exposure"),
        "--reason".to_string(),
        "operator closed private control surface".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("revoke exposure");
    assert!(revoked.contains("connectivity_exposure_revoked=true"));
    assert!(revoked.contains("status=revoked"));
    assert!(revoked.contains("health=disabled"));
    assert!(revoked.contains("reachable=false"));
    assert!(revoked.contains("revoked_at=unix:"));

    let readiness_after_revoke = run_cli(vec![
        "runtime".to_string(),
        "target".to_string(),
        "readiness".to_string(),
        "--target".to_string(),
        "remote-target-1".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("runtime target readiness after revoke");
    assert!(readiness_after_revoke.contains("control_exposure_status=revoked"));
    assert!(readiness_after_revoke.contains("control_exposure_reachable=false"));
    assert!(readiness_after_revoke.contains("ready=false"));
    assert!(readiness_after_revoke.contains("blockers=control_exposure_status_revoked"));
    assert!(readiness_after_revoke.contains("next_action=repair_or_replace_control_exposure"));

    let dashboard = run_cli(vec![
        "dashboard".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("dashboard after revoke");
    assert!(dashboard.contains("connectivity_exposures=1"));
    assert!(dashboard.contains("exposure_status=revoked"));
    assert!(dashboard.contains("health=disabled"));
    assert!(dashboard.contains("reachable=false"));
    assert!(dashboard.contains("revoked_at=unix:"));
    assert!(dashboard.contains("control_exposure_status=revoked"));
    assert!(dashboard.contains("control_exposure_reachable=false"));
    assert!(dashboard.contains("ready=false"));
    assert!(dashboard.contains("next_action=repair_or_replace_control_exposure"));

    let latest_status_after_revoke = run_cli(vec![
        "connectivity".to_string(),
        "exposure-status".to_string(),
        "--latest".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("latest exposure status after revoke");
    assert!(latest_status_after_revoke.contains("status=revoked"));

    let evidence_dir = temp_root("cli-connectivity-exposure-evidence");
    let evidence = run_cli(vec![
        "connectivity".to_string(),
        "exposure-evidence".to_string(),
        "--exposure".to_string(),
        output_value(&revoked, "exposure"),
        "--out".to_string(),
        evidence_dir.display().to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("export connectivity exposure evidence");
    assert!(evidence.contains("connectivity_exposure_evidence_exported=true"));
    assert!(evidence.contains("evidence_id=evidence-artifact-connectivity-exposure-evidence-"));
    let evidence_path = output_value(&evidence, "path");
    let markdown = fs::read_to_string(&evidence_path).expect("read connectivity evidence");
    assert!(markdown.starts_with("<!-- capo:connectivity-exposure-evidence -->"));
    assert!(markdown.contains("## Exposure"));
    assert!(markdown.contains("- Status: `revoked`"));
    assert!(markdown.contains("- Health: `disabled`"));
    assert!(markdown.contains("- Reachable: `false`"));
    assert!(markdown.contains("does not open tunnels"));

    let latest_evidence = run_cli(vec![
        "connectivity".to_string(),
        "exposure-evidence".to_string(),
        "--latest".to_string(),
        "--owner-kind".to_string(),
        "runtime_target".to_string(),
        "--owner-id".to_string(),
        "remote-target-1".to_string(),
        "--channel".to_string(),
        "control".to_string(),
        "--out".to_string(),
        evidence_dir.display().to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("export latest connectivity exposure evidence");
    assert!(latest_evidence.contains("connectivity_exposure_evidence_exported=true"));
    assert_eq!(
        output_value(&latest_evidence, "exposure"),
        output_value(&revoked, "exposure")
    );
    let latest_evidence_path = output_value(&latest_evidence, "path");
    let latest_markdown =
        fs::read_to_string(&latest_evidence_path).expect("read latest connectivity evidence");
    assert!(latest_markdown.starts_with("<!-- capo:connectivity-exposure-evidence -->"));
    assert!(latest_markdown.contains("- Status: `revoked`"));
    assert!(latest_markdown.contains("- Owner: `runtime_target:remote-target-1`"));

    let dashboard = run_cli(vec![
        "dashboard".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("dashboard after exposure evidence");
    assert!(dashboard.contains("project_evidence=2"));
    assert!(dashboard.contains("kind=runtime_target_readiness_evidence"));
    assert!(dashboard.contains("kind=connectivity_exposure_evidence"));
}

#[test]
fn adapter_fixture_replay_cli_exports_evidence_without_raw_provider_text() {
    let state_root = temp_root("cli-adapter-replay-state");
    let evidence_dir = temp_root("cli-adapter-replay-evidence");
    let fixture = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../capo-adapters/fixtures/codex-exec.jsonl"
    ));

    let output = run_cli(vec![
        "adapter".to_string(),
        "replay-fixture".to_string(),
        "--adapter".to_string(),
        "codex".to_string(),
        "--fixture".to_string(),
        fixture.display().to_string(),
        "--agent".to_string(),
        "replay-codex".to_string(),
        "--goal".to_string(),
        "Replay Codex fixture through Capo".to_string(),
        "--out".to_string(),
        evidence_dir.display().to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("adapter replay fixture");

    assert!(output.contains("adapter_replayed=true"));
    assert!(output.contains("adapter=codex_exec"));
    assert!(output.contains("session_id=session-replay-codex"));
    assert!(output.contains("tool_events=2"));
    assert!(output.contains("summary_events=1"));
    assert!(output.contains("completed_turns=1"));
    assert!(output.contains("evidence_exported=true"));
    assert!(!output.contains("Codex fixture response."));
    assert!(!output.contains("cargo test"));

    let evidence_path = evidence_dir.join("session-replay-codex.md");
    let evidence = fs::read_to_string(&evidence_path).expect("read replay evidence");
    assert!(evidence.contains("adapter_replay:codex_exec"));
    assert!(evidence.contains("adapter_native:codex_exec"));
    assert!(evidence.contains("## Tool Observations"));
    assert!(evidence.contains("source=`adapter_event:codex_exec`"));
    assert!(evidence.contains("instrumentation=`observed_only`"));
    assert!(evidence.contains("content_hash="));
    assert!(!evidence.contains("Codex fixture response."));
    assert!(!evidence.contains("cargo test"));
    let dashboard = run_cli(vec![
        "dashboard".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("dashboard after adapter replay");
    assert!(dashboard.contains("tool_observations=1"));
    assert!(dashboard.contains("source=adapter_event:codex_exec"));
    assert_text_absent_in_tree(&state_root, "Codex fixture response.");
    assert_text_absent_in_tree(&state_root, "cargo test");
    assert_text_absent_in_tree(&evidence_dir, "Codex fixture response.");
    assert_text_absent_in_tree(&evidence_dir, "cargo test");
}

#[test]
fn voice_status_reads_shared_query_without_mutating_or_retaining_transcript() {
    let state_root = temp_root("cli-voice-status");
    seed_running_agent(&state_root, "fake-codex", "Inspect the project");
    let state = SqliteStateStore::open(&state_root).expect("state");
    let before_sequence = state.last_sequence().expect("before sequence");

    let output = run_cli(vec![
        "voice".to_string(),
        "submit".to_string(),
        "--transcript".to_string(),
        "What is fake-codex doing?".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("voice status");

    assert!(output.contains("voice_plan=agent_status"));
    assert!(output.contains("origin=voice"));
    assert!(output.contains("mutation_applied=false"));
    assert!(output.contains("raw_transcript_retained=false"));
    assert!(output.contains("memory_ingestion=none"));
    assert!(output.contains("read_scope=agent"));
    assert!(output.contains("spoken_agent=fake-codex agent_status=running"));
    assert!(output.contains("current_goal=Inspect the project"));
    assert!(!output.contains("What is fake-codex doing?"));
    assert_eq!(
        state.last_sequence().expect("after sequence"),
        before_sequence
    );
}

#[test]
fn voice_recent_work_reads_project_and_agent_work_without_mutating() {
    let state_root = temp_root("cli-voice-recent-work");
    seed_running_agent(&state_root, "fake-codex", "Inspect the project");
    seed_running_agent(&state_root, "fake-reviewer", "Review the summary");
    let state = SqliteStateStore::open(&state_root).expect("state");
    state
        .append_event(
            NewEvent {
                event_id: "event-voice-recent-tool-activity".to_string(),
                kind: EventKind::ToolObservationRecorded,
                actor: "test".to_string(),
                project_id: Some(project_id()),
                task_id: Some(TaskId::new("task-inspect-the-project")),
                agent_id: Some(AgentId::new("agent-fake-codex")),
                session_id: Some(SessionId::new("session-fake-codex")),
                run_id: Some(RunId::new("run-fake-codex")),
                turn_id: Some("turn-voice-recent-work".to_string()),
                item_id: None,
                payload_json: "{}".to_string(),
                idempotency_key: None,
                redaction_state: RedactionState::Safe,
            },
            &[
                ProjectionRecord::ToolCall(ToolCallProjection {
                    tool_call_id: ToolCallId::new("tool-voice-recent-work"),
                    session_id: SessionId::new("session-fake-codex"),
                    turn_id: Some("turn-voice-recent-work".to_string()),
                    tool_name: "capo.session_summary".to_string(),
                    tool_origin: "capo".to_string(),
                    status: "completed".to_string(),
                    input_artifact_id: None,
                    output_artifact_id: Some("artifact-voice-tool-output".to_string()),
                    updated_sequence: 0,
                }),
                ProjectionRecord::ToolObservation(ToolObservationProjection {
                    tool_observation_id: "tool-observation-voice-recent-work".to_string(),
                    session_id: SessionId::new("session-fake-codex"),
                    tool_call_id: Some(ToolCallId::new("tool-voice-recent-work")),
                    source: "adapter_event:codex_exec".to_string(),
                    external_tool_ref: Some("provider-tool-voice".to_string()),
                    tool_name: "exec_command".to_string(),
                    observed_status: "completed".to_string(),
                    instrumentation_level: "observed_only".to_string(),
                    confidence: "high".to_string(),
                    raw_event_hash: "hash-voice-tool-observation".to_string(),
                    artifact_id: Some("artifact-voice-observed-tool".to_string()),
                    updated_sequence: 0,
                }),
            ],
        )
        .expect("append tool activity");
    let before_sequence = state.last_sequence().expect("before sequence");

    let project_output = run_cli(vec![
        "voice".to_string(),
        "submit".to_string(),
        "--transcript".to_string(),
        "What have my agents done?".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("voice project recent work");

    assert!(project_output.contains("voice_plan=recent_work"));
    assert!(project_output.contains("mutation_applied=false"));
    assert!(project_output.contains("raw_transcript_retained=false"));
    assert!(project_output.contains("read_scope=project_recent_work"));
    assert!(project_output.contains("spoken_agents=2"));
    assert!(project_output.contains("spoken_active_sessions=2"));
    assert!(project_output.contains("spoken_agent=fake-codex agent_status=running"));
    assert!(project_output.contains("spoken_agent=fake-reviewer agent_status=running"));
    assert!(project_output.contains("tool_observations=1"));
    assert!(project_output.contains("spoken_tool_call=tool-voice-recent-work"));
    assert!(
        project_output.contains(
            "spoken_tool_observation=tool-observation-voice-recent-work tool=exec_command"
        )
    );
    assert!(project_output.contains("instrumentation=observed_only confidence=high"));
    assert!(project_output.contains("latest_summary=Fake adapter processed goal for fake-codex"));
    assert!(!project_output.contains("What have my agents done?"));

    let agent_output = run_cli(vec![
        "voice".to_string(),
        "submit".to_string(),
        "--transcript".to_string(),
        "What has fake-codex done?".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("voice agent recent work");

    assert!(agent_output.contains("voice_plan=recent_work"));
    assert!(agent_output.contains("mutation_applied=false"));
    assert!(agent_output.contains("raw_transcript_retained=false"));
    assert!(agent_output.contains("read_scope=agent"));
    assert!(agent_output.contains("spoken_agent=fake-codex agent_status=running"));
    assert!(agent_output.contains("current_goal=Inspect the project"));
    assert!(agent_output.contains("latest_summary=Fake adapter processed goal for fake-codex"));
    assert!(agent_output.contains("tool_observations=1"));
    assert!(agent_output.contains("spoken_tool_call=tool-voice-recent-work"));
    assert!(agent_output.contains("spoken_tool_observation=tool-observation-voice-recent-work"));
    assert!(!agent_output.contains("What has fake-codex done?"));
    let project_tool_output = run_cli(vec![
        "voice".to_string(),
        "submit".to_string(),
        "--transcript".to_string(),
        "What tools have my agents used?".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("voice project tool activity");

    assert!(project_tool_output.contains("voice_plan=tool_activity"));
    assert!(project_tool_output.contains("mutation_applied=false"));
    assert!(project_tool_output.contains("raw_transcript_retained=false"));
    assert!(project_tool_output.contains("read_scope=project_tool_activity"));
    assert!(project_tool_output.contains("spoken_tool_activity_agents=2"));
    assert!(project_tool_output.contains("spoken_tool_calls=3"));
    assert!(project_tool_output.contains("spoken_tool_observations=1"));
    assert!(project_tool_output.contains("spoken_tool_activity_agent=fake-codex"));
    assert!(project_tool_output.contains("spoken_tool_call=tool-voice-recent-work"));
    assert!(
        project_tool_output.contains("spoken_tool_observation=tool-observation-voice-recent-work")
    );
    assert!(!project_tool_output.contains("What tools have my agents used?"));

    let agent_tool_output = run_cli(vec![
        "voice".to_string(),
        "submit".to_string(),
        "--transcript".to_string(),
        "What tools has fake-codex used?".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("voice agent tool activity");

    assert!(agent_tool_output.contains("voice_plan=tool_activity"));
    assert!(agent_tool_output.contains("mutation_applied=false"));
    assert!(agent_tool_output.contains("raw_transcript_retained=false"));
    assert!(agent_tool_output.contains("read_scope=agent_tool_activity"));
    assert!(agent_tool_output.contains("spoken_tool_activity_agents=1"));
    assert!(agent_tool_output.contains("spoken_tool_calls=2"));
    assert!(agent_tool_output.contains("spoken_tool_observations=1"));
    assert!(agent_tool_output.contains("spoken_tool_activity_agent=fake-codex"));
    assert!(agent_tool_output.contains("spoken_tool_call=tool-voice-recent-work"));
    assert!(
        agent_tool_output.contains("spoken_tool_observation=tool-observation-voice-recent-work")
    );
    assert!(!agent_tool_output.contains("What tools has fake-codex used?"));
    assert_eq!(
        state.last_sequence().expect("after sequence"),
        before_sequence
    );
}

#[test]
fn voice_next_work_reads_workpad_queue_without_mutating() {
    let state_root = temp_root("cli-voice-next-work");
    let state = SqliteStateStore::open(&state_root).expect("state");
    state
        .append_event(
            NewEvent {
                event_id: "event-cli-voice-next-work-pending".to_string(),
                kind: EventKind::WorkpadIndexed,
                actor: "test".to_string(),
                project_id: Some(project_id()),
                task_id: None,
                agent_id: None,
                session_id: None,
                run_id: None,
                turn_id: None,
                item_id: Some("workpads:features:voice.md#v7".to_string()),
                payload_json: "{}".to_string(),
                idempotency_key: None,
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::WorkpadTask(WorkpadTaskProjection {
                workpad_task_id: "workpads:features:voice.md#v7".to_string(),
                project_id: project_id(),
                path: "workpads/features/voice.md".to_string(),
                source_anchor: "v7".to_string(),
                title: "Next Work Conversation".to_string(),
                observed_status: "pending".to_string(),
                capo_execution_status: "observed_only".to_string(),
                observed_unix: 1,
                updated_sequence: 0,
            })],
        )
        .expect("append pending workpad task");
    state
        .append_event(
            NewEvent {
                event_id: "event-cli-voice-next-work-imported".to_string(),
                kind: EventKind::WorkpadIndexed,
                actor: "test".to_string(),
                project_id: Some(project_id()),
                task_id: None,
                agent_id: None,
                session_id: None,
                run_id: None,
                turn_id: None,
                item_id: Some("workpads:features:tasks.md#f1".to_string()),
                payload_json: "{}".to_string(),
                idempotency_key: None,
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::WorkpadTask(WorkpadTaskProjection {
                workpad_task_id: "workpads:features:tasks.md#f1".to_string(),
                project_id: project_id(),
                path: "workpads/features/tasks.md".to_string(),
                source_anchor: "f1".to_string(),
                title: "Real Local Agent Connector Proof".to_string(),
                observed_status: "in_progress".to_string(),
                capo_execution_status: "imported".to_string(),
                observed_unix: 1,
                updated_sequence: 0,
            })],
        )
        .expect("append imported workpad task");
    let before_sequence = state.last_sequence().expect("before sequence");

    let output = run_cli(vec![
        "voice".to_string(),
        "submit".to_string(),
        "--transcript".to_string(),
        "What should we do next?".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("voice next work");

    assert!(output.contains("voice_plan=next_work"));
    assert!(output.contains("mutation_applied=false"));
    assert!(output.contains("raw_transcript_retained=false"));
    assert!(output.contains("read_scope=project_next_work"));
    assert!(output.contains("spoken_workpad_tasks=2"));
    assert!(output.contains("spoken_next_work_candidates=1"));
    assert!(output.contains("spoken_next_workpad_task=workpads:features:voice.md#v7"));
    assert!(output.contains("default_task_id=task-workpad-workpads-features-voice-md-v7"));
    assert!(output.contains("title=Next Work Conversation"));
    assert!(output.contains("observed_status=pending"));
    assert!(output.contains("capo_execution_status=observed_only"));
    assert!(!output.contains("What should we do next?"));
    assert_eq!(
        state.last_sequence().expect("after sequence"),
        before_sequence
    );
}

#[test]
fn voice_confirmed_start_next_work_imports_and_dispatches_after_approval() {
    let state_root = temp_root("cli-voice-start-next-work");
    run_cli(vec![
        "agent".to_string(),
        "register".to_string(),
        "--name".to_string(),
        "fake-codex".to_string(),
        "--adapter".to_string(),
        "fake".to_string(),
        "--runtime".to_string(),
        "fake".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("register fake-codex");
    let state = SqliteStateStore::open(&state_root).expect("state");
    state
        .append_event(
            NewEvent {
                event_id: "event-cli-voice-start-next-work-index".to_string(),
                kind: EventKind::WorkpadIndexed,
                actor: "test".to_string(),
                project_id: Some(project_id()),
                task_id: None,
                agent_id: None,
                session_id: None,
                run_id: None,
                turn_id: None,
                item_id: Some("workpads:features:voice.md#v8".to_string()),
                payload_json: "{}".to_string(),
                idempotency_key: None,
                redaction_state: RedactionState::Safe,
            },
            &[
                ProjectionRecord::WorkpadFile(WorkpadFileProjection {
                    path: "workpads/features/voice.md".to_string(),
                    project_id: project_id(),
                    content_hash: "hash-voice-workpad".to_string(),
                    headings: "V8 - Start Next Work Conversation".to_string(),
                    objective: Some("Voice workpad".to_string()),
                    observed_unix: 1,
                    updated_sequence: 0,
                }),
                ProjectionRecord::WorkpadTask(WorkpadTaskProjection {
                    workpad_task_id: "workpads:features:voice.md#v8".to_string(),
                    project_id: project_id(),
                    path: "workpads/features/voice.md".to_string(),
                    source_anchor: "v8".to_string(),
                    title: "Start Next Work Conversation".to_string(),
                    observed_status: "pending".to_string(),
                    capo_execution_status: "observed_only".to_string(),
                    observed_unix: 1,
                    updated_sequence: 0,
                }),
            ],
        )
        .expect("append workpad file and task");
    let before_unconfirmed = state.last_sequence().expect("before unconfirmed");

    let unconfirmed = run_cli(vec![
        "voice".to_string(),
        "submit".to_string(),
        "--transcript".to_string(),
        "Start next task with fake-codex.".to_string(),
        "--voice-session".to_string(),
        "voice-session-start-next".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("voice start next requires confirmation");

    assert!(unconfirmed.contains("voice_plan=start_next_work"));
    assert!(unconfirmed.contains("confirmation_required=true"));
    assert!(unconfirmed.contains("mutation_applied=false"));
    assert!(unconfirmed.contains("permission_status=pending"));
    assert!(!unconfirmed.contains("workpad_next_started=true"));
    assert!(!unconfirmed.contains("Start next task with fake-codex"));
    assert_eq!(
        state.last_sequence().expect("after unconfirmed"),
        before_unconfirmed + 1
    );

    let confirmed = run_cli(vec![
        "voice".to_string(),
        "submit".to_string(),
        "--transcript".to_string(),
        "Start next task with fake-codex.".to_string(),
        "--voice-session".to_string(),
        "voice-session-start-next".to_string(),
        "--confirm".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("voice confirmed start next");

    assert!(confirmed.contains("voice_plan=start_next_work"));
    assert!(confirmed.contains("confirmation_required=true"));
    assert!(confirmed.contains("mutation_applied=true"));
    assert!(confirmed.contains("permission_status=decided"));
    assert!(confirmed.contains("permission_decision=allow_once"));
    assert!(confirmed.contains("controlled_agent=fake-codex"));
    assert!(confirmed.contains("workpad_next_started=true"));
    assert!(confirmed.contains("workpad_task_id=workpads:features:voice.md#v8"));
    assert!(confirmed.contains("task_id=task-workpad-workpads-features-voice-md-v8"));
    assert!(confirmed.contains("session_id=session-fake-codex"));
    assert!(confirmed.contains("spoken_next_workpad_task=none"));
    assert!(!confirmed.contains("Start next task with fake-codex"));

    let imported = state
        .workpad_task(&project_id(), "workpads:features:voice.md#v8")
        .expect("workpad task query")
        .expect("imported workpad task");
    assert_eq!(imported.capo_execution_status, "imported");
    let session = state
        .session(&SessionId::new("session-fake-codex"))
        .expect("session query")
        .expect("started session");
    assert_eq!(
        session.task_id.as_ref().map(ToString::to_string).as_deref(),
        Some("task-workpad-workpads-features-voice-md-v8")
    );
    let grants = state.capability_grants().expect("capability grants");
    assert!(grants.iter().any(|grant| {
        grant.decision_source == "user_visible_voice_confirmation"
            && grant.subject_json.contains("voice-session-start-next")
            && grant.subject_json.contains("start_next_work")
    }));
}

#[test]
fn voice_review_needs_reads_review_and_outcome_state_without_mutating() {
    let state_root = temp_root("cli-voice-review-needs");
    let evidence_dir = temp_root("cli-voice-review-needs-evidence");
    seed_running_agent(&state_root, "fake-codex", "Inspect the project");

    run_cli(vec![
        "review".to_string(),
        "record".to_string(),
        "--session".to_string(),
        "session-fake-codex".to_string(),
        "--reviewer".to_string(),
        "focused-review".to_string(),
        "--kind".to_string(),
        "blocker".to_string(),
        "--summary".to_string(),
        "Dashboard must expose review blockers.".to_string(),
        "--out".to_string(),
        evidence_dir.display().to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("record review blocker");

    SqliteStateStore::open(&state_root)
        .expect("state")
        .append_event(
            NewEvent {
                event_id: "event-cli-voice-review-needs-task-outcome".to_string(),
                kind: EventKind::TaskOutcomeReportGenerated,
                actor: "test".to_string(),
                project_id: Some(project_id()),
                task_id: Some(TaskId::new("task-inspect-the-project")),
                agent_id: Some(AgentId::new("agent-fake-codex")),
                session_id: Some(SessionId::new("session-fake-codex")),
                run_id: Some(RunId::new("run-fake-codex")),
                turn_id: None,
                item_id: Some("task-outcome-report-voice-review-needs".to_string()),
                payload_json: "{}".to_string(),
                idempotency_key: None,
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::TaskOutcomeReport(
                capo_state::TaskOutcomeReportProjection {
                    task_outcome_report_id: "task-outcome-report-voice-review-needs".to_string(),
                    project_id: project_id(),
                    task_id: TaskId::new("task-inspect-the-project"),
                    session_id: SessionId::new("session-fake-codex"),
                    run_id: RunId::new("run-fake-codex"),
                    outcome_status: "completed".to_string(),
                    started_sequence: 1,
                    completed_sequence: 12,
                    duration_sequence_span: 11,
                    action_count: 4,
                    tool_call_count: 1,
                    evidence_count: 1,
                    memory_packet_count: 1,
                    confidence: Some(78),
                    blocker: Some("Needs review follow-up".to_string()),
                    review_outcome: "reviewed_with_findings".to_string(),
                    report_artifact_id: Some(
                        "artifact-task-outcome-voice-review-needs".to_string(),
                    ),
                    updated_sequence: 0,
                },
            )],
        )
        .expect("append task outcome report");
    let state = SqliteStateStore::open(&state_root).expect("state");
    let before_sequence = state.last_sequence().expect("before sequence");

    let output = run_cli(vec![
        "voice".to_string(),
        "submit".to_string(),
        "--transcript".to_string(),
        "What needs review?".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("voice review needs");

    assert!(output.contains("voice_plan=review_needs"));
    assert!(output.contains("mutation_applied=false"));
    assert!(output.contains("raw_transcript_retained=false"));
    assert!(output.contains("read_scope=project_review_needs"));
    assert!(output.contains("spoken_review_findings=1"));
    assert!(output.contains("spoken_open_review_findings=1"));
    assert!(output.contains("spoken_review_blockers=1"));
    assert!(output.contains("spoken_task_outcome_reports=1"));
    assert!(output.contains("spoken_reports_with_findings=1"));
    assert!(output.contains("spoken_latest_review_outcome=reviewed_with_findings"));
    assert!(output.contains("kind=blocker severity=high status=open"));
    assert!(output.contains("summary=Dashboard must expose review blockers."));
    assert!(output.contains("spoken_task_outcome_report=task-outcome-report-voice-review-needs"));
    assert!(!output.contains("What needs review?"));
    assert_eq!(
        state.last_sequence().expect("after sequence"),
        before_sequence
    );
}

#[test]
fn voice_dogfood_readiness_reads_shared_query_without_mutating() {
    let state_root = temp_root("cli-voice-dogfood-readiness");
    let state = SqliteStateStore::open(&state_root).expect("state");
    let before_sequence = state.last_sequence().expect("before sequence");

    let output = run_cli(vec![
        "voice".to_string(),
        "submit".to_string(),
        "--transcript".to_string(),
        "Are we ready to dogfood?".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("voice dogfood readiness");

    assert!(output.contains("voice_plan=dogfood_readiness"));
    assert!(output.contains("mutation_applied=false"));
    assert!(output.contains("raw_transcript_retained=false"));
    assert!(output.contains("read_scope=project_dogfood_readiness"));
    assert!(output.contains("spoken_dogfood_ready=false"));
    assert!(output.contains("spoken_dogfood_status=blocked_pending_dogfood_prerequisites"));
    assert!(output.contains("spoken_runtime_target_ready=false"));
    assert!(output.contains("spoken_blockers=real_agent_connector_not_proven,available_runtime_target_missing,workpad_index_missing,dispatch_chain_missing"));
    assert!(output.contains("spoken_next_actions=record_clean_codex_smoke_evidence,register_available_runtime_target,run_workpad_index,record_or_replay_workpad_dispatch_plan"));
    assert!(output.contains("spoken_connector_evidence_refs=none"));
    assert!(output.contains("spoken_runtime_target_refs=none"));
    assert!(output.contains("spoken_workpad_task_refs=none"));
    assert!(output.contains("spoken_dispatch_chain_refs=none"));
    assert!(output.contains("spoken_project_evidence_refs=none"));
    assert!(!output.contains("Are we ready to dogfood?"));
    assert_eq!(
        state.last_sequence().expect("after sequence"),
        before_sequence
    );
}

#[test]
fn voice_dispatch_status_reads_shared_query_without_mutating() {
    let state_root = temp_root("cli-voice-dispatch-status");
    let workspace = temp_root("cli-voice-dispatch-status-workspace");
    let artifacts = temp_root("cli-voice-dispatch-status-artifacts");
    run_cli(vec![
        "adapter".to_string(),
        "plan-launch".to_string(),
        "--adapter".to_string(),
        "codex".to_string(),
        "--agent".to_string(),
        "codex-worker".to_string(),
        "--goal".to_string(),
        "Do not render this voice dispatch prompt.".to_string(),
        "--workspace".to_string(),
        workspace.display().to_string(),
        "--artifacts".to_string(),
        artifacts.display().to_string(),
        "--record".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("record dispatch plan");
    let state = SqliteStateStore::open(&state_root).expect("state");
    let dispatch_plan_id = state
        .adapter_dispatch_plans(&project_id())
        .expect("dispatch plans")[0]
        .dispatch_plan_id
        .clone();
    let before_sequence = state.last_sequence().expect("before sequence");

    let output = run_cli(vec![
        "voice".to_string(),
        "submit".to_string(),
        "--transcript".to_string(),
        format!("What is dispatch status for {dispatch_plan_id}?"),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("voice dispatch status");

    assert!(output.contains("voice_plan=dispatch_status"));
    assert!(output.contains("mutation_applied=false"));
    assert!(output.contains("raw_transcript_retained=false"));
    assert!(output.contains("read_scope=project_dispatch_status"));
    assert!(output.contains(&format!("spoken_dispatch_plan={dispatch_plan_id}")));
    assert!(output.contains("spoken_adapter=codex_exec"));
    assert!(output.contains("spoken_provider_kind=codex_subscription"));
    assert!(output.contains("spoken_credential_scope=user_local_subscription"));
    assert!(output.contains("spoken_provider_cli_executed=false"));
    assert!(output.contains("spoken_dogfood_gate=blocked_pending_real_smoke"));
    assert!(output.contains("spoken_latest_gate_status=missing"));
    assert!(output.contains("spoken_latest_dispatch_replay=none"));
    assert!(output.contains("spoken_latest_execution_status=missing"));
    assert!(output.contains("spoken_next_action=record_clean_real_smoke_evidence"));
    assert!(!output.contains("What is dispatch status"));
    assert!(!output.contains("Do not render this voice dispatch prompt"));
    assert_eq!(
        state.last_sequence().expect("after sequence"),
        before_sequence
    );

    let latest_output = run_cli(vec![
        "voice".to_string(),
        "submit".to_string(),
        "--transcript".to_string(),
        "What is the latest dispatch status?".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("voice latest dispatch status");

    assert!(latest_output.contains("voice_plan=dispatch_status"));
    assert!(latest_output.contains("mutation_applied=false"));
    assert!(latest_output.contains("read_scope=project_latest_dispatch_status"));
    assert!(latest_output.contains(&format!("spoken_dispatch_plan={dispatch_plan_id}")));
    assert!(latest_output.contains("spoken_agent=codex-worker"));
    assert!(latest_output.contains("spoken_next_action=record_clean_real_smoke_evidence"));
    assert!(!latest_output.contains("What is the latest dispatch status"));
    assert!(!latest_output.contains("Do not render this voice dispatch prompt"));

    let latest_agent_output = run_cli(vec![
        "voice".to_string(),
        "submit".to_string(),
        "--transcript".to_string(),
        "What is the latest dispatch status for codex-worker?".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("voice latest dispatch status for agent");

    assert!(latest_agent_output.contains("read_scope=project_latest_dispatch_status"));
    assert!(latest_agent_output.contains(&format!("spoken_dispatch_plan={dispatch_plan_id}")));
    assert!(latest_agent_output.contains("spoken_agent=codex-worker"));
    assert!(!latest_agent_output.contains("What is the latest dispatch status"));
    assert_eq!(
        state.last_sequence().expect("after latest sequence"),
        before_sequence
    );
}

#[test]
fn voice_adapter_smoke_status_reads_shared_query_without_mutating() {
    let state_root = temp_root("cli-voice-adapter-smoke-status");
    let record = run_cli(vec![
        "adapter".to_string(),
        "smoke-report".to_string(),
        "record".to_string(),
        "--adapter".to_string(),
        "codex".to_string(),
        "--status".to_string(),
        "skipped".to_string(),
        "--credential-scan".to_string(),
        "not_run".to_string(),
        "--reason".to_string(),
        "voice status check no provider execution".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("record smoke report");
    let smoke_report_id = output_value(&record, "smoke_report_id");
    let state = SqliteStateStore::open(&state_root).expect("state");
    let before_sequence = state.last_sequence().expect("before sequence");

    let output = run_cli(vec![
        "voice".to_string(),
        "submit".to_string(),
        "--transcript".to_string(),
        format!("What is smoke report status for {smoke_report_id}?"),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("voice smoke report status");

    assert!(output.contains("voice_plan=adapter_smoke_status"));
    assert!(output.contains("mutation_applied=false"));
    assert!(output.contains("raw_transcript_retained=false"));
    assert!(output.contains("read_scope=project_adapter_smoke_report_status"));
    assert!(output.contains(&format!("spoken_smoke_report={smoke_report_id}")));
    assert!(output.contains("spoken_adapter=codex_exec"));
    assert!(output.contains("spoken_smoke_status=skipped"));
    assert!(output.contains("spoken_credential_scan_status=not_run"));
    assert!(output.contains("spoken_marker_found=false"));
    assert!(
        output.contains("spoken_dogfood_readiness_effect=real_subscription_smoke_not_recorded")
    );
    assert!(output.contains("spoken_provider_cli_executed=false"));
    assert!(output.contains("spoken_credential_material_rendered=false"));
    assert!(output.contains("spoken_state_mutated=false"));
    assert!(!output.contains("What is smoke report status"));
    assert_eq!(
        state.last_sequence().expect("after sequence"),
        before_sequence
    );

    let latest = run_cli(vec![
        "voice".to_string(),
        "submit".to_string(),
        "--transcript".to_string(),
        "What is the latest smoke report status for Codex?".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("voice latest smoke report status");

    assert!(latest.contains("voice_plan=adapter_smoke_status"));
    assert!(latest.contains("read_scope=project_latest_adapter_smoke_report"));
    assert!(latest.contains(&format!("spoken_smoke_report={smoke_report_id}")));
    assert!(latest.contains("spoken_adapter=codex_exec"));
    assert!(!latest.contains("What is the latest smoke report status"));
    assert_eq!(
        state.last_sequence().expect("after latest sequence"),
        before_sequence
    );
}

#[test]
fn voice_redirect_routes_through_controller_and_preserves_transient_transcript() {
    let state_root = temp_root("cli-voice-redirect");
    seed_running_agent(&state_root, "fake-reviewer", "Review the status summary");

    let output = run_cli(vec![
        "voice".to_string(),
        "submit".to_string(),
        "--transcript".to_string(),
        "Steer fake-reviewer to focus only on dogfood blockers.".to_string(),
        "--voice-session".to_string(),
        "voice-session-test".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("voice redirect");

    assert!(output.contains("voice_plan=redirect_session"));
    assert!(output.contains("command_id=cmd-voice-redirect-voice-session-test"));
    assert!(output.contains("mutation_applied=true"));
    assert!(output.contains("read_scope=session_for_agent"));
    assert!(output.contains("spoken_agent=fake-reviewer agent_status=running"));
    assert!(output.contains("current_goal=focus only on dogfood blockers"));
    assert!(!output.contains("Steer fake-reviewer"));

    let status = run_cli(vec![
        "session".to_string(),
        "status".to_string(),
        "--agent".to_string(),
        "fake-reviewer".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("status after voice redirect");
    assert!(status.contains("current_goal=focus only on dogfood blockers"));
    assert!(status.contains("kind=session.redirected"));
}

#[test]
fn voice_unknown_does_not_mutate_and_unconfirmed_stop_only_queues_approval() {
    let state_root = temp_root("cli-voice-no-mutation");
    seed_running_agent(&state_root, "fake-codex", "Inspect the project");
    let state = SqliteStateStore::open(&state_root).expect("state");
    let before_unknown = state.last_sequence().expect("before unknown");

    let unknown = run_cli(vec![
        "voice".to_string(),
        "submit".to_string(),
        "--transcript".to_string(),
        "Maybe later, never mind".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("voice unknown");
    assert!(unknown.contains("voice_plan=unknown"));
    assert!(unknown.contains("command_id=none"));
    assert!(unknown.contains("mutation_applied=false"));
    assert_eq!(
        state.last_sequence().expect("after unknown"),
        before_unknown
    );

    let before_stop = state.last_sequence().expect("before stop");
    let stop = run_cli(vec![
        "voice".to_string(),
        "submit".to_string(),
        "--transcript".to_string(),
        "Stop fake-codex because smoke is done".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("voice stop needs confirmation");
    assert!(stop.contains("voice_plan=stop_session"));
    assert!(stop.contains("confirmation_required=true"));
    assert!(stop.contains("mutation_applied=false"));
    assert!(stop.contains("permission_status=pending"));
    assert!(stop.contains("permission_scope=[\"voice:approve:privileged\"]"));
    assert_eq!(state.last_sequence().expect("after stop"), before_stop + 1);
    let approvals = state
        .permission_approvals(&project_id())
        .expect("voice approvals");
    let approval = approvals
        .iter()
        .find(|approval| approval.requested_by == "voice:local-user")
        .expect("voice approval");
    assert_eq!(approval.status, "pending");
    assert_eq!(
        approval
            .session_id
            .as_ref()
            .map(ToString::to_string)
            .as_deref(),
        Some("session-fake-codex")
    );
    assert_eq!(
        approval.reason,
        "visible confirmation required for stop_session"
    );
    assert!(!approval.reason.contains("smoke is done"));

    let status = run_cli(vec![
        "session".to_string(),
        "status".to_string(),
        "--agent".to_string(),
        "fake-codex".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("status after unconfirmed stop");
    assert!(status.contains("status=active"));
    assert!(status.contains("run_status=running"));
}

#[test]
fn voice_confirmed_stop_audits_decision_before_controller_mutation() {
    let state_root = temp_root("cli-voice-confirmed-stop");
    seed_running_agent(&state_root, "fake-codex", "Inspect the project");

    let stop = run_cli(vec![
        "voice".to_string(),
        "submit".to_string(),
        "--transcript".to_string(),
        "Stop fake-codex because smoke is done".to_string(),
        "--voice-session".to_string(),
        "voice-session-confirmed".to_string(),
        "--confirm".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("voice confirmed stop");

    assert!(stop.contains("voice_plan=stop_session"));
    assert!(stop.contains("confirmation_required=true"));
    assert!(stop.contains("mutation_applied=true"));
    assert!(stop.contains("permission_status=decided"));
    assert!(stop.contains("permission_decision=allow_once"));
    assert!(stop.contains("spoken_agent=fake-codex agent_status=available"));
    assert!(stop.contains("session_status=completed"));
    assert!(!stop.contains("Stop fake-codex"));

    let state = SqliteStateStore::open(&state_root).expect("state");
    let approvals = state
        .permission_approvals(&project_id())
        .expect("voice approvals");
    assert_eq!(approvals.len(), 1);
    let approval = &approvals[0];
    assert_eq!(approval.status, "decided");
    assert_eq!(approval.decision.as_deref(), Some("allow_once"));
    assert_eq!(approval.capability_profile_id, "voice-control");
    assert_eq!(approval.scope_json, "[\"voice:approve:privileged\"]");
    assert_eq!(approval.requested_by, "voice:local-user");
    assert!(approval.capability_grant_id.is_some());
    let grants = state.capability_grants().expect("capability grants");
    assert!(grants.iter().any(|grant| {
        grant.capability_grant_id == approval.capability_grant_id.clone().unwrap()
            && grant.decision_source == "user_visible_voice_confirmation"
            && grant.persistence == "once"
            && grant.subject_json.contains("voice-session-confirmed")
    }));
    let events = state
        .recent_events_for_session(&SessionId::new("session-fake-codex"), 10)
        .expect("recent events");
    assert!(
        events
            .iter()
            .any(|event| event.kind == "permission.approval_queued")
    );
    assert!(
        events
            .iter()
            .any(|event| event.kind == "permission.decided")
    );
    assert!(events.iter().any(|event| event.kind == "session.stopped"));
    for event in events {
        assert!(!event.payload_json.contains("Stop fake-codex"));
        assert!(!event.payload_json.contains("smoke is done"));
    }
}

#[test]
fn voice_confirmed_interrupt_audits_decision_before_controller_mutation() {
    let state_root = temp_root("cli-voice-confirmed-interrupt");
    seed_running_agent(&state_root, "fake-codex", "Inspect the project");

    let interrupted = run_cli(vec![
        "voice".to_string(),
        "submit".to_string(),
        "--transcript".to_string(),
        "Interrupt fake-codex because output is stale".to_string(),
        "--confirm".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("voice confirmed interrupt");

    assert!(interrupted.contains("voice_plan=interrupt_session"));
    assert!(interrupted.contains("mutation_applied=true"));
    assert!(interrupted.contains("permission_status=decided"));
    assert!(interrupted.contains("permission_decision=allow_once"));
    assert!(interrupted.contains("controlled_session=session-fake-codex"));
    assert!(interrupted.contains("session_status=canceled"));
    assert!(interrupted.contains("run_status=stopping"));
    assert!(!interrupted.contains("Interrupt fake-codex"));

    let state = SqliteStateStore::open(&state_root).expect("state");
    let approvals = state
        .permission_approvals(&project_id())
        .expect("voice approvals");
    assert_eq!(approvals.len(), 1);
    assert_eq!(
        approvals[0].reason,
        "visible confirmation required for interrupt_session"
    );
    let events = state
        .recent_events_for_session(&SessionId::new("session-fake-codex"), 10)
        .expect("recent events");
    assert!(
        events
            .iter()
            .any(|event| event.kind == "permission.approval_queued")
    );
    assert!(
        events
            .iter()
            .any(|event| event.kind == "permission.decided")
    );
    assert!(
        events
            .iter()
            .any(|event| event.kind == "session.interrupted")
    );
    for event in events {
        assert!(!event.payload_json.contains("Interrupt fake-codex"));
        assert!(!event.payload_json.contains("output is stale"));
    }
}

#[test]
fn voice_reviewed_redacted_summary_ingests_memory_without_raw_transcript() {
    let state_root = temp_root("cli-voice-memory");
    seed_running_agent(&state_root, "fake-codex", "Inspect the project");
    let raw_phrase = "raw-private-voice-token";
    let redacted_summary = "User asked to stop fake-codex after a redacted reason.";

    let output = run_cli(vec![
        "voice".to_string(),
        "submit".to_string(),
        "--transcript".to_string(),
        format!("Stop fake-codex because {raw_phrase}"),
        "--redacted-summary".to_string(),
        redacted_summary.to_string(),
        "--reviewed-summary".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("voice summary memory");

    assert!(output.contains("voice_plan=stop_session"));
    assert!(output.contains("memory_ingestion=reviewed_redacted_summary_only"));
    assert!(output.contains("memory_review_state=reviewed"));
    assert!(output.contains("memory_redaction_state=redacted"));
    assert!(!output.contains(raw_phrase));

    let state = SqliteStateStore::open(&state_root).expect("state");
    let records = state
        .memory_records_for_project(&project_id())
        .expect("memory records");
    assert_eq!(records.len(), 1);
    let record = &records[0];
    assert_eq!(record.review_state, "reviewed");
    assert_eq!(record.redaction_state, "redacted");
    assert_eq!(record.record_kind, "summary");
    assert_eq!(record.body, redacted_summary);
    assert!(!record.body.contains(raw_phrase));
    let sources = state
        .memory_sources_for_record(&record.memory_record_id)
        .expect("memory sources");
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].source_kind, "event");
    assert_eq!(
        sources[0].source_anchor.as_deref(),
        Some("voice:redacted-summary")
    );
    assert!(sources[0].source_content_hash.is_some());
    let eligible = state
        .packet_eligible_memory_records(&project_id())
        .expect("packet eligible records");
    assert_eq!(eligible.len(), 1);
    assert_eq!(eligible[0].memory_record_id, record.memory_record_id);
    assert_text_absent_in_tree(&state_root, raw_phrase);
}

#[test]
fn voice_redacted_summary_requires_review_before_memory_ingestion() {
    let state_root = temp_root("cli-voice-memory-review-required");
    seed_running_agent(&state_root, "fake-codex", "Inspect the project");

    let error = run_cli(vec![
        "voice".to_string(),
        "submit".to_string(),
        "--transcript".to_string(),
        "What is fake-codex doing?".to_string(),
        "--redacted-summary".to_string(),
        "User asked for a redacted status summary.".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .unwrap_err();

    assert!(error.contains("--redacted-summary requires --reviewed-summary"));
    let state = SqliteStateStore::open(&state_root).expect("state");
    let records = state
        .memory_records_for_project(&project_id())
        .expect("memory records");
    assert!(records.is_empty());
}

#[test]
fn evidence_export_handles_completed_runs_and_refuses_foreign_files() {
    let state_root = temp_root("cli-completed-state");
    let evidence_dir = temp_root("cli-completed-evidence");

    run_cli(vec![
        "agent".to_string(),
        "register".to_string(),
        "--name".to_string(),
        "fake-reviewer".to_string(),
        "--adapter".to_string(),
        "fake".to_string(),
        "--runtime".to_string(),
        "fake".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .unwrap();
    run_cli(vec![
        "task".to_string(),
        "send".to_string(),
        "--agent".to_string(),
        "fake-reviewer".to_string(),
        "--goal".to_string(),
        "Review the status summary for blockers".to_string(),
        "--scenario".to_string(),
        "summary-review".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .unwrap();
    run_cli(vec![
        "session".to_string(),
        "stop".to_string(),
        "--agent".to_string(),
        "fake-reviewer".to_string(),
        "--reason".to_string(),
        "completed smoke".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .unwrap();

    run_cli(vec![
        "evidence".to_string(),
        "export".to_string(),
        "--session".to_string(),
        "session-fake-reviewer".to_string(),
        "--out".to_string(),
        evidence_dir.display().to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .unwrap();
    let evidence_path = evidence_dir.join("session-fake-reviewer.md");
    let exported = fs::read_to_string(&evidence_path).expect("read completed evidence");
    assert!(exported.contains("- Session status: `completed`"));
    assert!(exported.contains("- Run status: `exited`"));
    assert!(exported.contains("session.stopped"));

    fs::write(&evidence_path, "# user-authored workpad\n").expect("replace with foreign file");
    let error = run_cli(vec![
        "evidence".to_string(),
        "export".to_string(),
        "--session".to_string(),
        "session-fake-reviewer".to_string(),
        "--out".to_string(),
        evidence_dir.display().to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .unwrap_err();
    assert!(error.contains("refusing to overwrite non-Capo evidence file"));
}

#[test]
fn prototype_e2e_smoke_tracks_two_agents_recovers_and_exports_evidence() {
    let state_root = temp_root("cli-e2e-state");
    let evidence_dir = temp_root("cli-e2e-evidence");
    let mut transcript = String::new();

    let mut run = |args: Vec<&str>| {
        let mut owned = args
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<String>>();
        owned.push("--state".to_string());
        owned.push(state_root.display().to_string());
        let output = run_cli(owned).expect("run smoke command");
        transcript.push_str(&output);
        output
    };

    let initialized = run(vec!["init"]);
    assert!(initialized.contains("initialized=true"));

    let codex = run(vec![
        "agent",
        "spawn",
        "--name",
        "fake-codex",
        "--adapter",
        "fake",
        "--runtime",
        "fake",
    ]);
    assert!(codex.contains("agent_spawned=true"));
    let reviewer = run(vec![
        "agent",
        "register",
        "--name",
        "fake-reviewer",
        "--adapter",
        "fake",
        "--runtime",
        "fake",
    ]);
    assert!(reviewer.contains("agent_registered=true"));

    let codex_send = run(vec![
        "task",
        "send",
        "--agent",
        "fake-codex",
        "--goal",
        "Inspect the project and write a short status summary",
        "--scenario",
        "tool-memory",
    ]);
    assert!(codex_send.contains("session_id=session-fake-codex"));
    SqliteStateStore::open(&state_root)
        .expect("state for observed tool")
        .append_event(
            NewEvent {
                event_id: "event-observed-provider-tool".to_string(),
                kind: EventKind::ToolObservationRecorded,
                actor: "test".to_string(),
                project_id: Some(ProjectId::new(DEFAULT_PROJECT_ID)),
                task_id: Some(TaskId::new(
                    "task-inspect-the-project-and-write-a-short-status-summary",
                )),
                agent_id: Some(AgentId::new("agent-fake-codex")),
                session_id: Some(SessionId::new("session-fake-codex")),
                run_id: Some(RunId::new("run-fake-codex")),
                turn_id: Some("turn-fake-codex".to_string()),
                item_id: None,
                payload_json: "{}".to_string(),
                idempotency_key: None,
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::ToolObservation(
                ToolObservationProjection {
                    tool_observation_id: "tool-observation-fake-codex".to_string(),
                    session_id: SessionId::new("session-fake-codex"),
                    tool_call_id: Some(ToolCallId::new("tool-fake-codex")),
                    source: "adapter_event".to_string(),
                    external_tool_ref: Some("provider-tool-fake-codex".to_string()),
                    tool_name: "provider.native_search".to_string(),
                    observed_status: "completed".to_string(),
                    instrumentation_level: "observed_only".to_string(),
                    confidence: "high".to_string(),
                    raw_event_hash: "hash-observed-provider-tool".to_string(),
                    artifact_id: Some("artifact-observed-provider-tool".to_string()),
                    updated_sequence: 0,
                },
            )],
        )
        .expect("append observed tool");
    let reviewer_send = run(vec![
        "task",
        "send",
        "--agent",
        "fake-reviewer",
        "--goal",
        "Review the status summary for blockers",
        "--scenario",
        "summary-review",
    ]);
    assert!(reviewer_send.contains("session_id=session-fake-reviewer"));

    let agents = run(vec!["agent", "list"]);
    assert!(agents.contains("active_agents=2"));
    assert!(agents.contains("agent=fake-codex status=running"));
    assert!(agents.contains("agent=fake-reviewer status=running"));
    let dashboard = run(vec!["dashboard"]);
    assert!(dashboard.contains("view=dashboard"));
    assert!(dashboard.contains("agents=2"));
    assert!(dashboard.contains("active_sessions=2"));
    assert!(dashboard.contains("tool_activity_agents=2"));
    assert!(dashboard.contains("tool_activity_active_sessions=2"));
    assert!(dashboard.contains("tool_calls=2"));
    assert!(dashboard.contains("tool_observations=1"));
    assert!(dashboard.contains("agent=fake-codex agent_status=running"));
    assert!(dashboard.contains("session=session-fake-codex session_status=active"));
    assert!(dashboard.contains("goal=Inspect the project"));
    assert!(dashboard.contains("blocker=none"));
    assert!(dashboard.contains("evidence_refs=evidence-fake-codex"));
    assert!(dashboard.contains("tool_calls=1"));
    assert!(dashboard.contains("tool_call=tool-fake-codex tool=capo.session_summary"));
    assert!(dashboard.contains("tool_observations=1"));
    assert!(
        dashboard
            .contains("tool_observation=tool-observation-fake-codex tool=provider.native_search")
    );
    assert!(dashboard.contains("instrumentation=observed_only confidence=high"));
    assert!(dashboard.contains("memory_packet_refs=1"));
    assert!(dashboard.contains("memory_packet=packet-fake-codex purpose=turn_context"));
    assert!(dashboard.contains("kind=tool.result_delivered"));
    assert!(dashboard.contains("agent=fake-reviewer agent_status=running"));
    let session_dashboard = run(vec!["dashboard", "--session", "session-fake-codex"]);
    assert!(session_dashboard.contains("agents=1"));
    assert!(session_dashboard.contains("agent=fake-codex agent_status=running"));
    assert!(!session_dashboard.contains("agent=fake-reviewer agent_status=running"));
    let running_dashboard = run(vec!["dashboard", "--status", "running"]);
    assert!(running_dashboard.contains("agents=2"));
    let missing_dashboard = run(vec!["dashboard", "--status", "waiting_for_input"]);
    assert!(missing_dashboard.contains("agents=0"));
    assert!(missing_dashboard.contains("active_sessions=0"));
    let other_project_dashboard = run(vec!["dashboard", "--project", "project-other"]);
    assert!(other_project_dashboard.contains("agents=0"));
    assert!(other_project_dashboard.contains("active_sessions=0"));

    let codex_status = run(vec!["session", "status", "--agent", "fake-codex"]);
    assert!(codex_status.contains("current_goal=Inspect the project"));
    assert!(codex_status.contains("tool_calls=1"));
    assert!(codex_status.contains("tool_call=tool-fake-codex tool=capo.session_summary"));
    assert!(codex_status.contains("tool_observations=1"));
    assert!(
        codex_status
            .contains("tool_observation=tool-observation-fake-codex tool=provider.native_search")
    );
    assert!(codex_status.contains("instrumentation=observed_only confidence=high"));
    assert!(codex_status.contains("kind=permission.decided"));
    assert!(codex_status.contains("kind=capability.grant_used"));
    assert!(codex_status.contains("kind=tool.result_delivered"));
    assert!(codex_status.contains("kind=memory.packet_built"));
    assert!(codex_status.contains("evidence_refs=evidence-fake-codex"));

    let redirect = run(vec![
        "session",
        "redirect",
        "--agent",
        "fake-reviewer",
        "--goal",
        "Focus only on dogfood blockers",
    ]);
    assert!(redirect.contains("redirected=true"));
    assert!(redirect.contains("current_goal=Focus only on dogfood blockers"));
    let reviewer_status = run(vec!["session", "status", "--agent", "fake-reviewer"]);
    assert!(reviewer_status.contains("current_goal=Focus only on dogfood blockers"));
    assert!(reviewer_status.contains("kind=session.redirected"));
    let second_redirect = run(vec![
        "session",
        "redirect",
        "--agent",
        "fake-reviewer",
        "--goal",
        "Focus only on evidence export blockers",
    ]);
    assert!(second_redirect.contains("redirected=true"));
    assert!(second_redirect.contains("current_goal=Focus only on evidence export blockers"));
    let redirected_dashboard = run(vec!["dashboard"]);
    assert!(redirected_dashboard.contains("Focus only on evidence export blockers"));
    assert!(redirected_dashboard.contains("kind=session.redirected"));

    let interrupted = run(vec![
        "session",
        "interrupt",
        "--agent",
        "fake-codex",
        "--reason",
        "smoke interrupt",
    ]);
    assert!(interrupted.contains("status=canceled"));
    let stopped = run(vec![
        "session",
        "stop",
        "--agent",
        "fake-reviewer",
        "--reason",
        "smoke stop",
    ]);
    assert!(stopped.contains("status=completed"));

    let recovered = run(vec!["recover"]);
    assert!(recovered.contains("recovered=true"));
    assert!(recovered.contains("recovered_run_count=1"));
    let recovered_again = run(vec!["recover"]);
    assert!(recovered_again.contains("recovered=true"));
    assert!(recovered_again.contains("recovered_run_count=0"));

    let export_codex = run_cli(vec![
        "evidence".to_string(),
        "export".to_string(),
        "--session".to_string(),
        "session-fake-codex".to_string(),
        "--out".to_string(),
        evidence_dir.display().to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("export codex evidence");
    transcript.push_str(&export_codex);
    let export_reviewer = run_cli(vec![
        "evidence".to_string(),
        "export".to_string(),
        "--session".to_string(),
        "session-fake-reviewer".to_string(),
        "--out".to_string(),
        evidence_dir.display().to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("export reviewer evidence");
    transcript.push_str(&export_reviewer);

    let codex_evidence =
        fs::read_to_string(evidence_dir.join("session-fake-codex.md")).expect("codex evidence");
    let reviewer_evidence = fs::read_to_string(evidence_dir.join("session-fake-reviewer.md"))
        .expect("reviewer evidence");
    assert!(codex_evidence.contains("- Session status: `canceled`"));
    assert!(codex_evidence.contains("- Run status: `exited_unknown`"));
    assert!(codex_evidence.contains("tool.result_delivered"));
    assert!(codex_evidence.contains("## Tool Observations"));
    assert!(codex_evidence.contains("`tool-observation-fake-codex` name=`provider.native_search`"));
    assert!(codex_evidence.contains("instrumentation=`observed_only` confidence=`high`"));
    assert!(codex_evidence.contains("artifact=`artifact-memory-packet-packet-fake-codex`"));
    assert!(reviewer_evidence.contains("- Session status: `completed`"));
    assert!(reviewer_evidence.contains("Focus only on evidence export blockers"));
    assert!(reviewer_evidence.contains("session.redirected"));
    assert!(reviewer_evidence.contains("session.stopped"));

    let reopened = SqliteStateStore::open(&state_root).expect("restart state");
    assert_eq!(
        reopened
            .session(&SessionId::new("session-fake-codex"))
            .expect("read codex session")
            .expect("codex session")
            .status,
        "canceled"
    );
    assert_eq!(
        reopened
            .run_for_session(&SessionId::new("session-fake-codex"))
            .expect("read codex run")
            .expect("codex run")
            .status,
        "exited_unknown"
    );
    assert_eq!(
        reopened
            .session(&SessionId::new("session-fake-reviewer"))
            .expect("read reviewer session")
            .expect("reviewer session")
            .status,
        "completed"
    );
    assert_eq!(reopened.agents().expect("read agents").len(), 2);
    assert_eq!(
        reopened
            .evidence_for_session(&SessionId::new("session-fake-codex"))
            .expect("codex evidence")
            .len(),
        1
    );
    assert_eq!(
        reopened
            .evidence_for_session(&SessionId::new("session-fake-reviewer"))
            .expect("reviewer evidence")
            .len(),
        1
    );
    assert_eq!(
        reopened
            .tool_calls_for_session(&SessionId::new("session-fake-codex"))
            .expect("codex tool calls")
            .len(),
        1
    );
    assert_eq!(
        reopened
            .tool_observations_for_session(&SessionId::new("session-fake-codex"))
            .expect("codex tool observations")
            .len(),
        1
    );
    assert_eq!(
        reopened
            .memory_packets_for_session(&SessionId::new("session-fake-codex"))
            .expect("codex memory packets")
            .len(),
        1
    );
    assert_eq!(
        reopened
            .task(&capo_core::TaskId::new(
                "task-inspect-the-project-and-write-a-short-status-summary"
            ))
            .expect("read codex task")
            .expect("codex task")
            .evidence_id
            .as_ref()
            .map(ToString::to_string),
        Some("evidence-fake-codex".to_string())
    );
    assert_eq!(
        reopened
            .task(&capo_core::TaskId::new(
                "task-review-the-status-summary-for-blockers"
            ))
            .expect("read reviewer task")
            .expect("reviewer task")
            .evidence_id
            .as_ref()
            .map(ToString::to_string),
        Some("evidence-fake-reviewer".to_string())
    );

    assert_no_sensitive_markers(&transcript);
    assert_no_sensitive_markers(&codex_evidence);
    assert_no_sensitive_markers(&reviewer_evidence);
    assert_no_sensitive_markers_in_tree(&state_root);
    assert_no_sensitive_markers_in_tree(&evidence_dir);
}

fn assert_no_sensitive_markers(contents: &str) {
    for marker in [
        "OPENAI_API_KEY",
        "ANTHROPIC_API_KEY",
        "Authorization:",
        "Cookie:",
        "Set-Cookie:",
        "session_token",
        "access_token",
        "refresh_token",
        "oauth",
        "api_key",
        "sk-proj-",
        "sk-ant-",
        "sk-live-",
        "sk_test_",
    ] {
        assert!(
            !contents
                .to_ascii_lowercase()
                .contains(&marker.to_ascii_lowercase()),
            "sensitive marker leaked: {marker}"
        );
    }
}

fn assert_no_sensitive_markers_in_tree(root: &Path) {
    if !root.exists() {
        return;
    }
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        if path.is_dir() {
            for entry in fs::read_dir(&path).expect("read scan dir") {
                stack.push(entry.expect("scan dir entry").path());
            }
        } else if path.is_file() {
            let bytes = fs::read(&path).expect("read scan file");
            let contents = String::from_utf8_lossy(&bytes);
            assert_no_sensitive_markers(&contents);
        }
    }
}

fn assert_text_absent_in_tree(root: &Path, needle: &str) {
    if !root.exists() {
        return;
    }
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        if path.is_dir() {
            for entry in fs::read_dir(&path).expect("read scan dir") {
                stack.push(entry.expect("scan dir entry").path());
            }
        } else if path.is_file() {
            let bytes = fs::read(&path).expect("read scan file");
            let contents = String::from_utf8_lossy(&bytes);
            assert!(
                !contents.contains(needle),
                "unexpected raw text in {}",
                path.display()
            );
        }
    }
}

fn seed_running_agent(state_root: &Path, agent: &str, goal: &str) {
    run_cli(vec![
        "agent".to_string(),
        "register".to_string(),
        "--name".to_string(),
        agent.to_string(),
        "--adapter".to_string(),
        "fake".to_string(),
        "--runtime".to_string(),
        "fake".to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("register agent");
    run_cli(vec![
        "task".to_string(),
        "send".to_string(),
        "--agent".to_string(),
        agent.to_string(),
        "--goal".to_string(),
        goal.to_string(),
        "--state".to_string(),
        state_root.display().to_string(),
    ])
    .expect("send task");
}

fn temp_root(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    std::env::temp_dir().join(format!("capo-{name}-{nanos}"))
}

fn output_value(output: &str, key: &str) -> String {
    let prefix = format!("{key}=");
    output
        .lines()
        .find_map(|line| line.strip_prefix(&prefix))
        .unwrap_or_else(|| panic!("missing output key: {key}"))
        .to_string()
}
