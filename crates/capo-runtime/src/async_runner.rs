//! tokio-based streaming local process runner (ST1).
//!
//! The synchronous [`crate::LocalProcessRunner`] runs a child to exit and only
//! then buffers, caps, and persists its output. Under a streaming transport the
//! controller wants the output as it is produced, a way to write to a run's
//! stdin mid-flight, and a cancel that reaps the whole process group. This
//! module adds [`AsyncLocalProcessRunner`] for exactly that, while leaving the
//! synchronous runner (and the `Fake`/`RemoteProcessRunner` shapes) untouched so
//! existing deterministic tests never need a tokio reactor.
//!
//! Design notes:
//!
//! - **Incremental output.** Each child stdout/stderr chunk is forwarded over an
//!   in-process channel as a `runtime.output_delta` [`RuntimeEvent`] the caller
//!   can await with [`AsyncRunningProcess::next_delta`], instead of the
//!   buffer-then-cap-after-exit shape.
//! - **stdin.** [`AsyncRunningProcess::write_stdin`] writes to the live child's
//!   stdin so the controller can talk to a process mid-flight.
//! - **Provable descendant reaping.** The child is spawned in its own process
//!   group (`process_group(0)`), and [`AsyncRunningProcess::cancel`] reuses the
//!   same `SIGTERM` then `SIGKILL` process-group teardown the synchronous
//!   runner's timeout/hard-kill paths use, so a cancelled run leaves no
//!   surviving descendant process group.
//! - **Output-cap classification.** Output is streamed and TRUNCATED at the cap
//!   rather than discarded: a successful run that exceeds the cap keeps its
//!   (capped) artifact, is classified `exited` (not `failed`), and records
//!   `truncated = true` as artifact metadata.

use std::path::PathBuf;
use std::process::Stdio;

use capo_core::RunId;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::task::JoinHandle;

use crate::{
    LocalProcessConfig, LocalProcessRequest, LocalProcessRunner, LocalRuntimeProcessRef,
    RedactionPolicy, RuntimeError, RuntimeEvent, RuntimeOutputArtifact, RuntimeResult,
};

/// The maximum number of bytes read from a child pipe in a single incremental
/// delta. Small enough that a streaming caller observes progress, large enough
/// that an ordinary run produces few deltas.
const DELTA_CHUNK_BYTES: usize = 8 * 1024;

/// Which standard stream a [`runtime.output_delta`](RuntimeEvent) carries.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StreamSource {
    Stdout,
    Stderr,
}

impl StreamSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::Stdout => "stdout",
            Self::Stderr => "stderr",
        }
    }
}

/// A streaming local-process runner backed by tokio.
///
/// Reuses [`LocalProcessConfig`] (workspace roots, artifact root, env allowlist,
/// redaction rules, output cap) and the synchronous runner's cwd/allowlist
/// guards, so it enforces the same boundary as [`LocalProcessRunner`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AsyncLocalProcessRunner {
    inner: LocalProcessRunner,
}

impl AsyncLocalProcessRunner {
    pub fn new(config: LocalProcessConfig) -> Self {
        Self {
            inner: LocalProcessRunner::new(config),
        }
    }

    fn config(&self) -> &LocalProcessConfig {
        self.inner.config()
    }

    /// Spawn `request` as a streaming child process.
    ///
    /// The child runs in its own process group, with stdin/stdout/stderr piped.
    /// stdout and stderr are drained by background tasks that forward each chunk
    /// as a `runtime.output_delta` event and accumulate the (capped) bytes for
    /// the final artifacts. Returns immediately with a live
    /// [`AsyncRunningProcess`]; the child keeps running.
    pub fn spawn_streaming(
        &self,
        request: LocalProcessRequest,
    ) -> RuntimeResult<AsyncRunningProcess> {
        self.inner.ensure_cwd_allowed(&request.cwd)?;
        std::fs::create_dir_all(&self.config().artifact_root)?;
        let run_dir = self
            .inner
            .run_dir_for(&request.run_id, request.turn_id.as_deref());
        std::fs::create_dir_all(&run_dir)?;
        let stdout_path = run_dir.join("stdout.txt");
        let stderr_path = run_dir.join("stderr.txt");

        let mut command = tokio::process::Command::new(&request.program);
        command.args(&request.argv);
        command.current_dir(&request.cwd);
        command.env_clear();
        for name in &self.config().env_allowlist {
            if let Ok(value) = std::env::var(name) {
                command.env(name, value);
            }
        }
        for (name, value) in &request.env {
            if self
                .config()
                .env_allowlist
                .iter()
                .any(|allowed| allowed == name)
            {
                command.env(name, value);
            } else {
                return Err(RuntimeError::DisallowedEnvOverride(name.clone()));
            }
        }
        command.stdin(Stdio::piped());
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());
        #[cfg(unix)]
        {
            command.process_group(0);
        }
        // Reap the child on drop only as a backstop; we wait/cancel explicitly.
        command.kill_on_drop(true);

