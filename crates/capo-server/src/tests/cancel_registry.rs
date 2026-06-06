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
