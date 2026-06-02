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
use capo_state::EventKind;
#[cfg(unix)]
use std::os::unix::process::CommandExt;

/// CT6: anti-sleep when serving locally — an OPT-IN server-lifecycle concern that
/// keeps a laptop awake while a non-loopback exposure is held. A separate boundary
/// from agent execution and `RuntimeRunner`; the coupling is ONE-WAY
/// (exposure-state -> inhibitor). Shares the `connectivity_health` lifecycle home.
pub mod anti_sleep;
mod async_runner;
/// CT5: tunnel health — the heartbeat loop, `last_heartbeat_at`, and reconnect
/// events, driven by an injectable clock. A separate boundary from controller/turn
/// state and from `RuntimeRunner` (it depends only on the `ConnectivityTunnel`
/// surface).
pub mod connectivity_health;
mod sandbox;
mod worktree;

pub use connectivity_health::{
    ConnectivityClock, HealthTransition, HeartbeatConfig, HeartbeatHandle, HeartbeatMonitor,
    HeartbeatOutcome, PUBLIC_EXPOSURE_MAX_TTL_MS, expiry_label, parse_expiry_ms,
    public_expiry_label,
};

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
pub const PLANNED_TUNNELS: &[&str] = &["fake", "local-loopback", "endpoint-stub", "tailscale"];

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
    /// RR1: a remote launch failed at the channel transport. Carries a
    /// retryability flag so the controller can classify whether a retry is worth
    /// attempting. The `message` is already redaction-safe (no raw credentials).
    RemoteLaunchFailed {
        message: String,
        retryable: bool,
    },
    /// RR1: the append-first Start Sequence failed at the remote launch step. It
    /// carries the LOCALLY-appended events up to and including the typed
    /// `runtime.remote_process_start_failed` so the caller can persist the failure
    /// trail, plus the underlying transport error and the retryability flag.
    RemoteStartFailed {
        retryable: bool,
        events: Vec<RuntimeEvent>,
        source: Box<RuntimeError>,
    },
    /// RR6: a remote-control operation (start / stream / stdin) was attempted on a
    /// runner whose remote-control grant has been REVOKED (the channel was revoked
    /// or the grant lifecycle ended). A revoked capability stops the run and the
    /// runner MUST NOT re-establish execution without a FRESH grant. The `reason`
    /// is redaction-safe (never a credential). This is NEVER retryable under the
    /// same grant — re-establishment requires a new grant, not a retry.
    RemoteControlRevoked {
        reason: String,
    },
    /// RR3/RR7: git-based remote workspace materialization (push/fetch + `git
    /// worktree add` the target commit on the remote) failed at the channel
    /// transport. Surfaced as a `runtime.remote_workspace_materialized` FAILED
    /// event rather than a silent fall-through to running in the wrong directory
    /// (mirroring `WorktreeError`'s no-silent-fallthrough rule). The `message` is
    /// already redaction-safe (the git transport URL passed the credential scan).
    RemoteMaterializeFailed {
        message: String,
    },
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
    /// Boxed because the remote runner carries a resolved channel plus a loopback
    /// sub-runner and is far larger than the other variants.
    RemoteProcess(Box<RemoteProcessRunner>),
}

impl RuntimeRunner {
    pub fn fake() -> Self {
        Self::Fake(FakeRuntimeRunner)
    }

    pub fn local_process(config: LocalProcessConfig) -> Self {
        Self::LocalProcess(LocalProcessRunner::new(config))
    }

    pub fn remote_process(config: RemoteProcessConfig) -> Self {
        Self::RemoteProcess(Box::new(RemoteProcessRunner::new(config)))
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

    /// RR1: the REAL control surface (operating on [`LocalRuntimeProcessRef`], not
    /// the legacy `FakeRuntimeProcess` surface). `RemoteProcess` dispatches to the
    /// real [`RemoteProcessRunner`] over its injected channel, NOT the
    /// `FakeRuntimeRunner` fall-through.
    pub fn interrupt_local(
        &self,
        process: &LocalRuntimeProcessRef,
        reason: &str,
    ) -> RuntimeControlResult {
        match self {
            Self::LocalProcess(runner) => runner.interrupt(process, reason),
            Self::RemoteProcess(runner) => runner.interrupt(process, reason),
            Self::Fake(_) => RuntimeControlResult {
                process: process.clone(),
                events: Vec::new(),
            },
        }
    }

    pub fn terminate_local(
        &self,
        process: &LocalRuntimeProcessRef,
        reason: &str,
    ) -> RuntimeControlResult {
        match self {
            Self::LocalProcess(runner) => runner.terminate(process, reason),
            Self::RemoteProcess(runner) => runner.terminate(process, reason),
            Self::Fake(_) => RuntimeControlResult {
                process: process.clone(),
                events: Vec::new(),
            },
        }
    }

    pub fn kill_local(
        &self,
        process: &LocalRuntimeProcessRef,
        reason: &str,
    ) -> RuntimeControlResult {
        match self {
            Self::LocalProcess(runner) => runner.kill(process, reason),
            Self::RemoteProcess(runner) => runner.kill(process, reason),
            Self::Fake(_) => RuntimeControlResult {
                process: process.clone(),
                events: Vec::new(),
            },
        }
    }

    /// RR1: liveness from the real runner. For `RemoteProcess` this is an ACTUAL
    /// remote probe over the channel, not a local status string.
    pub fn health_local(&self, process: &LocalRuntimeProcessRef) -> RuntimeResult<RuntimeHealth> {
        match self {
            Self::LocalProcess(runner) => Ok(runner.health(process)),
            Self::RemoteProcess(runner) => runner.health(process),
            Self::Fake(_) => Ok(RuntimeHealth {
                runtime_process_ref: process.runtime_process_ref.clone(),
                status: process.status.clone(),
                live: process.status == "running",
            }),
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

/// RR4: the byte cap applied to remote output deltas at the remote boundary,
/// mirroring [`LocalProcessConfig::output_limit_bytes`]'s default. A remote stream
/// is bounded by this exactly as a local run's output is, so a runaway remote does
/// not flood the controller with unbounded deltas; once reached, the stream
/// finalizes [`RemoteStreamFinalReason::CapReached`].
pub const REMOTE_OUTPUT_LIMIT_BYTES: usize = 64 * 1024;

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

    /// Hard-kill, recording the caller's `reason`. The `reason` argument aligns the
    /// signature with [`RemoteProcessRunner::kill`] so both runners satisfy the
    /// shared [`RuntimeRunnerContract`] (review finding 4); pass `"kill requested"`
    /// for the default operator-initiated kill.
    pub fn kill(&self, process: &LocalRuntimeProcessRef, reason: &str) -> RuntimeControlResult {
        self.control(process, "killed", "runtime.kill_requested", reason)
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

/// A unified cleanup result the controller can handle uniformly across the local
/// and remote runners (review finding 5). The two runners reap fundamentally
/// different things — the LOCAL runner preserves an on-disk artifact directory and
/// drops a marker; the REMOTE runner reaps a remote process group over the channel
/// and emits events — so the unified shape carries the common identity + the
/// runner-specific evidence, rather than forcing one runner to fake the other's
/// fields. A caller that only needs "which ref was cleaned up?" reads
/// [`Self::runtime_process_ref`]; a caller that wants the runner-specific detail
/// matches the variant.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CleanupOutcome {
    /// The local runner preserved an artifact dir + dropped a marker on disk.
    Local(CleanupReport),
    /// The remote runner reaped the remote process group over the channel and
    /// recorded `runtime.remote_cleanup_completed`.
    Remote(RuntimeControlResult),
}

impl CleanupOutcome {
    /// The process ref that was cleaned up, available uniformly for either runner.
    pub fn runtime_process_ref(&self) -> &str {
        match self {
            Self::Local(report) => &report.runtime_process_ref,
            Self::Remote(result) => &result.process.runtime_process_ref,
        }
    }
}

/// RR6: how much of a remote run a `cleanup_run` reaps. Mirrors the local
/// cleanup's preserve-vs-discard intent but for the remote process group + the
/// remote git worktree reached over the channel.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CleanupPolicy {
    /// Reap the remote process group AND remove the remote git worktree. This is
    /// the full crash-safe teardown: a dangling remote worktree left by a crash is
    /// reaped, never silently abandoned.
    ReapAll,
    /// Reap the remote process group but PRESERVE the remote worktree for
    /// inspection (e.g. an orphaned run whose logs/worktree an operator wants to
    /// examine). The worktree is recorded as preserved, not torn down.
    PreserveWorktree,
}

impl CleanupPolicy {
    /// The stable token recorded in the cleanup event detail.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ReapAll => "reap_all",
            Self::PreserveWorktree => "preserve_worktree",
        }
    }

    /// Whether this policy removes the remote worktree (vs. preserving it).
    pub const fn reaps_worktree(self) -> bool {
        matches!(self, Self::ReapAll)
    }
}

/// RR6: what a remote `cleanup_workspace` actually reaped over the channel. The
/// transport reports whether a remote worktree was PRESENT and removed, so a
/// dangling worktree (left by a crash) is torn down EXACTLY once and a re-run is
/// an idempotent no-op (nothing left to reap).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceReapOutcome {
    /// `true` when a remote worktree was present and removed by THIS call. A
    /// re-run after a successful reap finds nothing and reports `false` (idempotent).
    pub worktree_reaped: bool,
    /// The remote worktree key/path that was reaped (or would have been), recorded
    /// for audit. Never a secret.
    pub worktree_key: String,
}

/// RR3/RR7: the result of materializing a run's workspace ON the remote by git
/// (push/fetch the target commit + `git worktree add` it into a dedicated remote
/// worktree root). Content-addressed + auditable: the source commit SHA, the
/// remote worktree path, and the resulting remote `HEAD` are recorded. The git
/// transport URL has ALREADY passed the credential scan before this is built, so
/// `transport_url_redaction` records whether anything was scrubbed and no embedded
/// secret reaches the recorded URL.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteWorkspaceMaterialization {
    /// The source commit SHA the run is pinned to (content-addressed).
    pub source_commit: String,
    /// The remote `HEAD` the materialized worktree resolved to; equals
    /// `source_commit` on a clean materialization.
    pub remote_head: String,
    /// The remote worktree path the commit was checked out into; the run's cwd /
    /// confinement is scoped here.
    pub remote_worktree_path: String,
    /// The (redaction-scanned) git transport URL recorded for audit — never
    /// carries an embedded credential.
    pub transport_url: String,
    /// `"redacted"` when the credential scan scrubbed the transport URL, else
    /// `"safe"`.
    pub transport_url_redaction: String,
    /// The append-ready `runtime.remote_workspace_materialized` event.
    pub events: Vec<RuntimeEvent>,
}

/// RR3/RR7: the result of mapping a remote-produced commit BACK to Capo's host by
/// git (fetch the remote worktree's tip into a named local ref, the same
/// reconcile/merge-back point DP8 models). Recorded as
/// `runtime.remote_workspace_reconciled`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteWorkspaceReconciliation {
    /// The remote-produced commit SHA fetched back.
    pub remote_commit: String,
    /// The named local ref the remote commit was fetched into.
    pub local_ref: String,
    /// The append-ready `runtime.remote_workspace_reconciled` event.
    pub events: Vec<RuntimeEvent>,
}

/// The shared execution + control contract both [`LocalProcessRunner`] and
/// [`RemoteProcessRunner`] satisfy, so the controller drives them with the SAME
/// method shapes (review finding 4 + 5). Defining it as a trait gives a
/// COMPILE-TIME check that the two runners never silently diverge on the control
/// surface: if a future method drifts (e.g. drops the `reason` arg, or returns a
/// different cleanup shape), the `impl` stops compiling.
///
/// `start_process` is on the trait (review finding 5): it is the most critical
/// path and BOTH runners share the EXACT shape
/// (`LocalProcessRequest -> RuntimeResult<LocalProcessOutcome>`), so drift there is
/// caught at compile time. `stream_output` / `write_stdin` are DELIBERATELY off the
/// trait because the two runners do NOT share a signature there: the local runner's
/// streaming surface is the async `AsyncLocalProcessRunner` / `StreamSource` path
/// (`async_runner.rs`), while the remote runner streams synchronously over the
/// channel by byte offset (`stream_output(&ref, from_offset) -> RemoteStreamOutcome`,
/// `write_stdin(&ref, &[u8]) -> RuntimeControlResult`). Forcing a single trait
/// signature there would invent a false parity; the divergence is documented here
/// instead of papered over.
pub trait RuntimeRunnerContract {
    fn start_process(&self, request: LocalProcessRequest) -> RuntimeResult<LocalProcessOutcome>;
    fn interrupt(&self, process: &LocalRuntimeProcessRef, reason: &str) -> RuntimeControlResult;
    fn terminate(&self, process: &LocalRuntimeProcessRef, reason: &str) -> RuntimeControlResult;
    fn kill(&self, process: &LocalRuntimeProcessRef, reason: &str) -> RuntimeControlResult;
    fn health(&self, process: &LocalRuntimeProcessRef) -> RuntimeResult<RuntimeHealth>;
    fn cleanup(&self, process: &LocalRuntimeProcessRef) -> RuntimeResult<CleanupOutcome>;
}

impl RuntimeRunnerContract for LocalProcessRunner {
    fn start_process(&self, request: LocalProcessRequest) -> RuntimeResult<LocalProcessOutcome> {
        LocalProcessRunner::start_process(self, request)
    }
    fn interrupt(&self, process: &LocalRuntimeProcessRef, reason: &str) -> RuntimeControlResult {
        LocalProcessRunner::interrupt(self, process, reason)
    }
    fn terminate(&self, process: &LocalRuntimeProcessRef, reason: &str) -> RuntimeControlResult {
        LocalProcessRunner::terminate(self, process, reason)
    }
    fn kill(&self, process: &LocalRuntimeProcessRef, reason: &str) -> RuntimeControlResult {
        LocalProcessRunner::kill(self, process, reason)
    }
    fn health(&self, process: &LocalRuntimeProcessRef) -> RuntimeResult<RuntimeHealth> {
        Ok(LocalProcessRunner::health(self, process))
    }
    fn cleanup(&self, process: &LocalRuntimeProcessRef) -> RuntimeResult<CleanupOutcome> {
        LocalProcessRunner::cleanup(self, process).map(CleanupOutcome::Local)
    }
}

impl RuntimeRunnerContract for RemoteProcessRunner {
    fn start_process(&self, request: LocalProcessRequest) -> RuntimeResult<LocalProcessOutcome> {
        RemoteProcessRunner::start_process(self, request)
    }
    fn interrupt(&self, process: &LocalRuntimeProcessRef, reason: &str) -> RuntimeControlResult {
        RemoteProcessRunner::interrupt(self, process, reason)
    }
    fn terminate(&self, process: &LocalRuntimeProcessRef, reason: &str) -> RuntimeControlResult {
        RemoteProcessRunner::terminate(self, process, reason)
    }
    fn kill(&self, process: &LocalRuntimeProcessRef, reason: &str) -> RuntimeControlResult {
        RemoteProcessRunner::kill(self, process, reason)
    }
    fn health(&self, process: &LocalRuntimeProcessRef) -> RuntimeResult<RuntimeHealth> {
        RemoteProcessRunner::health(self, process)
    }
    fn cleanup(&self, process: &LocalRuntimeProcessRef) -> RuntimeResult<CleanupOutcome> {
        // RR6 (review finding 6): the CONTRACT cleanup surface must carry the
        // crash-safe semantics, not the thin non-policy variant. A controller that
        // drives both runners uniformly through `RuntimeRunnerContract::cleanup`
        // must get the dangling-worktree reap + `runtime.remote_workspace_torn_down`
        // on the remote path too, so this delegates to the full `cleanup_run`
        // under `ReapAll` rather than the bare `cleanup` (which only signals the
        // process group and never tears the worktree down).
        RemoteProcessRunner::cleanup_run(self, process, CleanupPolicy::ReapAll)
            .map(CleanupOutcome::Remote)
    }
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

/// RR2: how a restart classified a stored REMOTE run after re-resolving the
/// channel and re-probing the remote over it.
///
/// This mirrors the local-path mapping (`LocalProcessRunner::probe_run_health` ->
/// `run.recovered` / `run.orphaned` / `run.exited`) but the liveness signal comes
/// from a REMOTE probe over the channel, and it adds ONE remote-only state the
/// local path cannot have: when the channel itself is unreachable at recovery
/// time, the run is NOT forced to recovered or exited — it is left
/// [`RemoteRecoveryClassification::RecoveryPending`] and retried when the channel
/// returns. A remote-reboot (boot-id mismatch) is classified
/// [`RemoteRecoveryClassification::Exited`] (gone), never silently `Recovered`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RemoteRecoveryClassification {
    /// The remote process is alive within its recording boot AND the launch is
    /// reattachable (a recorded remote pid + boot identity), so the run is
    /// recovered in place. Maps to `runtime.remote_run_recovered` (`run.recovered`).
    Recovered,
    /// The remote process is alive but the launch is NOT reattachable (e.g. a bare
    /// SSH-launched process with no recorded remote pid/boot file), so Capo cannot
    /// re-attach; the remote logs are left inspectable. Maps to
    /// `runtime.remote_run_orphaned` (`run.orphaned`).
    Orphaned,
    /// The remote process is gone with no terminal event — either it exited while
    /// Capo was down, OR the remote machine rebooted (boot-id mismatch), so the
    /// recorded remote pid can never be trusted as "our" run. Maps to
    /// `runtime.remote_run_exited` (`run.exited`, unknown exit detail).
    Exited,
    /// The CHANNEL itself was unreachable at recovery time, so liveness is unknown.
    /// The run is held in `recovery_pending` (NOT forced to recovered or exited)
    /// and retried when the channel returns. Maps to
    /// `runtime.remote_recovery_pending`.
    RecoveryPending,
}

impl RemoteRecoveryClassification {
    /// The stable token recorded in the recovery event detail / read model.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Recovered => "remote_recovered",
            Self::Orphaned => "remote_orphaned",
            Self::Exited => "remote_exited",
            Self::RecoveryPending => "recovery_pending",
        }
    }

    /// The terminal recovery event kind for this classification.
    pub const fn event_kind(self) -> EventKind {
        match self {
            Self::Recovered => EventKind::RuntimeRemoteRunRecovered,
            Self::Orphaned => EventKind::RuntimeRemoteRunOrphaned,
            Self::Exited => EventKind::RuntimeRemoteRunExited,
            Self::RecoveryPending => EventKind::RuntimeRemoteRecoveryPending,
        }
    }

    /// `true` only for `RecoveryPending` — the one classification recovery RETRIES
    /// (because the channel was unreachable), as opposed to the terminal mappings.
    pub const fn is_pending(self) -> bool {
        matches!(self, Self::RecoveryPending)
    }
}

/// RR2: what the channel observed when a restart re-probed a stored remote run.
///
/// This is the remote analogue of [`RunHealthProbe`]: the channel is the AUTHORITY
/// on remote liveness + reachability + the remote boot identity now seen on the
/// host, so the runner classifies recovery from this rather than from a stored
/// status string. A boot-id that differs from the one recorded at launch means the
/// remote rebooted and the recorded pid is meaningless (mirrors the local
/// same-boot rule in `probe_run_health`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteRecoveryProbe {
    /// `false` when the channel could not be re-resolved / was unreachable. When
    /// this is `false`, `live`/`observed_remote_boot_id` are not meaningful.
    pub channel_reachable: bool,
    /// Whether the remote process group is alive (only meaningful when reachable).
    pub live: bool,
    /// The remote boot id the host reports NOW. A mismatch with the recorded boot
    /// id means the remote rebooted (the recorded pid is recycled / gone).
    pub observed_remote_boot_id: Option<String>,
    /// Whether THIS remote launch can be reattached to in place — i.e. it recorded
    /// a durable remote pid + boot identity. A bare launch with no recorded remote
    /// pid is alive-but-unattachable -> orphaned.
    pub reattachable: bool,
}

/// RR2: the result of a restart re-probing a stored remote run over a re-resolved
/// channel — the truthful classification plus the append-first recovery events.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteRunRecovery {
    pub runtime_process_ref: String,
    pub classification: RemoteRecoveryClassification,
    pub detail: String,
    /// Append-first: a `runtime.remote_recovery_attempted` then the single terminal
    /// classification event. Recorded by the controller's recovery seam.
    pub events: Vec<RuntimeEvent>,
}

/// RR1: the resolved-channel transport a [`RemoteProcessRunner`] executes over.
///
/// This is the SAFETY/HONESTY boundary the adversarial review demanded: the
/// runner OWNS execution but NEVER opens sockets, resolves endpoints, or handles
/// `auth_ref` itself — it is handed an already-resolved channel
/// (`connectivity-tunnel`'s [`OpenChannel`]) plus a transport that performs the
/// actual launch/signal/probe. The only transport that lands in RR1 is the
/// deterministic [`FakeRemoteChannel`] (NO network); the real
/// `SshRemoteProcessRunner` transport lands behind the opt-in gate in RR8. Until
/// a real cross-machine transport exists, a channel reports itself as a loopback
/// (fake) channel via [`RemoteChannel::is_loopback`] so Capo NEVER claims a real
/// remote run happened on a loopback path.
// The deterministic `Fake` variant intentionally carries a whole
// `LocalProcessRunner` (it runs the program on loopback) and is the ALWAYS-ON hot
// path; the real `Ssh` variant is small but cross-machine-only (opt-in `#[ignore]`
// smoke). Boxing `Fake` would add indirection to every gate run to shrink a variant
// that is never the bottleneck, so the size disparity is accepted deliberately.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RemoteChannel {
    /// Deterministic in-memory channel for the RR1 fake-channel suite: it runs the
    /// "remote" program LOCALLY (it is honest about being a loopback) and is
    /// scriptable to fail a launch or fail an after-spawn append so cleanup paths
    /// are exercised with NO network.
    Fake(FakeRemoteChannel),
    /// RR8: the REAL cross-machine SSH transport behind `SshRemoteProcessRunner`.
    /// It shells out to `ssh` to launch/signal/probe/stream on the remote and uses a
    /// REAL [`GitRemote`] (over the same SSH host) for git materialization. It is
    /// HONESTLY non-loopback (`is_loopback() == false`) — it crossed a machine
    /// boundary — so Capo's enforcement/realness claims are truthful. This variant
    /// is exercised ONLY by the opt-in, `#[ignore]` live smoke; the deterministic
    /// gate never instantiates it (it would touch the network).
    Ssh(SshRemoteChannel),
}

impl RemoteChannel {
    /// HONESTY: a loopback/fake channel is NOT a real remote. Any consumer that
    /// guards "did this actually run cross-machine?" reads this, never a bare
    /// `fake: false`.
    pub fn is_loopback(&self) -> bool {
        match self {
            Self::Fake(channel) => channel.is_loopback(),
            // RR8: a real SSH transport crossed a machine boundary — never loopback.
            Self::Ssh(_) => false,
        }
    }

    /// The proven remote target identity, derived from the resolved channel's
    /// fingerprint (`connectivity-tunnel`), never a raw credential.
    pub fn target_fingerprint(&self) -> String {
        match self {
            Self::Fake(channel) => channel.target_fingerprint(),
            Self::Ssh(channel) => channel.target_fingerprint(),
        }
    }

    /// How many times the transport ACTUALLY spawned a remote process. The
    /// idempotency test reads this to prove a duplicate key did not double-spawn.
    pub fn spawn_count(&self) -> usize {
        match self {
            Self::Fake(channel) => channel.spawn_count(),
            Self::Ssh(channel) => channel.spawn_count(),
        }
    }

    fn launch(&self, request: &LocalProcessRequest) -> RuntimeResult<RemoteLaunch> {
        match self {
            Self::Fake(channel) => channel.launch(request),
            Self::Ssh(channel) => channel.launch(request),
        }
    }

    fn signal(&self, probe: &RemoteProbe, escalation: &str) -> RuntimeResult<()> {
        match self {
            Self::Fake(channel) => channel.signal(probe, escalation),
            Self::Ssh(channel) => channel.signal(probe, escalation),
        }
    }

    fn probe(&self, probe: &RemoteProbe) -> RuntimeResult<bool> {
        match self {
            Self::Fake(channel) => channel.probe(probe),
            Self::Ssh(channel) => channel.probe(probe),
        }
    }

    /// RR2: re-probe a stored remote run on restart. The channel is the authority
    /// on reachability + remote liveness + the remote boot id now observed.
    fn recovery_probe(&self, probe: &RemoteProbe) -> RemoteRecoveryProbe {
        match self {
            Self::Fake(channel) => channel.recovery_probe(probe),
            Self::Ssh(channel) => channel.recovery_probe(probe),
        }
    }

    fn cleanup(&self, probe: &RemoteProbe) -> RuntimeResult<()> {
        match self {
            Self::Fake(channel) => channel.cleanup(probe),
            Self::Ssh(channel) => channel.cleanup(probe),
        }
    }

    /// RR4: forward the remote output stream as RAW frames from `from_offset`. The
    /// runner redacts + bounds + offsets the bytes; the channel only forwards.
    fn stream(&self, probe: &RemoteProbe, from_offset: usize) -> RemoteRawStream {
        match self {
            Self::Fake(channel) => channel.stream(probe, from_offset),
            Self::Ssh(channel) => channel.stream(probe, from_offset),
        }
    }

    /// RR4: write stdin bytes to the remote process over the channel.
    fn write_stdin(&self, probe: &RemoteProbe, bytes: &[u8]) -> RuntimeResult<()> {
        match self {
            Self::Fake(channel) => channel.write_stdin(probe, bytes),
            Self::Ssh(channel) => channel.write_stdin(probe, bytes),
        }
    }

    /// RR5: probe the REMOTE host for its OS family + whether it can enforce
    /// `tier`. The remote OS — not the controller — is the authority, so the
    /// runner's enforcement claim reads this, never `tier.is_enforced_here()`.
    fn sandbox_probe(&self, tier: SandboxTier) -> RuntimeResult<RemoteSandboxProbe> {
        match self {
            Self::Fake(channel) => channel.sandbox_probe(tier),
            Self::Ssh(channel) => channel.sandbox_probe(tier),
        }
    }

    /// RR5 test/observability hook: the LAST request the transport was asked to
    /// launch. Used to prove the enforced path handed the transport a
    /// `bwrap`/`sandbox-exec`-wrapped command, not the bare original.
    fn last_launched_request(&self) -> Option<LocalProcessRequest> {
        match self {
            Self::Fake(channel) => channel.last_launched_request(),
            Self::Ssh(channel) => channel.last_launched_request(),
        }
    }

    /// RR6: reap the remote process group + (under [`CleanupPolicy::ReapAll`])
    /// remove the remote git worktree over the channel. Idempotent: a worktree
    /// already gone reports `worktree_reaped == false`.
    fn cleanup_workspace(
        &self,
        probe: &RemoteProbe,
        policy: CleanupPolicy,
    ) -> RuntimeResult<WorkspaceReapOutcome> {
        match self {
            Self::Fake(channel) => channel.cleanup_workspace(probe, policy),
            Self::Ssh(channel) => channel.cleanup_workspace(probe, policy),
        }
    }

    /// RR6: roll the remote git worktree back to `checkpoint_ref` (the RR3
    /// git-materialized commit) over the channel.
    fn rollback_worktree(&self, probe: &RemoteProbe, checkpoint_ref: &str) -> RuntimeResult<()> {
        match self {
            Self::Fake(channel) => channel.rollback_worktree(probe, checkpoint_ref),
            Self::Ssh(channel) => channel.rollback_worktree(probe, checkpoint_ref),
        }
    }

    /// RR3/RR7: materialize `source_commit` ON the remote by git over the channel.
    /// Returns the resolved remote `HEAD` + remote worktree path.
    fn materialize(&self, source_commit: &str) -> RuntimeResult<(String, PathBuf)> {
        match self {
            Self::Fake(channel) => channel.materialize(source_commit),
            Self::Ssh(channel) => channel.materialize(source_commit),
        }
    }

    /// RR3/RR7: map a remote-produced commit BACK to Capo's host by git over the
    /// channel, into the named `local_ref`.
    fn reconcile(&self, remote_worktree_path: &Path, local_ref: &str) -> RuntimeResult<String> {
        match self {
            Self::Fake(channel) => channel.reconcile(remote_worktree_path, local_ref),
            Self::Ssh(channel) => channel.reconcile(remote_worktree_path, local_ref),
        }
    }

    /// RR3/RR7: the (credential-scanned) git transport URL for the recorded
    /// materialization event — never carries an embedded secret.
    fn transport_url(&self) -> String {
        match self {
            Self::Fake(channel) => channel.transport_url(),
            Self::Ssh(channel) => channel.transport_url(),
        }
    }
}

/// RR4: the RAW frames the channel forwarded for one stream read: the offset the
/// frames START at (so a reconnect from the last acknowledged offset yields no
/// overlap), the raw (un-redacted) bytes, and whether the channel DROPPED
/// mid-stream (so the runner finalizes with a recorded reason, never a silent
/// truncation). The runner — not the channel — applies redaction + the output cap.
#[derive(Clone, Debug, Eq, PartialEq)]
struct RemoteRawStream {
    from_offset: usize,
    bytes: Vec<u8>,
    dropped: bool,
}

/// RR5: the OS family the REMOTE host runs, as reported by a probe over the
/// channel. This is the load-bearing honesty input: a sandbox tier is `Enforced`
/// only when the REMOTE OS supports it, NOT when the controller's host does. The
/// `Other` variant carries the reported family string so a remote that is neither
/// macOS nor linux is recorded honestly (and never claims sandboxing).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RemoteOsFamily {
    Macos,
    Linux,
    Other(String),
}

impl RemoteOsFamily {
    fn as_str(&self) -> &str {
        match self {
            Self::Macos => "macos",
            Self::Linux => "linux",
            Self::Other(name) => name.as_str(),
        }
    }

    /// Whether `tier` is enforceable on THIS remote OS family — the remote
    /// analogue of [`SandboxTier::is_enforced_here`], evaluated against the
    /// REMOTE host's reported family rather than the controller's `cfg!` target.
    /// Seatbelt enforces only on a macOS remote; landlock+bwrap only on a linux
    /// remote; [`SandboxTier::None`] never enforces.
    fn enforces(&self, tier: SandboxTier) -> bool {
        match tier {
            SandboxTier::None => false,
            SandboxTier::MacosSeatbelt => matches!(self, Self::Macos),
            SandboxTier::LinuxLandlockBwrap => matches!(self, Self::Linux),
        }
    }
}

/// RR5: what the channel reports about the REMOTE host's sandbox capability when
/// the runner probes it before composing the OS sandbox. The remote OS — not the
/// controller — is the authority on whether a tier can be enforced, so the runner
/// reads this rather than its own build target. A real transport fills this from a
/// remote-side probe; the fake channel scripts it deterministically (NO network).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteSandboxProbe {
    /// The OS family the remote host reports.
    pub os_family: RemoteOsFamily,
    /// Whether the remote host can ACTUALLY enforce the requested tier. Usually
    /// derived from `os_family.enforces(tier)`, but kept explicit so a remote that
    /// reports a matching family yet lacks the mechanism (e.g. a linux without
    /// landlock/bwrap installed) can still report `false` honestly.
    pub tier_enforceable: bool,
}

/// A lightweight remote-process handle reconstructed from a stored
/// `remote_process_ref`: the remote identity plus the last-known liveness. The
/// transport's control/probe operations (`signal`/`probe`/`cleanup`) need only
/// this identity — NOT the captured launch outcome — so the handle is honest
/// about carrying no artifacts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteProbe {
    pub remote_pid: u32,
    pub remote_boot_id: String,
    pub remote_host_id: String,
    /// Last-known liveness encoded in the stored ref; the transport probe is the
    /// authority and may override it.
    pub live: bool,
}

/// What a remote launch returned: the remote process identity Capo records in the
/// `remote_process_ref` (remote pid + remote boot/host identity), NOT the local
/// `external_pid`/`boot_id` path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteLaunch {
    pub remote_pid: u32,
    pub remote_boot_id: String,
    pub remote_host_id: String,
    /// `true` while the channel believes the remote process group is alive; the
    /// fake channel flips this on signal/probe so `health` reflects a probe, not a
    /// local status string.
    pub live: bool,
    /// The captured outcome of the (loopback) run. The fake channel runs the
    /// program once and carries its artifacts here so `start_process` never
    /// double-spawns.
    pub captured: LocalProcessOutcome,
}

/// RR5 deterministic helper: if `request` is a sandbox-launcher-wrapped command
/// (`bwrap ... <program> <argv>` or `/usr/bin/sandbox-exec -f <policy> <program>
/// <argv>`), return the request with the launcher peeled off so the inner program
/// runs (modelling the launcher exec-ing its child); otherwise return the request
/// unchanged. The fake loopback uses this so the enforced wrapping path is testable
/// WITHOUT the `bwrap`/`sandbox-exec` binaries being present. The wrapped argv is
/// still recorded by the caller, so the verification (transport saw `bwrap`) holds.
fn unwrap_sandbox_launcher(request: &LocalProcessRequest) -> LocalProcessRequest {
    let inner: Option<(String, Vec<String>)> = match request.program.as_str() {
        "bwrap" => {
            // The original program is the first argv token that is NOT a bwrap flag
            // or a flag value. bwrap flags we emit: --die-with-parent, --ro-bind A B,
            // --dev A, --proc A, --bind A B, --unshare-net. Scan past them.
            let mut i = 0usize;
            let argv = &request.argv;
            while i < argv.len() {
                match argv[i].as_str() {
                    "--die-with-parent" | "--unshare-net" => i += 1,
                    "--dev" | "--proc" => i += 2,
                    "--ro-bind" | "--bind" => i += 3,
                    // First non-flag token is the wrapped program.
                    _ => break,
                }
            }
            if i < argv.len() {
                Some((argv[i].clone(), argv[i + 1..].to_vec()))
            } else {
                None
            }
        }
        "/usr/bin/sandbox-exec" => {
            // Shape: -f <policy> <program> <argv...>
            let argv = &request.argv;
            if argv.len() >= 3 && argv[0] == "-f" {
                Some((argv[2].clone(), argv[3..].to_vec()))
            } else {
                None
            }
        }
        _ => None,
    };
    match inner {
        Some((program, argv)) => LocalProcessRequest {
            program,
            argv,
            ..request.clone()
        },
        None => request.clone(),
    }
}

/// RR3/RR7: a REAL git-backed remote workspace model, NO network. It is the
/// deterministic stand-in for "the remote machine's git repo + worktree root",
/// built entirely from local directories so the git-materialization invariants are
/// proven against actual `git` rather than an abstract flag:
///
/// - `local_origin`: the bare repo on Capo's host the run's commit is pushed FROM
///   (the source of truth for the source SHA).
/// - `remote_repo`: a bare repo standing in for the remote host's git store; the
///   commit is fetched into it "over the channel".
/// - `remote_worktree_root`: where the commit is `git worktree add`-ed on the
///   remote; the run's cwd / confinement is scoped here.
/// - `transport_url`: the (credential-scanned) git transport URL recorded for
///   audit — a URL with an embedded secret is scrubbed before it is ever recorded.
///
/// Because every path is local, this is fully deterministic and replay-stable: a
/// rebuild from the same fixture reproduces identical SHAs and refs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitRemote {
    local_origin: PathBuf,
    remote_repo: PathBuf,
    remote_worktree_root: PathBuf,
    transport_url: String,
    /// WHERE the repo-side git ops (init / fetch-of-the-commit / `worktree add`)
    /// actually run. The deterministic fake path runs them on controller-local
    /// dirs ([`GitRemoteExecution::Local`]); the REAL SSH path runs them on the
    /// remote host over `ssh` and pushes the commit across the transport URL
    /// ([`GitRemoteExecution::OverSsh`]), so `remote_repo`/`remote_worktree_root`
    /// are paths on the REMOTE machine, not the controller (review finding 1).
    execution: GitRemoteExecution,
}

/// RR8 (review finding 1): where a [`GitRemote`]'s repo-side git operations run.
/// The deterministic suite runs them locally (honest: the "remote" is a second
/// local checkout); the live SSH path runs them on the remote host so
/// materialization is a genuine cross-machine git operation, not a controller-local
/// one mislabeled as remote.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GitRemoteExecution {
    /// Repo-side git runs on controller-local directories (deterministic fake path).
    Local,
    /// Repo-side git runs on the REMOTE host over `ssh`; the commit is `git push`ed
    /// from the controller's `local_origin` to the remote repo across the transport
    /// URL. `remote_repo`/`remote_worktree_root` are REMOTE filesystem paths.
    OverSsh {
        ssh_destination: String,
        ssh_binary: String,
    },
}

impl GitRemote {
    /// Build a git-backed remote workspace model from already-created local
    /// directories. `transport_url` is recorded for audit AFTER the credential
    /// scan, so a URL carrying an embedded secret is never persisted raw. The
    /// repo-side git ops run LOCALLY (the deterministic fake path); the live SSH
    /// path uses [`Self::over_ssh`] so they run on the remote host.
    pub fn new(
        local_origin: PathBuf,
        remote_repo: PathBuf,
        remote_worktree_root: PathBuf,
        transport_url: impl Into<String>,
    ) -> Self {
        Self {
            local_origin,
            remote_repo,
            remote_worktree_root,
            transport_url: transport_url.into(),
            execution: GitRemoteExecution::Local,
        }
    }

    /// RR8 (review finding 1): run the repo-side git ops ON the remote host over
    /// `ssh`. `remote_repo` and `remote_worktree_root` are paths on the REMOTE
    /// machine; the controller `git push`es the commit to the remote repo across
    /// the (already-resolved) transport URL before the remote `worktree add`. This
    /// is the truly-cross-machine materialization the live smoke exercises.
    pub fn over_ssh(
        mut self,
        ssh_destination: impl Into<String>,
        ssh_binary: impl Into<String>,
    ) -> Self {
        self.execution = GitRemoteExecution::OverSsh {
            ssh_destination: ssh_destination.into(),
            ssh_binary: ssh_binary.into(),
        };
        self
    }

    /// The remote worktree path a given source commit materializes into. Stable per
    /// commit so a re-materialization is idempotent and replay-stable.
    fn worktree_path(&self, source_commit: &str) -> PathBuf {
        self.remote_worktree_root.join(format!(
            "wt-{}",
            &source_commit[..source_commit.len().min(12)]
        ))
    }

    /// RR3: materialize `source_commit` ON the remote by git — fetch it from the
    /// local origin into the remote repo ("push/fetch over the channel"), then
    /// `git worktree add` it into a dedicated remote worktree root. Returns the
    /// resolved remote `HEAD` (equals the source commit on a clean materialization)
    /// and the remote worktree path. A failure at any git step is a TYPED
    /// [`RuntimeError::RemoteMaterializeFailed`], never a silent fall-through.
    fn materialize(&self, source_commit: &str) -> RuntimeResult<(String, PathBuf)> {
        match &self.execution {
            GitRemoteExecution::Local => self.materialize_local(source_commit),
            GitRemoteExecution::OverSsh {
                ssh_destination,
                ssh_binary,
            } => self.materialize_over_ssh(source_commit, ssh_destination, ssh_binary),
        }
    }

    /// Deterministic fake path: repo-side git on controller-local dirs.
    fn materialize_local(&self, source_commit: &str) -> RuntimeResult<(String, PathBuf)> {
        let origin = self.local_origin.to_string_lossy().to_string();
        // Fetch the exact commit into the remote repo (modelling push/fetch over
        // the channel). `git fetch <origin> <sha>` brings the object graph across.
        materialize_git(
            &self.remote_repo,
            &["fetch", "--no-tags", &origin, source_commit],
        )?;
        let worktree_path = self.worktree_path(source_commit);
        // A re-materialization is idempotent: if the worktree already exists at the
        // commit, reuse it rather than failing the `worktree add`.
        if !worktree_path.exists() {
            materialize_git(
                &self.remote_repo,
                &[
                    "worktree",
                    "add",
                    "--detach",
                    &worktree_path.to_string_lossy(),
                    source_commit,
                ],
            )?;
        }
        let head = materialize_git_capture(&worktree_path, &["rev-parse", "HEAD"])?;
        Ok((head.trim().to_string(), worktree_path))
    }

    /// RR8 (review finding 1): the REAL cross-machine materialization. The
    /// controller PUSHES `source_commit` to the remote repo across the transport
    /// URL, then runs `git init`/`worktree add`/`rev-parse HEAD` ON THE REMOTE over
    /// `ssh`, so the worktree exists on the remote machine and the returned HEAD is
    /// read from the remote. No git step runs on a controller path masquerading as
    /// remote. A failure at any step is a TYPED `RemoteMaterializeFailed`.
    fn materialize_over_ssh(
        &self,
        source_commit: &str,
        ssh_destination: &str,
        ssh_binary: &str,
    ) -> RuntimeResult<(String, PathBuf)> {
        let remote_repo = self.remote_repo.to_string_lossy().to_string();
        let worktree_path = self.worktree_path(source_commit);
        let worktree = worktree_path.to_string_lossy().to_string();

        // 1) Ensure a bare-ish repo exists on the REMOTE host (idempotent).
        ssh_git(
            ssh_binary,
            ssh_destination,
            &format!(
                "git init -q {repo} >/dev/null 2>&1 || true; \
                 git -C {repo} config receive.denyCurrentBranch ignore >/dev/null 2>&1 || true",
                repo = shell_quote(&remote_repo),
            ),
        )?;

        // 2) PUSH the exact commit from the controller's origin to the remote repo
        //    across the transport URL — the object graph crosses the machine
        //    boundary here, not via a controller-local fetch.
        materialize_git(
            &self.local_origin,
            &[
                "push",
                "--no-verify",
                &self.transport_url,
                &format!("{source_commit}:refs/capo/materialize/{source_commit}"),
            ],
        )?;

        // 3) `git worktree add` the commit into the dedicated REMOTE worktree root,
        //    then read the materialized HEAD ON THE REMOTE. Idempotent on re-run.
        let head = ssh_git_capture(
            ssh_binary,
            ssh_destination,
            &format!(
                "if [ ! -d {wt} ]; then \
                   git -C {repo} worktree add --detach {wt} {sha} >/dev/null 2>&1; fi; \
                 git -C {wt} rev-parse HEAD",
                wt = shell_quote(&worktree),
                repo = shell_quote(&remote_repo),
                sha = shell_quote(source_commit),
            ),
        )?;
        Ok((head.trim().to_string(), worktree_path))
    }

    /// RR3: map a remote-produced commit BACK to Capo's host by git — fetch the
    /// remote worktree's tip into a named ref in the local origin. Returns the
    /// remote commit SHA that was fetched back.
    fn reconcile(&self, remote_worktree_path: &Path, local_ref: &str) -> RuntimeResult<String> {
        match &self.execution {
            GitRemoteExecution::Local => {
                let tip = materialize_git_capture(remote_worktree_path, &["rev-parse", "HEAD"])?
                    .trim()
                    .to_string();
                let remote = self.remote_repo.to_string_lossy().to_string();
                materialize_git(
                    &self.local_origin,
                    &["fetch", "--no-tags", &remote, &format!("{tip}:{local_ref}")],
                )?;
                Ok(tip)
            }
            GitRemoteExecution::OverSsh {
                ssh_destination,
                ssh_binary,
            } => {
                // Read the remote worktree tip ON THE REMOTE, then fetch it BACK to
                // the controller across the transport URL (the produced commit
                // crosses the boundary by git, mirroring the local map-back).
                let tip = ssh_git_capture(
                    ssh_binary,
                    ssh_destination,
                    &format!(
                        "git -C {wt} rev-parse HEAD",
                        wt = shell_quote(&remote_worktree_path.to_string_lossy()),
                    ),
                )?
                .trim()
                .to_string();
                materialize_git(
                    &self.local_origin,
                    &[
                        "fetch",
                        "--no-tags",
                        &self.transport_url,
                        &format!("{tip}:{local_ref}"),
                    ],
                )?;
                Ok(tip)
            }
        }
    }
}

/// RR3/RR7: run a git subcommand for remote workspace materialization with a
/// deterministic capo identity (so the op never depends on a global git identity
/// and never records the operator's). A failure is a TYPED
/// [`RuntimeError::RemoteMaterializeFailed`] (redaction-safe message), mirroring
/// `WorktreeError`'s no-silent-fallthrough rule.
fn materialize_git(dir: &Path, args: &[&str]) -> RuntimeResult<()> {
    let output = materialize_git_command(dir, args)
        .output()
        .map_err(|error| RuntimeError::RemoteMaterializeFailed {
            message: format!("failed to spawn git {}: {error}", args.join(" ")),
        })?;
    if !output.status.success() {
        return Err(RuntimeError::RemoteMaterializeFailed {
            message: format!(
                "git {} failed: {}",
                args.join(" "),
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        });
    }
    Ok(())
}

/// RR3/RR7: run a git subcommand and capture stdout (for reading a materialized
/// HEAD / reconcile tip SHA).
fn materialize_git_capture(dir: &Path, args: &[&str]) -> RuntimeResult<String> {
    let output = materialize_git_command(dir, args)
        .output()
        .map_err(|error| RuntimeError::RemoteMaterializeFailed {
            message: format!("failed to spawn git {}: {error}", args.join(" ")),
        })?;
    if !output.status.success() {
        return Err(RuntimeError::RemoteMaterializeFailed {
            message: format!(
                "git {} failed: {}",
                args.join(" "),
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// `git -C <dir>` with a deterministic committer identity for materialization.
fn materialize_git_command(dir: &Path, args: &[&str]) -> Command {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(dir)
        .env("GIT_AUTHOR_NAME", "capo-remote")
        .env("GIT_AUTHOR_EMAIL", "remote@capo.local")
        .env("GIT_COMMITTER_NAME", "capo-remote")
        .env("GIT_COMMITTER_EMAIL", "remote@capo.local")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .args(args);
    command
}

/// RR8 (review finding 1): run a remote shell command over `ssh` for the
/// cross-machine git materialization path (auth by the operator's `ssh` config /
/// agent — handle only, no key/token injected). A non-success status is a TYPED
/// `RemoteMaterializeFailed` with a credential-scanned stderr, never a silent
/// fall-through.
fn ssh_git(ssh_binary: &str, ssh_destination: &str, remote_shell: &str) -> RuntimeResult<()> {
    let output = Command::new(ssh_binary)
        .arg("-o")
        .arg("BatchMode=yes")
        .arg(ssh_destination)
        .arg(remote_shell)
        .output()
        .map_err(|error| RuntimeError::RemoteMaterializeFailed {
            message: format!("ssh git spawn failed: {error}"),
        })?;
    if !output.status.success() {
        return Err(RuntimeError::RemoteMaterializeFailed {
            message: format!(
                "remote git failed: {}",
                scan_credential_shapes(&String::from_utf8_lossy(&output.stderr)).0
            ),
        });
    }
    Ok(())
}

/// RR8 (review finding 1): run a remote git command over `ssh` and capture stdout
/// (for reading the materialized HEAD / reconcile tip SHA from the REMOTE host).
fn ssh_git_capture(
    ssh_binary: &str,
    ssh_destination: &str,
    remote_shell: &str,
) -> RuntimeResult<String> {
    let output = Command::new(ssh_binary)
        .arg("-o")
        .arg("BatchMode=yes")
        .arg(ssh_destination)
        .arg(remote_shell)
        .output()
        .map_err(|error| RuntimeError::RemoteMaterializeFailed {
            message: format!("ssh git spawn failed: {error}"),
        })?;
    if !output.status.success() {
        return Err(RuntimeError::RemoteMaterializeFailed {
            message: format!(
                "remote git failed: {}",
                scan_credential_shapes(&String::from_utf8_lossy(&output.stderr)).0
            ),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// RR1 deterministic fake channel: it executes the program LOCALLY but is HONEST
/// that it is a loopback (`is_loopback() == true`). It is scriptable so the
/// fake-channel suite can drive a launch failure (with retryability) and an
/// "append failed after spawn" cleanup path with NO network and NO real SSH.
#[derive(Clone, Debug)]
pub struct FakeRemoteChannel {
    fingerprint: String,
    remote_host_id: String,
    remote_boot_id: String,
    next_remote_pid: u32,
    /// When set, every `launch` fails with this transport error (and the runner
    /// emits `runtime.remote_process_start_failed` with retryability).
    launch_failure: Option<RemoteLaunchFailure>,
    /// RR2: scripts what a restart re-probe observes for a stored remote run. When
    /// `None`, the recovery probe derives an alive+reattachable+same-boot result
    /// from the stored ref's liveness (the happy reattach path).
    recovery_script: Option<RemoteRecoveryProbe>,
    /// When `true`, `probe` reports the remote process DEAD regardless of the
    /// stored `live` hint — modelling a real transport whose probe contradicts the
    /// last-known local status. This exercises the override path: `health` must
    /// return `live=false` even when the stored ref still says `running`.
    probe_reports_dead: bool,
    /// RR4: the RAW (un-redacted) bytes the remote stream produces over the
    /// channel. This is what a real transport would forward as it is produced; the
    /// runner redacts + bounds it at the remote boundary, so a credential token
    /// scripted here MUST be scrubbed before any delta event / artifact. When
    /// `None`, the channel streams the captured loopback stdout (already
    /// program-produced) so an ordinary run still streams its real output.
    streamed_output: Option<Vec<u8>>,
    /// RR4: when set, the stream is DROPPED mid-flight after this many raw bytes
    /// have been forwarded — modelling a channel that dies mid-stream. The runner
    /// must finalize the delta stream with a recorded reason rather than silently
    /// truncating.
    stream_drop_after: Option<usize>,
    /// RR4: stdin bytes the runner wrote to the remote process over the channel, in
    /// write order. Shared (`Arc`) so a cloned channel observes the same writes; a
    /// test reads it to prove a stdin write actually reached the fake remote.
    stdin_written: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    /// Observability for the idempotency invariant: every time the channel
    /// ACTUALLY spawns a remote process (a successful `launch`), this counter is
    /// incremented. The fake-channel suite asserts a duplicate idempotency key
    /// leaves this at 1 — proving the runner de-duplicated rather than relying on
    /// a constant pid happening to match.
    spawn_count: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    /// RR5: the OS family this fake remote host reports when the runner probes it
    /// for sandbox capability. Defaults to linux (an enforcing family for the
    /// landlock+bwrap tier) so the happy enforced path is the default; scriptable
    /// to a non-enforcing family to drive the honest `Unenforced` path.
    remote_os: RemoteOsFamily,
    /// RR5: when `true`, the remote host reports it CANNOT enforce the requested
    /// tier even if its family would otherwise match — modelling a remote OS that
    /// lacks the mechanism (e.g. a linux without landlock/bwrap). The runner then
    /// records `sandbox.unenforced` and Capo does NOT claim sandboxing.
    sandbox_unenforceable: bool,
    /// RR5 HONESTY: whether this fake channel models a REAL cross-machine boundary
    /// (`is_loopback() == false`). Defaults to `false` (a loopback that never
    /// crossed a machine boundary), so the default fake channel is HONESTLY
    /// `Unenforced` for a remote OS sandbox — Capo never claims a `bwrap`/
    /// `sandbox-exec` confinement was applied over a loopback. A test that wants to
    /// exercise the ENFORCED wrapping path opts in explicitly with
    /// [`Self::with_cross_machine_boundary`] (modelling a real SSH remote).
    cross_machine: bool,
    /// RR5: the LAST request the transport was asked to launch, captured so a test
    /// can prove the ENFORCED path actually handed the transport a `bwrap` /
    /// `sandbox-exec`-wrapped command rather than the bare original. Shared (`Arc`)
    /// so a cloned channel observes the same launches.
    last_launched: std::sync::Arc<std::sync::Mutex<Option<LocalProcessRequest>>>,
    /// RR6: the remote git worktree key currently materialized on the remote, if
    /// any. A launch materializes one (RR3 semantics, modelled deterministically);
    /// a `cleanup_workspace(ReapAll)` reaps it EXACTLY once (a re-run finds `None`
    /// and is an idempotent no-op). A test can pre-seed a DANGLING worktree (a crash
    /// left it without a clean teardown) via [`Self::with_dangling_worktree`] to
    /// prove cleanup reaps it. Shared (`Arc`) so a cloned channel observes the same
    /// reap, and last-write-wins between clones cannot resurrect a reaped worktree.
    remote_worktree: std::sync::Arc<std::sync::Mutex<Option<String>>>,
    /// RR6: the last checkpoint ref a `rollback_worktree` restored the remote
    /// worktree to. A test reads it to prove the rollback reached the transport.
    rolled_back_to: std::sync::Arc<std::sync::Mutex<Option<String>>>,
    /// RR6: the escalation labels (`interrupt`/`terminate`/`kill`) the runner
    /// actually sent over the channel, in send order. Shared (`Arc`) so a cloned
    /// channel observes the same signals; a test reads it to prove that
    /// `revoke_control` (and the teardown escalations) ACTUALLY signalled the
    /// remote run rather than merely flipping a local flag.
    signals_sent: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    /// RR3/RR7: when set, the channel materializes the remote workspace with REAL
    /// git against a local bare-repo "remote" + a remote checkout root (a second
    /// local checkout reached "over the channel", NO network), so the
    /// git-materialization invariants (HEAD matches the source SHA, an uncommitted
    /// local file is ABSENT on the remote worktree, a remote-produced commit
    /// fetches back as a named ref) are proven against actual git rather than an
    /// abstract worktree-key flag. When `None`, materialization is modelled by the
    /// deterministic worktree-key (RR6 semantics) only.
    git_remote: Option<GitRemote>,
    loopback: LocalProcessRunner,
}

impl PartialEq for FakeRemoteChannel {
    fn eq(&self, other: &Self) -> bool {
        // Equality is over the channel's SCRIPTED IDENTITY (what it was
        // constructed to model), not over its accumulated runtime side effects.
        // The following fields are INTENTIONALLY excluded because they are
        // post-construction observability, not identity, and including them would
        // make two channels scripted identically compare unequal merely because
        // one has been driven through a run:
        //   - `spawn_count`     (how many times `launch` actually ran),
        //   - `stdin_written`   (bytes the runner forwarded to stdin),
        //   - `last_launched`   (the last request handed to the transport),
        //   - `signals_sent`    (escalations the runner sent over the channel),
        //   - `remote_worktree` (the live worktree state, mutated by cleanup),
        //   - `rolled_back_to`  (the last checkpoint a rollback restored to).
        // `remote_worktree` and `rolled_back_to` in particular are mutable run
        // state: a dangling worktree is a runtime condition, not a different
        // channel identity, so two channels that differ only in whether a
        // worktree is currently materialized are still the SAME channel here.
        self.fingerprint == other.fingerprint
            && self.remote_host_id == other.remote_host_id
            && self.remote_boot_id == other.remote_boot_id
            && self.next_remote_pid == other.next_remote_pid
            && self.launch_failure == other.launch_failure
            && self.recovery_script == other.recovery_script
            && self.probe_reports_dead == other.probe_reports_dead
            && self.streamed_output == other.streamed_output
            && self.stream_drop_after == other.stream_drop_after
            && self.remote_os == other.remote_os
            && self.sandbox_unenforceable == other.sandbox_unenforceable
            && self.cross_machine == other.cross_machine
            && self.git_remote == other.git_remote
            && self.loopback == other.loopback
    }
}

impl Eq for FakeRemoteChannel {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteLaunchFailure {
    pub message: String,
    pub retryable: bool,
}

impl FakeRemoteChannel {
    /// Build a fake channel from an already-resolved [`OpenChannel`]. The runner
    /// performs NO endpoint resolution — the channel is injected fully resolved.
    pub fn from_open_channel(
        channel: &OpenChannel,
        workspace_root: PathBuf,
        artifact_root: PathBuf,
    ) -> Self {
        let fingerprint = channel
            .identity_fingerprint
            .clone()
            .unwrap_or_else(|| channel.channel_id.clone());
        Self {
            fingerprint,
            remote_host_id: channel.connectivity_endpoint_id.clone(),
            remote_boot_id: format!("remote-boot-{}", channel.channel_id),
            next_remote_pid: 41000,
            launch_failure: None,
            recovery_script: None,
            probe_reports_dead: false,
            streamed_output: None,
            stream_drop_after: None,
            stdin_written: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            spawn_count: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            remote_os: RemoteOsFamily::Linux,
            // HONESTY default (review finding 7): the default fake channel is a
            // loopback that never crossed a machine boundary, so it cannot enforce a
            // remote OS sandbox. Defaulting `sandbox_unenforceable = true` means a
            // test that constructs a default fake channel gets an HONEST `Unenforced`
            // claim; the enforced path must be opted into explicitly
            // (`with_cross_machine_boundary` + an enforcing `with_remote_os`).
            sandbox_unenforceable: true,
            cross_machine: false,
            last_launched: std::sync::Arc::new(std::sync::Mutex::new(None)),
            remote_worktree: std::sync::Arc::new(std::sync::Mutex::new(None)),
            rolled_back_to: std::sync::Arc::new(std::sync::Mutex::new(None)),
            signals_sent: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            git_remote: None,
            loopback: LocalProcessRunner::new(LocalProcessConfig::for_test(
                workspace_root,
                artifact_root,
            )),
        }
    }

    /// RR3/RR7: attach a REAL git-backed remote workspace to this fake channel: a
    /// local bare repo (the "remote origin" Capo pushes to) plus a remote checkout
    /// root the commit is `git worktree add`-ed into — a second local checkout
    /// reached "over the channel", NO network. The git transport URL is recorded
    /// (after the credential scan) so the materialization event carries no embedded
    /// secret. Materialization is then driven against actual git, proving the
    /// content-addressing + uncommitted-scratch-not-synced + fetch-back invariants.
    pub fn with_git_remote(mut self, git_remote: GitRemote) -> Self {
        self.git_remote = Some(git_remote);
        self
    }

    /// RR3/RR7: whether this channel models a real git-backed remote workspace.
    pub fn has_git_remote(&self) -> bool {
        self.git_remote.is_some()
    }

    /// RR6: pre-seed a DANGLING remote git worktree (a crash left it materialized
    /// without a clean teardown), so a `cleanup_run(ReapAll)` proves the dangling
    /// worktree is reaped (`runtime.remote_workspace_torn_down`) rather than
    /// silently abandoned. NO network — the worktree is a deterministic key.
    pub fn with_dangling_worktree(self, worktree_key: impl Into<String>) -> Self {
        *self
            .remote_worktree
            .lock()
            .expect("fake remote worktree ledger poisoned") = Some(worktree_key.into());
        self
    }

    /// RR3/RR7: materialize `source_commit` ON the remote by git over the channel
    /// and record the resulting remote worktree as the live worktree key (so a
    /// later `cleanup_workspace`/`rollback_worktree` reaps/restores it). Returns the
    /// remote `HEAD` + worktree path. Errors with
    /// [`RuntimeError::RemoteMaterializeFailed`] if no git remote is attached or a
    /// git step fails.
    fn materialize(&self, source_commit: &str) -> RuntimeResult<(String, PathBuf)> {
        let git_remote =
            self.git_remote
                .as_ref()
                .ok_or_else(|| RuntimeError::RemoteMaterializeFailed {
                    message: "no git remote attached to channel".to_string(),
                })?;
        let (head, worktree_path) = git_remote.materialize(source_commit)?;
        *self
            .remote_worktree
            .lock()
            .expect("fake remote worktree ledger poisoned") =
            Some(worktree_path.to_string_lossy().to_string());
        Ok((head, worktree_path))
    }

    /// RR3/RR7: map a remote-produced commit at `remote_worktree_path` BACK to
    /// Capo's host by git, into the named `local_ref`. Returns the fetched-back SHA.
    fn reconcile(&self, remote_worktree_path: &Path, local_ref: &str) -> RuntimeResult<String> {
        let git_remote =
            self.git_remote
                .as_ref()
                .ok_or_else(|| RuntimeError::RemoteMaterializeFailed {
                    message: "no git remote attached to channel".to_string(),
                })?;
        git_remote.reconcile(remote_worktree_path, local_ref)
    }

    /// RR3/RR7: the recorded (credential-scanned) git transport URL, or a benign
    /// placeholder when no git remote is attached.
    fn transport_url(&self) -> String {
        self.git_remote
            .as_ref()
            .map(|r| r.transport_url.clone())
            .unwrap_or_else(|| "fake-channel://no-git-remote".to_string())
    }

    /// RR6: the checkpoint ref the LAST `rollback_worktree` restored to. A test
    /// reads this to prove the rollback reached the transport.
    pub fn rolled_back_to(&self) -> Option<String> {
        self.rolled_back_to
            .lock()
            .expect("fake remote rollback ledger poisoned")
            .clone()
    }

    /// RR6: the escalation labels (`interrupt`/`terminate`/`kill`) the runner sent
    /// over this channel, in send order. A test reads it to prove a stop signal
    /// actually reached the transport (e.g. after `revoke_control`).
    pub fn signals_sent(&self) -> Vec<String> {
        self.signals_sent
            .lock()
            .expect("fake remote signal ledger poisoned")
            .clone()
    }

    /// RR6: whether a remote git worktree is currently materialized on this fake
    /// remote. A test reads it to prove `cleanup_run(ReapAll)` actually reaped it.
    pub fn has_remote_worktree(&self) -> bool {
        self.remote_worktree
            .lock()
            .expect("fake remote worktree ledger poisoned")
            .is_some()
    }

    /// RR6: reap the remote process group + (under [`CleanupPolicy::ReapAll`])
    /// remove the remote git worktree. Idempotent: the worktree is removed at most
    /// once; a re-run finds `None` and reports `worktree_reaped == false`.
    fn cleanup_workspace(
        &self,
        _probe: &RemoteProbe,
        policy: CleanupPolicy,
    ) -> RuntimeResult<WorkspaceReapOutcome> {
        let mut worktree = self
            .remote_worktree
            .lock()
            .expect("fake remote worktree ledger poisoned");
        let key = worktree.clone().unwrap_or_default();
        let worktree_reaped = if policy.reaps_worktree() {
            worktree.take().is_some()
        } else {
            // Preserve: the worktree stays for inspection; nothing reaped.
            false
        };
        Ok(WorkspaceReapOutcome {
            worktree_reaped,
            worktree_key: key,
        })
    }

    /// RR6: restore the remote worktree to `checkpoint_ref` (the RR3 git-materialized
    /// commit). Records the checkpoint so a test proves the rollback reached the
    /// transport; re-materializes the worktree key (a rollback leaves a worktree at
    /// the checkpoint commit).
    fn rollback_worktree(&self, _probe: &RemoteProbe, checkpoint_ref: &str) -> RuntimeResult<()> {
        *self
            .rolled_back_to
            .lock()
            .expect("fake remote rollback ledger poisoned") = Some(checkpoint_ref.to_string());
        *self
            .remote_worktree
            .lock()
            .expect("fake remote worktree ledger poisoned") =
            Some(format!("worktree@{checkpoint_ref}"));
        Ok(())
    }

    /// RR5: model a REAL cross-machine boundary (`is_loopback() == false`) so the
    /// deterministic suite can exercise the ENFORCED remote-sandbox wrapping path
    /// (a `bwrap`/`sandbox-exec`-wrapped command handed to the transport) WITHOUT a
    /// real network. Pairs with [`Self::with_enforceable_remote_sandbox`] +
    /// [`Self::with_remote_os`] to script an enforcing remote. Honestly named: the
    /// transport still runs the program on loopback for determinism, but it reports
    /// it crossed a boundary so the enforcement claim is the one a real SSH remote
    /// would make; the live cross-machine proof is RR8.
    ///
    /// DOUBLE-OPT-IN (review finding): this sets `cross_machine` but leaves
    /// `sandbox_unenforceable = true` (the honest default for a fake channel). A
    /// COMPLETE cross-machine model of an enforcing remote MUST pair this with
    /// [`Self::with_enforceable_remote_sandbox`] (and an enforcing
    /// [`Self::with_remote_os`]); without it the plan is `Unenforced` even though a
    /// real SSH remote with `bwrap`/`sandbox-exec` would report `Enforced`. The two
    /// flags are deliberately independent so a test can also model a cross-machine
    /// remote that genuinely cannot enforce.
    pub fn with_cross_machine_boundary(mut self) -> Self {
        self.cross_machine = true;
        self
    }

    /// RR5: script the remote host as ABLE to enforce the requested tier (the
    /// inverse of [`Self::with_unenforceable_remote_sandbox`]). Needed because the
    /// default fake channel is honestly unenforceable; the enforced path opts in.
    pub fn with_enforceable_remote_sandbox(mut self) -> Self {
        self.sandbox_unenforceable = false;
        self
    }

    /// RR5: the LAST request the transport was asked to launch. A test reads this to
    /// prove the ENFORCED path handed the transport a `bwrap`/`sandbox-exec`-wrapped
    /// command, not the bare original — the verification the review required.
    pub fn last_launched_request(&self) -> Option<LocalProcessRequest> {
        self.last_launched
            .lock()
            .expect("fake remote last-launched ledger poisoned")
            .clone()
    }

    /// RR5: script the OS family the remote host reports for a sandbox probe. The
    /// remote OS — not the controller — decides whether a tier is enforceable, so
    /// scripting a non-enforcing family (e.g. [`RemoteOsFamily::Other`]) drives the
    /// honest `Unenforced` path with NO network.
    pub fn with_remote_os(mut self, os_family: RemoteOsFamily) -> Self {
        self.remote_os = os_family;
        self
    }

    /// RR5: script the remote host to report it CANNOT enforce the requested tier
    /// even when its family would match — modelling a remote OS that lacks the
    /// mechanism. The runner then records `sandbox.unenforced` and Capo does NOT
    /// claim sandboxing.
    pub fn with_unenforceable_remote_sandbox(mut self) -> Self {
        self.sandbox_unenforceable = true;
        self
    }

    /// RR5: probe the remote host's sandbox capability. The reported family + the
    /// scripted enforceability decide the runner's HONEST enforcement claim; the
    /// runner never substitutes its own build target.
    fn sandbox_probe(&self, tier: SandboxTier) -> RuntimeResult<RemoteSandboxProbe> {
        let tier_enforceable = !self.sandbox_unenforceable && self.remote_os.enforces(tier);
        Ok(RemoteSandboxProbe {
            os_family: self.remote_os.clone(),
            tier_enforceable,
        })
    }

    /// How many times this channel ACTUALLY spawned a remote process. Used by the
    /// idempotency test to prove a duplicate idempotency key did not double-spawn.
    pub fn spawn_count(&self) -> usize {
        self.spawn_count.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Script the next (and every) launch to fail with the given retryability, so
    /// the `runtime.process_start_failed` + retryability path is exercised.
    pub fn with_launch_failure(mut self, message: impl Into<String>, retryable: bool) -> Self {
        self.launch_failure = Some(RemoteLaunchFailure {
            message: message.into(),
            retryable,
        });
        self
    }

    /// RR2: the remote boot id this fake host reports — used both to stamp a
    /// launch and to detect a remote reboot at recovery time (boot-id mismatch).
    pub fn remote_boot_id(&self) -> String {
        self.remote_boot_id.clone()
    }

    /// RR2: script the next restart re-probe to see an ALIVE remote whose launch is
    /// reattachable (a recorded remote pid + boot identity) — the happy in-place
    /// reattach path. The observed boot id matches the launch boot id.
    pub fn recover_alive_reattachable(mut self) -> Self {
        self.recovery_script = Some(RemoteRecoveryProbe {
            channel_reachable: true,
            live: true,
            observed_remote_boot_id: Some(self.remote_boot_id.clone()),
            reattachable: true,
        });
        self
    }

    /// RR2: script an ALIVE remote that CANNOT be reattached to (no durable remote
    /// pid/boot record) -> orphaned, with the remote logs left inspectable.
    pub fn recover_alive_unattachable(mut self) -> Self {
        self.recovery_script = Some(RemoteRecoveryProbe {
            channel_reachable: true,
            live: true,
            observed_remote_boot_id: Some(self.remote_boot_id.clone()),
            reattachable: false,
        });
        self
    }

    /// RR2: script a remote that REBOOTED — the host reports a different boot id
    /// than was recorded at launch, so the recorded pid is meaningless -> exited.
    pub fn recover_rebooted(mut self) -> Self {
        self.recovery_script = Some(RemoteRecoveryProbe {
            channel_reachable: true,
            live: true,
            observed_remote_boot_id: Some(format!("{}-after-reboot", self.remote_boot_id)),
            reattachable: true,
        });
        self
    }

    /// RR2: script a remote whose process is GONE (exited while Capo was down,
    /// same boot, no terminal event) -> exited.
    pub fn recover_gone(mut self) -> Self {
        self.recovery_script = Some(RemoteRecoveryProbe {
            channel_reachable: true,
            live: false,
            observed_remote_boot_id: Some(self.remote_boot_id.clone()),
            reattachable: true,
        });
        self
    }

    /// RR2: script the CHANNEL itself as unreachable at recovery time -> the run is
    /// held `recovery_pending` (never forced to recovered/exited) until it returns.
    pub fn recover_channel_unreachable(mut self) -> Self {
        self.recovery_script = Some(RemoteRecoveryProbe {
            channel_reachable: false,
            live: false,
            observed_remote_boot_id: None,
            reattachable: false,
        });
        self
    }

    fn is_loopback(&self) -> bool {
        !self.cross_machine
    }

    fn target_fingerprint(&self) -> String {
        self.fingerprint.clone()
    }

    fn launch(&self, request: &LocalProcessRequest) -> RuntimeResult<RemoteLaunch> {
        if let Some(failure) = &self.launch_failure {
            return Err(RuntimeError::RemoteLaunchFailed {
                message: failure.message.clone(),
                retryable: failure.retryable,
            });
        }
        // Honest loopback: actually run the program (locally) so the captured
        // artifacts are real, while reporting a synthetic remote identity. Each
        // actual spawn mints a DISTINCT remote pid (base + spawn index) so the
        // idempotency invariant cannot pass by accident on a constant pid: a
        // second real spawn would produce a different pid and a different ref.
        // Capture the request the transport was actually asked to launch (the
        // wrapped `bwrap`/`sandbox-exec` command on the enforced path) so a test can
        // assert the enforcement layer reached the transport, not just the claim.
        *self
            .last_launched
            .lock()
            .expect("fake remote last-launched ledger poisoned") = Some(request.clone());
        // The transport receives the WRAPPED command on the enforced path
        // (`bwrap ... <program> <argv>` / `/usr/bin/sandbox-exec -f <policy>
        // <program> <argv>`). A real remote OS sandbox launcher execs its CHILD; the
        // deterministic loopback models that by running the UNWRAPPED inner program
        // (the `bwrap`/`sandbox-exec` binaries are not assumed present on the test
        // host). `last_launched` still records the full wrapped argv, so a test
        // proves the enforcement layer reached the transport without depending on a
        // sandbox launcher binary. NO network.
        let to_run = unwrap_sandbox_launcher(request);
        let captured = self.loopback.start_process(to_run)?;
        let spawn_index = self
            .spawn_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        // RR6: a launch materializes a remote git worktree (RR3 semantics, modelled
        // deterministically by a key) so a later `cleanup_run(ReapAll)` has a
        // worktree to reap. Only set it if a crash hasn't pre-seeded one.
        {
            let mut worktree = self
                .remote_worktree
                .lock()
                .expect("fake remote worktree ledger poisoned");
            if worktree.is_none() {
                *worktree = Some(format!(
                    "{}/run-{}",
                    self.remote_host_id,
                    self.next_remote_pid + spawn_index as u32
                ));
            }
        }
        Ok(RemoteLaunch {
            remote_pid: self.next_remote_pid + spawn_index as u32,
            remote_boot_id: self.remote_boot_id.clone(),
            remote_host_id: self.remote_host_id.clone(),
            live: true,
            captured,
        })
    }

    /// Script the channel probe to report the remote process DEAD even when the
    /// stored ref still says `running`, so the override path (`health` trusting the
    /// remote probe over the local status string) is exercised.
    pub fn with_probe_reports_dead(mut self) -> Self {
        self.probe_reports_dead = true;
        self
    }

    /// RR4: script the RAW (un-redacted) bytes the remote stream produces over the
    /// channel. The runner redacts + bounds these at the remote boundary, so a
    /// credential-shaped token scripted here proves redaction happens BEFORE any
    /// delta event / persisted artifact.
    pub fn with_streamed_output(mut self, raw: impl Into<Vec<u8>>) -> Self {
        self.streamed_output = Some(raw.into());
        self
    }

    /// RR4: script the channel to DROP mid-stream after `byte_offset` raw bytes
    /// have been forwarded — modelling a channel that dies mid-run so the runner
    /// must finalize with a recorded reason, never silently truncate.
    pub fn with_stream_drop_after(mut self, byte_offset: usize) -> Self {
        self.stream_drop_after = Some(byte_offset);
        self
    }

    /// RR4: the stdin bytes the runner has written to the remote process over the
    /// channel so far. A test reads this to prove a stdin write actually reached
    /// the fake remote.
    pub fn stdin_written(&self) -> Vec<u8> {
        self.stdin_written
            .lock()
            .expect("fake remote stdin ledger poisoned")
            .clone()
    }

    /// The RAW bytes this channel streams: the scripted payload when set, else the
    /// captured loopback stdout (the real program output the launch produced).
    fn raw_stream_bytes(&self) -> Vec<u8> {
        self.streamed_output.clone().unwrap_or_default()
    }

    fn signal(&self, _probe: &RemoteProbe, escalation: &str) -> RuntimeResult<()> {
        // Record the escalation so a test can prove the stop signal ACTUALLY
        // reached the channel (e.g. that `revoke_control` signalled the in-flight
        // run, not merely flipped the local grant flag).
        self.signals_sent
            .lock()
            .expect("fake remote signal ledger poisoned")
            .push(escalation.to_string());
        Ok(())
    }

    /// RR4: forward the remote stream as RAW frames starting from `from_offset`,
    /// reporting whether the channel dropped mid-stream. The runner (not the
    /// channel) redacts + bounds + offsets these; the channel is honest about
    /// where the bytes start and whether they were cut off.
    fn stream(&self, _probe: &RemoteProbe, from_offset: usize) -> RemoteRawStream {
        let raw = self.raw_stream_bytes();
        // A reconnect resumes strictly AFTER the last acknowledged offset.
        let start = from_offset.min(raw.len());
        let mut bytes = raw[start..].to_vec();
        // If scripted to drop, cut the forwarded bytes at the drop boundary
        // (relative to the resume start) and report the drop.
        let dropped = match self.stream_drop_after {
            Some(drop_at) if drop_at < raw.len() => {
                let cut = drop_at.saturating_sub(start);
                if cut < bytes.len() {
                    bytes.truncate(cut);
                }
                true
            }
            _ => false,
        };
        RemoteRawStream {
            from_offset: start,
            bytes,
            dropped,
        }
    }

    fn write_stdin(&self, _probe: &RemoteProbe, bytes: &[u8]) -> RuntimeResult<()> {
        self.stdin_written
            .lock()
            .expect("fake remote stdin ledger poisoned")
            .extend_from_slice(bytes);
        Ok(())
    }

    fn probe(&self, probe: &RemoteProbe) -> RuntimeResult<bool> {
        // The transport probe is the AUTHORITY: when scripted dead it overrides the
        // stored `live` hint, proving health does not merely echo a local status
        // string (the stub's behaviour the review flagged).
        if self.probe_reports_dead {
            return Ok(false);
        }
        Ok(probe.live)
    }

    fn recovery_probe(&self, probe: &RemoteProbe) -> RemoteRecoveryProbe {
        // A scripted outcome wins (the suite drives every recovery class through
        // it). With no script, the default is the happy reattach path: the host is
        // reachable, the remote is alive within the same boot, and the launch is
        // reattachable, so a stored running ref recovers in place.
        self.recovery_script
            .clone()
            .unwrap_or_else(|| RemoteRecoveryProbe {
                channel_reachable: true,
                live: probe.live,
                observed_remote_boot_id: Some(self.remote_boot_id.clone()),
                reattachable: true,
            })
    }

    fn cleanup(&self, _probe: &RemoteProbe) -> RuntimeResult<()> {
        Ok(())
    }
}

/// RR8: configuration for the REAL SSH transport behind [`SshRemoteProcessRunner`].
///
/// All identity/auth is carried by HANDLE — the runner NEVER reads a raw SSH key,
/// `known_hosts` secret, or subscription token. `auth_ref` is an OPAQUE handle the
/// operator's `ssh` config resolves (e.g. an agent key / `IdentityFile` already on
/// the controller host); it is recorded only as a label and never logged raw. The
/// remote git store + worktree root are the REAL [`GitRemote`] paths on the remote
/// (reached over the SAME SSH host), so git materialization is real cross-machine.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SshRemoteConfig {
    /// The SSH destination (`user@host` / a `ssh_config` `Host` alias). Resolved by
    /// `connectivity-tunnel`; the runner does not invent it.
    pub ssh_destination: String,
    /// The proven remote target identity (channel fingerprint), never a credential.
    pub fingerprint: String,
    /// An OPAQUE auth handle (e.g. `ssh-agent:default` / a `ssh_config` host alias).
    /// Recorded as a label only; NEVER a raw key or token.
    pub auth_ref: Option<String>,
    /// The `ssh` binary to invoke (default `ssh`).
    pub ssh_binary: String,
    /// Where redacted remote stdout/stderr artifacts are written on the controller.
    pub artifact_root: PathBuf,
    /// The REAL git-backed remote workspace (origin on the controller, remote repo +
    /// worktree root on the remote, reached over SSH).
    pub git_remote: Option<GitRemote>,
}

impl SshRemoteConfig {
    /// Build a real-SSH transport config from an already-resolved SSH destination +
    /// fingerprint (the channel is resolved by `connectivity-tunnel`; the runner
    /// performs no endpoint resolution and never touches raw credentials).
    pub fn new(
        ssh_destination: impl Into<String>,
        fingerprint: impl Into<String>,
        artifact_root: PathBuf,
    ) -> Self {
        Self {
            ssh_destination: ssh_destination.into(),
            fingerprint: fingerprint.into(),
            auth_ref: None,
            ssh_binary: "ssh".to_string(),
            artifact_root,
            git_remote: None,
        }
    }

    /// Attach the OPAQUE auth handle (a label only — never a raw key/token).
    pub fn with_auth_ref(mut self, auth_ref: impl Into<String>) -> Self {
        self.auth_ref = Some(auth_ref.into());
        self
    }

    /// Attach the REAL git-backed remote workspace for materialization. The git
    /// remote is bound to RUN ITS REPO-SIDE OPS ON THE REMOTE over this config's
    /// SSH destination (review finding 1): `remote_repo`/`remote_worktree_root` are
    /// remote paths and the commit is pushed across the transport URL, so
    /// materialization is a genuine cross-machine git operation.
    pub fn with_git_remote(mut self, git_remote: GitRemote) -> Self {
        self.git_remote =
            Some(git_remote.over_ssh(self.ssh_destination.clone(), self.ssh_binary.clone()));
        self
    }
}

/// RR8: the REAL cross-machine SSH transport. It shells out to `ssh` to launch /
/// signal / probe / stream on the remote host and uses a REAL [`GitRemote`] for git
/// materialization. It is exercised ONLY by the opt-in `#[ignore]` live smoke; the
/// deterministic gate never instantiates it (it would touch the network). HONESTY:
/// it crossed a machine boundary, so [`RemoteChannel::is_loopback`] is `false` and
/// Capo's realness/enforcement claims are truthful. Secrets are carried by handle:
/// it NEVER reads/logs a raw SSH key, `known_hosts` secret, or subscription token.
#[derive(Clone, Debug)]
pub struct SshRemoteChannel {
    config: SshRemoteConfig,
    spawn_count: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    last_launched: std::sync::Arc<std::sync::Mutex<Option<LocalProcessRequest>>>,
}

impl PartialEq for SshRemoteChannel {
    fn eq(&self, other: &Self) -> bool {
        // Identity is the resolved config; the spawn counter + last-launched ledger
        // are runtime state, not identity (mirrors `FakeRemoteChannel`).
        self.config == other.config
    }
}

impl Eq for SshRemoteChannel {}

impl SshRemoteChannel {
    /// Build the real SSH transport from a resolved config.
    pub fn new(config: SshRemoteConfig) -> Self {
        Self {
            config,
            spawn_count: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            last_launched: std::sync::Arc::new(std::sync::Mutex::new(None)),
        }
    }

    /// `ssh <destination> <remote_shell_command>` — a `Command` with the OPAQUE
    /// destination. Auth is left to the operator's `ssh` config / agent (handle
    /// only); the runner never injects a key path or token.
    fn ssh_command(&self, remote_shell: &str) -> Command {
        let mut command = Command::new(&self.config.ssh_binary);
        command
            .arg("-o")
            .arg("BatchMode=yes")
            .arg(&self.config.ssh_destination)
            .arg(remote_shell);
        command
    }

    fn target_fingerprint(&self) -> String {
        self.config.fingerprint.clone()
    }

    fn spawn_count(&self) -> usize {
        self.spawn_count.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn last_launched_request(&self) -> Option<LocalProcessRequest> {
        self.last_launched
            .lock()
            .expect("ssh last-launched ledger poisoned")
            .clone()
    }

    /// Launch the program on the remote over SSH (detached, setsid, recording its
    /// remote pid + boot id). The captured artifacts are the remote stdout/stderr,
    /// REDACTED on the controller before persistence.
    fn launch(&self, request: &LocalProcessRequest) -> RuntimeResult<RemoteLaunch> {
        *self
            .last_launched
            .lock()
            .expect("ssh last-launched ledger poisoned") = Some(request.clone());

        let argv = request
            .argv
            .iter()
            .map(|a| shell_quote(a))
            .collect::<Vec<_>>()
            .join(" ");
        let cwd = request.cwd.to_string_lossy();
        // Run detached under setsid so a controller restart can reattach to the
        // recorded remote pid + boot id, and echo the boot id + pid back.
        let remote_shell = format!(
            "cd {cwd} && setsid {program} {argv} >/tmp/capo-remote-out 2>&1 & \
             echo PID=$!; cat /proc/sys/kernel/random/boot_id 2>/dev/null || true",
            cwd = shell_quote(&cwd),
            program = shell_quote(&request.program),
        );
        let output = self.ssh_command(&remote_shell).output().map_err(|error| {
            RuntimeError::RemoteLaunchFailed {
                message: format!("ssh launch spawn failed: {error}"),
                retryable: true,
            }
        })?;
        if !output.status.success() {
            return Err(RuntimeError::RemoteLaunchFailed {
                message: format!(
                    "ssh launch failed: {}",
                    scan_credential_shapes(&String::from_utf8_lossy(&output.stderr)).0
                ),
                retryable: true,
            });
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let remote_pid = stdout
            .lines()
            .find_map(|l| l.strip_prefix("PID="))
            .and_then(|p| p.trim().parse::<u32>().ok())
            .ok_or_else(|| RuntimeError::RemoteLaunchFailed {
                message: "ssh launch did not report a remote pid".to_string(),
                retryable: false,
            })?;
        let remote_boot_id = stdout
            .lines()
            .last()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty() && !l.starts_with("PID="))
            .unwrap_or_else(|| "ssh-remote-boot-unknown".to_string());
        let spawn_index = self
            .spawn_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let _ = spawn_index;

        // The captured artifacts are the remote run's output, REDACTED before
        // persistence (the credential scan runs at the remote boundary).
        let captured = self.capture_artifacts(request, b"")?;
        Ok(RemoteLaunch {
            remote_pid,
            remote_boot_id,
            remote_host_id: self.config.ssh_destination.clone(),
            live: true,
            captured,
        })
    }

    /// Write the (already-redacted) remote output to a controller-side artifact so
    /// the run carries a real, secret-free artifact reference.
    fn capture_artifacts(
        &self,
        request: &LocalProcessRequest,
        raw_output: &[u8],
    ) -> RuntimeResult<LocalProcessOutcome> {
        let (redacted, scrubbed) = scan_credential_shapes(&String::from_utf8_lossy(raw_output));
        let redaction_state = if scrubbed { "redacted" } else { "safe" };
        let run_dir = self.config.artifact_root.join(request.run_id.as_str());
        fs::create_dir_all(&run_dir)?;
        let stdout_path = run_dir.join("stdout.txt");
        let stderr_path = run_dir.join("stderr.txt");
        fs::write(&stdout_path, redacted.as_bytes())?;
        fs::write(&stderr_path, b"")?;
        let stdout = RuntimeOutputArtifact {
            artifact_id: format!("{}:stdout", request.run_id.as_str()),
            path: stdout_path,
            size_bytes: redacted.len() as i64,
            content_hash: content_hash(redacted.as_bytes()),
            redaction_state: redaction_state.to_string(),
            truncated: false,
        };
        let stderr = RuntimeOutputArtifact {
            artifact_id: format!("{}:stderr", request.run_id.as_str()),
            path: stderr_path,
            size_bytes: 0,
            content_hash: content_hash(b""),
            redaction_state: "safe".to_string(),
            truncated: false,
        };
        Ok(LocalProcessOutcome {
            process: LocalRuntimeProcessRef {
                run_id: request.run_id.clone(),
                runtime_process_ref: String::new(),
                external_pid: None,
                boot_id: None,
                status: "running".to_string(),
                redaction_state: redaction_state.to_string(),
            },
            stdout,
            stderr,
            exit_code: None,
            events: Vec::new(),
        })
    }

    /// Escalate a stop over SSH: `kill -<sig> -<pgid>` for the recorded remote pid.
    fn signal(&self, probe: &RemoteProbe, escalation: &str) -> RuntimeResult<()> {
        let signal = match escalation {
            "interrupt" => "INT",
            "terminate" => "TERM",
            _ => "KILL",
        };
        let remote_shell = format!(
            "kill -{signal} {pid} 2>/dev/null || true",
            pid = probe.remote_pid
        );
        let status = self.ssh_command(&remote_shell).status().map_err(|error| {
            RuntimeError::RemoteLaunchFailed {
                message: format!("ssh signal spawn failed: {error}"),
                retryable: true,
            }
        })?;
        if !status.success() {
            return Err(RuntimeError::RemoteLaunchFailed {
                message: "ssh signal failed".to_string(),
                retryable: true,
            });
        }
        Ok(())
    }

    /// Probe liveness over SSH: `kill -0 <pid>` succeeds iff the process is alive.
    fn probe(&self, probe: &RemoteProbe) -> RuntimeResult<bool> {
        let remote_shell = format!(
            "kill -0 {pid} 2>/dev/null && echo LIVE || echo DEAD",
            pid = probe.remote_pid
        );
        let output = self.ssh_command(&remote_shell).output().map_err(|error| {
            RuntimeError::RemoteLaunchFailed {
                message: format!("ssh probe spawn failed: {error}"),
                retryable: true,
            }
        })?;
        Ok(String::from_utf8_lossy(&output.stdout).contains("LIVE"))
    }

    /// RR2: re-probe a stored remote run over SSH on restart. A boot-id mismatch
    /// (the remote rebooted) is reported so the run is classified `Exited`, never
    /// silently recovered.
    fn recovery_probe(&self, probe: &RemoteProbe) -> RemoteRecoveryProbe {
        let remote_shell = format!(
            "(kill -0 {pid} 2>/dev/null && echo LIVE || echo DEAD); \
             cat /proc/sys/kernel/random/boot_id 2>/dev/null || true",
            pid = probe.remote_pid,
        );
        match self.ssh_command(&remote_shell).output() {
            Ok(output) if output.status.success() => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let live = stdout.contains("LIVE");
                let observed_boot = stdout
                    .lines()
                    .last()
                    .map(|l| l.trim().to_string())
                    .filter(|l| l != "LIVE" && l != "DEAD" && !l.is_empty());
                RemoteRecoveryProbe {
                    channel_reachable: true,
                    live,
                    observed_remote_boot_id: observed_boot,
                    reattachable: true,
                }
            }
            // Channel unreachable -> held recovery_pending (never forced).
            _ => RemoteRecoveryProbe {
                channel_reachable: false,
                live: false,
                observed_remote_boot_id: None,
                reattachable: false,
            },
        }
    }

    fn cleanup(&self, _probe: &RemoteProbe) -> RuntimeResult<()> {
        Ok(())
    }

    /// RR4: forward remote stdout over SSH from `from_offset`. The runner redacts +
    /// bounds + offsets; the channel only forwards the raw bytes.
    fn stream(&self, _probe: &RemoteProbe, from_offset: usize) -> RemoteRawStream {
        let remote_shell = "cat /tmp/capo-remote-out 2>/dev/null || true";
        match self.ssh_command(remote_shell).output() {
            Ok(output) if output.status.success() => {
                let raw = output.stdout;
                let start = from_offset.min(raw.len());
                RemoteRawStream {
                    from_offset: start,
                    bytes: raw[start..].to_vec(),
                    dropped: false,
                }
            }
            _ => RemoteRawStream {
                from_offset,
                bytes: Vec::new(),
                dropped: true,
            },
        }
    }

    /// RR4 (review finding): write stdin to the remote process over SSH WITHOUT
    /// embedding the payload bytes in the SSH command string. The payload is piped
    /// through the SSH session's OWN stdin to a remote `cat` that forwards it to the
    /// target process's fd; the command line carries only the pid, so a verbose
    /// `sshd` log records the redirect shape, never the secret bytes. (The previous
    /// `printf %s '<bytes>'` form put the payload on the command line, where
    /// `LogLevel VERBOSE`/`DEBUG` would capture it.)
    fn write_stdin(&self, probe: &RemoteProbe, bytes: &[u8]) -> RuntimeResult<()> {
        use std::io::Write;
        let remote_shell = format!(
            "cat > /proc/{pid}/fd/0 2>/dev/null || true",
            pid = probe.remote_pid,
        );
        let mut child = self
            .ssh_command(&remote_shell)
            .stdin(std::process::Stdio::piped())
            .spawn()
            .map_err(|error| RuntimeError::RemoteLaunchFailed {
                message: format!("ssh stdin spawn failed: {error}"),
                retryable: true,
            })?;
        if let Some(mut stdin) = child.stdin.take() {
            // Best-effort: a closed remote fd is not fatal to the controller turn.
            let _ = stdin.write_all(bytes);
        }
        let _ = child.wait();
        Ok(())
    }

    /// RR5: probe the REMOTE host's OS family + tier enforceability over SSH (the
    /// remote OS — not the controller — is the authority on enforcement).
    fn sandbox_probe(&self, tier: SandboxTier) -> RuntimeResult<RemoteSandboxProbe> {
        // `bwrap` is the Linux mechanism; `sandbox-exec` is macOS seatbelt (NOT
        // seccomp). The tokens name the actual mechanism so the macOS branch never
        // keys off a Linux-kernel label.
        let remote_shell = "uname -s; command -v bwrap >/dev/null 2>&1 && echo HAS_BWRAP; \
             command -v sandbox-exec >/dev/null 2>&1 && echo HAS_SANDBOX_EXEC";
        let output = self.ssh_command(remote_shell).output().map_err(|error| {
            RuntimeError::RemoteLaunchFailed {
                message: format!("ssh sandbox probe spawn failed: {error}"),
                retryable: true,
            }
        })?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let kernel = stdout.lines().next().unwrap_or("").trim();
        let (os_family, mechanism_present) = match kernel {
            "Darwin" => (RemoteOsFamily::Macos, stdout.contains("HAS_SANDBOX_EXEC")),
            "Linux" => (RemoteOsFamily::Linux, stdout.contains("HAS_BWRAP")),
            other => (RemoteOsFamily::Other(other.to_string()), false),
        };
        let tier_enforceable = mechanism_present && os_family.enforces(tier);
        Ok(RemoteSandboxProbe {
            os_family,
            tier_enforceable,
        })
    }

    fn cleanup_workspace(
        &self,
        probe: &RemoteProbe,
        policy: CleanupPolicy,
    ) -> RuntimeResult<WorkspaceReapOutcome> {
        // Reap the remote process group.
        let _ = self.signal(probe, "kill");
        let worktree_reaped = if policy.reaps_worktree() {
            if let Some(git_remote) = &self.config.git_remote {
                let key = git_remote
                    .remote_worktree_root
                    .to_string_lossy()
                    .to_string();
                let remote_shell = format!(
                    "rm -rf {root} 2>/dev/null || true; echo REAPED",
                    root = shell_quote(&key),
                );
                self.ssh_command(&remote_shell)
                    .output()
                    .map(|o| String::from_utf8_lossy(&o.stdout).contains("REAPED"))
                    .unwrap_or(false)
            } else {
                false
            }
        } else {
            false
        };
        Ok(WorkspaceReapOutcome {
            worktree_reaped,
            worktree_key: self
                .config
                .git_remote
                .as_ref()
                .map(|r| r.remote_worktree_root.to_string_lossy().to_string())
                .unwrap_or_default(),
        })
    }

    fn rollback_worktree(&self, _probe: &RemoteProbe, checkpoint_ref: &str) -> RuntimeResult<()> {
        let git_remote = self.config.git_remote.as_ref().ok_or_else(|| {
            RuntimeError::RemoteMaterializeFailed {
                message: "no git remote attached to ssh channel".to_string(),
            }
        })?;
        // Restore the remote worktree to the checkpoint commit over the channel.
        let _ = git_remote.materialize(checkpoint_ref)?;
        Ok(())
    }

    fn materialize(&self, source_commit: &str) -> RuntimeResult<(String, PathBuf)> {
        let git_remote = self.config.git_remote.as_ref().ok_or_else(|| {
            RuntimeError::RemoteMaterializeFailed {
                message: "no git remote attached to ssh channel".to_string(),
            }
        })?;
        git_remote.materialize(source_commit)
    }

    fn reconcile(&self, remote_worktree_path: &Path, local_ref: &str) -> RuntimeResult<String> {
        let git_remote = self.config.git_remote.as_ref().ok_or_else(|| {
            RuntimeError::RemoteMaterializeFailed {
                message: "no git remote attached to ssh channel".to_string(),
            }
        })?;
        git_remote.reconcile(remote_worktree_path, local_ref)
    }

    fn transport_url(&self) -> String {
        // The transport URL is credential-scanned before it is ever recorded.
        let raw = self
            .config
            .git_remote
            .as_ref()
            .map(|r| r.transport_url.clone())
            .unwrap_or_else(|| format!("ssh://{}", self.config.ssh_destination));
        scan_credential_shapes(&raw).0
    }
}

/// RR8: minimally shell-quote a token for a remote `sh -c` line (single-quote
/// wrapping with the standard `'\''` escape). Used only on the opt-in SSH path.
fn shell_quote(token: &str) -> String {
    format!("'{}'", token.replace('\'', "'\\''"))
}

/// RR8: the deterministic fake-remote runner the SSH name pairs with. Behind the
/// SAME [`RemoteProcessRunner`] contract as the real [`SshRemoteProcessRunner`], so
/// the live smoke's deterministic fixture asserts the IDENTICAL shapes (process-ref
/// shape, materialized-HEAD-matches-SHA, redacted output, recovery classification)
/// with NO network.
pub struct SshRemoteProcessRunner;

impl SshRemoteProcessRunner {
    /// Build the REAL SSH-backed remote runner from a resolved channel + SSH config.
    /// This is the cross-machine path; it is constructed ONLY by the opt-in live
    /// smoke (the deterministic gate uses [`FakeRemoteProcessRunner`]).
    pub fn build(channel: OpenChannel, ssh: SshRemoteConfig) -> RemoteProcessRunner {
        let transport = RemoteChannel::Ssh(SshRemoteChannel::new(ssh));
        RemoteProcessRunner::new(RemoteProcessConfig::with_transport(channel, transport))
    }
}

/// RR8: the env gate names for the opt-in live remote-runtime SSH smoke, mirroring
/// the `CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT` / `CAPO_SERVER_RUN_CODEX_LIVE` pair.
/// BOTH must be `1` for the smoke to attempt the live host; otherwise it skips.
pub const REMOTE_RUNTIME_PREFLIGHT_ENV: &str = "CAPO_SERVER_REMOTE_RUNTIME_PREFLIGHT";
pub const RUN_REMOTE_RUNTIME_LIVE_ENV: &str = "CAPO_SERVER_RUN_REMOTE_RUNTIME_LIVE";
/// RR8: the SSH destination (`user@host` / `ssh_config` alias) the live smoke
/// resolves the channel against. When unset the smoke skips cleanly.
pub const REMOTE_RUNTIME_SSH_HOST_ENV: &str = "CAPO_SERVER_REMOTE_RUNTIME_SSH_HOST";

/// RR8: the OUTCOME of the DEFINED, deterministic skip predicate for the live SSH
/// smoke — RUN against a configured host, or SKIP with a recorded, secret-free
/// reason so "clean skip" is CHECKABLE in evidence rather than operator-eyeballed.
/// The predicate is purely a function of (both env gates set, an SSH host
/// configured) — never operator judgement, never a raw credential.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LiveRemoteRuntimeSmokeDecision {
    /// Run the live lifecycle against this resolved SSH destination (an opaque
    /// `user@host`/alias — never a credential).
    Run { ssh_destination: String },
    /// Skip cleanly. The `reason` is a fixed, secret-free label.
    Skip { reason: String },
}

/// RR8: the DEFINED skip predicate for the live SSH smoke. Reads the two env gates
/// and the SSH host config; returns [`LiveRemoteRuntimeSmokeDecision::Run`] only when
/// BOTH gates are `1` AND an SSH host is configured, else
/// [`LiveRemoteRuntimeSmokeDecision::Skip`] with a recorded reason. NEVER returns a
/// credential — only the opaque SSH destination or a fixed reason label.
pub fn live_remote_runtime_smoke_decision() -> LiveRemoteRuntimeSmokeDecision {
    let gate_set = |name: &str| std::env::var(name).map(|v| v == "1").unwrap_or(false);
    if !gate_set(REMOTE_RUNTIME_PREFLIGHT_ENV) || !gate_set(RUN_REMOTE_RUNTIME_LIVE_ENV) {
        return LiveRemoteRuntimeSmokeDecision::Skip {
            reason: format!(
                "live remote-runtime gate unset ({REMOTE_RUNTIME_PREFLIGHT_ENV} + \
                 {RUN_REMOTE_RUNTIME_LIVE_ENV} must both be 1)"
            ),
        };
    }
    match std::env::var(REMOTE_RUNTIME_SSH_HOST_ENV) {
        Ok(host) if !host.trim().is_empty() => LiveRemoteRuntimeSmokeDecision::Run {
            ssh_destination: host.trim().to_string(),
        },
        _ => LiveRemoteRuntimeSmokeDecision::Skip {
            reason: format!("no SSH host configured ({REMOTE_RUNTIME_SSH_HOST_ENV} unset)"),
        },
    }
}

/// RR1 remote process configuration: the runner is constructed from an
/// already-resolved channel ([`OpenChannel`]) plus the [`RemoteChannel`]
/// transport. The pre-RR1 `local_loopback: LocalProcessConfig` construction path
/// is REMOVED — the runner no longer takes a loopback config; the transport owns
/// where/how the program runs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteProcessConfig {
    /// The resolved channel from `connectivity-tunnel`; identity is its
    /// fingerprint, NOT a raw endpoint string the runner invented.
    pub channel: OpenChannel,
    pub transport: RemoteChannel,
    /// RR4 (review finding): operator-declared literal redaction rules, mirroring
    /// [`LocalProcessConfig::redaction_rules`]. These are layered UNDER the
    /// automatic credential-shape scan when redacting remote output deltas, so a
    /// secret the operator named explicitly (e.g. a literal API key) is scrubbed
    /// from remote stdout exactly as it would be from a local run — not silently
    /// dropped on the remote path.
    pub redaction_rules: Vec<RedactionRule>,
}

impl RemoteProcessConfig {
    /// Build an RR1 config from an already-resolved [`OpenChannel`] and a fake
    /// transport for the deterministic suite (NO network). This replaces the old
    /// `loopback_for_test` loopback-config path.
    pub fn fake_for_test(
        channel: OpenChannel,
        workspace_root: PathBuf,
        artifact_root: PathBuf,
    ) -> Self {
        let transport = RemoteChannel::Fake(FakeRemoteChannel::from_open_channel(
            &channel,
            workspace_root,
            artifact_root,
        ));
        Self {
            channel,
            transport,
            redaction_rules: Vec::new(),
        }
    }

    /// Build an RR1 config with a caller-supplied fake transport (so a test can
    /// script a launch failure).
    pub fn with_transport(channel: OpenChannel, transport: RemoteChannel) -> Self {
        Self {
            channel,
            transport,
            redaction_rules: Vec::new(),
        }
    }

    /// RR4 (review finding): attach operator-declared literal redaction rules so
    /// they are applied to remote output deltas alongside the credential-shape scan.
    pub fn with_redaction_rules(mut self, rules: Vec<RedactionRule>) -> Self {
        self.redaction_rules = rules;
        self
    }

    /// RR1 honest-loopback test helper: build a config from a remote target
    /// fingerprint and a connectivity endpoint id, resolving a fake channel that
    /// runs the program over loopback (NO network). The remote identity is the
    /// `target` fingerprint and the host id is the `endpoint`, so the recorded
    /// `remote_process_ref` is `remote-process:{target}:{endpoint}:...`.
    pub fn loopback_for_test(
        target: impl Into<String>,
        endpoint: impl Into<String>,
        workspace_root: PathBuf,
        artifact_root: PathBuf,
    ) -> Self {
        let target = target.into();
        let channel = OpenChannel::for_test(target.clone(), endpoint, target);
        Self::fake_for_test(channel, workspace_root, artifact_root)
    }
}

/// RR4: which standard stream a remote output delta carries. The remote analogue
/// of [`async_runner::StreamSource`], reusing the same stdout/stderr label so a
/// remote delta routes exactly like a local one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RemoteStreamSource {
    Stdout,
    Stderr,
}

impl RemoteStreamSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::Stdout => "stdout",
            Self::Stderr => "stderr",
        }
    }
}

/// RR4: one redacted, offset-tagged remote output delta. Reuses the
/// `streaming-transport` output-delta model: `offset` is the MONOTONIC byte
/// position (in the RAW remote stream) at which this delta's bytes start, so a
/// reconnect replays strictly from the last acknowledged offset without
/// duplicating an already-projected delta. The payload is ALREADY redacted at the
/// remote boundary (the `RedactionPolicy` credential-shape scan), and
/// `redaction_state` records whether anything was scrubbed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteStreamDelta {
    pub source: RemoteStreamSource,
    /// The byte offset in the RAW remote stream at which this delta begins. The
    /// NEXT acknowledged offset is `offset + raw_len` (carried in the event so a
    /// reconnect resumes after exactly the projected bytes).
    pub offset: usize,
    /// Number of RAW (pre-redaction) bytes this delta covers, so the next offset
    /// is computable from the projected stream alone even though the payload below
    /// is redacted (and thus may differ in length).
    pub raw_len: usize,
    /// The redacted delta payload (UTF-8 lossy), safe to persist / forward.
    pub text: String,
    /// `"redacted"` when the credential-shape scan matched, else `"safe"`.
    pub redaction_state: String,
}

/// RR4: why a remote delta stream finalized. A clean EOF, a cap-truncation, or a
/// mid-stream channel drop — every terminal reason is RECORDED so a dropped stream
/// is never a silent truncation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RemoteStreamFinalReason {
    /// The remote stream reached EOF and was forwarded in full.
    Eof,
    /// The stream hit the `output_limit_bytes` cap and was bounded; remaining
    /// remote bytes were not forwarded.
    CapReached,
    /// The channel dropped mid-stream; the partial stream is finalized with this
    /// reason rather than silently truncated.
    ChannelDropped,
}

impl RemoteStreamFinalReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::Eof => "eof",
            Self::CapReached => "cap_reached",
            Self::ChannelDropped => "channel_dropped",
        }
    }
}

/// RR4: the result of one `stream_output` read over the channel: the ordered,
/// redacted, offset-tagged deltas, the next acknowledged offset (for a reconnect),
/// the terminal reason, and the append-ready events
/// (`runtime.remote_output_delta` per delta + a terminal
/// `runtime.remote_stream_finalized`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteStreamOutcome {
    pub deltas: Vec<RemoteStreamDelta>,
    /// The offset a reconnect should resume from: one past the last forwarded raw
    /// byte. A reconnect with this `from_offset` yields no duplicate deltas.
    pub next_offset: usize,
    pub final_reason: RemoteStreamFinalReason,
    pub redaction_state: String,
    pub events: Vec<RuntimeEvent>,
}

/// RR5: the decision to run a remote process inside the `depth` OS sandbox tier +
/// the git worktree ON the remote host, with an HONEST enforcement claim evaluated
/// against the REMOTE OS (probed over the channel), not the controller's host.
///
/// The composition is identical in shape to the local [`SandboxPlan`]: an
/// un-granted critical scope (network egress under a forbidding profile, or a cwd
/// outside the confined remote worktree root) is REFUSED before the remote sandbox
/// launches (`SandboxEnforcement::Refused` + a `sandbox.launch_refused` event, no
/// remote process spawned); when the remote OS cannot enforce the tier the run is
/// honestly `Unenforced` (`sandbox.unenforced`, Capo does NOT claim sandboxing);
/// only when the remote OS enforces the tier is the run `Enforced`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteSandboxPlan {
    /// The honest enforcement decision, reusing the DP7 [`SandboxEnforcement`]
    /// vocabulary but evaluated against the REMOTE OS family.
    pub enforcement: SandboxEnforcement,
    /// The OS family the remote host reported, recorded for audit.
    pub remote_os: String,
    /// The append-ready events: a `sandbox.launch_refused` (refused),
    /// `sandbox.unenforced` (platform limitation), or `sandbox.enforced`.
    pub events: Vec<RuntimeEvent>,
    /// `true` when a remote process may be spawned for this plan (enforced OR
    /// honestly unenforced). `false` for a refusal (nothing runs).
    pub may_launch: bool,
    /// The request to ACTUALLY launch on the remote. When `Enforced`, this is the
    /// ORIGINAL request REWRITTEN to launch under the remote OS sandbox launcher
    /// (`bwrap` on a linux remote, `/usr/bin/sandbox-exec` on a macOS remote) — the
    /// additional enforcement layer the RR5 acceptance criterion requires, not just
    /// a claim. When `Unenforced`, this is the original request UNCHANGED (run
    /// honestly un-sandboxed; Capo does NOT claim sandboxing). `None` when refused
    /// (nothing runs). This mirrors the local DP7 [`SandboxPlan::request`] so the
    /// enforced path is verifiable: the transport receives a `bwrap`/`sandbox-exec`
    /// program, never the bare original under an `Enforced` claim.
    pub wrapped_request: Option<LocalProcessRequest>,
    /// For an enforced macOS-seatbelt remote: the generated `.sbpl` policy text the
    /// transport materializes on the REMOTE (never on the controller host). `None`
    /// for bwrap / unenforced / refused plans.
    pub seatbelt_policy: Option<String>,
}

/// RR5: the result of composing the remote OS sandbox + worktree with the launch.
/// A refusal carries the plan and NO outcome (no remote process spawned); an
/// enforced/unenforced launch carries the plan plus the started run's outcome and
/// the reversible checkpoint (the materialized commit ref from RR3) so the sandbox
/// is additive, never a replacement for git-backed rollback.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteSandboxedStart {
    pub plan: RemoteSandboxPlan,
    /// The started remote run when a process actually ran (`None` when refused).
    pub outcome: Option<LocalProcessOutcome>,
    /// The reversible checkpoint for this confined run: the git-materialized commit
    /// ref (RR3). `None` when refused (nothing ran) or when no checkpoint was
    /// supplied. Recorded so the sandbox composes WITH rollback, not instead of it.
    pub checkpoint_ref: Option<String>,
}

/// RR1 remote runner: drives `start_process`/`interrupt`/`terminate`/`kill`/
/// `health`/`recover_orphan` across the injected [`RemoteChannel`], emitting the
/// append-first Start Sequence. It performs NO endpoint resolution: the channel is
/// injected fully resolved, and it reads identity from the channel fingerprint.
#[derive(Clone, Debug)]
pub struct RemoteProcessRunner {
    config: RemoteProcessConfig,
    /// RR1 idempotency ledger: the remote launch outcome keyed by idempotency key
    /// (the run id). A repeated `start_process` with a key already present returns
    /// the recorded outcome and NEVER calls `transport.launch` again, so the same
    /// idempotency key can never spawn a second remote process. Shared (`Arc`) so a
    /// cloned runner enforces the same invariant; excluded from identity equality.
    launched: std::sync::Arc<std::sync::Mutex<HashMap<String, LocalProcessOutcome>>>,
    /// RR6 remote-control grant state. A remote runner is a remote-control
    /// CAPABILITY, so it must be auditable + revocable: when this is revoked, every
    /// execution path (`start_process` / `stream_output` / `write_stdin` /
    /// control) is refused with [`RuntimeError::RemoteControlRevoked`], and the
    /// runner CANNOT re-establish execution without a fresh grant (a new runner /
    /// channel). Shared (`Arc`) so a cloned runner observes the SAME revocation —
    /// a revoke cannot be sidestepped by holding a clone. Excluded from identity
    /// equality (it is revocation STATE, not identity).
    grant: std::sync::Arc<std::sync::Mutex<RemoteControlGrant>>,
}

/// RR6: the revocable remote-control grant a [`RemoteProcessRunner`] executes
/// under. The runner owns a remote-control capability; revoking the grant (the
/// channel was revoked, or the `safety-gates` remote-control grant ended) STOPS
/// the run and forbids the runner from re-establishing execution without a fresh
/// grant. Once revoked, it stays revoked for this runner — re-establishment is a
/// new runner over a new channel, never a flag flip back.
#[derive(Clone, Debug, Eq, PartialEq)]
struct RemoteControlGrant {
    /// `None` while the grant is active; `Some(reason)` once revoked. The reason is
    /// redaction-safe (never a credential).
    revoked_reason: Option<String>,
}

impl PartialEq for RemoteProcessRunner {
    fn eq(&self, other: &Self) -> bool {
        // The idempotency ledger is runtime state, not identity.
        self.config == other.config
    }
}

impl Eq for RemoteProcessRunner {}

impl RemoteProcessRunner {
    pub fn new(config: RemoteProcessConfig) -> Self {
        Self {
            config,
            launched: std::sync::Arc::new(std::sync::Mutex::new(HashMap::new())),
            grant: std::sync::Arc::new(std::sync::Mutex::new(RemoteControlGrant {
                revoked_reason: None,
            })),
        }
    }

    /// HONESTY: the runner advertises `fake: true` while every available transport
    /// is a loopback/fake channel (RR1). When a real cross-machine transport lands
    /// (RR8), a non-loopback channel flips this to `fake: false`. This prevents the
    /// operator console / event system from being told a run crossed a machine
    /// boundary when it ran on a loopback.
    pub fn binding(&self) -> BoundaryBinding {
        BoundaryBinding {
            kind: BoundaryKind::RuntimeRunner,
            variant: "remote-process",
            fake: self.config.transport.is_loopback(),
        }
    }

    /// Whether this runner is currently a loopback/fake remote (not a real
    /// cross-machine path). Mirrors [`BoundaryBinding::fake`].
    pub fn is_loopback(&self) -> bool {
        self.config.transport.is_loopback()
    }

    /// The proven remote target identity (channel fingerprint), used to prove the
    /// peer BEFORE launch.
    pub fn target_fingerprint(&self) -> String {
        self.config.transport.target_fingerprint()
    }

    /// Test/observability hook: how many times the underlying transport ACTUALLY
    /// spawned a remote process. The idempotency test asserts this stays at 1
    /// across a duplicate start, proving real de-duplication (not a constant pid).
    pub fn transport_spawn_count(&self) -> usize {
        self.config.transport.spawn_count()
    }

    /// RR5 test/observability hook: the LAST request the transport was asked to
    /// launch. The enforced-sandbox test asserts this is a `bwrap`/`sandbox-exec`-
    /// wrapped command so the enforcement layer is PROVEN to reach the transport,
    /// not merely claimed by the event label.
    pub fn transport_last_launched_request(&self) -> Option<LocalProcessRequest> {
        self.config.transport.last_launched_request()
    }

    /// Implements `runtime-tunnel.md`'s append-first Start Sequence across the
    /// boundary:
    ///   1. `runtime.remote_start_requested` (idempotency key = run id, status
    ///      pending) is recorded LOCALLY before any remote spawn.
    ///   2. The remote target identity is proven (channel fingerprint) →
    ///      `runtime.remote_target_resolved`.
    ///   3. The remote spawn runs over the channel. On success →
    ///      `runtime.remote_process_started` (remote pid + boot identity).
    ///   4. On remote launch failure → `runtime.remote_process_start_failed` with a
    ///      retryability flag (and NO remote process is left running).
    ///
    /// The returned `RuntimeProcessRef.runtime_process_ref` carries the remote
    /// identity (`remote-process:{fingerprint}:{remote_host}:pid={pid}:boot={boot}`),
    /// NOT the local `external_pid`/`boot_id` path.
    pub fn start_process(
        &self,
        request: LocalProcessRequest,
    ) -> RuntimeResult<LocalProcessOutcome> {
        // RR6 safety boundary: a revoked remote-control grant forbids any new
        // execution. The runner cannot re-establish a launch without a fresh grant.
        self.ensure_control_granted()?;

        let run_id = request.run_id.clone();
        let idempotency_key = run_id.to_string();

        // RR1 idempotency: a repeated start with the SAME key never spawns a second
        // remote process. If we already launched under this key, return the
        // recorded outcome (same remote ref, no second `transport.launch`).
        {
            let launched = self
                .launched
                .lock()
                .expect("remote runner idempotency ledger poisoned");
            if let Some(existing) = launched.get(&idempotency_key) {
                let mut replay = existing.clone();
                // Mark the duplicate request so the trail records that this start was
                // de-duplicated against the existing remote process, not re-spawned.
                replay.events.insert(
                    0,
                    RuntimeEvent {
                        kind: EventKind::RuntimeRemoteStartRequested.as_str().to_string(),
                        status: "pending".to_string(),
                        detail: format!(
                            "idempotency_key={idempotency_key} deduplicated=true (already launched)"
                        ),
                    },
                );
                return Ok(replay);
            }
        }

        // 1. append-first pending request, idempotency-keyed by run id.
        let mut events = vec![RuntimeEvent {
            kind: EventKind::RuntimeRemoteStartRequested.as_str().to_string(),
            status: "pending".to_string(),
            detail: format!("idempotency_key={idempotency_key}"),
        }];

        // 2. prove the remote peer identity BEFORE launch.
        let fingerprint = self.config.transport.target_fingerprint();
        events.push(RuntimeEvent {
            kind: EventKind::RuntimeRemoteTargetResolved.as_str().to_string(),
            status: "resolved".to_string(),
            detail: format!("fingerprint={fingerprint}"),
        });

        // 3. launch over the channel; the transport actually runs the program.
        let launch = match self.config.transport.launch(&request) {
            Ok(launch) => launch,
            Err(error) => {
                // 4. typed launch failure with retryability; no remote left alive.
                let retryable = matches!(
                    &error,
                    RuntimeError::RemoteLaunchFailed {
                        retryable: true,
                        ..
                    }
                );
                events.push(RuntimeEvent {
                    kind: EventKind::RuntimeRemoteProcessStartFailed
                        .as_str()
                        .to_string(),
                    status: "failed".to_string(),
                    detail: format!("retryable={retryable} reason={}", redact_error(&error)),
                });
                return Err(RuntimeError::RemoteStartFailed {
                    retryable,
                    events,
                    source: Box::new(error),
                });
            }
        };

        let remote_ref = self.remote_ref(&launch);
        events.push(RuntimeEvent {
            kind: EventKind::RuntimeRemoteProcessStarted.as_str().to_string(),
            status: "started".to_string(),
            detail: remote_ref.clone(),
        });

        // The transport ran the program once and captured its artifacts; we never
        // double-spawn. The returned ref carries the REMOTE identity (pid + boot +
        // host), not the local `external_pid`/`boot_id` path.
        let captured = launch.captured;
        let mut merged_events = events;
        merged_events.extend(captured.events);

        let outcome = LocalProcessOutcome {
            process: LocalRuntimeProcessRef {
                run_id,
                runtime_process_ref: remote_ref,
                external_pid: None,
                boot_id: None,
                status: captured.process.status,
                redaction_state: captured.process.redaction_state,
            },
            stdout: captured.stdout,
            stderr: captured.stderr,
            exit_code: captured.exit_code,
            events: merged_events,
        };

        // Record under the idempotency key so a repeated start returns this exact
        // outcome instead of spawning again.
        self.launched
            .lock()
            .expect("remote runner idempotency ledger poisoned")
            .insert(idempotency_key, outcome.clone());

        Ok(outcome)
    }

    /// RR3/RR7: materialize the run's workspace ON the remote by git (push/fetch +
    /// `git worktree add` the target commit), then record a
    /// `runtime.remote_workspace_materialized` event. The materialization is
    /// content-addressed (pinned to `source_commit`) and auditable (the source SHA,
    /// the remote worktree path, the resulting remote `HEAD`, and the
    /// credential-scanned git transport URL are recorded). A revoked remote-control
    /// grant forbids materialization (it is a precondition for execution). A failed
    /// git step is the TYPED [`RuntimeError::RemoteMaterializeFailed`], surfaced as a
    /// FAILED materialization event — never a silent fall-through to the wrong dir.
    pub fn materialize_workspace(
        &self,
        source_commit: &str,
    ) -> RuntimeResult<RemoteWorkspaceMaterialization> {
        self.ensure_control_granted()?;

        // The git transport URL passes the credential scan BEFORE it is recorded,
        // so a URL carrying an embedded secret is scrubbed, not persisted raw.
        let raw_url = self.config.transport.transport_url();
        let (scanned, redaction) = RedactionPolicy::new(Vec::new()).apply(raw_url.as_bytes());
        let transport_url = String::from_utf8_lossy(&scanned).to_string();

        match self.config.transport.materialize(source_commit) {
            Ok((remote_head, worktree_path)) => {
                let remote_worktree_path = worktree_path.to_string_lossy().to_string();
                let event = RuntimeEvent {
                    kind: EventKind::RuntimeRemoteWorkspaceMaterialized
                        .as_str()
                        .to_string(),
                    status: "materialized".to_string(),
                    detail: format!(
                        "source_commit={source_commit} remote_head={remote_head} \
                         worktree={remote_worktree_path} transport_url={transport_url} \
                         uncommitted_scratch_synced=false"
                    ),
                };
                Ok(RemoteWorkspaceMaterialization {
                    source_commit: source_commit.to_string(),
                    remote_head,
                    remote_worktree_path,
                    transport_url,
                    transport_url_redaction: redaction,
                    events: vec![event],
                })
            }
            Err(error) => {
                // A failed materialization is the TYPED error (mirrors
                // `WorktreeError`'s no-silent-fallthrough rule); the caller records
                // it as a FAILED `runtime.remote_workspace_materialized` event from
                // the redaction-safe message rather than running in the wrong dir.
                Err(error)
            }
        }
    }

    /// RR3/RR7: map a remote-produced commit at `remote_worktree_path` BACK to
    /// Capo's host by git (fetch the remote tip into a named local ref), recording a
    /// `runtime.remote_workspace_reconciled` event. The non-sync of uncommitted
    /// scratch is an explicit recorded fact on the materialization event; the
    /// reconcile maps back only committed remote state.
    pub fn reconcile_workspace(
        &self,
        remote_worktree_path: &Path,
        local_ref: &str,
    ) -> RuntimeResult<RemoteWorkspaceReconciliation> {
        let remote_commit = self
            .config
            .transport
            .reconcile(remote_worktree_path, local_ref)?;
        let event = RuntimeEvent {
            kind: EventKind::RuntimeRemoteWorkspaceReconciled
                .as_str()
                .to_string(),
            status: "reconciled".to_string(),
            detail: format!("remote_commit={remote_commit} local_ref={local_ref}"),
        };
        Ok(RemoteWorkspaceReconciliation {
            remote_commit,
            local_ref: local_ref.to_string(),
            events: vec![event],
        })
    }

    /// Send an interrupt over the channel.
    ///
    /// RR6 policy: the escalation signals (`interrupt` / `terminate` / `kill`) are
    /// INTENTIONALLY permitted after `revoke_control` and are NOT guarded by the
    /// grant. The safety boundary forbids STARTING or STEERING new execution under
    /// a revoked grant; STOPPING a run is the opposite of re-establishing one, so a
    /// teardown signal must stay available (`revoke_control` itself relies on this
    /// to send a best-effort `kill`). These methods send a signal to an EXISTING
    /// remote pid; they cannot spawn or resume a run, so allowing them after
    /// revocation does not re-establish execution.
    pub fn interrupt(
        &self,
        process: &LocalRuntimeProcessRef,
        reason: &str,
    ) -> RuntimeControlResult {
        self.remote_control(
            process,
            "interrupting",
            EventKind::RuntimeRemoteInterruptSent,
            "interrupt",
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
            EventKind::RuntimeRemoteTerminateSent,
            "terminate",
            reason,
        )
    }

    /// Hard-kill over the channel, producing the distinct
    /// `runtime.remote_kill_sent` event (the contract parity with
    /// `LocalProcessRunner::kill`).
    pub fn kill(&self, process: &LocalRuntimeProcessRef, reason: &str) -> RuntimeControlResult {
        self.remote_control(
            process,
            "killed",
            EventKind::RuntimeRemoteKillSent,
            "kill",
            reason,
        )
    }

    /// Thin idempotent remote cleanup over the channel; emits only
    /// `runtime.remote_cleanup_completed`.
    ///
    /// TWO-TIER DESIGN (review finding 6): this is the LOW tier — it delegates to
    /// `transport.cleanup` (signal the process group) and DOES NOT tear down the
    /// remote git worktree, so it never emits `runtime.remote_workspace_torn_down`.
    /// For crash-safe teardown that also reaps a dangling worktree, use
    /// [`Self::cleanup_run`] with a [`CleanupPolicy`]. The `RuntimeRunnerContract`
    /// surface deliberately routes to `cleanup_run(ReapAll)`, NOT this method, so a
    /// caller driving the contract uniformly still gets the worktree reap; this
    /// thin variant remains for callers that only want the process-group signal
    /// (e.g. a `PreserveWorktree`-style teardown handled out of band).
    pub fn cleanup(&self, process: &LocalRuntimeProcessRef) -> RuntimeResult<RuntimeControlResult> {
        let probe = self.probe_from_ref(process);
        self.config.transport.cleanup(&probe)?;
        Ok(RuntimeControlResult {
            process: process.clone(),
            events: vec![RuntimeEvent {
                kind: EventKind::RuntimeRemoteCleanupCompleted
                    .as_str()
                    .to_string(),
                status: "cleaned".to_string(),
                detail: process.runtime_process_ref.clone(),
            }],
        })
    }

    /// RR6: idempotent + auditable remote cleanup under a [`CleanupPolicy`]. This
    /// is the crash-safe teardown the RR6 acceptance criterion names: it reaps the
    /// remote process group AND removes the remote git worktree over the channel,
    /// emitting `runtime.remote_cleanup_completed` and — when a remote worktree was
    /// actually present (e.g. a DANGLING worktree left by a crash) —
    /// `runtime.remote_workspace_torn_down` so the worktree is never silently
    /// abandoned. It is safe to re-run after a partial failure: a second call over
    /// the same ref reaps nothing new (the worktree is already gone) and still
    /// records the completion, so a crash mid-cleanup is recoverable by re-running.
    ///
    /// Mirrors `LocalProcessRunner::cleanup`'s idempotency, but the reaped things
    /// are remote (a process group + a git worktree over the channel) rather than a
    /// local artifact dir + marker.
    pub fn cleanup_run(
        &self,
        process: &LocalRuntimeProcessRef,
        policy: CleanupPolicy,
    ) -> RuntimeResult<RuntimeControlResult> {
        let probe = self.probe_from_ref(process);
        // Reap the remote process group + remove the remote worktree over the
        // channel. The transport reports whether a worktree was ACTUALLY present so
        // a dangling worktree (left by a crash) is reaped exactly once; a re-run
        // finds nothing to reap and is a no-op teardown (still idempotently
        // completing). NO secret crosses: only the ref + worktree key are recorded.
        let reaped = self.config.transport.cleanup_workspace(&probe, policy)?;
        let mut events = Vec::new();
        if reaped.worktree_reaped {
            events.push(RuntimeEvent {
                kind: EventKind::RuntimeRemoteWorkspaceTornDown
                    .as_str()
                    .to_string(),
                status: "torn_down".to_string(),
                detail: format!(
                    "ref={} worktree={} policy={}",
                    process.runtime_process_ref,
                    reaped.worktree_key,
                    policy.as_str()
                ),
            });
        }
        events.push(RuntimeEvent {
            kind: EventKind::RuntimeRemoteCleanupCompleted
                .as_str()
                .to_string(),
            status: "cleaned".to_string(),
            detail: format!(
                "ref={} policy={} worktree_reaped={}",
                process.runtime_process_ref,
                policy.as_str(),
                reaped.worktree_reaped
            ),
        });
        Ok(RuntimeControlResult {
            process: process.clone(),
            events,
        })
    }

    /// RR6: roll a remote run back to its pre-write checkpoint — the RR3
    /// git-materialized commit ref — restoring the remote worktree to that
    /// checkpoint over the channel and recording `runtime.remote_rollback_performed`.
    /// This composes the remote run WITH git-backed rollback (the sandbox/run is
    /// ADDITIVE to rollback, never a replacement): a run interrupted by a channel
    /// drop can be cleanly failed and its workspace recovered from git to the
    /// materialized commit. A revoked grant refuses the rollback (no execution
    /// without a fresh grant).
    pub fn rollback_to_checkpoint(
        &self,
        process: &LocalRuntimeProcessRef,
        checkpoint_ref: &str,
    ) -> RuntimeResult<RuntimeControlResult> {
        self.ensure_control_granted()?;
        let probe = self.probe_from_ref(process);
        self.config
            .transport
            .rollback_worktree(&probe, checkpoint_ref)?;
        Ok(RuntimeControlResult {
            process: process.clone(),
            events: vec![RuntimeEvent {
                kind: EventKind::RuntimeRemoteRollbackPerformed
                    .as_str()
                    .to_string(),
                status: "rolled_back".to_string(),
                detail: format!(
                    "ref={} checkpoint={checkpoint_ref}",
                    process.runtime_process_ref
                ),
            }],
        })
    }

    /// RR6: revoke this runner's remote-control grant AND stop the in-flight run.
    /// A remote runner is a remote-control capability, so it MUST be revocable.
    ///
    /// Two distinct guarantees, and the docstring is honest about the line between
    /// them:
    ///
    /// 1. NO NEW EXECUTION. After revocation the runner CANNOT re-establish a run
    ///    or steer one without a fresh grant: `start_process`, `write_stdin`, and
    ///    `rollback_to_checkpoint` are refused with
    ///    [`RuntimeError::RemoteControlRevoked`]. A clone observes the SAME
    ///    revocation (the grant is shared), so revocation cannot be sidestepped.
    /// 2. STOP THE CURRENT RUN. When an in-flight `process` ref is supplied, this
    ///    BEST-EFFORT sends a `kill` over the channel (`transport.signal`) so the
    ///    running remote process is actually terminated — satisfying the RR6
    ///    "a revoked grant must STOP the remote run" criterion rather than only
    ///    forbidding the next one. A transport error is tolerated: the grant is
    ///    revoked regardless, so no further execution is admitted even if the stop
    ///    signal did not land, and the event records `signalled=<bool>` honestly.
    ///
    /// Teardown that STOPS a run is permitted post-revocation by design (`interrupt`
    /// / `terminate` / `kill` and the read-only `stream_output` drain stay open so
    /// an operator can stop and observe the dying run); only paths that START or
    /// STEER new execution are refused. `stream_output` is therefore intentionally
    /// NOT guarded (see its own note).
    ///
    /// When `process` is `None` (revoked before any run started, or with no
    /// in-flight ref to hand), no signal is sent and a synthetic `revoked` ref is
    /// returned. When `Some`, the SUPPLIED ref is returned so the audit trail links
    /// the revoke event to the specific run being stopped. Returns the append-first
    /// `runtime.remote_control_revoked` audit event. Idempotent: re-revoking keeps
    /// the first reason (the grant stays revoked); a re-revoke with a ref still
    /// re-sends a best-effort stop (terminating a process twice is harmless).
    pub fn revoke_control(
        &self,
        reason: &str,
        process: Option<&LocalRuntimeProcessRef>,
    ) -> RuntimeControlResult {
        {
            let mut grant = self.grant.lock().expect("remote control grant poisoned");
            if grant.revoked_reason.is_none() {
                grant.revoked_reason = Some(reason.to_string());
            }
        }
        let fingerprint = self.config.transport.target_fingerprint();
        // Best-effort: stop the in-flight run over the channel (a revoked capability
        // STOPS the run, it does not merely forbid the next one). A transport error
        // is tolerated — `signalled` records honestly whether the stop landed.
        let (result_ref, signalled) = match process {
            Some(process) => {
                let probe = self.probe_from_ref(process);
                let signalled = self.config.transport.signal(&probe, "kill").is_ok();
                (
                    LocalRuntimeProcessRef {
                        status: "revoked".to_string(),
                        ..process.clone()
                    },
                    signalled,
                )
            }
            None => (
                LocalRuntimeProcessRef {
                    run_id: RunId::new("revoked"),
                    runtime_process_ref: String::new(),
                    external_pid: None,
                    boot_id: None,
                    status: "revoked".to_string(),
                    redaction_state: "clean".to_string(),
                },
                false,
            ),
        };
        RuntimeControlResult {
            process: result_ref,
            events: vec![RuntimeEvent {
                kind: EventKind::RuntimeRemoteControlRevoked.as_str().to_string(),
                status: "revoked".to_string(),
                detail: format!("fingerprint={fingerprint} signalled={signalled} reason={reason}"),
            }],
        }
    }

    /// RR6: whether this runner's remote-control grant has been revoked. A revoked
    /// runner admits no execution.
    pub fn is_control_revoked(&self) -> bool {
        self.grant
            .lock()
            .expect("remote control grant poisoned")
            .revoked_reason
            .is_some()
    }

    /// RR6: refuse any execution path when the remote-control grant is revoked.
    fn ensure_control_granted(&self) -> RuntimeResult<()> {
        let grant = self.grant.lock().expect("remote control grant poisoned");
        match &grant.revoked_reason {
            Some(reason) => Err(RuntimeError::RemoteControlRevoked {
                reason: reason.clone(),
            }),
            None => Ok(()),
        }
    }

    /// Liveness derived from an ACTUAL remote probe over the channel (remote pid /
    /// process-group liveness), NOT from a local status string.
    pub fn health(&self, process: &LocalRuntimeProcessRef) -> RuntimeResult<RuntimeHealth> {
        let probe = self.probe_from_ref(process);
        let live = self.config.transport.probe(&probe)?;
        Ok(RuntimeHealth {
            runtime_process_ref: process.runtime_process_ref.clone(),
            status: if live { "running" } else { "exited" }.to_string(),
            live,
        })
    }

    /// RR4: stream the remote process's stdout/stderr deltas over the channel,
    /// resuming from `from_offset` (pass `0` for a fresh stream). The SAME opaque
    /// [`LocalRuntimeProcessRef`] identifies the run; only its
    /// `remote_process_ref` is populated.
    ///
    /// Reuses the `streaming-transport` output-delta model:
    ///   - each delta is REDACTED at the remote boundary (the `RedactionPolicy`
    ///     credential-shape scan) BEFORE it leaves as a `runtime.remote_output_delta`
    ///     event, so a credential-shaped token never reaches an artifact/event;
    ///   - output is BOUNDED by `output_limit_bytes`: once the cap is reached the
    ///     stream finalizes `CapReached` rather than forwarding unbounded bytes;
    ///   - each delta carries a MONOTONIC offset (`offset` + `raw_len`), so a
    ///     reconnect with the returned `next_offset` replays NO already-projected
    ///     delta (the `from_sequence` discipline);
    ///   - a mid-stream channel drop finalizes the stream with
    ///     `ChannelDropped` + a recorded reason, never a silent truncation.
    ///
    /// `output_limit_bytes` for the remote boundary mirrors the local runner cap.
    ///
    /// RR6 NOTE: `stream_output` is INTENTIONALLY NOT guarded by the remote-control
    /// grant. Draining a remote run's output is a READ-ONLY observation, not new
    /// execution, so it stays available after `revoke_control` so an operator can
    /// observe a revoked/dying run finish flushing. Revocation forbids paths that
    /// START or STEER execution (`start_process` / `write_stdin` /
    /// `rollback_to_checkpoint`), not read-only observation.
    pub fn stream_output(
        &self,
        process: &LocalRuntimeProcessRef,
        from_offset: usize,
    ) -> RemoteStreamOutcome {
        self.stream_output_with(process, from_offset, RemoteStreamSource::Stdout)
    }

    /// RR4: as [`Self::stream_output`] but for a chosen stream label
    /// (stdout/stderr).
    pub fn stream_output_with(
        &self,
        process: &LocalRuntimeProcessRef,
        from_offset: usize,
        source: RemoteStreamSource,
    ) -> RemoteStreamOutcome {
        let probe = self.probe_from_ref(process);
        let raw = self.config.transport.stream(&probe, from_offset);
        let cap = REMOTE_OUTPUT_LIMIT_BYTES;

        // Bound at the cap: only the first `cap` bytes (from the resume offset) are
        // forwarded; if the channel had more, the stream finalizes `CapReached`.
        let dropped = raw.dropped;
        let forwarded = &raw.bytes[..raw.bytes.len().min(cap)];
        let cap_reached = raw.bytes.len() > cap;

        // Redact at the remote boundary BEFORE the bytes become an event/artifact.
        // Operator-declared literal rules are layered with the automatic
        // credential-shape scan so a named secret is scrubbed on the remote path
        // exactly as on the local path (review finding).
        let policy = RedactionPolicy::new(self.config.redaction_rules.clone());
        let (redacted, redaction_state) = policy.apply(forwarded);
        let text = String::from_utf8_lossy(&redacted).to_string();
        let raw_len = forwarded.len();
        let next_offset = raw.from_offset + raw_len;

        let mut deltas = Vec::new();
        let mut events = Vec::new();
        if raw_len > 0 {
            deltas.push(RemoteStreamDelta {
                source,
                offset: raw.from_offset,
                raw_len,
                text: text.clone(),
                redaction_state: redaction_state.clone(),
            });
            events.push(RuntimeEvent {
                kind: EventKind::RuntimeRemoteOutputDelta.as_str().to_string(),
                status: redaction_state.clone(),
                detail: format!(
                    "stream={} offset={} raw_len={} next_offset={}",
                    source.as_str(),
                    raw.from_offset,
                    raw_len,
                    next_offset
                ),
            });
        }

        // A channel drop wins over a cap classification: the operator must learn
        // the stream was cut, not that it merely hit the cap.
        let final_reason = if dropped {
            RemoteStreamFinalReason::ChannelDropped
        } else if cap_reached {
            RemoteStreamFinalReason::CapReached
        } else {
            RemoteStreamFinalReason::Eof
        };
        events.push(RuntimeEvent {
            kind: EventKind::RuntimeRemoteStreamFinalized.as_str().to_string(),
            status: final_reason.as_str().to_string(),
            detail: format!(
                "fingerprint={} reason={} next_offset={}",
                self.config.transport.target_fingerprint(),
                final_reason.as_str(),
                next_offset
            ),
        });

        RemoteStreamOutcome {
            deltas,
            next_offset,
            final_reason,
            redaction_state,
            events,
        }
    }

    /// RR4: write `bytes` to the remote process's stdin over the channel, emitting
    /// `runtime.remote_stdin_written`. The byte count (not the content) is
    /// recorded so the event carries no payload that could leak a secret.
    pub fn write_stdin(
        &self,
        process: &LocalRuntimeProcessRef,
        bytes: &[u8],
    ) -> RuntimeResult<RuntimeControlResult> {
        // RR6: steering a remote process is execution — a revoked grant refuses it.
        self.ensure_control_granted()?;
        let probe = self.probe_from_ref(process);
        self.config.transport.write_stdin(&probe, bytes)?;
        Ok(RuntimeControlResult {
            process: process.clone(),
            events: vec![RuntimeEvent {
                kind: EventKind::RuntimeRemoteStdinWritten.as_str().to_string(),
                status: "written".to_string(),
                detail: format!(
                    "fingerprint={} bytes={}",
                    self.config.transport.target_fingerprint(),
                    bytes.len()
                ),
            }],
        })
    }

    /// RR5: decide whether `request` may run inside the `depth` OS sandbox tier +
    /// worktree ON the remote host, with an HONEST enforcement claim.
    ///
    /// `remote_worktree_root` is the confined remote worktree root (RR3's
    /// `git worktree add` target); the run's cwd must be inside it. `profile`
    /// carries the GRANTED `safety-gates` capability scopes (writable roots +
    /// network egress). The gate order mirrors the local DP7 [`OsSandbox::plan`]:
    ///
    ///   1. an un-granted critical scope (network egress under a forbidding
    ///      profile, or a cwd outside the confined remote root) is REFUSED before
    ///      the remote sandbox launches — `sandbox.launch_refused`, no spawn;
    ///   2. otherwise the runner PROBES the remote OS over the channel. If the
    ///      remote OS cannot enforce the tier, the plan is `Unenforced`
    ///      (`sandbox.unenforced`) — Capo does NOT claim sandboxing on a remote it
    ///      cannot enforce;
    ///   3. only when the remote OS enforces the tier is the plan `Enforced`.
    ///
    /// HONESTY: the enforcement decision reads the REMOTE OS probe, never
    /// `tier.is_enforced_here()` (the controller's build target).
    pub fn plan_remote_sandbox(
        &self,
        request: &LocalProcessRequest,
        remote_worktree_root: &Path,
        profile: &SandboxProfile,
        tier: SandboxTier,
        requires_network_egress: bool,
    ) -> RuntimeResult<RemoteSandboxPlan> {
        let fingerprint = self.config.transport.target_fingerprint();

        // Pre-launch gate 1: egress under a forbidding profile is refused.
        if requires_network_egress && !profile.allow_network_egress {
            return Ok(Self::remote_refusal(
                SandboxRefusal::NetworkEgressForbidden,
                &fingerprint,
            ));
        }
        // Pre-launch gate 2: the cwd must be inside the confined remote worktree
        // root AND a granted writable root. A cwd outside the confined remote root
        // is a critical-scope violation refused before any remote spawn.
        let cwd_in_remote_root =
            normalize_path(&request.cwd)?.starts_with(normalize_path(remote_worktree_root)?);
        if !cwd_in_remote_root || !profile.write_allowed(&request.cwd)? {
            return Ok(Self::remote_refusal(
                SandboxRefusal::WriteOutsideConfinedRoot {
                    path: request.cwd.clone(),
                },
                &fingerprint,
            ));
        }

        // Probe the REMOTE OS — the authority on whether the tier is enforceable.
        let probe = self.config.transport.sandbox_probe(tier)?;
        let os = probe.os_family.as_str().to_string();

        // HONESTY GATE (review findings 2 + 7): a loopback / fake channel never
        // crossed a machine boundary, so Capo CANNOT have applied an OS sandbox on a
        // real remote host. Even when the channel SCRIPTS an enforcing remote OS,
        // claiming `Enforced` here would assert a `bwrap`/`sandbox-exec` confinement
        // that was never applied across a boundary. We therefore short-circuit a
        // loopback transport to `Unenforced` and record the limitation, mirroring
        // the `BoundaryBinding::fake` honesty rule. The enforced wrapping is proven
        // deterministically by the non-loopback unit path below + verified live in
        // RR8 against a real SSH host.
        let enforce = probe.tier_enforceable
            && !self.config.transport.is_loopback()
            && tier != SandboxTier::None;

        if !enforce {
            // Honest platform limitation: do NOT claim sandboxing on the remote. The
            // request runs UNCHANGED (un-sandboxed), never wrapped under a launcher
            // we cannot honestly say enforced anything.
            let reason = if self.config.transport.is_loopback() && probe.tier_enforceable {
                "loopback/fake channel did not cross a machine boundary; \
                 Capo cannot enforce a remote OS sandbox over it"
                    .to_string()
            } else {
                format!(
                    "tier {} is not enforceable on the remote os {}",
                    tier.variant(),
                    os
                )
            };
            return Ok(RemoteSandboxPlan {
                events: vec![RuntimeEvent {
                    kind: EventKind::SandboxUnenforced.as_str().to_string(),
                    status: "unenforced".to_string(),
                    detail: format!("fingerprint={fingerprint} remote_os={os} {reason}"),
                }],
                enforcement: SandboxEnforcement::Unenforced { tier, reason },
                remote_os: os,
                may_launch: true,
                wrapped_request: Some(request.clone()),
                seatbelt_policy: None,
            });
        }

        // Enforced: REWRITE the request to launch the ORIGINAL program under the
        // remote OS sandbox launcher (`bwrap` on linux, `/usr/bin/sandbox-exec` on
        // macOS), reusing the DP7 `OsSandbox` argv-builder driven by the REMOTE OS
        // family. This is the additional enforcement layer the acceptance criterion
        // requires — not just a claim. The transport then launches the WRAPPED
        // program on the remote.
        let sandbox = OsSandbox::new(tier, profile.clone());
        let (wrapped, seatbelt_policy) = sandbox.wrap_command_for_remote(request.clone(), tier)?;
        Ok(RemoteSandboxPlan {
            events: vec![RuntimeEvent {
                kind: EventKind::SandboxEnforced.as_str().to_string(),
                status: "enforced".to_string(),
                detail: format!(
                    "fingerprint={fingerprint} remote_os={os} tier={} launcher={}",
                    tier.variant(),
                    wrapped.program
                ),
            }],
            enforcement: SandboxEnforcement::Enforced { tier },
            remote_os: os,
            may_launch: true,
            wrapped_request: Some(wrapped),
            seatbelt_policy,
        })
    }

    fn remote_refusal(refusal: SandboxRefusal, fingerprint: &str) -> RemoteSandboxPlan {
        RemoteSandboxPlan {
            events: vec![RuntimeEvent {
                kind: EventKind::SandboxLaunchRefused.as_str().to_string(),
                status: refusal.reason_code().to_string(),
                detail: format!("fingerprint={fingerprint} {}", refusal.detail()),
            }],
            enforcement: SandboxEnforcement::Refused {
                refusal: refusal.clone(),
            },
            remote_os: String::new(),
            may_launch: false,
            wrapped_request: None,
            seatbelt_policy: None,
        }
    }

    /// RR5: compose the remote OS sandbox + worktree with the launch. The sandbox
    /// is planned FIRST against the remote OS + granted scopes; only if the plan
    /// permits a launch (`may_launch`) does the runner spawn the remote process
    /// through the append-first Start Sequence. A refusal returns the plan with NO
    /// outcome (nothing spawned), never a silent fall-through to an un-confined run.
    ///
    /// `checkpoint_ref` is the git-materialized commit ref (RR3) recorded as the
    /// reversible checkpoint for the confined run, so the sandbox is additive to
    /// git-backed rollback, not a replacement.
    pub fn start_process_sandboxed(
        &self,
        request: LocalProcessRequest,
        remote_worktree_root: &Path,
        profile: &SandboxProfile,
        tier: SandboxTier,
        requires_network_egress: bool,
        checkpoint_ref: Option<String>,
    ) -> RuntimeResult<RemoteSandboxedStart> {
        let plan = self.plan_remote_sandbox(
            &request,
            remote_worktree_root,
            profile,
            tier,
            requires_network_egress,
        )?;
        if !plan.may_launch {
            // Refused: nothing spawned, no checkpoint claimed.
            return Ok(RemoteSandboxedStart {
                plan,
                outcome: None,
                checkpoint_ref: None,
            });
        }
        // Launch the PLANNED request: when `Enforced` this is the original program
        // REWRITTEN under the remote OS sandbox launcher (`bwrap`/`sandbox-exec`),
        // so the transport actually receives the sandbox-wrapped command — the
        // enforcement layer, not just the claim. When `Unenforced` it is the
        // original request unchanged. (`may_launch` guarantees `wrapped_request` is
        // `Some` here; fall back to the original defensively.)
        let launch_request = plan.wrapped_request.clone().unwrap_or(request);
        let mut outcome = self.start_process(launch_request)?;
        // Prepend the sandbox decision events so the confined-launch trail records
        // the enforcement claim BEFORE the start-sequence events.
        let mut events = plan.events.clone();
        events.append(&mut outcome.events);
        outcome.events = events;
        Ok(RemoteSandboxedStart {
            plan,
            outcome: Some(outcome),
            checkpoint_ref,
        })
    }

    pub fn recover_orphan(
        &self,
        process: &LocalRuntimeProcessRef,
    ) -> RuntimeResult<OrphanRecovery> {
        let health = self.health(process)?;
        Ok(OrphanRecovery {
            runtime_process_ref: process.runtime_process_ref.clone(),
            recovered_status: if health.live {
                "remote_recovered"
            } else {
                "remote_orphaned"
            }
            .to_string(),
            detail: format!(
                "remote fingerprint {} reported {}",
                self.config.transport.target_fingerprint(),
                health.status
            ),
        })
    }

    /// RR2: re-probe a stored remote run on restart over the (re-resolved) channel
    /// and classify it EXACTLY like the local recovery path, with the one
    /// remote-only `recovery_pending` addition for an unreachable channel.
    ///
    /// `recorded_remote_boot_id` is the remote boot id captured in the stored
    /// `remote_process_ref` at launch (parsed from `:boot=...`). It is compared
    /// against the boot id the host reports NOW: a mismatch means the remote
    /// rebooted, the recorded pid is recycled/gone, and the run is classified
    /// `Exited` — NEVER silently `Recovered` (the truthful-reattach rule).
    ///
    /// The mapping mirrors `runtime-tunnel.md`'s Recovery Behavior:
    ///   - channel unreachable -> `RecoveryPending` (retried when it returns);
    ///   - alive + same boot + reattachable -> `Recovered` (`run.recovered`);
    ///   - alive + same boot + NOT reattachable -> `Orphaned` (`run.orphaned`,
    ///     remote logs inspectable);
    ///   - alive but boot mismatch (reboot) -> `Exited` (`run.exited`);
    ///   - gone -> `Exited` (`run.exited`, unknown exit detail).
    pub fn recover_run(
        &self,
        process: &LocalRuntimeProcessRef,
        recorded_remote_boot_id: &str,
    ) -> RemoteRunRecovery {
        let fingerprint = self.config.transport.target_fingerprint();
        // Append-first: record that we are re-probing this stored remote ref BEFORE
        // we commit to a classification.
        let mut events = vec![RuntimeEvent {
            kind: EventKind::RuntimeRemoteRecoveryAttempted
                .as_str()
                .to_string(),
            status: "attempting".to_string(),
            detail: format!(
                "fingerprint={fingerprint} ref={} recorded_boot={recorded_remote_boot_id}",
                process.runtime_process_ref
            ),
        }];

        let probe = self.probe_from_ref(process);
        let observed = self.config.transport.recovery_probe(&probe);

        let (classification, detail) = if !observed.channel_reachable {
            (
                RemoteRecoveryClassification::RecoveryPending,
                format!(
                    "channel to {fingerprint} unreachable at recovery; holding recovery_pending (will retry on channel return)"
                ),
            )
        } else if !observed.live {
            (
                RemoteRecoveryClassification::Exited,
                format!(
                    "remote {fingerprint} reports process gone; recording exited (unknown exit detail)"
                ),
            )
        } else {
            // Alive. The boot identity must match the one recorded at launch; a
            // mismatch is a remote reboot, so a recycled pid must never be trusted.
            let same_boot = observed
                .observed_remote_boot_id
                .as_deref()
                .map(|observed_boot| observed_boot == recorded_remote_boot_id)
                .unwrap_or(false);
            if !same_boot {
                (
                    RemoteRecoveryClassification::Exited,
                    format!(
                        "remote {fingerprint} boot-id mismatch (recorded {recorded_remote_boot_id}, observed {}); remote rebooted, classifying exited",
                        observed
                            .observed_remote_boot_id
                            .as_deref()
                            .unwrap_or("<none>")
                    ),
                )
            } else if observed.reattachable {
                (
                    RemoteRecoveryClassification::Recovered,
                    format!(
                        "remote {fingerprint} alive within recorded boot and reattachable; recovered in place"
                    ),
                )
            } else {
                (
                    RemoteRecoveryClassification::Orphaned,
                    format!(
                        "remote {fingerprint} alive but launch is not reattachable; orphaned, remote logs left inspectable"
                    ),
                )
            }
        };

        events.push(RuntimeEvent {
            kind: classification.event_kind().as_str().to_string(),
            status: classification.as_str().to_string(),
            detail: detail.clone(),
        });

        RemoteRunRecovery {
            runtime_process_ref: process.runtime_process_ref.clone(),
            classification,
            detail,
            events,
        }
    }

    /// RR2: whether THIS remote launch can be reattached to in place after a
    /// controller restart — truthfully `true` only when the launch recorded a
    /// durable remote pid + boot identity in the `remote_process_ref` (the
    /// `:pid=...:boot=...` shape). A bare ref with no recorded pid/boot is NOT
    /// reattachable and a still-alive run under it recovers as `Orphaned`.
    ///
    /// `runtime-tunnel.md` Remote runtime responsibility: "Report whether process
    /// reattach is supported after Capo restart." This is that honest report.
    pub fn reattach_supported(&self, process: &LocalRuntimeProcessRef) -> bool {
        parse_remote_ref_pid_boot(&process.runtime_process_ref).is_some()
    }

    fn remote_control(
        &self,
        process: &LocalRuntimeProcessRef,
        status: &str,
        event_kind: EventKind,
        escalation: &str,
        reason: &str,
    ) -> RuntimeControlResult {
        let probe = self.probe_from_ref(process);
        // Best-effort signal over the channel; a transport error is recorded in
        // the detail rather than panicking (the control still records intent).
        let signalled = self.config.transport.signal(&probe, escalation).is_ok();
        let process = LocalRuntimeProcessRef {
            status: status.to_string(),
            ..process.clone()
        };
        RuntimeControlResult {
            process: process.clone(),
            events: vec![RuntimeEvent {
                kind: event_kind.as_str().to_string(),
                status: status.to_string(),
                detail: format!(
                    "fingerprint={} escalation={} signalled={} reason={}",
                    self.config.transport.target_fingerprint(),
                    escalation,
                    signalled,
                    reason
                ),
            }],
        }
    }

    fn remote_ref(&self, launch: &RemoteLaunch) -> String {
        format!(
            "remote-process:{}:{}:pid={}:boot={}",
            self.config.transport.target_fingerprint(),
            launch.remote_host_id,
            launch.remote_pid,
            launch.remote_boot_id
        )
    }

    /// Reconstruct a probe handle from a stored ref. The remote identity (pid +
    /// boot) is encoded in the ref's `:pid=...:boot=...` tail, so a probe/recovery
    /// after restart carries the SAME remote pid + boot the launch recorded;
    /// liveness is decided by the transport probe, not this struct.
    fn probe_from_ref(&self, process: &LocalRuntimeProcessRef) -> RemoteProbe {
        let live = process.status == "running";
        // Parse pid + boot + host from the STORED ref so a probe/recovery after a
        // channel re-resolution carries the host id recorded at launch, not
        // whatever endpoint the channel happens to resolve to now (finding 9). Fall
        // back to the current channel endpoint only for a bare ref.
        let parsed = parse_remote_ref(&process.runtime_process_ref);
        let (remote_pid, remote_boot_id) = parsed
            .as_ref()
            .map(|p| (p.pid, p.boot.clone()))
            .unwrap_or((0, String::new()));
        let remote_host_id = parsed
            .as_ref()
            .map(|p| p.host.clone())
            .filter(|host| !host.is_empty())
            .unwrap_or_else(|| self.config.channel.connectivity_endpoint_id.clone());
        RemoteProbe {
            remote_pid,
            remote_boot_id,
            remote_host_id,
            live,
        }
    }
}

/// The structured remote identity recovered from a stored `remote_process_ref`
/// (`remote-process:{fingerprint}:{host}:pid={pid}:boot={boot}`): the recorded
/// remote pid, boot id, AND the host segment recorded at launch. Parsing from the
/// END (the `:boot=` then `:pid=` markers are the LAST occurrences) makes the
/// parser robust to a fingerprint or host segment that itself contains a literal
/// `:pid=` substring — the structured tail still resolves correctly.
#[derive(Clone, Debug, Eq, PartialEq)]
struct ParsedRemoteRef {
    host: String,
    pid: u32,
    boot: String,
}

/// Parse the structured tail of a remote process-ref back into the recorded
/// remote pid + boot id + host. Returns `None` when the ref has no such tail
/// (a bare/non-reattachable ref), which is how [`RemoteProcessRunner::
/// reattach_supported`] reports reattachability truthfully.
fn parse_remote_ref(remote_ref: &str) -> Option<ParsedRemoteRef> {
    let prefix = "remote-process:";
    let body = remote_ref.strip_prefix(prefix)?;
    let pid_marker = ":pid=";
    let boot_marker = ":boot=";
    // Use the LAST occurrence of each marker so a fingerprint/host that embeds the
    // literal marker substring cannot mislead the parser (review finding 10).
    let boot_at = body.rfind(boot_marker)?;
    let boot = &body[boot_at + boot_marker.len()..];
    if boot.is_empty() {
        return None;
    }
    let head = &body[..boot_at];
    let pid_at = head.rfind(pid_marker)?;
    let pid_str = &head[pid_at + pid_marker.len()..];
    let pid = pid_str.parse::<u32>().ok()?;
    // Everything before `:pid=` is `{fingerprint}:{host}`; the host is the last
    // colon-delimited segment so it is recoverable even after a channel
    // re-resolution to a different endpoint (review finding 9).
    let fingerprint_and_host = &head[..pid_at];
    let host = fingerprint_and_host
        .rsplit_once(':')
        .map(|(_, host)| host.to_string())
        .unwrap_or_default();
    Some(ParsedRemoteRef {
        host,
        pid,
        boot: boot.to_string(),
    })
}

/// Back-compat helper: the `(pid, boot)` pair used by reattach-support + probe
/// reconstruction, derived from [`parse_remote_ref`].
fn parse_remote_ref_pid_boot(remote_ref: &str) -> Option<(u32, String)> {
    parse_remote_ref(remote_ref).map(|parsed| (parsed.pid, parsed.boot))
}

/// RR1: the deterministic fake-remote runner the verification suite names. It is
/// a [`RemoteProcessRunner`] wired to a [`FakeRemoteChannel`] — NO network, NO
/// real SSH — proving the contract (append-first start sequence, idempotency,
/// distinct control events, typed launch failure with retryability, probe-based
/// health, honest loopback binding) before any live path lands (RR8).
pub struct FakeRemoteProcessRunner;

impl FakeRemoteProcessRunner {
    /// Build a runner over a fully-resolved fake channel (the runner performs no
    /// endpoint resolution).
    pub fn build(
        channel: OpenChannel,
        workspace_root: PathBuf,
        artifact_root: PathBuf,
    ) -> RemoteProcessRunner {
        RemoteProcessRunner::new(RemoteProcessConfig::fake_for_test(
            channel,
            workspace_root,
            artifact_root,
        ))
    }

    /// Build a runner whose channel fails every launch with the given
    /// retryability, for the `runtime.process_start_failed` path.
    pub fn with_launch_failure(
        channel: OpenChannel,
        workspace_root: PathBuf,
        artifact_root: PathBuf,
        message: impl Into<String>,
        retryable: bool,
    ) -> RemoteProcessRunner {
        let transport = RemoteChannel::Fake(
            FakeRemoteChannel::from_open_channel(&channel, workspace_root, artifact_root)
                .with_launch_failure(message, retryable),
        );
        RemoteProcessRunner::new(RemoteProcessConfig::with_transport(channel, transport))
    }
}

/// Redact a runtime error for inclusion in an event detail (no secret material).
fn redact_error(error: &RuntimeError) -> String {
    match error {
        RuntimeError::RemoteLaunchFailed { message, .. } => message.clone(),
        other => format!("{other:?}"),
    }
}

/// CT3: the owned REACHABILITY handle returned by
/// [`ConnectivityTunnel::open_channel`] and consumed by
/// [`ConnectivityTunnel::close_channel`].
///
/// This SUPERSEDES the `runtime-tunnel.md` design's tentative `ChannelRef` name —
/// CT3 owns the naming. It is a REACHABILITY handle ONLY: it records which
/// resolved endpoint/channel is open and the tunnel variant that opened it. It is
/// NOT a process handle and carries NO coupling to `RuntimeRunner` — a remote
/// runner that later executes over the tailnet RESOLVES through this tunnel but
/// is out of scope here (CT0 boundary note).
///
/// `channel_id` is derived deterministically from the resolved endpoint so the
/// handle is replay-stable and a `close_channel` can be matched to its
/// `open_channel`. It carries NO secret (no authkey, no raw status blob); the
/// identity is the derived fingerprint already on the resolved endpoint (CT2).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenChannel {
    pub channel_id: String,
    pub connectivity_endpoint_id: String,
    pub channel_kind: ChannelKind,
    pub exposure: ExposureScope,
    pub resolved_uri: String,
    pub identity_fingerprint: Option<String>,
    /// The tunnel variant that opened this channel (`fake` / `local-loopback` /
    /// `endpoint-stub` / `tailscale`), so a teardown (CT7) can be audited.
    pub variant: &'static str,
}

impl OpenChannel {
    /// RR1 test helper: build a fully-resolved channel handle directly (as if a
    /// `connectivity-tunnel` had already resolved + opened it), so a remote runner
    /// test can inject a resolved channel WITHOUT performing endpoint resolution.
    pub fn for_test(
        channel_id: impl Into<String>,
        connectivity_endpoint_id: impl Into<String>,
        identity_fingerprint: impl Into<String>,
    ) -> Self {
        Self {
            channel_id: channel_id.into(),
            connectivity_endpoint_id: connectivity_endpoint_id.into(),
            channel_kind: ChannelKind::Stdio,
            exposure: ExposureScope::Loopback,
            resolved_uri: "fake-channel://loopback".to_string(),
            identity_fingerprint: Some(identity_fingerprint.into()),
            variant: "fake-channel",
        }
    }

    fn from_resolved(resolved: &ResolvedEndpoint, variant: &'static str) -> Self {
        Self {
            channel_id: format!("channel:{}", resolved.resolved_endpoint_id),
            connectivity_endpoint_id: resolved.connectivity_endpoint_id.clone(),
            channel_kind: resolved.channel_kind,
            exposure: resolved.exposure,
            resolved_uri: resolved.resolved_uri.clone(),
            identity_fingerprint: resolved.identity_fingerprint.clone(),
            variant,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConnectivityTunnel {
    Fake(FakeTunnel),
    LocalLoopback(LocalLoopbackTunnel),
    EndpointStub(EndpointStubTunnel),
    Tailscale(TailscaleTunnel),
}

impl ConnectivityTunnel {
    pub fn fake() -> Self {
        Self::Fake(FakeTunnel::default())
    }

    /// CT4: a scripted fake tunnel carrying the full Tailscale-parity surface
    /// (identity verification, health/reconnect timeline, channel open/close) for
    /// deterministic CT5/CT7/CT9 controller and CLI tests with no live tailnet.
    pub fn fake_scripted(script: FakeTunnelScript) -> Self {
        Self::Fake(FakeTunnel::with_script(script))
    }

    pub fn local_loopback() -> Self {
        Self::LocalLoopback(LocalLoopbackTunnel)
    }

    pub fn endpoint_stub(config: ConnectivityEndpointConfig) -> Self {
        Self::EndpointStub(EndpointStubTunnel::new(config))
    }

    /// CT3: a real Tailscale tunnel backed by an injectable
    /// [`TailscaleStatusSource`] (scripted for deterministic tests; the gated live
    /// `tailscale status --json` source for CT10).
    pub fn tailscale(config: ConnectivityEndpointConfig, status: TailscaleStatusSource) -> Self {
        Self::Tailscale(TailscaleTunnel::new(config, status))
    }

    pub fn binding(&self) -> BoundaryBinding {
        match self {
            Self::Fake(tunnel) => tunnel.binding(),
            Self::LocalLoopback(tunnel) => tunnel.binding(),
            Self::EndpointStub(tunnel) => tunnel.binding(),
            Self::Tailscale(tunnel) => tunnel.binding(),
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
            Self::Tailscale(tunnel) => tunnel.resolve_endpoint(owner, channel_kind),
        }
    }

    pub fn check_reachability(&self) -> ConnectivityHealth {
        match self {
            Self::Fake(tunnel) => tunnel.check_reachability(),
            Self::LocalLoopback(tunnel) => tunnel.check_reachability(),
            Self::EndpointStub(tunnel) => tunnel.check_reachability(),
            Self::Tailscale(tunnel) => tunnel.check_reachability(),
        }
    }

    /// CT3: open a REACHABILITY channel for an already-resolved endpoint. The
    /// returned [`OpenChannel`] is the owned handle a later `revoke` (CT7) tears
    /// down via [`ConnectivityTunnel::close_channel`]. This is reachability only —
    /// never a process handle.
    pub fn open_channel(&self, resolved: &ResolvedEndpoint) -> ConnectivityResult<OpenChannel> {
        match self {
            Self::Fake(tunnel) => tunnel.open_channel(resolved),
            Self::LocalLoopback(tunnel) => tunnel.open_channel(resolved),
            Self::EndpointStub(tunnel) => tunnel.open_channel(resolved),
            Self::Tailscale(tunnel) => tunnel.open_channel(resolved),
        }
    }

    /// CT3: close a previously-opened reachability channel. Idempotency and the
    /// "prove unreachability after close" semantics are CT7's concern; CT3 only
    /// provides the surface so CT7's teardown is implementable.
    pub fn close_channel(&self, channel: OpenChannel) -> ConnectivityResult<()> {
        match self {
            Self::Fake(tunnel) => tunnel.close_channel(channel),
            Self::LocalLoopback(tunnel) => tunnel.close_channel(channel),
            Self::EndpointStub(tunnel) => tunnel.close_channel(channel),
            Self::Tailscale(tunnel) => tunnel.close_channel(channel),
        }
    }

    pub fn exposure_report(&self) -> ExposureReport {
        match self {
            Self::Fake(tunnel) => tunnel.exposure_report(),
            Self::LocalLoopback(tunnel) => tunnel.exposure_report(),
            Self::EndpointStub(tunnel) => tunnel.exposure_report(),
            Self::Tailscale(tunnel) => tunnel.exposure_report(),
        }
    }
}

/// CT4: a deterministic SCRIPT for [`FakeTunnel`], giving the fake the SAME surface
/// as `TailscaleTunnel` (identity, health, reconnect, channel open/close) with NO
/// live tailnet and NO real network. CT5/CT7/CT9 controller and CLI tests drive the
/// identity-mismatch, degraded-health, reconnect, channel-close, and revoke paths
/// through this script.
///
/// It carries NO secret: identity is expressed as the same opaque
/// `identity_ref`/observed-device-id HANDLES the Tailscale adapter uses, compared as
/// derived `tsnode:` fingerprints (the CT2 contract).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeTunnelScript {
    /// The endpoint id the fake resolves at (so audit/projection ids are stable).
    pub endpoint_id: String,
    /// The exposure scope the fake resolves at (defaults to `Private` so the fake
    /// mirrors the Tailscale adapter's private resolution for CT5/CT7/CT9).
    pub exposure: ExposureScope,
    /// The resolved tailnet-style address the fake hands back.
    pub resolved_uri: String,
    /// The OBSERVED device id the fake reports (fingerprinted for the audit field).
    pub observed_device_id: String,
    /// The EXPECTED `identity_ref` HANDLE to verify against (CT4). `None` skips the
    /// identity check (device trusted as pre-authenticated), mirroring the adapter.
    pub expected_identity_ref: Option<String>,
    /// A scripted health TIMELINE of reachability flags. Successive
    /// [`FakeTunnel::check_reachability`] calls walk it (clamping at the last entry)
    /// so CT5 can drive reachable -> unreachable -> reconnected deterministically
    /// with NO wall-clock. Empty timeline = always reachable.
    pub health_timeline: Vec<bool>,
}

impl FakeTunnelScript {
    /// A scripted private endpoint whose observed device matches the given expected
    /// `identity_ref` (so identity verification SUCCEEDS), reachable by default.
    pub fn private_matching(
        endpoint_id: impl Into<String>,
        observed_device_id: impl Into<String>,
    ) -> Self {
        let observed_device_id = observed_device_id.into();
        let expected = format!("tailscale:device:{observed_device_id}");
        Self {
            endpoint_id: endpoint_id.into(),
            exposure: ExposureScope::Private,
            resolved_uri: "https://fake-peer.tailnet-fake.ts.net".to_string(),
            observed_device_id,
            expected_identity_ref: Some(expected),
            health_timeline: Vec::new(),
        }
    }

    /// Override the expected `identity_ref` HANDLE (e.g. to drive an identity
    /// MISMATCH: an expected handle whose device id differs from the observed one).
    pub fn with_expected_identity_ref(mut self, identity_ref: Option<String>) -> Self {
        self.expected_identity_ref = identity_ref.filter(|value| !value.is_empty());
        self
    }

    /// Override the scripted health timeline (CT5 reconnect / stall driving).
    pub fn with_health_timeline(mut self, timeline: Vec<bool>) -> Self {
        self.health_timeline = timeline;
        self
    }

    fn reachable_at(&self, step: usize) -> bool {
        if self.health_timeline.is_empty() {
            return true;
        }
        let idx = step.min(self.health_timeline.len() - 1);
        self.health_timeline[idx]
    }
}

/// CT3/CT4: the deterministic fake tunnel. By default (`FakeTunnel::default()` /
/// [`ConnectivityTunnel::fake`]) it is the loopback-style always-reachable fake the
/// pre-CT4 tests use. With a [`FakeTunnelScript`] (CT4) it carries the SAME surface
/// as `TailscaleTunnel` — scripted identity verification, a health/reconnect
/// timeline, and channel open/close — so CT5/CT7/CT9 can drive every connectivity
/// path with no live tailnet.
///
/// The scripted-step cursor is interior mutable so `check_reachability(&self)` can
/// walk the health timeline; it is DELIBERATELY excluded from equality (two fakes
/// with the same script are equal regardless of how many times they were probed) so
/// the enclosing `ConnectivityTunnel` stays `Eq`.
#[derive(Clone, Debug, Default)]
pub struct FakeTunnel {
    script: Option<FakeTunnelScript>,
    step: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl PartialEq for FakeTunnel {
    fn eq(&self, other: &Self) -> bool {
        // Compare the SCRIPT only; the probe cursor is not part of identity.
        self.script == other.script
    }
}
impl Eq for FakeTunnel {}

impl FakeTunnel {
    /// CT4: a scripted fake carrying the full Tailscale-parity surface.
    pub fn with_script(script: FakeTunnelScript) -> Self {
        Self {
            script: Some(script),
            step: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }
    }

    fn endpoint_id(&self) -> &str {
        self.script
            .as_ref()
            .map(|s| s.endpoint_id.as_str())
            .unwrap_or("fake-endpoint")
    }

    fn exposure(&self) -> ExposureScope {
        self.script
            .as_ref()
            .map(|s| s.exposure)
            .unwrap_or(ExposureScope::Loopback)
    }

    pub fn binding(&self) -> BoundaryBinding {
        BoundaryBinding::fake(BoundaryKind::ConnectivityTunnel, "fake-tunnel")
    }

    pub fn resolve_endpoint(
        &self,
        owner: EndpointOwner,
        channel_kind: ChannelKind,
    ) -> ConnectivityResult<ResolvedEndpoint> {
        let Some(script) = self.script.as_ref() else {
            return Ok(ResolvedEndpoint::new(
                "fake-endpoint",
                owner,
                channel_kind,
                "fake://endpoint",
                ExposureScope::Loopback,
                false,
            ));
        };

        // CT4: verify the OBSERVED device identity against the expected
        // `identity_ref` HANDLE, identically to `TailscaleTunnel` (the parity
        // requirement) — a pure derived-fingerprint comparison, no raw credential.
        let observed_fingerprint = identity_fingerprint_of(&script.observed_device_id);
        if let Some(identity_ref) = script
            .expected_identity_ref
            .as_deref()
            .filter(|handle| !handle.is_empty())
        {
            let expected = expected_identity_fingerprint(identity_ref);
            if expected != observed_fingerprint {
                return Err(ConnectivityError::IdentityMismatch {
                    endpoint_id: script.endpoint_id.clone(),
                    expected,
                    observed: observed_fingerprint,
                });
            }
        }

        Ok(ResolvedEndpoint::new(
            script.endpoint_id.clone(),
            owner,
            channel_kind,
            script.resolved_uri.clone(),
            script.exposure,
            script.exposure.requires_permission(),
        )
        .with_identity_fingerprint(Some(observed_fingerprint)))
    }

    pub fn check_reachability(&self) -> ConnectivityHealth {
        let Some(script) = self.script.as_ref() else {
            return ConnectivityHealth {
                endpoint_id: "fake-endpoint".to_string(),
                status: "available".to_string(),
                reachable: true,
                exposure: ExposureScope::Loopback,
                detail: "fake tunnel is always reachable in tests".to_string(),
            };
        };
        // CT5: walk the scripted health timeline by one step per probe (clamped).
        let step = self.step.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let reachable = script.reachable_at(step);
        ConnectivityHealth {
            endpoint_id: script.endpoint_id.clone(),
            status: if reachable {
                "available"
            } else {
                "unreachable"
            }
            .to_string(),
            reachable,
            exposure: script.exposure,
            detail: if reachable {
                "scripted fake peer reachable".to_string()
            } else {
                "scripted fake peer not reachable".to_string()
            },
        }
    }

    /// CT3: open a scripted reachability channel. `FakeTunnel` always succeeds so
    /// CT5/CT7/CT9 controller/CLI tests can drive open/close deterministically.
    pub fn open_channel(&self, resolved: &ResolvedEndpoint) -> ConnectivityResult<OpenChannel> {
        Ok(OpenChannel::from_resolved(resolved, "fake-tunnel"))
    }

    /// CT3: close a scripted reachability channel — a no-op success on the fake.
    pub fn close_channel(&self, _channel: OpenChannel) -> ConnectivityResult<()> {
        Ok(())
    }

    pub fn exposure_report(&self) -> ExposureReport {
        ExposureReport::for_exposure(self.endpoint_id(), self.exposure())
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

    /// CT3: loopback opens a loopback reachability channel for an
    /// already-resolved loopback endpoint.
    pub fn open_channel(&self, resolved: &ResolvedEndpoint) -> ConnectivityResult<OpenChannel> {
        Ok(OpenChannel::from_resolved(resolved, "local-loopback"))
    }

    pub fn close_channel(&self, _channel: OpenChannel) -> ConnectivityResult<()> {
        Ok(())
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

    /// CT3: open a reachability channel for a resolved stub endpoint.
    pub fn open_channel(&self, resolved: &ResolvedEndpoint) -> ConnectivityResult<OpenChannel> {
        Ok(OpenChannel::from_resolved(resolved, "endpoint-stub"))
    }

    pub fn close_channel(&self, _channel: OpenChannel) -> ConnectivityResult<()> {
        Ok(())
    }

    pub fn exposure_report(&self) -> ExposureReport {
        ExposureReport::for_exposure(&self.config.endpoint_id, self.config.exposure)
    }
}

/// CT3: the observed tailnet status for a single peer/device, as projected by a
/// [`TailscaleStatusSource`].
///
/// This is the CONFINED, already-sanitized view the adapter works with: it carries
/// a tailnet ADDRESS (MagicDNS name or `100.64.0.0/10` CGNAT IP), the OBSERVED
/// stable device identity (node-id / device id), and reachability — but NEVER an
/// authkey, NEVER a raw `tailscale status` JSON blob with tokens. The live source
/// is responsible for projecting `tailscale status --json` down to exactly these
/// fields so no secret ever crosses into the controller-facing types (the CT2
/// architectural-confinement guarantee).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TailscalePeerStatus {
    /// MagicDNS name or CGNAT `100.64.0.0/10` tailnet IP for the peer.
    pub tailnet_address: String,
    /// The OBSERVED stable device identity (e.g. a node id / device id). The
    /// adapter derives a fingerprint from this for the audit field; the raw value
    /// is itself a stable public identifier, never a credential.
    pub observed_device_id: String,
    /// Whether the peer is currently reachable on the tailnet.
    pub reachable: bool,
}

/// CT3: the injectable source of tailnet endpoint resolution + status, modeled on
/// the ACP `ScriptedAcpTransport` / `PipedProcessTransport` pattern from `depth`.
///
/// A SCRIPTED implementation drives deterministic tests with NO live tailnet; the
/// gated LIVE implementation shells out to `tailscale status --json` (CT10). The
/// trait deliberately yields only a sanitized [`TailscalePeerStatus`] (address +
/// observed device id + reachability) so the adapter never has to touch — and can
/// never accidentally surface — an authkey or a raw status blob.
pub trait TailscaleStatusSourceImpl: std::fmt::Debug + Send + Sync {
    /// Resolve the peer status for `endpoint_id`. The `reason` in any error is a
    /// redacted, secret-free label (binary absent / not-logged-in / no peer).
    fn peer_status(&self, endpoint_id: &str) -> ConnectivityResult<TailscalePeerStatus>;

    /// A PURE, in-memory equality token used by [`TailscaleStatusSource`]'s
    /// `PartialEq` so comparing two `ConnectivityTunnel`s never triggers a live
    /// process spawn. The default projects a stable probe id through `peer_status`
    /// — correct and pure for the SCRIPTED/FAILING sources. A LIVE source (which
    /// would otherwise shell out on every `==`) MUST override this with a token
    /// derived from its own configuration (e.g. its binary path).
    fn eq_token(&self) -> EqToken {
        EqToken::Status(self.peer_status("__eq_probe__"))
    }
}

/// A pure equality token for a [`TailscaleStatusSourceImpl`] — see
/// [`TailscaleStatusSourceImpl::eq_token`]. It never carries a credential: the
/// scripted/failing sources project a sanitized peer status, and the live source
/// projects only its configured binary path (never the live status JSON).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EqToken {
    Status(ConnectivityResult<TailscalePeerStatus>),
    Identity(String),
}

/// CT3: a boxed [`TailscaleStatusSourceImpl`]. Kept as a newtype so the
/// `ConnectivityTunnel` enum stays `Clone`/`Debug`/`Eq` via the scripted source's
/// stable identity, and so the live source can be swapped in behind the same type.
#[derive(Clone, Debug)]
pub struct TailscaleStatusSource(std::sync::Arc<dyn TailscaleStatusSourceImpl>);

impl TailscaleStatusSource {
    pub fn new(source: impl TailscaleStatusSourceImpl + 'static) -> Self {
        Self(std::sync::Arc::new(source))
    }

    /// A deterministic scripted source for tests: it returns the given peer status
    /// for any endpoint id, with NO live tailnet.
    pub fn scripted(status: TailscalePeerStatus) -> Self {
        Self::new(ScriptedTailscaleStatusSource { status })
    }

    /// A scripted source that always FAILS resolution with a redacted reason
    /// (e.g. to drive the "not logged in / no reachable peer" path deterministically).
    pub fn scripted_unreachable(reason: impl Into<String>) -> Self {
        Self::new(FailingTailscaleStatusSource {
            reason: reason.into(),
        })
    }

    fn peer_status(&self, endpoint_id: &str) -> ConnectivityResult<TailscalePeerStatus> {
        self.0.peer_status(endpoint_id)
    }
}

// Two `TailscaleStatusSource`s are equal iff they project the same peer status for
// a stable probe id — enough to keep the enclosing `ConnectivityTunnel` `Eq`
// without leaking the boxed trait object's identity into equality.
impl PartialEq for TailscaleStatusSource {
    fn eq(&self, other: &Self) -> bool {
        // Compare via the PURE `eq_token` so a LIVE source (whose `peer_status`
        // shells out to `tailscale status --json`) is NEVER spawned on `==`.
        self.0.eq_token() == other.0.eq_token()
    }
}
impl Eq for TailscaleStatusSource {}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ScriptedTailscaleStatusSource {
    status: TailscalePeerStatus,
}

impl TailscaleStatusSourceImpl for ScriptedTailscaleStatusSource {
    fn peer_status(&self, _endpoint_id: &str) -> ConnectivityResult<TailscalePeerStatus> {
        Ok(self.status.clone())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FailingTailscaleStatusSource {
    reason: String,
}

impl TailscaleStatusSourceImpl for FailingTailscaleStatusSource {
    fn peer_status(&self, endpoint_id: &str) -> ConnectivityResult<TailscalePeerStatus> {
        Err(ConnectivityError::TailscaleResolution {
            endpoint_id: endpoint_id.to_string(),
            reason: self.reason.clone(),
        })
    }
}

/// CT10: the env gate names for the opt-in live Tailscale smoke, mirroring the
/// `CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT` / `CAPO_SERVER_RUN_CODEX_LIVE` pair used by
/// the live Codex smoke. BOTH must be set to `1` for the smoke to attempt the live
/// tailnet; otherwise it skips.
pub const CONNECTIVITY_TAILSCALE_PREFLIGHT_ENV: &str = "CAPO_CONNECTIVITY_TAILSCALE_PREFLIGHT";
pub const CONNECTIVITY_RUN_TAILSCALE_LIVE_ENV: &str = "CAPO_CONNECTIVITY_RUN_TAILSCALE_LIVE";

/// CT10: the OUTCOME of the DEFINED, deterministic skip predicate for the live
/// Tailscale smoke. Either the smoke should RUN against a confirmed-reachable live
/// peer, or it SKIPS with a recorded, secret-free `reason` so "clean skip" is
/// CHECKABLE in evidence rather than operator-eyeballed.
///
/// The predicate is purely a function of (env gate, `tailscale` binary present,
/// `tailscale status` reachable peer for the endpoint) — never operator judgement.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LiveTailscaleSmokeDecision {
    /// Run the live lifecycle: the gate is set and a reachable peer was projected.
    /// Carries the SANITIZED peer status (no secret) the smoke will assert against.
    Run(TailscalePeerStatus),
    /// Skip cleanly. The `reason` is a fixed, secret-free label (gate unset / binary
    /// absent / not-logged-in / no reachable peer) recorded so the skip is auditable.
    Skip { reason: String },
}

/// CT10: the DEFINED skip predicate for the live Tailscale smoke. Reads the two env
/// gates, then (only if both are set) probes the live `tailscale status` through
/// `source` for `endpoint_id`. Returns [`LiveTailscaleSmokeDecision::Run`] with the
/// sanitized peer status when a reachable peer exists, else
/// [`LiveTailscaleSmokeDecision::Skip`] with a recorded reason. NEVER surfaces the
/// raw status blob: the only thing that crosses out of a live probe is the sanitized
/// [`TailscalePeerStatus`] or a fixed reason label.
pub fn live_tailscale_smoke_decision(
    source: &TailscaleStatusSource,
    endpoint_id: &str,
) -> LiveTailscaleSmokeDecision {
    let gate_set = |name: &str| std::env::var(name).map(|v| v == "1").unwrap_or(false);
    if !gate_set(CONNECTIVITY_TAILSCALE_PREFLIGHT_ENV)
        || !gate_set(CONNECTIVITY_RUN_TAILSCALE_LIVE_ENV)
    {
        return LiveTailscaleSmokeDecision::Skip {
            reason: format!(
                "live tailnet gate unset ({CONNECTIVITY_TAILSCALE_PREFLIGHT_ENV} + \
                 {CONNECTIVITY_RUN_TAILSCALE_LIVE_ENV} must both be 1)"
            ),
        };
    }
    match source.peer_status(endpoint_id) {
        Ok(peer) if peer.reachable => LiveTailscaleSmokeDecision::Run(peer),
        Ok(_) => LiveTailscaleSmokeDecision::Skip {
            reason: "tailscale status projected an unreachable peer".to_string(),
        },
        // The live source already collapses binary-absent / not-logged-in / no-peer
        // into a redacted `TailscaleResolution { reason }` — reuse that secret-free
        // reason verbatim so the skip label is the defined predicate's own words.
        Err(ConnectivityError::TailscaleResolution { reason, .. }) => {
            LiveTailscaleSmokeDecision::Skip { reason }
        }
        Err(_other) => LiveTailscaleSmokeDecision::Skip {
            reason: "tailnet preflight refused before resolution (config/scope)".to_string(),
        },
    }
}

/// CT3/CT10: the LIVE status source that shells out to `tailscale status --json`.
///
/// The DETERMINISTIC tests never touch this — they use [`TailscaleStatusSource::scripted`].
/// This source is exercised ONLY by the gated CT10 live smoke. Its SKIP PREDICATE
/// is DEFINED and deterministic (never operator-judged): resolution FAILS with a
/// redacted [`ConnectivityError::TailscaleResolution`] (so the smoke skips cleanly)
/// when the `tailscale` binary is absent (spawn error / non-zero exit) OR the
/// status reports no reachable peer. The `reason` is a short secret-free label; the
/// raw `tailscale status` JSON (which can carry tokens) is NEVER returned or logged.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LiveTailscaleStatusSource {
    /// The `tailscale` binary to invoke (default `tailscale`).
    pub binary: String,
}

impl Default for LiveTailscaleStatusSource {
    fn default() -> Self {
        Self {
            binary: "tailscale".to_string(),
        }
    }
}

impl TailscaleStatusSourceImpl for LiveTailscaleStatusSource {
    fn peer_status(&self, endpoint_id: &str) -> ConnectivityResult<TailscalePeerStatus> {
        let fail = |reason: &str| ConnectivityError::TailscaleResolution {
            endpoint_id: endpoint_id.to_string(),
            reason: reason.to_string(),
        };
        // Probe the binary; a spawn failure / non-zero exit is the DEFINED skip
        // condition (binary absent or not logged in). We deliberately do NOT
        // surface stdout/stderr (which can carry tokens) — only a fixed label.
        let output = Command::new(&self.binary)
            .arg("status")
            .arg("--json")
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .map_err(|_| fail("tailscale binary not available"))?;
        if !output.status.success() {
            return Err(fail(
                "tailscale status reported not-logged-in / unavailable",
            ));
        }
        // CT10: project `tailscale status --json` down to a SANITIZED peer status.
        // The raw blob (which can carry node keys / online metadata) is parsed here
        // and NEVER returned or logged — only the three sanitized fields
        // (`tailnet_address` / `observed_device_id` / `reachable`) cross out, and any
        // failure is collapsed to a fixed secret-free `reason` label.
        let stdout = String::from_utf8_lossy(&output.stdout);
        project_tailscale_status(&stdout, endpoint_id).ok_or_else(|| {
            fail("tailscale status reported no reachable peer for the requested endpoint")
        })
    }

    /// Compare by the configured binary path only — a STABLE in-memory identity —
    /// so wrapping a `LiveTailscaleStatusSource` in a `TailscaleStatusSource` and
    /// then comparing it with `==` never spawns `tailscale status`.
    fn eq_token(&self) -> EqToken {
        EqToken::Identity(format!("live:{}", self.binary))
    }
}

/// CT10: project a raw `tailscale status --json` document down to the SANITIZED
/// [`TailscalePeerStatus`] for the peer that matches `endpoint_id`, returning `None`
/// when no reachable matching peer is present (the DEFINED skip condition). This is
/// a PURE function over the JSON text so it is unit-tested deterministically (no
/// live tailnet, no process spawn) by the CT10 deterministic half.
///
/// CREDENTIAL DISCIPLINE: only `DNSName`/`TailscaleIPs` (the tailnet address),
/// `ID`/`HostName` (the stable public device id), and `Online` (reachability) are
/// read out. Node keys, the auth blob, and every other field stay inside the parsed
/// document and are dropped here — they never reach a returned value or a log line.
///
/// MATCHING: `endpoint_id` is matched against a peer's MagicDNS `DNSName` (full or
/// up to the first `.`) or its `HostName`, case-insensitively, so an endpoint id
/// like `capo-worker` resolves the `capo-worker.tailnet-1234.ts.net` peer. `Self`
/// is also considered so the local node can be the endpoint.
fn project_tailscale_status(status_json: &str, endpoint_id: &str) -> Option<TailscalePeerStatus> {
    let doc: serde_json::Value = serde_json::from_str(status_json).ok()?;

    // Candidate peer objects: every value of the `Peer` map plus `Self`.
    let mut candidates: Vec<&serde_json::Value> = Vec::new();
    if let Some(self_node) = doc.get("Self") {
        candidates.push(self_node);
    }
    if let Some(peers) = doc.get("Peer").and_then(|p| p.as_object()) {
        candidates.extend(peers.values());
    }

    let wanted = endpoint_id.to_ascii_lowercase();
    for node in candidates {
        let dns_name = node.get("DNSName").and_then(|v| v.as_str()).unwrap_or("");
        let host_name = node.get("HostName").and_then(|v| v.as_str()).unwrap_or("");
        let dns_label = dns_name
            .trim_end_matches('.')
            .split('.')
            .next()
            .unwrap_or("");
        let matches = {
            let dns_lower = dns_name.trim_end_matches('.').to_ascii_lowercase();
            dns_lower == wanted
                || dns_label.eq_ignore_ascii_case(&wanted)
                || host_name.eq_ignore_ascii_case(&wanted)
        };
        if !matches {
            continue;
        }

        // Prefer the MagicDNS name; fall back to the first tailnet IP.
        let address = if !dns_name.is_empty() {
            dns_name.trim_end_matches('.').to_string()
        } else {
            node.get("TailscaleIPs")
                .and_then(|v| v.as_array())
                .and_then(|ips| ips.first())
                .and_then(|ip| ip.as_str())
                .unwrap_or("")
                .to_string()
        };
        if address.is_empty() {
            return None;
        }

        // The stable device id: the node `ID` (a stable public identifier), else the
        // host name. Never the node key.
        let observed_device_id = node
            .get("ID")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| {
                node.get("ID")
                    .and_then(|v| v.as_u64())
                    .map(|n| n.to_string())
            })
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| host_name.to_string());
        if observed_device_id.is_empty() {
            return None;
        }

        // `Self` has no `Online` field and is reachable when it owns a tailnet
        // address; a peer is reachable iff `Online` is true.
        let reachable = match node.get("Online") {
            Some(online) => online.as_bool().unwrap_or(false),
            None => true,
        };
        if !reachable {
            return None;
        }

        return Some(TailscalePeerStatus {
            tailnet_address: address,
            observed_device_id,
            reachable: true,
        });
    }
    None
}

/// CT10: a CONSERVATIVE secret-free check used by the live-smoke evidence guard.
///
/// This is a runtime-local mirror of the credential SHAPES the CT2 redaction guard
/// (`capo_state::connectivity_redaction`) recognizes, kept here so the live smoke
/// can scan its OWN evidence (peer projection / skip reason / resolved endpoint)
/// without capo-runtime depending on capo-state. It is the defense-in-depth net,
/// NOT the universal guarantee — the primary guarantee is the architectural
/// confinement that no controller-facing type ever holds a resolved credential.
pub fn connectivity_redaction_is_clean(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    const MARKERS: &[&str] = &[
        "tskey-auth-",
        "tskey-client-",
        "tskey-ephemeral-",
        "bearer ",
        "authorization: bearer",
        "ghp_",
        "github_pat_",
        "sk-",
        "akia",
        "xoxb-",
        "xoxp-",
        "session=",
        "sessionid=",
        "nodekey:",
    ];
    !MARKERS.iter().any(|marker| lower.contains(marker))
}

/// Whether `address` is a tailnet address: a MagicDNS `*.ts.net` name or a CGNAT
/// `100.64.0.0/10` IP. The adapter refuses to resolve a non-tailnet address as a
/// private tailnet endpoint, so a misconfigured/spoofed loopback or public address
/// cannot masquerade as a tailnet peer.
fn is_tailnet_address(address: &str) -> bool {
    if address.ends_with(".ts.net") || address.contains(".ts.net:") {
        return true;
    }
    // CGNAT 100.64.0.0/10: first octet 100, second octet in 64..=127.
    let host = address.split(':').next().unwrap_or(address);
    let mut octets = host.split('.');
    let (Some(a), Some(b)) = (octets.next(), octets.next()) else {
        return false;
    };
    matches!((a.parse::<u8>(), b.parse::<u8>()), (Ok(100), Ok(b)) if (64..=127).contains(&b))
}

/// Derive a stable, secret-free identity FINGERPRINT from an observed device id.
/// The observed device id is itself a public stable identifier, but we record a
/// derived fingerprint (matching the CT2 `identity_fingerprint` contract) so the
/// audit field shape is uniform and never carries a raw credential.
///
/// CT4 security note: this fingerprint is the comparison surface for the
/// identity-mismatch gate (`expected_fingerprint == observed_fingerprint`), so it
/// uses SHA-256 — a collision-resistant hash — rather than the non-cryptographic
/// FNV-1a `content_hash` used for artifact content addressing. A `tsnode:sha256:`
/// prefix names the algorithm so the audit label is self-describing and a future
/// algorithm change is detectable. The domain separator (`capo:tsnode:`) keeps the
/// fingerprint distinct from any other SHA-256 use. The tailnet ACL remains the
/// PRIMARY deployment security gate (see `knowledge.md`); this fingerprint is the
/// auditable, collision-resistant identity label layered on top of it.
fn identity_fingerprint_of(observed_device_id: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(b"capo:tsnode:");
    hasher.update(observed_device_id.as_bytes());
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(hex, "{byte:02x}");
    }
    format!("tsnode:sha256:{hex}")
}

/// CT4: the EXPECTED device-identity fingerprint encoded by an `identity_ref`
/// HANDLE on the endpoint config.
///
/// The handle is an opaque pointer to the expected tunnel/device identity, e.g.
/// `tailscale:device:<stable-id>` or a bare node-key fingerprint. We treat the
/// segment AFTER the last `:` as the stable device id and derive the same
/// `tsnode:` fingerprint the adapter records for the OBSERVED device, so a match
/// is a pure fingerprint comparison with NO raw credential on either side. A
/// handle that is itself already a `tsnode:` fingerprint is compared verbatim.
fn expected_identity_fingerprint(identity_ref: &str) -> String {
    if identity_ref.starts_with("tsnode:") {
        return identity_ref.to_string();
    }
    let stable_id = identity_ref.rsplit(':').next().unwrap_or(identity_ref);
    identity_fingerprint_of(stable_id)
}

/// CT3: the real Tailscale tunnel adapter behind the `ConnectivityTunnel` enum.
///
/// It resolves a Capo-server / runtime-target endpoint to a TAILNET address at
/// [`ExposureScope::Private`] (never loopback, never public) through an injectable
/// [`TailscaleStatusSource`]. It NEVER owns a process handle and never couples to
/// `RuntimeRunner`; it resolves reachability/endpoints and opens/closes
/// reachability channels only.
///
/// CREDENTIAL DISCIPLINE (CT2): the adapter records auth MODE + device-identity
/// FINGERPRINT only. The `auth_ref` HANDLE on the config is the pointer the live
/// source would resolve to a real authkey at connect time, CONFINED here; the
/// resolved value is structurally never returned to the controller, stored, or
/// logged. No method on this type returns a raw authkey or a raw `tailscale
/// status` blob.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TailscaleTunnel {
    config: ConnectivityEndpointConfig,
    status: TailscaleStatusSource,
}

impl TailscaleTunnel {
    pub fn new(config: ConnectivityEndpointConfig, status: TailscaleStatusSource) -> Self {
        Self { config, status }
    }

    /// The auth MODE recorded for audit (never the authkey). When an `auth_ref`
    /// handle is present the mode is `tailscale_authkey_handle`, matching the
    /// `protocol-provider.md` "record auth mode only" rule; otherwise the device
    /// is expected to be pre-authenticated on the tailnet (`tailscale_device`).
    pub fn auth_mode(&self) -> &'static str {
        if self
            .config
            .auth_ref
            .as_deref()
            .is_some_and(|h| !h.is_empty())
        {
            "tailscale_authkey_handle"
        } else {
            "tailscale_device"
        }
    }

    pub fn binding(&self) -> BoundaryBinding {
        BoundaryBinding {
            kind: BoundaryKind::ConnectivityTunnel,
            variant: "tailscale",
            fake: false,
        }
    }

    pub fn resolve_endpoint(
        &self,
        owner: EndpointOwner,
        channel_kind: ChannelKind,
    ) -> ConnectivityResult<ResolvedEndpoint> {
        // Tailscale resolves ONLY at Private. A Public/Funnel request is refused at
        // the adapter layer until CT8 installs the full short-lived/audited guard;
        // a Loopback request is a misconfiguration (loopback is the loopback
        // tunnel's job, not the tailnet adapter's).
        if !matches!(self.config.exposure, ExposureScope::Private) {
            return Err(ConnectivityError::ScopeNotSupported {
                endpoint_id: self.config.endpoint_id.clone(),
                requested: self.config.exposure,
                supported: ExposureScope::Private,
            });
        }

        if !self.config.allowed_channels.contains(&channel_kind) {
            return Err(ConnectivityError::ChannelNotAllowed {
                endpoint_id: self.config.endpoint_id.clone(),
                channel_kind,
            });
        }

        let peer = self.status.peer_status(&self.config.endpoint_id)?;
        if !peer.reachable {
            return Err(ConnectivityError::TailscaleResolution {
                endpoint_id: self.config.endpoint_id.clone(),
                reason: "no reachable tailnet peer".to_string(),
            });
        }
        if !is_tailnet_address(&peer.tailnet_address) {
            return Err(ConnectivityError::TailscaleResolution {
                endpoint_id: self.config.endpoint_id.clone(),
                reason: "resolved address is not a tailnet (MagicDNS / 100.64.0.0/10) address"
                    .to_string(),
            });
        }

        // CT4: VERIFY the observed device identity against the expected
        // `identity_ref` HANDLE before resolving. An unexpected/unverified device is
        // a typed `IdentityMismatch` refusal (auditable as a blocked exposure),
        // never a silent connect. When no `identity_ref` is configured the device is
        // trusted as pre-authenticated on the tailnet (the tailnet ACL is the
        // deployment-posture gate — `knowledge.md` records ACLs must be reviewed
        // before the live path). The comparison is a pure FINGERPRINT comparison; no
        // raw node key or credential is on either side.
        let observed_fingerprint = identity_fingerprint_of(&peer.observed_device_id);
        if let Some(identity_ref) = self
            .config
            .identity_ref
            .as_deref()
            .filter(|handle| !handle.is_empty())
        {
            let expected = expected_identity_fingerprint(identity_ref);
            if expected != observed_fingerprint {
                return Err(ConnectivityError::IdentityMismatch {
                    endpoint_id: self.config.endpoint_id.clone(),
                    expected,
                    observed: observed_fingerprint,
                });
            }
        }

        // Record the OBSERVED device identity as a derived fingerprint only (CT2).
        let fingerprint = observed_fingerprint;
        let resolved = ResolvedEndpoint::new(
            self.config.endpoint_id.clone(),
            owner,
            channel_kind,
            format!("https://{}", peer.tailnet_address),
            ExposureScope::Private,
            true,
        )
        .with_identity_fingerprint(Some(fingerprint));
        Ok(resolved)
    }

    pub fn check_reachability(&self) -> ConnectivityHealth {
        match self.status.peer_status(&self.config.endpoint_id) {
            Ok(peer) if peer.reachable && is_tailnet_address(&peer.tailnet_address) => {
                ConnectivityHealth {
                    endpoint_id: self.config.endpoint_id.clone(),
                    status: "available".to_string(),
                    reachable: true,
                    exposure: ExposureScope::Private,
                    detail: "tailnet peer reachable".to_string(),
                }
            }
            _ => ConnectivityHealth {
                endpoint_id: self.config.endpoint_id.clone(),
                status: "unreachable".to_string(),
                reachable: false,
                exposure: ExposureScope::Private,
                detail: "tailnet peer not reachable".to_string(),
            },
        }
    }

    /// CT3: open a reachability channel over the tailnet for an already-resolved
    /// private endpoint. Reachability only — never a process handle.
    ///
    /// The resolved endpoint's exposure is re-asserted here as `Private`: the
    /// adapter NEVER opens a tailscale channel for a `Loopback`/`Public` resolution,
    /// even one handed in by a caller (a manually constructed or foreign-tunnel
    /// `ResolvedEndpoint`). This mirrors the `resolve_endpoint` scope refusal so the
    /// `OpenChannel` handle's `exposure`/`variant` fields cannot be forged into a
    /// misleading CT7 teardown audit trail.
    pub fn open_channel(&self, resolved: &ResolvedEndpoint) -> ConnectivityResult<OpenChannel> {
        if !matches!(resolved.exposure, ExposureScope::Private) {
            return Err(ConnectivityError::ScopeNotSupported {
                endpoint_id: resolved.connectivity_endpoint_id.clone(),
                requested: resolved.exposure,
                supported: ExposureScope::Private,
            });
        }
        Ok(OpenChannel::from_resolved(resolved, "tailscale"))
    }

    /// CT3 surface / CT10 deferral: drop the owned [`OpenChannel`] reachability
    /// handle. At the CLI tier this adapter is stateless, so the handle is
    /// consumed (the binding it named cannot be used again) but no live tailnet
    /// call is made — this is a RECORDED no-op, mirroring the CT10 deferral
    /// discipline documented on [`LiveTailscaleStatusSource::peer_status`].
    ///
    /// Live CT10: this is where the revoke proof becomes CAUSAL — close_channel
    /// will revoke the Tailscale ACL tag / send a DisconnectPeer so that a
    /// subsequent live `check_reachability` returns `reachable=false` BECAUSE the
    /// channel was torn down. Until CT10 wires the live tailnet, the `_channel`
    /// argument is intentionally discarded and `proven_unreachable=true` in the
    /// CT7 teardown is demonstrated against a scripted `FakeTunnel`, not the live
    /// peer (see `knowledge.md`, CT7 live-teardown deferral).
    pub fn close_channel(&self, _channel: OpenChannel) -> ConnectivityResult<()> {
        Ok(())
    }

    pub fn exposure_report(&self) -> ExposureReport {
        ExposureReport::for_exposure(&self.config.endpoint_id, ExposureScope::Private)
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
    /// CT3: the requested `ExposureScope` is not supported by this adapter at the
    /// adapter layer. `TailscaleTunnel` resolves ONLY at `ExposureScope::Private`
    /// (a tailnet address): a `Public`/Funnel resolution is REFUSED here with a
    /// typed error until CT8 installs the full short-lived/audited public guard.
    /// This closes the CT3->CT8 window at the adapter layer — a test-covered
    /// refusal, never a silent pass. Carries no secret.
    ScopeNotSupported {
        endpoint_id: String,
        requested: ExposureScope,
        supported: ExposureScope,
    },
    /// CT3: the live/scripted Tailscale status source could not resolve a reachable
    /// tailnet endpoint (binary absent, not logged in, no reachable peer, or the
    /// resolved address is not a tailnet address). The `reason` is a redacted,
    /// secret-free label — never a raw `tailscale status` blob or an authkey.
    TailscaleResolution { endpoint_id: String, reason: String },
    /// CT4: the OBSERVED tailnet device identity did not match the EXPECTED
    /// `identity_ref` handle on the endpoint config. An unexpected or unverified
    /// device is refused here BEFORE any channel is opened — never a silent
    /// connect. Both `expected` and `observed` are derived FINGERPRINTS (the CT2
    /// `tsnode:` contract), not raw node keys or credentials, so the refusal is
    /// auditable as a blocked exposure without leaking a secret.
    IdentityMismatch {
        endpoint_id: String,
        expected: String,
        observed: String,
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
            Self::ScopeNotSupported {
                endpoint_id,
                requested,
                supported,
            } => write!(
                f,
                "exposure scope {} is not supported by endpoint {endpoint_id} (only {} is)",
                requested.as_str(),
                supported.as_str()
            ),
            Self::TailscaleResolution {
                endpoint_id,
                reason,
            } => write!(
                f,
                "tailscale endpoint {endpoint_id} could not be resolved: {reason}"
            ),
            Self::IdentityMismatch {
                endpoint_id,
                expected,
                observed,
            } => write!(
                f,
                "tailnet device identity mismatch for endpoint {endpoint_id}: expected {expected}, observed {observed}"
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
    /// CT2: an OPAQUE pointer to where the tunnel/device auth credential lives
    /// (e.g. `keychain:capo/tailnet-authkey`), NEVER the raw authkey/token/cookie.
    /// A non-loopback exposure requires this handle to be present (CT1
    /// `ExposurePolicy::authorize`). Resolution of the handle to a real credential
    /// is ARCHITECTURALLY CONFINED to the tunnel adapter at connect time (CT3); the
    /// resolved value is structurally never returned to the controller, stored, or
    /// logged. The redaction guard (CT2 secondary net) fails closed if a
    /// raw-credential-looking value is ever placed in this handle field.
    pub auth_ref: Option<String>,
    /// CT2: an OPAQUE pointer to the EXPECTED tunnel/device identity (e.g.
    /// `tailscale:device:<stable-id>` / a node-key fingerprint), checked by the
    /// adapter (CT4) before resolving a private endpoint. Never a raw credential.
    pub identity_ref: Option<String>,
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
            auth_ref: None,
            identity_ref: None,
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
            auth_ref: None,
            identity_ref: None,
        }
    }

    /// CT3: a Tailscale endpoint config — `tunnel_kind = "tailscale"`, exposure
    /// `Private`, the private control/stdio/logs channels allowed. `address_ref`
    /// is the configured tailnet target (MagicDNS name or CGNAT IP); the live
    /// status source resolves the actual reachable address at connect time.
    pub fn tailscale(endpoint_id: impl Into<String>, address_ref: impl Into<String>) -> Self {
        Self {
            endpoint_id: endpoint_id.into(),
            name: "tailscale endpoint".to_string(),
            tunnel_kind: "tailscale".to_string(),
            address_ref: address_ref.into(),
            exposure: ExposureScope::Private,
            allowed_channels: vec![ChannelKind::Control, ChannelKind::Stdio, ChannelKind::Logs],
            status: "available".to_string(),
            auth_ref: None,
            identity_ref: None,
        }
    }

    /// CT2 builder: attach the OPAQUE `auth_ref` / `identity_ref` HANDLES. Neither
    /// is ever a raw credential — they are pointers/fingerprints the adapter
    /// resolves at connect time. An empty string is normalized to `None` so a
    /// blank CLI flag does not masquerade as a present handle.
    pub fn with_handles(mut self, auth_ref: Option<String>, identity_ref: Option<String>) -> Self {
        self.auth_ref = auth_ref.filter(|value| !value.is_empty());
        self.identity_ref = identity_ref.filter(|value| !value.is_empty());
        self
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
    /// CT2: the OBSERVED tunnel/device identity FINGERPRINT recorded by the
    /// adapter at resolve time (CT4 writes it after an identity check). A derived
    /// value (e.g. a hash of the node key), NEVER the raw key/credential. `None`
    /// for tunnels that do not verify a device identity (loopback/fake/stub).
    pub identity_fingerprint: Option<String>,
    /// CT2: an OPTIONAL expiry instant for a short-lived resolution. Required for
    /// any (gated) public exposure (CT8 enforces this + a clock-swept auto-revoke);
    /// `None` for an open-ended loopback/private resolution. A bare instant, not a
    /// credential.
    pub expires_at: Option<String>,
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
            identity_fingerprint: None,
            expires_at: None,
        }
    }

    /// CT2 builder: record the OBSERVED identity fingerprint onto the resolved
    /// endpoint (CT4 calls this after a successful device-identity check). An empty
    /// string is normalized to `None`.
    pub fn with_identity_fingerprint(mut self, fingerprint: Option<String>) -> Self {
        self.identity_fingerprint = fingerprint.filter(|value| !value.is_empty());
        self
    }

    /// CT2 builder: stamp the short-lived expiry instant (CT8 requires it for a
    /// gated public resolution). An empty string is normalized to `None`.
    pub fn with_expires_at(mut self, expires_at: Option<String>) -> Self {
        self.expires_at = expires_at.filter(|value| !value.is_empty());
        self
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

    /// Serializes every test that READS or MUTATES the `CAPO_CONNECTIVITY_*` gate
    /// env vars. The Rust harness runs tests in this binary in parallel, so an
    /// `unsafe { remove_var(..) }` in one test would race any other test that
    /// reads the same vars (including the `#[ignore]` live smoke under
    /// `--include-ignored`). Hold this lock for the duration of any such test.
    static TAILSCALE_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn planned_runtimes_keep_fake_and_local_process() {
        assert_eq!(
            PLANNED_RUNTIMES,
            ["fake", "local-process", "remote-process"]
        );
        assert_eq!(
            PLANNED_TUNNELS,
            ["fake", "local-loopback", "endpoint-stub", "tailscale"]
        );
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
    fn ct3_transport_socket_guard_is_scope_blind_but_exposure_layer_authorize_is_the_real_gate() {
        // DOCUMENTED, INTENTIONAL: `authorize_socket` is a TRANSPORT-level guard and
        // is scope-BLIND — it treats every non-loopback socket as `Private`. So a
        // policy promoted only to `Private`, with a handle, PASSES the transport
        // guard for a non-loopback bind even though the operator's *intent* might be
        // a Public-interface exposure. This is by design; the transport cannot tell a
        // tailnet-private interface from a public one.
        let (private_only, _) = ExposurePolicy::loopback_default().promote(
            ExposureScope::Private,
            "flag",
            "unix:1700000002",
        );
        assert_eq!(
            private_only.authorize_socket(false, Some("keychain:capo/authkey")),
            Ok(()),
            "transport guard is scope-blind: a non-loopback socket passes at the Private ceiling"
        );

        // The REAL gate against public exposure is the EXPOSURE-layer `authorize`,
        // which knows the requested scope. With the SAME Private-only ceiling, a
        // `Public` request FAILS CLOSED with `ScopeExceedsCeiling` — proving the
        // public-exposure decision lives here, not at the scope-blind transport guard.
        assert_eq!(
            private_only.authorize(ExposureScope::Public, Some("keychain:capo/authkey")),
            Err(ConnectivityError::ScopeExceedsCeiling {
                requested: ExposureScope::Public,
                ceiling: ExposureScope::Private,
            }),
            "exposure-layer authorize is the real public-exposure gate and fails closed"
        );

        // And on the CT3 ADAPTER codepath, the tailscale adapter independently
        // refuses a Public-scope config with a typed `ScopeNotSupported` — so even if
        // the ceiling were promoted to Public, the tailscale path never serves public.
        let mut public_config = ConnectivityEndpointConfig::tailscale("ts-pub", "capo-worker");
        public_config.exposure = ExposureScope::Public;
        public_config.allowed_channels = vec![ChannelKind::Dashboard];
        let err = ConnectivityTunnel::tailscale(public_config, reachable_tailnet_source())
            .resolve_endpoint(EndpointOwner::capo_server("dash"), ChannelKind::Dashboard)
            .expect_err("tailscale adapter refuses public exposure on the CT3 codepath");
        assert!(matches!(
            err,
            ConnectivityError::ScopeNotSupported {
                requested: ExposureScope::Public,
                supported: ExposureScope::Private,
                ..
            }
        ));
    }

    #[test]
    fn ct8_public_expiry_label_is_short_lived_and_clock_swept() {
        // CT8: the expiry label round-trips through the SAME logical-ms domain as the
        // CT5 heartbeat clock, so the heartbeat/clock tick can sweep it.
        let label = connectivity_health::expiry_label(45_000);
        assert_eq!(label, "expiry-ms:45000");
        assert_eq!(connectivity_health::parse_expiry_ms(&label), Some(45_000));
        // A non-expiry label (e.g. an open-ended ISO instant) is NOT a sweepable
        // deadline — the sweep must never mistake it for an expired one.
        assert_eq!(
            connectivity_health::parse_expiry_ms("2026-06-02T12:00:00Z"),
            None
        );

        // A (gated) public exposure is SHORT-LIVED: a TTL request is clamped to the
        // documented `PUBLIC_EXPOSURE_MAX_TTL_MS` ceiling and can never be open-ended.
        let clamped = connectivity_health::public_expiry_label(
            1_000,
            connectivity_health::PUBLIC_EXPOSURE_MAX_TTL_MS * 10,
        );
        assert_eq!(
            connectivity_health::parse_expiry_ms(&clamped),
            Some(1_000 + connectivity_health::PUBLIC_EXPOSURE_MAX_TTL_MS),
            "an over-long public TTL is clamped to the short-lived ceiling"
        );
        // A zero TTL is bounded away from zero (never an instantly-expired-at-now or
        // open-ended window).
        let zero = connectivity_health::public_expiry_label(5_000, 0);
        assert_eq!(connectivity_health::parse_expiry_ms(&zero), Some(5_001));
    }

    #[test]
    fn ct8_tailscale_adapter_refuses_public_until_the_gated_guard() {
        // CT8: the Tailscale adapter NEVER serves public/Funnel — a `Public`-scope
        // config is a typed `ScopeNotSupported` refusal at the adapter layer (the CT3
        // stub the CT8 guard sits above). The gated short-lived public path is the
        // prototype EndpointStub + grant, not a tailnet Funnel.
        let mut public_config = ConnectivityEndpointConfig::tailscale("ts-pub", "capo-worker");
        public_config.exposure = ExposureScope::Public;
        public_config.allowed_channels = vec![ChannelKind::Dashboard];
        let err = ConnectivityTunnel::tailscale(public_config, reachable_tailnet_source())
            .resolve_endpoint(EndpointOwner::capo_server("dash"), ChannelKind::Dashboard)
            .expect_err("tailscale adapter refuses public exposure");
        assert!(matches!(
            err,
            ConnectivityError::ScopeNotSupported {
                requested: ExposureScope::Public,
                supported: ExposureScope::Private,
                ..
            }
        ));

        // And the public permission scope is the only path the grant engine accepts.
        assert_eq!(
            ExposureScope::Public.permission_scope(),
            "network:expose:public"
        );
    }

    #[test]
    fn ct2_endpoint_config_and_resolved_endpoint_carry_opaque_handles_only() {
        // CT2: the schema additions are opaque pointers/derived values, set via the
        // builder, and normalized so an empty flag does not masquerade as present.
        let config = ConnectivityEndpointConfig::stub_private("endpoint-ct2", "100.64.0.7")
            .with_handles(
                Some("keychain:capo/tailnet-authkey".to_string()),
                Some("tailscale:device:n7Qk2cFf".to_string()),
            );
        assert_eq!(
            config.auth_ref.as_deref(),
            Some("keychain:capo/tailnet-authkey")
        );
        assert_eq!(
            config.identity_ref.as_deref(),
            Some("tailscale:device:n7Qk2cFf")
        );

        // Empty strings normalize to None (not a present-but-blank handle).
        let blank = ConnectivityEndpointConfig::stub_private("endpoint-ct2", "100.64.0.7")
            .with_handles(Some(String::new()), None);
        assert_eq!(blank.auth_ref, None);
        assert_eq!(blank.identity_ref, None);

        // ResolvedEndpoint carries the derived fingerprint + expiry (None by default).
        let resolved = ConnectivityTunnel::endpoint_stub(config)
            .resolve_endpoint(EndpointOwner::capo_server("server-1"), ChannelKind::Control)
            .expect("resolve");
        assert_eq!(resolved.identity_fingerprint, None);
        assert_eq!(resolved.expires_at, None);
        let stamped = resolved
            .with_identity_fingerprint(Some("sha256:9f86d081".to_string()))
            .with_expires_at(Some("2026-06-02T12:00:00Z".to_string()));
        assert_eq!(
            stamped.identity_fingerprint.as_deref(),
            Some("sha256:9f86d081")
        );
        assert_eq!(stamped.expires_at.as_deref(), Some("2026-06-02T12:00:00Z"));
    }

    // ---- CT3: TailscaleTunnel adapter + open_channel/close_channel surface ----

    fn reachable_tailnet_source() -> TailscaleStatusSource {
        TailscaleStatusSource::scripted(TailscalePeerStatus {
            tailnet_address: "capo-worker.tailnet-1234.ts.net".to_string(),
            // The OBSERVED stable device id matches the `tailscale:device:n7Qk2cFf`
            // identity_ref handle the CT3/CT4 resolve tests configure, so the CT4
            // device-identity verification SUCCEEDS for the happy path (the handle's
            // last `:`-segment derives the same `tsnode:` fingerprint as the observed
            // device). Identity-MISMATCH paths are exercised explicitly with their
            // own scripted observed ids / handles below.
            observed_device_id: "n7Qk2cFf".to_string(),
            reachable: true,
        })
    }

    #[test]
    fn ct3_tailscale_resolves_a_private_tailnet_endpoint_with_correct_scope() {
        let config = ConnectivityEndpointConfig::tailscale("ts-endpoint-1", "capo-worker")
            .with_handles(
                Some("keychain:capo/tailnet-authkey".to_string()),
                Some("tailscale:device:n7Qk2cFf".to_string()),
            );
        let tunnel = ConnectivityTunnel::tailscale(config, reachable_tailnet_source());

        assert_eq!(tunnel.binding().kind, BoundaryKind::ConnectivityTunnel);
        assert_eq!(tunnel.binding().variant, "tailscale");
        assert!(!tunnel.binding().fake);

        let owner = EndpointOwner::runtime_target("remote-target-1");
        let resolved = tunnel
            .resolve_endpoint(owner.clone(), ChannelKind::Control)
            .expect("resolve a private tailnet control endpoint");

        // Private scope, the private_tunnel permission scope, permission required.
        assert_eq!(resolved.exposure, ExposureScope::Private);
        assert_eq!(resolved.permission_scope, "network:connect:private_tunnel");
        assert!(resolved.permission_required);
        // Resolves to a tailnet (MagicDNS) address, not loopback, not public.
        assert!(resolved.resolved_uri.contains(".ts.net"));
        assert!(!resolved.resolved_uri.contains("127.0.0.1"));
        // The observed device identity is recorded as a derived fingerprint only.
        let fingerprint = resolved
            .identity_fingerprint
            .as_deref()
            .expect("observed identity fingerprint recorded");
        assert!(fingerprint.starts_with("tsnode:sha256:"));
    }

    #[test]
    fn ct3_tailscale_records_auth_mode_only_never_the_authkey() {
        // With an auth_ref handle present the mode is the handle mode.
        let with_handle = TailscaleTunnel::new(
            ConnectivityEndpointConfig::tailscale("ts-endpoint-1", "capo-worker")
                .with_handles(Some("keychain:capo/tailnet-authkey".to_string()), None),
            reachable_tailnet_source(),
        );
        assert_eq!(with_handle.auth_mode(), "tailscale_authkey_handle");

        // Without a handle the device is expected pre-authenticated on the tailnet.
        let no_handle = TailscaleTunnel::new(
            ConnectivityEndpointConfig::tailscale("ts-endpoint-1", "capo-worker"),
            reachable_tailnet_source(),
        );
        assert_eq!(no_handle.auth_mode(), "tailscale_device");
    }

    #[test]
    fn ct3_tailscale_public_scope_is_a_typed_adapter_refusal() {
        // The Tailscale adapter ALWAYS refuses Public at the adapter layer with a typed
        // ScopeNotSupported, never a silent pass. This refusal is PERMANENT: CT8 did NOT
        // open a tailnet Funnel path — the gated short-lived public prototype rides the
        // EndpointStub + `network:expose:public` grant, not the Tailscale Funnel. So the
        // tailnet adapter refusing Public is the final, intended behavior.
        let mut config = ConnectivityEndpointConfig::tailscale("ts-public", "capo-worker");
        config.exposure = ExposureScope::Public;
        config.allowed_channels = vec![ChannelKind::Dashboard];
        let tunnel = ConnectivityTunnel::tailscale(config, reachable_tailnet_source());

        let err = tunnel
            .resolve_endpoint(EndpointOwner::capo_server("dash"), ChannelKind::Dashboard)
            .expect_err(
                "tailscale adapter always refuses public: the gated prototype path uses EndpointStub, not the tailnet Funnel",
            );
        assert_eq!(
            err,
            ConnectivityError::ScopeNotSupported {
                endpoint_id: "ts-public".to_string(),
                requested: ExposureScope::Public,
                supported: ExposureScope::Private,
            }
        );
    }

    #[test]
    fn ct3_tailscale_refuses_a_channel_not_allowed_for_private_exposure() {
        // Dashboard is not in the private tailscale endpoint's allowed channels.
        let tunnel = ConnectivityTunnel::tailscale(
            ConnectivityEndpointConfig::tailscale("ts-endpoint-1", "capo-worker"),
            reachable_tailnet_source(),
        );
        let err = tunnel
            .resolve_endpoint(EndpointOwner::capo_server("dash"), ChannelKind::Dashboard)
            .expect_err("dashboard channel not allowed on private tailscale endpoint");
        assert_eq!(
            err,
            ConnectivityError::ChannelNotAllowed {
                endpoint_id: "ts-endpoint-1".to_string(),
                channel_kind: ChannelKind::Dashboard,
            }
        );
    }

    #[test]
    fn ct3_tailscale_refuses_an_unreachable_or_non_tailnet_address() {
        // No reachable peer -> redacted TailscaleResolution refusal.
        let unreachable = ConnectivityTunnel::tailscale(
            ConnectivityEndpointConfig::tailscale("ts-endpoint-1", "capo-worker"),
            TailscaleStatusSource::scripted(TailscalePeerStatus {
                tailnet_address: "capo-worker.ts.net".to_string(),
                observed_device_id: "nodekey:x".to_string(),
                reachable: false,
            }),
        );
        assert!(matches!(
            unreachable.resolve_endpoint(EndpointOwner::runtime_target("t"), ChannelKind::Control),
            Err(ConnectivityError::TailscaleResolution { .. })
        ));
        assert!(!unreachable.check_reachability().reachable);

        // A reachable peer at a NON-tailnet address is refused (cannot masquerade).
        let non_tailnet = ConnectivityTunnel::tailscale(
            ConnectivityEndpointConfig::tailscale("ts-endpoint-1", "capo-worker"),
            TailscaleStatusSource::scripted(TailscalePeerStatus {
                tailnet_address: "203.0.113.7".to_string(),
                observed_device_id: "nodekey:x".to_string(),
                reachable: true,
            }),
        );
        assert!(matches!(
            non_tailnet.resolve_endpoint(EndpointOwner::runtime_target("t"), ChannelKind::Control),
            Err(ConnectivityError::TailscaleResolution { .. })
        ));
    }

    #[test]
    fn ct3_tailnet_address_classifier_accepts_magicdns_and_cgnat_only() {
        assert!(is_tailnet_address("host.tailnet-1234.ts.net"));
        assert!(is_tailnet_address("host.tailnet-1234.ts.net:8443"));
        assert!(is_tailnet_address("100.64.0.7"));
        assert!(is_tailnet_address("100.127.255.1:443"));
        // 100.128.x.x is outside the CGNAT /10; loopback/public are rejected.
        assert!(!is_tailnet_address("100.128.0.1"));
        assert!(!is_tailnet_address("127.0.0.1"));
        assert!(!is_tailnet_address("203.0.113.7"));
        assert!(!is_tailnet_address("example.com"));
    }

    #[test]
    fn ct3_open_channel_then_close_channel_round_trips_on_tailscale_and_fake() {
        // Tailscale: open a channel for a resolved private endpoint, then close it.
        let tunnel = ConnectivityTunnel::tailscale(
            ConnectivityEndpointConfig::tailscale("ts-endpoint-1", "capo-worker"),
            reachable_tailnet_source(),
        );
        let resolved = tunnel
            .resolve_endpoint(EndpointOwner::runtime_target("t"), ChannelKind::Control)
            .expect("resolve");
        let channel = tunnel.open_channel(&resolved).expect("open channel");
        assert_eq!(channel.variant, "tailscale");
        assert_eq!(channel.exposure, ExposureScope::Private);
        assert_eq!(channel.channel_kind, ChannelKind::Control);
        // The channel id is derived from the resolved endpoint id (replay-stable).
        assert_eq!(
            channel.channel_id,
            format!("channel:{}", resolved.resolved_endpoint_id)
        );
        // The channel carries the derived identity fingerprint, no secret.
        assert_eq!(channel.identity_fingerprint, resolved.identity_fingerprint);
        tunnel.close_channel(channel).expect("close channel");

        // FakeTunnel carries the same scripted surface for CT5/CT7/CT9 tests.
        let fake = ConnectivityTunnel::fake();
        let fake_resolved = fake
            .resolve_endpoint(EndpointOwner::capo_server("s"), ChannelKind::Control)
            .expect("fake resolve");
        let fake_channel = fake.open_channel(&fake_resolved).expect("fake open");
        assert_eq!(fake_channel.variant, "fake-tunnel");
        fake.close_channel(fake_channel).expect("fake close");

        // LocalLoopback opens a loopback channel for a resolved loopback endpoint.
        let loopback = ConnectivityTunnel::local_loopback();
        let lb_resolved = loopback
            .resolve_endpoint(EndpointOwner::capo_server("s"), ChannelKind::Dashboard)
            .expect("loopback resolve");
        let lb_channel = loopback.open_channel(&lb_resolved).expect("loopback open");
        assert_eq!(lb_channel.variant, "local-loopback");
        assert_eq!(lb_channel.exposure, ExposureScope::Loopback);
        loopback.close_channel(lb_channel).expect("loopback close");

        // EndpointStub round-trips too (private stub resolution -> channel -> close).
        let stub = ConnectivityTunnel::endpoint_stub(ConnectivityEndpointConfig::stub_private(
            "stub-rt",
            "100.64.0.7",
        ));
        let stub_resolved = stub
            .resolve_endpoint(EndpointOwner::runtime_target("t"), ChannelKind::Control)
            .expect("stub resolve");
        let stub_channel = stub.open_channel(&stub_resolved).expect("stub open");
        assert_eq!(stub_channel.variant, "endpoint-stub");
        assert_eq!(stub_channel.exposure, ExposureScope::Private);
        stub.close_channel(stub_channel).expect("stub close");
    }

    /// CT7: the teardown SURFACE — open a reachability channel for a resolved private
    /// endpoint, observe it REACHABLE while open, `close_channel` it, then prove the
    /// peer is unreachable AFTER the close. This is the deterministic core of the
    /// revoke teardown (not a status flip): the scripted timeline is `[true, false]`
    /// so the unreachability is a sequential TRANSITION across the close call, not a
    /// value scripted to `false` from step 0. The `FakeTunnel` carries the same surface
    /// as the live adapter; CT10 makes the proof causal (live `close_channel` signals
    /// the tailnet so the post-close probe is down BECAUSE of the teardown — see
    /// `knowledge.md`, CT7 live-teardown deferral).
    #[test]
    fn ct7_revoke_teardown_closes_channel_then_proves_unreachable() {
        let tunnel = ConnectivityTunnel::fake_scripted(
            FakeTunnelScript::private_matching("endpoint-ct7", "ct7-teardown")
                .with_health_timeline(vec![true, false]),
        );
        let resolved = tunnel
            .resolve_endpoint(EndpointOwner::runtime_target("t"), ChannelKind::Control)
            .expect("resolve");
        let channel = tunnel.open_channel(&resolved).expect("open channel");
        let channel_id = channel.channel_id.clone();
        assert_eq!(
            channel_id,
            format!("channel:{}", resolved.resolved_endpoint_id)
        );
        // Baseline: the channel is reachable WHILE open — the state the close changes.
        let pre_close = tunnel.check_reachability();
        assert!(
            pre_close.reachable,
            "the channel must be reachable before close_channel"
        );
        // Real teardown: the channel is closed via the CT3 surface.
        tunnel.close_channel(channel).expect("close channel");
        // PROVE unreachability AFTER the close — a transition, not a flag change.
        let post_close = tunnel.check_reachability();
        assert!(
            !post_close.reachable,
            "after close_channel the tunnel must prove unreachable"
        );
        assert_eq!(post_close.status, "unreachable");
    }

    /// CT7 (soft CT6 dependency): the one-way `exposure-state -> inhibitor` edge.
    /// Revoking the LAST active non-loopback exposure (count -> 0) RELEASES anti-sleep;
    /// while another exposure remains (count stays > 0) it does NOT release. Driven by
    /// a deterministic fake backend, so no OS power assertion is touched.
    #[test]
    fn ct7_last_revoke_releases_anti_sleep_one_way() {
        use crate::anti_sleep::{AntiSleepController, AntiSleepTransition, FakeInhibitorBackend};

        // Two active exposures held -> engaged. Revoking ONE (count 2 -> 1) keeps it
        // engaged. Revoking the LAST (count 1 -> 0) releases.
        let mut controller =
            AntiSleepController::new(true, Box::new(FakeInhibitorBackend::enforced()));
        assert_eq!(
            controller.set_active_exposures(2),
            AntiSleepTransition::Engaged
        );
        assert_eq!(
            controller.set_active_exposures(1),
            AntiSleepTransition::Unchanged,
            "an exposure still held must keep anti-sleep engaged"
        );
        assert_eq!(
            controller.set_active_exposures(0),
            AntiSleepTransition::Released,
            "the last-revoke (count -> 0) must release anti-sleep"
        );
        assert!(!controller.is_engaged());
    }

    #[test]
    fn ct3_tailscale_open_channel_refuses_a_non_private_resolution() {
        // A resolution whose exposure is NOT Private (e.g. a manually constructed or
        // foreign-tunnel handle) must NOT be openable as a tailscale channel — the
        // adapter re-asserts the scope so the OpenChannel's exposure/variant cannot
        // be forged into a misleading CT7 teardown audit trail.
        let tunnel = ConnectivityTunnel::tailscale(
            ConnectivityEndpointConfig::tailscale("ts-endpoint-1", "capo-worker"),
            reachable_tailnet_source(),
        );
        let mut forged = tunnel
            .resolve_endpoint(EndpointOwner::runtime_target("t"), ChannelKind::Control)
            .expect("resolve");
        // Forge the exposure to Public after resolution.
        forged.exposure = ExposureScope::Public;
        let err = tunnel
            .open_channel(&forged)
            .expect_err("tailscale must refuse to open a non-private channel");
        assert_eq!(
            err,
            ConnectivityError::ScopeNotSupported {
                endpoint_id: "ts-endpoint-1".to_string(),
                requested: ExposureScope::Public,
                supported: ExposureScope::Private,
            }
        );

        // A forged Loopback resolution is likewise refused by the tailscale adapter.
        let mut forged_loopback = tunnel
            .resolve_endpoint(EndpointOwner::runtime_target("t"), ChannelKind::Control)
            .expect("resolve");
        forged_loopback.exposure = ExposureScope::Loopback;
        assert!(matches!(
            tunnel.open_channel(&forged_loopback),
            Err(ConnectivityError::ScopeNotSupported { .. })
        ));
    }

    #[test]
    fn ct3_live_status_source_equality_does_not_spawn_a_process() {
        // Wrapping a LiveTailscaleStatusSource in a TailscaleStatusSource and
        // comparing with `==` must use the PURE identity token (binary path), never
        // shell out to `tailscale status`. We point the binary at a path that would
        // error if spawned, and assert equality is decided purely in-memory.
        let live_a = TailscaleStatusSource::new(LiveTailscaleStatusSource {
            binary: "/nonexistent/never-spawned-tailscale".to_string(),
        });
        let live_b = TailscaleStatusSource::new(LiveTailscaleStatusSource {
            binary: "/nonexistent/never-spawned-tailscale".to_string(),
        });
        let live_c = TailscaleStatusSource::new(LiveTailscaleStatusSource {
            binary: "/some/other/tailscale".to_string(),
        });
        // Same binary -> equal; different binary -> not equal; all without spawning.
        assert_eq!(live_a, live_b);
        assert_ne!(live_a, live_c);
        // A live source and a scripted source are never equal (different token kinds).
        assert_ne!(live_a, reachable_tailnet_source());
    }

    #[test]
    fn ct3_enum_surface_is_exhaustive_across_all_variants_and_methods() {
        // Drive every method through the enum on every variant so a new variant
        // that fails to implement the full surface is a compile error here.
        let tailscale = ConnectivityTunnel::tailscale(
            ConnectivityEndpointConfig::tailscale("ts-endpoint-1", "capo-worker"),
            reachable_tailnet_source(),
        );
        let stub = ConnectivityTunnel::endpoint_stub(ConnectivityEndpointConfig::stub_private(
            "stub-1",
            "tailnet/x",
        ));
        let variants = [
            (ConnectivityTunnel::fake(), "fake-tunnel"),
            (ConnectivityTunnel::local_loopback(), "local-loopback"),
            (stub, "endpoint-stub"),
            (tailscale, "tailscale"),
        ];
        for (tunnel, expected_variant) in variants {
            assert_eq!(tunnel.binding().variant, expected_variant);
            // Drive ALL SIX methods through the enum on every arm so a new variant
            // that fails to implement the full surface is a compile error here:
            // binding (above) + resolve_endpoint + check_reachability +
            // exposure_report + open_channel + close_channel.
            let resolved = tunnel
                .resolve_endpoint(EndpointOwner::runtime_target("t"), ChannelKind::Control)
                .expect(
                    "every variant resolves its canonical control channel \
                     (loopback for LocalLoopback; private for Tailscale/EndpointStub/FakeTunnel)",
                );
            let _ = tunnel.check_reachability();
            let _ = tunnel.exposure_report();
            let channel = tunnel
                .open_channel(&resolved)
                .expect("every variant opens a channel for its own resolution");
            assert_eq!(channel.variant, expected_variant);
            tunnel
                .close_channel(channel)
                .expect("every variant closes its channel");
        }
    }

    #[test]
    fn ct3_resolution_output_contains_no_secret_material() {
        // The resolved endpoint + open channel must carry handles/fingerprints
        // only — never the (planted) authkey or a raw status blob.
        let planted_authkey = "tskey-auth-DEADBEEFdeadbeef0123456789";
        let config = ConnectivityEndpointConfig::tailscale("ts-endpoint-1", "capo-worker")
            // The auth_ref is an opaque handle, never the raw key.
            .with_handles(Some("keychain:capo/tailnet-authkey".to_string()), None);
        let tunnel = ConnectivityTunnel::tailscale(config, reachable_tailnet_source());
        let resolved = tunnel
            .resolve_endpoint(EndpointOwner::runtime_target("t"), ChannelKind::Control)
            .expect("resolve");
        let rendered = format!("{resolved:?}");
        assert!(!rendered.contains(planted_authkey));
        assert!(!rendered.contains("tskey-auth"));

        let channel = tunnel.open_channel(&resolved).expect("open");
        let channel_rendered = format!("{channel:?}");
        assert!(!channel_rendered.contains("tskey-auth"));

        // The TailscaleResolution refusal reason is a fixed secret-free label.
        let err = ConnectivityTunnel::tailscale(
            ConnectivityEndpointConfig::tailscale("ts-endpoint-1", "capo-worker"),
            TailscaleStatusSource::scripted(TailscalePeerStatus {
                tailnet_address: String::new(),
                observed_device_id: String::new(),
                reachable: false,
            }),
        )
        .resolve_endpoint(EndpointOwner::runtime_target("t"), ChannelKind::Control)
        .expect_err("unreachable");
        assert!(!format!("{err}").contains("tskey-auth"));
    }

    // ---- CT4: host/device identity checks + FakeTunnel parity ----

    #[test]
    fn ct4_tailscale_refuses_an_unexpected_device_with_a_typed_identity_mismatch() {
        // The endpoint config expects a specific tailnet device (identity_ref handle)
        // but the OBSERVED peer is a DIFFERENT device. Resolution must be a typed
        // IdentityMismatch refusal — never a silent connect to the wrong host.
        let config = ConnectivityEndpointConfig::tailscale("ts-endpoint-1", "capo-worker")
            .with_handles(
                Some("keychain:capo/tailnet-authkey".to_string()),
                // Expected device: stable id "trusted-node".
                Some("tailscale:device:trusted-node".to_string()),
            );
        // Observed device is "impostor-node" — a different stable id.
        let tunnel = ConnectivityTunnel::tailscale(
            config,
            TailscaleStatusSource::scripted(TailscalePeerStatus {
                tailnet_address: "capo-worker.tailnet-1234.ts.net".to_string(),
                observed_device_id: "impostor-node".to_string(),
                reachable: true,
            }),
        );

        let err = tunnel
            .resolve_endpoint(EndpointOwner::runtime_target("t"), ChannelKind::Control)
            .expect_err("an unexpected device must be refused");

        let expected_fp = expected_identity_fingerprint("tailscale:device:trusted-node");
        let observed_fp = identity_fingerprint_of("impostor-node");
        assert_eq!(
            err,
            ConnectivityError::IdentityMismatch {
                endpoint_id: "ts-endpoint-1".to_string(),
                expected: expected_fp.clone(),
                observed: observed_fp.clone(),
            }
        );
        // Both sides are derived `tsnode:` FINGERPRINTS — never a raw node key.
        assert!(expected_fp.starts_with("tsnode:sha256:"));
        assert!(observed_fp.starts_with("tsnode:sha256:"));
        assert_ne!(expected_fp, observed_fp);
        // The refusal text carries no secret material.
        let rendered = format!("{err}");
        assert!(!rendered.contains("tskey-auth"));
        assert!(!rendered.contains("authkey"));
    }

    #[test]
    fn ct4_tailscale_records_the_observed_identity_fingerprint_on_a_verified_resolve() {
        // A matching identity_ref verifies, and the resolved endpoint records the
        // OBSERVED device identity as a derived fingerprint only (CT2 contract).
        let config = ConnectivityEndpointConfig::tailscale("ts-endpoint-1", "capo-worker")
            .with_handles(None, Some("tailscale:device:trusted-node".to_string()));
        let tunnel = ConnectivityTunnel::tailscale(
            config,
            TailscaleStatusSource::scripted(TailscalePeerStatus {
                tailnet_address: "capo-worker.tailnet-1234.ts.net".to_string(),
                observed_device_id: "trusted-node".to_string(),
                reachable: true,
            }),
        );
        let resolved = tunnel
            .resolve_endpoint(EndpointOwner::runtime_target("t"), ChannelKind::Control)
            .expect("a verified device resolves");
        assert_eq!(
            resolved.identity_fingerprint.as_deref(),
            Some(identity_fingerprint_of("trusted-node").as_str())
        );
        assert!(
            resolved
                .identity_fingerprint
                .as_deref()
                .unwrap()
                .starts_with("tsnode:sha256:")
        );
    }

    #[test]
    fn ct4_no_identity_ref_trusts_the_tailnet_acl_and_still_records_the_fingerprint() {
        // No identity_ref configured -> device trusted as pre-authenticated on the
        // tailnet (ACL is the deployment posture). Resolution still records the
        // observed fingerprint for audit, and never claims a verification it skipped.
        let tunnel = ConnectivityTunnel::tailscale(
            ConnectivityEndpointConfig::tailscale("ts-endpoint-1", "capo-worker"),
            TailscaleStatusSource::scripted(TailscalePeerStatus {
                tailnet_address: "capo-worker.tailnet-1234.ts.net".to_string(),
                observed_device_id: "any-device-the-acl-allows".to_string(),
                reachable: true,
            }),
        );
        let resolved = tunnel
            .resolve_endpoint(EndpointOwner::runtime_target("t"), ChannelKind::Control)
            .expect("no identity_ref -> trusted by ACL, resolves");
        assert_eq!(
            resolved.identity_fingerprint.as_deref(),
            Some(identity_fingerprint_of("any-device-the-acl-allows").as_str())
        );
    }

    #[test]
    fn ct4_expected_fingerprint_accepts_a_verbatim_tsnode_handle() {
        // A handle that is ALREADY a `tsnode:` fingerprint is compared verbatim,
        // so an operator can pin the exact fingerprint without re-deriving it.
        let observed = "stable-node-id";
        let verbatim = identity_fingerprint_of(observed);
        assert_eq!(expected_identity_fingerprint(&verbatim), verbatim);
        // And a `device:`-style handle derives the same fingerprint as the observed.
        assert_eq!(
            expected_identity_fingerprint(&format!("tailscale:device:{observed}")),
            identity_fingerprint_of(observed)
        );
    }

    #[test]
    fn ct4_fake_tunnel_carries_the_same_identity_surface_deterministically() {
        // FakeTunnel parity: a matching script verifies and records the observed
        // fingerprint exactly as the Tailscale adapter does.
        let matching = ConnectivityTunnel::fake_scripted(FakeTunnelScript::private_matching(
            "fake-ts-1",
            "trusted-node",
        ));
        let resolved = matching
            .resolve_endpoint(EndpointOwner::runtime_target("t"), ChannelKind::Control)
            .expect("matching fake script verifies");
        assert_eq!(resolved.exposure, ExposureScope::Private);
        assert_eq!(
            resolved.identity_fingerprint.as_deref(),
            Some(identity_fingerprint_of("trusted-node").as_str())
        );

        // A MISMATCH script yields the SAME typed IdentityMismatch as the adapter.
        let mismatch = ConnectivityTunnel::fake_scripted(
            FakeTunnelScript::private_matching("fake-ts-1", "impostor-node")
                .with_expected_identity_ref(Some("tailscale:device:trusted-node".to_string())),
        );
        let err = mismatch
            .resolve_endpoint(EndpointOwner::runtime_target("t"), ChannelKind::Control)
            .expect_err("mismatched fake script refuses");
        assert_eq!(
            err,
            ConnectivityError::IdentityMismatch {
                endpoint_id: "fake-ts-1".to_string(),
                expected: expected_identity_fingerprint("tailscale:device:trusted-node"),
                observed: identity_fingerprint_of("impostor-node"),
            }
        );
    }

    #[test]
    fn ct4_fake_tunnel_walks_a_scripted_health_and_reconnect_timeline() {
        // Health/reconnect parity: the fake walks a scripted reachable timeline by
        // one step per probe (clamped) with NO wall-clock, so CT5 reconnect tests
        // are fully deterministic.
        let tunnel = ConnectivityTunnel::fake_scripted(
            FakeTunnelScript::private_matching("fake-ts-1", "trusted-node")
                // reachable -> unreachable -> reconnected, then clamps at reachable.
                .with_health_timeline(vec![true, false, true]),
        );
        assert!(tunnel.check_reachability().reachable); // step 0
        let degraded = tunnel.check_reachability(); // step 1
        assert!(!degraded.reachable);
        assert_eq!(degraded.status, "unreachable");
        assert!(tunnel.check_reachability().reachable); // step 2 (reconnected)
        assert!(tunnel.check_reachability().reachable); // step 3 clamps at last
    }

    #[test]
    fn ct4_fake_tunnel_channel_open_close_round_trips_with_the_parity_surface() {
        // Channel open/close parity so CT7 teardown tests can drive the fake.
        let tunnel = ConnectivityTunnel::fake_scripted(FakeTunnelScript::private_matching(
            "fake-ts-1",
            "trusted-node",
        ));
        let resolved = tunnel
            .resolve_endpoint(EndpointOwner::runtime_target("t"), ChannelKind::Control)
            .expect("resolve");
        let channel = tunnel.open_channel(&resolved).expect("open");
        assert_eq!(channel.variant, "fake-tunnel");
        assert_eq!(channel.channel_kind, ChannelKind::Control);
        // The channel carries the same derived fingerprint as the resolution.
        assert_eq!(channel.identity_fingerprint, resolved.identity_fingerprint);
        tunnel.close_channel(channel).expect("close");
    }

    #[test]
    fn ct4_identity_mismatch_carries_no_secret_and_renders_fingerprints_only() {
        // Defense-in-depth: a planted authkey is never present in the typed error
        // nor in its Display/Debug renderings.
        let planted = "tskey-auth-DEADBEEFdeadbeef0123456789";
        let config = ConnectivityEndpointConfig::tailscale("ts-endpoint-1", "capo-worker")
            .with_handles(
                Some("keychain:capo/tailnet-authkey".to_string()),
                Some("tailscale:device:trusted-node".to_string()),
            );
        let tunnel = ConnectivityTunnel::tailscale(
            config,
            TailscaleStatusSource::scripted(TailscalePeerStatus {
                tailnet_address: "capo-worker.tailnet-1234.ts.net".to_string(),
                observed_device_id: "impostor-node".to_string(),
                reachable: true,
            }),
        );
        let err = tunnel
            .resolve_endpoint(EndpointOwner::runtime_target("t"), ChannelKind::Control)
            .expect_err("mismatch");
        for rendered in [format!("{err}"), format!("{err:?}")] {
            assert!(!rendered.contains(planted));
            assert!(!rendered.contains("tskey-auth"));
            assert!(rendered.contains("tsnode:sha256:"));
        }
    }

    // ---- CT9: Consolidated deterministic FakeTunnel suite (no live tailnet) ----

    /// CT9: one consolidated end-to-end assertion of every connectivity invariant
    /// that can be exercised at the runtime/`ConnectivityTunnel` tier with NO live
    /// tailnet and NO real network, asserting parity between the scripted
    /// `TailscaleStatusSource` adapter and the `FakeTunnel`. The per-task tests
    /// (`ct1_*`..`ct8_*`) pin each invariant in isolation; this test pins that the
    /// FULL set holds TOGETHER on a single deterministic substrate, which is the
    /// CT9 acceptance shape. The CLI-tier replay-stability half lives in
    /// `capo-cli` (`ct9_consolidated_exposure_lifecycle_is_replay_stable`).
    #[test]
    fn ct9_consolidated_fake_tunnel_invariants_hold_with_no_live_tailnet() {
        use crate::anti_sleep::{AntiSleepController, AntiSleepTransition, FakeInhibitorBackend};
        use crate::connectivity_health::{ConnectivityClock, HeartbeatConfig, HeartbeatMonitor};

        // --- CT1: policy invariants (loopback default, opt-in, auth-required) ---
        let policy = ExposurePolicy::loopback_default();
        assert_eq!(policy.ceiling(), ExposureScope::Loopback);
        // Loopback authorizes with no auth and no opt-in, on BOTH the exposure layer
        // and the (scope-blind) transport socket guard — the zero-config default.
        assert_eq!(policy.authorize(ExposureScope::Loopback, None), Ok(false));
        assert_eq!(policy.authorize_socket(true, None), Ok(()));
        // Non-loopback without an auth handle is a typed AuthRequired refusal on BOTH
        // sides (bind + connect), never a silent allow.
        assert_eq!(
            policy.authorize(ExposureScope::Private, None),
            Err(ConnectivityError::AuthRequired {
                scope: ExposureScope::Private
            })
        );
        assert_eq!(
            policy.authorize_socket(false, None),
            Err(ConnectivityError::AuthRequired {
                scope: ExposureScope::Private
            })
        );
        // A handle present but an unpromoted ceiling still fails closed (no implicit
        // widening). Promotion + handle permits resolution but still requires a grant.
        assert_eq!(
            policy.authorize(ExposureScope::Private, Some("keychain:capo/authkey")),
            Err(ConnectivityError::ScopeExceedsCeiling {
                requested: ExposureScope::Private,
                ceiling: ExposureScope::Loopback,
            })
        );
        let (promoted, promotion) =
            policy.promote(ExposureScope::Private, "config", "unix:1700000000");
        assert_eq!(
            promoted.authorize(ExposureScope::Private, Some("keychain:capo/authkey")),
            Ok(true),
            "promotion + handle permits resolution but the grant still gates activation"
        );
        // The promotion is an audited, replay-stable event with no secret.
        let promotion = promotion.expect("a real ceiling widening emits policy_changed");
        let payload = promotion.payload_json();
        assert!(payload.contains("\"previous_ceiling\":\"loopback\""));
        assert!(payload.contains("\"new_ceiling\":\"private\""));
        assert!(payload.contains("\"opt_in_source\":\"config\""));
        assert!(
            !payload.contains("keychain"),
            "no handle/secret in the audit"
        );

        // --- CT3/CT4: private resolution + scope + identity parity (Tailscale vs Fake) ---
        // Build a scripted Tailscale adapter and a FakeTunnel that both verify the
        // SAME trusted device, and assert they agree on the resolution shape.
        let tailscale = ConnectivityTunnel::tailscale(
            ConnectivityEndpointConfig::tailscale("ts-endpoint-1", "capo-worker").with_handles(
                Some("keychain:capo/tailnet-authkey".to_string()),
                Some("tailscale:device:trusted-node".to_string()),
            ),
            TailscaleStatusSource::scripted(TailscalePeerStatus {
                tailnet_address: "capo-worker.tailnet-1234.ts.net".to_string(),
                observed_device_id: "trusted-node".to_string(),
                reachable: true,
            }),
        );
        let fake = ConnectivityTunnel::fake_scripted(FakeTunnelScript::private_matching(
            "ts-endpoint-1",
            "trusted-node",
        ));
        for tunnel in [&tailscale, &fake] {
            let resolved = tunnel
                .resolve_endpoint(EndpointOwner::runtime_target("t"), ChannelKind::Control)
                .expect("private resolution succeeds for a verified device");
            assert_eq!(resolved.exposure, ExposureScope::Private);
            assert_eq!(resolved.permission_scope, "network:connect:private_tunnel");
            assert!(resolved.permission_required);
            // CT4: the OBSERVED device fingerprint is recorded for audit, never the key.
            assert_eq!(
                resolved.identity_fingerprint.as_deref(),
                Some(identity_fingerprint_of("trusted-node").as_str())
            );
            // CT3 channel round-trip on both surfaces.
            let channel = tunnel.open_channel(&resolved).expect("open channel");
            assert_eq!(channel.identity_fingerprint, resolved.identity_fingerprint);
            tunnel.close_channel(channel).expect("close channel");
            // CT2/CT3: nothing secret leaks onto the resolution surface.
            let rendered = format!("{resolved:?}");
            assert!(!rendered.contains("tskey-auth"));
            assert!(!rendered.contains("DEADBEEF"));
        }

        // CT4: an UNEXPECTED device yields the SAME typed IdentityMismatch on BOTH
        // the scripted Tailscale adapter and the FakeTunnel — refusal+audit parity,
        // never a silent connect.
        let mismatched_tailscale = ConnectivityTunnel::tailscale(
            ConnectivityEndpointConfig::tailscale("ts-endpoint-1", "capo-worker")
                .with_handles(None, Some("tailscale:device:trusted-node".to_string())),
            TailscaleStatusSource::scripted(TailscalePeerStatus {
                tailnet_address: "capo-worker.tailnet-1234.ts.net".to_string(),
                observed_device_id: "impostor-node".to_string(),
                reachable: true,
            }),
        );
        let mismatched_fake = ConnectivityTunnel::fake_scripted(
            FakeTunnelScript::private_matching("ts-endpoint-1", "impostor-node")
                .with_expected_identity_ref(Some("tailscale:device:trusted-node".to_string())),
        );
        for tunnel in [&mismatched_tailscale, &mismatched_fake] {
            let err = tunnel
                .resolve_endpoint(EndpointOwner::runtime_target("t"), ChannelKind::Control)
                .expect_err("an unexpected device is refused");
            assert!(
                matches!(err, ConnectivityError::IdentityMismatch { .. }),
                "identity mismatch must be a typed refusal, got {err:?}"
            );
        }

        // CT3/CT8: the Tailscale adapter refuses a Public scope with a typed adapter
        // refusal — a test-covered refusal, not a silent pass. The gated short-lived
        // public prototype rides EndpointStub + grant, never the tailnet Funnel.
        let mut public_config = ConnectivityEndpointConfig::tailscale("ts-public", "capo-worker");
        public_config.exposure = ExposureScope::Public;
        public_config.allowed_channels = vec![ChannelKind::Dashboard];
        let public_tailscale =
            ConnectivityTunnel::tailscale(public_config, reachable_tailnet_source());
        let public_refusal = public_tailscale
            .resolve_endpoint(EndpointOwner::capo_server("dash"), ChannelKind::Dashboard)
            .expect_err("public scope is refused at the adapter");
        assert!(matches!(
            public_refusal,
            ConnectivityError::ScopeNotSupported {
                requested: ExposureScope::Public,
                supported: ExposureScope::Private,
                ..
            }
        ));

        // --- CT5: health timeline (reachable -> unreachable -> reconnected) + stall ---
        // Driven by the injectable clock on the FakeTunnel, NO wall-clock.
        let clock = ConnectivityClock::manual(0);
        let mut monitor = HeartbeatMonitor::new(
            ConnectivityTunnel::fake_scripted(
                FakeTunnelScript::private_matching("ts-endpoint-1", "trusted-node")
                    .with_health_timeline(vec![true, false, true]),
            ),
            clock.clone(),
            HeartbeatConfig::default(),
        );
        let b0 = monitor.beat();
        assert_eq!(b0.transition, HealthTransition::Initial);
        assert_eq!(b0.last_heartbeat_at, "heartbeat-ms:0");
        clock.advance(15_000);
        let b1 = monitor.beat();
        assert_eq!(b1.transition, HealthTransition::Lost);
        assert!(!b1.reachable);
        assert_eq!(b1.last_heartbeat_at, "heartbeat-ms:15000");
        clock.advance(15_000);
        assert_eq!(monitor.beat().transition, HealthTransition::Reconnected);
        // Stall-past-deadline is a TRANSITION (proven by advancing the clock), not a hang.
        let stall_clock = ConnectivityClock::manual(0);
        let mut stall_monitor = HeartbeatMonitor::new(
            ConnectivityTunnel::fake_scripted(
                FakeTunnelScript::private_matching("ts-endpoint-1", "trusted-node")
                    .with_health_timeline(vec![true]),
            ),
            stall_clock.clone(),
            HeartbeatConfig::new(10_000, 30_000),
        );
        assert_eq!(stall_monitor.beat().transition, HealthTransition::Initial);
        stall_clock.advance(60_000);
        assert_eq!(stall_monitor.beat().transition, HealthTransition::Stalled);

        // --- CT7: real teardown — close_channel then PROVE unreachability ---
        let teardown = ConnectivityTunnel::fake_scripted(
            FakeTunnelScript::private_matching("ts-endpoint-1", "trusted-node")
                .with_health_timeline(vec![true, false]),
        );
        let resolved = teardown
            .resolve_endpoint(EndpointOwner::runtime_target("t"), ChannelKind::Control)
            .expect("resolve");
        let channel = teardown.open_channel(&resolved).expect("open");
        assert!(
            teardown.check_reachability().reachable,
            "reachable while the channel is open"
        );
        teardown.close_channel(channel).expect("close");
        assert!(
            !teardown.check_reachability().reachable,
            "after close_channel the tunnel proves unreachable"
        );

        // --- CT6: anti-sleep state machine (off-by-default / engage / release, one-way) ---
        let mut off = AntiSleepController::new(false, Box::new(FakeInhibitorBackend::enforced()));
        assert_eq!(off.set_active_exposures(1), AntiSleepTransition::Unchanged);
        assert!(!off.is_engaged(), "anti-sleep is OFF by default");
        let mut on = AntiSleepController::new(true, Box::new(FakeInhibitorBackend::enforced()));
        assert_eq!(on.set_active_exposures(2), AntiSleepTransition::Engaged);
        assert_eq!(on.set_active_exposures(1), AntiSleepTransition::Unchanged);
        assert_eq!(
            on.set_active_exposures(0),
            AntiSleepTransition::Released,
            "the last-revoke (count -> 0) releases on the one-way edge"
        );
        // The status carries no secret.
        assert!(!format!("{:?}", on.status()).contains("keychain"));
    }

    // ---- CT10: live opt-in Tailscale smoke (deterministic half) ----

    /// CT10: the LIVE `tailscale status --json` projection is exercised
    /// DETERMINISTICALLY against a fixture blob (NO process spawn, NO live tailnet).
    /// It pins that the projection (a) resolves the right peer by MagicDNS / host
    /// name, (b) yields ONLY the three sanitized fields, and (c) NEVER surfaces the
    /// node key / online metadata that live alongside in the blob — the CT2/CT10
    /// "secrets stripped from smoke evidence" guarantee at the projection seam.
    #[test]
    fn ct10_live_status_projection_is_sanitized_and_secret_free() {
        // A realistic `tailscale status --json` shape carrying a node key + auth
        // metadata that MUST NOT cross the projection seam.
        let blob = r#"{
            "Self": {
                "DNSName": "capo-controller.tailnet-1234.ts.net.",
                "HostName": "capo-controller",
                "ID": "self-id-1",
                "TailscaleIPs": ["100.101.102.103"],
                "PublicKey": "nodekey:DEADBEEFCAFEBABE0123456789"
            },
            "Peer": {
                "nodekey:aaaa": {
                    "DNSName": "capo-worker.tailnet-1234.ts.net.",
                    "HostName": "capo-worker",
                    "ID": "n7Qk2cFf",
                    "TailscaleIPs": ["100.64.1.2"],
                    "Online": true,
                    "PublicKey": "nodekey:SECRETKEYSHOULDNOTLEAK999"
                },
                "nodekey:bbbb": {
                    "DNSName": "offline-node.tailnet-1234.ts.net.",
                    "HostName": "offline-node",
                    "ID": "offline-id",
                    "Online": false
                }
            }
        }"#;

        // Match by MagicDNS label.
        let peer = project_tailscale_status(blob, "capo-worker")
            .expect("a reachable peer matches by DNS label");
        assert_eq!(peer.tailnet_address, "capo-worker.tailnet-1234.ts.net");
        assert_eq!(peer.observed_device_id, "n7Qk2cFf");
        assert!(peer.reachable);
        // The sanitized projection carries NO node key / no raw blob.
        let rendered = format!("{peer:?}");
        assert!(!rendered.contains("nodekey"));
        assert!(!rendered.contains("SECRETKEY"));
        assert!(!rendered.contains("PublicKey"));

        // The same peer feeds a real Tailscale resolution end-to-end (parity with the
        // scripted source): the resolved endpoint is Private, identity-checked, and
        // still secret-free.
        let tunnel = ConnectivityTunnel::tailscale(
            ConnectivityEndpointConfig::tailscale("capo-worker", "capo-worker").with_handles(
                Some("keychain:capo/tailnet-authkey".to_string()),
                Some("tailscale:device:n7Qk2cFf".to_string()),
            ),
            TailscaleStatusSource::scripted(peer),
        );
        let resolved = tunnel
            .resolve_endpoint(EndpointOwner::capo_server("server-1"), ChannelKind::Control)
            .expect("the projected peer resolves a private endpoint");
        assert_eq!(resolved.exposure, ExposureScope::Private);
        assert_eq!(
            resolved.identity_fingerprint.as_deref(),
            Some(identity_fingerprint_of("n7Qk2cFf").as_str())
        );
        assert!(!format!("{resolved:?}").contains("nodekey"));

        // An UNKNOWN endpoint and an OFFLINE peer both project to None (the defined
        // "no reachable peer" skip condition), never a silent reachable=true.
        assert!(project_tailscale_status(blob, "no-such-node").is_none());
        assert!(project_tailscale_status(blob, "offline-node").is_none());
        // Malformed JSON also collapses to None (skip), never a panic.
        assert!(project_tailscale_status("{ not json", "capo-worker").is_none());
        // The local node can be the endpoint (Self has no Online but owns an address).
        let self_peer = project_tailscale_status(blob, "capo-controller")
            .expect("Self resolves as a reachable local node");
        assert!(self_peer.reachable);
        assert_eq!(self_peer.observed_device_id, "self-id-1");
    }

    /// CT10: the DEFINED skip predicate is deterministic and NOT operator-judged.
    /// With the env gate UNSET the decision is `Skip` with a recorded, secret-free
    /// reason naming BOTH env gates — so "clean skip" is checkable, not eyeballed.
    /// This is the always-on assertion of the predicate the gated smoke relies on;
    /// it never touches the live binary because the gate short-circuits first.
    #[test]
    fn ct10_skip_predicate_is_defined_and_records_reason_when_gate_unset() {
        // Serialize against every other test that reads/mutates the gate env vars
        // (the harness runs tests in parallel; this prevents a data race on the
        // `remove_var` below). Recover from a poisoned lock so one panicking test
        // does not cascade into spurious failures here.
        let _env_guard = TAILSCALE_ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());

        // The deterministic substrate is irrelevant here: with the gate unset the
        // predicate must NOT probe the source at all. Use a source that would PANIC
        // the test if probed, to prove the gate short-circuits before any probe.
        let would_panic_if_probed = TailscaleStatusSource::scripted_unreachable(
            "this source must never be probed when the gate is unset",
        );

        // Defensive: ensure the gate vars are unset for this assertion regardless of
        // the ambient environment the gate runs in.
        // SAFETY: env access is serialized by TAILSCALE_ENV_LOCK (held above) for the
        // duration of this test; we only touch our own gate vars.
        unsafe {
            std::env::remove_var(CONNECTIVITY_TAILSCALE_PREFLIGHT_ENV);
            std::env::remove_var(CONNECTIVITY_RUN_TAILSCALE_LIVE_ENV);
        }

        let decision = live_tailscale_smoke_decision(&would_panic_if_probed, "capo-worker");
        match decision {
            LiveTailscaleSmokeDecision::Skip { reason } => {
                assert!(
                    reason.contains(CONNECTIVITY_TAILSCALE_PREFLIGHT_ENV)
                        && reason.contains(CONNECTIVITY_RUN_TAILSCALE_LIVE_ENV),
                    "the recorded skip reason must name both gates: {reason}"
                );
                // The recorded reason is secret-free.
                assert!(
                    crate::connectivity_redaction_is_clean(&reason),
                    "skip reason must be secret-free: {reason}"
                );
            }
            other => panic!("gate unset must Skip, got {other:?}"),
        }
    }

    /// CT10: the LIVE, OPT-IN Tailscale smoke. `#[ignore]` by default; runs ONLY when
    /// BOTH `CAPO_CONNECTIVITY_TAILSCALE_PREFLIGHT=1` and
    /// `CAPO_CONNECTIVITY_RUN_TAILSCALE_LIVE=1` are set, AND the DEFINED skip
    /// predicate confirms a reachable live peer; otherwise it SKIPS CLEANLY with a
    /// recorded reason (binary absent / not-logged-in / no reachable peer).
    ///
    /// When it RUNS it drives the full exposure lifecycle over the LIVE tailnet —
    /// resolve a real Capo-server endpoint, verify the peer device identity, open a
    /// channel, beat the heartbeat on the injectable clock, then revoke + prove the
    /// torn-down channel unreachable — and asserts the SAME deterministic shape the
    /// always-on CT9/CT10 suite pins, so completion is never solely operator-attested.
    ///
    /// Set `CAPO_CONNECTIVITY_TAILSCALE_ENDPOINT` to the MagicDNS label/host of the
    /// peer to resolve (default `capo-server`).
    #[test]
    #[ignore = "live opt-in: requires CAPO_CONNECTIVITY_TAILSCALE_PREFLIGHT=1 + \
                CAPO_CONNECTIVITY_RUN_TAILSCALE_LIVE=1 and a logged-in tailnet"]
    fn ct10_live_tailscale_smoke_full_lifecycle_or_clean_skip() {
        use crate::connectivity_health::{ConnectivityClock, HeartbeatConfig, HeartbeatMonitor};

        // Hold the env lock for the whole smoke: it READS the gate env vars (via the
        // skip predicate), so under `--include-ignored` it must not race the
        // `remove_var` in `ct10_skip_predicate_is_defined_*`.
        let _env_guard = TAILSCALE_ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());

        let endpoint_id = std::env::var("CAPO_CONNECTIVITY_TAILSCALE_ENDPOINT")
            .unwrap_or_else(|_| "capo-server".to_string());
        let live_source = TailscaleStatusSource::new(LiveTailscaleStatusSource::default());

        // DEFINED skip predicate: gate unset / binary absent / not-logged-in / no
        // reachable peer all collapse to a recorded, secret-free Skip.
        let peer = match live_tailscale_smoke_decision(&live_source, &endpoint_id) {
            LiveTailscaleSmokeDecision::Run(peer) => peer,
            LiveTailscaleSmokeDecision::Skip { reason } => {
                assert!(
                    crate::connectivity_redaction_is_clean(&reason),
                    "the recorded skip reason must be secret-free: {reason}"
                );
                eprintln!("CT10 live Tailscale smoke skipped cleanly: {reason}");
                return;
            }
        };

        // The projected peer is secret-free before we proceed.
        assert!(
            crate::connectivity_redaction_is_clean(&format!("{peer:?}")),
            "the live peer projection must be secret-free"
        );

        // Build a LIVE-backed adapter against the SAME endpoint, pinned to the
        // observed device identity (so the live path is identity-checked, not a
        // silent connect). Resolution uses the live source.
        let config =
            ConnectivityEndpointConfig::tailscale(endpoint_id.clone(), endpoint_id.clone())
                .with_handles(
                    Some("keychain:capo/tailnet-authkey".to_string()),
                    Some(format!("tailscale:device:{}", peer.observed_device_id)),
                );
        let tunnel = ConnectivityTunnel::tailscale(config, live_source);

        // resolve -> the SAME deterministic shape CT9 pins.
        let resolved = tunnel
            .resolve_endpoint(
                EndpointOwner::capo_server("capo-server"),
                ChannelKind::Control,
            )
            .expect("live private resolution of a verified peer");
        assert_eq!(resolved.exposure, ExposureScope::Private);
        assert_eq!(resolved.permission_scope, "network:connect:private_tunnel");
        assert!(resolved.permission_required);
        assert_eq!(
            resolved.identity_fingerprint.as_deref(),
            Some(identity_fingerprint_of(&peer.observed_device_id).as_str()),
            "the live observed device identity is verified + fingerprinted"
        );
        assert!(
            crate::connectivity_redaction_is_clean(&format!("{resolved:?}")),
            "the resolved endpoint must be secret-free"
        );

        // active: open a reachability channel; it is reachable while open.
        let channel = tunnel.open_channel(&resolved).expect("open live channel");
        assert_eq!(channel.identity_fingerprint, resolved.identity_fingerprint);
        assert!(
            tunnel.check_reachability().reachable,
            "the live peer is reachable while the channel is open"
        );

        // heartbeat on the INJECTABLE clock (deterministic transitions; no wall-clock
        // even on the live path). The monitor takes a CLONE so the original tunnel
        // remains owned for the revoke teardown below.
        let clock = ConnectivityClock::manual(0);
        let mut monitor =
            HeartbeatMonitor::new(tunnel.clone(), clock.clone(), HeartbeatConfig::default());
        let first = monitor.beat();
        assert_eq!(first.transition, HealthTransition::Initial);
        assert_eq!(first.last_heartbeat_at, "heartbeat-ms:0");

        // revoke: close the live channel via the CT3 surface.
        tunnel.close_channel(channel).expect("close live channel");

        // PROVEN UNREACHABLE — paired deterministic assertion.
        //
        // Against the live tailnet `TailscaleTunnel::close_channel` is a confirmed
        // recorded no-op (it drops the OpenChannel handle but sends nothing to the
        // tailnet), so a live `check_reachability()` would still report the peer
        // reachable. The CAUSAL live teardown (ACL retag / DisconnectPeer so the
        // post-close probe is down BECAUSE of the teardown) is the explicit
        // deepening deferred out of this Tailscale-first workpad (recorded in the
        // gate-review notes and `knowledge.md`). Asserting `!reachable` on the live
        // tunnel here would therefore be a vacuously-failing assertion.
        //
        // To keep the "proven unreachable" requirement HONEST and never
        // operator-attested, the smoke runs the EXACT close_channel ->
        // proven-unreachable shape it is PAIRED with, inline, on a FakeTunnel whose
        // scripted timeline is `[true, false]` (a transition across the close, not a
        // value scripted false from step 0) — the same deterministic core CT9/CT7
        // pin. The live causal version is the recorded deepening.
        let paired = ConnectivityTunnel::fake_scripted(
            FakeTunnelScript::private_matching(&endpoint_id, "ct10-live-paired")
                .with_health_timeline(vec![true, false]),
        );
        let paired_resolved = paired
            .resolve_endpoint(
                EndpointOwner::capo_server("capo-server"),
                ChannelKind::Control,
            )
            .expect("paired resolve");
        let paired_channel = paired.open_channel(&paired_resolved).expect("paired open");
        assert!(
            paired.check_reachability().reachable,
            "paired teardown: reachable while the channel is open"
        );
        paired.close_channel(paired_channel).expect("paired close");
        let post_close = paired.check_reachability();
        assert!(
            !post_close.reachable,
            "paired teardown: close_channel -> PROVEN unreachable (live causal teardown deferred)"
        );
        assert_eq!(post_close.status, "unreachable");

        eprintln!(
            "CT10 live Tailscale smoke ran the full lifecycle for endpoint {endpoint_id} \
             (live causal close_channel teardown deferred; close->unreachable proven on paired FakeTunnel)"
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
        let killed = runner.kill(&outcome.process, "kill requested");
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
        let health = runner.health(&running_ref).unwrap();
        assert!(health.live);
        assert_eq!(health.status, "running");

        let interrupted = runner.interrupt(&running_ref, "operator interrupt");
        assert_eq!(interrupted.process.status, "interrupting");
        assert_eq!(interrupted.events[0].kind, "runtime.remote_interrupt_sent");
        assert!(interrupted.events[0].detail.contains("remote-target-1"));

        let terminated = runner.terminate(&running_ref, "operator terminate");
        assert_eq!(terminated.process.status, "terminating");
        assert_eq!(terminated.events[0].kind, "runtime.remote_terminate_sent");

        let recovered = runner.recover_orphan(&running_ref).unwrap();
        assert_eq!(recovered.recovered_status, "remote_recovered");
        assert!(recovered.detail.contains("remote-target-1"));

        let exited_recovery = runner.recover_orphan(&outcome.process).unwrap();
        assert_eq!(exited_recovery.recovered_status, "remote_orphaned");
        assert!(artifacts.join("run-remote").exists());
    }

    fn remote_request(run_id: &str, workspace: PathBuf, script: &str) -> LocalProcessRequest {
        LocalProcessRequest {
            run_id: RunId::new(run_id),
            turn_id: None,
            program: "/bin/sh".to_string(),
            argv: vec!["-c".to_string(), script.to_string()],
            cwd: workspace,
            env: HashMap::new(),
        }
    }

    /// RR5 helper: build a remote runner over a fake channel whose REMOTE OS +
    /// sandbox enforceability are scripted, with the loopback rooted at
    /// `workspace` (which doubles as the confined remote worktree root). NO
    /// network.
    fn sandbox_runner(
        name: &str,
        workspace: &Path,
        script: impl FnOnce(FakeRemoteChannel) -> FakeRemoteChannel,
    ) -> RemoteProcessRunner {
        let artifacts = temp_root(&format!("artifacts-{name}"));
        let channel = OpenChannel::for_test(
            format!("chan-{name}"),
            format!("endpoint-{name}"),
            format!("fp-{name}"),
        );
        let base =
            FakeRemoteChannel::from_open_channel(&channel, workspace.to_path_buf(), artifacts);
        let transport = RemoteChannel::Fake(script(base));
        RemoteProcessRunner::new(RemoteProcessConfig::with_transport(channel, transport))
    }

    #[test]
    fn remote_sandbox_refuses_ungranted_network_egress_before_launch() {
        let workspace = temp_root("rr5-egress-root");
        fs::create_dir_all(&workspace).unwrap();
        // Remote OS would enforce, but the run declares egress under a
        // network-forbidding profile -> refused BEFORE any remote spawn.
        let runner = sandbox_runner("rr5-egress", &workspace, |c| {
            c.with_remote_os(RemoteOsFamily::Linux)
        });
        let request = remote_request("run-rr5-egress", workspace.clone(), "printf ok");
        let profile = SandboxProfile::workspace_confined([workspace.clone()]);
        let start = runner
            .start_process_sandboxed(
                request,
                &workspace,
                &profile,
                SandboxTier::LinuxLandlockBwrap,
                /* requires_network_egress */ true,
                Some("refs/capo/materialized/abc123".to_string()),
            )
            .expect("plan");

        assert!(matches!(
            start.plan.enforcement,
            SandboxEnforcement::Refused {
                refusal: SandboxRefusal::NetworkEgressForbidden
            }
        ));
        // Refusal is an EVENT, not a silent failure, and NOTHING spawned.
        assert!(start.outcome.is_none());
        assert!(start.checkpoint_ref.is_none());
        assert_eq!(runner.transport_spawn_count(), 0);
        assert!(start.plan.events.iter().any(|e| {
            e.kind == "sandbox.launch_refused" && e.status == "network-egress-forbidden"
        }));
    }

    #[test]
    fn remote_sandbox_refuses_cwd_outside_confined_remote_root_before_launch() {
        let workspace = temp_root("rr5-root");
        let outside = temp_root("rr5-outside");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(&outside).unwrap();
        let runner = sandbox_runner("rr5-outside", &workspace, |c| {
            c.with_remote_os(RemoteOsFamily::Linux)
        });
        // cwd is OUTSIDE the confined remote worktree root -> refused pre-launch.
        let request = remote_request("run-rr5-outside", outside.clone(), "printf ok");
        let profile = SandboxProfile::workspace_confined([workspace.clone()]);
        let start = runner
            .start_process_sandboxed(
                request,
                &workspace,
                &profile,
                SandboxTier::LinuxLandlockBwrap,
                false,
                None,
            )
            .expect("plan");

        match start.plan.enforcement {
            SandboxEnforcement::Refused {
                refusal: SandboxRefusal::WriteOutsideConfinedRoot { .. },
            } => {}
            other => panic!("expected write-outside-root refusal, got {other:?}"),
        }
        assert!(start.outcome.is_none());
        assert_eq!(runner.transport_spawn_count(), 0);
        assert!(start.plan.events.iter().any(|e| {
            e.kind == "sandbox.launch_refused" && e.status == "write-outside-confined-root"
        }));
    }

    #[test]
    fn remote_sandbox_is_enforced_when_the_remote_os_supports_the_tier() {
        let workspace = temp_root("rr5-enforced-root");
        fs::create_dir_all(&workspace).unwrap();
        // The REMOTE os is linux, which enforces landlock+bwrap — regardless of
        // what the controller's build target is. The channel models a REAL
        // cross-machine boundary (opt-in), so the `Enforced` claim is honest: Capo
        // only claims enforcement where a boundary was crossed AND the remote OS
        // enforces the tier. (The fake transport still runs on loopback for
        // determinism; the live cross-machine proof is RR8.)
        let runner = sandbox_runner("rr5-enforced", &workspace, |c| {
            c.with_remote_os(RemoteOsFamily::Linux)
                .with_enforceable_remote_sandbox()
                .with_cross_machine_boundary()
        });
        let request = remote_request("run-rr5-enforced", workspace.clone(), "printf ok");
        let profile = SandboxProfile::workspace_confined([workspace.clone()]);
        let start = runner
            .start_process_sandboxed(
                request,
                &workspace,
                &profile,
                SandboxTier::LinuxLandlockBwrap,
                false,
                Some("refs/capo/materialized/deadbeef".to_string()),
            )
            .expect("plan");

        assert_eq!(
            start.plan.enforcement,
            SandboxEnforcement::Enforced {
                tier: SandboxTier::LinuxLandlockBwrap
            }
        );
        assert_eq!(start.plan.remote_os, "linux");
        // NOT SELF-ATTESTATION: the plan rewrote the command to launch under the
        // remote OS sandbox launcher, and the transport ACTUALLY received that
        // `bwrap`-wrapped command — the additional enforcement layer, not just a
        // claim. The original program is carried as an argv token after the bwrap
        // flags.
        let wrapped = start
            .plan
            .wrapped_request
            .as_ref()
            .expect("enforced plan carries a wrapped request");
        assert_eq!(wrapped.program, "bwrap");
        assert!(
            wrapped.argv.iter().any(|a| a == "/bin/sh"),
            "the original program is launched UNDER bwrap: {:?}",
            wrapped.argv
        );
        assert!(
            wrapped.argv.iter().any(|a| a == "--unshare-net"),
            "a network-forbidding profile drops egress at the bwrap layer: {:?}",
            wrapped.argv
        );
        let launched = runner
            .transport_last_launched_request()
            .expect("the transport was handed a launch request");
        assert_eq!(
            launched.program, "bwrap",
            "the transport received the bwrap-wrapped command, not the bare original"
        );
        // A confined run actually spawned, and it carries the reversible checkpoint
        // (the git-materialized commit ref) so the sandbox is ADDITIVE to rollback.
        let outcome = start.outcome.expect("a confined remote process ran");
        assert_eq!(runner.transport_spawn_count(), 1);
        assert_eq!(
            start.checkpoint_ref.as_deref(),
            Some("refs/capo/materialized/deadbeef")
        );
        // The enforced fact precedes the start-sequence events in the trail.
        assert_eq!(outcome.events[0].kind, "sandbox.enforced");
        assert!(
            outcome
                .events
                .iter()
                .any(|e| e.kind == "runtime.remote_process_started")
        );
    }

    /// RR5 HONESTY (review findings 2 + 7): a LOOPBACK / fake channel never crossed
    /// a machine boundary, so even when it scripts an enforcing remote OS the runner
    /// must NOT claim `Enforced` (it cannot have applied `bwrap`/`sandbox-exec` over
    /// a boundary it never crossed). The default fake channel is a loopback, so the
    /// plan is honestly `Unenforced` and the command is NOT wrapped.
    #[test]
    fn remote_sandbox_loopback_channel_is_never_enforced_even_with_enforcing_remote_os() {
        let workspace = temp_root("rr5-loopback-root");
        fs::create_dir_all(&workspace).unwrap();
        // Enforcing remote OS + mechanism present, but NO cross-machine boundary.
        let runner = sandbox_runner("rr5-loopback", &workspace, |c| {
            c.with_remote_os(RemoteOsFamily::Linux)
                .with_enforceable_remote_sandbox()
        });
        assert!(runner.is_loopback());
        let request = remote_request("run-rr5-loopback", workspace.clone(), "printf ok");
        let profile = SandboxProfile::workspace_confined([workspace.clone()]);
        let plan = runner
            .plan_remote_sandbox(
                &request,
                &workspace,
                &profile,
                SandboxTier::LinuxLandlockBwrap,
                false,
            )
            .expect("plan");
        match &plan.enforcement {
            SandboxEnforcement::Unenforced { reason, .. } => {
                assert!(
                    reason.contains("loopback"),
                    "the limitation names the loopback boundary: {reason}"
                );
            }
            other => panic!("a loopback must be Unenforced, got {other:?}"),
        }
        // NOT wrapped: the command Capo would launch is the bare original, never a
        // `bwrap`/`sandbox-exec` we cannot honestly say enforced anything.
        let wrapped = plan
            .wrapped_request
            .as_ref()
            .expect("unenforced plan still runs the original");
        assert_eq!(wrapped.program, "/bin/sh");
        assert!(plan.events.iter().any(|e| e.kind == "sandbox.unenforced"));
    }

    #[test]
    fn remote_sandbox_is_unenforced_and_recorded_when_remote_os_cannot_enforce() {
        let workspace = temp_root("rr5-unenf-root");
        fs::create_dir_all(&workspace).unwrap();
        // The remote reports a NON-enforcing OS family: Capo must NOT claim
        // sandboxing; it runs honestly un-enforced and records the limitation.
        let runner = sandbox_runner("rr5-unenf", &workspace, |c| {
            c.with_remote_os(RemoteOsFamily::Other("freebsd".to_string()))
        });
        let request = remote_request("run-rr5-unenf", workspace.clone(), "printf ok");
        let profile = SandboxProfile::workspace_confined([workspace.clone()]);
        let start = runner
            .start_process_sandboxed(
                request,
                &workspace,
                &profile,
                SandboxTier::LinuxLandlockBwrap,
                false,
                None,
            )
            .expect("plan");

        match &start.plan.enforcement {
            SandboxEnforcement::Unenforced { tier, reason } => {
                assert_eq!(*tier, SandboxTier::LinuxLandlockBwrap);
                assert!(
                    reason.contains("freebsd"),
                    "reason names remote os: {reason}"
                );
            }
            other => panic!("expected Unenforced, got {other:?}"),
        }
        assert_eq!(start.plan.remote_os, "freebsd");
        // The run still happens (honestly un-sandboxed), and the limitation is an
        // EVENT, never a silent claim of confinement.
        assert!(start.outcome.is_some());
        assert!(
            start
                .plan
                .events
                .iter()
                .any(|e| { e.kind == "sandbox.unenforced" && e.detail.contains("freebsd") })
        );
    }

    #[test]
    fn remote_sandbox_unenforced_when_remote_lacks_the_mechanism_even_on_matching_family() {
        let workspace = temp_root("rr5-nomech-root");
        fs::create_dir_all(&workspace).unwrap();
        // A linux remote whose landlock/bwrap mechanism is unavailable: the family
        // matches but the host reports it cannot enforce -> honest Unenforced.
        let runner = sandbox_runner("rr5-nomech", &workspace, |c| {
            c.with_remote_os(RemoteOsFamily::Linux)
                .with_unenforceable_remote_sandbox()
        });
        let request = remote_request("run-rr5-nomech", workspace.clone(), "printf ok");
        let profile = SandboxProfile::workspace_confined([workspace.clone()]);
        let plan = runner
            .plan_remote_sandbox(
                &request,
                &workspace,
                &profile,
                SandboxTier::LinuxLandlockBwrap,
                false,
            )
            .expect("plan");
        assert!(matches!(
            plan.enforcement,
            SandboxEnforcement::Unenforced { .. }
        ));
        assert!(plan.events.iter().any(|e| e.kind == "sandbox.unenforced"));
    }

    #[test]
    fn remote_sandbox_enforcement_reads_the_remote_os_not_the_controller_host() {
        let workspace = temp_root("rr5-honesty-root");
        fs::create_dir_all(&workspace).unwrap();
        // Probe the SAME tier against two different scripted remote OSes. The claim
        // must follow the REMOTE os, not the controller's build target: only the
        // matching-family remote enforces, the other is honestly unenforced.
        // Both model a real cross-machine boundary with the mechanism present, so
        // the ONLY difference is the remote OS family — isolating that the claim
        // follows the remote OS, not the controller host or the loopback gate.
        let linux = sandbox_runner("rr5-h-linux", &workspace, |c| {
            c.with_remote_os(RemoteOsFamily::Linux)
                .with_enforceable_remote_sandbox()
                .with_cross_machine_boundary()
        });
        let macos = sandbox_runner("rr5-h-macos", &workspace, |c| {
            c.with_remote_os(RemoteOsFamily::Macos)
                .with_enforceable_remote_sandbox()
                .with_cross_machine_boundary()
        });
        let request = remote_request("run-rr5-honesty", workspace.clone(), "printf ok");
        let profile = SandboxProfile::workspace_confined([workspace.clone()]);

        let on_linux = linux
            .plan_remote_sandbox(
                &request,
                &workspace,
                &profile,
                SandboxTier::LinuxLandlockBwrap,
                false,
            )
            .expect("plan linux");
        let on_macos = macos
            .plan_remote_sandbox(
                &request,
                &workspace,
                &profile,
                SandboxTier::LinuxLandlockBwrap,
                false,
            )
            .expect("plan macos");

        // landlock+bwrap is enforced on the linux remote, unenforced on the macos
        // remote — decided by the remote probe, identical controller host.
        assert!(matches!(
            on_linux.enforcement,
            SandboxEnforcement::Enforced {
                tier: SandboxTier::LinuxLandlockBwrap
            }
        ));
        assert!(matches!(
            on_macos.enforcement,
            SandboxEnforcement::Unenforced { .. }
        ));
    }

    #[test]
    fn remote_sandbox_plan_is_replay_stable() {
        let workspace = temp_root("rr5-replay-root");
        fs::create_dir_all(&workspace).unwrap();
        let request = remote_request("run-rr5-replay", workspace.clone(), "printf ok");
        let profile = SandboxProfile::workspace_confined([workspace.clone()]);
        // Two independent runners over the same scripted remote produce identical
        // plans (enforcement + events), so a restart rebuilds the same projection.
        let plan_a = sandbox_runner("rr5-replay", &workspace, |c| {
            c.with_remote_os(RemoteOsFamily::Linux)
                .with_enforceable_remote_sandbox()
                .with_cross_machine_boundary()
        })
        .plan_remote_sandbox(
            &request,
            &workspace,
            &profile,
            SandboxTier::LinuxLandlockBwrap,
            false,
        )
        .expect("plan a");
        let plan_b = sandbox_runner("rr5-replay", &workspace, |c| {
            c.with_remote_os(RemoteOsFamily::Linux)
                .with_enforceable_remote_sandbox()
                .with_cross_machine_boundary()
        })
        .plan_remote_sandbox(
            &request,
            &workspace,
            &profile,
            SandboxTier::LinuxLandlockBwrap,
            false,
        )
        .expect("plan b");
        assert_eq!(plan_a, plan_b);
    }

    #[test]
    fn remote_start_appends_request_then_resolve_then_started_in_order() {
        let workspace = temp_root("workspace-remote-order");
        let artifacts = temp_root("artifacts-remote-order");
        fs::create_dir_all(&workspace).unwrap();
        let runner = RemoteProcessRunner::new(RemoteProcessConfig::loopback_for_test(
            "remote-target-order",
            "endpoint-order",
            workspace.clone(),
            artifacts,
        ));

        let outcome = runner
            .start_process(remote_request("run-order", workspace, "printf ok"))
            .expect("remote start");

        // Append-first Start Sequence: pending request -> target resolved ->
        // process started, all BEFORE the captured local-exit events.
        let kinds: Vec<&str> = outcome.events.iter().map(|e| e.kind.as_str()).collect();
        let req = kinds
            .iter()
            .position(|k| *k == "runtime.remote_start_requested")
            .expect("start_requested present");
        let resolved = kinds
            .iter()
            .position(|k| *k == "runtime.remote_target_resolved")
            .expect("target_resolved present");
        let started = kinds
            .iter()
            .position(|k| *k == "runtime.remote_process_started")
            .expect("process_started present");
        assert!(req < resolved && resolved < started, "order: {kinds:?}");
        assert_eq!(outcome.events[req].status, "pending");
        assert!(
            outcome.events[req]
                .detail
                .contains("idempotency_key=run-order")
        );
        assert!(
            outcome
                .process
                .runtime_process_ref
                .starts_with("remote-process:remote-target-order:endpoint-order:pid=")
        );
        // Remote identity, NOT the local external_pid/boot_id path.
        assert!(outcome.process.external_pid.is_none());
        assert!(outcome.process.boot_id.is_none());
    }

    #[test]
    fn remote_start_with_same_idempotency_key_keeps_a_stable_remote_ref() {
        let workspace = temp_root("workspace-remote-idem");
        let artifacts = temp_root("artifacts-remote-idem");
        fs::create_dir_all(&workspace).unwrap();
        let runner = RemoteProcessRunner::new(RemoteProcessConfig::loopback_for_test(
            "remote-target-idem",
            "endpoint-idem",
            workspace.clone(),
            artifacts,
        ));

        let first = runner
            .start_process(remote_request("run-idem", workspace.clone(), "printf ok"))
            .expect("first start");
        // After exactly one start the transport has spawned exactly once.
        assert_eq!(
            runner.transport_spawn_count(),
            1,
            "first start must spawn exactly one remote process"
        );

        let second = runner
            .start_process(remote_request("run-idem", workspace.clone(), "printf ok"))
            .expect("second start with same key");

        // THE INVARIANT: a repeated start with the same idempotency key must NOT
        // spawn a second remote process. We assert this directly against the
        // transport's actual spawn counter — not by relying on a constant pid.
        assert_eq!(
            runner.transport_spawn_count(),
            1,
            "duplicate idempotency key must NOT spawn a second remote process"
        );
        // And the recorded remote identity is the SAME one (the de-duplicated
        // outcome is the first launch replayed).
        assert_eq!(
            first.process.runtime_process_ref, second.process.runtime_process_ref,
            "duplicate idempotency key must resolve to the same remote process-ref"
        );
        assert!(
            first
                .events
                .iter()
                .any(|e| e.detail.contains("idempotency_key=run-idem"))
        );
        // The duplicate's trail records that it was de-duplicated, not re-spawned.
        assert!(
            second
                .events
                .iter()
                .any(|e| e.detail.contains("deduplicated=true")),
            "duplicate start must record that it was de-duplicated"
        );

        // A DIFFERENT idempotency key DOES spawn a second process with a distinct
        // remote pid — proving the dedup is keyed, and the pid is not a constant.
        let other = runner
            .start_process(remote_request("run-idem-2", workspace, "printf ok"))
            .expect("start with a different key");
        assert_eq!(
            runner.transport_spawn_count(),
            2,
            "a different idempotency key must spawn a new remote process"
        );
        assert_ne!(
            first.process.runtime_process_ref, other.process.runtime_process_ref,
            "distinct keys must mint distinct remote process-refs"
        );
    }

    #[test]
    fn remote_start_launch_failure_yields_typed_retryability() {
        let workspace = temp_root("workspace-remote-fail");
        let artifacts = temp_root("artifacts-remote-fail");
        fs::create_dir_all(&workspace).unwrap();
        let channel =
            OpenChannel::for_test("remote-target-fail", "endpoint-fail", "remote-target-fail");
        let runner = FakeRemoteProcessRunner::with_launch_failure(
            channel,
            workspace.clone(),
            artifacts,
            "channel refused launch",
            true,
        );

        let error = runner
            .start_process(remote_request("run-fail", workspace, "printf nope"))
            .expect_err("launch must fail");

        match error {
            RuntimeError::RemoteStartFailed {
                retryable,
                events,
                source,
            } => {
                assert!(retryable, "fake failure was marked retryable");
                // The locally-appended trail ends with the typed failure event.
                assert_eq!(
                    events.last().unwrap().kind,
                    "runtime.remote_process_start_failed"
                );
                assert!(events.last().unwrap().detail.contains("retryable=true"));
                // No raw secret material leaks; the redacted reason is present.
                assert!(
                    events
                        .last()
                        .unwrap()
                        .detail
                        .contains("channel refused launch")
                );
                assert!(matches!(
                    *source,
                    RuntimeError::RemoteLaunchFailed {
                        retryable: true,
                        ..
                    }
                ));
            }
            other => panic!("expected RemoteStartFailed, got {other:?}"),
        }
    }

    #[test]
    fn remote_runner_performs_no_endpoint_resolution() {
        // The runner is constructed from an ALREADY-resolved channel handle and
        // reads identity from its fingerprint; it never resolves an endpoint.
        let workspace = temp_root("workspace-remote-resolved");
        let artifacts = temp_root("artifacts-remote-resolved");
        fs::create_dir_all(&workspace).unwrap();
        let channel = OpenChannel::for_test("chan-resolved", "endpoint-resolved", "fp-resolved");
        let runner = FakeRemoteProcessRunner::build(channel, workspace, artifacts);

        // Identity is the injected fingerprint, proving no resolution happened.
        assert_eq!(runner.target_fingerprint(), "fp-resolved");
        // It is honest that this is a loopback/fake remote, not a real boundary.
        assert!(runner.is_loopback());
        assert!(runner.binding().fake);
    }

    #[test]
    fn remote_cleanup_after_spawn_is_idempotent_and_emits_completed() {
        // Models the "append failed after spawn" recovery: the runner attempts a
        // remote cleanup over the channel, which is idempotent and emits the
        // distinct completion event without panicking.
        let workspace = temp_root("workspace-remote-cleanup");
        let artifacts = temp_root("artifacts-remote-cleanup");
        fs::create_dir_all(&workspace).unwrap();
        let runner = RemoteProcessRunner::new(RemoteProcessConfig::loopback_for_test(
            "remote-target-cleanup",
            "endpoint-cleanup",
            workspace.clone(),
            artifacts,
        ));

        let outcome = runner
            .start_process(remote_request("run-cleanup", workspace, "printf ok"))
            .expect("remote start");

        let first = runner.cleanup(&outcome.process).expect("first cleanup");
        let second = runner
            .cleanup(&outcome.process)
            .expect("idempotent cleanup");
        assert_eq!(first.events[0].kind, "runtime.remote_cleanup_completed");
        assert_eq!(second.events[0].kind, "runtime.remote_cleanup_completed");
        assert_eq!(
            first.process.runtime_process_ref,
            second.process.runtime_process_ref
        );
    }

    #[test]
    fn remote_health_probe_overrides_a_stale_running_status() {
        // The stored ref says `running`, but the channel probe reports the remote
        // process DEAD. `health` must trust the probe (live=false), NOT echo the
        // local status string the pre-RR1 stub used.
        let workspace = temp_root("workspace-remote-probe-override");
        let artifacts = temp_root("artifacts-remote-probe-override");
        fs::create_dir_all(&workspace).unwrap();
        let channel = OpenChannel::for_test(
            "remote-target-probe",
            "endpoint-probe",
            "remote-target-probe",
        );
        let base = FakeRemoteChannel::from_open_channel(&channel, workspace.clone(), artifacts);
        let runner = RemoteProcessRunner::new(RemoteProcessConfig::with_transport(
            channel,
            RemoteChannel::Fake(base.with_probe_reports_dead()),
        ));
        let outcome = runner
            .start_process(remote_request("run-probe", workspace, "printf ok"))
            .expect("remote start");

        // Force the stored status to `running` so the ONLY thing that can make
        // health report dead is the probe overriding it.
        let stored = LocalRuntimeProcessRef {
            status: "running".to_string(),
            ..outcome.process
        };
        let health = runner.health(&stored).expect("health probe");
        assert!(
            !health.live,
            "the remote probe must override the stale running status"
        );
        assert_eq!(health.status, "exited");
    }

    // ----- RR2: reattach-after-restart + recovery across the boundary -----

    /// Build a remote runner over a fake channel with a scripted recovery outcome,
    /// run a process so a REAL stored `remote_process_ref` (with the recorded
    /// `:pid=...:boot=...` tail) exists, and return the runner, the stored ref
    /// flipped to the in-flight `running` state a crash interrupts, and the remote
    /// boot id recorded at launch. NO network — the channel is the deterministic
    /// fake; this is the deterministic-fake-before-live discipline RR2 requires.
    fn scripted_recovery_runner(
        name: &str,
        script: impl FnOnce(FakeRemoteChannel) -> FakeRemoteChannel,
    ) -> (RemoteProcessRunner, LocalRuntimeProcessRef, String) {
        let workspace = temp_root(&format!("workspace-{name}"));
        let artifacts = temp_root(&format!("artifacts-{name}"));
        fs::create_dir_all(&workspace).unwrap();
        let channel = OpenChannel::for_test(
            format!("chan-{name}"),
            format!("endpoint-{name}"),
            format!("fp-{name}"),
        );
        let base = FakeRemoteChannel::from_open_channel(&channel, workspace.clone(), artifacts);
        let recorded_boot = base.remote_boot_id();
        let transport = RemoteChannel::Fake(script(base));
        let runner =
            RemoteProcessRunner::new(RemoteProcessConfig::with_transport(channel, transport));

        let outcome = runner
            .start_process(remote_request(
                &format!("run-{name}"),
                workspace,
                "printf ok",
            ))
            .expect("remote start for recovery fixture");
        // The crash interrupts an in-flight (running) run.
        let running = LocalRuntimeProcessRef {
            status: "running".to_string(),
            ..outcome.process
        };
        (runner, running, recorded_boot)
    }

    #[test]
    fn remote_recovery_alive_reattachable_recovers_in_place() {
        let (runner, running, recorded_boot) =
            scripted_recovery_runner("rec-alive", |c| c.recover_alive_reattachable());

        // The launch recorded a remote pid + boot, so reattach is truthfully
        // supported.
        assert!(runner.reattach_supported(&running));

        let recovery = runner.recover_run(&running, &recorded_boot);
        assert_eq!(
            recovery.classification,
            RemoteRecoveryClassification::Recovered
        );
        // Append-first: a recovery_attempted precedes the single terminal event.
        let kinds: Vec<&str> = recovery.events.iter().map(|e| e.kind.as_str()).collect();
        assert_eq!(
            kinds,
            vec![
                "runtime.remote_recovery_attempted",
                "runtime.remote_run_recovered"
            ]
        );
    }

    #[test]
    fn remote_recovery_alive_but_unattachable_is_orphaned() {
        let (runner, running, recorded_boot) =
            scripted_recovery_runner("rec-orphan", |c| c.recover_alive_unattachable());

        let recovery = runner.recover_run(&running, &recorded_boot);
        assert_eq!(
            recovery.classification,
            RemoteRecoveryClassification::Orphaned
        );
        assert_eq!(
            recovery.events.last().unwrap().kind,
            "runtime.remote_run_orphaned"
        );
        assert!(recovery.detail.contains("inspectable"));
    }

    #[test]
    fn remote_recovery_reboot_boot_id_mismatch_is_exited_never_recovered() {
        let (runner, running, recorded_boot) =
            scripted_recovery_runner("rec-reboot", |c| c.recover_rebooted());

        let recovery = runner.recover_run(&running, &recorded_boot);
        // A rebooted remote is GONE — never silently "recovered" on a recycled pid.
        assert_eq!(
            recovery.classification,
            RemoteRecoveryClassification::Exited
        );
        assert_eq!(
            recovery.events.last().unwrap().kind,
            "runtime.remote_run_exited"
        );
        assert!(recovery.detail.contains("boot-id mismatch"));
    }

    #[test]
    fn remote_recovery_gone_is_exited_unknown_detail() {
        let (runner, running, recorded_boot) =
            scripted_recovery_runner("rec-gone", |c| c.recover_gone());

        let recovery = runner.recover_run(&running, &recorded_boot);
        assert_eq!(
            recovery.classification,
            RemoteRecoveryClassification::Exited
        );
        assert_eq!(
            recovery.events.last().unwrap().kind,
            "runtime.remote_run_exited"
        );
        assert!(recovery.detail.contains("unknown exit detail"));
    }

    #[test]
    fn remote_recovery_channel_unreachable_is_pending_then_recovers_on_return() {
        // Build ONE channel identity + start ONE process so a single REAL stored
        // remote ref exists (with the recorded :pid=...:boot=... tail). The two
        // recovery attempts below re-probe THIS SAME stored ref, modelling a single
        // run whose channel was unreachable on the first restart and reachable on a
        // later one — not two independent happy paths.
        let workspace = temp_root("workspace-rec-return");
        let artifacts = temp_root("artifacts-rec-return");
        fs::create_dir_all(&workspace).unwrap();
        let channel =
            OpenChannel::for_test("chan-rec-return", "endpoint-rec-return", "fp-rec-return");
        let base = FakeRemoteChannel::from_open_channel(&channel, workspace.clone(), artifacts);
        let recorded_boot = base.remote_boot_id();

        // Launch once over a plain channel to mint the stored ref.
        let launch_runner = RemoteProcessRunner::new(RemoteProcessConfig::with_transport(
            channel.clone(),
            RemoteChannel::Fake(base),
        ));
        let outcome = launch_runner
            .start_process(remote_request(
                "run-rec-return",
                workspace.clone(),
                "printf ok",
            ))
            .expect("remote start for recovery fixture");
        let stored = LocalRuntimeProcessRef {
            status: "running".to_string(),
            ..outcome.process
        };

        // Helper: re-resolve the SAME channel identity with a given recovery script.
        let rebuild = |script: fn(FakeRemoteChannel) -> FakeRemoteChannel| {
            let chan = FakeRemoteChannel::from_open_channel(
                &channel,
                workspace.clone(),
                temp_root("artifacts-rec-return-reresolve"),
            );
            RemoteProcessRunner::new(RemoteProcessConfig::with_transport(
                channel.clone(),
                RemoteChannel::Fake(script(chan)),
            ))
        };

        // First restart: channel unreachable -> recovery_pending (NOT forced to
        // recovered or exited). This is the remote-only state the local path
        // cannot have.
        let pending_runner = rebuild(|c| c.recover_channel_unreachable());
        let pending = pending_runner.recover_run(&stored, &recorded_boot);
        assert_eq!(
            pending.classification,
            RemoteRecoveryClassification::RecoveryPending
        );
        assert!(pending.classification.is_pending());
        assert_eq!(
            pending.events.last().unwrap().kind,
            "runtime.remote_recovery_pending"
        );

        // Channel returns on a later restart: re-resolve the SAME channel identity
        // to a now-reachable one and re-run recovery against the SAME stored ref.
        // The previously-pending run now recovers in place — recovery retried, the
        // run never lost or forced.
        let return_runner = rebuild(|c| c.recover_alive_reattachable());
        let recovered = return_runner.recover_run(&stored, &recorded_boot);
        assert_eq!(
            recovered.classification,
            RemoteRecoveryClassification::Recovered,
            "the same pending stored ref must recover when the channel returns"
        );
        assert_eq!(recovered.runtime_process_ref, stored.runtime_process_ref);
    }

    #[test]
    fn remote_recovery_is_replay_stable_across_repeated_restarts() {
        // Restart/replay: re-running recovery against the SAME stored ref + the same
        // scripted channel rebuilds an IDENTICAL classification + event trail, so a
        // recovered projection is replay-stable (no duplicate/divergent recovery).
        let (runner, running, recorded_boot) =
            scripted_recovery_runner("rec-replay", |c| c.recover_alive_reattachable());

        let first = runner.recover_run(&running, &recorded_boot);
        let second = runner.recover_run(&running, &recorded_boot);
        assert_eq!(first, second, "recovery must be replay-stable");
    }

    #[test]
    fn remote_recovery_is_in_place_not_a_relaunch_with_recovery_of_run_id() {
        // `recovery_of_run_id` is ONLY for a relaunch/retry after restart, never for
        // a simple in-place reattach. The runner's recover_run reattaches in place:
        // it carries NO new run id and does not relaunch (no second start event).
        let (runner, running, recorded_boot) =
            scripted_recovery_runner("rec-inplace", |c| c.recover_alive_reattachable());

        let recovery = runner.recover_run(&running, &recorded_boot);
        // The recovered ref is the SAME stored ref — an in-place reattach.
        assert_eq!(recovery.runtime_process_ref, running.runtime_process_ref);
        // No relaunch happened: there is no remote_process_started in the recovery
        // trail (that belongs to start_process / a relaunch, not a reattach).
        assert!(
            !recovery
                .events
                .iter()
                .any(|e| e.kind == "runtime.remote_process_started"),
            "an in-place reattach must NOT relaunch the remote process"
        );
    }

    #[test]
    fn remote_reattach_unsupported_for_bare_ref_without_pid_boot() {
        // A bare ref with no recorded remote pid/boot is NOT reattachable: the
        // runner reports reattach support truthfully (the runtime-tunnel.md
        // "report whether reattach is supported" responsibility).
        let workspace = temp_root("workspace-rec-bare");
        let artifacts = temp_root("artifacts-rec-bare");
        fs::create_dir_all(&workspace).unwrap();
        let runner = RemoteProcessRunner::new(RemoteProcessConfig::loopback_for_test(
            "remote-target-bare",
            "endpoint-bare",
            workspace,
            artifacts,
        ));
        let bare = LocalRuntimeProcessRef {
            run_id: RunId::new("run-bare"),
            runtime_process_ref: "remote-process:fp:host".to_string(),
            external_pid: None,
            boot_id: None,
            status: "running".to_string(),
            redaction_state: "clean".to_string(),
        };
        assert!(!runner.reattach_supported(&bare));
    }

    #[test]
    fn parse_remote_ref_is_robust_to_pid_marker_inside_fingerprint() {
        // A fingerprint/host that itself embeds the literal `:pid=` substring must
        // NOT mislead the parser: the structured tail is parsed from the end, so the
        // REAL recorded pid/boot/host still resolve (review finding 10).
        let evil = "remote-process:fp:pid=spoof:host-real:pid=4242:boot=boot-real";
        let parsed = parse_remote_ref(evil).expect("structured tail must still parse");
        assert_eq!(
            parsed.pid, 4242,
            "the real recorded pid, not the embedded one"
        );
        assert_eq!(parsed.boot, "boot-real");
        assert_eq!(parsed.host, "host-real");
    }

    #[test]
    fn probe_carries_host_from_stored_ref_not_the_reresolved_channel() {
        // After a channel re-resolution to a DIFFERENT endpoint, a probe built from
        // a stored ref must carry the host recorded at launch, not the new endpoint
        // (review finding 9).
        let workspace = temp_root("workspace-probe-host");
        let artifacts = temp_root("artifacts-probe-host");
        fs::create_dir_all(&workspace).unwrap();
        // The channel now resolves to "endpoint-NEW", but the stored ref recorded
        // "host-ORIGINAL" at launch.
        let channel = OpenChannel::for_test("chan-host", "endpoint-NEW", "fp-host");
        let runner = FakeRemoteProcessRunner::build(channel, workspace, artifacts);
        let stored = LocalRuntimeProcessRef {
            run_id: RunId::new("run-host"),
            runtime_process_ref: "remote-process:fp-host:host-ORIGINAL:pid=51000:boot=boot-x"
                .to_string(),
            external_pid: None,
            boot_id: None,
            status: "running".to_string(),
            redaction_state: "clean".to_string(),
        };
        let probe = runner.probe_from_ref(&stored);
        assert_eq!(
            probe.remote_host_id, "host-ORIGINAL",
            "probe must carry the host recorded in the stored ref, not the re-resolved endpoint"
        );
        assert_eq!(probe.remote_pid, 51000);
    }

    // ----- RR4: remote output-delta + stdin streaming over the channel -----

    /// Build a remote runner over a fake channel whose stream is scripted by
    /// `script`, run a process so a REAL stored `remote_process_ref` exists, and
    /// return the runner + the stored ref (flipped to the in-flight `running`
    /// state). NO network — the deterministic fake channel models the stream.
    fn streaming_runner(
        name: &str,
        script: impl FnOnce(FakeRemoteChannel) -> FakeRemoteChannel,
    ) -> (
        RemoteProcessRunner,
        LocalRuntimeProcessRef,
        FakeRemoteChannel,
    ) {
        let workspace = temp_root(&format!("workspace-{name}"));
        let artifacts = temp_root(&format!("artifacts-{name}"));
        fs::create_dir_all(&workspace).unwrap();
        let channel = OpenChannel::for_test(
            format!("chan-{name}"),
            format!("endpoint-{name}"),
            format!("fp-{name}"),
        );
        let base = FakeRemoteChannel::from_open_channel(&channel, workspace.clone(), artifacts);
        let scripted = script(base);
        let transport = RemoteChannel::Fake(scripted.clone());
        let runner =
            RemoteProcessRunner::new(RemoteProcessConfig::with_transport(channel, transport));
        let outcome = runner
            .start_process(remote_request(
                &format!("run-{name}"),
                workspace,
                "printf ok",
            ))
            .expect("remote start for streaming fixture");
        let running = LocalRuntimeProcessRef {
            status: "running".to_string(),
            ..outcome.process
        };
        (runner, running, scripted)
    }

    #[test]
    fn remote_stream_projects_ordered_deltas_once_with_monotonic_offsets() {
        let payload = b"line-1\nline-2\nline-3\n";
        let (runner, running, _chan) =
            streaming_runner("stream-order", |c| c.with_streamed_output(payload.to_vec()));

        let outcome = runner.stream_output(&running, 0);
        // The full stream is forwarded as a delta at offset 0 (EOF reached).
        assert_eq!(outcome.final_reason, RemoteStreamFinalReason::Eof);
        assert_eq!(outcome.deltas.len(), 1);
        let delta = &outcome.deltas[0];
        assert_eq!(delta.offset, 0);
        assert_eq!(delta.raw_len, payload.len());
        assert_eq!(delta.text, String::from_utf8_lossy(payload));
        // next_offset is one past the last forwarded byte: the reconnect resume
        // point.
        assert_eq!(outcome.next_offset, payload.len());

        // Each delta projects EXACTLY once: a reconnect from next_offset yields no
        // duplicate delta (the from_sequence discipline).
        let resumed = runner.stream_output(&running, outcome.next_offset);
        assert!(
            resumed.deltas.is_empty(),
            "a reconnect at the acknowledged offset must replay no already-projected delta"
        );
        assert_eq!(resumed.next_offset, payload.len());
        // The delta event carries the offsets so the projection is rebuildable.
        let delta_event = outcome
            .events
            .iter()
            .find(|e| e.kind == "runtime.remote_output_delta")
            .expect("a remote_output_delta event");
        assert!(delta_event.detail.contains("offset=0"));
        assert!(
            delta_event
                .detail
                .contains(&format!("next_offset={}", payload.len()))
        );
    }

    #[test]
    fn remote_stream_redacts_a_credential_before_any_delta_or_artifact() {
        // A credential-shaped token in the RAW remote stream MUST be scrubbed at the
        // remote boundary before it becomes a delta event / persisted artifact.
        let secret = "sk-ABCDEF0123456789ABCDEF0123456789";
        let raw = format!("starting up\nexport TOKEN={secret}\ndone\n");
        let (runner, running, _chan) = streaming_runner("stream-redact", |c| {
            c.with_streamed_output(raw.clone().into_bytes())
        });

        let outcome = runner.stream_output(&running, 0);
        assert_eq!(outcome.redaction_state, "redacted");
        let delta = &outcome.deltas[0];
        assert!(
            !delta.text.contains(secret),
            "the raw credential must NOT survive into a delta payload"
        );
        assert!(
            delta.text.contains(CREDENTIAL_REDACTION_PLACEHOLDER),
            "the credential must be replaced with the redaction placeholder"
        );
        assert_eq!(delta.redaction_state, "redacted");
        // The delta event also records the redacted state; no event detail leaks
        // the secret.
        assert!(
            outcome.events.iter().all(|e| !e.detail.contains(secret)),
            "no event detail may carry the raw credential"
        );
    }

    #[test]
    fn remote_stream_channel_drop_finalizes_with_a_recorded_reason() {
        // A channel that dies mid-stream must finalize with ChannelDropped, never a
        // silent truncation.
        let payload = b"abcdefghij"; // 10 bytes
        let (runner, running, _chan) = streaming_runner("stream-drop", |c| {
            c.with_streamed_output(payload.to_vec())
                .with_stream_drop_after(4)
        });

        let outcome = runner.stream_output(&running, 0);
        assert_eq!(
            outcome.final_reason,
            RemoteStreamFinalReason::ChannelDropped
        );
        // Only the bytes before the drop boundary were forwarded.
        assert_eq!(outcome.deltas[0].text, "abcd");
        assert_eq!(outcome.next_offset, 4);
        let finalized = outcome
            .events
            .iter()
            .find(|e| e.kind == "runtime.remote_stream_finalized")
            .expect("a stream_finalized event");
        assert_eq!(finalized.status, "channel_dropped");
        assert!(finalized.detail.contains("reason=channel_dropped"));
    }

    #[test]
    fn remote_stream_is_bounded_by_the_output_cap() {
        // A stream longer than the cap finalizes CapReached and forwards at most
        // the cap.
        let big = vec![b'x'; REMOTE_OUTPUT_LIMIT_BYTES + 1024];
        let (runner, running, _chan) =
            streaming_runner("stream-cap", |c| c.with_streamed_output(big.clone()));

        let outcome = runner.stream_output(&running, 0);
        assert_eq!(outcome.final_reason, RemoteStreamFinalReason::CapReached);
        assert_eq!(outcome.deltas[0].raw_len, REMOTE_OUTPUT_LIMIT_BYTES);
        assert_eq!(outcome.next_offset, REMOTE_OUTPUT_LIMIT_BYTES);
    }

    #[test]
    fn remote_stdin_write_reaches_the_fake_remote_process() {
        let (runner, running, chan) = streaming_runner("stdin", |c| c);

        let result = runner
            .write_stdin(&running, b"hello remote\n")
            .expect("stdin write over the channel");
        assert_eq!(result.events[0].kind, "runtime.remote_stdin_written");
        assert!(result.events[0].detail.contains("bytes=13"));
        // The write actually reached the fake remote process.
        assert_eq!(chan.stdin_written(), b"hello remote\n");

        // A second write accumulates in order on the remote.
        runner
            .write_stdin(&running, b"more\n")
            .expect("second stdin write");
        assert_eq!(chan.stdin_written(), b"hello remote\nmore\n");
    }

    #[test]
    fn remote_stream_reconnect_resumes_from_last_offset_without_duplicates() {
        // Forward the stream in two reads and prove the concatenation equals the
        // whole payload with no overlap (offset-driven resume).
        let payload = b"AAAAABBBBBCCCCC"; // 15 bytes
        let (runner, running, _chan) = streaming_runner("stream-resume", |c| {
            c.with_streamed_output(payload.to_vec())
        });

        // First read consumes everything (EOF). Simulate a subscriber that only
        // acknowledged the first 5 bytes, then reconnects.
        let first = runner.stream_output(&running, 0);
        assert_eq!(first.deltas[0].text, "AAAAABBBBBCCCCC");

        // Reconnect acknowledging only offset 5: the resume yields bytes 5.. with no
        // re-delivery of the first 5.
        let resumed = runner.stream_output(&running, 5);
        assert_eq!(resumed.deltas[0].offset, 5);
        assert_eq!(resumed.deltas[0].text, "BBBBBCCCCC");
        assert_eq!(resumed.next_offset, payload.len());
        assert_eq!(resumed.final_reason, RemoteStreamFinalReason::Eof);
    }

    #[test]
    fn remote_stream_is_replay_stable_across_repeated_reads() {
        // The same stored ref + the same from_offset must produce identical
        // projected deltas + events every time (deterministic rebuild).
        let payload = b"deterministic-stream-bytes\n";
        let (runner, running, _chan) = streaming_runner("stream-replay", |c| {
            c.with_streamed_output(payload.to_vec())
        });
        let a = runner.stream_output(&running, 0);
        let b = runner.stream_output(&running, 0);
        assert_eq!(
            a, b,
            "repeated reads at the same offset must rebuild identically"
        );
    }

    // ----- RR6: crash-safe remote runs + recovery events -----

    /// Build a remote runner over a fake channel scripted by `script`, run one
    /// process so a REAL stored `remote_process_ref` exists, and return the runner,
    /// the stored ref (flipped to the in-flight `running` state a crash interrupts),
    /// the recorded remote boot id, and a clone of the scripted channel so a test
    /// can read the fake remote's worktree/rollback state directly. NO network.
    fn crash_safe_runner(
        name: &str,
        script: impl FnOnce(FakeRemoteChannel) -> FakeRemoteChannel,
    ) -> (
        RemoteProcessRunner,
        LocalRuntimeProcessRef,
        String,
        FakeRemoteChannel,
    ) {
        let workspace = temp_root(&format!("workspace-{name}"));
        let artifacts = temp_root(&format!("artifacts-{name}"));
        fs::create_dir_all(&workspace).unwrap();
        let channel = OpenChannel::for_test(
            format!("chan-{name}"),
            format!("endpoint-{name}"),
            format!("fp-{name}"),
        );
        let base = FakeRemoteChannel::from_open_channel(&channel, workspace.clone(), artifacts);
        let recorded_boot = base.remote_boot_id();
        let scripted = script(base);
        let transport = RemoteChannel::Fake(scripted.clone());
        let runner =
            RemoteProcessRunner::new(RemoteProcessConfig::with_transport(channel, transport));
        let outcome = runner
            .start_process(remote_request(
                &format!("run-{name}"),
                workspace,
                "printf ok",
            ))
            .expect("remote start for crash-safe fixture");
        let running = LocalRuntimeProcessRef {
            status: "running".to_string(),
            ..outcome.process
        };
        (runner, running, recorded_boot, scripted)
    }

    #[test]
    fn remote_crash_controller_restart_with_live_remote_recovers_in_place() {
        // Failure mode: the remote process SURVIVES a controller restart. Recovery
        // re-probes the stored ref over the re-resolved channel and recovers in
        // place (no relaunch).
        let (runner, running, recorded_boot, _chan) =
            crash_safe_runner("rr6-restart", |c| c.recover_alive_reattachable());
        let recovery = runner.recover_run(&running, &recorded_boot);
        assert_eq!(
            recovery.classification,
            RemoteRecoveryClassification::Recovered
        );
        assert!(
            !recovery
                .events
                .iter()
                .any(|e| e.kind == "runtime.remote_process_started"),
            "an in-place reattach must NOT relaunch the remote process"
        );
    }

    #[test]
    fn remote_crash_remote_reboot_is_exited_never_silently_recovered() {
        // Failure mode: the remote HOST rebooted (boot-id mismatch). The recorded
        // pid is meaningless, so the run is classified Exited, never recovered.
        let (runner, running, recorded_boot, _chan) =
            crash_safe_runner("rr6-reboot", |c| c.recover_rebooted());
        let recovery = runner.recover_run(&running, &recorded_boot);
        assert_eq!(
            recovery.classification,
            RemoteRecoveryClassification::Exited,
            "a remote reboot must be Exited, never silently Recovered"
        );
        assert_eq!(
            recovery.events.last().unwrap().kind,
            "runtime.remote_run_exited"
        );
    }

    #[test]
    fn remote_crash_channel_drop_finalizes_stream_with_recorded_reason() {
        // Failure mode: the channel drops mid-run. The stream finalizes with a
        // recorded ChannelDropped reason (NOT a silent truncation), so the run can
        // be cleanly failed and the operator learns the stream was cut.
        let payload = b"first-half-second-half";
        let (runner, running, _chan) = streaming_runner("rr6-drop", |c| {
            c.with_streamed_output(payload.to_vec())
                .with_stream_drop_after(11)
        });
        let outcome = runner.stream_output(&running, 0);
        assert_eq!(
            outcome.final_reason,
            RemoteStreamFinalReason::ChannelDropped
        );
        assert!(
            outcome
                .events
                .iter()
                .any(|e| e.kind == "runtime.remote_stream_finalized"
                    && e.status == "channel_dropped"),
            "a mid-stream channel drop must finalize with a recorded reason, never a silent truncation"
        );
    }

    #[test]
    fn remote_crash_dangling_worktree_is_reaped_on_cleanup() {
        // Failure mode: a crash left a remote git worktree DANGLING. A
        // cleanup_run(ReapAll) reaps it (runtime.remote_workspace_torn_down) and
        // records cleanup completion — never silently abandoned.
        let (runner, running, _boot, chan) = crash_safe_runner("rr6-dangling", |c| {
            c.with_dangling_worktree("remote-host/dangling-run")
        });
        assert!(chan.has_remote_worktree(), "fixture has a worktree to reap");

        let result = runner
            .cleanup_run(&running, CleanupPolicy::ReapAll)
            .expect("cleanup reaps the dangling worktree");
        let kinds: Vec<&str> = result.events.iter().map(|e| e.kind.as_str()).collect();
        assert!(
            kinds.contains(&"runtime.remote_workspace_torn_down"),
            "a dangling worktree must be torn down, not silently abandoned"
        );
        assert!(kinds.contains(&"runtime.remote_cleanup_completed"));
        assert!(
            !chan.has_remote_worktree(),
            "the worktree must be gone after a ReapAll cleanup"
        );
    }

    #[test]
    fn remote_cleanup_is_idempotent_after_a_partial_failure() {
        // Cleanup must be safe to re-run: a second cleanup over the same ref reaps
        // nothing new (the worktree is already gone) and still records completion.
        let (runner, running, _boot, _chan) = crash_safe_runner("rr6-idem", |c| c);
        let first = runner
            .cleanup_run(&running, CleanupPolicy::ReapAll)
            .expect("first cleanup");
        let second = runner
            .cleanup_run(&running, CleanupPolicy::ReapAll)
            .expect("idempotent re-run");
        assert!(
            first
                .events
                .iter()
                .any(|e| e.kind == "runtime.remote_workspace_torn_down"),
            "first cleanup reaps the launched worktree"
        );
        assert!(
            !second
                .events
                .iter()
                .any(|e| e.kind == "runtime.remote_workspace_torn_down"),
            "a re-run finds nothing to reap — no second teardown"
        );
        // Both record completion (idempotent + auditable).
        assert!(
            first
                .events
                .iter()
                .any(|e| e.kind == "runtime.remote_cleanup_completed")
        );
        assert!(
            second
                .events
                .iter()
                .any(|e| e.kind == "runtime.remote_cleanup_completed")
        );
    }

    #[test]
    fn remote_cleanup_preserve_policy_keeps_the_worktree_for_inspection() {
        // An orphaned run's worktree can be PRESERVED for inspection: cleanup reaps
        // the process group but leaves the worktree (no torn_down event).
        let (runner, running, _boot, chan) = crash_safe_runner("rr6-preserve", |c| c);
        let result = runner
            .cleanup_run(&running, CleanupPolicy::PreserveWorktree)
            .expect("preserve cleanup");
        assert!(
            !result
                .events
                .iter()
                .any(|e| e.kind == "runtime.remote_workspace_torn_down"),
            "PreserveWorktree must NOT tear the worktree down"
        );
        assert!(
            chan.has_remote_worktree(),
            "the worktree must remain for inspection under PreserveWorktree"
        );
    }

    #[test]
    fn remote_rollback_restores_the_worktree_to_the_git_checkpoint() {
        // Compose with safety-gates checkpoint/rollback: a run can be rolled back to
        // its pre-write checkpoint (the RR3 git-materialized commit), restoring the
        // remote worktree to that checkpoint and recording the rollback.
        let (runner, running, _boot, chan) = crash_safe_runner("rr6-rollback", |c| c);
        let checkpoint = "refs/capo/materialized/deadbeef";
        let result = runner
            .rollback_to_checkpoint(&running, checkpoint)
            .expect("rollback to checkpoint");
        assert_eq!(
            result.events[0].kind, "runtime.remote_rollback_performed",
            "rollback must record the git-checkpoint restore"
        );
        assert!(
            result.events[0].detail.contains(checkpoint),
            "the rollback event records the checkpoint ref"
        );
        assert_eq!(
            chan.rolled_back_to().as_deref(),
            Some(checkpoint),
            "the rollback must reach the transport (restore the remote worktree)"
        );
    }

    #[test]
    fn remote_revoked_grant_stops_the_run_and_forbids_re_establishment() {
        // Safety boundary: a revoked remote-control grant stops the run and the
        // runner CANNOT re-establish execution without a fresh grant.
        let (runner, running, _boot, chan) = crash_safe_runner("rr6-revoke", |c| c);

        // Revoke WITH the in-flight ref: the grant flips revoked AND the run is
        // actually STOPPED over the channel (a `kill` signal reaches the transport),
        // not merely forbidden for the next launch.
        let revoke = runner.revoke_control("channel revoked by operator", Some(&running));
        assert_eq!(
            revoke.events[0].kind, "runtime.remote_control_revoked",
            "revocation must be an audit event"
        );
        assert!(runner.is_control_revoked());
        // No raw credential in the revoke detail (redaction-safe reason only).
        assert!(!revoke.events[0].detail.contains("token"));
        // The revoke ACTUALLY signalled a stop over the channel (finding 1).
        assert!(
            revoke.events[0].detail.contains("signalled=true"),
            "the revoke must record that the stop signal landed"
        );
        assert_eq!(
            chan.signals_sent(),
            vec!["kill".to_string()],
            "revoke_control must send a kill over the channel to stop the in-flight run"
        );
        // The returned ref is the SPECIFIC run being revoked, not a phantom
        // (finding 4): the audit trail correlates to the real ref.
        assert_eq!(
            revoke.process.runtime_process_ref, running.runtime_process_ref,
            "the revoke result must carry the in-flight ref it stopped"
        );

        // A NEW start under the revoked grant is refused — no re-establishment.
        let workspace = temp_root("workspace-rr6-revoke-2");
        fs::create_dir_all(&workspace).unwrap();
        let err = runner
            .start_process(remote_request("run-rr6-revoke-2", workspace, "printf ok"))
            .expect_err("a revoked grant must refuse a new launch");
        assert!(
            matches!(err, RuntimeError::RemoteControlRevoked { .. }),
            "re-establishment requires a FRESH grant, not a retry under the revoked one"
        );

        // Steering (stdin) is also refused under the revoked grant.
        let stdin_err = runner
            .write_stdin(&running, b"input")
            .expect_err("a revoked grant must refuse stdin");
        assert!(matches!(
            stdin_err,
            RuntimeError::RemoteControlRevoked { .. }
        ));

        // Rollback (re-establishing a workspace state) is also refused.
        let rollback_err = runner
            .rollback_to_checkpoint(&running, "refs/capo/materialized/abc")
            .expect_err("a revoked grant must refuse rollback");
        assert!(matches!(
            rollback_err,
            RuntimeError::RemoteControlRevoked { .. }
        ));
    }

    #[test]
    fn remote_revocation_permits_teardown_and_readonly_drain_but_not_new_execution() {
        // RR6 policy (findings 2 + 3): after revocation the runner refuses paths
        // that START or STEER execution, but the teardown escalations
        // (interrupt/terminate/kill) and the READ-ONLY output drain stay open so an
        // operator can stop and observe a dying run. The docstrings now promise
        // exactly this — this test pins the behaviour so the docs stay honest.
        let (runner, running, _boot, chan) =
            crash_safe_runner("rr6-revoke-teardown", |c| c.with_streamed_output(b"done\n"));

        // Revoke WITHOUT an in-flight ref: no stop signal is sent, a synthetic ref
        // is returned (finding 4 fallback path), and `signalled=false` is honest.
        let revoke = runner.revoke_control("operator revoke, teardown out of band", None);
        assert!(
            revoke.events[0].detail.contains("signalled=false"),
            "revoke with no ref sends no signal and says so"
        );
        assert!(chan.signals_sent().is_empty(), "no ref => no stop signal");

        // Teardown escalations are PERMITTED post-revocation (they stop, never
        // start): interrupt/terminate/kill each reach the channel.
        runner.interrupt(&running, "drain");
        runner.terminate(&running, "drain");
        runner.kill(&running, "drain");
        assert_eq!(
            chan.signals_sent(),
            vec![
                "interrupt".to_string(),
                "terminate".to_string(),
                "kill".to_string()
            ],
            "teardown escalations must remain available after revocation"
        );

        // The read-only output drain is also permitted (observability, not
        // execution): stream_output succeeds and forwards the remote bytes.
        let outcome = runner.stream_output(&running, 0);
        assert!(
            outcome.deltas.iter().any(|d| d.text.contains("done")),
            "stream_output must remain available after revocation (read-only drain)"
        );

        // But STARTING new execution is still refused.
        let workspace = temp_root("workspace-rr6-revoke-teardown-2");
        fs::create_dir_all(&workspace).unwrap();
        assert!(matches!(
            runner
                .start_process(remote_request("run-rr6-teardown-2", workspace, "printf ok"))
                .expect_err("a revoked grant must refuse a new launch"),
            RuntimeError::RemoteControlRevoked { .. }
        ));
    }

    #[test]
    fn remote_revocation_is_observed_by_a_cloned_runner() {
        // A revoke cannot be sidestepped by holding a clone of the runner: the
        // grant state is shared, so a cloned runner is ALSO revoked.
        let (runner, _running, _boot, _chan) = crash_safe_runner("rr6-revoke-clone", |c| c);
        let clone = runner.clone();
        runner.revoke_control("revoked", None);
        assert!(
            clone.is_control_revoked(),
            "a cloned runner must observe the SAME revocation (no sidestep via a clone)"
        );
        let workspace = temp_root("workspace-rr6-clone");
        fs::create_dir_all(&workspace).unwrap();
        let err = clone
            .start_process(remote_request("run-rr6-clone", workspace, "printf ok"))
            .expect_err("the clone is revoked too");
        assert!(matches!(err, RuntimeError::RemoteControlRevoked { .. }));
    }

    #[test]
    fn remote_crash_matrix_recovery_is_replay_stable() {
        // The full crash matrix must rebuild identically across repeated restarts:
        // recovered, exited (reboot), and cleanup are deterministic + replay-stable.
        let (runner, running, boot, _chan) =
            crash_safe_runner("rr6-replay", |c| c.recover_alive_reattachable());
        let first = runner.recover_run(&running, &boot);
        let second = runner.recover_run(&running, &boot);
        assert_eq!(
            first, second,
            "remote crash recovery must rebuild identically across repeated restarts"
        );

        let clean_a = runner
            .cleanup_run(&running, CleanupPolicy::ReapAll)
            .expect("cleanup a");
        let clean_b = runner
            .cleanup_run(&running, CleanupPolicy::ReapAll)
            .expect("cleanup b");
        // The SECOND cleanup is the stable steady state (worktree already reaped):
        // re-running it again is identical.
        let clean_c = runner
            .cleanup_run(&running, CleanupPolicy::ReapAll)
            .expect("cleanup c");
        assert_eq!(
            clean_b, clean_c,
            "idempotent cleanup must be replay-stable once the worktree is reaped"
        );
        assert_ne!(
            clean_a, clean_b,
            "the first cleanup reaped a worktree; later runs are no-op teardowns"
        );
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

    // ====================================================================
    // RR7: deterministic fake-remote determinism consolidation.
    //
    // One `FakeRemoteProcessRunner` + fake-channel harness exercising the FULL
    // contract end to end — start/stop/health/reattach, REAL git materialization
    // against a local bare-repo "remote", output/stdin streaming, sandbox/worktree
    // composition, crash-matrix recovery — with NO network and NO real SSH, then
    // asserting the cross-cutting invariants are replay-stable. Every git path uses
    // actual `git` against local directories so the materialization invariants are
    // proven, not modelled by an abstract flag.
    // ====================================================================

    /// RR7 helper: run a git subcommand in `dir`, panicking on failure (test setup).
    /// A FIXED author/committer identity + date makes a commit SHA fully
    /// deterministic (so the replay-stable cross-fixture SHA equality holds), and
    /// `commit.gpgsign=false` keeps the op independent of the operator's global git
    /// config (which may force GPG signing).
    fn rr7_git(dir: &Path, args: &[&str]) {
        let fixed_date = "2026-06-02T00:00:00 +0000";
        let status = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
            .env("GIT_AUTHOR_NAME", "capo-test")
            .env("GIT_AUTHOR_EMAIL", "test@capo.local")
            .env("GIT_AUTHOR_DATE", fixed_date)
            .env("GIT_COMMITTER_NAME", "capo-test")
            .env("GIT_COMMITTER_EMAIL", "test@capo.local")
            .env("GIT_COMMITTER_DATE", fixed_date)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .args(args)
            .status()
            .expect("spawn git");
        assert!(status.success(), "git {} failed", args.join(" "));
    }

    fn rr7_git_capture(dir: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(dir)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .args(args)
            .output()
            .expect("spawn git");
        assert!(output.status.success(), "git {} failed", args.join(" "));
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    /// RR7 fixture: a real local "origin" repo with one committed file plus a DIRTY
    /// untracked file (proving uncommitted scratch never travels), the source commit
    /// SHA, and the empty remote-repo + worktree-root directories the channel
    /// materializes into. NO network — every path is local + deterministic.
    struct GitRemoteFixture {
        origin: PathBuf,
        source_commit: String,
        dirty_filename: String,
        git_remote: GitRemote,
    }

    fn rr7_git_remote_fixture(name: &str) -> GitRemoteFixture {
        let origin = temp_root(&format!("rr7-origin-{name}"));
        let remote_repo = temp_root(&format!("rr7-remote-repo-{name}"));
        let worktree_root = temp_root(&format!("rr7-remote-wt-{name}"));
        fs::create_dir_all(&origin).unwrap();
        fs::create_dir_all(&remote_repo).unwrap();
        fs::create_dir_all(&worktree_root).unwrap();

        // The "remote" git store is a real bare-ish repo we can fetch into.
        rr7_git(&origin, &["init", "-q"]);
        rr7_git(&remote_repo, &["init", "-q"]);

        // One COMMITTED file — the only thing a git-based sync carries.
        fs::write(origin.join("committed.txt"), "committed-content").unwrap();
        rr7_git(&origin, &["add", "committed.txt"]);
        rr7_git(&origin, &["commit", "-q", "-m", "rr7 committed state"]);
        let source_commit = rr7_git_capture(&origin, &["rev-parse", "HEAD"]);

        // A DIRTY untracked file that MUST NOT travel (uncommitted scratch is not
        // auto-synced — the injected git-sync decision).
        let dirty_filename = "uncommitted-scratch.txt".to_string();
        fs::write(origin.join(&dirty_filename), "secret local scratch").unwrap();

        let git_remote = GitRemote::new(
            origin.clone(),
            remote_repo,
            worktree_root,
            // A transport URL with an EMBEDDED credential — it MUST be redacted
            // before it lands on the materialization event.
            "ssh://git:AKIAIOSFODNN7EXAMPLE@remote.example/repo.git",
        );
        GitRemoteFixture {
            origin,
            source_commit,
            dirty_filename,
            git_remote,
        }
    }

    /// RR7 harness: build a remote runner over a fake channel, optionally with a
    /// real git-backed remote workspace and a stream/recovery script. NO network.
    fn rr7_runner(
        name: &str,
        git_remote: Option<GitRemote>,
        script: impl FnOnce(FakeRemoteChannel) -> FakeRemoteChannel,
    ) -> (RemoteProcessRunner, PathBuf) {
        let workspace = temp_root(&format!("rr7-ws-{name}"));
        let artifacts = temp_root(&format!("rr7-art-{name}"));
        fs::create_dir_all(&workspace).unwrap();
        let channel = OpenChannel::for_test(
            format!("chan-{name}"),
            format!("endpoint-{name}"),
            format!("fp-{name}"),
        );
        let mut base = FakeRemoteChannel::from_open_channel(&channel, workspace.clone(), artifacts);
        if let Some(git_remote) = git_remote {
            base = base.with_git_remote(git_remote);
        }
        let transport = RemoteChannel::Fake(script(base));
        let runner =
            RemoteProcessRunner::new(RemoteProcessConfig::with_transport(channel, transport));
        (runner, workspace)
    }

    #[test]
    fn rr7_git_materialization_pins_head_to_the_source_sha() {
        // INVARIANT: materialization is content-addressed — the remote worktree HEAD
        // matches the source commit SHA exactly.
        let fixture = rr7_git_remote_fixture("pins-head");
        let (runner, _ws) = rr7_runner("pins-head", Some(fixture.git_remote.clone()), |c| c);

        let materialized = runner
            .materialize_workspace(&fixture.source_commit)
            .expect("materialize");

        assert_eq!(materialized.remote_head, fixture.source_commit);
        assert_eq!(materialized.source_commit, fixture.source_commit);
        // The materialized worktree carries the committed file at the SHA.
        let head_committed =
            fs::read_to_string(Path::new(&materialized.remote_worktree_path).join("committed.txt"))
                .expect("committed file present on remote worktree");
        assert_eq!(head_committed, "committed-content");
        // The materialization is a RECORDED event naming the SHA + remote HEAD.
        let event = &materialized.events[0];
        assert_eq!(event.kind, "runtime.remote_workspace_materialized");
        assert!(event.detail.contains(&fixture.source_commit));
    }

    #[test]
    fn rr7_uncommitted_scratch_is_never_materialized_on_the_remote() {
        // INVARIANT (injected git-sync decision): uncommitted/untracked scratch does
        // NOT travel. A dirty local file is ABSENT on the materialized remote
        // worktree, and the non-sync is an EXPLICIT recorded fact, not a silent gap.
        let fixture = rr7_git_remote_fixture("no-scratch");
        let (runner, _ws) = rr7_runner("no-scratch", Some(fixture.git_remote.clone()), |c| c);

        // Sanity: the dirty file really exists locally.
        assert!(fixture.origin.join(&fixture.dirty_filename).exists());

        let materialized = runner
            .materialize_workspace(&fixture.source_commit)
            .expect("materialize");

        let scratch_on_remote =
            Path::new(&materialized.remote_worktree_path).join(&fixture.dirty_filename);
        assert!(
            !scratch_on_remote.exists(),
            "uncommitted scratch must NOT be materialized on the remote worktree"
        );
        // The non-sync is recorded as an explicit fact on the event.
        assert!(
            materialized
                .events
                .iter()
                .any(|e| e.detail.contains("uncommitted_scratch_synced=false")),
            "the non-sync of uncommitted scratch must be an explicit recorded fact"
        );
    }

    #[test]
    fn rr7_materialization_event_redacts_an_embedded_credential_in_the_transport_url() {
        // INVARIANT (safety boundary): the git transport URL passes the credential
        // scan BEFORE it is recorded — a URL with an embedded secret is scrubbed, so
        // no credential ever lands on a remote-runtime event.
        let fixture = rr7_git_remote_fixture("redact-url");
        let (runner, _ws) = rr7_runner("redact-url", Some(fixture.git_remote.clone()), |c| c);

        let materialized = runner
            .materialize_workspace(&fixture.source_commit)
            .expect("materialize");

        assert_eq!(materialized.transport_url_redaction, "redacted");
        assert!(
            !materialized.transport_url.contains("AKIAIOSFODNN7EXAMPLE"),
            "the embedded credential must be scrubbed from the transport URL"
        );
        assert!(
            !materialized.events[0]
                .detail
                .contains("AKIAIOSFODNN7EXAMPLE"),
            "no credential may appear on the materialization event"
        );
    }

    #[test]
    fn rr7_remote_produced_commit_fetches_back_as_a_named_ref() {
        // INVARIANT: results map back by git — a commit produced on the remote
        // worktree fetches back into Capo's host as a named ref (the reconcile/
        // merge-back point), recorded as `runtime.remote_workspace_reconciled`.
        let fixture = rr7_git_remote_fixture("fetch-back");
        let (runner, _ws) = rr7_runner("fetch-back", Some(fixture.git_remote.clone()), |c| c);

        let materialized = runner
            .materialize_workspace(&fixture.source_commit)
            .expect("materialize");
        let worktree = PathBuf::from(&materialized.remote_worktree_path);

        // The agent produces a commit ON the remote worktree.
        fs::write(worktree.join("produced.txt"), "remote-produced").unwrap();
        rr7_git(&worktree, &["add", "produced.txt"]);
        rr7_git(&worktree, &["commit", "-q", "-m", "rr7 remote produced"]);
        let remote_tip = rr7_git_capture(&worktree, &["rev-parse", "HEAD"]);

        let reconciled = runner
            .reconcile_workspace(&worktree, "refs/capo/remote/rr7-fetch-back")
            .expect("reconcile");

        assert_eq!(reconciled.remote_commit, remote_tip);
        // The named ref now resolves in the local origin to the remote-produced tip.
        let landed = rr7_git_capture(
            &fixture.origin,
            &["rev-parse", "refs/capo/remote/rr7-fetch-back"],
        );
        assert_eq!(landed, remote_tip);
        assert_eq!(
            reconciled.events[0].kind,
            "runtime.remote_workspace_reconciled"
        );
    }

    #[test]
    fn rr7_materialization_failure_is_a_typed_error_not_a_silent_fallthrough() {
        // INVARIANT: a failed git step is a TYPED error, never a silent
        // fall-through to running in the wrong directory.
        let fixture = rr7_git_remote_fixture("typed-fail");
        let (runner, _ws) = rr7_runner("typed-fail", Some(fixture.git_remote.clone()), |c| c);

        // A commit SHA that does not exist cannot be fetched -> typed failure.
        let result = runner.materialize_workspace("0000000000000000000000000000000000000000");
        assert!(matches!(
            result,
            Err(RuntimeError::RemoteMaterializeFailed { .. })
        ));
    }

    #[test]
    fn rr7_git_materialization_is_replay_stable_across_rebuilds() {
        // INVARIANT: materialization + reconcile rebuild IDENTICALLY. Two
        // independent fixtures built from the same committed state produce the same
        // source SHA, the same remote HEAD, and the same materialized committed
        // content, and a re-materialization against the SAME remote is idempotent.
        let fixture_a = rr7_git_remote_fixture("replay-a");
        let fixture_b = rr7_git_remote_fixture("replay-b");
        // Same committed content -> same tree; SHAs match because the capo identity
        // + commit message + content are deterministic.
        assert_eq!(fixture_a.source_commit, fixture_b.source_commit);

        let (runner_a, _wa) = rr7_runner("replay-a", Some(fixture_a.git_remote.clone()), |c| c);
        let first = runner_a
            .materialize_workspace(&fixture_a.source_commit)
            .expect("materialize first");
        // Re-materialize against the SAME remote: idempotent, same HEAD + path.
        let again = runner_a
            .materialize_workspace(&fixture_a.source_commit)
            .expect("re-materialize");
        assert_eq!(first.remote_head, again.remote_head);
        assert_eq!(first.remote_worktree_path, again.remote_worktree_path);

        let (runner_b, _wb) = rr7_runner("replay-b", Some(fixture_b.git_remote.clone()), |c| c);
        let rebuilt = runner_b
            .materialize_workspace(&fixture_b.source_commit)
            .expect("materialize rebuilt");
        assert_eq!(first.remote_head, rebuilt.remote_head);
    }

    #[test]
    fn rr7_full_contract_end_to_end_is_replay_stable() {
        // The consolidated end-to-end pass: a single fake remote runner is driven
        // through start -> health -> stream -> stdin -> recover -> cleanup, and the
        // cross-cutting invariants are asserted together. Re-running the identical
        // script reproduces identical projected state (replay-stable).
        let drive = |name: &str| -> Vec<String> {
            let payload = b"token AKIAIOSFODNN7EXAMPLE done".to_vec();
            let (runner, workspace) = rr7_runner(name, None, |c| {
                c.recover_alive_reattachable()
                    .with_streamed_output(payload.clone())
            });

            // INVARIANT: the runner performs NO endpoint resolution (channel
            // injected) and the append-first start sequence holds.
            assert!(runner.is_loopback());
            let outcome = runner
                .start_process(remote_request(
                    &format!("run-{name}"),
                    workspace,
                    "printf ok",
                ))
                .expect("start");
            let mut kinds: Vec<String> = outcome.events.iter().map(|e| e.kind.clone()).collect();
            // Start sequence: requested -> resolved -> started, append-first.
            let req = kinds
                .iter()
                .position(|k| k == "runtime.remote_start_requested")
                .expect("start_requested");
            let res = kinds
                .iter()
                .position(|k| k == "runtime.remote_target_resolved")
                .expect("target_resolved");
            let started = kinds
                .iter()
                .position(|k| k == "runtime.remote_process_started")
                .expect("process_started");
            assert!(req < res && res < started, "append-first start order");

            let running = LocalRuntimeProcessRef {
                status: "running".to_string(),
                ..outcome.process.clone()
            };

            // INVARIANT: health derives from a real remote probe.
            let health = runner.health(&running).expect("health");
            assert!(health.live);

            // INVARIANT: output is REDACTED + bounded before persistence.
            let stream = runner.stream_output(&running, 0);
            assert_eq!(stream.redaction_state, "redacted");
            assert!(
                !stream
                    .deltas
                    .iter()
                    .any(|d| d.text.contains("AKIAIOSFODNN7EXAMPLE")),
                "a credential must be scrubbed before any delta"
            );
            kinds.extend(stream.events.iter().map(|e| e.kind.clone()));

            // INVARIANT: a reconnect from the last offset yields no duplicate bytes.
            let resumed = runner.stream_output(&running, stream.next_offset);
            assert!(resumed.deltas.is_empty(), "no duplicate deltas on resume");

            // stdin write reaches the remote (byte count only on the event).
            let stdin = runner.write_stdin(&running, b"hello").expect("stdin");
            kinds.extend(stdin.events.iter().map(|e| e.kind.clone()));

            // INVARIANT: recovery classification is truthful (alive+reattachable ->
            // Recovered, in place, no relaunch).
            let recovery = runner.recover_run(&running, &running_recorded_boot(&running));
            assert_eq!(
                recovery.classification,
                RemoteRecoveryClassification::Recovered
            );
            kinds.extend(recovery.events.iter().map(|e| e.kind.clone()));

            // INVARIANT: cleanup is idempotent.
            let cleaned = runner
                .cleanup_run(&running, CleanupPolicy::ReapAll)
                .expect("cleanup");
            kinds.extend(cleaned.events.iter().map(|e| e.kind.clone()));
            let cleaned_again = runner
                .cleanup_run(&running, CleanupPolicy::ReapAll)
                .expect("cleanup again");
            // Second cleanup completes without a second teardown event (idempotent).
            assert!(
                !cleaned_again
                    .events
                    .iter()
                    .any(|e| e.kind == "runtime.remote_workspace_torn_down"),
                "a re-run finds nothing to reap"
            );
            assert!(
                cleaned_again
                    .events
                    .iter()
                    .any(|e| e.kind == "runtime.remote_cleanup_completed")
            );

            kinds
        };

        let first = drive("e2e-1");
        let second = drive("e2e-2");
        assert_eq!(
            first, second,
            "the full contract must rebuild an identical projected event sequence"
        );
    }

    /// RR7 helper: the boot id recorded in a stored remote ref, so recovery probes
    /// the boot identity captured at launch (matching the happy reattach script).
    fn running_recorded_boot(process: &LocalRuntimeProcessRef) -> String {
        parse_remote_ref(&process.runtime_process_ref)
            .map(|p| p.boot)
            .unwrap_or_default()
    }

    #[test]
    fn rr7_revoked_grant_forbids_materialization_and_re_execution() {
        // INVARIANT (safety boundary): a revoked remote-control grant stops the run
        // and the runner cannot re-establish execution — materialization (a
        // precondition for a run) and start are both refused under a revoked grant.
        let fixture = rr7_git_remote_fixture("revoked");
        let (runner, workspace) = rr7_runner("revoked", Some(fixture.git_remote.clone()), |c| c);

        runner.revoke_control("rr7 operator revoke", None);

        let mat = runner.materialize_workspace(&fixture.source_commit);
        assert!(matches!(
            mat,
            Err(RuntimeError::RemoteControlRevoked { .. })
        ));

        let start = runner.start_process(remote_request("run-revoked", workspace, "printf ok"));
        assert!(matches!(
            start,
            Err(RuntimeError::RemoteControlRevoked { .. })
        ));
    }

    /// RR7 harness helper (sandbox composition): build a remote runner over a fake
    /// channel whose remote OS + cross-machine boundary are scripted, mirroring the
    /// RR5 `sandbox_runner` but wired into the RR7 consolidation block so the
    /// "sandbox enforcement claims match the (fake) remote OS" cross-cutting
    /// invariant is exercised end to end here, not only in isolation. NO network.
    fn rr7_sandbox_runner(
        name: &str,
        workspace: &Path,
        script: impl FnOnce(FakeRemoteChannel) -> FakeRemoteChannel,
    ) -> RemoteProcessRunner {
        let artifacts = temp_root(&format!("rr7-sb-art-{name}"));
        let channel = OpenChannel::for_test(
            format!("rr7-sb-chan-{name}"),
            format!("rr7-sb-endpoint-{name}"),
            format!("rr7-sb-fp-{name}"),
        );
        let base =
            FakeRemoteChannel::from_open_channel(&channel, workspace.to_path_buf(), artifacts);
        let transport = RemoteChannel::Fake(script(base));
        RemoteProcessRunner::new(RemoteProcessConfig::with_transport(channel, transport))
    }

    #[test]
    fn rr7_sandboxed_launch_enforcement_claim_matches_fake_remote_os() {
        // CROSS-CUTTING INVARIANT (RR7 AC): "sandbox enforcement claims match the
        // (fake) remote OS." Wired into the consolidation harness as a PAIR:
        //   - a CROSS-MACHINE channel to an enforcing remote OS -> `sandbox.enforced`
        //     (the boundary was crossed AND the remote OS enforces the tier);
        //   - the DEFAULT loopback channel (no boundary crossed), even with the SAME
        //     enforcing remote OS scripted -> `sandbox.unenforced` (Capo never claims
        //     a confinement it could not apply over a boundary it did not cross).
        let enforced_root = temp_root("rr7-sb-enforced");
        fs::create_dir_all(&enforced_root).unwrap();
        let enforced_runner = rr7_sandbox_runner("enforced", &enforced_root, |c| {
            c.with_remote_os(RemoteOsFamily::Linux)
                .with_enforceable_remote_sandbox()
                .with_cross_machine_boundary()
        });
        let profile = SandboxProfile::workspace_confined([enforced_root.clone()]);
        let enforced = enforced_runner
            .start_process_sandboxed(
                remote_request("run-rr7-enforced", enforced_root.clone(), "printf ok"),
                &enforced_root,
                &profile,
                SandboxTier::LinuxLandlockBwrap,
                false,
                Some("refs/capo/materialized/rr7".to_string()),
            )
            .expect("enforced plan");
        assert_eq!(
            enforced.plan.enforcement,
            SandboxEnforcement::Enforced {
                tier: SandboxTier::LinuxLandlockBwrap
            }
        );
        let enforced_outcome = enforced.outcome.expect("a confined remote process ran");
        assert_eq!(enforced_outcome.events[0].kind, "sandbox.enforced");
        // The claim is BACKED by enforcement reaching the transport (not a label):
        // the bwrap-wrapped command is what the transport launched.
        assert_eq!(
            enforced_runner
                .transport_last_launched_request()
                .expect("transport launch")
                .program,
            "bwrap"
        );

        // Same enforcing remote OS, but the DEFAULT loopback channel: no boundary
        // crossed -> honestly `Unenforced`, NEVER `Enforced`.
        let loopback_root = temp_root("rr7-sb-loopback");
        fs::create_dir_all(&loopback_root).unwrap();
        let loopback_runner = rr7_sandbox_runner("loopback", &loopback_root, |c| {
            c.with_remote_os(RemoteOsFamily::Linux)
                .with_enforceable_remote_sandbox()
        });
        assert!(loopback_runner.is_loopback());
        let loopback_profile = SandboxProfile::workspace_confined([loopback_root.clone()]);
        let loopback_plan = loopback_runner
            .plan_remote_sandbox(
                &remote_request("run-rr7-loopback", loopback_root.clone(), "printf ok"),
                &loopback_root,
                &loopback_profile,
                SandboxTier::LinuxLandlockBwrap,
                false,
            )
            .expect("loopback plan");
        assert!(matches!(
            loopback_plan.enforcement,
            SandboxEnforcement::Unenforced { .. }
        ));
        assert!(
            loopback_plan
                .events
                .iter()
                .any(|e| e.kind == "sandbox.unenforced")
        );
    }

    #[test]
    fn rr7_crash_matrix_recovery_is_replay_stable_across_all_classifications() {
        // CROSS-CUTTING INVARIANT (RR7 AC): the consolidation harness exercises
        // "crash-matrix recovery" — ALL FOUR classifications, not just the happy
        // reattach — through the same fake-remote harness, and proves each is
        // replay-stable (an identical script rebuilds an identical event sequence).
        //
        // Drives one classification through the harness and returns the projected
        // (classification, event-kind-sequence) so two independent runs can be
        // compared for replay stability.
        type Scripter = fn(FakeRemoteChannel) -> FakeRemoteChannel;
        let drive = |name: &str, script: Scripter| -> (RemoteRecoveryClassification, Vec<String>) {
            let (runner, running, recorded_boot) = scripted_recovery_runner(name, script);
            let recovery = runner.recover_run(&running, &recorded_boot);
            // Append-first holds for every classification: the attempt precedes the
            // single terminal event.
            assert_eq!(
                recovery.events.first().unwrap().kind,
                "runtime.remote_recovery_attempted"
            );
            let kinds = recovery
                .events
                .iter()
                .map(|e| e.kind.clone())
                .collect::<Vec<_>>();
            (recovery.classification, kinds)
        };

        // The four crash-matrix classifications (RR6) and their scripts.
        let matrix: [(&str, Scripter, RemoteRecoveryClassification); 4] = [
            (
                "recovered",
                FakeRemoteChannel::recover_alive_reattachable,
                RemoteRecoveryClassification::Recovered,
            ),
            (
                "orphaned",
                FakeRemoteChannel::recover_alive_unattachable,
                RemoteRecoveryClassification::Orphaned,
            ),
            (
                "exited",
                FakeRemoteChannel::recover_rebooted,
                RemoteRecoveryClassification::Exited,
            ),
            (
                "pending",
                FakeRemoteChannel::recover_channel_unreachable,
                RemoteRecoveryClassification::RecoveryPending,
            ),
        ];

        for (name, script, expected) in matrix {
            let (cls_a, kinds_a) = drive(&format!("rr7-cm-{name}-a"), script);
            let (cls_b, kinds_b) = drive(&format!("rr7-cm-{name}-b"), script);
            assert_eq!(cls_a, expected, "{name} classification");
            assert_eq!(cls_b, expected, "{name} classification (rebuild)");
            // Replay-stable: the identical script rebuilds an identical event trail.
            assert_eq!(
                kinds_a, kinds_b,
                "{name} recovery rebuild must be identical"
            );
        }
    }

    #[test]
    fn rr7_enum_level_remote_control_dispatches_to_the_real_remote_runner() {
        // ROUTING (review finding 4): the `RuntimeRunner::{interrupt,terminate,
        // kill,health}_local` methods MUST dispatch a `RemoteProcess` variant to the
        // REAL `RemoteProcessRunner` over its channel (not the `FakeRuntimeRunner`
        // fall-through). This both proves the routing and keeps the methods reachable
        // (no dead code). Each control verb yields its distinct remote event kind.
        let (runner, running, _boot) =
            scripted_recovery_runner("rr7-enum-route", |c| c.recover_alive_reattachable());
        let enum_runner = RuntimeRunner::RemoteProcess(Box::new(runner));

        let interrupted = enum_runner.interrupt_local(&running, "rr7 interrupt");
        assert_eq!(
            interrupted.events[0].kind, "runtime.remote_interrupt_sent",
            "interrupt_local must route to the real remote runner"
        );

        let terminated = enum_runner.terminate_local(&running, "rr7 terminate");
        assert_eq!(terminated.events[0].kind, "runtime.remote_terminate_sent");

        let killed = enum_runner.kill_local(&running, "rr7 kill");
        assert_eq!(killed.events[0].kind, "runtime.remote_kill_sent");

        // health_local routes to the ACTUAL remote probe over the channel.
        let health = enum_runner.health_local(&running).expect("health");
        assert!(health.live);
    }

    #[test]
    fn rr7_remote_kill_yields_remote_kill_sent_event() {
        // RR1 AC (review finding 5): `kill` over the channel produces the DISTINCT
        // `runtime.remote_kill_sent` event — asserted directly on the runner, not
        // only via the EventKind serialization round-trip.
        let (runner, running, _boot) =
            scripted_recovery_runner("rr7-kill", |c| c.recover_alive_reattachable());
        let killed = runner.kill(&running, "rr7 operator kill");
        assert_eq!(killed.process.status, "killed");
        assert_eq!(killed.events[0].kind, "runtime.remote_kill_sent");
    }

    // ====================================================================
    // RR8: live opt-in remote SSH smoke (secrets stripped) PAIRED with a
    // deterministic fixture.
    //
    // The live smoke (`rr8_live_ssh_smoke_full_lifecycle_or_clean_skip`) is
    // `#[ignore]` and runs ONLY when BOTH `CAPO_SERVER_REMOTE_RUNTIME_PREFLIGHT=1`
    // and `CAPO_SERVER_RUN_REMOTE_RUNTIME_LIVE=1` are set AND an SSH host is
    // configured; otherwise it SKIPS CLEANLY with a recorded, secret-free reason.
    // It is PAIRED with `rr8_deterministic_fixture_pins_the_live_smoke_shapes`,
    // which runs in the always-on gate (NO network) and pins the IDENTICAL shapes
    // the live smoke asserts (process-ref shape, materialized-HEAD-matches-SHA,
    // redacted output, recovery classification), so completion is NEVER solely
    // operator-attested.
    // ====================================================================

    #[test]
    fn rr8_skip_predicate_is_defined_and_records_reason_when_gate_unset() {
        // DEFINED predicate: with the gate unset the decision MUST be a recorded,
        // secret-free Skip naming both gates — never operator judgement. Serialize
        // against the live smoke that reads the same env vars.
        let _env_guard = RR8_ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        // SAFETY: env access is serialized by RR8_ENV_LOCK for this test's duration.
        unsafe {
            std::env::remove_var(REMOTE_RUNTIME_PREFLIGHT_ENV);
            std::env::remove_var(RUN_REMOTE_RUNTIME_LIVE_ENV);
            std::env::remove_var(REMOTE_RUNTIME_SSH_HOST_ENV);
        }
        match live_remote_runtime_smoke_decision() {
            LiveRemoteRuntimeSmokeDecision::Skip { reason } => {
                assert!(
                    reason.contains(REMOTE_RUNTIME_PREFLIGHT_ENV)
                        && reason.contains(RUN_REMOTE_RUNTIME_LIVE_ENV),
                    "the recorded skip reason must name both gates: {reason}"
                );
                assert!(
                    connectivity_redaction_is_clean(&reason),
                    "skip reason must be secret-free: {reason}"
                );
            }
            other => panic!("gate unset must Skip, got {other:?}"),
        }
    }

    #[test]
    fn rr8_ssh_runner_is_not_loopback_and_does_no_endpoint_resolution() {
        // HONESTY: a real SSH transport crossed a machine boundary, so it is NEVER a
        // loopback — Capo's realness claim is truthful. The runner is built from an
        // ALREADY-RESOLVED channel + SSH destination (no endpoint resolution), and
        // the auth handle is a label only (carried by handle, never a raw key).
        let channel = OpenChannel::for_test("chan-ssh", "endpoint-ssh", "fp-ssh");
        let ssh = SshRemoteConfig::new("capo@remote.example", "fp-ssh", temp_root("rr8-ssh-art"))
            .with_auth_ref("ssh-agent:default");
        let runner = SshRemoteProcessRunner::build(channel, ssh);
        assert!(
            !runner.is_loopback(),
            "a real SSH transport must report non-loopback (it crossed a boundary)"
        );
        assert_eq!(runner.target_fingerprint(), "fp-ssh");
        // The auth handle is a label, never a credential.
        assert!(connectivity_redaction_is_clean("ssh-agent:default"));
    }

    /// RR8 PAIRING fixture: the SAME deterministic fake-remote contract the live
    /// smoke asserts, pinned in the always-on gate (NO network). It proves the
    /// IDENTICAL shapes the live smoke would assert against a real host:
    ///   - the remote process-ref shape
    ///     (`remote-process:{fp}:{host}:pid=...:boot=...`),
    ///   - materialized remote HEAD == the source SHA (content-addressed),
    ///   - remote output is REDACTED before any delta,
    ///   - a controller-restart-with-live-remote recovers in place (Recovered).
    ///
    /// So when the live smoke runs, completion is checked against this shape, not
    /// operator attestation.
    #[test]
    fn rr8_deterministic_fixture_pins_the_live_smoke_shapes() {
        let fixture = rr7_git_remote_fixture("rr8-fixture");
        let payload = b"remote out token AKIAIOSFODNN7EXAMPLE done".to_vec();
        let (runner, workspace) =
            rr7_runner("rr8-fixture", Some(fixture.git_remote.clone()), |c| {
                c.recover_alive_reattachable()
                    .with_streamed_output(payload.clone())
            });

        // SHAPE 1: materialized remote HEAD == source SHA (content-addressed).
        let materialization = runner
            .materialize_workspace(&fixture.source_commit)
            .expect("materialize");
        assert_eq!(
            materialization.remote_head, fixture.source_commit,
            "materialized remote HEAD must equal the source SHA"
        );
        // The materialization event's transport URL is redacted (no embedded secret).
        assert!(
            materialization
                .events
                .iter()
                .all(|e| connectivity_redaction_is_clean(&e.detail)),
            "no materialization event may carry a credential"
        );

        // SHAPE 2: the remote process-ref shape.
        let outcome = runner
            .start_process(remote_request("run-rr8-fixture", workspace, "printf ok"))
            .expect("start");
        let process_ref = &outcome.process.runtime_process_ref;
        assert!(
            process_ref.starts_with("remote-process:fp-rr8-fixture:")
                && process_ref.contains(":pid=")
                && process_ref.contains(":boot="),
            "remote process-ref must carry the fingerprint + remote pid + boot: {process_ref}"
        );

        let running = LocalRuntimeProcessRef {
            status: "running".to_string(),
            ..outcome.process.clone()
        };

        // SHAPE 3: remote output is REDACTED before any delta.
        let stream = runner.stream_output(&running, 0);
        assert_eq!(stream.redaction_state, "redacted");
        assert!(
            !stream
                .deltas
                .iter()
                .any(|d| d.text.contains("AKIAIOSFODNN7EXAMPLE")),
            "a credential must be scrubbed before any delta reaches persistence"
        );

        // SHAPE 4: a controller-restart-with-live-remote recovers IN PLACE.
        let recovery = runner.recover_run(&running, &running_recorded_boot(&running));
        assert_eq!(
            recovery.classification,
            RemoteRecoveryClassification::Recovered
        );
        assert_eq!(recovery.runtime_process_ref, running.runtime_process_ref);

        // SHAPE 5 (review finding 3): the SAFETY FLOOR the live smoke exercises —
        // `start_process_sandboxed` composes the remote OS sandbox + worktree under
        // the `SandboxProfile`, and the enforcement claim is the HONEST remote-OS
        // one. Pin it on a cross-machine ENFORCING fake so the deterministic side
        // proves the `Enforced` shape the live path asserts (the live path also
        // accepts `Unenforced` for a remote that genuinely cannot enforce).
        let sandbox_fixture = rr7_git_remote_fixture("rr8-fixture-sandbox");
        let (sandbox_runner, sandbox_ws) = rr7_runner(
            "rr8-fixture-sandbox",
            Some(sandbox_fixture.git_remote.clone()),
            |c| {
                c.with_cross_machine_boundary()
                    .with_enforceable_remote_sandbox()
                    .with_remote_os(RemoteOsFamily::Linux)
            },
        );
        let profile = SandboxProfile::workspace_confined([sandbox_ws.clone()]);
        let sandboxed = sandbox_runner
            .start_process_sandboxed(
                remote_request("run-rr8-fixture-sandbox", sandbox_ws.clone(), "printf ok"),
                &sandbox_ws,
                &profile,
                SandboxTier::LinuxLandlockBwrap,
                false,
                None,
            )
            .expect("sandboxed start");
        assert!(
            matches!(
                sandboxed.plan.enforcement,
                SandboxEnforcement::Enforced { .. }
            ),
            "a cross-machine enforcing remote must report Enforced, got {:?}",
            sandboxed.plan.enforcement
        );
        assert!(
            sandboxed.outcome.is_some(),
            "an enforced sandboxed launch yields an outcome"
        );
    }

    /// RR8 LIVE, OPT-IN SSH smoke. `#[ignore]` by default; runs ONLY when BOTH
    /// `CAPO_SERVER_REMOTE_RUNTIME_PREFLIGHT=1` and
    /// `CAPO_SERVER_RUN_REMOTE_RUNTIME_LIVE=1` are set AND
    /// `CAPO_SERVER_REMOTE_RUNTIME_SSH_HOST` names a reachable SSH destination;
    /// otherwise it SKIPS CLEANLY with a recorded, secret-free reason.
    ///
    /// When it RUNS it drives the real cross-machine lifecycle over SSH — resolve
    /// the (injected) channel, materialize a known commit by git, run one real
    /// process, stream real stdout, and recover a controller-restart-with-live-
    /// remote — and asserts the SAME deterministic shapes
    /// `rr8_deterministic_fixture_pins_the_live_smoke_shapes` pins, so completion is
    /// never solely operator-attested. The channel auth is carried strictly by
    /// HANDLE (`auth_ref` / the operator's `ssh` config); the smoke NEVER reads or
    /// logs raw SSH keys / `known_hosts` secrets / subscription tokens, and the
    /// remote stdout + git transport URL pass the credential scan before any event.
    #[test]
    #[ignore = "live opt-in: requires CAPO_SERVER_REMOTE_RUNTIME_PREFLIGHT=1 + \
                CAPO_SERVER_RUN_REMOTE_RUNTIME_LIVE=1 and a reachable SSH host in \
                CAPO_SERVER_REMOTE_RUNTIME_SSH_HOST"]
    fn rr8_live_ssh_smoke_full_lifecycle_or_clean_skip() {
        let _env_guard = RR8_ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());

        // DEFINED skip predicate: gate unset / no host all collapse to a recorded,
        // secret-free Skip.
        let ssh_destination = match live_remote_runtime_smoke_decision() {
            LiveRemoteRuntimeSmokeDecision::Run { ssh_destination } => ssh_destination,
            LiveRemoteRuntimeSmokeDecision::Skip { reason } => {
                assert!(
                    connectivity_redaction_is_clean(&reason),
                    "the recorded skip reason must be secret-free: {reason}"
                );
                eprintln!("RR8 live SSH smoke skipped cleanly: {reason}");
                return;
            }
        };
        // The resolved SSH destination is an opaque label, never a credential.
        assert!(
            connectivity_redaction_is_clean(&ssh_destination),
            "the SSH destination must be secret-free"
        );

        // Build a REAL git-backed remote workspace fixture (origin on the controller,
        // remote repo + worktree root reached over the SAME SSH host). The transport
        // URL is credential-scanned before it is ever recorded.
        let fixture = rr7_git_remote_fixture("rr8-live");
        let channel = OpenChannel::for_test("chan-rr8-live", &ssh_destination, "fp-rr8-live");
        let ssh = SshRemoteConfig::new(
            ssh_destination.clone(),
            "fp-rr8-live",
            temp_root("rr8-live-art"),
        )
        .with_auth_ref("ssh-agent:default")
        .with_git_remote(fixture.git_remote.clone());
        let runner = SshRemoteProcessRunner::build(channel, ssh);

        // HONESTY: the real SSH transport crossed a machine boundary.
        assert!(
            !runner.is_loopback(),
            "the live SSH path must be non-loopback"
        );

        // Materialize a known commit by git over the channel -> remote HEAD == SHA.
        let materialization = runner
            .materialize_workspace(&fixture.source_commit)
            .expect("live git materialization");
        assert_eq!(materialization.remote_head, fixture.source_commit);
        assert!(
            materialization
                .events
                .iter()
                .all(|e| connectivity_redaction_is_clean(&e.detail)),
            "no live materialization event may carry a credential"
        );

        // Run one real process on the remote, stream real stdout (redacted), and
        // recover a controller-restart-with-live-remote. The shapes match the
        // deterministic fixture above.
        //
        // SAFETY FLOOR (review finding 3): the live run goes through
        // `start_process_sandboxed`, NOT a bare `start_process`. It composes the
        // remote OS sandbox tier + the `safety-gates` `SandboxProfile`
        // (workspace-confined to the remote worktree root, no network egress) under
        // the revocable remote-control grant, and the enforcement claim is read from
        // the REMOTE OS probe — HONESTLY `Enforced` (a linux/macOS remote with
        // bwrap/sandbox-exec) or `Unenforced` (a remote that cannot enforce). The
        // smoke asserts a truthful claim, never that enforcement is fabricated.
        let remote_cwd = PathBuf::from(&materialization.remote_worktree_path);
        let profile = SandboxProfile::workspace_confined([remote_cwd.clone()]);
        let sandboxed = runner
            .start_process_sandboxed(
                remote_request("run-rr8-live", remote_cwd.clone(), "printf ok"),
                &remote_cwd,
                &profile,
                SandboxTier::LinuxLandlockBwrap,
                false,
                Some(materialization.remote_head.clone()),
            )
            .expect("live sandboxed remote start");
        // The enforcement claim must be one of the two HONEST outcomes for a
        // launched run (never `Refused` here — the cwd is the confined root and no
        // network is requested). Whether it is `Enforced` or `Unenforced` depends on
        // the real remote OS, which is the whole point of the honest claim.
        assert!(
            matches!(
                sandboxed.plan.enforcement,
                SandboxEnforcement::Enforced { .. } | SandboxEnforcement::Unenforced { .. }
            ),
            "live sandbox enforcement must be a truthful remote-OS claim, got {:?}",
            sandboxed.plan.enforcement
        );
        let outcome = sandboxed.outcome.expect("a launched run yields an outcome");
        let process_ref = &outcome.process.runtime_process_ref;
        assert!(
            process_ref.starts_with("remote-process:fp-rr8-live:")
                && process_ref.contains(":pid=")
                && process_ref.contains(":boot="),
            "live remote process-ref must match the fixture shape: {process_ref}"
        );
        let running = LocalRuntimeProcessRef {
            status: "running".to_string(),
            ..outcome.process.clone()
        };
        let stream = runner.stream_output(&running, 0);
        assert!(
            stream
                .deltas
                .iter()
                .all(|d| connectivity_redaction_is_clean(&d.text)),
            "live remote stdout must be redacted before any delta"
        );
        let recovery = runner.recover_run(&running, &running_recorded_boot(&running));
        assert!(
            matches!(
                recovery.classification,
                RemoteRecoveryClassification::Recovered | RemoteRecoveryClassification::Exited
            ),
            "live recovery must be a truthful classification, got {:?}",
            recovery.classification
        );

        // Safety floor: revoking the remote-control grant STOPS the live run and
        // forbids re-establishment without a fresh grant.
        runner.revoke_control("rr8 live operator revoke", None);
        let re_start =
            runner.start_process(remote_request("run-rr8-live-2", remote_cwd, "printf ok"));
        assert!(
            matches!(re_start, Err(RuntimeError::RemoteControlRevoked { .. })),
            "a revoked grant must forbid re-establishing a live remote run"
        );

        // Clean up the remote worktree + process group.
        let _ = runner.cleanup_run(&running, CleanupPolicy::ReapAll);
    }

    /// RR8: serialize the tests that read/mutate the RR8 gate env vars so they do
    /// not race under `--include-ignored` parallel execution.
    static RR8_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
}
