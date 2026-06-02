# Claude Subscription Knowledge

## Objective

Capture decisions for the `claude-subscription` workpad: lift Claude to a real
subscription-backed workspace-write + chat provider at CODEX PARITY. Codex is the
live-proven reference and Claude must reach the SAME two real surfaces Codex
already reaches, while treating the `claude` subscription CLI as a privileged
connector whose credentials are never logged and never handed to the spawned
process.

## Scope And Independence

This is an INDEPENDENT, parallelizable workpad (prefix `CS`). It depends on
`real-turn-loop` + `tools-aci` (the `AgentAdapter` trait, the `LocalProcessRunner`
env-scrub/confinement, the `apply_normalized_adapter_events_with_turn` ingestion
route, the `scan_artifacts_for_sensitive_markers` scan, and the `safety_floor.rs`
write-mode gate), all of which exist. It is breadth, not a re-architecture: Claude
plugs in below the `AgentAdapter` trait and through the existing dispatch
live-provider executor, exactly where Codex does. It touches no goal model,
transport protocol, permission engine, OS sandbox, ACP, or web client, and it does
NOT change `safety_floor.rs::resolve_write_mode`.

## Codex Is The Reference: The Two Real Surfaces

Codex is real on TWO distinct surfaces, and Claude must match BOTH:

1. The CHAT/STEER one-shot route. A Codex-bound agent's `SendTask`/`SteerAgent`
   turn routes (by binding) through `CodexLiveAdapter::try_send_turn`
   (`crates/capo-adapters/src/codex_live.rs`), which spawns a confined one-shot,
   parses its output, and reduces to a provider-neutral `TurnOutput`,
   fail-closed-fast behind `CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT` +
   `CAPO_SERVER_RUN_CODEX_LIVE`. The server wires this via `chat_controller` /
   `RealChatBinding` (`crates/capo-server/src/lib.rs`).
2. The DISPATCH live-provider route. A Codex dispatch (`server dispatch
   live-preflight` -> `run-local`) runs the real workspace-write provider through
   `CapoServer::run_live_provider_local` (`crates/capo-server/src/live_provider.rs`),
   ingesting parsed output via `apply_normalized_adapter_events_with_turn`, gated by
   `live_execution_blockers` + the caller opt-in + the `safety_floor.rs` write-mode
   gate + attended.

Verified current state: Claude's chat-route slice already landed under `depth`
DP4 (`ClaudeCodeLiveAdapter`, server `bind_claude`/`claude_handle`,
`claude_chat.rs` E2E + ignore smoke, the parser, and the
`claude_normalized_events_match_codex_trait_seam_shape` parity test). The parity
gaps are (a) the CLI chat register seam rejecting `--adapter claude`, (b) the
dispatch executor hard-blocking every adapter except `codex_exec` AND being
Codex-shaped in its spawn arm with no Claude program-override, and (c) the observed
tool-result mapping + the live write smoke. This workpad closes all three.

## The Gate Map (Verified, Do Not Misstate)

Two gates exist and they are DIFFERENT:

- CHAT one-shot gate: `claude_live_chat_gate_open()` requires
  `CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT` + `CAPO_SERVER_RUN_CLAUDE_LIVE`.
  `CAPO_SERVER_RUN_CLAUDE_LIVE` is consumed ONLY here.
- DISPATCH live-write gate: `safety_floor.rs::resolve_write_mode` reads ONE
  provider-agnostic constant `LIVE_WRITE_OPT_IN_ENV = "CAPO_SERVER_RUN_CODEX_LIVE"`
  (line 63), and a `LiveWrite` requires `live_execution_opt_in && env_opt_in &&
  !unattended`. There is NO per-provider RUN env on the dispatch path. The Claude
  dispatch write therefore rides the EXISTING `CAPO_SERVER_RUN_CODEX_LIVE`
  write-mode gate, UNCHANGED. Introducing a provider-keyed dispatch write gate would
  re-architect the safety floor and is a Non-Goal; if it is ever wanted it belongs
  to `safety-gates`, not here.

NAMING CAUTION (load-bearing): the env var name `CAPO_SERVER_RUN_CODEX_LIVE` is read
by TWO independent code paths for DIFFERENT purposes -- the Codex CHAT one-shot gate
(`codex_live.rs::CODEX_LIVE_RUN_OPT_IN_ENV`) and the provider-agnostic DISPATCH
write-mode gate (`safety_floor.rs::LIVE_WRITE_OPT_IN_ENV`). They happen to share the
string but are independent reads. Any future rename of `LIVE_WRITE_OPT_IN_ENV` must
NOT silently break the Codex chat gate (and vice versa); CS5/CS6 implementers must
treat the two as distinct seams that currently alias the same env name.

## Connector Policy (Injected Decision)

The `claude` subscription CLI is a PRIVILEGED CONNECTOR, not an ordinary API key.

