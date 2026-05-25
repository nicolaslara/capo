# R7 - Input Surfaces: CLI, Dashboard, Mobile, Voice

Observed: 2026-05-25.

## Recommendation

Start with a local CLI as the first implementation surface, backed by a controller API and event stream that every later surface uses. Voice, however, should be designed as a first-class conversational interface to Capo, not just dictation.

The first dogfoodable UI sequence should be:

1. `capo` CLI for create/list/show/send/cancel/approve commands.
2. A local read/write web dashboard for inspection, approvals, interrupts, and steering.
3. Mobile as a responsive authenticated web/PWA view of the same dashboard.
4. Voice as conversational interaction with Capo: ask what agents have done, ask for status and blockers, discuss next steps, and steer one or more agents through Capo.

Do not make voice or native mobile the first implementation surface. They add privacy, platform, background execution, and distribution questions before Capo has proven its controller state model. Still, architecture should treat voice as more than a speech-to-text shortcut.

## Source Facts

- Capo's local spec requires spawn/register, send task, inspect current goal/status/recent events/latest summary, interrupt/stop, persist/recover, and record workpad evidence before dogfooding (`workpads/prototype/spec.md`).
- ACP treats clients as user-facing interfaces and agents as subprocess-like programs; a normal turn is `session/prompt`, streamed `session/update` events, optional permission/file/terminal requests, and `session/cancel` for interruption. Source: https://agentclientprotocol.com/protocol/overview
- ACP slash commands are advertised through `available_commands_update` and invoked as ordinary prompt text such as `/web agent client protocol`; the command list can change during a session. Source: https://agentclientprotocol.com/protocol/slash-commands
- ACP prompt capabilities include text as baseline and optional audio/image/resource blocks, but Capo should not depend on agent-native audio support for v0; transcript-to-command is more portable. Source: https://agentclientprotocol.com/protocol/initialization
- ACP session metadata now supports real-time `session_info_update` for generated titles and metadata, useful for dashboard/session lists. Source: https://agentclientprotocol.com/protocol/session-list and https://agentclientprotocol.com/announcements/session-info-update-stabilized
- Tauri v2 targets desktop and mobile with a web frontend plus Rust backend, but it still introduces app packaging and mobile-plugin maturity questions. Source: https://v2.tauri.app/start/
- React Native recommends Expo for new apps when native mobile is required. Source: https://reactnative.dev/
- PWAs can be installable and can launch standalone where browsers support the manifest/install model. Source: https://developer.mozilla.org/en-US/docs/Web/Progressive_web_apps/Guides/Making_PWAs_installable
- Web Speech API recognition has limited availability, and Chrome may send audio to a server-side recognition engine, so it is not a privacy-safe or portable default. Source: https://developer.mozilla.org/en-US/docs/Web/API/SpeechRecognition
- OpenAI Audio API supports transcription endpoints and current `gpt-4o-*transcribe*` models, with file and streaming options. Source: https://platform.openai.com/docs/guides/speech-to-text
- `whisper.cpp` is MIT-licensed, cross-platform, and supports CPU/GPU/on-device ASR paths. Source: https://github.com/ggml-org/whisper.cpp
- Vosk is Apache-2.0, offline, supports 20+ languages, streaming, mobile, Raspberry Pi, server, and multiple language bindings including Rust. Source: https://github.com/alphacep/vosk-api
- Deepgram documents real-time transcription via WebSocket/TLS and explicitly warns not to expose API keys in browsers. Source: https://developers.deepgram.com/docs/transcribe-meetings-in-realtime
- Picovoice Porcupine is a candidate wake-word engine across desktop/mobile/web, but it requires an access key and has commercial/service dependency implications. Source: https://picovoice.ai/docs/faq/porcupine/

## Canonical Command Model

All input surfaces should submit a controller-owned command envelope, not surface-specific RPCs:

