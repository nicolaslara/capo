use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use capo_controller::FakeBoundaryController;
use capo_core::{
    AgentId, CommandEnvelope, CommandId, CommandIntent, CommandTarget, InputOrigin, ProjectId,
    SessionId,
};
use capo_state::{
    EventRecord, EvidenceProjection, MemoryPacketProjection, RunProjection, SessionProjection,
    SqliteStateStore, ToolCallProjection,
};

const DEFAULT_STATE_ROOT: &str = ".capo-dev";
const DEFAULT_PROJECT_ID: &str = "project-capo";

const HELP: &str = "\
Capo - local controller for coding-agent sessions

Usage:
  capo --help
  capo version
  capo init [--state PATH]
  capo agent register --name NAME --adapter fake --runtime fake [--state PATH]
  capo agent spawn --name NAME --adapter fake --runtime fake [--state PATH]
  capo agent list [--state PATH]
  capo task send --agent NAME --goal GOAL [--scenario NAME] [--state PATH]
  capo session status --agent NAME [--state PATH]
  capo session redirect --agent NAME --goal GOAL [--state PATH]
  capo session interrupt --agent NAME --reason REASON [--state PATH]
  capo session stop --agent NAME --reason REASON [--state PATH]
  capo recover [--state PATH]
  capo evidence export --session SESSION_ID --out DIR [--state PATH]

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
        [area, command, rest @ ..] if area == "evidence" && command == "export" => {
            export_evidence(&parsed, rest)
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

fn require_fake_arg(args: &[String], key: &str) -> Result<(), String> {
    match optional_arg(args, key).as_deref() {
        None | Some("fake") => Ok(()),
        Some(other) => Err(format!("{key} only supports `fake` in P4, got `{other}`")),
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
