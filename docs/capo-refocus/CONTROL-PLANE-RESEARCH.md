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

---

# Phase 2 — PROTOTYPE RESULTS (empirical, real subscription, logged frames)

Harness: a ~250-line ACP JSON-RPC client spawning an INSTRUMENTED copy of
claude-code-acp@0.16.2 (SDK 0.2.44) on the Pro/Max subscription (no API key). Full
artifacts were under /tmp/capo-cp-proto/ (client.mjs, instrumented bridge/, per-run
frames.json + stderr.log). Env gotcha: must delete `CLAUDECODE`/`CLAUDE_CODE_*` from the
spawned bridge env or the nested `claude` refuses to launch.

## PROTO-1 — Path-1 lockdown integrity: **PASS**
- `disallowedTools` reliably strips native Bash/Task/Read; a hostile project
  `.claude/settings.json` allow-list did NOT re-grant them; a hostile `.mcp.json` server
  never appeared. (OBSERVED, 3 variants.)
- `_meta.claudeCode.options.settingSources:[]` **DOES override** the bridge's hardcoded
  `["user","project","local"]` (Phase-1 "unproven" claim now PROVEN). The real ambient leak
  was USER-level settingSources pulling in the operator's own MCP servers (e.g.
  `hardened-workspace`) — present without the override, gone with it.
- **Sharpened recipe / gotchas:** `tools:[]` is IGNORED (bridge hardcodes the preset after the
  spread). `permissionMode` is hardcoded `default` (can't be overridden — fine). Plain
  `disallowedTools` of native names leaves the bridge's OWN `mcp__acp__Bash` shell live (agent
  ran `echo HOSTILE` through it). **TRUE total lockdown** (Proto-1b) needs
  `_meta.disableBuiltInTools:true` (removes the acp MCP entirely → agent had only inert
  TaskOutput/TaskStop/EnterPlanMode stubs, ZERO shell/fs/Task) OR disallow the `mcp__acp__*`
  names and re-supply file/shell via a capo MCP server.

