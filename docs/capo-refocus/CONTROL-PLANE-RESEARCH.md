# Control-plane research — can capo own ALL agent orchestration?

**The problem.** capo is the control plane only for agents it spawns via `start_agent`
(capo-driven `claude-code-acp` ACP sessions). When a coding agent uses its OWN
sub-agents (the `Task` tool, Claude Agent SDK sub-agents, Claude Code workflows, bg
agents), those run *inside* the nested `claude` process and are invisible/unsteerable
to capo. We want capo to be able to see/steer ALL orchestration.

**Two candidate paths** (we may need both):

- **Path 1 — Lock down the agent; capo owns orchestration.** The agent(s) we drive get
  ONLY capo tools — no native `Task`/sub-agents/workflows, and file/shell I/O gated
  through capo. capo itself implements fan-out/workflows/orchestration (it already does,
  via `start_agent`). Enforcement, not just prompt-steering.
- **Path 2 — Intercept & proxy ALL tool calls.** Interpose on every tool call an
  ACP-started agent makes — *including its sub-agents'* — and route them through capo, so
  even Claude-spawned sub-agents become capo-controlled. Requires understanding how Claude
  Code / the Agent SDK dispatch tools internally and whether there's an interposition
  point that sees sub-agent tool calls.

**Questions to answer for each path: Can it be done? What are the limitations? What are
the costs?** — backed by working PROTOTYPES before any full implementation.

## References (gitignored, `references/`)
- `claude-code-acp/` (`@zed-industries/claude-code-acp@0.16.2`) — the ACP bridge capo spawns.
- `claude-agent-sdk/` (`@anthropic-ai/claude-agent-sdk@0.2.44`) — the engine. `sdk.mjs`
  (thin), `sdk.d.ts`/`sdk-tools.d.ts` (types), `cli.js` (~7560 lines, bundled — the real
  tool/sub-agent dispatch; de-obfuscation target for Path 2).
- `acp-sdk/` (`@agentclientprotocol/sdk@0.14.1`) — the ACP protocol lib.

**Confirmed levers exist** (grep of claude-code-acp dist): `disableBuiltInTools`,
`allowedTools`/`disallowedTools`, `permissionMode`, `canUseTool`, `hooks`,
`settingSources`, `mcpServers`, and `Task`.

## Plan / phases (iterate until knowledge is sufficient)

### Phase 1 — Feasibility investigation (read the references deeply)
- **P1 levers** (Path 1): exactly what `disableBuiltInTools` removes; can a session run with
  ONLY MCP (capo) tools + no built-ins and still function? How `allowedTools`/`disallowedTools`/
  `permissionMode`/`canUseTool` compose. Config recipe + limitations.
- **P2 internals** (Path 2 — THE crux): how does the `Task`/sub-agent tool execute? Do
  sub-agent tool calls pass through the SAME `canUseTool` and/or `hooks` (PreToolUse)
  interception point as top-level calls, or are they internal/opaque? Is there ANY hook that
  sees sub-agent tool calls? De-obfuscate `cli.js` around Task/subagent/canUseTool/hook
  dispatch.
- **ACP capability**: does ACP model sub-sessions? Could capo run sub-agents as separate ACP
  sessions if it owned spawning? What control surface do `session/new` mcpServers + canUseTool
  give us.
- **External reality**: Anthropic Claude Agent SDK docs on subagents, `canUseTool`, hooks
  (PreToolUse/PostToolUse), `allowedTools`, `disableBuiltInTools`; confirm vs source.

### Phase 2 — Prototypes (prove the uncertain claims with the real SDK on the subscription)
- **Proto-1 (Path 1):** spawn `claude-code-acp` with `disableBuiltInTools` + a single MCP
  tool; prove the agent can ONLY use capo tools (no Bash/Write/Task).
- **Proto-2 (Path 2 crux):** drive a turn that uses `Task`/a sub-agent with `canUseTool`
  + a PreToolUse hook logging EVERY tool call; observe whether sub-agent tool calls are
  intercepted. This single result decides Path 2's feasibility.
- **Proto-3 (conditional):** if Proto-2 shows sub-agent calls are interceptable, prototype
  routing them through a capo-style proxy (e.g. all tools as MCP/canUseTool-mediated).

### Phase 3 — Synthesis
Feasibility matrix: for Path 1 and Path 2 — Can? / Limitations / Costs / Recommendation
(and whether a hybrid is best). Decide what to implement.

## Status log
- (start) References cloned; levers confirmed present. Launching Phase 1 investigation.


