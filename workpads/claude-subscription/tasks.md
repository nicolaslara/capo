# Claude Subscription Tasks

## Objective

Lift Claude from a gated stub to a REAL subscription-backed workspace-write + chat
provider at CODEX PARITY. Codex is the live-proven reference: a Codex-bound agent
drives a one-shot chat turn through `CodexLiveAdapter`
(`crates/capo-adapters/src/codex_live.rs`), and a Codex dispatch runs the
workspace-write live-provider path
(`crates/capo-server/src/live_provider.rs::run_live_provider_local`). The chat
one-shot is gated by `CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT` +
`CAPO_SERVER_RUN_CODEX_LIVE`; the dispatch live-write is gated by the SAME
provider-agnostic write-mode gate in `safety_floor.rs`
(`LIVE_WRITE_OPT_IN_ENV = "CAPO_SERVER_RUN_CODEX_LIVE"` + caller opt-in +
attended). This workpad makes Claude reach the SAME two real surfaces: (1) treat
the `claude` subscription CLI as a privileged connector whose tokens are never
logged and never passed to the spawned process; (2) a real one-shot CHAT turn and
a real workspace-write run; (3) `stream-json` parsing into the loop's existing
normalized-event ingestion route; (4) observed-only tool-result round-trip; (5)
UNBLOCK to parity across the CLI register seam AND the dispatch live-provider
executor (including a NEW Claude spawn arm + `CAPO_CLAUDE_BIN` dispatch override);
(6) deterministic stub tests plus a live opt-in chat-and-write smoke that skips
cleanly.

## Status

Planned. All tasks pending.

## Current State (verified in-tree, 2026-06-02)

A substantial Claude adapter slice already landed under the `depth` workpad's DP4.
This workpad is scoped against what EXISTS, not a green field. Already in-tree
(verified):

- `crates/capo-adapters/src/claude_live.rs`: `ClaudeCodeLiveAdapter` (an
  `AgentAdapter` impl, `binding.variant = "claude-live"`, `fake: false`) whose
  `run_one_shot` (line 153) drives the confined one-shot via
  `ClaudeCodeAdapter::local_workspace_write_launch_plan` (line 158) and asserts
  `assert_subscription_safe()` BEFORE spawn (line 174), fail-closed-fast behind
  `claude_live_chat_gate_open()` (`CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT` +
  `CAPO_SERVER_RUN_CLAUDE_LIVE`), with bounded wall-clock,
  `scan_artifacts_for_sensitive_markers`, a `CAPO_CLAUDE_BIN`/test-stub
  absolute-path program override (chat path only), and a gate-off
  `GateClosed { missing_env }` test already present.
- `crates/capo-adapters/src/local_subscription.rs`: TWO distinct Claude profiles --
  `local_launch_plan` (line 293: `--permission-mode plan`, `--tools ""`,
  `--disallowedTools *`, NO `--add-dir`; the no-tools profile used by
  `local_smoke_plan`), and `local_workspace_write_launch_plan` (line 345:
  `--permission-mode acceptEdits`, MCP/slash disabled, `--add-dir <workspace>`; the
  profile the live chat adapter and the dispatch write arm use). Plus
  `assert_subscription_safe` (rejects TOKEN/KEY/SECRET/COOKIE in the env
  allowlist), and `local_subscription_cli_env_allowlist` (HOME/PATH/TMPDIR/... only)
  so the runtime's `env_clear()` spawn never leaks `ANTHROPIC_API_KEY` /
  `ANTHROPIC_AUTH_TOKEN`.
- `crates/capo-adapters/src/provider_parsers.rs`:
  `ClaudeCodeAdapter::parse_stream_json` -> `NormalizedAdapterEvent`s
  (system/assistant/tool_use/result), with `claude:*` timeline keys; and the
  already-landed parity guard test `claude_normalized_events_match_codex_trait_seam_shape`
  (`crates/capo-adapters/src/claude_live.rs:420`).
- `crates/capo-server/src/lib.rs`: server-side `bind_claude`,
  `RealChatBinding::Claude`, `claude_handle()`, and `RegisterAgent { adapter:
  "claude" }` accepted and routed through `chat_controller`
  (`AgentAdapterHandle::claude`).
