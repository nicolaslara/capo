use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use capo_core::{BoundaryKind, RunId, SessionId, ToolCallId};
use serde_json::Value;

use super::*;

#[test]
fn planned_adapters_include_fake_and_first_real_targets() {
    assert!(PLANNED_ADAPTERS.contains(&"fake"));
    assert!(PLANNED_ADAPTERS.contains(&"codex-exec"));
    assert!(PLANNED_ADAPTERS.contains(&"claude-code"));
    assert!(PLANNED_ADAPTERS.contains(&"acp"));
}

#[test]
fn fake_adapter_reports_adapter_boundary() {
    assert_eq!(
        AgentAdapterHandle::fake().binding().kind,
        BoundaryKind::AgentAdapter
    );
}

#[test]
fn fake_provider_reports_provider_boundary() {
    assert_eq!(
        ProviderConnector::fake().binding().kind,
        BoundaryKind::ProviderConnector
    );
}

#[test]
fn codex_jsonl_fixture_maps_to_normalized_events() {
    let parsed =
        CodexExecAdapter::parse_jsonl(include_str!("../fixtures/codex-exec.jsonl")).unwrap();

    assert_eq!(parsed.raw_event_count, 5);
    assert!(parsed.events.iter().any(|event| {
        event.kind == "adapter.session_started"
            && event.external_session_ref.as_deref() == Some("codex-thread-1")
    }));
    let message = parsed
        .events
        .iter()
        .find(|event| event.kind == "adapter.item_completed")
        .expect("message event");
    assert_eq!(message.external_item_ref.as_deref(), Some("codex-item-1"));
    assert_eq!(message.role.as_deref(), Some("assistant"));
    assert_eq!(
        message.timeline_confidence,
        AdapterTimelineConfidence::Stable
    );
    assert!(parsed.events.iter().any(|event| {
        event.kind == "adapter.tool_call_completed"
            && event.tool_name.as_deref() == Some("exec_command")
    }));
    assert!(parsed.events.iter().any(|event| {
        event.kind == "adapter.turn_completed"
            && event.input_tokens == Some(11)
            && event.output_tokens == Some(7)
    }));
}

#[test]
fn codex_workspace_write_fixture_maps_a_tool_result_round_trip() {
    // RTL9: the workspace-write round-trip parses into a tool-call observation
    // that carries the OBSERVED applied result (the diff/output), distinct from
    // the agent's own `item.completed` message claim.
    let parsed =
        CodexExecAdapter::parse_jsonl(include_str!("../fixtures/codex-exec-workspace-write.jsonl"))
            .unwrap();

    // The agent's reported claim is an item message, not the observed tool result.
    let claim = parsed
        .events
        .iter()
        .find(|event| event.kind == "adapter.item_completed")
        .expect("agent message claim");
    assert_eq!(claim.role.as_deref(), Some("assistant"));
    assert_eq!(
        claim.content.as_deref(),
        Some("I will add a greeting to NOTES.md.")
    );

    // The OBSERVED tool result is the `apply_patch` completion, carrying the
    // applied diff/output -- separate from the agent's message above.
    let observed_write = parsed
        .events
        .iter()
        .find(|event| {
            event.kind == "adapter.tool_call_completed"
                && event.tool_name.as_deref() == Some("apply_patch")
        })
        .expect("observed apply_patch tool result");
    assert_eq!(
        observed_write.external_item_ref.as_deref(),
        Some("codex-write-tool-1")
    );
    let observed_content = observed_write.content.as_deref().expect("observed result");
    assert!(observed_content.contains("Applied patch to NOTES.md"));

    // The observed tool result projects into a tool observation distinct from
    // the agent claim (the summary path), which is exactly the RTL9 contract.
    let observation = observed_write
        .tool_observation()
        .expect("tool observation for the observed write");
    assert_eq!(observation.tool_name, "apply_patch");
    assert_eq!(observation.observed_status, "completed");
    assert_eq!(observation.instrumentation_level, "observed_only");
}

