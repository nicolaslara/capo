# Architecture Knowledge

## Objective

Record the architectural decisions that make Capo modular: each boundary should be explicit enough to implement, test, replace, and review independently.

## Status

Architecture gate not passed.

## Initial Direction

- Keep the controller, agent runtime, connectivity/tunnel, provider connector, state store, memory layer, and input surfaces separate.
- Start with local execution before remote/cloud execution.
- Start with simple durable state and human-readable workpads before advanced memory.
- Build for ACP compatibility, but verify exact protocol fit during research before making it the only agent boundary.

## Research Gate Input

Research gate passed 2026-05-25. Use `workpads/research/knowledge.md` and `workpads/research/findings/` as architecture inputs.

Key research decisions to carry forward:

- ACP should be an adapter boundary, not Capo's core domain model.
- Capo should be a Rust-first hybrid system: Rust controller, SQLite event log, markdown workpads, Python sidecars only where ecosystem leverage warrants it.
- First runtime should be local process execution with explicit capability profiles; do not claim hard sandboxing until OS/container enforcement exists.
- Subscription-backed connectors should use vendor-supported local CLIs/SDKs first; reject web scraping and private endpoint reuse.
- Memory v0 should be markdown plus SQLite; semantic/graph memory is a rebuildable v1 layer.
- Input implementation sequence should be CLI, local dashboard, mobile/PWA, then voice; voice architecture should still be first-class conversational interaction with Capo.
- Source-code architecture inspection favors controller-owned events/read models, raw adapter events mapped into normalized Capo events, local process runtime first, durable permission events, and adapter boundaries for Codex/Claude/ACP.

## User Decisions - 2026-05-25

- First target adapters: Claude Code and Codex. Architecture should treat both as first-class initial targets rather than generic examples.
- Capo should expose tools to agents. Start with easy tools, but design the tool-exposure boundary to grow.
- Capo-exposed tools should wrap existing agent tools where possible so Capo can track, instrument, audit, and eventually enforce policy around tool use.
- ACP streaming replay and restart recovery deduplication needs more research before locking the event model. Track this as an architecture task rather than hand-waving it in A2.
- Initial permissions should be simple and permissive: everything allowed for the early local prototype.
- Permission decisions still need a modular policy architecture so later versions can route decisions through static policy, user approval, or a fast security agent.
- Capo should not expose itself as an ACP agent right now. Capo should be the user's entrypoint and remain primarily a controller/client for the prototype.
- Voice should be a conversational interface to Capo for asking what agents have done, checking status/blockers, discussing next steps, and steering agents. It is not just speech-to-text input.

## A0 - Research Ingestion

Status: completed on 2026-05-25.

Architecture inputs ingested:

- Research gate summary: `workpads/research/knowledge.md`
- ACP protocol mapping: `workpads/research/findings/R1-acp.md`
- Prior-art product comparison: `workpads/research/findings/R2-prior-art.md`
- Prior-art source-code architecture: `workpads/research/findings/R2-code-architecture.md`
- Subscription connector security boundary: `workpads/research/findings/R3-subscriptions.md`
- Stack/runtime/tunnel recommendation: `workpads/research/findings/R4-R6-stack-runtime.md`
- Memory recommendation: `workpads/research/findings/R5-memory.md`
- Input and conversational voice recommendation: `workpads/research/findings/R7-input-surfaces.md`

Architecture direction:

- Use controller-owned event/state IDs and store external adapter IDs separately.
- Persist raw adapter events separately from normalized Capo events.
- Project CLI/dashboard/voice state from Capo read models, not live agent process memory.
- Implement Claude Code and Codex adapters first, with ACP as an adapter boundary rather than the Capo domain model.
- Use Rust for controller, state, runtime supervision, command handling, and audit.
- Use SQLite for operational truth and markdown workpads for human-auditable project state.
- Use local process runtime first; remote, Tailscale, SSH, container, and stronger sandboxing are later adapters.
- Start permissions with a trusted local profile for dogfooding while still routing all decisions through a modular policy boundary.
- Make Capo-exposed tools instrumented wrappers so tool calls become durable, auditable events.
- Treat conversational voice as a Capo-facing control surface over the same read models and command envelopes as CLI/dashboard.

Architecture risks:

