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

    assert_eq!(tools.len(), 8);
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
    assert!(
        fs::read_to_string(&write.output_artifacts[0].uri)
            .expect("diff summary")
            .contains("before=fnv1a64:")
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
        let result = registry.authorize_and_invoke(
            CapoToolRequest {
                tool_call_id: ToolCallId::new(format!("call-aci2-{tool_id}")),
                session_id: SessionId::new("session-aci2"),
                tool_id: tool_id.to_string(),
                capability_profile_id: "trusted-local-dev".to_string(),
                context: context.clone(),
            },
            &policy,
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
