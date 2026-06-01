//! DP10: the consolidated deterministic fake/replay suite that must pass with NO
//! live provider and NO real OS sandbox network.
//!
//! DP1-DP9 each shipped their own focused deterministic tests in their owning
//! crate (`capo-adapters` for the ACP wire/replay engine, `capo-memory` for FTS5
//! retrieval, `capo-runtime` for the OS sandbox, `capo-state` for the
//! projections). DP10 does not re-implement those; it CONSOLIDATES the
//! cross-cutting invariants end-to-end through the REAL controller orchestration
//! seam -- the same `FakeBoundaryController` a server dispatches to -- and proves
//! each one is replay-stable (a clear-and-rebuild from the event log reconstructs
//! identical projected state).
//!
//! Concretely, this module asserts, in one place:
//!
//! - ACP replay/dedupe (DP1-DP3), driven through `ingest_acp_replay_plan` /
//!   `drive_acp_live_turn`:
//!   - `session/resume` adds no items,
//!   - `session/load` imports each item once,
//!   - repeated identical `tool_call_update`s yield ONE read-model row (one
//!     timeline key) while every raw frame is retained,
//!   - ID-less consecutive same-type message chunks record `low` boundary
//!     confidence,
//!   - cancel-while-permission-pending finalizes the turn `cancelled` (the
//!     pending permission is answered `cancelled` and the late update is still
//!     ingested),
//!   - and every ACP read model rebuilds identically on restart.
//! - Memory retrieval (DP5-DP6), driven through the REAL `MemoryBackend`:
//!   - the packet derives from RETRIEVED + eligibility-filtered sources (no
//!     hardcoded packet literals),
//!   - secret / unreviewed / superseded records are excluded with auditable
//!     reasons,
//!   - and the attached packet replays byte-for-byte from its artifact id across
//!     an index rebuild.
//! - Sandbox refusal modes (DP7), driven through the REAL `OsSandbox` plan:
//!   - an out-of-root write and a forbidden network egress are REFUSED (recorded
//!     as `sandbox.launch_refused` events with no planned process), deterministic
//!     on every platform; the OS-enforced refusal tests stay in `capo-runtime`
//!     behind their platform gate.
//!
//! There is no live provider, no real ACP process, and no real sandbox network
//! anywhere in this module (the DP10 Must: deterministic-only).

#[cfg(test)]
mod tests {
    use capo_adapters::{
        AcpAdapter, AcpLiveAdapter, AcpPermissionOutcome, AcpReconcileDecision, AcpReplayEngine,
        AcpReplaySource, ScriptedAcpTransport, ScriptedServerFrame, TurnRequest,
    };
    use capo_core::{MemoryPacketId, ProjectId, SessionId, TurnId};
    use capo_memory::{
        LiveMemoryPacketRequest, MarkdownSource, MemoryBackend, MemoryBudget, MemoryCandidate,
        MemoryQuery, MemoryReviewState, MemorySensitivity, MemorySourceKind, MemorySourceRef,
        SqliteFtsMemoryBackend,
    };
    use capo_runtime::{
        LocalProcessRequest, OsSandbox, SandboxEnforcement, SandboxProfile, SandboxRefusal,
        SandboxTier,
    };

    use crate::{FakeBoundaryController, FakeRunRefs};
    use capo_tools::PermissionPolicy;

