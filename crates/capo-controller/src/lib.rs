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
    AgentId, EvidenceId, MemoryPacketId, ProjectId, RunId, SessionId, TaskId, ToolCallId, TurnId,
};
use capo_memory::{FakeMemoryPacketRequest, MemoryBackend};
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
        let memory_packet = self.memory.build_packet(FakeMemoryPacketRequest {
            memory_packet_id: memory_packet_id.clone(),
            session_id: session_id.clone(),
            goal_slug: slug(goal),
            summary: adapter_output.summary.clone(),
        });

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
            artifact_id: memory_packet.artifact_id.clone(),
            project_id: Some(self.project_id.clone()),
            session_id: Some(session_id.clone()),
            run_id: Some(run_id.clone()),
            kind: "memory-packet".to_string(),
            uri: format!("artifacts/{}/memory-packet.md", session_id),
            content_hash: "fake-memory-packet-hash".to_string(),
            size_bytes: memory_packet.source_summary.len() as i64,
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
                "{{\"packet_artifact_id\":\"{}\"}}",
                memory_packet.artifact_id
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
                    packet_artifact_id: Some(memory_packet.artifact_id),
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
            recent_events: self.state.recent_events_for_session(&refs.session_id, 10)?,
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

fn escape_json(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
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
            "capability.grant_created",
            "tool.call_requested",
            "tool.call_completed",
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
