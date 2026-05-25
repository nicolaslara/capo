# Architecture References

Record architecture-specific references here after research is ingested.

## Research Gate Inputs

- Research gate summary: `../research/knowledge.md`
- ACP mapping: `../research/findings/R1-acp.md`
- Prior art comparison: `../research/findings/R2-prior-art.md`
- Prior art source-code architecture: `../research/findings/R2-code-architecture.md`
- Subscription connector policy: `../research/findings/R3-subscriptions.md`
- Stack/runtime/tunnel findings: `../research/findings/R4-R6-stack-runtime.md`
- Memory findings: `../research/findings/R5-memory.md`
- Input and voice findings: `../research/findings/R7-input-surfaces.md`

## Source-Code Architecture References

Observed 2026-05-25. Local clones are gitignored under `workpads/references/repos/`.

- OpenAI Codex: `workpads/references/repos/openai-codex`, observed commit `9f42c89c0112771dc29100a6f3fc904049b2655f`
- Cline: `workpads/references/repos/cline`, observed commit `8a6441fddd3b4d372d086886ebe4ee11e78dc993`
- OpenHands: `workpads/references/repos/openhands`, observed commit `5e311f7f995008ffe4c74f8cf6f3085d4030c670`
- OpenCode: `workpads/references/repos/opencode`, observed commit `73ee493265acf15fcd8caab2bc8cd3bd375b63cb`
- Aider: `workpads/references/repos/aider`, observed commit `5dc9490bb35f9729ef2c95d00a19ccd30c26339c`

## Protocol

- Agent Client Protocol repo: https://github.com/agentclientprotocol/agent-client-protocol
- Agent Client Protocol docs: https://agentclientprotocol.com/
- Local ACP clone for A2a: `workpads/references/repos/agent-client-protocol`, observed 2026-05-25 at commit `ec66afe2f0f9fce4e3348b38f8007b5583e4b20f`, Apache-2.0.
- ACP replay/dedupe architecture artifact: `acp-replay-dedupe.md` (created 2026-05-25)
- ACP session setup/load/resume docs: https://agentclientprotocol.com/protocol/session-setup
- ACP prompt turn docs: https://agentclientprotocol.com/protocol/prompt-turn
- ACP tool call docs: https://agentclientprotocol.com/protocol/tool-calls
- ACP agent plan docs: https://agentclientprotocol.com/protocol/agent-plan
- ACP session resume announcement: https://agentclientprotocol.com/announcements/session-resume-stabilized
- ACP Message ID RFD: https://agentclientprotocol.com/rfds/message-id
- Capability/permission architecture artifact: `capability-permissions.md` (created 2026-05-25)
- ACP permission requests and option mapping, observed 2026-05-25:
  - `workpads/references/repos/agent-client-protocol/docs/protocol/tool-calls.mdx`
  - `workpads/references/repos/agent-client-protocol/schema/schema.json`
- Protocol/provider architecture artifact: `protocol-provider.md` (created 2026-05-25)
- Official Codex/Claude docs refreshed for A5 on 2026-05-25:
  - Codex non-interactive mode: https://developers.openai.com/codex/noninteractive
  - Codex CLI: https://developers.openai.com/codex/cli
  - Claude Code headless/programmatic usage: https://code.claude.com/docs/en/headless
  - Claude Code authentication: https://code.claude.com/docs/en/authentication
- Local CLI observations for A5, observed 2026-05-25:
  - `codex --version`: `codex-cli 0.133.0`
  - `codex exec --help`: supports `--json`, `--sandbox`, `--ephemeral`, `--ignore-user-config`, `--ignore-rules`, `--output-schema`, and `--cd`
  - `claude --version`: `2.1.150 (Claude Code)`
  - `claude -p --help`: supports `--output-format text|json|stream-json`, `--input-format text|stream-json`, `--permission-mode`, `--allowedTools`, `--disallowedTools`, `--tools`, `--mcp-config`, `--session-id`, `--resume`, `--continue`, `--no-session-persistence`, `--bare`, `--max-budget-usd`, and partial-message flags

## Tool Exposure

- Tool exposure architecture artifact: `tool-exposure.md` (created 2026-05-25)
- ACP tool call docs, observed 2026-05-25: `workpads/references/repos/agent-client-protocol/docs/protocol/tool-calls.mdx`
- ACP filesystem client capability docs, observed 2026-05-25: `workpads/references/repos/agent-client-protocol/docs/protocol/file-system.mdx`
- ACP terminal client capability docs, observed 2026-05-25: `workpads/references/repos/agent-client-protocol/docs/protocol/terminals.mdx`
- ACP client capability schema, observed 2026-05-25: `workpads/references/repos/agent-client-protocol/schema/schema.json`
- ACP MCP server setup docs, observed 2026-05-25: `workpads/references/repos/agent-client-protocol/docs/protocol/session-setup.mdx`

## Runtime And Connectivity

- Runtime/tunnel architecture artifact: `runtime-tunnel.md` (created 2026-05-25)
- Runtime and tunnel research finding: `../research/findings/R4-R6-stack-runtime.md`
- Prior-art runtime source-code architecture finding: `../research/findings/R2-code-architecture.md`
- Local prior-art source references observed 2026-05-25:
  - OpenAI Codex process/runtime handling: `workpads/references/repos/openai-codex/codex-rs/core/src/exec.rs`
  - OpenHands process/Docker/remote sandbox service notes: `../research/findings/R2-code-architecture.md`
  - OpenCode shell/tool prior-art notes: `../research/findings/R2-code-architecture.md`
- Connectivity references already checked in R4/R6:
  - Tailscale SSH docs: https://tailscale.com/kb/1193/tailscale-ssh
  - Tailscale policy syntax: https://tailscale.com/kb/1337/policy-syntax
  - Tailscale Funnel/Serve docs: https://tailscale.com/docs/reference/tailscale-cli/funnel and https://tailscale.com/docs/features/tailscale-funnel
  - OpenSSH: https://www.openssh.org/
  - cloudflared tunnel setup: https://developers.cloudflare.com/tunnel/setup/

## State And Memory

Local architecture artifacts:

- State model: `state-model.md` (created 2026-05-25)
- Memory architecture: `memory-architecture.md` (created 2026-05-25)
- Prototype plan: `prototype-plan.md` (created 2026-05-25)
- Memory research finding: `../research/findings/R5-memory.md`
- Memory primary sources observed during research on 2026-05-25:
  - SQLite overview: https://www.sqlite.org/about.html
  - SQLite FTS5: https://www.sqlite.org/fts5.html
  - Graphiti: https://github.com/getzep/graphiti
  - mem0: https://github.com/mem0ai/mem0
  - Letta: https://github.com/letta-ai/letta
  - Chroma OSS: https://docs.trychroma.com/docs/overview/oss
  - pgvector: https://github.com/pgvector/pgvector
