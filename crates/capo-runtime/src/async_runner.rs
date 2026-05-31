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
//! - **Redaction on emit.** ACI7 redaction is applied to *each delta* before it
//!   leaves the reader task, not only to the durable artifact at `wait()` time.
//!   `process stdout/stderr is the classic place credentials leak` (see
//!   `crate::redact_output`), so a live subscriber (broadcast/SSE) must never
//!   receive raw child bytes. Because the credential-shape scan is token-based,
//!   the reader redacts on whitespace boundaries and carries any trailing partial
//!   token (or incomplete UTF-8 sequence) over to the next chunk, so a secret
//!   split across a read boundary cannot slip through unredacted. Each delta's
//!   `status` carries the stream label (`stdout`/`stderr`); ST7 still owns the
//!   broader broadcast/SSE egress guard for non-runner frames, but the runner's
//!   own delta stream is safe by construction here.
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

/// When a stream produces no whitespace boundary, the reader cannot wait forever
/// to delimit a token without defeating incremental streaming. Once the unsent
/// buffer reaches this size it force-emits everything except a retained tail
/// (`DELTA_TOKEN_TAIL_BYTES`) long enough to cover any credential token that
/// might straddle the forced boundary.
const DELTA_FORCE_FLUSH_BYTES: usize = 4 * 1024;