- `crates/capo-server/src/live_provider.rs`: the preflight stamps `adapter_kind`
  from `adapter_label(request.adapter)` (line 87), which maps `claude` ->
  `claude_code`, rejecting only `acp` (line 88). So a Claude dispatch DOES reach
  `live_execution_blockers` (line 542) carrying `adapter_kind == "claude_code"`.
- `crates/capo-server/src/tests/claude_chat.rs`: an always-on end-to-end chat test
  that already asserts the summary is the STUB's parsed Claude text, NOT the fake
  summary (lines 174-177), a gate-off fail-closed-fast E2E test (line 220), and a
  `#[ignore]` live chat smoke (line 289).
- `crates/capo-server/src/util.rs`: `claude` -> `claude_code` adapter label,
  `claude_code` -> `anthropic_claude_code_cli` provider kind, and `parse_stream_json`
  fixture wiring.

The REMAINING parity gaps this workpad closes:

- The CLI chat register seam (`crates/capo-cli/src/server_client.rs::require_chat_adapter_arg`,
  line 527) accepts only `fake`/`codex` and REJECTS `claude`, so the server's
  Claude binding is unreachable from `capo server agent register --adapter claude`.
  (`require_adapter_arg` and `require_live_provider_adapter_arg`, lines 537/547,
  already accept `claude`.)
- The dispatch live-provider executor is HARDCODED to Codex far beyond the blocker
  line: `live_execution_blockers` (line 551) blocks every `adapter_kind !=
  "codex_exec"`; AND `run_live_provider_local`'s spawn arm (lines 464-489)
  unconditionally builds `CodexExecAdapter` plans, with a Codex-only program
  override (`codex_program_override` field, `CAPO_CODEX_BIN`, lines 484-489) and a
  Codex-shaped `execute_codex_live_provider`. There is NO `claude_program_override`
  / `CAPO_CLAUDE_BIN` seam on the dispatch struct.
- The dispatch live-write gate is provider-agnostic: `safety_floor.rs::resolve_write_mode`
  reads ONE constant `LIVE_WRITE_OPT_IN_ENV = "CAPO_SERVER_RUN_CODEX_LIVE"` (line
  63); there is NO per-provider RUN env on the dispatch path.
  `CAPO_SERVER_RUN_CLAUDE_LIVE` is consumed ONLY by the chat one-shot
  (`claude_live_chat_gate_open`). The Claude dispatch write therefore rides the
  EXISTING `CAPO_SERVER_RUN_CODEX_LIVE` write-mode gate; this workpad does NOT
  introduce a provider-keyed write gate (that would re-architect the safety floor,
  which is a Non-Goal).
- Claude `tool_use`/`tool_result` `stream-json` records are not yet mapped to the
  observed-only tool-result projection Codex emits for `file_change`/`apply_patch`.
- No workspace-write live smoke for Claude (only the chat smoke exists), and no
  deterministic dispatch-route stub test for a Claude live executor spawn arm.

## CS0 - Workpad, Routing, Scope, Connector Policy, And Verification Invariant

Status: pending.

Scope:

- Establish `claude-subscription` as an INDEPENDENT, parallelizable workpad (task
  prefix `CS`) that lifts Claude to Codex parity across BOTH real surfaces: the
  chat/steer one-shot route and the dispatch live-provider workspace-write route.
- Record the connector policy (the injected decision): the `claude` subscription
  CLI is a PRIVILEGED CONNECTOR, not an ordinary API key. Capo references the
  subscription by HANDLE (the local `claude` login / `~/.claude` session), never by
  raw token. `ANTHROPIC_API_KEY` / `ANTHROPIC_AUTH_TOKEN` are UNRELATED credentials
  that must be scrubbed (absent from the launch env allowlist; cleared by the
  runtime `env_clear()` spawn). No API keys, OAuth/subscription tokens, cookies,
  session files, or transcripts-with-secrets are ever logged; raw `stream-json`
  output is content-hashed/redacted before retention, not rendered.
- Record the gate map precisely: the chat one-shot is gated by
  `CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT` + `CAPO_SERVER_RUN_CLAUDE_LIVE`; the
  dispatch live-write is gated by the provider-agnostic `safety_floor.rs`
  write-mode gate (`CAPO_SERVER_RUN_CODEX_LIVE` + caller opt-in + attended), which
  Claude rides UNCHANGED.
