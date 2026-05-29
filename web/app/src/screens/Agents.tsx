import { useState } from 'react'
import { useSearchParams } from 'react-router-dom'
import * as Collapsible from '@radix-ui/react-collapsible'
import { ChevronRight, Send, Pause, Square } from 'lucide-react'
import { useStore } from '@/data/store'
import {
  Panel, PanelHeader, PanelBody, StatusBadge, StatusDot, ConfidenceMeter, Chip, Button, Eyebrow,
} from '@/components/ui'
import { DispatchPipeline } from '@/components/DispatchPipeline'
import { cn } from '@/lib/cn'
import type { Agent } from '@/data/types'

export function Agents() {
  const { data, steer, interrupt, stop, commandLog } = useStore()
  const [params, setParams] = useSearchParams()
  const selId = params.get('sel') ?? data.agents[0]?.id
  const agent = data.agents.find((a) => a.id === selId) ?? data.agents[0]

  return (
    <div className="grid grid-cols-1 gap-3 p-4 lg:grid-cols-[300px_minmax(0,1fr)]">
      {/* master list */}
      <Panel className="h-fit">
        <PanelHeader prompt="$ agents" right={<span className="mono text-[11px] text-fg-3">{data.agents.length}</span>} />
        <div className="divide-y divide-line">
          {data.agents.map((a) => {
            const active = a.id === agent?.id
            return (
              <button
                key={a.id}
                onClick={() => setParams({ sel: a.id })}
                className={cn(
                  'relative grid w-full grid-cols-[auto_minmax(0,1fr)] items-center gap-2.5 px-3.5 py-2.5 text-left transition-colors hover:bg-surface-hover',
                  active && 'bg-surface-active',
                )}
              >
                {active && <span className="absolute left-0 top-1/2 h-6 w-0.5 -translate-y-1/2 rounded-full bg-violet" />}
                <StatusDot status={a.status} />
                <div className="min-w-0">
                  <div className="mono truncate text-[13px] text-fg">{a.name}</div>
                  <div className="mono truncate text-[11px] text-fg-2">{a.adapter} · {a.status}</div>
                </div>
              </button>
            )
          })}
        </div>
      </Panel>

      {/* detail */}
      {agent && (
        <div className="flex min-w-0 flex-col gap-3">
          {/* header */}
          <div className="flex flex-wrap items-center gap-3 rounded-lg border border-line bg-surface-1 px-4 py-3 shadow-[var(--shadow-sm)]">
            <span className="mono text-[16px] font-semibold tracking-tight">{agent.name}</span>
            <StatusBadge status={agent.status} />
            <Chip mono tone="muted">{agent.adapter}</Chip>
            <div className="ml-auto flex items-center gap-3">
              <ConfidenceMeter value={agent.confidence} />
              <span className="mono text-[11px] text-fg-3">updated {agent.updatedAt}</span>
            </div>
          </div>

          <div className="grid grid-cols-1 gap-3 xl:grid-cols-[minmax(0,1fr)_360px]">
            <div className="flex min-w-0 flex-col gap-3">
              {/* session detail */}
              <Panel>
                <PanelHeader prompt="$ session --detail" />
                <PanelBody className="flex flex-col gap-3">
                  <div>
                    <Eyebrow>Goal</Eyebrow>
                    <p className="mt-1 text-[13px] text-fg-1">{agent.goal}</p>
                  </div>
                  <div className="rounded-md border border-line bg-surface-2 p-3">
                    <Eyebrow>Latest result</Eyebrow>
                    <p className="mt-1 text-[13px] text-fg">{agent.result}</p>
                  </div>
                  {agent.blocker && (
                    <div className="mono rounded-md border border-red-ln bg-red-bg px-3 py-2 text-[12px] text-red">
                      [!] {agent.blocker}
                    </div>
                  )}
                  <div className="grid grid-cols-2 gap-2 sm:grid-cols-4">
                    <Stat label="evidence" value={agent.evidence.length} />
                    <Stat label="reviews" value={agent.reviews} />
                    <Stat label="validations" value={agent.validations} />
                    <Stat label="tools / mem" value={`${agent.tools} / ${agent.memory}`} />
                  </div>
                </PanelBody>
              </Panel>

              {/* dispatch pipeline */}
              {agent.dispatch && (
                <Panel>
                  <PanelHeader prompt="$ dispatch" title="plan → preflight → gate → run" />
                  <PanelBody><DispatchPipeline dispatch={agent.dispatch} /></PanelBody>
                </Panel>
              )}

              {/* ledger for this agent */}
              <Panel>
                <PanelHeader prompt="$ ledger" title={`${agent.name}`} />
                <PanelBody className="flex flex-col gap-3">
                  <LedgerLane label="Evidence" rows={data.evidence.filter((e) => e.agent === agent.id).map((e) => ({ id: e.id, meta: e.kind, status: e.status }))} />
                  <LedgerLane label="Reviews" rows={data.reviews.filter((r) => r.target === agent.id).map((r) => ({ id: r.id, meta: 'review', status: r.status }))} />
                  <LedgerLane label="Validation" rows={data.validations.filter((v) => v.target === agent.id).map((v) => ({ id: v.id, meta: 'validation', status: v.status }))} />
                </PanelBody>
              </Panel>

              <RawDetails agent={agent} />
            </div>

            {/* control panel */}
            <ControlPanel
              agentId={agent.id}
              onSteer={(m) => steer(agent.id, m)}
              onInterrupt={() => interrupt(agent.id, 'operator interrupt')}
              onStop={() => stop(agent.id, 'operator stop')}
              log={commandLog}
            />
          </div>
        </div>
      )}
    </div>
  )
}

