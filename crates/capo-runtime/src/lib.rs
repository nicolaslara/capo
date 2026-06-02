//! Runtime runner and connectivity scaffolding.
//!
//! P5 adds the first real runtime path: a local process runner that executes a
//! bounded command with a scrubbed environment and captures stdout/stderr as
//! rule-redacted artifacts. Connectivity remains a separate boundary.

use std::collections::HashMap;
use std::fs;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use capo_core::{BoundaryBinding, BoundaryKind, RunId};
#[cfg(unix)]
use std::os::unix::process::CommandExt;

mod async_runner;
mod sandbox;
mod worktree;

pub use async_runner::{
    AsyncLocalProcessRunner, AsyncRunningProcess, StreamSource, StreamingOutcome,
};
pub use sandbox::{
    OsSandbox, SandboxEnforcement, SandboxPlan, SandboxProfile, SandboxRefusal, SandboxRun,
    SandboxTier,
};
pub use worktree::{
    IsolatedWorktree, WORKTREE_ISOLATION_NONE_VARIANT, WORKTREE_ISOLATION_VARIANT, WorktreeError,
    WorktreeEvent, WorktreeManager, WorktreeOutcome, WorktreeRequest,
};

/// First runtime variants from the prototype plan.
pub const PLANNED_RUNTIMES: &[&str] = &["fake", "local-process", "remote-process"];
/// First tunnel variants from the runtime/tunnel plan.
pub const PLANNED_TUNNELS: &[&str] = &["fake", "local-loopback", "endpoint-stub"];

pub type RuntimeResult<T> = Result<T, RuntimeError>;

#[derive(Debug)]
pub enum RuntimeError {
    Io(std::io::Error),
    CwdOutsideWorkspace {
        cwd: PathBuf,
        workspace_roots: Vec<PathBuf>,
    },
    OutputLimitExceeded {
        limit_bytes: usize,
        actual_bytes: usize,
    },
    DisallowedEnvOverride(String),
}

impl From<std::io::Error> for RuntimeError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeRunner {
    Fake(FakeRuntimeRunner),
    LocalProcess(LocalProcessRunner),
    RemoteProcess(RemoteProcessRunner),
}

impl RuntimeRunner {
    pub fn fake() -> Self {
        Self::Fake(FakeRuntimeRunner)
    }

    pub fn local_process(config: LocalProcessConfig) -> Self {
        Self::LocalProcess(LocalProcessRunner::new(config))
    }

    pub fn remote_process(config: RemoteProcessConfig) -> Self {
        Self::RemoteProcess(RemoteProcessRunner::new(config))
    }

    pub fn binding(&self) -> BoundaryBinding {
        match self {
            Self::Fake(runner) => runner.binding(),
            Self::LocalProcess(runner) => runner.binding(),
            Self::RemoteProcess(runner) => runner.binding(),
        }
    }

    pub fn start(&self, request: FakeRuntimeStartRequest) -> FakeRuntimeProcess {
        match self {
            Self::Fake(runner) => runner.start(request),
            Self::LocalProcess(_) => FakeRuntimeRunner.start(request),
            Self::RemoteProcess(_) => FakeRuntimeRunner.start(request),
        }
    }

    pub fn interrupt(&self, process: &FakeRuntimeProcess, reason: &str) -> FakeRuntimeProcess {
        match self {
            Self::Fake(runner) => runner.interrupt(process, reason),
            Self::LocalProcess(_) => FakeRuntimeRunner.interrupt(process, reason),
            Self::RemoteProcess(_) => FakeRuntimeRunner.interrupt(process, reason),
        }
    }

    pub fn stop(&self, process: &FakeRuntimeProcess, reason: &str) -> FakeRuntimeProcess {
        match self {
            Self::Fake(runner) => runner.stop(process, reason),
            Self::LocalProcess(_) => FakeRuntimeRunner.stop(process, reason),
            Self::RemoteProcess(_) => FakeRuntimeRunner.stop(process, reason),
        }
    }

    pub fn attach_process(&self, run_id: RunId, runtime_process_ref: String) -> FakeRuntimeProcess {
        match self {
            Self::Fake(runner) => runner.attach_process(run_id, runtime_process_ref),
            Self::LocalProcess(_) => FakeRuntimeRunner.attach_process(run_id, runtime_process_ref),
            Self::RemoteProcess(_) => FakeRuntimeRunner.attach_process(run_id, runtime_process_ref),
        }
    }