---

# Phase 1 findings (full)

The key citations check out. The critical confirmation: `acp-agent.js:575` destructures only `{ signal, suggestions, toolUseID }` — `agentID` is indeed dropped today, and `sdk.mjs:21` shows the SDK passes `agent_id` in the `can_use_tool` control request. The synthesis is well-grounded. Producing the Phase-1 findings.

---

# PHASE-1 FINDINGS — capo Orchestration Ownership: Path 1 vs Path 2

## Context recap
capo is an ACP client spawning `claude-code-acp@0.16.2`, which drives `@anthropic-ai/claude-agent-sdk@0.2.44`. Today capo sees only the sessions it spawns; an agent's *internal* sub-agents (native `Task`/`Agent` tool) are opaque. Two candidate paths to make capo own ALL orchestration:
- **PATH 1** — lock the agent to capo (MCP) tools only; no native Task/subagents/built-ins; file/shell gated through capo.
- **PATH 2** — intercept & proxy every tool call, *including sub-agents'*, so sub-agents become capo-controlled.

A cross-cutting fact constrains both (Investigation C): **ACP has no sub-session/parent-child primitive.** `SessionId` is flat and opaque (`acp-sdk/.../types.gen.d.ts:2017-2024`); `RequestPermissionRequest` carries only `sessionId`, no sub-agent attribution (`types.gen.d.ts:1648-1671`); claude-code-acp deliberately strips `isSidechain` sub-agent activity from `session/list` and history replay (`acp-agent.js:241,539`). Therefore **neither path can turn a native sub-agent into a separate capo-controlled ACP session.** The only way to get a first-class capo session per sub-agent is to forbid native Task and force the model to call capo's own `start_agent` MCP tool (a Path-1 posture).

---

## 1. FEASIBILITY MATRIX

### PATH 1 — Lock to capo-only tools

