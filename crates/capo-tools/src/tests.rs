use super::*;
use capo_core::RunId;
use capo_runtime::RedactionRule;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn first_tool_set_supports_status_and_evidence() {
    assert!(CAPO_OWNED_TOOLS.contains(&"capo.task_status"));
    assert!(CAPO_OWNED_TOOLS.contains(&"capo.evidence_record"));
}

#[test]
fn fake_tool_and_permission_are_separate_boundaries() {
    assert_eq!(
        ToolExposure::fake().binding().kind,
        BoundaryKind::ToolExposure
    );
    assert_eq!(
        ToolExposure::capo().binding(),
        BoundaryBinding {
            kind: BoundaryKind::ToolExposure,
            variant: "capo-registry",
            fake: false,
        }
    );
    assert_eq!(
        PermissionPolicy::fake().binding().kind,
        BoundaryKind::PermissionPolicy
    );
}

#[test]
fn trusted_local_policy_is_explicitly_not_fake() {
    let binding = PermissionPolicy::allow_trusted_local().binding();
    assert_eq!(binding.kind, BoundaryKind::PermissionPolicy);
    assert_eq!(binding.variant, "trusted-local");
    assert!(!binding.fake);

    let static_binding = PermissionPolicy::static_read_only_local().binding();
    assert_eq!(static_binding.kind, BoundaryKind::PermissionPolicy);
    assert_eq!(static_binding.variant, "static");
    assert!(!static_binding.fake);
}

#[test]
fn capo_registry_defines_first_tools() {
    let registry = CapoToolRegistry;
    let tools = registry.list_tools();

    assert_eq!(tools.len(), 7);
    for tool_id in CAPO_OWNED_TOOLS {
        let definition = registry.describe_tool(tool_id).expect("tool definition");
        assert_eq!(definition.origin, "capo");
        assert_eq!(definition.handler_kind, "capo_registry");
        assert_eq!(definition.instrumentation_level, "full");
        assert!(
            definition
                .required_scopes_json
                .contains(&format!("tool:invoke:{tool_id}"))
        );
    }
}

#[test]
fn runtime_wrappers_define_shell_git_file_and_project_memory_tools() {
    let wrappers = RuntimeToolWrappers::new(RuntimeToolConfig::local_workspace(
        PathBuf::from("/tmp/capo-workspace"),
        PathBuf::from("/tmp/capo-artifacts"),
    ));
    let tools = wrappers.list_tools();

    assert_eq!(tools.len(), 9);
    for tool_id in CAPO_WRAPPER_TOOLS {
        let definition = wrappers.describe_tool(tool_id).expect("wrapper definition");
        assert_eq!(definition.origin, "runtime");
        assert_eq!(definition.handler_kind, "runtime_wrapper");
        assert_eq!(definition.instrumentation_level, "full");
        assert!(
            definition
                .required_scopes_json
                .contains(&format!("tool:invoke:{tool_id}"))
        );
    }
    assert_eq!(
        wrappers
            .describe_tool("capo.shell_run")
            .expect("shell tool")
            .risk,
        "high"
    );
    assert!(
        wrappers
            .describe_tool("capo.git_commit")
            .expect("git commit tool")
            .mutates_state
    );
    assert_eq!(
        wrappers
            .describe_tool("capo.git_commit")
            .expect("git commit tool")
            .risk,
        "high"
    );
    assert!(
        wrappers
            .describe_tool("capo.file_write")
            .expect("file write tool")
            .mutates_state
    );
}

#[test]
fn acp_client_capabilities_require_wrappers_and_policy_allow() {
    let wrappers = RuntimeToolWrappers::new(RuntimeToolConfig::local_workspace(
        PathBuf::from("/tmp/capo-workspace"),
        PathBuf::from("/tmp/capo-artifacts"),
    ));

    let trusted = AcpClientCapabilityPlan::from_runtime_wrappers(
        &wrappers,
        &PermissionPolicy::allow_trusted_local(),
        SessionId::new("session-acp-trusted"),
    );
    assert_eq!(
        trusted.advertised_capabilities(),
        vec![
            "filesystem.read_text_file",
            "filesystem.write_text_file",
            "terminal"
        ]
    );
    assert_eq!(
        trusted.filesystem_read.reason,
        "backing_wrapper_tool_allowed"
    );
    assert_eq!(trusted.terminal.permission_effect.as_deref(), Some("allow"));

    let read_only = AcpClientCapabilityPlan::from_runtime_wrappers(
        &wrappers,
        &PermissionPolicy::static_read_only_local(),
        SessionId::new("session-acp-read-only"),
    );
    assert!(read_only.filesystem_read.advertise);
    assert!(!read_only.filesystem_write.advertise);
    assert!(!read_only.terminal.advertise);
    assert_eq!(
        read_only.advertised_capabilities(),
        vec!["filesystem.read_text_file"]
    );
    assert_eq!(
        read_only.filesystem_write.permission_effect.as_deref(),
        Some("deny")
    );
    assert!(
        read_only
            .terminal
            .reason
            .contains("permission_policy_rejected")
    );
}

#[test]
fn acp_client_capabilities_fail_closed_without_backing_wrappers() {
    let definitions = RuntimeToolWrappers::new(RuntimeToolConfig::local_workspace(
        PathBuf::from("/tmp/capo-workspace"),
        PathBuf::from("/tmp/capo-artifacts"),
    ))
    .list_tools()
    .into_iter()
    .filter(|definition| definition.tool_id != "capo.shell_run")
    .collect::<Vec<_>>();

    let plan = AcpClientCapabilityPlan::from_tool_definitions(
        &definitions,
        &PermissionPolicy::allow_trusted_local(),
        SessionId::new("session-acp-missing-wrapper"),
    );

    assert!(!plan.terminal.advertise);
    assert_eq!(plan.terminal.reason, "missing_backing_wrapper_tool");
    assert_eq!(plan.terminal.required_scopes_json, None);
    assert_eq!(plan.terminal.permission_effect, None);
    assert!(plan.filesystem_read.advertise);
}

#[test]
fn file_wrappers_record_input_output_artifacts_and_reject_workspace_escape() {
    let workspace = temp_root("tool-wrapper-workspace");
    let artifacts = temp_root("tool-wrapper-artifacts");
    fs::create_dir_all(&workspace).expect("workspace");
    fs::write(workspace.join("note.md"), "hello").expect("write note");
    let wrappers = RuntimeToolWrappers::new(RuntimeToolConfig::local_workspace(
        workspace.clone(),
        artifacts.clone(),
    ));
    let policy = PermissionPolicy::allow_trusted_local();

    let read = wrappers.authorize_and_invoke(
        wrapper_request(
            "call-file-read",
            "run-file-read",
            "capo.file_read",
            serde_json::json!({"path":"note.md"}),
        ),
        &policy,
    );
    assert_eq!(read.status, "completed");
    assert!(read.input_artifact.is_some());
    assert_eq!(read.output_artifacts.len(), 1);
    assert_eq!(
        fs::read_to_string(&read.output_artifacts[0].uri).expect("read artifact"),
        "hello"
    );
    assert!(
        read.events.iter().any(|event| {
            event.kind == "tool.output_artifact_recorded" && event.status == "safe"
        })
    );

    let write = wrappers.authorize_and_invoke(
        wrapper_request(
            "call-file-write",
            "run-file-write",
            "capo.file_write",
            serde_json::json!({"path":"nested/out.txt","content":"new text"}),
        ),
        &policy,
    );
    assert_eq!(write.status, "completed");
    assert_eq!(
        fs::read_to_string(workspace.join("nested/out.txt")).expect("written file"),
        "new text"
    );
    assert_eq!(write.output_artifacts[0].kind, "file_write_diff");
    // ACI3: the diff artifact is now a real unified diff (new file write).
    assert!(
        fs::read_to_string(&write.output_artifacts[0].uri)
            .expect("diff artifact")
            .contains("+new text")
    );
    assert_eq!(
        write.narrow_output()["mode"],
        serde_json::json!("overwrite")
    );

    let escaped = wrappers.authorize_and_invoke(
        wrapper_request(
            "call-file-escape",
            "run-file-escape",
            "capo.file_read",
            serde_json::json!({"path":"../outside.txt"}),
        ),
        &policy,
    );
    assert_eq!(escaped.status, "failed");
    assert!(
        escaped.summary.contains("escapes workspace"),
        "a `..`-escape must be rejected as a containment violation, got {}",
        escaped.summary
    );

    let workpad_escape = wrappers.authorize_and_invoke(
        wrapper_request(
            "call-workpad-escape",
            "run-workpad-escape",
            "capo.workpad_read",
            serde_json::json!({"path":"note.md"}),
        ),
        &PermissionPolicy::static_read_only_local(),
    );
    assert_eq!(workpad_escape.status, "failed");
    assert!(
        workpad_escape
            .summary
            .contains("workpad_read only supports")
    );
    let project_memory_escape = wrappers.authorize_and_invoke(
        wrapper_request(
            "call-project-memory-escape",
            "run-project-memory-escape",
            "capo.project_memory_read",
            serde_json::json!({"path":"note.md"}),
        ),
        &PermissionPolicy::static_read_only_local(),
    );
    assert_eq!(project_memory_escape.status, "failed");
    assert!(
        project_memory_escape
            .summary
            .contains("project_memory_read only supports")
    );

    fs::create_dir_all(workspace.join("workpads/features")).expect("workpad dir");
    fs::write(workspace.join("workpads/features/tasks.md"), "# Tasks\n").expect("workpad");
    let project_memory = wrappers.authorize_and_invoke(
        wrapper_request(
            "call-project-memory-read",
            "run-project-memory-read",
            "capo.project_memory_read",
            serde_json::json!({"path":"workpads/features/tasks.md"}),
        ),
        &PermissionPolicy::static_read_only_local(),
    );
    assert_eq!(project_memory.status, "completed");
    assert_eq!(
        project_memory.output_artifacts[0].kind,
        "project_memory_read"
    );
    let workpad = wrappers.authorize_and_invoke(
        wrapper_request(
            "call-workpad-read",
            "run-workpad-read",
            "capo.workpad_read",
            serde_json::json!({"path":"workpads/features/tasks.md"}),
        ),
        &PermissionPolicy::static_read_only_local(),
    );
    assert_eq!(workpad.status, "completed");
    assert_eq!(workpad.output_artifacts[0].kind, "workpad_read");

    let denied = wrappers.authorize_and_invoke(
        wrapper_request(
            "call-file-write-denied",
            "run-file-write-denied",
            "capo.file_write",
            serde_json::json!({"path":"denied.txt","content":"nope"}),
        ),
        &PermissionPolicy::static_read_only_local(),
    );
    assert_eq!(denied.status, "denied");
    assert!(denied.output_artifacts.is_empty());
    assert!(denied.events.iter().any(|event| {
        event.kind == "tool.call_canceled" && event.status == "permission_denied"
    }));
    // ACI3: a denied result still emits a schema-valid typed output carrying the
    // terminal status, so "every emitted result validates" holds on deny paths.
    let denied_definition = wrappers
        .describe_tool("capo.file_write")
        .expect("definition");
    assert!(
        denied_definition
            .validate_output(&denied.narrow_output())
            .is_empty()
    );
    assert_eq!(
        denied.narrow_output()["status"],
        serde_json::json!("denied")
    );
}

