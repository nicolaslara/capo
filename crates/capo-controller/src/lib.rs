//! Controller orchestration for Capo.
//!
//! P3 keeps this intentionally fake-only, but the control flow is real: the
//! controller calls each boundary, persists Capo-owned events/projections, and
//! answers inspection requests from SQLite read models.

use std::path::Path;

use capo_adapters::{
    AgentAdapter, FakeAdapterSessionRequest, FakeAdapterTurnRequest, ProviderConnector,
};
use capo_core::{
    AgentId, CommandEnvelope, CommandIntent, EvidenceId, MemoryPacketId, ProjectId, RunId,
    SessionId, TaskId, ToolCallId, TurnId,
};
use capo_memory::{
    MemoryBackend, MemoryCandidate, MemoryReviewState, MemorySensitivity, MemorySourceKind,
    MemorySourceRef, SourceLinkedMemoryPacketRequest,
};
use capo_runtime::{FakeRuntimeStartRequest, RuntimeRunner};
use capo_state::{
    AgentProjection, ArtifactRecord, EventKind, EventRecord, NewEvent, ProjectProjection,
    ProjectionRecord, RedactionState, RunProjection, SessionProjection, SqliteStateStore,
    StateError, StateResult, TaskProjection,
};
use capo_tools::{FakeToolRequest, PermissionPolicy, PermissionRequest, ToolExposure};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeBoundaryController {
    project_id: ProjectId,
    state: SqliteStateStore,
    adapter: AgentAdapter,
    runtime: RuntimeRunner,
    provider: ProviderConnector,
    permission_policy: PermissionPolicy,
    tools: ToolExposure,
    memory: MemoryBackend,
}

impl FakeBoundaryController {
    pub fn open(project_id: ProjectId, state_root: impl AsRef<Path>) -> StateResult<Self> {
        Ok(Self {
            project_id,
            state: SqliteStateStore::open(state_root)?,
            adapter: AgentAdapter::fake(),
            runtime: RuntimeRunner::fake(),
            provider: ProviderConnector::fake(),
            permission_policy: PermissionPolicy::allow_trusted_local(),
            tools: ToolExposure::fake(),
            memory: MemoryBackend::fake(),
        })
    }

    pub fn state(&self) -> &SqliteStateStore {
        &self.state
    }

    pub fn initialize(&self, command: &CommandEnvelope) -> StateResult<ControllerInit> {
        require_intent(command, CommandIntent::InitializeProject);
        Ok(ControllerInit {
            command_id: command.command_id.to_string(),
            state_db_path: self.state.db_path().display().to_string(),
        })
    }

    pub fn register_agent_command(
        &self,
        command: &CommandEnvelope,
    ) -> StateResult<FakeAgentRegistration> {
        require_intent(command, CommandIntent::RegisterAgent);
        let name = command
            .text
            .as_deref()
            .ok_or_else(|| missing_read_model("command.text", &command.command_id))?;
        self.register_agent(name)
    }

    pub fn spawn_agent_command(
        &self,
        command: &CommandEnvelope,
    ) -> StateResult<FakeAgentRegistration> {
        require_intent(command, CommandIntent::RegisterAgent);
        let name = command
            .text
            .as_deref()
            .ok_or_else(|| missing_read_model("command.text", &command.command_id))?;
        self.register_agent(name)
    }

    pub fn send_task_command(&self, command: &CommandEnvelope) -> StateResult<FakeRunRefs> {
        require_intent(command, CommandIntent::SendTask);
        let agent_name = required_structured_arg(command, "agent")?;
        let goal = command
            .text
            .as_deref()
            .ok_or_else(|| missing_read_model("command.text", &command.command_id))?;
        self.send_task_to_agent_name(agent_name, goal)
    }

    pub fn interrupt_command(
        &self,
        command: &CommandEnvelope,
    ) -> StateResult<FakeReadModelObservation> {
        require_intent(command, CommandIntent::InterruptSession);
        let agent_name = required_structured_arg(command, "agent")?;
        let reason = command.text.as_deref().unwrap_or("interrupt requested");
        self.interrupt_agent_name(agent_name, reason)
    }

