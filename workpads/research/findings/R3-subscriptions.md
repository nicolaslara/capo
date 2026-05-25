# R3 - Subscription-Backed Agent Connectors

Observed: 2026-05-25

Scope: Claude Code Max, ChatGPT Pro/ChatGPT plan workflows, local CLI automation, browser/session automation, credential isolation, audit, and revocation.

## Executive Recommendation

Capo should support subscription-backed agents only through vendor-supported local agent surfaces first:

1. `codex` CLI / Codex SDK / Codex access tokens where available.
2. `claude` CLI interactive mode and `claude -p` / Claude Agent SDK where the user's plan and terms permit it.
3. API-key or workload-identity providers for shared, hosted, CI, or productized automation.

Capo should not scrape or remote-control ChatGPT or Claude web UIs as a normal connector. Browser automation may be useful for local smoke tests or user-owned experiments, but it is brittle, hard to audit, may capture session cookies, and can conflict with service terms against automated extraction, rate-limit bypass, credential sharing, or routing consumer credentials through a third-party product.

## Primary Source Facts

### Claude Code and Claude Max

- Claude Max is a paid consumer plan with 5x and 20x tiers, higher usage than Pro, priority access, and explicit access to Claude Code for terminal workflows. Source: https://support.claude.com/en/articles/11049741-what-is-the-max-plan
- Claude Code supports login with Claude Pro or Max subscriptions. If `ANTHROPIC_API_KEY` is set, Claude Code uses the API key instead of the subscription and may incur API charges. Source: https://support.claude.com/en/articles/11145838-use-claude-code-with-your-pro-or-max-plan
- Claude Code authentication supports Claude.ai accounts, Teams/Enterprise, Claude Console, and cloud providers. It stores credentials in macOS Keychain, `~/.claude/.credentials.json` with mode `0600` on Linux, or `%USERPROFILE%\.claude\.credentials.json` on Windows. `CLAUDE_CONFIG_DIR` can relocate credentials on Linux/Windows. Source: https://code.claude.com/docs/en/authentication
- Claude Code auth precedence is cloud provider credentials, `ANTHROPIC_AUTH_TOKEN`, `ANTHROPIC_API_KEY`, `apiKeyHelper`, `CLAUDE_CODE_OAUTH_TOKEN`, then subscription OAuth credentials from `/login`. Source: https://code.claude.com/docs/en/authentication
- Claude Code can generate a one-year `CLAUDE_CODE_OAUTH_TOKEN` for CI/scripts, but bare mode does not read it and requires `ANTHROPIC_API_KEY` or `apiKeyHelper`. Source: https://code.claude.com/docs/en/authentication
- Claude Code supports non-interactive mode with `claude -p`, JSON output, allowed tools, and permission-prompt tooling. Bare mode skips local hooks, skills, plugins, MCP servers, memory, and `CLAUDE.md`, and is recommended for CI/scripts. Source: https://code.claude.com/docs/en/headless
- Starting 2026-06-15, Claude Agent SDK and `claude -p` usage on eligible subscription plans moves to a separate monthly Agent SDK credit rather than normal interactive subscription limits. The credit covers Python/TypeScript Agent SDK, `claude -p`, Claude Code GitHub Actions, and third-party apps built on the Agent SDK; usage beyond credit moves to usage credits only if enabled. Source: https://support.claude.com/en/articles/15036540-use-the-claude-agent-sdk-with-your-claude-plan
- Anthropic legal docs say OAuth subscription authentication is for ordinary use of Claude Code and native Anthropic apps, and that developers building products/services that interact with Claude capabilities should use Claude Console API keys or supported cloud providers. Anthropic says it does not permit third-party developers to offer Claude.ai login or route requests through Free, Pro, or Max credentials on behalf of users. Source: https://code.claude.com/docs/en/legal-and-compliance
- Claude API supports API keys and Workload Identity Federation. API keys have no expiry and should be stored, rotated, and revoked like static secrets; WIF exchanges an IdP-issued token for short-lived Claude API access. Source: https://platform.claude.com/docs/en/manage-claude/authentication

### ChatGPT Plans, Codex, and OpenAI

