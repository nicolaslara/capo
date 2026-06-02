# Claude Subscription References

## Objective

Record the in-repo files/modules/docs this workpad touches or builds on, plus the
external surfaces it depends on. Verified in-tree on 2026-06-02.

## Local Implementation Sources (Touched Or Built On)

- `crates/capo-adapters/src/claude_live.rs`
  - `ClaudeCodeLiveAdapter` (the real Claude `AgentAdapter`, `binding.variant =
    "claude-live"`, `fake: false`); `try_send_turn` / `run_one_shot` (line 153),
    which builds `ClaudeCodeAdapter::local_workspace_write_launch_plan` (line 158)
    and asserts `assert_subscription_safe()` before spawn (line 174);
    `claude_live_chat_gate_open()` + `CLAUDE_LIVE_RUN_OPT_IN_ENV =
    "CAPO_SERVER_RUN_CLAUDE_LIVE"`; `turn_output_from_events` (Codex-parity
    reduction); the chat-path `CAPO_CLAUDE_BIN` / test-stub program override; the
    parity guard test `claude_normalized_events_match_codex_trait_seam_shape` (line
    420). CS1-CS2 harden + test this; CS4 extends parsing for observed tool results.
- `crates/capo-adapters/src/codex_live.rs`
  - The reference chat adapter: `CodexLiveAdapter`, `CodexLiveChatError`
    (`GateClosed { agent_name, missing_env }`), `codex_live_chat_gate_open()`,
    `CODEX_LIVE_PREFLIGHT_OPT_IN_ENV = "CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT"`,
    `CODEX_LIVE_RUN_OPT_IN_ENV = "CAPO_SERVER_RUN_CODEX_LIVE"`, and the
    `--skip-git-repo-check` chat fix. The exact pattern CS2/CS5/CS6 mirror.
- `crates/capo-adapters/src/adapter.rs`
  - The `AgentAdapter` trait, `try_send_turn` fallible seam, `AgentAdapterHandle`
    (`Fake`/`ScriptedMock`/`Codex`/`Claude`/`Acp`), `AgentAdapterHandle::claude(..)`
    and `is_real()`. Claude already a handle variant; CS proves the seam parity.
- `crates/capo-adapters/src/local_subscription.rs`
  - TWO Claude profiles: `local_launch_plan` (line 293: `--permission-mode plan`,
    `--tools ""`, `--disallowedTools *`, NO `--add-dir`; the no-tools profile,
    used by `local_smoke_plan`) and `local_workspace_write_launch_plan` (line 345:
    `acceptEdits`, MCP/slash disabled, `--add-dir <workspace>`; the profile the live
    chat adapter and the CS5 dispatch write arm use). Plus `assert_subscription_safe()`,
    `local_subscription_cli_env_allowlist()` (the scrub allowlist),
    `local_adapter_redaction_rules()`, `sensitive_marker`. CS1 tests these for
    Claude; CS5's executor builds the write plan here.
- `crates/capo-adapters/src/provider_parsers.rs`
  - `ClaudeCodeAdapter::parse_stream_json` -> `NormalizedAdapterEvent`s; the Codex
    `codex_tool_result_content` / `codex_item_tool_result_content` observed-result
    mappers CS4 mirrors for Claude `tool_use`/`tool_result`.
- `crates/capo-adapters/src/event.rs`
  - `instrumentation_level = "observed_only"` on tool observations; CS4's contract.
- `crates/capo-adapters/src/tests.rs`
  - The existing Codex `env_allowlist`-has-no-TOKEN test and the observed
    tool-result tests (`codex_workspace_write_fixture_maps_a_tool_result_round_trip`,
    `adapter_tool_observations_are_observed_only`) CS1/CS4 mirror for Claude.
- `crates/capo-server/src/lib.rs`
  - `CodexChatBindings` (now holding `claude_bound_agents`), `bind_claude`,
    `RealChatBinding::Claude`, `claude_handle()`, `chat_controller` routing, and
    `RegisterAgent { adapter: "claude" }` acceptance. Already wired; CS5 makes it
    reachable from the CLI chat seam and CS6 asserts E2E.
- `crates/capo-server/src/live_provider.rs`
  - `run_live_provider_local` (the spawn arm, lines 464-489, today building
    `CodexExecAdapter` plans with the Codex-only `codex_program_override` /
    `CAPO_CODEX_BIN` at 484-489 -- CS5 branches this on `adapter_kind` and adds a
    `claude_program_override` / `CAPO_CLAUDE_BIN` seam); `live_execution_blockers`
    (the hard `adapter_kind != "codex_exec"` block at line 551 CS5 widens to
    `{codex_exec, claude_code}`); the preflight `adapter_label(request.adapter)`
    stamping at line 87 that maps `claude` -> `claude_code` (only `acp` rejected,
    line 88); `ingest_mock_live_provider_output` (the line-587 short-circuit BEFORE
    the spawn arm); `execute_codex_live_provider` (the Codex-shaped exec helper);
    and `confine_and_checkpoint_for_write` (provider-agnostic floor sequence CS5
    reuses for Claude).
