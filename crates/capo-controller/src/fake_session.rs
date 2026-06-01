use capo_tools::{CapoToolContext, CapoToolRequest, ToolExposureRequest, ToolExposureResult};

use crate::tool_dispatch::ToolDispatchScope;

use super::*;

/// AI3: how the per-turn memory-packet summary tool is executed on a turn.
///
/// The fake/default path keeps the legacy summary shim (`ToolExposure::fake()`
/// via [`FakeBoundaryController::tools`]) -- a canned observation used only by
/// the deterministic fake-boundary e2e fixtures. The real production path
/// (`RealBoundaryController`) dispatches the SAME `capo.session_summary` tool
/// selection through the REAL [`FakeBoundaryController::dispatch_tool_call`]
/// seam (`authorize_and_invoke` against the Capo registry), so a real turn's
/// tool call produces the canonical `ACI1` observed audit sequence +
/// `ToolInvocation`/`ToolObservation` projections keyed to the turn -- never a
/// fabricated fake summary masquerading as a real dispatched result.
pub(crate) enum ToolDispatchMode<'a> {
    /// The legacy fake summary shim (`self.tools.invoke`). Test/fixture only.
    Fake,
    /// The real dispatch seam: route the per-turn summary tool through
    /// `dispatch_tool_call` over the supplied REAL Capo exposure.
    Real(&'a ToolExposure),
}

/// The narrow turn-tool result the rest of `send_task` consumes (memory packet
/// candidate + tool-output artifact), produced by EITHER dispatch mode so the
/// surrounding scaffolding is identical regardless of which tool surface ran.
struct TurnToolResult {
    tool_call_id: ToolCallId,
    tool_name: String,
    output_artifact_id: String,
    summary: String,
}

impl FakeBoundaryController {
    pub fn send_task(
        &self,
        registration: &FakeAgentRegistration,
        goal: &str,
    ) -> StateResult<FakeRunRefs> {
        let task_id = TaskId::new(format!("task-{}", slug(goal)));
        self.send_task_with_task_id(registration, task_id, goal)
    }

    pub fn send_task_with_task_id(
        &self,
        registration: &FakeAgentRegistration,
        task_id: TaskId,
        goal: &str,
    ) -> StateResult<FakeRunRefs> {
        self.send_task_with_dispatch_mode(registration, task_id, goal, ToolDispatchMode::Fake)
    }

    /// AI3: the production `send_task` entry that routes the per-turn summary
    /// tool through the REAL dispatch seam (`exposure.authorize_and_invoke`)
    /// instead of the fake summary shim. `RealBoundaryController::send_task`
    /// calls this with its live Capo exposure.
    pub(crate) fn send_task_with_real_tools(
        &self,
        registration: &FakeAgentRegistration,
        task_id: TaskId,
        goal: &str,
        exposure: &ToolExposure,
    ) -> StateResult<FakeRunRefs> {
        self.send_task_with_dispatch_mode(
            registration,
            task_id,
            goal,
            ToolDispatchMode::Real(exposure),
        )
    }

