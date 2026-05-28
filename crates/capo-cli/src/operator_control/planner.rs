use serde_json::Value;

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
    ResultsAndEvidence {
        agent: Option<String>,
    },
    Details {
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
    StartAgent {
        adapter: String,
        agent: String,
        goal: String,
    },
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

pub(super) fn plan_from_llm_reply(reply: &str) -> Result<PlannerDecision, String> {
    let json = extract_json_object(reply)
        .ok_or_else(|| "capo planner LLM did not return a JSON action object".to_string())?;
    let value = serde_json::from_str::<Value>(json)
        .map_err(|error| format!("capo planner LLM returned invalid JSON: {error}"))?;
    let action = value
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let summary = value
        .get("summary")
        .and_then(Value::as_str)
        .unwrap_or("LLM planner selected a server-backed operator action");
    match action {
        "help" => Ok(PlannerDecision::audited(OperatorAction::Help, summary)),
        "list_agents" => Ok(PlannerDecision::audited(
            OperatorAction::ListAgents,
            summary,
        )),
        "dashboard" => Ok(PlannerDecision::audited(OperatorAction::Dashboard, summary)),
        "status" => Ok(PlannerDecision::audited(
            OperatorAction::Status {
                agent: optional_field(&value, "agent"),
            },
            summary,
        )),
        "recent_work" | "recent" | "result" => Ok(PlannerDecision::audited(
            OperatorAction::RecentWork {
                agent: optional_field(&value, "agent"),
            },
            summary,
        )),
        "results_evidence" | "responses_evidence" | "agent_results" => {
            Ok(PlannerDecision::audited(
                OperatorAction::ResultsAndEvidence {
                    agent: optional_field(&value, "agent"),
                },
                summary,
            ))
        }
        "details" => Ok(PlannerDecision::audited(
            OperatorAction::Details {
                agent: optional_field(&value, "agent"),
            },
            summary,
        )),
        "tool_activity" | "tools" => Ok(PlannerDecision::audited(
            OperatorAction::ToolActivity {
                agent: optional_field(&value, "agent"),
            },
            summary,
        )),
        "evidence" => Ok(PlannerDecision::audited(
            OperatorAction::Evidence {
                agent: optional_field(&value, "agent"),
            },
            summary,
        )),
        "review_needs" | "reviews" => Ok(PlannerDecision::audited(
            OperatorAction::ReviewNeeds {
                agent: optional_field(&value, "agent"),
            },
            summary,
        )),
        "attach" => Ok(PlannerDecision::audited(
            OperatorAction::Attach {
                agent: required_field(&value, "agent", action)?,
            },
            summary,
        )),
        "send" => Ok(PlannerDecision::audited(
            OperatorAction::Send {
                agent: Some(required_field(&value, "agent", action)?),
                message: required_field(&value, "message", action)?,
            },
            summary,
        )),
        "interrupt" => Ok(PlannerDecision::audited(
            OperatorAction::Interrupt {
                agent: optional_field(&value, "agent"),
                reason: required_field(&value, "reason", action)?,
            },
            summary,
        )),
        "stop" => Ok(PlannerDecision::audited(
            OperatorAction::Stop {
                agent: optional_field(&value, "agent"),
                reason: required_field(&value, "reason", action)?,
            },
            summary,
        )),
        "unknown" => Err(value
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("capo planner could not map that input")
            .to_string()),
        other => Err(format!(
            "capo planner LLM selected unsupported action: {other}"
        )),
    }
}

fn extract_json_object(value: &str) -> Option<&str> {
    let start = value.find('{')?;
    let end = value.rfind('}')?;
    (end >= start).then_some(&value[start..=end])
}

fn optional_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn required_field(value: &Value, key: &str, action: &str) -> Result<String, String> {
    optional_field(value, key).ok_or_else(|| {
        format!("capo planner LLM action `{action}` is missing required field `{key}`")
    })
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
        "details" | "debug" => Ok(OperatorAction::Details {
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
        "new" | "start" => parse_start_agent(line, command),
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
        "state" | "result" => Ok(OperatorAction::RecentWork {
            agent: parts.next().map(ToString::to_string),
        }),
        "results" | "responses" => Ok(OperatorAction::ResultsAndEvidence {
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

fn parse_start_agent(line: &str, command: &str) -> Result<OperatorAction, String> {
    let rest = line
        .strip_prefix(command)
        .expect("parse_start_agent is only called for matching commands")
        .trim();
    let mut split = rest.splitn(3, char::is_whitespace);
    let adapter = split
        .next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{command} requires an adapter, agent name, and goal"))?;
    let agent = split
        .next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{command} requires an agent name and goal"))?;
    let goal = split
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{command} requires a goal"))?;
    Ok(OperatorAction::StartAgent {
        adapter: adapter.to_string(),
        agent: agent.to_string(),
        goal: goal.to_string(),
    })
}

fn parse_capo_intent(line: &str) -> Result<PlannerDecision, String> {
    let line = line.trim();
    let normalized = line.to_ascii_lowercase();
    match normalized.as_str() {
        "what happened?"
        | "what happened"
        | "what has happened?"
        | "what has happened"
        | "what's happened?"
        | "what's happened"
        | "what are my agents doing?"
        | "what are my agents doing"
        | "what are the agents doing?"
        | "what are the agents doing" => {
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
    if let Some(agent) = line.strip_prefix("summarize ") {
        let agent = agent.trim();
        if !agent.is_empty() {
            return Ok(PlannerDecision::audited(
                OperatorAction::RecentWork {
                    agent: Some(agent.to_string()),
                },
                "operator asked for one agent's recent work",
            ));
        }
    }
    if let Some(after_steer) = line.strip_prefix("steer ") {
        return parse_capo_steer(after_steer);
    }
    if let Some(after_tell) = line
        .strip_prefix("tell ")
        .or_else(|| line.strip_prefix("ask "))
    {
        return parse_capo_steer(after_tell);
    }
    Err("capo planner could not map that input; try `help`, `what are my agents doing?`, `what is blocked?`, `summarize AGENT`, or `tell AGENT to ...`".to_string())
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
            Self::ResultsAndEvidence { .. } => "results_evidence",
            Self::Details { .. } => "details",
            Self::ToolActivity { .. } => "tool_activity",
            Self::Evidence { .. } => "evidence",
            Self::ReviewNeeds { .. } => "review_needs",
            Self::Attach { .. } => "attach",
            Self::StartAgent { .. } => "start_agent",
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
            | Self::ResultsAndEvidence { agent }
            | Self::Details { agent }
            | Self::ToolActivity { agent }
            | Self::Evidence { agent }
            | Self::ReviewNeeds { agent }
            | Self::Send { agent, .. }
            | Self::Interrupt { agent, .. }
            | Self::Stop { agent, .. } => agent.clone(),
            Self::Attach { agent } => Some(agent.clone()),
            Self::StartAgent { agent, .. } => Some(agent.clone()),
            _ => None,
        }
    }

    fn is_mutation(&self) -> bool {
        matches!(
            self,
            Self::Attach { .. }
                | Self::StartAgent { .. }
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
        assert_eq!(
            parse_action("state demo"),
            Ok(OperatorAction::RecentWork {
                agent: Some("demo".to_string())
            })
        );
        assert_eq!(
            parse_action("details demo"),
            Ok(OperatorAction::Details {
                agent: Some("demo".to_string())
            })
        );
        assert_eq!(
            parse_action("result"),
            Ok(OperatorAction::RecentWork { agent: None })
        );
        assert_eq!(
            parse_action("results"),
            Ok(OperatorAction::ResultsAndEvidence { agent: None })
        );
        assert_eq!(
            parse_action("responses demo"),
            Ok(OperatorAction::ResultsAndEvidence {
                agent: Some("demo".to_string())
            })
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
    fn parser_supports_starting_adapter_agents() {
        assert_eq!(
            parse_action("new codex codex-demo Say hello"),
            Ok(OperatorAction::StartAgent {
                adapter: "codex".to_string(),
                agent: "codex-demo".to_string(),
                goal: "Say hello".to_string()
            })
        );
        assert_eq!(
            parse_action("start codex codex-demo Say hello"),
            Ok(OperatorAction::StartAgent {
                adapter: "codex".to_string(),
                agent: "codex-demo".to_string(),
                goal: "Say hello".to_string()
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

        let decision = planner
            .plan("tell mock-control to Run the checks")
            .expect("planned natural steering");
        assert_eq!(
            decision.action,
            OperatorAction::Send {
                agent: Some("mock-control".to_string()),
                message: "Run the checks".to_string()
            }
        );

        let decision = planner
            .plan("what are my agents doing?")
            .expect("planned agent overview");
        assert_eq!(decision.action, OperatorAction::Dashboard);

        let decision = planner
            .plan("summarize mock-control")
            .expect("planned recent work");
        assert_eq!(
            decision.action,
            OperatorAction::RecentWork {
                agent: Some("mock-control".to_string())
            }
        );
    }

    #[test]
    fn capo_planner_rejects_ambiguous_mutations() {
        let planner = CapoPlanner;
        let error = planner
            .plan("please make mock-control continue")
            .expect_err("ambiguous steering should fail closed");
        assert!(error.contains("could not map"));
    }

    #[test]
    fn llm_reply_maps_to_validated_operator_action() {
        let decision = plan_from_llm_reply(
            r#"{"action":"send","agent":"mock-control","message":"Please continue","summary":"operator asked mock-control to continue"}"#,
        )
        .expect("llm action");

        assert_eq!(
            decision.action,
            OperatorAction::Send {
                agent: Some("mock-control".to_string()),
                message: "Please continue".to_string()
            }
        );
        assert!(decision.audit.expect("audit").mutation);

        let decision = plan_from_llm_reply(
            r#"{"action":"results_evidence","summary":"operator asked for all agent responses and evidence"}"#,
        )
        .expect("llm all-agent result action");
        assert_eq!(
            decision.action,
            OperatorAction::ResultsAndEvidence { agent: None }
        );
    }

    #[test]
    fn llm_reply_rejects_unsupported_or_incomplete_action() {
        let error = plan_from_llm_reply(r#"{"action":"send","agent":"mock-control"}"#)
            .expect_err("missing message should fail");
        assert!(error.contains("missing required field `message`"));

        let error = plan_from_llm_reply(r#"{"action":"delete_everything"}"#)
            .expect_err("unsupported action should fail");
        assert!(error.contains("unsupported action"));
    }
}