#[test]
fn codex_live_file_change_item_maps_to_an_observed_apply_patch_tool_result() {
    // RTL13: the LIVE `codex exec --json` workspace-write stream (codex 0.134)
    // reports an applied edit as an `item.completed` whose `item.type` is
    // `file_change` (carrying the applied `changes`), NOT a `patch_apply.*` tool
    // event. The parser must still record this as an OBSERVED `apply_patch` tool
    // result distinct from the agent's `agent_message` claim, so the live
    // round-trip produces the identical normalized shape the deterministic
    // `patch_apply.*` fixture does -- the paired-assertion invariant.
    let parsed = CodexExecAdapter::parse_jsonl(include_str!(
        "../fixtures/codex-exec-workspace-write-file-change.jsonl"
    ))
    .unwrap();

    // The agent's reported claim is an `agent_message` item, not the observed
    // tool result.
    let claim = parsed
        .events
        .iter()
        .find(|event| {
            event.kind == "adapter.item_completed" && event.role.as_deref() == Some("assistant")
        })
        .expect("agent message claim");
    assert!(
        claim
            .content
            .as_deref()
            .unwrap_or_default()
            .contains("create only the requested file")
    );

    // The OBSERVED tool result is the `file_change` completion routed to
    // `apply_patch`, carrying the applied changes -- separate from the agent
    // message above.
    let observed_write = parsed
        .events
        .iter()
        .find(|event| {
            event.kind == "adapter.tool_call_completed"
                && event.tool_name.as_deref() == Some("apply_patch")
        })
        .expect("observed apply_patch tool result from a file_change item");
    assert_eq!(observed_write.external_item_ref.as_deref(), Some("item_1"));
    let observed_content = observed_write.content.as_deref().expect("observed result");
    assert!(
        observed_content.contains("CAPO_RTL13.txt"),
        "the observed result must carry the applied file change, got: {observed_content}"
    );

    // It projects into a tool observation distinct from the agent claim.
    let observation = observed_write
        .tool_observation()
        .expect("tool observation for the observed write");
    assert_eq!(observation.tool_name, "apply_patch");
    assert_eq!(observation.observed_status, "completed");
    assert_eq!(observation.instrumentation_level, "observed_only");

    // The in-progress `item.started` file_change maps to a tool_call_started for
    // the SAME tool call id (so begin/end dedup to one observation).
    let started = parsed
        .events
        .iter()
        .find(|event| {
            event.kind == "adapter.tool_call_started"
                && event.tool_name.as_deref() == Some("apply_patch")
        })
        .expect("observed apply_patch tool start from the in-progress file_change item");
    assert_eq!(started.external_item_ref.as_deref(), Some("item_1"));
}

#[test]
fn codex_exec_agent_message_text_maps_to_assistant_content() {
    let parsed = CodexExecAdapter::parse_jsonl(
        r#"{"type":"item.completed","item":{"id":"item_0","type":"agent_message","text":"CAPO_UI_LIVE_OK"}}"#,
    )
    .unwrap();

    let message = parsed
        .events
        .iter()
        .find(|event| event.kind == "adapter.item_completed")
        .expect("message event");
    assert_eq!(message.role.as_deref(), Some("assistant"));
    assert_eq!(message.content.as_deref(), Some("CAPO_UI_LIVE_OK"));
}

#[test]
fn claude_stream_json_fixture_maps_to_normalized_events() {
    let parsed =
        ClaudeCodeAdapter::parse_stream_json(include_str!("../fixtures/claude-code-stream.jsonl"))
            .unwrap();

    assert_eq!(parsed.raw_event_count, 5);
    assert!(parsed.events.iter().any(|event| {
        event.kind == "adapter.session_started"
            && event.external_session_ref.as_deref() == Some("claude-session-1")
    }));
    let message = parsed
        .events
        .iter()
        .find(|event| event.external_item_ref.as_deref() == Some("msg_1"))
        .expect("claude message");
    assert_eq!(message.content.as_deref(), Some("Claude fixture response."));
    assert_eq!(message.input_tokens, Some(13));
    assert_eq!(message.output_tokens, Some(8));
    assert!(parsed.events.iter().any(|event| {
        event.kind == "adapter.tool_call_completed"
            && event.external_item_ref.as_deref() == Some("toolu_1")
    }));
}

