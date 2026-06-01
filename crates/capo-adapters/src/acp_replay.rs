//! DP2 (acp-replay-dedupe.md): the ACP replay/reconciliation engine.
//!
//! This is the conservative adapter ingestion layer the design mandates. It is a
//! PURE, deterministic reducer: given the ordered raw `session/update` frames of
//! a `session/load` (or the empty stream of a `session/resume` attach) plus the
//! fingerprints of the items Capo already holds for the external session, it
//! produces an [`AcpReplayPlan`] -- the batch summary, the ordered raw-update
//! records, the derived timeline keys, and the per-candidate reconciliation
//! decisions (accepted import / duplicate observation / ambiguous).
//!
//! The engine never touches a database. Capo owns durable identity, so the
//! controller seam (`capo-controller`) consumes an [`AcpReplayPlan`] and appends
//! the `adapter.replay_*` / `adapter.attach_*` events plus the read-model
//! projections to the event store. Keeping the reducer in `capo-adapters` and the
//! persistence in `capo-controller` is the same single-ingestion-route boundary
//! DP1 established: ACP stays an adapter, and no `session/update` is directly
//! authoritative for read models.
//!
//! Design rules realized here:
//! - Raw updates are persisted BEFORE normalization and never mutate read models
//!   directly (every frame becomes an [`AcpRawUpdateRecord`]; large payloads ride
//!   as an artifact ref + hash, never inline).
//! - Tool calls dedupe by stable `toolCallId` (`acp:{session}:tool:{id}`);
//!   `tool_call_update` `content`/`locations` are replacement fields, so repeated
//!   identical updates collapse to ONE candidate (one read model) while EVERY raw
//!   frame is still retained as a raw observation.
//! - Plans use one `acp:{session}:plan:current` key and are full replacements.
//! - Message chunks lack stable ACP v1 IDs, so consecutive same-type chunks are
//!   grouped into a finalized candidate with a `content_hash` and a
//!   `message_boundary_confidence` that drops to `low` when more than one chunk
//!   collapses into a group (the boundary is genuinely ambiguous).
//! - Reconciliation: a finalized candidate whose role + content hash matches an
//!   existing Capo item is a duplicate observation (no item events); a no-match is
//!   an imported item; a low-confidence candidate that neither cleanly matches nor
//!   is clearly new is imported as ambiguous (low import confidence).

use serde_json::Value;

use crate::event::{stable_hash, string_at};
use crate::provider_parsers::AcpAdapter;
use crate::{AdapterTimelineConfidence, NormalizedAdapterEvent};

/// Which ingest a replay batch records, mirroring the `AcpReplayBatch.source`
/// vocabulary in `acp-replay-dedupe.md`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AcpReplaySource {
    /// A live `session/prompt` turn stream.
    LivePrompt,
    /// A `session/load` history replay (import / repair / resume-less reconnect).
    SessionLoad,
    /// A `session/resume` reconnect attach (NO history replay).
    SessionResumeAttach,
    /// A Capo restart-recovery ingest.
    RestartRecovery,
    /// A foreign-session import where Capo has no local history.
    ForeignImport,
}

impl AcpReplaySource {
    /// The persisted `source` string, matching
    /// `AdapterReplayBatchProjection::SOURCE_*` in `capo-state`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LivePrompt => "live_prompt",
            Self::SessionLoad => "session_load",
            Self::SessionResumeAttach => "session_resume_attach",
            Self::RestartRecovery => "restart_recovery",
            Self::ForeignImport => "foreign_import",
        }
    }
}

/// The protocol-aware confidence of a derived timeline key / dedupe decision,
/// mirroring `AcpRawUpdate.dedupe_confidence` and `AcpTimelineKey.confidence`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AcpDedupeConfidence {
    /// A stable external id keyed the entry (`toolCallId`, plan-current).
    Stable,
    /// Inferred from ACP update ordering (a single message group).
    Heuristic,
    /// Genuinely ambiguous (consecutive same-type chunks may be one or many
    /// messages). `as_str` renders this as `"low"` for timeline keys and `"none"`
    /// for raw-update dedupe, matching the design's two field vocabularies.
    Low,
}

