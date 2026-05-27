use capo_server::{
    AgentSummary, CapoServer, ServerCommand, ServerRequest, ServerResponse, ServerResponsePayload,
};

use crate::cli_surface::{ParsedArgs, optional_arg, required_arg};
use crate::{debug_error, project_id, stable_cli_hash};

pub(crate) fn server_agent_register(
    parsed: &ParsedArgs,
    args: &[String],
) -> Result<String, String> {
    require_fake_arg(args, "--adapter")?;
    require_fake_arg(args, "--runtime")?;
    let name = required_arg(args, "--name")?;
    let response = server(parsed)?
        .handle(request(
            args,
            "server-agent-register",
            ServerCommand::RegisterAgent { name },
        )?)
        .map_err(debug_error)?;
    let header = render_response_header(&response);
    let ServerResponsePayload::AgentRegistered(agent) = response.payload else {
        return Err("server returned unexpected response for agent register".to_string());
    };
    Ok(format!(
        "{}server_agent_registered=true\nagent={}\nagent_id={}\nstatus={}\n",
        header, agent.name, agent.agent_id, agent.status
    ))
}

pub(crate) fn server_agent_list(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let response = server(parsed)?
        .handle(request(
            args,
            "server-agent-list",
            ServerCommand::ListAgents,
        )?)
        .map_err(debug_error)?;
    let header = render_response_header(&response);
    let ServerResponsePayload::Agents(agents) = response.payload else {
        return Err("server returned unexpected response for agent list".to_string());
    };
    let mut output = format!(
        "{}server_agents_listed=true\nactive_agents={}\n",
        header,
        agents.len()
    );
    for agent in agents {
        output.push_str(&render_agent_line(&agent));
    }
    Ok(output)
}

pub(crate) fn server_task_send(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let agent_name = required_arg(args, "--agent")?;
    let goal = required_arg(args, "--goal")?;
    let scenario = optional_arg(args, "--scenario").unwrap_or_else(|| "default".to_string());
    let response = server(parsed)?
        .handle(request(
            args,
            "server-task-send",
            ServerCommand::SendTask {
                agent_name: agent_name.clone(),
                goal,
                scenario: scenario.clone(),
            },
        )?)
        .map_err(debug_error)?;
    let header = render_response_header(&response);
    let ServerResponsePayload::TaskSent(run) = response.payload else {
        return Err("server returned unexpected response for task send".to_string());
    };
    Ok(format!(
        "{}server_task_sent=true\nagent={agent_name}\nscenario={scenario}\ntask_id={}\nsession_id={}\nrun_id={}\n",
        header, run.task_id, run.session_id, run.run_id
    ))
}

pub(crate) fn server_agent_status(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let agent_name = required_arg(args, "--agent")?;
    let response = server(parsed)?
        .handle(request(
            args,
            "server-agent-status",
            ServerCommand::AgentStatus { agent_name },
        )?)
        .map_err(debug_error)?;
    let header = render_response_header(&response);
    let ServerResponsePayload::AgentStatus(agent) = response.payload else {
        return Err("server returned unexpected response for agent status".to_string());
    };
    Ok(format!(
        "{}server_agent_status=true\n{}",
        header,
        render_agent_line(&agent)
    ))
}

pub(crate) fn server_dashboard(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let recent_event_limit = optional_arg(args, "--recent-events")
        .map(|value| {
            value
                .parse::<usize>()
                .map_err(|_| "--recent-events must be a positive integer".to_string())
        })
        .transpose()?
        .unwrap_or(5);
    let response = server(parsed)?
        .handle(request(
            args,
            "server-dashboard",
            ServerCommand::Dashboard { recent_event_limit },
        )?)
        .map_err(debug_error)?;
    let header = render_response_header(&response);
    let ServerResponsePayload::Dashboard(snapshot) = response.payload else {
        return Err("server returned unexpected response for dashboard".to_string());
    };
    let mut output = format!(
        "{}server_dashboard=true\nproject={}\nagent_count={}\nactive_session_count={}\n",
        header, snapshot.project_id, snapshot.agent_count, snapshot.active_session_count
    );
    for agent in snapshot.agents {
        output.push_str(&render_agent_line(&agent));
    }
    Ok(output)
}

pub(crate) fn server_recover(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let response = server(parsed)?
        .handle(request(args, "server-recover", ServerCommand::Recover)?)
        .map_err(debug_error)?;
    let header = render_response_header(&response);
    let ServerResponsePayload::Recovery(recovery) = response.payload else {
        return Err("server returned unexpected response for recovery".to_string());
    };
    Ok(format!(
        "{}server_recovered=true\nrecovery_attempt_id={}\nwatermark={}\nrecovered_run_count={}\n",
        header,
        recovery.recovery_attempt_id,
        recovery
            .watermark
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_string()),
        recovery.recovered_run_count
    ))
}

fn server(parsed: &ParsedArgs) -> Result<CapoServer, String> {
    CapoServer::open(project_id(), &parsed.state_root).map_err(debug_error)
}

fn request(
    args: &[String],
    default_slug: &str,
    command: ServerCommand,
) -> Result<ServerRequest, String> {
    let request_id = optional_arg(args, "--request").unwrap_or_else(|| {
        format!(
            "{default_slug}-{}",
            stable_cli_hash(&format!("{command:?}"))
        )
    });
    let mut request = ServerRequest::local_cli(request_id, command);
    if let Some(client_id) = optional_arg(args, "--client") {
        request.origin.client_id = client_id;
    }
    if let Some(actor_id) = optional_arg(args, "--actor") {
        request.origin.actor_id = actor_id;
    }
    Ok(request)
}

fn render_response_header(response: &ServerResponse) -> String {
    format!(
        "server_boundary=capo-server\nserver_request_id={}\nserver_client_id={}\nserver_actor_id={}\nserver_input_origin={:?}\n",
        response.request_id, response.client_id, response.actor_id, response.input_origin
    )
}

fn render_agent_line(agent: &AgentSummary) -> String {
    let session = agent.session.as_ref();
    format!(
        "agent={} status={} current_session={} session_status={} run_status={} tool_calls={} memory_packets={}\n",
        agent.name,
        agent.status,
        agent
            .current_session_id
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_else(|| "none".to_string()),
        session
            .map(|session| session.status.as_str())
            .unwrap_or("none"),
        session
            .and_then(|session| session.run_status.as_deref())
            .unwrap_or("none"),
        session.map(|session| session.tool_call_count).unwrap_or(0),
        session
            .map(|session| session.memory_packet_count)
            .unwrap_or(0)
    )
}

fn require_fake_arg(args: &[String], key: &str) -> Result<(), String> {
    match optional_arg(args, key).as_deref() {
        None | Some("fake") => Ok(()),
        Some(other) => Err(format!("{key} only supports `fake` in SV1, got `{other}`")),
    }
}