#[test]
fn acp_replay_fixture_maps_stable_and_heuristic_timeline_keys() {
    let parsed =
        AcpAdapter::parse_replay_jsonl(include_str!("../fixtures/acp-replay.jsonl")).unwrap();

    assert_eq!(parsed.raw_event_count, 7);
    let message = parsed
        .events
        .iter()
        .find(|event| event.kind == "adapter.item_delta")
        .expect("message delta");
    assert_eq!(
        message.timeline_confidence,
        AdapterTimelineConfidence::Heuristic
    );
    assert_eq!(message.role.as_deref(), Some("assistant"));
    let tool_events = parsed
        .events
        .iter()
        .filter(|event| event.timeline_key.as_deref() == Some("acp:acp-session-1:tool:tool-1"))
        .collect::<Vec<_>>();
    assert_eq!(tool_events.len(), 4);
    assert!(
        tool_events
            .iter()
            .all(|event| event.timeline_confidence == AdapterTimelineConfidence::Stable)
    );
}

#[test]
fn acp_duplicate_tool_updates_dedupe_by_stable_idempotency_key() {
    let parsed =
        AcpAdapter::parse_replay_jsonl(include_str!("../fixtures/acp-replay.jsonl")).unwrap();

    let before = parsed
        .events
        .iter()
        .filter(|event| event.kind == "adapter.tool_call_completed")
        .count();
    let after = parsed
        .deduped_by_idempotency()
        .iter()
        .filter(|event| event.kind == "adapter.tool_call_completed")
        .count();

    assert_eq!(before, 2);
    assert_eq!(after, 1);
}

#[test]
fn adapter_tool_observations_are_observed_only() {
    let acp = AcpAdapter::parse_replay_jsonl(include_str!("../fixtures/acp-replay.jsonl")).unwrap();
    let acp_observations = acp.tool_observations();

    assert_eq!(acp_observations.len(), 3);
    assert!(acp_observations.iter().all(|observation| {
        observation.source_adapter == "acp"
            && observation.instrumentation_level == "observed_only"
            && observation.confidence == "high"
            && observation.external_tool_ref.as_deref() == Some("tool-1")
    }));
    assert!(
        acp_observations
            .iter()
            .any(|observation| observation.observed_status == "completed")
    );

    let codex =
        CodexExecAdapter::parse_jsonl(include_str!("../fixtures/codex-exec.jsonl")).unwrap();
    let codex_observations = codex.tool_observations();
    assert!(codex_observations.iter().any(|observation| {
        observation.source_adapter == "codex_exec"
            && observation.instrumentation_level == "observed_only"
            && observation.tool_name == "exec_command"
    }));

    let claude =
        ClaudeCodeAdapter::parse_stream_json(include_str!("../fixtures/claude-code-stream.jsonl"))
            .unwrap();
    let claude_observations = claude.tool_observations();
    assert!(claude_observations.iter().any(|observation| {
        observation.source_adapter == "claude_code"
            && observation.instrumentation_level == "observed_only"
            && observation.external_tool_ref.as_deref() == Some("toolu_1")
    }));
}