- **Event identity and replay:** ACP `session/load`, Codex JSONL streams, Claude Code output, and Capo restart recovery can duplicate partial updates unless A2/A2a defines stable idempotency rules.
- **Adapter drift:** Codex/Claude CLI output schemas and subscription semantics can change. Adapter contracts need raw event capture, version metadata, and golden transcript tests.
- **Permission over-simplification:** All-allowed v0 can hide missing policy boundaries. Every allow decision still needs a durable decision source, scope, and audit event.
- **Tool observability gaps:** If Capo only wraps top-level CLI processes, provider-native tools may remain opaque. A5a must define what can be instrumented in v0 and where visibility is deferred.
- **Runtime safety claims:** Local process execution is controllable but not a sandbox. Documentation and UI must not imply stronger isolation than exists.
- **State/source split:** Markdown workpads and SQLite event state can diverge unless architecture defines which store is authoritative for each class of fact.
- **Voice privacy:** Conversational voice can expose sensitive status, code, and credentials through transcripts. Retention/redaction rules must be explicit before implementation.
- **UI ownership:** Dashboard or voice surfaces must not become the owner of orchestration state; they submit commands and render read models only.
- **Naming drift:** Terms like session, run, turn, task, event, checkpoint, agent, adapter, runtime, and tool must be defined before implementation to keep modules readable.

Resolved open questions:

- First concrete agent connectors: Claude Code and Codex.
- Capo modes: rejected as a Capo product model; modes belong to adapters/subagents if present.
- Capo as ACP agent/editor backend: deferred; Capo remains the entrypoint.
- Voice role: first-class conversational interface to Capo, not generic dictation.

## A1 - Boundary Contracts

Status: completed on 2026-05-25.

Decision:

- Use `workpads/architecture/boundaries.md` as the implementation-facing contract map.
- Keep all core boundaries explicit: input surface, controller, agent adapter, runtime runner, connectivity/tunnel, provider connector, permission policy, tool exposure, state store, memory layer, and evaluation layer.
- Start with static dispatch for known in-tree variants. This keeps the initial Rust scaffold readable and makes missing variant handling visible at compile time.
- Defer dynamic dispatch/plugin loading until third-party extension or runtime-loaded adapters are real requirements.

Naming decisions:

- `AgentAdapter`, not `AgentProtocolAdapter`, for the broad boundary that covers Codex, Claude Code, ACP, and fake test adapters.
- `RuntimeRunner` for process/container/remote execution.
- `ProviderConnector` for non-secret provider/auth/usage metadata behind adapters.
- `CapabilityProfile` plus `PermissionPolicy` for scopes and decision source.
- `ToolExposure` for Capo-exposed tools and instrumentation wrappers.
- `CommandEnvelope` for normalized commands from CLI/dashboard/mobile/voice/API.

Implementation implications:

- A first code scaffold should include fake adapter/runtime/tool/policy variants for e2e tests before real Claude/Codex integration.
- Controller tests should use static fake variants to prove dispatch, event append, read model projection, and restart recovery.
- Adapter outputs are inputs, not persistence truth; the state store owns normalized events and read models.
- UI and voice must depend on read models and command envelopes, not live process state.
- Review pass added fake/static variants for each scaffold boundary, an explicit adapter tool-call loop, and a runtime/tunnel separation where Tailscale and SSH stay in connectivity instead of runtime execution.

## A2 - State Model And Event Log

Status: completed on 2026-05-25.

Decision:

- Use `workpads/architecture/state-model.md` as the implementation-facing state architecture artifact.
- SQLite is the prototype source of truth for operational state: projects, agents, sessions, runs, turns, items, tool calls, permissions, evidence, evaluations, checkpoints, commands, projections, raw-event indexes, and recovery attempts.
- Markdown workpads remain the human-readable planning source. Capo stores workpad paths and observed status snapshots, but its own scheduling state is `capo_execution_status`.
- Large raw streams, logs, prompts, tool inputs/outputs, diffs, reviews, checkpoints, and summaries are file artifacts referenced by SQLite rows.
- Read models are rebuildable from events and artifacts; UI/dashboard/mobile/voice surfaces use read models and Capo event sequence IDs, not adapter IDs, for dedupe.

Restart and replay direction:

