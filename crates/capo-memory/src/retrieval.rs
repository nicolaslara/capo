//! DP5: real memory retrieval backends.
//!
//! Two real backends sit alongside `FakeMemoryBackend`:
//!
//! - [`MarkdownMemoryBackend`] supplies workpad / source pointers with content
//!   hashes (the human-facing decision memory in `workpads/**`). It turns a set
//!   of curated markdown sources into [`MemoryCandidate`]s with `Markdown`
//!   provenance, so the live packet derives from real source pointers rather
//!   than the four hardcoded literal candidates.
//! - [`SqliteFtsMemoryBackend`] provides FTS5 search/ranking over
//!   `memory_records`-shaped candidates. It implements the
//!   `search(MemoryQuery, MemoryBudget)` contract from `memory-architecture.md`:
//!   it builds a rebuildable FTS5 index over candidate text, ranks matches by
//!   `bm25`, filters out invalidated/rejected/superseded/secret/unreviewed
//!   records (defense-in-depth on top of the `capo-state`
//!   `packet_eligible_memory_records` SQL filter), and returns a budget-bounded
//!   selection that retains per-item inclusion reasons and excluded-reason
//!   decisions.
//!
//! No vector / embedding / graph backend is required for this first retrieval
//! path; those stay deferred per the architecture doc.

use crate::{
    MemoryCandidate, MemoryPacketDecision, MemoryReviewState, MemorySensitivity, MemorySourceKind,
    MemorySourceRef,
};
use rusqlite::Connection;

/// A retrieval query against a memory backend.
///
/// Mirrors the `search(MemoryQuery, MemoryBudget)` contract: a free-text query
/// (matched against subject/predicate/object/body via FTS5) plus the eligible
/// candidate set the caller is authorized to see. The candidate set is the
/// already-eligibility-filtered output of
/// `SqliteQueries::packet_eligible_memory_records` projected into
/// [`MemoryCandidate`]s; the backend never reaches back into operational state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryQuery {
    /// Free-text retrieval query (e.g. the active task goal).
    pub text: String,
    /// The eligible candidate corpus to rank and filter.
    pub candidates: Vec<MemoryCandidate>,
}

impl MemoryQuery {
    pub fn new(text: impl Into<String>, candidates: Vec<MemoryCandidate>) -> Self {
        Self {
            text: text.into(),
            candidates,
        }
    }
}

/// A token budget for a retrieval/packet build.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemoryBudget {
    pub budget_tokens: usize,
}

impl MemoryBudget {
    pub fn new(budget_tokens: usize) -> Self {
        Self { budget_tokens }
    }
}

/// One ranked, budget-eligible retrieval hit.
#[derive(Clone, Debug, PartialEq)]
pub struct MemoryHit {
    pub candidate: MemoryCandidate,
    /// FTS5 `bm25` rank (lower is a better match); ties broken by insertion
    /// order so retrieval is deterministic across rebuilds.
    pub rank: f64,
}

/// The full result of a [`SqliteFtsMemoryBackend::search`]: the ranked hits
/// that fit the budget, plus the auditable excluded-reason decisions for the
/// rest (no-match, ineligible, over-budget).
#[derive(Clone, Debug, PartialEq)]
pub struct MemorySearchResult {
    pub hits: Vec<MemoryHit>,
    pub excluded: Vec<MemoryPacketDecision>,
}

/// `MarkdownMemoryBackend`: workpad / source pointers with content hashes.
///
/// It does not parse arbitrary markdown here (that is the DP6 indexer's job);
/// it carries a curated set of source pointers (path + anchor + content hash +
/// body) so the live packet derives from real markdown provenance.
#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct MarkdownMemoryBackend {
    sources: Vec<MarkdownSource>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarkdownSource {
    pub title: String,
    pub path: String,
    pub anchor: Option<String>,
    pub content_hash: String,
    pub body: String,
    pub inclusion_reason: String,
}

impl MarkdownMemoryBackend {
    pub fn new(sources: Vec<MarkdownSource>) -> Self {
        Self { sources }
    }

    /// Project the curated markdown source pointers into reviewed
    /// [`MemoryCandidate`]s with `Markdown` provenance. Workpad/source pointers
    /// are human-reviewed decision memory, so they enter the candidate set as
    /// `Reviewed` / `Internal`.
    pub fn candidates(&self) -> Vec<MemoryCandidate> {
        self.sources
            .iter()
            .map(|source| MemoryCandidate {
                title: source.title.clone(),
                body: source.body.clone(),
                source: MemorySourceRef {
                    source_kind: MemorySourceKind::Markdown,
                    source_ref: source.path.clone(),
                    anchor: source.anchor.clone(),
                    content_hash: source.content_hash.clone(),
                },
                review_state: MemoryReviewState::Reviewed,
                sensitivity: MemorySensitivity::Internal,
                estimated_tokens: estimate_tokens(&source.body),
                inclusion_reason: source.inclusion_reason.clone(),
            })
            .collect()
    }
}

