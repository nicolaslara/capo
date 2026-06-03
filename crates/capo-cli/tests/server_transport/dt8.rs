//! DT8: doc-vs-CLI consistency check for the operator distributed-run guide.
//!
//! The DT8 acceptance criteria require an operator doc that covers the DT1 CLI
//! surface, the DT2 keep-alive planes, the DT5 audit/revoke lifecycle, the DT6
//! all-local default, and the DT7 deterministic + live gates. Its verification is
//! "a doc-vs-CLI consistency check ... internally consistent with the shipped
//! flags". Docs drift silently, so this is a TEST, not attestation: it asserts
//! that every CLI command the operator doc instructs an operator to run is a
//! command the CLI actually DISPATCHES (`crates/capo-cli/src/main.rs`), and that
//! the doc names the load-bearing role/connectivity/safety/gate surfaces. If the
//! CLI surface changes without the doc following (or vice versa), this fails.

use std::path::PathBuf;

/// Read a repo-root-relative file via `CARGO_MANIFEST_DIR` (the `capo-cli` crate
/// dir), so the test does not depend on the process working directory.
fn read_repo_file(relative_from_repo_root: &str) -> String {
    // CARGO_MANIFEST_DIR = <repo>/crates/capo-cli
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("repo root is two levels above the capo-cli manifest dir");
    let path = repo_root.join(relative_from_repo_root);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()))
}

fn operator_doc() -> String {
    read_repo_file("workpads/distributed-topology/operator-distributed-run.md")
}

fn cli_dispatch() -> String {
    read_repo_file("crates/capo-cli/src/main.rs")
}

/// Normalize whitespace so a match-guard substring assertion is robust to
/// formatting (rustfmt may wrap a long guard across lines). We collapse every run of
/// ASCII whitespace to a single space, so `area == "connectivity"\n    && command`
/// and `area == "connectivity" && command` both contain the same normalized guard.
fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Every command the operator doc tells an operator to run MUST be dispatched by
/// the CLI. We assert the command exists in the doc (so the doc covers it) AND that
/// a SPECIFIC match-arm GUARD routes it in main.rs (so the doc is not lying about a
/// command the CLI cannot run).
///
/// Resolving review finding 5: the check no longer uses a coarse bare-literal
/// substring (`dispatch.contains("\"connectivity\"")`), which would pass on a mere
/// comment mentioning the word. It asserts the ACTUAL guard structure the dispatch
/// uses (e.g. `area == "connectivity" && command == "expose-stub"`), so a renamed or
/// removed dispatch path is caught even if a stale comment still names the string.
#[test]
fn operator_doc_commands_are_all_dispatched_by_the_cli() {
    let doc = operator_doc();
    // Normalize so a guard rustfmt wrapped across lines still matches as a contiguous
    // substring.
    let dispatch = collapse_whitespace(&cli_dispatch());

    // (doc-substring, the EXACT match-arm guard fragment that proves the command
    // routes). Each fragment is the real `if area == ... && command == ...` guard in
    // `main.rs`, not a bare quoted literal.
    let commands: &[(&str, &str)] = &[
        (
            "capo role server",
            r#"area == "role" && command == "server""#,
        ),
        (
            "capo role runner",
            r#"area == "role" && command == "runner""#,
        ),
        (
            "capo role client",
            r#"area == "role" && command == "client""#,
        ),
        (
            "capo server serve",
            r#"area == "server" && command == "serve""#,
        ),
        ("capo control", r#"command == "control""#),
        (
            "capo connectivity expose-stub",
            r#"area == "connectivity" && command == "expose-stub""#,
        ),
        (
            "capo connectivity request-approval",
            r#"area == "connectivity" && command == "request-approval""#,
        ),
        (
            "capo connectivity activate-exposure",
            r#"area == "connectivity" && command == "activate-exposure""#,
        ),
        (
            "capo connectivity revoke-exposure",
            r#"area == "connectivity" && command == "revoke-exposure""#,
        ),
        (
            "capo connectivity exposure-status",
            r#"area == "connectivity" && command == "exposure-status""#,
        ),
        (
            "capo permission decide",
            r#"area == "permission" && command == "decide""#,
        ),
    ];

    for (doc_command, guard) in commands {
        assert!(
            doc.contains(doc_command),
            "operator doc must document the `{doc_command}` command"
        );
        let normalized_guard = collapse_whitespace(guard);
        assert!(
            dispatch.contains(&normalized_guard),
            "CLI dispatch (main.rs) must route `{doc_command}` via the guard `{guard}` \
             (a comment mentioning the string is not enough)"
        );
    }
}

/// The doc must reference the actual shipped FLAGS for the role surface, not
/// invented ones. These are the flags `role_config.rs` / the connectivity handlers
/// actually parse.
#[test]
fn operator_doc_references_shipped_flags() {
    let doc = operator_doc();
    let shipped_flags = [
        "--server-addr",
        "--server-endpoint",
        "--runner-endpoint",
        "--endpoint",
        "--connect",
        "--exposure",
        "--target",
        "--name",
        "--runner",
        "--workspace",
        "--artifacts",
        "--owner-kind",
        "--owner-id",
        "--channel",
        "--approval",
        "--decision",
        "--latest",
    ];
    for flag in shipped_flags {
        assert!(
            doc.contains(flag),
            "operator doc must reference the shipped flag `{flag}`"
        );
    }
}

/// The doc must state the DT2/DT5/DT6/DT7 load-bearing concepts in operator terms:
/// the two health planes, the privileged-connector / handle / transport-confidentiality
/// safety posture, the tailnet-ACL review, the all-local default, and the live gate.
#[test]
fn operator_doc_states_the_safety_and_gate_posture() {
    let doc = operator_doc();
    let required_phrases = [
        // DT2: two planes, which is logged.
        "blocked_pending_permission",
        "health_changed",
        "LOGGED",
        "EPHEMERAL",
        // DT5 / safety posture.
        "PRIVILEGED CONNECTOR",
        "auth_ref",
        "identity_ref",
        "never logged",
        "TRANSPORT property",
        "SSH / Tailscale encryption",
        "runner-side redaction",
        "Tailnet ACLs",
        // DT6 default.
        "All-local is the DEFAULT",
        "ST9 contract wire snapshots",
        // DT7 gates.
        "CAPO_SERVER_RUN_DISTRIBUTED_LIVE",
        "cargo test --workspace",
        "#[ignore]",
    ];
    for phrase in required_phrases {
        assert!(
            doc.contains(phrase),
            "operator doc must state the posture phrase: `{phrase}`"
        );
    }
}
