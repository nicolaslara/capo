use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

use capo_server::{AgentSummary, ServerDashboardSnapshot, ServerThread};

pub(super) trait AgentRenderer {
    fn render(&self, agent: &AgentSummary) -> String;
}

pub(super) trait AgentResultRenderer {
    fn render_result(&self, agent: &AgentSummary) -> String;
}

trait AgentResultBodyRenderer {
    fn render_body(&self, view: &AgentResultView, max_chars: usize) -> String;
}

// Keep provider output parsing and terminal rendering behind Capo-owned types.
// Current first pass: pulldown-cmark classifies Markdown/code structure, and
// PlainMarkdownTerminalRenderer prints readable terminal text without ANSI.
// Later renderer backends to consider:
// - comrak: richer GFM AST, custom formatters, and code-highlighting hooks.
// - markdown-to-ansi or termimad: direct ANSI terminal rendering with styled tables/code.
// - syntect: syntax highlighting for fenced ResultBlock::Code blocks.
// - ratatui/tui-markdown: full TUI widgets if `capo control` grows beyond a line REPL.
#[derive(Debug, PartialEq, Eq)]
struct AgentResultView {
    source: String,
    blocks: Vec<ResultBlock>,
}

#[derive(Debug, PartialEq, Eq)]
enum ResultBlock {
    Paragraph(String),
    Markdown(String),
    Code {
        language: Option<String>,
        text: String,
    },
}

#[derive(Clone, Copy)]
pub(super) struct RecentWorkRenderer;

#[derive(Clone, Copy)]
pub(super) struct ConciseResultRenderer;

#[derive(Clone, Copy)]
pub(super) struct ResultsAndEvidenceRenderer;

#[derive(Clone, Copy)]
struct PlainMarkdownTerminalRenderer;

#[derive(Clone, Copy)]
pub(super) struct DetailsRenderer;

#[derive(Clone, Copy)]
pub(super) struct ToolActivityRenderer;

#[derive(Clone, Copy)]
pub(super) struct EvidenceRenderer;

#[derive(Clone, Copy)]
pub(super) struct ReviewNeedsRenderer;

/// Render the multi-turn conversation thread (ST5) for an agent: a projected,
/// ordered view of turns and their items, replacing the single `latest_summary`
/// line. The thread is read-only here; the REPL renders it and never authors the
/// ordering.
pub(super) fn render_thread(agent_name: &str, thread: &ServerThread) -> String {
    if thread.turns.is_empty() {
        return format!("Thread\n- {agent_name}: no turns yet\n");
    }
    let mut output = format!("Thread ({} turns)\n", thread.turns.len());
    for (index, turn) in thread.turns.iter().enumerate() {
        output.push_str(&format!(
            "Turn {} [{}] {}\n",
            index + 1,
            turn.status,
            turn.turn_id
        ));
        if turn.items.is_empty() {
            output.push_str("  (no items)\n");
            continue;
        }
        for item in &turn.items {
            let body = item
                .text
                .as_deref()
                .filter(|text| !text.trim().is_empty())
                .map(|text| display_text(text, 280))
                .or_else(|| item.item_ref.clone())
                .unwrap_or_else(|| item.event_kind.clone());
            output.push_str(&format!("  - {}: {}\n", item.kind, body));
        }
    }
    output
}

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

impl AgentRenderer for ResultsAndEvidenceRenderer {
    fn render(&self, agent: &AgentSummary) -> String {
        render_results_and_evidence(agent)
    }
}

