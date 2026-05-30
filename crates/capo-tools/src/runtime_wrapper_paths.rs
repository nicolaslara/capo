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

/// Confine a write target to a workspace root, rejecting any path that escapes
/// the confined workspace before a caller acts on it.
///
/// This is the public reuse seam for the RTL safety floor: the live workspace-
/// write path calls it so a write outside the confined workspace is rejected
/// *before* any process is spawned. It reuses the same containment engine the
/// runtime tool wrappers use ([`ensure_under_workspace`] plus the
/// nearest-existing-ancestor handling for not-yet-created targets), so the
/// confinement rule is defined once.
///
/// `workspace_root` must exist (it is the confined boundary). `target` may be
/// absolute or workspace-relative and need not exist yet; if it does not, the
/// nearest existing ancestor is canonicalized and confined instead, which still
/// rejects `..`-escapes and symlink escapes. On success the canonical confined
/// path is returned.
pub fn confine_write_path(target: &Path, workspace_root: &Path) -> Result<PathBuf, String> {
    let workspace_root = workspace_root
        .canonicalize()
        .map_err(|error| format!("workspace root does not exist or is unreadable: {error}"))?;
    let candidate = if target.is_absolute() {
        target.to_path_buf()
    } else {
        workspace_root.join(target)
    };
    if candidate.exists() {
        let canonical = candidate
            .canonicalize()
            .map_err(|error| error.to_string())?;
        ensure_under_workspace(&canonical, &workspace_root)?;
        return Ok(canonical);
    }
    let ancestor = nearest_existing_ancestor(&candidate).ok_or_else(|| {
        format!(
            "write path has no existing ancestor: {}",
            candidate.display()
        )
    })?;
    let canonical_ancestor = ancestor.canonicalize().map_err(|error| error.to_string())?;
    // Confine the nearest existing ancestor: this rejects `..`-escapes and any
    // symlinked prefix that resolves outside the workspace before the path is
    // created, mirroring the runtime tool wrappers' allow-missing containment.
    ensure_under_workspace(&canonical_ancestor, &workspace_root)?;
    Ok(candidate)
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
