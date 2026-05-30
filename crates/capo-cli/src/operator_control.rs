use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};

use capo_adapters::CodexExecAdapter;
use capo_server::{
    AgentSummary, DispatchRunSummary, ServerCommand, ServerRequest, ServerResponsePayload,
    TaskRunSummary, send_tcp,
};
use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;

use crate::cli_surface::ParsedArgs;
use crate::{debug_error, stable_cli_hash};

mod planner;
mod render;
mod server_process;

use planner::{
    CapoPlanner, NonePlanner, OperatorAction, Planner, PlannerDecision, PlannerDecisionAudit,
    plan_from_llm_reply,
};
use render::{
    AgentRenderer, AgentResultRenderer, ConciseResultRenderer, DetailsRenderer, EvidenceRenderer,
    RecentWorkRenderer, ResultsAndEvidenceRenderer, ReviewNeedsRenderer, ToolActivityRenderer,
    display_text, render_agent_result_body, render_dashboard, render_human_agent,
    render_human_agent_with_marker, render_recent_work, render_thread,
};
use server_process::{AutoServer, ensure_server_running, require_loopback_address, server_address};

pub(crate) fn operator_control(parsed: &ParsedArgs, args: &[String]) -> Result<String, String> {
    reject_unknown_control_flags(args)?;
    let planner = control_planner(args)?;
    let address = server_address(args)?;
    require_loopback_address(&address.address)?;
    let server = ensure_server_running(&address, parsed)?;

    let mut repl = ControlRepl::new(address.address, server, planner, parsed.state_root.clone());
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

fn reject_unknown_control_flags(args: &[String]) -> Result<(), String> {
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--planner" | "--connect" | "--state" => {
                index += 2;
            }
            flag if flag.starts_with("--") => {
                return Err(format!("unknown control option: {flag}"));
            }
            _ => {
                index += 1;
            }
        }
    }
    Ok(())
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
    state_root: PathBuf,
    attached_agent: Option<String>,
    request_counter: usize,
}

impl ControlRepl {
    fn new(
        address: String,
        server: Option<AutoServer>,
        planner: Box<dyn Planner>,
        state_root: PathBuf,
    ) -> Self {
        let server_started = server.is_some();
        Self {
            address,
            server_started,
            _server: server,
            planner,
            state_root,
            attached_agent: None,
            request_counter: 0,
        }
    }

    fn run_script(&mut self, script: &str) -> Result<String, String> {
        self.prepare_planner()?;
        let mut output = self.render_banner();
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
        print!("{}", self.render_banner());
        let mut editor = DefaultEditor::new().map_err(debug_error)?;
        loop {
            let line = match editor.readline(&self.prompt()) {
                Ok(line) => line,
                Err(ReadlineError::Interrupted) => {
                    println!("Use `quit` to exit.");
                    continue;
                }
                Err(ReadlineError::Eof) => {
                    println!();
                    return Ok(String::new());
                }
                Err(error) => return Err(debug_error(error)),
            };
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let _ = editor.add_history_entry(line);
            match self.run_line(line) {
                LineResult::Continue(rendered) => print!("{rendered}"),
                LineResult::Quit(rendered) => {
                    print!("{rendered}");
                    return Ok(String::new());
                }
            }
            std::io::stdout().flush().map_err(debug_error)?;
        }
    }

