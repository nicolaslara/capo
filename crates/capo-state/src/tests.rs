use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn prototype_state_backend_is_sqlite() {
    assert_eq!(PROTOTYPE_STATE_BACKEND, "sqlite");
}

#[test]
fn fake_store_reports_state_boundary() {
    assert_eq!(StateStore::fake().binding().kind, BoundaryKind::StateStore);
}

#[test]
fn sqlite_store_persists_events_and_core_projections() {
    let store = temp_store("core-projections");
    let project_id = ProjectId::new("project-capo");
    let task_id = TaskId::new("task-p2");
    let agent_id = AgentId::new("agent-fake");
    let session_id = SessionId::new("session-fake");
    let run_id = RunId::new("run-fake");

    let sequence = store
        .append_event(
            NewEvent {
                event_id: "event-1".to_string(),
                kind: EventKind::SessionStarted,
                actor: "test".to_string(),
                project_id: Some(project_id.clone()),
                task_id: Some(task_id.clone()),
                agent_id: Some(agent_id.clone()),
                session_id: Some(session_id.clone()),
                run_id: Some(run_id.clone()),
                turn_id: None,
                item_id: None,
                payload_json: "{\"kind\":\"session.started\"}".to_string(),
                idempotency_key: Some("session-started:test".to_string()),
                redaction_state: RedactionState::Safe,
            },
            &[
                ProjectionRecord::Project(ProjectProjection {
                    project_id: project_id.clone(),
                    name: "Capo".to_string(),
                    status: "active".to_string(),
                    updated_sequence: 0,
                }),
                ProjectionRecord::Task(TaskProjection {
                    task_id: task_id.clone(),
                    project_id: project_id.clone(),
                    title: "P2".to_string(),
                    capo_execution_status: "active".to_string(),
                    active_session_id: Some(session_id.clone()),
                    latest_summary: Some("state scaffold".to_string()),
                    evidence_id: None,
                    updated_sequence: 0,
                }),
                ProjectionRecord::Agent(AgentProjection {
                    agent_id: agent_id.clone(),
                    project_id: project_id.clone(),
                    name: "fake".to_string(),
                    status: "active".to_string(),
                    current_session_id: Some(session_id.clone()),
                    updated_sequence: 0,
                }),
                ProjectionRecord::Session(SessionProjection {
                    session_id: session_id.clone(),
                    project_id: project_id.clone(),
                    task_id: Some(task_id.clone()),
                    agent_id,
                    title: "Fake session".to_string(),
                    status: "starting".to_string(),
                    current_goal: "prove state".to_string(),
                    latest_summary: Some("booting".to_string()),
                    latest_confidence: Some(70),
                    latest_blocker: None,
                    external_session_ref: Some("adapter-session-fake".to_string()),
                    updated_sequence: 0,
                }),
                ProjectionRecord::Run(RunProjection {
                    run_id,
                    session_id: session_id.clone(),
                    status: "running".to_string(),
                    recovery_of_run_id: None,
                    updated_sequence: 0,
                }),
            ],
        )
        .expect("append event");

    assert_eq!(sequence, 1);
    assert_eq!(store.event_count().unwrap(), 1);
    assert_eq!(store.watermark("default").unwrap(), Some(1));
    let session = store.session(&session_id).unwrap().expect("session");
    assert_eq!(session.current_goal, "prove state");
    assert_eq!(session.latest_confidence, Some(70));
    assert_eq!(
        session.external_session_ref.as_deref(),
        Some("adapter-session-fake")
    );
    let task = store.task(&task_id).unwrap().expect("task");
    assert_eq!(task.latest_summary.as_deref(), Some("state scaffold"));

    // external_session_ref rides in payload_json, so confirm it survives a
    // full projection rebuild from the persisted projection records.
    store.rebuild_projections().expect("rebuild projections");
    let rebuilt = store
        .session(&session_id)
        .unwrap()
        .expect("rebuilt session");
    assert_eq!(
        rebuilt.external_session_ref.as_deref(),
        Some("adapter-session-fake")
    );
}

#[test]
fn source_binding_projection_is_persisted_and_rebuilt() {
    let store = temp_store("source-binding-rebuild");
    let project_id = ProjectId::new("project-capo");
    let task_id = TaskId::new("task-source-binding");

    store
        .append_event(
            NewEvent {
                event_id: "event-source-binding".to_string(),
                kind: EventKind::WorkpadTaskImported,
                actor: "test".to_string(),
                project_id: Some(project_id.clone()),
                task_id: Some(task_id.clone()),
                agent_id: None,
                session_id: None,
                run_id: None,
                turn_id: None,
                item_id: Some("source-binding-task-source-binding".to_string()),
                payload_json: "{}".to_string(),
                idempotency_key: None,
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::SourceBinding(SourceBindingProjection {
                source_binding_id: "source-binding-task-source-binding".to_string(),
                project_id: project_id.clone(),
                task_id: task_id.clone(),
                source_kind: "markdown".to_string(),
                source_task_id: "workpads:scaffold:tasks.md#s5".to_string(),
                source_path: "workpads/scaffold/tasks.md".to_string(),
                source_anchor: "S5 - Explicit Source Binding Projection".to_string(),
                source_hash: "hash-source-binding".to_string(),
                binding_status: "active".to_string(),
                updated_sequence: 0,
            })],
        )
        .expect("append source binding");

    let binding = store
        .source_binding_for_task(&task_id)
        .expect("query source binding")
        .expect("source binding");
    assert_eq!(binding.source_kind, "markdown");
    assert_eq!(binding.source_task_id, "workpads:scaffold:tasks.md#s5");
    assert_eq!(binding.source_hash, "hash-source-binding");
    assert_eq!(binding.binding_status, "active");
    assert_eq!(
        store.source_bindings(&project_id).unwrap(),
        vec![binding.clone()]
    );

    store.rebuild_projections().expect("rebuild projections");
    assert_eq!(
        store
            .source_binding_for_task(&task_id)
            .expect("query rebuilt source binding"),
        Some(binding)
    );
}

#[test]
fn tool_observations_are_persisted_and_rebuilt() {
    let store = temp_store("tool-observations");
    let project_id = ProjectId::new("project-capo");
    let session_id = SessionId::new("session-tools-observed");
    let tool_call_id = ToolCallId::new("tool-adapter-native");

    store
        .append_event(
            NewEvent {
                event_id: "event-tool-observation".to_string(),
                kind: EventKind::ToolObservationRecorded,
                actor: "adapter-replay".to_string(),
                project_id: Some(project_id),
                task_id: None,
                agent_id: None,
                session_id: Some(session_id.clone()),
                run_id: None,
                turn_id: None,
                item_id: Some("tool-1".to_string()),
                payload_json: "{\"kind\":\"tool.observation_recorded\"}".to_string(),
                idempotency_key: Some("tool-observation:tool-1:completed".to_string()),
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::ToolObservation(
                ToolObservationProjection {
                    tool_observation_id: "observation-tool-1-completed".to_string(),
                    session_id: session_id.clone(),
                    tool_call_id: Some(tool_call_id.clone()),
                    source: "adapter_event".to_string(),
                    external_tool_ref: Some("tool-1".to_string()),
                    tool_name: "exec_command".to_string(),
                    observed_status: "completed".to_string(),
                    instrumentation_level: "observed_only".to_string(),
                    confidence: "high".to_string(),
                    raw_event_hash: "fnv1a64:testhash".to_string(),
                    artifact_id: Some("artifact-adapter-output".to_string()),
                    updated_sequence: 0,
                },
            )],
        )
        .expect("append observation");

    let observations = store
        .tool_observations_for_session(&session_id)
        .expect("read observations");
    assert_eq!(observations.len(), 1);
    assert_eq!(observations[0].tool_call_id, Some(tool_call_id));
    assert_eq!(observations[0].instrumentation_level, "observed_only");
    assert_eq!(observations[0].confidence, "high");
    assert_eq!(
        observations[0].artifact_id.as_deref(),
        Some("artifact-adapter-output")
    );

    store.rebuild_projections().expect("rebuild projections");
    let rebuilt = store
        .tool_observations_for_session(&session_id)
        .expect("read rebuilt observations");
    assert_eq!(rebuilt, observations);
}

/// ACI8: an agent-reported observation (`source=agent_reported`, carrying
/// confidence) is persisted as a DISTINCT class from observed evidence
/// (`source=runtime_output` / `adapter_event`); the classification survives
/// replay and a duplicate report submission (same idempotency key) dedupes.
#[test]
fn agent_reported_observations_are_distinct_from_observed_and_dedupe_on_replay() {
    let store = temp_store("agent-reported-observations");
    let project_id = ProjectId::new("project-capo");
    let session_id = SessionId::new("session-aci8-state");
    let report_call = ToolCallId::new("tool-agent-report");
    let observed_call = ToolCallId::new("tool-runtime-observed");

    let append_report = |event_suffix: &str| {
        store
            .append_event(
                NewEvent {
                    event_id: format!("event-agent-report-{event_suffix}"),
                    kind: EventKind::ToolObservationRecorded,
                    actor: "agent-report".to_string(),
                    project_id: Some(project_id.clone()),
                    task_id: None,
                    agent_id: None,
                    session_id: Some(session_id.clone()),
                    run_id: None,
                    turn_id: None,
                    item_id: Some(report_call.to_string()),
                    payload_json: "{\"source\":\"agent_reported\"}".to_string(),
                    // The idempotency key duplicate submissions dedupe on.
                    idempotency_key: Some("agent-report:sub-1".to_string()),
                    redaction_state: RedactionState::Safe,
                },
                &[ProjectionRecord::ToolObservation(
                    ToolObservationProjection {
                        tool_observation_id: "agent-report-obs-1".to_string(),
                        session_id: session_id.clone(),
                        tool_call_id: Some(report_call.clone()),
                        source: "agent_reported".to_string(),
                        external_tool_ref: None,
                        tool_name: "capo.complete_requirement".to_string(),
                        observed_status: "reported".to_string(),
                        instrumentation_level: "structured_observed".to_string(),
                        confidence: "80".to_string(),
                        raw_event_hash: "agent-report:tool-agent-report".to_string(),
                        artifact_id: None,
                        updated_sequence: 0,
                    },
                )],
            )
            .expect("append agent report")
    };

    // The agent report (a CLAIM) ...
    append_report("first");
    // ... and an OBSERVED runtime-evidence observation, distinct class.
    store
        .append_event(
            NewEvent {
                event_id: "event-runtime-observed".to_string(),
                kind: EventKind::ToolObservationRecorded,
                actor: "runtime".to_string(),
                project_id: Some(project_id.clone()),
                task_id: None,
                agent_id: None,
                session_id: Some(session_id.clone()),
                run_id: None,
                turn_id: None,
                item_id: Some(observed_call.to_string()),
                payload_json: "{\"source\":\"runtime_output\"}".to_string(),
                idempotency_key: Some("runtime-observed:tool-runtime-observed".to_string()),
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::ToolObservation(
                ToolObservationProjection {
                    tool_observation_id: "runtime-observed-obs-1".to_string(),
                    session_id: session_id.clone(),
                    tool_call_id: Some(observed_call.clone()),
                    source: "runtime_output".to_string(),
                    external_tool_ref: None,
                    tool_name: "capo.test_run".to_string(),
                    observed_status: "completed".to_string(),
                    instrumentation_level: "full".to_string(),
                    confidence: "high".to_string(),
                    raw_event_hash: "fnv1a64:runtimehash".to_string(),
                    artifact_id: Some("artifact-test-output".to_string()),
                    updated_sequence: 0,
                },
            )],
        )
        .expect("append runtime observation");

    // A DUPLICATE report submission (same idempotency key) dedupes: no second row.
    append_report("duplicate");

    let observations = store
        .tool_observations_for_session(&session_id)
        .expect("read observations");
    assert_eq!(
        observations.len(),
        2,
        "the duplicate agent report must dedupe; only the report + the observed row remain"
    );

    let report = observations
        .iter()
        .find(|observation| observation.tool_call_id.as_ref() == Some(&report_call))
        .expect("agent report observation");
    let observed = observations
        .iter()
        .find(|observation| observation.tool_call_id.as_ref() == Some(&observed_call))
        .expect("runtime observed observation");

    // The two are a DISTINCT class: the agent report is `agent_reported`, the
    // runtime evidence is `runtime_output`. Completion is never reachable by the
    // agent claim alone because the two never share a source classification.
    assert_eq!(report.source, "agent_reported");
    assert_eq!(report.confidence, "80");
    assert_eq!(observed.source, "runtime_output");
    assert_ne!(report.source, observed.source);

    // The classification survives a restart/replay.
    store.rebuild_projections().expect("rebuild projections");
    let rebuilt = store
        .tool_observations_for_session(&session_id)
        .expect("read rebuilt observations");
    assert_eq!(
        rebuilt, observations,
        "classification must replay identically"
    );
}