#[test]
fn acp_session_setup_uses_tool_capability_plan() {
    let wrappers =
        capo_tools::RuntimeToolWrappers::new(capo_tools::RuntimeToolConfig::local_workspace(
            PathBuf::from("/tmp/capo-acp-workspace"),
            PathBuf::from("/tmp/capo-acp-artifacts"),
        ));

    let setup = AcpAdapter::session_setup_plan(
        &wrappers.list_tools(),
        &capo_tools::PermissionPolicy::static_read_only_local(),
        SessionId::new("session-acp-setup"),
    );

    assert_eq!(setup.protocol_version, 1);
    assert_eq!(setup.client_kind, "capo");
    assert_eq!(
        setup.advertised_capabilities,
        vec!["filesystem.read_text_file"]
    );
    assert!(setup.filesystem_read.advertise);
    assert!(!setup.filesystem_write.advertise);
    assert!(!setup.terminal.advertise);
    assert_eq!(setup.credential_policy, "not_inspected");
    assert_eq!(setup.mcp_server_count, 0);
    assert!(!setup.runtime_started);
    assert!(!setup.provider_cli_executed);
}

#[test]
fn acp_session_setup_fails_closed_when_backing_tool_missing() {
    let definitions =
        capo_tools::RuntimeToolWrappers::new(capo_tools::RuntimeToolConfig::local_workspace(
            PathBuf::from("/tmp/capo-acp-workspace"),
            PathBuf::from("/tmp/capo-acp-artifacts"),
        ))
        .list_tools()
        .into_iter()
        .filter(|definition| definition.tool_id != "capo.file_read")
        .collect::<Vec<_>>();

    let setup = AcpAdapter::session_setup_plan(
        &definitions,
        &capo_tools::PermissionPolicy::allow_trusted_local(),
        SessionId::new("session-acp-missing-file-read"),
    );

    assert!(!setup.filesystem_read.advertise);
    assert_eq!(setup.filesystem_read.reason, "missing_backing_wrapper_tool");
    assert!(
        !setup
            .advertised_capabilities
            .contains(&"filesystem.read_text_file".to_string())
    );
}

#[test]
fn acp_client_calls_route_only_when_capability_advertised() {
    let wrappers =
        capo_tools::RuntimeToolWrappers::new(capo_tools::RuntimeToolConfig::local_workspace(
            PathBuf::from("/tmp/capo-acp-workspace"),
            PathBuf::from("/tmp/capo-acp-artifacts"),
        ));
    let read_only_setup = AcpAdapter::session_setup_plan(
        &wrappers.list_tools(),
        &capo_tools::PermissionPolicy::static_read_only_local(),
        SessionId::new("session-acp-client-read-only"),
    );

    let read = read_only_setup
        .wrapper_request_for_client_call(acp_client_call(
            "fs/read_text_file",
            serde_json::json!({"path":"README.md"}),
        ))
        .expect("read advertised");
    assert_eq!(read.tool_id, "capo.file_read");
    assert_eq!(read.input["path"].as_str(), Some("README.md"));
    assert_eq!(read.capability_profile_id, "read-only-local");

    let write = read_only_setup.wrapper_request_for_client_call(acp_client_call(
        "fs/write_text_file",
        serde_json::json!({"path":"README.md","content":"changed"}),
    ));
    assert!(write.unwrap_err().contains("filesystem.write_text_file"));

    let terminal = read_only_setup.wrapper_request_for_client_call(acp_client_call(
        "terminal/run",
        serde_json::json!({"program":"cargo","argv":["test"],"cwd":"."}),
    ));
    assert!(terminal.unwrap_err().contains("terminal"));
}

#[test]
fn acp_terminal_call_routes_to_shell_wrapper_for_trusted_profile() {
    let wrappers =
        capo_tools::RuntimeToolWrappers::new(capo_tools::RuntimeToolConfig::local_workspace(
            PathBuf::from("/tmp/capo-acp-workspace"),
            PathBuf::from("/tmp/capo-acp-artifacts"),
        ));
    let setup = AcpAdapter::session_setup_plan(
        &wrappers.list_tools(),
        &capo_tools::PermissionPolicy::allow_trusted_local(),
        SessionId::new("session-acp-client-trusted"),
    );

    let request = setup
        .wrapper_request_for_client_call(acp_client_call_with_profile(
            "terminal/run",
            serde_json::json!({"program":"cargo","argv":["test","-p","capo-adapters"],"cwd":"."}),
            "trusted-local-dev",
        ))
        .expect("terminal advertised");

    assert_eq!(request.tool_id, "capo.shell_run");
    assert_eq!(request.input["program"].as_str(), Some("cargo"));
    assert_eq!(request.input["argv"].as_array().expect("argv").len(), 3);
}

