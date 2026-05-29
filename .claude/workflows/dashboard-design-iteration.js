export const meta = {
  name: 'dashboard-design-iteration',
  description: 'Generate diverse production-grade design directions for the Capo operator dashboard, render them, evaluate with a judge panel, and synthesize a winning design spec',
  whenToUse: 'When you want to explore and pick a serious, production-grade visual design for the Capo web dashboard. Produces standalone HTML mockups, screenshots, scored evaluation, and a consolidated design spec for the winner.',
  phases: [
    { title: 'Explore', detail: 'one agent per design direction authors a self-contained mockup + design notes' },
    { title: 'Render', detail: 'batch screenshot every mockup at desktop + mobile via headless Chrome' },
    { title: 'Evaluate', detail: 'diverse judge panel scores and ranks all candidates comparatively' },
    { title: 'Synthesize', detail: 'aggregate the panel, pick the winner, and write a consolidated design spec' }
  ]
}

// ---------------------------------------------------------------------------
// Configuration (overridable via `args`)
// ---------------------------------------------------------------------------
const ROOT = '/Users/nicolas/devel/capo'
const EXPLORE_DIR = (args && args.exploreDir) || `${ROOT}/workpads/dashboard-webclient/design-explorations`
const SHOTS_DIR = (args && args.shotsDir) || `${EXPLORE_DIR}/shots`

// The real fixture the dashboard renders, baked into mockups so every candidate
// shows identical, realistic operator content and is judged on design alone.
const FIXTURE = {
  project: { id: 'project-capo', name: 'Capo', mode: 'fixture', server: 'mock server-command API', updatedAt: '2026-05-28T12:00:00Z' },
  summary: { agents: 5, active: 3, blocked: 1, evidence: 6, reviews: 2, validations: 3 },
  agents: [
    { name: 'capo-operator', status: 'finished', adapter: 'codex_exec', goal: 'Map operator input into validated server-backed actions', result: 'Selected results_evidence for an all-agent evidence request.', confidence: 'medium', evidence: ['planner-action-results-evidence'], reviews: 0, validations: 1, tools: 0, memory: 0 },
    { name: 'codex-local', status: 'timed out', adapter: 'codex_exec', goal: 'Inspect local workspace and report blockers', result: 'Timed out after recording two tool calls and a memory packet.', confidence: 'low', evidence: ['timeout-log', 'tool-summary'], reviews: 1, validations: 0, tools: 2, memory: 1, blocker: 'Provider run exceeded the configured timeout.' },
    { name: 'codex-one', status: 'running', adapter: 'codex_exec', goal: 'Continue dashboard implementation', result: 'Working on webclient design and browser verification.', confidence: 'medium', evidence: ['workpad-dwc-design'], reviews: 0, validations: 1, tools: 3, memory: 0 },
    { name: 'codex-proof', status: 'finished', adapter: 'codex_exec', goal: 'Prove Codex live provider path', result: 'Finished Codex-backed proof through the server boundary.', confidence: 'high', evidence: ['codex-live-smoke', 'server-dispatch-proof'], reviews: 1, validations: 1, tools: 1, memory: 0 },
    { name: 'demo', status: 'running', adapter: 'fake', goal: 'Exercise mocked steering and dashboard fixtures', result: 'Fake adapter processed the latest steering goal.', confidence: 'medium', evidence: ['demo-evidence'], reviews: 0, validations: 0, tools: 1, memory: 1 }
  ],
  goals: [ { id: 'goal-dashboard-webclient', title: 'Complete dashboard webclient workpad', status: 'active', requirements: ['design accepted', 'browser screenshots', 'interactive smoke'], blockers: ['live HTTP server integration is future work'], validation: 'fixture browser smoke' } ],
  activity: [
    { time: '12:00', agent: 'capo-operator', kind: 'planner', text: 'Mapped an all-agent response/evidence request to results_evidence.' },
    { time: '11:42', agent: 'codex-proof', kind: 'validation', text: 'Codex proof completed through server dispatch.' },
    { time: '11:15', agent: 'codex-local', kind: 'blocker', text: 'Timed out while collecting workspace internals.' }
  ],
  evidence: [
    { id: 'planner-action-results-evidence', kind: 'test', status: 'validated', agent: 'capo-operator' },
    { id: 'codex-live-smoke', kind: 'manual', status: 'validated', agent: 'codex-proof' },
    { id: 'timeout-log', kind: 'artifact', status: 'partial', agent: 'codex-local' }
  ],
  reviews: [
    { id: 'review-codex-local-timeout', status: 'needs follow-up', target: 'codex-local' },
    { id: 'review-codex-proof', status: 'accepted', target: 'codex-proof' }
  ],
  validations: [
    { id: 'browser-smoke', status: 'pending', target: 'dashboard-webclient' },
    { id: 'codex-proof-smoke', status: 'passed', target: 'codex-proof' },
    { id: 'planner-results-evidence', status: 'passed', target: 'capo-operator' }
  ]
}

