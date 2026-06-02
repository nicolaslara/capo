//! CT2: the connectivity REDACTION GUARD — the SECONDARY net behind the
//! architectural confinement of credentials.
//!
//! The PRIMARY never-logged guarantee is architectural: an `auth_ref` /
//! `identity_ref` is an opaque HANDLE, and resolution to a real credential is
//! confined to the tunnel adapter at connect time — no controller-facing type
//! carries the resolved secret. This module is defense-in-depth, NOT the proof of
//! "never logged" (the handle fields are `Option<String>`, so the compiler cannot
//! prevent a raw value being placed in one — which is exactly why this guard
//! exists).
//!
//! The guard enforces PER-FIELD rules (not a single "or"):
//!
//! - A credential-pattern match in a HANDLE field (`auth_ref` / `identity_ref` /
//!   `identity_fingerprint`) is a BUG and FAILS CLOSED — [`scan_handle_field`]
//!   returns an error and the caller must REFUSE TO PERSIST. A raw value in a
//!   handle field must never be silently scrubbed, because silent scrubbing can
//!   mask a real programming error that was about to log a token.
//! - A credential-pattern match in a FREE-TEXT payload field is SCRUBBED
//!   ([`scrub_free_text`]), because redaction is the documented behavior there.
//!
//! This guard ADDS the enforcement behind the existing `RedactionState::Safe`
//! marker on `connectivity.*` events, which today is an UNVERIFIED assertion
//! (nothing scans the events). [`assert_connectivity_event_safe`] is what makes
//! the marker mean something.
//!
//! IMPORTANT (do not over-claim): the planted-pattern detector proves that the
//! KNOWN credential shapes are caught — it does NOT prove an arbitrary credential
//! is universally caught. The universal guarantee rests on the architectural
//! confinement above, never on this regex-free pattern net.

/// Marker substituted for a scrubbed credential in a free-text payload field.
pub const CONNECTIVITY_REDACTION_MARKER: &str = "[redacted-credential]";

/// A handle field that failed the CT2 fail-closed guard: a raw-credential-looking
/// value was found where only an opaque handle/fingerprint is permitted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HandleRedactionError {
    /// The handle field name (`auth_ref` / `identity_ref` / `identity_fingerprint`).
    pub field: String,
    /// The name of the credential pattern that matched (NEVER the matched value).
    pub matched_pattern: String,
}

impl std::fmt::Display for HandleRedactionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "connectivity handle field `{}` contains a raw-credential-looking value \
             (matched the `{}` pattern); refusing to persist — a handle field must \
             carry an opaque reference, never a raw credential",
            self.field, self.matched_pattern
        )
    }
}

impl std::error::Error for HandleRedactionError {}

/// A single credential-pattern entry: `(pattern_name, predicate)`. The predicate
/// inspects a candidate value and returns whether it matches the named shape.
type CredentialPattern = (&'static str, fn(&str) -> bool);

/// The known credential SHAPES the guard recognizes. These cover the planted
/// patterns used in the CT2 tests (tailscale authkeys, bearer-ish tokens, session
/// cookies); the list is the defense-in-depth net, not the universal guarantee.
fn credential_patterns() -> &'static [CredentialPattern] {
    &[
        // Tailscale authkeys: `tskey-auth-...`, `tskey-...`.
        ("tailscale_authkey", |value| {
            let lower = value.to_ascii_lowercase();
            lower.contains("tskey-auth-") || lower.contains("tskey-client-") || {
                // `tskey-<base>` with a long opaque tail.
                if let Some(rest) = lower.strip_prefix("tskey-") {
                    rest.len() >= 8
                } else {
                    false
                }
            }
        }),
        // OAuth/bearer style tokens explicitly labelled.
        ("bearer_token", |value| {
            let lower = value.to_ascii_lowercase();
            lower.starts_with("bearer ") || lower.contains("authorization: bearer")
        }),
        // Common secret markers (GitHub, generic `sk-`, AWS, Slack). Matched with
        // `contains` (not `starts_with`) so a token embedded in free text — e.g. a
        // `--reason` like "close: ghp_0123... was used" — is caught, consistent with
        // the tailscale `contains` pattern above (defense-in-depth, not a guarantee).
        ("api_token_prefix", |value| {
            let lower = value.to_ascii_lowercase();
            ["ghp_", "github_pat_", "sk-", "akia", "xoxb-", "xoxp-"]
                .iter()
                .any(|marker| lower.contains(marker))
        }),
        // Session-cookie style `name=<long-opaque>` for known auth cookie names.
        ("session_cookie", |value| {
            let lower = value.to_ascii_lowercase();
            ["session=", "sessionid=", "auth=", "token=", "cookie:"]
                .iter()
                .any(|needle| lower.contains(needle))
        }),
    ]
}

