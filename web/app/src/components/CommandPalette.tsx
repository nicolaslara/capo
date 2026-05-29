import type { ReactNode } from 'react'
import { useNavigate } from 'react-router-dom'
import * as Dialog from '@radix-ui/react-dialog'
import { Command } from 'cmdk'
import { CornerDownLeft } from 'lucide-react'
import { NAV } from './nav'
import { useStore } from '@/data/store'
import { useTheme } from '@/lib/theme'
import { StatusDot } from './ui'

export function CommandPalette({ open, onOpenChange }: { open: boolean; onOpenChange: (o: boolean) => void }) {
  const navigate = useNavigate()
  const { data } = useStore()
  const { toggle } = useTheme()
  const run = (fn: () => void) => {
    fn()
    onOpenChange(false)
  }

  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-50 bg-black/40 backdrop-blur-[1px]" />
        <Dialog.Content
          aria-describedby={undefined}
          className="fixed left-1/2 top-[16vh] z-50 w-[min(560px,92vw)] -translate-x-1/2 overflow-hidden rounded-xl border border-line bg-surface-1 shadow-[var(--shadow)]"
        >
          <Dialog.Title className="sr-only">Command palette</Dialog.Title>
          <Command className="[&_[cmdk-group-heading]]:px-3 [&_[cmdk-group-heading]]:py-1.5 [&_[cmdk-group-heading]]:text-[10px] [&_[cmdk-group-heading]]:uppercase [&_[cmdk-group-heading]]:tracking-[0.08em] [&_[cmdk-group-heading]]:text-fg-3">
            <div className="flex items-center gap-2 border-b border-line px-3">
              <span className="mono text-cyan">$</span>
              <Command.Input
                autoFocus
                placeholder="Search screens, agents, actions…"
                className="h-11 w-full bg-transparent text-[13px] text-fg outline-none placeholder:text-fg-3"
              />
            </div>
            <Command.List className="max-h-[52vh] overflow-y-auto p-1.5">
              <Command.Empty className="px-3 py-6 text-center text-[12px] text-fg-3">No matches.</Command.Empty>

              <Command.Group heading="Navigate">
                {NAV.map((n) => (
                  <Item key={n.to} onSelect={() => run(() => navigate(n.to))}>
                    <n.icon size={15} className="text-fg-2" />
                    {n.label}
                    <span className="mono ml-auto text-[11px] text-fg-3">{n.to}</span>
                  </Item>
                ))}
              </Command.Group>

              <Command.Group heading="Agents">
                {data.agents.map((a) => (
                  <Item key={a.id} value={`agent ${a.name} ${a.status}`} onSelect={() => run(() => navigate(`/agents?sel=${a.id}`))}>
                    <StatusDot status={a.status} />
                    <span className="mono">{a.name}</span>
                    <span className="ml-auto text-[11px] text-fg-3">{a.status}</span>
                  </Item>
                ))}
              </Command.Group>

              <Command.Group heading="Actions">
                <Item value="toggle theme dark light" onSelect={() => run(toggle)}>
                  Toggle light / dark theme
                </Item>
                <Item value="open chat console" onSelect={() => run(() => navigate('/chat'))}>
                  Open chat console
                </Item>
                <Item value="review permissions tools" onSelect={() => run(() => navigate('/tools'))}>
                  Review pending permissions
                </Item>
              </Command.Group>
            </Command.List>
          </Command>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  )
}

function Item({
  children,
  onSelect,
  value,
}: {
  children: ReactNode
  onSelect: () => void
  value?: string
}) {
  return (
    <Command.Item
      value={value}
      onSelect={onSelect}
      className="flex cursor-pointer items-center gap-2.5 rounded-md px-3 py-2 text-[13px] text-fg-1 data-[selected=true]:bg-surface-active data-[selected=true]:text-fg"
    >
      {children}
      <CornerDownLeft size={12} className="ml-auto hidden text-fg-3 data-[selected=true]:block" />
    </Command.Item>
  )
}