```text
CommandEnvelope {
  id,
  created_at,
  origin: cli | dashboard | mobile | voice | api,
  actor_id,
  project_id,
  target: controller | session_id | task_id | agent_id,
  intent: send_prompt | slash_command | query_status | summarize_work | steer_agent | approve | deny | cancel | pause | resume | spawn | stop | set_capability_profile | update_task | record_evidence,
  text,
  structured_args,
  attachments,
  risk: low | confirmation_required | privileged,
  transcript_meta?,
  idempotency_key
}
```

Controller behavior:

- The controller validates authorization, target state, and capability/risk policy before touching an agent.
- Text commands and voice commands both lower into this envelope or into a conversational turn that produces one or more envelopes.
- Agent-facing text should use ACP-compatible `session/prompt` where possible; command shortcuts can remain slash-command strings for ACP adapters.
- Controller commands such as `cancel`, `approve`, `spawn`, and `set_capability_profile` should not be tunneled as plain agent chat. They should be first-class controller intents and then mapped to protocol/runtime actions.
- Every accepted command emits durable events, so CLI, dashboard, and mobile converge on the same read models.

## Voice Conversation Model

Voice should be a conversational surface for Capo. The user is not trying to talk directly to every subagent; the user is talking to Capo, which maintains controller state and can inspect, summarize, and steer the other agents.

Voice responsibilities:

- Answer questions from Capo read models: what agents are running, what they did, what is blocked, what changed, what needs review, and what evidence exists.
- Maintain short conversational context over the current project, active workpad, selected agents, and recent commands.
- Translate steering decisions into explicit controller commands.
- Ask for clarification when a command target or scope is ambiguous.
- Require visible confirmation for privileged, destructive, broad, or security-sensitive actions.
- Record accepted voice-derived decisions as durable Capo events.

Recommended early flow:

1. User holds push-to-talk.
2. Surface records a short utterance.
3. ASR returns transcript plus confidence/alternatives when available.
4. Capo interprets the utterance against controller state and returns a spoken/text response or a proposed command.
5. If the action is privileged, destructive, broad, or ambiguous, Capo displays the parsed command and asks for confirmation.
6. Accepted transcript becomes a conversational turn plus one or more `CommandEnvelope(origin=voice, text=...)` records.
7. Capo emits durable events for the command, response, and any agent steering action.

Examples:

| Spoken command | Canonical intent | Notes |
| --- | --- | --- |
| "What did the agents finish while I was away?" | `summarize_work` | Capo summarizes completed work from event log, evidence, and session summaries. |
| "What's blocked right now?" | `query_status` | Capo lists blocked/stale sessions and pending permissions. |
| "What is agent two doing?" | `query_status` | Capo answers from session state, recent events, and latest summary. |
| "Ask agent one to summarize its current blocker" | `send_prompt` | Sends transcript text to the target session. |
| "Tell the research agent to focus on source code, not docs" | `steer_agent` | Capo records a steering command and sends a scoped message to the target agent. |
| "Cancel agent two" | `cancel` | Controller maps to ACP `session/cancel` or runtime interrupt. Confirmation optional because it stops work but does not grant power. |
| "Approve the test command" | `approve` | Must bind to a visible pending permission request, not infer from stale context. |
| "Give agent one network access" | `set_capability_profile` | Requires confirmation and scoped expiry. |
| "Run slash test on the frontend session" | `slash_command` | Lower to `/test ...` text for ACP-compatible agents when appropriate. |

Avoid wake-word-first control initially. Push-to-talk gives a clean privacy boundary and avoids background microphone policy work. Wake word can be an opt-in later feature after the audit model exists.

## Dashboard State Required For Dogfooding

The dashboard must render controller read models, not scrape terminal output. Minimum dogfood state:

- Projects/workpads: active workpad, task queue, task status, acceptance criteria, evidence artifact links.
- Agents: identity, runtime, provider/adapter, adapter/subagent-reported state/config, health, current capability profile.
- Sessions: session ID, cwd, title, active goal, status, started/updated timestamps, current turn state.
- Live activity: recent events, latest summary, plan, tool calls, command history, pending operations.
- Permission queue: requested action, scope, target, requester, risk label, approve/deny/cancel controls, expiry.
- Interrupt controls: cancel current turn, pause/resume, stop runtime, mark blocked.
- Recovery state: last persisted event sequence, restart status, stale/offline indicators.
- Review/evidence: tests run, manual smoke evidence, review notes, confidence, unresolved blockers.
- Worktree context: branch, dirty state, last commit, files touched if available.
- Metrics: elapsed time, retry count, provider/model when known, token/cost estimates when available.
- Audit trail: who/what issued each command, original text/transcript, normalized intent, outcome.

For v0, the dashboard can be local-only and single-user. It still needs authentication or loopback-only binding before mobile/remote use.

## Surface Sequence

### 1. CLI

Use CLI first because it is deterministic, scriptable, testable, and fits Capo's file/git workflow.

Candidate libraries:

- `clap` for Rust subcommands and arguments; Apache-2.0/MIT. Source: https://github.com/clap-rs/clap
- `rustyline` or `reedline` for optional interactive shell mode.
- `ratatui` only if a TUI becomes necessary; MIT and suitable for terminal dashboards, but it is more UI work than the first CLI needs. Source: https://github.com/ratatui/ratatui

Initial commands:

- `capo project list`
- `capo agent list`
- `capo agent spawn --profile local-coding`
- `capo session send <session> --text "..."`
- `capo session watch <session>`
- `capo session cancel <session>`
- `capo permission list`
- `capo permission approve|deny <request>`

### 2. Web Dashboard

Use a local web dashboard for dogfooding once the CLI proves the command/event model.

Candidate libraries:

- Rust server: `axum`, `tokio`, `tower`, `serde`, `sqlx` or `rusqlite`.
- Event delivery: Server-Sent Events for one-way updates; WebSocket only when the UI needs bidirectional low-latency interaction.
- Frontend: Vite + React or Svelte; TanStack Query is a good fit for server state sync. Source: https://tanstack.com/query/latest/docs/framework/react/overview
- Components: shadcn/ui/Radix if using React and Tailwind; MIT. Source: https://github.com/shadcn-ui/ui

Do not put orchestration policy in frontend state. The dashboard submits commands and renders controller state.

### 3. Mobile

Recommended v0 mobile strategy: responsive authenticated web/PWA dashboard.

Rationale:

- It reuses the dogfood dashboard.
- It avoids native app signing, stores, background execution, mobile microphone permissions, and push notification complexity during architecture/prototype.
- It is enough for remote monitoring, cancel/approve, and short steering messages.

Native mobile should be deferred until Capo needs OS-native notifications, secure enclave/biometric auth, share sheets, background audio, or deep links.

Candidate native paths:

- Tauri v2 if reusing Rust/web frontend is more important than mature mobile plugin ergonomics.
- Expo/React Native if mobile-native UI, notifications, microphone, and platform SDKs become first-class.

### 4. Voice

Recommended v0 voice strategy: push-to-talk conversational control in the dashboard or CLI-side helper, not always-listening voice control.

The first version can use transcript text internally, but it should expose a Capo conversation loop:

- listen
- transcribe
- interpret against controller read models
- answer or propose action
- confirm if needed
- execute through command envelopes
- summarize result

Candidate ASR options:

| Option | Fit | Privacy | License/service notes |
| --- | --- | --- | --- |
| `whisper.cpp` / `whisper-rs` | Best local-first spike for desktop voice commands. | Audio can stay local. | MIT upstream; validate model license/distribution separately. |
| Vosk | Offline, low-latency, broad bindings, smaller models. | Audio can stay local. | Apache-2.0; accuracy may lag modern neural cloud models. |
| OpenAI transcription API | High quality and easy to prototype. | Audio leaves machine; must classify as cloud transcript processing. | Paid API; do not log raw audio or API keys. |
| Deepgram | Strong real-time WebSocket option. | Audio leaves machine unless self-hosted option is separately purchased/evaluated. | Keep keys server-side only. |
| Web Speech API | Browser-only convenience spike. | Not portable; Chrome may use server recognition. | Do not rely on for privacy-sensitive default. |
| Apple Speech framework | Good iOS/macOS-native candidate later. | Platform-specific; on-device/server behavior depends on OS/API availability. | Requires native app path. |
| Picovoice Porcupine | Later wake-word candidate. | On-device detection possible, but access-key/commercial dependency. | Defer until wake-word requirement is explicit. |

