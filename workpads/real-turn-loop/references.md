# Real Turn Loop References

## Objective

Record the local and external sources that shape the `real-turn-loop` workpad.
Dated claims reflect the observed state on 2026-05-29.

## Local Architecture Sources

- `workpads/architecture/state-model.md`
  - Key facts: SQLite events are the source of operational truth and read models
    rebuild from events plus artifacts; the Restart Recovery section
    (`state-model.md:1155-1179`) designs `run.orphaned` (restart detected a
    process without an owner) and `run.recovered` (reattached or relaunched
    after restart) and fixes recovery idempotency keys to
    `(run_id, recovery_observation_kind, observed_runtime_state_hash)`,
    intentionally excluding `recovery_attempt_id`; the artifact privacy contract
    (`state-model.md:1117-1124`) requires `redaction_state = safe` or `redacted`
    for durable persistence and forbids attaching `unknown`/`contains_sensitive`
    artifacts to read models, evidence exports, or provider-smoke results.
- `workpads/architecture/protocol-provider.md`
  - Key facts: the implementation-facing Adapter Contract
    (`protocol-provider.md:160-189`) is the design source the RTL1 `AgentAdapter`
    trait realizes - `build_runtime_request`, `attach_started_process`,
    `send_turn`, `deliver_tool_result`, `cancel`, `stream_events`, `shutdown`;
    `build_runtime_request` does not spawn directly and `attach_started_process`
    observes a process the controller already started; the controller
    orchestration sequence rejects mismatched adapter/provider/runtime bindings
    before `runtime.start_requested`. This is the seam the closed Fake/Scripted
    enum currently violates.
- `workpads/harness-research/daily-driver-review.md`
  - Key facts: the lead synthesis (2026-05-29) that motivates this workpad.
    Verdict is NO (not a daily driver yet); the two most central dimensions -
    chat loop (1.0) and transport streaming (1.5) - are the lowest scoring.
    Top blocker 1 is "default chat loop is a fake-adapter echo; build a genuine
    observe->emit turn loop on a real non-fake adapter - the substrate everything
    else attaches to." Phase 0 names: replace `FakeBoundaryController` with a real
    observe->decide->emit loop, extract the adapter contract into a real trait and
    implement Codex first as a workspace-write tool-result-round-trip adapter, and
    persist per-turn artifacts keyed by `turn_id` (fix the `stdout.txt` overwrite
    bug). Confirms the only live path is `--sandbox read-only --ephemeral` parsed
    after exit, runtime is synchronous with buffer-then-cap output (a long
    successful run is misclassified as an error), and recovery bluntly marks live
    runs `exited_unknown`.

## Local Product And Implementation Sources

- `crates/capo-controller/src/lib.rs`
  - Key facts: `FakeBoundaryController` (`lib.rs:40`) is the only controller; its
    doc-comment states P3 is "intentionally fake-only." It imports concrete
    `FakeAdapterSessionRequest`/`FakeAdapterTurnRequest` directly (`lib.rs:9-13`),
    returns `FakeRunRefs` from `send_task_command` (`lib.rs:113,258`), exposes
    `open`/`open_with_permission_policy` (`lib.rs:51-60`), and recovers via the
    blunt `mark_active_runs_exited_unknown` (`lib.rs:166`). RTL2/RTL5 add
    `RealBoundaryController` mirroring this surface; RTL10 improves recovery.
- `crates/capo-controller/src/local_dispatch.rs`
  - Key facts: `LocalAdapterDispatchRunStart` is the controller-side dispatch
    start path the loop drives; relevant to RTL4's reconciliation of the loop
    with the dispatch primitives.
- `crates/capo-controller/src/fake_session.rs`
  - Key facts: the fake session-control path the loop replaces; RTL2 migrates it
    off concrete `Fake*` signatures behind the trait.
- `crates/capo-adapters/src/adapter.rs`
  - Key facts: `AgentAdapter` is a closed enum `Fake`/`ScriptedMock`
    (`adapter.rs:5-8`); `open_session`/`send_turn`/`attach_session`/`interrupt`
    take and return concrete `FakeAdapterSessionRequest`/`FakeAdapterSession`/
    `FakeAdapterTurnRequest`/`FakeAdapterTurnOutput` (`adapter.rs:147-175`); the
    turn output carries `turn_id`, `external_session_ref`, `summary`, `confidence`,
    `status`, `tool_name`. RTL1 converts this into a provider-neutral
    `AgentAdapter` trait keeping that output shape.
