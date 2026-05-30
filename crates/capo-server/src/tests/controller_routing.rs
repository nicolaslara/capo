//! RTL11/RTL12: route default chat/steer through the single-switch controller
//! cutover, keeping the scripted-mock fallback. The RTL12 cutover flipped the
//! default routing to `Real`; the fake routing is the rollback target.
//!
//! These tests prove the two routings ([`ControllerSelection::Fake`] and
//! [`ControllerSelection::Real`]) both handle `send`/`steer`/`interrupt`/`stop`
//! through the UNCHANGED `ServerCommand` surface, and produce equivalent
//! observable results -- the controller swap is invisible above the boundary.
//! They also pin the post-RTL12-cutover default to `Real` and the single-switch
//! env knob (now a falsey-value rollback to `Fake`).
//!
//! Scope: these are boundary-wiring / smoke tests for the RTL11 single switch,
//! not the parity authority. With the default (fake-adapter) core, both
//! routings drive the SAME `FakeBoundaryController` method bodies, so the
//! fake-vs-real `assert_eq!` cannot itself catch a divergent real ORCHESTRATION
//! path -- by construction at this seam there is none. The byte-level parity
//! invariant is owned by RTL5
//! (`crates/capo-controller/src/tests.rs::real_controller_read_models_match_fake_path_for_identical_scripted_output`,
//! over a scripted-mock adapter, plus the restart/replay sibling) and the
//! loop-level parity criterion by RTL12. To give the RTL11 server seam real
//! signal beyond "the swap compiles", `real_routing_over_injected_scripted_mock_drives_that_adapter`
//! drives the `Real` routing over an explicitly injected scripted-mock adapter
//! (the RTL12/RTL13 seam) and asserts the persisted projection payloads reflect
//! that injected adapter's output, not the default fake echo.

use capo_adapters::{AgentAdapterHandle, ScriptedMockAgent};
use capo_core::SessionId;

use super::*;
use crate::ControllerSelection;

/// What an observer above the `ServerCommand` boundary can see after a
/// lifecycle, independent of which controller served it. Adapter-identity
/// fields (ids derived purely from agent/goal) are equal across routings
/// because both drive the same orchestration core over the same inputs.
#[derive(Debug, Eq, PartialEq)]
struct ObservableLifecycle {
    final_agent_status: String,
    final_session_status: Option<String>,
    final_run_status: Option<String>,
    session_event_kinds: Vec<String>,
    response_payload_variants: Vec<&'static str>,
}

fn payload_variant(payload: &ServerResponsePayload) -> &'static str {
    match payload {
        ServerResponsePayload::AgentRegistered(_) => "AgentRegistered",
        ServerResponsePayload::TaskSent(_) => "TaskSent",
        ServerResponsePayload::AgentStatus(_) => "AgentStatus",
        ServerResponsePayload::Agents(_) => "Agents",
        ServerResponsePayload::Dashboard(_) => "Dashboard",
        _ => "Other",
    }
}

/// Drive `register -> send -> steer -> <terminal>` through one server bound to
/// `selection`, and capture what an observer above the boundary sees.
fn run_lifecycle(selection: ControllerSelection, terminal: ServerCommand) -> ObservableLifecycle {
    let root = temp_root();
    let server = CapoServer::open_with_controller(ProjectId::new("project-capo"), &root, selection)
        .expect("server");
    assert_eq!(server.controller_selection(), selection);

    let mut response_payload_variants = Vec::new();

    let registered = handle(
        &server,
        ServerCommand::RegisterAgent {
            name: "routing-agent".to_string(),
        },
    );
    response_payload_variants.push(payload_variant(&registered.payload));

    let sent = handle(
        &server,
        ServerCommand::SendTask {
            agent_name: "routing-agent".to_string(),
            goal: "Route default chat through the selected controller".to_string(),
            scenario: "default".to_string(),
        },
    );
    response_payload_variants.push(payload_variant(&sent.payload));
    let ServerResponsePayload::TaskSent(run) = &sent.payload else {
        panic!("expected task sent response");
    };
    let session_id = run.session_id.clone();

    let steered = handle(
        &server,
        ServerCommand::SteerAgent {
            agent_name: "routing-agent".to_string(),
            goal: "Please refocus on the highest-value subtask".to_string(),
        },
    );
    response_payload_variants.push(payload_variant(&steered.payload));

    let terminated = handle(&server, terminal);
    response_payload_variants.push(payload_variant(&terminated.payload));

    let snapshot = server.dashboard_snapshot().expect("dashboard");
    let agent = snapshot
        .agents
        .iter()
        .find(|agent| agent.name == "routing-agent")
        .expect("agent present");
    let final_agent_status = agent.status.clone();

    // Read the terminal session/run status from the projections directly, keyed
    // by the session id captured at send. The dashboard surfaces a session only
    // through the agent's *current* session, which a terminal stop/interrupt
    // detaches (`current_session_id = None`), so reading via `agent.session`
    // yields `None` after a terminal command -- which is why the prior
    // `assert_ne!` against the nonexistent "active_running" was vacuous. Reading
    // the projection directly captures the real post-terminal status.
    let state = SqliteStateStore::open(&root).expect("state");
    let final_session_status = state
        .session(&session_id)
        .expect("session lookup")
        .map(|session| session.status);
    let final_run_status = state
        .run_for_session(&session_id)
        .expect("run lookup")
        .map(|run| run.status);

    let session_event_kinds = session_event_kinds(&state, &session_id);

    ObservableLifecycle {
        final_agent_status,
        final_session_status,
        final_run_status,
        session_event_kinds,
        response_payload_variants,
    }
}