    pub fn start_local_process(
        &self,
        request: LocalProcessRequest,
    ) -> RuntimeResult<LocalProcessOutcome> {
        match self {
            Self::LocalProcess(runner) => runner.start_process(request),
            Self::RemoteProcess(runner) => runner.start_process(request),
            Self::Fake(_) => LocalProcessRunner::new(LocalProcessConfig::for_test(
                std::env::current_dir()?,
                std::env::temp_dir().join("capo-runtime-fake-local"),
            ))
            .start_process(request),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeRuntimeRunner;

impl FakeRuntimeRunner {
    pub fn binding(&self) -> BoundaryBinding {
        BoundaryBinding::fake(BoundaryKind::RuntimeRunner, "fake-runtime")
    }

    pub fn start(&self, request: FakeRuntimeStartRequest) -> FakeRuntimeProcess {
        FakeRuntimeProcess {
            run_id: request.run_id,
            runtime_process_ref: format!("fake-runtime-process-{}", request.agent_name),
            status: "running".to_string(),
        }
    }

    pub fn interrupt(&self, process: &FakeRuntimeProcess, _reason: &str) -> FakeRuntimeProcess {
        FakeRuntimeProcess {
            run_id: process.run_id.clone(),
            runtime_process_ref: process.runtime_process_ref.clone(),
            status: "stopping".to_string(),
        }
    }

    pub fn stop(&self, process: &FakeRuntimeProcess, _reason: &str) -> FakeRuntimeProcess {
        FakeRuntimeProcess {
            run_id: process.run_id.clone(),
            runtime_process_ref: process.runtime_process_ref.clone(),
            status: "exited".to_string(),
        }
    }

    pub fn attach_process(&self, run_id: RunId, runtime_process_ref: String) -> FakeRuntimeProcess {
        FakeRuntimeProcess {
            run_id,
            runtime_process_ref,
            status: "running".to_string(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeRuntimeStartRequest {
    pub run_id: RunId,
    pub agent_name: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeRuntimeProcess {
    pub run_id: RunId,
    pub runtime_process_ref: String,
    pub status: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalProcessConfig {
    pub workspace_roots: Vec<PathBuf>,
    pub artifact_root: PathBuf,
    pub env_allowlist: Vec<String>,
    pub redaction_rules: Vec<RedactionRule>,
    pub output_limit_bytes: usize,
}

impl LocalProcessConfig {
    pub fn for_test(workspace_root: PathBuf, artifact_root: PathBuf) -> Self {
        Self {
            workspace_roots: vec![workspace_root],
            artifact_root,
            env_allowlist: Vec::new(),
            redaction_rules: Vec::new(),
            output_limit_bytes: 64 * 1024,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RedactionRule {
    pub pattern: String,
    pub replacement: String,
}

/// The placeholder a credential-shape match is replaced with.
pub const CREDENTIAL_REDACTION_PLACEHOLDER: &str = "[REDACTED:credential]";

/// A real redaction policy: configurable literal [`RedactionRule`] patterns PLUS
/// a default credential-shape / high-entropy scan (ACI7).
///
/// Today's runner redaction is a literal substring replace of operator-declared
/// patterns only; that misses any secret the operator did not name (the common
/// case for tool OUTPUT -- shell stdout/stderr, a read file, a diff -- which is
/// exactly where credentials leak). This policy keeps the explicit-pattern pass
/// AND layers a default scan that recognizes credential-shaped tokens (known key
/// prefixes, bearer headers, and long high-entropy strings) so an unnamed secret
/// is still scrubbed before it reaches an artifact or the agent. The same policy
/// is applied at the runtime runner boundary (process stdout/stderr) and at the
/// tool wrapper boundary (input AND output artifacts), so redaction is uniform.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RedactionPolicy {
    rules: Vec<RedactionRule>,
    scan_credentials: bool,
}

impl RedactionPolicy {
    /// A policy with the given literal rules and the default credential-shape
    /// scan enabled.
    pub fn new(rules: Vec<RedactionRule>) -> Self {
        Self {
            rules,
            scan_credentials: true,
        }
    }

    /// A policy with explicit rules but the default credential scan disabled.
    /// Used where only operator-declared patterns should apply.
    pub fn rules_only(rules: Vec<RedactionRule>) -> Self {
        Self {
            rules,
            scan_credentials: false,
        }
    }

    /// Whether the default credential-shape scan is enabled.
    pub fn scans_credentials(&self) -> bool {
        self.scan_credentials
    }

    /// Apply the policy to `bytes`, returning the redacted bytes and a
    /// `redaction_state` of `"redacted"` (something matched) or `"safe"`.
    ///
    /// The explicit literal rules run first (so an operator pattern always wins
    /// its exact replacement), then the credential-shape scan rewrites any
    /// remaining credential-shaped token to [`CREDENTIAL_REDACTION_PLACEHOLDER`].
    pub fn apply(&self, bytes: &[u8]) -> (Vec<u8>, String) {
        let mut text = String::from_utf8_lossy(bytes).to_string();
        let mut redacted = false;
        for rule in &self.rules {
            if text.contains(&rule.pattern) {
                text = text.replace(&rule.pattern, &rule.replacement);
                redacted = true;
            }
        }
        if self.scan_credentials {
            let (scanned, scanned_any) = scan_credential_shapes(&text);
            if scanned_any {
                text = scanned;
                redacted = true;
            }
        }
        (
            text.into_bytes(),
            if redacted { "redacted" } else { "safe" }.to_string(),
        )
    }
}

/// Rewrite credential-shaped tokens in `text` to the credential placeholder,
/// returning the rewritten text and whether anything matched (ACI7).
///
/// A token is credential-shaped when it carries a known secret prefix
/// (`AKIA`/`ASIA`, `sk-`, `ghp_`/`gho_`/`github_pat_`, `xox[bap]-`, `AIza`,
/// `glpat-`), or when it is a long, high-entropy run of credential characters
/// (>= 20 chars of `[A-Za-z0-9_\-+/=.]` that mixes upper-case, lower-case and
/// digits -- the shape of an opaque base64/random API key). The shape check
/// runs against the token AND the candidate substrings extracted from quoting,
/// `key=value`, JSON, and URL-query wrappers (see [`is_credential_shaped`]), so a
/// secret leaks through none of those. A `Bearer <token>` header has its token
/// component scrubbed even if the token itself is short. Ordinary prose words,
/// file paths, hex digests / git SHAs, and dashed UUIDs are excluded so the scan
/// does not blank out useful command output.
fn scan_credential_shapes(text: &str) -> (String, bool) {
    let mut out = String::with_capacity(text.len());
    let mut redacted = false;
    // Split on whitespace boundaries, preserving the exact whitespace so the
    // redacted output keeps its line/column structure (callers diff and display
    // it). A "token" is a maximal run of non-whitespace characters.
    let mut token = String::new();
    let mut prev_token: Option<String> = None;
    let flush = |token: &mut String,
                 prev_token: &mut Option<String>,
                 out: &mut String,
                 redacted: &mut bool| {
        if token.is_empty() {
            return;
        }
        let after_bearer = prev_token
            .as_deref()
            .is_some_and(|prev| prev.eq_ignore_ascii_case("bearer"));
        if is_credential_shaped(token, after_bearer) {
            out.push_str(CREDENTIAL_REDACTION_PLACEHOLDER);
            *redacted = true;
        } else {
            out.push_str(token);
        }
        *prev_token = Some(std::mem::take(token));
    };
    for ch in text.chars() {
        if ch.is_whitespace() {
            flush(&mut token, &mut prev_token, &mut out, &mut redacted);
            out.push(ch);
            if ch == '\n' {
                prev_token = None;
            }
        } else {
            token.push(ch);
        }
    }
    flush(&mut token, &mut prev_token, &mut out, &mut redacted);
    (out, redacted)
}

/// Whether `raw` looks like a credential token. `after_bearer` lowers the bar
/// for a token that directly follows a `Bearer` header.
///
/// A whitespace-delimited token rarely arrives as a bare secret: it may be
/// quoted (`"sk-..."`), part of a `key=value` assignment (`AWS_SECRET=AKIA...`),
/// embedded in JSON (`{"k":"AKIA..."}`), or a URL query (`?token=AKIA...&x=1`).
/// We therefore derive a set of CANDIDATE substrings from `raw` -- the trimmed
/// whole token, the value after a `key=` split, and the pieces between interior
/// quote/JSON/URL delimiters -- and treat the token as credential-shaped if ANY
/// candidate matches. This means a secret that the operator did not name still
/// gets scrubbed regardless of the punctuation it is wrapped in.
fn is_credential_shaped(raw: &str, after_bearer: bool) -> bool {
    credential_candidates(raw)
        .iter()
        .any(|candidate| candidate_is_credential(candidate, after_bearer))
}

/// Characters that delimit a credential from surrounding quoting/structure.
/// Trimmed at candidate ENDS and split on in the interior so quoted, JSON, and
/// URL-embedded secrets are isolated from their wrapper.
fn is_credential_boundary(c: char) -> bool {
    matches!(
        c,
        '"' | '\''
            | '`'
            | '('
            | ')'
            | '['
            | ']'
            | '{'
            | '}'
            | ','
            | ';'
            | ':'
            | '?'
            | '&'
            | '<'
            | '>'
            | '|'
    )
}

/// Derive candidate credential substrings from a whitespace-delimited token.
///
/// Yields (in addition to the trimmed whole token): the value after a `key=`
/// assignment prefix, and every interior piece split on quote/JSON/URL
/// boundaries -- each re-trimmed of surrounding boundary punctuation. The
/// `key=` value is kept WHOLE (its own `=`, e.g. base64 padding, is preserved)
/// AND the whole trimmed token is also scanned, so neither a real `key=secret`
/// nor a bare base64 token whose only `=` is padding can slip past.
fn credential_candidates(raw: &str) -> Vec<String> {
    let trim = |s: &str| s.trim_matches(is_credential_boundary).to_string();
    let mut candidates: Vec<String> = Vec::new();
    let push = |s: String, candidates: &mut Vec<String>| {
        if !s.is_empty() && !candidates.contains(&s) {
            candidates.push(s);
        }
    };

    let trimmed = trim(raw);

    // The whole trimmed token: catches a bare secret AND a base64 token whose
    // only `=` is trailing padding (no real key/value structure).
    push(trimmed.clone(), &mut candidates);

    // A `key=value` assignment: scan the value after the FIRST `=` when the key
    // segment is a plausible env-var/identifier name. Keep the value whole so
    // its own `=` (base64 padding) survives, then re-trim wrapping quotes so
    // `token="AKIA..."` exposes the bare secret.
    if let Some(value) = assignment_value(&trimmed) {
        push(trim(value), &mut candidates);
    }

    // Interior pieces split on quote/JSON/URL boundaries so a secret embedded in
    // `{"aws_key":"AKIA..."}` or `https://x/y?token=AKIA...&z=1` is isolated.
    for piece in raw.split(is_credential_boundary) {
        // Each piece may still be a `k=v` pair (URL query, env line): take both
        // the trimmed piece and the post-`=` value.
        let piece_trimmed = trim(piece);
        if let Some(value) = assignment_value(&piece_trimmed) {
            push(trim(value), &mut candidates);
        }
        push(piece_trimmed, &mut candidates);
    }

    candidates
}

/// If `token` is a `key=value` assignment whose key segment is a plausible
/// env-var / identifier name, return the value after the FIRST `=` (kept whole,
/// so the value's own `=`, e.g. base64 padding, is preserved). Returns `None`
/// when there is no `=` or the key segment is not identifier-shaped, so a bare
/// base64 token whose only `=` is padding is NOT mistaken for an assignment.
fn assignment_value(token: &str) -> Option<&str> {
    let (key, rest) = token.split_once('=')?;
    let identifier_key = !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        && key.chars().any(|c| c.is_ascii_alphabetic());
    identifier_key.then_some(rest)
}

/// Whether a single, already-isolated candidate string is credential-shaped.
fn candidate_is_credential(value: &str, after_bearer: bool) -> bool {
    if value.is_empty() {
        return false;
    }
    const KNOWN_PREFIXES: &[&str] = &[
        "AKIA",
        "ASIA",
        "sk-",
        "ghp_",
        "gho_",
        "ghu_",
        "ghs_",
        "github_pat_",
        "xoxb-",
        "xoxp-",
        "xoxa-",
        "AIza",
        "glpat-",
    ];
    if KNOWN_PREFIXES
        .iter()
        .any(|prefix| value.starts_with(prefix) && value.len() > prefix.len() + 4)
    {
        return true;
    }
    if after_bearer && value.len() >= 8 && value.chars().all(is_credential_char) {
        return true;
    }
    // Long, high-entropy run of credential characters: the shape of a base64/
    // random API key. Require a minimum length, all credential characters, a mix
    // of letters and digits, and enough distinct characters that an obvious
    // structured string does not trip it.
    //
    // To keep ordinary command output intact we additionally EXCLUDE the two
    // structured shapes that pollute git/test output and would otherwise clear
    // this bar: a hex digest (e.g. a 40-char git commit SHA) and a dashed UUID.
    // Both are single-case hex; real opaque secrets that reach this fallback
    // (un-prefixed base64 / random tokens) mix upper-case, lower-case AND
    // digits, so we require that character-class diversity (or a base64 symbol
    // `+`/`/`/`=`) and reject pure hex / UUID shapes.
    if value.len() >= 20 && value.chars().all(is_credential_char) {
        if is_hex_digest(value) || is_uuid_shaped(value) {
            return false;
        }
        let has_digit = value.chars().any(|c| c.is_ascii_digit());
        let has_upper = value.chars().any(|c| c.is_ascii_uppercase());
        let has_lower = value.chars().any(|c| c.is_ascii_lowercase());
        // `+`/`=` are base64 fingerprints that filesystem paths never carry; `/`
        // is deliberately EXCLUDED here because it dominates paths and would
        // otherwise flag `/usr/local/lib/...` as a credential.
        let has_base64_symbol = value.chars().any(|c| matches!(c, '+' | '='));
        let distinct = {
            let mut seen = [false; 128];
            let mut count = 0usize;
            for c in value.chars() {
                let idx = c as usize;
                if idx < 128 && !seen[idx] {
                    seen[idx] = true;
                    count += 1;
                }
            }
            count
        };
        // Mixed-case + digit is the random/base64 fingerprint; a base64 symbol
        // is an equally strong signal (e.g. `+`/`/` in a padded key).
        let diverse = has_digit && ((has_upper && has_lower) || has_base64_symbol);
        if diverse && distinct >= 12 {
            return true;
        }
    }
    false
}

/// Whether `value` is a pure hexadecimal digest (e.g. a git commit SHA or a
/// sha256 hex digest), ignoring interior `-` grouping. These are single-case
/// hex strings that show up constantly in `git_diff`/`git_status` output and
/// must NOT be mistaken for credentials.
fn is_hex_digest(value: &str) -> bool {
    let core: String = value.chars().filter(|&c| c != '-').collect();
    core.len() >= 12 && core.chars().all(|c| c.is_ascii_hexdigit())
}

/// Whether `value` has the canonical 8-4-4-4-12 dashed UUID shape (hex digits
/// in five dash-separated groups). UUIDs appear in logs and test output.
fn is_uuid_shaped(value: &str) -> bool {
    let groups: Vec<&str> = value.split('-').collect();
    let lens = [8usize, 4, 4, 4, 12];
    groups.len() == 5
        && groups
            .iter()
            .zip(lens.iter())
            .all(|(group, &len)| group.len() == len && group.chars().all(|c| c.is_ascii_hexdigit()))
}

fn is_credential_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '+' | '/' | '=' | '.')
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalProcessRunner {
    config: LocalProcessConfig,
}

impl LocalProcessRunner {
    pub fn new(config: LocalProcessConfig) -> Self {
        Self { config }
    }

    pub fn binding(&self) -> BoundaryBinding {
        BoundaryBinding {
            kind: BoundaryKind::RuntimeRunner,
            variant: "local-process",
            fake: false,
        }
    }

    pub fn start_process(
        &self,
        request: LocalProcessRequest,
    ) -> RuntimeResult<LocalProcessOutcome> {
        self.ensure_cwd_allowed(&request.cwd)?;
        fs::create_dir_all(&self.config.artifact_root)?;

        let mut command = Command::new(&request.program);
        command.args(&request.argv);
        command.current_dir(&request.cwd);
        command.env_clear();
        for name in &self.config.env_allowlist {
            if let Ok(value) = std::env::var(name) {
                command.env(name, value);
            }
        }
        self.apply_request_env(&mut command, &request)?;

        let output = command.output()?;
        let stdout = capped_output(output.stdout, self.config.output_limit_bytes)?;
        let stderr = capped_output(output.stderr, self.config.output_limit_bytes)?;
        let stdout = self.redact_output(&stdout);
        let stderr = self.redact_output(&stderr);

        let runtime_process_ref = LocalRuntimeProcessRef {
            run_id: request.run_id.clone(),
            runtime_process_ref: format!("local-process-{}", request.run_id),
            external_pid: None,
            boot_id: None,
            status: "exited".to_string(),
            redaction_state: stdout.redaction_state.clone(),
        };
        let stdout_artifact = self.write_artifact(
            &request.run_id,
            request.turn_id.as_deref(),
            "stdout",
            &stdout.bytes,
            &stdout.redaction_state,
        )?;
        let stderr_artifact = self.write_artifact(
            &request.run_id,
            request.turn_id.as_deref(),
            "stderr",
            &stderr.bytes,
            &stderr.redaction_state,
        )?;
        let output_detail = format!(
            "{},{}",
            stdout_artifact.artifact_id, stderr_artifact.artifact_id
        );
        let exit_code = output.status.code();
        let exit_status = if output.status.success() {
            "exited"
        } else {
            "failed"
        };

        Ok(LocalProcessOutcome {
            process: LocalRuntimeProcessRef {
                status: exit_status.to_string(),
                ..runtime_process_ref
            },
            stdout: stdout_artifact,
            stderr: stderr_artifact,
            exit_code,
            events: vec![
                RuntimeEvent {
                    kind: "runtime.start_requested".to_string(),
                    status: "pending".to_string(),
                    detail: request.program,
                },
                RuntimeEvent {
                    kind: "runtime.process_started".to_string(),
                    status: "started".to_string(),
                    detail: request.run_id.to_string(),
                },
                RuntimeEvent {
                    kind: "runtime.output_delta".to_string(),
                    status: stdout.redaction_state.clone(),
                    detail: "stdout,stderr".to_string(),
                },
                RuntimeEvent {
                    kind: "runtime.output_artifact_recorded".to_string(),
                    status: stdout.redaction_state.clone(),
                    detail: output_detail,
                },
                RuntimeEvent {
                    kind: "runtime.process_exited".to_string(),
                    status: exit_status.to_string(),
                    detail: exit_code
                        .map(|code| code.to_string())
                        .unwrap_or_else(|| "signal".to_string()),
                },
            ],
        })
    }

    pub fn spawn_process(
        &self,
        request: LocalProcessRequest,
    ) -> RuntimeResult<LocalRunningProcess> {
        self.ensure_cwd_allowed(&request.cwd)?;
        fs::create_dir_all(&self.config.artifact_root)?;

        let run_dir = self.run_dir_for(&request.run_id, request.turn_id.as_deref());
        fs::create_dir_all(&run_dir)?;
        let stdout_path = run_dir.join("stdout.txt");
        let stderr_path = run_dir.join("stderr.txt");

        let mut command = Command::new(&request.program);
        command.args(&request.argv);
        command.current_dir(&request.cwd);
        command.env_clear();
        for name in &self.config.env_allowlist {
            if let Ok(value) = std::env::var(name) {
                command.env(name, value);
            }
        }
        self.apply_request_env(&mut command, &request)?;
        command.stdout(Stdio::from(File::create(&stdout_path)?));
        command.stderr(Stdio::from(File::create(&stderr_path)?));
        #[cfg(unix)]
        {
            command.process_group(0);
        }

        let child = command.spawn()?;
        let external_pid = child.id();
        Ok(LocalRunningProcess {
            process: LocalRuntimeProcessRef {
                run_id: request.run_id.clone(),
                runtime_process_ref: format!("local-process-{}", request.run_id),
                external_pid: Some(external_pid),
                // Stamp the spawning boot id so restart recovery only reaps this
                // process group within the same boot (a reused PID after a
                // reboot must not be signalled).
                boot_id: boot_id(),
                status: "running".to_string(),
                redaction_state: "redacted".to_string(),
            },
            child,
            turn_id: request.turn_id.clone(),
            stdout_path,
            stderr_path,
            events: vec![
                RuntimeEvent {
                    kind: "runtime.start_requested".to_string(),
                    status: "pending".to_string(),
                    detail: request.program,
                },
                RuntimeEvent {
                    kind: "runtime.process_started".to_string(),
                    status: "started".to_string(),
                    detail: external_pid.to_string(),
                },
            ],
        })
    }

    /// Spawn a long-lived process with PIPED stdin+stdout for a bidirectional
    /// line protocol (e.g. an ACP JSON-RPC 2.0 stdio agent), reusing the same
    /// env-scrub, workspace confinement, and process-group ownership as
    /// [`Self::spawn_process`].
    ///
    /// Unlike [`Self::spawn_process`] (which redirects the child's stdout/stderr
    /// to artifact files for a one-shot read), this keeps stdin/stdout as pipes
    /// so a caller can drive a request/response + notification protocol over the
    /// wire. The RUNTIME still owns the process group (`process_group(0)` on
    /// unix), so an adapter that drives the protocol never owns the process
    /// group itself -- it only borrows the pipe handles via
    /// [`PipedRunningProcess`]. stderr is still redirected to an artifact file so
    /// the child's diagnostics are captured and redacted out-of-band.
    pub fn spawn_piped_process(
        &self,
        request: LocalProcessRequest,
    ) -> RuntimeResult<PipedRunningProcess> {
        self.ensure_cwd_allowed(&request.cwd)?;
        fs::create_dir_all(&self.config.artifact_root)?;

        let run_dir = self.run_dir_for(&request.run_id, request.turn_id.as_deref());
        fs::create_dir_all(&run_dir)?;
        let stderr_path = run_dir.join("stderr.txt");

        let mut command = Command::new(&request.program);
        command.args(&request.argv);
        command.current_dir(&request.cwd);
        command.env_clear();
        for name in &self.config.env_allowlist {
            if let Ok(value) = std::env::var(name) {
                command.env(name, value);
            }
        }
        self.apply_request_env(&mut command, &request)?;
        command.stdin(Stdio::piped());
        command.stdout(Stdio::piped());
        command.stderr(Stdio::from(File::create(&stderr_path)?));
        #[cfg(unix)]
        {
            command.process_group(0);
        }

        let mut child = command.spawn()?;
        let external_pid = child.id();
        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        Ok(PipedRunningProcess {
            process: LocalRuntimeProcessRef {
                run_id: request.run_id.clone(),
                runtime_process_ref: format!("local-piped-process-{}", request.run_id),
                external_pid: Some(external_pid),
                boot_id: boot_id(),
                status: "running".to_string(),
                redaction_state: "redacted".to_string(),
            },
            child,
            stdin,
            stdout,
            stderr_path,
            events: vec![
                RuntimeEvent {
                    kind: "runtime.start_requested".to_string(),
                    status: "pending".to_string(),
                    detail: request.program,
                },
                RuntimeEvent {
                    kind: "runtime.process_started".to_string(),
                    status: "started".to_string(),
                    detail: external_pid.to_string(),
                },
            ],
        })
    }

    pub fn interrupt(
        &self,
        process: &LocalRuntimeProcessRef,
        reason: &str,
    ) -> RuntimeControlResult {
        self.control(
            process,
            "interrupting",
            "runtime.interrupt_requested",
            reason,
        )
    }

    pub fn terminate(
        &self,
        process: &LocalRuntimeProcessRef,
        reason: &str,
    ) -> RuntimeControlResult {
        self.control(
            process,
            "terminating",
            "runtime.terminate_requested",
            reason,
        )
    }

    pub fn kill(&self, process: &LocalRuntimeProcessRef) -> RuntimeControlResult {
        self.control(
            process,
            "killed",
            "runtime.kill_requested",
            "kill requested",
        )
    }

    pub fn kill_running(
        &self,
        process: &mut LocalRunningProcess,
    ) -> RuntimeResult<RuntimeControlResult> {
        process.child.kill()?;
        process.process.status = "killed".to_string();
        Ok(RuntimeControlResult {
            process: process.process.clone(),
            events: vec![RuntimeEvent {
                kind: "runtime.kill_requested".to_string(),
                status: "killed".to_string(),
                detail: process.process.runtime_process_ref.clone(),
            }],
        })
    }

    /// Hard-kill a live run by terminating its entire process group.
    ///
    /// This reuses the same `SIGTERM` then `SIGKILL` process-group teardown the
    /// timeout path uses ([`Self::terminate_process_group`]), so a hard kill
    /// reaps the spawned child and all of its descendants rather than only the
    /// direct child. It is the runtime primitive the RTL6 controller-owned hard
    /// kill drives mid-run.
    pub fn kill_running_process_group(
        &self,
        process: &mut LocalRunningProcess,
    ) -> RuntimeResult<RuntimeControlResult> {
        self.terminate_process_group(process);
        let _ = process.child.kill();
        process.process.status = "killed".to_string();
        Ok(RuntimeControlResult {
            process: process.process.clone(),
            events: vec![RuntimeEvent {
                kind: "runtime.kill_requested".to_string(),
                status: "killed".to_string(),
                detail: format!(
                    "process-group hard-kill: {}",
                    process.process.runtime_process_ref
                ),
            }],
        })
    }

    pub fn wait_running(
        &self,
        process: &mut LocalRunningProcess,
    ) -> RuntimeResult<LocalProcessOutcome> {
        let status = process.child.wait()?;
        let stdout = fs::read(&process.stdout_path)?;
        let stderr = fs::read(&process.stderr_path)?;
        let stdout = match capped_output(stdout, self.config.output_limit_bytes) {
            Ok(stdout) => self.redact_output(&stdout),
            Err(error) => {
                self.remove_raw_output_files(process);
                return Err(error);
            }
        };
        let stderr = match capped_output(stderr, self.config.output_limit_bytes) {
            Ok(stderr) => self.redact_output(&stderr),
            Err(error) => {
                self.remove_raw_output_files(process);
                return Err(error);
            }
        };
        fs::write(&process.stdout_path, &stdout.bytes)?;
        fs::write(&process.stderr_path, &stderr.bytes)?;
        let stdout_artifact = self.output_artifact_from_path(
            &process.process.run_id,
            process.turn_id.as_deref(),
            "stdout",
            &process.stdout_path,
            &stdout.bytes,
            &stdout.redaction_state,
        );
        let stderr_artifact = self.output_artifact_from_path(
            &process.process.run_id,
            process.turn_id.as_deref(),
            "stderr",
            &process.stderr_path,
            &stderr.bytes,
            &stderr.redaction_state,
        );
        let output_detail = format!(
            "{},{}",
            stdout_artifact.artifact_id, stderr_artifact.artifact_id
        );
        let exit_status = if status.success() {
            "exited"
        } else if process.process.status == "timed_out" {
            "timed_out"
        } else if process.process.status == "killed" {
            "killed"
        } else {
            "failed"
        };
        process.process.status = exit_status.to_string();

        Ok(LocalProcessOutcome {
            process: process.process.clone(),
            stdout: stdout_artifact,
            stderr: stderr_artifact,
            exit_code: status.code(),
            events: vec![
                RuntimeEvent {
                    kind: "runtime.output_delta".to_string(),
                    status: stdout.redaction_state.clone(),
                    detail: "stdout,stderr".to_string(),
                },
                RuntimeEvent {
                    kind: "runtime.output_artifact_recorded".to_string(),
                    status: stdout.redaction_state.clone(),
                    detail: output_detail,
                },
                RuntimeEvent {
                    kind: "runtime.process_exited".to_string(),
                    status: exit_status.to_string(),
                    detail: status
                        .code()
                        .map(|code| code.to_string())
                        .unwrap_or_else(|| "signal".to_string()),
                },
            ],
        })
    }

    pub fn wait_running_with_timeout(
        &self,
        process: &mut LocalRunningProcess,
        timeout: Duration,
    ) -> RuntimeResult<LocalProcessOutcome> {
        let started = Instant::now();
        loop {
            if process.child.try_wait()?.is_some() {
                return self.wait_running(process);
            }
            if started.elapsed() >= timeout {
                self.terminate_process_group(process);
                process.child.kill()?;
                process.process.status = "timed_out".to_string();
                return self.wait_running(process);
            }
            thread::sleep(Duration::from_millis(100));
        }
    }

    pub fn health(&self, process: &LocalRuntimeProcessRef) -> RuntimeHealth {
        RuntimeHealth {
            runtime_process_ref: process.runtime_process_ref.clone(),
            status: process.status.clone(),
            live: process.status == "running",
        }
    }

    pub fn health_running(
        &self,
        process: &mut LocalRunningProcess,
    ) -> RuntimeResult<RuntimeHealth> {
        let live = process.child.try_wait()?.is_none();
        Ok(RuntimeHealth {
            runtime_process_ref: process.process.runtime_process_ref.clone(),
            status: if live { "running" } else { "exited" }.to_string(),
            live,
        })
    }

    pub fn cleanup(&self, process: &LocalRuntimeProcessRef) -> RuntimeResult<CleanupReport> {
        let run_dir = self.config.artifact_root.join(process.run_id.as_str());
        fs::create_dir_all(&run_dir)?;
        let marker = run_dir.join("cleanup.marker");
        fs::write(&marker, b"cleanup requested; durable artifacts preserved")?;
        Ok(CleanupReport {
            runtime_process_ref: process.runtime_process_ref.clone(),
            preserved_artifact_dir: run_dir,
            marker_path: marker,
        })
    }

    pub fn recover_orphan(&self, process: &LocalRuntimeProcessRef) -> OrphanRecovery {
        let health = self.health(process);
        OrphanRecovery {
            runtime_process_ref: process.runtime_process_ref.clone(),
            recovered_status: if health.live { "recovered" } else { "orphaned" }.to_string(),
            detail: if health.live {
                "process still reports running".to_string()
            } else {
                "process is not live; preserving captured artifacts".to_string()
            },
        }
    }

    /// Reap an orphaned process group by its persisted PID after a restart.
    ///
    /// On restart Capo no longer owns the spawned [`Child`] handle, so the only
    /// durable reference to a run that was in-flight when the controller died is
    /// the PID/process-group reference it persisted *before* the spawn returned
    /// (RTL10). This probes that PID with a no-op signal (`kill -0`) to observe
    /// liveness, and if it is still alive sends the same `SIGTERM` then `SIGKILL`
    /// process-group teardown the timeout/hard-kill paths use
    /// ([`Self::terminate_process_group`]), so the orphaned child *and all of its
    /// descendants* are reaped rather than left running.
    ///
    /// The returned [`OrphanReap`] carries a stable
    /// `observed_runtime_state_hash` (over the PID, the recorded boot id, and the
    /// observed liveness) that the recovery layer folds into its idempotency key,
    /// so repeated restarts that observe the same runtime state never emit a
    /// second recovery event.
    ///
    /// `recorded_boot_id` is the [`boot_id`] captured at spawn time. Because PIDs
    /// and process-group ids are recycled by the OS, the persisted PID is only a
    /// meaningful handle within the boot that recorded it. If the recorded boot
    /// id is absent or differs from the current boot's id, this does NOT signal
    /// anything (a recycled PID after a reboot would otherwise SIGKILL an
    /// unrelated process group) and records the run as `already_gone`.
    #[cfg(unix)]
    pub fn reap_orphan_process_group(
        external_pid: u32,
        recorded_boot_id: Option<&str>,
    ) -> OrphanReap {
        let current_boot_id = boot_id();
        // Only reap within the same boot: a PID persisted before a reboot is
        // almost certainly recycled onto an unrelated process group afterwards.
        let same_boot = match (recorded_boot_id, current_boot_id.as_deref()) {
            (Some(recorded), Some(current)) => recorded == current,
            // No recorded or unreadable current boot id => identity cannot be
            // verified, so we conservatively decline to signal.
            _ => false,
        };
        let alive_before = same_boot && process_group_is_alive(external_pid);
        if alive_before {
            kill_process_group(external_pid, "-TERM");
            thread::sleep(Duration::from_millis(100));
            kill_process_group(external_pid, "-KILL");
        }
        let observed_state = if alive_before {
            "alive_reaped"
        } else {
            "already_gone"
        };
        OrphanReap {
            external_pid,
            reaped: alive_before,
            observed_state: observed_state.to_string(),
            observed_runtime_state_hash: orphan_state_hash(
                external_pid,
                recorded_boot_id,
                observed_state,
            ),
        }
    }

    /// Non-Unix fallback: there is no portable process-group reaping primitive,
    /// so the orphan is recorded as already gone (Capo never spawns process
    /// groups off Unix).
    #[cfg(not(unix))]
    pub fn reap_orphan_process_group(
        external_pid: u32,
        recorded_boot_id: Option<&str>,
    ) -> OrphanReap {
        let observed_state = "already_gone";
        OrphanReap {
            external_pid,
            reaped: false,
            observed_state: observed_state.to_string(),
            observed_runtime_state_hash: orphan_state_hash(
                external_pid,
                recorded_boot_id,
                observed_state,
            ),
        }
    }

    /// SG9: NON-DESTRUCTIVELY probe the liveness/health of a run that was
    /// in-flight when the controller died, by its persisted PID/process-group.
    ///
    /// This is the liveness-aware counterpart to
    /// [`Self::reap_orphan_process_group`]: the reaper KILLS a live orphan (the
    /// RTL10 phase-1 behavior), whereas this probe only OBSERVES it (`kill -0`),
    /// so the recovery layer can REATTACH to a still-alive run in place rather
    /// than blindly terminating it (SG9 acceptance / `state-model.md` Restart
    /// Recovery). A run observed alive within the same boot is classified
    /// [`RuntimeHealthState::Alive`] (reattachable); a run whose group is gone, or
    /// whose recorded boot id cannot be confirmed against the current boot (a
    /// recycled PID after a reboot, which must never be trusted as "our" run), is
    /// classified [`RuntimeHealthState::Exited`].
    ///
    /// The returned [`RunHealthProbe`] carries a stable `observed_state_hash`
    /// (over the PID, recorded boot id, and observed liveness) that the recovery
    /// layer folds into its idempotency key, so repeated restarts that observe the
    /// same runtime state never emit a second recovery event.
    #[cfg(unix)]
    pub fn probe_run_health(external_pid: u32, recorded_boot_id: Option<&str>) -> RunHealthProbe {
        let current_boot_id = boot_id();
        // Only trust a PID within the boot that recorded it: a PID persisted
        // before a reboot is almost certainly recycled onto an unrelated group
        // afterwards, so it must never be read as "our run still alive".
        let same_boot = match (recorded_boot_id, current_boot_id.as_deref()) {
            (Some(recorded), Some(current)) => recorded == current,
            _ => false,
        };
        let alive = same_boot && process_group_is_alive(external_pid);
        let state = if alive {
            RuntimeHealthState::Alive
        } else {
            RuntimeHealthState::Exited
        };
        RunHealthProbe {
            external_pid: Some(external_pid),
            state,
            observed_state_hash: orphan_state_hash(
                external_pid,
                recorded_boot_id,
                state.observed_state(),
            ),
        }
    }

    /// Non-Unix fallback: with no portable process-group probe, a previously
    /// in-flight run is conservatively classified as exited (Capo never spawns
    /// process groups off Unix).
    #[cfg(not(unix))]
    pub fn probe_run_health(external_pid: u32, recorded_boot_id: Option<&str>) -> RunHealthProbe {
        let state = RuntimeHealthState::Exited;
        RunHealthProbe {
            external_pid: Some(external_pid),
            state,
            observed_state_hash: orphan_state_hash(
                external_pid,
                recorded_boot_id,
                state.observed_state(),
            ),
        }
    }

    fn control(
        &self,
        process: &LocalRuntimeProcessRef,
        status: &str,
        event_kind: &str,
        reason: &str,
    ) -> RuntimeControlResult {
        let process = LocalRuntimeProcessRef {
            status: status.to_string(),
            ..process.clone()
        };
        RuntimeControlResult {
            process: process.clone(),
            events: vec![RuntimeEvent {
                kind: event_kind.to_string(),
                status: status.to_string(),
                detail: reason.to_string(),
            }],
        }
    }

    fn remove_raw_output_files(&self, process: &LocalRunningProcess) {
        let _ = fs::remove_file(&process.stdout_path);
        let _ = fs::remove_file(&process.stderr_path);
    }

    fn terminate_process_group(&self, process: &LocalRunningProcess) {
        #[cfg(unix)]
        {
            if let Some(pid) = process.process.external_pid {
                kill_process_group(pid, "-TERM");
                thread::sleep(Duration::from_millis(100));
                kill_process_group(pid, "-KILL");
            }
        }
    }

    /// The runner's immutable configuration (workspace roots, artifact root,
    /// env allowlist, redaction rules, output cap). Used by the streaming runner
    /// which reuses the same configuration surface.
    pub(crate) fn config(&self) -> &LocalProcessConfig {
        &self.config
    }

    fn apply_request_env(
        &self,
        command: &mut Command,
        request: &LocalProcessRequest,
    ) -> RuntimeResult<()> {
        for (name, value) in &request.env {
            if self
                .config
                .env_allowlist
                .iter()
                .any(|allowed| allowed == name)
            {
                command.env(name, value);
            } else {
                return Err(RuntimeError::DisallowedEnvOverride(name.clone()));
            }
        }
        Ok(())
    }

    fn redact_output(&self, bytes: &[u8]) -> RedactedOutput {
        // ACI7: process stdout/stderr is the classic place credentials leak, so
        // the runner applies the full redaction policy (operator patterns PLUS
        // the default credential-shape scan), not only the literal patterns.
        let (bytes, redaction_state) =
            RedactionPolicy::new(self.config.redaction_rules.clone()).apply(bytes);
        RedactedOutput {
            bytes,
            redaction_state,
        }
    }

    pub(crate) fn ensure_cwd_allowed(&self, cwd: &Path) -> RuntimeResult<()> {
        let cwd = normalize_path(cwd)?;
        let allowed = self.config.workspace_roots.iter().any(|root| {
            normalize_path(root)
                .map(|root| cwd.starts_with(root))
                .unwrap_or(false)
        });
        if allowed {
            Ok(())
        } else {
            Err(RuntimeError::CwdOutsideWorkspace {
                cwd,
                workspace_roots: self.config.workspace_roots.clone(),
            })
        }
    }

    /// Resolve the artifact directory for a run/turn.
    ///
    /// With no turn key this is the legacy `artifact_root/run_id`. With a turn
    /// key the artifacts are nested under `artifact_root/run_id/turns/<turn_id>`
    /// so multiple turns in the same run keep distinct `stdout.txt`/`stderr.txt`.
    pub(crate) fn run_dir_for(&self, run_id: &RunId, turn_id: Option<&str>) -> PathBuf {
        let run_dir = self.config.artifact_root.join(run_id.as_str());
        match turn_id {
            Some(turn_id) => run_dir.join("turns").join(sanitize_artifact_key(turn_id)),
            None => run_dir,
        }
    }

    fn write_artifact(
        &self,
        run_id: &RunId,
        turn_id: Option<&str>,
        stream: &str,
        bytes: &[u8],
        redaction_state: &str,
    ) -> RuntimeResult<RuntimeOutputArtifact> {
        let run_dir = self.run_dir_for(run_id, turn_id);
        fs::create_dir_all(&run_dir)?;
        let path = run_dir.join(format!("{stream}.txt"));
        fs::write(&path, bytes)?;
        Ok(RuntimeOutputArtifact {
            artifact_id: artifact_id_for(run_id, turn_id, stream),
            path,
            size_bytes: bytes.len() as i64,
            content_hash: content_hash(bytes),
            redaction_state: redaction_state.to_string(),
            truncated: false,
        })
    }

    fn output_artifact_from_path(
        &self,
        run_id: &RunId,
        turn_id: Option<&str>,
        stream: &str,
        path: &Path,
        bytes: &[u8],
        redaction_state: &str,
    ) -> RuntimeOutputArtifact {
        RuntimeOutputArtifact {
            artifact_id: artifact_id_for(run_id, turn_id, stream),
            path: path.to_path_buf(),
            size_bytes: bytes.len() as i64,
            content_hash: content_hash(bytes),
            redaction_state: redaction_state.to_string(),
            truncated: false,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalProcessRequest {
    pub run_id: RunId,
    /// Optional per-turn key.
    ///
    /// When set, the runtime keys the artifact directory and artifact ids per
    /// `(run_id, turn_id)` so multiple turns in the same run no longer overwrite
    /// each other's `stdout`/`stderr`. When `None`, the legacy single-turn
    /// `run_dir = artifact_root/run_id` layout (one `stdout.txt`/`stderr.txt`) is
    /// preserved byte-for-byte for callers that have no turn (tool wrappers,
    /// single-turn dispatch runs).
    pub turn_id: Option<String>,
    pub program: String,
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    pub env: HashMap<String, String>,
}

impl LocalProcessRequest {
    /// Construct a request with no per-turn key (legacy single-turn layout).
    pub fn new(
        run_id: RunId,
        program: impl Into<String>,
        argv: Vec<String>,
        cwd: PathBuf,
        env: HashMap<String, String>,
    ) -> Self {
        Self {
            run_id,
            turn_id: None,
            program: program.into(),
            argv,
            cwd,
            env,
        }
    }

    /// Attach a per-turn key so this request's artifacts are keyed by `turn_id`.
    pub fn with_turn_id(mut self, turn_id: impl Into<String>) -> Self {
        self.turn_id = Some(turn_id.into());
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RedactedOutput {
    bytes: Vec<u8>,
    redaction_state: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalRuntimeProcessRef {
    pub run_id: RunId,
    pub runtime_process_ref: String,
    pub external_pid: Option<u32>,
    /// The machine boot id ([`boot_id`]) observed when this process was spawned.
    /// Persisted alongside the PID so restart recovery only reaps the persisted
    /// process group within the same boot (a reused PID after a reboot must not
    /// be signalled). `None` when the boot id was unreadable at spawn time.
    pub boot_id: Option<String>,
    pub status: String,
    pub redaction_state: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalProcessOutcome {
    pub process: LocalRuntimeProcessRef,
    pub stdout: RuntimeOutputArtifact,
    pub stderr: RuntimeOutputArtifact,
    pub exit_code: Option<i32>,
    pub events: Vec<RuntimeEvent>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeControlResult {
    pub process: LocalRuntimeProcessRef,
    pub events: Vec<RuntimeEvent>,
}

#[derive(Debug)]
pub struct LocalRunningProcess {
    pub process: LocalRuntimeProcessRef,
    child: Child,
    /// The per-turn artifact key, if this run was spawned for a specific turn.
    turn_id: Option<String>,
    stdout_path: PathBuf,
    stderr_path: PathBuf,
    pub events: Vec<RuntimeEvent>,
}

/// A runtime-owned process spawned with PIPED stdin+stdout for a bidirectional
/// line protocol (see [`LocalProcessRunner::spawn_piped_process`]).
///
/// The runtime owns the process group; the caller borrows the pipe handles to
/// drive the protocol and calls [`Self::wait`] (or drops the handle) to reap the
/// child. The pipe handles are `take`-able exactly once so a wire client can own
/// them for the lifetime of the protocol.
#[derive(Debug)]
pub struct PipedRunningProcess {
    pub process: LocalRuntimeProcessRef,
    child: Child,
    stdin: Option<std::process::ChildStdin>,
    stdout: Option<std::process::ChildStdout>,
    stderr_path: PathBuf,
    pub events: Vec<RuntimeEvent>,
}

impl PipedRunningProcess {
    /// Take the child's stdin pipe (writable) exactly once.
    pub fn take_stdin(&mut self) -> Option<std::process::ChildStdin> {
        self.stdin.take()
    }

    /// Take the child's stdout pipe (readable) exactly once.
    pub fn take_stdout(&mut self) -> Option<std::process::ChildStdout> {
        self.stdout.take()
    }

    /// The artifact path the child's stderr is captured to.
    pub fn stderr_path(&self) -> &Path {
        &self.stderr_path
    }

    /// Signal the whole process group and reap the child, returning its exit
    /// status string. Closing the wire (dropping the taken stdin) typically lets
    /// a well-behaved agent exit; this is the explicit teardown path.
    pub fn shutdown(&mut self, reason: &str) -> RuntimeControlResult {
        // Drop any retained pipe handle so the child sees EOF on stdin.
        self.stdin = None;
        #[cfg(unix)]
        if let Some(pid) = self.process.external_pid {
            kill_process_group(pid, "-TERM");
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.process.status = "exited".to_string();
        RuntimeControlResult {
            process: self.process.clone(),
            events: vec![RuntimeEvent {
                kind: "runtime.stop_requested".to_string(),
                status: "exited".to_string(),
                detail: reason.to_string(),
            }],
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeOutputArtifact {
    pub artifact_id: String,
    pub path: PathBuf,
    pub size_bytes: i64,
    pub content_hash: String,
    pub redaction_state: String,
    /// Whether the captured output was truncated at the output cap.
    ///
    /// The synchronous runner never truncates (it errors on overflow and the
    /// tool wrappers buffer the whole output), so it always records `false`.
    /// The streaming runner ([`AsyncLocalProcessRunner`]) streams-and-truncates:
    /// a successful run that exceeds the cap keeps its (capped) artifact and
    /// records `truncated = true` here as artifact metadata rather than failing
    /// the run.
    pub truncated: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeEvent {
    pub kind: String,
    pub status: String,
    pub detail: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeHealth {
    pub runtime_process_ref: String,
    pub status: String,
    pub live: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CleanupReport {
    pub runtime_process_ref: String,
    pub preserved_artifact_dir: PathBuf,
    pub marker_path: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OrphanRecovery {
    pub runtime_process_ref: String,
    pub recovered_status: String,
    pub detail: String,
}

/// The result of probing and reaping an orphaned process group by PID on
/// restart (RTL10). See [`LocalProcessRunner::reap_orphan_process_group`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OrphanReap {
    /// The PID Capo persisted before the spawn returned.
    pub external_pid: u32,
    /// `true` if the process was still alive and was reaped; `false` if it had
    /// already exited (a true orphan whose terminal status is unknown).
    pub reaped: bool,
    /// The observed runtime state: `alive_reaped` or `already_gone`. A group
    /// observed under a different boot id than was recorded at spawn time (a
    /// recycled PID after a reboot) is reported as `already_gone` without being
    /// signalled.
    pub observed_state: String,
    /// A stable hash over `(external_pid, recorded_boot_id, observed_state)` for
    /// the recovery idempotency key.
    pub observed_runtime_state_hash: String,
}

/// SG9: how a restart observed a previously in-flight run's persisted
/// process-group when probing its liveness NON-destructively
/// ([`LocalProcessRunner::probe_run_health`]).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeHealthState {
    /// The process group is still alive within the recording boot, so the run is
    /// reattachable in place (the recovery layer records `run.recovered`).
    Alive,
    /// The process group is gone (or its boot id could not be confirmed against
    /// the current boot), so the run terminated while Capo was down (the recovery
    /// layer records a terminal `run.exited`).
    Exited,
}

impl RuntimeHealthState {
    /// Whether the probed run is still alive and reattachable.
    pub const fn is_alive(self) -> bool {
        matches!(self, Self::Alive)
    }

    /// The stable observed-state token folded into the recovery idempotency hash.
    pub const fn observed_state(self) -> &'static str {
        match self {
            Self::Alive => "alive",
            Self::Exited => "exited",
        }
    }
}

/// SG9: the result of NON-destructively probing a previously in-flight run's
/// liveness on restart by its persisted PID. See
/// [`LocalProcessRunner::probe_run_health`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunHealthProbe {
    /// The PID Capo persisted before the spawn returned, if one was recorded.
    pub external_pid: Option<u32>,
    /// Whether the run is still alive (reattachable) or has exited.
    pub state: RuntimeHealthState,
    /// A stable hash over `(external_pid, recorded_boot_id, observed_state)` for
    /// the recovery idempotency key, so a repeated restart observing the same
    /// runtime state never emits a second recovery event.
    pub observed_state_hash: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteProcessConfig {
    pub remote_target_id: String,
    pub endpoint_ref: String,
    pub local_loopback: LocalProcessConfig,
}

impl RemoteProcessConfig {
    pub fn loopback_for_test(
        remote_target_id: impl Into<String>,
        endpoint_ref: impl Into<String>,
        workspace_root: PathBuf,
        artifact_root: PathBuf,
    ) -> Self {
        Self {
            remote_target_id: remote_target_id.into(),
            endpoint_ref: endpoint_ref.into(),
            local_loopback: LocalProcessConfig::for_test(workspace_root, artifact_root),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteProcessRunner {
    config: RemoteProcessConfig,
    loopback: LocalProcessRunner,
}

impl RemoteProcessRunner {
    pub fn new(config: RemoteProcessConfig) -> Self {
        let loopback = LocalProcessRunner::new(config.local_loopback.clone());
        Self { config, loopback }
    }

    pub fn binding(&self) -> BoundaryBinding {
        BoundaryBinding {
            kind: BoundaryKind::RuntimeRunner,
            variant: "remote-process",
            fake: false,
        }
    }

    pub fn start_process(
        &self,
        request: LocalProcessRequest,
    ) -> RuntimeResult<LocalProcessOutcome> {
        let mut outcome = self.loopback.start_process(request)?;
        outcome.process.runtime_process_ref = self.remote_ref(&outcome.process.runtime_process_ref);
        prepend_remote_events(
            &self.config,
            &mut outcome.events,
            &outcome.process.runtime_process_ref,
        );
        Ok(outcome)
    }

    pub fn interrupt(
        &self,
        process: &LocalRuntimeProcessRef,
        reason: &str,
    ) -> RuntimeControlResult {
        self.remote_control(
            process,
            "interrupting",
            "runtime.remote_interrupt_sent",
            reason,
        )
    }

    pub fn terminate(
        &self,
        process: &LocalRuntimeProcessRef,
        reason: &str,
    ) -> RuntimeControlResult {
        self.remote_control(
            process,
            "terminating",
            "runtime.remote_terminate_sent",
            reason,
        )
    }

    pub fn health(&self, process: &LocalRuntimeProcessRef) -> RuntimeHealth {
        RuntimeHealth {
            runtime_process_ref: process.runtime_process_ref.clone(),
            status: format!("remote:{}:{}", self.config.remote_target_id, process.status),
            live: process.status == "running",
        }
    }

    pub fn recover_orphan(&self, process: &LocalRuntimeProcessRef) -> OrphanRecovery {
        let health = self.health(process);
        OrphanRecovery {
            runtime_process_ref: process.runtime_process_ref.clone(),
            recovered_status: if health.live {
                "remote_recovered"
            } else {
                "remote_orphaned"
            }
            .to_string(),
            detail: format!(
                "remote target {} via endpoint {} reported {}",
                self.config.remote_target_id, self.config.endpoint_ref, health.status
            ),
        }
    }

    fn remote_control(
        &self,
        process: &LocalRuntimeProcessRef,
        status: &str,
        event_kind: &str,
        reason: &str,
    ) -> RuntimeControlResult {
        let process = LocalRuntimeProcessRef {
            status: status.to_string(),
            ..process.clone()
        };
        RuntimeControlResult {
            process: process.clone(),
            events: vec![RuntimeEvent {
                kind: event_kind.to_string(),
                status: status.to_string(),
                detail: format!(
                    "target={} endpoint={} reason={}",
                    self.config.remote_target_id, self.config.endpoint_ref, reason
                ),
            }],
        }
    }

    fn remote_ref(&self, local_ref: &str) -> String {
        format!(
            "remote-process:{}:{}:{}",
            self.config.remote_target_id, self.config.endpoint_ref, local_ref
        )
    }
}

fn prepend_remote_events(
    config: &RemoteProcessConfig,
    events: &mut Vec<RuntimeEvent>,
    runtime_process_ref: &str,
) {
    let mut prefixed = vec![
        RuntimeEvent {
            kind: "runtime.remote_target_resolved".to_string(),
            status: "resolved".to_string(),
            detail: format!(
                "target={} endpoint={}",
                config.remote_target_id, config.endpoint_ref
            ),
        },
        RuntimeEvent {
            kind: "runtime.remote_process_started".to_string(),
            status: "started".to_string(),
            detail: runtime_process_ref.to_string(),
        },
    ];
    prefixed.append(events);
    *events = prefixed;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConnectivityTunnel {
    Fake(FakeTunnel),
    LocalLoopback(LocalLoopbackTunnel),
    EndpointStub(EndpointStubTunnel),
}

impl ConnectivityTunnel {
    pub fn fake() -> Self {
        Self::Fake(FakeTunnel)
    }

    pub fn local_loopback() -> Self {
        Self::LocalLoopback(LocalLoopbackTunnel)
    }

    pub fn endpoint_stub(config: ConnectivityEndpointConfig) -> Self {
        Self::EndpointStub(EndpointStubTunnel::new(config))
    }

    pub fn binding(&self) -> BoundaryBinding {
        match self {
            Self::Fake(tunnel) => tunnel.binding(),
            Self::LocalLoopback(tunnel) => tunnel.binding(),
            Self::EndpointStub(tunnel) => tunnel.binding(),
        }
    }

    pub fn resolve_endpoint(
        &self,
        owner: EndpointOwner,
        channel_kind: ChannelKind,
    ) -> ConnectivityResult<ResolvedEndpoint> {
        match self {
            Self::Fake(tunnel) => tunnel.resolve_endpoint(owner, channel_kind),
            Self::LocalLoopback(tunnel) => tunnel.resolve_endpoint(owner, channel_kind),
            Self::EndpointStub(tunnel) => tunnel.resolve_endpoint(owner, channel_kind),
        }
    }

    pub fn check_reachability(&self) -> ConnectivityHealth {
        match self {
            Self::Fake(tunnel) => tunnel.check_reachability(),
            Self::LocalLoopback(tunnel) => tunnel.check_reachability(),
            Self::EndpointStub(tunnel) => tunnel.check_reachability(),
        }
    }

    pub fn exposure_report(&self) -> ExposureReport {
        match self {
            Self::Fake(tunnel) => tunnel.exposure_report(),
            Self::LocalLoopback(tunnel) => tunnel.exposure_report(),
            Self::EndpointStub(tunnel) => tunnel.exposure_report(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeTunnel;

impl FakeTunnel {
    pub fn binding(&self) -> BoundaryBinding {
        BoundaryBinding::fake(BoundaryKind::ConnectivityTunnel, "fake-tunnel")
    }

    pub fn resolve_endpoint(
        &self,
        owner: EndpointOwner,
        channel_kind: ChannelKind,
    ) -> ConnectivityResult<ResolvedEndpoint> {
        Ok(ResolvedEndpoint::new(
            "fake-endpoint",
            owner,
            channel_kind,
            "fake://endpoint",
            ExposureScope::Loopback,
            false,
        ))
    }

    pub fn check_reachability(&self) -> ConnectivityHealth {
        ConnectivityHealth {
            endpoint_id: "fake-endpoint".to_string(),
            status: "available".to_string(),
            reachable: true,
            exposure: ExposureScope::Loopback,
            detail: "fake tunnel is always reachable in tests".to_string(),
        }
    }

    pub fn exposure_report(&self) -> ExposureReport {
        ExposureReport::for_exposure("fake-endpoint", ExposureScope::Loopback)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalLoopbackTunnel;

impl LocalLoopbackTunnel {
    pub fn binding(&self) -> BoundaryBinding {
        BoundaryBinding {
            kind: BoundaryKind::ConnectivityTunnel,
            variant: "local-loopback",
            fake: false,
        }
    }

    pub fn resolve_endpoint(
        &self,
        owner: EndpointOwner,
        channel_kind: ChannelKind,
    ) -> ConnectivityResult<ResolvedEndpoint> {
        if !channel_kind.is_loopback_safe() {
            return Err(ConnectivityError::ChannelNotAllowed {
                endpoint_id: "local-loopback".to_string(),
                channel_kind,
            });
        }

        Ok(ResolvedEndpoint::new(
            "local-loopback",
            owner,
            channel_kind,
            "http://127.0.0.1",
            ExposureScope::Loopback,
            false,
        ))
    }

    pub fn check_reachability(&self) -> ConnectivityHealth {
        ConnectivityHealth {
            endpoint_id: "local-loopback".to_string(),
            status: "available".to_string(),
            reachable: true,
            exposure: ExposureScope::Loopback,
            detail: "loopback endpoint resolves to localhost only".to_string(),
        }
    }

    pub fn exposure_report(&self) -> ExposureReport {
        ExposureReport::for_exposure("local-loopback", ExposureScope::Loopback)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EndpointStubTunnel {
    config: ConnectivityEndpointConfig,
}

impl EndpointStubTunnel {
    pub fn new(config: ConnectivityEndpointConfig) -> Self {
        Self { config }
    }

    pub fn binding(&self) -> BoundaryBinding {
        BoundaryBinding {
            kind: BoundaryKind::ConnectivityTunnel,
            variant: "endpoint-stub",
            fake: false,
        }
    }

    pub fn resolve_endpoint(
        &self,
        owner: EndpointOwner,
        channel_kind: ChannelKind,
    ) -> ConnectivityResult<ResolvedEndpoint> {
        if !self.config.allowed_channels.contains(&channel_kind) {
            return Err(ConnectivityError::ChannelNotAllowed {
                endpoint_id: self.config.endpoint_id.clone(),
                channel_kind,
            });
        }

        Ok(ResolvedEndpoint::new(
            self.config.endpoint_id.clone(),
            owner,
            channel_kind,
            self.config.resolved_uri(),
            self.config.exposure,
            self.config.exposure.requires_permission(),
        ))
    }

    pub fn check_reachability(&self) -> ConnectivityHealth {
        ConnectivityHealth {
            endpoint_id: self.config.endpoint_id.clone(),
            status: self.config.status.clone(),
            reachable: self.config.status == "available",
            exposure: self.config.exposure,
            detail: format!(
                "stub endpoint {} via {}",
                self.config.endpoint_id, self.config.tunnel_kind
            ),
        }
    }

    pub fn exposure_report(&self) -> ExposureReport {
        ExposureReport::for_exposure(&self.config.endpoint_id, self.config.exposure)
    }
}

pub type ConnectivityResult<T> = Result<T, ConnectivityError>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConnectivityError {
    ChannelNotAllowed {
        endpoint_id: String,
        channel_kind: ChannelKind,
    },
    /// CT1: a non-loopback (`Private`/`Public`) bind/connect/resolution was
    /// requested with no `auth_ref` HANDLE attached. The bind/connect side and
    /// the exposure-stub side both fail closed with this rather than silently
    /// allowing an unauthenticated non-loopback exposure. Carries no secret.
    AuthRequired { scope: ExposureScope },
    /// CT1: the requested `ExposureScope` exceeds the effective policy ceiling,
    /// which defaults to `Loopback` and is only promoted by explicit opt-in
    /// (config/flag/grant). A non-loopback request under an unpromoted (default)
    /// policy fails closed here — the loopback default is never implicitly
    /// widened.
    ScopeExceedsCeiling {
        requested: ExposureScope,
        ceiling: ExposureScope,
    },
}

impl std::fmt::Display for ConnectivityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ChannelNotAllowed {
                endpoint_id,
                channel_kind,
            } => write!(
                f,
                "channel {} is not allowed for endpoint {endpoint_id}",
                channel_kind.as_str()
            ),
            Self::AuthRequired { scope } => write!(
                f,
                "non-loopback exposure ({}) requires an auth_ref handle",
                scope.as_str()
            ),
            Self::ScopeExceedsCeiling { requested, ceiling } => write!(
                f,
                "requested exposure scope {} exceeds the effective policy ceiling {}",
                requested.as_str(),
                ceiling.as_str()
            ),
        }
    }
}

impl std::error::Error for ConnectivityError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExposureScope {
    Loopback,
    Private,
    Public,
}

impl ExposureScope {
    pub fn permission_scope(self) -> &'static str {
        match self {
            Self::Loopback => "network:connect:localhost",
            Self::Private => "network:connect:private_tunnel",
            Self::Public => "network:expose:public",
        }
    }

    pub fn requires_permission(self) -> bool {
        !matches!(self, Self::Loopback)
    }

    /// Stable wire label for the scope, used by the `connectivity.policy_changed`
    /// audit payload and the typed error `Display`. Mirrors the CLI's
    /// `exposure_scope_str` so the policy and the exposure trail share one
    /// vocabulary.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Loopback => "loopback",
            Self::Private => "private",
            Self::Public => "public",
        }
    }

    /// Rank scopes so the policy can compare a requested scope against the
    /// effective ceiling: `Loopback < Private < Public`. A request is permitted
    /// by the ceiling only when `request.rank() <= ceiling.rank()`.
    fn rank(self) -> u8 {
        match self {
            Self::Loopback => 0,
            Self::Private => 1,
            Self::Public => 2,
        }
    }
}

/// CT1: the explicit gate the server bind, the client connect, and tunnel
/// resolution consult before any non-loopback exposure.
///
/// The DEFAULT ceiling is [`ExposureScope::Loopback`] — loopback passes with
/// zero config and is byte-for-byte the prior behavior. Promotion to `Private`
/// or `Public` is an EXPLICIT opt-in (config/flag/grant), never an implicit
/// default, and is itself an audited fact (`connectivity.policy_changed`, built
/// via [`ExposurePolicy::promote`]). A non-loopback request fails closed unless
/// the ceiling was promoted AND an `auth_ref` handle is present.
///
/// This is the connectivity boundary's policy only; the `safety-gates` grant
/// engine still independently gates ACTIVATION via the permission scope. The two
/// checks are separate and both required for a live private/public exposure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExposurePolicy {
    ceiling: ExposureScope,
    opt_in_source: Option<String>,
}

impl Default for ExposurePolicy {
    fn default() -> Self {
        // Loopback default with no opt-in source: the safe zero-config policy.
        Self {
            ceiling: ExposureScope::Loopback,
            opt_in_source: None,
        }
    }
}

/// The replay-stable audit fact emitted when the effective exposure ceiling is
/// promoted (Loopback -> Private/Public). It records the old/new ceiling, the
/// opt-in SOURCE (config/flag/grant), and a caller-supplied timestamp so an
/// operator can reconstruct WHY a private/public exposure became possible. It
/// carries NO secret — the opt-in source is a provenance label, never a handle
/// or credential value.
///
/// FORWARD-COMPATIBLE STUB (CT1): `promote()` and this event type, together with
/// the `EventKind::ConnectivityPolicyChanged` codec, are wired and round-trip
/// tested, but CT1 has NO live emitter — `promote()` is exercised only in tests.
/// The opt-in promotion CLI path (a `--promote`/grant-driven flag that actually
/// emits `connectivity.policy_changed` into the state store) lands in CT3/CT5.
/// Until then the default loopback-only policy is the only one the live bind /
/// connect / expose-stub paths construct, so the audit trail for promotions is
/// deliberately not yet live. Do not assume a populated `policy_changed` history
/// exists in the store before that path is wired.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicyChangeEvent {
    pub previous_ceiling: ExposureScope,
    pub new_ceiling: ExposureScope,
    pub opt_in_source: String,
    pub changed_at: String,
}

impl PolicyChangeEvent {
    /// The `connectivity.policy_changed` payload as a stable, secret-free JSON
    /// object. Replay-stable: the same inputs always produce the same bytes.
    pub fn payload_json(&self) -> String {
        format!(
            "{{\"previous_ceiling\":\"{}\",\"new_ceiling\":\"{}\",\"opt_in_source\":\"{}\",\"changed_at\":\"{}\"}}",
            self.previous_ceiling.as_str(),
            self.new_ceiling.as_str(),
            escape_policy_json(&self.opt_in_source),
            escape_policy_json(&self.changed_at),
        )
    }
}

/// Minimal JSON string escaping for the policy-change provenance labels. The
/// opt-in source / timestamp are operator-facing provenance, never secrets, but
/// they still must not break the payload framing.
fn escape_policy_json(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            other => escaped.push(other),
        }
    }
    escaped
}

impl ExposurePolicy {
    /// The default loopback-only policy: loopback passes with no auth and no
    /// opt-in; any non-loopback request fails closed.
    pub fn loopback_default() -> Self {
        Self::default()
    }

    pub fn ceiling(&self) -> ExposureScope {
        self.ceiling
    }

    pub fn opt_in_source(&self) -> Option<&str> {
        self.opt_in_source.as_deref()
    }

    /// Promote the effective ceiling to `new_ceiling` via an EXPLICIT opt-in
    /// `source` (config/flag/grant), returning the promoted policy together with
    /// the replay-stable [`PolicyChangeEvent`] the caller must emit. Promotion to
    /// `Loopback` (or to the same/lower ceiling) is not a widening and returns no
    /// event.
    ///
    /// `source` is a PROVENANCE TOKEN — a short label naming WHERE the opt-in came
    /// from (e.g. `config`, `flag:--expose-private`, `grant:<grant-id>`), never a
    /// credential. It is serialized verbatim into the secret-free
    /// [`PolicyChangeEvent::payload_json`] audit payload, so the CT3/CT5 caller
    /// that wires the live promotion path MUST pass a provenance label here and
    /// resolve any real credential through the `auth_ref` HANDLE confined to the
    /// adapter (CT2), not through this string. CT2's redaction guard scans handle
    /// fields, not this free-text provenance label; when the live promotion path
    /// lands (CT3/CT5) it must add a defense-in-depth test asserting
    /// `payload_json()` contains no known credential pattern, mirroring the CT2
    /// planted-pattern net.
    pub fn promote(
        &self,
        new_ceiling: ExposureScope,
        source: impl Into<String>,
        changed_at: impl Into<String>,
    ) -> (Self, Option<PolicyChangeEvent>) {
        let source = source.into();
        if new_ceiling.rank() <= self.ceiling.rank() {
            // Not a widening: keep the current (or stricter) ceiling, no audit.
            return (self.clone(), None);
        }
        let event = PolicyChangeEvent {
            previous_ceiling: self.ceiling,
            new_ceiling,
            opt_in_source: source.clone(),
            changed_at: changed_at.into(),
        };
        (
            Self {
                ceiling: new_ceiling,
                opt_in_source: Some(source),
            },
            Some(event),
        )
    }

    /// Authorize a requested `scope` against this policy.
    ///
    /// - `Loopback` always passes (no auth, no opt-in needed).
    /// - A non-loopback scope FAILS CLOSED with [`ConnectivityError::AuthRequired`]
    ///   when no `auth_ref` handle is attached.
    /// - A non-loopback scope above the effective ceiling FAILS CLOSED with
    ///   [`ConnectivityError::ScopeExceedsCeiling`].
    ///
    /// On success it returns `Ok(permission_required)` — `false` only for
    /// loopback; `true` for an authorized private/public scope (the grant engine
    /// still gates activation).
    pub fn authorize(
        &self,
        scope: ExposureScope,
        auth_ref: Option<&str>,
    ) -> ConnectivityResult<bool> {
        if matches!(scope, ExposureScope::Loopback) {
            return Ok(false);
        }
        if auth_ref.is_none_or(str::is_empty) {
            return Err(ConnectivityError::AuthRequired { scope });
        }
        if scope.rank() > self.ceiling.rank() {
            return Err(ConnectivityError::ScopeExceedsCeiling {
                requested: scope,
                ceiling: self.ceiling,
            });
        }
        Ok(true)
    }

    /// CT1 bind/connect guard: decide whether a socket address may be served or
    /// connected under this policy. Loopback addresses always pass (the
    /// zero-config default). A non-loopback address requires the policy to have
    /// been promoted to at least `Private` AND an `auth_ref` handle present,
    /// otherwise it fails closed — symmetric on both the listener and connect
    /// sides so loosening one side cannot open an asymmetric hole.
    ///
    /// SCOPE NOTE (by design): this is a TRANSPORT-LEVEL guard, so it treats ANY
    /// non-loopback socket as [`ExposureScope::Private`] regardless of whether the
    /// bind is to a tailnet-private or a public interface — the transport does not
    /// know the difference, and must not. The Private-vs-Public distinction is an
    /// EXPOSURE-level concern enforced upstream at the [`ExposurePolicy::authorize`]
    /// call in the tunnel-resolution path (CT3+), where the requested
    /// [`ExposureScope`] is known and checked against the ceiling. Consequently a
    /// `Public`-scope bind validated here passes as long as the ceiling is at least
    /// `Private`; a future implementer wiring a public-interface bind must NOT rely
    /// on this method to reject public exposure — that gate lives in `authorize`.
    pub fn authorize_socket(
        &self,
        is_loopback: bool,
        auth_ref: Option<&str>,
    ) -> ConnectivityResult<()> {
        if is_loopback {
            return Ok(());
        }
        self.authorize(ExposureScope::Private, auth_ref).map(|_| ())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChannelKind {
    Control,
    Stdio,
    Logs,
    Dashboard,
    Artifact,
}

impl ChannelKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Control => "control",
            Self::Stdio => "stdio",
            Self::Logs => "logs",
            Self::Dashboard => "dashboard",
            Self::Artifact => "artifact",
        }
    }

    fn is_loopback_safe(self) -> bool {
        matches!(self, Self::Control | Self::Dashboard | Self::Artifact)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EndpointOwner {
    pub owner_kind: String,
    pub owner_id: String,
}

impl EndpointOwner {
    pub fn runtime_target(owner_id: impl Into<String>) -> Self {
        Self {
            owner_kind: "runtime_target".to_string(),
            owner_id: owner_id.into(),
        }
    }

    pub fn capo_server(owner_id: impl Into<String>) -> Self {
        Self {
            owner_kind: "capo_server".to_string(),
            owner_id: owner_id.into(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConnectivityEndpointConfig {
    pub endpoint_id: String,
    pub name: String,
    pub tunnel_kind: String,
    pub address_ref: String,
    pub exposure: ExposureScope,
    pub allowed_channels: Vec<ChannelKind>,
    pub status: String,
}

impl ConnectivityEndpointConfig {
    pub fn stub_private(endpoint_id: impl Into<String>, address_ref: impl Into<String>) -> Self {
        Self {
            endpoint_id: endpoint_id.into(),
            name: "private endpoint stub".to_string(),
            tunnel_kind: "endpoint-stub".to_string(),
            address_ref: address_ref.into(),
            exposure: ExposureScope::Private,
            allowed_channels: vec![ChannelKind::Control, ChannelKind::Stdio, ChannelKind::Logs],
            status: "available".to_string(),
        }
    }

    pub fn stub_public(endpoint_id: impl Into<String>, address_ref: impl Into<String>) -> Self {
        Self {
            endpoint_id: endpoint_id.into(),
            name: "public endpoint stub".to_string(),
            tunnel_kind: "endpoint-stub".to_string(),
            address_ref: address_ref.into(),
            exposure: ExposureScope::Public,
            allowed_channels: vec![ChannelKind::Dashboard],
            status: "available".to_string(),
        }
    }

    fn resolved_uri(&self) -> String {
        format!(
            "stub://{}/{}",
            self.endpoint_id,
            self.address_ref.trim_start_matches('/')
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedEndpoint {
    pub resolved_endpoint_id: String,
    pub connectivity_endpoint_id: String,
    pub owner: EndpointOwner,
    pub channel_kind: ChannelKind,
    pub resolved_uri: String,
    pub exposure: ExposureScope,
    pub permission_scope: String,
    pub permission_required: bool,
}

impl ResolvedEndpoint {
    fn new(
        connectivity_endpoint_id: impl Into<String>,
        owner: EndpointOwner,
        channel_kind: ChannelKind,
        resolved_uri: impl Into<String>,
        exposure: ExposureScope,
        permission_required: bool,
    ) -> Self {
        let connectivity_endpoint_id = connectivity_endpoint_id.into();
        let resolved_uri = resolved_uri.into();
        let resolved_endpoint_id = format!(
            "{}:{}:{}:{}",
            connectivity_endpoint_id,
            owner.owner_kind,
            owner.owner_id,
            channel_kind.as_str()
        );
        Self {
            resolved_endpoint_id,
            connectivity_endpoint_id,
            owner,
            channel_kind,
            resolved_uri,
            exposure,
            permission_scope: exposure.permission_scope().to_string(),
            permission_required,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConnectivityHealth {
    pub endpoint_id: String,
    pub status: String,
    pub reachable: bool,
    pub exposure: ExposureScope,
    pub detail: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExposureReport {
    pub endpoint_id: String,
    pub exposure: ExposureScope,
    pub permission_scope: String,
    pub permission_required: bool,
    pub audit_event_kind: String,
}

impl ExposureReport {
    fn for_exposure(endpoint_id: impl Into<String>, exposure: ExposureScope) -> Self {
        Self {
            endpoint_id: endpoint_id.into(),
            exposure,
            permission_scope: exposure.permission_scope().to_string(),
            permission_required: exposure.requires_permission(),
            audit_event_kind: "connectivity.exposure_changed".to_string(),
        }
    }
}

fn capped_output(bytes: Vec<u8>, limit_bytes: usize) -> RuntimeResult<Vec<u8>> {
    if bytes.len() > limit_bytes {
        Err(RuntimeError::OutputLimitExceeded {
            limit_bytes,
            actual_bytes: bytes.len(),
        })
    } else {
        Ok(bytes)
    }
}

pub(crate) fn normalize_path(path: &Path) -> RuntimeResult<PathBuf> {
    if path.exists() {
        Ok(path.canonicalize()?)
    } else {
        Ok(path.to_path_buf())
    }
}

/// Build the artifact id for a run/turn stream.
///
/// With no turn key this is the legacy `artifact-runtime-{run_id}-{stream}`.
/// With a turn key the turn is folded in so per-turn artifacts in the same run
/// have distinct ids: `artifact-runtime-{run_id}-turn-{turn_id}-{stream}`.
pub(crate) fn artifact_id_for(run_id: &RunId, turn_id: Option<&str>, stream: &str) -> String {
    match turn_id {
        Some(turn_id) => format!(
            "artifact-runtime-{run_id}-turn-{}-{stream}",
            sanitize_artifact_key(turn_id)
        ),
        None => format!("artifact-runtime-{run_id}-{stream}"),
    }
}

/// Sanitize a turn key for use as a path segment and artifact-id component.
///
/// Keeps the key filesystem-safe and free of separators so it cannot escape the
/// run directory or collide across turns.
pub(crate) fn sanitize_artifact_key(key: &str) -> String {
    let sanitized: String = key
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect();
    if sanitized.is_empty() {
        "turn".to_string()
    } else {
        sanitized
    }
}

pub(crate) fn content_hash(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("fnv1a64:{hash:016x}")
}

/// The lowest PID the orphan reaper will ever signal as a process *group*.
///
/// `kill -<pid>` targets a process *group*, and the low group ids are
/// catastrophic to signal: group 0 is the *caller's own* group (so
/// `kill -KILL -0` would SIGKILL Capo and every child it spawned), and group 1
/// is init's group. A corrupted/zero/low PID in the durable in-flight marker
/// must therefore never reach `/bin/kill`; we treat anything `<= 1` as "no
/// process to reap" (see [`is_reapable_pid`]).
const MIN_REAPABLE_PID: u32 = 2;

/// Whether `pid` is safe to use as a negative-PID process-*group* signal target.
///
/// Guards against `kill -<0|1>` (self-group / init) reaching the reaper from a
/// corrupted or zero-defaulted marker PID.
fn is_reapable_pid(pid: u32) -> bool {
    pid >= MIN_REAPABLE_PID
}

/// Probe whether the process *group* led by `pid` still has any live member,
/// without affecting it (`kill -0 -<pid>`).
#[cfg(unix)]
pub(crate) fn process_group_is_alive(pid: u32) -> bool {
    // Never probe (or, downstream, signal) the self/init groups: a low PID from
    // a corrupted marker must read as "not alive" so the reaper records
    // `already_gone` rather than `kill -0 -0` (which would succeed against our
    // own group and lead the reaper to SIGKILL it).
    if !is_reapable_pid(pid) {
        return false;
    }
    // `kill -0 -<pid>` succeeds iff at least one process in the group exists and
    // we may signal it; it never delivers a signal. We probe the *group* (not
    // the leader PID) so a backgrounded descendant whose group leader already
    // exited still reads as alive -- that descendant is exactly the orphan we
    // must reap.
    Command::new("/bin/kill")
        .arg("-0")
        .arg("--")
        .arg(format!("-{pid}"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// Send `signal` to the whole process group led by `pid` (negative PID target).
#[cfg(unix)]
pub(crate) fn kill_process_group(pid: u32, signal: &str) {
    // Defence in depth alongside `process_group_is_alive`: refuse to signal the
    // self/init groups even if a caller reaches here with a low PID.
    if !is_reapable_pid(pid) {
        return;
    }
    let _ = Command::new("/bin/kill")
        .arg(signal)
        .arg("--")
        .arg(format!("-{pid}"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// A coarse identity token for the machine's current boot, recorded alongside a
/// run's PID at spawn time (RTL10) and re-checked on restart before any reap.
///
/// PIDs (and process-group ids) are recycled freely by the OS, so a PID
/// persisted before a crash is only a meaningful handle *within the same boot*.
/// After a reboot the persisted PID/PGID is almost certainly attached to an
/// unrelated process group, and reaping it would SIGKILL an innocent group. We
/// therefore stamp each marker with the boot id and skip reaping (recording the
/// run as already gone) when it differs from the boot id observed on restart.
///
/// The token is derived from the kernel's recorded boot instant
/// (`/proc/stat`'s `btime` on Linux, `kern.boottime` on macOS). If neither is
/// readable we return `None`; callers treat an unknown boot id conservatively
/// (no reap), so an unverifiable identity never escalates to a group kill.
pub fn boot_id() -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        let stat = fs::read_to_string("/proc/stat").ok()?;
        for line in stat.lines() {
            if let Some(btime) = line.strip_prefix("btime ") {
                let btime = btime.trim();
                if !btime.is_empty() {
                    return Some(format!("linux-btime-{btime}"));
                }
            }
        }
        None
    }
    #[cfg(target_os = "macos")]
    {
        let output = Command::new("/usr/sbin/sysctl")
            .arg("-n")
            .arg("kern.boottime")
            .stderr(Stdio::null())
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&output.stdout);
        // Format: `{ sec = 1700000000, usec = 0 } Tue ...`; the `sec` field is a
        // stable per-boot value.
        let sec = text
            .split("sec =")
            .nth(1)?
            .trim_start()
            .split(|c: char| !c.is_ascii_digit())
            .next()?;
        if sec.is_empty() {
            return None;
        }
        Some(format!("macos-boottime-{sec}"))
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        None
    }
}

/// A stable hash over the observed orphan runtime state for recovery
/// idempotency. Stable across restarts that observe the same PID + recorded boot
/// id + liveness.
fn orphan_state_hash(pid: u32, recorded_boot_id: Option<&str>, observed_state: &str) -> String {
    content_hash(
        format!(
            "{pid}:{}:{observed_state}",
            recorded_boot_id.unwrap_or("no-boot-id")
        )
        .as_bytes(),
    )
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn planned_runtimes_keep_fake_and_local_process() {
        assert_eq!(
            PLANNED_RUNTIMES,
            ["fake", "local-process", "remote-process"]
        );
        assert_eq!(PLANNED_TUNNELS, ["fake", "local-loopback", "endpoint-stub"]);
    }

    #[test]
    fn fake_runtime_and_tunnel_are_separate_boundaries() {
        assert_eq!(
            RuntimeRunner::fake().binding().kind,
            BoundaryKind::RuntimeRunner
        );
        assert_eq!(
            ConnectivityTunnel::fake().binding().kind,
            BoundaryKind::ConnectivityTunnel
        );
    }

    #[test]
    fn tunnel_endpoint_stub_separates_resolution_health_and_exposure_policy() {
        let tunnel = ConnectivityTunnel::endpoint_stub(ConnectivityEndpointConfig::stub_private(
            "endpoint-private-1",
            "tailnet/capo-worker",
        ));

        assert_eq!(tunnel.binding().kind, BoundaryKind::ConnectivityTunnel);
        assert_eq!(tunnel.binding().variant, "endpoint-stub");

        let owner = EndpointOwner::runtime_target("remote-target-1");
        let resolved = tunnel
            .resolve_endpoint(owner.clone(), ChannelKind::Control)
            .expect("resolve private control endpoint");
        assert_eq!(resolved.connectivity_endpoint_id, "endpoint-private-1");
        assert_eq!(resolved.owner, owner);
        assert_eq!(resolved.channel_kind, ChannelKind::Control);
        assert_eq!(resolved.exposure, ExposureScope::Private);
        assert_eq!(resolved.permission_scope, "network:connect:private_tunnel");
        assert!(resolved.permission_required);
        assert!(
            resolved
                .resolved_endpoint_id
                .contains("runtime_target:remote-target-1:control")
        );
        assert!(resolved.resolved_uri.contains("tailnet/capo-worker"));

        let health = tunnel.check_reachability();
        assert!(health.reachable);
        assert_eq!(health.endpoint_id, "endpoint-private-1");
        assert_eq!(health.exposure, ExposureScope::Private);

        let exposure = tunnel.exposure_report();
        assert_eq!(exposure.permission_scope, "network:connect:private_tunnel");
        assert!(exposure.permission_required);
        assert_eq!(exposure.audit_event_kind, "connectivity.exposure_changed");

        let denied = tunnel
            .resolve_endpoint(
                EndpointOwner::capo_server("dashboard"),
                ChannelKind::Dashboard,
            )
            .expect_err("dashboard channel is not allowed on private runtime stub");
        assert_eq!(
            denied,
            ConnectivityError::ChannelNotAllowed {
                endpoint_id: "endpoint-private-1".to_string(),
                channel_kind: ChannelKind::Dashboard,
            }
        );
    }

    #[test]
    fn local_loopback_tunnel_resolves_only_loopback_safe_channels_without_remote_permission() {
        let tunnel = ConnectivityTunnel::local_loopback();
        let resolved = tunnel
            .resolve_endpoint(
                EndpointOwner::capo_server("local-api"),
                ChannelKind::Dashboard,
            )
            .expect("resolve local dashboard");

        assert_eq!(resolved.connectivity_endpoint_id, "local-loopback");
        assert_eq!(resolved.exposure, ExposureScope::Loopback);
        assert_eq!(resolved.permission_scope, "network:connect:localhost");
        assert!(!resolved.permission_required);
        assert!(resolved.resolved_uri.starts_with("http://127.0.0.1"));

        let health = tunnel.check_reachability();
        assert!(health.reachable);
        assert_eq!(health.exposure, ExposureScope::Loopback);

        assert!(matches!(
            tunnel.resolve_endpoint(EndpointOwner::runtime_target("target"), ChannelKind::Stdio),
            Err(ConnectivityError::ChannelNotAllowed { .. })
        ));
    }

    // ---- CT1: ExposurePolicy (loopback default, explicit opt-in, auth-required) ----

    #[test]
    fn ct1_exposure_policy_default_is_loopback_only_with_no_auth_or_opt_in() {
        let policy = ExposurePolicy::loopback_default();
        assert_eq!(policy.ceiling(), ExposureScope::Loopback);
        assert_eq!(policy.opt_in_source(), None);

        // Loopback authorizes with no auth and no opt-in; it never requires
        // permission.
        assert_eq!(policy.authorize(ExposureScope::Loopback, None), Ok(false));
        // Even with a handle present, loopback stays permission-free.
        assert_eq!(
            policy.authorize(ExposureScope::Loopback, Some("keychain:capo/x")),
            Ok(false)
        );
    }

    #[test]
    fn ct1_non_loopback_without_auth_ref_is_a_typed_auth_required_refusal() {
        let policy = ExposurePolicy::loopback_default();
        assert_eq!(
            policy.authorize(ExposureScope::Private, None),
            Err(ConnectivityError::AuthRequired {
                scope: ExposureScope::Private
            })
        );
        assert_eq!(
            policy.authorize(ExposureScope::Public, None),
            Err(ConnectivityError::AuthRequired {
                scope: ExposureScope::Public
            })
        );
        // An empty handle is treated as no handle (fail closed).
        assert_eq!(
            policy.authorize(ExposureScope::Private, Some("")),
            Err(ConnectivityError::AuthRequired {
                scope: ExposureScope::Private
            })
        );
    }

    #[test]
    fn ct1_non_loopback_above_ceiling_fails_closed_even_with_a_handle() {
        // A handle is present but the default ceiling is still Loopback: the
        // request must fail closed against the ceiling, never implicitly widen.
        let policy = ExposurePolicy::loopback_default();
        assert_eq!(
            policy.authorize(ExposureScope::Private, Some("keychain:capo/authkey")),
            Err(ConnectivityError::ScopeExceedsCeiling {
                requested: ExposureScope::Private,
                ceiling: ExposureScope::Loopback,
            })
        );
    }

    #[test]
    fn ct1_promoted_policy_permits_resolution_but_still_requires_permission() {
        let (policy, event) = ExposurePolicy::loopback_default().promote(
            ExposureScope::Private,
            "config",
            "unix:1700000000",
        );
        assert_eq!(policy.ceiling(), ExposureScope::Private);
        assert_eq!(policy.opt_in_source(), Some("config"));

        // Promotion emitted a replay-stable policy_changed event with old/new
        // ceiling and the opt-in source and NO secret.
        let event = event.expect("promotion emits a policy_changed event");
        assert_eq!(event.previous_ceiling, ExposureScope::Loopback);
        assert_eq!(event.new_ceiling, ExposureScope::Private);
        assert_eq!(event.opt_in_source, "config");
        let payload = event.payload_json();
        assert_eq!(
            payload,
            "{\"previous_ceiling\":\"loopback\",\"new_ceiling\":\"private\",\"opt_in_source\":\"config\",\"changed_at\":\"unix:1700000000\"}"
        );
        // Replay-stable: same inputs, identical bytes.
        let (_again, replay) = ExposurePolicy::loopback_default().promote(
            ExposureScope::Private,
            "config",
            "unix:1700000000",
        );
        assert_eq!(replay.expect("event").payload_json(), payload);

        // With opt-in + handle, private is permitted BUT still permission_required
        // (the grant engine independently gates activation).
        assert_eq!(
            policy.authorize(ExposureScope::Private, Some("keychain:capo/authkey")),
            Ok(true)
        );
    }

    #[test]
    fn ct1_promotion_to_same_or_lower_ceiling_is_not_a_widening_and_emits_no_event() {
        let (promoted, _) =
            ExposurePolicy::loopback_default().promote(ExposureScope::Public, "grant", "unix:1");
        // Re-promoting to a lower ceiling does not widen and emits no audit event.
        let (still, event) = promoted.promote(ExposureScope::Private, "config", "unix:2");
        assert_eq!(still.ceiling(), ExposureScope::Public);
        assert!(event.is_none());

        // Loopback default -> Loopback is not a promotion.
        let (_same, event) =
            ExposurePolicy::loopback_default().promote(ExposureScope::Loopback, "config", "unix:3");
        assert!(event.is_none());
    }

    #[test]
    fn ct1_authorize_socket_is_symmetric_and_fails_closed_for_non_loopback_by_default() {
        let policy = ExposurePolicy::loopback_default();
        // Loopback socket passes with no handle (the zero-config default).
        assert_eq!(policy.authorize_socket(true, None), Ok(()));
        // Non-loopback fails closed (no handle) under the default policy.
        assert_eq!(
            policy.authorize_socket(false, None),
            Err(ConnectivityError::AuthRequired {
                scope: ExposureScope::Private
            })
        );
        // Non-loopback with a handle but unpromoted ceiling still fails closed.
        assert_eq!(
            policy.authorize_socket(false, Some("keychain:capo/authkey")),
            Err(ConnectivityError::ScopeExceedsCeiling {
                requested: ExposureScope::Private,
                ceiling: ExposureScope::Loopback,
            })
        );
        // Promoted + handle: non-loopback socket is authorized.
        let (promoted, _) = policy.promote(ExposureScope::Private, "flag", "unix:1700000001");
        assert_eq!(
            promoted.authorize_socket(false, Some("keychain:capo/authkey")),
            Ok(())
        );
    }

    #[test]
    fn local_process_runner_captures_redacted_output_and_lifecycle_controls() {
        let workspace = temp_root("workspace");
        let artifacts = temp_root("artifacts");
        fs::create_dir_all(&workspace).unwrap();
        let mut config = LocalProcessConfig::for_test(workspace.clone(), artifacts.clone());
        config.redaction_rules.push(RedactionRule {
            pattern: "SECRET".to_string(),
            replacement: "[REDACTED]".to_string(),
        });
        let runner = LocalProcessRunner::new(config);

        let outcome = runner
            .start_process(LocalProcessRequest {
                run_id: RunId::new("run-local"),
                turn_id: None,
                program: "/bin/sh".to_string(),
                argv: vec![
                    "-c".to_string(),
                    "printf stdout-SECRET; printf stderr-SECRET >&2".to_string(),
                ],
                cwd: workspace,
                env: HashMap::new(),
            })
            .expect("run local process");

        assert_eq!(outcome.process.status, "exited");
        assert_eq!(outcome.stdout.redaction_state, "redacted");
        assert_eq!(outcome.stderr.redaction_state, "redacted");
        assert_eq!(
            fs::read_to_string(&outcome.stdout.path).unwrap(),
            "stdout-[REDACTED]"
        );
        assert_eq!(
            fs::read_to_string(&outcome.stderr.path).unwrap(),
            "stderr-[REDACTED]"
        );
        assert!(outcome.stdout.content_hash.starts_with("fnv1a64:"));
        assert!(
            outcome
                .events
                .iter()
                .any(|event| event.kind == "runtime.output_artifact_recorded")
        );

        let interrupted = runner.interrupt(&outcome.process, "test interrupt");
        assert_eq!(interrupted.process.status, "interrupting");
        assert_eq!(interrupted.events[0].kind, "runtime.interrupt_requested");
        let terminated = runner.terminate(&outcome.process, "test terminate");
        assert_eq!(terminated.process.status, "terminating");
        let killed = runner.kill(&outcome.process);
        assert_eq!(killed.process.status, "killed");
        assert!(!runner.health(&outcome.process).live);
        assert_eq!(
            runner.recover_orphan(&outcome.process).recovered_status,
            "orphaned"
        );
        runner.cleanup(&outcome.process).expect("cleanup");
        assert!(artifacts.join("run-local").exists());
        assert!(artifacts.join("run-local").join("cleanup.marker").exists());
    }

    #[test]
    fn local_process_runner_credential_scan_redacts_unnamed_secret_in_output() {
        // ACI7 regression: the credential scan must run through the REAL runner
        // boundary (`start_process` -> `redact_output`), not only at the unit
        // level. Here the process prints an UNNAMED secret (no operator rule),
        // and the artifact on disk must be scrubbed with redaction_state=redacted.
        let workspace = temp_root("workspace-credscan");
        let artifacts = temp_root("artifacts-credscan");
        fs::create_dir_all(&workspace).unwrap();
        // No redaction_rules configured: this proves the default credential scan.
        let runner = LocalProcessRunner::new(LocalProcessConfig::for_test(
            workspace.clone(),
            artifacts.clone(),
        ));

        let outcome = runner
            .start_process(LocalProcessRequest {
                run_id: RunId::new("run-credscan"),
                turn_id: None,
                program: "/bin/sh".to_string(),
                argv: vec![
                    "-c".to_string(),
                    "printf 'key AKIAIOSFODNN7EXAMPLE'; \
                     printf 'tok ghp_abcdEFGH1234ijklMNOP5678qrst' >&2"
                        .to_string(),
                ],
                cwd: workspace,
                env: HashMap::new(),
            })
            .expect("run local process");

        assert_eq!(outcome.process.status, "exited");
        assert_eq!(outcome.stdout.redaction_state, "redacted");
        assert_eq!(outcome.stderr.redaction_state, "redacted");
        let stdout = fs::read_to_string(&outcome.stdout.path).unwrap();
        let stderr = fs::read_to_string(&outcome.stderr.path).unwrap();
        assert!(
            !stdout.contains("AKIAIOSFODNN7EXAMPLE"),
            "unnamed secret leaked to stdout artifact: {stdout}"
        );
        assert!(
            !stderr.contains("ghp_abcdEFGH1234ijklMNOP5678qrst"),
            "unnamed secret leaked to stderr artifact: {stderr}"
        );
        assert!(stdout.contains(CREDENTIAL_REDACTION_PLACEHOLDER));
        // The benign words around the secret survive.
        assert!(stdout.contains("key"));

        runner.cleanup(&outcome.process).expect("cleanup");
    }

    #[test]
    fn local_process_runner_credential_scan_keeps_benign_git_output_intact() {
        // ACI7 regression / false-positive guard at the runner boundary: a
        // benign command emitting a git SHA and a filesystem path must NOT be
        // corrupted by the credential scan (redaction_state stays "safe").
        let workspace = temp_root("workspace-benign");
        let artifacts = temp_root("artifacts-benign");
        fs::create_dir_all(&workspace).unwrap();
        let runner = LocalProcessRunner::new(LocalProcessConfig::for_test(
            workspace.clone(),
            artifacts.clone(),
        ));

        let outcome = runner
            .start_process(LocalProcessRequest {
                run_id: RunId::new("run-benign"),
                turn_id: None,
                program: "/bin/sh".to_string(),
                argv: vec![
                    "-c".to_string(),
                    "printf 'commit 9fceb02d0ae598e95dc970b74767f19372d61af8 \
                     /usr/local/lib/python3.11/site-packages/numpy'"
                        .to_string(),
                ],
                cwd: workspace,
                env: HashMap::new(),
            })
            .expect("run local process");

        assert_eq!(outcome.process.status, "exited");
        assert_eq!(
            outcome.stdout.redaction_state, "safe",
            "benign git output wrongly marked redacted"
        );
        let stdout = fs::read_to_string(&outcome.stdout.path).unwrap();
        assert!(
            stdout.contains("9fceb02d0ae598e95dc970b74767f19372d61af8"),
            "git SHA was corrupted by the credential scan: {stdout}"
        );
        assert!(
            stdout.contains("/usr/local/lib/python3.11/site-packages/numpy"),
            "path was corrupted by the credential scan: {stdout}"
        );

        runner.cleanup(&outcome.process).expect("cleanup");
    }

    #[test]
    fn local_process_runner_can_kill_a_live_child_and_collect_artifacts() {
        let workspace = temp_root("workspace-live");
        let artifacts = temp_root("artifacts-live");
        fs::create_dir_all(&workspace).unwrap();
        let runner =
            LocalProcessRunner::new(LocalProcessConfig::for_test(workspace.clone(), artifacts));

        let mut running = runner
            .spawn_process(LocalProcessRequest {
                run_id: RunId::new("run-live"),
                turn_id: None,
                program: "/bin/sh".to_string(),
                argv: vec![
                    "-c".to_string(),
                    "printf before-sleep; sleep 5; printf after-sleep".to_string(),
                ],
                cwd: workspace,
                env: HashMap::new(),
            })
            .expect("spawn local process");

        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(runner.health_running(&mut running).unwrap().live);
        let killed = runner.kill_running(&mut running).unwrap();
        assert_eq!(killed.process.status, "killed");
        let outcome = runner.wait_running(&mut running).unwrap();
        assert_eq!(outcome.process.status, "killed");
        assert!(
            fs::read_to_string(&outcome.stdout.path)
                .unwrap()
                .contains("before-sleep")
        );
    }

    #[test]
    fn multiple_turns_in_one_run_keep_distinct_per_turn_artifacts() {
        let workspace = temp_root("workspace-multi-turn");
        let artifacts = temp_root("artifacts-multi-turn");
        fs::create_dir_all(&workspace).unwrap();
        let runner = LocalProcessRunner::new(LocalProcessConfig::for_test(
            workspace.clone(),
            artifacts.clone(),
        ));

        let run_id = RunId::new("run-multi-turn");
        let mut outcomes = Vec::new();
        for turn in ["turn-1", "turn-2"] {
            let mut running = runner
                .spawn_process(
                    LocalProcessRequest::new(
                        run_id.clone(),
                        "/bin/sh",
                        vec![
                            "-c".to_string(),
                            format!("printf stdout-{turn}; printf stderr-{turn} >&2"),
                        ],
                        workspace.clone(),
                        HashMap::new(),
                    )
                    .with_turn_id(turn),
                )
                .expect("spawn turn process");
            let outcome = runner
                .wait_running(&mut running)
                .expect("wait turn process");
            outcomes.push((turn, outcome));
        }

        // Each turn keeps a distinct artifact directory, distinct artifact ids,
        // and its own stdout/stderr content -- no overwriting across turns.
        let (turn1, outcome1) = &outcomes[0];
        let (turn2, outcome2) = &outcomes[1];
        assert_ne!(outcome1.stdout.path, outcome2.stdout.path);
        assert_ne!(outcome1.stderr.path, outcome2.stderr.path);
        assert_ne!(outcome1.stdout.artifact_id, outcome2.stdout.artifact_id);
        assert_ne!(outcome1.stderr.artifact_id, outcome2.stderr.artifact_id);
        assert!(
            outcome1
                .stdout
                .artifact_id
                .contains(&format!("turn-{turn1}"))
        );
        assert!(
            outcome2
                .stdout
                .artifact_id
                .contains(&format!("turn-{turn2}"))
        );

        for (turn, outcome) in &outcomes {
            assert_eq!(
                fs::read_to_string(&outcome.stdout.path).unwrap(),
                format!("stdout-{turn}")
            );
            assert_eq!(
                fs::read_to_string(&outcome.stderr.path).unwrap(),
                format!("stderr-{turn}")
            );
            // Per-turn artifacts are nested under run_id/turns/<turn_id>.
            let expected_dir = artifacts.join(run_id.as_str()).join("turns").join(turn);
            assert_eq!(outcome.stdout.path.parent().unwrap(), expected_dir);
        }

        // Every turn's artifact is reconstructable from the run directory alone
        // (the replay/rebuild surface): the on-disk layout enumerates each turn.
        let turns_dir = artifacts.join(run_id.as_str()).join("turns");
        let mut recorded_turns: Vec<String> = fs::read_dir(&turns_dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        recorded_turns.sort();
        assert_eq!(recorded_turns, vec!["turn-1", "turn-2"]);
        for turn in &recorded_turns {
            let turn_dir = turns_dir.join(turn);
            assert!(turn_dir.join("stdout.txt").exists());
            assert!(turn_dir.join("stderr.txt").exists());
        }
    }

    #[test]
    fn run_without_a_turn_id_keeps_the_legacy_single_turn_artifact_layout() {
        let workspace = temp_root("workspace-legacy-layout");
        let artifacts = temp_root("artifacts-legacy-layout");
        fs::create_dir_all(&workspace).unwrap();
        let runner = LocalProcessRunner::new(LocalProcessConfig::for_test(
            workspace.clone(),
            artifacts.clone(),
        ));

        let mut running = runner
            .spawn_process(LocalProcessRequest::new(
                RunId::new("run-legacy"),
                "/bin/sh",
                vec!["-c".to_string(), "printf legacy-out".to_string()],
                workspace.clone(),
                HashMap::new(),
            ))
            .expect("spawn legacy process");
        let outcome = runner
            .wait_running(&mut running)
            .expect("wait legacy process");

        // No turn key -> legacy run_dir = artifact_root/run_id and legacy id shape.
        assert_eq!(
            outcome.stdout.path,
            artifacts.join("run-legacy").join("stdout.txt")
        );
        assert_eq!(
            outcome.stdout.artifact_id,
            "artifact-runtime-run-legacy-stdout"
        );
        assert_eq!(
            outcome.stderr.artifact_id,
            "artifact-runtime-run-legacy-stderr"
        );
        assert!(!artifacts.join("run-legacy").join("turns").exists());
    }

    #[test]
    fn local_process_runner_times_out_and_collects_partial_artifacts() {
        let workspace = temp_root("workspace-timeout");
        let artifacts = temp_root("artifacts-timeout");
        fs::create_dir_all(&workspace).unwrap();
        let runner =
            LocalProcessRunner::new(LocalProcessConfig::for_test(workspace.clone(), artifacts));

        let mut running = runner
            .spawn_process(LocalProcessRequest {
                run_id: RunId::new("run-timeout"),
                turn_id: None,
                program: "/bin/sh".to_string(),
                argv: vec![
                    "-c".to_string(),
                    "printf before-timeout; sleep 5; printf after-timeout".to_string(),
                ],
                cwd: workspace,
                env: HashMap::new(),
            })
            .expect("spawn local process");

        let outcome = runner
            .wait_running_with_timeout(&mut running, std::time::Duration::from_millis(100))
            .unwrap();

        assert_eq!(outcome.process.status, "timed_out");
        let stdout = fs::read_to_string(&outcome.stdout.path).unwrap();
        assert!(stdout.contains("before-timeout"));
        assert!(!stdout.contains("after-timeout"));
    }

    #[test]
    fn local_process_timeout_terminates_descendant_processes() {
        let workspace = temp_root("workspace-timeout-tree");
        let artifacts = temp_root("artifacts-timeout-tree");
        fs::create_dir_all(&workspace).unwrap();
        let marker = workspace.join("descendant-survived.txt");
        let runner =
            LocalProcessRunner::new(LocalProcessConfig::for_test(workspace.clone(), artifacts));

        let mut running = runner
            .spawn_process(LocalProcessRequest {
                run_id: RunId::new("run-timeout-tree"),
                turn_id: None,
                program: "/bin/sh".to_string(),
                argv: vec![
                    "-c".to_string(),
                    format!("(sleep 1; printf survived > {}) & wait", marker.display()),
                ],
                cwd: workspace,
                env: HashMap::new(),
            })
            .expect("spawn local process tree");

        let outcome = runner
            .wait_running_with_timeout(&mut running, std::time::Duration::from_millis(100))
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1200));

        assert_eq!(outcome.process.status, "timed_out");
        assert!(!marker.exists());
    }

    #[test]
    fn local_process_runner_removes_raw_files_when_output_exceeds_limit() {
        let workspace = temp_root("workspace-output-limit");
        let artifacts = temp_root("artifacts-output-limit");
        fs::create_dir_all(&workspace).unwrap();
        let mut config = LocalProcessConfig::for_test(workspace.clone(), artifacts.clone());
        config.output_limit_bytes = 8;
        let runner = LocalProcessRunner::new(config);

        let mut running = runner
            .spawn_process(LocalProcessRequest {
                run_id: RunId::new("run-output-limit"),
                turn_id: None,
                program: "/bin/sh".to_string(),
                argv: vec![
                    "-c".to_string(),
                    "printf this-output-is-too-large".to_string(),
                ],
                cwd: workspace,
                env: HashMap::new(),
            })
            .expect("spawn local process");

        let error = runner.wait_running(&mut running).unwrap_err();

        assert!(matches!(error, RuntimeError::OutputLimitExceeded { .. }));
        assert!(!artifacts.join("run-output-limit/stdout.txt").exists());
        assert!(!artifacts.join("run-output-limit/stderr.txt").exists());
    }

    #[test]
    fn loopback_remote_runtime_contract_keeps_runtime_refs_and_control_events() {
        let workspace = temp_root("workspace-remote");
        let artifacts = temp_root("artifacts-remote");
        fs::create_dir_all(&workspace).unwrap();
        let runner = RemoteProcessRunner::new(RemoteProcessConfig::loopback_for_test(
            "remote-target-1",
            "endpoint-loopback-1",
            workspace.clone(),
            artifacts.clone(),
        ));

        assert_eq!(runner.binding().variant, "remote-process");
        let outcome = runner
            .start_process(LocalProcessRequest {
                run_id: RunId::new("run-remote"),
                turn_id: None,
                program: "/bin/sh".to_string(),
                argv: vec!["-c".to_string(), "printf remote-ok".to_string()],
                cwd: workspace,
                env: HashMap::new(),
            })
            .expect("run remote loopback process");

        assert_eq!(outcome.process.status, "exited");
        assert!(
            outcome
                .process
                .runtime_process_ref
                .starts_with("remote-process:remote-target-1:endpoint-loopback-1:")
        );
        assert!(
            outcome
                .events
                .iter()
                .any(|event| event.kind == "runtime.remote_target_resolved")
        );
        assert!(
            outcome
                .events
                .iter()
                .any(|event| event.kind == "runtime.remote_process_started")
        );
        assert!(
            outcome
                .events
                .iter()
                .any(|event| event.kind == "runtime.process_exited")
        );
        assert_eq!(
            fs::read_to_string(&outcome.stdout.path).unwrap(),
            "remote-ok"
        );

        let running_ref = LocalRuntimeProcessRef {
            status: "running".to_string(),
            ..outcome.process.clone()
        };
        let health = runner.health(&running_ref);
        assert!(health.live);
        assert_eq!(health.status, "remote:remote-target-1:running");

        let interrupted = runner.interrupt(&running_ref, "operator interrupt");
        assert_eq!(interrupted.process.status, "interrupting");
        assert_eq!(interrupted.events[0].kind, "runtime.remote_interrupt_sent");
        assert!(interrupted.events[0].detail.contains("endpoint-loopback-1"));

        let terminated = runner.terminate(&running_ref, "operator terminate");
        assert_eq!(terminated.process.status, "terminating");
        assert_eq!(terminated.events[0].kind, "runtime.remote_terminate_sent");

        let recovered = runner.recover_orphan(&running_ref);
        assert_eq!(recovered.recovered_status, "remote_recovered");
        assert!(recovered.detail.contains("remote-target-1"));

        let exited_recovery = runner.recover_orphan(&outcome.process);
        assert_eq!(exited_recovery.recovered_status, "remote_orphaned");
        assert!(artifacts.join("run-remote").exists());
    }

    #[test]
    fn local_process_runner_rejects_non_allowlisted_env_overrides() {
        let workspace = temp_root("workspace-env");
        fs::create_dir_all(&workspace).unwrap();
        let runner = LocalProcessRunner::new(LocalProcessConfig::for_test(
            workspace.clone(),
            temp_root("artifacts-env"),
        ));

        let error = runner
            .start_process(LocalProcessRequest {
                run_id: RunId::new("run-env"),
                turn_id: None,
                program: "/usr/bin/env".to_string(),
                argv: Vec::new(),
                cwd: workspace,
                env: HashMap::from([("SECRET_TOKEN".to_string(), "secret".to_string())]),
            })
            .unwrap_err();

        assert!(matches!(
            error,
            RuntimeError::DisallowedEnvOverride(name) if name == "SECRET_TOKEN"
        ));
    }

    #[test]
    fn local_process_runner_rejects_cwd_outside_workspace() {
        let workspace = temp_root("workspace-allowed");
        let outside = temp_root("workspace-outside");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(&outside).unwrap();
        let runner = LocalProcessRunner::new(LocalProcessConfig::for_test(
            workspace,
            temp_root("artifacts"),
        ));

        let error = runner
            .start_process(LocalProcessRequest {
                run_id: RunId::new("run-reject"),
                turn_id: None,
                program: "/bin/echo".to_string(),
                argv: vec!["nope".to_string()],
                cwd: outside,
                env: HashMap::new(),
            })
            .unwrap_err();

        assert!(matches!(error, RuntimeError::CwdOutsideWorkspace { .. }));
    }

    #[cfg(unix)]
    #[test]
    fn reap_orphan_process_group_kills_a_live_descendant_tree_by_pid() {
        // RTL10: simulate a controller crash mid-run. The runtime spawned a
        // process group with a backgrounded descendant that would survive its
        // parent; on restart Capo no longer holds the `Child`, only the
        // persisted PID. Reaping by that PID must terminate the whole group --
        // so the descendant's delayed marker never appears.
        let workspace = temp_root("workspace-reap-tree");
        let artifacts = temp_root("artifacts-reap-tree");
        fs::create_dir_all(&workspace).unwrap();
        let marker = workspace.join("orphan-survived.txt");
        let runner =
            LocalProcessRunner::new(LocalProcessConfig::for_test(workspace.clone(), artifacts));

        let mut running = runner
            .spawn_process(LocalProcessRequest {
                run_id: RunId::new("run-reap-tree"),
                turn_id: None,
                program: "/bin/sh".to_string(),
                argv: vec![
                    "-c".to_string(),
                    // A backgrounded descendant writes the marker after a delay;
                    // the parent exits immediately, leaving the descendant as
                    // the orphan we must reap by the persisted group PID.
                    format!("(sleep 2; printf survived > {}) &", marker.display()),
                ],
                cwd: workspace,
                env: HashMap::new(),
            })
            .expect("spawn orphan tree");
        let pid = running.process.external_pid.expect("pid recorded");
        let recorded_boot_id = running.process.boot_id.clone();
        // Let the parent exit but the descendant keep sleeping.
        let _ = running.child.wait();
        thread::sleep(Duration::from_millis(100));

        let reap = LocalProcessRunner::reap_orphan_process_group(pid, recorded_boot_id.as_deref());
        assert!(reap.reaped, "a live orphan group must be reaped");
        assert_eq!(reap.observed_state, "alive_reaped");
        assert_eq!(reap.external_pid, pid);

        // Give the descendant well past its delay; if reaping worked it never
        // wrote the marker.
        thread::sleep(Duration::from_millis(2200));
        assert!(
            !marker.exists(),
            "reaping the process group must kill the descendant before it writes"
        );
    }

    #[cfg(unix)]
    #[test]
    fn reap_orphan_process_group_reports_already_gone_for_a_dead_pid() {
        // A process that has already exited has no group to reap; the reaper
        // reports `already_gone` so recovery records a terminal exit, and the
        // observed-state hash is stable for idempotency.
        let workspace = temp_root("workspace-reap-gone");
        let artifacts = temp_root("artifacts-reap-gone");
        fs::create_dir_all(&workspace).unwrap();
        let runner =
            LocalProcessRunner::new(LocalProcessConfig::for_test(workspace.clone(), artifacts));
        let mut running = runner
            .spawn_process(LocalProcessRequest {
                run_id: RunId::new("run-reap-gone"),
                turn_id: None,
                program: "/bin/sh".to_string(),
                argv: vec!["-c".to_string(), "exit 0".to_string()],
                cwd: workspace,
                env: HashMap::new(),
            })
            .expect("spawn short process");
        let pid = running.process.external_pid.expect("pid recorded");
        let recorded_boot_id = running.process.boot_id.clone();
        let _ = running.child.wait();
        thread::sleep(Duration::from_millis(100));

        let reap = LocalProcessRunner::reap_orphan_process_group(pid, recorded_boot_id.as_deref());
        assert!(!reap.reaped);
        assert_eq!(reap.observed_state, "already_gone");
        // Stable hash: re-observing the same gone PID hashes identically.
        assert_eq!(
            reap.observed_runtime_state_hash,
            LocalProcessRunner::reap_orphan_process_group(pid, recorded_boot_id.as_deref())
                .observed_runtime_state_hash
        );
    }

    #[cfg(unix)]
    #[test]
    fn reap_orphan_process_group_does_not_reap_across_a_reboot_boundary() {
        // RTL10 safety: PIDs/PGIDs are recycled across reboots, so a live group
        // observed under a *different* boot id than the one recorded at spawn
        // must NOT be signalled -- it is almost certainly an unrelated process
        // group. The reaper records it as `already_gone` (no kill).
        let workspace = temp_root("workspace-reap-reboot");
        let artifacts = temp_root("artifacts-reap-reboot");
        fs::create_dir_all(&workspace).unwrap();
        let marker = workspace.join("survivor-across-reboot.txt");
        let runner =
            LocalProcessRunner::new(LocalProcessConfig::for_test(workspace.clone(), artifacts));
        let mut running = runner
            .spawn_process(LocalProcessRequest {
                run_id: RunId::new("run-reap-reboot"),
                turn_id: None,
                program: "/bin/sh".to_string(),
                argv: vec![
                    "-c".to_string(),
                    format!("(sleep 2; printf survived > {}) &", marker.display()),
                ],
                cwd: workspace,
                env: HashMap::new(),
            })
            .expect("spawn process group");
        let pid = running.process.external_pid.expect("pid recorded");
        let _ = running.child.wait();
        thread::sleep(Duration::from_millis(100));

        // Recorded boot id from a *different* boot than the current one.
        let reap = LocalProcessRunner::reap_orphan_process_group(
            pid,
            Some("linux-btime-000000000-stale-reboot"),
        );
        assert!(
            !reap.reaped,
            "a recycled PID after a reboot must not be reaped"
        );
        assert_eq!(reap.observed_state, "already_gone");

        // The live descendant is left alone and writes its marker.
        thread::sleep(Duration::from_millis(2200));
        assert!(
            marker.exists(),
            "the group under a different boot id must be left untouched"
        );
    }

    #[cfg(unix)]
    #[test]
    fn reap_orphan_process_group_never_signals_self_or_init_groups() {
        // RTL10 safety: a corrupted/zero/low PID in the durable marker must never
        // become `kill -<0|1>` (self group / init). Both report `already_gone`
        // with no signal, regardless of the recorded boot id.
        let current = boot_id();
        for pid in [0u32, 1u32] {
            let reap = LocalProcessRunner::reap_orphan_process_group(pid, current.as_deref());
            assert!(!reap.reaped, "pid {pid} must never be reaped");
            assert_eq!(reap.observed_state, "already_gone");
        }
        assert!(!is_reapable_pid(0));
        assert!(!is_reapable_pid(1));
        assert!(is_reapable_pid(2));
    }

    #[cfg(unix)]
    #[test]
    fn probe_run_health_reports_alive_without_killing_the_process() {
        // SG9: the liveness-aware probe must OBSERVE a live in-flight run's
        // process group WITHOUT terminating it (unlike the RTL10 reaper), so the
        // recovery layer can reattach in place. The backgrounded descendant must
        // survive the probe and write its marker.
        let workspace = temp_root("workspace-probe-alive");
        let artifacts = temp_root("artifacts-probe-alive");
        fs::create_dir_all(&workspace).unwrap();
        let marker = workspace.join("probe-survivor.txt");
        let runner =
            LocalProcessRunner::new(LocalProcessConfig::for_test(workspace.clone(), artifacts));
        let mut running = runner
            .spawn_process(LocalProcessRequest {
                run_id: RunId::new("run-probe-alive"),
                turn_id: None,
                program: "/bin/sh".to_string(),
                argv: vec![
                    "-c".to_string(),
                    format!("(sleep 2; printf survived > {}) &", marker.display()),
                ],
                cwd: workspace,
                env: HashMap::new(),
            })
            .expect("spawn live group");
        let pid = running.process.external_pid.expect("pid recorded");
        let recorded_boot_id = running.process.boot_id.clone();
        let _ = running.child.wait();
        thread::sleep(Duration::from_millis(100));

        let probe = LocalProcessRunner::probe_run_health(pid, recorded_boot_id.as_deref());
        assert_eq!(probe.state, RuntimeHealthState::Alive);
        assert!(probe.state.is_alive());
        assert_eq!(probe.external_pid, Some(pid));

        // The probe never signalled, so the descendant survives and writes.
        thread::sleep(Duration::from_millis(2200));
        assert!(
            marker.exists(),
            "a non-destructive liveness probe must leave the live group running"
        );
    }

    #[cfg(unix)]
    #[test]
    fn probe_run_health_reports_exited_for_a_dead_pid_and_is_stable() {
        // SG9: a gone process group classifies as exited, with a stable
        // observed-state hash so repeated restart probes are idempotent.
        let workspace = temp_root("workspace-probe-gone");
        let artifacts = temp_root("artifacts-probe-gone");
        fs::create_dir_all(&workspace).unwrap();
        let runner =
            LocalProcessRunner::new(LocalProcessConfig::for_test(workspace.clone(), artifacts));
        let mut running = runner
            .spawn_process(LocalProcessRequest {
                run_id: RunId::new("run-probe-gone"),
                turn_id: None,
                program: "/bin/sh".to_string(),
                argv: vec!["-c".to_string(), "exit 0".to_string()],
                cwd: workspace,
                env: HashMap::new(),
            })
            .expect("spawn short process");
        let pid = running.process.external_pid.expect("pid recorded");
        let recorded_boot_id = running.process.boot_id.clone();
        let _ = running.child.wait();
        thread::sleep(Duration::from_millis(100));

        let probe = LocalProcessRunner::probe_run_health(pid, recorded_boot_id.as_deref());
        assert_eq!(probe.state, RuntimeHealthState::Exited);
        assert_eq!(
            probe.observed_state_hash,
            LocalProcessRunner::probe_run_health(pid, recorded_boot_id.as_deref())
                .observed_state_hash,
            "re-probing the same gone PID must hash identically (idempotency)"
        );
    }

    #[cfg(unix)]
    #[test]
    fn probe_run_health_treats_a_recycled_pid_across_reboot_as_exited() {
        // SG9: a PID observed under a DIFFERENT boot id than was recorded at spawn
        // is a recycled/unrelated group and must never be trusted as "our run
        // still alive" -- it classifies as exited (like the reaper declines to
        // signal it), so recovery never reattaches to an unrelated process.
        let workspace = temp_root("workspace-probe-reboot");
        let artifacts = temp_root("artifacts-probe-reboot");
        fs::create_dir_all(&workspace).unwrap();
        let marker = workspace.join("probe-reboot-survivor.txt");
        let runner =
            LocalProcessRunner::new(LocalProcessConfig::for_test(workspace.clone(), artifacts));
        let mut running = runner
            .spawn_process(LocalProcessRequest {
                run_id: RunId::new("run-probe-reboot"),
                turn_id: None,
                program: "/bin/sh".to_string(),
                argv: vec![
                    "-c".to_string(),
                    format!("(sleep 2; printf survived > {}) &", marker.display()),
                ],
                cwd: workspace,
                env: HashMap::new(),
            })
            .expect("spawn live group");
        let pid = running.process.external_pid.expect("pid recorded");
        let _ = running.child.wait();
        thread::sleep(Duration::from_millis(100));

        let probe =
            LocalProcessRunner::probe_run_health(pid, Some("linux-btime-000000000-stale-reboot"));
        assert_eq!(
            probe.state,
            RuntimeHealthState::Exited,
            "a PID under a different boot id must classify as exited (no reattach)"
        );

        // And it was never signalled: the live descendant is left running.
        thread::sleep(Duration::from_millis(2200));
        assert!(
            marker.exists(),
            "the probe must not signal a group under a different boot id"
        );
    }

    fn temp_root(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!("capo-runtime-{name}-{nanos}"))
    }

    #[test]
    fn redaction_policy_applies_rules_then_credential_scan() {
        // ACI7: the explicit operator pattern wins its exact replacement, and the
        // default credential-shape scan scrubs an UNNAMED credential too.
        let policy = RedactionPolicy::new(vec![RedactionRule {
            pattern: "NAMED".to_string(),
            replacement: "[X]".to_string(),
        }]);
        let (bytes, state) = policy.apply(b"token NAMED and key AKIAIOSFODNN7EXAMPLE done");
        let text = String::from_utf8(bytes).unwrap();
        assert_eq!(state, "redacted");
        assert!(text.contains("[X]"), "named rule should apply: {text}");
        assert!(
            !text.contains("AKIAIOSFODNN7EXAMPLE"),
            "credential scan should scrub the unnamed key: {text}"
        );
        assert!(text.contains(CREDENTIAL_REDACTION_PLACEHOLDER));
        // The benign words survive.
        assert!(text.contains("token") && text.contains("done"));
    }

    #[test]
    fn credential_scan_recognizes_credential_shapes() {
        let policy = RedactionPolicy::new(Vec::new());
        for secret in [
            "AKIAIOSFODNN7EXAMPLE",
            "sk-abcdEFGH1234ijklMNOP5678",
            "ghp_abcdEFGH1234ijklMNOP5678qrst",
            "AIzaSyA1b2C3d4E5f6G7h8I9j0KLmnopQRstuv",
            "dGhpcyBpcyBhIGxvbmcgYmFzZTY0IDEyMzQ1Njc4OTA=",
        ] {
            let (bytes, state) = policy.apply(format!("value={secret}").as_bytes());
            let text = String::from_utf8(bytes).unwrap();
            assert_eq!(state, "redacted", "expected redaction for {secret}");
            assert!(
                !text.contains(secret),
                "credential {secret} should be scrubbed: {text}"
            );
        }
    }

    #[test]
    fn credential_scan_recognizes_bare_tokens_without_a_key_wrapper() {
        // ACI7 regression: the scan must fire on a BARE token (no `value=`
        // wrapper), including a base64 token whose only `=` is trailing padding.
        // Previously the unconditional `key=` strip turned such a token into an
        // empty value and the function early-returned `false`, leaking it.
        let policy = RedactionPolicy::new(Vec::new());
        for secret in [
            "AKIAIOSFODNN7EXAMPLE",
            "ghp_abcdEFGH1234ijklMNOP5678qrst",
            "Zm9vYmFyMTIzNDU2Nzg5MGFiY2RlZmdoaQ=",
            "dGhpcyBpcyBhIGxvbmcgYmFzZTY0IDEyMzQ1Njc4OTA=",
            "QUJDZGVmMTIzNDU2Nzg5MGdoaWprbG1ub3A==",
        ] {
            let (bytes, state) = policy.apply(format!("leaked {secret} here").as_bytes());
            let text = String::from_utf8(bytes).unwrap();
            assert_eq!(state, "redacted", "expected redaction for bare {secret}");
            assert!(
                !text.contains(secret),
                "bare credential {secret} should be scrubbed: {text}"
            );
        }
    }

    #[test]
    fn credential_scan_redacts_known_prefix_tokens_containing_an_equals() {
        // ACI7 regression: a known-prefix token with an embedded `=` must still
        // be redacted. Previously the `key=` strip removed the `github_pat_` /
        // `AKIA`-bearing prefix before the KNOWN_PREFIXES check ran.
        let policy = RedactionPolicy::new(Vec::new());
        for secret in [
            "github_pat_11ABCDEF=DEF456ghi789jkl",
            "AKIA1234567890ABCDEF=",
            "ghp_abcdEFGH1234ijklMNOP=5678qrst",
        ] {
            let (bytes, state) = policy.apply(format!("token {secret} end").as_bytes());
            let text = String::from_utf8(bytes).unwrap();
            assert_eq!(
                state, "redacted",
                "expected redaction for prefix-with-= {secret}"
            );
            assert!(
                !text.contains(secret),
                "known-prefix credential {secret} should be scrubbed: {text}"
            );
        }
    }

    #[test]
    fn credential_scan_redacts_quoted_json_and_url_embedded_secrets() {
        // ACI7 regression: secrets do not arrive bare in real OUTPUT. They are
        // quoted, packed into JSON, or sit in a URL query. Each is exactly a
        // read-file / shell-stdout / diff shape the policy claims to scrub.
        let policy = RedactionPolicy::new(Vec::new());
        let secret = "AKIAIOSFODNN7EXAMPLE";
        for line in [
            format!("{{\"aws_key\":\"{secret}\"}}"),
            format!("token=\"{secret}\""),
            format!("https://example.com/path?token={secret}&page=1"),
            format!("export AWS_SECRET='{secret}'"),
            format!("Authorization: Bearer {secret}"),
        ] {
            let (bytes, state) = policy.apply(line.as_bytes());
            let text = String::from_utf8(bytes).unwrap();
            assert_eq!(state, "redacted", "expected redaction for: {line}");
            assert!(
                !text.contains(secret),
                "embedded credential should be scrubbed in {line}, got: {text}"
            );
        }
    }

    #[test]
    fn credential_scan_does_not_corrupt_shas_uuids_and_paths() {
        // ACI7 regression: the high-volume false-positive surface. git SHAs, hex
        // digests, dashed UUIDs, and long filesystem paths fill git/test output
        // and must NOT be replaced with the credential placeholder.
        let policy = RedactionPolicy::new(Vec::new());
        for benign in [
            // 40-char git commit SHA.
            "9fceb02d0ae598e95dc970b74767f19372d61af8",
            // 64-char sha256 hex digest.
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            // canonical dashed UUID.
            "550e8400-e29b-41d4-a716-446655440000",
            // un-dashed UUID (pure hex).
            "550e8400e29b41d4a716446655440000",
            // long filesystem path as a single token.
            "/usr/local/lib/python3.11/site-packages/numpy/core/_multiarray_umath",
        ] {
            let (bytes, state) = policy.apply(format!("ref {benign} ok").as_bytes());
            let text = String::from_utf8(bytes).unwrap();
            assert_eq!(state, "safe", "benign token wrongly redacted: {benign}");
            assert!(
                text.contains(benign),
                "benign token {benign} must survive untouched: {text}"
            );
        }
    }

    #[test]
    fn credential_scan_leaves_ordinary_text_untouched() {
        // ACI7: the scan must not blank out ordinary prose, paths, or short
        // identifiers, or it would hide useful output from the agent.
        let policy = RedactionPolicy::new(Vec::new());
        let prose = "the quick brown fox jumps over /usr/local/bin/cargo \
                     and runs test_case_42 then returns Ok(())";
        let (bytes, state) = policy.apply(prose.as_bytes());
        let text = String::from_utf8(bytes).unwrap();
        assert_eq!(
            state, "safe",
            "ordinary text should not be redacted: {text}"
        );
        assert_eq!(text, prose);
    }

    #[test]
    fn credential_scan_scrubs_a_bearer_token() {
        let policy = RedactionPolicy::new(Vec::new());
        let (bytes, state) = policy.apply(b"Authorization: Bearer abc123def456");
        let text = String::from_utf8(bytes).unwrap();
        assert_eq!(state, "redacted");
        assert!(
            !text.contains("abc123def456"),
            "bearer token leaked: {text}"
        );
    }

    #[test]
    fn rules_only_policy_skips_the_credential_scan() {
        // The rules-only policy applies declared patterns but does NOT run the
        // default credential-shape scan.
        let policy = RedactionPolicy::rules_only(Vec::new());
        assert!(!policy.scans_credentials());
        let (bytes, state) = policy.apply(b"key AKIAIOSFODNN7EXAMPLE");
        assert_eq!(state, "safe");
        assert_eq!(
            String::from_utf8(bytes).unwrap(),
            "key AKIAIOSFODNN7EXAMPLE"
        );
    }

    #[test]
    fn spawn_piped_process_drives_a_line_protocol_and_reaps_on_shutdown() {
        // DP1: deterministic coverage of the piped spawn path the live ACP wire
        // client borrows. Spawn a trivial echoing program through the runtime,
        // write one JSON-RPC-shaped line to the taken stdin, read the echoed line
        // back from the taken stdout, confirm the runtime owns the process group,
        // and assert shutdown() reaps the child to status "exited" with its stderr
        // artifact captured.
        use std::io::{BufRead, BufReader, Write};

        let workspace = temp_root("piped-workspace");
        let artifacts = temp_root("piped-artifacts");
        fs::create_dir_all(&workspace).unwrap();
        let runner =
            LocalProcessRunner::new(LocalProcessConfig::for_test(workspace.clone(), artifacts));

        // `/bin/cat` echoes each stdin line to stdout, a minimal bidirectional
        // line protocol.
        let mut process = runner
            .spawn_piped_process(LocalProcessRequest::new(
                RunId::new("run-piped"),
                "/bin/cat",
                Vec::new(),
                workspace,
                HashMap::new(),
            ))
            .expect("spawn piped process");

        // The runtime owns the process group (process_group(0) on unix sets the
        // child's pgid to its own pid); the adapter only borrows the pipe handles.
        assert!(
            process.process.external_pid.is_some(),
            "runtime must record the owned process pid"
        );
        assert!(
            process.events.iter().any(|event| {
                event.kind == "runtime.process_started" && event.status == "started"
            })
        );

        let mut stdin = process.take_stdin().expect("stdin pipe");
        let stdout = process.take_stdout().expect("stdout pipe");
        let mut reader = BufReader::new(stdout);

        writeln!(
            stdin,
            "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}}"
        )
        .unwrap();
        stdin.flush().unwrap();

        let mut echoed = String::new();
        reader.read_line(&mut echoed).expect("read echoed line");
        assert!(
            echoed.contains("\"method\":\"ping\""),
            "expected the echoed protocol line, got: {echoed}"
        );

        // The pipe handles are takeable exactly once.
        assert!(process.take_stdin().is_none());
        assert!(process.take_stdout().is_none());

        let stderr_path = process.stderr_path().to_path_buf();
        let shutdown = process.shutdown("piped protocol complete");
        assert_eq!(shutdown.process.status, "exited");
        assert!(
            stderr_path.exists(),
            "the child's stderr artifact must be captured at {stderr_path:?}"
        );
    }
}
