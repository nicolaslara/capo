# R4/R6 Finding: Stack Choice And Runtime/Tunnel Options

Date observed: 2026-05-25

Scope: R4 stack choice plus R6 runtime/tunnel options, with stack implications for controller, adapters, local models, voice, and memory.

## Recommendation

Use a Rust-first hybrid stack for the prototype:

- Rust controller daemon and CLI/TUI: orchestration state machine, event log, capability grants, process supervision, ACP/JSON-RPC boundary, HTTP/WebSocket dashboard API, and audit logging.
- SQLite plus markdown workpads for v0 state/memory: SQLite event log and read models for operational truth; markdown files remain human-auditable project state.
- Python sidecars for ecosystem-heavy adapters only: voice transcription, local-model experiments, memory-system experiments, and any provider SDK where Python support is materially better.
- IPC/plugin boundary: subprocess sidecars over newline-delimited JSON-RPC or ACP-compatible stdio, supervised by the Rust controller. Do not embed Python in the controller for v0.
- Runtime v0: local process execution first, with explicit workspace root, environment allowlist, process group tracking, PTY/log capture, and stop/interrupt semantics.
- Runtime v1 direction: add remote runners reached through SSH or Tailscale, with Capo controller state staying local until remote recovery and security semantics are proven.
- Sandbox v0: capability model plus conservative OS/process controls first; use stronger container or kernel sandboxing as an optional runtime adapter, not as the core controller assumption.

Confidence: medium-high for Rust-first hybrid and local-first runtime; medium for exact sandbox implementation because macOS, Linux, and cloud runners have different practical enforcement surfaces.

## Why Not Rust-Only Or Python-Only

| Option | Fit | Advantages | Costs | Recommendation |
| --- | --- | --- | --- | --- |
| Rust-only | Strong for controller, state, process, security, deployment | Single binary, strong typing, async process supervision, good CLI/TUI/server ecosystem, good ACP alignment | Slower for voice/local-model/memory integrations; fewer batteries for ML workflows | Do not force Rust-only for integrations. |
| Python-only | Strong for ML, voice, memory, quick adapters | Fast iteration, best library surface for Whisper/faster-whisper, vLLM/SGLang, mem0/Graphiti/Letta/Chroma | Harder to make a small durable daemon; weaker compile-time boundary for capability/security core; packaging drift | Do not put controller authority in Python for v0. |
| Rust + Python sidecars | Best fit for Capo boundaries | Rust owns authority and durable state; Python handles high-churn ecosystems; sidecars are killable, versioned, and permission-scoped | More IPC and test surface; requires contract tests | Recommended. |

The controller is security-sensitive and should stay boring: state transitions, capability grants, process lifecycle, and restart recovery belong in Rust. Python should only receive explicit requests and scoped inputs over IPC.

## Prototype Stack

### Rust Core Candidates

| Area | Candidate | License | Why |
| --- | --- | --- | --- |
| Async/process supervision | Tokio | MIT | Standard async runtime; pairs with `tokio::process` and axum. |
| HTTP/WebSocket API | axum | MIT | Tokio-native, composable handlers/middleware. |
| CLI | clap | MIT OR Apache-2.0 | Mature argument parser. |
| TUI | ratatui | MIT | Good local control surface without browser first. |
| Serialization | serde / serde_json | MIT OR Apache-2.0 | Stable JSON-RPC/event payloads. |
| SQLite | rusqlite or sqlx | rusqlite MIT; sqlx MIT OR Apache-2.0 | `rusqlite` is simple/sync and good for local event log; `sqlx` helps if Postgres becomes near-term. |
| Embedded DB | SQLite | Public domain | Local durable store with low ops burden. |
| ACP | official ACP Rust crate/schema | Apache-2.0 | ACP protocol version is currently `1`; Rust crate exists in upstream repo. |
| PTY | portable-pty or similar | check before adding | Needed if first CLI-agent adapter needs interactive terminal semantics. |

Initial choice: `tokio`, `axum`, `clap`, `serde`, `rusqlite`, and official ACP schema/crate if it is stable enough for the specific adapter path. Add `ratatui` only if the first surface is TUI rather than web.

