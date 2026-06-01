import { Fragment } from 'react'
import { cn } from '@/lib/cn'
import type { DispatchState, DispatchStageState } from '@/data/types'

const STAGE_META: Record<DispatchStageState, { glyph: string; dot: string; text: string; ring: string }> = {
  done: { glyph: '✓', dot: 'bg-green', text: 'text-green', ring: 'border-green-ln bg-green-bg' },
  active: { glyph: '◐', dot: 'bg-cyan', text: 'text-cyan', ring: 'border-cyan-ln bg-cyan-bg' },
  pending: { glyph: '○', dot: 'bg-fg-3', text: 'text-fg-3', ring: 'border-line bg-surface-3' },
  blocked: { glyph: '!', dot: 'bg-amber', text: 'text-amber', ring: 'border-amber-ln bg-amber-bg' },
  failed: { glyph: '⨯', dot: 'bg-red', text: 'text-red', ring: 'border-red-ln bg-red-bg' },
  none: { glyph: '·', dot: 'bg-line-strong', text: 'text-fg-3', ring: 'border-line bg-surface-2' },
}

const STAGES: { key: keyof Pick<DispatchState, 'plan' | 'preflight' | 'gate' | 'run'>; label: string }[] = [
  { key: 'plan', label: 'Plan' },
  { key: 'preflight', label: 'Preflight' },
  { key: 'gate', label: 'Gate' },
  { key: 'run', label: 'Run' },
]

export function DispatchPipeline({ dispatch }: { dispatch: DispatchState }) {
  return (
    <div>
      <div className="flex items-center">
        {STAGES.map((s, i) => {
          const state = dispatch[s.key]
          const m = STAGE_META[state]
          const pulse = state === 'active'
          return (
            <Fragment key={s.key}>
              <div className="flex flex-col items-center gap-1.5">
                <span
                  className={cn('mono grid size-7 place-items-center rounded-full border text-[12px] font-bold', m.ring, m.text)}
                  style={pulse ? { animation: 'capo-pulse 1.8s ease-out infinite' } : undefined}
                >
                  {m.glyph}
                </span>
                <span className={cn('text-[11px] font-medium', state === 'pending' || state === 'none' ? 'text-fg-3' : 'text-fg-1')}>
                  {s.label}
                </span>
              </div>
              {i < STAGES.length - 1 && (
                <span className={cn('mx-1 h-px flex-1', state === 'done' ? 'bg-green-ln' : 'bg-line')} />
              )}
            </Fragment>
          )
        })}
      </div>
      <div className="mono mt-3 flex flex-wrap items-center gap-x-4 gap-y-1 text-[11px] text-fg-2">
        {dispatch.gateStatus && (
          <span>gate: <span className="text-fg-1">{dispatch.gateStatus}</span></span>
        )}
        {dispatch.nextAction && (
          <span>next: <span className="text-cyan">{dispatch.nextAction}</span></span>
        )}
        {dispatch.credentialScan && (
          <span>
            cred-scan:{' '}
            <span className={dispatch.credentialScan === 'clean' ? 'text-green' : 'text-amber'}>
              {dispatch.credentialScan}
            </span>
          </span>
        )}
        <span>
          cli-exec: <span className="text-fg-1">{dispatch.providerCliExecuted ? 'yes' : 'no'}</span>
        </span>
      </div>
    </div>
  )
}
