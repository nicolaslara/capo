//! SG3: grant read-back in the decide step + the typed revoke command/flow.
//!
//! The durable grant store is no longer write-only. Two behaviors land here:
//!
//! 1. GRANT READ-BACK in decide ([`FakeBoundaryController::decide_with_grant_read_back`]):
//!    before authorizing, the controller queries the durable grant store and
//!    treats an existing VALID grant for the requested scope as authorization
//!    (grants authorize, not just record). A revoked or expired grant is treated
//!    as ABSENT -- it never authorizes, and it never becomes a standing denial
//!    that read-back would later misread. Expiry is a denial input: a grant past
//!    its `expires_at` does not authorize even if never explicitly revoked.
//!
//! 2. A typed REVOKE command/flow ([`FakeBoundaryController::revoke_capability_grant`])
//!    at the controller boundary that emits `capability.grant_revoked` with a
//!    revocation reason and stamps `revoked_at` on the grant projection. Future
//!    use of a revoked grant is denied while the old `capability.grant_created` /
//!    `capability.grant_used` events remain unchanged on the log.
//!
//! Both rebuild identically from the event log: the revoked/expired state lives
//! in the grant projection's `revoked_at`/`expires_at` columns, which the codec
//! round-trips through the projection payload.

use std::time::{SystemTime, UNIX_EPOCH};

use capo_state::CapabilityGrantProjection;
use capo_tools::{PermissionDecision, PermissionRequest};

use super::*;

/// Wall-clock millis-since-epoch, the instant a grant read-back / revoke is
/// evaluated against `expires_at`/`revoked_at`. Clamped to 0 before the epoch.
fn epoch_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

/// Where a grant revocation hangs on the loop's scope tree, so the persisted
/// `capability.grant_revoked` event carries the same task/agent/session/run/turn
/// provenance the rest of the loop uses.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GrantRevocationScope {
    pub task_id: TaskId,
    pub agent_id: AgentId,
    pub session_id: SessionId,
    pub run_id: RunId,
    pub turn_id: TurnId,
}

/// The recorded outcome of a typed grant revocation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GrantRevocation {
    pub capability_grant_id: String,
    pub reason: String,
    /// The `revoked_at` timestamp stamped onto the grant projection.
    pub revoked_at: String,
}

/// SG3: where a grant read-back decision came from.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GrantReadBackSource {
    /// A valid durable allow grant authorized the request without re-consulting
    /// the policy.
    DurableGrant,
    /// No valid grant existed (none, revoked, or expired), so the policy decided.
    Policy,
}

/// SG3: the typed outcome of a decide step that first consults the durable grant
/// store (read-back) and falls back to the policy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GrantReadBackDecision {
    /// `true` when the request is authorized (by a valid grant OR the policy).
    pub allowed: bool,
    /// Which authority decided.
    pub source: GrantReadBackSource,
    /// The capability grant id that authorized the request, when read-back hit a
    /// valid durable grant.
    pub authorizing_grant_id: Option<String>,
    /// The policy decision evaluated for this request. Always present so the
    /// audit trail records the policy's view even when a grant authorized.
    pub policy_decision: PermissionDecision,
}

impl FakeBoundaryController {
    /// SG3: decide a permission request with durable grant read-back.
    ///
    /// Read-back FIRST: if a valid (allow, not revoked, not expired) grant for the
    /// requested scope already exists in the durable store, the request is
    /// authorized by that grant and the policy is not the gate. A revoked or
    /// expired grant is treated as ABSENT, so it never authorizes -- read-back
    /// then falls through to the policy, which (for a stricter policy) denies.
    ///
    /// This is the SG3 contract: grants are not write-only (a valid grant
    /// authorizes a later request), and expiry/revocation are denial inputs (a
    /// revoked/expired grant does not authorize, even though its
    /// `capability.grant_created`/`grant_used` events remain on the log).
    pub fn decide_with_grant_read_back(
        &self,
        request: PermissionRequest,
    ) -> StateResult<GrantReadBackDecision> {
        let now = epoch_millis().to_string();
        let scope_json = request.scope_json.clone();
        // Read-back: an active allow grant for the SAME scope authorizes.
        let authorizing = self.active_allow_grant_for_scope(&scope_json, &now)?;
        // Always evaluate the policy so the decision record names the policy view.
        let policy_decision = self.permission_policy.decide(request);
        match authorizing {
            Some(grant) => Ok(GrantReadBackDecision {
                allowed: true,
                source: GrantReadBackSource::DurableGrant,
                authorizing_grant_id: Some(grant.capability_grant_id),
                policy_decision,
            }),
            None => Ok(GrantReadBackDecision {
                allowed: policy_decision.effect == "allow",
                source: GrantReadBackSource::Policy,
                authorizing_grant_id: None,
                policy_decision,
            }),
        }
    }

    /// SG3: the first active allow grant matching `scope_json`, or `None`.
    ///
    /// "Active" means an `allow` grant that is neither revoked nor past its
    /// `expires_at` at `now`. A revoked or expired grant -- even a matching one --
    /// is skipped, so it reads as absent. A `deny` grant is never an
    /// authorization here.
    pub fn active_allow_grant_for_scope(
        &self,
        scope_json: &str,
        now: &str,
    ) -> StateResult<Option<CapabilityGrantProjection>> {
        let grant = self
            .state
            .capability_grants()?
            .into_iter()
            .find(|grant| grant.scope_json == scope_json && grant.is_active_allow(now));
        Ok(grant)
    }

    /// SG3: revoke a durable grant by id, emitting `capability.grant_revoked`.
    ///
    /// Loads the existing grant, appends a `capability.grant_revoked` event
    /// carrying the revocation reason, and re-emits the grant projection with
    /// `revoked_at` stamped (the rest of the grant body is preserved verbatim).
    /// The old `capability.grant_created`/`capability.grant_used` events are left
    /// UNCHANGED on the log -- revocation is an additive event, not a rewrite --
    /// so a replay reconstructs the revoked state identically.
    ///
    /// Errors with [`StateError::MissingReadModel`] if no grant with that id
    /// exists.
    pub fn revoke_capability_grant(
        &self,
        scope: &GrantRevocationScope,
        capability_grant_id: &str,
        reason: &str,
    ) -> StateResult<GrantRevocation> {
        let existing = self
            .state
            .capability_grant_by_id(capability_grant_id)?
            .ok_or_else(|| missing_read_model("capability_grant", &capability_grant_id))?;
        let revoked_at = epoch_millis().to_string();

        let mut revoked = existing.clone();
        revoked.revoked_at = Some(revoked_at.clone());
        revoked.explanation = format!("revoked: {reason}");
        revoked.updated_sequence = 0;

        let payload = serde_json::json!({
            "capability_grant_id": capability_grant_id,
            "reason": reason,
            "revoked_at": revoked_at,
            "previous_effect": existing.effect,
            "scope_json": existing.scope_json,
        })
        .to_string();

        self.state.append_event(
            scoped_event(
                &format!(
                    "event-grant-revoked-{}-{}",
                    scope.session_id, capability_grant_id
                ),
                EventKind::CapabilityGrantRevoked,
                &self.project_id,
                &scope.task_id,
                &scope.agent_id,
                &scope.session_id,
                &scope.run_id,
            )
            .with_turn(scope.turn_id.to_string())
            .with_item(capability_grant_id.to_string())
            .with_payload(payload),
            &[ProjectionRecord::CapabilityGrant(revoked)],
        )?;

        Ok(GrantRevocation {
            capability_grant_id: capability_grant_id.to_string(),
            reason: reason.to_string(),
            revoked_at,
        })
    }
}
