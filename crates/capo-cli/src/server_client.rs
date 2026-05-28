use std::io::Write;
use std::net::{TcpListener, ToSocketAddrs};
use std::{fs, path::PathBuf};

use capo_server::{
    AgentSummary, CapoServer, ServerCommand, ServerRequest, ServerResponse, ServerResponsePayload,
    send_tcp, serve_tcp,
};

use crate::cli_surface::{ParsedArgs, required_arg};
use crate::{debug_error, project_id, stable_cli_hash};

mod dispatch;

pub(crate) use dispatch::{
    server_dispatch_gate, server_dispatch_live_preflight, server_dispatch_live_run_local,
    server_dispatch_plan, server_dispatch_run_local,
};

pub(super) const MAX_ADAPTER_FIXTURE_BYTES: u64 = 256 * 1024;
pub(crate) const DEFAULT_SERVER_ADDR: &str = "127.0.0.1:7878";

pub(crate) fn server_serve(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let address =
        optional_value(args, "--addr")?.unwrap_or_else(|| DEFAULT_SERVER_ADDR.to_string());
    require_loopback_address(&address)?;
    let max_requests = optional_value(args, "--max-requests")?
        .map(|value| {
            value
                .parse::<usize>()
                .map_err(|_| "--max-requests must be a non-negative integer".to_string())
        })
        .transpose()?;
    let listener = TcpListener::bind(&address).map_err(debug_error)?;
    let bound_address = listener.local_addr().map_err(debug_error)?;
    if !bound_address.ip().is_loopback() {
        return Err(format!(
            "server bind address must be loopback, got {bound_address}"
        ));
    }
    println!("server_listening=true\nserver_addr={bound_address}");
    std::io::stdout().flush().map_err(debug_error)?;
    let served =
        serve_tcp(listener, project_id(), &parsed.state_root, max_requests).map_err(debug_error)?;
    Ok(format!(
        "server_stopped=true\nserver_addr={bound_address}\nrequests_served={served}\n"
    ))
}

pub(crate) fn server_agent_register(
    parsed: &ParsedArgs,
    args: &[String],
) -> Result<String, String> {
    require_fake_arg(args, "--adapter")?;
    require_fake_arg(args, "--runtime")?;
    let name = required_arg(args, "--name")?;
    let response = handle(
        parsed,
        args,
        request(
            args,
            "server-agent-register",
            ServerCommand::RegisterAgent { name },
        )?,
    )?;
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
    let response = handle(
        parsed,
        args,
        request(args, "server-agent-list", ServerCommand::ListAgents)?,
    )?;
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
    let scenario = optional_value(args, "--scenario")?.unwrap_or_else(|| "default".to_string());
    let response = handle(
        parsed,
        args,
        request(
            args,
            "server-task-send",
            ServerCommand::SendTask {
                agent_name: agent_name.clone(),
                goal,
                scenario: scenario.clone(),
            },
        )?,
    )?;
    let header = render_response_header(&response);
    let ServerResponsePayload::TaskSent(run) = response.payload else {
        return Err("server returned unexpected response for task send".to_string());
    };
    Ok(format!(
        "{}server_task_sent=true\nagent={agent_name}\nscenario={scenario}\ntask_id={}\nsession_id={}\nrun_id={}\n",
        header, run.task_id, run.session_id, run.run_id
    ))
}

pub(crate) fn server_agent_steer(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let agent_name = required_arg(args, "--agent")?;
    let goal = required_arg(args, "--goal")?;
    let response = handle(
        parsed,
        args,
        request(
            args,
            "server-agent-steer",
            ServerCommand::SteerAgent {
                agent_name: agent_name.clone(),
                goal,
            },
        )?,
    )?;
    let header = render_response_header(&response);
    let ServerResponsePayload::AgentStatus(agent) = response.payload else {
        return Err("server returned unexpected response for agent steer".to_string());
    };
    Ok(format!(
        "{}server_agent_steered=true\n{}",
        header,
        render_agent_line(&agent)
    ))
}

