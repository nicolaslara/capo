//! Bounded search/locator result model for `capo.search` (ACI5).
//!
//! `capo.search` is ripgrep-backed (run through the bounded runtime runner) and
//! returns typed, capped `path:line:preview` matches inspired by aider's repo-map
//! and codex's file-search. The agent must find edit targets without the tool
//! dumping whole files, so results are decision-grade and bounded by TWO caps:
//!
//! - a per-call MATCH cap (`max N` matches), and
//! - a total preview BYTE cap.
//!
//! When either cap is hit the result carries an explicit truncation marker so the
//! agent knows the result is partial rather than silently incomplete. This module
//! owns the deterministic parsing/capping logic (no process, no filesystem) so it
//! is unit-testable in isolation; the wrapper handler in `runtime_wrappers.rs`
//! drives ripgrep and feeds its line-delimited JSON here.

use serde_json::Value;

/// One bounded, decision-grade search match (ACI5): a workspace-relative `path`,
/// the 1-based `line`, and a single-line `preview`. The preview is the matched
/// line trimmed of its trailing newline; the wrapper scrubs it through the
/// configured redaction before it reaches the agent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SearchMatch {
    pub path: String,
    pub line: i64,
    pub preview: String,
}

impl SearchMatch {
    pub(crate) fn to_json(&self) -> Value {
        serde_json::json!({
            "path": self.path,
            "line": self.line,
            "preview": self.preview,
        })
    }
}

/// The bounded outcome of a search: the capped matches plus an explicit
/// truncation marker and the reason a cap fired (ACI5).
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BoundedSearch {
    pub matches: Vec<SearchMatch>,
    /// Total matches ripgrep reported before capping. Lets the agent see how
    /// much was elided rather than only that *something* was.
    pub total_matches: usize,
    /// True when EITHER the per-call match cap or the total preview byte cap was
    /// hit, so the agent knows the result is partial.
    pub truncated: bool,
    /// Why truncation fired (`none`, `match_cap`, or `byte_cap`), so the agent
    /// can widen the query or raise the cap deliberately.
    pub truncation_reason: &'static str,
}

/// The per-call caps for a bounded search (ACI5).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SearchCaps {
    /// Maximum number of matches returned per call.
    pub max_matches: usize,
    /// Maximum total bytes of preview text across all returned matches.
    pub max_preview_bytes: usize,
    /// Maximum bytes kept for a single preview line (a pathological long line
    /// cannot blow the whole budget on its own; the preview is clipped and a
    /// trailing marker is appended).
    pub max_preview_line_bytes: usize,
}

impl Default for SearchCaps {
    fn default() -> Self {
        Self {
            max_matches: 50,
            max_preview_bytes: 8 * 1024,
            max_preview_line_bytes: 256,
        }
    }
}

/// The marker appended to a single preview line that exceeds the per-line cap.
const PREVIEW_CLIP_MARKER: &str = "...";

/// Apply the caps to an ordered list of raw matches, returning a [`BoundedSearch`]
/// with an explicit truncation marker (ACI5).
///
/// Capping is applied in match order: once the match count reaches `max_matches`
/// or appending the next preview would exceed `max_preview_bytes`, no further
/// matches are kept and the result is marked truncated. The byte budget counts
/// the (already per-line-clipped) preview bytes, so a small budget bounds the
/// total payload regardless of how many or how long the underlying matches are.
pub(crate) fn apply_caps(raw: Vec<SearchMatch>, caps: SearchCaps) -> BoundedSearch {
    let total_matches = raw.len();
    let mut kept = Vec::new();
    let mut used_bytes = 0usize;
    let mut truncation_reason = "none";
    for mut candidate in raw {
        if kept.len() >= caps.max_matches {
            truncation_reason = "match_cap";
            break;
        }
        candidate.preview = clip_preview(&candidate.preview, caps.max_preview_line_bytes);
        let next_bytes = used_bytes.saturating_add(candidate.preview.len());
        if !kept.is_empty() && next_bytes > caps.max_preview_bytes {
            // The byte budget would be exceeded by this match. Stop here so the
            // returned payload stays under the cap (the FIRST match is always
            // kept so a single oversized line still yields one decision-grade
            // result rather than an empty, useless answer).
            truncation_reason = "byte_cap";
            break;
        }
        used_bytes = next_bytes;
        kept.push(candidate);
    }
    BoundedSearch {
        truncated: truncation_reason != "none",
        truncation_reason,
        total_matches,
        matches: kept,
    }
}

/// Clip a single preview line to `max_line_bytes`, appending a trailing marker
/// when it was clipped. Clipping respects UTF-8 char boundaries so the preview
/// stays valid text.
fn clip_preview(preview: &str, max_line_bytes: usize) -> String {
    if preview.len() <= max_line_bytes {
        return preview.to_string();
    }
    let budget = max_line_bytes.saturating_sub(PREVIEW_CLIP_MARKER.len());
    let mut end = budget.min(preview.len());
    while end > 0 && !preview.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{PREVIEW_CLIP_MARKER}", &preview[..end])
}

