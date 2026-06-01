import { Link } from 'react-router-dom'
import { useStore } from '@/data/store'
import {
  Panel,
  PanelHeader,
  Metric,
  StatusBadge,
  ConfidenceMeter,
  CountPill,
  Chip,
} from '@/components/ui'
import type { ReactNode } from 'react'
import { cn } from '@/lib/cn'
import type { EvidenceStatus, ReviewStatus, ValidationStatus } from '@/data/types'

const EVI_TONE: Record<EvidenceStatus, 'green' | 'amber' | 'red' | 'muted'> = {
  validated: 'green', partial: 'amber', pending: 'muted', blocked: 'red',
}
const REV_TONE: Record<ReviewStatus, 'green' | 'amber' | 'muted'> = {
  accepted: 'green', 'needs follow-up': 'amber', pending: 'muted',
}
const VAL_TONE: Record<ValidationStatus, 'green' | 'amber' | 'red'> = {
  passed: 'green', pending: 'amber', failed: 'red',
}

export function Overview() {
  const { data } = useStore()

  return (
    <div className="flex flex-col gap-3 p-4">
      {/* status strip */}
      <div className="grid grid-cols-2 gap-2 sm:grid-cols-3 xl:grid-cols-6">
        {data.summary.map((m) => (
          <Metric key={m.key} label={m.label} value={m.value} tone={m.tone} hint={m.hint} />
        ))}
      </div>

      <div className="grid grid-cols-1 gap-3 xl:grid-cols-[minmax(0,1.55fr)_minmax(0,1fr)]">
        {/* left column */}
        <div className="flex min-w-0 flex-col gap-3">
          <Panel>
            <PanelHeader prompt="$ agents --watch" right={<span className="mono text-[11px] text-fg-3">{data.agents.length} agents</span>} />
            <div className="hidden grid-cols-[118px_180px_minmax(0,1fr)_104px_96px] items-center gap-3 border-b border-line px-3.5 py-2 text-[10px] uppercase tracking-[0.06em] text-fg-3 sm:grid">
              <span>status</span><span>agent</span><span>latest result</span><span>confidence</span><span className="text-right">e/r/v</span>
            </div>
            <div className="divide-y divide-line">
              {data.agents.map((a) => (
                <Link
                  key={a.id}
                  to={`/agents?sel=${a.id}`}
                  className="flex flex-col gap-2 px-3.5 py-3 transition-colors hover:bg-surface-hover sm:grid sm:grid-cols-[118px_180px_minmax(0,1fr)_104px_96px] sm:items-center sm:gap-3 sm:py-2.5"
                >
                  <StatusBadge status={a.status} />
                  <div className="min-w-0">
                    <div className="mono truncate text-[13px] text-fg">{a.name}</div>
                    <div className="mono truncate text-[11px] text-fg-2">{a.adapter}</div>
                  </div>
                  <div className="min-w-0">
                    <div className="truncate text-[12px] text-fg-1">{a.result}</div>
                    {a.blocker && (
                      <div className="mono mt-0.5 truncate text-[11px] text-red">
                        [!] {a.blocker}
                      </div>
                    )}
                  </div>
                  <ConfidenceMeter value={a.confidence} />
                  <div className="flex items-center gap-1 sm:justify-end">
                    <CountPill label="e" value={a.evidence.length} tone="cyan" />
                    <CountPill label="r" value={a.reviews} tone="amber" />
                    <CountPill label="v" value={a.validations} tone="green" />
                  </div>
                </Link>
              ))}
            </div>
          </Panel>

          <Panel>
            <PanelHeader prompt="$ activity --tail" right={<Link to="/activity" className="text-[11px] text-cyan hover:underline">view all →</Link>} />
            <ul className="divide-y divide-line">
              {data.activity.slice(0, 6).map((e) => (
                <li key={e.id} className="flex items-start gap-3 px-3.5 py-2">
                  <span className="mono w-10 shrink-0 text-[11px] text-fg-3">{e.time}</span>
                  <Chip tone={kindTone(e.kind)} mono className="shrink-0">{e.kind}</Chip>
                  <span className="mono shrink-0 text-[11px] text-fg-2">{e.agent}</span>
                  <span className="min-w-0 flex-1 truncate text-[12px] text-fg-1">{e.text}</span>
                </li>
              ))}
            </ul>
          </Panel>
        </div>

        {/* right column */}
        <div className="flex min-w-0 flex-col gap-3">
          <Panel>
            <PanelHeader prompt="$ ledgers" title="evidence · reviews · validation" />
            <div className="flex flex-col">
              <Lane label="Evidence">
                {data.evidence.map((e) => (
                  <Row key={e.id} id={e.id} meta={e.kind} tone={EVI_TONE[e.status]} status={e.status} />
                ))}
              </Lane>
              <Lane label="Reviews">
                {data.reviews.map((r) => (
                  <Row key={r.id} id={r.id} meta={r.target} tone={REV_TONE[r.status]} status={r.status} />
                ))}
              </Lane>
              <Lane label="Validation">
                {data.validations.map((v) => (
                  <Row key={v.id} id={v.id} meta={v.target} tone={VAL_TONE[v.status]} status={v.status} />
                ))}
              </Lane>
            </div>
          </Panel>

          <Panel>
            <PanelHeader prompt="$ goals" right={<Link to="/goals" className="text-[11px] text-cyan hover:underline">view all →</Link>} />
            <div className="divide-y divide-line">
              {data.goals.map((g) => {
                const done = g.requirements.filter((r) => r.done).length
                return (
                  <Link key={g.id} to="/goals" className="block px-3.5 py-2.5 transition-colors hover:bg-surface-hover">
                    <div className="flex items-center gap-2">
                      <Chip tone={g.status === 'completed' ? 'green' : g.status === 'blocked' ? 'red' : 'cyan'}>{g.status}</Chip>
                      <span className="min-w-0 flex-1 truncate text-[13px] text-fg">{g.title}</span>
                    </div>
                    <div className="mono mt-1.5 flex items-center gap-2 text-[11px] text-fg-2">
                      <span>{done}/{g.requirements.length} requirements</span>
                      <span className="text-fg-3">·</span>
                      <span>validation: {g.validation}</span>
                    </div>
                  </Link>
                )
              })}
            </div>
          </Panel>
        </div>
      </div>
    </div>
  )
}

function kindTone(kind: string): 'green' | 'amber' | 'red' | 'cyan' | 'violet' | 'muted' {
  switch (kind) {
    case 'validation': case 'gate': return 'green'
    case 'blocker': return 'red'
    case 'planner': return 'violet'
    case 'steer': case 'tool': return 'cyan'
    default: return 'muted'
  }
}

function Lane({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="border-b border-line last:border-b-0">
      <div className="px-3.5 pt-2.5 pb-1 text-[10px] font-medium uppercase tracking-[0.07em] text-fg-3">{label}</div>
      <div>{children}</div>
    </div>
  )
}

function Row({ id, meta, status, tone }: { id: string; meta: string; status: string; tone: 'green' | 'amber' | 'red' | 'muted' }) {
  return (
    <div className="flex items-center gap-2 px-3.5 py-1.5">
      <span className="mono min-w-0 flex-1 truncate text-[12px] text-fg-1">{id}</span>
      <span className="mono shrink-0 text-[11px] text-fg-3">{meta}</span>
      <Chip tone={tone} className={cn('shrink-0')}>{status}</Chip>
    </div>
  )
}