- `crates/capo-server/src/types.rs`
  - Key facts: the typed `ServerCommand` boundary. `SendTask`/`SteerAgent`
    (`types.rs:111-116`) are the chat/steer entry points RTL11 routes; the
    dispatch commands `PlanDispatch`/`PreflightLiveProvider`/`GateDispatch`/
    `RunDispatchLocal`/`RunLiveProviderLocal` (`types.rs:150-186`) are the
    execution substrate the loop drives. This boundary stays unchanged across the
    controller swap.
- `crates/capo-server/src/lib.rs`
  - Key facts: the command handlers - `SendTask`/`SteerAgent` at `lib.rs:464-469`
    and the dispatch commands at `lib.rs:535-826` - are the real second
    orchestration path RTL4 must drive rather than duplicate.
- `crates/capo-server/src/dispatch.rs`
  - Key facts: `dispatch_gate_for_plan` (`dispatch.rs:62`) is the gate the loop
    must reuse; `append_dispatch_run_exit`/`append_dispatch_run_exit_with_metadata`
    (`dispatch.rs:485-547`) and `AdapterDispatchExecutionProjection`
    (`dispatch.rs:401-440`) are the run-exit events/projections that RTL8's
    `TurnFinished` annotates without forking a second completion model.
- `crates/capo-server/src/live_provider.rs`
  - Key facts: `run_live_provider_local` (`live_provider.rs:357`) is the path that
    spawns the live Codex run today; `preflight_live_provider`
    (`live_provider.rs:62`) is the preflight the loop must not bypass;
    `live_execution_opt_in`/`mock_provider_output_jsonl` (`live_provider.rs:47,50`)
    are the opt-in gate and deterministic mock hook (`live_provider.rs:420,475`);
    `wait_running_with_timeout` (`live_provider.rs:573`) is the wall-clock path
    RTL7 wires its ceiling to; `normalize_policy_path` (`live_provider.rs:686`) is
    half the confinement engine RTL6 wires; `scan_artifacts_for_sensitive_markers`
    (`live_provider.rs:578`) is the credential scan RTL13 enforces.
    `CodexExecAdapter::local_launch_plan` (`live_provider.rs:438`) is what RTL9
    moves off `--sandbox read-only` onto a workspace-write profile.
- `crates/capo-runtime/src/lib.rs`
  - Key facts: synchronous local runner. `spawn_process` keys
    `run_dir = artifact_root/run_id` and reuses one `stdout.txt`/`stderr.txt`
    (`lib.rs:322-325`) - the per-turn overwrite RTL8 fixes by threading `turn_id`;
    it emits `runtime.start_requested` and records `external_pid`/`process_group`
    (`lib.rs:344-369`) - the persist-before-spawn signal RTL10 uses;
    `process_group(0)` plus `terminate_process_group` (`lib.rs:341,514`) is the
    reaper RTL6's hard kill and RTL10's orphan reaping reuse; `capped_output`
    returns `OutputLimitExceeded` (`lib.rs:33,240`), the buffer-then-cap behavior
    the review flagged.
- `crates/capo-state/src/lib.rs` and `crates/capo-state/src/event.rs`
  - Key facts: `append_event` (`lib.rs:100`) is the single persistence path the
    real controller uses exactly as the fake does; `mark_active_runs_exited_unknown`
    (`lib.rs:251`) is the blunt recovery RTL10 replaces. The `EventKind` enum
    (`event.rs:59-107`) already covers `session.*`/`run.started`/`run.exited`/
    `adapter.dispatch_*`/`tool.observation_recorded`/`tool.output_artifact_recorded`,
    and `capability.grant_created`/`grant_used`; `run.aborted` (RTL7) and
    `run.orphaned`/`run.recovered` (RTL10) are confirmed ABSENT and must be added
    with idempotency keys and projections.
- `crates/capo-tools/src/runtime_wrapper_paths.rs`
  - Key facts: `ensure_under_workspace` (`runtime_wrapper_paths.rs:77`) is the
    workspace-containment check RTL6 wires into the write path so an
    out-of-confinement write is rejected before any process runs; the file also
    holds `workspace_path`, `workspace_relative_path`, and `sanitize_path_component`
    helpers.
- `workpads/goal-orchestration/tasks.md`
  - Key facts: the authoritative design source for the goal model and
    continuation (GO1-GO14, all pending; GO0 done). The RTL safety floor's
    per-run resource ceiling is a strict subset of GO's `GoalBudget`; the
    `real-turn-loop` substrate is what goal continuation later drives. Cited, not
    duplicated.

## External Sources

No external URLs are specific to this workpad. External sources (Codex Goals,
Codex safety, ACP, and peer harness comparisons) are inherited via
`workpads/harness-research/references.md` and the `goal-orchestration` workpad,
which remain the dated source-of-record for provider behavior observed during
the harness research.