- Codex is included with ChatGPT Plus, Pro, Business, and Enterprise/Edu plans; for a limited period it is also included with Free and Go. Users sign in with a ChatGPT account and launch a supported client: Codex app, CLI, IDE extension, or Codex web. Source: https://help.openai.com/en/articles/11369540-using-codex-with-your-chatgpt-plan
- Codex usage limits vary by plan and count toward an "agentic usage" limit. Codex usage from larger repositories, long-running tasks, or extended sessions consumes more of the allowance. Source: https://help.openai.com/en/articles/11369540-using-codex-with-your-chatgpt-plan
- Codex CLI is OpenAI's local coding agent. It can read, change, and run code in the selected directory. It is open source, built in Rust, installable with `npm i -g @openai/codex`, and can authenticate with a ChatGPT account or API key. Source: https://developers.openai.com/codex/cli
- `codex exec` is the documented non-interactive mode. It streams progress to stderr, prints the final answer to stdout, supports JSONL event output, structured output schemas, sandbox modes, and `--ephemeral`. By default it runs in a read-only sandbox; broader access should be explicit. Source: https://developers.openai.com/codex/noninteractive
- OpenAI recommends API key auth for CI, with `CODEX_API_KEY` supported for `codex exec`. Source: https://developers.openai.com/codex/noninteractive
- Codex access tokens provide repeatable non-interactive ChatGPT-workspace identity for Codex local workflows, but are currently supported for ChatGPT Business and Enterprise workspaces, not consumer Pro/Plus. They should be stored in a secret manager, kept out of logs, rotated, and revoked when stale. Source: https://developers.openai.com/codex/enterprise/access-tokens
- The Codex SDK programmatically controls local Codex agents. The TypeScript SDK is documented for server-side use; the Python SDK is experimental and controls the local Codex app-server over JSON-RPC. Source: https://developers.openai.com/codex/sdk
- OpenAI Terms of Use prohibit sharing account credentials, automatically or programmatically extracting data/output, disrupting services, circumventing rate limits or restrictions, and bypassing protective measures. Source: https://openai.com/policies/terms-of-use/

### Browser Automation Baseline

- Playwright browser contexts provide isolated, incognito-like sessions and are the right primitive if Capo ever runs a user-owned browser automation experiment. Source: https://playwright.dev/docs/browser-contexts
- Playwright can save and reload authenticated storage state, which is useful for tests but creates a high-value secret artifact if used with ChatGPT or Claude sessions. Source: https://playwright.dev/docs/auth

## Feasibility Matrix

| Connector path | Feasibility | Support status | Main risks | Capo stance |
| --- | --- | --- | --- | --- |
| Claude Code interactive CLI with Max/Pro login | High | Documented first-party CLI flow | Shared subscription limits, local credentials, provider UX changes, tool permissions | Good v0 adapter. Spawn local `claude`, observe output, do not read token files. |
| Claude `claude -p` non-interactive | Medium-high | Documented. Billing/limit behavior changes on 2026-06-15 to Agent SDK credit | Credit exhaustion, bare-mode auth differences, local config leakage unless `--bare`, ToS boundary for productized third-party use | Good for user-owned local scripts. Require explicit mode and budget display. |
| Claude Agent SDK with user's subscription | Medium | Documented credit coverage for eligible users starting 2026-06-15, but legal docs warn third-party products should use API/cloud auth | Product/legal ambiguity for Capo if it offers Claude.ai login or routes requests on behalf of users | Use only for local, user-owned Capo installs. For hosted/shared Capo, require API key/WIF/enterprise route. |
| Claude API key / WIF | High | Documented API auth | Static-key leakage for API keys, cost exposure | Preferred for hosted, CI, multi-user, or organization-managed automation. |
| ChatGPT plan via Codex CLI login | High | Documented first-party CLI flow | Agentic usage limits, local session credentials, plan availability changes | Good v0 adapter. Treat Codex as local agent runtime/provider connector. |
| Codex `codex exec` | High | Documented automation path | Sandbox misuse, JSON/event schema drift, local config/rules surprises, cost/limit ambiguity under ChatGPT auth | Strong v0 adapter. Prefer read-only by default; use JSONL event stream. |
| Codex SDK | Medium-high | Documented TS SDK; Python experimental | Requires local Codex/app-server assumptions; SDK maturity differs by language | Use TS later if Capo needs richer control than CLI. Rust core should wrap CLI first. |
| Codex access tokens | Medium | Documented for Business/Enterprise only | Not available for consumer Pro; token theft allows runs as creator; audit identity can blur if shared | Good enterprise path. Not a ChatGPT Pro solution. |
| OpenAI API key with Codex/GPT coding models | High | Documented OpenAI API | Separate billing from ChatGPT subscription; API rate/cost management | Preferred for service/CI/hosted Capo when subscription semantics are not required. |
| ChatGPT web UI browser automation | Low | Not a documented automation interface for Capo | Session cookie exposure, UI brittleness, automated extraction/ToS risk, anti-bot controls, weak audit | Do not build as first-class connector. Allow only an explicitly labeled local experiment if ever needed. |
| Claude web UI browser automation | Low | Not a documented automation interface for Capo | Same as above plus subscription OAuth routing caveats | Do not build as first-class connector. Use Claude Code/Agent SDK/API instead. |
| Reverse-engineered private endpoints/session-token reuse | Very low | Unsupported | Account suspension, credential theft, breakage, legal/product risk | Out of scope. Explicitly reject. |

