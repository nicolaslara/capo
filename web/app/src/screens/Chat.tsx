import { useState } from 'react'
import { Send, Pause, Square, Wrench, FileCheck2 } from 'lucide-react'
import { useStore } from '@/data/store'
import { Panel, PanelHeader, StatusDot, Chip, Button } from '@/components/ui'
import { PermissionCard } from '@/components/PermissionCard'
import { cn } from '@/lib/cn'
import type { ChatMessage } from '@/data/types'

export function Chat() {
  const { data, steer, interrupt, stop, decidePermission } = useStore()
  const [selId, setSelId] = useState(data.agents[0]?.id ?? '')
  const [msg, setMsg] = useState('')
  const [decided, setDecided] = useState<Record<string, 'once' | 'always' | 'reject'>>({})
  const agent = data.agents.find((a) => a.id === selId)
  const messages = data.chats[selId] ?? []

  const send = () => {
    if (!msg.trim()) return
    steer(selId, msg)
    setMsg('')
  }

  return (
    <div className="grid h-full min-h-0 grid-cols-1 gap-3 p-4 lg:grid-cols-[260px_minmax(0,1fr)]">
      {/* session list */}
      <Panel className="h-fit">
        <PanelHeader prompt="> sessions" />
        <div className="divide-y divide-line">
          {data.agents.map((a) => (
            <button
              key={a.id}
              onClick={() => setSelId(a.id)}
              className={cn(
                'relative flex w-full items-center gap-2.5 px-3.5 py-2.5 text-left transition-colors hover:bg-surface-hover',
                a.id === selId && 'bg-surface-active',
              )}
            >
              {a.id === selId && <span className="absolute left-0 top-1/2 h-6 w-0.5 -translate-y-1/2 rounded-full bg-violet" />}
              <StatusDot status={a.status} />
              <div className="min-w-0">
                <div className="mono truncate text-[13px] text-fg">{a.name}</div>
                <div className="truncate text-[11px] text-fg-2">{(data.chats[a.id]?.length ?? 0)} messages</div>
              </div>
            </button>
          ))}
        </div>
      </Panel>

      {/* conversation */}
      <Panel className="min-h-0">
        <PanelHeader
          prompt="> chat"
          title={agent?.name}
          right={agent && (
            <span className="flex items-center gap-1.5">
              <StatusDot status={agent.status} />
              <span className="mono text-[11px] text-fg-2">{agent.status}</span>
            </span>
          )}
        />
        <div className="flex min-h-[52vh] flex-1 flex-col gap-3 overflow-y-auto p-4">
          {messages.length === 0 ? (
            <div className="grid flex-1 place-items-center text-[12px] text-fg-3">No messages yet — steer the agent below.</div>
          ) : (
            messages.map((m) => (
              <Message key={m.id} m={m} decided={m.permission ? decided[m.permission.id] : undefined}
                onDecide={(d) => {
                  if (!m.permission) return
                  setDecided((s) => ({ ...s, [m.permission!.id]: d }))
                  decidePermission(m.permission.id, d)
                }}
              />
            ))
          )}
        </div>

        {/* composer */}
        <div className="border-t border-line p-3">
          <div className="flex items-start gap-2 rounded-md border border-line bg-surface-3 px-2.5 py-2 focus-within:ring-2 focus-within:ring-cyan/40">
            <span className="mono mt-0.5 text-cyan">{'>'}</span>
            <textarea
              value={msg}
              onChange={(e) => setMsg(e.target.value)}
              onKeyDown={(e) => { if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) send() }}
              rows={2}
              placeholder={`Steer ${agent?.name ?? 'agent'}…  (⌘↵ to send)`}
              className="mono max-h-32 min-h-[40px] w-full resize-y bg-transparent text-[12px] text-fg outline-none placeholder:text-fg-3"
            />
          </div>
          <div className="mt-2 flex items-center gap-2">
            <Button variant="primary" onClick={send} disabled={!msg.trim()}><Send size={13} /> Send</Button>
            <Button variant="danger" onClick={() => interrupt(selId, 'operator interrupt')}><Pause size={13} /> Interrupt</Button>
            <Button variant="danger" onClick={() => stop(selId, 'operator stop')}><Square size={13} /> Stop</Button>
            <span className="mono ml-auto text-[11px] text-fg-3">steer → poll summary · streaming when wired</span>
          </div>
        </div>
      </Panel>
    </div>
  )
}

function Message({
  m, decided, onDecide,
}: {
  m: ChatMessage
  decided?: 'once' | 'always' | 'reject'
  onDecide: (d: 'once' | 'always' | 'reject') => void
}) {
  if (m.role === 'permission' && m.permission) {
    return (
      <Row time={m.time} label="permission" tone="amber">
        <PermissionCard req={m.permission} decided={decided} onDecide={onDecide} />
      </Row>
    )
  }
  if (m.role === 'tool' && m.tool) {
    return (
      <Row time={m.time} label="tool" tone="cyan">
        <div className="rounded-lg border border-line bg-surface-2 p-3">
          <div className="flex items-center gap-2">
            <Wrench size={13} className="text-cyan" />
            <span className="mono text-[12px] font-semibold text-fg">{m.tool.name}</span>
            {m.tool.args && <span className="mono text-[11px] text-fg-3">{m.tool.args}</span>}
          </div>
          {m.tool.result && <p className="mono mt-1.5 text-[12px] text-fg-1">{m.tool.result}</p>}
        </div>
      </Row>
    )
  }
  if (m.role === 'evidence') {
    return (
      <Row time={m.time} label="evidence" tone="green">
        <div className="flex items-center gap-2 rounded-md border border-green-ln bg-green-bg px-3 py-2">
          <FileCheck2 size={13} className="text-green" />
          <span className="mono text-[12px] text-green">{m.text}</span>
        </div>
      </Row>
    )
  }
  if (m.role === 'system') {
    return <div className="mono text-center text-[11px] text-fg-3">— {m.text} —</div>
  }
  // operator / agent
  const isOp = m.role === 'operator'
  return (
    <Row time={m.time} label={isOp ? 'operator' : (m.agent ?? 'agent')} tone={isOp ? 'cyan' : 'violet'}>
      <div className={cn('rounded-lg border px-3 py-2 text-[13px]', isOp ? 'border-cyan-ln bg-cyan-bg text-fg' : 'border-line bg-surface-2 text-fg-1')}>
        {m.text}
      </div>
    </Row>
  )
}

function Row({ time, label, tone, children }: { time: string; label: string; tone: 'cyan' | 'violet' | 'amber' | 'green'; children: React.ReactNode }) {
  return (
    <div className="flex gap-3">
      <div className="flex w-20 shrink-0 flex-col items-end pt-1">
        <Chip tone={tone} mono>{label}</Chip>
        <span className="mono mt-1 text-[10px] text-fg-3">{time}</span>
      </div>
      <div className="min-w-0 flex-1">{children}</div>
    </div>
  )
}