impl AcpDedupeConfidence {
    /// The `AdapterTimelineKeyProjection::confidence` string (`stable`/`heuristic`/
    /// `low`).
    pub const fn timeline_str(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Heuristic => "heuristic",
            Self::Low => "low",
        }
    }

    /// The `AdapterRawUpdateProjection::dedupe_confidence` string
    /// (`stable`/`heuristic`/`none`). A genuinely ambiguous chunk has no usable
    /// dedupe key, so it renders as `none`.
    pub const fn raw_str(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Heuristic => "heuristic",
            Self::Low => "none",
        }
    }

    fn from_timeline(confidence: AdapterTimelineConfidence) -> Self {
        match confidence {
            AdapterTimelineConfidence::Stable => Self::Stable,
            AdapterTimelineConfidence::Heuristic => Self::Heuristic,
            AdapterTimelineConfidence::None => Self::Low,
        }
    }
}

/// The kind of timeline a key addresses.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AcpTimelineKind {
    Tool,
    Plan,
    Message,
    SessionInfo,
}

impl AcpTimelineKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Tool => "tool",
            Self::Plan => "plan",
            Self::Message => "message",
            Self::SessionInfo => "session_info",
        }
    }
}

/// One raw `session/update` frame observed during a batch, persisted BEFORE
/// normalization. Identity is `(batch, batch_index)`; large payloads ride as a
/// hash + artifact ref, never inline.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcpRawUpdateRecord {
    pub batch_index: i64,
    pub jsonrpc_method: String,
    pub session_update_kind: Option<String>,
    pub external_item_ref: Option<String>,
    pub acp_timeline_key: Option<String>,
    pub payload_hash: String,
    pub dedupe_confidence: AcpDedupeConfidence,
}

/// A protocol-aware timeline key derived from the batch's raw updates.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcpTimelineKeyRecord {
    pub timeline_key: String,
    pub kind: AcpTimelineKind,
    /// The stable external id (`toolCallId` / messageId) when present.
    pub stable_ref: Option<String>,
    /// The synthetic ref (role + content-hash window) when no stable id exists.
    pub synthetic_ref: Option<String>,
    pub confidence: AcpDedupeConfidence,
}

/// What reconciliation decided for one finalized candidate item.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AcpReconcileDecision {
    /// No existing Capo item matched: import the missing historical item.
    Imported,
    /// Role + content hash matched an existing Capo item: a duplicate
    /// observation (NOT re-projected as a UI item).
    Duplicate,
    /// A low-confidence / ambiguous match: imported with low import confidence.
    Ambiguous,
}

/// DP3 (acp-replay-dedupe.md): how confident the import/reconciliation of a
/// candidate is, recorded so a low-confidence reconciliation is AUDITABLE rather
/// than silently projected.
///
/// `Stable`/`Heuristic`/`None` mirror the dedupe-confidence vocabulary: a stable
/// timeline-keyed import is `stable`, a single inferred message group is
/// `heuristic`, and an ambiguous collapsed-chunk import (or a duplicate observation
/// with no usable key) is `none`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AcpImportConfidence {
    Stable,
    Heuristic,
    None,
}

impl AcpImportConfidence {
    /// The persisted string (`stable` / `heuristic` / `none`).
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Heuristic => "heuristic",
            Self::None => "none",
        }
    }

    fn from_dedupe(confidence: AcpDedupeConfidence) -> Self {
        match confidence {
            AcpDedupeConfidence::Stable => Self::Stable,
            AcpDedupeConfidence::Heuristic => Self::Heuristic,
            AcpDedupeConfidence::Low => Self::None,
        }
    }
}

