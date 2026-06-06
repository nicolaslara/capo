use super::*;

impl FakeBoundaryController {
    pub fn apply_normalized_adapter_events(
        &self,
        refs: &FakeRunRefs,
        adapter_events: &[NormalizedAdapterEvent],
    ) -> StateResult<AdapterReplayReport> {
        self.apply_normalized_adapter_events_with_turn(refs, adapter_events, None)
    }

    pub fn apply_normalized_adapter_events_with_turn(
        &self,
        refs: &FakeRunRefs,
        adapter_events: &[NormalizedAdapterEvent],
        turn_id_override: Option<&str>,
    ) -> StateResult<AdapterReplayReport> {
        let session = self
            .state
            .session(&refs.session_id)?
            .ok_or_else(|| missing_read_model("session", &refs.session_id))?;
        let task = self
            .state
            .task(&refs.task_id)?
            .ok_or_else(|| missing_read_model("task", &refs.task_id))?;
        let mut appended_event_count = 0;
        let mut tool_event_count = 0;
        let mut summary_event_count = 0;
        let mut completed_turn_count = 0;

        for (index, adapter_event) in adapter_events.iter().enumerate() {
            let event_identity = adapter_event_identity(adapter_event, index);
            let Some((event_kind, projection)) = self.adapter_event_projection(
                refs,
                adapter_event,
                &session,
                &task,
                turn_id_override,
            )?
            else {
                continue;
            };
            let mut event = scoped_event(
                &format!(
                    "event-adapter-replay-{}-{}-{}",
                    adapter_event.adapter_kind.as_str(),
                    refs.session_id,
                    event_identity
                ),
                event_kind,
                &self.project_id,
                &refs.task_id,
                &refs.agent_id,
                &refs.session_id,
                &refs.run_id,
            );
            event.turn_id = adapter_replay_turn_id(adapter_event, turn_id_override);
            event.item_id = adapter_event.external_item_ref.clone();
            event.payload_json = adapter_event_payload_json(adapter_event);
            event.idempotency_key = adapter_event.idempotency_key.clone().or_else(|| {
                Some(adapter_replay_fallback_key(
                    &refs.session_id,
                    turn_id_override,
                    index,
                    None,
                ))
            });
            event.redaction_state = RedactionState::Safe;
            let before = self.state.event_count()?;
            self.state.append_event(event, &[projection])?;
            if self.state.event_count()? > before {
                appended_event_count += 1;
                // Classify the appended event through the SAME taxonomy the
                // turn-loop outcome uses (NormalizedAdapterEvent::is_*), so the
                // two classifiers cannot drift.
                if adapter_event.is_tool_event() {
                    tool_event_count += 1;
                } else if adapter_event.is_summary_event() {
                    summary_event_count += 1;
                } else if adapter_event.terminal_outcome()
                    == Some(capo_adapters::AdapterTerminalOutcome::Completed)
                {
                    completed_turn_count += 1;
                }
            }
            if let Some(observation_projection) =
                self.adapter_tool_observation_projection(refs, adapter_event)?
            {
                let mut observation_event = scoped_event(
                    &format!(
                        "event-adapter-tool-observation-{}-{}-{}",
                        adapter_event.adapter_kind.as_str(),
                        refs.session_id,
                        event_identity
                    ),
                    EventKind::ToolObservationRecorded,
                    &self.project_id,
                    &refs.task_id,
                    &refs.agent_id,
                    &refs.session_id,
                    &refs.run_id,
                );
                observation_event.turn_id = adapter_replay_turn_id(adapter_event, turn_id_override);
                observation_event.item_id = adapter_event.external_item_ref.clone();
                observation_event.payload_json = adapter_event_payload_json(adapter_event);
                observation_event.idempotency_key = adapter_event
                    .idempotency_key
                    .as_ref()
                    .map(|key| format!("{key}:tool-observation"))
                    .or_else(|| {
                        Some(adapter_replay_fallback_key(
                            &refs.session_id,
                            turn_id_override,
                            index,
                            Some("tool-observation"),
                        ))
                    });
                observation_event.redaction_state = RedactionState::Safe;
                let before = self.state.event_count()?;
                self.state
                    .append_event(observation_event, &[observation_projection])?;
                if self.state.event_count()? > before {
                    appended_event_count += 1;
                }
            }
        }

        Ok(AdapterReplayReport {
            input_event_count: adapter_events.len(),
            appended_event_count,
            tool_event_count,
            summary_event_count,
            completed_turn_count,
        })
    }

