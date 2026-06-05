# CUT-PLAN.md

**Project:** capo (Rust workspace) · `/Users/nicolas/devel/capo-sliceA` · branch `slice-a-acp-wiring`
**Status:** PLAN ONLY — for a supervised session. **Nothing is deleted or gated by this document.** Read-only synthesis of the validated-loop / peripheral-crate / superseded-provider / advanced-machinery analyses.
**Goal:** shrink the DEFAULT running binary path back under control while keeping the proven ACP refocus loop (and CI compilation of dormant seams) intact.

The validated loop = `capo-core, capo-state, capo-server, capo-controller` (real turn loop + `drive_acp_live_turn` + permission decider), `capo-adapters` (acp_* path), `capo-runtime` (LocalProcessRunner + worktree), `capo-tools` (wrapper tools + capo-tool registry), `capo-query` (read models), `capo-web` (`/api/chat` + `acp_mcp_http`).

---

## 1. Summary table

| Candidate | Location | Classification | LoC saved (src / +test) | Risk | Effort |
|---|---|---|---|---|---|
| **capo-eval** | `crates/capo-eval` + 1 CLI use-site | **decouple-then-remove** (CLI-local) | ~1,084 src | Low | XS (1 fn + 1 dispatch arm + 1 `use`) |
| **capo-workpads** | `crates/capo-workpads` + CLI `workpad.rs` | **decouple-then-remove** (CLI-local) | ~302 src | Low | S (mod + 7 dispatch arms; must precede/accompany voice) |
| **capo-voice** | `crates/capo-voice` + CLI `voice.rs`/`voice_render.rs` | **decouple-then-remove** (CLI-local) | ~2,196 src | Low | S (2 mods + 1 arm; cross-imports workpad) |
| **Chat one-shot adapters** (`codex_live.rs` + `claude_live.rs`) | `capo-adapters` + server bindings | **gate** (`legacy-providers`, off by default) | ~1,095 src / +~1,360 test | Med | M (cross-crate `#[cfg]`, gate Codex+Claude together) |
| **Dispatch lane** (`RunLiveProviderLocal` + `live_provider.rs`) | `capo-server` | **decouple-then-remove → gate later** | ~924 src / +~3,400 test (shared) | High | L (loop-step + state codec coupling) |
| **Recovery / orphan-reaping / liveness** | `capo-controller/lib.rs`, `reattach.rs`, `capo-state` | **gate** (off default binary) | ~598 src | Med | M (`Recover` command + restart-event invariants) |
| **Checkpoint / shadow-git** (`checkpoint.rs`) | `capo-controller` + `safety_floor.rs` | **gate** (with goal-autonomy) | ~878 src | Med | M (state `checkpoint.created` codec) |
| **Continuation scheduler** (`continuation_scheduler.rs`) | `capo-controller` + `goal_commands.rs` | **gate** (with goal-autonomy) | ~1,190 src | Med | M (`ContinueGoal` cmd + `ContinuationDecisionRecorded` event) |
| **capo-memory jobs** (`jobs.rs` extraction/staleness/FTS) | `capo-memory` + `memory_index_ingest.rs` | **gate** (keep packet builder) | ~560 src | Low-Med | M (1 ingest consumer; packet builder stays) |
| **parent_child scaffolding** (`parent_child.rs`) | `capo-controller` | **keep** (KEEP-DORMANT — L2/L3 seam) | 0 (compile-only via feature) | n/a | XS (feature wrapper only) |

Total workspace today: **~154K LoC across 14 crates** (confirmed via `wc`; `capo-tmptest` is a scratch crate not in the validated set).

---

## 2. STAGE 1 — safe, mechanical, full-suite-green

These are the **zero-dependent CLI-local removals**. The validated server/web/controller turn loop has **no** dependency on capo-eval/capo-voice/capo-workpads (confirmed: `cargo metadata` lists `capo-cli` as the ONLY reverse-dependent). The projection/event types that matter (`WorkpadTaskProjection`, `WorkpadIndexResetProjection`, `TaskOutcomeReport` projection, `TaskOutcomeReportGenerated` event) live in **capo-state**, NOT in these crates — so the persisted state schema is untouched. Build breaks at `capo-cli` only.

