use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use capo_adapters::{
    AcpAdapter, AdapterFixtureParse, ClaudeCodeAdapter, CodexExecAdapter, LocalAdapterLaunchPlan,
    LocalAdapterSmokeError, LocalAdapterSmokePlan, NormalizedAdapterEvent,
    scan_artifacts_for_sensitive_markers,
};
use capo_controller::FakeBoundaryController;
use capo_core::{
    AgentId, CommandEnvelope, CommandId, CommandIntent, CommandTarget, InputOrigin, ProjectId,
    RunId, SessionId, TaskId, ToolCallId,
};
use capo_eval::TaskOutcomeReport;
use capo_query::{
    AdapterDispatchStatus, AdapterDogfoodGate, ProjectDashboard, ProjectDashboardQuery,
    ProjectDogfoodReadiness, project_dashboard, project_dogfood_readiness,
};
use capo_runtime::{
    ChannelKind, ConnectivityEndpointConfig, ConnectivityTunnel, EndpointOwner, ExposureScope,
    LocalProcessRunner,
};
use capo_state::{
    AdapterDispatchExecutionProjection, AdapterDispatchExecutionRequestProjection,
    AdapterDispatchGateProjection, AdapterDispatchPlanProjection,
    AdapterDispatchPromptMaterializationProjection, AdapterDispatchPromptSourceProjection,
    AdapterDispatchReplayProjection, AdapterReadinessProjection, AdapterSmokeReportProjection,
    ArtifactRecord, CapabilityGrantProjection, ConnectivityExposureProjection, EventKind,
    EventRecord, EvidenceProjection, MemoryPacketProjection, MemoryRecordProjection,
    MemorySourceProjection, NewEvent, PermissionApprovalProjection, ProjectionRecord,
    RedactionState, ReviewFindingProjection, RunProjection, SessionProjection, SqliteStateStore,
    ToolCallProjection, WorkpadFileProjection, WorkpadIndexResetProjection, WorkpadTaskProjection,
};
use capo_voice::{
    MemoryIngestionPolicy, TranscriptRetentionPolicy, VOICE_TRANSCRIPT_RETENTION_DEFAULT,
    VoiceCommandPlan, VoiceIntentKind, VoiceReadScope, VoiceTranscriptInput, plan_dummy_transcript,
};
use capo_workpads::{WorkpadIndex, index_project_workpads};

const DEFAULT_STATE_ROOT: &str = ".capo-dev";
const DEFAULT_PROJECT_ID: &str = "project-capo";

const HELP: &str = "\
Capo - local controller for coding-agent sessions

Usage:
  capo --help
  capo version
  capo init [--state PATH]
  capo dashboard [--project PROJECT_ID] [--session SESSION_ID] [--status STATUS] [--workpad-path PATH] [--workpad-status STATUS] [--state PATH]
  capo agent register --name NAME --adapter fake --runtime fake [--state PATH]
  capo agent spawn --name NAME --adapter fake --runtime fake [--state PATH]
  capo agent list [--state PATH]
  capo adapter readiness [--record] [--state PATH]
  capo adapter plan-launch --adapter codex|claude --agent NAME --goal GOAL [--workspace PATH] [--artifacts PATH] [--record] [--state PATH]
  capo adapter dispatch-gate --dispatch-plan DISPATCH_PLAN_ID [--record] [--state PATH]
  capo adapter dispatch-status --dispatch-plan DISPATCH_PLAN_ID [--state PATH]
  capo adapter dispatch-status --latest [--agent NAME] [--state PATH]
  capo adapter dispatch-evidence --dispatch-plan DISPATCH_PLAN_ID --out DIR [--state PATH]
  capo adapter dispatch-evidence --latest [--agent NAME] --out DIR [--state PATH]
  capo adapter execution-request --dispatch-plan DISPATCH_PLAN_ID [--record] [--state PATH]
  capo adapter materialize-prompt --dispatch-plan DISPATCH_PLAN_ID [--record] [--state PATH]
  capo adapter run-preflight --dispatch-plan DISPATCH_PLAN_ID [--state PATH]
  capo adapter run-local --dispatch-plan DISPATCH_PLAN_ID [--record] [--state PATH]
  capo adapter dogfood-gate [--state PATH]
  capo adapter smoke-report scan --artifact-root PATH [--state PATH]
  capo adapter smoke-report record --adapter codex|claude --status skipped|passed|failed --credential-scan clean|blocked|not_run --reason TEXT [--marker-found] [--artifact-root PATH] [--state PATH]
  capo adapter replay-fixture --adapter codex|claude|acp --fixture PATH --agent NAME --goal GOAL [--out DIR] [--state PATH]
  capo adapter replay-dispatch --dispatch-plan DISPATCH_PLAN_ID --fixture PATH [--out DIR] [--state PATH]
  capo dogfood readiness [--out DIR] [--state PATH]
  capo task send --agent NAME --goal GOAL [--scenario NAME] [--state PATH]
  capo session status --agent NAME [--state PATH]
  capo session redirect --agent NAME --goal GOAL [--state PATH]
  capo session interrupt --agent NAME --reason REASON [--state PATH]
  capo session stop --agent NAME --reason REASON [--state PATH]
  capo voice submit --transcript TEXT [--voice-session SESSION_ID] [--actor ACTOR] [--confirm] [--redacted-summary TEXT --reviewed-summary] [--state PATH]
  capo recover [--state PATH]
  capo permission request --approval APPROVAL_ID --scope-json JSON --reason REASON [--profile PROFILE] [--session SESSION_ID] [--tool-call TOOL_CALL_ID] [--subject-json JSON] [--requested-by ACTOR] [--state PATH]
  capo permission list [--state PATH]
  capo permission decide --approval APPROVAL_ID --decision allow_once|allow_always|reject_once|reject_always [--state PATH]
  capo connectivity expose-stub --endpoint ENDPOINT_ID --owner-kind runtime_target|capo_server --owner-id OWNER_ID --channel control|stdio|logs|dashboard|artifact --exposure loopback|private|public [--address REF] [--record] [--state PATH]
  capo connectivity request-approval --exposure EXPOSURE_ID [--approval APPROVAL_ID] [--state PATH]
  capo connectivity activate-exposure --exposure EXPOSURE_ID [--state PATH]
  capo connectivity revoke-exposure --exposure EXPOSURE_ID [--reason REASON] [--state PATH]
  capo connectivity exposure-status --exposure EXPOSURE_ID [--state PATH]
  capo connectivity exposure-status --latest [--owner-kind runtime_target|capo_server] [--owner-id OWNER_ID] [--channel control|stdio|logs|dashboard|artifact] [--state PATH]
  capo connectivity exposure-evidence --exposure EXPOSURE_ID --out DIR [--state PATH]
  capo workpad index --root PATH [--state PATH]
  capo workpad next [--path PATH] [--state PATH]
  capo workpad plan-next --agent NAME --adapter codex|claude [--path PATH] [--workspace PATH] [--artifacts PATH] [--record] [--state PATH]
  capo workpad start-next --agent NAME [--path PATH] [--state PATH]
  capo workpad import --workpad-task WORKPAD_TASK_ID [--expected-hash HASH] [--task TASK_ID] [--state PATH]
  capo workpad propose --workpad-task WORKPAD_TASK_ID --out DIR [--expected-hash HASH] [--task TASK_ID] [--summary TEXT] [--state PATH]
  capo workpad apply --proposal PATH [--confirm] [--state PATH]
  capo evidence export --session SESSION_ID --out DIR [--state PATH]
  capo eval task-outcome --session SESSION_ID --out DIR [--state PATH]
  capo review record --session SESSION_ID --reviewer NAME --kind blocker|finding|no_blockers --summary TEXT --out DIR [--severity LEVEL] [--tool-call TOOL_CALL_ID] [--follow-up-workpad-task WORKPAD_TASK_ID] [--state PATH]

Prototype notes:
  The P4 CLI uses command envelopes, the fake controller, and SQLite read models.
  It does not read provider credentials or inspect vendor subscription state.
";

fn main() {
    match run_cli(env::args().skip(1).collect()) {
        Ok(output) => print!("{output}"),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    }
}

fn run_cli(raw_args: Vec<String>) -> Result<String, String> {
    let parsed = ParsedArgs::new(raw_args)?;
    let args = parsed.args.as_slice();

    match args {
        [] => Ok(HELP.to_string()),
        [flag] if flag == "--help" || flag == "-h" || flag == "help" => Ok(HELP.to_string()),
        [flag] if flag == "version" || flag == "--version" || flag == "-V" => {
            Ok(format!("capo {}\n", env!("CARGO_PKG_VERSION")))
        }
        [command] if command == "--help" || command == "-h" || command == "help" => {
            Ok(HELP.to_string())
        }
        [command] if command == "init" => init(&parsed),
        [command, rest @ ..] if command == "dashboard" => dashboard(&parsed, rest),
        [area, command, rest @ ..] if area == "agent" && command == "register" => {
            register_agent(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "agent" && command == "spawn" => {
            register_agent(&parsed, rest)
        }
        [area, command] if area == "agent" && command == "list" => list_agents(&parsed),
        [area, command, rest @ ..] if area == "adapter" && command == "replay-fixture" => {
            replay_adapter_fixture(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "adapter" && command == "replay-dispatch" => {
            replay_adapter_dispatch_fixture(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "dogfood" && command == "readiness" => {
            dogfood_readiness(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "adapter" && command == "readiness" => {
            adapter_readiness(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "adapter" && command == "plan-launch" => {
            plan_adapter_launch(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "adapter" && command == "dispatch-gate" => {
            adapter_dispatch_gate(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "adapter" && command == "dispatch-status" => {
            adapter_dispatch_status(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "adapter" && command == "dispatch-evidence" => {
            adapter_dispatch_evidence(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "adapter" && command == "execution-request" => {
            adapter_dispatch_execution_request(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "adapter" && command == "materialize-prompt" => {
            adapter_dispatch_materialize_prompt(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "adapter" && command == "run-preflight" => {
            adapter_dispatch_run_preflight(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "adapter" && command == "run-local" => {
            adapter_dispatch_run_local(&parsed, rest)
        }
        [area, command] if area == "adapter" && command == "dogfood-gate" => {
            adapter_dogfood_gate(&parsed)
        }
        [area, command, action, rest @ ..]
            if area == "adapter" && command == "smoke-report" && action == "scan" =>
        {
            scan_adapter_smoke_artifacts(rest)
        }
        [area, command, action, rest @ ..]
            if area == "adapter" && command == "smoke-report" && action == "record" =>
        {
            record_adapter_smoke_report(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "task" && command == "send" => {
            send_task(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "session" && command == "status" => {
            session_status(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "session" && command == "redirect" => {
            redirect_session(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "session" && command == "interrupt" => {
            interrupt_session(&parsed, rest, "interrupt")
        }
        [area, command, rest @ ..] if area == "session" && command == "stop" => {
            interrupt_session(&parsed, rest, "stop")
        }
        [area, command, rest @ ..] if area == "voice" && command == "submit" => {
            submit_voice(&parsed, rest)
        }
        [command] if command == "recover" => recover(&parsed),
        [area, command, rest @ ..] if area == "permission" && command == "request" => {
            request_permission_approval(&parsed, rest)
        }
        [area, command] if area == "permission" && command == "list" => {
            list_permission_approvals(&parsed)
        }
        [area, command, rest @ ..] if area == "permission" && command == "decide" => {
            decide_permission_approval(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "connectivity" && command == "expose-stub" => {
            expose_connectivity_stub(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "connectivity" && command == "request-approval" => {
            request_connectivity_exposure_approval(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "connectivity" && command == "activate-exposure" => {
            activate_connectivity_exposure(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "connectivity" && command == "revoke-exposure" => {
            revoke_connectivity_exposure(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "connectivity" && command == "exposure-status" => {
            connectivity_exposure_status(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "connectivity" && command == "exposure-evidence" => {
            connectivity_exposure_evidence(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "workpad" && command == "index" => {
            index_workpads(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "workpad" && command == "next" => {
            next_workpad_task(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "workpad" && command == "plan-next" => {
            plan_next_workpad_task(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "workpad" && command == "start-next" => {
            start_next_workpad_task(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "workpad" && command == "import" => {
            import_workpad_task(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "workpad" && command == "propose" => {
            propose_workpad_update(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "workpad" && command == "apply" => {
            apply_workpad_proposal(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "evidence" && command == "export" => {
            export_evidence(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "eval" && command == "task-outcome" => {
            export_task_outcome_report(&parsed, rest)
        }
        [area, command, rest @ ..] if area == "review" && command == "record" => {
            record_review_finding(&parsed, rest)
        }
        [unknown, ..] => Err(format!("unknown command: {unknown}\nrun `capo --help`")),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ParsedArgs {
    state_root: PathBuf,
    args: Vec<String>,
}

impl ParsedArgs {
    fn new(raw_args: Vec<String>) -> Result<Self, String> {
        let mut state_root = PathBuf::from(DEFAULT_STATE_ROOT);
        let mut args = Vec::new();
        let mut iter = raw_args.into_iter();

        while let Some(arg) = iter.next() {
            if arg == "--state" {
                let value = iter
                    .next()
                    .ok_or_else(|| "--state requires a path".to_string())?;
                state_root = PathBuf::from(value);
            } else {
                args.push(arg);
            }
        }

        Ok(Self { state_root, args })
    }
}

fn init(parsed: &ParsedArgs) -> Result<String, String> {
    let command = envelope(
        "init",
        CommandTarget::Project(project_id()),
        CommandIntent::InitializeProject,
        None,
    );
    let initialized = controller(parsed)?
        .initialize(&command)
        .map_err(debug_error)?;
    Ok(format!(
        "initialized=true\nstate={}\ncommand_id={}\n",
        initialized.state_db_path, initialized.command_id
    ))
}

fn register_agent(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    require_fake_arg(args, "--adapter")?;
    require_fake_arg(args, "--runtime")?;
    let name = required_arg(args, "--name")?;
    let spawn = parsed.args.get(1).is_some_and(|command| command == "spawn");
    let command = envelope(
        if spawn {
            "agent-spawn"
        } else {
            "agent-register"
        },
        CommandTarget::Project(project_id()),
        CommandIntent::RegisterAgent,
        Some(name.clone()),
    );
    let controller = controller(parsed)?;
    let registration = if spawn {
        controller
            .spawn_agent_command(&command)
            .map_err(debug_error)?
    } else {
        controller
            .register_agent_command(&command)
            .map_err(debug_error)?
    };
    Ok(format!(
        "{}=true\nagent={}\nagent_id={}\nspawn_semantics=registered_fake_agent_runtime_starts_on_task_send\ncommand_id={}\n",
        if spawn {
            "agent_spawned"
        } else {
            "agent_registered"
        },
        registration.agent_name,
        registration.agent_id,
        command.command_id
    ))
}

fn list_agents(parsed: &ParsedArgs) -> Result<String, String> {
    let state = state(parsed)?;
    let command = envelope(
        "agent-list",
        CommandTarget::Project(project_id()),
        CommandIntent::QueryStatus,
        None,
    );
    let agents = state.agents().map_err(debug_error)?;
    let mut output = format!(
        "command_id={}\nactive_agents={}\n",
        command.command_id,
        agents.len()
    );
    for agent in agents {
        output.push_str(&format!(
            "agent={} status={} current_session={}\n",
            agent.name,
            agent.status,
            agent
                .current_session_id
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| "none".to_string())
        ));
    }
    Ok(output)
}

fn replay_adapter_fixture(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
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

fn replay_adapter_dispatch_fixture(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
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

fn adapter_label(adapter: &str) -> &'static str {
    match adapter {
        "codex" | "codex-exec" | "codex_exec" => "codex_exec",
        "claude" | "claude-code" | "claude_code" => "claude_code",
        "acp" => "acp",
        _ => "unknown",
    }
}

fn adapter_readiness(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let record = args.iter().any(|arg| arg == "--record");
    if let Some(unknown) = args.iter().find(|arg| arg.as_str() != "--record") {
        return Err(format!("unknown adapter readiness option: {unknown}"));
    }
    let state_root = parsed.state_root.clone();
    let workspace_root = state_root.join("adapter-readiness").join("workspace");
    let artifact_root = state_root.join("adapter-readiness").join("artifacts");
    let plans = [
        CodexExecAdapter::local_smoke_plan(
            workspace_root.join("codex"),
            artifact_root.join("codex"),
        ),
        ClaudeCodeAdapter::local_smoke_plan(
            workspace_root.join("claude"),
            artifact_root.join("claude"),
        ),
    ];
    let mut output = format!(
        "adapter_readiness=true\nadapters={}\ncredential_policy=not_inspected\nreal_smoke_required_for_dogfood=true\n",
        plans.len()
    );
    let mut all_opted_in = true;
    let mut records = Vec::new();
    for plan in &plans {
        let opted_in = plan.is_opted_in();
        all_opted_in &= opted_in;
        output.push_str(&render_adapter_readiness(plan, opted_in));
        records.push(adapter_readiness_projection(plan, opted_in));
    }
    let recorded_sequence = if record {
        let event = NewEvent {
            event_id: format!(
                "event-adapter-readiness-{}",
                stable_cli_hash(&format!(
                    "{}:{}",
                    records
                        .iter()
                        .map(|record| format!(
                            "{}:{}:{}",
                            record.adapter_kind, record.opted_in, record.smoke_status
                        ))
                        .collect::<Vec<_>>()
                        .join("|"),
                    records.len()
                ))
            ),
            kind: EventKind::AdapterReadinessChecked,
            actor: "local-cli".to_string(),
            project_id: Some(project_id()),
            task_id: None,
            agent_id: None,
            session_id: None,
            run_id: None,
            turn_id: None,
            item_id: None,
            payload_json: format!(
                "{{\"adapter_count\":{},\"credential_policy\":\"not_inspected\",\"real_smoke_required_for_dogfood\":true}}",
                records.len()
            ),
            idempotency_key: Some("adapter-readiness-check:v1".to_string()),
            redaction_state: RedactionState::Safe,
        };
        let records = records
            .into_iter()
            .map(ProjectionRecord::AdapterReadiness)
            .collect::<Vec<_>>();
        Some(
            state(parsed)?
                .append_event(event, &records)
                .map_err(debug_error)?,
        )
    } else {
        None
    };
    output.push_str(&format!(
        "ready_to_run_all_real_smokes={}\nready_for_real_agent_dogfood=false\nblocked_reason=real_subscription_smoke_not_recorded\nrecorded={}\nrecorded_sequence={}\n",
        all_opted_in,
        record,
        recorded_sequence
            .map(|sequence| sequence.to_string())
            .unwrap_or_else(|| "none".to_string())
    ));
    Ok(output)
}

fn plan_adapter_launch(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let adapter = required_arg(args, "--adapter")?;
    validate_local_launch_adapter(&adapter)?;
    let agent = required_arg(args, "--agent")?;
    let goal = required_arg(args, "--goal")?;
    let record = has_flag(args, "--record");
    let state_root = parsed.state_root.clone();
    let workspace = optional_arg(args, "--workspace")
        .map(PathBuf::from)
        .unwrap_or_else(|| state_root.join("adapter-launch").join("workspace"));
    let artifacts = optional_arg(args, "--artifacts")
        .map(PathBuf::from)
        .unwrap_or_else(|| state_root.join("adapter-launch").join("artifacts"));
    if let Some(unknown) = args.iter().find(|arg| {
        arg.starts_with("--")
            && !matches!(
                arg.as_str(),
                "--adapter" | "--agent" | "--goal" | "--workspace" | "--artifacts" | "--record"
            )
    }) {
        return Err(format!("unknown adapter plan-launch option: {unknown}"));
    }
    let plan = recordable_adapter_dispatch_plan(
        parsed,
        DispatchPlanRecordRequest {
            adapter: &adapter,
            agent: &agent,
            goal: &goal,
            workspace,
            artifacts,
            prompt_source: DispatchPromptSourceInput::inline_cli_prompt(),
            record,
        },
    )?;
    Ok(format!(
        "adapter_launch_planned=true\n{}\n",
        render_adapter_dispatch_plan(&plan)
    ))
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RecordedAdapterDispatchPlan {
    projection: AdapterDispatchPlanProjection,
    prompt_source: AdapterDispatchPromptSourceProjection,
    runtime_safe_arg_count: usize,
    subscription_safe: bool,
    recorded: bool,
    recorded_sequence: Option<i64>,
}

struct DispatchPlanRecordRequest<'a> {
    adapter: &'a str,
    agent: &'a str,
    goal: &'a str,
    workspace: PathBuf,
    artifacts: PathBuf,
    prompt_source: DispatchPromptSourceInput,
    record: bool,
}

fn recordable_adapter_dispatch_plan(
    parsed: &ParsedArgs,
    request: DispatchPlanRecordRequest<'_>,
) -> Result<RecordedAdapterDispatchPlan, String> {
    let controller = controller(parsed)?;
    controller
        .registration_for_agent_name(request.agent)
        .or_else(|_| controller.register_agent(request.agent))
        .map_err(debug_error)?;
    let plan = controller
        .plan_local_adapter_dispatch(
            request.adapter,
            request.agent,
            request.goal,
            request.workspace,
            request.artifacts,
        )
        .map_err(|error| format!("adapter launch planning failed: {error}"))?;
    let safe_prompt_arg_count = plan.runtime_arg_count.saturating_sub(1);
    let goal_hash = stable_cli_hash(request.goal);
    let projection = adapter_dispatch_plan_projection(&plan, &goal_hash);
    let prompt_source_projection =
        adapter_dispatch_prompt_source_projection(&projection, &goal_hash, request.prompt_source);
    let recorded_sequence = if request.record {
        let event = NewEvent {
            event_id: format!(
                "event-adapter-dispatch-plan-{}",
                stable_cli_hash(&projection.dispatch_plan_id)
            ),
            kind: EventKind::AdapterDispatchPlanned,
            actor: "local-cli".to_string(),
            project_id: Some(project_id()),
            task_id: None,
            agent_id: Some(projection.agent_id.clone()),
            session_id: Some(projection.session_id.clone()),
            run_id: Some(projection.run_id.clone()),
            turn_id: None,
            item_id: None,
            payload_json: format!(
                "{{\"adapter\":\"{}\",\"agent\":\"{}\",\"runtime_prompt_policy\":\"not_rendered\",\"provider_cli_executed\":false}}",
                projection.adapter_kind,
                escape_json(&projection.agent_name)
            ),
            idempotency_key: Some(format!(
                "adapter-dispatch-plan:{}:{}:{}:{}:{}",
                projection.project_id,
                projection.adapter_kind,
                projection.agent_id,
                goal_hash,
                stable_cli_hash(&format!(
                    "{}:{}:{}",
                    projection.runtime_cwd, projection.artifact_root, projection.runtime_arg_count
                ))
            )),
            redaction_state: RedactionState::Safe,
        };
        let prompt_source_event = NewEvent {
            event_id: format!(
                "event-adapter-dispatch-prompt-source-{}",
                stable_cli_hash(&prompt_source_projection.prompt_source_id)
            ),
            kind: EventKind::AdapterDispatchPromptSourceRecorded,
            actor: "local-cli".to_string(),
            project_id: Some(project_id()),
            task_id: None,
            agent_id: Some(projection.agent_id.clone()),
            session_id: Some(projection.session_id.clone()),
            run_id: Some(projection.run_id.clone()),
            turn_id: None,
            item_id: Some(prompt_source_projection.prompt_source_id.clone()),
            payload_json: format!(
                "{{\"dispatch_plan_id\":\"{}\",\"prompt_hash\":\"{}\",\"source_kind\":\"{}\",\"raw_prompt_policy\":\"not_rendered\"}}",
                escape_json(&projection.dispatch_plan_id),
                prompt_source_projection.prompt_hash,
                escape_json(&prompt_source_projection.source_kind)
            ),
            idempotency_key: Some(format!(
                "adapter-dispatch-prompt-source:{}:{}:{}:{}",
                prompt_source_projection.project_id,
                prompt_source_projection.dispatch_plan_id,
                prompt_source_projection.prompt_hash,
                prompt_source_projection.source_kind
            )),
            redaction_state: RedactionState::Safe,
        };
        let state = state(parsed)?;
        state
            .append_event(
                event,
                &[ProjectionRecord::AdapterDispatchPlan(projection.clone())],
            )
            .map_err(debug_error)?;
        Some(
            state
                .append_event(
                    prompt_source_event,
                    &[ProjectionRecord::AdapterDispatchPromptSource(
                        prompt_source_projection.clone(),
                    )],
                )
                .map_err(debug_error)?,
        )
    } else {
        None
    };
    Ok(RecordedAdapterDispatchPlan {
        projection,
        prompt_source: prompt_source_projection,
        runtime_safe_arg_count: safe_prompt_arg_count,
        subscription_safe: true,
        recorded: request.record,
        recorded_sequence,
    })
}

fn render_adapter_dispatch_plan(plan: &RecordedAdapterDispatchPlan) -> String {
    format!(
        "adapter={}\nprovider_kind={}\ncredential_scope={}\nagent={}\nagent_id={}\nsession_id={}\nrun_id={}\nruntime_program={}\nruntime_arg_count={}\nruntime_prompt_policy={}\nruntime_prompt_source={}\nruntime_prompt_source_kind={}\nruntime_prompt_materialization={}\nruntime_safe_arg_count={}\nruntime_cwd={}\nartifact_root={}\nrequest_env_count={}\nenv_allowlist={}\nredaction_rules={}\nstdout_format={}\nstderr_policy={}\nsubscription_safe={}\nprovider_cli_executed={}\nrecorded={}\nrecorded_sequence={}",
        plan.projection.adapter_kind,
        plan.projection.provider_kind,
        plan.projection.credential_scope,
        plan.projection.agent_name,
        plan.projection.agent_id,
        plan.projection.session_id,
        plan.projection.run_id,
        plan.projection.runtime_program,
        plan.projection.runtime_arg_count,
        plan.projection.runtime_prompt_policy,
        plan.prompt_source.prompt_source_id,
        plan.prompt_source.source_kind,
        plan.prompt_source.materialization_status,
        plan.runtime_safe_arg_count,
        plan.projection.runtime_cwd,
        plan.projection.artifact_root,
        plan.projection.request_env_count,
        plan.projection.env_allowlist_count,
        plan.projection.redaction_rule_count,
        plan.projection.stdout_format,
        plan.projection.stderr_policy,
        plan.subscription_safe,
        plan.projection.provider_cli_executed,
        plan.recorded,
        plan.recorded_sequence
            .map(|sequence| sequence.to_string())
            .unwrap_or_else(|| "none".to_string())
    )
}

fn adapter_dispatch_plan_projection(
    plan: &capo_controller::LocalAdapterDispatchPlan,
    goal_hash: &str,
) -> AdapterDispatchPlanProjection {
    AdapterDispatchPlanProjection {
        dispatch_plan_id: format!(
            "adapter-dispatch-plan-{}-{}-{}",
            plan.launch_plan.adapter_kind.as_str(),
            goal_hash,
            stable_cli_hash(&format!(
                "{}:{}:{}:{}",
                plan.agent_id,
                plan.session_id,
                plan.runtime_cwd.display(),
                plan.launch_plan.artifact_root.display()
            ))
        ),
        project_id: plan.project_id.clone(),
        adapter_kind: plan.launch_plan.adapter_kind.as_str().to_string(),
        provider_kind: plan.launch_plan.provider_kind.clone(),
        credential_scope: plan.launch_plan.credential_scope.clone(),
        agent_id: plan.agent_id.clone(),
        agent_name: plan.agent_name.clone(),
        session_id: plan.session_id.clone(),
        run_id: plan.run_id.clone(),
        runtime_program: plan.runtime_program.clone(),
        runtime_arg_count: plan.runtime_arg_count as i64,
        runtime_prompt_policy: "not_rendered".to_string(),
        runtime_cwd: plan.runtime_cwd.display().to_string(),
        artifact_root: plan.launch_plan.artifact_root.display().to_string(),
        request_env_count: plan.request_env_count as i64,
        env_allowlist_count: plan.launch_plan.env_allowlist.len() as i64,
        redaction_rule_count: plan.launch_plan.redaction_rules.len() as i64,
        stdout_format: plan.launch_plan.stdout_format.clone(),
        stderr_policy: plan.launch_plan.stderr_policy.clone(),
        provider_cli_executed: false,
        status: "planned".to_string(),
        updated_sequence: 0,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DispatchPromptSourceInput {
    source_kind: String,
    source_ref: Option<String>,
    source_hash: Option<String>,
    materialization_status: String,
}

impl DispatchPromptSourceInput {
    fn inline_cli_prompt() -> Self {
        Self {
            source_kind: "inline_cli_prompt".to_string(),
            source_ref: None,
            source_hash: None,
            materialization_status: "manual_prompt_not_replayable".to_string(),
        }
    }

    fn workpad_task(task: &WorkpadTaskProjection, source_hash: String) -> Self {
        Self {
            source_kind: "workpad_task".to_string(),
            source_ref: Some(format!("{}#{}", task.path, task.source_anchor)),
            source_hash: Some(source_hash),
            materialization_status: "replayable_if_source_hash_matches".to_string(),
        }
    }
}

fn adapter_dispatch_prompt_source_projection(
    plan: &AdapterDispatchPlanProjection,
    prompt_hash: &str,
    input: DispatchPromptSourceInput,
) -> AdapterDispatchPromptSourceProjection {
    AdapterDispatchPromptSourceProjection {
        prompt_source_id: format!(
            "adapter-dispatch-prompt-source-{}-{}",
            stable_cli_hash(&plan.dispatch_plan_id),
            stable_cli_hash(&format!(
                "{}:{}:{}",
                prompt_hash,
                input.source_kind,
                input.source_ref.as_deref().unwrap_or("none")
            ))
        ),
        project_id: plan.project_id.clone(),
        dispatch_plan_id: plan.dispatch_plan_id.clone(),
        prompt_hash: prompt_hash.to_string(),
        source_kind: input.source_kind,
        source_ref: input.source_ref,
        source_hash: input.source_hash,
        materialization_status: input.materialization_status,
        raw_prompt_policy: "not_rendered".to_string(),
        updated_sequence: 0,
    }
}

fn validate_local_launch_adapter(adapter: &str) -> Result<(), String> {
    match adapter {
        "codex" | "codex-exec" | "codex_exec" | "claude" | "claude-code" | "claude_code" => Ok(()),
        other => Err(format!(
            "unsupported local adapter dispatch plan: {other}; expected codex or claude"
        )),
    }
}

fn render_adapter_readiness(plan: &LocalAdapterSmokePlan, opted_in: bool) -> String {
    format!(
        "adapter={} program={} opt_in_env={} opted_in={} smoke_status={} expected_marker={} env_allowlist={} redaction_rules={} output_limit_bytes={} workspace={} artifacts={}\n",
        plan.adapter_kind.as_str(),
        plan.program,
        plan.opt_in_env,
        opted_in,
        if opted_in {
            "ready_to_run"
        } else {
            "waiting_on_opt_in"
        },
        plan.expected_output_marker,
        plan.env_allowlist.len(),
        plan.redaction_rules.len(),
        plan.output_limit_bytes,
        plan.workspace_root.display(),
        plan.artifact_root.display()
    )
}

fn adapter_readiness_projection(
    plan: &LocalAdapterSmokePlan,
    opted_in: bool,
) -> AdapterReadinessProjection {
    AdapterReadinessProjection {
        adapter_kind: plan.adapter_kind.as_str().to_string(),
        project_id: project_id(),
        program: plan.program.clone(),
        opt_in_env: plan.opt_in_env.to_string(),
        opted_in,
        smoke_status: if opted_in {
            "ready_to_run".to_string()
        } else {
            "waiting_on_opt_in".to_string()
        },
        credential_policy: "not_inspected".to_string(),
        expected_marker: plan.expected_output_marker.to_string(),
        env_allowlist_count: plan.env_allowlist.len() as i64,
        redaction_rule_count: plan.redaction_rules.len() as i64,
        output_limit_bytes: plan.output_limit_bytes as i64,
        dogfood_blocker: Some("real_subscription_smoke_not_recorded".to_string()),
        updated_sequence: 0,
    }
}

fn record_adapter_smoke_report(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let adapter = adapter_label(&required_arg(args, "--adapter")?).to_string();
    if !matches!(adapter.as_str(), "codex_exec" | "claude_code") {
        return Err("adapter smoke reports currently support codex or claude".to_string());
    }
    let smoke_status = required_arg(args, "--status")?;
    if !matches!(smoke_status.as_str(), "skipped" | "passed" | "failed") {
        return Err("--status must be skipped, passed, or failed".to_string());
    }
    let credential_scan_status = required_arg(args, "--credential-scan")?;
    if !matches!(
        credential_scan_status.as_str(),
        "clean" | "blocked" | "not_run"
    ) {
        return Err("--credential-scan must be clean, blocked, or not_run".to_string());
    }
    let reason = required_arg(args, "--reason")?;
    let marker_found = has_flag(args, "--marker-found");
    let artifact_root = optional_arg(args, "--artifact-root");
    if smoke_status == "passed" && (credential_scan_status != "clean" || !marker_found) {
        return Err(
            "passed smoke reports require --credential-scan clean and --marker-found".to_string(),
        );
    }
    if smoke_status == "passed" {
        let artifact_root = artifact_root
            .as_ref()
            .ok_or_else(|| "passed smoke reports require --artifact-root".to_string())?;
        scan_artifact_root(Path::new(artifact_root))?;
    }
    let smoke_report_id = format!(
        "adapter-smoke-{}-{}",
        adapter,
        stable_cli_hash(&format!(
            "{adapter}:{smoke_status}:{credential_scan_status}:{marker_found}:{reason}"
        ))
    );
    let dogfood_readiness_effect =
        if smoke_status == "passed" && credential_scan_status == "clean" && marker_found {
            "real_agent_connector_proven"
        } else {
            "real_subscription_smoke_not_recorded"
        };
    let report = AdapterSmokeReportProjection {
        smoke_report_id: smoke_report_id.clone(),
        project_id: project_id(),
        adapter_kind: adapter.clone(),
        smoke_status: smoke_status.clone(),
        credential_scan_status: credential_scan_status.clone(),
        marker_found,
        artifact_root: artifact_root.clone(),
        reason: reason.clone(),
        dogfood_readiness_effect: dogfood_readiness_effect.to_string(),
        updated_sequence: 0,
    };
    let event = NewEvent {
        event_id: format!("event-adapter-smoke-{}", stable_cli_hash(&smoke_report_id)),
        kind: EventKind::AdapterSmokeRecorded,
        actor: "local-cli".to_string(),
        project_id: Some(project_id()),
        task_id: None,
        agent_id: None,
        session_id: None,
        run_id: None,
        turn_id: None,
        item_id: Some(smoke_report_id.clone()),
        payload_json: format!(
            "{{\"adapter\":\"{}\",\"smoke_status\":\"{}\",\"credential_scan_status\":\"{}\",\"dogfood_readiness_effect\":\"{}\"}}",
            escape_json(&adapter),
            escape_json(&smoke_status),
            escape_json(&credential_scan_status),
            escape_json(dogfood_readiness_effect)
        ),
        idempotency_key: Some(format!("adapter-smoke-report:{smoke_report_id}")),
        redaction_state: RedactionState::Safe,
    };
    let sequence = state(parsed)?
        .append_event(event, &[ProjectionRecord::AdapterSmokeReport(report)])
        .map_err(debug_error)?;
    Ok(format!(
        "adapter_smoke_report_recorded=true\nsmoke_report_id={smoke_report_id}\nadapter={adapter}\nsmoke_status={smoke_status}\ncredential_scan_status={credential_scan_status}\nmarker_found={marker_found}\ndogfood_readiness_effect={dogfood_readiness_effect}\nartifact_root={}\nsequence={sequence}\n",
        artifact_root.as_deref().unwrap_or("none")
    ))
}

fn scan_adapter_smoke_artifacts(args: &[String]) -> Result<String, String> {
    let artifact_root = required_arg(args, "--artifact-root")?;
    let scan = scan_artifact_root(Path::new(&artifact_root))?;
    Ok(format!(
        "adapter_smoke_artifact_scan=true\ncredential_scan_status=clean\nartifact_root={artifact_root}\nfiles_scanned={}\n",
        scan.files_scanned
    ))
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ArtifactScanSummary {
    files_scanned: usize,
}

fn scan_artifact_root(root: &Path) -> Result<ArtifactScanSummary, String> {
    if !root.is_dir() {
        return Err(format!(
            "artifact root does not exist or is not a directory: {}",
            root.display()
        ));
    }
    let files = collect_regular_files(root)?;
    if files.is_empty() {
        return Err(format!(
            "artifact root contains no files: {}",
            root.display()
        ));
    }
    scan_artifacts_for_sensitive_markers(files.iter()).map_err(format_smoke_scan_error)?;
    Ok(ArtifactScanSummary {
        files_scanned: files.len(),
    })
}

fn collect_regular_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut pending = vec![root.to_path_buf()];
    let mut files = Vec::new();
    while let Some(path) = pending.pop() {
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("failed to read artifact path {}: {error}", path.display()))?;
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "artifact scan refuses symlink path: {}",
                path.display()
            ));
        }
        if metadata.is_dir() {
            for entry in fs::read_dir(&path).map_err(|error| {
                format!(
                    "failed to read artifact directory {}: {error}",
                    path.display()
                )
            })? {
                let entry = entry.map_err(|error| {
                    format!(
                        "failed to read artifact directory entry {}: {error}",
                        path.display()
                    )
                })?;
                pending.push(entry.path());
            }
        } else if metadata.is_file() {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

fn format_smoke_scan_error(error: LocalAdapterSmokeError) -> String {
    match error {
        LocalAdapterSmokeError::SensitiveArtifact { path, marker } => format!(
            "credential scan blocked artifact {} because marker `{marker}` was not redacted",
            path.display()
        ),
        LocalAdapterSmokeError::Io(error) => {
            format!("credential scan failed to read artifact: {error}")
        }
        LocalAdapterSmokeError::Runtime(error) => {
            format!("credential scan runtime error: {error:?}")
        }
        LocalAdapterSmokeError::NotOptedIn(env) => {
            format!("credential scan unexpectedly hit opt-in gate: {env}")
        }
        LocalAdapterSmokeError::MarkerMissing { marker } => {
            format!("credential scan unexpectedly checked marker: {marker}")
        }
    }
}

fn scan_dispatch_artifacts_or_delete<'a>(
    paths: impl IntoIterator<Item = &'a PathBuf>,
) -> Result<(), LocalAdapterSmokeError> {
    let paths = paths.into_iter().cloned().collect::<Vec<_>>();
    match scan_artifacts_for_sensitive_markers(paths.iter()) {
        Ok(()) => Ok(()),
        Err(error) => {
            for path in &paths {
                let _ = fs::remove_file(path);
            }
            Err(error)
        }
    }
}

fn adapter_dogfood_gate(parsed: &ParsedArgs) -> Result<String, String> {
    let state = state(parsed)?;
    let dashboard =
        project_dashboard(&state, ProjectDashboardQuery::new(project_id())).map_err(debug_error)?;
    Ok(render_adapter_dogfood_gate(&dashboard.adapter_dogfood_gate))
}

fn dogfood_readiness(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let out = optional_arg(args, "--out").map(PathBuf::from);
    if has_flag(args, "--out") && out.is_none() {
        return Err("--out requires a value".to_string());
    }
    if let Some(unknown) = args
        .iter()
        .find(|arg| arg.starts_with("--") && !matches!(arg.as_str(), "--out"))
    {
        return Err(format!("unknown dogfood readiness option: {unknown}"));
    }
    let state = state(parsed)?;
    let project_id = project_id();
    let dashboard = project_dashboard(&state, ProjectDashboardQuery::new(project_id.clone()))
        .map_err(debug_error)?;
    let readiness = project_dogfood_readiness(&dashboard);
    let mut output = render_dogfood_readiness(&readiness);
    if let Some(out) = out {
        let command = envelope(
            "dogfood-readiness",
            CommandTarget::Project(project_id.clone()),
            CommandIntent::ExportEvidence,
            Some(readiness.status.clone()),
        );
        let markdown = render_dogfood_readiness_evidence(&project_id, &readiness);
        fs::create_dir_all(&out).map_err(|error| error.to_string())?;
        let content_hash = stable_cli_hash(&markdown);
        let artifact_id = format!("artifact-dogfood-readiness-{content_hash}");
        let path = out.join(format!("{artifact_id}.md"));
        write_dogfood_readiness_file(&path, &markdown)?;
        state
            .record_artifact(ArtifactRecord {
                artifact_id: artifact_id.clone(),
                project_id: Some(project_id.clone()),
                session_id: None,
                run_id: None,
                kind: "dogfood_readiness".to_string(),
                uri: path.display().to_string(),
                content_hash: content_hash.clone(),
                size_bytes: markdown.len() as i64,
                redaction_state: RedactionState::Safe,
            })
            .map_err(debug_error)?;

        let evidence_id = format!("evidence-{artifact_id}");
        let sequence = state
            .append_event(
                NewEvent {
                    event_id: format!("event-{evidence_id}"),
                    kind: EventKind::EvidenceRecorded,
                    actor: "cli".to_string(),
                    project_id: Some(project_id.clone()),
                    task_id: None,
                    agent_id: None,
                    session_id: None,
                    run_id: None,
                    turn_id: None,
                    item_id: Some(evidence_id.clone()),
                    payload_json: format!(
                        "{{\"artifact_id\":\"{}\",\"content_hash\":\"{}\",\"ready\":{},\"status\":\"{}\"}}",
                        escape_json(&artifact_id),
                        escape_json(&content_hash),
                        readiness.ready,
                        escape_json(&readiness.status)
                    ),
                    idempotency_key: Some(format!(
                        "dogfood-readiness:{}:{content_hash}",
                        project_id
                    )),
                    redaction_state: RedactionState::Safe,
                },
                &[ProjectionRecord::Evidence(EvidenceProjection {
                    evidence_id: capo_core::EvidenceId::new(evidence_id.clone()),
                    project_id: project_id.clone(),
                    task_id: None,
                    session_id: None,
                    run_id: None,
                    kind: "dogfood_readiness".to_string(),
                    artifact_id: Some(artifact_id.clone()),
                    confidence: dogfood_readiness_confidence(&readiness),
                    updated_sequence: 0,
                })],
            )
            .map_err(debug_error)?;
        output.push_str(&format!(
            "dogfood_readiness_evidence_exported=true\nevidence_id={evidence_id}\nartifact_id={artifact_id}\npath={}\ncontent_hash={content_hash}\nsequence={sequence}\ncommand_id={}\n",
            path.display(),
            command.command_id
        ));
    }
    Ok(output)
}

fn adapter_dispatch_gate(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let dispatch_plan_id = required_arg(args, "--dispatch-plan")?;
    let record = has_flag(args, "--record");
    if let Some(unknown) = args.iter().find(|arg| {
        arg.starts_with("--") && !matches!(arg.as_str(), "--dispatch-plan" | "--record")
    }) {
        return Err(format!("unknown adapter dispatch-gate option: {unknown}"));
    }
    let state = state(parsed)?;
    let dashboard =
        project_dashboard(&state, ProjectDashboardQuery::new(project_id())).map_err(debug_error)?;
    let plan = dashboard
        .adapter_dispatch_plans
        .iter()
        .find(|plan| plan.dispatch_plan_id == dispatch_plan_id)
        .ok_or_else(|| format!("unknown adapter dispatch plan: {dispatch_plan_id}"))?;
    let gate = adapter_dispatch_gate_projection(plan, &dashboard.adapter_dogfood_gate);
    let recorded_sequence = if record {
        let event = NewEvent {
            event_id: format!(
                "event-adapter-dispatch-gate-{}",
                stable_cli_hash(&gate.dispatch_gate_id)
            ),
            kind: EventKind::AdapterDispatchGateChecked,
            actor: "local-cli".to_string(),
            project_id: Some(gate.project_id.clone()),
            task_id: None,
            agent_id: Some(plan.agent_id.clone()),
            session_id: Some(plan.session_id.clone()),
            run_id: Some(plan.run_id.clone()),
            turn_id: None,
            item_id: Some(gate.dispatch_gate_id.clone()),
            payload_json: format!(
                "{{\"dispatch_plan_id\":\"{}\",\"provider_cli_execution_allowed\":{},\"provider_cli_executed\":false}}",
                escape_json(&gate.dispatch_plan_id),
                gate.provider_cli_execution_allowed
            ),
            idempotency_key: Some(format!(
                "adapter-dispatch-gate:{}:{}:{}",
                gate.project_id, gate.dispatch_plan_id, gate.reason_codes
            )),
            redaction_state: RedactionState::Safe,
        };
        Some(
            state
                .append_event(
                    event,
                    &[ProjectionRecord::AdapterDispatchGate(gate.clone())],
                )
                .map_err(debug_error)?,
        )
    } else {
        None
    };
    Ok(render_adapter_dispatch_gate(
        &gate,
        record,
        recorded_sequence,
    ))
}

fn adapter_dispatch_status(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let latest = has_flag(args, "--latest");
    let dispatch_plan_id = optional_arg(args, "--dispatch-plan");
    let agent_name = optional_arg(args, "--agent");
    if let Some(unknown) = args.iter().find(|arg| {
        arg.starts_with("--") && !matches!(arg.as_str(), "--dispatch-plan" | "--latest" | "--agent")
    }) {
        return Err(format!("unknown adapter dispatch-status option: {unknown}"));
    }
    if latest && dispatch_plan_id.is_some() {
        return Err(
            "adapter dispatch-status accepts either --dispatch-plan or --latest".to_string(),
        );
    }
    if !latest && agent_name.is_some() {
        return Err("adapter dispatch-status --agent requires --latest".to_string());
    }
    let state = state(parsed)?;
    let dashboard =
        project_dashboard(&state, ProjectDashboardQuery::new(project_id())).map_err(debug_error)?;
    let status = if latest {
        dashboard
            .latest_adapter_dispatch_status(agent_name.as_deref())
            .ok_or_else(|| {
                agent_name.map_or_else(
                    || "no recorded adapter dispatch plans".to_string(),
                    |agent| format!("no recorded adapter dispatch plans for agent: {agent}"),
                )
            })?
    } else {
        let dispatch_plan_id = dispatch_plan_id.ok_or_else(|| {
            "adapter dispatch-status requires --dispatch-plan or --latest".to_string()
        })?;
        dashboard
            .adapter_dispatch_status(&dispatch_plan_id)
            .ok_or_else(|| format!("unknown adapter dispatch plan: {dispatch_plan_id}"))?
    };
    Ok(render_adapter_dispatch_status(&status))
}

fn adapter_dispatch_evidence(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let latest = has_flag(args, "--latest");
    let dispatch_plan_id = optional_arg(args, "--dispatch-plan");
    let agent_name = optional_arg(args, "--agent");
    let out = PathBuf::from(required_arg(args, "--out")?);
    if let Some(unknown) = args.iter().find(|arg| {
        arg.starts_with("--")
            && !matches!(
                arg.as_str(),
                "--dispatch-plan" | "--latest" | "--agent" | "--out"
            )
    }) {
        return Err(format!(
            "unknown adapter dispatch-evidence option: {unknown}"
        ));
    }
    if latest && dispatch_plan_id.is_some() {
        return Err(
            "adapter dispatch-evidence accepts either --dispatch-plan or --latest".to_string(),
        );
    }
    if !latest && agent_name.is_some() {
        return Err("adapter dispatch-evidence --agent requires --latest".to_string());
    }
    let state = state(parsed)?;
    let dashboard =
        project_dashboard(&state, ProjectDashboardQuery::new(project_id())).map_err(debug_error)?;
    let dispatch_plan_id = if latest {
        dashboard
            .latest_adapter_dispatch_status(agent_name.as_deref())
            .ok_or_else(|| {
                agent_name.map_or_else(
                    || "no recorded adapter dispatch plans".to_string(),
                    |agent| format!("no recorded adapter dispatch plans for agent: {agent}"),
                )
            })?
            .dispatch_plan_id
    } else {
        dispatch_plan_id.ok_or_else(|| {
            "adapter dispatch-evidence requires --dispatch-plan or --latest".to_string()
        })?
    };
    let plan = dashboard
        .adapter_dispatch_plans
        .iter()
        .find(|plan| plan.dispatch_plan_id == dispatch_plan_id)
        .ok_or_else(|| format!("unknown adapter dispatch plan: {dispatch_plan_id}"))?;
    let latest_gate = dashboard
        .adapter_dispatch_gates
        .iter()
        .rev()
        .find(|gate| gate.dispatch_plan_id == plan.dispatch_plan_id);
    let latest_replay = dashboard
        .adapter_dispatch_replays
        .iter()
        .rev()
        .find(|replay| replay.dispatch_plan_id == plan.dispatch_plan_id);
    let latest_execution = dashboard
        .adapter_dispatch_executions
        .iter()
        .rev()
        .find(|execution| execution.dispatch_plan_id == plan.dispatch_plan_id);
    let command = envelope(
        "adapter-dispatch-evidence",
        CommandTarget::Session(plan.session_id.clone()),
        CommandIntent::ExportEvidence,
        Some(dispatch_plan_id.clone()),
    );
    let markdown = render_adapter_dispatch_evidence(
        plan,
        latest_gate,
        latest_replay,
        latest_execution,
        &dashboard.adapter_dogfood_gate,
    );
    fs::create_dir_all(&out).map_err(|error| error.to_string())?;
    let content_hash = stable_cli_hash(&markdown);
    let artifact_id = format!(
        "artifact-adapter-dispatch-evidence-{}-{}",
        stable_cli_hash(&plan.dispatch_plan_id),
        content_hash
    );
    let path = out.join(format!("{artifact_id}.md"));
    write_dispatch_evidence_file(&path, &markdown)?;
    state
        .record_artifact(ArtifactRecord {
            artifact_id: artifact_id.clone(),
            project_id: Some(plan.project_id.clone()),
            session_id: Some(plan.session_id.clone()),
            run_id: Some(plan.run_id.clone()),
            kind: "adapter_dispatch_evidence".to_string(),
            uri: path.display().to_string(),
            content_hash: content_hash.clone(),
            size_bytes: markdown.len() as i64,
            redaction_state: RedactionState::Safe,
        })
        .map_err(debug_error)?;

    let evidence_id = format!("evidence-{artifact_id}");
    let sequence = state
        .append_event(
            NewEvent {
                event_id: format!("event-{evidence_id}"),
                kind: EventKind::EvidenceRecorded,
                actor: "cli".to_string(),
                project_id: Some(plan.project_id.clone()),
                task_id: None,
                agent_id: Some(plan.agent_id.clone()),
                session_id: Some(plan.session_id.clone()),
                run_id: Some(plan.run_id.clone()),
                turn_id: None,
                item_id: Some(evidence_id.clone()),
                payload_json: format!(
                    "{{\"dispatch_plan_id\":\"{}\",\"artifact_id\":\"{}\",\"content_hash\":\"{}\"}}",
                    escape_json(&plan.dispatch_plan_id),
                    escape_json(&artifact_id),
                    escape_json(&content_hash)
                ),
                idempotency_key: Some(format!(
                    "adapter-dispatch-evidence:{}:{}",
                    plan.dispatch_plan_id, content_hash
                )),
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::Evidence(EvidenceProjection {
                evidence_id: capo_core::EvidenceId::new(evidence_id.clone()),
                project_id: plan.project_id.clone(),
                task_id: None,
                session_id: Some(plan.session_id.clone()),
                run_id: Some(plan.run_id.clone()),
                kind: "adapter_dispatch_evidence".to_string(),
                artifact_id: Some(artifact_id.clone()),
                confidence: dispatch_evidence_confidence(latest_execution, latest_replay),
                updated_sequence: 0,
            })],
        )
        .map_err(debug_error)?;
    Ok(format!(
        "adapter_dispatch_evidence_exported=true\ndispatch_plan={}\nevidence_id={evidence_id}\nartifact_id={artifact_id}\npath={}\ncontent_hash={content_hash}\nsequence={sequence}\ncommand_id={}\n",
        plan.dispatch_plan_id,
        path.display(),
        command.command_id
    ))
}

fn adapter_dispatch_execution_request(
    parsed: &ParsedArgs,
    args: &[String],
) -> Result<String, String> {
    let dispatch_plan_id = required_arg(args, "--dispatch-plan")?;
    let record = has_flag(args, "--record");
    if let Some(unknown) = args.iter().find(|arg| {
        arg.starts_with("--") && !matches!(arg.as_str(), "--dispatch-plan" | "--record")
    }) {
        return Err(format!(
            "unknown adapter execution-request option: {unknown}"
        ));
    }
    let state = state(parsed)?;
    let dashboard =
        project_dashboard(&state, ProjectDashboardQuery::new(project_id())).map_err(debug_error)?;
    let plan = dashboard
        .adapter_dispatch_plans
        .iter()
        .find(|plan| plan.dispatch_plan_id == dispatch_plan_id)
        .ok_or_else(|| format!("unknown adapter dispatch plan: {dispatch_plan_id}"))?;
    let latest_gate = dashboard
        .adapter_dispatch_gates
        .iter()
        .rev()
        .find(|gate| gate.dispatch_plan_id == plan.dispatch_plan_id);
    let request = adapter_dispatch_execution_request_projection(plan, latest_gate);
    let recorded_sequence = if record {
        let event = NewEvent {
            event_id: format!(
                "event-adapter-dispatch-execution-request-{}",
                stable_cli_hash(&request.execution_request_id)
            ),
            kind: EventKind::AdapterDispatchExecutionRequested,
            actor: "local-cli".to_string(),
            project_id: Some(request.project_id.clone()),
            task_id: None,
            agent_id: Some(plan.agent_id.clone()),
            session_id: Some(plan.session_id.clone()),
            run_id: Some(plan.run_id.clone()),
            turn_id: None,
            item_id: Some(request.execution_request_id.clone()),
            payload_json: format!(
                "{{\"dispatch_plan_id\":\"{}\",\"dispatch_gate_id\":\"{}\",\"provider_cli_execution_allowed\":{},\"provider_cli_executed\":false,\"status\":\"{}\"}}",
                escape_json(&request.dispatch_plan_id),
                escape_json(&request.dispatch_gate_id),
                request.provider_cli_execution_allowed,
                escape_json(&request.status)
            ),
            idempotency_key: Some(format!(
                "adapter-dispatch-execution-request:{}:{}:{}",
                request.project_id, request.dispatch_plan_id, request.status
            )),
            redaction_state: RedactionState::Safe,
        };
        Some(
            state
                .append_event(
                    event,
                    &[ProjectionRecord::AdapterDispatchExecutionRequest(
                        request.clone(),
                    )],
                )
                .map_err(debug_error)?,
        )
    } else {
        None
    };
    Ok(render_adapter_dispatch_execution_request(
        &request,
        record,
        recorded_sequence,
    ))
}

fn adapter_dispatch_materialize_prompt(
    parsed: &ParsedArgs,
    args: &[String],
) -> Result<String, String> {
    let dispatch_plan_id = required_arg(args, "--dispatch-plan")?;
    let record = has_flag(args, "--record");
    if let Some(unknown) = args.iter().find(|arg| {
        arg.starts_with("--") && !matches!(arg.as_str(), "--dispatch-plan" | "--record")
    }) {
        return Err(format!(
            "unknown adapter materialize-prompt option: {unknown}"
        ));
    }
    let state = state(parsed)?;
    let dashboard =
        project_dashboard(&state, ProjectDashboardQuery::new(project_id())).map_err(debug_error)?;
    let prompt_source = dashboard
        .adapter_dispatch_prompt_sources
        .iter()
        .rev()
        .find(|source| source.dispatch_plan_id == dispatch_plan_id)
        .ok_or_else(|| {
            format!("dispatch plan has no recorded prompt source: {dispatch_plan_id}")
        })?;
    let materialization =
        adapter_dispatch_prompt_materialization_projection(&state, prompt_source)?;
    let recorded_sequence = if record {
        let event = NewEvent {
            event_id: format!(
                "event-adapter-dispatch-prompt-materialization-{}",
                stable_cli_hash(&materialization.materialization_id)
            ),
            kind: EventKind::AdapterDispatchPromptMaterialized,
            actor: "local-cli".to_string(),
            project_id: Some(materialization.project_id.clone()),
            task_id: None,
            agent_id: None,
            session_id: None,
            run_id: None,
            turn_id: None,
            item_id: Some(materialization.materialization_id.clone()),
            payload_json: format!(
                "{{\"dispatch_plan_id\":\"{}\",\"prompt_source_id\":\"{}\",\"status\":\"{}\",\"raw_prompt_policy\":\"not_rendered\"}}",
                escape_json(&materialization.dispatch_plan_id),
                escape_json(&materialization.prompt_source_id),
                escape_json(&materialization.status)
            ),
            idempotency_key: Some(format!(
                "adapter-dispatch-prompt-materialization:{}:{}:{}:{}",
                materialization.project_id,
                materialization.prompt_source_id,
                materialization.status,
                materialization.materialization_id
            )),
            redaction_state: RedactionState::Safe,
        };
        Some(
            state
                .append_event(
                    event,
                    &[ProjectionRecord::AdapterDispatchPromptMaterialization(
                        materialization.clone(),
                    )],
                )
                .map_err(debug_error)?,
        )
    } else {
        None
    };
    Ok(render_adapter_dispatch_prompt_materialization(
        &materialization,
        record,
        recorded_sequence,
    ))
}

fn adapter_dispatch_run_preflight(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let dispatch_plan_id = required_arg(args, "--dispatch-plan")?;
    if let Some(unknown) = args
        .iter()
        .find(|arg| arg.starts_with("--") && !matches!(arg.as_str(), "--dispatch-plan"))
    {
        return Err(format!("unknown adapter run-preflight option: {unknown}"));
    }
    let state = state(parsed)?;
    let dashboard =
        project_dashboard(&state, ProjectDashboardQuery::new(project_id())).map_err(debug_error)?;
    let plan = dashboard
        .adapter_dispatch_plans
        .iter()
        .find(|plan| plan.dispatch_plan_id == dispatch_plan_id)
        .ok_or_else(|| format!("unknown adapter dispatch plan: {dispatch_plan_id}"))?;
    let execution_request = dashboard
        .adapter_dispatch_execution_requests
        .iter()
        .rev()
        .find(|request| request.dispatch_plan_id == plan.dispatch_plan_id);
    let materialization = dashboard
        .adapter_dispatch_prompt_materializations
        .iter()
        .rev()
        .find(|row| row.dispatch_plan_id == plan.dispatch_plan_id);
    Ok(render_adapter_dispatch_run_preflight(
        &dispatch_run_preflight(plan, execution_request, materialization),
    ))
}

fn adapter_dispatch_run_local(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let dispatch_plan_id = required_arg(args, "--dispatch-plan")?;
    let record = has_flag(args, "--record");
    if let Some(unknown) = args.iter().find(|arg| {
        arg.starts_with("--") && !matches!(arg.as_str(), "--dispatch-plan" | "--record")
    }) {
        return Err(format!("unknown adapter run-local option: {unknown}"));
    }
    let state = state(parsed)?;
    let dashboard =
        project_dashboard(&state, ProjectDashboardQuery::new(project_id())).map_err(debug_error)?;
    let plan = dashboard
        .adapter_dispatch_plans
        .iter()
        .find(|plan| plan.dispatch_plan_id == dispatch_plan_id)
        .ok_or_else(|| format!("unknown adapter dispatch plan: {dispatch_plan_id}"))?;
    let execution_request = dashboard
        .adapter_dispatch_execution_requests
        .iter()
        .rev()
        .find(|request| request.dispatch_plan_id == plan.dispatch_plan_id);
    let materialization = dashboard
        .adapter_dispatch_prompt_materializations
        .iter()
        .rev()
        .find(|row| row.dispatch_plan_id == plan.dispatch_plan_id);
    let preflight = dispatch_run_preflight(plan, execution_request, materialization);
    if !preflight.provider_cli_execution_allowed {
        let execution = adapter_dispatch_execution_projection(
            plan,
            execution_request,
            &preflight,
            AdapterDispatchExecutionRuntimeOutcome::blocked(),
        );
        let recorded_sequence = if record {
            Some(record_adapter_dispatch_execution(&state, plan, &execution)?)
        } else {
            None
        };
        return Ok(render_adapter_dispatch_run_local_blocked(
            &preflight,
            record,
            recorded_sequence,
        ));
    }

    let prompt_source = dashboard
        .adapter_dispatch_prompt_sources
        .iter()
        .rev()
        .find(|source| source.dispatch_plan_id == plan.dispatch_plan_id)
        .ok_or_else(|| {
            format!(
                "dispatch plan has no prompt source for local run: {}",
                plan.dispatch_plan_id
            )
        })?;
    let launch_plan = build_dispatch_run_launch_plan(&state, plan, prompt_source, materialization)?;
    launch_plan.assert_subscription_safe()?;
    fs::create_dir_all(&launch_plan.workspace_root)
        .map_err(|error| format!("failed to create dispatch workspace: {error}"))?;
    fs::create_dir_all(&launch_plan.artifact_root)
        .map_err(|error| format!("failed to create dispatch artifact root: {error}"))?;
    let runner = LocalProcessRunner::new(launch_plan.runtime_config());
    let outcome = runner
        .start_process(launch_plan.runtime_request(RunId::new(plan.run_id.to_string())))
        .map_err(LocalAdapterSmokeError::Runtime)
        .map_err(format_smoke_scan_error)?;
    scan_dispatch_artifacts_or_delete([&outcome.stdout.path, &outcome.stderr.path])
        .map_err(format_smoke_scan_error)?;
    let execution = adapter_dispatch_execution_projection(
        plan,
        execution_request,
        &preflight,
        AdapterDispatchExecutionRuntimeOutcome {
            provider_cli_executed: true,
            status: outcome.process.status.clone(),
            exit_code: outcome.exit_code.map(i64::from),
            runtime_process_ref: Some(outcome.process.runtime_process_ref.clone()),
            stdout_artifact_id: Some(outcome.stdout.artifact_id.clone()),
            stderr_artifact_id: Some(outcome.stderr.artifact_id.clone()),
            credential_scan_status: "clean".to_string(),
            raw_output_policy: "bounded_redacted_artifacts".to_string(),
            reason_codes: "provider_cli_executed_and_artifacts_scanned".to_string(),
        },
    );
    let recorded_sequence = record_adapter_dispatch_execution(&state, plan, &execution)?;
    Ok(format!(
        "adapter_dispatch_run_local=true\ndispatch_execution={}\ndispatch_plan={}\nadapter={}\nprovider_cli_execution_allowed=true\nprovider_cli_executed=true\nstatus={}\nruntime_process_ref={}\nexit_code={}\nstdout_artifact={}\nstderr_artifact={}\nartifact_root={}\nraw_prompt_policy={}\nraw_output_policy=bounded_redacted_artifacts\nrecorded=true\nrecorded_sequence={}\n",
        execution.dispatch_execution_id,
        plan.dispatch_plan_id,
        plan.adapter_kind,
        outcome.process.status,
        outcome.process.runtime_process_ref,
        outcome
            .exit_code
            .map(|code| code.to_string())
            .unwrap_or_else(|| "signal".to_string()),
        outcome.stdout.artifact_id,
        outcome.stderr.artifact_id,
        launch_plan.artifact_root.display(),
        materialization
            .map(|row| row.raw_prompt_policy.as_str())
            .unwrap_or("none"),
        recorded_sequence
    ))
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AdapterDispatchRunPreflight {
    dispatch_plan_id: String,
    adapter_kind: String,
    execution_request_id: String,
    materialization_id: String,
    provider_cli_execution_allowed: bool,
    opt_in_env: String,
    opt_in_set: bool,
    status: String,
    runtime_prompt_policy: String,
    raw_prompt_policy: String,
    reasons: Vec<String>,
    next_action: String,
}

fn dispatch_run_preflight(
    plan: &AdapterDispatchPlanProjection,
    execution_request: Option<&AdapterDispatchExecutionRequestProjection>,
    materialization: Option<&AdapterDispatchPromptMaterializationProjection>,
) -> AdapterDispatchRunPreflight {
    let opt_in_env = adapter_dispatch_opt_in_env(&plan.adapter_kind);
    let opt_in_set = env::var(&opt_in_env).as_deref() == Ok("1");
    let mut reasons = Vec::new();
    let mut status = "ready_to_execute_provider_cli".to_string();

    match execution_request {
        Some(request)
            if request.provider_cli_execution_allowed
                && request.status == "waiting_on_explicit_provider_opt_in"
                && !request.provider_cli_executed => {}
        Some(request) => {
            status = "blocked_execution_request_not_ready".to_string();
            reasons.push(request.status.clone());
        }
        None => {
            status = "blocked_missing_execution_request".to_string();
            reasons.push("recorded_execution_request_missing".to_string());
        }
    }

    match materialization {
        Some(row) if row.status == "ready_without_rendering_prompt" => {}
        Some(row) => {
            if status == "ready_to_execute_provider_cli" {
                status = "blocked_prompt_materialization_not_ready".to_string();
            }
            reasons.push(row.status.clone());
        }
        None => {
            if status == "ready_to_execute_provider_cli" {
                status = "blocked_missing_prompt_materialization".to_string();
            }
            reasons.push("recorded_prompt_materialization_missing".to_string());
        }
    }

    if !opt_in_set {
        if status == "ready_to_execute_provider_cli" {
            status = "blocked_missing_explicit_provider_opt_in".to_string();
        }
        reasons.push(format!("{opt_in_env}=1_required"));
    }
    if reasons.is_empty() {
        reasons.push("all_preflight_checks_passed".to_string());
    }
    let provider_cli_execution_allowed = status == "ready_to_execute_provider_cli";
    let next_action = if provider_cli_execution_allowed {
        "execute_provider_cli_through_runtime_runner"
    } else {
        "resolve_preflight_blockers_before_provider_execution"
    };

    AdapterDispatchRunPreflight {
        dispatch_plan_id: plan.dispatch_plan_id.clone(),
        adapter_kind: plan.adapter_kind.clone(),
        execution_request_id: execution_request
            .map(|request| request.execution_request_id.clone())
            .unwrap_or_else(|| "none".to_string()),
        materialization_id: materialization
            .map(|row| row.materialization_id.clone())
            .unwrap_or_else(|| "none".to_string()),
        provider_cli_execution_allowed,
        opt_in_env,
        opt_in_set,
        status,
        runtime_prompt_policy: plan.runtime_prompt_policy.clone(),
        raw_prompt_policy: materialization
            .map(|row| row.raw_prompt_policy.clone())
            .unwrap_or_else(|| "none".to_string()),
        reasons,
        next_action: next_action.to_string(),
    }
}

fn render_adapter_dispatch_run_preflight(preflight: &AdapterDispatchRunPreflight) -> String {
    format!(
        "adapter_dispatch_run_preflight=true\ndispatch_plan={}\nadapter={}\nexecution_request={}\nprompt_materialization={}\nprovider_cli_execution_allowed={}\nprovider_cli_executed=false\nopt_in_env={}\nopt_in_set={}\nstatus={}\nruntime_prompt_policy={}\nraw_prompt_policy={}\nreasons={}\nnext_action={}\n",
        preflight.dispatch_plan_id,
        preflight.adapter_kind,
        preflight.execution_request_id,
        preflight.materialization_id,
        preflight.provider_cli_execution_allowed,
        preflight.opt_in_env,
        preflight.opt_in_set,
        preflight.status,
        preflight.runtime_prompt_policy,
        preflight.raw_prompt_policy,
        preflight.reasons.join(","),
        preflight.next_action
    )
}

fn render_adapter_dispatch_run_local_blocked(
    preflight: &AdapterDispatchRunPreflight,
    recorded: bool,
    recorded_sequence: Option<i64>,
) -> String {
    format!(
        "adapter_dispatch_run_local=true\ndispatch_plan={}\nadapter={}\nexecution_request={}\nprompt_materialization={}\nprovider_cli_execution_allowed=false\nprovider_cli_executed=false\nopt_in_env={}\nopt_in_set={}\nstatus={}\nruntime_prompt_policy={}\nraw_prompt_policy={}\nreasons={}\nnext_action={}\nrecorded={}\nrecorded_sequence={}\n",
        preflight.dispatch_plan_id,
        preflight.adapter_kind,
        preflight.execution_request_id,
        preflight.materialization_id,
        preflight.opt_in_env,
        preflight.opt_in_set,
        preflight.status,
        preflight.runtime_prompt_policy,
        preflight.raw_prompt_policy,
        preflight.reasons.join(","),
        preflight.next_action,
        recorded,
        recorded_sequence
            .map(|sequence| sequence.to_string())
            .unwrap_or_else(|| "none".to_string())
    )
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AdapterDispatchExecutionRuntimeOutcome {
    provider_cli_executed: bool,
    status: String,
    exit_code: Option<i64>,
    runtime_process_ref: Option<String>,
    stdout_artifact_id: Option<String>,
    stderr_artifact_id: Option<String>,
    credential_scan_status: String,
    raw_output_policy: String,
    reason_codes: String,
}

impl AdapterDispatchExecutionRuntimeOutcome {
    fn blocked() -> Self {
        Self {
            provider_cli_executed: false,
            status: "blocked_by_preflight".to_string(),
            exit_code: None,
            runtime_process_ref: None,
            stdout_artifact_id: None,
            stderr_artifact_id: None,
            credential_scan_status: "not_run".to_string(),
            raw_output_policy: "not_captured".to_string(),
            reason_codes: "preflight_blocked_provider_execution".to_string(),
        }
    }
}

fn adapter_dispatch_execution_projection(
    plan: &AdapterDispatchPlanProjection,
    execution_request: Option<&AdapterDispatchExecutionRequestProjection>,
    preflight: &AdapterDispatchRunPreflight,
    outcome: AdapterDispatchExecutionRuntimeOutcome,
) -> AdapterDispatchExecutionProjection {
    let reason_codes = if outcome.provider_cli_executed {
        outcome.reason_codes
    } else {
        preflight.reasons.join(",")
    };
    AdapterDispatchExecutionProjection {
        dispatch_execution_id: format!(
            "adapter-dispatch-execution-{}-{}",
            stable_cli_hash(&plan.dispatch_plan_id),
            stable_cli_hash(&format!(
                "{}:{}:{}:{}:{}",
                preflight.status,
                outcome.provider_cli_executed,
                reason_codes,
                outcome.runtime_process_ref.as_deref().unwrap_or("none"),
                outcome.stdout_artifact_id.as_deref().unwrap_or("none")
            ))
        ),
        project_id: plan.project_id.clone(),
        dispatch_plan_id: plan.dispatch_plan_id.clone(),
        execution_request_id: execution_request
            .map(|request| request.execution_request_id.clone())
            .unwrap_or_else(|| "none".to_string()),
        adapter_kind: plan.adapter_kind.clone(),
        session_id: plan.session_id.clone(),
        run_id: plan.run_id.clone(),
        provider_cli_execution_allowed: preflight.provider_cli_execution_allowed,
        provider_cli_executed: outcome.provider_cli_executed,
        status: if outcome.provider_cli_executed {
            outcome.status
        } else {
            preflight.status.clone()
        },
        exit_code: outcome.exit_code,
        runtime_process_ref: outcome.runtime_process_ref,
        stdout_artifact_id: outcome.stdout_artifact_id,
        stderr_artifact_id: outcome.stderr_artifact_id,
        artifact_root: plan.artifact_root.clone(),
        credential_scan_status: outcome.credential_scan_status,
        raw_prompt_policy: preflight.raw_prompt_policy.clone(),
        raw_output_policy: outcome.raw_output_policy,
        reason_codes,
        updated_sequence: 0,
    }
}

fn record_adapter_dispatch_execution(
    state: &SqliteStateStore,
    plan: &AdapterDispatchPlanProjection,
    execution: &AdapterDispatchExecutionProjection,
) -> Result<i64, String> {
    let event = NewEvent {
        event_id: format!(
            "event-adapter-dispatch-execution-{}",
            stable_cli_hash(&execution.dispatch_execution_id)
        ),
        kind: EventKind::AdapterDispatchExecuted,
        actor: "local-cli".to_string(),
        project_id: Some(execution.project_id.clone()),
        task_id: None,
        agent_id: Some(plan.agent_id.clone()),
        session_id: Some(execution.session_id.clone()),
        run_id: Some(execution.run_id.clone()),
        turn_id: None,
        item_id: Some(execution.dispatch_execution_id.clone()),
        payload_json: format!(
            "{{\"dispatch_plan_id\":\"{}\",\"execution_request_id\":\"{}\",\"provider_cli_executed\":{},\"status\":\"{}\",\"raw_prompt_policy\":\"{}\",\"raw_output_policy\":\"{}\"}}",
            escape_json(&execution.dispatch_plan_id),
            escape_json(&execution.execution_request_id),
            execution.provider_cli_executed,
            escape_json(&execution.status),
            escape_json(&execution.raw_prompt_policy),
            escape_json(&execution.raw_output_policy)
        ),
        idempotency_key: Some(format!(
            "adapter-dispatch-execution:{}:{}:{}:{}:{}",
            execution.project_id,
            execution.dispatch_plan_id,
            execution.execution_request_id,
            execution.status,
            execution.dispatch_execution_id
        )),
        redaction_state: RedactionState::Safe,
    };
    state
        .append_event(
            event,
            &[ProjectionRecord::AdapterDispatchExecution(
                execution.clone(),
            )],
        )
        .map_err(debug_error)
}

fn build_dispatch_run_launch_plan(
    state: &SqliteStateStore,
    plan: &AdapterDispatchPlanProjection,
    source: &AdapterDispatchPromptSourceProjection,
    materialization: Option<&AdapterDispatchPromptMaterializationProjection>,
) -> Result<LocalAdapterLaunchPlan, String> {
    if source.source_kind != "workpad_task" {
        return Err(format!(
            "dispatch prompt source is not replayable for local run: {}",
            source.source_kind
        ));
    }
    let materialization = materialization
        .filter(|row| row.status == "ready_without_rendering_prompt")
        .ok_or_else(|| "dispatch prompt is not materialized for local run".to_string())?;
    let source_ref = source
        .source_ref
        .as_deref()
        .ok_or_else(|| "workpad prompt source missing source_ref".to_string())?;
    let (path, anchor) = split_source_ref(source_ref)?;
    let task = state
        .workpad_tasks(&source.project_id)
        .map_err(debug_error)?
        .into_iter()
        .find(|task| task.path == path && task.source_anchor == anchor)
        .ok_or_else(|| format!("workpad prompt source is missing: {source_ref}"))?;
    let prompt = workpad_task_goal(&task);
    let prompt_hash = stable_cli_hash(&prompt);
    if materialization.materialized_prompt_hash.as_deref() != Some(prompt_hash.as_str()) {
        return Err("dispatch prompt materialization no longer matches source".to_string());
    }
    let workspace_root = PathBuf::from(&plan.runtime_cwd);
    let artifact_root = PathBuf::from(&plan.artifact_root);
    match plan.adapter_kind.as_str() {
        "codex_exec" => Ok(CodexExecAdapter::local_launch_plan(
            workspace_root,
            artifact_root,
            prompt,
        )),
        "claude_code" => Ok(ClaudeCodeAdapter::local_launch_plan(
            workspace_root,
            artifact_root,
            prompt,
        )),
        other => Err(format!("unsupported adapter for local run: {other}")),
    }
}

fn adapter_dispatch_prompt_materialization_projection(
    state: &SqliteStateStore,
    source: &AdapterDispatchPromptSourceProjection,
) -> Result<AdapterDispatchPromptMaterializationProjection, String> {
    let mut observed_source_hash = None;
    let mut materialized_prompt_hash = None;
    let mut status = "blocked_non_replayable_prompt".to_string();
    let mut reasons = vec!["manual_prompt_not_replayable".to_string()];

    if source.source_kind == "workpad_task" {
        reasons.clear();
        let source_ref = source
            .source_ref
            .as_deref()
            .ok_or_else(|| "workpad prompt source missing source_ref".to_string())?;
        let (path, anchor) = split_source_ref(source_ref)?;
        let workpad_file = state
            .workpad_file(&source.project_id, path)
            .map_err(debug_error)?;
        let workpad_file = match workpad_file {
            Some(file) => file,
            None => {
                status = "blocked_missing_source".to_string();
                reasons.push("workpad_file_missing".to_string());
                return Ok(prompt_materialization_row(
                    source,
                    observed_source_hash,
                    materialized_prompt_hash,
                    status,
                    reasons,
                ));
            }
        };
        observed_source_hash = Some(workpad_file.content_hash.clone());
        if source.source_hash.as_deref() != Some(workpad_file.content_hash.as_str()) {
            status = "blocked_source_hash_mismatch".to_string();
            reasons.push("workpad_source_hash_mismatch".to_string());
            return Ok(prompt_materialization_row(
                source,
                observed_source_hash,
                materialized_prompt_hash,
                status,
                reasons,
            ));
        }
        let task = state
            .workpad_tasks(&source.project_id)
            .map_err(debug_error)?
            .into_iter()
            .find(|task| task.path == path && task.source_anchor == anchor);
        let Some(task) = task else {
            status = "blocked_missing_source".to_string();
            reasons.push("workpad_task_missing".to_string());
            return Ok(prompt_materialization_row(
                source,
                observed_source_hash,
                materialized_prompt_hash,
                status,
                reasons,
            ));
        };
        let prompt_hash = stable_cli_hash(&workpad_task_goal(&task));
        materialized_prompt_hash = Some(prompt_hash.clone());
        if prompt_hash == source.prompt_hash {
            status = "ready_without_rendering_prompt".to_string();
            reasons.push("prompt_hash_matches_source".to_string());
        } else {
            status = "blocked_prompt_hash_mismatch".to_string();
            reasons.push("prompt_hash_mismatch".to_string());
        }
    }

    Ok(prompt_materialization_row(
        source,
        observed_source_hash,
        materialized_prompt_hash,
        status,
        reasons,
    ))
}

fn prompt_materialization_row(
    source: &AdapterDispatchPromptSourceProjection,
    observed_source_hash: Option<String>,
    materialized_prompt_hash: Option<String>,
    status: String,
    reasons: Vec<String>,
) -> AdapterDispatchPromptMaterializationProjection {
    AdapterDispatchPromptMaterializationProjection {
        materialization_id: format!(
            "adapter-dispatch-prompt-materialization-{}-{}",
            stable_cli_hash(&source.prompt_source_id),
            stable_cli_hash(&format!(
                "{}:{}",
                status,
                observed_source_hash.as_deref().unwrap_or("none")
            ))
        ),
        project_id: source.project_id.clone(),
        dispatch_plan_id: source.dispatch_plan_id.clone(),
        prompt_source_id: source.prompt_source_id.clone(),
        source_kind: source.source_kind.clone(),
        source_ref: source.source_ref.clone(),
        expected_source_hash: source.source_hash.clone(),
        observed_source_hash,
        expected_prompt_hash: source.prompt_hash.clone(),
        materialized_prompt_hash,
        status,
        raw_prompt_policy: "not_rendered".to_string(),
        reason_codes: reasons.join(","),
        updated_sequence: 0,
    }
}

fn split_source_ref(source_ref: &str) -> Result<(&str, &str), String> {
    source_ref
        .split_once('#')
        .ok_or_else(|| format!("invalid prompt source ref: {source_ref}"))
}

fn adapter_dispatch_execution_request_projection(
    plan: &AdapterDispatchPlanProjection,
    latest_gate: Option<&AdapterDispatchGateProjection>,
) -> AdapterDispatchExecutionRequestProjection {
    let ready_gate = latest_gate.filter(|gate| {
        gate.provider_cli_execution_allowed
            && gate.status == "ready_for_execution"
            && !gate.provider_cli_executed
            && gate.runtime_prompt_policy == "not_rendered"
    });
    let opt_in_env = adapter_dispatch_opt_in_env(&plan.adapter_kind);
    let (dispatch_gate_id, allowed, status, reason_codes) = if let Some(gate) = ready_gate {
        (
            gate.dispatch_gate_id.clone(),
            true,
            "waiting_on_explicit_provider_opt_in".to_string(),
            "explicit_provider_execution_opt_in_required".to_string(),
        )
    } else if let Some(gate) = latest_gate {
        (
            gate.dispatch_gate_id.clone(),
            false,
            "blocked_by_dispatch_gate".to_string(),
            gate.reason_codes.clone(),
        )
    } else {
        (
            "none".to_string(),
            false,
            "blocked_missing_ready_gate".to_string(),
            "recorded_ready_dispatch_gate_missing".to_string(),
        )
    };

    AdapterDispatchExecutionRequestProjection {
        execution_request_id: format!(
            "adapter-dispatch-execution-request-{}-{}",
            stable_cli_hash(&plan.dispatch_plan_id),
            stable_cli_hash(&format!("{status}:{dispatch_gate_id}"))
        ),
        project_id: plan.project_id.clone(),
        dispatch_plan_id: plan.dispatch_plan_id.clone(),
        dispatch_gate_id,
        adapter_kind: plan.adapter_kind.clone(),
        provider_cli_execution_allowed: allowed,
        provider_cli_executed: false,
        status,
        opt_in_env,
        runtime_prompt_policy: plan.runtime_prompt_policy.clone(),
        reason_codes,
        updated_sequence: 0,
    }
}

fn adapter_dispatch_opt_in_env(adapter_kind: &str) -> String {
    match adapter_kind {
        "codex_exec" => "CAPO_RUN_CODEX_LOCAL_DISPATCH",
        "claude_code" => "CAPO_RUN_CLAUDE_LOCAL_DISPATCH",
        _ => "CAPO_RUN_LOCAL_ADAPTER_DISPATCH",
    }
    .to_string()
}

fn adapter_dispatch_gate_projection(
    plan: &AdapterDispatchPlanProjection,
    dogfood_gate: &AdapterDogfoodGate,
) -> AdapterDispatchGateProjection {
    let adapter_proven = dogfood_gate
        .proven_adapters
        .iter()
        .any(|adapter| adapter == &plan.adapter_kind);
    let execution_allowed = dogfood_gate.ready
        && adapter_proven
        && plan.status == "planned"
        && plan.runtime_prompt_policy == "not_rendered"
        && !plan.provider_cli_executed;
    let mut reasons = Vec::new();
    if !dogfood_gate.ready {
        reasons.extend(dogfood_gate.reasons.iter().cloned());
    }
    if dogfood_gate.ready && !adapter_proven {
        reasons.push(format!(
            "{}:real_subscription_smoke_not_recorded",
            plan.adapter_kind
        ));
    }
    if plan.status != "planned" {
        reasons.push(format!("dispatch_plan_status:{}", plan.status));
    }
    if plan.runtime_prompt_policy != "not_rendered" {
        reasons.push("runtime_prompt_policy_not_redacted".to_string());
    }
    if plan.provider_cli_executed {
        reasons.push("provider_cli_already_executed".to_string());
    }
    if reasons.is_empty() {
        reasons.push("required_real_smoke_evidence_recorded".to_string());
    }
    let status = if execution_allowed {
        "ready_for_execution"
    } else {
        "blocked"
    }
    .to_string();
    AdapterDispatchGateProjection {
        dispatch_gate_id: format!(
            "adapter-dispatch-gate-{}-{}",
            stable_cli_hash(&plan.dispatch_plan_id),
            stable_cli_hash(&reasons.join(","))
        ),
        project_id: plan.project_id.clone(),
        dispatch_plan_id: plan.dispatch_plan_id.clone(),
        adapter_kind: plan.adapter_kind.clone(),
        provider_cli_execution_allowed: execution_allowed,
        status,
        required_dogfood_gate: dogfood_gate.status.clone(),
        reason_codes: reasons.join(","),
        provider_cli_executed: plan.provider_cli_executed,
        runtime_prompt_policy: plan.runtime_prompt_policy.clone(),
        updated_sequence: 0,
    }
}

fn render_adapter_dispatch_gate(
    gate: &AdapterDispatchGateProjection,
    recorded: bool,
    recorded_sequence: Option<i64>,
) -> String {
    format!(
        "adapter_dispatch_gate=true\ndispatch_gate={}\ndispatch_plan={}\nadapter={}\nprovider_cli_execution_allowed={}\nstatus={}\nrequired_dogfood_gate={}\nprovider_cli_executed={}\nruntime_prompt_policy={}\nreasons={}\nrecorded={}\nrecorded_sequence={}\n",
        gate.dispatch_gate_id,
        gate.dispatch_plan_id,
        gate.adapter_kind,
        gate.provider_cli_execution_allowed,
        gate.status,
        gate.required_dogfood_gate,
        gate.provider_cli_executed,
        gate.runtime_prompt_policy,
        gate.reason_codes,
        recorded,
        recorded_sequence
            .map(|sequence| sequence.to_string())
            .unwrap_or_else(|| "none".to_string())
    )
}

fn render_adapter_dispatch_status(status: &AdapterDispatchStatus) -> String {
    format!(
        "adapter_dispatch_status=true\ndispatch_plan={}\nadapter={}\nagent={}\nsession_id={}\nrun_id={}\nplan_status={}\nprovider_kind={}\ncredential_scope={}\nruntime_program={}\nruntime_arg_count={}\nruntime_prompt_policy={}\nprovider_cli_executed={}\ndogfood_gate={}\nlatest_dispatch_gate={}\nlatest_gate_status={}\nlatest_gate_provider_cli_execution_allowed={}\nlatest_gate_reasons={}\nlatest_dispatch_replay={}\nlatest_replay_appended_events={}\nlatest_replay_raw_content_policy={}\nlatest_dispatch_execution={}\nlatest_execution_status={}\nlatest_execution_provider_cli_execution_allowed={}\nlatest_execution_provider_cli_executed={}\nlatest_execution_credential_scan_status={}\nlatest_execution_stdout_artifact={}\nlatest_execution_stderr_artifact={}\nlatest_execution_reasons={}\nnext_action={}\n",
        status.dispatch_plan_id,
        status.adapter_kind,
        status.agent_name,
        status.session_id,
        status.run_id,
        status.plan_status,
        status.provider_kind,
        status.credential_scope,
        status.runtime_program,
        status.runtime_arg_count,
        status.runtime_prompt_policy,
        status.provider_cli_executed,
        status.dogfood_gate_status,
        status.latest_dispatch_gate_id,
        status.latest_gate_status,
        status.latest_gate_provider_cli_execution_allowed,
        status.latest_gate_reasons,
        status.latest_dispatch_replay_id,
        status.latest_replay_appended_events,
        status.latest_replay_raw_content_policy,
        status.latest_dispatch_execution_id,
        status.latest_execution_status,
        status.latest_execution_provider_cli_execution_allowed,
        status.latest_execution_provider_cli_executed,
        status.latest_execution_credential_scan_status,
        status.latest_execution_stdout_artifact_id,
        status.latest_execution_stderr_artifact_id,
        status.latest_execution_reasons,
        status.next_action
    )
}

fn render_adapter_dispatch_execution_request(
    request: &AdapterDispatchExecutionRequestProjection,
    recorded: bool,
    recorded_sequence: Option<i64>,
) -> String {
    format!(
        "adapter_dispatch_execution_request=true\nexecution_request={}\ndispatch_plan={}\ndispatch_gate={}\nadapter={}\nprovider_cli_execution_allowed={}\nprovider_cli_executed={}\nstatus={}\nopt_in_env={}\nruntime_prompt_policy={}\nreasons={}\nrecorded={}\nrecorded_sequence={}\n",
        request.execution_request_id,
        request.dispatch_plan_id,
        request.dispatch_gate_id,
        request.adapter_kind,
        request.provider_cli_execution_allowed,
        request.provider_cli_executed,
        request.status,
        request.opt_in_env,
        request.runtime_prompt_policy,
        request.reason_codes,
        recorded,
        recorded_sequence
            .map(|sequence| sequence.to_string())
            .unwrap_or_else(|| "none".to_string())
    )
}

fn render_adapter_dispatch_prompt_materialization(
    materialization: &AdapterDispatchPromptMaterializationProjection,
    recorded: bool,
    recorded_sequence: Option<i64>,
) -> String {
    format!(
        "adapter_dispatch_prompt_materialization=true\nmaterialization={}\ndispatch_plan={}\nprompt_source={}\nsource_kind={}\nsource_ref={}\nexpected_source_hash={}\nobserved_source_hash={}\nexpected_prompt_hash={}\nmaterialized_prompt_hash={}\nstatus={}\nraw_prompt_policy={}\nreasons={}\nrecorded={}\nrecorded_sequence={}\n",
        materialization.materialization_id,
        materialization.dispatch_plan_id,
        materialization.prompt_source_id,
        materialization.source_kind,
        materialization.source_ref.as_deref().unwrap_or("none"),
        materialization
            .expected_source_hash
            .as_deref()
            .unwrap_or("none"),
        materialization
            .observed_source_hash
            .as_deref()
            .unwrap_or("none"),
        materialization.expected_prompt_hash,
        materialization
            .materialized_prompt_hash
            .as_deref()
            .unwrap_or("none"),
        materialization.status,
        materialization.raw_prompt_policy,
        materialization.reason_codes,
        recorded,
        recorded_sequence
            .map(|sequence| sequence.to_string())
            .unwrap_or_else(|| "none".to_string())
    )
}

fn render_adapter_dogfood_gate(gate: &AdapterDogfoodGate) -> String {
    format!(
        "adapter_dogfood_gate=true\nready_for_first_real_agent_dogfood={}\nstatus={}\nrequired_adapters={}\nproven_adapters={}\nblocked_adapters={}\nreasons={}\n",
        gate.ready,
        gate.status,
        comma_or_none(&gate.required_adapters),
        comma_or_none(&gate.proven_adapters),
        comma_or_none(&gate.blocked_adapters),
        comma_or_none(&gate.reasons)
    )
}

fn render_dogfood_readiness(readiness: &ProjectDogfoodReadiness) -> String {
    format!(
        "dogfood_readiness=true\nready={}\nstatus={}\nreal_agent_connector_ready={}\nworkpad_bridge_ready={}\ndispatch_chain_ready={}\nworkpad_tasks={}\nworkpad_tasks_observed_only={}\nworkpad_tasks_imported={}\ndispatch_plans={}\nready_dispatch_gates={}\ndispatch_replays={}\ndispatch_executions={}\nblockers={}\nnext_actions={}\n",
        readiness.ready,
        readiness.status,
        readiness.real_agent_connector_ready,
        readiness.workpad_bridge_ready,
        readiness.dispatch_chain_ready,
        readiness.workpad_task_count,
        readiness.observed_workpad_task_count,
        readiness.imported_workpad_task_count,
        readiness.dispatch_plan_count,
        readiness.ready_dispatch_gate_count,
        readiness.dispatch_replay_count,
        readiness.dispatch_execution_count,
        readiness.blockers.join(","),
        readiness.next_actions.join(",")
    )
}

fn dogfood_readiness_confidence(readiness: &ProjectDogfoodReadiness) -> i64 {
    if readiness.ready { 90 } else { 65 }
}

fn render_dogfood_readiness_evidence(
    project_id: &ProjectId,
    readiness: &ProjectDogfoodReadiness,
) -> String {
    format!(
        "<!-- capo:dogfood-readiness -->\n# Capo Dogfood Readiness - {}\n\n## Objective\n\nReview whether Capo is ready to move its own project workpads into Capo-managed dogfood.\n\n## Summary\n\n- Project: `{}`\n- Ready: `{}`\n- Status: `{}`\n- Real-agent connector ready: `{}`\n- Workpad bridge ready: `{}`\n- Dispatch chain ready: `{}`\n\n## Counts\n\n- Workpad tasks: `{}`\n- Observed-only workpad tasks: `{}`\n- Imported workpad tasks: `{}`\n- Dispatch plans: `{}`\n- Ready dispatch gates: `{}`\n- Dispatch replays: `{}`\n- Dispatch executions: `{}`\n\n## Blockers\n\n{}\n\n## Next Actions\n\n{}\n\n## Evidence Policy\n\n- This report is derived from persisted Capo read models only.\n- It does not run provider CLIs, inspect credentials, materialize prompts, open tunnels, or edit markdown.\n- Raw prompts, raw provider output, credentials, cookies, and subscription session material are not rendered.\n",
        readiness.status,
        project_id,
        readiness.ready,
        readiness.status,
        readiness.real_agent_connector_ready,
        readiness.workpad_bridge_ready,
        readiness.dispatch_chain_ready,
        readiness.workpad_task_count,
        readiness.observed_workpad_task_count,
        readiness.imported_workpad_task_count,
        readiness.dispatch_plan_count,
        readiness.ready_dispatch_gate_count,
        readiness.dispatch_replay_count,
        readiness.dispatch_execution_count,
        markdown_list_or_none(&readiness.blockers),
        markdown_list_or_none(&readiness.next_actions)
    )
}

fn dashboard(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let query = dashboard_query(args)?;
    let command = envelope(
        "dashboard",
        CommandTarget::Project(query.project_id.clone()),
        CommandIntent::QueryStatus,
        None,
    );
    let command = CommandEnvelope {
        project_id: query.project_id.clone(),
        ..command
    };
    let state = state(parsed)?;
    let dashboard = project_dashboard(&state, query).map_err(debug_error)?;
    Ok(render_dashboard(&command, &dashboard))
}

fn dashboard_query(args: &[String]) -> Result<ProjectDashboardQuery, String> {
    let mut project_id = project_id();
    let mut session_id = None;
    let mut status = None;
    let mut workpad_path = None;
    let mut workpad_status = None;
    let mut index = 0;
    while index < args.len() {
        let key = args[index].as_str();
        let value = args
            .get(index + 1)
            .filter(|value| !value.starts_with("--"))
            .ok_or_else(|| format!("{key} requires a value"))?;
        match key {
            "--project" => project_id = ProjectId::new(value),
            "--session" => session_id = Some(SessionId::new(value)),
            "--status" => status = Some(value.clone()),
            "--workpad-path" => workpad_path = Some(value.clone()),
            "--workpad-status" => workpad_status = Some(value.clone()),
            other => return Err(format!("unknown dashboard filter: {other}")),
        }
        index += 2;
    }
    let mut query = ProjectDashboardQuery::new(project_id);
    if let Some(session_id) = session_id {
        query = query.with_session_id(session_id);
    }
    if let Some(status) = status {
        query = query.with_status(status);
    }
    if let Some(workpad_path) = workpad_path {
        query = query.with_workpad_path(workpad_path);
    }
    if let Some(workpad_status) = workpad_status {
        query = query.with_workpad_status(workpad_status);
    }
    Ok(query)
}

fn render_dashboard(command: &CommandEnvelope, dashboard: &ProjectDashboard) -> String {
    let dogfood_readiness = dashboard.dogfood_readiness();
    let mut output = format!(
        "command_id={}\nview=dashboard\nagents={}\n",
        command.command_id,
        dashboard.agents.len()
    );

    for row in &dashboard.agents {
        let agent = &row.agent;
        output.push_str(&format!(
            "agent={} agent_status={} current_session={}\n",
            agent.name,
            agent.status,
            agent
                .current_session_id
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| "none".to_string())
        ));

        let Some(session_row) = &row.session else {
            continue;
        };
        let session = &session_row.session;

        output.push_str(&format!(
            "session={} session_status={} run={} run_status={} goal={} blocker={} confidence={} evidence_refs={} tool_calls={} memory_packet_refs={} recent_events={}\n",
            session.session_id,
            session.status,
            session_row
                .run
                .as_ref()
                .map(|item| item.run_id.to_string())
                .unwrap_or_else(|| "none".to_string()),
            session_row
                .run
                .as_ref()
                .map(|item| item.status.clone())
                .unwrap_or_else(|| "none".to_string()),
            session.current_goal,
            session.latest_blocker.as_deref().unwrap_or("none"),
            session
                .latest_confidence
                .map(|value| value.to_string())
                .unwrap_or_else(|| "none".to_string()),
            session_row
                .evidence
                .iter()
                .map(|item| item.evidence_id.to_string())
                .collect::<Vec<_>>()
                .join(","),
            session_row.tool_calls.len(),
            session_row.memory_packets.len(),
            session_row.recent_events.len()
        ));
        output.push_str(&format!(
            "session_review_findings={}\n",
            session_row.review_findings.len()
        ));
        for finding in &session_row.review_findings {
            output.push_str(&format!(
                "review_finding={} session={} kind={} severity={} status={} reviewer={} evidence_artifact={} follow_up={} summary={}\n",
                finding.review_finding_id,
                finding.session_id,
                finding.finding_kind,
                finding.severity,
                finding.status,
                finding.reviewer,
                finding.evidence_artifact_id.as_deref().unwrap_or("none"),
                finding.follow_up.as_deref().unwrap_or("none"),
                finding.summary
            ));
        }
        output.push_str(&format!(
            "session_task_outcome_reports={}\n",
            session_row.task_outcome_reports.len()
        ));
        for report in &session_row.task_outcome_reports {
            output.push_str(&format!(
                "task_outcome_report={} session={} task={} run={} outcome_status={} review_outcome={} actions={} tool_calls={} evidence={} memory_packets={} confidence={} blocker={} artifact={}\n",
                report.task_outcome_report_id,
                report.session_id,
                report.task_id,
                report.run_id,
                report.outcome_status,
                report.review_outcome,
                report.action_count,
                report.tool_call_count,
                report.evidence_count,
                report.memory_packet_count,
                report
                    .confidence
                    .map(|confidence| confidence.to_string())
                    .unwrap_or_else(|| "none".to_string()),
                report.blocker.as_deref().unwrap_or("none"),
                report.report_artifact_id.as_deref().unwrap_or("none")
            ));
        }
        for tool_call in &session_row.tool_calls {
            output.push_str(&format!(
                "tool_call={} tool={} tool_origin={} tool_status={} input_artifact={} output_artifact={}\n",
                tool_call.tool_call_id,
                tool_call.tool_name,
                tool_call.tool_origin,
                tool_call.status,
                tool_call.input_artifact_id.as_deref().unwrap_or("none"),
                tool_call.output_artifact_id.as_deref().unwrap_or("none")
            ));
        }
        for packet in &session_row.memory_packets {
            output.push_str(&format!(
                "memory_packet={} purpose={} artifact={}\n",
                packet.memory_packet_id,
                packet.purpose,
                packet.packet_artifact_id.as_deref().unwrap_or("none")
            ));
        }
        for event in &session_row.recent_events {
            output.push_str(&format!("event={} kind={}\n", event.sequence, event.kind));
        }
    }

    output.push_str(&format!(
        "project_evidence={}\n",
        dashboard.project_evidence.len()
    ));
    for evidence in &dashboard.project_evidence {
        output.push_str(&format!(
            "project_evidence_ref={} kind={} artifact={} confidence={}\n",
            evidence.evidence_id,
            evidence.kind,
            evidence.artifact_id.as_deref().unwrap_or("none"),
            evidence.confidence
        ));
    }

    output.push_str(&format!(
        "review_findings={}\n",
        dashboard.review_findings.len()
    ));
    for finding in &dashboard.review_findings {
        output.push_str(&format!(
            "project_review_finding={} session={} kind={} severity={} status={} reviewer={} evidence_artifact={} follow_up={} summary={}\n",
            finding.review_finding_id,
            finding.session_id,
            finding.finding_kind,
            finding.severity,
            finding.status,
            finding.reviewer,
            finding.evidence_artifact_id.as_deref().unwrap_or("none"),
            finding.follow_up.as_deref().unwrap_or("none"),
            finding.summary
        ));
    }

    output.push_str(&format!(
        "task_outcome_reports={}\n",
        dashboard.task_outcome_reports.len()
    ));
    for report in &dashboard.task_outcome_reports {
        output.push_str(&format!(
            "project_task_outcome_report={} session={} task={} run={} outcome_status={} review_outcome={} actions={} tool_calls={} evidence={} memory_packets={} confidence={} blocker={} artifact={}\n",
            report.task_outcome_report_id,
            report.session_id,
            report.task_id,
            report.run_id,
            report.outcome_status,
            report.review_outcome,
            report.action_count,
            report.tool_call_count,
            report.evidence_count,
            report.memory_packet_count,
            report
                .confidence
                .map(|confidence| confidence.to_string())
                .unwrap_or_else(|| "none".to_string()),
            report.blocker.as_deref().unwrap_or("none"),
            report.report_artifact_id.as_deref().unwrap_or("none")
        ));
    }

    output.push_str(&format!(
        "connectivity_exposures={}\n",
        dashboard.connectivity_exposures.len()
    ));
    for exposure in &dashboard.connectivity_exposures {
        output.push_str(&format!(
            "connectivity_exposure={} endpoint={} owner={}:{} channel={} exposure={} exposure_status={} health={} reachable={} permission_scope={} grant={} revoked_at={}\n",
            exposure.exposure_id,
            exposure.connectivity_endpoint_id,
            exposure.owner_kind,
            exposure.owner_id,
            exposure.channel_kind,
            exposure.exposure,
            exposure.status,
            exposure.health_status,
            exposure.reachable,
            exposure.permission_scope,
            exposure.capability_grant_id.as_deref().unwrap_or("none"),
            exposure.revoked_at.as_deref().unwrap_or("none")
        ));
    }
    output.push_str(&format!(
        "adapter_readiness={}\n",
        dashboard.adapter_readiness.len()
    ));
    for readiness in &dashboard.adapter_readiness {
        output.push_str(&format!(
            "adapter_readiness_row={} program={} opt_in_env={} opted_in={} smoke_status={} credential_policy={} expected_marker={} env_allowlist={} redaction_rules={} output_limit_bytes={} dogfood_blocker={}\n",
            readiness.adapter_kind,
            readiness.program,
            readiness.opt_in_env,
            readiness.opted_in,
            readiness.smoke_status,
            readiness.credential_policy,
            readiness.expected_marker,
            readiness.env_allowlist_count,
            readiness.redaction_rule_count,
            readiness.output_limit_bytes,
            readiness.dogfood_blocker.as_deref().unwrap_or("none")
        ));
    }
    output.push_str(&format!(
        "adapter_smoke_reports={}\n",
        dashboard.adapter_smoke_reports.len()
    ));
    for report in &dashboard.adapter_smoke_reports {
        output.push_str(&format!(
            "adapter_smoke_report={} adapter={} smoke_status={} credential_scan_status={} marker_found={} dogfood_readiness_effect={} artifact_root={} reason={}\n",
            report.smoke_report_id,
            report.adapter_kind,
            report.smoke_status,
            report.credential_scan_status,
            report.marker_found,
            report.dogfood_readiness_effect,
            report.artifact_root.as_deref().unwrap_or("none"),
            report.reason
        ));
    }
    output.push_str(&format!(
        "adapter_dispatch_plans={}\n",
        dashboard.adapter_dispatch_plans.len()
    ));
    for plan in &dashboard.adapter_dispatch_plans {
        output.push_str(&format!(
            "adapter_dispatch_plan={} adapter={} provider_kind={} credential_scope={} agent={} session={} run={} runtime_program={} runtime_arg_count={} runtime_prompt_policy={} runtime_cwd={} artifact_root={} provider_cli_executed={} status={}\n",
            plan.dispatch_plan_id,
            plan.adapter_kind,
            plan.provider_kind,
            plan.credential_scope,
            plan.agent_name,
            plan.session_id,
            plan.run_id,
            plan.runtime_program,
            plan.runtime_arg_count,
            plan.runtime_prompt_policy,
            plan.runtime_cwd,
            plan.artifact_root,
            plan.provider_cli_executed,
            plan.status
        ));
    }
    output.push_str(&format!(
        "adapter_dispatch_gates={}\n",
        dashboard.adapter_dispatch_gates.len()
    ));
    for gate in &dashboard.adapter_dispatch_gates {
        output.push_str(&format!(
            "adapter_dispatch_gate={} dispatch_plan={} adapter={} provider_cli_execution_allowed={} gate_status={} required_dogfood_gate={} provider_cli_executed={} runtime_prompt_policy={} reasons={}\n",
            gate.dispatch_gate_id,
            gate.dispatch_plan_id,
            gate.adapter_kind,
            gate.provider_cli_execution_allowed,
            gate.status,
            gate.required_dogfood_gate,
            gate.provider_cli_executed,
            gate.runtime_prompt_policy,
            gate.reason_codes
        ));
    }
    output.push_str(&format!(
        "adapter_dispatch_replays={}\n",
        dashboard.adapter_dispatch_replays.len()
    ));
    for replay in &dashboard.adapter_dispatch_replays {
        output.push_str(&format!(
            "adapter_dispatch_replay={} dispatch_plan={} dispatch_gate={} adapter={} session={} run={} fixture_hash={} input_events={} appended_events={} tool_events={} summary_events={} completed_turns={} provider_cli_executed={} raw_content_policy={}\n",
            replay.dispatch_replay_id,
            replay.dispatch_plan_id,
            replay.dispatch_gate_id,
            replay.adapter_kind,
            replay.session_id,
            replay.run_id,
            replay.fixture_hash,
            replay.input_event_count,
            replay.appended_event_count,
            replay.tool_event_count,
            replay.summary_event_count,
            replay.completed_turn_count,
            replay.provider_cli_executed,
            replay.raw_content_policy
        ));
    }
    output.push_str(&format!(
        "adapter_dispatch_execution_requests={}\n",
        dashboard.adapter_dispatch_execution_requests.len()
    ));
    for request in &dashboard.adapter_dispatch_execution_requests {
        output.push_str(&format!(
            "adapter_dispatch_execution_request={} dispatch_plan={} dispatch_gate={} adapter={} execution_status={} provider_cli_execution_allowed={} provider_cli_executed={} opt_in_env={} runtime_prompt_policy={} reasons={}\n",
            request.execution_request_id,
            request.dispatch_plan_id,
            request.dispatch_gate_id,
            request.adapter_kind,
            request.status,
            request.provider_cli_execution_allowed,
            request.provider_cli_executed,
            request.opt_in_env,
            request.runtime_prompt_policy,
            request.reason_codes
        ));
    }
    output.push_str(&format!(
        "adapter_dispatch_executions={}\n",
        dashboard.adapter_dispatch_executions.len()
    ));
    for execution in &dashboard.adapter_dispatch_executions {
        output.push_str(&format!(
            "adapter_dispatch_execution={} dispatch_plan={} execution_request={} adapter={} session={} run={} execution_status={} provider_cli_execution_allowed={} provider_cli_executed={} exit_code={} runtime_process_ref={} stdout_artifact={} stderr_artifact={} artifact_root={} credential_scan_status={} raw_prompt_policy={} raw_output_policy={} reasons={}\n",
            execution.dispatch_execution_id,
            execution.dispatch_plan_id,
            execution.execution_request_id,
            execution.adapter_kind,
            execution.session_id,
            execution.run_id,
            execution.status,
            execution.provider_cli_execution_allowed,
            execution.provider_cli_executed,
            execution
                .exit_code
                .map(|code| code.to_string())
                .unwrap_or_else(|| "none".to_string()),
            execution.runtime_process_ref.as_deref().unwrap_or("none"),
            execution.stdout_artifact_id.as_deref().unwrap_or("none"),
            execution.stderr_artifact_id.as_deref().unwrap_or("none"),
            execution.artifact_root,
            execution.credential_scan_status,
            execution.raw_prompt_policy,
            execution.raw_output_policy,
            execution.reason_codes
        ));
    }
    output.push_str(&format!(
        "adapter_dispatch_prompt_sources={}\n",
        dashboard.adapter_dispatch_prompt_sources.len()
    ));
    for source in &dashboard.adapter_dispatch_prompt_sources {
        output.push_str(&format!(
            "adapter_dispatch_prompt_source={} dispatch_plan={} source_kind={} source_ref={} source_hash={} materialization_status={} raw_prompt_policy={}\n",
            source.prompt_source_id,
            source.dispatch_plan_id,
            source.source_kind,
            source.source_ref.as_deref().unwrap_or("none"),
            source.source_hash.as_deref().unwrap_or("none"),
            source.materialization_status,
            source.raw_prompt_policy
        ));
    }
    output.push_str(&format!(
        "adapter_dispatch_prompt_materializations={}\n",
        dashboard.adapter_dispatch_prompt_materializations.len()
    ));
    for materialization in &dashboard.adapter_dispatch_prompt_materializations {
        output.push_str(&format!(
            "adapter_dispatch_prompt_materialization={} dispatch_plan={} prompt_source={} source_kind={} status={} raw_prompt_policy={} reasons={}\n",
            materialization.materialization_id,
            materialization.dispatch_plan_id,
            materialization.prompt_source_id,
            materialization.source_kind,
            materialization.status,
            materialization.raw_prompt_policy,
            materialization.reason_codes
        ));
    }
    output.push_str(&render_adapter_dogfood_gate(
        &dashboard.adapter_dogfood_gate,
    ));
    output.push_str(&format!(
        "project_dogfood_readiness={} status={} real_agent_connector_ready={} workpad_bridge_ready={} dispatch_chain_ready={} blockers={} next_actions={}\n",
        dogfood_readiness.ready,
        dogfood_readiness.status,
        dogfood_readiness.real_agent_connector_ready,
        dogfood_readiness.workpad_bridge_ready,
        dogfood_readiness.dispatch_chain_ready,
        comma_or_none(&dogfood_readiness.blockers),
        comma_or_none(&dogfood_readiness.next_actions)
    ));
    output.push_str(&format!(
        "workpad_tasks={}\n",
        dashboard.workpad_tasks.len()
    ));
    for task in &dashboard.workpad_tasks {
        output.push_str(&format!(
            "workpad_task={} path={} source_anchor={} observed_status={} capo_execution_status={} default_task_id={}\n",
            task.workpad_task_id,
            task.path,
            task.source_anchor,
            task.observed_status,
            task.capo_execution_status,
            default_workpad_task_id(&task.workpad_task_id)
        ));
    }

    output.push_str(&format!(
        "active_sessions={}\n",
        dashboard.active_session_count()
    ));
    output
}

fn send_task(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let agent = required_arg(args, "--agent")?;
    let goal = required_arg(args, "--goal")?;
    let scenario = optional_arg(args, "--scenario").unwrap_or_else(|| "default".to_string());
    let mut command = envelope(
        "task-send",
        CommandTarget::Agent(AgentId::new(format!("agent-{agent}"))),
        CommandIntent::SendTask,
        Some(goal.clone()),
    );
    command
        .structured_args
        .push(("agent".to_string(), agent.clone()));
    command
        .structured_args
        .push(("scenario".to_string(), scenario.clone()));
    let refs = controller(parsed)?
        .send_task_command(&command)
        .map_err(debug_error)?;
    Ok(format!(
        "task_sent=true\nagent={agent}\nscenario={scenario}\ntask_id={}\nsession_id={}\nrun_id={}\ncommand_id={}\n",
        refs.task_id, refs.session_id, refs.run_id, command.command_id
    ))
}

fn session_status(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let agent = required_arg(args, "--agent")?;
    let command = envelope(
        "session-status",
        CommandTarget::Agent(AgentId::new(format!("agent-{agent}"))),
        CommandIntent::QueryStatus,
        None,
    );
    let observation = controller(parsed)?
        .observe_agent_name(&agent)
        .map_err(debug_error)?;
    let evidence = state(parsed)?
        .evidence_for_session(&observation.session.session_id)
        .map_err(debug_error)?;
    Ok(render_status(&command, &observation, &evidence))
}

fn redirect_session(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let agent = required_arg(args, "--agent")?;
    let goal = required_arg(args, "--goal")?;
    let mut command = envelope(
        "redirect",
        CommandTarget::Agent(AgentId::new(format!("agent-{agent}"))),
        CommandIntent::RedirectSession,
        Some(goal.clone()),
    );
    command
        .structured_args
        .push(("agent".to_string(), agent.clone()));
    let observation = controller(parsed)?
        .redirect_command(&command)
        .map_err(debug_error)?;
    Ok(format!(
        "redirected=true\nagent={agent}\nsession_id={}\nstatus={}\ncurrent_goal={}\nlatest_summary={}\ncommand_id={}\n",
        observation.session.session_id,
        observation.session.status,
        observation.session.current_goal,
        observation
            .session
            .latest_summary
            .unwrap_or_else(|| "none".to_string()),
        command.command_id
    ))
}

fn interrupt_session(parsed: &ParsedArgs, args: &[String], action: &str) -> Result<String, String> {
    let agent = required_arg(args, "--agent")?;
    let reason = required_arg(args, "--reason")?;
    let mut command = envelope(
        action,
        CommandTarget::Agent(AgentId::new(format!("agent-{agent}"))),
        CommandIntent::InterruptSession,
        Some(reason.clone()),
    );
    command
        .structured_args
        .push(("agent".to_string(), agent.clone()));
    let controller = controller(parsed)?;
    let observation = if action == "stop" {
        controller.stop_command(&command).map_err(debug_error)?
    } else {
        controller
            .interrupt_command(&command)
            .map_err(debug_error)?
    };
    Ok(format!(
        "{action}=true\nagent={agent}\nsession_id={}\nstatus={}\nrun_status={}\nlatest_summary={}\ncommand_id={}\n",
        observation.session.session_id,
        observation.session.status,
        observation.run.status,
        observation
            .session
            .latest_summary
            .unwrap_or_else(|| "none".to_string()),
        command.command_id
    ))
}

fn submit_voice(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let transcript = required_arg(args, "--transcript")?;
    let voice_session_id =
        optional_arg(args, "--voice-session").unwrap_or_else(|| "voice-session-cli".to_string());
    let actor_id = optional_arg(args, "--actor").unwrap_or_else(|| "local-user".to_string());
    let confirmed = has_flag(args, "--confirm");
    let redacted_summary = optional_arg(args, "--redacted-summary");
    let retention_policy = if redacted_summary.is_some() {
        TranscriptRetentionPolicy::RetainRedactedSummary
    } else {
        VOICE_TRANSCRIPT_RETENTION_DEFAULT
    };
    if redacted_summary.is_some() && !has_flag(args, "--reviewed-summary") {
        return Err(
            "--redacted-summary requires --reviewed-summary before memory ingestion".to_string(),
        );
    }
    let plan = plan_dummy_transcript(VoiceTranscriptInput {
        voice_session_id,
        actor_id,
        project_id: project_id(),
        transcript_text: transcript,
        asr_confidence: None,
        retention_policy,
    });

    let retained_memory = if let Some(summary) = redacted_summary {
        Some(ingest_reviewed_voice_summary(parsed, &plan, &summary)?)
    } else {
        None
    };

    if plan.intent_kind == VoiceIntentKind::Unknown {
        let mut output = render_voice_header(&plan, None, false, false);
        if let Some(record) = &retained_memory {
            output.push_str(&render_voice_memory_retention(record));
        }
        return Ok(output);
    }
    if plan.requires_visible_confirmation && !confirmed {
        let approval = queue_voice_permission_approval(parsed, &plan)?;
        let mut output = render_voice_header(&plan, plan.command.as_ref(), true, false);
        output.push_str(&render_voice_approval(&approval, None));
        if let Some(record) = &retained_memory {
            output.push_str(&render_voice_memory_retention(record));
        }
        return Ok(output);
    }

    let mut output = render_voice_header(&plan, plan.command.as_ref(), false, false);
    match plan.intent_kind {
        VoiceIntentKind::ConnectivityStatus
        | VoiceIntentKind::DashboardSummary
        | VoiceIntentKind::DispatchStatus
        | VoiceIntentKind::DogfoodReadiness
        | VoiceIntentKind::NextWork
        | VoiceIntentKind::RecentWork
        | VoiceIntentKind::ReviewNeeds
        | VoiceIntentKind::AgentStatus => {
            let dashboard = voice_dashboard(parsed, &plan)?;
            output.push_str(&render_voice_read_contract(&plan, &dashboard));
        }
        VoiceIntentKind::RedirectSession => {
            let command = plan
                .command
                .as_ref()
                .ok_or_else(|| "voice redirect plan missing command".to_string())?;
            controller(parsed)?
                .redirect_command(command)
                .map_err(debug_error)?;
            output = render_voice_header(&plan, Some(command), false, true);
            let dashboard = voice_dashboard(parsed, &plan)?;
            output.push_str(&render_voice_read_contract(&plan, &dashboard));
        }
        VoiceIntentKind::StartNextWork => {
            let command = plan
                .command
                .as_ref()
                .ok_or_else(|| "voice start-next plan missing command".to_string())?;
            let approval = decide_voice_permission_approval(parsed, &plan, "allow_once")?;
            let agent = structured_arg_value(command, "agent")
                .ok_or_else(|| "voice start-next plan missing agent".to_string())?
                .to_string();
            let started = start_next_workpad_task(parsed, &["--agent".to_string(), agent.clone()])?;
            output = render_voice_header(&plan, Some(command), true, true);
            output.push_str(&render_voice_approval(
                &approval,
                approval.decision.as_deref(),
            ));
            output.push_str(&format!("controlled_agent={agent}\n"));
            output.push_str(&started);
            let dashboard = voice_dashboard(parsed, &plan)?;
            output.push_str(&render_voice_read_contract(&plan, &dashboard));
        }
        VoiceIntentKind::InterruptSession | VoiceIntentKind::StopSession => {
            let command = plan
                .command
                .as_ref()
                .ok_or_else(|| "voice session-control plan missing command".to_string())?;
            let approval = decide_voice_permission_approval(parsed, &plan, "allow_once")?;
            let durable_reason = match plan.intent_kind {
                VoiceIntentKind::InterruptSession => "voice interrupt confirmed",
                VoiceIntentKind::StopSession => "voice stop confirmed",
                _ => unreachable!("only privileged session-control intents reach this branch"),
            };
            let command = CommandEnvelope {
                text: Some(durable_reason.to_string()),
                ..command.clone()
            };
            let controller = controller(parsed)?;
            let observation = match plan.intent_kind {
                VoiceIntentKind::InterruptSession => controller
                    .interrupt_command(&command)
                    .map_err(debug_error)?,
                VoiceIntentKind::StopSession => {
                    controller.stop_command(&command).map_err(debug_error)?
                }
                _ => unreachable!("only privileged session-control intents reach this branch"),
            };
            output = render_voice_header(&plan, Some(&command), true, true);
            output.push_str(&render_voice_approval(
                &approval,
                approval.decision.as_deref(),
            ));
            output.push_str(&format!(
                "controlled_session={} session_status={} run_status={}\n",
                observation.session.session_id, observation.session.status, observation.run.status
            ));
            let dashboard = voice_dashboard(parsed, &plan)?;
            output.push_str(&render_voice_read_contract(&plan, &dashboard));
        }
        VoiceIntentKind::Unknown => {}
    }
    if let Some(record) = &retained_memory {
        output.push_str(&render_voice_memory_retention(record));
    }
    Ok(output)
}

fn ingest_reviewed_voice_summary(
    parsed: &ParsedArgs,
    plan: &VoiceCommandPlan,
    redacted_summary: &str,
) -> Result<MemoryRecordProjection, String> {
    let command = plan
        .command
        .as_ref()
        .ok_or_else(|| "reviewed voice summary requires a planned command".to_string())?;
    if redacted_summary.trim().is_empty() {
        return Err("--redacted-summary cannot be empty".to_string());
    }
    if !plan.transcript_policy.redaction_required
        || plan.transcript_policy.memory_ingestion
            != MemoryIngestionPolicy::ReviewedRedactedSummaryOnly
    {
        return Err(
            "voice summary memory ingestion requires reviewed redacted summary policy".to_string(),
        );
    }
    let voice_session_id = structured_arg_value(command, "voice_session_id")
        .unwrap_or("voice-session")
        .to_string();
    let summary_hash = stable_cli_hash(redacted_summary);
    let record_id = format!(
        "memory-voice-summary-{}",
        stable_cli_hash(&format!(
            "{}:{}:{}",
            command.project_id, voice_session_id, summary_hash
        ))
    );
    let source_id = format!("source-{record_id}");
    let event_id = format!("event-memory-voice-summary-{summary_hash}");
    let record = MemoryRecordProjection {
        memory_record_id: record_id.clone(),
        project_id: command.project_id.clone(),
        scope: "project".to_string(),
        scope_owner_ref: command.project_id.to_string(),
        subject_ref: Some(voice_session_id.clone()),
        sensitivity_classification: "internal".to_string(),
        record_kind: "summary".to_string(),
        subject: "voice_conversation".to_string(),
        predicate: "retained_reviewed_summary".to_string(),
        object: voice_intent_label(plan.intent_kind).to_string(),
        body: redacted_summary.trim().to_string(),
        confidence: "medium".to_string(),
        review_state: "reviewed".to_string(),
        source_count: 1,
        valid_from: None,
        valid_until: None,
        supersedes_memory_record_id: None,
        revoked_by_memory_record_id: None,
        redaction_state: RedactionState::Redacted.as_str().to_string(),
        invalidated_at: None,
        invalidation_reason: None,
        packet_item_ref: Some(format!("memory-record:{record_id}")),
        updated_sequence: 0,
    };
    let source = MemorySourceProjection {
        memory_source_id: source_id,
        memory_record_id: record_id.clone(),
        source_kind: "event".to_string(),
        source_event_id: Some(event_id.clone()),
        source_artifact_id: None,
        source_path: None,
        source_anchor: Some("voice:redacted-summary".to_string()),
        source_content_hash: Some(summary_hash.to_string()),
        source_sequence: None,
        quote_artifact_id: None,
        observed_at: Some("cli".to_string()),
        updated_sequence: 0,
    };
    let mut event = NewEvent::new(event_id, EventKind::MemoryRecordIngested, "capo-voice");
    event.project_id = Some(command.project_id.clone());
    event.payload_json = format!(
        "{{\"memory_record_id\":\"{}\",\"origin\":\"voice\",\"review_state\":\"reviewed\",\"redaction_state\":\"redacted\",\"voice_session_id\":\"{}\",\"intent\":\"{}\",\"summary_hash\":{}}}",
        escape_json(&record.memory_record_id),
        escape_json(&voice_session_id),
        voice_intent_label(plan.intent_kind),
        summary_hash
    );
    event.idempotency_key = Some(format!("voice-summary-memory:{record_id}"));
    event.redaction_state = RedactionState::Redacted;
    state(parsed)?
        .append_event(
            event,
            &[
                ProjectionRecord::MemoryRecord(Box::new(record.clone())),
                ProjectionRecord::MemorySource(source),
            ],
        )
        .map_err(debug_error)?;
    Ok(record)
}

fn queue_voice_permission_approval(
    parsed: &ParsedArgs,
    plan: &VoiceCommandPlan,
) -> Result<PermissionApprovalProjection, String> {
    let approval = voice_permission_approval(parsed, plan)?;
    let state = state(parsed)?;
    if let Some(existing) = state
        .permission_approval(&approval.project_id, &approval.approval_id)
        .map_err(debug_error)?
    {
        return Ok(existing);
    }
    let mut event = NewEvent::new(
        format!(
            "event-voice-permission-approval-queued-{}",
            stable_cli_hash(&approval.approval_id)
        ),
        EventKind::PermissionApprovalQueued,
        "capo-voice",
    );
    event.project_id = Some(approval.project_id.clone());
    event.session_id = approval.session_id.clone();
    event.payload_json = format!(
        "{{\"approval_id\":\"{}\",\"capability_profile_id\":\"{}\",\"scope_json\":{},\"subject_json\":{},\"requested_by\":\"{}\",\"reason\":\"{}\",\"origin\":\"voice\"}}",
        escape_json(&approval.approval_id),
        escape_json(&approval.capability_profile_id),
        approval.scope_json,
        approval.subject_json,
        escape_json(&approval.requested_by),
        escape_json(&approval.reason)
    );
    event.idempotency_key = Some(format!(
        "voice-permission-approval:{}",
        approval.approval_id
    ));
    event.redaction_state = RedactionState::Safe;
    state
        .append_event(
            event,
            &[ProjectionRecord::PermissionApproval(approval.clone())],
        )
        .map_err(debug_error)?;
    Ok(approval)
}

fn decide_voice_permission_approval(
    parsed: &ParsedArgs,
    plan: &VoiceCommandPlan,
    decision: &str,
) -> Result<PermissionApprovalProjection, String> {
    let approval = queue_voice_permission_approval(parsed, plan)?;
    if approval.status == "decided" {
        return Ok(approval);
    }
    let (effect, persistence) = approval_decision_effect(decision)?;
    let subject_json = approval_subject_json(&approval)?;
    let grant_id = format!(
        "grant-voice-approval-{}",
        stable_cli_hash(&format!(
            "{}:{}:{}:{}:{}",
            approval.approval_id,
            approval.capability_profile_id,
            approval.scope_json,
            subject_json,
            decision
        ))
    );
    let grant = CapabilityGrantProjection {
        capability_grant_id: grant_id.clone(),
        capability_profile_id: approval.capability_profile_id.clone(),
        scope_json: approval.scope_json.clone(),
        effect: effect.to_string(),
        subject_json,
        decision_source: "user_visible_voice_confirmation".to_string(),
        persistence: persistence.to_string(),
        explanation: format!(
            "visible voice confirmation {decision} for {}",
            approval.approval_id
        ),
        updated_sequence: 0,
    };
    let decided_approval = PermissionApprovalProjection {
        status: "decided".to_string(),
        decision: Some(decision.to_string()),
        capability_grant_id: Some(grant.capability_grant_id.clone()),
        updated_sequence: 0,
        ..approval.clone()
    };
    let mut event = NewEvent::new(
        format!(
            "event-voice-permission-decided-{}",
            stable_cli_hash(&format!("{}:{decision}:{grant_id}", approval.approval_id))
        ),
        EventKind::PermissionDecided,
        "capo-voice",
    );
    event.project_id = Some(approval.project_id.clone());
    event.session_id = approval.session_id.clone();
    event.payload_json = format!(
        "{{\"approval_id\":\"{}\",\"decision\":\"{}\",\"capability_grant_id\":\"{}\",\"effect\":\"{}\",\"persistence\":\"{}\",\"origin\":\"voice\"}}",
        escape_json(&approval.approval_id),
        escape_json(decision),
        escape_json(&grant.capability_grant_id),
        effect,
        persistence
    );
    event.redaction_state = RedactionState::Safe;
    let mut grant_event = NewEvent::new(
        format!(
            "event-voice-capability-grant-{}",
            stable_cli_hash(&format!("{}:{decision}:{grant_id}", approval.approval_id))
        ),
        EventKind::CapabilityGrantCreated,
        "capo-voice",
    );
    grant_event.project_id = Some(approval.project_id.clone());
    grant_event.session_id = approval.session_id.clone();
    grant_event.payload_json = format!(
        "{{\"approval_id\":\"{}\",\"capability_grant_id\":\"{}\",\"effect\":\"{}\",\"decision_source\":\"{}\",\"persistence\":\"{}\",\"origin\":\"voice\"}}",
        escape_json(&approval.approval_id),
        escape_json(&grant.capability_grant_id),
        escape_json(&grant.effect),
        escape_json(&grant.decision_source),
        escape_json(&grant.persistence)
    );
    grant_event.redaction_state = RedactionState::Safe;
    state(parsed)?
        .decide_permission_approval(
            &approval.approval_id,
            event,
            Some(grant_event),
            decided_approval.clone(),
            Some(grant),
        )
        .map_err(debug_error)?;
    Ok(decided_approval)
}

fn voice_permission_approval(
    parsed: &ParsedArgs,
    plan: &VoiceCommandPlan,
) -> Result<PermissionApprovalProjection, String> {
    let command = plan
        .command
        .as_ref()
        .ok_or_else(|| "voice privileged plan missing command".to_string())?;
    let session_id = voice_session_id(parsed, plan)?;
    let voice_session_id = structured_arg_value(command, "voice_session_id")
        .unwrap_or("voice-session")
        .to_string();
    let approval_id = format!(
        "approval-voice-{}",
        stable_cli_hash(&format!(
            "{}:{}:{}:{}",
            command.command_id,
            command.actor_id,
            session_id
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| "none".to_string()),
            voice_intent_label(plan.intent_kind)
        ))
    );
    Ok(PermissionApprovalProjection {
        approval_id,
        project_id: command.project_id.clone(),
        session_id,
        tool_call_id: None,
        capability_profile_id: "voice-control".to_string(),
        scope_json: "[\"voice:approve:privileged\"]".to_string(),
        subject_json: format!(
            "{{\"actor\":\"{}\",\"origin\":\"voice\",\"voice_session_id\":\"{}\",\"command_id\":\"{}\",\"intent\":\"{}\"}}",
            escape_json(&command.actor_id),
            escape_json(&voice_session_id),
            escape_json(command.command_id.as_str()),
            voice_intent_label(plan.intent_kind)
        ),
        status: "pending".to_string(),
        requested_by: format!("voice:{}", command.actor_id),
        reason: format!(
            "visible confirmation required for {}",
            voice_intent_label(plan.intent_kind)
        ),
        decision: None,
        capability_grant_id: None,
        updated_sequence: 0,
    })
}

fn voice_session_id(
    parsed: &ParsedArgs,
    plan: &VoiceCommandPlan,
) -> Result<Option<SessionId>, String> {
    match &plan.read_contract.query_scope {
        VoiceReadScope::SessionForAgent { agent_name } | VoiceReadScope::Agent { agent_name } => {
            let dashboard = voice_dashboard(parsed, plan)?;
            Ok(dashboard
                .agents
                .iter()
                .find(|row| row.agent.name == *agent_name)
                .and_then(|row| row.session.as_ref())
                .map(|row| row.session.session_id.clone()))
        }
        VoiceReadScope::ProjectDashboard
        | VoiceReadScope::ProjectLatestConnectivityExposure { .. }
        | VoiceReadScope::ProjectDispatchStatus { .. }
        | VoiceReadScope::ProjectLatestDispatchStatus { .. }
        | VoiceReadScope::ProjectDogfoodReadiness
        | VoiceReadScope::ProjectNextWork
        | VoiceReadScope::ProjectRecentWork
        | VoiceReadScope::ProjectReviewNeeds
        | VoiceReadScope::None => Ok(None),
    }
}

fn render_voice_approval(
    approval: &PermissionApprovalProjection,
    decision: Option<&str>,
) -> String {
    format!(
        "permission_approval={}\npermission_status={}\npermission_decision={}\npermission_scope={}\npermission_requested_by={}\npermission_reason={}\n",
        approval.approval_id,
        approval.status,
        decision.or(approval.decision.as_deref()).unwrap_or("none"),
        approval.scope_json,
        approval.requested_by,
        approval.reason
    )
}

fn render_voice_memory_retention(record: &MemoryRecordProjection) -> String {
    format!(
        "memory_record={}\nmemory_review_state={}\nmemory_redaction_state={}\nmemory_ingestion=reviewed_redacted_summary_only\n",
        record.memory_record_id, record.review_state, record.redaction_state
    )
}

fn voice_dashboard(
    parsed: &ParsedArgs,
    plan: &VoiceCommandPlan,
) -> Result<ProjectDashboard, String> {
    let command = plan
        .command
        .as_ref()
        .ok_or_else(|| "voice plan missing query command".to_string())?;
    project_dashboard(
        &state(parsed)?,
        ProjectDashboardQuery::new(command.project_id.clone()),
    )
    .map_err(debug_error)
}

fn render_voice_header(
    plan: &VoiceCommandPlan,
    command: Option<&CommandEnvelope>,
    confirmation_required: bool,
    mutation_applied: bool,
) -> String {
    format!(
        "voice_plan={}\norigin=voice\ncommand_id={}\nconfirmation_required={}\nmutation_applied={}\nraw_transcript_retained={}\nredaction_required={}\nmemory_ingestion={}\nassistant_reply_hint={}\n",
        voice_intent_label(plan.intent_kind),
        command
            .map(|command| command.command_id.to_string())
            .unwrap_or_else(|| "none".to_string()),
        confirmation_required,
        mutation_applied,
        plan.transcript_policy.retain_raw_transcript,
        plan.transcript_policy.redaction_required,
        memory_ingestion_label(plan.transcript_policy.memory_ingestion),
        plan.assistant_reply_hint
    )
}

fn render_voice_read_contract(plan: &VoiceCommandPlan, dashboard: &ProjectDashboard) -> String {
    let mut output = format!(
        "read_scope={}\nrequired_fields={}\n",
        voice_scope_label(&plan.read_contract.query_scope),
        plan.read_contract.required_fields.join(",")
    );
    match &plan.read_contract.query_scope {
        VoiceReadScope::ProjectDashboard => {
            output.push_str(&format!(
                "spoken_agents={}\nspoken_active_sessions={}\n",
                dashboard.agents.len(),
                dashboard.active_session_count()
            ));
            for row in &dashboard.agents {
                append_voice_agent_row(&mut output, row);
            }
        }
        VoiceReadScope::ProjectDispatchStatus { dispatch_plan_id } => {
            if let Some(status) = dashboard.adapter_dispatch_status(dispatch_plan_id) {
                append_voice_dispatch_status(&mut output, &status);
            } else {
                output.push_str(&format!(
                    "spoken_dispatch_plan_missing={dispatch_plan_id}\n"
                ));
            }
        }
        VoiceReadScope::ProjectLatestDispatchStatus { agent_name } => {
            if let Some(status) = dashboard.latest_adapter_dispatch_status(agent_name.as_deref()) {
                append_voice_dispatch_status(&mut output, &status);
            } else if let Some(agent_name) = agent_name {
                output.push_str(&format!(
                    "spoken_latest_dispatch_missing_for_agent={agent_name}\n"
                ));
            } else {
                output.push_str("spoken_latest_dispatch_missing=true\n");
            }
        }
        VoiceReadScope::ProjectLatestConnectivityExposure {
            owner_kind,
            owner_id,
            channel_kind,
        } => {
            if let Some(exposure) = dashboard.latest_connectivity_exposure(
                owner_kind.as_deref(),
                owner_id.as_deref(),
                channel_kind.as_deref(),
            ) {
                append_voice_connectivity_exposure_status(&mut output, exposure);
            } else {
                output.push_str("spoken_latest_connectivity_exposure_missing=true\n");
            }
        }
        VoiceReadScope::ProjectDogfoodReadiness => {
            let readiness = dashboard.dogfood_readiness();
            output.push_str(&format!(
                "spoken_dogfood_ready={}\nspoken_dogfood_status={}\nspoken_real_agent_connector_ready={}\nspoken_workpad_bridge_ready={}\nspoken_dispatch_chain_ready={}\nspoken_blockers={}\nspoken_next_actions={}\n",
                readiness.ready,
                readiness.status,
                readiness.real_agent_connector_ready,
                readiness.workpad_bridge_ready,
                readiness.dispatch_chain_ready,
                comma_or_none(&readiness.blockers),
                comma_or_none(&readiness.next_actions)
            ));
        }
        VoiceReadScope::ProjectNextWork => {
            output.push_str(&format!(
                "spoken_workpad_tasks={}\nspoken_next_work_candidates={}\n",
                dashboard.workpad_tasks.len(),
                dashboard.next_workpad_candidate_count()
            ));
            if let Some(next) = dashboard.next_workpad_task() {
                output.push_str(&format!(
                    "spoken_next_workpad_task={} default_task_id={} path={} source_anchor={} source={}#{} title={} observed_status={} capo_execution_status={}\n",
                    next.workpad_task_id,
                    default_workpad_task_id(&next.workpad_task_id),
                    next.path,
                    next.source_anchor,
                    next.path,
                    next.source_anchor,
                    next.title,
                    next.observed_status,
                    next.capo_execution_status
                ));
            } else {
                output.push_str("spoken_next_workpad_task=none\n");
            }
        }
        VoiceReadScope::ProjectRecentWork => {
            output.push_str(&format!(
                "spoken_agents={}\nspoken_active_sessions={}\nspoken_project_evidence={}\n",
                dashboard.agents.len(),
                dashboard.active_session_count(),
                dashboard.project_evidence.len()
            ));
            for row in &dashboard.agents {
                append_voice_agent_row(&mut output, row);
            }
        }
        VoiceReadScope::ProjectReviewNeeds => {
            let open_review_findings = dashboard
                .review_findings
                .iter()
                .filter(|finding| finding.status == "open")
                .count();
            let review_blockers = dashboard
                .review_findings
                .iter()
                .filter(|finding| finding.status == "open" && finding.finding_kind == "blocker")
                .count();
            let reports_with_findings = dashboard
                .task_outcome_reports
                .iter()
                .filter(|report| report.review_outcome == "reviewed_with_findings")
                .count();
            let latest_review_outcome = dashboard
                .task_outcome_reports
                .iter()
                .max_by_key(|report| report.updated_sequence)
                .map(|report| report.review_outcome.as_str())
                .unwrap_or("none");
            output.push_str(&format!(
                "spoken_review_findings={}\nspoken_open_review_findings={}\nspoken_review_blockers={}\nspoken_task_outcome_reports={}\nspoken_reports_with_findings={}\nspoken_latest_review_outcome={}\n",
                dashboard.review_findings.len(),
                open_review_findings,
                review_blockers,
                dashboard.task_outcome_reports.len(),
                reports_with_findings,
                latest_review_outcome
            ));
            for finding in &dashboard.review_findings {
                output.push_str(&format!(
                    "spoken_review_finding={} kind={} severity={} status={} summary={}\n",
                    finding.review_finding_id,
                    finding.finding_kind,
                    finding.severity,
                    finding.status,
                    finding.summary
                ));
            }
            for report in &dashboard.task_outcome_reports {
                output.push_str(&format!(
                    "spoken_task_outcome_report={} outcome_status={} review_outcome={} artifact={}\n",
                    report.task_outcome_report_id,
                    report.outcome_status,
                    report.review_outcome,
                    report.report_artifact_id.as_deref().unwrap_or("none")
                ));
            }
        }
        VoiceReadScope::Agent { agent_name } | VoiceReadScope::SessionForAgent { agent_name } => {
            if let Some(row) = dashboard
                .agents
                .iter()
                .find(|row| row.agent.name == *agent_name)
            {
                append_voice_agent_row(&mut output, row);
            } else {
                output.push_str(&format!("spoken_agent_missing={agent_name}\n"));
            }
        }
        VoiceReadScope::None => {}
    }
    output
}

fn append_voice_dispatch_status(output: &mut String, status: &AdapterDispatchStatus) {
    output.push_str(&format!(
        "spoken_dispatch_plan={} spoken_adapter={} spoken_agent={} spoken_plan_status={} spoken_provider_kind={} spoken_credential_scope={} spoken_provider_cli_executed={} spoken_dogfood_gate={} spoken_latest_gate_status={} spoken_latest_gate_provider_cli_execution_allowed={} spoken_latest_gate_reasons={} spoken_latest_dispatch_replay={} spoken_latest_replay_appended_events={} spoken_latest_execution_status={} spoken_latest_execution_provider_cli_executed={} spoken_latest_execution_credential_scan_status={} spoken_next_action={}\n",
        status.dispatch_plan_id,
        status.adapter_kind,
        status.agent_name,
        status.plan_status,
        status.provider_kind,
        status.credential_scope,
        status.provider_cli_executed,
        status.dogfood_gate_status,
        status.latest_gate_status,
        status.latest_gate_provider_cli_execution_allowed,
        status.latest_gate_reasons,
        status.latest_dispatch_replay_id,
        status.latest_replay_appended_events,
        status.latest_execution_status,
        status.latest_execution_provider_cli_executed,
        status.latest_execution_credential_scan_status,
        status.next_action
    ));
}

fn append_voice_connectivity_exposure_status(
    output: &mut String,
    exposure: &ConnectivityExposureProjection,
) {
    output.push_str(&format!(
        "spoken_connectivity_exposure={} spoken_endpoint={} spoken_owner={}:{} spoken_channel={} spoken_exposure_scope={} spoken_permission_scope={} spoken_exposure_status={} spoken_health={} spoken_reachable={} spoken_grant={} spoken_revoked_at={}\n",
        exposure.exposure_id,
        exposure.connectivity_endpoint_id,
        exposure.owner_kind,
        exposure.owner_id,
        exposure.channel_kind,
        exposure.exposure,
        exposure.permission_scope,
        exposure.status,
        exposure.health_status,
        exposure.reachable,
        exposure.capability_grant_id.as_deref().unwrap_or("none"),
        exposure.revoked_at.as_deref().unwrap_or("none")
    ));
}

fn append_voice_agent_row(output: &mut String, row: &capo_query::AgentDashboardRow) {
    output.push_str(&format!(
        "spoken_agent={} agent_status={}\n",
        row.agent.name, row.agent.status
    ));
    if let Some(session_row) = &row.session {
        output.push_str(&format!(
            "spoken_session={} session_status={} run_status={} current_goal={} latest_summary={} blocker={} confidence={} evidence_refs={} recent_events={}\n",
            session_row.session.session_id,
            session_row.session.status,
            session_row
                .run
                .as_ref()
                .map(|run| run.status.clone())
                .unwrap_or_else(|| "none".to_string()),
            session_row.session.current_goal,
            session_row
                .session
                .latest_summary
                .as_deref()
                .unwrap_or("none"),
            session_row.session.latest_blocker.as_deref().unwrap_or("none"),
            session_row
                .session
                .latest_confidence
                .map(|value| value.to_string())
                .unwrap_or_else(|| "none".to_string()),
            session_row
                .evidence
                .iter()
                .map(|item| item.evidence_id.to_string())
                .collect::<Vec<_>>()
                .join(","),
            session_row.recent_events.len()
        ));
    }
}

fn voice_intent_label(intent: VoiceIntentKind) -> &'static str {
    match intent {
        VoiceIntentKind::AgentStatus => "agent_status",
        VoiceIntentKind::ConnectivityStatus => "connectivity_status",
        VoiceIntentKind::DashboardSummary => "dashboard_summary",
        VoiceIntentKind::DispatchStatus => "dispatch_status",
        VoiceIntentKind::DogfoodReadiness => "dogfood_readiness",
        VoiceIntentKind::NextWork => "next_work",
        VoiceIntentKind::RecentWork => "recent_work",
        VoiceIntentKind::ReviewNeeds => "review_needs",
        VoiceIntentKind::RedirectSession => "redirect_session",
        VoiceIntentKind::StartNextWork => "start_next_work",
        VoiceIntentKind::InterruptSession => "interrupt_session",
        VoiceIntentKind::StopSession => "stop_session",
        VoiceIntentKind::Unknown => "unknown",
    }
}

fn voice_scope_label(scope: &VoiceReadScope) -> &'static str {
    match scope {
        VoiceReadScope::ProjectDashboard => "project_dashboard",
        VoiceReadScope::ProjectLatestConnectivityExposure { .. } => {
            "project_latest_connectivity_exposure"
        }
        VoiceReadScope::ProjectDispatchStatus { .. } => "project_dispatch_status",
        VoiceReadScope::ProjectLatestDispatchStatus { .. } => "project_latest_dispatch_status",
        VoiceReadScope::ProjectDogfoodReadiness => "project_dogfood_readiness",
        VoiceReadScope::ProjectNextWork => "project_next_work",
        VoiceReadScope::ProjectRecentWork => "project_recent_work",
        VoiceReadScope::ProjectReviewNeeds => "project_review_needs",
        VoiceReadScope::Agent { .. } => "agent",
        VoiceReadScope::SessionForAgent { .. } => "session_for_agent",
        VoiceReadScope::None => "none",
    }
}

fn memory_ingestion_label(policy: MemoryIngestionPolicy) -> &'static str {
    match policy {
        MemoryIngestionPolicy::None => "none",
        MemoryIngestionPolicy::ReviewedRedactedSummaryOnly => "reviewed_redacted_summary_only",
    }
}

fn structured_arg_value<'a>(command: &'a CommandEnvelope, key: &str) -> Option<&'a str> {
    command
        .structured_args
        .iter()
        .find(|(candidate, _)| candidate == key)
        .map(|(_, value)| value.as_str())
}

fn recover(parsed: &ParsedArgs) -> Result<String, String> {
    let command = envelope(
        "recover",
        CommandTarget::Project(project_id()),
        CommandIntent::Recover,
        None,
    );
    let report = controller(parsed)?
        .recover_command(&command)
        .map_err(debug_error)?;
    Ok(format!(
        "recovered=true\nrecovery_attempt_id={}\nstarted_sequence={}\ncompleted_sequence={}\nwatermark={}\nrecovered_run_count={}\ncommand_id={}\n",
        report.recovery_attempt_id,
        report.started_sequence,
        report.completed_sequence,
        report
            .watermark
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_string()),
        report.recovered_run_count,
        command.command_id
    ))
}

fn request_permission_approval(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let approval_id = required_arg(args, "--approval")?;
    let scope_json = required_arg(args, "--scope-json")?;
    validate_scope_json(&scope_json)?;
    let subject_json = optional_arg(args, "--subject-json")
        .unwrap_or_else(|| "{\"actor\":\"local-user\"}".to_string());
    validate_json_object("subject-json", &subject_json)?;
    let reason = required_arg(args, "--reason")?;
    let capability_profile_id =
        optional_arg(args, "--profile").unwrap_or_else(|| "trusted-local-dev".to_string());
    let requested_by =
        optional_arg(args, "--requested-by").unwrap_or_else(|| "local-user".to_string());
    let project_id = project_id();
    let session_id = optional_arg(args, "--session").map(SessionId::new);
    let tool_call_id = optional_arg(args, "--tool-call").map(ToolCallId::new);
    let command = envelope(
        "permission-request",
        CommandTarget::Project(project_id.clone()),
        CommandIntent::QueuePermissionApproval,
        Some(reason.clone()),
    );
    let state = state(parsed)?;
    if state
        .permission_approval(&project_id, &approval_id)
        .map_err(debug_error)?
        .is_some()
    {
        return Err(format!("approval already exists: {approval_id}"));
    }
    let approval = PermissionApprovalProjection {
        approval_id: approval_id.clone(),
        project_id: project_id.clone(),
        session_id,
        tool_call_id,
        capability_profile_id,
        scope_json,
        subject_json,
        status: "pending".to_string(),
        requested_by,
        reason,
        decision: None,
        capability_grant_id: None,
        updated_sequence: 0,
    };
    let mut event = NewEvent::new(
        format!(
            "event-permission-approval-queued-{}",
            stable_cli_hash(&approval_id)
        ),
        EventKind::PermissionApprovalQueued,
        "capo-cli",
    );
    event.project_id = Some(project_id.clone());
    event.session_id = approval.session_id.clone();
    event.item_id = approval.tool_call_id.as_ref().map(ToString::to_string);
    event.payload_json = format!(
        "{{\"approval_id\":\"{}\",\"capability_profile_id\":\"{}\",\"scope_json\":{},\"subject_json\":{},\"requested_by\":\"{}\",\"reason\":\"{}\"}}",
        escape_json(&approval.approval_id),
        escape_json(&approval.capability_profile_id),
        approval.scope_json,
        approval.subject_json,
        escape_json(&approval.requested_by),
        escape_json(&approval.reason)
    );
    event.idempotency_key = Some(format!("permission-approval-request:{approval_id}"));
    event.redaction_state = RedactionState::Safe;
    let sequence = state
        .append_event(
            event,
            &[ProjectionRecord::PermissionApproval(approval.clone())],
        )
        .map_err(debug_error)?;
    Ok(format!(
        "permission_approval_queued=true\napproval_id={}\nstatus=pending\nprofile={}\nsequence={sequence}\ncommand_id={}\n",
        approval.approval_id, approval.capability_profile_id, command.command_id
    ))
}

fn list_permission_approvals(parsed: &ParsedArgs) -> Result<String, String> {
    let command = envelope(
        "permission-list",
        CommandTarget::Project(project_id()),
        CommandIntent::QueryStatus,
        None,
    );
    let approvals = state(parsed)?
        .permission_approvals(&project_id())
        .map_err(debug_error)?;
    let mut output = format!(
        "command_id={}\npermission_approvals={}\n",
        command.command_id,
        approvals.len()
    );
    for approval in approvals {
        output.push_str(&format!(
            "approval={} status={} profile={} session={} tool_call={} decision={} grant={} requested_by={} reason={}\n",
            approval.approval_id,
            approval.status,
            approval.capability_profile_id,
            approval
                .session_id
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| "none".to_string()),
            approval
                .tool_call_id
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| "none".to_string()),
            approval.decision.as_deref().unwrap_or("none"),
            approval.capability_grant_id.as_deref().unwrap_or("none"),
            approval.requested_by,
            approval.reason
        ));
    }
    Ok(output)
}

fn decide_permission_approval(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let approval_id = required_arg(args, "--approval")?;
    let decision = required_arg(args, "--decision")?;
    let (effect, persistence) = approval_decision_effect(&decision)?;
    let project_id = project_id();
    let command = envelope(
        "permission-decide",
        CommandTarget::Project(project_id.clone()),
        CommandIntent::DecidePermissionApproval,
        Some(decision.clone()),
    );
    let state = state(parsed)?;
    let approval = state
        .permission_approval(&project_id, &approval_id)
        .map_err(debug_error)?
        .ok_or_else(|| format!("missing approval: {approval_id}"))?;
    if approval.status != "pending" {
        return Err(format!(
            "approval is not pending: {approval_id} status={}",
            approval.status
        ));
    }
    validate_decision_scope(&decision, &approval)?;
    let subject_json = approval_subject_json(&approval)?;
    let grant_id = format!(
        "grant-approval-{}",
        stable_cli_hash(&format!(
            "{}:{}:{}:{}:{}",
            approval.approval_id,
            approval.capability_profile_id,
            approval.scope_json,
            subject_json,
            decision
        ))
    );
    let grant = (decision != "reject_once").then(|| CapabilityGrantProjection {
        capability_grant_id: grant_id.clone(),
        capability_profile_id: approval.capability_profile_id.clone(),
        scope_json: approval.scope_json.clone(),
        effect: effect.to_string(),
        subject_json: subject_json.clone(),
        decision_source: "user".to_string(),
        persistence: persistence.to_string(),
        explanation: format!("user approval decision {decision} for {approval_id}"),
        updated_sequence: 0,
    });
    let decided_approval = PermissionApprovalProjection {
        status: "decided".to_string(),
        decision: Some(decision.clone()),
        capability_grant_id: grant
            .as_ref()
            .map(|grant| grant.capability_grant_id.clone()),
        updated_sequence: 0,
        ..approval.clone()
    };
    let mut event = NewEvent::new(
        format!(
            "event-permission-decided-{}",
            stable_cli_hash(&format!("{approval_id}:{decision}:{grant_id}"))
        ),
        EventKind::PermissionDecided,
        "capo-cli",
    );
    event.project_id = Some(project_id.clone());
    event.session_id = approval.session_id.clone();
    event.item_id = approval.tool_call_id.as_ref().map(ToString::to_string);
    event.payload_json = format!(
        "{{\"approval_id\":\"{}\",\"decision\":\"{}\",\"capability_grant_id\":\"{}\",\"effect\":\"{}\",\"persistence\":\"{}\"}}",
        escape_json(&approval_id),
        escape_json(&decision),
        escape_json(&grant_id),
        effect,
        persistence
    );
    event.idempotency_key = None;
    event.redaction_state = RedactionState::Safe;
    let grant_event = grant.as_ref().map(|grant| {
        let mut event = NewEvent::new(
            format!(
                "event-capability-grant-{}",
                stable_cli_hash(&format!("{approval_id}:{decision}:{grant_id}"))
            ),
            EventKind::CapabilityGrantCreated,
            "capo-cli",
        );
        event.project_id = Some(project_id.clone());
        event.session_id = approval.session_id.clone();
        event.item_id = approval.tool_call_id.as_ref().map(ToString::to_string);
        event.payload_json = format!(
            "{{\"approval_id\":\"{}\",\"capability_grant_id\":\"{}\",\"effect\":\"{}\",\"decision_source\":\"{}\",\"persistence\":\"{}\"}}",
            escape_json(&approval_id),
            escape_json(&grant.capability_grant_id),
            escape_json(&grant.effect),
            escape_json(&grant.decision_source),
            escape_json(&grant.persistence)
        );
        event.idempotency_key = None;
        event.redaction_state = RedactionState::Safe;
        event
    });
    let sequence = state
        .decide_permission_approval(&approval_id, event, grant_event, decided_approval, grant)
        .map_err(debug_error)?;
    Ok(format!(
        "permission_approval_decided=true\napproval_id={approval_id}\ndecision={decision}\neffect={effect}\npersistence={persistence}\ncapability_grant_id={}\nsequence={sequence}\ncommand_id={}\n",
        if decision == "reject_once" {
            "none"
        } else {
            &grant_id
        },
        command.command_id
    ))
}

fn expose_connectivity_stub(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let endpoint_id = required_arg(args, "--endpoint")?;
    let owner_kind = required_arg(args, "--owner-kind")?;
    let owner_id = required_arg(args, "--owner-id")?;
    let channel = parse_channel_kind(&required_arg(args, "--channel")?)?;
    let exposure = parse_exposure_scope(&required_arg(args, "--exposure")?)?;
    let address_ref = optional_arg(args, "--address").unwrap_or_else(|| owner_id.clone());
    let record = has_flag(args, "--record");
    if let Some(unknown) = args.iter().find(|arg| {
        arg.starts_with("--")
            && !matches!(
                arg.as_str(),
                "--endpoint"
                    | "--owner-kind"
                    | "--owner-id"
                    | "--channel"
                    | "--exposure"
                    | "--address"
                    | "--record"
            )
    }) {
        return Err(format!(
            "unknown connectivity expose-stub option: {unknown}"
        ));
    }

    let owner = endpoint_owner(&owner_kind, &owner_id)?;
    let tunnel = match exposure {
        ExposureScope::Loopback => ConnectivityTunnel::local_loopback(),
        ExposureScope::Private => ConnectivityTunnel::endpoint_stub(
            ConnectivityEndpointConfig::stub_private(endpoint_id.clone(), address_ref),
        ),
        ExposureScope::Public => ConnectivityTunnel::endpoint_stub(
            ConnectivityEndpointConfig::stub_public(endpoint_id.clone(), address_ref),
        ),
    };
    let resolved = tunnel
        .resolve_endpoint(owner, channel)
        .map_err(|error| format!("connectivity endpoint resolution failed: {error:?}"))?;
    let health = tunnel.check_reachability();
    let status = if resolved.permission_required {
        "blocked_pending_permission"
    } else {
        "active"
    };
    let exposure = ConnectivityExposureProjection {
        exposure_id: format!(
            "connectivity-exposure-{}",
            stable_cli_hash(&format!(
                "{}:{}",
                resolved.resolved_endpoint_id,
                exposure_scope_str(resolved.exposure)
            ))
        ),
        project_id: project_id(),
        connectivity_endpoint_id: resolved.connectivity_endpoint_id.clone(),
        owner_kind: resolved.owner.owner_kind.clone(),
        owner_id: resolved.owner.owner_id.clone(),
        channel_kind: channel_kind_str(resolved.channel_kind).to_string(),
        exposure: exposure_scope_str(resolved.exposure).to_string(),
        permission_scope: resolved.permission_scope.clone(),
        status: status.to_string(),
        capability_grant_id: None,
        health_status: health.status.clone(),
        reachable: health.reachable,
        revoked_at: None,
        updated_sequence: 0,
    };
    let sequence = if record {
        let event_kind = if resolved.permission_required {
            EventKind::ConnectivityExposureRequested
        } else {
            EventKind::ConnectivityExposureChanged
        };
        let mut event = NewEvent::new(
            format!(
                "event-connectivity-exposure-{}",
                stable_cli_hash(&exposure.exposure_id)
            ),
            event_kind,
            "capo-cli",
        );
        event.project_id = Some(exposure.project_id.clone());
        event.item_id = Some(exposure.exposure_id.clone());
        event.payload_json = format!(
            "{{\"exposure_id\":\"{}\",\"resolved_endpoint_id\":\"{}\",\"endpoint_id\":\"{}\",\"owner_kind\":\"{}\",\"owner_id\":\"{}\",\"channel\":\"{}\",\"exposure\":\"{}\",\"permission_scope\":\"{}\",\"status\":\"{}\"}}",
            escape_json(&exposure.exposure_id),
            escape_json(&resolved.resolved_endpoint_id),
            escape_json(&exposure.connectivity_endpoint_id),
            escape_json(&exposure.owner_kind),
            escape_json(&exposure.owner_id),
            escape_json(&exposure.channel_kind),
            escape_json(&exposure.exposure),
            escape_json(&exposure.permission_scope),
            escape_json(&exposure.status)
        );
        event.idempotency_key = Some(format!(
            "connectivity-exposure:{}:{}:{}:{}:{}:{}",
            exposure.project_id,
            exposure.connectivity_endpoint_id,
            exposure.owner_kind,
            exposure.owner_id,
            exposure.channel_kind,
            exposure.exposure
        ));
        event.redaction_state = RedactionState::Safe;
        Some(
            state(parsed)?
                .append_event(
                    event,
                    &[ProjectionRecord::ConnectivityExposure(exposure.clone())],
                )
                .map_err(debug_error)?,
        )
    } else {
        None
    };

    Ok(format!(
        "connectivity_exposure_planned=true\nexposure={}\nendpoint={}\nresolved_endpoint={}\nowner={}:{}\nchannel={}\nexposure_scope={}\npermission_required={}\npermission_scope={}\nstatus={}\nhealth={}\nreachable={}\nrecorded={}\nrecorded_sequence={}\n",
        exposure.exposure_id,
        exposure.connectivity_endpoint_id,
        resolved.resolved_endpoint_id,
        exposure.owner_kind,
        exposure.owner_id,
        exposure.channel_kind,
        exposure.exposure,
        resolved.permission_required,
        exposure.permission_scope,
        exposure.status,
        exposure.health_status,
        exposure.reachable,
        record,
        sequence
            .map(|sequence| sequence.to_string())
            .unwrap_or_else(|| "none".to_string())
    ))
}

fn request_connectivity_exposure_approval(
    parsed: &ParsedArgs,
    args: &[String],
) -> Result<String, String> {
    let exposure_id = required_arg(args, "--exposure")?;
    let approval_id = optional_arg(args, "--approval").unwrap_or_else(|| {
        format!(
            "approval-connectivity-exposure-{}",
            stable_cli_hash(&exposure_id)
        )
    });
    if let Some(unknown) = args
        .iter()
        .find(|arg| arg.starts_with("--") && !matches!(arg.as_str(), "--exposure" | "--approval"))
    {
        return Err(format!(
            "unknown connectivity request-approval option: {unknown}"
        ));
    }
    let state = state(parsed)?;
    let exposure = connectivity_exposure(&state, &exposure_id)?;
    if exposure.status != "blocked_pending_permission" {
        return Err(format!(
            "connectivity exposure is not awaiting permission: {} status={}",
            exposure.exposure_id, exposure.status
        ));
    }
    if state
        .permission_approval(&project_id(), &approval_id)
        .map_err(debug_error)?
        .is_some()
    {
        return Err(format!("approval already exists: {approval_id}"));
    }
    let scope_json = connectivity_exposure_scope_json(&exposure);
    let subject_json = connectivity_exposure_subject_json(&exposure);
    let approval = PermissionApprovalProjection {
        approval_id: approval_id.clone(),
        project_id: project_id(),
        session_id: None,
        tool_call_id: None,
        capability_profile_id: "remote-control-reviewed".to_string(),
        scope_json,
        subject_json,
        status: "pending".to_string(),
        requested_by: "local-user".to_string(),
        reason: format!("approve connectivity exposure {}", exposure.exposure_id),
        decision: None,
        capability_grant_id: None,
        updated_sequence: 0,
    };
    let mut event = NewEvent::new(
        format!(
            "event-connectivity-exposure-approval-{}",
            stable_cli_hash(&approval.approval_id)
        ),
        EventKind::PermissionApprovalQueued,
        "capo-cli",
    );
    event.project_id = Some(project_id());
    event.item_id = Some(exposure.exposure_id.clone());
    event.payload_json = format!(
        "{{\"approval_id\":\"{}\",\"exposure_id\":\"{}\",\"scope_json\":{},\"subject_json\":{},\"reason\":\"{}\"}}",
        escape_json(&approval.approval_id),
        escape_json(&exposure.exposure_id),
        approval.scope_json,
        approval.subject_json,
        escape_json(&approval.reason)
    );
    event.idempotency_key = Some(format!(
        "connectivity-exposure-approval:{}:{}:{}",
        exposure.project_id, exposure.exposure_id, approval.approval_id
    ));
    event.redaction_state = RedactionState::Safe;
    let sequence = state
        .append_event(
            event,
            &[ProjectionRecord::PermissionApproval(approval.clone())],
        )
        .map_err(debug_error)?;
    Ok(format!(
        "connectivity_exposure_approval_requested=true\nexposure={}\napproval={}\nstatus=pending\npermission_scope={}\nsequence={sequence}\n",
        exposure.exposure_id, approval.approval_id, exposure.permission_scope
    ))
}

fn activate_connectivity_exposure(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let exposure_id = required_arg(args, "--exposure")?;
    if let Some(unknown) = args
        .iter()
        .find(|arg| arg.starts_with("--") && !matches!(arg.as_str(), "--exposure"))
    {
        return Err(format!(
            "unknown connectivity activate-exposure option: {unknown}"
        ));
    }
    let state = state(parsed)?;
    let exposure = connectivity_exposure(&state, &exposure_id)?;
    if exposure.status == "revoked" {
        return Err(format!("connectivity exposure is revoked: {exposure_id}"));
    }
    if exposure.status == "active" {
        return Ok(render_connectivity_exposure_activation(
            &exposure,
            exposure.capability_grant_id.as_deref().unwrap_or("none"),
            None,
        ));
    }
    if exposure.status != "blocked_pending_permission" {
        return Err(format!(
            "connectivity exposure is not activatable: {} status={}",
            exposure.exposure_id, exposure.status
        ));
    }
    let grant = matching_connectivity_exposure_grant(&state, &exposure)?;
    let active = ConnectivityExposureProjection {
        status: "active".to_string(),
        capability_grant_id: Some(grant.capability_grant_id.clone()),
        health_status: if exposure.health_status == "unknown" {
            "available".to_string()
        } else {
            exposure.health_status.clone()
        },
        reachable: exposure.reachable,
        revoked_at: None,
        updated_sequence: 0,
        ..exposure.clone()
    };
    let mut event = NewEvent::new(
        format!(
            "event-connectivity-exposure-activated-{}",
            stable_cli_hash(&format!(
                "{}:{}",
                active.exposure_id, grant.capability_grant_id
            ))
        ),
        EventKind::ConnectivityExposureChanged,
        "capo-cli",
    );
    event.project_id = Some(active.project_id.clone());
    event.item_id = Some(active.exposure_id.clone());
    event.payload_json = format!(
        "{{\"exposure_id\":\"{}\",\"capability_grant_id\":\"{}\",\"status\":\"active\",\"permission_scope\":\"{}\"}}",
        escape_json(&active.exposure_id),
        escape_json(&grant.capability_grant_id),
        escape_json(&active.permission_scope)
    );
    event.idempotency_key = Some(format!(
        "connectivity-exposure-activate:{}:{}:{}",
        active.project_id, active.exposure_id, grant.capability_grant_id
    ));
    event.redaction_state = RedactionState::Safe;
    let sequence = state
        .append_event(
            event,
            &[ProjectionRecord::ConnectivityExposure(active.clone())],
        )
        .map_err(debug_error)?;
    Ok(render_connectivity_exposure_activation(
        &active,
        &grant.capability_grant_id,
        Some(sequence),
    ))
}

fn revoke_connectivity_exposure(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let exposure_id = required_arg(args, "--exposure")?;
    let reason = optional_arg(args, "--reason").unwrap_or_else(|| "operator_revoked".to_string());
    if let Some(unknown) = args
        .iter()
        .find(|arg| arg.starts_with("--") && !matches!(arg.as_str(), "--exposure" | "--reason"))
    {
        return Err(format!(
            "unknown connectivity revoke-exposure option: {unknown}"
        ));
    }
    let state = state(parsed)?;
    let exposure = connectivity_exposure(&state, &exposure_id)?;
    if exposure.status == "revoked" {
        return Ok(render_connectivity_exposure_revocation(
            &exposure, &reason, None,
        ));
    }
    let revoked_at = unix_timestamp_label()?;
    let revoked = ConnectivityExposureProjection {
        status: "revoked".to_string(),
        health_status: "disabled".to_string(),
        reachable: false,
        revoked_at: Some(revoked_at.clone()),
        updated_sequence: 0,
        ..exposure.clone()
    };
    let mut event = NewEvent::new(
        format!(
            "event-connectivity-exposure-revoked-{}",
            stable_cli_hash(&revoked.exposure_id)
        ),
        EventKind::ConnectivityExposureRevoked,
        "capo-cli",
    );
    event.project_id = Some(revoked.project_id.clone());
    event.item_id = Some(revoked.exposure_id.clone());
    event.payload_json = format!(
        "{{\"exposure_id\":\"{}\",\"status\":\"revoked\",\"reason\":\"{}\",\"revoked_at\":\"{}\"}}",
        escape_json(&revoked.exposure_id),
        escape_json(&reason),
        escape_json(&revoked_at)
    );
    event.idempotency_key = Some(format!(
        "connectivity-exposure-revoke:{}:{}",
        revoked.project_id, revoked.exposure_id
    ));
    event.redaction_state = RedactionState::Safe;
    let sequence = state
        .append_event(
            event,
            &[ProjectionRecord::ConnectivityExposure(revoked.clone())],
        )
        .map_err(debug_error)?;
    Ok(render_connectivity_exposure_revocation(
        &revoked,
        &reason,
        Some(sequence),
    ))
}

fn connectivity_exposure_status(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let latest = has_flag(args, "--latest");
    let exposure_id = optional_arg(args, "--exposure");
    let owner_kind = optional_arg(args, "--owner-kind");
    let owner_id = optional_arg(args, "--owner-id");
    let channel = optional_arg(args, "--channel");
    if let Some(unknown) = args.iter().find(|arg| {
        arg.starts_with("--")
            && !matches!(
                arg.as_str(),
                "--exposure" | "--latest" | "--owner-kind" | "--owner-id" | "--channel"
            )
    }) {
        return Err(format!(
            "unknown connectivity exposure-status option: {unknown}"
        ));
    }
    if latest && exposure_id.is_some() {
        return Err(
            "connectivity exposure-status accepts either --exposure or --latest".to_string(),
        );
    }
    if !latest && (owner_kind.is_some() || owner_id.is_some() || channel.is_some()) {
        return Err("connectivity exposure-status filters require --latest".to_string());
    }
    if let Some(kind) = owner_kind.as_deref() {
        endpoint_owner(kind, owner_id.as_deref().unwrap_or("filter-validation"))?;
    }
    if let Some(channel) = channel.as_deref() {
        parse_channel_kind(channel)?;
    }

    let state = state(parsed)?;
    let dashboard =
        project_dashboard(&state, ProjectDashboardQuery::new(project_id())).map_err(debug_error)?;
    let exposure = if latest {
        dashboard
            .latest_connectivity_exposure(
                owner_kind.as_deref(),
                owner_id.as_deref(),
                channel.as_deref(),
            )
            .ok_or_else(|| {
                let mut filters = Vec::new();
                if let Some(owner_kind) = owner_kind.as_deref() {
                    filters.push(format!("owner_kind={owner_kind}"));
                }
                if let Some(owner_id) = owner_id.as_deref() {
                    filters.push(format!("owner_id={owner_id}"));
                }
                if let Some(channel) = channel.as_deref() {
                    filters.push(format!("channel={channel}"));
                }
                if filters.is_empty() {
                    "no recorded connectivity exposures".to_string()
                } else {
                    format!(
                        "no recorded connectivity exposures matching {}",
                        filters.join(",")
                    )
                }
            })?
    } else {
        let exposure_id = exposure_id.ok_or_else(|| {
            "connectivity exposure-status requires --exposure or --latest".to_string()
        })?;
        dashboard
            .connectivity_exposure_status(&exposure_id)
            .ok_or_else(|| format!("missing connectivity exposure: {exposure_id}"))?
    };

    Ok(render_connectivity_exposure_status(exposure))
}

fn connectivity_exposure_evidence(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let exposure_id = required_arg(args, "--exposure")?;
    let out = PathBuf::from(required_arg(args, "--out")?);
    if let Some(unknown) = args
        .iter()
        .find(|arg| arg.starts_with("--") && !matches!(arg.as_str(), "--exposure" | "--out"))
    {
        return Err(format!(
            "unknown connectivity exposure-evidence option: {unknown}"
        ));
    }
    let state = state(parsed)?;
    let exposure = connectivity_exposure(&state, &exposure_id)?;
    let project_id = project_id();
    let command = envelope(
        "connectivity-exposure-evidence",
        CommandTarget::Project(project_id.clone()),
        CommandIntent::ExportEvidence,
        Some(exposure.exposure_id.clone()),
    );
    let markdown = render_connectivity_exposure_evidence(&project_id, &exposure);
    fs::create_dir_all(&out).map_err(|error| error.to_string())?;
    let content_hash = stable_cli_hash(&markdown);
    let artifact_id = format!("artifact-connectivity-exposure-evidence-{content_hash}");
    let path = out.join(format!("{artifact_id}.md"));
    write_connectivity_exposure_evidence_file(&path, &markdown)?;
    state
        .record_artifact(ArtifactRecord {
            artifact_id: artifact_id.clone(),
            project_id: Some(project_id.clone()),
            session_id: None,
            run_id: None,
            kind: "connectivity_exposure_evidence".to_string(),
            uri: path.display().to_string(),
            content_hash: content_hash.clone(),
            size_bytes: markdown.len() as i64,
            redaction_state: RedactionState::Safe,
        })
        .map_err(debug_error)?;
    let evidence_id = format!("evidence-{artifact_id}");
    let sequence = state
        .append_event(
            NewEvent {
                event_id: format!("event-{evidence_id}"),
                kind: EventKind::EvidenceRecorded,
                actor: "cli".to_string(),
                project_id: Some(project_id.clone()),
                task_id: None,
                agent_id: None,
                session_id: None,
                run_id: None,
                turn_id: None,
                item_id: Some(evidence_id.clone()),
                payload_json: format!(
                    "{{\"artifact_id\":\"{}\",\"content_hash\":\"{}\",\"exposure_id\":\"{}\",\"status\":\"{}\"}}",
                    escape_json(&artifact_id),
                    escape_json(&content_hash),
                    escape_json(&exposure.exposure_id),
                    escape_json(&exposure.status)
                ),
                idempotency_key: Some(format!(
                    "connectivity-exposure-evidence:{}:{}:{content_hash}",
                    project_id, exposure.exposure_id
                )),
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::Evidence(EvidenceProjection {
                evidence_id: capo_core::EvidenceId::new(evidence_id.clone()),
                project_id: project_id.clone(),
                task_id: None,
                session_id: None,
                run_id: None,
                kind: "connectivity_exposure_evidence".to_string(),
                artifact_id: Some(artifact_id.clone()),
                confidence: connectivity_exposure_evidence_confidence(&exposure),
                updated_sequence: 0,
            })],
        )
        .map_err(debug_error)?;

    Ok(format!(
        "connectivity_exposure_evidence_exported=true\nexposure={}\nevidence_id={evidence_id}\nartifact_id={artifact_id}\npath={}\ncontent_hash={content_hash}\nsequence={sequence}\ncommand_id={}\n",
        exposure.exposure_id,
        path.display(),
        command.command_id
    ))
}

fn connectivity_exposure_evidence_confidence(exposure: &ConnectivityExposureProjection) -> i64 {
    if exposure.status == "active" && exposure.capability_grant_id.is_some() {
        85
    } else if exposure.status == "revoked" {
        80
    } else {
        65
    }
}

fn render_connectivity_exposure_evidence(
    project_id: &ProjectId,
    exposure: &ConnectivityExposureProjection,
) -> String {
    format!(
        "<!-- capo:connectivity-exposure-evidence -->\n# Capo Connectivity Exposure Evidence - {}\n\n## Objective\n\nReview a recorded connectivity exposure without opening tunnels or touching runtime/provider processes.\n\n## Exposure\n\n- Project: `{}`\n- Exposure: `{}`\n- Endpoint: `{}`\n- Owner: `{}:{}`\n- Channel: `{}`\n- Exposure scope: `{}`\n- Permission scope: `{}`\n- Status: `{}`\n- Health: `{}`\n- Reachable: `{}`\n- Linked grant: `{}`\n- Revoked at: `{}`\n- Updated sequence: `{}`\n\n## Review Notes\n\n- Active exposure requires a matching durable allow grant before the exposure state can become active.\n- Revocation disables the exposure read model and marks it unreachable while preserving historical grant evidence.\n- This artifact records Capo connectivity metadata only; it is not proof of real tunnel reachability.\n\n## Evidence Policy\n\n- This report is derived from persisted Capo connectivity read models only.\n- It does not open tunnels, launch runtimes, launch provider CLIs, inspect credentials, materialize prompts, or mutate exposure state.\n- Credential material, tokens, cookies, subscription sessions, raw prompts, and provider output are not rendered.\n",
        exposure.exposure_id,
        project_id,
        exposure.exposure_id,
        exposure.connectivity_endpoint_id,
        exposure.owner_kind,
        exposure.owner_id,
        exposure.channel_kind,
        exposure.exposure,
        exposure.permission_scope,
        exposure.status,
        exposure.health_status,
        exposure.reachable,
        exposure.capability_grant_id.as_deref().unwrap_or("none"),
        exposure.revoked_at.as_deref().unwrap_or("none"),
        exposure.updated_sequence
    )
}

fn render_connectivity_exposure_status(exposure: &ConnectivityExposureProjection) -> String {
    format!(
        "connectivity_exposure_status=true\nexposure={}\nendpoint={}\nowner={}:{}\nchannel={}\nexposure_scope={}\npermission_scope={}\nstatus={}\ngrant={}\nhealth={}\nreachable={}\nrevoked_at={}\nupdated_sequence={}\n",
        exposure.exposure_id,
        exposure.connectivity_endpoint_id,
        exposure.owner_kind,
        exposure.owner_id,
        exposure.channel_kind,
        exposure.exposure,
        exposure.permission_scope,
        exposure.status,
        exposure.capability_grant_id.as_deref().unwrap_or("none"),
        exposure.health_status,
        exposure.reachable,
        exposure.revoked_at.as_deref().unwrap_or("none"),
        exposure.updated_sequence
    )
}

fn render_connectivity_exposure_activation(
    exposure: &ConnectivityExposureProjection,
    grant_id: &str,
    sequence: Option<i64>,
) -> String {
    format!(
        "connectivity_exposure_activated=true\nexposure={}\nendpoint={}\nowner={}:{}\nchannel={}\nexposure_scope={}\npermission_scope={}\nstatus={}\ngrant={}\nhealth={}\nreachable={}\nrecorded_sequence={}\n",
        exposure.exposure_id,
        exposure.connectivity_endpoint_id,
        exposure.owner_kind,
        exposure.owner_id,
        exposure.channel_kind,
        exposure.exposure,
        exposure.permission_scope,
        exposure.status,
        grant_id,
        exposure.health_status,
        exposure.reachable,
        sequence
            .map(|sequence| sequence.to_string())
            .unwrap_or_else(|| "none".to_string())
    )
}

fn render_connectivity_exposure_revocation(
    exposure: &ConnectivityExposureProjection,
    reason: &str,
    sequence: Option<i64>,
) -> String {
    format!(
        "connectivity_exposure_revoked=true\nexposure={}\nendpoint={}\nowner={}:{}\nchannel={}\nexposure_scope={}\npermission_scope={}\nstatus={}\ngrant={}\nhealth={}\nreachable={}\nrevoked_at={}\nreason={}\nrecorded_sequence={}\n",
        exposure.exposure_id,
        exposure.connectivity_endpoint_id,
        exposure.owner_kind,
        exposure.owner_id,
        exposure.channel_kind,
        exposure.exposure,
        exposure.permission_scope,
        exposure.status,
        exposure.capability_grant_id.as_deref().unwrap_or("none"),
        exposure.health_status,
        exposure.reachable,
        exposure.revoked_at.as_deref().unwrap_or("none"),
        reason,
        sequence
            .map(|sequence| sequence.to_string())
            .unwrap_or_else(|| "none".to_string())
    )
}

fn connectivity_exposure(
    state: &SqliteStateStore,
    exposure_id: &str,
) -> Result<ConnectivityExposureProjection, String> {
    state
        .connectivity_exposures(&project_id())
        .map_err(debug_error)?
        .into_iter()
        .rev()
        .find(|exposure| exposure.exposure_id == exposure_id)
        .ok_or_else(|| format!("missing connectivity exposure: {exposure_id}"))
}

fn matching_connectivity_exposure_grant(
    state: &SqliteStateStore,
    exposure: &ConnectivityExposureProjection,
) -> Result<CapabilityGrantProjection, String> {
    let expected_subject = connectivity_exposure_subject_value(exposure);
    state
        .capability_grants()
        .map_err(debug_error)?
        .into_iter()
        .rev()
        .find(|grant| {
            grant.effect == "allow"
                && scope_values(&grant.scope_json)
                    .map(|scopes| {
                        scopes
                            .iter()
                            .any(|scope| scope == &exposure.permission_scope)
                    })
                    .unwrap_or(false)
                && subject_contains(&grant.subject_json, &expected_subject)
        })
        .ok_or_else(|| {
            format!(
                "missing allow grant for connectivity exposure {} scope={}",
                exposure.exposure_id, exposure.permission_scope
            )
        })
}

fn connectivity_exposure_scope_json(exposure: &ConnectivityExposureProjection) -> String {
    format!("[\"{}\"]", escape_json(&exposure.permission_scope))
}

fn connectivity_exposure_subject_json(exposure: &ConnectivityExposureProjection) -> String {
    connectivity_exposure_subject_value(exposure).to_string()
}

fn connectivity_exposure_subject_value(
    exposure: &ConnectivityExposureProjection,
) -> serde_json::Value {
    serde_json::json!({
        "exposure_id": exposure.exposure_id,
        "endpoint_id": exposure.connectivity_endpoint_id,
        "owner_kind": exposure.owner_kind,
        "owner_id": exposure.owner_id,
        "channel": exposure.channel_kind,
        "exposure": exposure.exposure,
    })
}

fn subject_contains(subject_json: &str, expected: &serde_json::Value) -> bool {
    let Ok(serde_json::Value::Object(subject)) =
        serde_json::from_str::<serde_json::Value>(subject_json)
    else {
        return false;
    };
    let Some(expected) = expected.as_object() else {
        return false;
    };
    expected
        .iter()
        .all(|(key, value)| subject.get(key) == Some(value))
}

fn unix_timestamp_label() -> Result<String, String> {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system time before unix epoch: {error}"))?
        .as_secs();
    Ok(format!("unix:{seconds}"))
}

fn parse_channel_kind(value: &str) -> Result<ChannelKind, String> {
    match value {
        "control" => Ok(ChannelKind::Control),
        "stdio" => Ok(ChannelKind::Stdio),
        "logs" => Ok(ChannelKind::Logs),
        "dashboard" => Ok(ChannelKind::Dashboard),
        "artifact" => Ok(ChannelKind::Artifact),
        other => Err(format!("unsupported channel kind: {other}")),
    }
}

fn channel_kind_str(value: ChannelKind) -> &'static str {
    match value {
        ChannelKind::Control => "control",
        ChannelKind::Stdio => "stdio",
        ChannelKind::Logs => "logs",
        ChannelKind::Dashboard => "dashboard",
        ChannelKind::Artifact => "artifact",
    }
}

fn parse_exposure_scope(value: &str) -> Result<ExposureScope, String> {
    match value {
        "loopback" => Ok(ExposureScope::Loopback),
        "private" => Ok(ExposureScope::Private),
        "public" => Ok(ExposureScope::Public),
        other => Err(format!("unsupported exposure scope: {other}")),
    }
}

fn exposure_scope_str(value: ExposureScope) -> &'static str {
    match value {
        ExposureScope::Loopback => "loopback",
        ExposureScope::Private => "private",
        ExposureScope::Public => "public",
    }
}

fn endpoint_owner(owner_kind: &str, owner_id: &str) -> Result<EndpointOwner, String> {
    match owner_kind {
        "runtime_target" => Ok(EndpointOwner::runtime_target(owner_id)),
        "capo_server" => Ok(EndpointOwner::capo_server(owner_id)),
        other => Err(format!("unsupported endpoint owner kind: {other}")),
    }
}

fn index_workpads(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let root = PathBuf::from(required_arg(args, "--root")?);
    let index = index_project_workpads(&root)?;
    let command = envelope(
        "workpad-index",
        CommandTarget::Project(project_id()),
        CommandIntent::IndexWorkpads,
        Some(root.display().to_string()),
    );
    let state = state(parsed)?;
    let existing_statuses = state
        .workpad_tasks(&project_id())
        .map_err(debug_error)?
        .into_iter()
        .map(|task| (task.workpad_task_id, task.capo_execution_status))
        .collect::<HashMap<_, _>>();
    let projections = workpad_index_projections(&index, &existing_statuses);
    let index_fingerprint = index
        .files
        .iter()
        .map(|file| file.content_hash.as_str())
        .collect::<Vec<_>>()
        .join(":");
    let next_sequence_hint = state.last_sequence().map_err(debug_error)? + 1;
    let event_suffix = stable_cli_hash(&format!(
        "{}:{}:{index_fingerprint}",
        root.display(),
        next_sequence_hint
    ));
    let mut event = NewEvent::new(
        format!("event-workpad-index-{}-{event_suffix}", index.observed_unix),
        EventKind::WorkpadIndexed,
        "capo-cli",
    );
    event.project_id = Some(project_id());
    event.payload_json = format!(
        "{{\"root\":\"{}\",\"files\":{},\"tasks\":{},\"observed_unix\":{}}}",
        escape_json(&root.display().to_string()),
        index.files.len(),
        index.tasks.len(),
        index.observed_unix
    );
    event.idempotency_key = None;
    event.redaction_state = RedactionState::Safe;
    let sequence = state
        .append_event(event, &projections)
        .map_err(debug_error)?;
    Ok(format!(
        "workpads_indexed=true\nroot={}\nfiles={}\ntasks={}\nsequence={sequence}\ncommand_id={}\n",
        root.display(),
        index.files.len(),
        index.tasks.len(),
        command.command_id
    ))
}

fn default_workpad_task_id(workpad_task_id: &str) -> String {
    format!("task-workpad-{}", sanitize_id_component(workpad_task_id))
}

fn next_workpad_task(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let path_filter = workpad_path_filter(args)?;
    let state = state(parsed)?;
    let (next, candidate_count) = next_workpad_selection(&state, path_filter.as_deref())?;
    Ok(render_next_workpad_task(
        next.as_ref(),
        candidate_count,
        path_filter.as_deref(),
    ))
}

fn plan_next_workpad_task(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let options = WorkpadPlanNextOptions::parse(args, &parsed.state_root)?;
    validate_local_launch_adapter(&options.adapter)?;
    let state = state(parsed)?;
    let (next, candidate_count) = next_workpad_selection(&state, options.path_filter.as_deref())?;
    let next = next.ok_or_else(|| "no actionable observed-only workpad task found".to_string())?;
    let workpad_file = state
        .workpad_file(&project_id(), &next.path)
        .map_err(debug_error)?
        .ok_or_else(|| format!("missing workpad file read model: {}", next.path))?;
    let goal = workpad_task_goal(&next);
    let plan = recordable_adapter_dispatch_plan(
        parsed,
        DispatchPlanRecordRequest {
            adapter: &options.adapter,
            agent: &options.agent,
            goal: &goal,
            workspace: options.workspace,
            artifacts: options.artifacts,
            prompt_source: DispatchPromptSourceInput::workpad_task(
                &next,
                workpad_file.content_hash,
            ),
            record: options.record,
        },
    )?;
    Ok(format!(
        "workpad_next_planned=true\nagent={}\nadapter={}\nworkpad_task_id={}\ndefault_task_id={}\nsource={}#{}\ntitle={}\nobserved_status={}\ncapo_execution_status={}\ncandidate_count={}\npath_filter={}\n{}\n",
        options.agent,
        plan.projection.adapter_kind,
        next.workpad_task_id,
        default_workpad_task_id(&next.workpad_task_id),
        next.path,
        next.source_anchor,
        next.title,
        next.observed_status,
        next.capo_execution_status,
        candidate_count,
        options.path_filter.as_deref().unwrap_or("none"),
        render_adapter_dispatch_plan(&plan)
    ))
}

fn start_next_workpad_task(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let agent = required_arg(args, "--agent")?;
    let args_without_agent = remove_option(args, "--agent");
    let path_filter = workpad_path_filter(&args_without_agent)?;
    let state = state(parsed)?;
    let (next, _) = next_workpad_selection(&state, path_filter.as_deref())?;
    let next = next.ok_or_else(|| "no actionable observed-only workpad task found".to_string())?;
    if state.agent_by_name(&agent).map_err(debug_error)?.is_none() {
        return Err(format!("missing registered agent: {agent}"));
    }
    let task_id = default_workpad_task_id(&next.workpad_task_id);
    import_workpad_task(
        parsed,
        &[
            "--workpad-task".to_string(),
            next.workpad_task_id.clone(),
            "--task".to_string(),
            task_id.clone(),
        ],
    )?;
    let goal = format!(
        "Work on {} from {}#{} (workpad_task_id={})",
        next.title, next.path, next.source_anchor, next.workpad_task_id
    );
    let mut command = envelope(
        "workpad-start-next",
        CommandTarget::Agent(AgentId::new(format!("agent-{agent}"))),
        CommandIntent::SendTask,
        Some(goal),
    );
    command
        .structured_args
        .push(("agent".to_string(), agent.clone()));
    command
        .structured_args
        .push(("scenario".to_string(), "workpad".to_string()));
    command
        .structured_args
        .push(("task_id".to_string(), task_id.clone()));
    let refs = controller(parsed)?
        .send_task_command(&command)
        .map_err(debug_error)?;
    Ok(format!(
        "workpad_next_started=true\nagent={agent}\nworkpad_task_id={}\ntask_id={}\nsession_id={}\nrun_id={}\nsource={}#{}\nobserved_status={}\ncapo_execution_status=active\ncommand_id={}\n",
        next.workpad_task_id,
        refs.task_id,
        refs.session_id,
        refs.run_id,
        next.path,
        next.source_anchor,
        next.observed_status,
        command.command_id
    ))
}

fn workpad_task_goal(task: &WorkpadTaskProjection) -> String {
    format!(
        "Work on {} from {}#{} (workpad_task_id={})",
        task.title, task.path, task.source_anchor, task.workpad_task_id
    )
}

fn workpad_path_filter(args: &[String]) -> Result<Option<String>, String> {
    let mut path_filter = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--path" => {
                let value = args
                    .get(index + 1)
                    .filter(|value| !value.starts_with("--"))
                    .ok_or_else(|| "--path requires a value".to_string())?;
                path_filter = Some(value.clone());
                index += 2;
            }
            other => return Err(format!("unknown workpad next option: {other}")),
        }
    }
    Ok(path_filter)
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct WorkpadPlanNextOptions {
    agent: String,
    adapter: String,
    path_filter: Option<String>,
    workspace: PathBuf,
    artifacts: PathBuf,
    record: bool,
}

impl WorkpadPlanNextOptions {
    fn parse(args: &[String], state_root: &Path) -> Result<Self, String> {
        let mut agent = None;
        let mut adapter = None;
        let mut path_filter = None;
        let mut workspace = None;
        let mut artifacts = None;
        let mut record = false;
        let mut index = 0;
        while index < args.len() {
            match args[index].as_str() {
                "--agent" => {
                    agent = Some(
                        args.get(index + 1)
                            .filter(|value| !value.starts_with("--"))
                            .ok_or_else(|| "--agent requires a value".to_string())?
                            .clone(),
                    );
                    index += 2;
                }
                "--adapter" => {
                    adapter = Some(
                        args.get(index + 1)
                            .filter(|value| !value.starts_with("--"))
                            .ok_or_else(|| "--adapter requires a value".to_string())?
                            .clone(),
                    );
                    index += 2;
                }
                "--path" => {
                    path_filter = Some(
                        args.get(index + 1)
                            .filter(|value| !value.starts_with("--"))
                            .ok_or_else(|| "--path requires a value".to_string())?
                            .clone(),
                    );
                    index += 2;
                }
                "--workspace" => {
                    workspace = Some(PathBuf::from(
                        args.get(index + 1)
                            .filter(|value| !value.starts_with("--"))
                            .ok_or_else(|| "--workspace requires a value".to_string())?,
                    ));
                    index += 2;
                }
                "--artifacts" => {
                    artifacts = Some(PathBuf::from(
                        args.get(index + 1)
                            .filter(|value| !value.starts_with("--"))
                            .ok_or_else(|| "--artifacts requires a value".to_string())?,
                    ));
                    index += 2;
                }
                "--record" => {
                    record = true;
                    index += 1;
                }
                other => return Err(format!("unknown workpad plan-next option: {other}")),
            }
        }
        Ok(Self {
            agent: agent.ok_or_else(|| "--agent is required".to_string())?,
            adapter: adapter.ok_or_else(|| "--adapter is required".to_string())?,
            path_filter,
            workspace: workspace
                .unwrap_or_else(|| state_root.join("workpad-plan-next").join("workspace")),
            artifacts: artifacts
                .unwrap_or_else(|| state_root.join("workpad-plan-next").join("artifacts")),
            record,
        })
    }
}

fn next_workpad_selection(
    state: &SqliteStateStore,
    path_filter: Option<&str>,
) -> Result<(Option<WorkpadTaskProjection>, usize), String> {
    let mut query = ProjectDashboardQuery::new(project_id());
    if let Some(path) = path_filter {
        query = query.with_workpad_path(path);
    }
    let dashboard = project_dashboard(state, query).map_err(debug_error)?;
    Ok((
        dashboard.next_workpad_task().cloned(),
        dashboard.next_workpad_candidate_count(),
    ))
}

fn render_next_workpad_task(
    next: Option<&WorkpadTaskProjection>,
    candidate_count: usize,
    path_filter: Option<&str>,
) -> String {
    let Some(next) = next else {
        return format!(
            "workpad_next_found=false\ncandidate_count=0\npath_filter={}\n",
            path_filter.unwrap_or("none")
        );
    };
    format!(
        "workpad_next_found=true\ncandidate_count={}\nworkpad_task_id={}\ndefault_task_id={}\npath={}\nsource_anchor={}\nsource={}#{}\ntitle={}\nobserved_status={}\ncapo_execution_status={}\npath_filter={}\n",
        candidate_count,
        next.workpad_task_id,
        default_workpad_task_id(&next.workpad_task_id),
        next.path,
        next.source_anchor,
        next.path,
        next.source_anchor,
        next.title,
        next.observed_status,
        next.capo_execution_status,
        path_filter.unwrap_or("none")
    )
}

fn remove_option(args: &[String], key: &str) -> Vec<String> {
    let mut filtered = Vec::new();
    let mut index = 0;
    while index < args.len() {
        if args[index] == key {
            index += 2;
        } else {
            filtered.push(args[index].clone());
            index += 1;
        }
    }
    filtered
}

fn import_workpad_task(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let workpad_task_id = required_arg(args, "--workpad-task")?;
    let state = state(parsed)?;
    let project_id = project_id();
    let workpad_task = state
        .workpad_task(&project_id, &workpad_task_id)
        .map_err(debug_error)?
        .ok_or_else(|| format!("missing workpad task read model: {workpad_task_id}"))?;
    let workpad_file = state
        .workpad_file(&project_id, &workpad_task.path)
        .map_err(debug_error)?
        .ok_or_else(|| format!("missing workpad file read model: {}", workpad_task.path))?;

    if let Some(expected_hash) = optional_arg(args, "--expected-hash")
        && expected_hash != workpad_file.content_hash
    {
        return Err(format!(
            "source drift detected for {}: expected_hash={} current_hash={}",
            workpad_task.path, expected_hash, workpad_file.content_hash
        ));
    }

    let task_id = TaskId::new(
        optional_arg(args, "--task")
            .unwrap_or_else(|| default_workpad_task_id(&workpad_task.workpad_task_id)),
    );
    if let Some(existing_task) = state.task(&task_id).map_err(debug_error)? {
        let same_source = existing_task
            .latest_summary
            .as_deref()
            .is_some_and(|summary| {
                summary.contains(&format!("workpad_task_id={}", workpad_task.workpad_task_id))
            });
        if !same_source
            || existing_task.capo_execution_status != "ready"
            || existing_task.active_session_id.is_some()
        {
            return Err(format!(
                "refusing to overwrite existing Capo task read model: {task_id}"
            ));
        }
    }
    let mut command = envelope(
        "workpad-import",
        CommandTarget::Task(task_id.clone()),
        CommandIntent::ImportWorkpadTask,
        Some(workpad_task.title.clone()),
    );
    command.structured_args.push((
        "workpad_task_id".to_string(),
        workpad_task.workpad_task_id.clone(),
    ));
    command
        .structured_args
        .push(("source_hash".to_string(), workpad_file.content_hash.clone()));
    let source_ref = format!("{}#{}", workpad_task.path, workpad_task.source_anchor);
    let latest_summary = format!(
        "source={} hash={} observed_status={} workpad_task_id={}",
        source_ref,
        workpad_file.content_hash,
        workpad_task.observed_status,
        workpad_task.workpad_task_id
    );
    let imported_workpad_task = WorkpadTaskProjection {
        capo_execution_status: "imported".to_string(),
        ..workpad_task.clone()
    };
    let task_projection = ProjectionRecord::Task(capo_state::TaskProjection {
        task_id: task_id.clone(),
        project_id: project_id.clone(),
        title: workpad_task.title.clone(),
        capo_execution_status: "ready".to_string(),
        active_session_id: None,
        latest_summary: Some(latest_summary),
        evidence_id: None,
        updated_sequence: 0,
    });
    let mut event = NewEvent::new(
        format!(
            "event-workpad-import-{}",
            stable_cli_hash(&format!(
                "{}:{}:{}",
                task_id, workpad_task.workpad_task_id, workpad_file.content_hash
            ))
        ),
        EventKind::WorkpadTaskImported,
        "capo-cli",
    );
    event.project_id = Some(project_id.clone());
    event.task_id = Some(task_id.clone());
    event.payload_json = format!(
        "{{\"task_id\":\"{}\",\"workpad_task_id\":\"{}\",\"path\":\"{}\",\"source_anchor\":\"{}\",\"content_hash\":\"{}\",\"observed_status\":\"{}\"}}",
        escape_json(task_id.as_str()),
        escape_json(&workpad_task.workpad_task_id),
        escape_json(&workpad_task.path),
        escape_json(&workpad_task.source_anchor),
        escape_json(&workpad_file.content_hash),
        escape_json(&workpad_task.observed_status)
    );
    event.idempotency_key = Some(format!(
        "workpad-import:{}:{}:{}",
        task_id, workpad_task.workpad_task_id, workpad_file.content_hash
    ));
    event.redaction_state = RedactionState::Safe;
    let sequence = state
        .append_event(
            event,
            &[
                task_projection,
                ProjectionRecord::WorkpadTask(imported_workpad_task),
            ],
        )
        .map_err(debug_error)?;

    Ok(format!(
        "workpad_task_imported=true\nworkpad_task_id={}\ntask_id={}\nsource={}#{}\nsource_hash={}\nobserved_status={}\ncapo_execution_status=ready\nsequence={sequence}\ncommand_id={}\n",
        workpad_task.workpad_task_id,
        task_id,
        workpad_task.path,
        workpad_task.source_anchor,
        workpad_file.content_hash,
        workpad_task.observed_status,
        command.command_id
    ))
}

fn propose_workpad_update(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let workpad_task_id = required_arg(args, "--workpad-task")?;
    let out = PathBuf::from(required_arg(args, "--out")?);
    let state = state(parsed)?;
    let project_id = project_id();
    let workpad_task = state
        .workpad_task(&project_id, &workpad_task_id)
        .map_err(debug_error)?
        .ok_or_else(|| format!("missing workpad task read model: {workpad_task_id}"))?;
    let workpad_file = state
        .workpad_file(&project_id, &workpad_task.path)
        .map_err(debug_error)?
        .ok_or_else(|| format!("missing workpad file read model: {}", workpad_task.path))?;

    if let Some(expected_hash) = optional_arg(args, "--expected-hash")
        && expected_hash != workpad_file.content_hash
    {
        return Err(format!(
            "source drift detected for {}: expected_hash={} current_hash={}",
            workpad_task.path, expected_hash, workpad_file.content_hash
        ));
    }

    let task_id = TaskId::new(
        optional_arg(args, "--task").unwrap_or_else(|| default_workpad_task_id(&workpad_task_id)),
    );
    let summary = optional_arg(args, "--summary").unwrap_or_else(|| {
        format!(
            "Review imported workpad task `{}` before any source markdown update.",
            workpad_task.workpad_task_id
        )
    });
    let command = envelope(
        "workpad-propose",
        CommandTarget::Task(task_id.clone()),
        CommandIntent::WriteWorkpadProposal,
        Some(summary.clone()),
    );
    fs::create_dir_all(&out).map_err(|error| error.to_string())?;
    let proposal_identity = stable_cli_hash(&format!(
        "{}:{}:{}:{}",
        task_id, workpad_task.workpad_task_id, workpad_file.content_hash, summary
    ));
    let artifact_id = format!("artifact-workpad-proposal-{proposal_identity}");
    let path = out.join(format!("{artifact_id}.md"));
    let markdown = render_workpad_proposal(
        &task_id,
        &workpad_task,
        &workpad_file,
        &summary,
        &artifact_id,
    );
    write_workpad_proposal_file(&path, &markdown)?;
    let content_hash = stable_cli_hash(&markdown);
    state
        .record_artifact(ArtifactRecord {
            artifact_id: artifact_id.clone(),
            project_id: Some(project_id.clone()),
            session_id: None,
            run_id: None,
            kind: "workpad_update_proposal".to_string(),
            uri: path.display().to_string(),
            content_hash: content_hash.clone(),
            size_bytes: markdown.len() as i64,
            redaction_state: RedactionState::Safe,
        })
        .map_err(debug_error)?;
    let evidence_id = format!("evidence-{artifact_id}");
    let mut event = NewEvent::new(
        format!("event-workpad-proposal-{}", stable_cli_hash(&artifact_id)),
        EventKind::WorkpadProposalWritten,
        "capo-cli",
    );
    event.project_id = Some(project_id.clone());
    event.task_id = Some(task_id.clone());
    event.payload_json = format!(
        "{{\"task_id\":\"{}\",\"workpad_task_id\":\"{}\",\"artifact_id\":\"{}\",\"path\":\"{}\",\"content_hash\":\"{}\",\"source_hash\":\"{}\"}}",
        escape_json(task_id.as_str()),
        escape_json(&workpad_task.workpad_task_id),
        escape_json(&artifact_id),
        escape_json(&path.display().to_string()),
        escape_json(&content_hash),
        escape_json(&workpad_file.content_hash)
    );
    event.idempotency_key = Some(format!(
        "workpad-proposal:{}:{}:{}:{}",
        task_id, workpad_task.workpad_task_id, workpad_file.content_hash, proposal_identity
    ));
    event.redaction_state = RedactionState::Safe;
    let sequence = state
        .append_event(
            event,
            &[ProjectionRecord::Evidence(EvidenceProjection {
                evidence_id: capo_core::EvidenceId::new(evidence_id.clone()),
                project_id,
                task_id: Some(task_id.clone()),
                session_id: None,
                run_id: None,
                kind: "workpad_update_proposal".to_string(),
                artifact_id: Some(artifact_id.clone()),
                confidence: 80,
                updated_sequence: 0,
            })],
        )
        .map_err(debug_error)?;

    Ok(format!(
        "workpad_proposal_written=true\nworkpad_task_id={}\ntask_id={}\nartifact_id={artifact_id}\npath={}\nsource_hash={}\ncontent_hash={content_hash}\nsequence={sequence}\ncommand_id={}\n",
        workpad_task.workpad_task_id,
        task_id,
        path.display(),
        workpad_file.content_hash,
        command.command_id
    ))
}

fn apply_workpad_proposal(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let proposal = PathBuf::from(required_arg(args, "--proposal")?);
    let command = envelope(
        "workpad-apply",
        CommandTarget::Project(project_id()),
        CommandIntent::ApplyWorkpadProposal,
        Some(proposal.display().to_string()),
    );
    if !has_flag(args, "--confirm") {
        return Err(
            "explicit --confirm is required before Capo applies workpad source changes".to_string(),
        );
    }
    let markdown = fs::read_to_string(&proposal).map_err(|error| error.to_string())?;
    if !markdown.starts_with("<!-- capo:workpad-proposal -->") {
        return Err(format!(
            "refusing to apply non-Capo workpad proposal: {}",
            proposal.display()
        ));
    }
    let _state = state(parsed)?;
    Ok(format!(
        "workpad_apply_supported=false\nproposal={}\nsource_modified=false\nreason=DB3 only supports reviewed proposal artifacts; apply manually after review using the rollback instructions in the proposal.\ncommand_id={}\n",
        proposal.display(),
        command.command_id
    ))
}

fn export_evidence(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
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

fn export_task_outcome_report(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
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
                    evidence_id: capo_core::EvidenceId::new(evidence_id.clone()),
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

fn record_review_finding(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
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
    let follow_up_workpad_task_id = optional_arg(args, "--follow-up-workpad-task");
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
        return Err(format!("missing follow-up workpad task: {workpad_task_id}"));
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
                    evidence_id: capo_core::EvidenceId::new(evidence_id.clone()),
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

fn workpad_index_projections(
    index: &WorkpadIndex,
    existing_statuses: &HashMap<String, String>,
) -> Vec<ProjectionRecord> {
    let project_id = project_id();
    let mut projections = vec![ProjectionRecord::WorkpadIndexReset(
        WorkpadIndexResetProjection {
            project_id: project_id.clone(),
            observed_unix: index.observed_unix,
            updated_sequence: 0,
        },
    )];
    for file in &index.files {
        projections.push(ProjectionRecord::WorkpadFile(WorkpadFileProjection {
            path: file.path.clone(),
            project_id: project_id.clone(),
            content_hash: file.content_hash.clone(),
            headings: file.headings.join("\n"),
            objective: file.objective.clone(),
            observed_unix: index.observed_unix,
            updated_sequence: 0,
        }));
    }
    for task in &index.tasks {
        projections.push(ProjectionRecord::WorkpadTask(WorkpadTaskProjection {
            workpad_task_id: task.workpad_task_id.clone(),
            project_id: project_id.clone(),
            path: task.path.clone(),
            source_anchor: task.source_anchor.clone(),
            title: task.title.clone(),
            observed_status: task.observed_status.clone(),
            capo_execution_status: existing_statuses
                .get(&task.workpad_task_id)
                .cloned()
                .unwrap_or_else(|| task.capo_execution_status.clone()),
            observed_unix: index.observed_unix,
            updated_sequence: 0,
        }));
    }
    projections
}

fn sanitize_id_component(value: &str) -> String {
    let mut sanitized = String::new();
    let mut previous_dash = false;
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            sanitized.push(ch.to_ascii_lowercase());
            previous_dash = false;
        } else if !previous_dash {
            sanitized.push('-');
            previous_dash = true;
        }
    }
    let trimmed = sanitized.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "workpad-task".to_string()
    } else {
        trimmed
    }
}

fn escape_json(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

fn stable_cli_hash(value: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn render_status(
    command: &CommandEnvelope,
    observation: &capo_controller::FakeReadModelObservation,
    evidence: &[EvidenceProjection],
) -> String {
    let mut output = format!(
        "command_id={}\nagent={} agent_status={}\nsession_id={} session_status={}\nrun_id={} run_status={}\ncurrent_goal={}\nlatest_summary={}\nconfidence={}\nblocker={}\nevidence_refs={}\nrecent_events={}\n",
        command.command_id,
        observation.agent.name,
        observation.agent.status,
        observation.session.session_id,
        observation.session.status,
        observation.run.run_id,
        observation.run.status,
        observation.session.current_goal,
        observation
            .session
            .latest_summary
            .as_deref()
            .unwrap_or("none"),
        observation
            .session
            .latest_confidence
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_string()),
        observation
            .session
            .latest_blocker
            .as_deref()
            .unwrap_or("none"),
        evidence
            .iter()
            .map(|item| item.evidence_id.to_string())
            .collect::<Vec<_>>()
            .join(","),
        observation.recent_events.len()
    );
    for event in &observation.recent_events {
        output.push_str(&format!("event={} kind={}\n", event.sequence, event.kind));
    }
    output
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

fn dispatch_evidence_confidence(
    latest_execution: Option<&AdapterDispatchExecutionProjection>,
    latest_replay: Option<&AdapterDispatchReplayProjection>,
) -> i64 {
    if latest_execution
        .map(|execution| execution.provider_cli_executed)
        .unwrap_or(false)
    {
        85
    } else if latest_replay.is_some() {
        75
    } else if latest_execution.is_some() {
        65
    } else {
        55
    }
}

fn render_adapter_dispatch_evidence(
    plan: &AdapterDispatchPlanProjection,
    latest_gate: Option<&AdapterDispatchGateProjection>,
    latest_replay: Option<&AdapterDispatchReplayProjection>,
    latest_execution: Option<&AdapterDispatchExecutionProjection>,
    dogfood_gate: &AdapterDogfoodGate,
) -> String {
    let mut markdown = format!(
        "<!-- capo:adapter-dispatch-evidence -->\n# Capo Adapter Dispatch Evidence - {}\n\n## Objective\n\nReview a prompt-redacted dispatch chain before treating provider execution as dogfood evidence.\n\n## Dispatch Plan\n\n- Project: `{}`\n- Dispatch plan: `{}`\n- Adapter: `{}`\n- Provider: `{}`\n- Credential scope: `{}`\n- Agent: `{}` `{}`\n- Session: `{}`\n- Run: `{}`\n- Plan status: `{}`\n- Runtime program: `{}`\n- Runtime arg count: `{}`\n- Runtime cwd: `{}`\n- Artifact root: `{}`\n- Runtime prompt policy: `{}`\n- Provider CLI executed in plan: `{}`\n\n## Dogfood Gate\n\n- Status: `{}`\n- Ready: `{}`\n- Reasons: `{}`\n\n",
        plan.dispatch_plan_id,
        plan.project_id,
        plan.dispatch_plan_id,
        plan.adapter_kind,
        plan.provider_kind,
        plan.credential_scope,
        plan.agent_id,
        plan.agent_name,
        plan.session_id,
        plan.run_id,
        plan.status,
        plan.runtime_program,
        plan.runtime_arg_count,
        plan.runtime_cwd,
        plan.artifact_root,
        plan.runtime_prompt_policy,
        plan.provider_cli_executed,
        dogfood_gate.status,
        dogfood_gate.ready,
        dogfood_gate.reasons.join(",")
    );
    markdown.push_str("## Latest Dispatch Gate\n\n");
    if let Some(gate) = latest_gate {
        markdown.push_str(&format!(
            "- Gate: `{}`\n- Status: `{}`\n- Provider execution allowed: `{}`\n- Required dogfood gate: `{}`\n- Runtime prompt policy: `{}`\n- Provider CLI executed: `{}`\n- Reasons: `{}`\n\n",
            gate.dispatch_gate_id,
            gate.status,
            gate.provider_cli_execution_allowed,
            gate.required_dogfood_gate,
            gate.runtime_prompt_policy,
            gate.provider_cli_executed,
            gate.reason_codes
        ));
    } else {
        markdown.push_str("- none\n\n");
    }
    markdown.push_str("## Latest Fixture Replay\n\n");
    if let Some(replay) = latest_replay {
        markdown.push_str(&format!(
            "- Replay: `{}`\n- Gate: `{}`\n- Fixture path: `{}`\n- Fixture hash: `{}`\n- Input events: `{}`\n- Appended events: `{}`\n- Tool events: `{}`\n- Summary events: `{}`\n- Completed turns: `{}`\n- Provider CLI executed: `{}`\n- Raw content policy: `{}`\n\n",
            replay.dispatch_replay_id,
            replay.dispatch_gate_id,
            replay.fixture_path,
            replay.fixture_hash,
            replay.input_event_count,
            replay.appended_event_count,
            replay.tool_event_count,
            replay.summary_event_count,
            replay.completed_turn_count,
            replay.provider_cli_executed,
            replay.raw_content_policy
        ));
    } else {
        markdown.push_str("- none\n\n");
    }
    markdown.push_str("## Latest Local Execution\n\n");
    if let Some(execution) = latest_execution {
        markdown.push_str(&format!(
            "- Execution: `{}`\n- Request: `{}`\n- Status: `{}`\n- Provider execution allowed: `{}`\n- Provider CLI executed: `{}`\n- Exit code: `{}`\n- Runtime process ref: `{}`\n- Stdout artifact: `{}`\n- Stderr artifact: `{}`\n- Artifact root: `{}`\n- Credential scan: `{}`\n- Raw prompt policy: `{}`\n- Raw output policy: `{}`\n- Reasons: `{}`\n\n",
            execution.dispatch_execution_id,
            execution.execution_request_id,
            execution.status,
            execution.provider_cli_execution_allowed,
            execution.provider_cli_executed,
            execution
                .exit_code
                .map(|code| code.to_string())
                .unwrap_or_else(|| "none".to_string()),
            execution.runtime_process_ref.as_deref().unwrap_or("none"),
            execution.stdout_artifact_id.as_deref().unwrap_or("none"),
            execution.stderr_artifact_id.as_deref().unwrap_or("none"),
            execution.artifact_root,
            execution.credential_scan_status,
            execution.raw_prompt_policy,
            execution.raw_output_policy,
            execution.reason_codes
        ));
    } else {
        markdown.push_str("- none\n\n");
    }
    markdown.push_str("## Redaction Policy\n\n- Raw dispatch prompts are not rendered.\n- Raw provider output is not rendered.\n- Runtime stdout/stderr are referenced by artifact IDs only.\n");
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
        "<!-- capo:review-finding -->\n# Capo Review Finding - {}\n\n## Review\n\n- Review finding: `{}`\n- Reviewer: `{}`\n- Kind: `{}`\n- Severity: `{}`\n- Status: `{}`\n- Artifact: `{}`\n\n## Links\n\n- Project: `{}`\n- Task: `{}`\n- Session: `{}`\n- Run: `{}`\n- Tool call: `{}`\n- Follow-up workpad task: `{}`\n\n## Summary\n\n{}\n",
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
        summary
    )
}

fn render_workpad_proposal(
    task_id: &TaskId,
    workpad_task: &WorkpadTaskProjection,
    workpad_file: &WorkpadFileProjection,
    summary: &str,
    artifact_id: &str,
) -> String {
    format!(
        "<!-- capo:workpad-proposal -->\n# Capo Workpad Proposal - {}\n\n## Objective\n\nReview a Capo-owned proposal artifact before any source markdown is edited.\n\n## Source\n\n- Capo task: `{}`\n- Workpad task: `{}`\n- Source path: `{}`\n- Source anchor: `{}`\n- Source hash: `{}`\n- Observed markdown status: `{}`\n- Capo workpad execution status: `{}`\n- Artifact: `{}`\n\n## Proposed Update\n\n{}\n\n## Apply Policy\n\nCapo has not modified `{}`. Automated source writeback is disabled for this proposal. Any source update must be reviewed by a human and must require an explicit confirmation step in Capo before future automated apply support can write markdown.\n\n## Rollback And Fallback\n\n- Fallback: leave the source markdown unchanged and keep this proposal as evidence.\n- Manual apply: edit `{}` by hand after review, then run the normal git diff and test gates.\n- Rollback after manual edits: use git to inspect or restore only the reviewed source file before committing.\n- Recovery: re-run `capo workpad index --root <project> --state <state>` to refresh Capo's observed workpad refs after any manual change.\n",
        workpad_task.title,
        task_id,
        workpad_task.workpad_task_id,
        workpad_task.path,
        workpad_task.source_anchor,
        workpad_file.content_hash,
        workpad_task.observed_status,
        workpad_task.capo_execution_status,
        artifact_id,
        summary,
        workpad_task.path,
        workpad_task.path
    )
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

fn write_dispatch_evidence_file(path: &Path, markdown: &str) -> Result<(), String> {
    if let Ok(existing) = fs::read_to_string(path) {
        if !existing.starts_with("<!-- capo:adapter-dispatch-evidence -->") {
            return Err(format!(
                "refusing to overwrite non-Capo adapter dispatch evidence file: {}",
                path.display()
            ));
        }
        if existing != markdown {
            return Err(format!(
                "refusing to overwrite changed Capo adapter dispatch evidence file: {}",
                path.display()
            ));
        }
    }
    fs::write(path, markdown).map_err(|error| error.to_string())
}

fn write_dogfood_readiness_file(path: &Path, markdown: &str) -> Result<(), String> {
    if let Ok(existing) = fs::read_to_string(path) {
        if !existing.starts_with("<!-- capo:dogfood-readiness -->") {
            return Err(format!(
                "refusing to overwrite non-Capo dogfood readiness file: {}",
                path.display()
            ));
        }
        if existing != markdown {
            return Err(format!(
                "refusing to overwrite changed Capo dogfood readiness file: {}",
                path.display()
            ));
        }
    }
    fs::write(path, markdown).map_err(|error| error.to_string())
}

fn write_connectivity_exposure_evidence_file(path: &Path, markdown: &str) -> Result<(), String> {
    if let Ok(existing) = fs::read_to_string(path) {
        if !existing.starts_with("<!-- capo:connectivity-exposure-evidence -->") {
            return Err(format!(
                "refusing to overwrite non-Capo connectivity exposure evidence file: {}",
                path.display()
            ));
        }
        if existing != markdown {
            return Err(format!(
                "refusing to overwrite changed Capo connectivity exposure evidence file: {}",
                path.display()
            ));
        }
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

fn write_workpad_proposal_file(path: &Path, markdown: &str) -> Result<(), String> {
    if let Ok(existing) = fs::read_to_string(path) {
        if !existing.starts_with("<!-- capo:workpad-proposal -->") {
            return Err(format!(
                "refusing to overwrite non-Capo workpad proposal file: {}",
                path.display()
            ));
        }
        if existing != markdown {
            return Err(format!(
                "refusing to overwrite changed Capo workpad proposal file: {}",
                path.display()
            ));
        }
    }
    fs::write(path, markdown).map_err(|error| error.to_string())
}

fn controller(parsed: &ParsedArgs) -> Result<FakeBoundaryController, String> {
    FakeBoundaryController::open(project_id(), &parsed.state_root).map_err(debug_error)
}

fn state(parsed: &ParsedArgs) -> Result<SqliteStateStore, String> {
    SqliteStateStore::open(&parsed.state_root).map_err(debug_error)
}

fn envelope(
    command_slug: &str,
    target: CommandTarget,
    intent: CommandIntent,
    text: Option<String>,
) -> CommandEnvelope {
    let mut command = CommandEnvelope::new(
        CommandId::new(format!("cmd-{command_slug}")),
        InputOrigin::Cli,
        "local-user",
        project_id(),
        target,
        intent,
    );
    if let Some(text) = text {
        command = command.with_text(text);
    }
    command
}

fn project_id() -> ProjectId {
    ProjectId::new(DEFAULT_PROJECT_ID)
}

fn required_arg(args: &[String], key: &str) -> Result<String, String> {
    optional_arg(args, key).ok_or_else(|| format!("{key} is required"))
}

fn optional_arg(args: &[String], key: &str) -> Option<String> {
    args.windows(2)
        .find_map(|window| (window[0] == key).then(|| window[1].clone()))
}

fn has_flag(args: &[String], key: &str) -> bool {
    args.iter().any(|arg| arg == key)
}

fn require_fake_arg(args: &[String], key: &str) -> Result<(), String> {
    match optional_arg(args, key).as_deref() {
        None | Some("fake") => Ok(()),
        Some(other) => Err(format!("{key} only supports `fake` in P4, got `{other}`")),
    }
}

fn validate_scope_json(scope_json: &str) -> Result<(), String> {
    match serde_json::from_str::<serde_json::Value>(scope_json) {
        Ok(serde_json::Value::Array(values))
            if values.iter().all(|value| value.as_str().is_some()) =>
        {
            Ok(())
        }
        Ok(_) => Err("--scope-json must be a JSON array of strings".to_string()),
        Err(error) => Err(format!("--scope-json is not valid JSON: {error}")),
    }
}

fn validate_json_object(label: &str, json: &str) -> Result<(), String> {
    match serde_json::from_str::<serde_json::Value>(json) {
        Ok(serde_json::Value::Object(_)) => Ok(()),
        Ok(_) => Err(format!("--{label} must be a JSON object")),
        Err(error) => Err(format!("--{label} is not valid JSON: {error}")),
    }
}

fn approval_decision_effect(decision: &str) -> Result<(&'static str, &'static str), String> {
    match decision {
        "allow_once" => Ok(("allow", "once")),
        "allow_always" => Ok(("allow", "until_revoked")),
        "reject_once" => Ok(("deny", "once")),
        "reject_always" => Ok(("deny", "until_revoked")),
        other => Err(format!(
            "unknown approval decision: {other}; expected allow_once, allow_always, reject_once, or reject_always"
        )),
    }
}

fn validate_decision_scope(
    decision: &str,
    approval: &PermissionApprovalProjection,
) -> Result<(), String> {
    if decision != "allow_always" {
        return Ok(());
    }
    let scopes = scope_values(&approval.scope_json)?;
    let durable_allowed = scopes.iter().all(|scope| {
        matches!(
            scope.as_str(),
            "tool:invoke:capo.task_status"
                | "tool:invoke:capo.agent_status"
                | "tool:invoke:capo.session_summary"
                | "tool:invoke:capo.workpad_read"
        ) || scope.starts_with("state:read:")
    });
    if durable_allowed {
        Ok(())
    } else {
        Err(
            "allow_always is restricted to Capo-owned read/status scopes in the PT2 CLI path"
                .to_string(),
        )
    }
}

fn approval_subject_json(approval: &PermissionApprovalProjection) -> Result<String, String> {
    let mut subject = match serde_json::from_str::<serde_json::Value>(&approval.subject_json) {
        Ok(serde_json::Value::Object(subject)) => subject,
        Ok(_) => return Err("approval subject_json must be a JSON object".to_string()),
        Err(error) => return Err(format!("approval subject_json is not valid JSON: {error}")),
    };
    subject.insert(
        "approval_id".to_string(),
        serde_json::Value::String(approval.approval_id.clone()),
    );
    subject.insert(
        "persistence_scope".to_string(),
        serde_json::Value::String("permission_approval".to_string()),
    );
    if let Some(session_id) = &approval.session_id {
        subject.insert(
            "session_id".to_string(),
            serde_json::Value::String(session_id.to_string()),
        );
    }
    if let Some(tool_call_id) = &approval.tool_call_id {
        subject.insert(
            "tool_call_id".to_string(),
            serde_json::Value::String(tool_call_id.to_string()),
        );
    }
    Ok(serde_json::Value::Object(subject).to_string())
}

fn scope_values(scope_json: &str) -> Result<Vec<String>, String> {
    match serde_json::from_str::<serde_json::Value>(scope_json) {
        Ok(serde_json::Value::Array(values)) => values
            .into_iter()
            .map(|value| {
                value
                    .as_str()
                    .map(ToString::to_string)
                    .ok_or_else(|| "scope_json must contain only strings".to_string())
            })
            .collect(),
        Ok(_) => Err("scope_json must be a JSON array of strings".to_string()),
        Err(error) => Err(format!("scope_json is not valid JSON: {error}")),
    }
}

fn debug_error(error: impl std::fmt::Debug) -> String {
    format!("{error:?}")
}

fn comma_or_none(items: &[String]) -> String {
    if items.is_empty() {
        "none".to_string()
    } else {
        items.join(",")
    }
}

fn markdown_list_or_none(items: &[String]) -> String {
    if items.is_empty() {
        "- none".to_string()
    } else {
        items
            .iter()
            .map(|item| format!("- `{item}`"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[allow(dead_code)]
fn path_exists(path: &Path) -> bool {
    path.exists()
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;
    use capo_state::ConnectivityExposureProjection;

    #[test]
    fn help_mentions_command_envelopes_and_no_credentials() {
        assert!(HELP.contains("command envelopes"));
        assert!(HELP.contains("does not read provider credentials"));
        assert!(HELP.contains("adapter readiness"));
        assert!(HELP.contains("adapter plan-launch"));
        assert!(HELP.contains("adapter dispatch-gate"));
        assert!(HELP.contains("adapter dispatch-status"));
        assert!(HELP.contains("adapter dispatch-evidence"));
        assert!(HELP.contains("adapter execution-request"));
        assert!(HELP.contains("adapter materialize-prompt"));
        assert!(HELP.contains("adapter run-preflight"));
        assert!(HELP.contains("adapter run-local"));
        assert!(HELP.contains("adapter replay-dispatch"));
        assert!(HELP.contains("adapter dogfood-gate"));
        assert!(HELP.contains("dogfood readiness"));
        assert!(HELP.contains("connectivity expose-stub"));
        assert!(HELP.contains("connectivity request-approval"));
        assert!(HELP.contains("connectivity activate-exposure"));
        assert!(HELP.contains("connectivity revoke-exposure"));
        assert!(HELP.contains("connectivity exposure-status"));
        assert!(HELP.contains("connectivity exposure-evidence"));
        assert!(HELP.contains("workpad index"));
        assert!(HELP.contains("workpad next"));
        assert!(HELP.contains("workpad plan-next"));
        assert!(HELP.contains("workpad start-next"));
        assert!(HELP.contains("workpad propose"));
        assert!(HELP.contains("workpad apply"));
    }

    #[test]
    fn dispatch_artifact_scan_deletes_sensitive_outputs_on_failure() {
        let artifact_root = temp_root("dispatch-sensitive-artifacts");
        fs::create_dir_all(&artifact_root).expect("artifact root");
        let stdout = artifact_root.join("stdout.txt");
        let stderr = artifact_root.join("stderr.txt");
        fs::write(&stdout, "Authorization: leaked\n").expect("stdout");
        fs::write(&stderr, "ordinary stderr\n").expect("stderr");

        let error = scan_dispatch_artifacts_or_delete([&stdout, &stderr])
            .expect_err("sensitive marker should fail scan");
        assert!(matches!(
            error,
            LocalAdapterSmokeError::SensitiveArtifact { .. }
        ));
        assert!(!stdout.exists());
        assert!(!stderr.exists());
    }

    #[test]
    fn adapter_plan_launch_builds_dispatch_contract_without_running_provider_cli() {
        let state_root = temp_root("adapter-plan-launch-state");
        let workspace = temp_root("adapter-plan-launch-workspace");
        let artifacts = temp_root("adapter-plan-launch-artifacts");
        let output = run_cli(vec![
            "adapter".to_string(),
            "plan-launch".to_string(),
            "--adapter".to_string(),
            "codex".to_string(),
            "--agent".to_string(),
            "codex-worker".to_string(),
            "--goal".to_string(),
            "Summarize this workpad without printing this prompt.".to_string(),
            "--workspace".to_string(),
            workspace.display().to_string(),
            "--artifacts".to_string(),
            artifacts.display().to_string(),
            "--record".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("adapter plan launch");

        assert!(output.contains("adapter_launch_planned=true"));
        assert!(output.contains("adapter=codex_exec"));
        assert!(output.contains("provider_kind=codex_subscription"));
        assert!(output.contains("credential_scope=user_local_subscription"));
        assert!(output.contains("runtime_program=codex"));
        assert!(output.contains("runtime_prompt_policy=not_rendered"));
        assert!(output.contains("runtime_prompt_source_kind=inline_cli_prompt"));
        assert!(output.contains("runtime_prompt_materialization=manual_prompt_not_replayable"));
        assert!(output.contains("request_env_count=0"));
        assert!(output.contains("subscription_safe=true"));
        assert!(output.contains("provider_cli_executed=false"));
        assert!(output.contains("recorded=true"));
        assert!(output.contains(&format!("runtime_cwd={}", workspace.display())));
        assert!(output.contains(&format!("artifact_root={}", artifacts.display())));
        assert!(!output.contains("Summarize this workpad"));
        assert!(!workspace.exists());
        assert!(!artifacts.exists());
        let plans = SqliteStateStore::open(&state_root)
            .expect("state")
            .adapter_dispatch_plans(&project_id())
            .expect("dispatch plans");
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].adapter_kind, "codex_exec");
        assert_eq!(plans[0].runtime_prompt_policy, "not_rendered");
        assert!(!plans[0].provider_cli_executed);
        let prompt_sources = SqliteStateStore::open(&state_root)
            .expect("state")
            .adapter_dispatch_prompt_sources(&project_id())
            .expect("dispatch prompt sources");
        assert_eq!(prompt_sources.len(), 1);
        assert_eq!(prompt_sources[0].source_kind, "inline_cli_prompt");
        assert_eq!(
            prompt_sources[0].materialization_status,
            "manual_prompt_not_replayable"
        );
        assert_eq!(prompt_sources[0].raw_prompt_policy, "not_rendered");
        let materialize = run_cli(vec![
            "adapter".to_string(),
            "materialize-prompt".to_string(),
            "--dispatch-plan".to_string(),
            plans[0].dispatch_plan_id.clone(),
            "--record".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("materialize inline prompt");
        assert!(materialize.contains("adapter_dispatch_prompt_materialization=true"));
        assert!(materialize.contains("status=blocked_non_replayable_prompt"));
        assert!(materialize.contains("raw_prompt_policy=not_rendered"));
        assert!(!materialize.contains("Summarize this workpad"));
        let dashboard = run_cli(vec![
            "dashboard".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("dashboard");
        assert!(dashboard.contains("adapter_dispatch_plans=1"));
        assert!(dashboard.contains("adapter_dispatch_prompt_sources=1"));
        assert!(dashboard.contains("adapter_dispatch_prompt_materializations=1"));
        assert!(dashboard.contains("status=blocked_non_replayable_prompt"));
        assert!(dashboard.contains("source_kind=inline_cli_prompt"));
        assert!(dashboard.contains("adapter_dispatch_plan=adapter-dispatch-plan-codex_exec"));
        assert!(!dashboard.contains("Summarize this workpad"));
    }

    #[test]
    fn adapter_plan_launch_records_distinct_prompt_identities_without_prompt_text() {
        let state_root = temp_root("adapter-plan-launch-distinct-state");
        let workspace = temp_root("adapter-plan-launch-distinct-workspace");
        let artifacts = temp_root("adapter-plan-launch-distinct-artifacts");
        for goal in ["First sensitive-ish prompt", "Second sensitive-ish prompt"] {
            run_cli(vec![
                "adapter".to_string(),
                "plan-launch".to_string(),
                "--adapter".to_string(),
                "codex".to_string(),
                "--agent".to_string(),
                "codex-worker".to_string(),
                "--goal".to_string(),
                goal.to_string(),
                "--workspace".to_string(),
                workspace.display().to_string(),
                "--artifacts".to_string(),
                artifacts.display().to_string(),
                "--record".to_string(),
                "--state".to_string(),
                state_root.display().to_string(),
            ])
            .expect("record dispatch plan");
        }

        let plans = SqliteStateStore::open(&state_root)
            .expect("state")
            .adapter_dispatch_plans(&project_id())
            .expect("dispatch plans");
        assert_eq!(plans.len(), 2);
        assert_ne!(plans[0].dispatch_plan_id, plans[1].dispatch_plan_id);
        let prompt_sources = SqliteStateStore::open(&state_root)
            .expect("state")
            .adapter_dispatch_prompt_sources(&project_id())
            .expect("dispatch prompt sources");
        assert_eq!(prompt_sources.len(), 2);
        assert_ne!(prompt_sources[0].prompt_hash, prompt_sources[1].prompt_hash);
        let dashboard = run_cli(vec![
            "dashboard".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("dashboard");
        assert!(dashboard.contains("adapter_dispatch_plans=2"));
        assert!(!dashboard.contains("First sensitive-ish prompt"));
        assert!(!dashboard.contains("Second sensitive-ish prompt"));
    }

    #[test]
    fn adapter_plan_launch_rejects_unknown_adapter() {
        let state_root = temp_root("adapter-plan-launch-unknown-state");
        let error = run_cli(vec![
            "adapter".to_string(),
            "plan-launch".to_string(),
            "--adapter".to_string(),
            "unknown".to_string(),
            "--agent".to_string(),
            "worker".to_string(),
            "--goal".to_string(),
            "Do work.".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .unwrap_err();

        assert!(error.contains("unsupported local adapter dispatch plan"));
        let state = SqliteStateStore::open(&state_root).expect("state");
        assert!(
            state
                .agent_by_name("worker")
                .expect("agent lookup after failed plan")
                .is_none()
        );
    }

    #[test]
    fn adapter_dispatch_gate_blocks_until_real_smoke_evidence_is_recorded() {
        let state_root = temp_root("adapter-dispatch-gate-state");
        let workspace = temp_root("adapter-dispatch-gate-workspace");
        let artifacts = temp_root("adapter-dispatch-gate-artifacts");
        run_cli(vec![
            "adapter".to_string(),
            "plan-launch".to_string(),
            "--adapter".to_string(),
            "codex".to_string(),
            "--agent".to_string(),
            "codex-worker".to_string(),
            "--goal".to_string(),
            "Do not render this dispatch prompt.".to_string(),
            "--workspace".to_string(),
            workspace.display().to_string(),
            "--artifacts".to_string(),
            artifacts.display().to_string(),
            "--record".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("record dispatch plan");
        let plans = SqliteStateStore::open(&state_root)
            .expect("state")
            .adapter_dispatch_plans(&project_id())
            .expect("dispatch plans");
        let dispatch_plan_id = plans[0].dispatch_plan_id.clone();

        let blocked = run_cli(vec![
            "adapter".to_string(),
            "dispatch-gate".to_string(),
            "--dispatch-plan".to_string(),
            dispatch_plan_id.clone(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("blocked dispatch gate");
        assert!(blocked.contains("adapter_dispatch_gate=true"));
        assert!(blocked.contains("provider_cli_execution_allowed=false"));
        assert!(blocked.contains("status=blocked"));
        assert!(blocked.contains("required_dogfood_gate=blocked_pending_real_smoke"));
        assert!(blocked.contains("provider_cli_executed=false"));
        assert!(blocked.contains("runtime_prompt_policy=not_rendered"));
        assert!(blocked.contains("codex_exec:real_subscription_smoke_not_recorded"));
        assert!(blocked.contains("recorded=false"));
        assert!(!blocked.contains("Do not render this dispatch prompt"));
        assert!(!workspace.exists());
        assert!(!artifacts.exists());
        let blocked_status = run_cli(vec![
            "adapter".to_string(),
            "dispatch-status".to_string(),
            "--dispatch-plan".to_string(),
            dispatch_plan_id.clone(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("blocked dispatch status");
        assert!(blocked_status.contains("adapter_dispatch_status=true"));
        assert!(blocked_status.contains("latest_dispatch_gate=none"));
        assert!(blocked_status.contains("latest_gate_status=missing"));
        assert!(blocked_status.contains("latest_dispatch_replay=none"));
        assert!(blocked_status.contains("latest_dispatch_execution=none"));
        assert!(blocked_status.contains("latest_execution_status=missing"));
        assert!(blocked_status.contains("next_action=record_clean_real_smoke_evidence"));
        assert!(!blocked_status.contains("Do not render this dispatch prompt"));
        let blocked_latest_status = run_cli(vec![
            "adapter".to_string(),
            "dispatch-status".to_string(),
            "--latest".to_string(),
            "--agent".to_string(),
            "codex-worker".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("latest blocked dispatch status");
        assert!(blocked_latest_status.contains(&format!("dispatch_plan={dispatch_plan_id}")));
        assert!(blocked_latest_status.contains("agent=codex-worker"));
        assert!(blocked_latest_status.contains("next_action=record_clean_real_smoke_evidence"));
        assert!(!blocked_latest_status.contains("Do not render this dispatch prompt"));
        let blocked_execution_request = run_cli(vec![
            "adapter".to_string(),
            "execution-request".to_string(),
            "--dispatch-plan".to_string(),
            dispatch_plan_id.clone(),
            "--record".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("blocked execution request");
        assert!(blocked_execution_request.contains("adapter_dispatch_execution_request=true"));
        assert!(blocked_execution_request.contains("provider_cli_execution_allowed=false"));
        assert!(blocked_execution_request.contains("provider_cli_executed=false"));
        assert!(blocked_execution_request.contains("status=blocked_missing_ready_gate"));
        assert!(blocked_execution_request.contains("recorded=true"));
        assert!(!blocked_execution_request.contains("Do not render this dispatch prompt"));
        let fixture = PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../capo-adapters/fixtures/codex-exec.jsonl"
        ));
        let blocked_replay = run_cli(vec![
            "adapter".to_string(),
            "replay-dispatch".to_string(),
            "--dispatch-plan".to_string(),
            dispatch_plan_id.clone(),
            "--fixture".to_string(),
            fixture.display().to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect_err("replay should require a recorded ready gate");
        assert!(blocked_replay.contains("has no recorded ready dispatch gate"));

        let artifact_root = temp_root("adapter-dispatch-gate-smoke-artifacts");
        fs::create_dir_all(&artifact_root).expect("artifact dir");
        fs::write(artifact_root.join("stdout.txt"), "CAPO_CODEX_SMOKE_OK\n").expect("artifact");
        run_cli(vec![
            "adapter".to_string(),
            "smoke-report".to_string(),
            "record".to_string(),
            "--adapter".to_string(),
            "codex".to_string(),
            "--status".to_string(),
            "passed".to_string(),
            "--credential-scan".to_string(),
            "clean".to_string(),
            "--marker-found".to_string(),
            "--artifact-root".to_string(),
            artifact_root.display().to_string(),
            "--reason".to_string(),
            "operator recorded clean opt-in smoke".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("record passed smoke");

        let ready = run_cli(vec![
            "adapter".to_string(),
            "dispatch-gate".to_string(),
            "--dispatch-plan".to_string(),
            dispatch_plan_id,
            "--record".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("ready dispatch gate");
        assert!(ready.contains("provider_cli_execution_allowed=true"));
        assert!(ready.contains("status=ready_for_execution"));
        assert!(ready.contains("required_dogfood_gate=ready_for_first_real_agent_dogfood"));
        assert!(ready.contains("reasons=required_real_smoke_evidence_recorded"));
        assert!(ready.contains("recorded=true"));
        let gates = SqliteStateStore::open(&state_root)
            .expect("state")
            .adapter_dispatch_gates(&project_id())
            .expect("dispatch gates");
        assert_eq!(gates.len(), 1);
        assert_eq!(gates[0].adapter_kind, "codex_exec");
        assert_eq!(gates[0].status, "ready_for_execution");
        assert!(gates[0].provider_cli_execution_allowed);
        assert!(!gates[0].provider_cli_executed);
        assert_eq!(gates[0].runtime_prompt_policy, "not_rendered");
        let dashboard = run_cli(vec![
            "dashboard".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("dashboard");
        assert!(dashboard.contains("adapter_dispatch_gates=1"));
        assert!(dashboard.contains("gate_status=ready_for_execution"));
        assert!(!dashboard.contains("Do not render this dispatch prompt"));
        let ready_status = run_cli(vec![
            "adapter".to_string(),
            "dispatch-status".to_string(),
            "--dispatch-plan".to_string(),
            gates[0].dispatch_plan_id.clone(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("ready dispatch status");
        assert!(ready_status.contains("latest_gate_status=ready_for_execution"));
        assert!(ready_status.contains("latest_gate_provider_cli_execution_allowed=true"));
        assert!(ready_status.contains("latest_dispatch_replay=none"));
        assert!(ready_status.contains(
            "next_action=replay_dispatch_fixture_or_run_provider_execution_after_explicit_opt_in"
        ));
        assert!(!ready_status.contains("Do not render this dispatch prompt"));
        let ready_execution_request = run_cli(vec![
            "adapter".to_string(),
            "execution-request".to_string(),
            "--dispatch-plan".to_string(),
            gates[0].dispatch_plan_id.clone(),
            "--record".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("ready execution request");
        assert!(ready_execution_request.contains("provider_cli_execution_allowed=true"));
        assert!(ready_execution_request.contains("provider_cli_executed=false"));
        assert!(ready_execution_request.contains("status=waiting_on_explicit_provider_opt_in"));
        assert!(ready_execution_request.contains("opt_in_env=CAPO_RUN_CODEX_LOCAL_DISPATCH"));
        assert!(
            ready_execution_request.contains("reasons=explicit_provider_execution_opt_in_required")
        );
        assert!(!ready_execution_request.contains("Do not render this dispatch prompt"));
        let preflight_without_materialization = run_cli(vec![
            "adapter".to_string(),
            "run-preflight".to_string(),
            "--dispatch-plan".to_string(),
            gates[0].dispatch_plan_id.clone(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("dispatch preflight without materialization");
        assert!(preflight_without_materialization.contains("adapter_dispatch_run_preflight=true"));
        assert!(
            preflight_without_materialization
                .contains("status=blocked_missing_prompt_materialization")
        );
        assert!(
            preflight_without_materialization
                .contains("reasons=recorded_prompt_materialization_missing")
        );
        assert!(preflight_without_materialization.contains("provider_cli_execution_allowed=false"));
        assert!(preflight_without_materialization.contains("provider_cli_executed=false"));
        assert!(!preflight_without_materialization.contains("Do not render this dispatch prompt"));
        let run_local_without_materialization = run_cli(vec![
            "adapter".to_string(),
            "run-local".to_string(),
            "--dispatch-plan".to_string(),
            gates[0].dispatch_plan_id.clone(),
            "--record".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("local dispatch runner blocks without materialization");
        assert!(run_local_without_materialization.contains("adapter_dispatch_run_local=true"));
        assert!(
            run_local_without_materialization
                .contains("status=blocked_missing_prompt_materialization")
        );
        assert!(run_local_without_materialization.contains("provider_cli_executed=false"));
        assert!(run_local_without_materialization.contains("recorded=true"));
        assert!(!run_local_without_materialization.contains("Do not render this dispatch prompt"));
        let executions = SqliteStateStore::open(&state_root)
            .expect("state")
            .adapter_dispatch_executions(&project_id())
            .expect("dispatch executions");
        assert_eq!(executions.len(), 1);
        assert_eq!(
            executions[0].status,
            "blocked_missing_prompt_materialization"
        );
        assert!(!executions[0].provider_cli_executed);
        assert_eq!(executions[0].credential_scan_status, "not_run");
        let execution_dashboard = run_cli(vec![
            "dashboard".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("dashboard after blocked local run");
        assert!(execution_dashboard.contains("adapter_dispatch_executions=1"));
        assert!(
            execution_dashboard.contains("execution_status=blocked_missing_prompt_materialization")
        );
        let blocked_execution_status = run_cli(vec![
            "adapter".to_string(),
            "dispatch-status".to_string(),
            "--dispatch-plan".to_string(),
            gates[0].dispatch_plan_id.clone(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("dispatch status after blocked local run");
        assert!(blocked_execution_status.contains("latest_dispatch_execution="));
        assert!(
            blocked_execution_status
                .contains("latest_execution_status=blocked_missing_prompt_materialization")
        );
        assert!(blocked_execution_status.contains("latest_execution_provider_cli_executed=false"));
        assert!(
            blocked_execution_status.contains("latest_execution_credential_scan_status=not_run")
        );
        assert!(blocked_execution_status.contains("next_action=resolve_latest_execution_blocker"));
        assert!(!blocked_execution_status.contains("Do not render this dispatch prompt"));
        let latest_after_blocked_execution = run_cli(vec![
            "adapter".to_string(),
            "dispatch-status".to_string(),
            "--latest".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("latest status after blocked local run");
        assert!(
            latest_after_blocked_execution
                .contains(&format!("dispatch_plan={}", gates[0].dispatch_plan_id))
        );
        assert!(
            latest_after_blocked_execution
                .contains("latest_execution_status=blocked_missing_prompt_materialization")
        );
        assert!(
            latest_after_blocked_execution.contains("next_action=resolve_latest_execution_blocker")
        );
        assert!(!latest_after_blocked_execution.contains("Do not render this dispatch prompt"));
        let inline_materialization = run_cli(vec![
            "adapter".to_string(),
            "materialize-prompt".to_string(),
            "--dispatch-plan".to_string(),
            gates[0].dispatch_plan_id.clone(),
            "--record".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("materialize inline prompt");
        assert!(inline_materialization.contains("status=blocked_non_replayable_prompt"));
        assert!(!inline_materialization.contains("Do not render this dispatch prompt"));
        let preflight_with_blocked_materialization = run_cli(vec![
            "adapter".to_string(),
            "run-preflight".to_string(),
            "--dispatch-plan".to_string(),
            gates[0].dispatch_plan_id.clone(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("dispatch preflight with blocked materialization");
        assert!(
            preflight_with_blocked_materialization
                .contains("status=blocked_prompt_materialization_not_ready")
        );
        assert!(preflight_with_blocked_materialization.contains("blocked_non_replayable_prompt"));
        assert!(preflight_with_blocked_materialization.contains("raw_prompt_policy=not_rendered"));
        assert!(
            !preflight_with_blocked_materialization.contains("Do not render this dispatch prompt")
        );
        let execution_requests = SqliteStateStore::open(&state_root)
            .expect("state")
            .adapter_dispatch_execution_requests(&project_id())
            .expect("dispatch execution requests");
        assert_eq!(execution_requests.len(), 2);
        assert!(
            execution_requests
                .iter()
                .any(|request| request.status == "blocked_missing_ready_gate")
        );
        assert!(
            execution_requests
                .iter()
                .any(|request| request.status == "waiting_on_explicit_provider_opt_in")
        );
        let evidence_dir = temp_root("adapter-dispatch-gate-replay-evidence");
        let replay = run_cli(vec![
            "adapter".to_string(),
            "replay-dispatch".to_string(),
            "--dispatch-plan".to_string(),
            gates[0].dispatch_plan_id.clone(),
            "--fixture".to_string(),
            fixture.display().to_string(),
            "--out".to_string(),
            evidence_dir.display().to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("replay dispatch fixture");
        assert!(replay.contains("adapter_dispatch_replayed=true"));
        assert!(replay.contains("adapter=codex_exec"));
        assert!(replay.contains("raw_content_policy=content_hashed_not_rendered"));
        assert!(replay.contains("provider_cli_executed=false"));
        assert!(replay.contains("tool_events=2"));
        assert!(replay.contains("summary_events=1"));
        assert!(replay.contains("completed_turns=1"));
        assert!(replay.contains("evidence_exported=true"));
        assert!(!replay.contains("Do not render this dispatch prompt"));
        assert!(!replay.contains("Codex fixture response."));
        assert!(!replay.contains("cargo test"));
        let replays = SqliteStateStore::open(&state_root)
            .expect("state")
            .adapter_dispatch_replays(&project_id())
            .expect("dispatch replays");
        assert_eq!(replays.len(), 1);
        assert_eq!(replays[0].dispatch_plan_id, gates[0].dispatch_plan_id);
        assert_eq!(replays[0].dispatch_gate_id, gates[0].dispatch_gate_id);
        assert_eq!(replays[0].adapter_kind, "codex_exec");
        assert_eq!(replays[0].tool_event_count, 2);
        assert!(!replays[0].provider_cli_executed);
        assert_eq!(replays[0].raw_content_policy, "content_hashed_not_rendered");
        let replay_dashboard = run_cli(vec![
            "dashboard".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("dashboard after replay");
        assert!(replay_dashboard.contains("adapter_dispatch_replays=1"));
        assert!(replay_dashboard.contains("adapter_dispatch_execution_requests=2"));
        assert!(replay_dashboard.contains("execution_status=waiting_on_explicit_provider_opt_in"));
        assert!(replay_dashboard.contains("raw_content_policy=content_hashed_not_rendered"));
        assert!(!replay_dashboard.contains("Codex fixture response."));
        assert!(!replay_dashboard.contains("cargo test"));
        let replay_status = run_cli(vec![
            "adapter".to_string(),
            "dispatch-status".to_string(),
            "--dispatch-plan".to_string(),
            replays[0].dispatch_plan_id.clone(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("replay dispatch status");
        assert!(replay_status.contains("latest_gate_status=ready_for_execution"));
        assert!(
            replay_status.contains("latest_replay_raw_content_policy=content_hashed_not_rendered")
        );
        assert!(
            replay_status
                .contains("latest_execution_status=blocked_missing_prompt_materialization")
        );
        assert!(replay_status.contains("latest_replay_appended_events="));
        assert!(replay_status.contains("next_action=inspect_replay_or_prepare_real_execution"));
        assert!(!replay_status.contains("Do not render this dispatch prompt"));
        assert!(!replay_status.contains("Codex fixture response."));
        assert!(!replay_status.contains("cargo test"));
        let dispatch_evidence_dir = temp_root("adapter-dispatch-chain-evidence");
        let dispatch_evidence = run_cli(vec![
            "adapter".to_string(),
            "dispatch-evidence".to_string(),
            "--dispatch-plan".to_string(),
            replays[0].dispatch_plan_id.clone(),
            "--out".to_string(),
            dispatch_evidence_dir.display().to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("export dispatch evidence");
        assert!(dispatch_evidence.contains("adapter_dispatch_evidence_exported=true"));
        assert!(dispatch_evidence.contains("evidence_id="));
        assert!(dispatch_evidence.contains("artifact_id=artifact-adapter-dispatch-evidence-"));
        assert!(
            dispatch_evidence.contains(&format!("dispatch_plan={}", replays[0].dispatch_plan_id))
        );
        let dispatch_evidence_path = dispatch_evidence
            .lines()
            .find_map(|line| line.strip_prefix("path="))
            .map(PathBuf::from)
            .expect("dispatch evidence path");
        let dispatch_evidence_markdown =
            fs::read_to_string(&dispatch_evidence_path).expect("read dispatch evidence");
        assert!(dispatch_evidence_markdown.starts_with("<!-- capo:adapter-dispatch-evidence -->"));
        assert!(dispatch_evidence_markdown.contains("## Dispatch Plan"));
        assert!(dispatch_evidence_markdown.contains("## Latest Dispatch Gate"));
        assert!(dispatch_evidence_markdown.contains("## Latest Fixture Replay"));
        assert!(dispatch_evidence_markdown.contains("## Latest Local Execution"));
        assert!(dispatch_evidence_markdown.contains("Raw dispatch prompts are not rendered"));
        assert!(
            dispatch_evidence_markdown.contains("Status: `blocked_missing_prompt_materialization`")
        );
        assert!(!dispatch_evidence_markdown.contains("Do not render this dispatch prompt"));
        assert!(!dispatch_evidence_markdown.contains("Codex fixture response."));
        assert!(!dispatch_evidence_markdown.contains("cargo test"));
        let latest_dispatch_evidence_dir = temp_root("adapter-dispatch-chain-latest-evidence");
        let latest_dispatch_evidence = run_cli(vec![
            "adapter".to_string(),
            "dispatch-evidence".to_string(),
            "--latest".to_string(),
            "--agent".to_string(),
            "codex-worker".to_string(),
            "--out".to_string(),
            latest_dispatch_evidence_dir.display().to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("export latest dispatch evidence");
        assert!(latest_dispatch_evidence.contains("adapter_dispatch_evidence_exported=true"));
        assert!(
            latest_dispatch_evidence
                .contains(&format!("dispatch_plan={}", replays[0].dispatch_plan_id))
        );
        let latest_dispatch_evidence_path = latest_dispatch_evidence
            .lines()
            .find_map(|line| line.strip_prefix("path="))
            .map(PathBuf::from)
            .expect("latest dispatch evidence path");
        let latest_dispatch_evidence_markdown =
            fs::read_to_string(&latest_dispatch_evidence_path).expect("read latest evidence");
        assert!(
            latest_dispatch_evidence_markdown
                .starts_with("<!-- capo:adapter-dispatch-evidence -->")
        );
        assert!(latest_dispatch_evidence_markdown.contains("## Latest Fixture Replay"));
        assert!(
            latest_dispatch_evidence_markdown.contains("Raw dispatch prompts are not rendered")
        );
        assert!(!latest_dispatch_evidence.contains("Do not render this dispatch prompt"));
        assert!(!latest_dispatch_evidence_markdown.contains("Do not render this dispatch prompt"));
        assert!(!latest_dispatch_evidence_markdown.contains("Codex fixture response."));
        assert!(!latest_dispatch_evidence_markdown.contains("cargo test"));
        let dispatch_evidence_rows = SqliteStateStore::open(&state_root)
            .expect("state")
            .evidence_for_session(&replays[0].session_id)
            .expect("dispatch evidence rows");
        assert!(
            dispatch_evidence_rows
                .iter()
                .any(|evidence| evidence.kind == "adapter_dispatch_evidence")
        );
        let readiness = run_cli(vec![
            "dogfood".to_string(),
            "readiness".to_string(),
            "--out".to_string(),
            dispatch_evidence_dir.display().to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("dogfood readiness");
        assert!(readiness.contains("dogfood_readiness=true"));
        assert!(readiness.contains("ready=false"));
        assert!(readiness.contains("real_agent_connector_ready=true"));
        assert!(readiness.contains("dispatch_chain_ready=true"));
        assert!(readiness.contains("workpad_bridge_ready=false"));
        assert!(readiness.contains("dispatch_plans=1"));
        assert!(readiness.contains("dispatch_replays=1"));
        assert!(readiness.contains("dispatch_executions=1"));
        assert!(readiness.contains("blockers=workpad_index_missing"));
        assert!(readiness.contains("next_actions=run_workpad_index"));
        assert!(readiness.contains("dogfood_readiness_evidence_exported=true"));
        assert!(readiness.contains("artifact_id=artifact-dogfood-readiness-"));
        let readiness_path = readiness
            .lines()
            .find_map(|line| line.strip_prefix("path="))
            .map(PathBuf::from)
            .expect("dogfood readiness evidence path");
        let readiness_markdown =
            fs::read_to_string(&readiness_path).expect("read dogfood readiness evidence");
        assert!(readiness_markdown.starts_with("<!-- capo:dogfood-readiness -->"));
        assert!(readiness_markdown.contains("## Summary"));
        assert!(readiness_markdown.contains("## Counts"));
        assert!(readiness_markdown.contains("`workpad_index_missing`"));
        assert!(readiness_markdown.contains("does not run provider CLIs"));
        assert!(!readiness_markdown.contains("Do not render this dispatch prompt"));
        assert!(!readiness_markdown.contains("Codex fixture response."));
        assert!(!readiness_markdown.contains("cargo test"));
        let dashboard_after_readiness = run_cli(vec![
            "dashboard".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("dashboard after dogfood readiness evidence");
        assert!(dashboard_after_readiness.contains("project_evidence=1"));
        assert!(dashboard_after_readiness.contains("kind=dogfood_readiness"));
        assert!(dashboard_after_readiness.contains("artifact=artifact-dogfood-readiness-"));
        assert!(dashboard_after_readiness.contains("project_dogfood_readiness=false"));
        assert!(dashboard_after_readiness.contains("status=blocked_pending_dogfood_prerequisites"));
        assert!(dashboard_after_readiness.contains("real_agent_connector_ready=true"));
        assert!(dashboard_after_readiness.contains("workpad_bridge_ready=false"));
        assert!(dashboard_after_readiness.contains("dispatch_chain_ready=true"));
        assert!(dashboard_after_readiness.contains("blockers=workpad_index_missing"));
        assert_text_absent_in_tree(&state_root, "Do not render this dispatch prompt");
        assert_text_absent_in_tree(&state_root, "Codex fixture response.");
        assert_text_absent_in_tree(&state_root, "cargo test");
        assert_text_absent_in_tree(&evidence_dir, "Codex fixture response.");
        assert_text_absent_in_tree(&evidence_dir, "cargo test");
        assert_text_absent_in_tree(&dispatch_evidence_dir, "Do not render this dispatch prompt");
        assert_text_absent_in_tree(&dispatch_evidence_dir, "Codex fixture response.");
        assert_text_absent_in_tree(&dispatch_evidence_dir, "cargo test");
        assert!(!workspace.exists());
        assert!(!artifacts.exists());
    }

    #[test]
    fn adapter_readiness_reports_opt_in_gates_without_running_provider_clis() {
        let state_root = temp_root("adapter-readiness-state");
        let output = run_cli(vec![
            "adapter".to_string(),
            "readiness".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("adapter readiness");

        assert!(output.contains("adapter_readiness=true"));
        assert!(output.contains("credential_policy=not_inspected"));
        assert!(output.contains("adapter=codex_exec"));
        assert!(output.contains("opt_in_env=CAPO_RUN_CODEX_LOCAL_SMOKE"));
        assert!(output.contains("adapter=claude_code"));
        assert!(output.contains("opt_in_env=CAPO_RUN_CLAUDE_LOCAL_SMOKE"));
        assert!(output.contains("ready_for_real_agent_dogfood=false"));
        assert!(output.contains("blocked_reason=real_subscription_smoke_not_recorded"));
        assert!(output.contains("recorded=false"));
        assert!(!state_root.join("adapter-readiness").exists());

        let recorded = run_cli(vec![
            "adapter".to_string(),
            "readiness".to_string(),
            "--record".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("record adapter readiness");
        assert!(recorded.contains("recorded=true"));
        assert!(recorded.contains("recorded_sequence="));

        let state = SqliteStateStore::open(&state_root).expect("state");
        let readiness = state
            .adapter_readiness(&project_id())
            .expect("adapter readiness rows");
        assert_eq!(readiness.len(), 2);
        assert!(readiness.iter().any(|row| row.adapter_kind == "codex_exec"
            && row.smoke_status == "waiting_on_opt_in"
            && row.credential_policy == "not_inspected"));

        let dashboard = run_cli(vec![
            "dashboard".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("dashboard");
        assert!(dashboard.contains("adapter_readiness=2"));
        assert!(dashboard.contains("adapter_readiness_row=codex_exec"));
        assert!(dashboard.contains("dogfood_blocker=real_subscription_smoke_not_recorded"));
    }

    #[test]
    fn adapter_smoke_report_records_skipped_and_blocks_invalid_pass() {
        let state_root = temp_root("adapter-smoke-report-state");
        let artifact_root = temp_root("adapter-smoke-report-artifacts");
        fs::create_dir_all(&artifact_root).expect("artifact dir");
        fs::write(
            artifact_root.join("stdout.txt"),
            "CAPO_CODEX_SMOKE_OK\nAuthorization: [REDACTED]\n",
        )
        .expect("clean artifact");
        let skipped = run_cli(vec![
            "adapter".to_string(),
            "smoke-report".to_string(),
            "record".to_string(),
            "--adapter".to_string(),
            "codex".to_string(),
            "--status".to_string(),
            "skipped".to_string(),
            "--credential-scan".to_string(),
            "not_run".to_string(),
            "--reason".to_string(),
            "waiting for explicit opt-in".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("record skipped smoke");
        assert!(skipped.contains("adapter_smoke_report_recorded=true"));
        assert!(skipped.contains("dogfood_readiness_effect=real_subscription_smoke_not_recorded"));

        let invalid_pass = run_cli(vec![
            "adapter".to_string(),
            "smoke-report".to_string(),
            "record".to_string(),
            "--adapter".to_string(),
            "codex".to_string(),
            "--status".to_string(),
            "passed".to_string(),
            "--credential-scan".to_string(),
            "not_run".to_string(),
            "--reason".to_string(),
            "bad pass".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect_err("passed report requires clean scan and marker");
        assert!(invalid_pass.contains("passed smoke reports require"));

        let passed = run_cli(vec![
            "adapter".to_string(),
            "smoke-report".to_string(),
            "record".to_string(),
            "--adapter".to_string(),
            "codex".to_string(),
            "--status".to_string(),
            "passed".to_string(),
            "--credential-scan".to_string(),
            "clean".to_string(),
            "--marker-found".to_string(),
            "--artifact-root".to_string(),
            artifact_root.display().to_string(),
            "--reason".to_string(),
            "clean opt-in smoke artifacts".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("record passed smoke");
        assert!(passed.contains("dogfood_readiness_effect=real_agent_connector_proven"));

        let dashboard = run_cli(vec![
            "dashboard".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("dashboard");
        assert!(dashboard.contains("adapter_smoke_reports=2"));
        assert!(dashboard.contains("adapter_smoke_report=adapter-smoke-codex_exec"));
        assert!(dashboard.contains("credential_scan_status=not_run"));
    }

    #[test]
    fn adapter_smoke_artifact_scan_blocks_raw_secret_markers() {
        let clean_root = temp_root("adapter-clean-artifacts");
        fs::create_dir_all(clean_root.join("nested")).expect("clean artifact dir");
        fs::write(
            clean_root.join("nested").join("stdout.txt"),
            "Cookie: [REDACTED]\n",
        )
        .expect("clean artifact");
        let clean = run_cli(vec![
            "adapter".to_string(),
            "smoke-report".to_string(),
            "scan".to_string(),
            "--artifact-root".to_string(),
            clean_root.display().to_string(),
        ])
        .expect("clean scan");
        assert!(clean.contains("adapter_smoke_artifact_scan=true"));
        assert!(clean.contains("credential_scan_status=clean"));
        assert!(clean.contains("files_scanned=1"));

        let blocked_root = temp_root("adapter-blocked-artifacts");
        fs::create_dir_all(&blocked_root).expect("blocked artifact dir");
        fs::write(
            blocked_root.join("stderr.txt"),
            "Authorization: Bearer secret\n",
        )
        .expect("blocked artifact");
        let blocked = run_cli(vec![
            "adapter".to_string(),
            "smoke-report".to_string(),
            "scan".to_string(),
            "--artifact-root".to_string(),
            blocked_root.display().to_string(),
        ])
        .expect_err("raw secret marker should block scan");
        assert!(blocked.contains("credential scan blocked artifact"));
        assert!(blocked.contains("authorization:"));

        let state_root = temp_root("adapter-blocked-report-state");
        let blocked_report = run_cli(vec![
            "adapter".to_string(),
            "smoke-report".to_string(),
            "record".to_string(),
            "--adapter".to_string(),
            "codex".to_string(),
            "--status".to_string(),
            "passed".to_string(),
            "--credential-scan".to_string(),
            "clean".to_string(),
            "--marker-found".to_string(),
            "--artifact-root".to_string(),
            blocked_root.display().to_string(),
            "--reason".to_string(),
            "should fail scan".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect_err("passed report should enforce scan");
        assert!(blocked_report.contains("credential scan blocked artifact"));
    }

    #[test]
    fn adapter_smoke_artifact_scan_refuses_symlinks() {
        let artifact_root = temp_root("adapter-symlink-artifacts");
        fs::create_dir_all(&artifact_root).expect("artifact dir");
        fs::write(artifact_root.join("stdout.txt"), "CAPO_CODEX_SMOKE_OK\n").expect("artifact");
        let outside = temp_root("adapter-symlink-outside");
        fs::create_dir_all(&outside).expect("outside dir");
        fs::write(
            outside.join("session.txt"),
            "Authorization: Bearer secret\n",
        )
        .expect("outside secret");
        #[cfg(unix)]
        std::os::unix::fs::symlink(
            outside.join("session.txt"),
            artifact_root.join("session-link"),
        )
        .expect("symlink");
        #[cfg(windows)]
        std::os::windows::fs::symlink_file(
            outside.join("session.txt"),
            artifact_root.join("session-link"),
        )
        .expect("symlink");

        let blocked = run_cli(vec![
            "adapter".to_string(),
            "smoke-report".to_string(),
            "scan".to_string(),
            "--artifact-root".to_string(),
            artifact_root.display().to_string(),
        ])
        .expect_err("symlink should be refused");
        assert!(blocked.contains("artifact scan refuses symlink path"));
    }

    #[test]
    fn adapter_dogfood_gate_requires_passed_codex_smoke_report() {
        let state_root = temp_root("adapter-dogfood-gate-state");
        let artifact_root = temp_root("adapter-dogfood-gate-artifacts");
        fs::create_dir_all(&artifact_root).expect("artifact dir");
        fs::write(artifact_root.join("stdout.txt"), "CAPO_CODEX_SMOKE_OK\n").expect("artifact");
        let blocked = run_cli(vec![
            "adapter".to_string(),
            "dogfood-gate".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("blocked gate");
        assert!(blocked.contains("adapter_dogfood_gate=true"));
        assert!(blocked.contains("ready_for_first_real_agent_dogfood=false"));
        assert!(blocked.contains("blocked_adapters=codex_exec"));

        run_cli(vec![
            "adapter".to_string(),
            "smoke-report".to_string(),
            "record".to_string(),
            "--adapter".to_string(),
            "codex".to_string(),
            "--status".to_string(),
            "passed".to_string(),
            "--credential-scan".to_string(),
            "clean".to_string(),
            "--marker-found".to_string(),
            "--artifact-root".to_string(),
            artifact_root.display().to_string(),
            "--reason".to_string(),
            "operator recorded clean opt-in smoke".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("record passed smoke");

        let ready = run_cli(vec![
            "adapter".to_string(),
            "dogfood-gate".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("ready gate");
        assert!(ready.contains("ready_for_first_real_agent_dogfood=true"));
        assert!(ready.contains("status=ready_for_first_real_agent_dogfood"));
        assert!(ready.contains("proven_adapters=codex_exec"));

        let dashboard = run_cli(vec![
            "dashboard".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("dashboard");
        assert!(dashboard.contains("adapter_dogfood_gate=true"));
        assert!(dashboard.contains("ready_for_first_real_agent_dogfood=true"));
    }

    #[test]
    fn workpad_index_imports_markdown_refs_without_modifying_sources() {
        let state_root = temp_root("workpad-index-state");
        let project_root = temp_root("workpad-index-project");
        fs::create_dir_all(project_root.join("workpads/features")).expect("feature dir");
        fs::write(
            project_root.join("TASKS.md"),
            "# Project Task Queue\n\n## Objective\n\nRoute work.\n\n## F2 - Workpad Dogfood Bridge\n\nStatus: pending\n",
        )
        .expect("write tasks");
        fs::write(
            project_root.join("project.md"),
            "# Capo\n\n## Objective\n\nBuild Capo.\n",
        )
        .expect("write project");
        fs::write(
            project_root.join("workpads/features/tasks.md"),
            "# Feature Tasks\n\n## Objective\n\nSplit work.\n\n## F1 - Real Local Agent Connector Proof\n\nStatus: pending\n\n## F2 - Workpad Dogfood Bridge\n\nStatus: in_progress\n",
        )
        .expect("write feature tasks");
        let before = fs::read_to_string(project_root.join("workpads/features/tasks.md"))
            .expect("read before");

        let output = run_cli(vec![
            "workpad".to_string(),
            "index".to_string(),
            "--root".to_string(),
            project_root.display().to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("index workpads");

        assert!(output.contains("workpads_indexed=true"));
        assert!(output.contains("files=3"));
        assert!(output.contains("tasks=3"));
        let state = SqliteStateStore::open(&state_root).expect("state");
        state
            .rebuild_projections()
            .expect("rebuild workpad projections");
        let files = state.workpad_files(&project_id()).expect("workpad files");
        let tasks = state.workpad_tasks(&project_id()).expect("workpad tasks");
        assert_eq!(files.len(), 3);
        assert!(files.iter().any(|file| file.path == "TASKS.md"));
        assert!(files.iter().any(|file| {
            file.path == "workpads/features/tasks.md"
                && file.objective.as_deref() == Some("Split work.")
        }));
        assert_eq!(
            tasks
                .iter()
                .find(|task| task.workpad_task_id == "workpads:features:tasks.md#f2")
                .map(|task| {
                    (
                        task.observed_status.as_str(),
                        task.capo_execution_status.as_str(),
                    )
                }),
            Some(("in_progress", "observed_only"))
        );
        assert_eq!(
            fs::read_to_string(project_root.join("workpads/features/tasks.md"))
                .expect("read after"),
            before
        );
        let next_output = run_cli(vec![
            "workpad".to_string(),
            "next".to_string(),
            "--path".to_string(),
            "workpads/features/tasks.md".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("select next indexed workpad task");
        assert!(next_output.contains("workpad_next_found=true"));
        assert!(next_output.contains("workpad_task_id=workpads:features:tasks.md#f2"));
        assert!(next_output.contains("observed_status=in_progress"));
        assert!(next_output.contains("capo_execution_status=observed_only"));
        assert!(next_output.contains("default_task_id=task-workpad-workpads-features-tasks-md-f2"));
        let dashboard_after_index = run_cli(vec![
            "dashboard".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("dashboard after workpad index");
        assert!(dashboard_after_index.contains("workpad_tasks=3"));
        assert!(dashboard_after_index.contains("workpad_task=workpads:features:tasks.md#f2"));
        assert!(dashboard_after_index.contains("capo_execution_status=observed_only"));
        let dashboard_by_workpad = run_cli(vec![
            "dashboard".to_string(),
            "--workpad-path".to_string(),
            "workpads/features/tasks.md".to_string(),
            "--workpad-status".to_string(),
            "in_progress".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("dashboard filtered by workpad task");
        assert!(dashboard_by_workpad.contains("workpad_tasks=1"));
        assert!(dashboard_by_workpad.contains("workpad_task=workpads:features:tasks.md#f2"));
        assert!(!dashboard_by_workpad.contains("workpad_task=TASKS.md#f2"));
        let plan_next = run_cli(vec![
            "workpad".to_string(),
            "plan-next".to_string(),
            "--agent".to_string(),
            "codex-dogfood".to_string(),
            "--adapter".to_string(),
            "codex".to_string(),
            "--path".to_string(),
            "workpads/features/tasks.md".to_string(),
            "--record".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("plan next workpad task for adapter");
        assert!(plan_next.contains("workpad_next_planned=true"));
        assert!(plan_next.contains("adapter=codex_exec"));
        assert!(plan_next.contains("workpad_task_id=workpads:features:tasks.md#f2"));
        assert!(plan_next.contains("runtime_prompt_policy=not_rendered"));
        assert!(plan_next.contains("runtime_prompt_source_kind=workpad_task"));
        assert!(
            plan_next.contains("runtime_prompt_materialization=replayable_if_source_hash_matches")
        );
        assert!(plan_next.contains("provider_cli_executed=false"));
        assert!(plan_next.contains("recorded=true"));
        assert!(!plan_next.contains("Work on Workpad Dogfood Bridge"));
        let plans = state
            .adapter_dispatch_plans(&project_id())
            .expect("dispatch plans");
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].adapter_kind, "codex_exec");
        assert_eq!(plans[0].runtime_prompt_policy, "not_rendered");
        assert!(!plans[0].provider_cli_executed);
        let prompt_sources = state
            .adapter_dispatch_prompt_sources(&project_id())
            .expect("dispatch prompt sources");
        assert_eq!(prompt_sources.len(), 1);
        assert_eq!(prompt_sources[0].source_kind, "workpad_task");
        assert_eq!(
            prompt_sources[0].source_ref.as_deref(),
            Some("workpads/features/tasks.md#F2 - Workpad Dogfood Bridge")
        );
        assert_eq!(
            prompt_sources[0].materialization_status,
            "replayable_if_source_hash_matches"
        );
        let materialize = run_cli(vec![
            "adapter".to_string(),
            "materialize-prompt".to_string(),
            "--dispatch-plan".to_string(),
            plans[0].dispatch_plan_id.clone(),
            "--record".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("materialize workpad prompt");
        assert!(materialize.contains("adapter_dispatch_prompt_materialization=true"));
        assert!(materialize.contains("status=ready_without_rendering_prompt"));
        assert!(materialize.contains("reasons=prompt_hash_matches_source"));
        assert!(materialize.contains("raw_prompt_policy=not_rendered"));
        assert!(!materialize.contains("Work on Workpad Dogfood Bridge"));
        let artifact_root = temp_root("workpad-plan-dispatch-smoke-artifacts");
        fs::create_dir_all(&artifact_root).expect("artifact dir");
        fs::write(artifact_root.join("stdout.txt"), "CAPO_CODEX_SMOKE_OK\n").expect("artifact");
        run_cli(vec![
            "adapter".to_string(),
            "smoke-report".to_string(),
            "record".to_string(),
            "--adapter".to_string(),
            "codex".to_string(),
            "--status".to_string(),
            "passed".to_string(),
            "--credential-scan".to_string(),
            "clean".to_string(),
            "--marker-found".to_string(),
            "--artifact-root".to_string(),
            artifact_root.display().to_string(),
            "--reason".to_string(),
            "operator recorded clean opt-in smoke".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("record passed smoke");
        let ready_gate = run_cli(vec![
            "adapter".to_string(),
            "dispatch-gate".to_string(),
            "--dispatch-plan".to_string(),
            plans[0].dispatch_plan_id.clone(),
            "--record".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("record ready dispatch gate");
        assert!(ready_gate.contains("status=ready_for_execution"));
        let execution_request = run_cli(vec![
            "adapter".to_string(),
            "execution-request".to_string(),
            "--dispatch-plan".to_string(),
            plans[0].dispatch_plan_id.clone(),
            "--record".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("record execution request");
        assert!(execution_request.contains("status=waiting_on_explicit_provider_opt_in"));
        let run_preflight = run_cli(vec![
            "adapter".to_string(),
            "run-preflight".to_string(),
            "--dispatch-plan".to_string(),
            plans[0].dispatch_plan_id.clone(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("run dispatch preflight");
        assert!(run_preflight.contains("adapter_dispatch_run_preflight=true"));
        assert!(run_preflight.contains("status=blocked_missing_explicit_provider_opt_in"));
        assert!(run_preflight.contains("provider_cli_execution_allowed=false"));
        assert!(run_preflight.contains("provider_cli_executed=false"));
        assert!(run_preflight.contains("opt_in_env=CAPO_RUN_CODEX_LOCAL_DISPATCH"));
        assert!(run_preflight.contains("CAPO_RUN_CODEX_LOCAL_DISPATCH=1_required"));
        assert!(run_preflight.contains("raw_prompt_policy=not_rendered"));
        assert!(!run_preflight.contains("Work on Workpad Dogfood Bridge"));
        let planned_workpad_task = state
            .workpad_task(&project_id(), "workpads:features:tasks.md#f2")
            .expect("planned workpad task query")
            .expect("planned workpad task");
        assert_eq!(planned_workpad_task.capo_execution_status, "observed_only");
        let dashboard_after_plan = run_cli(vec![
            "dashboard".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("dashboard after plan-next");
        assert!(dashboard_after_plan.contains("adapter_dispatch_plans=1"));
        assert!(dashboard_after_plan.contains("adapter_dispatch_prompt_sources=1"));
        assert!(dashboard_after_plan.contains("adapter_dispatch_prompt_materializations=1"));
        assert!(dashboard_after_plan.contains("status=ready_without_rendering_prompt"));
        assert!(dashboard_after_plan.contains("source_kind=workpad_task"));
        assert!(!dashboard_after_plan.contains("Work on Workpad Dogfood Bridge"));
        let source_hash = files
            .iter()
            .find(|file| file.path == "workpads/features/tasks.md")
            .expect("feature tasks file")
            .content_hash
            .clone();

        let import_output = run_cli(vec![
            "workpad".to_string(),
            "import".to_string(),
            "--workpad-task".to_string(),
            "workpads:features:tasks.md#f2".to_string(),
            "--expected-hash".to_string(),
            source_hash.clone(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("import workpad task");
        assert!(import_output.contains("workpad_task_imported=true"));
        assert!(import_output.contains("task_id=task-workpad-workpads-features-tasks-md-f2"));
        assert!(import_output.contains(&format!("source_hash={source_hash}")));
        let imported_task = state
            .task(&TaskId::new("task-workpad-workpads-features-tasks-md-f2"))
            .expect("imported task query")
            .expect("imported task");
        assert_eq!(imported_task.capo_execution_status, "ready");
        assert!(
            imported_task
                .latest_summary
                .as_deref()
                .is_some_and(|summary| summary
                    .contains("workpads/features/tasks.md#F2 - Workpad Dogfood Bridge")
                    && summary.contains(&format!("hash={source_hash}"))
                    && summary.contains("observed_status=in_progress"))
        );
        let imported_workpad_task = state
            .workpad_task(&project_id(), "workpads:features:tasks.md#f2")
            .expect("workpad task query")
            .expect("workpad task");
        assert_eq!(imported_workpad_task.observed_status, "in_progress");
        assert_eq!(imported_workpad_task.capo_execution_status, "imported");
        let next_after_import = run_cli(vec![
            "workpad".to_string(),
            "next".to_string(),
            "--path".to_string(),
            "workpads/features/tasks.md".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("select next after imported task");
        assert!(next_after_import.contains("workpad_next_found=true"));
        assert!(next_after_import.contains("workpad_task_id=workpads:features:tasks.md#f1"));
        assert!(next_after_import.contains("observed_status=pending"));
        assert!(next_after_import.contains("capo_execution_status=observed_only"));
        let missing_agent_start = run_cli(vec![
            "workpad".to_string(),
            "start-next".to_string(),
            "--agent".to_string(),
            "missing".to_string(),
            "--path".to_string(),
            "workpads/features/tasks.md".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect_err("missing agent should fail before import");
        assert!(missing_agent_start.contains("missing registered agent"));
        let next_after_missing_agent = run_cli(vec![
            "workpad".to_string(),
            "next".to_string(),
            "--path".to_string(),
            "workpads/features/tasks.md".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("missing agent should not consume next task");
        assert!(next_after_missing_agent.contains("workpad_task_id=workpads:features:tasks.md#f1"));
        assert!(next_after_missing_agent.contains("capo_execution_status=observed_only"));
        run_cli(vec![
            "agent".to_string(),
            "register".to_string(),
            "--name".to_string(),
            "dogfood".to_string(),
            "--adapter".to_string(),
            "fake".to_string(),
            "--runtime".to_string(),
            "fake".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("register dogfood agent");
        let started = run_cli(vec![
            "workpad".to_string(),
            "start-next".to_string(),
            "--agent".to_string(),
            "dogfood".to_string(),
            "--path".to_string(),
            "workpads/features/tasks.md".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("start next workpad task");
        assert!(started.contains("workpad_next_started=true"));
        assert!(started.contains("workpad_task_id=workpads:features:tasks.md#f1"));
        assert!(started.contains("task_id=task-workpad-workpads-features-tasks-md-f1"));
        assert!(started.contains("capo_execution_status=active"));
        let started_task = state
            .task(&TaskId::new("task-workpad-workpads-features-tasks-md-f1"))
            .expect("started task query")
            .expect("started task");
        assert_eq!(started_task.capo_execution_status, "active");
        assert_eq!(
            fs::read_to_string(project_root.join("workpads/features/tasks.md"))
                .expect("read source after start-next"),
            before
        );
        let proposal_dir = temp_root("workpad-proposal");
        let proposal_output = run_cli(vec![
            "workpad".to_string(),
            "propose".to_string(),
            "--workpad-task".to_string(),
            "workpads:features:tasks.md#f2".to_string(),
            "--expected-hash".to_string(),
            source_hash.clone(),
            "--out".to_string(),
            proposal_dir.display().to_string(),
            "--summary".to_string(),
            "Mark DB3 reviewed artifacts complete after verification.".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("write proposal");
        assert!(proposal_output.contains("workpad_proposal_written=true"));
        assert!(proposal_output.contains("source_hash="));
        let proposal_path = proposal_output
            .lines()
            .find_map(|line| line.strip_prefix("path="))
            .map(PathBuf::from)
            .expect("proposal path");
        let proposal = fs::read_to_string(&proposal_path).expect("read proposal");
        assert!(proposal.starts_with("<!-- capo:workpad-proposal -->"));
        assert!(proposal.contains("## Apply Policy"));
        assert!(proposal.contains("Automated source writeback is disabled"));
        assert!(proposal.contains("## Rollback And Fallback"));
        assert!(proposal.contains("Mark DB3 reviewed artifacts complete"));
        assert_eq!(
            fs::read_to_string(project_root.join("workpads/features/tasks.md"))
                .expect("read source after proposal"),
            before
        );
        let apply_without_confirm = run_cli(vec![
            "workpad".to_string(),
            "apply".to_string(),
            "--proposal".to_string(),
            proposal_path.display().to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect_err("apply should require confirmation");
        assert!(apply_without_confirm.contains("explicit --confirm is required"));
        let apply_with_confirm = run_cli(vec![
            "workpad".to_string(),
            "apply".to_string(),
            "--proposal".to_string(),
            proposal_path.display().to_string(),
            "--confirm".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("confirmed apply is guarded no-op in DB3");
        assert!(apply_with_confirm.contains("workpad_apply_supported=false"));
        assert!(apply_with_confirm.contains("source_modified=false"));
        assert_eq!(
            fs::read_to_string(project_root.join("workpads/features/tasks.md"))
                .expect("read source after apply"),
            before
        );
        let second_proposal_output = run_cli(vec![
            "workpad".to_string(),
            "propose".to_string(),
            "--workpad-task".to_string(),
            "workpads:features:tasks.md#f2".to_string(),
            "--expected-hash".to_string(),
            source_hash.clone(),
            "--out".to_string(),
            proposal_dir.display().to_string(),
            "--summary".to_string(),
            "A different reviewed proposal body gets a distinct artifact.".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("write second proposal");
        let second_proposal_path = second_proposal_output
            .lines()
            .find_map(|line| line.strip_prefix("path="))
            .map(PathBuf::from)
            .expect("second proposal path");
        assert_ne!(proposal_path, second_proposal_path);
        assert!(proposal_path.exists());
        assert!(second_proposal_path.exists());
        fs::write(
            &second_proposal_path,
            format!("{proposal}\nmanual review note\n"),
        )
        .expect("mutate Capo proposal");
        let changed_proposal_overwrite = run_cli(vec![
            "workpad".to_string(),
            "propose".to_string(),
            "--workpad-task".to_string(),
            "workpads:features:tasks.md#f2".to_string(),
            "--expected-hash".to_string(),
            source_hash.clone(),
            "--out".to_string(),
            proposal_dir.display().to_string(),
            "--summary".to_string(),
            "A different reviewed proposal body gets a distinct artifact.".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect_err("proposal should not overwrite changed Capo file");
        assert!(changed_proposal_overwrite.contains("refusing to overwrite changed Capo"));
        fs::write(&proposal_path, "# user-authored proposal\n").expect("replace proposal");
        let proposal_overwrite = run_cli(vec![
            "workpad".to_string(),
            "propose".to_string(),
            "--workpad-task".to_string(),
            "workpads:features:tasks.md#f2".to_string(),
            "--expected-hash".to_string(),
            source_hash.clone(),
            "--out".to_string(),
            proposal_dir.display().to_string(),
            "--summary".to_string(),
            "Mark DB3 reviewed artifacts complete after verification.".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect_err("proposal should not overwrite foreign file");
        assert!(proposal_overwrite.contains("refusing to overwrite non-Capo"));

        let conflicting_task_id = TaskId::new("task-existing-active");
        state
            .append_event(
                NewEvent {
                    event_id: "event-existing-active-task".to_string(),
                    kind: EventKind::TaskDiscovered,
                    actor: "test".to_string(),
                    project_id: Some(project_id()),
                    task_id: Some(conflicting_task_id.clone()),
                    agent_id: None,
                    session_id: None,
                    run_id: None,
                    turn_id: None,
                    item_id: None,
                    payload_json: "{}".to_string(),
                    idempotency_key: None,
                    redaction_state: RedactionState::Safe,
                },
                &[ProjectionRecord::Task(capo_state::TaskProjection {
                    task_id: conflicting_task_id.clone(),
                    project_id: project_id(),
                    title: "Existing active task".to_string(),
                    capo_execution_status: "active".to_string(),
                    active_session_id: None,
                    latest_summary: Some("unrelated task".to_string()),
                    evidence_id: None,
                    updated_sequence: 0,
                })],
            )
            .expect("existing active task");
        let collision_error = run_cli(vec![
            "workpad".to_string(),
            "import".to_string(),
            "--workpad-task".to_string(),
            "workpads:features:tasks.md#f2".to_string(),
            "--task".to_string(),
            conflicting_task_id.to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect_err("import should not overwrite existing Capo task");
        assert!(collision_error.contains("refusing to overwrite existing Capo task"));

        let event_count = state.event_count().expect("event count before re-import");
        run_cli(vec![
            "workpad".to_string(),
            "import".to_string(),
            "--workpad-task".to_string(),
            "workpads:features:tasks.md#f2".to_string(),
            "--expected-hash".to_string(),
            source_hash.clone(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("idempotent import");
        assert_eq!(
            state.event_count().expect("event count unchanged"),
            event_count
        );

        run_cli(vec![
            "workpad".to_string(),
            "index".to_string(),
            "--root".to_string(),
            project_root.display().to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("re-index workpads without source change");
        let imported_workpad_task = state
            .workpad_task(&project_id(), "workpads:features:tasks.md#f2")
            .expect("workpad task after re-index")
            .expect("workpad task after re-index");
        assert_eq!(imported_workpad_task.observed_status, "in_progress");
        assert_eq!(imported_workpad_task.capo_execution_status, "imported");

        fs::write(
            project_root.join("workpads/features/tasks.md"),
            "# Feature Tasks\n\n## Objective\n\nSplit work updated.\n\n## F1 - Real Local Agent Connector Proof\n\nStatus: pending\n\n## F2 - Workpad Dogfood Bridge\n\nStatus: in_progress\n",
        )
        .expect("drift feature tasks");
        run_cli(vec![
            "workpad".to_string(),
            "index".to_string(),
            "--root".to_string(),
            project_root.display().to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("re-index drifted workpads");
        let drift_error = run_cli(vec![
            "workpad".to_string(),
            "import".to_string(),
            "--workpad-task".to_string(),
            "workpads:features:tasks.md#f2".to_string(),
            "--expected-hash".to_string(),
            source_hash,
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect_err("old hash should detect source drift");
        assert!(drift_error.contains("source drift detected"));

        fs::write(
            project_root.join("workpads/features/tasks.md"),
            "# Feature Tasks\n\n## Objective\n\nSplit work.\n\n## F1 - Real Local Agent Connector Proof\n\nStatus: pending\n",
        )
        .expect("remove f2 from feature tasks");
        run_cli(vec![
            "workpad".to_string(),
            "index".to_string(),
            "--root".to_string(),
            project_root.display().to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("re-index workpads");
        state.rebuild_projections().expect("rebuild after re-index");
        let tasks = state
            .workpad_tasks(&project_id())
            .expect("tasks after re-index");
        assert!(
            !tasks
                .iter()
                .any(|task| task.workpad_task_id == "workpads:features:tasks.md#f2")
        );

        fs::write(project_root.join("workpads/features/tasks.md"), before)
            .expect("restore original feature tasks");
        run_cli(vec![
            "workpad".to_string(),
            "index".to_string(),
            "--root".to_string(),
            project_root.display().to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("re-index restored workpads");
        let tasks = state
            .workpad_tasks(&project_id())
            .expect("tasks after source recurrence");
        assert!(
            tasks
                .iter()
                .any(|task| task.workpad_task_id == "workpads:features:tasks.md#f2"),
            "restored source task should reappear after A-B-A fingerprint recurrence"
        );
    }

    #[test]
    fn permission_approval_queue_maps_decisions_to_scoped_grants() {
        let state_root = temp_root("permission-approval-state");

        let request_output = run_cli(vec![
            "permission".to_string(),
            "request".to_string(),
            "--approval".to_string(),
            "approval-evidence-record".to_string(),
            "--profile".to_string(),
            "trusted-local-dev".to_string(),
            "--session".to_string(),
            "session-test".to_string(),
            "--tool-call".to_string(),
            "tool-call-evidence".to_string(),
            "--scope-json".to_string(),
            "[\"tool:invoke:capo.evidence_record\",\"state:write:evidence\"]".to_string(),
            "--subject-json".to_string(),
            "{\"actor\":\"local-user\",\"agent\":\"codex\"}".to_string(),
            "--reason".to_string(),
            "record evidence".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("request approval");
        assert!(request_output.contains("permission_approval_queued=true"));
        assert!(request_output.contains("status=pending"));

        let pending = run_cli(vec![
            "permission".to_string(),
            "list".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("list pending approvals");
        assert!(pending.contains("permission_approvals=1"));
        assert!(pending.contains("approval=approval-evidence-record status=pending"));

        let decide_output = run_cli(vec![
            "permission".to_string(),
            "decide".to_string(),
            "--approval".to_string(),
            "approval-evidence-record".to_string(),
            "--decision".to_string(),
            "allow_once".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("decide approval");
        assert!(decide_output.contains("permission_approval_decided=true"));
        assert!(decide_output.contains("effect=allow"));
        assert!(decide_output.contains("persistence=once"));
        let grant_id = decide_output
            .lines()
            .find_map(|line| line.strip_prefix("capability_grant_id="))
            .expect("grant id")
            .to_string();

        let decided = run_cli(vec![
            "permission".to_string(),
            "list".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("list decided approvals");
        assert!(decided.contains("approval=approval-evidence-record status=decided"));
        assert!(decided.contains("decision=allow_once"));
        assert!(decided.contains(&format!("grant={grant_id}")));

        let second_decision = run_cli(vec![
            "permission".to_string(),
            "decide".to_string(),
            "--approval".to_string(),
            "approval-evidence-record".to_string(),
            "--decision".to_string(),
            "reject_once".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect_err("decided approval cannot be decided again");
        assert!(second_decision.contains("approval is not pending"));

        let state = SqliteStateStore::open(&state_root).expect("state");
        state
            .rebuild_projections()
            .expect("rebuild approval projections");
        let approval = state
            .permission_approval(&project_id(), "approval-evidence-record")
            .expect("approval query")
            .expect("approval read model");
        assert_eq!(approval.status, "decided");
        assert_eq!(approval.decision.as_deref(), Some("allow_once"));
        assert_eq!(
            approval.capability_grant_id.as_deref(),
            Some(grant_id.as_str())
        );
        let grant = state
            .capability_grants()
            .expect("grant query")
            .into_iter()
            .find(|grant| grant.capability_grant_id == grant_id)
            .expect("grant");
        assert_eq!(grant.effect, "allow");
        assert_eq!(grant.persistence, "once");
        assert_eq!(grant.decision_source, "user");

        run_cli(vec![
            "permission".to_string(),
            "request".to_string(),
            "--approval".to_string(),
            "approval-shell".to_string(),
            "--scope-json".to_string(),
            "[\"tool:invoke:shell\"]".to_string(),
            "--reason".to_string(),
            "run shell".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("request second approval");
        run_cli(vec![
            "permission".to_string(),
            "decide".to_string(),
            "--approval".to_string(),
            "approval-shell".to_string(),
            "--decision".to_string(),
            "reject_always".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("reject second approval");
        let denied = state
            .capability_grants()
            .expect("grant query")
            .into_iter()
            .find(|grant| {
                grant
                    .explanation
                    .contains("reject_always for approval-shell")
            })
            .expect("deny grant");
        assert_eq!(denied.effect, "deny");
        assert_eq!(denied.persistence, "until_revoked");

        run_cli(vec![
            "permission".to_string(),
            "request".to_string(),
            "--approval".to_string(),
            "approval-reject-once".to_string(),
            "--scope-json".to_string(),
            "[\"tool:invoke:shell\"]".to_string(),
            "--reason".to_string(),
            "reject one shell request".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("request reject-once approval");
        let reject_once = run_cli(vec![
            "permission".to_string(),
            "decide".to_string(),
            "--approval".to_string(),
            "approval-reject-once".to_string(),
            "--decision".to_string(),
            "reject_once".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("reject once");
        assert!(reject_once.contains("capability_grant_id=none"));
        let reject_once_approval = state
            .permission_approval(&project_id(), "approval-reject-once")
            .expect("reject-once approval query")
            .expect("reject-once approval");
        assert_eq!(
            reject_once_approval.decision.as_deref(),
            Some("reject_once")
        );
        assert!(reject_once_approval.capability_grant_id.is_none());

        run_cli(vec![
            "permission".to_string(),
            "request".to_string(),
            "--approval".to_string(),
            "approval-broad-always".to_string(),
            "--scope-json".to_string(),
            "[\"tool:invoke:shell\"]".to_string(),
            "--reason".to_string(),
            "broad remembered allow".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("request broad allow-always approval");
        let broad_always = run_cli(vec![
            "permission".to_string(),
            "decide".to_string(),
            "--approval".to_string(),
            "approval-broad-always".to_string(),
            "--decision".to_string(),
            "allow_always".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect_err("broad remembered allow is rejected");
        assert!(broad_always.contains("allow_always is restricted"));

        let bad_scope = run_cli(vec![
            "permission".to_string(),
            "request".to_string(),
            "--approval".to_string(),
            "approval-bad".to_string(),
            "--scope-json".to_string(),
            "{\"scope\":\"tool:invoke:shell\"}".to_string(),
            "--reason".to_string(),
            "bad scope".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect_err("object scope is rejected");
        assert!(bad_scope.contains("JSON array of strings"));
    }

    #[test]
    fn cli_drives_fake_controller_and_exports_evidence() {
        let state_root = temp_root("cli-state");
        let evidence_dir = temp_root("cli-evidence");

        assert!(
            run_cli(vec![
                "init".to_string(),
                "--state".to_string(),
                state_root.display().to_string(),
            ])
            .unwrap()
            .contains("initialized=true")
        );

        run_cli(vec![
            "agent".to_string(),
            "register".to_string(),
            "--name".to_string(),
            "fake-codex".to_string(),
            "--adapter".to_string(),
            "fake".to_string(),
            "--runtime".to_string(),
            "fake".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .unwrap();

        let send = run_cli(vec![
            "task".to_string(),
            "send".to_string(),
            "--agent".to_string(),
            "fake-codex".to_string(),
            "--goal".to_string(),
            "Inspect the project and write a short status summary".to_string(),
            "--scenario".to_string(),
            "tool-memory".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .unwrap();
        assert!(send.contains("session_id=session-fake-codex"));

        let status = run_cli(vec![
            "session".to_string(),
            "status".to_string(),
            "--agent".to_string(),
            "fake-codex".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .unwrap();
        assert!(status.contains("current_goal=Inspect the project"));
        assert!(status.contains("kind=tool.call_completed"));
        assert!(status.contains("evidence_refs=evidence-fake-codex"));

        let interrupted = run_cli(vec![
            "session".to_string(),
            "interrupt".to_string(),
            "--agent".to_string(),
            "fake-codex".to_string(),
            "--reason".to_string(),
            "smoke interrupt".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .unwrap();
        assert!(interrupted.contains("status=canceled"));

        assert!(
            run_cli(vec![
                "recover".to_string(),
                "--state".to_string(),
                state_root.display().to_string(),
            ])
            .unwrap()
            .contains("recovered=true")
        );

        let export = run_cli(vec![
            "evidence".to_string(),
            "export".to_string(),
            "--session".to_string(),
            "session-fake-codex".to_string(),
            "--out".to_string(),
            evidence_dir.display().to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .unwrap();
        assert!(export.contains("evidence_exported=true"));
        let evidence_path = evidence_dir.join("session-fake-codex.md");
        let exported = fs::read_to_string(&evidence_path).expect("read evidence export");
        assert!(exported.starts_with("<!-- capo:evidence-export -->"));
        assert!(exported.contains("## State Refs"));
        assert!(exported.contains("- Session status: `canceled`"));
        assert!(exported.contains("- Run status: `exited_unknown`"));
        assert!(exported.contains("- `evidence-fake-codex`"));
        assert!(exported.contains("artifact=`artifact-tool-session-fake-codex`"));
        assert!(exported.contains("## Tool Calls"));
        assert!(exported.contains("origin=`capo` status=`completed`"));
        assert!(exported.contains("## Memory Packets"));
        assert!(exported.contains("artifact=`artifact-memory-packet-packet-fake-codex`"));
        assert!(exported.contains("session.interrupted"));
        assert!(!exported.contains("OPENAI_API_KEY"));
        assert!(!exported.contains("ANTHROPIC_API_KEY"));
        let state = SqliteStateStore::open(&state_root).expect("open state");
        let review_recorded = run_cli(vec![
            "review".to_string(),
            "record".to_string(),
            "--session".to_string(),
            "session-fake-codex".to_string(),
            "--reviewer".to_string(),
            "focused-review".to_string(),
            "--kind".to_string(),
            "no_blockers".to_string(),
            "--summary".to_string(),
            "No blockers in exported fake controller evidence.".to_string(),
            "--out".to_string(),
            evidence_dir.display().to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("record no-blockers review");
        assert!(review_recorded.contains("review_finding_recorded=true"));

        let outcome = run_cli(vec![
            "eval".to_string(),
            "task-outcome".to_string(),
            "--session".to_string(),
            "session-fake-codex".to_string(),
            "--out".to_string(),
            evidence_dir.display().to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .unwrap();
        assert!(outcome.contains("task_outcome_report_exported=true"));
        assert!(outcome.contains("artifact_id=artifact-task-outcome-"));
        let reports = state
            .task_outcome_reports_for_task(&TaskId::new(
                "task-inspect-the-project-and-write-a-short-status-summary",
            ))
            .expect("task outcome reports");
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].review_outcome, "reviewed_no_blockers");
        assert!(reports[0].tool_call_count >= 1);
        assert!(reports[0].evidence_count >= 1);
        let report_path = evidence_dir.join(format!(
            "{}.md",
            reports[0]
                .report_artifact_id
                .as_deref()
                .expect("report artifact")
        ));
        let report = fs::read_to_string(report_path).expect("read task outcome report");
        assert!(report.starts_with("<!-- capo:task-outcome-report -->"));
        assert!(report.contains("Review outcome: `reviewed_no_blockers`"));
        assert!(report.contains("## Event Trace"));
        let rerun = run_cli(vec![
            "eval".to_string(),
            "task-outcome".to_string(),
            "--session".to_string(),
            "session-fake-codex".to_string(),
            "--out".to_string(),
            evidence_dir.display().to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .unwrap();
        assert!(rerun.contains("task_outcome_report_exported=true"));
        assert!(
            rerun.contains(
                reports[0]
                    .report_artifact_id
                    .as_deref()
                    .expect("stable report artifact")
            )
        );
        assert_eq!(
            state
                .task_outcome_reports_for_task(&TaskId::new(
                    "task-inspect-the-project-and-write-a-short-status-summary",
                ))
                .expect("task outcome reports")
                .len(),
            1
        );
        state
            .append_event(
                NewEvent::new(
                    "event-workpad-me3-follow-up",
                    EventKind::WorkpadIndexed,
                    "test",
                ),
                &[ProjectionRecord::WorkpadTask(WorkpadTaskProjection {
                    workpad_task_id: "ME3".to_string(),
                    project_id: project_id(),
                    path: "workpads/features/memory-eval.md".to_string(),
                    source_anchor: "ME3 - Review Feedback Loop".to_string(),
                    title: "Review Feedback Loop".to_string(),
                    observed_status: "pending".to_string(),
                    capo_execution_status: "observed_only".to_string(),
                    observed_unix: 1,
                    updated_sequence: 0,
                })],
            )
            .expect("append ME3 follow-up workpad task");

        let blocker_review = run_cli(vec![
            "review".to_string(),
            "record".to_string(),
            "--session".to_string(),
            "session-fake-codex".to_string(),
            "--reviewer".to_string(),
            "focused-review".to_string(),
            "--kind".to_string(),
            "blocker".to_string(),
            "--summary".to_string(),
            "Tool output needs follow-up workpad handling.".to_string(),
            "--tool-call".to_string(),
            "tool-fake-codex".to_string(),
            "--follow-up-workpad-task".to_string(),
            "ME3".to_string(),
            "--out".to_string(),
            evidence_dir.display().to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("record blocker review");
        assert!(blocker_review.contains("review_finding_recorded=true"));
        let blocker_outcome = run_cli(vec![
            "eval".to_string(),
            "task-outcome".to_string(),
            "--session".to_string(),
            "session-fake-codex".to_string(),
            "--out".to_string(),
            evidence_dir.display().to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .unwrap();
        assert!(blocker_outcome.contains("task_outcome_report_exported=true"));
        let blocker_reports = state
            .task_outcome_reports_for_task(&TaskId::new(
                "task-inspect-the-project-and-write-a-short-status-summary",
            ))
            .expect("task outcome reports after blocker review");
        assert!(
            blocker_reports
                .iter()
                .any(|report| report.review_outcome == "reviewed_with_findings")
        );
        let findings = state
            .review_findings_for_session(&SessionId::new("session-fake-codex"))
            .expect("review findings");
        assert_eq!(findings.len(), 2);
        let blocker = findings
            .iter()
            .find(|finding| finding.finding_kind == "blocker")
            .expect("blocker finding");
        assert_eq!(
            blocker
                .tool_call_id
                .as_ref()
                .map(ToString::to_string)
                .as_deref(),
            Some("tool-fake-codex")
        );
        assert_eq!(blocker.workpad_task_id.as_deref(), Some("ME3"));
        let review_artifact = fs::read_to_string(
            evidence_dir
                .join(
                    blocker
                        .evidence_artifact_id
                        .as_ref()
                        .expect("review artifact"),
                )
                .with_extension("md"),
        )
        .expect("read review artifact");
        assert!(review_artifact.starts_with("<!-- capo:review-finding -->"));
        assert!(review_artifact.contains("Follow-up workpad task: `ME3`"));
    }

    #[test]
    fn dashboard_rejects_malformed_filters() {
        let state_root = temp_root("cli-dashboard-filters");

        let missing_session = run_cli(vec![
            "dashboard".to_string(),
            "--session".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .unwrap_err();
        assert!(missing_session.contains("--session requires a value"));

        let missing_status = run_cli(vec![
            "dashboard".to_string(),
            "--status".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .unwrap_err();
        assert!(missing_status.contains("--status requires a value"));

        let unknown = run_cli(vec![
            "dashboard".to_string(),
            "--agent".to_string(),
            "fake-codex".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .unwrap_err();
        assert!(unknown.contains("unknown dashboard filter: --agent"));

        let missing_workpad_path = run_cli(vec![
            "dashboard".to_string(),
            "--workpad-path".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .unwrap_err();
        assert!(missing_workpad_path.contains("--workpad-path requires a value"));

        let missing_workpad_status = run_cli(vec![
            "dashboard".to_string(),
            "--workpad-status".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .unwrap_err();
        assert!(missing_workpad_status.contains("--workpad-status requires a value"));
    }

    #[test]
    fn dashboard_renders_review_findings_from_shared_query() {
        let state_root = temp_root("cli-dashboard-review-findings");
        let evidence_dir = temp_root("cli-dashboard-review-finding-evidence");
        seed_running_agent(&state_root, "fake-codex", "Inspect the project");

        let review = run_cli(vec![
            "review".to_string(),
            "record".to_string(),
            "--session".to_string(),
            "session-fake-codex".to_string(),
            "--reviewer".to_string(),
            "focused-review".to_string(),
            "--kind".to_string(),
            "blocker".to_string(),
            "--summary".to_string(),
            "Dashboard must expose review blockers.".to_string(),
            "--out".to_string(),
            evidence_dir.display().to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("record review finding");
        assert!(review.contains("review_finding_recorded=true"));
        let review_finding_id = output_value(&review, "review_finding_id");

        let dashboard = run_cli(vec![
            "dashboard".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("dashboard with review finding");

        assert!(dashboard.contains("review_findings=1"));
        assert!(dashboard.contains("session_review_findings=1"));
        assert!(dashboard.contains(&format!("project_review_finding={review_finding_id}")));
        assert!(dashboard.contains(&format!("review_finding={review_finding_id}")));
        assert!(dashboard.contains("kind=blocker"));
        assert!(dashboard.contains("severity=high"));
        assert!(dashboard.contains("status=open"));
        assert!(dashboard.contains("reviewer=focused-review"));
        assert!(dashboard.contains("summary=Dashboard must expose review blockers."));
    }

    #[test]
    fn dashboard_renders_task_outcome_reports_from_shared_query() {
        let state_root = temp_root("cli-dashboard-task-outcome");
        seed_running_agent(&state_root, "fake-codex", "Inspect the project");
        let report_id = "task-outcome-report-dashboard";
        let artifact_id = "artifact-task-outcome-dashboard";
        SqliteStateStore::open(&state_root)
            .expect("state")
            .append_event(
                NewEvent {
                    event_id: "event-cli-dashboard-task-outcome".to_string(),
                    kind: EventKind::TaskOutcomeReportGenerated,
                    actor: "test".to_string(),
                    project_id: Some(project_id()),
                    task_id: Some(TaskId::new("task-inspect-the-project")),
                    agent_id: Some(AgentId::new("agent-fake-codex")),
                    session_id: Some(SessionId::new("session-fake-codex")),
                    run_id: Some(RunId::new("run-fake-codex")),
                    turn_id: None,
                    item_id: Some(report_id.to_string()),
                    payload_json: "{}".to_string(),
                    idempotency_key: None,
                    redaction_state: RedactionState::Safe,
                },
                &[ProjectionRecord::TaskOutcomeReport(
                    capo_state::TaskOutcomeReportProjection {
                        task_outcome_report_id: report_id.to_string(),
                        project_id: project_id(),
                        task_id: TaskId::new("task-inspect-the-project"),
                        session_id: SessionId::new("session-fake-codex"),
                        run_id: RunId::new("run-fake-codex"),
                        outcome_status: "completed".to_string(),
                        started_sequence: 1,
                        completed_sequence: 10,
                        duration_sequence_span: 9,
                        action_count: 5,
                        tool_call_count: 1,
                        evidence_count: 2,
                        memory_packet_count: 1,
                        confidence: Some(82),
                        blocker: None,
                        review_outcome: "not_reviewed".to_string(),
                        report_artifact_id: Some(artifact_id.to_string()),
                        updated_sequence: 0,
                    },
                )],
            )
            .expect("append task outcome report");

        let dashboard = run_cli(vec![
            "dashboard".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("dashboard with task outcome report");

        assert!(dashboard.contains("task_outcome_reports=1"));
        assert!(dashboard.contains("session_task_outcome_reports=1"));
        assert!(dashboard.contains(&format!("project_task_outcome_report={report_id}")));
        assert!(dashboard.contains(&format!("task_outcome_report={report_id}")));
        assert!(dashboard.contains("outcome_status=completed"));
        assert!(dashboard.contains("review_outcome=not_reviewed"));
        assert!(dashboard.contains(&format!("artifact={artifact_id}")));
    }

    #[test]
    fn dashboard_renders_connectivity_exposure_state() {
        let state_root = temp_root("cli-dashboard-connectivity");
        let state = SqliteStateStore::open(&state_root).expect("state");
        state
            .append_event(
                NewEvent {
                    event_id: "event-cli-connectivity-exposure".to_string(),
                    kind: EventKind::ConnectivityExposureRequested,
                    actor: "test".to_string(),
                    project_id: Some(project_id()),
                    task_id: None,
                    agent_id: None,
                    session_id: None,
                    run_id: None,
                    turn_id: None,
                    item_id: Some("exposure-private-control".to_string()),
                    payload_json: "{}".to_string(),
                    idempotency_key: None,
                    redaction_state: RedactionState::Safe,
                },
                &[ProjectionRecord::ConnectivityExposure(
                    ConnectivityExposureProjection {
                        exposure_id: "exposure-private-control".to_string(),
                        project_id: project_id(),
                        connectivity_endpoint_id: "endpoint-private-1".to_string(),
                        owner_kind: "runtime_target".to_string(),
                        owner_id: "remote-target-1".to_string(),
                        channel_kind: "control".to_string(),
                        exposure: "private".to_string(),
                        permission_scope: "network:connect:private_tunnel".to_string(),
                        status: "blocked_pending_permission".to_string(),
                        capability_grant_id: None,
                        health_status: "unknown".to_string(),
                        reachable: false,
                        revoked_at: None,
                        updated_sequence: 0,
                    },
                )],
            )
            .expect("append exposure");

        let dashboard = run_cli(vec![
            "dashboard".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("dashboard");

        assert!(dashboard.contains("connectivity_exposures=1"));
        assert!(dashboard.contains("connectivity_exposure=exposure-private-control"));
        assert!(dashboard.contains("exposure_status=blocked_pending_permission"));
        assert!(dashboard.contains("permission_scope=network:connect:private_tunnel"));
        assert!(dashboard.contains("grant=none"));
    }

    #[test]
    fn connectivity_expose_stub_records_blocked_private_exposure_without_runtime_execution() {
        let state_root = temp_root("cli-connectivity-expose-stub");
        let planned = run_cli(vec![
            "connectivity".to_string(),
            "expose-stub".to_string(),
            "--endpoint".to_string(),
            "endpoint-private-1".to_string(),
            "--owner-kind".to_string(),
            "runtime_target".to_string(),
            "--owner-id".to_string(),
            "remote-target-1".to_string(),
            "--channel".to_string(),
            "control".to_string(),
            "--exposure".to_string(),
            "private".to_string(),
            "--record".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("record private exposure");

        assert!(planned.contains("connectivity_exposure_planned=true"));
        assert!(planned.contains("permission_required=true"));
        assert!(planned.contains("permission_scope=network:connect:private_tunnel"));
        assert!(planned.contains("status=blocked_pending_permission"));
        assert!(planned.contains("recorded=true"));

        let dashboard = run_cli(vec![
            "dashboard".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("dashboard");
        assert!(dashboard.contains("connectivity_exposures=1"));
        assert!(dashboard.contains("endpoint=endpoint-private-1"));
        assert!(dashboard.contains("owner=runtime_target:remote-target-1"));
        assert!(dashboard.contains("exposure_status=blocked_pending_permission"));
        assert!(dashboard.contains("permission_scope=network:connect:private_tunnel"));

        let denied = run_cli(vec![
            "connectivity".to_string(),
            "expose-stub".to_string(),
            "--endpoint".to_string(),
            "endpoint-public-1".to_string(),
            "--owner-kind".to_string(),
            "capo_server".to_string(),
            "--owner-id".to_string(),
            "server-1".to_string(),
            "--channel".to_string(),
            "control".to_string(),
            "--exposure".to_string(),
            "public".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .unwrap_err();
        assert!(denied.contains("connectivity endpoint resolution failed"));
        assert!(denied.contains("ChannelNotAllowed"));
    }

    #[test]
    fn connectivity_exposure_approval_activates_only_with_matching_grant() {
        let state_root = temp_root("cli-connectivity-exposure-approval");
        let planned = run_cli(vec![
            "connectivity".to_string(),
            "expose-stub".to_string(),
            "--endpoint".to_string(),
            "endpoint-private-1".to_string(),
            "--owner-kind".to_string(),
            "runtime_target".to_string(),
            "--owner-id".to_string(),
            "remote-target-1".to_string(),
            "--channel".to_string(),
            "control".to_string(),
            "--exposure".to_string(),
            "private".to_string(),
            "--record".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("record private exposure");
        let exposure_id = output_value(&planned, "exposure");

        let blocked = run_cli(vec![
            "connectivity".to_string(),
            "activate-exposure".to_string(),
            "--exposure".to_string(),
            exposure_id.clone(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .unwrap_err();
        assert!(blocked.contains("missing allow grant for connectivity exposure"));

        let approval = run_cli(vec![
            "connectivity".to_string(),
            "request-approval".to_string(),
            "--exposure".to_string(),
            exposure_id.clone(),
            "--approval".to_string(),
            "approval-private-control".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("request connectivity approval");
        assert!(approval.contains("connectivity_exposure_approval_requested=true"));
        assert!(approval.contains("approval=approval-private-control"));
        assert!(approval.contains("permission_scope=network:connect:private_tunnel"));

        let decided = run_cli(vec![
            "permission".to_string(),
            "decide".to_string(),
            "--approval".to_string(),
            "approval-private-control".to_string(),
            "--decision".to_string(),
            "allow_once".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("allow connectivity approval");
        assert!(decided.contains("permission_approval_decided=true"));
        assert!(decided.contains("decision=allow_once"));

        let activated = run_cli(vec![
            "connectivity".to_string(),
            "activate-exposure".to_string(),
            "--exposure".to_string(),
            exposure_id,
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("activate exposure");
        assert!(activated.contains("connectivity_exposure_activated=true"));
        assert!(activated.contains("status=active"));
        assert!(activated.contains("grant=grant-approval-"));

        let dashboard = run_cli(vec![
            "dashboard".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("dashboard");
        assert!(dashboard.contains("connectivity_exposures=1"));
        assert!(dashboard.contains("exposure_status=active"));
        assert!(dashboard.contains("grant=grant-approval-"));

        let exact_status = run_cli(vec![
            "connectivity".to_string(),
            "exposure-status".to_string(),
            "--exposure".to_string(),
            output_value(&activated, "exposure"),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("exact exposure status");
        assert!(exact_status.contains("connectivity_exposure_status=true"));
        assert!(exact_status.contains("status=active"));
        assert!(exact_status.contains("owner=runtime_target:remote-target-1"));

        let latest_status = run_cli(vec![
            "connectivity".to_string(),
            "exposure-status".to_string(),
            "--latest".to_string(),
            "--owner-kind".to_string(),
            "runtime_target".to_string(),
            "--owner-id".to_string(),
            "remote-target-1".to_string(),
            "--channel".to_string(),
            "control".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("latest exposure status");
        assert!(latest_status.contains("connectivity_exposure_status=true"));
        assert!(latest_status.contains("status=active"));
        assert_eq!(
            output_value(&latest_status, "exposure"),
            output_value(&activated, "exposure")
        );
        let before_voice_sequence = SqliteStateStore::open(&state_root)
            .expect("state")
            .last_sequence()
            .expect("sequence before voice");
        let voice_latest = run_cli(vec![
            "voice".to_string(),
            "submit".to_string(),
            "--transcript".to_string(),
            "What is the latest connectivity exposure status for runtime target remote-target-1?"
                .to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("voice latest connectivity exposure status");
        assert!(voice_latest.contains("voice_plan=connectivity_status"));
        assert!(voice_latest.contains("mutation_applied=false"));
        assert!(voice_latest.contains("raw_transcript_retained=false"));
        assert!(voice_latest.contains("read_scope=project_latest_connectivity_exposure"));
        assert!(voice_latest.contains("spoken_connectivity_exposure="));
        assert!(voice_latest.contains("spoken_owner=runtime_target:remote-target-1"));
        assert!(voice_latest.contains("spoken_channel=control"));
        assert!(voice_latest.contains("spoken_exposure_status=active"));
        assert!(voice_latest.contains("spoken_permission_scope=network:connect:private_tunnel"));
        assert!(!voice_latest.contains("What is the latest connectivity exposure status"));
        assert_eq!(
            SqliteStateStore::open(&state_root)
                .expect("state")
                .last_sequence()
                .expect("sequence after voice"),
            before_voice_sequence
        );

        let revoked = run_cli(vec![
            "connectivity".to_string(),
            "revoke-exposure".to_string(),
            "--exposure".to_string(),
            output_value(&activated, "exposure"),
            "--reason".to_string(),
            "operator closed private control surface".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("revoke exposure");
        assert!(revoked.contains("connectivity_exposure_revoked=true"));
        assert!(revoked.contains("status=revoked"));
        assert!(revoked.contains("health=disabled"));
        assert!(revoked.contains("reachable=false"));
        assert!(revoked.contains("revoked_at=unix:"));

        let dashboard = run_cli(vec![
            "dashboard".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("dashboard after revoke");
        assert!(dashboard.contains("connectivity_exposures=1"));
        assert!(dashboard.contains("exposure_status=revoked"));
        assert!(dashboard.contains("health=disabled"));
        assert!(dashboard.contains("reachable=false"));
        assert!(dashboard.contains("revoked_at=unix:"));

        let latest_status_after_revoke = run_cli(vec![
            "connectivity".to_string(),
            "exposure-status".to_string(),
            "--latest".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("latest exposure status after revoke");
        assert!(latest_status_after_revoke.contains("status=revoked"));

        let evidence_dir = temp_root("cli-connectivity-exposure-evidence");
        let evidence = run_cli(vec![
            "connectivity".to_string(),
            "exposure-evidence".to_string(),
            "--exposure".to_string(),
            output_value(&revoked, "exposure"),
            "--out".to_string(),
            evidence_dir.display().to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("export connectivity exposure evidence");
        assert!(evidence.contains("connectivity_exposure_evidence_exported=true"));
        assert!(evidence.contains("evidence_id=evidence-artifact-connectivity-exposure-evidence-"));
        let evidence_path = output_value(&evidence, "path");
        let markdown = fs::read_to_string(&evidence_path).expect("read connectivity evidence");
        assert!(markdown.starts_with("<!-- capo:connectivity-exposure-evidence -->"));
        assert!(markdown.contains("## Exposure"));
        assert!(markdown.contains("- Status: `revoked`"));
        assert!(markdown.contains("- Health: `disabled`"));
        assert!(markdown.contains("- Reachable: `false`"));
        assert!(markdown.contains("does not open tunnels"));

        let dashboard = run_cli(vec![
            "dashboard".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("dashboard after exposure evidence");
        assert!(dashboard.contains("project_evidence=1"));
        assert!(dashboard.contains("kind=connectivity_exposure_evidence"));
    }

    #[test]
    fn adapter_fixture_replay_cli_exports_evidence_without_raw_provider_text() {
        let state_root = temp_root("cli-adapter-replay-state");
        let evidence_dir = temp_root("cli-adapter-replay-evidence");
        let fixture = PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../capo-adapters/fixtures/codex-exec.jsonl"
        ));

        let output = run_cli(vec![
            "adapter".to_string(),
            "replay-fixture".to_string(),
            "--adapter".to_string(),
            "codex".to_string(),
            "--fixture".to_string(),
            fixture.display().to_string(),
            "--agent".to_string(),
            "replay-codex".to_string(),
            "--goal".to_string(),
            "Replay Codex fixture through Capo".to_string(),
            "--out".to_string(),
            evidence_dir.display().to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("adapter replay fixture");

        assert!(output.contains("adapter_replayed=true"));
        assert!(output.contains("adapter=codex_exec"));
        assert!(output.contains("session_id=session-replay-codex"));
        assert!(output.contains("tool_events=2"));
        assert!(output.contains("summary_events=1"));
        assert!(output.contains("completed_turns=1"));
        assert!(output.contains("evidence_exported=true"));
        assert!(!output.contains("Codex fixture response."));
        assert!(!output.contains("cargo test"));

        let evidence_path = evidence_dir.join("session-replay-codex.md");
        let evidence = fs::read_to_string(&evidence_path).expect("read replay evidence");
        assert!(evidence.contains("adapter_replay:codex_exec"));
        assert!(evidence.contains("adapter_native:codex_exec"));
        assert!(evidence.contains("content_hash="));
        assert!(!evidence.contains("Codex fixture response."));
        assert!(!evidence.contains("cargo test"));
        assert_text_absent_in_tree(&state_root, "Codex fixture response.");
        assert_text_absent_in_tree(&state_root, "cargo test");
        assert_text_absent_in_tree(&evidence_dir, "Codex fixture response.");
        assert_text_absent_in_tree(&evidence_dir, "cargo test");
    }

    #[test]
    fn voice_status_reads_shared_query_without_mutating_or_retaining_transcript() {
        let state_root = temp_root("cli-voice-status");
        seed_running_agent(&state_root, "fake-codex", "Inspect the project");
        let state = SqliteStateStore::open(&state_root).expect("state");
        let before_sequence = state.last_sequence().expect("before sequence");

        let output = run_cli(vec![
            "voice".to_string(),
            "submit".to_string(),
            "--transcript".to_string(),
            "What is fake-codex doing?".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("voice status");

        assert!(output.contains("voice_plan=agent_status"));
        assert!(output.contains("origin=voice"));
        assert!(output.contains("mutation_applied=false"));
        assert!(output.contains("raw_transcript_retained=false"));
        assert!(output.contains("memory_ingestion=none"));
        assert!(output.contains("read_scope=agent"));
        assert!(output.contains("spoken_agent=fake-codex agent_status=running"));
        assert!(output.contains("current_goal=Inspect the project"));
        assert!(!output.contains("What is fake-codex doing?"));
        assert_eq!(
            state.last_sequence().expect("after sequence"),
            before_sequence
        );
    }

    #[test]
    fn voice_recent_work_reads_project_and_agent_work_without_mutating() {
        let state_root = temp_root("cli-voice-recent-work");
        seed_running_agent(&state_root, "fake-codex", "Inspect the project");
        seed_running_agent(&state_root, "fake-reviewer", "Review the summary");
        let state = SqliteStateStore::open(&state_root).expect("state");
        let before_sequence = state.last_sequence().expect("before sequence");

        let project_output = run_cli(vec![
            "voice".to_string(),
            "submit".to_string(),
            "--transcript".to_string(),
            "What have my agents done?".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("voice project recent work");

        assert!(project_output.contains("voice_plan=recent_work"));
        assert!(project_output.contains("mutation_applied=false"));
        assert!(project_output.contains("raw_transcript_retained=false"));
        assert!(project_output.contains("read_scope=project_recent_work"));
        assert!(project_output.contains("spoken_agents=2"));
        assert!(project_output.contains("spoken_active_sessions=2"));
        assert!(project_output.contains("spoken_agent=fake-codex agent_status=running"));
        assert!(project_output.contains("spoken_agent=fake-reviewer agent_status=running"));
        assert!(
            project_output.contains("latest_summary=Fake adapter processed goal for fake-codex")
        );
        assert!(!project_output.contains("What have my agents done?"));

        let agent_output = run_cli(vec![
            "voice".to_string(),
            "submit".to_string(),
            "--transcript".to_string(),
            "What has fake-codex done?".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("voice agent recent work");

        assert!(agent_output.contains("voice_plan=recent_work"));
        assert!(agent_output.contains("mutation_applied=false"));
        assert!(agent_output.contains("raw_transcript_retained=false"));
        assert!(agent_output.contains("read_scope=agent"));
        assert!(agent_output.contains("spoken_agent=fake-codex agent_status=running"));
        assert!(agent_output.contains("current_goal=Inspect the project"));
        assert!(agent_output.contains("latest_summary=Fake adapter processed goal for fake-codex"));
        assert!(!agent_output.contains("What has fake-codex done?"));
        assert_eq!(
            state.last_sequence().expect("after sequence"),
            before_sequence
        );
    }

    #[test]
    fn voice_next_work_reads_workpad_queue_without_mutating() {
        let state_root = temp_root("cli-voice-next-work");
        let state = SqliteStateStore::open(&state_root).expect("state");
        state
            .append_event(
                NewEvent {
                    event_id: "event-cli-voice-next-work-pending".to_string(),
                    kind: EventKind::WorkpadIndexed,
                    actor: "test".to_string(),
                    project_id: Some(project_id()),
                    task_id: None,
                    agent_id: None,
                    session_id: None,
                    run_id: None,
                    turn_id: None,
                    item_id: Some("workpads:features:voice.md#v7".to_string()),
                    payload_json: "{}".to_string(),
                    idempotency_key: None,
                    redaction_state: RedactionState::Safe,
                },
                &[ProjectionRecord::WorkpadTask(WorkpadTaskProjection {
                    workpad_task_id: "workpads:features:voice.md#v7".to_string(),
                    project_id: project_id(),
                    path: "workpads/features/voice.md".to_string(),
                    source_anchor: "v7".to_string(),
                    title: "Next Work Conversation".to_string(),
                    observed_status: "pending".to_string(),
                    capo_execution_status: "observed_only".to_string(),
                    observed_unix: 1,
                    updated_sequence: 0,
                })],
            )
            .expect("append pending workpad task");
        state
            .append_event(
                NewEvent {
                    event_id: "event-cli-voice-next-work-imported".to_string(),
                    kind: EventKind::WorkpadIndexed,
                    actor: "test".to_string(),
                    project_id: Some(project_id()),
                    task_id: None,
                    agent_id: None,
                    session_id: None,
                    run_id: None,
                    turn_id: None,
                    item_id: Some("workpads:features:tasks.md#f1".to_string()),
                    payload_json: "{}".to_string(),
                    idempotency_key: None,
                    redaction_state: RedactionState::Safe,
                },
                &[ProjectionRecord::WorkpadTask(WorkpadTaskProjection {
                    workpad_task_id: "workpads:features:tasks.md#f1".to_string(),
                    project_id: project_id(),
                    path: "workpads/features/tasks.md".to_string(),
                    source_anchor: "f1".to_string(),
                    title: "Real Local Agent Connector Proof".to_string(),
                    observed_status: "in_progress".to_string(),
                    capo_execution_status: "imported".to_string(),
                    observed_unix: 1,
                    updated_sequence: 0,
                })],
            )
            .expect("append imported workpad task");
        let before_sequence = state.last_sequence().expect("before sequence");

        let output = run_cli(vec![
            "voice".to_string(),
            "submit".to_string(),
            "--transcript".to_string(),
            "What should we do next?".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("voice next work");

        assert!(output.contains("voice_plan=next_work"));
        assert!(output.contains("mutation_applied=false"));
        assert!(output.contains("raw_transcript_retained=false"));
        assert!(output.contains("read_scope=project_next_work"));
        assert!(output.contains("spoken_workpad_tasks=2"));
        assert!(output.contains("spoken_next_work_candidates=1"));
        assert!(output.contains("spoken_next_workpad_task=workpads:features:voice.md#v7"));
        assert!(output.contains("default_task_id=task-workpad-workpads-features-voice-md-v7"));
        assert!(output.contains("title=Next Work Conversation"));
        assert!(output.contains("observed_status=pending"));
        assert!(output.contains("capo_execution_status=observed_only"));
        assert!(!output.contains("What should we do next?"));
        assert_eq!(
            state.last_sequence().expect("after sequence"),
            before_sequence
        );
    }

    #[test]
    fn voice_confirmed_start_next_work_imports_and_dispatches_after_approval() {
        let state_root = temp_root("cli-voice-start-next-work");
        run_cli(vec![
            "agent".to_string(),
            "register".to_string(),
            "--name".to_string(),
            "fake-codex".to_string(),
            "--adapter".to_string(),
            "fake".to_string(),
            "--runtime".to_string(),
            "fake".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("register fake-codex");
        let state = SqliteStateStore::open(&state_root).expect("state");
        state
            .append_event(
                NewEvent {
                    event_id: "event-cli-voice-start-next-work-index".to_string(),
                    kind: EventKind::WorkpadIndexed,
                    actor: "test".to_string(),
                    project_id: Some(project_id()),
                    task_id: None,
                    agent_id: None,
                    session_id: None,
                    run_id: None,
                    turn_id: None,
                    item_id: Some("workpads:features:voice.md#v8".to_string()),
                    payload_json: "{}".to_string(),
                    idempotency_key: None,
                    redaction_state: RedactionState::Safe,
                },
                &[
                    ProjectionRecord::WorkpadFile(WorkpadFileProjection {
                        path: "workpads/features/voice.md".to_string(),
                        project_id: project_id(),
                        content_hash: "hash-voice-workpad".to_string(),
                        headings: "V8 - Start Next Work Conversation".to_string(),
                        objective: Some("Voice workpad".to_string()),
                        observed_unix: 1,
                        updated_sequence: 0,
                    }),
                    ProjectionRecord::WorkpadTask(WorkpadTaskProjection {
                        workpad_task_id: "workpads:features:voice.md#v8".to_string(),
                        project_id: project_id(),
                        path: "workpads/features/voice.md".to_string(),
                        source_anchor: "v8".to_string(),
                        title: "Start Next Work Conversation".to_string(),
                        observed_status: "pending".to_string(),
                        capo_execution_status: "observed_only".to_string(),
                        observed_unix: 1,
                        updated_sequence: 0,
                    }),
                ],
            )
            .expect("append workpad file and task");
        let before_unconfirmed = state.last_sequence().expect("before unconfirmed");

        let unconfirmed = run_cli(vec![
            "voice".to_string(),
            "submit".to_string(),
            "--transcript".to_string(),
            "Start next task with fake-codex.".to_string(),
            "--voice-session".to_string(),
            "voice-session-start-next".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("voice start next requires confirmation");

        assert!(unconfirmed.contains("voice_plan=start_next_work"));
        assert!(unconfirmed.contains("confirmation_required=true"));
        assert!(unconfirmed.contains("mutation_applied=false"));
        assert!(unconfirmed.contains("permission_status=pending"));
        assert!(!unconfirmed.contains("workpad_next_started=true"));
        assert!(!unconfirmed.contains("Start next task with fake-codex"));
        assert_eq!(
            state.last_sequence().expect("after unconfirmed"),
            before_unconfirmed + 1
        );

        let confirmed = run_cli(vec![
            "voice".to_string(),
            "submit".to_string(),
            "--transcript".to_string(),
            "Start next task with fake-codex.".to_string(),
            "--voice-session".to_string(),
            "voice-session-start-next".to_string(),
            "--confirm".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("voice confirmed start next");

        assert!(confirmed.contains("voice_plan=start_next_work"));
        assert!(confirmed.contains("confirmation_required=true"));
        assert!(confirmed.contains("mutation_applied=true"));
        assert!(confirmed.contains("permission_status=decided"));
        assert!(confirmed.contains("permission_decision=allow_once"));
        assert!(confirmed.contains("controlled_agent=fake-codex"));
        assert!(confirmed.contains("workpad_next_started=true"));
        assert!(confirmed.contains("workpad_task_id=workpads:features:voice.md#v8"));
        assert!(confirmed.contains("task_id=task-workpad-workpads-features-voice-md-v8"));
        assert!(confirmed.contains("session_id=session-fake-codex"));
        assert!(confirmed.contains("spoken_next_workpad_task=none"));
        assert!(!confirmed.contains("Start next task with fake-codex"));

        let imported = state
            .workpad_task(&project_id(), "workpads:features:voice.md#v8")
            .expect("workpad task query")
            .expect("imported workpad task");
        assert_eq!(imported.capo_execution_status, "imported");
        let session = state
            .session(&SessionId::new("session-fake-codex"))
            .expect("session query")
            .expect("started session");
        assert_eq!(
            session.task_id.as_ref().map(ToString::to_string).as_deref(),
            Some("task-workpad-workpads-features-voice-md-v8")
        );
        let grants = state.capability_grants().expect("capability grants");
        assert!(grants.iter().any(|grant| {
            grant.decision_source == "user_visible_voice_confirmation"
                && grant.subject_json.contains("voice-session-start-next")
                && grant.subject_json.contains("start_next_work")
        }));
    }

    #[test]
    fn voice_review_needs_reads_review_and_outcome_state_without_mutating() {
        let state_root = temp_root("cli-voice-review-needs");
        let evidence_dir = temp_root("cli-voice-review-needs-evidence");
        seed_running_agent(&state_root, "fake-codex", "Inspect the project");

        run_cli(vec![
            "review".to_string(),
            "record".to_string(),
            "--session".to_string(),
            "session-fake-codex".to_string(),
            "--reviewer".to_string(),
            "focused-review".to_string(),
            "--kind".to_string(),
            "blocker".to_string(),
            "--summary".to_string(),
            "Dashboard must expose review blockers.".to_string(),
            "--out".to_string(),
            evidence_dir.display().to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("record review blocker");

        SqliteStateStore::open(&state_root)
            .expect("state")
            .append_event(
                NewEvent {
                    event_id: "event-cli-voice-review-needs-task-outcome".to_string(),
                    kind: EventKind::TaskOutcomeReportGenerated,
                    actor: "test".to_string(),
                    project_id: Some(project_id()),
                    task_id: Some(TaskId::new("task-inspect-the-project")),
                    agent_id: Some(AgentId::new("agent-fake-codex")),
                    session_id: Some(SessionId::new("session-fake-codex")),
                    run_id: Some(RunId::new("run-fake-codex")),
                    turn_id: None,
                    item_id: Some("task-outcome-report-voice-review-needs".to_string()),
                    payload_json: "{}".to_string(),
                    idempotency_key: None,
                    redaction_state: RedactionState::Safe,
                },
                &[ProjectionRecord::TaskOutcomeReport(
                    capo_state::TaskOutcomeReportProjection {
                        task_outcome_report_id: "task-outcome-report-voice-review-needs"
                            .to_string(),
                        project_id: project_id(),
                        task_id: TaskId::new("task-inspect-the-project"),
                        session_id: SessionId::new("session-fake-codex"),
                        run_id: RunId::new("run-fake-codex"),
                        outcome_status: "completed".to_string(),
                        started_sequence: 1,
                        completed_sequence: 12,
                        duration_sequence_span: 11,
                        action_count: 4,
                        tool_call_count: 1,
                        evidence_count: 1,
                        memory_packet_count: 1,
                        confidence: Some(78),
                        blocker: Some("Needs review follow-up".to_string()),
                        review_outcome: "reviewed_with_findings".to_string(),
                        report_artifact_id: Some(
                            "artifact-task-outcome-voice-review-needs".to_string(),
                        ),
                        updated_sequence: 0,
                    },
                )],
            )
            .expect("append task outcome report");
        let state = SqliteStateStore::open(&state_root).expect("state");
        let before_sequence = state.last_sequence().expect("before sequence");

        let output = run_cli(vec![
            "voice".to_string(),
            "submit".to_string(),
            "--transcript".to_string(),
            "What needs review?".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("voice review needs");

        assert!(output.contains("voice_plan=review_needs"));
        assert!(output.contains("mutation_applied=false"));
        assert!(output.contains("raw_transcript_retained=false"));
        assert!(output.contains("read_scope=project_review_needs"));
        assert!(output.contains("spoken_review_findings=1"));
        assert!(output.contains("spoken_open_review_findings=1"));
        assert!(output.contains("spoken_review_blockers=1"));
        assert!(output.contains("spoken_task_outcome_reports=1"));
        assert!(output.contains("spoken_reports_with_findings=1"));
        assert!(output.contains("spoken_latest_review_outcome=reviewed_with_findings"));
        assert!(output.contains("kind=blocker severity=high status=open"));
        assert!(output.contains("summary=Dashboard must expose review blockers."));
        assert!(
            output.contains("spoken_task_outcome_report=task-outcome-report-voice-review-needs")
        );
        assert!(!output.contains("What needs review?"));
        assert_eq!(
            state.last_sequence().expect("after sequence"),
            before_sequence
        );
    }

    #[test]
    fn voice_dogfood_readiness_reads_shared_query_without_mutating() {
        let state_root = temp_root("cli-voice-dogfood-readiness");
        let state = SqliteStateStore::open(&state_root).expect("state");
        let before_sequence = state.last_sequence().expect("before sequence");

        let output = run_cli(vec![
            "voice".to_string(),
            "submit".to_string(),
            "--transcript".to_string(),
            "Are we ready to dogfood?".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("voice dogfood readiness");

        assert!(output.contains("voice_plan=dogfood_readiness"));
        assert!(output.contains("mutation_applied=false"));
        assert!(output.contains("raw_transcript_retained=false"));
        assert!(output.contains("read_scope=project_dogfood_readiness"));
        assert!(output.contains("spoken_dogfood_ready=false"));
        assert!(output.contains("spoken_dogfood_status=blocked_pending_dogfood_prerequisites"));
        assert!(output.contains("spoken_blockers=real_agent_connector_not_proven,workpad_index_missing,dispatch_chain_missing"));
        assert!(output.contains("spoken_next_actions=record_clean_codex_smoke_evidence,run_workpad_index,record_or_replay_workpad_dispatch_plan"));
        assert!(!output.contains("Are we ready to dogfood?"));
        assert_eq!(
            state.last_sequence().expect("after sequence"),
            before_sequence
        );
    }

    #[test]
    fn voice_dispatch_status_reads_shared_query_without_mutating() {
        let state_root = temp_root("cli-voice-dispatch-status");
        let workspace = temp_root("cli-voice-dispatch-status-workspace");
        let artifacts = temp_root("cli-voice-dispatch-status-artifacts");
        run_cli(vec![
            "adapter".to_string(),
            "plan-launch".to_string(),
            "--adapter".to_string(),
            "codex".to_string(),
            "--agent".to_string(),
            "codex-worker".to_string(),
            "--goal".to_string(),
            "Do not render this voice dispatch prompt.".to_string(),
            "--workspace".to_string(),
            workspace.display().to_string(),
            "--artifacts".to_string(),
            artifacts.display().to_string(),
            "--record".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("record dispatch plan");
        let state = SqliteStateStore::open(&state_root).expect("state");
        let dispatch_plan_id = state
            .adapter_dispatch_plans(&project_id())
            .expect("dispatch plans")[0]
            .dispatch_plan_id
            .clone();
        let before_sequence = state.last_sequence().expect("before sequence");

        let output = run_cli(vec![
            "voice".to_string(),
            "submit".to_string(),
            "--transcript".to_string(),
            format!("What is dispatch status for {dispatch_plan_id}?"),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("voice dispatch status");

        assert!(output.contains("voice_plan=dispatch_status"));
        assert!(output.contains("mutation_applied=false"));
        assert!(output.contains("raw_transcript_retained=false"));
        assert!(output.contains("read_scope=project_dispatch_status"));
        assert!(output.contains(&format!("spoken_dispatch_plan={dispatch_plan_id}")));
        assert!(output.contains("spoken_adapter=codex_exec"));
        assert!(output.contains("spoken_provider_kind=codex_subscription"));
        assert!(output.contains("spoken_credential_scope=user_local_subscription"));
        assert!(output.contains("spoken_provider_cli_executed=false"));
        assert!(output.contains("spoken_dogfood_gate=blocked_pending_real_smoke"));
        assert!(output.contains("spoken_latest_gate_status=missing"));
        assert!(output.contains("spoken_latest_dispatch_replay=none"));
        assert!(output.contains("spoken_latest_execution_status=missing"));
        assert!(output.contains("spoken_next_action=record_clean_real_smoke_evidence"));
        assert!(!output.contains("What is dispatch status"));
        assert!(!output.contains("Do not render this voice dispatch prompt"));
        assert_eq!(
            state.last_sequence().expect("after sequence"),
            before_sequence
        );

        let latest_output = run_cli(vec![
            "voice".to_string(),
            "submit".to_string(),
            "--transcript".to_string(),
            "What is the latest dispatch status?".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("voice latest dispatch status");

        assert!(latest_output.contains("voice_plan=dispatch_status"));
        assert!(latest_output.contains("mutation_applied=false"));
        assert!(latest_output.contains("read_scope=project_latest_dispatch_status"));
        assert!(latest_output.contains(&format!("spoken_dispatch_plan={dispatch_plan_id}")));
        assert!(latest_output.contains("spoken_agent=codex-worker"));
        assert!(latest_output.contains("spoken_next_action=record_clean_real_smoke_evidence"));
        assert!(!latest_output.contains("What is the latest dispatch status"));
        assert!(!latest_output.contains("Do not render this voice dispatch prompt"));

        let latest_agent_output = run_cli(vec![
            "voice".to_string(),
            "submit".to_string(),
            "--transcript".to_string(),
            "What is the latest dispatch status for codex-worker?".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("voice latest dispatch status for agent");

        assert!(latest_agent_output.contains("read_scope=project_latest_dispatch_status"));
        assert!(latest_agent_output.contains(&format!("spoken_dispatch_plan={dispatch_plan_id}")));
        assert!(latest_agent_output.contains("spoken_agent=codex-worker"));
        assert!(!latest_agent_output.contains("What is the latest dispatch status"));
        assert_eq!(
            state.last_sequence().expect("after latest sequence"),
            before_sequence
        );
    }

    #[test]
    fn voice_redirect_routes_through_controller_and_preserves_transient_transcript() {
        let state_root = temp_root("cli-voice-redirect");
        seed_running_agent(&state_root, "fake-reviewer", "Review the status summary");

        let output = run_cli(vec![
            "voice".to_string(),
            "submit".to_string(),
            "--transcript".to_string(),
            "Steer fake-reviewer to focus only on dogfood blockers.".to_string(),
            "--voice-session".to_string(),
            "voice-session-test".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("voice redirect");

        assert!(output.contains("voice_plan=redirect_session"));
        assert!(output.contains("command_id=cmd-voice-redirect-voice-session-test"));
        assert!(output.contains("mutation_applied=true"));
        assert!(output.contains("read_scope=session_for_agent"));
        assert!(output.contains("spoken_agent=fake-reviewer agent_status=running"));
        assert!(output.contains("current_goal=focus only on dogfood blockers"));
        assert!(!output.contains("Steer fake-reviewer"));

        let status = run_cli(vec![
            "session".to_string(),
            "status".to_string(),
            "--agent".to_string(),
            "fake-reviewer".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("status after voice redirect");
        assert!(status.contains("current_goal=focus only on dogfood blockers"));
        assert!(status.contains("kind=session.redirected"));
    }

    #[test]
    fn voice_unknown_does_not_mutate_and_unconfirmed_stop_only_queues_approval() {
        let state_root = temp_root("cli-voice-no-mutation");
        seed_running_agent(&state_root, "fake-codex", "Inspect the project");
        let state = SqliteStateStore::open(&state_root).expect("state");
        let before_unknown = state.last_sequence().expect("before unknown");

        let unknown = run_cli(vec![
            "voice".to_string(),
            "submit".to_string(),
            "--transcript".to_string(),
            "Maybe later, never mind".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("voice unknown");
        assert!(unknown.contains("voice_plan=unknown"));
        assert!(unknown.contains("command_id=none"));
        assert!(unknown.contains("mutation_applied=false"));
        assert_eq!(
            state.last_sequence().expect("after unknown"),
            before_unknown
        );

        let before_stop = state.last_sequence().expect("before stop");
        let stop = run_cli(vec![
            "voice".to_string(),
            "submit".to_string(),
            "--transcript".to_string(),
            "Stop fake-codex because smoke is done".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("voice stop needs confirmation");
        assert!(stop.contains("voice_plan=stop_session"));
        assert!(stop.contains("confirmation_required=true"));
        assert!(stop.contains("mutation_applied=false"));
        assert!(stop.contains("permission_status=pending"));
        assert!(stop.contains("permission_scope=[\"voice:approve:privileged\"]"));
        assert_eq!(state.last_sequence().expect("after stop"), before_stop + 1);
        let approvals = state
            .permission_approvals(&project_id())
            .expect("voice approvals");
        let approval = approvals
            .iter()
            .find(|approval| approval.requested_by == "voice:local-user")
            .expect("voice approval");
        assert_eq!(approval.status, "pending");
        assert_eq!(
            approval
                .session_id
                .as_ref()
                .map(ToString::to_string)
                .as_deref(),
            Some("session-fake-codex")
        );
        assert_eq!(
            approval.reason,
            "visible confirmation required for stop_session"
        );
        assert!(!approval.reason.contains("smoke is done"));

        let status = run_cli(vec![
            "session".to_string(),
            "status".to_string(),
            "--agent".to_string(),
            "fake-codex".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("status after unconfirmed stop");
        assert!(status.contains("status=active"));
        assert!(status.contains("run_status=running"));
    }

    #[test]
    fn voice_confirmed_stop_audits_decision_before_controller_mutation() {
        let state_root = temp_root("cli-voice-confirmed-stop");
        seed_running_agent(&state_root, "fake-codex", "Inspect the project");

        let stop = run_cli(vec![
            "voice".to_string(),
            "submit".to_string(),
            "--transcript".to_string(),
            "Stop fake-codex because smoke is done".to_string(),
            "--voice-session".to_string(),
            "voice-session-confirmed".to_string(),
            "--confirm".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("voice confirmed stop");

        assert!(stop.contains("voice_plan=stop_session"));
        assert!(stop.contains("confirmation_required=true"));
        assert!(stop.contains("mutation_applied=true"));
        assert!(stop.contains("permission_status=decided"));
        assert!(stop.contains("permission_decision=allow_once"));
        assert!(stop.contains("spoken_agent=fake-codex agent_status=available"));
        assert!(stop.contains("session_status=completed"));
        assert!(!stop.contains("Stop fake-codex"));

        let state = SqliteStateStore::open(&state_root).expect("state");
        let approvals = state
            .permission_approvals(&project_id())
            .expect("voice approvals");
        assert_eq!(approvals.len(), 1);
        let approval = &approvals[0];
        assert_eq!(approval.status, "decided");
        assert_eq!(approval.decision.as_deref(), Some("allow_once"));
        assert_eq!(approval.capability_profile_id, "voice-control");
        assert_eq!(approval.scope_json, "[\"voice:approve:privileged\"]");
        assert_eq!(approval.requested_by, "voice:local-user");
        assert!(approval.capability_grant_id.is_some());
        let grants = state.capability_grants().expect("capability grants");
        assert!(grants.iter().any(|grant| {
            grant.capability_grant_id == approval.capability_grant_id.clone().unwrap()
                && grant.decision_source == "user_visible_voice_confirmation"
                && grant.persistence == "once"
                && grant.subject_json.contains("voice-session-confirmed")
        }));
        let events = state
            .recent_events_for_session(&SessionId::new("session-fake-codex"), 10)
            .expect("recent events");
        assert!(
            events
                .iter()
                .any(|event| event.kind == "permission.approval_queued")
        );
        assert!(
            events
                .iter()
                .any(|event| event.kind == "permission.decided")
        );
        assert!(events.iter().any(|event| event.kind == "session.stopped"));
        for event in events {
            assert!(!event.payload_json.contains("Stop fake-codex"));
            assert!(!event.payload_json.contains("smoke is done"));
        }
    }

    #[test]
    fn voice_confirmed_interrupt_audits_decision_before_controller_mutation() {
        let state_root = temp_root("cli-voice-confirmed-interrupt");
        seed_running_agent(&state_root, "fake-codex", "Inspect the project");

        let interrupted = run_cli(vec![
            "voice".to_string(),
            "submit".to_string(),
            "--transcript".to_string(),
            "Interrupt fake-codex because output is stale".to_string(),
            "--confirm".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("voice confirmed interrupt");

        assert!(interrupted.contains("voice_plan=interrupt_session"));
        assert!(interrupted.contains("mutation_applied=true"));
        assert!(interrupted.contains("permission_status=decided"));
        assert!(interrupted.contains("permission_decision=allow_once"));
        assert!(interrupted.contains("controlled_session=session-fake-codex"));
        assert!(interrupted.contains("session_status=canceled"));
        assert!(interrupted.contains("run_status=stopping"));
        assert!(!interrupted.contains("Interrupt fake-codex"));

        let state = SqliteStateStore::open(&state_root).expect("state");
        let approvals = state
            .permission_approvals(&project_id())
            .expect("voice approvals");
        assert_eq!(approvals.len(), 1);
        assert_eq!(
            approvals[0].reason,
            "visible confirmation required for interrupt_session"
        );
        let events = state
            .recent_events_for_session(&SessionId::new("session-fake-codex"), 10)
            .expect("recent events");
        assert!(
            events
                .iter()
                .any(|event| event.kind == "permission.approval_queued")
        );
        assert!(
            events
                .iter()
                .any(|event| event.kind == "permission.decided")
        );
        assert!(
            events
                .iter()
                .any(|event| event.kind == "session.interrupted")
        );
        for event in events {
            assert!(!event.payload_json.contains("Interrupt fake-codex"));
            assert!(!event.payload_json.contains("output is stale"));
        }
    }

    #[test]
    fn voice_reviewed_redacted_summary_ingests_memory_without_raw_transcript() {
        let state_root = temp_root("cli-voice-memory");
        seed_running_agent(&state_root, "fake-codex", "Inspect the project");
        let raw_phrase = "raw-private-voice-token";
        let redacted_summary = "User asked to stop fake-codex after a redacted reason.";

        let output = run_cli(vec![
            "voice".to_string(),
            "submit".to_string(),
            "--transcript".to_string(),
            format!("Stop fake-codex because {raw_phrase}"),
            "--redacted-summary".to_string(),
            redacted_summary.to_string(),
            "--reviewed-summary".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("voice summary memory");

        assert!(output.contains("voice_plan=stop_session"));
        assert!(output.contains("memory_ingestion=reviewed_redacted_summary_only"));
        assert!(output.contains("memory_review_state=reviewed"));
        assert!(output.contains("memory_redaction_state=redacted"));
        assert!(!output.contains(raw_phrase));

        let state = SqliteStateStore::open(&state_root).expect("state");
        let records = state
            .memory_records_for_project(&project_id())
            .expect("memory records");
        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert_eq!(record.review_state, "reviewed");
        assert_eq!(record.redaction_state, "redacted");
        assert_eq!(record.record_kind, "summary");
        assert_eq!(record.body, redacted_summary);
        assert!(!record.body.contains(raw_phrase));
        let sources = state
            .memory_sources_for_record(&record.memory_record_id)
            .expect("memory sources");
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].source_kind, "event");
        assert_eq!(
            sources[0].source_anchor.as_deref(),
            Some("voice:redacted-summary")
        );
        assert!(sources[0].source_content_hash.is_some());
        let eligible = state
            .packet_eligible_memory_records(&project_id())
            .expect("packet eligible records");
        assert_eq!(eligible.len(), 1);
        assert_eq!(eligible[0].memory_record_id, record.memory_record_id);
        assert_text_absent_in_tree(&state_root, raw_phrase);
    }

    #[test]
    fn voice_redacted_summary_requires_review_before_memory_ingestion() {
        let state_root = temp_root("cli-voice-memory-review-required");
        seed_running_agent(&state_root, "fake-codex", "Inspect the project");

        let error = run_cli(vec![
            "voice".to_string(),
            "submit".to_string(),
            "--transcript".to_string(),
            "What is fake-codex doing?".to_string(),
            "--redacted-summary".to_string(),
            "User asked for a redacted status summary.".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .unwrap_err();

        assert!(error.contains("--redacted-summary requires --reviewed-summary"));
        let state = SqliteStateStore::open(&state_root).expect("state");
        let records = state
            .memory_records_for_project(&project_id())
            .expect("memory records");
        assert!(records.is_empty());
    }

    #[test]
    fn evidence_export_handles_completed_runs_and_refuses_foreign_files() {
        let state_root = temp_root("cli-completed-state");
        let evidence_dir = temp_root("cli-completed-evidence");

        run_cli(vec![
            "agent".to_string(),
            "register".to_string(),
            "--name".to_string(),
            "fake-reviewer".to_string(),
            "--adapter".to_string(),
            "fake".to_string(),
            "--runtime".to_string(),
            "fake".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .unwrap();
        run_cli(vec![
            "task".to_string(),
            "send".to_string(),
            "--agent".to_string(),
            "fake-reviewer".to_string(),
            "--goal".to_string(),
            "Review the status summary for blockers".to_string(),
            "--scenario".to_string(),
            "summary-review".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .unwrap();
        run_cli(vec![
            "session".to_string(),
            "stop".to_string(),
            "--agent".to_string(),
            "fake-reviewer".to_string(),
            "--reason".to_string(),
            "completed smoke".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .unwrap();

        run_cli(vec![
            "evidence".to_string(),
            "export".to_string(),
            "--session".to_string(),
            "session-fake-reviewer".to_string(),
            "--out".to_string(),
            evidence_dir.display().to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .unwrap();
        let evidence_path = evidence_dir.join("session-fake-reviewer.md");
        let exported = fs::read_to_string(&evidence_path).expect("read completed evidence");
        assert!(exported.contains("- Session status: `completed`"));
        assert!(exported.contains("- Run status: `exited`"));
        assert!(exported.contains("session.stopped"));

        fs::write(&evidence_path, "# user-authored workpad\n").expect("replace with foreign file");
        let error = run_cli(vec![
            "evidence".to_string(),
            "export".to_string(),
            "--session".to_string(),
            "session-fake-reviewer".to_string(),
            "--out".to_string(),
            evidence_dir.display().to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .unwrap_err();
        assert!(error.contains("refusing to overwrite non-Capo evidence file"));
    }

    #[test]
    fn prototype_e2e_smoke_tracks_two_agents_recovers_and_exports_evidence() {
        let state_root = temp_root("cli-e2e-state");
        let evidence_dir = temp_root("cli-e2e-evidence");
        let mut transcript = String::new();

        let mut run = |args: Vec<&str>| {
            let mut owned = args
                .into_iter()
                .map(str::to_string)
                .collect::<Vec<String>>();
            owned.push("--state".to_string());
            owned.push(state_root.display().to_string());
            let output = run_cli(owned).expect("run smoke command");
            transcript.push_str(&output);
            output
        };

        let initialized = run(vec!["init"]);
        assert!(initialized.contains("initialized=true"));

        let codex = run(vec![
            "agent",
            "spawn",
            "--name",
            "fake-codex",
            "--adapter",
            "fake",
            "--runtime",
            "fake",
        ]);
        assert!(codex.contains("agent_spawned=true"));
        let reviewer = run(vec![
            "agent",
            "register",
            "--name",
            "fake-reviewer",
            "--adapter",
            "fake",
            "--runtime",
            "fake",
        ]);
        assert!(reviewer.contains("agent_registered=true"));

        let codex_send = run(vec![
            "task",
            "send",
            "--agent",
            "fake-codex",
            "--goal",
            "Inspect the project and write a short status summary",
            "--scenario",
            "tool-memory",
        ]);
        assert!(codex_send.contains("session_id=session-fake-codex"));
        let reviewer_send = run(vec![
            "task",
            "send",
            "--agent",
            "fake-reviewer",
            "--goal",
            "Review the status summary for blockers",
            "--scenario",
            "summary-review",
        ]);
        assert!(reviewer_send.contains("session_id=session-fake-reviewer"));

        let agents = run(vec!["agent", "list"]);
        assert!(agents.contains("active_agents=2"));
        assert!(agents.contains("agent=fake-codex status=running"));
        assert!(agents.contains("agent=fake-reviewer status=running"));
        let dashboard = run(vec!["dashboard"]);
        assert!(dashboard.contains("view=dashboard"));
        assert!(dashboard.contains("agents=2"));
        assert!(dashboard.contains("active_sessions=2"));
        assert!(dashboard.contains("agent=fake-codex agent_status=running"));
        assert!(dashboard.contains("session=session-fake-codex session_status=active"));
        assert!(dashboard.contains("goal=Inspect the project"));
        assert!(dashboard.contains("blocker=none"));
        assert!(dashboard.contains("evidence_refs=evidence-fake-codex"));
        assert!(dashboard.contains("tool_calls=1"));
        assert!(dashboard.contains("tool_call=tool-fake-codex tool=capo.session_summary"));
        assert!(dashboard.contains("memory_packet_refs=1"));
        assert!(dashboard.contains("memory_packet=packet-fake-codex purpose=turn_context"));
        assert!(dashboard.contains("kind=tool.result_delivered"));
        assert!(dashboard.contains("agent=fake-reviewer agent_status=running"));
        let session_dashboard = run(vec!["dashboard", "--session", "session-fake-codex"]);
        assert!(session_dashboard.contains("agents=1"));
        assert!(session_dashboard.contains("agent=fake-codex agent_status=running"));
        assert!(!session_dashboard.contains("agent=fake-reviewer agent_status=running"));
        let running_dashboard = run(vec!["dashboard", "--status", "running"]);
        assert!(running_dashboard.contains("agents=2"));
        let missing_dashboard = run(vec!["dashboard", "--status", "waiting_for_input"]);
        assert!(missing_dashboard.contains("agents=0"));
        assert!(missing_dashboard.contains("active_sessions=0"));
        let other_project_dashboard = run(vec!["dashboard", "--project", "project-other"]);
        assert!(other_project_dashboard.contains("agents=0"));
        assert!(other_project_dashboard.contains("active_sessions=0"));

        let codex_status = run(vec!["session", "status", "--agent", "fake-codex"]);
        assert!(codex_status.contains("current_goal=Inspect the project"));
        assert!(codex_status.contains("kind=permission.decided"));
        assert!(codex_status.contains("kind=capability.grant_used"));
        assert!(codex_status.contains("kind=tool.result_delivered"));
        assert!(codex_status.contains("kind=memory.packet_built"));
        assert!(codex_status.contains("evidence_refs=evidence-fake-codex"));

        let redirect = run(vec![
            "session",
            "redirect",
            "--agent",
            "fake-reviewer",
            "--goal",
            "Focus only on dogfood blockers",
        ]);
        assert!(redirect.contains("redirected=true"));
        assert!(redirect.contains("current_goal=Focus only on dogfood blockers"));
        let reviewer_status = run(vec!["session", "status", "--agent", "fake-reviewer"]);
        assert!(reviewer_status.contains("current_goal=Focus only on dogfood blockers"));
        assert!(reviewer_status.contains("kind=session.redirected"));
        let second_redirect = run(vec![
            "session",
            "redirect",
            "--agent",
            "fake-reviewer",
            "--goal",
            "Focus only on evidence export blockers",
        ]);
        assert!(second_redirect.contains("redirected=true"));
        assert!(second_redirect.contains("current_goal=Focus only on evidence export blockers"));
        let redirected_dashboard = run(vec!["dashboard"]);
        assert!(redirected_dashboard.contains("Focus only on evidence export blockers"));
        assert!(redirected_dashboard.contains("kind=session.redirected"));

        let interrupted = run(vec![
            "session",
            "interrupt",
            "--agent",
            "fake-codex",
            "--reason",
            "smoke interrupt",
        ]);
        assert!(interrupted.contains("status=canceled"));
        let stopped = run(vec![
            "session",
            "stop",
            "--agent",
            "fake-reviewer",
            "--reason",
            "smoke stop",
        ]);
        assert!(stopped.contains("status=completed"));

        let recovered = run(vec!["recover"]);
        assert!(recovered.contains("recovered=true"));
        assert!(recovered.contains("recovered_run_count=1"));
        let recovered_again = run(vec!["recover"]);
        assert!(recovered_again.contains("recovered=true"));
        assert!(recovered_again.contains("recovered_run_count=0"));

        let export_codex = run_cli(vec![
            "evidence".to_string(),
            "export".to_string(),
            "--session".to_string(),
            "session-fake-codex".to_string(),
            "--out".to_string(),
            evidence_dir.display().to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("export codex evidence");
        transcript.push_str(&export_codex);
        let export_reviewer = run_cli(vec![
            "evidence".to_string(),
            "export".to_string(),
            "--session".to_string(),
            "session-fake-reviewer".to_string(),
            "--out".to_string(),
            evidence_dir.display().to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("export reviewer evidence");
        transcript.push_str(&export_reviewer);

        let codex_evidence =
            fs::read_to_string(evidence_dir.join("session-fake-codex.md")).expect("codex evidence");
        let reviewer_evidence = fs::read_to_string(evidence_dir.join("session-fake-reviewer.md"))
            .expect("reviewer evidence");
        assert!(codex_evidence.contains("- Session status: `canceled`"));
        assert!(codex_evidence.contains("- Run status: `exited_unknown`"));
        assert!(codex_evidence.contains("tool.result_delivered"));
        assert!(codex_evidence.contains("artifact=`artifact-memory-packet-packet-fake-codex`"));
        assert!(reviewer_evidence.contains("- Session status: `completed`"));
        assert!(reviewer_evidence.contains("Focus only on evidence export blockers"));
        assert!(reviewer_evidence.contains("session.redirected"));
        assert!(reviewer_evidence.contains("session.stopped"));

        let reopened = SqliteStateStore::open(&state_root).expect("restart state");
        assert_eq!(
            reopened
                .session(&SessionId::new("session-fake-codex"))
                .expect("read codex session")
                .expect("codex session")
                .status,
            "canceled"
        );
        assert_eq!(
            reopened
                .run_for_session(&SessionId::new("session-fake-codex"))
                .expect("read codex run")
                .expect("codex run")
                .status,
            "exited_unknown"
        );
        assert_eq!(
            reopened
                .session(&SessionId::new("session-fake-reviewer"))
                .expect("read reviewer session")
                .expect("reviewer session")
                .status,
            "completed"
        );
        assert_eq!(reopened.agents().expect("read agents").len(), 2);
        assert_eq!(
            reopened
                .evidence_for_session(&SessionId::new("session-fake-codex"))
                .expect("codex evidence")
                .len(),
            1
        );
        assert_eq!(
            reopened
                .evidence_for_session(&SessionId::new("session-fake-reviewer"))
                .expect("reviewer evidence")
                .len(),
            1
        );
        assert_eq!(
            reopened
                .tool_calls_for_session(&SessionId::new("session-fake-codex"))
                .expect("codex tool calls")
                .len(),
            1
        );
        assert_eq!(
            reopened
                .memory_packets_for_session(&SessionId::new("session-fake-codex"))
                .expect("codex memory packets")
                .len(),
            1
        );
        assert_eq!(
            reopened
                .task(&capo_core::TaskId::new(
                    "task-inspect-the-project-and-write-a-short-status-summary"
                ))
                .expect("read codex task")
                .expect("codex task")
                .evidence_id
                .as_ref()
                .map(ToString::to_string),
            Some("evidence-fake-codex".to_string())
        );
        assert_eq!(
            reopened
                .task(&capo_core::TaskId::new(
                    "task-review-the-status-summary-for-blockers"
                ))
                .expect("read reviewer task")
                .expect("reviewer task")
                .evidence_id
                .as_ref()
                .map(ToString::to_string),
            Some("evidence-fake-reviewer".to_string())
        );

        assert_no_sensitive_markers(&transcript);
        assert_no_sensitive_markers(&codex_evidence);
        assert_no_sensitive_markers(&reviewer_evidence);
        assert_no_sensitive_markers_in_tree(&state_root);
        assert_no_sensitive_markers_in_tree(&evidence_dir);
    }

    fn assert_no_sensitive_markers(contents: &str) {
        for marker in [
            "OPENAI_API_KEY",
            "ANTHROPIC_API_KEY",
            "Authorization:",
            "Cookie:",
            "Set-Cookie:",
            "session_token",
            "access_token",
            "refresh_token",
            "oauth",
            "api_key",
            "sk-proj-",
            "sk-ant-",
            "sk-live-",
            "sk_test_",
        ] {
            assert!(
                !contents
                    .to_ascii_lowercase()
                    .contains(&marker.to_ascii_lowercase()),
                "sensitive marker leaked: {marker}"
            );
        }
    }

    fn assert_no_sensitive_markers_in_tree(root: &Path) {
        if !root.exists() {
            return;
        }
        let mut stack = vec![root.to_path_buf()];
        while let Some(path) = stack.pop() {
            if path.is_dir() {
                for entry in fs::read_dir(&path).expect("read scan dir") {
                    stack.push(entry.expect("scan dir entry").path());
                }
            } else if path.is_file() {
                let bytes = fs::read(&path).expect("read scan file");
                let contents = String::from_utf8_lossy(&bytes);
                assert_no_sensitive_markers(&contents);
            }
        }
    }

    fn assert_text_absent_in_tree(root: &Path, needle: &str) {
        if !root.exists() {
            return;
        }
        let mut stack = vec![root.to_path_buf()];
        while let Some(path) = stack.pop() {
            if path.is_dir() {
                for entry in fs::read_dir(&path).expect("read scan dir") {
                    stack.push(entry.expect("scan dir entry").path());
                }
            } else if path.is_file() {
                let bytes = fs::read(&path).expect("read scan file");
                let contents = String::from_utf8_lossy(&bytes);
                assert!(
                    !contents.contains(needle),
                    "unexpected raw text in {}",
                    path.display()
                );
            }
        }
    }

    fn seed_running_agent(state_root: &Path, agent: &str, goal: &str) {
        run_cli(vec![
            "agent".to_string(),
            "register".to_string(),
            "--name".to_string(),
            agent.to_string(),
            "--adapter".to_string(),
            "fake".to_string(),
            "--runtime".to_string(),
            "fake".to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("register agent");
        run_cli(vec![
            "task".to_string(),
            "send".to_string(),
            "--agent".to_string(),
            agent.to_string(),
            "--goal".to_string(),
            goal.to_string(),
            "--state".to_string(),
            state_root.display().to_string(),
        ])
        .expect("send task");
    }

    fn temp_root(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!("capo-{name}-{nanos}"))
    }

    fn output_value(output: &str, key: &str) -> String {
        let prefix = format!("{key}=");
        output
            .lines()
            .find_map(|line| line.strip_prefix(&prefix))
            .unwrap_or_else(|| panic!("missing output key: {key}"))
            .to_string()
    }
}
