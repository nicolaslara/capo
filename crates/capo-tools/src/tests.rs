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

    assert_eq!(tools.len(), 11);
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
fn file_read_redacts_a_configured_secret_in_the_output_artifact() {
    // ACI7: redaction is enforced on OUTPUT, not only input. A configured secret
    // pattern sitting in a file the agent reads must be scrubbed in the artifact
    // the read produced.
    let workspace = temp_root("tool-redact-output-workspace");
    let artifacts = temp_root("tool-redact-output-artifacts");
    fs::create_dir_all(&workspace).expect("workspace");
    fs::write(
        workspace.join("config.env"),
        "DB_PASSWORD=SUPERSECRET\nname=ok\n",
    )
    .expect("seed file");

    let mut config = RuntimeToolConfig::local_workspace(workspace.clone(), artifacts);
    config.redaction_rules.push(RedactionRule {
        pattern: "SUPERSECRET".to_string(),
        replacement: "[REDACTED]".to_string(),
    });
    let wrappers = RuntimeToolWrappers::new(config);
    let policy = PermissionPolicy::allow_trusted_local();

    let read = wrappers.authorize_and_invoke(
        wrapper_request(
            "call-read-secret",
            "run-read-secret",
            "capo.file_read",
            serde_json::json!({ "path": "config.env" }),
        ),
        &policy,
    );
    assert_eq!(read.status, "completed");
    let output = read.output_artifacts.first().expect("read output artifact");
    // The OUTPUT artifact is marked redacted and the secret is scrubbed on disk.
    assert_eq!(output.redaction_state, "redacted");
    let on_disk = fs::read_to_string(&output.uri).expect("read output artifact");
    assert!(
        !on_disk.contains("SUPERSECRET"),
        "secret leaked into output artifact: {on_disk}"
    );
    assert!(on_disk.contains("[REDACTED]"));
    // The benign content survives the scrub.
    assert!(on_disk.contains("name=ok"));
}