    fn run_line(&mut self, line: &str) -> LineResult {
        let result = match self.planner.plan(line) {
            Ok(decision) if decision.action == OperatorAction::Quit => {
                return LineResult::Quit("bye\n".to_string());
            }
            Ok(decision) => self.execute_planner_decision(line, decision),
            Err(_) if self.planner.mode() == "capo" => self.plan_with_operator_agent(line),
            Err(_) if self.should_direct_send(line) => self.send(None, line.to_string()),
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
        let planner_adapter = planner_status
            .session
            .as_ref()
            .and_then(|session| session.adapter_kind.as_deref());
        if planner_adapter.is_some() && planner_adapter != Some("codex_exec") {
            let response = self.send_request(ServerCommand::StopAgent {
                agent_name: CAPO_PLANNER_AGENT.to_string(),
                reason: "restart capo-operator with codex planner provider".to_string(),
            })?;
            if !matches!(response, ServerResponsePayload::AgentStatus(_)) {
                return Err("server returned unexpected response for planner restart".to_string());
            }
        }
        let planner_status = self.agent_status(CAPO_PLANNER_AGENT)?;
        if planner_status.session.is_none() {
            self.start_agent_session(
                CAPO_PLANNER_AGENT,
                "codex",
                "Capo operator planner mode: use Codex to map operator input into validated server-backed actions",
            )?;
        }
        Ok(())
    }

    fn execute_planner_decision(
        &mut self,
        line: &str,
        decision: PlannerDecision,
    ) -> Result<String, String> {
        if let Err(error) = self.audit_planner_decision(line, decision.audit.as_ref()) {
            Err(error)
        } else {
            self.execute(decision.action)
        }
    }

    fn plan_with_operator_agent(&mut self, line: &str) -> Result<String, String> {
        let provider =
            std::env::var("CAPO_CONTROL_PLANNER_PROVIDER").unwrap_or_else(|_| "codex".to_string());
        if provider != "codex" {
            return Err(format!(
                "unsupported capo planner provider: {provider}; supported providers: codex"
            ));
        }
        let prompt = self.operator_planner_prompt(line)?;
        let summary = self.agent_status(CAPO_PLANNER_AGENT)?;
        let session = summary
            .session
            .as_ref()
            .ok_or_else(|| "capo-operator has no planner session".to_string())?;
        let mock_jsonl = std::env::var("CAPO_CONTROL_PLANNER_MOCK_CODEX_JSONL").ok();
        let run = self.run_codex_live_turn_with_options(
            CAPO_PLANNER_AGENT,
            &prompt,
            &session.session_id.to_string(),
            &session
                .run_id
                .as_ref()
                .ok_or_else(|| "capo-operator has no active planner run".to_string())?
                .to_string(),
            mock_jsonl.as_deref(),
            90,
        )?;
        let reply = if let Some(jsonl) = mock_jsonl.as_deref() {
            latest_codex_reply_from_jsonl(jsonl)
        } else {
            latest_codex_reply_from_artifact(&self.state_root, &run)
        }
        .ok_or_else(|| "capo planner Codex run produced no assistant action".to_string())?;
        let decision = plan_from_llm_reply(&reply)?;
        self.execute_planner_decision(line, decision)
    }

    fn operator_planner_prompt(&mut self, line: &str) -> Result<String, String> {
        let agents = self.list_agent_summaries()?;
        let mut roster = String::new();
        for agent in agents {
            if agent.name == CAPO_PLANNER_AGENT {
                continue;
            }
            let status = agent
                .session
                .as_ref()
                .and_then(|session| {
                    session
                        .dispatch_execution_status
                        .as_deref()
                        .or(session.run_status.as_deref())
                })
                .unwrap_or("idle");
            roster.push_str(&format!("- {}: {status}\n", agent.name));
        }
        if roster.is_empty() {
            roster.push_str("- none\n");
        }
        Ok(format!(
            "\
You are Capo's operator agent. Convert the operator input into exactly one JSON object.
Do not include prose, Markdown, or code fences.

Known agents:
{roster}
Attached agent: {attached}

Allowed actions:
- {{\"action\":\"dashboard\",\"summary\":\"...\"}}
- {{\"action\":\"list_agents\",\"summary\":\"...\"}}
- {{\"action\":\"status\",\"agent\":\"AGENT\",\"summary\":\"...\"}}
- {{\"action\":\"recent_work\",\"agent\":\"AGENT\",\"summary\":\"...\"}}
- {{\"action\":\"results_evidence\",\"summary\":\"...\"}}
- {{\"action\":\"results_evidence\",\"agent\":\"AGENT\",\"summary\":\"...\"}}
- {{\"action\":\"details\",\"agent\":\"AGENT\",\"summary\":\"...\"}}
- {{\"action\":\"tool_activity\",\"agent\":\"AGENT\",\"summary\":\"...\"}}
- {{\"action\":\"evidence\",\"agent\":\"AGENT\",\"summary\":\"...\"}}
- {{\"action\":\"review_needs\",\"agent\":\"AGENT\",\"summary\":\"...\"}}
- {{\"action\":\"attach\",\"agent\":\"AGENT\",\"summary\":\"...\"}}
- {{\"action\":\"send\",\"agent\":\"AGENT\",\"message\":\"MESSAGE\",\"summary\":\"...\"}}
- {{\"action\":\"interrupt\",\"agent\":\"AGENT\",\"reason\":\"REASON\",\"summary\":\"...\"}}
- {{\"action\":\"stop\",\"agent\":\"AGENT\",\"reason\":\"REASON\",\"summary\":\"...\"}}
- {{\"action\":\"help\",\"summary\":\"...\"}}
- {{\"action\":\"unknown\",\"message\":\"ask the operator for a clearer request\",\"summary\":\"...\"}}

Rules:
- Use only listed agents for agent-specific actions.
- Do not invent agents.
- For casual greetings or vague prompts like \"what's up?\", choose dashboard.
- For requests for agent responses, results, replies, output, or evidence, choose results_evidence. Omit agent to inspect all agents.
- For follow-ups that say \"their\" or \"each\", choose an all-agent action by omitting agent.
- For requests to tell, ask, instruct, or have an agent do work, choose send.
- If a mutation is unclear or unsafe, choose unknown.

Operator input: {line}
",
            attached = self.attached_agent.as_deref().unwrap_or("none")
        ))
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
            OperatorAction::ResultsAndEvidence { agent } => self.results_and_evidence(agent),
            OperatorAction::Thread { agent } => self.thread(agent),
            OperatorAction::Details { agent } => self.details(agent),
            OperatorAction::ToolActivity { agent } => self.tool_activity(agent),
            OperatorAction::Evidence { agent } => self.evidence(agent),
            OperatorAction::ReviewNeeds { agent } => self.review_needs(agent),
            OperatorAction::Attach { agent } => self.attach(agent),
            OperatorAction::Detach => {
                self.attached_agent = None;
                Ok("detached\n".to_string())
            }
            OperatorAction::StartAgent {
                adapter,
                agent,
                goal,
            } => self.start_agent(adapter, agent, goal),
            OperatorAction::Send { agent, message } => self.send(agent, message),
            OperatorAction::Interrupt { agent, reason } => self.interrupt(agent, reason),
            OperatorAction::Stop { agent, reason } => self.stop(agent, reason),
        }
    }

    fn list_agents(&mut self) -> Result<String, String> {
        let agents = self.list_agent_summaries()?;
        let mut output = format!("Agents ({})\n", agents.len());
        for agent in agents {
            output.push_str(&self.render_agent_line(&agent));
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
        Ok(format!(
            "Status\n{}{}",
            self.render_agent_line(&summary),
            render_recent_work(&summary)
        ))
    }

    fn recent_work(&mut self, agent: Option<String>) -> Result<String, String> {
        self.read_agent_or_all(agent, RecentWorkRenderer)
    }

    fn results_and_evidence(&mut self, agent: Option<String>) -> Result<String, String> {
        self.read_agent_or_all(agent, ResultsAndEvidenceRenderer)
    }

    /// Render the agent's multi-turn conversation thread (ST5): the read model
    /// projected from the event log, replacing the single `latest_summary`. The
    /// thread is read incrementally from sequence 0 and rendered turn-by-turn;
    /// the client never authors thread ordering.
    fn thread(&mut self, agent: Option<String>) -> Result<String, String> {
        let summary = self.resolved_agent_status(agent)?;
        let Some(session) = summary.session.as_ref() else {
            return Ok(format!("{} has no active session.\n", summary.name));
        };
        let response = self.send_request(ServerCommand::ReadThread {
            session_id: session.session_id.to_string(),
            from_sequence: 0,
        })?;
        let ServerResponsePayload::Thread(thread) = response else {
            return Err("server returned unexpected response for thread".to_string());
        };
        Ok(render_thread(&summary.name, &thread))
    }

    fn details(&mut self, agent: Option<String>) -> Result<String, String> {
        self.read_agent_or_all(agent, DetailsRenderer)
    }

    fn tool_activity(&mut self, agent: Option<String>) -> Result<String, String> {
        self.read_agent_or_all(agent, ToolActivityRenderer)
    }

    fn evidence(&mut self, agent: Option<String>) -> Result<String, String> {
        self.read_agent_or_all(agent, EvidenceRenderer)
    }

    fn review_needs(&mut self, agent: Option<String>) -> Result<String, String> {
        self.read_agent_or_all(agent, ReviewNeedsRenderer)
    }

    fn attach(&mut self, agent: String) -> Result<String, String> {
        let summary = self.agent_status(&agent)?;
        self.attached_agent = Some(agent.clone());
        Ok(format!(
            "Attached to {agent}.\n{}",
            self.render_agent_result(&summary, ConciseResultRenderer)
        ))
    }

    fn start_agent(
        &mut self,
        adapter: String,
        agent: String,
        goal: String,
    ) -> Result<String, String> {
        let adapter = normalized_start_adapter(&adapter)?;
        let goal = strip_surrounding_quotes(&goal).to_string();
        if adapter == "codex" {
            require_codex_live_opt_in()?;
        }
        self.ensure_agent_registered(&agent)?;
        let started = self.start_agent_session(&agent, &adapter, &goal)?;
        let mut output = String::new();
        if adapter == "codex" {
            let run = self.run_codex_live_turn(
                &agent,
                &goal,
                &started.session_id.to_string(),
                &started.run_id.to_string(),
            )?;
            output.push_str(&self.render_live_codex_result(&agent, &run));
        } else {
            output.push_str(&format!("Started {adapter} agent `{agent}`.\n"));
        }
        self.attached_agent = Some(agent.clone());
        let summary = self.agent_status(&agent)?;
        output.push_str(&format!("Attached to {agent}.\n"));
        if adapter != "codex" {
            output.push_str(&self.render_agent_result(&summary, ConciseResultRenderer));
        }
        Ok(output)
    }

    fn send(&mut self, agent: Option<String>, message: String) -> Result<String, String> {
        let agent = self.resolve_agent(agent)?;
        let message = strip_surrounding_quotes(&message).to_string();
        let current = self.agent_status(&agent)?;
        if current
            .session
            .as_ref()
            .and_then(|session| session.adapter_kind.as_deref())
            == Some("codex_exec")
        {
            require_codex_live_opt_in()?;
            let session = current
                .session
                .as_ref()
                .ok_or_else(|| format!("agent {agent} has no active session"))?;
            let run_id = session
                .run_id
                .as_ref()
                .ok_or_else(|| format!("agent {agent} has no active run"))?
                .to_string();
            let run = self.run_codex_live_turn(
                &agent,
                &message,
                &session.session_id.to_string(),
                &run_id,
            )?;
            return Ok(format!(
                "Sent to {agent}.\n{}",
                self.render_live_codex_result(&agent, &run)
            ));
        }
        let response = self.send_request(ServerCommand::SteerAgent {
            agent_name: agent.clone(),
            goal: message,
        })?;
        let ServerResponsePayload::AgentStatus(summary) = response else {
            return Err("server returned unexpected response for agent mutation".to_string());
        };
        Ok(format!(
            "Sent to {agent}.\n{}",
            self.render_agent_result(&summary, ConciseResultRenderer)
        ))
    }

    fn render_live_codex_result(&self, agent: &str, run: &DispatchRunSummary) -> String {
        latest_codex_reply_from_artifact(&self.state_root, run)
            .map(|reply| render_agent_result_body(agent, &reply))
            .unwrap_or_else(|| {
                if run.status == "exited" {
                    format!("{agent}: reply captured; use `details` for artifact metadata.\n")
                } else {
                    format!("{agent}: Codex run did not finish cleanly.\n")
                }
            })
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

    fn read_agent_or_all<R: AgentRenderer>(
        &mut self,
        agent: Option<String>,
        renderer: R,
    ) -> Result<String, String> {
        if agent.is_some() || self.attached_agent.is_some() {
            return self
                .resolved_agent_status(agent)
                .map(|summary| renderer.render(&summary));
        }
        let agents = self.list_agent_summaries()?;
        if agents.is_empty() {
            return Ok("no agents\n".to_string());
        }
        let mut output = String::new();
        for agent in agents {
            output.push_str(&renderer.render(&agent));
        }
        Ok(output)
    }

    fn render_agent_result<R: AgentResultRenderer>(
        &self,
        agent: &AgentSummary,
        renderer: R,
    ) -> String {
        renderer.render_result(agent)
    }

    fn list_agent_summaries(&mut self) -> Result<Vec<AgentSummary>, String> {
        let response = self.send_request(ServerCommand::ListAgents)?;
        let ServerResponsePayload::Agents(agents) = response else {
            return Err("server returned unexpected response for agents".to_string());
        };
        Ok(agents)
    }

    fn ensure_agent_registered(&mut self, agent: &str) -> Result<(), String> {
        if self
            .list_agent_summaries()?
            .iter()
            .any(|summary| summary.name == agent)
        {
            return Ok(());
        }
        let response = self.send_request(ServerCommand::RegisterAgent {
            name: agent.to_string(),
        })?;
        if matches!(response, ServerResponsePayload::AgentRegistered(_)) {
            Ok(())
        } else {
            Err("server returned unexpected response for agent registration".to_string())
        }
    }

    fn start_agent_session(
        &mut self,
        agent: &str,
        adapter: &str,
        goal: &str,
    ) -> Result<TaskRunSummary, String> {
        let response = self.send_request(ServerCommand::StartSession {
            agent_name: agent.to_string(),
            goal: goal.to_string(),
            adapter: adapter.to_string(),
            session_id: None,
            run_id: None,
        })?;
        let ServerResponsePayload::SessionStarted(started) = response else {
            return Err("server returned unexpected response for session start".to_string());
        };
        Ok(started)
    }

    fn run_codex_live_turn(
        &mut self,
        agent: &str,
        goal: &str,
        session_id: &str,
        run_id: &str,
    ) -> Result<capo_server::DispatchRunSummary, String> {
        self.run_codex_live_turn_with_options(agent, goal, session_id, run_id, None, 300)
    }

    fn run_codex_live_turn_with_options(
        &mut self,
        agent: &str,
        goal: &str,
        session_id: &str,
        run_id: &str,
        mock_provider_output_jsonl: Option<&str>,
        timeout_seconds: u64,
    ) -> Result<capo_server::DispatchRunSummary, String> {
        let turn_id = format!("turn-{}-{}", slug(agent), stable_cli_hash(goal));
        let workspace = std::env::current_dir()
            .map_err(debug_error)?
            .display()
            .to_string();
        let artifacts = self
            .state_root
            .join("control-live-artifacts")
            .display()
            .to_string();
        let preflight = self.send_request(ServerCommand::PreflightLiveProvider {
            agent_name: agent.to_string(),
            adapter: "codex".to_string(),
            goal: goal.to_string(),
            workspace,
            artifacts,
            session_id: session_id.to_string(),
            run_id: run_id.to_string(),
            turn_id,
            capability_profile: "trusted-local".to_string(),
            runtime_scope: "local_process_loopback".to_string(),
            credential_scan_policy: "metadata_only_no_secret_read".to_string(),
            raw_prompt_policy: "not_rendered".to_string(),
            raw_output_policy: "artifacts_scanned_redacted".to_string(),
            tool_wrapper_policy: "capo_wrapped_required".to_string(),
            live_provider_opt_in: true,
        })?;
        let ServerResponsePayload::LiveProviderPreflighted(preflight) = preflight else {
            return Err("server returned unexpected response for Codex preflight".to_string());
        };
        let run = self.send_request(ServerCommand::RunLiveProviderLocal {
            dispatch_plan_id: preflight.dispatch_plan_id,
            goal: goal.to_string(),
            live_execution_opt_in: mock_provider_output_jsonl.is_none(),
            mock_runtime_opt_in: mock_provider_output_jsonl.is_some(),
            mock_provider_output_name: mock_provider_output_jsonl
                .map(|_| "capo-control-planner-mock-codex.jsonl".to_string()),
            mock_provider_output_jsonl: mock_provider_output_jsonl.map(ToString::to_string),
            timeout_seconds,
            // Ops set the spawn-path codex binary via `CAPO_CODEX_BIN`, resolved
            // server-side; the operator-control flow passes no explicit override.
            codex_program_override: None,
            // The operator-control planner flow stays read-only/dry-run: a live
            // workspace write goes through the attended `dispatch run-live` path.
            unattended: true,
        })?;
        let ServerResponsePayload::DispatchRun(run) = run else {
            return Err("server returned unexpected response for Codex live run".to_string());
        };
        Ok(run)
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

    fn render_agent_line(&self, agent: &AgentSummary) -> String {
        let marker =
            (self.attached_agent.as_deref() == Some(agent.name.as_str())).then_some("(attached)");
        render_human_agent_with_marker(agent, marker)
    }

    fn prompt(&self) -> String {
        match self.attached_agent.as_deref() {
            Some(agent) => format!("capo[{agent}]> "),
            None => "capo> ".to_string(),
        }
    }

    fn should_direct_send(&self, line: &str) -> bool {
        self.attached_agent.is_some() && !looks_like_control_command(line)
    }

    fn render_banner(&self) -> String {
        if self.server_started {
            "Capo\nserver started\nType `help` for commands.\n".to_string()
        } else {
            "Capo\nType `help` for commands.\n".to_string()
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
  state [AGENT] | result [AGENT]
  results [AGENT] | responses [AGENT]
  thread [AGENT] | conversation [AGENT]
  recent [AGENT] | work [AGENT]
  details [AGENT]
  tools [AGENT]
  evidence [AGENT]
  reviews [AGENT]
  new codex AGENT GOAL | start codex AGENT GOAL
  attach AGENT | jump AGENT
  send [--agent AGENT] MESSAGE
  interrupt [--agent AGENT] REASON
  stop [--agent AGENT] REASON
  detach | back
  help
  quit | exit

When attached, ordinary text is sent directly to the attached agent.
Run `capo control --planner capo` for the tracked `capo-operator` agent. It uses Codex as the first LLM planner backend, validates the selected action, and executes only known server-backed actions. `CAPO_CONTROL_PLANNER_PROVIDER=codex` is the current/default provider; future providers such as local Gemma can implement the same boundary.
Codex agents require starting control with CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT=1 and CAPO_SERVER_RUN_CODEX_LIVE=1.
Use `details` when you need session ids, run ids, dispatch ids, and raw policy metadata.
"
    .to_string()
}

fn latest_codex_reply_from_artifact(state_root: &Path, run: &DispatchRunSummary) -> Option<String> {
    if !run.provider_cli_executed || run.credential_scan_status != "clean" {
        return None;
    }
    let stdout = state_root
        .join("control-live-artifacts")
        .join(run.run_id.as_str())
        .join("stdout.txt");
    let content = std::fs::read_to_string(stdout).ok()?;
    latest_codex_reply_from_jsonl(&content)
}

fn latest_codex_reply_from_jsonl(content: &str) -> Option<String> {
    let parsed = CodexExecAdapter::parse_jsonl(content).ok()?;
    parsed
        .deduped_by_idempotency()
        .into_iter()
        .rev()
        .filter(|event| event.role.as_deref() == Some("assistant"))
        .filter_map(|event| event.content)
        .find(|content| !content.trim().is_empty())
        .map(|content| display_text(&content, 2_000))
}

fn strip_surrounding_quotes(value: &str) -> &str {
    let trimmed = value.trim();
    if trimmed.len() >= 2
        && ((trimmed.starts_with('"') && trimmed.ends_with('"'))
            || (trimmed.starts_with('\'') && trimmed.ends_with('\'')))
    {
        &trimmed[1..trimmed.len() - 1]
    } else {
        trimmed
    }
}

fn normalized_start_adapter(adapter: &str) -> Result<String, String> {
    match adapter {
        "codex" | "codex-exec" | "codex_exec" => Ok("codex".to_string()),
        other => Err(format!(
            "unsupported start adapter: {other}; currently only `codex` is supported"
        )),
    }
}

fn require_codex_live_opt_in() -> Result<(), String> {
    let preflight = std::env::var("CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT").as_deref() == Ok("1");
    let run = std::env::var("CAPO_SERVER_RUN_CODEX_LIVE").as_deref() == Ok("1");
    if preflight && run {
        return Ok(());
    }
    Err("Codex live execution from control requires starting capo control with CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT=1 and CAPO_SERVER_RUN_CODEX_LIVE=1".to_string())
}

fn slug(value: &str) -> String {
    let mut slug = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    slug.trim_matches('-').to_string()
}

fn looks_like_control_command(line: &str) -> bool {
    let command = line.split_whitespace().next().unwrap_or_default();
    matches!(
        command,
        "help"
            | "?"
            | "quit"
            | "exit"
            | "agents"
            | "ls"
            | "dashboard"
            | "overview"
            | "recent"
            | "work"
            | "details"
            | "debug"
            | "state"
            | "result"
            | "results"
            | "responses"
            | "tools"
            | "evidence"
            | "reviews"
            | "detach"
            | "back"
            | "attach"
            | "jump"
            | "new"
            | "start"
            | "status"
            | "send"
            | "interrupt"
            | "stop"
    )
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
    fn unknown_control_flags_are_rejected_before_defaulting_planner() {
        let error = reject_unknown_control_flags(&["--planer".to_string(), "capo".to_string()])
            .expect_err("unknown flag should fail");
        assert!(error.contains("unknown control option: --planer"));
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

    #[test]
    fn codex_reply_renderer_reads_scanned_stdout_artifact_without_state_summary() {
        let root = std::env::temp_dir().join(format!(
            "capo-codex-reply-render-{}",
            stable_cli_hash("codex-reply-render")
        ));
        let run_id = capo_core::RunId::new("run-codex-render");
        let stdout = root
            .join("control-live-artifacts")
            .join(run_id.as_str())
            .join("stdout.txt");
        std::fs::create_dir_all(stdout.parent().expect("stdout parent")).expect("mkdir");
        std::fs::write(
            &stdout,
            r#"{"type":"thread.started","thread_id":"codex-thread-render"}
{"type":"item.completed","thread_id":"codex-thread-render","item":{"id":"codex-item-render","role":"assistant","content":[{"type":"output_text","text":"| Number | Double |\n|---:|---:|\n| 1 | 2 |\n| 2 | 4 |\n"}]}}
{"type":"turn.completed","thread_id":"codex-thread-render"}
"#,
        )
        .expect("write stdout");
        let run = DispatchRunSummary {
            dispatch_plan_id: "plan-codex-render".to_string(),
            dispatch_execution_id: "execution-codex-render".to_string(),
            adapter: "codex_exec".to_string(),
            session_id: capo_core::SessionId::new("session-codex-render"),
            run_id,
            provider_cli_execution_allowed: true,
            provider_cli_executed: true,
            status: "exited".to_string(),
            runtime_process_ref: Some("process-codex-render".to_string()),
            credential_scan_status: "clean".to_string(),
            raw_prompt_policy: "not_rendered".to_string(),
            raw_output_policy: "bounded_redacted_artifacts".to_string(),
            reason_codes: "provider_cli_executed_and_artifacts_scanned".to_string(),
            input_event_count: 3,
            appended_event_count: 3,
            tool_event_count: 0,
            summary_event_count: 1,
            completed_turn_count: 1,
            observed_token_cost: None,
        };

        assert_eq!(
            latest_codex_reply_from_artifact(&root, &run).as_deref(),
            Some("| Number | Double |\n|---:|---:|\n| 1 | 2 |\n| 2 | 4 |")
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