#[test]
fn wrapper_split_authorization_cannot_be_replayed_for_another_tool() {
    let workspace = temp_root("tool-wrapper-replay-workspace");
    let artifacts = temp_root("tool-wrapper-replay-artifacts");
    fs::create_dir_all(&workspace).expect("workspace");
    let wrappers = RuntimeToolWrappers::new(RuntimeToolConfig::local_workspace(
        workspace.clone(),
        artifacts,
    ));
    let authorization = wrappers.authorize_tool_call(
        &wrapper_request(
            "call-status-auth",
            "run-status-auth",
            "capo.git_status",
            serde_json::json!({}),
        ),
        &PermissionPolicy::static_read_only_local(),
    );

    let replay = wrappers.invoke_authorized(
        wrapper_request(
            "call-shell-replay",
            "run-shell-replay",
            "capo.shell_run",
            serde_json::json!({"program":"/bin/sh","argv":["-c","touch replayed"]}),
        ),
        authorization,
    );

    assert_eq!(replay.status, "denied");
    assert!(replay.summary.contains("authorization tool mismatch"));
    assert!(!workspace.join("replayed").exists());

    let shell_authorization = wrappers.authorize_tool_call(
        &wrapper_request(
            "call-shell-auth",
            "run-shell-auth",
            "capo.shell_run",
            serde_json::json!({"program":"/bin/sh","argv":["-c","true"]}),
        ),
        &PermissionPolicy::allow_trusted_local(),
    );
    let changed_input = wrappers.invoke_authorized(
        wrapper_request(
            "call-shell-auth",
            "run-shell-auth",
            "capo.shell_run",
            serde_json::json!({"program":"/bin/sh","argv":["-c","touch replayed"]}),
        ),
        shell_authorization,
    );
    assert_eq!(changed_input.status, "denied");
    assert!(
        changed_input
            .summary
            .contains("authorization input mismatch")
    );
    assert!(!workspace.join("replayed").exists());
}

#[test]
fn shell_and_git_wrappers_execute_through_runtime_with_artifacts() {
    let workspace = temp_root("tool-wrapper-git-workspace");
    let artifacts = temp_root("tool-wrapper-git-artifacts");
    fs::create_dir_all(&workspace).expect("workspace");
    Command::new("git")
        .args(["init"])
        .current_dir(&workspace)
        .output()
        .expect("git init");
    fs::write(workspace.join("tracked.txt"), "tracked\n").expect("write tracked");

    let mut config = RuntimeToolConfig::local_workspace(workspace.clone(), artifacts);
    config.redaction_rules.push(RedactionRule {
        pattern: "SECRET".to_string(),
        replacement: "[REDACTED]".to_string(),
    });
    let wrappers = RuntimeToolWrappers::new(config);
    let policy = PermissionPolicy::allow_trusted_local();

    let shell = wrappers.authorize_and_invoke(
        wrapper_request(
            "call-shell",
            "run-shell",
            "capo.shell_run",
            serde_json::json!({
                "program":"/bin/sh",
                "argv":["-c","printf SECRET"],
                "cwd":"."
            }),
        ),
        &policy,
    );
    assert_eq!(shell.status, "exited");
    let shell_input = shell.input_artifact.as_ref().expect("shell input");
    assert_eq!(shell_input.redaction_state, "redacted");
    assert!(
        fs::read_to_string(&shell_input.uri)
            .expect("shell input artifact")
            .contains("[REDACTED]")
    );
    assert_eq!(shell.output_artifacts.len(), 2);
    assert!(
        shell
            .output_artifacts
            .iter()
            .any(|artifact| artifact.redaction_state == "redacted")
    );
    assert!(
        shell
            .events
            .iter()
            .any(|event| event.kind == "capability.grant_used")
    );

    let git_status = wrappers.authorize_and_invoke(
        wrapper_request(
            "call-git-status",
            "run-git-status",
            "capo.git_status",
            serde_json::json!({}),
        ),
        &policy,
    );
    assert_eq!(git_status.status, "exited");
    let stdout = git_status
        .output_artifacts
        .iter()
        .find(|artifact| artifact.kind == "git_stdout")
        .expect("git stdout");
    assert!(
        fs::read_to_string(&stdout.uri)
            .expect("git stdout artifact")
            .contains("tracked.txt")
    );

    let denied_shell = wrappers.authorize_and_invoke(
        wrapper_request(
            "call-shell-denied",
            "run-shell-denied",
            "capo.shell_run",
            serde_json::json!({"program":"/bin/sh","argv":["-c","true"]}),
        ),
        &PermissionPolicy::static_read_only_local(),
    );
    assert_eq!(denied_shell.status, "denied");
    assert!(
        !denied_shell
            .events
            .iter()
            .any(|event| event.kind == "tool.invocation_started")
    );

    let escaped_artifact = wrappers.authorize_and_invoke(
        wrapper_request(
            "../call-shell-escape",
            "../run-shell-escape",
            "capo.shell_run",
            serde_json::json!({"program":"/bin/sh","argv":["-c","true"]}),
        ),
        &policy,
    );
    assert_eq!(escaped_artifact.status, "exited");
    assert!(
        !workspace
            .parent()
            .expect("workspace parent")
            .join("call-shell-escape")
            .exists()
    );
    assert!(
        !workspace
            .parent()
            .expect("workspace parent")
            .join("run-shell-escape")
            .exists()
    );
}

#[test]
fn git_commit_wrapper_commits_staged_changes_and_denies_static_profiles() {
    let workspace = temp_root("tool-wrapper-git-commit-workspace");
    let artifacts = temp_root("tool-wrapper-git-commit-artifacts");
    fs::create_dir_all(&workspace).expect("workspace");
    Command::new("git")
        .args(["init"])
        .current_dir(&workspace)
        .output()
        .expect("git init");
    fs::write(workspace.join("tracked.txt"), "tracked\n").expect("write tracked");
    Command::new("git")
        .args(["add", "tracked.txt"])
        .current_dir(&workspace)
        .output()
        .expect("git add");

    let wrappers = RuntimeToolWrappers::new(RuntimeToolConfig::local_workspace(
        workspace.clone(),
        artifacts,
    ));

    let commit = wrappers.authorize_and_invoke(
        wrapper_request(
            "call-git-commit",
            "run-git-commit",
            "capo.git_commit",
            serde_json::json!({"message":"Capo wrapper commit"}),
        ),
        &PermissionPolicy::allow_trusted_local(),
    );
    assert_eq!(commit.status, "exited");
    assert!(commit.input_artifact.is_some());
    assert_eq!(commit.output_artifacts.len(), 2);
    assert!(
        commit
            .output_artifacts
            .iter()
            .any(|artifact| artifact.kind == "git_commit_stdout")
    );
    assert!(
        commit
            .events
            .iter()
            .any(|event| event.kind == "tool.invocation_started")
    );
    let log = Command::new("git")
        .args(["log", "--oneline", "-1"])
        .current_dir(&workspace)
        .output()
        .expect("git log");
    assert!(
        String::from_utf8_lossy(&log.stdout).contains("Capo wrapper commit"),
        "git log should show wrapper commit"
    );

    fs::write(workspace.join("denied.txt"), "denied\n").expect("write denied");
    Command::new("git")
        .args(["add", "denied.txt"])
        .current_dir(&workspace)
        .output()
        .expect("git add denied");
    let denied = wrappers.authorize_and_invoke(
        wrapper_request(
            "call-git-commit-denied",
            "run-git-commit-denied",
            "capo.git_commit",
            serde_json::json!({"message":"Denied commit"}),
        ),
        &PermissionPolicy::static_read_only_local(),
    );
    assert_eq!(denied.status, "denied");
    assert!(denied.output_artifacts.is_empty());
    assert!(denied.summary.contains("git:commit:workspace"));
    assert!(
        !denied
            .events
            .iter()
            .any(|event| event.kind == "tool.invocation_started")
    );

    let reviewer_denied = wrappers.authorize_and_invoke(
        wrapper_request(
            "call-git-commit-reviewer-denied",
            "run-git-commit-reviewer-denied",
            "capo.git_commit",
            serde_json::json!({"message":"Reviewer denied commit"}),
        ),
        &PermissionPolicy::static_reviewer(),
    );
    assert_eq!(reviewer_denied.status, "denied");
    assert!(reviewer_denied.summary.contains("git:commit:workspace"));

    let missing_message = wrappers.authorize_and_invoke(
        wrapper_request(
            "call-git-commit-empty",
            "run-git-commit-empty",
            "capo.git_commit",
            serde_json::json!({"message":"   "}),
        ),
        &PermissionPolicy::allow_trusted_local(),
    );
    assert_eq!(missing_message.status, "failed");
    assert!(
        missing_message
            .summary
            .contains("git_commit requires a non-empty message")
    );
    assert!(missing_message.output_artifacts.is_empty());
}