Use `rusqlite` before `sqlx` unless architecture chooses Postgres soon. Capo needs an append-only local event log first; compile-time SQL checking is less important than simple local persistence and migrations.

### Python Sidecar Candidates

| Area | Candidate | License | Why |
| --- | --- | --- | --- |
| Packaging | uv | MIT OR Apache-2.0 | Fast lock/sync and managed Python versions; good for repeatable sidecar envs. |
| IPC models | Pydantic | MIT | Schema validation for JSON-RPC payloads and ACP Python SDK models. |
| Optional sidecar API | FastAPI | MIT | Useful only if a sidecar needs a long-running local HTTP API; stdio is simpler first. |
| ACP Python | official ACP Python SDK | Apache-2.0 inherited from ACP project | Good for Python agent/client experiments and schema parity. |
| Rust/Python binding | PyO3 | MIT OR Apache-2.0 | Defer. Embedding/bindings increase coupling; subprocess IPC is safer for v0. |

Initial choice: no Python in the critical path unless needed by a concrete adapter. When needed, run `uv sync --locked` in a sidecar directory and launch via Rust process supervision.

## Boundary: IPC And Plugins

Use process boundaries, not in-process plugins, for v0.

Recommended sidecar contract:

- Transport: stdio newline-delimited JSON-RPC 2.0, or ACP stdio when implementing an ACP agent/client directly.
- Lifecycle: Rust spawns sidecar, sends `initialize`, receives declared capabilities, then grants only requested scopes.
- Identity: every sidecar process has `adapter_id`, version, command path, working directory, environment allowlist, and declared capability set.
- Data minimization: pass file paths, event IDs, and redacted summaries where possible; avoid handing whole workspaces or secrets to sidecars.
- Failure semantics: sidecar exit becomes a typed adapter failure event; controller remains authoritative and restartable.
- Contract tests: replay golden JSON-RPC transcripts from fixtures for every plugin protocol version.

Do not use dynamic native plugins for the prototype. ABI stability, crash isolation, and per-plugin permissions are harder than supervised processes.

## Subsystem Split

### Controller

Rust. Owns:

- agent/session/task state machine
- append-only event log and read-model projections
- capability grants, revocation, expiry, and audit trail
- process supervision, cancellation, and restart recovery
- adapter registry and sidecar launch policy
- dashboard/TUI API

### Agent Adapters

Rust for stable local CLI/ACP adapters; Python acceptable for fast-moving provider or ML adapters.

First adapter should be local-process-first:

- command template plus args
- workspace root
- environment allowlist
- PTY or stdio mode
- stdout/stderr/event parsing
- interrupt/stop strategy
- log redaction

ACP adapter should stay close to upstream schema rather than inventing Capo-only message shapes. Capo-specific concepts such as workpads, review gates, runtime health, and capability policy should wrap ACP rather than fork it.

### Local Models

Use external model servers first, not embedded inference.

| Candidate | License | Fit |
| --- | --- | --- |
| Ollama | MIT | Best v0 local model adapter because it is easy to install, has REST API/docs, and hides backend complexity. Model licenses still vary per pulled model. |
| llama.cpp | MIT | Good lower-level local inference path and portable CPU/GPU story; more integration work than Ollama. |
| vLLM | Apache-2.0 | Best for GPU server/high-throughput OpenAI-compatible serving; heavy Python/CUDA dependency surface. |
| SGLang | Apache-2.0 | Strong serving/runtime candidate for structured/high-throughput experiments; heavy for v0. |

Recommendation: prototype local-model support as a provider adapter to an OpenAI-compatible or Ollama HTTP endpoint. Do not link model runtimes into Capo.

### Voice

Python sidecar first.

| Candidate | License | Fit |
| --- | --- | --- |
| OpenAI Whisper | MIT | Reference implementation; Python/PyTorch dependency surface. |
| faster-whisper | MIT | Practical sidecar candidate for local transcription; uses CTranslate2 and supports quantized inference. |
| whisper.cpp | MIT | Good future Rust/C++ integration path for smaller local binary footprint. |

Recommendation: defer production voice, but define voice as an input-surface adapter that emits the same command/message model as text. Store raw audio/transcripts only behind explicit user policy; default to transient audio and redacted transcript events.

### Memory

Use local, exportable memory first.

