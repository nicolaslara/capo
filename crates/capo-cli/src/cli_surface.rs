use std::path::PathBuf;

const DEFAULT_STATE_ROOT: &str = ".capo-dev";

pub(crate) const HELP: &str = "\
Capo - local controller for coding-agent sessions

Usage:
  capo --help
  capo version
  capo init [--state PATH]
  capo dashboard [--project PROJECT_ID] [--session SESSION_ID] [--status STATUS] [--source-path PATH] [--source-status STATUS] [--state PATH]
  capo agent register --name NAME --adapter fake --runtime fake [--state PATH]
  capo agent spawn --name NAME --adapter fake --runtime fake [--state PATH]
  capo agent list [--state PATH]
  capo adapter readiness [--record] [--state PATH]
  capo adapter plan-launch --adapter codex|claude --agent NAME --goal GOAL [--workspace PATH] [--artifacts PATH] [--record] [--state PATH]
  capo adapter plan-proof --adapter codex|claude --agent NAME [--workspace PATH] [--artifacts PATH] [--record] [--state PATH]
  capo adapter dispatch-gate --dispatch-plan DISPATCH_PLAN_ID [--record] [--state PATH]
  capo adapter dispatch-status --dispatch-plan DISPATCH_PLAN_ID [--state PATH]
  capo adapter dispatch-status --latest [--agent NAME] [--state PATH]
  capo adapter dispatch-evidence --dispatch-plan DISPATCH_PLAN_ID --out DIR [--state PATH]
  capo adapter dispatch-evidence --latest [--agent NAME] --out DIR [--state PATH]
  capo adapter execution-request --dispatch-plan DISPATCH_PLAN_ID [--record] [--state PATH]
  capo adapter materialize-prompt --dispatch-plan DISPATCH_PLAN_ID [--record] [--state PATH]
  capo adapter run-preflight --dispatch-plan DISPATCH_PLAN_ID [--state PATH]
  capo adapter run-local --dispatch-plan DISPATCH_PLAN_ID [--record] [--out DIR] [--timeout-seconds N] [--state PATH]
  capo adapter dogfood-gate [--state PATH]
  capo adapter dogfood-gate evidence --out DIR [--state PATH]
  capo adapter smoke-report scan --artifact-root PATH [--state PATH]
  capo adapter smoke-report record --adapter codex|claude --status skipped|passed|failed --credential-scan clean|blocked|not_run --reason TEXT [--marker-found] [--artifact-root PATH] [--state PATH]
  capo adapter smoke-report status --smoke-report SMOKE_REPORT_ID [--state PATH]
  capo adapter smoke-report status --latest [--adapter codex|claude] [--state PATH]
  capo adapter smoke-report evidence --smoke-report SMOKE_REPORT_ID --out DIR [--state PATH]
  capo adapter smoke-report evidence --latest [--adapter codex|claude] --out DIR [--state PATH]
  capo adapter replay-fixture --adapter codex|claude|acp --fixture PATH --agent NAME --goal GOAL [--out DIR] [--state PATH]
  capo adapter replay-dispatch --dispatch-plan DISPATCH_PLAN_ID --fixture PATH [--out DIR] [--state PATH]
  capo dogfood readiness [--out DIR] [--state PATH]
  capo task send --agent NAME --goal GOAL [--scenario NAME] [--state PATH]
  capo session status --agent NAME [--state PATH]
  capo session redirect --agent NAME --goal GOAL [--state PATH]
  capo session interrupt --agent NAME --reason REASON [--state PATH]
  capo session stop --agent NAME --reason REASON [--state PATH]
  capo voice submit --transcript TEXT [--voice-session SESSION_ID] [--actor ACTOR] [--confirm] [--redacted-summary TEXT --reviewed-summary] [--state PATH]
  capo recover [--state PATH]
  capo permission request --approval APPROVAL_ID --scope-json JSON --reason REASON [--profile PROFILE] [--session SESSION_ID] [--tool-call TOOL_CALL_ID] [--subject-json JSON] [--requested-by ACTOR] [--state PATH]
  capo permission list [--state PATH]
  capo permission decide --approval APPROVAL_ID --decision allow_once|allow_always|reject_once|reject_always [--state PATH]
  capo runtime target register --target TARGET_ID --name NAME --runner local-process|remote-process|container --workspace PATH --artifacts PATH [--cwd PATH] [--capability-profile PROFILE] [--endpoint ENDPOINT_ID] [--status available|disabled|unhealthy] [--state PATH]
  capo runtime target set-status --target TARGET_ID --status available|disabled|unhealthy [--state PATH]
  capo runtime target status --target TARGET_ID [--state PATH]
  capo runtime target status --latest [--runner local-process|remote-process|container] [--status available|disabled|unhealthy] [--state PATH]
  capo runtime target readiness --target TARGET_ID [--state PATH]
  capo runtime target readiness --latest [--runner local-process|remote-process|container] [--status available|disabled|unhealthy] [--state PATH]
  capo runtime target readiness-evidence --target TARGET_ID --out DIR [--state PATH]
  capo runtime target readiness-evidence --latest [--runner local-process|remote-process|container] [--status available|disabled|unhealthy] --out DIR [--state PATH]
  capo runtime target evidence --target TARGET_ID --out DIR [--state PATH]
  capo runtime target evidence --latest [--runner local-process|remote-process|container] [--status available|disabled|unhealthy] --out DIR [--state PATH]
  capo runtime target list [--state PATH]
  capo connectivity expose-stub --endpoint ENDPOINT_ID --owner-kind runtime_target|capo_server --owner-id OWNER_ID --channel control|stdio|logs|dashboard|artifact --exposure loopback|private|public [--address REF] [--record] [--state PATH]
  capo connectivity request-approval --exposure EXPOSURE_ID [--approval APPROVAL_ID] [--state PATH]
  capo connectivity activate-exposure --exposure EXPOSURE_ID [--state PATH]
  capo connectivity revoke-exposure --exposure EXPOSURE_ID [--reason REASON] [--state PATH]
  capo connectivity exposure-status --exposure EXPOSURE_ID [--state PATH]
  capo connectivity exposure-status --latest [--owner-kind runtime_target|capo_server] [--owner-id OWNER_ID] [--channel control|stdio|logs|dashboard|artifact] [--state PATH]
  capo connectivity exposure-evidence --exposure EXPOSURE_ID --out DIR [--state PATH]
  capo connectivity exposure-evidence --latest [--owner-kind runtime_target|capo_server] [--owner-id OWNER_ID] [--channel control|stdio|logs|dashboard|artifact] --out DIR [--state PATH]
  capo project memory index --root PATH [--state PATH]
  capo project memory next [--path PATH] [--state PATH]
  capo project memory plan-next --agent NAME --adapter codex|claude [--path PATH] [--workspace PATH] [--artifacts PATH] [--record] [--state PATH]
  capo project memory start-next --agent NAME [--path PATH] [--state PATH]
  capo project memory import --source-task SOURCE_TASK_ID [--expected-hash HASH] [--task TASK_ID] [--state PATH]
  capo project memory propose --source-task SOURCE_TASK_ID --out DIR [--expected-hash HASH] [--task TASK_ID] [--summary TEXT] [--state PATH]
  capo project memory apply --proposal PATH [--confirm] [--state PATH]
  capo evidence export --session SESSION_ID --out DIR [--state PATH]
  capo eval task-outcome --session SESSION_ID --out DIR [--state PATH]
  capo review record --session SESSION_ID --reviewer NAME --kind blocker|finding|no_blockers --summary TEXT --out DIR [--severity LEVEL] [--tool-call TOOL_CALL_ID] [--follow-up-source-task SOURCE_TASK_ID] [--state PATH]
  capo tool run-wrapper --tool TOOL --workspace PATH --artifacts PATH [--policy read-only|reviewer|trusted-local] [--path PATH] [--content TEXT] [--message TEXT] [--program PROGRAM] [--argv-json JSON] [--cwd PATH] [--record] [--state PATH]