- Capo references the subscription by HANDLE: the operator's local `claude` login
  (the `~/.claude` session the CLI manages). Capo never reads, copies, or logs that
  credential material; it only spawns `claude` as the logged-in user.
- The ONLY secret defense at spawn is the env allowlist plus the runtime's
  `env_clear()`: `local_subscription_cli_env_allowlist` is HOME/PATH/TMPDIR/USER/
  LOGNAME/SHELL/LANG only. `ANTHROPIC_API_KEY` / `ANTHROPIC_AUTH_TOKEN` are
  UNRELATED credentials (an API-billing path, not the subscription) and are never in
  the allowlist, so `env_clear()` strips them. `assert_subscription_safe()` fails
  closed if the allowlist ever contains a TOKEN/KEY/SECRET/COOKIE name or the argv
  carries a secret-like marker, and it is asserted on every Claude launch plan
  before spawn (verified `claude_live.rs:174`).
- Never log API keys, OAuth/subscription tokens, cookies, session files, or
  transcripts-with-secrets. The PRIMARY (and most reliable) defense is the env
  allowlist + `env_clear()` above: the token values are never PASSED to the spawned
  process, so they cannot appear in its output. Raw `stream-json` is content-hashed /
  redacted before retention as a SECONDARY retention defense
  (`local_adapter_redaction_rules` + `scan_artifacts_for_sensitive_markers`); a
  leaked-marker artifact is dropped. Connector concerns stay separate from agent
  execution and controller state.
- KNOWN GAP in the secondary scan (verified in `local_subscription.rs::sensitive_marker`,
  lines 488-521): the scanner catches known API-key SHAPES (`sk-proj-`, `sk-ant-`,
  `sk-live-`, `sk_test_`, `sk-svcacct-`, legacy `sk-`) and explicit KEYWORD
  substrings (`authorization:`, `cookie:`, `set-cookie:`, `session_token`,
  `access_token`, `refresh_token`, `oauth`, `api_key`, `anthropic_api_key`,
  `openai_api_key`). It does NOT recognize `auth_token` / `anthropic_auth_token` as a
  keyword, and an `ANTHROPIC_AUTH_TOKEN` value has no `sk-` prefix shape -- so if such
  a bearer/session token value ever appeared in stdout, the scan would miss it. The
  env-clear allowlist is why this gap is not a live leak (the token is never handed to
  the process), but the scan did NOT cover all bearer/session token value formats.
  CS1 CLOSED this by adding `auth_token` / `anthropic_auth_token` to the
  `sensitive_marker` keyword list (`local_subscription.rs`), with the deterministic
  `sensitive_marker_scan_flags_auth_token_values` test asserting an
  `ANTHROPIC_AUTH_TOKEN`/`auth_token`-style value in stdout is now flagged. The
  env-clear allowlist remains the PRIMARY defense; this hardens the secondary scan.
- The connector records auth MODE only (e.g. `user_local_subscription`), never
  credential material. CS1's allowlist test + the spawned-stub env-scrub test are
  the load-bearing checks for this; no connector record/event carries a credential
  field (if any connector record is emitted on a Claude launch, a CS1 assertion
  confirms it contains no credential value).

## CS1 Status (Landed)

CS1 hardened and TESTED the connector policy so it is a checkable acceptance
criterion rather than a code comment. The subscription is referenced by HANDLE
only -- the operator's local `claude` login (the `~/.claude` session the CLI
manages). Capo never reads, copies, or logs that credential material; it only
spawns `claude` as the logged-in user. The redaction rules
(`local_adapter_redaction_rules`) + the `scan_artifacts_for_sensitive_markers`
scan are the SECONDARY retention defense on stdout; the env allowlist +
`env_clear()` is the PRIMARY defense. The load-bearing CS1 tests live in
`crates/capo-adapters/src/tests.rs`:

- `claude_launch_plans_carry_no_secret_like_env_allowlist_entries`: BOTH Claude
  launch plans (`local_launch_plan` + `local_workspace_write_launch_plan`) carry an
  env_allowlist with NONE of `ANTHROPIC_API_KEY` / `ANTHROPIC_AUTH_TOKEN` nor any
  TOKEN/KEY/SECRET/COOKIE name.
- `claude_workspace_write_plan_assert_subscription_safe_is_load_bearing`: the plan
  is `Ok` as built; injecting an `ANTHROPIC_AUTH_TOKEN` allowlist entry makes
  `assert_subscription_safe()` return `Err` (fail-closed).
- `claude_spawned_stub_does_not_inherit_anthropic_connector_env`: a stub spawned
  through `LocalProcessRunner` does NOT receive `ANTHROPIC_API_KEY` /
  `ANTHROPIC_AUTH_TOKEN` even when both are set in the parent env (the `env_clear()`
  + allowlist scrub is enforced end-to-end).