- `crates/capo-server/src/safety_floor.rs`
  - `resolve_write_mode` / `resolve_write_mode_with_env` (lines 97-119) and the
    provider-agnostic `LIVE_WRITE_OPT_IN_ENV = "CAPO_SERVER_RUN_CODEX_LIVE"` (line
    63) -- the dispatch live-write gate the Claude write rides UNCHANGED. CS does
    NOT modify this (Non-Goal); CS5/CS6 only consume it.
- `crates/capo-server/src/util.rs`
  - `adapter_label` (`claude` -> `claude_code`, line 39), `provider_kind_for_adapter`
    (`claude_code` -> `anthropic_claude_code_cli`, line 50), and `parse_stream_json`
    fixture wiring CS3/CS5 use.
- `crates/capo-cli/src/server_client.rs`
  - `require_chat_adapter_arg` (line 527, accepts `fake`/`codex` only -- CS5 adds
    `claude`); `require_adapter_arg` (line 537) / `require_live_provider_adapter_arg`
    (line 547) already accept `claude`. The single CLI chat gap.
- `crates/capo-cli/src/server_client/dispatch.rs`
  - The `server dispatch live-preflight` / `run-local` (`ln`) plumbing and the
    `CAPO_SERVER_RUN_CODEX_LIVE` opt-in CS5/CS6 drive for Claude; the seam where the
    new `CAPO_CLAUDE_BIN` dispatch override is threaded to `run_live_provider_local`.
- `crates/capo-cli/src/cli_surface.rs`
  - The `--adapter codex|claude|acp` usage strings the seam changes keep honest.
- `crates/capo-server/src/dispatch.rs`
  - The dispatch plan projection that carries `plan.adapter_kind` (e.g. lines
    103/157/417/652) into `live_execution_blockers`; CS5's "Claude reaches the
    blocker with `adapter_kind == claude_code`" assertion depends on this.
- `crates/capo-server/src/tests/claude_chat.rs`
  - The always-on Claude E2E chat test
    (`claude_bound_chat_flows_real_stub_output_end_to_end_through_the_running_server`,
    asserting the STUB summary at lines 174-177), the gate-off fail-closed-fast E2E
    test (`..._fails_closed_fast_end_to_end_when_gate_is_off`, line 220), and the
    `#[ignore]` live chat smoke (`claude_live_chat_smoke`, line 289) with a
    deterministic absolute-path `claude` stub + `CLAUDE_CHAT_ENV_LOCK`. CS6 extends
    with the write smoke and the always-on dispatch-stub/mock assertions.
- `crates/capo-server/src/tests/{codex_chat,live_smoke,codex_workspace_write,live_provider}.rs`
  - The Codex chat/write smoke + dispatch live-provider tests CS6 mirrors for Claude.

## Local Architecture / Workflow Sources

- `AGENTS.md` -- workflow, git rules, and the SAFETY BOUNDARY (auditable + revocable
  remote control; subscription agents are privileged connectors; never log
  tokens/transcripts; credentials by handle). The connector policy in CS0/CS1
  derives directly from this and it is a first-class acceptance criterion.
- `workpads/architecture/protocol-provider.md`
  - The `AgentAdapter` surface, the observed `claude -p --output-format stream-json
    --verbose` (observed CLI `2.1.150`), the `ANTHROPIC_API_KEY`/`ANTHROPIC_AUTH_TOKEN`
    scrub, and the connector-records-auth-mode-only rule.
- `workpads/architecture/boundaries.md`
  - The clients -> controller -> `AgentAdapter` -> `RuntimeRunner` boundary model
    and the event log as authoritative; Claude sits below the adapter boundary.
- `workpads/architecture/runtime-tunnel.md`
  - `RuntimeRunner` owns process lifecycle and env scrub; adapters never own process
    groups; the basis for the confined Claude one-shot spawn and the `env_clear()`
    that strips `ANTHROPIC_*`.
- `workpads/depth/tasks.md` (DP4) + `knowledge.md`
  - The already-landed Claude chat-adapter slice and the "Claude as the second write
    adapter" decision; this workpad continues that line to full parity and avoids
    re-planning landed work.

## External Sources

- Anthropic `claude` CLI (subscription / Claude Code)
  - Observed surface: `claude -p --output-format stream-json --verbose`, the
    `--permission-mode {plan,acceptEdits}` modes, `--no-session-persistence`,
    `--disable-slash-commands`, `--tools`/`--disallowedTools`,
    `--mcp-config`/`--strict-mcp-config`, `--add-dir`, and the `stream-json` record
    families (`system`/`assistant`/`tool_use`/`tool_result`/`result` with
    `session_id` and `usage`). The wire contract CS2-CS4 implement against. Capo
    sends no Capo-authored tool result back over the one-shot (observed-only, CS4);
    whether the CLI would accept one is an Open Question, not an assertion.
- OpenAI `codex` CLI (reference provider, already live-proven in-tree)
  - `codex exec --json --sandbox {read-only,workspace-write} --ephemeral
    --skip-git-repo-check --cd <ws>`; the parity reference for the gate posture, the
    dispatch executor structure (including the `codex_program_override` /
    `CAPO_CODEX_BIN` seam CS5 mirrors as `claude_program_override` /
    `CAPO_CLAUDE_BIN`), and the observed tool-result mapping.