/// Return the name of the first credential pattern that matches `value`, if any.
/// Never returns the matched value — only the pattern label, so the result itself
/// is safe to log.
pub fn matched_credential_pattern(value: &str) -> Option<&'static str> {
    credential_patterns()
        .iter()
        .find(|(_, predicate)| predicate(value))
        .map(|(name, _)| *name)
}

/// CT2 HANDLE-FIELD rule: a credential pattern in a handle field is a BUG and
/// FAILS CLOSED. Returns `Ok(())` when the (optional) handle is absent or carries
/// only an opaque reference; returns [`HandleRedactionError`] when it carries a
/// raw-credential-looking value, so the caller must refuse to persist.
pub fn scan_handle_field(field: &str, value: Option<&str>) -> Result<(), HandleRedactionError> {
    let Some(value) = value else {
        return Ok(());
    };
    if let Some(pattern) = matched_credential_pattern(value) {
        return Err(HandleRedactionError {
            field: field.to_string(),
            matched_pattern: pattern.to_string(),
        });
    }
    Ok(())
}

/// CT2 FREE-TEXT rule: scrub any recognized credential pattern out of a free-text
/// payload field, replacing the whole value with [`CONNECTIVITY_REDACTION_MARKER`]
/// when a pattern is present. Returns the (possibly scrubbed) value plus whether a
/// scrub occurred. Reserved ONLY for fields where redaction is the documented
/// behavior — never for handle fields (those fail closed via [`scan_handle_field`]).
pub fn scrub_free_text(value: &str) -> (String, bool) {
    if matched_credential_pattern(value).is_some() {
        (CONNECTIVITY_REDACTION_MARKER.to_string(), true)
    } else {
        (value.to_string(), false)
    }
}

/// The connectivity HANDLE bundle scanned before any persistence/emission. All
/// three are handle/fingerprint fields, so all three FAIL CLOSED on a credential
/// pattern (no scrubbing).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ConnectivityHandles<'a> {
    pub auth_ref: Option<&'a str>,
    pub identity_ref: Option<&'a str>,
    pub identity_fingerprint: Option<&'a str>,
}

/// Scan every connectivity HANDLE field and FAIL CLOSED on the first
/// credential-pattern match. The single entry point a CLI/codec/projection path
/// calls before emission so the `RedactionState::Safe` marker is earned, not
/// assumed.
pub fn guard_connectivity_handles(
    handles: &ConnectivityHandles<'_>,
) -> Result<(), HandleRedactionError> {
    scan_handle_field("auth_ref", handles.auth_ref)?;
    scan_handle_field("identity_ref", handles.identity_ref)?;
    scan_handle_field("identity_fingerprint", handles.identity_fingerprint)?;
    Ok(())
}

/// CT2 emitted-surface assertion: scan a SAFE-marked connectivity event payload
/// for any leaked credential pattern. Because a `connectivity.*` event payload is
/// a free-text/handle blend that is ALREADY supposed to be secret-free (handles
/// failed closed upstream; free text was scrubbed), the presence of a credential
/// pattern here means the marker is a lie. Returns the matched pattern name (never
/// the value) so the caller can fail the emission. `None` means the payload is
/// clean and the `Safe` marker is honest.
pub fn scan_emitted_surface(payload: &str) -> Option<&'static str> {
    matched_credential_pattern(payload)
}

