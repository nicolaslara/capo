use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use capo_adapters::{
    CodexExecAdapter, LocalAdapterLaunchPlan, scan_artifacts_for_sensitive_markers,
};
use capo_core::{CommandIntent, CommandTarget, RunId, SessionId};
use capo_runtime::LocalProcessRunner;
use capo_state::{
    AdapterDispatchGateProjection, AdapterDispatchPlanProjection,
    AdapterDispatchPromptSourceProjection, EventKind, NewEvent, ProjectionRecord, RedactionState,
};

use crate::dispatch::{DispatchExecutionOutcome, DispatchReplayMetadata};
use crate::util::{
    adapter_label, command_identity_hash, parse_adapter_events, provider_kind_for_adapter,
    stable_hash,
};
use crate::{
    CapoServer, DispatchRunSummary, LiveProviderPreflightSummary, ServerClientOrigin, ServerError,
    ServerResult,
};

pub(crate) struct LiveProviderPreflightRequest<'a> {
    pub(crate) request_id: &'a str,
    pub(crate) agent_name: &'a str,
    pub(crate) adapter: &'a str,
    pub(crate) goal: &'a str,
    pub(crate) workspace: &'a str,
    pub(crate) artifacts: &'a str,
    pub(crate) session_id: &'a str,
    pub(crate) run_id: &'a str,
    pub(crate) turn_id: &'a str,
    pub(crate) capability_profile: &'a str,
    pub(crate) runtime_scope: &'a str,
    pub(crate) credential_scan_policy: &'a str,
    pub(crate) raw_prompt_policy: &'a str,
    pub(crate) raw_output_policy: &'a str,
    pub(crate) tool_wrapper_policy: &'a str,
    pub(crate) live_provider_opt_in: bool,
}

pub(crate) struct LiveProviderLocalRunRequest<'a> {
    pub(crate) dispatch_plan_id: &'a str,
    pub(crate) goal: &'a str,
    pub(crate) live_execution_opt_in: bool,
    pub(crate) mock_runtime_opt_in: bool,
    pub(crate) mock_provider_output_name: Option<&'a str>,
    pub(crate) mock_provider_output_jsonl: Option<&'a str>,
    pub(crate) timeout_seconds: u64,
}

struct LiveExecutionContext<'a> {
    plan: &'a AdapterDispatchPlanProjection,
    gate: &'a AdapterDispatchGateProjection,
    execution_request: &'a capo_state::AdapterDispatchExecutionRequestProjection,
    turn_id: &'a str,
}

