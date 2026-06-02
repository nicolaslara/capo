# Project Task Queue

**You edit this file.** It tells agents which workpad to load for `/next` and similar commands.

Read top to bottom. The **first unchecked** item is the active workpad unless Notes override it. Check items off when a phase is finished, not when pausing mid-phase.

## Active Now

**real-turn-loop** - Replace the fake controller with a real observe->decide->emit turn loop that drives the existing dispatch primitives, run one real workspace-write Codex adapter end-to-end behind a minimal safety floor, and extract a provider-neutral `AgentAdapter` trait. First workpad of the daily-driver harness track.

**Daily-driver harness track (server/CLI):** real-turn-loop -> (streaming-transport || tools-aci) -> safety-gates -> goal-autonomy -> depth. Derived from the systematic daily-driver review in `workpads/harness-research/daily-driver-review.md` and prepared with adversarial architecture / plan / tools-ACI review. The web UI is owned by a separate agent and is out of scope for these workpads; they deliver only the server-side streaming contract (evolving `crates/capo-web`), not the browser client.

**Recently completed:** **dashboard-webclient** - Browser dashboard/web client first slice completed with design gate, mocked server-command API, and screenshot-reviewed browser smoke on 2026-05-28.

## Workpad Queue

- [x] **research** - ACP, prior art, subscriptions, local models, memory systems, voice, tunnel/connectivity, and language stack (gate passed 2026-05-25)
- [x] **architecture** - System boundaries, module contracts, data model, security model, and technical plan (gate passed 2026-05-25)
- [x] **prototype** - Minimal e2e Capo that can spawn, track, and interact with at least one coding agent (gate passed with constraints 2026-05-25)
- [x] **features** - Product feature workpads derived from the architecture and prototype (feature gate passed 2026-05-26)
- [x] **dogfood** - Move Capo project execution into Capo itself once stable enough (dogfood gate passed 2026-05-26 for Capo-assisted development with markdown/git fallback)
- [x] **scaffold** - Align the implemented scaffold with the intended product spine before more breadth: Capo server/control plane, ACP-tracked agents, simple DB-backed project memory, minimal CLI client, deterministic e2e tests (completed 2026-05-26)
- [x] **server** - Implement the server/control plane, CLI-through-server path, mocked-agent tests, and Codex-backed proof (completed 2026-05-27).
- [x] **harness-research** - Research spike on modern coding-agent harness practice and whether ACP is enough (completed 2026-05-28).
- [x] **operator-control** - Create a human operator REPL/control surface for inspecting and steering running agents through the Capo server, with planner modes starting at `none` (completed 2026-05-28).
- [~] **goal-orchestration** - Goal-loop DESIGN source (GO0-GO14). Implementation re-sequenced onto the real-turn-loop substrate and realized by **goal-autonomy** (+ the GO2 reporting tools in **tools-aci**); closes as "design realized" after goal-autonomy. Active design, blocked implementation. See Notes.
- [x] **dashboard-webclient** - Build a browser dashboard/web client for Capo with explicit design review, accepted design, implementation, screenshot review, and visual iteration gates (completed 2026-05-28).

### Daily-driver harness track (server/CLI; web UI owned by a separate agent)

Decomposed and adversarially reviewed on 2026-05-29 from the daily-driver review. Sequence by dependency; `[~]` marks the design-source entry above, not an active queue item.

- [ ] **real-turn-loop** - Real observe->decide->emit controller turn loop driving the existing dispatch primitives; one real workspace-write Codex adapter end-to-end; provider-neutral `AgentAdapter` trait; minimal safety floor (path confinement, hard-kill, pre-write checkpoint, resource ceiling, dry-run default). **Active.**
- [ ] **streaming-transport** - Tokio streaming runtime (output deltas + stdin) + JSON-RPC framing + `Subscribe{session_id, from_sequence}` event tail + concurrent serve loop + multi-turn thread read model + typed mid-turn interrupt + the server-side SSE/HTTP contract (evolves `crates/capo-web`; does not build the web client). Depends on real-turn-loop.
- [ ] **tools-aci** - Wire the real tool path (kill the fake-only routing) + typed narrow tool I/O + edit/patch/search/test ACI quality + provenance/redaction/artifact instrumentation + the GO2 agent-reporting/evidence tools. Depends on real-turn-loop; parallel with streaming-transport.
- [ ] **safety-gates** - Wire `PermissionPolicy` + `ToolExposure` into the loop + grant read-back/revoke/expiry + a real `VerificationRunner` (runs lint/test, real exit status) + controller-owned checkpoint/rollback + liveness-aware recovery. Depends on real-turn-loop, streaming-transport, tools-aci.
- [ ] **goal-autonomy** - Implement the goal-orchestration design on the real substrate: goal/evidence/report event model + projections + lifecycle/server/read commands (M1), then continuation scheduler + evidence-gated completion auditor + reattach-after-compaction (M2, gated on safety-gates). Depends on real-turn-loop, tools-aci, safety-gates.
- [ ] **depth** - Live ACP + Claude workspace-write adapters + real memory packet/FTS5 retrieval + first OS sandbox tier + git worktree isolation + optional OTel. Differentiated per-task prerequisites.

