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
        }
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
                    string_at(item, &["text"])
                        .or_else(|| string_at(item, &["content"]))
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