- Record the boundaries this workpad owns (the Claude chat one-shot, the Claude
  workspace-write launch profile, `stream-json` normalization + observed tool
  results, the CLI chat register seam for `--adapter claude`, the dispatch
  live-provider executor's Claude spawn arm + `CAPO_CLAUDE_BIN` dispatch override +
  the blocker allow-list, the live opt-in smokes) and what it defers (the turn loop
  / confinement / checkpoint / ceiling floor to `real-turn-loop`; the
  `PermissionPolicy`/grant lifecycle to `safety-gates`; the OS sandbox + git
  worktree isolation to `depth` DP7/DP8; ACP to `depth` DP1-DP3; the web client to
  the web agent; any change to `resolve_write_mode`'s provider-agnostic gate to
  `safety-gates`).
- Record the verification invariant (mirrors `depth`): no task completes on
  operator self-attestation; deterministic stub/fixture tests land BEFORE any live
  Claude path; every live smoke is paired with a deterministic assertion
  (normalized-event shape, stub output, or restart/replay); the live path stays
  behind its documented gates and `#[ignore]`/skips cleanly when the gate is off or
  `claude` is absent; secrets are stripped from all evidence.

Acceptance criteria:

- `workpads/claude-subscription/{tasks,knowledge,references}.md` exist and state
  the objective, the connector policy decision, the exact gate map, the
  owned/deferred boundaries, the per-task dependencies, and the verification
  invariant above.
- `knowledge.md` records that Codex is the reference and names the TWO real Codex
  surfaces Claude must match (`CodexLiveAdapter` chat one-shot;
  `run_live_provider_local` dispatch executor), and that the chat-route Claude
  slice already exists while the CLI chat seam, the dispatch spawn arm + override,
  and the blocker allow-list are the open gaps.
- `knowledge.md` records the scrub rule concretely: the only secret defense is the
  env allowlist + runtime `env_clear()`; `assert_subscription_safe()` is the
  fail-closed assertion, asserted on every Claude launch plan before spawn.

Verification:

- Markdown-only task; no code. Reviewed against `AGENTS.md` safety boundary and the
  `depth/DP4` evidence so no already-landed work is re-planned, and against
  `safety_floor.rs` so the gate map is not misstated.

Dependencies: none (planning task). Intra-workpad: gates CS1-CS6.

## CS1 - Subscription Connector Policy: Scrub, Handle-Only, Never-Log (Hardening + Tests)

Status: pending.

Prerequisite: `real-turn-loop` + `tools-aci` (the runtime `env_clear()` spawn and
`scan_artifacts_for_sensitive_markers` exist).

Scope:

- Harden and TEST the existing Claude connector policy so it is a checkable
  acceptance criterion rather than a code comment. The mechanism already exists
  (`local_subscription_cli_env_allowlist`, `assert_subscription_safe`,
  `local_adapter_redaction_rules`, runtime `env_clear()`); this task proves it for
  Claude specifically and closes any gap.

Acceptance criteria:

- A deterministic test asserts BOTH Claude launch plans
  (`local_launch_plan` and `local_workspace_write_launch_plan`) carry an
  `env_allowlist` containing NONE of `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`,
  nor any name matching TOKEN/KEY/SECRET/COOKIE (mirroring the existing Codex
  `env_allowlist` test at `crates/capo-adapters/src/tests.rs`).
- A deterministic test asserts `ClaudeCodeAdapter::local_workspace_write_launch_plan(..).assert_subscription_safe()`
  returns `Ok`, and that injecting an `ANTHROPIC_AUTH_TOKEN` allowlist entry makes
  it return `Err` (fail-closed), so the assertion is load-bearing (mirrors the
  existing `assert_subscription_safe` ok/err shape).
- A deterministic test proves a spawned Claude stub that prints its visible
  environment does NOT receive `ANTHROPIC_API_KEY`/`ANTHROPIC_AUTH_TOKEN` even when
  both are set in the parent process env (the `env_clear()` + allowlist scrub is
  enforced end-to-end, not just declared).