    fn send_task_with_dispatch_mode(
        &self,
        registration: &FakeAgentRegistration,
        task_id: TaskId,
        goal: &str,
        dispatch_mode: ToolDispatchMode<'_>,
    ) -> StateResult<FakeRunRefs> {
        let session_id = SessionId::new(format!("session-{}", registration.agent_name));
        let run_id = RunId::new(format!("run-{}", registration.agent_name));
        let turn_id = TurnId::new(format!("turn-{}", registration.agent_name));
        let tool_call_id = ToolCallId::new(format!("tool-{}", registration.agent_name));
        let memory_packet_id = MemoryPacketId::new(format!("packet-{}", registration.agent_name));
        let evidence_id = EvidenceId::new(format!("evidence-{}", registration.agent_name));

        let runtime_process = self.runtime.start(FakeRuntimeStartRequest {
            run_id: run_id.clone(),
            agent_name: registration.agent_name.clone(),
        });
        let adapter_session = self.adapter.open_session(AdapterSessionRequest {
            session_id: session_id.clone(),
            agent_name: registration.agent_name.clone(),
        });
        // AI2: drive chat through the agent's BOUND adapter via the fallible
        // `try_send_turn` seam. The deterministic fake/scripted handles default
        // this to an infallible `send_turn` (unchanged behaviour). A Codex-BOUND
        // handle drives the real read-only one-shot when the live-provider gate is
        // open, or fails CLOSED-FAST with a typed error (no spawn, no blocking)
        // when it is off -- surfaced here as `StateError::CodexLiveChat`, never a
        // fabricated fake summary. Mock/unbound agents never reach the Codex path.
        let adapter_output = self
            .adapter
            .try_send_turn(
                &adapter_session,
                &TurnRequest {
                    turn_id: turn_id.clone(),
                    agent_name: registration.agent_name.clone(),
                    goal: goal.to_string(),
                },
            )
            .map_err(|error| StateError::CodexLiveChat(error.to_string()))?;
        // SG1 (single canonical decide): the FAKE fixture path keeps its legacy
        // upfront `permission_policy.decide` gate (a hand-rolled scope string) so
        // the deterministic e2e fixtures stay byte-for-byte. The REAL loop does NOT
        // decide here: its single canonical decide is the one the dispatch's
        // `authorize_and_invoke` runs over the tool's OWN required scope, and the
        // loop CONSUMES the typed `PermissionDecideOutcome`/`ToolRefusal` that
        // dispatch returns (below). A second upfront decide on a different,
        // hand-rolled scope would be exactly the two-decide-paths smell this removes,
        // so it is fake-only.
        let dispatch_scope = ToolDispatchScope {
            task_id: task_id.clone(),
            agent_id: registration.agent_id.clone(),
            session_id: session_id.clone(),
            run_id: run_id.clone(),
            turn_id: turn_id.clone(),
            tool_call_id: tool_call_id.clone(),
        };
        let permission: Option<PermissionDecision> = match &dispatch_mode {
            ToolDispatchMode::Fake => Some(self.permission_policy.decide(PermissionRequest {
                session_id: session_id.clone(),
                capability_profile_id: self.permission_policy.default_profile_id().to_string(),
                scope_json: "[\"tool:invoke:capo.session_summary\",\"state:read:session\",\"state:read:tool\",\"state:read:permission_queue\",\"memory:build_packet:session\"]".to_string(),
            })),
            // The real path's decide is the dispatch's own; nothing decides here.
            ToolDispatchMode::Real(_) => None,
        };
        if let ToolDispatchMode::Fake = dispatch_mode {
            let permission = permission
                .as_ref()
                .expect("fake dispatch always decides upfront");
            let permission_event_suffix = slug(&format!(
                "{} {}",
                tool_call_id, permission.capability_grant_id
            ));
            if permission.effect != "allow" {
                self.record_denied_tool_request(
                    registration,
                    goal,
                    &task_id,
                    &session_id,
                    &run_id,
                    &turn_id,
                    &tool_call_id,
                    runtime_process.status.clone(),
                    &adapter_output.status,
                    adapter_output.confidence,
                    &adapter_output.summary,
                    &adapter_session.external_session_ref,
                    permission,
                    &permission_event_suffix,
                )?;
                return Ok(FakeRunRefs {
                    task_id,
                    agent_id: registration.agent_id.clone(),
                    session_id,
                    run_id,
                    runtime_process_ref: runtime_process.runtime_process_ref,
                    external_session_ref: adapter_output.external_session_ref,
                });
            }
        }
        // AI3: execute the per-turn summary tool. In the FAKE mode the legacy
        // summary shim runs here and the canonical tool-call events are
        // hand-rolled later (preserving the fixture path byte-for-byte). In the
        // REAL mode the SAME `capo.session_summary` selection is dispatched
        // through `dispatch_tool_call` (`authorize_and_invoke`), which persists
        // the canonical observed audit sequence + `ToolCall`/`ToolObservation`
        // projections keyed to the turn -- so a real production turn's tool call
        // is a real dispatched result, not a fabricated fake summary.
        let tool_result = match &dispatch_mode {
            ToolDispatchMode::Fake => {
                let fake = self.tools.invoke(FakeToolRequest {
                    tool_call_id: tool_call_id.clone(),
                    session_id: session_id.clone(),
                    tool_name: adapter_output.tool_name.clone(),
                    input_summary: adapter_output.summary.clone(),
                });
                TurnToolResult {
                    tool_call_id: fake.tool_call_id,
                    tool_name: fake.tool_name,
                    output_artifact_id: fake.output_artifact_id,
                    summary: fake.summary,
                }
            }
            // SG1: the real loop consumes the dispatch's typed decide outcome. On a
            // DENY, `dispatch_turn_summary_tool` returns the structured `ToolRefusal`
            // the dispatch built; the loop reflects it back into the blocked session
            // state and drives the early return FROM THE TYPED REFUSAL, rather than a
            // second upfront decide short-circuiting before any tool runs.
            ToolDispatchMode::Real(exposure) => {
                match self.dispatch_turn_summary_tool(
                    exposure,
                    &dispatch_scope,
                    &adapter_output.tool_name,
                    &adapter_output.summary,
                )? {
                    Ok(tool_result) => tool_result,
                    Err(refusal) => {
                        self.record_real_dispatch_denied(
                            registration,
                            goal,
                            &dispatch_scope,
                            runtime_process.status.clone(),
                            &adapter_output.status,
                            adapter_output.confidence,
                            &adapter_output.summary,
                            &adapter_session.external_session_ref,
                            &refusal,
                        )?;
                        return Ok(FakeRunRefs {
                            task_id,
                            agent_id: registration.agent_id.clone(),
                            session_id,
                            run_id,
                            runtime_process_ref: runtime_process.runtime_process_ref,
                            external_session_ref: adapter_output.external_session_ref,
                        });
                    }
                }
            }
        };
        let memory_packet = self.memory.build_source_linked_packet(
            SourceLinkedMemoryPacketRequest {
                memory_packet_id: memory_packet_id.clone(),
                session_id: session_id.clone(),
                run_id: run_id.to_string(),
                turn_id: turn_id.to_string(),
                purpose: "turn_context".to_string(),
                budget_tokens: 256,
                candidates: vec![
                    MemoryCandidate {
                        title: "Active task goal".to_string(),
                        body: goal.to_string(),
                        source: MemorySourceRef {
                            source_kind: MemorySourceKind::Event,
                            source_ref: format!("event-task-started-{}", session_id),
                            anchor: Some("goal".to_string()),
                            content_hash: stable_hash(goal.as_bytes()),
                        },
                        review_state: MemoryReviewState::Reviewed,
                        sensitivity: MemorySensitivity::Internal,
                        estimated_tokens: 32,
                        inclusion_reason: "current task goal for this turn".to_string(),
                    },
                    MemoryCandidate {
                        title: "Adapter summary".to_string(),
                        body: adapter_output.summary.clone(),
                        source: MemorySourceRef {
                            source_kind: MemorySourceKind::Artifact,
                            source_ref: tool_result.output_artifact_id.clone(),
                            anchor: Some("summary".to_string()),
                            content_hash: stable_hash(adapter_output.summary.as_bytes()),
                        },
                        review_state: MemoryReviewState::Reviewed,
                        sensitivity: MemorySensitivity::Internal,
                        estimated_tokens: 48,
                        inclusion_reason: "latest session summary from tool output".to_string(),
                    },
                    MemoryCandidate {
                        title: "Prototype workpad authority".to_string(),
                        body: "Prototype task status and evidence live in workpads/prototype/tasks.md and knowledge.md.".to_string(),
                        source: MemorySourceRef {
                            source_kind: MemorySourceKind::Markdown,
                            source_ref: "workpads/prototype/tasks.md".to_string(),
                            anchor: Some("P9".to_string()),
                            content_hash: stable_hash(b"workpads/prototype/tasks.md#P9"),
                        },
                        review_state: MemoryReviewState::Reviewed,
                        sensitivity: MemorySensitivity::Internal,
                        estimated_tokens: 36,
                        inclusion_reason: "current workpad is the planning authority".to_string(),
                    },
                    MemoryCandidate {
                        title: "Generated scratch note".to_string(),
                        body: "Generated notes require review before packet inclusion.".to_string(),
                        source: MemorySourceRef {
                            source_kind: MemorySourceKind::Artifact,
                            source_ref: format!("artifact-scratch-{}", session_id),
                            anchor: None,
                            content_hash: stable_hash(b"generated scratch note"),
                        },
                        review_state: MemoryReviewState::Generated,
                        sensitivity: MemorySensitivity::Internal,
                        estimated_tokens: 16,
                        inclusion_reason: "should be excluded until reviewed".to_string(),
                    },
                ],
            },
        );

        self.state.record_artifact(ArtifactRecord {
            artifact_id: tool_result.output_artifact_id.clone(),
            project_id: Some(self.project_id.clone()),
            session_id: Some(session_id.clone()),
            run_id: Some(run_id.clone()),
            kind: "tool-output".to_string(),
            uri: format!("artifacts/{}/tool-output.md", session_id),
            content_hash: "fake-tool-output-hash".to_string(),
            size_bytes: tool_result.summary.len() as i64,
            redaction_state: RedactionState::Safe,
        })?;
        self.state.record_artifact(ArtifactRecord {
            artifact_id: memory_packet.packet_artifact_id.clone(),
            project_id: Some(self.project_id.clone()),
            session_id: Some(session_id.clone()),
            run_id: Some(run_id.clone()),
            kind: "memory-packet".to_string(),
            uri: format!("artifacts/{}/memory-packet.md", session_id),
            content_hash: stable_hash(memory_packet.packet_markdown.as_bytes()),
            size_bytes: memory_packet.packet_markdown.len() as i64,
            redaction_state: RedactionState::Safe,
        })?;
        self.state.record_artifact(ArtifactRecord {
            artifact_id: memory_packet.explanation_artifact_id.clone(),
            project_id: Some(self.project_id.clone()),
            session_id: Some(session_id.clone()),
            run_id: Some(run_id.clone()),
            kind: "memory-explanation".to_string(),
            uri: format!("artifacts/{}/memory-explanation.md", session_id),
            content_hash: stable_hash(memory_packet.explanation_markdown.as_bytes()),
            size_bytes: memory_packet.explanation_markdown.len() as i64,
            redaction_state: RedactionState::Safe,
        })?;

        self.state.append_event(
            scoped_event(
                &format!("event-task-started-{}", session_id),
                EventKind::SessionStarted,
                &self.project_id,
                &task_id,
                &registration.agent_id,
                &session_id,
                &run_id,
            )
            .with_turn(turn_id.clone())
            .with_payload(format!(
                "{{\"goal\":\"{}\",\"runtime_process_ref\":\"{}\",\"external_session_ref\":\"{}\"}}",
                escape_json(goal),
                runtime_process.runtime_process_ref,
                adapter_output.external_session_ref
            )),
            &[
                ProjectionRecord::Task(TaskProjection {
                    task_id: task_id.clone(),
                    project_id: self.project_id.clone(),
                    title: goal.to_string(),
                    capo_execution_status: "active".to_string(),
                    active_session_id: Some(session_id.clone()),
                    latest_summary: Some(adapter_output.summary.clone()),
                    evidence_id: Some(evidence_id.clone()),
                    updated_sequence: 0,
                }),
                ProjectionRecord::Agent(AgentProjection {
                    agent_id: registration.agent_id.clone(),
                    project_id: self.project_id.clone(),
                    name: registration.agent_name.clone(),
                    status: "running".to_string(),
                    current_session_id: Some(session_id.clone()),
                    updated_sequence: 0,
                }),
                ProjectionRecord::Session(SessionProjection {
                    session_id: session_id.clone(),
                    project_id: self.project_id.clone(),
                    task_id: Some(task_id.clone()),
                    agent_id: registration.agent_id.clone(),
                    title: goal.to_string(),
                    status: adapter_output.status.clone(),
                    current_goal: goal.to_string(),
                    latest_summary: Some(adapter_output.summary.clone()),
                    latest_confidence: Some(adapter_output.confidence),
                    latest_blocker: None,
                    external_session_ref: Some(adapter_session.external_session_ref.clone()),
                    updated_sequence: 0,
                }),
                ProjectionRecord::Run(RunProjection {
                    run_id: run_id.clone(),
                    session_id: session_id.clone(),
                    status: runtime_process.status,
                    recovery_of_run_id: None,
                    updated_sequence: 0,
                }),
            ],
        )?;

        // AI3: the hand-rolled permission + canonical tool-call event sequence is
        // the LEGACY fake summary-shim path. The REAL path already persisted the
        // permission + canonical sequence + projection inside
        // `dispatch_turn_summary_tool` above (through `authorize_and_invoke`), so
        // it must NOT also hand-roll a second, duplicate tool-call sequence.
        if let ToolDispatchMode::Fake = dispatch_mode {
            let permission = permission
                .as_ref()
                .expect("fake dispatch always decides upfront");
            let permission_event_suffix = slug(&format!(
                "{} {}",
                tool_call_id, permission.capability_grant_id
            ));
            self.record_permission_decision(
                registration,
                &task_id,
                &session_id,
                &run_id,
                &turn_id,
                &tool_call_id,
                permission,
                &permission_event_suffix,
            )?;

            self.state.append_event(
                scoped_event(
                    &format!("event-tool-requested-{}", session_id),
                    EventKind::ToolCallRequested,
                    &self.project_id,
                    &task_id,
                    &registration.agent_id,
                    &session_id,
                    &run_id,
                )
                .with_turn(turn_id.to_string())
                // Stamp the shared item ref (the tool_call_id) on every tool.* event of
                // this one synthetic turn-context tool call so
                // `persisted_turn_ref`/`reconstruct_turn_finished`'s dedup collapses
                // tool.call_requested/invocation_started/call_completed into a SINGLE
                // observed tool ref per call -- matching the replay-identity invariant
                // the real dispatch path already honors. Without it the distinct
                // per-kind payloads fall through to the raw_event_hash/payload_json
                // fallback and over-count one call as three refs.
                .with_item(tool_call_id.to_string())
                .with_payload(format!("{{\"tool\":\"{}\"}}", tool_result.tool_name)),
                &[ProjectionRecord::ToolCall(capo_state::ToolCallProjection {
                    tool_call_id: tool_call_id.clone(),
                    session_id: session_id.clone(),
                    turn_id: Some(turn_id.to_string()),
                    tool_name: tool_result.tool_name.clone(),
                    tool_origin: "capo".to_string(),
                    status: "requested".to_string(),
                    input_artifact_id: None,
                    output_artifact_id: None,
                    provenance: capo_state::ToolCallProvenance::default(),
                    updated_sequence: 0,
                })],
            )?;

            self.state.append_event(
                scoped_event(
                    &format!(
                        "event-capability-grant-used-{}-{}",
                        session_id, permission_event_suffix
                    ),
                    EventKind::CapabilityGrantUsed,
                    &self.project_id,
                    &task_id,
                    &registration.agent_id,
                    &session_id,
                    &run_id,
                )
                .with_turn(turn_id.to_string())
                .with_item(tool_call_id.to_string())
                .with_payload(format!(
                    "{{\"capability_grant_id\":\"{}\",\"tool_call_id\":\"{}\"}}",
                    permission.capability_grant_id, tool_result.tool_call_id
                )),
                &[],
            )?;

            self.state.append_event(
                scoped_event(
                    &format!("event-tool-invocation-started-{}", session_id),
                    EventKind::ToolInvocationStarted,
                    &self.project_id,
                    &task_id,
                    &registration.agent_id,
                    &session_id,
                    &run_id,
                )
                .with_turn(turn_id.to_string())
                .with_item(tool_call_id.to_string())
                .with_payload(format!(
                    "{{\"tool_call_id\":\"{}\",\"tool\":\"{}\",\"instrumentation\":\"full\"}}",
                    tool_result.tool_call_id, tool_result.tool_name
                )),
                &[],
            )?;

            self.state.append_event(
            scoped_event(
                &format!("event-tool-output-artifact-{}", session_id),
                EventKind::ToolOutputArtifactRecorded,
                &self.project_id,
                &task_id,
                &registration.agent_id,
                &session_id,
                &run_id,
            )
            .with_turn(turn_id.to_string())
            .with_item(tool_call_id.to_string())
            .with_payload(format!(
                "{{\"tool_call_id\":\"{}\",\"output_artifact_id\":\"{}\",\"redaction_state\":\"safe\"}}",
                tool_result.tool_call_id, tool_result.output_artifact_id
            )),
            &[],
        )?;

            self.state.append_event(
                scoped_event(
                    &format!("event-tool-output-observed-{}", session_id),
                    EventKind::ToolOutputObserved,
                    &self.project_id,
                    &task_id,
                    &registration.agent_id,
                    &session_id,
                    &run_id,
                )
                .with_turn(turn_id.to_string())
                .with_item(tool_call_id.to_string())
                .with_payload(format!(
                    "{{\"tool_call_id\":\"{}\",\"summary\":\"{}\"}}",
                    tool_result.tool_call_id,
                    escape_json(&tool_result.summary)
                )),
                &[],
            )?;

            self.state.append_event(
                scoped_event(
                    &format!("event-tool-completed-{}", session_id),
                    EventKind::ToolCallCompleted,
                    &self.project_id,
                    &task_id,
                    &registration.agent_id,
                    &session_id,
                    &run_id,
                )
                .with_turn(turn_id.to_string())
                .with_item(tool_call_id.to_string())
                .with_payload(format!(
                    "{{\"tool\":\"{}\",\"output_artifact_id\":\"{}\"}}",
                    tool_result.tool_name, tool_result.output_artifact_id
                )),
                &[ProjectionRecord::ToolCall(capo_state::ToolCallProjection {
                    tool_call_id: tool_call_id.clone(),
                    session_id: session_id.clone(),
                    turn_id: Some(turn_id.to_string()),
                    tool_name: tool_result.tool_name.clone(),
                    tool_origin: "capo".to_string(),
                    status: "completed".to_string(),
                    input_artifact_id: None,
                    output_artifact_id: Some(tool_result.output_artifact_id.clone()),
                    provenance: capo_state::ToolCallProvenance::default(),
                    updated_sequence: 0,
                })],
            )?;

            self.state.append_event(
            scoped_event(
                &format!("event-tool-result-delivered-{}", session_id),
                EventKind::ToolResultDelivered,
                &self.project_id,
                &task_id,
                &registration.agent_id,
                &session_id,
                &run_id,
            )
            .with_turn(turn_id.to_string())
            .with_item(tool_result.tool_call_id.to_string())
            .with_payload(format!(
                "{{\"tool_call_id\":\"{}\",\"external_session_ref\":\"{}\",\"delivery\":\"fake-adapter-accepted\"}}",
                tool_result.tool_call_id, adapter_session.external_session_ref
            )),
            &[],
        )?;
        }

        self.state.append_event(
            scoped_event(
                &format!("event-memory-packet-{}", session_id),
                EventKind::MemoryPacketBuilt,
                &self.project_id,
                &task_id,
                &registration.agent_id,
                &session_id,
                &run_id,
            )
            .with_turn(turn_id.to_string())
            .with_payload(format!(
                "{{\"packet_artifact_id\":\"{}\",\"explanation_artifact_id\":\"{}\",\"included_count\":{},\"excluded_count\":{}}}",
                memory_packet.packet_artifact_id,
                memory_packet.explanation_artifact_id,
                memory_packet.included.len(),
                memory_packet.excluded.len()
            )),
            &[ProjectionRecord::MemoryPacketRef(
                capo_state::MemoryPacketProjection {
                    memory_packet_id,
                    project_id: self.project_id.clone(),
                    task_id: Some(task_id.clone()),
                    agent_id: Some(registration.agent_id.clone()),
                    session_id: Some(session_id.clone()),
                    run_id: Some(run_id.clone()),
                    turn_id: Some(turn_id.to_string()),
                    packet_artifact_id: Some(memory_packet.packet_artifact_id),
                    purpose: memory_packet.purpose,
                    updated_sequence: 0,
                },
            )],
        )?;

        self.state.append_event(
            scoped_event(
                &format!("event-evidence-{}", session_id),
                EventKind::EvidenceRecorded,
                &self.project_id,
                &task_id,
                &registration.agent_id,
                &session_id,
                &run_id,
            )
            .with_turn(turn_id.to_string())
            .with_payload(format!(
                "{{\"artifact_id\":\"{}\",\"confidence\":{}}}",
                tool_result.output_artifact_id, adapter_output.confidence
            )),
            &[ProjectionRecord::Evidence(capo_state::EvidenceProjection {
                evidence_id,
                project_id: self.project_id.clone(),
                task_id: Some(task_id.clone()),
                session_id: Some(session_id.clone()),
                run_id: Some(run_id.clone()),
                kind: "fake-boundary-e2e".to_string(),
                artifact_id: Some(tool_result.output_artifact_id),
                confidence: adapter_output.confidence,
                updated_sequence: 0,
            })],
        )?;

        Ok(FakeRunRefs {
            task_id,
            agent_id: registration.agent_id.clone(),
            session_id,
            run_id,
            runtime_process_ref: runtime_process.runtime_process_ref,
            external_session_ref: adapter_session.external_session_ref,
        })
    }