#[test]
fn codex_launch_plan_builds_subscription_safe_runtime_request() {
    let workspace = temp_root("codex-launch-workspace");
    let artifacts = temp_root("codex-launch-artifacts");
    let plan = CodexExecAdapter::local_launch_plan(
        workspace.clone(),
        artifacts.clone(),
        "Summarize this project state.",
    );

    plan.assert_subscription_safe().unwrap();
    assert_eq!(plan.provider_kind, "codex_subscription");
    assert_eq!(plan.credential_scope, "user_local_subscription");
    assert_eq!(plan.stdout_format, "jsonl");
    assert_eq!(plan.stderr_policy, "logs_redacted");
    assert_eq!(
        plan.runtime_config().workspace_roots,
        vec![workspace.clone()]
    );
    let request = plan.runtime_request(RunId::new("run-codex-launch"));
    assert_eq!(request.program, "codex");
    assert_eq!(request.cwd, workspace);
    assert!(request.env.is_empty());
    assert!(
        request
            .argv
            .windows(2)
            .any(|args| args == ["--sandbox", "read-only"])
    );
    assert!(request.argv.iter().any(|arg| arg == "--ephemeral"));
    assert!(request.argv.iter().any(|arg| arg == "--ignore-user-config"));
    assert!(request.argv.iter().any(|arg| arg == "--ignore-rules"));
    assert!(
        request
            .argv
            .iter()
            .all(|arg| arg != "--skip-git-repo-check")
    );
    assert!(
        request
            .argv
            .windows(2)
            .any(|args| args == ["--cd", workspace.to_string_lossy().as_ref()])
    );
    assert_eq!(
        request.argv.last().map(String::as_str),
        Some("Summarize this project state.")
    );
    assert_eq!(plan.artifact_root, artifacts);
}

#[test]
fn codex_workspace_write_launch_plan_uses_workspace_write_sandbox_without_ephemeral() {
    // RTL6: the workspace-write profile can apply edits inside the confined
    // workspace. It moves Codex off `--sandbox read-only --ephemeral` to
    // `--sandbox workspace-write` (no `--ephemeral`) while staying
    // subscription-safe and confined via `--cd`.
    let workspace = temp_root("codex-write-workspace");
    let artifacts = temp_root("codex-write-artifacts");
    let plan = CodexExecAdapter::local_workspace_write_launch_plan(
        workspace.clone(),
        artifacts.clone(),
        "Apply the requested edit.",
    );

    plan.assert_subscription_safe().unwrap();
    assert_eq!(plan.provider_kind, "codex_subscription");
    assert_eq!(plan.credential_scope, "user_local_subscription");
    let request = plan.runtime_request(RunId::new("run-codex-write"));
    assert_eq!(request.program, "codex");
    assert_eq!(request.cwd, workspace);
    assert!(
        request
            .argv
            .windows(2)
            .any(|args| args == ["--sandbox", "workspace-write"]),
        "workspace-write profile must request the workspace-write sandbox"
    );
    assert!(
        request.argv.iter().all(|arg| arg != "--ephemeral"),
        "workspace-write profile must not be ephemeral so edits persist"
    );
    assert!(
        request.argv.iter().all(|arg| arg != "read-only"),
        "workspace-write profile must not be read-only"
    );
    assert!(
        request
            .argv
            .iter()
            .any(|arg| arg == "--skip-git-repo-check"),
        "the RTL6-confined workspace is a fresh non-git dir, so the workspace-write \
         profile must pass --skip-git-repo-check or `codex exec` refuses to run"
    );
    assert!(
        request
            .argv
            .windows(2)
            .any(|args| args == ["--cd", workspace.to_string_lossy().as_ref()]),
        "writes stay confined to the workspace via --cd"
    );
    assert_eq!(
        request.argv.last().map(String::as_str),
        Some("Apply the requested edit.")
    );
}