    pub fn apply_scripted_mock_turn(
        &self,
        refs: &FakeRunRefs,
        turn: &ScriptedMockTurn,
    ) -> StateResult<AdapterReplayReport> {
        let adapter = AgentAdapterHandle::scripted_mock(
            ScriptedMockAgent::new(refs.external_session_ref.clone()).with_turn(turn.clone()),
        );
        let events = adapter
            .scripted_turn_events(turn.turn_ref())
            .ok_or_else(|| missing_read_model("scripted_mock_turn", &turn.turn_ref()))?;
        self.apply_normalized_adapter_events(refs, &events)
    }

    pub fn apply_scripted_acp_mock_turn(
        &self,
        refs: &FakeRunRefs,
        turn: &ScriptedMockTurn,
    ) -> StateResult<AdapterReplayReport> {
        let adapter = AgentAdapterHandle::scripted_mock(
            ScriptedMockAgent::acp_shaped(refs.external_session_ref.clone())
                .with_turn(turn.clone()),
        );
        let events = adapter
            .scripted_turn_events(turn.turn_ref())
            .ok_or_else(|| missing_read_model("scripted_acp_mock_turn", &turn.turn_ref()))?;
        self.apply_normalized_adapter_events(refs, &events)
    }

    fn adapter_event_projection(
        &self,
        refs: &FakeRunRefs,
        adapter_event: &NormalizedAdapterEvent,
        session: &SessionProjection,
        task: &TaskProjection,
        turn_id_override: Option<&str>,
    ) -> StateResult<Option<(EventKind, ProjectionRecord)>> {
        match adapter_event.kind.as_str() {
            "adapter.item_completed" | "adapter.item_delta" | "adapter.plan_replaced" => {
                let content_hash = adapter_event
                    .content
                    .as_ref()
                    .map(|content| stable_hash(content.as_bytes()))
                    .unwrap_or_else(|| adapter_event.raw_event_hash.clone());
                // SLICE-A LEGIBILITY: for ASSISTANT prose, the summary IS the
                // conductor's real words (capped) rather than a hash label, so the
                // dashboard/thread readback is legible. Non-assistant items keep
                // the hash-label form.
                let is_assistant_prose = adapter_event.role.as_deref() == Some("assistant");
                let latest_summary = match (is_assistant_prose, adapter_event.content.as_deref()) {
                    (true, Some(content)) if !content.is_empty() => {
                        Some(content.chars().take(ADAPTER_PROSE_INLINE_CAP).collect())
                    }
                    _ => Some(format!(
                        "Adapter {} {} observed content_hash={content_hash}",
                        adapter_event.adapter_kind.as_str(),
                        adapter_event.role.as_deref().unwrap_or("event")
                    )),
                };
                Ok(Some((
                    EventKind::SessionSummaryUpdated,
                    ProjectionRecord::Session(SessionProjection {
                        session_id: refs.session_id.clone(),
                        project_id: self.project_id.clone(),
                        task_id: Some(refs.task_id.clone()),
                        agent_id: refs.agent_id.clone(),
                        title: session.title.clone(),
                        status: "active".to_string(),
                        current_goal: session.current_goal.clone(),
                        latest_summary,
                        latest_confidence: Some(match adapter_event.timeline_confidence {
                            capo_adapters::AdapterTimelineConfidence::Stable => 82,
                            capo_adapters::AdapterTimelineConfidence::Heuristic => 60,
                            capo_adapters::AdapterTimelineConfidence::None => 40,
                        }),
                        latest_blocker: None,
                        external_session_ref: session.external_session_ref.clone(),
                        updated_sequence: 0,
                    }),
                )))
            }
            "adapter.tool_call_requested"
            | "adapter.tool_call_started"
            | "adapter.tool_call_completed"
            | "adapter.tool_call_failed" => {
                let tool_call_id = adapter_tool_call_id(adapter_event);
                let existing_tool_name = self
                    .state
                    .tool_calls_for_session(&refs.session_id)?
                    .into_iter()
                    .find(|tool| tool.tool_call_id == tool_call_id)
                    .map(|tool| tool.tool_name);
                let status = match adapter_event.kind.as_str() {
                    "adapter.tool_call_requested" => "requested",
                    "adapter.tool_call_started" => "started",
                    "adapter.tool_call_completed" => "completed",
                    "adapter.tool_call_failed" => "failed",
                    _ => unreachable!("matched above"),
                };
                Ok(Some((
                    match adapter_event.kind.as_str() {
                        "adapter.tool_call_requested" => EventKind::ToolCallRequested,
                        "adapter.tool_call_started" => EventKind::ToolInvocationStarted,
                        "adapter.tool_call_completed" | "adapter.tool_call_failed" => {
                            EventKind::ToolCallCompleted
                        }
                        _ => unreachable!("matched above"),
                    },
                    ProjectionRecord::ToolCall(capo_state::ToolCallProjection {
                        tool_call_id,
                        session_id: refs.session_id.clone(),
                        turn_id: adapter_replay_turn_id(adapter_event, turn_id_override),
                        tool_name: adapter_event
                            .tool_name
                            .clone()
                            .or(existing_tool_name)
                            .unwrap_or_else(|| "adapter-native-tool".to_string()),
                        tool_origin: format!(
                            "adapter_native:{}",
                            adapter_event.adapter_kind.as_str()
                        ),
                        status: status.to_string(),
                        input_artifact_id: None,
                        output_artifact_id: adapter_event.content.as_ref().map(|_| {
                            format!("artifact-adapter-output-{}", adapter_event.raw_event_hash)
                        }),
                        // ACI7: adapter-native tool provenance/timing is the
                        // adapter dedup's concern (ACI9); the locally-dispatched
                        // path is what carries the queryable permission/grant/timing
                        // chain, so this defaults cleanly.
                        provenance: capo_state::ToolCallProvenance::default(),
                        updated_sequence: 0,
                    }),
                )))
            }
            "adapter.turn_completed" => Ok(Some((
                EventKind::EvidenceRecorded,
                ProjectionRecord::Evidence(capo_state::EvidenceProjection {
                    // GA6 (GO13): the evidence row id is keyed PER TURN, not just by
                    // `(adapter_kind, session_id)`. A session takes many provider
                    // turns; keying only by session collapses every turn's
                    // `adapter.turn_completed` evidence onto ONE row, so the
                    // `ON CONFLICT(evidence_id) DO UPDATE` of the next turn destroys
                    // the prior turn's observed evidence -- exactly the overwrite the
                    // auditor and historical report must not suffer (the observed
                    // `stdout.txt`-reuse pattern). Per-turn keying keeps each turn's
                    // evidence recoverable after restart/rebuild.
                    evidence_id: EvidenceId::new(format!(
                        "evidence-adapter-replay-{}-{}-{}",
                        adapter_event.adapter_kind.as_str(),
                        refs.session_id,
                        adapter_replay_evidence_discriminator(adapter_event, turn_id_override),
                    )),
                    project_id: self.project_id.clone(),
                    task_id: Some(refs.task_id.clone()),
                    session_id: Some(refs.session_id.clone()),
                    run_id: Some(refs.run_id.clone()),
                    kind: format!("adapter_replay:{}", adapter_event.adapter_kind.as_str()),
                    artifact_id: None,
                    confidence: if task.capo_execution_status == "active" {
                        78
                    } else {
                        60
                    },
                    updated_sequence: 0,
                }),
            ))),
            "adapter.permission_requested" => Ok(Some((
                EventKind::PermissionRequested,
                ProjectionRecord::Session(SessionProjection {
                    session_id: refs.session_id.clone(),
                    project_id: self.project_id.clone(),
                    task_id: Some(refs.task_id.clone()),
                    agent_id: refs.agent_id.clone(),
                    title: session.title.clone(),
                    status: "waiting_for_permission".to_string(),
                    current_goal: session.current_goal.clone(),
                    latest_summary: session.latest_summary.clone(),
                    latest_confidence: session.latest_confidence,
                    latest_blocker: Some(format!(
                        "Adapter {} requested permission for content_hash={}",
                        adapter_event.adapter_kind.as_str(),
                        adapter_event
                            .content
                            .as_ref()
                            .map(|content| stable_hash(content.as_bytes()))
                            .unwrap_or_else(|| adapter_event.raw_event_hash.clone())
                    )),
                    external_session_ref: session.external_session_ref.clone(),
                    updated_sequence: 0,
                }),
            ))),
            "adapter.turn_failed" | "adapter.turn_interrupted" => {
                let interrupted = adapter_event.kind == "adapter.turn_interrupted";
                Ok(Some((
                    if interrupted {
                        EventKind::SessionInterrupted
                    } else {
                        EventKind::RunExited
                    },
                    ProjectionRecord::Session(SessionProjection {
                        session_id: refs.session_id.clone(),
                        project_id: self.project_id.clone(),
                        task_id: Some(refs.task_id.clone()),
                        agent_id: refs.agent_id.clone(),
                        title: session.title.clone(),
                        status: if interrupted {
                            "canceled".to_string()
                        } else {
                            "failed".to_string()
                        },
                        current_goal: session.current_goal.clone(),
                        latest_summary: Some(format!(
                            "Adapter {} {} content_hash={}",
                            adapter_event.adapter_kind.as_str(),
                            if interrupted { "interrupted" } else { "failed" },
                            adapter_event
                                .content
                                .as_ref()
                                .map(|content| stable_hash(content.as_bytes()))
                                .unwrap_or_else(|| adapter_event.raw_event_hash.clone())
                        )),
                        latest_confidence: Some(40),
                        latest_blocker: adapter_event.content.as_ref().map(|content| {
                            format!(
                                "Adapter {} {} content_hash={}",
                                adapter_event.adapter_kind.as_str(),
                                if interrupted { "interrupted" } else { "failed" },
                                stable_hash(content.as_bytes())
                            )
                        }),
                        external_session_ref: session.external_session_ref.clone(),
                        updated_sequence: 0,
                    }),
                )))
            }
            "adapter.session_started" | "adapter.raw_event" => Ok(None),
            _ => Ok(None),
        }
    }

