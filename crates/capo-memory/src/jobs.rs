//! DP6: memory extraction + staleness `MemoryJob`s, and indexing the working
//! repo's markdown into provenance-bearing memory records.
//!
//! This module is a PURE, deterministic engine (no event store, no IO except an
//! explicit markdown-source read step the caller performs). It produces a
//! [`MemoryJobOutcome`] that the controller seam turns into durable
//! `memory.job_requested` / `memory.job_completed` / `memory.record_invalidated`
//! / `memory.record_superseded` events plus `memory_records` + `memory_sources`
//! projections, exactly mirroring how `acp_replay` produces a plan the
//! controller ingests.
//!
//! It implements the four `MemoryJob` kinds from `memory-architecture.md`:
//!
//! - `extract_facts`: turn a markdown source into generated `MemoryRecord`s,
//!   each with at least one `MemorySource` provenance edge (a record can NEVER
//!   be created without a source).
//! - `index_fts`: register the extracted records into the rebuildable FTS index
//!   (the search side lives in [`crate::SqliteFtsMemoryBackend`]).
//! - `invalidate`: when an indexed source's `source_content_hash` drifts, emit
//!   `record_invalidated` (and `record_superseded` when a fresh record replaces
//!   a drifted one) so stale records are excluded from packets by default.
//! - `rebuild`: re-run extraction/index over the same source ranges and produce
//!   IDENTICAL record IDs via source-range idempotency keys.
//!
//! Discipline enforced here:
//!
//! - Generated records land in `review_state = generated` and cannot supersede a
//!   `reviewed` workpad decision without an explicit promotion
//!   (`promote_generated_record`).
//! - Secrets, credentials, subscription sessions, and raw voice transcripts are
//!   REJECTED as memory sources by default (never indexed).

use crate::{MemoryReviewState, MemorySensitivity, MemorySourceKind};

/// fnv1a64 content hash, matching the `fnv1a64:<hex>` shape used elsewhere in
/// the workspace so a source's `source_content_hash` is comparable across crates.
pub fn content_hash(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("fnv1a64:{hash:016x}")
}

/// The four DP6 `MemoryJob` kinds (the subset of `memory-architecture.md`'s
/// `job_kind` that DP6 implements; `build_packet`/`export` stay with DP5/later).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryJobKind {
    ExtractFacts,
    IndexFts,
    Invalidate,
    Rebuild,
}

impl MemoryJobKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ExtractFacts => "extract_facts",
            Self::IndexFts => "index_fts",
            Self::Invalidate => "invalidate",
            Self::Rebuild => "rebuild",
        }
    }
}

/// A markdown source range to index: a repo-relative path, the heading anchor
/// for the section, the raw section body, and the byte range within the file
/// the section occupies. The `(path, byte_start, byte_end)` triple is the
/// source-range idempotency key: re-running a job over the same range yields the
/// same record ID.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarkdownSourceRange {
    pub path: String,
    pub anchor: String,
    pub body: String,
    pub byte_start: usize,
    pub byte_end: usize,
}

impl MarkdownSourceRange {
    /// The deterministic, source-range idempotency key for this range. Stable
    /// across rebuilds for the same `(path, byte_start, byte_end)` regardless of
    /// body content, so a re-index produces the same record ID.
    pub fn idempotency_key(&self) -> String {
        format!("{}:{}:{}", self.path, self.byte_start, self.byte_end)
    }

    /// The record ID derived from the idempotency key. Stable across rebuilds.
    pub fn record_id(&self) -> String {
        format!("memrec-{}", content_hash(self.idempotency_key().as_bytes()))
    }

    /// The source-provenance edge ID for this range's record.
    pub fn source_id(&self) -> String {
        format!("memsrc-{}", content_hash(self.idempotency_key().as_bytes()))
    }

    pub fn source_content_hash(&self) -> String {
        content_hash(self.body.as_bytes())
    }
}

