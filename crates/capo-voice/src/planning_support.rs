use super::*;

pub(super) fn voice_command(
    slug: &str,
    input: &VoiceTranscriptInput,
    target: CommandTarget,
    intent: CommandIntent,
    text: Option<String>,
) -> CommandEnvelope {
    let mut command = CommandEnvelope::new(
        CommandId::new(format!("cmd-{slug}-{}", input.voice_session_id)),
        InputOrigin::Voice,
        input.actor_id.clone(),
        input.project_id.clone(),
        target,
        intent,
    );
    if let Some(text) = text {
        command = command.with_text(text);
    }
    command.structured_args.push((
        "voice_session_id".to_string(),
        input.voice_session_id.clone(),
    ));
    command.structured_args.push((
        "transcript_retention".to_string(),
        "raw_not_retained".to_string(),
    ));
    command
}

pub(super) fn transcript_policy(
    retention_policy: TranscriptRetentionPolicy,
) -> VoiceTranscriptPolicy {
    match retention_policy {
        TranscriptRetentionPolicy::DoNotRetainRaw => VoiceTranscriptPolicy {
            retain_raw_transcript: false,
            redaction_required: true,
            memory_ingestion: MemoryIngestionPolicy::None,
            audit_note: "store normalized command plus voice-derived marker; do not retain raw transcript",
        },
        TranscriptRetentionPolicy::RetainRedactedSummary => VoiceTranscriptPolicy {
            retain_raw_transcript: false,
            redaction_required: true,
            memory_ingestion: MemoryIngestionPolicy::ReviewedRedactedSummaryOnly,
            audit_note: "store reviewed redacted summary only; raw transcript remains transient",
        },
    }
}

pub(super) fn status_agent(normalized: &str) -> Option<String> {
    normalized
        .strip_prefix("what is ")
        .and_then(|rest| rest.strip_suffix(" doing"))
        .or_else(|| normalized.strip_prefix("status for "))
        .map(agent_slug)
}

pub(super) fn recent_work_agent(normalized: &str) -> Option<String> {
    normalized
        .strip_prefix("what has ")
        .and_then(|rest| rest.strip_suffix(" done"))
        .or_else(|| {
            normalized
                .strip_prefix("what did ")
                .and_then(|rest| rest.strip_suffix(" do"))
        })
        .map(agent_slug)
}

pub(super) fn start_next_work_agent(normalized: &str) -> Option<String> {
    normalized
        .strip_prefix("start next task with ")
        .or_else(|| normalized.strip_prefix("start the next task with "))
        .or_else(|| normalized.strip_prefix("start next work with "))
        .or_else(|| {
            normalized
                .strip_prefix("have ")
                .and_then(|rest| rest.strip_suffix(" start next task"))
        })
        .map(agent_slug)
}

pub(super) fn redirect_agent(normalized: &str) -> Option<(String, String)> {
    let rest = normalized.strip_prefix("steer ")?;
    let (agent, goal) = rest.split_once(" to ")?;
    Some((agent_slug(agent), goal.trim().to_string()))
}

pub(super) fn stop_agent(normalized: &str) -> Option<(String, String)> {
    let rest = normalized.strip_prefix("stop ")?;
    let (agent, reason) = rest
        .split_once(" because ")
        .map(|(agent, reason)| (agent, reason.to_string()))
        .unwrap_or((rest, "voice stop requested".to_string()));
    Some((agent_slug(agent), reason.trim().to_string()))
}

pub(super) fn interrupt_agent(normalized: &str) -> Option<(String, String)> {
    let rest = normalized.strip_prefix("interrupt ")?;
    let (agent, reason) = rest
        .split_once(" because ")
        .map(|(agent, reason)| (agent, reason.to_string()))
        .unwrap_or((rest, "voice interrupt requested".to_string()));
    Some((agent_slug(agent), reason.trim().to_string()))
}

pub(super) fn is_dogfood_readiness_question(input: &str) -> bool {
    matches!(
        input,
        "are we ready to dogfood"
            | "are we ready for dogfood"
            | "can we dogfood capo"
            | "can capo dogfood itself"
            | "is capo ready to dogfood"
            | "what is dogfood readiness"
    )
}