Primary model:
  Capo is a local-first controller/server for tracked coding agents.
  The CLI is one client for inspecting state, sending instructions, dispatching agents, and exporting evidence.
  Markdown-backed planning files enter Capo as project memory via `capo project memory ...`.
  Prefer project memory, source task, agent, session, dispatch, and evidence commands for new workflows.

Compatibility commands:
  These transitional commands remain for existing local scripts and repository migration only.
  Prefer the equivalent `capo project memory ...` commands in new examples and tests.

  capo workpad index --root PATH [--state PATH]
  capo workpad next [--path PATH] [--state PATH]
  capo workpad plan-next --agent NAME --adapter codex|claude [--path PATH] [--workspace PATH] [--artifacts PATH] [--record] [--state PATH]
  capo workpad start-next --agent NAME [--path PATH] [--state PATH]
  capo workpad import --workpad-task WORKPAD_TASK_ID [--expected-hash HASH] [--task TASK_ID] [--state PATH]
  capo workpad propose --workpad-task WORKPAD_TASK_ID --out DIR [--expected-hash HASH] [--task TASK_ID] [--summary TEXT] [--state PATH]
  capo workpad apply --proposal PATH [--confirm] [--state PATH]

Compatibility options:
  `capo dashboard` still accepts `--workpad-path` and `--workpad-status` as aliases for `--source-path` and `--source-status`.
  `capo review record` still accepts `--follow-up-workpad-task` as an alias for `--follow-up-source-task`.

Safety notes:
  Capo uses command envelopes, controller/state read models, and bounded adapter evidence.
  It does not read provider credentials or inspect vendor subscription state.
";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ParsedArgs {
    pub(crate) state_root: PathBuf,
    pub(crate) args: Vec<String>,
}

impl ParsedArgs {
    pub(crate) fn new(raw_args: Vec<String>) -> Result<Self, String> {
        let mut state_root = PathBuf::from(DEFAULT_STATE_ROOT);
        let mut args = Vec::new();
        let mut iter = raw_args.into_iter();

        while let Some(arg) = iter.next() {
            if arg == "--state" {
                let value = iter
                    .next()
                    .ok_or_else(|| "--state requires a path".to_string())?;
                state_root = PathBuf::from(value);
            } else {
                args.push(arg);
            }
        }

        Ok(Self { state_root, args })
    }
}

pub(crate) fn required_arg(args: &[String], key: &str) -> Result<String, String> {
    optional_arg(args, key).ok_or_else(|| format!("{key} is required"))
}

pub(crate) fn optional_arg(args: &[String], key: &str) -> Option<String> {
    args.windows(2)
        .find_map(|window| (window[0] == key).then(|| window[1].clone()))
}

pub(crate) fn has_flag(args: &[String], key: &str) -> bool {
    args.iter().any(|arg| arg == key)
}