#[test]
fn claude_launch_plan_builds_subscription_safe_runtime_request() {
    let workspace = temp_root("claude-launch-workspace");
    let artifacts = temp_root("claude-launch-artifacts");
    let plan = ClaudeCodeAdapter::local_launch_plan(
        workspace.clone(),
        artifacts,
        "Summarize this project state.",
    );

    plan.assert_subscription_safe().unwrap();
    assert_eq!(plan.provider_kind, "claude_subscription");
    assert_eq!(plan.credential_scope, "user_local_subscription");
    assert_eq!(plan.stdout_format, "stream-json");
    let request = plan.runtime_request(RunId::new("run-claude-launch"));
    assert_eq!(request.program, "claude");
    assert_eq!(request.cwd, workspace);
    assert!(request.env.is_empty());
    assert!(
        request
            .argv
            .windows(2)
            .any(|args| args == ["--output-format", "stream-json"])
    );
    assert!(
        request
            .argv
            .windows(2)
            .any(|args| args == ["--permission-mode", "plan"])
    );
    assert!(
        request
            .argv
            .iter()
            .any(|arg| arg == "--no-session-persistence")
    );
    assert!(
        request
            .argv
            .iter()
            .any(|arg| arg == "--disable-slash-commands")
    );
    assert!(request.argv.windows(2).any(|args| args == ["--tools", ""]));
    assert!(
        request
            .argv
            .windows(2)
            .any(|args| args == ["--disallowedTools", "*"])
    );
    assert!(request.argv.iter().any(|arg| arg == "--strict-mcp-config"));
    assert_eq!(
        request.argv.last().map(String::as_str),
        Some("Summarize this project state.")
    );
}

#[test]
fn launch_plan_rejects_secret_like_env_or_argv_markers() {
    let workspace = temp_root("unsafe-launch-workspace");
    let artifacts = temp_root("unsafe-launch-artifacts");
    let mut plan = CodexExecAdapter::local_launch_plan(workspace, artifacts, "hello");
    plan.env_allowlist.push("OPENAI_API_KEY".to_string());
    assert!(
        plan.assert_subscription_safe()
            .unwrap_err()
            .contains("env allowlist")
    );

    plan.env_allowlist.clear();
    plan.argv.push("Authorization: bearer secret".to_string());
    assert!(
        plan.assert_subscription_safe()
            .unwrap_err()
            .contains("argv")
    );
}

#[test]
fn codex_local_smoke_plan_uses_restrictive_defaults() {
    let workspace = temp_root("codex-workspace");
    let artifacts = temp_root("codex-artifacts");
    let plan = CodexExecAdapter::local_smoke_plan(workspace.clone(), artifacts.clone());

    assert_eq!(plan.opt_in_env, "CAPO_RUN_CODEX_LOCAL_SMOKE");
    assert_eq!(plan.program, "codex");
    assert!(
        plan.argv
            .windows(2)
            .any(|args| args == ["--sandbox", "read-only"])
    );
    assert!(plan.argv.iter().any(|arg| arg == "--ephemeral"));
    assert!(plan.argv.iter().any(|arg| arg == "--ignore-user-config"));
    assert!(plan.argv.iter().any(|arg| arg == "--ignore-rules"));
    assert!(plan.argv.iter().any(|arg| arg == "--skip-git-repo-check"));
    assert!(
        plan.argv
            .windows(2)
            .any(|args| args == ["--cd", workspace.to_string_lossy().as_ref()])
    );
    assert_eq!(plan.workspace_root, workspace);
    assert_eq!(plan.artifact_root, artifacts);
    assert!(!plan.env_allowlist.iter().any(|name| name.contains("TOKEN")));
}