impl AgentResultRenderer for ConciseResultRenderer {
    fn render_result(&self, agent: &AgentSummary) -> String {
        render_agent_reply(agent)
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
        output.push_str(&render_labeled_display("reply", &reply));
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

pub(super) fn render_results_and_evidence(agent: &AgentSummary) -> String {
    let Some(session) = agent.session.as_ref() else {
        return format!(
            "{}\nstatus: idle\nreply: no active session\nevidence: none\n",
            agent.name
        );
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
        output.push_str(&render_labeled_display("reply", &reply));
    } else if session.dispatch_provider_cli_executed == Some(true) {
        output.push_str("reply: captured; use `details` for artifact metadata.\n");
    } else {
        output.push_str("reply: none yet\n");
    }
    output.push_str(&format!(
        "evidence: {}\n",
        none_if_empty(&session.evidence_refs)
    ));
    if let Some(blocker) = session.latest_blocker.as_deref() {
        output.push_str(&format!("blocker: {blocker}\n"));
    }
    output.push('\n');
    output
}

fn render_agent_reply(agent: &AgentSummary) -> String {
    let Some(session) = agent.session.as_ref() else {
        return format!("{} has no active session.\n", agent.name);
    };
    if let Some(reply) = display_summary(session.latest_summary.as_deref()) {
        return render_agent_result_body(&agent.name, &reply);
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

pub(super) fn render_agent_result_body(agent: &str, body: &str) -> String {
    let body = body.trim_end();
    if body.contains('\n') {
        format!("{agent}:\n{body}\n")
    } else {
        format!("{agent}: {body}\n")
    }
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
    Some(display_text(summary, 500))
}

fn compact_one_line(value: &str) -> String {
    let compact = value.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_chars(&compact, 500)
}

pub(super) fn display_text(value: &str, max_chars: usize) -> String {
    display_text_with(value, max_chars, PlainMarkdownTerminalRenderer)
}

fn display_text_with<R: AgentResultBodyRenderer>(
    value: &str,
    max_chars: usize,
    renderer: R,
) -> String {
    let view = parse_agent_result_view(value);
    renderer.render_body(&view, max_chars)
}

impl AgentResultBodyRenderer for PlainMarkdownTerminalRenderer {
    fn render_body(&self, view: &AgentResultView, max_chars: usize) -> String {
        let has_structured_blocks = view
            .blocks
            .iter()
            .any(|block| !matches!(block, ResultBlock::Paragraph(_)));
        if has_structured_blocks || should_preserve_lines(&view.source) {
            return truncate_chars(&normalize_display_lines(&view.source), max_chars);
        }
        truncate_chars(
            &view.source.split_whitespace().collect::<Vec<_>>().join(" "),
            max_chars,
        )
    }
}

fn parse_agent_result_view(value: &str) -> AgentResultView {
    let source = value.trim().to_string();
    let mut blocks = Vec::new();
    let mut paragraph = String::new();
    let mut code: Option<(Option<String>, String)> = None;
    let mut saw_markdown_block = false;
    let parser = Parser::new_ext(
        &source,
        Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS | Options::ENABLE_STRIKETHROUGH,
    );

    for event in parser {
        match event {
            Event::Start(Tag::CodeBlock(kind)) => {
                flush_paragraph(&mut paragraph, &mut blocks);
                let language = match kind {
                    CodeBlockKind::Fenced(info) => {
                        info.split_whitespace().next().map(str::to_string)
                    }
                    CodeBlockKind::Indented => None,
                };
                code = Some((language, String::new()));
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some((language, text)) = code.take() {
                    blocks.push(ResultBlock::Code { language, text });
                }
            }
            Event::Start(Tag::Table(_))
            | Event::Start(Tag::Heading { .. })
            | Event::Start(Tag::List(_))
            | Event::Start(Tag::BlockQuote(_)) => {
                saw_markdown_block = true;
            }
            Event::Text(text) | Event::Code(text) => {
                if let Some((_, code_text)) = code.as_mut() {
                    code_text.push_str(&text);
                } else {
                    paragraph.push_str(&text);
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if let Some((_, code_text)) = code.as_mut() {
                    code_text.push('\n');
                } else {
                    paragraph.push('\n');
                }
            }
            _ => {}
        }
    }
    flush_paragraph(&mut paragraph, &mut blocks);
    if saw_markdown_block {
        blocks.push(ResultBlock::Markdown(source.clone()));
    }
    if blocks.is_empty() && !source.is_empty() {
        blocks.push(ResultBlock::Paragraph(source.clone()));
    }

    AgentResultView { source, blocks }
}

fn flush_paragraph(paragraph: &mut String, blocks: &mut Vec<ResultBlock>) {
    let text = paragraph.trim();
    if !text.is_empty() {
        blocks.push(ResultBlock::Paragraph(text.to_string()));
    }
    paragraph.clear();
}

fn render_labeled_display(label: &str, body: &str) -> String {
    if body.contains('\n') {
        format!("{label}:\n{body}\n")
    } else {
        format!("{label}: {body}\n")
    }
}

fn should_preserve_lines(value: &str) -> bool {
    if !value.contains('\n') {
        return false;
    }
    let lines = value.lines().map(str::trim).collect::<Vec<_>>();
    lines.iter().any(|line| line.starts_with("```"))
        || lines.iter().any(|line| is_markdown_table_separator(line))
        || lines.iter().filter(|line| is_list_line(line)).count() >= 2
        || lines.iter().filter(|line| !line.is_empty()).count() > 1
}

fn is_markdown_table_separator(line: &str) -> bool {
    line.starts_with('|') && line.ends_with('|') && line.contains("---")
}

fn is_list_line(line: &str) -> bool {
    line.starts_with("- ")
        || line.starts_with("* ")
        || line.split_once(". ").is_some_and(|(prefix, _)| {
            !prefix.is_empty() && prefix.chars().all(|ch| ch.is_ascii_digit())
        })
}

fn normalize_display_lines(value: &str) -> String {
    let mut output = String::new();
    let mut blank_count = 0;
    for line in value.lines() {
        let line = line.trim_end();
        if line.trim().is_empty() {
            blank_count += 1;
            if blank_count <= 1 {
                output.push('\n');
            }
            continue;
        }
        blank_count = 0;
        output.push_str(line);
        output.push('\n');
    }
    output.trim_end().to_string()
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut shortened = value.chars().take(max_chars).collect::<String>();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_text_compacts_plain_prose() {
        assert_eq!(display_text("hello     world", 500), "hello world");
        assert_eq!(compact_one_line("hello\nworld"), "hello world");
    }

    #[test]
    fn display_text_preserves_markdown_tables() {
        let table = "| Number | Double |\n|---:|---:|\n| 1 | 2 |\n| 2 | 4 |\n";

        assert_eq!(
            display_text(table, 500),
            "| Number | Double |\n|---:|---:|\n| 1 | 2 |\n| 2 | 4 |"
        );
    }

    #[test]
    fn display_text_preserves_fenced_code() {
        let code = "```rust\nfn main() {\n    println!(\"hi\");\n}\n```";

        assert_eq!(display_text(code, 500), code);
        assert_eq!(
            parse_agent_result_view(code).blocks,
            vec![ResultBlock::Code {
                language: Some("rust".to_string()),
                text: "fn main() {\n    println!(\"hi\");\n}\n".to_string(),
            }]
        );
    }

    #[test]
    fn result_body_multiline_starts_on_next_line() {
        let rendered = render_agent_result_body("demo", "| A | B |\n|---|---|\n| 1 | 2 |");

        assert_eq!(rendered, "demo:\n| A | B |\n|---|---|\n| 1 | 2 |\n");
    }
}
