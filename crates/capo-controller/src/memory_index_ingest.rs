//! DP6 (memory-architecture.md): persist a [`MemoryJobOutcome`] into Capo's
//! event store at the single controller orchestration seam.
//!
//! The extraction/staleness engine lives in `capo-memory` (`MemoryJobEngine`,
//! pure + storage-free); this module is the PRODUCER that turns its outcome into
//! durable Capo events + projections, mirroring the `acp_replay_ingest` /
//! `CapabilityGrant` producer pattern:
//!
//! - brackets the job with `memory.job_requested` / `memory.job_completed`,
//! - for an `extract_facts` / `rebuild` job, appends one `memory.record_ingested`
//!   event per generated record together with its `memory_records` +
//!   `memory_sources` projections (a record is NEVER written without its source
//!   provenance edge),
//! - for an `invalidate` job, emits `memory.record_invalidated` (and
//!   `memory.record_superseded` when a fresh record replaces a drifted one),
//!   carrying the drift reason so the staleness is auditable.
//!
//! Every event carries a `memory:{job_kind}:{record}:{op}` idempotency key, so a
//! re-run is a no-op and the projections rebuild identically from the log on
//! restart. Generated records land in `review_state = generated`; nothing here
//! promotes them.

use capo_memory::{MemoryJobKind, MemoryJobOutcome};
use capo_state::{
    EventKind, MemoryRecordProjection, MemorySourceProjection, ProjectionRecord, RedactionState,
};

use super::*;

/// What a memory-index job ingest persisted, for assertions + observability.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MemoryJobIngestReport {
    pub memory_job_id: String,
    pub job_kind: String,
    pub ingested_record_count: i64,
    pub rejected_source_count: i64,
    pub invalidated_count: i64,
    pub superseded_count: i64,
    /// Events actually appended (job open/close + per-record events).
    pub appended_event_count: usize,
}

impl FakeBoundaryController {
    /// DP6: derive the fingerprints of the markdown-sourced records Capo ALREADY
    /// holds for the project, so the engine can detect source-hash drift. Capo --
    /// not the engine -- owns this identity read from the durable `memory_records`
    /// + `memory_sources` tables; the engine only sees the fingerprints.
    pub fn indexed_markdown_fingerprints(
        &self,
    ) -> StateResult<Vec<capo_memory::IndexedRecordFingerprint>> {
        let mut fingerprints = Vec::new();
        for record in self.state.memory_records_for_project(&self.project_id)? {
            // Only markdown-sourced records participate in repo-staleness; their
            // single source carries the indexed `source_content_hash`.
            let sources = self
                .state
                .memory_sources_for_record(&record.memory_record_id)?;
            if let Some(source) = sources
                .iter()
                .find(|source| source.source_kind == MemorySourceKind::Markdown.as_str())
                && let Some(hash) = source.source_content_hash.clone()
            {
                fingerprints.push(capo_memory::IndexedRecordFingerprint {
                    record_id: record.memory_record_id.clone(),
                    source_content_hash: hash,
                });
            }
        }
        Ok(fingerprints)
    }

