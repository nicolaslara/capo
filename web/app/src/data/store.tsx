import { createContext, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import type { ConsoleData, ChatMessage } from './types'
import { fixtureData } from './fixtures'
import {
  SSE_EVENT_NAME,
  eventToChatMessage,
  fetchThread,
  parseEventFrame,
  threadToChatMessages,
  type CapoEvent,
} from './live'

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
  // Mirror of the current data so the SSE handler (a stable closure) can resolve
  // an incoming event's session/agent against the live agent table. Synced in an
  // effect (never during render).
  const dataRef = useRef(data)
  useEffect(() => { dataRef.current = data }, [data])
  // Event ids already projected into chat, so a re-delivered tail frame (or a
  // thread item we hydrated) is never appended twice.
  const seenEventIds = useRef<Set<string>>(new Set())

  // Resolve the chat key (the agent table's `id`, which is the agent name) for a
  // committed event, matching first on its session id then its `agent-<name>` id.
  const chatKeyForEvent = (event: CapoEvent): string | null => {
    const agents = dataRef.current.agents
    if (event.session_id) {
      const bySession = agents.find((a) => a.sessionId === event.session_id)
      if (bySession) return bySession.id
    }
    if (event.agent_id) {
      const name = event.agent_id.replace(/^agent-/, '')
      const byName = agents.find((a) => a.id === name || a.name === name)
      if (byName) return byName.id
      return name
    }
    return null
  }

  // Append already-keyed chat messages, skipping any whose id was already seen.
  const appendChatDeduped = (agentId: string, msgs: ChatMessage[]) => {
    const fresh = msgs.filter((m) => !seenEventIds.current.has(m.id))
    if (fresh.length === 0) return
    for (const m of fresh) seenEventIds.current.add(m.id)
    setData((d) => ({ ...d, chats: { ...d.chats, [agentId]: [...(d.chats[agentId] ?? []), ...fresh] } }))
  }

  // Auto-detect the live facade: if GET /api/dashboard answers, switch to live
  // read-model data, keep it fresh with a light re-poll, and consume the
  // INCREMENTAL event tail (ST4/ST8) -- each `event: event` SSE frame is a
  // committed CapoEvent that we project into the targeted agent's chat. This is
  // not a full-dashboard re-poll: the streaming agent reply arrives here.
  useEffect(() => {
    let es: EventSource | null = null
    let poll: ReturnType<typeof setInterval> | null = null
    let cancelled = false

    // The dashboard read model owns agents / lanes / summary, NOT the chat
    // conversation (chats are built client-side from the event tail + thread).
    // `keepChats` preserves the accumulated `chats` across re-polls so a poll
    // never wipes the streamed reply; the initial live load drops the fixture
    // chats by starting fresh.
    const refreshDashboard = (keepChats: boolean) =>
      fetch('/api/dashboard', { cache: 'no-store' })
        .then((r) => (r.ok ? r.json() : Promise.reject(new Error('no facade'))))
        .then((j) => {
          if (cancelled) return
          setData((prev) => {
            const next = normalize(j)
            return keepChats ? { ...next, chats: prev.chats } : next
          })
        })

    refreshDashboard(false)
      .then(() => {
        if (cancelled) return
        liveRef.current = true
        // Re-poll the projected read model (agents/lanes/dispatch) on a slow
        // timer; the chat/conversation surface is driven by the event tail, not
        // this poll.
        poll = setInterval(() => { void refreshDashboard(true).catch(() => {}) }, 4000)

        // Resume the tail from "now" (the server's default): only events
        // committed after we subscribed stream in, so we are not flooded with
        // backlog. Per-session history is hydrated on demand via /api/thread.
        es = new EventSource('/api/events')
        es.addEventListener(SSE_EVENT_NAME, (ev: MessageEvent) => {
          const event = parseEventFrame(ev.data)
          if (!event) return
          const key = chatKeyForEvent(event)
          if (!key) return
          const msg = eventToChatMessage(event)
          if (msg) appendChatDeduped(key, [msg])
        })
      })
      .catch(() => { /* fixture mode */ })
    return () => {
      cancelled = true
      es?.close()
      if (poll) clearInterval(poll)
    }
  }, [])

  const logCmd = (text: string, tone?: 'default' | 'danger') =>
    setCommandLog((l) => [{ id: nextId('cmd'), time: now(), text, tone }, ...l].slice(0, 30))

  const postCommand = async (
    kind: string,
    agent: string,
    extra: Record<string, string>,
  ): Promise<{ ok: boolean; error?: string; sessionId?: string | null }> => {
    try {
      const res = await fetch('/api/commands', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ kind, agent, ...extra }),
      })
      if (res.ok) {
        // The reply carries the targeted session so we can read its thread and
        // tail its streaming reply (see capo-web /api/commands).
        const body = (await res.json().catch(() => ({}))) as { sessionId?: string | null }
        return { ok: true, sessionId: body.sessionId ?? null }
      }
      return { ok: false, error: humanizeErr(await res.text()) }
    } catch (e) {
      return { ok: false, error: `could not reach server: ${String(e)}` }
    }
  }

  // Hydrate an agent's chat history from the session's projected thread (ST5):
  // read it once, project to chat messages, and dedupe against the live tail so
  // history + streamed reply never double up.
  const hydrateThread = async (agentId: string, sessionId: string) => {
    const thread = await fetchThread(sessionId)
    if (!thread) return
    appendChatDeduped(agentId, threadToChatMessages(thread))
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
          appendChat(agentId, [sysMsg('Steering goal sent through the server boundary · the agent reply streams in live.')])
          // The streamed reply arrives on the event tail; also read the session
          // thread once so any already-committed turn items render immediately.
          if (r.sessionId) void hydrateThread(agentId, r.sessionId)
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

// eslint-disable-next-line react-refresh/only-export-components -- the store hook lives with its provider; the provider is the fast-refresh boundary.
export function useStore() {
  const s = useContext(StoreContext)
  if (!s) throw new Error('useStore must be used within StoreProvider')
  return s
}
