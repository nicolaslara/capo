use std::path::{Component, Path, PathBuf};

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
/// This is the SINGLE public containment engine for the RTL write path: the live
/// workspace-write path calls it so a write outside the confined workspace is
/// rejected *before* any process is spawned. It reuses the same primitives the
/// runtime tool wrappers use ([`ensure_under_workspace`] plus the
/// nearest-existing-ancestor handling for not-yet-created targets) and folds in
/// the credential-component and `..`-rejection rules the live provider's
/// `normalize_policy_path` enforces, so the confinement rule is defined once.
///
/// `workspace_root` must exist (it is the confined boundary). `target` may be
/// absolute or workspace-relative and need not exist yet. The full joined
/// candidate is first **lexically normalized** (folding `.`/`..` without
/// touching the filesystem); a candidate whose normalized form escapes the
/// workspace root -- including a not-yet-created target with interior `..`
/// segments such as `src/sub/../../../escape.txt` -- is rejected here. The
/// normalized candidate is then re-checked against the filesystem (its existing
/// prefix is canonicalized and confined, which also catches symlinked-prefix
/// escapes). On success the canonical confined path is returned (never a value
/// that still contains `..`).
pub fn confine_write_path(target: &Path, workspace_root: &Path) -> Result<PathBuf, String> {
    let workspace_root = workspace_root
        .canonicalize()
        .map_err(|error| format!("workspace root does not exist or is unreadable: {error}"))?;
    let joined = if target.is_absolute() {
        target.to_path_buf()
    } else {
        workspace_root.join(target)
    };
    // Reject credential-like components (`.ssh`, `secrets`, ...) anywhere in the
    // requested path, matching the live provider's containment rule.
    reject_credential_like_components(target)?;
    // Lexically fold `.`/`..` so an interior `..` in a not-yet-created target
    // cannot escape the workspace by hiding behind a non-existent intermediate
    // dir. This is the fix for the deep-`..` confinement bypass: we confine the
    // NORMALIZED candidate, not just the nearest existing ancestor.
    let normalized = lexically_normalize(&joined);
    if normalized.exists() {
        // The path exists, so the FILESYSTEM-canonical form is authoritative:
        // canonicalizing resolves symlinks on both the candidate and the
        // workspace root (`workspace_root` was already canonicalized above), so
        // a `/tmp/...` candidate that the OS symlinks to `/private/tmp/...`
        // confines correctly against a `/private/tmp/...` root. The lexical
        // check is skipped here precisely because it would compare an
        // un-resolved candidate against a resolved root and spuriously reject.
        let canonical = normalized
            .canonicalize()
            .map_err(|error| error.to_string())?;
        ensure_under_workspace(&canonical, &workspace_root)?;
        return Ok(canonical);
    }
    // Not-yet-created: the candidate has no filesystem form to canonicalize, so
    // confine the CANONICAL nearest-existing-ancestor of the lexically-folded
    // candidate. Folding `.`/`..` first means a `..`-escape hiding behind a
    // non-existent intermediate dir already popped above the workspace, so the
    // ancestor walk lands outside and `ensure_under_workspace` rejects it.
    // Canonicalizing the ancestor also catches a symlinked prefix that resolves
    // outside the workspace.
    let ancestor = nearest_existing_ancestor(&normalized).ok_or_else(|| {
        format!(
            "write path has no existing ancestor: {}",
            normalized.display()
        )
    })?;
    let canonical_ancestor = ancestor.canonicalize().map_err(|error| error.to_string())?;
    ensure_under_workspace(&canonical_ancestor, &workspace_root)?;
    // Re-anchor the not-yet-created tail onto the canonical ancestor so the
    // returned path is symlink-resolved and confined, never a `/tmp/...` value
    // that lexically sits outside a `/private/tmp/...` workspace root.
    let tail = normalized.strip_prefix(&ancestor).unwrap_or(&normalized);
    let canonical_candidate = canonical_ancestor.join(tail);
    ensure_under_workspace(&canonical_candidate, &workspace_root)?;
    Ok(canonical_candidate)
}

/// Lexically fold a path's `.`/`..`/separator components WITHOUT touching the
/// filesystem (no symlink resolution). A leading `..` that would pop above the
/// root is dropped, so the result can never lexically ascend past the base; the
/// caller still confines the result with [`ensure_under_workspace`], which is
/// what actually rejects an escape. Shared by [`confine_write_path`] and the
/// runtime tool wrappers' `resolve_workspace_path` so the lexical rule is
/// defined once.
pub(crate) fn lexically_normalize(path: &Path) -> PathBuf {
    let mut normalized: Vec<Component> = Vec::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => match normalized.last() {
                Some(Component::Normal(_)) => {
                    normalized.pop();
                }
                // Popping above a Prefix/RootDir is meaningless; keep an
                // unmatched `..` so a relative escape stays lexically visible
                // (and is then rejected by `ensure_under_workspace`).
                Some(Component::ParentDir) | None => normalized.push(Component::ParentDir),
                Some(Component::Prefix(_)) | Some(Component::RootDir) => {}
                Some(Component::CurDir) => unreachable!("CurDir is never pushed"),
            },
            other => normalized.push(other),
        }
    }
    let mut out = PathBuf::new();
    for component in normalized {
        out.push(component.as_os_str());
    }
    out
}

/// Reject credential-like path components (`.ssh`, `.aws`, `secrets`, tokens,
/// ...) anywhere in `target`, mirroring the live provider's `normalize_policy_path`
/// so both ends of the write path agree on the same forbidden components.
pub(crate) fn reject_credential_like_components(target: &Path) -> Result<(), String> {
    for component in target.components() {
        if let Component::Normal(part) = component {
            let lower = part.to_string_lossy().to_ascii_lowercase();
            if is_credential_like_component(&lower) {
                return Err(format!(
                    "credential-like path component `{}`",
                    part.to_string_lossy()
                ));
            }
        }
    }
    Ok(())
}

fn is_credential_like_component(component: &str) -> bool {
    matches!(
        component,
        ".ssh"
            | ".aws"
            | ".config"
            | ".codex"
            | ".claude"
            | ".anthropic"
            | "credentials"
            | "credential"
            | "secrets"
            | "secret"
            | "tokens"
            | "token"
            | "cookies"
            | "cookie"
            | "oauth"
            | "sessions"
            | "session"
    )
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
