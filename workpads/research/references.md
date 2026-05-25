# Research References

Record primary sources, date observed, license notes, and local clone paths.

## Findings Files

- `workpads/research/findings/R1-acp.md` - ACP protocol, SDKs, clients, agents, Capo mapping.
- `workpads/research/findings/R2-prior-art.md` - prior-art comparison across agent orchestration and coding-agent harnesses.
- `workpads/research/findings/R2-code-architecture.md` - source-code architecture inspection of Codex, Cline, OpenHands, OpenCode, Aider, and LangGraph checkpointing docs.
- `workpads/research/findings/R3-subscriptions.md` - subscription-backed connector feasibility and security boundary.
- `workpads/research/findings/R4-R6-stack-runtime.md` - stack choice, runtime, tunnel, sandboxing.
- `workpads/research/findings/R5-memory.md` - memory systems and layered/fractional memory recommendation.
- `workpads/research/findings/R7-input-surfaces.md` - CLI/dashboard/mobile/voice sequence and command model.

## Agent Client Protocol

Observed 2026-05-25.

- ACP repository: https://github.com/agentclientprotocol/agent-client-protocol
- ACP docs: https://agentclientprotocol.com/
- Protocol overview: https://agentclientprotocol.com/protocol/overview
- Architecture: https://agentclientprotocol.com/get-started/architecture
- Initialization/capabilities: https://agentclientprotocol.com/protocol/initialization
- Authentication: https://agentclientprotocol.com/protocol/authentication
- Session setup/load/resume/close: https://agentclientprotocol.com/protocol/session-setup
- Prompt turn: https://agentclientprotocol.com/protocol/prompt-turn
- Tool calls and permission requests: https://agentclientprotocol.com/protocol/tool-calls
- Transports: https://agentclientprotocol.com/protocol/transports
- Extensibility: https://agentclientprotocol.com/protocol/extensibility
- Schema: https://agentclientprotocol.com/protocol/schema
- Clients: https://agentclientprotocol.com/get-started/clients
- Agents: https://agentclientprotocol.com/get-started/agents
- Python SDK docs: https://agentclientprotocol.github.io/python-sdk/
- TypeScript SDK: https://www.npmjs.com/package/@agentclientprotocol/sdk
- Rust crates: https://crates.io/crates/agent-client-protocol and https://crates.io/crates/agent-client-protocol-schema
- Python package: https://pypi.org/project/agent-client-protocol/
- Zed ACP: https://zed.dev/acp
- JetBrains ACP: https://www.jetbrains.com/acp/
- GitHub Copilot ACP server docs: https://docs.github.com/en/enterprise-cloud@latest/copilot/reference/copilot-cli-reference/acp-server

## Prior Art

Observed 2026-05-25.

- Swarms: https://github.com/kyegomez/swarms
- OpenHands: https://github.com/All-Hands-AI/OpenHands
- OpenHands runtime docs: https://docs.openhands.dev/openhands/usage/runtimes/overview
- OpenHands Cloud: https://github.com/All-Hands-AI/OpenHands-Cloud
- Cline: https://github.com/cline/cline
- OpenCode: https://github.com/anomalyco/opencode and https://www.opencode.ai/
- OpenAI Codex: https://github.com/openai/codex and https://developers.openai.com/codex/
- Aider: https://github.com/aider-ai/aider
- CrewAI: https://github.com/crewAIInc/crewAI and https://docs.crewai.com/en/introduction
- AutoGen: https://github.com/microsoft/autogen
- LangGraph persistence: https://docs.langchain.com/oss/javascript/langgraph/persistence

## Prior Art Code Architecture

Observed 2026-05-25. Local clones are under ignored `workpads/references/repos/`.

- OpenAI Codex local clone: `workpads/references/repos/openai-codex`, observed commit `9f42c89c0112771dc29100a6f3fc904049b2655f`
- Cline local clone: `workpads/references/repos/cline`, observed commit `8a6441fddd3b4d372d086886ebe4ee11e78dc993`
- OpenHands local clone: `workpads/references/repos/openhands`, observed commit `5e311f7f995008ffe4c74f8cf6f3085d4030c670`
- OpenCode local clone: `workpads/references/repos/opencode`, observed commit `73ee493265acf15fcd8caab2bc8cd3bd375b63cb`
- Aider local clone: `workpads/references/repos/aider`, observed commit `5dc9490bb35f9729ef2c95d00a19ccd30c26339c`
- LangGraph durable execution docs: https://docs.langchain.com/oss/python/langgraph/durable-execution
- LangGraph human-in-the-loop docs: https://docs.langchain.com/oss/python/langgraph/human-in-the-loop

## Subscription Connectors

Observed 2026-05-25.

- Claude Max plan: https://support.claude.com/en/articles/11049741-what-is-the-max-plan
- Claude Code with Pro/Max: https://support.claude.com/en/articles/11145838-use-claude-code-with-your-pro-or-max-plan
- Claude Code authentication: https://code.claude.com/docs/en/authentication
- Claude Code headless: https://code.claude.com/docs/en/headless
- Claude Agent SDK with plans: https://support.claude.com/en/articles/15036540-use-the-claude-agent-sdk-with-your-claude-plan
- Claude legal/compliance: https://code.claude.com/docs/en/legal-and-compliance
- Claude API authentication: https://platform.claude.com/docs/en/manage-claude/authentication
- Codex with ChatGPT plan: https://help.openai.com/en/articles/11369540-using-codex-with-your-chatgpt-plan
- Codex CLI: https://developers.openai.com/codex/cli
- Codex non-interactive: https://developers.openai.com/codex/noninteractive
- Codex access tokens: https://developers.openai.com/codex/enterprise/access-tokens
- Codex SDK: https://developers.openai.com/codex/sdk
- OpenAI terms: https://openai.com/policies/terms-of-use/
- Playwright browser contexts/auth: https://playwright.dev/docs/browser-contexts and https://playwright.dev/docs/auth

