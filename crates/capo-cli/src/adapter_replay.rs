use std::fs;
use std::path::PathBuf;

use capo_adapters::{
    AcpAdapter, AdapterFixtureParse, ClaudeCodeAdapter, CodexExecAdapter, NormalizedAdapterEvent,
};
use capo_query::{ProjectDashboardQuery, project_dashboard};
use capo_state::{
    AdapterDispatchReplayProjection, EventKind, NewEvent, ProjectionRecord, RedactionState,
};

use crate::cli_surface::{ParsedArgs, optional_arg, required_arg};
use crate::{
    controller, debug_error, escape_json, export_evidence, project_id, stable_cli_hash, state,
};

pub(crate) fn replay_adapter_fixture(
    parsed: &ParsedArgs,
    args: &[String],
) -> Result<String, String> {
    let adapter = required_arg(args, "--adapter")?;
    let fixture_path = PathBuf::from(required_arg(args, "--fixture")?);
    let agent = required_arg(args, "--agent")?;
    let goal = required_arg(args, "--goal")?;
    let fixture = fs::read_to_string(&fixture_path).map_err(|error| error.to_string())?;
    let adapter_events = parse_adapter_fixture(&adapter, &fixture)?;
    let controller = controller(parsed)?;
    let registration = if state(parsed)?
        .agent_by_name(&agent)
        .map_err(debug_error)?
        .is_some()
    {
        controller
            .registration_for_agent_name(&agent)
            .map_err(debug_error)?
    } else {
        controller.register_agent(&agent).map_err(debug_error)?
    };
    let refs = controller
        .send_task(&registration, &goal)
        .map_err(debug_error)?;
    let report = controller
        .apply_normalized_adapter_events(&refs, &adapter_events)
        .map_err(debug_error)?;
    let mut output = format!(
        "adapter_replayed=true\nadapter={}\nfixture={}\nagent={}\nsession_id={}\nrun_id={}\ninput_events={}\nappended_events={}\ntool_events={}\nsummary_events={}\ncompleted_turns={}\n",
        adapter_label(&adapter),
        fixture_path.display(),
        agent,
        refs.session_id,
        refs.run_id,
        report.input_event_count,
        report.appended_event_count,
        report.tool_event_count,
        report.summary_event_count,
        report.completed_turn_count
    );
    if let Some(out) = optional_arg(args, "--out") {
        output.push_str(&export_evidence(
            parsed,
            &[
                "--session".to_string(),
                refs.session_id.to_string(),
                "--out".to_string(),
                out,
            ],
        )?);
    }
    Ok(output)
}

