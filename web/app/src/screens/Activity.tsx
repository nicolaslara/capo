import { useMemo, useState } from 'react'
import { Search } from 'lucide-react'
import { useStore } from '@/data/store'
import { Panel, PanelHeader, Chip } from '@/components/ui'
import { cn } from '@/lib/cn'

function kindTone(kind: string): 'green' | 'amber' | 'red' | 'cyan' | 'violet' | 'muted' {
  switch (kind) {
    case 'validation': case 'gate': return 'green'
    case 'blocker': return 'red'
    case 'planner': return 'violet'
    case 'steer': case 'tool': return 'cyan'
    case 'interrupt': case 'stop': return 'red'
    case 'permission': return 'amber'
    default: return 'muted'
  }
}

export function Activity() {
  const { data } = useStore()
  const [q, setQ] = useState('')
  const [kind, setKind] = useState<string>('all')

  const kinds = useMemo(() => ['all', ...Array.from(new Set(data.activity.map((e) => e.kind)))], [data.activity])
  const rows = data.activity.filter((e) => {
    if (kind !== 'all' && e.kind !== kind) return false
    if (q && !`${e.agent} ${e.kind} ${e.text}`.toLowerCase().includes(q.toLowerCase())) return false
    return true
  })

  return (
    <div className="p-4">
      <Panel>
        <PanelHeader
          prompt="$ activity --tail"
          right={
            <div className="flex items-center gap-2">
              <div className="flex items-center gap-1.5 rounded-md border border-line bg-surface-3 px-2 py-1">
                <Search size={12} className="text-fg-3" />
                <input
                  value={q}
                  onChange={(e) => setQ(e.target.value)}
                  placeholder="filter…"
                  className="mono w-32 bg-transparent text-[11px] text-fg outline-none placeholder:text-fg-3"
                />
              </div>
            </div>
          }
        />
        <div className="flex flex-wrap items-center gap-1.5 border-b border-line px-3.5 py-2">
          {kinds.map((k) => (
            <button
              key={k}
              onClick={() => setKind(k)}
              className={cn(
                'mono rounded-full border px-2 py-0.5 text-[11px] transition-colors',
                kind === k ? 'border-cyan-ln bg-cyan-bg text-cyan' : 'border-line text-fg-2 hover:bg-surface-hover',
              )}
            >
              {k}
            </button>
          ))}
        </div>
        <div className="divide-y divide-line overflow-x-auto">
          {rows.map((e) => (
            <div key={e.id} className="grid min-w-[600px] grid-cols-[52px_44px_96px_minmax(0,1fr)_140px] items-center gap-3 px-3.5 py-2">
              <span className="mono text-[11px] text-fg-3">{e.time}</span>
              <span className="mono text-[11px] text-fg-3">#{e.sequence}</span>
              <Chip tone={kindTone(e.kind)} mono>{e.kind}</Chip>
              <span className="min-w-0 truncate text-[12px] text-fg-1">{e.text}</span>
              <span className="mono truncate text-right text-[11px] text-fg-2">{e.agent}</span>
            </div>
          ))}
          {rows.length === 0 && <div className="px-3.5 py-8 text-center text-[12px] text-fg-3">No events match.</div>}
        </div>
      </Panel>
    </div>
  )
}
