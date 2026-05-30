//! Typed search/replace patch model with whitespace/fuzzy-tolerant location and
//! a structured, retryable no-match error (ACI4).
//!
//! `capo.apply_patch` takes one or more search/replace hunks against a single
//! file and locates each hunk's `search` block in the current file content using
//! a cascade of strategies borrowed from aider's editblocks: a perfect match
//! first, then a whitespace-insensitive match, then a "dotdotdot" elided-context
//! match, then an edit-distance fuzzy fallback. When no strategy locates a hunk
//! the tool returns a STRUCTURED retryable error (which path, which hunk, the
//! nearest candidate span) rather than a raw string, so the loop can reflect and
//! retry -- mirroring aider's `SearchReplaceNoExactMatch`.

/// One typed search/replace hunk: locate `search` in the file and substitute
/// `replace`. An empty `search` means "insert/create" (append to a new or empty
/// file).
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PatchHunk {
    pub search: String,
    pub replace: String,
}

/// How a hunk's `search` block was located in the file (ACI4).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MatchStrategy {
    /// Empty search applied to an empty/new file (insert).
    Insert,
    /// Byte-for-byte match.
    Perfect,
    /// Match ignoring leading/trailing whitespace per line.
    Whitespace,
    /// Match with `...` elided interior context (dotdotdot).
    DotDotDot,
    /// Best edit-distance window above the similarity threshold (fuzzy).
    Fuzzy,
}

impl MatchStrategy {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Insert => "insert",
            Self::Perfect => "perfect",
            Self::Whitespace => "whitespace",
            Self::DotDotDot => "dotdotdot",
            Self::Fuzzy => "fuzzy",
        }
    }
}

/// A located hunk: the `[start, end)` line span in the source it matched and the
/// strategy that found it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HunkMatch {
    pub start_line: usize,
    pub end_line: usize,
    pub strategy: MatchStrategy,
}

/// A structured, retryable no-match error for a single hunk (ACI4).
///
/// Carries which hunk failed, why, and the nearest candidate span/preview so the
/// loop can reflect and retry instead of seeing a raw error string. Shaped after
/// aider's `SearchReplaceNoExactMatch`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NoMatch {
    pub hunk_index: usize,
    pub reason: String,
    /// The nearest candidate line span `[start, end)` (1-based start) and its
    /// preview text, when one exists, so the agent can correct the `search`.
    pub nearest_start_line: Option<usize>,
    pub nearest_preview: Option<String>,
    /// The best similarity ratio (0.0..=1.0) observed for the nearest candidate.
    pub nearest_similarity: f64,
}

/// The minimum similarity ratio for the fuzzy fallback to accept a window.
const FUZZY_THRESHOLD: f64 = 0.8;

/// Locate `hunk.search` in `lines` starting at `from_line`, trying perfect, then
/// whitespace-tolerant, then dotdotdot, then a fuzzy edit-distance fallback.
///
/// Returns the matched span on success, or a structured [`NoMatch`] describing
/// the nearest candidate on failure.
pub(crate) fn locate_hunk(
    lines: &[&str],
    hunk: &PatchHunk,
    hunk_index: usize,
    from_line: usize,
) -> Result<HunkMatch, NoMatch> {
    let search_lines: Vec<&str> = split_keep_block(&hunk.search);

    // Empty search = insert (create/append). It always "matches" at end-of-file.
    if hunk.search.is_empty() {
        return Ok(HunkMatch {
            start_line: lines.len(),
            end_line: lines.len(),
            strategy: MatchStrategy::Insert,
        });
    }

    if let Some(start) = find_perfect(lines, &search_lines, from_line) {
        return Ok(HunkMatch {
            start_line: start,
            end_line: start + search_lines.len(),
            strategy: MatchStrategy::Perfect,
        });
    }
    if let Some(start) = find_whitespace(lines, &search_lines, from_line) {
        return Ok(HunkMatch {
            start_line: start,
            end_line: start + search_lines.len(),
            strategy: MatchStrategy::Whitespace,
        });
    }
    if let Some(span) = find_dotdotdot(lines, &search_lines, from_line) {
        return Ok(HunkMatch {
            start_line: span.0,
            end_line: span.1,
            strategy: MatchStrategy::DotDotDot,
        });
    }
    match find_fuzzy(lines, &search_lines, from_line) {
        FuzzyResult::Match { start, similarity } if similarity >= FUZZY_THRESHOLD => {
            Ok(HunkMatch {
                start_line: start,
                end_line: start + search_lines.len(),
                strategy: MatchStrategy::Fuzzy,
            })
        }
        FuzzyResult::Match { start, similarity } => Err(NoMatch {
            hunk_index,
            reason: format!(
                "no search block matched (best similarity {:.2} below threshold {:.2})",
                similarity, FUZZY_THRESHOLD
            ),
            nearest_start_line: Some(start + 1),
            nearest_preview: Some(window_preview(lines, start, search_lines.len())),
            nearest_similarity: similarity,
        }),
        FuzzyResult::Empty => Err(NoMatch {
            hunk_index,
            reason: "file has no content to match the search block against".to_string(),
            nearest_start_line: None,
            nearest_preview: None,
            nearest_similarity: 0.0,
        }),
    }
}