// Shared product + IA context every candidate must honor.
const CONTEXT = `
CAPO is a controller/harness for supervising autonomous coding LLM agents. This
screen is the OPERATOR DASHBOARD: a quiet, dense, scan-optimized console an
engineer keeps open while several coding agents run. It is NOT a marketing page.

The operator's questions, answered at a glance:
- What is running right now? (active / blocked / finished agents)
- What did each agent do, and is its work reviewed / validated / blocked?
- What evidence backs the latest result?
- What can I safely do next: steer, interrupt, or stop an agent?

REQUIRED CONTENT on the overview screen (use the fixture verbatim — same names,
counts, statuses, copy):
- App shell: product mark "Capo", view label "Operator Dashboard", a server/mode
  badge ("${FIXTURE.project.server}", mode "${FIXTURE.project.mode}"), tabs
  (Overview / Goals / Settings), and refresh + details controls.
- Status strip: 6 metrics — Agents 5, Active 3, Blocked 1, Evidence 6, Reviews 2,
  Validations 3. "Blocked 1" must read as a warning, not as neutral.
- Agent list/table: all 5 agents with name, status, adapter, one-line result,
  and evidence/review counts. Status semantics must be visually distinct:
  running, finished, timed out, blocked.
- Session detail: the selected agent (capo-operator) — latest result, goal,
  confidence, evidence, reviews, validations, blocker.
- Command panel: a steer textarea + Send, Interrupt, Stop controls (Interrupt and
  Stop are destructive and must read as such), and a small command log line.
- Recent activity timeline: the 3 activity events.
- Evidence / Reviews / Validation lanes: compact rows with status semantics.

STATUS SEMANTICS: running=in-progress, finished=ok/done, timed out & blocked=
attention/warning, plus accepted/passed/validated=good and needs-follow-up/
pending/partial=caution. Color must never be the ONLY signal — pair it with a
label, dot, icon, or shape.

HARD CONSTRAINTS:
- A SINGLE self-contained .html file: all CSS in one <style> block, content as
  static HTML. No JavaScript required to look complete. No external network
  requests of any kind (no Google Fonts, no CDNs, no remote images). Use only
  system font stacks (e.g. ui-sans-serif/system-ui and ui-monospace) and inline
  SVG or CSS for any glyphs/dots/icons.
- Design for a 1440px-wide desktop frame that the content fills top-to-bottom
  with no large dead zones, and degrade sanely to a 390px mobile width.
- This is an operational tool: no hero sections, no decorative gradient/orb
  backgrounds, no oversized marketing headings, no cards-nested-in-cards clutter.
- It must look like a real, shipped, production-grade product — the bar is
  Linear / Vercel / Stripe / Datadog / Grafana / Sentry — not a prototype.

FIXTURE DATA (authoritative content):
${JSON.stringify(FIXTURE, null, 2)}
`

