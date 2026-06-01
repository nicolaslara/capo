//! DP2 (acp-replay-dedupe.md): persist an [`AcpReplayPlan`] into Capo's event
//! store at the single controller orchestration seam.
//!
//! The reconciliation REDUCER lives in `capo-adapters` (`AcpReplayEngine`); this
//! module is the producer that turns its plan into durable Capo events +
//! projections, mirroring the existing `PermissionApproval`/`CapabilityGrant`
//! pattern where the producer appends the event together with its projection
//! record. Concretely it:
//!
//! - opens an `adapter_replay_batches` row (`adapter.replay_started` for a load,
//!   `adapter.attach_started` for a resume),
//! - appends one `adapter.raw_update_observed` event + `adapter_raw_updates`
//!   projection per raw frame (raw observations, persisted BEFORE normalization;
//!   they never mutate read models directly),
//! - records each derived `adapter_timeline_keys` row,
//! - emits, per reconciled candidate, a `adapter.replay_duplicate_detected` /
//!   `adapter.replay_ambiguous` marker (no item events) or imports the missing
//!   item, and
//! - finalizes the batch with `adapter.replay_completed` / `adapter.attach_completed`
//!   carrying the imported/duplicate/ambiguous counts.
//!
//! Capo owns durable identity, so [`Self::acp_existing_item_fingerprints`] reads
//! the existing read models and hands the engine the fingerprints it reconciles
//! against -- the engine never reaches into storage. Every event carries the
//! design's `acp:{session}:{event_family}:{timeline_key}:{operation}:{version}`
//! idempotency-key shape, so a replay re-run is a no-op and the projections
//! rebuild identically from the log on restart.

use capo_adapters::{
    AcpReconcileDecision, AcpReplayPlan, AcpReplaySource, AcpTimelineKind, ExistingItemFingerprint,
};
use capo_state::{
    AdapterRawUpdateProjection, AdapterReplayBatchProjection, AdapterTimelineKeyProjection,
    EventKind, ProjectionRecord,
};

use super::*;

/// What an ACP replay/attach ingest persisted, for assertions + observability.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AcpReplayIngestReport {
    pub acp_replay_batch_id: String,
    pub raw_update_count: i64,
    pub timeline_key_count: i64,
    pub imported_count: i64,
    pub duplicate_count: i64,
    pub ambiguous_count: i64,
    /// Events actually appended (batch open/close + raw observations + markers).
    pub appended_event_count: usize,
}

impl FakeBoundaryController {
    /// DP2: derive the fingerprints of the items Capo ALREADY holds for the
    /// session from the DURABLE read models, so the reconciliation engine can
    /// decide duplicates. Capo -- not the adapter -- owns this identity read: the
    /// `adapter_timeline_keys` table is the authoritative record of which stable
    /// keys + synthetic message anchors Capo has already projected, so a re-run of
    /// the same `session/load` reconciles every candidate against the keys the
    /// first load recorded (no duplicate UI items). The reducer never touches
    /// storage; it only sees these fingerprints.
    pub fn acp_existing_item_fingerprints(
        &self,
        refs: &FakeRunRefs,
    ) -> StateResult<Vec<ExistingItemFingerprint>> {
        let mut fingerprints = Vec::new();
        for key in self
            .state
            .adapter_timeline_keys_for_session(&refs.session_id)?
        {
            // Tool/plan keys dedupe by the stable timeline key; message keys carry
            // their synthetic `role:content_hash` anchor, which the engine's
            // content-hash + role anchor match also catches.
            let (role, content_hash) = match key.synthetic_ref.as_deref() {
                Some(synthetic) => synthetic
                    .split_once(':')
                    .map(|(role, hash)| (Some(role.to_string()), hash.to_string()))
                    .unwrap_or((None, String::new())),
                None => (None, String::new()),
            };
            fingerprints.push(ExistingItemFingerprint {
                timeline_key: Some(key.adapter_timeline_key_id.clone()),
                role,
                content_hash,
            });
        }
        Ok(fingerprints)
    }

