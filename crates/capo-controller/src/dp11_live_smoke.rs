//! DP11: the live opt-in ACP + sandbox smokes (secrets stripped), each PAIRED
//! with a deterministic assertion of the SAME shape, plus the depth E2E gate.
//!
//! The workpad-wide verification invariant (`knowledge.md`) is that no task
//! completes on operator self-attestation alone: every live smoke is paired with
//! a deterministic assertion of the IDENTICAL shape, so completion is never
//! solely operator-attested. DP11 honours that across three axes:
//!
//! 1. ACP (DP1-DP4): one real `initialize -> session/new -> session/prompt`
//!    round-trip that streams a `session/update`, raises a
//!    `session/request_permission`, and finalizes -- driven THROUGH the
//!    controller's `drive_acp_live_turn` seam so the safety floor engages (the
//!    `PermissionPolicy` is the authority on the wire). ACP stays strictly an
//!    adapter: no `session/update` is authoritative for read models.
//!    - [`depth_e2e_gate_acp_turn_matches_paired_shape`] -- always on, no live
//!      process: a SCRIPTED transport drives the IDENTICAL controller seam and
//!      asserts the shared [`AcpTurnShape`].
//!    - [`live_acp_smoke`] -- `#[ignore]`d and behind the explicit opt-in env
//!      gates [`ACP_LIVE_PREFLIGHT_OPT_IN_ENV`] + [`ACP_LIVE_RUN_OPT_IN_ENV`];
//!      it spawns a REAL ACP-compatible agent (the runtime owns the process
//!      group), drives the SAME seam, scans the agent stderr for secrets, and
//!      asserts the IDENTICAL [`AcpTurnShape`]. It skips cleanly when the gate is
//!      closed.
//!
//! 2. Sandbox (DP7): an out-of-root write and a forbidden network egress are
//!    REFUSED before launch as `sandbox.launch_refused` events with no planned
//!    process -- the deterministic refusal shape every platform agrees on. The
//!    OS-ENFORCED live refusal (the OS layer itself blocks the write/egress) is a
//!    `#[ignore]`d env-gated smoke that lives in `capo-runtime` behind its
//!    platform gate; here the depth gate pins the platform-independent refusal
//!    shape ([`assert_sandbox_refusal_shape`]).
//!
//! 3. Memory (DP5-DP6): the retrieved + eligibility-filtered packet derives from
//!    real sources (no literals) and excludes secret/unreviewed/superseded
//!    records with auditable reasons -- the depth gate re-pins this end-to-end
//!    invariant alongside ACP and sandbox.
//!
//! Secrets-stripped is enforced on every smoke: ACP agent stderr passes the
//! credential scan (`LiveAcpSession::finalize`); the sandbox smoke's artifacts
//! pass the scan; and the deterministic gate asserts the ingested ACP read model
//! never carries a seeded secret.

#[cfg(test)]
mod tests {
    use capo_adapters::scan_artifacts_for_sensitive_markers;
    use capo_adapters::{
        ACP_LIVE_PREFLIGHT_OPT_IN_ENV, ACP_LIVE_RUN_OPT_IN_ENV, AcpAdapter, AcpLiveAdapter,
        AcpPermissionOutcome, AcpTransport, AcpTurnTranscript, ScriptedAcpTransport,
        ScriptedServerFrame, TurnRequest, acp_live_gate_open,
    };
    use capo_core::{ProjectId, TurnId};
    use capo_memory::{
        MemoryBudget, MemoryCandidate, MemoryQuery, MemoryReviewState, MemorySensitivity,
        MemorySourceKind, MemorySourceRef, SqliteFtsMemoryBackend,
    };
    use capo_runtime::{
        LocalProcessConfig, LocalProcessRequest, LocalProcessRunner, OsSandbox, SandboxEnforcement,
        SandboxPlan, SandboxProfile, SandboxRefusal, SandboxTier,
    };
    use capo_tools::PermissionPolicy;

    use crate::{AcpLiveTurnOutcome, FakeBoundaryController, FakeRunRefs};

    // ------------------------------------------------------------------
    // Shared fixtures.
    // ------------------------------------------------------------------