## Privacy And Safety Notes

- Treat audio, transcripts, and voice-derived commands as sensitive input. They may contain secrets, credentials, or unrelated background speech.
- Default to no raw-audio persistence. If debugging requires clips, require explicit per-session opt-in and retention limits.
- Store command transcript text only when needed for the audit trail; redact or mark as sensitive when commands mention secrets.
- Store conversational summaries as Capo events when useful, but keep raw audio transient by default.
- Separate ASR provider credentials from agent/provider credentials.
- Never expose cloud ASR API keys to browser/mobile clients; proxy through the local/server controller.
- Voice commands that grant capabilities, approve tool calls, run shell commands, change filesystem/network scope, or stop multiple agents require visible confirmation.
- Keep raw voice transcripts out of memory distillation by default. Only user-approved summaries or decisions should enter long-term memory.
- For mobile/remote control, require authenticated sessions, audit every command, and make revocation obvious.

## Candidate Library/Service Shortlist

| Area | First choice | Alternatives | Notes |
| --- | --- | --- | --- |
| CLI | `clap` | `bpaf`, `argh` | `clap` has mature derive/subcommand ergonomics and permissive licensing. |
| TUI | Defer | `ratatui` | Useful if CLI watch mode needs richer panes before web dashboard exists. |
| Controller API | `axum` + SSE | WebSocket, gRPC | SSE is enough for dashboard state streams; commands can be HTTP POST. |
| Web state | TanStack Query | SWR, Svelte stores | Good fit for shared server state and invalidation. |
| Web UI | Vite + React/Svelte | Tauri webview later | Keep frontend portable to dashboard/PWA/Tauri. |
| Mobile v0 | Responsive PWA | Tauri mobile, Expo/RN | PWA first; native later only for OS-native capabilities. |
| Local ASR | `whisper.cpp` | Vosk | Prefer local desktop privacy; compare latency and accuracy with conversational command phrases. |
| Cloud ASR | OpenAI transcription | Deepgram, AssemblyAI | Use only behind explicit privacy mode and server-side key handling. |
| Wake word | Defer | Porcupine | Push-to-talk is safer for v0. |

## Open Questions

- Should Capo define its canonical command envelope before or during the architecture workpad's state/event model?
- Is mobile remote control required before first dogfood, or is same-machine/local-network dashboard enough?
- Should Capo keep voice transcript audit text by default, store only normalized commands plus a "derived from voice" marker, or store short user-approved conversation summaries?
- Which ASR quality bar matters more for v0: fully local privacy or high-accuracy cloud transcription?
- Should dashboard approvals require a second factor or local OS confirmation for privileged capability grants?
- How should Capo expose ACP `available_commands_update` when multiple agents/sessions each advertise different commands?
- What is the smallest conversational state Capo needs to answer "what happened?", "what is blocked?", and "what should I do next?" accurately by voice?

## Confidence

High confidence:

- CLI-first is the smallest useful and testable surface.
- Voice should be a conversational interface to Capo that lowers decisions into the same command model as text.
- Dogfooding requires a dashboard/read model with session, task, permission, evidence, and recovery state.
- Mobile should start as responsive web/PWA, not native.

Medium confidence:

- `whisper.cpp` should be the first local ASR spike; Vosk may be better for low-latency small command grammars.
- SSE should be enough for the first dashboard stream; WebSocket may be needed once live terminal/agent interaction becomes bidirectional.
- Tauri is a good later desktop packaging path, but mobile maturity should be reassessed before committing.
