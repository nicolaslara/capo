//! DP1: drive a live [`AcpLiveAdapter`] turn THROUGH the controller so the ACP
//! wire round-trip reuses the single orchestration seam -- not a parallel one.
//!
//! Two findings are closed here:
//!
//! 1. SAFETY (permission authority): the wire client is NOT the policy authority.
//!    The controller installs a [`ControllerAcpDecider`] into the wire client; when
//!    the agent calls `session/request_permission`, the wire client builds an
//!    `AdapterPermissionRequest` and routes it through this decider, which runs the
//!    controller's `decide_adapter_permission` (i.e. `PermissionPolicy::decide` +
//!    the `capability-permissions.md` ACP option mapping + the `permission.requested
//!    -> permission.decided -> capability.grant_created` lifecycle). The wire client
//!    writes back ONLY the controller-returned outcome -- a policy DENY over-rules an
//!    adapter-offered allow, and `must_not_proceed` is honored.
//!
//! 2. INGESTION: the driven turn's normalized `session/update` events flow through
//!    the SAME `apply_normalized_adapter_events_with_turn` ingestion route every
//!    other provider uses, rather than being collapsed into a single `TurnOutput`
//!    summary that discards the per-event batch.

use std::cell::Cell;

use capo_adapters::{
    AcpLiveAdapter, AcpPermissionDecider, AcpTurnTranscript, AdapterPermissionRequest,
    AdapterPermissionResponse, TurnRequest,
};
use capo_core::TurnId;

use super::*;

/// The outcome of one controller-driven live ACP turn: the driven wire transcript
/// plus the report from ingesting its events through the loop's normal route.
#[derive(Debug)]
pub struct AcpLiveTurnOutcome {
    pub transcript: AcpTurnTranscript,
    pub ingest: AdapterReplayReport,
}

/// The controller's policy-authority seam for the live ACP wire client.
///
/// Holds a borrow of the controller and the round-trip scope so every inbound
/// `session/request_permission` is decided by `decide_adapter_permission`
/// (`PermissionPolicy` + ACP option mapping + persisted lifecycle). The wire
/// client writes back ONLY the [`AdapterPermissionResponse`] this returns. On a
/// persistence error the decider FAILS CLOSED (cancel + `must_not_proceed`) rather
/// than authorizing.
struct ControllerAcpDecider<'a> {
    controller: &'a FakeBoundaryController,
    scope: PermissionRoundTripScope,
    /// Monotonic counter so multiple permission round-trips in one turn get
    /// distinct `request_ref`s (and therefore distinct persisted decision ids).
    seq: Cell<u64>,
}

impl AcpPermissionDecider for ControllerAcpDecider<'_> {
    fn decide_acp_permission(
        &self,
        request: &AdapterPermissionRequest,
    ) -> AdapterPermissionResponse {
        let n = self.seq.get();
        self.seq.set(n + 1);
        let mut scope = self.scope.clone();
        scope.request_ref = format!("{}-{n}", self.scope.request_ref);
        match self.controller.decide_adapter_permission(&scope, request) {
            Ok(response) => response,
            // Fail closed: a persistence/decision failure must never become an
            // authorization.
            Err(error) => AdapterPermissionResponse::fail_closed(format!(
                "controller permission decision failed: {error:?}"
            )),
        }
    }
}

