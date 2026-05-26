use std::path::{Path, PathBuf};

use capo_core::RunId;

pub(crate) fn is_workpad_path(path: &str) -> bool {
    let path = Path::new(path);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir | std::path::Component::Prefix(_)
            )
        })
    {
        return false;
    }
    let normalized = path.display().to_string();
    normalized == "TASKS.md"
        || normalized == "project.md"
        || (normalized.starts_with("workpads/")
            && normalized.ends_with(".md")
            && !normalized.contains("/research-clones/")
            && !normalized.contains("/scratch/"))
}

pub(crate) fn workspace_path(workspace_root: &Path, path: &str) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace_root.join(path)
    }
}

pub(crate) fn workspace_relative_path(path: &str) -> Result<String, String> {
    let path = Path::new(path);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir | std::path::Component::Prefix(_)
            )
        })
    {
        return Err(format!(
            "git path must be workspace-relative: {}",
            path.display()
        ));
    }
    Ok(path.display().to_string())
}

pub(crate) fn sanitize_path_component(value: &str) -> String {
    let mut sanitized = String::new();
    let mut previous_dash = false;
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            sanitized.push(ch);
            previous_dash = false;
        } else if !previous_dash {
            sanitized.push('-');
            previous_dash = true;
        }
    }
    let trimmed = sanitized.trim_matches('-');
    if trimmed.is_empty() {
        "tool-call".to_string()
    } else {
        trimmed.to_string()
    }
}

pub(crate) fn sanitized_run_id(run_id: &RunId) -> RunId {
    RunId::new(sanitize_path_component(run_id.as_str()))
}

pub(crate) fn ensure_under_workspace(path: &Path, workspace_root: &Path) -> Result<(), String> {
    let workspace_root = workspace_root
        .canonicalize()
        .map_err(|error| error.to_string())?;
    if path.starts_with(&workspace_root) {
        Ok(())
    } else {
        Err(format!(
            "path escapes workspace: {} not under {}",
            path.display(),
            workspace_root.display()
        ))
    }
}

pub(crate) fn nearest_existing_ancestor(path: &Path) -> Option<PathBuf> {
    let mut cursor = path.parent();
    while let Some(parent) = cursor {
        if parent.exists() {
            return Some(parent.to_path_buf());
        }
        cursor = parent.parent();
    }
    None
}
