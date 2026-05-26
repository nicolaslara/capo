use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use capo_core::RunId;
use capo_runtime::{
    LocalProcessConfig, LocalProcessOutcome, LocalProcessRequest, LocalProcessRunner,
    RedactionRule, RuntimeError,
};

use super::{ClaudeCodeAdapter, CodexExecAdapter, NormalizedAdapterKind};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalAdapterLaunchPlan {
    pub adapter_kind: NormalizedAdapterKind,
    pub provider_kind: String,
    pub credential_scope: String,
    pub program: String,
    pub argv: Vec<String>,
    pub workspace_root: PathBuf,
    pub artifact_root: PathBuf,
    pub env_allowlist: Vec<String>,
    pub redaction_rules: Vec<RedactionRule>,
    pub output_limit_bytes: usize,
    pub stdout_format: String,
    pub stderr_policy: String,
}

impl LocalAdapterLaunchPlan {
    pub fn runtime_config(&self) -> LocalProcessConfig {
        LocalProcessConfig {
            workspace_roots: vec![self.workspace_root.clone()],
            artifact_root: self.artifact_root.clone(),
            env_allowlist: self.env_allowlist.clone(),
            redaction_rules: self.redaction_rules.clone(),
            output_limit_bytes: self.output_limit_bytes,
        }
    }

    pub fn runtime_request(&self, run_id: RunId) -> LocalProcessRequest {
        LocalProcessRequest {
            run_id,
            program: self.program.clone(),
            argv: self.argv.clone(),
            cwd: self.workspace_root.clone(),
            env: HashMap::new(),
        }
    }

