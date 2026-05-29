import { createContext, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import type { ConsoleData, ChatMessage } from './types'
import { fixtureData } from './fixtures'

export interface CommandLogEntry {
  id: string
  time: string
  text: string
  tone?: 'default' | 'danger'
}

interface Store {
  data: ConsoleData
  commandLog: CommandLogEntry[]
  steer: (agentId: string, message: string) => void
  interrupt: (agentId: string, reason?: string) => void
  stop: (agentId: string, reason?: string) => void
  decidePermission: (id: string, decision: 'once' | 'always' | 'reject') => void
}

const StoreContext = createContext<Store | null>(null)

let seq = 1000
const nextId = (p: string) => `${p}-${++seq}`
function now() {
  const d = new Date()
  return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`
}

function humanizeErr(raw: string): string {
  if (raw.includes('AgentHasNoActiveSession')) return 'This agent has no active session to steer — start a session first.'
  if (raw.includes('UnknownAgent')) return 'Unknown agent.'
  if (raw.includes('AgentAlreadyHasSession')) return 'Agent already has an active session.'
  return raw.replace(/^handle:\s*/, '').trim().slice(0, 180) || 'command failed'
}

function normalize(j: Partial<ConsoleData>): ConsoleData {
  return {
    project: j.project ?? fixtureData.project,
    summary: j.summary ?? [],
    agents: j.agents ?? [],
    activity: j.activity ?? [],
    evidence: j.evidence ?? [],
    reviews: j.reviews ?? [],
    validations: j.validations ?? [],
    goals: j.goals ?? [],
    permissions: j.permissions ?? [],
    tools: j.tools ?? [],
    chats: j.chats ?? {},
  }
}

export function StoreProvider({ children }: { children: ReactNode }) {
  const [data, setData] = useState<ConsoleData>(() => structuredClone(fixtureData))
  const [commandLog, setCommandLog] = useState<CommandLogEntry[]>([])
  const liveRef = useRef(false)

  // Auto-detect the live facade: if GET /api/dashboard answers, switch to live
  // data + subscribe to the SSE stream. Otherwise stay on fixtures.
  useEffect(() => {
    let es: EventSource | null = null
    let cancelled = false
    fetch('/api/dashboard', { cache: 'no-store' })
      .then((r) => (r.ok ? r.json() : Promise.reject(new Error('no facade'))))
      .then((j) => {
        if (cancelled) return
        liveRef.current = true
        setData(normalize(j))
        es = new EventSource('/api/events')
        es.onmessage = (ev) => {
          try {
            const parsed = JSON.parse(ev.data)
            if (!parsed.error) setData(normalize(parsed))
          } catch { /* ignore malformed frame */ }
        }
      })
      .catch(() => { /* fixture mode */ })
    return () => { cancelled = true; es?.close() }
  }, [])

  const logCmd = (text: string, tone?: 'default' | 'danger') =>
    setCommandLog((l) => [{ id: nextId('cmd'), time: now(), text, tone }, ...l].slice(0, 30))

  const postCommand = async (
    kind: string,
    agent: string,
    extra: Record<string, string>,
  ): Promise<{ ok: boolean; error?: string }> => {
    try {
      const res = await fetch('/api/commands', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ kind, agent, ...extra }),
      })
      if (res.ok) return { ok: true }
      return { ok: false, error: humanizeErr(await res.text()) }
    } catch (e) {
      return { ok: false, error: `could not reach server: ${String(e)}` }
    }
  }

  const appendChat = (agentId: string, msgs: ChatMessage[]) =>
    setData((d) => ({ ...d, chats: { ...d.chats, [agentId]: [...(d.chats[agentId] ?? []), ...msgs] } }))

  const sysMsg = (text: string): ChatMessage => ({ id: nextId('m'), role: 'system', time: now(), text })

  const pushActivity = (agent: string, kind: string, text: string) =>
    setData((d) => ({
      ...d,
      activity: [
        { id: nextId('ev'), sequence: (d.activity[0]?.sequence ?? 0) + 1, time: now(), agent, kind, text },
        ...d.activity,
      ],
    }))

  const steer: Store['steer'] = (agentId, message) => {
    if (!message.trim()) return
    const t = now()
    appendChat(agentId, [{ id: nextId('m'), role: 'operator', time: t, text: message }]) // optimistic, both modes
    logCmd(`steer_agent → ${agentId}`)
    if (liveRef.current) {
      void postCommand('steer_agent', agentId, { message }).then((r) => {
        if (r.ok) {
          appendChat(agentId, [sysMsg('Steering goal sent through the server boundary · the agent reply updates live.')])
        } else {
          appendChat(agentId, [sysMsg(`⨯ ${r.error}`)])
          logCmd(`steer_agent failed: ${r.error}`, 'danger')
        }
      })
      return
    }
    appendChat(agentId, [{ id: nextId('m'), role: 'agent', agent: agentId, time: t, text: 'Acked steering goal through the server-command boundary. (fixture: live reply streams once connected to capo-web.)' }])
    pushActivity(agentId, 'steer', `Operator steered: "${message.slice(0, 64)}".`)
  }

  const interrupt: Store['interrupt'] = (agentId, reason) => {
    logCmd(`interrupt_agent → ${agentId}`, 'danger')
    if (liveRef.current) {
      void postCommand('interrupt_agent', agentId, { reason: reason ?? 'operator interrupt' }).then((r) => {
        appendChat(agentId, [sysMsg(r.ok ? 'Interrupt requested through the server boundary.' : `⨯ ${r.error}`)])
        if (!r.ok) logCmd(`interrupt failed: ${r.error}`, 'danger')
      })
      return
    }
    setData((d) => ({ ...d, agents: d.agents.map((a) => (a.id === agentId ? { ...a, status: 'paused' } : a)) }))
    pushActivity(agentId, 'interrupt', `Interrupt requested${reason ? `: ${reason}` : ''}.`)
  }

  const stop: Store['stop'] = (agentId, reason) => {
    logCmd(`stop_agent → ${agentId}`, 'danger')
    if (liveRef.current) {
      void postCommand('stop_agent', agentId, { reason: reason ?? 'operator stop' }).then((r) => {
        appendChat(agentId, [sysMsg(r.ok ? 'Stop requested through the server boundary.' : `⨯ ${r.error}`)])
        if (!r.ok) logCmd(`stop failed: ${r.error}`, 'danger')
      })
      return
    }
    setData((d) => ({ ...d, agents: d.agents.map((a) => (a.id === agentId ? { ...a, status: 'finished' } : a)) }))
    pushActivity(agentId, 'stop', `Stop requested${reason ? `: ${reason}` : ''}.`)
  }

  const decidePermission: Store['decidePermission'] = (id, decision) => {
    logCmd(`permission ${decision} → ${id}`, decision === 'reject' ? 'danger' : 'default')
    setData((d) => {
      const req = d.permissions.find((p) => p.id === id)
      return {
        ...d,
        permissions: d.permissions.filter((p) => p.id !== id),
        activity: req
          ? [
              { id: nextId('ev'), sequence: (d.activity[0]?.sequence ?? 0) + 1, time: now(), agent: req.agent, kind: 'permission', text: `Permission ${decision === 'reject' ? 'rejected' : 'granted (' + decision + ')'}: ${req.tool} (${req.scope}).` },
              ...d.activity,
            ]
          : d.activity,
      }
    })
  }

  const store = useMemo<Store>(
    () => ({ data, commandLog, steer, interrupt, stop, decidePermission }),
    [data, commandLog],
  )
  return <StoreContext.Provider value={store}>{children}</StoreContext.Provider>
}

export function useStore() {
  const s = useContext(StoreContext)
  if (!s) throw new Error('useStore must be used within StoreProvider')
  return s
}
