#[derive(Debug, Eq, PartialEq)]
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
    fn plan(&self, line: &str) -> Result<OperatorAction, String>;
}

#[derive(Debug)]
pub(super) struct NonePlanner;

impl Planner for NonePlanner {
    fn plan(&self, line: &str) -> Result<OperatorAction, String> {
        parse_action(line)
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
}
