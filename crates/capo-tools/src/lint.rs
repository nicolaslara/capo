//! Syntax/lint-on-edit findings for `capo.apply_patch` (ACI4).
//!
//! After a patch applies, a language-pluggable lint check runs and returns typed
//! findings (`file`, `line`, `rule`, `message`) the loop can reflect on and
//! repair -- mirroring aider's `auto_lint` -> `lint_edited` -> reflected-message
//! loop. Rust is first-class via `rustfmt --check`; the [`Linter`] selection is
//! pluggable so other languages slot in without a redesign.

use std::path::Path;

use serde_json::Value;

/// One typed lint finding (ACI4): the offending `file`, the 1-based `line`, the
/// `rule` that fired, and a human `message`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LintFinding {
    pub file: String,
    pub line: i64,
    pub rule: String,
    pub message: String,
}

impl LintFinding {
    pub(crate) fn to_json(&self) -> Value {
        serde_json::json!({
            "file": self.file,
            "line": self.line,
            "rule": self.rule,
            "message": self.message,
        })
    }
}

/// The language linter selected for an edited file (ACI4).
///
/// Pluggable: today only Rust (`rustfmt --check`) is wired, and any other
/// extension reports `None` so the patch tool records `lint_status:"skipped"`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Linter {
    Rustfmt,
}

impl Linter {
    /// Select a linter for a file by extension. Returns `None` when no linter is
    /// wired for the language (the patch still applies; lint is simply skipped).
    pub(crate) fn for_path(path: &Path) -> Option<Self> {
        match path.extension().and_then(|ext| ext.to_str()) {
            Some("rs") => Some(Self::Rustfmt),
            _ => None,
        }
    }

    /// The program + argv to run for this linter against `path`. The check is a
    /// non-mutating verification (`rustfmt --check`), never a rewrite.
    ///
    /// The program is resolved to an ABSOLUTE path: the bounded runtime runner
    /// clears the environment (no `PATH`), so a bare `rustfmt` would only resolve
    /// against the OS default path (`/usr/bin:/bin`) and miss a cargo-installed
    /// toolchain in `~/.cargo/bin`. We resolve against the current process
    /// `PATH` up front so the linter is found deterministically.
    pub(crate) fn command(self, path: &str) -> (String, Vec<String>) {
        match self {
            Self::Rustfmt => (
                resolve_program("rustfmt"),
                vec![
                    "--edition".to_string(),
                    "2021".to_string(),
                    "--check".to_string(),
                    path.to_string(),
                ],
            ),
        }
    }

    /// Parse the linter's stdout/stderr into typed findings.
    pub(crate) fn parse(
        self,
        file: &str,
        exit_code: Option<i32>,
        stderr: &str,
    ) -> Vec<LintFinding> {
        match self {
            Self::Rustfmt => parse_rustfmt(file, exit_code, stderr),
        }
    }
}

/// Resolve a program name to an absolute path by searching the current process
/// `PATH`, returning the bare name if no entry resolves (so the caller still
/// gets a deterministic command and the runner reports `unavailable` if it
/// genuinely cannot spawn).
fn resolve_program(program: &str) -> String {
    if program.contains('/') {
        return program.to_string();
    }
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            let candidate = dir.join(program);
            if candidate.is_file() {
                return candidate.display().to_string();
            }
        }
    }
    program.to_string()
}

/// Parse `rustfmt --check` output into typed findings.
///
/// `rustfmt --check` exits non-zero and prints a `Diff in <path> at line N:`
/// header per region that is not formatted, plus an `error[: ...]` line on a
/// genuine parse error. We surface one finding per `Diff in ... at line N`
/// region (rule `rustfmt`) and one finding per `error` line (rule `syntax`).
fn parse_rustfmt(file: &str, exit_code: Option<i32>, stderr: &str) -> Vec<LintFinding> {
    if exit_code == Some(0) {
        return Vec::new();
    }
    let mut findings = Vec::new();
    for line in stderr.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("Diff in ") {
            // Format: `Diff in <path> at line <N>:`
            let parsed_line = rest
                .rsplit_once("at line ")
                .and_then(|(_, tail)| tail.trim_end_matches(':').trim().parse::<i64>().ok())
                .unwrap_or(0);
            findings.push(LintFinding {
                file: file.to_string(),
                line: parsed_line,
                rule: "rustfmt".to_string(),
                message: "code is not rustfmt-formatted in this region".to_string(),
            });
        } else if trimmed.starts_with("error") {
            findings.push(LintFinding {
                file: file.to_string(),
                line: 0,
                rule: "syntax".to_string(),
                message: trimmed.to_string(),
            });
        }
    }
    // Non-zero exit with no recognized lines still counts as a finding so the
    // loop is told the check failed rather than silently passing.
    if findings.is_empty() {
        findings.push(LintFinding {
            file: file.to_string(),
            line: 0,
            rule: "rustfmt".to_string(),
            message: format!("rustfmt --check failed (exit {exit_code:?})"),
        });
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rustfmt_clean_exit_has_no_findings() {
        assert!(parse_rustfmt("a.rs", Some(0), "").is_empty());
    }

    #[test]
    fn rustfmt_diff_region_becomes_a_typed_finding() {
        let stderr = "Diff in /ws/src/lib.rs at line 3:\n some diff\n";
        let findings = parse_rustfmt("src/lib.rs", Some(1), stderr);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].rule, "rustfmt");
        assert_eq!(findings[0].line, 3);
    }

    #[test]
    fn rustfmt_parse_error_becomes_a_syntax_finding() {
        let stderr = "error: expected `;`, found `}`\n";
        let findings = parse_rustfmt("src/lib.rs", Some(1), stderr);
        assert!(findings.iter().any(|finding| finding.rule == "syntax"));
    }

    #[test]
    fn linter_selection_is_pluggable_by_extension() {
        assert_eq!(Linter::for_path(Path::new("a.rs")), Some(Linter::Rustfmt));
        assert_eq!(Linter::for_path(Path::new("a.py")), None);
        assert_eq!(Linter::for_path(Path::new("README.md")), None);
    }
}
