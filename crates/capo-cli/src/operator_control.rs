use std::io::{BufRead, IsTerminal, Read, Write};
use std::net::ToSocketAddrs;

use capo_server::{
    AgentSummary, ServerCommand, ServerDashboardSnapshot, ServerRequest, ServerResponsePayload,
    send_tcp,
};

use crate::cli_surface::ParsedArgs;
use crate::server_client::DEFAULT_SERVER_ADDR;
use crate::{debug_error, stable_cli_hash};

pub(crate) fn operator_control(_parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let planner = optional_value(args, "--planner")?.unwrap_or_else(|| "none".to_string());
    if planner != "none" {
        return Err(format!(
            "unsupported planner: {planner}; only --planner none is implemented"
        ));
    }

    let address = server_address(args)?;
    require_loopback_address(&address)?;

    let mut repl = ControlRepl::new(address);
    if std::io::stdin().is_terminal() {
        repl.run_interactive()
    } else {
        let mut script = String::new();
        std::io::stdin()
            .read_to_string(&mut script)
            .map_err(debug_error)?;
        repl.run_script(&script)
    }
}

struct ControlRepl {
    address: String,
    attached_agent: Option<String>,
    request_counter: usize,
}

impl ControlRepl {
    fn new(address: String) -> Self {
        Self {
            address,
            attached_agent: None,
            request_counter: 0,
        }
    }