- Startup recovery replays unprojected events, scans live-looking sessions/runs, probes runtime and adapter health, emits recovery events, and only then serves input surfaces.
- Recovery is append-only. It emits new `run.recovered`, `run.orphaned`, `run.exited`, `recovery.started`, and `recovery.completed` events instead of editing history.
- Pending permission requests survive restart and remain visible through `PermissionQueue`.
- Generic streaming dedupe is defined conservatively; ACP-specific `session/load` replay and partial-stream identity rules remain A2a.

Review findings accepted:

- Added `tool.result_delivered` so the fake-agent e2e flow can prove tool-result delivery back into adapters.
- Added concrete prototype tables for evidence, evaluations, memory refs, permission requests, and recovery attempts.
- Replaced the undefined recovery epoch idea with explicit recovery attempt records and idempotency keys.
- Split observed workpad status from Capo execution status to preserve markdown authority.
- Added interrupt and stop event families needed by the prototype minimum.
- Updated routing docs so future agents load `state-model.md` after A2.

## A2a - ACP Streaming Replay And Dedupe

Status: completed on 2026-05-25.

Primary recommendation:

- Treat ACP as an adapter input stream, not as Capo's UI/read-model truth.
- Prefer ACP `session/resume` over `session/load` when Capo already has complete local event history and the agent supports resume.
- Use ACP `session/load` for foreign session import, repair/reconciliation, or agents that cannot resume.
- Persist raw ACP updates in replay batches, stage candidate normalized records outside the projecting event log, reconcile them, and append only accepted import/update events or replay marker events.
- UI surfaces consume Capo sequence/read-model watermarks only; they never render raw ACP replay directly.

Identity and dedupe decisions:

- ACP `toolCallId` is a stable timeline key within an ACP session and is the main reliable tool-call dedupe anchor.
- ACP plan updates are complete replacement projections; keep event history but render the latest plan.
- Stable ACP v1 message chunks do not have stable `messageId` in the main schema. Capo must use content hashes, surrounding anchors, and boundary confidence for message replay dedupe.
- `_meta.messageId` is not generic ACP identity. It can only be used as adapter-specific, opt-in heuristic evidence when a concrete adapter documents and tests it.
- Replay duplicate, ambiguous, attach, and replay-completion events are explicit so restart recovery and ACP replay remain auditable.

Residual risk:

- Message-boundary dedupe without stable message IDs is inherently medium confidence. Prototype ACP fixtures must cover same-history `session/load`, foreign import, consecutive same-type chunks, plan replacement, repeated tool updates, and cancellation with pending permissions.
- If ACP stabilizes message IDs later, Capo should support them opportunistically without making them required for correctness.

## A3 - Capability And Permission Model

Status: completed on 2026-05-25.

Decision:

- Use `workpads/architecture/capability-permissions.md` as the implementation-facing capability and permission model.
- Keep `CapabilityProfile` as data and `PermissionPolicy` as the decision boundary.
- Start the prototype with `AllowTrustedLocalProfilePolicy`, but route every action through the same request, decision, grant, grant-use, expiry, and revocation event flow that stricter policies will use.
- Use static dispatch for policy variants: trusted local profile, static, user approval, security-agent, and fake test policy.
- Store capability profiles, grants, and grant-use audit records in SQLite projections derived from append-only events.

Scope and policy coverage:

- A3 covers shell, filesystem, git, network, browser, Capo tools, adapter/provider-native tools, MCP servers/tools, secrets/subscription material, and voice transcript/approval scopes.
- Trusted local dogfooding is audit-only convenience, not a sandbox claim.
- Credential material and raw voice transcripts remain privileged scopes and are not retained or read by default.

ACP mapping:

- ACP `allow_once` maps to an allow decision with once/turn-scoped grant.
- ACP `allow_always` maps to an allow decision with scoped durable grant only when the profile permits durable remembered grants; otherwise it is downscoped to session scope for the prototype.
- ACP `reject_once` maps to a one-request rejection with no allow grant.
- ACP `reject_always` maps to a scoped durable deny rule/grant record.
- ACP `session/cancel` while a permission request is pending maps to Capo decision `cancel` and ACP outcome `cancelled`.

## A4 - Runtime And Tunnel Plan

Status: completed on 2026-05-25.

Decision:

- Use `workpads/architecture/runtime-tunnel.md` as the implementation-facing runtime and connectivity model.
- Keep runtime execution and tunnel/connectivity as separate boundaries.
- `RuntimeRunner` owns local/remote/container process lifecycle: prepare, start, stdin, output, interrupt, terminate, kill, health, cleanup.
- `ConnectivityTunnel` owns endpoint resolution, channel reachability, identity/exposure metadata, and tunnel health. It does not own process handles or session truth.
- Prototype runtime is `LocalProcessRunner` plus `FakeRuntimeRunner`; prototype connectivity is `LocalLoopbackTunnel` plus `FakeTunnel`.
- Remote process, SSH, Tailscale, reverse tunnel, container, devcontainer, and Linux sandbox profiles are modeled but deferred.

Safety and recovery:

- Local process execution is a controllable trusted runtime, not a sandbox.
- Subscription-backed CLIs run as privileged local processes with environment allowlists and redacted output artifacts; Capo does not read provider session credentials.
- Runtime starts are event-sequenced with `runtime.start_requested` before launch, then `runtime.process_started` or `runtime.process_start_failed`; if append fails after spawn, recovery treats the live process as orphaned and cleans it up with evidence.
- Restart recovery probes stored runtime process refs and maps them to `run.recovered`, `run.orphaned`, or `run.exited`. Simple attach recovery marks the same run recovered; `recovery_of_run_id` is for new relaunch/retry runs.
- Public exposure through reverse tunnels or Tailscale Funnel is out of prototype scope and must require explicit permission plus audit events later.

Implementation implications:

- First Rust scaffold should include fake/local runtime and fake/local-loopback tunnel variants.
- Runtime/connectivity tables and events are added to `state-model.md` so dashboard/voice can inspect execution placement without live process ownership.
- Connectivity resolution uses owner-typed `ResolvedEndpoint` records so dashboard/API/input-surface endpoints do not pretend to belong to a runtime target.
- PTY is deferred until Claude Code/Codex adapter tests prove it is required; pipe stdio is the first implementation path.

Review findings accepted:

- Added explicit runtime start request/start failed event ordering to avoid ambiguous spawned-but-not-persisted processes.
- Replaced runtime-only endpoint records with owner-typed resolved endpoints.
- Aligned `boundaries.md` with the A4 contract and fake tunnel variant.
- Clarified same-run restart recovery versus new retry/relaunch runs.
- Replaced undefined environment profiles with `env_policy_json` for the prototype.
- Changed runtime launch vocabulary from command/args to program/argv plus launch mode.

## A5 - Protocol And Provider Plan

Status: completed on 2026-05-25.

Decision:

- Use `workpads/architecture/protocol-provider.md` as the implementation-facing adapter/provider model.
- First concrete adapters are `CodexExecAdapter` and `ClaudeCodeAdapter`; `FakeAdapter` remains the first e2e test adapter.
- ACP remains an adapter boundary where Capo is the ACP client. Capo does not expose itself as an ACP agent/editor backend for the prototype.
- Provider connectors store non-secret auth/provider/usage metadata and revocation instructions; they do not execute turns directly in v0.
- Subscription-backed Codex and Claude Code connectors are local-only user-owned integrations. Hosted/shared Capo must use API, WIF, enterprise access-token, or vendor-approved organization flows.

Adapter choices:

- Codex prototype path uses `codex exec --json` through `LocalProcessRunner`, with Codex JSONL treated as adapter input and Capo's event log as truth.
- Claude prototype path uses `claude -p --output-format stream-json` through `LocalProcessRunner`; `--bare` is reserved for deterministic API-key/scripted mode, not subscription OAuth, unless docs and smoke tests prove otherwise.
- ACP prototype support uses stdio JSON-RPC and maps initialize/session/prompt/update/permission/cancel flows into Capo events, with A2a replay/dedupe and A3 permission mapping.

Safety and productization:

- Capo must not read, copy, persist, log, or sync vendor OAuth tokens, browser cookies, keychain entries, `.credentials.json`, ChatGPT browser storage, or Playwright storage state.
- `credential_scope = user_local_subscription` implies `productization_allowed = local_only`.
- User-local subscription connectors use `auth_ref_kind = vendor_cli_default_login` with empty `auth_ref`; secret handles are reserved for API/WIF/enterprise connectors.
- `authorize_connector_use(...)` rejects local-only subscription connectors in hosted/shared deployment before any runtime request is created.
- Browser automation and reverse-engineered private endpoints remain out of scope.