#[test]
fn capo_tools_render_expected_context_outputs() {
    let registry = CapoToolRegistry;
    let policy = PermissionPolicy::allow_trusted_local();
    let context = tool_context();

    let cases = [
        ("capo.task_status", "task active"),
        ("capo.agent_status", "agent running"),
        ("capo.session_summary", "summary text"),
        ("capo.project_memory_read", "workpad section"),
        ("capo.workpad_read", "workpad section"),
        ("capo.evidence_record", "evidence recorded: tests passed"),
        (
            "capo.capability_request",
            "capability requested: shell:execute:workspace",
        ),
    ];

    for (tool_id, expected) in cases {
        let result = registry.authorize_and_invoke(
            CapoToolRequest {
                tool_call_id: ToolCallId::new(format!("call-{tool_id}")),
                session_id: SessionId::new("session-tools"),
                tool_id: tool_id.to_string(),
                capability_profile_id: "trusted-local-dev".to_string(),
                context: context.clone(),
            },
            &policy,
        );

        assert_eq!(result.output, expected);
    }
}

#[test]
fn trusted_local_tool_invocation_still_emits_audit_lifecycle() {
    let registry = CapoToolRegistry;
    let result = registry.authorize_and_invoke(
        CapoToolRequest {
            tool_call_id: ToolCallId::new("call-session-summary"),
            session_id: SessionId::new("session-tools"),
            tool_id: "capo.session_summary".to_string(),
            capability_profile_id: "trusted-local-dev".to_string(),
            context: tool_context(),
        },
        &PermissionPolicy::allow_trusted_local(),
    );

    let event_kinds = result
        .events
        .iter()
        .map(|event| event.kind.as_str())
        .collect::<Vec<_>>();

    assert_eq!(
        event_kinds,
        [
            "tool.call_requested",
            "permission.requested",
            "permission.decided",
            "capability.grant_used",
            "tool.invocation_started",
            "tool.output_artifact_recorded",
            "tool.output_observed",
            "tool.call_completed",
            "tool.result_delivered",
        ]
    );
    assert_eq!(result.permission_decision.effect, "allow");
    assert!(
        result
            .permission_decision
            .scope_json
            .contains("state:read:session")
    );
    assert_eq!(
        result.output_artifact_id,
        "artifact-call-session-summary-capo-session_summary"
    );
}

#[test]
fn capo_registry_split_authorization_cannot_be_replayed_with_changed_context() {
    let registry = CapoToolRegistry;
    let request = CapoToolRequest {
        tool_call_id: ToolCallId::new("call-evidence"),
        session_id: SessionId::new("session-tools"),
        tool_id: "capo.evidence_record".to_string(),
        capability_profile_id: "trusted-local-dev".to_string(),
        context: tool_context(),
    };
    let authorization =
        registry.authorize_tool_call(&request, &PermissionPolicy::allow_trusted_local());
    let replay = registry.invoke_authorized(
        CapoToolRequest {
            context: CapoToolContext {
                evidence_note: "different evidence".to_string(),
                ..tool_context()
            },
            ..request
        },
        authorization,
    );

    assert_eq!(replay.output, "authorization input mismatch");
    assert_eq!(replay.output_artifact_id, "none");
    assert!(replay.events.iter().any(|event| {
        event.kind == "tool.call_canceled" && event.status == "authorization_mismatch"
    }));

    let ambiguous = CapoToolRequest {
        tool_call_id: ToolCallId::new("call-ambiguous"),
        session_id: SessionId::new("session-tools"),
        tool_id: "capo.evidence_record".to_string(),
        capability_profile_id: "trusted-local-dev".to_string(),
        context: CapoToolContext {
            task_status: "a\nb".to_string(),
            agent_status: "c".to_string(),
            session_summary: "summary text".to_string(),
            workpad_excerpt: "workpad section".to_string(),
            evidence_note: "tests passed".to_string(),
            capability_scope: "shell:execute:workspace".to_string(),
        },
    };
    let ambiguous_authorization =
        registry.authorize_tool_call(&ambiguous, &PermissionPolicy::allow_trusted_local());
    let ambiguous_replay = registry.invoke_authorized(
        CapoToolRequest {
            context: CapoToolContext {
                task_status: "a".to_string(),
                agent_status: "b\nc".to_string(),
                session_summary: "summary text".to_string(),
                workpad_excerpt: "workpad section".to_string(),
                evidence_note: "tests passed".to_string(),
                capability_scope: "shell:execute:workspace".to_string(),
            },
            ..ambiguous
        },
        ambiguous_authorization,
    );
    assert_eq!(ambiguous_replay.output, "authorization input mismatch");
}

#[test]
fn static_read_only_policy_allows_read_tools_and_denies_writes() {
    let registry = CapoToolRegistry;
    let policy = PermissionPolicy::static_read_only_local();

    let read_result = registry.authorize_and_invoke(
        CapoToolRequest {
            tool_call_id: ToolCallId::new("call-session-summary"),
            session_id: SessionId::new("session-tools"),
            tool_id: "capo.session_summary".to_string(),
            capability_profile_id: "read-only-local".to_string(),
            context: tool_context(),
        },
        &policy,
    );

    assert_eq!(read_result.permission_decision.effect, "allow");
    assert_eq!(
        read_result.permission_decision.decision_source,
        "static_policy:read-only-local"
    );
    assert!(
        read_result
            .events
            .iter()
            .any(|event| { event.kind == "tool.invocation_started" && event.status == "running" })
    );

    let write_result = registry.authorize_and_invoke(
        CapoToolRequest {
            tool_call_id: ToolCallId::new("call-evidence-record"),
            session_id: SessionId::new("session-tools"),
            tool_id: "capo.evidence_record".to_string(),
            capability_profile_id: "read-only-local".to_string(),
            context: tool_context(),
        },
        &policy,
    );

    assert_eq!(write_result.permission_decision.effect, "deny");
    assert!(
        write_result
            .permission_decision
            .explanation
            .contains("state:write:evidence")
    );
    assert_eq!(write_result.output_artifact_id, "none");
    assert!(write_result.events.iter().any(|event| {
        event.kind == "tool.call_canceled" && event.status == "permission_denied"
    }));
    assert!(
        !write_result
            .events
            .iter()
            .any(|event| event.kind == "tool.invocation_started")
    );
}

#[test]
fn static_reviewer_policy_keeps_decisions_scoped() {
    let policy = PermissionPolicy::static_reviewer();
    let allowed = policy.decide(PermissionRequest {
        session_id: SessionId::new("session-review"),
        capability_profile_id: "reviewer".to_string(),
        scope_json: json_array(vec!["git:diff:workspace", "state:read:task"]),
    });
    assert_eq!(allowed.effect, "allow");
    assert_eq!(allowed.persistence, "once");
    assert!(allowed.scope_json.contains("git:diff:workspace"));

    let denied = policy.decide(PermissionRequest {
        session_id: SessionId::new("session-review"),
        capability_profile_id: "reviewer".to_string(),
        scope_json: json_array(vec!["shell:execute:workspace"]),
    });
    assert_eq!(denied.effect, "deny");
    assert!(denied.explanation.contains("shell:execute:workspace"));
    assert_eq!(denied.subject_json, "{\"session_id\":\"session-review\"}");
}

#[test]
fn static_policy_rejects_malformed_scope_payloads() {
    let policy = PermissionPolicy::static_read_only_local();
    let object_payload = policy.decide(PermissionRequest {
        session_id: SessionId::new("session-static"),
        capability_profile_id: "read-only-local".to_string(),
        scope_json: "{\"tool:invoke:capo.workpad_read\":true}".to_string(),
    });
    assert_eq!(object_payload.effect, "deny");
    assert!(object_payload.explanation.contains("non-array scope json"));

    let non_string_payload = policy.decide(PermissionRequest {
        session_id: SessionId::new("session-static"),
        capability_profile_id: "read-only-local".to_string(),
        scope_json: "[\"state:read:task\",true]".to_string(),
    });
    assert_eq!(non_string_payload.effect, "deny");
    assert!(
        non_string_payload
            .explanation
            .contains("non-string scope item")
    );
}

#[test]
fn grant_ids_include_scope_identity() {
    let policy = PermissionPolicy::static_read_only_local();
    let status = policy.decide(PermissionRequest {
        session_id: SessionId::new("session-static"),
        capability_profile_id: "read-only-local".to_string(),
        scope_json: json_array(vec!["state:read:task"]),
    });
    let summary = policy.decide(PermissionRequest {
        session_id: SessionId::new("session-static"),
        capability_profile_id: "read-only-local".to_string(),
        scope_json: json_array(vec!["state:read:session"]),
    });

    assert_ne!(status.capability_grant_id, summary.capability_grant_id);
    assert!(
        status
            .capability_grant_id
            .starts_with("grant-session-static-allow-")
    );
    assert!(
        summary
            .capability_grant_id
            .starts_with("grant-session-static-allow-")
    );
}