#[test]
fn file_read_credential_shape_scan_redacts_an_unnamed_secret_in_output() {
    // ACI7: the default credential-shape scan catches a secret the operator did
    // NOT name as a pattern -- the common case for tool output. With no
    // configured rule, a credential-shaped token in the read file is still
    // scrubbed in the artifact, while ordinary prose survives.
    let workspace = temp_root("tool-credential-scan-workspace");
    let artifacts = temp_root("tool-credential-scan-artifacts");
    fs::create_dir_all(&workspace).expect("workspace");
    let aws_key = "AKIAIOSFODNN7EXAMPLE";
    fs::write(
        workspace.join("notes.txt"),
        format!("aws key {aws_key}\nthis is ordinary documentation prose\n"),
    )
    .expect("seed file");

    // No RedactionRule configured: only the default credential-shape scan runs.
    let wrappers = RuntimeToolWrappers::new(RuntimeToolConfig::local_workspace(
        workspace.clone(),
        artifacts,
    ));
    let policy = PermissionPolicy::allow_trusted_local();

    let read = wrappers.authorize_and_invoke(
        wrapper_request(
            "call-read-key",
            "run-read-key",
            "capo.file_read",
            serde_json::json!({ "path": "notes.txt" }),
        ),
        &policy,
    );
    assert_eq!(read.status, "completed");
    let output = read.output_artifacts.first().expect("read output artifact");
    assert_eq!(output.redaction_state, "redacted");
    let on_disk = fs::read_to_string(&output.uri).expect("read output artifact");
    assert!(
        !on_disk.contains(aws_key),
        "unnamed credential leaked into output artifact: {on_disk}"
    );
    assert!(on_disk.contains("[REDACTED:credential]"));
    // The credential scan must not blank out ordinary prose.
    assert!(on_disk.contains("ordinary documentation prose"));
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

// --- SG4: TrustedLocal critical-scope exclusion ----------------------------

/// SG4: the enumerated critical scopes from `capability-permissions.md` that
/// TrustedLocal must DENY without an explicit grant: source-write outside the
/// workspace, network egress (connect + public expose), secret/credential read,
/// and arbitrary shell.
const SG4_CRITICAL_SCOPES: &[&str] = &[
    "filesystem:write:path",
    "network:connect:internet",
    "network:expose:public",
    "secret:read:credential_material",
    "shell:execute:path",
];

fn sg4_trusted_local_decision(policy: &PermissionPolicy, scope: &str) -> PermissionDecision {
    policy.decide(PermissionRequest {
        session_id: SessionId::new("session-sg4"),
        capability_profile_id: "trusted-local-dev".to_string(),
        scope_json: json_array(vec![scope]),
    })
}

#[test]
fn sg4_trusted_local_denies_each_ungranted_critical_scope() {
    // SG4: under the default TrustedLocal policy (no explicit critical grants),
    // EVERY enumerated critical scope is denied -- one assertion per scope.
    let policy = PermissionPolicy::allow_trusted_local();
    for scope in SG4_CRITICAL_SCOPES {
        let decision = sg4_trusted_local_decision(&policy, scope);
        assert_eq!(
            decision.effect, "deny",
            "critical scope `{scope}` must be denied under default TrustedLocal"
        );
        assert_eq!(decision.decision_source, "allow_trusted_local_profile");
        assert_eq!(decision.persistence, "once");
        assert!(
            decision.explanation.contains(scope),
            "denial explanation should name the critical scope `{scope}`, got: {}",
            decision.explanation
        );
        assert!(
            decision
                .capability_grant_id
                .starts_with("grant-session-sg4-deny-"),
            "deny decision should mint a deny-keyed grant id, got: {}",
            decision.capability_grant_id
        );
    }
}

#[test]
fn sg4_critical_scope_classifier_covers_enumerated_scopes() {
    // SG4: the classifier flags exactly the enumerated critical scopes and treats
    // the workspace-scoped variants as non-critical.
    assert_eq!(
        critical_scope_kind("filesystem:write:path"),
        Some(CriticalScope::SourceWriteOutsideWorkspace)
    );
    assert_eq!(
        critical_scope_kind("network:connect:internet"),
        Some(CriticalScope::NetworkEgress)
    );
    assert_eq!(
        critical_scope_kind("network:expose:public"),
        Some(CriticalScope::NetworkEgress)
    );
    assert_eq!(
        critical_scope_kind("secret:read:credential_material"),
        Some(CriticalScope::SecretRead)
    );
    assert_eq!(
        critical_scope_kind("shell:execute:path"),
        Some(CriticalScope::ArbitraryShell)
    );
    // Non-critical: workspace-scoped writes/shell, git, secret metadata, tool calls.
    for non_critical in [
        "filesystem:write:workspace",
        "filesystem:read:workspace",
        "shell:execute:workspace",
        "git:status:workspace",
        "git:diff:workspace",
        "network:connect:localhost",
        "secret:read:provider_metadata",
        "tool:invoke:capo.file_write",
    ] {
        assert_eq!(
            critical_scope_kind(non_critical),
            None,
            "`{non_critical}` must be treated as non-critical"
        );
    }
}

#[test]
fn sg4_trusted_local_still_allows_non_critical_workspace_request() {
    // SG4: ordinary local work (workspace write, git diff, Capo tool invocation)
    // keeps the audit-only allow with the SAME durable decision shape.
    let policy = PermissionPolicy::allow_trusted_local();
    let decision = policy.decide(PermissionRequest {
        session_id: SessionId::new("session-sg4"),
        capability_profile_id: "trusted-local-dev".to_string(),
        scope_json: json_array(vec![
            "tool:invoke:capo.file_write",
            "filesystem:write:workspace",
            "git:diff:workspace",
        ]),
    });
    assert_eq!(decision.effect, "allow");
    assert_eq!(decision.decision_source, "allow_trusted_local_profile");
    assert_eq!(decision.persistence, "until_session_end");
    assert!(
        decision
            .capability_grant_id
            .starts_with("grant-session-sg4-allow-")
    );
    assert!(decision.explanation.contains("audited local prototype"));
}

#[test]
fn sg4_trusted_local_denies_when_critical_mixed_with_non_critical() {
    // SG4: a request that bundles a non-critical scope with an un-granted critical
    // scope is denied as a whole -- the critical scope is not laundered through.
    let policy = PermissionPolicy::allow_trusted_local();
    let decision = policy.decide(PermissionRequest {
        session_id: SessionId::new("session-sg4"),
        capability_profile_id: "trusted-local-dev".to_string(),
        scope_json: json_array(vec![
            "filesystem:read:workspace",
            "network:connect:internet",
        ]),
    });
    assert_eq!(decision.effect, "deny");
    assert!(decision.explanation.contains("network:connect:internet"));
}

#[test]
fn sg4_explicit_grant_re_admits_critical_scope() {
    // SG4: with an explicit grant present, the SAME critical-scope request that the
    // default policy denies now ALLOWS -- one assertion per enumerated scope. A
    // critical scope NOT in the grant set is still denied.
    for scope in SG4_CRITICAL_SCOPES {
        let granted = PermissionPolicy::allow_trusted_local_with_grants([scope.to_string()]);
        let allowed = sg4_trusted_local_decision(&granted, scope);
        assert_eq!(
            allowed.effect, "allow",
            "explicitly granted critical scope `{scope}` must allow"
        );
        assert_eq!(allowed.decision_source, "allow_trusted_local_profile");
        assert_eq!(allowed.persistence, "until_session_end");

        // A different critical scope, not in this grant set, stays denied.
        let other = SG4_CRITICAL_SCOPES
            .iter()
            .find(|candidate| {
                **candidate != *scope
                    && critical_scope_kind(candidate) != critical_scope_kind(scope)
            })
            .expect("another critical scope of a different category exists");
        let still_denied = sg4_trusted_local_decision(&granted, other);
        assert_eq!(
            still_denied.effect, "deny",
            "un-granted critical scope `{other}` must stay denied when only `{scope}` is granted"
        );
    }
}

#[test]
fn sg4_trusted_local_fails_closed_on_malformed_scope_json() {
    // SG4: malformed scope json is a deny (fail-closed), not a blanket allow.
    let policy = PermissionPolicy::allow_trusted_local();
    let decision = policy.decide(PermissionRequest {
        session_id: SessionId::new("session-sg4"),
        capability_profile_id: "trusted-local-dev".to_string(),
        scope_json: "{\"filesystem:write:path\":true}".to_string(),
    });
    assert_eq!(decision.effect, "deny");
    assert_eq!(decision.decision_source, "allow_trusted_local_profile");
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
fn input_artifact_write_failure_yields_failed_result_not_panic() {
    // Followup FP2: `record_input_artifact` runs unconditionally before
    // `execute`. A write failure there (e.g. an unwritable artifact root) must
    // flow through the same failure arm the output path uses -- a failed
    // `WrapperToolResult` -- rather than panicking the controller mid-dispatch.
    let workspace = temp_root("fp2-input-artifact-fail-workspace");
    fs::create_dir_all(&workspace).expect("workspace");
    fs::write(workspace.join("note.md"), "hello fp2").expect("seed");

    // Make the artifact ROOT a regular FILE. `write_tool_artifact` then calls
    // `create_dir_all(artifact_root/<tool_call_id>)`, which deterministically
    // fails because a path component (the artifact root) is a file, not a
    // directory -- forcing the input-artifact write to error before `execute`.
    let artifacts = temp_root("fp2-input-artifact-fail-artifacts-file");
    fs::write(&artifacts, b"not a directory").expect("seed artifact-root file");

    let wrappers = RuntimeToolWrappers::new(RuntimeToolConfig::local_workspace(
        workspace.clone(),
        artifacts.clone(),
    ));
    let policy = PermissionPolicy::allow_trusted_local();

    // `authorize_and_invoke` must RETURN a failed result (no panic). A clean
    // read tool is chosen so the only failure source is the input artifact.
    let result = wrappers.authorize_and_invoke(
        wrapper_request(
            "call-fp2-input-fail",
            "run-fp2-input-fail",
            "capo.file_read",
            serde_json::json!({"path": "note.md"}),
        ),
        &policy,
    );

    assert_eq!(
        result.status, "failed",
        "an input-artifact write failure must yield a failed result, not a panic"
    );
    // The failed call records no artifacts: the input write never landed, and
    // execution was never reached.
    assert!(
        result.input_artifact.is_none(),
        "a failed input-artifact write records no input artifact"
    );
    assert!(
        result.output_artifacts.is_empty(),
        "a pre-execution failure produces no output artifacts"
    );
    // It flows through the non-completed audit shape (tool.call_failed), never
    // the success sequence (tool.call_completed).
    let event_names: Vec<&str> = result
        .events
        .iter()
        .map(|event| event.kind.as_str())
        .collect();
    assert!(
        event_names.contains(&"tool.call_failed"),
        "a failed input-artifact write must emit tool.call_failed, got {event_names:?}"
    );
    assert!(
        !event_names.contains(&"tool.call_completed"),
        "a failed input-artifact write must NOT emit tool.call_completed, got {event_names:?}"
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

/// Seed a fixture repo with `n` lines each containing the needle, plus a second
/// file in a subdirectory, so search caps/truncation can be exercised
/// deterministically. Returns the workspace path.
fn seed_search_fixture(name: &str, needle: &str, lines: usize) -> PathBuf {
    let workspace = temp_root(name);
    fs::create_dir_all(workspace.join("sub")).expect("workspace sub");
    let mut body = String::new();
    for n in 1..=lines {
        body.push_str(&format!("line {n} has {needle} here\n"));
    }
    fs::write(workspace.join("a.txt"), body).expect("seed a.txt");
    fs::write(
        workspace.join("sub/b.txt"),
        format!("{needle} in a subdir\n"),
    )
    .expect("seed sub/b.txt");
    workspace
}

#[test]
fn search_returns_typed_bounded_path_line_preview_matches() {
    // ACI5: a clean search returns decision-grade `path:line:preview` matches,
    // not whole files, and validates against its declared output_schema.
    let workspace = seed_search_fixture("aci5-search-clean", "needle", 3);
    let artifacts = temp_root("aci5-search-clean-artifacts");
    let wrappers = RuntimeToolWrappers::new(RuntimeToolConfig::local_workspace(
        workspace.clone(),
        artifacts,
    ));
    let result = wrappers.authorize_and_invoke(
        wrapper_request(
            "call-aci5-clean",
            "run-aci5-clean",
            "capo.search",
            serde_json::json!({"query": "needle"}),
        ),
        &PermissionPolicy::allow_trusted_local(),
    );
    assert_eq!(result.status, "completed", "summary: {}", result.summary);

    let definition = wrappers.describe_tool("capo.search").expect("definition");
    let errors = definition.validate_output(&result.narrow_output());
    assert!(errors.is_empty(), "search typed output: {errors:?}");

    let typed = result.narrow_output();
    // 3 matches in a.txt + 1 in sub/b.txt.
    assert_eq!(typed["total_matches"], serde_json::json!(4));
    assert_eq!(typed["returned_matches"], serde_json::json!(4));
    assert_eq!(typed["truncated"], serde_json::json!(false));
    assert_eq!(typed["truncation_reason"], serde_json::json!("none"));
    let matches = typed["matches"].as_array().expect("matches");
    assert_eq!(matches.len(), 4);
    // Each match is a decision-grade path:line:preview triple, never a whole file.
    for one in matches {
        assert!(one["path"].as_str().is_some(), "match carries a path");
        assert!(one["line"].as_i64().is_some(), "match carries a line");
        let preview = one["preview"].as_str().expect("preview");
        assert!(preview.contains("needle"), "preview shows the matched line");
        assert!(!preview.contains('\n'), "preview is a single line");
    }
    // The full file content is never inlined as a single blob: there is no
    // output artifact for a bounded search.
    assert!(result.output_artifacts.is_empty());
}

#[test]
fn search_per_call_match_cap_truncates_with_explicit_marker() {
    // ACI5: the per-call match cap bounds the number of returned matches and the
    // result carries an explicit truncation marker so the agent knows it is
    // partial rather than silently incomplete.
    let workspace = seed_search_fixture("aci5-search-matchcap", "needle", 20);
    let artifacts = temp_root("aci5-search-matchcap-artifacts");
    let wrappers = RuntimeToolWrappers::new(RuntimeToolConfig::local_workspace(
        workspace.clone(),
        artifacts,
    ));
    let result = wrappers.authorize_and_invoke(
        wrapper_request(
            "call-aci5-matchcap",
            "run-aci5-matchcap",
            "capo.search",
            serde_json::json!({"query": "needle", "max_matches": 5}),
        ),
        &PermissionPolicy::allow_trusted_local(),
    );
    assert_eq!(result.status, "completed", "summary: {}", result.summary);
    let typed = result.narrow_output();
    assert_eq!(typed["returned_matches"], serde_json::json!(5));
    // 20 in a.txt + 1 in sub/b.txt were found before capping.
    assert_eq!(typed["total_matches"], serde_json::json!(21));
    assert_eq!(typed["truncated"], serde_json::json!(true));
    assert_eq!(typed["truncation_reason"], serde_json::json!("match_cap"));
    assert_eq!(typed["matches"].as_array().expect("matches").len(), 5);
}

#[test]
fn search_total_byte_cap_truncates_with_explicit_marker() {
    // ACI5: even under the match cap, the total preview BYTE cap bounds the
    // payload so the tool cannot dump large amounts of content, and the result
    // is explicitly marked truncated via the byte cap.
    let workspace = seed_search_fixture("aci5-search-bytecap", "needle", 20);
    let artifacts = temp_root("aci5-search-bytecap-artifacts");
    let wrappers = RuntimeToolWrappers::new(RuntimeToolConfig::local_workspace(
        workspace.clone(),
        artifacts,
    ));
    let result = wrappers.authorize_and_invoke(
        wrapper_request(
            "call-aci5-bytecap",
            "run-aci5-bytecap",
            "capo.search",
            // A high match cap but a tiny byte budget: the byte cap fires first.
            serde_json::json!({"query": "needle", "max_matches": 100, "max_preview_bytes": 40}),
        ),
        &PermissionPolicy::allow_trusted_local(),
    );
    assert_eq!(result.status, "completed", "summary: {}", result.summary);
    let typed = result.narrow_output();
    assert_eq!(typed["truncated"], serde_json::json!(true));
    assert_eq!(typed["truncation_reason"], serde_json::json!("byte_cap"));
    // The total preview bytes returned stay within the budget.
    let total_preview_bytes: usize = typed["matches"]
        .as_array()
        .expect("matches")
        .iter()
        .map(|one| one["preview"].as_str().unwrap_or("").len())
        .sum();
    assert!(
        total_preview_bytes <= 40,
        "total preview bytes {total_preview_bytes} must stay within the 40-byte cap"
    );
    // The byte cap fired strictly before the match cap, so fewer than the 21
    // available matches were returned.
    assert!(
        typed["returned_matches"].as_i64().expect("returned") < 21,
        "byte cap must return fewer than all matches"
    );
}

#[test]
fn search_empty_result_is_a_successful_not_failed_call() {
    // ACI5: ripgrep exits 1 on no matches; that is a normal empty search, not a
    // tool failure.
    let workspace = seed_search_fixture("aci5-search-empty", "needle", 3);
    let artifacts = temp_root("aci5-search-empty-artifacts");
    let wrappers = RuntimeToolWrappers::new(RuntimeToolConfig::local_workspace(
        workspace.clone(),
        artifacts,
    ));
    let result = wrappers.authorize_and_invoke(
        wrapper_request(
            "call-aci5-empty",
            "run-aci5-empty",
            "capo.search",
            serde_json::json!({"query": "zzz_no_such_token_zzz"}),
        ),
        &PermissionPolicy::allow_trusted_local(),
    );
    assert_eq!(result.status, "completed", "summary: {}", result.summary);
    let typed = result.narrow_output();
    assert_eq!(typed["total_matches"], serde_json::json!(0));
    assert_eq!(typed["returned_matches"], serde_json::json!(0));
    assert_eq!(typed["truncated"], serde_json::json!(false));
}

#[test]
fn search_redacts_secrets_in_previews() {
    // ACI5: a configured secret on a matched line must be scrubbed in the preview
    // before it reaches the agent.
    let workspace = temp_root("aci5-search-redact");
    fs::create_dir_all(&workspace).expect("workspace");
    fs::write(
        workspace.join("config.txt"),
        "api_key = SECRET_TOKEN_abc123\nother line\n",
    )
    .expect("seed");
    let mut config =
        RuntimeToolConfig::local_workspace(workspace.clone(), temp_root("aci5-redact-art"));
    config.redaction_rules.push(RedactionRule {
        pattern: "SECRET_TOKEN_abc123".to_string(),
        replacement: "[REDACTED]".to_string(),
    });
    let wrappers = RuntimeToolWrappers::new(config);
    let result = wrappers.authorize_and_invoke(
        wrapper_request(
            "call-aci5-redact",
            "run-aci5-redact",
            "capo.search",
            serde_json::json!({"query": "api_key"}),
        ),
        &PermissionPolicy::allow_trusted_local(),
    );
    assert_eq!(result.status, "completed", "summary: {}", result.summary);
    let typed = result.narrow_output();
    assert!(
        !typed.to_string().contains("SECRET_TOKEN_abc123"),
        "secret leaked into search previews: {typed}"
    );
    let preview = typed["matches"][0]["preview"].as_str().expect("preview");
    assert!(
        preview.contains("[REDACTED]"),
        "the redacted placeholder must replace the secret in the preview: {preview}"
    );
}

#[test]
fn search_cannot_read_outside_the_workspace() {
    // ACI5: search reads stay inside the workspace via the shared path
    // confinement -- a `..`/absolute path root escape is rejected.
    let workspace = seed_search_fixture("aci5-search-escape", "needle", 2);
    let artifacts = temp_root("aci5-search-escape-artifacts");
    let wrappers = RuntimeToolWrappers::new(RuntimeToolConfig::local_workspace(
        workspace.clone(),
        artifacts,
    ));
    let escaped = wrappers.authorize_and_invoke(
        wrapper_request(
            "call-aci5-escape",
            "run-aci5-escape",
            "capo.search",
            serde_json::json!({"query": "needle", "path": "../../../etc"}),
        ),
        &PermissionPolicy::allow_trusted_local(),
    );
    assert_eq!(
        escaped.status, "failed",
        "a workspace-escaping search root must be rejected, got: {}",
        escaped.summary
    );
}

// --- ACI6: typed test/check tool --------------------------------------------

#[test]
fn test_run_passing_command_returns_typed_passed_record_with_timing() {
    // ACI6: capo.test_run runs a check command and returns the typed
    // {command, exit_status, passed, failing_items, duration_ms,
    // output_artifact_id} record, validating against the declared output_schema.
    // A passing command has no failing items and records wall-clock timing.
    let workspace = temp_root("aci6-test-pass-workspace");
    let artifacts = temp_root("aci6-test-pass-artifacts");
    fs::create_dir_all(&workspace).expect("workspace");
    let wrappers = RuntimeToolWrappers::new(RuntimeToolConfig::local_workspace(
        workspace.clone(),
        artifacts,
    ));
    let policy = PermissionPolicy::allow_trusted_local();

    let result = wrappers.authorize_and_invoke(
        wrapper_request(
            "call-test-pass",
            "run-test-pass",
            "capo.test_run",
            serde_json::json!({
                "program": "/bin/sh",
                "argv": ["-c", "echo 'test mod::ok ... ok'; exit 0"],
                "cwd": "."
            }),
        ),
        &policy,
    );
    assert_eq!(result.status, "exited", "summary: {}", result.summary);

    let definition = wrappers.describe_tool("capo.test_run").expect("definition");
    let errors = definition.validate_output(&result.narrow_output());
    assert!(
        errors.is_empty(),
        "test_run typed output must validate, got {errors:?}"
    );

    let typed = result.narrow_output();
    assert_eq!(typed["exit_status"], serde_json::json!(0));
    assert_eq!(typed["passed"], serde_json::json!(true));
    assert_eq!(
        typed["command"].as_str().expect("command"),
        "/bin/sh -c echo 'test mod::ok ... ok'; exit 0"
    );
    assert_eq!(
        typed["failing_items"]
            .as_array()
            .expect("failing_items")
            .len(),
        0,
        "a passing command has no failing items"
    );
    assert_eq!(typed["failing_items_total"], serde_json::json!(0));
    assert!(typed["duration_ms"].is_i64(), "duration_ms is recorded");
    let started = typed["started_at"].as_i64().expect("started_at");
    let completed = typed["completed_at"].as_i64().expect("completed_at");
    assert!(started > 0, "started_at is a wall-clock timestamp");
    assert!(
        completed >= started,
        "completed_at ({completed}) must not precede started_at ({started})"
    );
}

#[test]
fn test_run_failing_command_captures_bounded_failing_items_and_full_artifact() {
    // ACI6: a failing command surfaces the failing test names in `failing_items`
    // (bounded), is classified `passed:false`, and the FULL output lives in a
    // redacted artifact rather than inline.
    let workspace = temp_root("aci6-test-fail-workspace");
    let artifacts = temp_root("aci6-test-fail-artifacts");
    fs::create_dir_all(&workspace).expect("workspace");
    let wrappers = RuntimeToolWrappers::new(RuntimeToolConfig::local_workspace(
        workspace.clone(),
        artifacts,
    ));
    let policy = PermissionPolicy::allow_trusted_local();

    let result = wrappers.authorize_and_invoke(
        wrapper_request(
            "call-test-fail",
            "run-test-fail",
            "capo.test_run",
            serde_json::json!({
                "program": "/bin/sh",
                "argv": [
                    "-c",
                    "echo 'test mod::a ... ok'; echo 'test mod::b ... FAILED'; echo 'test mod::c ... FAILED'; exit 101"
                ],
                "cwd": "."
            }),
        ),
        &policy,
    );
    // The tool DID run the command and produce a complete typed evidence record;
    // the observed process `status` is "failed" because the run under test exited
    // non-zero (consistent with capo.shell_run). The decision-grade signal is
    // `passed:false` + the failing_items, NOT a tool error -- this is still a
    // completed call that delivered evidence.
    assert_eq!(result.status, "failed", "summary: {}", result.summary);
    assert!(
        result
            .events
            .iter()
            .any(|event| event.kind == "tool.call_completed"),
        "a test_run that produced evidence is a COMPLETED call even when the run failed"
    );

    let definition = wrappers.describe_tool("capo.test_run").expect("definition");
    let errors = definition.validate_output(&result.narrow_output());
    assert!(
        errors.is_empty(),
        "test_run failing typed output must validate, got {errors:?}"
    );

    let typed = result.narrow_output();
    assert_eq!(typed["exit_status"], serde_json::json!(101));
    assert_eq!(typed["passed"], serde_json::json!(false));
    let failing = typed["failing_items"].as_array().expect("failing_items");
    assert_eq!(failing.len(), 2, "two failing test names: {failing:?}");
    assert_eq!(failing[0], serde_json::json!("mod::b"));
    assert_eq!(failing[1], serde_json::json!("mod::c"));
    assert_eq!(typed["failing_items_total"], serde_json::json!(2));

    // The FULL output is in the artifact, including the passing line that is NOT
    // surfaced inline -- inline stays decision-grade, the artifact is complete.
    let artifact_id = typed["output_artifact_id"].as_str().expect("artifact id");
    let artifact = result
        .output_artifacts
        .iter()
        .find(|artifact| artifact.artifact_id == artifact_id)
        .expect("output artifact referenced by typed output");
    let full = fs::read_to_string(&artifact.uri).expect("output artifact");
    assert!(full.contains("test mod::a ... ok"));
    assert!(full.contains("test mod::b ... FAILED"));
    // No redaction rule matched this output, so the provenance is honestly "safe"
    // (the runner-computed state) rather than a hardcoded "redacted".
    assert_eq!(artifact.redaction_state, "safe");
}

#[test]
fn test_run_caps_inline_failing_items_with_explicit_truncation_marker() {
    // ACI6: the inline failing_items list is bounded -- a run with many failures
    // returns at most the cap plus an explicit elision marker, with the full set
    // counted in failing_items_total and the full log in the artifact, so the
    // tool never dumps the whole log inline.
    let workspace = temp_root("aci6-test-cap-workspace");
    let artifacts = temp_root("aci6-test-cap-artifacts");
    fs::create_dir_all(&workspace).expect("workspace");
    let wrappers = RuntimeToolWrappers::new(RuntimeToolConfig::local_workspace(
        workspace.clone(),
        artifacts,
    ));
    let policy = PermissionPolicy::allow_trusted_local();

    // 12 failing tests but a max_failing_items cap of 3.
    let result = wrappers.authorize_and_invoke(
        wrapper_request(
            "call-test-cap",
            "run-test-cap",
            "capo.test_run",
            serde_json::json!({
                "program": "/bin/sh",
                "argv": [
                    "-c",
                    "for n in $(seq 1 12); do echo \"test mod::case$n ... FAILED\"; done; exit 1"
                ],
                "cwd": ".",
                "max_failing_items": 3
            }),
        ),
        &policy,
    );
    // Non-zero exit under test -> observed `status` "failed", but a completed
    // evidence call (see the failing-command test).
    assert_eq!(result.status, "failed", "summary: {}", result.summary);
    let typed = result.narrow_output();
    assert_eq!(typed["passed"], serde_json::json!(false));
    assert_eq!(typed["failing_items_total"], serde_json::json!(12));
    assert_eq!(
        typed["failing_items_truncated"],
        serde_json::json!(true),
        "an over-cap failing set must be flagged truncated"
    );
    let failing = typed["failing_items"].as_array().expect("failing_items");
    // 3 capped names + 1 explicit elision marker line.
    assert_eq!(failing.len(), 4, "capped to 3 plus an elision marker");
    assert!(
        failing[3]
            .as_str()
            .expect("marker")
            .contains("more failing item(s) elided"),
        "the last item is an explicit elision marker: {failing:?}"
    );

    // The full set is still in the artifact.
    let artifact_id = typed["output_artifact_id"].as_str().expect("artifact id");
    let artifact = result
        .output_artifacts
        .iter()
        .find(|artifact| artifact.artifact_id == artifact_id)
        .expect("output artifact");
    let full = fs::read_to_string(&artifact.uri).expect("output artifact");
    assert!(full.contains("test mod::case12 ... FAILED"));
}

#[test]
fn test_run_redacts_secrets_in_the_output_artifact() {
    // ACI6: secrets in the command output are scrubbed in the redacted artifact
    // before they reach the agent/gate.
    let workspace = temp_root("aci6-test-redact-workspace");
    let artifacts = temp_root("aci6-test-redact-artifacts");
    fs::create_dir_all(&workspace).expect("workspace");
    let mut config = RuntimeToolConfig::local_workspace(workspace.clone(), artifacts);
    config.redaction_rules.push(RedactionRule {
        pattern: "SECRET_TOKEN_xyz789".to_string(),
        replacement: "[REDACTED]".to_string(),
    });
    let wrappers = RuntimeToolWrappers::new(config);
    let policy = PermissionPolicy::allow_trusted_local();

    let result = wrappers.authorize_and_invoke(
        wrapper_request(
            "call-test-redact",
            "run-test-redact",
            "capo.test_run",
            serde_json::json!({
                "program": "/bin/sh",
                "argv": ["-c", "echo 'leaked SECRET_TOKEN_xyz789 in output'; exit 0"],
                "cwd": "."
            }),
        ),
        &policy,
    );
    assert_eq!(result.status, "exited", "summary: {}", result.summary);
    let typed = result.narrow_output();
    let artifact_id = typed["output_artifact_id"].as_str().expect("artifact id");
    let artifact = result
        .output_artifacts
        .iter()
        .find(|artifact| artifact.artifact_id == artifact_id)
        .expect("output artifact");
    let full = fs::read_to_string(&artifact.uri).expect("output artifact");
    assert!(
        !full.contains("SECRET_TOKEN_xyz789"),
        "secret leaked into the test_run output artifact: {full}"
    );
    assert!(full.contains("[REDACTED]"));
}

#[test]
fn test_run_redacts_secrets_in_inline_failing_items() {
    // ACI6: a secret printed on a recognized FAILING line must be scrubbed in the
    // inline `failing_items` (the field the redaction policy declares), not just
    // in the output artifact. The wrapper establishes the redaction seam at its
    // own boundary (mirroring `capo.search`) rather than relying on the runner.
    let workspace = temp_root("aci6-failing-items-redact-workspace");
    let artifacts = temp_root("aci6-failing-items-redact-artifacts");
    fs::create_dir_all(&workspace).expect("workspace");
    let mut config = RuntimeToolConfig::local_workspace(workspace.clone(), artifacts);
    config.redaction_rules.push(RedactionRule {
        pattern: "SECRET_TOKEN_xyz789".to_string(),
        replacement: "[REDACTED]".to_string(),
    });
    let wrappers = RuntimeToolWrappers::new(config);
    let policy = PermissionPolicy::allow_trusted_local();

    // A failing pytest-shaped line carrying a secret in the test id; this is what
    // would surface in the clear via the first-N-lines fallback or the parsed
    // failing-test name if the inline list were built from un-redacted output.
    let result = wrappers.authorize_and_invoke(
        wrapper_request(
            "call-failing-redact",
            "run-failing-redact",
            "capo.test_run",
            serde_json::json!({
                "program": "/bin/sh",
                "argv": [
                    "-c",
                    "echo 'FAILED tests/test_db.py::test_conn[SECRET_TOKEN_xyz789]'; exit 1"
                ],
                "cwd": "."
            }),
        ),
        &policy,
    );
    assert_eq!(result.status, "failed", "summary: {}", result.summary);
    let typed = result.narrow_output();
    assert_eq!(typed["passed"], serde_json::json!(false));
    let failing = serde_json::to_string(&typed["failing_items"]).expect("failing_items json");
    assert!(
        !failing.contains("SECRET_TOKEN_xyz789"),
        "secret leaked into inline failing_items: {failing}"
    );
    assert!(
        failing.contains("[REDACTED]"),
        "expected redacted marker in failing_items: {failing}"
    );
}

#[test]
fn test_run_clean_output_is_labeled_safe() {
    // ACI6: redaction provenance must be honest. A run with no matching secret is
    // labeled `redaction_state == "safe"`, not hardcoded "redacted" -- otherwise
    // the gate/audit consuming this record gets misleading evidence.
    let workspace = temp_root("aci6-clean-safe-workspace");
    let artifacts = temp_root("aci6-clean-safe-artifacts");
    fs::create_dir_all(&workspace).expect("workspace");
    let mut config = RuntimeToolConfig::local_workspace(workspace.clone(), artifacts);
    // A redaction rule is configured but the command output never matches it.
    config.redaction_rules.push(RedactionRule {
        pattern: "SECRET_TOKEN_xyz789".to_string(),
        replacement: "[REDACTED]".to_string(),
    });
    let wrappers = RuntimeToolWrappers::new(config);
    let policy = PermissionPolicy::allow_trusted_local();

    let result = wrappers.authorize_and_invoke(
        wrapper_request(
            "call-clean-safe",
            "run-clean-safe",
            "capo.test_run",
            serde_json::json!({
                "program": "/bin/sh",
                "argv": ["-c", "echo 'all good, nothing sensitive here'; exit 0"],
                "cwd": "."
            }),
        ),
        &policy,
    );
    assert_eq!(result.status, "exited", "summary: {}", result.summary);
    let typed = result.narrow_output();
    let artifact_id = typed["output_artifact_id"].as_str().expect("artifact id");
    let artifact = result
        .output_artifacts
        .iter()
        .find(|artifact| artifact.artifact_id == artifact_id)
        .expect("output artifact");
    assert_eq!(
        artifact.redaction_state, "safe",
        "clean run should be labeled safe, got {}",
        artifact.redaction_state
    );
}

// --- ACI8: GO2 agent-reporting / evidence tools ----------------------------

fn report_request(tool_call_id: &str, tool_id: &str, body: Value) -> AgentReportRequest {
    AgentReportRequest {
        tool_call_id: ToolCallId::new(tool_call_id),
        session_id: SessionId::new("session-aci8"),
        tool_id: tool_id.to_string(),
        capability_profile_id: "trusted-local-dev".to_string(),
        confidence: 70,
        body,
        submission_id: None,
    }
}

#[test]
fn agent_report_registry_registers_every_go2_reporting_tool() {
    // ACI8: each GO2 reporting tool is registered in the typed registry, per
    // workpads/goal-orchestration/tasks.md:86-104.
    assert_eq!(CAPO_REPORTING_TOOLS.len(), 11);
    for expected in [
        "capo.report_intent",
        "capo.report_progress",
        "capo.record_evidence",
        "capo.report_confidence",
        "capo.record_assumption",
        "capo.raise_blocker",
        "capo.request_review",
        "capo.record_review",
        "capo.record_validation",
        "capo.complete_requirement",
        "capo.complete_subtask",
    ] {
        assert!(
            CAPO_REPORTING_TOOLS.contains(&expected),
            "{expected} must be a GO2 reporting tool"
        );
    }

    let registry = AgentReportRegistry;
    let tools = registry.list_tools();
    assert_eq!(tools.len(), CAPO_REPORTING_TOOLS.len());
    for tool_id in CAPO_REPORTING_TOOLS {
        let definition = registry
            .describe_tool(tool_id)
            .expect("reporting definition");
        assert_eq!(definition.origin, "capo");
        assert_eq!(definition.handler_kind, "agent_report");
        assert!(
            definition
                .required_scopes_json
                .contains(&format!("tool:invoke:{tool_id}"))
        );
    }
}

#[test]
fn every_reporting_tool_declares_schema_scopes_risk_redaction_and_mutates_state() {
    // ACI8: each GO2 reporting tool declares schema / required_scopes / risk /
    // redaction_policy / mutates_state.
    let registry = AgentReportRegistry;
    // mutates_state per the GO2 acceptance: pure intent/progress/confidence/
    // assumption reports are observations; evidence/blocker/review/validation/
    // completion records mutate the autonomy ledger.
    let mutating = [
        "capo.record_evidence",
        "capo.raise_blocker",
        "capo.request_review",
        "capo.record_review",
        "capo.record_validation",
        "capo.complete_requirement",
        "capo.complete_subtask",
    ];
    for tool_id in CAPO_REPORTING_TOOLS {
        let definition = registry
            .describe_tool(tool_id)
            .expect("reporting definition");

        // schema present and a well-formed `{"input":{...}}`.
        let schema: Value = serde_json::from_str(&definition.schema_json)
            .unwrap_or_else(|error| panic!("{tool_id} schema_json must be json: {error}"));
        let input = schema
            .get("input")
            .and_then(Value::as_object)
            .unwrap_or_else(|| panic!("{tool_id} schema_json must carry an `input` object"));
        assert!(
            !input.is_empty(),
            "{tool_id} schema must describe at least one field"
        );

        // output_schema present and well-formed.
        assert!(
            !definition.output_schema.trim().is_empty(),
            "{tool_id} must declare a non-empty output_schema"
        );

        // required_scopes non-empty.
        let scopes: Value = serde_json::from_str(&definition.required_scopes_json)
            .unwrap_or_else(|error| panic!("{tool_id} required_scopes_json must be json: {error}"));
        assert!(
            scopes.as_array().is_some_and(|scopes| !scopes.is_empty()),
            "{tool_id} must declare non-empty required_scopes_json"
        );

        // risk one of the tool-exposure.md levels.
        assert!(
            definition.risk_is_valid(),
            "{tool_id} risk `{}` must be one of {TOOL_RISK_LEVELS:?}",
            definition.risk
        );

        // redaction_policy present, well-formed, with a strategy.
        let policy: Value =
            serde_json::from_str(&definition.redaction_policy_json).unwrap_or_else(|error| {
                panic!("{tool_id} redaction_policy_json must be json: {error}")
            });
        assert!(
            policy.get("strategy").and_then(Value::as_str).is_some(),
            "{tool_id} redaction_policy_json must declare a strategy"
        );

        // mutates_state matches the GO2 acceptance.
        assert_eq!(
            definition.mutates_state,
            mutating.contains(tool_id),
            "{tool_id} mutates_state classification mismatch"
        );
    }
}

#[test]
fn agent_report_is_stored_as_agent_reported_not_observed_evidence() {
    // ACI8 (the load-bearing test): an agent report is persisted as a DISTINCT
    // class tagged `agent_reported`, carrying confidence, and is NOT
    // indistinguishable from observed tool evidence -- completion is never
    // reachable by agent assertion alone.
    let registry = AgentReportRegistry;
    let policy = PermissionPolicy::allow_trusted_local();
    let record = registry.authorize_and_invoke(
        report_request(
            "call-report-complete",
            "capo.complete_requirement",
            serde_json::json!({"requirement_id": "REQ-1", "summary": "done"}),
        ),
        &policy,
    );

    // The distinct classification: an agent claim, never observed evidence.
    assert_eq!(record.source, EVIDENCE_SOURCE_AGENT_REPORTED);
    assert!(
        !record.is_observed_evidence(),
        "an agent report must never classify as observed evidence"
    );
    assert!(
        record.is_completion_claim(),
        "complete_requirement is a completion CLAIM"
    );
    assert!(record.accepted);
    assert_eq!(record.confidence, 70);

    // The classification helper agrees: the observed-evidence sources are
    // distinct from the agent-reported source.
    assert!(source_is_observed_evidence(EVIDENCE_SOURCE_RUNTIME_OUTPUT));
    assert!(source_is_observed_evidence(EVIDENCE_SOURCE_ADAPTER_EVENT));
    assert!(source_is_observed_evidence("adapter_event:codex"));
    assert!(
        !source_is_observed_evidence(EVIDENCE_SOURCE_AGENT_REPORTED),
        "agent_reported must not be classified as observed evidence"
    );

    // The emitted observation event carries the agent_reported source, distinct
    // from a `tool.output_observed` runtime-evidence event.
    assert!(
        record
            .events
            .iter()
            .any(|event| event.kind == "tool.observation_recorded"
                && event.status == EVIDENCE_SOURCE_AGENT_REPORTED),
        "report must emit a `tool.observation_recorded` event tagged agent_reported"
    );
    assert!(
        !record
            .events
            .iter()
            .any(|event| event.kind == "tool.output_observed"),
        "an agent report must NOT emit a runtime `tool.output_observed` evidence event"
    );

    // Narrow typed output validates against the declared schema.
    let definition = registry
        .describe_tool("capo.complete_requirement")
        .expect("definition");
    let errors = definition.validate_output(&record.narrow_output());
    assert!(
        errors.is_empty(),
        "agent report output must validate against its schema, got {errors:?}"
    );
}

#[test]
fn agent_report_dispatches_through_the_typed_tool_exposure() {
    // ACI8: the reporting surface is a real ToolExposure variant routed through
    // the same typed authorize_and_invoke dispatch as the other tools, never the
    // fake summary shim.
    let exposure = ToolExposure::agent_reports();
    assert_eq!(exposure.binding().variant, "capo-agent-reports");
    assert!(!exposure.binding().fake);

    let policy = PermissionPolicy::allow_trusted_local();
    let result = exposure.authorize_and_invoke(
        ToolExposureRequest::AgentReport(report_request(
            "call-report-intent",
            "capo.report_intent",
            serde_json::json!({"intent": "wire the GO2 tools"}),
        )),
        &policy,
    );
    let ToolExposureResult::AgentReport(record) = result else {
        panic!("agent-report request must dispatch to an agent-report result");
    };
    assert_eq!(record.source, EVIDENCE_SOURCE_AGENT_REPORTED);
    assert!(record.accepted);
}

#[test]
fn denied_agent_report_is_not_a_claim_of_record() {
    // ACI8 failure path: a report the permission policy rejects is recorded for
    // audit (accepted=false) but is not an accepted agent claim, and never emits
    // the observation event.
    let registry = AgentReportRegistry;
    // The read-only-local static profile does not allow `state:write:agent_report`.
    let policy = PermissionPolicy::static_read_only_local();
    let record = registry.authorize_and_invoke(
        report_request(
            "call-report-denied",
            "capo.report_progress",
            serde_json::json!({"summary": "halfway"}),
        ),
        &policy,
    );
    assert_eq!(record.permission_decision.effect, "deny");
    assert!(!record.accepted, "a denied report is not an accepted claim");
    assert!(
        !record
            .events
            .iter()
            .any(|event| event.kind == "tool.observation_recorded"),
        "a denied report must not record an agent_reported observation"
    );
    // Even denied, it is still classified as a report (a claim), never observed
    // evidence.
    assert!(!record.is_observed_evidence());
}

#[test]
fn duplicate_agent_report_submissions_dedupe_on_replay() {
    // ACI8: each report carries an idempotency key so duplicate submissions
    // dedupe on replay. A re-emitted identical report collapses to one ledger
    // entry; a distinct report stays distinct.
    let registry = AgentReportRegistry;
    let policy = PermissionPolicy::allow_trusted_local();

    // An explicit agent-supplied submission id is the authoritative key.
    let mut request = report_request(
        "call-report-evidence-a",
        "capo.record_evidence",
        serde_json::json!({"evidence": "cargo test green", "evidence_kind": "test"}),
    );
    request.submission_id = Some("sub-42".to_string());
    let first = registry.authorize_and_invoke(request.clone(), &policy);

    // A retried submission with the SAME submission id (even with a different
    // tool_call_id, as a replay would carry) keeps the same idempotency key.
    let mut retry = request.clone();
    retry.tool_call_id = ToolCallId::new("call-report-evidence-a-retry");
    let second = registry.authorize_and_invoke(retry, &policy);
    assert_eq!(
        first.idempotency_key, second.idempotency_key,
        "a retried identical submission must dedupe on the same key"
    );

    let mut ledger = AgentReportLedger::new();
    assert!(ledger.record(first.clone()), "first record is new");
    assert!(
        !ledger.record(second),
        "the duplicate submission must dedupe"
    );
    assert_eq!(
        ledger.len(),
        1,
        "duplicate report must collapse to one entry"
    );

    // A DISTINCT report (no submission id, different body) gets a distinct key
    // derived from session/tool/body, so it is not swallowed by the dedupe.
    let distinct = registry.authorize_and_invoke(
        report_request(
            "call-report-intent-b",
            "capo.report_intent",
            serde_json::json!({"intent": "a different intent"}),
        ),
        &policy,
    );
    assert_ne!(distinct.idempotency_key, first.idempotency_key);
    assert!(
        ledger.record(distinct),
        "a distinct report is newly recorded"
    );
    assert_eq!(ledger.len(), 2);

    // Every ledger entry is an agent-reported claim, never observed evidence.
    assert_eq!(ledger.agent_reported().len(), ledger.len());
}

#[test]
fn identical_keyless_reports_dedupe_but_different_bodies_stay_distinct() {
    // ACI8: with no submission id, the idempotency key is a stable hash over
    // session/tool/body, so a re-emitted identical report dedupes while a
    // different body stays distinct.
    let registry = AgentReportRegistry;
    let policy = PermissionPolicy::allow_trusted_local();

    let body = serde_json::json!({"assumption": "the workspace is a git repo"});
    let a = registry.authorize_and_invoke(
        report_request("call-assume-a", "capo.record_assumption", body.clone()),
        &policy,
    );
    let b = registry.authorize_and_invoke(
        report_request("call-assume-b", "capo.record_assumption", body),
        &policy,
    );
    assert_eq!(
        a.idempotency_key, b.idempotency_key,
        "identical keyless reports must share an idempotency key"
    );

    let c = registry.authorize_and_invoke(
        report_request(
            "call-assume-c",
            "capo.record_assumption",
            serde_json::json!({"assumption": "a different assumption"}),
        ),
        &policy,
    );
    assert_ne!(
        a.idempotency_key, c.idempotency_key,
        "a different body must produce a distinct idempotency key"
    );
}

// ---------------------------------------------------------------------------
// ACI10: deterministic, scripted fake tool implementations for replayable tests
// ---------------------------------------------------------------------------

/// A fake-wrappers config pointing at a NON-EXISTENT workspace/artifact root.
///
/// The whole point of the fakes is that they never spawn a process or touch
/// disk, so the config paths need not exist; using a fixed path keeps the test
/// deterministic and proves nothing is read/written.
fn fake_wrapper_config() -> RuntimeToolConfig {
    RuntimeToolConfig::local_workspace(
        PathBuf::from("/nonexistent/capo-fake-workspace"),
        PathBuf::from("/nonexistent/capo-fake-artifacts"),
    )
}

#[test]
fn fake_wrappers_clean_path_covers_every_wrapper_tool_with_schema_valid_output() {
    // ACI10: a deterministic fake produces a clean result for EVERY runtime
    // wrapper tool, shaped exactly like the real path: the canonical observed
    // audit sequence, an input + output artifact, and a typed output that
    // validates against the tool's own declared `output_schema`.
    let fake = FakeRuntimeToolWrappers::new(fake_wrapper_config());
    let policy = PermissionPolicy::allow_trusted_local();

    for tool_id in CAPO_WRAPPER_TOOLS {
        let definition = fake
            .describe_tool(tool_id)
            .expect("fake wrapper definition");
        let result = fake.authorize_and_invoke(
            wrapper_request(
                &format!("call-{tool_id}"),
                "run-fake-clean",
                tool_id,
                serde_json::json!({"path": "src/lib.rs"}),
            ),
            &policy,
            ScriptedWrapperOutcome::ok(b"fake output bytes".to_vec()),
        );

        // The fake emits the SAME canonical completed audit sequence as the
        // real path -- the dispatch/projection layer is shape-driven, so a fake
        // must drive it identically.
        let kinds: Vec<&str> = result.events.iter().map(|e| e.kind.as_str()).collect();
        for expected in [
            "tool.call_requested",
            "permission.requested",
            "permission.decided",
            "capability.grant_used",
            "tool.invocation_started",
            "tool.output_artifact_recorded",
            "tool.output_observed",
            "tool.call_completed",
            "tool.result_delivered",
        ] {
            assert!(
                kinds.contains(&expected),
                "fake {tool_id} clean result missing audit event {expected}; got {kinds:?}"
            );
        }

        // Input + output artifacts exist, carry a redaction state, and are NOT
        // written to disk (a `fake://` uri).
        let input_artifact = result.input_artifact.as_ref().expect("fake input artifact");
        assert!(input_artifact.uri.starts_with("fake://"));
        let output_artifact = result
            .output_artifacts
            .first()
            .expect("fake output artifact");
        assert!(output_artifact.uri.starts_with("fake://"));
        assert!(
            !output_artifact.redaction_state.is_empty(),
            "fake artifact must record a redaction_state"
        );

        // The typed output validates against the tool's own declared schema, so
        // "narrow typed output" holds on the fake path too.
        let errors = definition.validate_output(&result.narrow_output());
        assert!(
            errors.is_empty(),
            "fake {tool_id} typed output must validate against its output_schema: {errors:?}"
        );
    }
}

#[test]
fn fake_wrappers_results_are_deterministic_and_replay_identically() {
    // ACI10: two fake invocations of the same scripted call produce a
    // byte-identical result (no clock, no process, no disk), so a replay /
    // projection-rebuild test over a fake is stable.
    let fake = FakeRuntimeToolWrappers::new(fake_wrapper_config());
    let policy = PermissionPolicy::allow_trusted_local();

    let first = fake.authorize_and_invoke(
        wrapper_request(
            "call-det",
            "run-det",
            "capo.shell_run",
            serde_json::json!({}),
        ),
        &policy,
        ScriptedWrapperOutcome::ok(b"deterministic output".to_vec()),
    );
    let second = fake.authorize_and_invoke(
        wrapper_request(
            "call-det",
            "run-det",
            "capo.shell_run",
            serde_json::json!({}),
        ),
        &policy,
        ScriptedWrapperOutcome::ok(b"deterministic output".to_vec()),
    );
    assert_eq!(first, second, "fake result must be deterministic on replay");
    // Determinism includes the pinned timing -- never a wall clock.
    assert_eq!(first.typed_output["duration_ms"], FAKE_DURATION_MS);
}

#[test]
fn fake_shell_run_ran_but_failed_is_a_completed_call_carrying_evidence() {
    // ACI10 (failure path): a scripted command that COMPLETED but did not pass
    // (non-zero exit) is still a completed call -- it delivered a full evidence
    // record. `passed:false` + `status=failed` is the decision-grade signal, not
    // a tool error. Mirrors the real `capo.shell_run` semantics.
    let fake = FakeRuntimeToolWrappers::new(fake_wrapper_config());
    let policy = PermissionPolicy::allow_trusted_local();

    let result = fake.authorize_and_invoke(
        wrapper_request(
            "call-fail-cmd",
            "run-fail",
            "capo.shell_run",
            serde_json::json!({}),
        ),
        &policy,
        ScriptedWrapperOutcome::ran_but_failed(b"boom\n".to_vec()),
    );
    assert_eq!(result.status, "failed");
    assert_eq!(result.typed_output["passed"], serde_json::json!(false));
    assert_eq!(result.typed_output["exit_status"], serde_json::json!(1));
    // It still completed: it produced an artifact and a completed call event.
    assert!(
        result
            .events
            .iter()
            .any(|e| e.kind == "tool.call_completed")
    );
    assert!(!result.output_artifacts.is_empty());
}

#[test]
fn fake_wrapper_handler_failure_emits_the_failed_non_completed_shape() {
    // ACI10 (failure path): a scripted handler ERROR emits the same
    // non-completed `failed` shape as the real failure path -- no
    // `tool.call_completed`, a `tool.call_failed`, and a schema-valid typed
    // output carrying status `failed`.
    let fake = FakeRuntimeToolWrappers::new(fake_wrapper_config());
    let policy = PermissionPolicy::allow_trusted_local();
    let definition = fake.describe_tool("capo.file_read").expect("definition");

    let result = fake.authorize_and_invoke(
        wrapper_request(
            "call-err",
            "run-err",
            "capo.file_read",
            serde_json::json!({}),
        ),
        &policy,
        ScriptedWrapperOutcome::Failed {
            error: "file_read input requires string field `path`".to_string(),
        },
    );
    assert_eq!(result.status, "failed");
    assert!(
        !result
            .events
            .iter()
            .any(|e| e.kind == "tool.call_completed"),
        "a failed call must not emit tool.call_completed"
    );
    assert!(
        result.events.iter().any(|e| e.kind == "tool.call_failed"),
        "a failed call must emit tool.call_failed"
    );
    let errors = definition.validate_output(&result.narrow_output());
    assert!(
        errors.is_empty(),
        "failed typed output must validate: {errors:?}"
    );
    assert_eq!(result.typed_output["status"], serde_json::json!("failed"));
}

#[test]
fn fake_apply_patch_rejected_hunk_is_a_structured_retryable_no_match() {
    // ACI10 (failure path): a scripted rejected `apply_patch` hunk is a
    // STRUCTURED retryable no-match that wrote nothing, carrying the rejected
    // hunk index and reason -- the same shape as the real `no_match_execution`.
    let fake = FakeRuntimeToolWrappers::new(fake_wrapper_config());
    let policy = PermissionPolicy::allow_trusted_local();
    let definition = fake.describe_tool("capo.apply_patch").expect("definition");

    let result = fake.authorize_and_invoke(
        wrapper_request(
            "call-nomatch",
            "run-nomatch",
            "capo.apply_patch",
            serde_json::json!({"path": "src/lib.rs"}),
        ),
        &policy,
        ScriptedWrapperOutcome::NoMatch {
            rejected_hunk_index: 2,
            reject_reason: "no strategy located the search block".to_string(),
        },
    );
    assert_eq!(result.status, "no_match");
    assert!(
        result.output_artifacts.is_empty(),
        "a no-match writes nothing"
    );
    assert!(
        !result
            .events
            .iter()
            .any(|e| e.kind == "tool.call_completed")
    );
    assert_eq!(
        result.typed_output["rejected_hunk_index"],
        serde_json::json!(2)
    );
    assert_eq!(result.typed_output["status"], serde_json::json!("no_match"));
    let errors = definition.validate_output(&result.narrow_output());
    assert!(
        errors.is_empty(),
        "no_match typed output must validate: {errors:?}"
    );
}

#[test]
fn fake_file_write_precondition_mismatch_writes_nothing() {
    // ACI10 (failure path): a scripted `file_write` precondition mismatch is a
    // typed `precondition_failed` that did NOT write, carrying expected/actual
    // hashes, exactly like the real precondition guard.
    let fake = FakeRuntimeToolWrappers::new(fake_wrapper_config());
    let policy = PermissionPolicy::allow_trusted_local();
    let definition = fake.describe_tool("capo.file_write").expect("definition");

    let result = fake.authorize_and_invoke(
        wrapper_request(
            "call-precond",
            "run-precond",
            "capo.file_write",
            serde_json::json!({"path": "src/lib.rs", "content": "new"}),
        ),
        &policy,
        ScriptedWrapperOutcome::PreconditionFailed {
            expected_hash: "fnv1a64:1111111111111111".to_string(),
            actual_hash: "fnv1a64:2222222222222222".to_string(),
        },
    );
    assert_eq!(result.status, "precondition_failed");
    assert!(
        result.output_artifacts.is_empty(),
        "a precondition fail writes nothing"
    );
    assert_eq!(
        result.typed_output["expected_hash"],
        serde_json::json!("fnv1a64:1111111111111111")
    );
    assert_eq!(
        result.typed_output["actual_hash"],
        serde_json::json!("fnv1a64:2222222222222222")
    );
    let errors = definition.validate_output(&result.narrow_output());
    assert!(
        errors.is_empty(),
        "precondition typed output must validate: {errors:?}"
    );
}

#[test]
fn fake_wrapper_permission_denial_runs_no_handler() {
    // ACI10 (failure path): a permission DENIAL on the fake path behaves exactly
    // like the real path -- the real authorization phase denies the call, no
    // scripted outcome is applied, and the denied audit/typed-output shape is
    // emitted. A read-only profile denies `capo.file_write`.
    let fake = FakeRuntimeToolWrappers::new(fake_wrapper_config());
    let read_only = PermissionPolicy::static_read_only_local();
    let definition = fake.describe_tool("capo.file_write").expect("definition");

    let result = fake.authorize_and_invoke(
        wrapper_request(
            "call-deny",
            "run-deny",
            "capo.file_write",
            serde_json::json!({"path": "src/lib.rs", "content": "x"}),
        ),
        &read_only,
        // The scripted "clean" outcome must be IGNORED because the policy denies.
        ScriptedWrapperOutcome::ok(b"should-never-be-used".to_vec()),
    );
    assert_eq!(result.status, "denied");
    assert_eq!(result.permission_decision.effect, "deny");
    assert!(
        result.output_artifacts.is_empty(),
        "a denied call runs no handler"
    );
    assert!(
        result.input_artifact.is_none(),
        "a denied call records no input artifact"
    );
    assert!(
        result
            .events
            .iter()
            .any(|e| e.kind == "tool.call_canceled" && e.status == "permission_denied")
    );
    let errors = definition.validate_output(&result.narrow_output());
    assert!(
        errors.is_empty(),
        "denied typed output must validate: {errors:?}"
    );
}

#[test]
fn fake_wrapper_redacts_a_configured_secret_in_the_output_artifact() {
    // ACI10 + ACI7: the fake reuses the REAL wrapper redaction policy, so a
    // configured secret in scripted output is scrubbed in the artifact and the
    // recorded `redaction_state` is honest -- the redaction contract holds on the
    // fake path without a live process.
    let mut config = fake_wrapper_config();
    config.redaction_rules.push(RedactionRule {
        pattern: "SECRET".to_string(),
        replacement: "[REDACTED]".to_string(),
    });
    let fake = FakeRuntimeToolWrappers::new(config);
    let policy = PermissionPolicy::allow_trusted_local();

    let result = fake.authorize_and_invoke(
        wrapper_request(
            "call-redact",
            "run-redact",
            "capo.file_read",
            serde_json::json!({"path": "src/secrets.txt"}),
        ),
        &policy,
        ScriptedWrapperOutcome::ok(b"token=SECRET-value".to_vec()),
    );
    let artifact = result.output_artifacts.first().expect("output artifact");
    assert_eq!(
        artifact.redaction_state, "redacted",
        "a configured secret in fake output must be recorded as redacted"
    );
    // The content hash is over the REDACTED bytes; the typed output references
    // the redacted artifact, never the cleartext.
    assert_eq!(result.typed_output["content_hash"], artifact.content_hash);
}

#[test]
fn fake_runtime_wrappers_are_a_test_only_boundary() {
    // ACI1 reconciliation: the fake is clearly TEST-ONLY (its binding is marked
    // `fake`) and is a distinct boundary from the live wrappers, so it can never
    // become the default for a real `Runtime` dispatch.
    let fake = FakeRuntimeToolWrappers::new(fake_wrapper_config());
    let binding = fake.binding();
    assert_eq!(binding.kind, BoundaryKind::ToolExposure);
    assert!(
        binding.fake,
        "the fake wrappers binding must be marked fake"
    );

    // The real wrappers are NOT fake.
    let real = RuntimeToolWrappers::new(fake_wrapper_config());
    assert!(!real.binding().fake);
}

#[test]
fn fake_capo_registry_clean_and_denied_paths_match_the_real_shape() {
    // ACI10: the scripted Capo-registry fake produces a stable `CapoToolResult`
    // with the canonical completed audit sequence on the clean path, and the
    // real denial shape when the policy denies -- without a controller-assembled
    // live context.
    let fake = FakeCapoToolRegistry;
    let trusted = PermissionPolicy::allow_trusted_local();

    let clean = fake.authorize_and_invoke(
        CapoToolRequest {
            tool_call_id: ToolCallId::new("call-capo-fake"),
            session_id: SessionId::new("session-fake"),
            tool_id: "capo.agent_status".to_string(),
            capability_profile_id: "trusted-local-dev".to_string(),
            context: tool_context(),
        },
        &trusted,
        "scripted agent status output",
    );
    assert_eq!(clean.output, "scripted agent status output");
    assert_ne!(clean.output_artifact_id, "none");
    assert!(clean.events.iter().any(|e| e.kind == "tool.call_completed"));
    // The narrow output validates against the Capo registry output schema.
    let definition = CapoToolRegistry
        .describe_tool("capo.agent_status")
        .expect("definition");
    assert!(
        definition
            .validate_output(&clean.narrow_output())
            .is_empty()
    );

    // A read-only profile denies the mutating `capo.capability_request`.
    let denied = fake.authorize_and_invoke(
        CapoToolRequest {
            tool_call_id: ToolCallId::new("call-capo-deny"),
            session_id: SessionId::new("session-fake"),
            tool_id: "capo.capability_request".to_string(),
            capability_profile_id: "read-only-local".to_string(),
            context: tool_context(),
        },
        &PermissionPolicy::static_read_only_local(),
        "should-never-be-used",
    );
    assert_eq!(denied.permission_decision.effect, "deny");
    assert_eq!(denied.output_artifact_id, "none");
    assert!(denied.events.iter().any(|e| e.kind == "tool.call_canceled"));

    assert!(
        fake.binding().fake,
        "the fake capo registry must be marked fake"
    );
}

#[test]
fn fake_agent_report_registry_emits_agent_reported_claims_and_dedupes() {
    // ACI10 + ACI8: the agent-report fake delegates to the (already
    // deterministic) real registry, so a report is persisted as a distinct
    // `agent_reported` claim -- never observed evidence -- and a re-submitted
    // identical report dedupes on its idempotency key in the replayable ledger.
    let fake = FakeAgentReportRegistry::new();
    let policy = PermissionPolicy::allow_trusted_local();
    assert!(
        fake.binding().fake,
        "the fake report registry must be marked fake"
    );

    let record = fake.authorize_and_invoke(
        report_request(
            "call-fake-report",
            "capo.complete_subtask",
            serde_json::json!({"subtask_id": "ST1", "summary": "done"}),
        ),
        &policy,
    );
    assert_eq!(record.source, EVIDENCE_SOURCE_AGENT_REPORTED);
    assert!(
        !record.is_observed_evidence(),
        "an agent report is never observed evidence"
    );
    assert!(record.is_completion_claim());

    // A re-emitted identical report dedupes in the replayable ledger.
    let again = fake.authorize_and_invoke(
        report_request(
            "call-fake-report-2",
            "capo.complete_subtask",
            serde_json::json!({"subtask_id": "ST1", "summary": "done"}),
        ),
        &policy,
    );
    assert_eq!(record.idempotency_key, again.idempotency_key);

    let mut ledger = AgentReportLedger::new();
    assert!(ledger.record(record));
    assert!(
        !ledger.record(again),
        "an identical re-submission must dedupe on replay"
    );
    assert_eq!(ledger.len(), 1);
}

// ---------------------------------------------------------------------------
// ACI11: deterministic full-surface gate + input-AND-output redaction
// ---------------------------------------------------------------------------

/// ACI11: run a deterministic fake/scripted test for EVERY tool, on BOTH the
/// clean and a failure path, with NO live provider (no process spawn, no disk).
///
/// This is the consolidated ACI11 gate: it walks the whole tool surface --
/// every runtime wrapper (`CAPO_WRAPPER_TOOLS`), every Capo-owned registry tool
/// (`CAPO_OWNED_TOOLS`), and every GO2 reporting tool (`CAPO_REPORTING_TOOLS`)
/// -- through the deterministic fakes, asserting that each clean result is
/// schema-valid and each failure path produces the structured failure shape the
/// real loop reflects on. Because the fakes pin timing and never touch disk, the
/// result is replayable; the per-tool ACI4/ACI5/ACI6/ACI10 tests cover the
/// strategy-level behaviour, while this gate proves the whole surface is
/// deterministically exercisable for clean AND failure in one pass.
#[test]
fn aci11_every_tool_runs_deterministically_for_clean_and_failure_paths() {
    let trusted = PermissionPolicy::allow_trusted_local();

    // -- Every runtime wrapper, clean + failure. --
    let fake = FakeRuntimeToolWrappers::new(fake_wrapper_config());
    for tool_id in CAPO_WRAPPER_TOOLS {
        let definition = fake.describe_tool(tool_id).expect("wrapper definition");

        // Clean: completed, schema-valid, with input + output artifacts that were
        // never written to disk (a `fake://` uri).
        let clean = fake.authorize_and_invoke(
            wrapper_request(
                &format!("aci11-clean-{tool_id}"),
                "run-aci11-clean",
                tool_id,
                serde_json::json!({"path": "src/lib.rs"}),
            ),
            &trusted,
            ScriptedWrapperOutcome::ok(b"deterministic fake output".to_vec()),
        );
        assert!(
            clean.events.iter().any(|e| e.kind == "tool.call_completed"),
            "fake clean {tool_id} must complete"
        );
        assert!(
            clean
                .output_artifacts
                .iter()
                .all(|a| a.uri.starts_with("fake://")),
            "fake {tool_id} must not touch disk"
        );
        let clean_errors = definition.validate_output(&clean.narrow_output());
        assert!(
            clean_errors.is_empty(),
            "fake clean {tool_id} output must validate: {clean_errors:?}"
        );

        // Failure: a scripted handler error emits the non-completed `failed`
        // shape with a schema-valid typed output -- no `tool.call_completed`.
        let failed = fake.authorize_and_invoke(
            wrapper_request(
                &format!("aci11-fail-{tool_id}"),
                "run-aci11-fail",
                tool_id,
                serde_json::json!({}),
            ),
            &trusted,
            ScriptedWrapperOutcome::Failed {
                error: format!("scripted failure for {tool_id}"),
            },
        );
        assert_eq!(failed.status, "failed");
        assert!(
            !failed
                .events
                .iter()
                .any(|e| e.kind == "tool.call_completed"),
            "a failed {tool_id} must not complete"
        );
        assert!(
            failed.events.iter().any(|e| e.kind == "tool.call_failed"),
            "a failed {tool_id} must emit tool.call_failed"
        );
        let failed_errors = definition.validate_output(&failed.narrow_output());
        assert!(
            failed_errors.is_empty(),
            "fake failed {tool_id} output must validate: {failed_errors:?}"
        );
    }

    // -- Every Capo-owned registry tool, clean + denied (failure). --
    let capo_fake = FakeCapoToolRegistry;
    let read_only = PermissionPolicy::static_read_only_local();
    for tool_id in CAPO_OWNED_TOOLS {
        let definition = CapoToolRegistry
            .describe_tool(tool_id)
            .expect("capo definition");

        let clean = capo_fake.authorize_and_invoke(
            CapoToolRequest {
                tool_call_id: ToolCallId::new(format!("aci11-capo-clean-{tool_id}")),
                session_id: SessionId::new("session-aci11"),
                tool_id: tool_id.to_string(),
                capability_profile_id: "trusted-local-dev".to_string(),
                context: tool_context(),
            },
            &trusted,
            "scripted output",
        );
        assert!(clean.events.iter().any(|e| e.kind == "tool.call_completed"));
        assert!(
            definition
                .validate_output(&clean.narrow_output())
                .is_empty(),
            "fake clean capo {tool_id} output must validate"
        );

        // Failure path: a read-only profile denies a mutating tool; a read tool
        // is still allowed, so only mutating tools exercise the deny branch here.
        if definition.mutates_state {
            let denied = capo_fake.authorize_and_invoke(
                CapoToolRequest {
                    tool_call_id: ToolCallId::new(format!("aci11-capo-deny-{tool_id}")),
                    session_id: SessionId::new("session-aci11"),
                    tool_id: tool_id.to_string(),
                    capability_profile_id: "read-only-local".to_string(),
                    context: tool_context(),
                },
                &read_only,
                "should-never-be-used",
            );
            assert_eq!(
                denied.permission_decision.effect, "deny",
                "a read-only profile must deny mutating capo {tool_id}"
            );
            assert!(denied.events.iter().any(|e| e.kind == "tool.call_canceled"));
        }
    }

    // -- Every GO2 reporting tool runs deterministically as an `agent_reported`
    //    claim (never observed evidence). --
    let report_fake = FakeAgentReportRegistry::new();
    for tool_id in CAPO_REPORTING_TOOLS {
        let record = report_fake.authorize_and_invoke(
            report_request(
                &format!("aci11-report-{tool_id}"),
                tool_id,
                serde_json::json!({"summary": "scripted report"}),
            ),
            &trusted,
        );
        assert_eq!(
            record.source, EVIDENCE_SOURCE_AGENT_REPORTED,
            "GO2 {tool_id} must persist as an agent_reported claim"
        );
        assert!(
            !record.is_observed_evidence(),
            "GO2 {tool_id} is never observed evidence"
        );
    }
}

/// ACI11: a known secret supplied as tool INPUT and a known secret sitting in
/// tool OUTPUT are BOTH stripped from the persisted artifacts on disk.
///
/// This is the load-bearing redaction invariant: the secret never appears in
/// ANY artifact the wrappers write -- neither the input artifact (the recorded
/// request, redacted through the same policy) nor the output artifact (the read
/// content / the diff). A `file_write` carries the input secret in its
/// `content`; a subsequent `file_read` of a seeded file carries the output
/// secret. After both calls, the test scans every file under the artifact root
/// and asserts neither cleartext secret survives.
#[test]
fn aci11_known_secret_is_redacted_from_both_input_and_output_artifacts() {
    let workspace = temp_root("aci11-redact-workspace");
    let artifacts = temp_root("aci11-redact-artifacts");
    fs::create_dir_all(&workspace).expect("workspace");

    // The OUTPUT secret: it sits in a file the agent reads.
    let output_secret = "OUTPUT-SUPERSECRET-abc123";
    fs::write(
        workspace.join("config.env"),
        format!("DB_PASSWORD={output_secret}\nname=ok\n"),
    )
    .expect("seed output secret");

    // The INPUT secret: the agent passes it as `file_write` content.
    let input_secret = "INPUT-SUPERSECRET-xyz789";

    let mut config = RuntimeToolConfig::local_workspace(workspace.clone(), artifacts.clone());
    config.redaction_rules.push(RedactionRule {
        pattern: output_secret.to_string(),
        replacement: "[REDACTED]".to_string(),
    });
    config.redaction_rules.push(RedactionRule {
        pattern: input_secret.to_string(),
        replacement: "[REDACTED]".to_string(),
    });
    let wrappers = RuntimeToolWrappers::new(config);
    let policy = PermissionPolicy::allow_trusted_local();

    // 1) file_write: the input secret rides in the request input -> input artifact.
    let write = wrappers.authorize_and_invoke(
        wrapper_request(
            "aci11-write-secret",
            "run-aci11-write",
            "capo.file_write",
            serde_json::json!({"path": "out.txt", "content": format!("token={input_secret}\n")}),
        ),
        &policy,
    );
    assert_eq!(write.status, "completed", "summary: {}", write.summary);

    // 2) file_read: the output secret rides in the read content -> output artifact.
    let read = wrappers.authorize_and_invoke(
        wrapper_request(
            "aci11-read-secret",
            "run-aci11-read",
            "capo.file_read",
            serde_json::json!({"path": "config.env"}),
        ),
        &policy,
    );
    assert_eq!(read.status, "completed");

    // The recorded artifacts are marked redacted (the policy matched).
    assert_eq!(
        write
            .input_artifact
            .as_ref()
            .expect("write input")
            .redaction_state,
        "redacted",
        "the input secret must be redacted in the recorded input artifact"
    );
    assert_eq!(
        read.output_artifacts
            .first()
            .expect("read output")
            .redaction_state,
        "redacted",
        "the output secret must be redacted in the read output artifact"
    );

    // The strongest assertion: scan EVERY persisted artifact file and prove
    // NEITHER cleartext secret survives anywhere on disk.
    let mut scanned = 0usize;
    for path in walk_files(&artifacts) {
        let bytes = fs::read(&path).expect("read artifact");
        let text = String::from_utf8_lossy(&bytes);
        assert!(
            !text.contains(input_secret),
            "input secret leaked into persisted artifact {}",
            path.display()
        );
        assert!(
            !text.contains(output_secret),
            "output secret leaked into persisted artifact {}",
            path.display()
        );
        scanned += 1;
    }
    assert!(
        scanned > 0,
        "the wrappers must have persisted artifacts to scan"
    );

    // The benign neighbours survived the scrub (redaction is targeted, not a wipe).
    let read_artifact = fs::read_to_string(&read.output_artifacts[0].uri).expect("read output");
    assert!(read_artifact.contains("name=ok"));
}

/// Recursively collect every regular file under `root` (depth-first). A tiny
/// helper so the redaction gate can assert a secret is absent from EVERY
/// persisted artifact, not just the one it queried.
fn walk_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                files.push(path);
            }
        }
    }
    files
}
