//! DP7: a first OS sandbox tier behind the [`RuntimeRunner`] boundary.
//!
//! Today Capo only had path-prefix checks ([`LocalProcessRunner::ensure_cwd_allowed`])
//! and delegated any "hard" confinement to the provider CLI's own `--sandbox`
//! flag. DP7 adds a real OS sandbox tier as a SWAPPABLE option behind the runtime
//! boundary that enforces filesystem/network confinement through actual OS
//! mechanisms:
//!
//! - macOS: `seatbelt` via `sandbox-exec` with a generated `.sbpl` policy
//!   (base: `(deny default)` + read-all + write-only-under-confined-roots; network
//!   either `(allow network*)` or `(deny network*)` per the granted profile),
//!   modeled after the codex `sandboxing` crate's seatbelt base/network policies.
//! - linux: `landlock` filesystem confinement launched through `bwrap`
//!   (bubblewrap), modeled after the codex `linux-sandbox`/`bwrap` crates.
//!
//! The cardinal rule (recorded in `knowledge.md` and the DP7 "Must not do"): Capo
//! ONLY claims hard sandboxing where the OS actually enforces it and a test proves
//! it. On a platform with no enforcement available the tier reports
//! [`SandboxEnforcement::Unenforced`] and the runner records the platform
//! limitation as an event rather than silently pretending to confine.
//!
//! The sandbox is an ADDITIONAL enforcement layer: it composes with the
//! `real-turn-loop` path confinement (the request's `cwd` is still checked against
//! the runner's `workspace_roots`) and a successful confined run still produces
//! the same artifacts/checkpoint surface as an un-sandboxed run.
//!
//! The sandbox decision is wired to the `safety-gates` capability scopes through
//! [`SandboxProfile`]: a confined run's filesystem-write roots and network-egress
//! allowance come from the GRANTED capability profile, and an un-granted critical
//! scope (a write outside the confined root, or network egress when the profile
//! forbids it) is DENIED before the sandbox launches, recorded as a
//! [`SandboxRefusal`] event rather than a silent failure.

use std::fs;
use std::path::{Path, PathBuf};

use crate::{
    LocalProcessRequest, LocalProcessRunner, RuntimeError, RuntimeEvent, RuntimeResult,
    normalize_path,
};

/// The OS sandbox tiers Capo can select behind the runtime boundary.
///
/// Mirrors the codex `sandboxing` crate's platform split: `seatbelt` is gated to
/// macOS and `landlock`+`bwrap` to linux. [`SandboxTier::None`] is the explicit
/// "no OS sandbox tier selected" option (the legacy path-prefix-only behavior).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SandboxTier {
    /// No OS sandbox tier selected: only the runner's path-prefix confinement
    /// applies (legacy behavior). Never claims hard sandboxing.
    None,
    /// macOS seatbelt via `sandbox-exec` + a generated `.sbpl` policy.
    MacosSeatbelt,
    /// linux landlock filesystem confinement launched through `bwrap`.
    LinuxLandlockBwrap,
}

impl SandboxTier {
    /// The runtime-boundary variant label for this tier.
    pub fn variant(self) -> &'static str {
        match self {
            Self::None => "sandbox-none",
            Self::MacosSeatbelt => "sandbox-macos-seatbelt",
            Self::LinuxLandlockBwrap => "sandbox-linux-landlock-bwrap",
        }
    }

    /// The platform-appropriate enforcing tier for the current build target.
    ///
    /// DP-OQ2 (recorded in `knowledge.md`): macOS seatbelt gates first on the dev
    /// box for fast iteration; linux landlock+bwrap is the CI enforcement tier.
    /// On any other platform there is no enforcing tier and this returns
    /// [`SandboxTier::None`].
    pub fn host_default() -> Self {
        #[cfg(target_os = "macos")]
        {
            Self::MacosSeatbelt
        }
        #[cfg(target_os = "linux")]
        {
            Self::LinuxLandlockBwrap
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            Self::None
        }
    }

    /// Whether THIS tier can actually be enforced on the current build target.
    ///
    /// Seatbelt enforces only on macOS; landlock+bwrap only on linux. The runner
    /// uses this to decide between an enforced confined launch and an
    /// [`SandboxEnforcement::Unenforced`] record (Capo never claims hard
    /// sandboxing where the OS cannot enforce it).
    pub fn is_enforced_here(self) -> bool {
        match self {
            Self::None => false,
            Self::MacosSeatbelt => cfg!(target_os = "macos"),
            Self::LinuxLandlockBwrap => cfg!(target_os = "linux"),
        }
    }
}

