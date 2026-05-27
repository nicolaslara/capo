use std::fs;
use std::path::{Path, PathBuf};

use capo_core::{CommandIntent, CommandTarget, EvidenceId, SessionId, ToolCallId};
use capo_eval::TaskOutcomeReport;
use capo_state::{
    ArtifactRecord, EventKind, EventRecord, EvidenceProjection, MemoryPacketProjection, NewEvent,
    ProjectionRecord, RedactionState, ReviewFindingProjection, RunProjection, SessionProjection,
    SqliteStateStore, ToolCallProjection, ToolObservationProjection,
};

use crate::cli_surface::{ParsedArgs, has_flag, optional_arg, required_arg};
use crate::{debug_error, envelope, escape_json, stable_cli_hash, state};

pub(crate) fn export_evidence(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let session_id = SessionId::new(required_arg(args, "--session")?);
    let out = PathBuf::from(required_arg(args, "--out")?);
    let command = envelope(
        "evidence-export",
        CommandTarget::Session(session_id.clone()),
        CommandIntent::ExportEvidence,
        Some(out.display().to_string()),
    );
    let state = state(parsed)?;
    let session = state
        .session(&session_id)
        .map_err(debug_error)?
        .ok_or_else(|| format!("missing session read model: {session_id}"))?;
    let evidence = state
        .evidence_for_session(&session_id)
        .map_err(debug_error)?;
    let events = state
        .recent_events_for_session(&session_id, 20)
        .map_err(debug_error)?;
    let run = state
        .run_for_session(&session_id)
        .map_err(debug_error)?
        .ok_or_else(|| format!("missing run read model for session: {session_id}"))?;
    let tool_calls = state
        .tool_calls_for_session(&session_id)
        .map_err(debug_error)?;
    let tool_observations = state
        .tool_observations_for_session(&session_id)
        .map_err(debug_error)?;
    let memory_packets = state
        .memory_packets_for_session(&session_id)
        .map_err(debug_error)?;
    fs::create_dir_all(&out).map_err(|error| error.to_string())?;
    let path = out.join(format!("{session_id}.md"));
    write_evidence_file(
        &path,
        &render_evidence(
            &session,
            &run,
            &evidence,
            &tool_calls,
            &tool_observations,
            &memory_packets,
            &events,
        ),
    )?;
    Ok(format!(
        "evidence_exported=true\npath={}\ncommand_id={}\n",
        path.display(),
        command.command_id
    ))
}

pub(crate) fn export_task_outcome_report(
    parsed: &ParsedArgs,
    args: &[String],
) -> Result<String, String> {
    let session_id = SessionId::new(required_arg(args, "--session")?);
    let out = PathBuf::from(required_arg(args, "--out")?);
    let command = envelope(
        "eval-task-outcome",
        CommandTarget::Session(session_id.clone()),
        CommandIntent::ExportEvidence,
        Some(out.display().to_string()),
    );
    let state = state(parsed)?;
    let review_outcome = derive_review_outcome(&state, &session_id)?;
    let report = TaskOutcomeReport::from_state(&state, &session_id, review_outcome.clone(), None)?;
    let artifact_seed = format!(
        "{}:{}",
        report.projection.task_outcome_report_id, review_outcome
    );
    let artifact_id = format!("artifact-task-outcome-{}", stable_cli_hash(&artifact_seed));
    let mut projection = report.projection.clone();
    projection.report_artifact_id = Some(artifact_id.clone());
    fs::create_dir_all(&out).map_err(|error| error.to_string())?;
    let path = out.join(format!("{artifact_id}.md"));
    write_task_outcome_report_file(&path, &report.markdown)?;
    let content_hash = stable_cli_hash(&report.markdown);
    state
        .record_artifact(ArtifactRecord {
            artifact_id: artifact_id.clone(),
            project_id: Some(projection.project_id.clone()),
            session_id: Some(session_id.clone()),
            run_id: Some(projection.run_id.clone()),
            kind: "task_outcome_report".to_string(),
            uri: path.display().to_string(),
            content_hash: content_hash.clone(),
            size_bytes: report.markdown.len() as i64,
            redaction_state: RedactionState::Safe,
        })
        .map_err(debug_error)?;

    let evidence_id = format!("evidence-{artifact_id}");
    let sequence = state
        .append_event(
            NewEvent {
                event_id: format!(
                    "event-task-outcome-{}",
                    stable_cli_hash(&projection.task_outcome_report_id)
                ),
                kind: EventKind::TaskOutcomeReportGenerated,
                actor: "cli".to_string(),
                project_id: Some(projection.project_id.clone()),
                task_id: Some(projection.task_id.clone()),
                agent_id: None,
                session_id: Some(session_id.clone()),
                run_id: Some(projection.run_id.clone()),
                turn_id: None,
                item_id: Some(projection.task_outcome_report_id.clone()),
                payload_json: format!(
                    "{{\"task_outcome_report_id\":\"{}\",\"artifact_id\":\"{}\",\"content_hash\":\"{}\",\"review_outcome\":\"{}\"}}",
                    escape_json(&projection.task_outcome_report_id),
                    escape_json(&artifact_id),
                    escape_json(&content_hash),
                    escape_json(&review_outcome)
                ),
                idempotency_key: Some(format!(
                    "task-outcome:{}:{}:{}:{}",
                    projection.task_id,
                    session_id,
                    review_outcome,
                    projection.completed_sequence
                )),
                redaction_state: RedactionState::Safe,
            },
            &[
                ProjectionRecord::TaskOutcomeReport(projection.clone()),
                ProjectionRecord::Evidence(EvidenceProjection {
                    evidence_id: EvidenceId::new(evidence_id.clone()),
                    project_id: projection.project_id.clone(),
                    task_id: Some(projection.task_id.clone()),
                    session_id: Some(session_id.clone()),
                    run_id: Some(projection.run_id.clone()),
                    kind: "task_outcome_report".to_string(),
                    artifact_id: Some(artifact_id.clone()),
                    confidence: projection.confidence.unwrap_or(0),
                    updated_sequence: 0,
                }),
            ],
        )
        .map_err(debug_error)?;

    Ok(format!(
        "task_outcome_report_exported=true\nreport_id={}\ntask_id={}\nsession_id={session_id}\nartifact_id={artifact_id}\npath={}\ncontent_hash={content_hash}\nsequence={sequence}\ncommand_id={}\n",
        projection.task_outcome_report_id,
        projection.task_id,
        path.display(),
        command.command_id
    ))
}