**Behavior change to the validated loop: none.** Only three CLI subcommand families disappear (`eval task-outcome`, `voice submit`, `workpad *`).

### Step 1a — capo-eval (independent, do first)
1. `crates/capo-cli/src/evidence.rs:5` — remove `use capo_eval::TaskOutcomeReport;`
2. `crates/capo-cli/src/evidence.rs:83` — remove the `export_task_outcome_report` fn body's `TaskOutcomeReport::from_state(...)` call (and the fn if it becomes the only consumer). The rest of `evidence.rs` is capo-core-based (`CommandIntent`, `EvidenceId`) and **stays**.
3. `crates/capo-cli/src/main.rs:59` — remove `use evidence::export_task_outcome_report;`
4. `crates/capo-cli/src/main.rs:421-422` — remove the `area == "eval" && command == "task-outcome"` dispatch arm.
5. `crates/capo-cli/Cargo.toml:17` — remove `capo-eval.workspace = true`.
6. Root `Cargo.toml:7` (member) and `:31` (path def) — remove `capo-eval`.

### Step 1b — capo-voice + capo-workpads (cut TOGETHER; workpad cannot precede voice's reference being removed)
Ordering constraint: `crates/capo-cli/src/voice.rs:15` calls `crate::workpad::start_next_workpad_task`. Remove the voice module first (or both in one commit) so no dangling intra-CLI reference remains.

**capo-voice:**
1. `crates/capo-cli/src/voice.rs:7-10` and `voice_render.rs:7` — delete files / their `use capo_voice::{...}` imports.
2. `crates/capo-cli/src/main.rs:30,31` — remove `mod voice; mod voice_render;`
3. `crates/capo-cli/src/main.rs:82` — remove `use voice::submit_voice;`
4. `crates/capo-cli/src/main.rs:282-283` — remove the `voice submit` dispatch arm.
5. `crates/capo-cli/Cargo.toml:24` — remove `capo-voice.workspace = true`.
6. Root `Cargo.toml:15` (member) and `:39` (path) — remove `capo-voice`.

**capo-workpads:**
1. `crates/capo-cli/src/workpad.rs:11` — remove `use capo_workpads::{WorkpadIndex, index_project_workpads};` and the file/module.
2. `crates/capo-cli/src/main.rs:32,83-85` — remove `mod workpad;` and imports.
3. `crates/capo-cli/src/main.rs:397-416` — remove the 7 dispatch arms (`workpad index|next|plan-next|start-next|import|propose|apply`).
4. `crates/capo-cli/Cargo.toml:25` — remove `capo-workpads.workspace = true`.
5. Root `Cargo.toml:17` (member) and `:41` (path) — remove `capo-workpads`.

### Step 1c — parent_child feature wrapper (KEEP-DORMANT, no removal)
Add a `parent_child` cargo feature in `capo-controller` and `#[cfg(feature = "parent_child")]`-gate the `mod parent_child;` + its re-exports (`lib.rs:80-84`). Enable the feature in CI only; exclude from the default binary. This removes 1,095 LoC from the **default running binary** while keeping the L2/L3 merge-gate seam compiling. No external consumer exists today (grep = 0 outside `capo-controller/src`), so this is a pure feature-wrapper with zero call-site fallthroughs needed.

**Stage-1 result:** three peripheral crates removed (CLI-only surgery), parent_child gated out of default. Run the full verification recipe (section 6) — expect green; the only behavioral delta is the three removed CLI subcommand families.

---

## 3. STAGE 2 — decoupling needed first

Each item here is **wired to a live non-turn command and/or the state event/codec schema**, so it must be gated (not silently cut) and may need cross-crate `#[cfg]` fallthroughs.