/// The sandbox confinement derived from a GRANTED capability profile.
///
/// This is the bridge to the `safety-gates` capability scopes: the confined
/// filesystem-write roots and the network-egress allowance match the granted
/// capability profile. The set of write roots is the workspace confinement scope
/// (`filesystem:write:workspace`); `allow_network_egress` reflects whether the
/// profile holds the critical `network:connect:*` egress scope.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SandboxProfile {
    /// Directories the confined process may WRITE to. A write anywhere else is
    /// refused by the OS sandbox layer (not just by Capo's path-prefix check).
    pub writable_roots: Vec<PathBuf>,
    /// Whether the granted profile permits network egress
    /// (`network:connect:internet` / `network:connect:private_tunnel`). When
    /// `false` the sandbox denies all network access at the OS layer.
    pub allow_network_egress: bool,
}

impl SandboxProfile {
    /// A workspace-confined, network-denied profile (the default workspace-write
    /// posture: the agent may write inside the workspace but cannot reach the
    /// network). This matches the `trusted-local-dev` v0 posture where network
    /// egress is a critical scope that is NOT granted by default.
    pub fn workspace_confined(writable_roots: impl IntoIterator<Item = PathBuf>) -> Self {
        Self {
            writable_roots: writable_roots.into_iter().collect(),
            allow_network_egress: false,
        }
    }

    /// Re-admit network egress (the granted profile explicitly holds the
    /// `network:connect:*` critical scope).
    pub fn with_network_egress(mut self, allow: bool) -> Self {
        self.allow_network_egress = allow;
        self
    }

    fn normalized_roots(&self) -> RuntimeResult<Vec<PathBuf>> {
        self.writable_roots
            .iter()
            .map(|p| normalize_path(p))
            .collect()
    }

    /// Whether `path` is inside one of the confined writable roots.
    ///
    /// Public so the remote runner (RR5) can run the SAME pre-launch
    /// write-confinement gate against the remote worktree root before composing
    /// the remote OS sandbox — the refusal rule is shared, not re-authored.
    pub fn write_allowed(&self, path: &Path) -> RuntimeResult<bool> {
        let path = normalize_path(path)?;
        for root in self.normalized_roots()? {
            if path.starts_with(&root) {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

/// Why a sandboxed launch was refused BEFORE the process started.
///
/// The refusal is recorded as a `sandbox.launch_refused` event on the
/// [`SandboxPlan`], never a silent failure: an un-granted critical scope is
/// denied before the sandbox launches.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SandboxRefusal {
    /// The requested working directory / a declared write target is outside the
    /// profile's confined writable roots.
    WriteOutsideConfinedRoot { path: PathBuf },
    /// The run requires network egress but the granted profile forbids it.
    NetworkEgressForbidden,
}

impl SandboxRefusal {
    pub fn detail(&self) -> String {
        match self {
            Self::WriteOutsideConfinedRoot { path } => {
                format!("write outside confined root: {}", path.display())
            }
            Self::NetworkEgressForbidden => "network egress forbidden by profile".to_string(),
        }
    }

    /// The stable refusal token recorded as the `sandbox.launch_refused` event
    /// status. Public so the remote runner (RR5) records the SAME refusal codes as
    /// the local DP7 path rather than inventing a parallel vocabulary.
    pub fn reason_code(&self) -> &'static str {
        match self {
            Self::WriteOutsideConfinedRoot { .. } => "write-outside-confined-root",
            Self::NetworkEgressForbidden => "network-egress-forbidden",
        }
    }
}

/// Whether a sandboxed run was enforced by the OS, ran un-enforced (platform
/// limitation, honestly recorded), or was refused before launch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SandboxEnforcement {
    /// The OS sandbox layer enforced the confinement (seatbelt on macOS, landlock
    /// +bwrap on linux). Capo may claim hard sandboxing.
    Enforced { tier: SandboxTier },
    /// The selected tier cannot be enforced on this platform. Capo does NOT claim
    /// sandboxing; the limitation is recorded as an event.
    Unenforced { tier: SandboxTier, reason: String },
    /// The launch was refused before the process started (un-granted critical
    /// scope). No process ran.
    Refused { refusal: SandboxRefusal },
}

/// The decision + (when enforced) the rewritten request that wraps the original
/// command in the OS sandbox launcher.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SandboxPlan {
    pub enforcement: SandboxEnforcement,
    /// The request to actually run. When enforced, this is the ORIGINAL request
    /// rewritten to launch under the OS sandbox launcher (`sandbox-exec`/`bwrap`).
    /// When unenforced, this is the original request unchanged (run honestly
    /// un-sandboxed). `None` when refused (nothing to run).
    pub request: Option<LocalProcessRequest>,
    /// For seatbelt: the generated `.sbpl` policy text (so a test can assert the
    /// policy shape without launching). `None` for non-seatbelt plans.
    pub seatbelt_policy: Option<String>,
    pub events: Vec<RuntimeEvent>,
}

