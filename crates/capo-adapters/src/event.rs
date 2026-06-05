use serde_json::Value;

pub type AdapterParseResult<T> = Result<T, AdapterParseError>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdapterParseError {
    pub line: usize,
    pub message: String,
}

impl AdapterParseError {
    pub(crate) fn new(line: usize, message: impl Into<String>) -> Self {
        Self {
            line,
            message: message.into(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NormalizedAdapterKind {
    Mock,
    CodexExec,
    ClaudeCode,
    Acp,
}

impl NormalizedAdapterKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Mock => "mock",
            Self::CodexExec => "codex_exec",
            Self::ClaudeCode => "claude_code",
            Self::Acp => "acp",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AdapterTimelineConfidence {
    Stable,
    Heuristic,
    None,
}

/// DP3 (acp-replay-dedupe.md): the `external_ref.adapter = "acp"` provenance block
/// every NORMALIZED ACP event carries once it is reconciled into a batch.
///
/// The pure `parse_acp_record` mapper cannot know the Capo session id, the replay
/// batch id, the raw-update id, or the replay source -- those are only known at the
/// controller ingest seam where the raw frame has already been persisted as an
/// `AcpRawUpdate`. So the provenance block is STAMPED onto the normalized event by
/// the reconciliation engine / controller producer (see
/// [`NormalizedAdapterEvent::stamp_acp_provenance`]), pointing the normalized event
/// back at its raw observation and the batch it was reconciled in. This is exactly
/// the design's "raw updates persisted before normalization; normalized events
/// reference their raw observation" rule made concrete on the event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcpExternalRef {
    /// Always `"acp"`.
    pub adapter: &'static str,
    /// The external (agent-side) session id this event was observed under.
    pub external_session_id: String,
    /// The ACP `session/update` variant kind (`tool_call`, `agent_message_chunk`,
    /// `plan`, ...), carried so the provenance is self-describing without re-parsing
    /// the raw frame.
    pub acp_update_kind: String,
    /// The replay batch this normalized event was reconciled in.
    pub acp_replay_batch_id: String,
    /// The raw observation this normalized event derives from.
    pub acp_raw_update_id: String,
    /// The protocol-aware timeline key when one exists (tool/plan keys, or a
    /// synthetic message anchor).
    pub acp_timeline_key: Option<String>,
    /// Which batch source (`live_prompt` / `session_load` / `session_resume_attach`
    /// / `restart_recovery` / `foreign_import`) the event was observed under.
    pub replay_source: String,
}

/// The terminal classification of a normalized adapter event.
///
/// This is the single source of truth for "this event ends a turn, and how".
/// Both the projection path and the turn-loop outcome derive their terminal
/// signal from [`NormalizedAdapterEvent::terminal_outcome`] so the two never
/// drift on which `adapter.turn_*` kinds are terminal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdapterTerminalOutcome {
    /// `adapter.turn_completed`: the turn finished normally.
    Completed,
    /// `adapter.turn_failed`: the turn failed.
    Failed,
    /// `adapter.turn_interrupted`: the turn was interrupted in flight.
    Interrupted,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NormalizedAdapterEvent {
    pub adapter_kind: NormalizedAdapterKind,
    pub kind: String,
    pub external_session_ref: Option<String>,
    pub external_item_ref: Option<String>,
    pub timeline_key: Option<String>,
    pub timeline_confidence: AdapterTimelineConfidence,
    pub role: Option<String>,
    pub content: Option<String>,
    pub tool_name: Option<String>,
    pub status: Option<String>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub raw_event_hash: String,
    pub idempotency_key: Option<String>,
    pub provider_event_kind: String,
    /// DP3: the `external_ref.adapter = "acp"` provenance block, stamped by the
    /// reconciliation engine / controller producer once the event is tied to a
    /// persisted raw observation + batch. `None` until stamped (the pure mapper
    /// does not know the batch context).
    pub external_ref: Option<AcpExternalRef>,
}

impl NormalizedAdapterEvent {
    pub(crate) fn new(
        adapter_kind: NormalizedAdapterKind,
        kind: impl Into<String>,
        provider_event_kind: impl Into<String>,
        raw: &Value,
    ) -> Self {
        Self {
            adapter_kind,
            kind: kind.into(),
            external_session_ref: None,
            external_item_ref: None,
            timeline_key: None,
            timeline_confidence: AdapterTimelineConfidence::None,
            role: None,
            content: None,
            tool_name: None,
            status: None,
            input_tokens: None,
            output_tokens: None,
            raw_event_hash: json_hash(raw),
            idempotency_key: None,
            provider_event_kind: provider_event_kind.into(),
            external_ref: None,
        }
    }

    /// The DP3 event family for an ACP normalized event, the `{event_family}`
    /// segment of the design's idempotency-key shape
    /// `acp:{capo_session_id}:{event_family}:{timeline_key}:{operation}:{operation_version}`.
    ///
    /// Derived from the kind so the family is the single, design-named bucket the
    /// timeline belongs to (`tool` / `message` / `plan` / `session` / `turn`),
    /// never the raw provider string.
    pub fn acp_event_family(&self) -> &'static str {
        match self.kind.as_str() {
            "adapter.tool_call_requested"
            | "adapter.tool_call_started"
            | "adapter.tool_call_completed"
            | "adapter.tool_call_failed" => "tool",
            "adapter.plan_replaced" => "plan",
            "adapter.session_started" => "session",
            "adapter.turn_completed" | "adapter.turn_failed" | "adapter.turn_interrupted" => "turn",
            _ => "message",
        }
    }

    /// DP3: stamp the `external_ref.adapter = "acp"` provenance block AND rewrite
    /// the idempotency key to the design's canonical
    /// `acp:{capo_session_id}:{event_family}:{timeline_key}:{operation}:{operation_version}`
    /// shape.
    ///
    /// The pure `parse_acp_record` mapper builds a provisional 4-part
    /// `{adapter}:{kind}:{timeline_key}:{operation}` key (it cannot know the Capo
    /// session id or the operation version). The reconciliation engine / controller
    /// producer calls this once it has the batch context, replacing the provisional
    /// key with the canonical 6-part key and attaching the provenance block that
    /// points back at the persisted raw observation. `operation` defaults to the
    /// event's status (or the kind suffix); `operation_version` is the
    /// monotonically-increasing replacement version for replacement-field timelines
    /// (tool/plan), `0` for a first observation.
    #[must_use]
    pub fn stamp_acp_provenance(
        mut self,
        capo_session_id: &str,
        external_ref: AcpExternalRef,
        operation_version: i64,
    ) -> Self {
        let family = self.acp_event_family();
        let timeline_key = self
            .timeline_key
            .clone()
            .or_else(|| external_ref.acp_timeline_key.clone())
            .unwrap_or_else(|| format!("{family}:{}", self.raw_event_hash));
        let operation = self.status.clone().unwrap_or_else(|| family.to_string());
        self.idempotency_key = Some(format!(
            "acp:{capo_session_id}:{family}:{timeline_key}:{operation}:{operation_version}"
        ));
        self.external_ref = Some(external_ref);
        self
    }

    pub(crate) fn with_timeline(
        mut self,
        external_session_ref: Option<String>,
        external_item_ref: Option<String>,
        timeline_key: String,
        confidence: AdapterTimelineConfidence,
        operation: &str,
    ) -> Self {
        self.external_session_ref = external_session_ref;
        self.external_item_ref = external_item_ref;
        self.timeline_key = Some(timeline_key.clone());
        self.timeline_confidence = confidence;
        self.idempotency_key = Some(format!(
            "{}:{}:{}:{}",
            self.adapter_kind.as_str(),
            self.kind,
            timeline_key,
            operation
        ));
        self
    }

    /// `true` for the item/summary kinds the projection records as
    /// `session.summary_updated`. Single source of truth for the summary
    /// taxonomy (shared by the projection path and the turn-loop outcome).
    pub fn is_summary_event(&self) -> bool {
        matches!(
            self.kind.as_str(),
            "adapter.item_completed" | "adapter.item_delta" | "adapter.plan_replaced"
        )
    }

    /// `true` for the tool-call kinds the projection records as `tool.*`.
    /// Single source of truth for the tool taxonomy.
    pub fn is_tool_event(&self) -> bool {
        matches!(
            self.kind.as_str(),
            "adapter.tool_call_requested"
                | "adapter.tool_call_started"
                | "adapter.tool_call_completed"
                | "adapter.tool_call_failed"
        )
    }

    /// The terminal outcome this event carries, if any. `None` for non-terminal
    /// events. Single source of truth for which `adapter.turn_*` kinds end a
    /// turn, shared by the projection path and the turn-loop outcome.
    pub fn terminal_outcome(&self) -> Option<AdapterTerminalOutcome> {
        match self.kind.as_str() {
            "adapter.turn_completed" => Some(AdapterTerminalOutcome::Completed),
            "adapter.turn_failed" => Some(AdapterTerminalOutcome::Failed),
            "adapter.turn_interrupted" => Some(AdapterTerminalOutcome::Interrupted),
            _ => None,
        }
    }

    pub fn tool_observation(&self) -> Option<AdapterToolObservation> {
        if !self.is_tool_event() {
            return None;
        }
        Some(AdapterToolObservation {
            source_adapter: self.adapter_kind.as_str().to_string(),
            external_tool_ref: self
                .external_item_ref
                .clone()
                .or_else(|| self.timeline_key.clone()),
            tool_name: self
                .tool_name
                .clone()
                .unwrap_or_else(|| "adapter-native-tool".to_string()),
            observed_status: self
                .status
                .clone()
                .unwrap_or_else(|| "observed".to_string()),
            instrumentation_level: "observed_only".to_string(),
            confidence: match self.timeline_confidence {
                AdapterTimelineConfidence::Stable => "high",
                AdapterTimelineConfidence::Heuristic => "medium",
                AdapterTimelineConfidence::None => "low",
            }
            .to_string(),
            raw_event_hash: self.raw_event_hash.clone(),
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdapterFixtureParse {
    pub raw_event_count: usize,
    pub events: Vec<NormalizedAdapterEvent>,
}

impl AdapterFixtureParse {
    pub fn deduped_by_idempotency(&self) -> Vec<NormalizedAdapterEvent> {
        let mut seen = std::collections::HashSet::new();
        let mut deduped = Vec::new();
        for event in &self.events {
            match &event.idempotency_key {
                Some(key) if seen.insert(key.clone()) => deduped.push(event.clone()),
                Some(_) => {}
                None => deduped.push(event.clone()),
            }
        }
        deduped
    }

    pub fn tool_observations(&self) -> Vec<AdapterToolObservation> {
        self.deduped_by_idempotency()
            .into_iter()
            .filter_map(|event| event.tool_observation())
            .collect()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdapterToolObservation {
    pub source_adapter: String,
    pub external_tool_ref: Option<String>,
    pub tool_name: String,
    pub observed_status: String,
    pub instrumentation_level: String,
    pub confidence: String,
    pub raw_event_hash: String,
}

pub(crate) fn parse_jsonl(
    input: &str,
    parser: fn(&Value) -> Vec<NormalizedAdapterEvent>,
) -> AdapterParseResult<AdapterFixtureParse> {
    let mut raw_event_count = 0;
    let mut events = Vec::new();
    for (index, line) in input.lines().enumerate() {
        let line_number = index + 1;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        raw_event_count += 1;
        let value: Value = serde_json::from_str(line)
            .map_err(|error| AdapterParseError::new(line_number, error.to_string()))?;
        events.extend(parser(&value));
    }
    Ok(AdapterFixtureParse {
        raw_event_count,
        events,
    })
}

pub(crate) fn string_at(value: &Value, path: &[&str]) -> Option<String> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment)?;
    }
    match current {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(boolean) => Some(boolean.to_string()),
        _ => None,
    }
}

pub(crate) fn integer_at(value: &Value, path: &[&str]) -> Option<i64> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment)?;
    }
    current.as_i64()
}

pub(crate) fn text_from_content_array(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => Some(text.clone()),
        Value::Array(items) => {
            let text = items
                .iter()
                .filter_map(|item| {
                    // A direct text block `{ type:"text", text }`.
                    string_at(item, &["text"])
                        // The real claude-code-acp 0.16.2 tool_call shape wraps the
                        // payload: `{ type:"content", content:{ type:"text", text } }`
                        // (content is an OBJECT, so a flat `string_at(["content"])`
                        // returns None and the block was previously dropped). Recurse
                        // into the wrapper — this also handles a scalar `content`.
                        .or_else(|| text_from_content_array(item.get("content")))
                        // A `{ type:"diff", path?, oldText?, newText }` block: surface
                        // the resulting content so observed edits aren't lost.
                        .or_else(|| string_at(item, &["newText"]))
                        .or_else(|| string_at(item, &["input"]))
                })
                .collect::<Vec<_>>()
                .join("");
            if text.is_empty() { None } else { Some(text) }
        }
        Value::Object(_) => {
            string_at(value?, &["text"]).or_else(|| string_at(value?, &["content"]))
        }
        _ => None,
    }
}

pub(crate) fn json_hash(value: &Value) -> String {
    stable_hash(value.to_string().as_bytes())
}

pub(crate) fn stable_hash(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("fnv1a64:{hash:016x}")
}

#[cfg(test)]
mod content_array_tests {
    use super::text_from_content_array;
    use serde_json::json;

