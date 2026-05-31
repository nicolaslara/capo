//! SG2: the provider-neutral `AgentAdapter` permission round-trip + ACP option
//! mapping, against fake/scripted adapters.
//!
//! An ACP-shaped adapter (or any provider that supports interactive permission
//! prompts) raises an [`AdapterPermissionRequest`] carrying the ACP-native
//! `options` (`PermissionOption[]`, each with `optionId`/`name`/`kind`). The
//! controller decides it through `PermissionPolicy`, and the chosen outcome is
//! returned to the adapter as an [`AdapterPermissionResponse`] -- the ACP outcome
//! plus the SELECTED `optionId`. These are adapter-native types below the
//! `AgentAdapter` boundary (NOT `Fake*`-named structs), so a real ACP adapter in
//! the depth workpad reuses the identical request/response shape.
//!
//! This module owns only the option-MAPPING logic from `capability-permissions.md`
//! (the ACP option-mapping table, lines 383-397). It deliberately does NOT speak
//! JSON-RPC: building/parsing the live ACP `session/request_permission` wire frame
//! is explicitly out of scope and lands in the depth workpad. The fixture-only
//! verification standard (scripted options, asserted outcomes) is the SG2 bar.

/// The ACP permission-option kind, the adapter-native taxonomy ACP attaches to
/// each `PermissionOption`. Mirrors the ACP `optionKind` enum.
///
/// Capo does NOT adopt these as its policy model; it MAPS them into its own
/// decision/persistence vocabulary (see [`map_decision_to_acp_option`]) and
/// returns the chosen `optionId`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AcpPermissionOptionKind {
    /// `allow_once`: allow for the current request/turn only.
    AllowOnce,
    /// `allow_always`: allow and remember (durable) under the adapter.
    AllowAlways,
    /// `reject_once`: reject this request only.
    RejectOnce,
    /// `reject_always`: reject and remember the rejection (durable deny).
    RejectAlways,
}

impl AcpPermissionOptionKind {
    /// The ACP wire string for this option kind.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AllowOnce => "allow_once",
            Self::AllowAlways => "allow_always",
            Self::RejectOnce => "reject_once",
            Self::RejectAlways => "reject_always",
        }
    }

    /// Whether this option kind allows the request (vs rejects it).
    pub fn is_allow(&self) -> bool {
        matches!(self, Self::AllowOnce | Self::AllowAlways)
    }

    /// Parse an ACP option-kind string into the typed kind. Unknown kinds are
    /// `None` so an adapter cannot smuggle an un-mapped option past the policy.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "allow_once" => Some(Self::AllowOnce),
            "allow_always" => Some(Self::AllowAlways),
            "reject_once" => Some(Self::RejectOnce),
            "reject_always" => Some(Self::RejectAlways),
            _ => None,
        }
    }
}

/// One ACP `PermissionOption` as the adapter presents it: `optionId`, `name`,
/// and `kind`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcpPermissionOption {
    pub option_id: String,
    pub name: String,
    pub kind: AcpPermissionOptionKind,
}

impl AcpPermissionOption {
    pub fn new(
        option_id: impl Into<String>,
        name: impl Into<String>,
        kind: AcpPermissionOptionKind,
    ) -> Self {
        Self {
            option_id: option_id.into(),
            name: name.into(),
            kind,
        }
    }
}

/// A permission request raised by an adapter through the `AgentAdapter` boundary.
///
/// Provider-neutral by construction: a fake/scripted adapter, the ACP adapter in
/// depth, and any future interactive provider all raise this same shape. It
/// carries the requesting `tool_name`, the requested capability `scope`, the
/// `capability_profile_id` the session runs under, and the ACP-native `options`
/// the adapter is offering.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdapterPermissionRequest {
    /// The tool / action the adapter is asking permission for (e.g.
    /// `capo.file_write`). Display + audit only.
    pub tool_name: String,
    /// The requested capability scope string (`{domain}:{action}:{resource}`),
    /// the policy input.
    pub scope: String,
    /// The capability profile the session runs under.
    pub capability_profile_id: String,
    /// The ACP `PermissionOption[]` the adapter offered, in adapter order. May be
    /// empty -- an empty list is the "no selectable option" adapter-error case.
    pub options: Vec<AcpPermissionOption>,
}