- `claude_live_one_shot_refuses_tampered_secret_arg_before_spawn`: a tampered
  workspace-write plan (the one `run_one_shot` drives, asserting subscription-safe
  before spawn at `claude_live.rs:174`) with a secret-like argv is refused before
  any process starts.
- `sensitive_marker_scan_flags_auth_token_values`: the closed scan gap (above).

No connector record/event carries a credential field; the connector records auth
MODE only (`user_local_subscription`).

## CS2 Status (Landed)

CS2 confirmed and HARDENED the real Claude `send_turn` so the chat path mirrors
the Codex chat path exactly, pinning parity and the launch-profile facts with
deterministic assertions (no live provider). The chat one-shot
(`ClaudeCodeLiveAdapter::try_send_turn` -> `run_one_shot` ->
`local_workspace_write_launch_plan` -> `parse_stream_json` ->
`turn_output_from_events` -> `TurnOutput`) already existed under DP4; CS2 added the
missing deterministic coverage. The load-bearing CS2 tests live in
`crates/capo-adapters/src/claude_live.rs`:

- `claude_try_send_turn_stub_matches_codex_turn_output_reduction`: drives
  `try_send_turn` with the gates ON (via a `GateGuard` Drop guard that restores env
  even on panic) through a pinned absolute-path POSIX stub spawned by
  `LocalProcessRunner`, and asserts the reduced `TurnOutput` (summary = last item
  content `applied the workspace edit`, status = `result.subtype` `success`,
  `tool_name` = `Edit`, `external_session_ref` = Claude `session-id`
  `claude-sess-1`, `confidence = 80`) equals the Codex `turn_output_from_events`
  reduction for the same logical turn (asserted via the test-only
  `codex_live::turn_output_from_events_for_test`).
- `claude_send_turn_fails_closed_fast_when_gate_off` (KEPT from DP4, annotated as
  CS2): with neither gate set, `try_send_turn` returns `GateClosed { missing_env:
  [PREFLIGHT, RUN_CLAUDE_LIVE] }` immediately and spawns nothing (the override
  points at a nonexistent binary), and the infallible `send_turn` shim surfaces a
  `blocked` turn with `confidence: 0`.
- `claude_launch_profiles_pin_exact_argv`: pins the EXACT argv of the
  `local_workspace_write_launch_plan` profile the live chat adapter invokes
  (`-p --output-format stream-json --verbose --permission-mode acceptEdits
  --no-session-persistence --disable-slash-commands --mcp-config /dev/null
  --strict-mcp-config --add-dir <ws> <prompt>`) AND asserts the SEPARATE
  read-bounded `local_launch_plan` profile (`--permission-mode plan --tools ""
  --disallowedTools *`, NO `--add-dir`) is distinct, so the two are never
  conflated.

The reused `CLAUDE_LIVE_ENV_LOCK` mutex serializes the process-global env-gate
mutation across the gated tests in the module; the `GateGuard` Drop guard restores
the prior gate env on unwind so a mid-test failure cannot leak gate env into other
tests in the binary. Live write/chat remain deferred to CS6.

## CS3 Status (Landed)

CS3 PROVED loop-route reuse + usage mapping with deterministic assertions (no
live provider). Claude `stream-json` normalizes through the SAME
`parse_adapter_events("claude_code", ..)` ->
`apply_normalized_adapter_events_with_turn` route the loop already uses, never a
parallel Claude-only route, and a Claude turn lands as the SAME projected
read-model row SHAPE a Codex turn produces. The load-bearing CS3 tests:

- `crates/capo-adapters/src/claude_live.rs::claude_normalized_events_match_codex_trait_seam_shape`
  (KEPT, parity guard): the ordered `NormalizedAdapterEvent` KINDS for a
  representative Claude `stream-json` turn (`system` -> `assistant` -> `tool_use`
  -> `result`) equal the Codex JSONL kinds for the same logical turn
  (`adapter.session_started`, `adapter.item_completed`,
  `adapter.tool_call_started`, `adapter.turn_completed`).
- `crates/capo-adapters/src/claude_live.rs::claude_turn_completed_carries_usage_tokens_and_session_ref`
  (KEPT): the Claude `external_session_ref` derives from the Claude `session-id`
  (`claude-sess-1`) and `adapter.turn_completed` carries `input_tokens`/
  `output_tokens` from the Claude `usage` block.
- `crates/capo-server/src/tests/claude_loop_route.rs::claude_turn_lands_same_read_model_row_shape_as_codex_through_shared_ingestion_route`
  (NEW, mirrors the DP1 ACP loop-route proof): drives BOTH a Claude
  `stream-json` fixture and a Codex JSONL fixture through the SAME ungated
  production write path (`ServerCommand::ReplayAdapterFixture` ->
  `parse_adapter_events(<adapter>, ..)` ->
  `apply_normalized_adapter_events_with_turn`) and asserts the projected
  read-model row SHAPE is identical: one completed turn, an assistant summary,
  exactly one OBSERVED-ONLY tool result distinct from the agent's claim, a
  recorded tool call, the `session.summary_updated` + `tool.observation_recorded`
  events, and equal replay tool/summary/turn ingestion counts. The
  `ReplayAdapterFixture` route is provider-agnostic and ungated (the dispatch
  live-execution gate that CS5 unblocks is a SEPARATE path), so the test does not
  depend on CS5.