function Stat({ label, value }: { label: string; value: number | string }) {
  return (
    <div className="rounded-md border border-line bg-surface-2 px-3 py-2">
      <div className="mono text-[18px] font-semibold leading-none">{value}</div>
      <div className="mt-1 text-[10px] uppercase tracking-[0.06em] text-fg-2">{label}</div>
    </div>
  )
}

function LedgerLane({ label, rows }: { label: string; rows: { id: string; meta: string; status: string }[] }) {
  return (
    <div>
      <Eyebrow>{label}</Eyebrow>
      {rows.length === 0 ? (
        <div className="mono mt-1 text-[12px] text-fg-3">none</div>
      ) : (
        <div className="mt-1 divide-y divide-line rounded-md border border-line">
          {rows.map((r) => (
            <div key={r.id} className="flex items-center gap-2 px-3 py-1.5">
              <span className="mono min-w-0 flex-1 truncate text-[12px] text-fg-1">{r.id}</span>
              <span className="mono text-[11px] text-fg-3">{r.meta}</span>
              <Chip tone={statusTone(r.status)}>{r.status}</Chip>
            </div>
          ))}
        </div>
      )}
    </div>
  )
}

function statusTone(s: string): 'green' | 'amber' | 'red' | 'muted' {
  if (['validated', 'accepted', 'passed'].includes(s)) return 'green'
  if (['partial', 'pending', 'needs follow-up'].includes(s)) return 'amber'
  if (['blocked', 'failed'].includes(s)) return 'red'
  return 'muted'
}

function RawDetails({ agent }: { agent: Agent }) {
  const rows: [string, string][] = [
    ['session_id', agent.sessionId ?? '—'],
    ['run_id', agent.runId ?? '—'],
    ['plan_id', agent.dispatch?.planId ?? '—'],
    ['gate_id', agent.dispatch?.gateId ?? '—'],
    ['execution_id', agent.dispatch?.executionId ?? '—'],
    ['raw_output_policy', agent.rawOutputPolicy ?? '—'],
    ['raw_prompt_policy', agent.rawPromptPolicy ?? '—'],
  ]
  return (
    <Collapsible.Root>
      <Panel>
        <Collapsible.Trigger className="group flex h-10 w-full items-center gap-2 px-3.5 text-left">
          <ChevronRight size={14} className="text-fg-3 transition-transform group-data-[state=open]:rotate-90" />
          <span className="mono text-[12px] text-fg-2">$ details --raw</span>
          <span className="mono ml-auto text-[11px] text-fg-3">ids · policies</span>
        </Collapsible.Trigger>
        <Collapsible.Content>
          <div className="mono border-t border-line p-3.5 text-[12px]">
            <div className="grid grid-cols-1 gap-1.5 sm:grid-cols-2">
              {rows.map(([k, v]) => (
                <div key={k} className="flex items-center gap-2">
                  <span className="w-36 shrink-0 text-fg-3">{k}</span>
                  <span className="truncate text-fg-1">{v}</span>
                </div>
              ))}
            </div>
          </div>
        </Collapsible.Content>
      </Panel>
    </Collapsible.Root>
  )
}