#[test]
fn append_event_is_idempotent_for_project_scoped_keys() {
    let store = temp_store("idempotency");
    let project_id = ProjectId::new("project-capo");
    let task_id = TaskId::new("task-idempotent");

    let first = store
        .append_event(
            NewEvent {
                event_id: "event-idempotent-1".to_string(),
                kind: EventKind::TaskDiscovered,
                actor: "test".to_string(),
                project_id: Some(project_id.clone()),
                task_id: Some(task_id.clone()),
                agent_id: None,
                session_id: None,
                run_id: None,
                turn_id: None,
                item_id: None,
                payload_json: "{}".to_string(),
                idempotency_key: Some("task:discover:one".to_string()),
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::Task(TaskProjection {
                task_id: task_id.clone(),
                project_id: project_id.clone(),
                title: "first".to_string(),
                capo_execution_status: "pending".to_string(),
                active_session_id: None,
                latest_summary: Some("first".to_string()),
                evidence_id: None,
                updated_sequence: 0,
            })],
        )
        .expect("append first");

    let second = store
        .append_event(
            NewEvent {
                event_id: "event-idempotent-2".to_string(),
                kind: EventKind::TaskDiscovered,
                actor: "test".to_string(),
                project_id: Some(project_id.clone()),
                task_id: Some(task_id.clone()),
                agent_id: None,
                session_id: None,
                run_id: None,
                turn_id: None,
                item_id: None,
                payload_json: "{}".to_string(),
                idempotency_key: Some("task:discover:one".to_string()),
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::Task(TaskProjection {
                task_id: task_id.clone(),
                project_id,
                title: "second".to_string(),
                capo_execution_status: "active".to_string(),
                active_session_id: None,
                latest_summary: Some("second".to_string()),
                evidence_id: None,
                updated_sequence: 0,
            })],
        )
        .expect("append duplicate");

    assert_eq!(first, second);
    assert_eq!(store.event_count().unwrap(), 1);
    assert_eq!(
        store
            .task(&task_id)
            .unwrap()
            .expect("task")
            .latest_summary
            .as_deref(),
        Some("first")
    );
}

#[test]
fn recovery_marks_active_looking_runs_exited_unknown_once() {
    let store = temp_store("active-run-recovery");
    let project_id = ProjectId::new("project-capo");
    let session_id = SessionId::new("session-running");
    let run_id = RunId::new("run-running");

    store
        .append_event(
            NewEvent {
                event_id: "event-run-started".to_string(),
                kind: EventKind::RunStarted,
                actor: "test".to_string(),
                project_id: Some(project_id.clone()),
                task_id: None,
                agent_id: None,
                session_id: Some(session_id.clone()),
                run_id: Some(run_id.clone()),
                turn_id: None,
                item_id: None,
                payload_json: "{}".to_string(),
                idempotency_key: Some("run:start".to_string()),
                redaction_state: RedactionState::Safe,
            },
            &[
                ProjectionRecord::Session(SessionProjection {
                    session_id: session_id.clone(),
                    project_id: project_id.clone(),
                    task_id: None,
                    agent_id: AgentId::new("agent-running"),
                    title: "Running session".to_string(),
                    status: "active".to_string(),
                    current_goal: "recover active run".to_string(),
                    latest_summary: None,
                    latest_confidence: None,
                    latest_blocker: None,
                    external_session_ref: None,
                    updated_sequence: 0,
                }),
                ProjectionRecord::Run(RunProjection {
                    run_id: run_id.clone(),
                    session_id: session_id.clone(),
                    status: "running".to_string(),
                    recovery_of_run_id: None,
                    updated_sequence: 0,
                }),
            ],
        )
        .expect("start run");

    assert_eq!(store.active_looking_runs().unwrap().len(), 1);
    let recovered = store
        .mark_active_runs_exited_unknown(&project_id, "recovery-1")
        .expect("recover active runs");
    let recovered_again = store
        .mark_active_runs_exited_unknown(&project_id, "recovery-1")
        .expect("recover active runs idempotently");

    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered_again.len(), 0);
    assert_eq!(
        store.run(&run_id).unwrap().expect("run").status,
        "exited_unknown"
    );
    assert_eq!(store.active_looking_runs().unwrap().len(), 0);
    assert_eq!(store.event_count().unwrap(), 2);
}

#[test]
fn run_aborted_event_projects_aborted_status_and_rebuilds_identically() {
    // RTL7: the `run.aborted` event (emitted when a per-run resource ceiling is
    // exceeded) carries a `Run` projection of status `aborted`, has an
    // idempotency key, and rebuilds identically from the event log.
    let store = temp_store("run-aborted-projection");
    let project_id = ProjectId::new("project-capo");
    let session_id = SessionId::new("session-ceiling");
    let run_id = RunId::new("run-ceiling");

    store
        .append_event(
            NewEvent {
                event_id: "event-run-started".to_string(),
                kind: EventKind::RunStarted,
                actor: "test".to_string(),
                project_id: Some(project_id.clone()),
                task_id: None,
                agent_id: None,
                session_id: Some(session_id.clone()),
                run_id: Some(run_id.clone()),
                turn_id: None,
                item_id: None,
                payload_json: "{}".to_string(),
                idempotency_key: Some("run:start".to_string()),
                redaction_state: RedactionState::Safe,
            },
            &[
                ProjectionRecord::Session(SessionProjection {
                    session_id: session_id.clone(),
                    project_id: project_id.clone(),
                    task_id: None,
                    agent_id: AgentId::new("agent-ceiling"),
                    title: "Ceiling session".to_string(),
                    status: "active".to_string(),
                    current_goal: "run under a ceiling".to_string(),
                    latest_summary: None,
                    latest_confidence: None,
                    latest_blocker: None,
                    external_session_ref: None,
                    updated_sequence: 0,
                }),
                ProjectionRecord::Run(RunProjection {
                    run_id: run_id.clone(),
                    session_id: session_id.clone(),
                    status: "running".to_string(),
                    recovery_of_run_id: None,
                    updated_sequence: 0,
                }),
            ],
        )
        .expect("start run");

    let aborted_event = NewEvent {
        event_id: "event-run-aborted".to_string(),
        kind: EventKind::RunAborted,
        actor: "capo-controller".to_string(),
        project_id: Some(project_id.clone()),
        task_id: None,
        agent_id: None,
        session_id: Some(session_id.clone()),
        run_id: Some(run_id.clone()),
        turn_id: Some("turn-2".to_string()),
        item_id: Some(run_id.to_string()),
        payload_json: "{\"reason_code\":\"max_turns_exceeded\",\"status\":\"aborted\"}".to_string(),
        idempotency_key: Some(
            "run-aborted:project-capo:run-ceiling:max_turns_exceeded".to_string(),
        ),
        redaction_state: RedactionState::Safe,
    };
    let aborted_projection = RunProjection {
        run_id: run_id.clone(),
        session_id: session_id.clone(),
        status: "aborted".to_string(),
        recovery_of_run_id: None,
        updated_sequence: 0,
    };
    store
        .append_event(
            aborted_event.clone(),
            &[ProjectionRecord::Run(aborted_projection.clone())],
        )
        .expect("abort run");

    assert_eq!(EventKind::RunAborted.as_str(), "run.aborted");
    assert_eq!(store.run(&run_id).unwrap().expect("run").status, "aborted");
    // An aborted run is not active-looking, so recovery never resurrects it.
    assert!(store.active_looking_runs().unwrap().is_empty());
    assert_eq!(store.event_count().unwrap(), 2);

    // Idempotent: re-appending the same abort appends nothing and the run stays
    // aborted.
    store
        .append_event(aborted_event, &[ProjectionRecord::Run(aborted_projection)])
        .expect("re-abort run idempotently");
    assert_eq!(store.event_count().unwrap(), 2);

    // Rebuild from the event log: the run is still aborted.
    store.rebuild_projections().expect("rebuild projections");
    assert_eq!(store.run(&run_id).unwrap().expect("run").status, "aborted");
}

#[test]
fn inflight_runs_carry_the_persisted_pid_marker() {
    // RTL10: the in-flight marker (a `run.started` event carrying `external_pid`
    // + the process-group reference, persisted before the spawn returned) is
    // what the orphan reaper reads to recover a crashed run.
    let store = temp_store("inflight-marker");
    let project_id = ProjectId::new("project-capo");
    let session_id = SessionId::new("session-inflight");
    let run_id = RunId::new("run-inflight");

    start_running_run(&store, &project_id, &session_id, &run_id);
    // Persist the in-flight pid marker as the live spawn path does.
    store
        .append_event(
            NewEvent {
                event_id: "event-run-started-inflight".to_string(),
                kind: EventKind::RunStarted,
                actor: "capo-server".to_string(),
                project_id: Some(project_id.clone()),
                task_id: None,
                agent_id: None,
                session_id: Some(session_id.clone()),
                run_id: Some(run_id.clone()),
                turn_id: Some("turn-1".to_string()),
                item_id: Some("local-process-run-inflight".to_string()),
                payload_json: "{\"status\":\"running\",\"runtime_process_ref\":\"local-process-run-inflight\",\"external_pid\":4242,\"boot_id\":\"linux-btime-1700000000\",\"marker\":\"start_requested_inflight\"}".to_string(),
                idempotency_key: Some("server-run-started-inflight:run-inflight:4242".to_string()),
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::Run(RunProjection {
                run_id: run_id.clone(),
                session_id: session_id.clone(),
                status: "running".to_string(),
                recovery_of_run_id: None,
                updated_sequence: 0,
            })],
        )
        .expect("persist in-flight marker");

    let inflight = store.inflight_runs_for_project(&project_id).unwrap();
    assert_eq!(inflight.len(), 1);
    assert_eq!(inflight[0].run_id, run_id);
    assert_eq!(inflight[0].external_pid, Some(4242));
    // The persisted boot id is read back so restart recovery can refuse to reap
    // a recycled PID across a reboot.
    assert_eq!(
        inflight[0].boot_id.as_deref(),
        Some("linux-btime-1700000000")
    );
    assert_eq!(
        inflight[0].runtime_process_ref.as_deref(),
        Some("local-process-run-inflight")
    );
}

#[test]
fn inflight_runs_treat_a_zero_pid_marker_as_no_process() {
    // RTL10 safety: a zero `external_pid` in a marker (e.g. from a defaulted
    // payload) is not a real process group target -- `kill -<0>` would hit the
    // caller's own group -- so it reads back as "no process to reap".
    let store = temp_store("inflight-zero-pid");
    let project_id = ProjectId::new("project-capo");
    let session_id = SessionId::new("session-zero");
    let run_id = RunId::new("run-zero");

    start_running_run(&store, &project_id, &session_id, &run_id);
    store
        .append_event(
            NewEvent {
                event_id: "event-run-started-inflight-zero".to_string(),
                kind: EventKind::RunStarted,
                actor: "capo-server".to_string(),
                project_id: Some(project_id.clone()),
                task_id: None,
                agent_id: None,
                session_id: Some(session_id.clone()),
                run_id: Some(run_id.clone()),
                turn_id: Some("turn-1".to_string()),
                item_id: Some("local-process-run-zero".to_string()),
                payload_json: "{\"status\":\"running\",\"runtime_process_ref\":\"local-process-run-zero\",\"external_pid\":0,\"marker\":\"start_requested_inflight\"}".to_string(),
                idempotency_key: Some("server-run-started-inflight:run-zero:0".to_string()),
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::Run(RunProjection {
                run_id: run_id.clone(),
                session_id: session_id.clone(),
                status: "running".to_string(),
                recovery_of_run_id: None,
                updated_sequence: 0,
            })],
        )
        .expect("persist zero-pid marker");

    let inflight = store.inflight_runs_for_project(&project_id).unwrap();
    assert_eq!(inflight.len(), 1);
    assert_eq!(
        inflight[0].external_pid, None,
        "a zero pid must not be a reapable process group target"
    );
}

