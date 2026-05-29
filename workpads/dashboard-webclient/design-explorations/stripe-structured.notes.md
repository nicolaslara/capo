# Capo Operator Dashboard — "Structured product (Stripe / Sentry)"

key: `stripe-structured` · light mode · single self-contained HTML file

## Intent
A mature, branded, light-mode operational console. Clean white surfaces float on a
faint cool-grey canvas; every region is a clearly bordered, lightly elevated panel
with a strong section header and supporting metadata. The feeling is a commercial
dashboard you'd trust with production infra: organized, legible, dense without being
noisy. No hero, no gradients-as-decoration, no nested-card clutter.

## Design tokens

### Color — by role (hex)
Canvas / surfaces
- `--canvas` #f5f6f8 (app background, faint cool grey)
- `--surface` #ffffff (panels/cards)
- `--surface-2` #fbfbfd (table head, inset rails, textarea)
- `--surface-3` #f3f4f7 (hover / selected wash)

Borders
- `--border` #e6e8ee (default hairline)
- `--border-strong` #d6d9e2 (controls, emphasized)
- `--border-faint` #eef0f4 (inner row dividers)

Ink (4-step text ramp)
- `--ink` #1a1f2b · `--ink-2` #545b6b · `--ink-3` #7c8494 · `--ink-4` #9aa1b1

Brand primary (indigo)
- `--brand` #4f46e5 · `--brand-strong` #4338ca
- `--brand-weak` #eef0fe (tint surface) · `--brand-line` #d9dbfb

Full semantic system (text / bg / line / dot per role)
- green/OK: #1a7f4b / #e8f6ee / #b9e3c9 / #22a565
- amber/WARN: #9a6212 / #fdf3e1 / #f3dca6 / #d98a13
- red/DANGER: #b3261e / #fdecec / #f4c5c2 / #db453b
- blue/INFO: #1f5fb8 / #e9f1fc / #c2d8f4 / #2f74d0
- neutral: #545b6b / #eef0f4 / #dfe2ea / #8b93a4

All status text colors are dark enough on their tint backgrounds to clear WCAG AA
for small text; ink-2/ink-3 on white also pass AA.

### Type scale
System stack only: `ui-sans-serif, system-ui, …` for prose, `ui-mono…` for IDs/code.
Steps: 11 / 12 / 13 / 14 (base) / 16 / 18 / 22 px. Weights 450–680. Tight negative
tracking on the larger sizes (h1 -0.012em, metric value -0.02em); slight positive
tracking + uppercase on the 11px micro-labels. Monospace is used deliberately as a
semantic signal: it marks machine identity (agent names, evidence/validation IDs,
adapters, the `fixture` mode chip, log line) versus human prose.

### Spacing / radius / elevation
- Spacing rhythm: 4 / 6 / 8 / 12 / 14 / 16 / 24 px.
- Radii: 6 (controls/chips), 8 (inset groups), 10 (metric cards), 12 (panels), pill.
- Elevation is soft and single-direction (downward): sm `0 1px 2px`, md adds
  `0 4px 12px`, both at ~5% ink. No hard shadows, no glows.

## Layout strategy
- Fixed 1440-max app frame, full-height flex column so the footer pins to the bottom
  and the content fills top-to-bottom with no dead zones.
- App shell: 56px sticky top bar (brand mark + "Operator Dashboard" + Overview/Goals/
  Settings tabs + env badge + Refresh/Details). A secondary context strip carries the
  H1, breadcrumb ("supervising 5 coding agents") and a live "Updated …" stamp.
- Status strip: a 6-up metric grid directly under the context strip — fast top-line scan.
- Body: an asymmetric 1.55fr / 1fr two-column grid. Left = the wide Agents table plus
  the three Evidence/Reviews/Validation lanes (3-up). Right = the operational stack the
  operator acts in: Session detail → Command panel → Recent activity timeline.
- Degrades through one `@media (max-width:900px)` query: top bar wraps, metrics go 2-up,
  the main grid and lanes collapse to a single column, detail counters go 2-up. Reads
  cleanly at 390px.

## Signature moves
- **Branded, structured chrome**: indigo brand mark + active tab in a tinted indigo
  pill; panels are real bordered sections with a header row (icon + title + right-aligned
  metadata `count-pill`s), echoing Stripe/Sentry section headers.
- **Aligned numeric table**: Ev / Rv / Vl columns are right-aligned, tabular-nums, with
  count chips that visibly dim to ink-4 when zero — so "0 reviews" reads as absence, not
  a typo.
- **Selected-row treatment**: the chosen session (capo-operator) gets a brand-weak wash
  plus a 3px inset brand bar on the leading cell, tying the table to the detail panel.
- **Confidence as a 3-bar meter** (not just a word): low/med/high lit bars colored
  danger/amber/green, with the text label beside it.
- **Timeline rail**: a real connector line with kind-colored nodes (planner=indigo,
  validation=green, blocker=amber) plus a textual kind tag.

## Status & uncertainty encoding (never color-only)
Every state carries at least two of: a worded label, a shaped glyph, a colored dot, and
a tint background.
- running → blue, solid filled (pulsing) dot + "Running"
- finished → green, ring+check glyph + "Finished"
- timed out → amber, clock glyph + "Timed out"
- blocked → red, triangle/alert glyph + "Blocked"
- validated / passed / accepted → green check + word
- needs follow-up / partial → amber alert-dot glyph + word
- pending → neutral grey clock glyph + word
The **Blocked = 1** metric is explicitly elevated to a warning treatment: amber tint
card, amber 3px left accent bar, amber value/label, and a "needs operator attention"
subline — so it never reads as a neutral count. **Interrupt** and **Stop** are styled as
destructive (red text, red hairline, red-tint hover) and visually separated from the
primary indigo **Send** by a flex spacer. A monospace command-log line confirms the last
dispatch with timestamp.
