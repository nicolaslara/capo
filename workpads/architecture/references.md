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

## Runtime And Connectivity

To research:

- Tailscale ACLs and auth keys
- SSH tunnel patterns
- Local process supervisors
- tmux/session management

## State And Memory

To research:

- SQLite event sourcing patterns
- Zep/Graphiti
- mem0
- Letta