impl CapoServer {
    pub(crate) fn preflight_live_provider(
        &self,
        origin: &ServerClientOrigin,
        request: LiveProviderPreflightRequest<'_>,
    ) -> ServerResult<LiveProviderPreflightSummary> {
        let adapter_label = adapter_label(request.adapter)?.to_string();
        if adapter_label == "acp" {
            return Err(ServerError::AdapterFixture(
                "live provider preflight supports codex or claude, not acp".to_string(),
            ));
        }
        let session_id = SessionId::new(request.session_id);
        let run_id = RunId::new(request.run_id);
        let (_session, _run_projection, agent, _run) =
            self.run_refs_for_session_run(&session_id, &run_id)?;
        self.require_session_adapter(&session_id, &adapter_label)?;
        if agent.name != request.agent_name {
            return Err(ServerError::AdapterFixture(format!(
                "live preflight agent mismatch: session belongs to {}, requested {}",
                agent.name, request.agent_name
            )));
        }
        let provider_kind = provider_kind_for_adapter(&adapter_label).to_string();
        let mut reasons = Vec::new();
        if !request.live_provider_opt_in {
            reasons.push("missing_live_provider_preflight_opt_in".to_string());
        }
        if request.runtime_scope != "local_process_loopback" {
            reasons.push("unsafe_runtime_scope".to_string());
        }
        if request.workspace.trim().is_empty() || request.workspace.contains("..") {
            reasons.push("unsafe_workspace_scope".to_string());
        }
        if request.artifacts.trim().is_empty() || request.artifacts.contains("..") {
            reasons.push("missing_artifact_root_policy".to_string());
        }
        if request.capability_profile != "trusted-local" {
            reasons.push("missing_live_capability_profile".to_string());
        }
        if request.credential_scan_policy != "metadata_only_no_secret_read" {
            reasons.push("credential_handling_policy_not_explicit".to_string());
        }
        if request.raw_prompt_policy != "not_rendered" {
            reasons.push("raw_prompt_policy_not_redacted".to_string());
        }
        if request.raw_output_policy != "artifacts_scanned_redacted" {
            reasons.push("raw_output_policy_missing_artifact_scan".to_string());
        }
        if request.tool_wrapper_policy != "capo_wrapped_required" {
            reasons.push("tool_wrapper_instrumentation_missing".to_string());
        }
        let stored_capability_profile = if request.capability_profile == "trusted-local" {
            request.capability_profile
        } else {
            "rejected"
        };
        let stored_runtime_scope = if request.runtime_scope == "local_process_loopback" {
            request.runtime_scope
        } else {
            "rejected"
        };
        let stored_credential_scan_policy =
            if request.credential_scan_policy == "metadata_only_no_secret_read" {
                request.credential_scan_policy
            } else {
                "rejected"
            };
        let stored_raw_prompt_policy = if request.raw_prompt_policy == "not_rendered" {
            request.raw_prompt_policy
        } else {
            "rejected"
        };
        let stored_raw_output_policy = if request.raw_output_policy == "artifacts_scanned_redacted"
        {
            request.raw_output_policy
        } else {
            "rejected"
        };
        let stored_tool_wrapper_policy = if request.tool_wrapper_policy == "capo_wrapped_required" {
            request.tool_wrapper_policy
        } else {
            "rejected"
        };
        if reasons.is_empty() {
            reasons.push("live_provider_preflight_ready".to_string());
        }
        let allowed = reasons == ["live_provider_preflight_ready"];
        let reason_codes = reasons.join(",");
        let status = if allowed {
            "ready_for_live_provider_execution"
        } else {
            "blocked_by_live_provider_preflight"
        };
        let next_action = if allowed {
            "run_explicit_live_provider_execution"
        } else {
            "fix_preflight_blockers"
        };
        let target_hash = stable_hash(
            format!(
                "{}:{adapter_label}:{session_id}:{run_id}:{}:{}:{}:{stored_capability_profile}:{stored_runtime_scope}:{stored_credential_scan_policy}:{stored_raw_prompt_policy}:{stored_raw_output_policy}:{stored_tool_wrapper_policy}:{}:{reason_codes}:{status}",
                request.agent_name,
                request.turn_id,
                request.workspace,
                request.artifacts,
                request.live_provider_opt_in
            )
            .as_bytes(),
        );
        let goal_hash = stable_hash(request.goal.as_bytes());
        let dispatch_plan_id =
            format!("server-live-provider-plan-{adapter_label}-{goal_hash}-{target_hash}");
        let dispatch_gate_id = format!(
            "server-live-provider-gate-{}-{}",
            stable_hash(dispatch_plan_id.as_bytes()),
            stable_hash(reason_codes.as_bytes())
        );
        let execution_request_id = format!(
            "server-live-provider-execution-request-{}",
            stable_hash(dispatch_gate_id.as_bytes())
        );
        let prompt_source_id = format!(
            "server-live-provider-prompt-source-{}",
            stable_hash(dispatch_plan_id.as_bytes())
        );
        let plan = AdapterDispatchPlanProjection {
            dispatch_plan_id: dispatch_plan_id.clone(),
            project_id: self.project_id.clone(),
            adapter_kind: adapter_label.clone(),
            provider_kind: provider_kind.clone(),
            credential_scope: "subscription_cli".to_string(),
            agent_id: agent.agent_id.clone(),
            agent_name: agent.name.clone(),
            session_id: session_id.clone(),
            run_id: run_id.clone(),
            runtime_program: "live-provider-cli-preflight".to_string(),
            runtime_arg_count: 0,
            runtime_prompt_policy: stored_raw_prompt_policy.to_string(),
            runtime_cwd: request.workspace.to_string(),
            artifact_root: request.artifacts.to_string(),
            request_env_count: usize::from(request.live_provider_opt_in) as i64,
            env_allowlist_count: usize::from(request.live_provider_opt_in) as i64,
            redaction_rule_count: 1,
            stdout_format: "provider_stream".to_string(),
            stderr_policy: "bounded_redacted_artifact".to_string(),
            provider_cli_executed: false,
            status: "live_provider_preflighted".to_string(),
            updated_sequence: 0,
        };
        let prompt_source = AdapterDispatchPromptSourceProjection {
            prompt_source_id: prompt_source_id.clone(),
            project_id: self.project_id.clone(),
            dispatch_plan_id: dispatch_plan_id.clone(),
            prompt_hash: goal_hash.clone(),
            source_kind: "server_inline_goal_hash".to_string(),
            source_ref: Some(format!("server-live-provider-turn:{}", request.turn_id)),
            source_hash: Some(goal_hash.clone()),
            materialization_status: "hash_only_live_prompt_required_at_execution".to_string(),
            raw_prompt_policy: stored_raw_prompt_policy.to_string(),
            updated_sequence: 0,
        };
        let gate = AdapterDispatchGateProjection {
            dispatch_gate_id: dispatch_gate_id.clone(),
            project_id: self.project_id.clone(),
            dispatch_plan_id: dispatch_plan_id.clone(),
            adapter_kind: adapter_label.clone(),
            provider_cli_execution_allowed: allowed,
            status: status.to_string(),
            required_dogfood_gate: "live_provider_preflight".to_string(),
            reason_codes: reason_codes.clone(),
            provider_cli_executed: false,
            runtime_prompt_policy: stored_raw_prompt_policy.to_string(),
            updated_sequence: 0,
        };
        let execution_request = capo_state::AdapterDispatchExecutionRequestProjection {
            execution_request_id: execution_request_id.clone(),
            project_id: self.project_id.clone(),
            dispatch_plan_id: dispatch_plan_id.clone(),
            dispatch_gate_id: dispatch_gate_id.clone(),
            adapter_kind: adapter_label.clone(),
            provider_cli_execution_allowed: allowed,
            provider_cli_executed: false,
            status: status.to_string(),
            opt_in_env: "CAPO_SERVER_LIVE_PROVIDER_PREFLIGHT".to_string(),
            runtime_prompt_policy: stored_raw_prompt_policy.to_string(),
            reason_codes: reason_codes.clone(),
            updated_sequence: 0,
        };
        let event = NewEvent {
            event_id: format!(
                "event-server-live-provider-preflight-{}",
                stable_hash(dispatch_gate_id.as_bytes())
            ),
            kind: EventKind::AdapterDispatchGateChecked,
            actor: origin.actor_id.clone(),
            project_id: Some(self.project_id.clone()),
            task_id: None,
            agent_id: Some(agent.agent_id.clone()),
            session_id: Some(session_id.clone()),
            run_id: Some(run_id.clone()),
            turn_id: Some(request.turn_id.to_string()),
            item_id: Some(dispatch_gate_id.clone()),
            payload_json: serde_json::json!({
                "dispatch_plan_id": dispatch_plan_id,
                "dispatch_gate_id": dispatch_gate_id,
                "execution_request_id": execution_request_id,
                "preflight_kind": "live_provider",
                "adapter": adapter_label,
                "provider_kind": provider_kind,
                "agent": agent.name,
                "target_turn_id": request.turn_id,
                "capability_profile": stored_capability_profile,
                "runtime_scope": stored_runtime_scope,
                "credential_scan_policy": stored_credential_scan_policy,
                "raw_prompt_policy": stored_raw_prompt_policy,
                "raw_output_policy": stored_raw_output_policy,
                "tool_wrapper_policy": stored_tool_wrapper_policy,
                "provider_cli_execution_allowed": allowed,
                "provider_cli_executed": false,
                "credential_material_rendered": false,
                "status": status,
                "reason_codes": reason_codes,
                "next_action": next_action,
            })
            .to_string(),
            idempotency_key: Some(format!(
                "server-live-provider-preflight:{}:{}:{}",
                self.project_id, session_id, target_hash
            )),
            redaction_state: RedactionState::Safe,
        };
        self.controller
            .state()
            .append_event(
                event,
                &[
                    ProjectionRecord::AdapterDispatchPlan(plan),
                    ProjectionRecord::AdapterDispatchPromptSource(prompt_source),
                    ProjectionRecord::AdapterDispatchGate(gate),
                    ProjectionRecord::AdapterDispatchExecutionRequest(execution_request),
                ],
            )
            .map_err(ServerError::State)?;
        let command_hash =
            command_identity_hash(format!("preflight_live_provider:{dispatch_gate_id}"));
        let command = self.command_envelope(
            request.request_id,
            origin,
            &command_hash,
            CommandTarget::Session(session_id.clone()),
            CommandIntent::SendTask,
            Some(request.goal.to_string()),
        );
        self.record_server_request_handled(
            &command,
            origin,
            "preflight_live_provider",
            None,
            Some(serde_json::json!({
                "dispatch_plan_id": dispatch_plan_id,
                "dispatch_gate_id": dispatch_gate_id,
                "execution_request_id": execution_request_id,
                "preflight_kind": "live_provider",
                "provider_cli_execution_allowed": allowed,
                "provider_cli_executed": false,
                "credential_material_rendered": false,
                "status": status,
                "reason_codes": reason_codes,
                "next_action": next_action,
            })),
        )
        .map_err(ServerError::State)?;
        Ok(LiveProviderPreflightSummary {
            dispatch_plan_id,
            dispatch_gate_id,
            execution_request_id,
            adapter: adapter_label,
            provider_kind,
            agent_name: request.agent_name.to_string(),
            session_id,
            run_id,
            capability_profile: stored_capability_profile.to_string(),
            runtime_scope: stored_runtime_scope.to_string(),
            credential_scan_policy: stored_credential_scan_policy.to_string(),
            raw_prompt_policy: stored_raw_prompt_policy.to_string(),
            raw_output_policy: stored_raw_output_policy.to_string(),
            tool_wrapper_policy: stored_tool_wrapper_policy.to_string(),
            provider_cli_execution_allowed: allowed,
            provider_cli_executed: false,
            status: status.to_string(),
            reasons: reason_codes,
            next_action: next_action.to_string(),
        })
    }