pub(crate) fn server_agent_status(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let agent_name = required_arg(args, "--agent")?;
    let response = handle(
        parsed,
        args,
        request(
            args,
            "server-agent-status",
            ServerCommand::AgentStatus { agent_name },
        )?,
    )?;
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
    let recent_event_limit = optional_value(args, "--recent-events")?
        .map(|value| {
            value
                .parse::<usize>()
                .map_err(|_| "--recent-events must be a positive integer".to_string())
        })
        .transpose()?
        .unwrap_or(5);
    let response = handle(
        parsed,
        args,
        request(
            args,
            "server-dashboard",
            ServerCommand::Dashboard { recent_event_limit },
        )?,
    )?;
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

pub(crate) fn server_session_start(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let agent_name = required_arg(args, "--agent")?;
    let goal = required_arg(args, "--goal")?;
    let adapter = optional_value(args, "--adapter")?.unwrap_or_else(|| "acp".to_string());
    require_adapter_arg(&adapter)?;
    let session_id = optional_value(args, "--session")?;
    let run_id = optional_value(args, "--run")?;
    let response = handle(
        parsed,
        args,
        request(
            args,
            "server-session-start",
            ServerCommand::StartSession {
                agent_name: agent_name.clone(),
                goal,
                adapter: adapter.clone(),
                session_id,
                run_id,
            },
        )?,
    )?;
    let header = render_response_header(&response);
    let ServerResponsePayload::SessionStarted(run) = response.payload else {
        return Err("server returned unexpected response for session start".to_string());
    };
    Ok(format!(
        "{}server_session_started=true\nagent={agent_name}\nadapter={adapter}\ntask_id={}\nsession_id={}\nrun_id={}\nprovider_cli_executed=false\nsession_start_kind=server_native\n",
        header, run.task_id, run.session_id, run.run_id
    ))
}

pub(crate) fn server_adapter_replay_fixture(
    parsed: &ParsedArgs,
    args: &[String],
) -> Result<String, String> {
    let adapter = required_arg(args, "--adapter")?;
    require_adapter_arg(&adapter)?;
    let fixture_path = PathBuf::from(required_arg(args, "--fixture")?);
    let session_id = required_arg(args, "--session")?;
    let run_id = required_arg(args, "--run")?;
    let turn_id = required_arg(args, "--turn")?;
    if let Ok(metadata) = fs::metadata(&fixture_path)
        && metadata.len() > MAX_ADAPTER_FIXTURE_BYTES
    {
        return Err(format!(
            "adapter fixture is too large: {} bytes > {} bytes",
            metadata.len(),
            MAX_ADAPTER_FIXTURE_BYTES
        ));
    }
    let fixture_jsonl = fs::read_to_string(&fixture_path)
        .map_err(|error| format!("failed to read adapter fixture: {error}"))?;
    if fixture_jsonl.len() as u64 > MAX_ADAPTER_FIXTURE_BYTES {
        return Err(format!(
            "adapter fixture is too large: {} bytes > {} bytes",
            fixture_jsonl.len(),
            MAX_ADAPTER_FIXTURE_BYTES
        ));
    }
    let response = handle(
        parsed,
        args,
        request(
            args,
            "server-adapter-replay-fixture",
            ServerCommand::ReplayAdapterFixture {
                adapter,
                session_id,
                run_id,
                turn_id,
                fixture_name: fixture_path.display().to_string(),
                fixture_jsonl,
            },
        )?,
    )?;
    let header = render_response_header(&response);
    let ServerResponsePayload::AdapterFixtureReplayed(replay) = response.payload else {
        return Err("server returned unexpected response for adapter fixture replay".to_string());
    };
    Ok(format!(
        "{}server_adapter_replayed=true\nadapter={}\nfixture={}\nfixture_hash={}\nagent={}\nsession_id={}\nrun_id={}\nturn_id={}\nprovider_cli_executed={}\nraw_content_policy={}\ninput_events={}\nappended_events={}\ntool_events={}\nsummary_events={}\ncompleted_turns={}\n",
        header,
        replay.adapter,
        replay.fixture_name,
        replay.fixture_hash,
        replay.agent_name,
        replay.session_id,
        replay.run_id,
        replay.turn_id,
        replay.provider_cli_executed,
        replay.raw_content_policy,
        replay.input_event_count,
        replay.appended_event_count,
        replay.tool_event_count,
        replay.summary_event_count,
        replay.completed_turn_count
    ))
}

pub(crate) fn server_recover(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let response = handle(
        parsed,
        args,
        request(args, "server-recover", ServerCommand::Recover)?,
    )?;
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

pub(super) fn read_bounded_fixture(path: &PathBuf) -> Result<String, String> {
    if let Ok(metadata) = fs::metadata(path)
        && metadata.len() > MAX_ADAPTER_FIXTURE_BYTES
    {
        return Err(format!(
            "adapter fixture is too large: {} bytes > {} bytes",
            metadata.len(),
            MAX_ADAPTER_FIXTURE_BYTES
        ));
    }
    let contents = fs::read_to_string(path)
        .map_err(|error| format!("failed to read adapter fixture: {error}"))?;
    if contents.len() as u64 > MAX_ADAPTER_FIXTURE_BYTES {
        return Err(format!(
            "adapter fixture is too large: {} bytes > {} bytes",
            contents.len(),
            MAX_ADAPTER_FIXTURE_BYTES
        ));
    }
    Ok(contents)
}

fn server(parsed: &ParsedArgs) -> Result<CapoServer, String> {
    CapoServer::open(project_id(), &parsed.state_root).map_err(debug_error)
}

pub(super) fn handle(
    parsed: &ParsedArgs,
    args: &[String],
    request: ServerRequest,
) -> Result<ServerResponse, String> {
    if let Some(address) = optional_value(args, "--connect")? {
        require_loopback_address(&address)?;
        return send_tcp(address, &request).map_err(debug_error);
    }
    if let Some(address) = default_running_server_address() {
        match send_tcp(&address, &request) {
            Ok(response) => return Ok(response),
            Err(capo_server::TransportError::Io(error))
                if error.kind() == std::io::ErrorKind::ConnectionRefused => {}
            Err(error) => return Err(debug_error(error)),
        }
    }
    server(parsed)?.handle(request).map_err(debug_error)
}

pub(super) fn request(
    args: &[String],
    default_slug: &str,
    command: ServerCommand,
) -> Result<ServerRequest, String> {
    let request_id = optional_value(args, "--request")?.unwrap_or_else(|| {
        format!(
            "{default_slug}-{}",
            stable_cli_hash(&format!("{command:?}"))
        )
    });
    let mut request = ServerRequest::local_cli(request_id, command);
    if let Some(client_id) = optional_value(args, "--client")? {
        request.origin.client_id = client_id;
    }
    if let Some(actor_id) = optional_value(args, "--actor")? {
        request.origin.actor_id = actor_id;
    }
    Ok(request)
}

pub(super) fn render_response_header(response: &ServerResponse) -> String {
    format!(
        "server_boundary=capo-server\nserver_request_id={}\nserver_client_id={}\nserver_actor_id={}\nserver_input_origin={:?}\n",
        response.request_id, response.client_id, response.actor_id, response.input_origin
    )
}

pub(crate) fn render_agent_line(agent: &AgentSummary) -> String {
    let session = agent.session.as_ref();
    format!(
        "agent={} status={} current_session={} session_status={} run_status={} adapter_kind={} evidence_count={} evidence_refs={} turn_count={} turn_ids={} latest_dispatch_plan={} latest_dispatch_gate={} latest_dispatch_execution={} dispatch_gate_status={} dispatch_gate_reasons={} dispatch_next_action={} dispatch_execution_status={} dispatch_runtime_process_ref={} dispatch_provider_cli_execution_allowed={} dispatch_provider_cli_executed={} dispatch_credential_scan_status={} dispatch_raw_prompt_policy={} dispatch_raw_output_policy={} tool_calls={} memory_packets={}\n",
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
        session
            .and_then(|session| session.adapter_kind.as_deref())
            .unwrap_or("none"),
        session.map(|session| session.evidence_count).unwrap_or(0),
        session
            .map(|session| session.evidence_refs.join(","))
            .filter(|refs| !refs.is_empty())
            .unwrap_or_else(|| "none".to_string()),
        session.map(|session| session.turn_count).unwrap_or(0),
        session
            .map(|session| session.turn_ids.join(","))
            .filter(|refs| !refs.is_empty())
            .unwrap_or_else(|| "none".to_string()),
        session
            .and_then(|session| session.latest_dispatch_plan_id.as_deref())
            .unwrap_or("none"),
        session
            .and_then(|session| session.latest_dispatch_gate_id.as_deref())
            .unwrap_or("none"),
        session
            .and_then(|session| session.latest_dispatch_execution_id.as_deref())
            .unwrap_or("none"),
        session
            .and_then(|session| session.dispatch_gate_status.as_deref())
            .unwrap_or("none"),
        session
            .and_then(|session| session.dispatch_gate_reasons.as_deref())
            .unwrap_or("none"),
        session
            .and_then(|session| session.dispatch_next_action.as_deref())
            .unwrap_or("none"),
        session
            .and_then(|session| session.dispatch_execution_status.as_deref())
            .unwrap_or("none"),
        session
            .and_then(|session| session.dispatch_runtime_process_ref.as_deref())
            .unwrap_or("none"),
        session
            .and_then(|session| session.dispatch_provider_cli_execution_allowed)
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_string()),
        session
            .and_then(|session| session.dispatch_provider_cli_executed)
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_string()),
        session
            .and_then(|session| session.dispatch_credential_scan_status.as_deref())
            .unwrap_or("none"),
        session
            .and_then(|session| session.dispatch_raw_prompt_policy.as_deref())
            .unwrap_or("none"),
        session
            .and_then(|session| session.dispatch_raw_output_policy.as_deref())
            .unwrap_or("none"),
        session.map(|session| session.tool_call_count).unwrap_or(0),
        session
            .map(|session| session.memory_packet_count)
            .unwrap_or(0)
    )
}