### 3a. Chat one-shot adapters → `legacy-providers` feature (off by default)
Reached only when an agent is explicitly registered with `adapter="codex"`/`"claude"` (`capo-server/src/lib.rs:343-344`); default/ACP agents never touch it — already fail-closed behind opt-in env gates.
- **Untangle first:** `claude_live.rs:35` imports `CODEX_LIVE_PREFLIGHT_OPT_IN_ENV` + `CodexLiveChatError` from `codex_live` → **gate Codex and Claude in the SAME feature**, never independently.
- Gate together behind `legacy-providers`: `mod codex_live;`/`mod claude_live;` (`lib.rs:12-13`), re-exports (`lib.rs:40-46`), `AgentAdapterHandle::{Codex,Claude}` variants + match arms (`adapter.rs:130,137,182,189-190`, ctors `:160,167`), `RealChatBinding`/`CodexChatBindings` + register arms (`lib.rs:91-213,343-370`), chat-route dispatch (`lib.rs:305-319`), env reads (`lib.rs:130-135,1138-1151`), and `real_controller.rs:164-176` (`open_codex_chat`).
- **Mandatory `#[cfg]` fallthroughs:** the trait-dispatch match in `adapter.rs:186-191` and `binding_for` in `lib.rs:305-319` must keep `Acp`/`Fake`/`ScriptedMock` arms and compile with the legacy arms absent.
- Move `tests/codex_chat.rs`, `tests/claude_chat.rs`, `tests/codex_workspace_write.rs`, `tests/claude_loop_route.rs` and the in-adapter unit tests behind the same feature.
- **Removes ~1,095 src + ~1,360 test LoC from the default build.**

### 3b. capo-memory jobs → `memory-jobs` feature (keep the packet builder)
The validated turn path uses the **minimal context packet** only (`LiveMemoryPacketRequest`, `MemoryBackend::sqlite_fts`, controller `lib.rs:18-21,182`) — exactly what the refocus plan keeps. The extraction/staleness/FTS jobs are reached only through `memory_index_ingest.rs:23` (one `MemoryJobIngestReport`).
- Gate `jobs.rs` (`lib.rs:11-15` exports) and `memory_index_ingest.rs` behind `memory-jobs`, off by default. Cleanly separable — the packet builder stays on the default path. **~560 src LoC out of default.**

### 3c. Goal-autonomy bundle → `goal-autonomy` feature (gate checkpoint + continuation + safety_floor together)
These three are interlocked through the safety floor and share the goal command surface — gate as ONE bundle:
- **Continuation scheduler** (`continuation_scheduler.rs`, 1,190 LoC): only live caller `goal_commands.rs:223`, command `ServerCommand::ContinueGoal` (`lib.rs:1688`). State event `ContinuationDecisionRecorded` (`capo-state/event.rs:108,348,475`).
- **Checkpoint / shadow-git** (`checkpoint.rs`, 878 LoC): consumed by `safety_floor.rs:32,322` as the rollback mechanism. State event `checkpoint.created` (`capo-state/event.rs:82,331,458`).
- **Untangle:** because both carry **state codec events**, gating must keep the event enum variants present in `capo-state` (codec/replay invariants) even when the producing code is gated out, OR pair this with a state-schema migration in the supervised session. Recommended: gate the *producers/commands* (`ContinueGoal`, `create_checkpoint`, `safety_floor`) but **leave the state event variants compiled** so existing journals still replay.
- **~2,068 src LoC out of default** (checkpoint + continuation), plus the safety_floor command surface.

### 3d. Recovery / orphan-reaping / liveness → `recovery` feature
Reached only via `ServerCommand::Recover` (`lib.rs:1550` → `server_core.rs:12` → `controller_routing.rs:244` → controller `recover_command*` `lib.rs:298-306`). Not on the turn loop.
- **Untangle:** `RunRecoveryKind` (`capo-state/projections.rs:171`) is bound to the restart-event model (`capo-state/lib.rs:517-699`) and exercised by `crash_recovery.rs`/`remote_crash_safety.rs`. Gate the `Recover` command + `reattach.rs` + the `recover_command_liveness_aware`/`recover_command_reaping` controller fns behind `recovery`; keep the state restart-event invariants compiled (same rationale as 3c). **~598 src LoC out of default.**

