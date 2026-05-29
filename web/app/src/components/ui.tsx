import type { ReactNode, ButtonHTMLAttributes } from 'react'
import { cn } from '@/lib/cn'
import type { AgentStatus, Confidence } from '@/data/types'

/* ---- tone system (literal class strings so Tailwind can see them) ---- */
type ToneKey = 'green' | 'amber' | 'red' | 'cyan' | 'violet' | 'muted'
const TONE: Record<ToneKey, { dot: string; text: string; chip: string }> = {
  green: { dot: 'bg-green', text: 'text-green', chip: 'bg-green-bg border-green-ln text-green' },
  amber: { dot: 'bg-amber', text: 'text-amber', chip: 'bg-amber-bg border-amber-ln text-amber' },
  red: { dot: 'bg-red', text: 'text-red', chip: 'bg-red-bg border-red-ln text-red' },
  cyan: { dot: 'bg-cyan', text: 'text-cyan', chip: 'bg-cyan-bg border-cyan-ln text-cyan' },
  violet: { dot: 'bg-violet', text: 'text-violet', chip: 'bg-violet-bg border-violet-ln text-violet' },
  muted: { dot: 'bg-fg-3', text: 'text-fg-2', chip: 'bg-surface-3 border-line text-fg-2' },
}

const STATUS_META: Record<AgentStatus, { tone: ToneKey; glyph: string; label: string }> = {
  running: { tone: 'cyan', glyph: '◐', label: 'running' },
  finished: { tone: 'green', glyph: '✓', label: 'finished' },
  'timed out': { tone: 'red', glyph: '⨯', label: 'timed out' },
  blocked: { tone: 'amber', glyph: '!', label: 'blocked' },
  available: { tone: 'muted', glyph: '○', label: 'available' },
  paused: { tone: 'amber', glyph: '‖', label: 'paused' },
}

export function statusTone(status: AgentStatus): ToneKey {
  return STATUS_META[status]?.tone ?? 'muted'
}

/* ---- Panel ---- */
export function Panel({ className, children }: { className?: string; children: ReactNode }) {
  return (
    <section
      className={cn(
        'flex min-h-0 min-w-0 flex-col rounded-lg border border-line bg-surface-1 shadow-[var(--shadow-sm)]',
        className,
      )}
    >
      {children}
    </section>
  )
}

export function PanelHeader({
  prompt,
  title,
  right,
}: {
  prompt?: string
  title?: ReactNode
  right?: ReactNode
}) {
  return (
    <header className="flex h-10 shrink-0 items-center gap-2 border-b border-line px-3.5">
      {prompt && <span className="mono text-[12px] text-cyan select-none">{prompt}</span>}
      {title && <span className="mono text-[12px] text-fg-2">{title}</span>}
      {right && <div className="ml-auto flex items-center gap-2">{right}</div>}
    </header>
  )
}

export function PanelBody({ className, children }: { className?: string; children: ReactNode }) {
  return <div className={cn('min-h-0 flex-1 p-3.5', className)}>{children}</div>
}

/* ---- Status ---- */
export function StatusDot({ status, className }: { status: AgentStatus; className?: string }) {
  const t = STATUS_META[status]?.tone ?? 'muted'
  const pulse = status === 'running'
  return (
    <span
      className={cn('inline-block size-2 rounded-full', TONE[t].dot, className)}
      style={pulse ? { animation: 'capo-pulse 1.8s ease-out infinite' } : undefined}
    />
  )
}

export function StatusBadge({ status }: { status: AgentStatus }) {
  const m = STATUS_META[status] ?? STATUS_META.available
  return (
    <span
      className={cn(
        'inline-flex items-center gap-1.5 whitespace-nowrap rounded-full border px-2 py-0.5 text-[11px] font-medium',
        TONE[m.tone].chip,
      )}
    >
      <StatusDot status={status} />
      <span className="mono">{m.glyph}</span>
      {m.label}
    </span>
  )
}

/* ---- Chip / tag ---- */
export function Chip({
  children,
  tone = 'muted',
  mono,
  className,
}: {
  children: ReactNode
  tone?: ToneKey
  mono?: boolean
  className?: string
}) {
  return (
    <span
      className={cn(
        'inline-flex items-center gap-1 rounded-full border px-2 py-0.5 text-[11px]',
        TONE[tone].chip,
        mono && 'mono',
        className,
      )}
    >
      {children}
    </span>
  )
}