#[test]
fn reap_orphaned_runs_records_orphan_and_exit_and_is_idempotent_across_restarts() {
    // RTL10: a restart mid-run reaps the orphaned process group and records the
    // outcome. A still-alive (now reaped) orphan records `run.orphaned`, a
    // terminal `run.exited`, and `run.recovered`; the run is no longer
    // active-looking; repeated restarts that observe the same runtime state
    // append nothing; and the recovered run rebuilds identically from the log.
    let store = temp_store("reap-orphaned");
    let project_id = ProjectId::new("project-capo");
    let session_id = SessionId::new("session-orphan");
    let run_id = RunId::new("run-orphan");

    start_running_run(&store, &project_id, &session_id, &run_id);
    assert_eq!(store.active_looking_runs().unwrap().len(), 1);

    let observation = RunReapObservation {
        run_id: run_id.clone(),
        session_id: session_id.clone(),
        previous_status: "running".to_string(),
        kind: RunReapKind::AliveReaped,
        external_pid: Some(4242),
        observed_runtime_state_hash: "fnv1a64:deadbeefdeadbeef".to_string(),
    };

    let recovered = store
        .reap_orphaned_runs(
            &project_id,
            "recovery-1",
            std::slice::from_ref(&observation),
        )
        .expect("reap orphaned runs");
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].status, "recovered");

    // orphaned -> exited -> recovered were all recorded for the reaped orphan.
    let events = store.recent_events_for_session(&session_id, 16).unwrap();
    let kinds: Vec<&str> = events.iter().map(|event| event.kind.as_str()).collect();
    assert!(kinds.contains(&"run.orphaned"), "kinds: {kinds:?}");
    assert!(kinds.contains(&"run.exited"), "kinds: {kinds:?}");
    assert!(kinds.contains(&"run.recovered"), "kinds: {kinds:?}");

    // The recovered run is terminal: recovery never resurrects it.
    assert!(store.active_looking_runs().unwrap().is_empty());
    let event_count_after_first = store.event_count().unwrap();

    // A repeated restart that observes the SAME runtime state appends nothing.
    let recovered_again = store
        .reap_orphaned_runs(
            &project_id,
            "recovery-2",
            std::slice::from_ref(&observation),
        )
        .expect("reap orphaned runs again");
    assert_eq!(recovered_again.len(), 1);
    assert_eq!(store.event_count().unwrap(), event_count_after_first);

    // Rebuild from the event log: the run is still recovered/terminal.
    store.rebuild_projections().expect("rebuild projections");
    assert_eq!(
        store.run(&run_id).unwrap().expect("run").status,
        "recovered"
    );
    assert!(store.active_looking_runs().unwrap().is_empty());
}

#[test]
fn reap_orphaned_runs_records_exit_for_an_already_gone_run_without_orphan_event() {
    // A run whose process was already gone (no terminal event) reaches a
    // terminal `run.exited` directly -- it is never recorded as orphaned,
    // because no live process was found on restart.
    let store = temp_store("reap-already-gone");
    let project_id = ProjectId::new("project-capo");
    let session_id = SessionId::new("session-gone");
    let run_id = RunId::new("run-gone");

    start_running_run(&store, &project_id, &session_id, &run_id);

    let observation = RunReapObservation {
        run_id: run_id.clone(),
        session_id: session_id.clone(),
        previous_status: "running".to_string(),
        kind: RunReapKind::AlreadyGone,
        external_pid: Some(9999),
        observed_runtime_state_hash: "fnv1a64:0000000000000001".to_string(),
    };
    store
        .reap_orphaned_runs(&project_id, "recovery-1", &[observation])
        .expect("reap already-gone run");

    let kinds: Vec<String> = store
        .recent_events_for_session(&session_id, 16)
        .unwrap()
        .into_iter()
        .map(|event| event.kind)
        .collect();
    assert!(!kinds.iter().any(|kind| kind == "run.orphaned"));
    assert!(kinds.iter().any(|kind| kind == "run.exited"));
    assert!(kinds.iter().any(|kind| kind == "run.recovered"));
    assert!(store.active_looking_runs().unwrap().is_empty());
}

fn start_running_run(
    store: &SqliteStateStore,
    project_id: &ProjectId,
    session_id: &SessionId,
    run_id: &RunId,
) {
    store
        .append_event(
            NewEvent {
                event_id: format!("event-run-started-{run_id}"),
                kind: EventKind::RunStarted,
                actor: "test".to_string(),
                project_id: Some(project_id.clone()),
                task_id: None,
                agent_id: None,
                session_id: Some(session_id.clone()),
                run_id: Some(run_id.clone()),
                turn_id: None,
                item_id: None,
                payload_json: "{}".to_string(),
                idempotency_key: Some(format!("run:start:{run_id}")),
                redaction_state: RedactionState::Safe,
            },
            &[
                ProjectionRecord::Session(SessionProjection {
                    session_id: session_id.clone(),
                    project_id: project_id.clone(),
                    task_id: None,
                    agent_id: AgentId::new("agent-orphan"),
                    title: "Orphan session".to_string(),
                    status: "active".to_string(),
                    current_goal: "recover an orphaned run".to_string(),
                    latest_summary: None,
                    latest_confidence: None,
                    latest_blocker: None,
                    external_session_ref: None,
                    updated_sequence: 0,
                }),
                ProjectionRecord::Run(RunProjection {
                    run_id: run_id.clone(),
                    session_id: session_id.clone(),
                    status: "running".to_string(),
                    recovery_of_run_id: None,
                    updated_sequence: 0,
                }),
            ],
        )
        .expect("start run");
}

#[test]
fn artifacts_tool_grants_memory_and_evidence_are_persisted_and_rebuilt() {
    let store = temp_store("artifact-rebuild");
    let project_id = ProjectId::new("project-capo");
    let session_id = SessionId::new("session-fake");
    let run_id = RunId::new("run-fake");
    let task_id = TaskId::new("task-p2");
    let artifact_id = "artifact-summary";

    store
        .record_artifact(ArtifactRecord {
            artifact_id: artifact_id.to_string(),
            project_id: Some(project_id.clone()),
            session_id: Some(session_id.clone()),
            run_id: Some(run_id.clone()),
            kind: "summary".to_string(),
            uri: "artifacts/raw/summary.md".to_string(),
            content_hash: "hash-summary".to_string(),
            size_bytes: 42,
            redaction_state: RedactionState::Redacted,
        })
        .expect("record artifact");

    store
        .append_event(
            NewEvent::new("event-2", EventKind::EvidenceRecorded, "test"),
            &[
                ProjectionRecord::CapabilityGrant(CapabilityGrantProjection {
                    capability_grant_id: "grant-local".to_string(),
                    capability_profile_id: "trusted-local-dev".to_string(),
                    scope_json: "[\"state:read:project\"]".to_string(),
                    effect: "allow".to_string(),
                    subject_json: "{\"agent\":\"fake\"}".to_string(),
                    decision_source: "allow_trusted_local_profile".to_string(),
                    persistence: "until_session_end".to_string(),
                    explanation: "test grant".to_string(),
                    updated_sequence: 0,
                }),
                ProjectionRecord::ToolCall(ToolCallProjection {
                    tool_call_id: ToolCallId::new("tool-status"),
                    session_id: session_id.clone(),
                    turn_id: Some("turn-1".to_string()),
                    tool_name: "capo.session_summary".to_string(),
                    tool_origin: "capo".to_string(),
                    status: "completed".to_string(),
                    input_artifact_id: None,
                    output_artifact_id: Some(artifact_id.to_string()),
                    provenance: Default::default(),
                    updated_sequence: 0,
                }),
                ProjectionRecord::MemoryPacketRef(MemoryPacketProjection {
                    memory_packet_id: MemoryPacketId::new("packet-1"),
                    project_id: project_id.clone(),
                    task_id: Some(task_id.clone()),
                    agent_id: None,
                    session_id: Some(session_id.clone()),
                    run_id: Some(run_id.clone()),
                    turn_id: Some("turn-1".to_string()),
                    packet_artifact_id: Some(artifact_id.to_string()),
                    purpose: "turn_context".to_string(),
                    updated_sequence: 0,
                }),
                ProjectionRecord::Evidence(EvidenceProjection {
                    evidence_id: EvidenceId::new("evidence-1"),
                    project_id,
                    task_id: Some(task_id),
                    session_id: Some(session_id),
                    run_id: Some(run_id),
                    kind: "summary".to_string(),
                    artifact_id: Some(artifact_id.to_string()),
                    confidence: 80,
                    updated_sequence: 0,
                }),
            ],
        )
        .expect("append evidence event");

    store.rebuild_projections().expect("rebuild projections");
    assert_eq!(store.watermark("default").unwrap(), Some(1));

    let connection = Connection::open(store.db_path()).unwrap();
    for (table, expected) in [
        ("artifacts", 1),
        ("capability_grants", 1),
        ("tool_calls", 1),
        ("memory_packet_refs", 1),
        ("evidence", 1),
    ] {
        let count: i64 = connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, expected, "{table}");
    }

    let grants = store.capability_grants().expect("read grants");
    assert_eq!(grants.len(), 1);
    assert_eq!(grants[0].decision_source, "allow_trusted_local_profile");
    assert_eq!(grants[0].persistence, "until_session_end");
    assert_eq!(grants[0].explanation, "test grant");
}

#[test]
fn tool_call_provenance_and_timing_persist_and_rebuild_identically() {
    // ACI7: the per-call provenance (correlation_id, permission_decision_id,
    // capability_grant_use_id) and wall-clock timing (started_at/completed_at)
    // are persisted on the `ToolCall` projection AND rebuild byte-identically on
    // replay, so provenance is queryable and survives a restart.
    let store = temp_store("tool-call-provenance-rebuild");
    let session_id = SessionId::new("session-prov");

    let provenance = ToolCallProvenance {
        correlation_id: Some("corr-session-prov-run-1-turn-1-tool-prov".to_string()),
        permission_decision_id: Some("decision-grant-allow-abc".to_string()),
        capability_grant_use_id: Some("grant-use-tool-prov-grant-allow-abc".to_string()),
        started_at: Some(1_700_000_000_123),
        completed_at: Some(1_700_000_000_456),
    };

    store
        .append_event(
            NewEvent::new("event-tool-prov", EventKind::ToolCallCompleted, "test"),
            &[ProjectionRecord::ToolCall(ToolCallProjection {
                tool_call_id: ToolCallId::new("tool-prov"),
                session_id: session_id.clone(),
                turn_id: Some("turn-1".to_string()),
                tool_name: "capo.file_read".to_string(),
                tool_origin: "runtime".to_string(),
                status: "completed".to_string(),
                input_artifact_id: Some("artifact-input".to_string()),
                output_artifact_id: Some("artifact-output".to_string()),
                provenance: provenance.clone(),
                updated_sequence: 0,
            })],
        )
        .expect("append tool call with provenance");

    // Provenance is queryable from the live projection.
    let before = store
        .tool_calls_for_session(&session_id)
        .expect("read tool calls");
    assert_eq!(before.len(), 1);
    assert_eq!(before[0].provenance, provenance);
    assert_eq!(before[0].provenance.started_at, Some(1_700_000_000_123));
    assert_eq!(before[0].provenance.completed_at, Some(1_700_000_000_456));

    // A restart/replay (rebuild from the event-sourced projection records)
    // reconstructs the exact same provenance and timing.
    store.rebuild_projections().expect("rebuild projections");
    let after = store
        .tool_calls_for_session(&session_id)
        .expect("read tool calls after rebuild");
    assert_eq!(after, before, "tool call must rebuild identically");
    assert_eq!(after[0].provenance, provenance);
}