// Five deliberately distinct directions. Diversity across light/dark, accent
// system, density, and typographic personality is the point — so the panel has
// a real spread to choose from.
const DEFAULT_DIRECTIONS = [
  {
    key: 'linear-graphite',
    title: 'Refined minimal (Linear / Vercel)',
    mode: 'light',
    brief: `Near-monochrome, high-craft SaaS. A cool graphite/zinc neutral ramp on near-white (#fafafa-ish) surfaces, hairline 1px borders (low-contrast), and a SINGLE restrained accent (indigo/violet ~#5b5bd6) used sparingly for the active/primary signal only. Status colors are desaturated and precise. Typography is the hero: a tight, deliberate type scale with crisp weight contrast (medium labels, semibold numerics), generous line-height, immaculate alignment to a baseline grid. Subtle elevation (faint shadows, not borders) on the few raised surfaces. Whitespace is disciplined, not lavish. Numerals tabular. The feeling is calm, exact, and expensive.`
  },
  {
    key: 'ops-console-dark',
    title: 'Operational console (Datadog / Grafana)',
    mode: 'dark',
    brief: `A serious dark ops console. Deep slate/navy base (#0e1117 / #11161f), panels one step lighter with crisp 1px seams, dense data tables with compact row height and tabular figures. Status is communicated with small filled status DOTS + label + desaturated background chips (green/amber/red/blue). Metrics read like a monitoring header (big tabular numbers, small uppercase labels, room for a sparkline). Monospace for ids/adapters/counts. High information density without feeling cramped — strong column alignment, zebra or hairline row separation. The feeling is mission-control: trustworthy, legible at a glance, built for repeated inspection.`
  },
  {
    key: 'terminal-native',
    title: 'Developer / terminal-native (IDE)',
    mode: 'dark',
    brief: `Developer-native, fitting a tool that drives coding agents. Near-black editor background (#0b0e14), a monospace-forward type system (UI in a clean sans, but ids/results/status/metrics in ui-monospace), and high-contrast semantic terminal colors (green #3fb950, amber #d29922, red #f85149, cyan #39c5cf, violet for accents). Think a polished IDE/terminal panel: a left rail or tab strip, a command-prompt-styled steer input ("> steer …"), status as colored glyph prefixes ([●]/[!]/[✓]). Crisp, slightly technical, but still composed and modern — JetBrains/VS Code quality, not a toy CRT. Avoid skeuomorphic scanlines.`
  },
  {
    key: 'stripe-structured',
    title: 'Structured product (Stripe / Sentry)',
    mode: 'light',
    brief: `The confident, structured light-product baseline. Clean white surfaces on a faint cool-grey canvas, clearly delineated sections with subtle headers, a real but tasteful color system (a brand indigo/blue primary plus full semantic green/amber/red/blue), and polished data presentation. Slightly rounded cards (8-12px) with soft single-direction shadows, strong section headers with supporting metadata, well-structured tables with aligned numeric columns. Confident type hierarchy with a clear h-scale. The feeling is a mature commercial dashboard you'd trust with production infrastructure — organized, branded, and legible.`
  },
  {
    key: 'slate-editorial',
    title: 'Calm editorial neutral (Notion / Height / Linear-light)',
    mode: 'light',
    brief: `A calm, warm-neutral, editorial take. Warm grey/stone surfaces (not pure white), soft contrast, generous-but-disciplined whitespace, and a muted teal/green primary accent. Borders are very light; structure comes from spacing and type rather than heavy lines. Friendly but unmistakably professional: rounded-but-restrained corners, gentle status pills, comfortable reading rhythm, and a quiet, low-stress information density. The feeling is a thoughtfully designed modern productivity tool — approachable, uncluttered, and easy to live in for hours.`
  }
]

const DIRECTIONS = (args && Array.isArray(args.directions) && args.directions.length)
  ? args.directions
  : DEFAULT_DIRECTIONS

// ---------------------------------------------------------------------------
// Schemas
// ---------------------------------------------------------------------------
const AUDIT_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  properties: {
    present: { type: 'array', items: { type: 'string' }, description: 'keys whose mockup .html exists and is > 2KB' },
    missing: { type: 'array', items: { type: 'string' }, description: 'keys whose mockup is absent or a stub' },
    details: { type: 'string' }
  },
  required: ['present', 'missing']
}

const RENDER_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  properties: {
    ok: { type: 'boolean' },
    shots: { type: 'array', items: { type: 'string' } },
    missing: { type: 'array', items: { type: 'string' } },
    log: { type: 'string' }
  },
  required: ['ok', 'shots']
}

const JUDGE_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  properties: {
    lens: { type: 'string' },
    candidates: {
      type: 'array',
      items: {
        type: 'object',
        additionalProperties: false,
        properties: {
          key: { type: 'string' },
          score: { type: 'number', description: '0-10 on this lens' },
          strengths: { type: 'string' },
          weaknesses: { type: 'string' }
        },
        required: ['key', 'score', 'strengths', 'weaknesses']
      }
    },
    ranking: { type: 'array', items: { type: 'string' }, description: 'candidate keys, best to worst, on this lens' },
    best: { type: 'string' },
    worst: { type: 'string' },
    notes: { type: 'string' }
  },
  required: ['lens', 'candidates', 'ranking', 'best', 'notes']
}