Retention note observed in CS3: the loop projects the assistant claim into a
session summary as a CONTENT-HASHED anchor (e.g.
`Adapter claude_code assistant observed content_hash=fnv1a64:...`), not the raw
assistant text -- the connector retention policy (raw `stream-json` is
content-hashed/redacted before retention, never rendered). This is identical to
the Codex projection shape; Claude introduces no new ingestion vocabulary.

## CS4 Status (Landed)

CS4 decided and IMPLEMENTED the Claude tool-result contract: Capo emits Claude
`tool_use`/`tool_result` records as OBSERVED-ONLY, exactly like the Codex
`file_change`/`apply_patch` observed results that project with
`instrumentation_level = "observed_only"`, and Capo injects NO Capo-authored tool
result back over the one-shot. No live provider.

- The parser (`crates/capo-adapters/src/provider_parsers.rs`) maps a Claude
  `tool_use` -> `adapter.tool_call_started` and the matching `tool_result` ->
  `adapter.tool_call_completed` carrying the OBSERVED tool-returned content. A new
  `claude_tool_result_content` helper reduces a `tool_result.content` that is
  EITHER a plain string OR a content-block array
  (`[{"type":"text","text":...}]`) to one observed-result string (the Claude
  analogue of `codex_tool_result_content`), so the observed result is captured
  distinct from the agent's `assistant` message. Both project through the existing
  `NormalizedAdapterEvent::tool_observation()` with `instrumentation_level =
  "observed_only"`; Claude introduces no new ingestion vocabulary.
- The load-bearing CS4 tests live in `crates/capo-adapters/src/tests.rs` (fixture
  `crates/capo-adapters/fixtures/claude-code-tool-result.jsonl`):
  - `claude_tool_use_result_pair_projects_observed_only_distinct_from_agent_message`:
    a `tool_use` + `tool_result` pair projects observed-only tool observations
    (`source_adapter = claude_code`, `external_tool_ref = toolu_cs4`) -- the
    `tool_use` start carries the tool NAME (`Edit`); Claude's `tool_result` record
    itself carries no name, so the named observation comes from the start event --
    whose observed result content (`Applied edit to NOTES.md ...`) is DISTINCT from
    the agent's reported `assistant` message (`I will edit NOTES.md ...`).
  - `claude_one_shot_writes_no_capo_authored_tool_result_and_has_no_result_channel`:
    the VERIFIABLE negative. (1) The workspace-write launch argv carries no
    result-injection flag (`--input`/`--input-format`/`--tool-result`/`--stdin`/
    `-i`). (2) The `LocalProcessRequest` the adapter builds is purely program +
    argv + cwd + env -- it has NO stdin/result payload field, and the one-shot
    spawn path (`LocalProcessRunner::spawn_process`) redirects stdout/stderr to
    artifact files and never pipes stdin, so there is structurally no channel to
    inject a result over. (3) The `claude_live.rs` source contains neither
    `write_stdin` nor `spawn_piped_process` (the only stdin-capable runtime APIs),
    so the one-shot opens no bidirectional/result-injection pipe.
- OBSERVATION (Open Question, NOT a load-bearing acceptance bullet): the observed
  `claude -p` one-shot CLI did not surface a result-injection channel. Capo cannot
  deterministically test that external-CLI claim; it only asserts Capo injects
  nothing (the negative above). Native result delivery is revisited only if such a
  channel appears (e.g. an ACP/stream-input mode), and that revisit routes through
  the `depth` ACP adapter, not a Claude-specific channel.

## Two Claude Launch Profiles (Precise Facts)

There are TWO distinct Claude launch plans in
`crates/capo-adapters/src/local_subscription.rs`; do not conflate them:

- `local_launch_plan` (line 293): `claude -p --output-format stream-json --verbose
  --permission-mode plan --no-session-persistence --disable-slash-commands --tools
  "" --disallowedTools * --mcp-config /dev/null --strict-mcp-config <prompt>`. A
  no-tools, read-bounded `plan`-mode profile with NO `--add-dir`. Used by
  `local_smoke_plan`.
- `local_workspace_write_launch_plan` (line 345): `claude -p --output-format
  stream-json --verbose --permission-mode acceptEdits --no-session-persistence
  --disable-slash-commands --mcp-config /dev/null --strict-mcp-config --add-dir
  <workspace> <prompt>`. The workspace-write profile.