pub(super) fn is_next_work_question(input: &str) -> bool {
    matches!(
        input,
        "what should we do next"
            | "what is next"
            | "what's next"
            | "what is the next task"
            | "what should capo do next"
            | "show next work"
    )
}

pub(super) fn dispatch_status_plan(normalized: &str) -> Option<String> {
    normalized
        .strip_prefix("what is dispatch status for ")
        .or_else(|| normalized.strip_prefix("what's dispatch status for "))
        .or_else(|| normalized.strip_prefix("show dispatch status for "))
        .or_else(|| normalized.strip_prefix("dispatch status for "))
        .map(str::trim)
        .filter(|plan| !plan.is_empty())
        .map(ToString::to_string)
}

pub(super) fn latest_dispatch_status_agent(normalized: &str) -> Option<Option<String>> {
    if matches!(
        normalized,
        "what is the latest dispatch status"
            | "what's the latest dispatch status"
            | "show latest dispatch status"
            | "latest dispatch status"
    ) {
        return Some(None);
    }

    normalized
        .strip_prefix("what is the latest dispatch status for ")
        .or_else(|| normalized.strip_prefix("what's the latest dispatch status for "))
        .or_else(|| normalized.strip_prefix("show latest dispatch status for "))
        .or_else(|| normalized.strip_prefix("latest dispatch status for "))
        .map(agent_slug)
        .filter(|agent_name| !agent_name.is_empty())
        .map(Some)
}

pub(super) fn adapter_smoke_report_status_plan(normalized: &str) -> Option<String> {
    normalized
        .strip_prefix("what is adapter smoke report status for ")
        .or_else(|| normalized.strip_prefix("what's adapter smoke report status for "))
        .or_else(|| normalized.strip_prefix("show adapter smoke report status for "))
        .or_else(|| normalized.strip_prefix("adapter smoke report status for "))
        .or_else(|| normalized.strip_prefix("what is smoke report status for "))
        .or_else(|| normalized.strip_prefix("what's smoke report status for "))
        .or_else(|| normalized.strip_prefix("show smoke report status for "))
        .or_else(|| normalized.strip_prefix("smoke report status for "))
        .map(str::trim)
        .filter(|report| !report.is_empty())
        .map(ToString::to_string)
}

pub(super) fn latest_adapter_smoke_report_filter(normalized: &str) -> Option<Option<String>> {
    if matches!(
        normalized,
        "what is latest adapter smoke report status"
            | "what is the latest adapter smoke report status"
            | "what's latest adapter smoke report status"
            | "what's the latest adapter smoke report status"
            | "show latest adapter smoke report status"
            | "latest adapter smoke report status"
            | "what is latest smoke report status"
            | "what is the latest smoke report status"
            | "what's latest smoke report status"
            | "what's the latest smoke report status"
            | "show latest smoke report status"
            | "latest smoke report status"
    ) {
        return Some(None);
    }

    normalized
        .strip_prefix("what is latest adapter smoke report status for ")
        .or_else(|| normalized.strip_prefix("what is the latest adapter smoke report status for "))
        .or_else(|| normalized.strip_prefix("what's latest adapter smoke report status for "))
        .or_else(|| normalized.strip_prefix("what's the latest adapter smoke report status for "))
        .or_else(|| normalized.strip_prefix("show latest adapter smoke report status for "))
        .or_else(|| normalized.strip_prefix("latest adapter smoke report status for "))
        .or_else(|| normalized.strip_prefix("what is latest smoke report status for "))
        .or_else(|| normalized.strip_prefix("what is the latest smoke report status for "))
        .or_else(|| normalized.strip_prefix("what's latest smoke report status for "))
        .or_else(|| normalized.strip_prefix("what's the latest smoke report status for "))
        .or_else(|| normalized.strip_prefix("show latest smoke report status for "))
        .or_else(|| normalized.strip_prefix("latest smoke report status for "))
        .and_then(adapter_kind_slug)
        .map(Some)
}

