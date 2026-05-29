# Capo Operator Dashboard — Design Notes

**Direction:** Refined minimal (Linear / Vercel), light mode
**key:** `linear-graphite`
**File:** `linear-graphite.html` (single self-contained file, no JS required, no network requests, system fonts only)

---

## Concept

A calm, exact, expensive-feeling operational console. Near-monochrome graphite/zinc
neutrals on near-white surfaces, hairline low-contrast borders, and a single restrained
indigo/violet accent used only for the active/primary signal (selected agent row, primary
Send button, planner activity, mode tag). Typography carries the hierarchy — weight and size
contrast do the work that color does in louder designs. Numerals are tabular throughout so
columns of counts line up to the pixel.

---

## Color tokens (by role)

### Neutral ramp (graphite / zinc)
| Token | Hex | Use |
|---|---|---|
| `--zinc-0` | `#ffffff` | raised panel surface |
| `--zinc-25` | `#fbfbfc` | table header / sunken stat cells |
| `--zinc-50` | `#f7f7f9` | app background |
| `--zinc-75` | `#f2f2f5` | sunken inputs / chips |
| `--zinc-950` | `#18181b` | strongest text, product mark |
| `--zinc-800` | `#3a3a42` | body text |
| `--zinc-600` | `#6a6a74` | muted text |
| `--zinc-500/400` | `#86868f` / `#a6a6b3` | faint / ghost labels |

### Surfaces & borders
- `--bg-app #f7f7f9`, `--bg-panel #ffffff`, `--bg-sunken #f2f2f5`, `--bg-rail #fcfcfd`
- Borders are deliberately low-contrast hairlines: `--border #e7e7ee`, `--border-strong #dcdce4`, `--border-faint #efeff3`. The right rail is separated by a single 1px grid gap, not a heavy divider.

### Accent (single, restrained)
- `--accent #5b5bd6`, `--accent-hover #4f4fcb`, `--accent-text #4a4ac0`
- `--accent-weak #eeeefb` / `--accent-weak-b #dcdcf6` for the selected-row tint and mode tag.

### Status colors (desaturated, precise — never the only signal)
| Role | Hex | Applied to |
|---|---|---|
| ok / good | `--ok #3f8f5f` (`--ok-text #2f7a4d`) | finished, passed, validated, accepted |
| warn / attention | `--warn #b06a1c` (`--warn-text #92560f`) | timed out, blocked metric, needs follow-up |
| danger / destructive | `--danger #c4493f` (`--danger-text #a93a31`) | Interrupt, Stop, blocked agent dot |
| info / in-progress | `--info #4f6bd6` (`--info-text #3f57b8`) | running, active, pending |
| caution | `--caution #9a7d22` | partial evidence, fake adapter |

---

## Type scale (tight, deliberate)
System stack only: `ui-sans-serif, system-ui, -apple-system, "Segoe UI", Roboto…`; mono is `ui-mono-space, "SF Mono", Menlo, Consolas…`.

| Step | px / weight | Use |
|---|---|---|
| 26 / 600 | metric values (the only large numerals) |
| 20 | reserved headline step |
| 16 / 600 | session-detail agent name, detail stat values |
| 14 / 600 | brand, button labels |
| 13 / 400–600 | body, results, section titles (600) |
| 12 | pills, lane rows, metadata |
| 11 / 500 uppercase, 0.05em tracking | metric + kv labels |

- Base body letter-spacing `-0.006em`; titles tighten to `-0.02em`.
- `font-variant-numeric: tabular-nums` globally; `.mono` for all identifiers (agent names, evidence/review/validation IDs, mode tag, timestamps) so IDs read as data, not prose.

## Spacing / radii / elevation
- Spacing scale 4 / 8 / 12 / 16 / 20 / 24 / 32. Column padding 24px; intra-section gap 20px.
- Radii: `--r-sm 5` (tags), `--r-md 7` (buttons/inputs), `--r-lg 10` (panels), pill 999.
- Elevation is faint shadow, not border weight: `--sh-1` (resting panels/buttons), `--sh-2`, `--sh-pop`. Raised surfaces float on hairline borders + a 1px/4px soft shadow.

---

## Layout strategy
- Centered 1440px app frame; sticky 52px top bar (brand → view label → server/mode badge → segmented tabs → Refresh/Details).
- **Status strip:** full-width 6-column grid of metrics directly under the bar — the at-a-glance answer to "what's running." Each metric is label + 26px value + sub-line.
- **Main:** two columns separated by a 1px grid gap — left `1fr` (agents table, lanes, activity, goal), right fixed `392px` rail (session detail + command panel) on a faintly cooler `--bg-rail`. This fills the 1440 frame top-to-bottom with no dead zones: the dense table and three lanes carry width, the rail carries depth.
- Footer note anchors the bottom with project id, mode, and last-sync time.
- **390px:** one `@media (max-width:900px)` query — top bar wraps with full-width tabs, status strip → 3 columns, main collapses to one column, lanes stack, and the agents table reflows from `<table>` to stacked block rows (headers hidden). Reads acceptably without horizontal scroll.

---

## Signature moves
1. **Tabular numerals as a design element** — all counts, times, and the 6 big metrics align to a baseline grid; the strip reads like an instrument panel.
2. **Selected-row treatment** — the active agent (`capo-operator`) gets the accent tint plus a 2px inset accent bar (`box-shadow: inset 2px 0`), so selection survives grayscale.
3. **Confidence as a 3-bar equalizer** — `high/medium/low` shown as filled mini-bars, not just a word; color is secondary.
4. **Compact "Ev · Rv · Vl" count cell** — each agent row ends with three icon+number chips (evidence/reviews/validations); zero-counts dim to ghost gray so the eye lands on what exists.
5. **Quiet timeline** — a hairline spine with per-kind node colors (planner=accent ring, validation=ok ring, blocker=warn square) and small uppercase kind tags.
6. **Destructive affordance** — Interrupt and Stop are outlined danger buttons (red text, red-tinted hover, square/pause glyphs) clearly separated from the indigo primary Send by a flex spacer.

---

## Encoding status & uncertainty without relying on color alone
Color is always paired with at least one of: a text label, a glyph/shape, or a dot — per the brief's constraint.
- **Agent status:** distinct pill *shape + icon + label* per state — finished = checkmark, timed out = clock, running = animated pulsing dot, blocked = danger glyph. The leading row dot also changes *shape* (square for timed-out) not just hue.
- **"Blocked 1" reads as warning, not neutral:** the metric gets a left warning bar, a warm gradient wash, a round (vs. square) tick, an inline triangle alert glyph, and a "needs attention" sub-line — so it stands out even in grayscale.
- **Lane rows:** validated/passed/accepted = ok pill with dot; needs-follow-up/pending = info/warn pill with dot; partial = a half-filled pie glyph + amber pill, distinguishable by shape.
- **Goal requirements:** done = filled check circle, todo = dashed open circle — shape carries completeness.
- **Confidence:** equalizer bar count encodes level independent of color.
- **Adapter `fake`** is tinted caution and title-tagged "Mock adapter" to distinguish it from real `codex_exec` adapters.

All fixture content (names, counts, statuses, copy, timestamps) is used verbatim.