The LIVE chat adapter (`ClaudeCodeLiveAdapter::run_one_shot`) and the dispatch
write arm (CS5) BOTH use `local_workspace_write_launch_plan` (verified
`claude_live.rs:158`). So Claude's chat and write surfaces SHARE one profile -- the
`acceptEdits` + `--add-dir` write profile -- unlike Codex's read-only chat vs
workspace-write split. That is safe because every Claude run is gated (its
respective env gate), confined by `--add-dir <workspace>` COMPOSED with the
`real-turn-loop` path confinement (the flag-level confinement is not a substitute
for the runtime confinement), and bracketed by the pre-write checkpoint floor; MCP
and slash commands are disabled so the run can only touch the confined workspace,
never an external connector. (Whether the chat route SHOULD instead use the
read-bounded `plan`-mode `local_launch_plan` is an Open Question below.)

## stream-json Normalization Reuses The Loop Route

Claude `stream-json` records normalize through `ClaudeCodeAdapter::parse_stream_json`
into the SAME `NormalizedAdapterEvent` vocabulary Codex emits, and ingest through
the SAME `parse_adapter_events` / `apply_normalized_adapter_events_with_turn` route
the loop already uses -- never a parallel Claude-only route. Mapping:

- `system` (session start) -> `adapter.session_started` (`external_session_ref` =
  Claude `session-id`).
- `assistant` message -> `adapter.item_completed` (content = text; `usage`
  carried).
- `tool_use` -> `adapter.tool_call_started`; `tool_result` -> observed tool-result
  event (`instrumentation_level = "observed_only"`).
- `result` -> `adapter.turn_completed` (status = `subtype`; `usage` ->
  input/output tokens).

The parity guard `claude_normalized_events_match_codex_trait_seam_shape`
(`claude_live.rs:420`) already pins the event KINDS against the Codex trait seam.
`stream-json` message chunks without stable IDs fall back to content-hash + ordinal
anchors with low confidence, consistent with the Codex/ACP ID-less handling. Claude
introduces NO new ingestion vocabulary.

## Tool Results Are Observed-Only

Capo emits Claude `tool_use`/`tool_result` records as OBSERVED-ONLY, exactly like
the Codex `file_change`/`apply_patch` observed results that project with
`instrumentation_level = "observed_only"`, and Capo sends NO Capo-authored tool
result back over the one-shot. The TESTABLE acceptance is the verifiable half:
Capo emits observed-only AND has no result-injection code path / writes no result
to stdin (CS4). As an OBSERVATION (and Open Question), the observed `claude -p`
one-shot CLI did not surface a result-injection channel; that "the CLI wouldn't
accept a result anyway" is NOT a load-bearing acceptance criterion (it is an
external-CLI behavioral claim Capo cannot deterministically test). Native result
delivery is revisited only if a result-injection channel appears (e.g. an
ACP/stream-input mode of the CLI), and that revisit routes through the `depth` ACP
adapter, not a Claude-specific channel.

## Unblock To Parity

Three gaps remain OPEN in-tree today; CS5 closes all three, behind the shared
preflight + the existing provider-agnostic write-mode gate, fail-closed when off.
Each bullet states the CURRENT (still-Codex-only) reality first, then what CS5 WILL
change -- nothing below has been done yet:

- CLI chat seam: TODAY `require_chat_adapter_arg` (`server_client.rs:527`) accepts
  only `fake`/`codex` and REJECTS `claude` (verified: the match arm rejects with
  "supports `fake` (default) or `codex`"). CS5 WILL widen it to accept `claude`,
  reaching the server's already-wired `RealChatBinding::Claude` / `claude_handle`.
  (`require_adapter_arg` / `require_live_provider_adapter_arg`, lines 537/547,
  already accept `claude`.)
- Dispatch blocker allow-list: TODAY `live_execution_blockers`
  (`live_provider.rs:551`) still has the literal `if plan.adapter_kind !=
  "codex_exec"` hard block, so a Claude dispatch IS blocked. CS5 WILL widen it to an
  ENABLED-providers allow-list (`{codex_exec, claude_code}`). The preflight already
  stamps `adapter_kind == "claude_code"` for a Claude dispatch (via `adapter_label`,
  line 87; only `acp` is rejected at line 88), so once widened the allow-list is
  exercised on a real Claude plan. An un-enabled adapter (e.g. `acp`) will still
  block, so the unblock is provider-scoped, not blanket.