fn session_event_kinds(state: &SqliteStateStore, session_id: &SessionId) -> Vec<String> {
    let mut events = state
        .recent_events_for_session(session_id, 200)
        .expect("session events");
    // `recent_events_for_session` returns newest-first; order by sequence so the
    // comparison is over the causal order, not the query order.
    events.sort_by_key(|event| event.sequence);
    events
        .into_iter()
        // Drop the server audit envelope: its idempotency key embeds the
        // command id, which is the same across routings, but excluding it keeps
        // the comparison about the controller-emitted domain events.
        .filter(|event| event.kind != "server.request_handled")
        .map(|event| event.kind)
        .collect()
}

#[test]
fn both_routings_handle_send_steer_and_interrupt_equivalently() {
    let interrupt = || ServerCommand::InterruptAgent {
        agent_name: "routing-agent".to_string(),
        reason: "operator pause".to_string(),
    };
    let fake = run_lifecycle(ControllerSelection::Fake, interrupt());
    let real = run_lifecycle(ControllerSelection::Real, interrupt());

    assert_eq!(
        fake, real,
        "fake and real routings diverged for send/steer/interrupt"
    );
    // Sanity: the lifecycle actually exercised the four commands.
    assert_eq!(
        fake.response_payload_variants,
        vec!["AgentRegistered", "TaskSent", "AgentStatus", "AgentStatus"],
    );
    assert!(
        fake.session_event_kinds
            .iter()
            .any(|kind| kind == "session.redirected"),
        "steer must record a redirect: {:?}",
        fake.session_event_kinds
    );
}

#[test]
fn both_routings_handle_send_steer_and_stop_equivalently() {
    let stop = || ServerCommand::StopAgent {
        agent_name: "routing-agent".to_string(),
        reason: "operator stop".to_string(),
    };
    let fake = run_lifecycle(ControllerSelection::Fake, stop());
    let real = run_lifecycle(ControllerSelection::Real, stop());

    assert_eq!(
        fake, real,
        "fake and real routings diverged for send/steer/stop"
    );
    // Positive terminal check (the equality above already proves real == fake,
    // so asserting on `fake` covers both routings): a stop drives the session to
    // the concrete `completed` status and the run to `exited` -- the values the
    // stop path actually writes (`stop_with_turn` /
    // `FakeRuntimeRunner::stop` in capo-controller/capo-runtime). The run is no
    // longer `running`. (The prior `assert_ne!` compared against the literal
    // "active_running", which exists nowhere in the codebase and so was always
    // true and proved nothing.)
    assert_eq!(fake.final_session_status.as_deref(), Some("completed"));
    assert_eq!(fake.final_run_status.as_deref(), Some("exited"));
    assert_ne!(fake.final_run_status.as_deref(), Some("running"));
}

#[test]
fn post_cutover_default_selection_is_real_with_a_one_value_fake_rollback() {
    // RTL12 cutover: the default selection is now `Real`, because the parity
    // suite passes. We assert the default value directly rather than mutating
    // process-global env in a parallel test (the env wiring is covered by the
    // `from_opt_in` unit test).
    assert_eq!(ControllerSelection::default(), ControllerSelection::Real);
    assert!(ControllerSelection::default().is_real());

    // A server constructed with the default selection routes through the real
    // controller, with the unchanged `ServerCommand` surface still working.
    let root = temp_root();
    let server = CapoServer::open_with_controller(
        ProjectId::new("project-capo"),
        &root,
        ControllerSelection::default(),
    )
    .expect("server");
    assert_eq!(server.controller_selection(), ControllerSelection::Real);
    let registered = handle(
        &server,
        ServerCommand::RegisterAgent {
            name: "default-routing-agent".to_string(),
        },
    );
    assert_eq!(payload_variant(&registered.payload), "AgentRegistered");

    // The documented rollback is a single value: the falsey env knob restores
    // the fake routing, with no schema/projection/`ServerCommand` change.
    assert_eq!(
        ControllerSelection::from_opt_in("0"),
        ControllerSelection::Fake
    );
    let fake_server = CapoServer::open_with_controller(
        ProjectId::new("project-capo"),
        temp_root(),
        ControllerSelection::Fake,
    )
    .expect("server");
    assert_eq!(
        fake_server.controller_selection(),
        ControllerSelection::Fake
    );
}

