use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use capo_controller::FakeBoundaryController;
use capo_core::{
    AgentId, CommandEnvelope, CommandId, CommandIntent, CommandTarget, InputOrigin, ProjectId,
    SessionId, TaskId, ToolCallId,
};
use capo_eval::TaskOutcomeReport;
use capo_query::{ProjectDashboard, ProjectDashboardQuery, project_dashboard};
use capo_state::{
    ArtifactRecord, CapabilityGrantProjection, EventKind, EventRecord, EvidenceProjection,
    MemoryPacketProjection, NewEvent, PermissionApprovalProjection, ProjectionRecord,
    RedactionState, ReviewFindingProjection, RunProjection, SessionProjection, SqliteStateStore,
    ToolCallProjection, WorkpadFileProjection, WorkpadIndexResetProjection, WorkpadTaskProjection,
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
  capo dashboard [--project PROJECT_ID] [--session SESSION_ID] [--status STATUS] [--state PATH]
  capo agent register --name NAME --adapter fake --runtime fake [--state PATH]
  capo agent spawn --name NAME --adapter fake --runtime fake [--state PATH]
  capo agent list [--state PATH]
  capo task send --agent NAME --goal GOAL [--scenario NAME] [--state PATH]
  capo session status --agent NAME [--state PATH]
  capo session redirect --agent NAME --goal GOAL [--state PATH]
  capo session interrupt --agent NAME --reason REASON [--state PATH]
  capo session stop --agent NAME --reason REASON [--state PATH]
  capo recover [--state PATH]
  capo permission request --approval APPROVAL_ID --scope-json JSON --reason REASON [--profile PROFILE] [--session SESSION_ID] [--tool-call TOOL_CALL_ID] [--subject-json JSON] [--requested-by ACTOR] [--state PATH]
  capo permission list [--state PATH]
  capo permission decide --approval APPROVAL_ID --decision allow_once|allow_always|reject_once|reject_always [--state PATH]
  capo workpad index --root PATH [--state PATH]
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
        [area, command, rest @ ..] if area == "workpad" && command == "index" => {
            index_workpads(&parsed, rest)
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
    Ok(query)
}

fn render_dashboard(command: &CommandEnvelope, dashboard: &ProjectDashboard) -> String {
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

#[allow(dead_code)]
fn path_exists(path: &Path) -> bool {
    path.exists()
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn help_mentions_command_envelopes_and_no_credentials() {
        assert!(HELP.contains("command envelopes"));
        assert!(HELP.contains("does not read provider credentials"));
        assert!(HELP.contains("workpad index"));
        assert!(HELP.contains("workpad propose"));
        assert!(HELP.contains("workpad apply"));
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

    fn temp_root(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!("capo-{name}-{nanos}"))
    }
}