pub(crate) fn record_review_finding(
    parsed: &ParsedArgs,
    args: &[String],
) -> Result<String, String> {
    let session_id = SessionId::new(required_arg(args, "--session")?);
    let reviewer = required_arg(args, "--reviewer")?;
    let finding_kind = required_arg(args, "--kind")?;
    if !matches!(finding_kind.as_str(), "blocker" | "finding" | "no_blockers") {
        return Err("--kind must be blocker, finding, or no_blockers".to_string());
    }
    let summary = required_arg(args, "--summary")?;
    let out = PathBuf::from(required_arg(args, "--out")?);
    let severity =
        optional_arg(args, "--severity").unwrap_or_else(|| default_review_severity(&finding_kind));
    let tool_call_id = optional_arg(args, "--tool-call").map(ToolCallId::new);
    let follow_up_workpad_task_id =
        aliased_optional_arg(args, "--follow-up-source-task", "--follow-up-workpad-task")?;
    let state = state(parsed)?;
    let session = state
        .session(&session_id)
        .map_err(debug_error)?
        .ok_or_else(|| format!("missing session read model: {session_id}"))?;
    let task_id = session
        .task_id
        .clone()
        .ok_or_else(|| format!("session is not linked to a task: {session_id}"))?;
    let run = state.run_for_session(&session_id).map_err(debug_error)?;
    let run_id = run.as_ref().map(|run| run.run_id.clone());
    if let Some(tool_call_id) = &tool_call_id {
        let session_tool_calls = state
            .tool_calls_for_session(&session_id)
            .map_err(debug_error)?;
        if !session_tool_calls
            .iter()
            .any(|tool_call| &tool_call.tool_call_id == tool_call_id)
        {
            return Err(format!(
                "tool call is not linked to session: {}",
                tool_call_id
            ));
        }
    }
    if let Some(workpad_task_id) = &follow_up_workpad_task_id
        && state
            .workpad_task(&session.project_id, workpad_task_id)
            .map_err(debug_error)?
            .is_none()
    {
        return Err(format!("missing follow-up source task: {workpad_task_id}"));
    }
    let command = envelope(
        "review-record",
        CommandTarget::Session(session_id.clone()),
        CommandIntent::RecordReviewFinding,
        Some(summary.clone()),
    );
    let finding_seed = format!(
        "{}:{}:{}:{}:{}:{}:{}",
        session_id,
        reviewer,
        finding_kind,
        severity,
        summary,
        tool_call_id
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_else(|| "none".to_string()),
        follow_up_workpad_task_id.as_deref().unwrap_or("none")
    );
    let review_finding_id = format!("review-finding-{}", stable_cli_hash(&finding_seed));
    let artifact_id = format!("artifact-{review_finding_id}");
    fs::create_dir_all(&out).map_err(|error| error.to_string())?;
    let path = out.join(format!("{artifact_id}.md"));
    let markdown = render_review_finding_artifact(
        &session,
        run.as_ref(),
        &review_finding_id,
        &artifact_id,
        &reviewer,
        &finding_kind,
        &severity,
        &summary,
        tool_call_id.as_ref(),
        follow_up_workpad_task_id.as_deref(),
    );
    write_review_finding_file(&path, &markdown)?;
    let content_hash = stable_cli_hash(&markdown);
    state
        .record_artifact(ArtifactRecord {
            artifact_id: artifact_id.clone(),
            project_id: Some(session.project_id.clone()),
            session_id: Some(session_id.clone()),
            run_id: run_id.clone(),
            kind: "review".to_string(),
            uri: path.display().to_string(),
            content_hash: content_hash.clone(),
            size_bytes: markdown.len() as i64,
            redaction_state: RedactionState::Safe,
        })
        .map_err(debug_error)?;

    let evidence_kind = review_evidence_kind(&finding_kind);
    let evidence_id = format!("evidence-{review_finding_id}");
    let sequence = state
        .append_event(
            NewEvent {
                event_id: format!("event-{}", review_finding_id),
                kind: EventKind::ReviewFindingRecorded,
                actor: "cli".to_string(),
                project_id: Some(session.project_id.clone()),
                task_id: Some(task_id.clone()),
                agent_id: Some(session.agent_id.clone()),
                session_id: Some(session_id.clone()),
                run_id: run_id.clone(),
                turn_id: None,
                item_id: Some(review_finding_id.clone()),
                payload_json: format!(
                    "{{\"review_finding_id\":\"{}\",\"artifact_id\":\"{}\",\"content_hash\":\"{}\",\"finding_kind\":\"{}\",\"severity\":\"{}\"}}",
                    escape_json(&review_finding_id),
                    escape_json(&artifact_id),
                    escape_json(&content_hash),
                    escape_json(&finding_kind),
                    escape_json(&severity)
                ),
                idempotency_key: Some(format!("review-finding:{review_finding_id}")),
                redaction_state: RedactionState::Safe,
            },
            &[
                ProjectionRecord::ReviewFinding(ReviewFindingProjection {
                    review_finding_id: review_finding_id.clone(),
                    project_id: session.project_id.clone(),
                    task_id: task_id.clone(),
                    session_id: session_id.clone(),
                    run_id: run_id.clone(),
                    tool_call_id: tool_call_id.clone(),
                    workpad_task_id: follow_up_workpad_task_id.clone(),
                    reviewer: reviewer.clone(),
                    finding_kind: finding_kind.clone(),
                    severity: severity.clone(),
                    summary: summary.clone(),
                    status: review_finding_status(&finding_kind).to_string(),
                    evidence_artifact_id: Some(artifact_id.clone()),
                    follow_up: follow_up_workpad_task_id.clone(),
                    updated_sequence: 0,
                }),
                ProjectionRecord::Evidence(EvidenceProjection {
                    evidence_id: EvidenceId::new(evidence_id.clone()),
                    project_id: session.project_id,
                    task_id: Some(task_id),
                    session_id: Some(session_id.clone()),
                    run_id,
                    kind: evidence_kind.to_string(),
                    artifact_id: Some(artifact_id.clone()),
                    confidence: review_confidence(&finding_kind),
                    updated_sequence: 0,
                }),
            ],
        )
        .map_err(debug_error)?;

    Ok(format!(
        "review_finding_recorded=true\nreview_finding_id={review_finding_id}\nevidence_id={evidence_id}\nartifact_id={artifact_id}\npath={}\nsequence={sequence}\ncommand_id={}\n",
        path.display(),
        command.command_id
    ))
}