/// Parse ripgrep's line-delimited `--json` output into ordered raw matches
/// (ACI5).
///
/// Each non-empty line is a JSON object; only `{"type":"match"}` records yield a
/// match. The match line text is trimmed of its trailing newline so the preview
/// is a single line. The ripgrep `path.text` is normalized from its `./`-relative
/// form to a clean workspace-relative path. Non-match records (`begin`/`end`/
/// `summary`) and unparseable lines are skipped, so a partial or noisy stream
/// still yields the matches it can.
pub(crate) fn parse_ripgrep_json(stdout: &str) -> Vec<SearchMatch> {
    let mut matches = Vec::new();
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(record) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if record.get("type").and_then(Value::as_str) != Some("match") {
            continue;
        }
        let Some(data) = record.get("data") else {
            continue;
        };
        let Some(path) = data
            .get("path")
            .and_then(|path| path.get("text"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        let line_number = data.get("line_number").and_then(Value::as_i64).unwrap_or(0);
        let preview = data
            .get("lines")
            .and_then(|lines| lines.get("text"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim_end_matches(['\n', '\r'])
            .to_string();
        matches.push(SearchMatch {
            path: normalize_match_path(path),
            line: line_number,
            preview,
        });
    }
    matches
}

/// Normalize ripgrep's `path.text` (which is `./`-prefixed when searching `.`)
/// into a clean workspace-relative path.
fn normalize_match_path(path: &str) -> String {
    path.strip_prefix("./").unwrap_or(path).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_match(path: &str, line: i64, preview: &str) -> SearchMatch {
        SearchMatch {
            path: path.to_string(),
            line,
            preview: preview.to_string(),
        }
    }

    #[test]
    fn parses_only_match_records_from_ripgrep_json() {
        // A real line-delimited rg --json stream: begin/match/end/summary. Only
        // the match records become typed matches, with the trailing newline
        // stripped and the `./` prefix normalized away.
        let stdout = concat!(
            "{\"type\":\"begin\",\"data\":{\"path\":{\"text\":\"./a.txt\"}}}\n",
            "{\"type\":\"match\",\"data\":{\"path\":{\"text\":\"./a.txt\"},\"lines\":{\"text\":\"alpha needle here\\n\"},\"line_number\":1,\"submatches\":[]}}\n",
            "{\"type\":\"match\",\"data\":{\"path\":{\"text\":\"./a.txt\"},\"lines\":{\"text\":\"gamma needle\\n\"},\"line_number\":3,\"submatches\":[]}}\n",
            "{\"type\":\"end\",\"data\":{\"path\":{\"text\":\"./a.txt\"}}}\n",
            "{\"type\":\"summary\",\"data\":{}}\n",
        );
        let matches = parse_ripgrep_json(stdout);
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0], sample_match("a.txt", 1, "alpha needle here"));
        assert_eq!(matches[1], sample_match("a.txt", 3, "gamma needle"));
    }

    #[test]
    fn skips_unparseable_lines_without_failing_the_whole_parse() {
        let stdout = concat!(
            "not json at all\n",
            "{\"type\":\"match\",\"data\":{\"path\":{\"text\":\"x.rs\"},\"lines\":{\"text\":\"hit\\n\"},\"line_number\":7}}\n",
        );
        let matches = parse_ripgrep_json(stdout);
        assert_eq!(matches, vec![sample_match("x.rs", 7, "hit")]);
    }

    #[test]
    fn match_cap_truncates_and_marks_the_result() {
        let raw = (1..=10)
            .map(|n| sample_match("f.txt", n, "line"))
            .collect::<Vec<_>>();
        let caps = SearchCaps {
            max_matches: 3,
            max_preview_bytes: 64 * 1024,
            max_preview_line_bytes: 256,
        };
        let bounded = apply_caps(raw, caps);
        assert_eq!(bounded.matches.len(), 3);
        assert_eq!(bounded.total_matches, 10);
        assert!(bounded.truncated);
        assert_eq!(bounded.truncation_reason, "match_cap");
    }

    #[test]
    fn byte_cap_truncates_before_the_match_cap() {
        // Each preview is 10 bytes; a 25-byte budget admits 2 (20 bytes) and
        // stops before the third would push to 30, even though the match cap is
        // far higher.
        let raw = (1..=10)
            .map(|n| sample_match("f.txt", n, "0123456789"))
            .collect::<Vec<_>>();
        let caps = SearchCaps {
            max_matches: 50,
            max_preview_bytes: 25,
            max_preview_line_bytes: 256,
        };
        let bounded = apply_caps(raw, caps);
        assert_eq!(bounded.matches.len(), 2);
        assert!(bounded.truncated);
        assert_eq!(bounded.truncation_reason, "byte_cap");
    }

    #[test]
    fn under_both_caps_is_not_truncated() {
        let raw = vec![sample_match("a", 1, "x"), sample_match("b", 2, "y")];
        let bounded = apply_caps(raw, SearchCaps::default());
        assert_eq!(bounded.matches.len(), 2);
        assert!(!bounded.truncated);
        assert_eq!(bounded.truncation_reason, "none");
    }

    #[test]
    fn first_match_is_always_kept_even_when_it_alone_exceeds_the_byte_budget() {
        // A single oversized match must still yield one decision-grade result
        // rather than an empty answer; it is clipped to the per-line cap and the
        // result is marked truncated by the per-line clip, not dropped.
        let raw = vec![sample_match("a", 1, &"z".repeat(10_000))];
        let caps = SearchCaps {
            max_matches: 50,
            max_preview_bytes: 16,
            max_preview_line_bytes: 32,
        };
        let bounded = apply_caps(raw, caps);
        assert_eq!(bounded.matches.len(), 1);
        // The single kept preview is clipped to the per-line cap.
        assert!(bounded.matches[0].preview.len() <= 32);
        assert!(bounded.matches[0].preview.ends_with(PREVIEW_CLIP_MARKER));
    }

    #[test]
    fn long_preview_line_is_clipped_to_the_per_line_cap() {
        let raw = vec![sample_match("a", 1, &"abc".repeat(200))];
        let caps = SearchCaps {
            max_matches: 50,
            max_preview_bytes: 64 * 1024,
            max_preview_line_bytes: 64,
        };
        let bounded = apply_caps(raw, caps);
        assert!(bounded.matches[0].preview.len() <= 64);
        assert!(bounded.matches[0].preview.ends_with(PREVIEW_CLIP_MARKER));
    }
}