    pub fn stop_command(&self, command: &CommandEnvelope) -> StateResult<FakeReadModelObservation> {
        require_intent(command, CommandIntent::InterruptSession);
        let agent_name = required_structured_arg(command, "agent")?;
        let reason = command.text.as_deref().unwrap_or("stop requested");
        self.stop_agent_name(agent_name, reason)
    }

    pub fn recover_command(&self, command: &CommandEnvelope) -> StateResult<RecoveryReport> {
        require_intent(command, CommandIntent::Recover);
        let recovery_attempt_id = format!("recovery-{}", command.command_id);
        let started = self.state.begin_recovery(&recovery_attempt_id)?;
        self.state.rebuild_projections()?;
        let completed = self.state.complete_recovery(&recovery_attempt_id)?;
        Ok(RecoveryReport {
            recovery_attempt_id,
            started_sequence: started.started_sequence,
            completed_sequence: completed.completed_sequence.unwrap_or_default(),
            watermark: self.state.watermark("default")?,
        })
    }

    pub fn register_agent(&self, agent_name: &str) -> StateResult<FakeAgentRegistration> {
        let agent_id = AgentId::new(format!("agent-{agent_name}"));
        let provider = self.provider.describe_provider();

        self.state.append_event(
            event(
                &format!("event-register-{}", registration_slug(agent_name)),
                EventKind::AgentRegistered,
                &self.project_id,
            )
            .with_payload(format!(
                "{{\"provider_kind\":\"{}\",\"auth_mode\":\"{}\"}}",
                provider.provider_kind, provider.auth_mode
            )),
            &[
                ProjectionRecord::Project(ProjectProjection {
                    project_id: self.project_id.clone(),
                    name: "Capo".to_string(),
                    status: "active".to_string(),
                    updated_sequence: 0,
                }),
                ProjectionRecord::Agent(AgentProjection {
                    agent_id: agent_id.clone(),
                    project_id: self.project_id.clone(),
                    name: agent_name.to_string(),
                    status: "available".to_string(),
                    current_session_id: None,
                    updated_sequence: 0,
                }),
            ],
        )?;

        Ok(FakeAgentRegistration {
            agent_id,
            agent_name: agent_name.to_string(),
        })
    }

    pub fn registration_for_agent_name(
        &self,
        agent_name: &str,
    ) -> StateResult<FakeAgentRegistration> {
        let agent = self
            .state
            .agent_by_name(agent_name)?
            .ok_or_else(|| missing_read_model("agent.name", &agent_name))?;
        Ok(FakeAgentRegistration {
            agent_id: agent.agent_id,
            agent_name: agent.name,
        })
    }