fn derive_review_outcome(
    state: &SqliteStateStore,
    session_id: &SessionId,
) -> Result<String, String> {
    let findings = state
        .review_findings_for_session(session_id)
        .map_err(debug_error)?;
    if let Some(finding) = findings.iter().max_by_key(|finding| {
        (
            finding.updated_sequence,
            review_finding_precedence(&finding.finding_kind),
        )
    }) {
        return Ok(match finding.finding_kind.as_str() {
            "blocker" | "finding" => "reviewed_with_findings",
            "no_blockers" => "reviewed_no_blockers",
            _ => "not_reviewed",
        }
        .to_string());
    }

    let evidence = state
        .evidence_for_session(session_id)
        .map_err(debug_error)?;
    let latest = evidence
        .iter()
        .filter_map(|item| {
            let rank = match item.kind.as_str() {
                "review_blockers" | "review_findings" => 2,
                "review_no_blockers" | "reviewed_no_blockers" => 1,
                _ => return None,
            };
            Some((item.updated_sequence, rank, item.kind.as_str()))
        })
        .max_by_key(|(sequence, rank, _)| (*sequence, *rank));

    Ok(match latest.map(|(_, _, kind)| kind) {
        Some("review_blockers" | "review_findings") => "reviewed_with_findings",
        Some("review_no_blockers" | "reviewed_no_blockers") => "reviewed_no_blockers",
        _ => "not_reviewed",
    }
    .to_string())
}