impl AdapterPermissionRequest {
    pub fn new(
        tool_name: impl Into<String>,
        scope: impl Into<String>,
        capability_profile_id: impl Into<String>,
        options: Vec<AcpPermissionOption>,
    ) -> Self {
        Self {
            tool_name: tool_name.into(),
            scope: scope.into(),
            capability_profile_id: capability_profile_id.into(),
            options,
        }
    }

    /// The ACP option-id list, for persisting the offered `adapter_options` on
    /// the decision record.
    pub fn option_ids(&self) -> Vec<String> {
        self.options
            .iter()
            .map(|option| option.option_id.clone())
            .collect()
    }

    /// The first offered reject option (`reject_once` preferred over
    /// `reject_always`), if any.
    ///
    /// Used when a policy deny over-rules an offered allow option: rather than
    /// returning the allow option's id (which an ACP adapter would read as
    /// "permitted, proceed"), Capo returns a reject option's id when one was
    /// offered so the wire outcome matches the Capo deny.
    pub fn first_reject_option(&self) -> Option<&AcpPermissionOption> {
        self.options
            .iter()
            .find(|option| option.kind == AcpPermissionOptionKind::RejectOnce)
            .or_else(|| {
                self.options
                    .iter()
                    .find(|option| option.kind == AcpPermissionOptionKind::RejectAlways)
            })
    }
}

/// The ACP outcome Capo returns to the adapter for a permission request.
///
/// Mirrors the ACP `RequestPermissionOutcome`: either the adapter `selected` a
/// concrete `optionId`, or the request was `cancelled`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AcpPermissionOutcome {
    /// `selected`: the adapter is told which `optionId` Capo chose.
    Selected { option_id: String },
    /// `cancelled`: the prompt/permission request was canceled (operator cancel,
    /// or an adapter error with no selectable option).
    Cancelled,
}

impl AcpPermissionOutcome {
    /// The ACP outcome discriminator string (`selected` / `cancelled`).
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Selected { .. } => "selected",
            Self::Cancelled => "cancelled",
        }
    }

    /// The selected option id, when an option was chosen.
    pub fn option_id(&self) -> Option<&str> {
        match self {
            Self::Selected { option_id } => Some(option_id.as_str()),
            Self::Cancelled => None,
        }
    }
}

/// Why the round-trip resolved to a cancel (vs an option selection). Recorded so
/// the audit trail distinguishes an operator cancel from an adapter error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdapterPermissionCancelReason {
    /// The prompt turn / permission request was actually canceled.
    Cancelled,
    /// No selectable option existed -- treated as an adapter error per the design
    /// (`capability-permissions.md:396`): record `cancel`, fail the request.
    NoSelectableOption,
}

impl AdapterPermissionCancelReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Cancelled => "cancelled",
            Self::NoSelectableOption => "no_selectable_option",
        }
    }
}

/// The result of mapping a Capo `PermissionDecision`-shaped outcome onto an ACP
/// option, before the controller persists it. Pure data; the controller turns
/// this into the persisted records + the returned [`AdapterPermissionResponse`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcpOptionMapping {
    /// The chosen ACP option, or `None` when the round-trip cancels.
    pub selected: Option<AcpPermissionOption>,
    /// The ACP outcome to return to the adapter.
    pub outcome: AcpPermissionOutcome,
    /// The Capo decision the chosen option maps to (`allow` / `reject` /
    /// `cancel`).
    pub capo_decision: &'static str,
    /// The Capo grant persistence the chosen option downscopes to (the design's
    /// table column), e.g. `until_turn_end` / `until_session_end` /
    /// `until_revoked`. `None` on cancel.
    pub capo_persistence: Option<&'static str>,
    /// On a cancel outcome, why it canceled. `None` when an option was selected.
    pub cancel_reason: Option<AdapterPermissionCancelReason>,
}

