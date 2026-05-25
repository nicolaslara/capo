use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use capo_controller::FakeBoundaryController;
use capo_core::{
    AgentId, CommandEnvelope, CommandId, CommandIntent, CommandTarget, InputOrigin, ProjectId,
    SessionId,
};
use capo_state::{EventRecord, EvidenceProjection, SessionProjection, SqliteStateStore};

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
        "recovered=true\nrecovery_attempt_id={}\nstarted_sequence={}\ncompleted_sequence={}\nwatermark={}\ncommand_id={}\n",
        report.recovery_attempt_id,
        report.started_sequence,
        report.completed_sequence,
        report
            .watermark
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_string()),
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
    fs::create_dir_all(&out).map_err(|error| error.to_string())?;
    let path = out.join(format!("{session_id}.md"));
    fs::write(&path, render_evidence(&session, &evidence, &events))
        .map_err(|error| error.to_string())?;
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
    evidence: &[EvidenceProjection],
    events: &[EventRecord],
) -> String {
    let mut markdown = format!(
        "# Capo Evidence - {}\n\n- Session: `{}`\n- Status: `{}`\n- Current goal: {}\n- Latest summary: {}\n- Confidence: `{}`\n\n## Evidence Refs\n\n",
        session.title,
        session.session_id,
        session.status,
        session.current_goal,
        session.latest_summary.as_deref().unwrap_or("none"),
        session
            .latest_confidence
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_string())
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
    markdown.push_str("\n## Recent Events\n\n");
    for event in events {
        markdown.push_str(&format!("- `{}` `{}`\n", event.sequence, event.kind));
    }
    markdown
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
        assert!(evidence_dir.join("session-fake-codex.md").exists());
    }

    fn temp_root(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!("capo-{name}-{nanos}"))
    }
}