fn review_finding_precedence(finding_kind: &str) -> i64 {
    match finding_kind {
        "blocker" => 3,
        "finding" => 2,
        "no_blockers" => 1,
        _ => 0,
    }
}

fn review_evidence_kind(finding_kind: &str) -> &'static str {
    match finding_kind {
        "blocker" => "review_blockers",
        "finding" => "review_findings",
        "no_blockers" => "review_no_blockers",
        _ => "review_findings",
    }
}

fn review_finding_status(finding_kind: &str) -> &'static str {
    match finding_kind {
        "no_blockers" => "closed",
        _ => "open",
    }
}

fn review_confidence(finding_kind: &str) -> i64 {
    match finding_kind {
        "no_blockers" => 90,
        "blocker" => 80,
        _ => 70,
    }
}

fn default_review_severity(finding_kind: &str) -> String {
    match finding_kind {
        "blocker" => "high",
        "finding" => "medium",
        "no_blockers" => "none",
        _ => "medium",
    }
    .to_string()
}

fn render_evidence(
    session: &SessionProjection,
    run: &RunProjection,
    evidence: &[EvidenceProjection],
    tool_calls: &[ToolCallProjection],
    tool_observations: &[ToolObservationProjection],
    memory_packets: &[MemoryPacketProjection],
    events: &[EventRecord],
) -> String {
    let mut markdown = format!(
        "<!-- capo:evidence-export -->\n# Capo Evidence - {}\n\n## Objective\n\n{}\n\n## State Refs\n\n- Project: `{}`\n- Task: `{}`\n- Session: `{}`\n- Session status: `{}`\n- Run: `{}`\n- Run status: `{}`\n- Agent: `{}`\n- Latest summary: {}\n- Confidence: `{}`\n- Blocker: {}\n\n## Evidence Refs\n\n",
        session.title,
        session.current_goal,
        session.project_id,
        session
            .task_id
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_else(|| "none".to_string()),
        session.session_id,
        session.status,
        run.run_id,
        run.status,
        session.agent_id,
        session.latest_summary.as_deref().unwrap_or("none"),
        session
            .latest_confidence
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_string()),
        session.latest_blocker.as_deref().unwrap_or("none")
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
        for tool_call in tool_calls {
            markdown.push_str(&format!(
                "- `{}` name=`{}` origin=`{}` status=`{}` input_artifact=`{}` output_artifact=`{}`\n",
                tool_call.tool_call_id,
                tool_call.tool_name,
                tool_call.tool_origin,
                tool_call.status,
                tool_call.input_artifact_id.as_deref().unwrap_or("none"),
                tool_call.output_artifact_id.as_deref().unwrap_or("none")
            ));
        }
    }
    markdown.push_str("\n## Tool Observations\n\n");
    if tool_observations.is_empty() {
        markdown.push_str("- none\n");
    } else {
        for observation in tool_observations {
            markdown.push_str(&format!(
                "- `{}` name=`{}` source=`{}` observed_status=`{}` instrumentation=`{}` confidence=`{}` external_ref=`{}` artifact=`{}` raw_event_hash=`{}`\n",
                observation.tool_observation_id,
                observation.tool_name,
                observation.source,
                observation.observed_status,
                observation.instrumentation_level,
                observation.confidence,
                observation.external_tool_ref.as_deref().unwrap_or("none"),
                observation.artifact_id.as_deref().unwrap_or("none"),
                observation.raw_event_hash
            ));
        }
    }
    markdown.push_str("\n## Memory Packets\n\n");
    if memory_packets.is_empty() {
        markdown.push_str("- none\n");
    } else {
        for packet in memory_packets {
            markdown.push_str(&format!(
                "- `{}` purpose=`{}` run=`{}` turn=`{}` artifact=`{}`\n",
                packet.memory_packet_id,
                packet.purpose,
                packet
                    .run_id
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_else(|| "none".to_string()),
                packet.turn_id.as_deref().unwrap_or("none"),
                packet.packet_artifact_id.as_deref().unwrap_or("none")
            ));
        }
    }
    markdown.push_str("\n## Recent Events\n\n");
    for event in events {
        markdown.push_str(&format!(
            "- `{}` `{}` id=`{}` turn=`{}` item=`{}`\n",
            event.sequence,
            event.kind,
            event.event_id,
            event.turn_id.as_deref().unwrap_or("none"),
            event.item_id.as_deref().unwrap_or("none")
        ));
    }
    markdown
}