    pub(crate) fn run_live_provider_local(
        &self,
        origin: &ServerClientOrigin,
        request: LiveProviderLocalRunRequest<'_>,
    ) -> ServerResult<DispatchRunSummary> {
        let (plan, prompt_source) = self.dispatch_plan_with_prompt(request.dispatch_plan_id)?;
        let gate = self.latest_dispatch_gate(request.dispatch_plan_id)?;
        let execution_request = self.latest_execution_request(request.dispatch_plan_id)?;
        let goal_hash = stable_hash(request.goal.as_bytes());
        let mut blockers =
            self.live_execution_blockers(&plan, &prompt_source, &gate, &goal_hash, &request);
        let existing_execution = self.latest_dispatch_execution(request.dispatch_plan_id)?;
        if let Some(existing) = existing_execution
            && existing.provider_cli_executed
            && blockers.is_empty()
        {
            return Ok(DispatchRunSummary::from_execution(&existing, 0, 0, 0, 0, 0));
        }
        let workspace = normalize_policy_path(&plan.runtime_cwd)
            .map_err(|reason| ServerError::AdapterFixture(format!("unsafe workspace: {reason}")));
        let artifacts = normalize_policy_path(&plan.artifact_root).map_err(|reason| {
            ServerError::AdapterFixture(format!("unsafe artifact root: {reason}"))
        });
        let (workspace, artifacts) = match (workspace, artifacts) {
            (Ok(workspace), Ok(artifacts)) => (workspace, artifacts),
            (Err(error), _) | (_, Err(error)) => {
                blockers.push(format!("{error:?}"));
                (
                    PathBuf::from(&plan.runtime_cwd),
                    PathBuf::from(&plan.artifact_root),
                )
            }
        };

        if !blockers.is_empty() {
            let reason_codes = blockers.join(",");
            let execution = self.dispatch_execution_projection(
                &plan,
                &execution_request,
                DispatchExecutionOutcome {
                    status: "blocked_by_live_provider_execution_gate",
                    provider_cli_executed: false,
                    runtime_process_ref: None,
                    exit_code: None,
                    stdout_artifact_id: None,
                    stderr_artifact_id: None,
                    credential_scan_status: "not_run",
                    raw_output_policy: "not_captured",
                    reason_codes: &reason_codes,
                },
            );
            self.append_dispatch_execution(origin, &plan, &execution)?;
            return Ok(DispatchRunSummary::from_execution(
                &execution, 0, 0, 0, 0, 0,
            ));
        }
        let target_turn_id = prompt_source
            .source_ref
            .as_deref()
            .and_then(|source| source.strip_prefix("server-live-provider-turn:"))
            .map(ToString::to_string)
            .unwrap_or_else(|| format!("turn-{}", stable_hash(plan.dispatch_plan_id.as_bytes())));

        if let Some(mock_output) = request.mock_provider_output_jsonl {
            let context = LiveExecutionContext {
                plan: &plan,
                gate: &gate,
                execution_request: &execution_request,
                turn_id: &target_turn_id,
            };
            return self.ingest_mock_live_provider_output(
                origin,
                context,
                request
                    .mock_provider_output_name
                    .unwrap_or("mock-provider-output"),
                mock_output,
            );
        }

        let launch_plan =
            CodexExecAdapter::local_launch_plan(workspace, artifacts, request.goal.to_string());
        let context = LiveExecutionContext {
            plan: &plan,
            gate: &gate,
            execution_request: &execution_request,
            turn_id: &target_turn_id,
        };
        self.execute_codex_live_provider(origin, context, launch_plan, request.timeout_seconds)
    }