/// A finalized candidate item the load replay staged, with its reconciliation
/// decision. Message chunks are finalized to a content hash + chunk count; tool
/// calls collapse their replacement updates into a single latest-state candidate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcpReconciledCandidate {
    pub timeline_key: String,
    pub kind: AcpTimelineKind,
    pub role: Option<String>,
    /// The finalized normalized content hash (role+content for a message, latest
    /// state for a tool/plan).
    pub content_hash: String,
    /// How many raw frames collapsed into this candidate.
    pub chunk_count: i64,
    pub boundary_confidence: AcpDedupeConfidence,
    pub decision: AcpReconcileDecision,
    /// DP3: how confident the import/reconciliation is, recorded so a
    /// low-confidence reconciliation is AUDITABLE rather than silently projected. A
    /// stable-keyed import is `stable`, a single inferred message group is
    /// `heuristic`, and an ambiguous collapsed-chunk import (or a duplicate
    /// observation with no usable key) is `none`.
    pub import_confidence: AcpImportConfidence,
    /// The latest normalized event for this candidate (drives the imported item's
    /// projection at the controller seam).
    pub representative: NormalizedAdapterEvent,
}

/// The fingerprint of an item Capo ALREADY holds for the external session, used
/// to decide whether a replayed candidate is a duplicate. The controller derives
/// these from the durable read models before calling the engine, keeping Capo the
/// owner of durable identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExistingItemFingerprint {
    /// The stable timeline key when one exists (tool calls, plan-current).
    pub timeline_key: Option<String>,
    pub role: Option<String>,
    /// The normalized content hash of the existing item.
    pub content_hash: String,
}

/// The complete, deterministic outcome of reconciling one ACP ingest. The
/// controller turns this into events + projections; nothing here touches storage.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcpReplayPlan {
    pub source: AcpReplaySource,
    pub external_session_ref: String,
    pub raw_updates: Vec<AcpRawUpdateRecord>,
    pub timeline_keys: Vec<AcpTimelineKeyRecord>,
    pub candidates: Vec<AcpReconciledCandidate>,
    /// Raw attach/load response metadata retained as a raw observation (the
    /// `session/resume` response, or the `session/load` response object).
    pub response_payload_hash: Option<String>,
}

impl AcpReplayPlan {
    pub fn raw_update_count(&self) -> i64 {
        self.raw_updates.len() as i64
    }

    pub fn imported_count(&self) -> i64 {
        self.candidates
            .iter()
            .filter(|c| c.decision == AcpReconcileDecision::Imported)
            .count() as i64
    }

    pub fn duplicate_count(&self) -> i64 {
        self.candidates
            .iter()
            .filter(|c| c.decision == AcpReconcileDecision::Duplicate)
            .count() as i64
    }

    pub fn ambiguous_count(&self) -> i64 {
        self.candidates
            .iter()
            .filter(|c| c.decision == AcpReconcileDecision::Ambiguous)
            .count() as i64
    }
}

/// The reconciliation engine. Stateless; every method is a pure function of its
/// inputs so the controller and the deterministic fixtures exercise the IDENTICAL
/// reducer.
pub struct AcpReplayEngine;

impl AcpReplayEngine {
    /// Build the plan for a `session/resume` ATTACH: emit NO item replay
    /// candidates and NO derived item timeline keys, only the single raw
    /// observation of the resume response. This realizes the design's "resume
    /// creates no message/item replay events" invariant structurally -- there is
    /// no path here that can produce a candidate.
    pub fn plan_resume_attach(
        external_session_ref: &str,
        resume_response: &Value,
    ) -> AcpReplayPlan {
        let payload_hash = stable_hash(resume_response.to_string().as_bytes());
        AcpReplayPlan {
            source: AcpReplaySource::SessionResumeAttach,
            external_session_ref: external_session_ref.to_string(),
            raw_updates: vec![AcpRawUpdateRecord {
                batch_index: 0,
                jsonrpc_method: "session/resume".to_string(),
                session_update_kind: None,
                external_item_ref: None,
                acp_timeline_key: None,
                payload_hash: payload_hash.clone(),
                dedupe_confidence: AcpDedupeConfidence::Stable,
            }],
            timeline_keys: Vec::new(),
            candidates: Vec::new(),
            response_payload_hash: Some(payload_hash),
        }
    }