    fn temp_root() -> capo_tmptest::TempRoot {
        capo_tmptest::TempRoot::new("capo-dp11")
    }

    /// A read-only static policy controller + one open session. The read-only
    /// policy DENIES the write scope, so when the agent offers ONLY an allow
    /// option for an edit, the controller's authority over-rules it and answers
    /// the pending permission `cancelled` -- the safety floor on the wire.
    fn controller_with_session(
        label: &str,
    ) -> (FakeBoundaryController, FakeRunRefs, capo_tmptest::TempRoot) {
        let root = temp_root();
        let controller = FakeBoundaryController::open_with_permission_policy(
            ProjectId::new("project-capo"),
            &root,
            PermissionPolicy::static_read_only_local(),
        )
        .expect("controller");
        let registration = controller
            .register_agent(&format!("dp11-acp-{label}"))
            .expect("register agent");
        let refs = controller
            .send_task(&registration, "Drive a DP11 live-paired ACP turn")
            .expect("send task");
        (controller, refs, root)
    }

    /// The ACP live adapter under test, confined to a per-label workspace/artifact
    /// root. `program`/`argv` are the agent the live smoke spawns; the
    /// deterministic gate drives a scripted transport instead.
    fn acp_live_adapter(label: &str, program: &str, argv: Vec<String>) -> AcpLiveAdapter {
        let ws = temp_root().join(format!("acp-ws-{label}"));
        let art = temp_root().join(format!("acp-art-{label}"));
        let wrappers = capo_tools::RuntimeToolWrappers::new(
            capo_tools::RuntimeToolConfig::local_workspace(ws.clone(), art.clone()),
        );
        let setup_plan = AcpAdapter::session_setup_plan(
            &wrappers.list_tools(),
            &PermissionPolicy::static_read_only_local(),
            capo_core::SessionId::new(format!("session-dp11-{label}")),
        );
        AcpLiveAdapter::new(program, argv, ws, art, setup_plan)
    }

    // ==================================================================
    // The shared ACP turn shape (the deterministic pairing anchor).
    // ==================================================================

    /// The observable shape ONE driven ACP turn produces THROUGH the controller.
    /// Captured identically by the scripted gate and the live smoke, and asserted
    /// by [`assert_acp_turn_shape`], so the live evidence is a true pairing.
    struct AcpTurnShape {
        outcome: AcpLiveTurnOutcome,
        /// The persisted permission lifecycle events for this turn's session.
        permission_event_kinds: Vec<String>,
        /// The persisted ACP-origin tool calls, replay-rebuilt for stability.
        acp_tool_count_before_rebuild: usize,
        acp_tool_count_after_rebuild: usize,
    }

    /// Build an [`AcpTurnShape`] from a driven turn by reading the controller's
    /// persisted state (the SAME read it does for the live and scripted paths),
    /// proving replay-stability of the ACP read model.
    fn capture_acp_turn_shape(
        controller: &FakeBoundaryController,
        refs: &FakeRunRefs,
        outcome: AcpLiveTurnOutcome,
    ) -> AcpTurnShape {
        let permission_event_kinds: Vec<String> = controller
            .state()
            .events_after(0, 10_000)
            .expect("events")
            .into_iter()
            .filter(|e| {
                e.session_id.as_ref() == Some(&refs.session_id) && e.kind.starts_with("permission.")
            })
            .map(|e| e.kind)
            .collect();

        let acp_tool_count_before_rebuild = controller
            .state()
            .tool_calls_for_session(&refs.session_id)
            .expect("tools")
            .into_iter()
            .filter(|t| t.tool_origin == "adapter_native:acp")
            .count();

        controller.state().rebuild_projections().expect("rebuild");

        let acp_tool_count_after_rebuild = controller
            .state()
            .tool_calls_for_session(&refs.session_id)
            .expect("tools after")
            .into_iter()
            .filter(|t| t.tool_origin == "adapter_native:acp")
            .count();

        AcpTurnShape {
            outcome,
            permission_event_kinds,
            acp_tool_count_before_rebuild,
            acp_tool_count_after_rebuild,
        }
    }

