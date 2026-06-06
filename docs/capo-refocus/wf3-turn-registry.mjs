export const meta = {
  name: 'capo-wf3-turn-registry',
  description: 'A2 verify (detached fan-out) + B1/B2 in-flight ACP-turn registry with cooperative cancel (honest, additive, gated)',
  phases: [
    { title: 'Investigate' },
    { title: 'Plan' },
    { title: 'Implement' },
    { title: 'Review' },
  ],
}

// Hard constraints for EVERY agent in this workflow (the owner is away; this is
// the riskiest item). State them verbatim so nobody guesses.
const RULES = [
  'CONTEXT: project capo, a Rust workspace. The validated demo loop is: capo conductor (L1) ->',
  'start_agent MCP tool -> RunAcpLiveTurnLocal spawns a real claude-code-acp worker (L2) that',
  'writes files over the ACP wire. There is a live gated E2E `conductor_live_e2e` and a',
  'deterministic suite. NOTHING you do may regress them.',
  '',
  'KEY CODE FACTS (verify by reading, do not trust blindly):',
  '- crates/capo-server/src/acp_mcp_http.rs: tool_start_agent (~L433) drives RegisterAgent ->',
  '  StartSession -> RunAcpLiveTurnLocal. It already supports `detached: true` (~L441,504-545):',
  '  spawns the worker turn on a std::thread and returns status:running immediately. The conductor',
  '  system prompt (crates/capo-web/src/main.rs conductor_goal, ~L327) already instructs detached',
  '  fan-out + collect_results. So A2 (non-blocking conductor) is ALREADY BUILT.',
  '- crates/capo-server/src/server_core.rs run_acp_live_turn_local (~L220): spawn_live_session ->',
  '  take_transport -> controller.drive_acp_live_turn(transport, ...) -> session.finalize(). The',
  '  transport is CONSUMED by the blocking turn loop and the connection is torn down at finalize.',
  '- crates/capo-controller/src/acp_live_dispatch.rs drive_acp_live_turn (~L85) -> adapter',
  '  .drive_with_decider(transport, goal, decider) in crates/capo-adapters/src/acp_live.rs (~L299):',
  '  the loop owns the transport &mut self and recv_line_within in a loop until stop.',
  '- crates/capo-adapters/src/acp_wire.rs: trait AcpTransport { send_line; recv_line; recv_line_within }',
  '  (~L71). There IS a cancel(&mut self, session_id) (~L503) that sends session/cancel on the wire.',
  '- ServerCommand::SteerAgent/InterruptAgent/StopAgent route through controller_routing.rs /',
  '  types.rs (~L1585-1605) and today only RECORD state/events — they do NOT reach a live worker.',
  '',
  'NON-NEGOTIABLE RULES:',
  '1. ADDITIVE + DEFAULT-OFF: any new param (e.g. a cancel token) must default to a value that makes',
  '   the existing validated path BYTE-IDENTICAL (no cancel => identical frames/behavior). The',
  '   deterministic suite and conductor_live_e2e must stay green.',
  '2. NEVER FAKE DELIVERY. If a command cannot actually reach a live worker, it must keep its',
  '   current honest "record intent" behavior and you must DOCUMENT what is deferred and why.',
  '3. Mid-turn session/prompt INJECTION is NOT supported by ACP (one prompt per turn) — do not fake',
  '   it. Steer can at most enqueue a follow-up turn; if that is risky, leave steer as record-intent',
  '   and document. COOPERATIVE CANCEL is the realistic live win: a shared cancel flag the turn loop',
  '   checks between frames, then calls the transport cancel(session_id) and stops.',
  '4. If a sub-item cannot land cleanly without risking the validated loop, STOP at the last green',
  '   state and DOCUMENT it precisely. Partial-but-honest beats complete-but-fake.',
].join('\n')