## Supported APIs/SDKs Vs Subscription Surfaces

Supported API/SDK paths:

- OpenAI API and SDKs with API keys for direct model calls.
- Codex CLI, `codex exec`, and Codex SDK for local coding-agent workflows.
- Codex access tokens for Business/Enterprise workspace local automation.
- Claude API with API keys or WIF.
- Claude Code CLI, `claude -p`, and Claude Agent SDK under the user's eligible plan and provider terms.

Subscription-backed but not generic API replacement:

- ChatGPT Pro/Plus/Business/Edu/Enterprise Codex entitlement is accessed through supported Codex clients, not by scraping ChatGPT web or reusing browser cookies.
- Claude Max/Pro subscription access is accessed through Claude Code login and related first-party/Agent SDK surfaces, not by extracting OAuth tokens into unrelated clients or routing other users through a Capo-hosted broker.

Unsupported or unacceptable for Capo:

- Browser scraping of ChatGPT/Claude as the default execution path.
- Sharing a user's consumer subscription credentials across teammates or hosted tenants.
- Capturing and replaying private session cookies or undocumented bearer tokens.
- Circumventing rate limits, approval gates, or provider safety controls.

## Security Boundary Proposal

### Boundary names

Capo should model subscription connectors as privileged local agent runtimes, not as ordinary model providers.

```text
Capo controller
  -> Agent protocol adapter
    -> Local agent process adapter
      -> Vendor CLI/SDK process
        -> Vendor-owned subscription/API credential store
```

The controller owns tasks, state, approvals, audit, and revocation metadata. The vendor CLI/SDK owns provider authentication and should keep tokens in its normal OS-backed location.

### Credential handling

- Capo must not read, copy, persist, log, or sync vendor OAuth tokens, browser cookies, keychain entries, `.credentials.json`, ChatGPT browser storage, or Playwright storage state.
- Capo may record non-secret credential metadata: provider, auth mode observed from a CLI status command when available, account/workspace label if the vendor exposes it safely, expiry if known, and revocation instructions.
- Capo should launch vendor CLIs with a minimized environment. Explicitly scrub unrelated provider keys unless the selected connector needs them; this avoids accidentally causing Claude Code to prefer `ANTHROPIC_API_KEY` over subscription auth.
- For Claude Code on Linux/Windows, Capo can support per-agent `CLAUDE_CONFIG_DIR` only when the user explicitly opts into isolated credentials. On macOS, Keychain-backed Claude credentials may remain user-global unless the vendor supports per-profile isolation.
- For Codex, Capo should support per-agent `CODEX_HOME`/config isolation if the current CLI supports it; otherwise, it should isolate workspaces and execution state but treat auth as user-global.
- API keys and Codex enterprise access tokens should be stored only in a user-approved secret manager or injected environment, never in Capo project workpads.

### Execution isolation