- Dispatch spawn arm: TODAY `run_live_provider_local`'s spawn arm (lines 464-489) is
  Codex-shaped -- it unconditionally builds `CodexExecAdapter` plans, with NO Claude
  branch, and has only a Codex override (`codex_program_override` field,
  `CAPO_CODEX_BIN`). CS5 WILL branch the plan builder on `adapter_kind` so a
  `claude_code` dispatch builds
  `ClaudeCodeAdapter::local_workspace_write_launch_plan` (write) /
  `local_launch_plan` (dry-run), assert subscription-safe, run through the SAME
  `LocalProcessRunner` + `confine_and_checkpoint_for_write` +
  `scan_artifacts_for_sensitive_markers` + `apply_normalized_adapter_events_with_turn`
  structure the Codex arm uses (those floor/scan/ingest steps are
  provider-agnostic), and add a NEW `claude_program_override` field +
  `CAPO_CLAUDE_BIN` dispatch threading (mirroring the Codex override). This dispatch
  `CAPO_CLAUDE_BIN` is DISTINCT from the chat-side `CAPO_CLAUDE_BIN` in
  `claude_live.rs`: they are two separate seams on two separate code paths. Without
  the dispatch override the deterministic stub-binary test and the live write smoke
  cannot pin a binary (the `env_clear()` spawn honors only absolute paths).

The mock-output path (`mock_provider_output_jsonl` ->
`ingest_mock_live_provider_output`, line 587) short-circuits BEFORE the spawn arm,
so it proves only `parse_adapter_events("claude_code", ..)` + ingestion. Proving the
spawn-arm unblock requires the stub-binary path via the new override. CS5 keeps
these two tests separate so neither claim is overstated.

## CS5 Status (Landed)

CS5 closed all THREE unblock gaps, behind the shared preflight + the existing
provider-agnostic `safety_floor.rs` write-mode gate, fail-closed when off. No live
`claude` is used; deterministic mock-output + stub-binary + dry-run paths only.

1. CLI chat seam: `crates/capo-cli/src/server_client.rs::require_chat_adapter_arg`
   now accepts `claude` (alongside the `fake` default and `codex`) and rejects
   anything else; its doc-comment and error message name `claude`. Made
   `pub(crate)` so the deterministic test
   `crates/capo-cli/src/tests.rs::require_chat_adapter_arg_accepts_claude_and_rejects_unknown`
   asserts `--adapter claude` resolves to `"claude"`, `fake`/`codex` are unchanged,
   and an unknown value (`gemini`) still errors with a message naming `claude`.
2. Dispatch blocker allow-list:
   `crates/capo-server/src/live_provider.rs::live_execution_blockers` replaced the
   hard `plan.adapter_kind != "codex_exec"` block with an enabled-providers
   allow-list `matches!(.., "codex_exec" | "claude_code")`. An un-enabled adapter
   still pushes `provider_not_enabled_for_first_live_slice`. The preflight already
   stamps `claude_code` via `adapter_label` (only `acp` is rejected up front), so
   the widening is exercised on a real Claude plan.
3. Dispatch spawn arm: `run_live_provider_local`'s plan builder is branched on
   `(plan.adapter_kind, write_mode)` -- a `claude_code` dispatch builds
   `ClaudeCodeAdapter::local_workspace_write_launch_plan` (LiveWrite) /
   `local_launch_plan` (DryRun), a Codex dispatch keeps the Codex plans. A NEW
   `claude_program_override` field on `LiveProviderLocalRunRequest` + a
   `CAPO_CLAUDE_BIN` fallback threaded at the `RunLiveProviderLocal` handler pins an
   absolute Claude stub; the program-override selection is keyed to the dispatch
   adapter so a Claude dispatch only honors the Claude override. This DISPATCH
   `CAPO_CLAUDE_BIN` is DISTINCT from the chat-side `CAPO_CLAUDE_BIN` in
   `claude_live.rs` (two separate seams, two separate code paths, same env name).
   The provider-agnostic `confine_and_checkpoint_for_write` floor +
   `scan_artifacts_for_sensitive_markers` + `assert_subscription_safe()` +
   `apply_normalized_adapter_events_with_turn` ingestion are UNCHANGED and run for a
   `claude_code` `LiveWrite` exactly as for Codex. The former Codex-named
   `execute_codex_live_provider` is renamed `execute_live_provider` to reflect that
   it is provider-agnostic (it already ingested via
   `parse_adapter_events(&plan.adapter_kind, ..)`).

The dispatch live-write gate is UNCHANGED: `safety_floor.rs::resolve_write_mode`
(`LIVE_WRITE_OPT_IN_ENV = "CAPO_SERVER_RUN_CODEX_LIVE"` + caller opt-in + attended).
A `claude_code` `LiveWrite` requires all three; missing any falls back to dry-run.
CS5 did NOT add a per-provider dispatch RUN env and did NOT change
`resolve_write_mode` (Non-Goal); the Claude dispatch write rides the existing
provider-agnostic `CAPO_SERVER_RUN_CODEX_LIVE` write-mode gate.

The load-bearing CS5 tests:

- `crates/capo-cli/src/tests.rs::require_chat_adapter_arg_accepts_claude_and_rejects_unknown`
  (CLI chat seam).
