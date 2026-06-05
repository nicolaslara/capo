use serde_json::Value;

use crate::event::{
    integer_at, json_hash, parse_jsonl, stable_hash, string_at, text_from_content_array,
};
use crate::{
    AdapterFixtureParse, AdapterParseResult, AdapterTimelineConfidence, NormalizedAdapterEvent,
    NormalizedAdapterKind,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodexExecAdapter;

impl CodexExecAdapter {
    pub fn parse_jsonl(input: &str) -> AdapterParseResult<AdapterFixtureParse> {
        parse_jsonl(input, parse_codex_record)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaudeCodeAdapter;

impl ClaudeCodeAdapter {
    pub fn parse_stream_json(input: &str) -> AdapterParseResult<AdapterFixtureParse> {
        parse_jsonl(input, parse_claude_record)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcpAdapter;

impl AcpAdapter {
    pub fn parse_replay_jsonl(input: &str) -> AdapterParseResult<AdapterFixtureParse> {
        parse_jsonl(input, parse_acp_record)
    }

    /// Normalize a SINGLE raw ACP JSON-RPC value (a `session/update`
    /// notification frame, or a `session/new`/`initialize` response) into the
    /// loop's [`NormalizedAdapterEvent`]s.
    ///
    /// The live wire client (DP1) ingests `session/update` notifications as they
    /// arrive off the wire and reuses THIS shared mapper -- the exact same
    /// `parse_acp_record` path the deterministic replay fixtures exercise -- so
    /// the live adapter never opens a parallel ingestion route.
    pub fn normalize_update(raw: &Value) -> Vec<NormalizedAdapterEvent> {
        parse_acp_record(raw)
    }
}

fn parse_codex_record(raw: &Value) -> Vec<NormalizedAdapterEvent> {
    let provider_kind = string_at(raw, &["type"]).unwrap_or_else(|| "unknown".to_string());
    let session_ref = string_at(raw, &["thread_id"]).or_else(|| string_at(raw, &["session_id"]));
    let mut event = NormalizedAdapterEvent::new(
        NormalizedAdapterKind::CodexExec,
        "adapter.raw_event",
        &provider_kind,
        raw,
    );

    match provider_kind.as_str() {
        "thread.started" => {
            event.kind = "adapter.session_started".to_string();
            event.external_session_ref = session_ref.clone();
            event.timeline_confidence = AdapterTimelineConfidence::Stable;
            event.timeline_key = session_ref.map(|session| format!("codex:{session}:session"));
        }
        "item.started" | "item.completed" | "item.updated" => {
            let item_ref = string_at(raw, &["item", "id"]).or_else(|| string_at(raw, &["id"]));
            let item_type = string_at(raw, &["item", "type"]);
            // The live `codex exec --json` workspace-write stream reports an
            // applied edit as an `item.completed` whose `item.type` is
            // `file_change` (not a `patch_apply.*` tool event), carrying the
            // applied `changes`. Route that to the SAME observed tool-result event
            // (`apply_patch`) the `patch_apply.*` family produces, so the live
            // round-trip records a `tool.observation_recorded` distinct from the
            // agent's `agent_message`/`message` claim -- the RTL9/RTL13 contract.
            // A read-only `command_execution` item maps to the `exec_command`
            // observed result for the same reason.
            if matches!(
                item_type.as_deref(),
                Some("file_change") | Some("command_execution")
            ) {
                let operation = if provider_kind == "item.started" {
                    "started"
                } else {
                    "completed"
                };
                let timeline_key = item_ref
                    .clone()
                    .map(|item| format!("codex:tool:{item}"))
                    .unwrap_or_else(|| format!("codex:tool:{}", json_hash(raw)));
                event = event.with_timeline(
                    session_ref,
                    item_ref,
                    timeline_key,
                    AdapterTimelineConfidence::Stable,
                    operation,
                );
                event.kind = if operation == "started" {
                    "adapter.tool_call_started".to_string()
                } else {
                    "adapter.tool_call_completed".to_string()
                };
                event.tool_name = Some(if item_type.as_deref() == Some("file_change") {
                    "apply_patch".to_string()
                } else {
                    "exec_command".to_string()
                });
                event.status = Some(operation.to_string());
                // The OBSERVED applied result: the file `changes` array (or an
                // `aggregated_output`/`output` for an exec), reduced to one string.
                event.content = codex_item_tool_result_content(raw);
                return vec![event];
            }
            let role = string_at(raw, &["item", "role"]).or_else(|| {
                (item_type.as_deref() == Some("agent_message")).then(|| "assistant".to_string())
            });
            let content = text_from_content_array(raw.pointer("/item/content")).or_else(|| {
                (item_type.as_deref() == Some("agent_message"))
                    .then(|| string_at(raw, &["item", "text"]))
                    .flatten()
            });
            let timeline_key = item_ref
                .clone()
                .map(|item| format!("codex:item:{item}"))
                .unwrap_or_else(|| format!("codex:item:{}", json_hash(raw)));
            event = event.with_timeline(
                session_ref,
                item_ref,
                timeline_key,
                AdapterTimelineConfidence::Stable,
                "upsert",
            );
            event.kind = "adapter.item_completed".to_string();
            event.role = role;
            event.content = content;
            event.status = Some("completed".to_string());
        }
        "exec_command.begin"
        | "exec_command.end"
        | "patch_apply.begin"
        | "patch_apply.end"
        | "tool_call.started"
        | "tool_call.completed" => {
            let call_ref = string_at(raw, &["call_id"])
                .or_else(|| string_at(raw, &["tool_call_id"]))
                .or_else(|| string_at(raw, &["id"]));
            let timeline_key = call_ref
                .clone()
                .map(|call| format!("codex:tool:{call}"))
                .unwrap_or_else(|| format!("codex:tool:{}", json_hash(raw)));
            let operation =
                if provider_kind.ends_with(".begin") || provider_kind.ends_with(".started") {
                    "started"
                } else {
                    "completed"
                };
            event = event.with_timeline(
                session_ref,
                call_ref,
                timeline_key,
                AdapterTimelineConfidence::Stable,
                operation,
            );
            event.kind = if operation == "started" {
                "adapter.tool_call_started".to_string()
            } else {
                "adapter.tool_call_completed".to_string()
            };
            // Workspace-write tool calls (e.g. `patch_apply.*`) name themselves
            // `apply_patch`; the read-only `exec_command.*` family defaults to
            // `exec_command`. The OBSERVED tool result -- the changes/diff Codex
            // applied -- is captured into `content` so the projection's
            // `tool.observation_recorded` carries the observed write result,
            // distinct from any agent-reported `item.completed` message claim.
            event.tool_name = string_at(raw, &["tool_name"])
                .or_else(|| string_at(raw, &["name"]))
                .or_else(|| {
                    provider_kind
                        .starts_with("patch_apply")
                        .then(|| "apply_patch".to_string())
                })
                .or_else(|| Some("exec_command".to_string()));
            event.status = Some(operation.to_string());
            event.content = codex_tool_result_content(raw);
        }
        "turn.completed" | "thread.completed" => {
            event.kind = "adapter.turn_completed".to_string();
            event.external_session_ref = session_ref;
            event.status = Some("completed".to_string());
            event.input_tokens = integer_at(raw, &["usage", "input_tokens"]);
            event.output_tokens = integer_at(raw, &["usage", "output_tokens"]);
        }
        _ => {}
    }

    vec![event]
}

/// Extract the OBSERVED result of a Codex tool call into a single string.
///
/// Workspace-write round-trips report the applied changes under a handful of
/// shapes (`changes`/`unified_diff`/`output`/`aggregated_output`/
/// `formatted_output`); this reduces them to one observed-result string so the
/// projection records a `tool.observation_recorded` carrying what Codex actually
/// did, separate from the agent's own `item.completed` message text.
fn codex_tool_result_content(raw: &Value) -> Option<String> {
    string_at(raw, &["aggregated_output"])
        .or_else(|| string_at(raw, &["formatted_output"]))
        .or_else(|| string_at(raw, &["output"]))
        .or_else(|| string_at(raw, &["unified_diff"]))
        .or_else(|| {
            raw.get("changes")
                .filter(|changes| !changes.is_null())
                .map(Value::to_string)
        })
}

/// Extract the OBSERVED result of a Codex `item.*` tool item (`file_change` /
/// `command_execution`) into a single string.
///
/// The live `codex exec --json` stream nests the applied changes under `item`
/// (e.g. `item.changes` for a `file_change`, `item.aggregated_output` for a
/// `command_execution`), so the result lives one level deeper than the
/// `patch_apply.*` shapes [`codex_tool_result_content`] handles.
fn codex_item_tool_result_content(raw: &Value) -> Option<String> {
    // Prefer a NON-EMPTY captured output/diff/changes; a `command_execution`
    // often reports an empty `aggregated_output` while still carrying the
    // `command` it ran, which is the meaningful observed result in that case.
    non_empty(string_at(raw, &["item", "aggregated_output"]))
        .or_else(|| non_empty(string_at(raw, &["item", "formatted_output"])))
        .or_else(|| non_empty(string_at(raw, &["item", "output"])))
        .or_else(|| non_empty(string_at(raw, &["item", "unified_diff"])))
        .or_else(|| {
            raw.pointer("/item/changes")
                .filter(|changes| !changes.is_null())
                .map(Value::to_string)
        })
        .or_else(|| non_empty(string_at(raw, &["item", "command"])))
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|text| !text.is_empty())
}

fn parse_claude_record(raw: &Value) -> Vec<NormalizedAdapterEvent> {
    let provider_kind = string_at(raw, &["type"]).unwrap_or_else(|| "unknown".to_string());
    let session_ref =
        string_at(raw, &["session_id"]).or_else(|| string_at(raw, &["message", "session_id"]));
    let mut event = NormalizedAdapterEvent::new(
        NormalizedAdapterKind::ClaudeCode,
        "adapter.raw_event",
        &provider_kind,
        raw,
    );

    match provider_kind.as_str() {
        "system" => {
            event.kind = "adapter.session_started".to_string();
            event.external_session_ref = session_ref.clone();
            event.timeline_confidence = AdapterTimelineConfidence::Stable;
            event.timeline_key = session_ref.map(|session| format!("claude:{session}:session"));
        }
        "assistant" | "user" => {
            let item_ref = string_at(raw, &["message", "id"]).or_else(|| string_at(raw, &["id"]));
            let role = string_at(raw, &["message", "role"]).or_else(|| Some(provider_kind.clone()));
            let content = text_from_content_array(raw.pointer("/message/content"));
            let timeline_key = item_ref
                .clone()
                .map(|item| format!("claude:item:{item}"))
                .unwrap_or_else(|| format!("claude:item:{}", json_hash(raw)));
            event = event.with_timeline(
                session_ref,
                item_ref,
                timeline_key,
                AdapterTimelineConfidence::Stable,
                "upsert",
            );
            event.kind = "adapter.item_completed".to_string();
            event.role = role;
            event.content = content;
            event.status = Some("completed".to_string());
            event.input_tokens = integer_at(raw, &["message", "usage", "input_tokens"]);
            event.output_tokens = integer_at(raw, &["message", "usage", "output_tokens"]);
        }
        "tool_use" | "tool_result" => {
            let call_ref = string_at(raw, &["id"]).or_else(|| string_at(raw, &["tool_use_id"]));
            let timeline_key = call_ref
                .clone()
                .map(|call| format!("claude:tool:{call}"))
                .unwrap_or_else(|| format!("claude:tool:{}", json_hash(raw)));
            let operation = if provider_kind == "tool_use" {
                "started"
            } else {
                "completed"
            };
            event = event.with_timeline(
                session_ref,
                call_ref,
                timeline_key,
                AdapterTimelineConfidence::Stable,
                operation,
            );
            event.kind = if operation == "started" {
                "adapter.tool_call_started".to_string()
            } else {
                "adapter.tool_call_completed".to_string()
            };
            event.tool_name = string_at(raw, &["name"]);
            event.status = Some(operation.to_string());
            // OBSERVED-ONLY (CS4): a `tool_result` carries the result Claude itself
            // observed from the tool, NOT a Capo-authored result. The parser only
            // OBSERVES it into `content` (the projection records a
            // `tool.observation_recorded` with `instrumentation_level =
            // "observed_only"` via `tool_observation()`); Capo injects nothing back
            // over the one-shot. Mirrors the Codex `apply_patch`/`exec_command`
            // observed tool-result shape.
            event.content = claude_tool_result_content(raw);
        }
        "result" => {
            event.kind = "adapter.turn_completed".to_string();
            event.external_session_ref = session_ref;
            event.status = string_at(raw, &["subtype"]).or_else(|| Some("completed".to_string()));
            event.input_tokens = integer_at(raw, &["usage", "input_tokens"]);
            event.output_tokens = integer_at(raw, &["usage", "output_tokens"]);
        }
        _ => {}
    }

    vec![event]
}

/// Extract the OBSERVED result of a Claude `tool_result` record into a single
/// string.
///
/// Claude `stream-json` reports a tool result's `content` either as a plain
/// string or as a content-block array (`[{"type":"text","text":...}]`); both
/// shapes are reduced to one observed-result string so the projection records a
/// `tool.observation_recorded` carrying what the tool actually returned,
/// distinct from the agent's own `assistant` message text. This is the Claude
/// analogue of [`codex_tool_result_content`]. Observed-only: Capo never authors
/// or injects a tool result back.
fn claude_tool_result_content(raw: &Value) -> Option<String> {
    text_from_content_array(raw.get("content")).or_else(|| string_at(raw, &["content"]))
}

fn parse_acp_record(raw: &Value) -> Vec<NormalizedAdapterEvent> {
    let provider_kind = string_at(raw, &["method"])
        .or_else(|| string_at(raw, &["result", "kind"]))
        .unwrap_or_else(|| "unknown".to_string());
    let session_ref = string_at(raw, &["params", "sessionId"])
        .or_else(|| string_at(raw, &["params", "session_id"]))
        .or_else(|| string_at(raw, &["result", "sessionId"]));
    let update = raw.pointer("/params/update").unwrap_or(&Value::Null);
    let update_kind = string_at(update, &["sessionUpdate"])
        .or_else(|| string_at(update, &["kind"]))
        .unwrap_or_else(|| provider_kind.clone());
    let mut event = NormalizedAdapterEvent::new(
        NormalizedAdapterKind::Acp,
        "adapter.raw_event",
        &update_kind,
        raw,
    );

    match update_kind.as_str() {
        "session_started" | "session_info" | "session_info_update" => {
            event.kind = "adapter.session_started".to_string();
            event.external_session_ref = session_ref.clone();
            event.timeline_confidence = AdapterTimelineConfidence::Stable;
            event.timeline_key = session_ref.map(|session| format!("acp:{session}:session_info"));
        }
        "agent_message_chunk" | "user_message_chunk" | "agent_thought_chunk" => {
            let role = if update_kind == "user_message_chunk" {
                "user"
            } else {
                "assistant"
            };
            // 0.16.2 sends chunk `content` as a single object
            // `{type:"text",text:...}` (older fixtures send a flat string or
            // `{content:{text}}`); `text_from_content_array` handles String,
            // Object, AND Array, so it covers every shape. Fall back to the legacy
            // `content.text` / `content` string lookups for the /bin/sh stub.
            let content = text_from_content_array(update.get("content"))
                .or_else(|| string_at(update, &["content", "text"]))
                .or_else(|| string_at(update, &["content"]))
                .unwrap_or_default();
            let synthetic = format!("{}:{}", role, stable_hash(content.as_bytes()));
            let timeline_key = match &session_ref {
                Some(session) => format!("acp:{session}:message:{synthetic}"),
                None => format!("acp:unknown:message:{synthetic}"),
            };
            event = event.with_timeline(
                session_ref,
                None,
                timeline_key,
                AdapterTimelineConfidence::Heuristic,
                "delta",
            );
            event.kind = "adapter.item_delta".to_string();
            event.role = Some(role.to_string());
            event.content = Some(content);
            event.status = Some("streaming".to_string());
        }
        "tool_call" | "tool_call_update" => {
            let call_ref =
                string_at(update, &["toolCallId"]).or_else(|| string_at(update, &["tool_call_id"]));
            let timeline_key = match (&session_ref, &call_ref) {
                (Some(session), Some(call)) => format!("acp:{session}:tool:{call}"),
                (_, Some(call)) => format!("acp:unknown:tool:{call}"),
                _ => format!("acp:unknown:tool:{}", json_hash(raw)),
            };
            let operation = string_at(update, &["status"]).unwrap_or_else(|| {
                if update_kind == "tool_call" {
                    "requested".to_string()
                } else {
                    "updated".to_string()
                }
            });
            event = event.with_timeline(
                session_ref,
                call_ref,
                timeline_key,
                AdapterTimelineConfidence::Stable,
                &operation,
            );
            event.kind = match operation.as_str() {
                "completed" => "adapter.tool_call_completed",
                "failed" => "adapter.tool_call_failed",
                "in_progress" => "adapter.tool_call_started",
                _ => "adapter.tool_call_requested",
            }
            .to_string();
            event.tool_name = string_at(update, &["title"])
                .or_else(|| string_at(update, &["name"]))
                .or_else(|| string_at(update, &["toolKind"]));
            event.status = Some(operation);
            // 0.16.2 sends tool_call `content` as an ARRAY of blocks
            // (`[{type:"content",content:{...}},{type:"diff",...}]`) and
            // `rawOutput` as `[{type:"text",text:...}]`; `text_from_content_array`
            // reaches into both, with the legacy `content.text` string lookup kept
            // for the /bin/sh stub.
            event.content = text_from_content_array(update.get("content"))
                .or_else(|| string_at(update, &["content", "text"]))
                .or_else(|| text_from_content_array(update.get("rawOutput")))
                .or_else(|| string_at(update, &["rawOutput"]));
        }
        "plan" | "plan_update" => {
            let timeline_key = match &session_ref {
                Some(session) => format!("acp:{session}:plan:current"),
                None => "acp:unknown:plan:current".to_string(),
            };
            event = event.with_timeline(
                session_ref,
                None,
                timeline_key,
                AdapterTimelineConfidence::Stable,
                "replace",
            );
            event.kind = "adapter.plan_replaced".to_string();
            event.content = raw.pointer("/params/update/entries").map(Value::to_string);
            event.status = Some("current".to_string());
        }
        _ => {}
    }

    vec![event]
}
