use std::io::{BufRead, IsTerminal, Read, Write};

use capo_server::{AgentSummary, ServerCommand, ServerRequest, ServerResponsePayload, send_tcp};

use crate::cli_surface::ParsedArgs;
use crate::{debug_error, stable_cli_hash};

mod planner;
mod render;
mod server_process;

use planner::{CapoPlanner, NonePlanner, OperatorAction, Planner, PlannerDecisionAudit};
use render::{
    render_dashboard, render_evidence_summary, render_human_agent, render_recent_work,
    render_review_needs, render_tool_activity,
};
use server_process::{AutoServer, ensure_server_running, require_loopback_address, server_address};

pub(crate) fn operator_control(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    let planner = control_planner(args)?;
    let address = server_address(args)?;
    require_loopback_address(&address)?;
    let server = ensure_server_running(&address, parsed)?;

    let mut repl = ControlRepl::new(address, server, planner);
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

const CAPO_PLANNER_AGENT: &str = "capo-operator";

fn control_planner(args: &[String]) -> Result<Box<dyn Planner>, String> {
    let planner = optional_value(args, "--planner")?.unwrap_or_else(|| "none".to_string());
    match planner.as_str() {
        "none" => Ok(Box::new(NonePlanner)),
        "capo" => Ok(Box::new(CapoPlanner)),
        _ => Err(format!(
            "unsupported planner: {planner}; supported planners: none, capo"
        )),
    }
}

struct ControlRepl {
    address: String,
    server_started: bool,
    _server: Option<AutoServer>,
    planner: Box<dyn Planner>,
    attached_agent: Option<String>,
    request_counter: usize,
}

impl ControlRepl {
    fn new(address: String, server: Option<AutoServer>, planner: Box<dyn Planner>) -> Self {
        let server_started = server.is_some();
        Self {
            address,
            server_started,
            _server: server,
            planner,
            attached_agent: None,
            request_counter: 0,
        }
    }

    fn run_script(&mut self, script: &str) -> Result<String, String> {
        self.prepare_planner()?;
        let mut output = format!(
            "Capo control\nplanner: {}\nserver: {}{}\n\nType `help` for commands.\n",
            self.planner.mode(),
            self.address,
            if self.server_started {
                " (started)"
            } else {
                ""
            }
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
        self.prepare_planner()?;
        println!(
            "Capo control\nplanner: {}\nserver: {}{}\n\nType `help` for commands.",
            self.planner.mode(),
            self.address,
            if self.server_started {
                " (started)"
            } else {
                ""
            }
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
        let result = match self.planner.plan(line) {
            Ok(decision) if decision.action == OperatorAction::Quit => {
                return LineResult::Quit("bye\n".to_string());
            }
            Ok(decision) => {
                if let Err(error) = self.audit_planner_decision(line, decision.audit.as_ref()) {
                    Err(error)
                } else {
                    self.execute(decision.action)
                }
            }
            Err(error) => Ok(format!("error: {error}\n")),
        };
        LineResult::Continue(match result {
            Ok(output) => output,
            Err(error) => format!("error: {error}\n"),
        })
    }

    fn prepare_planner(&mut self) -> Result<(), String> {
        if self.planner.mode() != "capo" {
            return Ok(());
        }
        let agents = self.list_agent_summaries()?;
        if !agents.iter().any(|agent| agent.name == CAPO_PLANNER_AGENT) {
            let response = self.send_request(ServerCommand::RegisterAgent {
                name: CAPO_PLANNER_AGENT.to_string(),
            })?;
            if !matches!(response, ServerResponsePayload::AgentRegistered(_)) {
                return Err(
                    "server returned unexpected response for planner registration".to_string(),
                );
            }
        }
        let planner_status = self.agent_status(CAPO_PLANNER_AGENT)?;
        if planner_status.session.is_none() {
            let response = self.send_request(ServerCommand::StartSession {
                agent_name: CAPO_PLANNER_AGENT.to_string(),
                goal: "Capo operator planner mode: inspect operator input and choose server-backed actions".to_string(),
                adapter: "acp".to_string(),
                session_id: None,
                run_id: None,
            })?;
            if !matches!(response, ServerResponsePayload::SessionStarted(_)) {
                return Err(
                    "server returned unexpected response for planner session start".to_string(),
                );
            }
        }
        Ok(())
    }

    fn audit_planner_decision(
        &mut self,
        line: &str,
        audit: Option<&PlannerDecisionAudit>,
    ) -> Result<(), String> {
        let Some(audit) = audit else {
            return Ok(());
        };
        let target = audit.target_agent.as_deref().unwrap_or("none");
        let input_hash = stable_cli_hash(line);
        let decision = format!(
            "capo planner decision input_hash={input_hash} action={} target_agent={target} mutation={} summary={}",
            audit.action_label, audit.mutation, audit.summary
        );
        let response = self.send_request(ServerCommand::SteerAgent {
            agent_name: CAPO_PLANNER_AGENT.to_string(),
            goal: decision,
        })?;
        if matches!(response, ServerResponsePayload::AgentStatus(_)) {
            Ok(())
        } else {
            Err("server returned unexpected response for planner audit".to_string())
        }
    }

    fn execute(&mut self, action: OperatorAction) -> Result<String, String> {
        match action {
            OperatorAction::Help => Ok(help_text()),
            OperatorAction::Quit => unreachable!("quit is handled before execution"),
            OperatorAction::ListAgents => self.list_agents(),
            OperatorAction::Dashboard => self.dashboard(),
            OperatorAction::Status { agent } => self.status(agent),
            OperatorAction::RecentWork { agent } => self.recent_work(agent),
            OperatorAction::ToolActivity { agent } => self.tool_activity(agent),
            OperatorAction::Evidence { agent } => self.evidence(agent),
            OperatorAction::ReviewNeeds { agent } => self.review_needs(agent),
            OperatorAction::Attach { agent } => self.attach(agent),
            OperatorAction::Detach => {
                self.attached_agent = None;
                Ok("detached\n".to_string())
            }
            OperatorAction::Send { agent, message } => self.send(agent, message),
            OperatorAction::Interrupt { agent, reason } => self.interrupt(agent, reason),
            OperatorAction::Stop { agent, reason } => self.stop(agent, reason),
        }
    }

    fn list_agents(&mut self) -> Result<String, String> {
        let agents = self.list_agent_summaries()?;
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
        let summary = self.resolved_agent_status(agent)?;
        Ok(format!("Status\n{}", render_human_agent(&summary)))
    }

    fn recent_work(&mut self, agent: Option<String>) -> Result<String, String> {
        self.read_agent_or_all(agent, render_recent_work)
    }

    fn tool_activity(&mut self, agent: Option<String>) -> Result<String, String> {
        self.read_agent_or_all(agent, render_tool_activity)
    }

    fn evidence(&mut self, agent: Option<String>) -> Result<String, String> {
        self.read_agent_or_all(agent, render_evidence_summary)
    }

    fn review_needs(&mut self, agent: Option<String>) -> Result<String, String> {
        self.read_agent_or_all(agent, render_review_needs)
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
        self.mutate_agent(agent, |agent_name| ServerCommand::SteerAgent {
            agent_name,
            goal: message,
        })
        .map(|(agent, summary)| format!("sent to {agent}\n{}", render_human_agent(&summary)))
    }

    fn interrupt(&mut self, agent: Option<String>, reason: String) -> Result<String, String> {
        self.mutate_agent(agent, |agent_name| ServerCommand::InterruptAgent {
            agent_name,
            reason,
        })
        .map(|(agent, summary)| {
            self.clear_attached_if(&agent);
            format!("interrupted {agent}\n{}", render_human_agent(&summary))
        })
    }

    fn stop(&mut self, agent: Option<String>, reason: String) -> Result<String, String> {
        self.mutate_agent(agent, |agent_name| ServerCommand::StopAgent {
            agent_name,
            reason,
        })
        .map(|(agent, summary)| {
            self.clear_attached_if(&agent);
            format!("stopped {agent}\n{}", render_human_agent(&summary))
        })
    }

    fn resolved_agent_status(&mut self, agent: Option<String>) -> Result<AgentSummary, String> {
        let agent = self.resolve_agent(agent)?;
        self.agent_status(&agent)
    }

    fn read_agent_or_all(
        &mut self,
        agent: Option<String>,
        render: fn(&AgentSummary) -> String,
    ) -> Result<String, String> {
        if agent.is_some() || self.attached_agent.is_some() {
            return self
                .resolved_agent_status(agent)
                .map(|summary| render(&summary));
        }
        let agents = self.list_agent_summaries()?;
        if agents.is_empty() {
            return Ok("no agents\n".to_string());
        }
        let mut output = String::new();
        for agent in agents {
            output.push_str(&render(&agent));
        }
        Ok(output)
    }

    fn list_agent_summaries(&mut self) -> Result<Vec<AgentSummary>, String> {
        let response = self.send_request(ServerCommand::ListAgents)?;
        let ServerResponsePayload::Agents(agents) = response else {
            return Err("server returned unexpected response for agents".to_string());
        };
        Ok(agents)
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

    fn mutate_agent(
        &mut self,
        agent: Option<String>,
        command: impl FnOnce(String) -> ServerCommand,
    ) -> Result<(String, AgentSummary), String> {
        let agent = self.resolve_agent(agent)?;
        let response = self.send_request(command(agent.clone()))?;
        let ServerResponsePayload::AgentStatus(summary) = response else {
            return Err("server returned unexpected response for agent mutation".to_string());
        };
        Ok((agent, summary))
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

    fn clear_attached_if(&mut self, agent: &str) {
        if self.attached_agent.as_deref() == Some(agent) {
            self.attached_agent = None;
        }
    }
}

enum LineResult {
    Continue(String),
    Quit(String),
}

fn help_text() -> String {
    "\
Commands:
  agents | ls
  dashboard | overview
  status [AGENT]
  recent [AGENT] | work [AGENT]
  tools [AGENT]
  evidence [AGENT]
  reviews [AGENT]
  attach AGENT | jump AGENT
  send [--agent AGENT] MESSAGE
  interrupt [--agent AGENT] REASON
  stop [--agent AGENT] REASON
  detach | back
  help
  quit | exit
"
    .to_string()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server_client::render_agent_line;

    #[test]
    fn unsupported_planner_is_rejected_before_server_connect() {
        let error = match control_planner(&["--planner".to_string(), "codex".to_string()]) {
            Ok(_) => panic!("unsupported planner should fail"),
            Err(error) => error,
        };
        assert!(error.contains("unsupported planner: codex"));
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