| Dimension | Assessment |
|---|---|
| **CAN IT BE DONE?** | **YES (high confidence).** Achievable through levers already plumbed by the bridge — no SDK fork required. Two composable mechanisms: (a) coarse ACP flag `_meta.disableBuiltInTools` which forces a blanket `disallowedTools` list including `Task` (`acp-agent.js:838,852-855`); (b) fine-grained SDK options forwarded verbatim via `_meta.claudeCode.options` and spread into the SDK query (`acp-agent.js:770,780`). The recommended surgical form: `tools:[]` (disable all built-ins) + capo `mcpServers` + `disallowedTools:["Task","Agent",...]` + `permissionMode:"default"` (forced anyway at `:768/:788`) + `canUseTool` as final gate (`:789`). `disallowedTools` is the only true capability-removal lever (a bare name strips the tool from model context — `sdk.d.ts:516-520`; corroborated by docs + issue #115). |
| **LIMITATIONS** | • `allowedTools` is **NOT** a sandbox — auto-approve only; unlisted tools fall through to mode/`canUseTool` (`sdk.d.ts:496-499`; docs `<Warning>`; issue #115). Security must come from `disallowedTools`/`tools:[]`/`canUseTool`-deny, and never `bypassPermissions`. • `disableBuiltInTools` is **blunt** — it also unregisters capo's own `acp` fs/terminal bridge (`acp-agent.js:748`), so you must re-supply file/shell via a capo MCP server or avoid the flag and use `tools:[]` instead. • **Settings-leak risk:** bridge hardcodes `settingSources:["user","project","local"]` (`acp-agent.js:777`) and sets no `strictMcpConfig`, so an ambient `.claude/settings.json` / `.mcp.json` / `CLAUDE.md` in cwd could re-introduce tools/permissions/MCP servers. Believed overridable via `claudeCode.options.settingSources:[]` (spread-before-override ordering at `:780`) — **unproven**. • Removing native tools means capo must reimplement Read/Write/Edit/Bash/Glob/Grep as MCP tools (the dispatch already exists in `tools.js` over the ACP wire). |
| **COSTS** | • **Eng effort:** Low–moderate. No fork if capo passes everything via `_meta`. Modest work to re-wrap fs/terminal as a dedicated capo MCP server (logic exists). • **Fragility vs Claude updates:** Low. Relies on stable, documented levers (`disallowedTools`, MCP, `canUseTool`). The one fragile dependency is the `settingSources` override ordering and the `Task`→`Agent` tool rename (v2.1.63; 0.2.44 likely still emits `Task`, but lists must cover both). • **Perf:** Every file/shell op round-trips through capo MCP (one extra IPC each) — same order as Path 2 but only for the work the conductor itself does. • **Loss of native capability:** **High and total** — no native Task, Skill, WebSearch/WebFetch, Glob/Grep, parallel native subagent fan-out. capo must provide every capability it wants the agent to have. |

### PATH 2 — Proxy every tool call, including sub-agents'

| Dimension | Assessment |
|---|---|
| **CAN IT BE DONE?** | **PARTIAL → likely YES for control, NO for true per-sub-agent sessions.** Code evidence is strong that sub-agent tool calls traverse the **same** interception points: the subagent runner `Wy` (`cli.js:2289`) re-enters the **same** turn-loop `iR` carrying the **same** `canUseTool`; the Task handler passes the top-level callback down wrapped but intact (`cli.js:2333`, wrapper `ACY` at `:2324`); the universal per-tool gate at `cli.js:3410` fires PreToolUse + `can_use_tool` + carries `queryDepth`/`parent_tool_use_id` for nested calls; and the SDK control request tags each `can_use_tool` with `agent_id` (`sdk.mjs:21`), surfaced as `CanUseTool` option `agentID` ("sub-agent's ID", `sdk.d.ts:119`). **However**, external docs/issue-tracker contradict the optimistic reading for the *settings.json hook* path (issue #34692, CLOSED not-planned: settings.json PreToolUse/PostToolUse do NOT fire for subagent tool calls; #26923: PreToolUse can't reliably block Task). Whether **SDK-callback** `canUseTool`/PreToolUse (capo's mechanism) fires for sub-agent inner calls in **0.2.44** is the unresolved crux. And regardless: sub-agents remain logical children inside one ACP session — **never** separate capo sessions (Investigation C). |
| **LIMITATIONS** | • **No process/session isolation** for sub-agents — they run in-process as sidechains (`cli.js:2289,3199`); no SDK option spawns a Task as its own ACP session. • **Bridge discards `agentID` today** — `acp-agent.js:575` destructures only `{signal,suggestions,toolUseID}`, so even when the SDK supplies sub-agent identity, capo can't currently attribute the call. Small patch to forward it into `requestPermission._meta`. • **"ask"-only trigger** — `canUseTool`/`requestPermission` is consulted only when the rule engine yields `behavior:"ask"`. Anything auto-allowed (`allowedTools`, `acceptEdits`, `bypassPermissions`) short-circuits and capo never sees it (`acp-agent.js:631`). To proxy *everything*, capo must use `default` mode and minimal allow-lists. • **PreToolUse in bridge is observe-only** (`continue:true`, `tools.js:588`) and doesn't forward to capo today. • **Visibility ≠ control:** even with control gating, streaming the sub-agent's *intermediate* steps to capo depends on `parent_tool_use_id`-tagged messages reaching the host, which the bridge currently filters (`acp-agent.js:539,1065`). |
| **COSTS** | • **Eng effort:** Low–moderate for control *if* the SDK-callback path fires for sub-agents (then: patch bridge to forward `agentID`+`parent_tool_use_id`; forward PreToolUse to capo; configure no-auto-allow mode). **High** if it does NOT fire — then Path 2 collapses into Path 1 (ban Task) or requires an SDK fork. • **Fragility vs Claude updates:** **High.** Depends on minified-internal behavior (`Wy`/`iR`/`ACY`/gate at `:3410`) and on `agent_id` propagation that docs do not firmly guarantee and the tracker partly contradicts. Internals can change between SDK versions silently. • **Perf:** **Highest** — every nested tool call in deep/fan-out sub-agent trees incurs a capo round-trip; multiplies with tree size. • **Loss of native capability:** **Low** — native tooling and subagents stay; capo merely observes/gates. This is Path 2's main advantage. |

---

## 2. THE SINGLE MOST IMPORTANT EMPIRICAL UNCERTAINTY (per path)

- **PATH 1:** Does `claudeCode.options.settingSources:[]` (+ `strictMcpConfig:true`) passed through `_meta` actually **override** the bridge's hardcoded `settingSources:["user","project","local"]` (`acp-agent.js:777`) and neutralize a hostile `.claude/settings.json` / `.mcp.json` in cwd? This is the make-or-break for lockdown *integrity* — if it doesn't override, ambient settings can silently re-introduce tools/permissions and the sandbox leaks. (Secondary: confirm `tools:[]` + capo MCP yields a working agent and that banning `Task`/`Agent` truly prevents any native subagent spawn in 0.2.44.)

- **PATH 2:** Does the **SDK-callback** `canUseTool` (and/or SDK-registered PreToolUse) **actually fire for a sub-agent's inner tool calls** in 0.2.44, with `agentID`/`agent_id` populated — and does returning `deny` actually block them? Code says yes; docs+tracker (#34692 closed not-planned, #26923) cast doubt for at least the settings.json path. Everything in Path 2 hinges on this single observation.

---

## 3. PROTOTYPE SPECS

All run against the real subscription, capo (or a minimal ACP client harness) spawning `claude-code-acp@0.16.2` over stdio with SDK 0.2.44. Each instruments the bridge by adding `console.error` logging (read-only references are cloned; copy to a writable scratch dir for instrumentation).

### Proto-1 — Path-1 lockdown integrity (settings leak + Task removal)
- **Setup:** Clean scratch cwd. Plant a *hostile* `.claude/settings.json` granting an extra permission/tool (e.g. allow `Bash`) and a `.mcp.json` adding a rogue MCP server. Spawn `session/new` with `_meta.claudeCode.options = { tools:[], mcpServers:{capo:{...}}, disallowedTools:["Task","Agent","Skill","SlashCommand","WebSearch","WebFetch","Read","Write","Edit","Bash","BashOutput","KillShell","Glob","Grep","NotebookEdit","TodoWrite"], allowedTools:["mcp__capo__*"], settingSources:[], strictMcpConfig:true }`. capo MCP server exposes `capo_read`/`capo_bash`.
- **Run:** Prompt the conductor: (a) "list your available tools / read file X / run command Y"; (b) "spawn a subagent using Task to do Z".
- **PASS:** Agent's tool list contains ONLY `mcp__capo__*`; it reads/runs exclusively via capo MCP; no `Bash` available despite hostile settings; rogue `.mcp.json` server absent; no Task tool present and no subagent created (no `SubagentStart`, no nested tool calls reach capo).
- **FAIL:** Agent can invoke `Bash` or the rogue MCP server (settings override didn't take → must patch bridge to pass `settingSources:[]`/`strictMcpConfig`), or a Task tool remains callable.

### Proto-2 — **Path-2 crux: does the interception point see sub-agent tool calls?** (highest priority)
- **Setup:** Spawn a normal `claude-code-acp` session via capo (built-ins enabled, native Task allowed). Instrument `acp-agent.js:574-575` to log the FULL `canUseTool` options object (not just the destructured subset) — capture `agentID`/`agent_id`, `toolUseID`. Also register an SDK-callback `PreToolUse` hook (via `claudeCode.options.hooks`) that logs `(agent_id, agent_type, tool_name, tool_use_id)`. Use `permissionMode:"default"`, no broad `allowedTools` (so everything routes to "ask").
- **Run:** Prompt: "Use the Task/Agent tool to spawn a subagent that (1) reads file A, then (2) runs bash command B." Ensure the inner steps are tools that require permission.
- **PASS:** capo receives a `requestPermission` / `can_use_tool` for **each** inner tool (Read, Bash), with `agentID`/`agent_id` **populated** (non-null) for inner calls and null/absent for the top-level Task call; PreToolUse hook logs the inner `(agent_id, tool_name)` pairs; returning `deny` for an inner call **blocks** it. ⇒ Path 2 control is real.
- **FAIL:** Only the top-level `Task` call reaches capo; inner Read/Bash never fire `canUseTool`/PreToolUse (matches issue #34692 for the SDK path). ⇒ Path 2 unviable via documented/SDK-callback levers in 0.2.44 → must ban Task (collapse to Path 1) or fork.

### Proto-3 — Path-2 auto-allow short-circuit (config requirement)
- **Setup:** Same as Proto-2.
- **Run:** Run twice — (a) `permissionMode:"default"`, minimal allow-list; (b) add an `allow` rule / `acceptEdits` for the inner tool type.
- **PASS:** In (a) every inner tool → ask → capo; in (b) inner tools auto-approved and capo NOT consulted. Confirms the "ask-only" limitation and that Path 2 must avoid auto-allow/bypass to proxy everything.

### Proto-4 — Path-2 nested-step visibility (visualization, separate from control)
- **Setup:** Same session as Proto-2; instrument `acp-agent.js:1124-1183` (`tool_call`/`tool_call_update` emission).
- **Run:** Same Task-with-2-inner-tools prompt.
- **PASS:** capo receives `tool_call` ACP notifications for the inner Read/Bash (i.e. `parent_tool_use_id`-tagged messages stream to host). **FAIL:** capo sees only the top-level Task tool_call and the final result — confirming the bridge filters sidechain steps (`:539`) and that nested *visualization* needs additional bridge work beyond control.

---

## 4. RECOMMENDATION (given current evidence)

**Recommend PATH 1 as the primary architecture, with a Path-2 observability layer as a hybrid complement — but gate the final decision on Proto-1 and Proto-2.**

Rationale:
1. **Path 1 is the only path that satisfies the stated goal in full.** The goal is for capo to *own ALL orchestration* and ideally treat sub-agents as first-class controllable entities. ACP fundamentally cannot model native sub-agents as sessions (Investigation C), and the SDK runs them in-process with no isolation option (Investigation B). The *only* way to get a real capo-controlled session per sub-agent is to **ban native `Task`/`Agent` and force the conductor to call capo's `start_agent` MCP tool** — which is intrinsically Path 1. Path 2 alone can never deliver per-sub-agent sessions; at best it gives capo a veto + telemetry over an opaque in-process tree.
2. **Path 1 is lower-fragility and uses documented, stable levers** (`disallowedTools` capability removal, MCP, `canUseTool`), whereas Path 2's core mechanism rests on minified SDK internals and an `agent_id` propagation that docs do not firmly guarantee and the issue tracker partly contradicts.
3. **Hybrid:** keep Path 2's `canUseTool`/PreToolUse gate as a *defense-in-depth backstop* (deny anything not `mcp__capo__*`) even under Path 1, and — if Proto-2 passes — optionally allow a *bounded* set of native tools while still observing nested calls. This buys Path 1's guarantees plus Path 2's visibility where it works.

**What would change this recommendation:**
- If **Proto-2 PASSES decisively** (SDK-callback `canUseTool`/PreToolUse fires for sub-agent inner calls with `agent_id`, and `deny` blocks them) **AND** stakeholders value retaining native subagent fan-out/Skills/WebSearch more than per-sub-agent session control, then a **Path-2-dominant hybrid** becomes attractive: keep native tooling, proxy+attribute every nested call, accept that sub-agents stay logical (not ACP sessions). The cost is higher per-call latency and ongoing fragility to SDK updates.
- If **Proto-1 FAILS** (settingSources override does not neutralize ambient settings via `_meta`), Path 1 still wins but **requires a small patch to claude-code-acp** (pass `settingSources:[]`/`strictMcpConfig:true`, or run only in sandboxed clean cwds) before it is safe — raising Path 1's effort/fragility slightly but not changing the path choice.
- If **Proto-2 FAILS**, Path 2 is effectively dead for 0.2.44 and the recommendation hardens to **pure Path 1** (ban Task, capo-MCP-only).

### Load-bearing citations (verified this session)
- `acp-agent.js:574-575` — `canUseTool` destructures only `{signal,suggestions,toolUseID}`; **drops `agentID`** (the Path-2 patch point).
- `acp-agent.js:748` — `acp` MCP bridge gated by `!disableBuiltInTools`; `:770/:780` — `claudeCode.options` passthrough+spread; `:789` — `canUseTool` wired; `:838,852-855` — `disableBuiltInTools` → blanket `disallowedTools` incl. `Task`.
- `sdk.mjs:21` — control-request handler passes `agent_id` into `canUseTool` (proves SDK supplies sub-agent identity).
- `cli.js:2289/2324/2333` — subagent runner `Wy` re-enters `iR` with same `canUseTool`; wrapper `ACY`; Task handler passes callback down.
- `sdk.d.ts:119` (`CanUseTool.agentID` "sub-agent's ID"), `:496-530` (`allowedTools` auto-approve-only, `disallowedTools` removes from context, `tools:[]` disables built-ins), `:787-796` (`settingSources` empty = SDK isolation).
- `acp-sdk/.../types.gen.d.ts:2017-2024` (flat `SessionId`), `:1648-1671` (`RequestPermissionRequest` no sub-agent field); `acp-agent.js:241,539` (sidechain stripping).
- External: issue #34692 (settings.json hooks don't fire for subagent tool calls — CLOSED not-planned), #115 (`allowedTools` doesn't restrict), #26923 (PreToolUse can't block Task). Docs: code.claude.com Agent SDK permissions/hooks/subagents. Doc-version caveat: all docs track SDK newer than pinned 0.2.44; `disableBuiltInTools` is undocumented; `Task`→`Agent` rename is v2.1.63 — Proto-2 must verify behavior in the bundled `cli.js`.