    fn live_execution_blockers(
        &self,
        plan: &AdapterDispatchPlanProjection,
        prompt_source: &AdapterDispatchPromptSourceProjection,
        gate: &AdapterDispatchGateProjection,
        goal_hash: &str,
        request: &LiveProviderLocalRunRequest<'_>,
    ) -> Vec<String> {
        let mut blockers = Vec::new();
        if plan.adapter_kind != "codex_exec" {
            blockers.push("provider_not_enabled_for_first_live_slice".to_string());
        }
        if plan.status != "live_provider_preflighted" {
            blockers.push(format!("dispatch_plan_status:{}", plan.status));
        }
        if !gate.provider_cli_execution_allowed
            || gate.status != "ready_for_live_provider_execution"
            || gate.reason_codes != "live_provider_preflight_ready"
        {
            blockers.push("latest_live_preflight_not_ready".to_string());
        }
        if prompt_source.prompt_hash != goal_hash {
            blockers.push("prompt_hash_mismatch".to_string());
        }
        if prompt_source.raw_prompt_policy != "not_rendered" {
            blockers.push("raw_prompt_policy_not_redacted".to_string());
        }
        if !request.live_execution_opt_in && request.mock_provider_output_jsonl.is_none() {
            blockers.push("missing_live_provider_execution_opt_in".to_string());
        }
        if request.mock_provider_output_jsonl.is_some() && !request.mock_runtime_opt_in {
            blockers.push("missing_mock_live_provider_runtime_opt_in".to_string());
        }
        blockers
    }