#[test]
fn claude_local_smoke_plan_disables_tools_and_mcp_by_default() {
    let workspace = temp_root("claude-workspace");
    let artifacts = temp_root("claude-artifacts");
    let plan = ClaudeCodeAdapter::local_smoke_plan(workspace, artifacts);

    assert_eq!(plan.opt_in_env, "CAPO_RUN_CLAUDE_LOCAL_SMOKE");
    assert_eq!(plan.program, "claude");
    assert!(
        plan.argv
            .windows(2)
            .any(|args| args == ["--output-format", "stream-json"])
    );
    assert!(
        plan.argv
            .windows(2)
            .any(|args| args == ["--permission-mode", "plan"])
    );
    assert!(
        plan.argv
            .iter()
            .any(|arg| arg == "--no-session-persistence")
    );
    assert!(
        plan.argv
            .iter()
            .any(|arg| arg == "--disable-slash-commands")
    );
    assert!(plan.argv.windows(2).any(|args| args == ["--tools", ""]));
    assert!(
        plan.argv
            .windows(2)
            .any(|args| args == ["--disallowedTools", "*"])
    );
    assert!(plan.argv.iter().any(|arg| arg == "--strict-mcp-config"));
    assert!(!plan.env_allowlist.iter().any(|name| name.contains("TOKEN")));
}

#[test]
fn local_adapter_smoke_runner_skips_without_explicit_opt_in() {
    let plan = LocalAdapterSmokePlan {
        adapter_kind: NormalizedAdapterKind::CodexExec,
        opt_in_env: "CAPO_TEST_UNSET_LOCAL_SMOKE",
        program: "/bin/echo".to_string(),
        argv: vec!["CAPO_CODEX_SMOKE_OK".to_string()],
        workspace_root: temp_root("skip-workspace"),
        artifact_root: temp_root("skip-artifacts"),
        env_allowlist: Vec::new(),
        redaction_rules: Vec::new(),
        output_limit_bytes: 1024,
        expected_output_marker: "CAPO_CODEX_SMOKE_OK",
    };

    let outcome = LocalAdapterSmokeRunner::run_if_opted_in(&plan).unwrap();

    assert!(outcome.is_none());
}

#[test]
fn local_adapter_smoke_runner_executes_through_runtime_boundary() {
    let workspace = temp_root("echo-workspace");
    let artifact_root = temp_root("echo-artifacts");
    let plan = LocalAdapterSmokePlan {
        adapter_kind: NormalizedAdapterKind::CodexExec,
        opt_in_env: "CAPO_TEST_UNSET_LOCAL_SMOKE",
        program: "/bin/echo".to_string(),
        argv: vec!["CAPO_CODEX_SMOKE_OK".to_string()],
        workspace_root: workspace,
        artifact_root,
        env_allowlist: Vec::new(),
        redaction_rules: Vec::new(),
        output_limit_bytes: 1024,
        expected_output_marker: "CAPO_CODEX_SMOKE_OK",
    };

    let outcome = LocalAdapterSmokeRunner::run(&plan).unwrap();

    assert_eq!(outcome.process.status, "exited");
    assert!(
        fs::read_to_string(&outcome.stdout.path)
            .unwrap()
            .contains("CAPO_CODEX_SMOKE_OK")
    );
    assert!(outcome.events.iter().any(|event| {
        event.kind == "runtime.output_artifact_recorded"
            && event.status == outcome.stdout.redaction_state
    }));
}

#[test]
#[ignore = "requires CAPO_RUN_CODEX_LOCAL_SMOKE=1 and local Codex login"]
fn local_codex_adapter_smoke() {
    let plan = CodexExecAdapter::local_smoke_plan(
        temp_root("real-codex-workspace"),
        temp_root("real-codex-artifacts"),
    );
    let outcome = LocalAdapterSmokeRunner::run_if_opted_in(&plan)
        .expect("codex local smoke should either skip or pass");

    assert!(
        outcome.is_some() || !plan.is_opted_in(),
        "set CAPO_RUN_CODEX_LOCAL_SMOKE=1 to execute the Codex local smoke"
    );
}