// --- ACI3: narrow typed wrapper output + tightened file_write ---------------

#[test]
fn shell_run_typed_output_carries_exit_status_passed_duration_and_artifact() {
    // ACI3: capo.shell_run typed output carries exit status, a `passed`
    // interpretation, duration, and output_artifact_id, validating against the
    // declared output_schema, with full output in the artifact (not inline).
    let workspace = temp_root("aci3-shell-typed-workspace");
    let artifacts = temp_root("aci3-shell-typed-artifacts");
    fs::create_dir_all(&workspace).expect("workspace");
    let wrappers = RuntimeToolWrappers::new(RuntimeToolConfig::local_workspace(
        workspace.clone(),
        artifacts,
    ));
    let policy = PermissionPolicy::allow_trusted_local();

    let ok = wrappers.authorize_and_invoke(
        wrapper_request(
            "call-shell-ok",
            "run-shell-ok",
            "capo.shell_run",
            serde_json::json!({"program":"/bin/sh","argv":["-c","echo hello"],"cwd":"."}),
        ),
        &policy,
    );
    assert_eq!(ok.status, "exited");
    let definition = wrappers
        .describe_tool("capo.shell_run")
        .expect("definition");
    let errors = definition.validate_output(&ok.narrow_output());
    assert!(
        errors.is_empty(),
        "shell_run typed output must validate, got {errors:?}"
    );
    let typed = ok.narrow_output();
    assert_eq!(typed["exit_status"], serde_json::json!(0));
    assert_eq!(typed["passed"], serde_json::json!(true));
    assert_eq!(typed["truncated"], serde_json::json!(false));
    assert!(typed["duration_ms"].is_i64());
    let artifact_id = typed["output_artifact_id"].as_str().expect("artifact id");
    let stdout = ok
        .output_artifacts
        .iter()
        .find(|artifact| artifact.artifact_id == artifact_id)
        .expect("output artifact referenced by typed output");
    assert!(
        fs::read_to_string(&stdout.uri)
            .expect("stdout artifact")
            .contains("hello")
    );

    // A non-zero exit is observed (status `failed`) but `passed` is false; the
    // tool still produces a typed result rather than only a status blob.
    let fail = wrappers.authorize_and_invoke(
        wrapper_request(
            "call-shell-fail",
            "run-shell-fail",
            "capo.shell_run",
            serde_json::json!({"program":"/bin/sh","argv":["-c","exit 3"],"cwd":"."}),
        ),
        &policy,
    );
    let fail_typed = fail.narrow_output();
    assert_eq!(fail_typed["exit_status"], serde_json::json!(3));
    assert_eq!(fail_typed["passed"], serde_json::json!(false));
}

#[test]
fn shell_run_over_cap_success_is_truncated_not_failed() {
    // ACI3: a successful run that exceeds the inline output cap is NOT
    // classified as failed -- the run still `passed`, output is preserved in the
    // artifact, and `truncated` is recorded in the typed result.
    let workspace = temp_root("aci3-shell-overcap-workspace");
    let artifacts = temp_root("aci3-shell-overcap-artifacts");
    fs::create_dir_all(&workspace).expect("workspace");
    let mut config = RuntimeToolConfig::local_workspace(workspace.clone(), artifacts);
    config.output_limit_bytes = 4; // tiny inline cap
    let wrappers = RuntimeToolWrappers::new(config);
    let policy = PermissionPolicy::allow_trusted_local();

    let over_cap = wrappers.authorize_and_invoke(
        wrapper_request(
            "call-shell-overcap",
            "run-shell-overcap",
            "capo.shell_run",
            // 26 bytes of stdout, far over the 4-byte inline cap, exit 0.
            serde_json::json!({"program":"/bin/sh","argv":["-c","echo abcdefghijklmnopqrstuvwxy"],"cwd":"."}),
        ),
        &policy,
    );

    assert_eq!(
        over_cap.status, "exited",
        "an over-cap SUCCESS must stay `exited`, not be downgraded to failed"
    );
    let typed = over_cap.narrow_output();
    assert_eq!(typed["passed"], serde_json::json!(true));
    assert_eq!(
        typed["truncated"],
        serde_json::json!(true),
        "over-cap output must be flagged truncated in the typed result"
    );
    // Full output is preserved in the artifact despite the small inline cap.
    let artifact_id = typed["output_artifact_id"].as_str().expect("artifact id");
    let stdout = over_cap
        .output_artifacts
        .iter()
        .find(|artifact| artifact.artifact_id == artifact_id)
        .expect("stdout artifact");
    assert!(
        stdout.size_bytes > 4,
        "the full output must live in the artifact, got {} bytes",
        stdout.size_bytes
    );
    assert!(
        fs::read_to_string(&stdout.uri)
            .expect("stdout artifact")
            .contains("abcdefghijklmnopqrstuvwxy")
    );
}

#[test]
fn file_read_typed_output_carries_path_bytes_and_hash() {
    // ACI3: capo.file_read returns a typed result (path, bytes_read,
    // content_hash, output_artifact_id) validating against its output_schema.
    let workspace = temp_root("aci3-file-read-workspace");
    let artifacts = temp_root("aci3-file-read-artifacts");
    fs::create_dir_all(&workspace).expect("workspace");
    fs::write(workspace.join("note.md"), "hello aci3").expect("seed");
    let wrappers = RuntimeToolWrappers::new(RuntimeToolConfig::local_workspace(
        workspace.clone(),
        artifacts,
    ));
    let read = wrappers.authorize_and_invoke(
        wrapper_request(
            "call-aci3-read",
            "run-aci3-read",
            "capo.file_read",
            serde_json::json!({"path":"note.md"}),
        ),
        &PermissionPolicy::allow_trusted_local(),
    );
    assert_eq!(read.status, "completed");
    let definition = wrappers
        .describe_tool("capo.file_read")
        .expect("definition");
    let errors = definition.validate_output(&read.narrow_output());
    assert!(errors.is_empty(), "file_read typed output: {errors:?}");
    let typed = read.narrow_output();
    assert_eq!(typed["bytes_read"], serde_json::json!(10));
    assert!(
        typed["content_hash"]
            .as_str()
            .expect("hash")
            .starts_with("fnv1a64:")
    );
}

#[test]
fn file_write_emits_a_unified_diff_artifact() {
    // ACI3: file_write emits a unified-diff artifact (before -> after), not a
    // before/after hash summary, and the typed result records mode + hashes.
    let workspace = temp_root("aci3-file-write-diff-workspace");
    let artifacts = temp_root("aci3-file-write-diff-artifacts");
    fs::create_dir_all(&workspace).expect("workspace");
    fs::write(workspace.join("doc.txt"), "line one\nline two\n").expect("seed");
    let wrappers = RuntimeToolWrappers::new(RuntimeToolConfig::local_workspace(
        workspace.clone(),
        artifacts,
    ));
    let write = wrappers.authorize_and_invoke(
        wrapper_request(
            "call-aci3-write",
            "run-aci3-write",
            "capo.file_write",
            serde_json::json!({"path":"doc.txt","content":"line one\nline two changed\n"}),
        ),
        &PermissionPolicy::allow_trusted_local(),
    );
    assert_eq!(write.status, "completed");
    let definition = wrappers
        .describe_tool("capo.file_write")
        .expect("definition");
    let errors = definition.validate_output(&write.narrow_output());
    assert!(errors.is_empty(), "file_write typed output: {errors:?}");
    let typed = write.narrow_output();
    assert_eq!(typed["mode"], serde_json::json!("overwrite"));
    assert_ne!(typed["before_hash"], typed["after_hash"]);

    let diff_artifact = write
        .output_artifacts
        .iter()
        .find(|artifact| artifact.kind == "file_write_diff")
        .expect("diff artifact");
    let diff = fs::read_to_string(&diff_artifact.uri).expect("diff");
    assert!(
        diff.contains("@@") && diff.contains("-line two") && diff.contains("+line two changed"),
        "diff artifact must be a real unified diff, got:\n{diff}"
    );
}

#[test]
fn file_write_precondition_mismatch_does_not_clobber() {
    // ACI3: a file_write whose expected-precondition hash does not match the
    // on-disk file returns a typed precondition-failed result WITHOUT writing,
    // so blind clobbers are impossible.
    let workspace = temp_root("aci3-file-write-precond-workspace");
    let artifacts = temp_root("aci3-file-write-precond-artifacts");
    fs::create_dir_all(&workspace).expect("workspace");
    let original = "original content\n";
    fs::write(workspace.join("guard.txt"), original).expect("seed");
    let wrappers = RuntimeToolWrappers::new(RuntimeToolConfig::local_workspace(
        workspace.clone(),
        artifacts,
    ));
    let policy = PermissionPolicy::allow_trusted_local();

    let mismatch = wrappers.authorize_and_invoke(
        wrapper_request(
            "call-aci3-precond-bad",
            "run-aci3-precond-bad",
            "capo.file_write",
            serde_json::json!({
                "path":"guard.txt",
                "content":"clobbered\n",
                "expected_hash":"fnv1a64:0000000000000000",
            }),
        ),
        &policy,
    );
    assert_eq!(mismatch.status, "precondition_failed");
    let typed = mismatch.narrow_output();
    assert_eq!(typed["status"], serde_json::json!("precondition_failed"));
    assert!(typed["expected_hash"].is_string());
    assert!(typed["actual_hash"].is_string());
    assert_ne!(typed["expected_hash"], typed["actual_hash"]);
    // The on-disk file is untouched.
    assert_eq!(
        fs::read_to_string(workspace.join("guard.txt")).expect("file"),
        original,
        "a precondition mismatch must NOT write"
    );

    // The matching expected hash allows the write through.
    let actual_hash = typed["actual_hash"].as_str().expect("actual hash");
    let ok = wrappers.authorize_and_invoke(
        wrapper_request(
            "call-aci3-precond-ok",
            "run-aci3-precond-ok",
            "capo.file_write",
            serde_json::json!({
                "path":"guard.txt",
                "content":"updated\n",
                "expected_hash":actual_hash,
            }),
        ),
        &policy,
    );
    assert_eq!(ok.status, "completed");
    assert_eq!(
        fs::read_to_string(workspace.join("guard.txt")).expect("file"),
        "updated\n"
    );
}