    fn adapter_tool_observation_projection(
        &self,
        refs: &FakeRunRefs,
        adapter_event: &NormalizedAdapterEvent,
    ) -> StateResult<Option<ProjectionRecord>> {
        let Some(mut observation) = adapter_event.tool_observation() else {
            return Ok(None);
        };
        if observation.tool_name == "adapter-native-tool" {
            let tool_call_id = adapter_tool_call_id(adapter_event);
            if let Some(existing_tool_name) = self
                .state
                .tool_calls_for_session(&refs.session_id)?
                .into_iter()
                .find(|tool| tool.tool_call_id == tool_call_id)
                .map(|tool| tool.tool_name)
            {
                observation.tool_name = existing_tool_name;
            }
        }
        Ok(Some(ProjectionRecord::ToolObservation(
            tool_observation_projection(refs, adapter_event, &observation),
        )))
    }
}

/// Inline cap for AGENT PROSE surfaced in the persisted event payload. Bounds a
/// runaway message so the event log line stays sane; the full prose still rides
/// the live transcript.
const ADAPTER_PROSE_INLINE_CAP: usize = 16 * 1024;

fn adapter_event_payload_json(adapter_event: &NormalizedAdapterEvent) -> String {
    let content_hash = adapter_event
        .content
        .as_ref()
        .map(|content| stable_hash(content.as_bytes()));
    // SLICE-A LEGIBILITY (acceptance #1/#2): for AGENT MESSAGE prose (assistant
    // item_delta / item_completed) carry the REAL WORDS inline under "content"
    // (JSON-escaped, length-capped) so `/api/events` (SSE), `/api/thread`, and
    // the chat reply-fallback render the conductor's text instead of the
    // "adapter.item_delta" label. This is PROSE legibility only -- credential-
    // shaped TOOL-payload redaction is a separate path and is untouched: tool-
    // call kinds (`adapter.tool_call_*`) never take this branch, so their
    // payloads keep carrying only refs/hashes.
    let is_assistant_prose = matches!(
        adapter_event.kind.as_str(),
        "adapter.item_completed" | "adapter.item_delta"
    ) && adapter_event.role.as_deref() == Some("assistant");
    let content_field = match (is_assistant_prose, adapter_event.content.as_deref()) {
        (true, Some(content)) if !content.is_empty() => {
            let capped: String = content.chars().take(ADAPTER_PROSE_INLINE_CAP).collect();
            format!(",\"content\":\"{}\"", escape_json(&capped))
        }
        _ => String::new(),
    };
    format!(
        "{{\"adapter_kind\":\"{}\",\"provider_event_kind\":\"{}\",\"normalized_kind\":\"{}\",\"external_session_ref\":\"{}\",\"external_item_ref\":\"{}\",\"timeline_key\":\"{}\",\"timeline_confidence\":\"{:?}\",\"tool_name\":\"{}\",\"status\":\"{}\",\"content_hash\":\"{}\",\"raw_event_hash\":\"{}\"{}}}",
        adapter_event.adapter_kind.as_str(),
        escape_json(&adapter_event.provider_event_kind),
        escape_json(&adapter_event.kind),
        escape_json(
            adapter_event
                .external_session_ref
                .as_deref()
                .unwrap_or("none")
        ),
        escape_json(adapter_event.external_item_ref.as_deref().unwrap_or("none")),
        escape_json(adapter_event.timeline_key.as_deref().unwrap_or("none")),
        adapter_event.timeline_confidence,
        escape_json(adapter_event.tool_name.as_deref().unwrap_or("none")),
        escape_json(adapter_event.status.as_deref().unwrap_or("none")),
        content_hash.as_deref().unwrap_or("none"),
        escape_json(&adapter_event.raw_event_hash),
        content_field,
    )
}