phase('Investigate')
const investigation = await agent(
  RULES + '\n\n' +
  'INVESTIGATE ONLY (no edits). Read the files above end-to-end and answer, with file:line evidence, ' +
  'a precise FEASIBILITY verdict for each sub-item:\n' +
  '- A2: confirm detached fan-out is already built + wired; is anything missing or worth a small ' +
  'doc/test? (Likely just verify + document.)\n' +
  '- B2 cooperative cancel (interrupt/stop -> reach a LIVE worker): can a shared cancel token be ' +
  'threaded from a process-wide in-flight-turn registry, through RunAcpLiveTurnLocal -> ' +
  'drive_acp_live_turn -> drive_with_decider, checked between recv frames, to call ' +
  'transport.cancel(session_id) and stop — WITHOUT a fork and WITHOUT changing the no-cancel path? ' +
  'Identify every signature that must change and whether each change is additive.\n' +
  '- B1 steer (follow-up): is enqueuing a follow-up turn after the current one safe, or should steer ' +
  'stay record-intent? Recommend.\n' +
  'Also specify exactly where the in-flight-turn registry should live (a process-wide ' +
  'Arc<Mutex<HashMap<session_id, InFlightTurn>>> on CapoServer or a module static), how a turn ' +
  'registers on start and deregisters on finalize even on panic/error, and how the command handlers ' +
  'look up a live turn. Return plain markdown with a clear GO / PARTIAL / NO-GO per sub-item and the ' +
  'minimal additive change set for each GO/PARTIAL.',
  { label: 'investigate:registry', phase: 'Investigate' }
)

phase('Plan')
const plan = await agent(
  RULES + '\n\nThe feasibility investigation concluded:\n\n' + investigation + '\n\n' +
  'Produce the concrete, surgical implementation PLAN (no edits yet) for ONLY the GO/PARTIAL items. ' +
  'For each: exact files/functions/signatures to change (showing the additive default), the registry ' +
  'type + where it lives + register/deregister (RAII guard so it cleans up on panic/early-return), ' +
  'the command wiring (InterruptAgent/StopAgent -> set cancel flag; Steer per your recommendation), ' +
  'and the GATED live test to add (mirroring conductor_live_e2e: #[ignore] + an env gate). Explicitly ' +
  'list what stays deferred + the one-paragraph reason. Return plain markdown.',
  { label: 'plan:registry', phase: 'Plan' }
)

phase('Implement')
const impl = await agent(
  RULES + '\n\nFollow this agreed plan (adapt only where it is wrong against the real code):\n\n' + plan + '\n\n' +
  'IMPLEMENT it now. Edit the Rust files to add the in-flight-turn registry + cooperative-cancel ' +
  'plumbing (default-off) + command wiring for the GO/PARTIAL items, plus the gated live test and a ' +
  'small deterministic test for the registry register/deregister + cancel-flag behavior if feasible. ' +
  'Keep every change additive and the no-cancel path byte-identical. Run `cargo build` and the ' +
  'relevant deterministic tests yourself and report the REAL output. Do NOT run live (gated) tests. ' +
  'Return plain markdown: every change function-by-function, the build/test output you saw, which ' +
  'sub-items are DONE vs DEFERRED, and exactly what you deferred and why. Be honest about partials.',
  { label: 'implement:registry', phase: 'Implement' }
)

phase('Review')
const review = await agent(
  RULES + '\n\nA change was made to add an in-flight ACP-turn registry + cooperative cancel + command ' +
  'wiring. Implementer summary:\n\n' + impl + '\n\n' +
  'REVIEW critically against the CURRENT code. Verify with file:line: (1) the no-cancel path is ' +
  'byte-identical / additive (default token never cancels) — the validated loop cannot regress; ' +
  '(2) the registry deregisters even on panic/early-return (RAII guard, not a bare remove at the ' +
  'end); (3) InterruptAgent/StopAgent actually reach a live worker via the cancel token -> ' +
  'transport.cancel, and if a turn is NOT live they keep honest record-intent behavior — no faked ' +
  'delivery; (4) steer is either a real follow-up or honestly record-intent + documented; (5) the ' +
  'gated live test is correctly #[ignore]+env-gated and the deterministic registry test is sound; ' +
  '(6) `cargo build` is clean. Run `cargo build -p capo-server -p capo-adapters -p capo-controller` ' +
  'and the deterministic tests yourself to confirm. Report concrete problems with file:line and a ' +
  'PASS/FAIL verdict per sub-item + an overall PASS/FAIL. Return plain markdown.',
  { label: 'review:registry', phase: 'Review' }
)

log('WF3 complete — investigate, plan, implement, review done.')
return { investigation, plan, impl, review }
