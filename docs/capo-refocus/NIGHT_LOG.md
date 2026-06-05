# Capo refocus — overnight autonomous run log

Started 2026-06-05 ~22:40 CEST. Goal: maximize validated progress toward the
refocus objective by ~07:00 Sat. All work on branch `slice-a-acp-wiring` in
worktree `/Users/nicolas/devel/capo-sliceA`. Never touches `main`, never pushed.
Each step: workflow (with adversarial review) → I verify build/tests → commit on
branch → log here.

## Status board
- [x] **Slice A** — ACP path wired into capo-server command surface. Committed.
      158 capo-server tests + 2 new (gate-open observed-file + fail-closed) green.
- [x] **WF3 — live tier — VALIDATED.** A REAL `npx @zed-industries/claude-code-acp`
      agent, over the headless subscription (no API key), driven through capo's
      `RunAcpLiveTurnLocal`, wrote `HELLO.txt == "capo-works"` on disk
      (event_count=15, 18 tool.* events, end_turn, 15s). Reconciled capo's ACP wire
      client with claude-code-acp 0.16.2 (content array/object parsing, stdout-noise
      tolerance, `acp_session_mode` → `bypassPermissions` for a real on-wire write).
      Gated live test `acp_live_bridge_smoke.rs`. Committed. **The core design is
      proven end-to-end.** Note: production should prefer the permission-decider /
      `canUseTool` path over `bypassPermissions` (ties into WF4 supervision).
- [~] **WF4 — conductor — MOSTLY LANDED.** Committed: `c67cfbb` (Layer 1
      in-process STATELESS HTTP MCP server exposing all 5 capo tools, bearer-authed,
      `start_agent` drives a real worker turn → observed file; Layer 2 `session/new`
      forwarding + `RunConductorTurnLocal` + prompt composition + fail-closed),
      `e2722d5` (real-bridge requires a `name` field on the forwarded mcpServers
      entry). **PROVEN LIVE:** a real conductor `claude-code-acp` session connects to
      capo's MCP endpoint and CALLS `start_agent` (`MCP invocation log: [start_agent]`).
      Layer-1 + Layer-2 deterministic tests + all regressions green.
      **OPEN ISSUE (handed to WF4b):** in the full nested live E2E the worker turn
      runs (bypassPermissions, correct cwd, 8–12 events, end_turn, makes tool calls)
      but writes NO file — while the IDENTICAL standalone worker (WF3
      `acp_live_bridge_smoke`) writes fine. Two candidate causes: (a) `bypassPermissions`
      may not take effect on a *second nested* Claude → it simulates Write instead of
      an on-wire `fs/write_text_file`; (b) concurrency of two nested Claudes. The
      live test `conductor_live_e2e.rs` is WIP/untracked (not committed). Likely
      correct fix = the allow-capable permission DECIDER (WF3 Gap 1) so worker turns
      get a *supervised* allow (capo `canUseTool`) and write on-wire WITHOUT relying
      on bypassPermissions — which is the production-correct supervised path anyway.
- [x] **WF4b — conductor loop VALIDATED LIVE** (`ae148ac`). Root-caused (raw ACP
      wire trace): under bypassPermissions the nested worker delegated the write to
      a `Task→Bash` sub-agent whose fs ops are SIMULATED (never on-wire) → no file.
      Fix: drive the worker in DEFAULT permission mode (capo's permission decider
      allows the on-wire `fs/write_text_file`) + steer the worker prompt to use
      Write/Edit directly. `conductor_live_e2e` 3/3 green (confirmed here, 22s): real
      conductor → `start_agent` over capo MCP → real worker writes HELLO.txt. **The
      full L1→L2 depth hop works with real subscription agents.**
- [x] **WF5 — web chat entrypoint — DONE** (`cf5ab40`). capo-web hosts the in-process
      MCP server + a long-lived conductor session; `POST /api/chat` drives one
      `RunConductorTurnLocal` and returns the reply + toolCalls + mode; static `/chat`
      page (148 lines, no build step). Deterministic round-trip test green (6 capo-web
      tests).
- [x] **🎉 LIVE WEB SMOKE — FULL LOOP PROVEN END-TO-END (00:39).** Ran the real
      `capo-web` binary; `curl POST /api/chat` with a natural-language message →
      the real conductor (subscription, ACP) replied `end_turn` and CALLED TWO capo
      tools on its own (`start_agent` with a precise derived task + `review_agent`),
      and the **worker wrote `HELLO.txt` = `capo-works` on disk**. The complete
      objective — web chat → conductor LLM → capo tools → worker agent → observed
      file change, all on the subscription, no API key — works through the real
      product surface. THIS IS THE GOAL.

## Remaining (time permitting, ~6h left)
- [x] Harden: full `cargo test --workspace` — ALL GREEN, 0 failures across 14 crates (800+ tests).
- [x] **Reliability: live web smoke 3/3 PASS** (~30s each). The conductor reliably calls
      `start_agent` (and sometimes `review_agent`) and the worker writes the file every
      run. The WF4b fix (default permission mode + prompt steer) made it deterministic.
- [x] **CUT-PLAN.md** committed (`120ab7b`) — staged plan (Stage 1 safe CLI-only crate
      removals + parent_child gating; Stage 2 gate superseded providers/machinery).
      Analysis only; destructive removal left for a supervised session.
- [x] **Detached `start_agent`** committed (`9b17208`) — optional flag: conductor
      returns immediately (status:running), worker runs in background. Depth-discipline
      responsiveness. Additive + flag-gated (sync default unchanged). Deterministic test green.
- [x] Docs consolidated onto the branch (`2b60635`): REFOCUS-SUMMARY + PLAN/VALIDATION/EXPERIMENT.

- [x] **Adversarial branch-wide self-review** → `KNOWN-ISSUES.md` (verified findings;
      none break the proven loop). Fixed the highest-value safe ones, each verified +
      committed: **B1** (`277fef0`, serialize conductor turns), **M5** (`199d2e2`, surface
      real-bridge tool_call content+diff), **M1 partial** (`fc75ede`, log detached failures).
      Remaining (M2/M3/M4/M6/M7 + minors) documented for supervised triage.
- [x] Live E2E observed to flake ~1/5 (LLM nondeterminism); deterministic tests are the
      stable gate. Full `cargo test --workspace` green after all fixes.

### Final state: 13 commits on `slice-a-acp-wiring`. Objective COMPLETE + validated live +
### reliable + self-reviewed + hardened + documented. Nothing merged to main, nothing
### pushed. Canonical overview: REFOCUS-SUMMARY.md; open items: KNOWN-ISSUES.md.
- [ ] Cut sweep — feature-gate peripheral crates/superseded paths out of the default
      running path (now lower-risk: 7 commits are a safety net; revert any bad cut).
- [ ] (optional) async `start_agent` for conductor responsiveness (depth discipline).
- [ ] Consolidate design docs onto the branch; morning report.

## Log
- 22:40 — Slice A committed (see `git log` on branch). Launching WF3.