/// ACI11: REOPEN the state store from disk (a true restart, not just an
/// in-process rebuild), rebuild projections from the event log, and assert the
/// tool-call, observation, AND agent-report projections rebuild IDENTICALLY,
/// and that an adapter-native tool update with a stable external id deduped on
/// append (`tool-exposure.md:352`).
///
/// This is the load-bearing replay-identity gate for ACI11: a fresh
/// `SqliteStateStore::open` over the same root sees only the persisted event
/// log, derives the read models from scratch, and yields byte-identical
/// projections across all three tool classes -- so a Capo restart loses
/// nothing and an adapter that re-sends the same `toolCallId` never doubles a
/// row.
#[test]
fn aci11_reopened_store_rebuilds_tool_call_observation_and_report_projections_identically() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("capo-state-aci11-reopen-{nanos}"));

    let project_id = ProjectId::new("project-capo");
    let session_id = SessionId::new("session-aci11-reopen");
    let tool_call = ToolCallId::new("tool-aci11-observed");
    let report_call = ToolCallId::new("tool-aci11-report");
    let adapter_call = ToolCallId::new("tool-aci11-adapter");

    // -- First "boot": write the three tool-projection classes + an
    //    adapter-native observation with a stable external id. --
    let (tools_before, observations_before, event_count_before) = {
        let store = SqliteStateStore::open(&root).expect("open state store");

        // 1) A tool-call (ToolInvocation) projection with provenance.
        store
            .append_event(
                NewEvent::new("event-aci11-call", EventKind::ToolCallCompleted, "runtime"),
                &[ProjectionRecord::ToolCall(ToolCallProjection {
                    tool_call_id: tool_call.clone(),
                    session_id: session_id.clone(),
                    turn_id: Some("turn-aci11".to_string()),
                    tool_name: "capo.file_read".to_string(),
                    tool_origin: "runtime".to_string(),
                    status: "completed".to_string(),
                    input_artifact_id: Some("artifact-input".to_string()),
                    output_artifact_id: Some("artifact-output".to_string()),
                    provenance: ToolCallProvenance {
                        correlation_id: Some("corr-aci11".to_string()),
                        permission_decision_id: Some("decision-aci11".to_string()),
                        capability_grant_use_id: Some("grant-use-aci11".to_string()),
                        started_at: Some(1_700_000_000_001),
                        completed_at: Some(1_700_000_000_002),
                    },
                    updated_sequence: 0,
                })],
            )
            .expect("append tool call");

        // 2) An OBSERVED runtime-evidence observation for that call.
        store
            .append_event(
                NewEvent {
                    event_id: "event-aci11-observed".to_string(),
                    kind: EventKind::ToolObservationRecorded,
                    actor: "runtime".to_string(),
                    project_id: Some(project_id.clone()),
                    task_id: None,
                    agent_id: None,
                    session_id: Some(session_id.clone()),
                    run_id: None,
                    turn_id: Some("turn-aci11".to_string()),
                    item_id: Some(tool_call.to_string()),
                    payload_json: "{\"source\":\"runtime_output\"}".to_string(),
                    idempotency_key: Some("runtime-observed:tool-aci11-observed".to_string()),
                    redaction_state: RedactionState::Safe,
                },
                &[ProjectionRecord::ToolObservation(
                    ToolObservationProjection {
                        tool_observation_id: "obs-aci11-observed".to_string(),
                        session_id: session_id.clone(),
                        tool_call_id: Some(tool_call.clone()),
                        source: "runtime_output".to_string(),
                        external_tool_ref: None,
                        tool_name: "capo.file_read".to_string(),
                        observed_status: "completed".to_string(),
                        instrumentation_level: "full".to_string(),
                        confidence: "observed".to_string(),
                        raw_event_hash: "fnv1a64:observedhash".to_string(),
                        artifact_id: Some("artifact-output".to_string()),
                        updated_sequence: 0,
                    },
                )],
            )
            .expect("append observed observation");

        // 3) An `agent_reported` claim (distinct class), carrying confidence.
        store
            .append_event(
                NewEvent {
                    event_id: "event-aci11-report".to_string(),
                    kind: EventKind::ToolObservationRecorded,
                    actor: "agent-report".to_string(),
                    project_id: Some(project_id.clone()),
                    task_id: None,
                    agent_id: None,
                    session_id: Some(session_id.clone()),
                    run_id: None,
                    turn_id: Some("turn-aci11".to_string()),
                    item_id: Some(report_call.to_string()),
                    payload_json: "{\"source\":\"agent_reported\"}".to_string(),
                    idempotency_key: Some("agent-report:sub-aci11".to_string()),
                    redaction_state: RedactionState::Safe,
                },
                &[ProjectionRecord::ToolObservation(
                    ToolObservationProjection {
                        tool_observation_id: "obs-aci11-report".to_string(),
                        session_id: session_id.clone(),
                        tool_call_id: Some(report_call.clone()),
                        source: "agent_reported".to_string(),
                        external_tool_ref: None,
                        tool_name: "capo.complete_subtask".to_string(),
                        observed_status: "reported".to_string(),
                        instrumentation_level: "structured_observed".to_string(),
                        confidence: "90".to_string(),
                        raw_event_hash: "agent-report:tool-aci11-report".to_string(),
                        artifact_id: None,
                        updated_sequence: 0,
                    },
                )],
            )
            .expect("append agent report");

        // 4) An adapter-native tool update with a STABLE external id, sent TWICE
        //    with the same idempotency key. The second append must dedupe
        //    (tool-exposure.md:352: a `toolCallId` is stable within a session).
        let append_adapter = |event_suffix: &str| {
            store
                .append_event(
                    NewEvent {
                        event_id: format!("event-aci11-adapter-{event_suffix}"),
                        kind: EventKind::ToolObservationRecorded,
                        actor: "adapter-replay".to_string(),
                        project_id: Some(project_id.clone()),
                        task_id: None,
                        agent_id: None,
                        session_id: Some(session_id.clone()),
                        run_id: None,
                        turn_id: Some("turn-aci11".to_string()),
                        item_id: Some(adapter_call.to_string()),
                        payload_json: "{\"source\":\"adapter_event\"}".to_string(),
                        idempotency_key: Some(
                            "tool-observation:tool-aci11-adapter:completed".to_string(),
                        ),
                        redaction_state: RedactionState::Safe,
                    },
                    &[ProjectionRecord::ToolObservation(
                        ToolObservationProjection {
                            tool_observation_id: "obs-aci11-adapter".to_string(),
                            session_id: session_id.clone(),
                            tool_call_id: Some(adapter_call.clone()),
                            source: "adapter_event".to_string(),
                            external_tool_ref: Some(adapter_call.to_string()),
                            tool_name: "exec_command".to_string(),
                            observed_status: "completed".to_string(),
                            instrumentation_level: "observed_only".to_string(),
                            confidence: "high".to_string(),
                            raw_event_hash: "fnv1a64:adapterhash".to_string(),
                            artifact_id: None,
                            updated_sequence: 0,
                        },
                    )],
                )
                .expect("append adapter observation")
        };
        append_adapter("first");
        // Same stable external id / idempotency key -> deduped (no second row).
        append_adapter("duplicate");

        let tools = store
            .tool_calls_for_session(&session_id)
            .expect("read tool calls");
        let observations = store
            .tool_observations_for_session(&session_id)
            .expect("read observations");
        // 3 observations (observed + report + adapter), NOT 4: the adapter
        // duplicate deduped on its stable external id.
        assert_eq!(
            observations.len(),
            3,
            "the adapter-native duplicate must dedupe on its stable external id"
        );
        let event_count = store.event_count().expect("event count");
        (tools, observations, event_count)
    };

    // -- Restart: a FRESH store reopened from the same root on disk derives the
    //    read models from the persisted event log alone. --
    let reopened = SqliteStateStore::open(&root).expect("reopen state store");
    reopened.rebuild_projections().expect("rebuild projections");

    assert_eq!(
        reopened
            .tool_calls_for_session(&session_id)
            .expect("reopened tool calls"),
        tools_before,
        "tool-call projection must rebuild identically after reopen",
    );
    assert_eq!(
        reopened
            .tool_observations_for_session(&session_id)
            .expect("reopened observations"),
        observations_before,
        "observation + report projections must rebuild identically after reopen",
    );
    assert_eq!(
        reopened.event_count().expect("reopened event count"),
        event_count_before,
        "reopen + rebuild introduces no new events",
    );
}

#[test]
fn memory_records_and_sources_are_persisted_rebuilt_and_packet_filterable() {
    let store = temp_store("memory-record-rebuild");
    let project_id = ProjectId::new("project-capo");
    let record_id = "memory-record-architecture-static-dispatch";

    store
            .append_event(
                NewEvent {
                    event_id: "event-memory-record-ingested".to_string(),
                    kind: EventKind::MemoryRecordIngested,
                    actor: "test".to_string(),
                    project_id: Some(project_id.clone()),
                    task_id: None,
                    agent_id: None,
                    session_id: None,
                    run_id: None,
                    turn_id: None,
                    item_id: Some(record_id.to_string()),
                    payload_json: "{\"kind\":\"memory.record_ingested\"}".to_string(),
                    idempotency_key: Some("memory:record:static-dispatch".to_string()),
                    redaction_state: RedactionState::Safe,
                },
                &[
                    ProjectionRecord::MemoryRecord(Box::new(MemoryRecordProjection {
                        memory_record_id: record_id.to_string(),
                        project_id: project_id.clone(),
                        scope: "project".to_string(),
                        scope_owner_ref: "project-capo".to_string(),
                        subject_ref: Some("workpads/architecture/boundaries.md".to_string()),
                        sensitivity_classification: "internal".to_string(),
                        record_kind: "repo_convention".to_string(),
                        subject: "architecture boundaries".to_string(),
                        predicate: "prefer".to_string(),
                        object: "static dispatch for known prototype boundaries".to_string(),
                        body: "Use static dispatch for known Capo boundaries while keeping adapter swaps explicit.".to_string(),
                        confidence: "high".to_string(),
                        review_state: "reviewed".to_string(),
                        source_count: 1,
                        valid_from: Some("2026-05-25T00:00:00Z".to_string()),
                        valid_until: None,
                        supersedes_memory_record_id: None,
                        revoked_by_memory_record_id: None,
                        redaction_state: RedactionState::Safe.as_str().to_string(),
                        invalidated_at: None,
                        invalidation_reason: None,
                        packet_item_ref: Some("memory-record:architecture-static-dispatch".to_string()),
                        updated_sequence: 0,
                    })),
                    ProjectionRecord::MemorySource(MemorySourceProjection {
                        memory_source_id: "memory-source-boundaries-static-dispatch".to_string(),
                        memory_record_id: record_id.to_string(),
                        source_kind: "markdown".to_string(),
                        source_event_id: None,
                        source_artifact_id: None,
                        source_path: Some("workpads/architecture/boundaries.md".to_string()),
                        source_anchor: Some("Static Dispatch Shape".to_string()),
                        source_content_hash: Some("sha256:boundaries".to_string()),
                        source_sequence: Some(1),
                        quote_artifact_id: Some("artifact-quote-static-dispatch".to_string()),
                        observed_at: Some("2026-05-25T00:00:00Z".to_string()),
                        updated_sequence: 0,
                    }),
                ],
            )
            .expect("append memory record");

    let records = store
        .memory_records_for_project(&project_id)
        .expect("memory records");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].review_state, "reviewed");
    assert_eq!(records[0].sensitivity_classification, "internal");
    assert_eq!(
        records[0].packet_item_ref.as_deref(),
        Some("memory-record:architecture-static-dispatch")
    );
    assert!(records[0].is_packet_eligible());

    let sources = store
        .memory_sources_for_record(record_id)
        .expect("memory sources");
    assert_eq!(sources.len(), 1);
    assert_eq!(
        sources[0].source_path.as_deref(),
        Some("workpads/architecture/boundaries.md")
    );
    assert_eq!(
        sources[0].source_anchor.as_deref(),
        Some("Static Dispatch Shape")
    );
    assert_eq!(
        sources[0].source_content_hash.as_deref(),
        Some("sha256:boundaries")
    );

    store
            .append_event(
                NewEvent {
                    event_id: "event-memory-record-invalidated".to_string(),
                    kind: EventKind::MemoryRecordInvalidated,
                    actor: "test".to_string(),
                    project_id: Some(project_id.clone()),
                    task_id: None,
                    agent_id: None,
                    session_id: None,
                    run_id: None,
                    turn_id: None,
                    item_id: Some(record_id.to_string()),
                    payload_json: "{\"kind\":\"memory.record_invalidated\"}".to_string(),
                    idempotency_key: Some("memory:record:static-dispatch:invalidated".to_string()),
                    redaction_state: RedactionState::Safe,
                },
                &[ProjectionRecord::MemoryRecord(Box::new(MemoryRecordProjection {
                    memory_record_id: record_id.to_string(),
                    project_id: project_id.clone(),
                    scope: "project".to_string(),
                    scope_owner_ref: "project-capo".to_string(),
                    subject_ref: Some("workpads/architecture/boundaries.md".to_string()),
                    sensitivity_classification: "internal".to_string(),
                    record_kind: "repo_convention".to_string(),
                    subject: "architecture boundaries".to_string(),
                    predicate: "prefer".to_string(),
                    object: "static dispatch for known prototype boundaries".to_string(),
                    body: "Use static dispatch for known Capo boundaries while keeping adapter swaps explicit.".to_string(),
                    confidence: "high".to_string(),
                    review_state: "superseded".to_string(),
                    source_count: 1,
                    valid_from: Some("2026-05-25T00:00:00Z".to_string()),
                    valid_until: Some("2026-05-25T01:00:00Z".to_string()),
                    supersedes_memory_record_id: None,
                    revoked_by_memory_record_id: Some("memory-record-new-convention".to_string()),
                    redaction_state: RedactionState::Safe.as_str().to_string(),
                    invalidated_at: Some("2026-05-25T01:00:00Z".to_string()),
                    invalidation_reason: Some("superseded by clearer boundary note".to_string()),
                    packet_item_ref: Some("memory-record:architecture-static-dispatch".to_string()),
                    updated_sequence: 0,
                }))],
            )
            .expect("append invalidation");

    assert!(
        store
            .packet_eligible_memory_records(&project_id)
            .expect("packet eligible records")
            .is_empty()
    );

    store.rebuild_projections().expect("rebuild projections");
    let rebuilt = store
        .memory_records_for_project(&project_id)
        .expect("rebuilt memory records");
    assert_eq!(rebuilt.len(), 1);
    assert_eq!(rebuilt[0].review_state, "superseded");
    assert_eq!(
        rebuilt[0].invalidation_reason.as_deref(),
        Some("superseded by clearer boundary note")
    );
    assert_eq!(
        store
            .memory_sources_for_record(record_id)
            .expect("rebuilt memory sources")[0]
            .source_content_hash
            .as_deref(),
        Some("sha256:boundaries")
    );
}

