use capo_server::{AgentSummary, ServerDashboardSnapshot};

pub(super) fn render_dashboard(snapshot: &ServerDashboardSnapshot) -> String {
    let mut output = format!(
        "Dashboard\nproject: {}\nagents: {}\nactive sessions: {}\n",
        snapshot.project_id, snapshot.agent_count, snapshot.active_session_count
    );
    for agent in &snapshot.agents {
        output.push_str(&render_human_agent(agent));
    }
    output
}

pub(super) fn render_human_agent(agent: &AgentSummary) -> String {
    let mut output = format!("- {} [{}]", agent.name, agent.status);
    if let Some(session) = agent.session.as_ref() {
        output.push_str(&format!(
            " session={} run={} run_status={} tools={} memory={} evidence={} reviews={}",
            session.session_id,
            session
                .run_id
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| "none".to_string()),
            session.run_status.as_deref().unwrap_or("none"),
            session.tool_call_count,
            session.memory_packet_count,
            session.evidence_count,
            session.review_finding_count
        ));
    } else {
        output.push_str(" session=none");
    }
    output.push('\n');
    output
}

pub(super) fn render_recent_work(agent: &AgentSummary) -> String {
    let Some(session) = agent.session.as_ref() else {
        return format!("Recent work\n- {} has no active session\n", agent.name);
    };
    format!(
        "Recent work\n- agent: {}\n- session: {}\n- run_status: {}\n- goal: {}\n- summary: {}\n- confidence: {}\n- blocker: {}\n- turns: {}\n- recent events: {}\n",
        agent.name,
        session.session_id,
        session.run_status.as_deref().unwrap_or("none"),
        session.current_goal,
        session.latest_summary.as_deref().unwrap_or("none"),
        session
            .latest_confidence
            .map(|confidence| confidence.to_string())
            .unwrap_or_else(|| "none".to_string()),
        session.latest_blocker.as_deref().unwrap_or("none"),
        none_if_empty(&session.turn_ids),
        session.recent_event_count
    )
}

pub(super) fn render_tool_activity(agent: &AgentSummary) -> String {
    let Some(session) = agent.session.as_ref() else {
        return format!("Tool activity\n- {} has no active session\n", agent.name);
    };
    format!(
        "Tool activity\n- agent: {}\n- session: {}\n- tool calls: {}\n- tool observations: {}\n- memory packets: {}\n",
        agent.name,
        session.session_id,
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
        "Evidence\n- agent: {}\n- session: {}\n- evidence refs: {}\n",
        agent.name,
        session.session_id,
        none_if_empty(&session.evidence_refs)
    )
}

pub(super) fn render_review_needs(agent: &AgentSummary) -> String {
    let Some(session) = agent.session.as_ref() else {
        return format!("Reviews\n- {} has no active session\n", agent.name);
    };
    format!(
        "Reviews\n- agent: {}\n- session: {}\n- review findings: {}\n- task outcome reports: {}\n",
        agent.name,
        session.session_id,
        session.review_finding_count,
        session.task_outcome_report_count
    )
}

fn none_if_empty(values: &[String]) -> String {
    if values.is_empty() {
        "none".to_string()
    } else {
        values.join(",")
    }
}