pub(super) fn adapter_kind_slug(value: &str) -> Option<String> {
    match agent_slug(value).as_str() {
        "codex" | "codex-exec" | "codex-exec-adapter" => Some("codex_exec".to_string()),
        "claude" | "claude-code" | "claude-code-adapter" => Some("claude_code".to_string()),
        _ => None,
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ConnectivityExposureVoiceFilter {
    pub owner_kind: Option<String>,
    pub owner_id: Option<String>,
    pub channel_kind: Option<String>,
}

pub(super) fn latest_connectivity_exposure_filter(
    normalized: &str,
) -> Option<ConnectivityExposureVoiceFilter> {
    if matches!(
        normalized,
        "what is latest connectivity exposure status"
            | "what is the latest connectivity exposure status"
            | "what's latest connectivity exposure status"
            | "what's the latest connectivity exposure status"
            | "show latest connectivity exposure status"
            | "latest connectivity exposure status"
            | "what is latest remote control exposure"
            | "what is the latest remote control exposure"
            | "show latest remote control exposure"
            | "latest remote control exposure"
    ) {
        return Some(ConnectivityExposureVoiceFilter::default());
    }

    let rest = normalized
        .strip_prefix("what is latest connectivity exposure status for ")
        .or_else(|| normalized.strip_prefix("what is the latest connectivity exposure status for "))
        .or_else(|| normalized.strip_prefix("what's latest connectivity exposure status for "))
        .or_else(|| normalized.strip_prefix("what's the latest connectivity exposure status for "))
        .or_else(|| normalized.strip_prefix("show latest connectivity exposure status for "))
        .or_else(|| normalized.strip_prefix("latest connectivity exposure status for "))
        .or_else(|| normalized.strip_prefix("what is latest remote control exposure for "))
        .or_else(|| normalized.strip_prefix("what is the latest remote control exposure for "))
        .or_else(|| normalized.strip_prefix("show latest remote control exposure for "))
        .or_else(|| normalized.strip_prefix("latest remote control exposure for "))?
        .trim();

    if let Some(owner_id) = rest.strip_prefix("runtime target ") {
        return Some(ConnectivityExposureVoiceFilter {
            owner_kind: Some("runtime_target".to_string()),
            owner_id: Some(agent_slug(owner_id)),
            channel_kind: None,
        });
    }
    if let Some(owner_id) = rest.strip_prefix("capo server ") {
        return Some(ConnectivityExposureVoiceFilter {
            owner_kind: Some("capo_server".to_string()),
            owner_id: Some(agent_slug(owner_id)),
            channel_kind: None,
        });
    }
    if matches!(
        rest,
        "control" | "stdio" | "logs" | "dashboard" | "artifact"
    ) {
        return Some(ConnectivityExposureVoiceFilter {
            owner_kind: None,
            owner_id: None,
            channel_kind: Some(rest.to_string()),
        });
    }

    None
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RuntimeTargetVoiceFilter {
    pub runner_kind: Option<String>,
    pub status: Option<String>,
}

pub(super) fn latest_runtime_target_status_filter(
    normalized: &str,
) -> Option<RuntimeTargetVoiceFilter> {
    if matches!(
        normalized,
        "what is latest runtime target status"
            | "what is the latest runtime target status"
            | "what's latest runtime target status"
            | "what's the latest runtime target status"
            | "show latest runtime target status"
            | "latest runtime target status"
    ) {
        return Some(RuntimeTargetVoiceFilter::default());
    }

    let rest = normalized
        .strip_prefix("what is latest runtime target status for ")
        .or_else(|| normalized.strip_prefix("what is the latest runtime target status for "))
        .or_else(|| normalized.strip_prefix("what's latest runtime target status for "))
        .or_else(|| normalized.strip_prefix("what's the latest runtime target status for "))
        .or_else(|| normalized.strip_prefix("show latest runtime target status for "))
        .or_else(|| normalized.strip_prefix("latest runtime target status for "))?
        .trim();

    runtime_target_voice_filter(rest)
}

pub(super) fn runtime_target_voice_filter(value: &str) -> Option<RuntimeTargetVoiceFilter> {
    let slug = agent_slug(value);
    if let Some(status) = runtime_target_status_slug(&slug) {
        return Some(RuntimeTargetVoiceFilter {
            runner_kind: None,
            status: Some(status.to_string()),
        });
    }
    if let Some(runner_kind) = runtime_runner_kind_slug(&slug) {
        return Some(RuntimeTargetVoiceFilter {
            runner_kind: Some(runner_kind.to_string()),
            status: None,
        });
    }

    let (status_part, runner_part) = slug.split_once('-')?;
    let status = runtime_target_status_slug(status_part)?;
    let runner_kind = runtime_runner_kind_slug(runner_part)?;
    Some(RuntimeTargetVoiceFilter {
        runner_kind: Some(runner_kind.to_string()),
        status: Some(status.to_string()),
    })
}

pub(super) fn runtime_target_status_slug(value: &str) -> Option<&'static str> {
    match value {
        "available" => Some("available"),
        "disabled" => Some("disabled"),
        "unhealthy" => Some("unhealthy"),
        _ => None,
    }
}

pub(super) fn runtime_runner_kind_slug(value: &str) -> Option<&'static str> {
    match value {
        "local" | "local-process" => Some("local-process"),
        "remote" | "remote-process" => Some("remote-process"),
        "container" => Some("container"),
        _ => None,
    }
}

pub(super) fn runtime_target_status_id(normalized: &str) -> Option<String> {
    normalized
        .strip_prefix("what is the runtime target status for ")
        .or_else(|| normalized.strip_prefix("what's the runtime target status for "))
        .or_else(|| normalized.strip_prefix("show runtime target status for "))
        .or_else(|| normalized.strip_prefix("runtime target status for "))
        .or_else(|| normalized.strip_prefix("what is the status of runtime target "))
        .or_else(|| normalized.strip_prefix("what's the status of runtime target "))
        .map(agent_slug)
        .filter(|runtime_target_id| !runtime_target_id.is_empty())
}

pub(super) fn runtime_target_readiness_id(normalized: &str) -> Option<String> {
    normalized
        .strip_prefix("is runtime target ")
        .and_then(|rest| rest.strip_suffix(" ready for remote control"))
        .or_else(|| {
            normalized
                .strip_prefix("is runtime target ")
                .and_then(|rest| rest.strip_suffix(" control ready"))
        })
        .or_else(|| normalized.strip_prefix("what is runtime target readiness for "))
        .or_else(|| normalized.strip_prefix("what is the runtime target readiness for "))
        .or_else(|| normalized.strip_prefix("show runtime target readiness for "))
        .or_else(|| normalized.strip_prefix("runtime target readiness for "))
        .map(agent_slug)
        .filter(|runtime_target_id| !runtime_target_id.is_empty())
}

pub(super) fn is_project_recent_work_question(input: &str) -> bool {
    matches!(
        input,
        "what have my agents done"
            | "what have the agents done"
            | "what did my agents do"
            | "what did the agents do"
            | "what has the team done"
            | "summarize agent work"
    )
}

pub(super) fn is_project_tool_activity_question(input: &str) -> bool {
    matches!(
        input,
        "what tools have my agents used"
            | "what tools have the agents used"
            | "what tools did my agents use"
            | "what tools did the agents use"
            | "show agent tool activity"
            | "show tool activity"
    )
}

pub(super) fn tool_activity_agent(normalized: &str) -> Option<String> {
    normalized
        .strip_prefix("what tools has ")
        .and_then(|rest| rest.strip_suffix(" used"))
        .or_else(|| {
            normalized
                .strip_prefix("what tools did ")
                .and_then(|rest| rest.strip_suffix(" use"))
        })
        .or_else(|| normalized.strip_prefix("show tool activity for "))
        .map(agent_slug)
}

pub(super) fn is_review_needs_question(input: &str) -> bool {
    matches!(
        input,
        "what needs review"
            | "what are the review blockers"
            | "show review blockers"
            | "what outcomes need attention"
            | "what needs attention"
            | "summarize review blockers"
    )
}

pub(super) fn normalize(input: &str) -> String {
    input
        .trim()
        .trim_end_matches(['.', '?', '!'])
        .to_ascii_lowercase()
}

pub(super) fn agent_slug(input: &str) -> String {
    input
        .trim()
        .trim_start_matches("agent ")
        .chars()
        .filter_map(|ch| {
            if ch.is_ascii_alphanumeric() {
                Some(ch.to_ascii_lowercase())
            } else if ch == '-' || ch == '_' || ch.is_ascii_whitespace() {
                Some('-')
            } else {
                None
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}