- `crates/capo-server/src/tests/live_provider.rs::server_live_provider_local_run_admits_claude_and_ingests_mock_stream_json`
  (CS5 (a): the allow-list admits `claude_code`; the plan carries
  `adapter_kind == "claude_code"`; a Claude `stream-json` fixture ingests via the
  mock-output path, which short-circuits BEFORE the spawn arm). This REPLACED the
  obsolete `server_live_provider_local_run_blocks_claude_in_first_live_slice` test,
  whose blocked-Claude assertion CS5 deliberately inverts.
- `crates/capo-server/src/tests/live_provider.rs::server_live_provider_claude_spawn_arm_ingests_stub_stream_json_through_override`
  (CS5 (b): the unblocked SPAWN ARM via the new `claude_program_override`. Asserts
  the selected argv is the Claude workspace-write profile -- `--permission-mode
  acceptEdits` + `--add-dir`, NO Codex `--sandbox` -- the stub spawned
  (`provider_cli_executed`), the confined write landed, a pre-write
  `checkpoint.created` was recorded, and the stub's `stream-json` ingested).
- `crates/capo-server/src/tests/live_provider.rs::server_live_provider_preflight_rejects_unsupported_acp_adapter`
  (the unblock is provider-scoped: `acp` cannot even reach the executor -- the
  live-provider preflight rejects it up front, so no plan is ever minted for it).

## CS6 Status (Landed) + Architecture-Fit Review

CS6 consolidated the deterministic Claude suite and added the paired live smokes:

- The always-on E2E gate is EXTENDED through the CLI chat seam: a new CLI process
  test `cli_registers_claude_agent_and_gets_real_stub_chat_through_running_server`
  (`crates/capo-cli/tests/server_transport/live.rs`) runs `capo server agent
  register --adapter claude` + `task send` against a running server with a
  `CAPO_CLAUDE_BIN` stub, and asserts the rendered `latest_summary` is the stub's
  parsed Claude assistant text — proving `require_chat_adapter_arg("claude")` (CS5)
  reaches the server's Claude binding end-to-end, not just at the unit level.
- The live CHAT smoke (`claude_chat.rs::claude_live_chat_smoke`) was strengthened
  (see the chat-smoke section above): real-summary liveness check + smoke-level
  secret scan; the vacuous `assert_ne!` was removed.
- The live WRITE smoke (`live_smoke.rs::live_claude_workspace_write_smoke`, NEW)
  drives the dispatch `run_live_provider_local` executor against the real `claude`
  workspace-write one-shot, gated on the chat gates AND
  `CAPO_SERVER_RUN_CODEX_LIVE=1`, paired with the always-on CS5 (b) stub-binary
  dispatch test. Asserts the edit landed, the pre-write checkpoint, ingested
  events, and a secrets scan over the artifact tree.
- Process-global env safety: the gate-toggling chat tests use a `LiveGateEnvGuard`
  Drop guard that snapshots and restores `CAPO_CLAUDE_BIN` + both live gates on
  EVERY exit path including a panicking assertion (the `CLAUDE_CHAT_ENV_LOCK` mutex
  serializes but does not restore on unwind). The adapter-level scrub test
  (`claude_spawned_stub_does_not_inherit_anthropic_connector_env`) now uses a
  `ScrubTestEnvGuard` (Drop guard behind `SCRUB_TEST_ENV_LOCK`) for the same reason,
  and drives the LIVE one-shot runtime path (`spawn_process` +
  `wait_running_with_timeout`) rather than `start_process`, so it exercises the
  exact env_clear branch a live Claude spawn goes through.

Architecture fit: Claude rides the SAME `AgentAdapter` trait, the SAME chat route
(`chat_controller` → `ClaudeCodeLiveAdapter`), and the SAME dispatch executor
(`run_live_provider_local`, branched on `adapter_kind`) as Codex. It introduces NO
new ingestion vocabulary — `stream-json` normalizes through
`apply_normalized_adapter_events_with_turn` into the identical read-model row shape
Codex produces. The connector stays scrub-only (env allowlist + runtime
`env_clear()`, `assert_subscription_safe()` before every spawn). The dispatch write
gate stays provider-agnostic (`CAPO_SERVER_RUN_CODEX_LIVE`). No Claude-specific
parity gap remains open: both real surfaces (chat one-shot, workspace-write
dispatch) are reachable, deterministically tested, and live-smoke-paired. Remaining
items are tracked as Open Questions (config-driven provider allow-list; a
plan-mode-vs-write-mode chat profile split; an eventual provider-keyed dispatch
write gate), all explicitly deferred to other workpads.

## Verification Discipline

Deterministic-tests-before-live holds across every task: env-scrub, `try_send_turn`
stub, `stream-json` normalization, observed tool-result + the no-injection
negative, the CLI seam, and the dispatch executor (both the mock-output ingestion
AND the stub-binary spawn arm) all have deterministic coverage (stub binary or
`mock_provider_output_jsonl`) BEFORE any live `claude` runs. Every live smoke is to
be paired with a deterministic assertion of the identical shape, stays `#[ignore]`
behind its documented gates, and skips cleanly when the gate is off or `claude` is
absent. Secrets are stripped from all evidence.

Chat smoke strengthening (CS6 — RESOLVED). The DP4-landed `claude_live_chat_smoke`
(`claude_chat.rs`) was a liveness ping only: it asserted the STATIC
`run_refs.external_session_ref == "claude-live-session-claude-live"` (which
`open_session()` returns unconditionally, proving only registration succeeded) and
`!summary.is_empty()`. CS6 strengthened it so the paired-assertion invariant
actually holds:

- It still asserts the session-ref equals the Claude binding ref, but the comment
  now states plainly this is a BINDING check (the ref is constructed by
  `open_session()` independent of whether `send_turn` ran), NOT liveness. The
  vacuous `assert_ne!` against the fake-adapter ref was REMOVED (the two refs are
  structurally distinct strings at construction, so it could never fail).
- The load-bearing liveness proof is now the summary: it must be present, non-empty,
  NOT the fake-adapter fallback (`"Fake adapter processed goal ..."`), and NOT a
  blocked/fail-closed marker — so only real provider output passes.
- It runs a smoke-level `scan_artifacts_for_sensitive_markers` over the server's
  persisted text tree (the in-`run_one_shot` scan is also correct, but the smoke now
  independently confirms the retained evidence is secrets-clean).

Intentionally NOT asserted by the live chat smoke: the exact `TurnOutput`
status/`tool_name`/confidence triple. Those are pinned by the always-on
server-level stub test
(`claude_bound_chat_flows_real_stub_output_end_to_end_through_the_running_server`,
which checks the parsed assistant summary) and the adapter-level
`claude_try_send_turn_stub_matches_codex_turn_output_reduction` (which pins the full
`TurnOutput` shape against the Codex reduction). The live model's status/tool/
confidence vary per run, so the live smoke asserts the run-shape (non-empty real
summary, ingested events, secrets-clean) rather than re-pinning fields the
deterministic tests already lock.

Both `#[ignore]` smokes are gate-paired with always-on deterministic assertions:
the live CHAT smoke pairs with the stub server-level test above; the live WRITE
smoke (`live_smoke.rs::live_claude_workspace_write_smoke`, gated on
`CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT=1` + `CAPO_SERVER_RUN_CLAUDE_LIVE=1` +
`CAPO_SERVER_RUN_CODEX_LIVE=1`) pairs with the always-on stub-binary dispatch test
`server_live_provider_claude_spawn_arm_ingests_stub_stream_json_through_override`
(CS5 b), which exercises the IDENTICAL `run_live_provider_local` spawn arm with a
`/bin/sh` claude stub. The write smoke rides the provider-agnostic
`CAPO_SERVER_RUN_CODEX_LIVE` dispatch write-mode gate, NOT a Claude-specific
dispatch RUN env.

## Non-Goals

- No web client (the web agent owns that surface).
- Do not change the turn loop, transport protocol, permission engine, goal model,
  OS sandbox, or git worktree isolation; earlier/parallel workpads own those.
- Do not change `safety_floor.rs::resolve_write_mode` or add a per-provider
  dispatch live-write RUN env; the Claude dispatch write rides the existing
  provider-agnostic `CAPO_SERVER_RUN_CODEX_LIVE` write-mode gate.
- Do not make Claude a global chat default: the Claude handle is bound only for
  agents explicitly registered `--adapter claude`.
- Do not implement native tool-result injection for the one-shot CLI (observed-only).
- Do not read or persist `~/.claude` credential material; reference the
  subscription by handle only.

## Open Questions

- Does the `claude -p` CLI ever expose a tool-result INJECTION channel (so results
  could round-trip rather than be observed-only)? If so, does it arrive as an ACP /
  stream-input mode (routing through the `depth` ACP adapter) rather than a
  Claude-specific path? (Capo cannot assert the negative about the external CLI; it
  only asserts Capo injects nothing.)
- Should the Claude CHAT route use the read-bounded `local_launch_plan`
  (`--permission-mode plan`, no `--add-dir`) instead of the shared
  `local_workspace_write_launch_plan` (`acceptEdits` + `--add-dir`)? Today the live
  chat adapter uses the write profile; a plan-mode chat would be strictly
  read-safer and would mirror Codex's read-only chat / workspace-write split.
- Should the dispatch executor's enabled-providers allow-list become config-driven
  (rather than a hard-coded `{codex_exec, claude_code}` set) as more providers land?
- Should the dispatch live-write gate eventually become provider-keyed (a
  `CAPO_SERVER_RUN_CLAUDE_LIVE`-style dispatch env) rather than the shared
  `CAPO_SERVER_RUN_CODEX_LIVE`? That is a `safety-gates` safety-floor change, not
  this workpad's.
- Is there a Claude-stable `session-id` reconnect/resume surface worth modeling
  (analogous to ACP `session/resume`), or does the one-shot make resume moot?