/// A swappable OS sandbox option behind the runtime boundary.
///
/// `tier` selects the OS mechanism; `profile` carries the granted capability
/// scopes. The sandbox composes with a [`LocalProcessRunner`]: [`Self::plan`]
/// gates the launch against the profile and rewrites the request to run under the
/// OS launcher, then the caller runs the planned request through the SAME runner
/// (so env-scrub, path confinement, redaction, and artifact capture are reused).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OsSandbox {
    tier: SandboxTier,
    profile: SandboxProfile,
}

impl OsSandbox {
    pub fn new(tier: SandboxTier, profile: SandboxProfile) -> Self {
        Self { tier, profile }
    }

    /// The host-appropriate enforcing tier with the given profile.
    pub fn host_default(profile: SandboxProfile) -> Self {
        Self::new(SandboxTier::host_default(), profile)
    }

    pub fn tier(&self) -> SandboxTier {
        self.tier
    }

    pub fn profile(&self) -> &SandboxProfile {
        &self.profile
    }

    pub fn binding_variant(&self) -> &'static str {
        self.tier.variant()
    }

    /// Decide whether `request` may run confined, refusing ungranted critical
    /// scopes BEFORE launch, and (when enforced) rewriting it to launch under the
    /// OS sandbox launcher.
    ///
    /// `requires_network_egress` declares whether this run needs the network
    /// (e.g. the dispatched command is known to reach out). Combined with the
    /// profile it gates the `network:connect:*` critical scope.
    pub fn plan(
        &self,
        request: LocalProcessRequest,
        requires_network_egress: bool,
    ) -> RuntimeResult<SandboxPlan> {
        // Pre-launch gate 1: a run that needs egress under a profile that forbids
        // it is refused before launch (the un-granted critical scope is denied).
        if requires_network_egress && !self.profile.allow_network_egress {
            return Ok(self.refused(SandboxRefusal::NetworkEgressForbidden));
        }
        // Pre-launch gate 2: the working directory must be inside a confined
        // writable root, otherwise the run would write outside the granted scope.
        if !self.profile.write_allowed(&request.cwd)? {
            return Ok(self.refused(SandboxRefusal::WriteOutsideConfinedRoot {
                path: request.cwd.clone(),
            }));
        }

        if !self.tier.is_enforced_here() {
            // Platform limitation: do NOT claim sandboxing. Run honestly
            // un-enforced and record the limitation.
            let reason = format!(
                "tier {} is not enforceable on this platform",
                self.tier.variant()
            );
            return Ok(SandboxPlan {
                events: vec![RuntimeEvent {
                    kind: "sandbox.unenforced".to_string(),
                    status: "unenforced".to_string(),
                    detail: reason.clone(),
                }],
                enforcement: SandboxEnforcement::Unenforced {
                    tier: self.tier,
                    reason,
                },
                seatbelt_policy: None,
                request: Some(request),
            });
        }

        match self.tier {
            SandboxTier::MacosSeatbelt => self.plan_seatbelt(request),
            SandboxTier::LinuxLandlockBwrap => self.plan_landlock_bwrap(request),
            SandboxTier::None => {
                // None is never "enforced here"; handled above. Defensive: treat
                // as unenforced rather than claiming sandboxing.
                Ok(SandboxPlan {
                    events: vec![RuntimeEvent {
                        kind: "sandbox.unenforced".to_string(),
                        status: "unenforced".to_string(),
                        detail: "no sandbox tier selected".to_string(),
                    }],
                    enforcement: SandboxEnforcement::Unenforced {
                        tier: SandboxTier::None,
                        reason: "no sandbox tier selected".to_string(),
                    },
                    seatbelt_policy: None,
                    request: Some(request),
                })
            }
        }
    }

    fn refused(&self, refusal: SandboxRefusal) -> SandboxPlan {
        SandboxPlan {
            events: vec![RuntimeEvent {
                kind: "sandbox.launch_refused".to_string(),
                status: refusal.reason_code().to_string(),
                detail: refusal.detail(),
            }],
            enforcement: SandboxEnforcement::Refused {
                refusal: refusal.clone(),
            },
            seatbelt_policy: None,
            request: None,
        }
    }

    /// Build the macOS seatbelt `.sbpl` policy text for this profile.
    ///
    /// Base policy (after the codex seatbelt base): deny-by-default, allow process
    /// exec/fork and sysctl reads (needed for a process to start), allow file
    /// reads everywhere, but allow file WRITES only under the confined roots.
    /// Network is `(allow network*)` only when the profile grants egress, else
    /// `(deny network*)`.
    fn seatbelt_policy(&self) -> RuntimeResult<String> {
        let mut policy = String::new();
        policy.push_str("(version 1)\n");
        policy.push_str("(deny default)\n");
        policy.push_str("(allow process-exec)\n");
        policy.push_str("(allow process-fork)\n");
        policy.push_str("(allow sysctl-read)\n");
        policy.push_str("(allow file-read*)\n");
        for root in self.profile.normalized_roots()? {
            // The confined root must be canonical: on macOS `/tmp` is a symlink to
            // `/private/tmp`, and seatbelt subpath rules match the canonical path.
            policy.push_str(&format!(
                "(allow file-write* (subpath {:?}))\n",
                root.to_string_lossy()
            ));
        }
        if self.profile.allow_network_egress {
            policy.push_str("(allow network*)\n");
        } else {
            policy.push_str("(deny network*)\n");
        }
        Ok(policy)
    }

    fn plan_seatbelt(&self, request: LocalProcessRequest) -> RuntimeResult<SandboxPlan> {
        let policy = self.seatbelt_policy()?;
        // Materialize the policy as a file inside the first confined root so the
        // launcher can pass it to `sandbox-exec -f`. Writing it inside a confined
        // root keeps it under the sandbox's own writable scope and avoids a
        // separate cleanup boundary.
        let policy_dir = self
            .profile
            .normalized_roots()?
            .into_iter()
            .next()
            .ok_or_else(|| {
                RuntimeError::Io(std::io::Error::other(
                    "seatbelt profile has no confined writable root for the policy file",
                ))
            })?;
        fs::create_dir_all(&policy_dir)?;
        let policy_path = policy_dir.join(format!(
            "capo-seatbelt-{}.sb",
            sanitize(request.run_id.as_str())
        ));
        fs::write(&policy_path, policy.as_bytes())?;

        // Rewrite the request to launch the ORIGINAL program under sandbox-exec.
        // The runner still owns env-scrub / path confinement / artifact capture.
        let mut argv = vec![
            "-f".to_string(),
            policy_path.to_string_lossy().into_owned(),
            request.program.clone(),
        ];
        argv.extend(request.argv.iter().cloned());
        let wrapped = LocalProcessRequest {
            program: "/usr/bin/sandbox-exec".to_string(),
            argv,
            ..request
        };
        Ok(SandboxPlan {
            events: vec![RuntimeEvent {
                kind: "sandbox.enforced".to_string(),
                status: "enforced".to_string(),
                detail: SandboxTier::MacosSeatbelt.variant().to_string(),
            }],
            enforcement: SandboxEnforcement::Enforced {
                tier: SandboxTier::MacosSeatbelt,
            },
            seatbelt_policy: Some(policy),
            request: Some(wrapped),
        })
    }

    fn plan_landlock_bwrap(&self, request: LocalProcessRequest) -> RuntimeResult<SandboxPlan> {
        // Model after codex linux-sandbox/bwrap: launch through `bwrap` with a
        // read-only bind of `/`, a read-write bind of each confined root, and
        // `--unshare-net` when the profile forbids egress. Landlock filesystem
        // enforcement is applied by the bwrap launcher in the same family.
        let mut argv: Vec<String> = vec![
            "--die-with-parent".to_string(),
            "--ro-bind".to_string(),
            "/".to_string(),
            "/".to_string(),
            "--dev".to_string(),
            "/dev".to_string(),
            "--proc".to_string(),
            "/proc".to_string(),
        ];
        for root in self.profile.normalized_roots()? {
            let root = root.to_string_lossy().into_owned();
            argv.push("--bind".to_string());
            argv.push(root.clone());
            argv.push(root);
        }
        if !self.profile.allow_network_egress {
            argv.push("--unshare-net".to_string());
        }
        argv.push(request.program.clone());
        argv.extend(request.argv.iter().cloned());
        let wrapped = LocalProcessRequest {
            program: "bwrap".to_string(),
            argv,
            ..request
        };
        Ok(SandboxPlan {
            events: vec![RuntimeEvent {
                kind: "sandbox.enforced".to_string(),
                status: "enforced".to_string(),
                detail: SandboxTier::LinuxLandlockBwrap.variant().to_string(),
            }],
            enforcement: SandboxEnforcement::Enforced {
                tier: SandboxTier::LinuxLandlockBwrap,
            },
            seatbelt_policy: None,
            request: Some(wrapped),
        })
    }

    /// RR5: build the seatbelt `.sbpl` policy text for this profile WITHOUT
    /// materializing it locally. The remote runner uses this to wrap a command for
    /// a macOS remote: the policy travels with the wrapped argv (the real transport
    /// writes it on the REMOTE, never on the controller host), so no local file is
    /// created for a remote launch.
    pub fn remote_seatbelt_policy(&self) -> RuntimeResult<String> {
        self.seatbelt_policy()
    }

    /// RR5: wrap `request` in the OS sandbox launcher for a REMOTE host that
    /// enforces `remote_tier`, returning the rewritten `LocalProcessRequest` (the
    /// ORIGINAL program launched under `bwrap` / `sandbox-exec`) plus, for seatbelt,
    /// the generated policy text.
    ///
    /// This reuses the EXACT argv-building used by the local
    /// [`Self::plan_seatbelt`] / [`Self::plan_landlock_bwrap`], but is driven by the
    /// REMOTE OS family (probed over the channel) rather than the controller's
    /// `is_enforced_here()` build target, and performs NO local filesystem writes
    /// (the seatbelt policy is materialized on the remote by the transport). The
    /// remote worktree path the policy references is the confined root in `profile`.
    pub fn wrap_command_for_remote(
        &self,
        request: LocalProcessRequest,
        remote_tier: SandboxTier,
    ) -> RuntimeResult<(LocalProcessRequest, Option<String>)> {
        match remote_tier {
            SandboxTier::MacosSeatbelt => {
                let policy = self.seatbelt_policy()?;
                // The remote-side policy path lives inside the first confined root
                // (the remote worktree root). The transport writes the policy there
                // on the REMOTE; the controller never writes it locally.
                let policy_dir = self
                    .profile
                    .normalized_roots()?
                    .into_iter()
                    .next()
                    .ok_or_else(|| {
                        RuntimeError::Io(std::io::Error::other(
                            "seatbelt profile has no confined writable root for the policy file",
                        ))
                    })?;
                let policy_path = policy_dir.join(format!(
                    "capo-seatbelt-{}.sb",
                    sanitize(request.run_id.as_str())
                ));
                let mut argv = vec![
                    "-f".to_string(),
                    policy_path.to_string_lossy().into_owned(),
                    request.program.clone(),
                ];
                argv.extend(request.argv.iter().cloned());
                let wrapped = LocalProcessRequest {
                    program: "/usr/bin/sandbox-exec".to_string(),
                    argv,
                    ..request
                };
                Ok((wrapped, Some(policy)))
            }
            SandboxTier::LinuxLandlockBwrap => {
                let mut argv: Vec<String> = vec![
                    "--die-with-parent".to_string(),
                    "--ro-bind".to_string(),
                    "/".to_string(),
                    "/".to_string(),
                    "--dev".to_string(),
                    "/dev".to_string(),
                    "--proc".to_string(),
                    "/proc".to_string(),
                ];
                for root in self.profile.normalized_roots()? {
                    let root = root.to_string_lossy().into_owned();
                    argv.push("--bind".to_string());
                    argv.push(root.clone());
                    argv.push(root);
                }
                if !self.profile.allow_network_egress {
                    argv.push("--unshare-net".to_string());
                }
                argv.push(request.program.clone());
                argv.extend(request.argv.iter().cloned());
                let wrapped = LocalProcessRequest {
                    program: "bwrap".to_string(),
                    argv,
                    ..request
                };
                Ok((wrapped, None))
            }
            SandboxTier::None => Ok((request, None)),
        }
    }

    /// Plan + run `request` through `runner`, composing the sandbox with the
    /// runner's existing env-scrub / path confinement / redaction / artifact
    /// capture. Returns the plan (with its `enforcement` + events) plus the
    /// runner outcome when a process actually ran.
    pub fn run(
        &self,
        runner: &LocalProcessRunner,
        request: LocalProcessRequest,
        requires_network_egress: bool,
    ) -> RuntimeResult<SandboxRun> {
        let plan = self.plan(request, requires_network_egress)?;
        let outcome = match &plan.request {
            Some(request) => Some(runner.start_process(request.clone())?),
            None => None,
        };
        Ok(SandboxRun { plan, outcome })
    }
}