    fn ingest_mock_live_provider_output(
        &self,
        origin: &ServerClientOrigin,
        context: LiveExecutionContext<'_>,
        fixture_name: &str,
        output_jsonl: &str,
    ) -> ServerResult<DispatchRunSummary> {
        let output_hash = stable_hash(output_jsonl.as_bytes());
        self.reject_changed_dispatch_fixture(&context.plan.dispatch_plan_id, &output_hash)?;
        let adapter_events = parse_adapter_events(&context.plan.adapter_kind, output_jsonl)
            .map_err(ServerError::AdapterFixture)?;
        if adapter_events.is_empty() {
            return Err(ServerError::AdapterFixture(
                "mock live provider output produced no normalized events".to_string(),
            ));
        }
        let (_session, run_projection, _agent, run) =
            self.run_refs_for_session_run(&context.plan.session_id, &context.plan.run_id)?;
        let report = self
            .controller
            .apply_normalized_adapter_events_with_turn(&run, &adapter_events, Some(context.turn_id))
            .map_err(ServerError::State)?;
        let execution = self.dispatch_execution_projection(
            context.plan,
            context.execution_request,
            DispatchExecutionOutcome {
                status: "mocked_live_provider_output_ingested",
                provider_cli_executed: false,
                runtime_process_ref: Some(format!(
                    "mock-live-provider-ingest-{}",
                    context.plan.dispatch_plan_id
                )),
                exit_code: None,
                stdout_artifact_id: None,
                stderr_artifact_id: None,
                credential_scan_status: "not_applicable_mock",
                raw_output_policy: "content_hashed_not_rendered",
                reason_codes: "mock_live_provider_output_ingested_without_provider_cli",
            },
        );
        self.append_dispatch_execution(origin, context.plan, &execution)?;
        self.append_dispatch_run_exit_with_metadata(
            origin,
            context.plan,
            &run_projection,
            false,
            "mock_live_provider_output_ingested_without_provider_cli",
        )?;
        let replay = self.dispatch_replay_projection(
            context.plan,
            context.gate,
            fixture_name,
            &output_hash,
            &report,
        );
        self.append_dispatch_replay(origin, context.plan, &replay)?;
        Ok(DispatchRunSummary::from_execution(
            &execution,
            report.input_event_count,
            report.appended_event_count,
            report.tool_event_count,
            report.summary_event_count,
            report.completed_turn_count,
        ))
    }