#[test]
fn packet_eligible_memory_records_require_replayable_sources() {
    let store = temp_store("memory-record-packet-eligibility");
    let project_id = ProjectId::new("project-capo");
    let complete_record = reviewed_memory_record(&project_id, "memory-record-complete", 1);
    let no_source_count_record =
        reviewed_memory_record(&project_id, "memory-record-no-source-count", 0);
    let missing_hash_record = reviewed_memory_record(&project_id, "memory-record-no-hash", 1);

    store
        .append_event(
            NewEvent::new(
                "event-memory-packet-eligibility",
                EventKind::MemoryRecordIngested,
                "test",
            ),
            &[
                ProjectionRecord::MemoryRecord(Box::new(complete_record)),
                ProjectionRecord::MemorySource(MemorySourceProjection {
                    memory_source_id: "memory-source-complete".to_string(),
                    memory_record_id: "memory-record-complete".to_string(),
                    source_kind: "markdown".to_string(),
                    source_event_id: None,
                    source_artifact_id: None,
                    source_path: Some("workpads/prototype/knowledge.md".to_string()),
                    source_anchor: Some("Prototype Gate".to_string()),
                    source_content_hash: Some("sha256:complete".to_string()),
                    source_sequence: Some(1),
                    quote_artifact_id: None,
                    observed_at: None,
                    updated_sequence: 0,
                }),
                ProjectionRecord::MemoryRecord(Box::new(no_source_count_record)),
                ProjectionRecord::MemoryRecord(Box::new(missing_hash_record)),
                ProjectionRecord::MemorySource(MemorySourceProjection {
                    memory_source_id: "memory-source-missing-hash".to_string(),
                    memory_record_id: "memory-record-no-hash".to_string(),
                    source_kind: "markdown".to_string(),
                    source_event_id: None,
                    source_artifact_id: None,
                    source_path: Some("workpads/prototype/knowledge.md".to_string()),
                    source_anchor: Some("Prototype Gate".to_string()),
                    source_content_hash: None,
                    source_sequence: Some(2),
                    quote_artifact_id: None,
                    observed_at: None,
                    updated_sequence: 0,
                }),
            ],
        )
        .expect("append memory eligibility records");

    let eligible = store
        .packet_eligible_memory_records(&project_id)
        .expect("eligible records");
    assert_eq!(eligible.len(), 1);
    assert_eq!(eligible[0].memory_record_id, "memory-record-complete");
}

#[test]
fn rebuild_fails_closed_on_incomplete_memory_record_payloads() {
    let store = temp_store("memory-record-malformed-projection");
    store
        .append_event(
            NewEvent::new(
                "event-malformed-memory-source",
                EventKind::MemoryRecordIngested,
                "test",
            ),
            &[],
        )
        .unwrap();

    let connection = Connection::open(store.db_path()).unwrap();
    connection
        .execute(
            "INSERT INTO projection_records (
                    sequence, projection_kind, record_id, a, b, c, d, e, f, g, h, payload_json
                 ) VALUES (1, 'memory_record', 'memory-record-bad', 'project-capo',
                    'project', 'project-capo', NULL, 'internal', 'fact', 'reviewed', '1', '{}')",
            [],
        )
        .unwrap();

    assert!(store.rebuild_projections().is_err());
}

#[test]
fn task_outcome_reports_are_persisted_and_rebuilt() {
    let store = temp_store("task-outcome-report-rebuild");
    let project_id = ProjectId::new("project-capo");
    let task_id = TaskId::new("task-me2");
    let session_id = SessionId::new("session-me2");
    let run_id = RunId::new("run-me2");
    let report_id = "task-outcome-task-me2";

    store
        .append_event(
            NewEvent {
                event_id: "event-task-outcome-report".to_string(),
                kind: EventKind::TaskOutcomeReportGenerated,
                actor: "test".to_string(),
                project_id: Some(project_id.clone()),
                task_id: Some(task_id.clone()),
                agent_id: None,
                session_id: Some(session_id.clone()),
                run_id: Some(run_id.clone()),
                turn_id: None,
                item_id: Some(report_id.to_string()),
                payload_json: "{}".to_string(),
                idempotency_key: None,
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::TaskOutcomeReport(
                TaskOutcomeReportProjection {
                    task_outcome_report_id: report_id.to_string(),
                    project_id: project_id.clone(),
                    task_id: task_id.clone(),
                    session_id,
                    run_id,
                    outcome_status: "completed".to_string(),
                    started_sequence: 2,
                    completed_sequence: 8,
                    duration_sequence_span: 6,
                    action_count: 7,
                    tool_call_count: 2,
                    evidence_count: 3,
                    memory_packet_count: 1,
                    confidence: Some(84),
                    blocker: None,
                    review_outcome: "reviewed_no_blockers".to_string(),
                    report_artifact_id: Some("artifact-task-outcome".to_string()),
                    updated_sequence: 0,
                },
            )],
        )
        .expect("append task outcome report");

    store.rebuild_projections().expect("rebuild projections");
    let reports = store
        .task_outcome_reports_for_task(&task_id)
        .expect("task outcome reports");
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].project_id, project_id);
    assert_eq!(reports[0].outcome_status, "completed");
    assert_eq!(reports[0].duration_sequence_span, 6);
    assert_eq!(reports[0].tool_call_count, 2);
    assert_eq!(reports[0].review_outcome, "reviewed_no_blockers");
    assert_eq!(
        reports[0].report_artifact_id.as_deref(),
        Some("artifact-task-outcome")
    );
}

#[test]
fn review_findings_are_persisted_and_rebuilt() {
    let store = temp_store("review-finding-rebuild");
    let project_id = ProjectId::new("project-capo");
    let task_id = TaskId::new("task-me3");
    let session_id = SessionId::new("session-me3");
    let run_id = RunId::new("run-me3");
    let tool_call_id = ToolCallId::new("tool-me3");
    let finding_id = "review-finding-me3";

    store
        .append_event(
            NewEvent {
                event_id: "event-review-finding".to_string(),
                kind: EventKind::ReviewFindingRecorded,
                actor: "test".to_string(),
                project_id: Some(project_id.clone()),
                task_id: Some(task_id.clone()),
                agent_id: None,
                session_id: Some(session_id.clone()),
                run_id: Some(run_id.clone()),
                turn_id: None,
                item_id: Some(finding_id.to_string()),
                payload_json: "{}".to_string(),
                idempotency_key: None,
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::ReviewFinding(ReviewFindingProjection {
                review_finding_id: finding_id.to_string(),
                project_id: project_id.clone(),
                task_id: task_id.clone(),
                session_id: session_id.clone(),
                run_id: Some(run_id.clone()),
                tool_call_id: Some(tool_call_id.clone()),
                workpad_task_id: Some("ME3".to_string()),
                reviewer: "focused-review".to_string(),
                finding_kind: "blocker".to_string(),
                severity: "high".to_string(),
                summary: "Link findings to follow-up workpad tasks.".to_string(),
                status: "open".to_string(),
                evidence_artifact_id: Some("artifact-review".to_string()),
                follow_up: Some("ME3".to_string()),
                updated_sequence: 0,
            })],
        )
        .expect("append review finding");

    store.rebuild_projections().expect("rebuild projections");
    let findings = store
        .review_findings_for_session(&session_id)
        .expect("review findings");
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].project_id, project_id);
    assert_eq!(findings[0].task_id, task_id);
    assert_eq!(findings[0].run_id.as_ref(), Some(&run_id));
    assert_eq!(findings[0].tool_call_id.as_ref(), Some(&tool_call_id));
    assert_eq!(findings[0].workpad_task_id.as_deref(), Some("ME3"));
    assert_eq!(findings[0].finding_kind, "blocker");
    assert_eq!(findings[0].status, "open");
    assert_eq!(
        findings[0].evidence_artifact_id.as_deref(),
        Some("artifact-review")
    );
}

#[test]
fn permission_approval_projection_is_persisted_and_rebuilt() {
    let store = temp_store("permission-approval-rebuild");
    let project_id = ProjectId::new("project-capo");
    let session_id = SessionId::new("session-fake");
    let approval_id = "approval-shell";
    let grant_id = "grant-approval-shell";

    store
        .append_event(
            NewEvent {
                event_id: "event-approval-queued".to_string(),
                kind: EventKind::PermissionApprovalQueued,
                actor: "test".to_string(),
                project_id: Some(project_id.clone()),
                task_id: None,
                agent_id: None,
                session_id: Some(session_id.clone()),
                run_id: None,
                turn_id: None,
                item_id: Some("tool-call-1".to_string()),
                payload_json: "{}".to_string(),
                idempotency_key: None,
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::PermissionApproval(
                PermissionApprovalProjection {
                    approval_id: approval_id.to_string(),
                    project_id: project_id.clone(),
                    session_id: Some(session_id.clone()),
                    tool_call_id: Some(ToolCallId::new("tool-call-1")),
                    capability_profile_id: "trusted-local-dev".to_string(),
                    scope_json: "[\"tool:invoke:shell\"]".to_string(),
                    subject_json: "{\"actor\":\"local-user\"}".to_string(),
                    status: "pending".to_string(),
                    requested_by: "local-user".to_string(),
                    reason: "run shell".to_string(),
                    decision: None,
                    capability_grant_id: None,
                    updated_sequence: 0,
                },
            )],
        )
        .expect("append queued approval");

    store
        .append_event(
            NewEvent::new(
                "event-approval-decided",
                EventKind::PermissionDecided,
                "test",
            ),
            &[
                ProjectionRecord::PermissionApproval(PermissionApprovalProjection {
                    approval_id: approval_id.to_string(),
                    project_id: project_id.clone(),
                    session_id: Some(session_id),
                    tool_call_id: Some(ToolCallId::new("tool-call-1")),
                    capability_profile_id: "trusted-local-dev".to_string(),
                    scope_json: "[\"tool:invoke:shell\"]".to_string(),
                    subject_json: "{\"actor\":\"local-user\"}".to_string(),
                    status: "decided".to_string(),
                    requested_by: "local-user".to_string(),
                    reason: "run shell".to_string(),
                    decision: Some("reject_always".to_string()),
                    capability_grant_id: Some(grant_id.to_string()),
                    updated_sequence: 0,
                }),
                ProjectionRecord::CapabilityGrant(CapabilityGrantProjection {
                    capability_grant_id: grant_id.to_string(),
                    capability_profile_id: "trusted-local-dev".to_string(),
                    scope_json: "[\"tool:invoke:shell\"]".to_string(),
                    effect: "deny".to_string(),
                    subject_json: "{\"actor\":\"local-user\"}".to_string(),
                    decision_source: "user".to_string(),
                    persistence: "until_revoked".to_string(),
                    explanation: "user approval decision reject_always for approval-shell"
                        .to_string(),
                    updated_sequence: 0,
                }),
            ],
        )
        .expect("append decided approval");

    store.rebuild_projections().expect("rebuild projections");
    let approval = store
        .permission_approval(&project_id, approval_id)
        .expect("approval query")
        .expect("approval");
    assert_eq!(approval.status, "decided");
    assert_eq!(approval.decision.as_deref(), Some("reject_always"));
    assert_eq!(approval.capability_grant_id.as_deref(), Some(grant_id));
    assert_eq!(approval.reason, "run shell");
    let grants = store.capability_grants().expect("grant query");
    assert_eq!(grants.len(), 1);
    assert_eq!(grants[0].effect, "deny");
    assert_eq!(grants[0].persistence, "until_revoked");
}

#[test]
fn runtime_targets_are_persisted_and_rebuilt() {
    let store = temp_store("runtime-target-rebuild");
    let project_id = ProjectId::new("project-capo");
    store
        .append_event(
            NewEvent {
                event_id: "event-runtime-target-registered".to_string(),
                kind: EventKind::RuntimeTargetRegistered,
                actor: "test".to_string(),
                project_id: Some(project_id.clone()),
                task_id: None,
                agent_id: None,
                session_id: None,
                run_id: None,
                turn_id: None,
                item_id: Some("runtime-target-local-1".to_string()),
                payload_json: "{}".to_string(),
                idempotency_key: None,
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::RuntimeTarget(RuntimeTargetProjection {
                runtime_target_id: "runtime-target-local-1".to_string(),
                project_id: project_id.clone(),
                name: "local dev box".to_string(),
                runner_kind: "local-process".to_string(),
                workspace_root: "/tmp/capo-workspace".to_string(),
                artifact_root: "/tmp/capo-artifacts".to_string(),
                default_cwd: "/tmp/capo-workspace".to_string(),
                capability_profile_id: "read-only-local".to_string(),
                connectivity_endpoint_id: Some("endpoint-loopback-1".to_string()),
                status: "available".to_string(),
                updated_sequence: 0,
            })],
        )
        .expect("append runtime target");

    store.rebuild_projections().expect("rebuild projections");
    let targets = store.runtime_targets(&project_id).expect("runtime targets");
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].runtime_target_id, "runtime-target-local-1");
    assert_eq!(targets[0].runner_kind, "local-process");
    assert_eq!(targets[0].workspace_root, "/tmp/capo-workspace");
    assert_eq!(
        targets[0].connectivity_endpoint_id.as_deref(),
        Some("endpoint-loopback-1")
    );
}