function ControlPanel({
  agentId, onSteer, onInterrupt, onStop, log,
}: {
  agentId: string
  onSteer: (m: string) => void
  onInterrupt: () => void
  onStop: () => void
  log: { id: string; time: string; text: string; tone?: 'default' | 'danger' }[]
}) {
  const [msg, setMsg] = useState('')
  const [confirm, setConfirm] = useState<'interrupt' | 'stop' | null>(null)

  return (
    <Panel className="h-fit xl:sticky xl:top-4">
      <PanelHeader prompt="> control" title={agentId} />
      <PanelBody className="flex flex-col gap-3">
        <div>
          <Eyebrow>Steer</Eyebrow>
          <div className="mt-1.5 flex items-start gap-2 rounded-md border border-line bg-surface-3 px-2.5 py-2 focus-within:ring-2 focus-within:ring-cyan/40">
            <span className="mono mt-0.5 text-cyan">{'>'}</span>
            <textarea
              value={msg}
              onChange={(e) => setMsg(e.target.value)}
              rows={3}
              placeholder="Ask the agent to continue, summarize, or validate…"
              className="mono min-h-[60px] w-full resize-y bg-transparent text-[12px] text-fg outline-none placeholder:text-fg-3"
            />
          </div>
          <div className="mt-2 flex items-center gap-2">
            <Button variant="primary" onClick={() => { onSteer(msg); setMsg('') }} disabled={!msg.trim()}>
              <Send size={13} /> Send
            </Button>
          </div>
        </div>

        <div className="border-t border-line pt-3">
          <Eyebrow>Lifecycle</Eyebrow>
          <p className="mt-1 text-[11px] text-fg-3">Interrupt and Stop are destructive and require confirmation.</p>
          <div className="mt-2 flex flex-wrap items-center gap-2">
            {confirm === null ? (
              <>
                <Button variant="danger" onClick={() => setConfirm('interrupt')}><Pause size={13} /> Interrupt</Button>
                <Button variant="danger" onClick={() => setConfirm('stop')}><Square size={13} /> Stop</Button>
              </>
            ) : (
              <>
                <span className="mono text-[12px] text-red">confirm {confirm}?</span>
                <Button
                  variant="danger"
                  onClick={() => { confirm === 'interrupt' ? onInterrupt() : onStop(); setConfirm(null) }}
                >
                  Yes, {confirm}
                </Button>
                <Button variant="ghost" onClick={() => setConfirm(null)}>Cancel</Button>
              </>
            )}
          </div>
        </div>

        <div className="border-t border-line pt-3">
          <Eyebrow>Command log</Eyebrow>
          <div className="mono mt-1.5 max-h-40 overflow-y-auto rounded-md border border-line bg-surface-2 p-2 text-[11px]">
            {log.length === 0 ? (
              <div className="text-fg-3">no commands sent yet</div>
            ) : (
              log.map((c) => (
                <div key={c.id} className="flex items-center gap-2 py-0.5">
                  <span className="text-fg-3">{c.time}</span>
                  <span className={c.tone === 'danger' ? 'text-red' : 'text-fg-1'}>{c.text}</span>
                </div>
              ))
            )}
          </div>
        </div>
      </PanelBody>
    </Panel>
  )
}
