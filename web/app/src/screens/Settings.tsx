import type { ReactNode } from 'react'
import * as Switch from '@radix-ui/react-switch'
import { useStore } from '@/data/store'
import { useTheme } from '@/lib/theme'
import { Panel, PanelHeader, PanelBody, Chip } from '@/components/ui'
import { cn } from '@/lib/cn'

export function Settings() {
  const { data } = useStore()
  const { theme, setTheme } = useTheme()
  const adapters = Array.from(new Set(data.agents.map((a) => a.adapter)))

  return (
    <div className="grid grid-cols-1 gap-3 p-4 lg:grid-cols-2">
      <Panel>
        <PanelHeader prompt="$ connection" />
        <PanelBody className="flex flex-col">
          <Row label="Server"><span className="mono text-[12px] text-fg-1">{data.project.addr}</span></Row>
          <Row label="Boundary"><span className="mono text-[12px] text-fg-1">{data.project.server}</span></Row>
          <Row label="Data mode">
            <Chip tone={data.project.mode === 'live' ? 'green' : 'amber'}>{data.project.mode}</Chip>
          </Row>
          <Row label="Live server" hint="requires the HTTP/SSE facade — coming next">
            <Toggle checked={false} disabled onChange={() => {}} />
          </Row>
          <Row label="Updates"><span className="mono text-[12px] text-fg-1">polling (SSE when wired)</span></Row>
        </PanelBody>
      </Panel>

      <Panel>
        <PanelHeader prompt="$ appearance" />
        <PanelBody className="flex flex-col">
          <Row label="Dark mode">
            <Toggle checked={theme === 'dark'} onChange={(v) => setTheme(v ? 'dark' : 'light')} />
          </Row>
          <Row label="Accent"><span className="mono text-[12px] text-violet">violet · terminal-native</span></Row>
          <Row label="Density"><span className="mono text-[12px] text-fg-1">compact</span></Row>
        </PanelBody>
      </Panel>

      <Panel>
        <PanelHeader prompt="$ safety" />
        <PanelBody className="flex flex-col">
          <Row label="Raw output" hint="redacted unless explicitly revealed"><Chip tone="green">redacted</Chip></Row>
          <Row label="Mutation policy"><span className="mono text-[12px] text-fg-1">server-command boundary</span></Row>
          <Row label="Secrets"><span className="mono text-[12px] text-fg-1">never exposed to client</span></Row>
          <Row label="Audit"><Chip tone="green">all commands logged</Chip></Row>
        </PanelBody>
      </Panel>

      <Panel>
        <PanelHeader prompt="$ adapters" />
        <PanelBody className="flex flex-col">
          {adapters.map((a) => (
            <Row key={a} label={a}>
              <span className="mono text-[12px] text-fg-2">{data.agents.filter((x) => x.adapter === a).length} agents</span>
            </Row>
          ))}
        </PanelBody>
      </Panel>
    </div>
  )
}

function Row({ label, hint, children }: { label: string; hint?: string; children: ReactNode }) {
  return (
    <div className="flex items-center justify-between gap-4 border-b border-line py-2.5 last:border-b-0">
      <div className="min-w-0">
        <div className="text-[13px] text-fg-1">{label}</div>
        {hint && <div className="text-[11px] text-fg-3">{hint}</div>}
      </div>
      <div className="shrink-0">{children}</div>
    </div>
  )
}

function Toggle({ checked, disabled, onChange }: { checked: boolean; disabled?: boolean; onChange: (v: boolean) => void }) {
  return (
    <Switch.Root
      checked={checked}
      disabled={disabled}
      onCheckedChange={onChange}
      className={cn(
        'relative h-5 w-9 rounded-full border transition-colors disabled:opacity-40',
        checked ? 'border-cyan-ln bg-cyan-bg' : 'border-line bg-surface-3',
      )}
    >
      <Switch.Thumb
        className={cn(
          'block size-3.5 translate-x-0.5 rounded-full transition-transform',
          checked ? 'translate-x-[18px] bg-cyan' : 'bg-fg-3',
        )}
      />
    </Switch.Root>
  )
}