#[test]
fn connectivity_exposure_requires_grant_and_projects_revocation_and_health() {
    let store = temp_store("connectivity-exposure-policy");
    let project_id = ProjectId::new("project-capo");
    let exposure_id = "exposure-private-control";
    let grant_id = "grant-private-tunnel";

    store
        .append_event(
            NewEvent {
                event_id: "event-connectivity-exposure-requested".to_string(),
                kind: EventKind::ConnectivityExposureRequested,
                actor: "test".to_string(),
                project_id: Some(project_id.clone()),
                task_id: None,
                agent_id: None,
                session_id: None,
                run_id: None,
                turn_id: None,
                item_id: Some(exposure_id.to_string()),
                payload_json: "{}".to_string(),
                idempotency_key: None,
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::ConnectivityExposure(
                ConnectivityExposureProjection {
                    exposure_id: exposure_id.to_string(),
                    project_id: project_id.clone(),
                    connectivity_endpoint_id: "endpoint-private-1".to_string(),
                    owner_kind: "runtime_target".to_string(),
                    owner_id: "remote-target-1".to_string(),
                    channel_kind: "control".to_string(),
                    exposure: "private".to_string(),
                    permission_scope: "network:connect:private_tunnel".to_string(),
                    status: "blocked_pending_permission".to_string(),
                    capability_grant_id: None,
                    health_status: "unknown".to_string(),
                    reachable: false,
                    revoked_at: None,
                    updated_sequence: 0,
                },
            )],
        )
        .expect("append requested exposure");

    assert_eq!(
        store
            .connectivity_exposures(&project_id)
            .expect("exposures")[0]
            .status,
        "blocked_pending_permission"
    );

    store
        .append_event(
            NewEvent {
                event_id: "event-connectivity-exposure-grant".to_string(),
                kind: EventKind::CapabilityGrantCreated,
                actor: "test".to_string(),
                project_id: Some(project_id.clone()),
                task_id: None,
                agent_id: None,
                session_id: None,
                run_id: None,
                turn_id: None,
                item_id: Some(exposure_id.to_string()),
                payload_json: "{}".to_string(),
                idempotency_key: None,
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::CapabilityGrant(
                CapabilityGrantProjection {
                    capability_grant_id: grant_id.to_string(),
                    capability_profile_id: "remote-control-reviewed".to_string(),
                    scope_json: "[\"network:connect:private_tunnel\"]".to_string(),
                    effect: "allow".to_string(),
                    subject_json:
                        "{\"endpoint_id\":\"endpoint-private-1\",\"owner_id\":\"remote-target-1\"}"
                            .to_string(),
                    decision_source: "user".to_string(),
                    persistence: "until_revoked".to_string(),
                    explanation: "operator allowed private remote-control exposure".to_string(),
                    updated_sequence: 0,
                },
            )],
        )
        .expect("append exposure grant");

    store
        .append_event(
            NewEvent {
                event_id: "event-connectivity-exposure-changed".to_string(),
                kind: EventKind::ConnectivityExposureChanged,
                actor: "test".to_string(),
                project_id: Some(project_id.clone()),
                task_id: None,
                agent_id: None,
                session_id: None,
                run_id: None,
                turn_id: None,
                item_id: Some(exposure_id.to_string()),
                payload_json: "{}".to_string(),
                idempotency_key: None,
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::ConnectivityExposure(
                ConnectivityExposureProjection {
                    exposure_id: exposure_id.to_string(),
                    project_id: project_id.clone(),
                    connectivity_endpoint_id: "endpoint-private-1".to_string(),
                    owner_kind: "runtime_target".to_string(),
                    owner_id: "remote-target-1".to_string(),
                    channel_kind: "control".to_string(),
                    exposure: "private".to_string(),
                    permission_scope: "network:connect:private_tunnel".to_string(),
                    status: "active".to_string(),
                    capability_grant_id: Some(grant_id.to_string()),
                    health_status: "available".to_string(),
                    reachable: true,
                    revoked_at: None,
                    updated_sequence: 0,
                },
            )],
        )
        .expect("append granted exposure");

    let active = store
        .connectivity_exposures(&project_id)
        .expect("active exposure")
        .pop()
        .expect("exposure row");
    assert_eq!(active.status, "active");
    assert_eq!(active.capability_grant_id.as_deref(), Some(grant_id));
    assert_eq!(active.health_status, "available");
    assert!(active.reachable);

    store
        .append_event(
            NewEvent {
                event_id: "event-connectivity-exposure-revoked".to_string(),
                kind: EventKind::ConnectivityExposureRevoked,
                actor: "test".to_string(),
                project_id: Some(project_id.clone()),
                task_id: None,
                agent_id: None,
                session_id: None,
                run_id: None,
                turn_id: None,
                item_id: Some(exposure_id.to_string()),
                payload_json: "{}".to_string(),
                idempotency_key: None,
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::ConnectivityExposure(
                ConnectivityExposureProjection {
                    status: "revoked".to_string(),
                    reachable: false,
                    health_status: "disabled".to_string(),
                    revoked_at: Some("2026-05-25T00:00:00Z".to_string()),
                    capability_grant_id: Some(grant_id.to_string()),
                    exposure_id: exposure_id.to_string(),
                    project_id: project_id.clone(),
                    connectivity_endpoint_id: "endpoint-private-1".to_string(),
                    owner_kind: "runtime_target".to_string(),
                    owner_id: "remote-target-1".to_string(),
                    channel_kind: "control".to_string(),
                    exposure: "private".to_string(),
                    permission_scope: "network:connect:private_tunnel".to_string(),
                    updated_sequence: 0,
                },
            )],
        )
        .expect("append revoked exposure");

    store.rebuild_projections().expect("rebuild projections");
    let revoked = store
        .connectivity_exposures(&project_id)
        .expect("rebuilt exposure")
        .pop()
        .expect("exposure row");
    assert_eq!(revoked.status, "revoked");
    assert_eq!(revoked.health_status, "disabled");
    assert!(!revoked.reachable);
    assert_eq!(revoked.revoked_at.as_deref(), Some("2026-05-25T00:00:00Z"));
}

#[test]
fn adapter_readiness_is_persisted_and_rebuilt() {
    let store = temp_store("adapter-readiness-rebuild");
    let project_id = ProjectId::new("project-capo");

    store
        .append_event(
            NewEvent {
                event_id: "event-adapter-readiness".to_string(),
                kind: EventKind::AdapterReadinessChecked,
                actor: "test".to_string(),
                project_id: Some(project_id.clone()),
                task_id: None,
                agent_id: None,
                session_id: None,
                run_id: None,
                turn_id: None,
                item_id: Some("codex_exec".to_string()),
                payload_json: "{}".to_string(),
                idempotency_key: None,
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::AdapterReadiness(
                AdapterReadinessProjection {
                    adapter_kind: "codex_exec".to_string(),
                    project_id: project_id.clone(),
                    program: "codex".to_string(),
                    opt_in_env: "CAPO_RUN_CODEX_LOCAL_SMOKE".to_string(),
                    opted_in: false,
                    smoke_status: "waiting_on_opt_in".to_string(),
                    credential_policy: "not_inspected".to_string(),
                    expected_marker: "CAPO_CODEX_SMOKE_OK".to_string(),
                    env_allowlist_count: 7,
                    redaction_rule_count: 6,
                    output_limit_bytes: 131072,
                    dogfood_blocker: Some("real_subscription_smoke_not_recorded".to_string()),
                    updated_sequence: 0,
                },
            )],
        )
        .expect("append adapter readiness");

    store.rebuild_projections().expect("rebuild projections");
    let readiness = store
        .adapter_readiness(&project_id)
        .expect("adapter readiness");
    assert_eq!(readiness.len(), 1);
    assert_eq!(readiness[0].adapter_kind, "codex_exec");
    assert_eq!(readiness[0].credential_policy, "not_inspected");
    assert_eq!(
        readiness[0].dogfood_blocker.as_deref(),
        Some("real_subscription_smoke_not_recorded")
    );
}

#[test]
fn adapter_smoke_report_is_persisted_and_rebuilt() {
    let store = temp_store("adapter-smoke-report-rebuild");
    let project_id = ProjectId::new("project-capo");

    store
        .append_event(
            NewEvent {
                event_id: "event-adapter-smoke-report".to_string(),
                kind: EventKind::AdapterSmokeRecorded,
                actor: "test".to_string(),
                project_id: Some(project_id.clone()),
                task_id: None,
                agent_id: None,
                session_id: None,
                run_id: None,
                turn_id: None,
                item_id: Some("adapter-smoke-codex-skipped".to_string()),
                payload_json: "{}".to_string(),
                idempotency_key: None,
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::AdapterSmokeReport(
                AdapterSmokeReportProjection {
                    smoke_report_id: "adapter-smoke-codex-skipped".to_string(),
                    project_id: project_id.clone(),
                    adapter_kind: "codex_exec".to_string(),
                    smoke_status: "skipped".to_string(),
                    credential_scan_status: "not_run".to_string(),
                    marker_found: false,
                    artifact_root: None,
                    reason: "waiting for opt-in".to_string(),
                    dogfood_readiness_effect: "real_subscription_smoke_not_recorded".to_string(),
                    updated_sequence: 0,
                },
            )],
        )
        .expect("append adapter smoke report");

    store.rebuild_projections().expect("rebuild projections");
    let reports = store
        .adapter_smoke_reports(&project_id)
        .expect("adapter smoke reports");
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].adapter_kind, "codex_exec");
    assert_eq!(reports[0].credential_scan_status, "not_run");
    assert_eq!(
        reports[0].dogfood_readiness_effect,
        "real_subscription_smoke_not_recorded"
    );
}

#[test]
fn adapter_dispatch_plan_is_persisted_and_rebuilt() {
    let store = temp_store("adapter-dispatch-plan-rebuild");
    let project_id = ProjectId::new("project-capo");

    store
        .append_event(
            NewEvent {
                event_id: "event-adapter-dispatch-plan".to_string(),
                kind: EventKind::AdapterDispatchPlanned,
                actor: "test".to_string(),
                project_id: Some(project_id.clone()),
                task_id: None,
                agent_id: Some(AgentId::new("agent-codex")),
                session_id: Some(SessionId::new("session-codex")),
                run_id: Some(RunId::new("run-codex")),
                turn_id: None,
                item_id: Some("adapter-dispatch-plan-codex".to_string()),
                payload_json:
                    "{\"runtime_prompt_policy\":\"not_rendered\",\"provider_cli_executed\":false}"
                        .to_string(),
                idempotency_key: None,
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::AdapterDispatchPlan(
                AdapterDispatchPlanProjection {
                    dispatch_plan_id: "adapter-dispatch-plan-codex".to_string(),
                    project_id: project_id.clone(),
                    adapter_kind: "codex_exec".to_string(),
                    provider_kind: "codex_subscription".to_string(),
                    credential_scope: "user_local_subscription".to_string(),
                    agent_id: AgentId::new("agent-codex"),
                    agent_name: "codex".to_string(),
                    session_id: SessionId::new("session-codex"),
                    run_id: RunId::new("run-codex"),
                    runtime_program: "codex".to_string(),
                    runtime_arg_count: 9,
                    runtime_prompt_policy: "not_rendered".to_string(),
                    runtime_cwd: "/tmp/capo-workspace".to_string(),
                    artifact_root: "/tmp/capo-artifacts".to_string(),
                    request_env_count: 0,
                    env_allowlist_count: 7,
                    redaction_rule_count: 6,
                    stdout_format: "jsonl".to_string(),
                    stderr_policy: "logs_redacted".to_string(),
                    provider_cli_executed: false,
                    status: "planned".to_string(),
                    updated_sequence: 0,
                },
            )],
        )
        .expect("append adapter dispatch plan");

    store.rebuild_projections().expect("rebuild projections");
    let plans = store
        .adapter_dispatch_plans(&project_id)
        .expect("adapter dispatch plans");
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].adapter_kind, "codex_exec");
    assert_eq!(plans[0].credential_scope, "user_local_subscription");
    assert_eq!(plans[0].runtime_prompt_policy, "not_rendered");
    assert!(!plans[0].provider_cli_executed);
    assert_eq!(plans[0].status, "planned");
}