- `ClaudeCodeLiveAdapter::run_one_shot` calls `assert_subscription_safe()` before
  spawn (already true, line 174); a test asserts a tampered plan with a secret-like
  arg is refused before any process starts.
- `knowledge.md` records that the subscription is referenced by handle (the local
  `claude` login), that Capo never reads `~/.claude` credential material, and that
  the redaction rules + sensitive-marker scan are the retention defense on stdout.
- Close the `sensitive_marker` scan gap (verified `local_subscription.rs:488-521`):
  the scanner does not recognize `auth_token` / `anthropic_auth_token` keyword
  substrings and an `ANTHROPIC_AUTH_TOKEN` value has no `sk-` shape, so a bearer
  token value in stdout would not be caught. Add `auth_token` / `anthropic_auth_token`
  (and any other bearer keyword) to the keyword list, with a deterministic test that
  a line containing an `auth_token`-style value is flagged by `sensitive_marker`. The
  env-clear allowlist remains the primary defense; this hardens the secondary scan.

Verification:

- Deterministic `cargo test -p capo-adapters` covering both allowlists, the
  `assert_subscription_safe` ok/err pair, and the spawned-stub env scrub.
- `cargo fmt --check`; `cargo clippy --all-targets --all-features -- -D warnings`.
- `git diff --check`.
- No live provider used.

Dependencies: CS0. Intra-workpad: feeds CS2/CS5.

## CS2 - Real send_turn: Chat One-Shot + Workspace-Write Profile (Mirror The Codex Chat Path)

Status: LANDED. Chat one-shot largely landed under DP4; CS2 pinned parity and the
profile facts with deterministic assertions in
`crates/capo-adapters/src/claude_live.rs`
(`claude_try_send_turn_stub_matches_codex_turn_output_reduction`,
`claude_launch_profiles_pin_exact_argv`, and the KEPT gate-off
`claude_send_turn_fails_closed_fast_when_gate_off`), plus the test-only
`codex_live::turn_output_from_events_for_test` parity helper. No live provider.

Prerequisite: `real-turn-loop` + `tools-aci`; CS1.

Scope:

- Confirm + harden the real Claude `send_turn` so the chat path mirrors the Codex
  chat path exactly, and pin the EXACT launch profile the live chat adapter and the
  dispatch write arm (CS5) drive. The chat one-shot
  (`ClaudeCodeLiveAdapter::try_send_turn` -> `run_one_shot` ->
  `local_workspace_write_launch_plan` -> parse -> `TurnOutput`) already exists; this
  task pins parity and the fail-closed contract with assertions.

Acceptance criteria:

- A deterministic test (no live provider) drives `ClaudeCodeLiveAdapter::try_send_turn`
  with a pinned absolute-path stub through `LocalProcessRunner` and asserts the
  resulting `TurnOutput` shape (summary = last item content, status =
  `result.subtype`, `tool_name`, `external_session_ref` = Claude `session-id`,
  `confidence = 80`) matches the Codex `turn_output_from_events` reduction for the
  same logical turn.
- A deterministic test asserts gate-OFF fail-closed-fast: with neither gate set,
  `try_send_turn` returns `GateClosed { missing_env: [PREFLIGHT, RUN_CLAUDE_LIVE] }`
  IMMEDIATELY and spawns nothing (program override pointed at a nonexistent binary
  that must never run), and the infallible `send_turn` shim surfaces a `blocked`
  turn with `confidence: 0`. This CONFIRMS/KEEPS the already-landed gate-off test
  (`claude_live.rs`); it is not new work, and is marked as such so the
  "what's remaining" accounting stays precise.
- A deterministic argv test asserts the EXACT profile the live chat adapter
  invokes. The live chat adapter calls `local_workspace_write_launch_plan`
  (verified `claude_live.rs:158`): `claude -p --output-format stream-json --verbose
  --permission-mode acceptEdits --no-session-persistence --disable-slash-commands
  --mcp-config /dev/null --strict-mcp-config --add-dir <workspace> <prompt>`. The
  test also asserts the SEPARATE `local_launch_plan` profile
  (`--permission-mode plan`, `--tools ""`, `--disallowedTools *`, NO `--add-dir`)
  exists and is distinct, so the spec does not conflate the two.