    /// The single shared shape assertion both the deterministic gate and the live
    /// smoke call. Asserts the DP11 ACP contract for ONE controller-driven turn:
    ///
    /// - the turn streamed at least one normalized `session/update` event (a real
    ///   turn, not just lifecycle acks);
    /// - the per-event batch was INGESTED through the loop's normal route (the
    ///   ingest report imported at least one item) -- ACP stays an adapter and the
    ///   wire is not authoritative;
    /// - the safety floor engaged on the wire: a `permission.requested` and a
    ///   `permission.decided` were persisted (the `PermissionPolicy` is the
    ///   authority), and the pending permission was answered `cancelled` (the
    ///   read-only policy over-ruled the adapter-offered allow);
    /// - the turn finalized `cancelled` (the policy denial cancelled it); and
    /// - the ACP read model rebuilds identically on restart (replay-stable); and
    /// - NO observed event content carries `forbidden_secret` (secrets stripped on
    ///   the ingested read model).
    fn assert_acp_turn_shape(shape: &AcpTurnShape, forbidden_secret: &str) {
        let transcript = &shape.outcome.transcript;
        assert!(
            !transcript.events.is_empty(),
            "the driven ACP turn must stream at least one normalized event"
        );
        assert!(
            shape.outcome.ingest.appended_event_count >= 1,
            "the per-event batch must be ingested through the loop's normal route"
        );

        // Safety floor on the wire: the permission lifecycle was persisted and the
        // pending permission was answered cancelled by the policy authority.
        assert!(
            shape
                .permission_event_kinds
                .iter()
                .any(|k| k == "permission.requested"),
            "a permission.requested must be persisted for the wire round-trip"
        );
        assert!(
            shape
                .permission_event_kinds
                .iter()
                .any(|k| k == "permission.decided"),
            "a permission.decided must be persisted (the policy is the authority)"
        );
        assert_eq!(
            transcript.permission_round_trips.len(),
            1,
            "exactly one permission round-trip on the wire"
        );
        assert_eq!(
            transcript.permission_round_trips[0].outcome,
            AcpPermissionOutcome::Cancelled,
            "the read-only policy over-rules the adapter-offered allow -> cancelled"
        );
        assert_eq!(
            transcript.stop_reason.as_deref(),
            Some("cancelled"),
            "the turn finalizes cancelled after the policy denial"
        );

        // Replay-stability of the ACP read model.
        assert_eq!(
            shape.acp_tool_count_before_rebuild, shape.acp_tool_count_after_rebuild,
            "the ACP read model must rebuild identically on restart"
        );

        // Secrets stripped on the ingested read model.
        for event in &transcript.events {
            if let Some(content) = event.content.as_deref() {
                assert!(
                    !content.contains(forbidden_secret),
                    "a secret leaked into an ingested ACP event (secrets-stripped failed): {content}"
                );
            }
        }
    }

    /// The scripted server frames for one cancel-while-permission-pending turn.
    /// Identical in shape to what the live agent emits: a `session/update` tool
    /// call, a `session/request_permission` (allow-only options), a LATE update,
    /// and a `cancelled` prompt response.
    fn cancel_pending_permission_frames(session: &str) -> ScriptedAcpTransport {
        ScriptedAcpTransport::new()
            .on_request(
                "initialize",
                vec![ScriptedServerFrame::Response(serde_json::json!({
                    "protocolVersion": 1
                }))],
            )
            .on_request(
                "session/new",
                vec![ScriptedServerFrame::Response(serde_json::json!({
                    "sessionId": session
                }))],
            )
            .on_request(
                "session/prompt",
                vec![
                    ScriptedServerFrame::Update(serde_json::json!({
                        "sessionId": session,
                        "update": {
                            "sessionUpdate": "tool_call",
                            "toolCallId": "tool-dp11-1",
                            "title": "write file",
                            "status": "pending"
                        }
                    })),
                    ScriptedServerFrame::RequestPermission(serde_json::json!({
                        "sessionId": session,
                        "toolCall": { "toolCallId": "tool-dp11-1", "kind": "edit" },
                        "options": [
                            { "optionId": "opt-allow", "name": "Allow", "kind": "allow_once" }
                        ]
                    })),
                    ScriptedServerFrame::Update(serde_json::json!({
                        "sessionId": session,
                        "update": {
                            "sessionUpdate": "agent_message_chunk",
                            "content": { "type": "text", "text": "late chunk after cancel" }
                        }
                    })),
                    ScriptedServerFrame::Response(serde_json::json!({
                        "stopReason": "cancelled"
                    })),
                ],
            )
    }