    /// DP2: persist an [`AcpReplayPlan`] (built by `AcpReplayEngine`) into the
    /// event store. Returns what it appended.
    pub fn ingest_acp_replay_plan(
        &self,
        refs: &FakeRunRefs,
        plan: &AcpReplayPlan,
    ) -> StateResult<AcpReplayIngestReport> {
        let session = self
            .state
            .session(&refs.session_id)?
            .ok_or_else(|| missing_read_model("session", &refs.session_id))?;
        let is_attach = plan.source == AcpReplaySource::SessionResumeAttach;
        let batch_id = format!(
            "acp-replay-batch-{}-{}-{}",
            refs.session_id,
            plan.source.as_str(),
            stable_hash(plan.external_session_ref.as_bytes())
        );
        let mut appended_event_count = 0usize;

        // 1. Open the batch (replay_started / attach_started).
        let open_status = AdapterReplayBatchProjection::STATUS_OPEN.to_string();
        let mut batch = AdapterReplayBatchProjection {
            acp_replay_batch_id: batch_id.clone(),
            session_id: refs.session_id.clone(),
            project_id: self.project_id.clone(),
            external_session_ref: plan.external_session_ref.clone(),
            source: plan.source.as_str().to_string(),
            status: open_status,
            load_request_id: None,
            prompt_request_id: None,
            recovery_attempt_id: None,
            raw_update_count: plan.raw_update_count(),
            imported_count: 0,
            duplicate_count: 0,
            ambiguous_count: 0,
            normalized_sequence_start: None,
            normalized_sequence_end: None,
            started_at: Some("1700000000000".to_string()),
            completed_at: None,
            updated_sequence: 0,
        };
        let open_kind = if is_attach {
            EventKind::AdapterAttachStarted
        } else {
            EventKind::AdapterReplayStarted
        };
        let start_sequence = self.append_acp_event(
            refs,
            open_kind,
            &batch_id,
            "open",
            &[ProjectionRecord::AdapterReplayBatch(batch.clone())],
        )?;
        appended_event_count += 1;

        // 2. Persist every raw update as a raw observation (before normalization).
        for raw in &plan.raw_updates {
            let raw_update_id = format!("{batch_id}-raw-{:05}", raw.batch_index);
            let projection = AdapterRawUpdateProjection {
                acp_raw_update_id: raw_update_id.clone(),
                acp_replay_batch_id: batch_id.clone(),
                project_id: self.project_id.clone(),
                external_session_ref: plan.external_session_ref.clone(),
                batch_index: raw.batch_index,
                jsonrpc_method: raw.jsonrpc_method.clone(),
                session_update_kind: raw.session_update_kind.clone(),
                external_item_ref: raw.external_item_ref.clone(),
                acp_timeline_key: raw.acp_timeline_key.clone(),
                payload_hash: raw.payload_hash.clone(),
                // Large payloads ride as an artifact ref; the fixtures store the
                // hash only, with the artifact ref derived from the hash so a
                // future large-payload path has a stable name to point at.
                payload_artifact_id: Some(format!("artifact-acp-raw-{}", raw.payload_hash)),
                replay_source: plan.source.as_str().to_string(),
                dedupe_confidence: raw.dedupe_confidence.raw_str().to_string(),
                observed_at: Some("1700000000001".to_string()),
                updated_sequence: 0,
            };
            self.append_acp_event(
                refs,
                EventKind::AdapterRawUpdateObserved,
                &raw_update_id,
                "observe",
                &[ProjectionRecord::AdapterRawUpdate(projection)],
            )?;
            appended_event_count += 1;
        }

        // 3. Record each derived timeline key. The canonical ACP timeline key IS
        //    the row id (it is globally unique within a session and is exactly the
        //    key the engine reconciles against on a later load), so a re-run reads
        //    back the same id and dedupes structurally.
        for key in &plan.timeline_keys {
            let projection = AdapterTimelineKeyProjection {
                adapter_timeline_key_id: key.timeline_key.clone(),
                session_id: refs.session_id.clone(),
                project_id: self.project_id.clone(),
                external_session_ref: plan.external_session_ref.clone(),
                kind: key.kind.as_str().to_string(),
                stable_ref: key.stable_ref.clone(),
                synthetic_ref: key.synthetic_ref.clone(),
                confidence: key.confidence.timeline_str().to_string(),
                first_sequence: Some(start_sequence),
                last_sequence: Some(start_sequence),
                updated_sequence: 0,
            };
            // The timeline key rides on the batch (no separate event family of its
            // own in the design); reuse the raw-update-observed kind so the record
            // is event-sourced. It is metadata, never a UI item.
            let timeline_key_id = projection.adapter_timeline_key_id.clone();
            self.append_acp_event(
                refs,
                EventKind::AdapterRawUpdateObserved,
                &timeline_key_id,
                "timeline-key",
                &[ProjectionRecord::AdapterTimelineKey(projection)],
            )?;
            appended_event_count += 1;
        }

        // 4. Reconcile each candidate: a duplicate/ambiguous marker (no item
        //    events) or an imported summary item.
        for candidate in &plan.candidates {
            match candidate.decision {
                AcpReconcileDecision::Duplicate => {
                    self.append_acp_event(
                        refs,
                        EventKind::AdapterReplayDuplicateDetected,
                        &slug(&candidate.timeline_key),
                        // Carry the import/reconciliation confidence on the marker so
                        // a low-confidence dedupe is auditable, not silently dropped.
                        &format!(
                            "duplicate:{}:{}",
                            candidate.import_confidence.as_str(),
                            candidate.content_hash
                        ),
                        &[],
                    )?;
                    appended_event_count += 1;
                }
                AcpReconcileDecision::Ambiguous => {
                    self.append_acp_event(
                        refs,
                        EventKind::AdapterReplayAmbiguous,
                        &slug(&candidate.timeline_key),
                        &format!(
                            "ambiguous:{}:{}",
                            candidate.import_confidence.as_str(),
                            candidate.content_hash
                        ),
                        &[],
                    )?;
                    appended_event_count += 1;
                    // An ambiguous candidate is imported with LOW confidence: it
                    // becomes a real (auditable) item, but flagged.
                    appended_event_count +=
                        self.import_candidate_item(refs, &session, candidate, true)?;
                }
                AcpReconcileDecision::Imported => {
                    appended_event_count +=
                        self.import_candidate_item(refs, &session, candidate, false)?;
                }
            }
        }

        // 5. Finalize the batch with the reconciliation counts.
        batch.status = AdapterReplayBatchProjection::STATUS_COMPLETED.to_string();
        batch.imported_count = plan.imported_count();
        batch.duplicate_count = plan.duplicate_count();
        batch.ambiguous_count = plan.ambiguous_count();
        batch.completed_at = Some("1700000000999".to_string());
        let close_kind = if is_attach {
            EventKind::AdapterAttachCompleted
        } else {
            EventKind::AdapterReplayCompleted
        };
        self.append_acp_event(
            refs,
            close_kind,
            &batch_id,
            "complete",
            &[ProjectionRecord::AdapterReplayBatch(batch)],
        )?;
        appended_event_count += 1;

        Ok(AcpReplayIngestReport {
            acp_replay_batch_id: batch_id,
            raw_update_count: plan.raw_update_count(),
            timeline_key_count: plan.timeline_keys.len() as i64,
            imported_count: plan.imported_count(),
            duplicate_count: plan.duplicate_count(),
            ambiguous_count: plan.ambiguous_count(),
            appended_event_count,
        })
    }

