# slate-editorial — Design Notes

Direction: **Calm editorial neutral** (Notion / Height / Linear-light), light mode.
A warm-neutral, low-stress operator console. Structure comes from spacing and
type, not from heavy lines. Muted teal carries identity and primary intent; status
color is muted and editorial, never neon, and never the sole signal.

## Design tokens

### Color — warm-neutral stone scale (by role)
| Role | Hex |
|---|---|
| App background (warmest) | `#fbfaf8` |
| Panel / card surface | `#ffffff` |
| Recessed surface (table head, insets) | `#f6f4f0` |
| Hover / subtle fill | `#f1eee9` |
| Hairline border (light) | `#e2ddd4` |
| Stronger border | `#d6d0c5` |
| Muted text / icons | `#a39b8b` |
| Secondary text | `#857d6e` |
| Body strong | `#514b40` |
| Primary text | `#3a352d` |
| Ink / headings | `#272219` |

### Color — primary accent (muted teal/green)
| Role | Hex |
|---|---|
| Primary (mark, Send, focus ring base) | `#3d8475` |
| Primary hover | `#336f63` |
| Primary text on light | `#28564d` |
| Tint surface | `#e9f2ef` / `#d3e6e0` |

### Color — status roles (muted, paired with non-color cues)
| State | fg / bg / border / dot |
|---|---|
| running (in-progress, blue-teal) | `#2f6c86` / `#e4eef3` / `#c5dce6` / `#3c87a6` |
| finished / ok / good (green) | `#3a6b48` / `#e7f0e7` / `#c9e0cb` / `#4f8a5f` |
| warning / attention (amber) | `#8a5a1e` / `#f6ecda` / `#e7d3ac` / `#c0832c` |
| danger / blocked-hard (clay) | `#97402f` / `#f6e3dd` / `#e8c3b8` / `#c0563f` |
| neutral / idle | stone-600 / stone-75 / stone-150 / stone-400 |

### Type
- Stacks: sans = `ui-sans-serif, system-ui, -apple-system, "Segoe UI", Roboto…`;
  mono = `ui-mono...` (used for agent names, IDs, timestamps, adapters — the
  "machine identity" layer).
- Scale (px): 11, 12, 13, 14 (base), 15, 18, 22, 28.
- Weights: 500 / 550 / 600 / 640–660. Headings tighten letter-spacing (-0.1 to
  -0.5px); body sits at +0.1px for editorial calm. Numerics use `tabular-nums`.

### Spacing / radius / border / elevation
- Spacing on a 4px base: 4 / 8 / 12 / 16 / 20 / 24 / 32 / 40.
- Radius: sm 6, md 9, lg 12, pill 999.
- Border: 1px hairline `#e2ddd4` default; 1px `#d6d0c5` for interactive controls.
- Elevation: three very soft, warm-tinted shadows (`rgba(39,34,25,…)`) — xs for
  metrics/buttons, sm for panels, md reserved. Depth is whisper-quiet by design.

## Layout strategy
- 1440px centered app; sticky 56px top bar (brand + view label / tabs / server
  badge / refresh + details).
- Below: a 6-up **status strip** of metric cards, then a 2-column work area
  (~1.55fr left : ~0.95fr right) that fills the frame:
  - **Left (scan + scale):** Agents table → 3 lane panels (Evidence / Reviews /
    Validation) → Active Goal card.
  - **Right (focus + act):** Session detail (capo-operator) → Command panel →
    Recent Activity timeline.
- This keeps "what's running / what's the evidence" on the wide left for scanning,
  and "the thing I'm steering + the controls" stacked on the right at thumb reach.
- Mobile (≤960px → 390px): single `@media` query. Columns collapse to one, the
  6-up strip becomes 2-up, the agent table reflows to stacked rows (adapter +
  confidence columns hidden, result wraps), tabs drop to a full-width row.

## Signature moves
- **Warm stone surfaces, not white.** App bg is `#fbfaf8`; cards are white and
  read as gently raised. Low-stress, "live in it for hours" feel.
- **Mono = machine identity.** Every agent name, evidence ID, target, timestamp,
  and adapter is monospace; prose is sans. The operator's eye learns the two
  registers instantly.
- **Teal as a quiet spine.** The selected agent row carries a 3px teal inset bar
  + teal-50 wash; the latest-result block has a teal left rule; Send is the only
  saturated solid. Accent is spent sparingly so it always means "primary / focus".
- **Metric strip as the alarm layer.** Five metrics are calm; **Blocked 1** is the
  one tinted amber card with an amber value, a triangle glyph, and a bold "needs
  attention" sublabel — it reads as a warning, not a neutral count. Reviews and
  Validations carry split sublabels ("1 accepted · 1 follow-up", "2 passed · 1
  pending") so the strip answers questions without a click.
- **Lanes mirror the data model.** Evidence / Reviews / Validation are three peer
  panels of compact rows (id + kind/target + status mini-pill) — directly the
  fixture's three arrays, scannable top-to-bottom.
- **Command panel reads its consequences.** Send is solid teal; Interrupt and Stop
  are clay-red — Interrupt is a tinted destructive button, Stop is a filled solid
  destructive button (a harder commit), with a small mono command-log line beneath.

## Encoding status & uncertainty without relying on color alone
Color is always one of at least two cues:
- **Status pills** pair tint with (a) a label word, (b) a shape/icon: finished =
  check, timed out = clock, running = pulsing dot, blocked = triangle. Lane
  mini-pills carry a dot + the literal status word ("validated", "partial",
  "needs follow-up", "pending", "passed", "accepted").
- **Confidence** is a 3-bar sparkline (1/2/3 bars lit) plus the word
  low/medium/high — readable in greyscale by bar count.
- **Blocked metric** combines amber tint + triangle glyph + "needs attention"
  text + a heavier value weight; even desaturated it stands apart by shape/copy.
- **Goal requirements** use filled-check (done) vs filled-dot (pending) icons in
  addition to tint, so "design accepted" vs the two pending items differ by shape.
- **Evidence/review/validation counts** use distinct outline glyphs (file / speech
  / shield) and dim to stone-300 at zero — quantity and "none" read structurally,
  not just by hue.

## Accessibility notes
- All status text/background pairs use dark muted foregrounds on light tints
  (e.g. `#8a5a1e` on `#f6ecda`, `#3a6b48` on `#e7f0e7`) targeting AA for the
  small label sizes.
- Focus ring on the steer textarea is a 3px teal-50 halo + teal border (visible
  without relying on the browser default).
- Tabs expose `aria-current`; sections use `aria-label`; the pulsing running dot
  is purely decorative (`aria-hidden`) and never the only running signal.
- Tabular numerics keep metric/count columns aligned for fast vertical scanning.