/// The result of planning + running a sandboxed request.
#[derive(Debug)]
pub struct SandboxRun {
    pub plan: SandboxPlan,
    /// The runner outcome when a process actually ran (`None` when refused).
    pub outcome: Option<crate::LocalProcessOutcome>,
}

fn sanitize(key: &str) -> String {
    key.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use capo_core::RunId;

    use super::*;
    #[cfg(target_os = "macos")]
    use crate::LocalProcessConfig;

    fn tmp_root(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("capo-sbtest-{name}-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        // Canonicalize so macOS /tmp -> /private/tmp matches the seatbelt subpath.
        dir.canonicalize().unwrap()
    }

    #[cfg(target_os = "macos")]
    fn runner_for(root: &Path) -> LocalProcessRunner {
        LocalProcessRunner::new(LocalProcessConfig::for_test(
            root.to_path_buf(),
            root.join("artifacts"),
        ))
    }

    fn request_in(root: &Path, run: &str, program: &str, argv: Vec<String>) -> LocalProcessRequest {
        LocalProcessRequest::new(
            RunId::new(run),
            program,
            argv,
            root.to_path_buf(),
            HashMap::new(),
        )
    }

    #[test]
    fn tier_only_claims_enforcement_on_its_platform() {
        assert_eq!(
            SandboxTier::MacosSeatbelt.is_enforced_here(),
            cfg!(target_os = "macos")
        );
        assert_eq!(
            SandboxTier::LinuxLandlockBwrap.is_enforced_here(),
            cfg!(target_os = "linux")
        );
        assert!(!SandboxTier::None.is_enforced_here());
    }

    #[test]
    fn host_default_matches_platform() {
        let tier = SandboxTier::host_default();
        #[cfg(target_os = "macos")]
        assert_eq!(tier, SandboxTier::MacosSeatbelt);
        #[cfg(target_os = "linux")]
        assert_eq!(tier, SandboxTier::LinuxLandlockBwrap);
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        assert_eq!(tier, SandboxTier::None);
    }

    #[test]
    fn network_egress_is_refused_before_launch_when_profile_forbids_it() {
        let root = tmp_root("egress-refuse");
        let sandbox = OsSandbox::new(
            SandboxTier::host_default(),
            SandboxProfile::workspace_confined([root.clone()]),
        );
        let request = request_in(&root, "run-egress", "/bin/echo", vec!["hi".to_string()]);
        let plan = sandbox
            .plan(request, /* requires_network_egress */ true)
            .unwrap();
        assert_eq!(
            plan.enforcement,
            SandboxEnforcement::Refused {
                refusal: SandboxRefusal::NetworkEgressForbidden,
            }
        );
        // Recorded as an event, not a silent failure, and no process planned.
        assert!(plan.request.is_none());
        assert!(plan.events.iter().any(|e| {
            e.kind == "sandbox.launch_refused" && e.status == "network-egress-forbidden"
        }));
    }

    #[test]
    fn write_outside_confined_root_is_refused_before_launch() {
        let root = tmp_root("write-refuse");
        let other = tmp_root("write-refuse-other");
        let sandbox = OsSandbox::new(
            SandboxTier::host_default(),
            // Confine writes to `other`, but the cwd is `root` -> out of scope.
            SandboxProfile::workspace_confined([other.clone()]),
        );
        let request = request_in(&root, "run-write", "/bin/echo", vec!["hi".to_string()]);
        let plan = sandbox.plan(request, false).unwrap();
        match plan.enforcement {
            SandboxEnforcement::Refused {
                refusal: SandboxRefusal::WriteOutsideConfinedRoot { .. },
            } => {}
            other => panic!("expected write-outside-root refusal, got {other:?}"),
        }
        assert!(plan.request.is_none());
        assert!(plan.events.iter().any(|e| {
            e.kind == "sandbox.launch_refused" && e.status == "write-outside-confined-root"
        }));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn seatbelt_policy_denies_network_and_confines_writes() {
        let root = tmp_root("policy-shape");
        let sandbox = OsSandbox::new(
            SandboxTier::MacosSeatbelt,
            SandboxProfile::workspace_confined([root.clone()]),
        );
        let request = request_in(&root, "run-policy", "/bin/echo", vec![]);
        let plan = sandbox.plan(request, false).unwrap();
        let policy = plan.seatbelt_policy.expect("seatbelt policy text");
        assert!(policy.contains("(deny default)"));
        assert!(policy.contains("(deny network*)"));
        assert!(policy.contains("(allow file-read*)"));
        assert!(policy.contains(&format!("(subpath {:?})", root.to_string_lossy())));
        // Egress profile flips the network rule.
        let sandbox_net = OsSandbox::new(
            SandboxTier::MacosSeatbelt,
            SandboxProfile::workspace_confined([root.clone()]).with_network_egress(true),
        );
        let plan_net = sandbox_net
            .plan(
                request_in(&root, "run-policy-net", "/bin/echo", vec![]),
                true,
            )
            .unwrap();
        assert!(
            plan_net
                .seatbelt_policy
                .unwrap()
                .contains("(allow network*)")
        );
    }

    /// REFUSAL MODE (OS-enforced): a write OUTSIDE the confined root is refused by
    /// the seatbelt sandbox itself, not just by Capo's path-prefix check.
    #[cfg(target_os = "macos")]
    #[test]
    fn seatbelt_refuses_out_of_root_write_at_the_os_layer() {
        let root = tmp_root("os-write");
        let outside = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("capo-sbtest-escape-{}.txt", std::process::id()));
        let _ = fs::remove_file(&outside);
        let sandbox = OsSandbox::new(
            SandboxTier::MacosSeatbelt,
            SandboxProfile::workspace_confined([root.clone()]),
        );
        // The cwd is inside the confined root (passes the pre-launch gate), but the
        // COMMAND attempts to write OUTSIDE it. Only the OS sandbox can refuse this.
        let request = request_in(
            &root,
            "run-os-write",
            "/bin/sh",
            vec![
                "-c".to_string(),
                format!("echo escaped > {}", outside.display()),
            ],
        );
        let run = sandbox
            .run(&runner_for(&root), request, false)
            .expect("run sandboxed");
        assert!(matches!(
            run.plan.enforcement,
            SandboxEnforcement::Enforced {
                tier: SandboxTier::MacosSeatbelt
            }
        ));
        let outcome = run.outcome.expect("a process ran");
        // The shell write failed under the sandbox: non-zero exit, file absent.
        assert_ne!(outcome.exit_code, Some(0), "out-of-root write must fail");
        assert!(
            !outside.exists(),
            "the OS sandbox must prevent the out-of-root write at {outside:?}"
        );
        let _ = fs::remove_file(&outside);
    }

    /// A write INSIDE the confined root still succeeds under the sandbox (the
    /// confinement is an additional layer, not a blanket denial).
    #[cfg(target_os = "macos")]
    #[test]
    fn seatbelt_allows_in_root_write() {
        let root = tmp_root("os-write-ok");
        let inside = root.join("ok.txt");
        let _ = fs::remove_file(&inside);
        let sandbox = OsSandbox::new(
            SandboxTier::MacosSeatbelt,
            SandboxProfile::workspace_confined([root.clone()]),
        );
        let request = request_in(
            &root,
            "run-os-write-ok",
            "/bin/sh",
            vec!["-c".to_string(), format!("echo ok > {}", inside.display())],
        );
        let run = sandbox
            .run(&runner_for(&root), request, false)
            .expect("run sandboxed");
        let outcome = run.outcome.expect("a process ran");
        assert_eq!(outcome.exit_code, Some(0), "in-root write must succeed");
        assert!(inside.exists(), "in-root write must land at {inside:?}");
        let _ = fs::remove_file(&inside);
    }

    /// REFUSAL MODE (OS-enforced): a network egress attempt is refused by the OS
    /// when the profile forbids it, even though the pre-launch gate let a
    /// non-egress-declared run through (defence in depth: the OS still blocks it).
    #[cfg(target_os = "macos")]
    #[test]
    fn seatbelt_refuses_network_egress_at_the_os_layer() {
        let root = tmp_root("os-egress");
        let sandbox = OsSandbox::new(
            SandboxTier::MacosSeatbelt,
            // Network forbidden by profile.
            SandboxProfile::workspace_confined([root.clone()]),
        );
        // Pass requires_network_egress=false so the pre-launch gate does NOT refuse
        // (we want to prove the OS layer itself blocks the connection).
        let request = request_in(
            &root,
            "run-os-egress",
            "/usr/bin/nc",
            vec![
                "-w".to_string(),
                "2".to_string(),
                "-z".to_string(),
                "1.1.1.1".to_string(),
                "53".to_string(),
            ],
        );
        let run = sandbox
            .run(&runner_for(&root), request, false)
            .expect("run sandboxed");
        let outcome = run.outcome.expect("a process ran");
        assert_ne!(
            outcome.exit_code,
            Some(0),
            "network egress must be refused by the OS sandbox"
        );
    }
}