### Connectivity + console track (new highest-priority work)

Registered 2026-06-02. The new highest-priority work. Sequence: **claude-subscription** runs in PARALLEL (no chain dependency); **connectivity-tunnel -> remote-runtime -> distributed-topology** is a strict dependency chain (each link must land in-tree before the next starts; distributed-topology carries a hard do-not-start DT0 substrate gate); **web-console** runs FULLY IN PARALLEL with the entire daily-driver harness track and the connectivity-tunnel track. Concretely: `claude-subscription || web-console || (connectivity-tunnel -> remote-runtime -> distributed-topology)`.

- [ ] **claude-subscription** - Lift Claude from a gated stub to a real subscription-backed workspace-write + chat provider at Codex parity: token-safe privileged connector, real one-shot chat + workspace-write, `stream-json` into the normalized-event route, observed-only tool-result round-trip, dispatch Claude spawn arm + `CAPO_CLAUDE_BIN`, stub tests + opt-in live smoke. Runs in parallel; no chain dependency.
- [ ] **connectivity-tunnel** - Real `ConnectivityTunnel` beyond loopback (first adapter: Tailscale) for cross-device reachability: `ExposurePolicy`, `auth_ref` handles, health/heartbeat/reconnect, opt-in anti-sleep, auditable + revocable exposure. Reachability only, strictly separate from execution. First link of the chain.
- [ ] **remote-runtime** - Real remote `RuntimeRunner` (`SshRemoteProcessRunner` + `FakeRemoteProcessRunner`) behind the same contract as `LocalProcessRunner`; git-based remote workspace materialization; lifecycle/health/reattach across a machine boundary. Depends on connectivity-tunnel.
- [ ] **distributed-topology** - Capstone integration: prove server/controller + remote runner + client as three roles on different devices over a tailnet, all-local default protected; keep-alive, resumable streaming, cross-device smoke, operator docs, auditable/revocable remote control. Deterministic-first. Depends on connectivity-tunnel + remote-runtime; hard DT0 do-not-start substrate gate.
- [ ] **web-console** - Finish `web/app` into a seven-screen terminal-native (light+dark) console; tighten live streaming-chat UX over the real ST4/ST5 event-tail + thread contract; remote-aware client over the connectivity-tunnel with reconnect/resume via `from_sequence`; offline fixture fallback. Client-side only; consumes (does not own) `crates/capo-web`. Runs fully in parallel.

## Notes