fn adapter_event_identity(adapter_event: &NormalizedAdapterEvent, fallback_index: usize) -> String {
    let mut identity = slug(
        adapter_event
            .idempotency_key
            .as_deref()
            .or(adapter_event.timeline_key.as_deref())
            .or(adapter_event.external_item_ref.as_deref())
            .unwrap_or(&adapter_event.raw_event_hash),
    );
    identity = identity.chars().take(96).collect::<String>();
    identity = identity.trim_matches('-').to_string();
    if identity.is_empty() {
        format!("event-{fallback_index}")
    } else {
        identity
    }
}

fn adapter_tool_call_id(adapter_event: &NormalizedAdapterEvent) -> ToolCallId {
    ToolCallId::new(format!(
        "tool-adapter-{}",
        slug(
            adapter_event
                .external_item_ref
                .as_deref()
                .or(adapter_event.timeline_key.as_deref())
                .unwrap_or(&adapter_event.raw_event_hash)
        )
    ))
}

/// The fallback idempotency key for an adapter-replay event that carries no
/// provider-supplied key.
///
/// RTL8/RTL12: when a turn id is supplied (the loop drives a turn explicitly),
/// the key is scoped by `(session, turn, index)` so DISTINCT turns in the same
/// session/run no longer collide -- a second turn's terminal/summary/tool events
/// would otherwise dedup against the first turn's at the same batch index and be
/// silently dropped. Re-running the SAME turn keeps the same key, so per-turn
/// replay stays idempotent. With no turn id (a single logical replay) the key
/// stays `(session, index)`, preserving the prior single-turn behavior.
fn adapter_replay_fallback_key(
    session_id: &SessionId,
    turn_id_override: Option<&str>,
    index: usize,
    suffix: Option<&str>,
) -> String {
    let mut key = match turn_id_override {
        Some(turn) => format!("adapter-replay:{session_id}:{turn}:{index}"),
        None => format!("adapter-replay:{session_id}:{index}"),
    };
    if let Some(suffix) = suffix {
        key.push(':');
        key.push_str(suffix);
    }
    key
}