    pub fn send_task(
        &self,
        registration: &FakeAgentRegistration,
        goal: &str,
    ) -> StateResult<FakeRunRefs> {
        let task_id = TaskId::new(format!("task-{}", slug(goal)));
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
        let adapter_session = self.adapter.open_session(FakeAdapterSessionRequest {
            session_id: session_id.clone(),
            agent_name: registration.agent_name.clone(),
        });
        let adapter_output = self.adapter.send_turn(
            &adapter_session,
            FakeAdapterTurnRequest {
                turn_id: turn_id.clone(),
                agent_name: registration.agent_name.clone(),
                goal: goal.to_string(),
            },
        );
        let permission = self.permission_policy.decide(PermissionRequest {
            session_id: session_id.clone(),
            capability_profile_id: "trusted-local-dev".to_string(),
            scope_json: "[\"tool:capo.session_summary\",\"memory:build_packet\"]".to_string(),
        });
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

        self.state.append_event(
            scoped_event(
                &format!("event-permission-requested-{}", session_id),
                EventKind::PermissionRequested,
                &self.project_id,
                &task_id,
                &registration.agent_id,
                &session_id,
                &run_id,
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
                &format!("event-permission-decided-{}", session_id),
                EventKind::PermissionDecided,
                &self.project_id,
                &task_id,
                &registration.agent_id,
                &session_id,
                &run_id,
            )
            .with_turn(turn_id.to_string())
            .with_payload(format!(
                "{{\"tool_call_id\":\"{}\",\"effect\":\"{}\",\"capability_grant_id\":\"{}\"}}",
                tool_call_id, permission.effect, permission.capability_grant_id
            )),
            &[],
        )?;

        self.state.append_event(
            scoped_event(
                &format!("event-capability-grant-{}", session_id),
                EventKind::CapabilityGrantCreated,
                &self.project_id,
                &task_id,
                &registration.agent_id,
                &session_id,
                &run_id,
            )
            .with_turn(turn_id.to_string())
            .with_payload(format!(
                "{{\"capability_grant_id\":\"{}\",\"effect\":\"{}\"}}",
                permission.capability_grant_id, permission.effect
            )),
            &[ProjectionRecord::CapabilityGrant(
                capo_state::CapabilityGrantProjection {
                    capability_grant_id: permission.capability_grant_id.clone(),
                    capability_profile_id: permission.capability_profile_id.clone(),
                    scope_json: permission.scope_json.clone(),
                    effect: permission.effect.clone(),
                    subject_json: permission.subject_json.clone(),
                    updated_sequence: 0,
                },
            )],
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
                updated_sequence: 0,
            })],
        )?;

        self.state.append_event(
            scoped_event(
                &format!("event-capability-grant-used-{}", session_id),
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

    pub fn send_task_to_agent_name(
        &self,
        agent_name: &str,
        goal: &str,
    ) -> StateResult<FakeRunRefs> {
        let registration = self.registration_for_agent_name(agent_name)?;
        self.send_task(&registration, goal)
    }

    pub fn refs_for_agent_name(&self, agent_name: &str) -> StateResult<FakeRunRefs> {
        let agent = self
            .state
            .agent_by_name(agent_name)?
            .ok_or_else(|| missing_read_model("agent.name", &agent_name))?;
        let session_id = agent
            .current_session_id
            .clone()
            .ok_or_else(|| missing_read_model("agent.current_session_id", &agent.agent_id))?;
        let session = self
            .state
            .session(&session_id)?
            .ok_or_else(|| missing_read_model("session", &session_id))?;
        let run = self
            .state
            .run_for_session(&session_id)?
            .ok_or_else(|| missing_read_model("run.session_id", &session_id))?;
        Ok(FakeRunRefs {
            task_id: session
                .task_id
                .ok_or_else(|| missing_read_model("session.task_id", &session_id))?,
            agent_id: agent.agent_id,
            session_id,
            run_id: run.run_id,
            runtime_process_ref: format!("fake-runtime-process-{agent_name}"),
            external_session_ref: format!("fake-adapter-session-{agent_name}"),
        })
    }

    pub fn observe_agent_name(&self, agent_name: &str) -> StateResult<FakeReadModelObservation> {
        let refs = self.refs_for_agent_name(agent_name)?;
        self.observe(&refs)
    }

    pub fn interrupt_agent_name(
        &self,
        agent_name: &str,
        reason: &str,
    ) -> StateResult<FakeReadModelObservation> {
        let registration = self.registration_for_agent_name(agent_name)?;
        let refs = self.refs_for_agent_name(agent_name)?;
        self.interrupt(&registration, &refs, reason)
    }

    pub fn stop_agent_name(
        &self,
        agent_name: &str,
        reason: &str,
    ) -> StateResult<FakeReadModelObservation> {
        let registration = self.registration_for_agent_name(agent_name)?;
        let refs = self.refs_for_agent_name(agent_name)?;
        self.stop(&registration, &refs, reason)
    }

    pub fn interrupt(
        &self,
        registration: &FakeAgentRegistration,
        refs: &FakeRunRefs,
        reason: &str,
    ) -> StateResult<FakeReadModelObservation> {
        let session = self
            .state
            .session(&refs.session_id)?
            .ok_or_else(|| missing_read_model("session", &refs.session_id))?;
        let runtime_process = self
            .runtime
            .attach_process(refs.run_id.clone(), refs.runtime_process_ref.clone());
        let interrupted_process = self.runtime.interrupt(&runtime_process, reason);
        let adapter_session = self
            .adapter
            .attach_session(refs.session_id.clone(), refs.external_session_ref.clone());
        let adapter_output = self.adapter.interrupt(&adapter_session, reason);

        self.state.append_event(
            scoped_event(
                &format!("event-session-interrupted-{}", refs.session_id),
                EventKind::SessionInterrupted,
                &self.project_id,
                &refs.task_id,
                &registration.agent_id,
                &refs.session_id,
                &refs.run_id,
            )
            .with_payload(format!(
                "{{\"reason\":\"{}\",\"adapter_summary\":\"{}\"}}",
                escape_json(reason),
                escape_json(&adapter_output.summary)
            )),
            &[
                ProjectionRecord::Task(TaskProjection {
                    task_id: refs.task_id.clone(),
                    project_id: self.project_id.clone(),
                    title: session.title.clone(),
                    capo_execution_status: "canceled".to_string(),
                    active_session_id: Some(refs.session_id.clone()),
                    latest_summary: Some(adapter_output.summary),
                    evidence_id: None,
                    updated_sequence: 0,
                }),
                ProjectionRecord::Agent(AgentProjection {
                    agent_id: refs.agent_id.clone(),
                    project_id: self.project_id.clone(),
                    name: registration.agent_name.clone(),
                    status: "available".to_string(),
                    current_session_id: None,
                    updated_sequence: 0,
                }),
                ProjectionRecord::Session(SessionProjection {
                    session_id: refs.session_id.clone(),
                    project_id: self.project_id.clone(),
                    task_id: Some(refs.task_id.clone()),
                    agent_id: refs.agent_id.clone(),
                    title: session.title,
                    status: "canceled".to_string(),
                    current_goal: session.current_goal,
                    latest_summary: Some(format!("Interrupted: {reason}")),
                    latest_confidence: Some(70),
                    latest_blocker: None,
                    updated_sequence: 0,
                }),
                ProjectionRecord::Run(RunProjection {
                    run_id: refs.run_id.clone(),
                    session_id: refs.session_id.clone(),
                    status: interrupted_process.status,
                    recovery_of_run_id: None,
                    updated_sequence: 0,
                }),
            ],
        )?;

        self.observe(refs)
    }

    pub fn stop(
        &self,
        registration: &FakeAgentRegistration,
        refs: &FakeRunRefs,
        reason: &str,
    ) -> StateResult<FakeReadModelObservation> {
        let session = self
            .state
            .session(&refs.session_id)?
            .ok_or_else(|| missing_read_model("session", &refs.session_id))?;
        let runtime_process = self
            .runtime
            .attach_process(refs.run_id.clone(), refs.runtime_process_ref.clone());
        let stopped_process = self.runtime.stop(&runtime_process, reason);
        let adapter_session = self
            .adapter
            .attach_session(refs.session_id.clone(), refs.external_session_ref.clone());
        let adapter_output = self.adapter.stop(&adapter_session, reason);

        self.state.append_event(
            scoped_event(
                &format!("event-session-stopped-{}", refs.session_id),
                EventKind::SessionStopped,
                &self.project_id,
                &refs.task_id,
                &registration.agent_id,
                &refs.session_id,
                &refs.run_id,
            )
            .with_payload(format!(
                "{{\"reason\":\"{}\",\"adapter_summary\":\"{}\"}}",
                escape_json(reason),
                escape_json(&adapter_output.summary)
            )),
            &[
                ProjectionRecord::Task(TaskProjection {
                    task_id: refs.task_id.clone(),
                    project_id: self.project_id.clone(),
                    title: session.title.clone(),
                    capo_execution_status: "completed".to_string(),
                    active_session_id: Some(refs.session_id.clone()),
                    latest_summary: Some(adapter_output.summary),
                    evidence_id: None,
                    updated_sequence: 0,
                }),
                ProjectionRecord::Agent(AgentProjection {
                    agent_id: refs.agent_id.clone(),
                    project_id: self.project_id.clone(),
                    name: registration.agent_name.clone(),
                    status: "available".to_string(),
                    current_session_id: None,
                    updated_sequence: 0,
                }),
                ProjectionRecord::Session(SessionProjection {
                    session_id: refs.session_id.clone(),
                    project_id: self.project_id.clone(),
                    task_id: Some(refs.task_id.clone()),
                    agent_id: refs.agent_id.clone(),
                    title: session.title,
                    status: "completed".to_string(),
                    current_goal: session.current_goal,
                    latest_summary: Some(format!("Stopped: {reason}")),
                    latest_confidence: Some(70),
                    latest_blocker: None,
                    updated_sequence: 0,
                }),
                ProjectionRecord::Run(RunProjection {
                    run_id: refs.run_id.clone(),
                    session_id: refs.session_id.clone(),
                    status: stopped_process.status,
                    recovery_of_run_id: None,
                    updated_sequence: 0,
                }),
            ],
        )?;

        self.observe(refs)
    }

    pub fn observe(&self, refs: &FakeRunRefs) -> StateResult<FakeReadModelObservation> {
        Ok(FakeReadModelObservation {
            task: self
                .state
                .task(&refs.task_id)?
                .ok_or_else(|| missing_read_model("task", &refs.task_id))?,
            agent: self
                .state
                .agent(&refs.agent_id)?
                .ok_or_else(|| missing_read_model("agent", &refs.agent_id))?,
            session: self
                .state
                .session(&refs.session_id)?
                .ok_or_else(|| missing_read_model("session", &refs.session_id))?,
            run: self
                .state
                .run(&refs.run_id)?
                .ok_or_else(|| missing_read_model("run", &refs.run_id))?,
            recent_events: self.state.recent_events_for_session(&refs.session_id, 16)?,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeAgentRegistration {
    pub agent_id: AgentId,
    pub agent_name: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeRunRefs {
    pub task_id: TaskId,
    pub agent_id: AgentId,
    pub session_id: SessionId,
    pub run_id: RunId,
    pub runtime_process_ref: String,
    pub external_session_ref: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeReadModelObservation {
    pub task: TaskProjection,
    pub agent: AgentProjection,
    pub session: SessionProjection,
    pub run: RunProjection,
    pub recent_events: Vec<EventRecord>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ControllerInit {
    pub command_id: String,
    pub state_db_path: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveryReport {
    pub recovery_attempt_id: String,
    pub started_sequence: i64,
    pub completed_sequence: i64,
    pub watermark: Option<i64>,
}

fn event(event_id: &str, kind: EventKind, project_id: &ProjectId) -> NewEvent {
    let mut event = NewEvent::new(event_id, kind, "capo-controller");
    event.project_id = Some(project_id.clone());
    event.idempotency_key = Some(event_id.to_string());
    event
}

fn scoped_event(
    event_id: &str,
    kind: EventKind,
    project_id: &ProjectId,
    task_id: &TaskId,
    agent_id: &AgentId,
    session_id: &SessionId,
    run_id: &RunId,
) -> NewEvent {
    let mut event = event(event_id, kind, project_id);
    event.task_id = Some(task_id.clone());
    event.agent_id = Some(agent_id.clone());
    event.session_id = Some(session_id.clone());
    event.run_id = Some(run_id.clone());
    event
}

trait EventBuilder {
    fn with_payload(self, payload_json: String) -> Self;
    fn with_turn(self, turn_id: impl ToString) -> Self;
}

impl EventBuilder for NewEvent {
    fn with_payload(mut self, payload_json: String) -> Self {
        self.payload_json = payload_json;
        self
    }

    fn with_turn(mut self, turn_id: impl ToString) -> Self {
        self.turn_id = Some(turn_id.to_string());
        self
    }
}

fn slug(value: &str) -> String {
    let slug = value
        .chars()
        .filter_map(|ch| {
            if ch.is_ascii_alphanumeric() {
                Some(ch.to_ascii_lowercase())
            } else if ch.is_ascii_whitespace() || ch == '-' || ch == '_' {
                Some('-')
            } else {
                None
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();

    if slug.is_empty() {
        "work".to_string()
    } else {
        slug
    }
}

fn registration_slug(value: &str) -> String {
    slug(value)
}

fn missing_read_model(kind: &'static str, id: &impl ToString) -> StateError {
    StateError::MissingReadModel {
        kind,
        id: id.to_string(),
    }
}

fn require_intent(command: &CommandEnvelope, expected: CommandIntent) {
    assert_eq!(
        command.intent, expected,
        "controller command intent did not match handler"
    );
}

fn required_structured_arg<'a>(command: &'a CommandEnvelope, key: &str) -> StateResult<&'a str> {
    command
        .structured_args
        .iter()
        .find_map(|(candidate, value)| (candidate == key).then_some(value.as_str()))
        .ok_or_else(|| missing_read_model("command.structured_args", &key))
}

fn escape_json(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn stable_hash(bytes: &[u8]) -> String {
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
    fn fake_boundaries_drive_controller_state_and_interrupt_from_read_models() {
        let controller = FakeBoundaryController::open(ProjectId::new("project-capo"), temp_root())
            .expect("open controller");
        let registration = controller.register_agent("fake-codex").expect("agent");
        let refs = controller
            .send_task(
                &registration,
                "Inspect the project and write a status summary",
            )
            .expect("send task");

        let observation = controller.observe(&refs).expect("observe");
        assert_eq!(observation.task.capo_execution_status, "active");
        assert_eq!(observation.agent.status, "running");
        assert_eq!(observation.session.status, "active");
        assert_eq!(observation.session.latest_confidence, Some(82));
        assert_eq!(observation.run.status, "running");
        assert!(
            observation
                .recent_events
                .iter()
                .any(|event| event.kind == "tool.call_completed")
        );
        for expected_kind in [
            "permission.decided",
            "capability.grant_created",
            "capability.grant_used",
            "tool.call_requested",
            "tool.invocation_started",
            "tool.output_artifact_recorded",
            "tool.output_observed",
            "tool.call_completed",
            "tool.result_delivered",
            "memory.packet_built",
            "evidence.recorded",
        ] {
            assert!(
                observation
                    .recent_events
                    .iter()
                    .any(|event| event.kind == expected_kind),
                "{expected_kind}"
            );
        }
        let packets = controller
            .state()
            .memory_packets_for_session(&refs.session_id)
            .expect("memory packet projection");
        assert_eq!(packets.len(), 1);
        assert_eq!(packets[0].turn_id.as_deref(), Some("turn-fake-codex"));
        assert_eq!(packets[0].run_id.as_ref(), Some(&refs.run_id));
        assert_eq!(
            packets[0].packet_artifact_id.as_deref(),
            Some("artifact-memory-packet-packet-fake-codex")
        );
        let memory_event = observation
            .recent_events
            .iter()
            .find(|event| event.kind == "memory.packet_built")
            .expect("memory packet event");
        assert!(memory_event.payload_json.contains("\"included_count\":3"));
        assert!(memory_event.payload_json.contains("\"excluded_count\":1"));
        assert!(
            memory_event
                .payload_json
                .contains("explanation_artifact_id")
        );

        let interrupted = controller
            .interrupt(&registration, &refs, "P3 smoke interrupt")
            .expect("interrupt");
        assert_eq!(interrupted.task.capo_execution_status, "canceled");
        assert_eq!(interrupted.agent.status, "available");
        assert_eq!(interrupted.session.status, "canceled");
        assert_eq!(interrupted.run.status, "stopping");
        assert!(
            interrupted
                .recent_events
                .iter()
                .any(|event| event.kind == "session.interrupted")
        );

        let reopened = SqliteStateStore::open(controller.state().db_path().parent().unwrap())
            .expect("reopen state");
        assert_eq!(
            reopened
                .session(&refs.session_id)
                .expect("read session")
                .expect("session")
                .status,
            "canceled"
        );
    }

    fn temp_root() -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!("capo-controller-{nanos}"))
    }
}