/// Convenience: assert a `connectivity.*` payload marked `Safe` is genuinely
/// secret-free. Returns `Ok(())` if clean, else the pattern name that leaked.
pub fn assert_connectivity_event_safe(payload: &str) -> Result<(), &'static str> {
    match scan_emitted_surface(payload) {
        Some(pattern) => Err(pattern),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handle_field_fails_closed_on_planted_tailscale_authkey() {
        let planted = "tskey-auth-DEADBEEFCAFEBABE1234567890";
        let error = scan_handle_field("auth_ref", Some(planted))
            .expect_err("a planted authkey in a handle field must FAIL CLOSED, not be scrubbed");
        assert_eq!(error.field, "auth_ref");
        assert_eq!(error.matched_pattern, "tailscale_authkey");
        // The error text records the PATTERN NAME, never the planted value.
        assert!(!format!("{error}").contains("DEADBEEF"));
    }

    #[test]
    fn handle_field_passes_for_opaque_reference() {
        // These are the legitimate opaque handle/fingerprint shapes CT2 expects.
        assert!(scan_handle_field("auth_ref", Some("keychain:capo/tailnet-authkey")).is_ok());
        assert!(scan_handle_field("identity_ref", Some("tailscale:device:n7Qk2cFf")).is_ok());
        assert!(scan_handle_field("identity_fingerprint", Some("sha256:9f86d081884c7d65")).is_ok());
        assert!(scan_handle_field("auth_ref", None).is_ok());
        assert!(scan_handle_field("auth_ref", Some("")).is_ok());
    }

    #[test]
    fn free_text_field_is_scrubbed_not_failed() {
        let (scrubbed, did_scrub) =
            scrub_free_text("connect failed: tskey-auth-DEADBEEFCAFE12345678");
        assert!(did_scrub);
        assert_eq!(scrubbed, CONNECTIVITY_REDACTION_MARKER);
        assert!(!scrubbed.contains("DEADBEEF"));

        let (kept, did_scrub) = scrub_free_text("connect failed: host unreachable");
        assert!(!did_scrub);
        assert_eq!(kept, "connect failed: host unreachable");
    }

    #[test]
    fn known_credential_shapes_are_detected_by_pattern_name() {
        assert_eq!(
            matched_credential_pattern("tskey-auth-XXXXXXXXXXXX"),
            Some("tailscale_authkey")
        );
        assert_eq!(
            matched_credential_pattern("Bearer abc.def.ghi"),
            Some("bearer_token")
        );
        assert_eq!(
            matched_credential_pattern("ghp_0123456789abcdef"),
            Some("api_token_prefix")
        );
        // A token EMBEDDED in free text is caught (the `contains` semantics, not
        // `starts_with`): a `--reason` value carrying a leaked PAT mid-string fails.
        assert_eq!(
            matched_credential_pattern("close: ghp_0123456789abcdef was used"),
            Some("api_token_prefix")
        );
        assert_eq!(
            matched_credential_pattern("session=0123456789abcdef"),
            Some("session_cookie")
        );
        // An opaque handle is NOT a credential pattern.
        assert_eq!(
            matched_credential_pattern("keychain:capo/tailnet-authkey"),
            None
        );
    }

    #[test]
    fn guard_bundle_fails_closed_on_any_handle() {
        let bundle = ConnectivityHandles {
            auth_ref: Some("keychain:capo/authkey"),
            identity_ref: Some("tskey-auth-LEAKEDLEAKEDLEAKED12"),
            identity_fingerprint: None,
        };
        let error = guard_connectivity_handles(&bundle)
            .expect_err("a leaked authkey in identity_ref must fail the whole bundle");
        assert_eq!(error.field, "identity_ref");
    }

    #[test]
    fn emitted_surface_assertion_catches_a_leak_and_passes_clean() {
        assert_eq!(
            assert_connectivity_event_safe(
                r#"{"endpoint":"cap","auth_mode":"tailscale_authkey_handle"}"#
            ),
            Ok(())
        );
        assert_eq!(
            assert_connectivity_event_safe(r#"{"blob":"tskey-auth-DEADBEEF12345678"}"#),
            Err("tailscale_authkey")
        );
    }
}