/// Apply the `capability-permissions.md` ACP option-mapping table (lines
/// 383-397) to a set of offered ACP options under the TrustedLocal prototype
/// selection rules.
///
/// Selection rules (the prototype `AllowTrustedLocalProfilePolicy` mapping,
/// lines 390-396), evaluated against the offered `options`:
///
/// - Prefer the first `allow_once` option -> Capo `allow`, persistence
///   `until_turn_end`.
/// - If only `allow_always` exists (no `allow_once`), choose it but DOWNSCOPE
///   Capo persistence to `until_session_end` (TrustedLocal never makes a durable
///   remembered grant without explicit profile opt-in) -> Capo `allow`.
/// - If no allow option exists but a reject option exists, select the first
///   `reject_once` / `reject_always` option and record a Capo `reject` (a
///   `reject_always` maps to a durable `until_revoked` deny; a `reject_once` to a
///   transient `once` rejection that creates no grant).
/// - If no selectable option exists at all, return a `cancelled` outcome with
///   [`AdapterPermissionCancelReason::NoSelectableOption`] (adapter error).
///
/// `cancelled` (the operator-cancel path) is NOT produced here; the controller
/// drives that explicitly via [`AcpOptionMapping::cancelled`] when a prompt is
/// actually canceled.
pub fn map_acp_options_trusted_local(options: &[AcpPermissionOption]) -> AcpOptionMapping {
    // Prefer allow_once, then allow_always, then reject_once, then reject_always,
    // matching the design's documented preference order.
    let find = |kind: AcpPermissionOptionKind| options.iter().find(|option| option.kind == kind);

    if let Some(option) = find(AcpPermissionOptionKind::AllowOnce) {
        return AcpOptionMapping::selected(option.clone(), "allow", "until_turn_end");
    }
    if let Some(option) = find(AcpPermissionOptionKind::AllowAlways) {
        // TrustedLocal downscope: a remembered allow becomes session-scoped, not
        // durable `until_revoked`, unless the profile explicitly opts in.
        return AcpOptionMapping::selected(option.clone(), "allow", "until_session_end");
    }
    if let Some(option) = find(AcpPermissionOptionKind::RejectOnce) {
        // A transient rejection: Capo `reject`, persistence `once`, no grant.
        return AcpOptionMapping::selected(option.clone(), "reject", "once");
    }
    if let Some(option) = find(AcpPermissionOptionKind::RejectAlways) {
        // A durable deny: Capo `reject`, persistence `until_revoked`, scoped deny
        // grant.
        return AcpOptionMapping::selected(option.clone(), "reject", "until_revoked");
    }
    // No selectable option exists -> adapter error.
    AcpOptionMapping::no_selectable_option()
}

impl AcpOptionMapping {
    fn selected(
        option: AcpPermissionOption,
        capo_decision: &'static str,
        capo_persistence: &'static str,
    ) -> Self {
        Self {
            outcome: AcpPermissionOutcome::Selected {
                option_id: option.option_id.clone(),
            },
            selected: Some(option),
            capo_decision,
            capo_persistence: Some(capo_persistence),
            cancel_reason: None,
        }
    }

    /// The adapter-error mapping: no option could be selected. Records `cancel`
    /// and fails the adapter request rather than inventing an ACP outcome.
    pub fn no_selectable_option() -> Self {
        Self {
            selected: None,
            outcome: AcpPermissionOutcome::Cancelled,
            capo_decision: "cancel",
            capo_persistence: None,
            cancel_reason: Some(AdapterPermissionCancelReason::NoSelectableOption),
        }
    }

    /// The operator-cancel mapping: the prompt turn / permission request was
    /// actually canceled.
    pub fn cancelled() -> Self {
        Self {
            selected: None,
            outcome: AcpPermissionOutcome::Cancelled,
            capo_decision: "cancel",
            capo_persistence: None,
            cancel_reason: Some(AdapterPermissionCancelReason::Cancelled),
        }
    }

    /// `true` when the chosen option is an allow.
    pub fn is_allow(&self) -> bool {
        self.capo_decision == "allow"
    }

    /// `true` when the round-trip resolved to an ACP `cancelled` outcome.
    pub fn is_cancelled(&self) -> bool {
        matches!(self.outcome, AcpPermissionOutcome::Cancelled)
    }
}