/// Split a block into lines, preserving content semantics for matching. A
/// trailing newline produces no spurious empty final element.
fn split_keep_block(block: &str) -> Vec<&str> {
    if block.is_empty() {
        return Vec::new();
    }
    let mut lines: Vec<&str> = block.split('\n').collect();
    // A block that ends in `\n` yields a trailing "" we don't want to match on.
    if block.ends_with('\n') {
        lines.pop();
    }
    lines
}

fn find_perfect(lines: &[&str], search: &[&str], from_line: usize) -> Option<usize> {
    if search.is_empty() || search.len() > lines.len() {
        return None;
    }
    (from_line..=lines.len() - search.len())
        .find(|&start| lines[start..start + search.len()] == *search)
}

fn find_whitespace(lines: &[&str], search: &[&str], from_line: usize) -> Option<usize> {
    if search.is_empty() || search.len() > lines.len() {
        return None;
    }
    let trimmed_search: Vec<&str> = search.iter().map(|line| line.trim()).collect();
    (from_line..=lines.len() - search.len()).find(|&start| {
        lines[start..start + search.len()]
            .iter()
            .map(|line| line.trim())
            .eq(trimmed_search.iter().copied())
    })
}

/// Match a search block whose interior is elided with a `...` marker line:
/// match the lines before the marker, then the lines after, allowing arbitrary
/// content between (aider's dotdotdot). Returns the full matched span.
fn find_dotdotdot(lines: &[&str], search: &[&str], from_line: usize) -> Option<(usize, usize)> {
    let marker = search.iter().position(|line| line.trim() == "...")?;
    let head = &search[..marker];
    let tail = &search[marker + 1..];
    // A leading/trailing-only `...` is ambiguous; require real anchors on both
    // sides so the elision is bounded.
    if head.is_empty() || tail.is_empty() {
        return None;
    }
    let head_start = find_whitespace(lines, head, from_line)?;
    let after_head = head_start + head.len();
    let tail_start = find_whitespace(lines, tail, after_head)?;
    Some((head_start, tail_start + tail.len()))
}

enum FuzzyResult {
    Match { start: usize, similarity: f64 },
    Empty,
}

/// Slide a window the size of `search` over `lines` and return the window with
/// the highest line-similarity ratio.
fn find_fuzzy(lines: &[&str], search: &[&str], from_line: usize) -> FuzzyResult {
    if lines.is_empty() {
        return FuzzyResult::Empty;
    }
    let window = search.len().min(lines.len()).max(1);
    let last = lines.len().saturating_sub(window);
    let mut best_start = from_line.min(last);
    let mut best_similarity = -1.0_f64;
    for start in from_line..=last {
        let candidate = &lines[start..start + window];
        let similarity = block_similarity(search, candidate);
        if similarity > best_similarity {
            best_similarity = similarity;
            best_start = start;
        }
    }
    FuzzyResult::Match {
        start: best_start,
        similarity: best_similarity.max(0.0),
    }
}

