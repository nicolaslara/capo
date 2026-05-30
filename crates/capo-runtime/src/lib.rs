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
            status: "exited".to_string(),
            redaction_state: stdout.redaction_state.clone(),
        };
        let stdout_artifact = self.write_artifact(
            &request.run_id,
            "stdout",
            &stdout.bytes,
            &stdout.redaction_state,
        )?;
        let stderr_artifact = self.write_artifact(
            &request.run_id,
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

        let run_dir = self.config.artifact_root.join(request.run_id.as_str());
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
                status: "running".to_string(),
                redaction_state: "redacted".to_string(),
            },
            child,
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
            "stdout",
            &process.stdout_path,
            &stdout.bytes,
            &stdout.redaction_state,
        );
        let stderr_artifact = self.output_artifact_from_path(
            &process.process.run_id,
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
                let _ = Command::new("/bin/kill")
                    .arg("-TERM")
                    .arg(format!("-{pid}"))
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
                thread::sleep(Duration::from_millis(100));
                let _ = Command::new("/bin/kill")
                    .arg("-KILL")
                    .arg(format!("-{pid}"))
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
        }
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
        let mut text = String::from_utf8_lossy(bytes).to_string();
        let mut redacted = false;
        for rule in &self.config.redaction_rules {
            if text.contains(&rule.pattern) {
                text = text.replace(&rule.pattern, &rule.replacement);
                redacted = true;
            }
        }
        RedactedOutput {
            bytes: text.into_bytes(),
            redaction_state: if redacted { "redacted" } else { "safe" }.to_string(),
        }
    }

    fn ensure_cwd_allowed(&self, cwd: &Path) -> RuntimeResult<()> {
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

    fn write_artifact(
        &self,
        run_id: &RunId,
        stream: &str,
        bytes: &[u8],
        redaction_state: &str,
    ) -> RuntimeResult<RuntimeOutputArtifact> {
        let run_dir = self.config.artifact_root.join(run_id.as_str());
        fs::create_dir_all(&run_dir)?;
        let path = run_dir.join(format!("{stream}.txt"));
        fs::write(&path, bytes)?;
        Ok(RuntimeOutputArtifact {
            artifact_id: format!("artifact-runtime-{run_id}-{stream}"),
            path,
            size_bytes: bytes.len() as i64,
            content_hash: content_hash(bytes),
            redaction_state: redaction_state.to_string(),
        })
    }

    fn output_artifact_from_path(
        &self,
        run_id: &RunId,
        stream: &str,
        path: &Path,
        bytes: &[u8],
        redaction_state: &str,
    ) -> RuntimeOutputArtifact {
        RuntimeOutputArtifact {
            artifact_id: format!("artifact-runtime-{run_id}-{stream}"),
            path: path.to_path_buf(),
            size_bytes: bytes.len() as i64,
            content_hash: content_hash(bytes),
            redaction_state: redaction_state.to_string(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalProcessRequest {
    pub run_id: RunId,
    pub program: String,
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    pub env: HashMap<String, String>,
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
    stdout_path: PathBuf,
    stderr_path: PathBuf,
    pub events: Vec<RuntimeEvent>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeOutputArtifact {
    pub artifact_id: String,
    pub path: PathBuf,
    pub size_bytes: i64,
    pub content_hash: String,
    pub redaction_state: String,
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
}

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

fn normalize_path(path: &Path) -> RuntimeResult<PathBuf> {
    if path.exists() {
        Ok(path.canonicalize()?)
    } else {
        Ok(path.to_path_buf())
    }
}

fn content_hash(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("fnv1a64:{hash:016x}")
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
    fn local_process_runner_can_kill_a_live_child_and_collect_artifacts() {
        let workspace = temp_root("workspace-live");
        let artifacts = temp_root("artifacts-live");
        fs::create_dir_all(&workspace).unwrap();
        let runner =
            LocalProcessRunner::new(LocalProcessConfig::for_test(workspace.clone(), artifacts));

        let mut running = runner
            .spawn_process(LocalProcessRequest {
                run_id: RunId::new("run-live"),
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
    fn local_process_runner_times_out_and_collects_partial_artifacts() {
        let workspace = temp_root("workspace-timeout");
        let artifacts = temp_root("artifacts-timeout");
        fs::create_dir_all(&workspace).unwrap();
        let runner =
            LocalProcessRunner::new(LocalProcessConfig::for_test(workspace.clone(), artifacts));

        let mut running = runner
            .spawn_process(LocalProcessRequest {
                run_id: RunId::new("run-timeout"),
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
                program: "/bin/echo".to_string(),
                argv: vec!["nope".to_string()],
                cwd: outside,
                env: HashMap::new(),
            })
            .unwrap_err();

        assert!(matches!(error, RuntimeError::CwdOutsideWorkspace { .. }));
    }

    fn temp_root(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!("capo-runtime-{name}-{nanos}"))
    }
}