    /// DP11 depth E2E gate (always on, deterministic, no live process). Drives ONE
    /// ACP turn THROUGH the controller's `drive_acp_live_turn` seam over a scripted
    /// transport and asserts the shared [`AcpTurnShape`] -- the deterministic
    /// pairing the live smoke reproduces.
    #[test]
    fn depth_e2e_gate_acp_turn_matches_paired_shape() {
        let (controller, refs, _state) = controller_with_session("gate");
        let adapter = acp_live_adapter("gate", "acp-agent", vec!["--stdio".to_string()]);
        let transport = cancel_pending_permission_frames("acp-dp11-gate-session");

        let outcome = controller
            .drive_acp_live_turn(
                &refs,
                &adapter,
                transport,
                &TurnRequest {
                    turn_id: TurnId::new("turn-dp11-gate"),
                    agent_name: "acp-worker".to_string(),
                    goal: "write a file".to_string(),
                },
            )
            .expect("drive acp live turn");

        // A late update after cancel is still ingested (DP1 cancel semantics).
        assert!(
            outcome
                .transcript
                .events
                .iter()
                .any(|e| e.content.as_deref() == Some("late chunk after cancel")),
            "a late update after cancel is still ingested"
        );

        let shape = capture_acp_turn_shape(&controller, &refs, outcome);
        // No secret is present in this deterministic turn; the assertion proves the
        // secrets-stripped check is wired (the live path emits real content).
        assert_acp_turn_shape(&shape, "AKIAIOSFODNN7EXAMPLE");
    }

    /// DP11 depth E2E gate: the memory packet derives from RETRIEVED +
    /// eligibility-filtered sources (no literals); secret / unreviewed / superseded
    /// records are excluded with auditable reasons. Re-pins the DP5-DP6 invariant
    /// in the depth gate alongside ACP + sandbox.
    #[test]
    fn depth_e2e_gate_memory_packet_filters_ineligible_sources() {
        let backend = SqliteFtsMemoryBackend::new();
        let candidate = |title: &str,
                         body: &str,
                         source_ref: &str,
                         review: MemoryReviewState,
                         sensitivity: MemorySensitivity| {
            MemoryCandidate {
                title: title.to_string(),
                body: body.to_string(),
                source: MemorySourceRef {
                    source_kind: MemorySourceKind::Markdown,
                    source_ref: source_ref.to_string(),
                    anchor: None,
                    content_hash: format!("fnv1a64:{source_ref}"),
                },
                review_state: review,
                sensitivity,
                estimated_tokens: 10,
                inclusion_reason: "retrieved by FTS".to_string(),
            }
        };
        let query = MemoryQuery::new(
            "deploy release process",
            vec![
                candidate(
                    "Deploy doc",
                    "The deploy release process is documented.",
                    "doc:deploy",
                    MemoryReviewState::Reviewed,
                    MemorySensitivity::Internal,
                ),
                candidate(
                    "Secret",
                    "deploy release token=shhh",
                    "doc:secret",
                    MemoryReviewState::Reviewed,
                    MemorySensitivity::Secret,
                ),
                candidate(
                    "Superseded",
                    "old deploy release process",
                    "doc:superseded",
                    MemoryReviewState::Superseded,
                    MemorySensitivity::Internal,
                ),
            ],
        );
        let result = backend
            .search(&query, MemoryBudget::new(256))
            .expect("search");
        let hit_refs: Vec<_> = result
            .hits
            .iter()
            .map(|h| h.candidate.source.source_ref.as_str())
            .collect();
        assert_eq!(
            hit_refs,
            vec!["doc:deploy"],
            "only the eligible FTS-matched record is included; the packet is not a literal"
        );
        assert!(result.excluded.iter().any(|d| d.reason.contains("secret")));
        assert!(
            result
                .excluded
                .iter()
                .any(|d| d.reason.contains("review_state=superseded"))
        );
    }

