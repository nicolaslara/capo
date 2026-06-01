# Design notes — "Developer / terminal-native (IDE)"

key: `terminal-native` · dark mode · single self-contained HTML, no JS, no network, system fonts only.

## Concept
A polished IDE/terminal panel for an operator babysitting several coding agents. UI labels live in a clean
system sans; everything an operator *scans* — agent ids, statuses, metrics, evidence ids, timestamps,
confidence — is in `ui-monospace` so columns align and numbers read like a console. The chrome borrows from
VS Code / JetBrains: a top tab strip, command-prompt-styled `>` inputs, glyph-prefixed status
(`[●] [!] [✓]`), and a bottom IDE status bar. No scanlines, no CRT skeuomorphism.

## Tokens

### Color (by role)
Surfaces (near-black editor stack):
- `--bg-0 #07090e` app backdrop · `--bg-1 #0b0e14` primary panel (the spec's editor bg)
- `--bg-2 #0f131b` raised/header · `--bg-3 #141a24` rows & inputs · `--bg-hover #1a212d` · `--bg-active #18202c` selected
Lines: `--line #1d242f` · `--line-strong #2a333f` · `--line-soft #161c25`
Text: `--fg #e6edf3` / `--fg-1 #aeb9c5` / `--fg-2 #7b8794` / `--fg-3 #55606d` (4-step ramp).

Semantic terminal palette (high-contrast, role-mapped):
- green `#3fb950` — ok / finished / validated / passed / accepted
- amber `#d29922` — attention & caution: blocked metric, timed-out blocker note, partial / pending / needs-follow-up
- red `#f85149` — destructive controls (Interrupt/Stop), timed-out glyph, blocker box
- cyan `#39c5cf` — running / active / info, the command prompt accent
- violet `#a371f7` — brand mark, selection, planner events (kept off-semantic so it never competes with status)

Each color ships as a 12–14% tinted fill (`*-bg`) and a ~35% border (`*-ln`) so chips read as quiet labeled
tags, not loud blocks.

### Type
System stacks only. Sans: `ui-sans-serif, system-ui, …`. Mono: `ui-monospace, "SF Mono", "JetBrains Mono", Menlo, …`
with `font-feature-settings: "tnum","zero"` for aligned tabular digits + slashed zero.
Scale: 11 / 12 / 13 (base) / 14 / 16 / 19, plus a 26px tabular metric numeral. Tight tracking on headings
(-0.01em), +0.07em uppercase tracking on tiny eyebrow labels.

### Spacing / radius / elevation
4-based spacing (4/8/12/16/20/24). Radii 4 / 6 / 9 (chips use 999px pills). One restrained elevation token
plus a cyan focus ring (`0 0 0 3px rgba(57,197,207,.45)`); destructive controls get a red focus ring.

## Layout strategy
- 1440px shell, 3 row-bands: top bar (52px) → 6-up status strip → main grid → IDE status bar (full-bleed footer).
- Main = two columns `1.55fr / 0.95fr` separated by 1px hairlines (panels sit on a `--line-soft` grid so
  every seam is a crisp single pixel, very IDE).
- Left col: the agent table (the answer to "what's running?") stacked over the activity timeline.
- Right col: session detail (selected `capo-operator` — result, goal, confidence, and the three lanes) over
  the command panel. This keeps "what's the latest result / evidence" adjacent to "what can I do next."
- Vertical fill: table rows, timeline, three lanes and the steer panel are sized to fill 1440×~900 with no
  dead zones; the footer status bar anchors the bottom edge.
- 390px: one `@media (max-width:900px)` query. Status strip → 3×2. Main → single column. The agent table
  reflows from a 5-column grid to a stacked card-ish layout via `grid-template-areas` (status glyph + name +
  chip on top, result and counts beneath); the adapter column drops. Topbar wraps, tabs go full width.

## Signature moves
- **Command-prompt headers**: every panel header is `$ agents --watch`, `$ activity --tail 3`, `> steer`.
  The steer textarea has a real `>` PS1 prefix baked into the input.
- **Glyph-prefixed status everywhere**: `[✓] [!] [◐] ⨯` rendered in mono, so status survives greyscale.
- **Animated running node**: a single CSS pulse ring on `running` agents (cyan), disabled under
  `prefers-reduced-motion`. Finished/timed-out are static glyphs — motion itself signals "live."
- **Confidence meter**: 3 segmented bars (low=1 red, medium=2 amber, high=3 green) + a mono label, so
  uncertainty is shape + count + color, never color alone.
- **IDE status bar** at the bottom: connection, mode `fixture`, blocked count, active/finished tally,
  project id, updated timestamp — the persistent at-a-glance line an engineer reads without scrolling.
- **Count pills** `e2 / r1` in the table: zero counts dim to `--fg-3`; non-zero evidence borders cyan,
  reviews border amber, so the eye lands on agents that actually have artifacts to inspect.

## Status & uncertainty without relying on color
Every state is encoded ≥2 ways:
- **running** — cyan + pulsing ring node + "running" label + glyph.
- **finished** — green + `✓` glyph + "finished" label.
- **timed out** — red `⨯` glyph in the status cell + an inline `[!] Provider run exceeded the configured
  timeout.` blocker line in the row.
- **blocked metric** — amber number **plus** a left accent bar, a tinted background wash, and an explicit
  `needs attention` flag pill, and `role="status"`. It can never read as neutral.
- **validated / passed / accepted** — green `✓`. **partial / pending** — amber `◐` (half-filled glyph).
  **needs follow-up** — amber `!`. The half-circle `◐` deliberately differs in *shape* from the full `✓`.
- **destructive controls** — Interrupt/Stop are outlined/filled red with red focus rings, distinct icons
  (pause bars / stop square), and a literal hint line stating they require confirmation; Send is the only
  filled-green primary, so the safe vs. destructive split is unmistakable.

## Why it fits a dense operational tool
No hero, no marketing type, no nested cards, no gradient orbs. Information density is high but banded by 1px
seams and a strict 4-step text ramp, so the operator scans agents → result → evidence → action top-to-bottom.
Tabular mono numerals and aligned id columns make multi-agent state legible at a glance — Datadog/Grafana
density with Linear/Vercel restraint, in a terminal-native skin.