const SYNTH_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  properties: {
    ranking: {
      type: 'array',
      items: {
        type: 'object',
        additionalProperties: false,
        properties: {
          key: { type: 'string' },
          rank: { type: 'number' },
          overallScore: { type: 'number' },
          rationale: { type: 'string' }
        },
        required: ['key', 'rank', 'overallScore', 'rationale']
      }
    },
    winner: { type: 'string' },
    runnersUp: { type: 'array', items: { type: 'string' } },
    graft: { type: 'string', description: 'specific elements to borrow from runners-up into the winner' },
    specPath: { type: 'string' },
    summary: { type: 'string' }
  },
  required: ['ranking', 'winner', 'specPath', 'summary']
}

// ---------------------------------------------------------------------------
// Prompts
// ---------------------------------------------------------------------------
function genPrompt(dir) {
  const htmlPath = `${EXPLORE_DIR}/${dir.key}.html`
  const notesPath = `${EXPLORE_DIR}/${dir.key}.notes.md`
  return `You are a senior product designer + front-end engineer. Produce ONE complete, beautiful, production-grade design mockup of the Capo operator dashboard in a specific visual direction, then write design notes.

DESIGN DIRECTION: "${dir.title}" (${dir.mode} mode), key="${dir.key}"
${dir.brief}

${CONTEXT}

DELIVERABLES (use the Write tool):
1. Write the mockup to EXACTLY this absolute path: ${htmlPath}
   - A single self-contained .html file (inline <style>, static HTML content, no JS needed, no external requests, system fonts only — re-read the HARD CONSTRAINTS above).
   - Realize the direction with conviction and craft: a real design system (tokens for color, type scale, spacing, radius, border, elevation), not a wireframe. Every required content block must be present and populated from the fixture.
   - Make it genuinely excellent and finished: something you would ship. Fill the 1440px desktop frame with no awkward empty regions; wrap the whole thing so it also reads acceptably at 390px (you may use a single @media query).
2. Write design notes to EXACTLY: ${notesPath}
   - The token values you chose (hex colors by role, type scale, spacing, radii), the layout strategy, the signature moves, and how status/uncertainty is encoded without relying on color alone.

QUALITY BAR: a skeptical design panel will compare your mockup screenshot side-by-side against four rival directions on hierarchy/typography, information design, production craft, and accessibility. Win on craft and on fitness for a dense operational tool. Avoid: muddy contrast, one-note palettes, generic Bootstrap look, dead space, and decoration that does not serve the operator.

IMPORTANT — write the FILE FIRST: your single most important action is to successfully Write the complete mockup to ${htmlPath}. Do that before anything else once you have enough context, so the deliverable survives even if the connection drops. Then Write the notes to ${notesPath}. When both files are written, reply with exactly one short line: "DONE ${dir.key}". The file on disk is the deliverable — no structured output is needed.`
}

function auditPrompt(keys) {
  return `Check which design mockups have been written to disk. A key has a VALID mockup when the file ${EXPLORE_DIR}/<key>.html exists AND is larger than 2KB (a real mockup, not a stub or partial write).

Keys to check: ${keys.join(', ')}

Run a shell command to list the directory and the byte size of each <key>.html, e.g.:
  ls -la ${EXPLORE_DIR} ; for k in ${keys.join(' ')}; do f=${EXPLORE_DIR}/$k.html; if [ -f "$f" ]; then echo "$k $(wc -c < "$f")"; else echo "$k MISSING"; fi; done

Return present (keys with a valid >2KB html), missing (all other keys), and a short details string. Do NOT create, modify, or delete any files — this is read-only.`
}