    fn run_script(&mut self, script: &str) -> Result<String, String> {
        let mut output = format!(
            "Capo control\nplanner: none\nserver: {}\n\nType `help` for commands.\n",
            self.address
        );
        for line in script.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            match self.run_line(line) {
                LineResult::Continue(rendered) => output.push_str(&rendered),
                LineResult::Quit(rendered) => {
                    output.push_str(&rendered);
                    return Ok(output);
                }
            }
        }
        Ok(output)
    }

    fn run_interactive(&mut self) -> Result<String, String> {
        println!(
            "Capo control\nplanner: none\nserver: {}\n\nType `help` for commands.",
            self.address
        );
        let stdin = std::io::stdin();
        let mut stdin = stdin.lock();
        loop {
            print!("capo> ");
            std::io::stdout().flush().map_err(debug_error)?;
            let mut line = String::new();
            let bytes = stdin.read_line(&mut line).map_err(debug_error)?;
            if bytes == 0 {
                println!();
                return Ok(String::new());
            }
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            match self.run_line(line) {
                LineResult::Continue(rendered) => print!("{rendered}"),
                LineResult::Quit(rendered) => {
                    print!("{rendered}");
                    return Ok(String::new());
                }
            }
        }
    }

    fn run_line(&mut self, line: &str) -> LineResult {
        let result = match parse_action(line) {
            Ok(OperatorAction::Help) => Ok(help_text()),
            Ok(OperatorAction::Quit) => return LineResult::Quit("bye\n".to_string()),
            Ok(OperatorAction::ListAgents) => self.list_agents(),
            Ok(OperatorAction::Dashboard) => self.dashboard(),
            Ok(OperatorAction::Status { agent }) => self.status(agent),
            Ok(OperatorAction::Attach { agent }) => self.attach(agent),
            Ok(OperatorAction::Detach) => {
                self.attached_agent = None;
                Ok("detached\n".to_string())
            }
            Ok(OperatorAction::Send { agent, message }) => self.send(agent, message),
            Err(error) => Ok(format!("error: {error}\n")),
        };
        LineResult::Continue(match result {
            Ok(output) => output,
            Err(error) => format!("error: {error}\n"),
        })
    }

    fn list_agents(&mut self) -> Result<String, String> {
        let response = self.send_request(ServerCommand::ListAgents)?;
        let ServerResponsePayload::Agents(agents) = response else {
            return Err("server returned unexpected response for agents".to_string());
        };
        let mut output = format!("Agents ({})\n", agents.len());
        for agent in agents {
            output.push_str(&render_human_agent(&agent));
        }
        Ok(output)
    }

    fn dashboard(&mut self) -> Result<String, String> {
        let response = self.send_request(ServerCommand::Dashboard {
            recent_event_limit: 5,
        })?;
        let ServerResponsePayload::Dashboard(snapshot) = response else {
            return Err("server returned unexpected response for dashboard".to_string());
        };
        Ok(render_dashboard(&snapshot))
    }

    fn status(&mut self, agent: Option<String>) -> Result<String, String> {
        let agent = self.resolve_agent(agent)?;
        let summary = self.agent_status(&agent)?;
        Ok(format!("Status\n{}", render_human_agent(&summary)))
    }

    fn attach(&mut self, agent: String) -> Result<String, String> {
        let summary = self.agent_status(&agent)?;
        self.attached_agent = Some(agent.clone());
        Ok(format!(
            "attached: {agent}\n{}",
            render_human_agent(&summary)
        ))
    }

    fn send(&mut self, agent: Option<String>, message: String) -> Result<String, String> {
        let agent = self.resolve_agent(agent)?;
        let response = self.send_request(ServerCommand::SteerAgent {
            agent_name: agent.clone(),
            goal: message,
        })?;
        let ServerResponsePayload::AgentStatus(summary) = response else {
            return Err("server returned unexpected response for send".to_string());
        };
        Ok(format!("sent to {agent}\n{}", render_human_agent(&summary)))
    }

    fn agent_status(&mut self, agent: &str) -> Result<AgentSummary, String> {
        let response = self.send_request(ServerCommand::AgentStatus {
            agent_name: agent.to_string(),
        })?;
        let ServerResponsePayload::AgentStatus(summary) = response else {
            return Err("server returned unexpected response for status".to_string());
        };
        Ok(summary)
    }

    fn send_request(&mut self, command: ServerCommand) -> Result<ServerResponsePayload, String> {
        self.request_counter += 1;
        let request_id = format!(
            "control-{}-{}",
            self.request_counter,
            stable_cli_hash(&format!("{command:?}"))
        );
        let response = send_tcp(
            &self.address,
            &ServerRequest::local_cli(request_id, command),
        )
        .map_err(debug_error)?;
        Ok(response.payload)
    }

    fn resolve_agent(&self, agent: Option<String>) -> Result<String, String> {
        agent
            .or_else(|| self.attached_agent.clone())
            .ok_or_else(|| {
                "no agent selected; use `attach NAME` or `send --agent NAME ...`".to_string()
            })
    }
}

enum LineResult {
    Continue(String),
    Quit(String),
}

enum OperatorAction {
    Help,
    Quit,
    ListAgents,
    Dashboard,
    Status {
        agent: Option<String>,
    },
    Attach {
        agent: String,
    },
    Detach,
    Send {
        agent: Option<String>,
        message: String,
    },
}

fn parse_action(line: &str) -> Result<OperatorAction, String> {
    let mut parts = line.split_whitespace();
    let command = parts.next().unwrap_or_default();
    match command {
        "help" | "?" => Ok(OperatorAction::Help),
        "quit" | "exit" => Ok(OperatorAction::Quit),
        "agents" | "ls" => Ok(OperatorAction::ListAgents),
        "dashboard" | "overview" => Ok(OperatorAction::Dashboard),
        "detach" | "back" => Ok(OperatorAction::Detach),
        "attach" | "jump" => {
            let agent = parts
                .next()
                .ok_or_else(|| "attach requires an agent name".to_string())?;
            Ok(OperatorAction::Attach {
                agent: agent.to_string(),
            })
        }
        "status" => Ok(OperatorAction::Status {
            agent: parts.next().map(ToString::to_string),
        }),
        "send" => parse_send(line),
        other => Err(format!("unknown command `{other}`")),
    }
}

