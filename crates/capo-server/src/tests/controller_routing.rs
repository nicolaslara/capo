//! RTL11: route default chat/steer through the single-switch controller
//! cutover, keeping the scripted-mock fallback and a fake default.
//!
//! These tests prove the two routings ([`ControllerSelection::Fake`] and
//! [`ControllerSelection::Real`]) both handle `send`/`steer`/`interrupt`/`stop`
//! through the UNCHANGED `ServerCommand` surface, and produce equivalent
//! observable results -- the controller swap is invisible above the boundary.
//! They also pin the phase-1 default to `Fake` and the opt-in env switch.

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
    let final_session_status = agent.session.as_ref().map(|s| s.status.clone());
    let final_run_status = agent.session.as_ref().and_then(|s| s.run_status.clone());

    let state = SqliteStateStore::open(&root).expect("state");
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
    // The terminal stop leaves a non-running session under both routings.
    assert_ne!(fake.final_session_status.as_deref(), Some("active_running"));
}

#[test]
fn default_selection_is_fake_and_chat_does_not_silently_route_real() {
    // Phase-1 invariant: the default selection is fake, so default chat keeps
    // routing through the fake controller until the RTL12 cutover. We assert the
    // default value directly rather than mutating process-global env in a
    // parallel test (the env wiring is covered by the `from_opt_in` unit test).
    assert_eq!(ControllerSelection::default(), ControllerSelection::Fake);
    assert!(!ControllerSelection::default().is_real());

    // A server constructed with the default selection routes through the fake
    // controller, with the unchanged `ServerCommand` surface still working.
    let root = temp_root();
    let server = CapoServer::open_with_controller(
        ProjectId::new("project-capo"),
        &root,
        ControllerSelection::default(),
    )
    .expect("server");
    assert_eq!(server.controller_selection(), ControllerSelection::Fake);
    let registered = handle(
        &server,
        ServerCommand::RegisterAgent {
            name: "default-routing-agent".to_string(),
        },
    );
    assert_eq!(payload_variant(&registered.payload), "AgentRegistered");
}

#[test]
fn opt_in_env_selects_the_real_controller_as_a_single_switch() {
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
    // Anything falsey keeps the default.
    assert_eq!(
        ControllerSelection::from_opt_in("0"),
        ControllerSelection::Fake
    );
    assert_eq!(
        ControllerSelection::from_opt_in(""),
        ControllerSelection::Fake
    );
    assert_eq!(
        ControllerSelection::from_opt_in("disabled"),
        ControllerSelection::Fake
    );
}
