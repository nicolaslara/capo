use capo_core::{AgentId, CommandIntent, CommandTarget};
use capo_state::{EvidenceProjection, ToolCallProjection, ToolObservationProjection};

use crate::cli_surface::{ParsedArgs, optional_arg, required_arg};
use crate::{controller, debug_error, envelope, project_id, state};

pub(crate) fn init(parsed: &ParsedArgs) -> Result<String, String> {
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

pub(crate) fn register_agent(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
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

pub(crate) fn list_agents(parsed: &ParsedArgs) -> Result<String, String> {
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

pub(crate) fn send_task(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
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

pub(crate) fn session_status(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
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
    let state = state(parsed)?;
    let evidence = state
        .evidence_for_session(&observation.session.session_id)
        .map_err(debug_error)?;
    let tool_calls = state
        .tool_calls_for_session(&observation.session.session_id)
        .map_err(debug_error)?;
    let tool_observations = state
        .tool_observations_for_session(&observation.session.session_id)
        .map_err(debug_error)?;
    Ok(render_status(
        &command,
        &observation,
        &evidence,
        &tool_calls,
        &tool_observations,
    ))
}

pub(crate) fn redirect_session(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
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

pub(crate) fn interrupt_session(
    parsed: &ParsedArgs,
    args: &[String],
    action: &str,
) -> Result<String, String> {
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

pub(crate) fn recover(parsed: &ParsedArgs) -> Result<String, String> {
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

fn require_fake_arg(args: &[String], key: &str) -> Result<(), String> {
    match optional_arg(args, key).as_deref() {
        None | Some("fake") => Ok(()),
        Some(other) => Err(format!("{key} only supports `fake` in P4, got `{other}`")),
    }
}

fn render_status(
    command: &capo_core::CommandEnvelope,
    observation: &capo_controller::FakeReadModelObservation,
    evidence: &[EvidenceProjection],
    tool_calls: &[ToolCallProjection],
    tool_observations: &[ToolObservationProjection],
) -> String {
    let mut output = format!(
        "command_id={}\nagent={} agent_status={}\nsession_id={} session_status={}\nrun_id={} run_status={}\ncurrent_goal={}\nlatest_summary={}\nconfidence={}\nblocker={}\nevidence_refs={}\ntool_calls={}\ntool_observations={}\nrecent_events={}\n",
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
        tool_calls.len(),
        tool_observations.len(),
        observation.recent_events.len()
    );
    for tool_call in tool_calls {
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
    for observation in tool_observations {
        output.push_str(&format!(
            "tool_observation={} tool={} source={} observed_status={} instrumentation={} confidence={} external_ref={} artifact={} raw_event_hash={}\n",
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
    for event in &observation.recent_events {
        output.push_str(&format!("event={} kind={}\n", event.sequence, event.kind));
    }
    output
}
