#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum OperatorAction {
    Help,
    Quit,
    ListAgents,
    Dashboard,
    Status {
        agent: Option<String>,
    },
    RecentWork {
        agent: Option<String>,
    },
    ToolActivity {
        agent: Option<String>,
    },
    Evidence {
        agent: Option<String>,
    },
    ReviewNeeds {
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
    Interrupt {
        agent: Option<String>,
        reason: String,
    },
    Stop {
        agent: Option<String>,
        reason: String,
    },
}

pub(super) trait Planner {
    fn mode(&self) -> &'static str;

    fn plan(&self, line: &str) -> Result<PlannerDecision, String>;
}

#[derive(Debug)]
pub(super) struct NonePlanner;

impl Planner for NonePlanner {
    fn mode(&self) -> &'static str {
        "none"
    }

    fn plan(&self, line: &str) -> Result<PlannerDecision, String> {
        parse_action(line).map(PlannerDecision::unaudited)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PlannerDecision {
    pub(super) action: OperatorAction,
    pub(super) audit: Option<PlannerDecisionAudit>,
}

impl PlannerDecision {
    fn unaudited(action: OperatorAction) -> Self {
        Self {
            action,
            audit: None,
        }
    }

    fn audited(action: OperatorAction, summary: impl Into<String>) -> Self {
        let action_label = action.audit_label();
        let target_agent = action.target_agent();
        let mutation = action.is_mutation();
        Self {
            action,
            audit: Some(PlannerDecisionAudit {
                summary: summary.into(),
                action_label,
                target_agent,
                mutation,
            }),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PlannerDecisionAudit {
    pub(super) summary: String,
    pub(super) action_label: &'static str,
    pub(super) target_agent: Option<String>,
    pub(super) mutation: bool,
}

#[derive(Debug)]
pub(super) struct CapoPlanner;

impl Planner for CapoPlanner {
    fn mode(&self) -> &'static str {
        "capo"
    }

    fn plan(&self, line: &str) -> Result<PlannerDecision, String> {
        if line.to_ascii_lowercase().starts_with("status of ") {
            return parse_capo_intent(line);
        }
        if let Ok(action) = parse_action(line) {
            return Ok(PlannerDecision::audited(
                action,
                "deterministic command parsed by Capo planner",
            ));
        }
        parse_capo_intent(line)
    }
}

fn parse_action(line: &str) -> Result<OperatorAction, String> {
    let mut parts = line.split_whitespace();
    let command = parts.next().unwrap_or_default();
    match command {
        "help" | "?" => Ok(OperatorAction::Help),
        "quit" | "exit" => Ok(OperatorAction::Quit),
        "agents" | "ls" => Ok(OperatorAction::ListAgents),
        "dashboard" | "overview" => Ok(OperatorAction::Dashboard),
        "recent" | "work" => Ok(OperatorAction::RecentWork {
            agent: parts.next().map(ToString::to_string),
        }),
        "tools" => Ok(OperatorAction::ToolActivity {
            agent: parts.next().map(ToString::to_string),
        }),
        "evidence" => Ok(OperatorAction::Evidence {
            agent: parts.next().map(ToString::to_string),
        }),
        "reviews" => Ok(OperatorAction::ReviewNeeds {
            agent: parts.next().map(ToString::to_string),
        }),
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
        "interrupt" => parse_reason_action(line, "interrupt", |agent, reason| {
            OperatorAction::Interrupt { agent, reason }
        }),
        "stop" => parse_reason_action(line, "stop", |agent, reason| OperatorAction::Stop {
            agent,
            reason,
        }),
        other => Err(format!("unknown command `{other}`")),
    }
}

fn parse_capo_intent(line: &str) -> Result<PlannerDecision, String> {
    let line = line.trim();
    let normalized = line.to_ascii_lowercase();
    match normalized.as_str() {
        "what happened?" | "what happened" | "what has happened?" | "what has happened"
        | "what's happened?" | "what's happened" => {
            return Ok(PlannerDecision::audited(
                OperatorAction::Dashboard,
                "operator asked for a recent state overview",
            ));
        }
        "what is blocked?" | "what is blocked" | "what's blocked?" | "what's blocked"
        | "what is stuck?" | "what is stuck" => {
            return Ok(PlannerDecision::audited(
                OperatorAction::ReviewNeeds { agent: None },
                "operator asked for blocked or review-needed work",
            ));
        }
        "list agents" | "show agents" | "who is running?" | "who is running" => {
            return Ok(PlannerDecision::audited(
                OperatorAction::ListAgents,
                "operator asked to list tracked agents",
            ));
        }
        _ => {}
    }
    if normalized.starts_with("status of ") {
        let agent = line["status of ".len()..].trim();
        if !agent.is_empty() {
            return Ok(PlannerDecision::audited(
                OperatorAction::Status {
                    agent: Some(agent.to_string()),
                },
                "operator asked for one agent status",
            ));
        }
    }
    if let Some(after_steer) = line.strip_prefix("steer ") {
        return parse_capo_steer(after_steer);
    }
    Err("capo planner could not map that input; try `help`, `what happened?`, `what is blocked?`, or `steer AGENT to ...`".to_string())
}

fn parse_capo_steer(after_steer: &str) -> Result<PlannerDecision, String> {
    let Some((agent, message)) = after_steer.split_once(" to ") else {
        return Err("safe planner steering requires `steer AGENT to MESSAGE`".to_string());
    };
    let agent = agent.trim();
    let message = message.trim();
    if agent.is_empty() || message.is_empty() {
        return Err("safe planner steering requires `steer AGENT to MESSAGE`".to_string());
    }
    Ok(PlannerDecision::audited(
        OperatorAction::Send {
            agent: Some(agent.to_string()),
            message: message.to_string(),
        },
        "operator explicitly requested steering with deterministic syntax",
    ))
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

fn parse_reason_action(
    line: &str,
    command: &str,
    build: impl Fn(Option<String>, String) -> OperatorAction,
) -> Result<OperatorAction, String> {
    let rest = line
        .strip_prefix(command)
        .expect("parse_reason_action is only called for matching commands")
        .trim();
    if rest.is_empty() {
        return Err(format!("{command} requires a reason"));
    }
    if let Some(after_flag) = rest.strip_prefix("--agent ") {
        let mut split = after_flag.splitn(2, char::is_whitespace);
        let agent = split
            .next()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("{command} --agent requires an agent name"))?;
        let reason = split
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("{command} requires a reason"))?;
        return Ok(build(Some(agent.to_string()), reason.to_string()));
    }
    Ok(build(None, rest.to_string()))
}

impl OperatorAction {
    fn audit_label(&self) -> &'static str {
        match self {
            Self::Help => "help",
            Self::Quit => "quit",
            Self::ListAgents => "list_agents",
            Self::Dashboard => "dashboard",
            Self::Status { .. } => "status",
            Self::RecentWork { .. } => "recent_work",
            Self::ToolActivity { .. } => "tool_activity",
            Self::Evidence { .. } => "evidence",
            Self::ReviewNeeds { .. } => "review_needs",
            Self::Attach { .. } => "attach",
            Self::Detach => "detach",
            Self::Send { .. } => "send",
            Self::Interrupt { .. } => "interrupt",
            Self::Stop { .. } => "stop",
        }
    }

    fn target_agent(&self) -> Option<String> {
        match self {
            Self::Status { agent }
            | Self::RecentWork { agent }
            | Self::ToolActivity { agent }
            | Self::Evidence { agent }
            | Self::ReviewNeeds { agent }
            | Self::Send { agent, .. }
            | Self::Interrupt { agent, .. }
            | Self::Stop { agent, .. } => agent.clone(),
            Self::Attach { agent } => Some(agent.clone()),
            _ => None,
        }
    }

    fn is_mutation(&self) -> bool {
        matches!(
            self,
            Self::Attach { .. }
                | Self::Detach
                | Self::Send { .. }
                | Self::Interrupt { .. }
                | Self::Stop { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn parser_supports_richer_read_commands() {
        assert_eq!(
            parse_action("recent demo"),
            Ok(OperatorAction::RecentWork {
                agent: Some("demo".to_string())
            })
        );
        assert_eq!(
            parse_action("tools"),
            Ok(OperatorAction::ToolActivity { agent: None })
        );
    }

    #[test]
    fn parser_supports_control_mutations_with_reasons() {
        assert_eq!(
            parse_action("interrupt --agent demo pause for review"),
            Ok(OperatorAction::Interrupt {
                agent: Some("demo".to_string()),
                reason: "pause for review".to_string()
            })
        );
        assert_eq!(
            parse_action("stop completed"),
            Ok(OperatorAction::Stop {
                agent: None,
                reason: "completed".to_string()
            })
        );
    }

    #[test]
    fn capo_planner_maps_simple_operator_intents() {
        let planner = CapoPlanner;
        let decision = planner.plan("what happened?").expect("planned overview");
        assert_eq!(decision.action, OperatorAction::Dashboard);
        assert!(decision.audit.expect("audit").summary.contains("overview"));

        let decision = planner
            .plan("what is blocked?")
            .expect("planned blocked query");
        assert_eq!(decision.action, OperatorAction::ReviewNeeds { agent: None });

        let decision = planner
            .plan("steer mock-control to Please continue")
            .expect("planned steering");
        assert_eq!(
            decision.action,
            OperatorAction::Send {
                agent: Some("mock-control".to_string()),
                message: "Please continue".to_string()
            }
        );
        assert!(decision.audit.expect("audit").mutation);
    }

    #[test]
    fn capo_planner_rejects_implicit_mutations() {
        let planner = CapoPlanner;
        let error = planner
            .plan("tell mock-control to continue")
            .expect_err("implicit steering should fail closed");
        assert!(error.contains("could not map"));
    }
}