/// `SqliteFtsMemoryBackend`: FTS5 search/ranking over candidate records.
#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct SqliteFtsMemoryBackend {
    /// Monotonic index version; bumped on each rebuild so an index entry's
    /// `index_version` is stable for a given rebuild.
    pub index_version: i64,
}

impl SqliteFtsMemoryBackend {
    pub fn new() -> Self {
        Self { index_version: 1 }
    }

    pub fn with_index_version(index_version: i64) -> Self {
        Self { index_version }
    }

    /// `search(MemoryQuery, MemoryBudget)`: build a fresh in-process FTS5 index
    /// over the eligible candidates, rank by `bm25` against the query, drop
    /// ineligible records (defense-in-depth), and select greedily within the
    /// token budget. The FTS index is rebuildable: the same candidates + query
    /// always yield the same ordered record set.
    pub fn search(
        &self,
        query: &MemoryQuery,
        budget: MemoryBudget,
    ) -> Result<MemorySearchResult, FtsError> {
        let connection = Connection::open_in_memory()?;
        connection.execute_batch(
            "CREATE VIRTUAL TABLE memory_fts USING fts5(
                ordinal UNINDEXED,
                subject,
                body
            );",
        )?;

        // Index every candidate; record the row ordinal so retrieval ties break
        // deterministically and so we can recover the original candidate.
        {
            let mut insert = connection
                .prepare("INSERT INTO memory_fts(ordinal, subject, body) VALUES (?1, ?2, ?3)")?;
            for (ordinal, candidate) in query.candidates.iter().enumerate() {
                insert.execute(rusqlite::params![
                    ordinal as i64,
                    candidate.title.as_str(),
                    candidate.body.as_str(),
                ])?;
            }
        }

        // Rank the matching rows. An empty/whitespace query matches nothing via
        // FTS, so fall back to insertion order (rank 0.0) for a no-query packet.
        let match_expr = fts_match_expression(&query.text);
        let mut ranked: Vec<(usize, f64)> = Vec::new();
        if let Some(match_expr) = match_expr {
            let mut statement = connection.prepare(
                "SELECT ordinal, bm25(memory_fts) AS rank
                 FROM memory_fts
                 WHERE memory_fts MATCH ?1
                 ORDER BY rank ASC, ordinal ASC",
            )?;
            let rows = statement.query_map(rusqlite::params![match_expr], |row| {
                Ok((row.get::<_, i64>(0)? as usize, row.get::<_, f64>(1)?))
            })?;
            for row in rows {
                ranked.push(row?);
            }
        }

        // Records that did not match the query are not retrieved; record them as
        // auditable no-match exclusions so the packet explanation is honest.
        let matched: std::collections::HashSet<usize> =
            ranked.iter().map(|(ordinal, _)| *ordinal).collect();

        let mut hits = Vec::new();
        let mut excluded = Vec::new();
        let mut used_budget = 0usize;

        for (ordinal, rank) in &ranked {
            let candidate = &query.candidates[*ordinal];
            if let Some(reason) = ineligible_reason(candidate) {
                excluded.push(decision(candidate, reason));
                continue;
            }
            if used_budget + candidate.estimated_tokens > budget.budget_tokens {
                excluded.push(decision(
                    candidate,
                    "excluded: packet budget exhausted".to_string(),
                ));
                continue;
            }
            used_budget += candidate.estimated_tokens;
            hits.push(MemoryHit {
                candidate: candidate.clone(),
                rank: *rank,
            });
        }

        for (ordinal, candidate) in query.candidates.iter().enumerate() {
            if matched.contains(&ordinal) {
                continue;
            }
            excluded.push(decision(
                candidate,
                "excluded: no FTS match for the retrieval query".to_string(),
            ));
        }

        Ok(MemorySearchResult { hits, excluded })
    }

    /// The set of searchable record refs the index would return for `query`,
    /// independent of budget. Used by the restart/replay test to prove a rebuilt
    /// FTS index returns the SAME searchable record IDs.
    pub fn searchable_refs(&self, query: &MemoryQuery) -> Result<Vec<String>, FtsError> {
        let result = self.search(query, MemoryBudget::new(usize::MAX))?;
        Ok(result
            .hits
            .into_iter()
            .map(|hit| hit.candidate.source.source_ref)
            .collect())
    }
}

