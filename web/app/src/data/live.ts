// Live capo-web client: read a session's projected multi-turn thread (ST5) and
// project the incremental event tail (ST4) into chat messages.
//
// The agent reply is NOT a fixture placeholder: it STREAMS back over the event
// tail. The store reads the thread once for history, then extends it as each
// committed `CapoEvent` arrives on `/api/events`, using the same event-kind
// classification the server's thread projection uses (summary -> agent output,
// tool.* -> tool, terminal kinds -> a turn-closing system note) so the live and
// historical renders agree.

import type { CapoEvent, EventNotification } from './capo-wire'
import { SSE_EVENT_NAME } from './capo-wire'
import type { ChatMessage } from './types'

export { SSE_EVENT_NAME }
export type { CapoEvent, EventNotification }

/** The JSON `/api/thread` returns: the projected multi-turn thread (ST5). */
export interface WireThreadItem {
  sequence: number
  eventId: string
  /** `output` | `tool` | `terminal` (the thread item role). */
  kind: string
  /** The projected event kind, e.g. `session.summary_updated`. */
  eventKind: string
  itemRef: string | null
  text: string | null
  redactionState: string
}

export interface WireThreadTurn {
  turnId: string
  status: string
  firstSequence: number
  lastSequence: number
  items: WireThreadItem[]
}

export interface WireThread {
  sessionId: string
  fromSequence: number
  nextSequence: number
  turns: WireThreadTurn[]
}

/** The `/api/commands` reply: the targeted session whose reply will stream. */
export interface CommandResult {
  ok: boolean
  sessionId?: string | null
  error?: string
}

/** Classify a projected event/thread kind into a chat role + label. */
function classifyEventKind(eventKind: string): { role: ChatMessage['role']; label: string } | null {
  if (eventKind === 'session.summary_updated') return { role: 'agent', label: eventKind }
  if (eventKind.startsWith('tool.')) return { role: 'tool', label: eventKind }
  if (
    eventKind === 'evidence.recorded' ||
    eventKind === 'session.interrupted' ||
    eventKind === 'session.stopped' ||
    eventKind === 'run.exited'
  ) {
    return { role: 'system', label: eventKind }
  }
  // Lifecycle/bookkeeping kinds are not conversation content (matches the
  // server thread projection skipping them).
  return null
}

/** A short, human label for a terminal event kind shown as a system note. */
function terminalNote(eventKind: string, text: string | null): string {
  const tail = eventKind.split('.').pop() ?? eventKind
  const verb = tail.replace(/_/g, ' ')
  return text && text.trim() ? `${verb}: ${text}` : `turn ${verb}`
}

/** Project one thread item (history) into a chat message, or null to skip. */
function itemToChatMessage(
  sessionId: string,
  turnId: string,
  item: WireThreadItem,
): ChatMessage | null {
  const cls = classifyEventKind(item.kind === 'output' ? 'session.summary_updated' : item.eventKind)
  // The thread item already carries `kind` as output/tool/terminal; trust it
  // first, falling back to event-kind classification.
  const role: ChatMessage['role'] =
    item.kind === 'output' ? 'agent' : item.kind === 'tool' ? 'tool' : item.kind === 'terminal' ? 'system' : (cls?.role ?? 'agent')
  const id = `live-${sessionId}-${item.eventId}`
  const text = item.text ?? item.itemRef ?? item.eventKind
  if (role === 'tool') {
    return { id, role: 'tool', time: '', tool: { name: item.eventKind, result: item.text ?? undefined } }
  }
  if (role === 'system') {
    return { id, role: 'system', time: '', text: terminalNote(item.eventKind, item.text) }
  }
  return { id, role: 'agent', time: '', text }
}

/** Project a full thread (oldest first) into ordered chat messages. */
export function threadToChatMessages(thread: WireThread): ChatMessage[] {
  const out: ChatMessage[] = []
  for (const turn of thread.turns) {
    for (const item of turn.items) {
      const msg = itemToChatMessage(thread.sessionId, turn.turnId, item)
      if (msg) out.push(msg)
    }
  }
  return out
}

/** Extract the human-facing text a payload carries (mirrors `item_text`). */
function payloadText(payloadJson: string): string | null {
  try {
    const obj = JSON.parse(payloadJson) as Record<string, unknown>
    for (const key of ['adapter_summary', 'latest_summary', 'detail', 'latest_blocker', 'message', 'reason', 'summary']) {
      const v = obj[key]
      if (typeof v === 'string' && v) return v
    }
    const field = (key: string) => {
      const v = obj[key]
      return typeof v === 'string' && v && v !== 'none' ? v : null
    }
    const tool = field('tool_name')
    const normalized = field('normalized_kind')
    const status = field('status')
    if (tool && status) return `${tool} (${status})`
    if (tool) return tool
    if (normalized && status) return `${normalized} (${status})`
    if (normalized) return normalized
    return null
  } catch {
    return null
  }
}

/**
 * Project one live committed event (from the event tail) into a chat message,
 * or null when the event is not conversation content. `event_id` keys the
 * message so a re-delivered event de-dupes against an already-rendered one.
 */
export function eventToChatMessage(event: CapoEvent): ChatMessage | null {
  const cls = classifyEventKind(event.kind)
  if (!cls) return null
  const id = `live-${event.session_id ?? 'session'}-${event.event_id}`
  const text = payloadText(event.payload_json)
  if (cls.role === 'tool') {
    return { id, role: 'tool', time: '', tool: { name: event.kind, result: text ?? undefined } }
  }
  if (cls.role === 'system') {
    return { id, role: 'system', time: '', text: terminalNote(event.kind, text) }
  }
  return { id, role: 'agent', agent: agentName(event), time: '', text: text ?? event.kind }
}

/** The display agent name for an event (strip the `agent-` id prefix). */
function agentName(event: CapoEvent): string | undefined {
  if (!event.agent_id) return undefined
  return event.agent_id.replace(/^agent-/, '')
}

/**
 * Parse one SSE `event:`/`data:` block's data line into an `EventNotification`,
 * returning the `CapoEvent` it carries (or null for any other frame). The data
 * line is the verbatim JSON-RPC notification per the wire contract.
 */
export function parseEventFrame(data: string): CapoEvent | null {
  try {
    const parsed = JSON.parse(data) as Partial<EventNotification>
    if (parsed.method === 'event' && parsed.params?.event) return parsed.params.event
    return null
  } catch {
    return null
  }
}