### 3e. Dispatch lane (`RunLiveProviderLocal` + `live_provider.rs`) — DECOUPLE-THEN-GATE, do LAST
**Do NOT cut in the same pass as the chat adapters.** This lane is invoked as a STEP inside the real turn loop (`turn_orchestration.rs:319`) and `live_provider.rs:95-114,479-570` carries the shared gate/preflight/credential-scan/write-mode/per-turn-artifact machinery that broad tests (`tests/live_provider.rs` 12 tests, `tests/multi_turn_edit.rs`, `tests/live_smoke.rs`) assert against. It is codex/claude-only (rejects `acp` at `:99`).
- **Untangle path:** confirm the chat adapters (3a) are dark in production, then feature-gate the launch-plan builder arms (`live_provider.rs:479-519`) **together with** the `RunLiveProviderLocal` command variant (`types.rs:287`), its codec (`transport/codec.rs:255,732`), and the loop-step (`turn_orchestration.rs:319`). This is a wide cross-crate `#[cfg]` surface — schedule as a dedicated follow-up.
- **KEEP regardless:** the dry-run fixture parsers `CodexExecAdapter`/`ClaudeCodeAdapter` (`provider_parsers.rs`, `local_subscription.rs`) are independent JSONL parsers reused elsewhere — never part of this cut.

---

## 4. KEEP list + why

| Keep | Why |
|---|---|
| **capo-core, capo-state** | Foundation + the canonical event/projection schema. Note: the workpad/eval projection + `TaskOutcomeReportGenerated` event live HERE, so peripheral-crate removal never touches persisted state. |
| **capo-server, capo-controller** (turn loop, `drive_acp_live_turn`, permission decider) | The validated loop core. |
| **capo-adapters** (acp_* path) | The live ACP adapter — the proven provider path. |
| **capo-runtime** (LocalProcessRunner + worktree) | Execution substrate of the loop. |
| **capo-tools** (wrapper tools + capo-tool registry) | Live tool surface. |
| **capo-query** (read models) | Read-side of the loop. |
| **capo-web** (`/api/chat`, `acp_mcp_http`) | Validated entry; `/api/chat` forwards `RunConductorTurnLocal`. |
| **`AgentAdapterHandle` enum + trait dispatch** (`adapter.rs:186-191`, `Acp`/`Fake`/`ScriptedMock` arms) | The shared seam; only `Codex`/`Claude` arms are gated. |
| **CodexExec/ClaudeCode fixture parsers** (`provider_parsers.rs`, `local_subscription.rs`) | Independent JSONL/dry-run parsers, reused outside the live paths. |
| **Minimal memory context packet** (`LiveMemoryPacketRequest`, `MemoryBackend::sqlite_fts`) | Exactly the "minimal context packet" the refocus plan retains. |
| **parent_child (`parent_child.rs`, 1,095 LoC)** | **KEEP-DORMANT — the intended L2/L3 depth/merge-gate seam.** Technically dead today (0 external consumers, self-contained, 12 in-file tests) but structurally load-bearing for future depth. Gate behind a `parent_child` feature so CI compiles it; exclude from the default binary. **Do not remove.** |
| **State event variants for gated machinery** (`checkpoint.created`, `ContinuationDecisionRecorded`, `RunRecoveryKind`) | Keep compiled even when producers are gated, to preserve journal replay/codec invariants. |

---

## 5. Estimated total LoC reduction & resulting crate count

**Crate count:** 14 → **11** after Stage 1 (remove capo-eval, capo-voice, capo-workpads). (`capo-tmptest` scratch crate is separate; the 10 validated crates remain, plus capo-cli and capo-memory.)

**LoC out of the DEFAULT running binary (cumulative):**