    #[test]
    fn extracts_text_from_the_real_0_16_2_tool_call_array_shape() {
        // claude-code-acp 0.16.2 emits tool_call `content` as an ARRAY of wrapped
        // blocks: a `content` wrapper whose `content` is an object, plus a `diff`
        // block. Both must be surfaced (regression guard for M5).
        let v = json!([
            {"type": "content", "content": {"type": "text", "text": "wrote HELLO.txt"}},
            {"type": "diff", "path": "HELLO.txt", "oldText": "", "newText": "capo-works"}
        ]);
        let got = text_from_content_array(Some(&v)).expect("must extract content");
        assert!(got.contains("wrote HELLO.txt"), "content block text lost: {got:?}");
        assert!(got.contains("capo-works"), "diff newText lost: {got:?}");
    }

    #[test]
    fn still_handles_legacy_string_and_object_and_flat_text_shapes() {
        assert_eq!(
            text_from_content_array(Some(&json!("hi"))).as_deref(),
            Some("hi")
        );
        assert_eq!(
            text_from_content_array(Some(&json!({"type": "text", "text": "obj"}))).as_deref(),
            Some("obj")
        );
        assert_eq!(
            text_from_content_array(Some(&json!([{"type": "text", "text": "flat"}]))).as_deref(),
            Some("flat")
        );
        assert_eq!(text_from_content_array(None), None);
        assert_eq!(text_from_content_array(Some(&json!([]))), None);
    }
}
