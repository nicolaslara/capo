//! Evaluation and evidence scaffolding.
//!
//! Later prototype tasks will turn run outcomes and review evidence into
//! inspectable evaluation records.

use capo_core::{BoundaryBinding, BoundaryKind};
use capo_state::{
    EventRecord, EvidenceProjection, MemoryPacketProjection, RunProjection, SessionProjection,
    SqliteStateStore, TaskOutcomeReportProjection, ToolCallProjection,
};

/// The first evaluation path is local and evidence-backed.
pub const PROTOTYPE_EVALUATION_LAYER: &str = "local-evidence-report";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EvaluationLayer {
    Fake(FakeEvaluationLayer),
}

impl EvaluationLayer {
    pub fn fake() -> Self {
        Self::Fake(FakeEvaluationLayer)
    }

    pub fn binding(&self) -> BoundaryBinding {
        match self {
            Self::Fake(layer) => layer.binding(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeEvaluationLayer;

impl FakeEvaluationLayer {
    pub fn binding(&self) -> BoundaryBinding {
        BoundaryBinding::fake(BoundaryKind::EvaluationLayer, "fake-eval")
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskOutcomeReportInput {
    pub session: SessionProjection,
    pub run: RunProjection,
    pub evidence: Vec<EvidenceProjection>,
    pub tool_calls: Vec<ToolCallProjection>,
    pub memory_packets: Vec<MemoryPacketProjection>,
    pub events: Vec<EventRecord>,
    pub review_outcome: String,
    pub report_artifact_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskOutcomeReport {
    pub projection: TaskOutcomeReportProjection,
    pub markdown: String,
}

impl TaskOutcomeReport {
    pub fn from_state(
        state: &SqliteStateStore,
        session_id: &capo_core::SessionId,
        review_outcome: impl Into<String>,
        report_artifact_id: Option<String>,
    ) -> Result<Self, String> {
        let session = state
            .session(session_id)
            .map_err(|error| format!("{error:?}"))?
            .ok_or_else(|| format!("missing session read model: {session_id}"))?;
        let run = state
            .run_for_session(session_id)
            .map_err(|error| format!("{error:?}"))?
            .ok_or_else(|| format!("missing run read model for session: {session_id}"))?;
        let evidence = state
            .evidence_for_session(session_id)
            .map_err(|error| format!("{error:?}"))?
            .into_iter()
            .filter(|item| item.kind != "task_outcome_report")
            .collect();
        let tool_calls = state
            .tool_calls_for_session(session_id)
            .map_err(|error| format!("{error:?}"))?;
        let memory_packets = state
            .memory_packets_for_session(session_id)
            .map_err(|error| format!("{error:?}"))?;
        let events = state
            .recent_events_for_session(session_id, 1000)
            .map_err(|error| format!("{error:?}"))?
            .into_iter()
            .filter(|event| event.kind != "task.outcome_report_generated")
            .collect();

        Self::build(TaskOutcomeReportInput {
            session,
            run,
            evidence,
            tool_calls,
            memory_packets,
            events,
            review_outcome: review_outcome.into(),
            report_artifact_id,
        })
    }

    pub fn build(input: TaskOutcomeReportInput) -> Result<Self, String> {
        if !is_terminal_outcome(&input.run.status) {
            return Err(format!(
                "task outcome reports require completed or interrupted runs, got {}",
                input.run.status
            ));
        }
        let task_id = input
            .session
            .task_id
            .clone()
            .ok_or_else(|| "task outcome reports require a task-linked session".to_string())?;
        let started_sequence = input
            .events
            .first()
            .map(|event| event.sequence)
            .unwrap_or(0);
        let completed_sequence = input.events.last().map(|event| event.sequence).unwrap_or(0);
        let duration_sequence_span = completed_sequence.saturating_sub(started_sequence);
        let action_count = input.events.len() as i64;
        let review_outcome = input.review_outcome;
        let report_id = format!(
            "task-outcome-{}-{}",
            task_id,
            stable_eval_hash(&format!(
                "{}:{}:{}:{}:{}",
                input.session.session_id,
                input.run.run_id,
                input.run.status,
                completed_sequence,
                review_outcome
            ))
        );
        let projection = TaskOutcomeReportProjection {
            task_outcome_report_id: report_id,
            project_id: input.session.project_id.clone(),
            task_id,
            session_id: input.session.session_id.clone(),
            run_id: input.run.run_id.clone(),
            outcome_status: input.run.status.clone(),
            started_sequence,
            completed_sequence,
            duration_sequence_span,
            action_count,
            tool_call_count: input.tool_calls.len() as i64,
            evidence_count: input.evidence.len() as i64,
            memory_packet_count: input.memory_packets.len() as i64,
            confidence: input.session.latest_confidence,
            blocker: input.session.latest_blocker.clone(),
            review_outcome,
            report_artifact_id: input.report_artifact_id,
            updated_sequence: 0,
        };
        let markdown = render_task_outcome_report(
            &projection,
            &input.session,
            &input.run,
            &input.evidence,
            &input.tool_calls,
            &input.memory_packets,
            &input.events,
        );
        Ok(Self {
            projection,
            markdown,
        })
    }
}

fn is_terminal_outcome(status: &str) -> bool {
    matches!(
        status,
        "completed" | "canceled" | "failed" | "interrupted" | "exited" | "exited_unknown"
    )
}

fn render_task_outcome_report(
    report: &TaskOutcomeReportProjection,
    session: &SessionProjection,
    run: &RunProjection,
    evidence: &[EvidenceProjection],
    tool_calls: &[ToolCallProjection],
    memory_packets: &[MemoryPacketProjection],
    events: &[EventRecord],
) -> String {
    let mut markdown = format!(
        "<!-- capo:task-outcome-report -->\n# Capo Task Outcome - {}\n\n## Summary\n\n- Project: `{}`\n- Task: `{}`\n- Session: `{}`\n- Run: `{}`\n- Outcome: `{}`\n- Review outcome: `{}`\n- Duration sequence span: `{}`\n- Actions: `{}`\n- Tool calls: `{}`\n- Evidence refs: `{}`\n- Memory packets: `{}`\n- Confidence: `{}`\n- Blocker: `{}`\n\n## Goal\n\n{}\n\n## Evidence\n\n",
        session.title,
        report.project_id,
        report.task_id,
        report.session_id,
        run.run_id,
        report.outcome_status,
        report.review_outcome,
        report.duration_sequence_span,
        report.action_count,
        report.tool_call_count,
        report.evidence_count,
        report.memory_packet_count,
        report
            .confidence
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_string()),
        report.blocker.as_deref().unwrap_or("none"),
        session.current_goal
    );
    if evidence.is_empty() {
        markdown.push_str("- none\n");
    } else {
        for item in evidence {
            markdown.push_str(&format!(
                "- `{}` kind=`{}` artifact=`{}` confidence=`{}`\n",
                item.evidence_id,
                item.kind,
                item.artifact_id.as_deref().unwrap_or("none"),
                item.confidence
            ));
        }
    }
    markdown.push_str("\n## Tool Calls\n\n");
    if tool_calls.is_empty() {
        markdown.push_str("- none\n");
    } else {
        for call in tool_calls {
            markdown.push_str(&format!(
                "- `{}` tool=`{}` origin=`{}` status=`{}` input=`{}` output=`{}`\n",
                call.tool_call_id,
                call.tool_name,
                call.tool_origin,
                call.status,
                call.input_artifact_id.as_deref().unwrap_or("none"),
                call.output_artifact_id.as_deref().unwrap_or("none")
            ));
        }
    }
    markdown.push_str("\n## Memory Packets\n\n");
    if memory_packets.is_empty() {
        markdown.push_str("- none\n");
    } else {
        for packet in memory_packets {
            markdown.push_str(&format!(
                "- `{}` purpose=`{}` artifact=`{}`\n",
                packet.memory_packet_id,
                packet.purpose,
                packet.packet_artifact_id.as_deref().unwrap_or("none")
            ));
        }
    }
    markdown.push_str("\n## Event Trace\n\n");
    for event in events {
        markdown.push_str(&format!(
            "- `{}` kind=`{}` event=`{}` item=`{}`\n",
            event.sequence,
            event.kind,
            event.event_id,
            event.item_id.as_deref().unwrap_or("none")
        ));
    }
    markdown
}

fn stable_eval_hash(value: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use capo_core::{
        AgentId, EvidenceId, MemoryPacketId, ProjectId, RunId, SessionId, TaskId, ToolCallId,
    };
    use capo_state::RedactionState;

    #[test]
    fn prototype_evaluation_layer_is_local() {
        assert_eq!(PROTOTYPE_EVALUATION_LAYER, "local-evidence-report");
    }

    #[test]
    fn fake_evaluation_reports_eval_boundary() {
        assert_eq!(
            EvaluationLayer::fake().binding().kind,
            BoundaryKind::EvaluationLayer
        );
    }

    #[test]
    fn task_outcome_report_is_derived_from_state_evidence() {
        let project_id = ProjectId::new("project-capo");
        let task_id = TaskId::new("task-me2");
        let session_id = SessionId::new("session-me2");
        let run_id = RunId::new("run-me2");
        let report = TaskOutcomeReport::build(TaskOutcomeReportInput {
            session: SessionProjection {
                session_id: session_id.clone(),
                project_id: project_id.clone(),
                task_id: Some(task_id.clone()),
                agent_id: AgentId::new("agent-me2"),
                title: "ME2".to_string(),
                status: "exited_unknown".to_string(),
                current_goal: "derive outcome report".to_string(),
                latest_summary: Some("done".to_string()),
                latest_confidence: Some(82),
                latest_blocker: Some("review found no blockers".to_string()),
                external_session_ref: Some("adapter-session-me2".to_string()),
                updated_sequence: 6,
            },
            run: RunProjection {
                run_id: run_id.clone(),
                session_id: session_id.clone(),
                status: "completed".to_string(),
                recovery_of_run_id: None,
                updated_sequence: 6,
            },
            evidence: vec![EvidenceProjection {
                evidence_id: EvidenceId::new("evidence-me2"),
                project_id: project_id.clone(),
                task_id: Some(task_id.clone()),
                session_id: Some(session_id.clone()),
                run_id: Some(run_id.clone()),
                kind: "summary".to_string(),
                artifact_id: Some("artifact-summary".to_string()),
                confidence: 82,
                updated_sequence: 5,
            }],
            tool_calls: vec![ToolCallProjection {
                tool_call_id: ToolCallId::new("tool-me2"),
                session_id: session_id.clone(),
                turn_id: Some("turn-1".to_string()),
                tool_name: "capo.status".to_string(),
                tool_origin: "capo".to_string(),
                status: "completed".to_string(),
                input_artifact_id: None,
                output_artifact_id: Some("artifact-tool".to_string()),
                updated_sequence: 4,
            }],
            memory_packets: vec![MemoryPacketProjection {
                memory_packet_id: MemoryPacketId::new("packet-me2"),
                project_id: project_id.clone(),
                task_id: Some(task_id.clone()),
                agent_id: None,
                session_id: Some(session_id.clone()),
                run_id: Some(run_id.clone()),
                turn_id: Some("turn-1".to_string()),
                packet_artifact_id: Some("artifact-packet".to_string()),
                purpose: "turn_context".to_string(),
                updated_sequence: 3,
            }],
            events: vec![
                EventRecord {
                    sequence: 2,
                    event_id: "event-start".to_string(),
                    kind: "session.started".to_string(),
                    actor: "test".to_string(),
                    project_id: Some(project_id.clone()),
                    task_id: Some(task_id.clone()),
                    agent_id: None,
                    session_id: Some(session_id.clone()),
                    run_id: Some(run_id.clone()),
                    turn_id: None,
                    item_id: None,
                    payload_json: "{}".to_string(),
                    idempotency_key: None,
                    redaction_state: RedactionState::Safe.as_str().to_string(),
                },
                EventRecord {
                    sequence: 6,
                    event_id: "event-complete".to_string(),
                    kind: "run.exited".to_string(),
                    actor: "test".to_string(),
                    project_id: Some(project_id),
                    task_id: Some(task_id.clone()),
                    agent_id: None,
                    session_id: Some(session_id),
                    run_id: Some(run_id),
                    turn_id: None,
                    item_id: None,
                    payload_json: "{}".to_string(),
                    idempotency_key: None,
                    redaction_state: RedactionState::Safe.as_str().to_string(),
                },
            ],
            review_outcome: "reviewed_no_blockers".to_string(),
            report_artifact_id: Some("artifact-task-outcome".to_string()),
        })
        .expect("build report");

        assert_eq!(report.projection.task_id, task_id);
        assert_eq!(report.projection.duration_sequence_span, 4);
        assert_eq!(report.projection.action_count, 2);
        assert_eq!(report.projection.tool_call_count, 1);
        assert_eq!(report.projection.evidence_count, 1);
        assert_eq!(report.projection.memory_packet_count, 1);
        assert_eq!(report.projection.confidence, Some(82));
        assert_eq!(report.projection.review_outcome, "reviewed_no_blockers");
        assert!(
            report
                .markdown
                .contains("<!-- capo:task-outcome-report -->")
        );
        assert!(report.markdown.contains("tool=`capo.status`"));
        assert!(report.markdown.contains("artifact=`artifact-summary`"));
        assert!(report.markdown.contains("review found no blockers"));
    }
}
