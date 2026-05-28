use capo_server::{AgentSummary, ServerDashboardSnapshot};

pub(super) trait AgentRenderer {
    fn render(&self, agent: &AgentSummary) -> String;
}

#[derive(Clone, Copy)]
pub(super) struct RecentWorkRenderer;

#[derive(Clone, Copy)]
pub(super) struct DetailsRenderer;

#[derive(Clone, Copy)]
pub(super) struct ToolActivityRenderer;

#[derive(Clone, Copy)]
pub(super) struct EvidenceRenderer;

#[derive(Clone, Copy)]
pub(super) struct ReviewNeedsRenderer;

pub(super) fn render_dashboard(snapshot: &ServerDashboardSnapshot) -> String {
    let mut output = format!(
        "Dashboard\nagents: {}\nactive: {}\n",
        snapshot.agent_count, snapshot.active_session_count
    );
    for agent in &snapshot.agents {
        output.push_str(&render_human_agent(agent));
    }
    output
}

pub(super) fn render_human_agent(agent: &AgentSummary) -> String {
    render_human_agent_with_marker(agent, None)
}

pub(super) fn render_human_agent_with_marker(agent: &AgentSummary, marker: Option<&str>) -> String {
    let mut output = format!("- {}", agent.name);
    if let Some(marker) = marker {
        output.push_str(&format!(" {marker}"));
    }
    if let Some(session) = agent.session.as_ref() {
        output.push_str(&format!(
            " - {}",
            user_status(
                session.run_status.as_deref(),
                session.dispatch_execution_status.as_deref()
            )
        ));
        let activity = activity_summary(
            session.tool_call_count,
            session.memory_packet_count,
            session.evidence_count,
            session.review_finding_count,
        );
        if activity != "idle" {
            output.push_str(&format!(" ({activity})"));
        }
    } else {
        output.push_str(" - idle");
    }
    output.push('\n');
    output
}

impl AgentRenderer for RecentWorkRenderer {
    fn render(&self, agent: &AgentSummary) -> String {
        render_recent_work(agent)
    }
}

impl AgentRenderer for DetailsRenderer {
    fn render(&self, agent: &AgentSummary) -> String {
        render_agent_details(agent)
    }
}

impl AgentRenderer for ToolActivityRenderer {
    fn render(&self, agent: &AgentSummary) -> String {
        render_tool_activity(agent)
    }
}

impl AgentRenderer for EvidenceRenderer {
    fn render(&self, agent: &AgentSummary) -> String {
        render_evidence_summary(agent)
    }
}

impl AgentRenderer for ReviewNeedsRenderer {
    fn render(&self, agent: &AgentSummary) -> String {
        render_review_needs(agent)
    }
}

pub(super) fn render_recent_work(agent: &AgentSummary) -> String {
    let Some(session) = agent.session.as_ref() else {
        return format!("{} has no active session.\n", agent.name);
    };
    let mut output = format!(
        "{}\nstatus: {}\ngoal: {}\n",
        agent.name,
        user_status(
            session.run_status.as_deref(),
            session.dispatch_execution_status.as_deref()
        ),
        display_goal(&session.current_goal)
    );
    if let Some(reply) = display_summary(session.latest_summary.as_deref()) {
        output.push_str(&format!("reply: {reply}\n"));
    } else if session.dispatch_provider_cli_executed == Some(true) {
        output.push_str("reply: captured; use `details` for artifact metadata.\n");
    } else {
        output.push_str("reply: none yet\n");
    }
    if let Some(blocker) = session.latest_blocker.as_deref() {
        output.push_str(&format!("blocker: {blocker}\n"));
    }
    output.push_str(&format!(
        "activity: {}\n",
        activity_summary(
            session.tool_call_count,
            session.memory_packet_count,
            session.evidence_count,
            session.review_finding_count,
        )
    ));
    output
}

pub(super) fn render_agent_reply(agent: &AgentSummary) -> String {
    let Some(session) = agent.session.as_ref() else {
        return format!("{} has no active session.\n", agent.name);
    };
    if let Some(reply) = display_summary(session.latest_summary.as_deref()) {
        return format!("{}: {reply}\n", agent.name);
    }
    if session.dispatch_provider_cli_executed == Some(true) {
        return format!(
            "{}: reply captured; use `details` for artifact metadata.\n",
            agent.name
        );
    }
    format!(
        "{}: {}\n",
        agent.name,
        user_status(
            session.run_status.as_deref(),
            session.dispatch_execution_status.as_deref()
        )
    )
}