    /// AI3 + SG1: dispatch the per-turn `capo.session_summary` tool through the
    /// REAL `dispatch_tool_call` seam (`authorize_and_invoke` against the Capo
    /// registry), persisting the canonical observed audit sequence +
    /// `ToolCall`/`ToolObservation` projections keyed to the turn.
    ///
    /// This dispatch's decide is the SINGLE canonical permission decide for the
    /// real loop. The method CONSUMES `outcome.decide`:
    ///
    /// - on ALLOW, returns the narrow [`TurnToolResult`] the surrounding
    ///   `send_task` scaffolding consumes (memory-packet candidate + tool-output
    ///   artifact), derived from the REAL result -- never a fabricated fake summary;
    /// - on DENY, returns the structured, agent-readable [`ToolRefusal`] the
    ///   dispatch built (`outcome.decide.refusal`), so the loop can reflect it back
    ///   to the agent / persist it and drive the early-return from the TYPED refusal
    ///   rather than a raw error string or a silent continue.
    ///
    /// The outer `StateResult` is the store I/O fallibility; the inner
    /// `Result<_, ToolRefusal>` is the decide verdict the loop reflects on.
    fn dispatch_turn_summary_tool(
        &self,
        exposure: &ToolExposure,
        scope: &ToolDispatchScope,
        tool_name: &str,
        adapter_summary: &str,
    ) -> StateResult<Result<TurnToolResult, ToolRefusal>> {
        let outcome = self.dispatch_tool_call(
            exposure,
            scope,
            ToolExposureRequest::Capo(CapoToolRequest {
                tool_call_id: scope.tool_call_id.clone(),
                session_id: scope.session_id.clone(),
                tool_id: tool_name.to_string(),
                capability_profile_id: self.permission_policy.default_profile_id().to_string(),
                context: CapoToolContext {
                    task_status: String::new(),
                    agent_status: String::new(),
                    // `capo.session_summary` renders its output from
                    // `context.session_summary`, so feeding the adapter's
                    // observed turn summary keeps the dispatched tool output the
                    // turn's real summary rather than an empty string.
                    session_summary: adapter_summary.to_string(),
                    workpad_excerpt: String::new(),
                    evidence_note: String::new(),
                    capability_scope: "state:read:session".to_string(),
                },
            }),
        )?;
        // SG1: the loop's single decide gate. The dispatch already recorded
        // `permission.requested`/`permission.decided` (and blocked the tool) for a
        // deny; here the loop CONSUMES the typed decide outcome it returned. On a
        // deny, surface the structured refusal so the caller reflects it back rather
        // than treating a `denied` result as a tool output.
        if !outcome.decide.allowed {
            let refusal = outcome
                .decide
                .refusal
                .clone()
                .unwrap_or_else(|| ToolRefusal {
                    tool_name: outcome.tool_name.clone(),
                    decision_source: outcome.decide.decision_source.clone(),
                    scope_json: String::new(),
                    reason: outcome.decide.explanation.clone(),
                });
            return Ok(Err(refusal));
        }
        // The narrow output the memory packet / artifacts consume comes from the
        // REAL dispatched result (the Capo registry's rendered output + the
        // dispatch-issued artifact id), not the fake shim.
        let summary = match &outcome.result {
            ToolExposureResult::Capo(result) => result.output.clone(),
            // The real path always dispatches a Capo `capo.session_summary` call;
            // any other variant here is a wiring bug.
            other => panic!("dispatch_turn_summary_tool expected a Capo result, got {other:?}"),
        };
        let output_artifact_id = outcome
            .output_artifact_id
            .clone()
            .unwrap_or_else(|| format!("artifact-tool-{}", scope.session_id));
        Ok(Ok(TurnToolResult {
            tool_call_id: outcome.tool_call_id,
            tool_name: outcome.tool_name,
            output_artifact_id,
            summary,
        }))
    }