/// Why a markdown source range was rejected as a memory source.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceRejection {
    /// The body contains credential / secret material.
    Secret,
    /// The body is a raw voice transcript.
    VoiceTranscript,
    /// The body is a subscription-session artifact.
    SubscriptionSession,
}

impl SourceRejection {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Secret => "rejected: secret or credential material is never a memory source",
            Self::VoiceTranscript => "rejected: raw voice transcript is not a memory source",
            Self::SubscriptionSession => {
                "rejected: subscription-session material is not a memory source"
            }
        }
    }
}

/// Classify a source body: `None` if eligible, `Some(reason)` if it must be
/// rejected as a memory source by default. Conservative substring scan: the
/// extraction path is non-destructive (we drop the source, never the file), so
/// false positives only mean a section is not indexed, never data loss.
pub fn classify_source(body: &str) -> Option<SourceRejection> {
    let lower = body.to_lowercase();
    const SECRET_MARKERS: &[&str] = &[
        "api_key",
        "api-key",
        "secret_key",
        "password",
        "authorization: bearer",
        "anthropic_api_key",
        "anthropic_auth_token",
        "private_key",
        "-----begin",
        "session_token",
    ];
    if SECRET_MARKERS.iter().any(|marker| lower.contains(marker)) {
        return Some(SourceRejection::Secret);
    }
    if lower.contains("voice transcript") || lower.contains("[transcript]") {
        return Some(SourceRejection::VoiceTranscript);
    }
    if lower.contains("subscription session") || lower.contains("subscription_session") {
        return Some(SourceRejection::SubscriptionSession);
    }
    None
}

/// A generated memory record produced by `extract_facts`, with its single
/// provenance edge. Generated records are untrusted: `review_state =
/// generated`. The record is replayable from its source range.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExtractedRecord {
    pub record_id: String,
    pub source_id: String,
    pub subject: String,
    pub body: String,
    pub source_path: String,
    pub source_anchor: String,
    pub source_content_hash: String,
    pub idempotency_key: String,
    pub review_state: MemoryReviewState,
    pub sensitivity: MemorySensitivity,
    pub source_kind: MemorySourceKind,
}

/// A drift / staleness transition emitted by an `invalidate` job: the indexed
/// record whose source hash drifted, and (when a fresh range replaces it) the
/// superseding record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StalenessTransition {
    pub record_id: String,
    pub previous_content_hash: String,
    pub current_content_hash: String,
    /// `Some(record_id)` when a fresh extraction supersedes the stale record;
    /// `None` when the source vanished and the record is only invalidated.
    pub superseded_by: Option<String>,
    pub reason: String,
}

/// A source range that was rejected and therefore NOT turned into a record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RejectedSource {
    pub path: String,
    pub anchor: String,
    pub rejection: SourceRejection,
}

/// The full deterministic outcome of running a `MemoryJob`. The controller seam
/// turns this into durable events + projections.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryJobOutcome {
    pub job_kind: MemoryJobKind,
    pub records: Vec<ExtractedRecord>,
    pub rejected: Vec<RejectedSource>,
    pub staleness: Vec<StalenessTransition>,
}

/// A previously-indexed record's identity, used to detect staleness on a
/// re-index: the record ID (derived from its source range) and the
/// `source_content_hash` it was indexed with.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IndexedRecordFingerprint {
    pub record_id: String,
    pub source_content_hash: String,
}

/// The DP6 deterministic extraction/staleness engine. Storage-free: it consumes
/// markdown source ranges (the caller reads the repo) plus the fingerprints of
/// records already indexed, and produces a [`MemoryJobOutcome`].
#[derive(Clone, Copy, Debug, Default)]
pub struct MemoryJobEngine;

impl MemoryJobEngine {
    pub fn new() -> Self {
        Self
    }