#[allow(clippy::too_many_arguments)]
fn render_review_finding_artifact(
    session: &SessionProjection,
    run: Option<&RunProjection>,
    review_finding_id: &str,
    artifact_id: &str,
    reviewer: &str,
    finding_kind: &str,
    severity: &str,
    summary: &str,
    tool_call_id: Option<&ToolCallId>,
    follow_up_workpad_task_id: Option<&str>,
) -> String {
    format!(
        "<!-- capo:review-finding -->\n# Capo Review Finding - {}\n\n## Review\n\n- Review finding: `{}`\n- Reviewer: `{}`\n- Kind: `{}`\n- Severity: `{}`\n- Status: `{}`\n- Artifact: `{}`\n\n## Links\n\n- Project: `{}`\n- Task: `{}`\n- Session: `{}`\n- Run: `{}`\n- Tool call: `{}`\n- Follow-up source task: `{}`\n- Compatibility workpad task: `{}`\n\n## Summary\n\n{}\n",
        session.title,
        review_finding_id,
        reviewer,
        finding_kind,
        severity,
        review_finding_status(finding_kind),
        artifact_id,
        session.project_id,
        session
            .task_id
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_else(|| "none".to_string()),
        session.session_id,
        run.map(|run| run.run_id.to_string())
            .unwrap_or_else(|| "none".to_string()),
        tool_call_id
            .map(ToString::to_string)
            .unwrap_or_else(|| "none".to_string()),
        follow_up_workpad_task_id.unwrap_or("none"),
        follow_up_workpad_task_id.unwrap_or("none"),
        summary
    )
}

fn aliased_optional_arg(
    args: &[String],
    preferred_key: &str,
    compatibility_key: &str,
) -> Result<Option<String>, String> {
    let preferred = arg_value(args, preferred_key)?;
    let compatibility = arg_value(args, compatibility_key)?;
    if preferred.is_some() && compatibility.is_some() {
        return Err(format!(
            "{preferred_key} and {compatibility_key} are aliases; provide only one"
        ));
    }
    Ok(preferred.or(compatibility))
}

fn arg_value(args: &[String], key: &str) -> Result<Option<String>, String> {
    if !has_flag(args, key) {
        return Ok(None);
    }
    args.windows(2)
        .find_map(|window| {
            if window[0] == key && !window[1].starts_with("--") {
                Some(window[1].clone())
            } else {
                None
            }
        })
        .map(Some)
        .ok_or_else(|| format!("{key} requires a value"))
}

fn write_evidence_file(path: &Path, markdown: &str) -> Result<(), String> {
    if let Ok(existing) = fs::read_to_string(path)
        && !existing.starts_with("<!-- capo:evidence-export -->")
    {
        return Err(format!(
            "refusing to overwrite non-Capo evidence file: {}",
            path.display()
        ));
    }
    fs::write(path, markdown).map_err(|error| error.to_string())
}

fn write_task_outcome_report_file(path: &Path, markdown: &str) -> Result<(), String> {
    if let Ok(existing) = fs::read_to_string(path) {
        if !existing.starts_with("<!-- capo:task-outcome-report -->") {
            return Err(format!(
                "refusing to overwrite non-Capo task outcome report file: {}",
                path.display()
            ));
        }
        if existing != markdown {
            return Err(format!(
                "refusing to overwrite changed Capo task outcome report file: {}",
                path.display()
            ));
        }
    }
    fs::write(path, markdown).map_err(|error| error.to_string())
}

fn write_review_finding_file(path: &Path, markdown: &str) -> Result<(), String> {
    if let Ok(existing) = fs::read_to_string(path) {
        if !existing.starts_with("<!-- capo:review-finding -->") {
            return Err(format!(
                "refusing to overwrite non-Capo review finding file: {}",
                path.display()
            ));
        }
        if existing != markdown {
            return Err(format!(
                "refusing to overwrite changed Capo review finding file: {}",
                path.display()
            ));
        }
    }
    fs::write(path, markdown).map_err(|error| error.to_string())
}