fn require_fake_arg(args: &[String], key: &str) -> Result<(), String> {
    match optional_value(args, key)?.as_deref() {
        None | Some("fake") => Ok(()),
        Some(other) => Err(format!("{key} only supports `fake` in SV1, got `{other}`")),
    }
}

pub(super) fn require_adapter_arg(adapter: &str) -> Result<(), String> {
    match adapter {
        "codex" | "codex-exec" | "codex_exec" | "claude" | "claude-code" | "claude_code"
        | "acp" => Ok(()),
        other => Err(format!(
            "unsupported server adapter fixture kind: {other}; expected codex, claude, or acp"
        )),
    }
}

pub(super) fn require_live_provider_adapter_arg(adapter: &str) -> Result<(), String> {
    match adapter {
        "codex" | "codex-exec" | "codex_exec" | "claude" | "claude-code" | "claude_code" => Ok(()),
        other => Err(format!(
            "unsupported live provider adapter kind: {other}; expected codex or claude"
        )),
    }
}

fn optional_value(args: &[String], key: &str) -> Result<Option<String>, String> {
    let Some(index) = args.iter().position(|arg| arg == key) else {
        return Ok(None);
    };
    let Some(value) = args.get(index + 1) else {
        return Err(format!("{key} requires a value"));
    };
    if value.starts_with("--") {
        return Err(format!("{key} requires a value"));
    }
    Ok(Some(value.clone()))
}

fn require_loopback_address(address: &str) -> Result<(), String> {
    let resolved = address
        .to_socket_addrs()
        .map_err(debug_error)?
        .collect::<Vec<_>>();
    if resolved.is_empty() {
        return Err(format!("server bind address did not resolve: {address}"));
    }
    if !resolved.iter().all(|address| address.ip().is_loopback()) {
        return Err(format!(
            "server bind address must resolve only to loopback addresses, got {address}"
        ));
    }
    Ok(())
}

fn default_running_server_address() -> Option<String> {
    std::env::var("CAPO_SERVER_ADDR")
        .ok()
        .filter(|address| !address.trim().is_empty())
        .or_else(|| Some(DEFAULT_SERVER_ADDR.to_string()))
}