/* count pill like `e2 / r1`, dimmed when zero */
export function CountPill({ label, value, tone = 'cyan' }: { label: string; value: number; tone?: ToneKey }) {
  const active = value > 0
  return (
    <span
      className={cn(
        'mono inline-flex items-center rounded border px-1.5 py-0.5 text-[11px]',
        active ? TONE[tone].chip : 'border-line text-fg-3',
      )}
      title={`${value} ${label}`}
    >
      {label}
      {value}
    </span>
  )
}

/* ---- Metric ---- */
const METRIC_TONE: Record<string, string> = {
  default: 'text-fg',
  good: 'text-green',
  warn: 'text-amber',
  info: 'text-cyan',
  danger: 'text-red',
}
export function Metric({
  label,
  value,
  tone = 'default',
  hint,
}: {
  label: string
  value: number | string
  tone?: 'default' | 'good' | 'warn' | 'info' | 'danger'
  hint?: string
}) {
  const warn = tone === 'warn' || tone === 'danger'
  return (
    <div
      className={cn(
        'relative flex min-w-0 flex-col justify-between rounded-lg border bg-surface-1 px-3.5 py-3',
        warn ? 'border-amber-ln bg-amber-bg' : 'border-line',
      )}
    >
      {warn && <span className="absolute inset-y-2 left-0 w-0.5 rounded-full bg-amber" />}
      <span className={cn('mono text-[26px] font-semibold leading-none tracking-tight', METRIC_TONE[tone])}>
        {value}
      </span>
      <div className="mt-2 flex min-w-0 items-center justify-between gap-2">
        <span className="truncate text-[11px] uppercase tracking-[0.07em] text-fg-2">{label}</span>
        {hint && <span className="mono hidden truncate text-[10px] text-amber sm:inline">{hint}</span>}
      </div>
    </div>
  )
}

/* ---- Confidence meter: shape + count + color (never color alone) ---- */
export function ConfidenceMeter({ value }: { value: Confidence }) {
  const n = value === 'high' ? 3 : value === 'medium' ? 2 : 1
  const tone: ToneKey = value === 'high' ? 'green' : value === 'medium' ? 'amber' : 'red'
  return (
    <span className="inline-flex items-center gap-1.5" title={`confidence: ${value}`}>
      <span className="flex items-center gap-0.5">
        {[0, 1, 2].map((i) => (
          <span
            key={i}
            className={cn('h-2.5 w-1.5 rounded-[1px]', i < n ? TONE[tone].dot : 'bg-line-strong')}
          />
        ))}
      </span>
      <span className={cn('mono text-[11px]', TONE[tone].text)}>{value}</span>
    </span>
  )
}

/* ---- Button ---- */
type ButtonVariant = 'primary' | 'ghost' | 'danger' | 'subtle'
const BTN: Record<ButtonVariant, string> = {
  primary: 'bg-green/15 border-green-ln text-green hover:bg-green/25',
  subtle: 'bg-surface-3 border-line text-fg-1 hover:bg-surface-hover',
  ghost: 'bg-transparent border-line text-fg-1 hover:bg-surface-hover',
  danger: 'bg-red-bg border-red-ln text-red hover:bg-red/20',
}
export function Button({
  variant = 'subtle',
  className,
  children,
  ...rest
}: { variant?: ButtonVariant } & ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button
      className={cn(
        'inline-flex h-8 items-center justify-center gap-1.5 rounded-md border px-3 text-[12px] font-medium',
        'transition-colors outline-none focus-visible:ring-2 focus-visible:ring-cyan/50 disabled:opacity-50',
        BTN[variant],
        className,
      )}
      {...rest}
    >
      {children}
    </button>
  )
}

export function Kbd({ children }: { children: ReactNode }) {
  return (
    <kbd className="mono rounded border border-line bg-surface-3 px-1.5 py-0.5 text-[10px] text-fg-2">
      {children}
    </kbd>
  )
}

export function Eyebrow({ children }: { children: ReactNode }) {
  return <div className="text-[11px] font-medium uppercase tracking-[0.07em] text-fg-2">{children}</div>
}
