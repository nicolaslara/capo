# Live steering design — persistent session actor (cancel + re-prompt)

Owner chose "A + live steering" (2026-06-06). The ACP spec
(https://agentclientprotocol.com/protocol/v1/prompt-turn) confirms:
- `session/cancel` (notification) aborts a turn → agent returns `cancelled`. (B2, done.)
- No mid-turn injection; the only way to steer is **cancel → send another
  `session/prompt` on the SAME session to continue the conversation.**

## Why this is a lifecycle change, not a flag
- `AcpLiveAdapter::drive_with_decider` (acp_live.rs:312) runs the whole flow on a
  transport it consumes: `initialize → session_new → set_mode → prompt(once)`,
  then `LiveAcpSession::finalize` tears down the process group.
- A fresh spawn calls `session_new` again → a NEW ACP session with no memory.
  Continuation requires re-`prompt` on the **same live `AcpWireClient`**
  (same `session_id`, same process).
- The permission decider is `!Send`, so the live client cannot be parked in a
  thread-shared registry; it must stay pinned to one thread.

## Design: a persistent session actor (additive, default-off)
1. **Adapter:** add a persistent path that performs `initialize + session_new +
   set_mode` ONCE and returns a handle exposing `prompt(msg) -> transcript`
   (callable repeatedly on the same session) and the installed cancel flag.
   Shares the lower-level `AcpWireClient` steps with the one-shot path WITHOUT
   changing `drive_with_decider` (the validated path stays byte-identical).
2. **Worker thread = the actor.** The dedicated `std::thread` `start_agent`
   already spawns for a detached worker owns the `!Send` client and loops:
   drive the initial prompt → block on an `mpsc::Receiver<String>` →
   on a steer message: flip the cancel flag (→ `session/cancel` → `cancelled`),
   wait for the in-flight prompt to settle, then `prompt(steer_msg)` on the same
   session → repeat until a stop message or idle timeout, then `finalize`.
3. **Registry.** Extend the WF3 in-flight registry entry to optionally carry a
   `Sender<String>` (the steer channel) alongside the existing cancel flag —
   both are `Send`, so the registry stays thread-safe. `None` for one-shot turns.
4. **Command wiring.** `SteerAgent` looks up the agent's session; if a persistent
   actor is registered, send the steer text on the channel (real delivery);
   otherwise keep today's honest record-intent. `InterruptAgent`/`StopAgent`
   already flip the cancel flag (B2) and additionally send a stop to the actor.
5. **Opt-in.** A `persistent: true` flag on `start_agent` (and/or a server flag)
   selects the actor path. Default keeps the one-shot validated loop unchanged.

## Tests
- Deterministic: registry carries a steer Sender; SteerAgent delivers to it; a
  scripted transport drives prompt → cancel → re-prompt on one session.
- Gated live (`#[ignore]` + env): a real persistent worker is steered mid-task
  and the second prompt continues the same session.

## Risk / discipline
This restructures the most-validated code path's neighborhood, so: additive,
default-off (byte-identical one-shot path), gated live test, and if any part
can't land cleanly it's documented — never faked. This is the riskiest change
in the codebase; it gets its own focused implementation pass, not a tail-of-turn
edit.