#[test]
fn adapter_dispatch_gate_is_persisted_and_rebuilt() {
    let store = temp_store("adapter-dispatch-gate-rebuild");
    let project_id = ProjectId::new("project-capo");

    store
        .append_event(
            NewEvent {
                event_id: "event-adapter-dispatch-gate".to_string(),
                kind: EventKind::AdapterDispatchGateChecked,
                actor: "test".to_string(),
                project_id: Some(project_id.clone()),
                task_id: None,
                agent_id: Some(AgentId::new("agent-codex")),
                session_id: Some(SessionId::new("session-codex")),
                run_id: Some(RunId::new("run-codex")),
                turn_id: None,
                item_id: Some("adapter-dispatch-gate-codex".to_string()),
                payload_json:
                    "{\"provider_cli_execution_allowed\":false,\"provider_cli_executed\":false}"
                        .to_string(),
                idempotency_key: None,
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::AdapterDispatchGate(
                AdapterDispatchGateProjection {
                    dispatch_gate_id: "adapter-dispatch-gate-codex".to_string(),
                    project_id: project_id.clone(),
                    dispatch_plan_id: "adapter-dispatch-plan-codex".to_string(),
                    adapter_kind: "codex_exec".to_string(),
                    provider_cli_execution_allowed: false,
                    status: "blocked".to_string(),
                    required_dogfood_gate: "blocked_pending_real_smoke".to_string(),
                    reason_codes: "codex_exec:real_subscription_smoke_not_recorded".to_string(),
                    provider_cli_executed: false,
                    runtime_prompt_policy: "not_rendered".to_string(),
                    updated_sequence: 0,
                },
            )],
        )
        .expect("append adapter dispatch gate");

    store.rebuild_projections().expect("rebuild projections");
    let gates = store
        .adapter_dispatch_gates(&project_id)
        .expect("adapter dispatch gates");
    assert_eq!(gates.len(), 1);
    assert_eq!(gates[0].dispatch_plan_id, "adapter-dispatch-plan-codex");
    assert_eq!(gates[0].adapter_kind, "codex_exec");
    assert_eq!(gates[0].status, "blocked");
    assert!(!gates[0].provider_cli_execution_allowed);
    assert!(!gates[0].provider_cli_executed);
    assert_eq!(gates[0].runtime_prompt_policy, "not_rendered");
}

#[test]
fn adapter_dispatch_replay_is_persisted_and_rebuilt() {
    let store = temp_store("adapter-dispatch-replay-rebuild");
    let project_id = ProjectId::new("project-capo");

    store
            .append_event(
                NewEvent {
                    event_id: "event-adapter-dispatch-replay".to_string(),
                    kind: EventKind::AdapterDispatchReplayed,
                    actor: "test".to_string(),
                    project_id: Some(project_id.clone()),
                    task_id: Some(TaskId::new("task-codex")),
                    agent_id: Some(AgentId::new("agent-codex")),
                    session_id: Some(SessionId::new("session-codex")),
                    run_id: Some(RunId::new("run-codex")),
                    turn_id: None,
                    item_id: Some("adapter-dispatch-replay-codex".to_string()),
                    payload_json:
                        "{\"provider_cli_executed\":false,\"raw_content_policy\":\"content_hashed_not_rendered\"}"
                            .to_string(),
                    idempotency_key: None,
                    redaction_state: RedactionState::Safe,
                },
                &[ProjectionRecord::AdapterDispatchReplay(
                    AdapterDispatchReplayProjection {
                        dispatch_replay_id: "adapter-dispatch-replay-codex".to_string(),
                        project_id: project_id.clone(),
                        dispatch_plan_id: "adapter-dispatch-plan-codex".to_string(),
                        dispatch_gate_id: "adapter-dispatch-gate-codex".to_string(),
                        adapter_kind: "codex_exec".to_string(),
                        session_id: SessionId::new("session-codex"),
                        run_id: RunId::new("run-codex"),
                        fixture_path: "fixtures/codex-exec.jsonl".to_string(),
                        fixture_hash: "fixture-hash".to_string(),
                        input_event_count: 4,
                        appended_event_count: 4,
                        tool_event_count: 2,
                        summary_event_count: 1,
                        completed_turn_count: 1,
                        provider_cli_executed: false,
                        raw_content_policy: "content_hashed_not_rendered".to_string(),
                        updated_sequence: 0,
                    },
                )],
            )
            .expect("append adapter dispatch replay");

    store.rebuild_projections().expect("rebuild projections");
    let replays = store
        .adapter_dispatch_replays(&project_id)
        .expect("adapter dispatch replays");
    assert_eq!(replays.len(), 1);
    assert_eq!(replays[0].dispatch_plan_id, "adapter-dispatch-plan-codex");
    assert_eq!(replays[0].dispatch_gate_id, "adapter-dispatch-gate-codex");
    assert_eq!(replays[0].adapter_kind, "codex_exec");
    assert_eq!(replays[0].fixture_hash, "fixture-hash");
    assert_eq!(replays[0].tool_event_count, 2);
    assert!(!replays[0].provider_cli_executed);
    assert_eq!(replays[0].raw_content_policy, "content_hashed_not_rendered");
}

#[test]
fn adapter_dispatch_execution_request_is_persisted_and_rebuilt() {
    let store = temp_store("adapter-dispatch-execution-request-rebuild");
    let project_id = ProjectId::new("project-capo");

    store
        .append_event(
            NewEvent {
                event_id: "event-adapter-dispatch-execution-request".to_string(),
                kind: EventKind::AdapterDispatchExecutionRequested,
                actor: "test".to_string(),
                project_id: Some(project_id.clone()),
                task_id: None,
                agent_id: Some(AgentId::new("agent-codex")),
                session_id: Some(SessionId::new("session-codex")),
                run_id: Some(RunId::new("run-codex")),
                turn_id: None,
                item_id: Some("adapter-dispatch-execution-request-codex".to_string()),
                payload_json:
                    "{\"provider_cli_execution_allowed\":true,\"provider_cli_executed\":false}"
                        .to_string(),
                idempotency_key: None,
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::AdapterDispatchExecutionRequest(
                AdapterDispatchExecutionRequestProjection {
                    execution_request_id: "adapter-dispatch-execution-request-codex".to_string(),
                    project_id: project_id.clone(),
                    dispatch_plan_id: "adapter-dispatch-plan-codex".to_string(),
                    dispatch_gate_id: "adapter-dispatch-gate-codex".to_string(),
                    adapter_kind: "codex_exec".to_string(),
                    provider_cli_execution_allowed: true,
                    provider_cli_executed: false,
                    status: "waiting_on_explicit_provider_opt_in".to_string(),
                    opt_in_env: "CAPO_RUN_CODEX_LOCAL_DISPATCH".to_string(),
                    runtime_prompt_policy: "not_rendered".to_string(),
                    reason_codes: "explicit_provider_execution_opt_in_required".to_string(),
                    updated_sequence: 0,
                },
            )],
        )
        .expect("append adapter dispatch execution request");

    store.rebuild_projections().expect("rebuild projections");
    let requests = store
        .adapter_dispatch_execution_requests(&project_id)
        .expect("adapter dispatch execution requests");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].dispatch_plan_id, "adapter-dispatch-plan-codex");
    assert_eq!(requests[0].dispatch_gate_id, "adapter-dispatch-gate-codex");
    assert_eq!(requests[0].status, "waiting_on_explicit_provider_opt_in");
    assert_eq!(requests[0].opt_in_env, "CAPO_RUN_CODEX_LOCAL_DISPATCH");
    assert!(requests[0].provider_cli_execution_allowed);
    assert!(!requests[0].provider_cli_executed);
    assert_eq!(requests[0].runtime_prompt_policy, "not_rendered");
}

#[test]
fn adapter_dispatch_execution_is_persisted_and_rebuilt() {
    let store = temp_store("adapter-dispatch-execution-rebuild");
    let project_id = ProjectId::new("project-capo");

    store
        .append_event(
            NewEvent {
                event_id: "event-adapter-dispatch-execution".to_string(),
                kind: EventKind::AdapterDispatchExecuted,
                actor: "test".to_string(),
                project_id: Some(project_id.clone()),
                task_id: None,
                agent_id: Some(AgentId::new("agent-codex")),
                session_id: Some(SessionId::new("session-codex")),
                run_id: Some(RunId::new("run-codex")),
                turn_id: None,
                item_id: Some("adapter-dispatch-execution-codex".to_string()),
                payload_json:
                    "{\"provider_cli_executed\":true,\"raw_prompt_policy\":\"not_rendered\"}"
                        .to_string(),
                idempotency_key: None,
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::AdapterDispatchExecution(
                AdapterDispatchExecutionProjection {
                    dispatch_execution_id: "adapter-dispatch-execution-codex".to_string(),
                    project_id: project_id.clone(),
                    dispatch_plan_id: "adapter-dispatch-plan-codex".to_string(),
                    execution_request_id: "adapter-dispatch-execution-request-codex".to_string(),
                    adapter_kind: "codex_exec".to_string(),
                    session_id: SessionId::new("session-codex"),
                    run_id: RunId::new("run-codex"),
                    provider_cli_execution_allowed: true,
                    provider_cli_executed: true,
                    status: "exited".to_string(),
                    exit_code: Some(0),
                    runtime_process_ref: Some("local-process-run-codex".to_string()),
                    stdout_artifact_id: Some("artifact-stdout".to_string()),
                    stderr_artifact_id: Some("artifact-stderr".to_string()),
                    artifact_root: "/tmp/capo-artifacts".to_string(),
                    credential_scan_status: "clean".to_string(),
                    raw_prompt_policy: "not_rendered".to_string(),
                    raw_output_policy: "bounded_redacted_artifacts".to_string(),
                    reason_codes: "provider_cli_executed_and_artifacts_scanned".to_string(),
                    updated_sequence: 0,
                },
            )],
        )
        .expect("append adapter dispatch execution");

    store.rebuild_projections().expect("rebuild projections");
    let executions = store
        .adapter_dispatch_executions(&project_id)
        .expect("adapter dispatch executions");
    assert_eq!(executions.len(), 1);
    assert_eq!(
        executions[0].dispatch_plan_id,
        "adapter-dispatch-plan-codex"
    );
    assert_eq!(
        executions[0].execution_request_id,
        "adapter-dispatch-execution-request-codex"
    );
    assert_eq!(executions[0].status, "exited");
    assert_eq!(executions[0].exit_code, Some(0));
    assert!(executions[0].provider_cli_execution_allowed);
    assert!(executions[0].provider_cli_executed);
    assert_eq!(executions[0].credential_scan_status, "clean");
    assert_eq!(executions[0].raw_prompt_policy, "not_rendered");
    assert_eq!(
        executions[0].raw_output_policy,
        "bounded_redacted_artifacts"
    );
}

#[test]
fn adapter_dispatch_prompt_source_is_persisted_and_rebuilt() {
    let store = temp_store("adapter-dispatch-prompt-source-rebuild");
    let project_id = ProjectId::new("project-capo");

    store
        .append_event(
            NewEvent {
                event_id: "event-adapter-dispatch-prompt-source".to_string(),
                kind: EventKind::AdapterDispatchPromptSourceRecorded,
                actor: "test".to_string(),
                project_id: Some(project_id.clone()),
                task_id: None,
                agent_id: Some(AgentId::new("agent-codex")),
                session_id: Some(SessionId::new("session-codex")),
                run_id: Some(RunId::new("run-codex")),
                turn_id: None,
                item_id: Some("adapter-dispatch-prompt-source-codex".to_string()),
                payload_json:
                    "{\"raw_prompt_policy\":\"not_rendered\",\"source_kind\":\"workpad_task\"}"
                        .to_string(),
                idempotency_key: None,
                redaction_state: RedactionState::Safe,
            },
            &[ProjectionRecord::AdapterDispatchPromptSource(
                AdapterDispatchPromptSourceProjection {
                    prompt_source_id: "adapter-dispatch-prompt-source-codex".to_string(),
                    project_id: project_id.clone(),
                    dispatch_plan_id: "adapter-dispatch-plan-codex".to_string(),
                    prompt_hash: "prompt-hash".to_string(),
                    source_kind: "workpad_task".to_string(),
                    source_ref: Some("workpads/features/tasks.md#f1".to_string()),
                    source_hash: Some("source-hash".to_string()),
                    materialization_status: "replayable_if_source_hash_matches".to_string(),
                    raw_prompt_policy: "not_rendered".to_string(),
                    updated_sequence: 0,
                },
            )],
        )
        .expect("append adapter dispatch prompt source");

    store.rebuild_projections().expect("rebuild projections");
    let sources = store
        .adapter_dispatch_prompt_sources(&project_id)
        .expect("adapter dispatch prompt sources");
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].dispatch_plan_id, "adapter-dispatch-plan-codex");
    assert_eq!(sources[0].source_kind, "workpad_task");
    assert_eq!(
        sources[0].source_ref.as_deref(),
        Some("workpads/features/tasks.md#f1")
    );
    assert_eq!(
        sources[0].materialization_status,
        "replayable_if_source_hash_matches"
    );
    assert_eq!(sources[0].raw_prompt_policy, "not_rendered");
}