        let mut child = command.spawn()?;
        let external_pid = child.id();
        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        let cap = self.config().output_limit_bytes;
        let (tx, rx) = mpsc::unbounded_channel();
        let stdout_reader = stdout.map(|pipe| {
            spawn_stream_reader(
                pipe_reader_stdout(pipe),
                StreamSource::Stdout,
                cap,
                tx.clone(),
            )
        });
        let stderr_reader = stderr.map(|pipe| {
            spawn_stream_reader(
                pipe_reader_stderr(pipe),
                StreamSource::Stderr,
                cap,
                tx.clone(),
            )
        });
        drop(tx);

        let process = LocalRuntimeProcessRef {
            run_id: request.run_id.clone(),
            runtime_process_ref: format!("local-process-{}", request.run_id),
            external_pid,
            boot_id: crate::boot_id(),
            status: "running".to_string(),
            redaction_state: "redacted".to_string(),
        };

        Ok(AsyncRunningProcess {
            child: Some(child),
            stdin,
            process,
            run_id: request.run_id,
            turn_id: request.turn_id,
            stdout_path,
            stderr_path,
            cap,
            redaction_rules: self.config().redaction_rules.clone(),
            deltas: rx,
            stdout_reader,
            stderr_reader,
            spawn_events: vec![
                RuntimeEvent {
                    kind: "runtime.start_requested".to_string(),
                    status: "pending".to_string(),
                    detail: request.program,
                },
                RuntimeEvent {
                    kind: "runtime.process_started".to_string(),
                    status: "started".to_string(),
                    detail: external_pid
                        .map(|pid| pid.to_string())
                        .unwrap_or_else(|| "unknown".to_string()),
                },
            ],
        })
    }
}

/// A reader task's collected result for one stream: the (capped) bytes that will
/// become the artifact and whether they were truncated at the cap.
#[derive(Debug)]
struct StreamCapture {
    bytes: Vec<u8>,
    truncated: bool,
}

/// A pipe that can be read in chunks. Abstracts over tokio stdout/stderr so the
/// reader task is generic without naming both concrete pipe types.
type BoxedPipe = std::pin::Pin<Box<dyn tokio::io::AsyncRead + Send>>;

fn pipe_reader_stdout(pipe: ChildStdout) -> BoxedPipe {
    Box::pin(pipe)
}

fn pipe_reader_stderr(pipe: ChildStderr) -> BoxedPipe {
    Box::pin(pipe)
}

/// Spawn the background task that drains one child pipe.
///
/// It reads in [`DELTA_CHUNK_BYTES`] chunks, forwarding each chunk as a
/// `runtime.output_delta` event over `tx` and accumulating the bytes up to
/// `cap`. Once the cap is reached the task keeps DRAINING the pipe (so the child
/// never blocks on a full pipe) but stops accumulating and records the stream as
/// truncated. Returns the final capped capture for the artifact.
fn spawn_stream_reader(
    mut pipe: BoxedPipe,
    source: StreamSource,
    cap: usize,
    tx: UnboundedSender<RuntimeEvent>,
) -> JoinHandle<StreamCapture> {
    tokio::spawn(async move {
        let mut collected: Vec<u8> = Vec::new();
        let mut truncated = false;
        let mut buf = [0u8; DELTA_CHUNK_BYTES];
        loop {
            match pipe.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    let chunk = &buf[..n];
                    // Forward the incremental delta regardless of the cap so a
                    // live subscriber sees progress; the cap only bounds what is
                    // persisted to the durable artifact.
                    let _ = tx.send(RuntimeEvent {
                        kind: "runtime.output_delta".to_string(),
                        status: source.as_str().to_string(),
                        detail: String::from_utf8_lossy(chunk).to_string(),
                    });
                    if collected.len() < cap {
                        let remaining = cap - collected.len();
                        if n <= remaining {
                            collected.extend_from_slice(chunk);
                        } else {
                            collected.extend_from_slice(&chunk[..remaining]);
                            truncated = true;
                        }
                    } else {
                        truncated = true;
                    }
                }
                Err(_) => break,
            }
        }
        StreamCapture {
            bytes: collected,
            truncated,
        }
    })
}

