//! DP6: index the working repo's markdown sources into provenance-bearing
//! memory records, and prove the rebuild is idempotent (re-indexing the same
//! source ranges yields identical searchable record IDs).
//!
//! This reads REAL markdown from the working tree (`workpads/**/knowledge.md`
//! and the architecture source docs) through the non-destructive section
//! indexer, runs the `extract_facts` and `rebuild` MemoryJobs, and asserts the
//! provenance + idempotency invariants. It is deterministic: it never spawns a
//! provider and never writes back to the repo.

use capo_memory::{
    IndexedRecordFingerprint, MemoryJobEngine, MemoryJobKind, MemoryReviewState, MemorySourceKind,
    split_markdown_sections,
};
use std::path::{Path, PathBuf};

/// The repo root is two levels up from this crate's manifest dir
/// (`crates/capo-memory` -> repo root).
fn repo_root() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(Path::parent)
        .expect("repo root")
        .to_path_buf()
}

/// Gather the working repo's markdown source ranges from a curated set of real
/// files that exist in the worktree.
fn working_repo_ranges() -> Vec<capo_memory::MarkdownSourceRange> {
    let root = repo_root();
    let candidates = [
        "workpads/depth/knowledge.md",
        "workpads/depth/tasks.md",
        "workpads/architecture/memory-architecture.md",
    ];
    let mut ranges = Vec::new();
    for relative in candidates {
        let path = root.join(relative);
        let contents = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        ranges.extend(split_markdown_sections(relative, &contents));
    }
    ranges
}

#[test]
fn indexes_working_repo_markdown_with_provenance() {
    let ranges = working_repo_ranges();
    assert!(
        ranges.len() > 5,
        "expected several markdown sections from the working repo, got {}",
        ranges.len()
    );

    let outcome = MemoryJobEngine::new().extract_facts(&ranges);
    assert_eq!(outcome.job_kind, MemoryJobKind::ExtractFacts);
    assert!(
        !outcome.records.is_empty(),
        "the working repo markdown produced at least one memory record"
    );

    for record in &outcome.records {
        // Every indexed record carries a real provenance edge: a workpad/source
        // path, a markdown source kind, and a content hash of the section body.
        assert_eq!(record.source_kind, MemorySourceKind::Markdown);
        assert!(record.source_path.ends_with(".md"));
        assert!(record.source_content_hash.starts_with("fnv1a64:"));
        // Indexed records are untrusted until reviewed.
        assert_eq!(record.review_state, MemoryReviewState::Generated);
        // Reusing the non-destructive indexer: the body is a copy of the repo
        // section, never a rewrite of the file.
        assert!(!record.body.is_empty());
    }
}

#[test]
fn reindexing_the_repo_yields_identical_record_ids() {
    let ranges = working_repo_ranges();
    let engine = MemoryJobEngine::new();

    let first: Vec<String> = engine
        .extract_facts(&ranges)
        .records
        .into_iter()
        .map(|record| record.record_id)
        .collect();
    // `rebuild` re-runs extraction over the same source ranges.
    let rebuilt: Vec<String> = engine
        .rebuild(&ranges)
        .records
        .into_iter()
        .map(|record| record.record_id)
        .collect();

    assert_eq!(
        first, rebuilt,
        "re-indexing the repo from the same source ranges must yield identical record IDs"
    );
}

#[test]
fn editing_an_indexed_repo_section_invalidates_and_supersedes() {
    let ranges = working_repo_ranges();
    let engine = MemoryJobEngine::new();
    let records = engine.extract_facts(&ranges).records;
    assert!(!records.is_empty());

    // Snapshot the current index as fingerprints.
    let indexed: Vec<IndexedRecordFingerprint> = records
        .iter()
        .map(|record| IndexedRecordFingerprint {
            record_id: record.record_id.clone(),
            source_content_hash: record.source_content_hash.clone(),
        })
        .collect();

    // Simulate an operator editing one section in place (same byte range -> same
    // record id, drifted content hash).
    let mut edited = ranges.clone();
    edited[0].body.push_str("\n\nAn operator added a new line.");

    let outcome = engine.invalidate(&edited, &indexed);
    assert_eq!(outcome.job_kind, MemoryJobKind::Invalidate);
    assert_eq!(
        outcome.staleness.len(),
        1,
        "exactly the one edited section drifts"
    );
    let transition = &outcome.staleness[0];
    assert_eq!(transition.record_id, edited[0].record_id());
    assert_eq!(
        transition.superseded_by.as_deref(),
        Some(edited[0].record_id().as_str()),
        "the freshly-extracted record supersedes the drifted one"
    );

    // Re-running invalidate after the edit is itself idempotent: with the edited
    // content now treated as the indexed baseline, there is no further drift.
    let reindexed: Vec<IndexedRecordFingerprint> = edited
        .iter()
        .map(|range| IndexedRecordFingerprint {
            record_id: range.record_id(),
            source_content_hash: range.source_content_hash(),
        })
        .collect();
    let stable = engine.invalidate(&edited, &reindexed);
    assert!(stable.staleness.is_empty());
}