#[test]
fn real_routing_over_injected_scripted_mock_drives_that_adapter() {
    // RTL12/RTL13 seam check: the `Real` routing over an INJECTED scripted-mock
    // adapter must actually drive that adapter (not the default fake echo). A
    // bare scripted-mock with no scripted turns produces deterministic,
    // adapter-specific summaries -- `Scripted mock accepted goal: {goal}` on
    // send/steer and `Scripted mock stopped session: {reason}` on stop (see
    // `capo-adapters/src/scripted_mock_agent.rs`). Asserting the persisted
    // session projection carries those payloads proves the routed command
    // surface reached the injected adapter through the single switch, which the
    // near-tautological fake-vs-fake equality tests cannot show.
    let root = temp_root();
    let adapter = AgentAdapterHandle::scripted_mock(ScriptedMockAgent::new(
        "rtl11-injected-scripted-session",
    ));
    let server = CapoServer::open_with_controller_and_adapter(
        ProjectId::new("project-capo"),
        &root,
        ControllerSelection::Real,
        adapter,
    )
    .expect("server with injected adapter");
    assert_eq!(server.controller_selection(), ControllerSelection::Real);

    handle(
        &server,
        ServerCommand::RegisterAgent {
            name: "injected-agent".to_string(),
        },
    );
    let sent = handle(
        &server,
        ServerCommand::SendTask {
            agent_name: "injected-agent".to_string(),
            goal: "Drive the injected scripted-mock adapter".to_string(),
            scenario: "default".to_string(),
        },
    );
    let ServerResponsePayload::TaskSent(run) = &sent.payload else {
        panic!("expected task sent response");
    };
    let session_id = run.session_id.clone();
    handle(
        &server,
        ServerCommand::SteerAgent {
            agent_name: "injected-agent".to_string(),
            goal: "Refocus through the injected adapter".to_string(),
        },
    );

    // Read the persisted session projection directly (keyed by session id) so a
    // terminal stop, which detaches the agent's current session, does not hide
    // the projection. After send+steer the session summary is the injected
    // adapter's send_turn output, keyed to the steer goal -- not a
    // fake-controller echo.
    let state = SqliteStateStore::open(&root).expect("state");
    let session = state
        .session(&session_id)
        .expect("session lookup")
        .expect("session present");
    assert_eq!(
        session.latest_summary.as_deref(),
        Some("Scripted mock accepted goal: Refocus through the injected adapter"),
        "steer did not flow through the injected scripted-mock adapter",
    );

    handle(
        &server,
        ServerCommand::StopAgent {
            agent_name: "injected-agent".to_string(),
            reason: "operator stop".to_string(),
        },
    );

    let session = state
        .session(&session_id)
        .expect("session lookup")
        .expect("session present");
    let run = state
        .run_for_session(&session_id)
        .expect("run lookup")
        .expect("run present");
    assert_eq!(session.status, "completed");
    assert_eq!(run.status, "exited");
    assert_eq!(
        session.latest_summary.as_deref(),
        Some("Stopped: operator stop"),
        "stop summary should reflect the stop path over the injected adapter",
    );
}

#[test]
fn opt_in_env_is_the_single_switch_with_a_falsey_fake_rollback() {
    // Truthy values pin the real routing explicitly.
    assert_eq!(
        ControllerSelection::from_opt_in("1"),
        ControllerSelection::Real
    );
    assert_eq!(
        ControllerSelection::from_opt_in("true"),
        ControllerSelection::Real
    );
    assert_eq!(
        ControllerSelection::from_opt_in("on"),
        ControllerSelection::Real
    );
    // Falsey values are the documented rollback knob: they force the fake
    // routing back on after the RTL12 cutover.
    assert_eq!(
        ControllerSelection::from_opt_in("0"),
        ControllerSelection::Fake
    );
    assert_eq!(
        ControllerSelection::from_opt_in("false"),
        ControllerSelection::Fake
    );
    assert_eq!(
        ControllerSelection::from_opt_in("off"),
        ControllerSelection::Fake
    );
    // An empty value defers to the post-cutover default (`Real`).
    assert_eq!(
        ControllerSelection::from_opt_in(""),
        ControllerSelection::Real
    );
    // An unparsable value conservatively keeps the fake routing.
    assert_eq!(
        ControllerSelection::from_opt_in("disabled"),
        ControllerSelection::Fake
    );
}