#[test]
fn file_write_structured_replace_edits_in_place() {
    // ACI3: file_write accepts a structured replace (replace/with) against the
    // current on-disk content instead of a whole-file overwrite.
    let workspace = temp_root("aci3-file-write-replace-workspace");
    let artifacts = temp_root("aci3-file-write-replace-artifacts");
    fs::create_dir_all(&workspace).expect("workspace");
    fs::write(workspace.join("cfg.toml"), "name = \"old\"\nkeep = 1\n").expect("seed");
    let wrappers = RuntimeToolWrappers::new(RuntimeToolConfig::local_workspace(
        workspace.clone(),
        artifacts,
    ));
    let policy = PermissionPolicy::allow_trusted_local();

    let replace = wrappers.authorize_and_invoke(
        wrapper_request(
            "call-aci3-replace",
            "run-aci3-replace",
            "capo.file_write",
            serde_json::json!({"path":"cfg.toml","replace":"\"old\"","with":"\"new\""}),
        ),
        &policy,
    );
    assert_eq!(replace.status, "completed");
    assert_eq!(
        replace.narrow_output()["mode"],
        serde_json::json!("replace")
    );
    assert_eq!(
        fs::read_to_string(workspace.join("cfg.toml")).expect("file"),
        "name = \"new\"\nkeep = 1\n"
    );

    // A replace whose target is absent is a structured failure, not a clobber.
    let missing = wrappers.authorize_and_invoke(
        wrapper_request(
            "call-aci3-replace-miss",
            "run-aci3-replace-miss",
            "capo.file_write",
            serde_json::json!({"path":"cfg.toml","replace":"NOT THERE","with":"x"}),
        ),
        &policy,
    );
    assert_eq!(missing.status, "failed");
    assert!(missing.summary.contains("replace target not found"));
    assert_eq!(
        fs::read_to_string(workspace.join("cfg.toml")).expect("file"),
        "name = \"new\"\nkeep = 1\n",
        "a missing replace target must not write"
    );
}

#[test]
fn apply_patch_clean_apply_returns_typed_diff_and_changed_ranges() {
    // ACI4: a clean search/replace apply returns a typed diff result (files
    // touched, hunks applied/rejected, changed line ranges) with the full diff
    // as an artifact, and validates against its declared output_schema.
    let workspace = temp_root("aci4-apply-clean-workspace");
    let artifacts = temp_root("aci4-apply-clean-artifacts");
    fs::create_dir_all(&workspace).expect("workspace");
    fs::write(
        workspace.join("notes.txt"),
        "first line\nsecond line\nthird line\n",
    )
    .expect("seed");
    let wrappers = RuntimeToolWrappers::new(RuntimeToolConfig::local_workspace(
        workspace.clone(),
        artifacts,
    ));
    let result = wrappers.authorize_and_invoke(
        wrapper_request(
            "call-aci4-clean",
            "run-aci4-clean",
            "capo.apply_patch",
            serde_json::json!({
                "path": "notes.txt",
                "auto_lint": false,
                "hunks": [{"search": "second line\n", "replace": "second line edited\n"}],
            }),
        ),
        &PermissionPolicy::allow_trusted_local(),
    );
    assert_eq!(result.status, "completed", "summary: {}", result.summary);
    let definition = wrappers
        .describe_tool("capo.apply_patch")
        .expect("definition");
    let errors = definition.validate_output(&result.narrow_output());
    assert!(errors.is_empty(), "apply_patch typed output: {errors:?}");
    let typed = result.narrow_output();
    assert_eq!(typed["hunks_total"], serde_json::json!(1));
    assert_eq!(typed["hunks_applied"], serde_json::json!(1));
    assert_eq!(typed["hunks_rejected"], serde_json::json!(0));
    let ranges = typed["changed_line_ranges"].as_array().expect("ranges");
    assert_eq!(ranges, &vec![serde_json::json!("2:2")]);
    assert_eq!(
        fs::read_to_string(workspace.join("notes.txt")).expect("file"),
        "first line\nsecond line edited\nthird line\n"
    );
    // The full diff is a redacted artifact, not inline in the typed output.
    let diff_artifact = result
        .output_artifacts
        .iter()
        .find(|artifact| artifact.kind == "apply_patch_diff")
        .expect("diff artifact");
    let diff = fs::read_to_string(&diff_artifact.uri).expect("diff");
    assert!(
        diff.contains("-second line") && diff.contains("+second line edited"),
        "diff artifact must be a real unified diff, got:\n{diff}"
    );
}

#[test]
fn apply_patch_whitespace_and_fuzzy_tolerant_location() {
    // ACI4: a hunk whose search drifts from the on-disk text (extra indent /
    // a small edit) still locates via the whitespace/fuzzy fallbacks.
    let workspace = temp_root("aci4-apply-fuzzy-workspace");
    let artifacts = temp_root("aci4-apply-fuzzy-artifacts");
    fs::create_dir_all(&workspace).expect("workspace");
    fs::write(
        workspace.join("indented.txt"),
        "        let value = compute();\n",
    )
    .expect("seed");
    let wrappers = RuntimeToolWrappers::new(RuntimeToolConfig::local_workspace(
        workspace.clone(),
        artifacts,
    ));
    let policy = PermissionPolicy::allow_trusted_local();
    // The search has NO leading indent; the file does. The whitespace-tolerant
    // strategy still locates and replaces it.
    let result = wrappers.authorize_and_invoke(
        wrapper_request(
            "call-aci4-ws",
            "run-aci4-ws",
            "capo.apply_patch",
            serde_json::json!({
                "path": "indented.txt",
                "auto_lint": false,
                "hunks": [{"search": "let value = compute();\n", "replace": "        let value = compute2();\n"}],
            }),
        ),
        &policy,
    );
    assert_eq!(result.status, "completed", "summary: {}", result.summary);
    assert_eq!(
        fs::read_to_string(workspace.join("indented.txt")).expect("file"),
        "        let value = compute2();\n"
    );
    assert!(
        result.summary.contains("whitespace"),
        "whitespace strategy must be reported, got: {}",
        result.summary
    );
}

#[test]
fn apply_patch_rejected_hunk_returns_structured_retryable_error_without_writing() {
    // ACI4: a hunk no strategy can locate returns a STRUCTURED retryable error
    // (which path, which hunk, nearest candidate), NOT a raw error string, and
    // does not write.
    let workspace = temp_root("aci4-apply-reject-workspace");
    let artifacts = temp_root("aci4-apply-reject-artifacts");
    fs::create_dir_all(&workspace).expect("workspace");
    let original = "alpha\nbravo\ncharlie\n";
    fs::write(workspace.join("src.txt"), original).expect("seed");
    let wrappers = RuntimeToolWrappers::new(RuntimeToolConfig::local_workspace(
        workspace.clone(),
        artifacts,
    ));
    let result = wrappers.authorize_and_invoke(
        wrapper_request(
            "call-aci4-reject",
            "run-aci4-reject",
            "capo.apply_patch",
            serde_json::json!({
                "path": "src.txt",
                "auto_lint": false,
                "hunks": [{"search": "totally\nunrelated\nblock\n", "replace": "x\n"}],
            }),
        ),
        &PermissionPolicy::allow_trusted_local(),
    );
    assert_eq!(result.status, "no_match");
    let definition = wrappers
        .describe_tool("capo.apply_patch")
        .expect("definition");
    // Even the structured no-match validates against the declared schema.
    let errors = definition.validate_output(&result.narrow_output());
    assert!(errors.is_empty(), "no_match typed output: {errors:?}");
    let typed = result.narrow_output();
    assert_eq!(typed["status"], serde_json::json!("no_match"));
    assert_eq!(typed["rejected_hunk_index"], serde_json::json!(0));
    assert!(
        typed["reject_reason"].as_str().is_some(),
        "structured reject must carry a reason"
    );
    assert!(
        typed["nearest_line"].as_i64().is_some(),
        "structured reject must carry the nearest candidate line"
    );
    // The file was NOT written.
    assert_eq!(
        fs::read_to_string(workspace.join("src.txt")).expect("file"),
        original,
        "a rejected hunk must not clobber the file"
    );
}