    /// Build the plan for a `session/load` (or restart-recovery / foreign-import)
    /// replay: persist every raw frame, derive timeline keys, stage and finalize
    /// candidates, then reconcile each finalized candidate against the existing
    /// Capo item fingerprints.
    ///
    /// `raw_frames` is the ordered list of raw `session/update` JSON values exactly
    /// as they arrived off the wire (the transcript the wire client pumped). The
    /// engine re-normalizes each frame through the SAME `parse_acp_record` path so
    /// the staged candidates match the live ingestion route.
    pub fn plan_load(
        source: AcpReplaySource,
        external_session_ref: &str,
        raw_frames: &[Value],
        existing_items: &[ExistingItemFingerprint],
    ) -> AcpReplayPlan {
        let mut raw_updates = Vec::with_capacity(raw_frames.len());
        // Candidate accumulator keyed by timeline key, preserving first-seen order
        // so the plan is deterministic across runs.
        let mut order: Vec<String> = Vec::new();
        let mut staged: std::collections::HashMap<String, StagedCandidate> =
            std::collections::HashMap::new();

        for (index, frame) in raw_frames.iter().enumerate() {
            let normalized = AcpAdapter::normalize_update(frame);
            // A frame normalizes to exactly one event in the ACP mapper, but loop
            // defensively so a future multi-event mapping still records every raw
            // observation against the frame's own index.
            let mut frame_timeline_key: Option<String> = None;
            let mut frame_kind: Option<String> = None;
            let mut frame_item_ref: Option<String> = None;
            let mut frame_confidence = AcpDedupeConfidence::Stable;

            for event in normalized {
                let Some(timeline_key) = event.timeline_key.clone() else {
                    continue;
                };
                let kind = classify_timeline_kind(&event);
                let confidence =
                    AcpDedupeConfidence::from_timeline(event.timeline_confidence.clone());
                frame_timeline_key = Some(timeline_key.clone());
                frame_kind = Some(event.kind.clone());
                frame_item_ref = event.external_item_ref.clone();
                frame_confidence = confidence;

                let entry = staged.entry(timeline_key.clone()).or_insert_with(|| {
                    order.push(timeline_key.clone());
                    StagedCandidate::new(timeline_key.clone(), kind)
                });
                entry.observe(&event);
            }

            raw_updates.push(AcpRawUpdateRecord {
                batch_index: index as i64,
                jsonrpc_method: string_at(frame, &["method"])
                    .unwrap_or_else(|| "session/update".to_string()),
                session_update_kind: frame
                    .pointer("/params/update/sessionUpdate")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or_else(|| frame_kind.clone()),
                external_item_ref: frame_item_ref,
                acp_timeline_key: frame_timeline_key,
                payload_hash: stable_hash(frame.to_string().as_bytes()),
                dedupe_confidence: frame_confidence,
            });
        }

        let mut timeline_keys = Vec::new();
        let mut candidates = Vec::new();
        for key in &order {
            let staged = staged
                .remove(key)
                .expect("every ordered key was staged above");
            let finalized = staged.finalize();
            timeline_keys.push(AcpTimelineKeyRecord {
                timeline_key: finalized.timeline_key.clone(),
                kind: finalized.kind,
                stable_ref: finalized.stable_ref.clone(),
                synthetic_ref: finalized.synthetic_ref.clone(),
                confidence: finalized.boundary_confidence,
            });
            let decision = reconcile(&finalized, existing_items);
            // The import/reconciliation confidence tracks the candidate's boundary
            // confidence: a stable-keyed candidate imports as `stable`, a single
            // inferred message group as `heuristic`, and a collapsed ambiguous chunk
            // (or any low-boundary candidate) as `none`, so a low-confidence
            // reconciliation is auditable rather than silently projected.
            let import_confidence = AcpImportConfidence::from_dedupe(finalized.boundary_confidence);
            candidates.push(AcpReconciledCandidate {
                timeline_key: finalized.timeline_key,
                kind: finalized.kind,
                role: finalized.role,
                content_hash: finalized.content_hash,
                chunk_count: finalized.chunk_count,
                boundary_confidence: finalized.boundary_confidence,
                decision,
                import_confidence,
                representative: finalized.representative,
            });
        }

        AcpReplayPlan {
            source,
            external_session_ref: external_session_ref.to_string(),
            raw_updates,
            timeline_keys,
            candidates,
            response_payload_hash: None,
        }
    }
}