pub(crate) fn replay_adapter_dispatch_fixture(
    parsed: &ParsedArgs,
    args: &[String],
) -> Result<String, String> {
    let dispatch_plan_id = required_arg(args, "--dispatch-plan")?;
    let fixture_path = PathBuf::from(required_arg(args, "--fixture")?);
    if let Some(unknown) = args.iter().find(|arg| {
        arg.starts_with("--") && !matches!(arg.as_str(), "--dispatch-plan" | "--fixture" | "--out")
    }) {
        return Err(format!("unknown adapter replay-dispatch option: {unknown}"));
    }
    let state = state(parsed)?;
    let dashboard =
        project_dashboard(&state, ProjectDashboardQuery::new(project_id())).map_err(debug_error)?;
    let plan = dashboard
        .adapter_dispatch_plans
        .iter()
        .find(|plan| plan.dispatch_plan_id == dispatch_plan_id)
        .ok_or_else(|| format!("unknown adapter dispatch plan: {dispatch_plan_id}"))?;
    let ready_gate = dashboard.adapter_dispatch_gates.iter().rev().find(|gate| {
        gate.dispatch_plan_id == plan.dispatch_plan_id
            && gate.provider_cli_execution_allowed
            && gate.status == "ready_for_execution"
            && !gate.provider_cli_executed
            && gate.runtime_prompt_policy == "not_rendered"
    });
    let ready_gate = ready_gate.ok_or_else(|| {
        format!(
            "dispatch plan {} has no recorded ready dispatch gate; run adapter dispatch-gate --record after clean smoke evidence",
            plan.dispatch_plan_id
        )
    })?;
    let fixture = fs::read_to_string(&fixture_path).map_err(|error| error.to_string())?;
    let adapter_events = parse_adapter_fixture(&plan.adapter_kind, &fixture)?;
    let controller = controller(parsed)?;
    let registration = controller
        .registration_for_agent_name(&plan.agent_name)
        .map_err(debug_error)?;
    let replay_goal = format!(
        "Replay fixture for dispatch plan {} without provider execution",
        plan.dispatch_plan_id
    );
    let refs = controller
        .send_task(&registration, &replay_goal)
        .map_err(debug_error)?;
    if refs.session_id != plan.session_id || refs.run_id != plan.run_id {
        return Err(format!(
            "dispatch replay ref mismatch for {}: expected session={} run={}, got session={} run={}",
            plan.dispatch_plan_id, plan.session_id, plan.run_id, refs.session_id, refs.run_id
        ));
    }
    let report = controller
        .apply_normalized_adapter_events(&refs, &adapter_events)
        .map_err(debug_error)?;
    let replay = AdapterDispatchReplayProjection {
        dispatch_replay_id: format!(
            "adapter-dispatch-replay-{}",
            stable_cli_hash(&format!(
                "{}:{}:{}",
                plan.dispatch_plan_id,
                ready_gate.dispatch_gate_id,
                stable_cli_hash(&fixture)
            ))
        ),
        project_id: project_id(),
        dispatch_plan_id: plan.dispatch_plan_id.clone(),
        dispatch_gate_id: ready_gate.dispatch_gate_id.clone(),
        adapter_kind: plan.adapter_kind.clone(),
        session_id: refs.session_id.clone(),
        run_id: refs.run_id.clone(),
        fixture_path: fixture_path.display().to_string(),
        fixture_hash: stable_cli_hash(&fixture),
        input_event_count: report.input_event_count as i64,
        appended_event_count: report.appended_event_count as i64,
        tool_event_count: report.tool_event_count as i64,
        summary_event_count: report.summary_event_count as i64,
        completed_turn_count: report.completed_turn_count as i64,
        provider_cli_executed: false,
        raw_content_policy: "content_hashed_not_rendered".to_string(),
        updated_sequence: 0,
    };
    let replay_sequence = state
        .append_event(
            NewEvent {
                event_id: format!(
                    "event-adapter-dispatch-replay-{}",
                    stable_cli_hash(&replay.dispatch_replay_id)
                ),
                kind: EventKind::AdapterDispatchReplayed,
                actor: "local-cli".to_string(),
                project_id: Some(replay.project_id.clone()),
                task_id: Some(refs.task_id.clone()),
                agent_id: Some(refs.agent_id.clone()),
                session_id: Some(refs.session_id.clone()),
                run_id: Some(refs.run_id.clone()),
                turn_id: None,
                item_id: Some(replay.dispatch_replay_id.clone()),
                payload_json: format!(
                    "{{\"dispatch_plan_id\":\"{}\",\"dispatch_gate_id\":\"{}\",\"fixture_hash\":\"{}\",\"provider_cli_executed\":false,\"raw_content_policy\":\"content_hashed_not_rendered\"}}",
                    escape_json(&replay.dispatch_plan_id),
                    escape_json(&replay.dispatch_gate_id),
                    replay.fixture_hash
                ),
                idempotency_key: Some(format!(
                    "adapter-dispatch-replay:{}:{}:{}",
                    replay.project_id, replay.dispatch_plan_id, replay.fixture_hash
                )),
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::AdapterDispatchReplay(replay.clone())],
        )
        .map_err(debug_error)?;
    let mut output = format!(
        "adapter_dispatch_replayed=true\ndispatch_replay={}\ndispatch_plan={}\ndispatch_gate={}\nadapter={}\nfixture={}\nfixture_hash={}\nagent={}\nsession_id={}\nrun_id={}\nprovider_cli_executed=false\nraw_content_policy={}\ninput_events={}\nappended_events={}\ntool_events={}\nsummary_events={}\ncompleted_turns={}\nrecorded_sequence={}\n",
        replay.dispatch_replay_id,
        plan.dispatch_plan_id,
        ready_gate.dispatch_gate_id,
        plan.adapter_kind,
        fixture_path.display(),
        replay.fixture_hash,
        plan.agent_name,
        refs.session_id,
        refs.run_id,
        replay.raw_content_policy,
        report.input_event_count,
        report.appended_event_count,
        report.tool_event_count,
        report.summary_event_count,
        report.completed_turn_count,
        replay_sequence
    );
    if let Some(out) = optional_arg(args, "--out") {
        output.push_str(&export_evidence(
            parsed,
            &[
                "--session".to_string(),
                refs.session_id.to_string(),
                "--out".to_string(),
                out,
            ],
        )?);
    }
    Ok(output)
}

fn parse_adapter_fixture(
    adapter: &str,
    fixture: &str,
) -> Result<Vec<NormalizedAdapterEvent>, String> {
    let parsed: AdapterFixtureParse = match adapter {
        "codex" | "codex-exec" | "codex_exec" => {
            CodexExecAdapter::parse_jsonl(fixture).map_err(adapter_parse_error)?
        }
        "claude" | "claude-code" | "claude_code" => {
            ClaudeCodeAdapter::parse_stream_json(fixture).map_err(adapter_parse_error)?
        }
        "acp" => AcpAdapter::parse_replay_jsonl(fixture).map_err(adapter_parse_error)?,
        other => {
            return Err(format!(
                "unsupported adapter fixture kind: {other}; expected codex, claude, or acp"
            ));
        }
    };
    Ok(parsed.deduped_by_idempotency())
}

fn adapter_parse_error(error: capo_adapters::AdapterParseError) -> String {
    format!(
        "adapter fixture parse failed at line {}: {}",
        error.line, error.message
    )
}

pub(crate) fn adapter_label(adapter: &str) -> &'static str {
    match adapter {
        "codex" | "codex-exec" | "codex_exec" => "codex_exec",
        "claude" | "claude-code" | "claude_code" => "claude_code",
        "acp" => "acp",
        _ => "unknown",
    }
}