#[test]
fn apply_patch_no_match_preview_is_redacted_in_summary() {
    // ACI4 security: on a fuzzy near-miss the nearest-candidate preview is a
    // window of the TARGET FILE. If a configured secret sits in that window it
    // must be scrubbed before reaching the operator/loop-facing summary, the
    // same as every other content surface -- otherwise the no-match preview is a
    // redaction bypass.
    let workspace = temp_root("aci4-apply-redact-workspace");
    let artifacts = temp_root("aci4-apply-redact-artifacts");
    fs::create_dir_all(&workspace).expect("workspace");
    // A file whose best fuzzy candidate window holds a secret adjacent to the
    // agent's near-miss search block.
    let original = "alpha\nSECRET_TOKEN_abc123\ncharlie\n";
    fs::write(workspace.join("creds.txt"), original).expect("seed");

    let mut config = RuntimeToolConfig::local_workspace(workspace.clone(), artifacts);
    config.redaction_rules.push(RedactionRule {
        pattern: "SECRET_TOKEN_abc123".to_string(),
        replacement: "[REDACTED]".to_string(),
    });
    let wrappers = RuntimeToolWrappers::new(config);

    let result = wrappers.authorize_and_invoke(
        wrapper_request(
            "call-aci4-redact",
            "run-aci4-redact",
            "capo.apply_patch",
            serde_json::json!({
                "path": "creds.txt",
                "auto_lint": false,
                // A 3-line search that drifts from the file's 3 lines: it locates
                // the secret-bearing window as the nearest candidate, below the
                // fuzzy threshold (no match).
                "hunks": [{"search": "alpha\nWRONG_LINE\ncharlie\n", "replace": "x\n"}],
            }),
        ),
        &PermissionPolicy::allow_trusted_local(),
    );

    assert_eq!(result.status, "no_match", "summary: {}", result.summary);
    // The raw secret must NOT appear in the summary nor the typed preview.
    assert!(
        !result.summary.contains("SECRET_TOKEN_abc123"),
        "secret leaked into no_match summary: {}",
        result.summary
    );
    assert!(
        result.summary.contains("[REDACTED]"),
        "the redacted placeholder must be present in the preview: {}",
        result.summary
    );
    let typed = result.narrow_output();
    assert!(
        !typed.to_string().contains("SECRET_TOKEN_abc123"),
        "secret leaked into typed no_match output: {typed}"
    );
}

#[test]
fn apply_patch_lint_on_edit_returns_typed_findings() {
    // ACI4: after applying, a Rust file runs `rustfmt --check` and returns typed
    // lint findings (file, line, rule, message) the loop can repair.
    let workspace = temp_root("aci4-apply-lint-workspace");
    let artifacts = temp_root("aci4-apply-lint-artifacts");
    fs::create_dir_all(&workspace).expect("workspace");
    // A syntactically-valid but BADLY formatted Rust file; the patch leaves it
    // unformatted so `rustfmt --check` reports a diff.
    fs::write(workspace.join("lib.rs"), "fn main() {}\n").expect("seed");
    let wrappers = RuntimeToolWrappers::new(RuntimeToolConfig::local_workspace(
        workspace.clone(),
        artifacts,
    ));
    let result = wrappers.authorize_and_invoke(
        wrapper_request(
            "call-aci4-lint",
            "run-aci4-lint",
            "capo.apply_patch",
            serde_json::json!({
                "path": "lib.rs",
                "hunks": [{
                    "search": "fn main() {}\n",
                    "replace": "fn   main( )    {let x=1;}\n",
                }],
            }),
        ),
        &PermissionPolicy::allow_trusted_local(),
    );
    assert_eq!(result.status, "completed", "summary: {}", result.summary);
    let definition = wrappers
        .describe_tool("capo.apply_patch")
        .expect("definition");
    let errors = definition.validate_output(&result.narrow_output());
    assert!(errors.is_empty(), "apply_patch typed output: {errors:?}");
    let typed = result.narrow_output();
    assert_eq!(
        typed["lint_status"],
        serde_json::json!("failed"),
        "badly-formatted Rust must fail rustfmt --check, summary: {}",
        result.summary
    );
    let findings = typed["lint_findings"].as_array().expect("findings");
    assert!(!findings.is_empty(), "lint must produce typed findings");
    let finding = &findings[0];
    assert!(finding["file"].as_str().is_some(), "finding has file");
    assert!(finding["line"].as_i64().is_some(), "finding has line");
    assert!(finding["rule"].as_str().is_some(), "finding has rule");
    assert!(finding["message"].as_str().is_some(), "finding has message");
    // The misformat is on the only (first) line, so the parsed rustfmt line must
    // be the REAL non-zero header line, not the 0 fallback -- this exercises the
    // parser against genuine `rustfmt --check` output and guards line locality.
    let rustfmt_finding = findings
        .iter()
        .find(|finding| finding["rule"] == serde_json::json!("rustfmt"))
        .expect("a rustfmt-region finding");
    assert_eq!(
        rustfmt_finding["line"].as_i64(),
        Some(1),
        "rustfmt finding must carry the real edited-region line, not 0; findings: {findings:?}"
    );
}

#[test]
fn apply_patch_lint_passes_on_well_formatted_rust() {
    // ACI4: a well-formatted Rust edit passes rustfmt --check with no findings.
    let workspace = temp_root("aci4-apply-lint-ok-workspace");
    let artifacts = temp_root("aci4-apply-lint-ok-artifacts");
    fs::create_dir_all(&workspace).expect("workspace");
    fs::write(workspace.join("ok.rs"), "fn main() {}\n").expect("seed");
    let wrappers = RuntimeToolWrappers::new(RuntimeToolConfig::local_workspace(
        workspace.clone(),
        artifacts,
    ));
    let result = wrappers.authorize_and_invoke(
        wrapper_request(
            "call-aci4-lint-ok",
            "run-aci4-lint-ok",
            "capo.apply_patch",
            serde_json::json!({
                "path": "ok.rs",
                "hunks": [{
                    "search": "fn main() {}\n",
                    "replace": "fn main() {\n    let _x = 1;\n}\n",
                }],
            }),
        ),
        &PermissionPolicy::allow_trusted_local(),
    );
    assert_eq!(result.status, "completed", "summary: {}", result.summary);
    let typed = result.narrow_output();
    assert_eq!(
        typed["lint_status"],
        serde_json::json!("passed"),
        "well-formatted Rust must pass rustfmt --check, summary: {}",
        result.summary
    );
    assert!(
        typed["lint_findings"]
            .as_array()
            .expect("findings")
            .is_empty(),
        "passing lint must have no findings"
    );
}

#[test]
fn apply_patch_cannot_edit_outside_the_workspace() {
    // ACI4: patch writes reuse the wrapper path confinement, so a patch cannot
    // edit outside the workspace (absolute escape and `..` traversal rejected).
    let workspace = temp_root("aci4-apply-escape-workspace");
    let artifacts = temp_root("aci4-apply-escape-artifacts");
    let outside = temp_root("aci4-apply-escape-outside");
    fs::create_dir_all(&workspace).expect("workspace");
    fs::create_dir_all(&outside).expect("outside");
    let secret = outside.join("secret.txt");
    fs::write(&secret, "do not touch\n").expect("seed secret");
    let wrappers = RuntimeToolWrappers::new(RuntimeToolConfig::local_workspace(
        workspace.clone(),
        artifacts,
    ));
    let policy = PermissionPolicy::allow_trusted_local();

    // Absolute path outside the workspace.
    let abs = wrappers.authorize_and_invoke(
        wrapper_request(
            "call-aci4-escape-abs",
            "run-aci4-escape-abs",
            "capo.apply_patch",
            serde_json::json!({
                "path": secret.display().to_string(),
                "auto_lint": false,
                "hunks": [{"search": "do not touch\n", "replace": "hacked\n"}],
            }),
        ),
        &policy,
    );
    assert_eq!(abs.status, "failed", "absolute escape must be rejected");

    // `..` traversal escape.
    let traversal = wrappers.authorize_and_invoke(
        wrapper_request(
            "call-aci4-escape-rel",
            "run-aci4-escape-rel",
            "capo.apply_patch",
            serde_json::json!({
                "path": "../aci4-apply-escape-outside-x/secret.txt",
                "auto_lint": false,
                "hunks": [{"search": "do not touch\n", "replace": "hacked\n"}],
            }),
        ),
        &policy,
    );
    assert_eq!(
        traversal.status, "failed",
        "`..` traversal escape must be rejected"
    );
    // The outside file is untouched.
    assert_eq!(
        fs::read_to_string(&secret).expect("secret"),
        "do not touch\n",
        "a confined patch must never write outside the workspace"
    );
}

fn tool_context() -> CapoToolContext {
    CapoToolContext {
        task_status: "task active".to_string(),
        agent_status: "agent running".to_string(),
        session_summary: "summary text".to_string(),
        workpad_excerpt: "workpad section".to_string(),
        evidence_note: "tests passed".to_string(),
        capability_scope: "shell:execute:workspace".to_string(),
    }
}

fn wrapper_request(
    tool_call_id: &str,
    run_id: &str,
    tool_id: &str,
    input: Value,
) -> WrapperToolRequest {
    WrapperToolRequest {
        tool_call_id: ToolCallId::new(tool_call_id),
        session_id: SessionId::new("session-wrapper"),
        run_id: RunId::new(run_id),
        tool_id: tool_id.to_string(),
        capability_profile_id: "trusted-local-dev".to_string(),
        input,
    }
}

fn temp_root(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    std::env::temp_dir().join(format!("capo-tools-{name}-{nanos}"))
}