    fn execute_codex_live_provider(
        &self,
        origin: &ServerClientOrigin,
        context: LiveExecutionContext<'_>,
        launch_plan: LocalAdapterLaunchPlan,
        timeout_seconds: u64,
    ) -> ServerResult<DispatchRunSummary> {
        launch_plan
            .assert_subscription_safe()
            .map_err(ServerError::AdapterFixture)?;
        fs::create_dir_all(&launch_plan.workspace_root).map_err(|error| {
            ServerError::AdapterFixture(format!("failed to create dispatch workspace: {error}"))
        })?;
        fs::create_dir_all(&launch_plan.artifact_root).map_err(|error| {
            ServerError::AdapterFixture(format!("failed to create dispatch artifact root: {error}"))
        })?;
        let runner = LocalProcessRunner::new(launch_plan.runtime_config());
        let mut process = runner
            .spawn_process(launch_plan.runtime_request(RunId::new(context.plan.run_id.to_string())))
            .map_err(|error| {
                ServerError::AdapterFixture(format!("runtime spawn failed: {error:?}"))
            })?;
        let outcome = runner
            .wait_running_with_timeout(&mut process, Duration::from_secs(timeout_seconds))
            .map_err(|error| {
                ServerError::AdapterFixture(format!("runtime wait failed: {error:?}"))
            })?;
        if let Err(_error) =
            scan_artifacts_for_sensitive_markers([&outcome.stdout.path, &outcome.stderr.path])
        {
            let _ = fs::remove_file(&outcome.stdout.path);
            let _ = fs::remove_file(&outcome.stderr.path);
            let execution = self.dispatch_execution_projection(
                context.plan,
                context.execution_request,
                DispatchExecutionOutcome {
                    status: "blocked_sensitive_artifact",
                    provider_cli_executed: true,
                    runtime_process_ref: Some(outcome.process.runtime_process_ref),
                    exit_code: outcome.exit_code.map(i64::from),
                    stdout_artifact_id: Some(outcome.stdout.artifact_id),
                    stderr_artifact_id: Some(outcome.stderr.artifact_id),
                    credential_scan_status: "blocked_sensitive_artifact",
                    raw_output_policy: "artifact_deleted_after_scan_failure",
                    reason_codes: "credential_artifact_scan_failed",
                },
            );
            self.append_dispatch_execution(origin, context.plan, &execution)?;
            return Ok(DispatchRunSummary::from_execution(
                &execution, 0, 0, 0, 0, 0,
            ));
        }
        if outcome.process.status != "exited" {
            let execution = self.dispatch_execution_projection(
                context.plan,
                context.execution_request,
                DispatchExecutionOutcome {
                    status: &outcome.process.status,
                    provider_cli_executed: true,
                    runtime_process_ref: Some(outcome.process.runtime_process_ref),
                    exit_code: outcome.exit_code.map(i64::from),
                    stdout_artifact_id: Some(outcome.stdout.artifact_id),
                    stderr_artifact_id: Some(outcome.stderr.artifact_id),
                    credential_scan_status: "clean",
                    raw_output_policy: "bounded_redacted_artifacts",
                    reason_codes: "provider_cli_exited_without_ingestion",
                },
            );
            self.append_dispatch_execution(origin, context.plan, &execution)?;
            return Ok(DispatchRunSummary::from_execution(
                &execution, 0, 0, 0, 0, 0,
            ));
        }
        let stdout = fs::read_to_string(&outcome.stdout.path).map_err(|error| {
            ServerError::AdapterFixture(format!("failed to read adapter stdout artifact: {error}"))
        })?;
        let adapter_events = parse_adapter_events(&context.plan.adapter_kind, &stdout)
            .map_err(ServerError::AdapterFixture)?;
        if adapter_events.is_empty() {
            return Err(ServerError::AdapterFixture(
                "live provider stdout produced no normalized events".to_string(),
            ));
        }
        let (_session, run_projection, _agent, run) =
            self.run_refs_for_session_run(&context.plan.session_id, &context.plan.run_id)?;
        let report = self
            .controller
            .apply_normalized_adapter_events_with_turn(&run, &adapter_events, Some(context.turn_id))
            .map_err(ServerError::State)?;
        let execution = self.dispatch_execution_projection(
            context.plan,
            context.execution_request,
            DispatchExecutionOutcome {
                status: "exited",
                provider_cli_executed: true,
                runtime_process_ref: Some(outcome.process.runtime_process_ref.clone()),
                exit_code: outcome.exit_code.map(i64::from),
                stdout_artifact_id: Some(outcome.stdout.artifact_id.clone()),
                stderr_artifact_id: Some(outcome.stderr.artifact_id.clone()),
                credential_scan_status: "clean",
                raw_output_policy: "bounded_redacted_artifacts",
                reason_codes: "provider_cli_executed_and_artifacts_scanned",
            },
        );
        self.append_dispatch_execution(origin, context.plan, &execution)?;
        self.append_dispatch_run_exit_with_metadata(
            origin,
            context.plan,
            &run_projection,
            true,
            "provider_cli_executed_and_artifacts_scanned",
        )?;
        let stdout_hash = stable_hash(stdout.as_bytes());
        let replay = self.dispatch_replay_projection_with_metadata(
            context.plan,
            context.gate,
            &outcome.stdout.artifact_id,
            &stdout_hash,
            &report,
            DispatchReplayMetadata {
                provider_cli_executed: true,
                raw_content_policy: "bounded_redacted_artifacts",
            },
        );
        self.append_dispatch_replay(origin, context.plan, &replay)?;
        Ok(DispatchRunSummary::from_execution(
            &execution,
            report.input_event_count,
            report.appended_event_count,
            report.tool_event_count,
            report.summary_event_count,
            report.completed_turn_count,
        ))
    }
}

