// Domain model for the Capo operator console. Shaped to mirror the real Capo
// read models (agents/sessions, dispatch pipeline, evidence/review/validation
// ledgers, event log, permission queue, tool catalog) so the same types serve
// fixtures today and the live HTTP facade later.

export type AgentStatus = 'running' | 'finished' | 'timed out' | 'blocked' | 'available' | 'paused'
export type Confidence = 'low' | 'medium' | 'high'
export type AdapterKind = 'codex_exec' | 'claude_code' | 'acp' | 'fake'

export type DispatchStageState = 'done' | 'active' | 'pending' | 'blocked' | 'failed' | 'none'

export interface DispatchState {
  plan: DispatchStageState
  preflight: DispatchStageState
  gate: DispatchStageState
  run: DispatchStageState
  gateStatus?: string
  nextAction?: string
  credentialScan?: 'clean' | 'pending' | 'flagged'
  providerCliExecuted?: boolean
  planId?: string
  gateId?: string
  executionId?: string
}

export interface Agent {
  id: string
  name: string
  status: AgentStatus
  adapter: AdapterKind
  goal: string
  result: string
  confidence: Confidence
  evidence: string[]
  reviews: number
  validations: number
  tools: number
  memory: number
  blocker?: string
  updatedAt: string
  dispatch?: DispatchState
  rawOutputPolicy?: string
  rawPromptPolicy?: string
  sessionId?: string
  runId?: string
}

export type Tone = 'default' | 'warn' | 'good' | 'info' | 'danger'

export interface Metric {
  key: string
  label: string
  value: number
  tone?: Tone
  hint?: string
}

export interface ActivityEvent {
  id: string
  sequence: number
  time: string
  agent: string
  kind: string
  text: string
}

export type EvidenceStatus = 'validated' | 'partial' | 'pending' | 'blocked'
export interface EvidenceItem {
  id: string
  kind: string
  status: EvidenceStatus
  agent: string
}

export type ReviewStatus = 'accepted' | 'needs follow-up' | 'pending'
export interface ReviewItem {
  id: string
  status: ReviewStatus
  target: string
}

export type ValidationStatus = 'passed' | 'pending' | 'failed'
export interface ValidationItem {
  id: string
  status: ValidationStatus
  target: string
}

export interface Requirement {
  text: string
  done: boolean
}

export interface Goal {
  id: string
  title: string
  status: 'active' | 'blocked' | 'completed'
  confidence?: Confidence
  requirements: Requirement[]
  blockers: string[]
  validation: string
  story: { time: string; text: string }[]
}

export type Risk = 'low' | 'medium' | 'high'

export interface PermissionRequest {
  id: string
  scope: string
  risk: Risk
  source: string
  agent: string
  tool: string
  detail: string
  status: 'pending' | 'decided'
}

export type ToolOrigin = 'capo' | 'adapter_native' | 'runtime' | 'provider_native' | 'mcp'
export type Instrumentation = 'full' | 'structured_observed' | 'text_observed' | 'none'
export interface Tool {
  id: string
  name: string
  origin: ToolOrigin
  risk: Risk
  exposure: string
  instrumentation: Instrumentation
  status: string
}

export type ChatRole = 'operator' | 'agent' | 'system' | 'tool' | 'permission' | 'evidence'
export interface ChatMessage {
  id: string
  role: ChatRole
  agent?: string
  time: string
  text?: string
  kind?: string
  tool?: { name: string; args?: string; result?: string }
  permission?: PermissionRequest
}

export interface Project {
  id: string
  name: string
  server: string
  mode: 'fixture' | 'live'
  addr: string
  updatedAt: string
}

export interface ConsoleData {
  project: Project
  summary: Metric[]
  agents: Agent[]
  activity: ActivityEvent[]
  evidence: EvidenceItem[]
  reviews: ReviewItem[]
  validations: ValidationItem[]
  goals: Goal[]
  permissions: PermissionRequest[]
  tools: Tool[]
  chats: Record<string, ChatMessage[]>
}