/// Classify a normalized ACP event into a timeline kind from its derived key /
/// event kind (the `parse_acp_record` mapper already shaped the key).
fn classify_timeline_kind(event: &NormalizedAdapterEvent) -> AcpTimelineKind {
    if event.kind == "adapter.plan_replaced" {
        AcpTimelineKind::Plan
    } else if event.is_tool_event() {
        AcpTimelineKind::Tool
    } else if event.kind == "adapter.session_started" {
        AcpTimelineKind::SessionInfo
    } else {
        AcpTimelineKind::Message
    }
}

/// A candidate being accumulated across raw frames before finalization.
struct StagedCandidate {
    timeline_key: String,
    kind: AcpTimelineKind,
    role: Option<String>,
    /// Concatenated content across grouped chunks / latest replacement state.
    content: String,
    chunk_count: i64,
    stable_ref: Option<String>,
    confidence: AcpDedupeConfidence,
    representative: Option<NormalizedAdapterEvent>,
}

struct FinalizedCandidate {
    timeline_key: String,
    kind: AcpTimelineKind,
    role: Option<String>,
    content_hash: String,
    chunk_count: i64,
    boundary_confidence: AcpDedupeConfidence,
    stable_ref: Option<String>,
    synthetic_ref: Option<String>,
    representative: NormalizedAdapterEvent,
}

impl StagedCandidate {
    fn new(timeline_key: String, kind: AcpTimelineKind) -> Self {
        Self {
            timeline_key,
            kind,
            role: None,
            content: String::new(),
            chunk_count: 0,
            stable_ref: None,
            confidence: AcpDedupeConfidence::Stable,
            representative: None,
        }
    }

    fn observe(&mut self, event: &NormalizedAdapterEvent) {
        self.chunk_count += 1;
        self.role = event.role.clone().or_else(|| self.role.clone());
        self.stable_ref = event
            .external_item_ref
            .clone()
            .or_else(|| self.stable_ref.clone());
        self.confidence = AcpDedupeConfidence::from_timeline(event.timeline_confidence.clone());
        match self.kind {
            // Tool calls and plans are REPLACEMENT fields: the latest frame's
            // content/locations replace the collection, so the candidate carries
            // only the latest observed content, not the concatenation.
            AcpTimelineKind::Tool | AcpTimelineKind::Plan | AcpTimelineKind::SessionInfo => {
                if let Some(content) = &event.content {
                    self.content = content.clone();
                }
            }
            // Message chunks APPEND: a message group accretes its chunks until a
            // boundary closes it.
            AcpTimelineKind::Message => {
                if let Some(content) = &event.content {
                    self.content.push_str(content);
                }
            }
        }
        self.representative = Some(event.clone());
    }

