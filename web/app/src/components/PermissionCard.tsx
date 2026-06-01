import { ShieldAlert, Check, X } from 'lucide-react'
import { cn } from '@/lib/cn'
import { Chip, Button } from './ui'
import type { PermissionRequest, Risk } from '@/data/types'

const RISK_TONE: Record<Risk, 'green' | 'amber' | 'red'> = { low: 'green', medium: 'amber', high: 'red' }
const RISK_ACCENT: Record<Risk, string> = { low: 'bg-green', medium: 'bg-amber', high: 'bg-red' }

export function PermissionCard({
  req,
  decided,
  onDecide,
}: {
  req: PermissionRequest
  decided?: 'once' | 'always' | 'reject'
  onDecide: (d: 'once' | 'always' | 'reject') => void
}) {
  return (
    <div className="relative overflow-hidden rounded-lg border border-line bg-surface-2 p-3">
      <span className={cn('absolute inset-y-0 left-0 w-0.5', RISK_ACCENT[req.risk])} />
      <div className="flex items-center gap-2">
        <ShieldAlert size={14} className="text-amber" />
        <span className="mono text-[12px] font-semibold text-fg">{req.tool}</span>
        <Chip mono tone="muted">{req.scope}</Chip>
        <Chip tone={RISK_TONE[req.risk]}>{req.risk} risk</Chip>
        <span className="mono ml-auto text-[11px] text-fg-3">{req.source} · {req.agent}</span>
      </div>
      <p className="mono mt-2 text-[12px] text-fg-1">{req.detail}</p>
      {decided ? (
        <div className="mono mt-2.5 text-[12px]">
          <span className={decided === 'reject' ? 'text-red' : 'text-green'}>
            {decided === 'reject' ? '⨯ rejected' : `✓ granted (${decided})`}
          </span>
        </div>
      ) : (
        <div className="mt-2.5 flex items-center gap-2">
          <Button variant="primary" onClick={() => onDecide('once')}><Check size={13} /> Allow once</Button>
          <Button variant="subtle" onClick={() => onDecide('always')}>Always</Button>
          <Button variant="danger" onClick={() => onDecide('reject')}><X size={13} /> Reject</Button>
        </div>
      )}
    </div>
  )
}
