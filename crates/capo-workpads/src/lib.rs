//! Non-destructive markdown workpad indexing.
//!
//! This crate treats project markdown as human-authored source material. It
//! reads files and produces observed refs; it does not write source workpads or
//! claim ownership of their task status.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkpadIndex {
    pub root: PathBuf,
    pub observed_unix: i64,
    pub files: Vec<WorkpadFileRef>,
    pub tasks: Vec<WorkpadTaskRef>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkpadFileRef {
    pub path: String,
    pub content_hash: String,
    pub headings: Vec<String>,
    pub objective: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkpadTaskRef {
    pub workpad_task_id: String,
    pub path: String,
    pub source_anchor: String,
    pub title: String,
    pub observed_status: String,
    pub capo_execution_status: String,
}

pub fn index_project_workpads(root: impl AsRef<Path>) -> Result<WorkpadIndex, String> {
    let root = root.as_ref().to_path_buf();
    let mut paths = selected_workpad_paths(&root);
    paths.sort();
    paths.dedup();

    let observed_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_secs() as i64;
    let mut files = Vec::new();
    let mut tasks = Vec::new();

    for path in paths {
        if !path.is_file() {
            continue;
        }
        let markdown = fs::read_to_string(&path).map_err(|error| error.to_string())?;
        let relative = relative_path(&root, &path)?;
        let parsed = parse_markdown(&relative, &markdown);
        tasks.extend(parsed.tasks);
        files.push(WorkpadFileRef {
            path: relative,
            content_hash: stable_hash(markdown.as_bytes()),
            headings: parsed.headings,
            objective: parsed.objective,
        });
    }

    Ok(WorkpadIndex {
        root,
        observed_unix,
        files,
        tasks,
    })
}

struct ParsedMarkdown {
    headings: Vec<String>,
    objective: Option<String>,
    tasks: Vec<WorkpadTaskRef>,
}

fn parse_markdown(path: &str, markdown: &str) -> ParsedMarkdown {
    let lines = markdown.lines().collect::<Vec<_>>();
    let mut headings = Vec::new();
    let mut objective = None;
    let mut tasks = Vec::new();

    for (line_index, line) in lines.iter().enumerate() {
        let Some(title) = heading_title(line) else {
            continue;
        };
        headings.push(title.to_string());

        if title == "Objective" {
            objective = section_text(&lines, line_index + 1);
            continue;
        }

        if let Some(task_key) = task_key(title)
            && let Some(status) = status_after_heading(&lines, line_index + 1)
        {
            tasks.push(WorkpadTaskRef {
                workpad_task_id: format!("{}#{task_key}", path.replace('/', ":")),
                path: path.to_string(),
                source_anchor: title.to_string(),
                title: title.to_string(),
                observed_status: status.to_string(),
                capo_execution_status: "observed_only".to_string(),
            });
        }
    }

    ParsedMarkdown {
        headings,
        objective,
        tasks,
    }
}

fn heading_title(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    let title = trimmed
        .strip_prefix("### ")
        .or_else(|| trimmed.strip_prefix("## "))
        .or_else(|| trimmed.strip_prefix("# "))?;
    let title = title.trim();
    (!title.is_empty()).then_some(title)
}

fn task_key(title: &str) -> Option<String> {
    let first = title.split_whitespace().next()?;
    let valid = first.len() >= 2 && first.chars().all(|ch| ch.is_ascii_alphanumeric());
    valid.then(|| {
        first
            .trim_end_matches([':', '-', '.'])
            .to_ascii_lowercase()
            .replace('_', "-")
    })
}

fn status_after_heading<'a>(lines: &'a [&str], start: usize) -> Option<&'a str> {
    for line in lines.iter().skip(start).take(12) {
        if heading_title(line).is_some() {
            return None;
        }
        if let Some(status) = line.trim().strip_prefix("Status:") {
            return Some(status.trim());
        }
    }
    None
}

fn section_text(lines: &[&str], start: usize) -> Option<String> {
    let mut collected = Vec::new();
    for line in lines.iter().skip(start) {
        if heading_title(line).is_some() {
            break;
        }
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            collected.push(trimmed);
        }
    }
    (!collected.is_empty()).then(|| collected.join("\n"))
}

fn selected_workpad_paths(root: &Path) -> Vec<PathBuf> {
    let mut paths = vec![root.join("TASKS.md"), root.join("project.md")];
    let workpads = root.join("workpads");
    for relative in [
        "WORKPADS.md",
        "research/tasks.md",
        "research/knowledge.md",
        "research/references.md",
        "architecture/tasks.md",
        "architecture/knowledge.md",
        "architecture/references.md",
        "architecture/boundaries.md",
        "architecture/state-model.md",
        "architecture/acp-replay-dedupe.md",
        "architecture/capability-permissions.md",
        "architecture/runtime-tunnel.md",
        "architecture/protocol-provider.md",
        "architecture/tool-exposure.md",
        "architecture/memory-architecture.md",
        "architecture/prototype-plan.md",
        "architecture/gate-review.md",
        "prototype/spec.md",
        "prototype/tasks.md",
        "prototype/knowledge.md",
        "prototype/references.md",
        "features/tasks.md",
        "features/knowledge.md",
        "features/references.md",
        "features/agent-connectors.md",
        "features/dogfood-bridge.md",
        "features/dashboard.md",
        "features/permissions-tools.md",
        "features/memory-eval.md",
        "features/voice.md",
        "features/remote-runtime.md",
        "dogfood/tasks.md",
        "dogfood/knowledge.md",
        "dogfood/references.md",
    ] {
        paths.push(workpads.join(relative));
    }

    for subdir in [
        "research/findings",
        "features",
        "dogfood",
        "prototype",
        "architecture",
    ] {
        collect_direct_markdown(&workpads.join(subdir), &mut paths);
    }
    paths
}

fn collect_direct_markdown(dir: &Path, paths: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && path.extension().is_some_and(|extension| extension == "md") {
            paths.push(path);
        }
    }
}

fn relative_path(root: &Path, path: &Path) -> Result<String, String> {
    path.strip_prefix(root)
        .map_err(|error| error.to_string())
        .map(|path| path.to_string_lossy().replace('\\', "/"))
}

fn stable_hash(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_objective_and_task_status_without_mutating_markdown() {
        let markdown = "\
# Demo

## Objective

Track useful work.

## F2 - Workpad Dogfood Bridge

Status: in_progress

Acceptance:

- Index files.
";

        let parsed = parse_markdown("workpads/features/tasks.md", markdown);

        assert_eq!(parsed.objective.as_deref(), Some("Track useful work."));
        assert_eq!(
            parsed.tasks,
            vec![WorkpadTaskRef {
                workpad_task_id: "workpads:features:tasks.md#f2".to_string(),
                path: "workpads/features/tasks.md".to_string(),
                source_anchor: "F2 - Workpad Dogfood Bridge".to_string(),
                title: "F2 - Workpad Dogfood Bridge".to_string(),
                observed_status: "in_progress".to_string(),
                capo_execution_status: "observed_only".to_string(),
            }]
        );
    }

    #[test]
    fn parses_mixed_case_task_keys() {
        let markdown = "\
# Tasks

## A2a - ACP Replay

Status: completed
";

        let parsed = parse_markdown("workpads/architecture/tasks.md", markdown);

        assert_eq!(
            parsed.tasks[0].workpad_task_id,
            "workpads:architecture:tasks.md#a2a"
        );
        assert_eq!(parsed.tasks[0].observed_status, "completed");
    }
}