/// The response the controller returns to the adapter to close one permission
/// round-trip: the ACP outcome (the selected `optionId` or `cancelled`), plus the
/// Capo-side audit identity (decision id + any created grant) the adapter can log
/// alongside its own record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdapterPermissionResponse {
    /// The ACP outcome returned to the adapter.
    pub outcome: AcpPermissionOutcome,
    /// The Capo decision this maps to (`allow` / `reject` / `cancel`).
    pub capo_decision: String,
    /// The Capo grant persistence the chosen option downscoped to, or `None` on
    /// cancel.
    pub capo_persistence: Option<String>,
    /// The Capo permission-decision id recorded for this round-trip.
    pub permission_decision_id: String,
    /// The Capo capability-grant id created (allow, or a durable `reject_always`
    /// deny), or `None` when no grant was materialized.
    pub capability_grant_id: Option<String>,
    /// `true` when the adapter request must be FAILED (the no-selectable-option
    /// adapter-error path); the adapter must not proceed.
    pub adapter_error: bool,
    /// `true` whenever the adapter MUST NOT proceed with the requested tool call:
    /// any Capo deny (including a policy deny over-ruling an offered allow
    /// option), any cancel, or the adapter-error path. This is the single,
    /// unambiguous "do not proceed" signal an ACP adapter consumes -- the raw
    /// `outcome` alone is not safe to read, because a `selected{optionId}` only
    /// means "proceed" when `must_not_proceed` is false.
    pub must_not_proceed: bool,
}

impl AdapterPermissionResponse {
    /// `true` when an allow option was selected (the adapter may proceed).
    pub fn allowed(&self) -> bool {
        self.capo_decision == "allow"
    }

    /// `true` when the adapter may proceed with the tool call: the policy
    /// allowed AND no halt signal is set. The inverse of [`Self::must_not_proceed`]
    /// for an allow.
    pub fn may_proceed(&self) -> bool {
        self.allowed() && !self.must_not_proceed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(kinds: &[AcpPermissionOptionKind]) -> Vec<AcpPermissionOption> {
        kinds
            .iter()
            .map(|kind| {
                AcpPermissionOption::new(format!("opt-{}", kind.as_str()), kind.as_str(), *kind)
            })
            .collect()
    }

    #[test]
    fn allow_once_maps_to_turn_scoped_allow() {
        let mapping = map_acp_options_trusted_local(&options(&[
            AcpPermissionOptionKind::AllowOnce,
            AcpPermissionOptionKind::AllowAlways,
            AcpPermissionOptionKind::RejectOnce,
        ]));
        assert!(mapping.is_allow());
        assert_eq!(mapping.capo_persistence, Some("until_turn_end"));
        assert_eq!(mapping.outcome.option_id(), Some("opt-allow_once"));
    }

    #[test]
    fn allow_always_alone_downscopes_to_session_end() {
        let mapping = map_acp_options_trusted_local(&options(&[
            AcpPermissionOptionKind::AllowAlways,
            AcpPermissionOptionKind::RejectAlways,
        ]));
        assert!(mapping.is_allow());
        // TrustedLocal downscope: never `until_revoked` without profile opt-in.
        assert_eq!(mapping.capo_persistence, Some("until_session_end"));
        assert_eq!(mapping.outcome.option_id(), Some("opt-allow_always"));
    }

    #[test]
    fn reject_once_maps_to_transient_reject_no_grant() {
        let mapping =
            map_acp_options_trusted_local(&options(&[AcpPermissionOptionKind::RejectOnce]));
        assert_eq!(mapping.capo_decision, "reject");
        assert_eq!(mapping.capo_persistence, Some("once"));
        assert_eq!(mapping.outcome.option_id(), Some("opt-reject_once"));
    }

    #[test]
    fn reject_always_maps_to_durable_deny() {
        let mapping =
            map_acp_options_trusted_local(&options(&[AcpPermissionOptionKind::RejectAlways]));
        assert_eq!(mapping.capo_decision, "reject");
        assert_eq!(mapping.capo_persistence, Some("until_revoked"));
        assert_eq!(mapping.outcome.option_id(), Some("opt-reject_always"));
    }

    #[test]
    fn no_options_is_adapter_error_cancel() {
        let mapping = map_acp_options_trusted_local(&[]);
        assert!(mapping.is_cancelled());
        assert_eq!(mapping.capo_decision, "cancel");
        assert_eq!(
            mapping.cancel_reason,
            Some(AdapterPermissionCancelReason::NoSelectableOption)
        );
        assert_eq!(mapping.outcome, AcpPermissionOutcome::Cancelled);
    }

    #[test]
    fn cancelled_is_operator_cancel() {
        let mapping = AcpOptionMapping::cancelled();
        assert!(mapping.is_cancelled());
        assert_eq!(mapping.capo_decision, "cancel");
        assert_eq!(
            mapping.cancel_reason,
            Some(AdapterPermissionCancelReason::Cancelled)
        );
    }
}
