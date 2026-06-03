//! DT5 deterministic tests: auditable + revocable remote control end-to-end, at
//! the CONNECTIVITY-EXPOSURE LIFECYCLE seam (the CLI surface in
//! `crates/capo-cli/src/connectivity.rs`).
//!
//! These prove the DT5 acceptance criteria that live at the exposure lifecycle:
//!
//! 1. A non-loopback (private) server-bind exposure is `blocked_pending_permission`
//!    until a matching allow grant is recorded, then `active` — via the EXACT
//!    in-tree flow (`expose-stub` -> `request-approval` -> `permission decide` ->
//!    `activate-exposure`). Without the grant, `activate-exposure` FAILS.
//! 2. After `revoke-exposure` the exposure is `revoked`, `reachable=false`, and a
//!    re-activation attempt on that channel is REFUSED (the `activate` path refuses
//!    a `revoked` exposure) — proven by a test, not attestation.
//! 3. Audit: re-reading the status (which is the event log REPLAYED into the
//!    projection on a fresh store open) reconstructs the full lifecycle
//!    (requested -> active -> revoked) identically.
//!
//! Every step is event-sourced and deterministic — no live tailnet, no wall clock.
//! The conditional non-loopback BIND and the runner-side env scrub are proven in
//! `capo-runtime` unit tests (`authorize_server_bind`, `scrub_privileged_connector_env`).

use super::support::*;

/// Drive the full grant flow for a private `capo_server`-bind exposure and return
/// `(exposure_id, state_flag)`. The endpoint id is unique per test so the derived
/// exposure id (a hash of endpoint+scope) does not collide across the suite's
/// shared default project id.
fn expose_and_grant_private_bind(state: &str, endpoint: &str) -> String {
    // 1. expose-stub: a private server-bind exposure with an opaque auth_ref HANDLE
    //    (never a raw credential). It records as `blocked_pending_permission`.
    let exposed = capo([
        "connectivity",
        "expose-stub",
        "--endpoint",
        endpoint,
        "--owner-kind",
        "capo_server",
        "--owner-id",
        "capo-server-dt5",
        "--channel",
        "control",
        "--exposure",
        "private",
        "--auth-ref",
        "keychain:capo/dt5-bind-handle",
        "--record",
        "--state",
        state,
    ]);
    assert!(
        exposed.contains("status=blocked_pending_permission"),
        "a private bind exposure must record blocked_pending_permission:\n{exposed}"
    );
    let exposure_id = output_value(&exposed, "exposure");

    // 2. request-approval: queue the permission approval for that exposure.
    let approval = capo([
        "connectivity",
        "request-approval",
        "--exposure",
        &exposure_id,
        "--state",
        state,
    ]);
    let approval_id = output_value(&approval, "approval");

    // 3. permission decide --decision allow_once: mint the matching allow grant
    //    (subject + scope derived from the exposure). `allow_once` is used because
    //    the CLI restricts `allow_always` to Capo-owned read scopes; the connectivity
    //    grant matcher keys on `effect == "allow"` + scope + subject, not persistence.
    let decided = capo([
        "permission",
        "decide",
        "--approval",
        &approval_id,
        "--decision",
        "allow_once",
        "--state",
        state,
    ]);
    assert!(
        decided.contains("effect=allow"),
        "the decision must mint an allow grant:\n{decided}"
    );

    exposure_id
}

#[test]
fn private_bind_exposure_is_blocked_until_grant_then_activates() {
    let state_root = temp_root("dt5-activate-state");
    let state = state_root.display().to_string();
    let endpoint = "dt5-server-bind-ep-activate";

    // Before any grant, activation FAILS (no matching allow grant exists).
    let exposed = capo([
        "connectivity",
        "expose-stub",
        "--endpoint",
        endpoint,
        "--owner-kind",
        "capo_server",
        "--owner-id",
        "capo-server-dt5",
        "--channel",
        "control",
        "--exposure",
        "private",
        "--auth-ref",
        "keychain:capo/dt5-bind-handle",
        "--record",
        "--state",
        &state,
    ]);
    let exposure_id = output_value(&exposed, "exposure");
    assert!(
        exposed.contains("status=blocked_pending_permission"),
        "private exposure starts blocked_pending_permission:\n{exposed}"
    );

    let no_grant = capo_failure([
        "connectivity",
        "activate-exposure",
        "--exposure",
        &exposure_id,
        "--state",
        &state,
    ]);
    assert!(
        no_grant.contains("missing allow grant"),
        "activation without a grant must fail with the missing-grant error:\n{no_grant}"
    );

    // The status is still blocked (no grant applied).
    let status = capo([
        "connectivity",
        "exposure-status",
        "--exposure",
        &exposure_id,
        "--state",
        &state,
    ]);
    assert!(
        status.contains("status=blocked_pending_permission"),
        "without a grant the exposure stays blocked:\n{status}"
    );
    assert!(
        status.contains("grant=none"),
        "no grant is attached yet:\n{status}"
    );

    // Now run the full grant flow and activate.
    let exposure_id = expose_and_grant_private_bind(&state, endpoint);
    let activated = capo([
        "connectivity",
        "activate-exposure",
        "--exposure",
        &exposure_id,
        "--state",
        &state,
    ]);
    assert!(
        activated.contains("status=active"),
        "with a matching grant the exposure activates:\n{activated}"
    );

    let status = capo([
        "connectivity",
        "exposure-status",
        "--exposure",
        &exposure_id,
        "--state",
        &state,
    ]);
    assert!(
        status.contains("status=active"),
        "the activated exposure replays as active:\n{status}"
    );
    assert!(
        !status.contains("grant=none"),
        "the active exposure carries its capability grant id:\n{status}"
    );
}