- The source-of-truth product prompt is captured in `project.md` and `workpads/prototype/spec.md`.
- Research gate passed 2026-05-25. Use `workpads/research/knowledge.md` and `workpads/research/findings/` as architecture inputs.
- Research and architecture may run in parallel only when task boundaries are independent and findings are recorded before implementation decisions.
- Favor Rust for durable controller/core work unless research shows Python ecosystem leverage is materially better for a specific subsystem.
- Python is acceptable for adapters, experiments, local-model integrations, voice pipelines, or memory-system prototypes.
- Do not start dogfooding until the prototype can persist state, show active agent state, and recover from a restart without losing the task ledger.
- Architecture gate passed 2026-05-25. Use `workpads/architecture/gate-review.md` and `workpads/architecture/prototype-plan.md` as prototype inputs.
- Prototype gate passed with constraints 2026-05-25. The local scaffold is proven with fake agents; real Codex/Claude connector proof and workpad import/update safety remain feature/dogfood blockers.
- Feature gate passed 2026-05-26. Real Codex connector proof, workpad indexing/import/proposals, dashboard/query, permissions/tools, memory/eval, voice, remote runtime, and maintainability splits are complete enough to start the dogfood workpad.
- Dogfood gate passed 2026-05-26 for Capo-assisted development with markdown/git fallback. Full unattended/source-writing dogfood remains future hardening.
- Current product correction: Capo should not expose `workpad` as a primary product concept. Existing workpad commands are transitional scaffolding for this repository's markdown planning files. The future-facing model is Capo server/controller plus clients, ACP-tracked agents, and DB-backed project/workpad/task memory records that point to markdown files and are exposed to agents through tools/context.
- Scaffold work should prefer the narrow e2e spine over breadth: inspect agents, send instructions, track state, expose requested context/tool activity, persist/recover, and export evidence. Voice, remote clients, rich dashboards, and graph/vector memory should remain planned or stubbed unless needed to prove that spine.
- Server work should make the product-spine real: a durable Capo process owns controller/state/query behavior; local CLI commands become clients of that process; agent interactions are tested deterministically with mocked agents before proving Codex behind the same boundary.
- Server milestone completed 2026-05-27: loopback server, CLI-through-server control, mocked-agent tests, mocked Codex live-run tests, and manual real Codex smoke through the running server are recorded in `workpads/server/tasks.md`.
- Harness research spike completed 2026-05-28: ACP remains the preferred agent/protocol boundary, but the best harnesses add controller-owned runtime, permission, tool instrumentation, checkpoint/recovery, context/memory, evaluation, observability, and multi-client/server layers around it. See `workpads/harness-research/knowledge.md`.
- Operator-control completed 2026-05-28. The local control loop is usable through the server boundary with default `--planner none`, tracked deterministic `capo-operator` mode, live-gated Codex, concise rendering, Markdown/code display preservation, and completion audit in `workpads/operator-control/completion-audit.md`.
- Goal-orchestration is the active controller feature after operator-control closure. It applies the harness-research lesson that Capo owns the outer loop: goals, reporting, evidence, validation, continuation, story projections, and historical reports belong in the server/controller, while provider-native goal modes are optional delegated inner loops.
- Dashboard-webclient first slice completed 2026-05-28. It builds on the existing shared dashboard/query shape through fixture data, avoids owning orchestration state, and includes design-gate docs, a mocked server-command API, static verification, browser smoke, and desktop/mobile screenshot evidence. Next dashboard work should replace the mocked API with a live Capo-server HTTP/query adapter.
- Daily-driver harness track prepared 2026-05-29 from `workpads/harness-research/daily-driver-review.md` (systematic per-dimension review; verdict: "not yet a daily driver - disciplined bones, unbuilt loop"). The six new workpads were decomposed and then improved by an adversarial workflow over five lenses (architecture-quality, plan/sequencing, red-team, tools-ACI, codebase-reality). The plan, goal-orchestration reconciliation, and full revision log are recorded in `workpads/harness-research/harness-track-plan.md`.
- Active workpad override: the active workpad is **real-turn-loop**, not the first-listed unchecked queue item. The critical path is a real controller turn loop plus one real workspace-write adapter, because the controller is currently fake-only (`crates/capo-controller` FakeBoundaryController) and no provider can edit code end-to-end today.
- goal-orchestration reconciliation: goal-orchestration stays the canonical goal-loop DESIGN (GO0-GO14). Its implementation is realized by **goal-autonomy** (goal/evidence event model, continuation scheduler, evidence-gated completion auditor, reattach-after-compaction) and **tools-aci** (the GO2 reporting/evidence tools), on the real-turn-loop substrate. It closes as "design realized" after goal-autonomy. Do not implement automatic continuation before checkpoint/rollback and verification land in safety-gates.
- Connectivity + console track registered 2026-06-02 as the new highest-priority work. Five workpads under `workpads/{claude-subscription,connectivity-tunnel,remote-runtime,distributed-topology,web-console}/`. Sequencing: `claude-subscription || web-console || (connectivity-tunnel -> remote-runtime -> distributed-topology)`. claude-subscription lifts Claude to Codex parity in parallel; the connectivity chain is strict (distributed-topology has a hard DT0 do-not-start substrate gate and must not start until connectivity-tunnel + remote-runtime + the streaming event-tail/resume have actually landed in-tree); web-console runs fully in parallel and is client-side only (consumes, does not own, `crates/capo-web`). Active workpad pointer is unchanged: **real-turn-loop** remains active; this is registration + sequencing only, not a focus switch.
- Web boundary: these server/CLI workpads deliver the server-side streaming contract (JSON-RPC/SSE event tail, evolving the in-tree `crates/capo-web` Rust bridge) and explicitly do NOT build the browser client (`web/app`, `web/dashboard`), which a separate agent owns. The contract is authoritative as a schema plus checked-in wire-snapshot tests verifiable without any web client; streaming-transport documents the Dashboard-poll -> Subscribe migration handoff for the web agent.