const LENSES = [
  {
    key: 'hierarchy-typography',
    title: 'Visual hierarchy & typography',
    focus: `Type scale and weight contrast, alignment and baseline rhythm, tabular numerals, scannability, clarity of primary vs secondary vs tertiary information, heading/label/value separation. Does the eye land in the right place first? Is the hierarchy effortless to read at a glance?`
  },
  {
    key: 'information-design',
    title: 'Operational information design',
    focus: `Fitness as a DENSE operator console: does it answer "what is running / blocked / needs attention" instantly? Status & uncertainty legibility (running/finished/timed-out/blocked, validated/pending/partial), data density vs breathing room, column alignment, absence of wasted space, and whether destructive actions (Interrupt/Stop) read as destructive. Reward designs an engineer could live in for hours.`
  },
  {
    key: 'production-craft',
    title: 'Production craft & credibility',
    focus: `Does it look like a real shipped product at the Linear/Stripe/Datadog/Sentry bar? Consistency and restraint of the color system, quality of borders/elevation/radii, component polish and cohesion, balance and composition of the full layout, and overall "would I believe this is a funded product" credibility. Penalize prototype tells and tasteless decoration.`
  },
  {
    key: 'accessibility-responsive',
    title: 'Accessibility & responsive quality',
    focus: `Text contrast (aim WCAG AA), status communicated by more than color alone, visible focus/interaction affordances, readable tap targets, and the quality of the 390px mobile layout (no overlap, no broken truncation, no awkward reflow). Reward designs that stay legible and usable on mobile.`
  }
]

function judgePrompt(lens, keys) {
  const shotList = keys
    .map((k) => `  - ${k}: ${SHOTS_DIR}/${k}.desktop.png  and  ${SHOTS_DIR}/${k}.mobile.png`)
    .join('\n')
  const noteList = keys.map((k) => `  - ${k}: ${EXPLORE_DIR}/${k}.notes.md`).join('\n')
  return `You are an exacting design critic on a panel choosing a production-grade design for the Capo operator dashboard (context below). Your assigned lens:

LENS: "${lens.title}"
WHAT TO WEIGH: ${lens.focus}

${CONTEXT}

EVIDENCE — you MUST Read each candidate's desktop AND mobile screenshot (use the Read tool on every PNG), and skim each candidate's design notes:
Screenshots:
${shotList}
Notes:
${noteList}

TASK: Evaluate ALL ${keys.length} candidates (${keys.join(', ')}) COMPARATIVELY on your lens only. For each, give a 0-10 score (calibrate hard: 10 = best-in-class shipped product on this lens; 5 = mediocre prototype; do not bunch scores), concrete strengths, and concrete weaknesses citing what you actually see in the screenshots. Then rank them best-to-worst on your lens, and name the single best and worst. Be specific and critical; cheap praise is useless. Return the structured result.`
}

function synthPrompt(judgeResults, dirs, keys) {
  const specPath = `${EXPLORE_DIR}/WINNER-spec.md`
  return `You are the design lead consolidating a panel's verdict into a single decision and an implementable spec for the Capo operator dashboard.

CANDIDATES: ${keys.join(', ')}
DIRECTION BRIEFS:
${dirs.map((d) => `- ${d.key} (${d.mode}): ${d.title}`).join('\n')}

PANEL RESULTS (four lenses, each with comparative scores + ranking):
${JSON.stringify(judgeResults, null, 2)}

You MAY Read the top candidates' screenshots to confirm your call:
${keys.map((k) => `  - ${SHOTS_DIR}/${k}.desktop.png`).join('\n')}

${CONTEXT}

TASKS:
1. Aggregate the four lenses into ONE overall ranking. Weight production-craft and operational-information-design most heavily (this is a serious ops tool), with hierarchy/typography and accessibility as strong supporting factors. Give each candidate an overall 0-10 score and a one-line rationale. Resolve disagreement between judges explicitly.
2. Pick the WINNER and name the runners-up.
3. Write a consolidated, implementation-ready DESIGN SPEC to EXACTLY: ${specPath}
   The spec must let an engineer rebuild the real dashboard (web/dashboard/styles.css + index.html + app.js, which renders the same fixture) to match the winner. Include:
   - Design tokens with concrete values: color roles + hex (surfaces, text, borders, the accent, and each status color for both background chip and text/dot), the type scale (sizes, weights, line-heights, where monospace is used), spacing scale, radii, border widths, and elevation/shadow values.
   - Density rules (row heights, paddings) and layout/grid for desktop and the 390px mobile breakpoint.
   - Component-by-component styling notes: app shell/header, tabs, status-strip metrics, agent list/table, session detail, command panel (incl. how destructive actions look), timeline, evidence/review/validation lanes, details drawer.
   - Status/uncertainty encoding (color + non-color signal) per state.
   - "GRAFT" section: specific superior elements from the runner-up candidates to fold into the winner.
   - Explicit do / don't list to preserve the operational, non-marketing character.
   Make the spec concrete and opinionated — values, not adjectives.

Return the structured result (set specPath to ${specPath}).`
}