impl FakeBoundaryController {
    /// Drive ONE live ACP turn over an already-attached transport (the scripted
    /// transport in tests; the runtime-spawned pipe in the live path) THROUGH the
    /// controller, then ingest its normalized events through the loop's standard
    /// `apply_normalized_adapter_events_with_turn` route.
    ///
    /// The inbound `session/request_permission` round-trip is routed through the
    /// controller's `PermissionPolicy` (a policy DENY over-rules an adapter-offered
    /// allow), and the per-event batch is event-sourced into the read models --
    /// never reduced to a `TurnOutput` summary.
    ///
    /// COOPERATIVE CANCEL (B2): the trailing `cancel` is an OPTIONAL shared flag
    /// forwarded to the adapter's `drive_with_decider`. `None` (the deterministic
    /// smoke suites) is byte-identical to the pre-cancel path. The live server
    /// path passes a registered flag so an `InterruptAgent`/`StopAgent` command
    /// can cooperatively cancel the in-flight turn.
    pub fn drive_acp_live_turn<T: capo_adapters::AcpTransport>(
        &self,
        refs: &FakeRunRefs,
        adapter: &AcpLiveAdapter,
        transport: T,
        request: &TurnRequest,
        cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    ) -> StateResult<AcpLiveTurnOutcome> {
        let turn_id = TurnId::new(format!("turn-acp-live-{}", request.turn_id.as_str()));
        let decider = Box::new(ControllerAcpDecider {
            controller: self,
            scope: PermissionRoundTripScope {
                task_id: refs.task_id.clone(),
                agent_id: refs.agent_id.clone(),
                session_id: refs.session_id.clone(),
                run_id: refs.run_id.clone(),
                turn_id: turn_id.clone(),
                request_ref: format!("acp-live-perm-{}", request.turn_id.as_str()),
            },
            seq: Cell::new(0),
        });

        // SAFETY: `drive_with_decider` installs the controller decider into the
        // wire client, so the wire client routes permission decisions through the
        // policy authority and writes back ONLY the controller-returned outcome.
        let transcript = adapter
            .drive_with_decider(transport, &request.goal, decider, cancel)
            .map_err(|error| StateError::AcpLiveDrive(format!("acp live drive failed: {error}")))?;

        // INGESTION: the per-event batch flows through the loop's normal route, not
        // a parallel one and not a collapsed summary.
        let ingest = self.apply_normalized_adapter_events_with_turn(
            refs,
            &transcript.events,
            Some(turn_id.as_str()),
        )?;

        Ok(AcpLiveTurnOutcome { transcript, ingest })
    }

    /// LIVE STEERING: attach a PERSISTENT live ACP session through the controller
    /// (installs the same policy-authority `ControllerAcpDecider` as
    /// [`Self::drive_acp_live_turn`]). The returned [`capo_adapters::PersistentAcpSession`]
    /// can be `prompt`ed REPEATEDLY on one session; each prompt's transcript is
    /// ingested via [`Self::ingest_acp_prompt`] through the loop's normal route.
    ///
    /// FIDELITY NOTE: the permission decider's `scope.turn_id` is fixed at attach
    /// time (`turn-acp-live-{base}`), so permission-round-trip events for STEERED
    /// follow-up prompts tag the session's initial turn. The session-update events
    /// themselves are ingested under the caller-supplied per-prompt turn id.
    pub fn attach_persistent_acp_session<'c, T: capo_adapters::AcpTransport>(
        &'c self,
        refs: &FakeRunRefs,
        adapter: &AcpLiveAdapter,
        transport: T,
        base_turn_id: &TurnId,
        cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    ) -> StateResult<capo_adapters::PersistentAcpSession<'c, T>> {
        let scope_turn_id = TurnId::new(format!("turn-acp-live-{}", base_turn_id.as_str()));
        let decider = Box::new(ControllerAcpDecider {
            controller: self,
            scope: PermissionRoundTripScope {
                task_id: refs.task_id.clone(),
                agent_id: refs.agent_id.clone(),
                session_id: refs.session_id.clone(),
                run_id: refs.run_id.clone(),
                turn_id: scope_turn_id,
                request_ref: format!("acp-live-perm-{}", base_turn_id.as_str()),
            },
            seq: Cell::new(0),
        });
        adapter
            .attach_persistent_session(transport, decider, cancel)
            .map_err(|error| {
                StateError::AcpLiveDrive(format!("acp persistent attach failed: {error}"))
            })
    }

    /// LIVE STEERING: ingest ONE persistent-session prompt's transcript through
    /// the loop's standard `apply_normalized_adapter_events_with_turn` route under
    /// `ingest_turn_id`. The initial prompt uses `turn-acp-live-{base}` (identical
    /// to [`Self::drive_acp_live_turn`], so the one-shot/zero-window path is
    /// byte-identical); each steered prompt uses a distinct per-prompt turn id.
    pub fn ingest_acp_prompt(
        &self,
        refs: &FakeRunRefs,
        transcript: &AcpTurnTranscript,
        ingest_turn_id: &str,
    ) -> StateResult<AdapterReplayReport> {
        self.apply_normalized_adapter_events_with_turn(
            refs,
            &transcript.events,
            Some(ingest_turn_id),
        )
    }
}