#[test]
fn confine_write_path_accepts_targets_under_the_workspace_and_rejects_escapes() {
    // RTL6: the shared path-containment engine. A write target under the
    // confined workspace is accepted (existing or not-yet-created); a target
    // escaping the workspace via `..` or an unrelated absolute path is rejected.
    let workspace = temp_root("confine-workspace");
    fs::create_dir_all(workspace.join("src")).expect("workspace src");
    fs::write(workspace.join("src/lib.rs"), b"contents").expect("seed file");
    let canonical_workspace = workspace.canonicalize().expect("canonical workspace");

    // Existing file under the workspace -> confined to its canonical path.
    let existing =
        confine_write_path(Path::new("src/lib.rs"), &workspace).expect("existing confined file");
    assert!(existing.starts_with(&canonical_workspace));

    // Not-yet-created file under the workspace -> accepted (allow-missing).
    let new_file =
        confine_write_path(Path::new("src/new_module.rs"), &workspace).expect("new confined file");
    assert!(new_file.starts_with(&canonical_workspace));

    // A `..`-escape is rejected.
    assert!(
        confine_write_path(Path::new("../outside.txt"), &workspace).is_err(),
        "parent-traversal escape must be rejected"
    );

    // A NOT-YET-CREATED target with interior `..` that escapes the workspace via
    // a non-existent intermediate dir must be rejected. `src/sub` does not exist,
    // so the nearest-existing-ancestor walk used to skip past the `..` segments
    // and accept the escape; the lexical normalization closes that bypass.
    assert!(
        confine_write_path(Path::new("src/sub/../../../escape.txt"), &workspace).is_err(),
        "a deep `..`-escape through a non-existent intermediate must be rejected"
    );
    // A confined interior `..` that stays under the workspace is still accepted
    // and the returned path is normalized (no `..` segments).
    let folded = confine_write_path(Path::new("src/sub/../kept.rs"), &workspace)
        .expect("interior `..` that stays confined is accepted");
    assert!(folded.starts_with(&canonical_workspace));
    assert!(
        !folded.components().any(|c| matches!(
            c,
            std::path::Component::ParentDir | std::path::Component::CurDir
        )),
        "returned confined path must be normalized, got {}",
        folded.display()
    );

    // A credential-like component is rejected anywhere in the path, matching the
    // live provider's `normalize_policy_path` rule (single containment engine).
    assert!(
        confine_write_path(Path::new(".ssh/id_rsa"), &workspace).is_err(),
        "credential-like components must be rejected"
    );

    // An unrelated absolute path outside the workspace is rejected.
    let outside = temp_root("confine-outside");
    fs::create_dir_all(&outside).expect("outside dir");
    assert!(
        confine_write_path(&outside.join("file.txt"), &workspace).is_err(),
        "an absolute path outside the workspace must be rejected"
    );
}

#[cfg(unix)]
#[test]
fn confine_write_path_accepts_a_target_reached_through_a_symlinked_workspace_prefix() {
    // RTL6 regression: on macOS `/tmp` is a symlink to `/private/tmp`, so a
    // workspace root handed in as `/tmp/ws` canonicalizes to `/private/tmp/ws`.
    // A write target reached through that same symlinked prefix must still
    // confine: the engine must compare the symlink-RESOLVED candidate against
    // the resolved workspace root, not the raw `/tmp/...` form (which lexically
    // is not "under" `/private/tmp/...`). We model this with a real symlinked
    // directory standing in for `/tmp`.
    use std::os::unix::fs::symlink;

    let real_parent = temp_root("confine-symlinked-prefix-real");
    fs::create_dir_all(&real_parent).expect("real parent");
    let link_parent = temp_root("confine-symlinked-prefix-link");
    symlink(&real_parent, &link_parent).expect("symlink standing in for /tmp");

    // Workspace root addressed THROUGH the symlink (like `/tmp/ws`).
    let workspace_via_link = link_parent.join("ws");
    fs::create_dir_all(workspace_via_link.join("src")).expect("workspace via link");
    fs::write(workspace_via_link.join("src/lib.rs"), b"contents").expect("seed");
    let canonical_workspace = workspace_via_link
        .canonicalize()
        .expect("canonical workspace");

    // Existing file: the engine canonicalizes through the symlink and confines.
    let existing = confine_write_path(Path::new("src/lib.rs"), &workspace_via_link)
        .expect("existing file under a symlinked-prefix workspace root must confine");
    assert!(existing.starts_with(&canonical_workspace));

    // Not-yet-created file: same — confines via the canonical ancestor and the
    // returned path is symlink-resolved (under the canonical workspace root).
    let new_file = confine_write_path(Path::new("src/new.rs"), &workspace_via_link)
        .expect("new file under a symlinked-prefix workspace root must confine");
    assert!(
        new_file.starts_with(&canonical_workspace),
        "returned path must be confined to the canonical workspace, got {}",
        new_file.display()
    );

    // The workspace root itself (target == root, as the pre-write checkpoint
    // passes it) confines to the canonical root.
    let root_target = confine_write_path(&workspace_via_link, &workspace_via_link)
        .expect("the workspace root itself must confine");
    assert_eq!(root_target, canonical_workspace);

    // A `..`-escape through the symlinked prefix is still rejected.
    assert!(
        confine_write_path(Path::new("../escape.txt"), &workspace_via_link).is_err(),
        "a `..`-escape must be rejected even through a symlinked workspace prefix"
    );
}

#[cfg(unix)]
#[test]
fn confine_write_path_rejects_symlinked_prefix_escaping_the_workspace() {
    // A symlinked directory under the workspace that points outside must not let
    // a write escape confinement.
    use std::os::unix::fs::symlink;

    let workspace = temp_root("confine-symlink-workspace");
    fs::create_dir_all(&workspace).expect("workspace");
    let outside = temp_root("confine-symlink-outside");
    fs::create_dir_all(&outside).expect("outside");
    symlink(&outside, workspace.join("escape")).expect("symlink into outside");

    assert!(
        confine_write_path(Path::new("escape/file.txt"), &workspace).is_err(),
        "a write through a symlink that escapes the workspace must be rejected"
    );
}

// --- ACI1: real tool dispatch, fake routing killed ------------------------

#[test]
#[should_panic(expected = "fake-only summary shim")]
fn tool_exposure_invoke_no_longer_routes_capo_to_the_fake() {
    // ACI1: the load-bearing dead-routing was `ToolExposure::invoke` sending the
    // `Capo` variant to `FakeToolExposure`. The real `Capo` exposure must NOT be
    // reachable through the fake summary shim anymore -- it must dispatch through
    // `authorize_and_invoke` instead. Routing it through `invoke` is now a wiring
    // bug and panics.
    let exposure = ToolExposure::capo();
    let _ = exposure.invoke(FakeToolRequest {
        tool_call_id: ToolCallId::new("call-capo"),
        session_id: SessionId::new("session-tools"),
        tool_name: "capo.session_summary".to_string(),
        input_summary: "should not route to fake".to_string(),
    });
}

#[test]
#[should_panic(expected = "fake-only summary shim")]
fn tool_exposure_invoke_no_longer_routes_runtime_to_the_fake() {
    let exposure = ToolExposure::runtime_wrappers(RuntimeToolConfig::local_workspace(
        temp_root("aci1-invoke-runtime-ws"),
        temp_root("aci1-invoke-runtime-artifacts"),
    ));
    let _ = exposure.invoke(FakeToolRequest {
        tool_call_id: ToolCallId::new("call-runtime"),
        session_id: SessionId::new("session-tools"),
        tool_name: "capo.file_read".to_string(),
        input_summary: "should not route to fake".to_string(),
    });
}

#[test]
fn tool_exposure_fake_invoke_remains_available_for_the_test_only_variant() {
    // The fake summary shim stays reachable through the explicit, test-only
    // `Fake` variant -- it is just no longer the default for the real variants.
    let exposure = ToolExposure::fake();
    let result = exposure.invoke(FakeToolRequest {
        tool_call_id: ToolCallId::new("call-fake"),
        session_id: SessionId::new("session-tools"),
        tool_name: "capo.session_summary".to_string(),
        input_summary: "summarize".to_string(),
    });
    assert_eq!(result.tool_name, "capo.session_summary");
}

#[test]
fn tool_exposure_authorize_and_invoke_dispatches_the_real_capo_registry() {
    // ACI1: the typed dispatch routes `Capo` requests into the REAL
    // `CapoToolRegistry::authorize_and_invoke`, emitting the real audit lifecycle
    // (not a fabricated fake observation).
    let exposure = ToolExposure::capo();
    let result = exposure.authorize_and_invoke(
        ToolExposureRequest::Capo(CapoToolRequest {
            tool_call_id: ToolCallId::new("call-capo-real"),
            session_id: SessionId::new("session-tools"),
            tool_id: "capo.session_summary".to_string(),
            capability_profile_id: "trusted-local-dev".to_string(),
            context: tool_context(),
        }),
        &PermissionPolicy::allow_trusted_local(),
    );
    let ToolExposureResult::Capo(result) = result else {
        panic!("Capo request must produce a Capo result");
    };
    assert_eq!(result.permission_decision.effect, "allow");
    assert_eq!(result.output, "summary text");
    assert!(
        result
            .events
            .iter()
            .any(|event| event.kind == "tool.invocation_started")
    );
}

#[test]
fn tool_exposure_authorize_and_invoke_dispatches_the_real_runtime_wrappers() {
    let workspace = temp_root("aci1-dispatch-runtime-ws");
    let artifacts = temp_root("aci1-dispatch-runtime-artifacts");
    fs::create_dir_all(&workspace).expect("workspace");
    fs::write(workspace.join("note.md"), "hello aci1").expect("seed file");
    let exposure = ToolExposure::runtime_wrappers(RuntimeToolConfig::local_workspace(
        workspace.clone(),
        artifacts,
    ));
    let result = exposure.authorize_and_invoke(
        ToolExposureRequest::Runtime(wrapper_request(
            "call-runtime-real",
            "run-runtime-real",
            "capo.file_read",
            serde_json::json!({"path":"note.md"}),
        )),
        &PermissionPolicy::allow_trusted_local(),
    );
    let ToolExposureResult::Runtime(result) = result else {
        panic!("Runtime request must produce a Runtime result");
    };
    assert_eq!(result.status, "completed");
    assert_eq!(result.output_artifacts.len(), 1);
    assert_eq!(
        fs::read_to_string(&result.output_artifacts[0].uri).expect("artifact"),
        "hello aci1"
    );
}