/// Reason a candidate is ineligible for a packet, or `None` if eligible.
///
/// This mirrors `MemoryRecordProjection::is_packet_eligible` /
/// `packet_eligible_memory_records` at the candidate level: secret material and
/// non-reviewed records (generated/rejected/superseded/invalidated) are never
/// packet memory.
fn ineligible_reason(candidate: &MemoryCandidate) -> Option<String> {
    if candidate.sensitivity == MemorySensitivity::Secret {
        return Some("excluded: secret or credential material is never packet memory".to_string());
    }
    if candidate.review_state != MemoryReviewState::Reviewed {
        return Some(format!(
            "excluded: review_state={}",
            candidate.review_state.as_str()
        ));
    }
    None
}

fn decision(candidate: &MemoryCandidate, reason: String) -> MemoryPacketDecision {
    MemoryPacketDecision {
        source: candidate.source.clone(),
        title: candidate.title.clone(),
        reason,
        estimated_tokens: candidate.estimated_tokens,
    }
}

/// Build an FTS5 `MATCH` expression from free text: OR the individual terms so
/// any term hit ranks the row, and quote each term so punctuation/operators in
/// the goal text cannot become FTS5 syntax. Returns `None` when there is no
/// usable term (empty/whitespace query).
fn fts_match_expression(text: &str) -> Option<String> {
    let terms: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|term| !term.is_empty())
        .map(|term| format!("\"{}\"", term.to_lowercase()))
        .collect();
    if terms.is_empty() {
        return None;
    }
    Some(terms.join(" OR "))
}

/// Cheap deterministic token estimate (~4 chars/token, min 1) used when a
/// markdown source does not carry a pre-computed estimate.
fn estimate_tokens(body: &str) -> usize {
    (body.len() / 4).max(1)
}

#[derive(Debug)]
pub enum FtsError {
    Sqlite(rusqlite::Error),
}

impl From<rusqlite::Error> for FtsError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

impl std::fmt::Display for FtsError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sqlite(error) => write!(formatter, "fts5 backend error: {error}"),
        }
    }
}