    /// SG1: record the blocked session state for a REAL-loop dispatch that the
    /// single canonical decide DENIED, reflecting the typed [`ToolRefusal`] back.
    ///
    /// The dispatch's decide step ALREADY persisted `permission.requested` ->
    /// `permission.decided` and drove the tool-call projection to its terminal
    /// `denied` status (and, for a `reject_always`, the durable deny grant), so this
    /// helper does NOT re-emit any permission/tool events -- doing so would be the
    /// second decide path this change removes. It only appends the `SessionStarted`
    /// event carrying the blocked task/agent/session/run projections, with the
    /// session blocker set to the refusal's agent-readable message so the loop
    /// SURFACES the typed refusal (it is queryable on the session read model, not
    /// just discarded). The memory-packet / artifact / evidence steps are skipped
    /// (the early return), exactly as the fake deny path skips them.
    #[allow(clippy::too_many_arguments)]
    fn record_real_dispatch_denied(
        &self,
        registration: &FakeAgentRegistration,
        goal: &str,
        scope: &ToolDispatchScope,
        run_status: String,
        adapter_status: &str,
        adapter_confidence: i64,
        adapter_summary: &str,
        external_session_ref: &str,
        refusal: &ToolRefusal,
    ) -> StateResult<()> {
        let blocker = refusal.agent_message();
        self.state.append_event(
            scoped_event(
                &format!("event-task-started-{}", scope.session_id),
                EventKind::SessionStarted,
                &self.project_id,
                &scope.task_id,
                &registration.agent_id,
                &scope.session_id,
                &scope.run_id,
            )
            .with_turn(scope.turn_id.clone())
            .with_payload(
                serde_json::json!({
                    "goal": goal,
                    "permission_effect": "deny",
                    "decision_source": refusal.decision_source,
                    "adapter_status": adapter_status,
                    "refusal": blocker,
                })
                .to_string(),
            ),
            &[
                ProjectionRecord::Task(TaskProjection {
                    task_id: scope.task_id.clone(),
                    project_id: self.project_id.clone(),
                    title: goal.to_string(),
                    capo_execution_status: "blocked".to_string(),
                    active_session_id: Some(scope.session_id.clone()),
                    latest_summary: Some(adapter_summary.to_string()),
                    evidence_id: None,
                    updated_sequence: 0,
                }),
                ProjectionRecord::Agent(AgentProjection {
                    agent_id: registration.agent_id.clone(),
                    project_id: self.project_id.clone(),
                    name: registration.agent_name.clone(),
                    status: "paused".to_string(),
                    current_session_id: Some(scope.session_id.clone()),
                    updated_sequence: 0,
                }),
                ProjectionRecord::Session(SessionProjection {
                    session_id: scope.session_id.clone(),
                    project_id: self.project_id.clone(),
                    task_id: Some(scope.task_id.clone()),
                    agent_id: registration.agent_id.clone(),
                    title: goal.to_string(),
                    status: "waiting_for_permission".to_string(),
                    current_goal: goal.to_string(),
                    latest_summary: Some(adapter_summary.to_string()),
                    latest_confidence: Some(adapter_confidence),
                    // The session blocker IS the typed refusal reflected back.
                    latest_blocker: Some(blocker),
                    external_session_ref: Some(external_session_ref.to_string()),
                    updated_sequence: 0,
                }),
                ProjectionRecord::Run(RunProjection {
                    run_id: scope.run_id.clone(),
                    session_id: scope.session_id.clone(),
                    status: run_status,
                    recovery_of_run_id: None,
                    updated_sequence: 0,
                }),
            ],
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn record_denied_tool_request(
        &self,
        registration: &FakeAgentRegistration,
        goal: &str,
        task_id: &TaskId,
        session_id: &SessionId,
        run_id: &RunId,
        turn_id: &TurnId,
        tool_call_id: &ToolCallId,
        run_status: String,
        adapter_status: &str,
        adapter_confidence: i64,
        adapter_summary: &str,
        external_session_ref: &str,
        permission: &PermissionDecision,
        permission_event_suffix: &str,
    ) -> StateResult<()> {
        self.state.append_event(
            scoped_event(
                &format!("event-task-started-{}", session_id),
                EventKind::SessionStarted,
                &self.project_id,
                task_id,
                &registration.agent_id,
                session_id,
                run_id,
            )
            .with_turn(turn_id.clone())
            .with_payload(format!(
                "{{\"goal\":\"{}\",\"permission_effect\":\"{}\"}}",
                escape_json(goal),
                permission.effect
            )),
            &[
                ProjectionRecord::Task(TaskProjection {
                    task_id: task_id.clone(),
                    project_id: self.project_id.clone(),
                    title: goal.to_string(),
                    capo_execution_status: "blocked".to_string(),
                    active_session_id: Some(session_id.clone()),
                    latest_summary: Some(adapter_summary.to_string()),
                    evidence_id: None,
                    updated_sequence: 0,
                }),
                ProjectionRecord::Agent(AgentProjection {
                    agent_id: registration.agent_id.clone(),
                    project_id: self.project_id.clone(),
                    name: registration.agent_name.clone(),
                    status: "paused".to_string(),
                    current_session_id: Some(session_id.clone()),
                    updated_sequence: 0,
                }),
                ProjectionRecord::Session(SessionProjection {
                    session_id: session_id.clone(),
                    project_id: self.project_id.clone(),
                    task_id: Some(task_id.clone()),
                    agent_id: registration.agent_id.clone(),
                    title: goal.to_string(),
                    status: "waiting_for_permission".to_string(),
                    current_goal: goal.to_string(),
                    latest_summary: Some(adapter_summary.to_string()),
                    latest_confidence: Some(adapter_confidence),
                    latest_blocker: Some(permission.explanation.clone()),
                    external_session_ref: Some(external_session_ref.to_string()),
                    updated_sequence: 0,
                }),
                ProjectionRecord::Run(RunProjection {
                    run_id: run_id.clone(),
                    session_id: session_id.clone(),
                    status: run_status,
                    recovery_of_run_id: None,
                    updated_sequence: 0,
                }),
            ],
        )?;
        self.record_permission_decision(
            registration,
            task_id,
            session_id,
            run_id,
            turn_id,
            tool_call_id,
            permission,
            permission_event_suffix,
        )?;
        self.state.append_event(
            scoped_event(
                &format!("event-tool-requested-{}-{}", session_id, permission_event_suffix),
                EventKind::ToolCallRequested,
                &self.project_id,
                task_id,
                &registration.agent_id,
                session_id,
                run_id,
            )
            .with_turn(turn_id.to_string())
            // Share the tool_call_id item ref with the rest of this call's tool.*
            // events (here only the one denied request) so reconstruction dedups on
            // the same identity the allow path uses.
            .with_item(tool_call_id.to_string())
            .with_payload(format!(
                "{{\"tool\":\"capo.session_summary\",\"status\":\"permission_denied\",\"adapter_status\":\"{}\"}}",
                escape_json(adapter_status)
            )),
            &[ProjectionRecord::ToolCall(capo_state::ToolCallProjection {
                tool_call_id: tool_call_id.clone(),
                session_id: session_id.clone(),
                turn_id: Some(turn_id.to_string()),
                tool_name: "capo.session_summary".to_string(),
                tool_origin: "capo".to_string(),
                status: "denied".to_string(),
                input_artifact_id: None,
                output_artifact_id: None,
                provenance: capo_state::ToolCallProvenance::default(),
                updated_sequence: 0,
            })],
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn record_permission_decision(
        &self,
        registration: &FakeAgentRegistration,
        task_id: &TaskId,
        session_id: &SessionId,
        run_id: &RunId,
        turn_id: &TurnId,
        tool_call_id: &ToolCallId,
        permission: &PermissionDecision,
        permission_event_suffix: &str,
    ) -> StateResult<()> {
        self.state.append_event(
            scoped_event(
                &format!(
                    "event-permission-requested-{}-{}",
                    session_id, permission_event_suffix
                ),
                EventKind::PermissionRequested,
                &self.project_id,
                task_id,
                &registration.agent_id,
                session_id,
                run_id,
            )
            .with_turn(turn_id.to_string())
            .with_payload(format!(
                "{{\"tool_call_id\":\"{}\",\"scope_json\":{}}}",
                tool_call_id, permission.scope_json
            )),
            &[],
        )?;

        self.state.append_event(
            scoped_event(
                &format!(
                    "event-permission-decided-{}-{}",
                    session_id, permission_event_suffix
                ),
                EventKind::PermissionDecided,
                &self.project_id,
                task_id,
                &registration.agent_id,
                session_id,
                run_id,
            )
            .with_turn(turn_id.to_string())
            .with_payload(format!(
                "{{\"tool_call_id\":\"{}\",\"effect\":\"{}\",\"capability_grant_id\":\"{}\",\"decision_source\":\"{}\",\"persistence\":\"{}\",\"explanation\":\"{}\"}}",
                tool_call_id,
                permission.effect,
                permission.capability_grant_id,
                permission.decision_source,
                permission.persistence,
                escape_json(&permission.explanation)
            )),
            &[],
        )?;

        self.state.append_event(
            scoped_event(
                &format!(
                    "event-capability-grant-{}-{}",
                    session_id, permission_event_suffix
                ),
                EventKind::CapabilityGrantCreated,
                &self.project_id,
                task_id,
                &registration.agent_id,
                session_id,
                run_id,
            )
            .with_turn(turn_id.to_string())
            .with_payload(format!(
                "{{\"capability_grant_id\":\"{}\",\"effect\":\"{}\",\"decision_source\":\"{}\",\"persistence\":\"{}\"}}",
                permission.capability_grant_id,
                permission.effect,
                permission.decision_source,
                permission.persistence
            )),
            &[ProjectionRecord::CapabilityGrant(
                capo_state::CapabilityGrantProjection {
                    capability_grant_id: permission.capability_grant_id.clone(),
                    capability_profile_id: permission.capability_profile_id.clone(),
                    scope_json: permission.scope_json.clone(),
                    effect: permission.effect.clone(),
                    subject_json: permission.subject_json.clone(),
                    decision_source: permission.decision_source.clone(),
                    persistence: permission.persistence.clone(),
                    explanation: permission.explanation.clone(),
                    // SG3: the legacy fake-session grant carries no lifecycle
                    // timestamps (the real loop's grant writer stamps them).
                    created_at: None,
                    expires_at: None,
                    revoked_at: None,
                    updated_sequence: 0,
                },
            )],
        )?;
        Ok(())
    }
}
