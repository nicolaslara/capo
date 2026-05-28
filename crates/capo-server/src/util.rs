use std::collections::BTreeSet;

use capo_adapters::{
    AcpAdapter, AdapterFixtureParse, AdapterParseError, ClaudeCodeAdapter, CodexExecAdapter,
    NormalizedAdapterEvent,
};
use capo_state::EventRecord;

use crate::{ServerError, ServerResult};

pub(crate) fn parse_adapter_events(
    adapter: &str,
    fixture_jsonl: &str,
) -> Result<Vec<NormalizedAdapterEvent>, String> {
    let parsed: AdapterFixtureParse =
        match adapter_label(adapter).map_err(|error| format!("{error:?}"))? {
            "codex_exec" => {
                CodexExecAdapter::parse_jsonl(fixture_jsonl).map_err(adapter_parse_error)?
            }
            "claude_code" => {
                ClaudeCodeAdapter::parse_stream_json(fixture_jsonl).map_err(adapter_parse_error)?
            }
            "acp" => AcpAdapter::parse_replay_jsonl(fixture_jsonl).map_err(adapter_parse_error)?,
            _ => unreachable!("adapter_label constrains variants"),
        };
    Ok(parsed.deduped_by_idempotency())
}

fn adapter_parse_error(error: AdapterParseError) -> String {
    format!(
        "adapter fixture parse failed at line {}: {}",
        error.line, error.message
    )
}

pub(crate) fn adapter_label(adapter: &str) -> ServerResult<&'static str> {
    match adapter {
        "codex" | "codex-exec" | "codex_exec" => Ok("codex_exec"),
        "claude" | "claude-code" | "claude_code" => Ok("claude_code"),
        "acp" => Ok("acp"),
        other => Err(ServerError::AdapterFixture(format!(
            "unsupported adapter fixture kind: {other}; expected codex, claude, or acp"
        ))),
    }
}

pub(crate) fn provider_kind_for_adapter(adapter: &str) -> &'static str {
    match adapter {
        "codex_exec" => "openai_codex_cli",
        "claude_code" => "anthropic_claude_code_cli",
        "acp" => "agent_client_protocol",
        _ => "unknown",
    }
}

pub(crate) fn command_identity_hash(material: String) -> String {
    stable_hash(material.as_bytes())
}

pub(crate) fn adapter_kind_for_events(events: &[EventRecord]) -> Option<String> {
    events.iter().find_map(|event| {
        if event.kind != "session.started" && event.kind != "server.request_handled" {
            return None;
        }
        serde_json::from_str::<serde_json::Value>(&event.payload_json)
            .ok()
            .and_then(|value| {
                value
                    .get("adapter_kind")
                    .or_else(|| value.get("adapter"))
                    .and_then(serde_json::Value::as_str)
                    .map(ToString::to_string)
            })
    })
}

pub(crate) fn turn_ids_for_events(events: &[EventRecord]) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| event.turn_id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub(crate) fn slug(value: &str) -> String {
    let mut slug = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    slug.trim_matches('-').to_string()
}

pub(crate) fn stable_hash(value: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in value {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}