    fn finalize(self) -> FinalizedCandidate {
        let role = self.role.clone();
        // The finalized normalized content hash compares role + content, the
        // dedupe key the design's reconciliation uses.
        let content_hash = stable_hash(
            format!("{}:{}", role.as_deref().unwrap_or("none"), self.content).as_bytes(),
        );
        // Boundary confidence: a stable-keyed candidate (tool/plan) stays at its
        // observed confidence; a message group that collapsed MORE THAN ONE
        // consecutive same-type chunk has a genuinely ambiguous boundary, so it
        // drops to `low` (the design's "consecutive same-type chunks may be one or
        // many messages" rule), regardless of the per-frame heuristic confidence.
        let boundary_confidence = match self.kind {
            AcpTimelineKind::Message if self.chunk_count > 1 => AcpDedupeConfidence::Low,
            _ => self.confidence,
        };
        let (stable_ref, synthetic_ref) = match self.kind {
            AcpTimelineKind::Tool => (self.stable_ref.clone(), None),
            AcpTimelineKind::Plan | AcpTimelineKind::SessionInfo => {
                (Some(self.timeline_key.clone()), None)
            }
            AcpTimelineKind::Message => (
                None,
                Some(format!(
                    "{}:{}",
                    role.as_deref().unwrap_or("none"),
                    content_hash
                )),
            ),
        };
        let representative = self.representative.unwrap_or_else(|| {
            NormalizedAdapterEvent::new(
                crate::NormalizedAdapterKind::Acp,
                "adapter.raw_event",
                "unknown",
                &Value::Null,
            )
        });
        FinalizedCandidate {
            timeline_key: self.timeline_key,
            kind: self.kind,
            role,
            content_hash,
            chunk_count: self.chunk_count,
            boundary_confidence,
            stable_ref,
            synthetic_ref,
            representative,
        }
    }
}