- Default to local process adapters using vendor CLI permission and sandbox controls.
- For Codex, default `codex exec` to read-only unless the task requires writes; escalate to `workspace-write` only for implementation tasks; reserve `danger-full-access` for disposable containers or explicit user approval.
- For Claude, prefer `claude -p --bare` for deterministic scripts only when API key or `apiKeyHelper` auth is intended. If using subscription OAuth, avoid `--bare` unless docs change because bare mode does not read `CLAUDE_CODE_OAUTH_TOKEN`.
- Each agent session should get its own workspace checkout, process group, stdout/stderr capture, and kill/revoke path.
- Browser automation experiments, if ever allowed, must use a dedicated browser profile/context, no default browser profile, encrypted storage, no cloud sync, and an explicit "experimental/unsupported" connector label.

### Audit model

Capo should emit audit events for:

- Connector selected and auth mode category: subscription CLI, API key, WIF, enterprise access token, or browser experiment.
- Process start/stop, command arguments after secret redaction, working directory, sandbox mode, and permission mode.
- Tool/capability grants, escalations, approvals, denials, and expiry.
- Provider-reported usage/cost/credit fields when available, such as Claude JSON `total_cost_usd` or Codex usage/event data.
- Revocation-relevant events: login requested, logout requested, token/access-token rotation requested, connector disabled.

Audit records must not include prompts or outputs by default if they may contain secrets. Capo should support redacted transcript storage and per-session retention controls.

### Revocation model

- Local subscription CLI: user can revoke by vendor logout (`/logout` or equivalent), deleting the isolated config/profile, disabling the connector in Capo, or removing the local vendor CLI.
- Claude API key: revoke in Claude Console; for WIF, revoke the IdP rule/service account or upstream identity permission.
- Codex API key: rotate/revoke via OpenAI platform secret management.
- Codex Business/Enterprise access token: revoke from the ChatGPT Access tokens page; workspace owners/admins can revoke workspace tokens.
- Browser experiment: delete the browser profile/storage state and revoke connected app sessions from the provider account settings if available.

Capo should make revocation a first-class connector action, even if the implementation is an instruction plus a local cleanup command for v0.

## Product And Legal Caveats

- Consumer subscriptions are personal entitlements. They are not a clean basis for a hosted multi-user Capo service unless the vendor offers an explicit workspace/enterprise token or API contract.
- "User-owned local automation" and "Capo-hosted connector serving other users" are different legal/product categories. The former can use local CLI/SDK surfaces more safely; the latter should use API keys, WIF, or enterprise access tokens.
- Current Claude docs contain a nuanced boundary: Agent SDK subscription credits cover some third-party apps, while legal docs say third-party developers may not offer Claude.ai login or route requests through consumer plan credentials on behalf of users. Capo should avoid being the credential broker and should require the vendor's own local login on the user's machine.
- OpenAI's Terms of Use make generic web automation risky because they prohibit credential sharing, programmatic extraction, disruption, rate-limit circumvention, and bypassing protective measures. Supported Codex clients are the safer route.
- Plan entitlements, limits, and SDK credit rules are changing quickly. Capo should date connector assumptions and expose provider capability/version metadata rather than hard-coding subscription promises.

## Prototype Recommendation

For the first Capo prototype:

1. Build a `LocalCliAgent` abstraction that can run `codex exec --json` and parse JSONL events into Capo's event model.
2. Add a second adapter for `claude -p --output-format stream-json` or plain JSON/text, but mark subscription auth mode as user-owned local only.
3. Do not implement ChatGPT/Claude web automation.
4. Store connector configuration as non-secret metadata plus command templates; delegate secrets to vendor CLIs or OS/secret-manager storage.
5. Add an explicit `credential_scope` field: `user-local-subscription`, `api-key`, `wif`, `enterprise-access-token`, `browser-experiment`.
6. Add an explicit `productization_allowed` field. Default `user-local-subscription` to local-only and not hosted/multi-tenant.

Confidence: medium-high. The local CLI paths are well supported. The main uncertainty is not technical feasibility; it is vendor policy drift around subscription-backed automation and the boundary between personal local use and third-party product integration.

## Open Questions

- Should Capo's v0 prioritize Codex because `codex exec --json` exposes a clean event stream, or Claude Code because Max is a core target?
- Should Capo enforce isolated per-agent vendor config directories where possible, or keep one user-global login per vendor for simplicity?
- Will Capo ever be hosted/multi-tenant? If yes, consumer subscription connectors should be disabled for hosted mode unless vendors offer a compliant enterprise auth flow.
- How much transcript detail should Capo persist by default for subscription-backed sessions, given prompt/output can contain source code and secrets?
