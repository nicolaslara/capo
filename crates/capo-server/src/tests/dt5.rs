//! DT5 (distributed-topology) conditional non-loopback bind tests, from the
//! `capo-server` crate (the `-p capo-server` gate the DT5 section names).
//!
//! DT5 acceptance (review finding 12): the server listener's bind path is
//! loopback-only by DEFAULT — a non-loopback bind HARD-FAILS, byte-for-byte the
//! prior enforcement — and a non-loopback bind is permitted ONLY when an ACTIVE,
//! grant-backed `ExposureBindGrant` is presented. Both branches are proven here
//! against the REAL transport bind guard
//! ([`serve_tcp_with_handler_and_grant`]), with NO client connection required:
//! the guard runs synchronously before the accept loop, and `max_connections =
//! Some(0)` makes the loop return immediately after the guard, so the test is
//! deterministic and never blocks on `accept`.
//!
//! The non-loopback address is `0.0.0.0:0` (INADDR_ANY), which is bindable in the
//! sandbox yet `is_loopback() == false`, so the guard's non-loopback branch is
//! exercised without depending on a routable external interface.

use std::net::TcpListener;
use std::sync::Arc;

use capo_runtime::{ExposureBindGrant, ExposureScope};

use crate::transport::{
    CancellationToken, RequestHandler, ServeConfig, TransportResult, serve_tcp_with_handler,
    serve_tcp_with_handler_and_grant,
};
use crate::{ServerRequest, ServerResponse};

/// A trivial handler: the bind-guard tests never accept a connection, so `handle`
/// is unreachable. It exists only to satisfy the `serve_*` signature.
struct NoopHandler;

impl RequestHandler for NoopHandler {
    fn handle(
        &self,
        _request: ServerRequest,
        _cancel: &CancellationToken,
    ) -> TransportResult<ServerResponse> {
        unreachable!("the bind-guard tests never accept a connection")
    }
}

fn active_bind_grant() -> ExposureBindGrant {
    ExposureBindGrant::from_active_exposure(
        "connectivity-exposure-dt5-server",
        "active",
        Some("grant-approval-dt5-server"),
        "network:connect:private_tunnel",
        Some("keychain:capo/dt5-server-bind-handle"),
        ExposureScope::Private,
    )
    .expect("an active exposure with a grant + handle builds a bind grant")
}

#[test]
fn loopback_bind_is_accepted_with_no_grant_default() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("loopback listener");
    // No grant, loopback address: the guard passes and the zero-sized accept loop
    // returns immediately. This is the all-local default, unchanged.
    let accepted = serve_tcp_with_handler(
        listener,
        Arc::new(NoopHandler),
        Some(0),
        ServeConfig::default(),
    )
    .expect("loopback bind is accepted with no grant");
    assert_eq!(
        accepted, 0,
        "no connection is accepted (max_connections = 0)"
    );
}

#[test]
fn non_loopback_bind_is_refused_without_a_grant() {
    let listener = TcpListener::bind("0.0.0.0:0").expect("non-loopback listener");
    // No grant, non-loopback address: the guard HARD-FAILS before any accept.
    let result = serve_tcp_with_handler_and_grant(
        listener,
        Arc::new(NoopHandler),
        Some(0),
        ServeConfig::default(),
        None,
    );
    let error = result.expect_err("a non-loopback bind must be refused without a grant");
    let message = format!("{error:?}");
    assert!(
        message.contains("loopback"),
        "the refusal must name the loopback requirement: {message}"
    );
}

#[test]
fn non_loopback_bind_is_permitted_with_an_active_grant() {
    let listener = TcpListener::bind("0.0.0.0:0").expect("non-loopback listener");
    // An ACTIVE grant authorizes the non-loopback bind; the guard passes and the
    // zero-sized accept loop returns immediately.
    let accepted = serve_tcp_with_handler_and_grant(
        listener,
        Arc::new(NoopHandler),
        Some(0),
        ServeConfig::default(),
        Some(active_bind_grant()),
    )
    .expect("an active grant authorizes a non-loopback bind");
    assert_eq!(
        accepted, 0,
        "no connection is accepted (max_connections = 0)"
    );
}