    fn temp_root() -> std::path::PathBuf {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "capo-dp10-{nanos}-{:?}",
            std::thread::current().id()
        ))
    }

    // ------------------------------------------------------------------
    // Shared fixtures.
    // ------------------------------------------------------------------

    fn controller_with_session(label: &str) -> (FakeBoundaryController, FakeRunRefs) {
        let root = temp_root();
        let controller = FakeBoundaryController::open(ProjectId::new("project-capo"), &root)
            .expect("controller");
        let registration = controller
            .register_agent(&format!("dp10-acp-{label}"))
            .expect("register agent");
        let refs = controller
            .send_task(&registration, "Drive a consolidated DP10 ACP path")
            .expect("send task");
        (controller, refs)
    }

    fn acp_update(session: &str, body: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": { "sessionId": session, "update": body }
        })
    }

    // ==================================================================
    // ACP (DP1-DP3) consolidated invariants.
    // ==================================================================

    /// DP10 / DP2: a `session/resume` attach imports NO items, records an attach
    /// batch (not a replay batch), and the read model rebuilds identically.
    #[test]
    fn dp10_acp_resume_adds_no_items_and_rebuilds_identically() {
        let (controller, refs) = controller_with_session("resume");

        let plan = AcpReplayEngine::plan_resume_attach(
            "acp-ext-dp10-resume",
            &serde_json::json!({ "resumed": true }),
        );
        let report = controller
            .ingest_acp_replay_plan(&refs, &plan)
            .expect("ingest resume attach");
        assert_eq!(report.imported_count, 0, "resume imports no items");
        assert_eq!(report.duplicate_count, 0);
        assert_eq!(report.ambiguous_count, 0);

        let batches = controller
            .state()
            .adapter_replay_batches_for_session(&refs.session_id)
            .expect("batches");
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].source, "session_resume_attach");

        // Restart/replay: the attach batch rebuilds identically.
        let before = batches;
        controller.state().rebuild_projections().expect("rebuild");
        assert_eq!(
            controller
                .state()
                .adapter_replay_batches_for_session(&refs.session_id)
                .expect("batches after"),
            before,
            "the resume attach batch rebuilds identically after restart",
        );
    }

    /// DP10 / DP2: a foreign `session/load` imports each item exactly once, and
    /// every ACP read model rebuilds identically after a clear-and-replay.
    #[test]
    fn dp10_acp_foreign_load_imports_once_and_rebuilds_identically() {
        let (controller, refs) = controller_with_session("foreign-load");

        let frames = vec![
            acp_update(
                "acp-ext-dp10-foreign",
                serde_json::json!({
                    "sessionUpdate": "user_message_chunk",
                    "content": { "type": "text", "text": "do the task" }
                }),
            ),
            acp_update(
                "acp-ext-dp10-foreign",
                serde_json::json!({
                    "sessionUpdate": "tool_call",
                    "toolCallId": "tool-dp10-foreign-1",
                    "title": "write file",
                    "status": "completed",
                    "content": { "type": "text", "text": "done" }
                }),
            ),
        ];
        let existing = controller
            .acp_existing_item_fingerprints(&refs)
            .expect("fingerprints");
        let plan = AcpReplayEngine::plan_load(
            AcpReplaySource::ForeignImport,
            "acp-ext-dp10-foreign",
            &frames,
            &existing,
        );
        let report = controller
            .ingest_acp_replay_plan(&refs, &plan)
            .expect("ingest foreign load");
        assert_eq!(
            report.imported_count, 2,
            "import user chunk + tool call once"
        );
        assert_eq!(report.duplicate_count, 0);
        assert_eq!(report.raw_update_count, 2, "every raw frame persisted");

        let acp_tools: usize = controller
            .state()
            .tool_calls_for_session(&refs.session_id)
            .expect("tools")
            .into_iter()
            .filter(|t| t.tool_origin == "adapter_native:acp" && t.status == "completed")
            .count();
        assert_eq!(acp_tools, 1, "the foreign tool call imports exactly once");

        // Restart/replay across ALL three DP2 read models + imported tool calls.
        let batches = controller
            .state()
            .adapter_replay_batches_for_session(&refs.session_id)
            .expect("batches");
        let raw = controller
            .state()
            .adapter_raw_updates_for_batch(&report.acp_replay_batch_id)
            .expect("raw");
        let keys = controller
            .state()
            .adapter_timeline_keys_for_session(&refs.session_id)
            .expect("keys");
        let tools = controller
            .state()
            .tool_calls_for_session(&refs.session_id)
            .expect("tools");

        controller.state().rebuild_projections().expect("rebuild");

        assert_eq!(
            controller
                .state()
                .adapter_replay_batches_for_session(&refs.session_id)
                .expect("batches after"),
            batches,
        );
        assert_eq!(
            controller
                .state()
                .adapter_raw_updates_for_batch(&report.acp_replay_batch_id)
                .expect("raw after"),
            raw,
        );
        assert_eq!(
            controller
                .state()
                .adapter_timeline_keys_for_session(&refs.session_id)
                .expect("keys after"),
            keys,
        );
        assert_eq!(
            controller
                .state()
                .tool_calls_for_session(&refs.session_id)
                .expect("tools after"),
            tools,
            "imported tool calls rebuild identically",
        );
    }

    /// DP10 / DP2-DP3: repeated identical `tool_call_update`s collapse to ONE
    /// read-model candidate (one timeline key) while every raw frame is retained,
    /// and the reduction is replay-stable.
    #[test]
    fn dp10_acp_repeated_tool_updates_yield_one_read_model() {
        let (controller, refs) = controller_with_session("repeat");

        let frames = vec![
            acp_update(
                "acp-ext-dp10-repeat",
                serde_json::json!({
                    "sessionUpdate": "tool_call",
                    "toolCallId": "tool-dp10-repeat-1",
                    "title": "write file",
                    "status": "pending"
                }),
            ),
            acp_update(
                "acp-ext-dp10-repeat",
                serde_json::json!({
                    "sessionUpdate": "tool_call_update",
                    "toolCallId": "tool-dp10-repeat-1",
                    "status": "completed",
                    "content": { "type": "text", "text": "done" }
                }),
            ),
            acp_update(
                "acp-ext-dp10-repeat",
                serde_json::json!({
                    "sessionUpdate": "tool_call_update",
                    "toolCallId": "tool-dp10-repeat-1",
                    "status": "completed",
                    "content": { "type": "text", "text": "done" }
                }),
            ),
        ];
        let plan = AcpReplayEngine::plan_load(
            AcpReplaySource::SessionLoad,
            "acp-ext-dp10-repeat",
            &frames,
            &controller
                .acp_existing_item_fingerprints(&refs)
                .expect("fingerprints"),
        );

        // The engine collapses the three frames for one toolCallId into a single
        // imported candidate while retaining all three raw frames.
        let imported = plan
            .candidates
            .iter()
            .filter(|c| matches!(c.decision, AcpReconcileDecision::Imported))
            .count();
        assert_eq!(imported, 1, "one tool call -> one imported candidate");
        assert_eq!(plan.raw_updates.len(), 3, "every raw frame retained");

        let report = controller
            .ingest_acp_replay_plan(&refs, &plan)
            .expect("ingest");
        assert_eq!(report.raw_update_count, 3, "all raw frames persisted");

        let acp_tools: usize = controller
            .state()
            .tool_calls_for_session(&refs.session_id)
            .expect("tools")
            .into_iter()
            .filter(|t| t.tool_origin == "adapter_native:acp")
            .count();
        assert_eq!(acp_tools, 1, "repeated updates yield one read-model row");

        // One stable timeline key for the tool call, replay-stable.
        let keys = controller
            .state()
            .adapter_timeline_keys_for_session(&refs.session_id)
            .expect("keys");
        let tool_keys: usize = keys
            .iter()
            .filter(|k| k.adapter_timeline_key_id.contains(":tool:"))
            .count();
        assert_eq!(tool_keys, 1, "one stable timeline key for the tool call");

        controller.state().rebuild_projections().expect("rebuild");
        assert_eq!(
            controller
                .state()
                .adapter_timeline_keys_for_session(&refs.session_id)
                .expect("keys after"),
            keys,
            "the collapsed tool timeline key rebuilds identically",
        );
    }

    /// DP10 / DP2-DP3: ID-less consecutive same-type message chunks record `low`
    /// boundary confidence (the boundary is genuinely ambiguous) and import as an
    /// ambiguous, auditable reconciliation -- never silently as stable.
    #[test]
    fn dp10_acp_idless_chunks_record_low_boundary_confidence() {
        let (controller, refs) = controller_with_session("idless");

        // Two consecutive same-type chunks (same role + content hash) collapse into
        // one message group; because more than one chunk collapses, the boundary is
        // genuinely ambiguous and confidence drops to `low`.
        let frames = vec![
            acp_update(
                "acp-ext-dp10-idless",
                serde_json::json!({
                    "sessionUpdate": "agent_message_chunk",
                    "content": { "type": "text", "text": "hello" }
                }),
            ),
            acp_update(
                "acp-ext-dp10-idless",
                serde_json::json!({
                    "sessionUpdate": "agent_message_chunk",
                    "content": { "type": "text", "text": "hello" }
                }),
            ),
        ];
        let plan = AcpReplayEngine::plan_load(
            AcpReplaySource::SessionLoad,
            "acp-ext-dp10-idless",
            &frames,
            &controller
                .acp_existing_item_fingerprints(&refs)
                .expect("fingerprints"),
        );

        // One ID-less message group with a synthetic ref and `low` boundary
        // confidence (no stable ACP v1 id keyed it).
        let low_key = plan
            .timeline_keys
            .iter()
            .find(|k| k.confidence.timeline_str() == "low")
            .expect("an ID-less collapsed message group records low boundary confidence");
        assert!(
            low_key.stable_ref.is_none() && low_key.synthetic_ref.is_some(),
            "the low-confidence key is synthetic (role + content-hash), not a stable id",
        );

        let report = controller
            .ingest_acp_replay_plan(&refs, &plan)
            .expect("ingest");
        // The ambiguous import is recorded (auditable), not silently projected as a
        // stable item.
        assert!(
            report.ambiguous_count >= 1,
            "the ID-less group imports as an auditable ambiguous reconciliation",
        );

        let keys = controller
            .state()
            .adapter_timeline_keys_for_session(&refs.session_id)
            .expect("keys");
        assert!(
            keys.iter().any(|k| k.confidence == "low"),
            "the low boundary confidence is persisted",
        );

        controller.state().rebuild_projections().expect("rebuild");
        assert_eq!(
            controller
                .state()
                .adapter_timeline_keys_for_session(&refs.session_id)
                .expect("keys after"),
            keys,
            "the low-confidence timeline key rebuilds identically",
        );
    }

    /// DP10 / DP1-DP3: cancel-while-permission-pending, driven END-TO-END through
    /// the controller's `drive_acp_live_turn` seam. The agent raises a permission
    /// request mid-turn; an operator-cancel policy answers it `cancelled`; the
    /// agent still streams a LATE update which is ingested; and the turn finalizes
    /// with stop reason `cancelled`. The permission lifecycle is persisted and the
    /// ingested events rebuild identically.
    #[test]
    fn dp10_acp_cancel_while_permission_pending_finalizes_cancelled_end_to_end() {
        let root = temp_root();
        // A read-only static policy DENIES the write scope, and the agent offers
        // ONLY an allow option -- so the controller's authority over-rules the
        // adapter-offered allow and answers the pending permission `cancelled`
        // (never an invented allow) while the turn is being cancelled.
        let controller = FakeBoundaryController::open_with_permission_policy(
            ProjectId::new("project-capo"),
            &root,
            PermissionPolicy::static_read_only_local(),
        )
        .expect("controller");
        let registration = controller
            .register_agent("dp10-acp-cancel")
            .expect("register agent");
        let refs = controller
            .send_task(&registration, "Cancel a turn while a permission is pending")
            .expect("send task");

        let wrappers =
            capo_tools::RuntimeToolWrappers::new(capo_tools::RuntimeToolConfig::local_workspace(
                std::path::PathBuf::from("/tmp/capo-dp10-cancel-ws"),
                std::path::PathBuf::from("/tmp/capo-dp10-cancel-art"),
            ));
        let setup_plan = AcpAdapter::session_setup_plan(
            &wrappers.list_tools(),
            &PermissionPolicy::static_read_only_local(),
            refs.session_id.clone(),
        );
        let adapter = AcpLiveAdapter::new(
            "acp-agent",
            vec!["--stdio".to_string()],
            std::path::PathBuf::from("/tmp/capo-dp10-cancel-ws"),
            std::path::PathBuf::from("/tmp/capo-dp10-cancel-art"),
            setup_plan,
        );

        let transport = ScriptedAcpTransport::new()
            .on_request(
                "initialize",
                vec![ScriptedServerFrame::Response(serde_json::json!({
                    "protocolVersion": 1
                }))],
            )
            .on_request(
                "session/new",
                vec![ScriptedServerFrame::Response(serde_json::json!({
                    "sessionId": "acp-dp10-cancel-session"
                }))],
            )
            .on_request(
                "session/prompt",
                vec![
                    ScriptedServerFrame::Update(serde_json::json!({
                        "sessionId": "acp-dp10-cancel-session",
                        "update": {
                            "sessionUpdate": "tool_call",
                            "toolCallId": "tool-dp10-cancel-1",
                            "title": "write file",
                            "status": "pending"
                        }
                    })),
                    // The agent asks for permission; the operator-cancel policy
                    // answers it `cancelled`.
                    ScriptedServerFrame::RequestPermission(serde_json::json!({
                        "sessionId": "acp-dp10-cancel-session",
                        "toolCall": { "toolCallId": "tool-dp10-cancel-1", "kind": "edit" },
                        "options": [
                            { "optionId": "opt-allow", "name": "Allow", "kind": "allow_once" }
                        ]
                    })),
                    // A LATE update streamed after the cancel is still ingested.
                    ScriptedServerFrame::Update(serde_json::json!({
                        "sessionId": "acp-dp10-cancel-session",
                        "update": {
                            "sessionUpdate": "agent_message_chunk",
                            "content": { "type": "text", "text": "late chunk after cancel" }
                        }
                    })),
                    ScriptedServerFrame::Response(serde_json::json!({
                        "stopReason": "cancelled"
                    })),
                ],
            );

        let outcome = controller
            .drive_acp_live_turn(
                &refs,
                &adapter,
                transport,
                &TurnRequest {
                    turn_id: TurnId::new("turn-dp10-cancel"),
                    agent_name: "acp-worker".to_string(),
                    goal: "write a file".to_string(),
                },
            )
            .expect("drive acp live turn");

        // The pending permission was answered `cancelled` on the wire.
        assert_eq!(outcome.transcript.permission_round_trips.len(), 1);
        assert_eq!(
            outcome.transcript.permission_round_trips[0].outcome,
            AcpPermissionOutcome::Cancelled,
            "the pending permission is answered cancelled",
        );
        // The turn finalized cancelled.
        assert_eq!(
            outcome.transcript.stop_reason.as_deref(),
            Some("cancelled"),
            "the turn finalizes with stop reason cancelled",
        );
        // The LATE update after cancel was still ingested.
        assert!(
            outcome
                .transcript
                .events
                .iter()
                .any(|e| e.content.as_deref() == Some("late chunk after cancel")),
            "a late update after cancel is still ingested",
        );

        // The permission lifecycle was PERSISTED through the controller seam.
        let perm_events: Vec<_> = controller
            .state()
            .events_after(0, 10_000)
            .expect("events")
            .into_iter()
            .filter(|e| {
                e.session_id.as_ref() == Some(&refs.session_id)
                    && e.kind.starts_with("permission.")
                    && e.payload_json.contains("acp-live-perm-turn-dp10-cancel")
            })
            .collect();
        assert!(
            perm_events.iter().any(|e| e.kind == "permission.requested"),
            "the pending permission request persisted",
        );
        assert!(
            perm_events.iter().any(|e| e.kind == "permission.decided"),
            "the cancelled decision persisted",
        );

        // The ingested events rebuild identically on restart.
        let tools = controller
            .state()
            .tool_calls_for_session(&refs.session_id)
            .expect("tools");
        controller.state().rebuild_projections().expect("rebuild");
        assert_eq!(
            controller
                .state()
                .tool_calls_for_session(&refs.session_id)
                .expect("tools after"),
            tools,
            "the cancelled turn's read models rebuild identically",
        );
    }

    // ==================================================================
    // Memory retrieval (DP5-DP6) consolidated invariants.
    // ==================================================================

    fn candidate(
        title: &str,
        body: &str,
        source_ref: &str,
        review_state: MemoryReviewState,
        sensitivity: MemorySensitivity,
        tokens: usize,
    ) -> MemoryCandidate {
        MemoryCandidate {
            title: title.to_string(),
            body: body.to_string(),
            source: MemorySourceRef {
                source_kind: MemorySourceKind::Markdown,
                source_ref: source_ref.to_string(),
                anchor: None,
                content_hash: format!("fnv1a64:{source_ref}"),
            },
            review_state,
            sensitivity,
            estimated_tokens: tokens,
            inclusion_reason: "retrieved by FTS".to_string(),
        }
    }

    fn reviewed(title: &str, body: &str, source_ref: &str, tokens: usize) -> MemoryCandidate {
        candidate(
            title,
            body,
            source_ref,
            MemoryReviewState::Reviewed,
            MemorySensitivity::Internal,
            tokens,
        )
    }

    /// DP10 / DP5: the packet derives from RETRIEVED + eligibility-filtered
    /// sources -- there are no hardcoded packet literals. Secret / unreviewed
    /// (generated) / superseded records are excluded with auditable reasons; only
    /// the eligible, FTS-matched record is included.
    #[test]
    fn dp10_memory_packet_derives_from_filtered_sources_no_literals() {
        let backend = SqliteFtsMemoryBackend::new();
        let query = MemoryQuery::new(
            "deploy release process",
            vec![
                reviewed(
                    "Deploy doc",
                    "The deploy release process is documented.",
                    "doc:deploy",
                    10,
                ),
                candidate(
                    "Secret",
                    "deploy release token=shhh",
                    "doc:secret",
                    MemoryReviewState::Reviewed,
                    MemorySensitivity::Secret,
                    10,
                ),
                candidate(
                    "Generated",
                    "generated deploy release note",
                    "doc:generated",
                    MemoryReviewState::Generated,
                    MemorySensitivity::Internal,
                    10,
                ),
                candidate(
                    "Superseded",
                    "old deploy release process",
                    "doc:superseded",
                    MemoryReviewState::Superseded,
                    MemorySensitivity::Internal,
                    10,
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
            "only the eligible FTS-matched record is included; the packet is not a literal",
        );
        assert!(result.excluded.iter().any(|d| d.reason.contains("secret")));
        assert!(
            result
                .excluded
                .iter()
                .any(|d| d.reason.contains("review_state=generated"))
        );
        assert!(
            result
                .excluded
                .iter()
                .any(|d| d.reason.contains("review_state=superseded"))
        );
    }

    /// DP10 / DP5: a `MarkdownMemoryBackend` projects real workpad/source pointers
    /// (with content hashes) into reviewed candidates -- the packet's provenance
    /// traces to real sources, not hardcoded strings.
    #[test]
    fn dp10_memory_markdown_backend_projects_real_source_pointers() {
        use capo_memory::MarkdownMemoryBackend;
        let backend = MarkdownMemoryBackend::new(vec![MarkdownSource {
            title: "Workpad authority".to_string(),
            path: "workpads/depth/knowledge.md".to_string(),
            anchor: Some("DP10".to_string()),
            content_hash: "fnv1a64:depth-knowledge".to_string(),
            body: "Depth consolidates the deterministic suite across ACP, memory, sandbox."
                .to_string(),
            inclusion_reason: "current workpad is the planning authority".to_string(),
        }]);
        let candidates = backend.candidates();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].review_state, MemoryReviewState::Reviewed);
        assert_eq!(candidates[0].source.source_kind, MemorySourceKind::Markdown);
        assert_eq!(candidates[0].source.anchor.as_deref(), Some("DP10"));
        assert!(candidates[0].source.content_hash.starts_with("fnv1a64:"));
    }

    /// DP10 / DP5: the attached packet replays byte-for-byte from its artifact id
    /// across an index rebuild (the replayability anchor).
    #[test]
    fn dp10_memory_packet_replays_byte_for_byte_across_rebuilds() {
        let request = || LiveMemoryPacketRequest {
            memory_packet_id: MemoryPacketId::new("packet-dp10-replay"),
            session_id: SessionId::new("session-dp10"),
            run_id: "run-dp10".to_string(),
            turn_id: "turn-dp10".to_string(),
            purpose: "turn_context".to_string(),
            budget_tokens: 256,
            query_text: "policy retrieval".to_string(),
            candidates: vec![
                reviewed("Alpha", "policy retrieval alpha note", "doc:a", 10),
                reviewed("Beta", "policy retrieval beta note", "doc:b", 10),
                reviewed("Gamma", "unrelated content", "doc:c", 10),
            ],
        };

        let first = MemoryBackend::sqlite_fts(SqliteFtsMemoryBackend::with_index_version(1))
            .build_live_packet(request())
            .expect("first packet");
        let rebuilt = MemoryBackend::sqlite_fts(SqliteFtsMemoryBackend::with_index_version(2))
            .build_live_packet(request())
            .expect("rebuilt packet");

        assert_eq!(
            first.packet_markdown, rebuilt.packet_markdown,
            "the packet markdown replays byte-for-byte across a rebuild",
        );
        assert_eq!(
            first.packet_artifact_id, rebuilt.packet_artifact_id,
            "the packet artifact id is stable across a rebuild",
        );
        assert_eq!(
            first.included.len(),
            2,
            "only the two matching records are included"
        );
    }

    // ==================================================================
    // Sandbox refusal modes (DP7) consolidated invariants.
    // ==================================================================

    fn sandbox_request(root: &std::path::Path, run: &str) -> LocalProcessRequest {
        LocalProcessRequest::new(
            capo_core::RunId::new(run),
            "/bin/echo",
            vec!["hi".to_string()],
            root.to_path_buf(),
            std::collections::HashMap::new(),
        )
    }

    fn sandbox_tmp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("capo-dp10-sb-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("sandbox tmp");
        dir.canonicalize().expect("canonicalize")
    }

    /// DP10 / DP7: a forbidden network egress is REFUSED before launch (an event,
    /// not a silent failure) with no planned process. Deterministic on every
    /// platform (the OS-enforced refusal lives in `capo-runtime` behind a gate).
    #[test]
    fn dp10_sandbox_forbidden_egress_is_refused_as_an_event() {
        let root = sandbox_tmp("egress");
        let sandbox = OsSandbox::new(
            SandboxTier::host_default(),
            SandboxProfile::workspace_confined([root.clone()]),
        );
        let plan = sandbox
            .plan(sandbox_request(&root, "run-dp10-egress"), true)
            .expect("plan");
        assert_eq!(
            plan.enforcement,
            SandboxEnforcement::Refused {
                refusal: SandboxRefusal::NetworkEgressForbidden,
            }
        );
        assert!(plan.request.is_none(), "a refused egress plans no process");
        assert!(
            plan.events
                .iter()
                .any(|e| e.kind == "sandbox.launch_refused"
                    && e.status == "network-egress-forbidden"),
            "the refusal is recorded as an event",
        );
    }

    /// DP10 / DP7: a write whose cwd is outside the confined root is REFUSED before
    /// launch with no planned process, recorded as an event. Deterministic on
    /// every platform.
    #[test]
    fn dp10_sandbox_out_of_root_write_is_refused_as_an_event() {
        let root = sandbox_tmp("write");
        let other = sandbox_tmp("write-other");
        let sandbox = OsSandbox::new(
            SandboxTier::host_default(),
            // Confine writes to `other`, but run with cwd `root` -> out of scope.
            SandboxProfile::workspace_confined([other]),
        );
        let plan = sandbox
            .plan(sandbox_request(&root, "run-dp10-write"), false)
            .expect("plan");
        assert!(
            matches!(
                plan.enforcement,
                SandboxEnforcement::Refused {
                    refusal: SandboxRefusal::WriteOutsideConfinedRoot { .. },
                }
            ),
            "an out-of-root cwd is refused, got {:?}",
            plan.enforcement,
        );
        assert!(plan.request.is_none(), "a refused write plans no process");
        assert!(
            plan.events
                .iter()
                .any(|e| e.kind == "sandbox.launch_refused"
                    && e.status == "write-outside-confined-root"),
            "the refusal is recorded as an event",
        );
    }
}