/// GA6 (GO13): the per-turn discriminator for an `adapter.turn_completed` evidence
/// row id, so successive provider turns in the same session DO NOT overwrite each
/// other's observed evidence.
///
/// It mirrors [`adapter_replay_turn_id`] / [`adapter_event_identity`]: when the loop
/// drives a turn explicitly the turn id is the stable key (so re-replaying the SAME
/// turn stays idempotent on one row); otherwise the event's own timeline/item key
/// (falling back to its raw-event hash) discriminates one turn from the next. The
/// result is slugged so it is a safe, stable id fragment.
fn adapter_replay_evidence_discriminator(
    adapter_event: &NormalizedAdapterEvent,
    turn_id_override: Option<&str>,
) -> String {
    let raw = turn_id_override
        .map(ToString::to_string)
        .or_else(|| adapter_event.timeline_key.clone())
        .or_else(|| adapter_event.external_item_ref.clone())
        .unwrap_or_else(|| adapter_event.raw_event_hash.clone());
    let discriminator = slug(&raw);
    if discriminator.is_empty() {
        adapter_event.raw_event_hash.clone()
    } else {
        discriminator
    }
}

fn adapter_replay_turn_id(
    adapter_event: &NormalizedAdapterEvent,
    turn_id_override: Option<&str>,
) -> Option<String> {
    turn_id_override
        .map(ToString::to_string)
        .or_else(|| {
            adapter_event
                .timeline_key
                .as_ref()
                .map(|key| format!("turn-{}", slug(key)))
        })
        .or_else(|| Some("turn-adapter-replay".to_string()))
}