/// A live streaming child process.
///
/// Holds the spawned child, its stdin handle, the incremental delta receiver,
/// and the two reader tasks that drain stdout/stderr. The caller awaits deltas
/// with [`Self::next_delta`], writes to stdin with [`Self::write_stdin`], may
/// [`Self::cancel`] mid-flight, and finally [`Self::wait`]s to collect the
/// redacted artifacts and a success/failure classification.
#[derive(Debug)]
pub struct AsyncRunningProcess {
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    process: LocalRuntimeProcessRef,
    run_id: RunId,
    turn_id: Option<String>,
    stdout_path: PathBuf,
    stderr_path: PathBuf,
    cap: usize,
    redaction_rules: Vec<crate::RedactionRule>,
    deltas: UnboundedReceiver<RuntimeEvent>,
    stdout_reader: Option<JoinHandle<StreamCapture>>,
    stderr_reader: Option<JoinHandle<StreamCapture>>,
    spawn_events: Vec<RuntimeEvent>,
}

impl AsyncRunningProcess {
    /// The runtime process reference (pid, boot id, status, runtime ref).
    pub fn process(&self) -> &LocalRuntimeProcessRef {
        &self.process
    }

    /// The spawned child's OS pid, if it was assigned one.
    pub fn external_pid(&self) -> Option<u32> {
        self.process.external_pid
    }

    /// The `runtime.start_requested` / `runtime.process_started` events recorded
    /// at spawn time.
    pub fn spawn_events(&self) -> &[RuntimeEvent] {
        &self.spawn_events
    }

    /// Await the next incremental `runtime.output_delta` event, or `None` once
    /// both stdout and stderr have closed and been fully drained.
    pub async fn next_delta(&mut self) -> Option<RuntimeEvent> {
        self.deltas.recv().await
    }