#[test]
fn adapter_dispatch_prompt_materialization_is_persisted_and_rebuilt() {
    let store = temp_store("adapter-dispatch-prompt-materialization-rebuild");
    let project_id = ProjectId::new("project-capo");

    store
            .append_event(
                NewEvent {
                    event_id: "event-adapter-dispatch-prompt-materialization".to_string(),
                    kind: EventKind::AdapterDispatchPromptMaterialized,
                    actor: "test".to_string(),
                    project_id: Some(project_id.clone()),
                    task_id: None,
                    agent_id: None,
                    session_id: None,
                    run_id: None,
                    turn_id: None,
                    item_id: Some("adapter-dispatch-prompt-materialization-codex".to_string()),
                    payload_json:
                        "{\"raw_prompt_policy\":\"not_rendered\",\"status\":\"ready_without_rendering_prompt\"}"
                            .to_string(),
                    idempotency_key: None,
                    redaction_state: RedactionState::Safe,
                },
                &[ProjectionRecord::AdapterDispatchPromptMaterialization(
                    AdapterDispatchPromptMaterializationProjection {
                        materialization_id: "adapter-dispatch-prompt-materialization-codex"
                            .to_string(),
                        project_id: project_id.clone(),
                        dispatch_plan_id: "adapter-dispatch-plan-codex".to_string(),
                        prompt_source_id: "adapter-dispatch-prompt-source-codex".to_string(),
                        source_kind: "workpad_task".to_string(),
                        source_ref: Some("workpads/features/tasks.md#f1".to_string()),
                        expected_source_hash: Some("source-hash".to_string()),
                        observed_source_hash: Some("source-hash".to_string()),
                        expected_prompt_hash: "prompt-hash".to_string(),
                        materialized_prompt_hash: Some("prompt-hash".to_string()),
                        status: "ready_without_rendering_prompt".to_string(),
                        raw_prompt_policy: "not_rendered".to_string(),
                        reason_codes: "prompt_hash_matches_source".to_string(),
                        updated_sequence: 0,
                    },
                )],
            )
            .expect("append adapter dispatch prompt materialization");

    store.rebuild_projections().expect("rebuild projections");
    let rows = store
        .adapter_dispatch_prompt_materializations(&project_id)
        .expect("adapter dispatch prompt materializations");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, "ready_without_rendering_prompt");
    assert_eq!(rows[0].raw_prompt_policy, "not_rendered");
    assert_eq!(
        rows[0].materialized_prompt_hash.as_deref(),
        Some("prompt-hash")
    );
}

#[test]
fn permission_approval_projection_rejects_invalid_json_payloads() {
    let store = temp_store("permission-approval-invalid-json");
    let project_id = ProjectId::new("project-capo");

    let error = store
        .append_event(
            NewEvent::new(
                "event-invalid-approval-json",
                EventKind::PermissionApprovalQueued,
                "test",
            ),
            &[ProjectionRecord::PermissionApproval(
                PermissionApprovalProjection {
                    approval_id: "approval-invalid".to_string(),
                    project_id,
                    session_id: None,
                    tool_call_id: None,
                    capability_profile_id: "trusted-local-dev".to_string(),
                    scope_json: "[\"tool:invoke:shell\"]".to_string(),
                    subject_json: "{not-json".to_string(),
                    status: "pending".to_string(),
                    requested_by: "local-user".to_string(),
                    reason: "invalid".to_string(),
                    decision: None,
                    capability_grant_id: None,
                    updated_sequence: 0,
                },
            )],
        )
        .expect_err("invalid projection JSON should fail before commit");
    assert!(matches!(
        error,
        StateError::InvalidProjectionJson {
            kind: "permission_approval",
            field: "subject_json",
            ..
        }
    ));
    assert_eq!(store.event_count().expect("event count"), 0);
}

#[test]
fn artifact_persistence_rejects_unclassified_or_sensitive_rows() {
    let store = temp_store("artifact-redaction");
    let artifact = |artifact_id: &str, redaction_state| ArtifactRecord {
        artifact_id: artifact_id.to_string(),
        project_id: None,
        session_id: None,
        run_id: None,
        kind: "raw-output".to_string(),
        uri: "artifacts/raw/output.txt".to_string(),
        content_hash: "hash-output".to_string(),
        size_bytes: 99,
        redaction_state,
    };

    assert!(matches!(
        store.record_artifact(artifact("artifact-unknown", RedactionState::Unknown)),
        Err(StateError::UnsafeArtifactRedactionState(
            RedactionState::Unknown
        ))
    ));
    assert!(matches!(
        store.record_artifact(artifact(
            "artifact-sensitive",
            RedactionState::ContainsSensitive
        )),
        Err(StateError::UnsafeArtifactRedactionState(
            RedactionState::ContainsSensitive
        ))
    ));
}

#[test]
fn rebuild_watermark_tracks_events_without_projection_records() {
    let store = temp_store("empty-projection-watermark");
    let project_id = ProjectId::new("project-capo");
    let task_id = TaskId::new("task-p2");

    store
        .append_event(
            NewEvent::new("event-with-projection", EventKind::TaskDiscovered, "test"),
            &[ProjectionRecord::Task(TaskProjection {
                task_id,
                project_id,
                title: "P2".to_string(),
                capo_execution_status: "active".to_string(),
                active_session_id: None,
                latest_summary: None,
                evidence_id: None,
                updated_sequence: 0,
            })],
        )
        .unwrap();
    store
        .append_event(
            NewEvent::new(
                "event-without-projection",
                EventKind::RecoveryStarted,
                "test",
            ),
            &[],
        )
        .unwrap();

    assert_eq!(store.watermark("default").unwrap(), Some(2));
    store.rebuild_projections().expect("rebuild projections");
    assert_eq!(store.watermark("default").unwrap(), Some(2));
}

#[test]
fn rebuild_fails_closed_on_malformed_projection_numbers() {
    let store = temp_store("malformed-projection");
    store
        .append_event(
            NewEvent::new("event-malformed-source", EventKind::SessionStarted, "test"),
            &[],
        )
        .unwrap();

    let connection = Connection::open(store.db_path()).unwrap();
    connection
        .execute(
            "INSERT INTO projection_records (
                    sequence, projection_kind, record_id, a, b, c, d, e, f, g, h, payload_json
                 ) VALUES (1, 'session', 'session-bad', 'project-capo', NULL,
                    'agent-fake', 'Bad session', 'running', 'prove decode', NULL,
                    'not-a-number', '{}')",
            [],
        )
        .unwrap();

    assert!(store.rebuild_projections().is_err());
}

#[test]
fn recovery_attempts_record_restart_shape_without_mutating_events() {
    let store = temp_store("recovery");
    store
        .append_event(
            NewEvent::new("event-recovery-source", EventKind::RecoveryStarted, "test"),
            &[],
        )
        .unwrap();

    let started = store.begin_recovery("recovery-1").unwrap();
    assert_eq!(started.status, "started");
    assert_eq!(started.started_sequence, 1);
    assert_eq!(store.event_count().unwrap(), 1);

    let completed = store.complete_recovery("recovery-1").unwrap();
    assert_eq!(completed.status, "completed");
    assert_eq!(completed.started_sequence, 1);
    assert_eq!(completed.completed_sequence, Some(1));
    assert_eq!(store.event_count().unwrap(), 1);
}

#[test]
fn recovery_completion_requires_started_attempt() {
    let store = temp_store("missing-recovery");
    assert!(matches!(
        store.complete_recovery("missing"),
        Err(StateError::MissingRecoveryAttempt(id)) if id == "missing"
    ));
}

fn temp_store(name: &str) -> SqliteStateStore {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("capo-state-{name}-{nanos}"));
    SqliteStateStore::open(root).expect("open temp store")
}

fn reviewed_memory_record(
    project_id: &ProjectId,
    memory_record_id: &str,
    source_count: i64,
) -> MemoryRecordProjection {
    MemoryRecordProjection {
        memory_record_id: memory_record_id.to_string(),
        project_id: project_id.clone(),
        scope: "project".to_string(),
        scope_owner_ref: project_id.to_string(),
        subject_ref: Some("workpads/prototype/knowledge.md".to_string()),
        sensitivity_classification: "internal".to_string(),
        record_kind: "fact".to_string(),
        subject: "prototype gate".to_string(),
        predicate: "requires".to_string(),
        object: "source-linked memory".to_string(),
        body: "Prototype memory must stay source linked.".to_string(),
        confidence: "high".to_string(),
        review_state: "reviewed".to_string(),
        source_count,
        valid_from: None,
        valid_until: None,
        supersedes_memory_record_id: None,
        revoked_by_memory_record_id: None,
        redaction_state: RedactionState::Safe.as_str().to_string(),
        invalidated_at: None,
        invalidation_reason: None,
        packet_item_ref: Some(format!("memory-record:{memory_record_id}")),
        updated_sequence: 0,
    }
}

/// Append a minimal, distinctly-keyed event for the ST4 event-tail tests.
fn append_tail_event(store: &SqliteStateStore, project_id: &ProjectId, ordinal: usize) -> i64 {
    store
        .append_event(
            NewEvent {
                event_id: format!("event-tail-{ordinal}"),
                kind: EventKind::SessionSummaryUpdated,
                actor: "tail-test".to_string(),
                project_id: Some(project_id.clone()),
                task_id: None,
                agent_id: None,
                session_id: Some(SessionId::new("session-tail")),
                run_id: None,
                turn_id: None,
                item_id: None,
                payload_json: format!("{{\"ordinal\":{ordinal}}}"),
                idempotency_key: Some(format!("tail:{ordinal}")),
                redaction_state: RedactionState::Safe,
            },
            &[],
        )
        .expect("append tail event")
}

#[test]
fn events_after_returns_only_events_strictly_after_the_watermark_in_order() {
    let store = temp_store("events-after");
    let project_id = ProjectId::new("project-capo");

    let mut sequences = Vec::new();
    for ordinal in 0..5 {
        sequences.push(append_tail_event(&store, &project_id, ordinal));
    }
    // Sequences are monotonic and strictly increasing (the append-only log).
    assert!(
        sequences.windows(2).all(|pair| pair[0] < pair[1]),
        "sequences must be strictly increasing: {sequences:?}"
    );

    // A watermark in the middle returns exactly the events after it, in order.
    let watermark = sequences[1];
    let after = store
        .events_after(watermark, 1024)
        .expect("events_after returns");
    let returned: Vec<i64> = after.iter().map(|event| event.sequence).collect();
    assert_eq!(returned, sequences[2..].to_vec());
    assert!(
        after.iter().all(|event| event.sequence > watermark),
        "every returned event must be strictly after the watermark"
    );

    // A watermark of 0 returns the whole log (no event has sequence 0).
    let from_zero = store.events_after(0, 1024).expect("events_after(0)");
    assert_eq!(
        from_zero
            .iter()
            .map(|event| event.sequence)
            .collect::<Vec<_>>(),
        sequences,
    );

    // A watermark at/after the tail returns nothing (no gap-filling, no replay).
    let tail = *sequences.last().expect("at least one event");
    assert!(
        store
            .events_after(tail, 1024)
            .expect("after tail")
            .is_empty(),
        "no events exist after the latest sequence"
    );

    // The `limit` bounds the catch-up page; callers advance the watermark to page.
    let first_page = store.events_after(0, 2).expect("first page");
    assert_eq!(
        first_page
            .iter()
            .map(|event| event.sequence)
            .collect::<Vec<_>>(),
        sequences[..2].to_vec(),
    );
}

#[test]
fn committed_events_fan_out_to_live_subscribers_after_append() {
    let store = temp_store("events-broadcast");
    let project_id = ProjectId::new("project-capo");

    // Subscribing before any write means the subscriber sees every event the
    // store commits, fanned out after the transaction commits.
    let subscription = store.event_broadcaster().subscribe();
    let seq0 = append_tail_event(&store, &project_id, 0);
    let seq1 = append_tail_event(&store, &project_id, 1);

    let delivered = subscription.drain_pending();
    let delivered_sequences: Vec<i64> = delivered.iter().map(|event| event.sequence).collect();
    assert_eq!(delivered_sequences, vec![seq0, seq1]);
    // A live-delivered event is identical to the catch-up read for that sequence.
    let backlog = store.events_after(0, 1024).expect("backlog");
    assert_eq!(delivered, backlog);

    // A duplicate (idempotent) append commits nothing new and fans nothing out.
    append_tail_event(&store, &project_id, 1);
    assert!(
        subscription.drain_pending().is_empty(),
        "an idempotent no-op append must not be broadcast"
    );

    // Dropping the subscription unsubscribes it on the next publish (pruned).
    drop(subscription);
    append_tail_event(&store, &project_id, 2);
    assert_eq!(
        store.event_broadcaster().subscriber_count(),
        0,
        "a dropped subscriber is pruned from the fan-out on the next publish"
    );
}