    /// Import a reconciled candidate as a normalized read-model item, reusing the
    /// SAME `apply_normalized_adapter_events` ingestion route every provider uses
    /// (never a parallel route). Returns how many events the import appended.
    fn import_candidate_item(
        &self,
        refs: &FakeRunRefs,
        _session: &SessionProjection,
        candidate: &capo_adapters::AcpReconciledCandidate,
        _low_confidence: bool,
    ) -> StateResult<usize> {
        // A plan/session-info candidate is metadata, not a UI item -- the batch +
        // timeline-key rows already record it. Only message/tool candidates import
        // as read-model items, through the shared route.
        if matches!(
            candidate.kind,
            AcpTimelineKind::Plan | AcpTimelineKind::SessionInfo
        ) {
            return Ok(0);
        }
        let report = self.apply_normalized_adapter_events(
            refs,
            std::slice::from_ref(&candidate.representative),
        )?;
        Ok(report.appended_event_count)
    }

    /// Append one ACP attach/replay event with the design's idempotency-key shape
    /// `acp:{session}:{event_family}:{timeline_key/record}:{operation}` and the
    /// supplied projections, mirroring the existing producer pattern.
    fn append_acp_event(
        &self,
        refs: &FakeRunRefs,
        kind: EventKind,
        record_key: &str,
        operation: &str,
        projections: &[ProjectionRecord],
    ) -> StateResult<i64> {
        let mut event = scoped_event(
            &format!(
                "event-acp-{}-{}-{}-{}",
                kind.as_str().replace('.', "-"),
                refs.session_id,
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
            "acp:{}:{}:{}:{}",
            refs.session_id,
            kind.as_str(),
            record_key,
            operation
        ));
        event.redaction_state = RedactionState::Safe;
        self.state.append_event(event, projections)
    }
}