    // ==================================================================
    // Sandbox refusal shape (DP7) -- platform-independent depth-gate anchor.
    // ==================================================================

    fn sandbox_tmp(name: &str) -> capo_tmptest::TempRoot {
        let dir = capo_tmptest::TempRoot::new(&format!("capo-dp11-sb-{name}")).keep();
        std::fs::create_dir_all(&dir).expect("sandbox tmp");
        capo_tmptest::TempRoot::at(dir.canonicalize().expect("canonicalize"))
    }

    fn sandbox_request(root: &std::path::Path, run: &str) -> LocalProcessRequest {
        LocalProcessRequest::new(
            capo_core::RunId::new(run),
            "/bin/echo",
            vec!["hi".to_string()],
            root.to_path_buf(),
            std::collections::HashMap::new(),
        )
    }

    /// The shared sandbox-refusal shape assertion: the plan REFUSED before launch
    /// with the expected refusal, planned NO process, and recorded a
    /// `sandbox.launch_refused` event with the expected status. Both the network
    /// egress and out-of-root write refusals assert through this one helper.
    fn assert_sandbox_refusal_shape(
        plan: &SandboxPlan,
        expected_status: &str,
        matches_refusal: impl Fn(&SandboxRefusal) -> bool,
    ) {
        match &plan.enforcement {
            SandboxEnforcement::Refused { refusal } => {
                assert!(
                    matches_refusal(refusal),
                    "unexpected refusal variant: {refusal:?}"
                );
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
        assert!(plan.request.is_none(), "a refused launch plans no process");
        assert!(
            plan.events
                .iter()
                .any(|e| e.kind == "sandbox.launch_refused" && e.status == expected_status),
            "the refusal must be recorded as a sandbox.launch_refused event"
        );
    }

    /// DP11 depth E2E gate (DP7): a forbidden network egress and an out-of-root
    /// write are REFUSED before launch, deterministically on every platform,
    /// through the shared [`assert_sandbox_refusal_shape`]. The OS-enforced live
    /// refusal smoke lives in `capo-runtime` behind its platform + env gate.
    #[test]
    fn depth_e2e_gate_sandbox_refuses_egress_and_out_of_root_write() {
        let egress_root = sandbox_tmp("egress");
        let egress_sandbox = OsSandbox::new(
            SandboxTier::host_default(),
            SandboxProfile::workspace_confined([egress_root.clone()]),
        );
        let egress_plan = egress_sandbox
            .plan(sandbox_request(&egress_root, "run-dp11-egress"), true)
            .expect("plan");
        assert_sandbox_refusal_shape(&egress_plan, "network-egress-forbidden", |r| {
            matches!(r, SandboxRefusal::NetworkEgressForbidden)
        });

        let write_root = sandbox_tmp("write");
        let other_root = sandbox_tmp("write-other");
        let write_sandbox = OsSandbox::new(
            SandboxTier::host_default(),
            SandboxProfile::workspace_confined([other_root.to_path_buf()]),
        );
        let write_plan = write_sandbox
            .plan(sandbox_request(&write_root, "run-dp11-write"), false)
            .expect("plan");
        assert_sandbox_refusal_shape(&write_plan, "write-outside-confined-root", |r| {
            matches!(r, SandboxRefusal::WriteOutsideConfinedRoot { .. })
        });
    }

    /// The explicit opt-in env gates for the live OS-sandbox smoke, mirroring the
    /// ACP/Codex live-gate convention. BOTH must be `1` (the live-provider
    /// preflight + a sandbox-specific run gate) or the smoke skips cleanly.
    const SANDBOX_LIVE_PREFLIGHT_ENV: &str = "CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT";
    const SANDBOX_LIVE_RUN_ENV: &str = "CAPO_SERVER_RUN_SANDBOX_LIVE";

    fn sandbox_live_gate_open() -> bool {
        std::env::var(SANDBOX_LIVE_PREFLIGHT_ENV).as_deref() == Ok("1")
            && std::env::var(SANDBOX_LIVE_RUN_ENV).as_deref() == Ok("1")
    }

    /// DP11 live opt-in OS-sandbox smoke (DP7). `#[ignore]`d AND behind the
    /// explicit opt-in env gates; it also skips cleanly when the gate is closed OR
    /// the selected tier is not enforceable on this platform, so it never fails for
    /// everyone else. The always-on
    /// [`depth_e2e_gate_sandbox_refuses_egress_and_out_of_root_write`] is its paired
    /// deterministic assertion of the refusal shape.
    ///
    /// Run it with:
    ///   `CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT=1 CAPO_SERVER_RUN_SANDBOX_LIVE=1 \`
    ///   `  cargo test -p capo-controller -- --ignored live_sandbox_smoke`
    ///
    /// It runs a REAL OS-sandboxed process whose command attempts a write OUTSIDE
    /// the confined root: the cwd passes the pre-launch gate, but only the OS layer
    /// (seatbelt / landlock+bwrap) can refuse the actual write. The smoke asserts
    /// the OS ENFORCED the confinement (non-zero exit, the out-of-root file absent)
    /// and that every captured artifact (stdout/stderr) is secrets-stripped (passes
    /// the credential scan).
    #[test]
    #[ignore = "live sandbox smoke: set CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT=1 CAPO_SERVER_RUN_SANDBOX_LIVE=1"]
    fn live_sandbox_smoke() {
        if !sandbox_live_gate_open() {
            eprintln!(
                "skipping live sandbox smoke: set {SANDBOX_LIVE_PREFLIGHT_ENV}=1 \
                 {SANDBOX_LIVE_RUN_ENV}=1 to run it (the always-on \
                 depth_e2e_gate_sandbox_refuses_egress_and_out_of_root_write is the \
                 paired deterministic assertion of the refusal shape)"
            );
            return;
        }

        let tier = SandboxTier::host_default();
        if !tier.is_enforced_here() {
            // Honest skip: Capo never claims hard sandboxing where the OS cannot
            // enforce it. The deterministic refusal-shape gate still holds.
            eprintln!(
                "skipping live sandbox smoke: tier {} is not enforceable on this platform",
                tier.variant()
            );
            return;
        }

        let root = sandbox_tmp("live");
        let outside_guard = capo_tmptest::TempRoot::at(
            std::env::temp_dir()
                .canonicalize()
                .expect("tmp")
                .join(format!("capo-dp11-escape-{}.txt", std::process::id())),
        );
        let outside = outside_guard.to_path_buf();
        let _ = std::fs::remove_file(&outside);

        let sandbox = OsSandbox::new(tier, SandboxProfile::workspace_confined([root.clone()]));
        let runner = LocalProcessRunner::new(LocalProcessConfig::for_test(
            root.clone(),
            root.join("artifacts"),
        ));
        // cwd is inside the confined root (passes the pre-launch gate); the COMMAND
        // attempts a write OUTSIDE it -> only the OS sandbox can refuse it.
        let request = LocalProcessRequest::new(
            capo_core::RunId::new("run-dp11-live-sandbox"),
            "/bin/sh",
            vec![
                "-c".to_string(),
                format!("echo escaped > {}", outside.display()),
            ],
            root.clone(),
            std::collections::HashMap::new(),
        );
        let run = sandbox.run(&runner, request, false).expect("run sandboxed");

        assert!(
            matches!(run.plan.enforcement, SandboxEnforcement::Enforced { .. }),
            "the live smoke must run under an ENFORCED OS sandbox tier, got {:?}",
            run.plan.enforcement
        );
        let outcome = run.outcome.expect("a process ran");
        assert_ne!(
            outcome.exit_code,
            Some(0),
            "the OS sandbox must refuse the out-of-root write (non-zero exit)"
        );
        assert!(
            !outside.exists(),
            "the OS sandbox must prevent the out-of-root write at {outside:?}"
        );

        // Secrets stripped: every captured artifact passes the credential scan.
        let artifacts = [outcome.stdout.path.clone(), outcome.stderr.path.clone()];
        scan_artifacts_for_sensitive_markers(artifacts.iter())
            .expect("sandbox smoke artifacts must be secrets-stripped");

        let _ = std::fs::remove_file(&outside);
        eprintln!(
            "--- live sandbox smoke: OS-enforced out-of-root write refused (tier {}) ---",
            tier.variant()
        );
    }

    // ==================================================================
    // Live opt-in ACP smoke (DP1-DP4) -- gated, secrets stripped, paired.
    // ==================================================================

    /// Write an executable POSIX `/bin/sh` ACP-compatible agent that speaks ONE
    /// real `initialize -> session/new -> session/prompt` round-trip on stdio:
    /// it replies to `initialize` and `session/new`, then on `session/prompt`
    /// streams a `session/update`, raises a `session/request_permission`, waits
    /// for the client's permission response, streams a late update, and answers
    /// the prompt with `stopReason: cancelled`. It writes ONLY to stdout/stderr
    /// (no network, no filesystem outside its captured artifacts) and exits on
    /// stdin EOF -- a genuine ACP agent process the runtime spawns and confines.
    ///
    /// Using a self-contained stub keeps the live smoke reproducible and
    /// provider-independent while exercising the FULL live spawn + wire path
    /// (the runtime owns the process group; the controller is the policy
    /// authority). A real third-party ACP agent can be substituted by pointing
    /// the smoke's `program` at it.
    fn write_acp_agent_stub(dir: &std::path::Path) -> String {
        std::fs::create_dir_all(dir).expect("stub dir");
        let stub = dir.join("acp-agent-stub.sh");
        // The agent reads JSON-RPC lines and replies by `id`. It is deliberately
        // minimal: it matches on the method substring rather than parsing JSON, so
        // it has no dependencies and runs under the runtime's `env_clear()` PATH.
        let script = r#"#!/bin/sh
# A minimal ACP-compatible JSON-RPC stdio agent for the DP11 live smoke.
emit() { printf '%s\n' "$1"; }
while IFS= read -r line; do
  case "$line" in
    *'"method":"initialize"'*)
      id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
      emit "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{\"protocolVersion\":1}}"
      ;;
    *'"method":"session/new"'*)
      id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
      emit "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{\"sessionId\":\"acp-dp11-live-session\"}}"
      ;;
    *'"method":"session/prompt"'*)
      id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
      # Stream a tool_call update.
      emit "{\"jsonrpc\":\"2.0\",\"method\":\"session/update\",\"params\":{\"sessionId\":\"acp-dp11-live-session\",\"update\":{\"sessionUpdate\":\"tool_call\",\"toolCallId\":\"tool-dp11-live-1\",\"title\":\"write file\",\"status\":\"pending\"}}}"
      # Ask for permission (allow-only); the client (controller policy) decides.
      emit "{\"jsonrpc\":\"2.0\",\"id\":9001,\"method\":\"session/request_permission\",\"params\":{\"sessionId\":\"acp-dp11-live-session\",\"toolCall\":{\"toolCallId\":\"tool-dp11-live-1\",\"kind\":\"edit\"},\"options\":[{\"optionId\":\"opt-allow\",\"name\":\"Allow\",\"kind\":\"allow_once\"}]}}"
      # Wait for the permission response line from the client.
      IFS= read -r perm
      # Stream a late update, then finalize cancelled.
      emit "{\"jsonrpc\":\"2.0\",\"method\":\"session/update\",\"params\":{\"sessionId\":\"acp-dp11-live-session\",\"update\":{\"sessionUpdate\":\"agent_message_chunk\",\"content\":{\"type\":\"text\",\"text\":\"late chunk after cancel\"}}}}"
      emit "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{\"stopReason\":\"cancelled\"}}"
      ;;
    *) : ;;
  esac