Review-sensitive residual risks:

- Exact Codex JSONL and Claude stream JSON field mapping needs fixture captures before implementation claims high confidence.
- Native tool-call observability may be incomplete in both first adapters; A5a must define observed-only versus instrumented tool states.

Review findings accepted:

- Split adapter launch into `build_runtime_request(...)` and `attach_started_process(...)` so the controller/runtime own process start ordering.
- Made `Agent` the owner of runtime/provider/capability binding; `AdapterConfig` is a reusable integration template.
- Typed `auth_ref` so subscription connectors cannot point to vendor credential files or keychain records.
- Added an explicit connector-use authorization gate and denial event before runtime launch.
- Split ACP client calls from ACP client handlers.
- Marked local absolute CLI paths as diagnostic observations only.

## A5a - Capo Tool Exposure And Instrumentation

Status: completed on 2026-05-25.

Decision:

- Use `workpads/architecture/tool-exposure.md` as the implementation-facing tool registry and instrumentation model.
- Treat Capo tools as controller capabilities rather than agent identities.
- Start with six Capo-owned tools: `capo.task_status`, `capo.agent_status`, `capo.session_summary`, `capo.workpad_read`, `capo.evidence_record`, and `capo.capability_request`.
- Use static dispatch for tool exposure variants: Capo registry, runtime wrappers, adapter-native observer, provider-native observer, MCP bridge, and fake tests.
- Build fake and Capo-owned tools before shell/file/git wrappers so the controller, permission, state, artifact, read-model, and adapter result-delivery path can be tested without relying on Codex/Claude native tool visibility.

Tool boundary:

- Fully governed tools are tools Capo executes through a registered handler or wrapper.
- Adapter-native and provider-native tools are `observed_only` unless Capo receives structured lifecycle evidence or executes the backing action itself.
- ACP `session/update` tool calls are observations; ACP `fs/*` and `terminal/*` requests are executable only when Capo advertises the matching client capability and routes through wrappers.
- ACP terminal requests route through `RuntimeRunner`, not direct shell execution.
- ACP filesystem requests route through canonicalized workspace file wrappers, not raw filesystem access.
- MCP is deferred as a publication/bridge surface over `ToolExposure`, not a second internal tool registry.

State implications:

- Added `ToolDefinition`, `ToolInvocation`, and `ToolObservation` records to separate tool catalog, execution projection, and partial native-tool observations.
- Added tool definition, invocation, observation, output artifact, authorization, and instrumentation downgrade events.
- Added `ToolCatalogReadModel` and read-model requirements for instrumentation confidence in agent/session/evaluation views.
- Added explicit state read/write scopes so status and summary tools cannot use broad tool-invoke grants as implicit access to all Capo state.
- Added actor, subject, permission decision, grant-use, and correlation fields to tool invocation records.
- Clarified that `tool.call_requested` owns timeline creation, permission events own authorization, and `tool.invocation_started` owns execution projection creation.
- Set the v0 ACP default to advertise no `fs` or `terminal` client capability until backing wrapper tools and tests exist.

Residual risks:

- Exact Codex and Claude native-tool visibility remains medium confidence until fixture captures show which structured fields are exposed.
- Shell/file/git wrappers are intentionally deferred; prototype tests must not claim runtime enforcement until wrappers exist.

## Architecture Gate

Status: not passed.

Required evidence:

- Boundary contracts.
- State/event model.
- Capability model.
- Runtime/tunnel plan.
- Protocol/provider plan.
- Memory architecture.
- Prototype task plan.

## Open Questions

- Should the core process be a long-running server from day one, or a CLI that later grows a daemon?
- Should the first UI be TUI, web dashboard, or both?
- How should low-confidence ACP message-boundary matches be surfaced in dashboard/review UX without creating noisy false alarms?
- What is the exact vocabulary for `project`, `agent`, `adapter`, `runtime`, `session`, `run`, `turn`, `task`, `event`, `item`, `tool_call`, `artifact`, and `checkpoint`?
- Which data belongs only in SQLite, which belongs in markdown workpads, and which is mirrored between them?