fn normalize_policy_path(path: &str) -> Result<PathBuf, String> {
    if path.trim().is_empty() {
        return Err("empty path".to_string());
    }
    let raw = Path::new(path);
    for component in raw.components() {
        match component {
            Component::ParentDir => return Err("parent traversal is not allowed".to_string()),
            Component::Normal(part) => {
                let lower = part.to_string_lossy().to_ascii_lowercase();
                if is_credential_like_component(&lower) {
                    return Err(format!(
                        "credential-like path component `{}`",
                        part.to_string_lossy()
                    ));
                }
            }
            _ => {}
        }
    }
    let absolute = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| format!("failed to resolve current dir: {error}"))?
            .join(raw)
    };
    let normalized = normalize_existing_path_prefix(&absolute)?;
    reject_credential_like_path_components(&normalized)?;
    Ok(normalized)
}

fn normalize_existing_path_prefix(path: &Path) -> Result<PathBuf, String> {
    if path.exists() {
        return fs::canonicalize(path)
            .map_err(|error| format!("failed to canonicalize existing path: {error}"));
    }
    let mut missing_suffix = Vec::new();
    let mut cursor = path;
    while !cursor.exists() {
        let Some(name) = cursor.file_name() else {
            break;
        };
        missing_suffix.push(name.to_owned());
        let Some(parent) = cursor.parent() else {
            break;
        };
        cursor = parent;
    }
    let mut normalized = if cursor.exists() {
        fs::canonicalize(cursor)
            .map_err(|error| format!("failed to canonicalize existing path prefix: {error}"))?
    } else {
        cursor.to_path_buf()
    };
    for component in missing_suffix.into_iter().rev() {
        normalized.push(component);
    }
    Ok(normalized)
}

fn reject_credential_like_path_components(path: &Path) -> Result<(), String> {
    for component in path.components() {
        if let Component::Normal(part) = component {
            let lower = part.to_string_lossy().to_ascii_lowercase();
            if is_credential_like_component(&lower) {
                return Err(format!(
                    "credential-like path component `{}`",
                    part.to_string_lossy()
                ));
            }
        }
    }
    Ok(())
}

fn is_credential_like_component(component: &str) -> bool {
    matches!(
        component,
        ".ssh"
            | ".aws"
            | ".config"
            | ".codex"
            | ".claude"
            | ".anthropic"
            | "credentials"
            | "credential"
            | "secrets"
            | "secret"
            | "tokens"
            | "token"
            | "cookies"
            | "cookie"
            | "oauth"
            | "sessions"
            | "session"
    )
}