// ---------------------------------------------------------------------------
// Orchestration
// ---------------------------------------------------------------------------
const KEYS = DIRECTIONS.map((d) => d.key)
const BY_KEY = Object.fromEntries(DIRECTIONS.map((d) => [d.key, d]))
log(`Exploring ${DIRECTIONS.length} design directions: ${KEYS.join(', ')}`)

// Phase 1 — Explore: one mockup per direction. Self-healing: generate, then an
// audit agent checks which .html files actually landed on disk, and we
// regenerate only the missing ones. This survives transient agent/API failures
// (a dropped agent just leaves its key "missing" and gets retried).
phase('Explore')
let missing = [...KEYS]
let auditDetails = ''
const MAX_ROUNDS = 3
for (let round = 1; round <= MAX_ROUNDS && missing.length; round += 1) {
  log(`Explore round ${round}: generating ${missing.length} mockup(s): ${missing.join(', ')}`)
  await parallel(
    missing.map((k) => () => agent(genPrompt(BY_KEY[k]), { label: `mock:${k}`, phase: 'Explore' }))
  )
  const audit = await agent(auditPrompt(KEYS), { label: `audit:r${round}`, phase: 'Explore', schema: AUDIT_SCHEMA })
  missing = (audit.missing || []).filter((k) => KEYS.includes(k))
  auditDetails = audit.details || ''
  log(`After round ${round}: ${KEYS.length - missing.length}/${KEYS.length} present` + (missing.length ? `; still missing: ${missing.join(', ')}` : ''))
}
const presentKeys = KEYS.filter((k) => !missing.includes(k))
if (missing.length) log(`WARNING: proceeding without: ${missing.join(', ')}`)
if (!presentKeys.length) {
  return { error: 'no mockups were generated', missing, auditDetails, exploreDir: EXPLORE_DIR }
}
const ACTIVE = DIRECTIONS.filter((d) => presentKeys.includes(d.key))

// Phase 2 — Render: one agent batch-screenshots every mockup (sequential single
// Chrome process inside shoot.mjs; no browser contention).
phase('Render')
const render = await agent(
  `Render every design mockup to desktop + mobile screenshots.

Run this exact command from ${ROOT} and report its output:
  node web/dashboard/scripts/shoot.mjs --out ${SHOTS_DIR} --dir ${EXPLORE_DIR}

Then confirm that for each of these keys BOTH a .desktop.png and a .mobile.png exist in ${SHOTS_DIR}:
  ${presentKeys.join(', ')}
List any missing files. Set ok=true only if all expected screenshots exist. Return the list of shot paths and the command log.`,
  { label: 'render:all', phase: 'Render', schema: RENDER_SCHEMA }
)
log(render.ok ? `Rendered ${render.shots.length} screenshots` : `Render incomplete; missing: ${(render.missing || []).join(', ')}`)

// Phase 3 — Evaluate: comparative judge panel, one lens per judge, in parallel
// (barrier: synthesis needs every lens).
phase('Evaluate')
const judgeResults = (await parallel(
  LENSES.map((lens) => () =>
    agent(judgePrompt(lens, presentKeys), { label: `judge:${lens.key}`, phase: 'Evaluate', schema: JUDGE_SCHEMA })
  )
)).filter(Boolean)
log(`Collected ${judgeResults.length}/${LENSES.length} lens evaluations`)

// Phase 4 — Synthesize: aggregate, pick winner, write the implementable spec.
phase('Synthesize')
const synthesis = await agent(synthPrompt(judgeResults, ACTIVE, presentKeys), { label: 'synthesize', phase: 'Synthesize', schema: SYNTH_SCHEMA })
log(`Winner: ${synthesis.winner}`)

return {
  exploreDir: EXPLORE_DIR,
  shotsDir: SHOTS_DIR,
  directions: DIRECTIONS.map((d) => ({ key: d.key, title: d.title, mode: d.mode })),
  generated: presentKeys,
  missing,
  render,
  judgeResults,
  synthesis
}