#[test]
#[should_panic(expected = "variant mismatch")]
fn tool_exposure_authorize_and_invoke_rejects_a_cross_variant_request() {
    // A `Runtime` request against the `Capo` exposure is a wiring bug; it must be
    // rejected, never silently downgraded to the fake path.
    let exposure = ToolExposure::capo();
    let _ = exposure.authorize_and_invoke(
        ToolExposureRequest::Runtime(wrapper_request(
            "call-mismatch",
            "run-mismatch",
            "capo.file_read",
            serde_json::json!({"path":"note.md"}),
        )),
        &PermissionPolicy::allow_trusted_local(),
    );
}

// --- ACI2: per-tool input AND output schemas plus risk/scope/redaction ------

#[test]
fn every_registered_tool_declares_output_schema_risk_scope_and_redaction() {
    // ACI2: every tool in CAPO_OWNED_TOOLS and CAPO_WRAPPER_TOOLS must declare a
    // non-empty output_schema, non-empty required_scopes_json, a valid risk
    // level, and a non-empty redaction_policy_json -- present and checkable
    // rather than convention.
    let registry = CapoToolRegistry;
    let wrappers = RuntimeToolWrappers::new(RuntimeToolConfig::local_workspace(
        PathBuf::from("/tmp/capo-aci2-workspace"),
        PathBuf::from("/tmp/capo-aci2-artifacts"),
    ));

    let definitions = registry
        .list_tools()
        .into_iter()
        .chain(wrappers.list_tools())
        .collect::<Vec<_>>();

    assert_eq!(
        definitions.len(),
        CAPO_OWNED_TOOLS.len() + CAPO_WRAPPER_TOOLS.len()
    );

    for definition in &definitions {
        let tool_id = &definition.tool_id;

        // output_schema present, non-empty, and a well-formed `{"output":{...}}`.
        assert!(
            !definition.output_schema.trim().is_empty(),
            "{tool_id} must declare a non-empty output_schema"
        );
        let schema: Value = serde_json::from_str(&definition.output_schema)
            .unwrap_or_else(|error| panic!("{tool_id} output_schema must be json: {error}"));
        let output = schema
            .get("output")
            .and_then(Value::as_object)
            .unwrap_or_else(|| panic!("{tool_id} output_schema must carry an `output` object"));
        assert!(
            !output.is_empty(),
            "{tool_id} output_schema must describe at least one field"
        );

        // required_scopes non-empty and includes the tool-invoke scope.
        let scopes: Value = serde_json::from_str(&definition.required_scopes_json)
            .unwrap_or_else(|error| panic!("{tool_id} required_scopes_json must be json: {error}"));
        let scopes = scopes
            .as_array()
            .unwrap_or_else(|| panic!("{tool_id} required_scopes_json must be an array"));
        assert!(
            !scopes.is_empty(),
            "{tool_id} must declare non-empty required_scopes_json"
        );

        // risk present and one of the tool-exposure.md levels.
        assert!(
            definition.risk_is_valid(),
            "{tool_id} risk `{}` must be one of {:?}",
            definition.risk,
            TOOL_RISK_LEVELS
        );

        // redaction_policy present, non-empty, and well-formed json.
        assert!(
            !definition.redaction_policy_json.trim().is_empty(),
            "{tool_id} must declare a non-empty redaction_policy_json"
        );
        let policy: Value =
            serde_json::from_str(&definition.redaction_policy_json).unwrap_or_else(|error| {
                panic!("{tool_id} redaction_policy_json must be json: {error}")
            });
        assert!(
            policy.get("strategy").and_then(Value::as_str).is_some(),
            "{tool_id} redaction_policy_json must declare a strategy"
        );
    }
}

#[test]
fn wrapper_risk_levels_reconcile_with_tool_exposure() {
    // ACI2: risk stays aligned with the tool-exposure.md assignments.
    let wrappers = RuntimeToolWrappers::new(RuntimeToolConfig::local_workspace(
        PathBuf::from("/tmp/capo-aci2-risk-workspace"),
        PathBuf::from("/tmp/capo-aci2-risk-artifacts"),
    ));
    let risk = |tool_id: &str| {
        wrappers
            .describe_tool(tool_id)
            .unwrap_or_else(|| panic!("{tool_id} definition"))
            .risk
    };
    assert_eq!(risk("capo.shell_run"), "high");
    assert_eq!(risk("capo.git_commit"), "high");
    assert_eq!(risk("capo.file_write"), "medium");
    assert_eq!(risk("capo.git_status"), "low");
    assert_eq!(risk("capo.file_read"), "low");
}

#[test]
fn capo_registry_emitted_results_validate_against_their_output_schema() {
    // ACI2: each Capo tool's emitted result must validate against its declared
    // output_schema, so "narrow typed output" is checkable rather than
    // convention.
    let registry = CapoToolRegistry;
    let policy = PermissionPolicy::allow_trusted_local();
    let context = tool_context();

    for tool_id in CAPO_OWNED_TOOLS {
        let definition = registry.describe_tool(tool_id).expect("tool definition");
        let call_id = ToolCallId::new(format!("call-aci2-{tool_id}"));
        let result = registry.authorize_and_invoke(
            CapoToolRequest {
                tool_call_id: call_id.clone(),
                session_id: SessionId::new("session-aci2"),
                tool_id: tool_id.to_string(),
                capability_profile_id: "trusted-local-dev".to_string(),
                context: context.clone(),
            },
            &policy,
        );

        // Validate the *live* result fields the handler populated, not just the
        // re-typed projection: a rendered, non-empty `output` and a real
        // recorded `output_artifact_id` keyed to this call. This is what makes
        // the schema↔result coupling observable rather than a structural
        // tautology that can only break if projection and schema drift together.
        assert!(
            !result.output.trim().is_empty(),
            "{tool_id} must render a non-empty output, got {:?}",
            result.output
        );
        assert!(
            result.output_artifact_id.contains(call_id.as_str()),
            "{tool_id} must record a real output artifact for this call, got {:?}",
            result.output_artifact_id
        );

        let errors = definition.validate_output(&result.narrow_output());
        assert!(
            errors.is_empty(),
            "{tool_id} emitted result must validate against output_schema, got {errors:?}"
        );
    }
}

#[test]
fn wrapper_emitted_results_validate_against_their_output_schema() {
    // ACI2: each wrapper tool's emitted result validates against its declared
    // output_schema over a real fixture workspace.
    let workspace = temp_root("aci2-wrapper-output-workspace");
    let artifacts = temp_root("aci2-wrapper-output-artifacts");
    fs::create_dir_all(&workspace).expect("workspace");
    Command::new("git")
        .args(["init"])
        .current_dir(&workspace)
        .output()
        .expect("git init");
    fs::write(workspace.join("note.md"), "hello aci2").expect("seed note");
    let wrappers = RuntimeToolWrappers::new(RuntimeToolConfig::local_workspace(
        workspace.clone(),
        artifacts,
    ));
    let policy = PermissionPolicy::allow_trusted_local();

    let cases = [
        (
            "capo.file_read",
            "call-aci2-read",
            "run-aci2-read",
            serde_json::json!({"path":"note.md"}),
        ),
        (
            "capo.git_status",
            "call-aci2-status",
            "run-aci2-status",
            serde_json::json!({}),
        ),
        (
            "capo.shell_run",
            "call-aci2-shell",
            "run-aci2-shell",
            serde_json::json!({"program":"/bin/sh","argv":["-c","echo hi"],"cwd":"."}),
        ),
        (
            "capo.file_write",
            "call-aci2-write",
            "run-aci2-write",
            serde_json::json!({"path":"out.txt","content":"written"}),
        ),
    ];

    for (tool_id, call_id, run_id, input) in cases {
        let definition = wrappers.describe_tool(tool_id).expect("wrapper definition");
        let result = wrappers
            .authorize_and_invoke(wrapper_request(call_id, run_id, tool_id, input), &policy);
        assert_ne!(
            result.status, "denied",
            "{tool_id} should be allowed under trusted-local"
        );
        let errors = definition.validate_output(&result.narrow_output());
        assert!(
            errors.is_empty(),
            "{tool_id} emitted result must validate against output_schema, got {errors:?}"
        );
    }
}

#[test]
fn output_schema_validation_rejects_a_wrong_shaped_result() {
    // ACI2: the validator is a real check, not a rubber stamp -- a result that
    // is missing a required field or has the wrong type fails validation.
    let registry = CapoToolRegistry;
    let definition = registry
        .describe_tool("capo.session_summary")
        .expect("definition");

    // Missing both required fields.
    let missing = definition.validate_output(&serde_json::json!({}));
    assert!(
        missing
            .iter()
            .any(|error| error.contains("missing required field `output`")),
        "missing output must be reported, got {missing:?}"
    );

    // Wrong type for a declared field.
    let wrong_type = definition.validate_output(&serde_json::json!({
        "output": 7,
        "output_artifact_id": "artifact-1",
    }));
    assert!(
        wrong_type
            .iter()
            .any(|error| error.contains("field `output` expected `string`")),
        "wrong-typed output must be reported, got {wrong_type:?}"
    );
}