    /// Write `bytes` to the live child's stdin.
    ///
    /// Errors if stdin was already closed (e.g. after [`Self::wait`] consumed the
    /// handle) or the write fails.
    pub async fn write_stdin(&mut self, bytes: &[u8]) -> RuntimeResult<()> {
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| RuntimeError::Io(std::io::Error::other("stdin is closed")))?;
        stdin.write_all(bytes).await?;
        stdin.flush().await?;
        Ok(())
    }

    /// Close the child's stdin (signals EOF to the process).
    pub async fn close_stdin(&mut self) -> RuntimeResult<()> {
        if let Some(mut stdin) = self.stdin.take() {
            stdin.shutdown().await?;
        }
        Ok(())
    }

    /// Cancel the run mid-flight, reaping the whole process group.
    ///
    /// Reuses the same `SIGTERM` then `SIGKILL` process-group teardown the
    /// synchronous runner uses, so a backgrounded descendant of the run is reaped
    /// rather than orphaned. Marks the process `cancelled` and returns the
    /// `runtime.interrupt_requested` event.
    pub async fn cancel(&mut self, reason: &str) -> RuntimeEvent {
        self.terminate_process_group().await;
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill().await;
        }
        self.process.status = "cancelled".to_string();
        RuntimeEvent {
            kind: "runtime.interrupt_requested".to_string(),
            status: "cancelled".to_string(),
            detail: reason.to_string(),
        }
    }

    /// Send `SIGTERM` then `SIGKILL` to the child's process group.
    async fn terminate_process_group(&self) {
        #[cfg(unix)]
        {
            if let Some(pid) = self.process.external_pid {
                crate::kill_process_group(pid, "-TERM");
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                crate::kill_process_group(pid, "-KILL");
            }
        }
    }

    /// Wait for the child to exit, then collect the redacted artifacts.
    ///
    /// Drains any remaining incremental output, redacts the (capped) captured
    /// bytes, writes the stdout/stderr artifacts (recording `truncated` as
    /// artifact metadata), and classifies the run:
    ///
    /// - a clean exit is `exited` even when output was truncated at the cap (the
    ///   output-cap-discards-success fix: truncation is metadata, not failure);
    /// - a non-zero exit is `failed`;
    /// - a previously [`Self::cancel`]led run stays `cancelled`.
    pub async fn wait(mut self) -> RuntimeResult<StreamingOutcome> {
        let was_cancelled = self.process.status == "cancelled";
        let status = match self.child.take() {
            Some(mut child) => Some(child.wait().await?),
            None => None,
        };
        // Close stdin so the reader tasks can finish if the child was waiting on
        // input, then join the drain tasks to recover the captured bytes.
        let _ = self.close_stdin().await;
        let stdout_capture = join_capture(self.stdout_reader.take()).await;
        let stderr_capture = join_capture(self.stderr_reader.take()).await;

        let stdout = self.persist_stream("stdout", &self.stdout_path, &stdout_capture)?;
        let stderr = self.persist_stream("stderr", &self.stderr_path, &stderr_capture)?;

        let exit_code = status.as_ref().and_then(|status| status.code());
        let exit_status = if was_cancelled {
            "cancelled"
        } else if status
            .as_ref()
            .map(|status| status.success())
            .unwrap_or(false)
        {
            // A successful run is `exited` regardless of truncation: the cap
            // truncates output, it does NOT turn success into failure.
            "exited"
        } else {
            "failed"
        };
        self.process.status = exit_status.to_string();
        self.process.redaction_state = stdout.redaction_state.clone();

        let output_detail = format!("{},{}", stdout.artifact_id, stderr.artifact_id);
        let truncated = stdout.truncated || stderr.truncated;
        let mut events = vec![
            RuntimeEvent {
                kind: "runtime.output_artifact_recorded".to_string(),
                status: stdout.redaction_state.clone(),
                detail: if truncated {
                    format!("{output_detail} (truncated)")
                } else {
                    output_detail
                },
            },
            RuntimeEvent {
                kind: "runtime.process_exited".to_string(),
                status: exit_status.to_string(),
                detail: exit_code
                    .map(|code| code.to_string())
                    .unwrap_or_else(|| "signal".to_string()),
            },
        ];
        if truncated {
            events.insert(
                0,
                RuntimeEvent {
                    kind: "runtime.output_truncated".to_string(),
                    status: "truncated".to_string(),
                    detail: format!("output capped at {} bytes", self.cap),
                },
            );
        }

        Ok(StreamingOutcome {
            process: self.process.clone(),
            stdout,
            stderr,
            exit_code,
            truncated,
            events,
        })
    }

    fn persist_stream(
        &self,
        stream: &str,
        path: &std::path::Path,
        capture: &StreamCapture,
    ) -> RuntimeResult<RuntimeOutputArtifact> {
        let (bytes, redaction_state) =
            RedactionPolicy::new(self.redaction_rules.clone()).apply(&capture.bytes);
        std::fs::write(path, &bytes)?;
        Ok(RuntimeOutputArtifact {
            artifact_id: crate::artifact_id_for(&self.run_id, self.turn_id.as_deref(), stream),
            path: path.to_path_buf(),
            size_bytes: bytes.len() as i64,
            content_hash: crate::content_hash(&bytes),
            redaction_state,
            truncated: capture.truncated,
        })
    }
}

async fn join_capture(handle: Option<JoinHandle<StreamCapture>>) -> StreamCapture {
    match handle {
        Some(handle) => handle.await.unwrap_or(StreamCapture {
            bytes: Vec::new(),
            truncated: false,
        }),
        None => StreamCapture {
            bytes: Vec::new(),
            truncated: false,
        },
    }
}