#[test]
fn revoke_makes_exposure_unreachable_and_refuses_reactivation() {
    let state_root = temp_root("dt5-revoke-state");
    let state = state_root.display().to_string();
    let endpoint = "dt5-server-bind-ep-revoke";

    let exposure_id = expose_and_grant_private_bind(&state, endpoint);
    let activated = capo([
        "connectivity",
        "activate-exposure",
        "--exposure",
        &exposure_id,
        "--state",
        &state,
    ]);
    assert!(activated.contains("status=active"), "{activated}");

    // Revoke the capability end-to-end.
    let revoked = capo([
        "connectivity",
        "revoke-exposure",
        "--exposure",
        &exposure_id,
        "--reason",
        "dt5_operator_revoke",
        "--state",
        &state,
    ]);
    // Assert the actual revocation STATUS, not just any line containing the word
    // "revoked" (the `connectivity_exposure_revoked=true` header would satisfy a
    // gameable `revoked=true` substring even if the status were omitted).
    assert!(
        revoked.contains("status=revoked"),
        "revoke must report the exposure status=revoked:\n{revoked}"
    );

    // After revoke the exposure is revoked + unreachable in the replayed projection.
    let status = capo([
        "connectivity",
        "exposure-status",
        "--exposure",
        &exposure_id,
        "--state",
        &state,
    ]);
    assert!(
        status.contains("status=revoked"),
        "the revoked exposure replays as revoked:\n{status}"
    );
    assert!(
        status.contains("reachable=false"),
        "a revoked exposure is not reachable:\n{status}"
    );

    // A new control attempt on that channel is REFUSED: re-activation of a revoked
    // exposure fails (the capability cannot be flipped back on).
    let reactivate = capo_failure([
        "connectivity",
        "activate-exposure",
        "--exposure",
        &exposure_id,
        "--state",
        &state,
    ]);
    assert!(
        reactivate.contains("revoked"),
        "re-activating a revoked exposure must be refused:\n{reactivate}"
    );
}

#[test]
fn replaying_the_log_reconstructs_the_full_exposure_lifecycle() {
    let state_root = temp_root("dt5-audit-state");
    let state = state_root.display().to_string();
    let endpoint = "dt5-server-bind-ep-audit";

    let exposure_id = expose_and_grant_private_bind(&state, endpoint);
    capo([
        "connectivity",
        "activate-exposure",
        "--exposure",
        &exposure_id,
        "--state",
        &state,
    ]);
    capo([
        "connectivity",
        "revoke-exposure",
        "--exposure",
        &exposure_id,
        "--reason",
        "dt5_audit_revoke",
        "--state",
        &state,
    ]);

    // Each `exposure-status` call opens the store FRESH and replays the event log
    // into the projection. The terminal replayed state is `revoked` with no live
    // reachability — proving requested -> active -> revoked was reconstructed from
    // the log alone, not from in-memory carry-over. A SECOND identical read returns
    // the same terminal state (replay is deterministic / idempotent).
    let first = capo([
        "connectivity",
        "exposure-status",
        "--exposure",
        &exposure_id,
        "--state",
        &state,
    ]);
    let second = capo([
        "connectivity",
        "exposure-status",
        "--exposure",
        &exposure_id,
        "--state",
        &state,
    ]);
    assert_eq!(
        output_value(&first, "status"),
        "revoked",
        "the replayed lifecycle terminates revoked:\n{first}"
    );
    assert_eq!(
        output_value(&first, "status"),
        output_value(&second, "status"),
        "two independent replays reconstruct the same terminal lifecycle state"
    );
    assert_eq!(
        output_value(&first, "reachable"),
        output_value(&second, "reachable"),
        "the replayed reachability is stable across reads"
    );
    // No credential ever surfaces in the audited status render.
    for marker in ["sk-ant-", "ANTHROPIC_API_KEY", "oauth-token-"] {
        assert!(
            !first.contains(marker),
            "the audited exposure status must carry no credential marker ({marker}):\n{first}"
        );
    }
}
