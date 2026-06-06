export const meta = {
  name: 'capo-phase-a-console',
  description: 'Phase A FE-only console: per-agent action bar (A1), reload-survival (A3), conductor-worker tree (A4)',
  phases: [
    { title: 'Plan' },
    { title: 'Implement' },
    { title: 'Review' },
  ],
}

// All three items edit the SAME file (crates/capo-web/static/chat.html) against
// existing endpoints. No backend change is needed: POST /api/commands already
// accepts steer_agent / interrupt_agent / stop_agent; GET /api/events?from=0
// replays the full log; GET /api/dashboard lists agents. So this is one
// sequential editor (no parallel file conflict), planned then reviewed.

const FILE = 'crates/capo-web/static/chat.html'

const SPEC = [
  'Project: capo. Target file: ' + FILE + ' (a single static HTML file, ~612 lines, plain JS, no build step).',
  'You MUST read that file and crates/capo-web/src/main.rs first to anchor to real function names and the real JSON shapes.',
  '',
  'Known facts about the existing endpoints (do not re-invent):',
  '- POST /api/commands  body {kind, agent, message?, reason?, goal?}. kind is one of: send_task, steer_agent, interrupt_agent, stop_agent. Returns {ok, sessionId}.',
  '- GET /api/events?from=N&session=S  SSE tail; each frame data is JSON-RPC with params.event. from=0 replays the whole durable log.',
  '- GET /api/dashboard  returns {agents:[{id,name,status,sessionId,adapter,goal,result,runId,...}]}. There is NO parent field.',
  '- GET /api/thread?session=S&from=N  projected multi-turn thread.',
  'Existing JS already has: legibleLine(ev), actorLabel(ev), makeActivity().addEvent, selectAgent(id), renderAgents(agents), pollDashboard(), showPane(name), el(tag,cls,text), statusClass(s), conductorSessionId.',
  '',
  'Implement these THREE items, FE-only, no fork, reusing existing helpers and styles:',
  '',
  'A1 — Per-agent action bar in the detail pane (#detail, built in selectAgent). When an agent has a sessionId, render a small row of buttons: Steer (prompts for a message -> POST /api/commands {kind:"steer_agent", agent:id, message}), Interrupt (POST {kind:"interrupt_agent", agent:id, reason}), Stop (POST {kind:"stop_agent", agent:id, reason}). Use safeFetch. After a command, show a brief inline status (ok / error text). NOTE: today these only RECORD INTENT server-side (live delivery lands later in B1/B2) — add a small muted note in the action bar saying steer/interrupt/stop currently record intent and become live once the in-flight turn registry lands. Do not fake delivery.',
  '',
  'A3 — Reload-survival. On page load, before/independent of the 2s dashboard poll, replay GET /api/events?from=0 ONCE to rebuild the main conductor transcript/activity so a browser refresh does not lose the conversation. Reuse makeActivity()/legibleLine so the rebuilt feed looks identical to the live one. Make it idempotent (the existing live tail from openChatStream must not double-render the same sequences — dedupe by sequence/lastSeq). Keep it cheap: one fetch of the SSE replay (you can read it via fetch + manual parse of the event: / data: lines, or a short-lived EventSource you close after the backlog drains). Pick the simplest robust approach and comment it.',
  '',
  'A4 — Sidebar conductor->worker TREE. In renderAgents, group agents so the conductor (id or name matches /conductor/i) is the root and all other agents render as indented children beneath it (a simple one-level tree with a connector/indent). Show real lifecycle status via the existing statusClass dot. If there is no conductor agent yet, fall back to the current flat list. There is no parent field in the read model, so this one-level heuristic (conductor = root, rest = workers) is the intended fork-free approach — add a short code comment saying so.',
  '',
  'Constraints: keep it a single self-contained HTML file (no new deps, no build). Match the existing dark theme / class names / code idiom. Do not break the existing chat send flow, the details toggle, the agent detail thread/live-events tail, or pollDashboard. Keep diffs surgical and well-commented.',
].join('\n')

phase('Plan')
const plan = await agent(
  'You are planning a small FE change. ' + SPEC + '\n\n' +
  'Do NOT edit anything yet. Read ' + FILE + ' and crates/capo-web/src/main.rs, then return a concrete, surgical implementation plan: for each of A1/A3/A4, the exact functions/lines you will touch in ' + FILE + ', the new helper(s) you will add, and any risk to the existing send flow / detail tail / poll. Return plain markdown.',
  { label: 'plan:phase-a', phase: 'Plan' }
)

phase('Implement')
const impl = await agent(
  'You are implementing a small FE change in ONE file. ' + SPEC + '\n\n' +
  'Here is the agreed plan to follow (adapt only if it is wrong against the real code):\n\n' + plan + '\n\n' +
  'Now EDIT ' + FILE + ' to implement A1, A3, and A4. Make surgical, well-commented edits reusing existing helpers. Do NOT touch Rust files (the endpoints already exist). After editing, return plain markdown: a concise summary of every change you made (function-by-function), and the exact list of items (A1/A3/A4) you completed. Be honest if anything was left partial.',
  { label: 'implement:phase-a', phase: 'Implement' }
)

phase('Review')
const review = await agent(
  'You are a critical reviewer. The following FE change was made to ' + FILE + ' to implement A1 (per-agent action bar), A3 (reload-survival via /api/events?from=0 replay), A4 (conductor->worker sidebar tree). ' +
  'Read the CURRENT ' + FILE + ' and check: (1) all three items are actually implemented and wired to the real endpoints (POST /api/commands kinds steer_agent/interrupt_agent/stop_agent; GET /api/events?from=0; GET /api/dashboard); (2) no regression to the existing chat send flow, details toggle, detail-pane thread/live tail, or pollDashboard; (3) reload replay does not double-render live events (dedupe by sequence); (4) the action bar honestly says steer/interrupt/stop record intent (no faked live delivery); (5) it is still a single self-contained HTML file with no new deps. ' +
  'Report concrete problems with file:line, and a clear PASS/FAIL verdict per item. If something is broken, say exactly what to fix. Return plain markdown.\n\n' +
  'Implementer summary was:\n\n' + impl,
  { label: 'review:phase-a', phase: 'Review' }
)

log('Phase A workflow complete — plan, implement, review done.')
return { plan, impl, review }