/// The terminal result of a streaming run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamingOutcome {
    pub process: LocalRuntimeProcessRef,
    pub stdout: RuntimeOutputArtifact,
    pub stderr: RuntimeOutputArtifact,
    pub exit_code: Option<i32>,
    /// Whether either stream was truncated at the output cap.
    pub truncated: bool,
    pub events: Vec<RuntimeEvent>,
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!("capo-runtime-async-{name}-{nanos}"))
    }

    fn runner(workspace: &std::path::Path, artifacts: PathBuf) -> AsyncLocalProcessRunner {
        AsyncLocalProcessRunner::new(LocalProcessConfig::for_test(
            workspace.to_path_buf(),
            artifacts,
        ))
    }

    #[tokio::test]
    async fn streams_output_deltas_incrementally_before_exit() {
        // The runner emits stdout/stderr as `runtime.output_delta` events as the
        // child produces them, instead of buffering-then-capping after exit.
        let workspace = temp_root("workspace-stream");
        let artifacts = temp_root("artifacts-stream");
        std::fs::create_dir_all(&workspace).unwrap();
        let runner = runner(&workspace, artifacts);

        let mut running = runner
            .spawn_streaming(LocalProcessRequest::new(
                RunId::new("run-stream"),
                "/bin/sh",
                vec![
                    "-c".to_string(),
                    "printf chunk-one; printf chunk-err >&2; printf chunk-two".to_string(),
                ],
                workspace.clone(),
                HashMap::new(),
            ))
            .expect("spawn streaming");

        let mut stdout_seen = String::new();
        let mut stderr_seen = String::new();
        while let Some(delta) = running.next_delta().await {
            assert_eq!(delta.kind, "runtime.output_delta");
            match delta.status.as_str() {
                "stdout" => stdout_seen.push_str(&delta.detail),
                "stderr" => stderr_seen.push_str(&delta.detail),
                other => panic!("unexpected delta stream {other}"),
            }
        }

        let outcome = running.wait().await.expect("wait");
        assert_eq!(outcome.process.status, "exited");
        assert_eq!(outcome.exit_code, Some(0));
        assert!(!outcome.truncated);
        // The incremental deltas reconstruct the full output.
        assert_eq!(stdout_seen, "chunk-onechunk-two");
        assert_eq!(stderr_seen, "chunk-err");
        // And the durable artifact holds the same bytes.
        assert_eq!(
            std::fs::read_to_string(&outcome.stdout.path).unwrap(),
            "chunk-onechunk-two"
        );
    }

    #[tokio::test]
    async fn writes_to_stdin_mid_flight() {
        // The controller can talk to a live process via stdin; the child echoes
        // what it reads back to stdout.
        let workspace = temp_root("workspace-stdin");
        let artifacts = temp_root("artifacts-stdin");
        std::fs::create_dir_all(&workspace).unwrap();
        let runner = runner(&workspace, artifacts);

        let mut running = runner
            .spawn_streaming(LocalProcessRequest::new(
                RunId::new("run-stdin"),
                "/bin/cat",
                Vec::new(),
                workspace.clone(),
                HashMap::new(),
            ))
            .expect("spawn streaming");

        running.write_stdin(b"hello-stdin\n").await.expect("write");
        running.close_stdin().await.expect("close stdin");

        let outcome = running.wait().await.expect("wait");
        assert_eq!(outcome.process.status, "exited");
        assert_eq!(
            std::fs::read_to_string(&outcome.stdout.path).unwrap(),
            "hello-stdin\n"
        );
    }

    #[tokio::test]
    async fn cap_exceeding_successful_run_is_exited_and_truncated_not_failed() {
        // The output-cap-discards-success fix: a successful run whose output
        // exceeds the cap is classified `exited` (NOT `failed`), its artifact is
        // preserved (capped), and truncation is recorded as artifact metadata.
        let workspace = temp_root("workspace-cap");
        let artifacts = temp_root("artifacts-cap");
        std::fs::create_dir_all(&workspace).unwrap();
        let mut config = LocalProcessConfig::for_test(workspace.clone(), artifacts);
        config.output_limit_bytes = 16;
        let runner = AsyncLocalProcessRunner::new(config);

        let mut running = runner
            .spawn_streaming(LocalProcessRequest::new(
                RunId::new("run-cap"),
                "/bin/sh",
                vec![
                    "-c".to_string(),
                    // Far more than the 16-byte cap, but the command SUCCEEDS.
                    "printf 'AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA'; exit 0".to_string(),
                ],
                workspace.clone(),
                HashMap::new(),
            ))
            .expect("spawn streaming");

        // Drain deltas (the full output streamed; only the artifact is capped).
        while running.next_delta().await.is_some() {}

        let outcome = running.wait().await.expect("wait");
        assert_eq!(
            outcome.process.status, "exited",
            "a successful over-cap run must be `exited`, not `failed`"
        );
        assert_eq!(outcome.exit_code, Some(0));
        assert!(outcome.truncated, "over-cap run must record truncation");
        assert!(outcome.stdout.truncated);
        // The (capped) artifact is preserved, not discarded.
        let persisted = std::fs::read(&outcome.stdout.path).unwrap();
        assert_eq!(persisted.len(), 16, "artifact must be capped to the limit");
        assert!(outcome.stdout.path.exists());
        // The truncation is surfaced as a runtime event.
        assert!(
            outcome
                .events
                .iter()
                .any(|event| event.kind == "runtime.output_truncated")
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancel_terminates_descendant_process_group() {
        // Ported process-group kill regression (in intent) onto the tokio
        // runner: a cancelled run with a backgrounded descendant leaves no
        // surviving process group, so the descendant's delayed marker never
        // appears. This is the orphan-after-cancel reaping assertion.
        let workspace = temp_root("workspace-cancel-tree");
        let artifacts = temp_root("artifacts-cancel-tree");
        std::fs::create_dir_all(&workspace).unwrap();
        let marker = workspace.join("descendant-survived.txt");
        let runner = runner(&workspace, artifacts);

        let mut running = runner
            .spawn_streaming(LocalProcessRequest::new(
                RunId::new("run-cancel-tree"),
                "/bin/sh",
                vec![
                    "-c".to_string(),
                    format!("(sleep 2; printf survived > {}) & wait", marker.display()),
                ],
                workspace.clone(),
                HashMap::new(),
            ))
            .expect("spawn streaming tree");

        // Let the tree start, then cancel mid-flight.
        tokio::time::sleep(Duration::from_millis(150)).await;
        let pid = running.external_pid().expect("pid recorded");
        let cancel_event = running.cancel("operator interrupt").await;
        assert_eq!(cancel_event.kind, "runtime.interrupt_requested");
        let outcome = running.wait().await.expect("wait after cancel");
        assert_eq!(outcome.process.status, "cancelled");

        // Give the descendant well past its delay; if the group was reaped it
        // never wrote the marker.
        tokio::time::sleep(Duration::from_millis(2200)).await;
        assert!(
            !marker.exists(),
            "cancelling the run must reap the descendant process group (pid {pid})"
        );
    }

    #[tokio::test]
    async fn non_zero_exit_is_failed() {
        let workspace = temp_root("workspace-fail");
        let artifacts = temp_root("artifacts-fail");
        std::fs::create_dir_all(&workspace).unwrap();
        let runner = runner(&workspace, artifacts);

        let mut running = runner
            .spawn_streaming(LocalProcessRequest::new(
                RunId::new("run-fail"),
                "/bin/sh",
                vec!["-c".to_string(), "printf oops >&2; exit 3".to_string()],
                workspace.clone(),
                HashMap::new(),
            ))
            .expect("spawn streaming");
        while running.next_delta().await.is_some() {}
        let outcome = running.wait().await.expect("wait");
        assert_eq!(outcome.process.status, "failed");
        assert_eq!(outcome.exit_code, Some(3));
        assert!(!outcome.truncated);
    }

    #[tokio::test]
    async fn rejects_non_allowlisted_env_overrides() {
        let workspace = temp_root("workspace-async-env");
        let artifacts = temp_root("artifacts-async-env");
        std::fs::create_dir_all(&workspace).unwrap();
        let runner = runner(&workspace, artifacts);

        let error = runner
            .spawn_streaming(LocalProcessRequest {
                run_id: RunId::new("run-async-env"),
                turn_id: None,
                program: "/usr/bin/env".to_string(),
                argv: Vec::new(),
                cwd: workspace.clone(),
                env: HashMap::from([("SECRET_TOKEN".to_string(), "secret".to_string())]),
            })
            .expect_err("disallowed env override must be rejected");
        assert!(matches!(
            error,
            RuntimeError::DisallowedEnvOverride(name) if name == "SECRET_TOKEN"
        ));
    }

    #[tokio::test]
    async fn rejects_cwd_outside_workspace() {
        let workspace = temp_root("workspace-async-allowed");
        let outside = temp_root("workspace-async-outside");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let runner = runner(&workspace, temp_root("artifacts-async-cwd"));

        let error = runner
            .spawn_streaming(LocalProcessRequest::new(
                RunId::new("run-async-reject"),
                "/bin/echo",
                vec!["nope".to_string()],
                outside,
                HashMap::new(),
            ))
            .expect_err("cwd outside workspace must be rejected");
        assert!(matches!(error, RuntimeError::CwdOutsideWorkspace { .. }));
    }
}