#[test]
#[ignore = "requires CAPO_RUN_CLAUDE_LOCAL_SMOKE=1 and verified restricted Claude Code args"]
fn local_claude_adapter_smoke() {
    let plan = ClaudeCodeAdapter::local_smoke_plan(
        temp_root("real-claude-workspace"),
        temp_root("real-claude-artifacts"),
    );
    let outcome = LocalAdapterSmokeRunner::run_if_opted_in(&plan)
        .expect("claude local smoke should either skip or pass");

    assert!(
        outcome.is_some() || !plan.is_opted_in(),
        "set CAPO_RUN_CLAUDE_LOCAL_SMOKE=1 after verifying restricted Claude Code args"
    );
}

#[test]
fn artifact_scanner_allows_redacted_markers_and_rejects_raw_secrets() {
    let root = temp_root("scan");
    fs::create_dir_all(&root).unwrap();
    let redacted = root.join("redacted.txt");
    let raw = root.join("raw.txt");
    let benign = root.join("benign.txt");
    let provider_key = root.join("provider-key.txt");
    let redacted_with_key = root.join("redacted-with-key.txt");
    fs::write(&redacted, "Authorization: [REDACTED]\n").unwrap();
    fs::write(&raw, "Authorization: bearer secret\n").unwrap();
    fs::write(&benign, "Task-specific context is not a secret marker.\n").unwrap();
    fs::write(&provider_key, "example leaked key sk-proj-not-a-real-key\n").unwrap();
    fs::write(
        &redacted_with_key,
        "Authorization: [REDACTED] Bearer sk-proj-not-a-real-key\n",
    )
    .unwrap();

    scan_artifacts_for_sensitive_markers([&redacted]).unwrap();
    scan_artifacts_for_sensitive_markers([&benign]).unwrap();
    let error = scan_artifacts_for_sensitive_markers([&raw]).unwrap_err();
    let key_error = scan_artifacts_for_sensitive_markers([&provider_key]).unwrap_err();
    let redacted_key_error =
        scan_artifacts_for_sensitive_markers([&redacted_with_key]).unwrap_err();

    assert!(matches!(
        error,
        LocalAdapterSmokeError::SensitiveArtifact { marker, .. } if marker == "authorization:"
    ));
    assert!(matches!(
        key_error,
        LocalAdapterSmokeError::SensitiveArtifact { marker, .. } if marker == "sk-proj-"
    ));
    assert!(matches!(
        redacted_key_error,
        LocalAdapterSmokeError::SensitiveArtifact { marker, .. } if marker == "sk-proj-"
    ));

    let legacy = root.join("legacy-key.txt");
    fs::write(&legacy, "legacy leaked key sk-abcdefghijklmnopqrstuvwx\n").unwrap();
    let legacy_error = scan_artifacts_for_sensitive_markers([&legacy]).unwrap_err();
    assert!(matches!(
        legacy_error,
        LocalAdapterSmokeError::SensitiveArtifact { marker, .. } if marker == "sk-"
    ));
}

fn temp_root(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    std::env::temp_dir().join(format!("capo-adapter-{name}-{nanos}"))
}

fn acp_client_call(method: &str, params: Value) -> AcpClientCall {
    acp_client_call_with_profile(method, params, "read-only-local")
}

fn acp_client_call_with_profile(
    method: &str,
    params: Value,
    capability_profile_id: &str,
) -> AcpClientCall {
    AcpClientCall {
        method: method.to_string(),
        params,
        tool_call_id: ToolCallId::new(format!("tool-call-{}", method.replace(['/', '_'], "-"))),
        session_id: SessionId::new("session-acp-client-call"),
        run_id: RunId::new("run-acp-client-call"),
        capability_profile_id: capability_profile_id.to_string(),
    }
}