| Stage | Source LoC removed/gated-out-of-default | Notes |
|---|---|---|
| Stage 1 — crate removals | 2,196 + 1,084 + 302 = **3,582 src** | true deletion |
| Stage 1 — parent_child gate | **1,095** | compile-only in CI, out of default binary |
| Stage 2a — chat adapters | **1,095 src** (+~1,360 test) | gated |
| Stage 2b — memory jobs | **560 src** | gated |
| Stage 2c — goal-autonomy (checkpoint + continuation) | **2,068 src** | gated |
| Stage 2d — recovery/reaping | **598 src** | gated |
| Stage 2e — dispatch lane (follow-up) | **~924 src** (+~3,400 shared test) | gated, deferred |

- **Stage 1 alone:** ~3,582 LoC deleted + ~1,095 gated = **~4,677 LoC out of default**, 3 crates gone.
- **Stage 1+2 (excl. 2e):** **~8,998 src LoC** out of the default binary (3,582 deleted, ~5,416 gated), plus ~1,360 test LoC gated.
- **Stage 1+2 incl. 2e:** **~9,900+ src LoC** out of default + ~4,760 test LoC gated.

Net: the default running path drops from ~154K toward **~144K LoC**, with the remaining gated machinery still compilable in CI behind explicit features. The validated loop's own footprint is unchanged.

---

## 6. Verification recipe — run after EACH stage

```
# 1. Default build must be green (this is the "shrunk default" guarantee)
cargo build --workspace
cargo test --workspace

# 2. Gated/dormant code must still compile in CI (Stage 1c + all Stage 2 gates)
cargo test --workspace --features parent_child,legacy-providers,memory-jobs,goal-autonomy,recovery

# 3. Live smokes — the validated ACP refocus loop (the proof-of-life)
#    Run the ignored live-binary smokes that exercise drive_acp_live_turn / RunConductorTurnLocal:
cargo test -p capo-server -- --ignored live_smoke
#    Plus the web entry:
cargo test -p capo-web -- --ignored api_chat   # /api/chat -> RunConductorTurnLocal

# 4. Manual loop smoke (supervised): RegisterAgent -> StartSession ->
#    RunAcpLiveTurnLocal / RunConductorTurnLocal -> confirm ACP transcript +
#    apply_normalized_adapter_events_with_turn ingestion produces a turn.
```

**Gate criteria between stages:**
- Stage 1 → only the three removed CLI subcommand families (`eval task-outcome`, `voice submit`, `workpad *`) disappear; everything else green; no state-codec/journal change.
- Stage 2a → confirm chat adapters dark in prod before 2e; ACP path unaffected.
- Stage 2c/2d → confirm gated state event variants still replay existing journals (codec round-trip tests green).
- Stage 2e → run the FULL `tests/live_provider.rs` (12) + `tests/multi_turn_edit.rs` + `tests/live_smoke.rs` suite, since this lane shares safety infrastructure.

---

**Reminder:** this is a PLAN for a supervised session. No source is modified, gated, deleted, or committed by this document. All file:line references were validated read-only against branch `slice-a-acp-wiring`.

Relevant paths: `/Users/nicolas/devel/capo-sliceA/Cargo.toml`, `/Users/nicolas/devel/capo-sliceA/crates/capo-cli/{Cargo.toml,src/main.rs,src/evidence.rs,src/voice.rs,src/voice_render.rs,src/workpad.rs}`, `/Users/nicolas/devel/capo-sliceA/crates/capo-adapters/src/{codex_live.rs,claude_live.rs,adapter.rs,lib.rs}`, `/Users/nicolas/devel/capo-sliceA/crates/capo-server/src/{lib.rs,live_provider.rs,turn_orchestration.rs,safety_floor.rs,goal_commands.rs,server_core.rs,controller_routing.rs}`, `/Users/nicolas/devel/capo-sliceA/crates/capo-controller/src/{lib.rs,parent_child.rs,checkpoint.rs,continuation_scheduler.rs,reattach.rs,real_controller.rs,memory_index_ingest.rs}`, `/Users/nicolas/devel/capo-sliceA/crates/capo-memory/src/jobs.rs`, `/Users/nicolas/devel/capo-sliceA/crates/capo-state/src/{event.rs,projections.rs}`.