impl std::error::Error for FtsError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(
        title: &str,
        body: &str,
        source_ref: &str,
        review_state: MemoryReviewState,
        sensitivity: MemorySensitivity,
        estimated_tokens: usize,
    ) -> MemoryCandidate {
        MemoryCandidate {
            title: title.to_string(),
            body: body.to_string(),
            source: MemorySourceRef {
                source_kind: MemorySourceKind::Markdown,
                source_ref: source_ref.to_string(),
                anchor: None,
                content_hash: format!("fnv1a64:{source_ref}"),
            },
            review_state,
            sensitivity,
            estimated_tokens,
            inclusion_reason: "retrieved by FTS".to_string(),
        }
    }

    fn reviewed(title: &str, body: &str, source_ref: &str, tokens: usize) -> MemoryCandidate {
        candidate(
            title,
            body,
            source_ref,
            MemoryReviewState::Reviewed,
            MemorySensitivity::Internal,
            tokens,
        )
    }

    #[test]
    fn fts_search_ranks_more_relevant_record_first() {
        let backend = SqliteFtsMemoryBackend::new();
        let query = MemoryQuery::new(
            "sandbox confinement policy",
            vec![
                reviewed(
                    "Unrelated",
                    "Dashboard rendering uses the read model only.",
                    "doc:dashboard",
                    10,
                ),
                reviewed(
                    "Sandbox policy",
                    "The sandbox confinement policy denies egress outside the granted scope.",
                    "doc:sandbox",
                    10,
                ),
            ],
        );
        let result = backend.search(&query, MemoryBudget::new(256)).unwrap();
        assert_eq!(result.hits.len(), 1);
        assert_eq!(result.hits[0].candidate.source.source_ref, "doc:sandbox");
        // The unrelated record is recorded as an auditable no-match exclusion.
        assert!(
            result
                .excluded
                .iter()
                .any(|d| d.reason.contains("no FTS match"))
        );
    }

    #[test]
    fn fts_search_excludes_secret_and_unreviewed_and_superseded() {
        let backend = SqliteFtsMemoryBackend::new();
        let query = MemoryQuery::new(
            "deploy release process",
            vec![
                reviewed(
                    "Deploy doc",
                    "The deploy release process is documented.",
                    "doc:deploy",
                    10,
                ),
                candidate(
                    "Secret",
                    "deploy release token=shhh",
                    "doc:secret",
                    MemoryReviewState::Reviewed,
                    MemorySensitivity::Secret,
                    10,
                ),
                candidate(
                    "Generated",
                    "generated deploy release note",
                    "doc:generated",
                    MemoryReviewState::Generated,
                    MemorySensitivity::Internal,
                    10,
                ),
                candidate(
                    "Superseded",
                    "old deploy release process",
                    "doc:superseded",
                    MemoryReviewState::Superseded,
                    MemorySensitivity::Internal,
                    10,
                ),
            ],
        );
        let result = backend.search(&query, MemoryBudget::new(256)).unwrap();
        let hit_refs: Vec<_> = result
            .hits
            .iter()
            .map(|h| h.candidate.source.source_ref.as_str())
            .collect();
        assert_eq!(hit_refs, vec!["doc:deploy"]);
        assert!(result.excluded.iter().any(|d| d.reason.contains("secret")));
        assert!(
            result
                .excluded
                .iter()
                .any(|d| d.reason.contains("review_state=generated"))
        );
        assert!(
            result
                .excluded
                .iter()
                .any(|d| d.reason.contains("review_state=superseded"))
        );
    }

    #[test]
    fn fts_search_is_budget_bounded() {
        let backend = SqliteFtsMemoryBackend::new();
        let query = MemoryQuery::new(
            "policy",
            vec![
                reviewed("First", "policy one", "doc:1", 40),
                reviewed("Second", "policy two", "doc:2", 40),
                reviewed("Third", "policy three", "doc:3", 40),
            ],
        );
        let result = backend.search(&query, MemoryBudget::new(80)).unwrap();
        // Only two of three 40-token records fit an 80-token budget.
        assert_eq!(result.hits.len(), 2);
        assert!(
            result
                .excluded
                .iter()
                .any(|d| d.reason.contains("budget exhausted"))
        );
    }

    #[test]
    fn rebuilt_index_returns_the_same_searchable_record_ids() {
        let query = MemoryQuery::new(
            "policy retrieval",
            vec![
                reviewed("A", "policy retrieval alpha", "doc:a", 10),
                reviewed("B", "policy retrieval beta", "doc:b", 10),
                reviewed("C", "unrelated", "doc:c", 10),
            ],
        );
        let first = SqliteFtsMemoryBackend::new()
            .searchable_refs(&query)
            .unwrap();
        // Rebuild with a bumped index version: searchable record IDs are stable.
        let rebuilt = SqliteFtsMemoryBackend::with_index_version(2)
            .searchable_refs(&query)
            .unwrap();
        assert_eq!(first, rebuilt);
        assert_eq!(first, vec!["doc:a".to_string(), "doc:b".to_string()]);
    }

    #[test]
    fn live_packet_replays_byte_for_byte_across_rebuilds() {
        use crate::{LiveMemoryPacketRequest, MemoryBackend};
        use capo_core::{MemoryPacketId, SessionId};

        let request = || LiveMemoryPacketRequest {
            memory_packet_id: MemoryPacketId::new("packet-dp5-replay"),
            session_id: SessionId::new("session-dp5"),
            run_id: "run-dp5".to_string(),
            turn_id: "turn-dp5".to_string(),
            purpose: "turn_context".to_string(),
            budget_tokens: 256,
            query_text: "policy retrieval".to_string(),
            candidates: vec![
                reviewed("Alpha", "policy retrieval alpha note", "doc:a", 10),
                reviewed("Beta", "policy retrieval beta note", "doc:b", 10),
                reviewed("Gamma", "unrelated content", "doc:c", 10),
            ],
        };

        let first = MemoryBackend::sqlite_fts(SqliteFtsMemoryBackend::with_index_version(1))
            .build_live_packet(request())
            .unwrap();
        // Rebuild with a fresh index version: the packet markdown + artifact id
        // reconstruct byte-for-byte (the replayability anchor).
        let rebuilt = MemoryBackend::sqlite_fts(SqliteFtsMemoryBackend::with_index_version(2))
            .build_live_packet(request())
            .unwrap();
        assert_eq!(first.packet_markdown, rebuilt.packet_markdown);
        assert_eq!(first.packet_artifact_id, rebuilt.packet_artifact_id);
        assert_eq!(first.included.len(), rebuilt.included.len());
        // Only the two matching records are included; the unrelated one is a
        // recorded no-match exclusion.
        assert_eq!(first.included.len(), 2);
        assert!(
            first
                .excluded
                .iter()
                .any(|d| d.reason.contains("no FTS match"))
        );
    }

    #[test]
    fn markdown_backend_projects_source_pointers_into_reviewed_candidates() {
        let backend = MarkdownMemoryBackend::new(vec![MarkdownSource {
            title: "Workpad authority".to_string(),
            path: "workpads/depth/knowledge.md".to_string(),
            anchor: Some("DP5".to_string()),
            content_hash: "fnv1a64:depth-knowledge".to_string(),
            body: "Depth deepens the working harness; tasks are last in sequence.".to_string(),
            inclusion_reason: "current workpad is the planning authority".to_string(),
        }]);
        let candidates = backend.candidates();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].review_state, MemoryReviewState::Reviewed);
        assert_eq!(candidates[0].source.source_kind, MemorySourceKind::Markdown);
        assert_eq!(candidates[0].source.anchor.as_deref(), Some("DP5"));
        assert!(candidates[0].estimated_tokens >= 1);
    }
}