fn tool_observation_projection(
    refs: &FakeRunRefs,
    adapter_event: &NormalizedAdapterEvent,
    observation: &AdapterToolObservation,
) -> capo_state::ToolObservationProjection {
    capo_state::ToolObservationProjection {
        tool_observation_id: format!(
            "tool-observation-{}-{}",
            adapter_event.adapter_kind.as_str(),
            slug(
                observation
                    .external_tool_ref
                    .as_deref()
                    .or(adapter_event.timeline_key.as_deref())
                    .unwrap_or(&observation.raw_event_hash)
            )
        ),
        session_id: refs.session_id.clone(),
        tool_call_id: Some(adapter_tool_call_id(adapter_event)),
        source: format!("adapter_event:{}", observation.source_adapter),
        external_tool_ref: observation.external_tool_ref.clone(),
        tool_name: observation.tool_name.clone(),
        observed_status: observation.observed_status.clone(),
        instrumentation_level: observation.instrumentation_level.clone(),
        confidence: observation.confidence.clone(),
        raw_event_hash: observation.raw_event_hash.clone(),
        artifact_id: adapter_event
            .content
            .as_ref()
            .map(|_| format!("artifact-adapter-output-{}", adapter_event.raw_event_hash)),
        updated_sequence: 0,
    }
}
