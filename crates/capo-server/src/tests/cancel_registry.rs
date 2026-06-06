//! COOPERATIVE CANCEL (B2): deterministic unit coverage for the in-flight-turn
//! registry on [`CapoServer`] -- `register_in_flight` / `deregister_in_flight` /
//! `cancel_session`.
//!
//! These exercise ONLY the registry + cancel-flag bookkeeping (no live ACP
//! process, no wire). They assert the byte-identical-by-default invariant at the
//! registry layer: with nothing registered, `cancel_session` is a no-op that
//! returns `false` (so `InterruptAgent`/`StopAgent` keep their honest
//! record-intent behavior and never fake delivery), and a registered turn's flag
//! is observable + clearable across `CapoServer` clones (the same `Arc` the
//! detached worker thread holds).

use std::sync::atomic::Ordering;

use super::*;
use crate::SteerSignal;

fn server() -> CapoServer {
    let root = temp_root();
    CapoServer::open(ProjectId::new("project-capo"), &root).expect("server")
}

#[test]
fn cancel_session_is_noop_when_no_turn_registered() {
    let server = server();
    // No live turn in flight: cancel must NOT claim a live delivery. This is the
    // record-intent-only path -- byte-identical to the pre-cancel behavior.
    assert!(!server.cancel_session("session-absent"));
}

#[test]
fn register_then_cancel_flips_the_flag_and_returns_true() {
    let server = server();
    let handle = server.register_in_flight("session-live");
    // The freshly registered flag starts false (no cancel requested yet).
    assert!(!handle.cancel.load(Ordering::Relaxed));

    // A cancel for the registered session is a real signal.
    assert!(server.cancel_session("session-live"));
    // The flag the in-flight turn's pump observes is now set.
    assert!(handle.cancel.load(Ordering::Relaxed));
}

#[test]
fn deregister_makes_cancel_a_noop_again() {
    let server = server();
    let _handle = server.register_in_flight("session-live");
    server.deregister_in_flight("session-live");
    // After deregistration the turn is no longer a cancel target.
    assert!(!server.cancel_session("session-live"));
}

#[test]
fn cancel_flag_is_shared_across_server_clones() {
    // The registry is an `Arc<Mutex<..>>` field, so a `CapoServer` clone (e.g. the
    // one moved into the detached worker thread) shares the SAME registry. A
    // cancel issued through the clone is observed on the handle the original
    // returned.
    let server = server();
    let handle = server.register_in_flight("session-shared");
    let clone = server.clone();
    assert!(clone.cancel_session("session-shared"));
    assert!(handle.cancel.load(Ordering::Relaxed));
}

#[test]
fn reregister_clears_a_stale_flag() {
    // A new turn under a reused session key gets a fresh, un-cancelled flag even
    // if a prior turn's flag was set (defensive: keys can be reused across turns).
    let server = server();
    let first = server.register_in_flight("session-reused");
    assert!(server.cancel_session("session-reused"));
    assert!(first.cancel.load(Ordering::Relaxed));

    let second = server.register_in_flight("session-reused");
    assert!(!second.cancel.load(Ordering::Relaxed));
}

// ---- LIVE STEERING: steer-channel delivery on the same registry ----

#[test]
fn steer_session_is_noop_when_no_steerable_session_registered() {
    let server = server();
    // Nothing registered: honest record-intent, no fake delivery.
    assert!(!server.steer_session("session-absent", "go left"));
    // A one-shot (non-steerable) registration has NO steer channel, so steering
    // it is also a no-op (only persistent sessions are steerable).
    let _h = server.register_in_flight("session-oneshot");
    assert!(!server.steer_session("session-oneshot", "go left"));
}

#[test]
fn steer_session_delivers_to_the_actor_and_flips_cancel() {
    let server = server();
    let (tx, rx) = std::sync::mpsc::channel::<SteerSignal>();
    let handle = server.register_in_flight_steerable("session-steer", tx);
    assert!(!handle.cancel.load(Ordering::Relaxed));

    // A steer for the registered persistent session is a REAL delivery: it flips
    // the cancel flag (aborting any in-flight prompt) and sends the message so the
    // actor re-prompts the SAME session.
    assert!(server.steer_session("session-steer", "now do X"));
    assert!(handle.cancel.load(Ordering::Relaxed));
    match rx.recv().expect("a steer signal was delivered") {
        SteerSignal::Steer(msg) => assert_eq!(msg, "now do X"),
        other => panic!("expected Steer, got {other:?}"),
    }
}

#[test]
fn stop_session_signals_the_actor_to_finalize() {
    let server = server();
    let (tx, rx) = std::sync::mpsc::channel::<SteerSignal>();
    let _handle = server.register_in_flight_steerable("session-stop", tx);
    server.stop_session("session-stop");
    assert!(
        matches!(rx.recv(), Ok(SteerSignal::Stop)),
        "stop_session must deliver a Stop to the actor"
    );
}

#[test]
fn steer_channel_is_shared_across_server_clones() {
    // The registry is shared across clones (the detached worker thread holds one),
    // so a steer issued through a clone reaches the actor's receiver.
    let server = server();
    let (tx, rx) = std::sync::mpsc::channel::<SteerSignal>();
    let _handle = server.register_in_flight_steerable("session-shared-steer", tx);
    let clone = server.clone();
    assert!(clone.steer_session("session-shared-steer", "via clone"));
    assert!(matches!(rx.recv(), Ok(SteerSignal::Steer(m)) if m == "via clone"));
}
