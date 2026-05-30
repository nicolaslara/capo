use super::*;

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
        let adapter_output = self.adapter.send_turn(
            &adapter_session,
            TurnRequest {
                turn_id: turn_id.clone(),
                agent_name: registration.agent_name.clone(),
                goal: goal.to_string(),
            },
        );
        let permission = self.permission_policy.decide(PermissionRequest {
            session_id: session_id.clone(),
            capability_profile_id: self.permission_policy.default_profile_id().to_string(),
            scope_json: "[\"tool:invoke:capo.session_summary\",\"state:read:session\",\"state:read:tool\",\"state:read:permission_queue\",\"memory:build_packet:session\"]".to_string(),
        });
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
                &permission,
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
        let tool_result = self.tools.invoke(FakeToolRequest {
            tool_call_id: tool_call_id.clone(),
            session_id: session_id.clone(),
            tool_name: adapter_output.tool_name.clone(),
            input_summary: adapter_output.summary.clone(),
        });
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

        self.record_permission_decision(
            registration,
            &task_id,
            &session_id,
            &run_id,
            &turn_id,
            &tool_call_id,
            &permission,
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
            .with_payload(format!(
                "{{\"tool\":\"{}\",\"output_artifact_id\":\"{}\"}}",
                tool_result.tool_name, tool_result.output_artifact_id
            )),
            &[ProjectionRecord::ToolCall(capo_state::ToolCallProjection {
                tool_call_id,
                session_id: session_id.clone(),
                turn_id: Some(turn_id.to_string()),
                tool_name: tool_result.tool_name,
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
            .with_payload(format!(
                "{{\"tool_call_id\":\"{}\",\"external_session_ref\":\"{}\",\"delivery\":\"fake-adapter-accepted\"}}",
                tool_result.tool_call_id, adapter_session.external_session_ref
            )),
            &[],
        )?;

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
                    updated_sequence: 0,
                },
            )],
        )?;
        Ok(())
    }
}
