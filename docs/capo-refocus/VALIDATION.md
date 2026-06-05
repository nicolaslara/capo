# Capo Refocus — Validation Harness

> How we prove the loop works "as expected." Two tiers + a manual demo. The gate asserts
> **observed** results (filesystem + event log), never anything the agent merely *reports*.

## Why two tiers

The real proof is a live `claude-code-acp` agent making a real change over the subscription —
but that needs network + auth and is non-deterministic, so it can't be the CI default. So:

| Tier | Transport | Proves | Where it runs |
| --- | --- | --- | --- |
| **Replay (CI gate)** | `ScriptedAcpTransport` replaying a recorded session | the **wiring**: server → controller → `drive_acp_live_turn` → adapter → fs/events, deterministically | every CI run |
| **Live (real proof)** | `PipedProcessTransport` spawning `npx @zed-industries/claude-code-acp` | the **real bridge + subscription auth + real edit** | locally / nightly, gated by `CAPO_E2E_LIVE_ACP=1` |

**Honesty rule (anti-false-green):** the replay tier passing does NOT prove the bridge works —
only that capo's plumbing is correct. The **live tier must pass at least once for any change
that touches the ACP launch/auth/transport path.** CI shows both tiers' last-green status; a
green replay with a stale live run is flagged, not hidden.

## Headless E2E acceptance test (Slice A)

```
GIVEN  a fresh scratch git repo (temp dir, `git init`, one commit)
  AND  an `acp`-bound agent registered against it
WHEN   a dispatch turn runs with a fixed task:
       "Create a file HELLO.txt whose contents are exactly: capo-works"
       (replay tier: ScriptedAcpTransport replays the recorded edit;
        live  tier: real claude-code-acp over subscription, HOME set, no API key)
THEN   (observed, from the filesystem)   HELLO.txt exists and contains "capo-works"
  AND  (observed, from the event log)     events include agent.registered, a turn,
                                          tool/edit calls, and — if worktree requested —
                                          worktree.created
  AND  the change is attributed to the agent run (run/session ids line up)
```

A second worktree variant: same test with `worktree=true` → assert the edit lands in the
worktree path (not the main checkout) and `worktree.created` fired.

Implementation: a Rust integration test under `tests/` or `crates/capo-server/tests/`, building
on the existing `dp11` smoke scaffolding (reuse its turn-driving, swap stub→real/recorded and
test→server-command path). The replay fixture is the recording captured by the live tier.

## Conductor E2E (Slice B)

Extend the headless test to drive through the conductor instead of a raw command:

```
GIVEN  capo-web up with a conductor claude-code-acp session + capo MCP tools
WHEN   a chat message "in this repo, create HELLO.txt containing capo-works" is posted
       to /api/chat
THEN   the conductor calls the start_agent capo tool (assert from event log / tool audit)
  AND  the Slice-A assertions hold (file on disk + events)
  AND  the conductor's reply references the completed agent
```

Mode coverage: one test with `mode=one` (steer reaches exactly one agent) and one with
`mode=all` (a steer fans out to every active agent), asserting from the event log which agents
received the turn.

## Manual web-UI demo (dogfood)

```
1. capo-web serve                       # axum server + web/app chat
2. open the chat, set mode = one-agent
3. type: "in this repo, add a hello() function to src/lib.rs and run the tests"
4. watch: conductor calls start_agent → a claude-code-acp agent runs in the repo →
   src/lib.rs gains hello() → conductor reports back with the result
5. git diff confirms the real change
```

A passing demo = the loop is dogfoodable; a passing **live** headless tier = it's gated.

## Definition of done for "the design is validated"

- [ ] Replay tier green in CI (wiring proven).
- [ ] Live tier green locally at least once (real bridge + subscription proven).
- [ ] Worktree variant green.
- [ ] Conductor E2E green (chat → tool → agent → observed change).
- [ ] One-agent and all-agents modes both proven from the event log.
- [ ] Manual web demo reproducible from these steps.
