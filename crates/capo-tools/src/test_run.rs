//! Bounded failing-item extraction for `capo.test_run` / `capo.check` (ACI6).
//!
//! `capo.test_run` is a specialized shell wrapper (`tool-exposure.md:196-198`)
//! that runs a test/check command and returns a typed
//! `{command, exit_status, passed, failing_items, duration_ms,
//! output_artifact_id}` record. It emits decision-grade evidence only: it does
//! NOT compute a score or own the verification gate -- `safety-gates`'
//! `VerificationRunner` consumes this typed record and owns `score_run`.
//!
//! This module owns the deterministic, process-free part: parsing a command's
//! captured output into a BOUNDED `failing_items` list. The full output always
//! lives in a redacted artifact; `failing_items` is only the decision-grade
//! summary (failing test names, or the first-N failure lines when no test names
//! are recognized), capped so the inline payload never dumps the whole log.

/// The per-call caps that bound the inline `failing_items` payload (ACI6).
///
/// `failing_items` is capped by BOTH a count cap (`max_items`) AND a per-item
/// byte cap (`max_item_bytes`, a pathological long line cannot blow the budget),
/// so the inline result stays decision-grade while the FULL output lives in the
/// redacted artifact.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FailingItemsCaps {
    /// Maximum number of failing items returned inline.
    pub max_items: usize,
    /// Maximum bytes kept for a single failing item (clipped with a trailing
    /// marker when exceeded).
    pub max_item_bytes: usize,
}

impl Default for FailingItemsCaps {
    fn default() -> Self {
        Self {
            max_items: 20,
            max_item_bytes: 256,
        }
    }
}

/// The marker appended to a failing item clipped to the per-item byte cap, and
/// the trailing item appended when more failures were elided than the count cap
/// admits.
const CLIP_MARKER: &str = "...";

/// The bounded outcome of extracting failing items from a command's output
/// (ACI6): the capped items plus an explicit `truncated` marker so the agent
/// (and the verification gate) knows the inline list is partial rather than
/// silently complete.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FailingItems {
    pub items: Vec<String>,
    /// Total failures detected before capping. Lets the consumer see how much
    /// was elided rather than only that *something* was.
    pub total: usize,
    /// True when the count cap or a per-item clip elided content.
    pub truncated: bool,
}

/// Extract a BOUNDED list of failing items from a finished command's combined
/// output (ACI6).
///
/// A non-failing command (`passed`) has no failing items. For a failing
/// command, failing test NAMES are preferred when the output uses a recognized
/// harness shape (`cargo test`/`cargo nextest` `FAILED`/`test ... FAILED`,
/// pytest `FAILED ...`, a generic `FAIL`/`✗`/`not ok` prefix); when no test
/// names are recognized, the FIRST-N non-empty lines are used as a fallback so a
/// failing command always yields some decision-grade signal. Either way the
/// result is capped by `caps` and marked `truncated` when content was elided.
pub(crate) fn extract_failing_items(
    output: &str,
    passed: bool,
    caps: FailingItemsCaps,
) -> FailingItems {
    if passed {
        return FailingItems {
            items: Vec::new(),
            total: 0,
            truncated: false,
        };
    }
    let mut named = parse_failing_test_names(output);
    let raw = if named.is_empty() {
        // No recognized test-name shape: fall back to the first non-empty lines
        // so a failing command still surfaces something actionable.
        first_failure_lines(output)
    } else {
        named.dedup();
        named
    };
    apply_caps(raw, caps)
}

/// Apply the count + per-item byte caps to an ordered list of failing items.
fn apply_caps(raw: Vec<String>, caps: FailingItemsCaps) -> FailingItems {
    let total = raw.len();
    let mut items: Vec<String> = raw
        .into_iter()
        .take(caps.max_items)
        .map(|item| clip_item(&item, caps.max_item_bytes))
        .collect();
    let count_truncated = total > caps.max_items;
    let item_clipped = items.iter().any(|item| item.ends_with(CLIP_MARKER));
    if count_truncated {
        items.push(format!(
            "{CLIP_MARKER} {} more failing item(s) elided (see output artifact)",
            total - caps.max_items
        ));
    }
    FailingItems {
        truncated: count_truncated || item_clipped,
        total,
        items,
    }
}

/// Clip a single failing item to `max_bytes`, appending a trailing marker when
/// clipped. Respects UTF-8 char boundaries so the item stays valid text.
fn clip_item(item: &str, max_bytes: usize) -> String {
    if item.len() <= max_bytes {
        return item.to_string();
    }
    let budget = max_bytes.saturating_sub(CLIP_MARKER.len());
    let mut end = budget.min(item.len());
    while end > 0 && !item.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{CLIP_MARKER}", &item[..end])
}