/// Reconcile one finalized candidate against Capo's existing item fingerprints.
///
/// Order of the design's rules:
/// 1. Stable timeline-key match -> duplicate (the same tool/plan key already
///    exists; the replacement state is a duplicate observation, not a new item).
/// 2. Content-hash + role match (anchor) -> duplicate observation.
/// 3. No match, high/medium confidence -> import the missing item.
/// 4. No match, low boundary confidence -> import as AMBIGUOUS (low import
///    confidence) rather than silently creating a possibly-duplicate UI item.
fn reconcile(
    candidate: &FinalizedCandidate,
    existing: &[ExistingItemFingerprint],
) -> AcpReconcileDecision {
    let stable_key_match = existing.iter().any(|item| {
        item.timeline_key
            .as_deref()
            .is_some_and(|key| key == candidate.timeline_key)
    });
    if stable_key_match {
        return AcpReconcileDecision::Duplicate;
    }
    let content_match = existing
        .iter()
        .any(|item| item.content_hash == candidate.content_hash && item.role == candidate.role);
    if content_match {
        return AcpReconcileDecision::Duplicate;
    }
    match candidate.boundary_confidence {
        AcpDedupeConfidence::Low => AcpReconcileDecision::Ambiguous,
        _ => AcpReconcileDecision::Imported,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn update(session: &str, body: Value) -> Value {
        json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": { "sessionId": session, "update": body }
        })
    }

    /// Fixture 5: repeated identical `tool_call_update` for the same `toolCallId`
    /// yields ONE read-model candidate (replacement semantics) while EVERY raw
    /// frame is retained as a raw observation.
    #[test]
    fn repeated_tool_call_update_collapses_to_one_candidate_with_raw_duplicates() {
        let frames = vec![
            update(
                "s1",
                json!({
                    "sessionUpdate": "tool_call",
                    "toolCallId": "tool-1",
                    "title": "write",
                    "status": "in_progress"
                }),
            ),
            update(
                "s1",
                json!({
                    "sessionUpdate": "tool_call_update",
                    "toolCallId": "tool-1",
                    "status": "completed",
                    "content": { "type": "text", "text": "done" }
                }),
            ),
            update(
                "s1",
                json!({
                    "sessionUpdate": "tool_call_update",
                    "toolCallId": "tool-1",
                    "status": "completed",
                    "content": { "type": "text", "text": "done" }
                }),
            ),
        ];
        let plan = AcpReplayEngine::plan_load(AcpReplaySource::SessionLoad, "s1", &frames, &[]);
        // Three raw observations retained.
        assert_eq!(plan.raw_update_count(), 3);
        // ONE tool candidate (one read model) keyed by the stable toolCallId.
        assert_eq!(plan.candidates.len(), 1);
        let candidate = &plan.candidates[0];
        assert_eq!(candidate.kind, AcpTimelineKind::Tool);
        assert_eq!(candidate.timeline_key, "acp:s1:tool:tool-1");
        assert_eq!(candidate.chunk_count, 3, "all raw frames collapsed");
        assert_eq!(candidate.decision, AcpReconcileDecision::Imported);
        // ONE stable timeline key.
        assert_eq!(plan.timeline_keys.len(), 1);
        assert_eq!(
            plan.timeline_keys[0].confidence,
            AcpDedupeConfidence::Stable
        );
    }

    /// Fixture 6: consecutive same-type `agent_message_chunk` updates WITHOUT
    /// stable message IDs collapse into one group with LOW boundary confidence
    /// (the boundary is genuinely ambiguous).
    #[test]
    fn idless_consecutive_chunks_record_low_boundary_confidence() {
        // Two consecutive agent chunks with the SAME text -> the ACP mapper keys
        // them by role+content-hash, so they land in one message group.
        let frames = vec![
            update(
                "s1",
                json!({
                    "sessionUpdate": "agent_message_chunk",
                    "content": { "type": "text", "text": "hello" }
                }),
            ),
            update(
                "s1",
                json!({
                    "sessionUpdate": "agent_message_chunk",
                    "content": { "type": "text", "text": "hello" }
                }),
            ),
        ];
        let plan = AcpReplayEngine::plan_load(AcpReplaySource::SessionLoad, "s1", &frames, &[]);
        assert_eq!(plan.raw_update_count(), 2);
        assert_eq!(plan.candidates.len(), 1, "one message group");
        let candidate = &plan.candidates[0];
        assert_eq!(candidate.kind, AcpTimelineKind::Message);
        assert!(candidate.chunk_count > 1);
        assert_eq!(
            candidate.boundary_confidence,
            AcpDedupeConfidence::Low,
            "collapsed consecutive same-type chunks are low confidence"
        );
        // A low-confidence, no-existing-match candidate imports as AMBIGUOUS.
        assert_eq!(candidate.decision, AcpReconcileDecision::Ambiguous);
        assert_eq!(plan.ambiguous_count(), 1);
        // The timeline key is synthetic with low confidence.
        let key = &plan.timeline_keys[0];
        assert!(key.stable_ref.is_none());
        assert!(key.synthetic_ref.is_some());
        assert_eq!(key.confidence, AcpDedupeConfidence::Low);
    }

    /// Fixture 3: load replaying KNOWN history (an existing Capo item matches by
    /// stable timeline key) adds no duplicate UI item -> duplicate observation.
    #[test]
    fn load_of_known_tool_history_is_duplicate_not_reimport() {
        let frames = vec![update(
            "s1",
            json!({
                "sessionUpdate": "tool_call",
                "toolCallId": "tool-1",
                "title": "write",
                "status": "completed",
                "content": { "type": "text", "text": "done" }
            }),
        )];
        let existing = vec![ExistingItemFingerprint {
            timeline_key: Some("acp:s1:tool:tool-1".to_string()),
            role: None,
            content_hash: "irrelevant-stable-key-wins".to_string(),
        }];
        let plan =
            AcpReplayEngine::plan_load(AcpReplaySource::SessionLoad, "s1", &frames, &existing);
        assert_eq!(plan.candidates.len(), 1);
        assert_eq!(
            plan.candidates[0].decision,
            AcpReconcileDecision::Duplicate,
            "a known tool timeline key reconciles as a duplicate observation"
        );
        assert_eq!(plan.duplicate_count(), 1);
        assert_eq!(plan.imported_count(), 0);
    }

    /// A message whose finalized role+content-hash matches an existing Capo item
    /// is a content-anchor duplicate even without a stable key.
    #[test]
    fn load_of_known_message_matches_by_content_hash() {
        let frames = vec![update(
            "s1",
            json!({
                "sessionUpdate": "agent_message_chunk",
                "content": { "type": "text", "text": "the answer is 42" }
            }),
        )];
        // Derive the same finalized hash the engine produces (role+content).
        let content_hash = stable_hash(b"assistant:the answer is 42");
        let existing = vec![ExistingItemFingerprint {
            timeline_key: None,
            role: Some("assistant".to_string()),
            content_hash,
        }];
        let plan =
            AcpReplayEngine::plan_load(AcpReplaySource::SessionLoad, "s1", &frames, &existing);
        assert_eq!(plan.candidates.len(), 1);
        assert_eq!(plan.candidates[0].decision, AcpReconcileDecision::Duplicate);
        assert_eq!(plan.duplicate_count(), 1);
    }

    /// Fixture 7: a plan update uses the single `acp:{session}:plan:current` key
    /// and is a full replacement (latest plan wins, one candidate).
    #[test]
    fn plan_updates_collapse_to_single_current_plan_candidate() {
        let frames = vec![
            update(
                "s1",
                json!({
                    "sessionUpdate": "plan",
                    "entries": [{ "content": "step 1", "status": "pending" }]
                }),
            ),
            update(
                "s1",
                json!({
                    "sessionUpdate": "plan",
                    "entries": [
                        { "content": "step 1", "status": "completed" },
                        { "content": "step 2", "status": "pending" }
                    ]
                }),
            ),
        ];
        let plan = AcpReplayEngine::plan_load(AcpReplaySource::SessionLoad, "s1", &frames, &[]);
        assert_eq!(plan.raw_update_count(), 2, "both plan states retained raw");
        assert_eq!(plan.candidates.len(), 1, "one current-plan candidate");
        assert_eq!(plan.candidates[0].kind, AcpTimelineKind::Plan);
        assert_eq!(plan.candidates[0].timeline_key, "acp:s1:plan:current");
    }

    /// DP3: every reconciled candidate records an auditable `import_confidence` so a
    /// low-confidence reconciliation is never silently projected. A stable-keyed
    /// tool import is `stable`; an ID-less collapsed-chunk ambiguous import is
    /// `none`.
    #[test]
    fn reconciled_candidates_record_auditable_import_confidence() {
        let stable_frames = vec![update(
            "s1",
            json!({
                "sessionUpdate": "tool_call",
                "toolCallId": "tool-1",
                "title": "write",
                "status": "completed",
                "content": { "type": "text", "text": "done" }
            }),
        )];
        let stable_plan =
            AcpReplayEngine::plan_load(AcpReplaySource::SessionLoad, "s1", &stable_frames, &[]);
        assert_eq!(stable_plan.candidates.len(), 1);
        let stable = &stable_plan.candidates[0];
        assert_eq!(stable.decision, AcpReconcileDecision::Imported);
        assert_eq!(stable.import_confidence, AcpImportConfidence::Stable);
        assert_eq!(stable.import_confidence.as_str(), "stable");

        let ambiguous_frames = vec![
            update(
                "s1",
                json!({
                    "sessionUpdate": "agent_message_chunk",
                    "content": { "type": "text", "text": "hello" }
                }),
            ),
            update(
                "s1",
                json!({
                    "sessionUpdate": "agent_message_chunk",
                    "content": { "type": "text", "text": "hello" }
                }),
            ),
        ];
        let ambiguous_plan =
            AcpReplayEngine::plan_load(AcpReplaySource::SessionLoad, "s1", &ambiguous_frames, &[]);
        assert_eq!(ambiguous_plan.candidates.len(), 1);
        let ambiguous = &ambiguous_plan.candidates[0];
        assert_eq!(ambiguous.decision, AcpReconcileDecision::Ambiguous);
        assert_eq!(ambiguous.import_confidence, AcpImportConfidence::None);
        assert_eq!(ambiguous.import_confidence.as_str(), "none");
    }
}