    /// `extract_facts`: project eligible markdown source ranges into generated
    /// records, each with exactly one provenance edge. Rejected sources are
    /// reported (never silently dropped) and never become records. Idempotent:
    /// the same ranges produce the same record IDs.
    pub fn extract_facts(&self, ranges: &[MarkdownSourceRange]) -> MemoryJobOutcome {
        let mut records = Vec::new();
        let mut rejected = Vec::new();
        for range in ranges {
            if let Some(rejection) = classify_source(&range.body) {
                rejected.push(RejectedSource {
                    path: range.path.clone(),
                    anchor: range.anchor.clone(),
                    rejection,
                });
                continue;
            }
            records.push(ExtractedRecord {
                record_id: range.record_id(),
                source_id: range.source_id(),
                subject: range.anchor.clone(),
                body: range.body.clone(),
                source_path: range.path.clone(),
                source_anchor: range.anchor.clone(),
                source_content_hash: range.source_content_hash(),
                idempotency_key: range.idempotency_key(),
                review_state: MemoryReviewState::Generated,
                sensitivity: MemorySensitivity::Internal,
                source_kind: MemorySourceKind::Markdown,
            });
        }
        MemoryJobOutcome {
            job_kind: MemoryJobKind::ExtractFacts,
            records,
            rejected,
            staleness: Vec::new(),
        }
    }

    /// `rebuild`: re-run extraction over the same ranges. Because record IDs are
    /// derived from the source-range idempotency key, this yields identical
    /// record IDs to the original `extract_facts` run.
    pub fn rebuild(&self, ranges: &[MarkdownSourceRange]) -> MemoryJobOutcome {
        let mut outcome = self.extract_facts(ranges);
        outcome.job_kind = MemoryJobKind::Rebuild;
        outcome
    }

    /// `invalidate`: detect staleness by comparing the current source ranges
    /// against the fingerprints of already-indexed records. A record whose
    /// source range still exists but whose `source_content_hash` drifted is
    /// invalidated AND superseded by the freshly-extracted record (same record
    /// ID, new content). A record whose source range no longer exists is
    /// invalidated only. Unchanged records produce no transition.
    pub fn invalidate(
        &self,
        ranges: &[MarkdownSourceRange],
        indexed: &[IndexedRecordFingerprint],
    ) -> MemoryJobOutcome {
        let mut staleness = Vec::new();
        for fingerprint in indexed {
            match ranges
                .iter()
                .find(|range| range.record_id() == fingerprint.record_id)
            {
                Some(range) => {
                    let current = range.source_content_hash();
                    if current != fingerprint.source_content_hash {
                        // The fresh extraction (same record ID, new content)
                        // supersedes the drifted record.
                        staleness.push(StalenessTransition {
                            record_id: fingerprint.record_id.clone(),
                            previous_content_hash: fingerprint.source_content_hash.clone(),
                            current_content_hash: current.clone(),
                            superseded_by: Some(range.record_id()),
                            reason: "source_content_hash drift: source edited since indexing"
                                .to_string(),
                        });
                    }
                }
                None => {
                    staleness.push(StalenessTransition {
                        record_id: fingerprint.record_id.clone(),
                        previous_content_hash: fingerprint.source_content_hash.clone(),
                        current_content_hash: String::new(),
                        superseded_by: None,
                        reason: "source range removed: indexed source no longer present"
                            .to_string(),
                    });
                }
            }
        }
        MemoryJobOutcome {
            job_kind: MemoryJobKind::Invalidate,
            records: Vec::new(),
            rejected: Vec::new(),
            staleness,
        }
    }
}

/// Promote a generated record to `reviewed` (the `memory.record_promoted`
/// transition). A generated record can only supersede a reviewed workpad
/// decision AFTER this promotion: callers gate `record_superseded` against a
/// reviewed target on the promoted state of the superseding record.
pub fn promote_generated_record(record: &mut ExtractedRecord) {
    if record.review_state == MemoryReviewState::Generated {
        record.review_state = MemoryReviewState::Reviewed;
    }
}

