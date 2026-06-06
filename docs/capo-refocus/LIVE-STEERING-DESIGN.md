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

## Progress
- **Increment 1 — DONE (`0a160b5`).** `AcpLiveAdapter::attach_persistent_session`
  + `PersistentAcpSession::prompt` (attach once, prompt repeatedly on one
  session). Deterministic test proves two prompts on one
  initialize+session/new. One-shot `drive_with_decider` untouched.
- **Increment 2 — DONE.** End-to-end wiring:
  - Controller `attach_persistent_acp_session` + `ingest_acp_prompt` (per-prompt
    ingest through the loop's normal route; initial prompt uses the IDENTICAL
    `turn-acp-live-{turn_id}` so the zero-window path is byte-identical).
  - `run_acp_live_turn_local` restructured into attach → initial prompt+ingest →
    `while let` steer loop (cancel + re-prompt the same session) → finalize.
    `steer_window_secs` (additive command field) gates it: **0 ⇒ one-shot,
    byte-identical** (every deterministic test + the validated live loop);
    positive ⇒ persistent + steerable.
  - Registry extended with a steer `Sender` (`register_in_flight_steerable`);
    `steer_session` (flip cancel + send `Steer`) and `stop_session` (send `Stop`).
  - `SteerAgent` delivers live to a registered persistent session IN ADDITION to
    the durable redirect record; `StopAgent` also signals the actor. No fake
    delivery when no persistent session is registered (honest record-intent).
  - capo-web sets `steer_window_secs` from `CAPO_WEB_STEER_WINDOW_SECS` (default
    30) so ALL ops workers are persistent + steerable; tests use 0.
  - Tests: adapter `persistent_session_drives_multiple_prompts_on_one_session`;
    server registry `steer_session_*` / `stop_session_*` / clone-sharing (9 total).
    capo-adapters 83, capo-controller 201, capo-server 167, capo-web 6 — green;
    clippy clean.
  - **VERIFIED LIVE** against a REAL `claude-code-acp` bridge over the
    subscription (gated test `steer_live_e2e.rs`): an initial prompt wrote
    `alpha.txt`, a `SteerAgent` follow-up continued the SAME session and wrote
    `bravo.txt`, `stop_reason=end_turn`. Run with `CAPO_E2E_LIVE_STEER=1`
    (+ the live ACP env gate). Both files present ⇒ the steered prompt ran on the
    one persistent session (adapter does `session/new` exactly once).

## DECISION (owner): ALL workers persist
The owner chose "all workers persist" — every worker is steerable in ops via a
positive `steer_window_secs` (default 30s). The one-shot path is preserved only
as the `0` case (tests + the validated live fan-out `conductor_live_e2e`, which
sets 0 to stay stable/byte-identical). Trade-off accepted: ops workers linger for
the steer window after their turn (extra latency + held-open processes); they
finalize on `stop_agent` (immediate) or the window timeout.

## OPEN DECISION for Increment 2 — worker lifecycle / steer window
Real steering needs the worker process to STAY ALIVE after a turn so a follow-up
prompt can continue. So after the initial prompt the actor must wait for a steer
before finalizing. Two questions decide how the validated `start_agent` path
changes:
1. **Which workers persist?** All workers (incl. fan-out), or only an interactive
   single-agent session? Fan-out workers are aggregated via `collect_results` and
   are never steered in practice — making them linger is pure latency + live
   processes held open.
2. **Steer window.** How long does an un-steered worker stay open before it
   finalizes (e.g. finalize immediately after the turn unless a steer is pending;
   or hold open N seconds; or hold until an explicit stop)?
   - Note: a deterministic stub worker EXITS after its turn, so the actor sees
     EOF and finalizes immediately regardless — the window only affects a real,
     still-alive `claude-code-acp` worker.

Recommended default (pending owner): **interactive single-agent sessions persist
and are steerable; fan-out (`detached`) workers stay one-shot** (finalize at turn
end, as today) so `collect_results` latency and process count are unchanged. This
keeps the validated fan-out loop byte-identical while making steering real where
it is actually used. Steering a fan-out worker would then first "promote" it to a
persistent session — deferred unless wanted.

## Risk / discipline
This restructures the most-validated code path's neighborhood, so: additive,
default-off (byte-identical one-shot path), gated live test, and if any part
can't land cleanly it's documented — never faked. This is the riskiest change
in the codebase; it gets its own focused implementation pass, not a tail-of-turn
edit.