/// Bytes retained at the tail of a force-flushed (whitespace-free) buffer so a
/// credential token split across the forced boundary is still scanned whole on
/// the next read. Comfortably longer than the longest recognized secret token.
const DELTA_TOKEN_TAIL_BYTES: usize = 256;

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
        let redaction_rules = self.config().redaction_rules.clone();
        let (tx, rx) = mpsc::unbounded_channel();
        let stdout_reader = stdout.map(|pipe| {
            spawn_stream_reader(
                pipe_reader_stdout(pipe),
                StreamSource::Stdout,
                cap,
                redaction_rules.clone(),
                tx.clone(),
            )
        });
        let stderr_reader = stderr.map(|pipe| {
            spawn_stream_reader(
                pipe_reader_stderr(pipe),
                StreamSource::Stderr,
                cap,
                redaction_rules.clone(),
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
/// It reads in [`DELTA_CHUNK_BYTES`] chunks, REDACTS each emitted delta (ACI7),
/// forwards it as a `runtime.output_delta` event over `tx`, and accumulates the
/// raw bytes up to `cap` for the durable artifact.
///
/// Redaction on the delta path:
///
/// - The credential-shape scan is token-based (whitespace-delimited), so a secret
///   that straddles a read boundary would be missed if we redacted fixed 8KiB
///   slices. The reader therefore only emits up to the last whitespace boundary
///   in the accumulated unsent text and carries the trailing partial token (and
///   any incomplete UTF-8 sequence) over to the next read, redacting the emitted
///   prefix with the same [`RedactionPolicy`] the artifact uses.
/// - The delta stream is forwarded REGARDLESS of the cap so a live subscriber
///   sees full progress; the cap only bounds what is persisted to the durable
///   artifact (`truncated` is recorded as artifact metadata, the stream is not
///   capped).
fn spawn_stream_reader(
    mut pipe: BoxedPipe,
    source: StreamSource,
    cap: usize,
    redaction_rules: Vec<crate::RedactionRule>,
    tx: UnboundedSender<RuntimeEvent>,
) -> JoinHandle<StreamCapture> {
    tokio::spawn(async move {
        let policy = RedactionPolicy::new(redaction_rules);
        let mut collected: Vec<u8> = Vec::new();
        let mut truncated = false;
        // Unsent raw bytes whose trailing partial token / incomplete UTF-8 byte
        // sequence has not yet been delimited, so the credential scan never sees
        // a secret cut in half.
        let mut pending: Vec<u8> = Vec::new();
        let mut buf = [0u8; DELTA_CHUNK_BYTES];
        loop {
            match pipe.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    let chunk = &buf[..n];
                    // Accumulate the raw bytes for the artifact (capped); the
                    // artifact is independently re-redacted at persist time.
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
                    // Emit a redacted delta for everything up to the last
                    // whitespace boundary, holding the trailing partial token.
                    pending.extend_from_slice(chunk);
                    if let Some(emit) = take_emittable_prefix(&mut pending) {
                        send_redacted_delta(&tx, source, &policy, &emit);
                    }
                }
                Err(_) => break,
            }
        }
        // Flush whatever partial token remains at EOF.
        if !pending.is_empty() {
            send_redacted_delta(&tx, source, &policy, &pending);
        }
        StreamCapture {
            bytes: collected,
            truncated,
        }
    })
}

/// Split `pending` into the prefix that is safe to redact-and-emit and the
/// trailing bytes that must be carried to the next read so the token-based
/// credential scan never sees a secret cut in half.
///
/// Normally the split is the last whitespace boundary (everything up to and
/// including it forms complete tokens). When the buffer has grown past
/// [`DELTA_FORCE_FLUSH_BYTES`] with no whitespace at all -- e.g. a long
/// no-newline stream -- it force-emits everything except a retained tail of
/// [`DELTA_TOKEN_TAIL_BYTES`] (on a UTF-8 char boundary) so streaming stays
/// incremental while a token straddling the forced cut is still scanned whole
/// next time. Returns `None` while the whole buffer is one short unfinished
/// token, so the reader keeps buffering.
fn take_emittable_prefix(pending: &mut Vec<u8>) -> Option<Vec<u8>> {
    // ASCII whitespace is never a UTF-8 continuation byte, so a whitespace
    // boundary is always a valid char-boundary split point.
    if let Some(last_ws) = pending.iter().rposition(|byte| byte.is_ascii_whitespace()) {
        let remainder = pending.split_off(last_ws + 1);
        return Some(std::mem::replace(pending, remainder));
    }
    if pending.len() > DELTA_FORCE_FLUSH_BYTES {
        let mut split = pending.len() - DELTA_TOKEN_TAIL_BYTES;
        // Back up to a UTF-8 char boundary so neither half splits a code point.
        while split > 0 && (pending[split] & 0b1100_0000) == 0b1000_0000 {
            split -= 1;
        }
        let remainder = pending.split_off(split);
        return Some(std::mem::replace(pending, remainder));
    }
    None
}

/// Redact `bytes` with `policy` and forward them as a `runtime.output_delta`
/// event. `status` carries the stream label so consumers can route stdout vs
/// stderr; the payload is already ACI7-redacted.
fn send_redacted_delta(
    tx: &UnboundedSender<RuntimeEvent>,
    source: StreamSource,
    policy: &RedactionPolicy,
    bytes: &[u8],
) {
    let (redacted, _state) = policy.apply(bytes);
    let _ = tx.send(RuntimeEvent {
        kind: "runtime.output_delta".to_string(),
        status: source.as_str().to_string(),
        detail: String::from_utf8_lossy(&redacted).to_string(),
    });
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
    ///
    /// The event's `detail` is ALREADY ACI7-redacted (see the reader task in
    /// [`spawn_stream_reader`]); `status` is the stream label (`stdout`/`stderr`).
    /// The delta is forwarded in full regardless of the output cap (the cap only
    /// bounds the persisted artifact), so a live subscriber sees complete progress.
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
        // Prove incrementality OBSERVABLY (not by buffering-then-emitting once at
        // exit): the child emits a first line, blocks reading stdin, then emits a
        // second line and exits. We must receive the first delta WHILE the child
        // is still running (blocked on stdin, second line not yet produced), then
        // unblock it and receive the rest. A buffer-then-emit-after-exit
        // implementation would deadlock here (the child never exits until we
        // write stdin, and we never write stdin until we see the first delta).
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
                    // Emit `first`, flush, block on a stdin read, then emit the
                    // rest. `printf` to stderr proves both pipes stream.
                    "printf 'first\\n'; printf 'err\\n' >&2; read _line; printf 'second\\n'"
                        .to_string(),
                ],
                workspace.clone(),
                HashMap::new(),
            ))
            .expect("spawn streaming");

        // The first stdout delta must arrive BEFORE we write stdin, i.e. while the
        // child is still running and the second line does not yet exist.
        let mut stdout_seen = String::new();
        let mut stderr_seen = String::new();
        loop {
            let delta = tokio::time::timeout(Duration::from_secs(5), running.next_delta())
                .await
                .expect("first delta must arrive before exit (incrementality)")
                .expect("a delta before EOF");
            assert_eq!(delta.kind, "runtime.output_delta");
            match delta.status.as_str() {
                "stdout" => stdout_seen.push_str(&delta.detail),
                "stderr" => stderr_seen.push_str(&delta.detail),
                other => panic!("unexpected delta stream {other}"),
            }
            if stdout_seen.contains("first") {
                break;
            }
        }
        // We observed `first` while the child is still blocked on stdin -- the
        // second line has not been produced yet, proving the stream is live.
        assert!(
            !stdout_seen.contains("second"),
            "received `second` before unblocking the child: not incremental, got {stdout_seen:?}"
        );
        // Positive liveness check: "before exit" is a CHECKED fact, not an
        // inference. The child is parked on `read _line`, so its process group
        // must still have a live member at the instant the first delta arrived.
        // A buffer-then-emit-after-exit runner could only deliver `first` once
        // the child had already exited, so this would read `false` (and, because
        // the child never exits without stdin we never write until we see the
        // delta, the prior `next_delta()` would have timed out first).
        #[cfg(unix)]
        {
            let pid = running.external_pid().expect("pid recorded");
            assert!(
                crate::process_group_is_alive(pid),
                "the child must still be running when the first delta is observed (pid {pid}); \
                 a delta arriving only after exit is not incremental"
            );
        }

        // Unblock the child so it emits the rest and exits.
        running.write_stdin(b"go\n").await.expect("write stdin");
        running.close_stdin().await.expect("close stdin");

        while let Some(delta) = running.next_delta().await {
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
        assert_eq!(stdout_seen, "first\nsecond\n");
        assert_eq!(stderr_seen, "err\n");
        // And the durable artifact holds the same bytes.
        assert_eq!(
            std::fs::read_to_string(&outcome.stdout.path).unwrap(),
            "first\nsecond\n"
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
        let config_cap = 16;
        let mut config = LocalProcessConfig::for_test(workspace.clone(), artifacts);
        config.output_limit_bytes = config_cap;
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

        // Drain deltas, ACCUMULATING their content: the full pre-cap output must
        // stream even though the artifact is capped. (40 identical 'A's are not
        // credential-shaped -- uppercase only, no case/digit mix -- so the scan
        // leaves them intact and the delta text equals the raw output.)
        let mut streamed = String::new();
        while let Some(delta) = running.next_delta().await {
            assert_eq!(delta.status, "stdout");
            streamed.push_str(&delta.detail);
        }
        assert_eq!(
            streamed,
            "A".repeat(40),
            "the live delta stream must carry the FULL output, uncapped"
        );
        // The cap-the-stream failure mode pinned directly: the streamed bytes
        // must exceed the cap. A runner that capped the egress stream (not just
        // the persisted artifact) would deliver at most `cap` bytes here.
        assert!(
            streamed.len() > config_cap,
            "the delta stream ({} bytes) must exceed the {config_cap}-byte cap; \
             capping the stream (not just the artifact) violates the invariant",
            streamed.len()
        );

        let outcome = running.wait().await.expect("wait");
        assert_eq!(
            outcome.process.status, "exited",
            "a successful over-cap run must be `exited`, not `failed`"
        );
        assert_eq!(outcome.exit_code, Some(0));
        assert!(outcome.truncated, "over-cap run must record truncation");
        assert!(outcome.stdout.truncated);
        // The (capped) artifact is preserved, not discarded -- both halves of the
        // invariant pinned: deltas un-capped (40 bytes), artifact capped (16).
        let persisted = std::fs::read(&outcome.stdout.path).unwrap();
        assert_eq!(
            persisted.len(),
            config_cap,
            "artifact must be capped to the limit"
        );
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

    #[tokio::test]
    async fn streamed_deltas_and_artifact_redact_an_unnamed_secret() {
        // ACI7 regression on the STREAMING path (ported from the sync runner's
        // `local_process_runner_credential_scan_redacts_unnamed_secret_in_output`):
        // a child that prints an unnamed secret must have it scrubbed from BOTH
        // the live `runtime.output_delta` stream AND the durable artifact, with
        // `redaction_state = "redacted"`. No operator rule is configured, so this
        // exercises the default credential-shape scan on emit.
        let workspace = temp_root("workspace-async-credscan");
        let artifacts = temp_root("artifacts-async-credscan");
        std::fs::create_dir_all(&workspace).unwrap();
        let runner = runner(&workspace, artifacts);

        let mut running = runner
            .spawn_streaming(LocalProcessRequest::new(
                RunId::new("run-async-credscan"),
                "/bin/sh",
                vec![
                    "-c".to_string(),
                    "printf 'key AKIAIOSFODNN7EXAMPLE\\n'; \
                     printf 'tok ghp_abcdEFGH1234ijklMNOP5678qrst\\n' >&2"
                        .to_string(),
                ],
                workspace.clone(),
                HashMap::new(),
            ))
            .expect("spawn streaming");

        let mut stdout_seen = String::new();
        let mut stderr_seen = String::new();
        while let Some(delta) = running.next_delta().await {
            match delta.status.as_str() {
                "stdout" => stdout_seen.push_str(&delta.detail),
                "stderr" => stderr_seen.push_str(&delta.detail),
                other => panic!("unexpected delta stream {other}"),
            }
        }

        // The LIVE delta stream is redacted (this is the gap the fix closes: the
        // old code streamed raw bytes and only redacted the at-exit artifact).
        assert!(
            !stdout_seen.contains("AKIAIOSFODNN7EXAMPLE"),
            "secret leaked to the live stdout delta stream: {stdout_seen:?}"
        );
        assert!(
            !stderr_seen.contains("ghp_abcdEFGH1234ijklMNOP5678qrst"),
            "secret leaked to the live stderr delta stream: {stderr_seen:?}"
        );
        assert!(
            stdout_seen.contains(crate::CREDENTIAL_REDACTION_PLACEHOLDER),
            "stdout delta should carry the redaction placeholder: {stdout_seen:?}"
        );
        // Benign surrounding words survive on the live stream.
        assert!(stdout_seen.contains("key"));

        let outcome = running.wait().await.expect("wait");
        assert_eq!(outcome.process.status, "exited");
        // The durable artifact is also redacted, and redaction_state reflects it.
        assert_eq!(outcome.stdout.redaction_state, "redacted");
        assert_eq!(outcome.stderr.redaction_state, "redacted");
        assert_eq!(outcome.process.redaction_state, "redacted");
        let stdout = std::fs::read_to_string(&outcome.stdout.path).unwrap();
        let stderr = std::fs::read_to_string(&outcome.stderr.path).unwrap();
        assert!(
            !stdout.contains("AKIAIOSFODNN7EXAMPLE"),
            "secret leaked to the stdout artifact: {stdout}"
        );
        assert!(
            !stderr.contains("ghp_abcdEFGH1234ijklMNOP5678qrst"),
            "secret leaked to the stderr artifact: {stderr}"
        );
    }

    #[tokio::test]
    async fn secret_split_across_a_chunk_boundary_is_still_redacted() {
        // The credential scan is token-based, so a secret printed without a
        // trailing whitespace boundary (and potentially split across pipe reads)
        // must still be redacted: the reader holds the trailing partial token
        // until a boundary/EOF rather than emitting a half-token raw.
        let workspace = temp_root("workspace-async-split");
        let artifacts = temp_root("artifacts-async-split");
        std::fs::create_dir_all(&workspace).unwrap();
        let runner = runner(&workspace, artifacts);

        let mut running = runner
            .spawn_streaming(LocalProcessRequest::new(
                RunId::new("run-async-split"),
                "/bin/sh",
                vec![
                    "-c".to_string(),
                    // No trailing newline: the secret is the final, unterminated
                    // token and is only emitted on the EOF flush.
                    "printf 'AKIAIOSFODNN7EXAMPLE'".to_string(),
                ],
                workspace.clone(),
                HashMap::new(),
            ))
            .expect("spawn streaming");

        let mut stdout_seen = String::new();
        while let Some(delta) = running.next_delta().await {
            if delta.status == "stdout" {
                stdout_seen.push_str(&delta.detail);
            }
        }
        assert!(
            !stdout_seen.contains("AKIAIOSFODNN7EXAMPLE"),
            "an unterminated secret token leaked to the live stream: {stdout_seen:?}"
        );
        assert!(stdout_seen.contains(crate::CREDENTIAL_REDACTION_PLACEHOLDER));

        let outcome = running.wait().await.expect("wait");
        assert_eq!(outcome.stdout.redaction_state, "redacted");
    }
}