    /// DP6: persist a [`MemoryJobOutcome`] (produced by `MemoryJobEngine`) into
    /// the event store. Returns what it appended.
    pub fn ingest_memory_job(
        &self,
        refs: &FakeRunRefs,
        outcome: &MemoryJobOutcome,
    ) -> StateResult<MemoryJobIngestReport> {
        let job_kind = outcome.job_kind.as_str();
        // The job id is deterministic over the job kind + the record/staleness set
        // it covers, so a re-run of the same job is a no-op.
        let cover_hash = stable_hash(
            outcome
                .records
                .iter()
                .map(|r| r.idempotency_key.as_str())
                .chain(outcome.staleness.iter().map(|s| s.record_id.as_str()))
                .collect::<Vec<_>>()
                .join("|")
                .as_bytes(),
        );
        let memory_job_id = format!("memjob-{job_kind}-{cover_hash}");
        let mut appended_event_count = 0usize;

        // 1. job_requested.
        self.append_memory_event(
            refs,
            EventKind::MemoryJobRequested,
            &memory_job_id,
            "requested",
            &format!("{{\"memory_job_id\":\"{memory_job_id}\",\"job_kind\":\"{job_kind}\"}}"),
            &[],
        )?;
        appended_event_count += 1;

        let mut ingested_record_count = 0i64;
        let mut invalidated_count = 0i64;
        let mut superseded_count = 0i64;

        match outcome.job_kind {
            MemoryJobKind::ExtractFacts | MemoryJobKind::IndexFts | MemoryJobKind::Rebuild => {
                for record in &outcome.records {
                    let memory_record = MemoryRecordProjection {
                        memory_record_id: record.record_id.clone(),
                        project_id: self.project_id.clone(),
                        scope: "project".to_string(),
                        scope_owner_ref: self.project_id.as_str().to_string(),
                        subject_ref: Some(record.source_path.clone()),
                        sensitivity_classification: record.sensitivity.as_str().to_string(),
                        record_kind: "markdown_section".to_string(),
                        subject: record.subject.clone(),
                        predicate: "documented_in".to_string(),
                        object: record.source_path.clone(),
                        body: record.body.clone(),
                        confidence: "heuristic".to_string(),
                        // Generated records are untrusted until promoted.
                        review_state: record.review_state.as_str().to_string(),
                        source_count: 1,
                        valid_from: None,
                        valid_until: None,
                        supersedes_memory_record_id: None,
                        revoked_by_memory_record_id: None,
                        redaction_state: RedactionState::Safe.as_str().to_string(),
                        invalidated_at: None,
                        invalidation_reason: None,
                        packet_item_ref: None,
                        updated_sequence: 0,
                    };
                    let memory_source = MemorySourceProjection {
                        memory_source_id: record.source_id.clone(),
                        memory_record_id: record.record_id.clone(),
                        source_kind: record.source_kind.as_str().to_string(),
                        source_event_id: None,
                        source_artifact_id: None,
                        source_path: Some(record.source_path.clone()),
                        source_anchor: Some(record.source_anchor.clone()),
                        source_content_hash: Some(record.source_content_hash.clone()),
                        source_sequence: None,
                        quote_artifact_id: None,
                        observed_at: Some("1700000000000".to_string()),
                        updated_sequence: 0,
                    };
                    // Append the record + its provenance edge together: a record is
                    // never persisted without its source.
                    self.append_memory_event(
                        refs,
                        EventKind::MemoryRecordIngested,
                        &record.record_id,
                        "ingest",
                        &format!(
                            "{{\"memory_record_id\":\"{}\",\"source_path\":\"{}\",\"source_content_hash\":\"{}\",\"idempotency_key\":\"{}\"}}",
                            record.record_id,
                            record.source_path,
                            record.source_content_hash,
                            record.idempotency_key
                        ),
                        &[
                            ProjectionRecord::MemoryRecord(Box::new(memory_record)),
                            ProjectionRecord::MemorySource(memory_source),
                        ],
                    )?;
                    appended_event_count += 1;
                    ingested_record_count += 1;
                }
            }
            MemoryJobKind::Invalidate => {
                let existing = self.state.memory_records_for_project(&self.project_id)?;
                for transition in &outcome.staleness {
                    // Mark the drifted/removed record invalidated so it is excluded
                    // from the next packet (valid_until / invalidated_at gate it).
                    if let Some(record) = existing
                        .iter()
                        .find(|record| record.memory_record_id == transition.record_id)
                    {
                        let mut invalidated = record.clone();
                        invalidated.review_state = "invalidated".to_string();
                        invalidated.invalidated_at = Some("1700000000002".to_string());
                        invalidated.invalidation_reason = Some(transition.reason.clone());
                        invalidated.valid_until = Some("1700000000002".to_string());
                        if let Some(superseder) = transition.superseded_by.clone() {
                            invalidated.review_state = "superseded".to_string();
                            invalidated.revoked_by_memory_record_id = Some(superseder);
                        }
                        invalidated.updated_sequence = 0;
                        self.append_memory_event(
                            refs,
                            EventKind::MemoryRecordInvalidated,
                            &transition.record_id,
                            "invalidate",
                            &format!(
                                "{{\"memory_record_id\":\"{}\",\"reason\":\"{}\"}}",
                                transition.record_id, transition.reason
                            ),
                            &[ProjectionRecord::MemoryRecord(Box::new(invalidated))],
                        )?;
                        appended_event_count += 1;
                        invalidated_count += 1;

                        if let Some(superseder) = transition.superseded_by.clone() {
                            self.append_memory_event(
                                refs,
                                EventKind::MemoryRecordSuperseded,
                                &transition.record_id,
                                "supersede",
                                &format!(
                                    "{{\"memory_record_id\":\"{}\",\"superseded_by\":\"{}\"}}",
                                    transition.record_id, superseder
                                ),
                                &[],
                            )?;
                            appended_event_count += 1;
                            superseded_count += 1;
                        }
                    }
                }
            }
        }

        // 2. job_completed.
        self.append_memory_event(
            refs,
            EventKind::MemoryJobCompleted,
            &memory_job_id,
            "completed",
            &format!(
                "{{\"memory_job_id\":\"{memory_job_id}\",\"job_kind\":\"{job_kind}\",\"ingested\":{ingested_record_count},\"invalidated\":{invalidated_count},\"superseded\":{superseded_count}}}"
            ),
            &[],
        )?;
        appended_event_count += 1;

        Ok(MemoryJobIngestReport {
            memory_job_id,
            job_kind: job_kind.to_string(),
            ingested_record_count,
            rejected_source_count: outcome.rejected.len() as i64,
            invalidated_count,
            superseded_count,
            appended_event_count,
        })
    }

    fn append_memory_event(
        &self,
        refs: &FakeRunRefs,
        kind: EventKind,
        record_key: &str,
        operation: &str,
        payload_json: &str,
        projections: &[ProjectionRecord],
    ) -> StateResult<i64> {
        let mut event = scoped_event(
            &format!(
                "event-memory-{}-{}-{}",
                kind.as_str().replace('.', "-"),
                slug(record_key),
                operation
            ),
            kind,
            &self.project_id,
            &refs.task_id,
            &refs.agent_id,
            &refs.session_id,
            &refs.run_id,
        );
        event.idempotency_key = Some(format!(
            "memory:{}:{}:{}",
            kind.as_str(),
            record_key,
            operation
        ));
        event.payload_json = payload_json.to_string();
        event.redaction_state = RedactionState::Safe;
        self.state.append_event(event, projections)
    }
}