fn parse_send(line: &str) -> Result<OperatorAction, String> {
    let rest = line
        .strip_prefix("send")
        .expect("parse_send is only called for send")
        .trim();
    if rest.is_empty() {
        return Err("send requires a message".to_string());
    }
    if let Some(after_flag) = rest.strip_prefix("--agent ") {
        let mut split = after_flag.splitn(2, char::is_whitespace);
        let agent = split
            .next()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "send --agent requires an agent name".to_string())?;
        let message = split
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "send requires a message".to_string())?;
        return Ok(OperatorAction::Send {
            agent: Some(agent.to_string()),
            message: message.to_string(),
        });
    }
    Ok(OperatorAction::Send {
        agent: None,
        message: rest.to_string(),
    })
}

fn render_dashboard(snapshot: &ServerDashboardSnapshot) -> String {
    let mut output = format!(
        "Dashboard\nproject: {}\nagents: {}\nactive sessions: {}\n",
        snapshot.project_id, snapshot.agent_count, snapshot.active_session_count
    );
    for agent in &snapshot.agents {
        output.push_str(&render_human_agent(agent));
    }
    output
}

fn render_human_agent(agent: &AgentSummary) -> String {
    let mut output = format!("- {} [{}]", agent.name, agent.status);
    if let Some(session) = agent.session.as_ref() {
        output.push_str(&format!(
            " session={} run={} run_status={} tools={} memory={}",
            session.session_id,
            session
                .run_id
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| "none".to_string()),
            session.run_status.as_deref().unwrap_or("none"),
            session.tool_call_count,
            session.memory_packet_count
        ));
    } else {
        output.push_str(" session=none");
    }
    output.push('\n');
    output
}

fn help_text() -> String {
    "\
Commands:
  agents | ls
  dashboard | overview
  status [AGENT]
  attach AGENT | jump AGENT
  send [--agent AGENT] MESSAGE
  detach | back
  help
  quit | exit
"
    .to_string()
}

fn server_address(args: &[String]) -> Result<String, String> {
    optional_value(args, "--connect")?
        .or_else(|| {
            std::env::var("CAPO_SERVER_ADDR")
                .ok()
                .filter(|value| !value.trim().is_empty())
        })
        .or_else(|| Some(DEFAULT_SERVER_ADDR.to_string()))
        .ok_or_else(|| "missing server address".to_string())
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
        return Err(format!("server address did not resolve: {address}"));
    }
    if !resolved.iter().all(|address| address.ip().is_loopback()) {
        return Err(format!(
            "server address must resolve only to loopback addresses, got {address}"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server_client::render_agent_line;

    #[test]
    fn unsupported_planner_is_rejected_before_server_connect() {
        let parsed = ParsedArgs::new(vec![]).expect("parsed");
        let error = operator_control(&parsed, &["--planner".to_string(), "codex".to_string()])
            .expect_err("unsupported planner should fail");
        assert!(error.contains("unsupported planner: codex"));
    }

    #[test]
    fn parser_keeps_send_message_text_together() {
        let Ok(OperatorAction::Send {
            agent: Some(agent),
            message,
        }) = parse_send("send --agent demo please inspect the current status")
        else {
            panic!("expected send action");
        };
        assert_eq!(agent, "demo");
        assert_eq!(message, "please inspect the current status");
    }

    #[test]
    fn parser_supports_attached_agent_send() {
        let Ok(OperatorAction::Send {
            agent: None,
            message,
        }) = parse_send("send please continue")
        else {
            panic!("expected send action");
        };
        assert_eq!(message, "please continue");
    }

    #[test]
    fn raw_server_line_renderer_stays_available_for_existing_commands() {
        let agent = AgentSummary {
            agent_id: capo_core::AgentId::new("agent-demo"),
            name: "demo".to_string(),
            status: "available".to_string(),
            current_session_id: None,
            session: None,
        };
        assert!(render_agent_line(&agent).contains("agent=demo"));
    }
}