/// Recognize failing test names from common harness output shapes (ACI6).
///
/// Deterministic, line-oriented, and conservative: it only collects a name when
/// a line matches a known failure shape, so a passing line near the word "fail"
/// is not mis-collected. Order is preserved (first occurrence wins).
fn parse_failing_test_names(output: &str) -> Vec<String> {
    let mut names = Vec::new();
    for line in output.lines() {
        let trimmed = line.trim();
        if let Some(name) = failing_test_name(trimmed) {
            names.push(name);
        }
    }
    names
}

/// Extract the failing test name from a single trimmed output line, if it is a
/// recognized failure line.
fn failing_test_name(line: &str) -> Option<String> {
    // cargo test: `test path::to::name ... FAILED`
    if let Some(rest) = line.strip_prefix("test ")
        && rest.ends_with(" ... FAILED")
    {
        let name = rest.trim_end_matches(" ... FAILED").trim();
        if !name.is_empty() {
            return Some(name.to_string());
        }
    }
    // cargo nextest: `FAIL [   0.001s] crate::test name`
    if let Some(rest) = line.strip_prefix("FAIL ")
        && let Some(after_bracket) = rest.split(']').nth(1)
    {
        let name = after_bracket.trim();
        if !name.is_empty() {
            return Some(name.to_string());
        }
    }
    // pytest: `FAILED tests/test_x.py::test_name - AssertionError`
    if let Some(rest) = line.strip_prefix("FAILED ") {
        let name = rest.split(" - ").next().unwrap_or(rest).trim();
        if !name.is_empty() {
            return Some(name.to_string());
        }
    }
    // TAP: `not ok 3 - test name`
    if let Some(rest) = line.strip_prefix("not ok ") {
        let name = rest
            .split_once(" - ")
            .map(|(_, name)| name)
            .unwrap_or(rest)
            .trim();
        if !name.is_empty() {
            return Some(name.to_string());
        }
    }
    None
}

/// Fallback when no test names are recognized: the first non-empty output lines
/// (ACI6). Capped downstream by [`apply_caps`].
fn first_failure_lines(output: &str) -> Vec<String> {
    output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToString::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passing_command_has_no_failing_items() {
        let items = extract_failing_items("test foo ... ok\n", true, FailingItemsCaps::default());
        assert!(items.items.is_empty());
        assert_eq!(items.total, 0);
        assert!(!items.truncated);
    }

    #[test]
    fn parses_cargo_test_failed_names() {
        let output = concat!(
            "running 3 tests\n",
            "test mod::passes ... ok\n",
            "test mod::alpha_fails ... FAILED\n",
            "test mod::beta_fails ... FAILED\n",
            "failures:\n",
        );
        let items = extract_failing_items(output, false, FailingItemsCaps::default());
        assert_eq!(items.items, vec!["mod::alpha_fails", "mod::beta_fails"]);
        assert_eq!(items.total, 2);
        assert!(!items.truncated);
    }

    #[test]
    fn parses_pytest_and_tap_names() {
        let pytest = "FAILED tests/test_x.py::test_one - AssertionError: nope\n";
        let items = extract_failing_items(pytest, false, FailingItemsCaps::default());
        assert_eq!(items.items, vec!["tests/test_x.py::test_one"]);

        let tap = "not ok 4 - widget renders\n";
        let items = extract_failing_items(tap, false, FailingItemsCaps::default());
        assert_eq!(items.items, vec!["widget renders"]);
    }

    #[test]
    fn falls_back_to_first_lines_when_no_test_names() {
        let output = "error[E0433]: failed to resolve\n  --> src/lib.rs:1:5\n";
        let items = extract_failing_items(output, false, FailingItemsCaps::default());
        assert_eq!(items.total, 2);
        assert_eq!(items.items[0], "error[E0433]: failed to resolve");
    }

    #[test]
    fn count_cap_truncates_and_appends_elision_marker() {
        let output = (0..30)
            .map(|n| format!("test t::case{n} ... FAILED"))
            .collect::<Vec<_>>()
            .join("\n");
        let caps = FailingItemsCaps {
            max_items: 5,
            max_item_bytes: 256,
        };
        let items = extract_failing_items(&output, false, caps);
        assert_eq!(items.total, 30);
        assert!(items.truncated);
        // 5 capped names + 1 elision marker line.
        assert_eq!(items.items.len(), 6);
        assert!(items.items.last().expect("marker").starts_with(CLIP_MARKER));
    }

    #[test]
    fn per_item_byte_cap_clips_long_items() {
        let long = format!("test {}  ... FAILED", "x".repeat(1000));
        let caps = FailingItemsCaps {
            max_items: 20,
            max_item_bytes: 64,
        };
        let items = extract_failing_items(&long, false, caps);
        assert_eq!(items.items.len(), 1);
        assert!(items.items[0].len() <= 64);
        assert!(items.items[0].ends_with(CLIP_MARKER));
        assert!(items.truncated);
    }
}