- `knowledge.md` records the profile facts precisely: the LIVE chat adapter and the
  dispatch write arm share ONE profile (`local_workspace_write_launch_plan`,
  `acceptEdits` + `--add-dir`), while a distinct read-bounded `local_launch_plan`
  (`plan` mode, no `--add-dir`, used by `local_smoke_plan`) also exists. The shared
  write profile is safe because the run is gated (env flags), confined by `--add-dir
  <workspace>` COMPOSED with the `real-turn-loop` path confinement, and bracketed by
  the pre-write checkpoint floor; MCP and slash commands are disabled so the run can
  only touch the confined workspace.

Verification:

- Deterministic `cargo test -p capo-adapters` (stub-driven `try_send_turn`,
  gate-off fast path keep-test, argv assertions for both profiles). Reuses the
  existing `CLAUDE_LIVE_ENV_LOCK` serialization for the process-global env gate.
- `cargo fmt --check`; `cargo clippy`; `git diff --check`.
- Live write/chat deferred to CS6.

Dependencies: CS0, CS1. Intra-workpad: feeds CS5/CS6.

## CS3 - stream-json Parsing Into The Loop's Existing Ingestion Route

Status: pending (parser landed under DP4; this task PROVES loop-route reuse + usage
mapping with assertions; the parity guard test already exists and is KEPT).

Prerequisite: `real-turn-loop` + `tools-aci`; CS2.

Scope:

- Prove that Claude `stream-json` normalizes through the SAME ingestion route the
  loop already uses (`apply_normalized_adapter_events_with_turn` /
  `parse_adapter_events`), never a parallel Claude-only route, and that the
  normalized-event vocabulary is identical to Codex at the trait seam.

Acceptance criteria:

- The already-landed parity guard test
  `claude_normalized_events_match_codex_trait_seam_shape`
  (`crates/capo-adapters/src/claude_live.rs:420`) is KEPT and asserts the ordered
  `NormalizedAdapterEvent` KINDS for a representative Claude `stream-json`
  transcript (`system` session start, `assistant` message with `usage`, `tool_use`,
  terminal `result`) equal the Codex JSONL kinds for the same logical turn
  (`adapter.session_started`, `adapter.item_completed`,
  `adapter.tool_call_started`, `adapter.turn_completed`).
- A deterministic test asserts the Claude `external_session_ref` derives from the
  Claude `session-id`, and `adapter.turn_completed` carries `input_tokens` /
  `output_tokens` from the Claude `usage` block. (If this assertion is not already
  in the DP4 parser tests, ADD it; otherwise keep it.)
- A capo-server test feeds Claude-parsed events through
  `parse_adapter_events("claude_code", ..)` ->
  `apply_normalized_adapter_events_with_turn` and asserts the Claude turn lands as
  the SAME read-model row shape a Codex turn produces (loop-route reuse proven, not
  self-attested), mirroring the DP1 ACP loop-route proof.
- `knowledge.md` records the mapping table: Claude record type -> normalized event
  kind, and notes that `stream-json` message chunks without stable IDs fall back to
  content-hash + ordinal anchors with low confidence (consistent with the
  ACP/Codex ID-less handling), so Claude introduces no new ingestion vocabulary.

Verification:

- Deterministic `cargo test -p capo-adapters -p capo-server`.
- `cargo fmt --check`; `cargo clippy`; `git diff --check`.
- No live provider used.

Dependencies: CS0, CS2. Intra-workpad: feeds CS5.

## CS4 - Tool-Result Round-Trip (Observed-Only, Mirroring Codex)

Status: pending.

Prerequisite: `real-turn-loop` + `tools-aci`; CS3.

Scope:

- Decide and implement the Claude tool-result contract. Capo emits Claude tool
  results as OBSERVED-ONLY, exactly like the Codex `file_change`/`apply_patch`
  observed tool results (`instrumentation_level = "observed_only"`), and Capo sends
  NO Capo-authored tool result back over the one-shot. The verifiable half is what
  this task tests; the claim that the `claude -p` one-shot would not accept a
  result anyway is an OBSERVATION (Open Question), not a load-bearing acceptance
  bullet.

Acceptance criteria:

- `ClaudeCodeAdapter` `stream-json` parsing maps a Claude `tool_use` to an
  `adapter.tool_call_started` and the matching `tool_result` to an OBSERVED tool
  result event carrying the observed content, projecting with
  `instrumentation_level = "observed_only"` (mirroring
  `crates/capo-adapters/src/provider_parsers.rs` `codex_*_tool_result_content` and
  the `adapter_tool_observations_are_observed_only` test).
- A deterministic fixture test asserts a Claude `tool_use` + `tool_result` pair
  projects into a tool observation distinct from the agent's reported message, with
  `observed_only` instrumentation.
- A deterministic test asserts the VERIFIABLE negative: the Claude adapter writes
  NO Capo-authored tool result to the one-shot stdin and has no result-injection
  code path (assert the adapter's stdin write set / argv carries no result channel).
  This is the testable half of "observed-only is explicit, not an accident."
- `knowledge.md` records the decision: Capo emits Claude tool results observed-only
  and injects nothing back. As an OBSERVATION (and Open Question), the observed
  `claude -p` one-shot CLI did not surface a result-injection channel; native
  result delivery is revisited only if/when such a channel appears (e.g. an
  ACP/stream-input mode), and that revisit routes through the `depth` ACP adapter,
  not a Claude-specific channel. The CLI-can't-accept-it statement is NOT a
  load-bearing acceptance criterion.

Verification:

- Deterministic `cargo test -p capo-adapters` (observed tool-result fixture + the
  no-injection negative).
- `cargo fmt --check`; `cargo clippy`; `git diff --check`.
- No live provider used.

Dependencies: CS0, CS3. Intra-workpad: feeds CS5.

## CS5 - Unblock To Parity: CLI Chat Seam + Dispatch Executor (Spawn Arm + Override + Allow-List)

Status: pending.

Prerequisite: `real-turn-loop` + `tools-aci`; CS1-CS4. (The `safety-gates`
confinement/permission floor is the live-execution precondition the executor
already enforces via the gate + blockers + the `safety_floor.rs` write-mode gate.)

Scope:

- Close the THREE unblock gaps that keep Claude from Codex parity. The chat seam is
  a one-line CLI fix; the dispatch executor is a deeper change because
  `run_live_provider_local` is Codex-shaped well beyond the blocker line and has no
  Claude program-override seam. All behind the shared preflight + the EXISTING
  provider-agnostic `safety_floor.rs` write-mode gate (`CAPO_SERVER_RUN_CODEX_LIVE`
  + caller opt-in + attended), fail-closed when off:
  1. The CLI chat-adapter register seam, so `capo server agent register --adapter
     claude` reaches the server's already-wired Claude binding.
  2. The dispatch live-provider executor blocker allow-list.
  3. The dispatch executor SPAWN ARM: branch the plan builder on `adapter_kind`,
     add a Claude program-override seam, and confirm the floor/scan are
     provider-agnostic.

Acceptance criteria:

- `crates/capo-cli/src/server_client.rs::require_chat_adapter_arg` (line 527)
  accepts `claude` (alongside `fake` default and `codex`) and rejects anything
  else; its doc-comment and the error message name `claude`. A test asserts
  `--adapter claude` resolves to `"claude"` and an unknown value still errors.
- `crates/capo-server/src/live_provider.rs::live_execution_blockers` (line 551)
  admits `adapter_kind == "claude_code"` for live execution (replacing the hard
  `!= "codex_exec"` block with an allow-list of enabled live providers, currently
  `{codex_exec, claude_code}`); an un-enabled adapter still pushes
  `provider_not_enabled_for_first_live_slice`. A test asserts a Claude dispatch
  reaches `live_execution_blockers` carrying `adapter_kind == "claude_code"` (the
  preflight already stamps it via `adapter_label`, verified line 87), so the
  allow-list widening is exercised on a real Claude plan, not a tolerated string.
- The dispatch SPAWN ARM is branched on `adapter_kind`
  (`run_live_provider_local`, lines 464-489): for `claude_code` it builds
  `ClaudeCodeAdapter::local_workspace_write_launch_plan` (write) /
  `local_launch_plan` (dry-run) instead of the Codex plans, asserts
  `assert_subscription_safe()`, and runs through the SAME structure the Codex arm
  uses. A NEW `claude_program_override` field + `CAPO_CLAUDE_BIN` threading is added
  to the dispatch request struct (mirroring `codex_program_override` /
  `CAPO_CODEX_BIN`, lines 484-489) -- WITHOUT it the deterministic stub and the live
  smoke cannot pin a binary, because the `env_clear()` spawn honors only absolute
  paths. This dispatch override is DISTINCT from the chat-side `CAPO_CLAUDE_BIN` in
  `claude_live.rs`; both are named as separate seams.
- The `confine_and_checkpoint_for_write` floor sequence and
  `scan_artifacts_for_sensitive_markers` are confirmed provider-agnostic and run
  for a `claude_code` `LiveWrite` exactly as for Codex (the spawn arm builds the
  Claude plan, but confinement/checkpoint/scan are unchanged). The Claude write
  ingests parsed `stream-json` via `apply_normalized_adapter_events_with_turn`.
- The dispatch live-write gate is the EXISTING provider-agnostic
  `safety_floor.rs::resolve_write_mode` gate (`LIVE_WRITE_OPT_IN_ENV =
  "CAPO_SERVER_RUN_CODEX_LIVE"` + caller opt-in + attended). A `claude_code`
  `LiveWrite` requires all three; with any missing it falls back to dry-run (no
  live edit). This workpad does NOT add a per-provider dispatch RUN env and does NOT
  change `resolve_write_mode` (Non-Goal). `knowledge.md` states this plainly.
- TWO deterministic dispatch tests (no live `claude`), because they exercise
  DIFFERENT code paths:
  (a) MOCK-OUTPUT ingestion: drive the Claude `run-local` route with
      `mock_provider_output_jsonl` (a Claude `stream-json` fixture) +
      `mock_runtime_opt_in`, asserting the mocked output ingests into the run's read
      models. NOTE this short-circuits at `ingest_mock_live_provider_output` (line
      587) BEFORE the spawn arm, so it proves ONLY `parse_adapter_events("claude_code",
      ..)` + ingestion -- NOT that the spawn arm is unblocked.
  (b) STUB-BINARY spawn: drive the unblocked spawn arm with an absolute-path Claude
      stub via the new `claude_program_override`/`CAPO_CLAUDE_BIN` seam, the caller
      opt-in, and the write-mode env gate, asserting the stub's `stream-json`
      ingests through `apply_normalized_adapter_events_with_turn` -- this is the test
      that actually proves the executor unblock through the spawn arm.
- A deterministic test asserts an UNSUPPORTED adapter (e.g. `acp`) is still blocked
  from the live executor, so the unblock is provider-scoped, not blanket.

Verification:

- Deterministic `cargo test -p capo-cli -p capo-server` (CLI chat seam; Claude
  dispatch via mock output; Claude dispatch via stub-binary spawn arm;
  unsupported-adapter still blocked; gate-off/dry-run fail-closed).
- `cargo fmt --check`; `cargo clippy --all-targets --all-features -- -D warnings`.
- `git diff --check`.
- Live execution deferred to CS6; here only mock-output + stub-binary + dry-run
  paths run.

Dependencies: CS0-CS4. Intra-workpad: feeds CS6. Cross-workpad: builds on
`real-turn-loop` confinement/checkpoint and the `safety_floor.rs` write-mode gate
and the existing `safety-gates`/operator attended-run gating that the executor
already consults; does NOT change those.

## CS6 - Deterministic Stub Tests + Live Opt-In Claude Smoke (Chat + Write), Paired

Status: pending.

Prerequisite: CS1-CS5 landed with their deterministic suites green (mirrors
`depth` DP11's "subject tasks first" rule).

Scope:

- Consolidate the deterministic Claude suite and add the live opt-in smoke for BOTH
  real surfaces (chat one-shot and workspace-write dispatch), each PAIRED with a
  deterministic assertion so completion is never operator-attested. Mirror the
  Codex smokes (`crates/capo-server/src/tests/codex_chat.rs` chat smoke;
  `live_smoke.rs` / `codex_workspace_write.rs` write smoke).

Acceptance criteria:

- The always-on deterministic E2E gate asserts: (a) a Claude-bound agent registered
  via the running server (`RegisterAgent { adapter: "claude" }`) drives the stub
  chat turn end-to-end and the summary is the STUB's parsed Claude assistant text,
  NOT the fake summary -- this EXTENDS the already-landed
  `claude_bound_chat_flows_real_stub_output_end_to_end_through_the_running_server`
  test (`claude_chat.rs`, which already asserts the stub summary at lines 174-177)
  by routing registration through the now-unblocked CLI chat seam (CS5); (b) the
  Claude dispatch `run-local` route ingests a mock `stream-json` fixture into read
  models AND the stub-binary spawn arm ingests (CS5 (a) and (b)); (c) a fake-bound
  agent on the SAME server still routes through the fake adapter (already asserted);
  (d) gate-OFF Claude chat returns an immediate typed error, fast (the landed
  `claude_bound_chat_fails_closed_fast_end_to_end_when_gate_is_off` test, KEPT).
- A live CHAT smoke (`#[ignore]` + `CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT=1`
  `CAPO_SERVER_RUN_CLAUDE_LIVE=1`) spawns the REAL `claude` one-shot through the
  running server and asserts the returned summary has the SAME `TurnOutput` shape
  the always-on stub test pins; it skips cleanly (passing) when the gates are off or
  `claude` is absent. This EXTENDS the already-landed `claude_live_chat_smoke`
  (`claude_chat.rs:289`).
- A live WRITE smoke (`#[ignore]` + the chat gates AND the dispatch write-mode gate
  `CAPO_SERVER_RUN_CODEX_LIVE=1` + caller opt-in + attended) drives the Claude
  dispatch `run-local` executor against the REAL `claude` workspace-write one-shot
  in a confined throwaway workspace, asserts a file edit landed and the parsed
  `stream-json` ingested, and is PAIRED with the deterministic stub-binary dispatch
  assertion from CS5 (b) (same normalized-event/read-model shape). Skips cleanly
  when gates are off or `claude` is absent. `knowledge.md` notes the write smoke
  needs `CAPO_SERVER_RUN_CODEX_LIVE` (the provider-agnostic write gate), NOT a
  Claude-specific dispatch RUN env.
- Every smoke strips secrets: artifacts pass `scan_artifacts_for_sensitive_markers`;
  the agent stderr is scanned for token markers; raw `stream-json` is redacted /
  content-hashed before retention; a leaked-marker artifact is dropped.
- STRENGTHEN the already-landed `claude_live_chat_smoke` (`claude_chat.rs:289`),
  which today asserts only the STATIC `external_session_ref` and `!summary.is_empty()`
  (a liveness ping, not a shape check): the extended smoke MUST assert the SAME
  `TurnOutput` shape the always-on stub test pins AND run a smoke-level
  `scan_artifacts_for_sensitive_markers`, so the paired-deterministic-assertion
  invariant actually holds (it does not for the landed smoke as written).
- Process-global env safety: the gate-toggling tests
  (`open_live_gate`/`close_live_gate`/`set_claude_bin`/`clear_claude_bin`) must
  restore env on PANIC, not only on the happy path. Use a Drop guard (a struct whose
  `Drop` impl resets the gate/bin env) rather than a manual close at the end of the
  test body, so a mid-test assertion failure cannot leak gate env into other tests in
  the same binary (the `CLAUDE_CHAT_ENV_LOCK` mutex serializes but does not restore
  env on unwind).
- A review note in `knowledge.md` records architecture fit (Claude rides the same
  `AgentAdapter` trait, the same chat route, and the same dispatch executor as
  Codex; no new ingestion vocabulary; connector stays scrub-only; the dispatch
  write gate stays provider-agnostic) and whether any Claude-specific parity gap
  remains open.

Verification:

- `cargo fmt --check`.
- Deterministic `cargo test -p capo-adapters -p capo-server -p capo-cli` (the
  always-on E2E gate, stub chat, mock dispatch, stub-binary dispatch spawn).
- Live chat + write smokes behind explicit opt-in env gates, secrets stripped, each
  paired with its deterministic fixture assertion; both `#[ignore]` and skip
  cleanly when unavailable.
- `git diff --check`.

Dependencies: CS0-CS5. Cross-workpad: none required to run the deterministic gate;
the live chat smoke consumes the chat gates, the live write smoke consumes the
shared `CAPO_SERVER_RUN_CODEX_LIVE` write-mode gate Codex uses.