/// `true` when a (generated) record is allowed to supersede a `reviewed` target.
/// Per `memory-architecture.md`, a generated summary cannot supersede a reviewed
/// workpad decision without an explicit promotion.
pub fn may_supersede_reviewed(superseding: &ExtractedRecord) -> bool {
    superseding.review_state == MemoryReviewState::Reviewed
}

/// Split a markdown document into `##`/`#`-heading source ranges. Each range
/// covers a heading and its body up to the next heading; the byte offsets are
/// the idempotency-key anchor. This is the NON-DESTRUCTIVE indexer: it reads the
/// file's headings into ranges and never rewrites the file (human truth stays
/// authoritative).
pub fn split_markdown_sections(path: &str, contents: &str) -> Vec<MarkdownSourceRange> {
    let mut ranges = Vec::new();
    let bytes = contents.as_bytes();
    // Collect heading byte offsets (lines starting with '#').
    let mut heading_starts: Vec<usize> = Vec::new();
    let mut offset = 0usize;
    for line in contents.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            heading_starts.push(offset);
        }
        offset += line.len();
    }
    if heading_starts.is_empty() {
        return ranges;
    }
    for (index, &start) in heading_starts.iter().enumerate() {
        let end = heading_starts
            .get(index + 1)
            .copied()
            .unwrap_or(bytes.len());
        let section = &contents[start..end];
        let anchor = section
            .lines()
            .next()
            .unwrap_or("")
            .trim_start_matches('#')
            .trim()
            .to_string();
        ranges.push(MarkdownSourceRange {
            path: path.to_string(),
            anchor,
            body: section.trim_end().to_string(),
            byte_start: start,
            byte_end: end,
        });
    }
    ranges
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(
        path: &str,
        anchor: &str,
        body: &str,
        start: usize,
        end: usize,
    ) -> MarkdownSourceRange {
        MarkdownSourceRange {
            path: path.to_string(),
            anchor: anchor.to_string(),
            body: body.to_string(),
            byte_start: start,
            byte_end: end,
        }
    }

    #[test]
    fn extract_facts_attaches_a_provenance_edge_to_every_record() {
        let engine = MemoryJobEngine::new();
        let ranges = vec![range(
            "workpads/depth/knowledge.md",
            "DP6",
            "Memory jobs index the repo with provenance.",
            0,
            44,
        )];
        let outcome = engine.extract_facts(&ranges);
        assert_eq!(outcome.records.len(), 1);
        let record = &outcome.records[0];
        // A record can never be created without a source: it carries a source
        // id, a source path, and a source content hash.
        assert!(!record.source_id.is_empty());
        assert_eq!(record.source_path, "workpads/depth/knowledge.md");
        assert!(record.source_content_hash.starts_with("fnv1a64:"));
        // Generated records are untrusted.
        assert_eq!(record.review_state, MemoryReviewState::Generated);
    }

    #[test]
    fn credential_and_voice_and_subscription_sources_are_rejected() {
        let engine = MemoryJobEngine::new();
        let ranges = vec![
            range("a.md", "secret", "export ANTHROPIC_API_KEY=sk-abc", 0, 10),
            range("b.md", "voice", "Voice transcript: hello there", 0, 10),
            range("c.md", "sub", "subscription session cookie", 0, 10),
            range("d.md", "ok", "A normal reviewed decision note.", 0, 10),
        ];
        let outcome = engine.extract_facts(&ranges);
        assert_eq!(outcome.records.len(), 1);
        assert_eq!(outcome.records[0].source_path, "d.md");
        assert_eq!(outcome.rejected.len(), 3);
        assert!(
            outcome
                .rejected
                .iter()
                .any(|r| r.rejection == SourceRejection::Secret)
        );
        assert!(
            outcome
                .rejected
                .iter()
                .any(|r| r.rejection == SourceRejection::VoiceTranscript)
        );
        assert!(
            outcome
                .rejected
                .iter()
                .any(|r| r.rejection == SourceRejection::SubscriptionSession)
        );
    }

    #[test]
    fn rebuild_yields_identical_record_ids() {
        let engine = MemoryJobEngine::new();
        let ranges = vec![
            range("x.md", "A", "alpha body", 0, 11),
            range("x.md", "B", "beta body", 11, 20),
        ];
        let first: Vec<String> = engine
            .extract_facts(&ranges)
            .records
            .into_iter()
            .map(|r| r.record_id)
            .collect();
        let rebuilt: Vec<String> = engine
            .rebuild(&ranges)
            .records
            .into_iter()
            .map(|r| r.record_id)
            .collect();
        assert_eq!(first, rebuilt);
        assert_eq!(first.len(), 2);
    }

    #[test]
    fn drifted_source_hash_invalidates_and_supersedes() {
        let engine = MemoryJobEngine::new();
        let original = range("k.md", "Anchor", "original body", 0, 40);
        let indexed = vec![IndexedRecordFingerprint {
            record_id: original.record_id(),
            source_content_hash: original.source_content_hash(),
        }];
        // The source body changed (same byte range -> same record id).
        let edited = range("k.md", "Anchor", "EDITED body", 0, 40);
        let outcome = engine.invalidate(std::slice::from_ref(&edited), &indexed);
        assert_eq!(outcome.staleness.len(), 1);
        let transition = &outcome.staleness[0];
        assert_eq!(transition.record_id, edited.record_id());
        assert_eq!(
            transition.superseded_by.as_deref(),
            Some(edited.record_id().as_str())
        );
        assert!(transition.reason.contains("drift"));
    }

    #[test]
    fn removed_source_invalidates_without_supersede() {
        let engine = MemoryJobEngine::new();
        let gone = range("gone.md", "Anchor", "body", 0, 40);
        let indexed = vec![IndexedRecordFingerprint {
            record_id: gone.record_id(),
            source_content_hash: gone.source_content_hash(),
        }];
        let outcome = engine.invalidate(&[], &indexed);
        assert_eq!(outcome.staleness.len(), 1);
        assert!(outcome.staleness[0].superseded_by.is_none());
    }

    #[test]
    fn unchanged_source_produces_no_staleness() {
        let engine = MemoryJobEngine::new();
        let stable = range("s.md", "Anchor", "stable body", 0, 40);
        let indexed = vec![IndexedRecordFingerprint {
            record_id: stable.record_id(),
            source_content_hash: stable.source_content_hash(),
        }];
        let outcome = engine.invalidate(std::slice::from_ref(&stable), &indexed);
        assert!(outcome.staleness.is_empty());
    }

    #[test]
    fn generated_record_cannot_supersede_reviewed_without_promotion() {
        let engine = MemoryJobEngine::new();
        let ranges = vec![range("p.md", "A", "a generated fact", 0, 10)];
        let mut record = engine.extract_facts(&ranges).records.remove(0);
        assert!(!may_supersede_reviewed(&record));
        promote_generated_record(&mut record);
        assert_eq!(record.review_state, MemoryReviewState::Reviewed);
        assert!(may_supersede_reviewed(&record));
    }

    #[test]
    fn split_markdown_sections_is_nondestructive_and_range_keyed() {
        let doc = "# Title\n\nintro\n\n## Section A\n\nbody a\n\n## Section B\n\nbody b\n";
        let ranges = split_markdown_sections("doc.md", doc);
        assert_eq!(ranges.len(), 3);
        assert_eq!(ranges[0].anchor, "Title");
        assert_eq!(ranges[1].anchor, "Section A");
        assert_eq!(ranges[2].anchor, "Section B");
        // Reconstruct: the concatenated section byte-ranges cover the whole doc
        // (the indexer reads, it does not rewrite).
        let last = ranges.last().unwrap();
        assert_eq!(last.byte_end, doc.len());
        // Distinct byte ranges -> distinct idempotency keys -> distinct record ids.
        let ids: std::collections::HashSet<_> = ranges.iter().map(|r| r.record_id()).collect();
        assert_eq!(ids.len(), 3);
    }
}
