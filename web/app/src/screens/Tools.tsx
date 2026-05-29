import { useState } from 'react'
import { useStore } from '@/data/store'
import { Panel, PanelHeader, Chip } from '@/components/ui'
import { PermissionCard } from '@/components/PermissionCard'
import type { Risk, Instrumentation } from '@/data/types'

const RISK_TONE: Record<Risk, 'green' | 'amber' | 'red'> = { low: 'green', medium: 'amber', high: 'red' }
const STATUS_TONE: Record<string, 'green' | 'amber' | 'red' | 'muted'> = {
  enabled: 'green', gated: 'amber', disabled: 'muted',
}
const INSTR_LABEL: Record<Instrumentation, string> = {
  full: 'full', structured_observed: 'structured', text_observed: 'text', none: 'none',
}

export function Tools() {
  const { data, decidePermission } = useStore()
  const [decided, setDecided] = useState<Record<string, 'once' | 'always' | 'reject'>>({})

  return (
    <div className="grid grid-cols-1 gap-3 p-4 xl:grid-cols-[380px_minmax(0,1fr)]">
      {/* pending permissions */}
      <Panel className="h-fit">
        <PanelHeader prompt="$ permissions --pending" right={<span className="mono text-[11px] text-amber">{data.permissions.length}</span>} />
        <div className="flex flex-col gap-2.5 p-3.5">
          {data.permissions.length === 0 ? (
            <div className="py-8 text-center text-[12px] text-fg-3">No pending permission requests.</div>
          ) : (
            data.permissions.map((p) => (
              <PermissionCard
                key={p.id}
                req={p}
                decided={decided[p.id]}
                onDecide={(d) => { setDecided((s) => ({ ...s, [p.id]: d })); decidePermission(p.id, d) }}
              />
            ))
          )}
        </div>
      </Panel>

      {/* tool catalog */}
      <Panel className="h-fit">
        <PanelHeader prompt="$ tools --catalog" right={<span className="mono text-[11px] text-fg-3">{data.tools.length} tools</span>} />
        <div className="overflow-x-auto">
        <div className="grid min-w-[720px] grid-cols-[minmax(0,1fr)_120px_80px_90px_110px_90px] items-center gap-3 border-b border-line px-3.5 py-2 text-[10px] uppercase tracking-[0.06em] text-fg-3">
          <span>tool</span><span>origin</span><span>risk</span><span>exposure</span><span>instrumentation</span><span className="text-right">status</span>
        </div>
        <div className="min-w-[720px] divide-y divide-line">
          {data.tools.map((t) => (
            <div key={t.id} className="grid grid-cols-[minmax(0,1fr)_120px_80px_90px_110px_90px] items-center gap-3 px-3.5 py-2.5">
              <span className="mono truncate text-[12px] text-fg">{t.name}</span>
              <span className="mono truncate text-[11px] text-fg-2">{t.origin}</span>
              <Chip tone={RISK_TONE[t.risk]}>{t.risk}</Chip>
              <span className="mono text-[11px] text-fg-2">{t.exposure}</span>
              <span className="mono text-[11px] text-fg-2">{INSTR_LABEL[t.instrumentation]}</span>
              <div className="flex justify-end"><Chip tone={STATUS_TONE[t.status] ?? 'muted'}>{t.status}</Chip></div>
            </div>
          ))}
        </div>
        </div>
      </Panel>
    </div>
  )
}