pub(super) fn render_agent_details(agent: &AgentSummary) -> String {
    let Some(session) = agent.session.as_ref() else {
        return format!("Details\n- {} has no active session\n", agent.name);
    };
    format!(
        "Details\n- agent: {}\n- session: {}\n- run: {}\n- run_status: {}\n- adapter: {}\n- goal: {}\n- summary: {}\n- confidence: {}\n- blocker: {}\n- turns: {}\n- recent events: {}\n- dispatch: plan={} gate={} execution={} execution_status={} cli_executed={} raw_output_policy={}\n",
        agent.name,
        session.session_id,
        session
            .run_id
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_else(|| "none".to_string()),
        session.run_status.as_deref().unwrap_or("none"),
        session.adapter_kind.as_deref().unwrap_or("none"),
        session.current_goal,
        session.latest_summary.as_deref().unwrap_or("none"),
        session
            .latest_confidence
            .map(|confidence| confidence.to_string())
            .unwrap_or_else(|| "none".to_string()),
        session.latest_blocker.as_deref().unwrap_or("none"),
        none_if_empty(&session.turn_ids),
        session.recent_event_count,
        session.latest_dispatch_plan_id.as_deref().unwrap_or("none"),
        session.latest_dispatch_gate_id.as_deref().unwrap_or("none"),
        session
            .latest_dispatch_execution_id
            .as_deref()
            .unwrap_or("none"),
        session
            .dispatch_execution_status
            .as_deref()
            .unwrap_or("none"),
        session
            .dispatch_provider_cli_executed
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_string()),
        session
            .dispatch_raw_output_policy
            .as_deref()
            .unwrap_or("none")
    )
}

pub(super) fn render_tool_activity(agent: &AgentSummary) -> String {
    let Some(session) = agent.session.as_ref() else {
        return format!("Tool activity\n- {} has no active session\n", agent.name);
    };
    format!(
        "Tool activity\n{}: {} tool calls, {} observations, {} memory packets\n",
        agent.name,
        session.tool_call_count,
        session.tool_observation_count,
        session.memory_packet_count
    )
}

pub(super) fn render_evidence_summary(agent: &AgentSummary) -> String {
    let Some(session) = agent.session.as_ref() else {
        return format!("Evidence\n- {} has no active session\n", agent.name);
    };
    format!(
        "Evidence\n{}: {}\n",
        agent.name,
        none_if_empty(&session.evidence_refs)
    )
}

pub(super) fn render_review_needs(agent: &AgentSummary) -> String {
    let Some(session) = agent.session.as_ref() else {
        return format!("Reviews\n- {} has no active session\n", agent.name);
    };
    format!(
        "Reviews\n{}: {} review findings, {} task outcome reports\n",
        agent.name, session.review_finding_count, session.task_outcome_report_count
    )
}

fn none_if_empty(values: &[String]) -> String {
    if values.is_empty() {
        "none".to_string()
    } else {
        values.join(",")
    }
}

fn user_status(run_status: Option<&str>, dispatch_status: Option<&str>) -> String {
    match (dispatch_status, run_status) {
        (Some("exited"), _) | (_, Some("exited")) => "finished".to_string(),
        (Some("blocked_sensitive_artifact"), _) => "blocked by artifact scan".to_string(),
        (Some(status), _) => status.replace('_', " "),
        (_, Some("running")) => "running".to_string(),
        (_, Some(status)) => status.replace('_', " "),
        _ => "started".to_string(),
    }
}

fn display_goal(goal: &str) -> String {
    if goal.starts_with("goal_hash:") {
        "not shown".to_string()
    } else {
        compact_one_line(goal)
    }
}

fn display_summary(summary: Option<&str>) -> Option<String> {
    let summary = summary?.trim();
    if summary.is_empty() {
        return None;
    }
    if summary.starts_with("Adapter ") && summary.contains("content_hash=") {
        return None;
    }
    Some(compact_one_line(summary))
}

fn compact_one_line(value: &str) -> String {
    let compact = value.split_whitespace().collect::<Vec<_>>().join(" ");
    const MAX_CHARS: usize = 500;
    if compact.chars().count() <= MAX_CHARS {
        return compact;
    }
    let mut shortened = compact.chars().take(MAX_CHARS).collect::<String>();
    shortened.push_str("...");
    shortened
}

fn activity_summary(
    tool_calls: usize,
    memory_packets: usize,
    evidence: usize,
    reviews: usize,
) -> String {
    let mut parts = Vec::new();
    if tool_calls > 0 {
        parts.push(format!("{tool_calls} tools"));
    }
    if memory_packets > 0 {
        parts.push(format!("{memory_packets} memories"));
    }
    if evidence > 0 {
        parts.push(format!("{evidence} evidence"));
    }
    if reviews > 0 {
        parts.push(format!("{reviews} reviews"));
    }
    if parts.is_empty() {
        "idle".to_string()
    } else {
        parts.join(", ")
    }
}