## Stack, Runtime, Tunnel, Sandboxing

Observed 2026-05-25.

- Tokio: https://tokio.rs/ and https://github.com/tokio-rs/tokio
- axum: https://github.com/tokio-rs/axum
- clap: https://github.com/clap-rs/clap
- serde: https://github.com/serde-rs/serde
- rusqlite: https://github.com/rusqlite/rusqlite
- sqlx: https://github.com/launchbadge/sqlx
- ratatui: https://github.com/ratatui/ratatui
- SQLite: https://www.sqlite.org/copyright.html
- uv: https://github.com/astral-sh/uv
- Pydantic: https://github.com/pydantic/pydantic
- FastAPI: https://github.com/FastAPI/FastAPI
- PyO3: https://github.com/PyO3/pyo3
- Ollama: https://github.com/ollama/ollama and https://docs.ollama.com/index
- llama.cpp: https://github.com/ggml-org/llama.cpp
- vLLM: https://github.com/vllm-project/vllm
- SGLang: https://github.com/sgl-project/sglang
- Whisper: https://github.com/openai/whisper
- faster-whisper: https://github.com/SYSTRAN/faster-whisper
- Dev Container spec: https://github.com/devcontainers/spec
- Docker Engine: https://docs.docker.com/engine/
- containerd: https://github.com/containerd/containerd
- Tailscale SSH: https://tailscale.com/kb/1193/tailscale-ssh
- Tailscale policy syntax: https://tailscale.com/kb/1337/policy-syntax
- Tailscale Funnel/Serve: https://tailscale.com/docs/reference/tailscale-cli/funnel and https://tailscale.com/docs/features/tailscale-funnel
- OpenSSH: https://www.openssh.org/
- cloudflared: https://github.com/cloudflare/cloudflared and https://developers.cloudflare.com/tunnel/setup/
- Landlock: https://kernel.org/doc/html/v6.0/security/landlock.html
- bubblewrap: https://github.com/containers/bubblewrap
- nsjail: https://github.com/google/nsjail
- gVisor: https://gvisor.dev/
- Firecracker: https://github.com/firecracker-microvm/firecracker

## Memory

Observed 2026-05-25.

- SQLite overview: https://www.sqlite.org/about.html
- SQLite FTS5: https://www.sqlite.org/fts5.html
- Tana Input API: https://outliner.tana.inc/learn/features/input-api
- Tana export: https://outliner.tana.inc/learn/features/copy-paste-and-export
- Tana terms/privacy/security: https://tana.inc/pages/terms-privacy-security
- Capacities API: https://docs.capacities.io/developer/api
- Capacities export: https://docs.capacities.io/reference/export
- Capacities import: https://docs.capacities.io/reference/import and https://docs.capacities.io/reference/bulk-import
- Capacities E2EE note: https://docs.capacities.io/more/end-to-end-encryption
- Graphiti: https://github.com/getzep/graphiti
- Zep sessions: https://help.getzep.com/v2/sessions
- Zep graph concepts: https://help.getzep.com/v2/understanding-the-graph
- Zep vs Graphiti: https://help.getzep.com/docs/faq/zep-vs-graphiti
- Zep paper: https://arxiv.org/abs/2501.13956
- mem0: https://github.com/mem0ai/mem0
- mem0 platform: https://docs.mem0.ai/platform/overview
- mem0 OSS configuration: https://docs.mem0.ai/open-source/configuration
- mem0 exports: https://docs.mem0.ai/cookbooks/essentials/exporting-memories
- Letta: https://github.com/letta-ai/letta
- Letta stateful agents: https://docs.letta.com/guides/core-concepts/stateful-agents
- Letta Code memory: https://docs.letta.com/letta-code/memory/
- Letta MemFS: https://docs.letta.com/letta-code/memfs/
- Letta local mode: https://docs.letta.com/letta-code/local-mode
- Letta context repositories: https://www.letta.com/blog/context-repositories
- Chroma OSS: https://docs.trychroma.com/docs/overview/oss
- pgvector: https://github.com/pgvector/pgvector

## Input Surfaces

Observed 2026-05-25.

- ACP slash commands: https://agentclientprotocol.com/protocol/slash-commands
- ACP session list/info: https://agentclientprotocol.com/protocol/session-list and https://agentclientprotocol.com/announcements/session-info-update-stabilized
- Tauri v2: https://v2.tauri.app/start/
- React Native: https://reactnative.dev/
- PWA installability: https://developer.mozilla.org/en-US/docs/Web/Progressive_web_apps/Guides/Making_PWAs_installable
- Web Speech API: https://developer.mozilla.org/en-US/docs/Web/API/SpeechRecognition
- OpenAI speech-to-text: https://platform.openai.com/docs/guides/speech-to-text
- whisper.cpp: https://github.com/ggml-org/whisper.cpp
- Vosk: https://github.com/alphacep/vosk-api
- Deepgram real-time transcription: https://developers.deepgram.com/docs/transcribe-meetings-in-realtime
- Picovoice Porcupine: https://picovoice.ai/docs/faq/porcupine/
- TanStack Query: https://tanstack.com/query/latest/docs/framework/react/overview
- shadcn/ui: https://github.com/shadcn-ui/ui

## Local Clones

Keep generated clones under gitignored paths:

```bash
git clone --depth 1 https://github.com/agentclientprotocol/agent-client-protocol.git workpads/references/repos/agent-client-protocol
git clone --depth 1 https://github.com/kyegomez/swarms.git workpads/references/repos/swarms
```