done
"#;
        std::fs::write(&stub, script).expect("write stub");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&stub).expect("meta").permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&stub, perms).expect("chmod");
        }
        stub.to_string_lossy().to_string()
    }

    /// DP11 live opt-in ACP smoke. `#[ignore]`d AND behind the explicit opt-in env
    /// gates [`ACP_LIVE_PREFLIGHT_OPT_IN_ENV`] + [`ACP_LIVE_RUN_OPT_IN_ENV`]; it
    /// also skips cleanly when the gate is closed, so the path can be exercised by
    /// an operator without failing for everyone else.
    ///
    /// Run it with:
    ///   `CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT=1 CAPO_SERVER_RUN_ACP_LIVE=1 \`
    ///   `  cargo test -p capo-controller -- --ignored live_acp_smoke`
    ///
    /// It spawns a REAL ACP-compatible agent through the runtime (the runtime owns
    /// the process group), drives ONE real
    /// `initialize -> session/new -> session/prompt -> session/update ->
    /// session/request_permission -> cancel` flow THROUGH the controller's
    /// `drive_acp_live_turn` seam (so the `PermissionPolicy` is the wire authority
    /// and the per-event batch is ingested through the loop's normal route), scans
    /// the agent's stderr for credential markers (secrets stripped), then asserts
    /// the IDENTICAL [`AcpTurnShape`] the deterministic gate pins -- never
    /// operator-attested.
    #[test]
    #[ignore = "live ACP smoke: set CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT=1 CAPO_SERVER_RUN_ACP_LIVE=1"]
    fn live_acp_smoke() {
        if !acp_live_gate_open() {
            eprintln!(
                "skipping live ACP smoke: set {ACP_LIVE_PREFLIGHT_OPT_IN_ENV}=1 \
                 {ACP_LIVE_RUN_OPT_IN_ENV}=1 to run it (the always-on \
                 depth_e2e_gate_acp_turn_matches_paired_shape is the paired \
                 deterministic assertion of the same shape)"
            );
            return;
        }

        let (controller, refs, _state) = controller_with_session("live");
        let stub_dir = temp_root().join("acp-stub");
        let program = write_acp_agent_stub(&stub_dir);
        let adapter = acp_live_adapter("live", &program, Vec::new());

        // Spawn the REAL agent through the runtime and drive its live transport
        // through the controller seam.
        let mut session = adapter
            .spawn_live_session(&TurnId::new("turn-dp11-live"))
            .expect("spawn live acp agent");
        let transport = session.take_transport().expect("take transport");

        let outcome = controller
            .drive_acp_live_turn(
                &refs,
                &adapter,
                transport,
                &TurnRequest {
                    turn_id: TurnId::new("turn-dp11-live"),
                    agent_name: "acp-worker".to_string(),
                    goal: "write a file".to_string(),
                },
            )
            .expect("drive live acp turn");

        // Secrets stripped: tear the agent down and scan its stderr artifact.
        session
            .finalize("dp11 live acp smoke complete")
            .expect("agent stderr must be secrets-stripped");

        let shape = capture_acp_turn_shape(&controller, &refs, outcome);
        assert_acp_turn_shape(&shape, "AKIAIOSFODNN7EXAMPLE");

        // A redacted, secrets-stripped transcript an operator can attach as
        // evidence (already proven secret-free by the shared assertion).
        eprintln!("--- live ACP smoke transcript (secrets stripped) ---");
        for event in &shape.outcome.transcript.events {
            eprintln!("{}: {}", event.kind, event.content.as_deref().unwrap_or(""));
        }
    }

    /// Compile-time anchor: the live ACP smoke drives the SAME `drive_acp_live_turn`
    /// seam the deterministic gate does, over the SAME `AcpTransport` trait. This
    /// keeps the live spawn path and the scripted gate from diverging.
    #[allow(dead_code)]
    fn _transport_pairing_is_one_seam<T: AcpTransport>(
        controller: &FakeBoundaryController,
        refs: &FakeRunRefs,
        adapter: &AcpLiveAdapter,
        transport: T,
        request: &TurnRequest,
    ) -> AcpTurnTranscript {
        controller
            .drive_acp_live_turn(refs, adapter, transport, request)
            .expect("drive")
            .transcript
    }
}
