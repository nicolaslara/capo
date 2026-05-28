# Dashboard Webclient References

## Objective

Record local sources and verification expectations for Capo's browser
dashboard/web client workpad.

## Local Sources

- `workpads/features/dashboard.md`
  - Completed shared dashboard/query feature. Key input: web, voice, mobile,
    CLI, and future TUI surfaces should share the same query/read-model
    contract rather than duplicate dashboard logic.
- `workpads/operator-control/knowledge.md`
  - Current CLI operator UX decisions. Key input: concise normal output,
    debug/details separation, attached-agent interaction, safe live-provider
    gates, and markdown-shaped result rendering.
- `workpads/goal-orchestration/knowledge.md`
  - Planned goal/story/evidence model. Key input: the rich web dashboard should
    eventually show goal lifecycle, agent reports, evidence, validation,
    reviews, blockers, confidence, and historical execution reports.
- `workpads/server/knowledge.md`
  - Server/control-plane evidence. Key input: webclient mutations should go
    through server commands; the UI should not own controller state.
- `workpads/architecture/boundaries.md`
  - Input surfaces submit commands and render read models; they do not own
    orchestration state.
- `workpads/architecture/state-model.md`
  - SQLite/event/artifact authority model and read-model rebuild rules.
- `workpads/architecture/tool-exposure.md`
  - Tool and evidence instrumentation model.
- `workpads/architecture/memory-architecture.md`
  - Provenance and memory packet rules for later dashboard context surfaces.

## Verification Expectations

- Design review evidence before implementation.
- Browser screenshots for desktop and mobile after each UI slice.
- Screenshot iteration notes before marking UI tasks complete.
- Local dev-server URL and commands recorded when implementation begins.
- Browser smoke that verifies nonblank, correctly framed, interactive views.
- No raw secrets, provider tokens, cookies, or sensitive transcripts in
  screenshots, browser storage, or committed fixtures.