/// Line-level similarity of two blocks: the fraction of `search` lines that
/// match (whitespace-insensitively) in order, normalized by the larger block.
fn block_similarity(search: &[&str], candidate: &[&str]) -> f64 {
    let matched = search
        .iter()
        .zip(candidate.iter())
        .filter(|(a, b)| a.trim() == b.trim())
        .count();
    let denom = search.len().max(candidate.len()).max(1);
    matched as f64 / denom as f64
}

fn window_preview(lines: &[&str], start: usize, len: usize) -> String {
    let end = (start + len.max(1)).min(lines.len());
    lines[start..end].join("\n")
}

/// Apply a sequence of located hunks to `original`, returning the new file
/// content, the per-hunk matches, and the changed line ranges (1-based,
/// inclusive) in the resulting file.
#[derive(Debug)]
pub(crate) struct AppliedPatch {
    pub new_content: String,
    pub matches: Vec<HunkMatch>,
    pub changed_line_ranges: Vec<String>,
}

/// Apply all `hunks` to `original`. Hunks are located left-to-right; each hunk
/// search begins after the previous hunk's match so overlapping/identical blocks
/// edit distinct sites. On any hunk miss the whole apply fails with the
/// structured [`NoMatch`] (no partial writes).
pub(crate) fn apply_hunks(original: &str, hunks: &[PatchHunk]) -> Result<AppliedPatch, NoMatch> {
    let original_had_trailing_newline = original.ends_with('\n') || original.is_empty();
    let mut lines: Vec<String> = if original.is_empty() {
        Vec::new()
    } else {
        let mut split: Vec<String> = original.split('\n').map(ToString::to_string).collect();
        if original.ends_with('\n') {
            split.pop();
        }
        split
    };

    let mut matches = Vec::with_capacity(hunks.len());
    let mut changed_line_ranges = Vec::with_capacity(hunks.len());
    let mut cursor = 0usize;

    for (index, hunk) in hunks.iter().enumerate() {
        let view: Vec<&str> = lines.iter().map(String::as_str).collect();
        let located = locate_hunk(&view, hunk, index, cursor)?;
        let replacement: Vec<String> = split_keep_block(&hunk.replace)
            .into_iter()
            .map(ToString::to_string)
            .collect();
        let replaced_len = replacement.len();
        let start = located.start_line;
        let end = located.end_line.min(lines.len());
        lines.splice(start..end, replacement);
        // Record the changed range in the resulting file (1-based inclusive).
        if replaced_len == 0 {
            changed_line_ranges.push(format!("{}:{}", start + 1, start + 1));
        } else {
            changed_line_ranges.push(format!("{}:{}", start + 1, start + replaced_len));
        }
        cursor = start + replaced_len;
        matches.push(located);
    }

    let mut new_content = lines.join("\n");
    if original_had_trailing_newline && !new_content.is_empty() {
        new_content.push('\n');
    } else if original_had_trailing_newline && new_content.is_empty() {
        // An emptied file keeps no trailing newline.
    }
    Ok(AppliedPatch {
        new_content,
        matches,
        changed_line_ranges,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hunk(search: &str, replace: &str) -> PatchHunk {
        PatchHunk {
            search: search.to_string(),
            replace: replace.to_string(),
        }
    }

    #[test]
    fn perfect_match_replaces_in_place() {
        let original = "fn a() {}\nfn b() {}\n";
        let applied =
            apply_hunks(original, &[hunk("fn b() {}\n", "fn b() { 1 }\n")]).expect("apply");
        assert_eq!(applied.new_content, "fn a() {}\nfn b() { 1 }\n");
        assert_eq!(applied.matches[0].strategy, MatchStrategy::Perfect);
    }

    #[test]
    fn whitespace_tolerant_match_ignores_indent() {
        let original = "    fn a() {}\n";
        // Search has no leading indent, file does -- whitespace fallback locates it.
        let applied =
            apply_hunks(original, &[hunk("fn a() {}\n", "fn a() { 2 }\n")]).expect("apply");
        assert_eq!(applied.matches[0].strategy, MatchStrategy::Whitespace);
        assert!(applied.new_content.contains("fn a() { 2 }"));
    }

    #[test]
    fn dotdotdot_match_spans_elided_interior() {
        let original = "start\nmid one\nmid two\nend\n";
        let applied = apply_hunks(original, &[hunk("start\n...\nend\n", "only\n")]).expect("apply");
        assert_eq!(applied.matches[0].strategy, MatchStrategy::DotDotDot);
        assert_eq!(applied.new_content, "only\n");
    }

    #[test]
    fn fuzzy_match_tolerates_a_small_drift() {
        let original = "alpha\nbravo\ncharlie\ndelta\necho\n";
        // One of five lines drifted ("echo" -> "drift") within an
        // otherwise-identical block -> 0.8 similarity, at the fuzzy threshold.
        let applied = apply_hunks(
            original,
            &[hunk("alpha\nbravo\ncharlie\ndelta\ndrift\n", "X\n")],
        )
        .expect("apply");
        assert_eq!(applied.matches[0].strategy, MatchStrategy::Fuzzy);
        assert!(applied.new_content.starts_with("X\n"));
    }

    #[test]
    fn no_match_returns_structured_error_with_nearest_candidate() {
        let original = "one\ntwo\nthree\n";
        let err = apply_hunks(original, &[hunk("completely\nunrelated\nblock\n", "x\n")])
            .expect_err("should not match");
        assert_eq!(err.hunk_index, 0);
        assert!(err.nearest_start_line.is_some());
        assert!(err.nearest_similarity < FUZZY_THRESHOLD);
    }

    #[test]
    fn empty_search_inserts_at_end() {
        let original = "a\n";
        let applied = apply_hunks(original, &[hunk("", "b\n")]).expect("apply");
        assert_eq!(applied.matches[0].strategy, MatchStrategy::Insert);
        assert_eq!(applied.new_content, "a\nb\n");
    }

    #[test]
    fn multiple_hunks_apply_at_distinct_sites_in_order() {
        // Two hunks against different blocks of the same file land independently.
        let original = "alpha\nbeta\ngamma\ndelta\n";
        let applied = apply_hunks(
            original,
            &[hunk("alpha\n", "ALPHA\n"), hunk("delta\n", "DELTA\n")],
        )
        .expect("apply");
        assert_eq!(applied.new_content, "ALPHA\nbeta\ngamma\nDELTA\n");
        assert_eq!(applied.matches.len(), 2);
        // Each hunk's changed range is the resulting-file line it edited (1-based).
        assert_eq!(applied.changed_line_ranges, vec!["1:1", "4:4"]);
    }

    #[test]
    fn two_hunks_target_identical_text_at_different_sites() {
        // Both hunks search the SAME block ("dup\n"); the cursor must advance past
        // the first match so the second hunk edits the SECOND occurrence, not the
        // first again. This is the index-shifting/cursor invariant in apply_hunks.
        let original = "dup\nmiddle\ndup\n";
        let applied = apply_hunks(
            original,
            &[hunk("dup\n", "first\n"), hunk("dup\n", "second\n")],
        )
        .expect("apply");
        assert_eq!(applied.new_content, "first\nmiddle\nsecond\n");
        assert_eq!(applied.matches[0].start_line, 0);
        assert_eq!(applied.matches[1].start_line, 2);
        assert_eq!(applied.changed_line_ranges, vec!["1:1", "3:3"]);
    }

    #[test]
    fn hunk_that_grows_line_count_shifts_later_hunk_correctly() {
        // The first hunk replaces 1 line with 3, growing the file; the second hunk
        // must still locate its block (now shifted down) and report ranges in the
        // RESULTING file, proving the post-splice cursor/range arithmetic is right.
        let original = "head\ntarget\ntail\n";
        let applied = apply_hunks(
            original,
            &[hunk("head\n", "h1\nh2\nh3\n"), hunk("tail\n", "TAIL\n")],
        )
        .expect("apply");
        assert_eq!(applied.new_content, "h1\nh2\nh3\ntarget\nTAIL\n");
        // First hunk replaced line 1 with three lines -> resulting range 1:3.
        // Second hunk's "tail" moved to resulting line 5 -> range 5:5.
        assert_eq!(applied.changed_line_ranges, vec!["1:3", "5:5"]);
    }
}
