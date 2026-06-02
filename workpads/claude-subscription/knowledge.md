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
  the process), but the scan does NOT cover all bearer/session token value formats.
  CS1 closes this by adding `auth_token` / `anthropic_auth_token` (and any other
  bearer keyword) to the `sensitive_marker` keyword list; until then the connector
  policy's "raw output is scrubbed" claim holds for API-key shapes and the listed
  keywords ONLY, not for arbitrary auth-token values.
- The connector records auth MODE only (e.g. `user_local_subscription`), never
  credential material. CS1's allowlist test + the spawned-stub env-scrub test are
  the load-bearing checks for this; no connector record/event carries a credential
  field (if any connector record is emitted on a Claude launch, a CS1 assertion
  confirms it contains no credential value).

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

## Verification Discipline

Deterministic-tests-before-live holds across every task: env-scrub, `try_send_turn`
stub, `stream-json` normalization, observed tool-result + the no-injection
negative, the CLI seam, and the dispatch executor (both the mock-output ingestion
AND the stub-binary spawn arm) all have deterministic coverage (stub binary or
`mock_provider_output_jsonl`) BEFORE any live `claude` runs. Every live smoke is to
be paired with a deterministic assertion of the identical shape, stays `#[ignore]`
behind its documented gates, and skips cleanly when the gate is off or `claude` is
absent. Secrets are stripped from all evidence.

Caveat on the already-landed chat smoke (verified, not yet remediated): the
DP4-landed `claude_live_chat_smoke` (`claude_chat.rs:289`) currently asserts only
`run_refs.external_session_ref == "claude-live-session-claude-live"` (a STATIC
string `open_session()` returns unconditionally as
`format!("claude-live-session-{}", request.agent_name)`, proving only registration
succeeded) and `!summary.is_empty()` (passes for a single character). This is
effectively a liveness ping, NOT the paired deterministic SHAPE assertion the
invariant requires: there is no `TurnOutput`-shape check (status/`tool_name`/
confidence) and no smoke-level `scan_artifacts_for_sensitive_markers` call (the scan
runs inside `run_one_shot`, which is correct, but the smoke cannot confirm it ran).
So the paired-assertion invariant does NOT yet hold for the landed chat smoke. CS6
MUST strengthen `claude_live_chat_smoke` to assert the SAME `TurnOutput` shape its
always-on stub test pins (and a smoke-level secret scan) so the pairing is real.
Until CS6 lands, treat the landed smoke as a liveness probe only.

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