    pub fn assert_subscription_safe(&self) -> Result<(), String> {
        if self.credential_scope != "user_local_subscription" {
            return Err(format!(
                "unsupported credential scope for local subscription launch: {}",
                self.credential_scope
            ));
        }
        if self.env_allowlist.iter().any(|name| {
            let upper = name.to_ascii_uppercase();
            upper.contains("TOKEN")
                || upper.contains("KEY")
                || upper.contains("SECRET")
                || upper.contains("COOKIE")
        }) {
            return Err(
                "local subscription launch env allowlist includes secret-like names".into(),
            );
        }
        if self.argv.iter().any(|arg| sensitive_marker(arg).is_some()) {
            return Err("local subscription launch argv includes secret-like markers".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalAdapterSmokePlan {
    pub adapter_kind: NormalizedAdapterKind,
    pub opt_in_env: &'static str,
    pub program: String,
    pub argv: Vec<String>,
    pub workspace_root: PathBuf,
    pub artifact_root: PathBuf,
    pub env_allowlist: Vec<String>,
    pub redaction_rules: Vec<RedactionRule>,
    pub output_limit_bytes: usize,
    pub expected_output_marker: &'static str,
}

impl LocalAdapterSmokePlan {
    pub fn runtime_config(&self) -> LocalProcessConfig {
        LocalProcessConfig {
            workspace_roots: vec![self.workspace_root.clone()],
            artifact_root: self.artifact_root.clone(),
            env_allowlist: self.env_allowlist.clone(),
            redaction_rules: self.redaction_rules.clone(),
            output_limit_bytes: self.output_limit_bytes,
        }
    }

    pub fn runtime_request(&self, run_id: RunId) -> LocalProcessRequest {
        LocalProcessRequest {
            run_id,
            program: self.program.clone(),
            argv: self.argv.clone(),
            cwd: self.workspace_root.clone(),
            env: HashMap::new(),
        }
    }

    pub fn is_opted_in(&self) -> bool {
        std::env::var(self.opt_in_env).as_deref() == Ok("1")
    }
}

#[derive(Debug)]
pub enum LocalAdapterSmokeError {
    Io(std::io::Error),
    Runtime(RuntimeError),
    NotOptedIn(&'static str),
    SensitiveArtifact { path: PathBuf, marker: String },
    MarkerMissing { marker: &'static str },
}

impl From<std::io::Error> for LocalAdapterSmokeError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<RuntimeError> for LocalAdapterSmokeError {
    fn from(error: RuntimeError) -> Self {
        Self::Runtime(error)
    }
}

pub type LocalAdapterSmokeResult<T> = Result<T, LocalAdapterSmokeError>;

pub struct LocalAdapterSmokeRunner;

impl LocalAdapterSmokeRunner {
    pub fn run_if_opted_in(
        plan: &LocalAdapterSmokePlan,
    ) -> LocalAdapterSmokeResult<Option<LocalProcessOutcome>> {
        if !plan.is_opted_in() {
            return Ok(None);
        }

        Self::run(plan).map(Some)
    }

    pub fn run(plan: &LocalAdapterSmokePlan) -> LocalAdapterSmokeResult<LocalProcessOutcome> {
        fs::create_dir_all(&plan.workspace_root)?;
        fs::create_dir_all(&plan.artifact_root)?;
        let runner = LocalProcessRunner::new(plan.runtime_config());
        let outcome = runner.start_process(
            plan.runtime_request(RunId::new(format!("{}-smoke", plan.adapter_kind.as_str()))),
        )?;
        scan_artifacts_for_sensitive_markers([&outcome.stdout.path, &outcome.stderr.path])?;
        let stdout = fs::read_to_string(&outcome.stdout.path)?;
        let stderr = fs::read_to_string(&outcome.stderr.path)?;
        if !stdout.contains(plan.expected_output_marker)
            && !stderr.contains(plan.expected_output_marker)
        {
            return Err(LocalAdapterSmokeError::MarkerMissing {
                marker: plan.expected_output_marker,
            });
        }
        Ok(outcome)
    }
}

impl CodexExecAdapter {
    pub fn local_launch_plan(
        workspace_root: PathBuf,
        artifact_root: PathBuf,
        prompt: impl Into<String>,
    ) -> LocalAdapterLaunchPlan {
        LocalAdapterLaunchPlan {
            adapter_kind: NormalizedAdapterKind::CodexExec,
            provider_kind: "codex_subscription".to_string(),
            credential_scope: "user_local_subscription".to_string(),
            program: "codex".to_string(),
            argv: vec![
                "exec".to_string(),
                "--json".to_string(),
                "--sandbox".to_string(),
                "read-only".to_string(),
                "--ephemeral".to_string(),
                "--ignore-user-config".to_string(),
                "--ignore-rules".to_string(),
                "--cd".to_string(),
                workspace_root.to_string_lossy().to_string(),
                prompt.into(),
            ],
            workspace_root,
            artifact_root,
            env_allowlist: local_subscription_cli_env_allowlist(),
            redaction_rules: local_adapter_redaction_rules(),
            output_limit_bytes: 1024 * 1024,
            stdout_format: "jsonl".to_string(),
            stderr_policy: "logs_redacted".to_string(),
        }
    }

    pub fn local_smoke_plan(
        workspace_root: PathBuf,
        artifact_root: PathBuf,
    ) -> LocalAdapterSmokePlan {
        let launch_plan = Self::local_launch_plan(
            workspace_root,
            artifact_root,
            "Reply with exactly CAPO_CODEX_SMOKE_OK and do not inspect files.",
        );
        let mut argv = launch_plan.argv;
        argv.insert(7, "--skip-git-repo-check".to_string());
        LocalAdapterSmokePlan {
            adapter_kind: NormalizedAdapterKind::CodexExec,
            opt_in_env: "CAPO_RUN_CODEX_LOCAL_SMOKE",
            program: launch_plan.program,
            argv,
            workspace_root: launch_plan.workspace_root,
            artifact_root: launch_plan.artifact_root,
            env_allowlist: launch_plan.env_allowlist,
            redaction_rules: launch_plan.redaction_rules,
            output_limit_bytes: launch_plan.output_limit_bytes,
            expected_output_marker: "CAPO_CODEX_SMOKE_OK",
        }
    }
}

impl ClaudeCodeAdapter {
    pub fn local_launch_plan(
        workspace_root: PathBuf,
        artifact_root: PathBuf,
        prompt: impl Into<String>,
    ) -> LocalAdapterLaunchPlan {
        LocalAdapterLaunchPlan {
            adapter_kind: NormalizedAdapterKind::ClaudeCode,
            provider_kind: "claude_subscription".to_string(),
            credential_scope: "user_local_subscription".to_string(),
            program: "claude".to_string(),
            argv: vec![
                "-p".to_string(),
                "--output-format".to_string(),
                "stream-json".to_string(),
                "--verbose".to_string(),
                "--permission-mode".to_string(),
                "plan".to_string(),
                "--no-session-persistence".to_string(),
                "--disable-slash-commands".to_string(),
                "--tools".to_string(),
                "".to_string(),
                "--disallowedTools".to_string(),
                "*".to_string(),
                "--mcp-config".to_string(),
                "/dev/null".to_string(),
                "--strict-mcp-config".to_string(),
                prompt.into(),
            ],
            workspace_root,
            artifact_root,
            env_allowlist: local_subscription_cli_env_allowlist(),
            redaction_rules: local_adapter_redaction_rules(),
            output_limit_bytes: 1024 * 1024,
            stdout_format: "stream-json".to_string(),
            stderr_policy: "logs_redacted".to_string(),
        }
    }

    pub fn local_smoke_plan(
        workspace_root: PathBuf,
        artifact_root: PathBuf,
    ) -> LocalAdapterSmokePlan {
        let launch_plan = Self::local_launch_plan(
            workspace_root,
            artifact_root,
            "Reply with exactly CAPO_CLAUDE_SMOKE_OK and do not inspect files.",
        );
        LocalAdapterSmokePlan {
            adapter_kind: NormalizedAdapterKind::ClaudeCode,
            opt_in_env: "CAPO_RUN_CLAUDE_LOCAL_SMOKE",
            program: launch_plan.program,
            argv: launch_plan.argv,
            workspace_root: launch_plan.workspace_root,
            artifact_root: launch_plan.artifact_root,
            env_allowlist: launch_plan.env_allowlist,
            redaction_rules: launch_plan.redaction_rules,
            output_limit_bytes: launch_plan.output_limit_bytes,
            expected_output_marker: "CAPO_CLAUDE_SMOKE_OK",
        }
    }
}

pub fn scan_artifacts_for_sensitive_markers<'a>(
    paths: impl IntoIterator<Item = &'a PathBuf>,
) -> LocalAdapterSmokeResult<()> {
    for path in paths {
        let contents = fs::read_to_string(path)?;
        if let Some(marker) = sensitive_marker(&contents) {
            return Err(LocalAdapterSmokeError::SensitiveArtifact {
                path: path.clone(),
                marker,
            });
        }
    }
    Ok(())
}

fn local_subscription_cli_env_allowlist() -> Vec<String> {
    vec![
        "HOME".to_string(),
        "PATH".to_string(),
        "TMPDIR".to_string(),
        "USER".to_string(),
        "LOGNAME".to_string(),
        "SHELL".to_string(),
        "LANG".to_string(),
    ]
}

fn local_adapter_redaction_rules() -> Vec<RedactionRule> {
    [
        ("Authorization:", "Authorization: [REDACTED]"),
        ("Cookie:", "Cookie: [REDACTED]"),
        ("session_token", "session_[REDACTED]"),
        ("api_key", "api_[REDACTED]"),
        ("access_token", "access_[REDACTED]"),
        ("refresh_token", "refresh_[REDACTED]"),
    ]
    .into_iter()
    .map(|(pattern, replacement)| RedactionRule {
        pattern: pattern.to_string(),
        replacement: replacement.to_string(),
    })
    .collect()
}

fn sensitive_marker(contents: &str) -> Option<String> {
    for line in contents.lines() {
        if line.to_ascii_lowercase().contains("[redacted]") {
            continue;
        }
        let lower = line.to_ascii_lowercase();
        if let Some(marker) = [
            "authorization:",
            "cookie:",
            "set-cookie:",
            "session_token",
            "access_token",
            "refresh_token",
            "oauth",
            "api_key",
            "anthropic_api_key",
            "openai_api_key",
        ]
        .into_iter()
        .find(|marker| lower.contains(marker))
        {
            return Some(marker.to_string());
        }
        if let Some(marker) = ["sk-proj-", "sk-ant-", "sk-live-", "sk_test_", "sk-svcacct-"]
            .into_iter()
            .find(|marker| lower.contains(marker))
        {
            return Some(marker.to_string());
        }
        if has_legacy_openai_key_shape(&lower) {
            return Some("sk-".to_string());
        }
    }
    None
}

fn has_legacy_openai_key_shape(line: &str) -> bool {
    let mut rest = line;
    while let Some(index) = rest.find("sk-") {
        let candidate = &rest[index + 3..];
        let token_len = candidate
            .chars()
            .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_' || *ch == '-')
            .count();
        if token_len >= 20 {
            return true;
        }
        rest = &candidate[token_len..];
    }
    false
}
