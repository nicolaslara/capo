# ops-console-dark — Design Notes

Direction: **Operational console (Datadog / Grafana), dark mode.** A mission-control
surface an engineer keeps open while several coding agents run. Optimized for repeated
scanning, glanceable status, and safe action — not for marketing.

## Design tokens

### Color (by role)
Surfaces step *up* one shade per nesting level so seams read as crisp 1px hairlines, not boxes.

| Role | Hex |
|---|---|
| App background (outermost) | `#0e1117` |
| Canvas (app frame) | `#11161f` |
| Panel | `#151b26` |
| Panel-2 (table head, lane head) | `#19202c` |
| Raised (hover) | `#1d2532` |
| Zebra row alt | `#131925` |
| Input field | `#0d121b` |
| Hairline seam | `#232c3a` / soft `#1c2431` / strong `#2d3a4d` |
| Text strong / body / muted / dim / faint | `#f4f7fb` / `#e6ebf2` / `#9aa6b8` / `#6c788c` / `#4d5869` |

Status roles, each as a **filled dot + label + desaturated chip background + border**:

| Role | Dot/Text | Chip bg | Chip border | Meaning |
|---|---|---|---|---|
| ok / good | `#36c98d` | `#11281f` | `#1d4634` | finished, validated, accepted, passed |
| info / running | `#4f9bf2` | `#0f2236` | `#1d3c5c` | running (in-progress), pending |
| warn / attention | `#e8b341` | `#2c2310` | `#4d3f17` | timed out, partial, needs-follow-up, blocked |
| danger / destructive | `#f0664f` | `#2e1714` | `#502721` | blocker, Interrupt/Stop |
| neutral | `#8492a6` | `#1a212d` | `#28323f` | absence (no reviews/blocker) |
| accent (planner) | `#7c8cf8` | — | — | planner activity kind |

Focus ring `#3b6ef0` with a 3px translucent halo. Mode tag uses the info role so
`fixture` reads as informational, never alarming.

### Type scale
System stacks only: `ui-sans-serif/system-ui` for prose, `ui-monospace` for ids, adapters,
counts, timestamps. Scale (px): 10 (uppercase labels), 11, 12 (table/body), 13 (base/result),
14 (brand/detail name), 16 (detail title + ministat numbers), 22, **28 (metric headline)**.
Tabular figures forced everywhere numbers matter (`font-variant-numeric: tabular-nums` +
`"tnum"`) so columns and metrics stay vertically aligned.

### Spacing / radius / border / elevation
- Spacing: strict 4px base — 4 / 8 / 12 / 16 / 20 / 24 / 32 / 40.
- Radius: 4 (tags/inputs), 6 (buttons/panels), 8 (rare), pill (chips/badges).
- Border: uniform 1px hairlines; emphasis via the strong seam color, not thicker rules.
- Elevation: almost flat — a 1px dark drop (`--shadow-1`) and one soft popover shadow held in reserve. Depth comes from surface-step + seams, the console idiom.

## Layout strategy
A fixed 1440px-centered app frame, full-height flex column:

1. **Top bar (52px, sticky):** brand mark + "Capo", divider, "Operator Dashboard", a server
   badge (`mock server-command API` + `fixture` mode tag), tabs (Overview/Goals/Settings),
   and right-aligned updated-clock + Refresh + Details.
2. **Status strip:** 6 equal cells in a `grid` with hairline dividers, reading like a monitoring
   header — small uppercase label, big tabular number, a sub-line, and room for a sparkline.
3. **Two-column body** `minmax(0,1fr) / 420px`:
   - **Main:** the agent table, then the activity timeline, then a 3-up Evidence/Reviews/Validation
     lane grid, then the goals row — stacked panels with shared hairline seams, no nested cards.
   - **Rail:** the selected session (capo-operator) detail — header, result + goal + confidence
     meter + 4-up ministats, evidence tags, reviews/validations, blocker state, and the command panel pinned in the same column.
4. **Status bar (30px):** connection, project id, mode, and a fleet recap with the blocked count colored amber.

The frame fills top-to-bottom: tall stacked panels in the left column absorb vertical space, and
the rail's command panel anchors the bottom-right so there are no dead zones.

## Signature moves
- **Monitoring header metrics** with 28px monospaced tabular numbers, uppercase micro-labels, and
  inline sparkline/delta room — the Datadog/Grafana "numbers you trust at a glance" feeling.
- **Surface-step + 1px seams** instead of shadows/cards. Panels are differentiated only by a one-shade
  lift and a hairline, which is what keeps density high without clutter.
- **Per-row count cluster** (`ev / rv / vd`) in mono with a tiny role-colored dot each, so the table
  doubles as a coverage matrix without a second view.
- **Selected row** marked three ways: tinted gradient, a 2px inset info bar on the left edge, and an
  explicit `SELECTED` micro-tag — never relying on background tint alone.
- **Confidence meter** as a 3-segment bar (low fills 1 / medium fills 2 / high fills 3) plus the word,
  so uncertainty is encoded by *length and label*, not just hue.
- **Live dots** for running agents/goal get a subtle pulsing ring (disabled under
  `prefers-reduced-motion`), distinguishing "running" from static "ok" beyond color.

## Status / uncertainty without color alone
Every status carries at least two non-color signals:
- **Label text** in every chip (`finished`, `running`, `timed out`, `blocked`, `validated`,
  `partial`, `needs follow-up`, `accepted`, `pending`, `passed`).
- **Shape glyphs** where the distinction is critical: `timed out` carries a clock SVG; the blocker
  uses a triangle-warning SVG; checkmarks mark met requirements; Interrupt = pause bars, Stop = filled
  square (universal transport semantics).
- **Position / structure:** the Blocked metric is the only cell with an amber left-rail accent and a
  tinted wash plus a "needs attention" sub-line — so "Blocked 1" reads as a warning, not a neutral count.
- **Confidence as length** via the segmented meter, independent of its color.
- **Destructive intent** on Interrupt/Stop uses the danger surface *and* explicit verbs *and* warning
  tooltips ("cannot be resumed"), separated from the primary Send button.

Contrast: body text `#e6ebf2` on `#11161f` (~13:1) and muted `#9aa6b8` (~6:1) clear WCAG AA; chip text
uses lightened role tints (e.g. `#7be0b4`, `#ffa593`) over their dark desaturated backgrounds to stay
legible at 11px.

## Responsive
A single `@media` breakpoint set: at ≤980px the body collapses to one column (rail drops under main,
lanes stack), the status strip becomes 3×2, the top bar wraps with tabs on their own row, and the
table sheds the free-text result column; at ≤480px the metric headline drops to 22px and the count
cluster column hides, keeping name + status + adapter legible at 390px.