## PROTO-2 — Path-2 crux (do sub-agent inner calls reach interception?): **PASS for control**
A native `Task` subagent did inner Read(A.txt) + inner `echo INNER`. OBSERVED:
- Inner Bash fired BOTH the SDK `canUseTool` callback AND `session/request_permission` to the
  client, with **`agentID:"ae69749"` POPULATED** (the sub-agent's id). Logged frame:
  `[CANUSE] {toolName:"mcp__acp__Bash", agentID:"ae69749", optionKeys:[...,"agentID"], ...}`.
- The SDK-callback **PreToolUse hook fired for ALL three** (Task, inner Read, inner Bash) — the
  most COMPLETE observation point (sees even auto-allowed tools).
- **DENY genuinely blocks:** replying `reject` to the inner Bash's request_permission → NO
  `terminal/create` ran → `echo INNER` blocked (deny carried `interrupt:true`).
- This contradicts issues #34692/#26923 — but those concern the *settings.json hook* path;
  capo's *SDK-callback* path (canUseTool + SDK-registered hooks) DOES fire for sub-agents in 0.2.44.
- **Limitations (observed):** (1) the bridge DROPS `agentID` when it forwards to the ACP client
  (`acp-agent.js` ~:575 destructure) → capo can SEE+GATE the inner call but needs a **one-line
  forward-patch** to ATTRIBUTE it to a sub-agent; (2) only non-auto-allowed tools hit
  request_permission (inner Read was auto-allowed and bypassed it) → to proxy EVERYTHING via
  request_permission avoid broad allowlists, or use PreToolUse for universal visibility; (3)
  sub-agents stay in-process sidechains in ONE ACP session — never separate capo sessions.

## Updated verdict
- **Path 1 (lockdown): CONFIRMED** — total lockdown demonstrated; no fork needed (all via `_meta`).
- **Path 2 (intercept sub-agents): CONFIRMED for control** — capo CAN observe (PreToolUse, every
  nested call) and GATE/veto (canUseTool deny blocks) a native sub-agent's inner tool calls in
  0.2.44, with `agentID` available; needs a one-line bridge patch to attribute. NOT achievable:
  per-sub-agent ACP sessions (that still requires Path 1's ban-Task-force-start_agent).
- **HYBRID (evidence-backed):** Path 1 where capo must OWN orchestration (per-sub-agent sessions,
  hard lockdown); Path-2 PreToolUse+canUseTool gate as a defense-in-depth + observability layer
  over native sub-agents (proven to see + block them), pending the small `agentID` forward-patch.


---

# Phase 1b — deny-with-guidance + ACP fork/upstream

Two follow-up threads refine the Phase-1/Phase-2 verdict. Thread A nails down a *third control primitive* — denial that carries a model-visible redirect message — which materially upgrades both paths' UX but is **blocked by the pinned bridge today**. Thread B maps the upstream/fork landscape that determines the *cost* of the bridge patches both paths now depend on. Neither overturns the Phase-2 verdict (Path 1 primary, Path 2 hybrid gate); both sharpen the recommendations and add two small, well-scoped prototypes/patches.

---

## A. Deny-with-guidance: a model-visible redirect, not just a veto

### Finding (concrete)

There is a first-class mechanism to **deny a tool call AND feed a free-text message back to the model as the tool result**, so the model adapts instead of failing silently. Two surfaces:

1. **`canUseTool` → `{ behavior:"deny", message, interrupt? }`** (`references/claude-agent-sdk/sdk.d.ts:895-905`). `message` is required on deny; `interrupt?:boolean` controls whether the turn aborts. The bundled engine (`cli.js` fn `iiY`, ~offset 9035905) writes that `message` as the **`content` of a `tool_result` block with `is_error:true`** for the denied call — the model reads it next turn. Anthropic docs carry a verbatim **"Suggest alternative"** recipe (`agent-sdk/user-input`): `return { behavior:"deny", message:"User doesn't want to delete files. They asked if you could compress them into an archive instead." }`. This is exactly the "graceful redirect" mechanism the premise referenced.
2. **PreToolUse hook → `permissionDecision:"deny"` + `permissionDecisionReason`** (`sdk.d.ts:984-990`). Reason text surfaces to the model the same way. ⚠️ but *enforcement* via this path is contested (issues #37210/#33106/#4669/#39344 — deny silently ignored for some tools). Reason-text surfacing is reliable; hard blocking is not. Use as backstop only.

A third lever — `disallowedTools:["Task"]` — removes the tool from context entirely: strongest lockdown, but **no deny event and no message** (nothing to redirect from).

**Critical interaction with our own Phase-2 result:** Proto-2 already observed that a `reject` to inner Bash blocked it *and carried `interrupt:true`*. Thread A explains why and why that is a problem — see below.

### How it changes Path 1

**CAN (new):** Path 1's enforcement story gains a *guidance* mode. Instead of the binary choice between (a) `disallowedTools:["Task"]` (Task invisible — model may not know to call `start_agent`) and (b) silent denial (model stops/loops), capo can keep `Task` *present-but-not-auto-approved* and, on a `Task` call, deny with `message:"Task is disabled here. Call mcp__capo__start_agent with {...} instead."` The model then pivots to capo's orchestration tool **at call time**, which is precisely the "force the conductor to call `start_agent`" requirement that Phase-1 identified as the *only* way to get a first-class capo session per sub-agent. This makes the ban-Task-force-start_agent posture far more reliable than prompt-steering alone.

**LIMITS:** Two postures, with a real trade:
- *Posture 1 — hard removal* (`disallowedTools:["Task","Agent"]`): guaranteed, but no redirect message; relies on system-prompt + `start_agent` being the obvious tool.
- *Posture 2 — deny-with-guidance*: keep Task interceptable (no bare-name disallow, `permissionMode:"default"`, no allow rule for Task → call reaches `canUseTool`). Subject to the **"ask-only" trigger** the doc already documents — the call only reaches the permission callback if not auto-allowed.

Best practice is **both**: `disallowedTools` to *guarantee* Task can't execute, OR keep it interceptable for guidance but have `canUseTool` deny-with-message anything not `mcp__capo__*`.

**COSTS:** Confidence that the message reaches and steers the model is **HIGH** (cli.js sink + 3 doc pages + the explicit example). Confidence the model *reliably pivots* to `start_agent` rather than giving up/looping is **MEDIUM** — it's prompt-following, not a guarantee; reinforce with a system-prompt instruction naming `start_agent`. Needs a prototype to confirm the pivot.

### How it changes Path 2

**CAN (new):** Path 2's gate upgrades from "veto + telemetry" to "veto + steer." When capo denies a native sub-agent's inner call (proven to fire and block in Proto-2), it can now attach a redirect message so the sub-agent adapts rather than dies mid-turn.

**LIMITS / blocker (the load-bearing finding):** **capo cannot deliver a custom deny message through `claude-code-acp@0.16.2` today.** The bridge's `canUseTool` reject branch (`acp-agent.js:683-689`) **hardcodes** `message:"User refused permission to run tool"` and `interrupt:true`. Two consequences:
1. capo's intended redirect never reaches the model — it only ever sees the generic denial (more likely to make it *stop* than redirect).
2. `interrupt:true` **aborts the turn** — the opposite of deny-with-guidance. This is exactly the `interrupt:true` Proto-2 logged. So the deny *that we already proved works* is the wrong flavor for steering.

Also: the ACP wire protocol has **no free-text field** on the permission response — `SelectedPermissionOutcome` carries only `optionId` (`acp-sdk/.../types.gen.d.ts:1636-1640,1799-1813`); `RequestPermissionResponse._meta` (`:1675-1690`) is the only extensibility escape hatch, and the bridge ignores it. So this is a bridge patch, not just a config change.

**COSTS:** Small, well-scoped bridge patch (a few lines), same hot file as the already-needed `agentID` forward-patch — so it folds into one bridge-fork effort, not a new one.

### Recommendation (Thread A)

- **Adopt deny-with-guidance as the steering mechanism for the ban-Task-force-`start_agent` posture** in Path 1, and as the steer layer on Path 2's gate.
- **Use mechanism (a) `canUseTool` deny+message** as primary (best-documented, model-visible). Use `disallowedTools` for hard removal where guidance is unneeded. Treat PreToolUse-deny as backstop only (enforcement unreliable per the issue cluster).
- **Land the bridge patch** (below) — without it, every capo denial reaches the model as the generic hardcoded string with `interrupt:true`, defeating the redirect.

**Patch needed (load-bearing):** in `acp-agent.js` reject branch, derive `message` and `interrupt` from capo's response instead of hardcoding. Preferred form — `_meta` passthrough (general, keeps the ACP options menu clean):
```js
const denyMsg = response._meta?.["capo.denyMessage"] ?? "User refused permission to run tool";
return { behavior:"deny", message: denyMsg, interrupt: response._meta?.["capo.interrupt"] ?? false };
```
(Alternative: encode semantics in a distinct `optionId` and map bridge-side.) Note `interrupt` now defaults **false** so the model keeps going and reads the message.

**Prototype needed — Proto-5 (deny-with-guidance pivot):** patch the bridge reject branch to forward `_meta["capo.denyMessage"]` + `interrupt:false`; drive a turn where the conductor calls `Task`; have capo deny with `message:"Task is disabled. Call mcp__capo__start_agent with {...}."`
- **PASS:** model receives the message as an `is_error:true` tool_result and **next calls `mcp__capo__start_agent`** (not retry-Task, not stop). Confirms HIGH-confidence transport + MEDIUM-confidence pivot.
- **FAIL:** model loops on Task / gives up → fall back to Posture 1 (hard `disallowedTools` removal) + system-prompt steering, and rely on capo owning `start_agent` as the only orchestration tool.

---

## B. Fork/upstream feasibility for the bridge patches

### Finding (concrete)

Both bridge patches Phase-1/1b now depend on (`agentID` forward + deny-message passthrough) require modifying the bridge. Thread B maps that cost.

1. **Repos moved to a vendor-neutral org.** Spec is now `agentclientprotocol/agent-client-protocol`; the bridge is `agentclientprotocol/claude-agent-acp` (npm `@zed-industries/claude-code-acp` → `@agentclientprotocol/claude-agent-acp`). **Version delta vs our doc:** our references pin `claude-code-acp@0.16.2`; upstream is now **v0.33.x**. The old `zed-industries/*` repos redirect.
2. **Premise correction on "strips sidechain."** The doc's "bridge strips `isSidechain`" observation (`acp-agent.js:241,539`) reflects **0.16.x**. Current v0.33.x already **forwards subagent tool calls tagged with `_meta.claudeCode.parentToolUseId`** (proven on the wire in bridge issue #708's packet capture). What it still does *not* do: stream subagent *text/thinking* as live partials (gated on `parent_tool_use_id === null`, ~lines 1288/1395), and give subagents first-class ACP identity. **This must be re-verified against v0.33.x `src/acp-agent.ts` before designing any fork** — the line numbers and some behaviors in our Phase-1 citations are 0.16.2-specific.
3. **No sub-agent primitive in the spec, v1 or v2.** No parent/child, sub-session, or `agent_id` field on tool calls/updates. Maintainer posture (#623): subagent semantics belong to the Claude Agent SDK; ACP only relays. v2 roadmap has no sub-agent primitive.
4. **`_meta` is the blessed extension channel**, and the **meta-propagation RFD moved to Completed 2026-06-03** — the project actively endorses namespaced `_meta` (e.g. `_meta.claudeCode.*`) for cross-protocol attribution. The bridge already discards a known bucket of SDK events in dead-letter switch arms (issues #650/#679/#624; PR #713 forwards task events; PR #653 adds agent selection as session config) — meaning our needed changes are *the same shape as changes the maintainers already accept*.

### How it changes Path 1 / Path 2 (can/limits/costs)

**CAN:** Everything both paths need is achievable **additively, via `_meta` + capability advertisement, by forking the bridge only — not the spec.** The deny-message passthrough (Thread A), the `agentID`→`requestPermission._meta` forward (Phase-2 limitation #1), and `settingSources:[]` override (already proven) are all `_meta`-shaped and consistent with upstream conventions. This validates the doc's "no SDK fork required" claim for Path 1 and "one-line bridge patch" for Path 2 — and adds that they are **plausibly upstreamable**, which can collapse fork-maintenance to near-zero.

**LIMITS:**
- **Spec cannot give us per-sub-agent ACP sessions** — confirms Investigation C is a *durable* constraint, not a version artifact. The only upstream peg for isolated sub-conversations is the `session/fork` RFD (author @josevalim, explicitly names subagents as a use case) — worth watching, but not shipping, and not a parent/child runtime tree.
- **SDK coupling is the irreducible ceiling for Path 2.** Subagent dispatch + context inheritance live in the closed Claude Agent SDK (#623: subagents don't even inherit CLAUDE.md). The fork can only relay what the SDK exposes; it cannot make sub-agents first-class. External risk: Agent SDK Pro/Max availability churn (#658).
- **Live subagent text/thinking** is gated off in the bridge (`parent_tool_use_id === null`) — Path-2 *visualization* (our Proto-4 FAIL mode) needs a bridge change to relax that gate and tag chunks with `parentToolUseId`; this is additive and PR #713-shaped.

**COSTS:**
- **Bridge fork: moderate, ongoing but mitigable.** `src/acp-agent.ts` is one ~3,750-line file under heavy churn (frequent SDK bumps), and our diffs land in the *hottest* zone (the message-routing switch / `toAcpNotifications`). Expect rebase conflicts. Mitigate by keeping changes small, additive, and **behind an env flag** (repo precedent: `CLAUDE_CODE_EMIT_SESSION_STATE_EVENTS`) — which also eases upstreaming.
- **Spec fork: high cost, low value — avoid.** Multi-vendor governance, fast-moving; a divergent core type is unmergeable and perpetually stale.
- **Upstreaming: realistically good for the bridge.** "Don't drop SDK events" PRs already land (#649/#676/#694/#713); `_meta.claudeCode.*` extensions match the completed meta-propagation RFD; responsive triager (benbrandt). If accepted, fork maintenance → ~zero.

### Recommendation (Thread B)

- **Fork `agentclientprotocol/claude-agent-acp` (the bridge), never the spec.** Bundle all needed changes — deny-message `_meta` passthrough, `agentID` forward, relax the `parent_tool_use_id===null` streaming gate, `subagentType`/`subSessionId` in `_meta.claudeCode` — into **one additive, env-flag-gated patch set**.
- **Re-baseline our references to v0.33.x before implementing.** Our Phase-1 line numbers (`:539,:575,:683`, the sidechain-strip claim) are 0.16.2-specific; confirm each against current `src/acp-agent.ts`. This is a prerequisite for both bridge patches above.
- **Pursue upstreaming** via the "don't drop SDK events" precedent + `_meta` extension model to drive fork cost toward zero. **Watch `session/fork`** as the only upstream primitive that could ever yield isolated sub-conversations.

**Prototype/patch needed — Proto-6 (re-baseline + unified bridge patch):** clone `claude-agent-acp@0.33.x`, confirm (a) subagent tool calls already arrive with `_meta.claudeCode.parentToolUseId`, (b) the reject-branch hardcoded message/`interrupt` still exists, (c) the streaming gate location. Then apply the unified `_meta` patch set behind a `CAPO_*` env flag and re-run Proto-2/Proto-4/Proto-5 against it. This single effort de-risks Thread A's blocker, Path-2's attribution gap, and Path-2's visualization gap together.

---

## Updated recommendations

### Path 1 — Lock to capo-only tools (still PRIMARY)
Unchanged as the primary architecture, now **strengthened**: pair `disallowedTools`/`disableBuiltInTools` hard-lockdown (Proto-1 PASS, no fork) with **deny-with-guidance** to reliably force the conductor onto `mcp__capo__start_agent` — the only route to first-class per-sub-agent capo sessions, which Thread B confirms the spec will *never* provide natively. Guidance needs the bridge deny-message patch (Thread A) + the model-pivot confirmation (Proto-5). Posture: prefer hard removal where guidance is unneeded; use deny-with-guidance where you want the model to discover `start_agent` at call time. Keep both as defense-in-depth.

### Path 2 — Proxy every tool call, incl. sub-agents' (still HYBRID gate)
Unchanged as the observability/veto hybrid (Proto-2 PASS for control), now with two concrete upgrades, both gated on the **same** bridge fork:
1. **Deny-with-guidance** turns the gate from veto-only into veto+steer — but requires replacing the hardcoded `message:"User refused permission to run tool"` + `interrupt:true` (Thread A; this is exactly the `interrupt:true` Proto-2 logged).
2. **Attribution + live nested visualization** require the `agentID` forward and relaxing the `parent_tool_use_id===null` streaming gate (Thread B; resolves Proto-4's FAIL mode).
Ceiling unchanged and now confirmed durable: **no per-sub-agent ACP sessions** (spec has no primitive; only `session/fork` is a distant peg). SDK coupling (#623) caps how first-class sub-agents can ever be.

### Net
Both paths now converge on **one additive, env-flag-gated fork of `claude-agent-acp` v0.33.x** carrying: `settingSources:[]` override (proven), `agentID`→`requestPermission._meta` forward, deny-message + `interrupt:false` passthrough, and a relaxed subagent-streaming gate — all `_meta`-shaped, upstreamable, and re-baselined off our 0.16.2 references. Outstanding empirical risks reduce to **Proto-5** (does the model actually pivot to `start_agent` on a guided deny?) and **Proto-6** (re-baseline + unified patch against v0.33.x). Recommendation stands: **Path 1 primary, Path 2 as defense-in-depth + observability hybrid**, now with deny-with-guidance as the steering layer across both.

---

Relevant files:
- `/Users/nicolas/devel/capo-sliceA/docs/capo-refocus/CONTROL-PLANE-RESEARCH.md` — the doc this section appends to (note: Phase-1 citations are pinned to `claude-code-acp@0.16.2` / SDK `0.2.44`; Thread B requires re-baselining to bridge v0.33.x).


---

# Build Roadmap — Composing P1 + P2 into the Hybrid Control Plane

This section synthesizes the [P1 lockdown plan](#p1-build-plan) and the [P2 observe/veto plan](#build-slice-p2) into a single sequenced roadmap. All anchors are `file:line` in capo (`crates/...`) or the bridge (`references/claude-code-acp/dist/acp-agent.js`).

## The end state (what P1+P2 compose into)

capo runs a **hybrid control plane** over one ACP session per conductor turn:

- **P1 (enforcement)** makes capo the *only* orchestrator the conductor knows: `disableBuiltInTools:true` + `settingSources:[]` + `disallowedTools:[Task,Agent,...]` strip every native tool and sub-agent, and capo re-supplies file/shell/search as its OWN MCP tools (`acp_mcp_http.rs`). The conductor delegates exclusively through `start_agent`. This is the *primary* control surface — banned tools are absent from context, so the model never reaches for `Task`.
- **P2 (observation + backstop veto)** is the *safety net* for any native sub-agent that ever slips through (a future bridge bump, a worker profile that isn't locked, a partial-lockdown mode): a patched bridge fires a universal `PreToolUse` hook that streams every inner call to capo's event log + sidebar, attributed by `agentID`, and capo's existing permission decider can veto each call.

They share **one injection seam** — `_meta.claudeCode.options` rendered in `session_new` (`acp_wire.rs:375`) — and one wiring site (`server_core.rs:424-435`). P1 fills the `options` blob (lockdown recipe); P2 adds the sibling hook marker + reads `agentID` back in the permission round-trip. **The two plans were designed to occupy the same struct field** (`AcpSessionSetupPlan`) and the same params builder, so they do not collide; they layer.

```
                 session/new  _meta
                 ┌──────────────────────────────────────┐
   P1 fills ───► │ claudeCode.options { settingSources:[],│
                 │   disallowedTools:[Task..], strictMcp }│
                 │ disableBuiltInTools:true               │
   P2 adds ────► │ (vendored-bridge PreToolUse hook)      │
                 └──────────────────────────────────────┘
   conductor: ZERO native tools ──► must use capo MCP (capo_read/write/bash/search + start_agent)
   any native sub-agent (defense-in-depth): PreToolUse hook ─► capo event log + veto via request_permission
```

## Recommended order: **P1 first, then P2. Not parallel.**

Three reasons, all dependency-driven:

1. **P1 is the actual product; P2 is insurance.** Once `Task`/`Agent` are in `disallowedTools` and absent from context (P1 §3a), native sub-agents essentially *don't happen* on the conductor. P2's value (observe/veto native sub-agents) is therefore a backstop and a path toward locking *workers* — strictly lower priority than making the conductor lockable at all. Ship the thing that changes behavior first.
2. **They share the injection seam, and P1 lands it cleanly.** P1 introduces `AcpSessionSetupPlan.session_lockdown` + the `session_new` `_meta` rendering (`acp_wire.rs:375`) and the byte-identity guard for stub/scripted transports (`acp_client.rs:39-46`). P2's `meta_options`/hook-marker wiring is a *small extension of the same field and the same params builder*. Building P1 first means P2 inherits a tested seam instead of two threads racing to edit `session_new` and the same `AcpSessionSetupPlan` constructor (`acp_wire.rs:21-47`) simultaneously — a guaranteed merge conflict on `slice-a-acp-wiring`.
3. **P2 has a heavier, riskier dependency (vendored bridge fork) that P1 does not need.** P1 ships entirely against the *stock* `claude-code-acp@0.16.2` using only documented-by-observation `_meta` keys. P2 *requires* a vendored, patched bridge (the `agentID` drop at `acp-agent.js:575` and the JSON-can't-carry-a-JS-callback hook constraint are hard blockers — see Dependencies). Don't take on the vendoring risk until the enforcement layer is proven on the real subscription.

**Parallelizable sub-thread:** the deny-with-guidance research (P1 §3b wire-field) and the upstream `agentID` PR (P2 option (c)) can both run as *non-blocking background threads* alongside P1 implementation. Neither gates the critical path.

## The single smallest first slice that delivers user-visible value

**P1 §1 + §2 + §3a only — "Locked conductor with capo file/shell tools and system-prompt guidance"** (no worker lockdown, no reactive deny-message, no P2).

Concretely:
- `AcpSessionSetupPlan.session_lockdown` field + `AcpSessionLockdown::conductor_default()` + `with_session_lockdown` builder (`acp_client.rs:99-164`).
- Render `_meta` in `session_new` (`acp_wire.rs:375`), gated `Some`/`None` for byte-identity.
- Wire `conductor_default()` at `server_core.rs:430`.
- The 4 capo MCP tools `capo_read/write/bash/search` over existing `runtime_wrappers.rs` (confinement inherited free).
- `system_prompt_append` telling the model to use `start_agent` / `capo_*` instead of `Task`/`Bash`.

**Why this is the smallest valuable unit:** after this slice, a real conductor turn (gated live test, `CAPO_SERVER_RUN_ACP_LIVE=1`) demonstrably has **zero native tools** — it lists only `mcp__capo__*`, cannot call `Task`/`Bash`/`Read`, and performs all I/O through capo's observable invocation log (`acp_mcp_http.rs:101`). That is the headline capability of the entire refocus ("capo owns ALL orchestration") proven end-to-end on the real subscription, in ~2 days, with no bridge fork and no dependency on the deny-message thread. Everything else (worker lockdown, deny-with-guidance text, P2 observe/veto) is additive on top.

## Dependencies

| Item | Depends on | Blocking? |
|---|---|---|
| P1 §1/§2/§3a (smallest slice) | stock bridge `0.16.2`; existing `runtime_wrappers` | **No deps** — ships immediately |
| P1 §3b reactive deny-message | **deny-with-guidance research thread** (exact ACP wire field for a model-visible deny string) | Blocks §3b only; ship §3a meanwhile (sufficient — banned tools are absent from context) |
| P1 §1d worker lockdown | P1 §2 capo MCP tools must exist first (locked workers lose native `fs/*`, must route through capo MCP; workers don't currently get `with_http_mcp_server`) | Blocks worker lockdown; conductor lockdown ships without it |
| **P2 (all)** | **Vendored bridge fork** — `agentID` is dropped at `acp-agent.js:575`; SDK hooks are JS callbacks (`sdk.d.ts:259-268`) that cannot survive JSON in `_meta`. The `_meta`-only "thin wrapper" option is **rejected by source** | **Hard blocker** — P2 cannot deliver attribution or universal observation without the fork |
| P2 wiring (`acp_wire`/decider/event) | P1's `AcpSessionSetupPlan` `_meta` seam (reuses `with_meta_options` next to `with_session_lockdown`) | Soft — strongly prefer P1 merged first to avoid seam conflict |
| P2 upstream `agentID` PR (option c) | upstream review/merge timing | **Non-blocking** — parallel track; P2 ships on the vendored fork regardless |

**On the vendored fork (the one architectural commitment in P2):** copy `references/claude-code-acp` → `vendor/claude-code-acp/`, apply a 3-site `.patch` (forward `agentID` at `:575`; attach `_meta.capo.agentID` to both `requestPermission` payloads `:585`/`:641`; add the capo `PreToolUse` `HookCallback` at `:797-804`), and spawn *that* `dist/acp-agent.js` via the already-capo-controlled `req.acp_program`/`req.acp_argv` (`server_core.rs:436-442`). Pin `0.16.2` + SDK `0.2.44`; CI-check the patch still applies. This is the single highest-fragility item in the whole roadmap — it is the reason P2 is sequenced second and gated behind P1's value being banked.

## Effort per slice

| Slice | Effort | Notes |
|---|---|---|
| **P1 §1** plan field + builder + `session_new` render + conductor wire | ~0.5 day | |
| **P1 §2** 4 capo MCP file/shell/search tools | ~1 day | most cost is `WrapperToolRequest` envelope + result→MCP mapping; safety reused |
| **P1 §3a** systemPrompt append | folded into §1 | |
| **P1 tests** (deterministic + 1 gated live) | ~1 day | |
| **P1 smallest-slice subtotal** | **~2.5–3 days** | conductor lockdown, fully usable, no fork |
| P1 §1d worker lockdown | follow-up | after §2 exists |
| P1 §3b reactive deny-message | ~0.5 day | after deny-guidance thread resolves |
| **P2** bridge fork + 3-site patch + vendor/build | ~0.5–1 day | |
| **P2** `acp_wire` + decider/event struct threading | ~1 day | |
| **P2** new `EventKind` + codec + projection + normalizer | ~1 day | |
| **P2** web sidebar render + tests | ~0.5–1 day | events flow to `/api/events` free |
| **P2 subtotal** | **~3–4 days** | |
| **Roadmap total** (P1 conductor + P1 follow-ups + P2) | **~7–9 days** | |

## Sequenced plan

1. **Slice 0 (the smallest valuable slice) — P1 §1+§2+§3a.** ~2.5–3 days. Locked conductor + capo MCP file/shell/search + system-prompt guidance. Gated live test proves zero native tools on the real subscription. *Ship this first; it is the product.*
2. **Slice 1 — P1 §3b reactive deny-message.** ~0.5 day, **after** the deny-with-guidance research thread confirms the wire field. Belt-and-suspenders; pre-design `AcpPermissionOutcome.deny_message` now (`acp_wire.rs:826`) so it's a drop-in.
3. **Slice 2 — P1 §1d worker lockdown.** Follow-up once §2 tools exist and workers are given `with_http_mcp_server`. Extends lockdown from conductor to workers (caveat: workers lose on-wire `fs/*`, must use capo MCP).
4. **Slice 3 — P2 observe/veto.** ~3–4 days. Vendor + patch the bridge; thread `agentID` through `answer_permission` (`acp_wire.rs:791-852`); add `ToolSubagentCallObserved` `EventKind` (`event.rs:50-56`) + normalizer branch; render nested sub-agent rows + veto badges in the web sidebar (`capo-web/src/main.rs:480-503`). Delivers attributed observation + per-call veto of any native sub-agent as defense-in-depth.

**Parallel non-blocking threads** (run alongside Slices 0–3, gate nothing): the deny-with-guidance wire-field research (feeds Slice 1) and the upstream `agentID` PR (feeds Slice 3's maintainability, not its delivery).

## Why this composition is correct

P1 makes the *common case* impossible-to-violate by construction (no `Task` in context). P2 makes the *residual case* observable and vetoable without requiring per-sub-agent ACP sessions (which ACP cannot model — `SessionId` is flat, no parent/child). Together: capo **owns** orchestration through the locked conductor + `start_agent`, and **watches/vetoes** anything native that escapes — the hybrid control plane the research set out to build, with enforcement (P1) as the load-bearing layer and interception (P2) as the backstop.

---

Roadmap section delivered above (Markdown, ready to append to `/Users/nicolas/devel/capo-sliceA/docs/capo-refocus/CONTROL-PLANE-RESEARCH.md`). Key decisions: P1 first (no bridge fork, ships the product), P2 second (hard-gated on a vendored bridge fork), smallest valuable slice = P1 §1+§2+§3a locked conductor (~2.5–3 days), deny-guidance and upstream-PR threads run parallel and block nothing.

> **RECONCILIATION NOTE (added after Phase-1b).** This roadmap's bridge file:line anchors
> (`acp-agent.js:575/585/641/797`) are pinned to `claude-code-acp@0.16.2`. Phase-1b (Thread B)
> found upstream is now `agentclientprotocol/claude-agent-acp` **v0.33.x**, which ALREADY
> forwards `_meta.claudeCode.parentToolUseId` for subagent calls. So **P2's vendored fork must
> be re-based on v0.33.x (Proto-6)** before patching — some P2 patch sites may already be partly
> done upstream. Also: capo currently spawns UNPINNED `npx … claude-code-acp` (likely the newer
> line), so even P1 should pin/verify the bridge version it runs against. P1's `_meta` lockdown
> recipe is version-robust (proven behaviors), so P1 ships first regardless.