| Candidate | License/status | Fit |
| --- | --- | --- |
| Markdown files | Project-owned | Best v0 human-auditable memory and workpad continuity. |
| SQLite event log/index | SQLite public domain | Best v0 operational state and searchable index over markdown artifacts. |
| Chroma | Apache-2.0 | Good Python vector DB candidate if simple semantic search is needed. |
| Qdrant | Apache-2.0 | Stronger service-style vector DB; better v1 than v0 unless remote/shared memory is required. |
| LanceDB | Apache-2.0 | Attractive local columnar/vector option; evaluate Rust/Python maturity before choosing. |
| mem0 | Apache-2.0 | Useful prior art for agent memory APIs, but do not let it own Capo operational truth. |
| Graphiti | Apache-2.0 | Useful for temporal graph memory research; too much dependency/ontology weight for v0. |
| Letta | likely useful prior art; license must be rechecked before dependency | Study stateful-agent concepts; do not embed in v0 controller. |
| Tana / Capacities | hosted/product integrations | Treat as import/export or optional sync targets, not core memory stores. |

Recommendation: v0 = SQLite event log plus markdown references. v1 = optional semantic index sidecar, probably Chroma/LanceDB for local mode or Qdrant for service mode, after data ownership/export semantics are defined.

## Runtime Options

| Runtime | Security | Operations | Fit |
| --- | --- | --- | --- |
| Local process | Weak isolation unless combined with OS controls; clear audit if controlled by capability profile | Easiest to build and debug; works on current project | Prototype default. |
| Local container/devcontainer | Better filesystem/process isolation; Docker socket is high privilege; Desktop licensing matters | Good reproducibility; requires image/build lifecycle | v1 adapter, not v0 requirement. |
| Linux sandbox wrapper | Good for scoped FS/network if Landlock/bubblewrap/nsjail available | Linux-specific; policy UX matters | Add as optional local runtime profile. |
| Cloud VM/devbox | Strong host isolation and clean environments; secrets/network policy need care | More lifecycle cost and recovery complexity | v1 once local event model works. |
| Firecracker/gVisor micro-sandbox | Stronger tenant isolation | Operationally heavy for a personal prototype | Later, for untrusted/multi-tenant execution. |

### Local Process Execution Requirements

The v0 runtime adapter should implement:

- spawn with explicit command, args, workspace root, and env allowlist
- process group/session tracking so stop/interrupt reaches children where possible
- stdout/stderr capture with redaction hooks
- optional PTY mode for interactive CLIs
- heartbeats and last-event timestamps
- configurable kill escalation: interrupt -> terminate -> kill
- working-directory and path-scope checks before launch
- persisted runtime events: spawned, ready, output, capability_request, interrupted, exited, failed

Local process execution is not a security sandbox. It is a controllable runtime for trusted local agents. Capo should label it accordingly.

### Cloud/Devbox Options

| Candidate | License/status | Role |
| --- | --- | --- |
| Dev Container spec | spec CC-BY-4.0, code MIT | Good portable workspace descriptor for local/container/cloud reuse. |
| Docker Engine/containerd | Apache-2.0 | Standard local/container runtime substrate; Docker Desktop has separate commercial terms. |
| DevPod | open-source devcontainer workflow; dependency page shows permissive stack | Candidate for personal cloud/devbox provisioning if Capo should target existing devcontainer workflows. |
| Coder | AGPL-3.0 core plus enterprise licensing | Strong prior art for governed developer workspaces and agents, but license/operational weight make it a poor embedded dependency. |
| Daytona | Apache-2.0 per docs | Candidate prior art for development-environment management; evaluate before integrating. |

Recommendation: model remote execution as `RuntimeRunner` implementations. The controller should not care whether the runner is a local process, SSH command, devcontainer, DevPod/Coder/Daytona workspace, or future VM.

## Tunnel And Connectivity Options

| Option | Strengths | Weaknesses | Prototype recommendation |
| --- | --- | --- | --- |
| Local loopback only | Smallest attack surface; no account dependency | No remote/mobile control | Default v0. |
| SSH | Ubiquitous, auditable, supports command execution and reverse forwarding | Key management, host trust, port forwarding footguns | First remote-runner path. |
| Tailscale tailnet | Device identity, ACLs, private connectivity, Tailscale SSH, Serve for private services | Account/control-plane dependency; tailnet policy must be audited | Best near-term private remote-control path. |
| Tailscale Funnel | Easy public HTTPS exposure for demos/webhooks | Public exposure by design; policy/cert/rate-limit considerations | Demo-only, off by default. |
| Cloudflare Tunnel/cloudflared | Mature reverse tunnel, Apache-2.0 client | External SaaS dependency and token management | Optional public dashboard/webhook path, not v0. |
| Generic reverse tunnel/frp/ngrok-like | Works behind NAT | More exposed secrets and external relay trust | Avoid until specific need. |

Keep connectivity separate from runtime. A local runner reached through Tailscale is still a local-process runtime on that machine; Tailscale is only reachability/auth plumbing.

## Filesystem And Capability Sandboxing

Start with a capability profile that is explicit even when enforcement is partial:

- filesystem: read roots, write roots, forbidden roots, temp root, artifact root
- shell: allowed command mode, interactive vs non-interactive, timeout, process limit
- git: allowed repos/remotes/branches, push permission, destructive command policy
- network: off, loopback, tailnet, allowlist, unrestricted
- secrets: named secret grants, expiry, redaction rules, never-log policy
- browser/subscription: separate privileged connector profile, no raw cookie/session logging

Enforcement layers by platform:

| Layer | License/status | Value | Caveat |
| --- | --- | --- | --- |
| Path checks before spawn | Capo-owned | Portable and easy to audit | Not sufficient once child has shell access. |
| Dedicated workspace copy/worktree | Capo-owned | Reduces accidental blast radius | Costs disk/time; still not sandboxing. |
| OS user separation | OS feature | Simple privilege boundary | Setup friction; cross-platform details. |
| Containers | Docker Engine/containerd Apache-2.0 | Good filesystem/process/network isolation patterns | Docker socket is privileged; Desktop licensing; not all coding agents like containers. |
| Landlock | Linux kernel feature | Unprivileged additive FS/network restrictions on Linux | Linux-only; ABI/version differences; cannot be sole cross-platform policy. |
| bubblewrap | LGPL-2.0-or-later | Practical unprivileged Linux sandbox used by Flatpak-like flows | Linux-only and namespace availability varies. |
| nsjail | Apache-2.0 | Strong Linux namespace/cgroup/seccomp sandbox | More policy and ops complexity. |
| gVisor | Apache-2.0 | Stronger container sandbox via userspace kernel | Linux/container-oriented; heavier. |
| Firecracker | Apache-2.0 | Strong VM isolation for untrusted workloads | Too heavy for first local prototype. |

Recommendation: v0 capability profiles plus workspace scoping and audit; v1 add Linux sandbox profiles and container runtime adapter. Do not claim hard isolation until enforced by OS/container/VM mechanisms and smoke-tested.

## Build And Test Implications

Rust:

- Use `cargo fmt`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test`.
- Add integration tests for event-log recovery, process lifecycle, cancellation, and adapter transcript replay.
- Keep all external process tests hermetic with temp workspaces and fake agent commands before testing real coding agents.
- Add dependency/license checks before vendoring or shipping binaries.

Python sidecars:

- Use `uv lock` and `uv sync --locked`.
- Use `ruff format`, `ruff check`, and `pytest` once sidecars exist.
- Generate or share JSON Schemas for IPC payloads; test Rust-to-Python golden transcripts.
- Keep sidecar model downloads and large caches outside the repo and behind explicit config.

Runtime/tunnel:

- Smoke-test local spawn/stop/recover before remote execution.
- For SSH/Tailscale runners, test connection failure, lost connection, duplicate runner identity, and stale process cleanup.
- Redaction tests must prove logs do not capture env secrets, tunnel auth keys, OAuth cookies, or raw voice transcripts by default.

## Security Tradeoffs

- Rust core reduces accidental authority leakage but does not make shell execution safe by itself.
- Python sidecars are acceptable only when the Rust controller grants narrow inputs and can terminate them.
- Local process execution is appropriate for trusted dogfooding, not untrusted code.
- Containers improve blast-radius control but introduce Docker daemon/socket risks.
- Tailscale is a strong private-connectivity default, but tailnet ACLs become part of Capo's security boundary.
- Reverse tunnels and public dashboard exposure should be opt-in with short-lived tokens and audit events.
- Subscription-backed connectors must be treated as privileged runtimes because browser/session credentials are not ordinary API keys.
- Voice adds privacy risk: raw audio and transcripts can contain secrets, so default retention should be minimal.

## Open Questions

- Should the first real agent adapter target ACP directly, an existing CLI agent through PTY/stdio, or both behind the same trait?
- Is the first user-facing surface CLI-only, TUI, or a small local web dashboard? This affects whether `ratatui` or `axum` is first in the scaffold.
- What OS should v0 optimize for: macOS local dogfood first, Linux runner first, or both? Strong sandbox choices depend on this.
- Should Capo require Tailscale for remote dogfood, or keep SSH as the only required remote primitive?
- Which local-model path matters first: Ollama-compatible HTTP for convenience, or vLLM/SGLang for serious GPU serving?
- What is the retention policy for voice transcripts and agent logs containing potentially sensitive project data?
- Before depending on Letta/Tana/Capacities integrations, what export/data-ownership guarantees are acceptable?

## Source Notes

Primary sources checked on 2026-05-25:

- ACP repository: https://github.com/agentclientprotocol/agent-client-protocol
- ACP Python SDK docs: https://agentclientprotocol.github.io/python-sdk/
- ACP Rust SDK/RFD: https://agentclientprotocol.com/rfds/rust-sdk-v1 and https://github.com/agentclientprotocol/rust-sdk
- Tokio: https://github.com/tokio-rs/tokio and https://tokio.rs/
- axum: https://github.com/tokio-rs/axum
- clap: https://github.com/clap-rs/clap
- serde: https://github.com/serde-rs/serde
- rusqlite: https://github.com/rusqlite/rusqlite
- sqlx: https://github.com/launchbadge/sqlx
- ratatui: https://github.com/ratatui/ratatui
- uv: https://github.com/astral-sh/uv and https://docs.astral.sh/uv/reference/policies/python/
- Pydantic: https://github.com/pydantic/pydantic
- FastAPI: https://github.com/FastAPI/FastAPI
- PyO3: https://github.com/PyO3/pyo3
- SQLite copyright/license: https://www.sqlite.org/copyright.html
- Ollama: https://github.com/ollama/ollama and https://docs.ollama.com/index
- llama.cpp: https://github.com/ggml-org/llama.cpp
- vLLM: https://github.com/vllm-project/vllm
- SGLang: https://github.com/sgl-project/sglang
- Whisper: https://github.com/openai/whisper
- faster-whisper: https://github.com/SYSTRAN/faster-whisper
- Chroma OSS docs: https://docs.trychroma.com/docs/overview/oss
- Qdrant: https://github.com/qdrant/qdrant
- LanceDB: https://docs.lancedb.com/faq/faq-oss
- mem0: https://github.com/mem0ai/mem0
- Graphiti: https://github.com/getzep/graphiti
- Dev Container spec: https://github.com/devcontainers/spec
- Docker Engine docs: https://docs.docker.com/engine/
- containerd: https://github.com/containerd/containerd
- Coder: https://github.com/coder/coder
- Daytona docs: https://docs.app.codeanywhere.com/about/what-is-daytona/
- Tailscale SSH docs: https://tailscale.com/kb/1193/tailscale-ssh
- Tailscale policy syntax: https://tailscale.com/kb/1337/policy-syntax
- Tailscale Funnel/Serve docs: https://tailscale.com/docs/reference/tailscale-cli/funnel and https://tailscale.com/docs/features/tailscale-funnel
- Tailscale repository/license: https://github.com/tailscale/tailscale
- OpenSSH: https://www.openssh.org/
- cloudflared: https://github.com/cloudflare/cloudflared and https://developers.cloudflare.com/tunnel/setup/
- Landlock kernel docs: https://kernel.org/doc/html/v6.0/security/landlock.html and https://cdn.kernel.org/doc/html/latest/userspace-api/landlock.html
- bubblewrap: https://github.com/containers/bubblewrap
- nsjail: https://github.com/google/nsjail
- gVisor: https://gvisor.dev/
- Firecracker: https://github.com/firecracker-microvm/firecracker
